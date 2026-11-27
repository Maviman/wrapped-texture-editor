// Owner: round 5, package A (R5-1). Final stamp bakes off the UI thread (ARCHITECTURE.md 8.9, 5.20 "Deferred Final
// bakes"): the document marks a stamp unbaked when an edit ends; `pump` takes its bake job (`next_final_bake`) and runs
// it on a worker, one at a time; `on_output` hands the result back (`apply_final_bake`); `settle` waits for the bake in
// flight and bakes the rest on the UI thread before the commands that keep stamp pixels (save, export, the export
// dialog, merge down, rasterize).
//
// Round 8 (R8-2, ROUND8_PLAN 4.1): the Preview bakes of wrapped stamps run on a worker too (`WrapJobs`): one silent
// job at a time, latest wins (the job is made from the stamp as it is when the job starts); the stamp shows its last
// finished picture until the next one lands; a landed picture whose stamp moved since asks for the next job
// (`due_again`). The selected wrapped stamp without a cached parameterization (after an open, an undo) gets it from a
// param-only job. A Final of a stamp whose wrap job is in flight starts after that job lands. `settle` never waits for
// wrap jobs (their outputs are stale after the flush).
//
// Round 9 (package B, T8): a Final waits for its stamp's wrap job only when that job was made for the Final's placement
// (it brings the parameterization the Final reuses); a job made for an earlier placement of the drag does not hold the
// Final back (its output lands stale). A selected 3D stamp warms up the document's mesh data (the shared texels; for a
// wrapped stamp also the welded mesh graph) on a background thread outside `Jobs` (`WrapJobs::request_warmup`), so the
// first wrap job does not pay the graph's build and the panel can name shared texels.

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui;
use wte_core::{FinalBake, Placement, StampBakeOutput, StampId, StampJobKind, WrapPreview};

use crate::app::WteApp;
use crate::jobs::{JobOutput, Jobs};
use crate::state::AppState;

/// Status-bar titles of the bake job (non-modal; the status bar shows the title).
const TITLE_STAMP: &str = "Finishing stamp";
const TITLE_TEXT: &str = "Finishing text";
/// Titles (thread names) of the silent wrap jobs; the status bar does not show them.
const TITLE_WRAP_PREVIEW: &str = "Wrap preview";
const TITLE_WRAP_PARAM: &str = "Wrap fit";

/// Round 8 (R8-2): the wrap Preview or param-only job in flight, a Preview output held while a stroke is open, the
/// param-only request of the selected stamp and counters. Lives in `AppState::wrap_jobs`.
#[derive(Default)]
pub struct WrapJobs {
    in_flight: Option<WrapFlight>,
    /// A Preview output that arrived while a stroke was open, with its scene generation; applied when it ends.
    held: Option<(u64, Box<StampBakeOutput>)>,
    /// The selected wrapped stamp whose parameterization is not cached: asked for by the Transform tool in every frame
    /// it is needed (`request_param`), taken by the next `pump` when no wrap job is in flight.
    want_param: Option<StampId>,
    /// The stamp and placement whose param-only job found no surface to wrap onto: not asked for again. Objects-tabs:
    /// with the document's visibility generation of the job (after a visibility change it is asked for again).
    pub(crate) param_failed: Option<(StampId, Placement, u64)>,
    /// What the jobs did (tests, the scenario and the log).
    pub counters: WrapJobCounters,
    /// Round 9 (package B, T8 / T9): a selected 3D stamp asked for the warm-up of the mesh's derived data (taken by
    /// `after_tools`; `Some(true)`: a wrapped stamp, also the graph), and a warm-up job runs.
    want_warm: Option<bool>,
    warm_thread: Option<std::thread::JoinHandle<f64>>,
}

/// Counts of `WrapJobs` since the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WrapJobCounters {
    pub previews_started: u64,
    pub previews_applied: u64,
    /// Applied previews whose stamp had moved since the job started (the next job follows).
    pub previews_due_again: u64,
    /// Preview outputs dropped (Esc, undo, a Final, another document, a removed stamp).
    pub previews_stale: u64,
    pub params_started: u64,
    pub params_applied: u64,
    /// Parameterizations made on the UI thread by `TransformTool::ensure_wrap_param` (synchronous mode only).
    pub ui_params: u64,
    /// Round 9 (package B): warm-up jobs of the mesh graph started.
    pub warmups_started: u64,
}

/// The one wrap job running on a worker.
struct WrapFlight {
    id: StampId,
    kind: StampJobKind,
    /// `Session::scene_generation` when it started.
    generation: u64,
    started: Instant,
    /// The stamp's placement when the job was made (a param-only job that brings nothing for it: no surface).
    placement: Placement,
    /// Objects-tabs: the document's `visibility_generation` when the job was made. A param-only job that lands after a
    /// visibility change is not cached (`Document::apply_wrap_param`) and proves nothing about the surface.
    visibility: u64,
}

impl WrapJobs {
    /// The Transform tool asks for the parameterization of the selected wrapped stamp (async mode).
    pub(crate) fn request_param(&mut self, id: StampId) {
        self.want_param = Some(id);
    }

    /// Round 9 (package B, T8 / T9): a selected 3D stamp wants the document's shared texels found and, when `graph` (a
    /// wrapped stamp), the mesh graph built (off the UI thread).
    pub(crate) fn request_warmup(&mut self, graph: bool) {
        self.want_warm = Some(self.want_warm.unwrap_or(false) || graph);
    }

    /// The stamp and kind of the wrap job in flight.
    pub(crate) fn in_flight(&self) -> Option<(StampId, StampJobKind)> {
        self.in_flight.as_ref().map(|f| (f.id, f.kind))
    }
}

/// The bake in flight and an output held while a stroke is open. Lives in `AppState::final_bakes`.
#[derive(Default)]
pub struct FinalBakes {
    in_flight: Option<InFlight>,
    /// An output that arrived while a stroke was open, with its scene generation; applied when the stroke ends.
    held: Option<(u64, Box<StampBakeOutput>)>,
    /// When each waiting stamp was first seen unbaked at the end of a frame's dispatch (`after_dispatch`): the frame of
    /// its Final. The "applied after T ms" log line measures T from here (else from the start of the job).
    marked: HashMap<StampId, Instant>,
    /// Round 8: the stamp and the "applied after" milliseconds of the last applied Final (the scenario's log).
    last_applied: Option<(StampId, f64)>,
    /// accept_fixes item C: the `settle` calls that had work to do (waited for or baked a Final on the UI thread),
    /// for the self-test: an action that must not block the UI leaves it unchanged.
    settles: u64,
}

/// The one bake job running on a worker.
struct InFlight {
    id: StampId,
    title: &'static str,
    /// `Session::scene_generation` when it started.
    generation: u64,
    started: Instant,
}

impl FinalBakes {
    /// True while a bake runs on a worker.
    #[cfg(test)]
    pub fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Round 8: the stamp and the "applied after" milliseconds of the last applied Final bake.
    pub(crate) fn last_applied(&self) -> Option<(StampId, f64)> {
        self.last_applied
    }

    /// accept_fixes item C: how many `settle` calls waited for or baked a Final on the UI thread.
    pub(crate) fn settle_count(&self) -> u64 {
        self.settles
    }
}

/// Called from `App::logic` right after the job poll: applies a held output once no stroke is open, starts the next
/// bake when none is in flight (also while a pending edit is open on another stamp; none while a stroke is open) and
/// updates `final_bakes_pending`.
pub fn pump(app: &mut WteApp) {
    pump_with(&mut app.state, &mut app.jobs, &app.ctx);
}

/// `pump` on the parts of the app it needs (tests drive it without a GPU).
pub(crate) fn pump_with(state: &mut AppState, jobs: &mut Jobs, ctx: &egui::Context) {
    if let Some((generation, output)) = state.final_bakes.held.take() {
        if state.doc().is_some_and(|d| d.stroke_active()) {
            state.final_bakes.held = Some((generation, output));
        } else {
            apply_output(state, generation, output, None);
        }
    }
    // Round 8: the wrap Preview / param-only jobs first; a Final of the stamp whose wrap job is in flight waits for it.
    pump_wrap(state, jobs, ctx);
    if state.final_bakes.in_flight.is_none()
        && state.final_bakes.held.is_none()
        && !final_waits_for_wrap_job(state)
        && let Some(session) = state.session.as_mut()
        && let Some(job) = session.doc.next_final_bake()
    {
        let (id, text, generation) = (job.stamp(), job.is_text(), session.scene_generation);
        let title = if text { TITLE_TEXT } else { TITLE_STAMP };
        log::info!("final bake: stamp {} started ({})", id.0, if text { "text" } else { "image" });
        let started = Instant::now();
        let spawned = jobs.spawn_staged(ctx, title, false, "", move |_| JobOutput::StampBake {
            generation,
            output: Box::new(job.run_isolated()),
        });
        if spawned {
            state.final_bakes.in_flight = Some(InFlight { id, title, generation, started });
        } else {
            // No worker thread: bake here, as round 4 did.
            session.doc.settle_final_bakes();
        }
    }
    update_pending(state);
}

/// Round 8 (R8-2): applies a held Preview output once no stroke is open; when no wrap job is in flight starts the
/// Preview job of the lowest waiting wrapped stamp (`Document::next_wrap_preview_job`, made from the stamp as it is
/// now: latest wins), else the param-only job the Transform tool asked for. None while a stroke is open.
fn pump_wrap(state: &mut AppState, jobs: &mut Jobs, ctx: &egui::Context) {
    if let Some((generation, output)) = state.wrap_jobs.held.take() {
        if state.doc().is_some_and(|d| d.stroke_active()) {
            state.wrap_jobs.held = Some((generation, output));
        } else {
            apply_wrap(state, generation, output, None);
        }
    }
    let want = state.wrap_jobs.want_param.take();
    if state.wrap_jobs.in_flight.is_some() || state.wrap_jobs.held.is_some() {
        return;
    }
    let Some(session) = state.session.as_mut() else { return };
    // Only while the document defers wrapped Previews (the asynchronous mode): otherwise a wrapped stamp waiting in the
    // deferred set waits for the views' throttled Preview bake on the UI thread (`view::bake_stale_previews`), as in
    // round 6, and `next_wrap_preview_job` must not take it.
    if session.doc.stroke_active() || !session.doc.defers_wrap_previews() {
        return;
    }
    let generation = session.scene_generation;
    let visibility = session.doc.visibility_generation();
    let job = match session.doc.next_wrap_preview_job() {
        Some(job) => job,
        None => match want.and_then(|id| session.doc.wrap_param_job(id)) {
            Some(job) => job,
            None => return,
        },
    };
    let (id, kind) = (job.stamp(), job.kind());
    let Some(placement) = session.doc.stamp(id).map(|s| s.placement) else { return };
    let preview = kind == StampJobKind::Preview;
    let spawned = if preview {
        jobs.spawn_silent(ctx, TITLE_WRAP_PREVIEW, move |_| JobOutput::WrapPreview { generation, output: Box::new(job.run_isolated()) })
    } else {
        jobs.spawn_silent(ctx, TITLE_WRAP_PARAM, move |_| JobOutput::WrapParam { generation, output: Box::new(job.run_isolated()) })
    };
    if !spawned {
        // No worker thread: the wrapped Previews go back to the UI thread (round 6); the Final bake brings the picture
        // and the parameterization of the stamp whose job was lost.
        log::warn!("wrap {}: stamp {}: no worker thread, wrapped Previews are baked on the UI thread", if preview { "preview" } else { "fit" }, id.0);
        session.doc.set_defer_wrap_previews(false);
        return;
    }
    let counters = &mut state.wrap_jobs.counters;
    if preview {
        counters.previews_started += 1;
        log::info!("wrap preview: stamp {} job started", id.0);
    } else {
        counters.params_started += 1;
        log::info!("wrap fit: stamp {} param-only job started", id.0);
    }
    state.wrap_jobs.in_flight = Some(WrapFlight { id, kind, generation, started: Instant::now(), placement, visibility });
}

/// True while the wrap job in flight belongs to a stamp of the open document that waits for its Final bake and was made
/// for the stamp's placement now: the Final starts after that job lands and reuses its parameterization (plan 4.1).
/// Round 9 (package B, T8): a job made for another placement (an earlier frame of the drag) does not hold the Final
/// back: the Final starts at once and the job's output lands stale.
fn final_waits_for_wrap_job(state: &AppState) -> bool {
    let (Some(flight), Some(session)) = (state.wrap_jobs.in_flight.as_ref(), state.session.as_ref()) else { return false };
    flight.generation == session.scene_generation
        && session.doc.stamp_is_unbaked(flight.id)
        && session.doc.stamp(flight.id).map(|s| s.placement) == Some(flight.placement)
}

/// Called from `io_flows::on_job_output` with the output of a wrap Preview or param-only job.
pub fn on_wrap_output(app: &mut WteApp, generation: u64, output: Box<StampBakeOutput>) {
    on_wrap_output_with(&mut app.state, generation, output);
}

/// `on_wrap_output` on the app state.
pub(crate) fn on_wrap_output_with(state: &mut AppState, generation: u64, output: Box<StampBakeOutput>) {
    // One wrap job in flight at a time: this is its output.
    let flight = state.wrap_jobs.in_flight.take();
    apply_wrap(state, generation, output, flight);
    update_pending(state);
}

/// Applies a wrap job's output to the open document when it belongs to it (same scene generation); a Preview output
/// is held while a stroke is open. Logs the outcome ("wrap preview: ..." / "wrap fit: ...").
fn apply_wrap(state: &mut AppState, generation: u64, output: Box<StampBakeOutput>, flight: Option<WrapFlight>) {
    let id = output.stamp();
    let kind = output.kind();
    let worker_ms = output.elapsed().as_secs_f64() * 1000.0;
    let after = flight.as_ref().map_or(worker_ms, |f| f.started.elapsed().as_secs_f64() * 1000.0);
    let preview = kind == StampJobKind::Preview;
    let what = if preview { "wrap preview" } else { "wrap fit" };
    let Some(session) = state.session.as_mut().filter(|s| s.scene_generation == generation) else {
        log::info!("{what}: stamp {} result dropped (other document)", id.0);
        if preview {
            state.wrap_jobs.counters.previews_stale += 1;
        }
        return;
    };
    let counters = &mut state.wrap_jobs.counters;
    if preview {
        if session.doc.stroke_active() {
            log::debug!("wrap preview: stamp {} held until the stroke ends", id.0);
            state.wrap_jobs.held = Some((generation, output));
            return;
        }
        match session.doc.apply_wrap_preview(*output) {
            WrapPreview::Applied { due_again } => {
                counters.previews_applied += 1;
                counters.previews_due_again += u64::from(due_again);
                log::info!("wrap preview: stamp {} applied after {after:.0} ms (worker {worker_ms:.0} ms, due again: {due_again})", id.0);
            }
            WrapPreview::Stale => {
                counters.previews_stale += 1;
                log::info!("wrap preview: stamp {} result dropped (stale) after {after:.0} ms (worker {worker_ms:.0} ms)", id.0);
            }
        }
        return;
    }
    if session.doc.apply_wrap_param(*output) {
        counters.params_applied += 1;
        log::info!("wrap fit: stamp {} parameterization landed after {after:.0} ms (worker {worker_ms:.0} ms)", id.0);
    } else if let Some(f) = flight
        && session.doc.stamp(id).map(|s| s.placement) == Some(f.placement)
        && session.doc.visibility_generation() == f.visibility
        && session.doc.stamp_wrap_stats(id).is_none()
    {
        // Made for the stamp as it still is (and the objects shown as they still are) and nothing came: no surface
        // within reach. Not asked for again.
        log::info!("wrap fit: stamp {} has no surface to wrap onto ({after:.0} ms)", id.0);
        state.wrap_jobs.param_failed = Some((id, f.placement, f.visibility));
    } else {
        log::info!("wrap fit: stamp {} result dropped (stale) after {after:.0} ms", id.0);
    }
}

/// Round 8: called from `App::logic` after the Transform tool's `ensure_wrap_param`: starts the param-only job that
/// tool just asked for when no wrap job is in flight (else a later `pump` does, when the job in flight lands), and
/// updates `wrap_jobs_pending`.
pub(crate) fn after_tools(state: &mut AppState, jobs: &mut Jobs, ctx: &egui::Context) {
    if state.wrap_jobs.want_param.is_some() && state.wrap_jobs.in_flight.is_none() {
        pump_wrap(state, jobs, ctx);
    }
    warm_up(state);
    update_pending(state);
}

/// Round 9 (package B, T8 / T9): starts the warm-up of the open document's mesh data when a selected 3D stamp asked
/// for it, something is not built yet and no warm-up runs: a background thread outside `Jobs` (it changes no pixels
/// and has no output: it fills the document's `WrapMeshCell`, which the wrap jobs share; a wrap job that needs the
/// graph meanwhile waits for it in the cell). A thread that cannot start leaves the build to the first wrap job.
fn warm_up(state: &mut AppState) {
    let jobs = &mut state.wrap_jobs;
    if let Some(handle) = jobs.warm_thread.take_if(|h| h.is_finished()) {
        match handle.join() {
            Ok(ms) => log::info!("wrap mesh: warm-up ended after {ms:.0} ms"),
            Err(_) => log::warn!("wrap mesh: the warm-up thread failed"),
        }
    }
    let Some(graph) = jobs.want_warm.take() else { return };
    if jobs.warm_thread.is_some() {
        return;
    }
    let Some(session) = state.session.as_mut() else { return };
    let Some(warm) = session.doc.wrap_mesh_warmup(graph) else { return };
    match std::thread::Builder::new().name("wrap-warm-up".to_owned()).spawn(move || warm.run()) {
        Ok(handle) => {
            state.wrap_jobs.warm_thread = Some(handle);
            state.wrap_jobs.counters.warmups_started += 1;
            log::info!("wrap mesh: warm-up started (graph: {graph})");
        }
        Err(e) => log::warn!("wrap mesh: the warm-up thread cannot start ({e}); the first wrap job builds the graph"),
    }
}

/// Called from `App::ui` after the frame's actions were dispatched (a Final of a gizmo release or of a tool gesture
/// that ends on a timer happens earlier in the same frame). Two things:
/// - a Final that left a stamp waiting while no bake is in flight asks for the next frame, whose `pump` starts the bake.
///   Without it a Final made in a frame without input (the Transform tool's wheel gesture ends on a timer, a stroke
///   whose paced work ends after the release) would leave the Preview on screen until the next mouse move;
/// - every stamp first seen waiting here gets its mark time (the frame of its Final) for the log line. While a pending
///   edit is open the marks are dropped (the edited stamp is not waiting; its Final marks it again).
pub(crate) fn after_dispatch(state: &mut AppState, ctx: &egui::Context) {
    if needs_frame_after_dispatch(state) {
        ctx.request_repaint();
    }
}

/// The work of `after_dispatch`; true when the next frame must come even without input (its `pump` starts the bake).
fn needs_frame_after_dispatch(state: &mut AppState) -> bool {
    // check:B: a Final that waits for its stamp's wrap job does not start in the next frame either; the job's landing
    // asks for the frame that starts it (`Jobs`: a finished worker requests a repaint). Without this every frame of the
    // wait (up to a whole Preview job: 0.3-0.6 s on ABeautifulGame, seconds on high_poly_sphere) asked for the next one,
    // and the software renderer drew at the display rate beside the worker.
    let waits_for_wrap = final_waits_for_wrap_job(state);
    let bakes = &mut state.final_bakes;
    let Some(doc) = state.session.as_ref().map(|s| &s.doc) else {
        bakes.marked.clear();
        return false;
    };
    if doc.has_pending_edit() {
        bakes.marked.clear();
    } else {
        bakes.marked.retain(|id, _| doc.stamp_is_unbaked(*id));
        for stamp in doc.stamps.iter().filter(|s| doc.stamp_is_unbaked(s.id)) {
            bakes.marked.entry(stamp.id).or_insert_with(Instant::now);
        }
    }
    let waiting = bakes.held.is_some() || doc.has_unbaked_stamps();
    // Round 8: a wrapped stamp's Preview edit of this frame (a wheel notch, typing) waits for the next frame's `pump`
    // to start its job.
    let wrap_waiting = state.wrap_jobs.in_flight.is_none() && doc.has_stale_wrap_previews();
    ((bakes.in_flight.is_none() && waiting && !waits_for_wrap) || wrap_waiting) && !doc.stroke_active()
}

/// fix_export item 6: true while `settle` would have something to do for the open document: a Final bake of it in
/// flight, an output held, or a stamp still waiting for its Final.
pub(crate) fn bakes_pending(state: &AppState) -> bool {
    let current = state.session.as_ref().map(|s| s.scene_generation);
    state.final_bakes.in_flight.as_ref().is_some_and(|f| Some(f.generation) == current)
        || state.final_bakes.held.is_some()
        || state.doc().is_some_and(|d| d.has_unbaked_stamps())
}

/// fix_export item 6: true while a Final bake or a wrap job runs on a worker, i.e. the pending Finals progress without
/// the UI thread (`pump` starts the next one when it lands).
pub(crate) fn bake_job_running(state: &AppState) -> bool {
    state.final_bakes.in_flight.is_some() || state.wrap_jobs.in_flight.is_some()
}

/// Called from `io_flows::on_job_output` with the output of a bake job.
pub fn on_output(app: &mut WteApp, generation: u64, output: Box<StampBakeOutput>) {
    on_output_with(&mut app.state, generation, output);
}

/// `on_output` on the app state.
pub(crate) fn on_output_with(state: &mut AppState, generation: u64, output: Box<StampBakeOutput>) {
    // One bake in flight at a time: this is its output.
    let started = state.final_bakes.in_flight.take().map(|f| f.started);
    apply_output(state, generation, output, started);
    update_pending(state);
}

/// Waits for the bake in flight of the open document and applies it, then bakes every other waiting stamp on the UI
/// thread (`Document::settle_final_bakes`). The one place the UI thread blocks on a worker (4.6). Called before merge
/// down and rasterize. Export (fix_export item 6) and save (polish item 2) do not call it: their jobs wait behind the
/// progress dialog until the bakes have landed (`io_flows::feed_waiting_snapshot`), and so do not freeze
/// the UI for a wrapped stamp's Final (seconds on a large model). Stamps of an open pending
/// edit are not baked (the callers that keep pixels flush first; merge down and rasterize bake their own stamp in the
/// core, rule 8).
pub fn settle(app: &mut WteApp) {
    settle_with(&mut app.state, &mut app.jobs);
}

/// `settle` on the parts of the app it needs.
pub(crate) fn settle_with(state: &mut AppState, jobs: &mut Jobs) {
    let current = state.session.as_ref().map(|s| s.scene_generation);
    let start = Instant::now();
    let mut worked = false;
    // A bake of an earlier document is left to finish on its own (its result is dropped).
    if state.final_bakes.in_flight.as_ref().is_some_and(|f| Some(f.generation) == current)
        && let Some(flight) = state.final_bakes.in_flight.take()
    {
        worked = true;
        match jobs.wait_for(flight.title) {
            Some(JobOutput::StampBake { generation, output }) => apply_output(state, generation, output, Some(flight.started)),
            Some(_) => log::error!("final bake: the job \"{}\" delivered another output", flight.title),
            None => log::warn!("final bake: stamp {} delivered no result", flight.id.0),
        }
    }
    if let Some((generation, output)) = state.final_bakes.held.take() {
        if state.doc().is_some_and(|d| d.stroke_active()) {
            state.final_bakes.held = Some((generation, output));
        } else {
            worked = true;
            apply_output(state, generation, output, None);
        }
    }
    if let Some(doc) = state.doc_mut()
        && doc.has_unbaked_stamps()
    {
        worked = true;
        doc.settle_final_bakes();
    }
    if worked {
        state.final_bakes.settles += 1;
        log::info!("final bake: settle waited {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    }
    debug_assert!(
        state.doc().is_none_or(|d| d.stroke_active() || !d.has_unbaked_stamps()),
        "unbaked stamps after settle"
    );
    update_pending(state);
}

/// Applies one output to the open document when it belongs to it (same scene generation); logs the outcome.
fn apply_output(state: &mut AppState, generation: u64, output: Box<StampBakeOutput>, started: Option<Instant>) {
    let id = output.stamp();
    let what = if output.is_text() { "text" } else { "stamp" };
    let worker_ms = output.elapsed().as_secs_f64() * 1000.0;
    let Some(session) = state.session.as_mut().filter(|s| s.scene_generation == generation) else {
        log::info!("final bake: stamp {} result dropped (other document)", id.0);
        return;
    };
    match session.doc.apply_final_bake(*output) {
        FinalBake::Applied { id, empty } => {
            // From the frame of the Final when it was marked (`after_dispatch`), else from the start of the job.
            let since = state.final_bakes.marked.remove(&id).or(started);
            let after = since.map_or(worker_ms, |t| t.elapsed().as_secs_f64() * 1000.0);
            log::info!("final bake: stamp {} applied after {after:.0} ms (worker {worker_ms:.0} ms, empty: {empty})", id.0);
            state.final_bakes.last_applied = Some((id, after));
            if state.await_first_bake == Some(id) {
                state.await_first_bake = None;
                if empty {
                    state.toasts.warn(crate::tools::stamp_place::no_texels_text(what));
                }
            }
        }
        FinalBake::Busy(output) => {
            log::debug!("final bake: stamp {} held until the stroke ends", id.0);
            state.final_bakes.held = Some((generation, output));
        }
        FinalBake::Stale => log::info!("final bake: stamp {} result dropped (stale)", id.0),
        FinalBake::Failed { id } => {
            log::warn!("final bake: stamp {} failed, the earlier picture stays", id.0);
            if state.await_first_bake == Some(id) {
                state.await_first_bake = None;
            }
        }
    }
    if let Some(awaited) = state.await_first_bake
        && state.doc().is_none_or(|d| d.stamp(awaited).is_none())
    {
        state.await_first_bake = None;
    }
}

/// `final_bakes_pending` = a bake in flight, an output held, or a stamp of the open document waiting for its bake.
/// Round 8: `wrap_jobs_pending` = a wrap job in flight, a Preview output held, a wrapped stamp waiting for its Preview
/// job, or a param-only job asked for.
fn update_pending(state: &mut AppState) {
    let waiting = state.doc().is_some_and(|d| d.has_unbaked_stamps());
    state.final_bakes_pending = state.final_bakes.in_flight.is_some() || state.final_bakes.held.is_some() || waiting;
    let wrap = &state.wrap_jobs;
    let wrap_waiting = state.doc().is_some_and(|d| d.has_stale_wrap_previews());
    state.wrap_jobs_pending = wrap.in_flight.is_some() || wrap.held.is_some() || wrap.want_param.is_some() || wrap_waiting;
}

/// Drops what belongs to the previous document when a new one is installed (`io_flows::install_document`): a held
/// output and the awaited first bake. A bake in flight finishes on its own; its result is dropped by its generation.
/// Round 8: the same for the wrap jobs (a held Preview, the param-only request and its failure mark).
pub(crate) fn reset_for_new_document(state: &mut AppState) {
    state.final_bakes.held = None;
    state.final_bakes.marked.clear();
    state.await_first_bake = None;
    state.wrap_jobs.held = None;
    state.wrap_jobs.want_param = None;
    state.wrap_jobs.param_failed = None;
    state.wrap_jobs.want_warm = None;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use glam::{Vec2, Vec3};
    use wte_core::{BrushParams, Commit, LayerKind, Placement, PngLevel, Projector, SampleKind, StampSource, encode_png, samples};

    use super::*;
    use crate::state::ToolKind;
    use crate::tools::ToolController;
    use crate::tools::test_support::{pointer, state_with, xf_3d};

    /// The sphere sample with the Stamp tool's image set up; `deferred`: the app's asynchronous mode.
    fn stamp_state(deferred: bool) -> AppState {
        let mut state = state_with(SampleKind::Sphere);
        state.async_final_bakes = deferred;
        let img = samples::sample_stamp_image(64);
        let png = encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        doc.set_defer_final_bakes(deferred);
        let blob = doc.add_image_blob("ring.png", Arc::from(png)).unwrap();
        state.tools.stamp.source = crate::state::StampToolSource::Blob(blob);
        state.tools.stamp.source_name = "ring.png".to_owned();
        state
    }

    /// A click of the Stamp tool on the sphere; the placed stamp.
    fn place(state: &mut AppState) -> StampId {
        state.tool = ToolKind::Stamp;
        let xf = xf_3d(state);
        let mut tools = ToolController::new();
        let p = pointer(state, &xf, xf.rect.center() + egui::vec2(40.0, -30.0), egui::Modifiers::NONE);
        tools.press(state, &xf, &p, egui::PointerButton::Primary);
        state.session.as_ref().unwrap().selected_stamp.expect("placed")
    }

    /// The dense pixels of every stamp layer of the document, in layer order.
    fn stamp_pixels(state: &AppState) -> Vec<Vec<u8>> {
        let doc = state.doc().unwrap();
        doc.sets
            .iter()
            .flat_map(|set| set.layers.iter().map(move |l| (set, l)))
            .filter(|(_, l)| !l.is_raster())
            .map(|(set, l)| l.to_dense(set.width, set.height).data)
            .collect()
    }

    /// Waits for the output of the bake job and hands it over as `App::logic` does (poll, then `on_output`).
    fn deliver(state: &mut AppState, jobs: &mut Jobs) {
        let start = Instant::now();
        loop {
            for done in jobs.poll() {
                if let JobOutput::StampBake { generation, output } = done.output {
                    on_output_with(state, generation, output);
                    return;
                }
            }
            assert!(start.elapsed() < Duration::from_secs(60), "the bake job delivered nothing");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn has_toast(state: &AppState, part: &str) -> bool {
        state.toasts.visible_labels().iter().any(|l| l.contains(part))
    }

    #[test]
    fn a_placement_bakes_on_a_worker_and_lands_with_the_synchronous_pixels() {
        let mut sync = stamp_state(false);
        place(&mut sync);
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        pump_with(&mut sync, &mut jobs, &ctx);
        assert!(!sync.final_bakes_pending && !jobs.any_running(), "the synchronous mode starts no job");

        let mut state = stamp_state(true);
        let id = place(&mut state);
        let doc = state.doc().unwrap();
        assert!(doc.stamp_is_unbaked(id) && doc.has_unbaked_stamps());
        assert_eq!(state.await_first_bake, Some(id));
        assert_eq!(doc.stamp_layers(id).len(), 1, "the home layer is there at once");
        assert_eq!(state.tool, ToolKind::Transform, "the gizmo shows where the decal will appear");
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight() && state.final_bakes_pending);
        let busy = jobs.busy().unwrap();
        assert_eq!((busy.title.as_str(), busy.stage.as_str(), busy.modal), ("Finishing stamp", "", false));
        // A second pump while the bake runs starts nothing new.
        pump_with(&mut state, &mut jobs, &ctx);
        deliver(&mut state, &mut jobs);
        assert!(!state.final_bakes.in_flight() && !state.final_bakes_pending);
        assert!(!state.doc().unwrap().stamp_is_unbaked(id));
        assert_eq!(state.await_first_bake, None);
        assert!(!has_toast(&state, "covers no texels"));
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
        assert!(!stamp_pixels(&state)[0].iter().all(|&b| b == 0));

        // A Final edit: unbaked again, baked by the next pump.
        let doc = state.doc_mut().unwrap();
        let mut at = doc.stamp(id).unwrap().placement;
        if let Placement::Projected(p) = &mut at {
            p.half_size *= 0.8;
        }
        doc.set_stamp_placement(id, at, Commit::Final).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes_pending);
        deliver(&mut state, &mut jobs);
        assert!(!state.final_bakes_pending && !state.doc().unwrap().has_unbaked_stamps());
    }

    #[test]
    fn a_final_without_input_asks_for_the_frame_that_starts_its_bake() {
        // Check-and-fix: a Final made in a frame without input (the wheel gesture of the Transform tool ends on a timer)
        // must not leave the Preview on screen until the next mouse move.
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        // (A fresh `egui::Context` already has one repaint outstanding, so the decision is tested, not the context.)
        let mut state = stamp_state(true);
        assert!(!needs_frame_after_dispatch(&mut state), "nothing waits: no extra frame");
        let id = place(&mut state);
        assert!(needs_frame_after_dispatch(&mut state), "a stamp waits and no bake runs: the next frame starts it");
        assert!(state.final_bakes.marked.contains_key(&id), "the frame of the Final is the start of the log's T");
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!needs_frame_after_dispatch(&mut state), "the bake in flight asks for its frame when it is done");
        deliver(&mut state, &mut jobs);
        assert!(state.final_bakes.marked.is_empty(), "the mark ends with the bake");
        // A pending edit drops the marks; the Final marks the stamp again.
        let doc = state.doc_mut().unwrap();
        let mut at = doc.stamp(id).unwrap().placement;
        if let Placement::Projected(p) = &mut at {
            p.half_size *= 0.95;
        }
        doc.set_stamp_placement(id, at, Commit::Preview).unwrap();
        assert!(doc.has_pending_edit());
        state.final_bakes.marked.insert(id, Instant::now());
        after_dispatch(&mut state, &ctx);
        assert!(state.final_bakes.marked.is_empty());
    }

    #[test]
    fn a_result_for_another_document_is_dropped() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = stamp_state(true);
        place(&mut state);
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight());
        // New / Open: another session (as `install_document` sets it up).
        let mut next = stamp_state(true);
        let mut session = next.session.take().unwrap();
        session.scene_generation = 2;
        state.session = Some(session);
        reset_for_new_document(&mut state);
        // Settle does not wait for a bake of the old document.
        settle_with(&mut state, &mut jobs);
        assert!(state.final_bakes_pending, "the old bake still runs");
        let before = stamp_pixels(&state);
        deliver(&mut state, &mut jobs);
        assert!(stamp_pixels(&state) == before && state.doc().unwrap().stamps.is_empty());
        assert!(!state.final_bakes.in_flight() && !state.final_bakes_pending);
    }

    #[test]
    fn settle_waits_for_the_bake_in_flight_and_bakes_the_rest() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = stamp_state(true);
        let id = place(&mut state);
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight());
        // A second stamp waits behind the first one.
        let doc = state.doc_mut().unwrap();
        let copy = doc.duplicate_stamp(id).unwrap();
        assert!(doc.stamp_is_unbaked(copy));
        settle_with(&mut state, &mut jobs);
        let doc = state.doc().unwrap();
        assert!(!doc.has_unbaked_stamps() && !doc.stamp_is_unbaked(id) && !doc.stamp_is_unbaked(copy));
        assert!(!state.final_bakes.in_flight() && !state.final_bakes_pending && !jobs.any_running());
        // With nothing pending it is a no-op.
        let before = stamp_pixels(&state);
        settle_with(&mut state, &mut jobs);
        assert_eq!(stamp_pixels(&state), before);
        // Before a merge: a Final, then settle, then the merge reads the final pixels.
        let doc = state.doc_mut().unwrap();
        let at = doc.stamp(copy).unwrap().placement;
        let moved = match at {
            Placement::Projected(mut p) => {
                p.half_size *= 1.2;
                Placement::Projected(p)
            }
            other => other,
        };
        doc.set_stamp_placement(copy, moved, Commit::Final).unwrap();
        assert!(doc.stamp_is_unbaked(copy));
        settle_with(&mut state, &mut jobs);
        assert!(!state.doc().unwrap().has_unbaked_stamps());
    }

    #[test]
    fn an_empty_first_bake_warns_when_it_lands() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = stamp_state(true);
        let blob = state.tools.stamp.source.blob().unwrap();
        let doc = state.doc_mut().unwrap();
        let c = doc.mesh.bounds.center();
        let r = doc.mesh.bounds.diagonal() * 0.29;
        // A decal in front of the sphere that faces away from it: culled everywhere.
        let away = Projector::from_surface(c + Vec3::Z * r, -Vec3::Z, Vec3::Y, Vec2::splat(r * 0.2), r * 0.05);
        let id = doc.add_stamp("away", StampSource::Image { blob }, Placement::Projected(away), wte_core::TextureSetId(0)).unwrap();
        state.await_first_bake = Some(id);
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!has_toast(&state, "covers no texels"), "no warning before the bake lands");
        deliver(&mut state, &mut jobs);
        assert!(has_toast(&state, "The stamp was added as a layer but covers no texels"));
        assert_eq!(state.await_first_bake, None);
        assert_eq!(state.doc().unwrap().stamp_layers(id).len(), 1, "the empty home layer stays");
    }

    /// The placement of stamp `id` scaled by `f` (half size only).
    fn scaled_placement(state: &AppState, id: StampId, f: f32) -> Placement {
        match state.doc().unwrap().stamp(id).unwrap().placement {
            Placement::Projected(mut p) => {
                p.half_size *= f;
                Placement::Projected(p)
            }
            other => other,
        }
    }

    #[test]
    fn esc_on_a_new_drag_while_the_bake_runs_keeps_its_result() {
        // Check-and-fix (plan 1.5, "Undo / Redo / Esc on a pending edit"): Esc brings back the stamp object of the bake
        // in flight, so its result is used and no second bake starts.
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = stamp_state(true);
        let id = place(&mut state);
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight());
        let dragged = scaled_placement(&state, id, 1.3);
        let doc = state.doc_mut().unwrap();
        doc.set_stamp_placement(id, dragged, Commit::Preview).unwrap();
        assert!(!doc.has_unbaked_stamps(), "the stamp being edited does not wait");
        assert!(doc.cancel_pending_edit());
        assert!(doc.stamp_is_unbaked(id), "Esc brings back the waiting bake");
        deliver(&mut state, &mut jobs);
        assert!(!state.doc().unwrap().stamp_is_unbaked(id) && !state.final_bakes_pending, "the result was used");
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!state.final_bakes.in_flight() && !jobs.any_running(), "no second bake");
        let mut sync = stamp_state(false);
        place(&mut sync);
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
    }

    #[test]
    fn undo_then_redo_while_the_bake_runs_uses_its_result_and_a_final_after_undo_drops_it() {
        // Check-and-fix (plan 1.5): undo and redo never wait; a result that matches the now-current stamp object is
        // used, any other is dropped and the restored state is baked in the background.
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = stamp_state(true);
        let id = place(&mut state);
        pump_with(&mut state, &mut jobs, &ctx);
        let doc = state.doc_mut().unwrap();
        assert_eq!(doc.undo().as_deref(), Some("Add Stamp"));
        assert!(doc.stamp(id).is_none() && !doc.has_unbaked_stamps());
        assert_eq!(doc.redo().as_deref(), Some("Add Stamp"));
        assert!(doc.stamp_is_unbaked(id), "the redone stamp still waits for its bake");
        deliver(&mut state, &mut jobs);
        assert!(!state.doc().unwrap().stamp_is_unbaked(id) && !state.final_bakes_pending, "same object: used");
        let mut sync = stamp_state(false);
        place(&mut sync);
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));

        // A Final, its bake in flight, Undo: the result is stale and nothing is left to bake (the state before the
        // Final was baked).
        let bigger = scaled_placement(&state, id, 1.2);
        state.doc_mut().unwrap().set_stamp_placement(id, bigger, Commit::Final).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight());
        assert_eq!(state.doc_mut().unwrap().undo().as_deref(), Some("Edit Stamp"));
        assert!(!state.doc().unwrap().has_unbaked_stamps());
        deliver(&mut state, &mut jobs);
        assert!(!state.final_bakes_pending && !state.doc().unwrap().has_unbaked_stamps());
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync), "the stale result changed nothing");
    }

    #[test]
    fn a_result_during_a_stroke_is_held_until_it_ends() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = stamp_state(true);
        let id = place(&mut state);
        pump_with(&mut state, &mut jobs, &ctx);
        // Paint while the bake runs (the stamp layer is active: make the paint layer active first).
        let doc = state.doc_mut().unwrap();
        let paint = doc.sets[0].layers.iter().find(|l| l.is_raster() && !l.locked).map(|l| l.id).unwrap();
        doc.set_active_layer(wte_core::TextureSetId(0), Some(paint)).unwrap();
        doc.stroke_begin(&BrushParams::default()).unwrap();
        deliver(&mut state, &mut jobs);
        assert!(state.final_bakes_pending && state.doc().unwrap().stamp_is_unbaked(id), "held during the stroke");
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.doc().unwrap().stamp_is_unbaked(id), "still held");
        state.doc_mut().unwrap().stroke_end();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!state.final_bakes_pending && !state.doc().unwrap().stamp_is_unbaked(id));
        let home = state.doc().unwrap().stamp_layers(id)[0].1;
        let layer = state.doc().unwrap().layer(home).unwrap();
        assert!(matches!(&layer.kind, LayerKind::Stamp { cache, .. } if !cache.is_empty()));
    }

    /// Check:B (ROUND6_PLAN 3.3, "Deferred Final bakes, settle before save / export / merge down / rasterize" and
    /// "Undo during a bake"): a wrapped stamp's Final bakes on the worker with the synchronous pixels and leaves its
    /// parameterization in the cache; `settle` (the step before save, export, merge down and rasterize) waits for a
    /// wrapped bake in flight; a toggle to Project undone while its bake runs leaves the wrapped pixels.
    #[test]
    fn a_wrapped_stamp_bakes_on_the_worker_and_settles_like_a_projected_one() {
        use wte_core::SurfaceMode;
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = stamp_state(false);
        sync.tools.stamp.mode = SurfaceMode::Wrap;
        let sync_id = place(&mut sync);
        let mut state = stamp_state(true);
        state.tools.stamp.mode = SurfaceMode::Wrap;
        let id = place(&mut state);
        let wrapped = |state: &AppState, id: StampId| {
            matches!(state.doc().unwrap().stamp(id).unwrap().placement, Placement::Projected(p) if p.mode == SurfaceMode::Wrap)
        };
        assert!(wrapped(&state, id) && wrapped(&sync, sync_id));
        assert!(state.doc().unwrap().stamp_is_unbaked(id) && state.doc().unwrap().stamp_wrap_stats(id).is_none());
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight());
        deliver(&mut state, &mut jobs);
        assert!(!state.final_bakes_pending && !state.doc().unwrap().stamp_is_unbaked(id));
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
        assert!(!stamp_pixels(&state)[0].iter().all(|&b| b == 0));
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some(), "the applied bake cached its parameterization");

        // A Final scale, its bake in flight, then settle: the pixels of the same edit made synchronously.
        let bigger = scaled_placement(&state, id, 1.3);
        state.doc_mut().unwrap().set_stamp_placement(id, bigger, Commit::Final).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight());
        settle_with(&mut state, &mut jobs);
        assert!(!state.final_bakes_pending && !state.doc().unwrap().has_unbaked_stamps() && !jobs.any_running());
        let sync_bigger = scaled_placement(&sync, sync_id, 1.3);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_bigger, Commit::Final).unwrap();
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
        let settled = stamp_pixels(&state);

        // Toggle to Project, its bake in flight, Undo: the result is stale, the wrapped pixels stay.
        let Placement::Projected(p) = state.doc().unwrap().stamp(id).unwrap().placement else { panic!("3D") };
        let projected = Placement::Projected(Projector { mode: SurfaceMode::Project, ..p });
        state.doc_mut().unwrap().set_stamp_placement(id, projected, Commit::Final).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight());
        assert_eq!(state.doc_mut().unwrap().undo().as_deref(), Some("Edit Stamp"));
        deliver(&mut state, &mut jobs);
        assert!(!state.final_bakes_pending && !state.doc().unwrap().has_unbaked_stamps());
        assert!(wrapped(&state, id));
        assert_eq!(stamp_pixels(&state), settled, "the stale Project result changed nothing");
    }

    // ----- round 8 (R8-2, ROUND8_PLAN 4.2): wrapped Previews as latest-wins jobs -----

    /// `stamp_state` with the Stamp tool in Wrap; `deferred`: the app's asynchronous mode (Finals and wrapped
    /// Previews on workers, as `install_document` sets it up).
    fn wrap_state(deferred: bool) -> AppState {
        let mut state = stamp_state(deferred);
        state.tools.stamp.mode = wte_core::SurfaceMode::Wrap;
        state.doc_mut().unwrap().set_defer_wrap_previews(deferred);
        state
    }

    /// The placement of stamp `id` turned by `radians` (a new spine: the parameterization of the old one never serves).
    fn turned(state: &AppState, id: StampId, radians: f32) -> Placement {
        match state.doc().unwrap().stamp(id).unwrap().placement {
            Placement::Projected(p) => Placement::Projected(p.rotated(radians)),
            other => other,
        }
    }

    /// What `deliver_next` handed over.
    #[derive(Debug, PartialEq)]
    enum Delivered {
        Final,
        Wrap(StampJobKind),
    }

    /// Waits for the next job output and hands it over as `App::logic` does.
    fn deliver_next(state: &mut AppState, jobs: &mut Jobs) -> Delivered {
        let start = Instant::now();
        loop {
            if let Some(done) = jobs.poll().into_iter().next() {
                return match done.output {
                    JobOutput::StampBake { generation, output } => {
                        on_output_with(state, generation, output);
                        Delivered::Final
                    }
                    JobOutput::WrapPreview { generation, output } | JobOutput::WrapParam { generation, output } => {
                        let kind = output.kind();
                        on_wrap_output_with(state, generation, output);
                        Delivered::Wrap(kind)
                    }
                    _ => panic!("another job's output"),
                };
            }
            assert!(start.elapsed() < Duration::from_secs(120), "no job delivered");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A wrapped stamp placed and its first Final landed (asynchronous mode).
    fn placed_wrapped(state: &mut AppState, jobs: &mut Jobs, ctx: &egui::Context) -> StampId {
        let id = place(state);
        pump_with(state, jobs, ctx);
        assert_eq!(deliver_next(state, jobs), Delivered::Final);
        assert!(!state.final_bakes_pending && !state.wrap_jobs_pending);
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some());
        id
    }

    #[test]
    fn a_drag_of_a_wrapped_stamp_bakes_its_previews_on_a_worker_and_the_latest_wins() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        let sync_id = place(&mut sync);
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
        let before = stamp_pixels(&state);
        // Ten Preview moves of one drag, no output handed over in between: one job at a time, nothing on the UI thread
        // (check:B: the core's counter of parameterizations made on this thread stays put).
        for k in 0..10 {
            let built = wte_core::param_counter::get();
            let at = turned(&state, id, 0.03);
            state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
            let doc = state.doc().unwrap();
            assert!(doc.stamp_wrap_stats(id).is_none(), "move {k}: no parameterization of the moved placement on the UI thread");
            assert_eq!(stamp_pixels(&state), before, "move {k}: nothing baked on the UI thread");
            pump_with(&mut state, &mut jobs, &ctx);
            assert_eq!(wte_core::param_counter::get(), built, "move {k}: a parameterization was built on the UI thread");
            assert!(state.wrap_jobs_pending && jobs.running_count() == 1, "move {k}: one job in flight");
            assert_eq!(state.wrap_jobs.in_flight(), Some((id, StampJobKind::Preview)));
            assert!(jobs.busy().is_none(), "the wrap job has no status-bar title");
            let sync_at = turned(&sync, sync_id, 0.03);
            sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_at, Commit::Preview).unwrap();
        }
        assert_eq!(state.wrap_jobs.counters.previews_started, 1, "latest wins: one job while the first runs");
        // The first job (made for the first move) lands: the last good picture, and the stamp is due again.
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!((state.wrap_jobs.counters.previews_applied, state.wrap_jobs.counters.previews_due_again), (1, 1));
        assert_ne!(stamp_pixels(&state), before, "the first job's picture is shown");
        assert!(state.doc().unwrap().has_stale_wrap_previews() && state.wrap_jobs_pending);
        // The next pump starts the job of the last placement; it lands with the synchronous Preview's pixels.
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.counters.previews_started, 2);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!((state.wrap_jobs.counters.previews_applied, state.wrap_jobs.counters.previews_due_again), (2, 1));
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!jobs.any_running() && !state.wrap_jobs_pending, "nothing left to do");
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync), "the last Preview = the synchronous Preview of the last placement");
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some(), "the job brought the parameterization");
        // The release: the Final (on its worker) equals the synchronous Final.
        state.doc_mut().unwrap().flush_pending_edit();
        sync.doc_mut().unwrap().flush_pending_edit();
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Final);
        assert!(!state.final_bakes_pending && !state.wrap_jobs_pending);
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
        assert_eq!(state.wrap_jobs.counters.ui_params, 0);
    }

    #[test]
    fn esc_or_undo_while_a_wrap_preview_job_runs_keeps_the_old_pixels() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let before = stamp_pixels(&state);
        let placed = state.doc().unwrap().stamp(id).unwrap().placement;
        // Esc.
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        assert!(state.doc_mut().unwrap().cancel_pending_edit());
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!(state.wrap_jobs.counters.previews_stale, 1);
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!jobs.any_running() && !state.wrap_jobs_pending && !state.final_bakes_pending);
        assert_eq!(stamp_pixels(&state), before, "Esc: the old pixels");
        // Undo (it flushes the Preview edit, then undoes it): the job's output is stale, the old pixels come back.
        let at = turned(&state, id, -0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        assert_eq!(state.doc_mut().unwrap().undo().as_deref(), Some("Edit Stamp"));
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!(state.wrap_jobs.counters.previews_stale, 2);
        for _ in 0..10 {
            pump_with(&mut state, &mut jobs, &ctx);
            if !jobs.any_running() {
                break;
            }
            deliver_next(&mut state, &mut jobs);
        }
        assert!(!jobs.any_running() && !state.final_bakes_pending && !state.wrap_jobs_pending);
        assert_eq!(state.doc().unwrap().stamp(id).unwrap().placement, placed);
        assert_eq!(stamp_pixels(&state), before, "Undo: the old pixels");
        assert_eq!(state.wrap_jobs.counters.previews_applied, 0);
    }

    #[test]
    fn a_wrap_preview_of_another_document_is_dropped() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        // New / Open: another session (as `install_document` sets it up), with a wrapped stamp of the same id.
        let mut next = wrap_state(true);
        let other = placed_wrapped(&mut next, &mut Jobs::new(), &ctx);
        assert_eq!(other, id);
        let mut session = next.session.take().unwrap();
        session.scene_generation = 2;
        state.session = Some(session);
        reset_for_new_document(&mut state);
        let before = stamp_pixels(&state);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!(stamp_pixels(&state), before, "the old document's Preview changed nothing");
        assert_eq!((state.wrap_jobs.counters.previews_stale, state.wrap_jobs.counters.previews_applied), (1, 0));
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!jobs.any_running() && !state.wrap_jobs_pending);
    }

    #[test]
    fn a_selected_wrapped_stamp_without_parameterization_gets_it_from_a_param_only_job() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        // A Final turn, its bake, then Undo: the caches come back, the parameterization does not.
        let at = turned(&state, id, 0.3);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Final).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Final);
        assert_eq!(state.doc_mut().unwrap().undo().as_deref(), Some("Edit Stamp"));
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_none() && !state.doc().unwrap().stamp_is_unbaked(id));
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        let before = stamp_pixels(&state);
        // The Transform tool asks for a param-only job instead of making it on the UI thread; `after_tools` starts it.
        let mut tools = ToolController::new();
        assert!(!tools.ensure_wrap_param(&mut state), "nothing made on the UI thread");
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.in_flight(), Some((id, StampJobKind::ParamOnly)));
        assert!(state.wrap_jobs_pending && jobs.busy().is_none());
        // While it runs the tool asks again; nothing more starts.
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(jobs.running_count(), 1);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::ParamOnly));
        let doc = state.doc().unwrap();
        assert!(doc.stamp_wrap_stats(id).is_some(), "the stats line has its numbers");
        assert!(!doc.stamp_wrap_outline(id, 8).is_empty(), "the outline can be drawn");
        assert_eq!(stamp_pixels(&state), before, "no pixel changed");
        assert_eq!((state.wrap_jobs.counters.params_started, state.wrap_jobs.counters.params_applied), (1, 1));
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert!(!jobs.any_running() && !state.wrap_jobs_pending, "nothing more to do");
        assert_eq!(state.wrap_jobs.counters.ui_params, 0);
    }

    /// Objects-tabs check of WP2: a param-only job made before a visibility change lands without being cached
    /// (`Document::apply_wrap_param`); that proves nothing about the surface, so the stamp is asked for again rather
    /// than marked "no surface". A failure is remembered per visibility: showing the objects again retries.
    #[test]
    fn a_param_only_job_overtaken_by_a_visibility_change_is_asked_for_again() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.3);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Final).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Final);
        assert_eq!(state.doc_mut().unwrap().undo().as_deref(), Some("Edit Stamp"));
        state.session.as_mut().unwrap().selected_stamp = Some(id);
        let mut tools = ToolController::new();
        let ask = |state: &mut AppState, tools: &mut ToolController, jobs: &mut Jobs| {
            assert!(!tools.ensure_wrap_param(state));
            after_tools(state, jobs, &ctx);
        };
        ask(&mut state, &mut tools, &mut jobs);
        assert_eq!(state.wrap_jobs.in_flight(), Some((id, StampJobKind::ParamOnly)));
        // The only object is hidden while the job runs: its parameterization is not cached and proves nothing.
        assert!(state.doc_mut().unwrap().set_hidden_objects(&[true]).unwrap());
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::ParamOnly));
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_none());
        assert_eq!(state.wrap_jobs.param_failed, None, "not marked as having no surface");
        // Asked for again; with everything hidden there is no surface: marked for this visibility, not asked again.
        ask(&mut state, &mut tools, &mut jobs);
        assert_eq!(state.wrap_jobs.in_flight(), Some((id, StampJobKind::ParamOnly)));
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::ParamOnly));
        assert!(state.wrap_jobs.param_failed.is_some());
        ask(&mut state, &mut tools, &mut jobs);
        assert!(!jobs.any_running());
        // Shown again: asked for again, and it lands.
        assert!(state.doc_mut().unwrap().set_hidden_objects(&[false]).unwrap());
        ask(&mut state, &mut tools, &mut jobs);
        assert_eq!(state.wrap_jobs.in_flight(), Some((id, StampJobKind::ParamOnly)));
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::ParamOnly));
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some());
    }

    /// Objects-tabs check of WP2, synchronous mode: "no surface to wrap onto" is remembered per visibility, so a stamp
    /// whose surface was hidden gets its parameterization again once the surface is shown.
    #[test]
    fn the_synchronous_wrap_failure_is_retried_after_a_visibility_change() {
        let mut state = wrap_state(false);
        let id = place(&mut state);
        let mut tools = ToolController::new();
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some(), "placed with its parameterization");
        assert!(state.doc_mut().unwrap().set_hidden_objects(&[true]).unwrap());
        assert!(!tools.ensure_wrap_param(&mut state), "nothing to wrap onto");
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_none());
        let tries = state.wrap_jobs.counters.ui_params;
        assert!(!tools.ensure_wrap_param(&mut state));
        assert_eq!(state.wrap_jobs.counters.ui_params, tries, "not tried again for the same visibility");
        assert!(state.doc_mut().unwrap().set_hidden_objects(&[false]).unwrap());
        assert!(tools.ensure_wrap_param(&mut state), "tried again once the surface is shown");
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some());
    }

    #[test]
    fn a_final_waits_for_its_stamps_wrap_preview_job() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        let sync_id = place(&mut sync);
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.25);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        // The release while the Preview job runs: the Final waits for it.
        state.doc_mut().unwrap().flush_pending_edit();
        assert!(state.doc().unwrap().stamp_is_unbaked(id));
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!state.final_bakes.in_flight() && jobs.running_count() == 1, "the Final waits");
        // check:B: and asks for no frame meanwhile (the job's landing does).
        assert!(!needs_frame_after_dispatch(&mut state), "a frame per frame while the Final waits for the Preview job");
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!(state.wrap_jobs.counters.previews_stale, 1, "after the release the Preview is stale");
        // check:B (core request 1): its parameterization serves the released placement and is cached (no pixel change).
        let from_job = state.doc().unwrap().stamp_wrap_stats(id).expect("the stale Preview's parameterization is cached");
        let built = wte_core::param_counter::get();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight(), "then the Final starts");
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Final);
        assert_eq!(wte_core::param_counter::get(), built);
        // The Final reused it: the same statistics, build time included (a second build would have its own time).
        assert_eq!(state.doc().unwrap().stamp_wrap_stats(id), Some(from_job), "the Final built the parameterization again");
        let sync_at = turned(&sync, sync_id, 0.25);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_at, Commit::Final).unwrap();
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
    }

    /// Round 9 (package B, T8): a Final released while the Preview job runs that was made for an earlier placement of
    /// the drag starts at once (the same pump); that job's output lands stale and the Final equals the synchronous one.
    #[test]
    fn a_final_does_not_wait_for_a_wrap_preview_job_of_another_placement() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        let sync_id = place(&mut sync);
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.25);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.in_flight(), Some((id, StampJobKind::Preview)));
        // The drag goes on, then the release: the job in flight was made for the earlier placement.
        let at2 = turned(&state, id, 0.25);
        state.doc_mut().unwrap().set_stamp_placement(id, at2, Commit::Preview).unwrap();
        state.doc_mut().unwrap().flush_pending_edit();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight(), "the Final starts in the frame of the release");
        assert_eq!(jobs.running_count(), 2, "beside the Preview job");
        // Both land, in either order (both may finish between two polls).
        let (mut finals, mut previews) = (0, 0);
        let start = Instant::now();
        while finals + previews < 2 {
            for done in jobs.poll() {
                match done.output {
                    JobOutput::StampBake { generation, output } => {
                        on_output_with(&mut state, generation, output);
                        finals += 1;
                    }
                    JobOutput::WrapPreview { generation, output } => {
                        on_wrap_output_with(&mut state, generation, output);
                        previews += 1;
                    }
                    _ => panic!("another job's output"),
                }
            }
            assert!(start.elapsed() < Duration::from_secs(120), "the jobs delivered nothing");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!((finals, previews), (1, 1));
        assert_eq!(state.wrap_jobs.counters.previews_stale, 1, "the Preview of the earlier placement is stale");
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!jobs.any_running() && !state.final_bakes_pending && !state.wrap_jobs_pending);
        for _ in 0..2 {
            let sync_at = turned(&sync, sync_id, 0.25);
            sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_at, Commit::Final).unwrap();
        }
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
    }

    // ----- check:B (round 9): the warm-up thread against the wrap jobs, a new document and a close -----

    /// Waits for the warm-up thread (if one runs) to end and joins it.
    fn join_warm_up(state: &mut AppState) {
        if let Some(handle) = state.wrap_jobs.warm_thread.take() {
            handle.join().expect("the warm-up thread panicked");
        }
    }

    /// check:B (round 9): the warm-up started by the selection and the first wrap Final in the same frame share one
    /// graph (built once, by whichever comes first; the other waits in the cell); the Final equals the synchronous one;
    /// the shared texels are known afterwards and nothing more is warmed up.
    #[test]
    fn the_warm_up_and_the_first_wrap_final_build_one_graph() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        place(&mut sync);
        let mut state = wrap_state(true);
        let id = place(&mut state);
        let cell = state.doc_mut().unwrap().wrap_mesh_cell();
        assert!(!cell.is_built() && cell.uv_built().is_none(), "nothing built before the first wrap");
        let mut tools = ToolController::new();
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.counters.warmups_started, 1);
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.final_bakes.in_flight(), "the Final starts while the warm-up runs");
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Final);
        join_warm_up(&mut state);
        assert_eq!(cell.builds(), 1, "one graph for the warm-up and the job");
        assert!(Arc::ptr_eq(&cell, &state.doc_mut().unwrap().wrap_mesh_cell()));
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
        assert_eq!(state.doc().unwrap().stamp_shared_texels(id), Some(0.0), "the sphere reuses no texels");
        for _ in 0..3 {
            assert!(!tools.ensure_wrap_param(&mut state));
            after_tools(&mut state, &mut jobs, &ctx);
        }
        assert!(state.wrap_jobs.warm_thread.is_none() && state.wrap_jobs.counters.warmups_started == 1, "nothing left to warm up");
        drain(&mut state, &mut jobs, &ctx, 4);
    }

    /// check:B (round 9, plan T8): choosing Wrap in the Stamp tool warms up the graph before any stamp is placed; in
    /// Project it warms up nothing until a 3D stamp is selected.
    #[test]
    fn choosing_wrap_in_the_stamp_tool_warms_up_the_graph() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut tools = ToolController::new();
        let mut state = stamp_state(true);
        state.tool = ToolKind::Stamp;
        assert!(state.session.as_ref().unwrap().selected_stamp.is_none());
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.counters.warmups_started, 0, "Project: nothing to warm up");
        state.tools.stamp.mode = wte_core::SurfaceMode::Wrap;
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.counters.warmups_started, 1);
        join_warm_up(&mut state);
        let cell = state.doc_mut().unwrap().wrap_mesh_cell();
        assert!(cell.is_built() && cell.uv_built().is_some() && cell.builds() == 1);
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.counters.warmups_started, 1, "built once");
        assert!(!jobs.any_running());
    }

    /// check:B (round 9): a document opened while the previous one's warm-up runs gets its own warm-up (once the old
    /// thread ended; the Transform tool asks every frame) and its own graph; a document closed during a warm-up leaves
    /// nothing behind (the thread ends on its own and is joined; no new one starts without a document).
    #[test]
    fn a_new_document_or_a_close_during_the_warm_up_is_safe() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let old_cell = state.doc_mut().unwrap().wrap_mesh_cell();
        assert!(old_cell.is_built() && old_cell.uv_built().is_none(), "the Final built the graph; the shared texels wait for the warm-up");
        let mut tools = ToolController::new();
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.counters.warmups_started, 1);
        // New / Open while it may still run (as `install_document` sets it up).
        let mut next = wrap_state(true);
        let other = placed_wrapped(&mut next, &mut Jobs::new(), &ctx);
        assert_eq!(other, id);
        let mut session = next.session.take().unwrap();
        session.scene_generation = 2;
        state.session = Some(session);
        reset_for_new_document(&mut state);
        let start = Instant::now();
        while state.wrap_jobs.counters.warmups_started < 2 {
            assert!(!tools.ensure_wrap_param(&mut state));
            after_tools(&mut state, &mut jobs, &ctx);
            assert!(start.elapsed() < Duration::from_secs(60), "the new document's warm-up never started");
            std::thread::sleep(Duration::from_millis(1));
        }
        join_warm_up(&mut state);
        let new_cell = state.doc_mut().unwrap().wrap_mesh_cell();
        assert!(!Arc::ptr_eq(&new_cell, &old_cell), "the new document has its own cell");
        assert!(old_cell.uv_built().is_some() && new_cell.uv_built().is_some() && new_cell.builds() == 1);
        assert_eq!(state.doc().unwrap().stamp_shared_texels(other), Some(0.0));
        // A third document whose warm-up builds the graph, closed while it runs.
        let mut third = wrap_state(true);
        place(&mut third);
        let mut session = third.session.take().unwrap();
        session.scene_generation = 3;
        state.session = Some(session);
        reset_for_new_document(&mut state);
        let third_cell = state.doc_mut().unwrap().wrap_mesh_cell();
        assert!(!tools.ensure_wrap_param(&mut state));
        after_tools(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.counters.warmups_started, 3);
        state.session = None;
        reset_for_new_document(&mut state);
        for _ in 0..3 {
            assert!(!tools.ensure_wrap_param(&mut state));
            after_tools(&mut state, &mut jobs, &ctx);
        }
        join_warm_up(&mut state);
        after_tools(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.warm_thread.is_none() && state.wrap_jobs.counters.warmups_started == 3, "no warm-up without a document");
        assert!(third_cell.is_built() && third_cell.builds() == 1, "the closed document's warm-up finished on its own");
        assert!(!jobs.any_running());
    }

    // ----- check:B (ROUND8_PLAN 6 row 4): the races of the wrap Preview jobs -----

    /// Hands over job outputs and pumps until nothing runs and nothing waits (at most `n` outputs).
    fn drain(state: &mut AppState, jobs: &mut Jobs, ctx: &egui::Context, n: usize) {
        for _ in 0..n {
            pump_with(state, jobs, ctx);
            if !jobs.any_running() {
                break;
            }
            deliver_next(state, jobs);
        }
        pump_with(state, jobs, ctx);
        assert!(!jobs.any_running() && !state.final_bakes_pending && !state.wrap_jobs_pending, "work left after the drain");
    }

    #[test]
    fn esc_after_a_landed_wrap_preview_restores_the_old_pixels() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let before = stamp_pixels(&state);
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!(state.wrap_jobs.counters.previews_applied, 1);
        assert_ne!(stamp_pixels(&state), before, "the Preview landed");
        assert!(state.doc_mut().unwrap().cancel_pending_edit());
        assert_eq!(stamp_pixels(&state), before, "Esc: the pixels from before the drag");
        drain(&mut state, &mut jobs, &ctx, 4);
        assert_eq!(stamp_pixels(&state), before);
    }

    #[test]
    fn a_stale_preview_landing_after_a_newer_edit_is_dropped_and_the_newer_edit_wins() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        let sync_id = place(&mut sync);
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        // Drag 1: its Preview job runs; the release; drag 2 starts before the job lands.
        let first = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, first, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        state.doc_mut().unwrap().flush_pending_edit();
        let second = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, second, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!state.final_bakes.in_flight(), "the stamp is edited again: no Final of drag 1");
        let shown = stamp_pixels(&state);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!((state.wrap_jobs.counters.previews_stale, state.wrap_jobs.counters.previews_applied), (1, 0));
        assert_eq!(stamp_pixels(&state), shown, "the older edit's Preview changed nothing");
        // Drag 2's own job: the synchronous Preview of the same edits.
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(state.wrap_jobs.in_flight(), Some((id, StampJobKind::Preview)));
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!(state.wrap_jobs.counters.previews_applied, 1);
        let sync_first = turned(&sync, sync_id, 0.2);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_first, Commit::Final).unwrap();
        let sync_second = turned(&sync, sync_id, 0.2);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_second, Commit::Preview).unwrap();
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync), "drag 2's Preview");
        // Its release: the Final equals the synchronous one; undo goes back to drag 1's result.
        state.doc_mut().unwrap().flush_pending_edit();
        sync.doc_mut().unwrap().flush_pending_edit();
        drain(&mut state, &mut jobs, &ctx, 4);
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync), "drag 2's Final");
        assert_eq!(state.doc_mut().unwrap().undo().as_deref(), Some("Edit Stamp"));
        assert_eq!(sync.doc_mut().unwrap().undo().as_deref(), Some("Edit Stamp"));
        drain(&mut state, &mut jobs, &ctx, 4);
        assert_eq!(state.doc().unwrap().stamp(id).unwrap().placement, first);
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync), "undo: drag 1's Final");
    }

    #[test]
    fn a_stamp_deleted_while_its_preview_job_runs_is_left_alone_and_undo_brings_it_back() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        let sync_id = place(&mut sync);
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        state.doc_mut().unwrap().remove_stamp(id).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!state.final_bakes.in_flight() && !state.final_bakes_pending, "nothing waits for a removed stamp");
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!((state.wrap_jobs.counters.previews_stale, state.wrap_jobs.counters.previews_applied), (1, 0));
        assert!(state.doc().unwrap().stamp(id).is_none() && stamp_pixels(&state).is_empty());
        drain(&mut state, &mut jobs, &ctx, 2);
        // Undo of the delete: the stamp comes back waiting for the Final of its drag, which then lands.
        assert_eq!(state.doc_mut().unwrap().undo().as_deref(), Some("Delete Stamp"));
        assert!(state.doc().unwrap().stamp_is_unbaked(id));
        drain(&mut state, &mut jobs, &ctx, 4);
        let sync_at = turned(&sync, sync_id, 0.2);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_at, Commit::Final).unwrap();
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
    }

    #[test]
    fn a_stroke_during_a_wrap_preview_job_holds_its_output_until_it_ends() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        let sync_id = place(&mut sync);
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        let before = stamp_pixels(&state);
        // A stroke starts (it ends the pending edit: the stamp waits for its Final, which cannot run during a stroke).
        let doc = state.doc_mut().unwrap();
        let paint = doc.sets[0].layers.iter().find(|l| l.is_raster() && !l.locked).map(|l| l.id).unwrap();
        doc.set_active_layer(wte_core::TextureSetId(0), Some(paint)).unwrap();
        doc.stroke_begin(&BrushParams::default()).unwrap();
        assert!(state.doc().unwrap().stamp_is_unbaked(id));
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert!(state.wrap_jobs.held.is_some() && state.wrap_jobs_pending, "held during the stroke");
        assert_eq!(state.wrap_jobs.counters, WrapJobCounters { previews_started: 1, ..WrapJobCounters::default() });
        for _ in 0..3 {
            pump_with(&mut state, &mut jobs, &ctx);
            assert!(!jobs.any_running() && state.wrap_jobs.held.is_some(), "nothing starts and nothing lands during a stroke");
        }
        assert_eq!(stamp_pixels(&state), before);
        state.doc_mut().unwrap().stroke_end();
        // The stroke ends: the held output is dropped (its edit ended) and the Final starts with its parameterization.
        let built = wte_core::param_counter::get();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.held.is_none() && state.final_bakes.in_flight());
        assert_eq!(state.wrap_jobs.counters.previews_stale, 1);
        let from_job = state.doc().unwrap().stamp_wrap_stats(id).expect("the held Preview's parameterization is cached");
        drain(&mut state, &mut jobs, &ctx, 2);
        assert_eq!(wte_core::param_counter::get(), built);
        assert_eq!(state.doc().unwrap().stamp_wrap_stats(id), Some(from_job), "the Final reused it");
        let sync_at = turned(&sync, sync_id, 0.2);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_at, Commit::Final).unwrap();
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
    }

    #[test]
    fn save_during_a_wrap_preview_job_settles_without_waiting_for_it() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut sync = wrap_state(false);
        let sync_id = place(&mut sync);
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        // Save: the flush, then settle (the snapshot follows): the Final is baked here, the Preview job is not awaited.
        state.doc_mut().unwrap().flush_pending_edit();
        settle_with(&mut state, &mut jobs);
        assert!(!state.doc().unwrap().has_unbaked_stamps() && !state.final_bakes_pending);
        assert!(jobs.any_running() && state.wrap_jobs.in_flight().is_some(), "the Preview job was not awaited");
        let sync_at = turned(&sync, sync_id, 0.2);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, sync_at, Commit::Final).unwrap();
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync), "the saved pixels are the Final's");
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!(state.wrap_jobs.counters.previews_stale, 1);
        drain(&mut state, &mut jobs, &ctx, 2);
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync), "the late Preview changed nothing");
    }

    #[test]
    fn a_wrap_preview_landing_after_a_turn_to_project_in_the_same_edit_is_dropped() {
        // check:B: a Preview edit of the open pending edit turns the stamp to Project while its wrap Preview job runs:
        // the projected Preview is baked on the calling thread at once; the late wrapped picture must not replace it.
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(true);
        let id = placed_wrapped(&mut state, &mut jobs, &ctx);
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(state.wrap_jobs.in_flight().is_some());
        let Placement::Projected(p) = at else { panic!("3D") };
        let projected = Placement::Projected(Projector { mode: wte_core::SurfaceMode::Project, ..p });
        state.doc_mut().unwrap().set_stamp_placement(id, projected, Commit::Preview).unwrap();
        let shown = stamp_pixels(&state);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Wrap(StampJobKind::Preview));
        assert_eq!((state.wrap_jobs.counters.previews_stale, state.wrap_jobs.counters.previews_applied), (1, 0));
        assert_eq!(stamp_pixels(&state), shown, "the projected Preview stays");
        state.doc_mut().unwrap().flush_pending_edit();
        drain(&mut state, &mut jobs, &ctx, 2);
        let mut sync = stamp_state(false);
        let sync_id = place(&mut sync);
        sync.doc_mut().unwrap().set_stamp_placement(sync_id, projected, Commit::Final).unwrap();
        assert_eq!(stamp_pixels(&state), stamp_pixels(&sync));
    }

    #[test]
    fn the_synchronous_mode_keeps_wrapped_previews_on_the_ui_thread() {
        // Found by the scenario (`WTE_SYNC_FINAL_BAKES=1`): with the views' throttle on (`set_defer_preview_bakes`, as
        // `install_document` sets it) a wrapped stamp's Preview waits in the deferred set; `pump` must leave it to the
        // throttled bake on the UI thread when the document does not defer wrapped Previews.
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = wrap_state(false);
        let id = place(&mut state);
        let before = stamp_pixels(&state);
        let doc = state.doc_mut().unwrap();
        doc.set_defer_preview_bakes(true);
        let at = turned(&state, id, 0.2);
        state.doc_mut().unwrap().set_stamp_placement(id, at, Commit::Preview).unwrap();
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!jobs.any_running() && !state.wrap_jobs_pending && state.wrap_jobs.counters == WrapJobCounters::default());
        let doc = state.doc_mut().unwrap();
        assert!(doc.has_stale_previews() && !doc.has_stale_wrap_previews(), "waits for the views' throttled bake");
        doc.bake_stale_previews();
        assert_ne!(stamp_pixels(&state), before);
        assert!(state.doc().unwrap().stamp_wrap_stats(id).is_some());
    }

    #[test]
    fn projected_stamps_keep_their_previews_on_the_ui_thread() {
        let ctx = egui::Context::default();
        let mut jobs = Jobs::new();
        let mut state = stamp_state(true);
        state.doc_mut().unwrap().set_defer_wrap_previews(true);
        let id = place(&mut state);
        pump_with(&mut state, &mut jobs, &ctx);
        assert_eq!(deliver_next(&mut state, &mut jobs), Delivered::Final);
        let before = stamp_pixels(&state);
        let bigger = scaled_placement(&state, id, 1.3);
        state.doc_mut().unwrap().set_stamp_placement(id, bigger, Commit::Preview).unwrap();
        let doc = state.doc().unwrap();
        assert!(!doc.has_stale_wrap_previews() && !doc.has_stale_previews());
        assert_ne!(stamp_pixels(&state), before, "baked on the calling thread at once");
        pump_with(&mut state, &mut jobs, &ctx);
        assert!(!jobs.any_running() && !state.wrap_jobs_pending);
        assert_eq!(state.wrap_jobs.counters, WrapJobCounters::default());
    }
}
