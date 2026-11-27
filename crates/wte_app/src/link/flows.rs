//! The commands of the editor link (STICKER_PLAN 5.1, 5.3 app parts, 5.5, 5.7, 5.8; stickers P9a): "Edit in", "Send
//! to", "Update now", the conflict and load answers, Restore, Unlink, Show file, the command-line conversion, Import
//! PSD as sticker, and the hooks of the save, Save As and load flows.
//!
//! Stickers P12 (5.9): the same commands for a linked raster paint layer (`LinkId::Layer`); "Edit Layer in <editor>"
//! is `edit_layer_in`.
//!
//! Only three things write a link file (5.3, review B1): making the link, an explicit "Send to <editor>" (also "Keep
//! WTE's version" and "Send and open"), and restoring a missing file from the project's archived copy. A project save
//! never does. Every write goes through `link::write_link_file` with the watcher's `own_writes` registration.

use std::path::{Path, PathBuf};

use wte_core::link::{
    EDITOR_OTHER, EDITOR_PHOTOCRAFT, FileStamp, LinkFile, LinkMode, LinkState, OpenedState, StaleEvent, archive_entry,
    link_file_name, links_folder, next_stale, paint_archive_entry, paint_link_file_name, rewrite_warning, send_first_text, texts,
};
use wte_core::psd::{PsdColorMode, PsdDoc, PsdError, PsdLimits};
use wte_core::doc_links::LinkApplied;
use wte_core::{LayerId, ShareScope, StampSource, StickerId};

use super::editor::{self, ExternalEditor};
use super::launch::ChildState;
use super::watch::OwnWritesIo;
use super::{
    EditRequest, HeldEvent, LinkAction, LinkId, LinkQuestion, LinkTarget, QuestionKind, SendRequest, ask, common, editor_name,
    file_name, link_name, link_path, sticker_name, wte_changed,
};
use crate::action::AppAction;
use crate::app::WteApp;
use crate::state::{AppState, Prefs};

/// The note shown the first time a link is made in a session (5.4: the styles are not in the file).
pub fn styles_note(editor: &str) -> String {
    format!(
        "WTE's layer styles and stamp styles are not in the file: {editor} shows the artwork without them, and WTE adds them again to whatever comes back."
    )
}

/// How long the command-line conversion may run (5.7).
const CONVERT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn act(a: LinkAction) -> AppAction {
    AppAction::Link(a)
}

/// Executes a `LinkAction` (dispatch): the file dialogs here, everything else in `run_in`.
pub(crate) fn run(app: &mut WteApp, action: LinkAction, frame: &eframe::Frame) {
    match action {
        LinkAction::LocateEditor { editor, path } => {
            let title = locate_title(&editor_for_key(&app.state.prefs, &editor));
            let path = crate::dispatch::resolve_path(app, path, |prefs| crate::io_flows::pick_program(frame, prefs, &title, &editor));
            if let Some(path) = path {
                set_editor_exe(&mut app.state, &editor, path);
            }
        }
        LinkAction::PickOther(req) => {
            // "Other program..." (13.1, D5): the user's own exe, then the edit in it.
            let title = locate_title(&ExternalEditor::Other(PathBuf::new()));
            let path = crate::dispatch::resolve_path(app, None, |prefs| crate::io_flows::pick_program(frame, prefs, &title, EDITOR_OTHER));
            if let Some(path) = path {
                set_editor_exe(&mut app.state, EDITOR_OTHER, path);
                app.state.push(act(LinkAction::EditIn(EditRequest { editor: Some(EDITOR_OTHER.to_owned()), ..req })));
            }
        }
        LinkAction::ImportPsd { path, keep_linked } => {
            if app.state.session.is_none() {
                app.state.toasts.warn("Open a model first");
                return;
            }
            let path = crate::dispatch::resolve_path(app, path, |prefs| crate::io_flows::pick_psd(frame, prefs));
            if let Some(path) = path {
                import_psd(&mut app.state, &path, keep_linked);
            }
        }
        LinkAction::EditIn(req) => {
            let selftest = app.state.selftest;
            let mut pick = |prefs: &Prefs, editor: &ExternalEditor| {
                if selftest {
                    return None;
                }
                crate::io_flows::pick_program(frame, prefs, &locate_title(editor), editor.key())
            };
            edit_in(&mut app.state, req, &mut pick);
        }
        other => run_in(&mut app.state, other),
    }
}

/// "Where is photocraft.exe?", "Where is Photoshop.exe?", "Choose the program to edit stickers in" (the file
/// dialog's title).
fn locate_title(editor: &ExternalEditor) -> String {
    match editor.exe_name() {
        Some(exe) => format!("Where is {exe}?"),
        None => "Choose the program to edit stickers in".to_owned(),
    }
}

/// The editor of a key for the file dialog's title (`other` without a remembered program is still "other").
fn editor_for_key(prefs: &Prefs, key: &str) -> ExternalEditor {
    editor::editor_of_key(prefs, key).unwrap_or(if key == EDITOR_OTHER { ExternalEditor::Other(PathBuf::new()) } else { ExternalEditor::PhotoCraft })
}

/// The `LinkAction`s that need only the state.
pub(crate) fn run_in(state: &mut AppState, action: LinkAction) {
    match action {
        LinkAction::EditIn(req) => edit_in(state, req, &mut |_, _| None),
        LinkAction::Send(req) => send(state, req),
        LinkAction::UpdateNow(id) => {
            let s = state.link_ui.session(id);
            s.stale_in_editor = next_stale(s.stale_in_editor, StaleEvent::Answered);
            s.conflict_open = false;
            s.closed_change = false;
            s.ignore_crc = None;
            s.held = None;
            if let Some(ev) = read_now(state, id) {
                apply(state, id, &ev.psd, ev.stamp, false);
            }
            if let Some(w) = &state.link_ui.watcher {
                w.poke();
            }
        }
        LinkAction::Take(id) => {
            let s = state.link_ui.session(id);
            s.stale_in_editor = next_stale(s.stale_in_editor, StaleEvent::Answered);
            s.conflict_open = false;
            let held = s.held.take();
            if let Some(ev) = held.or_else(|| read_now(state, id)) {
                apply(state, id, &ev.psd, ev.stamp, false);
            }
        }
        LinkAction::Later(id) => {
            let s = state.link_ui.session(id);
            s.stale_in_editor = next_stale(s.stale_in_editor, StaleEvent::Answered);
            s.conflict_open = false;
            s.held = None;
        }
        LinkAction::UpdateClosed(id) => {
            let s = state.link_ui.session(id);
            s.closed_change = false;
            let held = s.held.take();
            if let Some(ev) = held.or_else(|| read_now(state, id)) {
                apply(state, id, &ev.psd, ev.stamp, false);
            }
        }
        LinkAction::IgnoreClosed(id) => {
            let observed = state.link_ui.sessions.get(&id).and_then(|s| s.observed).flatten();
            let crc = observed.map(|o| o.crc32).or_else(|| link_path(state, id).and_then(|p| FileStamp::read(&p).ok()).map(|(s, _)| s.crc32));
            let s = state.link_ui.session(id);
            s.closed_change = false;
            s.held = None;
            s.ignore_crc = crc;
        }
        LinkAction::Fit(id) => {
            if let Some(ev) = read_now(state, id.into()) {
                apply(state, id.into(), &ev.psd, ev.stamp, true);
            }
        }
        LinkAction::Unlink(id) => {
            let name = link_name(state, id);
            let file = file_name(state, id);
            if let Some(doc) = state.doc_mut() {
                match id {
                    LinkId::Sticker(s) => doc.unlink(s),
                    LinkId::Layer(l) => doc.unlink_layer(l),
                }
            }
            state.link_ui.sessions.remove(&id);
            state.link_ui.made_since_save.remove(&id);
            state.toasts.info(format!("{name} is no longer linked; {file} stays on disk"));
        }
        LinkAction::ShowFile(id) => {
            if let Some(path) = link_path(state, id)
                && let Err(e) = state.link_ui.launcher.show_file(&path)
            {
                state.toasts.warn(e);
            }
        }
        LinkAction::Restore(id) => restore(state, id),
        LinkAction::SaveThenEdit(req) => {
            // Dispatch runs the queue in order: the save starts (`before_save` takes the request) or it does not
            // (cancelled file dialog, a running job), and then `DropUnstartedEdit` drops what is left (check-P9a).
            state.link_ui.save_then_edit = Some(req);
            state.push(AppAction::Save);
            state.push(act(LinkAction::DropUnstartedEdit));
        }
        LinkAction::DropUnstartedEdit => {
            if state.link_ui.save_then_edit.take().is_some() {
                log::info!("the save did not start; the waiting \"Edit in\" is dropped");
            }
        }
        LinkAction::Convert(id) => start_convert(state, id),
        LinkAction::LocateEditor { editor, path: Some(path) } => set_editor_exe(state, &editor, path),
        LinkAction::ImportPsd { path: Some(path), keep_linked } => import_psd(state, &path, keep_linked),
        LinkAction::LocateEditor { path: None, .. } | LinkAction::ImportPsd { path: None, .. } | LinkAction::PickOther(_) => {
            log::debug!("a file dialog request needs the app; dropped");
        }
        LinkAction::ClearEditor(key) => {
            let name = editor_for_key(&state.prefs, &key).name();
            editor::set_exe_pref(&mut state.prefs, &key, None);
            editor::refresh_known(state);
            save_prefs(state);
            state.toasts.info(format!("{name}: no program"));
        }
        LinkAction::SetDefaultEditor(key) => {
            state.prefs.default_editor = key;
            save_prefs(state);
        }
        LinkAction::ShowEditorLocations => {
            editor::ensure_discovered(state, true);
            state.dialog = crate::state::Dialog::EditorLocations;
        }
    }
}

/// Writes the prefs (never in a self-test or a test build: rule 5 of 0.4).
fn save_prefs(state: &AppState) {
    if !state.selftest && !cfg!(test) {
        crate::prefs::save(&state.prefs);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Edit in
// ---------------------------------------------------------------------------------------------------------------------

/// "Edit in <editor>" (5.1, 5.5, 5.7): save first, find the editor, convert a text stamp, the shared-picture question
/// (before any file is written), make the link or offer to send WTE's changes, the two-copies guard, then start the
/// editor. Each question re-enters here with its answer. `pick` asks for the editor's exe (`None` in a self-test).
///
/// Stickers P9b (13.1): the editor is the one the request names, else the link's, else the default editor, else the
/// only known one (which becomes the default without a question); with two or more known and no default the chooser
/// "Edit in which program?" asks. A link opened in another editor in this session asks "<name> may still have this
/// file open..." before it opens in a second one.
pub(crate) fn edit_in(state: &mut AppState, req: EditRequest, pick: &mut dyn FnMut(&Prefs, &ExternalEditor) -> Option<PathBuf>) {
    // Stickers P11: an answer to a question the PSD handle asked comes back here; the link is made, nothing starts.
    if req.prepare_only {
        if let Some(ready) = prepare_handle(state, req) {
            state.status_hint = format!("{} is ready: drag the PSD handle to {}", ready.file_name(), ready.editor_name);
        }
        return;
    }
    if let LinkTarget::Layer(layer) = req.target {
        edit_layer_in(state, req, layer, pick);
        return;
    }
    let Some(session) = state.session.as_ref() else { return };
    let doc = &session.doc;
    let (stamp, sticker) = match req.target {
        LinkTarget::Layer(_) => return,
        LinkTarget::Stamp(s) => {
            if doc.stamp(s).is_none() {
                return;
            }
            (Some(s), super::sticker_of_stamp(state, s))
        }
        LinkTarget::Sticker(id) => {
            if doc.sticker(id).is_none() {
                return;
            }
            (None, Some(id))
        }
    };
    let existing = sticker.and_then(|id| doc.link(id).cloned());
    // A link needs a saved project (5.3, reviews A10, B8); an external link does not.
    let external = existing.as_ref().is_some_and(|l| matches!(l.file, LinkFile::External(_)));
    if session.path.is_none() && !external {
        let q = LinkQuestion::new(QuestionKind::SaveFirst, sticker.map(LinkId::from), "Save first", vec![texts::SAVE_FIRST.to_owned()])
            .cancel("Cancel")
            .button("Save...", vec![act(LinkAction::SaveThenEdit(req))]);
        ask(state, q);
        return;
    }
    editor::ensure_discovered(state, false);
    let Some(editor) = choose_editor(state, &req, sticker, existing.as_ref().map(|l| l.editor.as_str())) else { return };
    // A link opened in another editor in this session (13.1): two programs saving one file overwrite each other.
    if let Some(id) = sticker
        && existing.is_some()
        && switch_question(state, id.into(), &req, &editor)
    {
        return;
    }
    let Some(exe) = editor_exe(state, &editor, pick) else { return };
    // A text stamp becomes a layered sticker first (5.1, with a confirmation).
    if let Some(s) = stamp
        && state.doc().and_then(|d| d.stamp(s)).is_some_and(|o| matches!(o.source, StampSource::Text(_)))
    {
        let q = LinkQuestion::new(
            QuestionKind::ConvertText,
            None,
            "Convert the text",
            vec![
                format!("Convert the text stamp to a layered sticker first, so {} can open it?", editor.name()),
                "Its text stays editable in WTE as a text layer.".to_owned(),
            ],
        )
        .cancel("Cancel")
        .button("Convert and edit", vec![AppAction::Sticker(crate::sticker::StickerAction::ConvertTextStamp(s)), act(LinkAction::EditIn(req))]);
        ask(state, q);
        return;
    }
    let Some(id) = resolve_sticker(state, &req) else { return };
    let has_link = state.doc().and_then(|d| d.link(id)).is_some();
    if !has_link {
        if !make_link(state, id, &editor) {
            return;
        }
    } else if !existing_ready(state, id.into(), &req, &editor) {
        return;
    }
    open_in_editor(state, id.into(), &req, &editor, &exe);
}

/// The question before a link opened in another editor in this session is opened in `editor` (13.1): true when it
/// was asked (the edit waits for the answer).
fn switch_question(state: &mut AppState, id: LinkId, req: &EditRequest, editor: &ExternalEditor) -> bool {
    if req.switch_ok {
        return false;
    }
    let Some(old) = state.link_ui.sessions.get(&id).filter(|s| s.opened).and_then(|s| s.opened_editor.clone()) else { return false };
    if old == editor.key() {
        return false;
    }
    let old_name = editor::name_of_key(&old, state.prefs.other_editor_exe.as_deref());
    let anyway = EditRequest { editor: Some(editor.key().to_owned()), switch_ok: true, ..req.clone() };
    let q = LinkQuestion::new(QuestionKind::OtherEditor, Some(id), "Open in another program?", vec![texts::other_editor_open(&old_name)])
        .cancel("Cancel")
        .button(format!("Open in {} anyway", editor.name()), vec![act(LinkAction::EditIn(anyway))]);
    ask(state, q);
    true
}

/// True when the link's editor key and the file hold more than plain picture layers WTE wrote (the "Send first?"
/// question adds that a rewrite replaces them): a sticker in Merged mode or with the editor's own layers, a paint layer
/// whose file had editor layers.
fn replaces_editor_layers(state: &AppState, id: LinkId) -> bool {
    let Some(doc) = state.doc() else { return false };
    match id {
        LinkId::Sticker(s) => doc.link(s).is_some_and(|l| l.mode == LinkMode::Merged || l.editor_native),
        LinkId::Layer(l) => doc.layer_link(l).is_some_and(|l| l.editor_layers),
    }
}

/// An existing link before its file opens (5.5): a missing file is written again from WTE's content; when WTE holds
/// newer content (and the request is not "Open the file as it is") the question "Send WTE's changes to the file
/// first?" is asked. False when the edit stops here (a question, or a failed write).
fn existing_ready(state: &mut AppState, id: LinkId, req: &EditRequest, editor: &ExternalEditor) -> bool {
    let missing = link_path(state, id).as_deref().is_none_or(|p| !p.is_file());
    if missing {
        // The file is gone: written again from WTE's content (nothing of the editor's is lost by it).
        let file = state.doc().and_then(|d| common(d, id)).map(|l| (l.file, l.editor));
        if let Some((file, key)) = file
            && let Err(text) = write_file(state, id, file, &key)
        {
            ask(state, info(Some(id), "Could not write the file", text));
            return false;
        }
        return true;
    }
    if req.open_as_is {
        return true;
    }
    let changed = wte_changed(state, id);
    let Some(q) = send_first_text(changed, replaces_editor_layers(state, id), &editor.name()) else { return true };
    let mut as_is = req.clone();
    as_is.open_as_is = true;
    let mut then = req.clone();
    then.open_as_is = true;
    let send = SendRequest { then_open: Some(then), ..SendRequest::new(id) };
    let q = LinkQuestion::new(QuestionKind::SendFirst, Some(id), "Send WTE's changes?", vec![q])
        .cancel("Cancel")
        .button("Open the file as it is", vec![link_action(LinkAction::EditIn(as_is))])
        .button("Send and open", vec![link_action(LinkAction::Send(send))]);
    ask(state, q);
    false
}

/// The two-copies guard and the start of the editor on link `id`'s file (5.7, 13.1), then the link's editor is
/// `editor`.
fn open_in_editor(state: &mut AppState, id: LinkId, req: &EditRequest, editor: &ExternalEditor, exe: &Path) {
    // The two-copies guard (5.7, 13.1): a launched child of this editor that still runs (PhotoCraft v0.2.0 and portable
    // copies, another program); never for Photoshop (A3: opening an open file shows its tab; its first child lives as
    // long as Photoshop does). A child of another editor was asked about before.
    let s = state.link_ui.session(id);
    let same_editor = s.opened_editor.as_deref().is_none_or(|k| k == editor.key());
    let running = same_editor && s.child_state() == ChildState::Running;
    if running && !req.open_anyway && guards_two_copies(editor) {
        let mut anyway = req.clone();
        anyway.open_anyway = true;
        let what = if id.layer().is_some() { "layer" } else { "sticker" };
        let q = LinkQuestion::new(
            QuestionKind::AlreadyOpen,
            Some(id),
            "Already open",
            vec![format!("This {what} is already open in {}. A second copy could overwrite your changes.", editor.name())],
        )
        .cancel("Cancel")
        .button("Show file", vec![link_action(LinkAction::ShowFile(id))])
        .button("Open anyway", vec![link_action(LinkAction::EditIn(anyway))]);
        ask(state, q);
        return;
    }
    let Some(path) = link_path(state, id) else { return };
    let s = state.link_ui.session(id);
    let exited = s.child_state() == ChildState::Exited;
    s.stale_in_editor = next_stale(s.stale_in_editor, StaleEvent::EditAgain { child_exited: exited });
    match state.link_ui.launcher.open(editor, exe, &path) {
        Ok(child) => {
            let s = state.link_ui.session(id);
            if child.is_some() {
                s.child = child;
            }
            s.opened = true;
            s.opened_editor = Some(editor.key().to_owned());
            if let Some(doc) = state.doc_mut() {
                let _ = match id {
                    LinkId::Sticker(s) => doc.set_link_file_and_editor(s, None, Some(editor.key())),
                    LinkId::Layer(l) => doc.set_layer_link_file_and_editor(l, None, Some(editor.key())),
                };
            }
            let file = file_name(state, id);
            state.toasts.info(format!("Opening {file} in {}", editor.name()));
        }
        Err(e) => ask(state, info(Some(id), &format!("{} could not be started", editor.name()), e)),
    }
}

/// The note shown the first time a paint layer is linked in a session (5.9).
pub fn layer_note(editor: &str) -> String {
    format!("{editor} gets the layer's pixels at the texture's size; the layer's opacity and blend mode stay in WTE. Keep the canvas size in {editor}.")
}

/// Why a layer cannot be edited in an outside editor (the Layer menu and the row menu leave the command out for these;
/// this is the status line when it arrives anyway): `None` when it can.
pub(crate) fn layer_refusal(state: &AppState, layer: LayerId) -> Option<&'static str> {
    let l = state.doc()?.layer(layer)?;
    if !l.is_raster() {
        Some("Only paint layers can be edited in another program; edit a stamp's sticker instead")
    } else if l.locked {
        Some("The layer is locked: unlock it to edit it in another program")
    } else {
        None
    }
}

/// Stickers P12 (5.9, 13.9): "Edit Layer in <editor>" on a raster paint layer: save first (an `External` link needs
/// no folder, but a paint layer's link is always in the links folder), the editor (the request's, the link's, the
/// default, the only known one, else the chooser), the switch question, the editor's program, then the link's one-layer
/// PSD (made, or written again when missing, or the "Send first?" question), the two-copies guard, and the start.
/// A stamp or text layer and a locked layer are refused with a status line (the menus do not offer them).
pub(crate) fn edit_layer_in(state: &mut AppState, req: EditRequest, layer: LayerId, pick: &mut dyn FnMut(&Prefs, &ExternalEditor) -> Option<PathBuf>) {
    let Some(session) = state.session.as_ref() else { return };
    if session.doc.layer(layer).is_none() {
        return;
    }
    if let Some(why) = layer_refusal(state, layer) {
        state.status_hint = why.to_owned();
        return;
    }
    let id = LinkId::Layer(layer);
    let session = state.session.as_ref().expect("checked above");
    let existing = session.doc.layer_link(layer).cloned();
    let external = existing.as_ref().is_some_and(|l| matches!(l.file, LinkFile::External(_)));
    if session.path.is_none() && !external {
        let q = LinkQuestion::new(QuestionKind::SaveFirst, None, "Save first", vec![texts::SAVE_FIRST.to_owned()])
            .cancel("Cancel")
            .button("Save...", vec![act(LinkAction::SaveThenEdit(req))]);
        ask(state, q);
        return;
    }
    editor::ensure_discovered(state, false);
    let Some(editor) = choose_editor(state, &req, None, existing.as_ref().map(|l| l.editor.as_str())) else { return };
    if existing.is_some() && switch_question(state, id, &req, &editor) {
        return;
    }
    let Some(exe) = editor_exe(state, &editor, pick) else { return };
    match existing {
        None => {
            let name = link_name(state, id);
            let file = LinkFile::InFolder(paint_link_file_name(&name, layer));
            if let Err(text) = write_file(state, id, file, editor.key()) {
                ask(state, info(None, "Could not write the file", text));
                return;
            }
            state.link_ui.made_since_save.insert(id);
            if !state.link_ui.layer_note_shown {
                state.link_ui.layer_note_shown = true;
                state.toasts.info(layer_note(&editor.name()));
            }
        }
        Some(_) => {
            if !existing_ready(state, id, &req, &editor) {
                return;
            }
        }
    }
    open_in_editor(state, id, &req, &editor, &exe);
}

/// Stickers P11 (STICKER_PLAN 6.4, 13.5, 13.9): what the PSD handle hands to the drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HandleReady {
    /// The linked PSD.
    pub path: PathBuf,
    pub sticker: StickerId,
    /// The link's editor key and name (the default editor, 13.9).
    pub editor: String,
    pub editor_name: String,
}

impl HandleReady {
    fn file_name(&self) -> String {
        self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())
    }
}

/// The status line when the PSD handle is pressed on a text stamp.
pub(crate) const HANDLE_TEXT_HINT: &str = "Convert the text stamp to a layered sticker first (Sticker tab): then its PSD can be dragged out";

/// The editor of a link the PSD handle makes or hands out (13.9: the default editor): the default editor (or the only
/// known one), else the link's own, else the first known one, else PhotoCraft. Never a question: the handle is pressed
/// and dragged in one gesture.
fn handle_editor(state: &AppState, existing: Option<&wte_core::link::StickerLink>) -> ExternalEditor {
    editor::quiet_editor(state)
        .or_else(|| existing.and_then(|l| editor::editor_of_key(&state.prefs, &l.editor)))
        .or_else(|| state.link_ui.known.first().cloned())
        .unwrap_or(ExternalEditor::PhotoCraft)
}

/// Stickers P11 (6.4): the PSD handle's press - the link of 5.1 step 1 without starting an editor. A saved project is
/// needed (else "Save first", whose Save... comes back here), a picture several stamps show asks the shared-picture
/// question first (its answers come back here), a text stamp is refused (status line). An image stamp is promoted, a
/// new link's PSD is written into the links folder, a missing file is written again; the link's editor becomes the
/// default editor (13.9). `None` when a question was asked or nothing can be handed out: the drag does not start.
pub(crate) fn prepare_handle(state: &mut AppState, req: EditRequest) -> Option<HandleReady> {
    let LinkTarget::Stamp(stamp) = req.target else { return None };
    let session = state.session.as_ref()?;
    let object = session.doc.stamp(stamp)?;
    if matches!(object.source, StampSource::Text(_)) {
        state.status_hint = HANDLE_TEXT_HINT.to_owned();
        return None;
    }
    let sticker = super::sticker_of_stamp(state, stamp);
    let existing = sticker.and_then(|id| session.doc.link(id).cloned());
    let external = existing.as_ref().is_some_and(|l| matches!(l.file, LinkFile::External(_)));
    if session.path.is_none() && !external {
        let q = LinkQuestion::new(QuestionKind::SaveFirst, sticker.map(LinkId::from), "Save first", vec![texts::SAVE_FIRST.to_owned()])
            .cancel("Cancel")
            .button("Save...", vec![act(LinkAction::SaveThenEdit(req))]);
        ask(state, q);
        return None;
    }
    editor::ensure_discovered(state, false);
    let id = resolve_sticker(state, &req)?;
    let existing = state.doc().and_then(|d| d.link(id).cloned());
    let editor = handle_editor(state, existing.as_ref());
    match existing {
        None => {
            if !make_link(state, id, &editor) {
                return None;
            }
        }
        Some(link) => {
            if link.editor != editor.key()
                && let Some(doc) = state.doc_mut()
            {
                let _ = doc.set_link_file_and_editor(id, None, Some(editor.key()));
            }
            if link_path(state, id).as_deref().is_none_or(|p| !p.is_file()) {
                // The file is gone: written again from the sticker (WTE's content), as "Edit in" does.
                if let Err(text) = write_file(state, id.into(), link.file.clone(), editor.key()) {
                    ask(state, info(Some(id.into()), "Could not write the file", text));
                    return None;
                }
            } else if wte_changed(state, id) {
                state.status_hint = format!("The file does not have WTE's latest changes yet: Send to {} first to include them", editor.name());
            }
        }
    }
    let path = link_path(state, id)?;
    Some(HandleReady { path, sticker: id, editor: editor.key().to_owned(), editor_name: editor.name() })
}

fn link_action(a: LinkAction) -> AppAction {
    act(a)
}

/// The per-editor two-copies guard of 13.1: PhotoCraft and another program ask when their launched child still runs;
/// Photoshop never does (A3).
pub(crate) fn guards_two_copies(editor: &ExternalEditor) -> bool {
    !matches!(editor, ExternalEditor::Photoshop)
}

/// The editor of an "Edit in" (13.1, D1); `None` after the chooser was asked (or nothing can be opened). `existing`
/// is the key of the link's editor, when there is a link.
fn choose_editor(state: &mut AppState, req: &EditRequest, sticker: Option<StickerId>, existing: Option<&str>) -> Option<ExternalEditor> {
    if let Some(key) = &req.editor {
        let e = editor::editor_of_key(&state.prefs, key);
        if e.is_none() {
            state.toasts.warn("Choose the program first: Edit in... > Other program...");
        }
        return e;
    }
    if let Some(e) = existing.and_then(|key| editor::editor_of_key(&state.prefs, key)) {
        return Some(e);
    }
    if let Some(e) = editor::default_of(&state.prefs) {
        return Some(e);
    }
    let known = editor::known_editors(&state.prefs, &|p: &Path| p.is_file());
    match known.as_slice() {
        // None known: PhotoCraft, which can be located ("Where is photocraft.exe?").
        [] => Some(ExternalEditor::PhotoCraft),
        // One known: it is used and becomes the default without a question (D1).
        [one] => {
            state.prefs.default_editor = Some(one.key().to_owned());
            save_prefs(state);
            Some(one.clone())
        }
        several => {
            // check-P12: a paint layer's chooser speaks of the layer (its file is one picture layer, not the sticker's).
            let (link, line) = match req.target {
                LinkTarget::Layer(l) => (Some(LinkId::Layer(l)), "WTE opens the layer's pixels in it; Ctrl+S there updates the layer on the model."),
                _ => (sticker.map(LinkId::from), "WTE opens the sticker's layers in it; Ctrl+S there updates the sticker on the model."),
            };
            let mut q = LinkQuestion::new(QuestionKind::ChooseEditor, link, "Edit in which program?", vec![line.to_owned()])
            .cancel("Cancel")
            .choice("Other program...", EDITOR_OTHER, vec![act(LinkAction::PickOther(EditRequest { editor: None, ..req.clone() }))]);
            for e in several {
                let pick = EditRequest { editor: Some(e.key().to_owned()), ..req.clone() };
                q = q.choice(e.name(), e.key(), vec![act(LinkAction::EditIn(pick))]);
            }
            state.link_ui.chooser_always = true;
            ask(state, q);
            None
        }
    }
}

/// An OK message.
fn info(link: Option<LinkId>, title: &str, text: String) -> LinkQuestion {
    LinkQuestion::new(QuestionKind::Info, link, title, vec![text]).cancel("OK")
}

/// The sticker an edit acts on, after the shared-picture question (3.1, review B2): an image stamp is promoted (with
/// every stamp of its picture for "Edit all"), a shared sticker stamp gets its own copy for "Only this one". `None`
/// while the question is asked (or when nothing can be linked).
fn resolve_sticker(state: &mut AppState, req: &EditRequest) -> Option<StickerId> {
    let stamp = match req.target {
        LinkTarget::Sticker(id) => return Some(id),
        LinkTarget::Stamp(s) => s,
        LinkTarget::Layer(_) => return None,
    };
    let doc = state.doc()?;
    let source = doc.stamp(stamp)?.source.clone();
    let users = match &source {
        StampSource::Sticker { sticker } if doc.link(*sticker).is_some() => return Some(*sticker),
        StampSource::Sticker { sticker } => doc.sticker_users(*sticker).len(),
        StampSource::Image { blob } => doc.blob_users(*blob).len(),
        StampSource::Text(_) => return None,
    };
    let scope = if users > 1 {
        match req.share {
            Some(scope) => scope,
            None => {
                let mut this = req.clone();
                this.share = Some(ShareScope::ThisStamp);
                let mut all = req.clone();
                all.share = Some(ShareScope::AllUsers);
                let q = LinkQuestion::new(
                    QuestionKind::Shared,
                    None,
                    "Shared picture",
                    vec![
                        crate::dialogs::sticker_shared::question(users),
                        "Edit all: every stamp that shows it follows the linked file. Only this one: this stamp gets its own copy first.".to_owned(),
                    ],
                )
                .cancel("Cancel")
                .button("Only this one", vec![act(LinkAction::EditIn(this))])
                .button("Edit all", vec![act(LinkAction::EditIn(all))]);
                ask(state, q);
                return None;
            }
        }
    } else {
        ShareScope::ThisStamp
    };
    let r = match source {
        StampSource::Image { .. } => state.doc_mut()?.promote_stamp(stamp, scope),
        StampSource::Sticker { sticker } if scope == ShareScope::AllUsers => Ok(sticker),
        StampSource::Sticker { sticker } if users <= 1 => Ok(sticker),
        StampSource::Sticker { .. } => state.doc_mut()?.detach_sticker(stamp),
        StampSource::Text(_) => return None,
    };
    crate::dispatch::edited(state, r)
}

/// Makes the link of sticker `id`: its PSD in the links folder (5.3). False (after a message) when it fails.
fn make_link(state: &mut AppState, id: StickerId, editor: &ExternalEditor) -> bool {
    let name = sticker_name(state, id);
    let file = LinkFile::InFolder(link_file_name(&name, id));
    if let Err(text) = write_file(state, id.into(), file, editor.key()) {
        ask(state, info(Some(id.into()), "Could not write the file", text));
        return false;
    }
    state.link_ui.made_since_save.insert(id.into());
    if !state.link_ui.first_note_shown {
        state.link_ui.first_note_shown = true;
        state.toasts.info(styles_note(&editor.name()));
    }
    true
}

/// Writes link `id`'s file (a new link or a send): the PSD of the sticker (or the paint layer's one-layer PSD, P12),
/// atomically, registered with the watcher before the rename; the link is in sync afterwards. `Err` = the sentence
/// the user sees; nothing changed.
pub(crate) fn write_file(state: &mut AppState, id: LinkId, file: LinkFile, editor_key: &str) -> Result<FileStamp, String> {
    // The other program's texts name its exe's stem ("could not be replaced ... Close it in gimp-2.10"), check-P9b.
    let editor = super::editor::name_of_key(editor_key, state.prefs.other_editor_exe.as_deref());
    let project = state.session.as_ref().and_then(|s| s.path.clone());
    let path = file.path(project.as_deref()).ok_or_else(|| texts::SAVE_FIRST.to_owned())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{} could not be made: {e}.", dir.display()))?;
    }
    let mut io = OwnWritesIo(state.link_ui.own_writes.clone());
    let doc = state.doc_mut().ok_or("no document")?;
    let stamp = match id {
        LinkId::Sticker(sid) => {
            let prepared = doc.prepare_link_file(sid, file, editor_key).map_err(|e| e.to_string())?;
            let stamp = wte_core::link::write_link_file(&path, &prepared.bytes, &mut io).map_err(|e| e.sentence(&editor))?;
            doc.finish_link_write(prepared, stamp).map_err(|e| e.to_string())?;
            stamp
        }
        LinkId::Layer(lid) => {
            let prepared = doc.prepare_layer_link_file(lid, file, editor_key).map_err(|e| e.to_string())?;
            let stamp = wte_core::link::write_link_file(&path, &prepared.bytes, &mut io).map_err(|e| e.sentence(&editor))?;
            doc.finish_layer_link_write(prepared, stamp).map_err(|e| e.to_string())?;
            stamp
        }
    };
    let s = state.link_ui.session(id);
    s.observed = Some(Some(stamp));
    s.unreadable = None;
    s.ignore_crc = None;
    state.link_ui.counters.writes += 1;
    Ok(stamp)
}

// ---------------------------------------------------------------------------------------------------------------------
// Send, read, apply
// ---------------------------------------------------------------------------------------------------------------------

/// "Send to <editor>" (5.5): the warning when the file was opened in this session, then the write; afterwards
/// `stale_in_editor` is set when it was opened (the editor's next save is a question).
pub(crate) fn send(state: &mut AppState, req: SendRequest) {
    let id = req.link;
    let Some(link) = state.doc().and_then(|d| common(d, id)) else { return };
    let editor = editor_name(state, id);
    let s = state.link_ui.session(id);
    if req.answers_conflict {
        s.stale_in_editor = next_stale(s.stale_in_editor, StaleEvent::Answered);
        s.conflict_open = false;
        s.held = None;
    }
    let opened = if !s.opened {
        OpenedState::NotOpened
    } else if s.child_state() == ChildState::Running {
        OpenedState::ChildRunning
    } else {
        OpenedState::Unknown
    };
    if !req.anyway
        && let Some(text) = rewrite_warning(opened, &editor)
    {
        let mut anyway = req.clone();
        anyway.anyway = true;
        anyway.answers_conflict = false;
        let q = LinkQuestion::new(QuestionKind::SendWarning, Some(id), "Send to the file?", vec![text])
            .cancel("Cancel")
            .button("Send anyway", vec![act(LinkAction::Send(anyway))]);
        ask(state, q);
        return;
    }
    match write_file(state, id, link.file.clone(), &link.editor) {
        Ok(_) => {
            let s = state.link_ui.session(id);
            let was_opened = s.opened;
            s.stale_in_editor = next_stale(s.stale_in_editor, StaleEvent::Rewrote { opened: was_opened });
            let name = link_name(state, id);
            state.toasts.info(format!("Sent {name} to {editor}"));
            if let Some(then) = req.then_open {
                state.push(act(LinkAction::EditIn(then)));
            }
        }
        Err(text) => ask(state, info(Some(id), "Could not send", text)),
    }
}

/// Reads link `id`'s file now ("Update now", the answers): `None` (with a toast or the badge's problem) when it is
/// missing, half written or refused.
fn read_now(state: &mut AppState, id: LinkId) -> Option<HeldEvent> {
    let path = link_path(state, id)?;
    let file = file_name(state, id);
    let (stamp, bytes) = match FileStamp::read(&path) {
        Ok(r) => r,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                state.link_ui.session(id).observed = Some(None);
            }
            state.toasts.warn(format!("{file} could not be read: {e}"));
            return None;
        }
    };
    state.link_ui.session(id).observed = Some(Some(stamp));
    match wte_core::psd::read(&bytes, &PsdLimits::default()) {
        Ok(psd) => Some(HeldEvent { stamp, psd: Box::new(psd) }),
        Err(PsdError::Truncated) => {
            state.toasts.info(format!("{file} is still being written; try again in a moment"));
            None
        }
        Err(e) => {
            if let Some(doc) = state.doc_mut() {
                let r = match id {
                    LinkId::Sticker(s) => doc.link_file_refused(s, &e, stamp, &file),
                    LinkId::Layer(l) => doc.layer_link_file_refused(l, &e, stamp, &file),
                };
                crate::dispatch::edited(state, r);
            }
            None
        }
    }
}

/// Applies a file to link `id` (5.4, 5.6): one undo step "Update Linked Sticker" (a paint layer: "Update Linked
/// Layer", 5.9), the toast "Updated from <editor>" (and the merged reason, the profile note once). In the Sticker view
/// on that sticker the selection is dropped and the active layer re-picked. A canvas too large for a sticker asks "Fit
/// it into the sticker" / Ignore; a paint layer's file that cannot be applied (another size, no merged picture) says
/// why in a toast, since its badge shows only while the layer is active.
pub(crate) fn apply(state: &mut AppState, id: LinkId, psd: &PsdDoc, stamp: FileStamp, fit: bool) {
    let editor = editor_name(state, id);
    let file = file_name(state, id);
    let name = link_name(state, id);
    let Some(doc) = state.doc_mut() else { return };
    doc.flush_pending_edit();
    let r = match id {
        LinkId::Sticker(s) if fit => doc.apply_link_fit(s, psd, stamp, &editor, &file),
        LinkId::Sticker(s) => doc.apply_link_update(s, psd, stamp, &editor, &file),
        LinkId::Layer(l) => doc.apply_layer_link_update(l, psd, stamp, &editor, &file),
    };
    let Some(applied) = crate::dispatch::edited(state, r) else { return };
    let s = state.link_ui.session(id);
    s.held = None;
    s.observed = Some(Some(stamp));
    match applied {
        LinkApplied::Updated { toast, profile_note, log, .. } => {
            for line in &log {
                log::info!("{line}");
            }
            let note = profile_note.filter(|_| !std::mem::replace(&mut s.profile_noted, true));
            state.link_ui.counters.applied += 1;
            state.toasts.info(texts::updated(&editor));
            if let Some(t) = toast {
                state.toasts.info(t);
            }
            if let Some(n) = note {
                state.toasts.info(n);
            }
            if let LinkId::Sticker(s) = id {
                after_update_in_view(state, s);
            }
        }
        LinkApplied::Problem(LinkState::CanvasTooLarge { width, height }) if let LinkId::Sticker(s) = id => {
            let q = LinkQuestion::new(
                QuestionKind::CanvasTooLarge,
                Some(id),
                "Sticker too large",
                vec![texts::canvas_too_large(&name, width, height, &editor)],
            )
            .cancel("Ignore")
            .button("Fit it into the sticker", vec![act(LinkAction::Fit(s))]);
            ask(state, q);
        }
        LinkApplied::Problem(st) => {
            log::info!("the linked file of {id:?} was not applied: {st:?}");
            if id.layer().is_some() {
                let text = match &st {
                    LinkState::NeedsMerged => {
                        let key = state.doc().and_then(|d| common(d, id)).map(|l| l.editor).unwrap_or_default();
                        texts::needs_merged(&key, &editor)
                    }
                    other => other.badge(&editor, &name, &file),
                };
                state.toasts.warn(text);
            }
        }
    }
}

/// After an update of the sticker the Sticker view shows (5.6): the selection, a crop box, a popover and an open lasso
/// belong to the old layers; the active layer stays if it still exists, else the top one (`sticker::validate_in`).
fn after_update_in_view(state: &mut AppState, id: StickerId) {
    let shows = crate::sticker::edit_of(state).is_some_and(|e| match e.target {
        crate::state::StickerTarget::Tray(s) => s == id,
        crate::state::StickerTarget::Stamp(stamp) => super::sticker_of_stamp(state, stamp) == Some(id),
    });
    if !shows {
        return;
    }
    if let Some(e) = crate::sticker::edit_mut(state) {
        e.selection = None;
        e.held_update = false;
    }
    state.sticker_ui.popover = None;
    state.sticker_ui.lasso = None;
    state.sticker_ui.crop = None;
    state.sticker_ui.layer_box = None;
    crate::sticker::validate_in(state);
}

/// The conflict question (5.5): **Take <editor>'s version** / **Keep WTE's version** / Decide later.
pub(crate) fn conflict_question(state: &AppState, id: LinkId, saved_over: bool) -> LinkQuestion {
    let editor = editor_name(state, id);
    let name = link_name(state, id);
    let text = if saved_over { texts::saved_over(&name, &editor) } else { texts::conflict(&name, &editor) };
    let title = if id.layer().is_some() { "Linked layer changed" } else { "Linked sticker changed" };
    LinkQuestion::new(
        QuestionKind::Conflict { saved_over },
        Some(id),
        title,
        vec![text, "Taking one version keeps the other one an undo step away (WTE's) or in the .bak.psd file (the editor's).".to_owned()],
    )
    .button("Decide later", vec![act(LinkAction::Later(id))])
    .button(texts::KEEP_WTES_VERSION, vec![act(LinkAction::Send(SendRequest { answers_conflict: true, ..SendRequest::new(id) }))])
    .button(texts::take_editors_version(&editor), vec![act(LinkAction::Take(id))])
}

/// "Restore from project": the archived copy of a missing `InFolder` file is written back (5.3 "At load").
fn restore(state: &mut AppState, id: LinkId) {
    let Some(project) = state.session.as_ref().and_then(|s| s.path.clone()) else { return };
    let Some(path) = link_path(state, id) else { return };
    let file = file_name(state, id);
    match restore_from_archive(state, &project, id, &path) {
        Ok(()) => state.toasts.info(format!("Restored {file} from the project")),
        Err(e) => state.toasts.warn(format!("{file} could not be restored: {e}")),
    }
}

/// The archive entry of link `id`'s file inside the project (`links/k<id>.psd`, a paint layer's `links/l<id>.psd`).
fn archive_of(id: LinkId) -> String {
    match id {
        LinkId::Sticker(s) => archive_entry(s),
        LinkId::Layer(l) => paint_archive_entry(l),
    }
}

/// Writes the archived copy of link `id`'s file to `path`.
fn restore_from_archive(state: &mut AppState, project: &Path, id: LinkId, path: &Path) -> Result<(), String> {
    let bytes = wte_core::project::read_link_archive(project, &archive_of(id)).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut io = OwnWritesIo(state.link_ui.own_writes.clone());
    let stamp = wte_core::link::write_link_file(path, &bytes, &mut io).map_err(|e| e.to_string())?;
    state.link_ui.session(id).observed = Some(Some(stamp));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// The editor's exe, the command-line conversion
// ---------------------------------------------------------------------------------------------------------------------

/// The editor's exe (5.7, 13.1): the remembered one when it exists, else discovery (registry only; never for another
/// program), else the user's file dialog once ("Where is photocraft.exe?", "Where is Photoshop.exe?"). `None` (after a
/// hint) when none is known.
fn editor_exe(state: &mut AppState, editor: &ExternalEditor, pick: &mut dyn FnMut(&Prefs, &ExternalEditor) -> Option<PathBuf>) -> Option<PathBuf> {
    let exists = |p: &Path| p.is_file();
    let remembered = match editor {
        ExternalEditor::Other(p) => Some(p.clone()),
        _ => editor::exe_pref(&state.prefs, editor.key()).cloned(),
    };
    if let Some(p) = remembered.filter(|p| exists(p)) {
        return Some(p);
    }
    if let Some(p) = super::launch::discover(editor, &*state.link_ui.registry, &exists) {
        log::info!("{} found at {}", editor.name(), p.display());
        editor::set_exe_pref(&mut state.prefs, editor.key(), Some(p.clone()));
        editor::refresh_known(state);
        return Some(p);
    }
    match pick(&state.prefs, editor) {
        Some(p) => {
            set_editor_exe(state, editor.key(), p.clone());
            Some(p)
        }
        None => {
            state.toasts.warn(format!("{} was not found. Choose its program with Help > Editor locations...", editor.name()));
            None
        }
    }
}

/// Remembers the program of the editor with `key` (Help > Editor locations..., the first-time question, "Other
/// program..."). A file not named like the editor's exe is taken with a warning (5.7: warn, do not refuse).
pub(crate) fn set_editor_exe(state: &mut AppState, key: &str, path: PathBuf) {
    let editor = if key == EDITOR_OTHER { ExternalEditor::Other(path.clone()) } else { editor_for_key(&state.prefs, key) };
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if let Some(expected) = editor.exe_name()
        && name != expected.to_ascii_lowercase()
    {
        state.toasts.warn(format!("{} is not named {expected}; WTE will start it anyway", path.display()));
    } else {
        state.toasts.info(format!("{}: {}", editor.name(), path.display()));
    }
    editor::set_exe_pref(&mut state.prefs, editor.key(), Some(path));
    editor::refresh_known(state);
    save_prefs(state);
}

/// `photocraft-cli.exe` next to the remembered `photocraft.exe` (5.7), when it exists.
pub fn cli_exe(prefs: &Prefs) -> Option<PathBuf> {
    let exe = prefs.photocraft_exe.as_ref()?;
    let cli = exe.with_file_name("photocraft-cli.exe");
    cli.is_file().then_some(cli)
}

/// "Convert with PhotoCraft's command-line tool" (5.4, 5.7): `photocraft-cli convert <psd> <png>` next to the PSD,
/// headless (`CREATE_NO_WINDOW`, stdin null), at most 30 s; the PNG becomes a merged update (`poll_convert`).
fn start_convert(state: &mut AppState, id: StickerId) {
    let Some(cli) = cli_exe(&state.prefs) else {
        state.toasts.warn("photocraft-cli.exe was not found next to photocraft.exe");
        return;
    };
    let Some(path) = link_path(state, id) else { return };
    let Ok((stamp, _)) = FileStamp::read(&path) else {
        state.toasts.warn(format!("{} could not be read", file_name(state, id)));
        return;
    };
    let png = path.with_extension("converted.png");
    match state.link_ui.launcher.convert(&cli, &path, &png) {
        Ok(child) => {
            state.link_ui.convert = Some(super::ConvertJob { sticker: id, child, png, stamp, started: std::time::Instant::now() });
        }
        Err(e) => state.toasts.warn(e),
    }
}

/// Every frame: a finished conversion is applied as one merged picture; one that runs too long is stopped.
pub(crate) fn poll_convert(state: &mut AppState) {
    let Some(job) = state.link_ui.convert.as_mut() else { return };
    match job.child.state() {
        ChildState::Running if job.started.elapsed() < CONVERT_TIMEOUT => {}
        ChildState::Running => {
            job.child.kill();
            state.link_ui.convert = None;
            state.toasts.warn("The command-line conversion took too long and was stopped");
        }
        ChildState::Exited | ChildState::Unknown => {
            let Some(job) = state.link_ui.convert.take() else { return };
            let picture = std::fs::read(&job.png).map_err(|e| e.to_string()).and_then(|b| {
                wte_core::decode_image(&b, Some("x.png"), 8192).map(|d| d.pixels).map_err(|e| e.to_string())
            });
            let _ = std::fs::remove_file(&job.png);
            match picture {
                Ok(pixels) => {
                    let psd = PsdDoc {
                        size: [pixels.width, pixels.height],
                        depth: 8,
                        psb: false,
                        mode: PsdColorMode::Rgb,
                        has_real_merged: true,
                        merged: Some(pixels),
                        layers: Vec::new(),
                        too_large: false,
                        icc_description: None,
                    };
                    apply(state, job.sticker.into(), &psd, job.stamp, false);
                }
                Err(e) => state.toasts.warn(format!("The command-line conversion gave no picture: {e}")),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Import PSD as sticker (5.8)
// ---------------------------------------------------------------------------------------------------------------------

/// A PSD file as a new tray sticker (layer by layer, or merged), made the Stamp tool's source; `keep_linked` makes it
/// an `External` link to the file (5.8).
pub(crate) fn import_psd(state: &mut AppState, path: &Path, keep_linked: bool) {
    let file = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            state.toasts.warn(format!("Cannot open {file}: {e}"));
            return;
        }
    };
    let modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let stamp = FileStamp::of_bytes(&bytes, modified);
    let psd = match parse_psd(&bytes, &file) {
        Ok(p) => p,
        Err(text) => {
            state.toasts.warn(text);
            return;
        }
    };
    let stem = path.file_stem().map_or_else(|| "Sticker".to_owned(), |s| s.to_string_lossy().into_owned());
    if let Some((id, toast)) = psd_sticker(state, &psd, stamp, &file, &stem, keep_linked.then_some(path)) {
        psd_as_source(state, id, &file, &stem, toast);
    }
}

/// Reads a PSD for an import (Import Sticker from PSD..., a drop): the refusal sentence of 13.2 or "Cannot open".
pub(crate) fn parse_psd(bytes: &[u8], file: &str) -> Result<PsdDoc, String> {
    match wte_core::psd::read(bytes, &PsdLimits::default()) {
        Ok(p) => Ok(p),
        Err(PsdError::Unsupported(r)) => Err(r.sentence(file)),
        Err(e) => Err(format!("Cannot open {file}: {e}")),
    }
}

/// A read PSD as a new tray sticker (5.8): the layers, or one merged picture when something forces it; with
/// `keep_linked` an `External` link to that file. Returns the sticker and the merged toast, if any; a refusal is a
/// warning toast and `None`. Stickers P10a: also for a PSD dropped without a file (`keep_linked` `None`).
pub(crate) fn psd_sticker(
    state: &mut AppState,
    psd: &PsdDoc,
    stamp: FileStamp,
    file: &str,
    stem: &str,
    keep_linked: Option<&Path>,
) -> Option<(StickerId, Option<String>)> {
    // The editor a kept link follows: the default editor, else the only known one, else PhotoCraft (13.1). A file
    // imported without a link counts as "not PhotoCraft" for smart objects (13.2): with PhotoCraft as that editor the
    // key passed is "other" (the generic sentences), the name stays.
    let editor = editor::quiet_editor(state).unwrap_or(ExternalEditor::PhotoCraft);
    let key = if keep_linked.is_none() && editor.key() == EDITOR_PHOTOCRAFT { EDITOR_OTHER } else { editor.key() };
    let doc = state.doc_mut()?;
    let made = doc.add_sticker_from_psd(psd, stem, stamp, keep_linked.map(Path::to_path_buf), key, &editor.name(), file);
    match made {
        Ok(made) => {
            if made.linked {
                state.link_ui.session(made.id).observed = Some(Some(stamp));
            }
            Some((made.id, made.toast))
        }
        Err(text) => {
            state.toasts.warn(text);
            None
        }
    }
}

/// An imported PSD sticker becomes the selected tray item and the Stamp tool's source (5.8); `toast` is the merged
/// toast of the import.
pub(crate) fn psd_as_source(state: &mut AppState, id: StickerId, file: &str, stem: &str, toast: Option<String>) {
    let layers = state.doc().and_then(|d| d.sticker(id)).map_or(0, |d| d.layers.len());
    state.tools.stamp.source = crate::state::StampToolSource::Sticker(id);
    state.tools.stamp.source_name = stem.to_owned();
    state.sticker_ui.tray_selected = Some(id);
    let s = if layers == 1 { "" } else { "s" };
    state.toasts.info(format!("Imported {file} as a sticker ({layers} layer{s}); click on the model to place it"));
    if let Some(t) = toast {
        state.toasts.info(t);
    }
    state.push(AppAction::SetTool(crate::state::ToolKind::Stamp));
    state.status_hint = "Click on the model to place the sticker".to_owned();
}

// ---------------------------------------------------------------------------------------------------------------------
// Save, Save As, load
// ---------------------------------------------------------------------------------------------------------------------

/// Before a save to `new_path` (5.3 "At Save As"): when the project moves, every `InFolder` link file is copied into
/// the new links folder (the save archives from there) and the links opened in this session are remembered as
/// aliases-to-be (set when the save succeeds).
pub(crate) fn before_save(state: &mut AppState, new_path: &Path) {
    state.link_ui.pending_aliases.clear();
    // This save is the one a "Save..." of the save-first question waits for (check-P9a), and it holds the links made
    // since the last save as they are now (a link made while it runs stays in the close prompt's line).
    state.link_ui.after_save = state.link_ui.save_then_edit.take();
    state.link_ui.saving_made = state.link_ui.made_since_save.clone();
    let Some(session) = state.session.as_ref() else { return };
    let Some(old) = session.path.clone() else { return };
    if old == new_path {
        return;
    }
    let (from_dir, to_dir) = (links_folder(&old), links_folder(new_path));
    let mut aliases = Vec::new();
    let doc = &session.doc;
    let ids = doc.links().map(|(id, _)| LinkId::Sticker(id)).chain(doc.layer_links().map(|(id, _)| LinkId::Layer(id)));
    for (id, l) in ids.filter_map(|id| common(doc, id).map(|l| (id, l))) {
        let LinkFile::InFolder(name) = &l.file else { continue };
        let (src, dst) = (from_dir.join(name), to_dir.join(name));
        if !src.is_file() {
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&to_dir).and_then(|_| std::fs::copy(&src, &dst)) {
            log::warn!("{} could not be copied to {}: {e}", src.display(), dst.display());
            continue;
        }
        if state.link_ui.sessions.get(&id).is_some_and(|s| s.opened) {
            aliases.push((id, src));
        }
    }
    state.link_ui.pending_aliases = aliases;
}

/// After a successful save: the links of the saved snapshot are on disk, the close prompt has no link line any more,
/// a Save As's aliases are set (with one toast), the save's warnings are shown, and an edit that waited for the save
/// continues.
pub(crate) fn after_save(state: &mut AppState, links_revision: u64, warnings: &[String]) {
    if let Some(doc) = state.doc_mut() {
        doc.mark_links_saved(links_revision);
    }
    let saved = std::mem::take(&mut state.link_ui.saving_made);
    state.link_ui.made_since_save.retain(|id| !saved.contains(id));
    let aliases = std::mem::take(&mut state.link_ui.pending_aliases);
    if !aliases.is_empty() {
        let editor = super::editors_of(state, aliases.iter().map(|(id, _)| *id));
        let names: Vec<String> = aliases.iter().map(|(id, _)| link_name(state, *id)).collect();
        for (id, old) in aliases {
            state.link_ui.session(id).alias = Some(old);
        }
        state.toasts.info(format!(
            "{} may still be open in {editor} from the old folder. WTE keeps following them until you close WTE.",
            names.join(", ")
        ));
    }
    for w in warnings {
        state.toasts.warn(w.clone());
    }
    if let Some(req) = state.link_ui.after_save.take() {
        state.push(act(LinkAction::EditIn(req)));
    }
}

/// A save that failed or was cancelled: nothing of it continues.
pub(crate) fn save_failed(state: &mut AppState) {
    state.link_ui.pending_aliases.clear();
    state.link_ui.after_save = None;
    state.link_ui.saving_made.clear();
}

/// After a project was opened (5.3 "At load"; `project::load` only reported): a missing file of the links folder
/// is restored from its archived copy, a file that changed while the project was closed is asked about (Update /
/// Ignore), a missing external file shows "file missing".
pub(crate) fn after_load(state: &mut AppState, project: &Path, links: &[wte_core::project::LinkAtLoad]) {
    for l in links {
        found_at_load(state, project, l.sticker.into(), l.state, l.path.as_deref(), l.archive.is_some());
    }
}

/// Stickers P12: `after_load` for the linked paint layers (`LoadReport::layer_links`).
pub(crate) fn after_load_layers(state: &mut AppState, project: &Path, links: &[wte_core::project::LayerLinkAtLoad]) {
    for l in links {
        found_at_load(state, project, l.layer.into(), l.state, l.path.as_deref(), l.archive.is_some());
    }
}

/// What `after_load` does for one link that `load` found in `found` at `path`.
fn found_at_load(
    state: &mut AppState,
    project: &Path,
    id: LinkId,
    found: wte_core::project::LinkFileState,
    path: Option<&Path>,
    archived: bool,
) {
    use wte_core::project::LinkFileState;
    {
        let Some(link) = state.doc().and_then(|d| common(d, id)) else { return };
        match found {
            LinkFileState::Present => state.link_ui.session(id).observed = Some(Some(link.synced)),
            LinkFileState::Missing => {
                let restorable = matches!(link.file, LinkFile::InFolder(_)) && archived;
                match (restorable, path) {
                    (true, Some(path)) => match restore_from_archive(state, project, id, path) {
                        Ok(()) => state.toasts.info(format!("Restored {} from the project", link.file.display_name())),
                        Err(e) => {
                            log::warn!("{} could not be restored: {e}", path.display());
                            state.link_ui.session(id).observed = Some(None);
                        }
                    },
                    _ => state.link_ui.session(id).observed = Some(None),
                }
            }
            LinkFileState::Changed => {
                let stamp = path.and_then(|p| FileStamp::read(p).ok()).map(|(s, _)| s);
                let s = state.link_ui.session(id);
                s.observed = Some(stamp);
                s.closed_change = true;
                let name = link_name(state, id);
                let (title, text) = match id {
                    LinkId::Sticker(_) => ("Linked sticker changed", texts::changed_while_closed(&name)),
                    LinkId::Layer(_) => ("Linked layer changed", texts::layer_changed_while_closed(&name)),
                };
                let q = LinkQuestion::new(QuestionKind::ChangedWhileClosed, Some(id), title, vec![text])
                .button("Ignore", vec![act(LinkAction::IgnoreClosed(id))])
                .button("Update", vec![act(LinkAction::UpdateClosed(id))]);
                ask(state, q);
            }
        }
    }
}
