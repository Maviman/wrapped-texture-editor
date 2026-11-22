//! Fill (ARCHITECTURE.md 8.6): fills the active layer, or the UV island under the cursor, with the primary colour.

use crate::state::AppState;
use crate::tools::{NO_UV_TEXT, Surface, Tool, ViewPointer, ViewXform, error_text, paint_layer};

/// Fill: fills the active layer or the UV island under the cursor.
#[derive(Default)]
pub struct FillTool {}

impl Tool for FillTool {
    fn press(&mut self, state: &mut AppState, _xf: &ViewXform, p: &ViewPointer) -> bool {
        let (set, uv) = match p.surface {
            Surface::Model(hit) => match hit.texture_set.zip(hit.uv) {
                Some(v) => v,
                None => {
                    state.toasts.warn(NO_UV_TEXT);
                    return false;
                }
            },
            Surface::Uv { set, uv, .. } => (set, uv),
            Surface::Miss => return false,
        };
        if let Some(session) = state.session.as_mut() {
            session.active_set = set;
        }
        let Some(layer) = paint_layer(state, set) else { return false };
        let (color, scope) = (state.colors.primary, state.tools.fill_scope);
        let Some(doc) = state.doc_mut() else { return false };
        if let Err(e) = doc.fill(set, layer, uv, color, scope) {
            state.toasts.warn(error_text(&e));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::state::ToolKind;
    use crate::tools::ToolController;
    use crate::tools::test_support::*;
    use eframe::egui;
    use wte_core::{FillScope, LayerKind, SampleKind};

    fn opaque_count(state: &crate::state::AppState, set: usize, layer: usize) -> usize {
        let LayerKind::Raster(buf) = &state.doc().unwrap().sets[set].layers[layer].kind else { panic!() };
        buf.data.chunks(4).filter(|p| p[3] == 255).count()
    }

    #[test]
    fn fills_an_island_from_the_3d_view_and_the_whole_layer_from_2d() {
        let mut state = state_with(SampleKind::Cube);
        state.tool = ToolKind::Fill;
        state.colors.primary = [10, 200, 30, 255];
        let mut tools = ToolController::new();
        let xf = xf_3d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy());
        let island = opaque_count(&state, 0, 1);
        let set = &state.doc().unwrap().sets[0];
        let total = (set.width * set.height) as usize;
        assert!(island > 0 && island < total / 2, "one cube face island: {island} of {total}");

        state.tools.fill_scope = FillScope::Layer;
        let xf = xf_2d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert_eq!(opaque_count(&state, 0, 1), total);
        assert_eq!(state.doc().unwrap().history.undo_len(), 2);
    }
}
