// Stickers P13 (STICKER_PLAN 7 "P13", 5.7, 6.4, 6.5, 13.2, 13.5, 13.8 D6, 13.9 "P13"; ARCHITECTURE.md 8.8): Help >
// Working with PhotoCraft and Photoshop. Static help: how the link works and what comes back, what is lost, the limits of
// the "already open" guard, no drag from either editor, the Photoshop table "In Photoshop you... / In WTE", the
// Photoshop settings (Maximize Compatibility = Always, "Leave as is" for colour profiles, Export Clipboard), drops on
// Photoshop's tab bar, the administrator note, drag and drop / copy and paste, and the trademark sentence. The texts name
// the editors as plain words only (STICKER_PLAN 0.4 rule 9); the company behind Photoshop is named only in the trademark
// sentence.

use eframe::egui;

use crate::view::ViewCx;

/// "Working with PhotoCraft" (and any editor: the link works the same with each).
pub(crate) const LINK_HELP: [&str; 9] = [
    "Select a sticker or an image stamp and click \"Edit in <editor>\" in the Sticker tab (also in the Sticker view's header, \
     the Layer menu and a stamp layer's right-click menu; \"Edit in...\" beside it lists every editor). The project must be \
     saved first: the linked PSD file lives next to it, in the folder <project>_links. Press Ctrl+S in the editor and \
     within about a second the sticker on the model updates by itself, in place.",
    "Moving, scaling or turning a sticker on the model in WTE never touches the file or the link. Only \"Edit in\" and \
     \"Send to\" write the file; saving the project never rewrites it (the project keeps a copy of it, and puts it back \
     if the file is deleted).",
    "What comes back: every layer, one by one, with its name, visibility, opacity, blend mode (Normal, Multiply, Screen, \
     Overlay, Darken, Lighten, Add), mask and clipping. WTE's own text and shape layers you did not change in the editor \
     stay text and shapes.",
    "What is lost: WTE's layer styles and stamp styles are not written into the file (WTE adds them again to what comes \
     back). A WTE text or shape layer you change in the editor comes back as pixels. The editor's text, smart and fill \
     layers come back as pictures. Layer effects, adjustment layers, groups, other blend modes and vector masks make the \
     whole sticker come back as one finished picture, with a note that names the reason (Undo in WTE brings the layers \
     back). 16-bit files come back as 8-bit. Parts outside the canvas are cut off.",
    "If both WTE and the editor changed a sticker, WTE asks which version to keep. A save in the editor never brings WTE \
     to the front, flashes its taskbar button or plays a sound: questions wait inside WTE's window. While you paint or \
     type in the Sticker view, an update waits until you let go.",
    "The \"already open\" question: clicking \"Edit in PhotoCraft\" a second time asks before starting a second copy only \
     with PhotoCraft v0.2.0 and portable copies, which keep one program per window. The installed PhotoCraft hands the \
     file to its running window and closes at once, so WTE cannot know the file is open: a second click simply starts \
     it again (it may open a second tab or bring the first one forward).",
    "PhotoCraft's latest version: turn on Preferences > File Handling > Maximize PSD and PSB File Compatibility. Without \
     it a file that needs the finished picture (effects, groups) is not applied and the badge says what to turn on; \
     \"Convert with PhotoCraft's command-line tool\" is offered when photocraft-cli.exe is next to photocraft.exe.",
    "A texture paint layer can be linked the same way: Layer > \"Edit Layer in <editor>\" (or the layer's right-click \
     menu). Its opacity and blend mode stay in WTE, and the editor's canvas must keep the texture's size.",
    "There is no drag from PhotoCraft into WTE: PhotoCraft cannot start a drag. Use Ctrl+S on a linked sticker or layer, \
     Ctrl+C there and Ctrl+V in WTE, or drag the saved file from Explorer.",
];

/// The table "In Photoshop you... / In WTE" (STICKER_PLAN 13.2).
pub(crate) const PHOTOSHOP_TABLE: [(&str, &str); 8] = [
    (
        "paint on a layer, add a layer, paste, move, rename, reorder, hide, change opacity or one of the seven blend modes, \
         add or paint a layer mask, clip a layer",
        "comes back layer by layer; untouched WTE layers stay text, shape or live",
    ),
    ("add an empty layer", "an empty picture layer; everything else stays as it was"),
    ("add a text layer", "a new picture layer (pixels); still text in the PSD"),
    (
        "add a shape or a fill layer",
        "a new picture layer when Photoshop stored its pixels: an 8-bit document with Maximize Compatibility on. In a \
         16-bit document the sticker comes back as one finished picture; with Maximize Compatibility off, the badge says \
         what to turn on",
    ),
    (
        "add a smart object (also File > Place Embedded), a layer style (stroke, shadow, glow...), an adjustment layer, a \
         group or artboard, a vector mask on a pixel layer, mask feather or density, Fill below 100 %, another blend mode",
        "the sticker comes back as one finished picture (needs Maximize Compatibility on); Undo in WTE restores the layers",
    ),
    ("flatten the image", "one picture layer"),
    ("switch to 16 bits", "works, reduced to 8"),
    ("switch to CMYK, Lab, Indexed, 32 bits...", "nothing is applied; the badge says how to switch back"),
];

/// "Working with Photoshop": the settings and what to expect.
pub(crate) const PHOTOSHOP_HELP: [&str; 8] = [
    "Edit in... > Photoshop opens the linked file as a tab in the running Photoshop (WTE finds Photoshop by itself; Help > \
     Editor locations... shows where). Ctrl+S there updates the sticker in WTE. Clicking it again asks nothing: \
     Photoshop is expected to show the tab that is already open.",
    "Set Edit > Preferences > File Handling > \"Maximize PSD and PSB File Compatibility\" to Always. Layer styles, \
     adjustment layers and smart objects come back only with it on, and with \"Ask\" Photoshop asks at every save of a \
     layered file (if it asks, leave the box ticked). With it off WTE does not update and the badge says \"needs a merged \
     picture\" with the same advice.",
    "If Photoshop asks about a colour profile when it opens a sticker, choose \"Leave as is\" (or Assign sRGB), not \
     Convert: a conversion changes every pixel, and then every layer comes back as a new picture. WTE reads the numbers as \
     sRGB; a file with another profile is applied with one note. Grayscale files are read as they are.",
    "Dragging a sticker out of WTE onto an open Photoshop picture places it as a layer (press Enter); onto Photoshop's tab \
     bar (or the empty workspace) it opens as its own document. The \"PSD\" handle under the sticker's picture in the \
     Sticker tab stays linked only when you drop it on the tab bar; dropped on a picture, Photoshop places a copy that is \
     not linked.",
    "Copy and paste: Ctrl+C in Photoshop reaches WTE only with Edit > Preferences > General > \"Export Clipboard\" on (its \
     default). Transparency may arrive as white or black; for a cut-out use the link or a saved PNG. Help > Clipboard \
     contents... lists what the clipboard offers.",
    "If Photoshop runs \"as administrator\" and WTE does not, Windows blocks drags and drops between them. \"Edit in \
     Photoshop\" still works.",
    "16 bits per channel works (reduced to 8). CMYK, Lab, Indexed, Duotone, Multichannel, Bitmap and 32-bit files are \
     not applied; the badge says what to choose in Photoshop (Image > Mode).",
    "There is no drag from Photoshop into WTE: Photoshop cannot drag a layer or a document out to another program. Use \
     Ctrl+S on a linked sticker or layer, copy and paste, or drag the saved file from Explorer.",
];

/// "Drag, drop, copy, paste".
pub(crate) const DND_HELP: [&str; 4] = [
    "Drop pictures from Explorer or a web browser onto the model, the 2D view, the sticker canvas or a stamp's row in the \
     Layers list: they land where you let go (one Undo step). A .wtes sticker file or a .psd becomes a sticker; several \
     pictures at once: the first is placed, the rest go to the Sticker tab's tray.",
    "Drag a Layers row, the Sticker tab's picture or a tray sticker out of the window: a PNG file of what you see, styles \
     included (written to %LOCALAPPDATA%\\WrappedTextureEditor\\dragout; WTE removes only its own unchanged files there, \
     a day later).",
    "Drag out with the mouse: Windows may end a drag made with a pen at once. The tablet companion cannot drag out; use \
     Copy, Export Picture... or \"Edit in\" there.",
    "Ctrl+C on a stamp copies its finished picture; Ctrl+V gives the picture to the Stamp tool (a click places it), and \
     Edit > Paste at View Centre places it at once. In the Sticker view Ctrl+C / Ctrl+V copy and paste the selected \
     pixels as a new layer.",
];

/// Draws Help > Working with PhotoCraft and Photoshop.
pub(super) fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>) {
    let max = super::max_dialog_size(ctx);
    let response = super::modal("editor_help").show(ctx, |ui| {
        ui.set_width(max.x.min(560.0));
        ui.heading("Working with PhotoCraft and Photoshop");
        ui.separator();
        egui::ScrollArea::vertical().id_salt("wte_editor_help").max_height((max.y - 120.0).max(120.0)).show(ui, |ui| {
            ui.strong("Working with PhotoCraft (and any editor)");
            for text in LINK_HELP {
                ui.add(egui::Label::new(text).wrap());
                ui.add_space(4.0);
            }
            ui.add_space(6.0);
            ui.strong("Working with Photoshop");
            egui::Grid::new("wte_photoshop_table").num_columns(2).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                let col = (ui.available_width() / 2.0 - 12.0).max(120.0);
                ui.strong("In Photoshop you...");
                ui.strong("In WTE");
                ui.end_row();
                for (left, right) in PHOTOSHOP_TABLE {
                    ui.add_sized([col, 0.0], egui::Label::new(left).wrap());
                    ui.add_sized([col, 0.0], egui::Label::new(right).wrap());
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            for text in PHOTOSHOP_HELP {
                ui.add(egui::Label::new(text).wrap());
                ui.add_space(4.0);
            }
            ui.add_space(6.0);
            ui.strong("Drag, drop, copy, paste");
            for text in DND_HELP {
                ui.add(egui::Label::new(text).wrap());
                ui.add_space(4.0);
            }
            ui.add_space(6.0);
            ui.weak(super::about::TRADEMARK);
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

    fn all_text() -> String {
        let mut t: Vec<&str> = LINK_HELP.to_vec();
        t.extend(PHOTOSHOP_TABLE.iter().flat_map(|(a, b)| [*a, *b]));
        t.extend(PHOTOSHOP_HELP);
        t.extend(DND_HELP);
        t.join("\n")
    }

    #[test]
    fn the_help_says_what_the_plan_asks_for() {
        let t = all_text();
        // STICKER_PLAN 5.7: the guard's limits, said exactly.
        assert!(t.contains("only with PhotoCraft v0.2.0 and portable copies"));
        assert!(t.contains("WTE cannot know the file is open"));
        // 0.2, 13.5: no drag from either editor.
        assert!(t.contains("There is no drag from PhotoCraft into WTE"));
        assert!(t.contains("There is no drag from Photoshop into WTE"));
        // 13.9 "P13": Maximize Compatibility = Always, "Leave as is", the tab bar, Export Clipboard, the administrator.
        assert!(t.contains("\"Maximize PSD and PSB File Compatibility\" to Always"));
        assert!(t.contains("choose \"Leave as is\" (or Assign sRGB), not Convert"));
        assert!(t.contains("stays linked only when you drop it on the tab bar"));
        assert!(t.contains("\"Export Clipboard\" on"));
        assert!(t.contains("runs \"as administrator\""));
        // 6.4: the pen and the companion.
        assert!(t.contains("Drag out with the mouse"));
        // The user's workflow: placement edits never touch the link.
        assert!(t.contains("never touches the file or the link"));
        // 5.4 / 13.2: what is lost and the grayscale exemption.
        assert!(t.contains("What is lost"));
        assert!(t.contains("Grayscale files are read as they are"));
    }

    #[test]
    fn the_photoshop_table_is_the_plans_table() {
        // STICKER_PLAN 13.2 "What this means for the user": eight rows, the left column in the plan's order.
        let left: Vec<String> = PHOTOSHOP_TABLE.iter().map(|(a, _)| a.split(' ').take(2).collect::<Vec<_>>().join(" ")).collect();
        assert_eq!(left, ["paint on", "add an", "add a", "add a", "add a", "flatten the", "switch to", "switch to"]);
        assert!(PHOTOSHOP_TABLE[7].1.contains("says how to switch back"));
    }

    #[test]
    fn the_company_is_named_only_in_the_trademark_sentence() {
        // STICKER_PLAN 0.4 rule 9, 8.6 item 7: the help names Photoshop as plain text; the company only in the trademark
        // sentence; no other program name than the two editors.
        // The names are taken from the trademark sentence and the licence file, so this test does not spell them.
        let company = super::super::about::TRADEMARK.split_whitespace().next().unwrap();
        assert_eq!(company.len(), 5);
        assert!(!all_text().contains(company));
        let holder = super::super::about::PHOTOCRAFT_MIT.lines().find_map(|l| l.strip_prefix("Copyright (c) 2026 ")).unwrap();
        let team = holder.split(" Team").next().unwrap();
        assert!(!team.is_empty() && !all_text().contains(team));
    }
}
