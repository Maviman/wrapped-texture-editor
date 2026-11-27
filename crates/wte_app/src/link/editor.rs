//! The outside editors a link can be opened in (STICKER_PLAN 13.1; stickers P9a builds the type with PhotoCraft as
//! the only discoverable editor, P9b adds Photoshop's discovery, the editor list and the default editor).
//!
//! The core knows only the editor's key (`StickerLink::editor`); every text the user sees takes the editor's name
//! (`name()`), never a literal.
//!
//! Stickers P9b: the programs of the three editors live in the prefs (`photoshop_exe`, `photocraft_exe`,
//! `other_editor_exe`), the default editor's key in `Prefs::default_editor` (decision D1: none until the user picks one
//! in the chooser, or the only known editor). Discovery reads the registry once per session, when an "Edit in" control
//! is first shown or used and whenever Help > Editor locations... opens ([`ensure_discovered`]); the editors whose
//! program exists are cached in `LinkUi::known` for the labels drawn every frame.

use std::path::{Path, PathBuf};

use wte_core::link::{EDITOR_OTHER, EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP};

use crate::state::{AppState, Prefs};

/// An outside editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalEditor {
    PhotoCraft,
    Photoshop,
    /// Any program the user picked ("Other program..."), named by its exe's file stem.
    Other(PathBuf),
}

impl ExternalEditor {
    /// The name every text shows: "PhotoCraft", "Photoshop", or the exe's file stem ("gimp-2.10").
    pub fn name(&self) -> String {
        match self {
            ExternalEditor::PhotoCraft => "PhotoCraft".to_owned(),
            ExternalEditor::Photoshop => "Photoshop".to_owned(),
            ExternalEditor::Other(exe) => exe_stem(exe),
        }
    }

    /// The key saved in prefs and in the project (`"photocraft"`, `"photoshop"`, `"other"`).
    pub fn key(&self) -> &'static str {
        match self {
            ExternalEditor::PhotoCraft => EDITOR_PHOTOCRAFT,
            ExternalEditor::Photoshop => EDITOR_PHOTOSHOP,
            ExternalEditor::Other(_) => EDITOR_OTHER,
        }
    }

    /// The editor of a key; `other` needs the program it names (`None` without one). An unknown key is PhotoCraft
    /// (the manifest's default).
    pub fn from_key(key: &str, other_exe: Option<&Path>) -> Option<ExternalEditor> {
        match key {
            EDITOR_PHOTOSHOP => Some(ExternalEditor::Photoshop),
            EDITOR_OTHER => other_exe.map(|p| ExternalEditor::Other(p.to_path_buf())),
            _ => Some(ExternalEditor::PhotoCraft),
        }
    }

    /// The file name its exe usually has ("photocraft.exe"); `None` for another program.
    pub fn exe_name(&self) -> Option<&'static str> {
        match self {
            ExternalEditor::PhotoCraft => Some("photocraft.exe"),
            ExternalEditor::Photoshop => Some("Photoshop.exe"),
            ExternalEditor::Other(_) => None,
        }
    }
}

/// The editors in the order of every list (13.1): Photoshop, PhotoCraft, then another program.
pub const KEYS: [&str; 3] = [EDITOR_PHOTOSHOP, EDITOR_PHOTOCRAFT, EDITOR_OTHER];

/// The remembered program of the editor with `key` (`None` when not set).
pub fn exe_pref<'a>(prefs: &'a Prefs, key: &str) -> Option<&'a PathBuf> {
    match key {
        EDITOR_PHOTOSHOP => prefs.photoshop_exe.as_ref(),
        EDITOR_PHOTOCRAFT => prefs.photocraft_exe.as_ref(),
        EDITOR_OTHER => prefs.other_editor_exe.as_ref(),
        _ => None,
    }
}

/// Sets (or clears) the remembered program of the editor with `key`.
pub fn set_exe_pref(prefs: &mut Prefs, key: &str, path: Option<PathBuf>) {
    match key {
        EDITOR_PHOTOSHOP => prefs.photoshop_exe = path,
        EDITOR_PHOTOCRAFT => prefs.photocraft_exe = path,
        EDITOR_OTHER => prefs.other_editor_exe = path,
        _ => {}
    }
}

/// The editor of `key` as this machine knows it (`other` needs its remembered program).
pub fn editor_of_key(prefs: &Prefs, key: &str) -> Option<ExternalEditor> {
    match key {
        EDITOR_PHOTOSHOP => Some(ExternalEditor::Photoshop),
        EDITOR_PHOTOCRAFT => Some(ExternalEditor::PhotoCraft),
        EDITOR_OTHER => prefs.other_editor_exe.clone().map(ExternalEditor::Other),
        _ => None,
    }
}

/// The editors whose program is remembered and exists, in list order (looks at the disk; not for every frame).
pub fn known_editors(prefs: &Prefs, exists: &dyn Fn(&Path) -> bool) -> Vec<ExternalEditor> {
    KEYS.iter()
        .filter(|k| exe_pref(prefs, k).is_some_and(|p| exists(p)))
        .filter_map(|k| editor_of_key(prefs, k))
        .collect()
}

/// Discovery for every editor whose program is not remembered or no longer exists (13.1): once per session, or again
/// with `force` (Help > Editor locations...). A found program is remembered in the prefs (written with them at exit);
/// `LinkUi::known` is refreshed.
pub fn ensure_discovered(state: &mut AppState, force: bool) {
    if state.link_ui.discovered && !force {
        return;
    }
    state.link_ui.discovered = true;
    let exists = |p: &Path| p.is_file();
    for editor in [ExternalEditor::Photoshop, ExternalEditor::PhotoCraft] {
        if exe_pref(&state.prefs, editor.key()).is_some_and(|p| exists(p)) {
            continue;
        }
        if let Some(found) = super::launch::discover(&editor, &*state.link_ui.registry, &exists) {
            log::info!("{} found at {}", editor.name(), found.display());
            set_exe_pref(&mut state.prefs, editor.key(), Some(found));
        }
    }
    refresh_known(state);
}

/// Recomputes `LinkUi::known` (after discovery and after a program was chosen or cleared).
pub fn refresh_known(state: &mut AppState) {
    state.link_ui.known = known_editors(&state.prefs, &|p: &Path| p.is_file());
}

/// The default editor the prefs name, if any (an `other` default without a remembered program is none).
pub fn default_of(prefs: &Prefs) -> Option<ExternalEditor> {
    prefs.default_editor.as_deref().and_then(|k| editor_of_key(prefs, k))
}

/// The editor a new link opens in without asking (the labels, an import that keeps the file linked): the default
/// editor, else the only known editor, else PhotoCraft (it can be located). `None` when two or more editors are known
/// and none is the default: the main button reads "Edit in..." and its click opens the chooser.
pub fn quiet_editor(state: &AppState) -> Option<ExternalEditor> {
    if let Some(e) = default_of(&state.prefs) {
        return Some(e);
    }
    match state.link_ui.known.as_slice() {
        [] => Some(ExternalEditor::PhotoCraft),
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// The file stem of an exe path ("program" when it has none).
fn exe_stem(exe: &Path) -> String {
    exe.file_stem().map_or_else(|| "program".to_owned(), |s| s.to_string_lossy().into_owned())
}

/// The name shown for a link's editor key (`StickerLink::editor`): "PhotoCraft", "Photoshop", or the stem of the
/// remembered other program ("program" when none is remembered).
pub fn name_of_key(key: &str, other_exe: Option<&Path>) -> String {
    match ExternalEditor::from_key(key, other_exe) {
        Some(e) => e.name(),
        None => other_exe.map_or_else(|| "program".to_owned(), exe_stem),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_keys() {
        assert_eq!(ExternalEditor::PhotoCraft.name(), "PhotoCraft");
        assert_eq!(ExternalEditor::Photoshop.name(), "Photoshop");
        let gimp = ExternalEditor::Other(PathBuf::from(r"C:\Program Files\GIMP 2\bin\gimp-2.10.exe"));
        assert_eq!(gimp.name(), "gimp-2.10");
        assert_eq!(
            [ExternalEditor::PhotoCraft.key(), ExternalEditor::Photoshop.key(), gimp.key()],
            ["photocraft", "photoshop", "other"]
        );
        for e in [ExternalEditor::PhotoCraft, ExternalEditor::Photoshop] {
            assert_eq!(ExternalEditor::from_key(e.key(), None), Some(e));
        }
        assert_eq!(ExternalEditor::from_key("other", None), None);
        assert_eq!(ExternalEditor::from_key("other", Some(Path::new(r"C:\x\paint.exe"))), Some(ExternalEditor::Other("C:\\x\\paint.exe".into())));
        assert_eq!(ExternalEditor::from_key("something new", None), Some(ExternalEditor::PhotoCraft));
        assert_eq!(name_of_key("photoshop", None), "Photoshop");
        assert_eq!(name_of_key("other", Some(Path::new(r"C:\x\paint.exe"))), "paint");
        assert_eq!(name_of_key("other", None), "program");
    }
}
