// Owner: app-dialogs (A5). About and Shortcuts dialogs of ARCHITECTURE.md 8.8: static content; the shortcut list is
// generated from `shortcuts::table()`.

use eframe::egui;

use crate::action::AppAction;
use crate::shortcuts::{self, ShortcutDef};
use crate::state::Dialog;
use crate::view::ViewCx;

/// Mouse and held-key navigation that is not part of the shortcut table (8.10).
const MOUSE_ROWS: [(&str, &str); 5] = [
    ("Orbit (3D) / pan (2D)", "Right drag; left drag with Navigate"),
    ("Pan", "Middle drag, Space + left drag"),
    ("Zoom", "Mouse wheel"),
    ("Stamp size (Stamp, Text, Transform)", "Ctrl + wheel"),
    ("Stamp rotation (Stamp, Text, Transform)", "Shift + wheel"),
];

/// One row per action label, in table order; a label bound to several keys (Redo) lists them all.
pub(super) fn shortcut_rows(defs: &[ShortcutDef], key_text: impl Fn(&ShortcutDef) -> String) -> Vec<(&'static str, String)> {
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    for def in defs {
        let keys = key_text(def);
        match rows.iter_mut().find(|(label, _)| *label == def.action_label) {
            Some((_, text)) => {
                text.push_str(", ");
                text.push_str(&keys);
            }
            None => rows.push((def.action_label, keys)),
        }
    }
    rows
}

/// The key text of one entry as menus show it: the display override, else egui's formatting of the shortcut.
fn key_text(ctx: &egui::Context, def: &ShortcutDef) -> String {
    def.display.map_or_else(|| ctx.format_shortcut(&def.shortcut), str::to_owned)
}

/// Draws the About dialog.
pub(super) fn show_about(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let response = super::modal("about").show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.vertical_centered(|ui| {
            ui.heading(crate::io_flows::APP_TITLE);
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
        });
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                "Import a model with its texture, see how the texture wraps around it, and paint on the texture \
                 directly on the model in 3D or in the 2D UV view. Images and text are placed as stamps on separate \
                 layers that export as separate files. Projects (.wtep) contain everything, including the original \
                 model and texture files.",
            )
            .wrap(),
        );
        ui.add_space(6.0);
        ui.weak("Formats: OBJ (+MTL), glTF, GLB in; PNG, JPEG, TGA, BMP, TIFF, WebP, OpenRaster and PSD out.");
        ui.add_space(8.0);
        let mut action = None;
        super::button_row(ui, |ui| {
            if ui.button("Close").clicked() {
                action = Some(AppAction::CloseDialog);
            }
            if ui.button("Keyboard Shortcuts...").clicked() {
                action = Some(AppAction::ShowDialog(Dialog::Shortcuts));
            }
        });
        action
    });
    if let Some(action) = response.inner {
        cx.state.push(action);
    } else if super::backdrop_clicked(&response) {
        super::close(cx);
    }
}

/// Draws the shortcut list (generated from `shortcuts::table()`).
pub(super) fn show_shortcuts(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let rows = shortcut_rows(shortcuts::table(), |def| key_text(ctx, def));
    let max = super::max_dialog_size(ctx);
    let response = super::modal("shortcuts").show(ctx, |ui| {
        ui.set_width(max.x.min(440.0));
        ui.heading("Keyboard Shortcuts");
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("wte_shortcut_list")
            .max_height((max.y - 120.0).max(120.0))
            .show(ui, |ui| {
                egui::Grid::new("wte_shortcut_grid").num_columns(2).striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
                    for (label, keys) in &rows {
                        ui.label(*label);
                        ui.monospace(keys.as_str());
                        ui.end_row();
                    }
                    for (label, keys) in MOUSE_ROWS {
                        ui.label(label);
                        ui.label(keys);
                        ui.end_row();
                    }
                });
            });
        ui.separator();
        let mut close = false;
        super::button_row(ui, |ui| close = ui.button("Close").clicked());
        close
    });
    if response.inner || super::backdrop_clicked(&response) {
        super::close(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_rows_cover_every_label_once() {
        let defs = shortcuts::table();
        let rows = shortcut_rows(defs, |d| d.display.unwrap_or("key").to_owned());
        for def in defs {
            assert_eq!(rows.iter().filter(|(l, _)| *l == def.action_label).count(), 1, "{}", def.action_label);
        }
        // Redo has two bindings, listed in one row.
        let redo = rows.iter().find(|(l, _)| *l == "Redo").expect("Redo row");
        assert_eq!(redo.1, "key, key");
        let softer = rows.iter().find(|(l, _)| *l == "Softer Brush").expect("Softer Brush row");
        assert_eq!(softer.1, "Shift+[");
        assert_eq!(rows.len(), defs.len() - 1);
    }
}
