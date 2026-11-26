//! Every sentence the user sees about a link (STICKER_PLAN 5.1-5.5, 13.1, 13.2), each taking the editor's name as a
//! parameter: the core never writes an editor's name where the editor is variable. Only the sentences that describe
//! one editor's own menus (the MaxCompat sentence, 13.2) branch on the editor's key.

use crate::psd::PsdFeatures;

use super::{EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP};

/// The toast after an applied update: "Updated from <editor>".
pub fn updated(editor: &str) -> String {
    format!("Updated from {editor}")
}

/// The name of the one picture layer of a merged update: "<editor> picture".
pub fn merged_layer_name(editor: &str) -> String {
    format!("{editor} picture")
}

/// "a", "a and b", "a, b and c".
fn joined(words: &[&str]) -> String {
    match words {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The toast of a merged update (5.4): "This sticker now uses layer effects, which only <editor> can show. WTE shows
/// the finished picture; keep editing its layers in <editor>." `extra` names reasons that are not PSD features
/// ("more than 32 layers").
pub fn merged_reason(features: PsdFeatures, extra: &[&str], editor: &str) -> String {
    let mut words = features.words();
    words.extend_from_slice(extra);
    format!(
        "This sticker now uses {}, which only {editor} can show. WTE shows the finished picture; keep editing its layers in {editor}.",
        joined(&words)
    )
}

/// The sticky "needs a merged picture" message (5.4, 13.2), worded for the editor that saved the file.
pub fn needs_merged(editor_key: &str, editor: &str) -> String {
    match editor_key {
        EDITOR_PHOTOCRAFT => format!(
            "{editor} saved this file without a merged picture. In {editor} turn on Preferences > File Handling > Maximize PSD and PSB File Compatibility and save again."
        ),
        EDITOR_PHOTOSHOP => format!(
            "{editor} saved this file without a merged picture. In {editor} choose Edit > Preferences > File Handling, set \"Maximize PSD and PSB File Compatibility\" to Always, and save again."
        ),
        _ => format!(
            "{editor} saved this file without a finished picture. Turn on its option to store a merged (maximum compatibility) picture in PSD files and save again."
        ),
    }
}

/// True when the "needs a merged picture" message offers "Convert with PhotoCraft's command-line tool" (13.2: only on
/// a PhotoCraft link).
pub fn offers_cli_convert(editor_key: &str) -> bool {
    editor_key == EDITOR_PHOTOCRAFT
}

/// The canvas is larger than a sticker may be (5.4, review A18): with **Fit it into the sticker** / **Ignore**.
pub fn canvas_too_large(sticker: &str, width: u32, height: u32, editor: &str) -> String {
    format!("{sticker} is now {width} x {height} in {editor}; a sticker can be at most 4096 px.")
}

/// Over the reader's limits (5.2): the "too large" message only.
pub fn too_large(file: &str, width: u32, height: u32) -> String {
    format!("{file} is too large for WTE ({width} x {height}; at most 8192 px a side).")
}

/// A damaged file (5.6 "Unreadable").
pub fn unreadable(file: &str, reason: &str) -> String {
    format!("{file} could not be read ({reason}).")
}

/// The conflict dialog (5.5): **Take <editor>'s version** / **Keep WTE's version** / Decide later.
pub fn conflict(sticker: &str, editor: &str) -> String {
    format!("Both WTE and {editor} changed {sticker}.")
}

/// The conflict after WTE rewrote a file the editor had open (5.5 "stale").
pub fn saved_over(sticker: &str, editor: &str) -> String {
    format!("{editor} saved over the changes WTE sent to {sticker}.")
}

/// The answer buttons of the conflict dialogs.
pub fn take_editors_version(editor: &str) -> String {
    format!("Take {editor}'s version")
}

/// "Keep WTE's version".
pub const KEEP_WTES_VERSION: &str = "Keep WTE's version";

/// A link write whose rename failed because another program holds the file (13.1, A8).
pub fn could_not_replace(file: &str, editor: &str) -> String {
    format!(
        "{file} could not be replaced because another program has it open. Close it in {editor} (or save it there), then choose Send to {editor} again."
    )
}

/// The save warning of 5.3 (review A26): the linked file changed and does not read completely.
pub fn being_written(sticker: &str) -> String {
    format!("{sticker}: the linked file is being written; the project keeps the earlier copy.")
}

/// The dialog after load (5.3): **Update** / **Ignore**.
pub fn changed_while_closed(sticker: &str) -> String {
    format!("Linked sticker {sticker} changed while the project was closed.")
}

/// "Edit in" on a never-saved project (5.1, 5.3): **Save...** / Cancel.
pub const SAVE_FIRST: &str = "Save the project first, so the linked file has a place next to it.";

/// The colour-profile note of 13.2 (once per link; the app keeps the "once").
pub fn profile_note(file: &str, description: &str) -> String {
    format!(
        "{file} uses the colour profile \"{description}\". WTE shows its colours as sRGB; they can look different. In Photoshop: Edit > Convert to Profile > sRGB."
    )
}

/// The log line when the ids of a file could not be trusted (13.2).
pub fn ids_not_kept(file: &str, rule: &str) -> String {
    format!("layer ids of {file} were not kept ({rule})")
}

/// Badge menu and button labels (13.1): "Send to <editor>", "Edit in <editor>".
pub fn send_to(editor: &str) -> String {
    format!("Send to {editor}")
}

/// "Edit in <editor>".
pub fn edit_in(editor: &str) -> String {
    format!("Edit in {editor}")
}

/// The close / New / Open prompt's extra line (5.3, review B8).
pub fn unsaved_links(names: &[&str], editor: &str, folder: &str) -> String {
    format!(
        "Stickers linked to {editor} since the last save: {}. If you do not save, WTE will not follow them any more; the PSD files stay in {folder}.",
        names.join(", ")
    )
}

/// The question when a link opened in one editor is opened in another (13.1): **Open in <new> anyway** / Cancel.
pub fn other_editor_open(old_editor: &str) -> String {
    format!("{old_editor} may still have this file open. Two programs saving the same file overwrite each other.")
}
