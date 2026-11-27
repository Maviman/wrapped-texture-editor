// Owner: stickers P7a (STICKER_PLAN 3.3 row Eyedropper). The sticker Eyedropper: a click (and a drag) picks the colour of
// the finished sticker (all visible layers, their styles) under the pointer into the primary colour. A fully
// transparent pixel picks nothing (the status line says so); the colour is taken opaque.

use glam::Vec2;

use crate::action::AppAction;
use crate::state::AppState;
use crate::sticker::{edit_of, edit_ref};

/// Status line of a pick on a transparent pixel.
pub(crate) const NOTHING_HINT: &str = "Nothing to pick there (the sticker is transparent at that point)";

/// Picks the colour at layer-space point `p`; true when a colour was picked (`SetPrimaryColor` queued).
pub(crate) fn pick(state: &mut AppState, p: Vec2) -> bool {
    let Some(on) = edit_of(state).and_then(edit_ref) else { return false };
    let Some(color) = state.doc_mut().and_then(|d| d.sticker_color_at(on, p)) else { return false };
    if color[3] == 0 {
        state.status_hint = NOTHING_HINT.to_owned();
        return false;
    }
    let opaque = [color[0], color[1], color[2], 255];
    if state.colors.primary != opaque {
        state.push(AppAction::SetPrimaryColor(opaque));
    }
    state.status_hint.clear();
    true
}
