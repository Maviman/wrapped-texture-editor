// Owner: round 6, package A (R6-1). The surface parameterization of a wrapped stamp (ARCHITECTURE.md 6.5a, plan
// sections 2 and 4.3): strip-seeded geodesic unfolding. Every vertex near the stamp gets 2D coordinates (stamp-local,
// world units along the projector's right / up, origin at the anchor); `wrap_bake.rs` paints through them.
//
// Choices the plan leaves open, all documented at the items below:
// - Orientation is propagated combinatorially (a triangle inherits the orientation of an already oriented neighbour
//   across a shared edge, from the edge's traversal directions), not from normals: exact at 90 and 180 degree folds
//   (box edges, a thin sheet's rim), on inconsistent winding and on a Moebius strip. Normals decide only where no
//   oriented neighbour exists.
// - The reach is rounded up to a power of 1.25 of the model diagonal ("bucketed", 0-25 % to spare): the coordinates
//   depend on the reach (spine length, front order), so a parameterization serves another projector only for the same
//   bucketed reach; a fresh one for that projector is then bit-identical (4.4).
// - The two traces of the spine advance alternately by arc length, so the ends of a picture longer than the way round
//   meet opposite the anchor.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use glam::{DQuat, DVec2, DVec3, Vec2, Vec3};

use crate::bvh::{Bvh, Cull, closest_point_on_triangle};
use crate::ids::TriId;
use crate::math::Ray;
use crate::mesh::MeshData;
use crate::stamp::{Projector, WrapOptions};
use crate::stamp_bake::MIN_SIZE;
use crate::texel_map::{SurfacePoint, TexelMap};

/// The reach is computed with up to 25 % to spare (rounded up to a power of this factor of the model diagonal), so
/// a slightly larger picture (scale, a longer text) reuses the parameterization (4.4).
pub const WRAP_HEADROOM: f64 = 1.25;
/// Most welded vertices one wrapped stamp lays out; beyond, the front stops (`WrapStats::limited`).
pub const WRAP_MAX_VERTS: usize = 1_000_000;
/// Most candidate triangles; beyond, the nearest ones are kept (`WrapStats::limited`).
pub const WRAP_MAX_TRIS: usize = 3_000_000;
/// Boundary vertices closer than this (times the model diagonal) are welded: cracks close.
pub const WELD_SNAP: f64 = 1e-5;
/// Candidates of a vertex agree when all lie within this share of its shortest incident edge of their mean.
pub const CONSENSUS: f64 = 0.25;
/// A triangle whose parameter edge is stretched beyond this factor (either way), or flipped, is a tear.
pub const TEAR_RATIO: f64 = 2.0;
/// A spine exit this close (share of the edge) to a vertex is moved off it.
pub const SPINE_NUDGE: f64 = 1e-6;
/// Jump small gaps: the largest gap crossed, times the model diagonal.
pub const GAP_JUMP: f64 = 0.01;
/// Overlap: the most turns laid.
pub const WRAP_MAX_TURNS: u32 = 8;
/// Anchor: candidate triangles within this distance (times the diagonal) of the nearest one compete for the anchor.
const ANCHOR_TIE: f64 = 1e-4;
/// Round 8: the front stops at this many times the bucketed across reach (geodesic distance from the spine), so a
/// relaxed layout still covers the picture. The plan's 1.3 left the picture's corners on halo triangles on WA4 at 60
/// degrees (13 halo triangles inside the picture, dup 1.5 %, a corner cell 2.16 x); 1.7 leaves none there (builder's
/// measurements: 1.5 and 1.7 met WA4b, 1.7 with more margin).
pub const WRAP_STOP_SLACK: f64 = 1.7;

// Round 8: seams, wedges, the relaxation and the owner grid (`wrap_relax.rs`, a child module: it reads the patch and
// the layout state).
#[path = "wrap_relax.rs"]
mod relax;
pub(crate) use relax::{OwnerGrid, owner_scale};
/// check:A (round 8): the relaxation's parallel threshold, lowered by a test of the parallel path.
#[cfg(test)]
pub(crate) fn relax_par_min_for_tests() -> &'static std::sync::atomic::AtomicUsize {
    &relax::PAR_MIN_TEST
}
pub use relax::{BURY_REACH, CONE_CUT_DEG, ISO_TOL, OWNER_MAX_CELLS, OWNER_SAME, PAR_CHUNK, PAR_MIN_UNKNOWNS, RELAX_ARAP_ITERS, RELAX_CG_BUDGET, SEAM_NEAR};

/// What a parameterization covers and how it went (for the panel, the log and the tests).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WrapStats {
    pub verts: u32,
    pub tris: u32,
    /// Triangles re-unfolded as tears (4.3 step 6).
    pub cut_tris: u32,
    /// Share of the picture rectangle that has surface under it (parameter areas, 0..1).
    pub covered: f32,
    /// The spine closed on itself (longer than the way around).
    pub met_itself: bool,
    /// The front met a boundary edge inside the rectangle, or the spine ended at one.
    pub reached_edge: bool,
    /// `WRAP_MAX_VERTS`, `WRAP_MAX_TRIS` or `WRAP_MAX_TURNS` reached.
    pub limited: bool,
    /// Separate parts entered across a gap (Jump small gaps).
    pub jumps: u32,
    /// Layers laid (1 unless Overlap and the spine met itself).
    pub turns: u32,
    /// Time to build.
    pub micros: u64,
    /// Round 8: the layout was relaxed (false: the seed was exact where the picture lies and is kept, 2.2-3).
    pub relaxed: bool,
    /// Round 8: seam edges inside the picture (relaxed layouts; all rules), and per rule S1..S5.
    pub seams: u32,
    pub seams_by_rule: [u32; 5],
    /// Round 8: relaxed triangles inside the picture whose orientation flipped.
    pub flipped: u32,
    /// Round 8: 95th percentile of |area ratio - 1| (layout over 3D) of the triangles inside the picture.
    pub stretch: f32,
    /// Round 8: share of the laid picture area the owner test drops (estimated at the owner grid's cell centres:
    /// round 9, the (cell centre, laid triangle) pairs inside the picture whose triangle is not the owner's place).
    pub rejected: f32,
    /// Round 8: conjugate-gradient iterations of the relaxation (all layers and components).
    pub cg_iters: u32,
    /// Round 9 (T2): the spine's two traces passed beside each other (a helix round a tube) and stopped there (Ends
    /// meet) or started their next turns (Overlap), without sharing a triangle (`met_itself`).
    pub passed: bool,
    /// Round 9 (T2): the share of the picture's length the spine lays when its ends meet (`met_itself` or `passed`
    /// without Overlap turns): `(end0+ + end0-) / (2 x needed along)` clamped to 1; 1 for an open spine and for
    /// Overlap turns (the rest lies on the next turn).
    pub ends_share: f32,
    /// Round 9 (A7): the share of the picture's cells without an owner that lie inside the laid set's outline in the
    /// layout (cut wedges, shrink, holes: an uncovered cell with covered cells on both sides in its row or column), as
    /// against picture beyond a mesh edge or beyond `ends_share` (the top layer's owner grid; 0 without one).
    pub lost_cut: f32,
}

/// Round 8 (plan 3.3): how once and how whole the picture of a wrapped stamp lies on the model, measured from its
/// baked texels (`Document::stamp_wrap_quality`). Picture points are binned in cells of `c = 2 x max(source pixel in
/// world units, texel_world)`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WrapQuality {
    /// `Document::stamp_ink_area` of the stamp.
    pub ink_area: f32,
    /// Share of the inked bins whose surface points spread over more than `6 c` (the same picture spot in two places).
    pub dup: f32,
    /// Share of the 4-neighbour pairs of inked bins whose surface centroids lie more than `6 c` apart (scraps and cut
    /// lines; a seam adds a thin line of such pairs).
    pub scatter: f32,
    /// Inked bins.
    pub bins: u32,
}

/// One layer (turn) of a parameterization: per triangle its stamp-local corner coordinates, ascending `TriId`.
pub(crate) struct WrapLayer {
    pub(crate) tris: Vec<TriId>,
    pub(crate) corners: Vec<[[f32; 2]; 3]>,
    /// Overlap with several turns: the turn's along range `[lo, hi)` on coordinate `axis` (0 = x, 1 = y); a
    /// fragment outside it belongs to another turn.
    pub(crate) clip: Option<[f32; 2]>,
    /// A closed turn: its period along `axis` (a fragment outside `clip` is moved by it).
    pub(crate) period: Option<f32>,
    pub(crate) axis: usize,
    /// Round 8: per triangle the depth `g` (geodesic distance from the spine) of its corners.
    pub(crate) depth: Vec<[f32; 3]>,
    /// Round 8: the owner grid of the owner test (`None` for a layer clipped to an Overlap turn's range); round 9: the
    /// place owner (T1).
    pub(crate) owner: Option<OwnerGrid>,
    /// Uniform grid over the layer's coordinate box for `surface_of` (built on first use).
    grid: OnceLock<Grid>,
}

/// The parameterization of the surface around one wrapped stamp (4.3). Immutable; shared by `Arc`.
pub struct WrapParam {
    /// Address of the `MeshData` it was made for (with `mesh_tag` against an address reused after a free).
    mesh_addr: usize,
    mesh_tag: (u32, u64),
    origin_bits: [u32; 3],
    rotation_bits: [u32; 4],
    spine_y: bool,
    options: WrapOptions,
    /// Bucketed reach along and across the spine (world units).
    reach: [f64; 2],
    /// Overlap with later turns: the exact half extent along the spine the turns were chosen for (bits).
    needed_along: Option<u64>,
    /// The inputs the reach was computed from (for `serves`): source size and largest texel size.
    src_size: [u32; 2],
    texel_world: f32,
    /// The picture's half extents `|half_size|` (bits) `stats.covered` was measured for; a projector that this
    /// parameterization serves with another size (a scale inside the reach bucket) gets its own `covered`
    /// (`stats_for`).
    half_bits: [u32; 2],
    /// Layers bottom first (the last is the top turn). One layer unless Overlap laid several turns.
    pub(crate) layers: Vec<WrapLayer>,
    stats: WrapStats,
}

impl std::fmt::Debug for WrapParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WrapParam").field("reach", &self.reach).field("stats", &self.stats).finish()
    }
}

/// The 2x3 linear part of a development: `M x = (x . rx, x . ry)`.
#[derive(Copy, Clone, Debug, Default)]
struct Frame {
    rx: DVec3,
    ry: DVec3,
}

impl Frame {
    fn apply(&self, x: DVec3) -> DVec2 {
        DVec2::new(x.dot(self.rx), x.dot(self.ry))
    }

    /// The linear map of the affine map that sends the 3D triangle `p` to the 2D triangle `q`; `None` when `p` is
    /// degenerate.
    fn of_triangle(p: [DVec3; 3], q: [DVec2; 3]) -> Option<Frame> {
        let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
        let (f1, f2) = (q[1] - q[0], q[2] - q[0]);
        let (g00, g01, g11) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let det = g00 * g11 - g01 * g01;
        if det.is_nan() || det <= 1e-30 * g00.max(g11).powi(2) || !det.is_finite() {
            return None;
        }
        let (i00, i01, i11) = (g11 / det, -g01 / det, g00 / det);
        // M x = [f1 f2] Ginv [e1.x; e2.x]
        let row = |a: f64, b: f64| (a * i00 + b * i01) * e1 + (a * i01 + b * i11) * e2;
        let frame = Frame { rx: row(f1.x, f2.x), ry: row(f1.y, f2.y) };
        (frame.rx.is_finite() && frame.ry.is_finite()).then(|| frame.isometric(e1, e2)).flatten()
    }

    /// The nearest isometry of the plane of `e1`, `e2` to this map (polar decomposition of its 2x2 part; a
    /// reflection stays a reflection): a frame carries a vertex's orientation and turn, never a scale.
    fn isometric(&self, e1: DVec3, e2: DVec3) -> Option<Frame> {
        let t1 = e1.normalize_or_zero();
        let t2 = (e2 - t1 * e2.dot(t1)).normalize_or_zero();
        if t1 == DVec3::ZERO || t2 == DVec3::ZERO {
            return None;
        }
        let (a, b, c, d) = (self.rx.dot(t1), self.rx.dot(t2), self.ry.dot(t1), self.ry.dot(t2));
        let det = a * d - b * c;
        let (r00, r01, r10, r11) = if det >= 0.0 {
            let n = (a + d).hypot(c - b);
            if n.is_nan() || n <= 0.0 {
                return None;
            }
            let (cs, sn) = ((a + d) / n, (c - b) / n);
            (cs, -sn, sn, cs)
        } else {
            let n = (a - d).hypot(c + b);
            if n.is_nan() || n <= 0.0 {
                return None;
            }
            let (cs, sn) = ((a - d) / n, (c + b) / n);
            (cs, sn, sn, -cs)
        };
        let frame = Frame { rx: t1 * r00 + t2 * r01, ry: t1 * r10 + t2 * r11 };
        (frame.rx.is_finite() && frame.ry.is_finite()).then_some(frame)
    }
}

/// `a x b` in 2D.
fn cross2(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// `v` turned a quarter counter-clockwise.
fn rot90(v: DVec2) -> DVec2 {
    DVec2::new(-v.y, v.x)
}

fn dvec(v: Vec3) -> DVec3 {
    DVec3::new(v.x as f64, v.y as f64, v.z as f64)
}

/// A fast hasher for position bits (a copy of `tip_bake.rs`'s `PositionHasher`).
#[derive(Default)]
struct PositionHasher(u64);

impl Hasher for PositionHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }

    fn write_u128(&mut self, i: u128) {
        let mix = |h: u64, w: u64| (h.rotate_left(5) ^ w).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        self.0 = mix(mix(self.0, i as u64), (i >> 64) as u64);
    }

    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 29)
    }
}

type PosMap<K, V> = HashMap<K, V, BuildHasherDefault<PositionHasher>>;

/// Bits of a position with `-0.0 == 0.0`.
fn position_key(p: Vec3) -> u128 {
    let bits = |c: f32| (if c == 0.0 { 0.0f32 } else { c }).to_bits() as u128;
    bits(p.x) | (bits(p.y) << 32) | (bits(p.z) << 64)
}

/// `x` rounded up to `diag * 1.25^k` (integer k): the bucketed reach.
fn bucket(x: f64, diag: f64) -> f64 {
    if !(x > 0.0 && x.is_finite() && diag > 0.0) {
        return 0.0;
    }
    let k = ((x / diag).ln() / WRAP_HEADROOM.ln() - 1e-9).ceil();
    let k = k.clamp(-400.0, 400.0) as i32;
    let mut r = diag * WRAP_HEADROOM.powi(k);
    // Guard the rounding of `powi`: the bucket always holds `x`.
    while r < x {
        r *= WRAP_HEADROOM;
    }
    r
}

/// The geometry the reach of a projector is computed from.
#[derive(Copy, Clone)]
struct Reach {
    hw: f64,
    hh: f64,
    spine_y: bool,
    /// Bucketed reach along and across the spine.
    reach: [f64; 2],
    /// The picture's half extent along the spine plus one source texel (where its last later turn may start).
    needed_along: f64,
}

/// The reach of `p` (4.3 step 1); `None` for an unusable projector.
fn reach_of(p: &Projector, src_size: [u32; 2], texel_world: f32, diag: f64) -> Option<Reach> {
    let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
    if !p.is_finite() || !(hw > MIN_SIZE && hh > MIN_SIZE) || src_size[0] == 0 || src_size[1] == 0 {
        return None;
    }
    let (hw, hh) = (hw as f64, hh as f64);
    let (bx, by) = (2.0 * hw / src_size[0] as f64, 2.0 * hh / src_size[1] as f64);
    let tw = if texel_world.is_finite() && texel_world > 0.0 { texel_world as f64 } else { 0.0 };
    let spine_y = hh > hw;
    let (along, across) = if spine_y { (hh + by, hw + bx) } else { (hw + bx, hh + by) };
    let reach = [bucket(along + 2.0 * tw, diag), bucket(across + 2.0 * tw, diag)];
    (reach[0] > 0.0 && reach[1] > 0.0).then_some(Reach { hw, hh, spine_y, reach, needed_along: along })
}

/// Largest finite `texel_world` of the maps.
fn max_texel_world(maps: &[std::sync::Arc<TexelMap>]) -> f32 {
    maps.iter().map(|m| m.stats.texel_world).filter(|t| t.is_finite()).fold(0.0f32, f32::max)
}

// ----- the patch: welded candidate triangles -----

/// The candidate triangles around the anchor, welded by position (4.3 step 2).
struct Patch {
    /// Mesh triangle of each local triangle, ascending.
    tris: Vec<TriId>,
    /// Welded vertices of each local triangle.
    tv: Vec<[u32; 3]>,
    pos: Vec<DVec3>,
    /// Unit winding normal per local triangle (zero when degenerate).
    geo: Vec<DVec3>,
    /// Vertex -> local triangles (CSR).
    vt_start: Vec<u32>,
    vt: Vec<u32>,
    /// Half-edge `3 t + k` (the edge opposite corner k) -> the range of its edge group in `edge_tris`.
    he_group: Vec<(u32, u32)>,
    /// Half-edges grouped by edge (sorted by welded vertex pair).
    edge_tris: Vec<u32>,
    /// Part (connected component) per vertex, numbered by first vertex.
    part: Vec<u32>,
    part_count: u32,
    /// Per part: bounding box (min, max) of its vertices.
    part_box: Vec<(DVec3, DVec3)>,
    /// Vertex lies on a boundary edge.
    on_boundary: Vec<bool>,
    /// Shortest incident edge per vertex (3D).
    min_edge: Vec<f64>,
    /// Round 8: longest incident edge per vertex (3D; seam rule S2).
    max_edge: Vec<f64>,
    /// Rank of each vertex in position order (`position_order`): the tie-break of every vertex comparison.
    rank: Vec<u32>,
    diag: f64,
}

impl Patch {
    fn vtris(&self, v: u32) -> &[u32] {
        &self.vt[self.vt_start[v as usize] as usize..self.vt_start[v as usize + 1] as usize]
    }

    /// The other half-edges of the edge opposite corner `k` of local triangle `t`.
    fn across(&self, t: u32, k: usize) -> impl Iterator<Item = (u32, usize)> + '_ {
        let (start, len) = self.he_group[t as usize * 3 + k];
        self.edge_tris[start as usize..(start + len) as usize]
            .iter()
            .filter(move |&&he| he / 3 != t)
            .map(|&he| (he / 3, (he % 3) as usize))
    }

    fn degenerate(&self, t: u32) -> bool {
        self.geo[t as usize] == DVec3::ZERO
    }

    fn corners(&self, t: u32) -> [DVec3; 3] {
        self.tv[t as usize].map(|v| self.pos[v as usize])
    }

    /// Builds the patch of `candidates` (mesh triangles, ascending).
    fn build(mesh: &MeshData, candidates: Vec<TriId>) -> Patch {
        let diag = mesh.diag() as f64;
        // Weld by bit-exact position.
        let mut map: PosMap<u128, u32> = PosMap::with_capacity_and_hasher(candidates.len() * 2, Default::default());
        let mut pos: Vec<DVec3> = Vec::new();
        let mut tv: Vec<[u32; 3]> = Vec::with_capacity(candidates.len());
        for &t in &candidates {
            let corners = mesh.tri(t).map(|i| {
                let p = mesh.positions[i as usize];
                *map.entry(position_key(p)).or_insert_with(|| {
                    pos.push(dvec(p));
                    (pos.len() - 1) as u32
                })
            });
            tv.push(corners);
        }
        drop(map);
        // Boundary snap: boundary-edge vertices within WELD_SNAP of another one are joined.
        let snap = WELD_SNAP * diag;
        let edges = edge_groups(&tv);
        let mut boundary = vec![false; pos.len()];
        for group in edges.groups() {
            if group.len() == 1 {
                let he = group[0];
                let (t, k) = (he / 3, (he % 3) as usize);
                let [a, b] = edge_of(tv[t as usize], k);
                boundary[a as usize] = true;
                boundary[b as usize] = true;
            }
        }
        let mut rep: Vec<u32> = (0..pos.len() as u32).collect();
        if snap > 0.0 && boundary.iter().any(|&b| b) {
            let cell = |p: DVec3| -> [i64; 3] { [(p.x / snap).floor() as i64, (p.y / snap).floor() as i64, (p.z / snap).floor() as i64] };
            let mut grid: PosMap<[i64; 3], Vec<u32>> = PosMap::default();
            for v in 0..pos.len() as u32 {
                if !boundary[v as usize] {
                    continue;
                }
                let p = pos[v as usize];
                let c = cell(p);
                let mut best: Option<u32> = None;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let Some(list) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else { continue };
                            for &u in list {
                                if pos[u as usize].distance(p) <= snap && best.is_none_or(|b| position_order(pos[u as usize], pos[b as usize]).is_lt()) {
                                    best = Some(u);
                                }
                            }
                        }
                    }
                }
                match best {
                    Some(u) => rep[v as usize] = u,
                    None => grid.entry(c).or_default().push(v),
                }
            }
        }
        // Apply the snap, drop collapsed triangles, renumber the used vertices in order of first use.
        let mut new_id = vec![u32::MAX; pos.len()];
        let mut new_pos = Vec::with_capacity(pos.len());
        let mut tris = Vec::with_capacity(candidates.len());
        let mut new_tv = Vec::with_capacity(candidates.len());
        // Nothing snapped and nothing dropped: the renumbering (first use, same order) is the identity, so the edge
        // groups of the weld still hold (check:A: saves the second sort of every half-edge on a large patch).
        let mut changed = rep.iter().enumerate().any(|(v, &r)| r as usize != v);
        for (i, &t) in candidates.iter().enumerate() {
            let c = tv[i].map(|v| rep[v as usize]);
            if c[0] == c[1] || c[1] == c[2] || c[0] == c[2] {
                changed = true;
                continue;
            }
            let c = c.map(|v| {
                if new_id[v as usize] == u32::MAX {
                    new_id[v as usize] = new_pos.len() as u32;
                    new_pos.push(pos[v as usize]);
                }
                new_id[v as usize]
            });
            tris.push(t);
            new_tv.push(c);
        }
        let pos = new_pos;
        let tv = new_tv;
        let n = pos.len();
        let geo: Vec<DVec3> = tv
            .iter()
            .map(|c| {
                let [a, b, d] = c.map(|v| pos[v as usize]);
                let g = (b - a).cross(d - a);
                let len = g.length();
                let scale = (b - a).length_squared().max((d - a).length_squared()).max((d - b).length_squared());
                if len > 1e-12 * scale && len.is_finite() { g / len } else { DVec3::ZERO }
            })
            .collect();
        // CSR vertex -> triangles.
        let mut vt_start = vec![0u32; n + 1];
        for c in &tv {
            for &v in c {
                vt_start[v as usize + 1] += 1;
            }
        }
        for i in 0..n {
            vt_start[i + 1] += vt_start[i];
        }
        let mut fill = vt_start.clone();
        let mut vt = vec![0u32; vt_start[n] as usize];
        for (t, c) in tv.iter().enumerate() {
            for &v in c {
                vt[fill[v as usize] as usize] = t as u32;
                fill[v as usize] += 1;
            }
        }
        let edges = if changed { edge_groups(&tv) } else { edges };
        debug_assert!(changed || tv.len() == candidates.len());
        let mut on_boundary = vec![false; n];
        let mut min_edge = vec![f64::INFINITY; n];
        let mut max_edge = vec![0.0f64; n];
        for group in edges.groups() {
            let he = group[0];
            let [a, b] = edge_of(tv[(he / 3) as usize], (he % 3) as usize);
            if group.len() == 1 {
                on_boundary[a as usize] = true;
                on_boundary[b as usize] = true;
            }
            let len = pos[a as usize].distance(pos[b as usize]);
            if len > 0.0 {
                min_edge[a as usize] = min_edge[a as usize].min(len);
                min_edge[b as usize] = min_edge[b as usize].min(len);
                max_edge[a as usize] = max_edge[a as usize].max(len);
                max_edge[b as usize] = max_edge[b as usize].max(len);
            }
        }
        // Parts: union-find over the triangles' vertices.
        let mut uf: Vec<u32> = (0..n as u32).collect();
        fn find(uf: &mut [u32], mut x: u32) -> u32 {
            while uf[x as usize] != x {
                uf[x as usize] = uf[uf[x as usize] as usize];
                x = uf[x as usize];
            }
            x
        }
        for c in &tv {
            let r0 = find(&mut uf, c[0]);
            for &v in &c[1..] {
                let r = find(&mut uf, v);
                if r != r0 {
                    let (lo, hi) = if r < r0 { (r, r0) } else { (r0, r) };
                    uf[hi as usize] = lo;
                }
            }
        }
        let mut part = vec![0u32; n];
        let mut number: PosMap<u32, u32> = PosMap::default();
        let mut part_box: Vec<(DVec3, DVec3)> = Vec::new();
        for v in 0..n as u32 {
            let r = find(&mut uf, v);
            let next = number.len() as u32;
            let id = *number.entry(r).or_insert(next);
            if id as usize == part_box.len() {
                part_box.push((pos[v as usize], pos[v as usize]));
            }
            let b = &mut part_box[id as usize];
            b.0 = b.0.min(pos[v as usize]);
            b.1 = b.1.max(pos[v as usize]);
            part[v as usize] = id;
        }
        let part_count = part_box.len() as u32;
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_unstable_by(|&a, &b| position_order(pos[a as usize], pos[b as usize]).then(a.cmp(&b)));
        let mut rank = vec![0u32; n];
        for (r, &v) in order.iter().enumerate() {
            rank[v as usize] = r as u32;
        }
        Patch {
            tris,
            tv,
            pos,
            geo,
            vt_start,
            vt,
            he_group: edges.he_group,
            edge_tris: edges.sorted,
            part,
            part_count,
            part_box,
            on_boundary,
            min_edge,
            max_edge,
            rank,
            diag,
        }
    }
}

/// The total order of positions (x, then y, then z) that breaks ties between vertices.
fn position_order(a: DVec3, b: DVec3) -> Ordering {
    a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z))
}

/// The welded vertex pair of the edge opposite corner `k` (in the triangle's own order: corner k+1 to k+2).
fn edge_of(c: [u32; 3], k: usize) -> [u32; 2] {
    [c[(k + 1) % 3], c[(k + 2) % 3]]
}

/// Half-edges grouped by their undirected welded edge.
struct EdgeGroups {
    sorted: Vec<u32>,
    he_group: Vec<(u32, u32)>,
}

impl EdgeGroups {
    fn groups(&self) -> impl Iterator<Item = &[u32]> + '_ {
        let mut i = 0usize;
        std::iter::from_fn(move || {
            if i >= self.sorted.len() {
                return None;
            }
            let he = self.sorted[i];
            let (start, len) = self.he_group[he as usize];
            debug_assert_eq!(start as usize, i);
            i += len as usize;
            Some(&self.sorted[start as usize..(start + len) as usize])
        })
    }
}

fn edge_groups(tv: &[[u32; 3]]) -> EdgeGroups {
    let key = |he: u32| -> u64 {
        let [a, b] = edge_of(tv[(he / 3) as usize], (he % 3) as usize);
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        ((lo as u64) << 32) | hi as u64
    };
    let mut sorted: Vec<u32> = (0..tv.len() as u32 * 3).collect();
    sorted.sort_unstable_by_key(|&he| (key(he), he));
    let mut he_group = vec![(0u32, 0u32); sorted.len()];
    let mut i = 0;
    while i < sorted.len() {
        let k = key(sorted[i]);
        let mut j = i + 1;
        while j < sorted.len() && key(sorted[j]) == k {
            j += 1;
        }
        for &he in &sorted[i..j] {
            he_group[he as usize] = (i as u32, (j - i) as u32);
        }
        i = j;
    }
    EdgeGroups { sorted, he_group }
}

// ----- the layout state of one turn -----

/// A candidate position of a vertex.
#[derive(Copy, Clone)]
struct Cand {
    p: DVec2,
    /// Keys of the producing vertices (larger, smaller) and the producing triangle (lower wins ties).
    k0: f64,
    k1: f64,
    tri: u32,
    /// Producing vertex (the one whose finalization made it) and the other final vertex (`u32::MAX` for a fallback).
    from: u32,
    other: u32,
    next: u32,
}

const NONE: u32 = u32::MAX;

#[derive(Copy, Clone, PartialEq, Eq)]
enum State {
    Unknown,
    Tentative,
    Final,
}

/// A heap entry: smallest key first, then the lowest vertex.
#[derive(Copy, Clone)]
struct Entry {
    key: f64,
    /// `Patch::rank` of the vertex: ties go to the lower position, not the lower index, so the order does not
    /// depend on which other triangles the candidate set holds.
    rank: u32,
    v: u32,
}

impl PartialEq for Entry {
    fn eq(&self, o: &Entry) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Entry) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Entry) -> Ordering {
        // Reversed: BinaryHeap is a max-heap.
        o.key.total_cmp(&self.key).then(o.rank.cmp(&self.rank))
    }
}

/// The spine of one turn: the 2D spine direction and its length on either side (for the front's key).
#[derive(Copy, Clone)]
struct SpineKey {
    e: DVec2,
    /// The spine's along range in this turn.
    lo: f64,
    hi: f64,
}

impl SpineKey {
    /// Round 8: the distance of a developed strip position from the spine segment (a seed's exact geodesic `g`).
    fn dist(&self, p: DVec2) -> f64 {
        let along = p.dot(self.e);
        let across = p.dot(rot90(self.e));
        let out = (self.lo - along).max(along - self.hi).max(0.0);
        across.hypot(out)
    }
}

/// A seed vertex: exact coordinates, its frame and oriented normal.
#[derive(Copy, Clone)]
struct Seed {
    v: u32,
    p: DVec2,
    frame: Frame,
    normal: DVec3,
    /// Round 8: the strip triangle that gives the position (its wedge is pinned by the relaxation).
    tri: u32,
}

/// Layout state of one turn.
struct Layout<'a> {
    patch: &'a Patch,
    spine: SpineKey,
    state: Vec<State>,
    p2: Vec<DVec2>,
    /// Round 8: the front's key is the geodesic distance `g` from the spine (fast-marching from the strip seeds, 2.2-1):
    /// tentative while the vertex waits, final when laid.
    key: Vec<f64>,
    /// Round 8: arc length of the vertex's spine point and its side of the spine (+1 / -1, 0 on it), inherited from
    /// the producer (seams S1 / S2).
    foot: Vec<f64>,
    side: Vec<i8>,
    /// Round 8: the strip triangle of each strip seed (`NONE` for others) and the front jumps (source vertex, landing
    /// triangle) in order.
    seed_tri: Vec<u32>,
    jump_log: Vec<(u32, u32)>,
    /// Round 8: the picture rectangle grown by its border (Jump small gaps lands only inside it, 2.2-5).
    footprint: [f64; 2],
    frame: Vec<Frame>,
    normal: Vec<DVec3>,
    seed: Vec<bool>,
    head: Vec<u32>,
    cands: Vec<Cand>,
    /// Winning candidate per vertex (for its frame).
    best: Vec<u32>,
    /// Orientation per local triangle: 0 unknown, +1 / -1 = sign of its 2D area in its own corner order.
    orient: Vec<i8>,
    heap: BinaryHeap<Entry>,
    /// Vertices with fallback candidates only (a triangle with one final corner): laid only when the main front has
    /// nothing left, so a vertex never takes a fallback position while a triangle candidate is still to come.
    fallback: BinaryHeap<Entry>,
    finals: usize,
    /// Parts entered (Jump small gaps) and their count.
    entered: Vec<bool>,
    entered_count: usize,
    jumps: u32,
    reached_edge: bool,
    limited: bool,
}

impl<'a> Layout<'a> {
    fn new(patch: &'a Patch, spine: SpineKey) -> Layout<'a> {
        let n = patch.pos.len();
        Layout {
            patch,
            spine,
            state: vec![State::Unknown; n],
            p2: vec![DVec2::ZERO; n],
            key: vec![f64::INFINITY; n],
            foot: vec![0.0; n],
            side: vec![0; n],
            seed_tri: vec![NONE; n],
            jump_log: Vec::new(),
            footprint: [f64::INFINITY; 2],
            frame: vec![Frame::default(); n],
            normal: vec![DVec3::ZERO; n],
            seed: vec![false; n],
            head: vec![NONE; n],
            cands: Vec::new(),
            best: vec![NONE; n],
            orient: vec![0; patch.tv.len()],
            heap: BinaryHeap::new(),
            fallback: BinaryHeap::new(),
            finals: 0,
            entered: vec![false; patch.part_count as usize],
            entered_count: 0,
            jumps: 0,
            reached_edge: false,
            limited: false,
        }
    }

    /// Marks part `part` as entered.
    fn enter(&mut self, part: u32) {
        if !self.entered[part as usize] {
            self.entered[part as usize] = true;
            self.entered_count += 1;
        }
    }

    fn add_seed(&mut self, s: Seed) {
        let v = s.v as usize;
        if self.seed[v] || self.state[v] == State::Final {
            return;
        }
        self.seed[v] = true;
        self.state[v] = State::Tentative;
        self.p2[v] = s.p;
        self.frame[v] = s.frame;
        self.normal[v] = s.normal;
        self.key[v] = self.spine.dist(s.p);
        self.foot[v] = s.p.dot(self.spine.e).clamp(self.spine.lo, self.spine.hi.max(self.spine.lo));
        self.side[v] = relax::side_of(s.p.dot(rot90(self.spine.e)), self.patch.diag);
        self.seed_tri[v] = s.tri;
        self.heap.push(Entry { key: self.key[v], rank: self.patch.rank[v], v: s.v });
    }

    /// The orientation of local triangle `t`, determined on first use: from an oriented neighbour across an edge
    /// (`first` = the corner pair to look across first), else from `normal` (a vertex's oriented normal).
    fn orientation(&mut self, t: u32, first_edge: usize, normal: DVec3) -> i8 {
        if self.orient[t as usize] != 0 {
            return self.orient[t as usize];
        }
        let c = self.patch.tv[t as usize];
        for k in [first_edge, (first_edge + 1) % 3, (first_edge + 2) % 3] {
            let [a, b] = edge_of(c, k);
            let mut found: Option<(u32, i8)> = None;
            for (u, ku) in self.patch.across(t, k) {
                let o = self.orient[u as usize];
                if o == 0 {
                    continue;
                }
                // Consistently oriented neighbours run the shared edge in opposite directions.
                let [ua, _] = edge_of(self.patch.tv[u as usize], ku);
                let opposite = ua == b;
                let mine = if opposite { o } else { -o };
                if found.is_none_or(|(f, _)| u < f) {
                    found = Some((u, mine));
                }
                debug_assert!(ua == a || ua == b);
            }
            if let Some((_, o)) = found {
                self.orient[t as usize] = o;
                return o;
            }
        }
        let o = if self.patch.geo[t as usize].dot(normal) < 0.0 { -1 } else { 1 };
        self.orient[t as usize] = o;
        o
    }

    /// The position of corner `w` of triangle `t` unfolded onto the final corners `v` (at `pv`) and `a` (at `pa`) by
    /// the similarity of the triangle's own development, on the side of its orientation.
    fn hinge(&mut self, t: u32, v: u32, a: u32, w: u32, pv: DVec2, pa: DVec2) -> Option<DVec2> {
        let patch = self.patch;
        let c = patch.tv[t as usize];
        let (qv, qa, qw) = (patch.pos[v as usize], patch.pos[a as usize], patch.pos[w as usize]);
        let edge = qa - qv;
        let len = edge.length();
        let e2 = pa - pv;
        let len2 = e2.length();
        if !(len > 0.0 && len2 > 0.0) {
            return None;
        }
        let ex = edge / len;
        let d = qw - qv;
        let s = d.dot(ex);
        let h = (d - ex * s).length();
        // Which corner pair to look across first for the orientation: the edge (v, a), opposite corner w.
        let kw = c.iter().position(|&x| x == w).unwrap_or(0);
        let normal = self.normal[v as usize];
        let o = self.orientation(t, kw, normal) as f64;
        // (v, a, w) in the triangle's own cyclic order has the triangle's orientation; otherwise the opposite.
        let kv = c.iter().position(|&x| x == v).unwrap_or(0);
        let even = c[(kv + 1) % 3] == a;
        let sign = if even { o } else { -o };
        // Rigid, centred on the edge: the triangle keeps its own size (a similarity would carry the length error of a
        // developed edge into every triangle unfolded after it, a scale drift on curved surfaces).
        let u = e2 / len2;
        let mid = (pv + pa) * 0.5;
        let p = mid + u * (s - 0.5 * len) + rot90(u) * (sign * h);
        p.is_finite().then_some(p)
    }

    /// Adds a candidate for `w` and updates its tentative position (consensus rule); `g` is the candidate's geodesic
    /// distance (the tentative key is the smallest).
    fn add_cand(&mut self, w: u32, cand: Cand, g: f64) {
        let wi = w as usize;
        if self.seed[wi] || self.state[wi] == State::Final {
            return;
        }
        let mut c = cand;
        c.next = self.head[wi];
        self.head[wi] = self.cands.len() as u32;
        self.cands.push(c);
        // Tentative position: the mean of the triangle candidates when they agree, else the one whose producers have
        // the smallest key; fallback candidates only without triangle candidates.
        let rank = &self.patch.rank;
        let better = |a: &Cand, b: &Cand| {
            (a.k0, a.k1, a.tri, rank[a.from as usize]).partial_cmp(&(b.k0, b.k1, b.tri, rank[b.from as usize])) == Some(Ordering::Less)
        };
        let (mut sum, mut count) = (DVec2::ZERO, 0usize);
        let mut best_tri: Option<u32> = None;
        let mut best_fb: Option<u32> = None;
        let mut i = self.head[wi];
        while i != NONE {
            let c = &self.cands[i as usize];
            if c.other != NONE {
                sum += c.p;
                count += 1;
                if best_tri.is_none_or(|b| better(c, &self.cands[b as usize])) {
                    best_tri = Some(i);
                }
            } else if best_fb.is_none_or(|b| better(c, &self.cands[b as usize])) {
                best_fb = Some(i);
            }
            i = c.next;
        }
        let (p, best) = match best_tri {
            Some(b) => {
                let mean = sum / count as f64;
                let tol = CONSENSUS * self.patch.min_edge[wi];
                let mut agree = true;
                let mut i = self.head[wi];
                while i != NONE {
                    let c = &self.cands[i as usize];
                    if c.other != NONE && c.p.distance(mean) > tol {
                        agree = false;
                        break;
                    }
                    i = c.next;
                }
                // Candidates that disagree only by the surface's curvature (their producers lie consistently in the
                // layout: every pair's layout distance within TEAR_RATIO of its 3D distance) are one front: averaged
                // too. Producers placed far apart are fronts meeting (behind a hole, round a band): the nearest wins.
                if !agree {
                    agree = self.one_front(wi);
                }
                let chosen = if agree { mean } else { self.cands[b as usize].p };
                // Never fold: a position that flips a complete triangle of `w` is replaced by the best candidate that
                // flips none (when there is one).
                if self.flips(wi, chosen) {
                    let mut order: Vec<u32> = Vec::new();
                    let mut i = self.head[wi];
                    while i != NONE {
                        if self.cands[i as usize].other != NONE {
                            order.push(i);
                        }
                        i = self.cands[i as usize].next;
                    }
                    order.sort_by(|&x, &y| {
                        let (a, c) = (&self.cands[x as usize], &self.cands[y as usize]);
                        (a.k0, a.k1, a.tri, rank[a.from as usize])
                            .partial_cmp(&(c.k0, c.k1, c.tri, rank[c.from as usize]))
                            .unwrap_or(Ordering::Equal)
                    });
                    match order.iter().find(|&&x| !self.flips(wi, self.cands[x as usize].p)) {
                        Some(&x) => (self.cands[x as usize].p, x),
                        None => (chosen, b),
                    }
                } else {
                    (chosen, b)
                }
            }
            None => match best_fb {
                Some(b) => (self.cands[b as usize].p, b),
                None => return,
            },
        };
        self.p2[wi] = p;
        self.best[wi] = best;
        self.state[wi] = State::Tentative;
        let key = if g.is_finite() { self.key[wi].min(g) } else { self.key[wi] };
        self.key[wi] = key;
        let entry = Entry { key, rank: self.patch.rank[wi], v: w };
        if self.cands[best as usize].other == NONE {
            self.fallback.push(entry);
        } else {
            self.heap.push(entry);
        }
    }

    /// True when `w` at `p` flips a triangle whose other two corners are final (against its orientation).
    fn flips(&self, wi: usize, p: DVec2) -> bool {
        let patch = self.patch;
        for &t in patch.vtris(wi as u32) {
            let o = self.orient[t as usize];
            if o == 0 || patch.degenerate(t) {
                continue;
            }
            let c = patch.tv[t as usize];
            if c.iter().any(|&x| x as usize != wi && self.state[x as usize] != State::Final) {
                continue;
            }
            let q = c.map(|x| if x as usize == wi { p } else { self.p2[x as usize] });
            if cross2(q[1] - q[0], q[2] - q[0]) * o as f64 <= 0.0 {
                return true;
            }
        }
        false
    }

    /// True when the producers of `w`'s triangle candidates lie consistently in the layout (one front).
    fn one_front(&self, wi: usize) -> bool {
        let mut producers: Vec<u32> = Vec::new();
        let mut i = self.head[wi];
        while i != NONE {
            let c = &self.cands[i as usize];
            if c.other != NONE {
                producers.push(c.from);
                producers.push(c.other);
            }
            i = c.next;
        }
        producers.sort_unstable();
        producers.dedup();
        for (k, &a) in producers.iter().enumerate() {
            for &b in &producers[k + 1..] {
                let l3 = self.patch.pos[a as usize].distance(self.patch.pos[b as usize]);
                let l2 = self.p2[a as usize].distance(self.p2[b as usize]);
                if !(l3 > 0.0 && l2 <= TEAR_RATIO * l3 && l2 * TEAR_RATIO >= l3) {
                    return false;
                }
            }
        }
        true
    }

    /// Runs the front until the key exceeds `stop` (4.3 step 4). `jump` handles Jump small gaps per final vertex.
    fn run(&mut self, stop: f64, rect: [f64; 2], jump: &mut Option<Jumper<'_>>) {
        loop {
            // The main front first; a fallback-only vertex when it has nothing left.
            let entry = match self.heap.pop() {
                Some(e) => e,
                None => match self.fallback.pop() {
                    Some(e) => e,
                    None => break,
                },
            };
            let Entry { key, v, .. } = entry;
            let vi = v as usize;
            if self.state[vi] == State::Final || key.to_bits() != self.key[vi].to_bits() {
                continue;
            }
            // Beyond the reach a vertex is not laid (a nearer candidate may still bring it in); seeds always are (their
            // coordinates are exact, and a large triangle of the strip can have every corner beyond the reach).
            if key > stop && !self.seed[vi] {
                continue;
            }
            if self.finals >= WRAP_MAX_VERTS {
                self.limited = true;
                break;
            }
            self.finalize(v);
            let p = self.p2[vi];
            if self.patch.on_boundary[vi] && p.x.abs() <= rect[0] && p.y.abs() <= rect[1] {
                self.reached_edge = true;
            }
            if let Some(j) = jump.as_mut() {
                j.jump_from_vertex(self, v);
            }
        }
    }

    fn finalize(&mut self, v: u32) {
        let vi = v as usize;
        self.state[vi] = State::Final;
        self.finals += 1;
        let patch = self.patch;
        if !self.seed[vi] {
            let b = self.best[vi];
            let c = self.cands[b as usize];
            if c.other != NONE {
                // Frame and oriented normal from the producing triangle's development.
                let t = c.tri;
                let tc = patch.tv[t as usize];
                let q = tc.map(|x| if x == v { c.p } else { self.p2[x as usize] });
                let frame = Frame::of_triangle(patch.corners(t), q).unwrap_or(self.frame[c.from as usize]);
                self.frame[vi] = frame;
                let o = self.orient[t as usize];
                let n = patch.geo[t as usize] * if o < 0 { -1.0 } else { 1.0 };
                self.normal[vi] = if n == DVec3::ZERO { self.normal[c.from as usize] } else { n };
            } else {
                self.frame[vi] = self.frame[c.from as usize];
                self.normal[vi] = self.normal[c.from as usize];
            }
            // Round 8: the labels of the producer (a producer on the spine gives the vertex's own side).
            let from = c.from as usize;
            self.foot[vi] = self.foot[from];
            self.side[vi] = if self.side[from] != 0 { self.side[from] } else { relax::side_of(self.p2[vi].dot(rot90(self.spine.e)), patch.diag) };
        }
        let pv = self.p2[vi];
        let kv = self.key[vi];
        for &t in patch.vtris(v) {
            if patch.degenerate(t) {
                continue;
            }
            let c = patch.tv[t as usize];
            let kv_pos = c.iter().position(|&x| x == v).unwrap_or(0);
            let (a, w) = (c[(kv_pos + 1) % 3], c[(kv_pos + 2) % 3]);
            let (fa, fw) = (self.state[a as usize] == State::Final, self.state[w as usize] == State::Final);
            match (fa, fw) {
                (true, true) => {
                    // Complete: its orientation (from its neighbours, never from its own possibly flipped layout).
                    let normal = self.normal[vi];
                    self.orientation(t, kv_pos, normal);
                }
                (true, false) | (false, true) => {
                    let (a, w) = if fa { (a, w) } else { (w, a) };
                    if let Some(p) = self.hinge(t, v, a, w, pv, self.p2[a as usize]) {
                        let ka = self.key[a as usize];
                        // Round 8: the geodesic distance of `w` (triangle update, else along the edges).
                        let (qv, qa, qw) = (patch.pos[vi], patch.pos[a as usize], patch.pos[w as usize]);
                        let edge = (kv + qv.distance(qw)).min(ka + qa.distance(qw));
                        let g = relax::fmm_update(qv, qa, qw, kv, ka).map_or(edge, |f| f.min(edge));
                        self.add_cand(w, Cand { p, k0: kv.max(ka), k1: kv.min(ka), tri: t, from: v, other: a, next: NONE }, g);
                    }
                }
                (false, false) => {
                    let frame = self.frame[vi];
                    let qv = patch.pos[vi];
                    for x in [a, w] {
                        let p = pv + frame.apply(patch.pos[x as usize] - qv);
                        if p.is_finite() {
                            let g = kv + qv.distance(patch.pos[x as usize]);
                            self.add_cand(x, Cand { p, k0: kv, k1: kv, tri: t, from: v, other: NONE, next: NONE }, g);
                        }
                    }
                }
            }
        }
    }

    /// Steps 5-7 for this turn: the halo, the tears and the stored triangles (local index, corners), plus the cut
    /// count and the area of the triangles inside the picture (and the turn's along range `clip`, Overlap).
    fn finish(&mut self, hw: f64, hh: f64, clip: Option<(usize, f64, f64)>, period: Option<f64>) -> (SeedTris, u32, f64) {
        let patch = self.patch;
        let mut out = Vec::new();
        let mut cut = 0u32;
        let mut covered = 0.0;
        for t in 0..patch.tv.len() as u32 {
            let c = patch.tv[t as usize];
            let fin = c.map(|x| self.state[x as usize] == State::Final);
            let count = fin.iter().filter(|&&f| f).count();
            if count == 0 {
                continue;
            }
            // The final corner nearest the spine (lowest key, then lowest index).
            let r = (0..3)
                .filter(|&i| fin[i])
                .min_by(|&i, &j| {
                    let rank = &patch.rank;
                    self.key[c[i] as usize].total_cmp(&self.key[c[j] as usize]).then(rank[c[i] as usize].cmp(&rank[c[j] as usize]))
                })
                .unwrap_or(0);
            let mut q = c.map(|x| self.p2[x as usize]);
            if patch.degenerate(t) {
                // Corners from the nearest final corner's frame (no area: nothing to tear).
                let (pr, fr, qr) = (q[r], self.frame[c[r] as usize], patch.pos[c[r] as usize]);
                for i in 0..3 {
                    if !fin[i] {
                        q[i] = pr + fr.apply(patch.pos[c[i] as usize] - qr);
                    }
                }
                if q.iter().all(|p| p.is_finite()) {
                    out.push((t, q, false));
                }
                continue;
            }
            // 5. Halo: the missing corners. Check:B (round 6): a missing corner that the front reached (a tentative
            // vertex: the consensus of its candidates from the laid vertices, step 4) keeps that one position in every
            // halo triangle it belongs to; placed per triangle, the halo triangles around such a vertex left thin
            // wedges without surface between them, inside the picture where the reach ends less than an edge beyond
            // it (the sphere sample: `covered` 0.998-0.999). Only a corner the front never reached is placed by the
            // triangle's own unfolding / frame rule. The tentative positions come from the laid vertices only, so the
            // layout still depends only on the bucketed reach (reuse stays byte-exact).
            let shared = c.map(|x| self.state[x as usize] == State::Tentative && self.p2[x as usize].is_finite());
            if count == 2 {
                let w = (0..3).find(|&i| !fin[i]).unwrap_or(0);
                if !shared[w] {
                    let (i1, i2) = ((w + 1) % 3, (w + 2) % 3);
                    let (v, a) = if self.key[c[i1] as usize] <= self.key[c[i2] as usize] { (i1, i2) } else { (i2, i1) };
                    match self.hinge(t, c[v], c[a], c[w], q[v], q[a]) {
                        Some(p) => q[w] = p,
                        None => continue,
                    }
                }
            } else if count == 1 {
                let (pr, fr, qr) = (q[r], self.frame[c[r] as usize], patch.pos[c[r] as usize]);
                for i in 0..3 {
                    if i != r && !shared[i] {
                        q[i] = pr + fr.apply(patch.pos[c[i] as usize] - qr);
                    }
                }
            }
            // 6. Tears: a flipped or strongly stretched triangle is unfolded rigidly from its corner nearest the spine.
            let normal = self.normal[c[r] as usize];
            let o = self.orientation(t, (r + 1) % 3, normal) as f64;
            let p3 = patch.corners(t);
            let area = cross2(q[1] - q[0], q[2] - q[0]);
            // Flipped, or collapsed (a 2D area below the 3D area over TEAR_RATIO^4: a sliver whose edges may still
            // look fine).
            let area3 = (p3[1] - p3[0]).cross(p3[2] - p3[0]).length();
            let flipped = area * o <= area3 / TEAR_RATIO.powi(4);
            let stretched = (0..3).any(|i| {
                let j = (i + 1) % 3;
                let l3 = p3[i].distance(p3[j]);
                let l2 = q[i].distance(q[j]);
                l3 > 0.0 && !(l2 <= TEAR_RATIO * l3 && l2 * TEAR_RATIO >= l3)
            });
            let torn = flipped || stretched || !q.iter().all(|p| p.is_finite());
            if torn {
                let nt = patch.geo[t as usize] * o;
                let nr = if self.normal[c[r] as usize] == DVec3::ZERO { nt } else { self.normal[c[r] as usize] };
                let turn = DQuat::from_rotation_arc(nt, nr);
                let (pr, fr, qr) = (q[r], self.frame[c[r] as usize], p3[r]);
                for i in 0..3 {
                    if i != r {
                        q[i] = pr + fr.apply(turn * (p3[i] - qr));
                    }
                }
                cut += 1;
                if !q.iter().all(|p| p.is_finite()) {
                    continue;
                }
            }
            covered += layer_area(q, hw, hh, clip, period);
            out.push((t, q, torn));
        }
        (out, cut, covered)
    }
}

/// The seed result of one turn: per triangle its local index, corners and whether it was torn.
type SeedTris = Vec<(u32, [DVec2; 3], bool)>;

/// The area of the layer triangle `q` inside the picture (and the turn's along range; a closed turn counts its copies
/// one period either way).
fn layer_area(q: [DVec2; 3], hw: f64, hh: f64, clip: Option<(usize, f64, f64)>, period: Option<f64>) -> f64 {
    match (clip, period) {
        (Some((axis, _, _)), Some(t)) => [-t, 0.0, t]
            .iter()
            .map(|&shift| {
                let mut moved = q;
                for p in &mut moved {
                    p[axis] += shift;
                }
                clipped_area(moved, hw, hh, clip)
            })
            .sum::<f64>(),
        _ => clipped_area(q, hw, hh, clip),
    }
}

/// Area of the triangle `q` clipped to `|x| <= hw, |y| <= hh` and, with `clip = (axis, lo, hi)`, to
/// `lo <= q[axis] <= hi`.
fn clipped_area(q: [DVec2; 3], hw: f64, hh: f64, clip: Option<(usize, f64, f64)>) -> f64 {
    let mut poly: Vec<DVec2> = q.to_vec();
    let mut planes = vec![(0, 1.0, hw), (0, -1.0, hw), (1, 1.0, hh), (1, -1.0, hh)];
    if let Some((axis, lo, hi)) = clip {
        planes.push((axis, 1.0, hi));
        planes.push((axis, -1.0, -lo));
    }
    for (axis, sign, limit) in planes {
        let inside = |p: DVec2| limit - sign * if axis == 0 { p.x } else { p.y };
        let mut next = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (da, db) = (inside(a), inside(b));
            if da >= 0.0 {
                next.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                next.push(a + (b - a) * (da / (da - db)));
            }
        }
        poly = next;
        if poly.len() < 3 {
            return 0.0;
        }
    }
    let mut area = 0.0;
    for i in 1..poly.len() - 1 {
        area += cross2(poly[i] - poly[0], poly[i + 1] - poly[0]);
    }
    (area * 0.5).abs()
}

// ----- the spine -----

/// One triangle crossed by a spine trace: its development (corners, frame) and oriented normal.
#[derive(Clone)]
struct StripTri {
    tri: u32,
    corners: [DVec2; 3],
    frame: Frame,
    normal: DVec3,
    /// Arc length where the trace entered it, the trace (+1 / -1) and the turn (Overlap).
    s: f64,
    sign: i8,
    turn: i32,
}

/// One trace of the spine.
struct Trace {
    tri: u32,
    q: DVec3,
    d: DVec3,
    nu: DVec3,
    s: f64,
    sign: f64,
    active: bool,
    /// Overlap: the turn this trace is laying (0 until it meets itself).
    turn: i32,
}

/// The anchor of a wrapped stamp: its triangle, point, oriented normal and spine direction (4.3 step 2).
struct Anchor {
    tri: u32,
    point: DVec3,
    /// Spine direction in the anchor triangle's plane and the triangle's oriented normal.
    d: DVec3,
    nu: DVec3,
    /// 2D spine direction (`(1, 0)` or `(0, 1)`).
    e: DVec2,
}

impl Trace {
    /// The development of the current triangle: 2D position of `x` (the trace point maps to `s * e2`).
    fn develop(&self, x: DVec3, e: DVec2) -> DVec2 {
        let e2 = e * self.sign;
        let w = self.nu.cross(self.d);
        e2 * self.s + e2 * (x - self.q).dot(self.d) + rot90(e2) * (x - self.q).dot(w)
    }

    fn frame(&self, e: DVec2) -> Frame {
        let e2 = e * self.sign;
        let p2 = rot90(e2);
        let w = self.nu.cross(self.d);
        Frame { rx: self.d * e2.x + w * p2.x, ry: self.d * e2.y + w * p2.y }
    }
}

/// One turn of a trace: its id (+k for the + trace, -k for the - trace) and its arc length range.
#[derive(Copy, Clone, Debug)]
struct TurnSpan {
    turn: i32,
    start: f64,
    end: f64,
    /// The turn ended by re-entering itself: a full turn round a closed band.
    closed: bool,
}

/// Result of the spine traces.
struct Spine {
    strips: Vec<StripTri>,
    /// Arc length where each trace's turn 0 ended (where the traces met, or their stop).
    end0_plus: f64,
    end0_minus: f64,
    met_itself: bool,
    /// Round 9 (T2): the turn-0 traces passed beside each other (ends meet by passing).
    passed: bool,
    reached_edge: bool,
    limited: bool,
    jumps: u32,
    /// Overlap: the turns after turn 0, in the order they were started.
    turns: Vec<TurnSpan>,
}

/// Where a trace leaves its triangle: the arc length step, the exit edge (opposite corner) and its point.
fn exit_of(c: [DVec2; 3], q: DVec2, e2: DVec2) -> Option<(f64, usize, f64)> {
    let a2 = cross2(c[1] - c[0], c[2] - c[0]);
    if a2 == 0.0 || !a2.is_finite() {
        return None;
    }
    let mut best: Option<(f64, usize)> = None;
    for k in 0..3 {
        let (i, j) = ((k + 1) % 3, (k + 2) % 3);
        let edge = c[j] - c[i];
        let b = (cross2(edge, q - c[i]) / a2).max(0.0);
        let db = cross2(edge, e2) / a2;
        if db < 0.0 {
            let t = b / -db;
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, k));
            }
        }
    }
    let (t, k) = best?;
    let (i, j) = ((k + 1) % 3, (k + 2) % 3);
    let x = q + e2 * t;
    let edge = c[j] - c[i];
    let len2 = edge.length_squared();
    let lambda = if len2 > 0.0 { ((x - c[i]).dot(edge) / len2).clamp(0.0, 1.0) } else { 0.5 };
    Some((t, k, lambda))
}

/// The bit of a turn in a triangle's crossing mask: bit 0 = turn 0, bits 1-7 = turns +1..+7, bits 8-14 = turns
/// -1..-7.
fn turn_bit(turn: i32) -> u16 {
    match turn {
        0 => 1,
        t if t > 0 => 1 << t.min(7),
        t => 1 << (7 + (-t).min(7)),
    }
}

/// The turn rule when trace `tr` (index `pick`) enters a triangle whose crossing mask is `mask`: turn 0 meeting turn 0
/// is the spine meeting itself (Ends meet: the trace stops; Overlap: its turn 1 starts); a later turn re-entering
/// itself starts the next turn. False when the trace stops here.
fn enter_turn(spine: &mut Spine, tr: &mut Trace, pick: usize, mask: u16, end0: &mut [Option<f64>; 2], total_turns: &mut u32, opts: WrapOptions) -> bool {
    if tr.turn == 0 {
        if mask & turn_bit(0) == 0 {
            return true;
        }
        spine.met_itself = true;
        end0[pick] = Some(tr.s);
        if !opts.overlap {
            tr.active = false;
            return false;
        }
    } else if mask & turn_bit(tr.turn) == 0 {
        return true;
    } else if let Some(span) = spine.turns.iter_mut().rev().find(|t| t.turn == tr.turn) {
        span.end = tr.s;
        span.closed = true;
    }
    if *total_turns >= WRAP_MAX_TURNS || tr.turn.abs() >= 7 {
        spine.limited = true;
        tr.active = false;
        return false;
    }
    *total_turns += 1;
    tr.turn += tr.sign as i32;
    spine.turns.push(TurnSpan { turn: tr.turn, start: tr.s, end: tr.s, closed: false });
    true
}

/// Traces the spine from the anchor both ways (4.3 step 3; Overlap 4.3b; the spine's jump 4.3a). The two traces
/// advance alternately (the one with the smaller arc length first, + on ties), so a picture longer than the way round
/// has its ends meet opposite the anchor.
fn trace_spine(patch: &Patch, anchor: &Anchor, reach: f64, across: f64, footprint: f64, opts: WrapOptions, jumper: &mut Option<Jumper<'_>>) -> Spine {
    let ntri = patch.tv.len();
    // The turns that crossed each triangle (`turn_bit`).
    let mut crossed: Vec<u16> = vec![0; ntri];
    let mut spine = Spine {
        strips: Vec::new(),
        end0_plus: 0.0,
        end0_minus: 0.0,
        met_itself: false,
        passed: false,
        reached_edge: false,
        limited: false,
        jumps: 0,
        turns: Vec::new(),
    };
    let start = |sign: f64| Trace { tri: anchor.tri, q: anchor.point, d: anchor.d * sign, nu: anchor.nu, s: 0.0, sign, active: true, turn: 0 };
    let mut traces = [start(1.0), start(-1.0)];
    // Arc length where each trace's turn 0 ended; None while it runs.
    let mut end0: [Option<f64>; 2] = [None, None];
    let e = anchor.e;
    {
        let t = &traces[0];
        let corners = patch.corners(anchor.tri).map(|x| t.develop(x, e));
        spine.strips.push(StripTri { tri: anchor.tri, corners, frame: t.frame(e), normal: anchor.nu, s: 0.0, sign: 1, turn: 0 });
        crossed[anchor.tri as usize] = turn_bit(0);
    }
    if anchor.d == DVec3::ZERO {
        traces[0].active = false;
        traces[1].active = false;
    }
    // 4 x the patch triangles per turn (Overlap lays up to WRAP_MAX_TURNS turns).
    let step_cap = (4 + 2 * WRAP_MAX_TURNS as usize) * ntri + 16;
    let mut steps = 0usize;
    let mut total_turns = 1u32;
    // Parts the spine reached (a spine jump enters a part once).
    let mut spine_parts = vec![false; patch.part_count as usize];
    spine_parts[patch.part[patch.tv[anchor.tri as usize][0] as usize] as usize] = true;
    // Round 9 (T2): at the last check, the turn-0 heads' distance, the - head's offset ahead of the + head along the
    // + trace's direction, and the traces' arc lengths; whether the heads approach each other within the across reach.
    let mut pass_prev: Option<(f64, f64, [f64; 2])> = None;
    let mut pass_armed = false;
    loop {
        // Round 9 (T2): ends meet by passing. A spine that is not exactly round a tube is a helix: its traces pass
        // beside each other without sharing a triangle. Once they have gone the across reach together, with their oriented
        // normals agreeing, heads that approach each other (their distance falling, below the across reach: armed) and
        // whose offset along the + trace's direction then changes sign have passed: the ends meet where the offset is
        // zero (interpolated between the two checks). Both traces stop (Ends meet) or start their next turn (Overlap,
        // as `enter_turn` does for a crossing).
        if !spine.passed
            && traces[0].active
            && traces[1].active
            && traces[0].turn == 0
            && traces[1].turn == 0
            && traces[0].s + traces[1].s > across
            && traces[0].nu.dot(traces[1].nu) > 0.0
        {
            let u = traces[1].q - traces[0].q;
            let (d, ahead) = (u.length(), u.dot(traces[0].d));
            let now = [traces[0].s, traces[1].s];
            if let Some((d0, a0, s0)) = pass_prev {
                if d < across && d < d0 {
                    pass_armed = true;
                }
                if pass_armed && a0 > 0.0 && ahead <= 0.0 && d < across {
                    let f = (a0 / (a0 - ahead)).clamp(0.0, 1.0);
                    let at = [s0[0] + f * (now[0] - s0[0]), s0[1] + f * (now[1] - s0[1])];
                    spine.passed = true;
                    end0 = [Some(at[0]), Some(at[1])];
                    for (k, tr) in traces.iter_mut().enumerate() {
                        if !opts.overlap {
                            tr.active = false;
                        } else if total_turns >= WRAP_MAX_TURNS {
                            spine.limited = true;
                            tr.active = false;
                        } else {
                            total_turns += 1;
                            tr.turn += tr.sign as i32;
                            spine.turns.push(TurnSpan { turn: tr.turn, start: at[k], end: tr.s, closed: false });
                        }
                    }
                    continue;
                }
            }
            pass_prev = Some((d, ahead, now));
        }
        let pick = match (traces[0].active, traces[1].active) {
            (true, true) => usize::from(traces[1].s < traces[0].s),
            (true, false) => 0,
            (false, true) => 1,
            (false, false) => break,
        };
        steps += 1;
        if steps > step_cap {
            spine.limited = true;
            break;
        }
        let tr = &mut traces[pick];
        let e2 = e * tr.sign;
        let c = patch.corners(tr.tri).map(|x| tr.develop(x, e));
        let Some((dt, k, lambda)) = exit_of(c, e2 * tr.s, e2) else {
            tr.active = false;
            continue;
        };
        // Jump small gaps: another part within the gap of the path ahead (sampled every half gap) takes the spine.
        let mut gap_jump = None;
        if let Some(j) = jumper.as_mut() {
            let own = patch.part[patch.tv[tr.tri as usize][0] as usize];
            spine_parts[own as usize] = true;
            // Round 8 (2.2-5): the spine jumps only within the picture's half length and border.
            if j.near(own) && tr.s < footprint {
                let end = dt.min(reach - tr.s).min(footprint - tr.s).max(0.0);
                let step = 0.5 * j.gap;
                let mut t = step.min(end);
                loop {
                    let e = tr.q + tr.d * t;
                    if let Some(found) = j.spine_jump(e, tr.d, tr.nu, tr.tri, &|part| !spine_parts[part as usize]) {
                        gap_jump = Some((t, found));
                        break;
                    }
                    if t >= end {
                        break;
                    }
                    t = (t + step).min(end);
                }
            }
        }
        if let Some((t, (n, q, d, nu, gap))) = gap_jump {
            spine.jumps += 1;
            tr.s += t + gap;
            if tr.s >= reach {
                tr.s = reach;
                tr.active = false;
                continue;
            }
            if !enter_turn(&mut spine, tr, pick, crossed[n as usize], &mut end0, &mut total_turns, opts) {
                continue;
            }
            tr.tri = n;
            tr.q = q;
            tr.d = d;
            tr.nu = nu;
            crossed[n as usize] |= turn_bit(tr.turn);
            spine_parts[patch.part[patch.tv[n as usize][0] as usize] as usize] = true;
            let t = &traces[pick];
            let corners = patch.corners(t.tri).map(|x| t.develop(x, e));
            spine.strips.push(StripTri { tri: t.tri, corners, frame: t.frame(e), normal: t.nu, s: t.s, sign: t.sign as i8, turn: t.turn });
            continue;
        }
        if tr.s + dt >= reach {
            tr.s = reach;
            tr.active = false;
            continue;
        }
        tr.s += dt;
        let [vi, vj] = edge_of(patch.tv[tr.tri as usize], k);
        let lambda = lambda.clamp(SPINE_NUDGE, 1.0 - SPINE_NUDGE);
        let (pi, pj) = (patch.pos[vi as usize], patch.pos[vj as usize]);
        let exit = pi + (pj - pi) * lambda;
        let u = (pj - pi).normalize_or_zero();
        let w_out = (tr.d - u * tr.d.dot(u)).normalize_or_zero();
        // The neighbour: the most nearly coplanar continuation (lowest index on ties).
        let mut next: Option<(f64, u32, DVec3)> = None;
        for (n, kn) in patch.across(tr.tri, k) {
            if patch.degenerate(n) {
                continue;
            }
            let into = patch.pos[patch.tv[n as usize][kn] as usize] - pi;
            let w_in = (into - u * into.dot(u)).normalize_or_zero();
            if w_in == DVec3::ZERO {
                continue;
            }
            let score = w_in.dot(w_out);
            if next.is_none_or(|(s, m, _)| score > s || (score == s && n < m)) {
                next = Some((score, n, w_in));
            }
        }
        // The next triangle, point, direction and oriented normal: across the edge, or across a small gap.
        let (n, q, d, nu) = match next {
            Some((_, n, w_in)) => {
                // The direction keeps its angle to the edge; the oriented normal follows the hinge.
                let d = (u * tr.d.dot(u) + w_in * tr.d.dot(w_out)).normalize_or_zero();
                let s_or = if tr.nu.dot(u.cross(w_out)) < 0.0 { -1.0 } else { 1.0 };
                (n, exit, d, (u.cross(w_in) * s_or).normalize_or_zero())
            }
            None => match jumper.as_mut().filter(|_| tr.s <= footprint).and_then(|j| j.spine_jump(exit, tr.d, tr.nu, tr.tri, &|part| !spine_parts[part as usize])) {
                Some((n, q, d, nu, gap)) => {
                    spine.jumps += 1;
                    tr.s += gap;
                    if tr.s >= reach {
                        tr.s = reach;
                        tr.active = false;
                        continue;
                    }
                    (n, q, d, nu)
                }
                None => {
                    spine.reached_edge = true;
                    tr.active = false;
                    continue;
                }
            },
        };
        if d == DVec3::ZERO || nu == DVec3::ZERO || !(d.is_finite() && nu.is_finite()) {
            tr.active = false;
            continue;
        }
        if !enter_turn(&mut spine, tr, pick, crossed[n as usize], &mut end0, &mut total_turns, opts) {
            continue;
        }
        tr.tri = n;
        tr.q = q;
        tr.d = d;
        tr.nu = nu;
        crossed[n as usize] |= turn_bit(tr.turn);
        spine_parts[patch.part[patch.tv[n as usize][0] as usize] as usize] = true;
        let t = &traces[pick];
        let corners = patch.corners(t.tri).map(|x| t.develop(x, e));
        spine.strips.push(StripTri { tri: t.tri, corners, frame: t.frame(e), normal: t.nu, s: t.s, sign: t.sign as i8, turn: t.turn });
    }
    spine.end0_plus = end0[0].unwrap_or(traces[0].s);
    spine.end0_minus = end0[1].unwrap_or(traces[1].s);
    // The end of each later turn: the start of the trace's next turn, or where the trace stopped.
    for span in &mut spine.turns {
        if !span.closed {
            span.end = if span.turn > 0 { traces[0].s } else { traces[1].s };
        }
    }
    spine
}

// ----- Jump small gaps (4.3a) -----

/// The gap jumps of one layout: candidates of other parts within `GAP_JUMP` of a vertex.
pub(crate) struct Jumper<'a> {
    mesh: &'a MeshData,
    bvh: &'a Bvh,
    patch: &'a Patch,
    /// Local triangle of each mesh triangle of the patch.
    local_of: PosMap<u32, u32>,
    gap: f64,
    /// Per part: true when another part lies within `gap` of its box.
    near_other: Vec<bool>,
    /// Per part: the other parts whose box lies within the gap of its box.
    neighbours: Vec<Vec<u32>>,
    scratch: Vec<(TriId, f32)>,
}

impl<'a> Jumper<'a> {
    fn new(mesh: &'a MeshData, bvh: &'a Bvh, patch: &'a Patch) -> Option<Jumper<'a>> {
        if patch.part_count < 2 {
            return None;
        }
        let gap = GAP_JUMP * patch.diag;
        let boxes = &patch.part_box;
        // A sweep along x over the part boxes: a part is "near another" when some other box lies within the gap.
        let mut near_other = vec![false; boxes.len()];
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); boxes.len()];
        let mut by_x: Vec<usize> = (0..boxes.len()).collect();
        by_x.sort_unstable_by(|&a, &b| boxes[a].0.x.total_cmp(&boxes[b].0.x).then(a.cmp(&b)));
        for (k, &i) in by_x.iter().enumerate() {
            for &j in &by_x[k + 1..] {
                if boxes[j].0.x > boxes[i].1.x + gap {
                    break;
                }
                let (a, b) = (boxes[i], boxes[j]);
                if (b.0 - a.1).max(a.0 - b.1).max(DVec3::ZERO).length() <= gap {
                    near_other[i] = true;
                    near_other[j] = true;
                    neighbours[i].push(j as u32);
                    neighbours[j].push(i as u32);
                }
            }
        }
        let local_of = patch.tris.iter().enumerate().map(|(i, &t)| (t, i as u32)).collect();
        Some(Jumper { mesh, bvh, patch, local_of, gap, near_other, neighbours, scratch: Vec::new() })
    }

    /// The landing of a jump from `p` (on part `own`, oriented normal `normal`): the nearest point within the gap on
    /// a triangle of a part other than `own` that `accept` accepts, as (local triangle, point, distance). Among the
    /// nearest (within a thousandth of the gap) the landing prefers a triangle that continues the surface (facing
    /// the same way), then one rising from it (a wall), then one turning away below it, then the back side of
    /// something lying on it; then the lower triangle. With `ahead` (the spine's direction) a landing behind `p` is
    /// not taken.
    fn nearest_other(
        &mut self,
        p: DVec3,
        own: u32,
        accept: &dyn Fn(u32) -> bool,
        normal: DVec3,
        ahead: Option<DVec3>,
    ) -> Option<(u32, DVec3, f64)> {
        let center = Vec3::new(p.x as f32, p.y as f32, p.z as f32);
        self.bvh.query_sphere_dist(self.mesh, center, self.gap as f32, &mut self.scratch);
        let mut found: Vec<(f64, u32, DVec3)> = Vec::new();
        for &(tri, _) in &self.scratch {
            let Some(&t) = self.local_of.get(&tri) else { continue };
            if self.patch.degenerate(t) {
                continue;
            }
            let part = self.patch.part[self.patch.tv[t as usize][0] as usize];
            if part == own || !accept(part) {
                continue;
            }
            let [a, b, c] = self.patch.corners(t);
            let to32 = |v: DVec3| Vec3::new(v.x as f32, v.y as f32, v.z as f32);
            let q = dvec(closest_point_on_triangle(center, to32(a), to32(b), to32(c)));
            let d = q.distance(p);
            if d > self.gap {
                continue;
            }
            if let Some(dir) = ahead {
                let off = q - p;
                if (off - normal * off.dot(normal)).dot(dir) < -1e-3 * self.gap {
                    continue;
                }
            }
            found.push((d, t, q));
        }
        let dmin = found.iter().map(|f| f.0).fold(f64::INFINITY, f64::min);
        let tol = 1e-3 * self.gap;
        let mut best: Option<(u8, f64, u32, DVec3)> = None;
        for &(d, t, q) in found.iter().filter(|f| f.0 <= dmin + tol) {
            let g = self.patch.geo[t as usize];
            let corners = self.patch.corners(t);
            let facing = g.dot(normal);
            let size = (0..3).map(|i| corners[i].distance(corners[(i + 1) % 3])).fold(0.0, f64::max);
            let rise = corners.iter().map(|c| (*c - q).dot(normal)).fold(f64::NEG_INFINITY, f64::max);
            let class = if facing.abs() > 0.9 {
                if facing > 0.0 { 0 } else { 3 }
            } else if rise > 1e-3 * size {
                1
            } else {
                2
            };
            if best.is_none_or(|(bc, bd, bt, _)| (class, d, t) < (bc, bd, bt)) {
                best = Some((class, d, t, q));
            }
        }
        best.map(|(_, d, t, q)| (t, q, d))
    }

    /// A front jump from final vertex `v` of `layout` (4.3a "Where a jump starts / lands").
    fn jump_from_vertex(&mut self, layout: &mut Layout<'_>, v: u32) {
        let vi = v as usize;
        let own = self.patch.part[vi];
        if !self.near_other[own as usize] {
            return;
        }
        layout.enter(own);
        if layout.entered_count == layout.entered.len() {
            return;
        }
        let p = self.patch.pos[vi];
        // Only a vertex within the gap of the box of a neighbouring part not entered yet asks the BVH.
        let gap = self.gap;
        let boxes = &self.patch.part_box;
        let near = self.neighbours[own as usize].iter().any(|&o| {
            !layout.entered[o as usize] && {
                let (lo, hi) = boxes[o as usize];
                (lo - p).max(p - hi).max(DVec3::ZERO).length() <= gap
            }
        });
        if !near {
            return;
        }
        // Round 8 (2.2-5): a jump starts only where the picture's own footprint lies.
        let fp = layout.footprint;
        let pv0 = layout.p2[vi];
        if !(pv0.x.abs() <= fp[0] && pv0.y.abs() <= fp[1]) {
            return;
        }
        let found = {
            let entered = &layout.entered;
            self.nearest_other(p, own, &|part| !entered[part as usize], layout.normal[vi], None)
        };
        let Some((t, q, gap)) = found else { return };
        let part = self.patch.part[self.patch.tv[t as usize][0] as usize];
        let (pv, fv, nv) = (layout.p2[vi], layout.frame[vi], layout.normal[vi]);
        let pq = pv + fv.apply(q - p);
        // Round 8: and lands only inside it.
        if !(pq.x.abs() <= fp[0] && pq.y.abs() <= fp[1]) {
            return;
        }
        layout.enter(part);
        layout.jumps += 1;
        layout.jump_log.push((v, t));
        // The landing triangle, developed rigidly with q at pq, its plane turned onto v's tangent plane.
        let g = self.patch.geo[t as usize];
        let nt = if g.dot(nv) < 0.0 { -g } else { g };
        let turn = DQuat::from_rotation_arc(nt, if nv == DVec3::ZERO { nt } else { nv });
        let o: i8 = if g.dot(nt) < 0.0 { -1 } else { 1 };
        if layout.orient[t as usize] == 0 {
            layout.orient[t as usize] = o;
        }
        let key = layout.key[vi] + gap;
        for &x in &self.patch.tv[t as usize] {
            let xp = pq + fv.apply(turn * (self.patch.pos[x as usize] - q));
            if !xp.is_finite() {
                continue;
            }
            let xi = x as usize;
            if layout.seed[xi] || layout.state[xi] == State::Final {
                continue;
            }
            layout.seed[xi] = true;
            layout.state[xi] = State::Tentative;
            layout.p2[xi] = xp;
            layout.frame[xi] = Frame { rx: turn.inverse() * fv.rx, ry: turn.inverse() * fv.ry };
            layout.normal[xi] = nt;
            // Seeds of a jump keep the order of the jump: the depth of the vertex it started from plus the gap (round 8:
            // plus the corner's distance from the landing point), and its labels.
            layout.key[xi] = key + self.patch.pos[xi].distance(q);
            layout.foot[xi] = layout.foot[vi];
            layout.side[xi] = layout.side[vi];
            layout.heap.push(Entry { key: layout.key[xi], rank: self.patch.rank[xi], v: x });
        }
    }

    /// True when part `part` has another part within the gap of its box.
    fn near(&self, part: u32) -> bool {
        self.near_other[part as usize]
    }

    /// The spine's jump from `e` on triangle `tri` (direction `d`, oriented normal `nu`) to a part `accept`
    /// accepts: (local triangle, landing point, direction and oriented normal there, the gap's length along `d`).
    fn spine_jump(&mut self, e: DVec3, d: DVec3, nu: DVec3, tri: u32, accept: &dyn Fn(u32) -> bool) -> Option<(u32, DVec3, DVec3, DVec3, f64)> {
        let own = self.patch.part[self.patch.tv[tri as usize][0] as usize];
        let (t, q, _) = self.nearest_other(e, own, accept, nu, Some(d))?;
        let g = self.patch.geo[t as usize];
        let nt = if g.dot(nu) < 0.0 { -g } else { g };
        let turn = DQuat::from_rotation_arc(nu, nt);
        let d2 = turn * d;
        let d2 = (d2 - nt * d2.dot(nt)).normalize_or_zero();
        if d2 == DVec3::ZERO {
            return None;
        }
        let gap = q - e;
        let tangential = (gap - nu * gap.dot(nu)).dot(d).max(0.0);
        Some((t, q, d2, nt, tangential))
    }
}

/// Round 8 (request 2 of package B, applied by check:B): the number of parameterizations this thread has started
/// (`wrap_param`), for the app tests of plan 4.2 ("no parameterization built on the UI thread"). One thread-local add
/// per call, as `doc_stamps::raster_counter`.
pub mod param_counter {
    use std::cell::Cell;

    thread_local! {
        static COUNT: Cell<u64> = const { Cell::new(0) };
    }

    pub(crate) fn bump() {
        COUNT.with(|c| c.set(c.get() + 1));
    }

    /// Parameterizations started by this thread so far.
    pub fn get() -> u64 {
        COUNT.with(Cell::get)
    }
}

// ----- the public entry -----

/// The parameterization of the surface around the wrapped stamp `p` (4.3). `None` when the projector is unusable
/// (non-finite, zero size, empty source) or no triangle lies within reach of `origin`.
pub fn wrap_param(
    mesh: &MeshData,
    bvh: &Bvh,
    maps: &[std::sync::Arc<TexelMap>],
    src_size: [u32; 2],
    p: &Projector,
) -> Option<WrapParam> {
    param_counter::bump();
    let started = Instant::now();
    let diag = mesh.diag() as f64;
    let texel_world = max_texel_world(maps);
    let reach = reach_of(p, src_size, texel_world, diag)?;
    let opts = p.wrap;
    let [along_r, across_r] = reach.reach;
    // Round 8: the front lays the surface within `WRAP_STOP_SLACK x across_r` of the spine (Euclidean <= geodesic).
    let slack = relax::knob("WTE_TEST_STOP_SLACK", WRAP_STOP_SLACK);
    let mut radius = along_r + slack * across_r;
    if opts.jump_gaps {
        radius += 4.0 * GAP_JUMP * diag;
    }
    let mut found: Vec<(TriId, f32)> = Vec::new();
    bvh.query_sphere_dist(mesh, p.origin, radius as f32, &mut found);
    if found.is_empty() {
        return None;
    }
    // The reach is measured from the anchor (the surface point nearest `origin`), not from `origin`: an origin off the
    // surface (beside an open edge, an off-centre text anchor moved in the tangent plane of a curved surface) is
    // `dmin` away from it, so the sphere grows by that much (check:A: a picture anchored beside a plate's edge lost
    // the far triangles of its own half).
    let off = found.iter().map(|e| e.1).fold(f32::INFINITY, f32::min).max(0.0).sqrt() as f64;
    if off > 1e-6 * diag {
        found.clear();
        bvh.query_sphere_dist(mesh, p.origin, (radius + off) as f32, &mut found);
    }
    let mut limited = false;
    if found.len() > WRAP_MAX_TRIS {
        found.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        found.truncate(WRAP_MAX_TRIS);
        found.sort_unstable_by_key(|e| e.0);
        limited = true;
    }
    // Anchor triangle: the nearest candidates, the one facing the projector most, then the lowest id.
    let (right, _, out) = p.axes();
    let (right, out) = (dvec(right), dvec(out));
    let dmin = found.iter().map(|e| e.1).fold(f32::INFINITY, f32::min).max(0.0).sqrt() as f64;
    let tie = dmin + ANCHOR_TIE * diag;
    let candidates: Vec<TriId> = found.iter().map(|e| e.0).collect();
    let near: Vec<TriId> = found.iter().filter(|e| (e.1.max(0.0) as f64).sqrt() <= tie).map(|e| e.0).collect();
    drop(found);
    let patch = Patch::build(mesh, candidates);
    if patch.tv.is_empty() {
        return None;
    }
    let local: PosMap<u32, u32> = patch.tris.iter().enumerate().map(|(i, &t)| (t, i as u32)).collect();
    let mut anchor_tri: Option<(f64, u32)> = None;
    for &t in &near {
        let Some(&l) = local.get(&t) else { continue };
        if patch.degenerate(l) {
            continue;
        }
        let facing = patch.geo[l as usize].dot(out);
        if anchor_tri.is_none_or(|(f, b)| facing > f || (facing == f && l < b)) {
            anchor_tri = Some((facing, l));
        }
    }
    let anchor_l = match anchor_tri {
        Some((_, l)) => l,
        None => *near.iter().filter_map(|t| local.get(t)).min()?,
    };
    drop(local);
    let tri = patch.tris[anchor_l as usize];
    let [a, b, c] = mesh.tri_positions(tri);
    let point = closest_point_on_triangle(p.origin, a, b, c);
    let bary = barycentric(dvec(point), dvec(a), dvec(b), dvec(c));
    let ns = mesh.tri_normals(tri);
    let shading = dvec(ns[0]) * bary[0] + dvec(ns[1]) * bary[1] + dvec(ns[2]) * bary[2];
    let g = patch.geo[anchor_l as usize];
    let mut n = shading.normalize_or_zero();
    if n == DVec3::ZERO || !n.is_finite() {
        n = if g == DVec3::ZERO { out } else { g };
    }
    if n.dot(out) < 0.0 {
        n = -n;
    }
    // The frame turned by the shortest arc from `out` onto the anchor triangle's plane (its normal oriented like the
    // shading normal): on a flat face with smoothed normals the picture keeps its direction exactly.
    let nu = if g == DVec3::ZERO { n } else if g.dot(n) < 0.0 { -g } else { g };
    let turn = DQuat::from_rotation_arc(out, nu);
    let right_n = turn * right;
    let up_n = nu.cross(right_n);
    let (dir3, e) = if reach.spine_y { (up_n, DVec2::Y) } else { (right_n, DVec2::X) };
    let d = (dir3 - nu * dir3.dot(nu)).normalize_or_zero();
    let anchor = Anchor { tri: anchor_l, point: dvec(point), d, nu, e };

    let mut jumper = if opts.jump_gaps { Jumper::new(mesh, bvh, &patch) } else { None };
    // Round 8: every layout decision that a footprint bounds uses the bucketed reach (the picture's half extents with
    // its border and texel margin, rounded up), so a parameterization reused for another size in the same bucket stays
    // bit-identical to a fresh one (4.4): the spine's jumps (its trace stops at the reach anyway), the front's jumps,
    // the early-out region and the owner grid.
    let spine = trace_spine(&patch, &anchor, along_r, across_r, along_r, opts, &mut jumper);
    drop(jumper);
    let (hw, hh) = (reach.hw, reach.hh);
    let rect = [hw, hh];
    let footprint = if reach.spine_y { [across_r, along_r] } else { [along_r, across_r] };
    let picture = [hw + 2.0 * hw / src_size[0] as f64, hh + 2.0 * hh / src_size[1] as f64];
    let stop = slack * across_r;
    let mut stretches: Vec<f64> = Vec::new();
    let mut rejected = (0usize, 0usize);
    let mut lost_cut = (0usize, 0usize);
    let mut stats = WrapStats {
        verts: 0,
        tris: 0,
        cut_tris: 0,
        covered: 0.0,
        met_itself: spine.met_itself,
        reached_edge: spine.reached_edge,
        limited: limited || spine.limited,
        jumps: spine.jumps,
        turns: 1,
        micros: 0,
        relaxed: false,
        seams: 0,
        seams_by_rule: [0; 5],
        flipped: 0,
        stretch: 0.0,
        rejected: 0.0,
        cg_iters: 0,
        passed: spine.passed,
        ends_share: 1.0,
        lost_cut: 0.0,
    };
    // The turns to lay: turn 0 (the Ends-meet spine), then with Overlap every later turn that starts inside the
    // picture, each with its along range (4.3b).
    // A closed turn (turn 0 when the spine met itself, a later turn that re-entered itself) covers its band once: its
    // along range is one period, and a fragment outside the range is that period away from its place in the turn.
    // Round 9 (T2): a spine whose traces passed beside each other is closed too (a helix: its feet differ by the
    // period across the line where the ends meet, S1).
    let period0 = (spine.met_itself || spine.passed).then_some(spine.end0_plus + spine.end0_minus);
    let mut turns: Vec<(i32, f64, f64, Option<f64>)> = vec![(0, -spine.end0_minus, spine.end0_plus, period0)];
    let overlap_turns = opts.overlap && (spine.met_itself || spine.passed);
    if overlap_turns {
        for span in &spine.turns {
            if span.start < reach.needed_along {
                let (lo, hi) = if span.turn > 0 { (span.start, span.end) } else { (-span.end, -span.start) };
                turns.push((span.turn, lo, hi, span.closed.then_some(span.end - span.start)));
            }
        }
    }
    let multi = turns.len() > 1;
    let axis = usize::from(reach.spine_y);
    let mut layers: Vec<(i32, WrapLayer)> = Vec::new();
    let mut covered = 0.0;
    for &(turn_id, lo, hi, period) in &turns {
        let mut strips: Vec<&StripTri> = spine.strips.iter().filter(|s| s.turn == turn_id).collect();
        if strips.is_empty() {
            continue;
        }
        // Seeds in trace order (arc length; + before - on ties; then the order traced).
        strips.sort_by(|a, b| a.s.total_cmp(&b.s).then(b.sign.cmp(&a.sign)));
        let mut layout = Layout::new(&patch, SpineKey { e, lo, hi });
        for st in &strips {
            if layout.orient[st.tri as usize] == 0 {
                let q = st.corners;
                let area = cross2(q[1] - q[0], q[2] - q[0]);
                layout.orient[st.tri as usize] = if area < 0.0 { -1 } else { 1 };
            }
            for (i, &v) in patch.tv[st.tri as usize].iter().enumerate() {
                layout.add_seed(Seed { v, p: st.corners[i], frame: st.frame, normal: st.normal, tri: st.tri });
            }
            // A part the spine reached (also across a gap) is laid from its strip, never again by a front jump.
            layout.enter(patch.part[patch.tv[st.tri as usize][0] as usize]);
        }
        let mut turn_jumper = if opts.jump_gaps { Jumper::new(mesh, bvh, &patch) } else { None };
        layout.footprint = footprint;
        layout.run(stop, rect, &mut turn_jumper);
        let clip = multi.then_some((axis, lo, hi));
        let closed = period.filter(|t| *t > 0.0);
        let period = if multi { closed } else { None };
        let (seed, cut, seed_area) = layout.finish(hw, hh, clip, period);
        stats.verts += layout.finals as u32;
        stats.reached_edge |= layout.reached_edge;
        stats.limited |= layout.limited;
        stats.jumps += layout.jumps;
        // Round 8 (3.2 steps 4-6): the seed is kept where it is exact (developable surfaces, closures); else the layout
        // is cut by the seam rules and relaxed. Overlap turns of a several-turn stack keep the seed (round 6): their
        // ends are not closures the seam rules know (a turn's range is shorter than the band), and WA18 needs them exact.
        let (tris, area) = if multi || layout.seed_is_exact(&seed, footprint, closed) {
            stats.cut_tris += cut;
            (seed.iter().map(|(t, q, _)| (*t, *q, layout.corner_g(*t))).collect::<Vec<_>>(), seed_area)
        } else {
            let r = layout.relax(&seed, picture, closed, anchor_l);
            if relax::knob("WTE_TEST_RELAX_DEBUG", false) {
                let laid = layout.laid();
                let (mut inside_laid, mut inside_halo) = (0, 0);
                for (t, q, _) in &r.tris {
                    if relax::meets(q, rect) {
                        if laid[*t as usize] { inside_laid += 1 } else { inside_halo += 1 }
                    }
                }
                eprintln!("relax: turn {turn_id}: {} stored, inside the picture {inside_laid} laid, {inside_halo} halo; flipped {}", r.tris.len(), r.flipped);
                for (t, q, _) in &r.tris {
                    let p3 = patch.corners(*t);
                    let a3 = 0.5 * (p3[1] - p3[0]).cross(p3[2] - p3[0]).length();
                    let a2 = 0.5 * cross2(q[1] - q[0], q[2] - q[0]).abs();
                    if relax::meets(q, picture) && !patch.degenerate(*t) && (a2 / a3 - 1.0).abs() > 0.5 {
                        eprintln!("relax:   tri {} laid {} area 2D {a2:.3e} 3D {a3:.3e} corners {q:?}", patch.tris[*t as usize], laid[*t as usize]);
                    }
                }
            }
            stats.relaxed = true;
            stats.seams += r.seams;
            for (a, b) in stats.seams_by_rule.iter_mut().zip(r.seams_by_rule) {
                *a += b;
            }
            stats.cut_tris += r.cut_tris;
            stats.flipped += r.flipped;
            stats.cg_iters += r.cg_iters;
            let area = r.tris.iter().map(|(_, q, _)| layer_area(*q, hw, hh, clip, period)).sum::<f64>();
            (r.tris, area)
        };
        covered += area;
        for (t, q, _) in &tris {
            if patch.degenerate(*t) || !relax::meets(q, picture) {
                continue;
            }
            let p3 = patch.corners(*t);
            let a3 = 0.5 * (p3[1] - p3[0]).cross(p3[2] - p3[0]).length();
            let a2 = 0.5 * cross2(q[1] - q[0], q[2] - q[0]).abs();
            // check:A: slivers and near-zero-area triangles (left out of the energies, 3.1) are left out here too: their
            // area ratio says nothing (check_a8's slivered plate reported a stretch of 2.7e6 %).
            if a3 >= 1e-12 * patch.diag * patch.diag && !relax::sliver(p3) && (a2 / a3).is_finite() {
                stretches.push((a2 / a3 - 1.0).abs());
            }
        }
        let along_clip = clip.map(|(_, lo, hi)| [lo as f32, hi as f32]);
        let laid = layout.laid();
        let owner_ctx = OwnerCtx { mesh, bvh, patch: &patch, orient: &layout.orient, entered: &layout.entered, laid: &laid };
        let (layer, (rej, lost)) = make_layer(&owner_ctx, tris, along_clip, period.map(|t| t as f32), axis, footprint, picture);
        rejected.0 += rej.0;
        rejected.1 += rej.1;
        lost_cut.0 += lost.0;
        lost_cut.1 += lost.1;
        layers.push((turn_id, layer));
    }
    if layers.is_empty() {
        return None;
    }
    // Bottom first: the layer later along the picture's spine axis lies on top, like tape wound from the picture's
    // start (its end on top); a flip on that axis reverses the order.
    let flip = if reach.spine_y { p.flip_y } else { p.flip_x };
    let order = |turn: i32| if flip { -turn } else { turn };
    layers.sort_by_key(|l| order(l.0));
    stats.turns = layers.len() as u32;
    // Round 9 (T2): the share of the picture's length the spine lays (Ends meet).
    if (spine.met_itself || spine.passed) && stats.turns == 1 {
        let share = (spine.end0_plus + spine.end0_minus) / (2.0 * reach.needed_along);
        stats.ends_share = if share.is_finite() { share.clamp(0.0, 1.0) as f32 } else { 1.0 };
    }
    stats.tris = layers.iter().map(|l| l.1.tris.len() as u32).sum();
    stats.covered = (covered / (4.0 * hw * hh)).clamp(0.0, 1.0) as f32;
    stretches.sort_by(f64::total_cmp);
    stats.stretch = stretches.get(((stretches.len() as f64 * 0.95) as usize).min(stretches.len().saturating_sub(1))).copied().unwrap_or(0.0) as f32;
    stats.rejected = if rejected.1 == 0 { 0.0 } else { (rejected.0 as f64 / rejected.1 as f64) as f32 };
    stats.lost_cut = if lost_cut.1 == 0 { 0.0 } else { (lost_cut.0 as f64 / lost_cut.1 as f64) as f32 };
    stats.micros = started.elapsed().as_micros() as u64;
    Some(WrapParam {
        mesh_addr: mesh as *const MeshData as usize,
        mesh_tag: (mesh.tri_count(), mesh.uv_hash),
        origin_bits: p.origin.to_array().map(f32::to_bits),
        rotation_bits: p.rotation.to_array().map(f32::to_bits),
        spine_y: reach.spine_y,
        options: opts,
        reach: reach.reach,
        needed_along: overlap_turns.then_some(reach.needed_along.to_bits()),
        src_size,
        texel_world,
        half_bits: [p.half_size.x.abs().to_bits(), p.half_size.y.abs().to_bits()],
        layers: layers.into_iter().map(|l| l.1).collect(),
        stats,
    })
}

/// What the owner grid of a layer needs besides its triangles (round 9, T1).
struct OwnerCtx<'c> {
    mesh: &'c MeshData,
    bvh: &'c Bvh,
    patch: &'c Patch,
    /// The layout's triangle orientations and entered parts.
    orient: &'c [i8],
    entered: &'c [bool],
    /// Laid local triangles (`Layout::laid`).
    laid: &'c [bool],
}

impl OwnerCtx<'_> {
    /// The local triangle of mesh triangle `tri` when it belongs to an entered part other than `own`.
    fn other(&self, tri: TriId, own: u32) -> Option<u32> {
        let patch = self.patch;
        let l = patch.tris.binary_search(&tri).ok()? as u32;
        let part = patch.part[patch.tv[l as usize][0] as usize];
        (part != own && self.entered.get(part as usize).copied().unwrap_or(false)).then_some(l)
    }

    /// Round 9 (T1): false when no point of the triangle with corners `p3` (oriented normal `n`, part `own`) can be
    /// buried: no triangle of another entered part meets the box of the prism its normal sweeps over `BURY_REACH x
    /// diag` (grown by `1e-5 diag`).
    fn may_bury(&self, p3: [Vec3; 3], n: DVec3, own: u32) -> bool {
        if n == DVec3::ZERO || !n.is_finite() || !self.entered.iter().any(|&e| e) {
            return false;
        }
        let diag = self.patch.diag;
        let (eps, reach) = ((1e-5 * diag) as f32, (BURY_REACH * diag) as f32);
        let d = Vec3::new(n.x as f32, n.y as f32, n.z as f32) * reach;
        let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        for c in p3 {
            lo = lo.min(c).min(c + d);
            hi = hi.max(c).max(c + d);
        }
        let (lo, hi) = (lo - Vec3::splat(eps), hi + Vec3::splat(eps));
        if !(lo.is_finite() && hi.is_finite()) {
            return false;
        }
        let mut near: Vec<(TriId, f32)> = Vec::new();
        self.bvh.query_sphere_dist(self.mesh, (lo + hi) * 0.5, (hi - lo).length() * 0.5, &mut near);
        near.iter().any(|&(tri, _)| {
            self.other(tri, own).is_some() && {
                let [a, b, c] = self.mesh.tri_positions(tri);
                let (tlo, thi) = (a.min(b).min(c), a.max(b).max(c));
                tlo.cmple(hi).all() && thi.cmpge(lo).all()
            }
        })
    }

    /// Round 9 (T1, answer (a)-1): true when the surface at `p` (oriented normal `n`, part `own`) lies under another
    /// entered part: a face of such a part touches `p` parallel to it (within `1e-5 diag`: something lying on it), or
    /// a ray from `p` along `n` hits such a part within `BURY_REACH x diag` (the floor under a cube, the board under a
    /// piece's base).
    fn buried_at(&self, p: DVec3, n: DVec3, own: u32) -> bool {
        let patch = self.patch;
        if n == DVec3::ZERO || !p.is_finite() || !n.is_finite() {
            return false;
        }
        let eps = 1e-5 * patch.diag;
        let o = Vec3::new(p.x as f32, p.y as f32, p.z as f32);
        let mut near: Vec<(TriId, f32)> = Vec::new();
        self.bvh.query_sphere_dist(self.mesh, o, eps as f32, &mut near);
        if near.iter().any(|&(tri, _)| self.other(tri, own).is_some_and(|l| patch.geo[l as usize].dot(n).abs() > 0.99)) {
            return true;
        }
        let ray = Ray::new(o, Vec3::new(n.x as f32, n.y as f32, n.z as f32));
        let t_max = (BURY_REACH * patch.diag) as f32;
        let mut t_min = eps as f32;
        for _ in 0..16 {
            let Some(hit) = self.bvh.raycast(self.mesh, &ray, t_min, t_max, Cull::None) else { return false };
            if self.other(hit.tri, own).is_some() {
                return true;
            }
            t_min = hit.t + (eps as f32).max(hit.t * 1e-6);
        }
        false
    }
}

/// The owner grid's counts of a layer: (rejected, all) (cell centre, laid triangle) pairs and (lost_cut, all) picture
/// cells.
type OwnerCounts = ((usize, usize), (usize, usize));

/// The stored layer of one turn: ascending mesh triangle, f32 corners and corner depths, and (a layer not clipped to
/// an Overlap turn's range) the owner grid over the reach rectangle `reach` (half extents; round 9: the place owner).
/// Also returns the owner test's (cell centre, triangle) pairs inside `picture`: (rejected, all).
fn make_layer(
    ctx: &OwnerCtx<'_>,
    mut tris: Vec<(u32, [DVec2; 3], [f64; 3])>,
    clip: Option<[f32; 2]>,
    period: Option<f32>,
    axis: usize,
    reach: [f64; 2],
    picture: [f64; 2],
) -> (WrapLayer, OwnerCounts) {
    let patch = ctx.patch;
    tris.sort_by_key(|e| patch.tris[e.0 as usize]);
    let (owner, rej) = if clip.is_none() {
        // The corners as stored (f32) are what the bake interpolates: the grid is built from them, with the mesh's
        // f32 positions.
        let stored: Vec<relax::OwnerTri> = tris
            .iter()
            .map(|e| {
                let q32 = e.1.map(|q| Vec2::new(q.x as f32, q.y as f32));
                let p3 = ctx.mesh.tri_positions(patch.tris[e.0 as usize]);
                let o = if ctx.orient.get(e.0 as usize).copied().unwrap_or(1) < 0 { -1.0 } else { 1.0 };
                relax::OwnerTri {
                    q: q32.map(|q| DVec2::new(q.x as f64, q.y as f64)),
                    g: e.2.map(|g| g as f32 as f64),
                    p3,
                    s: relax::owner_scale(p3, q32),
                    part: patch.part[patch.tv[e.0 as usize][0] as usize],
                    normal: patch.geo[e.0 as usize] * o,
                    laid: ctx.laid.get(e.0 as usize).copied().unwrap_or(false),
                }
            })
            .collect();
        let may = |i: usize| ctx.may_bury(stored[i].p3, stored[i].normal, stored[i].part);
        let at = |i: usize, p: DVec3| ctx.buried_at(p, stored[i].normal, stored[i].part);
        let (grid, rej, lost) = OwnerGrid::build(&stored, reach, 0.0, picture, &relax::Burial { may: &may, at: &at });
        (Some(grid), (rej, lost))
    } else {
        (None, ((0, 0), (0, 0)))
    };
    let layer = WrapLayer {
        tris: tris.iter().map(|e| patch.tris[e.0 as usize]).collect(),
        corners: tris.iter().map(|e| e.1.map(|q| [q.x as f32, q.y as f32])).collect(),
        clip,
        period,
        axis,
        depth: tris.iter().map(|e| e.2.map(|g| g as f32)).collect(),
        owner,
        grid: OnceLock::new(),
    };
    (layer, rej)
}

/// Barycentric weights of `p` in the triangle `(a, b, c)` (centroid for a degenerate one).
fn barycentric(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> [f64; 3] {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den == 0.0 || !den.is_finite() {
        return [1.0 / 3.0; 3];
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    [1.0 - v - w, v, w]
}

// ----- queries -----

/// A uniform grid over a layer's coordinates for `surface_of`.
struct Grid {
    lo: [f32; 2],
    cell: [f32; 2],
    dims: [usize; 2],
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    fn build(layer: &WrapLayer) -> Grid {
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for c in &layer.corners {
            for q in c {
                for a in 0..2 {
                    lo[a] = lo[a].min(q[a]);
                    hi[a] = hi[a].max(q[a]);
                }
            }
        }
        let n = layer.corners.len().max(1);
        let side = ((n as f64).sqrt().ceil() as usize).clamp(1, 1024);
        let dims = [side, side];
        let ext = [(hi[0] - lo[0]).max(1e-30), (hi[1] - lo[1]).max(1e-30)];
        let cell = [ext[0] / side as f32, ext[1] / side as f32];
        let index = |v: f32, a: usize| -> usize { (((v - lo[a]) / cell[a]).floor().max(0.0) as usize).min(dims[a] - 1) };
        let mut count = vec![0u32; dims[0] * dims[1] + 1];
        let span = |c: &[[f32; 2]; 3], a: usize| {
            let (mn, mx) = (c[0][a].min(c[1][a]).min(c[2][a]), c[0][a].max(c[1][a]).max(c[2][a]));
            (index(mn, a), index(mx, a))
        };
        let finite = |c: &[[f32; 2]; 3]| c.iter().all(|q| q[0].is_finite() && q[1].is_finite());
        for c in &layer.corners {
            if !finite(c) {
                continue;
            }
            let ((x0, x1), (y0, y1)) = (span(c, 0), span(c, 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    count[y * dims[0] + x + 1] += 1;
                }
            }
        }
        for i in 0..dims[0] * dims[1] {
            count[i + 1] += count[i];
        }
        let mut fill = count.clone();
        let mut items = vec![0u32; count[dims[0] * dims[1]] as usize];
        for (i, c) in layer.corners.iter().enumerate() {
            if !finite(c) {
                continue;
            }
            let ((x0, x1), (y0, y1)) = (span(c, 0), span(c, 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let k = y * dims[0] + x;
                    items[fill[k] as usize] = i as u32;
                    fill[k] += 1;
                }
            }
        }
        Grid { lo, cell, dims, start: count, items }
    }

    /// Indices of the triangles whose box covers the cell of `p` (ascending), or none outside the grid.
    fn cell_items(&self, p: Vec2) -> &[u32] {
        let fx = (p.x - self.lo[0]) / self.cell[0];
        let fy = (p.y - self.lo[1]) / self.cell[1];
        if !(fx >= 0.0 && fy >= 0.0) || fx > self.dims[0] as f32 || fy > self.dims[1] as f32 {
            return &[];
        }
        let (x, y) = ((fx as usize).min(self.dims[0] - 1), (fy as usize).min(self.dims[1] - 1));
        let k = y * self.dims[0] + x;
        &self.items[self.start[k] as usize..self.start[k + 1] as usize]
    }
}

impl WrapLayer {
    fn grid(&self) -> &Grid {
        self.grid.get_or_init(|| Grid::build(self))
    }
}

impl WrapParam {
    /// What the parameterization covers and how long it took.
    pub fn stats(&self) -> WrapStats {
        self.stats
    }

    /// The source size the parameterization was made for.
    pub fn src_size(&self) -> [u32; 2] {
        self.src_size
    }

    /// The largest texel size (world units) of the texel maps it was made with.
    pub fn texel_world(&self) -> f32 {
        self.texel_world
    }

    /// `stats` for the picture of `p` (a projector this parameterization serves): `covered` is measured for `p`'s
    /// size. A parameterization is reused for a scale inside its reach bucket (4.4), and its stored `covered` is the
    /// share of the picture it was built for; a larger picture may run off the surface where the smaller did not.
    /// Every other field is the stored one (`reached_edge` stays the one of the build).
    pub fn stats_for(&self, p: &Projector) -> WrapStats {
        let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
        if [hw.to_bits(), hh.to_bits()] == self.half_bits {
            return self.stats;
        }
        let (hw, hh) = (hw as f64, hh as f64);
        if !(hw > 0.0 && hh > 0.0 && hw.is_finite() && hh.is_finite()) {
            return self.stats;
        }
        let mut area = 0.0;
        for layer in &self.layers {
            let clip = layer.clip.map(|[lo, hi]| (layer.axis, lo as f64, hi as f64));
            let period = layer.period.map(|t| t as f64).filter(|t| *t > 0.0 && clip.is_some());
            for c in &layer.corners {
                let q = c.map(|q| DVec2::new(q[0] as f64, q[1] as f64));
                if !q.iter().all(|p| p.is_finite()) {
                    continue;
                }
                area += match (clip, period) {
                    (Some((axis, _, _)), Some(t)) => [-t, 0.0, t]
                        .iter()
                        .map(|&shift| {
                            let mut moved = q;
                            for p in &mut moved {
                                p[axis] += shift;
                            }
                            clipped_area(moved, hw, hh, clip)
                        })
                        .sum::<f64>(),
                    _ => clipped_area(q, hw, hh, clip),
                };
            }
        }
        WrapStats { covered: (area / (4.0 * hw * hh)).clamp(0.0, 1.0) as f32, ..self.stats }
    }

    /// True when this parameterization serves `p` on `mesh` (4.4): the same mesh, `origin` and `rotation`
    /// bit-equal, the same spine axis and Wrap options, and the same bucketed reach (computed with the source size
    /// and texel size this parameterization was made for; `serves_inputs` takes the current ones).
    pub fn serves(&self, mesh: &std::sync::Arc<MeshData>, p: &Projector) -> bool {
        self.serves_with(mesh, self.src_size, self.texel_world, p)
    }

    /// `serves` with the current texel maps and source size.
    pub(crate) fn serves_inputs(
        &self,
        mesh: &std::sync::Arc<MeshData>,
        maps: &[std::sync::Arc<TexelMap>],
        src_size: [u32; 2],
        p: &Projector,
    ) -> bool {
        self.serves_with(mesh, src_size, max_texel_world(maps), p)
    }

    fn serves_with(&self, mesh: &std::sync::Arc<MeshData>, src_size: [u32; 2], texel_world: f32, p: &Projector) -> bool {
        if std::sync::Arc::as_ptr(mesh) as usize != self.mesh_addr || (mesh.tri_count(), mesh.uv_hash) != self.mesh_tag {
            return false;
        }
        if p.origin.to_array().map(f32::to_bits) != self.origin_bits
            || p.rotation.to_array().map(f32::to_bits) != self.rotation_bits
            || p.wrap != self.options
        {
            return false;
        }
        match reach_of(p, src_size, texel_world, mesh.diag() as f64) {
            Some(r) => {
                r.spine_y == self.spine_y
                    && r.reach.map(f64::to_bits) == self.reach.map(f64::to_bits)
                    && self.needed_along.is_none_or(|bits| bits == r.needed_along.to_bits())
            }
            None => false,
        }
    }

    /// The top layer (the last turn laid on top).
    fn top(&self) -> &WrapLayer {
        &self.layers[self.layers.len() - 1]
    }

    /// Surface -> stamp-local coordinates (world units along the projector's right / up, origin at the anchor), in
    /// the top turn; `None` for a triangle the parameterization does not cover.
    pub fn local_of(&self, tri: TriId, bary: [f32; 3]) -> Option<Vec2> {
        let layer = self.top();
        let i = layer.tris.binary_search(&tri).ok()?;
        let c = layer.corners[i];
        let p = Vec2::new(
            c[0][0] * bary[0] + c[1][0] * bary[1] + c[2][0] * bary[2],
            c[0][1] * bary[0] + c[1][1] * bary[1] + c[2][1] * bary[2],
        );
        p.is_finite().then_some(p)
    }

    /// Stamp-local coordinates -> surface point (top turn); `None` where no surface lies under them. On a shared
    /// edge the first triangle (ascending id) containing the point is taken.
    pub fn surface_of(&self, mesh: &MeshData, local: Vec2) -> Option<SurfacePoint> {
        if !local.is_finite() {
            return None;
        }
        let layer = self.top();
        let grid = layer.grid();
        // Round 9 (T1): of the triangles containing the point the owner's place (the one whose 3D point there lies
        // nearest the cell owner's point), else (no owner) the one nearest the spine along the surface (lowest depth
        // `g` there, then the first); else the one the point is least outside of, within a small tolerance.
        let owner_point = layer.owner.as_ref().and_then(|o| o.index(local.x, local.y).and_then(|k| o.owner_at(k)));
        let mut best: Option<(f32, usize, [f32; 3])> = None;
        let mut owner: Option<(f32, usize, [f32; 3])> = None;
        for &i in grid.cell_items(local) {
            let c = layer.corners[i as usize];
            let (a, b, d) = (Vec2::from(c[0]), Vec2::from(c[1]), Vec2::from(c[2]));
            let area = (b - a).perp_dot(d - a);
            if area == 0.0 || !area.is_finite() {
                continue;
            }
            let w0 = (d - b).perp_dot(local - b) / area;
            let w1 = (a - d).perp_dot(local - d) / area;
            let w2 = 1.0 - w0 - w1;
            let inside = w0.min(w1).min(w2);
            if inside >= 0.0 {
                let g = match owner_point {
                    Some(o) => {
                        let p = mesh.tri_positions(layer.tris[i as usize]);
                        (p[0] * w0 + p[1] * w1 + p[2] * w2).distance(o)
                    }
                    None => layer.depth.get(i as usize).map_or(0.0, |z| z[0] * w0 + z[1] * w1 + z[2] * w2),
                };
                if owner.is_none_or(|(o, _, _)| g < o) {
                    owner = Some((g, i as usize, [w0, w1, w2]));
                }
            } else if inside >= -1e-5 && best.is_none_or(|(b, _, _)| inside > b) {
                best = Some((inside, i as usize, [w0, w1, w2]));
            }
        }
        let (_, i, bary) = owner.or(best)?;
        let tri = layer.tris[i];
        if tri >= mesh.tri_count() {
            return None;
        }
        let p = mesh.tri_positions(tri);
        let n = mesh.tri_normals(tri);
        let position = p[0] * bary[0] + p[1] * bary[1] + p[2] * bary[2];
        let normal = (n[0] * bary[0] + n[1] * bary[1] + n[2] * bary[2]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO || !normal.is_finite() { mesh.geo_normal(tri) } else { normal };
        Some(SurfacePoint { tri, bary, position, normal })
    }

    /// The picture's border on the surface (top turn): the rectangle `half_size` sampled with `samples_per_edge`
    /// points per side, as polylines split where the border leaves the surface (a closed border is one polyline
    /// whose last point equals its first).
    pub fn outline(&self, mesh: &MeshData, half_size: Vec2, samples_per_edge: usize) -> Vec<Vec<SurfacePoint>> {
        let n = samples_per_edge.max(1);
        let (hw, hh) = (half_size.x.abs(), half_size.y.abs());
        let corners = [Vec2::new(-hw, hh), Vec2::new(hw, hh), Vec2::new(hw, -hh), Vec2::new(-hw, -hh)];
        let points: Vec<Option<SurfacePoint>> = (0..4 * n + 1)
            .map(|k| {
                let (side, i) = ((k / n) % 4, k % n);
                let (a, b) = (corners[side], corners[(side + 1) % 4]);
                self.surface_of(mesh, a + (b - a) * (i as f32 / n as f32))
            })
            .collect();
        let mut lines: Vec<Vec<SurfacePoint>> = Vec::new();
        let mut current: Vec<SurfacePoint> = Vec::new();
        for p in &points {
            match p {
                Some(sp) => current.push(*sp),
                None => {
                    if current.len() > 1 {
                        lines.push(std::mem::take(&mut current));
                    }
                    current.clear();
                }
            }
        }
        if current.len() > 1 {
            lines.push(current);
        }
        // The border passes the first corner on the surface but leaves it elsewhere: the last run continues into
        // the first one.
        let wraps = points[0].is_some() && points[4 * n].is_some() && points.iter().any(Option::is_none);
        if wraps && lines.len() > 1 {
            let first = lines.remove(0);
            if let Some(last) = lines.last_mut() {
                last.extend(first.into_iter().skip(1));
            }
        }
        lines
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;

    /// One layer's triangles and corners.
    pub(crate) type Layer = (Vec<TriId>, Vec<[[f32; 2]; 3]>);

    /// Every layer's (tri, corners), for tests.
    pub(crate) fn layers(p: &WrapParam) -> Vec<Layer> {
        p.layers.iter().map(|l| (l.tris.clone(), l.corners.clone())).collect()
    }

    pub(crate) fn reach(p: &WrapParam) -> [f64; 2] {
        p.reach
    }
}
