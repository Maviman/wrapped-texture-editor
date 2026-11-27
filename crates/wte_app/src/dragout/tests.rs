//! Tests of drag out (STICKER_PLAN 7 "P11", 6.4, 13.5, 13.9): the data object each drag kind hands over (through
//! `ole::drag_source`'s slot, guard, data object and drop source, read back by the recorder - `DoDragDrop` itself is never
//! called), the state machine (the job at recognition, one post when the pointer has left, the post when a late job
//! finishes, the release in the window, the injected pointer-up), the folder's cleanup (review B7) and the PSD handle
//! (save first, the shared picture, the link and its PSD, the editor, the Photoshop toast once). Files live under
//! `target\wte-test-tmp\dragout\` (never `%TEMP%` or the real `%LOCALAPPDATA%`).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use super::*;
use crate::ole::drag_source::{self, RecordedDrag};
use crate::ole::formats;
use crate::state::{AppState, ViewMode};
use wte_core::{Commit, StylePreset};

/// A fresh folder under `target\wte-test-tmp\dragout\`, removed when dropped.
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
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wte-test-tmp/dragout").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TmpDir(dir)
}

/// The sphere with a selected image stamp; drag out on with the test port and `folder`.
fn state_in(folder: &Path) -> (AppState, wte_core::StampId) {
    let (mut state, stamp, _) = crate::sticker::tests::image_stamp_state(ViewMode::Split);
    assert!(state.drag_out.port.is_some(), "a self-test state has the test port");
    state.drag_out.folder = Some(folder.to_path_buf());
    (state, stamp)
}

/// The pointer held inside / outside the window, or released.
const HELD_IN: PointerNow = PointerNow { down: true, outside: false, pos: Some(egui::pos2(100.0, 100.0)) };
const HELD_OUT: PointerNow = PointerNow { down: true, outside: true, pos: Some(egui::pos2(-40.0, 100.0)) };
const RELEASED_IN: PointerNow = PointerNow { down: false, outside: false, pos: Some(egui::pos2(100.0, 100.0)) };

/// Steps with the button held inside until the job is ready.
fn wait_ready(state: &mut AppState) {
    for _ in 0..2000 {
        step(state, HELD_IN);
        if state.drag_out.current().is_none_or(|(_, phase)| phase != "preparing") {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the drag-out job did not finish");
}

/// A whole drag out of `source`: recognised, prepared, left the window, the post delivered to the recorder, the end
/// injected. Returns the recorded drag and the injected events.
fn drag_out(ctx: &egui::Context, state: &mut AppState, source: DragSource) -> (RecordedDrag, Vec<egui::Event>) {
    begin(ctx, state, source);
    wait_ready(state);
    let posted = state.drag_out.posted;
    step(state, HELD_OUT);
    assert_eq!(state.drag_out.posted, posted + 1, "posted once the pointer is outside: {:?}", state.drag_out.current());
    deliver_test_post(state);
    let mut raw = egui::RawInput::default();
    input_hook(ctx, &mut raw, state);
    let rec = drag_source::take_recorded().pop().expect("the recorder saw the drag");
    (rec, raw.events)
}

fn decode(png: &[u8]) -> PixelBuf {
    wte_core::decode_image(png, Some("x.png"), 16384).unwrap().pixels
}

fn dropped_path(rec: &RecordedDrag) -> PathBuf {
    let files = formats::parse_dropfiles(rec.bytes("CF_HDROP").expect("CF_HDROP"), 8);
    assert_eq!(files.len(), 1, "one file");
    files[0].clone()
}

/// A selected sticker stamp with the Outline style (promoted from the image stamp).
fn styled_sticker(state: &mut AppState, stamp: wte_core::StampId) -> StickerId {
    let doc = state.doc_mut().unwrap();
    let id = doc.promote_stamp(stamp, wte_core::ShareScope::ThisStamp).unwrap();
    doc.set_stamp_styles(stamp, StylePreset::Outline.styles([64, 64]), Commit::Final).unwrap();
    id
}

/// The data object of each drag kind (6.4): the thumbnail of a styled sticker stamp hands over `CF_HDROP` (one PNG
/// that exists in the folder) + `PNG` (what you see, with the stamp styles) + `CF_DIBV5` (the same pixels, alpha kept) +
/// `WTE.Sticker.v1` (this process); the stamp's layer row the same; a painted paint layer its picture at texture size
/// without a sticker; a tray sticker its picture without stamp styles and its sticker; a sticker layer in the Sticker
/// view its pixels.
#[test]
fn the_data_object_of_each_drag_kind() {
    let _one = drag_source::test_lock();
    drag_source::take_recorded();
    crate::ole::take_drag_ended();
    let dir = tmp("kinds");
    let ctx = egui::Context::default();
    let (mut state, stamp) = state_in(&dir);
    let sticker = styled_sticker(&mut state, stamp);
    let expected = state.doc_mut().unwrap().stamp_picture(stamp, true).unwrap();

    // The Sticker tab's thumbnail.
    let (rec, _) = drag_out(&ctx, &mut state, DragSource::Stamp(stamp));
    assert_eq!(rec.names(), ["CF_HDROP", "PNG", "CF_DIBV5", "WTE.Sticker.v1"]);
    assert!(rec.source_ok, "the drop source answers Esc / up / held as 6.4 says");
    let path = dropped_path(&rec);
    assert!(path.is_file() && path.parent() == Some(&*dir), "{}", path.display());
    assert_eq!(path.file_name().unwrap(), "Logo.png");
    assert_eq!(std::fs::read(&path).unwrap(), rec.bytes("PNG").unwrap(), "the file is the PNG offered");
    assert_eq!(decode(rec.bytes("PNG").unwrap()), expected, "the finished picture with the stamp styles");
    assert_eq!(formats::dib_to_pixels(rec.bytes("CF_DIBV5").unwrap()).unwrap(), expected, "alpha kept");
    let (pid, wtes) = formats::unwrap_sticker(rec.bytes("WTE.Sticker.v1").unwrap()).unwrap();
    assert_eq!(pid, std::process::id());
    let read = wte_core::read_wtes(wtes).unwrap();
    assert!(!read.styles.as_ref().unwrap().is_default(), "the sticker keeps the stamp styles");
    assert!(files::read_manifest(&dir).files.iter().any(|e| e.name == "Logo.png"), "recorded in the manifest");
    assert_eq!(state.status_hint, "Dragged out Logo.png");

    // The stamp's row in the Layers list: the same picture; a second file name.
    let row = state.doc().unwrap().stamp_layers(stamp)[0].1;
    let (rec, _) = drag_out(&ctx, &mut state, DragSource::Layer(row));
    assert_eq!(rec.names(), ["CF_HDROP", "PNG", "CF_DIBV5", "WTE.Sticker.v1"]);
    assert_eq!(dropped_path(&rec).file_name().unwrap(), "Logo (2).png", "never over an earlier file");
    assert_eq!(decode(rec.bytes("PNG").unwrap()), expected);

    // A painted paint layer: the full canvas at texture size, no sticker.
    let session = state.session.as_mut().unwrap();
    let set = session.active_set;
    let paint = session.doc.sets[set.index()].layers.iter().find(|l| matches!(l.kind, LayerKind::Raster(_)) && !l.locked).unwrap().id;
    session.doc.fill(set, paint, glam::Vec2::new(0.5, 0.5), [250, 10, 10, 255], wte_core::FillScope::Layer).unwrap();
    let (w, h) = { let s = state.doc().unwrap().set(set).unwrap(); (s.width, s.height) };
    let (rec, _) = drag_out(&ctx, &mut state, DragSource::Layer(paint));
    assert_eq!(rec.names(), ["CF_HDROP", "PNG", "CF_DIBV5"]);
    let got = decode(rec.bytes("PNG").unwrap());
    assert_eq!((got.width, got.height), (w, h), "texture size");
    let LayerKind::Raster(buf) = &state.doc().unwrap().layer(paint).unwrap().kind else { panic!() };
    assert_eq!(got.data, buf.data, "the layer's pixels, as Export this layer writes them");

    // A tray sticker: its picture without the stamp styles, and the sticker.
    let plain = state.doc_mut().unwrap().sticker_picture(sticker, None, 1.0).unwrap();
    let (rec, _) = drag_out(&ctx, &mut state, DragSource::Tray(sticker));
    assert_eq!(rec.names(), ["CF_HDROP", "PNG", "CF_DIBV5", "WTE.Sticker.v1"]);
    assert_eq!(decode(rec.bytes("PNG").unwrap()), plain);

    // A layer of the edited sticker (the Sticker view): its pixels.
    crate::sticker::run_in(&mut state, crate::sticker::StickerAction::Enter(Some(stamp)));
    let layer = crate::sticker::edit_of(&state).unwrap().active_layer.unwrap();
    let on = crate::sticker::edit_ref(crate::sticker::edit_of(&state).unwrap()).unwrap();
    let pixels = state.doc_mut().unwrap().sticker_layer_pixels(on, layer).unwrap();
    let (rec, _) = drag_out(&ctx, &mut state, DragSource::StickerLayer(layer));
    assert_eq!(rec.names(), ["CF_HDROP", "PNG", "CF_DIBV5"]);
    assert_eq!(decode(rec.bytes("PNG").unwrap()), *pixels);
    assert_eq!(state.drag_out.ended, 5);
    crate::ole::take_drag_ended();
}

/// A drag whose picture the test hands out after `gate` opens (a slow job).
fn held_job(ctx: &egui::Context, state: &mut AppState, gate: mpsc::Receiver<()>) {
    let tag = state.drag_out.next_tag;
    state.drag_out.next_tag += 1;
    let folder = state.drag_out.folder.clone().unwrap();
    let take: crate::clip::Take = Box::new(move || {
        gate.recv_timeout(Duration::from_secs(20)).map_err(|e| e.to_string())?;
        let mut p = PixelBuf::new(4, 3);
        p.data.fill(200);
        Ok((Arc::new(p), None))
    });
    start_job(ctx, state, tag, DragSource::Stamp(wte_core::StampId(999)), folder, Picked { what: "x".into(), name: "Held".into(), take });
}

/// The state machine (6.4): the job starts when the drag is recognised; with the button held inside nothing is posted;
/// leaving the window before the job is done posts nothing until it finishes, and then exactly once (also when the
/// pointer stays outside for many frames); the end injects the primary button's release and `PointerGone`, clears
/// egui's drag payload and marks the frame's release as no reorder (the next frame's hook clears the mark).
#[test]
fn a_drag_posts_once_when_the_pointer_has_left_and_the_job_is_done() {
    let _one = drag_source::test_lock();
    drag_source::take_recorded();
    crate::ole::take_drag_ended();
    let dir = tmp("machine");
    let ctx = egui::Context::default();
    let (mut state, _) = state_in(&dir);
    let (open, gate) = mpsc::channel();
    held_job(&ctx, &mut state, gate);
    assert_eq!(state.drag_out.jobs, 1, "the job starts at recognition");
    for _ in 0..5 {
        step(&mut state, HELD_IN);
    }
    step(&mut state, HELD_OUT);
    step(&mut state, HELD_OUT);
    assert_eq!(state.drag_out.posted, 0, "the job is not done: nothing starts yet");
    assert_eq!(state.drag_out.current().unwrap().1, "preparing");
    open.send(()).unwrap();
    for _ in 0..2000 {
        step(&mut state, HELD_OUT);
        if state.drag_out.posted > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(state.drag_out.posted, 1, "posted in the frame the job finished");
    for _ in 0..10 {
        step(&mut state, HELD_OUT);
    }
    assert_eq!(state.drag_out.posted, 1, "exactly one post");
    assert_eq!(state.drag_out.current().unwrap().1, "posted");
    // A drag in egui's memory (the Layers row's payload) is cleared by the end.
    egui::DragAndDrop::set_payload(&ctx, wte_core::LayerId(5));
    deliver_test_post(&state);
    let mut raw = egui::RawInput::default();
    input_hook(&ctx, &mut raw, &mut state);
    assert!(matches!(raw.events[0], egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, pos, .. } if pos == HELD_OUT.pos.unwrap()));
    assert_eq!(raw.events[1], egui::Event::PointerGone);
    assert!(!egui::DragAndDrop::has_any_payload(&ctx), "the row's drag is cancelled");
    assert!(state.drag_out.released_this_frame && state.drag_out.current().is_none());
    assert_eq!(drag_source::take_recorded().len(), 1);
    let mut raw = egui::RawInput::default();
    input_hook(&ctx, &mut raw, &mut state);
    assert!(raw.events.is_empty() && !state.drag_out.released_this_frame, "nothing more is injected");
}

/// Releasing inside the window before leaving posts nothing and removes the file nobody took; a job that is still
/// running when the button is released removes its own file; leaving after the release starts nothing.
#[test]
fn releasing_first_posts_nothing_and_removes_the_file() {
    let _one = drag_source::test_lock();
    let dir = tmp("release");
    let ctx = egui::Context::default();
    let (mut state, stamp) = state_in(&dir);
    begin(&ctx, &mut state, DragSource::Stamp(stamp));
    wait_ready(&mut state);
    let file = state.drag_out.current_file().unwrap().to_path_buf();
    assert!(file.is_file());
    assert_eq!(files::read_manifest(&dir).files.len(), 1);
    step(&mut state, RELEASED_IN);
    assert_eq!(state.drag_out.posted, 0);
    assert!(!file.exists(), "the file nobody took is removed");
    assert!(files::read_manifest(&dir).files.is_empty(), "and forgotten");
    step(&mut state, PointerNow { down: false, outside: true, pos: None });
    assert_eq!(state.drag_out.posted, 0);
    // Released while the job still runs: the worker removes its file when it finds the drag gone.
    let (open, gate) = mpsc::channel();
    held_job(&ctx, &mut state, gate);
    step(&mut state, RELEASED_IN);
    assert!(state.drag_out.current().is_none());
    open.send(()).unwrap();
    for _ in 0..500 {
        if std::fs::read_dir(&*dir).unwrap().filter_map(Result::ok).all(|e| e.file_name() == files::MANIFEST) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let left: Vec<_> = std::fs::read_dir(&*dir).unwrap().filter_map(Result::ok).map(|e| e.file_name()).filter(|n| n != files::MANIFEST).collect();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(state.drag_out.posted, 0);
}

/// A drag the user cancels (Esc: effect none) removes its PNG; a new drag while one is prepared replaces it.
#[test]
fn a_cancelled_drag_removes_its_file_and_a_new_drag_replaces_the_old() {
    let _one = drag_source::test_lock();
    drag_source::take_recorded();
    crate::ole::take_drag_ended();
    let dir = tmp("cancel");
    let ctx = egui::Context::default();
    let (mut state, stamp) = state_in(&dir);
    state.drag_out.test_effect = 0;
    let (rec, events) = drag_out(&ctx, &mut state, DragSource::Stamp(stamp));
    assert_eq!(rec.effect, 0);
    assert!(!dropped_path(&rec).exists(), "nobody took it: removed");
    assert_eq!(events.len(), 2, "the release is injected after a cancelled drag too");
    state.drag_out.test_effect = 1;
    begin(&ctx, &mut state, DragSource::Stamp(stamp));
    wait_ready(&mut state);
    let first = state.drag_out.current_file().unwrap().to_path_buf();
    begin(&ctx, &mut state, DragSource::Stamp(stamp));
    assert!(!first.exists(), "the replaced drag's file is removed");
    assert_eq!(state.drag_out.jobs, 3);
    cancel(&mut state);
    crate::ole::take_drag_ended();
}

/// P11 check: a drop that a program answered with "no effect" (`DRAGDROP_S_DROP`, `DROPEFFECT_NONE`) keeps the PNG -
/// the program may read it after the drop - listed in the manifest for the day-old cleanup; only a drag that ended
/// without a drop (Esc) removes it at once.
#[test]
fn check_p11_a_drop_without_an_effect_keeps_the_file_for_the_cleanup() {
    let _one = drag_source::test_lock();
    drag_source::take_recorded();
    crate::ole::take_drag_ended();
    let dir = tmp("noeffect");
    let ctx = egui::Context::default();
    let (mut state, stamp) = state_in(&dir);
    struct DroppedNoEffect;
    impl drag_source::DoDrag for DroppedNoEffect {
        fn do_drag(&self, _data: *mut std::ffi::c_void, _source: *mut std::ffi::c_void) -> (crate::ole::com::Hresult, u32) {
            (crate::ole::com::DRAGDROP_S_DROP, crate::ole::com::DROPEFFECT_NONE)
        }
    }
    begin(&ctx, &mut state, DragSource::Stamp(stamp));
    wait_ready(&mut state);
    let file = state.drag_out.current_file().unwrap().to_path_buf();
    step(&mut state, HELD_OUT);
    assert!(drag_source::deliver_test_post(&DroppedNoEffect).is_some_and(|e| e.dropped && e.effect == 0));
    let mut raw = egui::RawInput::default();
    input_hook(&ctx, &mut raw, &mut state);
    assert_eq!(raw.events.len(), 2, "the release is injected");
    assert!(file.exists(), "a dropped file stays");
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    assert!(files::read_manifest(&dir).files.iter().any(|e| e.name == name), "listed for the cleanup");
    assert_eq!(state.drag_out.removed, 0);
    assert!(state.drag_out.current().is_none());
    // A day later the start-up cleanup removes it.
    let later = SystemTime::now() + files::MAX_AGE + Duration::from_secs(60);
    assert_eq!(files::cleanup(&dir, later).removed, [name]);
    assert!(!file.exists());
    crate::ole::take_drag_ended();
}

/// Cleanup (review B7): a file of the manifest older than a day is removed, a fresh one kept, a file not in the
/// manifest kept, a manifest file whose size or time changed kept (and forgotten), a name with a path in it ignored,
/// the folder and the manifest never removed.
#[test]
fn the_cleanup_removes_only_old_unchanged_files_of_wtes() {
    let dir = tmp("cleanup");
    let day = files::MAX_AGE;
    let now = SystemTime::now();
    // Three files of WTE's, recorded as `pump` records them: "old" two days old, "fresh" an hour old, "changed" two
    // days old and then saved over by the user. And one file WTE never wrote.
    let mut written = Vec::new();
    for (name, age) in [("old.png", day * 2), ("fresh.png", Duration::from_secs(3600)), ("changed.png", day * 2)] {
        let (path, _) = files::write_new(&dir, name, name.as_bytes()).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(now - age).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let ns = meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        files::record(&dir, files::ManifestEntry { name: name.into(), size: meta.len(), modified_ns: ns });
        written.push(path);
    }
    std::fs::write(&written[2], b"the user saved over it").unwrap();
    std::fs::File::options().write(true).open(&written[2]).unwrap().set_modified(now - day * 2).unwrap();
    let foreign = dir.join("user's own.png");
    std::fs::write(&foreign, b"saved here by PhotoCraft's Save As").unwrap();
    std::fs::File::options().write(true).open(&foreign).unwrap().set_modified(now - day * 3).unwrap();
    // A hand-edited manifest that names a file outside the folder.
    let outside = dir.parent().unwrap().join(format!("outside-{}.png", std::process::id()));
    std::fs::write(&outside, b"x").unwrap();
    let mut m = files::read_manifest(&dir);
    let meta = std::fs::metadata(&outside).unwrap();
    let ns = meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    m.files.push(files::ManifestEntry { name: format!(r"..\{}", outside.file_name().unwrap().to_string_lossy()), size: 1, modified_ns: ns });
    std::fs::write(dir.join(files::MANIFEST), serde_json::to_vec(&m).unwrap()).unwrap();

    let c = files::cleanup(&dir, now);
    assert_eq!(c.removed, ["old.png"], "{c:?}");
    assert_eq!(c.kept_fresh, ["fresh.png"], "an hour old (and a day of waiting is not over): kept");
    assert_eq!(c.forgotten.len(), 2, "{c:?}");
    assert!(!written[0].exists());
    assert!(written[1].is_file() && written[2].is_file() && foreign.is_file() && outside.is_file());
    assert!(dir.is_dir() && dir.join(files::MANIFEST).is_file(), "the folder and the manifest stay");
    let names: Vec<String> = files::read_manifest(&dir).files.into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["fresh.png"], "a changed file is forgotten: never touched again");
    // Later the fresh one is old too; the changed one and the user's own file stay for good.
    let c = files::cleanup(&dir, now + day * 2);
    assert_eq!(c.removed, ["fresh.png"]);
    assert!(written[2].is_file() && foreign.is_file());
    // No manifest at all: nothing is removed.
    std::fs::remove_file(dir.join(files::MANIFEST)).unwrap();
    assert_eq!(files::cleanup(&dir, now + day * 50), files::Cleanup::default());
    assert!(written[2].is_file() && foreign.is_file());
    let _ = std::fs::remove_file(&outside);
}

#[test]
fn file_names_are_safe_and_never_replace_a_file() {
    assert_eq!(files::file_name("Logo", "png"), "Logo.png");
    assert_eq!(files::file_name(r#"a/b\c:d*e?"f<g>h|i"#, "png"), "a_b_c_d_e__f_g_h_i.png");
    assert_eq!(files::file_name(" .. ", "png"), "picture.png");
    assert_eq!(files::file_name("CON", "png"), "picture CON.png");
    assert_eq!(files::file_name("наклейка", "png"), "наклейка.png");
    assert_eq!(files::file_name(&"x".repeat(300), "png").len(), 84);
    let dir = tmp("names");
    let (a, _) = files::write_new(&dir, "Logo.png", b"1").unwrap();
    let (b, _) = files::write_new(&dir, "Logo.png", b"2").unwrap();
    assert_eq!((a.file_name().unwrap(), b.file_name().unwrap()), (std::ffi::OsStr::new("Logo.png"), std::ffi::OsStr::new("Logo (2).png")));
    assert_eq!(std::fs::read(&a).unwrap(), b"1");
}

// ---------------------------------------------------------------------------------------------------------------------
// The PSD handle
// ---------------------------------------------------------------------------------------------------------------------

/// A project "saved" at `<dir>\proj.wtep` (the path only) with a fake PhotoCraft and a fake Photoshop program.
fn saved(state: &mut AppState, dir: &Path, default_editor: Option<&str>) {
    state.session.as_mut().unwrap().path = Some(dir.join("proj.wtep"));
    for (exe, set) in [("photocraft.exe", 0), ("Photoshop.exe", 1)] {
        let path = dir.join("programs").join(exe);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();
        if set == 0 {
            state.prefs.photocraft_exe = Some(path);
        } else {
            state.prefs.photoshop_exe = Some(path);
        }
    }
    state.link_ui.discovered = true;
    crate::link::editor::refresh_known(state);
    state.prefs.default_editor = default_editor.map(str::to_owned);
}

/// The PSD handle (6.4, 13.9): on an unsaved project it asks "Save first" and no drag starts; on a picture two stamps
/// show it asks the shared-picture question first; otherwise it makes the link record and the PSD (the image stamp
/// promoted, the link's editor the default editor) and the drag hands over `CF_HDROP` with the linked PSD only.
#[test]
fn the_psd_handle_asks_first_then_links_and_hands_out_the_psd() {
    let _one = drag_source::test_lock();
    drag_source::take_recorded();
    crate::ole::take_drag_ended();
    let dir = tmp("handle");
    let ctx = egui::Context::default();
    let (mut state, stamp) = state_in(&dir.join("dragout"));
    // Unsaved: "Save first", nothing to drag, no link.
    begin(&ctx, &mut state, DragSource::PsdHandle(stamp));
    assert_eq!(crate::link::open_question(&state).map(|q| q.kind), Some(crate::link::QuestionKind::SaveFirst));
    assert!(state.drag_out.current().is_none());
    assert!(state.doc().unwrap().links().next().is_none());
    state.dialog = crate::state::Dialog::None;
    state.link_ui.questions.clear();
    // Two stamps show the picture: the shared question first.
    saved(&mut state, &dir, Some("photocraft"));
    let blob = match state.doc().unwrap().stamp(stamp).unwrap().source { wte_core::StampSource::Image { blob } => blob, _ => panic!() };
    let other = {
        let doc = state.doc_mut().unwrap();
        let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.3, 0.5), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::splat(0.2), 0.6);
        doc.add_stamp("Logo 2", wte_core::StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap()
    };
    begin(&ctx, &mut state, DragSource::PsdHandle(stamp));
    assert_eq!(crate::link::open_question(&state).map(|q| q.kind), Some(crate::link::QuestionKind::Shared));
    assert!(state.drag_out.current().is_none(), "no drag before the answer");
    // "Only this one": the link is made, nothing starts, a status line says how to go on.
    crate::link::press(&mut state, "Only this one").unwrap();
    run_queued(&mut state);
    let sticker = crate::link::sticker_of_stamp(&state, stamp).expect("promoted");
    let link = state.doc().unwrap().link(sticker).cloned().expect("the link record");
    assert_eq!(link.editor, "photocraft");
    let psd = crate::link::link_path(&state, sticker).unwrap();
    assert!(psd.is_file(), "the PSD is written into the links folder");
    assert!(state.status_hint.contains("drag the PSD handle to PhotoCraft"), "{}", state.status_hint);
    assert!(matches!(state.doc().unwrap().stamp(other).unwrap().source, wte_core::StampSource::Image { .. }), "the other stamp keeps its picture");
    assert!(!state.toasts.visible_labels().iter().any(|l| l.starts_with("Opening")), "no editor is started");
    assert!(state.link_ui.sessions.get(&sticker).is_none_or(|s| s.child.is_none() && !s.opened));
    // Now the drag: CF_HDROP with the linked PSD only; the link counts as opened in the link's editor.
    let (rec, _) = drag_out(&ctx, &mut state, DragSource::PsdHandle(stamp));
    assert_eq!(rec.names(), ["CF_HDROP"]);
    assert_eq!(dropped_path(&rec), psd);
    let s = state.link_ui.sessions.get(&sticker).unwrap();
    assert!(s.opened && s.opened_editor.as_deref() == Some("photocraft"));
    assert!(psd.is_file(), "the linked PSD is never removed");
    // Photoshop is known on this machine: the tab-bar toast once per session.
    let toasts = |s: &AppState| s.toasts.visible_labels().iter().filter(|l| l.starts_with("In Photoshop the sticker stays linked")).cloned().collect::<Vec<_>>();
    assert_eq!(toasts(&state), [PHOTOSHOP_TAB_TOAST]);
    // The default editor becomes Photoshop: the next press makes it the link's editor; the toast does not come again.
    state.prefs.default_editor = Some("photoshop".into());
    let (rec, _) = drag_out(&ctx, &mut state, DragSource::PsdHandle(stamp));
    assert_eq!(dropped_path(&rec), psd);
    assert_eq!(state.doc().unwrap().link(sticker).unwrap().editor, "photoshop", "the link's editor = the default editor");
    assert_eq!(toasts(&state), [PHOTOSHOP_TAB_TOAST], "once per session");
    assert!(state.drag_out.psd_toast_shown);
    // A text stamp: a status line, no link, no drag.
    crate::ole::take_drag_ended();
}

/// The toast comes only after a drag that was taken, and only when Photoshop plays a part (13.5).
#[test]
fn the_tab_bar_toast_needs_photoshop_and_a_taken_drag() {
    let _one = drag_source::test_lock();
    drag_source::take_recorded();
    crate::ole::take_drag_ended();
    let dir = tmp("toast");
    let ctx = egui::Context::default();
    let (mut state, stamp) = state_in(&dir.join("dragout"));
    saved(&mut state, &dir, Some("photocraft"));
    state.prefs.photoshop_exe = None;
    crate::link::editor::refresh_known(&mut state);
    let toast = |s: &AppState| s.toasts.visible_labels().iter().any(|l| l.starts_with("In Photoshop"));
    drag_out(&ctx, &mut state, DragSource::PsdHandle(stamp));
    assert!(!toast(&state), "PhotoCraft only: no Photoshop toast");
    saved(&mut state, &dir, Some("photoshop"));
    state.drag_out.test_effect = 0;
    drag_out(&ctx, &mut state, DragSource::PsdHandle(stamp));
    assert!(!toast(&state) && !state.drag_out.psd_toast_shown, "a cancelled drag: no toast");
    state.drag_out.test_effect = 1;
    drag_out(&ctx, &mut state, DragSource::PsdHandle(stamp));
    assert!(toast(&state));
    crate::ole::take_drag_ended();
}

/// Runs the queued link and sticker actions (what dispatch does next frame).
fn run_queued(state: &mut AppState) {
    for _ in 0..8 {
        let actions = std::mem::take(&mut state.actions);
        if actions.is_empty() {
            break;
        }
        for a in actions {
            match a {
                crate::action::AppAction::Link(l) => crate::link::flows::run_in(state, l),
                crate::action::AppAction::Sticker(s) => crate::sticker::run_in(state, s),
                crate::action::AppAction::ShowDialog(crate::state::Dialog::None) => state.dialog = crate::state::Dialog::None,
                _ => {}
            }
        }
    }
}

/// A text stamp's handle: a status line, nothing linked; without a port (drag out not available) nothing happens.
#[test]
fn a_text_stamp_and_no_port() {
    let dir = tmp("text");
    let ctx = egui::Context::default();
    let mut state = crate::tools::test_support::state_with(wte_core::SampleKind::Sphere);
    state.drag_out.folder = Some(dir.to_path_buf());
    saved(&mut state, &dir, None);
    let font = state.doc_mut().unwrap().default_font();
    let spec = wte_core::TextSpec::new("Hi", font);
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.0, 0.5), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::splat(0.3), 0.6);
    let text = state.doc_mut().unwrap().add_stamp("Hello", wte_core::StampSource::Text(spec), wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    begin(&ctx, &mut state, DragSource::PsdHandle(text));
    assert_eq!(state.status_hint, crate::link::flows::HANDLE_TEXT_HINT);
    assert!(state.doc().unwrap().links().next().is_none() && state.drag_out.current().is_none());
    state.drag_out.port = None;
    let layer = state.doc().unwrap().sets[0].layers[0].id;
    begin(&ctx, &mut state, DragSource::Layer(layer));
    assert_eq!(state.drag_out.jobs, 0);
}

/// WTE's drop target takes nothing while this thread runs one of WTE's own drags (R3), even plain files.
#[test]
fn wtes_own_drop_target_ignores_its_own_drag() {
    let _one = drag_source::test_lock();
    use crate::ole::com::*;
    use crate::ole::drop_target::{CAPS, DropTarget, Formats};
    struct OntoSelf;
    impl drag_source::DoDrag for OntoSelf {
        fn do_drag(&self, data: *mut std::ffi::c_void, _source: *mut std::ffi::c_void) -> (Hresult, u32) {
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sink = Arc::clone(&seen);
            let target = DropTarget::create(0, Formats::get(), CAPS, Box::new(move |e| sink.lock().unwrap().push(e)));
            let mut effect = DROPEFFECT_COPY;
            // SAFETY: our own live objects; a valid effect pointer.
            unsafe {
                let vt = &*(*(target as *mut ComObject<IDropTargetVtbl>)).vtbl;
                (vt.drag_enter)(target, data, MK_LBUTTON, PointL::default(), &mut effect);
                assert_eq!(effect, DROPEFFECT_NONE, "WTE's own drag is not taken back");
                let mut effect = DROPEFFECT_COPY;
                (vt.drop)(target, data, 0, PointL::default(), &mut effect);
                assert_eq!(effect, DROPEFFECT_NONE);
                release(target);
            }
            assert!(seen.lock().unwrap().is_empty(), "no hover, no drop");
            (DRAGDROP_S_DROP, DROPEFFECT_NONE)
        }
    }
    let job = drag_source::DragJob {
        tag: 901,
        entries: vec![crate::ole::data_object::Entry::global(formats::CF_HDROP, formats::build_dropfiles(&[PathBuf::from(r"C:\x\a.png")]))],
    };
    drag_source::request(job, &mut || true).unwrap();
    assert_eq!(drag_source::handle_message(&OntoSelf).map(|e| e.effect), Some(DROPEFFECT_NONE));
    assert!(!drag_source::own_drag_on_this_thread());
    crate::ole::take_drag_ended();
}

/// STICKER_PLAN 8.4: the drag-out PNG of a 4096 x 4096 texture layer (`PngLevel::Fast`) is made on a job; the UI thread
/// never waits for it (each `start_job` / `step` call is short while the job runs). Run with `--release --ignored`.
#[test]
#[ignore = "perf: run with --release --ignored perf_k11"]
fn perf_k11_a_4096_layer_is_prepared_off_the_ui_thread() {
    let _one = drag_source::test_lock();
    let dir = tmp("perf");
    let ctx = egui::Context::default();
    let (mut state, _) = state_in(&dir);
    let mut p = PixelBuf::new(4096, 4096);
    for (i, px) in p.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        *px = [(i % 251) as u8, (i / 4096 % 253) as u8, 90, if i % 7 == 0 { 0 } else { 255 }];
    }
    let buf = Arc::new(p);
    let t0 = std::time::Instant::now();
    let take: crate::clip::Take = Box::new(move || Ok((buf, None)));
    let tag = state.drag_out.next_tag;
    start_job(&ctx, &mut state, tag, DragSource::Layer(wte_core::LayerId(1)), dir.to_path_buf(), Picked { what: "x".into(), name: "Big".into(), take });
    let start_ui = t0.elapsed();
    let mut worst_step = Duration::ZERO;
    while state.drag_out.current().is_some_and(|(_, phase)| phase == "preparing") {
        let t = std::time::Instant::now();
        step(&mut state, HELD_IN);
        worst_step = worst_step.max(t.elapsed());
        std::thread::sleep(Duration::from_millis(1));
    }
    let job = t0.elapsed();
    println!("perf_k11: job {job:?}; UI: start_job {start_ui:?}, worst step {worst_step:?}");
    assert!(start_ui < Duration::from_millis(10) && worst_step < Duration::from_millis(10), "the UI thread waited");
    assert_eq!(state.drag_out.current().map(|(_, p)| p), Some("ready"));
    cancel(&mut state);
}

/// Stickers P11 check (8.4, review A23): the start of a drag of a large styled stamp (every Layers-row drag of a stamp
/// layer, also an in-window reorder) takes its base picture from the caches only; the styles are drawn by the job. Was
/// 34-77 ms on the UI thread for a 2048 x 2048 picture with a preset (release). Run with `--release --ignored`.
#[test]
#[ignore = "perf: run with --release --ignored perf_k11"]
fn perf_k11_a_styled_stamps_drag_starts_without_drawing_its_styles() {
    let _one = drag_source::test_lock();
    let dir = tmp("perf_styled");
    let ctx = egui::Context::default();
    let (mut state, _) = state_in(&dir);
    let doc = state.doc_mut().unwrap();
    let mut img = PixelBuf::new(2048, 2048);
    for (i, px) in img.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i % 2048) as f32 - 1024.0, (i / 2048) as f32 - 1024.0);
        *px = if x * x + y * y < 900.0 * 900.0 { [(i % 251) as u8, 120, 40, 255] } else { [0, 0, 0, 0] };
    }
    let png = wte_core::encode_png(2048, 2048, &img.data, wte_core::PngLevel::Fast).unwrap();
    let blob = doc.add_image_blob("big.png", std::sync::Arc::from(png)).unwrap();
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.0, 0.5), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::splat(0.3), 0.6);
    let stamp = doc.add_stamp("Big", wte_core::StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    let _ = doc.stamp_picture(stamp, false);
    for preset in [StylePreset::Outline, StylePreset::ClassicSticker, StylePreset::Glow] {
        state.doc_mut().unwrap().set_stamp_styles(stamp, preset.styles([2048, 2048]), Commit::Final).unwrap();
        let t = std::time::Instant::now();
        begin(&ctx, &mut state, DragSource::Stamp(stamp));
        let ui = t.elapsed();
        println!("perf_k11 styled: {}: begin on the UI thread {ui:?}", preset.name());
        assert!(ui < Duration::from_millis(10), "{}: the UI thread drew the styles ({ui:?})", preset.name());
        wait_ready(&mut state);
        let file = state.drag_out.current_file().unwrap().to_path_buf();
        let expected = state.doc_mut().unwrap().stamp_picture(stamp, true).unwrap();
        assert!(decode(&std::fs::read(&file).unwrap()) == expected, "{}: the job's PNG is the styled picture", preset.name());
        cancel(&mut state);
    }
}
