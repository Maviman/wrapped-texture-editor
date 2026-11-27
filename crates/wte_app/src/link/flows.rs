//! The commands of the editor link (STICKER_PLAN 5.1, 5.3 app parts, 5.5, 5.7, 5.8; stickers P9a): "Edit in", "Send
//! to", "Update now", the conflict and load answers, Restore, Unlink, Show file, the command-line conversion, Import
//! PSD as sticker, and the hooks of the save, Save As and load flows.
//!
//! Only three things write a link file (5.3, review B1): making the link, an explicit "Send to <editor>" (also "Keep
//! WTE's version" and "Send and open"), and restoring a missing file from the project's archived copy. A project save
//! never does. Every write goes through `link::write_link_file` with the watcher's `own_writes` registration.

use std::path::{Path, PathBuf};

use wte_core::link::{
    FileStamp, LinkFile, LinkState, OpenedState, StaleEvent, archive_entry, link_file_name, links_folder, next_stale,
    rewrite_warning, send_first_question, texts,
};
use wte_core::psd::{PsdColorMode, PsdDoc, PsdError, PsdLimits};
use wte_core::doc_links::LinkApplied;
use wte_core::{ShareScope, StampSource, StickerId};

use super::editor::ExternalEditor;
use super::launch::ChildState;
use super::watch::OwnWritesIo;
use super::{
    EditRequest, HeldEvent, LinkAction, LinkQuestion, LinkTarget, QuestionKind, SendRequest, ask, editor_name, file_name,
    link_path, sticker_name,
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
        LinkAction::LocateEditor(path) => {
            let title = locate_title(&super::default_editor());
            let path = crate::dispatch::resolve_path(app, path, |prefs| crate::io_flows::pick_program(frame, prefs, &title));
            if let Some(path) = path {
                set_editor_exe(&mut app.state, path);
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
            let mut pick = |prefs: &Prefs| {
                if selftest {
                    return None;
                }
                crate::io_flows::pick_program(frame, prefs, &locate_title(&super::default_editor()))
            };
            edit_in(&mut app.state, req, &mut pick);
        }
        other => run_in(&mut app.state, other),
    }
}

/// "Where is photocraft.exe?" (the file dialog's title).
fn locate_title(editor: &ExternalEditor) -> String {
    format!("Where is {}?", editor.exe_name().unwrap_or("the program"))
}

/// The `LinkAction`s that need only the state.
pub(crate) fn run_in(state: &mut AppState, action: LinkAction) {
    match action {
        LinkAction::EditIn(req) => edit_in(state, req, &mut |_| None),
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
            if let Some(ev) = read_now(state, id) {
                apply(state, id, &ev.psd, ev.stamp, true);
            }
        }
        LinkAction::Unlink(id) => {
            let name = sticker_name(state, id);
            let file = file_name(state, id);
            if let Some(doc) = state.doc_mut() {
                doc.unlink(id);
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
            state.link_ui.after_save = Some(req);
            state.push(AppAction::Save);
        }
        LinkAction::Convert(id) => start_convert(state, id),
        LinkAction::LocateEditor(Some(path)) => set_editor_exe(state, path),
        LinkAction::ImportPsd { path: Some(path), keep_linked } => import_psd(state, &path, keep_linked),
        LinkAction::LocateEditor(None) | LinkAction::ImportPsd { path: None, .. } => {
            log::debug!("a file dialog request needs the app; dropped");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Edit in
// ---------------------------------------------------------------------------------------------------------------------

/// "Edit in <editor>" (5.1, 5.5, 5.7): save first, find the editor, convert a text stamp, the shared-picture question
/// (before any file is written), make the link or offer to send WTE's changes, the two-copies guard, then start the
/// editor. Each question re-enters here with its answer. `pick` asks for the editor's exe (`None` in a self-test).
pub(crate) fn edit_in(state: &mut AppState, req: EditRequest, pick: &mut dyn FnMut(&Prefs) -> Option<PathBuf>) {
    let Some(session) = state.session.as_ref() else { return };
    let doc = &session.doc;
    let (stamp, sticker) = match req.target {
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
        let q = LinkQuestion::new(QuestionKind::SaveFirst, sticker, "Save first", vec![texts::SAVE_FIRST.to_owned()])
            .cancel("Cancel")
            .button("Save...", vec![act(LinkAction::SaveThenEdit(req))]);
        ask(state, q);
        return;
    }
    let editor = existing
        .as_ref()
        .and_then(|l| ExternalEditor::from_key(&l.editor, None))
        .unwrap_or_else(super::default_editor);
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
    } else {
        let path = link_path(state, id);
        let missing = path.as_deref().is_none_or(|p| !p.is_file());
        if missing {
            // The file is gone: written again from the sticker (WTE's content; nothing of the editor's is lost by it).
            let file = state.doc().and_then(|d| d.link(id)).map(|l| (l.file.clone(), l.editor.clone()));
            if let Some((file, key)) = file
                && let Err(text) = write_file(state, id, file, &key)
            {
                ask(state, info(Some(id), "Could not write the file", text));
                return;
            }
        } else if !req.open_as_is {
            let wte_changed = wte_changed(state, id);
            let link = state.doc().and_then(|d| d.link(id)).cloned();
            if let Some(link) = link
                && let Some(q) = send_first_question(&link, wte_changed, &editor.name())
            {
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
                return;
            }
        }
    }
    // The two-copies guard (5.7, 13.1): a launched child that still runs (PhotoCraft v0.2.0 and portable copies,
    // another program); never for Photoshop (A3).
    let running = state.link_ui.session(id).child_state() == ChildState::Running;
    if running && !req.open_anyway && editor != ExternalEditor::Photoshop {
        let mut anyway = req.clone();
        anyway.open_anyway = true;
        let q = LinkQuestion::new(
            QuestionKind::AlreadyOpen,
            Some(id),
            "Already open",
            vec![format!("This sticker is already open in {}. A second copy could overwrite your changes.", editor.name())],
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
    match state.link_ui.launcher.open(&editor, &exe, &path) {
        Ok(child) => {
            let s = state.link_ui.session(id);
            if child.is_some() {
                s.child = child;
            }
            s.opened = true;
            s.opened_editor = Some(editor.key().to_owned());
            if let Some(doc) = state.doc_mut() {
                let _ = doc.set_link_file_and_editor(id, None, Some(editor.key()));
            }
            let file = file_name(state, id);
            state.toasts.info(format!("Opening {file} in {}", editor.name()));
        }
        Err(e) => ask(state, info(Some(id), &format!("{} could not be started", editor.name()), e)),
    }
}

fn link_action(a: LinkAction) -> AppAction {
    act(a)
}

/// An OK message.
fn info(sticker: Option<StickerId>, title: &str, text: String) -> LinkQuestion {
    LinkQuestion::new(QuestionKind::Info, sticker, title, vec![text]).cancel("OK")
}

/// The sticker an edit acts on, after the shared-picture question (3.1, review B2): an image stamp is promoted (with
/// every stamp of its picture for "Edit all"), a shared sticker stamp gets its own copy for "Only this one". `None`
/// while the question is asked (or when nothing can be linked).
fn resolve_sticker(state: &mut AppState, req: &EditRequest) -> Option<StickerId> {
    let stamp = match req.target {
        LinkTarget::Sticker(id) => return Some(id),
        LinkTarget::Stamp(s) => s,
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
    if let Err(text) = write_file(state, id, file, editor.key()) {
        ask(state, info(Some(id), "Could not write the file", text));
        return false;
    }
    state.link_ui.made_since_save.insert(id);
    if !state.link_ui.first_note_shown {
        state.link_ui.first_note_shown = true;
        state.toasts.info(styles_note(&editor.name()));
    }
    true
}

/// True when the sticker changed in WTE since the last sync.
fn wte_changed(state: &mut AppState, id: StickerId) -> bool {
    let synced = state.doc().and_then(|d| d.link(id)).map(|l| l.synced_sticker);
    let hash = state.doc_mut().and_then(|d| d.sticker_link_hash(id).ok());
    synced.is_some() && hash != synced
}

/// Writes sticker `id`'s link file (a new link or a send): the PSD of the sticker, atomically, registered with the
/// watcher before the rename; the link is in sync afterwards. `Err` = the sentence the user sees; nothing changed.
pub(crate) fn write_file(state: &mut AppState, id: StickerId, file: LinkFile, editor_key: &str) -> Result<FileStamp, String> {
    let editor = super::editor::name_of_key(editor_key, None);
    let project = state.session.as_ref().and_then(|s| s.path.clone());
    let path = file.path(project.as_deref()).ok_or_else(|| texts::SAVE_FIRST.to_owned())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{} could not be made: {e}.", dir.display()))?;
    }
    let doc = state.doc_mut().ok_or("no document")?;
    let prepared = doc.prepare_link_file(id, file, editor_key).map_err(|e| e.to_string())?;
    let mut io = OwnWritesIo(state.link_ui.own_writes.clone());
    let stamp = wte_core::link::write_link_file(&path, &prepared.bytes, &mut io).map_err(|e| e.sentence(&editor))?;
    let doc = state.doc_mut().ok_or("no document")?;
    doc.finish_link_write(prepared, stamp).map_err(|e| e.to_string())?;
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
    let id = req.sticker;
    let Some(link) = state.doc().and_then(|d| d.link(id)).cloned() else { return };
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
            let name = sticker_name(state, id);
            state.toasts.info(format!("Sent {name} to {editor}"));
            if let Some(then) = req.then_open {
                state.push(act(LinkAction::EditIn(then)));
            }
        }
        Err(text) => ask(state, info(Some(id), "Could not send", text)),
    }
}

/// Reads sticker `id`'s file now ("Update now", the answers): `None` (with a toast or the badge's problem) when it
/// is missing, half written or refused.
fn read_now(state: &mut AppState, id: StickerId) -> Option<HeldEvent> {
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
                let r = doc.link_file_refused(id, &e, stamp, &file);
                crate::dispatch::edited(state, r);
            }
            None
        }
    }
}

/// Applies a file to sticker `id` (5.4, 5.6): one undo step "Update Linked Sticker", the toast "Updated from
/// <editor>" (and the merged reason, the profile note once). In the Sticker view on that sticker the selection is
/// dropped and the active layer re-picked. A canvas too large for a sticker asks "Fit it into the sticker" / Ignore.
pub(crate) fn apply(state: &mut AppState, id: StickerId, psd: &PsdDoc, stamp: FileStamp, fit: bool) {
    let editor = editor_name(state, id);
    let file = file_name(state, id);
    let Some(doc) = state.doc_mut() else { return };
    doc.flush_pending_edit();
    let r = if fit { doc.apply_link_fit(id, psd, stamp, &editor, &file) } else { doc.apply_link_update(id, psd, stamp, &editor, &file) };
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
            after_update_in_view(state, id);
        }
        LinkApplied::Problem(LinkState::CanvasTooLarge { width, height }) => {
            let name = sticker_name(state, id);
            let q = LinkQuestion::new(
                QuestionKind::CanvasTooLarge,
                Some(id),
                "Sticker too large",
                vec![texts::canvas_too_large(&name, width, height, &editor)],
            )
            .cancel("Ignore")
            .button("Fit it into the sticker", vec![act(LinkAction::Fit(id))]);
            ask(state, q);
        }
        LinkApplied::Problem(st) => log::info!("the linked file of sticker {} was not applied: {st:?}", id.0),
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
pub(crate) fn conflict_question(state: &AppState, id: StickerId, saved_over: bool) -> LinkQuestion {
    let editor = editor_name(state, id);
    let name = sticker_name(state, id);
    let text = if saved_over { texts::saved_over(&name, &editor) } else { texts::conflict(&name, &editor) };
    LinkQuestion::new(
        QuestionKind::Conflict { saved_over },
        Some(id),
        "Linked sticker changed",
        vec![text, "Taking one version keeps the other one an undo step away (WTE's) or in the .bak.psd file (the editor's).".to_owned()],
    )
    .button("Decide later", vec![act(LinkAction::Later(id))])
    .button(texts::KEEP_WTES_VERSION, vec![act(LinkAction::Send(SendRequest { answers_conflict: true, ..SendRequest::new(id) }))])
    .button(texts::take_editors_version(&editor), vec![act(LinkAction::Take(id))])
}

/// "Restore from project": the archived copy of a missing `InFolder` file is written back (5.3 "At load").
fn restore(state: &mut AppState, id: StickerId) {
    let Some(project) = state.session.as_ref().and_then(|s| s.path.clone()) else { return };
    let Some(path) = link_path(state, id) else { return };
    let file = file_name(state, id);
    match restore_from_archive(state, &project, id, &path) {
        Ok(()) => state.toasts.info(format!("Restored {file} from the project")),
        Err(e) => state.toasts.warn(format!("{file} could not be restored: {e}")),
    }
}

/// Writes the archived copy of sticker `id`'s file to `path`.
fn restore_from_archive(state: &mut AppState, project: &Path, id: StickerId, path: &Path) -> Result<(), String> {
    let bytes = wte_core::project::read_link_archive(project, &archive_entry(id)).map_err(|e| e.to_string())?;
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

/// The editor's exe (5.7): the remembered one when it exists, else discovery (registry only), else the user's file
/// dialog once ("Where is photocraft.exe?"). `None` (after a hint) when none is known.
fn editor_exe(state: &mut AppState, editor: &ExternalEditor, pick: &mut dyn FnMut(&Prefs) -> Option<PathBuf>) -> Option<PathBuf> {
    let exists = |p: &Path| p.is_file();
    if let Some(p) = state.prefs.photocraft_exe.clone().filter(|p| exists(p)) {
        return Some(p);
    }
    if let Some(p) = super::launch::discover(editor, &*state.link_ui.registry, &exists) {
        log::info!("{} found at {}", editor.name(), p.display());
        state.prefs.photocraft_exe = Some(p.clone());
        return Some(p);
    }
    match pick(&state.prefs) {
        Some(p) => {
            set_editor_exe(state, p.clone());
            Some(p)
        }
        None => {
            state.toasts.warn(format!("{} was not found. Choose its program with Help > {} location...", editor.name(), editor.name()));
            None
        }
    }
}

/// Remembers the editor's exe (Help > "PhotoCraft location...", the first-time question). A file not named like the
/// editor's exe is taken with a warning (5.7: warn, do not refuse).
pub(crate) fn set_editor_exe(state: &mut AppState, path: PathBuf) {
    let editor = super::default_editor();
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if let Some(expected) = editor.exe_name()
        && name != expected.to_ascii_lowercase()
    {
        state.toasts.warn(format!("{} is not named {expected}; WTE will start it anyway", path.display()));
    } else {
        state.toasts.info(format!("{}: {}", editor.name(), path.display()));
    }
    state.prefs.photocraft_exe = Some(path);
    if !state.selftest {
        crate::prefs::save(&state.prefs);
    }
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
                    apply(state, job.sticker, &psd, job.stamp, false);
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
    let psd = match wte_core::psd::read(&bytes, &PsdLimits::default()) {
        Ok(p) => p,
        Err(PsdError::Unsupported(r)) => {
            state.toasts.warn(r.sentence(&file));
            return;
        }
        Err(e) => {
            state.toasts.warn(format!("Cannot open {file}: {e}"));
            return;
        }
    };
    let stem = path.file_stem().map_or_else(|| "Sticker".to_owned(), |s| s.to_string_lossy().into_owned());
    let editor = super::default_editor();
    let Some(doc) = state.doc_mut() else { return };
    let made = doc.add_sticker_from_psd(&psd, &stem, stamp, keep_linked.then(|| path.to_path_buf()), editor.key(), &editor.name(), &file);
    match made {
        Ok(made) => {
            let layers = state.doc().and_then(|d| d.sticker(made.id)).map_or(0, |d| d.layers.len());
            state.tools.stamp.source = crate::state::StampToolSource::Sticker(made.id);
            state.tools.stamp.source_name = stem.clone();
            state.sticker_ui.tray_selected = Some(made.id);
            if made.linked {
                state.link_ui.session(made.id).observed = Some(Some(stamp));
            }
            let s = if layers == 1 { "" } else { "s" };
            state.toasts.info(format!("Imported {file} as a sticker ({layers} layer{s}); click on the model to place it"));
            if let Some(t) = made.toast {
                state.toasts.info(t);
            }
            state.push(AppAction::SetTool(crate::state::ToolKind::Stamp));
            state.status_hint = "Click on the model to place the sticker".to_owned();
        }
        Err(text) => state.toasts.warn(text),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Save, Save As, load
// ---------------------------------------------------------------------------------------------------------------------

/// Before a save to `new_path` (5.3 "At Save As"): when the project moves, every `InFolder` link file is copied into
/// the new links folder (the save archives from there) and the links opened in this session are remembered as
/// aliases-to-be (set when the save succeeds).
pub(crate) fn before_save(state: &mut AppState, new_path: &Path) {
    state.link_ui.pending_aliases.clear();
    let Some(session) = state.session.as_ref() else { return };
    let Some(old) = session.path.clone() else { return };
    if old == new_path {
        return;
    }
    let (from_dir, to_dir) = (links_folder(&old), links_folder(new_path));
    let mut aliases = Vec::new();
    for (id, l) in session.doc.links() {
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
    state.link_ui.made_since_save.clear();
    let aliases = std::mem::take(&mut state.link_ui.pending_aliases);
    if let Some((first, _)) = aliases.first() {
        let editor = editor_name(state, *first);
        let names: Vec<String> = aliases.iter().map(|(id, _)| sticker_name(state, *id)).collect();
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
}

/// After a project was opened (5.3 "At load"; `project::load` only reported): a missing file of the links folder
/// is restored from its archived copy, a file that changed while the project was closed is asked about (Update /
/// Ignore), a missing external file shows "file missing".
pub(crate) fn after_load(state: &mut AppState, project: &Path, links: &[wte_core::project::LinkAtLoad]) {
    use wte_core::project::LinkFileState;
    for l in links {
        let id = l.sticker;
        let Some(link) = state.doc().and_then(|d| d.link(id)).cloned() else { continue };
        match l.state {
            LinkFileState::Present => state.link_ui.session(id).observed = Some(Some(link.synced)),
            LinkFileState::Missing => {
                let restorable = matches!(link.file, LinkFile::InFolder(_)) && l.archive.is_some();
                match (restorable, l.path.as_deref()) {
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
                let stamp = l.path.as_deref().and_then(|p| FileStamp::read(p).ok()).map(|(s, _)| s);
                let s = state.link_ui.session(id);
                s.observed = Some(stamp);
                s.closed_change = true;
                let name = sticker_name(state, id);
                let q = LinkQuestion::new(
                    QuestionKind::ChangedWhileClosed,
                    Some(id),
                    "Linked sticker changed",
                    vec![texts::changed_while_closed(&name)],
                )
                .button("Ignore", vec![act(LinkAction::IgnoreClosed(id))])
                .button("Update", vec![act(LinkAction::UpdateClosed(id))]);
                ask(state, q);
            }
        }
    }
}
