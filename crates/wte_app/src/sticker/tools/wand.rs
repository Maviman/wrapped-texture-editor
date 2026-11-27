// Owner: stickers P7b (STICKER_PLAN 3.3 row Magic Wand, 3.4, 4.4). The Magic Wand: a click selects the colour region
// around the clicked pixel (Tolerance, Contiguous, Anti-alias) of the active layer drawn alone on the artboard, or with
// "Sample: all layers" of the sticker's composite without styles (`Document::sticker_layer_on_artboard` /
// `sticker_unstyled_composite`); the region is combined with the selection by the mode (Shift adds, Alt subtracts,
// both intersect; else the Tool tab's row). Color Range (the Selection command) is the same with Contiguous off, from
// a picked colour instead of a pixel. App state only: no history entry.

use glam::Vec2;
use wte_core::sticker::select;
use wte_core::{PixelBuf, RegionOptions, SelectMode};

use crate::state::AppState;
use crate::sticker::{edit_of, edit_ref};

/// The Magic Wand's options (`StickerUi::wand`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WandOptions {
    pub region: RegionOptions,
    /// Sample the composite of all visible layers (without styles), else the active layer alone.
    pub all_layers: bool,
}

impl Default for WandOptions {
    /// Tolerance 32, Contiguous, Anti-alias, the active layer (STICKER_PLAN 3.3).
    fn default() -> WandOptions {
        WandOptions { region: RegionOptions::default(), all_layers: false }
    }
}

/// Status line of a wand click outside the artboard.
pub(crate) const OUTSIDE_HINT: &str = "Click inside the artboard to select by colour";

/// The picture the wand and Color Range look at: the composite without styles, or the active layer alone on the
/// artboard (`None` after saying why).
pub(crate) fn sample_picture(state: &mut AppState, all_layers: bool) -> Option<PixelBuf> {
    let edit = edit_of(state)?.clone();
    let on = edit_ref(&edit)?;
    let layer = edit.active_layer;
    let doc = state.doc_mut()?;
    let r = if all_layers {
        doc.sticker_unstyled_composite(on)
    } else {
        match layer {
            Some(id) => doc.sticker_layer_on_artboard(on, id),
            None => {
                state.status_hint = "Pick a layer in the Layers tab first".to_owned();
                return None;
            }
        }
    };
    match r {
        Ok(p) => Some(p),
        Err(e) => {
            if matches!(e, wte_core::EditError::InvalidParam(_)) {
                state.status_hint = super::TEXT_STAMP_HINT.to_owned();
            } else {
                crate::dispatch::edited::<()>(state, Err(e));
            }
            None
        }
    }
}

/// The wand's click at layer-space point `p`.
pub(crate) fn select_at(state: &mut AppState, p: Vec2, mode: SelectMode) {
    let opts = state.sticker_ui.wand;
    let Some((origin, [w, h])) = super::artboard_of(state) else { return };
    let q = (p - origin).floor();
    if !(q.is_finite() && q.x >= 0.0 && q.y >= 0.0 && q.x < w as f32 && q.y < h as f32) {
        state.status_hint = OUTSIDE_HINT.to_owned();
        return;
    }
    let Some(picture) = sample_picture(state, opts.all_layers) else { return };
    let region = select::region(&picture, (q.x as u32, q.y as u32), opts.region);
    state.status_hint.clear();
    super::set_selection(state, region, mode);
}

/// Color Range: every pixel within `tolerance` of `color` in the sampled picture, combined by `mode`.
pub(crate) fn color_range(state: &mut AppState, color: [u8; 4], tolerance: u8, all_layers: bool, mode: SelectMode) {
    let Some(region) = color_range_mask(state, color, tolerance, all_layers) else { return };
    super::set_selection(state, region, mode);
}

/// The mask of Color Range (also the popover's live outline).
pub(crate) fn color_range_mask(state: &mut AppState, color: [u8; 4], tolerance: u8, all_layers: bool) -> Option<wte_core::AlphaBuf> {
    let picture = sample_picture(state, all_layers)?;
    Some(select::color_range(&picture, color, tolerance, true))
}
