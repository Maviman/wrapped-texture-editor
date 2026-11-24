// Owner: app-panels (A4). The Texture sets page of the side panel (ARCHITECTURE.md 8.7): one row per set with
// thumbnail (`doc.set_thumbnail`), name, resolution and UV coverage; a click makes the set active (`SetActiveSet`).

use eframe::egui;
use egui::{RichText, Sense, vec2};

use crate::action::AppAction;
use crate::panels::{PanelState, SET_THUMB_SIDE, paint_thumb};
use crate::state::AppState;

/// "1024 x 1024 | UV coverage 63 %".
pub(crate) fn set_details(width: u32, height: u32, coverage: f32) -> String {
    format!("{width} x {height}  |  UV coverage {:.0} %", (coverage * 100.0).clamp(0.0, 100.0))
}

/// Draws the Texture sets page (objects-tabs WP4: the tab is the section, no collapsing header): the list of texture
/// sets with thumbnails.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState) {
    let Some(session) = state.session.as_ref() else {
        ui.weak("No document open");
        return;
    };
    let doc = &session.doc;
    let mut actions = Vec::new();
    for set in &doc.sets {
        let active = set.id == session.active_set;
        let width = ui.available_width();
        let row_height = SET_THUMB_SIDE + 6.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(width, row_height), Sense::click());
        let visuals = ui.visuals();
        if active {
            ui.painter().rect_filled(rect, 4.0, visuals.selection.bg_fill.linear_multiply(0.6));
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
        }
        let thumb_rect = egui::Rect::from_min_size(rect.min + vec2(3.0, 3.0), vec2(SET_THUMB_SIDE, SET_THUMB_SIDE));
        let tex = ps.set_thumb(ui.ctx(), doc, set.id);
        paint_thumb(ui.painter(), thumb_rect, tex, set.width as f32 / set.height.max(1) as f32);

        let text_x = thumb_rect.max.x + 8.0;
        let name_color = if active { visuals.strong_text_color() } else { visuals.text_color() };
        ui.painter().text(
            egui::pos2(text_x, rect.min.y + 4.0),
            egui::Align2::LEFT_TOP,
            &set.name,
            egui::FontId::proportional(14.0),
            name_color,
        );
        ui.painter().text(
            egui::pos2(text_x, rect.min.y + 22.0),
            egui::Align2::LEFT_TOP,
            set_details(set.width, set.height, set.texel_map.stats.coverage),
            egui::FontId::proportional(11.0),
            visuals.weak_text_color(),
        );
        let hover = format!("{}\n{}", set.name, if active { "Active texture set" } else { "Click to make it the active set" });
        if resp.on_hover_text(RichText::new(hover)).clicked() && !active {
            actions.push(AppAction::SetActiveSet(set.id));
        }
    }
    state.actions.extend(actions);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_are_rounded_percentages() {
        assert_eq!(set_details(1024, 512, 0.634), "1024 x 512  |  UV coverage 63 %");
        assert_eq!(set_details(8, 8, 1.2), "8 x 8  |  UV coverage 100 %");
    }
}
