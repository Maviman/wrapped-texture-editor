// Owner: round 5, package A (R5-1). Final stamp bakes off the UI thread (ARCHITECTURE.md 8.9, 5.20 "Deferred Final
// bakes"): the document marks a stamp unbaked when an edit ends; `pump` takes its bake job (`next_final_bake`) and runs
// it on a worker, one at a time; `on_output` hands the result back (`apply_final_bake`); `settle` waits for the bake in
// flight and bakes the rest on the UI thread before the commands that keep stamp pixels (save, export, the export
// dialog, merge down, rasterize).

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui;
use wte_core::{FinalBake, StampBakeOutput, StampId};

use crate::app::WteApp;
use crate::jobs::{JobOutput, Jobs};
use crate::state::AppState;

/// Status-bar titles of the bake job (non-modal; the status bar shows the title).
const TITLE_STAMP: &str = "Finishing stamp";
const TITLE_TEXT: &str = "Finishing text";

/// The bake in flight and an output held while a stroke is open. Lives in `AppState::final_bakes`.
#[derive(Default)]
pub struct FinalBakes {
    in_flight: Option<InFlight>,
    /// An output that arrived while a stroke was open, with its scene generation; applied when the stroke ends.
    held: Option<(u64, Box<StampBakeOutput>)>,
    /// When each waiting stamp was first seen unbaked at the end of a frame's dispatch (`after_dispatch`): the frame of
    /// its Final. The "applied after T ms" log line measures T from here (else from the start of the job).
    marked: HashMap<StampId, Instant>,
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
    if state.final_bakes.in_flight.is_none()
        && state.final_bakes.held.is_none()
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
    bakes.in_flight.is_none() && waiting && !doc.stroke_active()
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
/// thread (`Document::settle_final_bakes`). The one place the UI thread blocks on a worker (4.6). Called before the
/// snapshot of save and export, by the export dialog, and before merge down and rasterize. Stamps of an open pending
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
fn update_pending(state: &mut AppState) {
    let waiting = state.doc().is_some_and(|d| d.has_unbaked_stamps());
    state.final_bakes_pending = state.final_bakes.in_flight.is_some() || state.final_bakes.held.is_some() || waiting;
}

/// Drops what belongs to the previous document when a new one is installed (`io_flows::install_document`): a held
/// output and the awaited first bake. A bake in flight finishes on its own; its result is dropped by its generation.
pub(crate) fn reset_for_new_document(state: &mut AppState) {
    state.final_bakes.held = None;
    state.final_bakes.marked.clear();
    state.await_first_bake = None;
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
        state.tools.stamp.source = Some(blob);
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
        let blob = state.tools.stamp.source.unwrap();
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
}
