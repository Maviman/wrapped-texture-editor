// Owner: app-shell (A1). `dispatch` of ARCHITECTURE.md 8.3: the single mutation point for commands and the only
// caller of the native file dialogs (through the `io_flows::pick_*` helpers).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use wte_core::{
    Commit, DocError, EditError, ExportOptions, FlatFormat, LayerId, PngLevel, SourceFile, StampSource, TextureChoice,
};

use crate::action::AppAction;
use crate::app::WteApp;
use crate::dnd::{MODEL_EXTENSIONS, extension_of, is_image_path};
use crate::io_flows;
use crate::object_ops;
use crate::state::{
    AppState, CameraPreset, Dialog, ExportDialogState, OrbitCamera, Pending, SetChoiceUi, ShadingMode, ToolKind,
    UvViewState, ViewId, ViewMode,
};

/// JPEG quality preset of a freshly opened Export dialog.
const DEFAULT_JPEG_QUALITY: u8 = 90;
/// Brush size limits (diameter in texels, 8.2).
const BRUSH_SIZE_RANGE: (f32, f32) = (1.0, 2048.0);

/// Executes one action: the single mutation point for commands and the only caller of native file dialogs
/// (rules in section 8.3).
pub fn dispatch(app: &mut WteApp, action: AppAction, frame: &eframe::Frame) {
    log::debug!("dispatch {action:?}");
    crate::crash::note_action(&action);
    // Interaction guard: a command never meets an open stroke or a half-finished gizmo drag. Save and Export finish
    // it (the file gets what the user sees); Undo discards it (round 4: also a released brush stroke that is still
    // being painted, FEATURE_BRUSH 5.10); every other command cancels it, which completes such a finishing stroke.
    // Stickers P6a: a drag on the sticker canvas is no `ToolController` interaction; a document command ends it the
    // same way (the moved layer goes back; its own Final at the release comes after the drag has ended).
    // P6a check: like the tools' interactions, Save and Export finish a moved layer where it is shown, and an Undo
    // pressed during a drag that moved something undoes that drag only (it was cancelled and undid the entry before
    // it as well).
    if needs_interaction_guard(&action) && app.state.sticker_ui.busy() {
        let moved = crate::sticker::drag_moved(&app.state);
        if finishes_interaction(&action) {
            crate::sticker::finish_drag(&mut app.state);
        } else {
            crate::sticker::cancel_drag(&mut app.state);
        }
        if moved && matches!(action, AppAction::Undo) {
            validate(app);
            return;
        }
    }
    if needs_interaction_guard(&action) && app.tools.is_busy() {
        if finishes_interaction(&action) {
            app.tools.finish(&mut app.state);
        } else if matches!(action, AppAction::Undo) {
            app.tools.discard(&mut app.state);
        } else {
            app.tools.cancel(&mut app.state);
        }
        if matches!(action, AppAction::Undo) {
            // The interrupted interaction is the thing that was undone.
            validate(app);
            return;
        }
    }
    run(app, action, frame);
    validate(app);
}

/// True for the document-changing actions of the groups file, edit, sets and layers, and stamps — except
/// `SetActiveSet`, `SelectLayer`, `SelectStamp` and the `Commit::Preview` variants (8.3).
fn needs_interaction_guard(action: &AppAction) -> bool {
    use AppAction as A;
    match action {
        A::NewFromModel(_)
        | A::OpenProject(_)
        | A::OpenSample(_)
        | A::OpenPath(_)
        | A::Save
        | A::SaveAs(_)
        | A::ShowExport
        | A::ShowExportLayers(_)
        | A::RunExport(_)
        | A::ExtractSources(_)
        | A::Quit
        | A::ConfirmPending(_)
        | A::SaveThen(_)
        | A::ImportCreate
        | A::Undo
        | A::Redo
        | A::DeleteSelection
        | A::AddPaintLayer
        | A::DuplicateLayer(_)
        | A::DeleteLayer(_)
        | A::DuplicateLayers(_)
        | A::DeleteLayers(_)
        | A::MergeDown(_)
        | A::RasterizeLayer(_)
        | A::MoveLayer { .. }
        | A::SetLayerVisible(..)
        | A::SetLayerLocked(..)
        | A::SetLayerAlphaLock(..)
        | A::SetLayerBlend(..)
        | A::RenameLayer(..)
        | A::DuplicateActiveLayer
        | A::MergeActiveDown
        | A::RasterizeActiveLayer
        | A::MoveActiveLayer(_)
        | A::ImportStampImage(_)
        | A::ReplaceStampImage(..)
        | A::DeleteStamp(_)
        // Stickers P10a: a drop adds stamps, layers or stickers.
        | A::DropReady(_)
        // Round 4: these start a modal bake that takes the pointer away from a held stroke (FEATURE_BRUSH 7.3).
        | A::LoadBrushShape(_)
        | A::AnswerOpenings(Some(_))
        | A::SetBrushShapeClose(_)
        // objects-tabs: a visibility change needs no open stroke (`Document::set_hidden_objects` refuses one).
        | A::SetObjectVisible(..)
        | A::HideSelectedObjects
        | A::IsolateSelectedObjects
        | A::ShowAllObjects => true,
        A::SetLayerOpacity(_, _, commit)
        | A::UpdateStampSource(_, _, commit)
        | A::UpdateStampPlacement(_, _, commit)
        | A::UpdateStampTint(_, _, commit) => *commit == Commit::Final,
        // Stickers P6a: the editor's document commands.
        A::Sticker(s) => s.changes_document(),
        // Stickers P9a: the link commands that change the document (an update, a promotion, a send).
        A::Link(l) => l.changes_document(),
        _ => false,
    }
}

/// The guarded actions that commit a running stroke or gizmo drag (`ToolController::finish`) instead of cancelling it:
/// Save, Save As and Export write what is on screen at that moment; showing or hiding objects (objects-tabs) is view
/// state and never throws away what the user painted.
fn finishes_interaction(action: &AppAction) -> bool {
    // Stickers P9a: "Edit in" and "Send to" write the file from what is on screen, like Save.
    if let AppAction::Link(l) = action {
        return l.finishes_interaction();
    }
    matches!(
        action,
        AppAction::Save
            | AppAction::SaveAs(_)
            | AppAction::SaveThen(_)
            | AppAction::ShowExport
            | AppAction::ShowExportLayers(_)
            | AppAction::RunExport(_)
            | AppAction::SetObjectVisible(..)
            | AppAction::HideSelectedObjects
            | AppAction::IsolateSelectedObjects
            | AppAction::ShowAllObjects
    )
}

/// Makes `layer` the active layer of its set (and its set the active set) and the only selected layer; the stamp
/// selection follows the layer (8.2). No history entry, no revision change.
fn activate_layer(state: &mut AppState, layer: LayerId) {
    let Some(session) = state.session.as_mut() else { return };
    let Some((set, _)) = session.doc.find_layer(layer) else { return };
    session.active_set = set;
    let r = session.doc.set_active_layer(set, Some(layer));
    session.selected_stamp = session.doc.stamp_of_layer(layer);
    session.selected_layers = vec![layer];
    edited(state, r);
}

/// Ctrl-click on a layer row (8.7): a layer of another set (or no active layer) is simply activated; otherwise the
/// layer is added to the selection, or taken out of it. The last selected layer cannot be taken out; taking out the
/// active layer makes the top-most remaining selected layer the active one.
fn toggle_layer_selected(state: &mut AppState, layer: LayerId) {
    let Some(session) = state.session.as_mut() else { return };
    let Some((set, _)) = session.doc.find_layer(layer) else { return };
    let mut selection = session.layer_selection();
    let active = session.doc.set(set).and_then(|s| s.active);
    if set != session.active_set || selection.is_empty() {
        activate_layer(state, layer);
        return;
    }
    if !selection.contains(&layer) {
        selection.push(layer);
        session.selected_layers = selection;
        return;
    }
    if selection.len() == 1 {
        return;
    }
    selection.retain(|id| *id != layer);
    if active == Some(layer) {
        // Bottom to top: the last one is the top-most.
        let Some(&top) = selection.last() else { return };
        let r = session.doc.set_active_layer(set, Some(top));
        session.selected_stamp = session.doc.stamp_of_layer(top);
        session.selected_layers = selection;
        edited(state, r);
    } else {
        session.selected_layers = selection;
    }
}

/// Shift-click on a layer row (8.7): selects every layer between the active layer and `layer` (both included); the
/// active layer stays. A layer of another set (or no active layer) is simply activated.
fn select_layer_range(state: &mut AppState, layer: LayerId) {
    let Some(session) = state.session.as_mut() else { return };
    let Some((set_id, _)) = session.doc.find_layer(layer) else { return };
    let range = session.doc.set(set_id).and_then(|set| {
        let active = set.active?;
        let (a, b) = (set.layer_index(active)?, set.layer_index(layer)?);
        Some(set.layers[a.min(b)..=a.max(b)].iter().map(|l| l.id).collect::<Vec<LayerId>>())
    });
    match range {
        Some(ids) if set_id == session.active_set => session.selected_layers = ids,
        _ => activate_layer(state, layer),
    }
}

/// Duplicates a layer; the selection follows the copy, which the core makes the active layer: a stamp or text copy
/// becomes the selected stamp (Transform, the property fields and Delete then act on the copy the Layers list
/// highlights, not on the original underneath it), a paint-layer copy clears the stamp selection.
fn duplicate_layer(state: &mut AppState, layer: LayerId) {
    let Some(session) = state.session.as_mut() else { return };
    match session.doc.duplicate_layer(layer) {
        Ok(copy) => {
            if let Some((set, _)) = session.doc.find_layer(copy) {
                session.active_set = set;
            }
            session.selected_stamp = session.doc.stamp_of_layer(copy);
        }
        Err(e) => report_edit_error(state, &e),
    }
}

/// fix_export item 5: Duplicate on several selected layers, one undo step ("Duplicate Layers"). The copies become the
/// selection; the copy of the active layer is the active one (and the selected stamp when it is a stamp's).
fn duplicate_layers(state: &mut AppState, layers: &[LayerId]) {
    let Some(session) = state.session.as_mut() else { return };
    match session.doc.duplicate_layers(layers) {
        Ok(copies) => {
            let active = session.doc.set(session.active_set).and_then(|s| s.active);
            session.selected_layers = copies;
            session.selected_stamp = active.and_then(|id| session.doc.stamp_of_layer(id));
        }
        Err(e) => report_edit_error(state, &e),
    }
}

/// fix_export item 5: Delete on several selected layers, one undo step ("Delete Layers"). A locked layer stays (a
/// warning says how many); a stamp or text layer takes its stamp along, as Delete on one layer does. The selection
/// collapses to the new active layer.
fn delete_layers(state: &mut AppState, layers: &[LayerId]) {
    let Some(session) = state.session.as_mut() else { return };
    match session.doc.remove_layers(layers) {
        Ok(removed) => {
            session.selected_layers.clear();
            if session.selected_stamp.is_some_and(|id| session.doc.stamp(id).is_none()) {
                session.selected_stamp = None;
            }
            let kept = layers.len().saturating_sub(removed);
            if kept > 0 {
                let what = if kept == 1 { "1 locked layer was kept".to_owned() } else { format!("{kept} locked layers were kept") };
                state.toasts.warn(format!("Deleted {removed} of {} layers: {what}", layers.len()));
            }
        }
        Err(e) => report_edit_error(state, &e),
    }
}

/// Rasterizes a stamp or text layer. A paint layer has nothing to rasterize: the button and the menu entry are disabled
/// for it, and an action that reaches this point anyway (a queued action, the fuzzer) does nothing and shows nothing
/// (R3-8; the core would reject it with an "invalid parameter" text).
fn rasterize_layer(state: &mut AppState, layer: LayerId) {
    let Some(doc) = state.doc_mut() else { return };
    if doc.layer(layer).is_some_and(|l| l.is_raster()) {
        log::debug!("rasterize {layer:?}: already a paint layer, nothing to do");
        return;
    }
    let r = doc.rasterize_layer(layer);
    edited(state, r);
}

/// Round 5: before merge down and rasterize keep a stamp's pixels, the waiting Final bakes land
/// (`stamp_bakes::settle`). False when the layer is gone after it (the bake removed an empty linked layer outside the
/// stamp's home set, which the synchronous bake would have removed already): the command then has nothing to do.
fn settle_keeps_layer(app: &mut WteApp, layer: LayerId) -> bool {
    crate::stamp_bakes::settle(app);
    let exists = app.state.doc().is_some_and(|d| d.layer(layer).is_some());
    if !exists {
        log::debug!("{layer:?} is gone after the final bakes landed; nothing to merge or rasterize");
    }
    exists
}

fn validate(app: &mut WteApp) {
    if let Some(session) = app.state.session.as_mut() {
        session.validate();
    }
    // Stickers P6a: the Sticker view follows its stamp and its layers.
    crate::sticker::validate(app);
}

/// Stickers P6a (STICKER_PLAN 3.3): in the Sticker view the layer commands of the Layer and Edit menus act on the
/// active sticker layer, never on a texture layer, and Delete never deletes the stamp.
pub(crate) fn in_sticker_view(action: AppAction) -> AppAction {
    use crate::sticker::StickerAction as S;
    match action {
        AppAction::DeleteSelection => AppAction::Sticker(S::DeleteKey),
        // P6a check: the Layer menu's Delete names the active texture layer, which entering the view made the edited
        // stamp's own layer: run there it deleted the stamp. The menu's other texture-layer commands are mapped too.
        AppAction::DeleteLayer(_) | AppAction::DeleteLayers(_) => AppAction::Sticker(S::DeleteLayer(None)),
        AppAction::DuplicateActiveLayer | AppAction::DuplicateLayer(_) | AppAction::DuplicateLayers(_) => {
            AppAction::Sticker(S::DuplicateLayer(None))
        }
        AppAction::MergeActiveDown | AppAction::MergeDown(_) => AppAction::Sticker(S::MergeDown(None)),
        AppAction::MoveActiveLayer(delta) => AppAction::Sticker(S::MoveLayer(None, delta)),
        AppAction::AddPaintLayer => AppAction::Sticker(S::AddPictureLayer(None)),
        // Rasterize of sticker layers comes with P8 (it says so); the stamp's layer is never rasterized from here.
        AppAction::RasterizeActiveLayer => AppAction::Sticker(S::Rasterize),
        other => other,
    }
}

/// `tools.cancel` when an interaction is running (a no-op otherwise).
fn cancel_tool(app: &mut WteApp) {
    if app.tools.is_busy() {
        app.tools.cancel(&mut app.state);
    }
}

/// Turns a rejected core command into a warning toast; `Some(value)` on success.
pub(crate) fn edited<T>(state: &mut AppState, result: Result<T, EditError>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            report_edit_error(state, &e);
            None
        }
    }
}

/// Reports a rejected core command (R3-8): a warning toast in plain words, except for a command whose layer, stamp
/// or set no longer exists (an action queued before an undo or a delete removed its target), which is dropped with a
/// debug log line: the user did nothing wrong and there is nothing to act on.
fn report_edit_error(state: &mut AppState, e: &EditError) {
    if is_stale_target(e) {
        log::debug!("dropped an action whose target no longer exists: {e}");
    } else {
        state.toasts.warn(sentence(&e.to_string()));
    }
}

/// True for the errors of a command whose target (layer, stamp, set) is gone.
fn is_stale_target(e: &EditError) -> bool {
    matches!(e, EditError::UnknownLayer | EditError::UnknownStamp | EditError::UnknownSet)
}

/// Capitalizes the first letter of an error text for a toast.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The active layer of the active set.
fn active_layer(state: &AppState) -> Option<LayerId> {
    let session = state.session.as_ref()?;
    session.doc.set(session.active_set)?.active
}

/// True (and nothing else happens) while a modal job runs: no new job may start and the session must not change.
fn blocked_by_job(app: &mut WteApp) -> bool {
    if app.jobs.modal_running() {
        app.state.toasts.warn("Please wait for the running operation to finish");
        return true;
    }
    false
}

/// The unsaved-changes prompt: true when it was opened (the pending operation then waits for the answer).
fn ask_unsaved(app: &mut WteApp, pending: &Pending) -> bool {
    if app.state.doc().is_some_and(io_flows::has_unsaved_changes) {
        app.state.dialog = Dialog::Unsaved { then: pending.clone() };
        return true;
    }
    false
}

/// Resolves an `Option<PathBuf>` action argument: `Some` is used as is, `None` opens the native dialog (never in
/// self-test mode). A picked path updates `prefs.last_dir`.
pub(crate) fn resolve_path(
    app: &mut WteApp,
    path: Option<PathBuf>,
    pick: impl FnOnce(&crate::state::Prefs) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if path.is_some() {
        return path;
    }
    if app.state.selftest {
        log::info!("self-test: a file dialog request without a path is ignored");
        return None;
    }
    let picked = pick(&app.state.prefs)?;
    let dir = if picked.is_dir() { Some(picked.clone()) } else { picked.parent().map(Path::to_path_buf) };
    if let Some(dir) = dir.filter(|d| !d.as_os_str().is_empty()) {
        app.state.prefs.last_dir = Some(dir);
    }
    Some(picked)
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Runs a file operation that may replace the document; `check_unsaved` = ask first when the document is modified.
fn run_pending(app: &mut WteApp, pending: Pending, frame: &eframe::Frame, check_unsaved: bool) {
    if blocked_by_job(app) {
        return;
    }
    if check_unsaved && ask_unsaved(app, &pending) {
        return;
    }
    match pending {
        Pending::NewFromModel(p) => {
            if let Some(path) = resolve_path(app, p, |prefs| io_flows::pick_model(frame, prefs)) {
                close_dialog(app);
                io_flows::start_import(app, path, false);
            }
        }
        Pending::OpenProject(p) => {
            if let Some(path) = resolve_path(app, p, |prefs| io_flows::pick_project(frame, prefs)) {
                close_dialog(app);
                io_flows::start_open_project(app, path);
            }
        }
        Pending::OpenSample(kind) => {
            let doc = wte_core::Document::sample(kind);
            io_flows::install_document(app, doc, None, false);
        }
        Pending::Quit => {
            app.state.allow_close = true;
            close_dialog(app);
            app.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Saves the document (Save As when it has no path or must not be overwritten); `then` runs after success.
fn save(app: &mut WteApp, frame: &eframe::Frame, explicit: Option<PathBuf>, force_dialog: bool, then: Option<Pending>) {
    if blocked_by_job(app) {
        return;
    }
    let Some(session) = app.state.session.as_ref() else { return };
    let current = if force_dialog || session.force_save_as { None } else { session.path.clone() };
    let name = session.doc.name.clone();
    let path = match explicit.or(current) {
        Some(p) => Some(p),
        None => resolve_path(app, None, |prefs| io_flows::pick_save_project(frame, prefs, &name)),
    };
    if let Some(path) = path {
        io_flows::start_save(app, path, then);
    }
}

/// Closes the dialog; a Message shown over a failed export brings the Export dialog back. Closing the closing-openings
/// question cancels its load (`brush_flows::answer_openings`).
fn close_dialog(app: &mut WteApp) {
    if matches!(app.state.dialog, Dialog::CloseOpenings(_)) {
        crate::brush_flows::answer_openings(app, None);
        return;
    }
    // Stickers P9a: Esc or a click outside a link question is its cancel: "Decide later" for a conflict, "Ignore"
    // for a change while the project was closed, nothing else for the others.
    if let Dialog::Link(q) = &app.state.dialog {
        let follow = match (q.kind, q.sticker) {
            (crate::link::QuestionKind::Conflict { .. }, Some(id)) => Some(crate::link::LinkAction::Later(id)),
            (crate::link::QuestionKind::ChangedWhileClosed, Some(id)) => Some(crate::link::LinkAction::IgnoreClosed(id)),
            _ => None,
        };
        app.state.dialog = Dialog::None;
        if let Some(a) = follow {
            crate::link::flows::run_in(&mut app.state, a);
        }
        return;
    }
    // Stickers P6b (E6): Cancel of the shared-picture question drops the edit that waited for it.
    if matches!(app.state.dialog, Dialog::StickerShared { .. }) {
        crate::sticker::run_in(&mut app.state, crate::sticker::StickerAction::AnswerShared(None));
        return;
    }
    let was_message = matches!(app.state.dialog, Dialog::Message { .. });
    app.state.dialog = Dialog::None;
    if was_message && let Some(st) = app.export_stash.take() {
        app.state.dialog = Dialog::Export(st);
    }
}

/// What opening the Export dialog does first: cancel the tool and flush a pending edit. It no longer settles the stamp
/// bakes on the UI thread (fix_export item 6: a wrapped stamp's Final takes seconds on a large model): the dialog's
/// plan and preview follow the document when a Final lands, and the export itself waits for the Finals behind its
/// progress dialog (`io_flows::start_export`).
fn prepare_export(app: &mut WteApp) {
    cancel_tool(app);
    if let Some(doc) = app.state.doc_mut() {
        doc.flush_pending_edit();
    }
}

/// The Export dialog opened for `layers` (the row menu's "Export this layer..." / "Export selected layers...", the
/// Layer menu): the Layers choice "Selected" with these layers in stack order (only those of the set of the first
/// existing one), per-layer PNGs only. No set filter (fix_export item 3): the export takes the layers' set, and the
/// other sets of a design that spans several (`export::linked_selection`). The normal ticks are kept in
/// `other_ticks` and come back when the user switches the Layers choice to Visible / All (fix_export item 1).
/// Without an existing layer the plain defaults stay.
fn export_for_layers(state: &AppState, mut st: ExportDialogState, layers: &[LayerId]) -> ExportDialogState {
    let Some(session) = state.session.as_ref() else { return st };
    let doc = &session.doc;
    let Some((set_id, _)) = layers.iter().find_map(|id| doc.find_layer(*id)) else { return st };
    let Some(set) = doc.set(set_id) else { return st };
    let ordered: Vec<LayerId> = set.layers.iter().map(|l| l.id).filter(|id| layers.contains(id)).collect();
    st.other_ticks = Some(crate::state::ExportTicks::of(&st.options));
    st.options.layers = Some(ordered.clone());
    st.options.sets = None;
    st.options.flat = None;
    st.options.per_layer_png = true;
    st.options.ora = false;
    st.options.psd = false;
    st.selected_layers = ordered;
    st
}

/// The defaults of a freshly opened Export dialog (8.8). `selected_layers` = the Layers list's selection, for the
/// Layers choice "Selected (N)".
fn export_defaults(state: &AppState) -> Option<ExportDialogState> {
    let session = state.session.as_ref()?;
    let doc = &session.doc;
    let dir = state
        .prefs
        .last_export_dir
        .clone()
        .or_else(|| session.path.as_ref().and_then(|p| p.parent()).map(|d| d.join("export")))
        .or_else(|| {
            doc.source.original_path.as_ref().and_then(|p| Path::new(p).parent().map(|d| d.join("export")))
        })
        .unwrap_or_default();
    Some(ExportDialogState {
        options: ExportOptions {
            dir,
            base_name: doc.name.clone(),
            sets: None,
            flat: Some(FlatFormat::Png(PngLevel::Balanced)),
            per_layer_png: true,
            ora: false,
            psd: false,
            include_hidden: false,
            dilate_px: 0,
            layers: None,
            package: wte_core::ExportPackage::Files,
        },
        format_index: 0,
        jpeg_quality: DEFAULT_JPEG_QUALITY,
        jpeg_matte: [255, 255, 255],
        show_preview: true,
        confirm_replace: None,
        selected_layers: session.layer_selection(),
        other_ticks: None,
    })
}

/// The tool settings the brush keys change: the eraser's while the Eraser is active, else the brush's. Stickers P7a:
/// in the Sticker view the sticker tool decides (the sticker Eraser uses the Eraser's settings).
fn active_brush(state: &mut AppState) -> &mut crate::state::BrushSettings {
    let eraser = match crate::sticker::edit_of(state) {
        Some(e) => e.tool == crate::state::StickerTool::Eraser,
        None => state.tool == ToolKind::Eraser,
    };
    if eraser { &mut state.tools.eraser } else { &mut state.tools.brush }
}

/// Next brush diameter for `steps` key presses: x1.25 per step, at least one texel of change, clamped.
fn step_brush_size(size: f32, steps: i32) -> f32 {
    let mut next = (size * 1.25f32.powi(steps)).round();
    if next == size.round() {
        next = size.round() + steps.signum() as f32;
    }
    next.clamp(BRUSH_SIZE_RANGE.0, BRUSH_SIZE_RANGE.1)
}

/// Selects a stamp and makes its layer in its home set the active layer there (8.2).
pub(crate) fn select_stamp(state: &mut AppState, id: Option<wte_core::StampId>) {
    let Some(session) = state.session.as_mut() else { return };
    session.selected_stamp = id;
    let Some(id) = id else { return };
    let Some(stamp) = session.doc.stamp(id) else { return };
    let home = stamp.home_set;
    if let Some((_, layer)) = session.doc.stamp_layers(id).into_iter().find(|(set, _)| *set == home) {
        session.active_set = home;
        if let Err(e) = session.doc.set_active_layer(home, Some(layer)) {
            report_edit_error(state, &e);
        }
    }
}

/// Fits the 2D view onto the active set's texture (the view fits on its next draw when it was never laid out).
fn fit_uv_view(state: &mut AppState) {
    let rect = state.view_rects.get(&ViewId::Main2D).copied();
    let Some(session) = state.session.as_mut() else { return };
    let Some(set) = session.doc.set(session.active_set) else { return };
    let tex = [set.width, set.height];
    match rect {
        Some(rect) => session.uv_view.fit(tex, rect),
        None => session.uv_view = UvViewState::default(),
    }
}

/// Reads and registers a stamp image; `None` (after the error toast and message) when it fails.
pub(crate) fn add_stamp_image(app: &mut WteApp, path: &Path) -> Option<(wte_core::BlobId, String)> {
    let name = file_name_of(path);
    let bytes: Arc<[u8]> = match std::fs::read(path) {
        Ok(b) => b.into(),
        Err(e) => {
            stamp_image_failed(app, path, &e.to_string());
            return None;
        }
    };
    let doc = app.state.doc_mut()?;
    match doc.add_image_blob(&name, bytes) {
        Ok(blob) => Some((blob, name)),
        Err(e) => {
            stamp_image_failed(app, path, &e.to_string());
            None
        }
    }
}

fn stamp_image_failed(app: &mut WteApp, path: &Path, error: &str) {
    app.state.toasts.error(format!("Cannot use {} as a stamp: {error}", file_name_of(path)));
    app.state.dialog = Dialog::Message {
        title: "Image not usable".to_owned(),
        lines: vec![format!("The image {} cannot be used as a stamp.", path.display()), sentence(error)],
    };
}

/// The base-texture choices of the import dialog as core `TextureChoice`s (`Err` = a choice is not usable).
fn texture_choices(choices: &[SetChoiceUi]) -> Result<Vec<TextureChoice>, String> {
    choices
        .iter()
        .enumerate()
        .map(|(set, choice)| match choice {
            SetChoiceUi::Detected => Ok(TextureChoice::Detected),
            SetChoiceUi::Blank { side, fill } => {
                let side = (*side).clamp(1, wte_core::MAX_TEXTURE_SIDE);
                Ok(TextureChoice::Blank { width: side, height: side, fill: *fill })
            }
            SetChoiceUi::File { path, bytes, image: Some(pixels), .. } => {
                let name = file_name_of(path);
                let original = bytes.as_ref().map(|b| SourceFile {
                    key: wte_core::import::normalize_ref(&name),
                    original_name: name,
                    bytes: Arc::clone(b),
                });
                Ok(TextureChoice::Image { pixels: pixels.clone(), original })
            }
            SetChoiceUi::File { path, error, .. } => Err(format!(
                "texture set {}: {} is not usable{}",
                set + 1,
                file_name_of(path),
                error.as_ref().map(|e| format!(" ({e})")).unwrap_or_default()
            )),
        })
        .collect()
}

fn run(app: &mut WteApp, action: AppAction, frame: &eframe::Frame) {
    use AppAction as A;
    let action = if app.state.view_mode == ViewMode::Sticker { in_sticker_view(action) } else { action };
    match action {
        // ---- file ----
        A::NewFromModel(p) => run_pending(app, Pending::NewFromModel(p), frame, true),
        A::OpenProject(p) => run_pending(app, Pending::OpenProject(p), frame, true),
        A::OpenSample(kind) => run_pending(app, Pending::OpenSample(kind), frame, true),
        A::OpenPath(path) => {
            let ext = extension_of(&path);
            if ext == wte_core::PROJECT_EXTENSION {
                run(app, A::OpenProject(Some(path)), frame);
            } else if MODEL_EXTENSIONS.contains(&ext.as_str()) {
                run(app, A::NewFromModel(Some(path)), frame);
            } else if is_image_path(&path) {
                run(app, A::ImportStampImage(Some(path)), frame);
            } else if crate::dnd::is_psd_path(&path) {
                // Stickers P9a (STICKER_PLAN 5.8): a PSD becomes a tray sticker, the Stamp tool's source.
                crate::link::flows::run(app, crate::link::LinkAction::ImportPsd { path: Some(path), keep_linked: false }, frame);
            } else if crate::dnd::is_wtes_path(&path) {
                // Stickers P10a (6.2): a sticker file is added to the tray as the Stamp tool's source.
                if app.state.session.is_none() {
                    app.state.toasts.warn("Open a model first");
                } else {
                    crate::sticker::library::add(&mut app.state, &path);
                    app.state.push(A::SetTool(ToolKind::Stamp));
                }
            } else {
                app.state.toasts.warn(format!("Cannot open {}: unsupported file type", file_name_of(&path)));
            }
        }
        A::Save => save(app, frame, None, false, None),
        A::SaveAs(p) => {
            let explicit = p.map(io_flows::with_project_extension);
            save(app, frame, explicit, true, None);
        }
        A::ShowExport => {
            prepare_export(app);
            if let Some(st) = export_defaults(&app.state) {
                app.export_stash = None;
                app.state.dialog = Dialog::Export(Box::new(st));
            }
        }
        A::ShowExportLayers(layers) => {
            prepare_export(app);
            if let Some(st) = export_defaults(&app.state).map(|st| export_for_layers(&app.state, st, &layers)) {
                app.export_stash = None;
                app.state.dialog = Dialog::Export(Box::new(st));
            }
        }
        A::RunExport(opts) => {
            if blocked_by_job(app) || app.state.session.is_none() {
                return;
            }
            if let Some(doc) = app.state.doc_mut() {
                doc.flush_pending_edit();
            }
            if !opts.dir.as_os_str().is_empty() {
                app.state.prefs.last_export_dir = Some(opts.dir.clone());
            }
            io_flows::start_export(app, opts);
        }
        A::ExportPickFolder(p) => {
            if let Some(dir) = resolve_path(app, p, |prefs| io_flows::pick_folder(frame, prefs)) {
                if let Dialog::Export(st) = &mut app.state.dialog {
                    st.options.dir = dir.clone();
                    st.confirm_replace = None;
                }
                app.state.prefs.last_export_dir = Some(dir);
            }
        }
        A::ExtractSources(p) => {
            if app.state.session.is_none() || blocked_by_job(app) {
                return;
            }
            if let Some(dir) = resolve_path(app, p, |prefs| io_flows::pick_folder(frame, prefs)) {
                io_flows::start_extract(app, dir);
            }
        }
        A::Quit => run_pending(app, Pending::Quit, frame, !app.state.allow_close),
        A::ConfirmPending(p) => {
            close_dialog(app);
            run_pending(app, p, frame, false);
        }
        A::SaveThen(p) => {
            close_dialog(app);
            save(app, frame, None, false, Some(p));
        }

        // ---- import dialog ----
        A::ImportReload { fallback_uvs } => {
            if blocked_by_job(app) {
                return;
            }
            let path = match &mut app.state.dialog {
                Dialog::Import(st) => {
                    st.loading = true;
                    st.fallback_uvs = fallback_uvs;
                    st.error = None;
                    st.path.clone()
                }
                _ => return,
            };
            io_flows::start_import(app, path, fallback_uvs);
        }
        A::ImportPickTexture { set, path } => {
            let Dialog::Import(st) = &app.state.dialog else { return };
            let Some(max_dim) = st.model.as_ref().map(|m| m.options.max_texture_dim.min(wte_core::MAX_TEXTURE_SIDE)) else {
                return;
            };
            if set >= st.choices.len() {
                return;
            }
            let Some(path) = resolve_path(app, path, |prefs| io_flows::pick_image(frame, prefs)) else { return };
            let name = file_name_of(&path);
            let choice = match std::fs::read(&path) {
                Ok(bytes) => match wte_core::decode_image(&bytes, Some(&name), max_dim) {
                    Ok(decoded) => {
                        if decoded.resized {
                            let [w, h] = decoded.original_size;
                            app.state.toasts.info(format!("{name} ({w}x{h}) was downscaled to fit {max_dim} pixels"));
                        }
                        SetChoiceUi::File { path, bytes: Some(bytes.into()), image: Some(decoded.pixels), error: None }
                    }
                    Err(e) => SetChoiceUi::File { path, bytes: None, image: None, error: Some(sentence(&e.to_string())) },
                },
                Err(e) => SetChoiceUi::File { path, bytes: None, image: None, error: Some(sentence(&e.to_string())) },
            };
            if let Dialog::Import(st) = &mut app.state.dialog
                && let Some(slot) = st.choices.get_mut(set)
            {
                *slot = choice;
            }
        }
        A::ImportCreate => {
            if blocked_by_job(app) {
                return;
            }
            let Dialog::Import(st) = &mut app.state.dialog else { return };
            let choices = match texture_choices(&st.choices) {
                Ok(c) => c,
                Err(e) => {
                    app.state.toasts.warn(sentence(&e));
                    return;
                }
            };
            let Some(model) = st.model.take() else { return };
            if model.mesh.sets.is_empty() {
                st.model = Some(model);
                app.state.toasts.warn(sentence(&DocError::NoTextureSets.to_string()));
                return;
            }
            if choices.len() != model.mesh.sets.len() {
                st.model = Some(model);
                app.state.toasts.warn("The texture choices do not match the model; reload the model");
                return;
            }
            app.state.dialog = Dialog::None;
            io_flows::start_build(app, model, choices);
        }

        // ---- edit ----
        A::Undo => {
            let Some(doc) = app.state.doc_mut() else { return };
            if let Some(label) = doc.undo() {
                app.state.toasts.info(format!("Undo: {label}"));
            }
        }
        A::Redo => {
            let Some(doc) = app.state.doc_mut() else { return };
            if let Some(label) = doc.redo() {
                app.state.toasts.info(format!("Redo: {label}"));
            }
        }
        A::Cancel => cancel(app),
        A::DeleteSelection => {
            let selection = app.state.session.as_ref().map(|s| s.layer_selection()).unwrap_or_default();
            if selection.len() > 1 {
                delete_layers(&mut app.state, &selection);
                return;
            }
            let Some(session) = app.state.session.as_mut() else { return };
            match session.selected_stamp.filter(|id| session.doc.stamp(*id).is_some()) {
                Some(id) => {
                    let r = session.doc.remove_stamp(id);
                    edited(&mut app.state, r);
                }
                None => {
                    if let Some(layer) = active_layer(&app.state) {
                        let r = app.state.doc_mut().map(|d| d.remove_layer(layer));
                        if let Some(r) = r {
                            edited(&mut app.state, r);
                        }
                    }
                }
            }
        }

        // ---- view ----
        A::SetViewMode(mode) => {
            if mode == ViewMode::Preview && app.state.session.is_none() {
                return;
            }
            // Stickers P6a: the Sticker view opens on the selected stamp; another mode leaves it first (its return
            // mode becomes the mode Preview goes back to).
            if mode == ViewMode::Sticker {
                crate::sticker::enter(app, None);
                return;
            }
            if app.state.view_mode == ViewMode::Sticker {
                crate::sticker::leave(app, None);
            }
            cancel_tool(app);
            if mode == ViewMode::Preview {
                if let Some(doc) = app.state.doc_mut() {
                    doc.flush_pending_edit();
                }
                if app.state.view_mode != ViewMode::Preview {
                    app.state.last_edit_mode = app.state.view_mode;
                }
            } else {
                app.state.last_edit_mode = mode;
            }
            app.state.view_mode = mode;
        }
        A::ToggleTexture => {
            app.state.flags.shading = match app.state.flags.shading {
                ShadingMode::Textured => ShadingMode::Clay,
                ShadingMode::Clay | ShadingMode::UvChecker => ShadingMode::Textured,
            };
        }
        A::SetShading(mode) => app.state.flags.shading = mode,
        A::ToggleWireframe => {
            app.state.flags.wireframe = !app.state.flags.wireframe;
            app.state.prefs.flags_wireframe = app.state.flags.wireframe;
        }
        A::ToggleSeams => app.state.flags.seams = !app.state.flags.seams,
        A::ToggleUvWire => app.state.flags.uv_wire = !app.state.flags.uv_wire,
        A::ToggleAlphaChecker => app.state.flags.alpha_checker = !app.state.flags.alpha_checker,
        A::ToggleLit => app.state.flags.lit = !app.state.flags.lit,
        A::FrameModel => {
            let mode = app.state.view_mode;
            if let Some(session) = app.state.session.as_mut() {
                // objects-tabs: the visible objects (the whole model when nothing is hidden).
                let bounds = session.doc.visible_bounds();
                match mode {
                    ViewMode::View3D | ViewMode::Split | ViewMode::Sticker => session.camera.frame(bounds),
                    ViewMode::Preview => session.preview_camera.frame(bounds),
                    ViewMode::View2D => {}
                }
            }
            if matches!(mode, ViewMode::View2D | ViewMode::Split) {
                fit_uv_view(&mut app.state);
            }
        }
        A::ResetCamera => {
            let mode = app.state.view_mode;
            if let Some(session) = app.state.session.as_mut() {
                let mut camera = OrbitCamera::default();
                camera.frame(session.doc.visible_bounds());
                if mode == ViewMode::Preview {
                    session.preview_camera = camera;
                } else {
                    session.camera = camera;
                }
            }
        }
        A::SetCameraPreset(preset) => set_camera_preset(&mut app.state, preset),

        // ---- tools and colours ----
        A::SetTool(tool) => {
            cancel_tool(app);
            app.state.tool = tool;
            app.state.status_hint.clear();
        }
        A::BrushSizeStep(steps) => {
            let brush = active_brush(&mut app.state);
            brush.size_px = step_brush_size(brush.size_px, steps);
        }
        A::BrushHardnessStep(steps) => {
            let brush = active_brush(&mut app.state);
            brush.hardness = ((brush.hardness + 0.1 * steps as f32) * 10.0).round().clamp(0.0, 10.0) / 10.0;
        }
        A::SwapColors => {
            let c = &mut app.state.colors;
            std::mem::swap(&mut c.primary, &mut c.secondary);
        }
        A::ResetColors => {
            app.state.colors.primary = [0, 0, 0, 255];
            app.state.colors.secondary = [255, 255, 255, 255];
        }
        A::SetPrimaryColor(c) => app.state.colors.primary = c,

        // ---- sets and layers ----
        A::SetActiveSet(set) => {
            if let Some(session) = app.state.session.as_mut()
                && session.doc.set(set).is_some()
            {
                session.active_set = set;
            }
        }
        A::SelectLayer(layer) => activate_layer(&mut app.state, layer),
        A::ToggleLayerSelected(layer) => toggle_layer_selected(&mut app.state, layer),
        A::SelectLayerRange(layer) => select_layer_range(&mut app.state, layer),
        A::AddPaintLayer => {
            let Some(session) = app.state.session.as_mut() else { return };
            let r = session.doc.add_raster_layer(session.active_set, None);
            edited(&mut app.state, r);
        }
        A::DuplicateLayer(layer) => duplicate_layer(&mut app.state, layer),
        A::DeleteLayer(layer) => layer_command(app, |d| d.remove_layer(layer)),
        A::DuplicateLayers(layers) => duplicate_layers(&mut app.state, &layers),
        A::DeleteLayers(layers) => delete_layers(&mut app.state, &layers),
        A::MergeDown(layer) => {
            if settle_keeps_layer(app, layer) {
                layer_command(app, |d| d.merge_down(layer).map(|_| ()));
            }
        }
        A::RasterizeLayer(layer) => {
            if settle_keeps_layer(app, layer) {
                rasterize_layer(&mut app.state, layer);
            }
        }
        A::MoveLayer { layer, to_index } => layer_command(app, |d| d.move_layer(layer, to_index)),
        A::SetLayerVisible(layer, on) => layer_command(app, |d| d.set_layer_visible(layer, on)),
        A::SetLayerLocked(layer, on) => layer_command(app, |d| d.set_layer_locked(layer, on)),
        A::SetLayerAlphaLock(layer, on) => layer_command(app, |d| d.set_layer_alpha_lock(layer, on)),
        A::SetLayerOpacity(layer, opacity, commit) => layer_command(app, |d| d.set_layer_opacity(layer, opacity, commit)),
        A::SetLayerBlend(layer, blend) => layer_command(app, |d| d.set_layer_blend(layer, blend)),
        A::RenameLayer(layer, name) => layer_command(app, |d| d.rename_layer(layer, &name)),
        A::DuplicateActiveLayer => {
            let selection = app.state.session.as_ref().map(|s| s.layer_selection()).unwrap_or_default();
            if selection.len() > 1 {
                run(app, A::DuplicateLayers(selection), frame);
            } else if let Some(layer) = active_layer(&app.state) {
                run(app, A::DuplicateLayer(layer), frame);
            }
        }
        A::MergeActiveDown => {
            if let Some(layer) = active_layer(&app.state) {
                run(app, A::MergeDown(layer), frame);
            }
        }
        A::RasterizeActiveLayer => {
            if let Some(layer) = active_layer(&app.state) {
                run(app, A::RasterizeLayer(layer), frame);
            }
        }
        A::MoveActiveLayer(delta) => {
            let Some(layer) = active_layer(&app.state) else { return };
            let Some(session) = app.state.session.as_ref() else { return };
            let Some((set, index)) = session.doc.find_layer(layer) else { return };
            let count = session.doc.set(set).map_or(0, |s| s.layers.len());
            let to_index = (index as i64 + delta as i64).clamp(0, count.saturating_sub(1) as i64) as usize;
            if to_index != index {
                run(app, A::MoveLayer { layer, to_index }, frame);
            }
        }

        // ---- stamps ----
        A::ImportStampImage(p) => {
            if app.state.session.is_none() {
                app.state.toasts.warn("Open a model first");
                return;
            }
            let Some(path) = resolve_path(app, p, |prefs| io_flows::pick_image(frame, prefs)) else { return };
            let Some((blob, name)) = add_stamp_image(app, &path) else { return };
            app.state.tools.stamp.source = crate::state::StampToolSource::Blob(blob);
            app.state.tools.stamp.source_name = name;
            run(app, A::SetTool(ToolKind::Stamp), frame);
            app.state.status_hint = "Click on the model to place the stamp".to_owned();
        }
        A::ReplaceStampImage(id, p) => {
            if app.state.doc().and_then(|d| d.stamp(id)).is_none() {
                return;
            }
            let Some(path) = resolve_path(app, p, |prefs| io_flows::pick_image(frame, prefs)) else { return };
            let Some((blob, _)) = add_stamp_image(app, &path) else { return };
            layer_command(app, |d| d.set_stamp_source(id, StampSource::Image { blob }, Commit::Final));
        }
        A::SelectStamp(id) => select_stamp(&mut app.state, id),
        A::UpdateStampSource(id, source, commit) => layer_command(app, |d| d.set_stamp_source(id, source, commit)),
        A::UpdateStampPlacement(id, placement, commit) => {
            layer_command(app, |d| d.set_stamp_placement(id, placement, commit));
        }
        A::UpdateStampTint(id, tint, commit) => layer_command(app, |d| d.set_stamp_tint(id, tint, commit)),
        A::DeleteStamp(id) => layer_command(app, |d| d.remove_stamp(id)),

        // ---- objects (objects-tabs) ----
        A::SelectObject(o) => with_session(app, |s| object_ops::select(s, o)),
        A::ToggleObjectSelected(o) => with_session(app, |s| object_ops::toggle(s, o)),
        A::SelectObjectRange(o) => with_session(app, |s| object_ops::select_range(s, o)),
        A::SelectAllObjects => with_session(app, object_ops::select_all),
        A::DeselectObjects => with_session(app, object_ops::deselect),
        A::SetObjectVisible(o, visible) => object_ops::set_visible(&mut app.state, o, visible),
        A::HideSelectedObjects => object_ops::hide_selected(&mut app.state),
        A::IsolateSelectedObjects => object_ops::isolate_selected(&mut app.state),
        A::ShowAllObjects => object_ops::show_all(&mut app.state),
        A::FrameSelection => object_ops::frame_selection(&mut app.state),

        // ---- brush shape and text flows (round 4) ----
        A::LoadBrushShape(p) => crate::brush_flows::load(app, p, frame),
        A::ReloadBrushShape => crate::brush_flows::reload(app),
        A::RemoveBrushShape => crate::brush_flows::remove(app),
        A::AnswerOpenings(choice) => crate::brush_flows::answer_openings(app, choice),
        A::SetBrushShapeClose(close) => crate::brush_flows::set_close(app, close),
        A::LoadFontSamples => crate::text_ops::load_font_samples(app),

        // ---- ui ----
        A::ShowDialog(dialog) => app.state.dialog = dialog,
        A::CloseDialog => close_dialog(app),
        // ---- stickers P9a: the editor link ----
        A::Link(l) => crate::link::flows::run(app, l, frame),
        A::CancelJob => app.jobs.cancel_modal(),
        A::Toast(level, text) => app.state.toasts.push(level, text),
        A::Companion(c) => companion_command(app, c, frame),

        // ---- stickers (P6a) ----
        A::SetStickerTool(tool) => {
            // Stickers P7a: a stroke is kept when the tool changes in mid-drag; every other drag is cancelled.
            if matches!(app.state.sticker_ui.drag, Some(crate::sticker::CanvasDrag::Stroke { .. })) {
                crate::sticker::finish_drag(&mut app.state);
            }
            crate::sticker::cancel_drag(&mut app.state);
            crate::sticker::tool_changing(&mut app.state, tool);
            if let Some(edit) = crate::sticker::edit_mut(&mut app.state) {
                edit.tool = tool;
                app.state.sticker_ui.last_tool = tool;
                app.state.status_hint.clear();
            }
        }
        A::Sticker(s) => crate::sticker::run(app, s, frame),
        A::DropReady(p) => crate::dnd::apply(&mut app.state, &p),
    }
}

/// Branch `companion`: a Tablet companion command; "Browse for adb.exe" asks with the native dialog first (never in a
/// self-test run).
fn companion_command(app: &mut WteApp, c: crate::companion::CompanionAction, frame: &eframe::Frame) {
    use crate::companion::CompanionAction;
    let c = match c {
        CompanionAction::BrowseAdb(None) => {
            if app.state.selftest {
                return;
            }
            let picked = rfd::FileDialog::new().set_title("Find adb.exe").add_filter("adb.exe", &["exe"]).set_parent(frame).pick_file();
            match picked {
                Some(path) => CompanionAction::BrowseAdb(Some(path)),
                None => return,
            }
        }
        other => other,
    };
    let ctx = app.ctx.clone();
    app.companion.command(&ctx, &mut app.state, c);
}

/// Runs `f` on the session; no-op without one.
fn with_session(app: &mut WteApp, f: impl FnOnce(&mut crate::state::Session)) {
    if let Some(session) = app.state.session.as_mut() {
        f(session);
    }
}

/// Runs a document command; an `Err` becomes a warning toast. No-op without a document.
fn layer_command(app: &mut WteApp, f: impl FnOnce(&mut wte_core::Document) -> Result<(), EditError>) {
    let Some(doc) = app.state.doc_mut() else { return };
    let r = f(doc);
    edited(&mut app.state, r);
}

/// Esc (8.3), first match wins.
fn cancel(app: &mut WteApp) {
    if app.jobs.modal_running() {
        return; // only the progress dialog's Cancel button cancels a job
    }
    if app.tools.is_busy() {
        // Esc throws the interaction away, also a released brush stroke that is still being painted (FEATURE_BRUSH
        // 5.10: only Undo and Esc discard it).
        app.tools.discard(&mut app.state);
        return;
    }
    match &mut app.state.dialog {
        Dialog::None => {}
        Dialog::Export(st) if st.confirm_replace.is_some() => {
            st.confirm_replace = None;
            return;
        }
        _ => {
            close_dialog(app);
            return;
        }
    }
    if let Some(doc) = app.state.doc_mut()
        && doc.has_pending_edit()
    {
        doc.cancel_pending_edit();
        return;
    }
    if app.state.view_mode == ViewMode::Preview {
        let back = if app.state.last_edit_mode == ViewMode::Preview { ViewMode::View3D } else { app.state.last_edit_mode };
        app.state.view_mode = back;
        return;
    }
    if let Some(session) = app.state.session.as_mut() {
        session.selected_stamp = None;
    }
}

fn set_camera_preset(state: &mut AppState, preset: CameraPreset) {
    let mode = state.view_mode;
    if let Some(session) = state.session.as_mut() {
        let camera = if mode == ViewMode::Preview { &mut session.preview_camera } else { &mut session.camera };
        camera.preset(preset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wte_core::{PixelBuf, TextureSetId};

    #[test]
    fn guard_covers_document_commands_but_not_selection_or_previews() {
        let layer = LayerId(1);
        assert!(needs_interaction_guard(&AppAction::Undo));
        assert!(needs_interaction_guard(&AppAction::Save));
        assert!(needs_interaction_guard(&AppAction::AddPaintLayer));
        assert!(needs_interaction_guard(&AppAction::SetLayerOpacity(layer, 0.5, Commit::Final)));
        assert!(!needs_interaction_guard(&AppAction::SetLayerOpacity(layer, 0.5, Commit::Preview)));
        assert!(!needs_interaction_guard(&AppAction::SelectLayer(layer)));
        assert!(!needs_interaction_guard(&AppAction::SetActiveSet(TextureSetId(0))));
        assert!(!needs_interaction_guard(&AppAction::SelectStamp(None)));
        assert!(!needs_interaction_guard(&AppAction::ToggleWireframe));
        assert!(!needs_interaction_guard(&AppAction::Cancel));
        // Round 4: a brush shape bake takes the pointer away; the quiet reload, Remove and Cancel do not.
        assert!(needs_interaction_guard(&AppAction::LoadBrushShape(None)));
        assert!(needs_interaction_guard(&AppAction::AnswerOpenings(Some(wte_core::CloseMode::FlatCaps))));
        assert!(needs_interaction_guard(&AppAction::SetBrushShapeClose(wte_core::CloseMode::RoundedCaps)));
        assert!(!needs_interaction_guard(&AppAction::AnswerOpenings(None)));
        assert!(!needs_interaction_guard(&AppAction::ReloadBrushShape));
        assert!(!needs_interaction_guard(&AppAction::RemoveBrushShape));
        assert!(!needs_interaction_guard(&AppAction::LoadFontSamples));
        // objects-tabs: a visibility change ends a held stroke by committing it; selecting and framing need nothing.
        for a in [
            AppAction::SetObjectVisible(0, false),
            AppAction::HideSelectedObjects,
            AppAction::IsolateSelectedObjects,
            AppAction::ShowAllObjects,
        ] {
            assert!(needs_interaction_guard(&a) && finishes_interaction(&a), "{a:?}");
        }
        for a in [
            AppAction::SelectObject(0),
            AppAction::ToggleObjectSelected(0),
            AppAction::SelectObjectRange(0),
            AppAction::SelectAllObjects,
            AppAction::DeselectObjects,
            AppAction::FrameSelection,
        ] {
            assert!(!needs_interaction_guard(&a), "{a:?}");
        }
    }

    /// Stickers P6a (STICKER_PLAN 3.3): in the Sticker view the Edit menu's Delete and the Layer menu's layer commands
    /// act on the sticker, never on a texture layer or the stamp; the editor's document commands are guarded.
    #[test]
    fn the_sticker_view_maps_the_layer_commands_to_the_sticker() {
        use crate::sticker::StickerAction as S;
        let text = |a: AppAction| format!("{:?}", in_sticker_view(a));
        assert_eq!(text(AppAction::DeleteSelection), format!("{:?}", AppAction::Sticker(S::DeleteKey)));
        assert_eq!(text(AppAction::DuplicateActiveLayer), format!("{:?}", AppAction::Sticker(S::DuplicateLayer(None))));
        assert_eq!(text(AppAction::MergeActiveDown), format!("{:?}", AppAction::Sticker(S::MergeDown(None))));
        assert_eq!(text(AppAction::MoveActiveLayer(-1)), format!("{:?}", AppAction::Sticker(S::MoveLayer(None, -1))));
        assert_eq!(text(AppAction::Undo), "Undo");
        // P6a check: the Layer menu's Delete / Rasterize / New and the texture-layer forms never reach a texture layer
        // (entering the view made the stamp's own layer the active one: Delete removed the stamp).
        let l = wte_core::LayerId(7);
        assert_eq!(text(AppAction::DeleteLayer(l)), format!("{:?}", AppAction::Sticker(S::DeleteLayer(None))));
        assert_eq!(text(AppAction::DeleteLayers(vec![l])), format!("{:?}", AppAction::Sticker(S::DeleteLayer(None))));
        assert_eq!(text(AppAction::DuplicateLayer(l)), format!("{:?}", AppAction::Sticker(S::DuplicateLayer(None))));
        assert_eq!(text(AppAction::DuplicateLayers(vec![l])), format!("{:?}", AppAction::Sticker(S::DuplicateLayer(None))));
        assert_eq!(text(AppAction::MergeDown(l)), format!("{:?}", AppAction::Sticker(S::MergeDown(None))));
        assert_eq!(text(AppAction::AddPaintLayer), format!("{:?}", AppAction::Sticker(S::AddPictureLayer(None))));
        assert_eq!(text(AppAction::RasterizeActiveLayer), format!("{:?}", AppAction::Sticker(S::Rasterize)));
        // ... and the mapped Delete leaves the stamp, its layer and the sticker's last layer.
        let (mut state, stamp, _) = crate::sticker::tests::image_stamp_state(ViewMode::View3D);
        crate::sticker::enter_in(&mut state, None);
        let set = state.session.as_ref().unwrap().active_set;
        let active = state.doc().unwrap().set(set).unwrap().active_layer().unwrap().id;
        assert_eq!(state.doc().unwrap().layer(active).unwrap().stamp_id(), Some(stamp), "the stamp's layer is active");
        let AppAction::Sticker(mapped) = in_sticker_view(AppAction::DeleteLayer(active)) else { panic!() };
        let layers = state.doc().unwrap().set(set).unwrap().layers.len();
        crate::sticker::run_in(&mut state, mapped);
        assert!(state.doc().unwrap().stamp(stamp).is_some());
        assert_eq!(state.doc().unwrap().set(set).unwrap().layers.len(), layers);
        assert!(needs_interaction_guard(&AppAction::Sticker(S::AddPictureLayer(None))));
        assert!(!needs_interaction_guard(&AppAction::Sticker(S::SelectLayer(wte_core::StickerLayerId(1)))));
        assert!(!needs_interaction_guard(&AppAction::SetStickerTool(crate::state::StickerTool::Hand)));
    }

    /// The Sphere sample with two more paint layers: [Base, Paint 1, A, B] bottom to top, B active.
    fn four_layer_state() -> (AppState, [LayerId; 4]) {
        let mut state = crate::tools::test_support::state_with(wte_core::SampleKind::Sphere);
        let doc = state.doc_mut().unwrap();
        doc.add_raster_layer(TextureSetId(0), Some("A")).unwrap();
        doc.add_raster_layer(TextureSetId(0), Some("B")).unwrap();
        let ids: Vec<LayerId> = doc.sets[0].layers.iter().map(|l| l.id).collect();
        assert_eq!(doc.sets[0].active, Some(ids[3]));
        (state, [ids[0], ids[1], ids[2], ids[3]])
    }

    fn selection(state: &AppState) -> Vec<LayerId> {
        state.session.as_ref().unwrap().layer_selection()
    }

    fn active(state: &AppState) -> Option<LayerId> {
        state.doc().unwrap().sets[0].active
    }

    /// fix_export item 5: Duplicate and Delete on a multi-selection act on every selected layer, one undo step each;
    /// the copies become the selection; after Delete the selection collapses; a locked layer stays with a warning.
    #[test]
    fn duplicate_and_delete_act_on_every_selected_layer_in_one_undo_step() {
        let (mut state, [base, paint, a, b]) = four_layer_state();
        select_layer_range(&mut state, paint);
        assert_eq!(selection(&state), [paint, a, b]);
        let undo = state.doc().unwrap().history.undo_len();
        duplicate_layers(&mut state, &[paint, a, b]);
        let doc = state.doc().unwrap();
        assert_eq!(doc.history.undo_len(), undo + 1, "one undo step");
        assert_eq!(doc.sets[0].layers.len(), 7);
        let copies = selection(&state);
        assert_eq!(copies.len(), 3);
        assert!(copies.iter().all(|c| ![base, paint, a, b].contains(c)), "the copies are selected");
        assert!(copies.contains(&active(&state).unwrap()), "the active layer's copy is active");
        assert!(state.doc_mut().unwrap().undo().is_some());
        assert_eq!(state.doc().unwrap().sets[0].layers.len(), 4, "one undo removes every copy");

        // Delete: Paint 1, A and B selected, B locked.
        state.doc_mut().unwrap().set_layer_locked(b, true).unwrap();
        state.doc_mut().unwrap().set_active_layer(TextureSetId(0), Some(b)).unwrap();
        state.session.as_mut().unwrap().selected_layers = vec![paint, a, b];
        let undo = state.doc().unwrap().history.undo_len();
        let warnings = state.toasts.visible_labels().len();
        delete_layers(&mut state, &[paint, a, b]);
        let doc = state.doc().unwrap();
        assert_eq!(doc.history.undo_len(), undo + 1);
        assert_eq!(doc.sets[0].layers.iter().map(|l| l.id).collect::<Vec<_>>(), [base, b], "the locked layer stays");
        assert_eq!(selection(&state).len(), 1, "the selection collapses");
        assert!(state.toasts.visible_labels().len() > warnings, "a warning names the kept layer");
        assert!(state.toasts.visible_labels().iter().any(|l| l.contains("1 locked layer was kept")));
        assert!(state.doc_mut().unwrap().undo().is_some());
        assert_eq!(state.doc().unwrap().sets[0].layers.len(), 4, "one undo brings both back");
    }

    #[test]
    fn ctrl_and_shift_select_layers_without_touching_the_document() {
        let (mut state, [base, paint, a, b]) = four_layer_state();
        let doc = state.doc().unwrap();
        let (undo, revision, modified) = (doc.history.undo_len(), doc.revision(), doc.is_modified());
        assert_eq!(selection(&state), [b]);

        // Toggle adds and removes; the active layer stays.
        toggle_layer_selected(&mut state, paint);
        assert_eq!(selection(&state), [paint, b]);
        toggle_layer_selected(&mut state, base);
        assert_eq!(selection(&state), [base, paint, b]);
        toggle_layer_selected(&mut state, paint);
        assert_eq!(selection(&state), [base, b]);
        assert_eq!(active(&state), Some(b));
        // Toggling the active layer off makes the top-most remaining one active.
        toggle_layer_selected(&mut state, b);
        assert_eq!(selection(&state), [base]);
        assert_eq!(active(&state), Some(base));
        // The last selected layer stays.
        toggle_layer_selected(&mut state, base);
        assert_eq!(selection(&state), [base]);
        assert_eq!(active(&state), Some(base));

        // Range up from the active layer, then down from a new active layer.
        select_layer_range(&mut state, a);
        assert_eq!(selection(&state), [base, paint, a]);
        assert_eq!(active(&state), Some(base));
        activate_layer(&mut state, b);
        assert_eq!(selection(&state), [b]);
        select_layer_range(&mut state, paint);
        assert_eq!(selection(&state), [paint, a, b]);
        assert_eq!(active(&state), Some(b));
        // A plain select collapses the selection.
        activate_layer(&mut state, a);
        assert_eq!(selection(&state), [a]);

        // None of this is a document edit.
        let doc = state.doc().unwrap();
        assert_eq!((doc.history.undo_len(), doc.revision(), doc.is_modified()), (undo, revision, modified));
        for action in [AppAction::ToggleLayerSelected(a), AppAction::SelectLayerRange(a)] {
            assert!(!needs_interaction_guard(&action), "{action:?}");
        }
        assert!(needs_interaction_guard(&AppAction::ShowExportLayers(vec![a])));
        assert!(finishes_interaction(&AppAction::ShowExportLayers(vec![a])));
    }

    #[test]
    fn the_selection_stays_valid_when_layers_go_away() {
        let (mut state, [_, paint, a, b]) = four_layer_state();
        select_layer_range(&mut state, paint);
        assert_eq!(selection(&state), [paint, a, b]);
        // Deleting the active layer: the core picks another active layer; the selection keeps the selected layers that
        // still exist, and always holds the active one.
        let valid = |state: &AppState| {
            let sel = selection(state);
            let set = &state.doc().unwrap().sets[0];
            !sel.is_empty() && sel.iter().all(|id| set.layer_index(*id).is_some()) && sel.contains(&set.active.unwrap())
        };
        state.doc_mut().unwrap().remove_layer(b).unwrap();
        assert!(valid(&state), "{:?}", selection(&state));
        assert!(!selection(&state).contains(&b));
        assert_eq!(selection(&state), [paint, a], "a is now active and was selected");
        // Undo brings B back as the active layer: still a valid selection.
        state.doc_mut().unwrap().undo().unwrap();
        assert!(valid(&state), "{:?}", selection(&state));
        // A new paint layer becomes the active layer, which is not selected: the selection collapses to it.
        let new = state.doc_mut().unwrap().add_raster_layer(TextureSetId(0), None).unwrap();
        assert_eq!(selection(&state), [new]);
        // A stale raw list and a layer of another set are ignored.
        let session = state.session.as_mut().unwrap();
        let active_now = session.doc.sets[0].active.unwrap();
        session.selected_layers = vec![LayerId(9_999), active_now, paint];
        let mut want = vec![paint, active_now];
        want.sort_by_key(|id| session.doc.sets[0].layer_index(*id));
        assert_eq!(session.layer_selection(), want);
    }

    #[test]
    fn export_for_layers_opens_the_dialog_on_the_selection() {
        let (mut state, [base, paint, _, b]) = four_layer_state();
        select_layer_range(&mut state, paint);
        let st = export_defaults(&state).unwrap();
        // The plain defaults: today's options, the selection captured for "Selected (N)".
        assert_eq!(st.options.layers, None);
        assert_eq!(st.options.package, wte_core::ExportPackage::Files);
        assert!(st.options.flat.is_some() && st.options.per_layer_png && !st.options.include_hidden);
        assert_eq!(st.selected_layers.len(), 3);
        // Opened for layers (any order, an unknown id ignored): stack order, per-layer PNGs only, no set filter (the
        // layers decide the sets, fix_export item 3); the normal ticks are kept for Visible / All (item 1).
        let normal = crate::state::ExportTicks::of(&st.options);
        let st = export_for_layers(&state, st, &[b, LayerId(9_999), base]);
        assert_eq!(st.options.layers, Some(vec![base, b]));
        assert_eq!(st.selected_layers, [base, b]);
        assert_eq!(st.options.sets, None);
        assert!(st.options.flat.is_none() && st.options.per_layer_png && !st.options.ora && !st.options.psd);
        assert_eq!(st.other_ticks, Some(normal));
        // Nothing that exists: the plain defaults stay.
        let st = export_for_layers(&state, export_defaults(&state).unwrap(), &[LayerId(9_999)]);
        assert_eq!(st.options.layers, None);
        assert!(st.options.flat.is_some());
    }

    /// check_exp_1 D4: "Export this layer..." on two sets' layers with the same index and name writes two different
    /// files, each named like the full export names that layer (with the set name), so one does not replace the other.
    #[test]
    fn export_for_layers_on_two_sets_keeps_the_set_names_apart() {
        let mut state = crate::tools::test_support::state_with(wte_core::SampleKind::TwoMaterial);
        let doc = state.doc_mut().unwrap();
        assert!(doc.sets.len() >= 2);
        let a = doc.add_raster_layer(TextureSetId(0), Some("Logo")).unwrap();
        let b = doc.add_raster_layer(TextureSetId(1), Some("Logo")).unwrap();
        assert_eq!(doc.find_layer(a).unwrap().1, doc.find_layer(b).unwrap().1, "the same stack index");
        let snapshot = state.doc().unwrap().snapshot();
        let names = |layer: LayerId| -> Vec<String> {
            let st = export_for_layers(&state, export_defaults(&state).unwrap(), &[layer]);
            wte_core::export::plan_contents(&snapshot, &st.options).into_iter().map(|f| f.name).collect()
        };
        let (na, nb) = (names(a), names(b));
        assert_eq!(na.len(), 1);
        assert_eq!(nb.len(), 1);
        assert_ne!(na, nb, "two sets' Logo layers never share a file name");
        let mut full = export_defaults(&state).unwrap().options;
        full.include_hidden = true;
        let full: Vec<String> = wte_core::export::plan_contents(&snapshot, &full).into_iter().map(|f| f.name).collect();
        assert!(full.contains(&na[0]) && full.contains(&nb[0]), "{na:?} {nb:?} not in the full export {full:?}");
    }

    #[test]
    fn save_and_export_finish_a_held_interaction_everything_else_cancels_it() {
        for a in [AppAction::Save, AppAction::SaveAs(None), AppAction::ShowExport, AppAction::SaveThen(crate::state::Pending::Quit)] {
            assert!(needs_interaction_guard(&a) && finishes_interaction(&a), "{a:?}");
        }
        for a in [AppAction::Undo, AppAction::Quit, AppAction::OpenProject(None), AppAction::AddPaintLayer, AppAction::DeleteSelection] {
            assert!(needs_interaction_guard(&a) && !finishes_interaction(&a), "{a:?}");
        }
    }

    #[test]
    fn duplicating_a_layer_moves_the_selection_to_the_copy() {
        use glam::{Vec2, Vec3};
        let mut state = crate::tools::test_support::state_with(wte_core::SampleKind::Sphere);
        let img = wte_core::samples::sample_stamp_image(32);
        let png = wte_core::encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        let blob = doc.add_image_blob("logo.png", Arc::from(png)).unwrap();
        let d = doc.mesh.bounds.diagonal();
        let hit = Vec3::new(0.0, 0.0, doc.mesh.bounds.max.z);
        let p = wte_core::Projector::from_surface(hit, Vec3::Z, Vec3::Y, Vec2::splat(d * 0.1), d * 0.1);
        let id = doc.add_stamp("Logo", StampSource::Image { blob }, wte_core::Placement::Projected(p), TextureSetId(0)).unwrap();
        let layer = doc.stamp_layers(id)[0].1;
        state.session.as_mut().unwrap().selected_stamp = Some(id);

        duplicate_layer(&mut state, layer);
        let session = state.session.as_ref().unwrap();
        let copy = session.selected_stamp.expect("a stamp is selected");
        assert_ne!(copy, id, "the copy is selected, not the original under it");
        let copy_layer = session.doc.stamp_layers(copy)[0].1;
        assert_eq!(session.doc.sets[0].active, Some(copy_layer), "selection and active layer agree");
        assert_eq!(session.doc.undo_label(), Some("Duplicate Stamp"));

        // A paint-layer copy is the active layer: no stamp stays selected.
        let paint = session.doc.sets[0].layers.iter().find(|l| l.is_raster() && !l.locked).unwrap().id;
        duplicate_layer(&mut state, paint);
        let session = state.session.as_ref().unwrap();
        assert_eq!(session.selected_stamp, None);
        assert_eq!(session.doc.undo_label(), Some("Duplicate Layer"));
    }

    #[test]
    fn stale_targets_are_dropped_without_a_toast() {
        let mut state = crate::tools::test_support::state_with(wte_core::SampleKind::Sphere);
        for e in [EditError::UnknownLayer, EditError::UnknownStamp, EditError::UnknownSet] {
            assert_eq!(edited::<()>(&mut state, Err(e)), None);
        }
        assert!(state.toasts.visible_labels().is_empty(), "{:?}", state.toasts.visible_labels());
        // A real refusal still says what happened.
        edited::<()>(&mut state, Err(EditError::LayerLocked));
        assert_eq!(state.toasts.visible_labels(), ["Layer is locked"]);
        // Through a layer command: an opacity Final for a layer that was removed (F2) shows nothing.
        let missing = LayerId(9_999);
        let r = state.doc_mut().unwrap().set_layer_opacity(missing, 0.5, Commit::Final);
        edited(&mut state, r);
        assert_eq!(state.toasts.visible_labels(), ["Layer is locked"]);
    }

    #[test]
    fn rasterizing_a_paint_layer_does_nothing_and_shows_nothing() {
        let mut state = crate::tools::test_support::state_with(wte_core::SampleKind::Sphere);
        let doc = state.doc().unwrap();
        let paint = doc.sets[0].layers.iter().find(|l| l.is_raster() && !l.locked).unwrap().id;
        let undo = doc.history.undo_len();
        let modified = doc.is_modified();
        rasterize_layer(&mut state, paint);
        let doc = state.doc().unwrap();
        assert_eq!(doc.history.undo_len(), undo, "no undo entry");
        assert_eq!(doc.is_modified(), modified);
        assert!(doc.layer(paint).is_some_and(|l| l.is_raster()));
        assert!(state.toasts.visible_labels().is_empty(), "{:?}", state.toasts.visible_labels());
        // A layer that is gone: nothing either.
        rasterize_layer(&mut state, LayerId(9_999));
        assert!(state.toasts.visible_labels().is_empty());
    }

    #[test]
    fn brush_size_steps_always_move_and_stay_in_range() {
        assert_eq!(step_brush_size(32.0, 1), 40.0);
        assert_eq!(step_brush_size(40.0, -1), 32.0);
        assert_eq!(step_brush_size(1.0, 1), 2.0);
        assert_eq!(step_brush_size(1.0, -1), 1.0);
        assert_eq!(step_brush_size(2000.0, 1), 2048.0);
        assert_eq!(step_brush_size(2.0, -1), 1.0);
    }

    #[test]
    fn sentence_capitalizes() {
        assert_eq!(sentence("layer is locked"), "Layer is locked");
        assert_eq!(sentence(""), "");
    }

    #[test]
    fn texture_choices_map_every_kind() {
        let pixels = PixelBuf::filled(4, 4, [1, 2, 3, 255]);
        let bytes: Arc<[u8]> = Arc::from(&b"png bytes"[..]);
        let choices = vec![
            SetChoiceUi::Detected,
            SetChoiceUi::Blank { side: 99_999, fill: [9, 9, 9, 255] },
            SetChoiceUi::File { path: PathBuf::from(r"E:\t\My Tex.PNG"), bytes: Some(bytes.clone()), image: Some(pixels), error: None },
        ];
        let mapped = texture_choices(&choices).unwrap();
        assert!(matches!(mapped[0], TextureChoice::Detected));
        assert!(matches!(mapped[1], TextureChoice::Blank { width: 8192, height: 8192, fill: [9, 9, 9, 255] }));
        match &mapped[2] {
            TextureChoice::Image { pixels, original: Some(f) } => {
                assert_eq!(pixels.width, 4);
                assert_eq!(f.original_name, "My Tex.PNG");
                assert_eq!(f.key, "my tex.png");
                assert_eq!(&f.bytes[..], &bytes[..]);
            }
            other => panic!("unexpected {other:?}"),
        }
        let bad = vec![SetChoiceUi::File { path: PathBuf::from("x.png"), bytes: None, image: None, error: Some("broken".into()) }];
        assert!(texture_choices(&bad).is_err());
    }
}
