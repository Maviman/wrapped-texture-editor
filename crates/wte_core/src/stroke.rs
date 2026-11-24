// Owner: core-paint (C3). Brush strokes on a document (ARCHITECTURE.md 5.19, algorithm 6.4): the stroke session
// (targets, spacing, interpolation), the re-apply of the stroke mask to the target layers, the history entry at the
// end, and the eyedropper.
//
// Stroke model: opacity caps, flow builds. The dabs of `brush.rs` raise a per-set u16 mask; after every dab the
// changed texels of the target layer are recomputed from their pre-stroke value (kept in a `TileBackup`) and the
// mask, so overlapping dabs never exceed the stroke's opacity. The backup becomes the undo patch at the end.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use rayon::prelude::*;

use glam::{Mat3, Quat};

use crate::brush::{self, BrushMode, BrushParams, DabQuery, DabShape, FieldDab, StrokeScratch, TipGeom, apply_brush, dab_2d, unit};
use crate::brush_tip::{BrushTip, SurfaceReach, TipAlign, TipPose, tip_frame};
use crate::bvh::Hit;
use crate::doc_layers::PaintTarget;
use crate::document::Document;
use crate::error::EditError;
use crate::history::{Edit, TileBackup};
use crate::ids::{LayerId, TextureSetId};
use crate::layer::{LayerKind, TextureSet};
use crate::math::{DirtyRegion, IRect};
use crate::pixel::PixelBuf;

/// Where the eyedropper reads from.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SampleSource {
    Composite,
    Layer(LayerId),
}

/// A 3D sample further than this many radii from the previous one starts a new segment (section 6.4).
const MAX_BRIDGE_RADII: f32 = 6.0;
/// A segment that would need more dabs than this is not bridged (guards absurd 2D jumps).
const MAX_DABS_PER_SAMPLE: u32 = 100_000;
/// Rects of at least this many texels are re-applied on the rayon pool.
const PARALLEL_TEXELS: u64 = 1 << 15;

/// What a paced stroke call did (round 5 B-3, ARCHITECTURE.md 5.19).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StrokeStep {
    /// Every dab of the sample's segment is painted.
    Done,
    /// The caller's `more` said no before the segment's last dab: the rest is kept in the stroke and painted by
    /// `stroke_resume` (or by the next stroke call, which finishes it first).
    Partial,
}

/// Number of timing slots of `StrokeCounters::nanos`.
pub const STROKE_PROBE_SLOTS: usize = brush::probe::SLOTS;

/// Names of the slots of `StrokeCounters::nanos` (the app's stroke log). Slots 4, 5 and 7 are counts, not times:
/// per-set dabs x 1 000 000, touched texels, tree walks x 1 000 000. Round 7: slots 10, 11, 12 and 14 are counts
/// (pass-2 texels looked at, pass-2 calls skipped, band jobs, prepared candidates); 13 is the band jobs' time summed
/// over all threads (band work / raster = the effective parallel width).
pub const STROKE_PROBE_NAMES: [&str; STROKE_PROBE_SLOTS] = [
    "setup",
    "raster",
    "commit",
    "re-apply",
    "set dabs",
    "touched",
    "query",
    "walks",
    "first touch",
    "buffer copy",
    "pass-2 texels",
    "pass-2 skipped",
    "bands",
    "band work",
    "candidates",
];

/// The counters of the open stroke (round 5 B-2, ARCHITECTURE.md 5.19); reset by `stroke_begin*`.
///
/// `dabs`, `sets_started` and `buffer_copies` are always counted. `nanos` holds the phase times of the stroke probe
/// (`set_stroke_probe`), zero while the probe is off; it is read from the calling thread, so it is the stroke's own
/// as long as the stroke is driven from one thread (the UI thread).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StrokeCounters {
    /// Dabs (anchors) of the stroke; a 3D dab that reaches three sets counts once.
    pub dabs: u64,
    /// Per-set dabs: a 3D dab that paints in three sets counts three times.
    pub set_dabs: u64,
    /// Sets the stroke started painting in (the first touch: layer allocation, stroke buffer).
    pub sets_started: u32,
    /// Times a dab had to copy a layer buffer that a snapshot or a history entry still shared.
    pub buffer_copies: u32,
    /// The probe's slots (`STROKE_PROBE_NAMES`), nanoseconds for the timing slots.
    pub nanos: [u64; STROKE_PROBE_SLOTS],
}

/// Turns the stroke probe on or off (process-wide; off by default). With it off the dab phases are not timed and
/// every probe point costs one relaxed atomic load.
pub fn set_stroke_probe(on: bool) {
    brush::probe::set_enabled(on);
}

/// True while the stroke probe is on.
pub fn stroke_probe_on() -> bool {
    brush::probe::enabled()
}

/// The rest of a bridged 3D segment that a paced call left (`StrokeStep::Partial`).
#[derive(Copy, Clone)]
struct OpenSegment {
    from: Vec3,
    from_normal: Vec3,
    from_facing: Vec3,
    from_pressure: f32,
    to: Vec3,
    normal: Vec3,
    facing: Vec3,
    pressure: f32,
    len: f32,
    step: f32,
    /// The residual at the start of the segment (dab `k` lies `k * step - residual` from `from`).
    start_residual: f32,
    /// The residual to keep once the segment is done.
    end_residual: f32,
    count: u32,
    /// The next dab to paint (1-based).
    next: u32,
    /// A pressure-sized segment (branch `pen-input`, 6.4): the walk by the current radius replaces `step` / `count`.
    var: Option<VarWalk>,
}

/// The walk of a pressure-sized segment (ARCHITECTURE.md 6.4 "Spacing and interpolation"); distances are measured
/// from the segment start.
#[derive(Copy, Clone)]
struct VarWalk {
    /// Where the next dab goes.
    next_d: f32,
    /// Where the last dab went (`-start_residual` before the first).
    last_d: f32,
    /// The step's lower bound (half a texel).
    floor: f32,
}

/// True for a pressure-sized segment (6.4): size dynamics on and a pressure below 1 at either end. Every other segment
/// (every mouse segment, every stroke without size dynamics) keeps round 4's constant step.
#[inline]
fn pressure_sized(brush: &BrushParams, from_pressure: f32, pressure: f32) -> bool {
    brush.dynamics.size && (from_pressure < 1.0 || pressure < 1.0)
}

/// The dab step at `pressure` of a pressure-sized segment: the spacing of the current radius, at least `floor`.
#[inline]
fn step_at(brush: &BrushParams, pressure: f32, floor: f32) -> f32 {
    (brush.spacing * 2.0 * brush.radius * brush.dynamics.size_factor(pressure)).max(floor)
}

/// Upper bound of the dabs of a pressure-sized segment (the guard of 6.4).
#[inline]
fn var_bound(brush: &BrushParams, residual: f32, len: f32, from_pressure: f32, pressure: f32, floor: f32) -> f32 {
    ((residual + len) / step_at(brush, from_pressure.min(pressure), floor)).floor()
}

/// A stroke is 3D or 2D, decided by its first sample.
#[derive(Copy, Clone, PartialEq, Eq)]
enum StrokeKind {
    Surface,
    Texture,
}

/// The previous sample of the current segment.
#[derive(Copy, Clone)]
enum Sample {
    /// `normal` = the reference normal (unflipped), `facing` = the viewer-facing normal (`Hit::normal`).
    Surface { position: Vec3, normal: Vec3, facing: Vec3, pressure: f32 },
    Texture { set: TextureSetId, px: Vec2, pressure: f32 },
}

/// A set the stroke paints into.
struct ActiveSet {
    layer: LayerId,
    /// Pre-stroke pixels of the target layer: blend source during the stroke, undo record afterwards.
    backup: TileBackup,
    /// Where the set's stroke mask is non-zero.
    dirty: DirtyRegion,
}

/// What the stroke does with one texture set.
enum SetState {
    /// No dab has reached the set yet.
    Untouched,
    /// The set has no `PaintTarget::Ready` layer: skipped for the whole stroke, with the reason.
    Skipped(PaintTarget),
    Active(ActiveSet),
}

/// State of the open stroke: brush, kind (3D/2D), last sample, residual, per-set state (target layer, TileBackup, dirty).
pub(crate) struct StrokeSession {
    /// Sanitized copy of the caller's parameters.
    brush: BrushParams,
    kind: Option<StrokeKind>,
    /// Previous sample of the current segment; `None` at the start and after `stroke_break`.
    last: Option<Sample>,
    /// Distance travelled along the segment since the last dab, always below one step.
    residual: f32,
    /// Index == set id.
    sets: Vec<SetState>,
    /// Reused buffers of the re-apply loop.
    row: Vec<u8>,
    block: Vec<u8>,
    /// The tip and its per-stroke state, boxed so that `Document` (which holds the session inline) keeps its size.
    tip: Box<TipState>,
    /// Round 5: the open segment and the counters, boxed for the same reason.
    pace: Box<PaceState>,
}

/// The pacing and probe part of a stroke session (round 5 B-2, B-3).
struct PaceState {
    /// The unpainted rest of the last 3D segment; `None` outside a paced partial sample.
    open: Option<OpenSegment>,
    /// The always-on counters of `StrokeCounters`.
    counters: SessionCounters,
    /// The probe's thread counters at `stroke_begin*` (the stroke's times are the difference).
    probe_base: [u64; STROKE_PROBE_SLOTS],
}

/// The counters a session keeps itself.
#[derive(Copy, Clone, Default)]
struct SessionCounters {
    dabs: u64,
    set_dabs: u64,
    sets_started: u32,
    buffer_copies: u32,
}

/// The tip part of a stroke session (FEATURE_BRUSH 5.9).
struct TipState {
    /// The sanitized tip of the stroke; `brush.same_side_only` mirrors `tip.reach`.
    tip: BrushTip,
    /// Per-stroke data of a field tip (`None` for the ball).
    geom: Option<TipGeom>,
    /// Reused buffers of the shared sphere query.
    query: DabQuery,
    /// The tangent carried from dab to dab along a segment (FEATURE_BRUSH 5.3); `None` at the start of a segment.
    prev_up: Option<Vec3>,
    /// Where the last dab put the tip.
    pose: Option<TipPose>,
}

#[cfg(test)]
thread_local! {
    /// Test seam (C2 and the bench): paint 3D dabs with round 3's per-set loop (`dab_reaches_set` + `dab_3d`).
    static LEGACY_DABS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test seam: run `f` with round 3's per-set dab loop (ball tips only).
#[cfg(test)]
pub(crate) fn with_legacy_dabs<T>(f: impl FnOnce() -> T) -> T {
    LEGACY_DABS.with(|l| l.set(true));
    let out = f();
    LEGACY_DABS.with(|l| l.set(false));
    out
}

/// Brush parameters with every field forced into its documented range (NaN and infinities included).
fn sanitize(brush: &BrushParams) -> BrushParams {
    BrushParams {
        radius: if brush.radius.is_finite() { brush.radius.max(0.0) } else { 0.0 },
        hardness: if brush.hardness.is_nan() { 1.0 } else { brush.hardness.clamp(0.0, 1.0) },
        opacity: unit(brush.opacity),
        flow: unit(brush.flow),
        spacing: if brush.spacing.is_nan() { 0.12 } else { brush.spacing.clamp(0.02, 1.0) },
        dynamics: brush.dynamics.sanitized(),
        ..*brush
    }
}

/// Dabs along a segment of length `len` when `residual` has already been travelled since the last dab:
/// `(count, new residual)`. Dab `k` (1-based) lies `k * step - residual` from the start of the segment, so over a
/// whole polyline the dabs are exactly `step` apart and their number is `floor(total length / step)`.
fn dabs_on_segment(residual: f32, len: f32, step: f32) -> (u32, f32) {
    let travelled = residual + len;
    let count = (travelled / step).floor().max(0.0);
    let rest = travelled - count * step;
    (count as u32, if rest >= 0.0 && rest < step { rest } else { 0.0 })
}

#[cfg(test)]
thread_local! {
    /// Test seam of round 7 A4: re-apply whole rects (round 5's re-apply) instead of the changed spans.
    pub(crate) static FULL_REAPPLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Round 7 A4: re-apply only the changed span of each row (false under the test seam `FULL_REAPPLY`).
#[inline]
fn reapply_spans() -> bool {
    #[cfg(test)]
    {
        !FULL_REAPPLY.with(|f| f.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// Recomputes the layer texels of `r` from their pre-stroke values and the stroke mask (the "re-apply" of 6.4).
/// `backup` must already hold every tile of `r`.
#[allow(clippy::too_many_arguments)]
fn reapply_rect(
    buf: &mut PixelBuf,
    backup: &TileBackup,
    scratch: &StrokeScratch,
    r: IRect,
    brush: &BrushParams,
    alpha_lock: bool,
    row: &mut Vec<u8>,
    block: &mut Vec<u8>,
) {
    let (x0, x1) = (r.x0 as u32, r.x1 as u32);
    let (color, mode) = (brush.color, brush.mode);
    // Opacity is applied here, never baked into the mask (pitfall 8); the colour's alpha scales the strength.
    let ceiling = brush.opacity * (color[3] as f32 / 255.0);
    let shade = |original: &[u8], mask: &[u16], out: &mut [u8]| {
        let (src, dst) = (original.as_chunks::<4>().0, out.as_chunks_mut::<4>().0);
        for ((o, s), &m) in dst.iter_mut().zip(src).zip(mask) {
            *o = if m > 0 { apply_brush(*s, color, (m as f32 / 65535.0) * ceiling, mode, alpha_lock) } else { *s };
        }
    };

    // Round 7 A4: per row only the span whose mask the last 3D dab raised is shaded again; the other texels of the
    // rect already hold `apply_brush(original, mask)` (exact; see `StrokeScratch::changed_span`). Without span
    // information (a 2D dab) the whole row.
    let spans = reapply_spans() && scratch.changed_span(r.y0 as u32, x0, x1).is_some();
    let span_of = |y: u32| if spans { scratch.changed_span(y, x0, x1).unwrap_or((x0, x1)) } else { (x0, x1) };
    if r.area() < PARALLEL_TEXELS {
        for y in r.y0 as u32..r.y1 as u32 {
            let (a, b) = span_of(y);
            if a >= b {
                continue;
            }
            backup.original_row(buf, y, a, b, row);
            shade(row, scratch.mask_row(y, a, b), &mut buf.row_mut(y)[a as usize * 4..b as usize * 4]);
        }
        return;
    }
    // Rows are independent: compute them in parallel into a block, then copy the block into the layer.
    let stride = r.width() as usize * 4;
    block.clear();
    block.resize(stride * r.height() as usize, 0);
    let source: &PixelBuf = buf;
    block.par_chunks_mut(stride).enumerate().for_each_init(Vec::new, |original, (j, out)| {
        let y = r.y0 as u32 + j as u32;
        let (a, b) = span_of(y);
        if a < b {
            backup.original_row(source, y, a, b, original);
            shade(original, scratch.mask_row(y, a, b), &mut out[(a - x0) as usize * 4..(b - x0) as usize * 4]);
        }
    });
    for (j, out) in block.chunks(stride).enumerate() {
        let y = r.y0 as u32 + j as u32;
        let (a, b) = span_of(y);
        if a < b {
            buf.row_mut(y)[a as usize * 4..b as usize * 4].copy_from_slice(&out[(a - x0) as usize * 4..(b - x0) as usize * 4]);
        }
    }
}

impl Document {
    /// Starts a stroke (flushes a pending edit first). Err(NoPaintableLayer) if no set has a paintable active layer;
    /// Err(StrokeInProgress) if one is open.
    ///
    /// The brush parameters are clamped to their documented ranges. A set's target is its active layer as of the
    /// moment the stroke first reaches the set (no command can change it while the stroke is open).
    pub fn stroke_begin(&mut self, brush: &BrushParams) -> Result<(), EditError> {
        // One source of truth (FEATURE_BRUSH 4.1): the legacy entry point is the ball with today's side filter.
        self.stroke_begin_with(brush, &BrushTip::legacy(brush.same_side_only))
    }

    /// As `stroke_begin`, with an explicit tip (FEATURE_BRUSH 4.1, 5.9). `brush.same_side_only` is NOT read:
    /// `tip.reach` decides. The tip is copied and clamped here (`BrushTip::sanitized`), so a later change of the
    /// settings does not affect the stroke. A 2D stroke ignores the tip.
    pub fn stroke_begin_with(&mut self, brush: &BrushParams, tip: &BrushTip) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        if !self.set_ids().into_iter().any(|set| matches!(self.paint_target(set), PaintTarget::Ready(_))) {
            return Err(EditError::NoPaintableLayer);
        }
        let tip = tip.sanitized();
        let geom = tip.field().map(|f| TipGeom::new(f, tip.through));
        self.stroke = Some(StrokeSession {
            brush: BrushParams { same_side_only: tip.reach == SurfaceReach::SameSide, ..sanitize(brush) },
            kind: None,
            last: None,
            residual: 0.0,
            sets: self.sets.iter().map(|_| SetState::Untouched).collect(),
            row: Vec::new(),
            block: Vec::new(),
            tip: Box::new(TipState { tip, geom, query: DabQuery::default(), prev_up: None, pose: None }),
            pace: Box::new(PaceState { open: None, counters: SessionCounters::default(), probe_base: brush::probe::read() }),
        });
        Ok(())
    }

    /// The counters of the open stroke (round 5 B-2): dabs, sets started, buffer copies and, while the stroke probe
    /// is on (`set_stroke_probe`), the phase times since `stroke_begin*`. All zero without an open stroke; read it
    /// before `stroke_end`.
    pub fn stroke_counters(&self) -> StrokeCounters {
        let Some(s) = &self.stroke else { return StrokeCounters::default() };
        let now = brush::probe::read();
        StrokeCounters {
            dabs: s.pace.counters.dabs,
            set_dabs: s.pace.counters.set_dabs,
            sets_started: s.pace.counters.sets_started,
            buffer_copies: s.pace.counters.buffer_copies,
            nanos: std::array::from_fn(|i| now[i].saturating_sub(s.pace.probe_base[i])),
        }
    }

    /// Pose of the last dab of the open 3D stroke (centre `c'`, world-from-tip rotation, world length of one tip
    /// unit); `None` without an open stroke or before its first 3D dab.
    pub fn stroke_tip_pose(&self) -> Option<TipPose> {
        self.stroke.as_ref()?.tip.pose
    }

    /// Adds a surface sample: dabs are interpolated from the previous sample (section 6.4). The reference normal is
    /// `hit.surface_normal()`. Ignored without an open 3D-capable stroke.
    ///
    /// `pressure` (0..=1) scales the flow and is interpolated along the segment like the position.
    pub fn stroke_to_3d(&mut self, hit: &Hit, pressure: f32) {
        self.stroke_to_3d_paced(hit, pressure, &mut || true);
    }

    /// `stroke_to_3d` that asks `more()` after every dab and stops with `StrokeStep::Partial` when it says no before
    /// the segment's last dab (round 5 B-3: the app paces a stroke by the dab, not by the sample). At least one dab
    /// is painted per call. The rest of the segment is kept in the stroke and painted by `stroke_resume`; every other
    /// stroke call (`stroke_to_3d*` with a new sample, `stroke_break`, `stroke_end`, `stroke_to_2d`) first finishes
    /// it, and `stroke_cancel` drops it, so the dabs and their order never depend on the pacing.
    pub fn stroke_to_3d_paced(&mut self, hit: &Hit, pressure: f32, more: &mut dyn FnMut() -> bool) -> StrokeStep {
        let Some(mut session) = self.stroke.take() else { return StrokeStep::Done };
        self.run_segment(&mut session, &mut || true);
        let mut result = StrokeStep::Done;
        if hit.position.is_finite() && *session.kind.get_or_insert(StrokeKind::Surface) == StrokeKind::Surface {
            let position = hit.position;
            let normal = if hit.surface_normal().is_finite() { hit.surface_normal() } else { Vec3::ZERO };
            let facing = if hit.normal.is_finite() { hit.normal } else { normal };
            let pressure = unit(pressure);
            let radius = session.brush.radius;
            let bridge = match session.last {
                Some(Sample::Surface { position: from, normal: from_normal, facing: from_facing, pressure: from_pressure })
                    if position.distance(from) <= MAX_BRIDGE_RADII * radius && from_normal.dot(normal) >= 0.0 =>
                {
                    Some((from, from_normal, from_facing, from_pressure))
                }
                _ => None,
            };
            match bridge {
                Some((from, from_normal, from_facing, from_pressure)) => {
                    let len = position.distance(from);
                    // Half a texel of the set under the cursor (the finest set if the hit has none).
                    let texel = match hit.texture_set {
                        Some(set) if set.index() < self.sets.len() => self.texel_world_size(set),
                        _ => self.sets.iter().map(|s| s.texel_map.stats.texel_world).fold(f32::INFINITY, f32::min),
                    };
                    let texel = if texel.is_finite() && texel > 0.0 { texel } else { 1.0 };
                    let step = (session.brush.spacing * 2.0 * radius).max(0.5 * texel);
                    // A pressure-sized segment walks by the current radius (6.4); every other keeps the constant step.
                    let var = pressure_sized(&session.brush, from_pressure, pressure).then(|| VarWalk {
                        next_d: (step_at(&session.brush, from_pressure, 0.5 * texel) - session.residual).max(0.0),
                        last_d: -session.residual,
                        floor: 0.5 * texel,
                    });
                    let (count, residual, too_many) = match var {
                        Some(w) => {
                            let bound = var_bound(&session.brush, session.residual, len, from_pressure, pressure, w.floor);
                            (0, 0.0, bound > MAX_DABS_PER_SAMPLE as f32)
                        }
                        None => {
                            let (count, residual) = dabs_on_segment(session.residual, len, step);
                            (count, residual, count > MAX_DABS_PER_SAMPLE)
                        }
                    };
                    if too_many {
                        session.tip.prev_up = None;
                        self.surface_dab(&mut session, position, normal, facing, pressure);
                        more();
                        session.residual = 0.0;
                    } else {
                        session.pace.open = Some(OpenSegment {
                            from,
                            from_normal,
                            from_facing,
                            from_pressure,
                            to: position,
                            normal,
                            facing,
                            pressure,
                            len,
                            step,
                            start_residual: session.residual,
                            end_residual: residual,
                            count,
                            next: 1,
                            var,
                        });
                        result = self.run_segment(&mut session, more);
                    }
                }
                None => {
                    // First sample of a segment: one dab at the sample; the tip frame starts from the up hint.
                    session.tip.prev_up = None;
                    self.surface_dab(&mut session, position, normal, facing, pressure);
                    more();
                    session.residual = 0.0;
                }
            }
            session.last = Some(Sample::Surface { position, normal, facing, pressure });
        }
        self.stroke = Some(session);
        result
    }

    /// Continues the segment a paced call left (`StrokeStep::Partial`), asking `more()` after every dab; at least one
    /// dab per call. `Done` (and nothing painted) when no segment is open or no stroke is.
    pub fn stroke_resume(&mut self, more: &mut dyn FnMut() -> bool) -> StrokeStep {
        let Some(mut session) = self.stroke.take() else { return StrokeStep::Done };
        let step = self.run_segment(&mut session, more);
        self.stroke = Some(session);
        step
    }

    /// Paints the open segment's dabs from its next one, asking `more()` after each; `Partial` (the segment kept)
    /// when `more()` says no before its last dab. The dab positions are those of round 4's loop, bit for bit.
    fn run_segment(&mut self, session: &mut StrokeSession, more: &mut dyn FnMut() -> bool) -> StrokeStep {
        let Some(mut seg) = session.pace.open.take() else { return StrokeStep::Done };
        if let Some(walk) = seg.var {
            return self.run_var_segment(session, seg, walk, more);
        }
        while seg.next <= seg.count {
            let k = seg.next;
            seg.next += 1;
            let t = ((k as f32 * seg.step - seg.start_residual) / seg.len).clamp(0.0, 1.0);
            let n = seg.from_normal.lerp(seg.normal, t).normalize_or_zero();
            let n = if n == Vec3::ZERO { seg.normal } else { n };
            // The viewer-facing normal is blended exactly like the reference normal (FEATURE_BRUSH 5.1).
            let f = seg.from_facing.lerp(seg.facing, t).normalize_or_zero();
            let f = if f == Vec3::ZERO { seg.facing } else { f };
            let p = seg.from_pressure + (seg.pressure - seg.from_pressure) * t;
            self.surface_dab(session, seg.from.lerp(seg.to, t), n, f, p);
            if !more() && seg.next <= seg.count {
                session.pace.open = Some(seg);
                return StrokeStep::Partial;
            }
        }
        session.residual = seg.end_residual;
        StrokeStep::Done
    }

    /// `run_segment` for a pressure-sized segment (6.4): a dab at `next_d`, the next one a step of that dab's own
    /// radius further; afterwards the residual is the distance since the last dab. Normals and pressure are
    /// interpolated as in the constant walk; the dab sequence does not depend on `more`.
    fn run_var_segment(
        &mut self,
        session: &mut StrokeSession,
        mut seg: OpenSegment,
        mut walk: VarWalk,
        more: &mut dyn FnMut() -> bool,
    ) -> StrokeStep {
        while walk.next_d <= seg.len {
            let d = walk.next_d;
            let t = if seg.len > 0.0 { (d / seg.len).clamp(0.0, 1.0) } else { 1.0 };
            let n = seg.from_normal.lerp(seg.normal, t).normalize_or_zero();
            let n = if n == Vec3::ZERO { seg.normal } else { n };
            let f = seg.from_facing.lerp(seg.facing, t).normalize_or_zero();
            let f = if f == Vec3::ZERO { seg.facing } else { f };
            let p = seg.from_pressure + (seg.pressure - seg.from_pressure) * t;
            self.surface_dab(session, seg.from.lerp(seg.to, t), n, f, p);
            walk.last_d = d;
            walk.next_d = d + step_at(&session.brush, p, walk.floor);
            if !more() && walk.next_d <= seg.len {
                seg.var = Some(walk);
                session.pace.open = Some(seg);
                return StrokeStep::Partial;
            }
        }
        session.residual = (seg.len - walk.last_d).max(0.0);
        StrokeStep::Done
    }

    /// Adds a sample in continuous texel coordinates of `set`.
    ///
    /// Dabs are interpolated from the previous sample of the same set, however far away it is (a sample on another
    /// set starts a new segment). Ignored without an open 2D-capable stroke, for an unknown set and for a
    /// non-finite position.
    pub fn stroke_to_2d(&mut self, set: TextureSetId, pos_px: Vec2, pressure: f32) {
        let Some(mut session) = self.stroke.take() else { return };
        self.run_segment(&mut session, &mut || true);
        if set.index() < self.sets.len()
            && pos_px.is_finite()
            && *session.kind.get_or_insert(StrokeKind::Texture) == StrokeKind::Texture
        {
            let pressure = unit(pressure);
            let bridge = match session.last {
                Some(Sample::Texture { set: from_set, px, pressure: from_pressure }) if from_set == set => {
                    Some((px, from_pressure))
                }
                _ => None,
            };
            let mut bridged = false;
            if let Some((from, from_pressure)) = bridge
                && pressure_sized(&session.brush, from_pressure, pressure)
            {
                // A pressure-sized segment (6.4): the walk by the current radius, step floor half a texel.
                let len = pos_px.distance(from);
                let brush = session.brush;
                if len.is_finite()
                    && var_bound(&brush, session.residual, len, from_pressure, pressure, 0.5) <= MAX_DABS_PER_SAMPLE as f32
                {
                    let mut d = (step_at(&brush, from_pressure, 0.5) - session.residual).max(0.0);
                    let mut last_d = -session.residual;
                    while d <= len {
                        let t = if len > 0.0 { (d / len).clamp(0.0, 1.0) } else { 1.0 };
                        let p = from_pressure + (pressure - from_pressure) * t;
                        self.texture_dab(&mut session, set, from.lerp(pos_px, t), p);
                        last_d = d;
                        d += step_at(&brush, p, 0.5);
                    }
                    session.residual = (len - last_d).max(0.0);
                    bridged = true;
                }
            } else if let Some((from, from_pressure)) = bridge {
                let len = pos_px.distance(from);
                let step = (session.brush.spacing * 2.0 * session.brush.radius).max(0.5);
                let (count, residual) = dabs_on_segment(session.residual, len, step);
                if len.is_finite() && count <= MAX_DABS_PER_SAMPLE {
                    for k in 1..=count {
                        let t = ((k as f32 * step - session.residual) / len).clamp(0.0, 1.0);
                        let p = from_pressure + (pressure - from_pressure) * t;
                        self.texture_dab(&mut session, set, from.lerp(pos_px, t), p);
                    }
                    session.residual = residual;
                    bridged = true;
                }
            }
            if !bridged {
                self.texture_dab(&mut session, set, pos_px, pressure);
                session.residual = 0.0;
            }
            session.last = Some(Sample::Texture { set, px: pos_px, pressure });
        }
        self.stroke = Some(session);
    }

    /// The pointer left the surface: the next sample starts a new segment without bridging.
    pub fn stroke_break(&mut self) {
        if let Some(mut session) = self.stroke.take() {
            self.run_segment(&mut session, &mut || true);
            session.last = None;
            session.residual = 0.0;
            self.stroke = Some(session);
        }
    }

    /// Ends the stroke and pushes one history entry (Edit::Multi over the touched sets). Returns false if nothing changed.
    ///
    /// The entry is labelled "Brush Stroke" or "Erase". Without an open stroke, or when no pixel differs from its
    /// pre-stroke value, nothing is pushed.
    pub fn stroke_end(&mut self) -> bool {
        let Some(mut session) = self.stroke.take() else { return false };
        // A partial segment is painted to its end first (round 5 B-3): the pixels never depend on the pacing.
        self.run_segment(&mut session, &mut || true);
        let label = match session.brush.mode {
            BrushMode::Paint => "Brush Stroke",
            BrushMode::Erase => "Erase",
        };
        let mut edits = Vec::new();
        for (index, state) in session.sets.into_iter().enumerate() {
            let SetState::Active(active) = state else { continue };
            let Some(set) = self.sets.get_mut(index) else { continue };
            if let Some(scratch) = set.scratch.as_mut() {
                active.dirty.rects().iter().for_each(|r| scratch.clear_rect(*r));
            }
            let Some(LayerKind::Raster(buf)) = set.layer(active.layer).map(|l| &l.kind) else { continue };
            let patch = active.backup.into_patch(buf);
            if !patch.is_empty() {
                edits.push(Edit::Pixels { set: TextureSetId(index as u32), layer: active.layer, patch });
            }
        }
        if edits.is_empty() {
            return false;
        }
        self.push_edit(label, Edit::Multi(edits));
        true
    }

    /// Restores every touched layer from its backup; no history entry.
    pub fn stroke_cancel(&mut self) {
        let Some(session) = self.stroke.take() else { return };
        for (index, state) in session.sets.into_iter().enumerate() {
            let SetState::Active(active) = state else { continue };
            let Some(set) = self.sets.get_mut(index) else { continue };
            if let Some(scratch) = set.scratch.as_mut() {
                active.dirty.rects().iter().for_each(|r| scratch.clear_rect(*r));
            }
            let Some(li) = set.layer_index(active.layer) else { continue };
            let LayerKind::Raster(buf) = &mut set.layers[li].kind else { continue };
            if active.backup.is_empty() {
                continue;
            }
            let dirty = active.backup.restore(Arc::make_mut(buf));
            self.mark_layer_dirty(active.layer, dirty);
        }
    }

    /// True while a stroke is open.
    pub fn stroke_active(&self) -> bool {
        self.stroke.is_some()
    }

    /// Sets (ascending) in which a dab of the open stroke found triangles but which have no `PaintTarget::Ready` layer,
    /// with the reason (`Locked`, `Hidden`, `NotRaster` or `NoActiveLayer`). Empty without an open stroke; read it just
    /// before `stroke_end` / `stroke_cancel`.
    ///
    /// For a 2D stroke a set counts as reached as soon as a sample lands on it.
    pub fn stroke_skipped_sets(&self) -> Vec<(TextureSetId, PaintTarget)> {
        let Some(session) = &self.stroke else { return Vec::new() };
        session
            .sets
            .iter()
            .enumerate()
            .filter_map(|(index, state)| match state {
                SetState::Skipped(reason) => Some((TextureSetId(index as u32), *reason)),
                _ => None,
            })
            .collect()
    }

    /// Eyedropper: nearest texel at `uv` (wrapped). None for an unknown set/layer.
    ///
    /// `Composite` reads the set's composite as of the last `recomposite_all`; `Layer(id)` reads that layer's own
    /// pixels (transparent outside a stamp layer's rect) and is `None` if the layer is not in `set`. A non-finite
    /// `uv` gives `None`.
    pub fn sample_color(&self, set: TextureSetId, uv: Vec2, source: SampleSource) -> Option<[u8; 4]> {
        let target = self.set(set)?;
        if target.width == 0 || target.height == 0 || !uv.is_finite() {
            return None;
        }
        // Wrap in integer pixel space (pitfall 1).
        let wrap = |t: f32, size: u32| ((t as f64 * size as f64).floor() as i64).rem_euclid(size as i64);
        let (x, y) = (wrap(uv.x, target.width), wrap(uv.y, target.height));
        match source {
            SampleSource::Composite => Some(target.composite.get(x as u32, y as u32)),
            SampleSource::Layer(id) => Some(target.layer(id)?.get(x as i32, y as i32)),
        }
    }

    /// Decides what the open stroke does with a set it reaches for the first time: paint on the active layer if it
    /// is `PaintTarget::Ready` (making sure the set's scratch exists), otherwise skip the set for the whole stroke.
    /// Counted in `StrokeCounters::sets_started` and timed in the probe's "first touch" slot.
    fn stroke_target(&mut self, counters: &mut SessionCounters, set: TextureSetId) -> SetState {
        let t = brush::probe::now();
        let state = match self.paint_target(set) {
            PaintTarget::Ready(layer) => {
                let target = &mut self.sets[set.index()];
                if let Some(index) = target.layer_index(layer) {
                    target.allocate_raster(index);
                }
                let (w, h) = (target.width, target.height);
                // Round 7 A1: the brush index of the set's map (once per map, in this "first touch" slot).
                target.texel_map.prepare_brush_index();
                if target.scratch.as_ref().is_none_or(|s| s.size() != (w, h)) {
                    target.scratch = Some(Box::new(StrokeScratch::new(w, h)));
                }
                counters.sets_started += 1;
                SetState::Active(ActiveSet { layer, backup: TileBackup::new(w, h), dirty: DirtyRegion::default() })
            }
            reason => SetState::Skipped(reason),
        };
        brush::probe::add(8, t);
        state
    }

    /// One dab of the open 3D stroke at the anchor `center`, applied to every set it reaches (FEATURE_BRUSH 5.3,
    /// 5.4): the tip frame, one shared sphere query for all sets, then per set the reach decision and the dab.
    /// `normal` = the reference normal (side test), `facing` = the viewer-facing normal (offset and frame).
    fn surface_dab(&mut self, session: &mut StrokeSession, center: Vec3, normal: Vec3, facing: Vec3, pressure: f32) {
        #[cfg(test)]
        if LEGACY_DABS.with(|l| l.get()) {
            return self.surface_dab_legacy(session, center, normal, pressure);
        }
        let (mesh, bvh) = (Arc::clone(&self.mesh), Arc::clone(&self.bvh));
        let brush = session.brush;
        // The tip's values (the field by its `Arc`, so that the session can be lent to `reapply` below).
        let tip = &session.tip.tip;
        let (field, reach, through) = (tip.field().cloned(), tip.reach, tip.through);
        // Pen pressure sizes the dab (branch `pen-input`): exactly `brush.radius` at pressure 1 or without size dynamics.
        let r = brush.radius * brush.dynamics.size_factor(pressure);
        let k = r * tip.scale;
        // The frame: A (aligned to the surface or fixed) times the user's rotation U.
        let align = match tip.align {
            TipAlign::Surface => {
                let (a, up) = tip_frame(facing, tip.up_hint, session.tip.prev_up);
                session.tip.prev_up = Some(up);
                a
            }
            TipAlign::World => Mat3::IDENTITY,
        };
        let rotation = align * Mat3::from_quat(if tip.rotation.is_finite() { tip.rotation } else { Quat::IDENTITY });
        // c' = c + offset * r * n_v (the anchor itself without an offset, bit for bit).
        let c = if tip.offset == 0.0 { center } else { center + tip.offset * r * facing.normalize_or_zero() };
        session.tip.pose = Some(TipPose { center: c, world_from_tip: rotation, unit: k });
        let shape = match (field.as_deref(), session.tip.geom) {
            (Some(field), Some(geom)) => DabShape {
                center: c,
                reach: brush::field_reach(field, k, through),
                field: Some(FieldDab { field, to_tip: rotation.transpose() * (1.0 / k), k, through, geom }),
            },
            _ => DabShape { center: c, reach: r, field: None },
        };
        if shape.field.is_some() && !(k > 0.0 && k.is_finite()) {
            return;
        }
        session.pace.counters.dabs += 1;
        let count = self.sets.len().min(session.sets.len());
        let (states, sets) = (&session.sets, &self.sets);
        let map_of = |i: usize| (!matches!(states[i], SetState::Skipped(_))).then(|| &*sets[i].texel_map);
        brush::shared_candidates(&mesh, &bvh, count, map_of, &shape, normal, reach, &mut session.tip.query);
        for index in 0..count {
            let set = TextureSetId(index as u32);
            if matches!(session.sets[index], SetState::Skipped(_)) {
                continue;
            }
            let list = std::mem::take(&mut session.tip.query.lists[index]);
            if matches!(session.sets[index], SetState::Untouched) {
                // The set is touched once a dab finds candidate triangles in it (5.4 step 6).
                if list.is_empty() {
                    session.tip.query.lists[index] = list;
                    continue;
                }
                session.sets[index] = self.stroke_target(&mut session.pace.counters, set);
            }
            let map = Arc::clone(&self.sets[index].texel_map);
            if matches!(session.sets[index], SetState::Active(_))
                && let Some(scratch) = self.sets[index].scratch.as_deref_mut()
            {
                session.pace.counters.set_dabs += 1;
                let strength = brush.flow * brush.dynamics.flow_factor(pressure);
                let rects = brush::dab_tip(&mesh, &map, scratch, &shape, &list, brush.hardness, strength);
                self.reapply(session, index, &rects);
            }
            session.tip.query.lists[index] = list;
        }
    }

    /// Round 3's dab loop: per set, `dab_reaches_set` and `dab_3d` (the reference of test C2; ball tips only).
    #[cfg(test)]
    fn surface_dab_legacy(&mut self, session: &mut StrokeSession, center: Vec3, normal: Vec3, pressure: f32) {
        use crate::brush::dab_3d;
        let (mesh, bvh) = (Arc::clone(&self.mesh), Arc::clone(&self.bvh));
        let brush = session.brush;
        let radius = brush.radius * brush.dynamics.size_factor(pressure);
        session.pace.counters.dabs += 1;
        for index in 0..self.sets.len().min(session.sets.len()) {
            let set = TextureSetId(index as u32);
            if matches!(session.sets[index], SetState::Skipped(_)) {
                continue;
            }
            let map = Arc::clone(&self.sets[index].texel_map);
            if matches!(session.sets[index], SetState::Untouched) {
                // The set is touched once a dab finds candidate triangles in it.
                if !brush::dab_reaches_set(&mesh, &bvh, &map, set, center, normal, radius, brush.same_side_only) {
                    continue;
                }
                session.sets[index] = self.stroke_target(&mut session.pace.counters, set);
            }
            if !matches!(session.sets[index], SetState::Active(_)) {
                continue;
            }
            let Some(scratch) = self.sets[index].scratch.as_deref_mut() else { continue };
            session.pace.counters.set_dabs += 1;
            let rects = dab_3d(
                &mesh,
                &bvh,
                &map,
                set,
                scratch,
                center,
                normal,
                radius,
                brush.hardness,
                brush.flow * brush.dynamics.flow_factor(pressure),
                brush.same_side_only,
            );
            self.reapply(session, index, &rects);
        }
    }

    /// One disc dab of the open 2D stroke on `set`.
    fn texture_dab(&mut self, session: &mut StrokeSession, set: TextureSetId, center_px: Vec2, pressure: f32) {
        let index = set.index();
        if index >= self.sets.len() || index >= session.sets.len() {
            return;
        }
        session.pace.counters.dabs += 1;
        if matches!(session.sets[index], SetState::Untouched) {
            session.sets[index] = self.stroke_target(&mut session.pace.counters, set);
        }
        if !matches!(session.sets[index], SetState::Active(_)) {
            return;
        }
        let brush = session.brush;
        let Some(scratch) = self.sets[index].scratch.as_deref_mut() else { return };
        session.pace.counters.set_dabs += 1;
        let radius = brush.radius * brush.dynamics.size_factor(pressure);
        let strength = brush.flow * brush.dynamics.flow_factor(pressure);
        let rects = dab_2d(scratch, center_px, radius, brush.hardness, strength);
        self.reapply(session, index, &rects);
    }

    /// Re-applies the stroke to the target layer of set `index` over the rects a dab changed: backs the tiles up,
    /// recomputes the texels from their pre-stroke values and reports the change.
    fn reapply(&mut self, session: &mut StrokeSession, index: usize, rects: &[IRect]) {
        if rects.is_empty() {
            return;
        }
        let StrokeSession { brush, sets, row, block, pace, .. } = session;
        let counters = &mut pace.counters;
        let SetState::Active(active) = &mut sets[index] else { return };
        // Recorded first: the mask changed there whatever happens below, and the stroke's end clears it.
        rects.iter().for_each(|r| active.dirty.add(*r));
        let TextureSet { layers, scratch, .. } = &mut self.sets[index];
        let Some(scratch) = scratch.as_deref() else { return };
        let Some(layer) = layers.iter_mut().find(|l| l.id == active.layer) else { return };
        let alpha_lock = layer.alpha_lock;
        let LayerKind::Raster(pixels) = &mut layer.kind else { return };
        // Copies only if a snapshot or a history entry still shares the buffer (section 6.4, "Targets"); the copy is
        // counted and timed (round 5 B-2).
        let shared = Arc::get_mut(pixels).is_none();
        let t = if shared { brush::probe::now() } else { None };
        let buf = Arc::make_mut(pixels);
        if shared {
            counters.buffer_copies += 1;
            brush::probe::add(9, t);
        }
        let bounds = buf.rect().intersect(&IRect::from_size(scratch.size().0, scratch.size().1));
        for r in rects {
            let r = r.intersect(&bounds);
            if r.is_empty() {
                continue;
            }
            let t = brush::probe::now();
            active.backup.touch_rect(buf, r);
            reapply_rect(buf, &active.backup, scratch, r, brush, alpha_lock, row, block);
            brush::probe::add(3, t);
        }
        let layer = active.layer;
        for r in rects {
            self.mark_layer_dirty(layer, *r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::BlendMode;
    use crate::document::Commit;
    use crate::fill::FillScope;
    use crate::history::testkit::{doc_state, doc_with_stamp, paint_rect};
    use crate::math::Ray;
    use crate::samples;
    use crate::testutil::doc_from_raw;

    const S0: TextureSetId = TextureSetId(0);
    const S1: TextureSetId = TextureSetId(1);

    #[track_caller]
    fn busy<T: std::fmt::Debug>(r: Result<T, EditError>) {
        assert!(matches!(r, Err(EditError::StrokeInProgress)), "{r:?}");
    }

    fn px(doc: &Document, layer: LayerId, x: u32, y: u32) -> [u8; 4] {
        doc.layer(layer).expect("layer").get(x as i32, y as i32)
    }

    fn mask(doc: &Document, set: TextureSetId, x: u32, y: u32) -> u16 {
        doc.sets[set.index()].scratch.as_ref().expect("scratch").mask(x, y)
    }

    /// Hit of a ray from `origin` along `dir`.
    /// Held by the tests that switch the process-wide stroke probe (round 7), so that one does not switch it off
    /// while another reads its counters.
    static PROBE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn hit(doc: &Document, origin: Vec3, dir: Vec3) -> Hit {
        doc.raycast(&Ray::new(origin, dir)).expect("the ray misses the mesh")
    }

    /// A one-sample 3D stroke.
    fn stroke_at(doc: &mut Document, b: &BrushParams, h: &Hit) -> bool {
        doc.stroke_begin(b).unwrap();
        doc.stroke_to_3d(h, 1.0);
        doc.stroke_end()
    }

    /// flat_quad document with a transparent paint layer on top of the white one.
    fn transparent_doc(size: u32) -> (Document, LayerId) {
        let mut doc = doc_from_raw(samples::flat_quad(), size);
        let layer = doc.add_raster_layer(S0, None).unwrap();
        doc.history.clear();
        (doc, layer)
    }

    /// Scaffold S1: `stroke_begin_with` with a ball equals `stroke_begin`; the reach decides, not `same_side_only`.
    #[test]
    fn stroke_begin_with_a_ball_is_the_legacy_stroke() {
        let paint = |same_side_only: bool, tip: Option<BrushTip>| {
            let mut doc = doc_from_raw(samples::seam_cube(1.0), 64);
            let b = BrushParams { radius: 1.2, hardness: 1.0, same_side_only, ..BrushParams::default() };
            match &tip {
                Some(tip) => doc.stroke_begin_with(&b, tip).unwrap(),
                None => doc.stroke_begin(&b).unwrap(),
            }
            assert!(doc.stroke_tip_pose().is_none(), "no dab yet");
            let h = hit(&doc, Vec3::new(0.1, 0.2, 3.0), Vec3::NEG_Z);
            doc.stroke_to_3d(&h, 1.0);
            let pose = doc.stroke_tip_pose().expect("a pose after a surface sample");
            assert!((pose.center - h.position).length() < 1e-6 && pose.unit == 1.2);
            assert!(doc.stroke_end());
            doc_state(&mut doc)
        };
        let same = paint(true, None);
        let all = paint(false, None);
        assert_ne!(same, all, "the ball reaches the back face only with All");
        assert_eq!(paint(false, Some(BrushTip::legacy(true))), same);
        assert_eq!(paint(true, Some(BrushTip::legacy(false))), all);
    }

    #[test]
    fn dabs_on_segment_carries_the_residual() {
        assert_eq!(dabs_on_segment(0.0, 3.0, 2.0), (1, 1.0));
        assert_eq!(dabs_on_segment(1.0, 7.0, 2.0), (4, 0.0));
        assert_eq!(dabs_on_segment(0.0, 0.5, 2.0), (0, 0.5));
        assert_eq!(dabs_on_segment(0.5, 4.5, 2.0), (2, 1.0));
        assert_eq!(dabs_on_segment(0.0, 0.0, 2.0), (0, 0.0));
    }

    /// Over a polyline the stroke emits one dab at the first sample plus floor(L / step) dabs, the residual carried
    /// from segment to segment. Every dab fully covers the probe texel, so its mask counts the dabs.
    #[test]
    fn dab_count_is_floor_of_length_over_step() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        // step = max(spacing * 2 * radius, 0.5) = 2 texels.
        let b = BrushParams { radius: 20.0, hardness: 1.0, flow: 0.1, spacing: 0.05, ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        let expected_mask = |dabs: u32| {
            let mut m = 0u16;
            for _ in 0..dabs {
                let rest = u16::MAX - m;
                m += ((rest as f32 * 0.1f32).ceil() as u32).min(rest as u32) as u16;
            }
            m
        };
        let points = [(20.0, 32.0), (23.0, 32.0), (23.0, 39.0), (23.0, 39.5), (27.5, 39.5)];
        let mut length = 0.0f32;
        let mut prev: Option<Vec2> = None;
        for (x, y) in points {
            let p = Vec2::new(x, y);
            length += prev.map_or(0.0, |q| q.distance(p));
            prev = Some(p);
            doc.stroke_to_2d(S0, p, 1.0);
            let dabs = 1 + (length / 2.0).floor() as u32;
            assert_eq!(mask(&doc, S0, 23, 35), expected_mask(dabs), "after {length} texels: {dabs} dabs expected");
        }
        assert_eq!(length, 15.0);
        // A break starts a new segment: one more dab at the next sample, no bridging.
        doc.stroke_break();
        doc.stroke_to_2d(S0, Vec2::new(23.0, 33.0), 1.0);
        assert_eq!(mask(&doc, S0, 23, 35), expected_mask(9));
        // Samples of the other kind are ignored in a 2D stroke.
        let h = hit(&doc, Vec3::new(0.3, 0.3, 1.0), Vec3::NEG_Z);
        doc.stroke_to_3d(&h, 1.0);
        assert_eq!(mask(&doc, S0, 23, 35), expected_mask(9));
        assert!(doc.stroke_end());
        assert!(doc.sets[0].scratch.as_ref().unwrap().mask_row(35, 0, 64).iter().all(|&m| m == 0), "scratch not cleared");
    }

    #[test]
    fn low_flow_soft_brush_builds_up_to_the_opacity() {
        let (mut doc, layer) = transparent_doc(64);
        let b = BrushParams { radius: 8.0, hardness: 0.0, opacity: 0.6, flow: 0.05, color: [200, 10, 10, 255], ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        let (mut prev_c, mut prev_e) = (0u8, 0u8);
        for i in 0..1500 {
            doc.stroke_to_2d(S0, Vec2::new(32.5, 32.5), 1.0);
            doc.stroke_break();
            let (c, e) = (px(&doc, layer, 32, 32)[3], px(&doc, layer, 37, 32)[3]);
            assert!(c >= prev_c && e >= prev_e, "alpha decreased");
            assert!(c <= 153 && e <= 153, "above the opacity");
            if i == 0 {
                assert!(c > 0 && c <= 8 && e < c, "one dab of 5 % flow: centre {c}, rim {e}");
            }
            if i == 30 {
                assert!(e > 0 && e < c, "the soft rim builds up more slowly: centre {c}, rim {e}");
            }
            (prev_c, prev_e) = (c, e);
        }
        // Flow builds up to the opacity wherever the brush has any coverage (ceil guarantees progress).
        assert_eq!(px(&doc, layer, 32, 32), [200, 10, 10, 153], "the stroke reaches its opacity");
        assert_eq!(prev_e, 153);
        assert!(doc.stroke_end());
        assert_eq!(doc.undo_label(), Some("Brush Stroke"));
    }

    #[test]
    fn opacity_ceiling_holds_under_self_overlap() {
        let (mut doc, layer) = transparent_doc(64);
        let b = BrushParams { radius: 6.0, hardness: 0.5, opacity: 0.5, color: [255, 0, 0, 255], ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        for i in 0..10 {
            let x = if i % 2 == 0 { 10.0 } else { 50.0 };
            doc.stroke_to_2d(S0, Vec2::new(x, 32.0), 1.0);
        }
        assert!(doc.stroke_end());
        let buf = crate::testutil::layer_dense(&doc, layer);
        assert!(buf.data.as_chunks::<4>().0.iter().all(|p| p[3] <= 128), "the opacity ceiling was exceeded");
        assert_eq!(px(&doc, layer, 30, 31), [255, 0, 0, 128]);
        // The colour's alpha scales the strength like the opacity.
        let (mut doc, layer) = transparent_doc(32);
        let b = BrushParams { radius: 6.0, color: [0, 0, 255, 51], ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(16.0, 16.0), 1.0);
        doc.stroke_to_2d(S0, Vec2::new(17.0, 16.0), 1.0);
        doc.stroke_end();
        assert_eq!(px(&doc, layer, 16, 16), [0, 0, 255, 51]);
    }

    #[test]
    fn pressure_scales_the_flow() {
        let (mut doc, layer) = transparent_doc(32);
        let b = BrushParams { radius: 6.0, hardness: 1.0, ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(16.0, 16.0), 0.5);
        assert_eq!(mask(&doc, S0, 16, 16), 32768);
        assert_eq!(px(&doc, layer, 16, 16), [0, 0, 0, 128]);
        doc.stroke_cancel();
    }

    #[test]
    fn erase_lowers_alpha_only() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let layer = LayerId(1);
        let b = BrushParams { radius: 6.0, hardness: 1.0, mode: BrushMode::Erase, ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(32.5, 32.5), 1.0);
        assert!(doc.stroke_end());
        assert_eq!(doc.undo_label(), Some("Erase"));
        assert_eq!(px(&doc, layer, 32, 32), [255, 255, 255, 0]);
        let rim = px(&doc, layer, 38, 32);
        assert!(rim[3] > 0 && rim[3] < 255, "{rim:?}");
        let buf = crate::testutil::layer_dense(&doc, layer);
        assert!(buf.data.as_chunks::<4>().0.iter().all(|p| p[..3] == [255, 255, 255]), "erase changed RGB");
    }

    #[test]
    fn alpha_lock_preserves_alpha() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let layer = LayerId(1);
        paint_rect(&mut doc, layer, IRect::new(20, 20, 40, 40), [0, 0, 255, 100]);
        paint_rect(&mut doc, layer, IRect::new(30, 30, 34, 34), [0, 255, 0, 0]);
        doc.set_layer_alpha_lock(layer, true).unwrap();
        let before = crate::testutil::layer_dense(&doc, layer).data;
        let b = BrushParams { radius: 12.0, hardness: 1.0, color: [255, 0, 0, 255], ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(32.0, 32.0), 1.0);
        assert!(doc.stroke_end());
        let after = crate::testutil::layer_dense(&doc, layer).data;
        for (a, b) in after.as_chunks::<4>().0.iter().zip(before.as_chunks::<4>().0) {
            assert_eq!(a[3], b[3], "alpha lock changed alpha");
        }
        assert_eq!(px(&doc, layer, 25, 25), [255, 0, 0, 100]);
        assert_eq!(px(&doc, layer, 32, 32), [255, 0, 0, 0], "RGB under alpha 0 is painted, alpha stays 0");
        // Erasing with alpha lock changes nothing: no history entry.
        let undo_len = doc.history.undo_len();
        doc.stroke_begin(&BrushParams { mode: BrushMode::Erase, ..b }).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(32.0, 32.0), 1.0);
        assert!(!doc.stroke_end());
        assert_eq!(doc.history.undo_len(), undo_len);
    }

    #[test]
    fn stroke_cancel_restores_exactly() {
        let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
        let before = doc_state(&mut doc);
        let revision = doc.revision();
        doc.stroke_begin(&BrushParams { radius: 0.3, color: [10, 200, 30, 255], ..BrushParams::default() }).unwrap();
        let a = hit(&doc, Vec3::new(2.0, 0.2, 0.1), Vec3::NEG_X);
        let b = hit(&doc, Vec3::new(0.3, 2.0, 0.1), Vec3::NEG_Y);
        doc.stroke_to_3d(&a, 1.0);
        doc.stroke_to_3d(&b, 0.7);
        assert_ne!(doc_state(&mut doc), before);
        doc.stroke_cancel();
        assert!(!doc.stroke_active());
        assert_eq!(doc_state(&mut doc), before);
        assert_eq!(doc.history.undo_len(), 0);
        assert_eq!(doc.revision(), revision);
        let scratch = doc.sets[0].scratch.as_ref().unwrap();
        assert!((0..128).all(|y| scratch.mask_row(y, 0, 128).iter().all(|&m| m == 0)), "scratch not cleared");
    }

    #[test]
    fn stroke_end_pushes_one_entry_and_undo_restores() {
        let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
        let before = doc_state(&mut doc);
        let b = BrushParams { radius: 0.2, color: [10, 200, 30, 255], ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        assert!(doc.stroke_active());
        // A 3D stroke along the top face and over the edge onto +X.
        for i in 0..=10 {
            let x = -0.3 + 0.07 * i as f32;
            doc.stroke_to_3d(&hit(&doc, Vec3::new(x, 2.0, 0.0), Vec3::NEG_Y), 1.0);
        }
        doc.stroke_to_3d(&hit(&doc, Vec3::new(2.0, 0.4, 0.0), Vec3::NEG_X), 1.0);
        assert!(doc.stroke_end());
        assert!(!doc.stroke_active());
        assert_eq!(doc.history.undo_len(), 1);
        assert!(doc.is_modified());
        let after = doc_state(&mut doc);
        assert_ne!(after, before);
        assert_eq!(doc.undo().as_deref(), Some("Brush Stroke"));
        assert_eq!(doc_state(&mut doc), before);
        assert_eq!(doc.redo().as_deref(), Some("Brush Stroke"));
        assert_eq!(doc_state(&mut doc), after);
        // An empty stroke changes nothing and pushes nothing.
        doc.stroke_begin(&b).unwrap();
        assert!(!doc.stroke_end());
        assert!(!doc.stroke_end(), "no open stroke");
        assert_eq!(doc.history.undo_len(), 1);
    }

    #[test]
    fn reversed_flat_quad_is_paintable_with_default_params() {
        let mut raw = samples::flat_quad();
        samples::reverse_winding(&mut raw);
        let mut doc = doc_from_raw(raw, 64);
        let h = hit(&doc, Vec3::new(0.5, 0.5, 1.0), Vec3::NEG_Z);
        assert!(!h.front_face);
        let b = BrushParams { radius: 0.1, ..BrushParams::default() };
        assert!(b.same_side_only);
        assert!(stroke_at(&mut doc, &b, &h));
        assert_eq!(px(&doc, LayerId(1), 32, 32), [0, 0, 0, 255]);
    }

    #[test]
    fn wrapped_quad_paints_at_wrapped_coordinates() {
        let mut doc = doc_from_raw(samples::wrapped_quad(), 64);
        let b = BrushParams { radius: 0.04, hardness: 1.0, ..BrushParams::default() };
        // (0.75, 0.25) on quad A has u = 1.25, v = 1.25: texel (16, 16).
        let h = hit(&doc, Vec3::new(0.75, 0.25, 1.0), Vec3::NEG_Z);
        assert!(stroke_at(&mut doc, &b, &h));
        assert_eq!(px(&doc, LayerId(1), 16, 16), [0, 0, 0, 255]);
        assert_eq!(px(&doc, LayerId(1), 48, 48), [255; 4]);
        // (0.5, 0.5) has u = v = 1.0: the disc is split over the four corners of the image.
        let h = hit(&doc, Vec3::new(0.5, 0.5, 1.0), Vec3::NEG_Z);
        assert!(stroke_at(&mut doc, &b, &h));
        for (x, y) in [(0, 0), (63, 63), (0, 63), (63, 0)] {
            assert_eq!(px(&doc, LayerId(1), x, y), [0, 0, 0, 255], "({x}, {y})");
        }
        assert_eq!(px(&doc, LayerId(1), 32, 32), [255; 4]);
        doc_state(&mut doc);
    }

    /// A dab on the head reaches the body too: one entry touching both sets; undo restores both.
    #[test]
    fn multi_set_stroke_gives_one_history_entry() {
        let mut doc = doc_from_raw(samples::two_material_model(), 64);
        let before = doc_state(&mut doc);
        let h = hit(&doc, Vec3::new(2.0, 0.8, 0.0), Vec3::NEG_X);
        assert_eq!(h.texture_set, Some(S1));
        assert!(stroke_at(&mut doc, &BrushParams { radius: 0.45, ..BrushParams::default() }, &h));
        assert_eq!(doc.history.undo_len(), 1);
        let Edit::Multi(edits) = &doc.history.undo.last().unwrap().edit else { panic!("not an Edit::Multi") };
        let sets: Vec<TextureSetId> = edits
            .iter()
            .map(|e| match e {
                Edit::Pixels { set, .. } => *set,
                _ => panic!("not a pixel edit"),
            })
            .collect();
        assert_eq!(sets, vec![S0, S1]);
        let after = doc_state(&mut doc);
        assert_ne!(after.sets[0], before.sets[0]);
        assert_ne!(after.sets[1], before.sets[1]);
        doc.undo().unwrap();
        assert_eq!(doc_state(&mut doc), before);
    }

    /// While a stroke is open every history-producing command returns StrokeInProgress and changes nothing.
    #[test]
    fn commands_are_refused_while_a_stroke_is_open() {
        let mut doc = doc_from_raw(samples::flat_quad(), 32);
        let upper = doc.add_raster_layer(S0, None).unwrap();
        paint_rect(&mut doc, upper, IRect::new(0, 0, 8, 8), [1, 2, 3, 255]);
        doc.undo().unwrap();
        doc.stroke_begin(&BrushParams { radius: 4.0, ..BrushParams::default() }).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(16.0, 16.0), 1.0);
        let state = doc_state(&mut doc);
        let (undo_len, redo_len) = (doc.history.undo_len(), doc.history.redo_len());
        let l = LayerId(1);
        busy(doc.stroke_begin(&BrushParams::default()));
        busy(doc.add_raster_layer(S0, None));
        busy(doc.remove_layer(upper));
        busy(doc.duplicate_layer(l));
        busy(doc.move_layer(l, 1));
        busy(doc.merge_down(upper));
        busy(doc.rasterize_layer(l));
        busy(doc.set_layer_visible(l, false));
        busy(doc.set_layer_locked(l, true));
        busy(doc.set_layer_alpha_lock(l, true));
        busy(doc.set_layer_blend(l, BlendMode::Multiply));
        busy(doc.set_layer_opacity(l, 0.5, Commit::Final));
        busy(doc.set_layer_opacity(l, 0.5, Commit::Preview));
        busy(doc.rename_layer(l, "x"));
        busy(doc.set_active_layer(S0, Some(l)));
        busy(doc.fill(S0, l, Vec2::splat(0.5), [9; 4], FillScope::Layer));
        busy(doc.fill(S0, l, Vec2::splat(0.5), [9; 4], FillScope::Island));
        assert_eq!(doc.undo(), None);
        assert_eq!(doc.redo(), None);
        assert!(doc.stroke_active());
        assert!(!doc.has_pending_edit());
        assert_eq!((doc.history.undo_len(), doc.history.redo_len()), (undo_len, redo_len));
        assert_eq!(doc_state(&mut doc), state);
        assert!(doc.stroke_end());
    }

    #[test]
    fn stroke_begin_flushes_a_pending_edit() {
        let mut doc = doc_from_raw(samples::flat_quad(), 32);
        doc.set_layer_opacity(LayerId(1), 0.5, Commit::Preview).unwrap();
        assert!(doc.has_pending_edit());
        doc.stroke_begin(&BrushParams::default()).unwrap();
        assert!(!doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), 1);
        doc.stroke_cancel();
        assert_eq!(doc.history.undo_len(), 1);
        assert_eq!(doc.layer(LayerId(1)).unwrap().opacity, 0.5);
    }

    #[test]
    fn locked_hidden_and_inactive_sets_are_skipped_and_reported() {
        let head_hit = |doc: &Document| hit(doc, Vec3::new(2.0, 0.8, 0.0), Vec3::NEG_X);
        let b = BrushParams { radius: 0.45, ..BrushParams::default() };
        type Case = (&'static str, fn(&mut Document), PaintTarget);
        let cases: [Case; 3] = [
            ("locked", |d| d.set_layer_locked(LayerId(2), true).unwrap(), PaintTarget::Locked(LayerId(2))),
            ("hidden", |d| d.set_layer_visible(LayerId(2), false).unwrap(), PaintTarget::Hidden(LayerId(2))),
            ("inactive", |d| d.set_active_layer(S1, None).unwrap(), PaintTarget::NoActiveLayer),
        ];
        for (name, setup, reason) in cases {
            let mut doc = doc_from_raw(samples::two_material_model(), 64);
            setup(&mut doc);
            let head = crate::testutil::layer_dense(&doc, LayerId(2)).data;
            let h = head_hit(&doc);
            doc.stroke_begin(&b).unwrap();
            assert!(doc.stroke_skipped_sets().is_empty(), "{name}: nothing reached yet");
            doc.stroke_to_3d(&h, 1.0);
            assert_eq!(doc.stroke_skipped_sets(), vec![(S1, reason)], "{name}");
            assert!(doc.stroke_end(), "{name}: the body set is still painted");
            assert!(doc.stroke_skipped_sets().is_empty());
            assert_eq!(crate::testutil::layer_dense(&doc, LayerId(2)).data, head, "{name}: the skipped set changed");
            let Edit::Multi(edits) = &doc.history.undo.last().unwrap().edit else { panic!() };
            assert!(matches!(edits[..], [Edit::Pixels { set: S0, .. }]), "{name}");
        }
        // A stroke that never reaches the skipped set does not report it.
        let mut doc = doc_from_raw(samples::two_material_model(), 64);
        doc.set_layer_locked(LayerId(2), true).unwrap();
        let h = hit(&doc, Vec3::new(0.0, -2.0, 0.0), Vec3::Y);
        doc.stroke_begin(&BrushParams { radius: 0.1, ..b }).unwrap();
        doc.stroke_to_3d(&h, 1.0);
        assert!(doc.stroke_skipped_sets().is_empty());
        assert!(doc.stroke_end());
        // No set ready at all.
        doc.set_layer_locked(LayerId(1), true).unwrap();
        assert!(matches!(doc.stroke_begin(&b), Err(EditError::NoPaintableLayer)));
        assert!(!doc.stroke_active());
    }

    #[test]
    fn non_raster_active_layer_is_skipped() {
        let mut doc = doc_with_stamp();
        doc.set_active_layer(S1, Some(LayerId(102))).unwrap();
        let h = hit(&doc, Vec3::new(2.0, 0.8, 0.0), Vec3::NEG_X);
        doc.stroke_begin(&BrushParams { radius: 0.45, ..BrushParams::default() }).unwrap();
        doc.stroke_to_3d(&h, 1.0);
        assert_eq!(doc.stroke_skipped_sets(), vec![(S1, PaintTarget::NotRaster(LayerId(102)))]);
        assert!(doc.stroke_end());
        // A 2D stroke reports the set as soon as a sample lands on it.
        doc.stroke_begin(&BrushParams { radius: 3.0, ..BrushParams::default() }).unwrap();
        doc.stroke_to_2d(S1, Vec2::new(10.0, 10.0), 1.0);
        assert_eq!(doc.stroke_skipped_sets(), vec![(S1, PaintTarget::NotRaster(LayerId(102)))]);
        assert!(!doc.stroke_end());
    }

    #[test]
    fn sample_color_wraps() {
        let mut doc = doc_from_raw(samples::flat_quad(), 16);
        paint_rect(&mut doc, LayerId(1), IRect::new(3, 5, 4, 6), [10, 20, 30, 40]);
        doc.recomposite_all();
        let uv = Vec2::new(1.0 + 3.5 / 16.0, -1.0 + 5.5 / 16.0);
        assert_eq!(doc.sample_color(S0, uv, SampleSource::Layer(LayerId(1))), Some([10, 20, 30, 40]));
        assert_eq!(doc.sample_color(S0, uv, SampleSource::Composite), Some(doc.sets[0].composite().get(3, 5)));
        assert_eq!(doc.sample_color(S0, Vec2::new(3.5 / 16.0, 5.5 / 16.0), SampleSource::Layer(LayerId(1))), Some([10, 20, 30, 40]));
        // Tiny negatives land on the last texel, not past it.
        assert_eq!(doc.sample_color(S0, Vec2::new(-1e-9, -1e-9), SampleSource::Layer(LayerId(1))), Some(px(&doc, LayerId(1), 15, 15)));
        assert_eq!(doc.sample_color(TextureSetId(3), uv, SampleSource::Composite), None);
        assert_eq!(doc.sample_color(S0, uv, SampleSource::Layer(LayerId(99))), None);
        assert_eq!(doc.sample_color(S0, Vec2::new(f32::NAN, 0.0), SampleSource::Composite), None);
        // A stamp layer reads transparent outside its rect.
        let doc = doc_with_stamp();
        assert_eq!(doc.sample_color(S0, Vec2::new(0.5 / 96.0, 0.5 / 96.0), SampleSource::Layer(LayerId(101))), Some([0; 4]));
        assert_eq!(doc.sample_color(S1, Vec2::splat(0.5), SampleSource::Layer(LayerId(101))), None, "layer of another set");
    }

    #[test]
    fn insane_brush_parameters_are_clamped() {
        let (mut doc, layer) = transparent_doc(32);
        let b = BrushParams { radius: 4.0, hardness: f32::NAN, opacity: 7.0, flow: -1.0, spacing: 0.0, ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(16.0, 16.0), 1.0);
        // Flow clamps to 0: nothing is painted, nothing panics.
        assert!(!doc.stroke_end());
        let b = BrushParams { radius: f32::INFINITY, opacity: f32::NAN, ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(16.0, 16.0), f32::NAN);
        doc.stroke_to_2d(S0, Vec2::new(f32::INFINITY, 16.0), 1.0);
        assert!(!doc.stroke_end());
        assert_eq!(px(&doc, layer, 16, 16), [0; 4]);
    }

    /// Timing probe on a real awkward model (not a regression test): `cargo test -p wte_core --release -- --ignored
    /// stroke_bench --nocapture`. Strokes across target/compat/harness/shapes/ngons_tiling.obj with growing brushes.
    #[test]
    #[ignore = "timing probe on a local model file"]
    fn stroke_bench_ngons_tiling() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/compat/harness/shapes/ngons_tiling.obj");
        if !path.exists() {
            return;
        }
        let model = crate::import::import_from_path(&path, &Default::default(), &crate::progress::NoProgress).unwrap();
        let n = model.mesh.sets.len();
        let mut doc = Document::from_import(model, vec![crate::doc_build::TextureChoice::Detected; n], &crate::progress::NoProgress).unwrap();
        set_stroke_probe(true);
        for i in 0..doc.sets.len() {
            let s = &doc.sets[i];
            eprintln!("set {i} {}x{} texel_world {}", s.width, s.height, s.texel_map.stats.texel_world);
        }
        let c = doc.mesh.bounds.center();
        let d = doc.mesh.diag();
        let mut hits = Vec::new();
        for k in 0..12 {
            let x = doc.mesh.bounds.min.x + (k as f32 + 0.5) / 12.0 * (doc.mesh.bounds.max.x - doc.mesh.bounds.min.x);
            for y in [0.3f32, 0.5, 0.7] {
                let yy = doc.mesh.bounds.min.y + y * (doc.mesh.bounds.max.y - doc.mesh.bounds.min.y);
                if let Some(h) = doc.raycast(&Ray::new(Vec3::new(x, yy, c.z + d), -Vec3::Z)) {
                    hits.push(h);
                }
            }
        }
        eprintln!("{} hits", hits.len());
        // A long stroke across the whole model (like a fuzz drag through model points), dense samples.
        let mut line = Vec::new();
        for k in 0..400 {
            let x = doc.mesh.bounds.min.x + (k as f32 + 0.5) / 400.0 * (doc.mesh.bounds.max.x - doc.mesh.bounds.min.x);
            let yy = doc.mesh.bounds.min.y + 0.5 * (doc.mesh.bounds.max.y - doc.mesh.bounds.min.y);
            if let Some(h) = doc.raycast(&Ray::new(Vec3::new(x, yy, c.z + d), -Vec3::Z)) {
                line.push(h);
            }
        }
        for (size_px, density_set) in [(512.0f32, 0u32), (2048.0, 0), (2048.0, 2)] {
            let radius = size_px * 0.5 * doc.texel_world_size(TextureSetId(density_set));
            let b = BrushParams { radius, ..Default::default() };
            let t = std::time::Instant::now();
            doc.stroke_begin(&b).unwrap();
            for h in &line {
                doc.stroke_to_3d(h, 1.0);
            }
            let painted = t.elapsed();
            doc.stroke_end();
            let phases: Vec<String> = brush::probe::take()
                .iter()
                .zip(["candidates ms", "raster ms", "commit ms", "re-apply ms", "dabs", "touched M"])
                .map(|(n, name)| format!("{name} {:.1}", *n as f64 / 1e6))
                .collect();
            eprintln!(
                "long stroke ({} samples), size {size_px} px of set {density_set} (radius {radius:.3}): {:.1} ms ({})",
                line.len(),
                painted.as_secs_f64() * 1e3,
                phases.join(", ")
            );
        }
        for hit_set in 0..doc.sets.len() as u32 {
            // A hit at the centroid of the first triangle of the set, looking along its normal.
            let Some(tri) = (0..doc.mesh.tri_count()).find(|&t| doc.mesh.tri_set(t) == Some(TextureSetId(hit_set))) else { continue };
            let p = doc.mesh.tri_positions(tri);
            let centroid = (p[0] + p[1] + p[2]) / 3.0;
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            let Some(h0) = doc.raycast(&Ray::new(centroid + n * 0.05, -n)) else { continue };
            assert_eq!(h0.texture_set, Some(TextureSetId(hit_set)));
            for size_px in [32.0f32, 512.0, 2048.0] {
                let radius = size_px * 0.5 * doc.texel_world_size(TextureSetId(hit_set));
                let b = BrushParams { radius, ..Default::default() };
                let t = std::time::Instant::now();
                doc.stroke_begin(&b).unwrap();
                // A short drag: the sample and one a radius away (bridged by dabs).
                doc.stroke_to_3d(&h0, 1.0);
                let mut h1 = h0;
                h1.position += Vec3::X * radius;
                doc.stroke_to_3d(&h1, 1.0);
                let painted = t.elapsed();
                doc.stroke_end();
                let t2 = std::time::Instant::now();
                doc.recomposite_all();
                let phases: Vec<String> = brush::probe::take()
                    .iter()
                    .zip(["candidates ms", "raster ms", "commit ms", "re-apply ms", "dabs", "touched M"])
                    .map(|(n, name)| format!("{name} {:.1}", *n as f64 / 1e6))
                    .collect();
                eprintln!(
                    "hit set {hit_set}, size {size_px} px (radius {radius:.4} world): stroke {:.1} ms, end+recomposite {:.1} ms ({})",
                    painted.as_secs_f64() * 1e3,
                    t2.elapsed().as_secs_f64() * 1e3,
                    phases.join(", ")
                );
            }
        }
    }

    // ----- Round 4 (FEATURE_BRUSH 8.1 C2, 8.4 bench) -----

    /// The masks of every set of the open stroke (empty for a set without a scratch).
    fn all_masks(doc: &Document) -> Vec<Vec<u16>> {
        doc.sets
            .iter()
            .map(|s| match s.scratch.as_deref() {
                Some(sc) => {
                    let (w, h) = sc.size();
                    (0..h).flat_map(|y| sc.mask_row(y, 0, w).to_vec()).collect()
                }
                None => Vec::new(),
            })
            .collect()
    }

    /// Paints with round 3's per-set loop and with the shared query; the masks of all sets before the end of the
    /// stroke and the documents after it must be equal. Returns the number of painted texels.
    fn both_paths(raw: &crate::mesh::RawMesh, size: u32, paint: impl Fn(&mut Document) -> Vec<Vec<u16>>) -> usize {
        let mut legacy = doc_from_raw(raw.clone(), size);
        let want = with_legacy_dabs(|| paint(&mut legacy));
        let mut shared = doc_from_raw(raw.clone(), size);
        let got = paint(&mut shared);
        assert_eq!(want.len(), got.len());
        for (set, (a, b)) in want.iter().zip(&got).enumerate() {
            assert!(a == b, "set {set}: the masks differ");
        }
        assert_eq!(doc_state(&mut legacy), doc_state(&mut shared));
        want.iter().flatten().filter(|&&m| m > 0).count()
    }

    /// Rays from around the model at random points of its bounds; the hits.
    fn random_hits(doc: &Document, seed: u64, count: usize) -> Vec<Hit> {
        let mut rng = crate::testutil::XorShift::new(seed);
        let (b, c) = (doc.mesh.bounds, doc.mesh.bounds.center());
        let mut hits = Vec::new();
        while hits.len() < count {
            let dir = Vec3::new(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5).normalize_or_zero();
            if dir == Vec3::ZERO {
                continue;
            }
            let target = b.min + (b.max - b.min) * Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32());
            let origin = c + dir * (doc.mesh.diag() * 2.0);
            if let Some(h) = doc.raycast(&Ray::new(origin, target - origin)) {
                hits.push(h);
            }
        }
        hits
    }

    /// C2: the shared query equals the per-set path. 200 seeded random dabs on `two_material_model` (256) and on
    /// `soup_sets(1, 20000, 4)`, and 30-sample strokes on `wrapped_quad`, `scattered_islands` and `mirrored_quads`,
    /// give equal masks of all sets and equal composites, for `SameSide` and for `All`.
    #[test]
    fn the_shared_query_equals_the_per_set_path() {
        for same_side_only in [true, false] {
            for (name, raw, size) in [
                ("two_material_model", samples::two_material_model(), 256u32),
                ("soup_sets", samples::soup_sets(1, 20_000, 4), 128),
            ] {
                let hits = random_hits(&doc_from_raw(raw.clone(), size), 7, 200);
                let painted = both_paths(&raw, size, |doc| {
                    let mut rng = crate::testutil::XorShift::new(11);
                    let diag = doc.mesh.diag();
                    let b = BrushParams { radius: 0.05 * diag, hardness: 0.7, same_side_only, ..BrushParams::default() };
                    doc.stroke_begin(&b).unwrap();
                    for h in &hits {
                        // One dab per sample (a break between them) with a radius of its own: separate strokes would
                        // compare the same thing, one stroke keeps every set's mask for the comparison.
                        doc.stroke_break();
                        if let Some(s) = doc.stroke.as_mut() {
                            s.brush.radius = (0.01 + 0.12 * rng.next_f32()) * diag;
                        }
                        doc.stroke_to_3d(h, 0.5 + 0.5 * rng.next_f32());
                    }
                    let masks = all_masks(doc);
                    assert!(doc.stroke_end());
                    masks
                });
                assert!(painted > 1000, "{name}: {painted}");
            }
            for (name, raw, from, to, radius) in [
                ("wrapped_quad", samples::wrapped_quad(), Vec2::new(0.1, 0.2), Vec2::new(2.4, 0.8), 0.15f32),
                ("scattered_islands", samples::scattered_islands(), Vec2::new(-1.0, -1.0), Vec2::new(1.0, 1.0), 0.2),
                ("mirrored_quads", samples::mirrored_quads(), Vec2::new(-0.9, -0.4), Vec2::new(0.9, 0.4), 0.2),
            ] {
                let painted = both_paths(&raw, 128, |doc| {
                    let b = BrushParams { radius, hardness: 0.5, same_side_only, ..BrushParams::default() };
                    doc.stroke_begin(&b).unwrap();
                    let lo = doc.mesh.bounds.min.truncate();
                    let hi = doc.mesh.bounds.max.truncate();
                    for i in 0..30 {
                        let t = i as f32 / 29.0;
                        let p = from.lerp(to, t).clamp(lo, hi);
                        if let Some(h) = doc.raycast(&Ray::new(Vec3::new(p.x, p.y, 3.0), Vec3::NEG_Z)) {
                            doc.stroke_to_3d(&h, 1.0);
                        }
                    }
                    let masks = all_masks(doc);
                    doc.stroke_end();
                    masks
                });
                assert!(painted > 500, "{name}: {painted}");
            }
            // C5's fold(60) row with the hit exactly as the raycast gives it (the interpolated normal, smoothed over
            // the hinge), as the app passes it: the shared query paints what round 3's loop paints.
            let painted = both_paths(&samples::fold(60.0), 256, |doc| {
                let h = doc.raycast(&Ray::new(Vec3::new(0.9, 0.5, 0.05), Vec3::NEG_Z)).expect("hit");
                assert!(h.tri < 2 && h.front_face);
                doc.stroke_begin(&BrushParams { radius: 0.2, hardness: 0.8, same_side_only, ..BrushParams::default() }).unwrap();
                doc.stroke_to_3d(&h, 1.0);
                let masks = all_masks(doc);
                doc.stroke_end();
                masks
            });
            assert!(painted > 50, "fold(60), same side {same_side_only}: {painted}");
        }
    }

    /// The pose of the last dab: the anchor moved by the offset along the viewer-facing normal, the Follow-surface
    /// frame, and `k = radius * scale`.
    #[test]
    fn the_tip_pose_follows_the_last_dab() {
        let mut raw = samples::flat_quad();
        samples::reverse_winding(&mut raw);
        let mut doc = doc_from_raw(raw, 64);
        let tip = BrushTip { offset: 0.5, scale: 2.0, reach: SurfaceReach::All, ..BrushTip::legacy(false) };
        doc.stroke_begin_with(&BrushParams { radius: 0.1, ..BrushParams::default() }, &tip).unwrap();
        assert!(doc.stroke_tip_pose().is_none());
        let h = hit(&doc, Vec3::new(0.4, 0.6, 1.0), Vec3::NEG_Z);
        assert!(!h.front_face, "the reversed quad is hit from behind its winding");
        doc.stroke_to_3d(&h, 1.0);
        let pose = doc.stroke_tip_pose().unwrap();
        // Facing +Z (towards the viewer) although the winding normal points to -Z.
        assert!((pose.center - (h.position + Vec3::Z * 0.05)).length() < 1e-6, "{pose:?}");
        assert!((pose.world_from_tip.y_axis - Vec3::Z).length() < 1e-5 && (pose.unit - 0.2).abs() < 1e-6);
        assert!(doc.stroke_end());
        assert!(doc.stroke_tip_pose().is_none());
    }

    /// C24, the stroke-buffer clause: a set that only the bounding ball of a Cube dab touches gets no stroke buffer.
    /// `two_material_model` with an unlocked paint layer on the head's set; the Cube dab on the body's top face is
    /// placed (as in `tests\brush_tips.rs`) so that its bounding ball holds head triangles and its box grown by one
    /// texel holds none. While the stroke is open the head's set stays untouched (no state, no scratch) and the body
    /// is active; the ball of the same reach makes the head's set active with a scratch.
    #[test]
    fn a_set_only_the_bounding_ball_touches_gets_no_stroke_buffer() {
        use crate::brush_tip::TipVolume;
        use crate::tip_field::TipField;
        let mut doc = doc_from_raw(samples::two_material_model(), 128);
        doc.add_raster_layer(S1, None).unwrap();
        let cube = Arc::new(TipField::cube());
        let up = Vec3::NEG_Z;
        let frame = tip_frame(Vec3::Y, up, None).0;
        let head_aa = doc.texel_world_size(S1);
        let head: Vec<u32> = (0..doc.mesh.tri_count()).filter(|&t| doc.mesh.tri_set(t) == Some(S1)).collect();
        let mut found = None;
        'search: for r in [0.2f32, 0.25, 0.3, 0.35, 0.4] {
            for d in [0.3f32, 0.35, 0.4, 0.45, 0.5] {
                let c = Vec3::new(d, 0.5, 0.0);
                let to_head = (Vec3::new(0.0, 0.8, 0.0) - c).normalize();
                let roll = Quat::from_rotation_z((-to_head.x).atan2(to_head.y));
                let world_from_tip = frame * Mat3::from_quat(roll);
                let mut q = Vec::new();
                doc.bvh.query_sphere(&doc.mesh, c, r * cube.bound() + 0.5 * head_aa, &mut q);
                let ball_holds_head = q.iter().any(|&t| doc.mesh.tri_set(t) == Some(S1));
                let (lo, hi) = (cube.bounds().min - Vec3::splat(head_aa / r), cube.bounds().max + Vec3::splat(head_aa / r));
                let box_holds_none = head.iter().all(|&t| {
                    let p = doc.mesh.tri_positions(t).map(|p| world_from_tip.transpose() * (p - c) / r);
                    (0..3).any(|a| p.iter().all(|v| v[a] < lo[a]) || p.iter().all(|v| v[a] > hi[a]))
                });
                if ball_holds_head && box_holds_none {
                    found = Some((c, r, roll));
                    break 'search;
                }
            }
        }
        let (c, r, roll) = found.expect("no placement where only the bounding ball reaches the head");
        let h = hit(&doc, c + Vec3::Y, Vec3::NEG_Y);
        assert_eq!(h.texture_set, Some(S0));
        let state = |doc: &Document, set: TextureSetId| match &doc.stroke.as_ref().unwrap().sets[set.index()] {
            SetState::Untouched => "untouched",
            SetState::Active(_) => "active",
            SetState::Skipped(_) => "skipped",
        };
        let tip = BrushTip { volume: TipVolume::Field(cube.clone()), up_hint: up, rotation: roll, ..BrushTip::legacy(false) };
        doc.stroke_begin_with(&BrushParams { radius: r, hardness: 1.0, ..BrushParams::default() }, &tip).unwrap();
        doc.stroke_to_3d(&h, 1.0);
        assert_eq!((state(&doc, S0), state(&doc, S1)), ("active", "untouched"));
        assert!(doc.sets[S1.index()].scratch.is_none(), "the head's set got a stroke buffer");
        assert!(doc.stroke_end());
        // The ball of the same reach reaches the head's set and gives it a buffer.
        let ball = BrushParams { radius: r * cube.bound(), hardness: 1.0, ..BrushParams::default() };
        doc.stroke_begin_with(&ball, &BrushTip::legacy(false)).unwrap();
        doc.stroke_to_3d(&h, 1.0);
        assert_eq!(state(&doc, S1), "active");
        assert!(doc.sets[S1.index()].scratch.is_some());
        doc.stroke_end();
    }

    /// Timing probe of FEATURE_BRUSH 8.4 (P1, P2, P3, P5, P6; not a regression test):
    /// `cargo test -p wte_core --release --lib -- --ignored stroke_bench_tips --nocapture`. 200 ray hits along a fixed
    /// path on (a) `tex_8192.obj`, (b) `ABeautifulGame.glb`, (c) `tex_4096.obj` (the path of P0's baseline bench),
    /// one warm-up stroke, then per diameter the tips round robin: round 3's per-set loop ("legacy", the in-run
    /// baseline), Ball/SameSide, Ball/All (hardness 0.8 and 1.0), Cube (0.8, 1.0) and Cube Through (0.8). Milliseconds
    /// per dab, median of the repetitions.
    ///
    /// Round 5: the stroke probe is off unless `WTE_BENCH_PROBE=1` (B-2: the bench then measures the probe's off cost;
    /// the phase columns need the probe on); the dabs come from `stroke_counters`. The row "Ball/SameSide r4 alloc"
    /// is Ball/SameSide with round 4's candidate list allocated per dab (B-4.1's in-run A/B; same pixels).
    #[test]
    #[ignore = "timing probe on local model files"]
    fn stroke_bench_tips() {
        use crate::brush_tip::TipVolume;
        const MODELS: [(&str, &str); 3] = [
            ("a", r"E:\claude_programs\wrapped_texture_editor\target\compat\topologies\models\tex_8192\tex_8192.obj"),
            ("b", r"E:\claude_downloads\glb_samples\ABeautifulGame.glb"),
            ("c", r"E:\claude_programs\wrapped_texture_editor\target\compat\topologies\models\tex_4096\tex_4096.obj"),
        ];
        let median = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.total_cmp(b));
            v[v.len() / 2]
        };
        let cube = Arc::new(crate::tip_field::TipField::cube());
        let only = std::env::var("WTE_BENCH_MODELS").unwrap_or_else(|_| "abc".into());
        let probe_on = std::env::var("WTE_BENCH_PROBE").is_ok_and(|v| v == "1");
        set_stroke_probe(probe_on);
        eprintln!("bench: stroke probe {}", if probe_on { "on" } else { "off (phase columns are 0)" });
        for (model, path) in MODELS {
            let path = std::path::Path::new(path);
            if !only.contains(model) || !path.exists() {
                eprintln!("bench skip ({model}) {}", path.display());
                continue;
            }
            let imported = crate::import::import_from_path(path, &Default::default(), &crate::progress::NoProgress).unwrap();
            let n = imported.mesh.sets.len();
            let mut doc =
                Document::from_import(imported, vec![crate::doc_build::TextureChoice::Detected; n], &crate::progress::NoProgress)
                    .unwrap();
            let (b, abg) = (doc.mesh.bounds, model == "b");
            let ext = b.max - b.min;
            let mut hits = Vec::new();
            for k in 0..200 {
                let f = 0.05 + 0.9 * (k as f32 + 0.5) / 200.0;
                let x = b.min.x + f * ext.x;
                let ray = if abg {
                    Ray::new(Vec3::new(x, b.max.y + 1.0, b.min.z + 0.0625 * ext.z), -Vec3::Y)
                } else {
                    Ray::new(Vec3::new(x, b.min.y + 0.55 * ext.y, b.max.z + 1.0), -Vec3::Z)
                };
                if let Some(h) = doc.raycast(&ray) {
                    hits.push(h);
                }
            }
            let set = hits[hits.len() / 2].texture_set.unwrap_or(TextureSetId(0));
            let up = if abg { Vec3::NEG_Z } else { Vec3::Y };
            eprintln!("bench model ({model}) {}: {} triangles, {} sets, {} hits", path.display(), doc.mesh.tri_count(), n, hits.len());
            // name, dab loop (0 shared query, 1 round 3's per-set loop, 2 shared with round 4's allocation), tip,
            // hardness
            let ball = |same: bool| BrushTip { up_hint: up, ..BrushTip::legacy(same) };
            let field = |through: bool| BrushTip {
                volume: TipVolume::Field(Arc::clone(&cube)),
                through,
                up_hint: up,
                ..BrushTip::legacy(false)
            };
            let mut tips: Vec<(&str, u8, BrushTip, f32)> = vec![
                ("legacy Ball/SameSide", 1, ball(true), 0.8),
                ("Ball/SameSide", 0, ball(true), 0.8),
                ("Ball/All", 0, ball(false), 0.8),
                ("Ball/All h1", 0, ball(false), 1.0),
                ("Ball/SameSide r4 alloc", 2, ball(true), 0.8),
            ];
            if !abg {
                tips.push(("Cube", 0, field(false), 0.8));
                tips.push(("Cube h1", 0, field(false), 1.0));
                tips.push(("Cube Through", 0, field(true), 0.8));
            }
            // Round 7: every shared-path tip also with round 5's rasterizer (`RASTER_R5`, mode 3), the in-process A/B;
            // the rows "... r5" follow their tip, and their pixels must be the same.
            let tips: Vec<(String, u8, BrushTip, f32)> = tips
                .into_iter()
                .flat_map(|(name, mode, tip, h)| {
                    let mut rows = vec![(name.to_string(), mode, tip.clone(), h)];
                    if mode == 0 {
                        rows.push((format!("{name} r5"), 3, tip, h));
                    }
                    rows
                })
                .collect();
            // Warm-up (first-use allocation of the layers and scratches).
            let warm = BrushParams { radius: 25.0 * doc.texel_world_size(set), ..Default::default() };
            doc.stroke_begin(&warm).unwrap();
            hits.iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
            doc.stroke_end();
            // Round 7: the warm-up's paint is undone, so that every hashed stroke starts from the same layer. (Before,
            // a stroke that changed nothing on top of the warm-up, e.g. tex_8192's first Cube row, left no history
            // entry, its `undo` removed the warm-up, and the next row's hash was taken on another base.)
            assert!(doc.undo().is_some(), "({model}) the warm-up stroke left no history entry");
            brush::probe::take();
            let diameters: &[f32] = if abg { &[50.0, 200.0, 512.0] } else { &[50.0, 512.0, 2048.0] };
            for &diameter in diameters {
                let radius = diameter * 0.5 * doc.texel_world_size(set);
                let reps = if diameter >= 2048.0 { 3 } else { 5 };
                let mut per_dab: Vec<Vec<f64>> = vec![Vec::new(); tips.len()];
                let mut phases: Vec<Vec<[f64; STROKE_PROBE_SLOTS]>> = vec![Vec::new(); tips.len()];
                let mut dabs = vec![0u64; tips.len()];
                // C1 on the real models: in the first repetition every stroke is hashed (the paint layers of all
                // sets) and undone, and the shared query must give round 3's pixels (legacy = Ball/SameSide).
                let mut hashes = vec![0u64; tips.len()];
                for rep in 0..reps {
                    for (i, (_, mode, tip, hardness)) in tips.iter().enumerate() {
                        let p = BrushParams { radius, hardness: *hardness, ..Default::default() };
                        let run = |doc: &mut Document| {
                            let t = std::time::Instant::now();
                            doc.stroke_begin_with(&p, tip).unwrap();
                            hits.iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
                            let ms = t.elapsed().as_secs_f64() * 1e3;
                            let set_dabs = doc.stroke_counters().set_dabs;
                            doc.stroke_end();
                            (ms, set_dabs)
                        };
                        let (ms, set_dabs) = match mode {
                            1 => with_legacy_dabs(|| run(&mut doc)),
                            2 => {
                                brush::FRESH_CANDIDATES.with(|f| f.set(true));
                                let out = run(&mut doc);
                                brush::FRESH_CANDIDATES.with(|f| f.set(false));
                                out
                            }
                            3 => {
                                brush::RASTER_R5.with(|f| f.set(true));
                                FULL_REAPPLY.with(|f| f.set(true));
                                let out = run(&mut doc);
                                brush::RASTER_R5.with(|f| f.set(false));
                                FULL_REAPPLY.with(|f| f.set(false));
                                out
                            }
                            _ => run(&mut doc),
                        };
                        if rep == 0 {
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            for s in doc.set_ids() {
                                if let PaintTarget::Ready(layer) = doc.paint_target(s) {
                                    std::hash::Hash::hash(&crate::testutil::hash_pixels(&crate::testutil::layer_dense(&doc, layer)), &mut h);
                                }
                            }
                            hashes[i] = std::hash::Hasher::finish(&h);
                            assert!(doc.undo().is_some(), "({model}) {diameter}: the stroke left no history entry");
                        }
                        let k = brush::probe::take();
                        let d = set_dabs.max(1);
                        dabs[i] = d;
                        per_dab[i].push(ms / d as f64);
                        phases[i].push(std::array::from_fn(|s| match s {
                            4 | 7 => k[s] as f64 / 1e6,
                            5 | 10 | 11 | 12 | 14 => k[s] as f64 / d as f64,
                            _ => k[s] as f64 / 1e6 / d as f64,
                        }));
                    }
                }
                eprintln!(
                    "bench ({model}) {diameter:>4} texels C1: legacy {:016x} shared {:016x} -> {}",
                    hashes[0],
                    hashes[1],
                    if hashes[0] == hashes[1] { "identical" } else { "DIFFERENT" }
                );
                assert_eq!(hashes[0], hashes[1], "({model}) {diameter}: the shared query changed round 3's pixels");
                let alloc = tips.iter().position(|t| t.1 == 2).expect("the r4 alloc row");
                assert_eq!(hashes[alloc], hashes[1], "({model}) {diameter}: the reused candidate list changed pixels");
                for (i, t) in tips.iter().enumerate().filter(|(_, t)| t.1 == 3) {
                    eprintln!(
                        "bench ({model}) {diameter:>4} texels P2: {} r7 {:016x} r5 {:016x} -> {}",
                        tips[i - 1].0,
                        hashes[i - 1],
                        hashes[i],
                        if hashes[i - 1] == hashes[i] { "identical" } else { "DIFFERENT" }
                    );
                    assert_eq!(hashes[i - 1], hashes[i], "({model}) {diameter}: {}: round 7 changed round 5's pixels", t.0);
                }
                for (i, (name, ..)) in tips.iter().enumerate() {
                    let ph: Vec<f64> =
                        (0..STROKE_PROBE_SLOTS).map(|s| median(phases[i].iter().map(|p| p[s]).collect())).collect();
                    eprintln!(
                        "bench ({model}) {diameter:>4} texels {name:<25} {:>5} dabs {:>9.4} ms/dab | setup {:.4} query {:.4} raster {:.4} commit {:.4} reapply {:.4} | walks {:.0} touched/dab {:.0} | pass2/dab {:.0} skipped/dab {:.0} bands/dab {:.1} bandwork {:.4} cands/dab {:.0}",
                        dabs[i],
                        median(per_dab[i].clone()),
                        ph[0],
                        ph[6],
                        ph[1],
                        ph[2],
                        ph[3],
                        ph[7],
                        ph[5],
                        ph[10],
                        ph[11],
                        ph[12],
                        ph[13],
                        ph[14]
                    );
                }
            }
        }
    }

    // ----- Round 5 (B-2 the stroke probe, B-3 pacing by the dab) -----

    /// A 3D path over the seam cube: along the top face in steps a few dab spacings long, then over the edge.
    fn cube_path(doc: &Document) -> Vec<Hit> {
        let mut hits: Vec<Hit> = (0..=10).map(|i| hit(doc, Vec3::new(-0.3 + 0.07 * i as f32, 2.0, 0.0), Vec3::NEG_Y)).collect();
        hits.push(hit(doc, Vec3::new(2.0, 0.4, 0.0), Vec3::NEG_X));
        hits.push(hit(doc, Vec3::new(2.0, 0.35, 0.2), Vec3::NEG_X));
        hits
    }

    fn cube_brush() -> BrushParams {
        BrushParams { radius: 0.05, hardness: 0.7, color: [10, 200, 30, 255], ..BrushParams::default() }
    }

    /// B1's new test: a stroke painted one dab per call (`stroke_to_3d_paced` + `stroke_resume` with a `more` that
    /// always says no) gives the masks, the dab count and the pixels of the unpaced stroke.
    #[test]
    fn one_dab_per_call_gives_the_unpaced_stroke() {
        let mut plain = doc_from_raw(samples::seam_cube(1.0), 128);
        let path = cube_path(&plain);
        plain.stroke_begin(&cube_brush()).unwrap();
        path.iter().for_each(|h| plain.stroke_to_3d(h, 1.0));
        let (dabs, masks) = (plain.stroke_counters().dabs, all_masks(&plain));
        assert!(plain.stroke_end());
        let want = doc_state(&mut plain);

        let mut paced = doc_from_raw(samples::seam_cube(1.0), 128);
        paced.stroke_begin(&cube_brush()).unwrap();
        let (mut partials, mut asked) = (0, 0u64);
        for h in &path {
            let before = paced.stroke_counters().dabs;
            let mut step = paced.stroke_to_3d_paced(h, 1.0, &mut || {
                asked += 1;
                false
            });
            assert!(paced.stroke_counters().dabs - before <= 1, "one dab per call");
            while step == StrokeStep::Partial {
                partials += 1;
                let before = paced.stroke_counters().dabs;
                step = paced.stroke_resume(&mut || false);
                assert_eq!(paced.stroke_counters().dabs - before, 1, "a resume paints exactly one dab");
            }
        }
        assert!(partials > 20, "the segments were split: {partials} partial calls");
        // Every first call paints one dab and asks once; every resume paints one more.
        assert_eq!(asked + partials as u64, dabs, "`more` is asked after every dab");
        assert_eq!(paced.stroke_counters().dabs, dabs);
        assert_eq!(all_masks(&paced), masks);
        assert_eq!(paced.stroke_resume(&mut || false), StrokeStep::Done, "nothing is open");
        assert!(paced.stroke_end());
        assert_eq!(doc_state(&mut paced), want);
    }

    /// A partial segment that is never resumed is finished by the next sample, by `stroke_break`, by `stroke_to_2d`
    /// and by `stroke_end` (the same pixels as the unpaced stroke); `stroke_cancel` drops it with the stroke.
    #[test]
    fn an_open_segment_is_finished_by_every_other_stroke_call() {
        let run = |paced: bool| {
            let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
            let path = cube_path(&doc);
            doc.stroke_begin(&cube_brush()).unwrap();
            let mut partials = 0;
            for (i, h) in path.iter().enumerate() {
                if i == 6 {
                    doc.stroke_break();
                }
                if paced {
                    partials += (doc.stroke_to_3d_paced(h, 1.0, &mut || false) == StrokeStep::Partial) as u32;
                } else {
                    doc.stroke_to_3d(h, 1.0);
                }
            }
            // A 2D sample is ignored by a 3D stroke, but it finishes the open segment first.
            doc.stroke_to_2d(S0, Vec2::new(10.0, 10.0), 1.0);
            let dabs = doc.stroke_counters().dabs;
            assert!(doc.stroke_end());
            (doc_state(&mut doc), dabs, partials)
        };
        let (want, dabs, _) = run(false);
        let (got, paced_dabs, partials) = run(true);
        assert!(partials > 5, "{partials}");
        assert_eq!(paced_dabs, dabs);
        assert_eq!(got, want);
        // `stroke_end` alone finishes the last open segment.
        let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
        let path = cube_path(&doc);
        doc.stroke_begin(&cube_brush()).unwrap();
        path[..4].iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
        assert_eq!(doc.stroke_to_3d_paced(&path[4], 1.0, &mut || false), StrokeStep::Partial);
        assert!(doc.stroke_end());
        let mut plain = doc_from_raw(samples::seam_cube(1.0), 128);
        plain.stroke_begin(&cube_brush()).unwrap();
        path[..5].iter().for_each(|h| plain.stroke_to_3d(h, 1.0));
        assert!(plain.stroke_end());
        assert_eq!(doc_state(&mut doc), doc_state(&mut plain));
        // Cancel with an open segment: the document as before, no entry, nothing left to resume.
        let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
        let before = doc_state(&mut doc);
        doc.stroke_begin(&cube_brush()).unwrap();
        doc.stroke_to_3d(&path[0], 1.0);
        assert_eq!(doc.stroke_to_3d_paced(&path[1], 1.0, &mut || false), StrokeStep::Partial);
        doc.stroke_cancel();
        assert_eq!(doc.stroke_resume(&mut || true), StrokeStep::Done);
        assert_eq!(doc_state(&mut doc), before);
        assert_eq!(doc.history.undo_len(), 0);
    }

    /// B-2: the counters of the open stroke (dabs, sets started, buffer copies), reset by `stroke_begin`; the probe's
    /// phase times while it is on.
    #[test]
    fn the_stroke_counters_count_dabs_sets_and_copies() {
        let mut doc = doc_from_raw(samples::two_material_model(), 64);
        assert_eq!(doc.stroke_counters(), StrokeCounters::default(), "no stroke");
        let h = hit(&doc, Vec3::new(2.0, 0.8, 0.0), Vec3::NEG_X);
        // A first stroke makes sure both paint layers have their buffers.
        assert!(stroke_at(&mut doc, &BrushParams { radius: 0.45, ..BrushParams::default() }, &h));
        // A snapshot shares the layer buffers: the first dab in each set copies its buffer once.
        let snapshot = doc.snapshot();
        let _probe = PROBE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        set_stroke_probe(true);
        doc.stroke_begin(&BrushParams { radius: 0.45, ..BrushParams::default() }).unwrap();
        doc.stroke_to_3d(&h, 1.0);
        let mut h2 = h;
        h2.position += Vec3::Y * 0.3;
        doc.stroke_to_3d(&h2, 1.0);
        let c = doc.stroke_counters();
        set_stroke_probe(false);
        assert!(c.dabs >= 2, "{c:?}");
        assert_eq!(c.sets_started, 2, "the dab reaches the head and the body");
        assert!(c.set_dabs > c.dabs, "a dab paints in both sets: {c:?}");
        assert_eq!(c.buffer_copies, 2, "one copy per shared buffer: {c:?}");
        assert!(c.nanos[1] > 0 && c.nanos[4] > 0 && c.nanos[8] > 0 && c.nanos[9] > 0, "the probe timed the dabs: {c:?}");
        assert!(doc.stroke_end());
        drop(snapshot);
        doc.stroke_begin(&BrushParams { radius: 0.45, ..BrushParams::default() }).unwrap();
        let c = doc.stroke_counters();
        assert_eq!((c.dabs, c.set_dabs, c.sets_started, c.buffer_copies), (0, 0, 0, 0), "reset by stroke_begin");
        doc.stroke_to_3d(&h, 1.0);
        assert_eq!(doc.stroke_counters().buffer_copies, 0, "nothing shares the buffers now");
        doc.stroke_cancel();
        assert!(!stroke_probe_on());
    }

    /// Check-and-fix of round 5: while a paced segment is open, undo, redo and a second `stroke_begin` are refused
    /// and leave the segment alone; the stroke is then resumed, ended, undone and redone like the unpaced one.
    #[test]
    fn undo_and_redo_wait_for_an_open_segment() {
        let mut plain = doc_from_raw(samples::seam_cube(1.0), 128);
        let path = cube_path(&plain);
        let before = doc_state(&mut plain);
        plain.stroke_begin(&cube_brush()).unwrap();
        path[..4].iter().for_each(|h| plain.stroke_to_3d(h, 1.0));
        assert!(plain.stroke_end());
        let want = doc_state(&mut plain);

        let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
        doc.stroke_begin(&cube_brush()).unwrap();
        path[..3].iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
        assert_eq!(doc.stroke_to_3d_paced(&path[3], 1.0, &mut || false), StrokeStep::Partial);
        let dabs = doc.stroke_counters().dabs;
        assert_eq!(doc.undo(), None, "undo waits for the stroke");
        assert_eq!(doc.redo(), None, "redo waits for the stroke");
        assert!(doc.stroke_begin(&cube_brush()).is_err(), "a second stroke is refused");
        assert_eq!(doc.stroke_counters().dabs, dabs, "the refused calls painted nothing");
        let mut resumes = 0;
        while doc.stroke_resume(&mut || false) == StrokeStep::Partial {
            resumes += 1;
        }
        assert!(resumes > 0, "the segment had more than two dabs left");
        assert!(doc.stroke_end());
        assert_eq!(doc_state(&mut doc), want);
        assert_eq!(doc.undo().as_deref(), Some("Brush Stroke"));
        assert_eq!(doc_state(&mut doc), before);
        assert!(doc.redo().is_some());
        assert_eq!(doc_state(&mut doc), want);
    }

    // ----- Round 7 (brush speed) -----

    /// T5: whole strokes over two sets (the cube path over the body's seams, then across the head and the body,
    /// small and large ball brushes and a Cube tip) paint round 5's pixels: the `RASTER_R5` seam on and off, the probe
    /// on and off, give the same document state.
    #[test]
    fn a_stroke_paints_round5s_pixels() {
        use crate::brush_tip::TipVolume;
        let cube = Arc::new(crate::tip_field::TipField::cube());
        let paint = |r5: bool, probe: bool, full_reapply: bool| {
            let mut doc = doc_from_raw(samples::two_material_model(), 256);
            let mut path = cube_path(&doc);
            path.extend((0..10).map(|i| hit(&doc, Vec3::new(-0.4 + 0.09 * i as f32, 3.0, 0.03), Vec3::NEG_Y)));
            let _probe = PROBE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
            set_stroke_probe(probe);
            brush::RASTER_R5.with(|r| r.set(r5));
            FULL_REAPPLY.with(|f| f.set(full_reapply));
            let field = BrushTip { volume: TipVolume::Field(Arc::clone(&cube)), ..BrushTip::legacy(false) };
            for (radius, hardness, tip, color) in [
                (0.05, 0.7, BrushTip::legacy(true), [10, 200, 30, 255]),
                (0.4, 0.5, BrushTip::legacy(false), [200, 20, 30, 255]),
                (0.25, 0.8, field.clone(), [20, 40, 220, 255]),
                (0.03, 1.0, field, [240, 240, 10, 255]),
            ] {
                let b = BrushParams { radius, hardness, color, ..BrushParams::default() };
                doc.stroke_begin_with(&b, &tip).unwrap();
                path.iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
                assert!(doc.stroke_end(), "radius {radius}: the stroke painted nothing");
            }
            brush::RASTER_R5.with(|r| r.set(false));
            FULL_REAPPLY.with(|f| f.set(false));
            set_stroke_probe(false);
            doc_state(&mut doc)
        };
        // Round 5 = its rasterizer and its whole-rect re-apply.
        let want = paint(true, false, true);
        for (r5, probe, full) in [(false, false, false), (false, true, false), (true, true, false), (false, false, true)] {
            assert!(paint(r5, probe, full) == want, "seam {r5}, probe {probe}, full re-apply {full}: the pixels differ from round 5's");
        }
    }

    /// Check agent (round 7): strokes over a soup of six sets (a large brush reaches every set in one dab), paced one
    /// dab per call, a stroke cancelled with a segment still open, a 2D stroke between 3D strokes (A4's spans after a
    /// 2D dab), tiny and huge ball and Cube brushes: the document state equals round 5's (its rasterizer and its
    /// whole-rect re-apply).
    #[test]
    fn check_r7_many_sets_cancel_and_2d_strokes_paint_round5s_pixels() {
        use crate::brush_tip::TipVolume;
        let cube = Arc::new(crate::tip_field::TipField::cube());
        let paint = |r5: bool| {
            brush::RASTER_R5.with(|r| r.set(r5));
            FULL_REAPPLY.with(|f| f.set(r5));
            let mut doc = doc_from_raw(samples::soup_sets(5, 3000, 6), 128);
            let path: Vec<Hit> = (0..40)
                .filter_map(|i| {
                    let z = 0.5 + 0.25 * (i as f32 * 0.7).sin();
                    doc.raycast(&Ray::new(Vec3::new(0.06 + 0.022 * i as f32, 2.0, z), Vec3::NEG_Y))
                })
                .collect();
            assert!(path.len() >= 20, "only {} hits", path.len());
            let field = BrushTip { volume: TipVolume::Field(Arc::clone(&cube)), ..BrushTip::legacy(false) };
            let strokes = [
                (0.02, 0.7, BrushTip::legacy(true), [10, 200, 30, 255]),
                (0.6, 0.4, BrushTip::legacy(false), [200, 20, 30, 200]),
                (0.15, 0.9, field.clone(), [20, 40, 220, 255]),
                (0.45, 0.6, BrushTip::legacy(false), [90, 90, 10, 255]),
                (0.004, 1.0, field, [240, 240, 10, 255]),
            ];
            for (k, (radius, hardness, tip, color)) in strokes.into_iter().enumerate() {
                let b = BrushParams { radius, hardness, color, ..BrushParams::default() };
                doc.stroke_begin_with(&b, &tip).unwrap();
                for (i, h) in path.iter().enumerate() {
                    let pressure = 0.3 + 0.7 * ((i % 5) as f32 / 4.0);
                    if k == 3 && i == path.len() / 2 {
                        // Cancelled mid-stroke, with the open segment's dabs still pending.
                        doc.stroke_to_3d_paced(h, pressure, &mut || false);
                        break;
                    }
                    doc.stroke_to_3d_paced(h, pressure, &mut || i % 3 != 0);
                }
                if k == 1 {
                    assert!(doc.stroke_counters().sets_started >= 5, "the large brush reached few sets");
                }
                if k == 3 {
                    doc.stroke_cancel();
                } else {
                    assert!(doc.stroke_end(), "stroke {k} painted nothing");
                }
                if k == 0 || k == 2 {
                    // A 2D stroke on set 2 between the 3D strokes.
                    let b = BrushParams { radius: 6.0, hardness: 0.5, color: [5, 5, 250, 255], ..BrushParams::default() };
                    doc.stroke_begin(&b).unwrap();
                    for j in 0..12 {
                        doc.stroke_to_2d(TextureSetId(2), Vec2::new(10.0 + 9.0 * j as f32, 20.0 + 7.0 * j as f32), 1.0);
                    }
                    assert!(doc.stroke_end(), "the 2D stroke painted nothing");
                }
            }
            brush::RASTER_R5.with(|r| r.set(false));
            FULL_REAPPLY.with(|f| f.set(false));
            doc_state(&mut doc)
        };
        let want = paint(true);
        assert!(paint(false) == want, "the pixels differ from round 5's");
    }

    // ----- Branch `pen-input`, part A: pressure dynamics (PEN_INPUT_PLAN 2.4, ARCHITECTURE 9.1) -----

    use crate::brush::PressureDynamics;

    /// Size and flow dynamics of a pen test.
    fn pen_dyn(size: bool, size_min: f32, flow: bool, flow_min: f32) -> PressureDynamics {
        PressureDynamics { size, size_min, flow, flow_min }
    }

    /// Rows of column `x` whose pixel passes `hit` (the painted height of a horizontal stroke).
    fn column_height(doc: &Document, layer: LayerId, x: u32, size: u32, hit: impl Fn([u8; 4]) -> bool) -> u32 {
        (0..size).filter(|&y| hit(px(doc, layer, x, y))).count() as u32
    }

    /// A straight 2D stroke from x0 to x1 at row y in `n + 1` samples, pressure ramping from p0 to p1.
    fn ramp_2d(doc: &mut Document, y: f32, (x0, x1): (f32, f32), (p0, p1): (f32, f32), n: u32) {
        for i in 0..=n {
            let t = i as f32 / n as f32;
            doc.stroke_to_2d(S0, Vec2::new(x0 + (x1 - x0) * t, y), p0 + (p1 - p0) * t);
        }
    }

    /// PEN 1: both factors as a table; exactly 1.0 at full pressure, `p` itself for `flow_min == 0`, monotonic.
    #[test]
    fn size_and_flow_factors() {
        for &m in &[0.0f32, 0.1, 0.3, 0.5, 1.0] {
            let d = pen_dyn(true, m, true, m);
            assert_eq!(d.size_factor(1.0).to_bits(), 1.0f32.to_bits(), "size at p = 1, min {m}");
            assert_eq!(d.flow_factor(1.0).to_bits(), 1.0f32.to_bits(), "flow at p = 1, min {m}");
            assert_eq!(d.size_factor(1.5), 1.0);
            assert!((d.size_factor(0.0) - m).abs() < 1e-6, "size at p = 0 is the minimum");
            assert!((d.flow_factor(0.0) - m).abs() < 1e-6, "flow at p = 0 is the minimum");
            let (mut ps, mut pf) = (-1.0f32, -1.0f32);
            for i in 0..=100 {
                let p = i as f32 / 100.0;
                let (s, f) = (d.size_factor(p), d.flow_factor(p));
                assert!(s >= ps && f >= pf, "not monotonic at {p}, min {m}");
                assert!((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&f));
                (ps, pf) = (s, f);
            }
        }
        let d = pen_dyn(false, 0.0, true, 0.0);
        for i in 0..=1000 {
            let p = i as f32 / 1000.0;
            assert_eq!(d.flow_factor(p).to_bits(), p.to_bits(), "flow_min 0 must give p itself");
            assert_eq!(d.size_factor(p), 1.0, "size off");
            assert_eq!(PressureDynamics::OFF.size_factor(p), 1.0);
            assert_eq!(PressureDynamics::OFF.flow_factor(p), 1.0);
        }
        assert_eq!(PressureDynamics::default(), pen_dyn(false, 0.0, true, 0.0), "the core default is round 3's");
        assert_eq!(BrushParams::default().dynamics, PressureDynamics::default());
        // size 0.3 at p 0.5: 1 - 0.7 * 0.5; flow 0.1 at p 0.5: 0.1 + 0.9 * 0.5.
        assert!((pen_dyn(true, 0.3, true, 0.1).size_factor(0.5) - 0.65).abs() < 1e-6);
        assert!((pen_dyn(true, 0.3, true, 0.1).flow_factor(0.5) - 0.55).abs() < 1e-6);
    }

    /// PEN 2 (mouse identity): at pressure 1.0 any dynamics paint exactly what the default paints, 3D and 2D, with
    /// the same dab count.
    #[test]
    fn full_pressure_ignores_the_dynamics() {
        let three_d = |d: PressureDynamics| {
            let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
            let path = cube_path(&doc);
            doc.stroke_begin(&BrushParams { dynamics: d, ..cube_brush() }).unwrap();
            path.iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
            let dabs = doc.stroke_counters().dabs;
            assert!(doc.stroke_end());
            (dabs, doc_state(&mut doc))
        };
        let two_d = |d: PressureDynamics| {
            let (mut doc, _) = transparent_doc(128);
            let b = BrushParams { radius: 9.0, hardness: 0.6, flow: 0.4, dynamics: d, ..BrushParams::default() };
            doc.stroke_begin(&b).unwrap();
            ramp_2d(&mut doc, 40.0, (10.0, 117.0), (1.0, 1.0), 13);
            ramp_2d(&mut doc, 90.0, (117.0, 20.0), (1.0, 1.0), 7);
            let dabs = doc.stroke_counters().dabs;
            assert!(doc.stroke_end());
            (dabs, doc_state(&mut doc))
        };
        let (want3, want2) = (three_d(PressureDynamics::default()), two_d(PressureDynamics::default()));
        let all = [pen_dyn(true, 0.1, true, 0.2), pen_dyn(true, 0.0, false, 0.0), PressureDynamics::OFF, pen_dyn(false, 0.5, true, 0.5)];
        for d in all {
            assert!(three_d(d) == want3, "3D differs at pressure 1 with {d:?}");
            assert!(two_d(d) == want2, "2D differs at pressure 1 with {d:?}");
        }
    }

    /// PEN 4: size dynamics make a 2D stroke as tall as the pressure says; with flow off the centre is full.
    #[test]
    fn pressure_sizes_the_stroke_2d() {
        let (mut doc, layer) = transparent_doc(256);
        let b = BrushParams { radius: 20.0, hardness: 1.0, dynamics: pen_dyn(true, 0.0, false, 0.0), ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        let (x0, x1) = (20.0f32, 236.0f32);
        ramp_2d(&mut doc, 128.0, (x0, x1), (0.1, 1.0), 54);
        assert!(doc.stroke_end());
        for frac in [0.2f32, 0.9] {
            let x = x0 + (x1 - x0) * frac;
            let want = 2.0 * 20.0 * (0.1 + 0.9 * frac);
            let h = column_height(&doc, layer, x as u32, 256, |c| c[3] >= 128) as f32;
            assert!((h - want).abs() <= 2.0, "column at {frac}: height {h}, want {want}");
            assert_eq!(px(&doc, layer, x as u32, 128)[3], 255, "flow off: the centre is full at {frac}");
        }
    }

    /// PEN 5: at constant pressure 0.25 the dabs are a quarter of the full-size step apart.
    #[test]
    fn pressure_spacing_follows_the_radius() {
        let dabs_at = |p: f32| {
            let (mut doc, _) = transparent_doc(256);
            let b = BrushParams { radius: 20.0, flow: 0.1, dynamics: pen_dyn(true, 0.0, false, 0.0), ..BrushParams::default() };
            doc.stroke_begin(&b).unwrap();
            doc.stroke_to_2d(S0, Vec2::new(20.0, 100.0), p);
            let first = doc.stroke_counters().dabs;
            doc.stroke_to_2d(S0, Vec2::new(220.0, 100.0), p);
            let n = doc.stroke_counters().dabs - first;
            doc.stroke_cancel();
            n as f32
        };
        let step = 0.12f32 * 2.0 * 20.0 * 0.25;
        let (quarter, full) = (dabs_at(0.25), dabs_at(1.0));
        assert!((quarter - (200.0f32 / step).floor()).abs() <= 1.0, "{quarter} dabs at 0.25, step {step}");
        assert!((quarter / full - 4.0).abs() < 0.2, "{quarter} vs {full} dabs");
    }

    /// Pressures of the pen tests' cube path, thin and full mixed.
    fn pen_pressures(n: usize) -> Vec<f32> {
        (0..n).map(|i| [0.2f32, 0.55, 1.0, 0.35, 0.8, 0.1][i % 6]).collect()
    }

    /// What a pressure-sized 3D stroke along the cube path left: dabs, partial calls, masks, document.
    type PenRun = (u64, u32, Vec<Vec<u16>>, crate::history::testkit::DocState);

    /// A pressure-sized 3D stroke along the cube path, paced by `more(k)` (`None` = unpaced).
    fn paced_pen_stroke(more: Option<&dyn Fn(usize) -> bool>) -> PenRun {
        let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
        let path = cube_path(&doc);
        let pressures = pen_pressures(path.len());
        doc.stroke_begin(&BrushParams { dynamics: pen_dyn(true, 0.1, true, 0.0), ..cube_brush() }).unwrap();
        let mut partials = 0;
        let mut calls = 0usize;
        for (h, &p) in path.iter().zip(&pressures) {
            match more {
                None => doc.stroke_to_3d(h, p),
                Some(more) => {
                    let mut ask = || {
                        calls += 1;
                        more(calls)
                    };
                    let mut step = doc.stroke_to_3d_paced(h, p, &mut ask);
                    while step == StrokeStep::Partial {
                        partials += 1;
                        step = doc.stroke_resume(&mut ask);
                    }
                }
            }
        }
        let (dabs, masks) = (doc.stroke_counters().dabs, all_masks(&doc));
        assert!(doc.stroke_end());
        (dabs, partials, masks, doc_state(&mut doc))
    }

    /// PEN 6: the dabs of a pressure-sized 3D stroke do not depend on the pacing.
    #[test]
    fn pressure_sized_stroke_is_pacing_independent() {
        let (dabs, _, masks, want) = paced_pen_stroke(None);
        let never: &dyn Fn(usize) -> bool = &|_| false;
        let mixed: &dyn Fn(usize) -> bool = &|k| k % 3 != 0;
        for (name, more) in [("one dab per call", never), ("mixed", mixed)] {
            let (d, partials, m, state) = paced_pen_stroke(Some(more));
            assert!(partials > 5, "{name}: the segments were not split ({partials})");
            assert_eq!(d, dabs, "{name}: dab count");
            assert!(m == masks, "{name}: masks differ");
            assert!(state == want, "{name}: pixels differ");
        }
        // The walk really ran: thin dabs are closer together than the constant step.
        let mut doc = doc_from_raw(samples::seam_cube(1.0), 128);
        let path = cube_path(&doc);
        doc.stroke_begin(&cube_brush()).unwrap();
        path.iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
        assert!(doc.stroke_counters().dabs < dabs, "thin dabs are closer together");
        doc.stroke_cancel();
    }

    /// PEN 7: the same pressure-sized stroke inside pools of 1 and 3 threads gives the same pixels.
    #[test]
    fn pressure_sized_stroke_is_thread_independent() {
        let paint = || {
            let mut doc = doc_from_raw(samples::seam_cube(1.0), 256);
            let path = cube_path(&doc);
            let pressures = pen_pressures(path.len());
            let b = BrushParams { radius: 0.4, hardness: 0.5, dynamics: pen_dyn(true, 0.2, true, 0.3), ..cube_brush() };
            doc.stroke_begin(&b).unwrap();
            path.iter().zip(&pressures).for_each(|(h, &p)| doc.stroke_to_3d(h, p));
            assert!(doc.stroke_end());
            doc_state(&mut doc)
        };
        let pools = [1usize, 3].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
        let one = pools[0].install(paint);
        let three = pools[1].install(paint);
        assert!(one == three, "the pressure-sized stroke depends on the thread count");
    }

    /// PEN 8: an eraser with size dynamics erases a band as tall as the pressure says.
    #[test]
    fn eraser_follows_pressure_size() {
        let mut doc = doc_from_raw(samples::flat_quad(), 256);
        let layer = LayerId(1);
        let b = BrushParams {
            radius: 20.0,
            hardness: 1.0,
            mode: BrushMode::Erase,
            dynamics: pen_dyn(true, 0.0, false, 0.0),
            ..BrushParams::default()
        };
        doc.stroke_begin(&b).unwrap();
        let (x0, x1) = (20.0f32, 236.0f32);
        ramp_2d(&mut doc, 128.0, (x0, x1), (0.1, 1.0), 54);
        assert!(doc.stroke_end());
        assert_eq!(doc.undo_label(), Some("Erase"));
        for frac in [0.2f32, 0.9] {
            let x = x0 + (x1 - x0) * frac;
            let want = 2.0 * 20.0 * (0.1 + 0.9 * frac);
            let h = column_height(&doc, layer, x as u32, 256, |c| c[3] < 128) as f32;
            assert!((h - want).abs() <= 2.0, "erased column at {frac}: height {h}, want {want}");
            assert_eq!(px(&doc, layer, x as u32, 128)[3], 0, "the centre is erased at {frac}");
        }
    }

    /// PEN 9: NaN and out-of-range minimums behave as clamped (NaN -> 0).
    #[test]
    fn flow_min_and_size_min_are_sanitized() {
        let paint = |d: PressureDynamics| {
            let (mut doc, _) = transparent_doc(64);
            doc.stroke_begin(&BrushParams { radius: 8.0, hardness: 0.7, dynamics: d, ..BrushParams::default() }).unwrap();
            ramp_2d(&mut doc, 32.0, (8.0, 56.0), (0.2, 0.6), 6);
            let dabs = doc.stroke_counters().dabs;
            assert!(doc.stroke_end());
            (dabs, doc_state(&mut doc))
        };
        assert!(paint(pen_dyn(true, f32::NAN, true, 7.0)) == paint(pen_dyn(true, 0.0, true, 1.0)));
        assert!(paint(pen_dyn(true, -3.0, true, f32::NAN)) == paint(pen_dyn(true, 0.0, true, 0.0)));
        assert!(paint(pen_dyn(true, 9.0, true, -1.0)) == paint(pen_dyn(true, 1.0, true, 0.0)));
        assert_eq!(pen_dyn(true, f32::INFINITY, false, f32::NEG_INFINITY).sanitized(), pen_dyn(true, 1.0, false, 0.0));
    }

    /// PEN 10: a tiny pressure over a huge segment hits the dab guard and paints one dab at the sample.
    #[test]
    fn too_many_pressure_dabs_fall_back_to_one() {
        let (mut doc, _) = transparent_doc(64);
        let b = BrushParams { radius: 1.0, dynamics: pen_dyn(true, 0.0, true, 0.0), ..BrushParams::default() };
        doc.stroke_begin(&b).unwrap();
        doc.stroke_to_2d(S0, Vec2::new(0.0, 10.0), 0.01);
        let t = std::time::Instant::now();
        doc.stroke_to_2d(S0, Vec2::new(60_000.0, 10.0), 0.01);
        assert_eq!(doc.stroke_counters().dabs, 2, "one dab at the far sample");
        assert!(t.elapsed().as_secs() < 5, "the guard did not stop the walk");
        // The next segment starts at the far sample with residual 0: a short one is walked at the 0.5 px floor.
        doc.stroke_to_2d(S0, Vec2::new(60_010.0, 10.0), 0.01);
        assert_eq!(doc.stroke_counters().dabs, 2 + 20, "10 px at a 0.5 px step");
        doc.stroke_cancel();
    }
}
