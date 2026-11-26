// Owner: stickers P6a (STICKER_PLAN 3.1-3.3, 3.5, 3.10; ARCHITECTURE 8.15). The Sticker view of the main window:
// entering and leaving it, the commands of `StickerAction` (each one `Document` command, so one history entry), the
// UI state that is not in the document (`StickerUi`), and the helpers the canvas (`canvas.rs`) and the panels
// (`layers_ui.rs`) share.
//
// What the editor edits (`StickerTarget`): the sticker of a stamp. An image stamp shows a virtual one-layer sticker
// built here from its picture (nothing in the document changes); its first content edit passes
// `StickerRef::OfStamp` and the core promotes it inside that edit (one undo step, STICKER_PLAN 2.10). A text stamp
// shows its picture read-only with the button "Convert to layered sticker".

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use wte_core::{
    BlendMode, BlobId, Commit, Document, PicturePixels, PixelBuf, REVERT_LABEL, ShareScope, StampId,
    StampSource, StickerContent, StickerDoc, StickerId, StickerLayer, StickerLayerId, StickerRef,
};

use crate::app::WteApp;
use crate::state::{AppState, Canvas2D, StickerEdit, StickerTarget, StickerTool, ViewMode};

pub mod canvas;
pub mod layers_ui;

/// Status line when the Sticker view is asked for without a selected stamp.
pub(crate) const NO_STAMP_HINT: &str = "Select a stamp or a text first (click it with the Transform tool, or its layer)";
/// Status line of Delete / Backspace in the Sticker view without a selection (STICKER_PLAN 3.3).
pub(crate) const DELETE_HINT: &str = "Select an area first, or use Delete Layer";
/// History labels of the editor's commands (one entry each; the Preview drags fold by label).
pub(crate) const MOVE_LABEL: &str = "Move Sticker Layer";
const NUDGE_LABEL: &str = "Nudge Sticker Layer";
const OPACITY_LABEL: &str = "Sticker Layer Opacity";
const BLEND_LABEL: &str = "Sticker Layer Blend";
const RENAME_LABEL: &str = "Rename Sticker Layer";
const REORDER_LABEL: &str = "Reorder Sticker Layers";
const ADD_LABEL: &str = "Add Picture Layer";
const NEW_LABEL: &str = "New Picture Layer";
const DUPLICATE_LABEL: &str = "Duplicate Sticker Layer";
const DELETE_LABEL: &str = "Delete Sticker Layer";
const RENAME_STICKER_LABEL: &str = "Rename Sticker";

/// A command of the Sticker view (dispatched by `run`). "`None` layer" = the active layer.
#[derive(Clone, Debug)]
pub enum StickerAction {
    /// Opens the Sticker view on this stamp (`None` = the selected stamp). In the view already: switches the target.
    Enter(Option<StampId>),
    /// "Done": back to the view mode the editor was opened from.
    Leave,
    /// "Revert": the sticker (and its stamp) as they were when the editor opened; one entry, itself undoable.
    Revert,
    /// Makes this layer the active one (app state, no entry).
    SelectLayer(StickerLayerId),
    /// A new picture layer above the active one: this image blob centred on the artboard, or (`None`) an empty one of
    /// the artboard's size.
    AddPictureLayer(Option<BlobId>),
    /// "Add picture...": the native file dialog (`None`), never in a self-test run; then `AddPictureLayer`.
    AddPictureFile(Option<PathBuf>),
    DuplicateLayer(Option<StickerLayerId>),
    DeleteLayer(Option<StickerLayerId>),
    /// Ctrl+] / Ctrl+[ and the Up / Down buttons: one step up (+1) or down (-1).
    MoveLayer(Option<StickerLayerId>, i32),
    /// A drop in the Layers list: the layer goes to this index (bottom = 0).
    MoveLayerTo(StickerLayerId, usize),
    /// Ctrl+M on the active sticker layer (never on a texture layer). Merging comes with the layer commands of P8.
    MergeDown(Option<StickerLayerId>),
    SetLayerVisible(StickerLayerId, bool),
    SetLayerLocked(StickerLayerId, bool),
    SetLayerOpacity(StickerLayerId, f32, Commit),
    SetLayerBlend(StickerLayerId, BlendMode),
    RenameLayer(StickerLayerId, String),
    /// The Move tool and the X / Y fields: the layer's translation in layer-space pixels.
    SetLayerPosition(StickerLayerId, [f32; 2], Commit),
    /// The arrow keys: the active layer by whole pixels (Shift: 10).
    Nudge(i32, i32),
    /// Delete / Backspace in the Sticker view: clears the selected pixels (P7b); never deletes the stamp.
    DeleteKey,
    /// Esc in the Sticker view: cancels a drag, leaves a field, drops the selection, else leaves the view.
    Escape,
    /// Enter in the Sticker view: applies a crop / closes a lasso (P7b, P8); nothing else.
    EnterKey,
    /// Fits the artboard into the canvas (F, "Fit").
    Fit,
    /// One sticker pixel per point ("100 %").
    Zoom100,
    /// A text stamp's "Convert to layered sticker" (`Document::convert_text_stamp`).
    ConvertText,
    /// The sticker's name (canvas header).
    Rename(String),
}

impl StickerAction {
    /// True for the commands that change the document at once (a `Commit::Final` or a structure edit): dispatch's
    /// interaction guard ends a running tool interaction before them.
    pub(crate) fn changes_document(&self) -> bool {
        use StickerAction as S;
        match self {
            S::SetLayerOpacity(_, _, c) | S::SetLayerPosition(_, _, c) => *c == Commit::Final,
            S::Revert
            | S::AddPictureLayer(_)
            | S::AddPictureFile(_)
            | S::DuplicateLayer(_)
            | S::DeleteLayer(_)
            | S::MoveLayer(..)
            | S::MoveLayerTo(..)
            | S::SetLayerVisible(..)
            | S::SetLayerLocked(..)
            | S::SetLayerBlend(..)
            | S::RenameLayer(..)
            | S::Nudge(..)
            | S::ConvertText
            | S::Rename(_) => true,
            _ => false,
        }
    }
}

/// What the canvas shows (`shown`).
pub(crate) enum Shown {
    /// A sticker of the document.
    Sticker { id: StickerId, doc: Arc<StickerDoc> },
    /// An image stamp that is not a sticker yet: the virtual one-layer sticker of its picture (layer id 1, the id the
    /// promotion gives it).
    Image { blob: BlobId, doc: StickerDoc },
    /// A text stamp: its picture, read-only.
    Text { stamp: StampId },
}

impl Shown {
    /// The layered document the Layers tab and the tools work on (`None` for a text stamp).
    pub(crate) fn layers(&self) -> Option<&StickerDoc> {
        match self {
            Shown::Sticker { doc, .. } => Some(doc),
            Shown::Image { doc, .. } => Some(doc),
            Shown::Text { .. } => None,
        }
    }
}

/// The canvas texture and what it was made from.
pub(crate) struct CanvasTex {
    pub(crate) key: canvas::CanvasKey,
    pub(crate) tex: egui::TextureHandle,
    pub(crate) size: [u32; 2],
}

/// A layer thumbnail of the Layers tab and what it was made from.
pub(crate) struct ThumbTex {
    pub(crate) key: layers_ui::ThumbKey,
    pub(crate) tex: Option<egui::TextureHandle>,
}

/// A drag on the canvas (stickers P6a: Move and pan).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CanvasDrag {
    /// The Move tool: the layer, where the press was (layer space), the layer's translation then, the translation
    /// last sent (`None` = nothing sent yet: a click), and the physical button.
    Move { layer: StickerLayerId, start: glam::Vec2, from: glam::Vec2, sent: Option<glam::Vec2>, button: egui::PointerButton },
    /// Hand, Space + drag, a Secondary or Middle drag (the companion's one-finger drag): the last pointer position.
    Pan { last: egui::Pos2, button: egui::PointerButton },
}

/// UI state of the Sticker view (`AppState::sticker_ui`): caches and the running drag; nothing of it is saved.
pub struct StickerUi {
    pub(crate) canvas: Option<CanvasTex>,
    pub(crate) thumbs: HashMap<u32, ThumbTex>,
    pub(crate) drag: Option<CanvasDrag>,
    /// The Move tool's "Auto-select layer" option (on: a click picks the top layer under the pointer).
    pub auto_select: bool,
    /// The tool the view had when it was left (the next opening starts with it).
    pub(crate) last_tool: StickerTool,
    /// A zoom asked for by "100 %" (applied about the canvas centre in the next frame).
    pub(crate) zoom_request: Option<f32>,
    /// The sticker name being typed in the canvas header.
    pub(crate) name_edit: Option<String>,
    /// The active layer's name being typed in the Layers tab.
    pub(crate) layer_name_edit: Option<(StickerLayerId, String)>,
    /// Pictures uploaded to the canvas texture so far (tests: the canvas re-renders only on a change).
    pub(crate) canvas_uploads: u64,
}

impl Default for StickerUi {
    fn default() -> StickerUi {
        StickerUi {
            canvas: None,
            thumbs: HashMap::new(),
            drag: None,
            auto_select: true,
            last_tool: StickerTool::Move,
            zoom_request: None,
            name_edit: None,
            layer_name_edit: None,
            canvas_uploads: 0,
        }
    }
}

impl StickerUi {
    /// True while a drag on the canvas runs (the pen's eraser end waits for it, like for a tool interaction).
    pub(crate) fn busy(&self) -> bool {
        self.drag.is_some()
    }
}

/// The open Sticker view of the state, if any.
pub(crate) fn edit_of(state: &AppState) -> Option<&StickerEdit> {
    state.session.as_ref()?.sticker_edit.as_ref()
}

/// The open Sticker view, mutable.
pub(crate) fn edit_mut(state: &mut AppState) -> Option<&mut StickerEdit> {
    state.session.as_mut()?.sticker_edit.as_mut()
}

/// The sticker tool of the open Sticker view (Move outside it).
pub(crate) fn tool_of(state: &AppState) -> StickerTool {
    edit_of(state).map_or(StickerTool::Move, |e| e.tool)
}

/// The sticker a content command of the editor acts on (`None` for the Stamp tool's unplaced sticker, P6b).
pub(crate) fn edit_ref(edit: &StickerEdit) -> Option<StickerRef> {
    match edit.target {
        StickerTarget::Stamp(stamp) => Some(StickerRef::OfStamp(stamp, edit.share_answer.unwrap_or(ShareScope::ThisStamp))),
        StickerTarget::Tray(id) => Some(StickerRef::Id(id)),
        StickerTarget::ToolSource => None,
    }
}

/// What the editor shows for `edit` (`None` when its target is gone).
pub(crate) fn shown(doc: &mut Document, edit: &StickerEdit) -> Option<Shown> {
    match edit.target {
        StickerTarget::Stamp(stamp) => {
            let object = doc.stamp(stamp)?;
            let name = object.name.clone();
            match object.source.clone() {
                StampSource::Sticker { sticker } => Some(Shown::Sticker { id: sticker, doc: Arc::clone(doc.sticker(sticker)?) }),
                StampSource::Image { blob } => {
                    let size = doc.image_size(blob).ok()?;
                    Some(Shown::Image { blob, doc: StickerDoc::from_picture(&name, PicturePixels::Blob(blob), size) })
                }
                StampSource::Text(_) => Some(Shown::Text { stamp }),
            }
        }
        StickerTarget::Tray(id) => Some(Shown::Sticker { id, doc: Arc::clone(doc.sticker(id)?) }),
        StickerTarget::ToolSource => None,
    }
}

/// The layer that becomes active when the editor opens on `doc`'s stamp `stamp`: the top layer (the virtual layer 1 of
/// an image stamp), none for a text stamp.
fn first_active(doc: &Document, stamp: StampId) -> Option<StickerLayerId> {
    match &doc.stamp(stamp)?.source {
        StampSource::Sticker { sticker } => doc.sticker(*sticker)?.layers.last().map(|l| l.id),
        StampSource::Image { .. } => Some(StickerLayerId(1)),
        StampSource::Text(_) => None,
    }
}

/// True when the sticker or its stamp differs from how they were when the editor opened (Revert is enabled).
pub(crate) fn differs_from_entry(doc: &Document, edit: &StickerEdit) -> bool {
    let stamps = edit.entry.stamps();
    let stamp_changed = stamps.iter().any(|s| doc.stamp(s.id).is_some_and(|cur| cur != s));
    let sticker_changed = stamps.iter().filter_map(|s| match s.source {
        StampSource::Sticker { sticker } => Some(sticker),
        _ => None,
    });
    let sticker_changed = sticker_changed.into_iter().any(|id| match (doc.sticker(id), edit.entry.sticker(id)) {
        (Some(cur), Some(old)) => !Arc::ptr_eq(cur, old) && **cur != **old,
        (None, Some(_)) => true,
        _ => false,
    });
    stamp_changed || sticker_changed
}

/// Opens the Sticker view on `stamp` (`None` = the selected stamp). Without a stamp the status line says what to do.
pub(crate) fn enter(app: &mut WteApp, stamp: Option<StampId>) {
    if app.tools.is_busy() {
        app.tools.cancel(&mut app.state);
    }
    enter_in(&mut app.state, stamp);
}

/// `enter` on the state alone (the tools' interaction is the caller's).
pub(crate) fn enter_in(state: &mut AppState, stamp: Option<StampId>) {
    cancel_drag(state);
    let Some(session) = state.session.as_mut() else { return };
    let Some(stamp) = stamp.or(session.selected_stamp).filter(|id| session.doc.stamp(*id).is_some()) else {
        state.status_hint = NO_STAMP_HINT.to_owned();
        return;
    };
    if state.view_mode == ViewMode::Sticker && session.sticker_edit.as_ref().is_some_and(|e| e.target == StickerTarget::Stamp(stamp)) {
        return;
    }
    session.doc.flush_pending_edit();
    let return_mode = match (&session.sticker_edit, state.view_mode) {
        (Some(e), ViewMode::Sticker) => e.return_mode,
        (None, ViewMode::Sticker) => state.last_edit_mode,
        (_, mode) => mode,
    };
    let tool = session.sticker_edit.as_ref().map_or(state.sticker_ui.last_tool, |e| e.tool);
    let entry = session.doc.sticker_checkpoint(&[stamp], &[]);
    let active_layer = first_active(&session.doc, stamp);
    session.sticker_edit = Some(StickerEdit {
        target: StickerTarget::Stamp(stamp),
        active_layer,
        selected_layers: active_layer.into_iter().collect(),
        selection: None,
        tool,
        camera: Canvas2D::default(),
        return_mode,
        share_answer: None,
        entry,
        held_update: false,
    });
    state.view_mode = ViewMode::Sticker;
    state.status_hint.clear();
    state.sticker_ui.name_edit = None;
    state.sticker_ui.layer_name_edit = None;
    crate::dispatch::select_stamp(state, Some(stamp));
}

/// Leaves the Sticker view (Done, Esc, another view mode): a pending edit is flushed, a drag on the canvas cancelled;
/// the view goes to `to`, or to the mode the editor was opened from. The paint tool is the one the user had.
pub(crate) fn leave(app: &mut WteApp, to: Option<ViewMode>) {
    if app.tools.is_busy() {
        app.tools.cancel(&mut app.state);
    }
    leave_in(&mut app.state, to);
}

/// `leave` on the state alone.
pub(crate) fn leave_in(state: &mut AppState, to: Option<ViewMode>) {
    cancel_drag(state);
    let edit = state.session.as_mut().and_then(|s| {
        s.doc.flush_pending_edit();
        s.sticker_edit.take()
    });
    if let Some(e) = &edit {
        state.sticker_ui.last_tool = e.tool;
    }
    let back = to.or(edit.map(|e| e.return_mode)).unwrap_or(state.last_edit_mode);
    state.view_mode = match back {
        ViewMode::Sticker => ViewMode::View3D,
        m => m,
    };
    state.sticker_canvas_rect = None;
    state.sticker_ui.name_edit = None;
    state.sticker_ui.layer_name_edit = None;
    // The editor's status lines (Delete without a selection, a tool that is not built) end with it.
    state.status_hint.clear();
}

/// After every dispatched action (`dispatch::validate`): the Sticker view closes when its document or its stamp is gone
/// (a new document, an undo that removed the stamp), and its active layer follows the sticker (a deleted or undone
/// layer: the top one).
pub(crate) fn validate(app: &mut WteApp) {
    validate_in(&mut app.state);
}

/// `validate` on the state alone.
pub(crate) fn validate_in(state: &mut AppState) {
    if state.view_mode != ViewMode::Sticker {
        if state.session.as_ref().is_some_and(|s| s.sticker_edit.is_some()) {
            let mode = state.view_mode;
            leave_in(state, Some(mode));
        }
        return;
    }
    let Some(session) = state.session.as_mut() else {
        leave_in(state, None);
        return;
    };
    let Some(edit) = session.sticker_edit.clone() else {
        leave_in(state, None);
        return;
    };
    let Some(shown) = shown(&mut session.doc, &edit) else {
        leave_in(state, None);
        state.status_hint = "The sticker is gone (deleted or undone); back in the previous view".to_owned();
        return;
    };
    let layers: Vec<StickerLayerId> = shown.layers().map(|d| d.layers.iter().map(|l| l.id).collect()).unwrap_or_default();
    if let Some(e) = session.sticker_edit.as_mut() {
        if e.active_layer.is_none_or(|id| !layers.contains(&id)) {
            e.active_layer = layers.last().copied();
        }
        e.selected_layers.retain(|id| layers.contains(id));
    }
    if let Some(CanvasDrag::Move { layer, .. }) = &state.sticker_ui.drag
        && !layers.contains(layer)
    {
        state.sticker_ui.drag = None;
    }
}

/// Ends a drag on the canvas: a moved layer goes back (`cancel_pending_edit` of its Move entry).
pub(crate) fn cancel_drag(state: &mut AppState) {
    if let Some(CanvasDrag::Move { sent: Some(_), .. }) = state.sticker_ui.drag.take()
        && let Some(doc) = state.doc_mut()
    {
        doc.cancel_pending_edit();
    }
}

/// Runs a structural edit of the edited sticker (`Document::set_sticker`; an image stamp is promoted inside it). An
/// error becomes a warning toast. True when the edit was applied.
fn apply(state: &mut AppState, label: &str, commit: Commit, f: impl FnOnce(&mut StickerDoc)) -> bool {
    let Some(session) = state.session.as_mut() else { return false };
    let Some(r) = session.sticker_edit.as_ref().and_then(edit_ref) else { return false };
    if let StickerRef::OfStamp(stamp, _) = r
        && session.doc.stamp(stamp).is_some_and(|s| matches!(s.source, StampSource::Text(_)))
    {
        state.status_hint = "A text stamp is edited in the Text tab; convert it to a layered sticker first".to_owned();
        return false;
    }
    let r = session.doc.set_sticker(r, f, label, commit);
    crate::dispatch::edited(state, r).is_some()
}

/// The layered document of the open editor and the id of `layer` (the active layer for `None`).
fn layered(state: &mut AppState, layer: Option<StickerLayerId>) -> Option<(StickerDoc, StickerLayerId)> {
    let session = state.session.as_mut()?;
    let edit = session.sticker_edit.clone()?;
    let shown = shown(&mut session.doc, &edit)?;
    let doc = shown.layers()?.clone();
    let id = layer.or(edit.active_layer)?;
    doc.layer(id)?;
    Some((doc, id))
}

/// Sets the active layer of the open editor.
fn set_active(state: &mut AppState, id: StickerLayerId) {
    if let Some(e) = edit_mut(state) {
        e.active_layer = Some(id);
        e.selected_layers = vec![id];
    }
}

/// Index of layer `id` in `doc` (bottom = 0).
fn index_of(doc: &StickerDoc, id: StickerLayerId) -> Option<usize> {
    doc.layers.iter().position(|l| l.id == id)
}

/// A layer name without the file extension of the picture it came from.
fn picture_name(name: &str) -> String {
    let stem = std::path::Path::new(name).file_stem().map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned());
    if stem.trim().is_empty() { "Picture".to_owned() } else { stem }
}

/// Executes a `StickerAction` (dispatch, 8.3).
pub(crate) fn run(app: &mut WteApp, action: StickerAction, frame: &eframe::Frame) {
    use StickerAction as S;
    match action {
        S::Enter(stamp) => enter(app, stamp),
        S::Leave => leave(app, None),
        S::AddPictureFile(path) => {
            let path = crate::dispatch::resolve_path(app, path, |prefs| crate::io_flows::pick_image(frame, prefs));
            if let Some(path) = path
                && let Some((blob, _)) = crate::dispatch::add_stamp_image(app, &path)
            {
                add_picture_layer(&mut app.state, Some(blob));
            }
        }
        S::Escape => escape(app, frame),
        other => run_in(&mut app.state, other),
    }
}

/// Executes the `StickerAction`s that need only the state (all but Enter / Leave with a busy tool, the file dialog
/// and Esc, which `run` handles).
pub(crate) fn run_in(app_state: &mut AppState, action: StickerAction) {
    use StickerAction as S;
    match action {
        S::Enter(stamp) => enter_in(app_state, stamp),
        S::Leave => leave_in(app_state, None),
        S::AddPictureFile(_) | S::Escape => log::debug!("{action:?} needs the app; dropped"),
        S::Revert => {
            let state = &mut *app_state;
            let Some(session) = state.session.as_mut() else { return };
            let Some(edit) = session.sticker_edit.as_ref() else { return };
            let entry = edit.entry.clone();
            let r = session.doc.restore_sticker_state(&entry, REVERT_LABEL);
            crate::dispatch::edited(state, r);
        }
        S::SelectLayer(id) => {
            if layered(app_state, Some(id)).is_some() {
                set_active(app_state, id);
            }
        }
        S::AddPictureLayer(blob) => add_picture_layer(app_state, blob),
        S::DuplicateLayer(layer) => {
            let Some((doc, id)) = layered(app_state, layer) else { return };
            let copy = StickerLayerId(doc.next_layer);
            let done = apply(app_state, DUPLICATE_LABEL, Commit::Final, |d| {
                let Some(i) = index_of(d, id) else { return };
                let new_id = d.alloc_layer();
                let source = &d.layers[i];
                let layer = StickerLayer { id: new_id, name: format!("{} copy", source.name), locked: false, ..source.clone() };
                d.layers.insert(i + 1, layer);
            });
            if done {
                set_active(app_state, copy);
            }
        }
        S::DeleteLayer(layer) => {
            let Some((doc, id)) = layered(app_state, layer) else { return };
            let Some(i) = index_of(&doc, id) else { return };
            if doc.layers.len() <= 1 {
                app_state.status_hint = "A sticker keeps at least one layer".to_owned();
                return;
            }
            if doc.layers[i].locked {
                app_state.status_hint = "The layer is locked: unlock it first".to_owned();
                return;
            }
            if apply(app_state, DELETE_LABEL, Commit::Final, |d| d.layers.retain(|l| l.id != id)) {
                let next = doc.layers.get(i.saturating_sub(1)).filter(|l| l.id != id).or(doc.layers.get(i + 1)).map(|l| l.id);
                if let Some(next) = next {
                    set_active(app_state, next);
                }
            }
        }
        S::MoveLayer(layer, delta) => {
            let Some((doc, id)) = layered(app_state, layer) else { return };
            let Some(i) = index_of(&doc, id) else { return };
            let to = (i as i64 + i64::from(delta)).clamp(0, doc.layers.len() as i64 - 1) as usize;
            if to != i {
                move_layer_to(app_state, id, to);
            }
        }
        S::MoveLayerTo(id, to) => move_layer_to(app_state, id, to),
        S::MergeDown(layer) => {
            // Stickers P8: Merge Down of sticker layers; never a texture layer (STICKER_PLAN 3.3).
            if layered(app_state, layer).is_some() {
                app_state.status_hint = "Merge Down of sticker layers is not available yet".to_owned();
            }
        }
        S::SetLayerVisible(id, on) => {
            let label = if on { "Show Sticker Layer" } else { "Hide Sticker Layer" };
            layer_edit(app_state, id, label, Commit::Final, move |l| l.visible = on);
        }
        S::SetLayerLocked(id, on) => {
            let label = if on { "Lock Sticker Layer" } else { "Unlock Sticker Layer" };
            layer_edit(app_state, id, label, Commit::Final, move |l| l.locked = on);
        }
        S::SetLayerOpacity(id, opacity, commit) => {
            layer_edit(app_state, id, OPACITY_LABEL, commit, move |l| l.opacity = opacity);
        }
        S::SetLayerBlend(id, blend) => layer_edit(app_state, id, BLEND_LABEL, Commit::Final, move |l| l.blend = blend),
        S::RenameLayer(id, name) => {
            let name = name.trim().to_owned();
            if !name.is_empty() {
                layer_edit(app_state, id, RENAME_LABEL, Commit::Final, move |l| l.name = name);
            }
        }
        S::SetLayerPosition(id, [x, y], commit) => {
            if !(x.is_finite() && y.is_finite()) {
                return;
            }
            if layered(app_state, Some(id)).is_some_and(|(d, _)| d.layer(id).is_some_and(|l| l.locked)) {
                app_state.status_hint = "The layer is locked: unlock it first".to_owned();
                return;
            }
            layer_edit(app_state, id, MOVE_LABEL, commit, move |l| l.transform.translate = glam::Vec2::new(x, y));
        }
        S::Nudge(dx, dy) => {
            let Some((doc, id)) = layered(app_state, None) else { return };
            let Some(layer) = doc.layer(id) else { return };
            if layer.locked {
                app_state.status_hint = "The layer is locked: unlock it first".to_owned();
                return;
            }
            let d = glam::Vec2::new(dx as f32, dy as f32);
            layer_edit(app_state, id, NUDGE_LABEL, Commit::Final, move |l| l.transform.translate += d);
        }
        S::DeleteKey => {
            if edit_of(app_state).is_some_and(|e| e.selection.is_some()) {
                // Stickers P7b: clears the selected pixels of the active picture layer.
                app_state.status_hint = "Clearing a selection comes with the selection tools".to_owned();
            } else {
                app_state.status_hint = DELETE_HINT.to_owned();
            }
        }
        S::EnterKey => {}
        S::Fit => {
            if let Some(e) = edit_mut(app_state) {
                e.camera = Canvas2D::default();
            }
        }
        S::Zoom100 => app_state.sticker_ui.zoom_request = Some(1.0),
        S::ConvertText => {
            let state = &mut *app_state;
            let Some(session) = state.session.as_mut() else { return };
            let Some(StickerTarget::Stamp(stamp)) = session.sticker_edit.as_ref().map(|e| e.target) else { return };
            let r = session.doc.convert_text_stamp(stamp);
            if crate::dispatch::edited(state, r).is_some()
                && let Some(session) = state.session.as_mut()
            {
                let top = first_active(&session.doc, stamp);
                if let Some(e) = session.sticker_edit.as_mut() {
                    e.active_layer = top;
                    e.selected_layers = top.into_iter().collect();
                }
            }
        }
        S::Rename(name) => {
            let name = name.trim().to_owned();
            let is_sticker = app_state.session.as_mut().is_some_and(|s| {
                let Some(edit) = s.sticker_edit.clone() else { return false };
                matches!(shown(&mut s.doc, &edit), Some(Shown::Sticker { doc, .. }) if doc.name != name)
            });
            if is_sticker && !name.is_empty() {
                apply(app_state, RENAME_STICKER_LABEL, Commit::Final, move |d| d.name = name);
            }
        }
    }
}

/// Esc in the Sticker view (STICKER_PLAN 3.3), first match wins: a drag on the canvas is cancelled; a dialog, a tool
/// interaction of the 3D half or a pending slider edit go through the ordinary Cancel; the selection is dropped;
/// otherwise the view is left. (The first Esc in a text field only leaves the field: `shortcuts::handle`.)
fn escape(app: &mut WteApp, frame: &eframe::Frame) {
    if app.state.sticker_ui.drag.is_some() {
        cancel_drag(&mut app.state);
        return;
    }
    let dialog = !matches!(app.state.dialog, crate::state::Dialog::None);
    let pending = app.state.doc().is_some_and(Document::has_pending_edit);
    if dialog || pending || app.tools.is_busy() || app.jobs.modal_running() {
        crate::dispatch::dispatch(app, crate::action::AppAction::Cancel, frame);
        return;
    }
    if let Some(e) = edit_mut(&mut app.state)
        && e.selection.take().is_some()
    {
        return;
    }
    leave(app, None);
}

/// Changes one layer of the edited sticker (`set_sticker`; Preview calls fold by label).
fn layer_edit(state: &mut AppState, id: StickerLayerId, label: &str, commit: Commit, f: impl FnOnce(&mut StickerLayer)) {
    if layered(state, Some(id)).is_none() {
        return;
    }
    apply(state, label, commit, move |d| {
        if let Some(l) = d.layer_mut(id) {
            f(l);
        }
    });
}

/// Puts layer `id` at index `to` (bottom = 0; clamped), one entry.
fn move_layer_to(state: &mut AppState, id: StickerLayerId, to: usize) {
    let Some((doc, _)) = layered(state, Some(id)) else { return };
    let Some(from) = index_of(&doc, id) else { return };
    let to = to.min(doc.layers.len().saturating_sub(1));
    if to == from {
        return;
    }
    apply(state, REORDER_LABEL, Commit::Final, move |d| {
        if let Some(i) = index_of(d, id) {
            let layer = d.layers.remove(i);
            d.layers.insert(to.min(d.layers.len()), layer);
        }
    });
}

/// "New picture layer" (`None`: transparent, the artboard's size) or a picture layer of an image blob centred on the
/// artboard, above the active layer; the new layer becomes the active one.
fn add_picture_layer(state: &mut AppState, blob: Option<BlobId>) {
    let Some(session) = state.session.as_mut() else { return };
    let Some(edit) = session.sticker_edit.clone() else { return };
    let Some(shown) = shown(&mut session.doc, &edit) else { return };
    let Some(current) = shown.layers() else {
        state.status_hint = "Convert the text to a layered sticker first".to_owned();
        return;
    };
    let artboard = current.artboard;
    let new_id = StickerLayerId(current.next_layer);
    let above = edit.active_layer.and_then(|a| index_of(current, a));
    let (content, name, size, label) = match blob {
        Some(blob) => {
            let size = match session.doc.image_size(blob) {
                Ok(s) => s,
                Err(e) => {
                    crate::dispatch::edited::<()>(state, Err(e));
                    return;
                }
            };
            let name = session.doc.blobs.get(blob).map_or_else(|| "Picture".to_owned(), |b| picture_name(&b.name));
            (StickerContent::Picture(PicturePixels::Blob(blob)), name, size, ADD_LABEL)
        }
        None => {
            let [w, h] = [artboard.width().max(1), artboard.height().max(1)];
            let pixels = PicturePixels::Owned { pixels: Arc::new(PixelBuf::new(w, h)), version: 0 };
            (StickerContent::Picture(pixels), format!("Layer {}", current.layers.len() + 1), [w, h], NEW_LABEL)
        }
    };
    let centre = glam::Vec2::new(
        artboard.x0 as f32 + (artboard.width() as f32 - size[0] as f32) * 0.5,
        artboard.y0 as f32 + (artboard.height() as f32 - size[1] as f32) * 0.5,
    )
    .round();
    let done = apply(state, label, Commit::Final, move |d| {
        let id = d.alloc_layer();
        let mut layer = StickerLayer::new(id, &name, content);
        layer.transform.translate = centre;
        let at = above.map_or(d.layers.len(), |i| (i + 1).min(d.layers.len()));
        d.layers.insert(at, layer);
    });
    if done {
        set_active(state, new_id);
    }
}

#[cfg(test)]
pub(crate) mod tests;
