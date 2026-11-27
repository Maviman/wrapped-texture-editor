//! Tests of the editor link in the app (stickers P9a; STICKER_PLAN 7 "P9", 5.3-5.8, 13.1). Files live under
//! `target\wte-test-tmp\link\` (never `%TEMP%`, the real `%LOCALAPPDATA%` or the registry); every launcher is the stub;
//! the registry is an injected map.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use wte_core::link::{FileStamp, LinkFile, LinkState, links_folder};
use wte_core::psd::{StickerPsd, write_sticker_to_vec};
use wte_core::{Commit, IRect, StampId, StampSource, StickerId, StickerRef};

use super::editor::ExternalEditor;
use super::launch::{self, Hive, RegistryReader, StubLauncher};
use super::watch::{LinkEvent, LinkWatcher, OwnWrites, WatchEntry, WatchTimes};
use super::*;
use crate::state::{AppState, ViewMode};

/// A fresh folder under `target\wte-test-tmp\link\`, removed when the guard is dropped (ARCHITECTURE 4.8).
pub(super) struct TmpDir(PathBuf);

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

pub(super) fn tmp(name: &str) -> TmpDir {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wte-test-tmp/link").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TmpDir(dir)
}

/// An image stamp on the sphere in a project "saved" at `<dir>\proj.wtep` (the path only: the links folder is beside
/// it), the stub launcher with an existing fake `photocraft.exe`.
pub(super) fn linked_state(name: &str) -> (AppState, StampId, TmpDir) {
    let dir = tmp(name);
    let (mut state, stamp, _) = crate::sticker::tests::image_stamp_state(ViewMode::Split);
    state.session.as_mut().unwrap().path = Some(dir.join("proj.wtep"));
    let exe = dir.join("pc").join("photocraft.exe");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"").unwrap();
    state.prefs.photocraft_exe = Some(exe);
    (state, stamp, dir)
}

/// Runs the queued link and sticker actions (what dispatch does next frame); other actions are returned.
pub(super) fn run_queued(state: &mut AppState) -> Vec<AppAction> {
    let mut other = Vec::new();
    for _ in 0..8 {
        let actions = std::mem::take(&mut state.actions);
        if actions.is_empty() {
            break;
        }
        for a in actions {
            match a {
                AppAction::Link(l) => flows::run_in(state, l),
                AppAction::Sticker(s) => crate::sticker::run_in(state, s),
                AppAction::ShowDialog(crate::state::Dialog::None) => state.dialog = crate::state::Dialog::None,
                other_action => other.push(other_action),
            }
        }
    }
    other
}

pub(super) fn edit(state: &mut AppState, stamp: StampId) {
    flows::run_in(state, LinkAction::EditIn(EditRequest::new(LinkTarget::Stamp(stamp))));
    run_queued(state);
}

pub(super) fn question_kind(state: &AppState) -> Option<QuestionKind> {
    open_question(state).map(|q| q.kind)
}

pub(super) fn sticker_id(state: &AppState, stamp: StampId) -> StickerId {
    sticker_of_stamp(state, stamp).expect("a sticker stamp")
}

/// Reads sticker `id`'s link file as the watcher would and hands the event to the app.
pub(super) fn deliver(state: &mut AppState, id: StickerId, tools_busy: bool) {
    let path = link_path(state, id).unwrap();
    let (stamp, bytes) = FileStamp::read(&path).unwrap();
    let psd = wte_core::psd::read(&bytes, &wte_core::psd::PsdLimits::default()).unwrap();
    on_event(state, LinkEvent::Changed { id: id.into(), stamp, psd: Box::new(psd), from_alias: false }, tools_busy);
}

pub(super) fn badge_text(state: &mut AppState, id: StickerId) -> String {
    badge(state, id).map(|(_, t)| t).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------------------------------
// Launcher and discovery
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn command_for_gives_the_file_as_one_argument_and_spawns_nothing() {
    let exe = PathBuf::from(r"C:\Program Files\PhotoCraft\photocraft.exe");
    for file in [
        r"G:\My Projects\robot arm_links\Logo_k12.psd",
        r"G:\Проекты\наклейка_k3.psd",
        r"\\server\share\x y\Logo_k1.psd",
        r"C:\a & b\100% done_k7.psd",
    ] {
        let file = PathBuf::from(file);
        let c = launch::command_for(&ExternalEditor::PhotoCraft, &exe, &file);
        assert_eq!(c.program, exe.as_os_str());
        assert_eq!(c.args, vec![file.as_os_str().to_owned()], "{}", file.display());
        assert_eq!(c.cwd.as_deref(), Some(Path::new(r"C:\Program Files\PhotoCraft")));
    }
    let c = launch::convert_command(Path::new(r"C:\pc\photocraft-cli.exe"), Path::new(r"C:\a b\x.psd"), Path::new(r"C:\a b\x.png"));
    assert_eq!(c.args, vec!["convert".into(), std::ffi::OsString::from(r"C:\a b\x.psd"), std::ffi::OsString::from(r"C:\a b\x.png")]);
}

#[test]
fn self_tests_and_test_builds_get_only_the_stub() {
    assert!(launch::launcher_for(true).is_stub());
    assert!(launch::launcher_for(false).is_stub(), "a test build has no real launcher");
    assert!(LinkUi::new(true).launcher.is_stub() && LinkUi::new(false).launcher.is_stub());
    assert!(AppState::new(crate::state::Prefs::default(), true).link_ui.launcher.is_stub());
    // The registry of a self-test (and of a test build) is empty.
    assert_eq!(
        launch::registry_for(false).default_value(Hive::CurrentUser, launch::RegView::Bits64, r"Software\Classes\Applications\photocraft.exe\shell\open\command"),
        None
    );
}

/// An injected registry: values of the 64-bit view by (hive, key); `MapRegistry32` adds the 32-bit view.
pub(super) struct MapRegistry(pub(super) HashMap<(Hive, String), String>);

impl RegistryReader for MapRegistry {
    fn default_value(&self, hive: Hive, view: launch::RegView, subkey: &str) -> Option<String> {
        if view != launch::RegView::Bits64 {
            return None;
        }
        self.0.get(&(hive, subkey.to_owned())).cloned()
    }
}

#[test]
fn discovery_reads_the_two_keys_hkcu_first_and_needs_an_existing_exe() {
    const K1: &str = r"Software\Classes\Photocraft.PhotoshopDocument\shell\open\command";
    const K2: &str = r"Software\Classes\Applications\photocraft.exe\shell\open\command";
    let exists_set = ["C:\\PC\\photocraft.exe", "D:\\Other PC\\photocraft.exe", "E:\\pc3\\photocraft.exe"];
    let exists = |p: &Path| exists_set.iter().any(|e| Path::new(e) == p);
    let reg = |entries: &[(Hive, &str, &str)]| {
        MapRegistry(entries.iter().map(|(h, k, v)| ((*h, (*k).to_owned()), (*v).to_owned())).collect())
    };
    let pc = ExternalEditor::PhotoCraft;
    // Key 1 (quoted, with "%1").
    let r = reg(&[(Hive::LocalMachine, K1, r#""C:\PC\photocraft.exe" "%1""#)]);
    assert_eq!(launch::discover(&pc, &r, &exists), Some(PathBuf::from(r"C:\PC\photocraft.exe")));
    // Key 2 only, unquoted.
    let r = reg(&[(Hive::CurrentUser, K2, r"E:\pc3\photocraft.exe %1")]);
    assert_eq!(launch::discover(&pc, &r, &exists), Some(PathBuf::from(r"E:\pc3\photocraft.exe")));
    // HKCU before HKLM.
    let r = reg(&[(Hive::LocalMachine, K1, r#""C:\PC\photocraft.exe" "%1""#), (Hive::CurrentUser, K1, r#""D:\Other PC\photocraft.exe" "%1""#)]);
    assert_eq!(launch::discover(&pc, &r, &exists), Some(PathBuf::from(r"D:\Other PC\photocraft.exe")));
    // A path that does not exist is skipped; neither key: nothing.
    let r = reg(&[(Hive::CurrentUser, K1, r#""C:\gone\photocraft.exe" "%1""#), (Hive::LocalMachine, K2, r#""C:\PC\photocraft.exe" "%1""#)]);
    assert_eq!(launch::discover(&pc, &r, &exists), Some(PathBuf::from(r"C:\PC\photocraft.exe")));
    assert_eq!(launch::discover(&pc, &reg(&[]), &exists), None);
    // These PhotoCraft keys name no Photoshop (P9b discovers Photoshop from its own keys: `link::editor_tests`).
    assert_eq!(launch::discover(&ExternalEditor::Photoshop, &r, &exists), None);
    assert_eq!(launch::exe_of_command(r#"  "C:\x y\a.exe"  "%1" "#), Some(PathBuf::from(r"C:\x y\a.exe")));
    assert_eq!(launch::exe_of_command(r"C:\x\A.EXE /open %1"), Some(PathBuf::from(r"C:\x\A.EXE")));
    assert_eq!(launch::exe_of_command(r"C:\plain\path"), Some(PathBuf::from(r"C:\plain\path")));
    assert_eq!(launch::exe_of_command(r#""""#), None);
}

// ---------------------------------------------------------------------------------------------------------------------
// The watcher
// ---------------------------------------------------------------------------------------------------------------------

fn small_psd(rgba: [u8; 4]) -> Vec<u8> {
    let psd = StickerPsd {
        size: [16, 12],
        layers: vec![testkit::rect_layer("Layer 1", 1, IRect::from_xywh(2, 2, 8, 6), rgba)],
        id_seed: 1,
    };
    write_sticker_to_vec(&psd).unwrap()
}

const TIMES: WatchTimes =
    WatchTimes { poll: Duration::from_millis(30), settle: Duration::from_millis(20), give_up: Duration::from_millis(400) };

fn watcher() -> (LinkWatcher, Arc<AtomicU64>, OwnWrites) {
    let own = OwnWrites::default();
    let repaints = Arc::new(AtomicU64::new(0));
    (LinkWatcher::start(own.clone(), Arc::new(|| {}), Arc::clone(&repaints), TIMES), repaints, own)
}

/// The events of the next `ms` milliseconds.
fn collect(w: &LinkWatcher, ms: u64) -> Vec<LinkEvent> {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut out = Vec::new();
    while Instant::now() < end {
        out.extend(w.drain());
        std::thread::sleep(Duration::from_millis(5));
    }
    out
}

/// Waits (at most 5 s) for the first event.
fn first_event(w: &LinkWatcher) -> LinkEvent {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if let Some(e) = w.drain().into_iter().next() {
            return e;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no event within 5 s");
}

fn entry(path: &Path, bytes: &[u8]) -> WatchEntry {
    let modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    WatchEntry { id: StickerId(7).into(), path: path.to_path_buf(), alias: None, synced: FileStamp::of_bytes(bytes, modified) }
}

#[test]
fn the_watcher_reports_a_change_once_and_is_quiet_without_one() {
    let dir = tmp("watch_once");
    let file = dir.join("a_k7.psd");
    let first = small_psd([10, 20, 30, 255]);
    std::fs::write(&file, &first).unwrap();
    let (w, repaints, _) = watcher();
    w.set_list(vec![entry(&file, &first)]);
    // The idle check: no change over many polls, no event, no repaint (review A14).
    assert!(collect(&w, 250).is_empty());
    assert_eq!(repaints.load(Ordering::SeqCst), 0, "repaints without a change");
    // A half-written file waits (no event, no repaint) ...
    let second = small_psd([200, 20, 30, 255]);
    std::fs::write(&file, &second[..second.len() / 2]).unwrap();
    assert!(collect(&w, 150).is_empty(), "a half-written file was reported");
    assert_eq!(repaints.load(Ordering::SeqCst), 0);
    // ... and the finished file is reported exactly once.
    testkit::save_bytes(&file, &second).unwrap();
    match first_event(&w) {
        LinkEvent::Changed { id, stamp, psd, from_alias } => {
            assert_eq!((id, from_alias), (StickerId(7).into(), false));
            assert_eq!(stamp.crc32, super::watch::crc(&second));
            assert_eq!(psd.layers.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    assert!(collect(&w, 200).is_empty(), "reported twice");
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn the_watcher_gives_up_on_a_file_that_stays_half_written_and_reports_a_deleted_file_once() {
    let dir = tmp("watch_unreadable");
    let file = dir.join("a_k7.psd");
    let first = small_psd([10, 20, 30, 255]);
    std::fs::write(&file, &first).unwrap();
    let (w, _, _) = watcher();
    w.set_list(vec![entry(&file, &first)]);
    std::thread::sleep(Duration::from_millis(80));
    let second = small_psd([200, 20, 30, 255]);
    std::fs::write(&file, &second[..second.len() - 7]).unwrap();
    assert!(matches!(first_event(&w), LinkEvent::Unreadable { id: LinkId::Sticker(StickerId(7)) }));
    assert!(collect(&w, 200).is_empty(), "unreadable twice");
    std::fs::remove_file(&file).unwrap();
    assert!(matches!(first_event(&w), LinkEvent::Missing { id: LinkId::Sticker(StickerId(7)) }));
    assert!(collect(&w, 150).is_empty(), "missing twice");
    // Complete bytes that are not a PSD: refused once.
    std::fs::write(&file, vec![b'x'; 64]).unwrap();
    assert!(matches!(first_event(&w), LinkEvent::Refused { .. }));
}

#[test]
fn wtes_own_write_is_not_reported_even_with_the_old_list() {
    let dir = tmp("watch_own");
    let file = dir.join("a_k7.psd");
    let first = small_psd([10, 20, 30, 255]);
    std::fs::write(&file, &first).unwrap();
    let (w, repaints, own) = watcher();
    w.set_list(vec![entry(&file, &first)]);
    std::thread::sleep(Duration::from_millis(80));
    // WTE writes new content; the list still has the old synced stamp.
    let mine = small_psd([1, 2, 3, 255]);
    let mut io = super::watch::OwnWritesIo(own.clone());
    wte_core::link::write_link_file(&file, &mine, &mut io).unwrap();
    assert!(own.len() <= 1, "registered once (the watcher may have used it up already)");
    assert!(collect(&w, 250).is_empty(), "WTE's own write was reported");
    assert_eq!(repaints.load(Ordering::SeqCst), 0);
}

/// Check-P9b: the first write of a new link (or a restored file) is never judged: the watcher's first look finds the
/// synced stamp and stops there. Its registration must still be used up, or an editor's later save of exactly those
/// bytes (after another save moved the synced content on) is taken for WTE's write and never reported.
#[test]
fn a_new_links_first_write_is_used_up_at_the_first_look() {
    let dir = tmp("watch_own_first_look");
    let file = dir.join("a_k7.psd");
    let first = small_psd([10, 20, 30, 255]);
    let (w, _, own) = watcher();
    // WTE makes the link: the file written through the registration, then the new entry with that stamp as synced.
    let mut io = super::watch::OwnWritesIo(own.clone());
    let stamp = wte_core::link::write_link_file(&file, &first, &mut io).unwrap();
    assert_eq!(own.len(), 1);
    let mut e = WatchEntry { id: StickerId(7).into(), path: file.clone(), alias: None, synced: stamp };
    w.set_list(vec![e.clone()]);
    assert!(collect(&w, 250).is_empty(), "the link's own first file was reported");
    assert_eq!(own.len(), 0, "the first write's registration stayed");
    // The editor saves a change (applied: the synced content moves on), then exactly the first file's bytes again.
    let other = small_psd([90, 90, 90, 255]);
    testkit::save_bytes(&file, &other).unwrap();
    let LinkEvent::Changed { stamp, .. } = first_event(&w) else { panic!("not a change") };
    assert_eq!(stamp.crc32, super::watch::crc(&other));
    e.synced = stamp;
    w.set_list(vec![e]);
    std::thread::sleep(Duration::from_millis(80));
    testkit::save_bytes(&file, &first).unwrap();
    assert!(matches!(first_event(&w), LinkEvent::Changed { stamp, .. } if stamp.crc32 == super::watch::crc(&first)));
}

/// Stickers P9b (check-P9a open item 1, met by the Photoshop pass of `sticker_link`): a registration of WTE's own write
/// is used up when the watcher sees it settle, so the editor saving the very same bytes later (another link of the
/// same picture, an undo back to what WTE sent) is reported instead of being taken for WTE's write for the session.
#[test]
fn an_own_write_is_used_up_when_seen_so_the_same_bytes_from_the_editor_are_reported() {
    let dir = tmp("watch_own_once");
    let file = dir.join("a_k7.psd");
    let first = small_psd([10, 20, 30, 255]);
    std::fs::write(&file, &first).unwrap();
    let (w, _, own) = watcher();
    w.set_list(vec![entry(&file, &first)]);
    std::thread::sleep(Duration::from_millis(80));
    let mine = small_psd([1, 2, 3, 255]);
    let mut io = super::watch::OwnWritesIo(own.clone());
    wte_core::link::write_link_file(&file, &mine, &mut io).unwrap();
    assert!(collect(&w, 250).is_empty(), "WTE's own write was reported");
    assert_eq!(own.len(), 0, "the registration was not used up");
    // The editor saves something else, then exactly WTE's earlier bytes: both are reported.
    let other = small_psd([90, 90, 90, 255]);
    testkit::save_bytes(&file, &other).unwrap();
    assert!(matches!(first_event(&w), LinkEvent::Changed { stamp, .. } if stamp.crc32 == super::watch::crc(&other)));
    testkit::save_bytes(&file, &mine).unwrap();
    assert!(matches!(first_event(&w), LinkEvent::Changed { stamp, .. } if stamp.crc32 == super::watch::crc(&mine)));
    // Two registrations of one content (two writes) are used up one at a time.
    own.add(7);
    own.add(7);
    assert!(own.take(7) && own.take(7) && !own.take(7));
}

#[test]
fn an_alias_change_is_copied_over_the_file_and_reported_once() {
    let dir = tmp("watch_alias");
    let (old, new) = (dir.join("old_links").join("a_k7.psd"), dir.join("new_links").join("a_k7.psd"));
    let first = small_psd([10, 20, 30, 255]);
    for p in [&old, &new] {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, &first).unwrap();
    }
    let (w, _, _) = watcher();
    w.set_list(vec![WatchEntry { alias: Some(old.clone()), ..entry(&new, &first) }]);
    std::thread::sleep(Duration::from_millis(80));
    let saved = small_psd([0, 200, 0, 255]);
    testkit::save_bytes(&old, &saved).unwrap();
    match first_event(&w) {
        LinkEvent::Changed { from_alias, stamp, .. } => {
            assert!(from_alias);
            assert_eq!(stamp.crc32, super::watch::crc(&saved));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(&new).unwrap(), saved, "the alias's content is now the file's");
    assert!(collect(&w, 200).is_empty(), "the copy was reported again");
}

#[test]
fn the_watcher_thread_stops_with_the_last_link() {
    let (w, _, _) = watcher();
    let alive = w.alive();
    assert!(alive.load(Ordering::SeqCst));
    drop(w);
    let end = Instant::now() + Duration::from_secs(2);
    while alive.load(Ordering::SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!alive.load(Ordering::SeqCst), "the thread still runs");
}

#[test]
fn pump_starts_the_watcher_for_the_first_link_and_stops_it_with_the_last() {
    let (mut state, stamp, _dir) = linked_state("pump_lifecycle");
    let ctx = egui::Context::default();
    pump(&mut state, &ctx, false);
    assert!(state.link_ui.watcher.is_none(), "no link, no watcher");
    edit(&mut state, stamp);
    pump(&mut state, &ctx, false);
    let alive = state.link_ui.watcher.as_ref().expect("a watcher for the link").alive();
    let id = sticker_id(&state, stamp);
    flows::run_in(&mut state, LinkAction::Unlink(id.into()));
    pump(&mut state, &ctx, false);
    assert!(state.link_ui.watcher.is_none());
    let end = Instant::now() + Duration::from_secs(3);
    while alive.load(Ordering::SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!alive.load(Ordering::SeqCst));
}

// ---------------------------------------------------------------------------------------------------------------------
// Edit in
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn an_unsaved_project_asks_to_save_first_and_writes_nothing() {
    let (mut state, stamp, _) = crate::sticker::tests::image_stamp_state(ViewMode::Split);
    let undo = state.doc().unwrap().history.undo_len();
    edit(&mut state, stamp);
    assert_eq!(question_kind(&state), Some(QuestionKind::SaveFirst));
    assert_eq!(open_question(&state).unwrap().lines, [wte_core::link::texts::SAVE_FIRST]);
    assert!(state.link_ui.launcher.calls().is_empty());
    assert_eq!(state.doc().unwrap().history.undo_len(), undo, "nothing changed");
    assert!(matches!(state.doc().unwrap().stamp(stamp).unwrap().source, StampSource::Image { .. }));
    // "Save..." saves, and the edit continues after the save. Dispatch runs the queue in order: the Save (whose
    // `start_save` calls `before_save` when the job starts), then `DropUnstartedEdit` (check-P9a).
    press(&mut state, "Save...").unwrap();
    save_then_edit_queues_save_and_drop(&mut state);
    flows::before_save(&mut state, Path::new(r"X:\never\proj.wtep"));
    flows::run_in(&mut state, LinkAction::DropUnstartedEdit);
    assert!(state.link_ui.after_save.is_some(), "the started save keeps the edit");
    flows::after_save(&mut state, 0, &[]);
    assert!(matches!(state.actions.as_slice(), [AppAction::Link(LinkAction::EditIn(_))]));
}

/// Runs the queued `SaveThenEdit` and checks that it queued the Save and, right behind it, `DropUnstartedEdit`.
fn save_then_edit_queues_save_and_drop(state: &mut AppState) {
    let queued = std::mem::take(&mut state.actions);
    assert!(matches!(queued.as_slice(), [AppAction::Link(LinkAction::SaveThenEdit(_))]), "{queued:?}");
    for a in queued {
        if let AppAction::Link(l) = a {
            flows::run_in(state, l);
        }
    }
    let queued = std::mem::take(&mut state.actions);
    assert!(matches!(queued.as_slice(), [AppAction::Save, AppAction::Link(LinkAction::DropUnstartedEdit)]), "{queued:?}");
}

/// check-P9a: "Save..." whose save never starts (the Save As dialog cancelled, a job running) must not leave the edit
/// waiting: a later, unrelated save would otherwise open the sticker in the editor out of the blue.
#[test]
fn check_p9a_a_save_that_did_not_start_drops_the_waiting_edit() {
    let (mut state, stamp, _) = crate::sticker::tests::image_stamp_state(ViewMode::Split);
    edit(&mut state, stamp);
    assert_eq!(question_kind(&state), Some(QuestionKind::SaveFirst));
    press(&mut state, "Save...").unwrap();
    save_then_edit_queues_save_and_drop(&mut state);
    // The Save did nothing (no `before_save`); the drop runs next.
    flows::run_in(&mut state, LinkAction::DropUnstartedEdit);
    assert!(state.link_ui.save_then_edit.is_none() && state.link_ui.after_save.is_none());
    // Later the user saves for another reason: nothing is opened in the editor.
    flows::before_save(&mut state, Path::new(r"X:\never\proj.wtep"));
    flows::after_save(&mut state, 0, &[]);
    assert!(state.actions.is_empty(), "a later save continued the dropped edit: {:?}", state.actions);
    assert!(state.link_ui.launcher.calls().is_empty());
}

/// check-P9a: a link made while a save runs is not in that save, so the close prompt keeps naming it.
#[test]
fn check_p9a_a_link_made_during_a_save_stays_in_the_close_prompt() {
    let (mut state, stamp, dir) = linked_state("made_during_save");
    let project = dir.join("proj.wtep");
    flows::before_save(&mut state, &project);
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    assert!(state.link_ui.made_since_save.contains(&id.into()));
    flows::after_save(&mut state, 0, &[]);
    assert!(state.link_ui.made_since_save.contains(&id.into()), "the running save did not hold the link");
    assert!(unsaved_line(&state).is_some());
    // The next save holds it.
    flows::before_save(&mut state, &project);
    flows::after_save(&mut state, 0, &[]);
    assert!(unsaved_line(&state).is_none());
}

#[test]
fn edit_in_promotes_writes_the_file_and_starts_the_stub() {
    let (mut state, stamp, dir) = linked_state("edit_in");
    let undo = state.doc().unwrap().history.undo_len();
    edit(&mut state, stamp);
    assert_eq!(state.dialog_name(), "None");
    let id = sticker_id(&state, stamp);
    assert_eq!(state.doc().unwrap().history.undo_len(), undo + 1, "the promotion is one entry");
    let path = link_path(&state, id).unwrap();
    assert_eq!(path.parent().unwrap(), links_folder(&dir.join("proj.wtep")));
    assert_eq!(path.file_name().unwrap().to_string_lossy(), format!("Logo_k{}.psd", id.0));
    let read = testkit::read(&path).unwrap();
    assert_eq!(read.layers.len(), 1);
    let calls = state.link_ui.launcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].editor.as_str(), calls[0].file.as_path()), ("photocraft", path.as_path()));
    assert_eq!(calls[0].exe, state.prefs.photocraft_exe.clone().unwrap());
    let s = &state.link_ui.sessions[&id.into()];
    assert!(s.opened && !s.stale_in_editor);
    assert_eq!(state.doc().unwrap().link(id).unwrap().editor, "photocraft");
    assert!(state.link_ui.own_writes.contains(super::watch::crc(&std::fs::read(&path).unwrap())));
    assert_eq!(badge_text(&mut state, id), "linked");
    assert!(state.link_ui.made_since_save.contains(&id.into()));
    let line = unsaved_line(&state).unwrap();
    assert!(line.starts_with("Stickers linked to PhotoCraft since the last save: Logo.") && line.contains("proj_links"), "{line}");
    assert!(state.toasts.visible_texts().iter().any(|t| t.contains("styles are not in the file")));
    // The project is modified by the link (the links are saved with it).
    assert!(state.doc().unwrap().is_modified());
}

#[test]
fn a_shared_picture_is_asked_about_before_any_file_exists() {
    let (mut state, stamp, dir) = linked_state("shared");
    let blob = match state.doc().unwrap().stamp(stamp).unwrap().source {
        StampSource::Image { blob } => blob,
        _ => unreachable!(),
    };
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.3, 0.0, 0.4), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::splat(0.2), 0.6);
    let second = state.doc_mut().unwrap().add_stamp("Logo 2", StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    edit(&mut state, stamp);
    assert_eq!(question_kind(&state), Some(QuestionKind::Shared));
    assert_eq!(open_question(&state).unwrap().lines[0], "This picture is used by 2 stickers.");
    assert!(!links_folder(&dir.join("proj.wtep")).exists(), "a file was written before the answer");
    press(&mut state, "Only this one").unwrap();
    run_queued(&mut state);
    let id = sticker_id(&state, stamp);
    assert!(matches!(state.doc().unwrap().stamp(second).unwrap().source, StampSource::Image { .. }), "the other stamp changed");
    assert!(link_path(&state, id).unwrap().is_file());
    assert_eq!(state.link_ui.launcher.calls().len(), 1);
}

#[test]
fn a_text_stamp_is_converted_after_a_question() {
    let (mut state, _, _dir) = linked_state("text");
    let font = state.doc_mut().unwrap().default_font();
    let spec = wte_core::TextSpec::new("Hi", font);
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.3, 0.4), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::splat(0.2), 0.6);
    let text = state.doc_mut().unwrap().add_stamp("Hi", StampSource::Text(spec), wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    edit(&mut state, text);
    assert_eq!(question_kind(&state), Some(QuestionKind::ConvertText));
    press(&mut state, "Convert and edit").unwrap();
    run_queued(&mut state);
    let id = sticker_id(&state, text);
    assert!(link_path(&state, id).unwrap().is_file());
    let layers = &state.doc().unwrap().sticker(id).unwrap().layers;
    assert!(matches!(layers[0].content, wte_core::StickerContent::Text(_)), "the text is a text layer");
}

#[test]
fn a_running_child_asks_already_open_and_an_exited_one_clears_stale() {
    let (mut state, stamp, _dir) = linked_state("guard");
    let running = Arc::new(AtomicBool::new(true));
    state.link_ui.launcher = Box::new(StubLauncher { child_running: Some(Arc::clone(&running)), ..Default::default() });
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    assert_eq!(state.link_ui.launcher.calls().len(), 1);
    edit(&mut state, stamp);
    assert_eq!(question_kind(&state), Some(QuestionKind::AlreadyOpen));
    assert!(open_question(&state).unwrap().lines[0].starts_with("This sticker is already open in PhotoCraft."));
    assert_eq!(state.link_ui.launcher.calls().len(), 1, "nothing started before the answer");
    press(&mut state, "Open anyway").unwrap();
    run_queued(&mut state);
    assert_eq!(state.link_ui.launcher.calls().len(), 2);
    // The editor exited: a new Edit in opens without a question, and clears a stale flag (a fresh window reads the file).
    running.store(false, Ordering::SeqCst);
    state.link_ui.session(id).stale_in_editor = true;
    edit(&mut state, stamp);
    assert_eq!(state.dialog_name(), "None");
    assert_eq!(state.link_ui.launcher.calls().len(), 3);
    assert!(!state.link_ui.sessions[&id.into()].stale_in_editor);
}

// ---------------------------------------------------------------------------------------------------------------------
// Updates, conflicts, sending
// ---------------------------------------------------------------------------------------------------------------------

/// The linked sticker with the editor's save of one more layer on its file.
pub(super) fn editor_adds_a_layer(state: &mut AppState, id: StickerId, name: &str) {
    let path = link_path(state, id).unwrap();
    let psd = testkit::with_new_layer(&path, name, 50, IRect::from_xywh(4, 4, 20, 10)).unwrap();
    testkit::save_as_editor(&path, &psd).unwrap();
}

#[test]
fn a_change_is_applied_once_and_an_undo_does_not_bring_it_back() {
    let (mut state, stamp, _dir) = linked_state("update");
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    let undo = state.doc().unwrap().history.undo_len();
    editor_adds_a_layer(&mut state, id, "red");
    deliver(&mut state, id, false);
    assert_eq!(state.doc().unwrap().undo_label(), Some("Update Linked Sticker"));
    assert_eq!(state.doc().unwrap().history.undo_len(), undo + 1);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 2);
    assert_eq!(badge_text(&mut state, id), "linked");
    assert!(state.toasts.visible_texts().contains(&"Updated from PhotoCraft"));
    // The same file again: nothing.
    deliver(&mut state, id, false);
    assert_eq!(state.doc().unwrap().history.undo_len(), undo + 1);
    // Undo: "changed in WTE", and the file (unchanged) is not applied again by itself.
    state.doc_mut().unwrap().undo().unwrap();
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 1);
    assert_eq!(badge_text(&mut state, id), "changed in WTE");
    deliver(&mut state, id, false);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 1, "the undone update came back by itself");
    // "Update now" applies it again.
    flows::run_in(&mut state, LinkAction::UpdateNow(id.into()));
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 2);
    assert_eq!(badge_text(&mut state, id), "linked");
}

#[test]
fn an_update_waits_while_the_sticker_view_is_busy() {
    let (mut state, stamp, _dir) = linked_state("held");
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    crate::sticker::enter_in(&mut state, Some(stamp));
    let first_layer = state.doc().unwrap().sticker(id).unwrap().layers[0].id;
    crate::sticker::run_in(&mut state, crate::sticker::StickerAction::SelectRect([0.0, 0.0, 10.0, 10.0]));
    assert!(crate::sticker::edit_of(&state).unwrap().selection.is_some());
    // The editor's file drops the active layer and adds one.
    let path = link_path(&state, id).unwrap();
    let mut psd = testkit::psd_of(&testkit::read(&path).unwrap());
    psd.layers.clear();
    psd.layers.push(testkit::rect_layer("pc layer", 9, IRect::from_xywh(3, 3, 12, 12), [0, 0, 250, 255]));
    testkit::save_as_editor(&path, &psd).unwrap();
    // A drag of the canvas is open: the update waits.
    state.sticker_ui.drag = Some(crate::sticker::CanvasDrag::Pan { last: egui::pos2(1.0, 1.0), button: egui::PointerButton::Secondary });
    deliver(&mut state, id, false);
    assert!(state.link_ui.sessions[&id.into()].held.is_some());
    assert!(crate::sticker::edit_of(&state).unwrap().held_update);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers[0].id, first_layer);
    super::retry_held(&mut state, false);
    assert!(state.link_ui.sessions[&id.into()].held.is_some(), "applied while the drag was open");
    // A pending Preview edit also holds it.
    state.sticker_ui.drag = None;
    super::retry_held(&mut state, true);
    assert!(state.link_ui.sessions[&id.into()].held.is_some(), "applied while a tool interaction was open");
    // Released: applied in the next frame; the selection is gone, the active layer is the top one.
    super::retry_held(&mut state, false);
    assert!(state.link_ui.sessions[&id.into()].held.is_none());
    let d = state.doc().unwrap().sticker(id).unwrap();
    assert_eq!(d.layers.len(), 1);
    assert_eq!(d.layers[0].name, "pc layer");
    let e = crate::sticker::edit_of(&state).unwrap();
    assert!(e.selection.is_none() && !e.held_update);
    assert_eq!(e.active_layer, Some(d.layers[0].id));
    assert_eq!(state.link_ui.counters.held, 1);
}

#[test]
fn both_changed_is_a_conflict_and_a_send_while_open_makes_the_next_save_a_question() {
    let (mut state, stamp, _dir) = linked_state("conflict");
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    let layer = state.doc().unwrap().sticker(id).unwrap().layers[0].id;
    let rename = |state: &mut AppState, name: &'static str| {
        state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), move |d| d.layers[0].name = name.to_owned(), "Rename", Commit::Final).unwrap();
    };
    // WTE and the editor both change it.
    rename(&mut state, "wte name");
    assert_eq!(badge_text(&mut state, id), "changed in WTE");
    editor_adds_a_layer(&mut state, id, "red");
    deliver(&mut state, id, false);
    assert_eq!(question_kind(&state), Some(QuestionKind::Conflict { saved_over: false }));
    assert_eq!(open_question(&state).unwrap().lines[0], "Both WTE and PhotoCraft changed Logo.");
    // Take PhotoCraft's version: the file's layers, WTE's rename one undo away.
    press(&mut state, "Take PhotoCraft's version").unwrap();
    run_queued(&mut state);
    let d = state.doc().unwrap().sticker(id).unwrap();
    assert_eq!(d.layers.len(), 2);
    assert_ne!(d.layer(layer).unwrap().name, "wte name", "WTE's rename was kept");
    assert_eq!(badge_text(&mut state, id), "linked");
    // Again both; Keep WTE's version: the warning (the file was opened), Send anyway: rewritten, .bak.psd kept.
    rename(&mut state, "wte again");
    editor_adds_a_layer(&mut state, id, "green");
    deliver(&mut state, id, false);
    press(&mut state, "Keep WTE's version").unwrap();
    run_queued(&mut state);
    assert_eq!(question_kind(&state), Some(QuestionKind::SendWarning));
    assert!(open_question(&state).unwrap().lines[0].starts_with("PhotoCraft may still have this file open."));
    press(&mut state, "Send anyway").unwrap();
    run_queued(&mut state);
    let path = link_path(&state, id).unwrap();
    let read = testkit::read(&path).unwrap();
    assert_eq!(read.layers.len(), 2);
    assert!(read.layers.iter().any(|l| l.name == "wte again"));
    assert!(wte_core::link::file::backup_path(&path).is_file());
    assert_eq!(badge_text(&mut state, id), "linked");
    assert!(state.link_ui.sessions[&id.into()].stale_in_editor);
    // The editor saves its old picture over it: never applied silently.
    editor_adds_a_layer(&mut state, id, "blue");
    let undo = state.doc().unwrap().history.undo_len();
    deliver(&mut state, id, false);
    assert_eq!(state.doc().unwrap().history.undo_len(), undo, "applied silently");
    assert_eq!(question_kind(&state), Some(QuestionKind::Conflict { saved_over: true }));
    assert_eq!(open_question(&state).unwrap().lines[0], "PhotoCraft saved over the changes WTE sent to Logo.");
    // Decide later (also Esc): the badge says so; the answer clears the stale flag.
    press(&mut state, "Decide later").unwrap();
    run_queued(&mut state);
    assert!(!state.link_ui.sessions[&id.into()].stale_in_editor);
    assert_eq!(badge_text(&mut state, id), "changed in PhotoCraft");
}

#[test]
fn send_with_wte_changes_asks_send_first_on_edit_in() {
    let (mut state, stamp, _dir) = linked_state("send_first");
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |d| d.layers[0].opacity = 0.5, "Opacity", Commit::Final).unwrap();
    edit(&mut state, stamp);
    assert_eq!(question_kind(&state), Some(QuestionKind::SendFirst));
    assert_eq!(open_question(&state).unwrap().lines[0], "Send WTE's changes to the file first?");
    press(&mut state, "Open the file as it is").unwrap();
    run_queued(&mut state);
    assert_eq!(state.link_ui.launcher.calls().len(), 2);
    assert_eq!(badge_text(&mut state, id), "changed in WTE");
    // Send and open: the warning (opened before), then written and opened.
    edit(&mut state, stamp);
    press(&mut state, "Send and open").unwrap();
    run_queued(&mut state);
    press(&mut state, "Send anyway").unwrap();
    run_queued(&mut state);
    assert_eq!(badge_text(&mut state, id), "linked");
    assert_eq!(state.link_ui.launcher.calls().len(), 3);
    assert_eq!(testkit::read(&link_path(&state, id).unwrap()).unwrap().layers[0].opacity, 128);
}

// ---------------------------------------------------------------------------------------------------------------------
// Save, Save As, load, close
// ---------------------------------------------------------------------------------------------------------------------

fn save_project(state: &mut AppState, path: &Path) {
    flows::before_save(state, path);
    let snapshot = state.doc().unwrap().snapshot();
    let opts = wte_core::SaveOptions {
        png: wte_core::PngLevel::Fast,
        write_flattened: false,
        keep_backup: false,
        now_unix: 0,
        app_version: "test".into(),
    };
    let stats = wte_core::project::save(&snapshot, path, &opts, &wte_core::NoProgress).unwrap();
    state.session.as_mut().unwrap().path = Some(path.to_path_buf());
    let warnings: Vec<String> = stats.warnings.iter().map(|w| w.to_string()).collect();
    flows::after_save(state, snapshot.links_revision, &warnings);
}

#[test]
fn a_save_writes_no_link_file_and_save_as_takes_the_files_along_with_aliases() {
    let (mut state, stamp, dir) = linked_state("save");
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    let path = link_path(&state, id).unwrap();
    let before = (std::fs::read(&path).unwrap(), std::fs::metadata(&path).unwrap().modified().unwrap());
    let project = dir.join("proj.wtep");
    save_project(&mut state, &project);
    assert_eq!((std::fs::read(&path).unwrap(), std::fs::metadata(&path).unwrap().modified().unwrap()), before, "the save touched the file");
    assert!(wte_core::project::read_link_archive(&project, &wte_core::link::archive_entry(id)).is_ok());
    assert!(unsaved_line(&state).is_none(), "saved: no line in the close prompt");
    // Save As: the file is copied into the new links folder; the old path is followed as an alias (it was opened).
    let moved = dir.join("moved").join("copy.wtep");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    save_project(&mut state, &moved);
    let new_path = link_path(&state, id).unwrap();
    assert_eq!(new_path.parent().unwrap(), links_folder(&moved));
    assert_eq!(std::fs::read(&new_path).unwrap(), before.0);
    assert_eq!(state.link_ui.sessions[&id.into()].alias.as_deref(), Some(path.as_path()));
    assert!(state.toasts.visible_texts().iter().any(|t| t.starts_with("Logo may still be open in PhotoCraft from the old folder.")));
    let want = super::wanted(&state);
    assert_eq!(want.len(), 1);
    assert_eq!((want[0].path.as_path(), want[0].alias.as_deref()), (new_path.as_path(), Some(path.as_path())));
}

#[test]
fn after_a_load_a_missing_file_is_restored_and_a_changed_one_is_asked_about() {
    let (mut state, stamp, dir) = linked_state("load");
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    let project = dir.join("proj.wtep");
    save_project(&mut state, &project);
    let path = link_path(&state, id).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    // Missing: restored from the archived copy.
    std::fs::remove_file(&path).unwrap();
    let report = wte_core::project::load(&project, &wte_core::NoProgress).unwrap();
    assert_eq!(report.links[0].state, wte_core::project::LinkFileState::Missing);
    flows::after_load(&mut state, &project, &report.links);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(badge_text(&mut state, id), "linked");
    // Changed while closed: the question; the watcher's event waits for the answer; Update applies it.
    editor_adds_a_layer(&mut state, id, "while closed");
    let report = wte_core::project::load(&project, &wte_core::NoProgress).unwrap();
    assert_eq!(report.links[0].state, wte_core::project::LinkFileState::Changed);
    state.link_ui.sessions.clear();
    flows::after_load(&mut state, &project, &report.links);
    assert_eq!(question_kind(&state), Some(QuestionKind::ChangedWhileClosed));
    assert_eq!(open_question(&state).unwrap().lines[0], "Linked sticker Logo changed while the project was closed.");
    deliver(&mut state, id, false);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 1, "applied before the answer");
    press(&mut state, "Update").unwrap();
    run_queued(&mut state);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 2);
    // Ignore: the file with that content is not applied by itself afterwards.
    editor_adds_a_layer(&mut state, id, "second");
    flows::after_load(&mut state, &project, &[wte_core::project::LinkAtLoad {
        sticker: id,
        state: wte_core::project::LinkFileState::Changed,
        path: Some(path.clone()),
        archive: None,
    }]);
    press(&mut state, "Ignore").unwrap();
    run_queued(&mut state);
    deliver(&mut state, id, false);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 2, "an ignored file was applied");
    assert_eq!(badge_text(&mut state, id), "changed in PhotoCraft");
}

// ---------------------------------------------------------------------------------------------------------------------
// Import PSD as sticker, the command-line conversion
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_psd_file_becomes_a_tray_sticker_optionally_linked() {
    let (mut state, _, dir) = linked_state("import");
    let file = dir.join("art.psd");
    let psd = StickerPsd {
        size: [40, 30],
        layers: vec![
            testkit::rect_layer("back", 1, IRect::from_xywh(0, 0, 40, 30), [250, 250, 250, 255]),
            testkit::rect_layer("mark", 2, IRect::from_xywh(5, 5, 10, 10), [200, 0, 0, 255]),
        ],
        id_seed: 2,
    };
    testkit::save_as_editor(&file, &psd).unwrap();
    flows::import_psd(&mut state, &file, false);
    let crate::state::StampToolSource::Sticker(id) = state.tools.stamp.source else { panic!("{:?}", state.tools.stamp.source) };
    let d = state.doc().unwrap().sticker(id).unwrap();
    assert_eq!((d.name.as_str(), d.layers.len(), d.size(), d.keep), ("art", 2, [40, 30], true));
    assert!(state.doc().unwrap().link(id).is_none());
    assert!(matches!(state.actions.as_slice(), [AppAction::SetTool(crate::state::ToolKind::Stamp)]));
    // Keep linked: an external link, in sync, watched where the file is.
    flows::import_psd(&mut state, &file, true);
    let crate::state::StampToolSource::Sticker(linked) = state.tools.stamp.source else { panic!() };
    let l = state.doc().unwrap().link(linked).unwrap();
    assert_eq!(l.file, LinkFile::External(file.clone()));
    assert_eq!(badge_text(&mut state, linked), "linked");
    assert!(super::wanted(&state).iter().any(|w| w.id == linked.into() && w.path == file));
    // A refused file: its sentence, nothing added.
    let n = state.doc().unwrap().stickers().count();
    std::fs::write(dir.join("bad.psd"), vec![b'x'; 64]).unwrap();
    flows::import_psd(&mut state, &dir.join("bad.psd"), false);
    assert_eq!(state.doc().unwrap().stickers().count(), n);
}

#[test]
fn the_command_line_conversion_becomes_a_merged_update() {
    let (mut state, stamp, _dir) = linked_state("convert");
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    let exe = state.prefs.photocraft_exe.clone().unwrap();
    assert!(flows::cli_exe(&state.prefs).is_none());
    std::fs::write(exe.with_file_name("photocraft-cli.exe"), b"").unwrap();
    let png = wte_core::encode_png(4, 3, &[90u8; 48], wte_core::PngLevel::Fast).unwrap();
    state.link_ui.launcher = Box::new(StubLauncher { convert_png: Some(png), ..Default::default() });
    flows::run_in(&mut state, LinkAction::Convert(id));
    assert_eq!(state.link_ui.launcher.calls()[0].editor, "convert");
    flows::poll_convert(&mut state);
    assert!(state.link_ui.convert.is_none());
    assert_eq!(state.doc().unwrap().link(id).unwrap().mode, wte_core::link::LinkMode::Merged);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers[0].name, "PhotoCraft picture");
    let _ = LinkState::Linked;
}

pub(super) trait DialogName {
    fn dialog_name(&self) -> &'static str;
}

impl DialogName for AppState {
    fn dialog_name(&self) -> &'static str {
        match &self.dialog {
            crate::state::Dialog::None => "None",
            crate::state::Dialog::Link(_) => "Link",
            crate::state::Dialog::EditorLocations => "EditorLocations",
            _ => "other",
        }
    }
}

/// check-P9a: a PSD file larger than a sticker may be (here 6000 x 100, a flat file) is imported fitted, as one picture
/// of at most 4096 px with the toast saying so, instead of being refused with the "is now W x H in <editor>" question
/// text of a linked update.
#[test]
fn check_p9a_a_psd_larger_than_a_sticker_is_imported_fitted() {
    let (mut state, _, _dir) = linked_state("import_large");
    let mut merged = wte_core::PixelBuf::new(6000, 100);
    merged.data.as_chunks_mut::<4>().0.iter_mut().for_each(|px| *px = [30, 120, 200, 255]);
    let psd = wte_core::psd::PsdDoc {
        size: [6000, 100],
        depth: 8,
        psb: false,
        mode: wte_core::psd::PsdColorMode::Rgb,
        has_real_merged: true,
        merged: Some(merged),
        layers: Vec::new(),
        too_large: false,
        icc_description: None,
    };
    let stamp = FileStamp::of_bytes(b"large", None);
    let made = state
        .doc_mut()
        .unwrap()
        .add_sticker_from_psd(&psd, "wide", stamp, None, "photocraft", "PhotoCraft", "wide.psd")
        .expect("a large file is fitted, not refused");
    let d = state.doc().unwrap().sticker(made.id).unwrap();
    assert_eq!(d.size(), [4096, 69]);
    assert_eq!(d.layers.len(), 1);
    assert_eq!(made.mode, wte_core::link::LinkMode::Merged);
    let toast = made.toast.expect("the toast says what happened");
    assert!(toast.starts_with("wide.psd is 6000 x 100; a sticker can be at most 4096 px"), "{toast}");
    // Over the reader's limits stays refused.
    let huge = wte_core::psd::PsdDoc { size: [9000, 9000], merged: None, too_large: true, ..psd };
    let r = state.doc_mut().unwrap().add_sticker_from_psd(&huge, "huge", stamp, None, "photocraft", "PhotoCraft", "huge.psd");
    assert_eq!(r.unwrap_err(), wte_core::link::texts::too_large("huge.psd", 9000, 9000));
}
