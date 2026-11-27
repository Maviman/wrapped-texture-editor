// Owner: stickers P7a (STICKER_PLAN 3.3 row Fill, 3.4, 4.4). The sticker Fill tool: a click fills the colour region
// of the active picture layer under the pointer (Tolerance, Contiguous, Anti-alias: `StickerUi::fill`) with the primary
// colour; with a selection it fills the selection instead. With "Paint on mask" it fills the region of the mask's
// values with the primary colour's grey level. One entry "Fill" (`Document::replace_sticker_pixels`: a tile patch of
// the changed tiles; an image stamp is promoted inside it).

use glam::Vec2;
use wte_core::{AlphaBuf, PixelBuf, PixelData, PixelTarget};
use wte_core::sticker::select;

use crate::state::{AppState, StickerTool};

/// History label of the Fill tool.
pub(crate) const FILL_LABEL: &str = "Fill";
/// Status line of a click outside the layer's pixels.
pub(crate) const OUTSIDE_HINT: &str = "Click on the layer's pixels to fill (or select an area first)";

/// The Fill tool's click at layer-space point `p` (`StickerAction::Fill`).
pub(crate) fn fill(state: &mut AppState, p: Vec2) {
    let Some(t) = super::paint_target(state, StickerTool::Fill, false) else { return };
    let selection = super::selection_for(state, &t);
    let opts = state.sticker_ui.fill;
    let color = state.colors.primary;
    let Some(doc) = state.doc_mut() else { return };
    // The buffer as a picture (a mask as grey pixels, so the same region rule applies to its values).
    let (pixels, mask) = match t.target {
        PixelTarget::Pixels => match doc.sticker_layer_pixels(t.on, t.layer) {
            Ok(p) => ((*p).clone(), None),
            Err(e) => {
                crate::dispatch::edited::<()>(state, Err(e));
                return;
            }
        },
        PixelTarget::Mask => match doc.sticker_layer_mask(t.on, t.layer) {
            Ok(Some(m)) => (grey_picture(&m), Some((*m).clone())),
            Ok(None) => return,
            Err(e) => {
                crate::dispatch::edited::<()>(state, Err(e));
                return;
            }
        },
    };
    let coverage: AlphaBuf = match selection {
        Some(sel) => (*sel).clone(),
        None => {
            let c = t.to_content(p).floor();
            if !(c.is_finite() && c.x >= 0.0 && c.y >= 0.0 && c.x < t.size[0] as f32 && c.y < t.size[1] as f32) {
                state.status_hint = OUTSIDE_HINT.to_owned();
                return;
            }
            select::region(&pixels, (c.x as u32, c.y as u32), opts)
        }
    };
    let new = match mask {
        Some(mut m) => {
            select::fill_mask(&mut m, &coverage, super::grey(color));
            PixelData::Alpha(m)
        }
        None => {
            let mut out = pixels;
            select::fill(&mut out, &coverage, color);
            PixelData::Rgba(out)
        }
    };
    let Some(doc) = state.doc_mut() else { return };
    let r = doc.replace_sticker_pixels(t.on, t.layer, t.target, new, FILL_LABEL);
    if crate::dispatch::edited(state, r).is_some() {
        state.status_hint.clear();
    }
}

/// A mask as an opaque grey picture (its values in R, G and B).
pub(crate) fn grey_picture(m: &AlphaBuf) -> PixelBuf {
    let mut p = PixelBuf::new(m.width, m.height);
    for (px, &v) in p.data.as_chunks_mut::<4>().0.iter_mut().zip(&m.data) {
        *px = [v, v, v, 255];
    }
    p
}
