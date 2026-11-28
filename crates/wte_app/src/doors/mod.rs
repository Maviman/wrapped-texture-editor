// Owner: stickers S2 (STICKER_PLAN 7 "S2"; ARCHITECTURE 8.18). Two more doors between WTE and PhotoCraft (or any
// program), both plain files, no FFI, nothing started:
//
// (a) the live PNG copy (`live.rs`): "Keep a PNG copy up to date" of a sticker (Sticker tab: the selected stamp's
//     sticker, the selected tray sticker) or of a raster paint layer (Layer menu, the row menu) keeps
//     `<project>_links\<name>_k<id>.png` / `<name>_l<id>.png` written 500 ms after the last change, for PhotoCraft's
//     File > Place Linked (v0.2.0: Layer > Smart Objects > Update Modified Content; later versions refresh by
//     themselves). It needs a saved project (the links folder is beside it), as a link does (5.3).
// (b) the watched folder (`watch.rs` = its worker): File > Watch Folder for Stickers... (or the Sticker tab's "Watch a
//     folder...") - every PNG that appears or changes there becomes, or updates, a tray sticker named after the file
//     (PhotoCraft's File > Generate > Image Assets writes `<document>-assets\<layer name>.png` at every save, critic
//     D1). A file's sticker is remembered (file name -> sticker and layer, the content last applied), so a change
//     updates the same sticker in place (its placements on the model are kept; a new size refits them as a crop
//     does, 2.9) and reopening the project reads nothing again. One undo step per batch; undoing it does not make the
//     folder apply the file again (as R6 for links); "Read again" does.
//
// Both settings are saved with the project in `Document::ui_state` (`io_flows::UiState`, keys `live_copies` and
// `watch_folder`, left out while unused: an old project's bytes do not change) and set the modified flag
// (`Document::mark_kept_settings_changed`), never a history entry. Updates wait while a stroke, a drag, a pending
// Preview edit or a focused text field is open, as the link's updates do (5.6).

pub mod live;
pub mod watch;

#[cfg(test)]
mod tests;

/// Test helpers of other modules' tests.
#[cfg(test)]
pub(crate) mod tests_support {
    /// The watched folder of `saved` (without a session).
    pub(crate) fn folder(saved: &super::SavedWatch) -> super::WatchedFolder {
        let mut w = super::WatchedFolder::new(saved.folder.clone());
        for f in &saved.files {
            let file = super::WatchedFile {
                sticker: wte_core::StickerId(f.sticker),
                layer: wte_core::StickerLayerId(f.layer),
                size: f.size,
                crc: f.crc,
                pixels: [f.width, f.height],
            };
            w.files.insert(f.name.clone(), file);
        }
        w
    }
}

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use eframe::egui;
use serde::{Deserialize, Serialize};
use wte_core::{Commit, IRect, PicturePixels, StickerContent, StickerDoc, StickerId, StickerLayerId, StickerRef};

use crate::action::AppAction;
use crate::state::{AppState, PanelHit};
use live::{LiveCopies, LiveEntry, LiveTarget};
use watch::{FolderEvent, FolderTimes, FolderWatcher, Known};

/// The undo label of a sticker updated from the watched folder.
pub const FOLDER_UPDATE_LABEL: &str = "Update Sticker from Folder";
/// The label of a batch of several new or updated stickers.
pub const FOLDER_BATCH_LABEL: &str = "Stickers from Folder";

/// A command of S2 (`AppAction::Doors`, run by [`run`]).
#[derive(Clone, Debug, PartialEq)]
pub enum DoorAction {
    /// "Keep a PNG copy up to date" on or off.
    SetLiveCopy(LiveTarget, bool),
    /// "Show file in Explorer" of a live copy.
    ShowLiveCopy(LiveTarget),
    /// File > Watch Folder for Stickers... (`None`: the folder dialog; the app resolves it).
    WatchFolder(Option<PathBuf>),
    StopWatching,
    /// "Read again": every picture of the folder is read and applied again.
    RescanFolder,
    /// "Show folder".
    ShowFolder,
}

/// One file of the watched folder that became a tray sticker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchedFile {
    pub sticker: StickerId,
    /// The picture layer the file fills.
    pub layer: StickerLayerId,
    /// The content last applied (size and CRC-32 of the file).
    pub size: u64,
    pub crc: u32,
    /// The picture's size in pixels when it was applied.
    pub pixels: [u32; 2],
}

/// The watched folder of the open document.
pub struct WatchedFolder {
    pub folder: PathBuf,
    /// File name -> its sticker.
    pub files: BTreeMap<String, WatchedFile>,
    watcher: Option<FolderWatcher>,
    /// Events held while an interaction is open (the latest per file).
    held: Vec<FolderEvent>,
    /// The folder is gone (the worker said so).
    pub missing: bool,
    /// The folder holds more pictures than are followed.
    pub too_many: Option<usize>,
}

impl WatchedFolder {
    fn new(folder: PathBuf) -> WatchedFolder {
        WatchedFolder { folder, files: BTreeMap::new(), watcher: None, held: Vec::new(), missing: false, too_many: None }
    }
}

/// Counts for the tests and scenarios.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderCounters {
    pub added: u64,
    pub updated: u64,
    /// Pictures that were what their sticker shows already ("Read again" of an unchanged folder).
    pub same: u64,
    pub refused: u64,
}

/// S2's state (`AppState::doors`).
pub struct DoorsUi {
    pub live: LiveCopies,
    pub watch: Option<WatchedFolder>,
    pub folder_times: FolderTimes,
    pub folder_counters: FolderCounters,
    /// Repaint requests of the folder workers (the idle check of 8.2: nothing moves while nothing changes).
    pub folder_repaints: Arc<AtomicU64>,
    focused: bool,
}

impl Default for DoorsUi {
    fn default() -> DoorsUi {
        DoorsUi {
            live: LiveCopies::default(),
            watch: None,
            folder_times: FolderTimes::default(),
            folder_counters: FolderCounters::default(),
            folder_repaints: Arc::new(AtomicU64::new(0)),
            focused: false,
        }
    }
}

impl DoorsUi {
    /// The document changes: its copies and its watched folder go with it (`io_flows::install_document`).
    pub fn reset(&mut self) {
        self.live.reset();
        self.watch = None;
    }

    /// True while a copy is being written (the self-test's idle check).
    pub fn busy(&self) -> bool {
        self.live.busy()
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Saved with the project
// ---------------------------------------------------------------------------------------------------------------------

/// How the watched folder is saved with the project (`Document::ui_state`, key `watch_folder`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedWatch {
    pub folder: PathBuf,
    pub files: Vec<SavedWatchFile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedWatchFile {
    pub name: String,
    pub sticker: u64,
    pub layer: u32,
    pub size: u64,
    pub crc: u32,
    pub width: u32,
    pub height: u32,
}

/// What `UiState::capture` writes: the copies (of stickers and layers in `doc`) and the watched folder.
pub fn saved(doors: &DoorsUi, doc: &wte_core::Document) -> (Vec<live::SavedLiveCopy>, Option<SavedWatch>) {
    let copies = live::saved(&doors.live, doc);
    let watch = doors.watch.as_ref().map(|w| SavedWatch {
        folder: w.folder.clone(),
        files: w
            .files
            .iter()
            .map(|(name, f)| SavedWatchFile {
                name: name.clone(),
                sticker: f.sticker.0,
                layer: f.layer.0,
                size: f.size,
                crc: f.crc,
                width: f.pixels[0],
                height: f.pixels[1],
            })
            .collect(),
    });
    (copies, watch)
}

/// Takes the copies and the folder of a loaded project (after `reset`).
pub fn restore(state: &mut AppState, copies: &[live::SavedLiveCopy], watch: Option<&SavedWatch>) {
    let Some(doc) = state.session.as_ref().map(|s| &s.doc) else { return };
    live::restore(&mut state.doors.live, copies, doc);
    if let Some(w) = watch.filter(|w| !w.folder.as_os_str().is_empty()) {
        let mut folder = WatchedFolder::new(w.folder.clone());
        for f in &w.files {
            if f.name.is_empty() || f.name.contains(['/', '\\']) {
                continue;
            }
            folder.files.insert(
                f.name.clone(),
                WatchedFile { sticker: StickerId(f.sticker), layer: StickerLayerId(f.layer), size: f.size, crc: f.crc, pixels: [f.width, f.height] },
            );
        }
        state.doors.watch = Some(folder);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------------------------------

/// Text when a live copy is turned on without a project file.
pub const NEEDS_SAVE: &str = "Save the project first: the PNG copy is kept in the project's links folder, beside the project file";

/// Runs a command (`dispatch`; the folder dialog of `WatchFolder(None)` is resolved there first).
pub fn run(state: &mut AppState, action: DoorAction) {
    match action {
        DoorAction::SetLiveCopy(target, on) => set_live_copy(state, target, on),
        DoorAction::ShowLiveCopy(target) => {
            if let Some(path) = live::path_of(state, target)
                && let Err(e) = state.link_ui.launcher.show_file(&path)
            {
                state.toasts.warn(e);
            }
        }
        DoorAction::WatchFolder(Some(folder)) => watch_folder(state, folder),
        DoorAction::WatchFolder(None) => log::debug!("a folder dialog request needs the app; dropped"),
        DoorAction::StopWatching => {
            if let Some(w) = state.doors.watch.take() {
                if let Some(doc) = state.doc_mut() {
                    doc.mark_kept_settings_changed();
                }
                state.status_hint = format!("No longer watching {}: its stickers stay in the tray", w.folder.display());
            }
        }
        DoorAction::RescanFolder => {
            if let Some(w) = state.doors.watch.as_mut() {
                match &w.watcher {
                    Some(watcher) => watcher.rescan(),
                    // Not started yet: the next `pump` starts it with nothing known.
                    None => w.files.values_mut().for_each(|f| f.crc = 0),
                }
            }
        }
        DoorAction::ShowFolder => {
            if let Some(folder) = state.doors.watch.as_ref().map(|w| w.folder.clone())
                && let Err(e) = state.link_ui.launcher.show_file(&folder)
            {
                state.toasts.warn(e);
            }
        }
    }
}

fn set_live_copy(state: &mut AppState, target: LiveTarget, on: bool) {
    let Some(session) = state.session.as_ref() else { return };
    let doc = &session.doc;
    if !on {
        if let Some(e) = state.doors.live.entries.remove(&target) {
            if let Some(doc) = state.doc_mut() {
                doc.mark_kept_settings_changed();
            }
            state.status_hint = format!("{} stays in the links folder; it is no longer updated", e.file);
        }
        return;
    }
    if state.doors.live.entries.contains_key(&target) || !live::exists(doc, target) {
        return;
    }
    let Some(project) = session.path.clone() else {
        state.toasts.info(NEEDS_SAVE);
        state.status_hint = NEEDS_SAVE.to_owned();
        return;
    };
    let Some(file) = live::file_name_for(doc, target) else { return };
    let folder = wte_core::link::links_folder(&project);
    state.doors.live.entries.insert(target, LiveEntry::new(file.clone()));
    if let Some(doc) = state.doc_mut() {
        doc.mark_kept_settings_changed();
    }
    state.status_hint = format!(
        "{file} in {} is kept up to date: place it in PhotoCraft with File > Place Linked... (PhotoCraft 0.2: Update Modified Content refreshes it)",
        folder.display()
    );
}

fn watch_folder(state: &mut AppState, folder: PathBuf) {
    let Some(session) = state.session.as_ref() else { return };
    if !folder.is_dir() {
        state.toasts.warn(format!("{} is not a folder", folder.display()));
        return;
    }
    let links = session.path.as_deref().map(wte_core::link::links_folder);
    if links.as_deref().is_some_and(|l| same_folder(l, &folder)) {
        state.toasts.warn("This is the project's links folder (WTE's own files are there): choose another folder");
        return;
    }
    let again = state.doors.watch.as_ref().is_some_and(|w| same_folder(&w.folder, &folder));
    if again {
        if let Some(w) = &state.doors.watch.as_ref().and_then(|w| w.watcher.as_ref()) {
            w.poke();
        }
    } else {
        state.doors.watch = Some(WatchedFolder::new(folder.clone()));
        if let Some(doc) = state.doc_mut() {
            doc.mark_kept_settings_changed();
        }
    }
    state.status_hint = format!(
        "Watching {}: every PNG picture there becomes a tray sticker, and a changed one updates its sticker",
        folder.display()
    );
}

/// The same folder (compared as written, then by their canonical paths).
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

// ---------------------------------------------------------------------------------------------------------------------
// The frame hook
// ---------------------------------------------------------------------------------------------------------------------

/// True while an update or a copy must wait: a 3D tool interaction, a pending Preview edit, an open texture or sticker
/// stroke, a drag on the sticker canvas or a focused text field.
pub fn interacting(state: &AppState, tools_busy: bool) -> bool {
    let Some(session) = state.session.as_ref() else { return false };
    tools_busy
        || session.doc.has_pending_edit()
        || session.doc.sticker_stroke_active()
        || session.doc.stroke_active()
        || state.sticker_ui.drag.is_some()
        || state.typing_prev
}

/// Every frame (`App::logic`): the live copies, the folder worker (started, stopped, its events applied or held).
pub fn pump(state: &mut AppState, ctx: &egui::Context, tools_busy: bool) {
    let busy = interacting(state, tools_busy);
    if let Some(wake) = live::pump(state, ctx, busy) {
        ctx.request_repaint_after(wake);
    }
    pump_folder(state, ctx, busy);
}

fn pump_folder(state: &mut AppState, ctx: &egui::Context, busy: bool) {
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
    let gained = focused && !state.doors.focused;
    state.doors.focused = focused;
    let times = state.doors.folder_times;
    let repaints = Arc::clone(&state.doors.folder_repaints);
    let Some(w) = state.doors.watch.as_mut() else { return };
    if w.watcher.as_ref().is_none_or(|x| !x.alive() || x.folder() != w.folder) {
        let known: Known = w.files.iter().map(|(n, f)| (n.clone(), (f.size, f.crc))).collect();
        let repaint_ctx = ctx.clone();
        w.watcher = Some(FolderWatcher::start(w.folder.clone(), known, Arc::new(move || repaint_ctx.request_repaint()), repaints, times));
    }
    if gained && let Some(x) = &w.watcher {
        x.poke();
    }
    let events = w.watcher.as_ref().map(FolderWatcher::drain).unwrap_or_default();
    let mut warnings = Vec::new();
    for e in events {
        match e {
            FolderEvent::Changed { ref name, .. } => {
                w.held.retain(|h| !matches!(h, FolderEvent::Changed { name: n, .. } if n == name));
                w.held.push(e);
            }
            FolderEvent::Unreadable { name, why } => {
                state.doors.folder_counters.refused += 1;
                warnings.push(format!("{name} in the watched folder cannot be read: {why}"));
            }
            FolderEvent::Missing => {
                w.missing = true;
                warnings.push(format!("The watched folder {} is not there any more", w.folder.display()));
            }
            FolderEvent::Back => w.missing = false,
            FolderEvent::TooMany { count } => {
                w.too_many = Some(count);
                warnings.push(format!(
                    "The watched folder holds {count} pictures: only the first {} (by name) are followed",
                    watch::MAX_FOLDER_PICTURES
                ));
            }
        }
    }
    for text in warnings {
        state.toasts.warn(text);
    }
    let Some(w) = state.doors.watch.as_mut() else { return };
    if busy || w.held.is_empty() {
        if !w.held.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        return;
    }
    let held = std::mem::take(&mut w.held);
    apply(state, held);
}

/// Applies settled pictures: one undo step for the batch.
fn apply(state: &mut AppState, events: Vec<FolderEvent>) {
    let Some(doc) = state.doc_mut() else { return };
    let group = doc.begin_edit_group();
    let mut added: Vec<String> = Vec::new();
    let mut updated: Vec<String> = Vec::new();
    for e in events {
        let FolderEvent::Changed { name, bytes, crc } = e else { continue };
        match apply_one(state, &name, bytes, crc) {
            Some(Applied::New) => added.push(name),
            Some(Applied::Updated) => updated.push(name),
            Some(Applied::Same) => state.doors.folder_counters.same += 1,
            None => {}
        }
    }
    if let Some(doc) = state.doc_mut() {
        doc.end_edit_group(group, FOLDER_BATCH_LABEL);
    }
    let c = &mut state.doors.folder_counters;
    c.added += added.len() as u64;
    c.updated += updated.len() as u64;
    let text = match (added.as_slice(), updated.as_slice()) {
        ([], []) => return,
        ([one], []) => format!("{one} from the watched folder is a new tray sticker"),
        ([], [one]) => format!("Updated the sticker of {one} from the watched folder"),
        (a, u) => {
            let plural = |n: usize| if n == 1 { "" } else { "s" };
            let mut parts = Vec::new();
            if !a.is_empty() {
                parts.push(format!("{} new tray sticker{}", a.len(), plural(a.len())));
            }
            if !u.is_empty() {
                parts.push(format!("{} sticker{} updated", u.len(), plural(u.len())));
            }
            format!("From the watched folder: {}", parts.join(", "))
        }
    };
    state.toasts.info(text.clone());
    state.status_hint = text;
}

/// What one settled picture did.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Applied {
    /// A new tray sticker.
    New,
    /// Its sticker was updated.
    Updated,
    /// Its sticker shows this picture already (no history entry).
    Same,
}

/// One settled picture (`None`: refused, said in a toast).
fn apply_one(state: &mut AppState, name: &str, bytes: Arc<[u8]>, crc: u32) -> Option<Applied> {
    let size = bytes.len() as u64;
    let doc = state.doc_mut()?;
    let blob = match doc.add_image_blob(name, bytes) {
        Ok(b) => b,
        Err(e) => {
            state.doors.folder_counters.refused += 1;
            state.toasts.warn(format!("{name} in the watched folder cannot be used: {e}"));
            return None;
        }
    };
    let pixels = match doc.image_size(blob) {
        Ok(p) => p,
        Err(e) => {
            state.doors.folder_counters.refused += 1;
            state.toasts.warn(format!("{name} in the watched folder cannot be used: {e}"));
            return None;
        }
    };
    let known = state.doors.watch.as_ref().and_then(|w| w.files.get(name)).cloned();
    let doc = state.doc_mut()?;
    let target = known.filter(|f| {
        doc.sticker(f.sticker)
            .and_then(|s| s.layer(f.layer))
            .is_some_and(|l| matches!(l.content, StickerContent::Picture(_)))
    });
    let entries = doc.history.undo_len();
    let (sticker, layer, applied) = match target {
        Some(f) => {
            // The size of the picture the layer shows now (an undo may have brought back an older one).
            let old_pixels = match doc.sticker(f.sticker).and_then(|s| s.layer(f.layer)).map(|l| l.content.clone()) {
                Some(StickerContent::Picture(PicturePixels::Blob(b))) => doc.image_size(b).unwrap_or(f.pixels),
                Some(StickerContent::Picture(PicturePixels::Owned { pixels, .. })) => [pixels.width, pixels.height],
                _ => f.pixels,
            };
            let r = doc.set_sticker(
                StickerRef::Id(f.sticker),
                |d| {
                    // An artboard that is still the old picture's own (as the file made it) follows the new size.
                    if d.artboard == IRect::from_size(old_pixels[0], old_pixels[1]) && old_pixels != pixels {
                        d.artboard = IRect::from_size(pixels[0], pixels[1]);
                    }
                    if let Some(l) = d.layer_mut(f.layer) {
                        l.content = StickerContent::Picture(PicturePixels::Blob(blob));
                    }
                },
                FOLDER_UPDATE_LABEL,
                Commit::Final,
            );
            if let Err(e) = r {
                state.doors.folder_counters.refused += 1;
                let what = state.doc().and_then(|d| d.sticker(f.sticker)).map_or_else(String::new, |s| s.name.clone());
                state.toasts.warn(format!("{name} changed in the watched folder, but its sticker {what} cannot be updated: {e}"));
                return None;
            }
            let changed = state.doc().is_some_and(|d| d.history.undo_len() != entries);
            (f.sticker, f.layer, if changed { Applied::Updated } else { Applied::Same })
        }
        None => {
            let stem = Path::new(name).file_stem().map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned());
            let mut sticker = StickerDoc::from_picture(&stem, PicturePixels::Blob(blob), pixels);
            sticker.keep = true;
            let layer = sticker.layers.first().map_or(StickerLayerId(1), |l| l.id);
            let r = doc.add_sticker(sticker);
            let id = crate::dispatch::edited(state, r)?;
            (id, layer, Applied::New)
        }
    };
    if let Some(w) = state.doors.watch.as_mut() {
        w.files.insert(name.to_owned(), WatchedFile { sticker, layer, size, crc, pixels });
    }
    Some(applied)
}

// ---------------------------------------------------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------------------------------------------------

/// The live copy's box "Keep a PNG copy up to date" and, while on, its status line and "Show file in Explorer". Every
/// control is a button with a `PanelHit` rect (tablet rule of 3.10). `None` target: the box is drawn disabled.
pub(crate) fn live_copy_row(
    ui: &mut egui::Ui,
    state: &AppState,
    target: Option<LiveTarget>,
    disabled_why: &str,
    hit: (PanelHit, PanelHit),
    actions: &mut Vec<AppAction>,
    hits: &mut Vec<(PanelHit, egui::Rect)>,
) {
    let entry = target.and_then(|t| state.doors.live.entries.get(&t));
    let saved = state.session.as_ref().is_some_and(|s| s.path.is_some());
    let mut on = entry.is_some();
    let enabled = target.is_some() && (saved || on);
    let tip = "For PhotoCraft's File > Place Linked: a PNG of this picture in the project's links folder, written again \
               half a second after every change";
    let why = if target.is_none() { disabled_why } else { NEEDS_SAVE };
    let resp = ui.add_enabled(enabled, egui::Checkbox::new(&mut on, "Keep a PNG copy up to date")).on_hover_text(tip).on_disabled_hover_text(why);
    hits.push((hit.0, resp.rect));
    if resp.changed()
        && let Some(t) = target
    {
        actions.push(AppAction::Doors(DoorAction::SetLiveCopy(t, on)));
    }
    if let (Some(e), Some(t)) = (entry, target) {
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(live::status_text(e)).weak()).wrap());
            let on_pc = if state.companion.session.is_some() { " (on the PC)" } else { "" };
            let resp = ui.button(format!("Show file{on_pc}"));
            hits.push((hit.1, resp.rect));
            if resp.clicked() {
                actions.push(AppAction::Doors(DoorAction::ShowLiveCopy(t)));
            }
        });
    }
}

/// The tray's watched-folder lines: "Watching: <folder>" with Read again / Show folder / Stop watching, or the button
/// "Watch a folder..." while none is watched.
pub(crate) fn folder_block(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, egui::Rect)>) {
    let on_pc = if state.companion.session.is_some() { " (on the PC)" } else { "" };
    let mut button = |ui: &mut egui::Ui, hit: PanelHit, label: String, tip: &str, action: DoorAction| {
        let resp = ui.button(label).on_hover_text(tip);
        hits.push((hit, resp.rect));
        if resp.clicked() {
            actions.push(AppAction::Doors(action));
        }
    };
    let Some(w) = state.doors.watch.as_ref() else {
        ui.horizontal_wrapped(|ui| {
            button(
                ui,
                PanelHit::TrayWatchFolder,
                format!("Watch a folder...{on_pc}"),
                "Every PNG picture in a folder becomes a tray sticker, and a changed one updates it (PhotoCraft: File > \
                 Generate > Image Assets writes one PNG per layer named like badge.png at every save)",
                DoorAction::WatchFolder(None),
            );
        });
        return;
    };
    let name = w.folder.file_name().map_or_else(|| w.folder.display().to_string(), |n| n.to_string_lossy().into_owned());
    ui.add(egui::Label::new(format!("Watching: {name}")).truncate()).on_hover_text(w.folder.display().to_string());
    let n = w.files.len();
    let line = if w.missing {
        "The folder is not there any more".to_owned()
    } else {
        match n {
            0 => "No PNG pictures there yet".to_owned(),
            1 => "1 picture from it is in the tray".to_owned(),
            n => format!("{n} pictures from it are in the tray"),
        }
    };
    ui.weak(line);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        button(ui, PanelHit::TrayWatchRescan, "Read again".into(), "Read every picture of the folder again and update its sticker", DoorAction::RescanFolder);
        button(ui, PanelHit::TrayWatchShow, format!("Show folder{on_pc}"), "Show the folder in Explorer", DoorAction::ShowFolder);
        button(ui, PanelHit::TrayWatchStop, "Stop watching".into(), "Stop following the folder (its stickers stay in the tray)", DoorAction::StopWatching);
    });
}
