//! Tests of the editor list (stickers P9b; STICKER_PLAN 13.1, 13.9 "P9b"): Photoshop's discovery from an injected
//! registry, the default editor and the chooser, `command_for` for Photoshop, the per-editor two-copies guard, the
//! question before a link opens in a second editor, Help > Editor locations..., the prefs, the "could not be replaced"
//! message. Every launcher is the stub, every registry an injected map; files live under `target\wte-test-tmp\link\`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use wte_core::link::{EDITOR_OTHER, EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP, texts};
use wte_core::{Commit, StickerRef};

use super::editor::{self, ExternalEditor};
use super::launch::{self, Hive, RegView, RegistryReader, StubLauncher};
use super::tests::{DialogName, TmpDir, badge_text, edit, linked_state, question_kind, run_queued, sticker_id};
use super::*;
use crate::state::AppState;

const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Photoshop.exe";
const PS_COMMAND: &str = r"Software\Classes\Applications\Photoshop.exe\shell\open\command";
const PS_2019: &str = r"C:\Program Files\Adobe\Adobe Photoshop CC 2019\Photoshop.exe";

/// A registry with values per (hive, view, key); counts the reads.
#[derive(Default)]
struct ViewRegistry {
    values: HashMap<(Hive, RegView, String), String>,
    reads: Arc<AtomicUsize>,
}

impl ViewRegistry {
    fn with(entries: &[(Hive, RegView, &str, &str)]) -> ViewRegistry {
        ViewRegistry {
            values: entries.iter().map(|(h, v, k, val)| ((*h, *v, (*k).to_owned()), (*val).to_owned())).collect(),
            reads: Arc::default(),
        }
    }
}

impl RegistryReader for ViewRegistry {
    fn default_value(&self, hive: Hive, view: RegView, subkey: &str) -> Option<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.values.get(&(hive, view, subkey.to_owned())).cloned()
    }
}

/// A fake program file `<dir>\<sub>\<name>` (an empty file: the stub never starts it).
fn fake_exe(dir: &Path, sub: &str, name: &str) -> PathBuf {
    let exe = dir.join(sub).join(name);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"").unwrap();
    exe
}

/// The linked-state fixture of `tests` with a fake Photoshop known as well (no default editor).
fn two_editors(name: &str) -> (AppState, wte_core::StampId, TmpDir) {
    let (mut state, stamp, dir) = linked_state(name);
    state.prefs.photoshop_exe = Some(fake_exe(&dir, "ps", "Photoshop.exe"));
    (state, stamp, dir)
}

fn labels(state: &AppState) -> Vec<String> {
    open_question(state).map(|q| q.buttons.iter().map(|b| b.label.clone()).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------------------------------
// Discovery, launching
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn photoshop_is_discovered_from_app_paths_or_its_command_in_both_views() {
    let exists_set = [PS_2019, r"D:\PS 2024\Photoshop.exe", r"E:\old\Photoshop.exe"];
    let exists = |p: &Path| exists_set.iter().any(|e| Path::new(e) == p);
    let ps = ExternalEditor::Photoshop;
    use {Hive::*, RegView::*};
    // App Paths: a plain path (no quotes), HKLM, 64-bit view: the machine's CC 2019.
    let r = ViewRegistry::with(&[(LocalMachine, Bits64, APP_PATHS, PS_2019)]);
    assert_eq!(launch::discover(&ps, &r, &exists), Some(PathBuf::from(PS_2019)));
    // The quoted command of the Applications key, with "%1".
    let r = ViewRegistry::with(&[(CurrentUser, Bits64, PS_COMMAND, r#""D:\PS 2024\Photoshop.exe" "%1""#)]);
    assert_eq!(launch::discover(&ps, &r, &exists), Some(PathBuf::from(r"D:\PS 2024\Photoshop.exe")));
    // Only in the 32-bit view (an old 32-bit Photoshop).
    let r = ViewRegistry::with(&[(LocalMachine, Bits32, APP_PATHS, r"E:\old\Photoshop.exe")]);
    assert_eq!(launch::discover(&ps, &r, &exists), Some(PathBuf::from(r"E:\old\Photoshop.exe")));
    // The 64-bit view before the 32-bit one, HKCU before HKLM, App Paths before the command.
    let r = ViewRegistry::with(&[
        (LocalMachine, Bits32, APP_PATHS, r"E:\old\Photoshop.exe"),
        (LocalMachine, Bits64, APP_PATHS, PS_2019),
        (CurrentUser, Bits64, PS_COMMAND, r#""D:\PS 2024\Photoshop.exe" "%1""#),
    ]);
    assert_eq!(launch::discover(&ps, &r, &exists), Some(PathBuf::from(PS_2019)));
    let r = ViewRegistry::with(&[(LocalMachine, Bits64, APP_PATHS, PS_2019), (CurrentUser, Bits64, APP_PATHS, r"D:\PS 2024\Photoshop.exe")]);
    assert_eq!(launch::discover(&ps, &r, &exists), Some(PathBuf::from(r"D:\PS 2024\Photoshop.exe")));
    // A path that does not exist is skipped; nothing in the registry: not found (and nothing is asked).
    let r = ViewRegistry::with(&[(CurrentUser, Bits64, APP_PATHS, r"C:\gone\Photoshop.exe"), (LocalMachine, Bits64, PS_COMMAND, &format!("\"{PS_2019}\" \"%1\""))]);
    assert_eq!(launch::discover(&ps, &r, &exists), Some(PathBuf::from(PS_2019)));
    assert_eq!(launch::discover(&ps, &ViewRegistry::default(), &exists), None);
    let r = ViewRegistry::with(&[(LocalMachine, Bits64, APP_PATHS, r"C:\gone\Photoshop.exe")]);
    assert_eq!(launch::discover(&ps, &r, &exists), None);
    // Photoshop's keys never name PhotoCraft, and another program is never discovered.
    let r = ViewRegistry::with(&[(LocalMachine, Bits64, APP_PATHS, PS_2019)]);
    assert_eq!(launch::discover(&ExternalEditor::PhotoCraft, &r, &exists), None);
    assert_eq!(launch::discover(&ExternalEditor::Other(PS_2019.into()), &r, &exists), None);
    assert_eq!(launch::registry_keys(&ps), [APP_PATHS, PS_COMMAND]);
}

#[test]
fn discovery_runs_once_per_session_and_again_for_editor_locations() {
    let (mut state, _stamp, dir) = linked_state("discover_once");
    let ps = fake_exe(&dir, "Adobe Photoshop CC 2019", "Photoshop.exe");
    let reg = ViewRegistry::with(&[(Hive::LocalMachine, RegView::Bits64, APP_PATHS, ps.to_str().unwrap())]);
    let reads = Arc::clone(&reg.reads);
    state.link_ui.registry = Box::new(reg);
    editor::ensure_discovered(&mut state, false);
    assert_eq!(state.prefs.photoshop_exe.as_deref(), Some(ps.as_path()));
    assert_eq!(state.link_ui.known, [ExternalEditor::Photoshop, ExternalEditor::PhotoCraft]);
    let after_first = reads.load(Ordering::SeqCst);
    editor::ensure_discovered(&mut state, false);
    assert_eq!(reads.load(Ordering::SeqCst), after_first, "the registry was read again in the same session");
    // Help > Editor locations... looks again (for programs that are not known or gone).
    std::fs::remove_file(&ps).unwrap();
    flows::run_in(&mut state, LinkAction::ShowEditorLocations);
    assert_eq!(state.dialog_name(), "EditorLocations");
    assert!(reads.load(Ordering::SeqCst) > after_first);
    assert_eq!(state.link_ui.known, [ExternalEditor::PhotoCraft], "a program that is gone is not known");
}

#[test]
fn command_for_photoshop_has_the_file_as_its_one_argument_and_spawns_nothing() {
    let exe = PathBuf::from(PS_2019);
    let stub = StubLauncher::default();
    let log = Arc::clone(&stub.log);
    let mut launcher: Box<dyn launch::Launcher> = Box::new(stub);
    for file in [r"G:\My Projects\robot arm_links\Logo_k12.psd", r"G:\Проекты\наклейка_k3.psd", r"C:\Users\Jörg Ånström\Désign_k4.psd"] {
        let file = PathBuf::from(file);
        let c = launch::command_for(&ExternalEditor::Photoshop, &exe, &file);
        assert_eq!(c.program, exe.as_os_str());
        assert_eq!(c.args, vec![file.as_os_str().to_owned()]);
        assert_eq!(c.cwd.as_deref(), Some(Path::new(r"C:\Program Files\Adobe\Adobe Photoshop CC 2019")));
        launcher.open(&ExternalEditor::Photoshop, &exe, &file).unwrap();
    }
    let calls = log.lock().unwrap().clone();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| c.editor == "photoshop" && c.exe == exe));
    assert!(launcher.is_stub());
}

// ---------------------------------------------------------------------------------------------------------------------
// The default editor and the chooser
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_chooser_asks_only_without_a_default_and_with_two_editors() {
    // Two editors known, no default: the chooser, before any file or start.
    let (mut state, stamp, dir) = two_editors("chooser");
    edit(&mut state, stamp);
    assert_eq!(question_kind(&state), Some(QuestionKind::ChooseEditor));
    let q = open_question(&state).unwrap();
    assert_eq!(q.title, "Edit in which program?");
    assert_eq!(labels(&state), ["Cancel", "Other program...", "Photoshop", "PhotoCraft"]);
    assert!(state.link_ui.chooser_always, "\"Always use this one\" starts ticked");
    assert!(!wte_core::link::links_folder(&dir.join("proj.wtep")).exists(), "a file before the answer");
    assert!(state.link_ui.launcher.calls().is_empty());
    assert_eq!(edit_in_label(&state, None), "Edit in...");
    assert_eq!(editor_name_new(&state), "your editor");
    // Photoshop with "Always use this one": the default, the link, the stub saw Photoshop's exe.
    press(&mut state, "Photoshop").unwrap();
    run_queued(&mut state);
    assert_eq!(state.prefs.default_editor.as_deref(), Some(EDITOR_PHOTOSHOP));
    let id = sticker_id(&state, stamp);
    assert_eq!(state.doc().unwrap().link(id).unwrap().editor, EDITOR_PHOTOSHOP);
    let calls = state.link_ui.launcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].editor.as_str(), calls[0].exe.as_path()), ("photoshop", state.prefs.photoshop_exe.as_deref().unwrap()));
    assert_eq!(editor_name(&state, id), "Photoshop");
    assert_eq!(edit_in_label(&state, Some(id)), "Edit in Photoshop");
    assert_eq!(edit_in_label(&state, None), "Edit in Photoshop", "the default names the main button");
    assert!(unsaved_line(&state).unwrap().starts_with("Stickers linked to Photoshop since the last save: Logo."));

    // With a default: no chooser.
    let (mut state, stamp, _dir) = two_editors("chooser_default");
    state.prefs.default_editor = Some(EDITOR_PHOTOCRAFT.to_owned());
    edit(&mut state, stamp);
    assert_eq!(state.dialog_name(), "None");
    assert_eq!(state.link_ui.launcher.calls()[0].editor, "photocraft");

    // "Always use this one" off: opened in the chosen one, no default.
    let (mut state, stamp, _dir) = two_editors("chooser_once");
    edit(&mut state, stamp);
    state.link_ui.chooser_always = false;
    press(&mut state, "PhotoCraft").unwrap();
    run_queued(&mut state);
    assert_eq!(state.prefs.default_editor, None);
    assert_eq!(state.link_ui.launcher.calls()[0].editor, "photocraft");

    // One editor known: used and made the default without a question (D1).
    let (mut state, stamp, _dir) = linked_state("chooser_one");
    edit(&mut state, stamp);
    assert_eq!(state.dialog_name(), "None");
    assert_eq!(state.prefs.default_editor.as_deref(), Some(EDITOR_PHOTOCRAFT));

    // Cancel: nothing.
    let (mut state, stamp, _dir) = two_editors("chooser_cancel");
    edit(&mut state, stamp);
    press(&mut state, "Cancel").unwrap();
    run_queued(&mut state);
    assert!(state.link_ui.launcher.calls().is_empty() && state.prefs.default_editor.is_none());
    assert!(sticker_of_stamp(&state, stamp).is_none(), "the image stamp was promoted without an answer");
}

#[test]
fn the_editor_list_names_the_known_editors_and_an_explicit_choice_opens_in_it() {
    let (mut state, stamp, dir) = two_editors("list");
    editor::refresh_known(&mut state);
    assert_eq!(ui::listed_editors(&state), [ExternalEditor::Photoshop, ExternalEditor::PhotoCraft]);
    let other = fake_exe(&dir, "gimp", "gimp-2.10.exe");
    flows::run_in(&mut state, LinkAction::LocateEditor { editor: EDITOR_OTHER.to_owned(), path: Some(other.clone()) });
    assert_eq!(state.prefs.other_editor_exe.as_deref(), Some(other.as_path()));
    let names: Vec<String> = ui::listed_editors(&state).iter().map(ExternalEditor::name).collect();
    assert_eq!(names, ["Photoshop", "PhotoCraft", "gimp-2.10"]);
    // Nothing known: PhotoCraft alone (it can be located).
    let (mut bare, _, _d) = linked_state("list_bare");
    bare.prefs.photocraft_exe = None;
    editor::refresh_known(&mut bare);
    assert_eq!(ui::listed_editors(&bare), [ExternalEditor::PhotoCraft]);
    assert_eq!(edit_in_label(&bare, None), "Edit in PhotoCraft");
    // "Edit in" the other program from the list: no chooser, its exe, its name.
    flows::run_in(&mut state, LinkAction::EditIn(EditRequest::in_editor(LinkTarget::Stamp(stamp), EDITOR_OTHER)));
    run_queued(&mut state);
    let calls = state.link_ui.launcher.calls();
    assert_eq!((calls[0].editor.as_str(), calls[0].exe.as_path()), ("other", other.as_path()));
    let id = sticker_id(&state, stamp);
    assert_eq!(editor_name(&state, id), "gimp-2.10");
    assert_eq!(badge_text(&mut state, id), "linked");
    // Default: from the list.
    flows::run_in(&mut state, LinkAction::SetDefaultEditor(Some(EDITOR_PHOTOSHOP.to_owned())));
    assert_eq!(editor::default_of(&state.prefs), Some(ExternalEditor::Photoshop));
}

// ---------------------------------------------------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_photoshop_link_with_a_running_child_asks_nothing_a_photocraft_one_does() {
    for (key, asks) in [(EDITOR_PHOTOSHOP, false), (EDITOR_PHOTOCRAFT, true), (EDITOR_OTHER, true)] {
        let (mut state, stamp, dir) = two_editors(&format!("guard_{key}"));
        state.prefs.other_editor_exe = Some(fake_exe(&dir, "paint", "paint.exe"));
        state.prefs.default_editor = Some(key.to_owned());
        let running = Arc::new(AtomicBool::new(true));
        state.link_ui.launcher = Box::new(StubLauncher { child_running: Some(Arc::clone(&running)), ..Default::default() });
        edit(&mut state, stamp);
        assert_eq!(state.link_ui.launcher.calls().len(), 1, "{key}");
        // The child still runs; Edit in again.
        edit(&mut state, stamp);
        if asks {
            assert_eq!(question_kind(&state), Some(QuestionKind::AlreadyOpen), "{key}");
            let name = editor::name_of_key(key, state.prefs.other_editor_exe.as_deref());
            assert!(open_question(&state).unwrap().lines[0].starts_with(&format!("This sticker is already open in {name}.")));
            assert_eq!(state.link_ui.launcher.calls().len(), 1, "{key}: started before the answer");
        } else {
            assert_eq!(state.dialog_name(), "None", "{key}: Photoshop shows its open tab itself (A3)");
            assert_eq!(state.link_ui.launcher.calls().len(), 2, "{key}");
        }
        assert_eq!(flows::guards_two_copies(&editor::editor_of_key(&state.prefs, key).unwrap()), asks);
    }
}

#[test]
fn opening_a_link_in_a_second_editor_asks_first() {
    let (mut state, stamp, _dir) = two_editors("switch");
    state.prefs.default_editor = Some(EDITOR_PHOTOCRAFT.to_owned());
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    assert_eq!(state.doc().unwrap().link(id).unwrap().editor, EDITOR_PHOTOCRAFT);
    // The same link in Photoshop from the list: the question, nothing started yet.
    flows::run_in(&mut state, LinkAction::EditIn(EditRequest::in_editor(LinkTarget::Stamp(stamp), EDITOR_PHOTOSHOP)));
    run_queued(&mut state);
    assert_eq!(question_kind(&state), Some(QuestionKind::OtherEditor));
    assert_eq!(
        open_question(&state).unwrap().lines[0],
        "PhotoCraft may still have this file open. Two programs saving the same file overwrite each other."
    );
    assert_eq!(labels(&state), ["Cancel", "Open in Photoshop anyway"]);
    assert_eq!(state.link_ui.launcher.calls().len(), 1);
    press(&mut state, "Open in Photoshop anyway").unwrap();
    run_queued(&mut state);
    let calls = state.link_ui.launcher.calls();
    assert_eq!((calls.len(), calls[1].editor.as_str()), (2, "photoshop"));
    // The link follows Photoshop now: every text names it.
    assert_eq!(state.doc().unwrap().link(id).unwrap().editor, EDITOR_PHOTOSHOP);
    assert_eq!(state.link_ui.sessions[&id.into()].opened_editor.as_deref(), Some(EDITOR_PHOTOSHOP));
    assert_eq!(edit_in_label(&state, Some(id)), "Edit in Photoshop");
    // Back to PhotoCraft: asked again (now Photoshop may have it open); Cancel keeps Photoshop.
    flows::run_in(&mut state, LinkAction::EditIn(EditRequest::in_editor(LinkTarget::Stamp(stamp), EDITOR_PHOTOCRAFT)));
    run_queued(&mut state);
    assert!(open_question(&state).unwrap().lines[0].starts_with("Photoshop may still have this file open."));
    press(&mut state, "Cancel").unwrap();
    run_queued(&mut state);
    assert_eq!(state.doc().unwrap().link(id).unwrap().editor, EDITOR_PHOTOSHOP);
    // A link not opened in this session opens in another editor without the question.
    let (mut state, stamp, _dir) = two_editors("switch_closed");
    state.prefs.default_editor = Some(EDITOR_PHOTOCRAFT.to_owned());
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    state.link_ui.session(id).opened = false;
    flows::run_in(&mut state, LinkAction::EditIn(EditRequest::in_editor(LinkTarget::Stamp(stamp), EDITOR_PHOTOSHOP)));
    run_queued(&mut state);
    assert_eq!(state.dialog_name(), "None");
    assert_eq!(state.link_ui.launcher.calls().len(), 2);
}

// ---------------------------------------------------------------------------------------------------------------------
// Texts, Help > Editor locations..., prefs
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn every_link_text_names_the_links_editor() {
    let (mut state, stamp, _dir) = two_editors("texts");
    state.prefs.default_editor = Some(EDITOR_PHOTOSHOP.to_owned());
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    let q = flows::conflict_question(&state, id.into(), true);
    assert_eq!(q.lines[0], "Photoshop saved over the changes WTE sent to Logo.");
    assert!(q.find("Take Photoshop's version").is_some());
    assert_eq!(texts::send_to(&editor_name(&state, id)), "Send to Photoshop");
    // The MaxCompat sentence of a Photoshop link, word for word (13.2).
    assert_eq!(
        texts::needs_merged(EDITOR_PHOTOSHOP, &editor_name(&state, id)),
        "Photoshop saved this file without a merged picture. In Photoshop choose Edit > Preferences > File Handling, set \"Maximize PSD and PSB File Compatibility\" to Always, and save again."
    );
    // Two links of two editors: the close prompt names both.
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.3, 0.0, 0.4), glam::Vec3::Z, glam::Vec3::Y, glam::Vec2::splat(0.2), 0.6);
    let blob = state.doc_mut().unwrap().add_image_blob("b.png", Arc::from(wte_core::encode_png(4, 4, &[200; 64], wte_core::PngLevel::Fast).unwrap())).unwrap();
    let second = state.doc_mut().unwrap().add_stamp("Badge", wte_core::StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    flows::run_in(&mut state, LinkAction::EditIn(EditRequest::in_editor(LinkTarget::Stamp(second), EDITOR_PHOTOCRAFT)));
    run_queued(&mut state);
    let line = unsaved_line(&state).unwrap();
    assert!(line.starts_with("Stickers linked to Photoshop and PhotoCraft since the last save:"), "{line}");
}

#[test]
fn editor_locations_browse_clear_and_default() {
    let (mut state, _stamp, dir) = two_editors("locations");
    // Browse for Photoshop with a file of another name: taken with a warning.
    let odd = fake_exe(&dir, "ps2", "ps.exe");
    flows::run_in(&mut state, LinkAction::LocateEditor { editor: EDITOR_PHOTOSHOP.to_owned(), path: Some(odd.clone()) });
    assert_eq!(state.prefs.photoshop_exe.as_deref(), Some(odd.as_path()));
    assert!(state.toasts.visible_texts().iter().any(|t| t.contains("is not named Photoshop.exe; WTE will start it anyway")));
    assert!(state.link_ui.known.contains(&ExternalEditor::Photoshop));
    flows::run_in(&mut state, LinkAction::ClearEditor(EDITOR_PHOTOSHOP.to_owned()));
    assert_eq!(state.prefs.photoshop_exe, None);
    assert!(!state.link_ui.known.contains(&ExternalEditor::Photoshop));
    flows::run_in(&mut state, LinkAction::SetDefaultEditor(Some(EDITOR_PHOTOCRAFT.to_owned())));
    assert_eq!(state.prefs.default_editor.as_deref(), Some(EDITOR_PHOTOCRAFT));
    flows::run_in(&mut state, LinkAction::SetDefaultEditor(None));
    assert_eq!(state.prefs.default_editor, None);
    // A dialog request without a path needs the app (the file dialog): dropped here.
    flows::run_in(&mut state, LinkAction::LocateEditor { editor: EDITOR_PHOTOCRAFT.to_owned(), path: None });
    assert!(state.prefs.photocraft_exe.is_some());
}

#[test]
fn an_old_prefs_file_loads_and_the_editor_prefs_round_trip() {
    use crate::prefs::{from_json, to_json};
    // A P9a prefs file: only PhotoCraft's program.
    let old = from_json(r#"{ "recent": [], "photocraft_exe": "C:\\pc\\photocraft.exe", "keep_backup": true }"#).unwrap();
    assert_eq!(old.photocraft_exe.as_deref(), Some(Path::new(r"C:\pc\photocraft.exe")));
    assert!(old.photoshop_exe.is_none() && old.other_editor_exe.is_none() && old.default_editor.is_none() && old.keep_backup);
    // Older still: none of them.
    let older = from_json(r#"{ "recent": ["x.wtep"] }"#).unwrap();
    assert!(older.photocraft_exe.is_none() && older.default_editor.is_none());
    // Not written when unset.
    assert!(!to_json(&crate::state::Prefs::default()).contains("editor"));
    let prefs = crate::state::Prefs {
        photoshop_exe: Some(PS_2019.into()),
        other_editor_exe: Some(r"C:\gimp\gimp-2.10.exe".into()),
        default_editor: Some(EDITOR_PHOTOSHOP.to_owned()),
        ..old.clone()
    };
    assert_eq!(from_json(&to_json(&prefs)).unwrap(), prefs);
    // A default that does not read is "not chosen"; the rest of the file is kept.
    for odd in [r#"{ "default_editor": "gimp", "keep_backup": true }"#, r#"{ "default_editor": 5, "keep_backup": true }"#] {
        let p = from_json(odd).unwrap();
        assert!(p.default_editor.is_none() && p.keep_backup, "{odd}");
    }
    assert_eq!(from_json(r#"{ "default_editor": "other" }"#).unwrap().default_editor.as_deref(), Some("other"));
    // "other" without its program is no default at all.
    assert_eq!(editor::default_of(&from_json(r#"{ "default_editor": "other" }"#).unwrap()), None);
}

// ---------------------------------------------------------------------------------------------------------------------
// A file the editor keeps locked (13.1, A8)
// ---------------------------------------------------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn a_locked_file_gives_the_could_not_be_replaced_message_and_changes_nothing() {
    // Check-P9b: also for another program, whose message names its exe's stem (it read "program").
    for (key, name) in [(EDITOR_PHOTOSHOP, "Photoshop"), (EDITOR_OTHER, "gimp-2.10")] {
        locked_file_case(key, name);
    }
}

#[cfg(windows)]
fn locked_file_case(key: &str, name: &str) {
    use std::os::windows::fs::OpenOptionsExt;
    let (mut state, stamp, dir) = two_editors(&format!("locked_{key}"));
    state.prefs.other_editor_exe = Some(fake_exe(&dir, "gimp", "gimp-2.10.exe"));
    state.prefs.default_editor = Some(key.to_owned());
    edit(&mut state, stamp);
    let id = sticker_id(&state, stamp);
    let path = link_path(&state, id).unwrap();
    let before = std::fs::read(&path).unwrap();
    state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |d| d.layers[0].opacity = 0.5, "Opacity", Commit::Final).unwrap();
    assert_eq!(badge_text(&mut state, id), "changed in WTE");
    // The editor holds the file without sharing (what A8 fears).
    let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
    flows::run_in(&mut state, LinkAction::Send(SendRequest { anyway: true, ..SendRequest::new(id) }));
    let q = open_question(&state).expect("a message");
    let file = path.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(q.kind, QuestionKind::Info);
    assert_eq!(
        q.lines[0],
        format!("{file} could not be replaced because another program has it open. Close it in {name} (or save it there), then choose Send to {name} again.")
    );
    drop(lock);
    // Nothing changed: the file, no .bak.psd, the badge, the stale flag, no temp file.
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!wte_core::link::file::backup_path(&path).exists());
    assert_eq!(badge_text(&mut state, id), "changed in WTE");
    assert!(!state.link_ui.sessions[&id.into()].stale_in_editor);
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".wte-tmp")).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// Import PSD (5.8, 13.2: a file imported without a link counts as "not PhotoCraft")
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn an_unlinked_import_of_a_smart_object_is_merged_whatever_the_default_editor() {
    let smart = Path::new(env!("CARGO_MANIFEST_DIR")).join("../wte_core/tests/fixtures/psd/photoshop/layers/smartobject-layer.psd");
    for default in [EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP] {
        let (mut state, _stamp, _dir) = two_editors(&format!("import_{default}"));
        state.prefs.default_editor = Some(default.to_owned());
        flows::import_psd(&mut state, &smart, false);
        let crate::state::StampToolSource::Sticker(id) = state.tools.stamp.source else { panic!("not imported") };
        let d = state.doc().unwrap().sticker(id).unwrap();
        assert_eq!(d.layers.len(), 1, "{default}");
        let toasts = state.toasts.visible_texts();
        assert!(toasts.iter().any(|t| t.starts_with("This sticker now uses smart objects")), "{default}: {toasts:?}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The scenario's helper: Photoshop's own layer bytes added to a file WTE wrote
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_foreign_layer_spliced_into_a_wte_file_reads_back_with_its_tagged_blocks() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../wte_core/tests/fixtures/psd/photoshop");
    let donor = std::fs::read(fixtures.join("layers/type-layer.psd")).unwrap();
    let base = wte_core::psd::write_sticker_to_vec(&wte_core::psd::StickerPsd {
        size: [32, 32],
        layers: vec![super::testkit::rect_layer("base", 3, wte_core::IRect::from_xywh(2, 2, 20, 12), [10, 200, 30, 255])],
        id_seed: 3,
    })
    .unwrap();
    let out = super::testkit::with_foreign_layer(&base, &donor, 0, 40).unwrap();
    let limits = wte_core::psd::PsdLimits::default();
    let read = wte_core::psd::read(&out, &limits).unwrap();
    let original = wte_core::psd::read(&base, &limits).unwrap();
    let text = wte_core::psd::read(&donor, &limits).unwrap();
    assert_eq!(read.layers.len(), 2);
    assert_eq!(read.layers[0], original.layers[0], "the base layer is untouched");
    assert_eq!((read.layers[1].name.as_str(), read.layers[1].lyid), ("A", Some(40)));
    assert!(read.layers[1].features.contains(wte_core::psd::PsdFeatures::TEXT));
    assert_eq!(read.layers[1].pixels, text.layers[0].pixels, "the type layer's pixels");
    assert_eq!(read.merged, original.merged, "the base's merged picture stays");
    // Not a version 1 PSD, or no such layer: refused.
    assert!(super::testkit::with_foreign_layer(&base, &donor, 3, 40).is_err());
    assert!(super::testkit::with_foreign_layer(b"nope", &donor, 0, 40).is_err());
}
