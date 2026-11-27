// Owner: stickers P7b (STICKER_PLAN 3.3 row Magic Eraser, 3.4, 4.4). The Magic Eraser: a click makes the colour region
// of the active picture layer under the pointer transparent (Tolerance, Contiguous, Anti-alias; `alpha * (255 - region)
// / 255`), limited by the selection. "Into mask (keep pixels)" (or "Paint on mask") hides the region on the layer mask
// instead, adding a mask when the layer has none. One entry "Magic Eraser"; an image stamp is promoted inside it.

use glam::Vec2;
use wte_core::sticker::select;
use wte_core::{AlphaBuf, PixelData, PixelTarget, RegionOptions};

use crate::state::{AppState, StickerTool};

/// History label of the Magic Eraser.
pub(crate) const MAGIC_ERASER_LABEL: &str = "Magic Eraser";

/// The Magic Eraser's options (`StickerUi::magic_eraser`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct MagicEraserOptions {
    pub region: RegionOptions,
    /// Hide the region on the layer mask; the pixels are kept.
    pub into_mask: bool,
}

/// The Magic Eraser's click at layer-space point `p` (`StickerAction::MagicErase`).
pub(crate) fn erase_at(state: &mut AppState, p: Vec2) {
    let Some(t) = super::paint_target(state, StickerTool::MagicEraser, false) else { return };
    let selection = super::selection_for(state, &t);
    let opts = state.sticker_ui.magic_eraser;
    let Some(doc) = state.doc_mut() else { return };
    let pixels = match doc.sticker_layer_pixels(t.on, t.layer) {
        Ok(p) => p,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return;
        }
    };
    let c = t.to_content(p).floor();
    if !(c.is_finite() && c.x >= 0.0 && c.y >= 0.0 && c.x < t.size[0] as f32 && c.y < t.size[1] as f32) {
        state.status_hint = "Click on the layer's pixels to erase".to_owned();
        return;
    }
    let mut coverage = select::region(&pixels, (c.x as u32, c.y as u32), opts.region);
    if let Some(sel) = &selection {
        super::limit(&mut coverage, sel);
    }
    if opts.into_mask || t.target == PixelTarget::Mask {
        let mask = match state.doc_mut().map(|d| d.sticker_layer_mask(t.on, t.layer)) {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                crate::dispatch::edited::<()>(state, Err(e));
                return;
            }
            None => return,
        };
        match mask {
            Some(m) => {
                let mut out = (*m).clone();
                select::erase_mask(&mut out, &coverage);
                super::put(state, &t, PixelData::Alpha(out), MAGIC_ERASER_LABEL);
            }
            None => {
                let mut out = AlphaBuf::filled(t.size[0], t.size[1], 255);
                select::erase_mask(&mut out, &coverage);
                super::put_new_mask(state, &t, out, MAGIC_ERASER_LABEL);
            }
        }
        return;
    }
    let mut out = (*pixels).clone();
    select::erase(&mut out, &coverage);
    super::put(state, &t, PixelData::Rgba(out), MAGIC_ERASER_LABEL);
}
