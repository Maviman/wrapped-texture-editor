//! The outside editors a link can be opened in (STICKER_PLAN 13.1; stickers P9a builds the type with PhotoCraft as
//! the only discoverable editor, P9b adds Photoshop's discovery, the editor list and the default editor).
//!
//! The core knows only the editor's key (`StickerLink::editor`); every text the user sees takes the editor's name
//! (`name()`), never a literal.

use std::path::{Path, PathBuf};

use wte_core::link::{EDITOR_OTHER, EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP};

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
