// Owner: stickers P7b (STICKER_PLAN 3.3 row Move with a selection, the Selection commands Layer via Copy / Layer via
// Cut). The selected pixels of the active picture layer become a layer of their own above it: "Layer via Copy" keeps
// them on the layer, "Layer via Cut" clears them there (one entry each; the new layer becomes the active one and the
// selection is dropped). The Move tool's drag that starts inside the selection on a picture layer cuts them first
// ("Layer via Cut") and moves that new layer: the cut and every position of the drag fold into one entry "Move
// Selection" (`Commit::Preview` calls, the Final at the release; Esc or another command takes the whole drag back), and
// the selection moves with the pixels.
//
// The new layer is a copy of the source layer (name + " copy" / " cut", opacity, blend, clip, styles, adjustments,
// mask, transform) with the selected pixels; on a layer that is only moved (scale 1, no rotation) its picture and mask
// are cropped to the selected pixels and its position moved to match, so a small piece of a big photo stays small.

use std::sync::Arc;

use glam::Vec2;
use wte_core::sticker::select;
use wte_core::{AlphaBuf, Commit, IRect, LayerMask, PicturePixels, PixelBuf, Selection, StickerContent, StickerLayer, StickerLayerId};

use crate::state::{AppState, StickerTool};
use crate::sticker::{edit_mut, edit_of, edit_ref, layered};

/// History labels.
pub(crate) const MOVE_SELECTION_LABEL: &str = "Move Selection";
pub(crate) const LAYER_VIA_COPY_LABEL: &str = "Layer via Copy";
pub(crate) const LAYER_VIA_CUT_LABEL: &str = "Layer via Cut";
/// Status line when the selection holds no pixel of the layer.
pub(crate) const NOTHING_SELECTED_HINT: &str = "The selection holds no pixels of this layer";

/// A Move Selection drag after its cut (`StickerUi::moving_cut`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct MovingCut {
    pub(crate) source: StickerLayerId,
    pub(crate) new: StickerLayerId,
    /// The new layer's translation where it was cut.
    pub(crate) from: Vec2,
}

/// The Move tool's press at layer-space point `p`: the active layer when the press is inside the selection and that
/// layer is a picture layer that can be changed (else the Move tool moves layers as before).
pub(crate) fn applies(state: &mut AppState, p: Vec2) -> Option<StickerLayerId> {
    let edit = edit_of(state)?.clone();
    let sel = edit.selection.as_ref()?;
    let (doc, id) = layered(state, None)?;
    let q = (p - Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32)).floor();
    if !q.is_finite() || sel.at(q.x as i32, q.y as i32) == 0 {
        return None;
    }
    let layer = doc.layer(id)?;
    (matches!(layer.content, StickerContent::Picture(_)) && layer.visible && !layer.locked).then_some(id)
}

/// The new layer and the source's pixels after a cut: `None` when the selection holds none of the layer's pixels.
struct Piece {
    layer: StickerLayer,
    erased: PixelBuf,
}

fn crop_pixels(p: &PixelBuf, r: IRect) -> PixelBuf {
    let (w, h) = (r.width(), r.height());
    let mut out = PixelBuf::new(w, h);
    for y in 0..h as usize {
        let s = ((r.y0 as usize + y) * p.width as usize + r.x0 as usize) * 4;
        out.data[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(&p.data[s..s + w as usize * 4]);
    }
    out
}

fn crop_alpha(m: &AlphaBuf, r: IRect) -> AlphaBuf {
    let (w, h) = (r.width(), r.height());
    let mut out = AlphaBuf::new(w, h);
    for y in 0..h as usize {
        let s = (r.y0 as usize + y) * m.width as usize + r.x0 as usize;
        out.data[y * w as usize..(y + 1) * w as usize].copy_from_slice(&m.data[s..s + w as usize]);
    }
    out
}

/// Cuts (or copies) the selected pixels of `layer` (its pixels `pixels`, the selection `sel` seen from them).
fn piece(layer: &StickerLayer, pixels: &PixelBuf, sel: &AlphaBuf, suffix: &str) -> Option<Piece> {
    let copy = select::extract(pixels, sel);
    let bounds = select::mask_bounds(&AlphaBuf::from_alpha(&copy));
    if bounds.is_empty() {
        return None;
    }
    let mut erased = pixels.clone();
    select::erase(&mut erased, sel);
    let mut new = StickerLayer { name: format!("{}{suffix}", layer.name), locked: false, ..layer.clone() };
    let plain_move = layer.transform.scale == Vec2::ONE && layer.transform.rotation == 0.0;
    let full = IRect::from_size(pixels.width, pixels.height);
    if plain_move && bounds != full {
        new.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(crop_pixels(&copy, bounds)), version: 0 });
        new.mask = layer.mask.as_ref().map(|m| LayerMask { pixels: Arc::new(crop_alpha(&m.pixels, bounds)), version: 0 });
        new.transform.translate += Vec2::new(bounds.x0 as f32, bounds.y0 as f32);
    } else {
        new.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(copy), version: 0 });
    }
    Some(Piece { layer: new, erased })
}

/// The selected pixels of the active picture layer (the rules of the paint tools: a picture layer, shown, unlocked).
fn cut_of(state: &mut AppState, source: StickerLayerId, suffix: &str) -> Option<(wte_core::StickerRef, Piece)> {
    let edit = edit_of(state)?.clone();
    if edit.active_layer != Some(source) {
        return None;
    }
    let Some(sel) = edit.selection.clone() else {
        state.status_hint = "Select an area first".to_owned();
        return None;
    };
    let t = super::paint_target(state, StickerTool::Brush, false)?;
    let (doc, _) = layered(state, Some(source))?;
    let layer = doc.layer(source)?.clone();
    let pixels = match state.doc_mut()?.sticker_layer_pixels(t.on, source) {
        Ok(p) => p,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return None;
        }
    };
    let seen = select::selection_in_content(&sel.mask, t.artboard, &t.transform, t.size);
    let Some(piece) = piece(&layer, &pixels, &seen, suffix) else {
        state.status_hint = NOTHING_SELECTED_HINT.to_owned();
        return None;
    };
    Some((t.on, piece))
}

/// Inserts `piece` above `source` (and clears the source when `cut`), moved by `offset`: one `set_sticker` call.
/// Returns the new layer's id and where it was cut.
#[allow(clippy::too_many_arguments)]
fn insert(state: &mut AppState, on: wte_core::StickerRef, source: StickerLayerId, piece: Piece, cut: bool, offset: Vec2, label: &str, commit: Commit) -> Option<(StickerLayerId, Vec2)> {
    let from = piece.layer.transform.translate;
    let Piece { mut layer, erased } = piece;
    let r = state.doc_mut()?.set_sticker(
        on,
        move |d| {
            let Some(i) = d.layers.iter().position(|l| l.id == source) else { return };
            layer.id = d.alloc_layer();
            layer.transform.translate += offset;
            if cut {
                d.layers[i].content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(erased), version: 0 });
            }
            d.layers.insert(i + 1, layer);
        },
        label,
        commit,
    );
    crate::dispatch::edited(state, r)?;
    let (doc, _) = layered(state, Some(source))?;
    let i = doc.layers.iter().position(|l| l.id == source)?;
    Some((doc.layers.get(i + 1)?.id, from))
}

/// Layer via Copy (`cut` false) / Layer via Cut (`cut` true) of the active picture layer.
pub(crate) fn layer_via(state: &mut AppState, cut: bool) {
    let Some((_, source)) = layered(state, None) else { return };
    let (suffix, label) = if cut { (" cut", LAYER_VIA_CUT_LABEL) } else { (" copy", LAYER_VIA_COPY_LABEL) };
    let Some((on, piece)) = cut_of(state, source, suffix) else { return };
    let Some((new, _)) = insert(state, on, source, piece, cut, Vec2::ZERO, label, Commit::Final) else { return };
    crate::sticker::set_active(state, new);
    if let Some(e) = edit_mut(state) {
        e.selection = None;
    }
    state.status_hint.clear();
}

/// One step of a Move Selection drag (`StickerAction::MoveSelection`): the first call cuts, every call places the new
/// layer at `offset` (rounded to whole pixels) from where it was cut; the Final ends the entry, makes the new layer
/// active and moves the selection by the same offset.
pub(crate) fn run(state: &mut AppState, source: StickerLayerId, offset: Vec2, commit: Commit) {
    if !offset.is_finite() {
        return;
    }
    let offset = offset.round();
    let continuing = state.sticker_ui.moving_cut.filter(|c| c.source == source).filter(|_| {
        state.doc().is_some_and(|d| d.has_pending_edit() && d.undo_label() == Some(MOVE_SELECTION_LABEL))
    });
    let cut = match continuing {
        Some(c) => {
            let Some(on) = edit_of(state).and_then(edit_ref) else { return };
            let to = c.from + offset;
            let new = c.new;
            let r = state.doc_mut().map(|d| {
                d.set_sticker(
                    on,
                    move |s| {
                        if let Some(l) = s.layer_mut(new) {
                            l.transform.translate = to;
                        }
                    },
                    MOVE_SELECTION_LABEL,
                    commit,
                )
            });
            if r.is_none_or(|r| crate::dispatch::edited(state, r).is_none()) {
                state.sticker_ui.moving_cut = None;
                return;
            }
            c
        }
        None => {
            let Some((on, piece)) = cut_of(state, source, " cut") else { return };
            let Some((new, from)) = insert(state, on, source, piece, true, offset, MOVE_SELECTION_LABEL, commit) else { return };
            MovingCut { source, new, from }
        }
    };
    if commit == Commit::Final {
        state.sticker_ui.moving_cut = None;
        crate::sticker::set_active(state, cut.new);
        shift_selection(state, offset);
        state.status_hint.clear();
    } else {
        state.sticker_ui.moving_cut = Some(cut);
    }
}

/// Esc or another command during a Move Selection drag: the cut and the moves go (the pending entry is cancelled).
pub(crate) fn cancel(state: &mut AppState) {
    state.sticker_ui.moving_cut = None;
    if let Some(doc) = state.doc_mut()
        && doc.has_pending_edit()
        && doc.undo_label() == Some(MOVE_SELECTION_LABEL)
    {
        doc.cancel_pending_edit();
    }
}

/// The selection moved by whole pixels (what leaves the artboard is dropped).
fn shift_selection(state: &mut AppState, offset: Vec2) {
    let Some(e) = edit_mut(state) else { return };
    let Some(sel) = e.selection.as_ref() else { return };
    let (dx, dy) = (offset.x as i64, offset.y as i64);
    if dx == 0 && dy == 0 {
        return;
    }
    let (w, h) = (sel.mask.width as i64, sel.mask.height as i64);
    let mut out = AlphaBuf::new(sel.mask.width, sel.mask.height);
    for y in 0..h {
        let sy = y - dy;
        if sy < 0 || sy >= h {
            continue;
        }
        for x in 0..w {
            let sx = x - dx;
            if sx >= 0 && sx < w {
                out.data[(y * w + x) as usize] = sel.mask.data[(sy * w + sx) as usize];
            }
        }
    }
    let moved = Selection::from_mask(out);
    e.selection = (!moved.is_empty()).then(|| Arc::new(moved));
}
