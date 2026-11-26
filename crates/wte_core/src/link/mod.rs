//! The editor link of a sticker (stickers P5c; STICKER_PLAN 5.3-5.5, 13.1, 13.2; ARCHITECTURE 5.28 "link", 6.13).
//!
//! A linked sticker has a PSD file that both WTE and an outside editor (PhotoCraft, Photoshop, another program) use.
//! This module holds the link data ([`StickerLink`]), the round trip of 5.4 ([`map::export`] writes a sticker as PSD
//! layers, [`map::import`] takes a PSD back layer by layer or as one merged picture, with the ids rules of 13.2), the
//! states and decisions of 5.5 ([`link_state`], [`sync_action`], [`next_stale`]), the atomic file write with its
//! `.bak.psd` and the "could not be replaced" result ([`file::write_link_file`], 13.1 A8), the links folder and file
//! names, and every sentence the user sees about a link, each taking the editor's name ([`texts`]).
//!
//! Pure and headless: nothing here starts a program, watches a file or touches the clipboard. The core has no editor
//! enum (13.1): the link stores the editor's key (`"photocraft"`, `"photoshop"`, `"other"`) and the texts take the
//! name the app shows. The `Document` side (link map, `apply_link_update`, `prepare_link_file`, `link_state`) is
//! `doc_links.rs`; the project side (archive entries, the save rule, `LoadReport::links`) is `project/`.

pub mod file;
pub mod map;
pub mod texts;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use crate::ids::{StickerId, StickerLayerId};

pub use file::{LinkFileIo, LinkWriteError, RealFileIo, write_link_file};
pub use map::{ImportContext, ImportProblem, LayerOutcome, LinkExport, LinkImport, export, import, link_scale};

/// SMART (smart objects) makes a file come back as one merged picture for every link whose editor is not PhotoCraft
/// (STICKER_PLAN 13.2, decision D3: what the user was told). `false` would bring a smart object back as its own
/// picture layer (item 13 of section 11; only on the user's word).
pub const SMART_FORCES_OUTSIDE_PHOTOCRAFT: bool = true;

/// Editor keys (`StickerLink::editor`, prefs, manifest).
pub const EDITOR_PHOTOCRAFT: &str = "photocraft";
pub const EDITOR_PHOTOSHOP: &str = "photoshop";
pub const EDITOR_OTHER: &str = "other";

/// The undo label of an applied update (5.1).
pub const UPDATE_LABEL: &str = "Update Linked Sticker";

/// The sticker layer id a map entry carries for the placeholder layer of a sticker without layers (an empty PSD
/// layer "Layer 1": a PSD without layers would open in Photoshop as an opaque picture; check-P5b finding 1). Layer
/// ids start at 1, so 0 names no layer.
pub const PLACEHOLDER_LAYER: StickerLayerId = StickerLayerId(0);

/// True when SMART forces the merged path for a link of this editor (13.2).
pub fn smart_forces(editor_key: &str) -> bool {
    SMART_FORCES_OUTSIDE_PHOTOCRAFT && editor_key != EDITOR_PHOTOCRAFT
}

/// The name of a known editor key ("PhotoCraft", "Photoshop"); `None` for "other" (the app names it by its exe).
pub fn known_editor_name(key: &str) -> Option<&'static str> {
    match key {
        EDITOR_PHOTOCRAFT => Some("PhotoCraft"),
        EDITOR_PHOTOSHOP => Some("Photoshop"),
        _ => None,
    }
}

/// Where a linked file lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkFile {
    /// A file name in the project's links folder (`X_links\` beside `X.wtep`).
    InFolder(String),
    /// A file anywhere (5.8 "Keep linked to this file"); never copied or archived.
    External(PathBuf),
}

impl LinkFile {
    /// The file's path: an `InFolder` file needs the project's path (`None` for a never-saved project).
    pub fn path(&self, project: Option<&Path>) -> Option<PathBuf> {
        match self {
            LinkFile::InFolder(name) => project.map(|p| links_folder(p).join(name)),
            LinkFile::External(path) => Some(path.clone()),
        }
    }

    /// The file name the user knows ("Logo_k12.psd").
    pub fn display_name(&self) -> String {
        match self {
            LinkFile::InFolder(name) => name.clone(),
            LinkFile::External(path) => path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
        }
    }
}

/// How the file came back the last time.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum LinkMode {
    /// Layer by layer (5.4 "Layers mode").
    #[default]
    Layers,
    /// As one picture (something forced it, the file was flat, or it was fitted into the sticker).
    Merged,
}

/// Size, modification time and CRC-32 of a linked file's content.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct FileStamp {
    pub size: u64,
    /// Nanoseconds since the Unix epoch, when known.
    pub modified_ns: Option<u64>,
    pub crc32: u32,
}

impl FileStamp {
    /// The stamp of `bytes` with the given modification time.
    pub fn of_bytes(bytes: &[u8], modified: Option<std::time::SystemTime>) -> FileStamp {
        FileStamp { size: bytes.len() as u64, modified_ns: modified.and_then(unix_ns), crc32: crc32fast::hash(bytes) }
    }

    /// Reads the file at `path`: its stamp and its bytes.
    pub fn read(path: &Path) -> std::io::Result<(FileStamp, Vec<u8>)> {
        let bytes = std::fs::read(path)?;
        let modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        Ok((FileStamp::of_bytes(&bytes, modified), bytes))
    }

    /// Equal content (size and CRC-32); the modification time is not compared, so a copy of the same bytes is the
    /// same file (WTE's own writes and restored copies are recognised by content, 5.6).
    pub fn same_content(&self, other: &FileStamp) -> bool {
        self.size == other.size && self.crc32 == other.crc32
    }
}

/// Nanoseconds since the Unix epoch.
pub fn unix_ns(t: std::time::SystemTime) -> Option<u64> {
    t.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| u64::try_from(d.as_nanos()).ok())
}

/// One PSD layer id <-> sticker layer pair, with what WTE wrote or read for it (normalised as in 5.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkLayer {
    pub lyid: u32,
    /// The sticker layer; [`PLACEHOLDER_LAYER`] for the empty layer written for a sticker without layers.
    pub layer: StickerLayerId,
    /// The layer name written or read (the "`lyid` reuse" rule and the ids rules compare names).
    pub name: String,
    /// Top-left of the normalised layer in canvas pixels (0, 0 for an empty layer).
    pub origin: [i32; 2],
    /// Size of the normalised layer (0 x 0 for an empty layer).
    pub size: [u32; 2],
    /// CRC-32 of the normalised RGBA bytes.
    pub pixels_crc: u32,
    /// CRC-32 of the user mask over the layer's rectangle (and its disabled flag); `None` without a mask.
    pub mask_crc: Option<u32>,
}

/// Why the last file could not be applied; shown on the badge while the file still has this content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkProblem {
    /// A colour mode or depth WTE does not read (13.2): the sentence says what to click.
    Refused { crc32: u32, sentence: String },
    /// Something forces the merged path but the file has no real merged picture (MaxCompat off).
    NeedsMerged { crc32: u32 },
    /// The canvas divided by the link scale is larger than a sticker may be (5.4 "Fit it into the sticker").
    CanvasTooLarge { crc32: u32, width: u32, height: u32 },
    /// The canvas is over the reader's pixel limits (8192 a side): "too large", nothing else.
    TooLarge { crc32: u32, width: u32, height: u32 },
    /// The file is damaged (it read completely but is not a well-formed PSD).
    Unreadable { crc32: u32, reason: String },
}

impl LinkProblem {
    /// The content the problem is about.
    pub fn crc32(&self) -> u32 {
        match self {
            LinkProblem::Refused { crc32, .. }
            | LinkProblem::NeedsMerged { crc32 }
            | LinkProblem::CanvasTooLarge { crc32, .. }
            | LinkProblem::TooLarge { crc32, .. }
            | LinkProblem::Unreadable { crc32, .. } => *crc32,
        }
    }
}

/// The link of one sticker (5.3). Kept in `Document` outside the history (R6): undoing an update does not make the
/// watcher apply the file again; changes set the document's modified flag.
#[derive(Clone, Debug, PartialEq)]
pub struct StickerLink {
    pub file: LinkFile,
    /// The editor's key (`"photocraft"`, `"photoshop"`, `"other"`), set at every "Edit in" (13.1).
    pub editor: String,
    pub mode: LinkMode,
    /// PSD pixels per layer-space pixel: 1, 2 or 4 when WTE wrote the file; any value >= 1 for a canvas fitted into
    /// the artboard limit. PSD canvas = artboard size * scale.
    pub scale: f32,
    /// The file content WTE last wrote or applied.
    pub synced: FileStamp,
    /// `Document::sticker_link_hash` of the sticker that corresponds to `synced`.
    pub synced_sticker: u64,
    pub layer_map: Vec<LinkLayer>,
    /// Above every `lyid` ever written or read for this link.
    pub next_lyid: u32,
    /// The file holds the editor's own text / smart / fill layers: a rewrite from WTE turns them into pixels (5.5).
    pub editor_native: bool,
    /// Why the file with this content could not be applied (not saved; the next read decides again).
    pub problem: Option<LinkProblem>,
}

impl StickerLink {
    /// The map entry of `lyid`.
    pub fn entry(&self, lyid: u32) -> Option<&LinkLayer> {
        self.layer_map.iter().find(|e| e.lyid == lyid)
    }

    /// The scale as a usable number (>= 1, finite).
    pub fn scale_or_one(&self) -> f32 {
        if self.scale.is_finite() && self.scale >= 1.0 { self.scale } else { 1.0 }
    }
}

/// What the badge shows (5.5; the texts are [`LinkState::badge`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// "linked": file and sticker are in sync.
    Linked,
    /// "changed in WTE": the sticker changed since the last sync; nothing is written by itself.
    ChangedInWte,
    /// "changed in <editor>": the file changed and waits to be applied.
    ChangedInEditor,
    /// "conflict": both changed.
    Conflict,
    /// "file missing".
    Missing,
    /// "needs a merged picture".
    NeedsMerged,
    /// A refused file: the sentence of 13.2 ("<file> is in CMYK Color mode. ...").
    Refused(String),
    /// The editor's canvas is larger than a sticker may be (offer "Fit it into the sticker" / "Ignore").
    CanvasTooLarge { width: u32, height: u32 },
    /// Over the reader's limits.
    TooLarge { width: u32, height: u32 },
    /// A damaged file.
    Unreadable(String),
}

impl LinkState {
    /// The badge text. `editor` is the link editor's name, `sticker` the sticker's name, `file` the file's name.
    pub fn badge(&self, editor: &str, sticker: &str, file: &str) -> String {
        match self {
            LinkState::Linked => "linked".to_owned(),
            LinkState::ChangedInWte => "changed in WTE".to_owned(),
            LinkState::ChangedInEditor => format!("changed in {editor}"),
            LinkState::Conflict => "conflict".to_owned(),
            LinkState::Missing => "file missing".to_owned(),
            LinkState::NeedsMerged => "needs a merged picture".to_owned(),
            LinkState::Refused(sentence) => sentence.clone(),
            LinkState::CanvasTooLarge { width, height } => texts::canvas_too_large(sticker, *width, *height, editor),
            LinkState::TooLarge { width, height } => texts::too_large(file, *width, *height),
            LinkState::Unreadable(reason) => texts::unreadable(file, reason),
        }
    }
}

/// The state of a link (5.5) from what the app observed: `observed` = the file's current stamp (`None` = the file is
/// missing), `sticker_hash` = the sticker's current `Document::sticker_link_hash`. A problem recorded for the file's
/// current content wins over "changed in <editor>".
pub fn link_state(link: &StickerLink, observed: Option<&FileStamp>, sticker_hash: u64) -> LinkState {
    let Some(observed) = observed else { return LinkState::Missing };
    let file_changed = !observed.same_content(&link.synced);
    let wte_changed = sticker_hash != link.synced_sticker;
    if let Some(problem) = link.problem.as_ref().filter(|p| p.crc32() == observed.crc32) {
        return match problem {
            LinkProblem::Refused { sentence, .. } => LinkState::Refused(sentence.clone()),
            LinkProblem::NeedsMerged { .. } => LinkState::NeedsMerged,
            LinkProblem::CanvasTooLarge { width, height, .. } => LinkState::CanvasTooLarge { width: *width, height: *height },
            LinkProblem::TooLarge { width, height, .. } => LinkState::TooLarge { width: *width, height: *height },
            LinkProblem::Unreadable { reason, .. } => LinkState::Unreadable(reason.clone()),
        };
    }
    match (file_changed, wte_changed) {
        (false, false) => LinkState::Linked,
        (true, false) => LinkState::ChangedInEditor,
        (false, true) => LinkState::ChangedInWte,
        (true, true) => LinkState::Conflict,
    }
}

/// What to do about a settled change of the file (the table of 5.5 plus the stale rule).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SyncAction {
    /// Nothing: the file did not change (an undo of an update leaves it so: R6).
    Nothing,
    /// Apply the file (row "yes / no").
    Apply,
    /// Ask (never automatic). `saved_over`: the editor saved over what WTE sent ("<editor> saved over the changes
    /// WTE sent to <name>."), else "Both WTE and <editor> changed <name>."
    Conflict { saved_over: bool },
}

/// The decision of 5.5 for a file change: `file_changed` / `wte_changed` as in [`link_state`], `stale_in_editor`
/// the session flag of the link ([`next_stale`]).
pub fn sync_action(file_changed: bool, wte_changed: bool, stale_in_editor: bool) -> SyncAction {
    match (file_changed, wte_changed, stale_in_editor) {
        (false, _, _) => SyncAction::Nothing,
        (true, _, true) => SyncAction::Conflict { saved_over: true },
        (true, true, false) => SyncAction::Conflict { saved_over: false },
        (true, false, false) => SyncAction::Apply,
    }
}

/// What happened to a link in this session, for the `stale_in_editor` flag (5.5, 13.1).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StaleEvent {
    /// WTE rewrote the file; `opened` = the file was opened in the editor in this session (`opened_at` set).
    Rewrote { opened: bool },
    /// The user answered the "saved over" (or any conflict) dialog.
    Answered,
    /// "Edit in" was started again; `child_exited` = the previous launched child is known to have exited (a fresh
    /// editor window reads the new file).
    EditAgain { child_exited: bool },
}

/// The `stale_in_editor` flag after `event` (a pure function of the inputs the app passes in). While it is set, the
/// next change of the file is never applied by itself ([`sync_action`]).
pub fn next_stale(stale: bool, event: StaleEvent) -> bool {
    match event {
        StaleEvent::Rewrote { opened } => stale || opened,
        StaleEvent::Answered => false,
        StaleEvent::EditAgain { child_exited } => stale && !child_exited,
    }
}

/// The question "Edit in <editor>" asks when WTE holds newer content (5.5 row "no / yes"): `None` when nothing needs
/// sending. The answers are **Send and open** / **Open the file as it is** / Cancel.
pub fn send_first_question(link: &StickerLink, wte_changed: bool, editor: &str) -> Option<String> {
    if !wte_changed {
        return None;
    }
    let mut q = "Send WTE's changes to the file first?".to_owned();
    if link.mode == LinkMode::Merged || link.editor_native {
        q.push_str(&format!(" This replaces {editor}'s own layers (text, effects) in the file with plain picture layers."));
    }
    Some(q)
}

/// What is known about the program a link file was opened in, for the warning before a rewrite (5.5, 13.1).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OpenedState {
    /// Not opened in this session: no warning.
    NotOpened,
    /// The launched child is still running (PhotoCraft v0.2.0 and portable copies, Other).
    ChildRunning,
    /// Opened, but whether the file is still open is unknown (the child exited or handed the file on: the installed
    /// PhotoCraft, Photoshop).
    Unknown,
}

/// The warning before WTE rewrites a link file (**Send anyway** / Cancel); `None` when the file was not opened.
pub fn rewrite_warning(opened: OpenedState, editor: &str) -> Option<String> {
    let tail = format!(
        " Its next save will replace what you send now. Close the file in {editor} first, or reopen it there after sending."
    );
    match opened {
        OpenedState::NotOpened => None,
        OpenedState::ChildRunning => Some(format!("{editor} has this file open.{tail}")),
        OpenedState::Unknown => Some(format!("{editor} may still have this file open.{tail}")),
    }
}

/// The project's links folder: `X_links` beside `X.wtep` (5.3).
pub fn links_folder(project: &Path) -> PathBuf {
    let stem = project.file_stem().map_or_else(|| "project".into(), |s| s.to_string_lossy().into_owned());
    project.with_file_name(format!("{stem}_links"))
}

/// The file name of a sticker's link file: `<sanitised sticker name>_k<id>.psd` (5.3). Letters, digits, spaces,
/// `-` and `_` are kept (any script), everything else becomes `_`; at most 40 characters of the name; an empty name
/// is "sticker".
pub fn link_file_name(sticker_name: &str, id: StickerId) -> String {
    let mut name: String = sticker_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    name = name.trim().trim_end_matches('.').to_owned();
    if name.is_empty() {
        name = "sticker".to_owned();
    }
    format!("{name}_k{}.psd", id.0)
}

/// The archive entry of a link file inside the project: `links/k<id:05>.psd`.
pub fn archive_entry(id: StickerId) -> String {
    format!("links/k{:05}.psd", id.0)
}
