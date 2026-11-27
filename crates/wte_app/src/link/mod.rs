// Owner: stickers P9a (STICKER_PLAN 5.1, 5.3 app parts, 5.5, 5.6, 5.7, 5.8, 13.1; ARCHITECTURE 8.16). The editor
// link in the app: the per-link session state (`LinkSession`: the launched child, whether the file was opened in this
// session, `stale_in_editor`, the Save As alias, a held update), the watcher (`watch.rs`), the launcher and discovery
// (`launch.rs`), the editor type (`editor.rs`), the commands (`flows.rs`: Edit in, Send, Update now, Show file,
// Unlink, Restore, the answers, Import PSD as sticker, the save / Save As / load / close hooks) and the Sticker tab's
// link block and the canvas header's badge (`ui.rs`).
//
// Every text takes the link editor's name (`editor::name_of_key`). P9a built the link with PhotoCraft as the only
// discoverable editor; P9b (STICKER_PLAN 13.1) adds Photoshop's discovery, the editor list ("Edit in..." beside every
// "Edit in <editor>", a submenu in the Layer and row menus), the chooser "Edit in which program?", the default editor,
// Help > Editor locations..., the per-editor two-copies guard and the question before a link is opened in a second
// editor.
//
// Stickers P12 (STICKER_PLAN 5.9, 13.9 "P12"): a raster paint layer can be linked too. Every link has a `LinkId`
// (`Sticker` or `Layer`); the sessions, the watcher, the generic commands (Send, Update now, Show file, Unlink,
// Restore, the conflict and load answers) and the questions are keyed by it. "Edit Layer in <editor>" is
// `LinkTarget::Layer` (`flows::edit_layer_in`); the layer's file is a one-layer PSD at the texture's size, and an
// update is one undo step "Update Linked Layer".
//
// Nothing here raises, focuses or flashes the window or plays a sound: an update applies in place and questions are
// in-window dialogs that wait there (`Dialog::Link`). The watcher's events are applied in `pump` (called by
// `App::logic` every frame), never while a stroke, a drag, a pending Preview edit or a focused text field is open
// (the update is held until they end, 5.6).

pub mod editor;
pub mod flows;
pub mod launch;
pub mod testkit;
pub mod ui;
pub mod watch;

#[cfg(test)]
mod editor_tests;
#[cfg(test)]
mod layer_tests;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use eframe::egui;
use wte_core::link::{FileStamp, LinkFile, LinkState, SyncAction, sync_action, texts};
use wte_core::psd::PsdDoc;
use wte_core::{Document, LayerId, ShareScope, StampId, StickerId};

use crate::action::AppAction;
use crate::state::{AppState, Dialog};
use launch::{ChildProc, ChildState, Launcher, RegistryReader};
use watch::{LinkEvent, LinkWatcher, OwnWrites, WatchEntry, WatchTimes};

/// Stickers P12: which link (a sticker's or a raster paint layer's).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkId {
    Sticker(StickerId),
    Layer(LayerId),
}

impl From<StickerId> for LinkId {
    fn from(id: StickerId) -> LinkId {
        LinkId::Sticker(id)
    }
}

impl From<LayerId> for LinkId {
    fn from(id: LayerId) -> LinkId {
        LinkId::Layer(id)
    }
}

impl LinkId {
    /// The layer of a paint layer link.
    pub fn layer(self) -> Option<LayerId> {
        match self {
            LinkId::Layer(l) => Some(l),
            LinkId::Sticker(_) => None,
        }
    }
}

/// What every kind of link has (stickers P12): its file, its editor's key, the content WTE last wrote or applied.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkCommon {
    pub file: LinkFile,
    pub editor: String,
    pub synced: FileStamp,
}

/// The common part of link `id` of `doc`.
pub fn common(doc: &Document, id: LinkId) -> Option<LinkCommon> {
    match id {
        LinkId::Sticker(s) => doc.link(s).map(|l| LinkCommon { file: l.file.clone(), editor: l.editor.clone(), synced: l.synced }),
        LinkId::Layer(l) => doc.layer_link(l).map(|l| LinkCommon { file: l.file.clone(), editor: l.editor.clone(), synced: l.synced }),
    }
}

/// True while link `id` exists in `doc` and its sticker or raster layer does too (what the watcher follows).
pub fn is_live(doc: &Document, id: LinkId) -> bool {
    match id {
        LinkId::Sticker(s) => doc.link(s).is_some() && doc.sticker(s).is_some(),
        LinkId::Layer(l) => doc.layer_link(l).is_some() && doc.layer(l).is_some_and(|x| x.is_raster()),
    }
}

/// What "Edit in" acts on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// A stamp (an image stamp is promoted to a sticker first; a text stamp is converted after a question).
    Stamp(StampId),
    /// A sticker of the document (a tray sticker, or the sticker the Sticker view shows).
    Sticker(StickerId),
    /// Stickers P12: a raster paint layer ("Edit Layer in <editor>", 5.9).
    Layer(LayerId),
}

/// "Edit in <editor>" and the answers it has collected so far (each question re-enters the flow with one more).
#[derive(Clone, Debug, PartialEq)]
pub struct EditRequest {
    pub target: LinkTarget,
    /// The answer to the shared-picture question (3.1, review B2).
    pub share: Option<ShareScope>,
    /// "Open the file as it is" (`false`) to "Send WTE's changes to the file first?".
    pub open_as_is: bool,
    /// "Open anyway" to the two-copies guard.
    pub open_anyway: bool,
    /// Stickers P9b: the editor's key when one was named (the editor list, the chooser); `None` = the link's editor,
    /// else the default editor.
    pub editor: Option<String>,
    /// Stickers P9b: "Open in <editor> anyway" to "<other editor> may still have this file open." (13.1).
    pub switch_ok: bool,
    /// Stickers P11 (6.4): the PSD handle - the link and its file are made (or written again when missing), no editor
    /// is started; the questions on the way (save first, the shared picture) come back here.
    pub prepare_only: bool,
}

impl EditRequest {
    pub fn new(target: LinkTarget) -> EditRequest {
        EditRequest { target, share: None, open_as_is: false, open_anyway: false, editor: None, switch_ok: false, prepare_only: false }
    }

    /// Stickers P11: the PSD handle's request for `stamp` (no editor is started).
    pub fn for_handle(stamp: StampId) -> EditRequest {
        EditRequest { prepare_only: true, ..EditRequest::new(LinkTarget::Stamp(stamp)) }
    }

    /// The same request in the editor with `key`.
    pub fn in_editor(target: LinkTarget, key: &str) -> EditRequest {
        EditRequest { editor: Some(key.to_owned()), ..EditRequest::new(target) }
    }
}

/// "Send to <editor>" (and "Keep WTE's version", "Send and open").
#[derive(Clone, Debug, PartialEq)]
pub struct SendRequest {
    pub link: LinkId,
    /// "Send anyway" to the warning that the editor may still have the file open.
    pub anyway: bool,
    /// The send answers a conflict ("Keep WTE's version"): `stale_in_editor` is cleared first.
    pub answers_conflict: bool,
    /// "Send and open": the edit to continue with after the send.
    pub then_open: Option<EditRequest>,
}

impl SendRequest {
    pub fn new(link: impl Into<LinkId>) -> SendRequest {
        SendRequest { link: link.into(), anyway: false, answers_conflict: false, then_open: None }
    }
}

/// A command of the editor link (`AppAction::Link`, executed by `flows::run`).
#[derive(Clone, Debug, PartialEq)]
pub enum LinkAction {
    EditIn(EditRequest),
    Send(SendRequest),
    /// The badge's "Update now": read the file and apply it (also after an undo of an update, 5.5).
    UpdateNow(LinkId),
    /// "Show file in Explorer".
    ShowFile(LinkId),
    /// Keep the picture, stop following the file (the PSD stays on disk).
    Unlink(LinkId),
    /// "Restore from project": the archived copy of a missing file.
    Restore(LinkId),
    /// The conflict answers "Take <editor>'s version" and "Decide later" ("Keep WTE's version" is a `Send`).
    Take(LinkId),
    Later(LinkId),
    /// "Changed while the project was closed": Update / Ignore.
    UpdateClosed(LinkId),
    IgnoreClosed(LinkId),
    /// "Fit it into the sticker" (5.4, review A18) / Ignore.
    Fit(StickerId),
    /// "Save..." of the save-first question: save, then continue the edit.
    SaveThenEdit(EditRequest),
    /// Queued right behind the `Save` of `SaveThenEdit` (check-P9a): a save that did not start (its file dialog was
    /// cancelled, a job was running) drops the waiting edit, so a later, unrelated save never starts the editor.
    DropUnstartedEdit,
    /// Help > Editor locations... "Browse..." and the first-time "Where is photocraft.exe?" (`path: None`: the file
    /// dialog) for the editor with key `editor`.
    LocateEditor { editor: String, path: Option<PathBuf> },
    /// Stickers P9b: Help > Editor locations... "Clear" (the program of `editor` is forgotten).
    ClearEditor(String),
    /// Stickers P9b: the default editor (a key; `None` = not chosen, the chooser asks again).
    SetDefaultEditor(Option<String>),
    /// Stickers P9b: "Other program..." of the editor list: the file dialog, then the edit in that program.
    PickOther(EditRequest),
    /// Stickers P9b: Help > Editor locations... (discovery runs again, then the dialog opens).
    ShowEditorLocations,
    /// "Convert with PhotoCraft's command-line tool" of "needs a merged picture" (5.4, 5.7).
    Convert(StickerId),
    /// File > Import > Sticker from PSD... (5.8): a new tray sticker; `keep_linked` makes an `External` link.
    ImportPsd { path: Option<PathBuf>, keep_linked: bool },
}

impl LinkAction {
    /// The commands that change the document at once (dispatch's interaction guard ends a running interaction first).
    pub(crate) fn changes_document(&self) -> bool {
        matches!(
            self,
            LinkAction::EditIn(_)
                | LinkAction::Send(_)
                | LinkAction::UpdateNow(_)
                | LinkAction::Take(_)
                | LinkAction::UpdateClosed(_)
                | LinkAction::Fit(_)
                | LinkAction::ImportPsd { .. }
        )
    }

    /// The guarded commands that keep a running stroke as painted (the file gets what the user sees, like Save).
    pub(crate) fn finishes_interaction(&self) -> bool {
        matches!(self, LinkAction::EditIn(_) | LinkAction::Send(_))
    }
}

/// What a question is about (scenarios and tests look at it; the text is in `lines`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QuestionKind {
    SaveFirst,
    Shared,
    ConvertText,
    SendFirst,
    SendWarning,
    AlreadyOpen,
    Conflict { saved_over: bool },
    ChangedWhileClosed,
    CanvasTooLarge,
    /// A message with OK (a failed write, a failed start).
    Info,
    /// Stickers P9b: "Edit in which program?" (one button per known editor, the box "Always use this one").
    ChooseEditor,
    /// Stickers P9b: the link was opened in another editor in this session (13.1): "Open in <name> anyway" / Cancel.
    OtherEditor,
}

/// One button of a question: its label and what it does (the dialog closes first).
#[derive(Clone, Debug)]
pub struct QuestionButton {
    pub label: String,
    pub actions: Vec<AppAction>,
    /// Stickers P9b: the editor key a chooser button picks; with "Always use this one" ticked the press makes it the
    /// default editor first.
    pub choice: Option<String>,
}

/// An in-window question of the link (`Dialog::Link`).
#[derive(Clone, Debug)]
pub struct LinkQuestion {
    pub kind: QuestionKind,
    /// The link the question is about (stickers P12: a sticker's or a paint layer's).
    pub link: Option<LinkId>,
    pub title: String,
    pub lines: Vec<String>,
    /// Left to right; the last one is the default.
    pub buttons: Vec<QuestionButton>,
}

impl LinkQuestion {
    fn new(kind: QuestionKind, link: Option<LinkId>, title: &str, lines: Vec<String>) -> LinkQuestion {
        LinkQuestion { kind, link, title: title.to_owned(), lines, buttons: Vec::new() }
    }

    fn button(mut self, label: impl Into<String>, actions: Vec<AppAction>) -> LinkQuestion {
        self.buttons.push(QuestionButton { label: label.into(), actions, choice: None });
        self
    }

    /// A Cancel button (closes, nothing else).
    fn cancel(self, label: &str) -> LinkQuestion {
        self.button(label, Vec::new())
    }

    /// A chooser button: picks the editor with `key`.
    fn choice(mut self, label: impl Into<String>, key: &str, actions: Vec<AppAction>) -> LinkQuestion {
        self.buttons.push(QuestionButton { label: label.into(), actions, choice: Some(key.to_owned()) });
        self
    }

    /// The button with this label.
    pub fn find(&self, label: &str) -> Option<&QuestionButton> {
        self.buttons.iter().find(|b| b.label == label)
    }
}

/// What pressing `b` queues: its actions, behind "make this the default editor" for a chooser button while "Always use
/// this one" is ticked (the dialog's buttons and [`press`]).
pub fn button_actions(state: &AppState, b: &QuestionButton) -> Vec<AppAction> {
    let mut out = Vec::new();
    if let Some(key) = &b.choice
        && state.link_ui.chooser_always
    {
        out.push(AppAction::Link(LinkAction::SetDefaultEditor(Some(key.clone()))));
    }
    out.extend(b.actions.iter().cloned());
    out
}

/// A file change that waits: for the view to be idle (5.6), for the answer to a conflict or to "changed while
/// closed".
pub struct HeldEvent {
    pub stamp: FileStamp,
    pub psd: Box<PsdDoc>,
}

/// What this session knows about one link (not saved; 5.3).
#[derive(Default)]
pub struct LinkSession {
    /// The editor process started for this link, if one was kept.
    pub child: Option<Box<dyn ChildProc>>,
    /// The file was opened in an editor in this session (`opened_at`).
    pub opened: bool,
    /// The editor key it was opened in.
    pub opened_editor: Option<String>,
    /// WTE rewrote the file while the editor had it open: the next change of the file is a question (5.5).
    pub stale_in_editor: bool,
    /// The old path of the file after a Save As while the editor may still have it open (5.3).
    pub alias: Option<PathBuf>,
    /// A change that waits.
    pub held: Option<HeldEvent>,
    /// The file's stamp as last seen (`Some(None)` = missing; `None` = not looked at yet: taken as the synced one).
    pub observed: Option<Option<FileStamp>>,
    /// "Changed while the project was closed" waits for its answer.
    pub closed_change: bool,
    /// A conflict question is open or queued.
    pub conflict_open: bool,
    /// "Ignore" was the answer for the file with this content: it is not applied by itself.
    pub ignore_crc: Option<u32>,
    /// The file did not become readable (the badge's extra line).
    pub unreadable: Option<String>,
    /// The colour-profile note was shown for this link.
    pub profile_noted: bool,
}

impl LinkSession {
    /// The launched child's state (`Unknown` without one).
    pub fn child_state(&mut self) -> ChildState {
        self.child.as_mut().map_or(ChildState::Unknown, |c| c.state())
    }
}

/// Counters for tests and scenarios.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkCounters {
    /// Updates applied (`apply_link_update` that changed the sticker or not).
    pub applied: u64,
    /// File changes held because the view was busy.
    pub held: u64,
    /// Conflict questions asked.
    pub conflicts: u64,
    /// Link files written by WTE.
    pub writes: u64,
}

/// A running "Convert with the command-line tool" (5.7): the child, its output, when it started.
pub struct ConvertJob {
    pub sticker: StickerId,
    pub child: Box<dyn ChildProc>,
    pub png: PathBuf,
    /// The PSD's stamp the converted picture stands for.
    pub stamp: FileStamp,
    pub started: std::time::Instant,
}

/// The link state of the app (`AppState::link_ui`).
pub struct LinkUi {
    pub sessions: BTreeMap<LinkId, LinkSession>,
    pub launcher: Box<dyn Launcher>,
    pub registry: Box<dyn RegistryReader>,
    pub own_writes: OwnWrites,
    pub watcher: Option<LinkWatcher>,
    pub times: WatchTimes,
    /// Repaint requests of every watcher of this run (the idle check of 8.2).
    pub repaints: Arc<AtomicU64>,
    /// The list last sent to the watcher.
    sent: Option<Vec<WatchEntry>>,
    /// Questions waiting for the dialog slot.
    pub questions: VecDeque<LinkQuestion>,
    /// Stickers (and paint layers, P12) linked since the last project save (the close prompt's line, 5.3).
    pub made_since_save: BTreeSet<LinkId>,
    /// "Save..." of the save-first question, until its save starts (`flows::before_save` moves it to `after_save`;
    /// `LinkAction::DropUnstartedEdit` drops it when no save started; check-P9a).
    pub save_then_edit: Option<EditRequest>,
    /// The edit that continues after the save that is running now.
    pub after_save: Option<EditRequest>,
    /// The links made since the last save that the running save holds (`made_since_save` when it started).
    pub saving_made: BTreeSet<LinkId>,
    /// A Save As in flight: the links whose old file becomes an alias when it succeeds.
    pub pending_aliases: Vec<(LinkId, PathBuf)>,
    /// The note about styles was shown in this session.
    pub first_note_shown: bool,
    /// Stickers P12: the note about a linked paint layer was shown in this session.
    pub layer_note_shown: bool,
    pub convert: Option<ConvertJob>,
    pub counters: LinkCounters,
    /// The window had the focus in the last frame (a gained focus pokes the watcher).
    focused: bool,
    /// Stickers P9b: discovery ran in this session (`editor::ensure_discovered`).
    pub discovered: bool,
    /// Stickers P9b: the editors whose program exists, in list order (refreshed by discovery and by every change of a
    /// program; the labels drawn every frame read it).
    pub known: Vec<editor::ExternalEditor>,
    /// Stickers P9b: the box "Always use this one" of the chooser (ticked when the chooser opens).
    pub chooser_always: bool,
}

impl LinkUi {
    /// A self-test run (and every test build) gets the stub launcher and an empty registry.
    pub fn new(selftest: bool) -> LinkUi {
        let launcher = launch::launcher_for(selftest);
        if launcher.is_stub() {
            log::info!("editor link: the stub launcher (nothing is started)");
        }
        LinkUi {
            sessions: BTreeMap::new(),
            launcher,
            registry: launch::registry_for(selftest),
            own_writes: OwnWrites::default(),
            watcher: None,
            times: WatchTimes::default(),
            repaints: Arc::new(AtomicU64::new(0)),
            sent: None,
            questions: VecDeque::new(),
            made_since_save: BTreeSet::new(),
            save_then_edit: None,
            after_save: None,
            saving_made: BTreeSet::new(),
            pending_aliases: Vec::new(),
            first_note_shown: false,
            layer_note_shown: false,
            convert: None,
            counters: LinkCounters::default(),
            focused: false,
            discovered: false,
            known: Vec::new(),
            chooser_always: true,
        }
    }

    /// The session of `id` (made on first use).
    pub fn session(&mut self, id: impl Into<LinkId>) -> &mut LinkSession {
        self.sessions.entry(id.into()).or_default()
    }

    /// Everything of the previous document goes (a new document, a load).
    pub fn reset(&mut self) {
        self.sessions.clear();
        self.watcher = None;
        self.sent = None;
        self.questions.clear();
        self.made_since_save.clear();
        self.save_then_edit = None;
        self.after_save = None;
        self.saving_made.clear();
        self.pending_aliases.clear();
        self.convert = None;
    }

    /// Repaint requests made by the watchers so far.
    pub fn repaint_count(&self) -> u64 {
        self.repaints.load(std::sync::atomic::Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------------------------------

/// The name of the link editor of link `id` (for a sticker or layer without a link: the editor a new link opens in,
/// or "your editor" while the chooser has to ask).
pub fn editor_name(state: &AppState, id: impl Into<LinkId>) -> String {
    match state.doc().and_then(|d| common(d, id.into())) {
        Some(l) => editor::name_of_key(&l.editor, state.prefs.other_editor_exe.as_deref()),
        None => editor_name_new(state),
    }
}

/// The name of the editor a new link opens in ("your editor" while the chooser has to ask).
pub fn editor_name_new(state: &AppState) -> String {
    editor::quiet_editor(state).map_or_else(|| "your editor".to_owned(), |e| e.name())
}

/// The label of the main "Edit in" control for sticker `sticker` (`None`: an image or text stamp, not linked yet):
/// "Edit in <link's editor>", else "Edit in <default or only known editor>", else "Edit in..." (the chooser asks).
pub fn edit_in_label(state: &AppState, sticker: Option<StickerId>) -> String {
    if let Some(id) = sticker
        && state.doc().and_then(|d| d.link(id)).is_some()
    {
        return texts::edit_in(&editor_name(state, id));
    }
    editor::quiet_editor(state).map_or_else(|| ui::LIST_LABEL.to_owned(), |e| texts::edit_in(&e.name()))
}

/// The sticker's name ("sticker" when it is gone).
pub fn sticker_name(state: &AppState, id: StickerId) -> String {
    state.doc().and_then(|d| d.sticker(id)).map_or_else(|| "sticker".to_owned(), |d| d.name.clone())
}

/// The name of what link `id` belongs to: the sticker's name, or the paint layer's ("layer" when it is gone).
pub fn link_name(state: &AppState, id: impl Into<LinkId>) -> String {
    match id.into() {
        LinkId::Sticker(s) => sticker_name(state, s),
        LinkId::Layer(l) => state.doc().and_then(|d| d.layer(l)).map_or_else(|| "layer".to_owned(), |x| x.name.clone()),
    }
}

/// Where link `id`'s file is (`None` without a link, or an `InFolder` link of a never-saved project).
pub fn link_path(state: &AppState, id: impl Into<LinkId>) -> Option<PathBuf> {
    let session = state.session.as_ref()?;
    common(&session.doc, id.into())?.file.path(session.path.as_deref())
}

/// The file name of link `id`.
pub fn file_name(state: &AppState, id: impl Into<LinkId>) -> String {
    state.doc().and_then(|d| common(d, id.into())).map_or_else(String::new, |l| l.file.display_name())
}

/// True when link `id`'s sticker or layer changed in WTE since the last sync (`wte_changed` of 5.5).
pub fn wte_changed(state: &mut AppState, id: impl Into<LinkId>) -> bool {
    let Some(doc) = state.doc_mut() else { return false };
    match id.into() {
        LinkId::Sticker(s) => {
            let synced = doc.link(s).map(|l| l.synced_sticker);
            let hash = doc.sticker_link_hash(s).ok();
            synced.is_some() && hash != synced
        }
        LinkId::Layer(l) => doc.layer_link_changed_in_wte(l),
    }
}

/// The sticker a stamp shows (`None` for an image or text stamp).
pub fn sticker_of_stamp(state: &AppState, stamp: StampId) -> Option<StickerId> {
    match state.doc()?.stamp(stamp)?.source {
        wte_core::StampSource::Sticker { sticker } => Some(sticker),
        _ => None,
    }
}

/// Shows `q` now when no dialog is open, else queues it.
pub fn ask(state: &mut AppState, q: LinkQuestion) {
    if matches!(state.dialog, Dialog::None) {
        state.dialog = Dialog::Link(Box::new(q));
    } else {
        state.link_ui.questions.push_back(q);
    }
}

/// Presses the button `label` of the open link question (the dialog's buttons and the scenarios): the dialog closes
/// and the button's actions are queued. `Err` when no link question with that button is open.
pub fn press(state: &mut AppState, label: &str) -> Result<(), String> {
    let Dialog::Link(q) = &state.dialog else { return Err("no link question is open".to_owned()) };
    let Some(b) = q.find(label) else {
        let labels: Vec<&str> = q.buttons.iter().map(|b| b.label.as_str()).collect();
        return Err(format!("the question has no \"{label}\" button: {labels:?}"));
    };
    let actions = button_actions(state, b);
    state.dialog = Dialog::None;
    state.actions.extend(actions);
    Ok(())
}

/// The open link question, if one is shown.
pub fn open_question(state: &AppState) -> Option<&LinkQuestion> {
    match &state.dialog {
        Dialog::Link(q) => Some(q),
        _ => None,
    }
}

/// The badge of link `id`: its state and its text (`None` without a link).
pub fn badge(state: &mut AppState, id: impl Into<LinkId>) -> Option<(LinkState, String)> {
    let id = id.into();
    let observed = {
        let link = common(state.doc()?, id)?;
        match state.link_ui.sessions.get(&id).and_then(|s| s.observed) {
            None => Some(link.synced),
            Some(o) => o,
        }
    };
    let editor = editor_name(state, id);
    let name = link_name(state, id);
    let file = file_name(state, id);
    let st = match id {
        LinkId::Sticker(s) => state.doc_mut()?.link_state(s, observed.as_ref()).ok()?,
        LinkId::Layer(l) => state.doc_mut()?.layer_link_state(l, observed.as_ref()).ok()?,
    };
    let text = st.badge(&editor, &name, &file);
    Some((st, text))
}

// ---------------------------------------------------------------------------------------------------------------------
// The frame hook
// ---------------------------------------------------------------------------------------------------------------------

/// True while an update of link `id` must wait (5.6): a 3D tool interaction, a pending Preview edit or an open
/// sticker stroke anywhere in the document, and in the Sticker view on that sticker a drag of the canvas or a
/// focused text field. A paint layer's update waits for the same document-wide interactions (stickers P12).
pub fn busy_for(state: &AppState, id: impl Into<LinkId>, tools_busy: bool) -> bool {
    let Some(session) = state.session.as_ref() else { return false };
    if tools_busy || session.doc.has_pending_edit() || session.doc.sticker_stroke_active() {
        return true;
    }
    // check-P12: an open texture stroke (also one whose button is already up) refuses `replace_layer_pixels`; the
    // update waits for its end instead of being lost with an error toast.
    let id = match id.into() {
        LinkId::Sticker(id) => id,
        LinkId::Layer(_) => return session.doc.stroke_active(),
    };
    let in_view = session.sticker_edit.as_ref().is_some_and(|e| match e.target {
        crate::state::StickerTarget::Tray(s) => s == id,
        crate::state::StickerTarget::Stamp(stamp) => session.doc.stamp(stamp).is_some_and(|o| {
            matches!(o.source, wte_core::StampSource::Sticker { sticker } if sticker == id)
        }),
    });
    in_view && (state.sticker_ui.drag.is_some() || state.typing_prev)
}

/// The list the watcher should have.
fn wanted(state: &AppState) -> Vec<WatchEntry> {
    let Some(session) = state.session.as_ref() else { return Vec::new() };
    let doc = &session.doc;
    let ids = doc.links().map(|(id, _)| LinkId::Sticker(id)).chain(doc.layer_links().map(|(id, _)| LinkId::Layer(id)));
    ids.filter(|id| is_live(doc, *id))
        .filter_map(|id| {
            let link = common(doc, id)?;
            let path = link.file.path(session.path.as_deref())?;
            let alias = state.link_ui.sessions.get(&id).and_then(|s| s.alias.clone());
            Some(WatchEntry { id, path, alias, synced: link.synced })
        })
        .collect()
}

/// Every frame (`App::logic`): keeps the watcher's list, applies or holds its events, retries held updates, drops
/// aliases whose editor has exited, polls a running conversion and shows a waiting question.
pub fn pump(state: &mut AppState, ctx: &egui::Context, tools_busy: bool) {
    let want = wanted(state);
    let ui = &mut state.link_ui;
    if ui.sent.as_ref() != Some(&want) {
        if want.is_empty() {
            ui.watcher = None;
        } else {
            if ui.watcher.as_ref().is_none_or(|w| !w.alive().load(std::sync::atomic::Ordering::SeqCst)) {
                let repaint_ctx = ctx.clone();
                ui.watcher = Some(LinkWatcher::start(
                    ui.own_writes.clone(),
                    Arc::new(move || repaint_ctx.request_repaint()),
                    Arc::clone(&ui.repaints),
                    ui.times,
                ));
            }
            if let Some(w) = &ui.watcher {
                w.set_list(want.clone());
            }
        }
        ui.sent = Some(want);
    }
    // A gained focus polls at once (5.6).
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
    if focused && !ui.focused
        && let Some(w) = &ui.watcher
    {
        w.poke();
    }
    ui.focused = focused;
    let events = ui.watcher.as_ref().map(|w| w.drain()).unwrap_or_default();
    for event in events {
        on_event(state, event, tools_busy);
    }
    retry_held(state, tools_busy);
    drop_finished_aliases(state);
    flows::poll_convert(state);
    if matches!(state.dialog, Dialog::None)
        && let Some(q) = state.link_ui.questions.pop_front()
    {
        state.dialog = Dialog::Link(Box::new(q));
    }
}

/// One watcher event.
pub fn on_event(state: &mut AppState, event: LinkEvent, tools_busy: bool) {
    let id = event.id();
    if state.doc().and_then(|d| common(d, id)).is_none() {
        return;
    }
    match event {
        LinkEvent::Missing { id } => {
            state.link_ui.session(id).observed = Some(None);
        }
        LinkEvent::Unreadable { id } => {
            let text = texts::unreadable(&file_name(state, id), "it stays incomplete");
            state.toasts.warn(text.clone());
            state.link_ui.session(id).unreadable = Some(text);
        }
        LinkEvent::Refused { id, stamp, error } => {
            let file = file_name(state, id);
            let s = state.link_ui.session(id);
            s.observed = Some(Some(stamp));
            s.unreadable = None;
            if let Some(doc) = state.doc_mut() {
                let r = match id {
                    LinkId::Sticker(sid) => doc.link_file_refused(sid, &error, stamp, &file),
                    LinkId::Layer(lid) => doc.layer_link_file_refused(lid, &error, stamp, &file),
                };
                crate::dispatch::edited(state, r);
            }
        }
        LinkEvent::Changed { id, stamp, psd, from_alias } => {
            if from_alias {
                log::info!("the linked file of {id:?} was saved at its old place and copied over");
            }
            let s = state.link_ui.session(id);
            s.observed = Some(Some(stamp));
            s.unreadable = None;
            if s.ignore_crc == Some(stamp.crc32) {
                return;
            }
            if s.closed_change || s.conflict_open {
                s.held = Some(HeldEvent { stamp, psd });
                return;
            }
            if busy_for(state, id, tools_busy) {
                hold(state, id, HeldEvent { stamp, psd });
                return;
            }
            decide(state, id, HeldEvent { stamp, psd });
        }
    }
}

/// Keeps a change until the view is idle (5.6).
fn hold(state: &mut AppState, id: LinkId, ev: HeldEvent) {
    state.link_ui.session(id).held = Some(ev);
    state.link_ui.counters.held += 1;
    if let Some(e) = crate::sticker::edit_mut(state) {
        e.held_update = true;
    }
}

/// Held changes whose sticker is no longer busy are decided now.
fn retry_held(state: &mut AppState, tools_busy: bool) {
    let ids: Vec<LinkId> = state
        .link_ui
        .sessions
        .iter()
        .filter(|(_, s)| s.held.is_some() && !s.closed_change && !s.conflict_open)
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        if busy_for(state, id, tools_busy) {
            continue;
        }
        if let Some(ev) = state.link_ui.session(id).held.take() {
            decide(state, id, ev);
        }
    }
    let any_held = state.link_ui.sessions.values().any(|s| s.held.is_some());
    if !any_held && let Some(e) = crate::sticker::edit_mut(state) {
        e.held_update = false;
    }
}

/// The table of 5.5 for a settled change of the file.
fn decide(state: &mut AppState, id: LinkId, ev: HeldEvent) {
    let Some(link) = state.doc().and_then(|d| common(d, id)) else { return };
    let file_changed = !ev.stamp.same_content(&link.synced);
    let wte_changed = wte_changed(state, id);
    let stale = state.link_ui.sessions.get(&id).is_some_and(|s| s.stale_in_editor);
    match sync_action(file_changed, wte_changed, stale) {
        SyncAction::Nothing => {}
        SyncAction::Apply => flows::apply(state, id, &ev.psd, ev.stamp, false),
        SyncAction::Conflict { saved_over } => {
            let s = state.link_ui.session(id);
            s.held = Some(ev);
            s.conflict_open = true;
            state.link_ui.counters.conflicts += 1;
            let q = flows::conflict_question(state, id, saved_over);
            ask(state, q);
        }
    }
}

/// Drops an alias once its launched editor is known to have exited (5.3).
fn drop_finished_aliases(state: &mut AppState) {
    for s in state.link_ui.sessions.values_mut() {
        if s.alias.is_some() && s.child.is_some() && s.child_state() == ChildState::Exited {
            s.alias = None;
        }
    }
}

/// The "linked since the last save" line of the unsaved-changes prompt (5.3, review B8); `None` without such links.
pub fn unsaved_line(state: &AppState) -> Option<String> {
    let session = state.session.as_ref()?;
    // check-P12: a paint layer deleted since (its link stays for an undo, but a save leaves it out) is not named.
    let pending: Vec<LinkId> = state
        .link_ui
        .made_since_save
        .iter()
        .copied()
        .filter(|id| common(&session.doc, *id).is_some_and(|l| matches!(l.file, LinkFile::InFolder(_))))
        .filter(|id| id.layer().is_none() || is_live(&session.doc, *id))
        .collect();
    let names: Vec<String> = pending
        .iter()
        .map(|id| match id {
            LinkId::Sticker(_) => link_name(state, *id),
            LinkId::Layer(_) => format!("{} (paint layer)", link_name(state, *id)),
        })
        .collect();
    if names.is_empty() {
        return None;
    }
    let folder = session.path.as_deref().map(wte_core::link::links_folder).map_or_else(String::new, |p| p.display().to_string());
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let editors = editors_of(state, pending.into_iter());
    Some(texts::unsaved_links(&refs, &editors, &folder))
}

/// The editors of these links' stickers, each once, in the order met: "Photoshop", "Photoshop and PhotoCraft".
pub fn editors_of(state: &AppState, ids: impl Iterator<Item = LinkId>) -> String {
    let mut names: Vec<String> = Vec::new();
    for id in ids {
        let name = editor_name(state, id);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
