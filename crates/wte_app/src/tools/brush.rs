//! Brush and eraser (ARCHITECTURE.md 8.6): strokes on the model in the 3D view and on the flat texture in the 2D view.
//!
//! Round 4 (FEATURE_BRUSH 4.3, 5.10, 5.11, 1.5, 6.5, 6.6):
//! - the settings become a `BrushTip` at the press (Circular = the round-3 stroke; Sphere = the ball or a shape);
//! - a 3D stroke keeps two queues: every pointer position with the view transform of its frame (`unresolved`), and
//!   the ray hits walked from them (`pending`). Positions are resolved in today's screen steps under a raycast budget,
//!   samples are painted under a time rule (`StrokePace`). The painted pixels depend on the pointer path only, never
//!   on the clock or on a later camera move; a stroke that still trails after the release is finished in the next
//!   frames ("finishing");
//! - round 5 (B-3): the time rule is checked after every dab, not only between samples (`stroke_to_3d_paced`): a
//!   sample whose segment does not fit in the frame stays at the front of `pending`, partly painted, and is resumed in
//!   the next frame (`stroke_resume`); the dabs and their order do not change;
//! - round 5 (B-2): the stroke's log line names the size, the ring, the dabs and the longest dab; with
//!   `WTE_STROKE_LOG` set the core's stroke probe is on and every sample of 20 ms or more is logged with its phases;
//! - Ctrl+wheel changes Size, Shift+wheel turns a shape;
//! - the ring with the side normal for Circular, the shape's footprint and wire sections (`cursor_segments`), and
//!   the status bar hints.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use glam::{Mat3, Quat, Vec2, Vec3};
use wte_core::{
    BrushMode, BrushParams, BrushTip, Hit, PaintTarget, STROKE_PROBE_NAMES, StrokeCounters, StrokeStep, SurfaceReach,
    TextureSetId, TipAlign, TipField, TipVolume, tip_frame,
};

use crate::gpu::BrushRing;
use crate::state::{AppState, BrushSettings, BrushType, ShadingMode, ShapeChoice, ToolKind};
use crate::tools::{NO_UV_TEXT, Surface, Tool, ViewPointer, ViewXform, error_text, paint_layer, set_size, wrap_uv};

/// Raycasts per frame of a 3D stroke while the button is down (8.6); the rest waits in the queue.
const RAYCASTS_PER_FRAME: u32 = 64;
/// Raycasts per frame after the release (round 3: the samples that finished a stroke on release).
const FINISH_SAMPLES: u32 = 4096;
/// Size factor per wheel notch of Ctrl+wheel (as the Stamp tool).
const SIZE_STEP: f32 = 1.1;
/// Turn per wheel notch of Shift+wheel, degrees (as the Stamp tool rotates).
const TURN_STEP_DEG: f32 = 5.0;
/// Brush size range (texels), as the `[` `]` shortcuts.
const SIZE_RANGE: (f32, f32) = (1.0, 2048.0);
/// Lattice of the footprint cut (FEATURE_BRUSH 5.11: 49 x 49 lookups).
const FOOTPRINT_LATTICE: u32 = 48;
/// Segments of one wire circle of the ball.
const CIRCLE_SEGMENTS: usize = 48;
/// Painted samples the mean sample time is taken over.
const MEAN_WINDOW: usize = 16;
/// With `WTE_STROKE_LOG`, a sample that took this long or longer gets its own log line (round 5 B-2).
const SLOW_SAMPLE: f64 = 0.020;

/// True when `WTE_STROKE_LOG` is set (read once); the first call then turns the core's stroke probe on.
fn stroke_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("WTE_STROKE_LOG").is_some();
        if on {
            wte_core::set_stroke_probe(true);
            log::info!("brush: WTE_STROKE_LOG is set, the stroke probe is on");
        }
        on
    })
}

/// The probe's phases of `c` minus `base` as "name ms" (times) or "name n" (counts, slots 4, 5, 7, 10, 11, 12, 14).
fn phase_text(c: &StrokeCounters, base: &StrokeCounters) -> String {
    let parts: Vec<String> = STROKE_PROBE_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let d = c.nanos[i].saturating_sub(base.nanos[i]);
            match i {
                4 | 7 => format!("{name} {}", d / 1_000_000),
                5 | 10 | 11 | 12 | 14 => format!("{name} {d}"),
                _ => format!("{name} {:.1} ms", d as f64 / 1e6),
            }
        })
        .collect();
    parts.join(", ")
}

/// The time rule of FEATURE_BRUSH 5.10 for `spent` seconds of painting in a frame: stop at `frame_cap`, and at `soft`
/// while the button is down and the backlog is at most `lag_target`.
fn over_limit(pace: &StrokePace, released: bool, spent: f64, backlog: f64) -> bool {
    spent >= pace.frame_cap || (!released && spent >= pace.soft && backlog <= pace.lag_target)
}

/// Status bar while the brush is over the model (FEATURE_BRUSH 6.6).
pub(crate) const HINT_CIRCULAR: &str = "Circular brush";
pub(crate) const HINT_SPHERE: &str = "Sphere brush: paints everything inside the ball, also hidden surfaces";
pub(crate) const HINT_SHAPE_PREFIX: &str = "Shape brush: ";
/// Status bar while the paint of a held stroke trails by more than the lag target.
pub(crate) const HINT_CATCHING_UP: &str = "Painting... the stroke is catching up";
/// Status bar after the release until the stroke is complete.
pub(crate) const HINT_FINISHING: &str = "Finishing the stroke... (Esc discards it)";
/// Info toast of a press with Shape = File while the shape is still loading (FEATURE_BRUSH 4.3).
pub(crate) const STILL_LOADING_TEXT: &str = "The brush shape is still loading";
/// Info toast of a press with Shape = File without a loaded shape and nothing loading, once per program run.
pub(crate) const NO_SHAPE_TEXT: &str = "No brush shape is loaded; painting with the ball";

/// The numbers of the stroke pacing (FEATURE_BRUSH 5.10). Fields of the tool so that tests can change them.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct StrokePace {
    /// Raycasts per frame while the button is down.
    pub raycasts_per_frame: u32,
    /// Raycasts per frame after the release.
    pub finish_raycasts: u32,
    /// Seconds of painting per frame while the backlog is at most `lag_target`.
    pub soft: f64,
    /// Seconds of waiting work above which frames grow to `frame_cap`.
    pub lag_target: f64,
    /// Seconds of painting per frame at most (plus the sample that crosses it).
    pub frame_cap: f64,
}

impl Default for StrokePace {
    fn default() -> StrokePace {
        StrokePace {
            raycasts_per_frame: RAYCASTS_PER_FRAME,
            finish_raycasts: FINISH_SAMPLES,
            soft: 0.010,
            lag_target: 0.250,
            frame_cap: 0.100,
        }
    }
}

/// The clock of the time rule: the wall clock, or (tests) a clock that advances by a fixed time per painted sample
/// (the time rule then never splits a sample), or by a fixed time per painted dab.
#[derive(Clone, Debug)]
pub(crate) enum StrokeClock {
    Wall(Instant),
    #[cfg(test)]
    Fixed { per_sample: f64, now: f64 },
    #[cfg(test)]
    PerDab { per_dab: f64, now: f64 },
}

impl Default for StrokeClock {
    fn default() -> StrokeClock {
        StrokeClock::Wall(Instant::now())
    }
}

impl StrokeClock {
    /// Seconds since the clock started.
    fn now(&self) -> f64 {
        match self {
            StrokeClock::Wall(origin) => origin.elapsed().as_secs_f64(),
            #[cfg(test)]
            StrokeClock::Fixed { now, .. } | StrokeClock::PerDab { now, .. } => *now,
        }
    }

    /// A sample was painted (the fixed clock advances).
    fn sample_painted(&mut self) {
        #[cfg(test)]
        if let StrokeClock::Fixed { per_sample, now } = self {
            *now += *per_sample;
        }
    }

    /// A dab was painted (the per-dab clock advances).
    fn dab_painted(&mut self) {
        #[cfg(test)]
        if let StrokeClock::PerDab { per_dab, now } = self {
            *now += *per_dab;
        }
    }

    /// True when the time rule is checked after every dab (the wall clock and the per-dab clock): a clock that only
    /// moves between samples cannot split one.
    fn paces_dabs(&self) -> bool {
        match self {
            StrokeClock::Wall(_) => true,
            #[cfg(test)]
            StrokeClock::Fixed { .. } => false,
            #[cfg(test)]
            StrokeClock::PerDab { .. } => true,
        }
    }
}

/// A walked step of the drag: a ray hit or a miss.
#[derive(Copy, Clone, Debug)]
enum QueuedSample {
    Hit(Hit),
    Miss,
}

/// The numbers of one stroke for the log line of FEATURE_BRUSH 8.4 (P8).
#[derive(Clone, Debug, Default)]
pub(crate) struct StrokeStats {
    pub samples: u32,
    /// Seconds of painting in the whole stroke.
    pub work: f64,
    /// The longest frame of painting (seconds), the release frame and the frames after it included.
    pub longest_frame: f64,
    pub longest_sample: f64,
    /// The largest backlog estimate (seconds).
    pub max_backlog: f64,
    /// Clock time of the release.
    pub released_at: Option<f64>,
    /// The longest single dab (seconds, wall clock).
    pub longest_dab: f64,
    /// The most dabs painted in one frame.
    pub most_dabs_in_frame: u32,
    /// Samples that a frame's time rule split (a partial sample counts once per split).
    pub splits: u32,
    /// The brush size (texels) and the ring on screen (points) at the press.
    pub size_px: f32,
    pub ring_pt: f32,
}

/// A stroke in progress.
#[derive(Copy, Clone, Debug)]
struct ActiveStroke {
    view_3d: bool,
    /// The set of a 2D stroke; the first set of a 3D stroke.
    set: TextureSetId,
    /// 3D: the last screen position that was resolved (raycast).
    last: egui::Pos2,
    /// 3D: brush radius in world units.
    radius_world: f32,
    /// 3D: brush radius on screen at the last resolved hit (points).
    ring_px: f32,
}

/// Where the ring sits on the model.
#[derive(Copy, Clone, Debug)]
struct RingAt {
    position: Vec3,
    set: TextureSetId,
    /// Viewer-facing normal of the hit (offset and frame); zero when located from the 2D view.
    facing: Vec3,
    /// Unflipped surface normal of the hit (the side filter of Circular); `None` when located from the 2D view.
    side: Option<Vec3>,
}

/// The shape cursor as screen segments (FEATURE_BRUSH 5.11): the bold footprint and the thin wire sections.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CursorLines {
    pub footprint: Vec<[egui::Pos2; 2]>,
    pub wires: Vec<[egui::Pos2; 2]>,
}

/// The same in world space (tests compare sizes there).
#[derive(Clone, Debug, Default)]
struct CursorWorld {
    footprint: Vec<[Vec3; 2]>,
    wires: Vec<[Vec3; 2]>,
}

/// What the settings give for a 3D stroke (FEATURE_BRUSH 4.3).
#[derive(Clone, Debug)]
enum TipChoice {
    Tip(BrushTip),
    /// Shape = File, no shape yet, a load is running: no stroke.
    StillLoading,
    /// Shape = File, no shape and nothing loading: the ball (with a toast once per program run).
    NoShape(BrushTip),
}

/// Brush and eraser: strokes in the 3D and the 2D view.
#[derive(Default)]
pub struct BrushTool {
    stroke: Option<ActiveStroke>,
    /// The tip of the open 3D stroke (for the cursor; the core holds its own copy).
    stroke_tip: Option<BrushTip>,
    /// Surface point of the ring on the model (from the 3D hover / the last painted sample, or located from the 2D
    /// cursor).
    ring: Option<RingAt>,
    /// Set and UV under the cursor (for the circle in the 2D view).
    cursor_uv: Option<(TextureSetId, Vec2)>,
    /// Raycasts left in this frame.
    budget: u32,
    /// Every pointer position of the drag that is not walked yet, with the transform of the frame it arrived in.
    unresolved: VecDeque<(egui::Pos2, ViewXform)>,
    /// Walked steps that are not painted yet.
    pending: VecDeque<QueuedSample>,
    /// The button of the open stroke went up.
    released: bool,
    pace: StrokePace,
    clock: StrokeClock,
    /// Times of the last painted samples (seconds), at most `MEAN_WINDOW`.
    recent: VecDeque<f64>,
    /// Painting in this frame: seconds, and whether anything (a dab or a sample) was painted.
    frame_spent: f64,
    frame_painted: bool,
    /// Dabs painted in this frame.
    frame_dabs: u32,
    /// The sample at the front of `pending` is partly painted (round 5 B-3): it is resumed, not started.
    partial: bool,
    /// Seconds spent on the front sample so far (over the frames of a partial sample).
    sample_spent: f64,
    /// Frames the front sample has been painted in so far.
    sample_frames: u32,
    /// With `WTE_STROKE_LOG`: the core's counters when the front sample started.
    sample_base: StrokeCounters,
    /// The largest backlog estimate of this frame (seconds), for the status bar.
    frame_backlog: f64,
    stats: StrokeStats,
    /// The 3D view transform of the last hover / drag (for the cursor probe of frames without input).
    last_xf: Option<ViewXform>,
    /// `hover` ran in this frame (the hover hint is cleared at the end of a frame without one).
    hovered: bool,
}

/// The settings of the active tool (eraser or brush).
fn settings(state: &AppState) -> &BrushSettings {
    if state.tool == ToolKind::Eraser { &state.tools.eraser } else { &state.tools.brush }
}

fn settings_mut(state: &mut AppState) -> &mut BrushSettings {
    if state.tool == ToolKind::Eraser { &mut state.tools.eraser } else { &mut state.tools.brush }
}

/// The user's rotation of a shape (FEATURE_BRUSH 4.3): `Ry(-turn) * Rx(tilt) * Rz(roll)`.
fn shape_rotation(s: &BrushSettings) -> Quat {
    Quat::from_rotation_y(-s.turn_deg.to_radians())
        * Quat::from_rotation_x(s.tilt_deg.to_radians())
        * Quat::from_rotation_z(s.roll_deg.to_radians())
}

/// The built-in field of `shape` if it has been made (`AppState::builtin_tips`: Cube, Cylinder).
fn builtin(state: &AppState, shape: ShapeChoice) -> Option<Arc<TipField>> {
    match shape {
        ShapeChoice::Cube => state.builtin_tips[0].clone(),
        ShapeChoice::Cylinder => state.builtin_tips[1].clone(),
        ShapeChoice::Ball | ShapeChoice::File => None,
    }
}

/// Makes the built-in field of `shape` on first use.
fn ensure_builtin(state: &mut AppState, shape: ShapeChoice) {
    match shape {
        ShapeChoice::Cube => {
            state.builtin_tips[0].get_or_insert_with(|| Arc::new(TipField::cube()));
        }
        ShapeChoice::Cylinder => {
            state.builtin_tips[1].get_or_insert_with(|| Arc::new(TipField::cylinder()));
        }
        ShapeChoice::Ball | ShapeChoice::File => {}
    }
}

/// The table of FEATURE_BRUSH 4.3 (without the connected-surface rows, plan section 7 item 1). `up_hint` = the
/// camera's up direction. A built-in shape that has not been made yet counts as the ball (callers that start strokes
/// make it first with `ensure_builtin`).
fn tip_choice(state: &AppState, up_hint: Vec3) -> TipChoice {
    let s = settings(state);
    let ball = |reach| BrushTip { reach, offset: s.offset, up_hint, ..BrushTip::legacy(reach == SurfaceReach::SameSide) };
    if s.kind == BrushType::Circular {
        return TipChoice::Tip(BrushTip { up_hint, ..BrushTip::legacy(true) });
    }
    let field = match s.shape {
        ShapeChoice::Ball => None,
        ShapeChoice::Cube | ShapeChoice::Cylinder => builtin(state, s.shape),
        ShapeChoice::File => match (&state.brush_shape.current, &state.brush_shape.pending) {
            (Some(ready), _) => Some(Arc::clone(&ready.field)),
            (None, Some(_)) => return TipChoice::StillLoading,
            (None, None) => return TipChoice::NoShape(ball(SurfaceReach::All)),
        },
    };
    match field {
        None => TipChoice::Tip(ball(SurfaceReach::All)),
        Some(field) => TipChoice::Tip(BrushTip {
            through: s.through || field.is_flat(),
            volume: TipVolume::Field(field),
            reach: SurfaceReach::All,
            align: s.align,
            rotation: shape_rotation(s),
            offset: s.offset,
            scale: 1.0,
            up_hint,
        }),
    }
}

/// The status bar text for the settings while the brush is over the model (FEATURE_BRUSH 6.6).
fn type_hint(state: &AppState) -> String {
    let s = settings(state);
    match (s.kind, s.shape) {
        (BrushType::Circular, _) => HINT_CIRCULAR.to_owned(),
        (BrushType::Sphere, ShapeChoice::Ball) => HINT_SPHERE.to_owned(),
        (BrushType::Sphere, ShapeChoice::Cube) => format!("{HINT_SHAPE_PREFIX}Cube"),
        (BrushType::Sphere, ShapeChoice::Cylinder) => format!("{HINT_SHAPE_PREFIX}Cylinder"),
        (BrushType::Sphere, ShapeChoice::File) => match &state.brush_shape.current {
            Some(ready) => format!("{HINT_SHAPE_PREFIX}{}", ready.name),
            None => HINT_SPHERE.to_owned(),
        },
    }
}

/// True for a status bar text this tool wrote.
fn is_brush_hint(hint: &str) -> bool {
    hint == HINT_CIRCULAR
        || hint == HINT_SPHERE
        || hint == HINT_CATCHING_UP
        || hint == HINT_FINISHING
        || hint.starts_with(HINT_SHAPE_PREFIX)
}

/// Shows `text` in the status bar unless another feature's hint is there.
fn set_hint(state: &mut AppState, text: &str) {
    if state.status_hint != text && (state.status_hint.is_empty() || is_brush_hint(&state.status_hint)) {
        state.status_hint = text.to_owned();
    }
}

/// Clears the status bar hint if this tool wrote it.
fn clear_hint(state: &mut AppState) {
    if is_brush_hint(&state.status_hint) {
        state.status_hint.clear();
    }
}

/// The segment `a`-`b` clipped to the ball of radius `r` around `c`; `None` when it lies outside.
fn clip_to_ball(a: Vec3, b: Vec3, c: Vec3, r: f32) -> Option<[Vec3; 2]> {
    let d = b - a;
    let f = a - c;
    let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - r * r);
    if qa <= f32::EPSILON {
        return (qc <= 0.0).then_some([a, b]);
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let (t0, t1) = (((-qb - sq) / (2.0 * qa)).max(0.0), ((-qb + sq) / (2.0 * qa)).min(1.0));
    (t0 < t1).then(|| [a + d * t0, a + d * t1])
}

impl BrushTool {
    /// Start of a frame: refills the raycast budget, starts the frame's account of painting time and clears the hover
    /// feedback.
    pub(crate) fn begin_frame(&mut self) {
        self.close_frame();
        self.budget = if self.released { self.pace.finish_raycasts } else { self.pace.raycasts_per_frame };
        self.hovered = false;
        if self.stroke.is_none() {
            self.ring = None;
            self.cursor_uv = None;
        }
    }

    /// Books the painting time of the frame that ends into the stroke's longest frame.
    fn close_frame(&mut self) {
        self.stats.longest_frame = self.stats.longest_frame.max(self.frame_spent);
        self.stats.most_dabs_in_frame = self.stats.most_dabs_in_frame.max(self.frame_dabs);
        self.frame_spent = 0.0;
        self.frame_painted = false;
        self.frame_dabs = 0;
        self.frame_backlog = 0.0;
    }

    /// Brush parameters for a stroke in `radius` units.
    fn params(state: &AppState, radius: f32) -> BrushParams {
        let s = settings(state);
        BrushParams {
            radius,
            hardness: s.hardness,
            opacity: s.opacity,
            flow: s.flow,
            spacing: s.spacing,
            color: state.colors.primary,
            mode: if state.tool == ToolKind::Eraser { BrushMode::Erase } else { BrushMode::Paint },
            // Read by the 2D stroke (`stroke_begin`) only; a 3D stroke's tip decides (`stroke_begin_with`).
            same_side_only: s.kind == BrushType::Circular,
        }
    }

    /// Stroke pacing (FEATURE_BRUSH 5.10), called by `ToolController::end_frame` once per frame: resolves and paints
    /// what waits under the budget and the time rule (nothing more when `drag` already used them up in this frame),
    /// and ends a released stroke whose queues are empty. `true` while work is still queued (the controller then asks
    /// for the next frame at once).
    pub(crate) fn pump(&mut self, state: &mut AppState) -> bool {
        let Some(st) = self.stroke else {
            if !self.hovered {
                clear_hint(state);
            }
            return false;
        };
        if !st.view_3d {
            return false;
        }
        self.advance(state, false);
        !self.queues_empty()
    }

    /// True while a stroke whose button is already up is still being painted (its queues are not empty).
    pub(crate) fn finishing(&self) -> bool {
        self.released && self.stroke.is_some()
    }

    /// Completes a finishing stroke without a time limit: resolves and paints everything that waits, then
    /// `stroke_end` (one history entry). Called by the controller for every command but Undo and Esc, and for a new
    /// press in a view.
    pub(crate) fn complete(&mut self, state: &mut AppState) {
        if self.stroke.is_some() {
            self.complete_stroke(state);
        }
    }

    /// Resolves and paints everything that waits (no budget, no time rule) and ends the stroke.
    fn complete_stroke(&mut self, state: &mut AppState) {
        let mut budget = u32::MAX;
        self.resolve(state, &mut budget);
        self.paint(state, true);
        self.end(state);
    }

    fn queues_empty(&self) -> bool {
        self.unresolved.is_empty() && self.pending.is_empty()
    }

    /// One step of a 3D stroke: resolve under this frame's raycast budget, paint under the time rule, keep the hint
    /// and the cursor probe current, and end a released stroke whose queues are empty. `unlimited` = no time rule.
    fn advance(&mut self, state: &mut AppState, unlimited: bool) {
        let mut budget = self.budget;
        self.resolve(state, &mut budget);
        self.budget = budget;
        self.paint(state, unlimited);
        if self.released && self.queues_empty() {
            self.end(state);
            return;
        }
        let backlog = self.frame_backlog.max(self.pending.len() as f64 * self.mean_sample());
        if self.released {
            set_hint(state, HINT_FINISHING);
        } else if backlog > self.pace.lag_target {
            set_hint(state, HINT_CATCHING_UP);
        } else if state.status_hint == HINT_CATCHING_UP {
            let hint = type_hint(state);
            set_hint(state, &hint);
        }
        if let Some(xf) = self.last_xf {
            self.write_probe(state, &xf);
        }
    }

    /// Mean time of the last painted samples (0 before the first).
    fn mean_sample(&self) -> f64 {
        if self.recent.is_empty() { 0.0 } else { self.recent.iter().sum::<f64>() / self.recent.len() as f64 }
    }

    /// Books one painted sample of `dt` seconds (summed over the frames of a partial sample; the frame's own time is
    /// booked as it is spent).
    fn note_sample(&mut self, dt: f64) {
        if self.recent.len() == MEAN_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(dt);
        self.stats.samples += 1;
        self.stats.work += dt;
        self.stats.longest_sample = self.stats.longest_sample.max(dt);
    }

    /// Appends a pointer position of the drag with the transform of its frame; a position within 0.001 points of
    /// the last queued (or resolved) one is dropped.
    fn queue_position(&mut self, pos: egui::Pos2, xf: &ViewXform) {
        let last = self.unresolved.back().map(|(p, _)| *p).or(self.stroke.map(|st| st.last));
        if last.is_some_and(|l| l.distance(pos) < 1e-3) || !pos.is_finite() {
            return;
        }
        self.unresolved.push_back((pos, *xf));
    }

    /// Walks the queued screen positions from the last resolved one in steps of `max(1 pt, ring / 4)`, one raycast per
    /// step with the transform stored with the position the step leads to, at most `budget` raycasts. Each step
    /// appends a hit or a miss to `pending`.
    fn resolve(&mut self, state: &mut AppState, budget: &mut u32) {
        let Some(mut st) = self.stroke else { return };
        let Some(session) = state.session.as_ref() else { return };
        while let Some(&(target, xf)) = self.unresolved.front() {
            let remaining = target - st.last;
            let len = remaining.length();
            if len < 1e-3 {
                self.unresolved.pop_front();
                continue;
            }
            if *budget == 0 {
                break;
            }
            let step = (0.25 * st.ring_px).max(1.0);
            let next = if len <= step { target } else { st.last + remaining / len * step };
            *budget -= 1;
            st.last = next;
            match xf.surface_at(&session.doc, session.active_set, next) {
                Surface::Model(hit) => {
                    if let Some(px) = xf.world_len_on_screen(hit.position, st.radius_world) {
                        st.ring_px = px;
                    }
                    self.pending.push_back(QueuedSample::Hit(hit));
                }
                _ => self.pending.push_back(QueuedSample::Miss),
            }
        }
        self.stroke = Some(st);
    }

    /// Paints the walked samples in order. Without `unlimited`: until the queue is empty, or `soft` seconds are spent
    /// in this frame while the backlog is at most `lag_target` (only while the button is down: a released stroke is
    /// finished in frames of up to `frame_cap`, FEATURE_BRUSH 5.10 / test A5 iii), or `frame_cap` seconds are spent;
    /// at least one dab per frame. Round 5 (B-3): the rule is checked after every dab too; a sample it splits stays
    /// at the front of `pending` (`partial`) and the frame ends. The backlog estimate counts it as one sample.
    fn paint(&mut self, state: &mut AppState, unlimited: bool) {
        let Some(st) = self.stroke else { return };
        let logging = stroke_log();
        while let Some(&sample) = self.pending.front() {
            let backlog = self.pending.len() as f64 * self.mean_sample();
            self.stats.max_backlog = self.stats.max_backlog.max(backlog);
            self.frame_backlog = self.frame_backlog.max(backlog);
            if !unlimited && self.frame_painted && over_limit(&self.pace, self.released, self.frame_spent, backlog) {
                break;
            }
            let Some(doc) = state.session.as_mut().map(|s| &mut s.doc) else { return };
            if logging && !self.partial {
                self.sample_base = doc.stroke_counters();
            }
            let t0 = self.clock.now();
            let step = match sample {
                QueuedSample::Hit(hit) => {
                    let pace_dabs = !unlimited && self.clock.paces_dabs();
                    let (pace, released, frame_spent) = (self.pace, self.released, self.frame_spent);
                    let (clock, stats, frame_dabs) = (&mut self.clock, &mut self.stats, &mut self.frame_dabs);
                    let mut last = Instant::now();
                    let mut more = || {
                        let now = Instant::now();
                        stats.longest_dab = stats.longest_dab.max((now - last).as_secs_f64());
                        last = now;
                        *frame_dabs += 1;
                        clock.dab_painted();
                        !pace_dabs || !over_limit(&pace, released, frame_spent + (clock.now() - t0), backlog)
                    };
                    let step =
                        if self.partial { doc.stroke_resume(&mut more) } else { doc.stroke_to_3d_paced(&hit, 1.0, &mut more) };
                    let set = hit.texture_set.unwrap_or(st.set);
                    self.ring = Some(RingAt { position: hit.position, set, facing: hit.normal, side: Some(hit.surface_normal()) });
                    self.cursor_uv = hit.texture_set.zip(hit.uv).map(|(s, uv)| (s, wrap_uv(uv)));
                    step
                }
                QueuedSample::Miss => {
                    doc.stroke_break();
                    self.ring = None;
                    self.cursor_uv = None;
                    StrokeStep::Done
                }
            };
            if step == StrokeStep::Done {
                self.clock.sample_painted();
            }
            let dt = (self.clock.now() - t0).max(0.0);
            self.frame_spent += dt;
            self.frame_painted = true;
            self.sample_spent += dt;
            self.sample_frames += 1;
            if step == StrokeStep::Partial {
                self.partial = true;
                self.stats.splits += 1;
                break;
            }
            self.pending.pop_front();
            self.partial = false;
            let (total, frames) = (std::mem::take(&mut self.sample_spent), std::mem::take(&mut self.sample_frames));
            self.note_sample(total);
            if logging && total >= SLOW_SAMPLE {
                self.log_slow_sample(state, total, frames);
            }
        }
    }

    /// `WTE_STROKE_LOG`: one line for a sample of `SLOW_SAMPLE` or more: its time, frames, dabs and phases.
    fn log_slow_sample(&self, state: &AppState, total: f64, frames: u32) {
        let Some(doc) = state.doc() else { return };
        let (c, base) = (doc.stroke_counters(), &self.sample_base);
        log::info!(
            "brush sample: {:.1} ms over {frames} frame(s), {} dabs, {} set dabs, sets started {}, buffer copies {}; {}",
            total * 1e3,
            c.dabs - base.dabs,
            c.set_dabs - base.set_dabs,
            c.sets_started - base.sets_started,
            c.buffer_copies - base.buffer_copies,
            phase_text(&c, base)
        );
    }

    /// `stroke_end` (one history entry), the skipped-sets warning and the log line of the stroke; the tool is idle
    /// afterwards.
    fn end(&mut self, state: &mut AppState) {
        let was_3d = self.stroke.is_some_and(|st| st.view_3d);
        if was_3d {
            self.close_frame();
            let s = &self.stats;
            let tail = s.released_at.map_or(0.0, |t| (self.clock.now() - t).max(0.0));
            let counters = state.doc().map(|d| d.stroke_counters()).unwrap_or_default();
            // Round 5 B-2: size, ring, dabs and the longest dab are appended at the end (the gate's regex keeps
            // matching the part before).
            log::info!(
                "brush stroke: {} samples, stroke work {:.1} ms, longest frame {:.1} ms, longest sample {:.1} ms, \
                 largest backlog {:.1} ms, release to end {:.1} ms, size {:.0} px, ring {:.1} pt, {} dabs, \
                 longest dab {:.1} ms",
                s.samples,
                s.work * 1e3,
                s.longest_frame * 1e3,
                s.longest_sample * 1e3,
                s.max_backlog * 1e3,
                tail * 1e3,
                s.size_px,
                s.ring_pt,
                counters.dabs,
                s.longest_dab * 1e3
            );
            if stroke_log() {
                log::info!(
                    "brush stroke probe: {} dabs, {} set dabs, sets started {}, buffer copies {}, split samples {}, \
                     most dabs in a frame {}; {}",
                    counters.dabs,
                    counters.set_dabs,
                    counters.sets_started,
                    counters.buffer_copies,
                    s.splits,
                    s.most_dabs_in_frame,
                    phase_text(&counters, &StrokeCounters::default())
                );
            }
        }
        self.reset_stroke();
        end_stroke(state);
        clear_hint(state);
    }

    /// Drops the stroke and both queues (the document is not touched).
    fn reset_stroke(&mut self) {
        self.stroke = None;
        self.stroke_tip = None;
        self.unresolved.clear();
        self.pending.clear();
        self.released = false;
        self.partial = false;
        self.sample_spent = 0.0;
        self.sample_frames = 0;
    }

    /// Opens a stroke; a core error is a warning toast.
    fn begin(state: &mut AppState, params: &BrushParams, tip: Option<&BrushTip>) -> bool {
        let Some(doc) = state.session.as_mut().map(|s| &mut s.doc) else { return false };
        let result = match tip {
            Some(tip) => doc.stroke_begin_with(params, tip),
            None => doc.stroke_begin(params),
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                state.toasts.warn(error_text(&e));
                false
            }
        }
    }

    /// The tip of a 3D stroke from the settings (FEATURE_BRUSH 4.3) with its toasts; `None` = no stroke.
    fn press_tip(state: &mut AppState, up_hint: Vec3) -> Option<BrushTip> {
        let (kind, shape) = (settings(state).kind, settings(state).shape);
        if kind == BrushType::Sphere {
            ensure_builtin(state, shape);
        }
        match tip_choice(state, up_hint) {
            TipChoice::Tip(tip) => Some(tip),
            TipChoice::StillLoading => {
                state.toasts.info(STILL_LOADING_TEXT);
                None
            }
            TipChoice::NoShape(tip) => {
                if !state.brush_shape.warned_missing {
                    state.brush_shape.warned_missing = true;
                    state.toasts.info(NO_SHAPE_TEXT);
                }
                Some(tip)
            }
        }
    }

    fn press_3d(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) -> bool {
        let Surface::Model(hit) = p.surface else { return false };
        // (1) No UVs here.
        let Some(set) = hit.texture_set else {
            state.toasts.warn(NO_UV_TEXT);
            return false;
        };
        // (1b) The tip of the settings; Shape = File while the shape still loads paints nothing.
        let up_hint = xf.camera.basis().1;
        let Some(tip) = Self::press_tip(state, up_hint) else { return false };
        // (2) The hit's set becomes the active set.
        let Some(session) = state.session.as_mut() else { return false };
        session.active_set = set;
        // (3) The paintable layer of that set.
        if paint_layer(state, set).is_none() {
            return false;
        }
        // (4) Every other set without a paint layer gets one, so a stroke across a material boundary paints both.
        let Some(doc) = state.session.as_mut().map(|s| &mut s.doc) else { return false };
        for other in doc.set_ids() {
            if other != set
                && matches!(doc.paint_target(other), PaintTarget::NotRaster(_) | PaintTarget::NoActiveLayer)
                && let Err(e) = doc.add_raster_layer(other, None)
            {
                state.toasts.warn(error_text(&e));
            }
        }
        // (5) Painting must be visible.
        if state.flags.shading != ShadingMode::Textured {
            state.flags.shading = ShadingMode::Textured;
            state.toasts.info("Texture display turned on");
        }
        // (6) Stroke.
        let Some(doc) = state.doc() else { return false };
        let radius = settings(state).size_px * 0.5 * doc.texel_world_size(set);
        let params = Self::params(state, radius);
        if !Self::begin(state, &params, Some(&tip)) {
            return false;
        }
        self.reset_stroke();
        self.recent.clear();
        let ring_px = xf.world_len_on_screen(hit.position, radius).unwrap_or(8.0);
        self.stats = StrokeStats { size_px: settings(state).size_px, ring_pt: ring_px, ..StrokeStats::default() };
        self.stroke = Some(ActiveStroke { view_3d: true, set, last: p.pos, radius_world: radius, ring_px });
        self.stroke_tip = Some(tip);
        self.last_xf = Some(*xf);
        // The first sample is painted at once (as in round 3).
        self.pending.push_back(QueuedSample::Hit(hit));
        self.paint(state, true);
        true
    }

    fn press_2d(&mut self, state: &mut AppState, p: &ViewPointer) -> bool {
        let Surface::Uv { set, uv, px } = p.surface else { return false };
        if paint_layer(state, set).is_none() {
            return false;
        }
        // A 2D stroke is a flat disc whatever the type (FEATURE_BRUSH 1.6): `stroke_begin`, no tip.
        let params = Self::params(state, settings(state).size_px * 0.5);
        if !Self::begin(state, &params, None) {
            return false;
        }
        if let Some(doc) = state.doc_mut() {
            doc.stroke_to_2d(set, px, p.pressure);
        }
        self.reset_stroke();
        self.stroke = Some(ActiveStroke { view_3d: false, set, last: p.pos, radius_world: 0.0, ring_px: 0.0 });
        self.cursor_uv = Some((set, uv));
        self.ring = state
            .doc()
            .and_then(|d| crate::view::locate_px(d, set, px))
            .map(|w| RingAt { position: w, set, facing: Vec3::ZERO, side: None });
        true
    }

    fn drag_2d(&mut self, state: &mut AppState, p: &ViewPointer) {
        let Some(st) = self.stroke else { return };
        let Some(doc) = state.doc_mut() else { return };
        match p.surface {
            Surface::Uv { set, uv, px } if set == st.set => {
                doc.stroke_to_2d(set, px, p.pressure);
                self.cursor_uv = Some((set, uv));
                self.ring = crate::view::locate_px(doc, set, px).map(|w| RingAt { position: w, set, facing: Vec3::ZERO, side: None });
            }
            _ => {
                doc.stroke_break();
                self.cursor_uv = None;
                self.ring = None;
            }
        }
    }

    /// The tip the cursor shows: the open stroke's, else the one the settings would give now.
    fn cursor_tip(&self, state: &AppState, up_hint: Vec3) -> BrushTip {
        if let Some(tip) = &self.stroke_tip {
            return tip.clone();
        }
        match tip_choice(state, up_hint) {
            TipChoice::Tip(tip) | TipChoice::NoShape(tip) => tip,
            TipChoice::StillLoading => BrushTip { up_hint, ..BrushTip::legacy(false) },
        }
    }

    /// Radius of the ring in world units: the open 3D stroke's, else from the live Size at the ring's set.
    fn ring_radius(&self, state: &AppState, ring: &RingAt) -> Option<f32> {
        match self.stroke {
            Some(st) if st.view_3d => Some(st.radius_world),
            _ => Some(settings(state).size_px * 0.5 * state.doc()?.texel_world_size(ring.set)),
        }
    }

    /// The shape cursor in world space (FEATURE_BRUSH 5.11): empty for Circular, in the 2D view and for a ring located
    /// from the 2D view; three wire circles for the ball; the footprint and the three principal sections for a shape.
    fn cursor_world(&self, state: &AppState, xf: &ViewXform) -> CursorWorld {
        let mut out = CursorWorld::default();
        if !xf.is_3d {
            return out;
        }
        let Some(ring) = self.ring else { return out };
        if ring.side.is_none() {
            return out;
        }
        let Some(r) = self.ring_radius(state, &ring) else { return out };
        let up_hint = xf.camera.basis().1;
        let tip = self.cursor_tip(state, up_hint).sanitized();
        if tip.reach == SurfaceReach::SameSide || !(r > 0.0 && r.is_finite()) {
            return out;
        }
        let h = ring.position;
        let facing = ring.facing.try_normalize().unwrap_or(Vec3::Y);
        let (a, up_t) = tip_frame(facing, tip.up_hint, None);
        let right = a.x_axis;
        // The pose: the last dab's during a stroke (shapes), else the one a dab at the hover point would get.
        let pose = if self.stroke_tip.is_some() { state.doc().and_then(|d| d.stroke_tip_pose()) } else { None };
        let k = r * tip.scale;
        let (center, rot, k) = match pose {
            Some(p) if !tip.is_ball() => (p.center, p.world_from_tip, p.unit),
            _ => {
                let align = if tip.align == TipAlign::Surface { a } else { Mat3::IDENTITY };
                (h + tip.offset * r * facing, align * Mat3::from_quat(tip.rotation), k)
            }
        };
        let Some(field) = tip.field() else {
            // The ball: three wire circles in the frame's planes.
            let center = h + tip.offset * r * facing;
            for (u, v) in [(right, up_t), (right, facing), (up_t, facing)] {
                for i in 0..CIRCLE_SEGMENTS {
                    let angle = |j: usize| j as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                    let at = |t: f32| center + (u * t.cos() + v * t.sin()) * r;
                    out.wires.push([at(angle(i)), at(angle(i + 1))]);
                }
            }
            return out;
        };
        let to_world = |q: Vec3| center + rot * q * k;
        let sections = field.principal_sections();
        for (axis, segs) in sections.iter().enumerate() {
            for [p, q] in segs {
                let lift = |s: Vec2| match axis {
                    0 => Vec3::new(0.0, s.x, s.y),
                    1 => Vec3::new(s.x, 0.0, s.y),
                    _ => Vec3::new(s.x, s.y, 0.0),
                };
                out.wires.push([to_world(lift(*p)), to_world(lift(*q))]);
            }
        }
        if tip.through || field.is_flat() {
            // The outline that is pressed through: the middle slice, clipped to the ring's ball.
            for [p, q] in &sections[1] {
                let (pa, pb) = (to_world(Vec3::new(p.x, 0.0, p.y)), to_world(Vec3::new(q.x, 0.0, q.y)));
                if let Some(seg) = clip_to_ball(pa, pb, center, r) {
                    out.footprint.push(seg);
                }
            }
        } else if k > 0.0 {
            // The cut of the shape with the tangent plane at the cursor.
            let inv = rot.transpose();
            let origin = inv * (h - center) / k;
            for [p, q] in field.section(origin, inv * right, inv * up_t, FOOTPRINT_LATTICE) {
                let world = |s: Vec2| h + (right * s.x + up_t * s.y) * k;
                out.footprint.push([world(p), world(q)]);
            }
        }
        out
    }

    /// The shape cursor as screen segments (FEATURE_BRUSH 5.11, test A18); segments with an end behind the camera are
    /// left out.
    pub(crate) fn cursor_segments(&self, state: &AppState, xf: &ViewXform) -> CursorLines {
        let world = self.cursor_world(state, xf);
        let project = |segs: Vec<[Vec3; 2]>| -> Vec<[egui::Pos2; 2]> {
            segs.into_iter().filter_map(|[a, b]| Some([xf.world_to_screen(a)?, xf.world_to_screen(b)?])).collect()
        };
        CursorLines { footprint: project(world.footprint), wires: project(world.wires) }
    }

    /// Publishes the cursor's segment counts for the self-test (`AppState::tool_probe`).
    fn write_probe(&self, state: &mut AppState, xf: &ViewXform) {
        let lines = self.cursor_segments(state, xf);
        state.tool_probe.cursor_footprint = lines.footprint.len();
        state.tool_probe.cursor_wires = lines.wires.len();
    }
}

/// `stroke_end` (one history entry) and the warning about sets the stroke skipped.
fn end_stroke(state: &mut AppState) {
    let Some(doc) = state.doc_mut() else { return };
    let skipped = doc.stroke_skipped_sets();
    doc.stroke_end();
    if !skipped.is_empty() {
        let names: Vec<String> = skipped.iter().filter_map(|(set, _)| doc.set(*set).map(|s| s.name.clone())).collect();
        state.toasts.warn(format!("Paint skipped on {}: the active layer is locked or hidden", names.join(", ")));
    }
}

impl Tool for BrushTool {
    fn hover(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        let Some(doc) = state.doc() else { return };
        self.hovered = true;
        match p.surface {
            Surface::Model(hit) => {
                self.ring = hit
                    .texture_set
                    .map(|set| RingAt { position: hit.position, set, facing: hit.normal, side: Some(hit.surface_normal()) });
                self.cursor_uv = hit.texture_set.zip(hit.uv).map(|(s, uv)| (s, wrap_uv(uv)));
            }
            Surface::Uv { set, uv, px } => {
                self.cursor_uv = Some((set, uv));
                self.ring = crate::view::locate_px(doc, set, px).map(|w| RingAt { position: w, set, facing: Vec3::ZERO, side: None });
            }
            Surface::Miss => {
                self.ring = None;
                self.cursor_uv = None;
            }
        }
        if xf.is_3d && matches!(p.surface, Surface::Model(_)) {
            let (kind, shape) = (settings(state).kind, settings(state).shape);
            if kind == BrushType::Sphere {
                ensure_builtin(state, shape);
            }
            let hint = type_hint(state);
            set_hint(state, &hint);
            self.last_xf = Some(*xf);
        } else {
            clear_hint(state);
        }
        self.write_probe(state, xf);
    }

    fn press(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) -> bool {
        if self.finishing() {
            self.complete_stroke(state);
        }
        self.stroke = None;
        if xf.is_3d { self.press_3d(state, xf, p) } else { self.press_2d(state, p) }
    }

    fn drag(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        let Some(st) = self.stroke else { return };
        if self.released {
            return; // a released stroke takes no more input (the view calls in every frame)
        }
        if st.view_3d {
            self.last_xf = Some(*xf);
            self.queue_position(p.pos, xf);
            self.advance(state, false);
        } else {
            self.drag_2d(state, p);
        }
    }

    fn release(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {
        let Some(st) = self.stroke else { return };
        if self.released {
            return;
        }
        if st.view_3d {
            self.queue_position(p.pos, xf);
            self.released = true;
            self.stats.released_at = Some(self.clock.now());
            self.budget = self.budget.max(self.pace.finish_raycasts);
            self.advance(state, false);
        } else {
            if p.pos != st.last {
                self.drag_2d(state, p);
            }
            self.reset_stroke();
            end_stroke(state);
        }
    }

    fn scroll(&mut self, state: &mut AppState, _xf: &ViewXform, p: &ViewPointer, notches: f32) -> bool {
        let s = settings_mut(state);
        if p.mods.ctrl || p.mods.command {
            s.size_px = (s.size_px * SIZE_STEP.powf(notches)).clamp(SIZE_RANGE.0, SIZE_RANGE.1);
            true
        } else if p.mods.shift && s.kind == BrushType::Sphere && s.shape != ShapeChoice::Ball {
            let turn = s.turn_deg + TURN_STEP_DEG * notches;
            // Keep the angle in (-180, 180].
            s.turn_deg = -((180.0 - turn).rem_euclid(360.0) - 180.0);
            true
        } else {
            false
        }
    }

    fn finish(&mut self, state: &mut AppState) {
        // The stroke up to the last pointer position the app has seen becomes the history entry (FEATURE_BRUSH 5.10:
        // the queued positions carry their own transforms).
        let Some(st) = self.stroke else { return };
        if st.view_3d {
            self.complete_stroke(state);
        } else {
            self.reset_stroke();
            end_stroke(state);
        }
    }

    fn cancel(&mut self, state: &mut AppState) {
        if self.stroke.is_some() {
            self.reset_stroke();
            if let Some(doc) = state.doc_mut() {
                doc.stroke_cancel();
            }
            clear_hint(state);
        }
    }

    fn overlay(&self, state: &AppState, xf: &ViewXform, painter: &egui::Painter) {
        if xf.is_3d {
            // The ring is drawn on the surface by the renderer (brush_ring); the shape cursor is drawn here.
            let lines = self.cursor_segments(state, xf);
            let wire = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(153));
            for seg in &lines.wires {
                painter.line_segment(*seg, wire);
            }
            let shadow = egui::Stroke::new(3.5, egui::Color32::from_black_alpha(140));
            for seg in &lines.footprint {
                painter.line_segment(*seg, shadow);
            }
            let bold = egui::Stroke::new(1.5, egui::Color32::WHITE);
            for seg in &lines.footprint {
                painter.line_segment(*seg, bold);
            }
            return;
        }
        let Some(session) = state.session.as_ref() else { return };
        let Some((set, uv)) = self.cursor_uv else { return };
        if set != session.active_set {
            return;
        }
        let tex = set_size(&session.doc, set);
        let center = xf.px_to_screen(wrap_uv(uv) * Vec2::new(tex[0] as f32, tex[1] as f32));
        let s = settings(state);
        let r = (s.size_px * 0.5 * xf.uv.zoom).max(1.0);
        painter.circle_stroke(center, r, egui::Stroke::new(3.0, egui::Color32::from_black_alpha(140)));
        painter.circle_stroke(center, r, egui::Stroke::new(1.25, egui::Color32::WHITE));
        let inner = r * s.hardness.clamp(0.0, 1.0);
        if inner > 1.5 && s.hardness < 0.999 {
            painter.circle_stroke(center, inner, egui::Stroke::new(1.0, egui::Color32::from_white_alpha(140)));
        }
    }

    fn brush_ring(&self, state: &AppState) -> Option<BrushRing> {
        let ring = self.ring?;
        let radius = self.ring_radius(state, &ring)?;
        let s = settings(state);
        // The open 3D stroke's type and offset, else the settings'.
        let (circular, offset) = match &self.stroke_tip {
            Some(tip) => (tip.reach == SurfaceReach::SameSide, if tip.reach == SurfaceReach::SameSide { 0.0 } else { tip.offset }),
            None => (s.kind == BrushType::Circular, if s.kind == BrushType::Circular { 0.0 } else { s.offset }),
        };
        let offset = if offset.is_finite() { offset.clamp(-1.0, 1.0) } else { 0.0 };
        let facing = ring.facing.try_normalize().unwrap_or(Vec3::ZERO);
        let center = if offset == 0.0 { ring.position } else { ring.position + offset * radius * facing };
        let side_normal = if circular { ring.side } else { None };
        // FEATURE_BRUSH 5.11: the inner hardness ring is drawn for the ball only (Circular or Sphere with the Ball). A
        // shape passes hardness 1, which the renderer draws without the inner ring (gate 1, B6's finding).
        let shape = match &self.stroke_tip {
            Some(tip) => !tip.is_ball(),
            None => {
                s.kind == BrushType::Sphere
                    && match s.shape {
                        ShapeChoice::Ball => false,
                        ShapeChoice::File => state.brush_shape.current.is_some(),
                        _ => true,
                    }
            }
        };
        let hardness = if shape { 1.0 } else { s.hardness };
        Some(BrushRing { center, radius, hardness, side_normal })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PendingShape, ReadyShape};
    use crate::tools::ToolController;
    use crate::tools::test_support::*;
    use wte_core::{CloseMode, LayerKind, SampleKind};

    /// FNV hash of a set's composite.
    fn composite_hash(state: &AppState, set: usize) -> u64 {
        wte_core::fnv1a64(0, &state.doc().unwrap().sets[set].composite().data)
    }

    /// FNV hash of every raster layer above the locked Base of every set (the painted pixels, without a recomposite).
    fn paint_hash(state: &AppState) -> u64 {
        let doc = state.doc().unwrap();
        doc.sets.iter().flat_map(|s| s.layers.iter().skip(1)).fold(0xcbf2_9ce4_8422_2325, |h, layer| match &layer.kind {
            LayerKind::Raster(buf) => wte_core::fnv1a64(h, &buf.data),
            _ => h,
        })
    }

    fn drag_path(tools: &mut ToolController, state: &mut AppState, xf: &ViewXform, points: &[egui::Pos2]) {
        let p = pointer(state, xf, points[0], egui::Modifiers::NONE);
        tools.begin_frame();
        tools.press(state, xf, &p, egui::PointerButton::Primary);
        for pos in &points[1..] {
            tools.begin_frame();
            let p = pointer(state, xf, *pos, egui::Modifiers::NONE);
            tools.drag(state, xf, &p);
        }
        let p = pointer(state, xf, *points.last().unwrap(), egui::Modifiers::NONE);
        tools.release(state, xf, &p);
        settle(tools, state);
    }

    /// Runs frames (`begin_frame` + `end_frame`) until the controller is idle; the number of frames.
    fn settle(tools: &mut ToolController, state: &mut AppState) -> u32 {
        let mut frames = 0;
        while tools.is_busy() && tools.finishing() && frames < 100_000 {
            tools.begin_frame();
            tools.end_frame(state, 0.0);
            frames += 1;
        }
        frames
    }

    /// No pacing: every position resolved and every sample painted in the frame it arrives in.
    fn unlimited() -> StrokePace {
        StrokePace {
            raycasts_per_frame: u32::MAX,
            finish_raycasts: u32::MAX,
            soft: f64::INFINITY,
            lag_target: f64::INFINITY,
            frame_cap: f64::INFINITY,
        }
    }

    #[test]
    fn stroke_in_3d_paints_and_switches_texture_on() {
        let mut state = state_with(SampleKind::Sphere);
        state.flags.shading = ShadingMode::Clay;
        state.colors.primary = [255, 0, 0, 255];
        let xf = xf_3d(&mut state);
        let before = composite_hash(&state, 0);
        let undo = state.doc().unwrap().history.undo_len();
        let mut tools = ToolController::new();
        drag_path(&mut tools, &mut state, &xf, &[at(&xf, 0.4, 0.45), at(&xf, 0.5, 0.5), at(&xf, 0.6, 0.55)]);
        assert!(!tools.is_busy());
        let doc = state.doc_mut().unwrap();
        doc.recomposite_all();
        assert_ne!(composite_hash(&state, 0), before);
        assert_eq!(state.doc().unwrap().history.undo_len(), undo + 1);
        assert_eq!(state.flags.shading, ShadingMode::Textured);
        // The Base layer was not touched (the stroke went to Paint 1).
        let set = &state.doc().unwrap().sets[0];
        let LayerKind::Raster(paint) = &set.layers[1].kind else { panic!() };
        assert!(paint.data.chunks(4).any(|p| p[0] == 255 && p[3] > 0));
    }

    #[test]
    fn stroke_in_2d_paints_and_eraser_erases() {
        let mut state = state_with(SampleKind::Sphere);
        state.colors.primary = [0, 0, 255, 255];
        let xf = xf_2d(&mut state);
        let mut tools = ToolController::new();
        drag_path(&mut tools, &mut state, &xf, &[at(&xf, 0.3, 0.5), at(&xf, 0.5, 0.5), at(&xf, 0.7, 0.5)]);
        let painted = |state: &AppState| {
            let LayerKind::Raster(paint) = &state.doc().unwrap().sets[0].layers[1].kind else { panic!() };
            paint.data.chunks(4).filter(|p| p[3] > 0).count()
        };
        let n = painted(&state);
        assert!(n > 100);
        state.tool = ToolKind::Eraser;
        drag_path(&mut tools, &mut state, &xf, &[at(&xf, 0.3, 0.5), at(&xf, 0.7, 0.5)]);
        assert!(painted(&state) < n / 4);
    }

    #[test]
    fn press_on_a_locked_layer_warns_and_does_not_paint() {
        let mut state = state_with(SampleKind::Sphere);
        let doc = state.doc_mut().unwrap();
        let base = doc.sets[0].layers[0].id;
        doc.set_active_layer(TextureSetId(0), Some(base)).unwrap();
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy());
        assert!(!state.doc().unwrap().stroke_active());
        assert_eq!(state.doc().unwrap().history.undo_len(), 0);
    }

    #[test]
    fn press_on_a_stamp_layer_creates_a_paint_layer() {
        let mut state = state_with(SampleKind::Sphere);
        let doc = state.doc_mut().unwrap();
        // No active layer: the press creates "Paint 2" above.
        doc.set_active_layer(TextureSetId(0), None).unwrap();
        let xf = xf_2d(&mut state);
        let mut tools = ToolController::new();
        drag_path(&mut tools, &mut state, &xf, &[at(&xf, 0.5, 0.5), at(&xf, 0.55, 0.5)]);
        let set = &state.doc().unwrap().sets[0];
        assert_eq!(set.layers.len(), 3);
        assert_eq!(set.active, Some(set.layers[2].id));
    }

    #[test]
    fn cancel_restores_the_layer() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(tools.is_busy() && state.doc().unwrap().stroke_active());
        tools.cancel(&mut state);
        assert!(!tools.is_busy() && !state.doc().unwrap().stroke_active());
        let LayerKind::Raster(paint) = &state.doc().unwrap().sets[0].layers[1].kind else { panic!() };
        assert!(paint.data.chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn finish_keeps_the_stroke_painted_so_far() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let undo = state.doc().unwrap().history.undo_len();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let p = pointer(&state, &xf, xf.rect.center() + egui::vec2(30.0, 0.0), egui::Modifiers::NONE);
        tools.drag(&mut state, &xf, &p);
        assert!(tools.is_busy() && state.doc().unwrap().stroke_active());
        // What Ctrl+S during the held stroke does (dispatch's interaction guard for Save / Export).
        tools.finish(&mut state);
        let doc = state.doc().unwrap();
        assert!(!tools.is_busy() && !doc.stroke_active());
        assert_eq!(doc.history.undo_len(), undo + 1, "the stroke is one entry");
        let LayerKind::Raster(paint) = &doc.sets[0].layers[1].kind else { panic!() };
        assert!(paint.data.chunks(4).any(|p| p[3] > 0), "the painted dabs stay");
    }

    #[test]
    fn long_drag_respects_the_raycast_budget_and_finishes_on_release() {
        let mut state = state_with(SampleKind::Sphere);
        state.tools.brush.size_px = 4.0;
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let a = at(&xf, 0.38, 0.5);
        let b = at(&xf, 0.62, 0.5);
        tools.begin_frame();
        let p = pointer(&state, &xf, a, egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        tools.begin_frame();
        let p = pointer(&state, &xf, b, egui::Modifiers::NONE);
        tools.drag(&mut state, &xf, &p);
        // The small brush needs more than 64 one-point steps for this distance: the budget stops the walk early.
        let st = tools.brush.stroke.unwrap();
        assert!(st.last.distance(b) > 1.0, "walk was not limited");
        tools.release(&mut state, &xf, &p);
        // Round 4 (FEATURE_BRUSH 5.10): the stroke may finish in the frames after the release.
        for _ in 0..10 {
            if !tools.is_busy() {
                break;
            }
            tools.begin_frame();
            tools.end_frame(&mut state, 0.0);
        }
        assert!(!state.doc().unwrap().stroke_active());
    }

    #[test]
    fn ring_follows_the_hover() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        tools.begin_frame();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        let ring = tools.brush_ring(&state).expect("ring over the model");
        assert!(ring.radius > 0.0);
        tools.begin_frame();
        assert!(tools.brush_ring(&state).is_none());
        let p = pointer(&state, &xf, xf.rect.min + egui::vec2(1.0, 1.0), egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        assert!(tools.brush_ring(&state).is_none());
    }

    // ----- round 4 (FEATURE_BRUSH 8.2) -----

    /// A1: the rows of the table in 4.3 (plan section 7: without the connected-surface rows), with `through` passed
    /// on; a 2D stroke never gets a field.
    #[test]
    fn a1_settings_to_tip() {
        let mut state = state_with(SampleKind::Sphere);
        let up = Vec3::new(0.0, 0.6, 0.8);
        let tip = |state: &mut AppState| BrushTool::press_tip(state, up);
        // Circular: the ball, SameSide, no offset (also with an offset set).
        state.tools.brush.offset = 0.5;
        let t = tip(&mut state).unwrap();
        assert!(t.is_ball() && t.reach == SurfaceReach::SameSide && t.offset == 0.0);
        // Sphere, Ball: All with the offset.
        state.tools.brush.kind = BrushType::Sphere;
        let t = tip(&mut state).unwrap();
        assert!(t.is_ball() && t.reach == SurfaceReach::All && t.offset == 0.5 && t.up_hint == up);
        // Sphere, Cube / Cylinder: the built-in field with align, rotation, offset, scale 1 and Use.
        for (shape, slot) in [(ShapeChoice::Cube, 0), (ShapeChoice::Cylinder, 1)] {
            state.tools.brush.shape = shape;
            state.tools.brush.through = true;
            state.tools.brush.turn_deg = 30.0;
            state.tools.brush.align = TipAlign::World;
            let t = tip(&mut state).unwrap();
            let field = t.field().expect("a field");
            assert!(Arc::ptr_eq(field, state.builtin_tips[slot].as_ref().unwrap()), "{shape:?} uses the built-in field");
            assert!(t.reach == SurfaceReach::All && t.through && t.align == TipAlign::World && t.scale == 1.0);
            assert!((t.rotation.dot(Quat::from_rotation_y(-30f32.to_radians()))).abs() > 0.9999);
            state.tools.brush.through = false;
            assert!(!tip(&mut state).unwrap().through);
        }
        // Sphere, File with a current shape (also while a newer load runs): the loaded field; a flat outline is
        // always Through.
        state.tools.brush.shape = ShapeChoice::File;
        let flat = Arc::new(TipField::outline_from_fn([33, 33], Vec2::splat(-1.0), 1.0 / 16.0, |q| q.length() - 0.8));
        assert!(flat.is_flat());
        state.brush_shape.current = Some(ReadyShape {
            path: "star.obj".into(),
            name: "star.obj".into(),
            field: Arc::clone(&flat),
            close: CloseMode::Refuse,
            closed_parts: false,
        });
        state.brush_shape.pending = Some(PendingShape {
            request: 2,
            path: "next.obj".into(),
            by_user: true,
            for_eraser: false,
            close: CloseMode::Refuse,
        });
        let t = tip(&mut state).unwrap();
        assert!(Arc::ptr_eq(t.field().unwrap(), &flat) && t.through);
        // Sphere, File, a load pending and no current: no stroke, an info toast.
        state.brush_shape.current = None;
        assert!(tip(&mut state).is_none());
        assert!(state.toasts.visible_labels().iter().any(|l| l.contains(STILL_LOADING_TEXT)));
        // Sphere, File, nothing at all: the ball with All and the offset.
        state.brush_shape.pending = None;
        let t = tip(&mut state).unwrap();
        assert!(t.is_ball() && t.reach == SurfaceReach::All && t.offset == 0.5);
        // A 2D stroke never gets a field: the same settings paint the round-3 disc in the 2D view.
        state.tools.brush = BrushSettings { kind: BrushType::Sphere, shape: ShapeChoice::Cube, ..BrushSettings::default() };
        let xf = xf_2d(&mut state);
        let mut tools = ToolController::new();
        drag_path(&mut tools, &mut state, &xf, &[at(&xf, 0.3, 0.5), at(&xf, 0.6, 0.5)]);
        let with_shape = paint_hash(&state);
        let mut plain = state_with(SampleKind::Sphere);
        let xf = xf_2d(&mut plain);
        drag_path(&mut ToolController::new(), &mut plain, &xf, &[at(&xf, 0.3, 0.5), at(&xf, 0.6, 0.5)]);
        assert_eq!(with_shape, paint_hash(&plain), "a 2D stroke ignores the type and the shape");
    }

    /// Texels with paint per island of the Cube sample (its six faces, grouped by face normal).
    fn island_counts(state: &AppState) -> [usize; 6] {
        let doc = state.doc().unwrap();
        let set = &doc.sets[0];
        let LayerKind::Raster(paint) = &set.layers[1].kind else { panic!("Paint 1") };
        let size = Vec2::new(set.width as f32, set.height as f32);
        let normals = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        let mut rects = [(Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)); 6];
        for t in 0..doc.mesh.tri_count() {
            let [a, b, c] = doc.mesh.tri_positions(t);
            let n = (b - a).cross(c - a).normalize();
            let face = normals.iter().position(|m| m.dot(n) > 0.9).expect("an axis face");
            for uv in doc.mesh.tri_uvs(t) {
                rects[face].0 = rects[face].0.min(uv * size);
                rects[face].1 = rects[face].1.max(uv * size);
            }
        }
        let mut counts = [0usize; 6];
        for (face, (lo, hi)) in rects.iter().enumerate() {
            for y in lo.y.floor() as u32..(hi.y.ceil() as u32).min(set.height) {
                for x in lo.x.floor() as u32..(hi.x.ceil() as u32).min(set.width) {
                    counts[face] += (paint.data[((y * set.width + x) * 4 + 3) as usize] > 0) as usize;
                }
            }
        }
        counts
    }

    /// One click at the centre of the view.
    fn click(tools: &mut ToolController, state: &mut AppState, xf: &ViewXform) {
        let p = pointer(state, xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.begin_frame();
        tools.press(state, xf, &p, egui::PointerButton::Primary);
        tools.release(state, xf, &p);
        settle(tools, state);
    }

    /// A2: Cube sample, Size 1024, hardness 1, a click at the centre of the front face: Circular leaves exactly one
    /// of the six islands unchanged (the back face), Sphere none.
    #[test]
    fn a2_types_through_the_tool() {
        for (kind, untouched) in [(BrushType::Circular, 1), (BrushType::Sphere, 0)] {
            let mut state = state_with(SampleKind::Cube);
            state.tools.brush.size_px = 1024.0;
            state.tools.brush.hardness = 1.0;
            state.tools.brush.kind = kind;
            let xf = xf_3d(&mut state);
            let r = 512.0 * state.doc().unwrap().texel_world_size(TextureSetId(0));
            assert!(r > 1.84, "the ball holds the whole cube (radius {r})");
            let mut tools = ToolController::new();
            click(&mut tools, &mut state, &xf);
            let counts = island_counts(&state);
            assert_eq!(counts.iter().filter(|&&n| n == 0).count(), untouched, "{kind:?}: {counts:?}");
            if kind == BrushType::Circular {
                assert_eq!(counts[5], 0, "the back face (-Z) is not painted: {counts:?}");
            }
            assert_eq!(state.doc().unwrap().history.undo_len(), 1);
        }
    }

    /// The cube sample with the given settings, a stroke along `path` (normalized view points) with the pacing and
    /// clock given; returns the painted hash and the state.
    fn stroke_with(
        settings: BrushSettings,
        path: &[[f32; 2]],
        pace: StrokePace,
        clock: StrokeClock,
    ) -> (u64, AppState, ToolController) {
        let mut state = state_with(SampleKind::Sphere);
        state.tools.brush = settings;
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        tools.brush.pace = pace;
        tools.brush.clock = clock;
        let points: Vec<egui::Pos2> = path.iter().map(|p| at(&xf, p[0], p[1])).collect();
        tools.begin_frame();
        let p = pointer(&state, &xf, points[0], egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        tools.end_frame(&mut state, 0.0);
        for pos in &points[1..] {
            tools.begin_frame();
            let p = pointer(&state, &xf, *pos, egui::Modifiers::NONE);
            tools.drag(&mut state, &xf, &p);
            tools.end_frame(&mut state, 0.0);
        }
        tools.begin_frame();
        let p = pointer(&state, &xf, *points.last().unwrap(), egui::Modifiers::NONE);
        tools.release(&mut state, &xf, &p);
        tools.end_frame(&mut state, 0.0);
        settle(&mut tools, &mut state);
        assert!(!tools.is_busy() && !state.doc().unwrap().stroke_active());
        (paint_hash(&state), state, tools)
    }

    fn small_brush() -> BrushSettings {
        BrushSettings { size_px: 6.0, ..BrushSettings::default() }
    }

    /// A path with a right-angle corner over the sphere sample.
    const CORNER: [[f32; 2]; 3] = [[0.40, 0.40], [0.60, 0.40], [0.60, 0.60]];

    /// A4: pacing and camera moves do not change pixels.
    #[test]
    fn a4_pacing_and_camera_moves_do_not_change_pixels() {
        let (reference, ..) = stroke_with(small_brush(), &CORNER, unlimited(), StrokeClock::default());
        // One raycast per frame.
        let one = StrokePace { raycasts_per_frame: 1, finish_raycasts: 1, ..unlimited() };
        let (hash, _, tools) = stroke_with(small_brush(), &CORNER, one, StrokeClock::default());
        assert_eq!(hash, reference, "a raycast budget of 1 per frame");
        assert!(tools.brush.stats.samples > 20);
        // A clock that makes every frame stop after one sample.
        let slow = StrokePace { soft: 0.0, frame_cap: 0.0, ..StrokePace::default() };
        let (hash, ..) = stroke_with(small_brush(), &CORNER, slow, StrokeClock::Fixed { per_sample: 1.0, now: 0.0 });
        assert_eq!(hash, reference, "one sample per frame");

        // A wheel zoom (and an orbit) between two drags while positions are unresolved.
        for camera_move in [0, 1] {
            let run = |pace: StrokePace| -> u64 {
                let mut state = state_with(SampleKind::Sphere);
                state.tools.brush = small_brush();
                let xf1 = xf_3d(&mut state);
                let mut tools = ToolController::new();
                tools.brush.pace = pace;
                tools.begin_frame();
                let p = pointer(&state, &xf1, at(&xf1, 0.42, 0.45), egui::Modifiers::NONE);
                tools.press(&mut state, &xf1, &p, egui::PointerButton::Primary);
                tools.begin_frame();
                let p = pointer(&state, &xf1, at(&xf1, 0.55, 0.45), egui::Modifiers::NONE);
                tools.drag(&mut state, &xf1, &p);
                tools.end_frame(&mut state, 0.0);
                // The camera moves.
                {
                    let session = state.session.as_mut().unwrap();
                    let radius = session.doc.mesh.bounds.diagonal() * 0.5;
                    if camera_move == 0 {
                        session.camera.zoom(2.0, radius);
                    } else {
                        session.camera.orbit(egui::vec2(25.0, 10.0));
                    }
                }
                let xf2 = xf_3d(&mut state);
                tools.begin_frame();
                let p = pointer(&state, &xf2, at(&xf2, 0.5, 0.58), egui::Modifiers::NONE);
                tools.drag(&mut state, &xf2, &p);
                tools.end_frame(&mut state, 0.0);
                tools.begin_frame();
                tools.release(&mut state, &xf2, &p);
                tools.end_frame(&mut state, 0.0);
                settle(&mut tools, &mut state);
                assert!(!state.doc().unwrap().stroke_active());
                paint_hash(&state)
            };
            let one = StrokePace { raycasts_per_frame: 1, finish_raycasts: 1, ..unlimited() };
            assert_eq!(run(one), run(unlimited()), "camera move {camera_move} while positions wait");
        }
    }

    /// Round 5 B-3 (B1's new test): a clock that advances per dab and a time rule that stops after any time paints one
    /// dab per frame, splitting the samples, and gives the hash of the unpaced stroke. The fixed per-sample clock never
    /// splits a sample.
    #[test]
    fn one_dab_per_frame_gives_the_unpaced_hash() {
        // A small spacing: several dabs per walked sample.
        let settings = BrushSettings { size_px: 24.0, spacing: 0.03, ..BrushSettings::default() };
        let (reference, _, tools) = stroke_with(settings.clone(), &CORNER, unlimited(), StrokeClock::default());
        assert_eq!(tools.brush.stats.splits, 0, "no time rule, no split");
        let slow = StrokePace { soft: 0.0, frame_cap: 0.0, ..StrokePace::default() };
        let (hash, _, tools) = stroke_with(settings.clone(), &CORNER, slow, StrokeClock::PerDab { per_dab: 1.0, now: 0.0 });
        assert_eq!(hash, reference, "one dab per frame");
        let stats = &tools.brush.stats;
        assert_eq!(stats.most_dabs_in_frame, 1, "{stats:?}");
        assert!(stats.splits > 0 && stats.splits >= stats.samples / 2, "the samples were split: {stats:?}");
        assert!(stats.longest_frame <= 1.0 + 1e-9, "a frame is one dab of the per-dab clock: {stats:?}");
        // A frame of three dabs.
        let three = StrokePace { soft: 2.5, frame_cap: 2.5, ..StrokePace::default() };
        let (hash, _, tools) = stroke_with(settings.clone(), &CORNER, three, StrokeClock::PerDab { per_dab: 1.0, now: 0.0 });
        assert_eq!(hash, reference, "three dabs per frame");
        assert_eq!(tools.brush.stats.most_dabs_in_frame, 3);
        // Check-and-fix: the wall clock with a zero budget also paints one dab per frame (the rule is met after any
        // dab), with the same pixels.
        let (hash, _, tools) = stroke_with(settings.clone(), &CORNER, slow, StrokeClock::default());
        assert_eq!(hash, reference, "wall clock, zero budget");
        assert_eq!(tools.brush.stats.most_dabs_in_frame, 1, "{:?}", tools.brush.stats);
        assert!(tools.brush.stats.splits > 0, "{:?}", tools.brush.stats);
        // The per-sample clock with the same rule: one sample per frame, never split.
        let (hash, _, tools) = stroke_with(settings, &CORNER, slow, StrokeClock::Fixed { per_sample: 1.0, now: 0.0 });
        assert_eq!(hash, reference);
        assert_eq!(tools.brush.stats.splits, 0);
        assert!(tools.brush.stats.most_dabs_in_frame > 1, "a whole sample per frame");
    }

    /// A stroke that is split mid-sample is completed by `finish` (Save), discarded by `discard` (Undo / Esc) and
    /// completed by a new press, as a finishing stroke is (A19); the pixels are the unpaced ones.
    #[test]
    fn a_split_sample_is_completed_or_discarded_like_the_rest_of_the_stroke() {
        let settings = BrushSettings { size_px: 24.0, spacing: 0.03, ..BrushSettings::default() };
        let start = |pace: StrokePace, clock: StrokeClock| {
            let mut state = state_with(SampleKind::Sphere);
            state.tools.brush = settings.clone();
            let xf = xf_3d(&mut state);
            let mut tools = ToolController::new();
            tools.brush.pace = pace;
            tools.brush.clock = clock;
            let points: Vec<egui::Pos2> = CORNER.iter().map(|p| at(&xf, p[0], p[1])).collect();
            tools.begin_frame();
            let p = pointer(&state, &xf, points[0], egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            tools.end_frame(&mut state, 0.0);
            for pos in &points[1..] {
                tools.begin_frame();
                let p = pointer(&state, &xf, *pos, egui::Modifiers::NONE);
                tools.drag(&mut state, &xf, &p);
                tools.end_frame(&mut state, 0.0);
            }
            (state, tools, xf)
        };
        // The reference: the same held stroke, finished without a time rule.
        let (mut state, mut tools, _) = start(unlimited(), StrokeClock::default());
        tools.finish(&mut state);
        let reference = paint_hash(&state);
        let slow = StrokePace { soft: 0.0, frame_cap: 0.0, ..StrokePace::default() };
        let per_dab = || StrokeClock::PerDab { per_dab: 1.0, now: 0.0 };
        // finish (Save while the button is held): the split sample and the rest are painted, one entry.
        let (mut state, mut tools, _) = start(slow, per_dab());
        assert!(tools.brush.partial, "a sample is split");
        tools.finish(&mut state);
        assert!(!tools.is_busy() && !state.doc().unwrap().stroke_active());
        assert_eq!(state.doc().unwrap().history.undo_len(), 1);
        assert_eq!(paint_hash(&state), reference);
        // discard (Undo / Esc): nothing stays, no entry, the next stroke starts clean.
        let (mut state, mut tools, xf) = start(slow, per_dab());
        assert!(tools.brush.partial);
        tools.discard(&mut state);
        assert!(!tools.brush.partial && !state.doc().unwrap().stroke_active());
        assert_eq!(state.doc().unwrap().history.undo_len(), 0);
        let LayerKind::Raster(paint) = &state.doc().unwrap().sets[0].layers[1].kind else { panic!() };
        assert!(paint.data.chunks(4).all(|p| p[3] == 0), "the discarded stroke left paint");
        drag_path(&mut tools, &mut state, &xf, &[at(&xf, 0.45, 0.45), at(&xf, 0.5, 0.45)]);
        assert_eq!(state.doc().unwrap().history.undo_len(), 1);
        // A release, then a new press while the split stroke still finishes: the first stroke is completed.
        let (mut state, mut tools, xf) = start(slow, per_dab());
        tools.begin_frame();
        let last = at(&xf, CORNER[2][0], CORNER[2][1]);
        let p = pointer(&state, &xf, last, egui::Modifiers::NONE);
        tools.release(&mut state, &xf, &p);
        assert!(tools.finishing() && tools.brush.partial);
        let p = pointer(&state, &xf, at(&xf, 0.45, 0.6), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert_eq!(state.doc().unwrap().history.undo_len(), 1);
        tools.discard(&mut state);
        assert_eq!(paint_hash(&state), reference);
    }

    /// Frame by frame: `per_frame` new pointer positions one point apart arrive in each of `frames` frames; the
    /// painting time of every frame and the backlog estimates are recorded.
    struct Paced {
        state: AppState,
        tools: ToolController,
        xf: ViewXform,
        pos: egui::Pos2,
        frame_times: Vec<f64>,
        busy_frames_after_release: u32,
        waiting_at_release: usize,
    }

    impl Paced {
        fn new(per_sample: f64) -> Paced {
            let mut state = state_with(SampleKind::Sphere);
            state.tools.brush = BrushSettings { size_px: 1.0, ..BrushSettings::default() };
            let xf = xf_3d(&mut state);
            let mut tools = ToolController::new();
            tools.brush.clock = StrokeClock::Fixed { per_sample, now: 0.0 };
            // The paths of the tests go at most 160 points to the right, which stays on the sphere (0.40 to 0.60).
            let pos = at(&xf, 0.4, 0.5);
            tools.begin_frame();
            let p = pointer(&state, &xf, pos, egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            assert!(tools.is_busy() && state.doc().unwrap().stroke_active(), "the press opened a stroke");
            tools.end_frame(&mut state, 0.0);
            let mut paced =
                Paced { state, tools, xf, pos, frame_times: Vec::new(), busy_frames_after_release: 0, waiting_at_release: 0 };
            paced.frame_times.push(paced.tools.brush.frame_spent);
            paced
        }

        /// One frame with `n` positions one point apart (one walked sample each).
        fn frame(&mut self, n: usize) -> Option<f64> {
            self.tools.begin_frame();
            for _ in 0..n {
                self.pos += egui::vec2(1.0, 0.0);
                let p = pointer(&self.state, &self.xf, self.pos, egui::Modifiers::NONE);
                self.tools.drag(&mut self.state, &self.xf, &p);
            }
            let wake = self.tools.end_frame(&mut self.state, 0.0);
            self.frame_times.push(self.tools.brush.frame_spent);
            wake
        }

        fn release(&mut self) {
            self.tools.begin_frame();
            let p = pointer(&self.state, &self.xf, self.pos, egui::Modifiers::NONE);
            self.tools.release(&mut self.state, &self.xf, &p);
            self.waiting_at_release = self.tools.brush.pending.len() + self.tools.brush.unresolved.len();
            let mut wake = self.tools.end_frame(&mut self.state, 0.0);
            self.frame_times.push(self.tools.brush.frame_spent);
            while self.tools.is_busy() {
                assert_eq!(wake, Some(0.0), "a finishing stroke asks for its next frame");
                self.busy_frames_after_release += 1;
                assert!(self.busy_frames_after_release < 10_000);
                self.tools.begin_frame();
                let before = self.tools.brush.stats.samples;
                wake = self.tools.end_frame(&mut self.state, 0.0);
                if self.tools.brush.stroke.is_some() {
                    assert!(self.tools.brush.stats.samples > before, "at least one sample per frame");
                }
                self.frame_times.push(self.tools.brush.frame_spent);
            }
        }
    }

    /// The hash of the same path (`frames` frames of `per_frame` one-point steps from the same start) painted without
    /// a time rule.
    fn unpaced_hash(frames: usize, per_frame: usize) -> u64 {
        let mut paced = Paced::new(0.0);
        paced.tools.brush.pace = unlimited();
        for _ in 0..frames {
            paced.frame(per_frame);
        }
        paced.release();
        assert_eq!(paced.busy_frames_after_release, 0);
        paint_hash(&paced.state)
    }

    /// A5: the time rule, with the replaceable clock.
    #[test]
    fn a5_the_time_rule() {
        // (i) 1 ms per sample: the stroke is closed in the frame of the release, with one history entry.
        let mut paced = Paced::new(0.001);
        for _ in 0..10 {
            paced.frame(2);
        }
        paced.release();
        assert_eq!(paced.busy_frames_after_release, 0);
        assert_eq!(paced.state.doc().unwrap().history.undo_len(), 1);

        // (ii) 5 ms per sample, 2 per frame: no frame over 10 ms plus one sample; backlog never over 250 ms.
        let mut paced = Paced::new(0.005);
        for _ in 0..40 {
            paced.frame(2);
        }
        let backlog = paced.tools.brush.stats.max_backlog;
        assert!(backlog <= 0.250, "backlog {backlog}");
        assert!(paced.frame_times.iter().all(|&t| t <= 0.010 + 0.005 + 1e-9), "{:?}", paced.frame_times);
        paced.release();

        // (iii) 20 ms per sample, 4 per frame for 40 frames: frames of at most 100 ms plus one sample, the release
        // frame included; the controller stays busy and asks for frames until the stroke is closed, which happens
        // within (samples waiting x 20 ms / 100 ms) + 2 frames; the pixels equal the unpaced stroke's.
        let mut paced = Paced::new(0.020);
        for _ in 0..40 {
            assert_eq!(paced.frame(4), Some(0.0), "work is queued");
        }
        paced.release();
        assert!(paced.frame_times.iter().all(|&t| t <= 0.100 + 0.020 + 1e-9), "{:?}", paced.frame_times);
        let longest = paced.tools.brush.stats.longest_frame;
        assert!((0.100 - 1e-9..=0.100 + 0.020 + 1e-9).contains(&longest), "longest frame {longest}");
        let limit = (paced.waiting_at_release as f64 * 0.020 / 0.100).ceil() as u32 + 2;
        assert!(paced.waiting_at_release > 0, "the drag outran the machine");
        assert!(
            paced.busy_frames_after_release <= limit,
            "{} frames after the release, limit {limit} ({} waiting)",
            paced.busy_frames_after_release,
            paced.waiting_at_release
        );
        assert_eq!(paced.state.doc().unwrap().history.undo_len(), 1);
        assert_eq!(paint_hash(&paced.state), unpaced_hash(40, 4));
        // (iv) is asserted inside `Paced::release` (at least one sample in every frame with waiting work).
    }

    /// A6: Ctrl+wheel sizes, Shift+wheel turns a shape.
    #[test]
    fn a6_wheel() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let wheel = |state: &AppState, mods| pointer(state, &xf, xf.rect.center(), mods);
        let p = wheel(&state, egui::Modifiers::CTRL);
        assert!(tools.scroll(&mut state, &xf, &p, 2.0));
        assert!((state.tools.brush.size_px - 32.0 * 1.21).abs() < 1e-3);
        assert!(tools.scroll(&mut state, &xf, &p, 200.0));
        assert_eq!(state.tools.brush.size_px, 2048.0);
        assert!(tools.scroll(&mut state, &xf, &p, -500.0));
        assert_eq!(state.tools.brush.size_px, 1.0);
        // Shift+wheel: Circular and Sphere with the Ball leave it to the view.
        let p = wheel(&state, egui::Modifiers::SHIFT);
        assert!(!tools.scroll(&mut state, &xf, &p, 1.0));
        state.tools.brush.kind = BrushType::Sphere;
        assert!(!tools.scroll(&mut state, &xf, &p, 1.0));
        state.tools.brush.shape = ShapeChoice::Cube;
        assert!(tools.scroll(&mut state, &xf, &p, 1.0));
        assert_eq!(state.tools.brush.turn_deg, 5.0);
        state.tools.brush.turn_deg = 178.0;
        assert!(tools.scroll(&mut state, &xf, &p, 1.0));
        assert_eq!(state.tools.brush.turn_deg, -177.0);
        // The eraser has its own settings.
        state.tool = ToolKind::Eraser;
        let p = wheel(&state, egui::Modifiers::CTRL);
        assert!(tools.scroll(&mut state, &xf, &p, 1.0));
        assert!((state.tools.eraser.size_px - 35.2).abs() < 1e-3);
        assert_eq!(state.tools.brush.size_px, 1.0);
        // A plain wheel is never the brush's.
        let p = wheel(&state, egui::Modifiers::NONE);
        assert!(!tools.scroll(&mut state, &xf, &p, 1.0));
    }

    /// A7: changing the type and the angles during a drag gives the same pixels as not changing them.
    #[test]
    fn a7_a_stroke_uses_the_tip_it_started_with() {
        let start = BrushSettings { kind: BrushType::Sphere, shape: ShapeChoice::Cube, size_px: 40.0, ..BrushSettings::default() };
        let run = |change: bool| -> u64 {
            let mut state = state_with(SampleKind::Sphere);
            state.tools.brush = start.clone();
            let xf = xf_3d(&mut state);
            let mut tools = ToolController::new();
            let path = [at(&xf, 0.42, 0.45), at(&xf, 0.5, 0.5), at(&xf, 0.58, 0.55)];
            tools.begin_frame();
            let p = pointer(&state, &xf, path[0], egui::Modifiers::NONE);
            tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
            for pos in &path[1..] {
                if change {
                    let b = &mut state.tools.brush;
                    (b.kind, b.shape, b.turn_deg, b.tilt_deg, b.offset) = (BrushType::Circular, ShapeChoice::Cylinder, 45.0, 30.0, 0.7);
                    b.size_px = 90.0;
                }
                tools.begin_frame();
                let p = pointer(&state, &xf, *pos, egui::Modifiers::NONE);
                tools.drag(&mut state, &xf, &p);
            }
            let p = pointer(&state, &xf, path[2], egui::Modifiers::NONE);
            tools.release(&mut state, &xf, &p);
            settle(&mut tools, &mut state);
            paint_hash(&state)
        };
        assert_eq!(run(true), run(false));
    }

    /// A8: the ring keeps the stroke's radius, carries the side normal for Circular only, and is lifted by Offset.
    #[test]
    fn a8_ring() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        let Surface::Model(hit) = p.surface else { panic!("hit") };
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        let ring = tools.brush_ring(&state).unwrap();
        assert_eq!(ring.side_normal, Some(hit.surface_normal()), "Circular filters by side");
        state.tools.brush.kind = BrushType::Sphere;
        let ring = tools.brush_ring(&state).unwrap();
        assert_eq!(ring.side_normal, None, "Sphere has no side filter");
        state.tools.brush.offset = 0.5;
        let ring = tools.brush_ring(&state).unwrap();
        let expected = hit.position + 0.5 * ring.radius * hit.normal;
        assert!((ring.center - expected).length() < 1e-5, "{:?} vs {expected:?}", ring.center);
        // During a stroke: the stroke's radius, whatever Size says now.
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let stroke_radius = tools.brush_ring(&state).unwrap().radius;
        state.tools.brush.size_px *= 3.0;
        assert_eq!(tools.brush_ring(&state).unwrap().radius, stroke_radius);
        tools.release(&mut state, &xf, &p);
        settle(&mut tools, &mut state);
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        assert!((tools.brush_ring(&state).unwrap().radius - 3.0 * stroke_radius).abs() < 1e-4 * stroke_radius);
    }

    /// Gate 1 (FEATURE_BRUSH 5.11): the inner hardness ring belongs to the ball only; a shape's ring has hardness 1.
    #[test]
    fn only_the_ball_shows_the_inner_hardness_ring() {
        let mut state = state_with(SampleKind::Sphere);
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        state.tools.brush.hardness = 0.5;
        tools.begin_frame();
        tools.hover(&mut state, &xf, &p);
        assert_eq!(tools.brush_ring(&state).unwrap().hardness, 0.5, "Circular");
        state.tools.brush.kind = BrushType::Sphere;
        assert_eq!(tools.brush_ring(&state).unwrap().hardness, 0.5, "Sphere with the Ball");
        state.tools.brush.shape = ShapeChoice::Cube;
        assert_eq!(tools.brush_ring(&state).unwrap().hardness, 1.0, "Cube");
        state.tools.brush.shape = ShapeChoice::File;
        assert_eq!(tools.brush_ring(&state).unwrap().hardness, 0.5, "File without a shape paints the ball");
        state.brush_shape.current = Some(ready_cube());
        assert_eq!(tools.brush_ring(&state).unwrap().hardness, 1.0, "File with a shape");
        // During a stroke the stroke's own tip decides.
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        state.tools.brush.shape = ShapeChoice::Ball;
        assert_eq!(tools.brush_ring(&state).unwrap().hardness, 1.0, "the open stroke has the file shape");
        tools.release(&mut state, &xf, &p);
        settle(&mut tools, &mut state);
    }

    fn ready_cube() -> ReadyShape {
        ReadyShape {
            path: "cube.obj".into(),
            name: "cube.obj".into(),
            field: Arc::new(TipField::cube()),
            close: CloseMode::Refuse,
            closed_parts: false,
        }
    }

    fn pending_load() -> PendingShape {
        PendingShape { request: 7, path: "slow.obj".into(), by_user: true, for_eraser: false, close: CloseMode::Refuse }
    }

    /// A9: a missing or loading file shape.
    #[test]
    fn a9_missing_and_loading_shape() {
        let file = BrushSettings { kind: BrushType::Sphere, shape: ShapeChoice::File, size_px: 40.0, ..BrushSettings::default() };
        // Identical toasts coalesce into one with a count, so "one toast" is one label that is exactly the text.
        let count = |state: &AppState, text: &str| {
            let labels = state.toasts.visible_labels();
            let mentions = labels.iter().filter(|l| l.contains(text)).count();
            let exact = labels.iter().filter(|l| l.as_str() == text).count();
            if mentions == exact { exact } else { usize::MAX }
        };
        // A load pending, no current shape: no stroke, one info toast.
        let mut state = state_with(SampleKind::Sphere);
        state.tools.brush = file.clone();
        state.brush_shape.pending = Some(pending_load());
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        assert!(!tools.is_busy() && !state.doc().unwrap().stroke_active());
        assert_eq!(count(&state, STILL_LOADING_TEXT), 1);
        assert_eq!(state.doc().unwrap().history.undo_len(), 0);

        // A current shape and a second load pending: the stroke uses the current shape (the same pixels as the Cube).
        state.brush_shape.current = Some(ready_cube());
        click(&mut tools, &mut state, &xf);
        let with_file = paint_hash(&state);
        let mut cube = state_with(SampleKind::Sphere);
        cube.tools.brush = BrushSettings { shape: ShapeChoice::Cube, ..file.clone() };
        let xf_c = xf_3d(&mut cube);
        click(&mut ToolController::new(), &mut cube, &xf_c);
        assert_eq!(with_file, paint_hash(&cube));

        // Nothing loaded and nothing pending: paints as the ball; the toast at the first press only.
        let mut state = state_with(SampleKind::Sphere);
        state.tools.brush = file.clone();
        let mut tools = ToolController::new();
        click(&mut tools, &mut state, &xf);
        assert_eq!(count(&state, NO_SHAPE_TEXT), 1);
        let mut ball = state_with(SampleKind::Sphere);
        ball.tools.brush = BrushSettings { shape: ShapeChoice::Ball, ..file.clone() };
        let xf_b = xf_3d(&mut ball);
        click(&mut ToolController::new(), &mut ball, &xf_b);
        assert_eq!(paint_hash(&state), paint_hash(&ball));
        click(&mut tools, &mut state, &xf);
        assert_eq!(count(&state, NO_SHAPE_TEXT), 1, "not at the second press");
        assert_eq!(state.doc().unwrap().history.undo_len(), 2);
    }

    /// A17: a waiting stroke gets its frames, also without a drag call.
    #[test]
    fn a17_a_waiting_stroke_gets_its_frames() {
        let mut paced = Paced::new(0.020);
        assert_eq!(paced.frame(12), Some(0.0), "queued work asks for the next frame");
        // A still pointer: no drag call, the stroke advances through end_frame alone.
        let mut frames = 0;
        loop {
            let before = paced.tools.brush.stats.samples;
            paced.tools.begin_frame();
            let wake = paced.tools.end_frame(&mut paced.state, 0.0);
            assert!(paced.tools.brush.stats.samples > before, "end_frame painted");
            frames += 1;
            if wake.is_none() {
                break;
            }
            assert!(frames < 100);
        }
        assert!(paced.tools.brush.queues_empty());
        assert!(paced.tools.is_busy() && paced.state.doc().unwrap().stroke_active(), "the held stroke stays open");
        paced.tools.begin_frame();
        assert_eq!(paced.tools.end_frame(&mut paced.state, 0.0), None, "nothing waits");
    }

    /// The Cube sample, Sphere with the Cube shape, hovering at the centre of the front face.
    fn hovering_cube(through: bool) -> (AppState, ToolController, ViewXform) {
        let mut state = state_with(SampleKind::Cube);
        state.tools.brush = BrushSettings { kind: BrushType::Sphere, shape: ShapeChoice::Cube, through, size_px: 200.0, ..BrushSettings::default() };
        let xf = xf_3d(&mut state);
        let mut tools = ToolController::new();
        tools.begin_frame();
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        (state, tools, xf)
    }

    /// Half sides (x, y) of the box of world segments, which lie in a plane z = const.
    fn half_sides(segs: &[[Vec3; 2]]) -> (f32, f32) {
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for p in segs.iter().flatten() {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        (0.5 * (hi.x - lo.x), 0.5 * (hi.y - lo.y))
    }

    /// A18: the shape cursor exists and has the cube's size.
    #[test]
    fn a18_the_shape_cursor_exists() {
        for through in [false, true] {
            let (state, tools, xf) = hovering_cube(through);
            let k = tools.brush_ring(&state).unwrap().radius;
            let lines = tools.brush.cursor_segments(&state, &xf);
            assert!(!lines.footprint.is_empty() && !lines.wires.is_empty(), "through {through}");
            let world = tools.brush.cursor_world(&state, &xf);
            let (hx, hy) = half_sides(&world.footprint);
            let want = TipField::CUBE_HALF_SIDE * k;
            for h in [hx, hy] {
                assert!((h - want).abs() <= 0.02 * want, "through {through}: half side {h}, expected {want}");
            }
            assert_eq!(state.tool_probe.cursor_footprint, lines.footprint.len());
            assert_eq!(state.tool_probe.cursor_wires, lines.wires.len());
        }
        // Circular: no shape lines.
        let (mut state, mut tools, xf) = hovering_cube(false);
        state.tools.brush.kind = BrushType::Circular;
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        assert_eq!(tools.brush.cursor_segments(&state, &xf), CursorLines::default());
        assert_eq!((state.tool_probe.cursor_footprint, state.tool_probe.cursor_wires), (0, 0));
        // During a stroke the lines follow `stroke_tip_pose()`: the wires sit at the last dab's centre.
        let (mut state, mut tools, xf) = hovering_cube(false);
        state.tools.brush.offset = 0.5;
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.press(&mut state, &xf, &p, egui::PointerButton::Primary);
        let pose = state.doc().unwrap().stroke_tip_pose().expect("a dab");
        state.tools.brush.offset = -0.5; // a change of the settings does not move the stroke's cursor
        let world = tools.brush.cursor_world(&state, &xf);
        let centre = world.wires.iter().flatten().fold(Vec3::ZERO, |a, p| a + *p) / (2 * world.wires.len()) as f32;
        assert!((centre - pose.center).length() < 0.05 * pose.unit, "wires at {centre:?}, pose at {:?}", pose.center);
        tools.release(&mut state, &xf, &p);
        settle(&mut tools, &mut state);
    }

    /// The A5 (iii) situation: a stroke that is still finishing after the release. Returns the run and the hash of
    /// the same stroke painted without a time rule.
    fn finishing_stroke() -> (Paced, u64) {
        let mut paced = Paced::new(0.020);
        for _ in 0..20 {
            paced.frame(4);
        }
        paced.tools.begin_frame();
        let p = pointer(&paced.state, &paced.xf, paced.pos, egui::Modifiers::NONE);
        paced.tools.release(&mut paced.state, &paced.xf, &p);
        assert!(paced.tools.is_busy() && paced.tools.finishing(), "the stroke still finishes");
        (paced, unpaced_hash(20, 4))
    }

    /// A19: a stroke that finishes after the release is never lost by accident. The calls are the ones dispatch makes
    /// (`dispatch.rs`: a tool change and every guarded command `tools.cancel`; Undo and Esc `tools.discard`; Save
    /// `tools.finish`); the view's press calls `tools.press`.
    #[test]
    fn a19_a_finishing_stroke_is_never_lost_by_accident() {
        // (a) another tool: completed, one history entry, the unpaced pixels.
        let (mut paced, unpaced) = finishing_stroke();
        paced.tools.cancel(&mut paced.state);
        paced.state.tool = ToolKind::Eyedropper;
        assert!(!paced.tools.is_busy());
        assert_eq!(paced.state.doc().unwrap().history.undo_len(), 1);
        assert_eq!(paint_hash(&paced.state), unpaced);

        // (b) Undo and (c) Esc: discarded, no entry, the pre-stroke pixels (the fresh paint layer is empty; its
        // buffer may be allocated by the stroke, so the test counts painted texels instead of hashing).
        for _ in ["undo", "esc"] {
            let (mut paced, _) = finishing_stroke();
            paced.tools.discard(&mut paced.state);
            assert!(!paced.tools.is_busy() && !paced.state.doc().unwrap().stroke_active());
            assert_eq!(paced.state.doc().unwrap().history.undo_len(), 0);
            let LayerKind::Raster(paint) = &paced.state.doc().unwrap().sets[0].layers[1].kind else { panic!() };
            assert!(paint.data.chunks(4).all(|p| p[3] == 0), "the stroke left paint");
        }

        // (d) a press in the view: the first stroke is one entry with the unpaced pixels, a second stroke is open.
        let (mut paced, unpaced) = finishing_stroke();
        let p = pointer(&paced.state, &paced.xf, at(&paced.xf, 0.5, 0.6), egui::Modifiers::NONE);
        paced.tools.press(&mut paced.state, &paced.xf, &p, egui::PointerButton::Primary);
        assert_eq!(paced.state.doc().unwrap().history.undo_len(), 1);
        assert!(paced.tools.is_busy() && !paced.tools.finishing() && paced.state.doc().unwrap().stroke_active());
        paced.tools.discard(&mut paced.state);
        assert_eq!(paint_hash(&paced.state), unpaced);

        // (e) Save while the button is held with unresolved positions queued: the stroke up to the last position.
        let mut paced = Paced::new(0.020);
        paced.tools.brush.pace.raycasts_per_frame = 2;
        for _ in 0..20 {
            paced.frame(4);
        }
        assert!(!paced.tools.brush.unresolved.is_empty(), "positions wait");
        paced.tools.finish(&mut paced.state);
        assert!(!paced.tools.is_busy());
        assert_eq!(paced.state.doc().unwrap().history.undo_len(), 1);
        assert_eq!(paint_hash(&paced.state), unpaced_hash(20, 4), "released at the last pointer position");

        // (f) release and drag calls after the release add no sample.
        let (mut paced, unpaced) = finishing_stroke();
        let waiting = paced.tools.brush.pending.len() + paced.tools.brush.unresolved.len();
        let samples = paced.tools.brush.stats.samples;
        let far = pointer(&paced.state, &paced.xf, paced.pos + egui::vec2(40.0, 5.0), egui::Modifiers::NONE);
        paced.tools.brush.drag(&mut paced.state, &paced.xf, &far);
        paced.tools.brush.release(&mut paced.state, &paced.xf, &far);
        paced.tools.release(&mut paced.state, &paced.xf, &far);
        assert_eq!(paced.tools.brush.pending.len() + paced.tools.brush.unresolved.len(), waiting);
        assert_eq!(paced.tools.brush.stats.samples, samples);
        settle(&mut paced.tools, &mut paced.state);
        assert_eq!(paint_hash(&paced.state), unpaced);
    }

    /// FEATURE_BRUSH 6.6: the status bar names the type over the model and says when the stroke trails.
    #[test]
    fn hints_name_the_type_and_the_trail() {
        let (mut state, mut tools, xf) = hovering_cube(false);
        assert_eq!(state.status_hint, "Shape brush: Cube");
        state.tools.brush.kind = BrushType::Circular;
        let p = pointer(&state, &xf, xf.rect.center(), egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &p);
        assert_eq!(state.status_hint, HINT_CIRCULAR);
        let miss = pointer(&state, &xf, xf.rect.min + egui::vec2(1.0, 1.0), egui::Modifiers::NONE);
        tools.hover(&mut state, &xf, &miss);
        assert!(state.status_hint.is_empty());
        // Another feature's hint is left alone.
        state.status_hint = "This is a 3D stamp - edit it in the 3D view".to_owned();
        tools.hover(&mut state, &xf, &p);
        assert_eq!(state.status_hint, "This is a 3D stamp - edit it in the 3D view");

        let (mut paced, _) = finishing_stroke();
        assert_eq!(paced.state.status_hint, HINT_FINISHING);
        settle(&mut paced.tools, &mut paced.state);
        assert!(paced.state.status_hint.is_empty());
        let mut paced = Paced::new(0.020);
        let mut catching_up = false;
        for _ in 0..30 {
            paced.frame(4);
            catching_up |= paced.state.status_hint == HINT_CATCHING_UP;
        }
        assert!(catching_up, "the trail over 250 ms is named in the status bar");
    }
}
