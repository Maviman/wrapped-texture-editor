// Stickers P9a (STICKER_PLAN 7 "P9", 8.2; ARCHITECTURE.md 8.11, 8.16): the editor link in the real window, on a project
// saved into the run folder, with the stub launcher (nothing is started: the stub records the call) and files the
// scenario writes itself the way an editor's save lands (`link::testkit`: the sticker writer, temp file + rename).
//
// - `sticker_link`: (1) two image stamps of one picture, "Edit in PhotoCraft" -> the shared-picture question, "Only
//   this one" -> the PSD in `<run>_links\`, the stub saw it, the other stamp is still an image stamp; (2) the editor
//   adds a layer -> one undo step "Update Linked Sticker"; (3) undo, 3 s: nothing comes back by itself, the badge says
//   "changed in WTE", the watcher asked for no repaint (the idle check); "Update now" applies it again; (4) a project
//   save leaves the PSD's bytes and time alone and archives it; (5) in the Sticker view with a selection and a focused
//   text field, a file without the active layer waits, and is applied when the field lets go: the selection is gone,
//   the top layer is active, "Updated from PhotoCraft"; (6) a tray sticker from PhotoCraft's own file (fixture f1w),
//   linked, then PhotoCraft's save with a text layer added (f5): the three WTE layers stay, one new picture layer;
//   then a file with a layer effect (f4) -> the merged toast, undo brings the layers back; (7) WTE and the file both
//   change -> the conflict, both answers; (8) "Send to PhotoCraft" after the launch -> the warning, Send anyway, then the
//   editor saves over it -> the "saved over" question, never a silent update; (9) save, reopen: the link is there;
//   the PSD deleted, reopen: restored; changed while closed, reopen: the question, Update.
//   Stickers P9b (STICKER_PLAN 13.9 "P9b"): then the same steps 1-3 and 6 run a second time with the stub as
//   **Photoshop**: Photoshop is found through an injected registry (its App Paths key names a stub program in the run
//   folder), so two editors are known and none is the default: the main button reads "Edit in...", its click opens
//   the chooser "Edit in which program?"; Photoshop with "Always use this one" -> the shared-picture question -> the
//   link in Photoshop (the stub saw Photoshop's program); the editor list; Photoshop's save -> one update "Updated from
//   Photoshop"; undo, the idle check, "changed in WTE", Update now; a sticker from Photoshop's own shape-layer file,
//   placed and linked, then Photoshop's own type layer added to the linked file (the bytes of
//   `layers\type-layer.psd`'s layer, `testkit::with_foreign_layer`) -> the WTE layer stays, one new picture layer;
//   `effects\outside-stroke.psd` -> merged with "layer effects"; `layers\smartobject-layer.psd` -> merged with "smart
//   objects"; `layers-minimal\shape-layer.psd` (MaxCompat off) -> nothing applied, "needs a merged picture" and
//   Photoshop's sentence word for word in the Sticker tab; two undos bring the layers back.
// - `sticker_link_path`: PATH = a PSD file: imported as a tray sticker layer by layer and placed on the sphere.
// Each shot says what it must show.

use super::*;
use crate::link::{EditRequest, LinkAction, LinkTarget, QuestionKind, SendRequest, testkit};
use crate::sticker::StickerAction;
use wte_core::StickerId;

/// The PSD fixtures of the core's tests (made with PhotoCraft v0.2.0's command-line tool, P0; Photoshop's own files
/// under `photoshop\`, P5a).
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../wte_core/tests/fixtures/psd/");

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// The registry of the Photoshop pass (stickers P9b): Photoshop's App Paths key names the stub program in the run
/// folder. An injected reader: a self-test never reads the real registry.
struct StubPhotoshopRegistry(String);

impl crate::link::launch::RegistryReader for StubPhotoshopRegistry {
    fn default_value(&self, hive: crate::link::launch::Hive, view: crate::link::launch::RegView, subkey: &str) -> Option<String> {
        let wanted = hive == crate::link::launch::Hive::LocalMachine
            && view == crate::link::launch::RegView::Bits64
            && subkey == r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Photoshop.exe";
        wanted.then(|| self.0.clone())
    }
}

/// The labels of the open link question's buttons.
fn button_labels(s: &AppState) -> Vec<String> {
    crate::link::open_question(s).map(|q| q.buttons.iter().map(|b| b.label.clone()).collect()).unwrap_or_default()
}

/// The toasts include one that starts with `start`.
fn toast_starts(s: &AppState, start: &str) -> Result<(), String> {
    let toasts = s.toasts.visible_labels();
    if toasts.iter().any(|t| t.starts_with(start)) { Ok(()) } else { Err(format!("no toast \"{start}...\": {toasts:?}")) }
}

/// Photoshop "saves" the linked file of sticker `key` as the fixture `rel` (its own bytes).
fn photoshop_saves(s: &AppState, sc: &Scratch, key: &str, rel: &str) -> Result<(), String> {
    let bytes = std::fs::read(fixture(rel)).map_err(|e| e.to_string())?;
    testkit::save_bytes(&link_file(s, sticker_key(sc, key)?)?, &bytes)
}

fn project(sc: &Scratch) -> PathBuf {
    with_suffix(&sc.out_stem, ".wtep")
}

fn sticker_key(sc: &Scratch, key: &str) -> Result<StickerId, String> {
    Ok(StickerId(remembered(sc, key)?))
}

fn stamp_key(s: &AppState, sc: &Scratch, key: &str) -> Result<StampId, String> {
    let id = StampId(remembered(sc, key)?);
    doc_of(s)?.stamp(id).ok_or_else(|| format!("the stamp \"{key}\" is gone"))?;
    Ok(id)
}

fn link_file(s: &AppState, id: StickerId) -> Result<PathBuf, String> {
    crate::link::link_path(s, id).ok_or_else(|| "the sticker has no link file".to_owned())
}

fn layer_names(s: &AppState, id: StickerId) -> Result<Vec<String>, String> {
    Ok(doc_of(s)?.sticker(id).ok_or("no sticker")?.layers.iter().map(|l| l.name.clone()).collect())
}

/// The badge of sticker `key` is `want` (a Setup: the badge needs the document mutably for its hash cache).
fn badge_is(s: &mut AppState, sc: &Scratch, key: &str, want: &str) -> Result<(), String> {
    let id = sticker_key(sc, key)?;
    let got = crate::link::badge(s, id).map(|(_, t)| t).ok_or("not linked")?;
    if got == want { Ok(()) } else { Err(format!("the badge reads \"{got}\", expected \"{want}\"")) }
}

/// The open link question is of this kind.
fn question_is(s: &AppState, kind: QuestionKind) -> Result<(), String> {
    match crate::link::open_question(s) {
        Some(q) if q.kind == kind => Ok(()),
        Some(q) => Err(format!("the question is {:?}: {:?}", q.kind, q.lines)),
        None => Err("no link question is open".into()),
    }
}

/// Remembers the update counter, the history length and the composite.
fn remember_link(s: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    sc.values.insert("applied", s.link_ui.counters.applied);
    sc.values.insert("composite", composite_hash(doc_of(s)?));
    remember_undo(s, sc)
}

/// The editor saved the file of sticker `key` with one more layer `name`.
fn editor_adds(s: &AppState, sc: &Scratch, key: &str, name: &str) -> Result<(), String> {
    let path = link_file(s, sticker_key(sc, key)?)?;
    let psd = testkit::with_new_layer(&path, name, 40, wte_core::IRect::from_xywh(40, 30, 120, 60))?;
    testkit::save_as_editor(&path, &psd)
}

/// One more update arrived since `remember_link`.
fn one_more_update(s: &AppState, sc: &Scratch) -> Result<bool, String> {
    Ok(s.link_ui.counters.applied == remembered(sc, "applied")? + 1)
}

/// The steps that place the sample logo at disc point `at` and remember the stamp under `key`.
fn place_logo(at: fn(&AppState, &Scratch) -> Result<Vec<Step>, String>, remember: fn(&mut AppState, &mut Scratch) -> Result<(), String>) -> Vec<Step> {
    vec![Step::Do(AppAction::SetTool(ToolKind::Stamp)), Step::Dyn("place the logo", at), Step::WaitIdle, Step::Setup("remember the stamp", remember)]
}

fn press(s: &mut AppState, label: &str) -> Result<(), String> {
    crate::link::press(s, label)
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_link
// ---------------------------------------------------------------------------------------------------------------------

pub(super) fn sticker_link() -> Vec<Step> {
    let mut steps = vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("the stub PhotoCraft program in the run folder; the sample logo; the Sticker tab", |s, sc| {
            let exe = with_suffix(&sc.out_stem, "_stub").join("photocraft.exe");
            std::fs::create_dir_all(exe.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
            std::fs::write(&exe, b"").map_err(|e| e.to_string())?;
            s.prefs.photocraft_exe = Some(exe);
            let links = wte_core::link::links_folder(&project(sc));
            if links.exists() {
                std::fs::remove_dir_all(&links).map_err(|e| e.to_string())?;
            }
            s.upper_tab = UpperTab::Sticker;
            setup_stamp_image(s, sc)
        }),
    ];
    steps.extend(place_logo(
        |s, _| model_click(s, ViewId::Main3D, [-0.3, -0.05]),
        |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            sc.values.insert("image", id.0);
            Ok(())
        },
    ));
    steps.extend(place_logo(
        |s, _| model_click(s, ViewId::Main3D, [0.3, 0.1]),
        |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            sc.values.insert("image2", id.0);
            Ok(())
        },
    ));
    steps.extend([
        Step::Dyn("Save As into the run folder", |_, sc| Ok(vec![Step::Do(AppAction::SaveAs(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Assert("saved", |s, sc| {
            if doc_of(s)?.is_modified() {
                return Err("still modified".into());
            }
            file_non_empty(&project(sc))
        }),
        // (1) Edit in PhotoCraft on a picture two stamps show: the question comes before any file.
        Step::Dyn("select the first logo", |s, sc| Ok(vec![Step::Do(AppAction::SelectStamp(Some(stamp_key(s, sc, "image")?)))])),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitFrames(3),
        Step::Dyn("click Edit in PhotoCraft in the Sticker tab", |s, _| {
            let rect = panel_rect(s, PanelHit::LinkEditIn)?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Assert("the shared-picture question, and no file yet", |s, sc| {
            question_is(s, QuestionKind::Shared)?;
            if wte_core::link::links_folder(&project(sc)).exists() {
                return Err("the links folder exists before the answer".into());
            }
            if !s.link_ui.launcher.calls().is_empty() {
                return Err("something was started".into());
            }
            Ok(())
        }),
        Step::Shot {
            name: "link_shared",
            expect: "the question \"Shared picture\" / \"This picture is used by 2 stickers.\" with Cancel, Only this one and \
                     Edit all over the sphere with two logos",
        },
        Step::Setup("Only this one", |s, sc| {
            remember_undo(s, sc)?;
            press(s, "Only this one")
        }),
        Step::WaitIdle,
        Step::Setup("the link: the PSD in the links folder reads back; the stub saw it; the other stamp is still an image", |s, sc| {
            let first = stamp_key(s, sc, "image")?;
            let id = crate::link::sticker_of_stamp(s, first).ok_or("the first stamp is not a sticker stamp")?;
            sc.values.insert("sticker", id.0);
            let path = link_file(s, id)?;
            if path.parent() != Some(wte_core::link::links_folder(&project(sc)).as_path()) {
                return Err(format!("the file is at {}", path.display()));
            }
            let read = testkit::read(&path)?;
            if read.layers.len() != 1 {
                return Err(format!("{} layers in the file", read.layers.len()));
            }
            let calls = s.link_ui.launcher.calls();
            if calls.len() != 1 || calls[0].file != path || calls[0].editor != "photocraft" {
                return Err(format!("stub calls {calls:?}"));
            }
            let second = stamp_key(s, sc, "image2")?;
            if !matches!(doc_of(s)?.stamp(second).ok_or("no stamp")?.source, StampSource::Image { .. }) {
                return Err("the second stamp changed".into());
            }
            undo_since(s, sc, 1)?;
            badge_is(s, sc, "sticker", "linked")
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "link_made",
            expect: "Sticker tab: the selected logo stamp (Sticker, 1 layer) with \"Link: linked\", \"Editor: PhotoCraft\", the \
                     file name Logo_k<id>.psd and the buttons Edit in PhotoCraft, Edit in..., Update now, Send to \
                     PhotoCraft, Show file in Explorer, Unlink",
        },
        // (2) The editor adds a red layer and saves: one update.
        Step::Setup("remember; the editor saves a red layer", |s, sc| {
            remember_link(s, sc)?;
            editor_adds(s, sc, "sticker", "red bar")
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("one undo step \"Update Linked Sticker\"; two layers; the model changed", |s, sc| {
            if doc_of(s)?.undo_label() != Some("Update Linked Sticker") {
                return Err(format!("the last entry is {:?}", doc_of(s)?.undo_label()));
            }
            undo_since(s, sc, 1)?;
            let names = layer_names(s, sticker_key(sc, "sticker")?)?;
            if names.len() != 2 || names[1] != "red bar" {
                return Err(format!("layers {names:?}"));
            }
            changed_since(s, sc, "composite")
        }),
        Step::Shot {
            name: "link_updated",
            expect: "3D view: the left logo now has a red bar over its upper part (the editor's new layer); the right logo \
                     unchanged; Sticker tab: \"Sticker, 2 layers\", \"Link: linked\"; toast \"Updated from PhotoCraft\"",
        },
        // (3) Undo; nothing comes back by itself; the idle check; Update now.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Setup("remember after the undo, and the watcher's repaint requests", |s, sc| {
            remember_link(s, sc)?;
            sc.values.insert("repaints", s.link_ui.repaint_count());
            Ok(())
        }),
        Step::WaitMs(3000),
        Step::Assert("3 s later: no second update, the same picture, no repaint asked by the watcher", |s, sc| {
            undo_since(s, sc, 0)?;
            if s.link_ui.counters.applied != remembered(sc, "applied")? {
                return Err("an update arrived by itself".into());
            }
            if composite_hash(doc_of(s)?) != remembered(sc, "composite")? {
                return Err("the model changed".into());
            }
            let r = s.link_ui.repaint_count();
            if r != remembered(sc, "repaints")? { Err(format!("the watcher asked for {r} repaints")) } else { Ok(()) }
        }),
        Step::Setup("the badge reads exactly \"changed in WTE\"", |s, sc| badge_is(s, sc, "sticker", "changed in WTE")),
        Step::Dyn("Update now", |_, sc| Ok(vec![Step::Do(AppAction::Link(LinkAction::UpdateNow(sticker_key(sc, "sticker")?)))])),
        Step::WaitIdle,
        Step::Setup("applied again: two layers, linked", |s, sc| {
            if !one_more_update(s, sc)? || layer_names(s, sticker_key(sc, "sticker")?)?.len() != 2 {
                return Err("Update now did not apply the file".into());
            }
            badge_is(s, sc, "sticker", "linked")
        }),
        // (4) A project save writes no link file and archives it.
        Step::Setup("remember the PSD's bytes and time", |s, sc| {
            let path = link_file(s, sticker_key(sc, "sticker")?)?;
            let (stamp, _) = FileStampOf::read(&path)?;
            sc.values.insert("psd_crc", stamp.crc32 as u64);
            sc.values.insert("psd_time", stamp.modified_ns.unwrap_or(0));
            Ok(())
        }),
        Step::Do(AppAction::Save),
        Step::WaitIdle,
        Step::Assert("the PSD is untouched; the project holds its copy", |s, sc| {
            let id = sticker_key(sc, "sticker")?;
            let (stamp, _) = FileStampOf::read(&link_file(s, id)?)?;
            if stamp.crc32 as u64 != remembered(sc, "psd_crc")? || stamp.modified_ns.unwrap_or(0) != remembered(sc, "psd_time")? {
                return Err("the save rewrote the link file".into());
            }
            if doc_of(s)?.is_modified() {
                return Err("still modified".into());
            }
            wte_core::project::read_link_archive(&project(sc), &wte_core::link::archive_entry(id)).map(|_| ()).map_err(|e| e.to_string())
        }),
        // check-P9a (the checker's "delete the links folder in mid-run"): the whole links folder goes away while WTE
        // runs; the watcher reports it, the badge says "file missing" and offers Restore from project, which writes the
        // archived copy back; nothing is applied, nothing asks.
        Step::Setup("delete the whole links folder", |s, sc| {
            remember_link(s, sc)?;
            std::fs::remove_dir_all(wte_core::link::links_folder(&project(sc))).map_err(|e| e.to_string())
        }),
        Step::Until("the watcher reports the file missing", |s, sc| {
            let id = sticker_key(sc, "sticker")?;
            Ok(s.link_ui.sessions.get(&id).is_some_and(|x| x.observed == Some(None)))
        }),
        Step::Setup("the badge says file missing", |s, sc| badge_is(s, sc, "sticker", "file missing")),
        Step::WaitFrames(3),
        Step::Dyn("click Restore from project", |s, _| {
            let rect = panel_rect(s, PanelHit::LinkRestore)?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::WaitMs(1500),
        Step::Setup("restored: the same bytes, linked, nothing applied, no question, no new entry", |s, sc| {
            let id = sticker_key(sc, "sticker")?;
            let (stamp, _) = FileStampOf::read(&link_file(s, id)?)?;
            if stamp.crc32 as u64 != remembered(sc, "psd_crc")? {
                return Err("the restored file differs".into());
            }
            if s.link_ui.counters.applied != remembered(sc, "applied")? || crate::link::open_question(s).is_some() {
                return Err("the restore was taken as a change of the file".into());
            }
            undo_since(s, sc, 0)?;
            badge_is(s, sc, "sticker", "linked")
        }),
        // (5) In the Sticker view: a selection, the bottom layer active, a focused text field; the update waits.
        Step::Dyn("open the Sticker view on the linked stamp", |s, sc| {
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::Enter(Some(stamp_key(s, sc, "image")?))))])
        }),
        Step::WaitIdle,
        Step::Dyn("select an area; make the bottom layer active", |s, sc| {
            let bottom = doc_of(s)?.sticker(sticker_key(sc, "sticker")?).ok_or("no sticker")?.layers[0].id;
            Ok(vec![
                Step::Do(AppAction::Sticker(StickerAction::SelectRect([10.0, 10.0, 120.0, 90.0]))),
                Step::Do(AppAction::Sticker(StickerAction::SelectLayer(bottom))),
            ])
        }),
        Step::WaitFrames(3),
        Step::Dyn("click into the sticker's name field (a focused text field)", |s, _| {
            let rect = panel_rect(s, PanelHit::StickerNameField)?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitFrames(3),
        Step::Setup("remember; the editor saves a version without the active (bottom) layer", |s, sc| {
            if !s.typing_prev {
                return Err("the name field has no focus".into());
            }
            remember_link(s, sc)?;
            let path = link_file(s, sticker_key(sc, "sticker")?)?;
            let mut psd = testkit::psd_of(&testkit::read(&path)?);
            psd.layers.remove(0);
            testkit::save_as_editor(&path, &psd)
        }),
        Step::WaitMs(2500),
        Step::Assert("held while the field has the focus: nothing applied, the selection still there", |s, sc| {
            if s.link_ui.counters.applied != remembered(sc, "applied")? {
                return Err("applied while a text field had the focus".into());
            }
            let e = super::sticker::edit(s)?;
            if !e.held_update || e.selection.is_none() {
                return Err(format!("held {} selection {}", e.held_update, e.selection.is_some()));
            }
            Ok(())
        }),
        Step::Click { view: ViewId::Main3D.into(), at: [0.97, 0.97], button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE },
        Step::Until("the field let go: the update is applied", one_more_update),
        Step::WaitIdle,
        Step::Assert("applied: one layer (the red bar), the selection gone, the top layer active, the toast", |s, sc| {
            let id = sticker_key(sc, "sticker")?;
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            if d.layers.len() != 1 || d.layers[0].name != "red bar" {
                return Err(format!("layers {:?}", layer_names(s, id)?));
            }
            let e = super::sticker::edit(s)?;
            if e.selection.is_some() || e.held_update || e.active_layer != Some(d.layers[0].id) {
                return Err(format!("selection {} held {} active {:?}", e.selection.is_some(), e.held_update, e.active_layer));
            }
            let toasts = s.toasts.visible_labels();
            if !toasts.iter().any(|t| t.starts_with("Updated from PhotoCraft")) {
                return Err(format!("toasts {toasts:?}"));
            }
            entry_is(s, sc, "Update Linked Sticker", 1)
        }),
        Step::Shot {
            name: "link_held_applied",
            expect: "Sticker view: the canvas shows only the red bar (the logo layer is gone), no dashed selection; the \
                     Layers tab lists one layer \"red bar\"; the 3D half shows the left logo as a red bar; the toast \
                     \"Updated from PhotoCraft\"",
        },
        Step::Do(AppAction::Sticker(StickerAction::Leave)),
        Step::WaitIdle,
        // (6) PhotoCraft's own text layer and a layer effect, on a tray sticker made from PhotoCraft's file (f1w).
        Step::Do(AppAction::Link(LinkAction::ImportPsd { path: Some(fixture("f1w_pc.psd")), keep_linked: false })),
        Step::WaitIdle,
        Step::Setup("the tray sticker of f1w (three layers); link it (Edit in on the tray sticker)", |s, sc| {
            let crate::state::StampToolSource::Sticker(id) = s.tools.stamp.source else { return Err("no tray sticker".into()) };
            sc.values.insert("tray", id.0);
            let names = layer_names(s, id)?;
            if names != ["photo", "text", "shape"] {
                return Err(format!("layers {names:?}"));
            }
            s.actions.push(AppAction::Link(LinkAction::EditIn(EditRequest::new(LinkTarget::Sticker(id)))));
            Ok(())
        }),
        Step::WaitIdle,
        Step::Setup("linked at scale 1; remember; PhotoCraft saves it with a text layer added (fixture f5)", |s, sc| {
            let id = sticker_key(sc, "tray")?;
            let link = doc_of(s)?.link(id).ok_or("not linked")?;
            if link.scale != 1.0 || link.layer_map.len() != 3 {
                return Err(format!("scale {} map {}", link.scale, link.layer_map.len()));
            }
            let ids: Vec<u64> = doc_of(s)?.sticker(id).ok_or("no sticker")?.layers.iter().map(|l| l.id.0 as u64).collect();
            sc.values.insert("tray_layers", wte_core::fnv1a64(0, format!("{ids:?}").as_bytes()));
            remember_link(s, sc)?;
            let bytes = std::fs::read(fixture("f5_f1_plus_text.psd")).map_err(|e| e.to_string())?;
            testkit::save_bytes(&link_file(s, id)?, &bytes)
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("the three WTE layers are still there and one new picture layer \"pc text\" is on top", |s, sc| {
            let id = sticker_key(sc, "tray")?;
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            let ids: Vec<u64> = d.layers[..3.min(d.layers.len())].iter().map(|l| l.id.0 as u64).collect();
            if d.layers.len() != 4 || wte_core::fnv1a64(0, format!("{ids:?}").as_bytes()) != remembered(sc, "tray_layers")? {
                return Err(format!("layers {:?}", layer_names(s, id)?));
            }
            if d.layers[3].name != "pc text" || !matches!(d.layers[3].content, wte_core::StickerContent::Picture(_)) {
                return Err(format!("the top layer {:?}", d.layers[3].name));
            }
            if !doc_of(s)?.link(id).ok_or("not linked")?.editor_native {
                return Err("the file's own text layer is not noted".into());
            }
            Ok(())
        }),
        Step::Setup("remember; PhotoCraft saves it with a layer effect (fixture f4)", |s, sc| {
            remember_link(s, sc)?;
            let bytes = std::fs::read(fixture("f4_mask_stroke.psd")).map_err(|e| e.to_string())?;
            testkit::save_bytes(&link_file(s, sticker_key(sc, "tray")?)?, &bytes)
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("one merged picture \"PhotoCraft picture\" and the toast that names the effects", |s, sc| {
            let id = sticker_key(sc, "tray")?;
            if layer_names(s, id)? != ["PhotoCraft picture"] || doc_of(s)?.link(id).ok_or("not linked")?.mode != wte_core::link::LinkMode::Merged {
                return Err(format!("layers {:?}", layer_names(s, id)?));
            }
            let toasts = s.toasts.visible_labels();
            if !toasts.iter().any(|t| t.starts_with("This sticker now uses layer effects, which only PhotoCraft can show.")) {
                return Err(format!("toasts {toasts:?}"));
            }
            entry_is(s, sc, "Update Linked Sticker", 1)
        }),
        Step::Shot {
            name: "link_merged",
            expect: "toasts \"Updated from PhotoCraft\" (counted) and \"This sticker now uses layer effects, which only \
                     PhotoCraft can show. WTE shows the finished picture; keep editing its layers in PhotoCraft.\"; the 3D \
                     view unchanged (the tray sticker is not placed): the red bar on the left, the logo on the right",
        },
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("undo brings the four layers back", |s, sc| {
            let names = layer_names(s, sticker_key(sc, "tray")?)?;
            if names.len() == 4 && names[3] == "pc text" { Ok(()) } else { Err(format!("layers {names:?}")) }
        }),
        // (7) Both changed: the conflict, both answers.
        Step::Dyn("open the Sticker view on the logo again", |s, sc| {
            Ok(vec![Step::Do(AppAction::SelectStamp(Some(stamp_key(s, sc, "image")?))), Step::Do(AppAction::Sticker(StickerAction::Enter(None)))])
        }),
        Step::WaitIdle,
        Step::Dyn("WTE: the layer's opacity 50 %", |s, sc| {
            let layer = doc_of(s)?.sticker(sticker_key(sc, "sticker")?).ok_or("no sticker")?.layers[0].id;
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::SetLayerOpacity(layer, 0.5, Commit::Final)))])
        }),
        Step::WaitIdle,
        Step::Setup("the badge says changed in WTE; remember; the editor adds a layer too", |s, sc| {
            badge_is(s, sc, "sticker", "changed in WTE")?;
            remember_link(s, sc)?;
            editor_adds(s, sc, "sticker", "editor one")
        }),
        Step::Until("the conflict question", |s, _| Ok(question_is(s, QuestionKind::Conflict { saved_over: false }).is_ok())),
        Step::Assert("nothing applied; the text names both", |s, sc| {
            undo_since(s, sc, 0)?;
            let q = crate::link::open_question(s).ok_or("no question")?;
            if q.lines[0] != "Both WTE and PhotoCraft changed Logo." { Err(format!("{:?}", q.lines)) } else { Ok(()) }
        }),
        Step::Shot {
            name: "link_conflict",
            expect: "the question \"Linked sticker changed\" / \"Both WTE and PhotoCraft changed Logo.\" with Decide later, \
                     Keep WTE's version and Take PhotoCraft's version, over the Sticker view",
        },
        Step::Setup("Take PhotoCraft's version", |s, _| press(s, "Take PhotoCraft's version")),
        Step::WaitIdle,
        Step::Setup("the file's two layers; WTE's opacity is one undo step away; linked", |s, sc| {
            let id = sticker_key(sc, "sticker")?;
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            if d.layers.len() != 2 || d.layers[0].opacity != 1.0 {
                return Err(format!("layers {:?} opacity {}", layer_names(s, id)?, d.layers[0].opacity));
            }
            undo_since(s, sc, 1)?;
            badge_is(s, sc, "sticker", "linked")
        }),
        Step::Dyn("WTE: opacity 50 % again", |s, sc| {
            let layer = doc_of(s)?.sticker(sticker_key(sc, "sticker")?).ok_or("no sticker")?.layers[0].id;
            Ok(vec![Step::Do(AppAction::Sticker(StickerAction::SetLayerOpacity(layer, 0.5, Commit::Final)))])
        }),
        Step::WaitIdle,
        Step::Setup("the editor adds another layer", |s, sc| editor_adds(s, sc, "sticker", "editor two")),
        Step::Until("the conflict question", |s, _| Ok(question_is(s, QuestionKind::Conflict { saved_over: false }).is_ok())),
        Step::Setup("Keep WTE's version", |s, _| press(s, "Keep WTE's version")),
        Step::WaitIdle,
        Step::Assert("the warning: the file was opened in PhotoCraft in this session", |s, _| {
            question_is(s, QuestionKind::SendWarning)?;
            let q = crate::link::open_question(s).ok_or("no question")?;
            if q.lines[0].starts_with("PhotoCraft may still have this file open.") { Ok(()) } else { Err(format!("{:?}", q.lines)) }
        }),
        Step::Setup("Send anyway", |s, _| press(s, "Send anyway")),
        Step::WaitIdle,
        Step::Setup("the file holds WTE's version (opacity 128), the old one is the .bak.psd; linked", |s, sc| {
            let path = link_file(s, sticker_key(sc, "sticker")?)?;
            let read = testkit::read(&path)?;
            if read.layers.len() != 2 || read.layers[0].opacity != 128 {
                return Err(format!("{} layers, opacity {}", read.layers.len(), read.layers.first().map_or(0, |l| l.opacity)));
            }
            file_non_empty(&wte_core::link::file::backup_path(&path))?;
            badge_is(s, sc, "sticker", "linked")
        }),
        // (8) Send to PhotoCraft again (the warning), then the editor saves its old picture: a question, never silent.
        Step::Dyn("Send to PhotoCraft", |_, sc| Ok(vec![Step::Do(AppAction::Link(LinkAction::Send(SendRequest::new(sticker_key(sc, "sticker")?))))])),
        Step::WaitIdle,
        Step::Setup("the warning again; Send anyway", |s, _| {
            question_is(s, QuestionKind::SendWarning)?;
            press(s, "Send anyway")
        }),
        Step::WaitIdle,
        Step::Setup("remember; the editor saves over it", |s, sc| {
            remember_link(s, sc)?;
            editor_adds(s, sc, "sticker", "saved over")
        }),
        Step::Until("the saved-over question", |s, _| Ok(question_is(s, QuestionKind::Conflict { saved_over: true }).is_ok())),
        Step::Assert("never applied silently; the text says so", |s, sc| {
            undo_since(s, sc, 0)?;
            if s.link_ui.counters.applied != remembered(sc, "applied")? {
                return Err("applied".into());
            }
            let q = crate::link::open_question(s).ok_or("no question")?;
            if q.lines[0] == "PhotoCraft saved over the changes WTE sent to Logo." { Ok(()) } else { Err(format!("{:?}", q.lines)) }
        }),
        Step::Shot {
            name: "link_saved_over",
            expect: "the question \"Linked sticker changed\" / \"PhotoCraft saved over the changes WTE sent to Logo.\" with \
                     the three answers",
        },
        Step::Setup("Take PhotoCraft's version", |s, _| press(s, "Take PhotoCraft's version")),
        Step::WaitIdle,
        Step::Do(AppAction::Sticker(StickerAction::Leave)),
        Step::WaitIdle,
        // (9) Save, reopen; the PSD deleted; changed while closed.
        Step::Setup("linked before the save", |s, sc| badge_is(s, sc, "sticker", "linked")),
        Step::Do(AppAction::Save),
        Step::WaitIdle,
        Step::Dyn("open the project again", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Setup("the link and its state came back", |s, sc| {
            let link = doc_of(s)?.link(sticker_key(sc, "sticker")?).ok_or("the link is gone")?;
            if link.editor != "photocraft" {
                return Err(format!("editor {}", link.editor));
            }
            badge_is(s, sc, "sticker", "linked")
        }),
        Step::Setup("delete the PSD", |s, sc| {
            let path = link_file(s, sticker_key(sc, "sticker")?)?;
            sc.values.insert("psd_crc", FileStampOf::read(&path)?.0.crc32 as u64);
            std::fs::remove_file(&path).map_err(|e| e.to_string())
        }),
        Step::Dyn("open the project again", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Setup("the app restored the PSD from the project", |s, sc| {
            let (stamp, _) = FileStampOf::read(&link_file(s, sticker_key(sc, "sticker")?)?)?;
            if stamp.crc32 as u64 != remembered(sc, "psd_crc")? {
                return Err("the restored file differs".into());
            }
            badge_is(s, sc, "sticker", "linked")
        }),
        Step::Setup("change the PSD while the project is \"closed\"", |s, sc| editor_adds(s, sc, "sticker", "while closed")),
        Step::Dyn("open the project again", |_, sc| Ok(vec![Step::Do(AppAction::OpenProject(Some(project(sc))))])),
        Step::WaitIdle,
        Step::Assert("the question about the change", |s, _| {
            question_is(s, QuestionKind::ChangedWhileClosed)?;
            let q = crate::link::open_question(s).ok_or("no question")?;
            if q.lines[0] == "Linked sticker Logo changed while the project was closed." { Ok(()) } else { Err(format!("{:?}", q.lines)) }
        }),
        Step::Shot {
            name: "link_changed_closed",
            expect: "the question \"Linked sticker changed\" / \"Linked sticker Logo changed while the project was closed.\" \
                     with Ignore and Update over the reopened sphere",
        },
        Step::Setup("remember; Update", |s, sc| {
            remember_link(s, sc)?;
            press(s, "Update")
        }),
        Step::WaitIdle,
        Step::Assert("the change is applied: the layer \"while closed\"", |s, sc| {
            let names = layer_names(s, sticker_key(sc, "sticker")?)?;
            if names.last().map(String::as_str) != Some("while closed") {
                return Err(format!("layers {names:?}"));
            }
            entry_is(s, sc, "Update Linked Sticker", 1)
        }),
        Step::Setup("the Sticker tab with the linked stamp", |s, sc| {
            let stamp = stamp_key(s, sc, "image")?;
            crate::dispatch::select_stamp(s, Some(stamp));
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "link_pc_done",
            expect: "Sticker tab: the selected logo stamp with \"Link: linked\", \"Editor: PhotoCraft\" and the buttons Edit in \
                     PhotoCraft, Edit in..., Update now, Send to PhotoCraft, Show file in Explorer, Unlink; the left logo \
                     on the sphere with its red bar",
        },
    ]);
    steps.extend(photoshop_pass());
    steps
}

/// Stickers P9b: steps 1-3 and 6 of `sticker_link` a second time with the stub as Photoshop (see the module docs).
fn photoshop_pass() -> Vec<Step> {
    let mut steps = vec![Step::Setup(
        "Photoshop known through the registry (the stub program in the run folder), PhotoCraft too, no default editor; \
         a fresh stub log; the logo as the Stamp tool's source",
        |s, sc| {
            let exe = with_suffix(&sc.out_stem, "_stub").join("Photoshop.exe");
            std::fs::write(&exe, b"").map_err(|e| e.to_string())?;
            s.link_ui.registry = Box::new(StubPhotoshopRegistry(exe.display().to_string()));
            s.link_ui.discovered = false;
            s.link_ui.launcher = Box::new(crate::link::launch::StubLauncher::default());
            s.prefs.default_editor = None;
            crate::dispatch::select_stamp(s, None);
            s.upper_tab = UpperTab::Sticker;
            setup_stamp_image(s, sc)
        },
    )];
    steps.extend(place_logo(
        |s, _| model_click(s, ViewId::Main3D, [-0.05, 0.4]),
        |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            sc.values.insert("ps1", id.0);
            Ok(())
        },
    ));
    steps.extend(place_logo(
        |s, _| model_click(s, ViewId::Main3D, [0.1, -0.4]),
        |s, sc| {
            let (id, _) = placed_stamp(s, false)?;
            sc.values.insert("ps2", id.0);
            Ok(())
        },
    ));
    steps.extend([
        // (1) Two editors known and no default: the chooser; Photoshop; the shared picture; the link.
        Step::Dyn("select the first new logo", |s, sc| Ok(vec![Step::Do(AppAction::SelectStamp(Some(stamp_key(s, sc, "ps1")?)))])),
        Step::Do(AppAction::SetTool(ToolKind::Transform)),
        Step::WaitFrames(3),
        Step::Assert("Photoshop was found (the injected App Paths key); the main button reads \"Edit in...\"", |s, sc| {
            let exe = with_suffix(&sc.out_stem, "_stub").join("Photoshop.exe");
            if s.prefs.photoshop_exe.as_deref() != Some(exe.as_path()) {
                return Err(format!("Photoshop's program {:?}", s.prefs.photoshop_exe));
            }
            let known: Vec<String> = s.link_ui.known.iter().map(|e| e.name()).collect();
            if known != ["Photoshop", "PhotoCraft"] {
                return Err(format!("known editors {known:?}"));
            }
            let label = crate::link::edit_in_label(s, None);
            if label == "Edit in..." { Ok(()) } else { Err(format!("the main button reads \"{label}\"")) }
        }),
        Step::Dyn("click Edit in... in the Sticker tab", |s, _| {
            let rect = panel_rect(s, PanelHit::LinkEditIn)?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Assert("the chooser, before any file or start", |s, _| {
            question_is(s, QuestionKind::ChooseEditor)?;
            let labels = button_labels(s);
            if labels != ["Cancel", "Other program...", "Photoshop", "PhotoCraft"] {
                return Err(format!("buttons {labels:?}"));
            }
            if !s.link_ui.chooser_always {
                return Err("\"Always use this one\" is not ticked".into());
            }
            if s.link_ui.launcher.calls().is_empty() { Ok(()) } else { Err("something was started".into()) }
        }),
        Step::Shot {
            name: "link_chooser",
            expect: "the question \"Edit in which program?\" with the ticked box \"Always use this one\" and the buttons \
                     Cancel, Other program..., Photoshop and PhotoCraft over the sphere",
        },
        Step::Setup("Photoshop, Always use this one", |s, sc| {
            remember_undo(s, sc)?;
            press(s, "Photoshop")
        }),
        Step::WaitIdle,
        Step::Setup("the shared-picture question (two stamps of one picture); Only this one", |s, _| {
            question_is(s, QuestionKind::Shared)?;
            press(s, "Only this one")
        }),
        Step::WaitIdle,
        Step::Setup("linked in Photoshop: the default, the file, the stub saw Photoshop's program, the other stamp is an image", |s, sc| {
            if s.prefs.default_editor.as_deref() != Some("photoshop") {
                return Err(format!("the default editor {:?}", s.prefs.default_editor));
            }
            let first = stamp_key(s, sc, "ps1")?;
            let id = crate::link::sticker_of_stamp(s, first).ok_or("the first stamp is not a sticker stamp")?;
            sc.values.insert("pss", id.0);
            let link = doc_of(s)?.link(id).ok_or("not linked")?;
            if link.editor != "photoshop" {
                return Err(format!("the link's editor {}", link.editor));
            }
            let path = link_file(s, id)?;
            let calls = s.link_ui.launcher.calls();
            let exe = with_suffix(&sc.out_stem, "_stub").join("Photoshop.exe");
            if calls.len() != 1 || calls[0].file != path || calls[0].editor != "photoshop" || calls[0].exe != exe {
                return Err(format!("stub calls {calls:?}"));
            }
            if !matches!(doc_of(s)?.stamp(stamp_key(s, sc, "ps2")?).ok_or("no stamp")?.source, StampSource::Image { .. }) {
                return Err("the second stamp changed".into());
            }
            undo_since(s, sc, 1)?;
            badge_is(s, sc, "pss", "linked")
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "link_made_ps",
            expect: "Sticker tab: the selected logo stamp (Sticker, 1 layer) with \"Link: linked\", \"Editor: Photoshop\", the \
                     file name Logo_k<id>.psd and the buttons Edit in Photoshop, Edit in..., Update now, Send to Photoshop, \
                     Show file in Explorer, Unlink; toast \"Opening Logo_k<id>.psd in Photoshop\"",
        },
        Step::Dyn("open the editor list (Edit in...)", |s, _| {
            let rect = panel_rect(s, PanelHit::LinkEditInList)?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitFrames(3),
        Step::Assert("the list has five items: Photoshop, PhotoCraft, Other program..., Default: Photoshop, Default: PhotoCraft", |s, _| {
            for i in 0..5 {
                panel_rect(s, PanelHit::LinkEditorItem(i))?;
            }
            Ok(())
        }),
        Step::Shot {
            name: "link_editor_list",
            expect: "the open list under Edit in...: Photoshop, PhotoCraft, Other program..., a line, then the radio items \
                     Default: Photoshop (chosen) and Default: PhotoCraft",
        },
        Step::Dyn("click Default: Photoshop (the list closes)", |s, _| {
            let rect = panel_rect(s, PanelHit::LinkEditorItem(3))?;
            Ok(vec![Step::ScreenClick { at: rect.center(), button: egui::PointerButton::Primary, mods: egui::Modifiers::NONE }])
        }),
        Step::WaitIdle,
        Step::Assert("still Photoshop; nothing started", |s, _| {
            if s.prefs.default_editor.as_deref() != Some("photoshop") || s.link_ui.launcher.calls().len() != 1 {
                return Err(format!("default {:?}, {} starts", s.prefs.default_editor, s.link_ui.launcher.calls().len()));
            }
            Ok(())
        }),
        // (2) Photoshop saves a red layer: one update.
        Step::Setup("remember; Photoshop saves a red layer", |s, sc| {
            remember_link(s, sc)?;
            editor_adds(s, sc, "pss", "red bar")
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("one undo step \"Update Linked Sticker\"; two layers; \"Updated from Photoshop\"", |s, sc| {
            entry_is(s, sc, "Update Linked Sticker", 1)?;
            let names = layer_names(s, sticker_key(sc, "pss")?)?;
            if names.len() != 2 || names[1] != "red bar" {
                return Err(format!("layers {names:?}"));
            }
            toast_starts(s, "Updated from Photoshop")
        }),
        // (3) Undo; nothing comes back by itself; the idle check; Update now.
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Setup("remember after the undo, and the watcher's repaint requests", |s, sc| {
            remember_link(s, sc)?;
            sc.values.insert("repaints", s.link_ui.repaint_count());
            Ok(())
        }),
        Step::WaitMs(3000),
        Step::Assert("3 s later: no second update, the same picture, no repaint asked by the watcher", |s, sc| {
            undo_since(s, sc, 0)?;
            if s.link_ui.counters.applied != remembered(sc, "applied")? || composite_hash(doc_of(s)?) != remembered(sc, "composite")? {
                return Err("an update arrived by itself".into());
            }
            let r = s.link_ui.repaint_count();
            if r != remembered(sc, "repaints")? { Err(format!("the watcher asked for {r} repaints")) } else { Ok(()) }
        }),
        Step::Setup("the badge reads exactly \"changed in WTE\"", |s, sc| badge_is(s, sc, "pss", "changed in WTE")),
        Step::Dyn("Update now", |_, sc| Ok(vec![Step::Do(AppAction::Link(LinkAction::UpdateNow(sticker_key(sc, "pss")?)))])),
        Step::WaitIdle,
        Step::Setup("applied again: two layers, linked", |s, sc| {
            if !one_more_update(s, sc)? || layer_names(s, sticker_key(sc, "pss")?)?.len() != 2 {
                return Err("Update now did not apply the file".into());
            }
            badge_is(s, sc, "pss", "linked")
        }),
        // (6) A sticker from Photoshop's own file, placed and linked; Photoshop's own layers and files.
        Step::Do(AppAction::Link(LinkAction::ImportPsd { path: Some(fixture("photoshop/layers/shape-layer.psd")), keep_linked: false })),
        Step::WaitIdle,
        Step::Setup("the tray sticker of Photoshop's shape-layer file (one picture layer)", |s, sc| {
            let crate::state::StampToolSource::Sticker(id) = s.tools.stamp.source else { return Err("no tray sticker".into()) };
            sc.values.insert("pstray", id.0);
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            if d.layers.len() != 1 || d.size() != [32, 32] {
                return Err(format!("layers {:?}, size {:?}", layer_names(s, id)?, d.size()));
            }
            Ok(())
        }),
        Step::Dyn("place it on the sphere", |s, _| model_click(s, ViewId::Main3D, [-0.4, 0.35])),
        Step::WaitIdle,
        Step::Setup("remember the placed stamp; link the sticker (Edit in, the default Photoshop: no question)", |s, sc| {
            let (stamp, _) = placed_stamp(s, false)?;
            sc.values.insert("pstray_stamp", stamp.0);
            s.actions.push(AppAction::Link(LinkAction::EditIn(EditRequest::new(LinkTarget::Sticker(sticker_key(sc, "pstray")?)))));
            Ok(())
        }),
        Step::WaitIdle,
        Step::Setup("linked in Photoshop at scale 1; Photoshop adds its own type layer (type-layer.psd's layer, an id above the seed)", |s, sc| {
            let id = sticker_key(sc, "pstray")?;
            if crate::link::open_question(s).is_some() {
                return Err(format!("a question: {:?}", crate::link::open_question(s).map(|q| q.lines.clone())));
            }
            let link = doc_of(s)?.link(id).ok_or("not linked")?;
            if link.editor != "photoshop" || link.scale != 1.0 || link.layer_map.len() != 1 {
                return Err(format!("editor {} scale {} map {}", link.editor, link.scale, link.layer_map.len()));
            }
            let lyid = link.next_lyid + 10;
            sc.values.insert("pstray_layer", doc_of(s)?.sticker(id).ok_or("no sticker")?.layers[0].id.0 as u64);
            remember_link(s, sc)?;
            let path = link_file(s, id)?;
            let base = std::fs::read(&path).map_err(|e| e.to_string())?;
            let donor = std::fs::read(fixture("photoshop/layers/type-layer.psd")).map_err(|e| e.to_string())?;
            testkit::save_bytes(&path, &testkit::with_foreign_layer(&base, &donor, 0, lyid)?)
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("the WTE layer is still there, and Photoshop's type layer \"A\" is a new picture layer on top", |s, sc| {
            let id = sticker_key(sc, "pstray")?;
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            if d.layers.len() != 2 || d.layers[0].id.0 as u64 != remembered(sc, "pstray_layer")? {
                return Err(format!("layers {:?}", layer_names(s, id)?));
            }
            if d.layers[1].name != "A" || !matches!(d.layers[1].content, wte_core::StickerContent::Picture(_)) {
                return Err(format!("the top layer {:?}", d.layers[1].name));
            }
            if !doc_of(s)?.link(id).ok_or("not linked")?.editor_native {
                return Err("the file's own type layer is not noted".into());
            }
            entry_is(s, sc, "Update Linked Sticker", 1)
        }),
        Step::Setup("remember; Photoshop saves its file with an outside stroke (effects\\outside-stroke.psd)", |s, sc| {
            remember_link(s, sc)?;
            photoshop_saves(s, sc, "pstray", "photoshop/effects/outside-stroke.psd")
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("one merged picture \"Photoshop picture\" and the toast that names layer effects", |s, sc| {
            let id = sticker_key(sc, "pstray")?;
            if layer_names(s, id)? != ["Photoshop picture"] || doc_of(s)?.link(id).ok_or("not linked")?.mode != wte_core::link::LinkMode::Merged {
                return Err(format!("layers {:?}", layer_names(s, id)?));
            }
            toast_starts(s, "This sticker now uses layer effects, which only Photoshop can show.")?;
            entry_is(s, sc, "Update Linked Sticker", 1)
        }),
        Step::Setup("remember; Photoshop saves a smart object (layers\\smartobject-layer.psd)", |s, sc| {
            remember_link(s, sc)?;
            photoshop_saves(s, sc, "pstray", "photoshop/layers/smartobject-layer.psd")
        }),
        Step::Until("the update arrives", one_more_update),
        Step::WaitIdle,
        Step::Assert("merged again, with the sentence that names smart objects", |s, sc| {
            let id = sticker_key(sc, "pstray")?;
            if layer_names(s, id)? != ["Photoshop picture"] {
                return Err(format!("layers {:?}", layer_names(s, id)?));
            }
            toast_starts(s, "This sticker now uses smart objects, which only Photoshop can show. WTE shows the finished picture; keep editing its layers in Photoshop.")?;
            entry_is(s, sc, "Update Linked Sticker", 1)
        }),
        Step::Shot {
            name: "link_merged_ps",
            expect: "toasts \"Updated from Photoshop\" (counted) and \"This sticker now uses smart objects, which only Photoshop \
                     can show. WTE shows the finished picture; keep editing its layers in Photoshop.\"; the selected \
                     shape-layer sticker (its frame left of or at the sphere's centre, over a logo) shows Photoshop's smart \
                     object picture (a red A), its placement unchanged since it was placed",
        },
        Step::Setup("remember; Photoshop saves a shape layer with Maximize Compatibility off (layers-minimal\\shape-layer.psd)", |s, sc| {
            remember_link(s, sc)?;
            photoshop_saves(s, sc, "pstray", "photoshop/layers-minimal/shape-layer.psd")
        }),
        Step::Until("the file is read and refused as needing a merged picture", |s, sc| {
            let link = doc_of(s)?.link(sticker_key(sc, "pstray")?).ok_or("not linked")?;
            Ok(matches!(link.problem, Some(wte_core::link::LinkProblem::NeedsMerged { .. })))
        }),
        Step::WaitIdle,
        Step::Setup("nothing applied; the badge reads \"needs a merged picture\"; the Sticker tab on the placed sticker", |s, sc| {
            undo_since(s, sc, 0)?;
            if s.link_ui.counters.applied != remembered(sc, "applied")? || layer_names(s, sticker_key(sc, "pstray")?)? != ["Photoshop picture"] {
                return Err("the file was applied".into());
            }
            badge_is(s, sc, "pstray", "needs a merged picture")?;
            let stamp = stamp_key(s, sc, "pstray_stamp")?;
            crate::dispatch::select_stamp(s, Some(stamp));
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Assert("Photoshop's sentence, word for word", |_, _| {
            let want = "Photoshop saved this file without a merged picture. In Photoshop choose Edit > Preferences > File Handling, \
                        set \"Maximize PSD and PSB File Compatibility\" to Always, and save again.";
            let got = wte_core::link::texts::needs_merged("photoshop", "Photoshop");
            if got == want { Ok(()) } else { Err(format!("the sentence reads {got:?}")) }
        }),
        Step::Shot {
            name: "link_needs_merged_ps",
            expect: "Sticker tab: the selected small sticker (Sticker, 1 layer) with \"Link: needs a merged picture\" in red, \
                     \"Editor: Photoshop\", its file name and the sentence \"Photoshop saved this file without a merged \
                     picture. In Photoshop choose Edit > Preferences > File Handling, set \"Maximize PSD and PSB File \
                     Compatibility\" to Always, and save again.\"; no command-line button (a Photoshop link)",
        },
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Do(AppAction::Undo),
        Step::WaitIdle,
        Step::Assert("two undos bring the layers back: the WTE layer and \"A\"", |s, sc| {
            let names = layer_names(s, sticker_key(sc, "pstray")?)?;
            if names.len() == 2 && names[1] == "A" { Ok(()) } else { Err(format!("layers {names:?}")) }
        }),
        Step::WaitFrames(3),
    ]);
    steps
}

/// The last entry is `label` and there are `extra` more entries than remembered.
fn entry_is(s: &AppState, sc: &Scratch, label: &str, extra: u64) -> Result<(), String> {
    let got = doc_of(s)?.undo_label();
    if got != Some(label) {
        return Err(format!("the last entry is {got:?}, expected {label:?}"));
    }
    undo_since(s, sc, extra)
}

/// `FileStamp::read` with a text error.
struct FileStampOf;

impl FileStampOf {
    fn read(path: &Path) -> Result<(wte_core::link::FileStamp, Vec<u8>), String> {
        wte_core::link::FileStamp::read(path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// sticker_link_path
// ---------------------------------------------------------------------------------------------------------------------

pub(super) fn sticker_link_path(path: Option<PathBuf>) -> Vec<Step> {
    let Some(path) = path else {
        return vec![Step::Assert("a PATH argument was given", |_, _| {
            Err("the sticker_link_path scenario needs a .psd PATH on the command line".to_owned())
        })];
    };
    if !crate::dnd::is_psd_path(&path) {
        return vec![Step::Assert("PATH is a PSD file", |_, _| Err("the PATH of sticker_link_path must be a .psd or .psb file".to_owned()))];
    }
    vec![
        Step::WaitIdle,
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::Setup("remember the history", remember_undo),
        Step::Do(AppAction::OpenPath(path)),
        Step::WaitIdle,
        Step::Assert("a tray sticker from the file, layer by layer; the Stamp tool's source; one entry", |s, sc| {
            let crate::state::StampToolSource::Sticker(id) = s.tools.stamp.source else { return Err("no sticker source".into()) };
            let d = doc_of(s)?.sticker(id).ok_or("no sticker")?;
            if d.layers.is_empty() || !d.keep {
                return Err(format!("{} layers, keep {}", d.layers.len(), d.keep));
            }
            log::info!("sticker_link_path: {} layers {:?}, {} x {}", d.layers.len(), d.layers.iter().map(|l| &l.name).collect::<Vec<_>>(), d.size()[0], d.size()[1]);
            tool_is(s, ToolKind::Stamp)?;
            entry_is(s, sc, "Add Sticker", 1)
        }),
        Step::Dyn("place it on the sphere", |s, _| model_click(s, ViewId::Main3D, [0.0, 0.0])),
        Step::WaitIdle,
        Step::Assert("a sticker stamp of it shows on the model", |s, _| {
            let session = session_of(s)?;
            let id = session.selected_stamp.ok_or("nothing placed")?;
            if !matches!(session.doc.stamp(id).ok_or("no stamp")?.source, StampSource::Sticker { .. }) {
                return Err("not a sticker stamp".into());
            }
            if session.doc.stamp_layers(id).is_empty() { Err("no layer".into()) } else { Ok(()) }
        }),
        Step::Setup("the Sticker tab", |s, _| {
            s.upper_tab = UpperTab::Sticker;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Shot {
            name: "psd_placed",
            expect: "3D view: the PSD's picture placed in the middle of the sphere; Sticker tab: the selected stamp \
                     (Sticker, N layers) with Edit in PhotoCraft, Edit in... and no link lines (not linked); toasts \"Open a model \
                     first\" (the PATH opened at start, before the sample) and \"Imported <file> as a sticker (N layers)\"",
        },
    ]
}
