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
/// swatches; objects-tabs: the names of the hidden objects). Every field is optional so that older or foreign values
/// never fail to load.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct UiState {
    camera: Option<OrbitCamera>,
    preview_camera: Option<OrbitCamera>,
    uv_view: Option<UvViewState>,
    view_mode: Option<ViewMode>,
    active_set: Option<u32>,
    colors: Option<ColorState>,
    /// objects-tabs: names of the hidden objects. Not written when nothing is hidden (a project then has the bytes of
    /// a build without objects); read leniently: anything but an array of strings counts as nothing hidden and the
    /// other fields still load.
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "lenient_names")]
    hidden_objects: Vec<String>,
}

/// `UiState::hidden_objects` from any JSON value: the strings of an array, nothing otherwise.
fn lenient_names<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let names = match value {
        serde_json::Value::Array(items) => {
            items.into_iter().filter_map(|v| if let serde_json::Value::String(s) = v { Some(s) } else { None }).collect()
        }
        serde_json::Value::Null => Vec::new(),
        other => {
            log::warn!("ignoring the stored hidden objects: not a list ({other})");
            Vec::new()
        }
    };
    Ok(names)
}

impl UiState {
    fn capture(state: &crate::state::AppState, session: &Session) -> UiState {
        // Stickers P6a: never Preview, never the Sticker view (the mode it returns to is written).
        let view_mode = state.remembered_view_mode();
        UiState {
            camera: Some(session.camera),
            preview_camera: Some(session.preview_camera),
            uv_view: Some(session.uv_view),
            view_mode: Some(view_mode),
            active_set: Some(session.active_set.0),
            colors: Some(state.colors.clone()),
            hidden_objects: crate::object_ops::hidden_names(&session.doc),
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
        JobOutput::Built { result, path, warnings, force_save_as, links } => match result {
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
                install_document(app, doc, path.clone(), force_save_as);
                // Load warnings (missing blobs, font fallbacks) are shown nowhere else. Import warnings were already
                // listed in the import dialog: `start_build` passes none.
                if !warnings.is_empty() {
                    app.state.dialog = Dialog::Message { title: "Warnings".to_owned(), lines: warnings };
                }
                // Stickers P9a (STICKER_PLAN 5.3 "At load"): restore missing link files, ask about changed ones.
                if let Some(p) = &path
                    && !links.is_empty()
                {
                    crate::link::flows::after_load(&mut app.state, p, &links);
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
            app.snapshot_feed = None;
            let Some(session) = app.state.session.as_mut().filter(|s| s.scene_generation == scene_generation) else {
                log::warn!("ignoring the result of a save that belongs to a document that is no longer open");
                return;
            };
            match result {
                Ok(saved) => {
                    let crate::jobs::Saved { path, history_state, reuse, links_revision, warnings } = saved;
                    // The state the snapshot was taken in: an edit made while the save ran keeps the document
                    // modified, undoing it again makes it unmodified.
                    session.doc.mark_saved(history_state);
                    // The next save copies the images that do not change until then from this file.
                    session.doc.set_png_reuse(reuse);
                    session.path = Some(path.clone());
                    session.force_save_as = false;
                    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                    remember_recent(app, &path);
                    app.state.toasts.info(format!("Saved {name}"));
                    // Stickers P9a: the links of the snapshot are saved; aliases of a Save As; a waiting "Edit in".
                    crate::link::flows::after_save(&mut app.state, links_revision, &warnings);
                    if let Some(pending) = then {
                        app.state.push(crate::action::AppAction::ConfirmPending(pending));
                    }
                }
                Err(e) => {
                    crate::link::flows::save_failed(&mut app.state);
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
                app.snapshot_feed = None;
                let text = export_toast_text(&report, &dir);
                app.state.toasts.push_with_detail(crate::action::ToastLevel::Info, text, dir.display().to_string());
            }
            Err(e) => {
                app.snapshot_feed = None;
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
    }
}

fn remember_recent(app: &mut WteApp, path: &Path) {
    if app.state.selftest {
        return;
    }
    crate::prefs::add_recent(&mut app.state.prefs, path);
    crate::brush_flows::sync_prefs(&mut app.state);
    crate::panels::sync_prefs(&mut app.state, &app.panels);
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
        JobOutput::Built { result, path: None, warnings: Vec::new(), force_save_as: false, links: Vec::new() }
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
            links: report.links,
        },
        Err(e) => {
            let text = cannot_open_text(&path).unwrap_or_else(|| e.to_string());
            JobOutput::Built { result: Err(text), path: Some(path), warnings: Vec::new(), force_save_as: false, links: Vec::new() }
        }
    });
}

/// The last component of a folder path (the whole path when it has none, e.g. a drive root).
fn folder_name(dir: &Path) -> String {
    dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// "1 file" / "N files".
fn files_text(n: usize) -> String {
    if n == 1 { "1 file".to_owned() } else { format!("{n} files") }
}

/// The toast of a finished export (fix_export item 8: singular for one file): "Exported 1 file to <folder>",
/// "Exported 3 files to <folder>", "Exported <zip> (1 file) to <folder>". The folder name only; the full path is the
/// toast's hover text (R3-7).
pub(crate) fn export_toast_text(report: &wte_core::ExportReport, dir: &Path) -> String {
    if report.packaged == 0 {
        format!("Exported {} to {}", files_text(report.files.len()), folder_name(dir))
    } else {
        let zip = report.files.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        format!("Exported {zip} ({}) to {}", files_text(report.packaged), folder_name(dir))
    }
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
    let scene_generation = session.scene_generation;
    // Stickers P9a (STICKER_PLAN 5.3 "At Save As"): a moved project takes its link files along (the save archives
    // them from the new links folder); the links opened in this session keep their old file as an alias.
    crate::link::flows::before_save(&mut app.state, &path);
    // Round 5: a saved project never holds the cache of a stamp whose Full bake is still due. Polish item 2: as for
    // export (fix_export item 6), no `settle` on the UI thread: while Final bakes are due (a wrapped stamp placed or
    // edited a moment ago: 3-4 s on ABeautifulGame) the job starts at once behind the progress dialog ("Finishing
    // stamps") and gets the snapshot from `feed_waiting_snapshot` when they have landed.
    let (tx, rx) = std::sync::mpsc::channel::<wte_core::DocumentSnapshot>();
    let waiting = crate::stamp_bakes::bakes_pending(&app.state);
    if !waiting {
        let Some(doc) = app.state.doc() else { return };
        if tx.send(doc.snapshot()).is_err() {
            return;
        }
    }
    let opts = SaveOptions {
        png: PngLevel::Fast,
        write_flattened: true,
        keep_backup,
        now_unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    log::info!("save {}{}", path.display(), if waiting { " (waiting for the Final bakes)" } else { "" });
    app.jobs.spawn(&app.ctx, "Saving project", true, move |p| {
        let snapshot = match receive_snapshot(&rx, p) {
            Ok(snapshot) => snapshot,
            Err(e) => return JobOutput::Saved { result: Err(e), scene_generation, then },
        };
        let (history_state, links_revision) = (snapshot.history_state, snapshot.links_revision);
        let result = wte_core::project::save(&snapshot, &path, &opts, p)
            .map(|stats| crate::jobs::Saved {
                path,
                history_state,
                reuse: stats.reuse,
                links_revision,
                warnings: stats.warnings.iter().map(|w| w.to_string()).collect(),
            })
            .map_err(|e| e.to_string());
        JobOutput::Saved { result, scene_generation, then }
    });
    if waiting {
        app.snapshot_feed = Some(SnapshotFeed { tx, since: std::time::Instant::now(), what: "save" });
    }
}

/// Starts the export job.
pub fn start_export(app: &mut WteApp, opts: ExportOptions) {
    if refuse_while_busy(app) {
        return;
    }
    let Some(doc) = app.state.doc_mut() else { return };
    doc.flush_pending_edit();
    // fix_export item 6: no `settle` on the UI thread. When Final bakes are still due (a wrapped stamp placed or
    // edited a moment ago: 3-4 s on ABeautifulGame), the job starts at once behind the progress dialog ("Finishing
    // stamps") and gets the snapshot from `feed_waiting_snapshot` when they have landed; the UI keeps drawing meanwhile.
    let (tx, rx) = std::sync::mpsc::channel::<wte_core::DocumentSnapshot>();
    let waiting = crate::stamp_bakes::bakes_pending(&app.state);
    if !waiting {
        let Some(doc) = app.state.doc() else { return };
        if tx.send(doc.snapshot()).is_err() {
            return;
        }
    }
    log::info!("export to {}{}", opts.dir.display(), if waiting { " (waiting for the Final bakes)" } else { "" });
    app.jobs.spawn(&app.ctx, "Exporting", true, move |p| {
        let dir = opts.dir.clone();
        let snapshot = match receive_snapshot(&rx, p) {
            Ok(snapshot) => snapshot,
            Err(e) => return JobOutput::Exported { result: Err(e), dir },
        };
        // fix_export item 7: what a crashed export of another process left in this folder.
        wte_core::export::remove_stale_temps(&dir, &process_running);
        let result = wte_core::export::run(&snapshot, &opts, p).map_err(|e| e.to_string());
        JobOutput::Exported { result, dir }
    });
    if waiting {
        app.snapshot_feed = Some(SnapshotFeed { tx, since: std::time::Instant::now(), what: "export" });
    }
}

/// fix_export item 6: an export job waiting for its snapshot (`start_export`), and since when; polish item 2: a save
/// job too (`start_save`). `what` names the job in the log. One modal job runs at a time, so one feed is enough.
pub struct SnapshotFeed {
    tx: std::sync::mpsc::Sender<wte_core::DocumentSnapshot>,
    since: std::time::Instant,
    what: &'static str,
}

/// The export or save job's wait for its snapshot: shows "Finishing stamps" until it comes; `Err("cancelled")` when the
/// user cancels meanwhile (the result counts as cancelled: `Finished::cancel_requested`), an error when the app dropped
/// the feed (the document went away).
fn receive_snapshot(
    rx: &std::sync::mpsc::Receiver<wte_core::DocumentSnapshot>,
    p: &dyn Progress,
) -> Result<wte_core::DocumentSnapshot, String> {
    use std::sync::mpsc::RecvTimeoutError;
    let mut shown = false;
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(if shown { 50 } else { 0 })) {
            Ok(snapshot) => return Ok(snapshot),
            Err(RecvTimeoutError::Disconnected) => return Err("the document is no longer open".to_owned()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if p.cancelled() {
            return Err(wte_core::ExportError::Cancelled.to_string());
        }
        if !shown {
            p.report(0.0, "Finishing stamps");
            shown = true;
        }
    }
}

/// fix_export item 6: called every frame by `App::logic` after `stamp_bakes::pump`. While an export (polish item 2: or
/// a save) waits for the Final bakes, does nothing as long as a bake (or the wrap job it waits for) runs on a worker;
/// once none is pending it hands the job the snapshot. A Final that is due while no job runs (no worker thread could
/// start) is baked here (`settle`, the old behaviour) so the job never waits forever.
pub fn feed_waiting_snapshot(app: &mut WteApp) {
    if app.snapshot_feed.is_none() {
        return;
    }
    if crate::stamp_bakes::bakes_pending(&app.state) {
        if crate::stamp_bakes::bake_job_running(&app.state) {
            app.ctx.request_repaint_after(std::time::Duration::from_millis(50));
            return;
        }
        let what = app.snapshot_feed.as_ref().map_or("", |f| f.what);
        log::warn!("{what}: a Final bake is due but none runs; baking it on the UI thread");
        crate::stamp_bakes::settle(app);
    }
    let Some(feed) = app.snapshot_feed.take() else { return };
    let Some(snapshot) = app.state.doc().map(|d| d.snapshot()) else { return };
    log::info!("{}: the Final bakes landed after {:.0} ms; starting", feed.what, feed.since.elapsed().as_secs_f64() * 1000.0);
    // A cancelled job has dropped the receiver: nothing to do.
    let _ = feed.tx.send(snapshot);
}

/// Whether a process with this id is running (fix_export item 7: a temp file of an export whose process is gone is
/// stale). Windows: `OpenProcess` + `GetExitCodeProcess` (a process that exists but may not be opened counts as
/// running); elsewhere `/proc/<pid>`.
pub fn process_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const ERROR_ACCESS_DENIED: i32 = 5;
        const STILL_ACTIVE: u32 = 259;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        // SAFETY: plain Win32 calls with a valid out pointer; the handle is closed before returning.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED);
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok == 0 || code == STILL_ACTIVE
        }
    }
    #[cfg(not(windows))]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
}

/// Starts the "Extract Original Files" job on the in-memory document (works for a document that was never saved).
pub fn start_extract(app: &mut WteApp, dir: PathBuf) {
    if refuse_while_busy(app) {
        return;
    }
    let Some(doc) = app.state.doc_mut() else { return };
    let source = std::sync::Arc::clone(&doc.source);
    let blobs = doc.blobs.clone();
    // The finished pictures of the saved stickers (STICKER_PLAN 2.11); none for a document without stickers.
    let stickers = doc.sticker_extract_pictures();
    log::info!("extract original files to {}", dir.display());
    app.jobs.spawn(&app.ctx, "Extracting original files", true, move |p| {
        p.report(0.0, "Writing files");
        let result = wte_core::project::extract_bundle(&source, &blobs, &stickers, &dir, false).map_err(|e| e.to_string());
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
    // Stickers P9a: the link sessions, the watcher and the questions belong to the old document.
    app.state.link_ui.reset();
    // Stickers P6a: the Sticker view belongs to the document it was opened on.
    if app.state.view_mode == ViewMode::Sticker {
        crate::sticker::leave(app, None);
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
    // objects-tabs: the objects of the model (names of the first 24).
    const LOGGED_NAMES: usize = 24;
    let names: Vec<&str> = doc.mesh.objects.iter().take(LOGGED_NAMES).map(|o| o.name.as_str()).collect();
    let more = doc.mesh.objects.len().saturating_sub(LOGGED_NAMES);
    let more = if more > 0 { format!(" and {more} more") } else { String::new() };
    log::info!("objects: {} ({}{more})", doc.mesh.objects.len(), names.join(", "));
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
        selected_layers: Vec::new(),
        selected_objects: Default::default(),
        object_anchor: None,
        object_selection_rev: 0,
        object_reveal: None,
        sticker_edit: None,
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
        if let Some(mode) = ui.view_mode.filter(|m| !matches!(m, ViewMode::Preview | ViewMode::Sticker)) {
            app.state.view_mode = mode;
            app.state.last_edit_mode = mode;
        }
        if let Some(mut colors) = ui.colors {
            colors.swatches.resize(SWATCH_COUNT, [255, 255, 255, 255]);
            app.state.colors = colors;
        }
        // objects-tabs: the hidden objects of the saved view (view state: no history entry, not modified).
        if !ui.hidden_objects.is_empty() {
            let hidden = crate::object_ops::hidden_from_names(&session.doc, &ui.hidden_objects);
            match session.doc.set_hidden_objects(&hidden) {
                Ok(_) => {
                    if let Some(text) = crate::object_ops::restored_hidden_text(&session.doc) {
                        app.state.toasts.info(text);
                    }
                }
                Err(e) => log::warn!("could not restore the hidden objects: {e}"),
            }
        }
    }
    session.validate();
    app.state.session = Some(session);
    app.scene = None; // rebuilt for the new generation by the views; frees the old GPU resources now
    app.state.tools.stamp.source = crate::state::StampToolSource::None;
    app.state.tools.stamp.source_name.clear();
    // Stickers P6b: the tray selection and a waiting shared-picture edit belong to the old document.
    app.state.sticker_ui.tray_selected = None;
    app.state.sticker_ui.shared_pending = None;
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

/// Stickers P6b: native "save picture" dialog (Export Picture...): a PNG named after `default_name`.
pub fn pick_save_png(frame: &eframe::Frame, prefs: &Prefs, default_name: &str) -> Option<PathBuf> {
    let stem = wte_core::export::sanitize_file_stem(default_name);
    let stem = if stem.is_empty() { "sticker".to_owned() } else { stem };
    let mut dialog = rfd::FileDialog::new()
        .set_title("Export Picture")
        .add_filter("PNG picture", &["png"])
        .set_file_name(format!("{stem}.png"))
        .set_parent(frame);
    if let Some(dir) = dialog_dir(prefs) {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file().map(|p| if crate::dnd::extension_of(&p) == "png" { p } else { p.with_extension("png") })
}

/// Stickers P8: native "save PSD" dialog (Export as PSD...): a `.psd` named after `default_name`.
pub fn pick_save_psd(frame: &eframe::Frame, prefs: &Prefs, default_name: &str) -> Option<PathBuf> {
    let stem = wte_core::export::sanitize_file_stem(default_name);
    let stem = if stem.is_empty() { "sticker".to_owned() } else { stem };
    let mut dialog = rfd::FileDialog::new()
        .set_title("Export as PSD")
        .add_filter("Photoshop document", &["psd"])
        .set_file_name(format!("{stem}.psd"))
        .set_parent(frame);
    if let Some(dir) = dialog_dir(prefs) {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file().map(|p| if crate::dnd::extension_of(&p) == "psd" { p } else { p.with_extension("psd") })
}

/// Stickers P9a: native "choose program" dialog ("Where is photocraft.exe?", Help > PhotoCraft location...).
pub fn pick_program(frame: &eframe::Frame, prefs: &Prefs, title: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title).add_filter("Programs", &["exe"]).set_parent(frame);
    if let Some(dir) = prefs.photocraft_exe.as_deref().and_then(Path::parent).filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file()
}

/// Stickers P9a: native "open PSD" dialog (File > Import > Sticker from PSD...).
pub fn pick_psd(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Import Sticker from PSD")
        .add_filter("Photoshop document", &["psd", "psb"])
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
            selected_layers: Vec::new(),
            selected_objects: Default::default(),
            object_anchor: None,
            object_selection_rev: 0,
            object_reveal: None,
            sticker_edit: None,
        };
        let ui = UiState::capture(&state, &session);
        let value = serde_json::to_value(&ui).unwrap();
        let back: UiState = serde_json::from_value(value).unwrap();
        assert_eq!(back, ui);
        assert_eq!(back.camera.unwrap().yaw, 1.5);
        let partial: UiState = serde_json::from_value(serde_json::json!({ "active_set": 3, "unknown": true })).unwrap();
        assert_eq!(partial.active_set, Some(3));
        assert!(partial.camera.is_none());
        // objects-tabs: nothing hidden is not written at all; hidden names round-trip; a value of another kind is
        // ignored without losing the other fields; non-string entries are skipped.
        assert!(serde_json::to_value(&ui).unwrap().get("hidden_objects").is_none());
        assert!(partial.hidden_objects.is_empty());
        let mut session = session;
        session.doc = Document::sample(wte_core::SampleKind::Objects);
        session.doc.set_hidden_objects(&[false, true, false]).unwrap();
        let ui = UiState::capture(&state, &session);
        assert_eq!(ui.hidden_objects, ["Cube"]);
        let value = serde_json::to_value(&ui).unwrap();
        assert_eq!(value["hidden_objects"], serde_json::json!(["Cube"]));
        assert_eq!(serde_json::from_value::<UiState>(value).unwrap(), ui);
        for garbage in [serde_json::json!(7), serde_json::json!("Cube"), serde_json::json!({ "a": 1 }), serde_json::Value::Null] {
            let v: UiState = serde_json::from_value(serde_json::json!({ "active_set": 2, "hidden_objects": garbage })).unwrap();
            assert_eq!(v.active_set, Some(2), "{garbage}");
            assert!(v.hidden_objects.is_empty(), "{garbage}");
        }
        let v: UiState = serde_json::from_value(serde_json::json!({ "hidden_objects": ["Cube", 3, null, "Ball"] })).unwrap();
        assert_eq!(v.hidden_objects, ["Cube", "Ball"]);
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
            selected_layers: Vec::new(),
            selected_objects: Default::default(),
            object_anchor: None,
            object_selection_rev: 0,
            object_reveal: None,
            sticker_edit: None,
        }
    }

    /// Stickers P6a: the Sticker view is never written into the project's view state (the mode it returns to is), and
    /// a stored "sticker" (which no build writes) is not restored.
    #[test]
    fn the_sticker_view_is_never_written_to_the_ui_state() {
        let (mut state, _, _) = crate::sticker::tests::image_stamp_state(ViewMode::Split);
        crate::sticker::enter_in(&mut state, None);
        assert_eq!(state.view_mode, ViewMode::Sticker);
        let session = state.session.as_ref().unwrap();
        let ui = UiState::capture(&state, session);
        assert_eq!(ui.view_mode, Some(ViewMode::Split));
        let text = serde_json::to_string(&ui).unwrap();
        assert!(!text.contains("sticker"), "{text}");
        let read: UiState = serde_json::from_value(serde_json::json!({ "view_mode": "sticker" })).unwrap();
        assert_eq!(read.view_mode, Some(ViewMode::Sticker), "it reads, and install_document filters it out");
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
            selected_layers: Vec::new(),
            selected_objects: Default::default(),
            object_anchor: None,
            object_selection_rev: 0,
            object_reveal: None,
            sticker_edit: None,
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
    fn export_toasts_count_files_in_the_singular_and_plural() {
        let dir: PathBuf = ["E:", "x", "out"].iter().collect();
        let files = |n: usize| (0..n).map(|i| dir.join(format!("f{i}.png"))).collect::<Vec<_>>();
        let loose = |n| wte_core::ExportReport { files: files(n), bytes_written: 1, packaged: 0 };
        assert_eq!(export_toast_text(&loose(1), &dir), "Exported 1 file to out");
        assert_eq!(export_toast_text(&loose(2), &dir), "Exported 2 files to out");
        let zip = |n| wte_core::ExportReport { files: vec![dir.join("m_selection.zip")], bytes_written: 1, packaged: n };
        assert_eq!(export_toast_text(&zip(1), &dir), "Exported m_selection.zip (1 file) to out");
        assert_eq!(export_toast_text(&zip(8), &dir), "Exported m_selection.zip (8 files) to out");
    }

    #[test]
    fn this_process_runs_and_a_finished_one_does_not() {
        assert!(process_running(std::process::id()));
        // A child that has exited (and been reaped): its id is free.
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) { &["/c", "exit"][..] } else { &[][..] })
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        drop(child);
        assert!(!process_running(pid), "process {pid} still counts as running");
        assert!(!process_running(u32::MAX - 3));
    }

    #[test]
    fn folder_names_drop_the_path() {
        let p: PathBuf = ["E:", "x", "runs", "shot_export"].iter().collect();
        assert_eq!(folder_name(&p), "shot_export");
    }
}
