// Owner: app-panels (A4). The Color page of the side panel's upper group (ARCHITECTURE.md 8.7; objects-tabs WP4: the
// tab is the section, no collapsing header): colour picker for the primary colour, hex field, 24 swatches (click =
// use, right-click = store the primary colour there).

use eframe::egui;
use egui::{Color32, Sense, Stroke, vec2};

use crate::panels::{PanelState, from_color32, hex_of, paint_checker, parse_hex, to_color32};
use crate::state::{AppState, SWATCH_COUNT};

/// Swatches per row.
const SWATCH_COLUMNS: usize = 12;

/// Stores the primary colour in swatch `index` and remembers the palette in the preferences.
pub(crate) fn store_swatch(state: &mut AppState, index: usize) {
    if index >= state.colors.swatches.len() {
        return;
    }
    state.colors.swatches[index] = state.colors.primary;
    state.prefs.swatches = state.colors.swatches.clone();
}

/// Draws the colour picker, the hex field and the swatches; `square` is the side of the picker's saturation/value
/// square and the width of its bars (the side panel fits it to the room of the Color page). Returns the side used.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, square: f32) -> f32 {
    ui.spacing_mut().slider_width = square;
    let mut c = to_color32(state.colors.primary);
    if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
        let [r, g, b, _] = from_color32(c);
        state.colors.primary = [r, g, b, 255];
    }

    ui.horizontal(|ui| {
        ui.label("Hex");
        let mut text = ps.hex.clone().unwrap_or_else(|| hex_of(state.colors.primary));
        let resp = ui.add(egui::TextEdit::singleline(&mut text).id_salt("primary_hex").desired_width(80.0).char_limit(7));
        if resp.has_focus() {
            if let Some(c) = parse_hex(&text) {
                state.colors.primary = c;
            }
            ps.hex = Some(text);
        } else {
            if resp.lost_focus()
                && let Some(c) = parse_hex(&text)
            {
                state.colors.primary = c;
            }
            ps.hex = None;
        }
        let (rect, _) = ui.allocate_exact_size(vec2(36.0, 18.0), Sense::hover());
        ui.painter().rect_filled(rect, 2.0, to_color32(state.colors.primary));
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
    });

    ui.add_space(2.0);
    let spacing = 2.0;
    let side = ((ui.available_width() - spacing * (SWATCH_COLUMNS as f32 - 1.0)) / SWATCH_COLUMNS as f32).clamp(10.0, 22.0);
    let mut store: Option<usize> = None;
    for row in 0..SWATCH_COUNT.div_ceil(SWATCH_COLUMNS) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(spacing, spacing);
            for col in 0..SWATCH_COLUMNS {
                let index = row * SWATCH_COLUMNS + col;
                let Some(&swatch) = state.colors.swatches.get(index) else { break };
                let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
                paint_checker(ui.painter(), rect);
                ui.painter().rect_filled(rect, 0.0, to_color32(swatch));
                let current = swatch == state.colors.primary;
                let stroke = if current || resp.hovered() {
                    Stroke::new(1.5, Color32::WHITE)
                } else {
                    Stroke::new(1.0, Color32::from_gray(60))
                };
                ui.painter().rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
                let resp = resp.on_hover_text(format!("{}\nClick: use  |  Right-click: store the current colour", hex_of(swatch)));
                if resp.clicked() {
                    state.colors.primary = swatch;
                }
                if resp.secondary_clicked() {
                    store = Some(index);
                }
            }
        });
    }
    if let Some(index) = store {
        store_swatch(state, index);
    }
    square
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Prefs;

    #[test]
    fn storing_a_swatch_updates_the_palette_and_the_prefs() {
        let mut state = AppState::new(Prefs::default(), true);
        state.colors.primary = [10, 20, 30, 255];
        store_swatch(&mut state, 5);
        assert_eq!(state.colors.swatches[5], [10, 20, 30, 255]);
        assert_eq!(state.colors.swatches.len(), SWATCH_COUNT);
        assert_eq!(state.prefs.swatches, state.colors.swatches);
        store_swatch(&mut state, SWATCH_COUNT); // out of range: ignored
        assert_eq!(state.colors.swatches.len(), SWATCH_COUNT);
    }
}
