// Owner: stickers P7a (STICKER_PLAN 3.2 "Mask", 3.3 "Layer-kind rules", 3.4; ARCHITECTURE 8.15). The mask commands of
// the active picture layer (Add from selection / Add, Invert, Apply, Delete; "Paint on mask"), Rasterize of a text or
// shape layer (the paint tools offer it) and the rectangle selection command (the Marquee tool of P7b makes the same
// selection; the scenarios use it). Each document change is one entry.
//
// Stickers P7b: the Selection commands (STICKER_PLAN 3.3): All, None, Invert, Grow / Shrink / Feather, Selection from
// layer alpha (app state, no entry), Delete (the selected pixels of the active picture layer, or hidden on its mask
// with "Paint on mask") and Crop to Selection (the artboard becomes the selection's bounds), one entry each. Color
// Range is in `tools/wand.rs`, Layer via Copy / Cut in `tools/move_selection.rs`.

use std::sync::Arc;

use glam::Vec2;
use wte_core::sticker::select;
use wte_core::{AlphaBuf, Commit, IRect, LayerMask, PicturePixels, PixelData, PixelTarget, SelectMode, Selection, StickerContent};

use crate::state::{AppState, StickerTool};
use crate::sticker::{edit_mut, edit_of, edit_ref, layered};

/// History labels of the mask commands.
pub(crate) const ADD_MASK_LABEL: &str = "Add Layer Mask";
pub(crate) const INVERT_MASK_LABEL: &str = "Invert Layer Mask";
pub(crate) const APPLY_MASK_LABEL: &str = "Apply Layer Mask";
pub(crate) const DELETE_MASK_LABEL: &str = "Delete Layer Mask";
/// Status line after a mask was added ("Paint on mask" is turned on).
pub(crate) const MASK_ADDED_HINT: &str = "Painting on the mask now: white shows, black hides (Paint on mask)";
/// Status line of a mask command on a layer without a mask.
pub(crate) const NO_MASK_HINT: &str = "The layer has no mask: Add mask first";
/// Stickers P7b: history labels of the selection commands.
pub(crate) const DELETE_SELECTED_LABEL: &str = "Delete Selected Pixels";
pub(crate) const CROP_TO_SELECTION_LABEL: &str = "Crop to Selection";
/// Status line of a selection command without a selection.
pub(crate) const NO_SELECTION_HINT: &str = "Select an area first";

/// What a mask command acts on: the sticker, the active picture layer, its mask and the picture's size.
type MaskTarget = (wte_core::StickerRef, wte_core::StickerLayerId, Option<Arc<AlphaBuf>>, [u32; 2]);

/// The active picture layer, its id and its mask, for a mask command (else the status line says why).
fn mask_target(state: &mut AppState) -> Option<MaskTarget> {
    // `paint_target` holds the rules (a picture layer, unlocked, shown); the Eyedropper never offers Rasterize.
    let paint_on_mask = state.sticker_ui.paint_on_mask;
    state.sticker_ui.paint_on_mask = false;
    let t = crate::sticker::tools::paint_target(state, StickerTool::Brush, false);
    state.sticker_ui.paint_on_mask = paint_on_mask;
    let t = t?;
    let doc = state.doc_mut()?;
    match doc.sticker_layer_mask(t.on, t.layer) {
        Ok(m) => Some((t.on, t.layer, m, t.size)),
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            None
        }
    }
}

/// "Add mask": from the selection (seen from the layer's pixels) or, without one, a mask that shows everything. The
/// paint tools then paint the mask ("Paint on mask" on).
pub(crate) fn add_mask(state: &mut AppState) {
    let Some((on, layer, mask, size)) = mask_target(state) else { return };
    if mask.is_some() {
        state.status_hint = "The layer has a mask already".to_owned();
        return;
    }
    let Some((doc, _)) = layered(state, Some(layer)) else { return };
    let Some(l) = doc.layer(layer) else { return };
    let values = match edit_of(state).and_then(|e| e.selection.as_ref()) {
        Some(sel) => wte_core::sticker::select::selection_in_content(&sel.mask, doc.artboard, &l.transform, size),
        None => AlphaBuf::filled(size[0], size[1], 255),
    };
    let r = state.doc_mut().map(|d| {
        d.set_sticker(
            on,
            move |s| {
                if let Some(l) = s.layer_mut(layer) {
                    l.mask = Some(LayerMask { pixels: Arc::new(values), version: 0 });
                }
            },
            ADD_MASK_LABEL,
            Commit::Final,
        )
    });
    if r.is_some_and(|r| crate::dispatch::edited(state, r).is_some()) {
        state.sticker_ui.paint_on_mask = true;
        state.status_hint = MASK_ADDED_HINT.to_owned();
    }
}

/// "Invert mask": every mask value `v` becomes `255 - v` (a tile patch of the changed tiles).
pub(crate) fn invert_mask(state: &mut AppState) {
    let Some((on, layer, mask, _)) = mask_target(state) else { return };
    let Some(mask) = mask else {
        state.status_hint = NO_MASK_HINT.to_owned();
        return;
    };
    let mut inverted = (*mask).clone();
    inverted.data.iter_mut().for_each(|v| *v = 255 - *v);
    let r = state.doc_mut().map(|d| d.replace_sticker_pixels(on, layer, PixelTarget::Mask, PixelData::Alpha(inverted), INVERT_MASK_LABEL));
    if let Some(r) = r {
        crate::dispatch::edited(state, r);
    }
}

/// "Apply mask": the picture's alpha is multiplied by the mask and the mask removed (one structure entry; the picture
/// becomes edited pixels).
pub(crate) fn apply_mask(state: &mut AppState) {
    let Some((on, layer, mask, _)) = mask_target(state) else { return };
    let Some(mask) = mask else {
        state.status_hint = NO_MASK_HINT.to_owned();
        return;
    };
    let Some(pixels) = state.doc_mut().and_then(|d| d.sticker_layer_pixels(on, layer).ok()) else { return };
    let mut applied = (*pixels).clone();
    if mask.data.len() * 4 == applied.data.len() {
        for (p, &m) in applied.data.as_chunks_mut::<4>().0.iter_mut().zip(&mask.data) {
            p[3] = wte_core::mul_u8(p[3], m);
        }
    }
    let r = state.doc_mut().map(|d| {
        d.set_sticker(
            on,
            move |s| {
                if let Some(l) = s.layer_mut(layer) {
                    l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(applied), version: 0 });
                    l.mask = None;
                }
            },
            APPLY_MASK_LABEL,
            Commit::Final,
        )
    });
    if r.is_some_and(|r| crate::dispatch::edited(state, r).is_some()) {
        state.sticker_ui.paint_on_mask = false;
    }
}

/// "Delete mask": the mask goes, the picture is unchanged.
pub(crate) fn delete_mask(state: &mut AppState) {
    let Some((on, layer, mask, _)) = mask_target(state) else { return };
    if mask.is_none() {
        state.status_hint = NO_MASK_HINT.to_owned();
        return;
    }
    let r = state.doc_mut().map(|d| {
        d.set_sticker(
            on,
            move |s| {
                if let Some(l) = s.layer_mut(layer) {
                    l.mask = None;
                }
            },
            DELETE_MASK_LABEL,
            Commit::Final,
        )
    });
    if r.is_some_and(|r| crate::dispatch::edited(state, r).is_some()) {
        state.sticker_ui.paint_on_mask = false;
    }
}

/// "Paint on mask" on or off (app state; on needs a mask on the active layer).
pub(crate) fn set_paint_on_mask(state: &mut AppState, on: bool) {
    if on && !layered(state, None).is_some_and(|(d, id)| d.layer(id).is_some_and(|l| l.mask.is_some())) {
        state.status_hint = NO_MASK_HINT.to_owned();
        state.sticker_ui.paint_on_mask = false;
        return;
    }
    state.sticker_ui.paint_on_mask = on;
}

/// Rasterize of the active text or shape layer (`Document::rasterize_sticker_layer`, one entry); the paint tools'
/// offer ends with it.
pub(crate) fn rasterize(state: &mut AppState) {
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(on) = edit_ref(&edit) else { return };
    let Some(layer) = state.sticker_ui.rasterize_offer.or(edit.active_layer) else { return };
    let Some((doc, _)) = layered(state, Some(layer)) else {
        state.status_hint = "Convert the text to a layered sticker first".to_owned();
        return;
    };
    if doc.layer(layer).is_some_and(|l| matches!(l.content, StickerContent::Picture(_))) {
        state.status_hint = "The layer is a picture already".to_owned();
        state.sticker_ui.rasterize_offer = None;
        return;
    }
    let Some(r) = state.doc_mut().map(|d| d.rasterize_sticker_layer(on, layer)) else { return };
    if crate::dispatch::edited(state, r).is_some() {
        state.sticker_ui.rasterize_offer = None;
        state.status_hint.clear();
    }
}

/// A rectangle selection between the layer-space corners `a` and `b` (replaces the selection; app state, no entry).
pub(crate) fn select_rect(state: &mut AppState, a: Vec2, b: Vec2) {
    let Some(doc) = layered_any(state) else { return };
    let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
    let [w, h] = doc.size();
    let sel = Selection::rect(w, h, a - origin, b - origin);
    if let Some(e) = edit_mut(state) {
        e.selection = (!sel.is_empty()).then(|| Arc::new(sel));
    }
}

/// Stickers P7b: what Grow / Shrink / Feather do.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Modify {
    Grow,
    Shrink,
    Feather,
}

/// Select All (Ctrl+A): the whole artboard.
pub(crate) fn select_all(state: &mut AppState) {
    let Some(doc) = layered_any(state) else { return };
    let [w, h] = doc.size();
    if let Some(e) = edit_mut(state) {
        e.selection = Some(Arc::new(Selection::all(w, h)));
    }
}

/// Invert Selection (Ctrl+Shift+I): `255 - m`; nothing selected becomes everything.
pub(crate) fn invert_selection(state: &mut AppState) {
    let Some(doc) = layered_any(state) else { return };
    let [w, h] = doc.size();
    let Some(e) = edit_mut(state) else { return };
    let mut sel = e.selection.as_deref().cloned().filter(|s| s.size() == [w, h]).unwrap_or_else(|| Selection::none(w, h));
    sel.invert();
    e.selection = (!sel.is_empty()).then(|| Arc::new(sel));
}

/// Grow / Shrink / Feather the selection by `n` pixels (`Selection::grow` / `shrink` / `feather`, at most 512).
pub(crate) fn modify_selection(state: &mut AppState, how: Modify, n: f32) {
    let Some(sel) = edit_of(state).and_then(|e| e.selection.clone()) else {
        state.status_hint = NO_SELECTION_HINT.to_owned();
        return;
    };
    let mut sel = (*sel).clone();
    modified(&mut sel, how, n);
    if let Some(e) = edit_mut(state) {
        e.selection = (!sel.is_empty()).then(|| Arc::new(sel));
    }
}

/// `sel` grown, shrunk or feathered by `n` (also the popover's live outline).
pub(crate) fn modified(sel: &mut Selection, how: Modify, n: f32) {
    match how {
        Modify::Grow => sel.grow(n),
        Modify::Shrink => sel.shrink(n),
        Modify::Feather => sel.feather(n),
    }
}

/// "Selection from layer alpha": the active layer's alpha (drawn alone on the artboard, its mask applied) becomes the
/// selection.
pub(crate) fn selection_from_alpha(state: &mut AppState) {
    let Some(picture) = crate::sticker::tools::wand::sample_picture(state, false) else { return };
    let mask = AlphaBuf::from_alpha(&picture);
    if mask.data.iter().all(|&v| v == 0) {
        state.status_hint = "The layer has no visible pixels on the artboard".to_owned();
    }
    crate::sticker::tools::set_selection(state, mask, SelectMode::Replace);
}

/// Delete with a selection (Delete / Backspace in the Sticker view, Selection > Delete): the selected pixels of the
/// active picture layer become transparent; with "Paint on mask" they are hidden on its mask. One entry; the stamp, its
/// texture layer and the sticker's layers stay.
pub(crate) fn delete_selected(state: &mut AppState) {
    if edit_of(state).is_none_or(|e| e.selection.is_none()) {
        state.status_hint = crate::sticker::DELETE_HINT.to_owned();
        return;
    }
    let Some(t) = crate::sticker::tools::paint_target(state, StickerTool::Eraser, false) else { return };
    let Some(sel) = crate::sticker::tools::selection_for(state, &t) else { return };
    let Some(doc) = state.doc_mut() else { return };
    let new = match t.target {
        PixelTarget::Pixels => match doc.sticker_layer_pixels(t.on, t.layer) {
            Ok(p) => {
                let mut out = (*p).clone();
                select::erase(&mut out, &sel);
                PixelData::Rgba(out)
            }
            Err(e) => {
                crate::dispatch::edited::<()>(state, Err(e));
                return;
            }
        },
        PixelTarget::Mask => match doc.sticker_layer_mask(t.on, t.layer) {
            Ok(Some(m)) => {
                let mut out = (*m).clone();
                select::fill_mask(&mut out, &sel, 0);
                PixelData::Alpha(out)
            }
            Ok(None) => return,
            Err(e) => {
                crate::dispatch::edited::<()>(state, Err(e));
                return;
            }
        },
    };
    crate::sticker::tools::put(state, &t, new, DELETE_SELECTED_LABEL);
}

/// Crop to Selection: the artboard becomes the bounds of the selected pixels (one entry; the users' placements are
/// refitted so the model shows the same pixels, STICKER_PLAN 2.9). The selection is dropped with the size change (3.4).
pub(crate) fn crop_to_selection(state: &mut AppState) {
    let Some(sel) = edit_of(state).and_then(|e| e.selection.clone()) else {
        state.status_hint = NO_SELECTION_HINT.to_owned();
        return;
    };
    let Some(edit) = edit_of(state).cloned() else { return };
    let Some(on) = edit_ref(&edit) else { return };
    let Some(doc) = layered_any(state) else { return };
    let b = sel.bounds;
    let ab = doc.artboard;
    let new = IRect::new(ab.x0 + b.x0, ab.y0 + b.y0, ab.x0 + b.x1, ab.y0 + b.y1);
    if new == ab {
        state.status_hint = "The selection covers the whole artboard already".to_owned();
        return;
    }
    let r = state.doc_mut().map(|d| d.set_sticker(on, move |s| s.artboard = new, CROP_TO_SELECTION_LABEL, Commit::Final));
    if let Some(r) = r
        && crate::dispatch::edited(state, r).is_some()
    {
        if let Some(e) = edit_mut(state) {
            e.selection = None;
        }
        state.status_hint.clear();
    }
}

/// The layered document of the open editor (any active layer).
fn layered_any(state: &mut AppState) -> Option<wte_core::StickerDoc> {
    let session = state.session.as_mut()?;
    let edit = session.sticker_edit.clone()?;
    let shown = crate::sticker::shown(&mut session.doc, &edit)?;
    Some(shown.layers()?.clone())
}
