// Owner: stickers P7a (STICKER_PLAN 3.2 "Mask", 3.3 "Layer-kind rules", 3.4; ARCHITECTURE 8.15). The mask commands of
// the active picture layer (Add from selection / Add, Invert, Apply, Delete; "Paint on mask"), Rasterize of a text or
// shape layer (the paint tools offer it) and the rectangle selection command (the Marquee tool of P7b makes the same
// selection; the scenarios use it). Each document change is one entry.

use std::sync::Arc;

use glam::Vec2;
use wte_core::{AlphaBuf, Commit, LayerMask, PicturePixels, PixelData, PixelTarget, Selection, StickerContent};

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
        e.selection = Some(sel);
    }
}

/// The layered document of the open editor (any active layer).
fn layered_any(state: &mut AppState) -> Option<wte_core::StickerDoc> {
    let session = state.session.as_mut()?;
    let edit = session.sticker_edit.clone()?;
    let shown = crate::sticker::shown(&mut session.doc, &edit)?;
    Some(shown.layers()?.clone())
}
