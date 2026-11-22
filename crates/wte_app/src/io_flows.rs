// Owner: app-shell (A1). File flows of ARCHITECTURE.md 8.9: starting the import / build / load / save / export /
// extract / fonts jobs, handling their results, installing documents, the close request, the window title and the
// native file dialogs (called by dispatch only, never in self-test mode).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui;
use serde::{Deserialize, Serialize};
use wte_core::{
    Document, ExportOptions, FontCatalog, ImportOptions, ImportedModel, PngLevel, Progress, SaveOptions, TexelMap,
    TextureChoice, TextureSetId, uv_layout_images,
};

use crate::app::WteApp;
use crate::dnd::{IMAGE_EXTENSIONS, MODEL_EXTENSIONS};
use crate::jobs::JobOutput;
use crate::state::{
    ColorState, Dialog, HoverInfo, ImportDialogState, OrbitCamera, Pending, Prefs, SWATCH_COUNT, Session, SetChoiceUi,
    UvViewState, ViewMode,
};

/// Window title suffix (and the title without a document).
pub const APP_TITLE: &str = "Wrapped Texture Editor";
/// Side of the UV-layout thumbnails and of the UV statistics raster of the import dialog.
const UV_THUMB_SIDE: u32 = 256;

static NEXT_SCENE_GENERATION: AtomicU64 = AtomicU64::new(1);

/// A process-wide unique `Session::scene_generation`.
pub fn next_scene_generation() -> u64 {
    NEXT_SCENE_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// The app-side view state stored in `Document::ui_state` (7.2: camera, UV view, view mode, active set, colours and
/// swatches). Every field is optional so that older or foreign values never fail to load.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct UiState {
    camera: Option<OrbitCamera>,
    preview_camera: Option<OrbitCamera>,
    uv_view: Option<UvViewState>,
    view_mode: Option<ViewMode>,
    active_set: Option<u32>,
    colors: Option<ColorState>,
}

impl UiState {
    fn capture(state: &crate::state::AppState, session: &Session) -> UiState {
        let view_mode = if state.view_mode == ViewMode::Preview { state.last_edit_mode } else { state.view_mode };
        UiState {
            camera: Some(session.camera),
            preview_camera: Some(session.preview_camera),
            uv_view: Some(session.uv_view),
            view_mode: Some(view_mode),
            active_set: Some(session.active_set.0),
            colors: Some(state.colors.clone()),
        }
    }
}

fn camera_is_sane(c: &OrbitCamera) -> bool {
    c.target.is_finite()
        && c.yaw.is_finite()
        && c.pitch.is_finite()
        && c.distance.is_finite()
        && c.distance > 0.0
        && c.fov_y.is_finite()
        && c.fov_y > 0.01
        && c.fov_y < 3.0
}

/// True when closing or replacing the document would lose something: a change since the last save, or an edit that
/// is still open as a `Commit::Preview` (text typed into a focused field, a value being dragged). A pending edit has
/// not bumped the revision yet, so `Document::is_modified` alone does not see it.
pub fn has_unsaved_changes(doc: &Document) -> bool {
    doc.is_modified() || doc.has_pending_edit()
}

/// Reports a failed job: an error toast plus a `Dialog::Message` with `title` and `lines`. When the user had asked
/// the job to stop (`cancelled`), it is no failure: only the info toast `cancelled_text`. Returns true when the
/// Message dialog was opened.
fn report_failure(
    state: &mut crate::state::AppState,
    cancelled: bool,
    cancelled_text: &str,
    toast: String,
    title: &str,
    lines: Vec<String>,
) -> bool {
    if cancelled {
        state.toasts.info(cancelled_text);
        return false;
    }
    state.toasts.error(toast);
    state.dialog = Dialog::Message { title: title.to_owned(), lines };
    true
}

/// Handles the result of a finished job (may replace the session). `cancel_requested`: the user pressed the progress
/// dialog's Cancel while it ran (an `Err` result is then a cancellation, reported as such).
pub fn on_job_output(app: &mut WteApp, out: JobOutput, cancel_requested: bool) {
    match out {
        JobOutput::Imported { path, result, uv_thumbs, uv_stats } => match result {
            Ok(model) => {
                log::info!(
                    "imported {}: {} triangles, {} texture set(s), {} warning(s)",
                    path.display(),
                    model.mesh.tri_count(),
                    model.mesh.sets.len(),
                    model.warnings.len()
                );
                if let Err(e) = crate::gpu::scene::check_mesh_fits(&model.mesh, app.gpu.limits()) {
                    let lines = vec![format!("The model {} cannot be shown.", path.display()), e.clone()];
                    report_failure(&mut app.state, false, "", format!("Import failed: {e}"), "Model too large", lines);
                    return;
                }
                let choices = vec![SetChoiceUi::Detected; model.mesh.sets.len()];
                let fallback_uvs = model.options.generate_fallback_uvs;
                app.state.dialog = Dialog::Import(Box::new(ImportDialogState {
                    path,
                    model: Some(model),
                    choices,
                    uv_thumbs,
                    uv_stats,
                    fallback_uvs,
                    loading: false,
                    error: None,
                }));
            }
            Err(e) => {
                let lines = vec![format!("The model {} could not be imported.", path.display()), e.clone()];
                // "Cannot open <name>: …" (R3-8) already names the file and says what happened.
                let toast = if e.starts_with(CANNOT_OPEN) { e } else { format!("Import failed: {e}") };
                report_failure(&mut app.state, cancel_requested, "Import cancelled", toast, "Import failed", lines);
            }
        },
        JobOutput::Built { result, path, warnings, force_save_as } => match result {
            Ok(doc) => {
                // Refused before it replaces anything: buffers above the device limits would end in a wgpu
                // validation panic when the views build the scene.
                if let Err(e) = crate::gpu::scene::check_mesh_fits(&doc.mesh, app.gpu.limits()) {
                    let what = match &path {
                        Some(p) => format!("The project {} cannot be shown.", p.display()),
                        None => "The document cannot be shown.".to_owned(),
                    };
                    app.state.toasts.error(format!("{} {e}", what.trim_end_matches('.')));
                    app.state.dialog = Dialog::Message { title: "Model too large".to_owned(), lines: vec![what, e] };
                    return;
                }
                if let Some(p) = &path {
                    remember_recent(app, p);
                }
                install_document(app, doc, path, force_save_as);
                // Load warnings (missing blobs, font fallbacks) are shown nowhere else. Import warnings were already
                // listed in the import dialog: `start_build` passes none.
                if !warnings.is_empty() {
                    app.state.dialog = Dialog::Message { title: "Warnings".to_owned(), lines: warnings };
                }
            }
            Err(e) => {
                let (what, cancelled_text) = match &path {
                    Some(p) => (format!("The project {} could not be opened.", p.display()), "Opening cancelled"),
                    None => ("The document could not be created.".to_owned(), "Creating the document cancelled"),
                };
                let toast = if e.starts_with(CANNOT_OPEN) { e.clone() } else { format!("{} {e}", what.trim_end_matches('.')) };
                report_failure(&mut app.state, cancel_requested, cancelled_text, toast, "Error", vec![what, e]);
            }
        },
        JobOutput::Saved { result, scene_generation, then } => {
            let Some(session) = app.state.session.as_mut().filter(|s| s.scene_generation == scene_generation) else {
                log::warn!("ignoring the result of a save that belongs to a document that is no longer open");
                return;
            };
            match result {
                Ok((path, saved_state, reuse)) => {
                    // The state the snapshot was taken in: an edit made while the save ran keeps the document
                    // modified, undoing it again makes it unmodified.
                    session.doc.mark_saved(saved_state);
                    // The next save copies the images that do not change until then from this file.
                    session.doc.set_png_reuse(reuse);
                    session.path = Some(path.clone());
                    session.force_save_as = false;
                    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    remember_recent(app, &path);
                    app.state.toasts.info(format!("Saved {name}"));
                    if let Some(pending) = then {
                        app.state.push(crate::action::AppAction::ConfirmPending(pending));
                    }
                }
                Err(e) => {
                    // The document stays modified and `then` does not run.
                    let lines = vec!["The project was not saved.".to_owned(), e.clone()];
                    let toast = format!("Save failed: {e}");
                    report_failure(&mut app.state, cancel_requested, "Save cancelled: the project was not saved", toast, "Save failed", lines);
                }
            }
        }
        JobOutput::Exported { result, dir } => match result {
            Ok(report) => {
                if matches!(app.state.dialog, Dialog::Export(_)) {
                    app.state.dialog = Dialog::None;
                }
                app.export_stash = None;
                // The folder name only; the full path is the toast's hover text (R3-7).
                let text = format!("Exported {} files to {}", report.files.len(), folder_name(&dir));
                app.state.toasts.push_with_detail(crate::action::ToastLevel::Info, text, dir.display().to_string());
            }
            Err(e) => {
                // A cancelled export leaves the Export dialog open as it is.
                let previous = std::mem::replace(&mut app.state.dialog, Dialog::None);
                let lines = vec![format!("Export to {} failed.", dir.display()), e.clone()];
                let toast = format!("Export failed: {e}");
                if report_failure(&mut app.state, cancel_requested, "Export cancelled", toast, "Export failed", lines) {
                    if let Dialog::Export(st) = previous {
                        app.export_stash = Some(st);
                    }
                } else {
                    app.state.dialog = previous;
                }
            }
        },
        JobOutput::Extracted { result, dir } => match result {
            Ok(files) => {
                // The model file lies at the top of the folder actually written: a fresh sub-folder when files of
                // the same names already existed in `dir`.
                let written = files.first().and_then(|f| f.parent()).unwrap_or(&dir);
                let text = format!("Extracted {} files to {}", files.len(), folder_name(written));
                app.state.toasts.push_with_detail(crate::action::ToastLevel::Info, text, written.display().to_string());
            }
            Err(e) => {
                let lines = vec![format!("The original files could not be extracted to {}.", dir.display()), e.clone()];
                let toast = format!("Extracting the original files failed: {e}");
                report_failure(&mut app.state, cancel_requested, "Extracting cancelled", toast, "Extract failed", lines);
            }
        },
        JobOutput::Fonts(catalog) => {
            log::info!("font catalog: {} families", catalog.families().len());
            app.state.fonts = catalog;
            app.state.fonts_loading = false;
        }
        JobOutput::BrushShape { request, path, close, result } => {
            crate::brush_flows::on_job_output(app, request, path, close, result, cancel_requested);
        }
        JobOutput::FontSamples(samples) => crate::text_ops::on_font_samples(app, samples),
        JobOutput::StampBake { generation, output } => crate::stamp_bakes::on_output(app, generation, output),
        JobOutput::WrapPreview { generation, output } | JobOutput::WrapParam { generation, output } => {
            crate::stamp_bakes::on_wrap_output(app, generation, output)
        }
        JobOutput::WrapWarm { generation, ms } => crate::stamp_bakes::on_warm_output(&mut app.state, generation, ms),
    }
}

fn remember_recent(app: &mut WteApp, path: &Path) {
    if app.state.selftest {
        return;
    }
    crate::prefs::add_recent(&mut app.state.prefs, path);
    crate::brush_flows::sync_prefs(&mut app.state);
    crate::prefs::save(&app.state.prefs);
}

/// Reacts to a close request of the window: `CancelClose` plus the unsaved prompt or the "please wait" toast.
pub fn handle_close_request(app: &mut WteApp, ctx: &egui::Context) {
    if !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    if app.state.selftest && app.state.allow_close {
        return; // the self-test runner ends the run; nothing may keep the window open
    }
    if app.jobs.modal_running() {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        app.state.toasts.warn("Please wait for the running operation to finish");
        return;
    }
    let unsaved = app.state.doc().is_some_and(has_unsaved_changes);
    if unsaved && !app.state.allow_close {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        // Quit wins: an unsaved-changes prompt that is already open for another operation (Open, New from Model)
        // now continues with quitting, and says so.
        if let Dialog::Unsaved { then } = &mut app.state.dialog {
            if !matches!(then, Pending::Quit) {
                *then = Pending::Quit;
                app.state.toasts.info("The window closes after you answer the unsaved-changes prompt");
            }
        } else {
            app.state.dialog = Dialog::Unsaved { then: Pending::Quit };
        }
        return;
    }
    // The window closes: remember its size (the normal size, not a maximized one) and the maximized state.
    if !app.state.selftest {
        let (inner, maximized) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().maximized.unwrap_or(false)));
        remember_window(&mut app.state.prefs, inner, maximized);
    }
    log::info!("closing the window");
}

/// Stores the window geometry for the next start: the maximized flag, and the inner size only when the window is not
/// maximized (a maximized size restored as a normal window would be as large as the screen and partly off it).
pub(crate) fn remember_window(prefs: &mut Prefs, inner: Option<egui::Rect>, maximized: bool) {
    prefs.window_maximized = maximized;
    if maximized {
        return;
    }
    if let Some(rect) = inner {
        let size = rect.size();
        if size.x >= 1.0 && size.y >= 1.0 {
            prefs.window_size = [size.x, size.y];
        }
    }
}

/// The window title for the current state: `"<name>[*] - Wrapped Texture Editor"`.
pub fn title_text(state: &crate::state::AppState) -> String {
    match state.doc() {
        Some(doc) => format!("{}{} - {APP_TITLE}", doc.name, if has_unsaved_changes(doc) { "*" } else { "" }),
        None => APP_TITLE.to_owned(),
    }
}

/// Sends `ViewportCommand::Title` only when the title text changed.
pub fn update_title(app: &mut WteApp, ctx: &egui::Context) {
    let title = title_text(&app.state);
    if title != app.state.last_title {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
        app.state.last_title = title;
    }
}

/// Starts the import job for a model file.
///
/// A fresh import (*New from Model*, drag and drop, CLI) generates the texture layout of a model without any UVs
/// right away, so every model with geometry can be painted; the import dialog explains it and its checkbox turns
/// the layout off again. A reload from that checkbox (the import dialog is open and loading) honours
/// `fallback_uvs` exactly.
pub fn start_import(app: &mut WteApp, path: PathBuf, fallback_uvs: bool) {
    if refuse_while_busy(app) {
        return;
    }
    let reload = matches!(&app.state.dialog, Dialog::Import(st) if st.loading);
    let auto_uvs = !reload;
    let opts = ImportOptions { generate_fallback_uvs: fallback_uvs, ..ImportOptions::default() };
    log::info!("import {} (fallback UVs: {fallback_uvs}, automatic UVs: {auto_uvs})", path.display());
    app.jobs.spawn(&app.ctx, "Importing model", true, move |p| {
        let start = std::time::Instant::now();
        match wte_core::import::import_from_path_auto_uvs(&path, &opts, auto_uvs, p) {
            Ok(model) => {
                let imported = start.elapsed();
                p.report(0.95, "Analysing the UV layout");
                let (uv_thumbs, uv_stats) = uv_previews(&model);
                log::info!(
                    "import job of {}: import {} ms, UV previews {} ms",
                    path.display(),
                    imported.as_millis(),
                    (start.elapsed() - imported).as_millis()
                );
                p.report(1.0, "Done");
                JobOutput::Imported { path, result: Ok(model), uv_thumbs, uv_stats }
            }
            Err(e) => {
                let text = cannot_open_text(&path).unwrap_or_else(|| e.to_string());
                JobOutput::Imported { path, result: Err(text), uv_thumbs: Vec::new(), uv_stats: Vec::new() }
            }
        }
    });
}

/// One UV-layout thumbnail and one set of UV statistics per texture set (import dialog). The wireframe edges are
/// derived once for all thumbnails (and drawn in parallel).
fn uv_previews(model: &ImportedModel) -> (Vec<wte_core::PixelBuf>, Vec<wte_core::UvStats>) {
    let thumbs = uv_layout_images(&model.mesh, UV_THUMB_SIDE);
    let stats = (0..model.mesh.sets.len() as u32)
        .map(|set| TexelMap::build(&model.mesh, TextureSetId(set), UV_THUMB_SIDE, UV_THUMB_SIDE, 0).stats)
        .collect();
    (thumbs, stats)
}

/// Starts the build job (`Document::from_import`) of the import dialog's Create button.
pub fn start_build(app: &mut WteApp, model: ImportedModel, choices: Vec<TextureChoice>) {
    if refuse_while_busy(app) {
        return;
    }
    // The import warnings were listed in the import dialog already; they are not shown a second time.
    app.jobs.spawn(&app.ctx, "Creating document", true, move |p| {
        let result = Document::from_import(model, choices, p).map_err(|e| e.to_string());
        JobOutput::Built { result, path: None, warnings: Vec::new(), force_save_as: false }
    });
}

/// Starts the load job for a `.wtep` project.
pub fn start_open_project(app: &mut WteApp, path: PathBuf) {
    if refuse_while_busy(app) {
        return;
    }
    log::info!("open project {}", path.display());
    app.jobs.spawn(&app.ctx, "Opening project", true, move |p| match wte_core::project::load(&path, p) {
        Ok(report) => JobOutput::Built {
            result: Ok(report.document),
            warnings: report.warnings.iter().map(|w| w.to_string()).collect(),
            force_save_as: report.force_save_as,
            path: Some(path),
        },
        Err(e) => {
            let text = cannot_open_text(&path).unwrap_or_else(|| e.to_string());
            JobOutput::Built { result: Err(text), path: Some(path), warnings: Vec::new(), force_save_as: false }
        }
    });
}

/// The last component of a folder path (the whole path when it has none, e.g. a drive root).
fn folder_name(dir: &Path) -> String {
    dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Start of the texts of `cannot_open_text`.
const CANNOT_OPEN: &str = "Cannot open ";

/// The failure text of a model or project whose file itself cannot be read (R3-8): "Cannot open <name>: file not
/// found" and the like, naming the file instead of the bare OS error. `None` when the file can be opened, so the
/// failure was about its content (or a file it refers to) and the core's own text says more. Checked on the path
/// itself after the failure, not on the error, so an I/O error about another file (a glTF buffer, a texture) is not
/// reported as this file missing. The core's error texts are not changed for this.
pub(crate) fn cannot_open_text(path: &Path) -> Option<String> {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let why = match std::fs::metadata(path) {
        Err(e) => io_reason(&e),
        Ok(m) if m.is_dir() => "it is a folder, not a file".to_owned(),
        Ok(_) => match std::fs::File::open(path) {
            Err(e) => io_reason(&e),
            Ok(_) => return None,
        },
    };
    Some(format!("{CANNOT_OPEN}{name}: {why}"))
}

/// A short reason for an I/O error opening a file.
fn io_reason(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "file not found".to_owned(),
        std::io::ErrorKind::PermissionDenied => "access denied".to_owned(),
        _ => e.to_string(),
    }
}

/// Starts the save job; `then` continues after a successful save.
pub fn start_save(app: &mut WteApp, path: PathBuf, then: Option<Pending>) {
    if refuse_while_busy(app) {
        return;
    }
    let Some(session) = app.state.session.as_ref() else { return };
    let ui_state = serde_json::to_value(UiState::capture(&app.state, session)).unwrap_or(serde_json::Value::Null);
    let keep_backup = app.state.prefs.keep_backup;
    let Some(session) = app.state.session.as_mut() else { return };
    session.doc.ui_state = ui_state;
    session.doc.flush_pending_edit();
    // Round 5: a saved project never holds the cache of a stamp whose Full bake is still due.
    crate::stamp_bakes::settle(app);
    let Some(session) = app.state.session.as_ref() else { return };
    let snapshot = session.doc.snapshot();
    let scene_generation = session.scene_generation;
    let opts = SaveOptions {
        png: PngLevel::Fast,
        write_flattened: true,
        keep_backup,
        now_unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    log::info!("save {}", path.display());
    app.jobs.spawn(&app.ctx, "Saving project", true, move |p| {
        let saved_state = snapshot.history_state;
        let result =
            wte_core::project::save(&snapshot, &path, &opts, p).map(|stats| (path, saved_state, stats.reuse)).map_err(|e| e.to_string());
        JobOutput::Saved { result, scene_generation, then }
    });
}

/// Starts the export job.
pub fn start_export(app: &mut WteApp, opts: ExportOptions) {
    if refuse_while_busy(app) {
        return;
    }
    let Some(doc) = app.state.doc_mut() else { return };
    doc.flush_pending_edit();
    crate::stamp_bakes::settle(app);
    let Some(doc) = app.state.doc() else { return };
    let snapshot = doc.snapshot();
    log::info!("export to {}", opts.dir.display());
    app.jobs.spawn(&app.ctx, "Exporting", true, move |p| {
        let dir = opts.dir.clone();
        let result = wte_core::export::run(&snapshot, &opts, p).map_err(|e| e.to_string());
        JobOutput::Exported { result, dir }
    });
}

/// Starts the "Extract Original Files" job on the in-memory document (works for a document that was never saved).
pub fn start_extract(app: &mut WteApp, dir: PathBuf) {
    if refuse_while_busy(app) {
        return;
    }
    let Some(doc) = app.state.doc() else { return };
    let source = std::sync::Arc::clone(&doc.source);
    let blobs = doc.blobs.clone();
    log::info!("extract original files to {}", dir.display());
    app.jobs.spawn(&app.ctx, "Extracting original files", true, move |p| {
        p.report(0.0, "Writing files");
        let result = wte_core::project::extract_bundle(&source, &blobs, &dir, false).map_err(|e| e.to_string());
        JobOutput::Extracted { result, dir }
    });
}

/// Starts the non-modal system-font scan; until it delivers, `state.fonts` is the bundled catalog.
pub fn start_fonts(app: &mut WteApp) {
    app.state.fonts_loading = true;
    app.jobs.spawn(&app.ctx, "Loading system fonts", false, |p| {
        p.report(0.0, "Scanning installed fonts");
        let mut catalog = FontCatalog::bundled();
        catalog.load_system();
        JobOutput::Fonts(catalog)
    });
}

/// While a modal job runs no other job may start (the session must not change under it): warns and returns true.
fn refuse_while_busy(app: &mut WteApp) -> bool {
    if app.jobs.modal_running() {
        app.state.toasts.warn("Please wait for the running operation to finish");
        return true;
    }
    false
}

/// Makes `doc` the open document: cancels the tool interaction, replaces the session (fresh camera, new
/// `scene_generation`), resets the document-bound tool settings and closes dialogs.
pub fn install_document(app: &mut WteApp, mut doc: Document, path: Option<PathBuf>, force_save_as: bool) {
    if app.tools.is_busy() {
        app.tools.cancel(&mut app.state);
    }
    // Stamp Preview bakes run throttled by the views (`view::bake_stale_previews`), not inside every drag frame.
    doc.set_defer_preview_bakes(true);
    // Round 5 (R5-1): Final bakes run on a worker (`stamp_bakes`) when the app is in that mode.
    doc.set_defer_final_bakes(app.state.async_final_bakes);
    // Round 8 (R8-2): and so do the Preview bakes of wrapped stamps (latest wins, `stamp_bakes`); with
    // `WTE_SYNC_FINAL_BAKES=1` they stay on the UI thread (the comparison mode).
    doc.set_defer_wrap_previews(app.state.async_final_bakes);
    crate::stamp_bakes::reset_for_new_document(&mut app.state);
    app.state.preview_bakes = crate::state::PreviewBakeClock::default();
    let mut camera = OrbitCamera::default();
    camera.frame(doc.mesh.bounds);
    let restored: Option<UiState> = match &doc.ui_state {
        serde_json::Value::Object(_) => match serde_json::from_value(doc.ui_state.clone()) {
            Ok(ui) => Some(ui),
            Err(e) => {
                log::warn!("ignoring the stored view state: {e}");
                None
            }
        },
        _ => None,
    };
    log::info!("document \"{}\": {} set(s), {} stamp(s)", doc.name, doc.sets.len(), doc.stamps.len());
    let mut session = Session {
        doc,
        path,
        force_save_as,
        active_set: TextureSetId(0),
        selected_stamp: None,
        camera,
        preview_camera: camera,
        uv_view: UvViewState::default(),
        scene_generation: next_scene_generation(),
        text_gap: None,
    };
    if let Some(ui) = restored {
        if let Some(c) = ui.camera.filter(camera_is_sane) {
            session.camera = c;
        }
        if let Some(c) = ui.preview_camera.filter(camera_is_sane) {
            session.preview_camera = c;
        }
        if let Some(uv) = ui.uv_view.filter(|u| u.center_uv.is_finite() && u.zoom.is_finite() && u.zoom > 0.0) {
            session.uv_view = UvViewState { center_uv: uv.center_uv, zoom: uv.zoom.clamp(0.02, 64.0) };
        }
        if let Some(set) = ui.active_set {
            session.active_set = TextureSetId(set);
        }
        if let Some(mode) = ui.view_mode.filter(|m| *m != ViewMode::Preview) {
            app.state.view_mode = mode;
            app.state.last_edit_mode = mode;
        }
        if let Some(mut colors) = ui.colors {
            colors.swatches.resize(SWATCH_COUNT, [255, 255, 255, 255]);
            app.state.colors = colors;
        }
    }
    session.validate();
    app.state.session = Some(session);
    app.scene = None; // rebuilt for the new generation by the views; frees the old GPU resources now
    app.state.tools.stamp.source = None;
    app.state.tools.stamp.source_name.clear();
    app.state.dialog = Dialog::None;
    app.export_stash = None;
    app.state.hover = HoverInfo::default();
    app.state.status_hint.clear();
}

fn dialog_dir(prefs: &Prefs) -> Option<&Path> {
    prefs.last_dir.as_deref().filter(|d| d.is_dir())
}

/// Native "open model" dialog (synchronous, parented to the window).
pub fn pick_model(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("New from Model")
        .add_filter("3D models (OBJ, glTF, GLB, FBX)", &MODEL_EXTENSIONS)
        .set_parent(frame);
    if let Some(dir) = dialog_dir(prefs) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file()
}

/// Native "open project" dialog.
pub fn pick_project(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Open Project")
        .add_filter("Wrapped Texture Editor project", &[wte_core::PROJECT_EXTENSION])
        .set_parent(frame);
    if let Some(dir) = dialog_dir(prefs) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file()
}

/// Native "save project" dialog.
pub fn pick_save_project(frame: &eframe::Frame, prefs: &Prefs, default_name: &str) -> Option<PathBuf> {
    let stem = wte_core::export::sanitize_file_stem(default_name);
    let stem = if stem.is_empty() { "project".to_owned() } else { stem };
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save Project")
        .add_filter("Wrapped Texture Editor project", &[wte_core::PROJECT_EXTENSION])
        .set_file_name(format!("{stem}.{}", wte_core::PROJECT_EXTENSION))
        .set_parent(frame);
    if let Some(dir) = dialog_dir(prefs) {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file().map(with_project_extension)
}

/// Appends `.wtep` unless the path already has that extension.
pub fn with_project_extension(path: PathBuf) -> PathBuf {
    if crate::dnd::extension_of(&path) == wte_core::PROJECT_EXTENSION {
        path
    } else {
        let mut s = path.into_os_string();
        s.push(".");
        s.push(wte_core::PROJECT_EXTENSION);
        PathBuf::from(s)
    }
}

/// Native "open image" dialog.
pub fn pick_image(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Choose Image")
        .add_filter("Images", &IMAGE_EXTENSIONS)
        .set_parent(frame);
    if let Some(dir) = dialog_dir(prefs) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file()
}

/// Native "choose folder" dialog.
pub fn pick_folder(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title("Choose Folder").set_parent(frame);
    if let Some(dir) = dialog_dir(prefs) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_folder()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[test]
    fn scene_generations_are_unique() {
        let a = next_scene_generation();
        let b = next_scene_generation();
        assert!(b > a);
    }

    #[test]
    fn project_extension_is_added_once() {
        assert_eq!(with_project_extension(PathBuf::from(r"E:\p\a")), PathBuf::from(r"E:\p\a.wtep"));
        assert_eq!(with_project_extension(PathBuf::from(r"E:\p\a.WTEP")), PathBuf::from(r"E:\p\a.WTEP"));
        assert_eq!(with_project_extension(PathBuf::from(r"E:\p\a.v2")), PathBuf::from(r"E:\p\a.v2.wtep"));
    }

    #[test]
    fn ui_state_round_trips_and_tolerates_garbage() {
        let state = AppState::new(Prefs::default(), true);
        let session = Session {
            doc: Document::sample(wte_core::SampleKind::Cube),
            path: None,
            force_save_as: false,
            active_set: TextureSetId(0),
            selected_stamp: None,
            camera: OrbitCamera { yaw: 1.5, ..OrbitCamera::default() },
            preview_camera: OrbitCamera::default(),
            uv_view: UvViewState { center_uv: glam::Vec2::new(0.3, 0.6), zoom: 2.0 },
            scene_generation: 1,
            text_gap: None,
        };
        let ui = UiState::capture(&state, &session);
        let value = serde_json::to_value(&ui).unwrap();
        let back: UiState = serde_json::from_value(value).unwrap();
        assert_eq!(back, ui);
        assert_eq!(back.camera.unwrap().yaw, 1.5);
        let partial: UiState = serde_json::from_value(serde_json::json!({ "active_set": 3, "unknown": true })).unwrap();
        assert_eq!(partial.active_set, Some(3));
        assert!(partial.camera.is_none());
        assert!(!camera_is_sane(&OrbitCamera { distance: f32::NAN, ..OrbitCamera::default() }));
    }

    fn session_of(doc: Document) -> Session {
        Session {
            doc,
            path: None,
            force_save_as: false,
            active_set: TextureSetId(0),
            selected_stamp: None,
            camera: OrbitCamera::default(),
            preview_camera: OrbitCamera::default(),
            uv_view: UvViewState::default(),
            scene_generation: 1,
            text_gap: None,
        }
    }

    #[test]
    fn an_edit_still_open_as_a_preview_counts_as_unsaved() {
        // Text typed into a focused field is a Commit::Preview edit: no revision bump yet, but it must not be lost
        // silently on close or when another project replaces the document.
        let mut state = AppState::new(Prefs::default(), true);
        let mut doc = Document::sample(wte_core::SampleKind::Sphere);
        doc.name = "ball".to_owned();
        doc.mark_saved(doc.history_state());
        state.session = Some(session_of(doc));
        assert!(!has_unsaved_changes(state.doc().unwrap()));
        let doc = state.doc_mut().unwrap();
        let layer = doc.sets[0].active.unwrap();
        doc.set_layer_opacity(layer, 0.4, wte_core::Commit::Preview).unwrap();
        assert!(!doc.is_modified() && doc.has_pending_edit());
        assert!(has_unsaved_changes(state.doc().unwrap()));
        assert_eq!(title_text(&state), "ball* - Wrapped Texture Editor");
        // Cancelled (Esc): nothing is left to lose.
        state.doc_mut().unwrap().cancel_pending_edit();
        assert!(!has_unsaved_changes(state.doc().unwrap()));
        assert_eq!(title_text(&state), "ball - Wrapped Texture Editor");
    }

    #[test]
    fn a_cancelled_job_is_an_info_toast_not_a_failure() {
        let mut state = AppState::new(Prefs::default(), true);
        let opened = report_failure(&mut state, true, "Import cancelled", "Import failed: cancelled".into(), "Import failed", vec![]);
        assert!(!opened && matches!(state.dialog, Dialog::None));
        assert_eq!(state.toasts.error_count(), 0);
        assert_eq!(state.toasts.visible_texts(), ["Import cancelled"]);
        let opened = report_failure(&mut state, false, "Import cancelled", "Import failed: bad file".into(), "Import failed", vec!["x".into()]);
        assert!(opened && matches!(&state.dialog, Dialog::Message { title, .. } if title == "Import failed"));
        assert_eq!(state.toasts.error_count(), 1);
    }

    #[test]
    fn a_maximized_window_keeps_its_normal_size() {
        let mut prefs = Prefs::default();
        let rect = |w: f32, h: f32| Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h)));
        remember_window(&mut prefs, rect(1400.0, 900.0), false);
        assert_eq!((prefs.window_size, prefs.window_maximized), ([1400.0, 900.0], false));
        remember_window(&mut prefs, rect(1920.0, 1129.0), true);
        assert_eq!((prefs.window_size, prefs.window_maximized), ([1400.0, 900.0], true), "the maximized size is not stored");
        remember_window(&mut prefs, None, false);
        assert_eq!((prefs.window_size, prefs.window_maximized), ([1400.0, 900.0], false));
    }

    #[test]
    fn title_marks_modified_documents() {
        let mut state = AppState::new(Prefs::default(), true);
        assert_eq!(title_text(&state), "Wrapped Texture Editor");
        let mut doc = Document::sample(wte_core::SampleKind::Sphere);
        doc.name = "ball".to_owned();
        state.session = Some(Session {
            doc,
            path: None,
            force_save_as: false,
            active_set: TextureSetId(0),
            selected_stamp: None,
            camera: OrbitCamera::default(),
            preview_camera: OrbitCamera::default(),
            uv_view: UvViewState::default(),
            scene_generation: 1,
            text_gap: None,
        });
        assert_eq!(title_text(&state), "ball - Wrapped Texture Editor");
        let doc = state.doc_mut().unwrap();
        let layer = doc.sets[0].active.unwrap();
        doc.rename_layer(layer, "Renamed").unwrap();
        assert_eq!(title_text(&state), "ball* - Wrapped Texture Editor");
    }

    #[test]
    fn a_missing_or_unreadable_file_is_named() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/io_flows_cannot_open_test");
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("DoesNotExist.glb");
        assert_eq!(cannot_open_text(&missing).as_deref(), Some("Cannot open DoesNotExist.glb: file not found"));
        let project = dir.join("gone.wtep");
        assert_eq!(cannot_open_text(&project).as_deref(), Some("Cannot open gone.wtep: file not found"));
        let folder = dir.join("model.obj");
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(cannot_open_text(&folder).as_deref(), Some("Cannot open model.obj: it is a folder, not a file"));
        // A file that can be opened: the failure was about its content, the core's text says more.
        let file = dir.join("broken.glb");
        std::fs::write(&file, b"not a model").unwrap();
        assert_eq!(cannot_open_text(&file), None);
        assert!(CANNOT_OPEN.ends_with(' '));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_names_drop_the_path() {
        let p: PathBuf = ["E:", "x", "runs", "shot_export"].iter().collect();
        assert_eq!(folder_name(&p), "shot_export");
    }
}
