// Owner: stickers P9b (STICKER_PLAN 13.1; ARCHITECTURE 8.8, 8.16). Help > Editor locations...: the programs of the
// outside editors a linked sticker opens in (Photoshop, PhotoCraft, another program), each with Browse... / Clear, and
// the default editor. Opening it runs the registry discovery again (read only, `link::editor::ensure_discovered`).
// Changes are actions (`LinkAction::LocateEditor`, `ClearEditor`, `SetDefaultEditor`); nothing here starts a program.

use eframe::egui;
use wte_core::link::{EDITOR_OTHER, EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP};

use crate::action::AppAction;
use crate::link::LinkAction;
use crate::link::editor::{exe_pref, name_of_key};
use crate::state::PanelHit;
use crate::view::ViewCx;

/// The line under the rows (also checked by the tests).
pub(crate) const EDITORS_NOTE: &str = "WTE looks for Photoshop and PhotoCraft in the registry (it only reads it). The portable PhotoCraft \
     leaves no trace there: choose its photocraft.exe with Browse. Another program is started with the PSD file as its only argument.";

/// The rows, in list order: key, name, what an empty row says.
fn rows(other_name: &str) -> [(&'static str, String, &'static str); 3] {
    [
        (EDITOR_PHOTOSHOP, "Photoshop".to_owned(), "not found"),
        (EDITOR_PHOTOCRAFT, "PhotoCraft".to_owned(), "not found"),
        (EDITOR_OTHER, other_name.to_owned(), "none chosen"),
    ]
}

/// Draws Help > Editor locations....
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let max = super::max_dialog_size(ctx);
    let mut actions = Vec::new();
    let mut hits = Vec::new();
    let prefs = &cx.state.prefs;
    let other_name = if prefs.other_editor_exe.is_some() { name_of_key(EDITOR_OTHER, prefs.other_editor_exe.as_deref()) } else { "Other program".to_owned() };
    let rows = rows(&other_name);
    let paths: Vec<Option<String>> = rows.iter().map(|(k, ..)| exe_pref(prefs, k).map(|p| p.display().to_string())).collect();
    let default = prefs.default_editor.clone();
    let response = super::modal("editor_locations").show(ctx, |ui| {
        ui.set_width(max.x.min(520.0));
        ui.heading("Editor locations");
        ui.separator();
        egui::Grid::new("wte_editor_locations").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
            for (i, ((key, name, empty), path)) in rows.iter().zip(&paths).enumerate() {
                ui.strong(name);
                match path {
                    Some(p) => ui.add(egui::Label::new(egui::RichText::new(p).weak()).truncate()),
                    None => ui.weak(*empty),
                };
                ui.horizontal(|ui| {
                    let resp = ui.button("Browse...");
                    hits.push((PanelHit::EditorBrowse(i as u8), resp.rect));
                    if resp.clicked() {
                        actions.push(AppAction::Link(LinkAction::LocateEditor { editor: (*key).to_owned(), path: None }));
                    }
                    let resp = ui.add_enabled(path.is_some(), egui::Button::new("Clear"));
                    hits.push((PanelHit::EditorClear(i as u8), resp.rect));
                    if resp.clicked() {
                        actions.push(AppAction::Link(LinkAction::ClearEditor((*key).to_owned())));
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        ui.label("Default editor (\"Edit in\" on a sticker that is not linked yet):");
        ui.horizontal_wrapped(|ui| {
            let resp = ui.radio(default.is_none(), "Ask");
            hits.push((PanelHit::EditorDefault(0), resp.rect));
            if resp.clicked() {
                actions.push(AppAction::Link(LinkAction::SetDefaultEditor(None)));
            }
            for (i, ((key, name, _), path)) in rows.iter().zip(&paths).enumerate() {
                // An editor can be the default when its program is known; PhotoCraft also without (it can be located).
                if path.is_none() && *key != EDITOR_PHOTOCRAFT {
                    continue;
                }
                let resp = ui.radio(default.as_deref() == Some(*key), name.as_str());
                hits.push((PanelHit::EditorDefault(i as u8 + 1), resp.rect));
                if resp.clicked() {
                    actions.push(AppAction::Link(LinkAction::SetDefaultEditor(Some((*key).to_owned()))));
                }
            }
        });
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(EDITORS_NOTE).weak()).wrap());
        ui.separator();
        let mut close = false;
        super::button_row(ui, |ui| close = ui.button("Close").clicked());
        close
    });
    cx.state.panel_rects.extend(hits);
    cx.state.actions.extend(actions);
    if response.inner || super::backdrop_clicked(&response) {
        super::close(cx);
    }
}
