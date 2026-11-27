//! The `.wtep` project file (ARCHITECTURE.md 5.23, 7.1-7.3): shared types and re-exports. Saving, loading, version
//! migration and atomic file replacement live in the sub-modules.

use crate::document::Document;
use crate::error::LoadWarning;
use crate::image_io::PngLevel;

pub mod atomic;
pub mod load;
pub mod manifest;
pub mod migrate;
pub mod save;
pub mod sticker_entries;
#[cfg(test)]
mod sticker_tests;

pub use load::{extract_bundle, extract_sources, load, read_link_archive, read_manifest};
pub use manifest::*;
pub use sticker_entries::*;
pub use save::{required_reader, save};

/// Version of the manifest schema written by this build. 2 (round 4): text entries may carry `fx`; a file whose
/// texts use none of it is written with `min_reader_version` 1 (`save::required_reader`). 3 (round 6): a projected
/// placement may carry `mode` ("wrap") and the Wrap options `wrap_gaps` / `wrap_overlap`; a file that uses none of
/// them is written with the `min_reader_version` it had before. 4 (stickers, STICKER_PLAN 2.11): stamps may show a
/// sticker and carry layer styles, the manifest may list stickers and PhotoCraft links; a file that uses none of them
/// is written with the `min_reader_version` it had before.
pub const FORMAT_VERSION: u32 = 4;
/// File extension of a project file (without the dot).
pub const PROJECT_EXTENSION: &str = "wtep";
/// Content of the `mimetype` entry, the first entry of every project archive.
pub const PROJECT_MIMETYPE: &str = "application/x-wtep";

/// Parameters of `project::save`.
#[derive(Clone, Debug)]
pub struct SaveOptions {
    /// PNG compression of the layer images (the app uses `PngLevel::Fast`).
    pub png: PngLevel,
    /// Also store the full-size composite of every set (the app uses `true`).
    pub write_flattened: bool,
    /// Rotate an existing file to `<path>.bak` instead of replacing it (the app uses `false`).
    pub keep_backup: bool,
    /// Save time in seconds since the Unix epoch; the core never reads the clock.
    pub now_unix: u64,
    pub app_version: String,
}

/// What `project::save` wrote.
#[derive(Clone, Debug, Default)]
pub struct SaveStats {
    pub bytes_written: u64,
    pub entries: u32,
    /// The images of the written file that a later save may copy (see `PngReuse`).
    pub reuse: Option<PngReuse>,
    /// Things the user is told after a successful save (P5c: a linked file that was being written).
    pub warnings: Vec<SaveWarning>,
}

/// A note about a save that succeeded (STICKER_PLAN 5.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveWarning {
    /// The linked file of sticker `sticker` changed and does not read completely (its editor is writing it): the
    /// project keeps the earlier copy of it (none on a first save).
    LinkBeingWritten { sticker: crate::ids::StickerId, name: String },
    /// Stickers P12: the same for the linked file of paint layer `layer`.
    LayerLinkBeingWritten { layer: crate::ids::LayerId, name: String },
}

impl std::fmt::Display for SaveWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveWarning::LinkBeingWritten { name, .. } | SaveWarning::LayerLinkBeingWritten { name, .. } => {
                f.write_str(&crate::link::texts::being_written(name))
            }
        }
    }
}

/// The layer and composite images of an existing project file that `save` copies instead of encoding them again,
/// for every layer whose pixels did not change since (same id and `Layer::version`) and every set whose composite
/// inputs did not change (same `composite_key`). Recorded by `load` and by every successful `save` for the file it
/// read or wrote, kept by the document (`Document::set_png_reuse`, not persisted) and handed to the next save in its
/// snapshot. Used only while that file still has the recorded size and modification time. Format 4: the owned
/// sticker pictures and masks too, by picture version (`stickers`; STICKER_PLAN 2.11 "Save reuse").
#[derive(Clone, Debug, Default)]
pub struct PngReuse {
    pub path: std::path::PathBuf,
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
    /// Layer id -> (layer version, archive entry, rectangle `[x, y, w, h]`).
    pub layers: std::collections::HashMap<crate::ids::LayerId, (u64, String, [i32; 4])>,
    /// Set id -> (composite key, `flattened.png` entry).
    pub flattened: std::collections::HashMap<crate::ids::TextureSetId, (u64, String)>,
    /// (composite key of the first set, `thumbnail.png` entry).
    pub thumbnail: Option<(u64, String)>,
    /// Sticker picture or mask version (STICKER_PLAN 2.2) -> the `stickers/...` PNG entry that holds exactly those
    /// bytes. Versions are never reused within a document, so a matching version can never copy another picture.
    pub stickers: std::collections::HashMap<u64, String>,
}

impl PngReuse {
    /// An empty record for the file at `path` with its current size and modification time; `None` when the file
    /// cannot be inspected.
    pub(crate) fn for_file(path: &std::path::Path) -> Option<PngReuse> {
        let meta = std::fs::metadata(path).ok()?;
        Some(PngReuse { path: path.to_path_buf(), len: meta.len(), modified: meta.modified().ok(), ..PngReuse::default() })
    }

    /// True while the file still has the recorded size and modification time.
    pub(crate) fn file_unchanged(&self) -> bool {
        std::fs::metadata(&self.path).is_ok_and(|m| m.len() == self.len && m.modified().ok() == self.modified)
    }
}

/// Result of `project::load`.
pub struct LoadReport {
    pub document: Document,
    pub warnings: Vec<LoadWarning>,
    /// The file was written by a newer format version: plain Save must be refused so it is never overwritten.
    pub force_save_as: bool,
    /// P5c (STICKER_PLAN 5.3 "At load"): what was found on disk for every editor link. `load` only reports; it writes
    /// nothing. The app restores a missing `InFolder` file from its archive entry (`read_link_archive`) and asks
    /// about a changed one.
    pub links: Vec<LinkAtLoad>,
    /// Stickers P12: the same for every linked paint layer (STICKER_PLAN 5.9).
    pub layer_links: Vec<LayerLinkAtLoad>,
}

/// Stickers P12: one paint layer's editor link as `load` found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerLinkAtLoad {
    pub layer: crate::ids::LayerId,
    pub state: LinkFileState,
    /// Where the file is expected.
    pub path: Option<std::path::PathBuf>,
    /// The copy of the file inside the project (`links/l<id:05>.psd`), when the archive has it.
    pub archive: Option<String>,
}

/// One editor link as `load` found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkAtLoad {
    pub sticker: crate::ids::StickerId,
    pub state: LinkFileState,
    /// Where the file is expected (`None` only for an `InFolder` link of a project path without a folder).
    pub path: Option<std::path::PathBuf>,
    /// The copy of the file inside the project (`links/k<id:05>.psd`), when the archive has it.
    pub archive: Option<String>,
}

/// What `load` found at a link's path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinkFileState {
    /// The file has the content the link last synced (size and CRC-32; the time is not compared).
    Present,
    /// No file (an `InFolder` one can be restored from the archive; an `External` one is "link broken").
    Missing,
    /// The file changed while the project was closed ("Update" / "Ignore").
    Changed,
}

/// Counters of `project::save` (tests): sticker picture and mask PNGs encoded by saves started on this thread (one
/// add per encoded entry, counted on the calling thread before the parallel encode).
pub mod counters {
    use std::cell::Cell;

    thread_local! {
        static STICKER_ENCODES: Cell<u64> = const { Cell::new(0) };
    }

    pub(crate) fn sticker_encoded(n: u64) {
        STICKER_ENCODES.with(|c| c.set(c.get() + n));
    }

    /// Sticker PNGs encoded by saves on this thread so far (copied entries are not counted).
    pub fn sticker_encodes() -> u64 {
        STICKER_ENCODES.with(Cell::get)
    }
}

/// The key of everything the composite of a set depends on: its size and, per layer bottom to top, the id, pixel
/// version, visibility, opacity and blend mode. Equal keys mean an identical `flattened.png`.
pub(crate) fn composite_key(set: &crate::document::SetSnapshot) -> u64 {
    let mut h = crate::math::fnv1a64(0xcbf2_9ce4_8422_2325, &set.width.to_le_bytes());
    h = crate::math::fnv1a64(h, &set.height.to_le_bytes());
    for l in &set.layers {
        h = crate::math::fnv1a64(h, &l.id.0.to_le_bytes());
        h = crate::math::fnv1a64(h, &l.version().to_le_bytes());
        h = crate::math::fnv1a64(h, &[l.visible as u8, l.blend as u8]);
        h = crate::math::fnv1a64(h, &l.opacity.to_bits().to_le_bytes());
    }
    h
}
