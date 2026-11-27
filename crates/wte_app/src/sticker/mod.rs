// Owner: stickers P6a (STICKER_PLAN 3.1-3.3, 3.5, 3.10; ARCHITECTURE 8.15). The Sticker view of the main window:
// entering and leaving it, the commands of `StickerAction` (each one `Document` command, so one history entry), the
// UI state that is not in the document (`StickerUi`), and the helpers the canvas (`canvas.rs`) and the panels
// (`layers_ui.rs`) share.
//
// What the editor edits (`StickerTarget`): the sticker of a stamp. An image stamp shows a virtual one-layer sticker
// built here from its picture (nothing in the document changes); its first content edit passes
// `StickerRef::OfStamp` and the core promotes it inside that edit (one undo step, STICKER_PLAN 2.10). A text stamp
// shows its picture read-only with the button "Convert to layered sticker".
//
// Stickers P6b: the editor also opens on a tray sticker (`StickerTarget::Tray`: "Edit" in the tray, "Edit before
// placing" of the Stamp tool); before the first content edit of a sticker or picture that several stamps show it asks
// "Edit all / Only this one" (E6, `Dialog::StickerShared`); the stamp-level and layer-level style commands, Copy /
// Paste / Clear Style (3.7) and the tray commands (3.8) are `StickerAction`s too (`styles_ui.rs`, `panels.rs`,
// `tray.rs` draw them).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use wte_core::{
    BlendMode, BlobId, Commit, Document, PicturePixels, PixelBuf, REVERT_LABEL, ShareScope, StampId,
    StampSource, StickerContent, StickerDoc, StickerId, StickerLayer, StickerLayerId, StickerRef, StylePreset, StyleStack,
};

use crate::app::WteApp;
use crate::state::{AppState, Canvas2D, Dialog, StickerEdit, StickerTarget, StickerTool, ViewMode};

pub mod canvas;
pub mod layers_ui;
pub mod panels;
pub mod styles_ui;
pub mod tray;

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
/// Stickers P6b: the history label of a sticker layer's styles (one entry per drag).
pub(crate) const LAYER_STYLE_LABEL: &str = "Sticker Layer Style";
/// The status line of the editor opened on a sticker no stamp shows yet (STICKER_PLAN 3.5).
pub(crate) const NOT_PLACED_HINT: &str = "Not placed yet: the model shows this sticker once you place it with the Stamp tool";

/// Stickers P6b (STICKER_PLAN 3.7): what a style command acts on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StyleTarget {
    /// The selected stamps: the selected stamp and every stamp linked to the selected layers of the Layers list.
    Stamps,
    /// In the Sticker view: the selected sticker layers (Copy: the active one).
    Layers,
}

/// Stickers P6b (STICKER_PLAN 3.8): what Export Picture... writes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PictureOf {
    /// The finished picture of a stamp, with its styles ("what you see").
    Stamp(StampId),
    /// A tray sticker at render scale 1, without stamp styles.
    Sticker(StickerId),
}

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
    /// The Layer menu's Rasterize in the view (P6a check): rasterizing text / shape layers comes with P8; never the
    /// stamp's texture layer.
    Rasterize,
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

    // ---- stickers P6b ----
    /// "Edit" on a tray item: opens the Sticker view on that sticker.
    EnterTray(StickerId),
    /// The Stamp tool's "Edit before placing" (E4): an image source becomes a tray sticker first; the view opens on it.
    EditBeforePlacing,
    /// The answer to the shared-picture question (E6): `Some(AllUsers)` = Edit all, `Some(ThisStamp)` = Only this
    /// one, `None` = Cancel (the waiting edit is dropped).
    AnswerShared(Option<ShareScope>),
    /// The stamp-level styles of these stamps (the Sticker tab's editor; one entry per drag; E3: never promotes).
    SetStampStyles(Vec<StampId>, Box<StyleStack>, Commit),
    /// A preset on the selected stamps, scaled to each stamp's picture (one entry).
    ApplyPreset(StylePreset),
    /// The styles of a sticker layer (the Tool tab's Layer styles in the Sticker view; a content edit).
    SetLayerStyles(StickerLayerId, Box<StyleStack>, Commit),
    /// A preset on the active sticker layer, scaled to the artboard.
    ApplyLayerPreset(StylePreset),
    /// Copy Style / Paste Style / Clear Style (3.7; the clipboard is `AppState::style_clipboard`).
    CopyStyle(StyleTarget),
    PasteStyle(StyleTarget),
    ClearStyle(StyleTarget),
    /// "New Sticker" of the tray: a blank 512 x 512 sticker with one empty picture layer, kept in the tray.
    NewSticker,
    /// "New from picture...": the picture as a one-layer tray sticker (`None`: the file dialog, never in a self-test).
    NewStickerFromPicture(Option<PathBuf>),
    /// A click on a tray item: it becomes the Stamp tool's source (and the Stamp tool the tool, outside the view).
    TrayUse(StickerId),
    TrayDuplicate(StickerId),
    /// Only a sticker no stamp shows.
    TrayDelete(StickerId),
    /// "Export Picture...": a PNG (`None`: the save dialog, never in a self-test).
    ExportPicture(PictureOf, Option<PathBuf>),
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
            | S::Rename(_)
            | S::EditBeforePlacing
            | S::ApplyPreset(_)
            | S::ApplyLayerPreset(_)
            | S::PasteStyle(_)
            | S::ClearStyle(_)
            | S::NewSticker
            | S::NewStickerFromPicture(_)
            | S::TrayDuplicate(_)
            | S::TrayDelete(_)
            | S::AnswerShared(Some(_)) => true,
            S::SetStampStyles(_, _, c) | S::SetLayerStyles(_, _, c) => *c == Commit::Final,
            _ => false,
        }
    }

    /// Stickers P6b (E6): the commands that change the content of the edited sticker (they ask the shared-picture
    /// question first). Style commands on stamps are no content edits (they never promote, E3).
    pub(crate) fn is_content(&self) -> bool {
        use StickerAction as S;
        matches!(
            self,
            S::AddPictureLayer(_)
                | S::DuplicateLayer(_)
                | S::DeleteLayer(_)
                | S::MoveLayer(..)
                | S::MoveLayerTo(..)
                | S::SetLayerVisible(..)
                | S::SetLayerLocked(..)
                | S::SetLayerOpacity(..)
                | S::SetLayerBlend(..)
                | S::RenameLayer(..)
                | S::SetLayerPosition(..)
                | S::Nudge(..)
                | S::Rename(_)
                | S::SetLayerStyles(..)
                | S::ApplyLayerPreset(_)
                | S::PasteStyle(StyleTarget::Layers)
                | S::ClearStyle(StyleTarget::Layers)
        )
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
    /// Stickers P6b: the content edit waiting for the answer to the shared-picture question (E6).
    pub(crate) shared_pending: Option<StickerAction>,
    /// The tray item last clicked (its buttons act on it).
    pub(crate) tray_selected: Option<StickerId>,
    /// Thumbnails of the Sticker tab (tray items and the selected stamp's picture) and the document they belong to.
    pub(crate) pictures: tray::PictureThumbs,
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
            shared_pending: None,
            tray_selected: None,
            pictures: tray::PictureThumbs::default(),
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
    let mut stickers: Vec<StickerId> = stamps
        .iter()
        .filter_map(|s| match s.source {
            StampSource::Sticker { sticker } => Some(sticker),
            _ => None,
        })
        .collect();
    // Stickers P6b: a tray sticker has no stamp in the checkpoint.
    if let StickerTarget::Tray(id) = edit.target {
        stickers.push(id);
    }
    let sticker_changed = stickers.into_iter().any(|id| match (doc.sticker(id), edit.entry.sticker(id)) {
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
    let Some(session) = state.session.as_mut() else { return };
    let Some(stamp) = stamp.or(session.selected_stamp).filter(|id| session.doc.stamp(*id).is_some()) else {
        state.status_hint = NO_STAMP_HINT.to_owned();
        return;
    };
    enter_target(state, StickerTarget::Stamp(stamp));
}

/// Opens the Sticker view on `target` (a stamp that exists, or a sticker of the document).
pub(crate) fn enter_target(state: &mut AppState, target: StickerTarget) {
    cancel_drag(state);
    let Some(session) = state.session.as_mut() else { return };
    let exists = match target {
        StickerTarget::Stamp(stamp) => session.doc.stamp(stamp).is_some(),
        StickerTarget::Tray(id) => session.doc.sticker(id).is_some(),
    };
    if !exists {
        return;
    }
    if state.view_mode == ViewMode::Sticker && session.sticker_edit.as_ref().is_some_and(|e| e.target == target) {
        return;
    }
    session.doc.flush_pending_edit();
    let return_mode = match (&session.sticker_edit, state.view_mode) {
        (Some(e), ViewMode::Sticker) => e.return_mode,
        (None, ViewMode::Sticker) => state.last_edit_mode,
        (_, mode) => mode,
    };
    let tool = session.sticker_edit.as_ref().map_or(state.sticker_ui.last_tool, |e| e.tool);
    let (entry, active_layer) = match target {
        StickerTarget::Stamp(stamp) => (session.doc.sticker_checkpoint(&[stamp], &[]), first_active(&session.doc, stamp)),
        StickerTarget::Tray(id) => {
            (session.doc.sticker_checkpoint(&[], &[id]), session.doc.sticker(id).and_then(|d| d.layers.last()).map(|l| l.id))
        }
    };
    let unplaced = matches!(target, StickerTarget::Tray(id) if session.doc.sticker_users(id).is_empty());
    session.sticker_edit = Some(StickerEdit {
        target,
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
    state.sticker_ui.shared_pending = None;
    match target {
        StickerTarget::Stamp(stamp) => crate::dispatch::select_stamp(state, Some(stamp)),
        StickerTarget::Tray(id) => {
            state.sticker_ui.tray_selected = Some(id);
            if unplaced {
                state.status_hint = NOT_PLACED_HINT.to_owned();
            }
        }
    }
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
    state.sticker_ui.shared_pending = None;
    state.sticker_ui.name_edit = None;
    state.sticker_ui.layer_name_edit = None;
    // P6a check: the canvas texture (a whole sticker at scale 1, up to the artboard limit) and the layer thumbnails
    // are freed with the view. Their keys hold picture versions, which are numbered per document
    // (`Document::next_pixel_version`): kept across a document change they could match another document's sticker.
    state.sticker_ui.canvas = None;
    state.sticker_ui.thumbs.clear();
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

/// P6a check: ends a drag on the canvas keeping what the user sees: a moved layer stays where the last Preview put
/// it, as one entry (the dispatch guard for Save / Export, like `ToolController::finish`).
pub(crate) fn finish_drag(state: &mut AppState) {
    if let Some(CanvasDrag::Move { layer, sent: Some(to), .. }) = state.sticker_ui.drag.take() {
        run_in(state, StickerAction::SetLayerPosition(layer, to.to_array(), Commit::Final));
    }
}

/// P6a check: true while a Move drag on the canvas has changed the document (a Preview is pending): an Undo pressed
/// then undoes the drag itself, as for the tools' interactions.
pub(crate) fn drag_moved(state: &AppState) -> bool {
    matches!(state.sticker_ui.drag, Some(CanvasDrag::Move { sent: Some(_), .. }))
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
                // Through `run_in`: the shared-picture question (E6) comes first.
                run_in(&mut app.state, S::AddPictureLayer(Some(blob)));
            }
        }
        S::NewStickerFromPicture(path) => {
            let path = crate::dispatch::resolve_path(app, path, |prefs| crate::io_flows::pick_image(frame, prefs));
            if let Some(path) = path
                && let Some((blob, name)) = crate::dispatch::add_stamp_image(app, &path)
            {
                tray::new_from_blob(&mut app.state, blob, &picture_name(&name));
            }
        }
        S::ExportPicture(what, path) => tray::export_picture(app, what, path, frame),
        S::Escape => escape(app, frame),
        other => run_in(&mut app.state, other),
    }
}

/// Executes the `StickerAction`s that need only the state (all but Enter / Leave with a busy tool, the file dialog
/// and Esc, which `run` handles).
pub(crate) fn run_in(app_state: &mut AppState, action: StickerAction) {
    use StickerAction as S;
    // Stickers P6b (E6): a content edit of a sticker or picture that several stamps show waits for the answer.
    if action.is_content()
        && !is_no_op(app_state, &action)
        && let Some(users) = shared_users(app_state)
    {
        ask_shared(app_state, action, users);
        return;
    }
    match action {
        S::Enter(stamp) => enter_in(app_state, stamp),
        S::Leave => leave_in(app_state, None),
        S::AddPictureFile(_) | S::Escape | S::NewStickerFromPicture(_) | S::ExportPicture(..) => {
            log::debug!("{action:?} needs the app; dropped");
        }
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
        S::Rasterize => {
            if layered(app_state, None).is_some() {
                app_state.status_hint = "Rasterize of sticker layers is not available yet".to_owned();
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
            // P6a check: the Name field sends its text whenever it loses the focus; an unchanged name is no edit (on an
            // image stamp it would promote the stamp and add an empty "Rename Sticker Layer" entry).
            let same = layered(app_state, Some(id)).is_some_and(|(d, _)| d.layer(id).is_some_and(|l| l.name == name));
            if !name.is_empty() && !same {
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
        S::EnterTray(id) => enter_target(app_state, StickerTarget::Tray(id)),
        S::EditBeforePlacing => tray::edit_before_placing(app_state),
        S::AnswerShared(answer) => answer_shared(app_state, answer),
        S::SetStampStyles(stamps, styles, commit) => {
            let r = app_state.doc_mut().map(|d| d.set_styles_of(&stamps, &styles, commit));
            if let Some(r) = r {
                crate::dispatch::edited(app_state, r);
            }
        }
        S::ApplyPreset(preset) => apply_preset(app_state, preset),
        S::SetLayerStyles(id, styles, commit) => {
            layer_edit(app_state, id, LAYER_STYLE_LABEL, commit, move |l| l.styles = *styles);
        }
        S::ApplyLayerPreset(preset) => {
            let Some((doc, id)) = layered(app_state, None) else { return };
            let styles = preset.styles(doc.size());
            layer_edit(app_state, id, LAYER_STYLE_LABEL, Commit::Final, move |l| l.styles = styles);
        }
        S::CopyStyle(target) => copy_style(app_state, target),
        S::PasteStyle(target) => {
            let Some(styles) = app_state.style_clipboard.clone() else {
                app_state.status_hint = "Copy a style first".to_owned();
                return;
            };
            put_style(app_state, target, styles, "Paste Layer Style");
        }
        S::ClearStyle(target) => put_style(app_state, target, StyleStack::default(), "Clear Layer Style"),
        S::NewSticker => tray::new_blank(app_state),
        S::TrayUse(id) => tray::use_as_source(app_state, id),
        S::TrayDuplicate(id) => tray::duplicate(app_state, id),
        S::TrayDelete(id) => tray::delete(app_state, id),
    }
}

/// Stickers P6b (E6): how many stamps show the sticker (or the image) of the open editor, when that is more than one
/// and the question was not answered in this opening of the editor; `None` otherwise (a tray sticker, a text stamp).
pub(crate) fn shared_users(state: &AppState) -> Option<usize> {
    let session = state.session.as_ref()?;
    let edit = session.sticker_edit.as_ref()?;
    if edit.share_answer.is_some() {
        return None;
    }
    let StickerTarget::Stamp(stamp) = edit.target else { return None };
    let users = match session.doc.stamp(stamp)?.source {
        StampSource::Sticker { sticker } => session.doc.sticker_users(sticker).len(),
        StampSource::Image { blob } => session.doc.blob_users(blob).len(),
        StampSource::Text(_) => return None,
    };
    (users > 1).then_some(users)
}

/// True for a content command that would change nothing (a name field that lost the focus with the same text): it
/// must not ask the shared-picture question.
fn is_no_op(state: &mut AppState, action: &StickerAction) -> bool {
    match action {
        StickerAction::RenameLayer(id, name) => {
            let name = name.trim();
            name.is_empty() || layered(state, Some(*id)).is_some_and(|(d, _)| d.layer(*id).is_some_and(|l| l.name == name))
        }
        StickerAction::Rename(name) => {
            let name = name.trim().to_owned();
            name.is_empty()
                || state.session.as_mut().is_none_or(|s| {
                    let Some(edit) = s.sticker_edit.clone() else { return true };
                    !matches!(shown(&mut s.doc, &edit), Some(Shown::Sticker { doc, .. }) if doc.name != name)
                })
        }
        _ => false,
    }
}

/// Opens the shared-picture question for `action` (a newer content edit replaces the waiting one). A drag on the
/// canvas ends: nothing of it was applied.
fn ask_shared(state: &mut AppState, action: StickerAction, users: usize) {
    state.sticker_ui.drag = None;
    state.sticker_ui.shared_pending = Some(action);
    if matches!(state.dialog, Dialog::None | Dialog::StickerShared { .. }) {
        state.dialog = Dialog::StickerShared { users };
    }
}

/// The answer to the shared-picture question (E6). Edit all: the edit reaches every stamp (an image stamp is promoted
/// with all the stamps that show its picture). Only this one: a sticker stamp first gets its own copy
/// (`detach_sticker`, one entry), an image stamp is promoted alone. Cancel drops the waiting edit. The answer holds
/// until the editor closes. The waiting edit runs after the answer, a Preview as a Final (its drag is over); a drag of
/// the Move tool is not replayed (the user drags again).
fn answer_shared(state: &mut AppState, answer: Option<ShareScope>) {
    if matches!(state.dialog, Dialog::StickerShared { .. }) {
        state.dialog = Dialog::None;
    }
    let pending = state.sticker_ui.shared_pending.take();
    let Some(scope) = answer else { return };
    let Some(session) = state.session.as_mut() else { return };
    let Some(edit) = session.sticker_edit.as_mut() else { return };
    edit.share_answer = Some(scope);
    let StickerTarget::Stamp(stamp) = edit.target else { return };
    if scope == ShareScope::ThisStamp && session.doc.stamp(stamp).is_some_and(|s| matches!(s.source, StampSource::Sticker { .. })) {
        let r = session.doc.detach_sticker(stamp);
        if crate::dispatch::edited(state, r).is_none() {
            return;
        }
    }
    let replay = match pending {
        Some(StickerAction::SetLayerPosition(_, _, Commit::Preview)) | None => None,
        Some(StickerAction::SetLayerOpacity(id, v, _)) => Some(StickerAction::SetLayerOpacity(id, v, Commit::Final)),
        Some(StickerAction::SetLayerStyles(id, v, _)) => Some(StickerAction::SetLayerStyles(id, v, Commit::Final)),
        Some(other) => Some(other),
    };
    if let Some(action) = replay {
        run_in(state, action);
    }
}

/// Stickers P6b (3.7): the stamps the Sticker tab's style commands act on: the selected stamp and every stamp linked to
/// the selected layers of the Layers list; the selected stamp first (its values are the ones shown).
pub(crate) fn selected_stamps(state: &AppState) -> Vec<StampId> {
    let Some(session) = state.session.as_ref() else { return Vec::new() };
    let mut out: Vec<StampId> = session.selected_stamp.filter(|id| session.doc.stamp(*id).is_some()).into_iter().collect();
    for layer in session.layer_selection() {
        if let Some(stamp) = session.doc.layer(layer).and_then(|l| l.stamp_id())
            && !out.contains(&stamp)
            && session.doc.stamp(stamp).is_some()
        {
            out.push(stamp);
        }
    }
    out
}

/// A preset on the selected stamps, scaled to each stamp's picture; one entry.
fn apply_preset(state: &mut AppState, preset: StylePreset) {
    let stamps = selected_stamps(state);
    if stamps.is_empty() {
        state.status_hint = "Select a stamp or a text first".to_owned();
        return;
    }
    let Some(doc) = state.doc_mut() else { return };
    let mut each = Vec::with_capacity(stamps.len());
    for id in stamps {
        let size = doc.stamp_style_reference_size(id).unwrap_or([512, 512]);
        each.push((id, preset.styles(size)));
    }
    let r = doc.set_styles_each(&each, Commit::Final);
    crate::dispatch::edited(state, r);
}

/// Copy Style: the active sticker layer's styles (`Layers`) or the first selected stamp's (`Stamps`).
fn copy_style(state: &mut AppState, target: StyleTarget) {
    let styles = match target {
        StyleTarget::Layers => layered(state, None).and_then(|(d, id)| d.layer(id).map(|l| l.styles.clone())),
        StyleTarget::Stamps => {
            let first = selected_stamps(state).first().copied();
            first.and_then(|id| state.doc().and_then(|d| d.stamp(id)).map(|s| s.styles.clone()))
        }
    };
    match styles {
        Some(styles) => {
            state.style_clipboard = Some(styles);
            state.status_hint = "Style copied".to_owned();
        }
        None => state.status_hint = "Select a stamp or a layer first".to_owned(),
    }
}

/// Paste Style / Clear Style: `styles` on every selected stamp (one entry, `set_stamps_styles`) or on the selected
/// sticker layers (one entry `label`).
fn put_style(state: &mut AppState, target: StyleTarget, styles: StyleStack, label: &str) {
    match target {
        StyleTarget::Stamps => {
            let stamps = selected_stamps(state);
            if stamps.is_empty() {
                state.status_hint = "Select a stamp or a text first".to_owned();
                return;
            }
            let r = state.doc_mut().map(|d| d.set_stamps_styles(&stamps, &styles));
            if let Some(r) = r {
                crate::dispatch::edited(state, r);
            }
        }
        StyleTarget::Layers => {
            let Some((doc, active)) = layered(state, None) else { return };
            let mut ids: Vec<StickerLayerId> = edit_of(state).map(|e| e.selected_layers.clone()).unwrap_or_default();
            ids.retain(|id| doc.layer(*id).is_some());
            if ids.is_empty() {
                ids.push(active);
            }
            apply(state, label, Commit::Final, move |d| {
                for l in d.layers.iter_mut().filter(|l| ids.contains(&l.id)) {
                    l.styles = styles.clone();
                }
            });
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
#[cfg(test)]
mod tests_p6b;
