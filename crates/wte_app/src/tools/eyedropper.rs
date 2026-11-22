//! Eyedropper (ARCHITECTURE.md 8.6): picks the composite colour under the cursor as the primary colour; dragging keeps
//! picking.

use eframe::egui;
use wte_core::SampleSource;

use crate::action::AppAction;
use crate::state::{AppState, ViewId};
use crate::tools::{Tool, ViewPointer, ViewXform};

/// Eyedropper: picks the composite colour under the cursor as the primary colour.
#[derive(Default)]
pub struct EyedropperTool {
    /// Last colour pushed during the current press (avoids queueing the same colour every frame).
    last: Option<[u8; 4]>,
    /// The cursor in a view (for the swatch next to it).
    cursor: Option<(ViewId, egui::Pos2)>,
}

impl EyedropperTool {
    /// Start of a frame: clears the hover feedback.
    pub(crate) fn begin_frame(&mut self) {
        self.cursor = None;
    }

    /// Samples the composite under the pointer; a hit without a texture set picks nothing.
    fn sample(&mut self, state: &mut AppState, p: &ViewPointer) {
        self.cursor = Some((p.view, p.pos));
        let Some((set, uv)) = p.surface.set_uv() else { return };
        let Some(color) = state.doc().and_then(|d| d.sample_color(set, uv, SampleSource::Composite)) else { return };
        if self.last != Some(color) {
            self.last = Some(color);
            state.push(AppAction::SetPrimaryColor(color));
        }
    }
}

impl Tool for EyedropperTool {
    fn hover(&mut self, _state: &mut AppState, _xf: &ViewXform, p: &ViewPointer) {
        self.cursor = Some((p.view, p.pos));
    }

    fn press(&mut self, state: &mut AppState, _xf: &ViewXform, p: &ViewPointer) -> bool {
        self.last = None;
        self.sample(state, p);
        true
    }

    fn drag(&mut self, state: &mut AppState, _xf: &ViewXform, p: &ViewPointer) {
        self.sample(state, p);
    }

    fn release(&mut self, _state: &mut AppState, _xf: &ViewXform, _p: &ViewPointer) {
        self.last = None;
    }

    fn cancel(&mut self, _state: &mut AppState) {
        self.last = None;
    }

    fn overlay(&self, state: &AppState, xf: &ViewXform, painter: &egui::Painter) {
        let Some((view, pos)) = self.cursor else { return };
        if view != xf.view {
            return;
        }
        let Some([r, g, b, _]) = state.hover.rgba else { return };
        let swatch = egui::Rect::from_min_size(pos + egui::vec2(14.0, 14.0), egui::vec2(18.0, 18.0));
        painter.rect(
            swatch,
            2.0,
            egui::Color32::from_rgb(r, g, b),
            egui::Stroke::new(1.0, egui::Color32::WHITE),
            egui::StrokeKind::Outside,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::action::AppAction;
    use crate::state::ToolKind;
    use crate::tools::ToolController;
    use crate::tools::test_support::*;
    use eframe::egui;
    use wte_core::SampleKind;

    #[test]
    fn picks_the_composite_colour_in_both_views() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Eyedropper;
        let mut tools = ToolController::new();
        for xf in [xf_3d(&mut state), xf_2d(&mut state)] {
            state.actions.clear();
            let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            tools.release(&mut state, &xf, &p);
            let (set, uv) = p.surface.set_uv().expect("surface under the centre");
            let expected = state.doc().unwrap().sample_color(set, uv, wte_core::SampleSource::Composite).unwrap();
            assert!(matches!(state.actions.as_slice(), [AppAction::SetPrimaryColor(c)] if *c == expected));
        }
    }

    #[test]
    fn alt_turns_the_brush_into_the_eyedropper() {
        let mut state = state_with(SampleKind::Sphere);
        state.tool = ToolKind::Brush;
        let mut tools = ToolController::new();
        let xf = xf_2d(&mut state);
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::ALT);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        tools.release(&mut state, &xf, &p);
        assert!(matches!(state.actions.as_slice(), [AppAction::SetPrimaryColor(_)]));
        assert_eq!(state.doc().unwrap().history.undo_len(), 0);
    }
}
