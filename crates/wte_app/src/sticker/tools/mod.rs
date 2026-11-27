// Owner: stickers P7a (STICKER_PLAN 3.3, 3.4, 3.10; ARCHITECTURE 8.15). The paint tools of the Sticker view: Brush and
// Eraser (`brush.rs`: strokes through `Document::sticker_stroke_*`), Fill (`fill.rs`), Gradient (`gradient.rs`) and
// Eyedropper (`eyedropper.rs`). The canvas (`canvas.rs`) routes the pointer to them; Fill and Gradient end in a
// `StickerAction` (one entry each through `Document::replace_sticker_pixels`).
//
// `paint_target` holds the rules every paint tool shares (3.3 "Layer-kind rules"): a picture layer (a text or shape
// layer offers Rasterize, never a silent rasterize), not locked, shown; the buffer is the picture or, with "Paint on
// mask" and a mask, the mask; the selection limits the change (3.4), seen from the layer's content pixels
// (`selection_in_content`, so a moved, scaled or turned layer is limited where the selection is on the artboard).

use std::sync::Arc;

use glam::Vec2;
use wte_core::{AlphaBuf, IRect, LayerTransform, PixelTarget, StickerContent, StickerLayerId, StickerRef};

use crate::state::{AppState, StickerTool};
use crate::sticker::{Shown, edit_of, edit_ref, shown};

pub mod brush;
pub mod eyedropper;
pub mod fill;
pub mod gradient;

/// The words of the Rasterize offer (status line, toast and the header).
pub(crate) const RASTERIZE_HINT: &str = "Rasterize this layer to paint on it";
/// Status line of a paint tool on a hidden layer.
pub(crate) const HIDDEN_HINT: &str = "The layer is hidden: show it first";
/// Status line of a paint tool on a locked layer.
pub(crate) const LOCKED_HINT: &str = "The layer is locked: unlock it first";
/// Status line of a paint tool on a text stamp.
pub(crate) const TEXT_STAMP_HINT: &str = "A text stamp is edited in the Text tab; convert it to a layered sticker to paint on it";

/// The buffer a paint tool changes, resolved before anything changes.
#[derive(Clone, Debug)]
pub(crate) struct PaintTarget {
    /// What the document command gets (an image stamp is promoted inside it).
    pub(crate) on: StickerRef,
    pub(crate) layer: StickerLayerId,
    /// The picture, or its mask ("Paint on mask" with a mask).
    pub(crate) target: PixelTarget,
    /// The buffer's size (content pixels).
    pub(crate) size: [u32; 2],
    pub(crate) transform: LayerTransform,
    pub(crate) artboard: IRect,
    /// The layer has an enabled style (a stroke runs in stroke mode).
    pub(crate) styled: bool,
}

impl PaintTarget {
    /// Layer space -> content pixels of the buffer (continuous).
    pub(crate) fn to_content(&self, p: Vec2) -> Vec2 {
        self.transform.invert(self.size, p)
    }
}

/// The paint target of the active layer for `tool`, or `None` after saying why (status line; a text or shape layer
/// also gets the warning toast and the Rasterize offer). `ask`: a content edit of a shared sticker asks the E6
/// question first (strokes; Fill and Gradient asked in `run_in` already).
pub(crate) fn paint_target(state: &mut AppState, tool: StickerTool, ask: bool) -> Option<PaintTarget> {
    let edit = edit_of(state)?.clone();
    let on = edit_ref(&edit)?;
    let session = state.session.as_mut()?;
    let shown = shown(&mut session.doc, &edit)?;
    if let Shown::Text { .. } = shown {
        state.status_hint = TEXT_STAMP_HINT.to_owned();
        return None;
    }
    let doc = shown.layers()?;
    let Some(layer) = edit.active_layer.and_then(|id| doc.layer(id)).cloned() else {
        state.status_hint = "Pick a layer in the Layers tab first".to_owned();
        return None;
    };
    let artboard = doc.artboard;
    if !matches!(layer.content, StickerContent::Picture(_)) {
        if tool.paints() {
            if state.sticker_ui.rasterize_offer != Some(layer.id) {
                state.toasts.warn(RASTERIZE_HINT);
            }
            state.sticker_ui.rasterize_offer = Some(layer.id);
            state.status_hint = RASTERIZE_HINT.to_owned();
        }
        return None;
    }
    if layer.locked {
        state.status_hint = LOCKED_HINT.to_owned();
        return None;
    }
    if !layer.visible {
        state.status_hint = HIDDEN_HINT.to_owned();
        return None;
    }
    if ask && let Some(users) = crate::sticker::shared_users(state) {
        crate::sticker::ask_shared_for(state, None, users);
        return None;
    }
    let mask = state.sticker_ui.paint_on_mask && layer.mask.is_some();
    let target = if mask { PixelTarget::Mask } else { PixelTarget::Pixels };
    let doc = state.doc_mut()?;
    let pixels = match doc.sticker_layer_pixels(on, layer.id) {
        Ok(p) => p,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return None;
        }
    };
    Some(PaintTarget {
        on,
        layer: layer.id,
        target,
        size: [pixels.width, pixels.height],
        transform: layer.transform,
        artboard,
        styled: layer.styles.any_on(),
    })
}

/// The open editor's selection seen from the target's content pixels (`None` without a selection).
pub(crate) fn selection_for(state: &AppState, t: &PaintTarget) -> Option<Arc<AlphaBuf>> {
    let sel = edit_of(state)?.selection.as_ref()?;
    Some(Arc::new(wte_core::sticker::select::selection_in_content(&sel.mask, t.artboard, &t.transform, t.size)))
}

/// Rec. 601 grey level of a colour (the value a paint tool puts into a mask: white shows, black hides).
pub(crate) fn grey(c: [u8; 4]) -> u8 {
    ((299 * c[0] as u32 + 587 * c[1] as u32 + 114 * c[2] as u32 + 500) / 1000) as u8
}

#[cfg(test)]
mod tests;
