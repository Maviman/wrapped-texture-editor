// Owner: B5 brush flows (round 4).
//
// The life of the brush shape loaded from a model file (FEATURE_BRUSH 4.4): "Load...", the quiet reload when the
// shape is needed after a start, "Remove file shape", the background bake with its request number, the event table of
// 4.4, `sync_prefs` before every write of `prefs.json`; and the closing-openings question of ROUND4_PLAN section 7
// item 5 (`Dialog::CloseOpenings`, the answer, the choice stored per file, the Brush tab's "Openings" control).
// `dispatch.rs`, `io_flows.rs`, `app.rs` and `dialogs/mod.rs` call into this file with one line each.
//
// Every decision is made by a function on `AppState` alone (`begin_load`, `begin_reload`, `answer`, `change_close`,
// `forget`, `apply_result`, `startup_state`), which returns the bake to start as a `ShapeJob`; the `WteApp` wrappers
// only start that job on a worker thread (`spawn`). The tests drive the state functions and run the bake on the test
// thread (`run_job`), so the whole flow is tested without a window or a GPU.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use wte_core::{CloseMode, Progress, TipError, TipField};

use crate::action::AppAction;
use crate::app::WteApp;
use crate::jobs::JobOutput;
use crate::state::{
    AppState, BrushSettings, BrushType, Dialog, OpeningsDialogState, PanelHit, PendingShape, ReadyShape, ShapeChoice,
    SideTab, ToolKind,
};
use crate::view::ViewCx;

/// Title of the bake job (progress dialog of "Load...", status bar of a quiet reload).
const JOB_TITLE: &str = "Loading brush shape";
/// `height_ratio` under this (and not flat): the shape is thin, so a "Load..." sets Use to Through (FEATURE_BRUSH 5.7,
/// 6.6).
const THIN_RATIO: f32 = 0.25;
/// Title of the message window of a failed "Load...".
const FAILED_TITLE: &str = "Brush shape not loaded";

/// One bake to start on a worker thread: `JobOutput::BrushShape` with the same `request`, `path` and `close`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShapeJob {
    pub request: u64,
    pub path: PathBuf,
    pub close: CloseMode,
    /// "Load..." (and every bake the user asked for): modal, behind the progress dialog with Cancel. A quiet reload is
    /// not modal.
    pub modal: bool,
}

// ---------------------------------------------------------------------------------------------------------------------
// The entry points of dispatch, io_flows, app and the dialogs
// ---------------------------------------------------------------------------------------------------------------------

/// `AppAction::LoadBrushShape`: asks for a file when `path` is `None` (never in self-test mode), then bakes it in a
/// modal job (`JobOutput::BrushShape`) with the choice stored for that file, `CloseMode::Refuse` (ask) without one.
pub fn load(app: &mut WteApp, path: Option<PathBuf>, frame: &eframe::Frame) {
    let path = match path {
        Some(path) => path,
        None if app.state.selftest => {
            log::info!("LoadBrushShape(None) in self-test mode: no file dialog");
            return;
        }
        None => match pick_shape(frame, &app.state) {
            Some(path) => {
                if let Some(dir) = path.parent() {
                    app.state.prefs.last_dir = Some(dir.to_path_buf());
                }
                path
            }
            None => return,
        },
    };
    let for_eraser = app.state.tool == ToolKind::Eraser;
    let modal_running = app.jobs.modal_running();
    let job = begin_load(&mut app.state, path, true, for_eraser, None, modal_running);
    spawn(app, job);
}

/// `AppAction::ReloadBrushShape`: bakes `prefs.brush_shape_path` again in a non-modal job, quietly.
pub fn reload(app: &mut WteApp) {
    let job = begin_reload(&mut app.state);
    spawn(app, job);
}

/// `AppAction::RemoveBrushShape`: forgets the loaded shape and its path; tools with `ShapeChoice::File` go back to
/// the Ball (FEATURE_BRUSH 4.4).
pub fn remove(app: &mut WteApp) {
    forget(&mut app.state);
}

/// `AppAction::AnswerOpenings` (also Esc and `CloseDialog` while the question is open, with `None`): closes the
/// dialog; `Some(mode)` bakes the file again with that mode and remembers it for the file, `None` cancels the load.
pub fn answer_openings(app: &mut WteApp, choice: Option<CloseMode>) {
    let modal_running = app.jobs.modal_running();
    let job = answer(&mut app.state, choice, modal_running);
    spawn(app, job);
}

/// `AppAction::SetBrushShapeClose`: the Brush tab's "Openings" control; bakes the current file again with `close`.
pub fn set_close(app: &mut WteApp, close: CloseMode) {
    let modal_running = app.jobs.modal_running();
    let job = change_close(&mut app.state, close, modal_running);
    spawn(app, job);
}

/// A finished bake (`JobOutput::BrushShape`): applies the event table of FEATURE_BRUSH 4.4; a result whose `request`
/// is not the pending one is dropped; `Err(TipError::HasOpenings { .. })` opens `Dialog::CloseOpenings`.
pub fn on_job_output(
    app: &mut WteApp,
    request: u64,
    path: PathBuf,
    close: CloseMode,
    result: Result<Arc<TipField>, TipError>,
    cancel_requested: bool,
) {
    apply_result(&mut app.state, request, path, close, result, cancel_requested);
}

/// At start (after the preferences are read): copies the brush and eraser settings from `Prefs` into the tools and
/// pushes `ReloadBrushShape` when a remembered shape is needed (FEATURE_BRUSH 4.4).
pub fn startup(app: &mut WteApp) {
    startup_state(&mut app.state);
}

/// Copies the brush and eraser settings (sanitized) into `state.prefs`; called directly before every `prefs::save`.
pub fn sync_prefs(state: &mut AppState) {
    state.prefs.brush = state.tools.brush.clone().sanitized();
    state.prefs.eraser = state.tools.eraser.clone().sanitized();
}

// ---------------------------------------------------------------------------------------------------------------------
// The flow on `AppState` (FEATURE_BRUSH 4.4, plan section 7 item 5)
// ---------------------------------------------------------------------------------------------------------------------

/// Starts a bake of `path`: takes the next request number, stores it as `pending` (so every older result is dropped)
/// and returns the job. `close`: `None` = the choice stored for the file, else `Refuse` (ask when it has openings).
/// Refused with the "please wait" toast while a modal job runs (only for a bake the user asked for; a quiet reload is
/// never started then, see `begin_reload`).
pub(crate) fn begin_load(
    state: &mut AppState,
    path: PathBuf,
    by_user: bool,
    for_eraser: bool,
    close: Option<CloseMode>,
    modal_running: bool,
) -> Option<ShapeJob> {
    if modal_running {
        state.toasts.warn("Please wait for the running operation to finish");
        return None;
    }
    let close = close.unwrap_or_else(|| crate::prefs::stored_close(&state.prefs, &path));
    let shape = &mut state.brush_shape;
    let request = shape.next_request;
    shape.next_request += 1;
    shape.pending = Some(PendingShape { request, path: path.clone(), by_user, for_eraser, close });
    log::info!(
        "brush shape: {} {} (request {request}, openings: {}{})",
        if by_user { "load" } else { "quiet reload of" },
        path.display(),
        close.label(),
        if for_eraser { ", for the eraser" } else { "" }
    );
    Some(ShapeJob { request, path, close, modal: by_user })
}

/// `ReloadBrushShape`: a quiet, non-modal bake of the remembered path; nothing when no path is remembered or a load
/// is already running (that load wins anyway, FEATURE_BRUSH 4.4).
pub(crate) fn begin_reload(state: &mut AppState) -> Option<ShapeJob> {
    let path = state.prefs.brush_shape_path.clone()?;
    if state.brush_shape.pending.is_some() {
        log::info!("brush shape: reload of {} skipped, a load is running", path.display());
        return None;
    }
    begin_load(state, path, false, false, None, false)
}

/// The answer of `Dialog::CloseOpenings`: closes the dialog; `Some(mode)` bakes the file again with that mode (for the
/// same tool and in the same manner as the load that asked), `None` cancels: the shape in use is kept.
pub(crate) fn answer(state: &mut AppState, choice: Option<CloseMode>, modal_running: bool) -> Option<ShapeJob> {
    let Dialog::CloseOpenings(st) = std::mem::replace(&mut state.dialog, Dialog::None) else {
        return None;
    };
    match choice {
        Some(mode) if mode != CloseMode::Refuse => {
            log::info!("brush shape: openings of {} closed with \"{}\"", st.path.display(), mode.label());
            let job = begin_load(state, st.path.clone(), st.by_user, st.for_eraser, Some(mode), modal_running);
            if job.is_none() {
                // Refused while busy: ask again later rather than lose the question.
                state.dialog = Dialog::CloseOpenings(st);
            }
            job
        }
        _ => {
            log::info!("brush shape: closing the openings of {} cancelled", st.path.display());
            state.toasts.info("Loading cancelled");
            if !st.by_user {
                // A quiet reload that is not completed leaves the tools as a failed reload does.
                back_to_ball_without_shape(state);
            }
            None
        }
    }
}

/// `SetBrushShapeClose`: bakes the current (else the remembered) shape file again with `close`; the result replaces
/// the shape and stores the choice for the file. `Refuse` forgets the stored choice, so the bake asks again.
pub(crate) fn change_close(state: &mut AppState, close: CloseMode, modal_running: bool) -> Option<ShapeJob> {
    let path = match &state.brush_shape.current {
        Some(current) => current.path.clone(),
        None => state.prefs.brush_shape_path.clone()?,
    };
    if close == CloseMode::Refuse {
        crate::prefs::forget_close(&mut state.prefs, &path);
    }
    let for_eraser = state.tool == ToolKind::Eraser;
    begin_load(state, path, true, for_eraser, Some(close), modal_running)
}

/// "Remove file shape": `current`, `pending`, `last_error` and the remembered path (with its stored choice) cleared;
/// every tool on File goes back to the Ball. A result still running is dropped by its number.
pub(crate) fn forget(state: &mut AppState) {
    let shape = &mut state.brush_shape;
    shape.current = None;
    shape.pending = None;
    shape.last_error = None;
    if let Some(path) = state.prefs.brush_shape_path.take() {
        crate::prefs::forget_close(&mut state.prefs, &path);
    }
    if matches!(state.dialog, Dialog::CloseOpenings(_)) {
        state.dialog = Dialog::None;
    }
    for tool in [&mut state.tools.brush, &mut state.tools.eraser] {
        if tool.shape == ShapeChoice::File {
            tool.shape = ShapeChoice::Ball;
        }
    }
    log::info!("brush shape removed");
}

/// The bake of a job, on the calling thread (a worker in the app, the test thread in the tests).
pub(crate) fn run_job(job: &ShapeJob, progress: &dyn Progress) -> JobOutput {
    let started = std::time::Instant::now();
    let result = TipField::bake_from_path(&job.path, job.close, progress).map(Arc::new);
    log::info!(
        "brush shape bake of {} ({}): {} in {:.2} s",
        job.path.display(),
        job.close.label(),
        result.as_ref().map_or_else(|e| format!("{e:?}"), |f| format!("ok, {:?}", f.report())),
        started.elapsed().as_secs_f64()
    );
    JobOutput::BrushShape { request: job.request, path: job.path.clone(), close: job.close, result }
}

/// A finished bake: the event table of FEATURE_BRUSH 4.4 with the closing-openings question.
pub(crate) fn apply_result(
    state: &mut AppState,
    request: u64,
    path: PathBuf,
    close: CloseMode,
    result: Result<Arc<TipField>, TipError>,
    cancel_requested: bool,
) {
    let Some(pending) = state.brush_shape.pending.take_if(|p| p.request == request) else {
        log::info!("brush shape: result {request} for {} dropped (not the newest load)", path.display());
        return;
    };
    let name = file_name(&path);
    match result {
        Ok(field) => {
            let report = field.report().clone();
            // `Refuse` = the file had no openings to close (`ReadyShape::close`).
            let had_openings = report.openings_closed > 0 || report.closed_parts;
            let close = if had_openings { close } else { CloseMode::Refuse };
            if close != CloseMode::Refuse {
                crate::prefs::remember_close(&mut state.prefs, &path, close);
            }
            state.brush_shape.current =
                Some(ReadyShape { path: path.clone(), name: name.clone(), field, close, closed_parts: report.closed_parts });
            state.brush_shape.last_error = None;
            if !pending.by_user {
                log::info!("brush shape {name} reloaded");
                return;
            }
            state.prefs.brush_shape_path = Some(path);
            let thin = !report.flat && report.height_ratio < THIN_RATIO;
            let tool = if pending.for_eraser { &mut state.tools.eraser } else { &mut state.tools.brush };
            tool.kind = BrushType::Sphere;
            tool.shape = ShapeChoice::File;
            tool.through = report.flat || thin;
            state.side_tab = SideTab::Brush;
            let toasts = &mut state.toasts;
            toasts.info(format!("Brush shape loaded: {name} ({} triangles)", thousands(report.triangles)));
            if report.partial && close != CloseMode::ClosedPartsOnly {
                toasts.warn(format!(
                    "Only the closed parts of {name} are used as the brush shape. Open or very thin parts are left out."
                ));
            }
            if report.thin {
                toasts.warn(format!("Parts of {name} are thinner than the brush can show."));
            }
            if report.flat {
                toasts.info(format!("{name} is flat. Its outline is pressed straight through the surface (Use: Through)."));
            } else if thin {
                toasts.info(format!(
                    "{name} is a thin shape, so Use was set to Through. With Solid it paints only where the surface \
                     passes through the shape."
                ));
            }
        }
        Err(TipError::HasOpenings { count, largest, closed_parts }) => {
            log::info!("brush shape {name}: {count} opening(s), largest {:.1} %, closed parts {closed_parts}", largest * 100.0);
            if matches!(state.dialog, Dialog::None) {
                state.dialog = Dialog::CloseOpenings(Box::new(OpeningsDialogState {
                    path,
                    count,
                    largest,
                    closed_parts,
                    choice: CloseMode::FlatCaps,
                    by_user: pending.by_user,
                    for_eraser: pending.for_eraser,
                }));
            } else {
                // Another dialog is open (only possible for a quiet reload): no second question on top of it.
                let error = TipError::HasOpenings { count, largest, closed_parts };
                failed(state, &pending, path, &error.to_string(), false);
            }
        }
        Err(TipError::Cancelled) => {
            state.toasts.info("Loading cancelled");
            if !pending.by_user {
                back_to_ball_without_shape(state);
            }
        }
        Err(_) if cancel_requested => state.toasts.info("Loading cancelled"),
        Err(error) => {
            let missing = matches!(error, TipError::Import(_)) && crate::io_flows::cannot_open_text(&path).is_some();
            let text = match &error {
                TipError::Import(_) => crate::io_flows::cannot_open_text(&path).unwrap_or_else(|| error.to_string()),
                _ => error.to_string(),
            };
            failed(state, &pending, path, &text, missing);
        }
    }
}

/// A load or reload that failed (rows 3 and 7 of the table in FEATURE_BRUSH 4.4): `current` and the remembered path
/// are kept, `last_error` names the file. "Load...": an error toast and the message window. Quiet reload: one warning
/// toast, and the tools on File without a shape go back to the Ball.
fn failed(state: &mut AppState, pending: &PendingShape, path: PathBuf, text: &str, missing: bool) {
    log::warn!("brush shape {} not loaded: {text}", path.display());
    state.brush_shape.last_error = Some((path.clone(), text.to_owned()));
    if pending.by_user {
        state.toasts.error(text);
        state.dialog = Dialog::Message {
            title: FAILED_TITLE.to_owned(),
            lines: vec![format!("{} cannot be used as a brush shape.", path.display()), text.to_owned()],
        };
    } else {
        if missing {
            state.toasts.warn(format!("Brush shape not found: {}. Using the ball.", path.display()));
        } else {
            state.toasts.warn(format!("Brush shape {} cannot be used: {text} Using the ball.", file_name(&path)));
        }
        back_to_ball_without_shape(state);
    }
}

/// Every tool with `ShapeChoice::File` goes back to the Ball when no shape is loaded (FEATURE_BRUSH 4.4: the "no shape"
/// toast cannot then repeat at every press).
fn back_to_ball_without_shape(state: &mut AppState) {
    if state.brush_shape.current.is_some() {
        return;
    }
    for tool in [&mut state.tools.brush, &mut state.tools.eraser] {
        if tool.shape == ShapeChoice::File {
            tool.shape = ShapeChoice::Ball;
        }
    }
}

/// Start-up: the brush and eraser settings of the preferences go into the tools; a remembered shape that one of them
/// uses (Sphere with File) is reloaded (pushes `ReloadBrushShape`). Returns whether it was pushed.
pub(crate) fn startup_state(state: &mut AppState) -> bool {
    state.tools.brush = state.prefs.brush.clone().sanitized();
    state.tools.eraser = state.prefs.eraser.clone().sanitized();
    let uses_file = |b: &BrushSettings| b.kind == BrushType::Sphere && b.shape == ShapeChoice::File;
    let needed = state.prefs.brush_shape_path.is_some() && (uses_file(&state.tools.brush) || uses_file(&state.tools.eraser));
    if needed {
        state.push(AppAction::ReloadBrushShape);
    }
    needed
}

/// Starts a job returned by the flow on a worker thread.
fn spawn(app: &mut WteApp, job: Option<ShapeJob>) {
    let Some(job) = job else { return };
    let modal = job.modal;
    app.jobs.spawn(&app.ctx, JOB_TITLE, modal, move |p| {
        p.report(0.0, "Reading the model");
        run_job(&job, p)
    });
}

/// The native "Load brush shape" file dialog (synchronous, parented to the window).
fn pick_shape(frame: &eframe::Frame, state: &AppState) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Load Brush Shape")
        .add_filter("3D models (OBJ, glTF, GLB, FBX)", &crate::dnd::MODEL_EXTENSIONS)
        .set_parent(frame);
    let dir = state.brush_shape.current.as_ref().and_then(|c| c.path.parent().map(Path::to_path_buf));
    if let Some(dir) = dir.or_else(|| state.prefs.last_dir.clone()).filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file()
}

/// The file name of a path, for messages.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// `1280` -> `"1,280"`.
fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// The dialog
// ---------------------------------------------------------------------------------------------------------------------

/// The choices of the dialog for a file, in order: the four ways to close (`CloseMode::CHOICES`), "Use only the closed
/// parts" only when the file has closed parts (contract request B1-3).
pub(crate) fn dialog_choices(closed_parts: bool) -> Vec<CloseMode> {
    CloseMode::CHOICES.into_iter().filter(|&m| m != CloseMode::ClosedPartsOnly || closed_parts).collect()
}

/// The explanation under each choice of the dialog.
pub(crate) fn choice_note(mode: CloseMode) -> &'static str {
    match mode {
        CloseMode::FlatCaps => "Each opening is filled with a flat patch across its edge.",
        CloseMode::RoundedCaps => "Each opening is filled with a patch that bulges outward.",
        CloseMode::FillHollows => "The model is closed from outside. This also fills deep hollows and pockets.",
        CloseMode::ClosedPartsOnly => "The open parts are left out; only the closed parts are used.",
        CloseMode::Refuse => "",
    }
}

/// The first line of the dialog: the file, the number of openings and the size of the largest.
pub(crate) fn dialog_message(st: &OpeningsDialogState) -> String {
    let percent = if st.largest.is_finite() { (st.largest * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let size = if percent > 0.0 && percent < 0.5 { "under 1 %".to_owned() } else { format!("{percent:.0} %") };
    let (one, many) = (st.count == 1, st.count != 1);
    format!(
        "{} has {} opening{}; {} is {size} of the model's size. A brush shape must be closed. How should {} be closed?",
        file_name(&st.path),
        st.count,
        if many { "s" } else { "" },
        if one { "it" } else { "the largest" },
        if one { "it" } else { "they" }
    )
}

/// Draws `Dialog::CloseOpenings`: the file, the number of openings and the size of the largest, the four choices
/// (`CloseMode::CHOICES`; "Use only the closed parts" only when `st.closed_parts`) and Cancel. Answers with
/// `AppAction::AnswerOpenings`. The rects of the choices and the two buttons go to `panel_rects` (self-test).
pub fn show_openings_dialog(ctx: &egui::Context, cx: &mut ViewCx<'_>, st: &mut OpeningsDialogState) {
    openings_dialog(ctx, cx.state, st);
}

/// `show_openings_dialog` on the state alone (the headless dialog test draws it without a GPU).
fn openings_dialog(ctx: &egui::Context, state: &mut AppState, st: &mut OpeningsDialogState) {
    let choices = dialog_choices(st.closed_parts);
    if !choices.contains(&st.choice) {
        st.choice = CloseMode::FlatCaps;
    }
    let mut rects: Vec<(PanelHit, egui::Rect)> = Vec::new();
    let max = crate::dialogs::max_dialog_size(ctx);
    let response = crate::dialogs::modal("close_openings").show(ctx, |ui| {
        let width = max.x.min(520.0);
        ui.set_min_width(width.min(420.0));
        ui.set_max_width(width);
        ui.heading("Close the openings of the brush shape?");
        ui.separator();
        ui.add(egui::Label::new(dialog_message(st)).wrap());
        ui.weak(st.path.display().to_string()).on_hover_text("The shape file");
        ui.add_space(6.0);
        for (index, &mode) in choices.iter().enumerate() {
            let label = if mode == CloseMode::FlatCaps { format!("{} (default)", mode.label()) } else { mode.label().to_owned() };
            let radio = ui.radio_value(&mut st.choice, mode, label).on_hover_text(choice_note(mode));
            rects.push((PanelHit::OpeningsChoice(index as u8), radio.rect));
            ui.indent(("close_openings_note", index), |ui| {
                ui.weak(choice_note(mode));
            });
        }
        ui.add_space(4.0);
        ui.weak("The choice is remembered for this file and can be changed in the Brush tab (Openings).");
        ui.separator();
        let mut answer = None;
        crate::dialogs::button_row(ui, |ui| {
            let cancel = ui.button("Cancel");
            rects.push((PanelHit::OpeningsCancel, cancel.rect));
            if cancel.clicked() {
                answer = Some(None);
            }
            let ok = ui.button(format!("Close with {}", st.choice.label().to_lowercase()));
            rects.push((PanelHit::OpeningsOk, ok.rect));
            if ok.clicked() {
                answer = Some(Some(st.choice));
            }
        });
        answer
    });
    for (hit, rect) in rects {
        state.panel_rects.insert(hit, rect);
    }
    if let Some(choice) = response.inner {
        state.push(AppAction::AnswerOpenings(choice));
    }
}

#[cfg(test)]
mod tests {
    use glam::{Mat4, Vec3};
    use wte_core::samples;
    use wte_core::{NoProgress, RawMesh};

    use super::*;
    use crate::state::Prefs;

    /// A fresh app state as the self-test has it (no prefs read), the Brush tool active.
    fn fresh() -> AppState {
        AppState::new(Prefs::default(), true)
    }

    /// Writes `raw` as `<name>.obj` + `<name>.mtl` into `dir`.
    fn write_obj(dir: &Path, name: &str, raw: &RawMesh) -> PathBuf {
        let (obj, mtl) = samples::to_obj(raw, &format!("{name}.mtl"), &[]);
        std::fs::write(dir.join(format!("{name}.mtl")), mtl).unwrap();
        let path = dir.join(format!("{name}.obj"));
        std::fs::write(&path, obj).unwrap();
        path
    }

    /// The cube of FEATURE_BRUSH 8 in all four formats: OBJ, GLB, glTF, ASCII FBX.
    fn cube_files(dir: &Path) -> [PathBuf; 4] {
        let cube = samples::seam_cube(1.0);
        let obj = write_obj(dir, "cube_shape", &cube);
        let glb = dir.join("cube_shape.glb");
        std::fs::write(&glb, samples::to_glb(&[(cube.clone(), Mat4::IDENTITY)])).unwrap();
        let gltf = dir.join("cube_shape.gltf");
        std::fs::write(&gltf, samples::to_gltf(&cube, Mat4::IDENTITY)).unwrap();
        let fbx = dir.join("cube_shape.fbx");
        std::fs::write(&fbx, samples::to_fbx_ascii(&cube, 1.0, [1.0, 1.0, 1.0])).unwrap();
        [obj, glb, gltf, fbx]
    }

    /// Runs the bake of `job` here and hands its result to the flow, as `io_flows::on_job_output` does.
    fn deliver(state: &mut AppState, job: &ShapeJob, progress: &dyn Progress, cancel_requested: bool) {
        let JobOutput::BrushShape { request, path, close, result } = run_job(job, progress) else {
            panic!("not a brush shape result")
        };
        apply_result(state, request, path, close, result, cancel_requested);
    }

    /// "Load..." of `path` with `tool` active, run to its end.
    fn user_load(state: &mut AppState, path: &Path, tool: ToolKind) {
        state.tool = tool;
        let job = begin_load(state, path.to_path_buf(), true, tool == ToolKind::Eraser, None, false).expect("a job");
        assert!(job.modal, "\"Load...\" is a modal job");
        deliver(state, &job, &NoProgress, false);
    }

    fn file_brush(through: bool) -> BrushSettings {
        BrushSettings { kind: BrushType::Sphere, shape: ShapeChoice::File, through, ..BrushSettings::default() }
    }

    /// The state after a successful "Load..." of the cube from `path` by `tool`.
    fn assert_cube_loaded(state: &AppState, path: &Path, tool: ToolKind) {
        let current = state.brush_shape.current.as_ref().expect("a current shape");
        assert_eq!(current.path, path);
        assert_eq!(current.name, file_name(path));
        assert_eq!(current.close, CloseMode::Refuse, "a closed cube has no openings");
        assert!(!current.closed_parts);
        let report = current.field.report();
        assert_eq!(report.triangles, 12, "{}", path.display());
        assert!(!report.partial && !report.flat, "{report:?}");
        assert!(state.brush_shape.pending.is_none() && state.brush_shape.last_error.is_none());
        assert_eq!(state.prefs.brush_shape_path.as_deref(), Some(path));
        let (changed, unchanged) = if tool == ToolKind::Eraser {
            (&state.tools.eraser, &state.tools.brush)
        } else {
            (&state.tools.brush, &state.tools.eraser)
        };
        assert_eq!(*changed, file_brush(false), "the issuing tool: Sphere, File, Solid");
        assert_eq!(*unchanged, BrushSettings::default(), "the other tool is not changed");
        assert_eq!(state.side_tab, SideTab::Brush);
        assert!(matches!(state.dialog, Dialog::None));
        assert_eq!(state.toasts.error_count(), 0);
        let loaded = format!("Brush shape loaded: {} (12 triangles)", file_name(path));
        assert!(state.toasts.visible_texts().contains(&loaded.as_str()), "{:?}", state.toasts.visible_texts());
    }

    /// A Progress that is cancelled from the start (the progress dialog's Cancel).
    struct Cancelled;
    impl Progress for Cancelled {
        fn report(&self, _: f32, _: &str) {}
        fn cancelled(&self) -> bool {
            true
        }
    }

    /// A16 (FEATURE_BRUSH 8.2): every row of the table in 4.4 through the load flow, with the cube in all four formats.
    #[test]
    fn a16_load_flow_every_row() {
        let dir = wte_core::testutil::temp_dir("brush_flows_a16");
        let [obj, glb, gltf, fbx] = cube_files(&dir);

        // "Load..." with the Brush active, then with the Eraser active.
        let mut state = fresh();
        user_load(&mut state, &obj, ToolKind::Brush);
        assert_cube_loaded(&state, &obj, ToolKind::Brush);
        let obj_fraction = state.brush_shape.current.as_ref().unwrap().field.report().inside_fraction;
        let mut state = fresh();
        user_load(&mut state, &obj, ToolKind::Eraser);
        assert_cube_loaded(&state, &obj, ToolKind::Eraser);

        // The same cube as GLB, glTF and ASCII FBX: the same state, each with its own path.
        for path in [&glb, &gltf, &fbx] {
            let mut state = fresh();
            user_load(&mut state, path, ToolKind::Brush);
            assert_cube_loaded(&state, path, ToolKind::Brush);
            let fraction = state.brush_shape.current.as_ref().unwrap().field.report().inside_fraction;
            assert!(
                (fraction - obj_fraction).abs() <= 0.02 * obj_fraction,
                "{}: {fraction} against {obj_fraction}",
                path.display()
            );
        }

        // A flat star: Use is Through.
        let star = write_obj(&dir, "star", &samples::flat_star());
        let mut state = fresh();
        user_load(&mut state, &star, ToolKind::Brush);
        assert!(state.brush_shape.current.as_ref().unwrap().field.report().flat);
        assert_eq!(state.tools.brush, file_brush(true));
        assert!(state.toasts.visible_texts().iter().any(|t| t.contains("star.obj is flat")));

        // A missing file and a tilted quad while the cube is current: error toast and message, the cube kept.
        let mut state = fresh();
        user_load(&mut state, &obj, ToolKind::Brush);
        let missing = dir.join("no_such_shape.obj");
        user_load(&mut state, &missing, ToolKind::Brush);
        assert_eq!(state.toasts.error_count(), 1);
        assert!(state.toasts.error_texts()[0].contains("no_such_shape.obj"), "{:?}", state.toasts.error_texts());
        assert!(matches!(&state.dialog, Dialog::Message { title, .. } if title == FAILED_TITLE));
        assert_eq!(state.brush_shape.current.as_ref().unwrap().path, obj);
        assert_eq!(state.prefs.brush_shape_path.as_deref(), Some(obj.as_path()));
        assert_eq!(state.brush_shape.last_error.as_ref().map(|e| e.0.clone()), Some(missing));
        state.dialog = Dialog::None;
        let quad = write_obj(&dir, "tilted_quad", &samples::tilted_quad());
        user_load(&mut state, &quad, ToolKind::Brush);
        assert_eq!(state.toasts.error_count(), 2);
        assert_eq!(state.toasts.error_texts()[1], TipError::NoInterior.to_string());
        assert_eq!(state.brush_shape.current.as_ref().unwrap().path, obj);
        assert_eq!(state.prefs.brush_shape_path.as_deref(), Some(obj.as_path()));
        assert_eq!(state.brush_shape.last_error.as_ref().map(|e| e.0.clone()), Some(quad));
        assert_eq!(state.tools.brush, file_brush(false), "a failed load changes no setting");

        // Cancel (the progress dialog's Cancel): "Loading cancelled", the cube kept, no error.
        let mut state = fresh();
        user_load(&mut state, &obj, ToolKind::Brush);
        let job = begin_load(&mut state, glb.clone(), true, false, None, false).unwrap();
        deliver(&mut state, &job, &Cancelled, true);
        assert_eq!(state.toasts.error_count(), 0);
        assert_eq!(state.toasts.visible_texts().last(), Some(&"Loading cancelled"));
        assert_eq!(state.brush_shape.current.as_ref().unwrap().path, obj);
        assert!(state.brush_shape.pending.is_none());

        // Quiet reload of an existing path: the shape, no toast, tab and settings unchanged.
        let mut state = fresh();
        state.prefs.brush_shape_path = Some(glb.clone());
        let job = begin_reload(&mut state).expect("a reload");
        assert!(!job.modal, "a quiet reload is not modal");
        deliver(&mut state, &job, &NoProgress, false);
        assert_eq!(state.brush_shape.current.as_ref().unwrap().path, glb);
        assert!(state.toasts.visible_texts().is_empty(), "{:?}", state.toasts.visible_texts());
        assert_eq!(state.side_tab, SideTab::Layers);
        assert_eq!(state.tools.brush, BrushSettings::default());
        assert_eq!(state.tools.eraser, BrushSettings::default());

        // Quiet reload of a missing path with the brush on Sphere / File: one warning, back to the Ball, path kept.
        let mut state = fresh();
        let gone = dir.join("gone.obj");
        state.prefs.brush_shape_path = Some(gone.clone());
        state.tools.brush = file_brush(false);
        let job = begin_reload(&mut state).unwrap();
        deliver(&mut state, &job, &NoProgress, false);
        assert_eq!(state.toasts.error_count(), 0);
        let expected = format!("Brush shape not found: {}. Using the ball.", gone.display());
        assert_eq!(state.toasts.visible_texts(), [expected.as_str()]);
        assert_eq!(state.tools.brush.shape, ShapeChoice::Ball, "a press afterwards paints the ball without a toast");
        assert_eq!(state.prefs.brush_shape_path, Some(gone));
        assert!(state.brush_shape.current.is_none() && state.brush_shape.last_error.is_some());

        // Start-up check: nothing with both tools on Circular; one ReloadBrushShape with the brush on Sphere / File.
        let mut state = AppState::new(Prefs { brush_shape_path: Some(obj.clone()), ..Prefs::default() }, false);
        assert!(!startup_state(&mut state));
        assert!(state.actions.is_empty());
        let prefs = Prefs { brush_shape_path: Some(obj.clone()), brush: file_brush(false), ..Prefs::default() };
        let mut state = AppState::new(prefs, false);
        assert!(startup_state(&mut state));
        assert_eq!(state.tools.brush, file_brush(false), "the settings of the preferences go into the tools");
        assert!(matches!(&state.actions[..], [AppAction::ReloadBrushShape]), "{:?}", state.actions);
        let mut state = AppState::new(Prefs { eraser: file_brush(false), ..Prefs::default() }, false);
        assert!(!startup_state(&mut state), "no remembered path: nothing to reload");

        // Remove: current, last_error, the remembered path and its choice cleared; tools on File back to the Ball.
        let mut state = fresh();
        user_load(&mut state, &obj, ToolKind::Brush);
        state.tools.eraser = file_brush(false);
        state.brush_shape.last_error = Some((dir.join("x.obj"), "x".into()));
        crate::prefs::remember_close(&mut state.prefs, &obj, CloseMode::FlatCaps);
        forget(&mut state);
        assert!(state.brush_shape.current.is_none() && state.brush_shape.last_error.is_none());
        assert!(state.prefs.brush_shape_path.is_none());
        assert!(state.prefs.brush_shape_close.is_empty());
        assert_eq!((state.tools.brush.shape, state.tools.eraser.shape), (ShapeChoice::Ball, ShapeChoice::Ball));
        assert_eq!(state.tools.brush.kind, BrushType::Sphere, "only the shape goes back");

        // "Load..." is refused while a modal job runs.
        let mut state = fresh();
        assert!(begin_load(&mut state, obj.clone(), true, false, None, true).is_none());
        assert!(state.brush_shape.pending.is_none());
        assert_eq!(state.toasts.visible_texts(), ["Please wait for the running operation to finish"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A20 (FEATURE_BRUSH 8.2): a quiet reload pending and a "Load..." started: in either order of arrival the user's
    /// shape wins and the dropped result shows nothing; a result after "Remove file shape" is dropped.
    #[test]
    fn a20_two_shape_jobs() {
        let dir = wte_core::testutil::temp_dir("brush_flows_a20");
        let [obj, glb, ..] = cube_files(&dir);
        for user_first in [false, true] {
            let mut state = fresh();
            state.prefs.brush_shape_path = Some(glb.clone());
            let reload = begin_reload(&mut state).unwrap();
            assert!(begin_reload(&mut state).is_none(), "a second reload is not started while one runs");
            let load = begin_load(&mut state, obj.clone(), true, false, None, false).unwrap();
            assert!(load.request > reload.request);
            let (first, second) = if user_first { (&load, &reload) } else { (&reload, &load) };
            deliver(&mut state, first, &NoProgress, false);
            deliver(&mut state, second, &NoProgress, false);
            assert_eq!(state.brush_shape.current.as_ref().unwrap().path, obj, "user first: {user_first}");
            assert_eq!(state.prefs.brush_shape_path.as_deref(), Some(obj.as_path()));
            assert!(state.brush_shape.pending.is_none());
            assert_eq!(state.toasts.visible_texts(), ["Brush shape loaded: cube_shape.obj (12 triangles)"]);
        }
        // A result that arrives after "Remove file shape" is dropped.
        let mut state = fresh();
        let load = begin_load(&mut state, obj.clone(), true, false, None, false).unwrap();
        forget(&mut state);
        deliver(&mut state, &load, &NoProgress, false);
        assert!(state.brush_shape.current.is_none() && state.prefs.brush_shape_path.is_none());
        assert!(state.toasts.visible_texts().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Plan section 7 item 5: a file with openings makes the app ask; the answer bakes it, is stored with the path and
    /// a reload closes the same way without asking; Cancel keeps the shape in use; "Use only the closed parts" is
    /// offered only for a file with closed parts; the Brush tab's Openings control bakes again with another mode.
    #[test]
    fn open_shapes_ask_how_to_close_and_remember_the_answer() {
        let dir = wte_core::testutil::temp_dir("brush_flows_openings");
        let [cube, ..] = cube_files(&dir);
        let open = write_obj(&dir, "open_box", &samples::open_box(1.0));

        let mut state = fresh();
        user_load(&mut state, &cube, ToolKind::Brush);
        user_load(&mut state, &open, ToolKind::Eraser);
        let Dialog::CloseOpenings(st) = &state.dialog else { panic!("no question: {:?}", state.dialog) };
        assert_eq!((st.path.clone(), st.count, st.closed_parts, st.choice), (open.clone(), 1, false, CloseMode::FlatCaps));
        assert!(st.by_user && st.for_eraser);
        assert!((st.largest - (2f32 / 3.0).sqrt()).abs() < 0.01, "{}", st.largest);
        let message = dialog_message(st);
        assert!(message.starts_with("open_box.obj has 1 opening; it is 82 % of the model's size."), "{message}");
        assert_eq!(dialog_choices(st.closed_parts), [CloseMode::FlatCaps, CloseMode::RoundedCaps, CloseMode::FillHollows]);
        assert!(choice_note(CloseMode::FillHollows).contains("also fills deep hollows"));
        assert_eq!(state.brush_shape.current.as_ref().unwrap().path, cube, "the shape in use is kept while asking");
        assert!(state.brush_shape.pending.is_none());
        assert_eq!(state.toasts.error_count(), 0);

        // Flat caps (the default answer): baked, stored with the path, given to the eraser that asked.
        let job = answer(&mut state, Some(CloseMode::FlatCaps), false).expect("a bake");
        assert_eq!((job.close, job.modal), (CloseMode::FlatCaps, true));
        assert!(matches!(state.dialog, Dialog::None));
        deliver(&mut state, &job, &NoProgress, false);
        let current = state.brush_shape.current.as_ref().unwrap();
        assert_eq!((current.path.clone(), current.close, current.closed_parts), (open.clone(), CloseMode::FlatCaps, false));
        assert_eq!(current.field.report().openings_closed, 1);
        assert!(current.field.principal_sections().iter().all(|s| !s.is_empty()), "the preview has lines");
        assert_eq!(crate::prefs::stored_close(&state.prefs, &open), CloseMode::FlatCaps);
        assert_eq!(state.prefs.brush_shape_path.as_deref(), Some(open.as_path()));
        assert_eq!(state.tools.eraser, file_brush(false));
        assert_eq!(state.tools.brush, file_brush(false), "the brush keeps the cube's settings");
        let flat_fraction = current.field.report().inside_fraction;

        // A reload (next start) closes the same way without asking.
        state.brush_shape.current = None;
        let job = begin_reload(&mut state).unwrap();
        assert_eq!(job.close, CloseMode::FlatCaps);
        deliver(&mut state, &job, &NoProgress, false);
        assert!(matches!(state.dialog, Dialog::None));
        assert_eq!(state.brush_shape.current.as_ref().unwrap().close, CloseMode::FlatCaps);

        // The Openings control of the Brush tab: Rounded caps bakes again (the cap bulges: more inside), stored.
        let job = change_close(&mut state, CloseMode::RoundedCaps, false).unwrap();
        deliver(&mut state, &job, &NoProgress, false);
        let current = state.brush_shape.current.as_ref().unwrap();
        assert_eq!(current.close, CloseMode::RoundedCaps);
        assert_eq!(crate::prefs::stored_close(&state.prefs, &open), CloseMode::RoundedCaps);
        let round_fraction = current.field.report().inside_fraction;
        log::info!("open box: inside fraction flat caps {flat_fraction}, rounded caps {round_fraction}");
        assert_ne!(round_fraction, flat_fraction);
        let job = change_close(&mut state, CloseMode::FillHollows, false).unwrap();
        deliver(&mut state, &job, &NoProgress, false);
        assert_eq!(state.brush_shape.current.as_ref().unwrap().close, CloseMode::FillHollows);
        assert_eq!(crate::prefs::stored_close(&state.prefs, &open), CloseMode::FillHollows, "Fill all hollows is stored too");
        assert_eq!(state.toasts.error_count(), 0);

        // Cancel: the shape in use is kept, nothing stored for the new file.
        let cup = write_obj(&dir, "cup", &samples::cup());
        user_load(&mut state, &cup, ToolKind::Brush);
        assert!(matches!(&state.dialog, Dialog::CloseOpenings(st) if st.path == cup));
        assert!(answer(&mut state, None, false).is_none());
        assert!(matches!(state.dialog, Dialog::None));
        assert_eq!(state.toasts.visible_texts().last(), Some(&"Loading cancelled"));
        assert_eq!(state.brush_shape.current.as_ref().unwrap().path, open);
        assert_eq!(crate::prefs::stored_close(&state.prefs, &cup), CloseMode::Refuse);
        assert_eq!(state.prefs.brush_shape_path.as_deref(), Some(open.as_path()));

        // A cup and a closed cube in one file: "Use only the closed parts" is offered and bakes the cube alone.
        let mut both = samples::cup();
        let mut extra = samples::seam_cube(1.0);
        for t in &mut extra.triangles {
            t.positions = t.positions.map(|p| p + Vec3::new(3.0, 0.5, 0.0));
            t.material = 0;
        }
        both.triangles.extend(extra.triangles);
        let both_path = write_obj(&dir, "cup_and_cube", &both);
        let mut state = fresh();
        user_load(&mut state, &both_path, ToolKind::Brush);
        let Dialog::CloseOpenings(st) = &state.dialog else { panic!("no question: {:?}", state.dialog) };
        assert!(st.closed_parts);
        assert_eq!(dialog_choices(true).last(), Some(&CloseMode::ClosedPartsOnly));
        let job = answer(&mut state, Some(CloseMode::ClosedPartsOnly), false).unwrap();
        deliver(&mut state, &job, &NoProgress, false);
        let current = state.brush_shape.current.as_ref().expect("the closed part");
        assert_eq!((current.close, current.closed_parts), (CloseMode::ClosedPartsOnly, true));
        assert_eq!(state.tools.brush, file_brush(false));
        assert_eq!(state.toasts.error_count(), 0);

        // A quiet reload of a file with openings and no stored answer asks too; Cancel then leaves the ball.
        let mut state = fresh();
        state.prefs.brush_shape_path = Some(open.clone());
        state.tools.brush = file_brush(false);
        let job = begin_reload(&mut state).unwrap();
        deliver(&mut state, &job, &NoProgress, false);
        assert!(matches!(&state.dialog, Dialog::CloseOpenings(st) if !st.by_user && st.path == open));
        answer(&mut state, None, false);
        assert_eq!(state.tools.brush.shape, ShapeChoice::Ball);
        assert_eq!(state.prefs.brush_shape_path.as_deref(), Some(open.as_path()), "the path stays remembered");

        // The same quiet reload answered with Flat caps: the bake stays quiet (not modal, no toast, settings and tab
        // unchanged) and the answer is stored, so the next reload does not ask.
        let mut state = fresh();
        state.prefs.brush_shape_path = Some(open.clone());
        state.tools.brush = file_brush(false);
        let job = begin_reload(&mut state).unwrap();
        deliver(&mut state, &job, &NoProgress, false);
        let job = answer(&mut state, Some(CloseMode::FlatCaps), false).expect("a bake");
        assert!(!job.modal, "the answer to a quiet reload bakes quietly");
        deliver(&mut state, &job, &NoProgress, false);
        assert_eq!(state.brush_shape.current.as_ref().map(|c| c.close), Some(CloseMode::FlatCaps));
        assert!(state.toasts.visible_texts().is_empty(), "{:?}", state.toasts.visible_texts());
        assert_eq!((state.tools.brush.clone(), state.side_tab), (file_brush(false), SideTab::Layers));
        assert_eq!(crate::prefs::stored_close(&state.prefs, &open), CloseMode::FlatCaps);
        assert_eq!(begin_reload(&mut state).map(|j| j.close), Some(CloseMode::FlatCaps));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A10, last part: `sync_prefs` copies the tools (sanitized) into the preferences, so every write holds them.
    #[test]
    fn sync_prefs_copies_the_tools() {
        let mut state = fresh();
        state.tools.brush.size_px = 123.0;
        state.tools.eraser.turn_deg = 900.0;
        sync_prefs(&mut state);
        assert_eq!(state.prefs.brush.size_px, 123.0);
        assert_eq!(state.prefs.eraser.turn_deg, 180.0);
        assert_eq!(state.prefs.brush, state.tools.brush);
    }

    /// The dialog draws its choices and buttons (rects for the self-test), offers "Use only the closed parts" only
    /// with closed parts, keeps the radio choice and answers with `AnswerOpenings` when a button is clicked.
    #[test]
    fn the_openings_dialog_draws_and_answers() {
        for closed_parts in [false, true] {
            let mut state = fresh();
            let mut st = OpeningsDialogState {
                path: PathBuf::from(r"E:\s\open.obj"),
                count: 3,
                largest: 0.004,
                closed_parts,
                choice: CloseMode::FlatCaps,
                by_user: true,
                for_eraser: false,
            };
            assert!(dialog_message(&st).starts_with("open.obj has 3 openings; the largest is under 1 %"), "{}", dialog_message(&st));
            let ctx = egui::Context::default();
            let frame = |state: &mut AppState, st: &mut OpeningsDialogState, events: Vec<egui::Event>| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                    events,
                    ..Default::default()
                };
                crate::theme::run_test_pass(&ctx, input, |ui| openings_dialog(ui.ctx(), state, st));
            };
            let click = |at: egui::Pos2| {
                let button = |pressed| egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                vec![egui::Event::PointerMoved(at), button(true), button(false)]
            };
            frame(&mut state, &mut st, Vec::new());
            frame(&mut state, &mut st, Vec::new());
            let choices = (0..4u8).filter(|i| state.panel_rects.contains_key(&PanelHit::OpeningsChoice(*i))).count();
            assert_eq!(choices, if closed_parts { 4 } else { 3 });
            assert!(state.actions.is_empty(), "drawing answers nothing");
            // "Rounded caps", then the answer button.
            let rounded = state.panel_rects[&PanelHit::OpeningsChoice(1)].center();
            frame(&mut state, &mut st, click(rounded));
            frame(&mut state, &mut st, Vec::new());
            assert_eq!(st.choice, CloseMode::RoundedCaps);
            assert!(state.actions.is_empty());
            let ok = state.panel_rects[&PanelHit::OpeningsOk].center();
            frame(&mut state, &mut st, click(ok));
            assert!(matches!(&state.actions[..], [AppAction::AnswerOpenings(Some(CloseMode::RoundedCaps))]), "{:?}", state.actions);
            state.actions.clear();
            frame(&mut state, &mut st, Vec::new());
            let cancel = state.panel_rects[&PanelHit::OpeningsCancel].center();
            frame(&mut state, &mut st, click(cancel));
            assert!(matches!(&state.actions[..], [AppAction::AnswerOpenings(None)]), "{:?}", state.actions);
        }
    }
}
