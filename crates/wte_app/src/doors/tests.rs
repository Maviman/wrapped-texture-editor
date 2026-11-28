//! Tests of stickers S2 (STICKER_PLAN 7 "S2"): the live PNG copy and the watched folder. Files live under
//! `target\wte-test-tmp\doors\` (never `%TEMP%` or the real `%LOCALAPPDATA%`); nothing is started (the stub launcher).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use wte_core::{Commit, LayerId, PixelBuf, PngLevel, SampleKind, StickerDoc, StickerId, StickerRef, TextureSetId};

use super::live::{LiveStatus, LiveTarget};
use super::watch::{FolderEvent, FolderTimes, FolderWatcher, Known};
use super::*;
use crate::state::AppState;

/// A fresh folder under `target\wte-test-tmp\doors\`, removed when the guard is dropped.
struct TmpDir(PathBuf);

impl std::ops::Deref for TmpDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> TmpDir {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wte-test-tmp/doors").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TmpDir(dir)
}

fn png_of(p: &PixelBuf) -> Vec<u8> {
    wte_core::encode_png(p.width, p.height, &p.data, PngLevel::Fast).unwrap()
}

fn decode(path: &Path) -> PixelBuf {
    wte_core::decode_image(&std::fs::read(path).unwrap(), None, 8192).unwrap().pixels
}

/// The Sphere sample "saved" at `<dir>\proj.wtep` (the path only), with short delays.
fn saved_state(dir: &Path) -> AppState {
    let mut state = crate::tools::test_support::state_with(SampleKind::Sphere);
    state.session.as_mut().unwrap().path = Some(dir.join("proj.wtep"));
    state.doors.live.delay = Duration::from_millis(60);
    state.doors.live.retry = Duration::from_millis(60);
    state
}

fn tray_sticker(state: &mut AppState, name: &str, p: PixelBuf) -> StickerId {
    let doc = state.doc_mut().unwrap();
    let blob = doc.add_image_blob(&format!("{name}.png"), Arc::from(png_of(&p))).unwrap();
    let mut s = StickerDoc::from_picture(name, wte_core::PicturePixels::Blob(blob), [p.width, p.height]);
    s.keep = true;
    doc.add_sticker(s).unwrap()
}

fn filled(w: u32, h: u32, rgba: [u8; 4]) -> PixelBuf {
    PixelBuf::filled(w, h, rgba)
}

/// Pumps the live copies until `done` or `limit`.
fn pump_until(state: &mut AppState, ctx: &egui::Context, limit: Duration, done: impl Fn(&AppState) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        live::pump(state, ctx, false);
        if done(state) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn copy_path(state: &AppState, target: LiveTarget) -> PathBuf {
    live::path_of(state, target).unwrap()
}

fn status(state: &AppState, target: LiveTarget) -> LiveStatus {
    state.doors.live.entries.get(&target).unwrap().status.clone()
}

// ---------------------------------------------------------------------------------------------------------------------
// The live PNG copy
// ---------------------------------------------------------------------------------------------------------------------

/// A sticker's copy: written at once when turned on, then again only after the delay since its last change, as the
/// picture `sticker_picture` draws; nothing is written while nothing changes; turning it off keeps the file.
#[test]
fn a_stickers_copy_is_written_at_once_then_after_each_settled_change() {
    let dir = tmp("sticker_copy");
    let mut state = saved_state(&dir);
    let ctx = egui::Context::default();
    let id = tray_sticker(&mut state, "Badge", filled(24, 16, [200, 40, 30, 255]));
    let target = LiveTarget::Sticker(id);
    let undo = state.doc().unwrap().history.undo_len();
    run(&mut state, DoorAction::SetLiveCopy(target, true));
    assert!(state.doc().unwrap().is_modified(), "turning it on is a project change");
    assert_eq!(state.doc().unwrap().history.undo_len(), undo, "but no history entry");
    assert_eq!(state.doors.live.entries[&target].file, format!("Badge_k{}.png", id.0));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 1));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(2), |s| status(s, target) == LiveStatus::UpToDate));
    let path = copy_path(&state, target);
    assert_eq!(path, wte_core::link::links_folder(&dir.join("proj.wtep")).join(format!("Badge_k{}.png", id.0)));
    assert_eq!(decode(&path), state.doc_mut().unwrap().sticker_picture(id, None, 1.0).unwrap());
    // Nothing changes: no more jobs.
    for _ in 0..20 {
        live::pump(&mut state, &ctx, false);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state.doors.live.counters.jobs, 1);
    // A change: not before the delay, then once.
    state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |d| d.layers[0].opacity = 0.5, "Layer Opacity", Commit::Final).unwrap();
    let changed = Instant::now();
    live::pump(&mut state, &ctx, false);
    assert_eq!(state.doors.live.counters.jobs, 1, "not at once");
    assert_eq!(status(&state, target), LiveStatus::Waiting);
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 2));
    assert!(changed.elapsed() >= Duration::from_millis(60), "written after the delay");
    assert_eq!(decode(&path), state.doc_mut().unwrap().sticker_picture(id, None, 1.0).unwrap());
    // Off: the file stays.
    run(&mut state, DoorAction::SetLiveCopy(target, false));
    assert!(state.doors.live.entries.is_empty() && path.exists());
}

/// Changes closer together than the delay give one write (the copy waits for the last one).
#[test]
fn quick_changes_give_one_write() {
    let dir = tmp("quick");
    let mut state = saved_state(&dir);
    state.doors.live.delay = Duration::from_millis(300);
    let ctx = egui::Context::default();
    let id = tray_sticker(&mut state, "Quick", filled(8, 8, [1, 2, 3, 255]));
    let target = LiveTarget::Sticker(id);
    run(&mut state, DoorAction::SetLiveCopy(target, true));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 1));
    for k in 0..5 {
        state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |d| d.layers[0].opacity = 0.9 - 0.1 * k as f32, "Layer Opacity", Commit::Final).unwrap();
        live::pump(&mut state, &ctx, false);
        std::thread::sleep(Duration::from_millis(40));
    }
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 2));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(2), |s| status(s, target) == LiveStatus::UpToDate));
    assert_eq!(state.doors.live.counters.jobs, 2, "one write for five quick changes");
}

/// A raster paint layer's copy is the layer's pixels at the texture's size; a never painted layer is transparent.
#[test]
fn a_paint_layers_copy_is_its_pixels() {
    let dir = tmp("layer_copy");
    let mut state = saved_state(&dir);
    let ctx = egui::Context::default();
    let doc = state.doc_mut().unwrap();
    let [w, h] = [doc.sets[0].width, doc.sets[0].height];
    let layer: LayerId = doc.add_paint_layer_with_pixels(TextureSetId(0), "Decal", PixelBuf::new(w, h), "New Layer").unwrap();
    let target = LiveTarget::Layer(layer);
    run(&mut state, DoorAction::SetLiveCopy(target, true));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 1));
    let path = copy_path(&state, target);
    assert!(path.file_name().unwrap().to_string_lossy().ends_with(&format!("_l{}.png", layer.0)));
    let got = decode(&path);
    assert_eq!([got.width, got.height], [w, h]);
    assert!(got.data.iter().all(|b| *b == 0), "transparent");
    let mut p = PixelBuf::new(w, h);
    for y in 0..h / 2 {
        for x in 0..w / 3 {
            p.set(x, y, [10, 220, 90, 255]);
        }
    }
    state.doc_mut().unwrap().replace_layer_pixels(layer, p.clone(), "Paint").unwrap();
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 2));
    assert_eq!(decode(&path), p);
    // A stamp layer is not offered (it is not a raster layer).
    let stamp_layer = state.doc().unwrap().sets[0].layers.iter().find(|l| !l.is_raster()).map(|l| l.id);
    if let Some(l) = stamp_layer {
        run(&mut state, DoorAction::SetLiveCopy(LiveTarget::Layer(l), true));
        assert!(!state.doors.live.entries.contains_key(&LiveTarget::Layer(l)));
    }
}

/// Without a project file nothing is turned on (the links folder is beside the project): the sentence says so.
#[test]
fn a_copy_needs_a_saved_project() {
    let mut state = crate::tools::test_support::state_with(SampleKind::Sphere);
    let id = tray_sticker(&mut state, "Unsaved", filled(4, 4, [9; 4]));
    run(&mut state, DoorAction::SetLiveCopy(LiveTarget::Sticker(id), true));
    assert!(state.doors.live.entries.is_empty());
    assert_eq!(state.status_hint, NEEDS_SAVE);
}

/// A copy restored with the project whose file holds the picture already is not written again (its time stays).
#[test]
fn a_restored_copy_with_the_same_picture_is_not_rewritten() {
    let dir = tmp("restored");
    let mut state = saved_state(&dir);
    let ctx = egui::Context::default();
    let id = tray_sticker(&mut state, "Same", filled(12, 12, [50, 60, 70, 255]));
    let target = LiveTarget::Sticker(id);
    run(&mut state, DoorAction::SetLiveCopy(target, true));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 1));
    let path = copy_path(&state, target);
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let saved = live::saved(&state.doors.live, state.doc().unwrap());
    assert_eq!(saved.len(), 1);
    state.doors.reset();
    let doc = &state.session.as_ref().unwrap().doc;
    live::restore(&mut state.doors.live, &saved, doc);
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.same == 1));
    assert_eq!(state.doors.live.counters.writes, 1);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
    // Unknown kinds, folder parts in the name and missing targets are left out.
    let bad = vec![
        live::SavedLiveCopy { kind: "photo".into(), id: id.0, file: "a.png".into() },
        live::SavedLiveCopy { kind: "sticker".into(), id: id.0, file: "..\\evil.png".into() },
        live::SavedLiveCopy { kind: "sticker".into(), id: 99_999, file: "b.png".into() },
        live::SavedLiveCopy { kind: "sticker".into(), id: id.0, file: "c.psd".into() },
    ];
    state.doors.reset();
    let doc = &state.session.as_ref().unwrap().doc;
    live::restore(&mut state.doors.live, &bad, doc);
    assert!(state.doors.live.entries.is_empty());
}

/// Nothing starts while an interaction is open; a Save As moves the copy into the new links folder.
#[test]
fn a_copy_waits_for_interactions_and_follows_save_as() {
    let dir = tmp("wait");
    let mut state = saved_state(&dir);
    let ctx = egui::Context::default();
    let id = tray_sticker(&mut state, "Wait", filled(6, 6, [5, 5, 200, 255]));
    let target = LiveTarget::Sticker(id);
    run(&mut state, DoorAction::SetLiveCopy(target, true));
    for _ in 0..10 {
        live::pump(&mut state, &ctx, true);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state.doors.live.counters.jobs, 0, "busy: nothing starts");
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 1));
    state.session.as_mut().unwrap().path = Some(dir.join("moved.wtep"));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 2));
    let moved = wte_core::link::links_folder(&dir.join("moved.wtep")).join(&state.doors.live.entries[&target].file);
    assert_eq!(copy_path(&state, target), moved);
    assert!(moved.exists());
}

/// A write that fails (a folder where the file should be) is tried again after the retry delay and succeeds once the
/// way is free; a deleted sticker is "gone" (not written) until an undo brings it back.
#[test]
fn a_failed_write_is_tried_again_and_a_deleted_sticker_waits() {
    let dir = tmp("retry");
    let mut state = saved_state(&dir);
    let ctx = egui::Context::default();
    let id = tray_sticker(&mut state, "Retry", filled(6, 6, [5, 99, 5, 255]));
    let target = LiveTarget::Sticker(id);
    run(&mut state, DoorAction::SetLiveCopy(target, true));
    let path = copy_path(&state, target);
    std::fs::create_dir_all(&path).unwrap();
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| matches!(status(s, target), LiveStatus::Failed(_))));
    assert!(live::status_text(&state.doors.live.entries[&target]).contains("trying again"));
    std::fs::remove_dir_all(&path).unwrap();
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 1));
    assert!(path.is_file());
    state.doc_mut().unwrap().remove_sticker(id).unwrap();
    live::pump(&mut state, &ctx, false);
    assert_eq!(status(&state, target), LiveStatus::Gone);
    assert!(live::saved(&state.doors.live, state.doc().unwrap()).is_empty(), "not saved while gone");
    state.doc_mut().unwrap().undo().unwrap();
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(2), |s| status(s, target) == LiveStatus::UpToDate));
}

/// `sticker_key` of the copy reads no pixels, and a sticker renamed keeps its copy's file name.
#[test]
fn a_renamed_sticker_keeps_its_copys_file() {
    let dir = tmp("rename");
    let mut state = saved_state(&dir);
    let ctx = egui::Context::default();
    let id = tray_sticker(&mut state, "Old name", filled(6, 6, [1, 1, 1, 255]));
    let target = LiveTarget::Sticker(id);
    run(&mut state, DoorAction::SetLiveCopy(target, true));
    assert!(pump_until(&mut state, &ctx, Duration::from_secs(10), |s| s.doors.live.counters.writes == 1));
    state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |d| d.name = "New name".into(), "Rename Sticker", Commit::Final).unwrap();
    for _ in 0..10 {
        live::pump(&mut state, &ctx, false);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state.doors.live.entries[&target].file, format!("Old name_k{}.png", id.0));
    assert_eq!(state.doors.live.counters.writes, 1, "a name is not part of the picture");
}

// ---------------------------------------------------------------------------------------------------------------------
// The folder worker
// ---------------------------------------------------------------------------------------------------------------------

fn fast_times() -> FolderTimes {
    FolderTimes { poll: Duration::from_millis(20), settle: Duration::from_millis(10), give_up: Duration::from_millis(400) }
}

fn start(folder: &Path, known: Known) -> (FolderWatcher, Arc<AtomicU64>) {
    let repaints = Arc::new(AtomicU64::new(0));
    let w = FolderWatcher::start(folder.to_path_buf(), known, Arc::new(|| {}), Arc::clone(&repaints), fast_times());
    (w, repaints)
}

/// Waits for events until `n` arrived or `limit` passed.
fn events(w: &FolderWatcher, n: usize, limit: Duration) -> Vec<FolderEvent> {
    let start = Instant::now();
    let mut out = Vec::new();
    while out.len() < n && start.elapsed() < limit {
        out.extend(w.drain());
        std::thread::sleep(Duration::from_millis(5));
    }
    out
}

fn write_via_rename(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("writing");
    std::fs::write(&tmp, bytes).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

fn changed_names(ev: &[FolderEvent]) -> Vec<String> {
    let mut v: Vec<String> = ev.iter().filter_map(|e| if let FolderEvent::Changed { name, .. } = e { Some(name.clone()) } else { None }).collect();
    v.sort();
    v
}

/// New and changed PNG files are reported once each when settled; other files, subfolders and unchanged files are
/// not; with nothing changing the repaint counter stays.
#[test]
fn the_worker_reports_new_and_changed_pictures_once() {
    let dir = tmp("worker");
    std::fs::write(dir.join("a.png"), png_of(&filled(4, 4, [1, 2, 3, 255]))).unwrap();
    std::fs::write(dir.join("notes.txt"), b"x").unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("c.png"), png_of(&filled(2, 2, [9; 4]))).unwrap();
    let (w, repaints) = start(&dir, Known::new());
    let ev = events(&w, 1, Duration::from_secs(5));
    assert_eq!(changed_names(&ev), vec!["a.png".to_owned()]);
    std::thread::sleep(Duration::from_millis(150));
    assert!(w.drain().is_empty(), "reported once");
    let quiet = repaints.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(repaints.load(Ordering::SeqCst), quiet, "no repaints while nothing changes");
    // A new file and a changed one (through a temp file and a rename, and in place).
    std::fs::write(dir.join("B.PNG"), png_of(&filled(3, 5, [7, 7, 7, 255]))).unwrap();
    write_via_rename(&dir.join("a.png"), &png_of(&filled(4, 4, [200, 2, 3, 255])));
    let ev = events(&w, 2, Duration::from_secs(5));
    assert_eq!(changed_names(&ev), vec!["B.PNG".to_owned(), "a.png".to_owned()]);
    let FolderEvent::Changed { bytes, crc, .. } = ev.iter().find(|e| matches!(e, FolderEvent::Changed { name, .. } if name == "a.png")).unwrap() else {
        unreachable!()
    };
    assert_eq!(*crc, crate::link::watch::crc(bytes));
    assert_eq!(wte_core::decode_image(bytes, None, 64).unwrap().pixels, filled(4, 4, [200, 2, 3, 255]));
    // Rescan: everything again.
    w.rescan();
    let ev = events(&w, 2, Duration::from_secs(5));
    assert_eq!(changed_names(&ev), vec!["B.PNG".to_owned(), "a.png".to_owned()]);
}

/// Known content (a reopened project) is not reported; a changed known file is.
#[test]
fn known_pictures_are_not_reported_again() {
    let dir = tmp("known");
    let a = png_of(&filled(4, 4, [1, 2, 3, 255]));
    let b = png_of(&filled(4, 4, [4, 5, 6, 255]));
    std::fs::write(dir.join("a.png"), &a).unwrap();
    std::fs::write(dir.join("b.png"), &b).unwrap();
    let mut known = Known::new();
    known.insert("a.png".into(), (a.len() as u64, crate::link::watch::crc(&a)));
    known.insert("b.png".into(), (b.len() as u64, crate::link::watch::crc(&b) ^ 1));
    let (w, _) = start(&dir, known);
    let ev = events(&w, 1, Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(150));
    let mut all = ev;
    all.extend(w.drain());
    assert_eq!(changed_names(&all), vec!["b.png".to_owned()], "only the one whose content differs");
}

/// A file that does not decode is waited for (a half-written save) and reported once as unreadable after the give-up
/// time; a later good save of it is reported. A missing folder is said once, and its return.
#[test]
fn half_written_and_broken_pictures_and_a_missing_folder() {
    let dir = tmp("broken");
    let good = png_of(&filled(8, 8, [10, 20, 30, 255]));
    std::fs::write(dir.join("half.png"), &good[..good.len() / 2]).unwrap();
    let (w, _) = start(&dir, Known::new());
    std::thread::sleep(Duration::from_millis(100));
    assert!(w.drain().is_empty(), "waited for");
    // The writer finishes within the give-up time: reported as a picture.
    std::fs::write(dir.join("half.png"), &good).unwrap();
    assert_eq!(changed_names(&events(&w, 1, Duration::from_secs(5))), vec!["half.png".to_owned()]);
    // A file that stays broken: one Unreadable after the give-up time.
    std::fs::write(dir.join("broken.png"), b"not a picture").unwrap();
    let ev = events(&w, 1, Duration::from_secs(5));
    assert!(matches!(ev.as_slice(), [FolderEvent::Unreadable { name, .. }] if name == "broken.png"), "{ev:?}");
    std::thread::sleep(Duration::from_millis(500));
    assert!(w.drain().is_empty(), "said once");
    // The folder goes away and comes back.
    std::fs::remove_file(dir.join("broken.png")).unwrap();
    let moved = dir.with_extension("away");
    let _ = std::fs::remove_dir_all(&moved);
    std::fs::rename(&*dir, &moved).unwrap();
    let ev = events(&w, 1, Duration::from_secs(5));
    assert!(matches!(ev.as_slice(), [FolderEvent::Missing]), "{ev:?}");
    std::fs::rename(&moved, &*dir).unwrap();
    let ev = events(&w, 1, Duration::from_secs(5));
    assert!(matches!(ev.as_slice(), [FolderEvent::Back]), "{ev:?}");
    std::thread::sleep(Duration::from_millis(150));
    assert!(w.drain().is_empty(), "its known picture is not reported again");
    std::fs::write(dir.join("half.png"), png_of(&filled(8, 8, [99, 20, 30, 255]))).unwrap();
    assert_eq!(changed_names(&events(&w, 1, Duration::from_secs(5))), vec!["half.png".to_owned()], "a change after it came back");
}

/// More pictures than are followed: said once; only the first ones by name are reported.
#[test]
fn a_folder_with_too_many_pictures_is_capped() {
    let dir = tmp("many");
    let bytes = png_of(&filled(1, 1, [1, 1, 1, 255]));
    for i in 0..watch::MAX_FOLDER_PICTURES + 3 {
        std::fs::write(dir.join(format!("p{i:04}.png")), &bytes).unwrap();
    }
    let (w, _) = start(&dir, Known::new());
    let ev = events(&w, watch::MAX_FOLDER_PICTURES + 1, Duration::from_secs(20));
    assert!(ev.iter().any(|e| matches!(e, FolderEvent::TooMany { count } if *count == watch::MAX_FOLDER_PICTURES + 3)));
    let names = changed_names(&ev);
    assert_eq!(names.len(), watch::MAX_FOLDER_PICTURES);
    assert_eq!(names.last().map(String::as_str), Some("p0255.png"));
    assert!(watch::is_picture_name("x.PnG") && !watch::is_picture_name(".png") && !watch::is_picture_name("~a.png") && !watch::is_picture_name("a.png.tmp"));
}

// ---------------------------------------------------------------------------------------------------------------------
// The folder in the app
// ---------------------------------------------------------------------------------------------------------------------

fn changed(name: &str, p: &PixelBuf) -> FolderEvent {
    let bytes = png_of(p);
    let crc = crate::link::watch::crc(&bytes);
    FolderEvent::Changed { name: name.to_owned(), bytes: Arc::from(bytes), crc }
}

fn watched_state(dir: &Path) -> AppState {
    let mut state = saved_state(dir);
    state.doors.watch = Some(WatchedFolder::new(dir.join("assets")));
    state
}

/// A new picture becomes a kept tray sticker named after the file (one entry "Add Sticker"); its change updates the
/// same sticker (one entry "Update Sticker from Folder") and keeps a placed stamp where it is (a new size refits it
/// as a crop does); undoing it leaves the folder alone; several at once are one entry "Stickers from Folder".
#[test]
fn pictures_become_and_update_tray_stickers() {
    let dir = tmp("apply");
    let mut state = watched_state(&dir);
    let undo = state.doc().unwrap().history.undo_len();
    apply(&mut state, vec![changed("badge.png", &filled(20, 10, [200, 0, 0, 255]))]);
    let f = state.doors.watch.as_ref().unwrap().files["badge.png"].clone();
    let doc = state.doc().unwrap();
    let s = doc.sticker(f.sticker).unwrap();
    assert!(s.keep && s.name == "badge" && s.size() == [20, 10]);
    assert_eq!(doc.undo_label(), Some("Add Sticker"));
    assert_eq!(doc.history.undo_len(), undo + 1);
    assert!(crate::sticker::tray::tray_stickers(&state).contains(&f.sticker));
    assert_eq!(state.doors.folder_counters.added, 1);
    // Place it.
    let doc = state.doc_mut().unwrap();
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.0, 0.5), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::new(0.4, 0.2), 0.6);
    let stamp = doc.add_stamp("badge", wte_core::StampSource::Sticker { sticker: f.sticker }, wte_core::Placement::Projected(p), TextureSetId(0)).unwrap();
    let before = doc.stamp(stamp).unwrap().placement;
    // The same size, new colour: the placement is kept exactly.
    apply(&mut state, vec![changed("badge.png", &filled(20, 10, [0, 200, 0, 255]))]);
    let doc = state.doc_mut().unwrap();
    assert_eq!(doc.undo_label(), Some(FOLDER_UPDATE_LABEL));
    assert_eq!(doc.stamp(stamp).unwrap().placement, before);
    assert_eq!(doc.sticker_picture(f.sticker, None, 1.0).unwrap(), filled(20, 10, [0, 200, 0, 255]));
    assert_eq!(state.doors.folder_counters.updated, 1);
    // A new size: the artboard follows (the file made it), the stamp is refitted, the sticker is the same one.
    apply(&mut state, vec![changed("badge.png", &filled(40, 10, [0, 0, 200, 255]))]);
    let doc = state.doc_mut().unwrap();
    assert_eq!(doc.sticker(f.sticker).unwrap().size(), [40, 10]);
    assert_eq!(doc.sticker_users(f.sticker), vec![stamp]);
    assert_eq!(state.doors.watch.as_ref().unwrap().files["badge.png"].pixels, [40, 10]);
    // Undo: the old picture; the folder's record is not undone (nothing is applied again by itself).
    let doc = state.doc_mut().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.sticker(f.sticker).unwrap().size(), [20, 10]);
    // The same wide file again ("Read again" after the undo): the artboard follows the picture the layer shows now.
    apply(&mut state, vec![changed("badge.png", &filled(40, 10, [0, 0, 200, 255]))]);
    let doc = state.doc_mut().unwrap();
    assert_eq!(doc.sticker(f.sticker).unwrap().size(), [40, 10], "the artboard follows after an undo too");
    assert_eq!(doc.sticker_picture(f.sticker, None, 1.0).unwrap(), filled(40, 10, [0, 0, 200, 255]));
    // Two at once: one entry.
    let n = state.doc().unwrap().history.undo_len();
    apply(&mut state, vec![changed("one.png", &filled(4, 4, [1; 4])), changed("two.png", &filled(4, 4, [2; 4]))]);
    assert_eq!(state.doc().unwrap().history.undo_len(), n + 1);
    assert_eq!(state.doc().unwrap().undo_label(), Some(FOLDER_BATCH_LABEL));
    assert_eq!(state.doors.watch.as_ref().unwrap().files.len(), 3);
}

/// check-S2: the folder's record is saved with the project outside the history, so a changed record keeps the project
/// modified after an undo back to the saved state (a close without saving would lose it and a reopen would apply the
/// undone file again by itself); a picture that changes nothing (the record stays) leaves a saved project unmodified.
#[test]
fn check_s2_a_changed_folder_record_keeps_the_project_modified_after_an_undo() {
    let dir = tmp("record_modified");
    let mut state = watched_state(&dir);
    let save = |state: &mut AppState| {
        let doc = state.doc_mut().unwrap();
        let h = doc.history_state();
        doc.mark_saved(h);
        doc.mark_links_saved(doc.links_revision());
        assert!(!doc.is_modified());
    };
    save(&mut state);
    apply(&mut state, vec![changed("badge.png", &filled(8, 8, [200, 0, 0, 255]))]);
    save(&mut state);
    apply(&mut state, vec![changed("badge.png", &filled(8, 8, [0, 200, 0, 255]))]);
    assert_eq!(state.doc().unwrap().undo_label(), Some(FOLDER_UPDATE_LABEL));
    state.doc_mut().unwrap().undo().unwrap();
    assert!(state.doc().unwrap().is_modified(), "the record changed since the save: still modified after the undo");
    state.doc_mut().unwrap().redo().unwrap();
    save(&mut state);
    // The same picture again ("Read again" of an unchanged folder): no entry, the record stays, not modified.
    let undo = state.doc().unwrap().history.undo_len();
    apply(&mut state, vec![changed("badge.png", &filled(8, 8, [0, 200, 0, 255]))]);
    assert_eq!(state.doors.folder_counters.same, 1);
    let doc = state.doc().unwrap();
    assert_eq!(doc.history.undo_len(), undo);
    assert!(!doc.is_modified(), "an unchanged record does not mark the project modified");
}

/// A file whose sticker was deleted becomes a new sticker; a locked user refuses the update with a toast (the record
/// keeps the old content, so "Read again" can apply it later); a picture that does not decode is refused.
#[test]
fn deleted_and_locked_stickers_and_bad_files() {
    let dir = tmp("apply_edge");
    let mut state = watched_state(&dir);
    apply(&mut state, vec![changed("logo.png", &filled(8, 8, [9, 9, 9, 255]))]);
    let first = state.doors.watch.as_ref().unwrap().files["logo.png"].sticker;
    state.doc_mut().unwrap().remove_sticker(first).unwrap();
    apply(&mut state, vec![changed("logo.png", &filled(8, 8, [90, 9, 9, 255]))]);
    let second = state.doors.watch.as_ref().unwrap().files["logo.png"].clone();
    assert_ne!(second.sticker, first);
    assert!(state.doc().unwrap().sticker(second.sticker).is_some());
    // Locked: a stamp showing it on a locked layer.
    let doc = state.doc_mut().unwrap();
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.0, 0.5), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::splat(0.3), 0.6);
    let stamp = doc.add_stamp("logo", wte_core::StampSource::Sticker { sticker: second.sticker }, wte_core::Placement::Projected(p), TextureSetId(0)).unwrap();
    let (_, layer) = doc.stamp_layers(stamp)[0];
    doc.set_layer_locked(layer, true).unwrap();
    let warnings = state.toasts.visible_texts().len();
    apply(&mut state, vec![changed("logo.png", &filled(8, 8, [0, 0, 255, 255]))]);
    assert_eq!(state.doors.watch.as_ref().unwrap().files["logo.png"].crc, second.crc, "the record keeps the old content");
    assert!(state.toasts.visible_texts().len() > warnings);
    assert_eq!(state.doors.folder_counters.refused, 1);
    // Bad bytes.
    apply(&mut state, vec![FolderEvent::Changed { name: "bad.png".into(), bytes: Arc::from(&b"nope"[..]), crc: 1 }]);
    assert!(!state.doors.watch.as_ref().unwrap().files.contains_key("bad.png"));
    assert_eq!(state.doors.folder_counters.refused, 2);
}

/// Choosing a folder: not a folder, the links folder refused; a new one replaces the old record; the same one keeps
/// it. Stop watching keeps the stickers. Both set the modified flag without a history entry.
#[test]
fn choosing_and_stopping_a_folder() {
    let dir = tmp("choose");
    let mut state = saved_state(&dir);
    run(&mut state, DoorAction::WatchFolder(Some(dir.join("nothing-here"))));
    assert!(state.doors.watch.is_none());
    let links = wte_core::link::links_folder(&dir.join("proj.wtep"));
    std::fs::create_dir_all(&links).unwrap();
    run(&mut state, DoorAction::WatchFolder(Some(links)));
    assert!(state.doors.watch.is_none(), "the links folder is refused");
    let assets = dir.join("doc-assets");
    std::fs::create_dir_all(&assets).unwrap();
    let undo = state.doc().unwrap().history.undo_len();
    run(&mut state, DoorAction::WatchFolder(Some(assets.clone())));
    assert_eq!(state.doors.watch.as_ref().unwrap().folder, assets);
    assert!(state.doc().unwrap().is_modified() && state.doc().unwrap().history.undo_len() == undo);
    apply(&mut state, vec![changed("x.png", &filled(4, 4, [3; 4]))]);
    run(&mut state, DoorAction::WatchFolder(Some(assets.clone())));
    assert_eq!(state.doors.watch.as_ref().unwrap().files.len(), 1, "the same folder keeps its record");
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    run(&mut state, DoorAction::WatchFolder(Some(other)));
    assert!(state.doors.watch.as_ref().unwrap().files.is_empty(), "another folder starts afresh");
    run(&mut state, DoorAction::StopWatching);
    assert!(state.doors.watch.is_none());
    assert_eq!(crate::sticker::tray::tray_stickers(&state).len(), 1, "its sticker stays in the tray");
}

/// The whole path through `pump`: the worker started for the folder, a saved file applied, held while busy, the
/// record saved and restored (a reopened project reads nothing again; a changed file updates its sticker).
#[test]
fn the_folder_through_pump_save_and_restore() {
    let dir = tmp("pump");
    let assets = dir.join("doc-assets");
    std::fs::create_dir_all(&assets).unwrap();
    let mut state = saved_state(&dir);
    state.doors.folder_times = fast_times();
    let ctx = egui::Context::default();
    run(&mut state, DoorAction::WatchFolder(Some(assets.clone())));
    std::fs::write(assets.join("star.png"), png_of(&filled(6, 6, [250, 200, 0, 255]))).unwrap();
    let until = |state: &mut AppState, busy: bool, done: &dyn Fn(&AppState) -> bool| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            pump_folder(state, &ctx, busy);
            if done(state) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    };
    // Held while busy.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        pump_folder(&mut state, &ctx, true);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(state.doors.folder_counters.added, 0, "held while busy");
    assert!(state.doors.watch.as_ref().unwrap().held.len() == 1);
    assert!(until(&mut state, false, &|s| s.doors.folder_counters.added == 1));
    let (copies, watch) = saved(&state.doors, state.doc().unwrap());
    assert!(copies.is_empty());
    let watch = watch.unwrap();
    assert_eq!(watch.files.len(), 1);
    assert_eq!(watch.files[0].name, "star.png");
    // "Reopen": the same document, the record restored; nothing is applied again.
    state.doors.reset();
    restore(&mut state, &[], Some(&watch));
    let quiet = Instant::now();
    while quiet.elapsed() < Duration::from_millis(300) {
        pump_folder(&mut state, &ctx, false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(state.doors.folder_counters.updated, 0);
    let n = state.doc().unwrap().history.undo_len();
    std::fs::write(assets.join("star.png"), png_of(&filled(6, 6, [0, 200, 250, 255]))).unwrap();
    assert!(until(&mut state, false, &|s| s.doors.folder_counters.updated == 1));
    assert_eq!(state.doc().unwrap().history.undo_len(), n + 1);
    let id = state.doors.watch.as_ref().unwrap().files["star.png"].sticker;
    assert_eq!(state.doc_mut().unwrap().sticker_picture(id, None, 1.0).unwrap(), filled(6, 6, [0, 200, 250, 255]));
    // Read again: the same pictures make no entry (the content did not change).
    let n = state.doc().unwrap().history.undo_len();
    run(&mut state, DoorAction::RescanFolder);
    assert!(until(&mut state, false, &|s| s.doors.folder_counters.same == 1));
    assert_eq!(state.doc().unwrap().history.undo_len(), n, "an unchanged picture is no edit");
    assert_eq!(state.doors.folder_counters.updated, 1);
}

/// Timing (ignored; release, serially): the UI thread's part of a copy (`live::pump` starting the job) and the whole
/// write, for a 2048 x 2048 sticker with a styled layer and a paint layer of the Sphere's texture (1024 x 1024), the first
/// write and one after a change.
#[test]
#[ignore = "perf: run with --release --ignored --test-threads=1"]
fn perf_s2_live_copy_jobs() {
    let dir = tmp("perf");
    let mut state = saved_state(&dir);
    let ctx = egui::Context::default();
    let mut p = PixelBuf::new(2048, 2048);
    for y in 0..2048 {
        for x in 0..2048 {
            p.set(x, y, [(x / 8) as u8, (y / 8) as u8, ((x ^ y) & 255) as u8, if (x / 64 + y / 64) % 3 == 0 { 0 } else { 255 }]);
        }
    }
    let id = tray_sticker(&mut state, "Big", p);
    state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |d| d.layers[0].styles = wte_core::StylePreset::ClassicSticker.styles([2048, 2048]), "Style", Commit::Final).unwrap();
    let doc = state.doc_mut().unwrap();
    let [w, h] = [doc.sets[0].width, doc.sets[0].height];
    let layer = doc.add_paint_layer_with_pixels(TextureSetId(0), "Big layer", PixelBuf::filled(w, h, [10, 120, 200, 255]), "New Layer").unwrap();
    for target in [LiveTarget::Sticker(id), LiveTarget::Layer(layer)] {
        run(&mut state, DoorAction::SetLiveCopy(target, true));
        for pass in ["first", "after a change"] {
            if pass != "first" {
                match target {
                    LiveTarget::Sticker(_) => {
                        state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |d| d.layers[0].opacity = 0.7, "Op", Commit::Final).unwrap();
                    }
                    LiveTarget::Layer(l) => {
                        state.doc_mut().unwrap().replace_layer_pixels(l, PixelBuf::filled(w, h, [200, 20, 20, 255]), "Paint").unwrap();
                    }
                }
                state.doors.live.delay = Duration::ZERO;
            }
            let before = state.doors.live.counters.writes;
            let t0 = Instant::now();
            live::pump(&mut state, &ctx, false);
            let ui = t0.elapsed();
            assert!(pump_until(&mut state, &ctx, Duration::from_secs(60), |s| s.doors.live.counters.writes == before + 1));
            let size = std::fs::metadata(copy_path(&state, target)).unwrap().len();
            eprintln!("perf_s2 {target:?} {pass}: UI thread {ui:?}, whole write {:?}, {size} bytes", t0.elapsed());
        }
        state.doors.live.delay = Duration::from_millis(60);
    }
}
