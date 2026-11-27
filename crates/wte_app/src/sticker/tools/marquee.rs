// Owner: stickers P7b (STICKER_PLAN 3.3 row Marquee, 3.4, 4.4, 4.8). The Marquee: a drag on the canvas draws a
// rectangle or an ellipse between the press and the pointer; at the release it becomes the selection, combined with the
// one there is by the mode (Shift adds, Alt subtracts, both intersect; else the Tool tab's mode row). Options: Feather
// (pixels, applied to the new shape before it is combined) and a fixed aspect (W : H). A click that draws no area
// drops the selection (New mode), as in Photoshop. App state only: no history entry.

use glam::Vec2;
use wte_core::{SelectMode, Selection};

use crate::state::AppState;

/// The Marquee's options (`StickerUi::marquee`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MarqueeOptions {
    /// An ellipse inscribed in the dragged box (else a rectangle).
    pub ellipse: bool,
    /// Feather of the new shape (pixels, 0 = hard edge).
    pub feather: f32,
    /// The box keeps the ratio `aspect[0] : aspect[1]`.
    pub fixed_aspect: bool,
    pub aspect: [f32; 2],
}

impl Default for MarqueeOptions {
    fn default() -> MarqueeOptions {
        MarqueeOptions { ellipse: false, feather: 0.0, fixed_aspect: false, aspect: [1.0, 1.0] }
    }
}

/// Below this box side (layer-space pixels) a drag is a click.
pub(crate) const MIN_SIDE: f32 = 0.5;

/// The mode of a new selection: the keys held (Shift adds, Alt subtracts, both intersect) or the Tool tab's row.
pub(crate) fn mode_of(mods: eframe::egui::Modifiers, row: SelectMode) -> SelectMode {
    match (mods.shift, mods.alt) {
        (true, true) => SelectMode::Intersect,
        (true, false) => SelectMode::Add,
        (false, true) => SelectMode::Subtract,
        (false, false) => row,
    }
}

/// The second corner of the box from `start` toward the pointer `to`, with the fixed aspect when it is on.
pub(crate) fn corner(opts: &MarqueeOptions, start: Vec2, to: Vec2) -> Vec2 {
    if !opts.fixed_aspect {
        return to;
    }
    let ax = if opts.aspect[0].is_finite() && opts.aspect[0] > 0.0 { opts.aspect[0] } else { 1.0 };
    let ay = if opts.aspect[1].is_finite() && opts.aspect[1] > 0.0 { opts.aspect[1] } else { 1.0 };
    let d = to - start;
    let k = (d.x.abs() / ax).max(d.y.abs() / ay);
    let sx = if d.x < 0.0 { -1.0 } else { 1.0 };
    let sy = if d.y < 0.0 { -1.0 } else { 1.0 };
    start + Vec2::new(sx * k * ax, sy * k * ay)
}

/// The Marquee's release: the box `start`..`to` (layer space) becomes the selection by `mode`.
pub(crate) fn finish(state: &mut AppState, start: Vec2, to: Vec2, mode: SelectMode) {
    let opts = state.sticker_ui.marquee;
    let Some((origin, [w, h])) = super::artboard_of(state) else { return };
    let b = corner(&opts, start, to);
    let (a, b) = (start - origin, b - origin);
    let d = (b - a).abs();
    if !(a.is_finite() && b.is_finite()) || d.x < MIN_SIDE || d.y < MIN_SIDE {
        if mode == SelectMode::Replace
            && let Some(e) = crate::sticker::edit_mut(state)
        {
            e.selection = None;
        }
        return;
    }
    let mut shape = if opts.ellipse { Selection::ellipse(w, h, a, b) } else { Selection::rect(w, h, a, b) };
    if opts.feather > 0.0 {
        shape.feather(opts.feather);
    }
    super::set_selection(state, shape.mask, mode);
}

/// The box of a drag as two corners (for the canvas outline while dragging).
pub(crate) fn drag_box(state: &AppState, start: Vec2, to: Vec2) -> (Vec2, Vec2) {
    (start, corner(&state.sticker_ui.marquee, start, to))
}

