// Owner: core-paint (C3). Brush engine (ARCHITECTURE.md 5.19, algorithm 6.4): the brush profile, the stroke-to-layer
// kernel, the per-set stroke mask and the two dab rasterizers (spherical dab on the surface, disc dab in texel space).
//
// A dab only ever raises the stroke mask (`StrokeScratch`); turning the mask into layer pixels is the job of
// `stroke.rs`. Within one dab every texel takes the maximum coverage of all fragments that reach it, so shared
// edges, overlapping and mirrored UVs are idempotent.

//
// Round 4 (FEATURE_BRUSH 5.2 to 5.6): a 3D dab of an open stroke runs one sphere query for all texture sets
// (`shared_candidates`) and then, per set, `dab_tip`, which rasterizes either the ball (today's code, bit for bit) or
// a shape field (`TipField`) with the coverage `falloff(thick + sd, thick, hardness, aa)` of 5.2. `dab_3d` is kept
// unchanged as the reference of the shared path (test C2).

use std::collections::BTreeMap;

use glam::{Mat3, Vec2, Vec3};
use rayon::prelude::*;

use crate::brush_tip::SurfaceReach;
use crate::bvh::Bvh;
use crate::history::TilePatch;
use crate::ids::{TextureSetId, TriId};
use crate::math::{IRect, smoothstep};
use crate::mesh::MeshData;
use crate::texel_map::{Fragment, Pass2Cost, TexelMap, raster_box, raster_tri_covered_lean, raster_tri_lean};
use crate::tip_field::TipField;

/// What a brush stroke does to the layer.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BrushMode {
    Paint,
    Erase,
}

/// Parameters of one brush stroke.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct BrushParams {
    /// World units for a 3D stroke, texels for a 2D stroke.
    pub radius: f32,
    /// 0..=1.
    pub hardness: f32,
    /// Stroke ceiling 0..=1.
    pub opacity: f32,
    /// Per-dab build-up 0..=1.
    pub flow: f32,
    /// Fraction of the diameter between dabs; clamped to 0.02..=1; default 0.12.
    pub spacing: f32,
    /// Straight sRGB; alpha scales the strength.
    pub color: [u8; 4],
    pub mode: BrushMode,
    /// 3D: skip triangles whose geometric normal opposes the stroke's reference normal (default true).
    pub same_side_only: bool,
    /// How pen pressure drives the stroke (branch `pen-input`); the default is round 3's "flow follows pressure".
    pub dynamics: PressureDynamics,
}

/// How pen pressure (0..=1) drives a stroke (branch `pen-input`, ARCHITECTURE.md 5.19). Default = round 3 behaviour:
/// flow follows pressure, size fixed. Both factors are exactly 1.0 at pressure 1.0, so a mouse (always 1.0) paints the
/// same whatever the dynamics.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct PressureDynamics {
    /// The radius follows the pressure.
    pub size: bool,
    /// 0..=1: fraction of the radius at pressure 0.
    pub size_min: f32,
    /// The per-dab strength follows the pressure.
    pub flow: bool,
    /// 0..=1: fraction of the flow at pressure 0.
    pub flow_min: f32,
}

impl Default for PressureDynamics {
    /// Size fixed, flow follows pressure from 0 (round 3).
    fn default() -> PressureDynamics {
        PressureDynamics { size: false, size_min: 0.0, flow: true, flow_min: 0.0 }
    }
}

impl PressureDynamics {
    /// Pressure changes nothing.
    pub const OFF: PressureDynamics = PressureDynamics { size: false, size_min: 0.0, flow: false, flow_min: 0.0 };

    /// The radius multiplier at `pressure`: exactly 1.0 when `!size` or `pressure >= 1.0`, else
    /// `1 - (1 - size_min) * (1 - pressure)`.
    #[inline]
    pub fn size_factor(&self, pressure: f32) -> f32 {
        if !self.size || pressure >= 1.0 { 1.0 } else { 1.0 - (1.0 - self.size_min) * (1.0 - pressure) }
    }

    /// The flow multiplier at `pressure`: exactly 1.0 when `!flow` or `pressure >= 1.0`; `pressure` itself (bit for
    /// bit) when `flow_min == 0`, else `flow_min + (1 - flow_min) * pressure`.
    #[inline]
    pub fn flow_factor(&self, pressure: f32) -> f32 {
        if !self.flow || pressure >= 1.0 {
            1.0
        } else if self.flow_min == 0.0 {
            pressure
        } else {
            self.flow_min + (1.0 - self.flow_min) * pressure
        }
    }

    /// Both minimums clamped to 0..=1 (NaN -> 0).
    pub fn sanitized(&self) -> PressureDynamics {
        PressureDynamics { size_min: unit(self.size_min), flow_min: unit(self.flow_min), ..*self }
    }
}

impl Default for BrushParams {
    /// Radius 16, hardness 0.8, opacity 1, flow 1, spacing 0.12, black, Paint, same side only, default dynamics.
    fn default() -> BrushParams {
        BrushParams {
            radius: 16.0,
            hardness: 0.8,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.12,
            color: [0, 0, 0, 255],
            mode: BrushMode::Paint,
            same_side_only: true,
            dynamics: PressureDynamics::default(),
        }
    }
}

/// Triangles whose geometric normal has a dot product below this with the reference normal are on the other side.
const SAME_SIDE_MIN_DOT: f32 = -0.2;
/// A triangle's own texel size counts at most this many times the set's mean (guards slivers with a tiny UV area).
const MAX_LOCAL_AA: f32 = 4.0;
/// Rows per band of the parallel pattern (section 6.3).
const BAND_ROWS: u32 = 64;
/// Candidates whose clip box holds at least this many texels have the core blocks of their box folded.
const FOLD_MIN_TEXELS: f32 = 64.0 * 64.0;
/// Dabs that may visit at least this many texels are rasterized on the rayon pool.
const PARALLEL_TEXELS: u64 = 1 << 16;
/// Round 7 A2: a 3D dab whose candidates' scan boxes hold fewer texels than this is rasterized and committed serially;
/// from it on, by the fused parallel pass over `DAB_BAND_ROWS`-row bands.
const PAR_DAB_WORK: u64 = 16_384;
/// Round 7 A2: rows per band of the fused pass. A constant, never derived from the thread count: the partition depends
/// only on the dab.
const DAB_BAND_ROWS: u32 = 32;
/// Round 7 A2 (set after the bench): a dab with fewer candidates than this is also serial up to `4 * PAR_DAB_WORK`
/// texels of work. A few large triangles (tex_8192 / tex_4096 at 50 texels: 3-4 candidates, 1,300 touched texels in
/// about 20,000 texels of boxes) are cheaper on the calling thread than the pool's wake-up; many small candidates (a
/// heavy model's dab) gain from the bands at the same work.
const PAR_DAB_MIN_CANDS: usize = 8;
/// Round 7 A2: a set's candidate list of at least this many triangles is prepared on the pool.
const PAR_PREPARE_TRIS: usize = 4096;
/// Round 7 A3: a shared query that found at least this many triangles runs its two passes over them on the pool, in
/// chunks of `QUERY_CHUNK` (a constant: the chunks depend only on the query).
const PAR_QUERY_TRIS: usize = 32_768;
const QUERY_CHUNK: usize = 8192;
/// Largest change of a field's distance (`F`, `G`) per unit of distance moved, with margin: neighbouring nodes
/// differ by at most one cell (+ 1/256 of quantization), so a trilinear blend changes by at most `sqrt(3) * 1.004`
/// (FEATURE_BRUSH 5.6). Only the full-strength fast paths use it; a larger value only folds less.
const FIELD_LIPSCHITZ: f32 = 1.8;

/// The shape of one 3D dab as the rasterizer sees it (FEATURE_BRUSH 5.1).
#[derive(Clone, Copy)]
pub(crate) struct DabShape<'a> {
    /// The tip centre `c'` (the anchor moved by the offset).
    pub center: Vec3,
    /// `R_b`: radius of a ball around `center` that holds the whole tip. The ball's radius `r` for the ball.
    pub reach: f32,
    /// `None` for the ball.
    pub field: Option<FieldDab<'a>>,
}

/// A field tip placed for one dab.
#[derive(Clone, Copy)]
pub(crate) struct FieldDab<'a> {
    pub field: &'a TipField,
    /// Tip from world: `q = to_tip * (P - c')` (= `R^T / k`).
    pub to_tip: Mat3,
    /// World length of one tip unit.
    pub k: f32,
    /// The outline of the middle slice pressed along tip Y (Through), else the volume (Solid).
    pub through: bool,
    /// Per-stroke data of the tip (`TipGeom`).
    pub geom: TipGeom,
}

/// What a dab needs to know about a field, computed once per stroke (tip units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TipGeom {
    /// Largest depth inside the volume (Solid) or the outline (Through): `depth_max / k` of FEATURE_BRUSH 5.1.
    pub depth: f32,
    /// At least every value the thickness lookup (`T` or `T2`) can return; the fade-zone lookup is skipped where
    /// the profile is 1 for every thickness up to this value.
    pub thick_max: f32,
    /// The box of the early reject of 5.6: the field's `bounds()` (Solid), or the outline's box in x and z and
    /// `-bound..bound` in y (Through).
    pub box_min: Vec3,
    pub box_max: Vec3,
    /// Test seam of C23: the thickness is replaced by the inradius (the defect the local thickness guards against).
    pub inradius_thickness: bool,
}

impl TipGeom {
    /// The per-stroke data of `field` used Solid (`through == false`) or Through.
    pub(crate) fn new(field: &TipField, through: bool) -> TipGeom {
        let cell = field.cell();
        let n = |a: f32, b: f32| if cell > 0.0 { (((b - a) / cell).round().max(0.0) as u32).min(4096) } else { 0 };
        if !through {
            let b = field.bounds();
            // A closed part cut off at the border of its grid (FEATURE_BRUSH 5.6) has negative border values, and its
            // zero level lies up to that depth beyond the grid, which can be outside `bounds()`. The box's faces lie
            // on node planes (or one cell beyond the grid), so the smallest value on them is found at their nodes;
            // growing the box by its negative part keeps the zero level inside. 0 for every other field.
            let mut face_min = f32::INFINITY;
            let (nx, ny, nz) = (n(b.min.x, b.max.x), n(b.min.y, b.max.y), n(b.min.z, b.max.z));
            let at = |i: u32, j: u32, k: u32| b.min + Vec3::new(i as f32, j as f32, k as f32) * cell;
            for j in 0..=ny {
                for i in 0..=nx {
                    face_min = face_min.min(field.sample(at(i, j, 0))).min(field.sample(at(i, j, nz)));
                }
                for k in 0..=nz {
                    face_min = face_min.min(field.sample(at(0, j, k))).min(field.sample(at(nx, j, k)));
                }
            }
            for k in 0..=nz {
                for i in 0..=nx {
                    face_min = face_min.min(field.sample(at(i, 0, k))).min(field.sample(at(i, ny, k)));
                }
            }
            let grow = Vec3::splat(if face_min < 0.0 { -face_min } else { 0.0 });
            return TipGeom {
                depth: field.inradius(),
                // Thickness nodes are depths of inside nodes (at most the inradius), stored in half cells.
                thick_max: field.inradius() + 0.5 * cell,
                box_min: b.min - grow,
                box_max: b.max + grow,
                inradius_thickness: false,
            };
        }
        // The outline: scan its nodes for the largest depth and thickness (the slice uses the field's cell, and its
        // box is made of nodes grown by one cell), and for the smallest value on the box's edge (an outline cut at
        // the border of its grid, as above).
        let [mut lo, mut hi] = field.outline_bounds();
        let (mut depth, mut thick, mut edge_min) = (0.0f32, 0.0f32, f32::INFINITY);
        if hi.x > lo.x && cell > 0.0 {
            let (ni, nj) = (n(lo.x, hi.x), n(lo.y, hi.y));
            for j in 0..=nj {
                for i in 0..=ni {
                    let (x, z) = (lo.x + i as f32 * cell, lo.y + j as f32 * cell);
                    let v = field.outline(x, z);
                    depth = depth.max(-v);
                    thick = thick.max(field.outline_thickness(x, z));
                    if i == 0 || j == 0 || i == ni || j == nj {
                        edge_min = edge_min.min(v);
                    }
                }
            }
        }
        if edge_min < 0.0 {
            (lo, hi) = (lo - Vec2::splat(-edge_min), hi + Vec2::splat(-edge_min));
        }
        // Through paints the prism up to the ring's ball of radius 1 (tip units), also where the field's bound is
        // smaller (a flat outline whose farthest node lies under 1).
        let reach = through_bound(field);
        TipGeom {
            depth,
            thick_max: thick.max(depth) + 0.5 * cell,
            box_min: Vec3::new(lo.x, -reach, lo.y),
            box_max: Vec3::new(hi.x, reach, hi.y),
            inradius_thickness: false,
        }
    }
}

/// `bound` of a field used Through: the prism is cut by the ring's ball of radius 1, which must lie inside the
/// bounding ball (FEATURE_BRUSH 5.6), so at least 1.
fn through_bound(field: &TipField) -> f32 {
    field.bound().max(1.0)
}

/// `R_b` of a field tip with tip unit `k` (FEATURE_BRUSH 5.1): `k * bound`, and for Through at least the ring's ball
/// of radius `k`.
pub(crate) fn field_reach(field: &TipField, k: f32, through: bool) -> f32 {
    k * if through { through_bound(field) } else { field.bound() }
}

impl FieldDab<'_> {
    /// Tip coordinates of a world point.
    #[inline]
    fn tip(&self, p: Vec3, center: Vec3) -> Vec3 {
        self.to_tip * (p - center)
    }

    /// Coverage 0..=1 of the tip at the world point `p` (FEATURE_BRUSH 5.2 and the Through formula of 5.6).
    ///
    /// Solid: a fragment whose tip point lies beyond a face of the early-reject box grown by `0.5 * aa / k` plus one
    /// cell is 0 without a grid lookup. The zero level lies inside `bounds()` (inside nodes grown by one cell) and
    /// `F` grows at least as fast as the distance to that box beyond it, so `sd >= aa / 2` there and the formula
    /// gives 0 too (the triangle reject of 5.6 rests on the same fact with no extra cell; test
    /// `the_fragment_reject_changes_no_coverage` and the oracle C9 pin it). Most of a Solid dab's clip box lies
    /// outside the shape, so this halves the cost of a small Cube dab (P5).
    #[inline]
    fn cover(&self, p: Vec3, center: Vec3, hardness: f32, aa: f32) -> f32 {
        let q = self.tip(p, center);
        // (A flat outline is always used Through; `sanitized` sees to that, and its `sample` ignores `q.y`.)
        if !self.through && !self.field.is_flat() {
            let grow = 0.5 * aa / self.k + self.field.cell();
            let g = &self.geom;
            if q.x < g.box_min.x - grow
                || q.y < g.box_min.y - grow
                || q.z < g.box_min.z - grow
                || q.x > g.box_max.x + grow
                || q.y > g.box_max.y + grow
                || q.z > g.box_max.z + grow
            {
                return 0.0;
            }
        }
        self.cover_at(q, p, center, hardness, aa)
    }

    /// `cover` without the fragment reject: the formula at the tip point `q` of `p`.
    #[inline]
    fn cover_at(&self, q: Vec3, p: Vec3, center: Vec3, hardness: f32, aa: f32) -> f32 {
        let k = self.k;
        let g = &self.geom;
        let fade = |sd: f32| hardness < 1.0 && -sd < (1.0 - hardness) * k * g.thick_max;
        if self.through {
            // The hard limit at the ring's ball: clamp(0.5 - (|P - c'| - k) / aa, 0, 1).
            let ball = ((k - (p - center).length()) / aa + 0.5).clamp(0.0, 1.0);
            if ball.is_nan() || ball <= 0.0 {
                return 0.0;
            }
            let sd = k * self.field.outline(q.x, q.z);
            if sd.is_nan() || sd >= 0.5 * aa {
                return 0.0;
            }
            let thick = if g.inradius_thickness {
                k * g.depth
            } else if fade(sd) {
                k * self.field.outline_thickness(q.x, q.z)
            } else {
                k * g.thick_max
            };
            falloff(thick + sd, thick, hardness, aa) * ball
        } else {
            let sd = k * self.field.sample(q);
            if sd.is_nan() || sd >= 0.5 * aa {
                return 0.0;
            }
            let thick = if g.inradius_thickness {
                k * g.depth
            } else if fade(sd) {
                k * self.field.thickness(q)
            } else {
                k * g.thick_max
            };
            falloff(thick + sd, thick, hardness, aa)
        }
    }

    /// True when every point within `rho` of `g` has coverage exactly 1 (FEATURE_BRUSH 5.6, the two fast paths):
    /// `-sd(g) - L * rho >= d1 * 1.001` with `d1 = max((1 - hardness) * thick_max, aa / 2)`, and for Through also
    /// `|g - c'| + rho <= k - aa / 2` (inside the ring's ball).
    fn full_within(&self, g: Vec3, rho: f32, center: Vec3, hardness: f32, aa: f32) -> bool {
        let k = self.k;
        let d1 = if hardness >= 1.0 { 0.5 * aa } else { ((1.0 - hardness.max(0.0)) * k * self.geom.thick_max).max(0.5 * aa) };
        let q = self.tip(g, center);
        let sd = if self.through {
            let outer = (g - center).length() + rho;
            if outer.is_nan() || outer > (k - 0.5 * aa) * 0.999 {
                return false;
            }
            k * self.field.outline(q.x, q.z)
        } else {
            k * self.field.sample(q)
        };
        // (With the test seam the thickness is the inradius, which is below `thick_max`: the same bound holds.)
        let depth = -sd - FIELD_LIPSCHITZ * rho;
        rho.is_finite() && depth.is_finite() && depth >= d1 * 1.001
    }

    /// True when the triangle with world corners `p` lies wholly outside the early-reject box grown by `grow` tip
    /// units: all three corners beyond the same face (FEATURE_BRUSH 5.6).
    fn rejects(&self, p: [Vec3; 3], center: Vec3, grow: f32) -> bool {
        let q = p.map(|p| self.tip(p, center));
        let (lo, hi) = (self.geom.box_min - Vec3::splat(grow), self.geom.box_max + Vec3::splat(grow));
        (0..3).any(|a| q.iter().all(|v| v[a] < lo[a]) || q.iter().all(|v| v[a] > hi[a]))
    }
}

/// A value in 0..=1; NaN counts as 0.
#[inline]
pub(crate) fn unit(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// Brush profile (section 6.4): 1 inside hardness*radius, smooth fall towards the rim, times an anti-aliasing ramp of
/// width `aa` centred on the rim. Non-zero for dist < radius + aa/2 (the support radius); there is no cut-off at dist == radius.
///
/// A non-finite `dist` or `radius` gives 0. With `aa <= 0` (or NaN) the ramp is a step at the rim (0.5 exactly on it).
pub fn falloff(dist: f32, radius: f32, hardness: f32, aa: f32) -> f32 {
    if !(dist.is_finite() && radius.is_finite()) {
        return 0.0;
    }
    let dist = dist.max(0.0);
    let ramp = if aa > 0.0 {
        ((radius - dist) / aa + 0.5).clamp(0.0, 1.0)
    } else if dist < radius {
        1.0
    } else if dist == radius {
        0.5
    } else {
        0.0
    };
    if ramp <= 0.0 {
        return 0.0;
    }
    let profile = if hardness >= 1.0 {
        1.0
    } else {
        let hardness = hardness.max(0.0);
        let t = if radius > 0.0 { (dist / radius).min(1.0) } else { 1.0 };
        if t <= hardness { 1.0 } else { 1.0 - smoothstep(0.0, 1.0, (t - hardness) / (1.0 - hardness)) }
    };
    profile * ramp
}

/// Rounds a value in 0..=255 to a byte.
#[inline]
fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 255.0) + 0.5) as u8
}

/// Stroke-to-layer kernel (section 6.4): dst = pre-stroke pixel, strength 0..=1.
///
/// Only the RGB of `color` is used; the caller folds the colour's alpha into `strength`. A strength of 0 returns
/// `dst` unchanged, and so does a paint so weak that the result would still have alpha 0 (the RGB stored under
/// alpha 0 is data, section 4.2).
pub fn apply_brush(dst: [u8; 4], color: [u8; 4], strength: f32, mode: BrushMode, alpha_lock: bool) -> [u8; 4] {
    let s = unit(strength);
    if s <= 0.0 {
        return dst;
    }
    match (mode, alpha_lock) {
        (BrushMode::Erase, true) => dst,
        (BrushMode::Erase, false) => [dst[0], dst[1], dst[2], to_u8(dst[3] as f32 * (1.0 - s))],
        (BrushMode::Paint, true) => {
            let mix = |cd: u8, cs: u8| to_u8(cd as f32 + (cs as f32 - cd as f32) * s);
            [mix(dst[0], color[0]), mix(dst[1], color[1]), mix(dst[2], color[2]), dst[3]]
        }
        (BrushMode::Paint, false) => {
            let ad = dst[3] as f32 / 255.0;
            // Weight of the old colour.
            let keep = ad * (1.0 - s);
            let ao = s + keep;
            let alpha = to_u8(ao * 255.0);
            if alpha == 0 {
                return dst;
            }
            let mix = |cd: u8, cs: u8| to_u8((cs as f32 * s + cd as f32 * keep) / ao);
            [mix(dst[0], color[0]), mix(dst[1], color[1]), mix(dst[2], color[2]), alpha]
        }
    }
}

/// Per-set scratch reused between strokes.
pub struct StrokeScratch {
    width: u32,
    height: u32,
    /// Accumulated stroke coverage, 0..=65535.
    mask: Vec<u16>,
    /// Coverage of the dab being rasterized (max rule); all zero between dabs.
    dab_cov: Vec<u8>,
    /// Texels with a non-zero `dab_cov`; empty between dabs.
    dab_touched: Vec<u32>,
    /// Reused output buffer of the sphere query.
    tris: Vec<TriId>,
    /// Reused candidate list of `dab_tip` (round 5 B-4.1); cleared at the start of every dab.
    cands: Vec<DabTri>,
    /// Round 7 A2, reused per dab: the candidates' scan boxes (`raster_box`) and the CSR binning of the fused pass
    /// (`band_start[b]..band_start[b + 1]` indexes `band_items`, the candidates of band `b`).
    boxes: Vec<[i64; 4]>,
    band_start: Vec<u32>,
    band_items: Vec<u32>,
    /// Round 7 A4: per row, the columns `lo..hi` from the first to the last texel whose mask the last 3D dab's commit
    /// raised in that row (`NO_SPAN` = none). A row the last dab did not commit in may hold an older span: re-applying
    /// it is then extra work, never a different result. `span_rows` lists the rows the last serial commit set.
    row_spans: Vec<(u32, u32)>,
    span_rows: Vec<u32>,
    /// True after a 3D dab's commit (the spans describe it), false after a 2D dab (re-apply its whole rects).
    spans_valid: bool,
}

/// `StrokeScratch::row_spans` of a row without a change.
const NO_SPAN: (u32, u32) = (u32::MAX, 0);

#[cfg(test)]
thread_local! {
    /// Test seam (the A/B of `stroke_bench_tips`): `dab_tip` allocates its candidate list per dab, as round 4 did.
    pub(crate) static FRESH_CANDIDATES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test seam of round 7 (the reference of the equality tests and the bench's A/B): `dab_tip` and `dab_3d` run
    /// round 5's rasterizer and commit (`rasterize_tip`, `commit_dab`: clip-box binning into 64-row bands, full pass 2).
    pub(crate) static RASTER_R5: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True while the round-5 seam is on (tests only; read once per dab on the calling thread).
#[inline]
fn raster_r5() -> bool {
    #[cfg(test)]
    {
        RASTER_R5.with(|r| r.get())
    }
    #[cfg(not(test))]
    {
        false
    }
}

impl StrokeScratch {
    /// Zeroed scratch for a `width x height` set.
    pub fn new(width: u32, height: u32) -> StrokeScratch {
        let n = width as usize * height as usize;
        StrokeScratch {
            width,
            height,
            mask: vec![0; n],
            dab_cov: vec![0; n],
            dab_touched: Vec::new(),
            tris: Vec::new(),
            cands: Vec::new(),
            boxes: Vec::new(),
            band_start: Vec::new(),
            band_items: Vec::new(),
            row_spans: vec![NO_SPAN; height as usize],
            span_rows: Vec::new(),
            spans_valid: false,
        }
    }

    /// Round 7 A4: the columns of row `y` whose mask the last 3D dab may have raised, intersected with `x0..x1`;
    /// `None` = re-apply the whole span (no span information: after a 2D dab). An empty range = nothing changed there.
    #[inline]
    pub(crate) fn changed_span(&self, y: u32, x0: u32, x1: u32) -> Option<(u32, u32)> {
        if !self.spans_valid {
            return None;
        }
        let (lo, hi) = self.row_spans.get(y as usize).copied().unwrap_or(NO_SPAN);
        let (a, b) = (lo.max(x0), hi.min(x1));
        Some(if a < b { (a, b) } else { (x0, x0) })
    }

    /// Accumulated stroke coverage of a texel, 0..=65535 (0 outside the set).
    pub fn mask(&self, x: u32, y: u32) -> u16 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.mask[y as usize * self.width as usize + x as usize]
    }

    /// Zeroes the mask inside `r`.
    pub fn clear_rect(&mut self, r: IRect) {
        let c = r.intersect(&IRect::from_size(self.width, self.height));
        let w = self.width as usize;
        for y in c.y0..c.y1 {
            let row = y as usize * w;
            self.mask[row + c.x0 as usize..row + c.x1 as usize].fill(0);
        }
    }

    /// `(width, height)` of the set this scratch belongs to.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The mask values of the span `x0..x1` of row `y` (which must lie inside the set).
    #[inline]
    pub(crate) fn mask_row(&self, y: u32, x0: u32, x1: u32) -> &[u16] {
        let row = y as usize * self.width as usize;
        &self.mask[row + x0 as usize..row + x1 as usize]
    }
}

/// How much a dab of strength `amount` (flow x pressure x coverage, 0..=1) adds to a mask value that is `remaining`
/// below full. `ceil` guarantees progress for low-flow soft brushes (pitfall 8 of section 6.9).
#[inline]
pub(crate) fn mask_increment(remaining: u16, amount: f32) -> u16 {
    ((remaining as f32 * amount).ceil() as u32).min(remaining as u32) as u16
}

/// Mean world units per texel of a set, as used for the anti-aliasing ramp; 1.0 if the map has no usable value.
fn set_texel_world(map: &TexelMap) -> f32 {
    let aa = map.stats.texel_world;
    if aa.is_finite() && aa > 0.0 { aa } else { 1.0 }
}

/// World units per texel of one triangle (`sqrt(area3d / area_uv_px)`), at most `MAX_LOCAL_AA` times the set's mean
/// `aa`; `aa` itself if the ratio is undefined.
fn tri_texel_world(mesh: &MeshData, map: &TexelMap, tri: TriId, aa: f32) -> f32 {
    let [p0, p1, p2] = mesh.tri_positions(tri);
    let [t0, t1, t2] = mesh.tri_uvs(tri);
    let area3d = 0.5 * (p1 - p0).cross(p2 - p0).length() as f64;
    let area_px = 0.5 * ((t1 - t0).perp_dot(t2 - t0)).abs() as f64 * map.width as f64 * map.height as f64;
    let local = (area3d / area_px).sqrt() as f32;
    if local.is_finite() && local > 0.0 { local.min(MAX_LOCAL_AA * aa) } else { aa }
}

/// Step 1 of the 3D dab: does the triangle belong to the set (which excludes `NO_UV`) and face the stroke's side?
#[inline]
fn is_candidate(mesh: &MeshData, tri: TriId, set: TextureSetId, normal: Vec3, same_side_only: bool) -> bool {
    mesh.tri_set(tri) == Some(set) && (!same_side_only || mesh.geo_normal(tri).dot(normal) >= SAME_SIDE_MIN_DOT)
}

/// True when a dab at `center` finds candidate triangles in `set` (step 1 of the 3D dab, section 6.4). `stroke.rs`
/// used it to decide when a set is first touched by a stroke; since round 4 only the reference loop of the tests
/// does (`shared_candidates` decides the reach).
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn dab_reaches_set(
    mesh: &MeshData,
    bvh: &Bvh,
    map: &TexelMap,
    set: TextureSetId,
    center: Vec3,
    normal: Vec3,
    radius: f32,
    same_side_only: bool,
) -> bool {
    if !(center.is_finite() && radius.is_finite() && radius >= 0.0) {
        return false;
    }
    let mut tris = Vec::new();
    bvh.query_sphere(mesh, center, radius + 0.5 * set_texel_world(map), &mut tris);
    tris.iter().any(|&t| is_candidate(mesh, t, set, normal, same_side_only))
}

/// One candidate triangle of a 3D dab, prepared for rasterization.
#[derive(Clone)]
struct DabTri {
    tri: TriId,
    positions: [Vec3; 3],
    /// Unwrapped pixel-space box that contains every texel the dab can reach on this triangle; `None` = no bound.
    clip: Option<[f32; 4]>,
    /// World units per texel of this triangle: the width of its anti-aliasing ramp.
    aa: f32,
    hardness: f32,
    /// Every inside fragment of the triangle lies in the dab's full-strength core (coverage 255): pass 1 is rasterized
    /// folded (`raster_tri_covered`), one visit per covered texel instead of one per image of a tiled triangle.
    full_core: bool,
    /// For a triangle whose UVs span more than one image (tiled) and that is not `full_core`: the affine map from
    /// unwrapped pixel space to the triangle's plane (`p0 + gx * (x - t0.x) + gy * (y - t0.y)`) and the dab's core
    /// radius (0.1 % inside). The 64 x 64 blocks of its box that lie wholly in the core are rasterized folded.
    blocks: Option<BlockFold>,
    /// How the coverage of a fragment is computed: the ball formula, or the field of the dab's shape.
    field: bool,
}

/// See `DabTri::blocks`.
#[derive(Clone, Copy)]
struct BlockFold {
    t0: Vec2,
    p0: Vec3,
    gx: Vec3,
    gy: Vec3,
    center: Vec3,
    core: f32,
    /// Field tips: the triangle's hardness and anti-aliasing width for `FieldDab::full_within` (unused by the ball).
    hardness: f32,
    aa: f32,
}

impl BlockFold {
    /// Every texel centre of the unwrapped pixel box `[x0, y0, x1, y1)` maps to a point of the plane within the core:
    /// the distance to the centre is convex, so its maximum over the box is at a corner.
    fn in_core(&self, b: [i64; 4]) -> bool {
        let at = |x: i64, y: i64| self.p0 + self.gx * (x as f32 - self.t0.x) + self.gy * (y as f32 - self.t0.y);
        [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])]
            .iter()
            .all(|&(x, y)| at(x, y).distance(self.center) <= self.core)
    }

    /// The field version of `in_core`: the block's four corners mapped to the plane hold every texel centre of the
    /// block (a parallelogram), so the ball around their mean through the farthest corner holds them too.
    fn in_field_core(&self, b: [i64; 4], fd: &FieldDab) -> bool {
        let at = |x: i64, y: i64| self.p0 + self.gx * (x as f32 - self.t0.x) + self.gy * (y as f32 - self.t0.y);
        let corners = [at(b[0], b[1]), at(b[2], b[1]), at(b[0], b[3]), at(b[2], b[3])];
        let g = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
        let rho = corners.iter().map(|c| c.distance(g)).fold(0.0f32, f32::max);
        fd.full_within(g, rho, self.center, self.hardness, self.aa)
    }

    /// The map of a triangle with pixel UVs `t` and positions `p`; `None` when the core is empty or the UVs are
    /// degenerate.
    fn new(t: [Vec2; 3], p: [Vec3; 3], center: Vec3, core: f32) -> Option<BlockFold> {
        let (d1, d2) = (t[1] - t[0], t[2] - t[0]);
        let det = d1.perp_dot(d2);
        if !(core > 0.0 && det.is_finite() && det.abs() > 1e-12) {
            return None;
        }
        let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
        let gx = (e1 * d2.y - e2 * d1.y) / det;
        let gy = (e2 * d1.x - e1 * d2.x) / det;
        (gx.is_finite() && gy.is_finite())
            .then_some(BlockFold { t0: t[0], p0: p[0], gx, gy, center, core: core * 0.999, hardness: 1.0, aa: 0.0 })
    }

    /// The map of a triangle for a field tip (blocks are tested with `in_field_core`); `None` for degenerate UVs.
    fn new_field(t: [Vec2; 3], p: [Vec3; 3], center: Vec3, hardness: f32, aa: f32) -> Option<BlockFold> {
        // `core` only has to be positive for `new`; the field test does not read it.
        BlockFold::new(t, p, center, 1.0).map(|b| BlockFold { core: 0.0, hardness, aa, ..b })
    }
}

/// The full-strength core radius of a dab: `min(hardness * radius, radius - aa / 2)` (the whole radius minus the
/// anti-aliasing ramp for `hardness >= 1`); `falloff` is exactly 1 inside it.
fn core_radius(radius: f32, hardness: f32, aa: f32) -> f32 {
    let ramp_core = radius - 0.5 * aa.max(0.0);
    if hardness >= 1.0 { ramp_core } else { (hardness.max(0.0) * radius).min(ramp_core) }
}

/// True when every fragment `raster_tri` reports as inside the triangle `positions` (barycentrics down to `-slack`)
/// is within the full-strength core of a dab at `center`: `falloff == 1` there, so its coverage is 255. The core
/// radius is `min(hardness * radius, radius - aa / 2)` (the whole radius minus the ramp for `hardness >= 1`); a
/// fragment's distance is at most `max |p_i - c| * (1 + 2 * sum(slack))` (convexity), kept 0.1 % inside the core.
fn in_full_core(positions: [Vec3; 3], slack: [f32; 3], center: Vec3, radius: f32, hardness: f32, aa: f32) -> bool {
    let core = core_radius(radius, hardness, aa);
    let far = positions.iter().map(|p| p.distance(center)).fold(0.0f32, f32::max);
    let bound = far * (1.0 + 2.0 * (slack[0] + slack[1] + slack[2]));
    core > 0.0 && bound.is_finite() && bound <= core * 0.999
}

impl DabTri {
    /// Coverage 0..=255 of the dab at a fragment of this triangle. Distance is measured in 3D.
    #[inline]
    fn coverage(&self, f: &Fragment, center: Vec3, radius: f32) -> u8 {
        let [p0, p1, p2] = self.positions;
        let p = p0 * f.bary[0] + p1 * f.bary[1] + p2 * f.bary[2];
        (falloff((p - center).length(), radius, self.hardness, self.aa) * 255.0 + 0.5) as u8
    }

    /// Coverage 0..=255 of the dab `shape` at a fragment: `coverage` with the shape's centre and radius for a ball
    /// triangle (bit for bit the ball's), the field formula otherwise. (The rasterizer inlines the two cases
    /// separately, `raster_ball` and the field branch of `raster_cand`; the tests use this as the reference.)
    #[cfg(test)]
    fn coverage_in(&self, f: &Fragment, shape: &DabShape) -> u8 {
        match (&shape.field, self.field) {
            (Some(fd), true) => {
                let [p0, p1, p2] = self.positions;
                let p = p0 * f.bary[0] + p1 * f.bary[1] + p2 * f.bary[2];
                (fd.cover(p, shape.center, self.hardness, self.aa) * 255.0 + 0.5) as u8
            }
            _ => self.coverage(f, shape.center, shape.reach),
        }
    }

    /// Number of texels the clip box spans (the whole image without a box) and its wrapped row range
    /// `(first row, row count)`; `None` = every row. (Round 5's binning, the `RASTER_R5` reference.)
    #[cfg(test)]
    fn extent(&self, width: u32, height: u32) -> (u64, Option<(u32, u32)>) {
        let all = (width as u64 * height as u64, None);
        let Some(c) = self.clip else { return all };
        let (x0, x1) = (c[0].floor() as i64, (c[2].floor() as i64).saturating_add(1));
        let (y0, y1) = (c[1].floor() as i64, (c[3].floor() as i64).saturating_add(1));
        let rows = y1.saturating_sub(y0).max(0) as u64;
        let cols = (x1.saturating_sub(x0).max(0) as u64).min(width as u64);
        if rows >= height as u64 {
            return (cols * height as u64, None);
        }
        (cols * rows, Some((y0.rem_euclid(height as i64) as u32, rows as u32)))
    }
}

/// Steps 1 and 2 of the 3D dab: the candidate triangles with their clip boxes.
#[allow(clippy::too_many_arguments)]
fn dab_candidates(
    mesh: &MeshData,
    bvh: &Bvh,
    map: &TexelMap,
    set: TextureSetId,
    center: Vec3,
    normal: Vec3,
    radius: f32,
    hardness: f32,
    same_side_only: bool,
    tris: &mut Vec<TriId>,
) -> Vec<DabTri> {
    let aa = set_texel_world(map);
    bvh.query_sphere(mesh, center, radius + 0.5 * aa, tris);
    // A triangle with coarser texels than the set's mean has a wider anti-aliasing ramp, hence a larger support:
    // widen the query once so that its neighbours of the same kind are not cut off at the rim.
    let local_max = tris
        .iter()
        .filter(|&&t| is_candidate(mesh, t, set, normal, same_side_only))
        .map(|&t| tri_texel_world(mesh, map, t, aa))
        .fold(aa, f32::max);
    if local_max > aa * 1.001 {
        bvh.query_sphere(mesh, center, radius + 0.5 * local_max, tris);
    }

    let shape = DabShape { center, reach: radius, field: None };
    tris.iter()
        .filter(|&&tri| is_candidate(mesh, tri, set, normal, same_side_only))
        .filter_map(|&tri| prepare_tri(mesh, map, tri, &shape, hardness, aa))
        .collect()
}

/// Step 2 of the 3D dab for one candidate triangle of `shape` (`aa` = the set's mean texel size): the clip box, the
/// hardness and the fast-path flags; `None` when the triangle is degenerate or its plane lies outside the support.
/// For the ball this is exactly the per-triangle code of round 3; a field tip uses `R_b` and `c'` in the place of
/// the radius and the centre (FEATURE_BRUSH 5.4 step 7, 5.6).
fn prepare_tri(mesh: &MeshData, map: &TexelMap, tri: TriId, shape: &DabShape, hardness: f32, aa: f32) -> Option<DabTri> {
    let (center, radius) = (shape.center, shape.reach);
    let size = Vec2::new(map.width as f32, map.height as f32);
    let margin = map.gutter as f32 + 1.0;
    let positions = mesh.tri_positions(tri);
    let [p0, p1, p2] = positions;
    let (e1, e2) = (p1 - p0, p2 - p0);
    let n = e1.cross(e2);
    let nn = n.length_squared();
    if !(nn > 0.0 && nn.is_finite()) {
        return None;
    }
    let unit_n = n / nn.sqrt();
    let tri_aa = tri_texel_world(mesh, map, tri, aa);
    // The support radius of this triangle's falloff.
    let support = radius + 0.5 * tri_aa;
    let d = (center - p0).dot(unit_n);
    if d.abs() >= support {
        return None;
    }
    // Radius of the circle in which the support sphere cuts the triangle's plane (pitfall 4: clamp at 0).
    let rho = (support * support - d * d).max(0.0).sqrt();
    // The 3D -> UV-pixel affine map of the triangle: barycentric weights of the projected centre and the
    // gradients of the weights of v1 and v2 (per world unit, in the plane).
    let q = center - unit_n * d - p0;
    let (b1, b2) = (q.cross(e2).dot(n) / nn, e1.cross(q).dot(n) / nn);
    let (g1, g2) = (e2.cross(n) / nn, n.cross(e1) / nn);
    let [t0, t1, t2] = mesh.tri_uvs(tri).map(|uv| uv * size);
    let (d1, d2) = (t1 - t0, t2 - t0);
    let c = t0 + d1 * b1 + d2 * b2;
    let grad_u = g1 * d1.x + g2 * d2.x;
    let grad_v = g1 * d1.y + g2 * d2.y;
    let (hx, hy) = (rho * grad_u.length() + margin, rho * grad_v.length() + margin);
    let clip = [c.x - hx, c.y - hy, c.x + hx, c.y + hy];
    let slack = crate::texel_map::inside_bary_slack(mesh.tri_uvs(tri), map.width, map.height);
    let large = 4.0 * hx * hy >= FOLD_MIN_TEXELS;
    let clip = clip.iter().all(|v| v.is_finite()).then_some(clip);

    // A field tip with a depth (Solid or Through) that the triangle's texels can show; anything smaller is painted
    // as the ball of radius `R_b` at full hardness (FEATURE_BRUSH 5.6 "Tiny brushes").
    let field = shape.field.as_ref().filter(|_| radius >= aa.max(tri_aa));
    if let Some(fd) = field {
        let tri_hardness = if fd.k * fd.geom.depth < tri_aa { 1.0 } else { hardness };
        let full_core = slack.is_some_and(|s| {
            let g = (p0 + p1 + p2) / 3.0;
            let far = positions.iter().map(|p| p.distance(g)).fold(0.0f32, f32::max);
            fd.full_within(g, far * (1.0 + 2.0 * (s[0] + s[1] + s[2])), center, tri_hardness, tri_aa)
        });
        let blocks = if large && !full_core {
            BlockFold::new_field([t0, t1, t2], positions, center, tri_hardness, tri_aa)
        } else {
            None
        };
        return Some(DabTri { tri, positions, clip, aa: tri_aa, hardness: tri_hardness, full_core, blocks, field: true });
    }

    // A brush under two texels across: hardness is meaningless below texel resolution, and a hard profile
    // keeps the texel under the cursor covered.
    let tri_hardness = if radius < aa.max(tri_aa) { 1.0 } else { hardness };
    let full_core = slack.is_some_and(|slack| in_full_core(positions, slack, center, radius, tri_hardness, tri_aa));
    // A triangle with a large box that is not wholly in the core: fold the 64 x 64 blocks of its UV box that are
    // (a tiled triangle repeats them in every image; triangles sharing UV space repeat them per triangle).
    let blocks = if large && !full_core {
        BlockFold::new([t0, t1, t2], positions, center, core_radius(radius, tri_hardness, tri_aa))
    } else {
        None
    };
    Some(DabTri { tri, positions, clip, aa: tri_aa, hardness: tri_hardness, full_core, blocks, field: false })
}

/// Rasterizes one candidate in the wrapped rows `rows`, reporting `(x, y, coverage)` for every non-zero coverage
/// (max rule at the caller). The parts of a candidate inside the full-strength core (the whole triangle, or the core
/// blocks of a large one) are rasterized folded: their covered texels are appended to `core_runs` as row runs
/// `(y, x0, x1)` at coverage 255 instead (see `put_core_runs`); the result of the max rule is the same as visiting
/// every image of every triangle.
#[allow(clippy::too_many_arguments)]
fn raster_cand(
    mesh: &MeshData,
    map: &TexelMap,
    cand: &DabTri,
    rows: std::ops::Range<u32>,
    shape: &DabShape,
    lean: bool,
    put: &mut impl FnMut(u32, u32, u8),
    core_runs: &mut Vec<(u32, u32, u32)>,
) -> Pass2Cost {
    // A ball triangle takes round 3's code exactly (its own instance of the rasterizer, with the ball's coverage
    // inlined): with the field branch in the per-fragment closure a 50-texel ball dab rasterized 1.7 x slower (P1).
    let fd = match &shape.field {
        Some(fd) if cand.field => fd,
        _ => return raster_ball(mesh, map, cand, rows, shape.center, shape.reach, lean, put, core_runs),
    };
    // The field triangle: the field's coverage without a per-fragment dispatch (`coverage_in` gives the same byte).
    let (center, hardness, aa) = (shape.center, cand.hardness, cand.aa);
    let [p0, p1, p2] = cand.positions;
    let cover = |f: &Fragment| -> u8 {
        let p = p0 * f.bary[0] + p1 * f.bary[1] + p2 * f.bary[2];
        (fd.cover(p, center, hardness, aa) * 255.0 + 0.5) as u8
    };
    if cand.full_core || cand.blocks.is_some() {
        let in_core = cand.blocks.map(|c| move |b: [i64; 4]| c.in_field_core(b, fd));
        raster_tri_covered_lean(
            mesh,
            map,
            cand.tri,
            cand.clip,
            rows,
            in_core.as_ref().map(|f| f as &dyn Fn([i64; 4]) -> bool),
            lean,
            &mut |y, a, b| core_runs.push((y, a, b)),
            &mut |f| {
                let cov = cover(&f);
                if cov > 0 {
                    put(f.x, f.y, cov);
                }
            },
        )
    } else {
        raster_tri_lean(mesh, map, cand.tri, cand.clip, rows, lean, &mut |f| {
            let cov = cover(&f);
            if cov > 0 {
                put(f.x, f.y, cov);
            }
        })
    }
}

/// `raster_cand` for a ball triangle: round 3's code (the ball's coverage with `center` and `radius`).
#[allow(clippy::too_many_arguments)]
fn raster_ball(
    mesh: &MeshData,
    map: &TexelMap,
    cand: &DabTri,
    rows: std::ops::Range<u32>,
    center: Vec3,
    radius: f32,
    lean: bool,
    put: &mut impl FnMut(u32, u32, u8),
    core_runs: &mut Vec<(u32, u32, u32)>,
) -> Pass2Cost {
    if cand.full_core || cand.blocks.is_some() {
        let in_core = cand.blocks.map(|c| move |b: [i64; 4]| c.in_core(b));
        raster_tri_covered_lean(
            mesh,
            map,
            cand.tri,
            cand.clip,
            rows,
            in_core.as_ref().map(|f| f as &dyn Fn([i64; 4]) -> bool),
            lean,
            &mut |y, a, b| core_runs.push((y, a, b)),
            &mut |f| {
                let cov = cand.coverage(&f, center, radius);
                if cov > 0 {
                    put(f.x, f.y, cov);
                }
            },
        )
    } else {
        raster_tri_lean(mesh, map, cand.tri, cand.clip, rows, lean, &mut |f| {
            let cov = cand.coverage(&f, center, radius);
            if cov > 0 {
                put(f.x, f.y, cov);
            }
        })
    }
}

/// Puts coverage 255 on every texel of the folded core runs of all candidates, each texel once: on a model whose
/// triangles share the same (tiled) UV space, a large dab folds every triangle onto the whole image, and visiting each
/// triangle's runs texel by texel would cost triangles x image texels.
fn put_core_runs(runs: &mut Vec<(u32, u32, u32)>, put: &mut impl FnMut(u32, u32, u8)) {
    runs.sort_unstable();
    let mut current: Option<(u32, u32, u32)> = None;
    for &(y, a, b) in runs.iter() {
        match &mut current {
            Some((cy, _, cb)) if *cy == y && a <= *cb => *cb = (*cb).max(b),
            _ => {
                if let Some((cy, ca, cb)) = current {
                    (ca..cb).for_each(|x| put(x, cy, 255));
                }
                current = Some((y, a, b));
            }
        }
    }
    if let Some((cy, ca, cb)) = current {
        (ca..cb).for_each(|x| put(x, cy, 255));
    }
    runs.clear();
}

/// Step 3 of the 3D dab: rasterizes the candidates into `dab_cov` (max rule) and lists the touched texels (round 5's
/// code: the reference of the tests and of the `RASTER_R5` seam).
#[cfg(test)]
fn rasterize_dab(mesh: &MeshData, map: &TexelMap, scratch: &mut StrokeScratch, cands: &[DabTri], center: Vec3, radius: f32) {
    rasterize_tip(mesh, map, scratch, cands, &DabShape { center, reach: radius, field: None });
}

/// `rasterize_dab` for any tip: the ball triangles with the shape's centre and radius, the field triangles with the
/// field's coverage. Round 5's code (full pass 2, clip-box binning into 64-row bands), kept as the reference of the
/// `RASTER_R5` seam; the brush paints through `raster_commit` since round 7.
#[cfg(test)]
fn rasterize_tip(mesh: &MeshData, map: &TexelMap, scratch: &mut StrokeScratch, cands: &[DabTri], shape: &DabShape) {
    let (w, h) = (map.width, map.height);
    let width = w as usize;
    let extents: Vec<(u64, Option<(u32, u32)>)> = cands.iter().map(|c| c.extent(w, h)).collect();
    let work = extents.iter().fold(0u64, |acc, e| acc.saturating_add(e.0));
    let StrokeScratch { dab_cov, dab_touched, .. } = scratch;
    dab_touched.clear();

    if work < PARALLEL_TEXELS {
        let mut put = |x: u32, y: u32, cov: u8| {
            let i = y as usize * width + x as usize;
            if dab_cov[i] == 0 {
                dab_touched.push(i as u32);
            }
            dab_cov[i] = dab_cov[i].max(cov);
        };
        let mut runs = Vec::new();
        let mut cost = Pass2Cost::default();
        for cand in cands {
            cost += raster_cand(mesh, map, cand, 0..h, shape, false, &mut put, &mut runs);
        }
        put_core_runs(&mut runs, &mut put);
        probe::count(10, cost.texels);
        probe::count(11, cost.skipped);
        return;
    }

    // Parallel pattern of section 6.3: bin the candidates into bands by wrapped row range (a wrapping box is listed
    // in every band it touches); each band owns its rows of `dab_cov`, so there are no shared writes.
    let bands = h.div_ceil(BAND_ROWS) as usize;
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); bands];
    for (ci, (_, rows)) in extents.iter().enumerate() {
        match *rows {
            None => bins.iter_mut().for_each(|b| b.push(ci as u32)),
            Some((first, count)) => {
                let end = first as u64 + count as u64;
                let spans = [(first as u64, end.min(h as u64)), (0, end.saturating_sub(h as u64))];
                for (b, bin) in bins.iter_mut().enumerate() {
                    let (b0, b1) = (b as u64 * BAND_ROWS as u64, (b as u64 + 1) * BAND_ROWS as u64);
                    if spans.iter().any(|&(s, e)| s < e && s < b1 && e > b0) {
                        bin.push(ci as u32);
                    }
                }
            }
        }
    }
    let touched: Vec<(Vec<u32>, Pass2Cost, u64)> = dab_cov
        .par_chunks_mut(BAND_ROWS as usize * width)
        .enumerate()
        .filter(|(b, _)| !bins[*b].is_empty())
        .map(|(b, chunk)| {
            let t = probe::now();
            let row0 = b as u32 * BAND_ROWS;
            let base = row0 as usize * width;
            let mut touched = Vec::new();
            let mut put = |x: u32, y: u32, cov: u8| {
                let i = y as usize * width + x as usize;
                let slot = &mut chunk[i - base];
                if *slot == 0 {
                    touched.push(i as u32);
                }
                *slot = (*slot).max(cov);
            };
            let mut runs = Vec::new();
            let mut cost = Pass2Cost::default();
            for &ci in &bins[b] {
                let rows = row0..(row0 + BAND_ROWS).min(h);
                cost += raster_cand(mesh, map, &cands[ci as usize], rows, shape, false, &mut put, &mut runs);
            }
            put_core_runs(&mut runs, &mut put);
            (touched, cost, t.map_or(0, |t| t.elapsed().as_nanos() as u64))
        })
        .collect();
    probe::count(12, touched.len() as u64);
    for (band, cost, nanos) in touched {
        dab_touched.extend(band);
        probe::count(10, cost.texels);
        probe::count(11, cost.skipped);
        probe::count(13, nanos);
    }
}
/// Collects changed texels into tight rectangles: one per group of edge-adjacent 64 x 64 tiles whose changes meet.
/// A dab that reaches several UV islands (or wraps around a border) so reports several small rects instead of one
/// bounding box spanning the atlas.
struct RectTracker {
    width: u32,
    /// Tight bounds of the changed texels per tile, keyed by `tile_y << 16 | tile_x`.
    tiles: BTreeMap<u32, IRect>,
    /// The tile of the previous texel (fragments arrive row by row, so most lookups hit it).
    current: Option<(u32, IRect)>,
}

impl RectTracker {
    fn new(width: u32) -> RectTracker {
        RectTracker { width, tiles: BTreeMap::new(), current: None }
    }

    fn flush(&mut self) {
        if let Some((key, rect)) = self.current.take() {
            let entry = self.tiles.entry(key).or_insert(rect);
            *entry = entry.union(&rect);
        }
    }

    /// Records the texel with the row-major index `i`.
    fn add(&mut self, i: u32) {
        let (x, y) = ((i % self.width) as i32, (i / self.width) as i32);
        let key = (y as u32 / TilePatch::TILE) << 16 | (x as u32 / TilePatch::TILE);
        let texel = IRect::new(x, y, x + 1, y + 1);
        match &mut self.current {
            Some((k, rect)) if *k == key => *rect = rect.union(&texel),
            _ => {
                self.flush();
                self.current = Some((key, texel));
            }
        }
    }

    /// The rectangles, in a deterministic order (by the first tile of each group, row-major).
    fn finish(mut self) -> Vec<IRect> {
        self.flush();
        let keys: Vec<u32> = self.tiles.keys().copied().collect();
        let tight: Vec<IRect> = self.tiles.values().copied().collect();
        let mut rects = tight.clone();
        // Union-find over the tiles; the root of a group is its first tile. Neighbouring tiles belong together
        // when their changed texels meet at the common border (separate blobs in adjacent tiles stay separate).
        let mut parent: Vec<usize> = (0..keys.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (i, &key) in keys.iter().enumerate() {
            let (tx, ty) = (key & 0xffff, key >> 16);
            let left = (tx > 0).then(|| key - 1);
            let up = (ty > 0).then(|| key - (1 << 16));
            for neighbour in [left, up].into_iter().flatten() {
                if let Ok(j) = keys.binary_search(&neighbour)
                    && tight[i].touches(&tight[j])
                {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
            }
        }
        let mut out = Vec::new();
        for i in 0..keys.len() {
            let r = root(&mut parent, i);
            if r != i {
                rects[r] = rects[r].union(&rects[i]);
            }
        }
        for i in 0..keys.len() {
            if parent[i] == i {
                out.push(rects[i]);
            }
        }
        out
    }
}

// The parallel commit gives every band its own tiles of the rect tracker.
const _: () = assert!(BAND_ROWS == TilePatch::TILE);

/// Step 4 of the 3D dab: adds the dab's coverage to the mask and resets the per-dab scratch.
///
/// A dab that touched many texels (at least `PARALLEL_TEXELS` and an eighth of the image: a large brush on a small or
/// tiled texture covers all of it) is committed on the rayon pool, band by band over the dense `dab_cov` (non-zero
/// exactly at the touched texels); each band owns its rows and its row of tracker tiles, so the result is the same.
fn commit_dab(scratch: &mut StrokeScratch, strength: f32) -> Vec<IRect> {
    let width = scratch.width;
    let mut tracker = RectTracker::new(width);
    let StrokeScratch { mask, dab_cov, dab_touched, row_spans, span_rows, spans_valid, .. } = scratch;
    // Round 7 A4: the spans of the previous serial commit are reset (a band commit resets its own rows).
    span_rows.iter().for_each(|&y| row_spans[y as usize] = NO_SPAN);
    span_rows.clear();
    *spans_valid = true;
    let touched = dab_touched.len() as u64;
    if touched >= PARALLEL_TEXELS && touched * 8 >= mask.len() as u64 {
        let band = BAND_ROWS as usize * width as usize;
        let bands: Vec<BTreeMap<u32, IRect>> = mask
            .par_chunks_mut(band)
            .zip(dab_cov.par_chunks_mut(band))
            .zip(row_spans.par_chunks_mut(BAND_ROWS as usize))
            .enumerate()
            .map(|(b, ((mask, cov), spans))| {
                let base = b * band;
                let mut tracker = RectTracker::new(width);
                spans.fill(NO_SPAN);
                for (k, (m, c)) in mask.iter_mut().zip(cov.iter_mut()).enumerate() {
                    if *c == 0 {
                        continue;
                    }
                    let cov = std::mem::take(c);
                    let inc = mask_increment(u16::MAX - *m, strength * (cov as f32 / 255.0));
                    if inc > 0 {
                        *m += inc;
                        tracker.add((base + k) as u32);
                        let (x, row) = ((k % width as usize) as u32, k / width as usize);
                        let span = &mut spans[row];
                        *span = (span.0.min(x), span.1.max(x + 1));
                    }
                }
                tracker.flush();
                tracker.tiles
            })
            .collect();
        bands.into_iter().for_each(|tiles| tracker.tiles.extend(tiles));
        dab_touched.clear();
        return tracker.finish();
    }
    for &i in dab_touched.iter() {
        let cov = std::mem::take(&mut dab_cov[i as usize]);
        let m = &mut mask[i as usize];
        let inc = mask_increment(u16::MAX - *m, strength * (cov as f32 / 255.0));
        if inc > 0 {
            *m += inc;
            tracker.add(i);
            let (x, y) = (i % width, i / width);
            let span = &mut row_spans[y as usize];
            if *span == NO_SPAN {
                span_rows.push(y);
            }
            *span = (span.0.min(x), span.1.max(x + 1));
        }
    }
    dab_touched.clear();
    tracker.finish()
}

/// The bands `[a, b]` (inclusive, at most two ranges) of `band_rows` rows that the unwrapped scan box `bx` meets in
/// an image of `height` rows split into `bands` bands.
fn box_bands(bx: &[i64; 4], height: u32, band_rows: u32, bands: u32) -> ([(u32, u32); 2], usize) {
    let (h, rows) = (height as i64, bx[3] - bx[1]);
    if rows >= h {
        return ([(0, bands - 1), (0, 0)], 1);
    }
    let first = bx[1].rem_euclid(h);
    let end = first + rows;
    let band = |row: i64| (row / band_rows as i64) as u32;
    if end <= h {
        return ([(band(first), band(end - 1)), (0, 0)], 1);
    }
    // Wrapping: rows first..h and 0..end - h; merged when the two band ranges meet.
    let (a, b) = (band(first), band(end - h - 1));
    if b + 1 >= a { ([(0, bands - 1), (0, 0)], 1) } else { ([(a, bands - 1), (0, b)], 2) }
}

/// Steps 3 and 4 of the 3D dab since round 7 (A2): rasterizes the candidates into `dab_cov` and commits them to the
/// mask; returns the rects whose mask changed, exactly those of `rasterize_tip` + `commit_dab`.
///
/// The candidates' scan boxes (`raster_box`) are computed first; candidates without one are dropped (they visit
/// nothing). Below `par_from` texels of box work (or with fewer than `PAR_DAB_MIN_CANDS` candidates and less than
/// `4 * PAR_DAB_WORK`, unless `par_from` is 0) the dab takes round 5's serial loop and `commit_dab`. Otherwise the
/// candidates are binned (CSR) into bands of `band_rows` rows by their boxes' wrapped row ranges, and one parallel
/// pass over the bands that have candidates rasterizes each band's candidates within its rows, puts the core runs,
/// commits the band's own texels and tracks their rects; the tiles are merged per tile key in band order. Each band
/// owns its rows of `dab_cov` and `mask`, coverage is the max over the fragments of a texel's own rows and a texel's
/// commit depends only on that texel, so the result does not depend on the partition or the thread count. Pass 2 is
/// lean (A1).
#[allow(clippy::too_many_arguments)]
fn raster_commit(
    mesh: &MeshData,
    map: &TexelMap,
    scratch: &mut StrokeScratch,
    cands: &mut Vec<DabTri>,
    shape: &DabShape,
    strength: f32,
    band_rows: u32,
    par_from: u64,
) -> Vec<IRect> {
    let (w, h) = (map.width, map.height);
    let width = w as usize;
    let t0 = probe::now();
    // The lean pass 2 reads the map's brush index. It is built here, on the calling thread, if the stroke has not
    // built it yet: its parallel build must not start inside a band job (a pool thread waiting in `OnceLock` for its
    // own initialisation would deadlock).
    map.prepare_brush_index();
    // Tight boxes (setup); an empty box marks a candidate that visits nothing.
    let mut boxes = std::mem::take(&mut scratch.boxes);
    boxes.clear();
    let box_of = |c: &DabTri| raster_box(mesh, map, c.tri, c.clip).unwrap_or([0; 4]);
    if cands.len() >= PAR_PREPARE_TRIS {
        boxes.par_extend(cands.par_iter().map(box_of));
    } else {
        boxes.extend(cands.iter().map(box_of));
    }
    // Drop the candidates without a box, keeping the order.
    let mut kept = 0;
    for i in 0..cands.len() {
        if boxes[i][0] < boxes[i][2] {
            cands.swap(kept, i);
            boxes[kept] = boxes[i];
            kept += 1;
        }
    }
    cands.truncate(kept);
    boxes.truncate(kept);
    let work = boxes.iter().fold(0u64, |acc, b| {
        let cols = ((b[2] - b[0]) as u64).min(w as u64);
        let rows = ((b[3] - b[1]) as u64).min(h as u64);
        acc.saturating_add(cols * rows)
    });
    probe::add(0, t0);

    let few = par_from > 0 && cands.len() < PAR_DAB_MIN_CANDS && work < 4 * PAR_DAB_WORK;
    if work < par_from || few {
        // Round 5's serial loop (pass 2 lean) and serial commit.
        let t1 = probe::now();
        let StrokeScratch { dab_cov, dab_touched, .. } = &mut *scratch;
        dab_touched.clear();
        let mut put = |x: u32, y: u32, cov: u8| {
            let i = y as usize * width + x as usize;
            if dab_cov[i] == 0 {
                dab_touched.push(i as u32);
            }
            dab_cov[i] = dab_cov[i].max(cov);
        };
        let mut runs = Vec::new();
        let mut cost = Pass2Cost::default();
        for cand in cands.iter() {
            cost += raster_cand(mesh, map, cand, 0..h, shape, true, &mut put, &mut runs);
        }
        put_core_runs(&mut runs, &mut put);
        probe::add(1, t1);
        probe::count(10, cost.texels);
        probe::count(11, cost.skipped);
        probe::count(5, scratch.dab_touched.len() as u64);
        scratch.boxes = boxes;
        let t2 = probe::now();
        let rects = commit_dab(scratch, strength);
        probe::add(2, t2);
        return rects;
    }

    let t1 = probe::now();
    // CSR binning by wrapped row range: count, prefix sums, fill (the fill leaves `band_start[b]` at the end of band
    // b; one shift restores the starts).
    let bands = h.div_ceil(band_rows);
    let StrokeScratch { mask, dab_cov, band_start, band_items, row_spans, span_rows, spans_valid, .. } = &mut *scratch;
    // Round 7 A4: the spans of the previous serial commit are reset; each band resets its own rows.
    span_rows.iter().for_each(|&y| row_spans[y as usize] = NO_SPAN);
    span_rows.clear();
    *spans_valid = true;
    band_start.clear();
    band_start.resize(bands as usize + 1, 0);
    for bx in &boxes {
        let (ranges, n) = box_bands(bx, h, band_rows, bands);
        for &(a, b) in &ranges[..n] {
            (a..=b).for_each(|band| band_start[band as usize + 1] += 1);
        }
    }
    for b in 0..bands as usize {
        band_start[b + 1] += band_start[b];
    }
    band_items.clear();
    band_items.resize(band_start[bands as usize] as usize, 0);
    for (ci, bx) in boxes.iter().enumerate() {
        let (ranges, n) = box_bands(bx, h, band_rows, bands);
        for &(a, b) in &ranges[..n] {
            for band in a..=b {
                let slot = &mut band_start[band as usize];
                band_items[*slot as usize] = ci as u32;
                *slot += 1;
            }
        }
    }
    for b in (1..=bands as usize).rev() {
        band_start[b] = band_start[b - 1];
    }
    band_start[0] = 0;

    let (band_start, band_items, cands) = (&*band_start, &*band_items, &*cands);
    let chunk = band_rows as usize * width;
    let results: Vec<(BTreeMap<u32, IRect>, u64, Pass2Cost, u64)> = mask
        .par_chunks_mut(chunk)
        .zip(dab_cov.par_chunks_mut(chunk))
        .zip(row_spans.par_chunks_mut(band_rows as usize))
        .enumerate()
        .filter(|(b, _)| band_start[*b] < band_start[*b + 1])
        .map(|(b, ((mask, cov), spans))| {
            let t = probe::now();
            let row0 = b as u32 * band_rows;
            let base = row0 as usize * width;
            let mut touched: Vec<u32> = Vec::new();
            let mut cost = Pass2Cost::default();
            {
                let mut put = |x: u32, y: u32, c: u8| {
                    let i = y as usize * width + x as usize;
                    let slot = &mut cov[i - base];
                    if *slot == 0 {
                        touched.push(i as u32);
                    }
                    *slot = (*slot).max(c);
                };
                let mut runs = Vec::new();
                let rows = row0..(row0 + band_rows).min(h);
                for &ci in &band_items[band_start[b] as usize..band_start[b + 1] as usize] {
                    cost += raster_cand(mesh, map, &cands[ci as usize], rows.clone(), shape, true, &mut put, &mut runs);
                }
                put_core_runs(&mut runs, &mut put);
            }
            // The band's own commit (as `commit_dab` per texel), with the changed span of each of its rows (A4).
            let mut tracker = RectTracker::new(w);
            spans.fill(NO_SPAN);
            for &i in &touched {
                let k = i as usize - base;
                let c = std::mem::take(&mut cov[k]);
                let m = &mut mask[k];
                let inc = mask_increment(u16::MAX - *m, strength * (c as f32 / 255.0));
                if inc > 0 {
                    *m += inc;
                    tracker.add(i);
                    let (x, row) = ((k % width) as u32, k / width);
                    let span = &mut spans[row];
                    *span = (span.0.min(x), span.1.max(x + 1));
                }
            }
            tracker.flush();
            (tracker.tiles, touched.len() as u64, cost, t.map_or(0, |t| t.elapsed().as_nanos() as u64))
        })
        .collect();
    probe::add(1, t1);
    let t2 = probe::now();
    // Bands of `band_rows` rows share the tracker's 64-row tiles: merge by union per tile key.
    let mut tracker = RectTracker::new(w);
    probe::count(12, results.len() as u64);
    for (tiles, touched, cost, nanos) in results {
        for (key, rect) in tiles {
            tracker.tiles.entry(key).and_modify(|r| *r = r.union(&rect)).or_insert(rect);
        }
        probe::count(5, touched);
        probe::count(10, cost.texels);
        probe::count(11, cost.skipped);
        probe::count(13, nanos);
    }
    scratch.boxes = boxes;
    let rects = tracker.finish();
    probe::add(2, t2);
    rects
}

/// One spherical dab on the surface. `normal` = the stroke's reference normal (unflipped shading normal, section 6.4).
/// Returns the wrapped texel rects whose mask changed.
///
/// `strength` is flow x pressure (0..=1). The rects are the bounds of groups of changed texels (one per UV island
/// the dab reaches, more when an island crosses an image border); they may overlap, re-applying a texel twice is
/// harmless. Nothing happens (and nothing is
/// returned) for non-finite input, a negative radius, a scratch of the wrong size or a strength of 0.
#[allow(clippy::too_many_arguments)]
pub fn dab_3d(
    mesh: &MeshData,
    bvh: &Bvh,
    map: &TexelMap,
    set: TextureSetId,
    scratch: &mut StrokeScratch,
    center: Vec3,
    normal: Vec3,
    radius: f32,
    hardness: f32,
    strength: f32,
    same_side_only: bool,
) -> Vec<IRect> {
    let strength = unit(strength);
    if scratch.size() != (map.width, map.height)
        || map.width == 0
        || map.height == 0
        || strength <= 0.0
        || !(center.is_finite() && radius.is_finite() && radius >= 0.0)
    {
        return Vec::new();
    }
    let mut tris = std::mem::take(&mut scratch.tris);
    let t0 = probe::now();
    let mut cands = dab_candidates(mesh, bvh, map, set, center, normal, radius, hardness, same_side_only, &mut tris);
    probe::add(0, t0);
    probe::count(14, cands.len() as u64);
    scratch.tris = tris;
    if cands.is_empty() {
        return Vec::new();
    }
    probe::count(4, 1_000_000);
    #[cfg(test)]
    if raster_r5() {
        let t1 = probe::now();
        rasterize_dab(mesh, map, scratch, &cands, center, radius);
        probe::add(1, t1);
        probe::count(5, scratch.dab_touched.len() as u64);
        let t2 = probe::now();
        let rects = commit_dab(scratch, strength);
        probe::add(2, t2);
        return rects;
    }
    let shape = DabShape { center, reach: radius, field: None };
    raster_commit(mesh, map, scratch, &mut cands, &shape, strength, DAB_BAND_ROWS, PAR_DAB_WORK)
}

/// Reused buffers of the shared sphere query of a stroke (FEATURE_BRUSH 5.4, 5.9): no allocation per dab.
#[derive(Default)]
pub(crate) struct DabQuery {
    /// `(triangle, squared distance)` of the last query, ascending by triangle.
    found: Vec<(TriId, f32)>,
    /// Per set (index = set id): the candidate triangles of the current dab, ascending. Empty = not reached.
    pub(crate) lists: Vec<Vec<TriId>>,
    /// Per set: mean texel size, first and final query radius.
    aa: Vec<f32>,
    rho1: Vec<f32>,
    rho: Vec<f32>,
}

/// Steps 1 to 4 and 6 of the restructured dab (FEATURE_BRUSH 5.4): one sphere query around `shape.center` for every
/// set `0..n` that `map_of` names (`None` = skipped by the stroke; a closure, so that no list is built per dab), at most one more when a set has coarser texels than its mean,
/// and per set the candidate list in ascending triangle id: the triangles of the set within the set's query radius
/// (`R_b + 0.5 * aa`, widened as `dab_candidates` widens it) that pass the side test of `reach` and, for a field,
/// the early reject against the shape's box. For the ball this is exactly the list `dab_candidates` filters for the
/// set, so the masks are those of the per-set path (test C2); a set is reached when its list is not empty.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shared_candidates<'m>(
    mesh: &MeshData,
    bvh: &Bvh,
    n: usize,
    map_of: impl Fn(usize) -> Option<&'m TexelMap>,
    shape: &DabShape,
    normal: Vec3,
    reach: SurfaceReach,
    q: &mut DabQuery,
) {
    let t = probe::now();
    q.lists.resize_with(n, Vec::new);
    q.lists.iter_mut().for_each(Vec::clear);
    let (center, radius) = (shape.center, shape.reach);
    if !(center.is_finite() && radius.is_finite() && radius >= 0.0) {
        return;
    }
    let same_side_only = reach == SurfaceReach::SameSide;
    q.aa.clear();
    q.aa.extend((0..n).map(|s| map_of(s).map_or(0.0, set_texel_world)));
    q.rho1.clear();
    q.rho1.extend(q.aa.iter().map(|&aa| radius + 0.5 * aa));
    let Some(r1) = (0..n).filter(|&s| map_of(s).is_some()).map(|s| q.rho1[s]).reduce(f32::max) else {
        return;
    };
    // The set of a triangle, when the query looks at that set.
    let set_of = |tri: TriId| -> Option<(usize, &TexelMap)> {
        let s = mesh.tri_set(tri)?.index();
        if s >= n {
            return None;
        }
        Some((s, map_of(s)?))
    };
    let side = |tri: TriId| !same_side_only || mesh.geo_normal(tri).dot(normal) >= SAME_SIDE_MIN_DOT;

    bvh.query_sphere_dist(mesh, center, r1, &mut q.found);
    probe::count(7, 1_000_000);
    // Round 7 A3: a large query runs both passes on the pool, in fixed chunks. `map_of` is a closure of the caller (not
    // `Sync`): its answers are collected once here.
    let maps: Vec<Option<&TexelMap>> =
        if q.found.len() >= PAR_QUERY_TRIS { (0..n).map(&map_of).collect() } else { Vec::new() };
    let set_of_par = |tri: TriId| -> Option<(usize, &TexelMap)> {
        let s = mesh.tri_set(tri)?.index();
        Some((s, *maps.get(s)?.as_ref()?))
    };
    // First pass: the coarsest texels among each set's candidates (as `dab_candidates`: at least the mean).
    q.rho.clear();
    q.rho.extend_from_slice(&q.aa);
    if q.found.len() >= PAR_QUERY_TRIS {
        // Per-chunk maxima, then the maximum of those (order-free).
        let (rho1, aa) = (&q.rho1, &q.aa);
        let maxima: Vec<Vec<f32>> = q
            .found
            .par_chunks(QUERY_CHUNK)
            .map(|chunk| {
                let mut m = aa.clone();
                for &(tri, d2) in chunk {
                    if let Some((s, map)) = set_of_par(tri)
                        && d2 <= rho1[s] * rho1[s]
                        && side(tri)
                    {
                        m[s] = m[s].max(tri_texel_world(mesh, map, tri, aa[s]));
                    }
                }
                m
            })
            .collect();
        for m in maxima {
            q.rho.iter_mut().zip(m).for_each(|(r, m)| *r = r.max(m));
        }
    } else {
        for &(tri, d2) in &q.found {
            if let Some((s, map)) = set_of(tri)
                && d2 <= q.rho1[s] * q.rho1[s]
                && side(tri)
            {
                q.rho[s] = q.rho[s].max(tri_texel_world(mesh, map, tri, q.aa[s]));
            }
        }
    }
    // A set is widened only when its coarsest texels exceed the mean by 0.1 %; otherwise it keeps its first radius.
    for s in 0..n {
        let local_max = q.rho[s];
        q.rho[s] = if local_max > q.aa[s] * 1.001 { radius + 0.5 * local_max } else { q.rho1[s] };
    }
    let r2 = (0..n).filter(|&s| map_of(s).is_some()).map(|s| q.rho[s]).fold(r1, f32::max);
    if r2 > r1 {
        bvh.query_sphere_dist(mesh, center, r2, &mut q.found);
        probe::count(7, 1_000_000);
    }
    // The candidate test of the second pass.
    let keep = |tri: TriId, d2: f32, s: usize, map: &TexelMap, rho: &[f32], aa: &[f32]| -> bool {
        if !(d2 <= rho[s] * rho[s] && side(tri)) {
            return false;
        }
        if let Some(fd) = &shape.field {
            let grow = 0.5 * tri_texel_world(mesh, map, tri, aa[s]) / fd.k;
            if fd.rejects(mesh.tri_positions(tri), center, grow) {
                return false;
            }
        }
        true
    };
    if q.found.len() >= PAR_QUERY_TRIS && maps.len() == n {
        // Per-chunk per-set lists, appended in chunk order: the lists stay ascending.
        let (rho, aa) = (&q.rho, &q.aa);
        let parts: Vec<Vec<(usize, TriId)>> = q
            .found
            .par_chunks(QUERY_CHUNK)
            .map(|chunk| {
                chunk
                    .iter()
                    .filter_map(|&(tri, d2)| {
                        let (s, map) = set_of_par(tri)?;
                        keep(tri, d2, s, map, rho, aa).then_some((s, tri))
                    })
                    .collect()
            })
            .collect();
        for (s, tri) in parts.into_iter().flatten() {
            q.lists[s].push(tri);
        }
    } else {
        for &(tri, d2) in &q.found {
            let Some((s, map)) = set_of(tri) else { continue };
            if keep(tri, d2, s, map, &q.rho, &q.aa) {
                q.lists[s].push(tri);
            }
        }
    }
    probe::add(6, t);
}

/// One dab of `shape` on one set, from the set's candidate list of `shared_candidates`: per-triangle setup,
/// rasterization and commit, as `dab_3d` does for the ball. Returns the wrapped texel rects whose mask changed;
/// nothing for a scratch of the wrong size, a strength of 0 or an empty list.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dab_tip(
    mesh: &MeshData,
    map: &TexelMap,
    scratch: &mut StrokeScratch,
    shape: &DabShape,
    list: &[TriId],
    hardness: f32,
    strength: f32,
) -> Vec<IRect> {
    let strength = unit(strength);
    if scratch.size() != (map.width, map.height) || map.width == 0 || map.height == 0 || strength <= 0.0 || list.is_empty() {
        return Vec::new();
    }
    let t0 = probe::now();
    let aa = set_texel_world(map);
    let r5 = raster_r5();
    // Round 5 B-4.1: the candidates go into a buffer the scratch keeps (dense pieces give thousands per dab).
    let mut cands = std::mem::take(&mut scratch.cands);
    cands.clear();
    #[cfg(test)]
    if FRESH_CANDIDATES.with(|f| f.get()) {
        cands = Vec::new(); // the bench's A/B of B-4.1: round 4's allocation per dab
    }
    // Round 7 A2: a long list is prepared on the pool (`par_extend` keeps the order; it affects no result anyway).
    let prepare = |&tri: &TriId| prepare_tri(mesh, map, tri, shape, hardness, aa);
    if list.len() >= PAR_PREPARE_TRIS && !r5 {
        cands.par_extend(list.par_iter().filter_map(prepare));
    } else {
        cands.extend(list.iter().filter_map(prepare));
    }
    probe::add(0, t0);
    probe::count(14, cands.len() as u64);
    if cands.is_empty() {
        scratch.cands = cands;
        return Vec::new();
    }
    probe::count(4, 1_000_000);
    #[cfg(test)]
    if r5 {
        let t1 = probe::now();
        rasterize_tip(mesh, map, scratch, &cands, shape);
        scratch.cands = cands;
        probe::add(1, t1);
        probe::count(5, scratch.dab_touched.len() as u64);
        let t2 = probe::now();
        let rects = commit_dab(scratch, strength);
        probe::add(2, t2);
        return rects;
    }
    let rects = raster_commit(mesh, map, scratch, &mut cands, shape, strength, DAB_BAND_ROWS, PAR_DAB_WORK);
    scratch.cands = cands;
    rects
}

/// Timing counters of the dab phases (the stroke probe, ARCHITECTURE.md 5.19; round 5 B-2): a runtime switch that
/// is off by default, in every build. Off, `now()` is one relaxed load and returns `None`, `add` and `count` do
/// nothing. On, the counters of the calling thread grow (the dab phases run on the thread that calls the stroke; the
/// parallel parts are timed around their parallel loops). Slots: see `STROKE_PROBE_NAMES` (0 candidates: per-set
/// setup, `dab_3d` also its queries; 1 raster; 2 commit; 3 re-apply; 4 dabs, 1 000 000 per per-set dab; 5 touched
/// texels; 6 shared query: tree walks and per-set lists; 7 tree walks of the shared query, 1 000 000 each; 8 first
/// touch of a set: `stroke_target`; 9 layer buffer copied: `Arc::make_mut` of a shared buffer; round 7: 10 pass-2
/// texels looked at, 11 pass-2 calls skipped by the lean rule, 12 band jobs, 13 band work: the band jobs' times
/// summed over all threads, 14 prepared candidates). The band jobs of the parallel paths return their counts and times
/// and the calling thread adds them (the counters of worker threads would be lost).
pub(crate) mod probe {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Number of slots.
    pub(crate) const SLOTS: usize = 15;

    static ON: AtomicBool = AtomicBool::new(false);

    thread_local! {
        static NANOS: [Cell<u64>; SLOTS] = const { [const { Cell::new(0) }; SLOTS] };
    }

    /// Turns the probe on or off for every thread.
    pub(crate) fn set_enabled(on: bool) {
        ON.store(on, Ordering::Relaxed);
    }

    /// True while the probe is on.
    #[inline(always)]
    pub(crate) fn enabled() -> bool {
        ON.load(Ordering::Relaxed)
    }

    /// Adds `n` to a counting slot (4, 5, 7, 10, 11, 12, 14; 13 takes summed band times); nothing while the probe is
    /// off.
    #[inline(always)]
    pub(crate) fn count(slot: usize, n: u64) {
        if enabled() {
            NANOS.with(|s| s[slot].set(s[slot].get().wrapping_add(n)));
        }
    }

    /// The start of a timed phase: `None` while the probe is off.
    #[inline(always)]
    pub(crate) fn now() -> Option<std::time::Instant> {
        if enabled() { Some(std::time::Instant::now()) } else { None }
    }

    /// Adds the time since `start` to a timing slot (nothing for `None`).
    #[inline(always)]
    pub(crate) fn add(slot: usize, start: Option<std::time::Instant>) {
        if let Some(t) = start {
            NANOS.with(|s| s[slot].set(s[slot].get().wrapping_add(t.elapsed().as_nanos() as u64)));
        }
    }

    /// The counters of the calling thread.
    pub(crate) fn read() -> [u64; SLOTS] {
        NANOS.with(|s| std::array::from_fn(|i| s[i].get()))
    }

    /// The counters of the calling thread, which are reset to zero (the timing benches of the tests).
    #[cfg(test)]
    pub(crate) fn take() -> [u64; SLOTS] {
        NANOS.with(|s| std::array::from_fn(|i| s[i].take()))
    }
}

/// A run of texels along one axis of a 2D dab, in wrapped coordinates.
#[derive(Copy, Clone, Default)]
struct AxisSpan {
    start: u32,
    len: u32,
    /// Signed distance from the dab centre to the centre of texel `start`.
    first: f32,
}

/// The texels along one axis that a dab at `c` with the given support radius reaches: at most two wrapped runs.
/// A dab wider than the image visits every texel once, at its image nearest the centre.
fn axis_spans(c: f32, support: f32, size: u32) -> ([AxisSpan; 2], usize) {
    let (c, support, n) = (c as f64, support as f64, size as i64);
    let mut a = (c - support).floor() as i64;
    let mut b = ((c + support).floor() as i64).saturating_add(1);
    if b.saturating_sub(a) > n {
        // Exactly `size` texels, every centre within half an image of `c`.
        a = (c - n as f64 * 0.5 - 0.5).ceil() as i64;
        b = a.saturating_add(n);
    }
    let count = b.saturating_sub(a).clamp(0, n);
    let start = a.rem_euclid(n);
    let len0 = count.min(n - start);
    let mut spans = [AxisSpan::default(); 2];
    let mut used = 0;
    for (start, len, offset) in [(start, len0, 0), (0, count - len0, len0)] {
        if len > 0 {
            let first = (a.saturating_add(offset) as f64 + 0.5 - c) as f32;
            spans[used] = AxisSpan { start: start as u32, len: len as u32, first };
            used += 1;
        }
    }
    (spans, used)
}

/// One disc dab in texel space (wraps around the image borders). Returns the texel rects whose mask changed.
///
/// `center_px` is in continuous texel coordinates, `radius` in texels, `strength` is flow x pressure (0..=1). A dab
/// crossing a border yields up to four rects. Nothing happens for non-finite input, a negative radius or a strength
/// of 0.
pub fn dab_2d(scratch: &mut StrokeScratch, center_px: Vec2, radius: f32, hardness: f32, strength: f32) -> Vec<IRect> {
    let (w, h) = scratch.size();
    let strength = unit(strength);
    if w == 0 || h == 0 || strength <= 0.0 || !(center_px.is_finite() && radius.is_finite() && radius >= 0.0) {
        return Vec::new();
    }
    // The anti-aliasing ramp is one texel wide; below two texels across the profile is hard (section 6.4).
    const AA: f32 = 1.0;
    let hardness = if radius < AA { 1.0 } else { hardness };
    let support = radius + 0.5 * AA;
    let (xs, nx) = axis_spans(center_px.x, support, w);
    let (ys, ny) = axis_spans(center_px.y, support, h);
    let (xs, ys) = (&xs[..nx], &ys[..ny]);
    let width = w as usize;
    // Round 7 A4: a 2D dab's changes are not in the row spans: its rects are re-applied whole.
    scratch.spans_valid = false;
    let cols: u64 = xs.iter().map(|s| s.len as u64).sum();
    let rows: u64 = ys.iter().map(|s| s.len as u64).sum();
    let parallel = cols * rows >= PARALLEL_TEXELS;

    // Raises the mask along one row; returns the changed column range (inclusive) per x span.
    let paint_row = |dy: f32, row: &mut [u16]| -> [Option<(u32, u32)>; 2] {
        let mut changed = [None; 2];
        let reach2 = support * support - dy * dy;
        if reach2 <= 0.0 {
            return changed;
        }
        let reach = reach2.sqrt();
        for (si, span) in xs.iter().enumerate() {
            // Texels whose centre lies within the support: |first + k| < reach.
            let k0 = (-reach - span.first).floor().max(0.0) as u32;
            let k1 = ((reach - span.first).ceil() + 1.0).clamp(0.0, span.len as f32) as u32;
            for k in k0..k1 {
                let dx = span.first + k as f32;
                let cov = falloff((dx * dx + dy * dy).sqrt(), radius, hardness, AA);
                if cov <= 0.0 {
                    continue;
                }
                let x = span.start + k;
                let m = &mut row[x as usize];
                let inc = mask_increment(u16::MAX - *m, strength * cov);
                if inc > 0 {
                    *m += inc;
                    changed[si] = Some(changed[si].map_or((x, x), |(lo, hi): (u32, u32)| (lo.min(x), hi.max(x))));
                }
            }
        }
        changed
    };

    let mut out = Vec::new();
    for yspan in ys {
        let band = &mut scratch.mask[yspan.start as usize * width..(yspan.start + yspan.len) as usize * width];
        let per_row: Vec<[Option<(u32, u32)>; 2]> = if parallel {
            band.par_chunks_mut(width).enumerate().map(|(j, row)| paint_row(yspan.first + j as f32, row)).collect()
        } else {
            band.chunks_mut(width).enumerate().map(|(j, row)| paint_row(yspan.first + j as f32, row)).collect()
        };
        for si in 0..xs.len() {
            let mut rect = IRect::EMPTY;
            for (j, changed) in per_row.iter().enumerate() {
                if let Some((lo, hi)) = changed[si] {
                    let y = (yspan.start + j as u32) as i32;
                    rect = rect.union(&IRect::new(lo as i32, y, hi as i32 + 1, y + 1));
                }
            }
            if !rect.is_empty() {
                out.push(rect);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::mesh::RawMesh;
    use crate::samples;
    use crate::texel_map::raster_tri;
    use crate::testutil::{XorShift, mesh_of};

    const SET: TextureSetId = TextureSetId(0);

    /// A mesh with its BVH, the texel map of set 0 and a matching scratch.
    struct Fixture {
        mesh: MeshData,
        bvh: Bvh,
        map: TexelMap,
        scratch: StrokeScratch,
    }

    impl Fixture {
        fn new(raw: RawMesh, size: u32) -> Fixture {
            let mesh = mesh_of(raw);
            let bvh = Bvh::build(&mesh);
            let map = TexelMap::build(&mesh, SET, size, size, TexelMap::default_gutter(size, size));
            Fixture { mesh, bvh, map, scratch: StrokeScratch::new(size, size) }
        }

        fn dab(&mut self, center: Vec3, normal: Vec3, radius: f32, hardness: f32, strength: f32) -> Vec<IRect> {
            dab_3d(&self.mesh, &self.bvh, &self.map, SET, &mut self.scratch, center, normal, radius, hardness, strength, true)
        }

        fn masks(&self) -> Vec<u16> {
            self.scratch.mask.clone()
        }

        /// Covered (non-gutter) texels of the triangles accepted by `keep`, with their surface point and mask.
        fn covered(&self, keep: impl Fn(TriId) -> bool) -> Vec<(u32, u32, Vec3, u16)> {
            let mut out = Vec::new();
            for y in 0..self.map.height {
                for x in 0..self.map.width {
                    if let Some((tri, false)) = self.map.owner(x, y)
                        && keep(tri)
                    {
                        let p = self.map.locate(&self.mesh, x, y).expect("owned texel").position;
                        out.push((x, y, p, self.scratch.mask(x, y)));
                    }
                }
            }
            out
        }
    }

    /// Cube face of a triangle in `seam_cube` atlas order: +X, -X, +Y, -Y, +Z, -Z.
    fn face_of(mesh: &MeshData, tri: TriId) -> usize {
        let n = mesh.geo_normal(tri);
        let a = n.abs();
        if a.x >= a.y && a.x >= a.z {
            if n.x > 0.0 { 0 } else { 1 }
        } else if a.y >= a.z {
            if n.y > 0.0 { 2 } else { 3 }
        } else if n.z > 0.0 {
            4
        } else {
            5
        }
    }

    /// Every texel whose mask changed lies inside one of the returned rects, and the rects lie inside the image.
    fn assert_rects_cover(rects: &[IRect], scratch: &StrokeScratch, before: &[u16]) {
        let (w, h) = scratch.size();
        let full = IRect::from_size(w, h);
        for r in rects {
            assert_eq!(r.intersect(&full), *r, "rect {r:?} outside the image");
        }
        for y in 0..h {
            for x in 0..w {
                if scratch.mask(x, y) != before[(y * w + x) as usize] {
                    let inside = rects.iter().any(|r| r.contains(x as i32, y as i32));
                    assert!(inside, "changed texel ({x}, {y}) outside the returned rects {rects:?}");
                }
            }
        }
    }

    #[test]
    fn falloff_profile() {
        let (r, aa) = (10.0, 1.0);
        // Hard brush: full inside, 0.5 at the rim, 0 at the support radius.
        assert_eq!(falloff(0.0, r, 1.0, aa), 1.0);
        assert_eq!(falloff(r - 0.5 * aa, r, 1.0, aa), 1.0);
        assert!((falloff(r, r, 1.0, aa) - 0.5).abs() < 1e-6);
        assert_eq!(falloff(r + 0.5 * aa, r, 1.0, aa), 0.0);
        assert_eq!(falloff(r + 5.0, r, 1.0, aa), 0.0);
        assert!(falloff(r + 0.49 * aa, r, 1.0, aa) > 0.0);
        // Soft brush: 1 inside hardness * radius, 0 at the rim and beyond, monotonic in between.
        assert_eq!(falloff(0.0, r, 0.5, aa), 1.0);
        assert_eq!(falloff(4.0, r, 0.5, aa), 1.0);
        assert_eq!(falloff(r, r, 0.5, aa), 0.0);
        assert_eq!(falloff(r + 0.5 * aa, r, 0.0, aa), 0.0);
        let mut prev = f32::INFINITY;
        for i in 0..=120 {
            let v = falloff(i as f32 * 0.1, r, 0.3, aa);
            assert!((0.0..=1.0).contains(&v) && v <= prev, "falloff not monotonic at {i}");
            prev = v;
        }
        // Garbage in, nothing out.
        assert_eq!(falloff(f32::NAN, r, 0.5, aa), 0.0);
        assert_eq!(falloff(1.0, f32::INFINITY, 0.5, aa), 0.0);
        // No usable AA width: a step at the rim.
        assert_eq!(falloff(1.0, r, 1.0, f32::NAN), 1.0);
        assert_eq!(falloff(r, r, 1.0, 0.0), 0.5);
        assert_eq!(falloff(r + 0.01, r, 1.0, 0.0), 0.0);
    }

    /// `apply_brush` against the f64 formulas of section 6.4.
    #[test]
    fn apply_brush_matches_the_reference() {
        let mut rng = XorShift::new(7);
        for _ in 0..20_000 {
            let dst: [u8; 4] = std::array::from_fn(|_| rng.next_u32() as u8);
            let color: [u8; 4] = std::array::from_fn(|_| rng.next_u32() as u8);
            let s = if rng.next_u32().is_multiple_of(8) { 1.0 } else { rng.next_f32() };
            let (sd, ad) = (s as f64, dst[3] as f64 / 255.0);
            let near = |a: u8, b: f64| (a as f64 - b).abs() <= 1.0;

            let p = apply_brush(dst, color, s, BrushMode::Paint, false);
            let ao = sd + ad * (1.0 - sd);
            if (ao * 255.0).round() >= 1.0 {
                assert!(near(p[3], ao * 255.0), "paint alpha {dst:?} {color:?} {s}: {p:?}");
                for c in 0..3 {
                    let co = (color[c] as f64 * sd + dst[c] as f64 * ad * (1.0 - sd)) / ao;
                    assert!(near(p[c], co), "paint rgb {dst:?} {color:?} {s}: {p:?}");
                }
            } else {
                assert_eq!(p, dst);
            }

            let e = apply_brush(dst, color, s, BrushMode::Erase, false);
            assert_eq!(e[..3], dst[..3], "erase changed RGB");
            assert!(near(e[3], dst[3] as f64 * (1.0 - sd)) && e[3] <= dst[3]);

            let l = apply_brush(dst, color, s, BrushMode::Paint, true);
            assert_eq!(l[3], dst[3], "alpha lock changed alpha");
            for c in 0..3 {
                assert!(near(l[c], dst[c] as f64 + (color[c] as f64 - dst[c] as f64) * sd));
            }
            assert_eq!(apply_brush(dst, color, s, BrushMode::Erase, true), dst);
            assert_eq!(apply_brush(dst, color, 0.0, BrushMode::Paint, false), dst);
        }
        assert_eq!(apply_brush([1, 2, 3, 0], [9, 8, 7, 255], 1.0, BrushMode::Paint, false), [9, 8, 7, 255]);
        assert_eq!(apply_brush([1, 2, 3, 200], [9, 8, 7, 255], 1.0, BrushMode::Erase, false), [1, 2, 3, 0]);
        assert_eq!(apply_brush([1, 2, 3, 200], [9, 8, 7, 255], f32::NAN, BrushMode::Erase, false), [1, 2, 3, 200]);
    }

    #[test]
    fn brush_params_default() {
        let b = BrushParams::default();
        assert_eq!((b.radius, b.hardness, b.opacity, b.flow, b.spacing), (16.0, 0.8, 1.0, 1.0, 0.12));
        assert_eq!((b.color, b.mode, b.same_side_only), ([0, 0, 0, 255], BrushMode::Paint, true));
    }

    #[test]
    fn scratch_mask_and_clear() {
        let mut s = StrokeScratch::new(8, 4);
        assert_eq!(s.mask(7, 3), 0);
        assert_eq!(s.mask(8, 0), 0, "outside the set");
        dab_2d(&mut s, Vec2::new(4.0, 2.0), 3.0, 1.0, 1.0);
        assert_eq!(s.mask(4, 2), u16::MAX);
        s.clear_rect(IRect::new(-5, -5, 4, 9));
        assert_eq!(s.mask(3, 2), 0);
        assert_eq!(s.mask(4, 2), u16::MAX);
        s.clear_rect(IRect::new(0, 0, 8, 4));
        assert!(s.mask.iter().all(|&m| m == 0));
        assert!(s.dab_cov.iter().all(|&c| c == 0));
    }

    #[test]
    fn cube_corner_dab_touches_exactly_three_islands() {
        let mut f = Fixture::new(samples::seam_cube(1.0), 256);
        let before = f.masks();
        let rects = f.dab(Vec3::splat(0.5), Vec3::ONE.normalize(), 0.15, 0.5, 1.0);
        assert_rects_cover(&rects, &f.scratch, &before);
        let mut per_face = [0u32; 6];
        for y in 0..256 {
            for x in 0..256 {
                if f.scratch.mask(x, y) > 0 {
                    let (tri, _) = f.map.owner(x, y).expect("painted texel without an owner");
                    per_face[face_of(&f.mesh, tri)] += 1;
                }
            }
        }
        let faces: BTreeSet<usize> = (0..6).filter(|&i| per_face[i] > 0).collect();
        assert_eq!(faces, BTreeSet::from([0, 2, 4]), "texels per face {per_face:?}");
        assert!(per_face.iter().all(|&n| n == 0 || n > 50), "{per_face:?}");
        // The corner lies in three different atlas cells: at least one rect per island.
        assert!(rects.len() >= 3, "{rects:?}");
        assert!(f.scratch.dab_cov.iter().all(|&c| c == 0) && f.scratch.dab_touched.is_empty());
    }

    /// The core seam-correctness assertion: a dab centred on a cube edge gives equal mask values at mirror-symmetric
    /// surface points of both faces. At 384 px the islands of +X and +Z are texel-aligned mirror images.
    #[test]
    fn dab_on_cube_edge_is_mirror_symmetric() {
        let mut f = Fixture::new(samples::seam_cube(1.0), 384);
        let c = Vec3::new(0.5, 0.1, 0.5);
        f.dab(c, Vec3::new(1.0, 0.0, 1.0).normalize(), 0.2, 0.3, 0.6);
        let near = |p: Vec3| p.distance(c) < 0.3;
        let on_x: Vec<_> = f.covered(|t| face_of(&f.mesh, t) == 0).into_iter().filter(|e| near(e.2)).collect();
        let on_z: Vec<_> = f.covered(|t| face_of(&f.mesh, t) == 4).into_iter().filter(|e| near(e.2)).collect();
        assert_eq!(on_x.len(), on_z.len());
        assert!(on_x.iter().filter(|e| e.3 > 0).count() > 100);
        let full = mask_increment(u16::MAX, 0.6);
        assert!(on_x.iter().any(|e| e.3 == full) && on_x.iter().any(|e| e.3 > 0 && e.3 < full / 2));
        for &(x, y, p, m) in &on_x {
            // Mirror in the plane x = z, which contains the dab centre.
            let q = Vec3::new(p.z, p.y, p.x);
            let (_, _, pq, mq) = *on_z.iter().min_by(|a, b| a.2.distance(q).total_cmp(&b.2.distance(q))).unwrap();
            assert!(pq.distance(q) < 1e-4, "no mirror texel for ({x}, {y}) at {p}");
            let (a, b) = (m as f32 / 257.0, mq as f32 / 257.0);
            assert!((a - b).abs() <= 1.0, "texel ({x}, {y}): {m} vs mirror {mq}");
        }
    }

    /// Sphere seam: the dab is centred on the u = 0/1 seam. The texel grid is mirror-symmetric there (texel k <->
    /// texel W-1-k); the faceted surface is only nearly so (the quad diagonals all lean the same way), hence the
    /// small tolerance. Coverage follows the 3D distance on both sides of the seam.
    #[test]
    fn sphere_seam_disc_is_continuous() {
        let mut f = Fixture::new(samples::uv_sphere(32, 16, 1.0), 256);
        let c = Vec3::new(0.0, 0.0, -1.0);
        let (radius, hardness) = (0.3, 0.5);
        f.dab(c, Vec3::NEG_Z, radius, hardness, 1.0);
        let (mut left, mut right) = (0u32, 0u32);
        for y in 0..256 {
            for x in 0..128 {
                let (a, b) = (f.scratch.mask(x, y), f.scratch.mask(255 - x, y));
                assert!((a as i32 - b as i32).abs() <= 4 * 257, "texels ({x}, {y}) / ({}, {y}): {a} vs {b}", 255 - x);
                left += (a > 0) as u32;
                right += (b > 0) as u32;
            }
        }
        assert!(left > 100 && left.abs_diff(right) <= left / 50, "{left} vs {right}");
        let aa = f.map.stats.texel_world;
        let (mut core, mut outside) = ([0u32; 2], 0u32);
        for (x, _, p, m) in f.covered(|_| true) {
            let d = p.distance(c);
            if d < hardness * radius - aa {
                assert_eq!(m, u16::MAX, "texel column {x} at distance {d}");
                core[(x >= 128) as usize] += 1;
            } else if d > radius + 4.0 * aa {
                assert_eq!(m, 0, "texel column {x} at distance {d}");
                outside += 1;
            }
        }
        assert!(core[0] > 20 && core[1] > 20 && outside > 1000, "{core:?} {outside}");
        // Full strength on both sides of the seam at the equator.
        assert_eq!(f.scratch.mask(0, 128), u16::MAX);
        assert_eq!(f.scratch.mask(255, 128), u16::MAX);
    }

    #[test]
    fn repeated_dabs_saturate_and_never_decrease() {
        let mut f = Fixture::new(samples::flat_quad(), 64);
        let c = Vec3::new(0.5, 0.5, 0.0);
        let (radius, hardness) = (0.2, 0.5);
        f.dab(c, Vec3::Z, radius, hardness, 1.0);
        let mut prev = f.masks();
        // Inside hardness * radius and the AA ramp the dab coverage is 255: one dab at flow 1 saturates.
        let core: Vec<_> = f.covered(|_| true).into_iter().filter(|e| e.2.distance(c) < hardness * radius - 0.02).collect();
        assert!(core.len() > 50);
        assert!(core.iter().all(|e| e.3 == u16::MAX));
        let mut rng = XorShift::new(3);
        for i in 0..20 {
            let strength = if i % 2 == 0 { 1.0 } else { rng.next_f32() };
            let jitter = Vec3::new(rng.next_f32() - 0.5, rng.next_f32() - 0.5, 0.0) * 0.1;
            f.dab(c + jitter, Vec3::Z, radius, hardness, strength);
            let now = f.masks();
            assert!(now.iter().zip(&prev).all(|(a, b)| a >= b), "a mask value decreased");
            prev = now;
        }
        for &(x, y, _, _) in &core {
            assert_eq!(f.scratch.mask(x, y), u16::MAX);
        }
    }

    /// Within one dab a texel visited twice (two quads on one UV island) gets the value of a single visit.
    #[test]
    fn overlapping_uvs_use_the_max_rule() {
        let mut both = Fixture::new(samples::mirrored_quads(), 128);
        let mut raw = samples::mirrored_quads();
        raw.triangles.truncate(2);
        let mut single = Fixture::new(raw, 128);
        // On the mirror plane: every texel near the shared edge is reached by both quads at the same distance.
        for f in [&mut both, &mut single] {
            f.dab(Vec3::ZERO, Vec3::Z, 0.3, 0.4, 0.5);
        }
        let mut partial = 0;
        for (i, (&a, &b)) in both.scratch.mask.iter().zip(&single.scratch.mask).enumerate() {
            assert!(a >= b && a <= b.saturating_add(257), "texel {i}: {a} with both quads, {b} with one");
            partial += (b > 0) as u32;
        }
        assert!(partial > 50);
        // Strength 0.5: a single visit gives at most half the range; accumulating would have doubled it.
        assert!(both.scratch.mask.iter().all(|&m| m <= mask_increment(u16::MAX, 0.5)));
        assert!(both.scratch.mask.contains(&mask_increment(u16::MAX, 0.5)));
    }

    #[test]
    fn a_dab_on_the_non_owner_quad_still_paints() {
        let mut f = Fixture::new(samples::mirrored_quads(), 128);
        let rects = f.dab(Vec3::new(0.5, 0.0, 0.0), Vec3::Z, 0.15, 0.8, 1.0);
        assert!(!rects.is_empty());
        let painted: Vec<_> = f.covered(|_| true).into_iter().filter(|e| e.3 > 0).collect();
        assert!(painted.len() > 50);
        // Quad A (triangles 0 and 1) owns every texel of the shared island, yet quad B's dab lands on them.
        assert!(painted.iter().all(|&(x, y, _, _)| f.map.owner(x, y).is_some_and(|(t, _)| t < 2)));
    }

    #[test]
    fn a_dab_on_either_mirrored_half_reaches_the_island_gutter() {
        // mirrored_quads at 64 px (gutter 2): the island's left border is column 16 (u = 0.25, x = -1 on quad A and
        // x = +1 on quad B); columns 14 and 15 are its gutter, owned by quad A only.
        for x in [-0.95f32, 0.95] {
            let mut f = Fixture::new(samples::mirrored_quads(), 64);
            f.dab(Vec3::new(x, 0.0, 0.0), Vec3::Z, 0.15, 1.0, 1.0);
            for col in 14..20 {
                assert_eq!(f.scratch.mask(col, 32), u16::MAX, "column {col} after a dab at x = {x}");
            }
            assert_eq!(f.map.owner(15, 32).map(|o| o.1), Some(true));
        }
    }

    #[test]
    fn same_side_only_drops_opposing_triangles() {
        let mut f = Fixture::new(samples::flat_quad(), 64);
        let c = Vec3::new(0.5, 0.5, 0.0);
        // The quad faces +Z: a reference normal of -Z opposes it.
        assert!(f.dab(c, Vec3::NEG_Z, 0.2, 0.5, 1.0).is_empty());
        let rects = dab_3d(&f.mesh, &f.bvh, &f.map, SET, &mut f.scratch, c, Vec3::NEG_Z, 0.2, 0.5, 1.0, false);
        assert!(!rects.is_empty());
        // Nothing for a wrong scratch, an unknown set, zero strength or NaN.
        let mut wrong = StrokeScratch::new(32, 32);
        assert!(dab_3d(&f.mesh, &f.bvh, &f.map, SET, &mut wrong, c, Vec3::Z, 0.2, 0.5, 1.0, true).is_empty());
        assert!(dab_3d(&f.mesh, &f.bvh, &f.map, TextureSetId(5), &mut f.scratch, c, Vec3::Z, 0.2, 0.5, 1.0, true).is_empty());
        assert!(f.dab(c, Vec3::Z, 0.2, 0.5, 0.0).is_empty());
        assert!(f.dab(Vec3::new(f32::NAN, 0.5, 0.0), Vec3::Z, 0.2, 0.5, 1.0).is_empty());
        assert!(f.dab(c, Vec3::Z, f32::INFINITY, 0.5, 1.0).is_empty());
    }

    /// A dab above the parallel threshold gives exactly the serial result (bands are exact).
    #[test]
    fn large_dab_parallel_path_matches_serial_rasterization() {
        let mut f = Fixture::new(samples::seam_cube(1.0), 512);
        let (c, n, radius) = (Vec3::new(0.5, 0.3, 0.2), Vec3::X, 0.6);
        let mut tris = Vec::new();
        let cands = dab_candidates(&f.mesh, &f.bvh, &f.map, SET, c, n, radius, 0.5, true, &mut tris);
        let work: u64 = cands.iter().map(|k| k.extent(512, 512).0).sum();
        assert!(work >= PARALLEL_TEXELS, "the test must exercise the parallel path ({work})");
        let rects = f.dab(c, n, radius, 0.5, 0.7);
        assert_rects_cover(&rects, &f.scratch, &[0; 512 * 512]);
        // Serial reference: one raster_tri per candidate over all rows.
        let mut cov = vec![0u8; 512 * 512];
        for k in &cands {
            raster_tri(&f.mesh, &f.map, k.tri, k.clip, 0..512, &mut |fr| {
                let i = (fr.y * 512 + fr.x) as usize;
                cov[i] = cov[i].max(k.coverage(&fr, c, radius));
            });
        }
        assert!(cov.iter().filter(|&&k| k > 0).count() > 20_000);
        for (i, (&m, &k)) in f.scratch.mask.iter().zip(&cov).enumerate() {
            assert_eq!(m, mask_increment(u16::MAX, 0.7 * (k as f32 / 255.0)), "texel {i}");
        }
        // Without a clip box every texel of every candidate is visited: the clip box loses nothing.
        let mut unclipped = vec![0u8; 512 * 512];
        for k in &cands {
            raster_tri(&f.mesh, &f.map, k.tri, None, 0..512, &mut |fr| {
                let i = (fr.y * 512 + fr.x) as usize;
                unclipped[i] = unclipped[i].max(k.coverage(&fr, c, radius));
            });
        }
        assert_eq!(cov, unclipped);
    }

    /// A 1-texel brush (radius 0.5) covers the texel under the cursor wherever the cursor sits inside it.
    #[test]
    fn one_texel_brush_covers_the_cursor_texel() {
        for hardness in [0.0, 1.0] {
            for pos in [(8.5, 8.5), (8.0, 8.5), (8.5, 8.0), (8.0, 8.0), (8.999, 8.999), (8.999, 8.0)] {
                let mut s = StrokeScratch::new(16, 16);
                let rects = dab_2d(&mut s, Vec2::new(pos.0, pos.1), 0.5, hardness, 1.0);
                assert!(s.mask(8, 8) > 0, "cursor {pos:?}, hardness {hardness}");
                assert!(!rects.is_empty());
                // Nothing at or beyond the support radius radius + 0.5.
                for y in 0..16u32 {
                    for x in 0..16u32 {
                        let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::new(pos.0, pos.1));
                        if d >= 1.0 {
                            assert_eq!(s.mask(x, y), 0, "({x}, {y}) for cursor {pos:?}");
                        }
                    }
                }
            }
        }
    }

    /// The same holds on the surface: a 3D brush smaller than a texel still covers the texel under the cursor.
    #[test]
    fn sub_texel_3d_brush_covers_the_cursor_texel() {
        let mut f = Fixture::new(samples::flat_quad(), 32);
        let texel = 1.0 / 32.0;
        for (fx, fy) in [(0.5, 0.5), (0.0, 0.5), (0.0, 0.0), (0.99, 0.99)] {
            f.scratch.clear_rect(IRect::from_size(32, 32));
            // Texel (10, 12): x = u, y = 1 - v.
            let (u, v) = ((10.0 + fx) * texel, (12.0 + fy) * texel);
            f.dab(Vec3::new(u, 1.0 - v, 0.0), Vec3::Z, 0.5 * texel, 0.0, 1.0);
            assert!(f.scratch.mask(10, 12) > 0, "cursor at ({fx}, {fy}) of the texel");
        }
    }

    #[test]
    fn dab_2d_wraps_at_the_borders() {
        let mut s = StrokeScratch::new(32, 32);
        let rects = dab_2d(&mut s, Vec2::new(1.0, 1.0), 4.0, 0.5, 1.0);
        assert_eq!(rects.len(), 4, "{rects:?}");
        assert_rects_cover(&rects, &s, &[0; 32 * 32]);
        for (x, y) in [(0, 0), (31, 31), (31, 0), (0, 31)] {
            assert!(s.mask(x, y) > 0, "({x}, {y})");
        }
        // Texel 31 is 1.5 left of the centre, texel 2 is 1.5 right of it.
        assert_eq!(s.mask(31, 31), s.mask(2, 2));
        assert_eq!(s.mask(31, 1), s.mask(2, 1));
        assert_eq!(s.mask(16, 16), 0);
        // Centres outside the image wrap too.
        let mut t = StrokeScratch::new(32, 32);
        dab_2d(&mut t, Vec2::new(33.0, -31.0), 4.0, 0.5, 1.0);
        assert_eq!(s.mask, t.mask);
        // A dab larger than the image visits every texel once.
        let mut s = StrokeScratch::new(8, 8);
        let rects = dab_2d(&mut s, Vec2::new(3.3, 4.7), 40.0, 1.0, 0.5);
        assert!(s.mask.iter().all(|&m| m == mask_increment(u16::MAX, 0.5)));
        assert!(rects.len() <= 4);
        assert_eq!(rects.iter().map(|r| r.area()).sum::<u64>(), 64, "{rects:?}");
        // Garbage in, nothing out.
        assert!(dab_2d(&mut s, Vec2::new(f32::NAN, 1.0), 4.0, 0.5, 1.0).is_empty());
        assert!(dab_2d(&mut s, Vec2::new(1.0, 1.0), -4.0, 0.5, 1.0).is_empty());
    }

    #[test]
    fn dab_2d_low_flow_builds_up_monotonically() {
        let mut s = StrokeScratch::new(16, 16);
        let mut prev = 0;
        for i in 0..2000 {
            dab_2d(&mut s, Vec2::new(8.0, 8.0), 5.0, 0.0, 0.01);
            let m = s.mask(8, 8);
            assert!(m > prev || m == u16::MAX, "no progress at dab {i}");
            prev = m;
        }
        // ceil guarantees that even 1 % steps reach the top.
        assert_eq!(prev, u16::MAX);
        assert!(s.mask(11, 8) > 0);
    }

    #[test]
    fn mask_increment_rounds_up() {
        assert_eq!(mask_increment(u16::MAX, 1.0), u16::MAX);
        assert_eq!(mask_increment(1, 1e-6), 1);
        assert_eq!(mask_increment(0, 1.0), 0);
        assert_eq!(mask_increment(100, 0.0), 0);
        assert_eq!(mask_increment(100, 0.501), 51);
    }

    /// Changed-rect grouping: separate blobs stay separate, touching tiles merge.
    #[test]
    fn rect_tracker_groups_changed_texels() {
        let mut t = RectTracker::new(256);
        for (x, y) in [(10, 10), (11, 10), (200, 5), (63, 70), (64, 70), (65, 71)] {
            t.add(y * 256 + x);
        }
        let rects = t.finish();
        assert_eq!(
            rects,
            vec![IRect::new(10, 10, 12, 11), IRect::new(200, 5, 201, 6), IRect::new(63, 70, 66, 72)],
            "{rects:?}"
        );
        assert!(RectTracker::new(16).finish().is_empty());
    }

    /// Quads of one world unit whose UVs repeat `tiles` times across each quad (every texel has several images).
    fn tiled_strip(quads: usize, tiles: f32) -> RawMesh {
        use crate::mesh::{RawMaterial, RawTriangle};
        let tri = |p: [Vec3; 3], uv: [[f32; 2]; 3]| RawTriangle { positions: p, normals: None, uvs: Some(uv.map(Vec2::from_array)), material: 0 };
        let mut triangles = Vec::new();
        for q in 0..quads {
            let x = q as f32;
            let (a, b, c, d) = (Vec3::new(x, 0.0, 0.0), Vec3::new(x + 1.0, 0.0, 0.0), Vec3::new(x + 1.0, 1.0, 0.0), Vec3::new(x, 1.0, 0.0));
            triangles.push(tri([a, b, d], [[0.0, 0.0], [tiles, 0.0], [0.0, tiles]]));
            triangles.push(tri([b, c, d], [[tiles, 0.0], [tiles, tiles], [0.0, tiles]]));
        }
        RawMesh { triangles, materials: vec![RawMaterial { name: "m".into(), base_color: [1.0; 4], double_sided: false, texture: None }] }
    }

    #[test]
    fn full_core_triangles_are_rasterized_folded_with_the_same_masks() {
        for (radius, hardness) in [(2.0f32, 0.8f32), (1.3, 1.0), (0.9, 0.5), (3.0, 0.3)] {
            let mut f = Fixture::new(tiled_strip(5, 3.3), 64);
            let center = Vec3::new(2.5, 0.5, 0.05);
            let mut tris = Vec::new();
            let cands = dab_candidates(&f.mesh, &f.bvh, &f.map, SET, center, Vec3::Z, radius, hardness, true, &mut tris);
            let folded = cands.iter().filter(|c| c.full_core).count();
            // The reference: every image rasterized (no folding).
            let plain: Vec<DabTri> = cands.iter().map(|c| DabTri { full_core: false, blocks: None, ..c.clone() }).collect();
            let mut reference = StrokeScratch::new(64, 64);
            rasterize_dab(&f.mesh, &f.map, &mut reference, &plain, center, radius);
            let reference_cov = reference.dab_cov.clone();
            let mut want_touched = reference.dab_touched.clone();
            let want_rects = commit_dab(&mut reference, 0.7);

            let mut folded_scratch = StrokeScratch::new(64, 64);
            rasterize_dab(&f.mesh, &f.map, &mut folded_scratch, &cands, center, radius);
            assert_eq!(folded_scratch.dab_cov, reference_cov, "radius {radius}, hardness {hardness}");
            let mut got_touched = folded_scratch.dab_touched.clone();
            got_touched.sort_unstable();
            want_touched.sort_unstable();
            assert_eq!(got_touched, want_touched);
            assert_eq!(commit_dab(&mut folded_scratch, 0.7), want_rects);
            assert_eq!(folded_scratch.mask, reference.mask);
            // Through the public entry point too.
            let rects = f.dab(center, Vec3::Z, radius, hardness, 0.7);
            assert_eq!(rects, want_rects);
            assert_eq!(f.masks(), reference.mask);
            if radius >= 1.3 {
                assert!(folded > 0, "radius {radius}: no triangle lies in the full-strength core");
            }
        }
    }

    /// Long quads whose UVs repeat many times along them: a dab covers part of every quad, so its triangles are
    /// neither wholly in nor wholly out of the core and only their core blocks are folded.
    #[test]
    fn core_blocks_of_large_triangles_are_folded_with_the_same_masks() {
        use crate::mesh::{RawMaterial, RawTriangle};
        let tri = |p: [Vec3; 3], uv: [[f32; 2]; 3]| RawTriangle { positions: p, normals: None, uvs: Some(uv.map(Vec2::from_array)), material: 0 };
        let mut triangles = Vec::new();
        for q in 0..3 {
            let y = q as f32;
            let (a, b, c, d) = (Vec3::new(0.0, y, 0.0), Vec3::new(8.0, y, 0.0), Vec3::new(8.0, y + 1.0, 0.0), Vec3::new(0.0, y + 1.0, 0.0));
            triangles.push(tri([a, b, d], [[0.0, 0.0], [7.5, 0.0], [0.0, 1.3]]));
            triangles.push(tri([b, c, d], [[7.5, 0.0], [7.5, 1.3], [0.0, 1.3]]));
        }
        let raw = RawMesh { triangles, materials: vec![RawMaterial { name: "m".into(), base_color: [1.0; 4], double_sided: false, texture: None }] };
        for (radius, hardness, size) in [(2.5f32, 1.0f32, 64u32), (3.0, 0.6, 64), (2.0, 0.9, 128)] {
            let mut f = Fixture::new(raw.clone(), size);
            let center = Vec3::new(3.3, 1.4, 0.1);
            let mut tris = Vec::new();
            let cands = dab_candidates(&f.mesh, &f.bvh, &f.map, SET, center, Vec3::Z, radius, hardness, true, &mut tris);
            assert!(cands.iter().any(|c| c.blocks.is_some()), "radius {radius}: no partly folded triangle");
            assert!(cands.iter().all(|c| !c.full_core));
            let plain: Vec<DabTri> = cands.iter().map(|c| DabTri { blocks: None, ..c.clone() }).collect();
            let mut reference = StrokeScratch::new(size, size);
            rasterize_dab(&f.mesh, &f.map, &mut reference, &plain, center, radius);
            let reference_cov = reference.dab_cov.clone();
            let mut want_touched = reference.dab_touched.clone();
            let want_rects = commit_dab(&mut reference, 0.6);

            let mut folded = StrokeScratch::new(size, size);
            rasterize_dab(&f.mesh, &f.map, &mut folded, &cands, center, radius);
            assert_eq!(folded.dab_cov, reference_cov, "radius {radius}, hardness {hardness}");
            assert!(folded.dab_cov.iter().filter(|&&c| c == 255).count() > 100);
            let mut got_touched = folded.dab_touched.clone();
            got_touched.sort_unstable();
            want_touched.sort_unstable();
            assert_eq!(got_touched, want_touched);
            assert_eq!(commit_dab(&mut folded, 0.6), want_rects);
            assert_eq!(folded.mask, reference.mask);
            assert_eq!(f.dab(center, Vec3::Z, radius, hardness, 0.6), want_rects);
            assert_eq!(f.masks(), reference.mask);
        }
    }

    #[test]
    fn the_parallel_commit_of_a_dense_dab_matches_the_serial_commit() {
        // A dense dab (every texel of a 256 x 320 image but one hole per row, coverage varying) and a sparse one.
        for (w, h, dense) in [(256u32, 320u32, true), (512, 512, false)] {
            let mut s = StrokeScratch::new(w, h);
            let mut seed = 12345u32;
            let mut next = || {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                seed >> 16
            };
            for i in 0..(w * h) as usize {
                s.mask[i] = (next() % 70_000).min(65_535) as u16;
                let x = i as u32 % w;
                let cov = if dense { (x != 17).then(|| (next() % 256) as u8) } else { (x < 40).then_some(200) };
                if let Some(cov) = cov.filter(|&c| c > 0) {
                    s.dab_cov[i] = cov;
                    s.dab_touched.push(i as u32);
                }
            }
            let parallel = (s.dab_touched.len() as u64) >= PARALLEL_TEXELS && s.dab_touched.len() as u64 * 8 >= (w * h) as u64;
            assert_eq!(parallel, dense);
            // The reference: the serial loop.
            let mut want_mask = s.mask.clone();
            let mut tracker = RectTracker::new(w);
            for &i in &s.dab_touched {
                let m = &mut want_mask[i as usize];
                let inc = mask_increment(u16::MAX - *m, 0.37 * (s.dab_cov[i as usize] as f32 / 255.0));
                if inc > 0 {
                    *m += inc;
                    tracker.add(i);
                }
            }
            let want_rects = tracker.finish();
            let rects = commit_dab(&mut s, 0.37);
            assert_eq!(rects, want_rects, "dense {dense}");
            assert_eq!(s.mask, want_mask);
            assert!(s.dab_cov.iter().all(|&c| c == 0) && s.dab_touched.is_empty());
        }
    }

    #[test]
    fn the_full_core_bound_is_conservative() {
        let p = [Vec3::ZERO, Vec3::X, Vec3::Y];
        // The farthest vertex from the centre (1, 1, 0) is the origin, sqrt(2) away.
        let c = Vec3::new(1.0, 1.0, 0.0);
        // Hardness 0.5, radius 4: core 2.
        assert!(in_full_core(p, [0.0; 3], c, 4.0, 0.5, 0.01));
        // Core 1.0 < sqrt(2).
        assert!(!in_full_core(p, [0.0; 3], c, 2.0, 0.5, 0.01));
        // The anti-aliasing ramp shrinks a hard brush's core: radius 1.5, aa 0.2 -> core 1.4 < sqrt(2).
        assert!(!in_full_core(p, [0.0; 3], c, 1.5, 1.0, 0.2));
        assert!(in_full_core(p, [0.0; 3], c, 1.6, 1.0, 0.2));
        // A sliver's barycentric slack widens the bound.
        assert!(!in_full_core(p, [0.2; 3], c, 4.0, 0.5, 0.01));
        // Everything at the bound: falloff is exactly 1 there.
        for d in [0.0f32, 1.0, 1.99] {
            assert_eq!(falloff(d, 4.0, 0.5, 0.01), 1.0);
        }
        assert!(!in_full_core(p, [0.0; 3], Vec3::ZERO, 0.0, 1.0, 0.0));
    }

    // ----- Round 4: tips through the shared path (FEATURE_BRUSH 8.1) -----

    /// The dab shape of a tip at `center` (ball when `field` is `None`), scale 1 (`k = radius`), world-from-tip
    /// `rotation`; `inradius` switches on the test seam of C23.
    fn shape_of<'a>(
        field: Option<&'a TipField>,
        center: Vec3,
        rotation: Mat3,
        radius: f32,
        through: bool,
        inradius: bool,
    ) -> DabShape<'a> {
        match field {
            None => DabShape { center, reach: radius, field: None },
            Some(field) => {
                let mut geom = TipGeom::new(field, through);
                geom.inradius_thickness = inradius;
                let fd = FieldDab { field, to_tip: rotation.transpose() * (1.0 / radius), k: radius, through, geom };
                DabShape { center, reach: field_reach(field, radius, through), field: Some(fd) }
            }
        }
    }

    /// The candidates of set 0 for `shape` (reach `All`).
    fn tip_cands(f: &Fixture, shape: &DabShape, hardness: f32) -> Vec<DabTri> {
        let mut q = DabQuery::default();
        shared_candidates(&f.mesh, &f.bvh, 1, |_| Some(&f.map), shape, Vec3::Z, SurfaceReach::All, &mut q);
        let aa = set_texel_world(&f.map);
        q.lists[0].iter().filter_map(|&t| prepare_tri(&f.mesh, &f.map, t, shape, hardness, aa)).collect()
    }

    /// One dab of `shape` at strength 1 through the shared path (reach `All`).
    fn tip_dab(f: &mut Fixture, shape: &DabShape, hardness: f32) -> Vec<IRect> {
        let mut q = DabQuery::default();
        shared_candidates(&f.mesh, &f.bvh, 1, |_| Some(&f.map), shape, Vec3::Z, SurfaceReach::All, &mut q);
        dab_tip(&f.mesh, &f.map, &mut f.scratch, shape, &q.lists[0], hardness, 1.0)
    }

    /// The coverage byte a dab of strength 1 left in the mask (`mask = ceil(65535 * cov / 255)`).
    fn cov_of(m: u16) -> u8 {
        (m as f32 / 257.0).round() as u8
    }

    /// The frame "Follow surface" gives for a facing normal with the hint +Y.
    fn follow(normal: Vec3) -> Mat3 {
        crate::brush_tip::tip_frame(normal, Vec3::Y, None).0
    }

    /// A ball of radius 1 as a field (129 nodes per axis).
    fn ball_field() -> TipField {
        TipField::from_fn([129, 129, 129], Vec3::splat(-1.1), 2.2 / 128.0, |p| p.length() - 1.0)
    }

    /// The union of two cubes of half side 0.4, the second shifted by 0.4 along X (centred on the origin): the
    /// formula stand-in for the baked `overlapping_cubes()` (the bake is package B1's).
    fn overlapping_cubes_field() -> TipField {
        TipField::from_fn([121, 121, 121], Vec3::splat(-1.0), 2.0 / 120.0, |p| {
            let cube = |c: Vec3| {
                let q = (p - c).abs() - Vec3::splat(0.4);
                q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
            };
            cube(Vec3::new(-0.2, 0.0, 0.0)).min(cube(Vec3::new(0.2, 0.0, 0.0)))
        })
    }

    /// A model baked by B1's `TipField::bake` (`CloseMode::Refuse`): the real counterparts of the formula stand-ins
    /// (added at M1, review requests of B3 and B1).
    fn baked_field(raw: crate::RawMesh) -> TipField {
        TipField::bake(&crate::testutil::mesh_of(raw), crate::CloseMode::Refuse, &crate::NoProgress).expect("the model bakes")
    }

    /// A box of half extents (0.62, 0.3, 0.3) on a grid that ends at x = +-0.5: a closed part cut off at the border
    /// of its grid (FEATURE_BRUSH 5.6), whose border values are negative (-0.12, about 7 cells) and whose zero level
    /// lies 0.12 beyond the grid, outside `bounds()`. The formula stand-in for a baked `cube_floor_rod()`-like cut.
    fn cut_box_field() -> TipField {
        TipField::from_fn([61, 41, 41], Vec3::new(-0.5, -1.0 / 3.0, -1.0 / 3.0), 1.0 / 60.0, |p| {
            let q = p.abs() - Vec3::new(0.62, 0.3, 0.3);
            q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
        })
    }

    /// A flat five-armed star outline (outer radius 0.95, inner 0.4) in the plane (x, z), from the signed distance to
    /// its polygon: the outline of C26 (usable Through only).
    fn star_outline_field() -> TipField {
        let star: Vec<Vec2> = (0..10)
            .map(|i| {
                let r = if i % 2 == 0 { 0.95 } else { 0.4 };
                let a = std::f32::consts::TAU * i as f32 / 10.0;
                Vec2::new(a.cos(), a.sin()) * r
            })
            .collect();
        TipField::outline_from_fn([129, 129], Vec2::splat(-1.04), 2.08 / 128.0, move |p| {
            let (mut d, mut inside) = (f32::INFINITY, false);
            for i in 0..star.len() {
                let (a, b) = (star[i], star[(i + 1) % star.len()]);
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                d = d.min((a + ab * t).distance(p));
                if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                    inside = !inside;
                }
            }
            if inside { -d } else { d }
        })
    }

    /// C12: a field made from `|q| - 1` paints what the ball paints: at most 4 of 255 apart at every texel and under
    /// 0.5 on average, on `flat_quad` (512) and `uv_sphere` (256), diameters 16, 64 and 200 texels, hardness 0.5 / 1.
    #[test]
    fn a_ball_shaped_field_equals_the_ball() {
        let ball = ball_field();
        let cases = [
            (samples::flat_quad(), 512u32, Vec3::new(0.5, 0.5, 0.0), Vec3::Z),
            (samples::uv_sphere(48, 24, 1.0), 256, Vec3::new(0.0, 0.3, 0.95).normalize(), Vec3::new(0.0, 0.3, 0.95).normalize()),
        ];
        for (raw, size, center, normal) in cases {
            for diameter in [16.0f32, 64.0, 200.0] {
                for hardness in [0.5f32, 1.0] {
                    let mut a = Fixture::new(raw.clone(), size);
                    let mut b = Fixture::new(raw.clone(), size);
                    let radius = 0.5 * diameter * a.map.stats.texel_world;
                    tip_dab(&mut a, &shape_of(None, center, follow(normal), radius, false, false), hardness);
                    tip_dab(&mut b, &shape_of(Some(&ball), center, follow(normal), radius, false, false), hardness);
                    let (mut worst, mut sum, mut n) = (0i32, 0i64, 0i64);
                    for (&ma, &mb) in a.scratch.mask.iter().zip(&b.scratch.mask) {
                        if ma > 0 || mb > 0 {
                            let d = (cov_of(ma) as i32 - cov_of(mb) as i32).abs();
                            worst = worst.max(d);
                            sum += d as i64;
                            n += 1;
                        }
                    }
                    let mean = sum as f64 / n.max(1) as f64;
                    assert!(n > 100, "size {size}, diameter {diameter}: {n} texels");
                    assert!(worst <= 4 && mean < 0.5, "size {size}, diameter {diameter}, hardness {hardness}: worst {worst}, mean {mean}");
                }
            }
        }
    }

    /// C9: the brute-force oracle. For the ball (`All`) and the Cube field on `uv_sphere` and `seam_cube` at 256,
    /// every covered non-gutter texel's coverage byte differs from the formula of 5.2 evaluated at its surface point
    /// (`TexelMap::locate`) by at most 1, and no texel with an oracle coverage over 1/255 is missed.
    #[test]
    fn coverage_matches_the_formula_at_every_texel() {
        let cube = TipField::cube();
        let cases = [
            (samples::uv_sphere(48, 24, 1.0), Vec3::new(0.3, 0.4, 0.86).normalize(), 0.55f32),
            (samples::seam_cube(1.0), Vec3::new(0.2, 0.1, 0.5), 0.45),
        ];
        for (raw, center, radius) in cases {
            for (name, field) in [("ball", None), ("cube", Some(&cube))] {
                for hardness in [0.8f32, 1.0] {
                    let mut f = Fixture::new(raw.clone(), 256);
                    let normal = if center.z == 0.5 { Vec3::Z } else { center };
                    let rotation = follow(normal) * Mat3::from_rotation_y(0.4);
                    let shape = shape_of(field, center, rotation, radius, false, false);
                    tip_dab(&mut f, &shape, hardness);
                    let aa = set_texel_world(&f.map);
                    let (mut checked, mut painted) = (0u32, 0u32);
                    for (x, y, p, m) in f.covered(|_| true) {
                        let (tri, _) = f.map.owner(x, y).unwrap();
                        let tri_aa = tri_texel_world(&f.mesh, &f.map, tri, aa);
                        let oracle = match field {
                            None => falloff(p.distance(center), radius, hardness, tri_aa),
                            Some(field) => {
                                let q = rotation.transpose() * (p - center) / radius;
                                let (sd, thick) = (radius * field.sample(q), radius * field.thickness(q));
                                falloff(thick + sd, thick, hardness, tri_aa)
                            }
                        };
                        let want = (oracle * 255.0 + 0.5) as u8;
                        let got = cov_of(m);
                        assert!(
                            (want as i32 - got as i32).abs() <= 1,
                            "{name}, hardness {hardness}: texel ({x}, {y}) at {p}: oracle {want}, dab {got}"
                        );
                        checked += (want > 1) as u32;
                        painted += (got > 0) as u32;
                    }
                    assert!(checked > 500 && painted > 500, "{name}: {checked} / {painted}");
                }
            }
        }
    }

    /// The fragment reject of `FieldDab::cover` (Solid) changes no coverage byte: for the Cube, the Cylinder, the
    /// two overlapping cubes, the ball field and a box cut off at its grid's border, 60,000 seeded random points each
    /// around and beyond the reject box (random rotation, tip unit 0.01 to 1, anti-aliasing width 0.1 % to 20 % of
    /// the tip unit, hardness 0 to 1), `cover` gives the byte the formula without the reject gives, and the triangle
    /// reject (`rejects`) keeps a triangle at every point the formula paints; and the reject is really taken.
    #[test]
    fn the_fragment_reject_changes_no_coverage() {
        let fields = [
            ("cube", TipField::cube()),
            ("cylinder", TipField::cylinder()),
            ("overlapping", overlapping_cubes_field()),
            ("ball", ball_field()),
            ("cut box", cut_box_field()),
            ("baked overlapping", baked_field(samples::overlapping_cubes())),
            ("baked floor rod", baked_field(samples::cube_floor_rod())),
            ("baked ball", baked_field(samples::uv_sphere(96, 48, 1.0))),
        ];
        let mut rng = XorShift::new(55);
        let byte = |c: f32| (c * 255.0 + 0.5) as u8;
        for (name, field) in &fields {
            let (mut rejected, mut painted, mut beyond_bounds) = (0u32, 0u32, 0u32);
            // Per stroke, as `stroke_begin_with` computes it.
            let geom = TipGeom::new(field, false);
            for i in 0..60_000u32 {
                let rotation = Mat3::from_quat(
                    glam::Quat::from_xyzw(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5)
                        .normalize(),
                );
                let k = 0.01 + rng.next_f32();
                let aa = k * (0.001 + 0.2 * rng.next_f32());
                let hardness = if i % 4 == 0 { 1.0 } else { rng.next_f32() };
                let center = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) - Vec3::splat(0.5);
                let fd = FieldDab { field, to_tip: rotation.transpose() * (1.0 / k), k, through: false, geom };
                // A tip point in the reject box grown by 0.3 tip units, so that both sides of every face are hit.
                let (lo, hi) = (fd.geom.box_min - Vec3::splat(0.3), fd.geom.box_max + Vec3::splat(0.3));
                let q = lo + (hi - lo) * Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32());
                let p = center + rotation * (q * k);
                let q = fd.tip(p, center);
                let (got, want) = (fd.cover(p, center, hardness, aa), fd.cover_at(q, p, center, hardness, aa));
                assert_eq!(byte(got), byte(want), "{name}: q {q}, k {k}, aa {aa}, hardness {hardness}");
                // The triangle reject of 5.6 keeps a (point-sized) triangle wherever the formula paints.
                if byte(want) > 0 {
                    assert!(!fd.rejects([p; 3], center, 0.5 * aa / k), "{name}: triangle rejected at q {q}, k {k}, aa {aa}");
                }
                let outside = q.cmplt(fd.geom.box_min - Vec3::splat(0.5 * aa / k + field.cell())).any()
                    || q.cmpgt(fd.geom.box_max + Vec3::splat(0.5 * aa / k + field.cell())).any();
                rejected += outside as u32;
                painted += (byte(want) > 0) as u32;
                // Painted beyond the field's own box (since contract request B3-1, applied at M1, `bounds()` holds
                // the zero level of a part cut off at its grid's border too, so this is 0 for every field).
                let b = field.bounds();
                let g = Vec3::splat(0.5 * aa / k + field.cell());
                beyond_bounds += (byte(want) > 0 && (q.cmplt(b.min - g).any() || q.cmpgt(b.max + g).any())) as u32;
            }
            eprintln!("{name}: {rejected} rejected, {painted} painted, {beyond_bounds} painted beyond bounds()");
            // (The cut box is thin in y and z: fewer of its random points are inside.)
            let min_painted = if *name == "cut box" { 5_000 } else { 10_000 };
            assert!(rejected > 10_000 && painted > min_painted, "{name}: {rejected} rejected, {painted} painted");
            assert_eq!(beyond_bounds, 0, "{name}");
        }
    }

    /// C16: folded and unfolded rasterization of field tips give the same coverage, texel for texel, on
    /// `wrapped_quad` (tiled UVs), for the Cube, the Cylinder, a union of two overlapping cubes and the flat star
    /// outline of C26 (always Through), 20 seeded random poses each, with hardness 0.3, 0.8 and 1.0; and the fast
    /// paths are really taken.
    #[test]
    fn folded_field_dabs_equal_unfolded() {
        let fields = [
            ("cube", TipField::cube()),
            ("cylinder", TipField::cylinder()),
            ("overlapping", overlapping_cubes_field()),
            ("star outline", star_outline_field()),
            ("baked overlapping", baked_field(samples::overlapping_cubes())),
        ];
        let mut rng = XorShift::new(16);
        let (mut folded_runs, mut full) = (0usize, 0usize);
        for (name, field) in &fields {
            for pose in 0..20 {
                let mut f = Fixture::new(samples::wrapped_quad(), 512);
                let rotation = Mat3::from_quat(
                    glam::Quat::from_xyzw(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5)
                        .normalize(),
                );
                let center = Vec3::new(0.3 + 0.4 * rng.next_f32(), 0.3 + 0.4 * rng.next_f32(), (rng.next_f32() - 0.5) * 0.3);
                let hardness = [0.3f32, 0.8, 1.0][pose % 3];
                let radius = 0.5 + 0.3 * rng.next_f32();
                let through = field.is_flat() || pose % 4 == 3;
                let shape = shape_of(Some(field), center, rotation, radius, through, false);
                let cands = tip_cands(&f, &shape, hardness);
                assert!(!cands.is_empty());
                for c in &cands {
                    let mut runs = Vec::new();
                    raster_cand(&f.mesh, &f.map, c, 0..512, &shape, false, &mut |_, _, _| {}, &mut runs);
                    folded_runs += runs.len();
                }
                let plain: Vec<DabTri> = cands.iter().map(|c| DabTri { full_core: false, blocks: None, ..c.clone() }).collect();
                let mut reference = StrokeScratch::new(512, 512);
                rasterize_tip(&f.mesh, &f.map, &mut reference, &plain, &shape);
                rasterize_tip(&f.mesh, &f.map, &mut f.scratch, &cands, &shape);
                assert_eq!(f.scratch.dab_cov, reference.dab_cov, "{name}, pose {pose}, hardness {hardness}, through {through}");
                full += reference.dab_cov.iter().filter(|&&c| c == 255).count();
                assert!(reference.dab_cov.iter().any(|&c| c > 0), "{name}, pose {pose}: nothing painted");
            }
        }
        assert!(folded_runs > 1000 && full > 100_000, "the fast paths must be taken ({folded_runs} folded runs, {full} full texels)");
    }

    /// C17: a field dab of more than 65,536 texels gives the same coverage on the pool as serially (one `raster_tri`
    /// per candidate over all rows, unfolded).
    #[test]
    fn a_large_field_dab_is_the_same_on_the_pool() {
        let cube = TipField::cube();
        for (hardness, through) in [(0.6f32, false), (1.0, true)] {
            let mut f = Fixture::new(samples::flat_quad(), 512);
            let shape = shape_of(Some(&cube), Vec3::new(0.5, 0.5, 0.0), follow(Vec3::Z) * Mat3::from_rotation_y(0.3), 200.0 / 512.0, through, false);
            let cands = tip_cands(&f, &shape, hardness);
            let work: u64 = cands.iter().map(|k| k.extent(512, 512).0).sum();
            assert!(work >= PARALLEL_TEXELS, "the test must exercise the parallel path ({work})");
            rasterize_tip(&f.mesh, &f.map, &mut f.scratch, &cands, &shape);
            let mut serial = vec![0u8; 512 * 512];
            for k in &cands {
                raster_tri(&f.mesh, &f.map, k.tri, k.clip, 0..512, &mut |fr| {
                    let i = (fr.y * 512 + fr.x) as usize;
                    serial[i] = serial[i].max(k.coverage_in(&fr, &shape));
                });
            }
            assert!(serial.iter().filter(|&&c| c > 0).count() > 10_000);
            assert_eq!(f.scratch.dab_cov, serial, "hardness {hardness}, through {through}");
        }
    }

    /// C18: a field dab with a radius of 0.4 texels covers the texel under its centre (it is painted as a hard ball).
    #[test]
    fn a_tiny_field_dab_covers_the_cursor_texel() {
        let cube = TipField::cube();
        let texel = 1.0 / 64.0;
        for (fx, fy) in [(0.5, 0.5), (0.1, 0.5), (0.9, 0.9), (0.02, 0.02)] {
            let mut f = Fixture::new(samples::flat_quad(), 64);
            let (u, v) = ((10.0 + fx) * texel, (12.0 + fy) * texel);
            let shape = shape_of(Some(&cube), Vec3::new(u, 1.0 - v, 0.0), follow(Vec3::Z), 0.4 * texel, false, false);
            assert!(!tip_dab(&mut f, &shape, 0.3).is_empty());
            assert!(f.scratch.mask(10, 12) > 0, "centre at ({fx}, {fy}) of the texel");
        }
    }

    /// C19: for 200 seeded random poses (Cube and Cylinder, Solid and Through, offsets) on `uv_sphere`, every
    /// non-gutter texel with a non-zero mask has its surface point within `R_b` plus one texel of `c'`.
    #[test]
    fn field_paint_never_leaves_the_bound() {
        let fields = [TipField::cube(), TipField::cylinder()];
        let mut rng = XorShift::new(19);
        let mut painted = 0usize;
        for pose in 0..200 {
            let mut f = Fixture::new(samples::uv_sphere(24, 12, 1.0), 128);
            let field = &fields[pose % 2];
            let dir = Vec3::new(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5).try_normalize().unwrap_or(Vec3::Z);
            let center = dir * (1.0 + (rng.next_f32() - 0.5) * 0.3);
            let rotation = follow(dir)
                * Mat3::from_quat(glam::Quat::from_euler(glam::EulerRot::YXZ, rng.next_f32() * 6.3, rng.next_f32() * 6.3, rng.next_f32() * 6.3));
            let radius = 0.05 + 0.4 * rng.next_f32();
            let hardness = rng.next_f32();
            let shape = shape_of(Some(field), center, rotation, radius, pose % 3 == 0, false);
            tip_dab(&mut f, &shape, hardness);
            let aa = set_texel_world(&f.map);
            for (x, y, p, m) in f.covered(|_| true) {
                if m > 0 {
                    let (tri, _) = f.map.owner(x, y).unwrap();
                    let limit = shape.reach + tri_texel_world(&f.mesh, &f.map, tri, aa);
                    assert!(p.distance(center) <= limit, "pose {pose}: texel ({x}, {y}) at {p}, {} > {limit}", p.distance(center));
                    painted += 1;
                }
            }
        }
        assert!(painted > 10_000, "{painted}");
    }

    /// C23: thin parts are not faded as a whole. The fin shape (a ball of radius 0.6 and a slab 0.95 x 0.06 x 0.3,
    /// thin along tip Y) on `flat_quad` at 512, radius 150 texels, hardness 0.8, Follow surface: the fin's centre
    /// line between 0.7 and 0.85 tip units is at least 242 (also with Offset 0.03), between 100 and 155 with Offset
    /// 0.054, the centre 255 in all three; with the thickness replaced by the inradius it is under 140.
    #[test]
    fn thin_parts_are_not_faded_as_a_whole() {
        let cell = 0.008f32;
        let fin = TipField::from_fn([251, 161, 161], Vec3::new(-1.0, -0.64, -0.64), cell, |p| {
            let ball = p.length() - 0.6;
            let q = p.abs() - Vec3::new(0.95, 0.06, 0.3);
            let slab = q.max(Vec3::ZERO).length() + q.max_element().min(0.0);
            ball.min(slab)
        });
        let radius = 150.0 / 512.0;
        let frame = follow(Vec3::Z);
        // The centre texel (256, 256) has its centre at (256.5 / 512, 1 - 256.5 / 512).
        let centre = Vec3::new(256.5 / 512.0, 1.0 - 256.5 / 512.0, 0.0);
        // Tip X is world +X here; the fin's centre line runs along it at world y = centre.y.
        let line = |f: &Fixture| -> Vec<u8> {
            (0..512u32)
                .filter_map(|x| {
                    let tx = (x as f32 + 0.5 - 256.5) / 150.0;
                    ((0.7..=0.85).contains(&tx.abs())).then(|| cov_of(f.scratch.mask(x, 256)))
                })
                .collect()
        };
        for (offset, inradius, lo, hi) in [(0.0f32, false, 242u8, 255u8), (0.03, false, 242, 255), (0.054, false, 100, 155), (0.0, true, 0, 139)] {
            let mut f = Fixture::new(samples::flat_quad(), 512);
            // Offset moves the tip along the facing normal by `offset` radii (= tip units at scale 1).
            let shape = shape_of(Some(&fin), centre + Vec3::Z * offset * radius, frame, radius, false, inradius);
            tip_dab(&mut f, &shape, 0.8);
            let values = line(&f);
            assert!(values.len() >= 40, "{values:?}");
            assert!(
                values.iter().all(|&v| v >= lo && v <= hi),
                "offset {offset}, inradius {inradius}: centre line {values:?} not in {lo}..={hi}"
            );
            if !inradius {
                assert_eq!(cov_of(f.scratch.mask(256, 256)), 255, "offset {offset}: the dab centre");
            }
        }
    }

    /// FEATURE_BRUSH 5.4 step 6 at the engine level: a set whose candidates are all beyond the shape's box gets an
    /// empty list (not reached), while the ball of the same reach lists them.
    #[test]
    fn the_early_reject_decides_the_reach_of_a_field() {
        let f = Fixture::new(samples::flat_quad(), 64);
        let cube = TipField::cube();
        // The cube's box reaches 0.577 * 0.3 + a cell below its centre: a centre 0.25 above the quad leaves it outside,
        // while the bounding ball (radius 0.3 * 1.03) still reaches the quad.
        let centre = Vec3::new(0.5, 0.5, 0.25);
        let field = shape_of(Some(&cube), centre, follow(Vec3::Z), 0.3, false, false);
        let ball = shape_of(None, centre, follow(Vec3::Z), field.reach, false, false);
        let mut q = DabQuery::default();
        shared_candidates(&f.mesh, &f.bvh, 1, |_| Some(&f.map), &ball, Vec3::Z, SurfaceReach::All, &mut q);
        assert_eq!(q.lists[0].len(), 2);
        shared_candidates(&f.mesh, &f.bvh, 1, |_| Some(&f.map), &field, Vec3::Z, SurfaceReach::All, &mut q);
        assert!(q.lists[0].is_empty());
        // Skipped sets and invalid centres give empty lists.
        shared_candidates(&f.mesh, &f.bvh, 1, |_| None, &ball, Vec3::Z, SurfaceReach::All, &mut q);
        assert!(q.lists[0].is_empty());
        let nan = DabShape { center: Vec3::NAN, ..ball };
        shared_candidates(&f.mesh, &f.bvh, 1, |_| Some(&f.map), &nan, Vec3::Z, SurfaceReach::All, &mut q);
        assert!(q.lists[0].is_empty());
    }

    // ----- Round 7 (A2: the fused parallel pass; the RASTER_R5 seam) -----

    /// Runs `f` with the round-5 seam on (`dab_tip` and `dab_3d` paint with round 5's rasterizer and commit).
    fn with_r5<T>(f: impl FnOnce() -> T) -> T {
        RASTER_R5.with(|r| r.set(true));
        let out = f();
        RASTER_R5.with(|r| r.set(false));
        out
    }

    fn rect_set(rects: &[IRect]) -> BTreeSet<(i32, i32, i32, i32)> {
        rects.iter().map(|r| (r.x0, r.y0, r.x1, r.y1)).collect()
    }

    /// A fixture with a `w` x `h` map of the given gutter.
    fn fixture_wh(raw: RawMesh, w: u32, h: u32, gutter: u8) -> Fixture {
        let mesh = mesh_of(raw);
        let bvh = Bvh::build(&mesh);
        let map = TexelMap::build(&mesh, SET, w, h, gutter);
        Fixture { mesh, bvh, map, scratch: StrokeScratch::new(w, h) }
    }

    /// A scratch of the size of `s` with its mask (and empty per-dab state).
    fn scratch_like(s: &StrokeScratch) -> StrokeScratch {
        let mut out = StrokeScratch::new(s.width, s.height);
        out.mask.copy_from_slice(&s.mask);
        out
    }

    /// A random point on a random triangle and the triangle's normal.
    fn random_anchor(mesh: &MeshData, rng: &mut XorShift) -> (Vec3, Vec3) {
        let t = rng.next_u32() % mesh.tri_count();
        let p = mesh.tri_positions(t);
        let (a, b) = (rng.next_f32(), rng.next_f32());
        let (a, b) = if a + b > 1.0 { (1.0 - a, 1.0 - b) } else { (a, b) };
        (p[0] + (p[1] - p[0]) * a + (p[2] - p[0]) * b, mesh.geo_normal(t))
    }

    /// One dab of `shape` through the shared path (`shared_candidates` + `dab_tip`).
    fn shared_dab(f: &mut Fixture, shape: &DabShape, normal: Vec3, reach: SurfaceReach, hardness: f32, strength: f32) -> Vec<IRect> {
        let mut q = DabQuery::default();
        shared_candidates(&f.mesh, &f.bvh, 1, |_| Some(&f.map), shape, normal, reach, &mut q);
        dab_tip(&f.mesh, &f.map, &mut f.scratch, shape, &q.lists[0], hardness, strength)
    }

    /// The box work of the last dab (`raster_commit` leaves the kept candidates' boxes in the scratch).
    fn last_work(s: &StrokeScratch) -> u64 {
        s.boxes.iter().map(|b| ((b[2] - b[0]) as u64).min(s.width as u64) * ((b[3] - b[1]) as u64).min(s.height as u64)).sum()
    }

    /// T1: random dabs (ball hard / soft, Cube Solid and Through, SameSide / All, radii from under a texel to the whole
    /// model) on round 7's meshes and maps of 1 x 1 to 1024 x 1024 with gutters 0 to 16 paint exactly round 5's masks
    /// and rects (the `RASTER_R5` seam), and leave the per-dab scratch empty.
    #[test]
    fn r7_dabs_equal_round5_on_random_dabs() {
        let cube = TipField::cube();
        let mut rng = XorShift::new(0x5eed7);
        let (mut dabs_done, mut painted, mut banded) = (0usize, 0usize, 0usize);
        for (name, raw) in crate::texel_map::r7_test_meshes() {
            for &(w, h, gutter, dabs) in &[(1u32, 1u32, 0u8, 3usize), (3, 5, 2, 3), (64, 64, 8, 8), (300, 420, 16, 6), (1024, 1024, 2, 3)] {
                let mut r7 = fixture_wh(raw.clone(), w, h, gutter);
                let mut r5 = fixture_wh(raw.clone(), w, h, gutter);
                let texel = set_texel_world(&r7.map);
                let diag = r7.mesh.diag();
                for k in 0..dabs {
                    let (c, n) = random_anchor(&r7.mesh, &mut rng);
                    let radius = match (k + rng.next_u32() as usize) % 5 {
                        0 => 0.3 * texel,
                        1 => 2.0 * texel,
                        2 => 12.0 * texel,
                        3 => 60.0 * texel,
                        _ => diag,
                    };
                    let reach = if rng.next_u32().is_multiple_of(2) { SurfaceReach::SameSide } else { SurfaceReach::All };
                    let strength = 0.3 + 0.7 * rng.next_f32();
                    let (field, through, hardness) = match rng.next_u32() % 4 {
                        0 => (None, false, 1.0),
                        1 => (None, false, 0.5),
                        2 => (Some(&cube), false, 0.8),
                        _ => (Some(&cube), true, 0.8),
                    };
                    let shape = shape_of(field, c, follow(n), radius, through, false);
                    let got = shared_dab(&mut r7, &shape, n, reach, hardness, strength);
                    let want = with_r5(|| shared_dab(&mut r5, &shape, n, reach, hardness, strength));
                    let what = format!("{name} {w}x{h} g{gutter}: dab {k} radius {radius} field {} through {through}", field.is_some());
                    assert_eq!(rect_set(&got), rect_set(&want), "{what}");
                    assert!(r7.scratch.mask == r5.scratch.mask, "{what}: masks differ");
                    assert!(r7.scratch.dab_cov.iter().all(|&c| c == 0) && r7.scratch.dab_touched.is_empty(), "{what}");
                    dabs_done += 1;
                    painted += usize::from(!got.is_empty());
                    banded += usize::from(last_work(&r7.scratch) >= PAR_DAB_WORK);
                }
            }
        }
        assert!(painted > dabs_done / 3 && banded >= 10, "{dabs_done} dabs, {painted} painted, {banded} banded");
    }

    /// T4: the fused pass with bands of 16, 32 and 64 rows (forced, also for small dabs) gives the masks and rects of
    /// the serial loop + `commit_dab` and of round 5's code, on sparse, dense and wrapped dabs, on images whose height
    /// is not a multiple of the band.
    #[test]
    fn fused_band_commit_equals_the_serial_commit() {
        let mut rng = XorShift::new(0xf05ed);
        let mut checked = 0;
        for (raw, w, h) in [
            (tiled_strip(3, 2.3), 100u32, 75u32),
            (samples::uv_sphere(24, 12, 1.0), 257, 131),
            (samples::wrapped_quad(), 64, 200),
            (samples::seam_cube(1.0), 90, 33),
            (samples::mirrored_quads(), 70, 45),
        ] {
            let mut f = fixture_wh(raw, w, h, TexelMap::default_gutter(w, h));
            let texel = set_texel_world(&f.map);
            let diag = f.mesh.diag();
            for k in 0..9 {
                let (c, n) = random_anchor(&f.mesh, &mut rng);
                let radius = [1.5 * texel, 8.0 * texel, 0.6 * diag][k % 3];
                let shape = DabShape { center: c, reach: radius, field: None };
                let mut q = DabQuery::default();
                shared_candidates(&f.mesh, &f.bvh, 1, |_| Some(&f.map), &shape, n, SurfaceReach::All, &mut q);
                let aa = set_texel_world(&f.map);
                let cands: Vec<DabTri> = q.lists[0].iter().filter_map(|&t| prepare_tri(&f.mesh, &f.map, t, &shape, 0.7, aa)).collect();
                // Round 5.
                let mut r5 = scratch_like(&f.scratch);
                rasterize_tip(&f.mesh, &f.map, &mut r5, &cands, &shape);
                let want_rects = commit_dab(&mut r5, 0.6);
                // Serial.
                let mut serial = scratch_like(&f.scratch);
                let rects = raster_commit(&f.mesh, &f.map, &mut serial, &mut cands.clone(), &shape, 0.6, DAB_BAND_ROWS, u64::MAX);
                assert_eq!(rects, want_rects, "{w}x{h} dab {k}: serial");
                assert!(serial.mask == r5.mask);
                for band_rows in [16u32, 32, 64] {
                    let mut fused = scratch_like(&f.scratch);
                    let rects = raster_commit(&f.mesh, &f.map, &mut fused, &mut cands.clone(), &shape, 0.6, band_rows, 0);
                    assert_eq!(rects, want_rects, "{w}x{h} dab {k}: bands of {band_rows}");
                    assert!(fused.mask == r5.mask, "{w}x{h} dab {k}: bands of {band_rows}: masks differ");
                    assert!(fused.dab_cov.iter().all(|&c| c == 0) && fused.dab_touched.is_empty());
                    checked += 1;
                }
                f.scratch = serial;
            }
        }
        assert_eq!(checked, 5 * 9 * 3);
    }

    /// T2: large dabs on every path (serial, banded, folded full-core triangles, folded core blocks of tiled ones)
    /// paint the same masks and rects inside pools of 1, 2 and 5 threads; a two-set document stroke paints the same
    /// layer bytes.
    #[test]
    fn dab_raster_is_the_same_on_any_thread_count() {
        use crate::mesh::{RawMaterial, RawTriangle};
        let long_quads = || {
            let tri = |p: [Vec3; 3], uv: [[f32; 2]; 3]| RawTriangle { positions: p, normals: None, uvs: Some(uv.map(Vec2::from_array)), material: 0 };
            let mut triangles = Vec::new();
            for q in 0..3 {
                let y = q as f32;
                let (a, b, c, d) = (Vec3::new(0.0, y, 0.0), Vec3::new(8.0, y, 0.0), Vec3::new(8.0, y + 1.0, 0.0), Vec3::new(0.0, y + 1.0, 0.0));
                triangles.push(tri([a, b, d], [[0.0, 0.0], [7.5, 0.0], [0.0, 1.3]]));
                triangles.push(tri([b, c, d], [[7.5, 0.0], [7.5, 1.3], [0.0, 1.3]]));
            }
            RawMesh { triangles, materials: vec![RawMaterial { name: "m".into(), base_color: [1.0; 4], double_sided: false, texture: None }] }
        };
        // (mesh, size, dabs: centre, normal, radius, hardness)
        type Scene = (RawMesh, u32, Vec<(Vec3, Vec3, f32, f32)>);
        let scenes: Vec<Scene> = vec![
            (samples::uv_sphere(32, 16, 1.0), 512, vec![(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, 0.02, 0.8), (Vec3::new(0.3, 0.2, 0.93), Vec3::Z, 0.9, 0.6)]),
            (tiled_strip(5, 3.3), 64, vec![(Vec3::new(2.5, 0.5, 0.05), Vec3::Z, 2.0, 0.8), (Vec3::new(1.2, 0.4, 0.0), Vec3::Z, 0.7, 0.5)]),
            (long_quads(), 128, vec![(Vec3::new(3.3, 1.4, 0.1), Vec3::Z, 2.0, 0.9), (Vec3::new(5.0, 0.5, 0.0), Vec3::Z, 3.0, 0.6)]),
        ];
        let cube = TipField::cube();
        let run = |threads: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            pool.install(|| {
                let mut out = Vec::new();
                let mut banded = 0;
                for (raw, size, dabs) in &scenes {
                    let mut f = Fixture::new(raw.clone(), *size);
                    for &(c, n, radius, hardness) in dabs {
                        let rects = f.dab(c, n, radius, hardness, 0.8);
                        banded += usize::from(last_work(&f.scratch) >= PAR_DAB_WORK);
                        let shape = shape_of(Some(&cube), c, follow(n), radius, false, false);
                        let tip_rects = shared_dab(&mut f, &shape, n, SurfaceReach::All, hardness, 0.6);
                        out.push((rects, tip_rects, f.masks()));
                    }
                }
                (out, banded, document_layers())
            })
        };
        let one = run(1);
        assert!(one.1 >= 3, "only {} dabs took the banded path", one.1);
        for threads in [2, 5] {
            let other = run(threads);
            assert!(other.0 == one.0, "{threads} threads: masks or rects differ");
            assert_eq!(other.2, one.2, "{threads} threads: layer bytes differ");
        }
    }

    /// The layer hashes of a two-set document after a large and a small stroke across the body and the head.
    fn document_layers() -> Vec<u64> {
        let mut doc = crate::testutil::doc_from_raw(samples::two_material_model(), 256);
        let hits: Vec<_> = (0..12)
            .filter_map(|i| doc.raycast(&crate::math::Ray::new(Vec3::new(-0.45 + 0.08 * i as f32, 3.0, 0.05), Vec3::NEG_Y)))
            .collect();
        assert!(hits.len() >= 10);
        for radius in [0.35, 0.04] {
            let b = BrushParams { radius, hardness: 0.6, color: [200, 30, 90, 255], ..BrushParams::default() };
            doc.stroke_begin(&b).unwrap();
            hits.iter().for_each(|h| doc.stroke_to_3d(h, 1.0));
            doc.stroke_end();
        }
        doc.set_ids()
            .into_iter()
            .filter_map(|s| match doc.paint_target(s) {
                crate::PaintTarget::Ready(layer) => Some(crate::testutil::hash_pixels(&crate::testutil::layer_dense(&doc, layer))),
                _ => None,
            })
            .collect()
    }

    // ----- Round 7 check agent: adversarial cases beyond T1-T6 -----

    /// Check agent (round 7): 1 x N, N x 1 and tiny images, heights that leave a short last band, gutters 0 / 1 / 16,
    /// boxes wider than the image (huge radii, tiled UVs), a multi-set soup, slivers, stacked and mirrored UVs,
    /// repeated dabs on one spot (saturated masks: increments of 0), ball hard / soft and Cube Solid / Through tips.
    /// Each dab through `dab_tip` equals round 5's code; its candidates through the forced fused pass (bands of 16 /
    /// 32 / 64 rows, pools of 1 and 3 threads) give the same masks and rects and leave the scratch clean; A4's row
    /// spans cover every texel whose mask rose.
    #[test]
    fn check_r7_adversarial_dabs_equal_round5() {
        let cube = TipField::cube();
        let mut rng = XorShift::new(0xc4ec7);
        let pools = [1usize, 3].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
        let mut meshes = crate::texel_map::r7_test_meshes();
        meshes.push(("tiled_strip", tiled_strip(5, 3.3)));
        meshes.push(("soup_3_sets", samples::soup_sets(11, 400, 3)));
        let (mut dabs_done, mut painted, mut fused_checked) = (0usize, 0usize, 0usize);
        for (name, raw) in meshes {
            for &(w, h, gutter) in &[(1u32, 97u32, 0u8), (97, 1, 1), (2, 300, 16), (333, 7, 3), (129, 65, 1), (64, 33, 16), (200, 97, 0)] {
                let mut r7 = fixture_wh(raw.clone(), w, h, gutter);
                let mut r5 = fixture_wh(raw.clone(), w, h, gutter);
                let texel = set_texel_world(&r7.map);
                let diag = r7.mesh.diag();
                for k in 0..6usize {
                    let (c, n) = random_anchor(&r7.mesh, &mut rng);
                    let radius = [0.1 * texel, texel, 6.0 * texel, 40.0 * texel, 0.3 * diag, 3.0 * diag][k];
                    let reach = if rng.next_u32().is_multiple_of(2) { SurfaceReach::SameSide } else { SurfaceReach::All };
                    let strength = if k == 5 { 1.0 } else { 0.2 + 0.8 * rng.next_f32() };
                    let (field, through, hardness) = match rng.next_u32() % 4 {
                        0 => (None, false, 1.0),
                        1 => (None, false, 0.4),
                        2 => (Some(&cube), false, 0.8),
                        _ => (Some(&cube), true, 1.0),
                    };
                    let shape = shape_of(field, c, follow(n), radius, through, false);
                    for rep in 0..(1 + k % 2) {
                        let what = format!("{name} {w}x{h} g{gutter}: dab {k} rep {rep} radius {radius} field {} through {through}", field.is_some());
                        // The forced fused pass on copies of the mask before the dab.
                        let mut q = DabQuery::default();
                        shared_candidates(&r7.mesh, &r7.bvh, 1, |_| Some(&r7.map), &shape, n, reach, &mut q);
                        let aa = set_texel_world(&r7.map);
                        let cands: Vec<DabTri> =
                            q.lists[0].iter().filter_map(|&t| prepare_tri(&r7.mesh, &r7.map, t, &shape, hardness, aa)).collect();
                        let mut fused = Vec::new();
                        for band_rows in [16u32, 32, 64] {
                            for pool in &pools {
                                let mut s = scratch_like(&r7.scratch);
                                let rects = pool.install(|| {
                                    raster_commit(&r7.mesh, &r7.map, &mut s, &mut cands.clone(), &shape, strength, band_rows, 0)
                                });
                                let clean = s.dab_cov.iter().all(|&c| c == 0) && s.dab_touched.is_empty();
                                fused.push((band_rows, rect_set(&rects), s.mask, clean));
                            }
                        }
                        let before = r7.scratch.mask.clone();
                        let got = shared_dab(&mut r7, &shape, n, reach, hardness, strength);
                        let want = with_r5(|| shared_dab(&mut r5, &shape, n, reach, hardness, strength));
                        assert_eq!(rect_set(&got), rect_set(&want), "{what}");
                        assert!(r7.scratch.mask == r5.scratch.mask, "{what}: masks differ");
                        assert!(r7.scratch.dab_cov.iter().all(|&c| c == 0) && r7.scratch.dab_touched.is_empty(), "{what}");
                        if !cands.is_empty() {
                            for (band_rows, rects, mask, clean) in fused {
                                assert_eq!(rects, rect_set(&want), "{what}: fused, bands of {band_rows}");
                                assert!(mask == r5.scratch.mask, "{what}: fused, bands of {band_rows}: masks differ");
                                assert!(clean, "{what}: fused, bands of {band_rows}: scratch not clean");
                                fused_checked += 1;
                            }
                        }
                        // A4: every texel whose mask rose lies in the changed span of its row.
                        for (i, (&a, &b)) in before.iter().zip(&r7.scratch.mask).enumerate() {
                            if a != b {
                                let (x, y) = (i as u32 % w, i as u32 / w);
                                let (lo, hi) = r7.scratch.changed_span(y, 0, w).expect("spans after a 3D dab");
                                assert!(lo <= x && x < hi, "{what}: texel ({x}, {y}) outside its row span {lo}..{hi}");
                            }
                        }
                        dabs_done += 1;
                        painted += usize::from(!got.is_empty());
                    }
                }
            }
        }
        assert!(painted > dabs_done / 3 && fused_checked > 1000, "{dabs_done} dabs, {painted} painted, {fused_checked} fused");
    }
}
