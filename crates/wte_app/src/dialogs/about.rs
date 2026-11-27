// Owner: app-dialogs (A5). About and Shortcuts dialogs of ARCHITECTURE.md 8.8: static content; the shortcut list is
// generated from `shortcuts::table()` (and, stickers P13, the Sticker view's keys from `shortcuts::sticker_table()`).
// Stickers P13 (STICKER_PLAN 4.0, 13.8 D6): About > Licences... - the MIT notice of the PhotoCraft code WTE adapts,
// word for word from `licenses/photocraft-MIT.txt`, and the trademark sentence.

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

/// Stickers P13 (STICKER_PLAN 13.8 D6): the one trademark sentence (About > Licences and Help > Working with PhotoCraft
/// and Photoshop). The only user-facing text that names the company behind Photoshop.
pub(crate) const TRADEMARK: &str = "Adobe and Photoshop are either registered trademarks or trademarks of Adobe in the United \
                                    States and/or other countries. PhotoCraft belongs to its authors. Wrapped Texture \
                                    Editor is not affiliated with or endorsed by either.";

/// The licence file the installer puts into `{app}\licenses` and About > Licences... shows, word for word.
pub(crate) const PHOTOCRAFT_MIT: &str = include_str!("../../../../licenses/photocraft-MIT.txt");

/// The line above the licence text: what it covers and where the file is.
pub(crate) const LICENCES_INTRO: &str = "Parts of the sticker editor (layer styles, selections, cut-out, adjustments and \
                                         shapes) are adapted from PhotoCraft's source code, under the MIT licence below. \
                                         The same text is in the file licenses\\photocraft-MIT.txt in the program's \
                                         folder.";

/// The Sticker view's keys whose table label is not a command name (`shortcuts::sticker_table()`): what they do.
const STICKER_KEY_TEXT: [(&str, &str); 3] = [
    ("Sticker Delete", "Clear the selected pixels"),
    ("Sticker Esc", "Cancel; drop the selection; else leave the view"),
    ("Sticker Enter", "Apply the crop; close the lasso"),
];

/// The row label of a Sticker view key (the table label, or what the key does for the three plain keys).
fn sticker_label(label: &'static str) -> &'static str {
    STICKER_KEY_TEXT.iter().find(|(l, _)| *l == label).map_or(label, |(_, t)| t)
}

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
            // Stickers P13 (STICKER_PLAN 4.0): the licence of the adapted code and the trademark sentence.
            if ui.button("Licences...").clicked() {
                action = Some(AppAction::ShowDialog(Dialog::Licences));
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

/// Draws About > Licences...: the MIT notice of the adapted PhotoCraft code (scrolling, selectable) and the trademark
/// sentence. Back returns to About.
pub(super) fn show_licences(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let max = super::max_dialog_size(ctx);
    let response = super::modal("licences").show(ctx, |ui| {
        ui.set_width(max.x.min(520.0));
        ui.heading("Licences");
        ui.separator();
        egui::ScrollArea::vertical().id_salt("wte_licences").max_height((max.y - 140.0).max(120.0)).show(ui, |ui| {
            ui.add(egui::Label::new(LICENCES_INTRO).wrap());
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(PHOTOCRAFT_MIT).monospace()).wrap().selectable(true));
            ui.add_space(8.0);
            ui.add(egui::Label::new(TRADEMARK).wrap());
        });
        ui.separator();
        let mut action = None;
        super::button_row(ui, |ui| {
            if ui.button("Close").clicked() {
                action = Some(AppAction::CloseDialog);
            }
            if ui.button("Back").clicked() {
                action = Some(AppAction::ShowDialog(Dialog::About));
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

/// Draws the shortcut list (generated from `shortcuts::table()`; stickers P13: then the Sticker view's keys).
pub(super) fn show_shortcuts(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let rows = shortcut_rows(shortcuts::table(), |def| key_text(ctx, def));
    let sticker_rows = shortcut_rows(shortcuts::sticker_table(), |def| key_text(ctx, def));
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
                // Stickers P13 (STICKER_PLAN 3.3, hand test 18): the keys of the Sticker view, which win there.
                ui.add_space(8.0);
                ui.strong("In the Sticker view (key 4)");
                ui.weak("These keys win over the ones above while the Sticker view is open; S and R do nothing there.");
                egui::Grid::new("wte_sticker_shortcut_grid").num_columns(2).striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
                    for (label, keys) in &sticker_rows {
                        ui.label(sticker_label(label));
                        ui.monospace(keys.as_str());
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

    #[test]
    fn the_sticker_key_rows_say_what_the_plain_keys_do() {
        let rows = shortcut_rows(shortcuts::sticker_table(), |d| d.display.unwrap_or("key").to_owned());
        // Delete and Backspace share one row.
        let delete = rows.iter().find(|(l, _)| *l == "Sticker Delete").expect("Delete row");
        assert_eq!(delete.1, "key, key");
        for (label, _) in &rows {
            assert!(!sticker_label(label).starts_with("Sticker "), "{label} has no text for the Shortcuts dialog");
        }
        assert_eq!(sticker_label("Text"), "Text");
    }

    #[test]
    fn the_licences_window_shows_the_mit_notice_word_for_word_and_the_trademark_sentence() {
        // Stickers P13 (STICKER_PLAN 4.0, 13.8 D6): the file's text, unchanged, with its copyright line.
        let file = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../licenses/photocraft-MIT.txt")).unwrap();
        assert_eq!(PHOTOCRAFT_MIT, file);
        assert!(PHOTOCRAFT_MIT.starts_with("MIT License"));
        assert!(PHOTOCRAFT_MIT.contains("and the PhotoCraft contributors"));
        assert!(PHOTOCRAFT_MIT.contains("THE SOFTWARE IS PROVIDED \"AS IS\""));
        assert_eq!(
            TRADEMARK,
            "Adobe and Photoshop are either registered trademarks or trademarks of Adobe in the United States and/or other \
             countries. PhotoCraft belongs to its authors. Wrapped Texture Editor is not affiliated with or endorsed by either."
        );
        assert!(LICENCES_INTRO.contains(r"licenses\photocraft-MIT.txt"));
    }

    #[test]
    fn the_version_is_0_4_0() {
        // Stickers P13: the workspace version (About, the crash report, --gpu-check, the project manifest).
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.4.0");
    }
}
