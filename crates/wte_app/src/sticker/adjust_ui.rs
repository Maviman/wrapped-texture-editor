// Owner: stickers P8 (STICKER_PLAN 3.2 "Adjustments", 2.4, 4.6; ARCHITECTURE 8.15). The Adjustments block of the Tool
// tab in the Sticker view: the active layer's adjustments in order (Brightness / Contrast, Hue / Saturation, Levels,
// Invert, Black & White, Posterize, Threshold), each with its sliders and Up / Down / Remove buttons, and the "Add
// adjustment" list. They are non-destructive (applied to the layer's content when it is drawn) and work on every
// layer kind. Each change is `StickerAction::SetLayerAdjustments`: one entry "Sticker Layer Adjustments" per drag or
// click (`text_block::value_edit`).

use eframe::egui;
use wte_core::{Adjustment, StickerLayerId};

use crate::action::AppAction;
use crate::panels::PanelState;
use crate::state::{AppState, PanelHit};
use crate::sticker::{StickerAction, edit_of, layered};

/// History label of an adjustment edit.
pub(crate) const ADJUST_LABEL: &str = "Sticker Layer Adjustments";
/// Most adjustments on one layer.
pub(crate) const MAX_ADJUSTMENTS: usize = 16;

/// The kinds the add list offers, with their neutral (or first) settings.
pub(crate) fn kinds() -> [(&'static str, Adjustment); 7] {
    [
        ("Brightness / Contrast", Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0 }),
        ("Hue / Saturation", Adjustment::HueSaturation { hue_deg: 0.0, saturation: 0.0, lightness: 0.0, colorize: false }),
        ("Levels", Adjustment::Levels { in_black: 0, in_white: 255, gamma: 1.0, out_black: 0, out_white: 255 }),
        ("Invert", Adjustment::Invert),
        ("Black & White", Adjustment::BlackWhite),
        ("Posterize", Adjustment::Posterize { levels: 4 }),
        ("Threshold", Adjustment::Threshold { level: 128 }),
    ]
}

/// The name of an adjustment.
pub(crate) fn name(a: &Adjustment) -> &'static str {
    match a {
        Adjustment::BrightnessContrast { .. } => "Brightness / Contrast",
        Adjustment::HueSaturation { .. } => "Hue / Saturation",
        Adjustment::Levels { .. } => "Levels",
        Adjustment::Invert => "Invert",
        Adjustment::BlackWhite => "Black & White",
        Adjustment::Posterize { .. } => "Posterize",
        Adjustment::Threshold { .. } => "Threshold",
    }
}

/// The sliders of one adjustment; the union of their responses.
fn sliders(ui: &mut egui::Ui, a: &mut Adjustment) -> Option<egui::Response> {
    let mut out: Option<egui::Response> = None;
    let mut add = |r: egui::Response| {
        out = Some(match out.take() {
            Some(o) => o.union(r),
            None => r,
        })
    };
    match a {
        Adjustment::BrightnessContrast { brightness, contrast } => {
            add(ui.add(egui::Slider::new(brightness, -100.0..=100.0).text("Brightness").max_decimals(0)));
            add(ui.add(egui::Slider::new(contrast, -100.0..=100.0).text("Contrast").max_decimals(0)));
        }
        Adjustment::HueSaturation { hue_deg, saturation, lightness, colorize } => {
            add(ui.add(egui::Slider::new(hue_deg, -180.0..=180.0).text("Hue").suffix(" deg").max_decimals(0)));
            let mut s = *saturation * 100.0;
            let r = ui.add(egui::Slider::new(&mut s, -100.0..=100.0).text("Saturation").max_decimals(0));
            if r.changed() {
                *saturation = s / 100.0;
            }
            add(r);
            let mut l = *lightness * 100.0;
            let r = ui.add(egui::Slider::new(&mut l, -100.0..=100.0).text("Lightness").max_decimals(0));
            if r.changed() {
                *lightness = l / 100.0;
            }
            add(r);
            add(ui.checkbox(colorize, "Colorize"));
        }
        Adjustment::Levels { in_black, in_white, gamma, out_black, out_white } => {
            add(ui.add(egui::Slider::new(in_black, 0..=254).text("Input black")));
            add(ui.add(egui::Slider::new(in_white, 1..=255).text("Input white")));
            add(ui.add(egui::Slider::new(gamma, 0.1..=9.99).logarithmic(true).text("Midtones").max_decimals(2)));
            add(ui.add(egui::Slider::new(out_black, 0..=255).text("Output black")));
            add(ui.add(egui::Slider::new(out_white, 0..=255).text("Output white")));
        }
        Adjustment::Posterize { levels } => add(ui.add(egui::Slider::new(levels, 2..=32).text("Levels"))),
        Adjustment::Threshold { level } => add(ui.add(egui::Slider::new(level, 0..=255).text("Level"))),
        Adjustment::Invert | Adjustment::BlackWhite => {}
    }
    out
}

/// The Adjustments block for the active layer of the open editor.
pub(crate) fn block(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState, actions: &mut Vec<AppAction>) {
    let Some((doc, id)) = layered(state, None) else { return };
    if edit_of(state).is_none() {
        return;
    }
    let Some(layer) = doc.layer(id) else { return };
    let locked = layer.locked;
    let current = layer.adjustments.clone();
    let mut hits = Vec::new();
    ui.separator();
    let head = ui.label(egui::RichText::new("Adjustments").strong());
    hits.push((PanelHit::StickerAdjustments, head.rect));
    if current.is_empty() {
        ui.weak("None: add one below (they change the look, never the pixels).");
    }
    let make = |list: Vec<Adjustment>, commit| AppAction::Sticker(StickerAction::SetLayerAdjustments(id, list, commit));
    ui.add_enabled_ui(!locked, |ui| {
        let mut structural: Option<Vec<Adjustment>> = None;
        for (i, a) in current.iter().enumerate() {
            ui.push_id(("sticker_adjustment", i), |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name(a)).italics());
                    let up = ui.add_enabled(i > 0, egui::Button::new("Up").small()).on_hover_text("Apply this one earlier");
                    if up.clicked() {
                        let mut l = current.clone();
                        l.swap(i, i - 1);
                        structural = Some(l);
                    }
                    let down = ui.add_enabled(i + 1 < current.len(), egui::Button::new("Down").small()).on_hover_text("Apply this one later");
                    if down.clicked() {
                        let mut l = current.clone();
                        l.swap(i, i + 1);
                        structural = Some(l);
                    }
                    let remove = ui.add(egui::Button::new("Remove").small()).on_hover_text("Remove this adjustment");
                    hits.push((PanelHit::StickerAdjustRemove(i as u8), remove.rect));
                    if remove.clicked() {
                        let mut l = current.clone();
                        l.remove(i);
                        structural = Some(l);
                    }
                });
                let mut edited = a.clone();
                if let Some(resp) = sliders(ui, &mut edited) {
                    let changed = edited != *a;
                    let mut list = current.clone();
                    list[i] = edited;
                    crate::panels::text_block::value_edit(ps, actions, ui, &resp, changed, |commit| make(list.clone(), commit));
                }
            });
        }
        if let Some(list) = structural {
            actions.push(make(list, wte_core::Commit::Final));
        }
        let full = current.len() >= MAX_ADJUSTMENTS;
        let combo = egui::ComboBox::from_id_salt("sticker_add_adjustment").selected_text("Add adjustment...").show_ui(ui, |ui| {
            for (label, a) in kinds() {
                if ui.add_enabled(!full, egui::Button::selectable(false, label)).clicked() {
                    let mut l = current.clone();
                    l.push(a);
                    actions.push(make(l, wte_core::Commit::Final));
                }
            }
        });
        hits.push((PanelHit::StickerAdjustAdd, combo.response.rect));
    });
    state.panel_rects.extend(hits);
}

/// `SetLayerAdjustments`: the layer's list becomes `list` (sanitized, at most `MAX_ADJUSTMENTS`).
pub(crate) fn set(state: &mut AppState, id: StickerLayerId, mut list: Vec<Adjustment>, commit: wte_core::Commit) {
    list.truncate(MAX_ADJUSTMENTS);
    for a in &mut list {
        a.sanitize();
    }
    crate::sticker::layer_edit_pub(state, id, ADJUST_LABEL, commit, move |l| l.adjustments = list);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_name_and_the_add_list_starts_sanitized() {
        for (label, a) in kinds() {
            assert_eq!(name(&a), label);
            let mut s = a.clone();
            s.sanitize();
            assert_eq!(s, a, "{label} starts sanitized");
        }
        assert!(kinds()[0].1.is_identity() && kinds()[1].1.is_identity() && kinds()[2].1.is_identity());
    }
}
