// Owner: stickers P7b (STICKER_PLAN 3.3 row Lasso, 3.4, 4.8). The Lasso: a drag draws a freehand outline that closes
// at the release; clicks put the corners of a polygon that closes with a double-click, a click on its first point,
// Enter or the Tool tab's "Close lasso" (Esc or "Cancel" drops it). The closed outline (non-zero rule, edge distance of
// 4.8) becomes the selection by the mode fixed at the first press (Shift adds, Alt subtracts, both intersect; else the
// Tool tab's row), feathered by the Lasso's Feather. App state only: no history entry.

use glam::Vec2;
use wte_core::{SelectMode, Selection};

use crate::state::AppState;
use crate::sticker::CanvasDrag;

/// A press this close to the first point (screen points) closes the polygon.
pub(crate) const CLOSE_DISTANCE: f32 = 8.0;
/// A held press that travels this far (screen points) draws a freehand lasso.
pub(crate) const FREEHAND_TRAVEL: f32 = 4.0;
/// A freehand point is kept when it is this far from the last one (screen points).
pub(crate) const FREEHAND_STEP: f32 = 2.0;
/// Most points of one lasso (a longer freehand drag keeps its first points and the closing line).
pub(crate) const MAX_POINTS: usize = 20_000;

/// An open lasso (`StickerUi::lasso`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LassoState {
    /// The corners so far (layer space).
    pub(crate) points: Vec<Vec2>,
    pub(crate) mode: SelectMode,
    /// The artboard's size when it started (a change of size drops it).
    pub(crate) size: [u32; 2],
    /// Drawn by a drag (it closes at the release).
    pub(crate) freehand: bool,
}

/// A press of the Lasso at layer point `p` (screen `pos`; `near_first`: on the open polygon's first point).
pub(crate) fn press(state: &mut AppState, p: Vec2, pos: eframe::egui::Pos2, mode: SelectMode, near_first: bool, button: eframe::egui::PointerButton) {
    let Some((_, size)) = super::artboard_of(state) else { return };
    match state.sticker_ui.lasso.as_mut() {
        Some(l) if near_first && l.points.len() >= 3 => {
            close(state);
            return;
        }
        Some(l) => {
            if l.points.len() < MAX_POINTS && l.points.last() != Some(&p) {
                l.points.push(p);
            }
        }
        None => state.sticker_ui.lasso = Some(LassoState { points: vec![p], mode, size, freehand: false }),
    }
    state.sticker_ui.drag = Some(CanvasDrag::Lasso { press: pos, freehand: false, button });
}

/// Pointer motion while the button is held: a drag from the first point draws freehand (a point every
/// `FREEHAND_STEP` screen points; `zoom` = screen points per layer pixel).
pub(crate) fn motion(state: &mut AppState, p: Vec2, pos: eframe::egui::Pos2, zoom: f32) {
    let Some(CanvasDrag::Lasso { press, freehand, button }) = state.sticker_ui.drag else { return };
    let Some(l) = state.sticker_ui.lasso.as_mut() else { return };
    let freehand = freehand || (l.points.len() == 1 && pos.distance(press) > FREEHAND_TRAVEL);
    if freehand {
        l.freehand = true;
        let step = FREEHAND_STEP / zoom.max(1e-3);
        if l.points.len() < MAX_POINTS && l.points.last().is_none_or(|q| q.distance(p) >= step) && p.is_finite() {
            l.points.push(p);
        }
    }
    state.sticker_ui.drag = Some(CanvasDrag::Lasso { press, freehand, button });
}

/// The release: a freehand lasso closes; a polygon waits for its next corner.
pub(crate) fn release(state: &mut AppState) {
    if state.sticker_ui.lasso.as_ref().is_some_and(|l| l.freehand) {
        close(state);
    }
}

/// Closes the open lasso: three corners or more become the selection by its mode; fewer drop it.
pub(crate) fn close(state: &mut AppState) {
    let Some(l) = state.sticker_ui.lasso.take() else { return };
    if matches!(state.sticker_ui.drag, Some(CanvasDrag::Lasso { .. })) {
        state.sticker_ui.drag = None;
    }
    if l.points.len() < 3 {
        state.status_hint = "A lasso needs at least three points".to_owned();
        return;
    }
    let Some((origin, [w, h])) = super::artboard_of(state) else { return };
    if [w, h] != l.size {
        return;
    }
    let points: Vec<Vec2> = l.points.iter().map(|p| *p - origin).collect();
    let mut shape = Selection::polygon(w, h, &points);
    let feather = state.sticker_ui.lasso_feather;
    if feather > 0.0 {
        shape.feather(feather);
    }
    super::set_selection(state, shape.mask, l.mode);
}
