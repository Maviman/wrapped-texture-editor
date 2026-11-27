// Owner: stickers P8 (STICKER_PLAN 3.8 "Library", E7; ARCHITECTURE 8.15). The sticker library of the Sticker tab: a
// folder of `.wtes` files (`Prefs::sticker_library_dir`, default `%APPDATA%\WrappedTextureEditor\stickers\`; "Choose
// folder..." changes it). The folder is listed on a job thread (each file's name and `preview.png` through
// `wte_core::read_wtes_preview_from`, never the layers); the UI takes at most one finished entry per frame and uploads its
// thumbnail then. A click on an entry adds the sticker to this project's tray (`read_wtes` + `Document::import_wtes`:
// one entry "Add Sticker", kept); its stamp styles, when the file has them, become the Stamp tool's styles. "Save to
// Library" writes a tray sticker or the edited sticker (with its stamp's styles) as `<name>.wtes` into the folder.
//
// Self-test runs and unit tests never use the real folder: without a folder in the prefs (a scenario sets one inside
// its output folder) the library has no folder at all.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use eframe::egui;
use egui::{Rect, Sense, vec2};
use wte_core::{PixelBuf, StickerId};

use crate::action::AppAction;
use crate::panels::paint_thumb;
use crate::state::{AppState, PanelHit};
use crate::sticker::StickerAction;

/// Side of a library thumbnail (points).
const THUMB: f32 = 52.0;
/// Most files listed (a folder with more shows the first ones by name).
pub(crate) const MAX_FILES: usize = 500;
/// Longest side of a library thumbnail picture (pixels; check P8: the 512-px `preview.png` is made this small on the
/// job thread, so 500 entries hold about 500 x 64 KB, not 500 x 1 MB, in RAM and on the GPU).
pub(crate) const THUMB_PX: u32 = 128;

/// One listed file.
pub(crate) struct LibItem {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    /// Width / height of the preview (1 without one).
    aspect: f32,
    tex: Option<egui::TextureHandle>,
}

/// What the listing job sends: one file, or the end of the folder.
enum Listed {
    Item(PathBuf, String, Option<PixelBuf>),
    Done,
}

/// The library's state (`StickerUi::library`).
#[derive(Default)]
pub(crate) struct LibraryState {
    /// The folder the list belongs to (`None`: not listed yet).
    listed: Option<PathBuf>,
    pub(crate) items: Vec<LibItem>,
    job: Option<mpsc::Receiver<Listed>>,
    /// The running listing has read every file (set by its thread; the idle check needs no frame of the tab).
    finished: Arc<AtomicBool>,
    /// A listing was asked for (Refresh, a save).
    stale: bool,
    /// Listings started (tests).
    pub(crate) listings: u64,
}

impl LibraryState {
    /// True while the listing job reads files (the self-test's idle check waits for it). A finished job whose results
    /// the Sticker tab has not taken yet is not busy: the tab may not be shown.
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some() && !self.finished.load(Ordering::Acquire)
    }

    /// Lists the folder again in the next frame of the Sticker tab (Refresh, a save, another folder).
    pub(crate) fn refresh(&mut self) {
        self.stale = true;
    }
}

/// The library folder: the one in the prefs, else (only in a normal run) `%APPDATA%\WrappedTextureEditor\stickers`.
pub(crate) fn folder(state: &AppState) -> Option<PathBuf> {
    if let Some(dir) = state.prefs.sticker_library_dir.clone() {
        return Some(dir);
    }
    if state.selftest || cfg!(test) {
        return None;
    }
    crate::prefs::app_data_dir().map(|d| d.join("stickers"))
}

/// The `.wtes` files of `dir`, by name (at most `MAX_FILES`).
fn wtes_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && crate::dnd::extension_of(p) == wte_core::WTES_EXTENSION)
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    files.truncate(MAX_FILES);
    files
}

/// Starts the listing job for `dir` (a running one is dropped: its results go nowhere).
fn start_listing(ctx: &egui::Context, lib: &mut LibraryState, dir: PathBuf) {
    lib.items.clear();
    lib.listed = Some(dir.clone());
    lib.stale = false;
    lib.listings += 1;
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let finished = Arc::new(AtomicBool::new(false));
    lib.finished = Arc::clone(&finished);
    let spawned = std::thread::Builder::new().name("Sticker library".to_owned()).spawn(move || {
        for path in wtes_files(&dir) {
            // Check P8: only the zip directory, the manifest and the preview are read (the file was read whole).
            let read = std::fs::File::open(&path).ok().and_then(|f| wte_core::read_wtes_preview_from(std::io::BufReader::new(f)));
            let Some((name, picture)) = read else { continue };
            let picture = picture.filter(|p| p.width > 0 && p.height > 0).map(|p| {
                if p.width.max(p.height) <= THUMB_PX { p } else { wte_core::thumbnail(p.view(), p.width, p.height, THUMB_PX) }
            });
            let name = if name.trim().is_empty() { path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()) } else { name };
            if tx.send(Listed::Item(path, name, picture)).is_err() {
                finished.store(true, Ordering::Release);
                return;
            }
            ctx.request_repaint();
        }
        let _ = tx.send(Listed::Done);
        finished.store(true, Ordering::Release);
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => lib.job = Some(rx),
        Err(e) => log::warn!("the sticker library cannot be listed: {e}"),
    }
}

/// Once per frame of the Sticker tab: (re)starts the listing when the folder changed or a refresh was asked for, and
/// takes at most one listed file (its thumbnail uploaded now).
pub(crate) fn frame(ctx: &egui::Context, state: &mut AppState) {
    let dir = folder(state);
    let lib = &mut state.sticker_ui.library;
    match dir {
        None => {
            lib.items.clear();
            lib.listed = None;
            lib.job = None;
            return;
        }
        Some(d) if lib.listed.as_ref() != Some(&d) || lib.stale => start_listing(ctx, lib, d),
        Some(_) => {}
    }
    let Some(rx) = lib.job.as_ref() else { return };
    match rx.try_recv() {
        Ok(Listed::Item(path, name, picture)) => {
            let tex = picture.as_ref().map(|p| {
                let image = egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.data);
                ctx.load_texture(format!("sticker-library-{}", lib.items.len()), image, egui::TextureOptions::LINEAR)
            });
            let aspect = picture.as_ref().map_or(1.0, |p| p.width as f32 / p.height.max(1) as f32);
            lib.items.push(LibItem { path, name, aspect, tex });
            ctx.request_repaint();
        }
        Ok(Listed::Done) | Err(mpsc::TryRecvError::Disconnected) => lib.job = None,
        Err(mpsc::TryRecvError::Empty) => {}
    }
}

/// The Library section of the Sticker tab.
pub(crate) fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<AppAction>, hits: &mut Vec<(PanelHit, Rect)>) {
    frame(ui.ctx(), state);
    let on_pc = if state.companion.session.is_some() { " (on the PC)" } else { "" };
    let dir = folder(state);
    match &dir {
        Some(d) => {
            ui.add(egui::Label::new(egui::RichText::new(d.display().to_string()).small().weak()).truncate());
        }
        None => {
            ui.weak("No library folder: choose one.");
        }
    }
    let lib = &state.sticker_ui.library;
    if dir.is_some() && lib.items.is_empty() && !lib.busy() {
        ui.weak("No stickers saved here yet: \"Save to Library\" on a tray sticker puts one here.");
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (i, item) in state.sticker_ui.library.items.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::click());
            paint_thumb(ui.painter(), rect, item.tex.as_ref().map(egui::TextureHandle::id), item.aspect);
            let resp = resp.on_hover_text(format!("{}. Click: add it to this project's tray", item.name));
            hits.push((PanelHit::LibraryItem(i as u32), rect));
            if resp.clicked() {
                actions.push(AppAction::Sticker(StickerAction::LibraryAdd(item.path.clone())));
            }
        }
    });
    if state.sticker_ui.library.busy() {
        ui.weak("Reading the library...");
    }
    ui.horizontal_wrapped(|ui| {
        let r = ui.button(format!("Choose folder...{on_pc}")).on_hover_text("The folder the library lists and saves to");
        hits.push((PanelHit::LibraryChoose, r.rect));
        if r.clicked() {
            actions.push(AppAction::Sticker(StickerAction::ChooseLibraryFolder(None)));
        }
        let r = ui.add_enabled(dir.is_some(), egui::Button::new("Refresh")).on_hover_text("List the folder again");
        hits.push((PanelHit::LibraryRefresh, r.rect));
        if r.clicked() {
            actions.push(AppAction::Sticker(StickerAction::LibraryRefresh));
        }
    });
}

/// A file name for `name` in `dir` that is not taken: `<name>.wtes`, `<name> (2).wtes`, ... (characters Windows
/// refuses replaced).
pub(crate) fn free_path(dir: &Path, name: &str) -> PathBuf {
    let clean: String = name.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches(['.', ' ']).to_owned();
    let base = if clean.is_empty() { "Sticker".to_owned() } else { clean };
    let ext = wte_core::WTES_EXTENSION;
    let first = dir.join(format!("{base}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{base} ({n}).{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// "Save to Library": the sticker `id` (`None`: the sticker of the open editor, with its stamp's styles) as a `.wtes`
/// file in the library folder. A toast says where.
pub(crate) fn save(state: &mut AppState, id: Option<StickerId>) {
    let Some(dir) = folder(state) else {
        state.toasts.warn("Choose a library folder first (Sticker tab, Library)");
        return;
    };
    let (id, styles) = match id {
        Some(id) => (Some(id), None),
        None => {
            let edit = crate::sticker::edit_of(state).cloned();
            match edit.map(|e| e.target) {
                Some(crate::state::StickerTarget::Tray(id)) => (Some(id), None),
                Some(crate::state::StickerTarget::Stamp(stamp)) => {
                    let object = state.doc().and_then(|d| d.stamp(stamp)).cloned();
                    match object.map(|o| (o.source, o.styles)) {
                        Some((wte_core::StampSource::Sticker { sticker }, styles)) => (Some(sticker), Some(styles)),
                        _ => (None, None),
                    }
                }
                None => (None, None),
            }
        }
    };
    let Some(id) = id else {
        state.status_hint = "Only a layered sticker can be saved to the library (edit the picture once, or convert the text)".to_owned();
        return;
    };
    let styles = styles.filter(|s| !s.is_default());
    let Some(doc) = state.doc_mut() else { return };
    let name = doc.sticker(id).map(|s| s.name.clone()).unwrap_or_default();
    let bytes = match doc.sticker_wtes(id, styles.as_ref()) {
        Ok(b) => b,
        Err(e) => {
            state.toasts.error(format!("Could not save the sticker: {e}"));
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        state.toasts.error(format!("Could not make the library folder {}: {e}", dir.display()));
        return;
    }
    let path = free_path(&dir, &name);
    let file = path.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
    match std::fs::write(&path, bytes) {
        Ok(()) => {
            state.toasts.info(format!("Saved {file} to the sticker library"));
            state.sticker_ui.library.stale = true;
        }
        Err(e) => state.toasts.error(format!("Could not write {file}: {e}")),
    }
}

/// A click on a library entry: the file's sticker is added to the tray (kept; one entry) and becomes the selected tray
/// item and the Stamp tool's source; the file's stamp styles, if any, the Stamp tool's styles.
pub(crate) fn add(state: &mut AppState, path: &Path) {
    let file = path.file_name().map_or_else(|| path.display().to_string(), |f| f.to_string_lossy().into_owned());
    let read = std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| wte_core::read_wtes(&b).map_err(|e| e.to_string()));
    let wtes = match read {
        Ok(w) => w,
        Err(e) => {
            state.toasts.error(format!("Could not read {file}: {e}"));
            return;
        }
    };
    for w in &wtes.warnings {
        log::warn!("{file}: {w}");
    }
    let Some(doc) = state.doc_mut() else { return };
    let r = doc.import_wtes(&wtes);
    let Some(id) = crate::dispatch::edited(state, r) else { return };
    let name = state.doc().and_then(|d| d.sticker(id)).map(|s| s.name.clone()).unwrap_or_default();
    state.sticker_ui.tray_selected = Some(id);
    state.tools.stamp.source = crate::state::StampToolSource::Sticker(id);
    state.tools.stamp.source_name = name.clone();
    if let Some(styles) = wtes.styles {
        state.tools.stamp.styles = styles;
    }
    state.status_hint = format!("{name} is in the tray: click the model with the Stamp tool to place it");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_names_never_overwrite_and_lose_bad_characters() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join("sticker").join("unit_library_names");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = free_path(&dir, "My: sticker?");
        assert_eq!(a.file_name().unwrap().to_string_lossy(), "My_ sticker_.wtes");
        std::fs::write(&a, b"x").unwrap();
        let b = free_path(&dir, "My: sticker?");
        assert_eq!(b.file_name().unwrap().to_string_lossy(), "My_ sticker_ (2).wtes");
        assert_eq!(free_path(&dir, " . ").file_name().unwrap().to_string_lossy(), "Sticker.wtes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Check P8: a library entry keeps a small thumbnail (at most `THUMB_PX` a side) and its aspect, not the 512-px
    /// preview picture; the job reads the file through the zip directory.
    #[test]
    fn library_thumbnails_are_small() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join("sticker").join("unit_library_thumbs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut doc = wte_core::testutil::doc_from_raw(wte_core::samples::flat_quad(), 64);
        let mut s = wte_core::StickerDoc::new("wide", 600, 300);
        let id = s.alloc_layer();
        let pixels = std::sync::Arc::new(PixelBuf::filled(600, 300, [200, 40, 40, 255]));
        let v = doc.next_pixel_version();
        s.layers.push(wte_core::StickerLayer::new(id, "p", wte_core::StickerContent::Picture(wte_core::PicturePixels::Owned { pixels, version: v })));
        let sid = doc.add_sticker(s).unwrap();
        std::fs::write(dir.join("wide.wtes"), doc.sticker_wtes(sid, None).unwrap()).unwrap();
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        state.prefs.sticker_library_dir = Some(dir.clone());
        let ctx = egui::Context::default();
        for _ in 0..2000 {
            frame(&ctx, &mut state);
            if !state.sticker_ui.library.busy() && !state.sticker_ui.library.items.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let item = state.sticker_ui.library.items.first().expect("listed");
        let size = item.tex.as_ref().expect("a thumbnail").size();
        assert_eq!(size, [THUMB_PX as usize, THUMB_PX as usize / 2], "a small thumbnail of the 512 x 256 preview");
        assert!((item.aspect - 2.0).abs() < 1e-3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_unit_test_or_self_test_has_no_library_folder_without_one_in_the_prefs() {
        let mut state = AppState::new(crate::state::Prefs::default(), false);
        assert_eq!(folder(&state), None, "never the real %APPDATA% folder in a test");
        state.prefs.sticker_library_dir = Some(PathBuf::from("X:\\lib"));
        assert_eq!(folder(&state), Some(PathBuf::from("X:\\lib")));
    }
}
