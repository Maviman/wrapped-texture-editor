// Owner: app-panels (A4). The toolbar of ARCHITECTURE.md 8.7: vector-painted tool buttons (`theme::tool_icon`) with
// shortcut tooltips, and below them the primary / secondary colour chip with swap and reset.

use eframe::egui;
use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};

use crate::action::AppAction;
use crate::panels::{paint_checker, to_color32};
use crate::shortcuts::shortcut_text;
use crate::state::{AppState, ToolKind};
use crate::theme;

/// The tools in toolbar order with their `shortcut_text` labels (which are also the tooltip names).
pub(crate) const TOOLS: [(ToolKind, &str); 8] = [
    (ToolKind::Navigate, "Navigate"),
    (ToolKind::Brush, "Brush"),
    (ToolKind::Eraser, "Eraser"),
    (ToolKind::Eyedropper, "Eyedropper"),
    (ToolKind::Fill, "Fill"),
    (ToolKind::Stamp, "Stamp"),
    (ToolKind::Text, "Text"),
    (ToolKind::Transform, "Transform"),
];

/// The display name of a tool.
pub(crate) fn tool_name(tool: ToolKind) -> &'static str {
    TOOLS.iter().find(|(t, _)| *t == tool).map_or("Tool", |(_, name)| name)
}

/// "Name (Key)" for a tooltip.
fn with_key(ctx: &egui::Context, text: &str, label: &str) -> String {
    let key = shortcut_text(ctx, label);
    if key.is_empty() { text.to_owned() } else { format!("{text} ({key})") }
}

/// Draws the tool buttons and the primary / secondary colour chip.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let mut actions = Vec::new();
    ui.vertical_centered(|ui| {
        ui.add_space(4.0);
        let side = ui.available_width().clamp(24.0, 34.0);
        for (tool, label) in TOOLS {
            let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
            let selected = state.tool == tool;
            let visuals = ui.visuals();
            let (fill, fg) = if selected {
                (theme::ACCENT, Color32::WHITE)
            } else if resp.hovered() {
                (visuals.widgets.hovered.weak_bg_fill, visuals.widgets.hovered.fg_stroke.color)
            } else {
                (Color32::TRANSPARENT, visuals.widgets.inactive.fg_stroke.color)
            };
            ui.painter().rect_filled(rect, 4.0, fill);
            theme::tool_icon(ui.painter(), rect.shrink(side * 0.17), tool, fg);
            if resp.on_hover_text(with_key(ui.ctx(), label, label)).clicked() && !selected {
                actions.push(AppAction::SetTool(tool));
            }
        }
        ui.add_space(10.0);
        color_chip(ui, state, side, &mut actions);
    });
    state.actions.extend(actions);
}

/// Primary colour (top left) over the secondary colour (bottom right); clicking the secondary swaps them. Below:
/// a swap arrow and a black/white reset chip.
fn color_chip(ui: &mut egui::Ui, state: &AppState, side: f32, actions: &mut Vec<AppAction>) {
    let (area, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let chip = (side * 0.62).round();
    let primary = Rect::from_min_size(area.min, vec2(chip, chip));
    let secondary = Rect::from_min_size(area.max - vec2(chip, chip), vec2(chip, chip));
    let outline = Stroke::new(1.0, Color32::from_gray(20));
    let highlight = Stroke::new(1.0, Color32::from_gray(220));

    let sec_resp = ui.interact(secondary, egui::Id::unique("wte_secondary_chip"), Sense::click());
    paint_checker(ui.painter(), secondary);
    ui.painter().rect_filled(secondary, 0.0, to_color32(state.colors.secondary));
    ui.painter().rect_stroke(secondary, 0.0, outline, egui::StrokeKind::Inside);
    if sec_resp.on_hover_text(with_key(ui.ctx(), "Secondary colour: click to swap", "Swap Colors")).clicked() {
        actions.push(AppAction::SwapColors);
    }

    let pri_resp = ui.interact(primary, egui::Id::unique("wte_primary_chip"), Sense::hover());
    paint_checker(ui.painter(), primary);
    ui.painter().rect_filled(primary, 0.0, to_color32(state.colors.primary));
    ui.painter().rect_stroke(primary, 0.0, outline, egui::StrokeKind::Inside);
    ui.painter().rect_stroke(primary.shrink(1.0), 0.0, highlight, egui::StrokeKind::Inside);
    pri_resp.on_hover_text("Primary colour (edit it in the Color section)");

    ui.add_space(4.0);
    let small = (side * 0.45).round();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(small, small), Sense::click());
        paint_swap_icon(ui.painter(), rect, ui.visuals().widgets.inactive.fg_stroke.color, resp.hovered());
        if resp.on_hover_text(with_key(ui.ctx(), "Swap colours", "Swap Colors")).clicked() {
            actions.push(AppAction::SwapColors);
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(small, small), Sense::click());
        let r = rect.shrink(small * 0.15);
        let half = r.width() * 0.6;
        let back = Rect::from_min_size(r.max - vec2(half, half), vec2(half, half));
        let front = Rect::from_min_size(r.min, vec2(half, half));
        ui.painter().rect_filled(back, 0.0, Color32::WHITE);
        ui.painter().rect_stroke(back, 0.0, outline, egui::StrokeKind::Inside);
        ui.painter().rect_filled(front, 0.0, Color32::BLACK);
        ui.painter().rect_stroke(front, 0.0, Stroke::new(1.0, Color32::from_gray(160)), egui::StrokeKind::Inside);
        if resp.hovered() {
            ui.painter().rect_stroke(rect, 2.0, highlight, egui::StrokeKind::Inside);
        }
        if resp.on_hover_text(with_key(ui.ctx(), "Reset colours to black and white", "Reset Colors")).clicked() {
            actions.push(AppAction::ResetColors);
        }
    });
}

/// A bent two-headed arrow (top right <-> bottom left).
fn paint_swap_icon(painter: &egui::Painter, rect: Rect, color: Color32, hovered: bool) {
    if hovered {
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(220)), egui::StrokeKind::Inside);
    }
    let r = rect.shrink(rect.width() * 0.2);
    let stroke = Stroke::new(1.5, color);
    let corner = pos2(r.max.x, r.max.y);
    let a = pos2(r.min.x + r.width() * 0.25, r.max.y);
    let b = pos2(r.max.x, r.min.y + r.height() * 0.25);
    painter.line_segment([a, corner], stroke);
    painter.line_segment([corner, b], stroke);
    let head = r.width() * 0.25;
    painter.line_segment([a, a + vec2(head, -head)], stroke);
    painter.line_segment([a, a + vec2(head, head)], stroke);
    painter.line_segment([b, b + vec2(-head, head)], stroke);
    painter.line_segment([b, b + vec2(head, head)], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_shortcut_label() {
        let ctx = egui::Context::default();
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |_| {});
        for (tool, label) in TOOLS {
            assert!(!shortcut_text(&ctx, label).is_empty(), "{tool:?} has no shortcut");
            assert_eq!(tool_name(tool), label);
        }
    }
}
