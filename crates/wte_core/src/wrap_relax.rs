// Owner: round 8, package A (R8-1). Seams, wedges and the seam-aware relaxation of a wrapped stamp's layout
// (ARCHITECTURE.md 6.5a steps 4-6, plan 2.2 / 3.2): a child module of `wrap_param` (it reads the patch and the layout
// state of one turn).
//
// - Seams (where the sheet may be cut), one rule each, all deterministic: S1 ends meet (closed spine: feet more than
//   half the period apart), S2 halves meet (opposite sides, both farther than `SEAM_NEAR` edges from the spine), S3 an
//   orientation flip or a non-manifold edge, S4 a cone cut (|angle deficit| > `CONE_CUT_DEG`, along the steepest
//   ascent of `g` to the laid set's boundary), S5 the tree-cotree remnant (cuts only what S1-S4 left non-disk).
// - Wedges: one unknown 2D position per vertex fan between seam edges, so the relaxed layout is continuous everywhere
//   except on seams by construction.
// - Relaxation: LSCM (conformal least squares) from the seed, then ARAP local / global iterations, both solved by a
//   matrix-free conjugate gradient with a Jacobi preconditioner. The strip seeds are pinned (the centre line stays
//   exact), each jumped part by its landing triangle. Deterministic: per-triangle terms are gathered per unknown in a
//   fixed order and sums are added chunk by chunk in chunk order, with 1 or N threads alike.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use glam::{DVec2, DVec3, Vec2, Vec3};
use rayon::prelude::*;

use super::{Frame, Layout, NONE, Patch, State, TEAR_RATIO, cross2, edge_of};

/// The early-out (2.2-3): a seed whose triangles meeting the picture have every edge ratio within `1 +- ISO_TOL` is
/// kept as it is (developable surfaces stay exact, no solver cost).
pub const ISO_TOL: f64 = 2e-4;
/// S4: an interior vertex whose angle deficit exceeds this (degrees, either sign) is a cone: one cut runs from it.
pub const CONE_CUT_DEG: f64 = 45.0;
/// Round 9 (T3): S4 also needs the net deficit of the laid interior vertices within this share of the bucketed across
/// reach (3D distance, walked over laid edges) to exceed `CONE_CUT_DEG`: a rivet, a bevel, a groove (whose tip and
/// ring cancel) is no cone.
pub const CONE_RADIUS: f64 = 0.1;
/// S2: both ends of an edge between the two halves must lie farther than this many times their longest incident edge
/// from the spine.
pub const SEAM_NEAR: f64 = 3.0;
/// S1 on an open spine (check:A): neighbours whose spine feet lie more than this many times their surface detour
/// (`g_a + |ab| + g_b`) apart were reached from the spine's two ends: the ends meet there.
pub const ENDS_MEET: f64 = 2.0;
/// S5 (check:A): a hole of the laid set whose boundary loop is at most this many times the longest edge at its
/// vertices is filled for the tree-cotree rule (no cut leads to it).
pub const HOLE_FILL: f64 = 8.0;

/// Union-find: the root of `x` (path halving).
fn uf_find(uf: &mut [u32], mut x: u32) -> u32 {
    while uf[x as usize] != x {
        uf[x as usize] = uf[uf[x as usize] as usize];
        x = uf[x as usize];
    }
    x
}

/// Union-find: joins the sets of `a` and `b` (the smaller root wins: deterministic).
fn uf_join(uf: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (uf_find(uf, a), uf_find(uf, b));
    if ra != rb {
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        uf[hi as usize] = lo;
    }
}
/// The owner grid: at most this many cells on its long side.
pub const OWNER_MAX_CELLS: usize = 1024;
/// ARAP local / global iterations from the start (round 9, T4: the Fermi start, no LSCM; fewer when the energy drops by
/// less than `ARAP_STOP`).
pub const RELAX_ARAP_ITERS: u32 = 12;
/// Conjugate-gradient iterations per component and layer, all solves together.
pub const RELAX_CG_BUDGET: u32 = 2_500;
/// From this many unknowns the kernels run in parallel (fixed chunks of `PAR_CHUNK`).
pub const PAR_MIN_UNKNOWNS: usize = 16_384;
pub const PAR_CHUNK: usize = 8_192;
const ARAP_STOP: f64 = 0.0005;
const LSCM_TOL: f64 = 1e-6;
const LSCM_ITERS: u32 = 600;
const ARAP_TOL: f64 = 1e-5;
/// Round 9 (T4): CG steps per ARAP global solve (round 8: 200).
const ARAP_ITERS: u32 = 60;
/// Plan 3.1: cotangents are clamped to `|cot| <= COT_MAX`. In this gradient-basis form a triangle with an angle whose
/// cotangent exceeds it (a sliver: an angle below ~0.06 degrees or above ~179.94) is left out of the energies like a
/// zero-area one (its wedges keep the seed position when they have no other triangle).
pub(super) const COT_MAX: f64 = 1e3;

/// check:A (round 8): test builds may lower the size from which the kernels run in parallel (the determinism test of
/// the parallel path: the bits must not depend on it).
#[cfg(test)]
pub(crate) static PAR_MIN_TEST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(PAR_MIN_UNKNOWNS);
#[cfg(test)]
fn par_min() -> usize {
    PAR_MIN_TEST.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg(not(test))]
fn par_min() -> usize {
    PAR_MIN_UNKNOWNS
}

/// True when a triangle with corners `p` has an angle whose cotangent exceeds `COT_MAX` (a sliver).
pub(super) fn sliver(p: [DVec3; 3]) -> bool {
    (0..3).any(|k| {
        let (u, w) = (p[(k + 1) % 3] - p[k], p[(k + 2) % 3] - p[k]);
        let s = u.cross(w).length();
        // NaN (a zero edge) counts as a sliver too.
        u.dot(w).abs().partial_cmp(&(COT_MAX * s)).is_none_or(|o| o == Ordering::Greater)
    })
}

/// Seam rule bits (`Seams::cut`).
pub(super) const S1: u8 = 1;
pub(super) const S2: u8 = 2;
pub(super) const S3: u8 = 4;
pub(super) const S4: u8 = 8;
pub(super) const S5: u8 = 16;

/// Side of the spine of a position with across coordinate `across` (0 within 1e-9 of the diagonal).
pub(super) fn side_of(across: f64, diag: f64) -> i8 {
    if across.abs() <= 1e-9 * diag {
        0
    } else if across > 0.0 {
        1
    } else {
        -1
    }
}

/// Geodesic distance at `w` from the distances `gv`, `ga` at `v`, `a` by the fast-marching triangle update (a plane
/// wave with unit gradient through the triangle); `None` when the wave does not reach `w` through the edge `(v, a)`
/// (non-causal) or the triangle is degenerate.
#[cfg(test)]
pub(super) fn fmm_update(v: DVec3, a: DVec3, w: DVec3, gv: f64, ga: f64) -> Option<f64> {
    fmm_update_tau(v, a, w, gv, ga).map(|(g, _)| g)
}

/// Round 9 (T4): `fmm_update` and where the wave's characteristic through `w` meets the edge `(v, a)`: `tau = -alpha /
/// beta` in 0..1 (0 at `v`, 1 at `a`).
pub(super) fn fmm_update_tau(v: DVec3, a: DVec3, w: DVec3, gv: f64, ga: f64) -> Option<(f64, f64)> {
    let (ea, eb) = (a - v, w - v);
    let (aa, ab, bb) = (ea.dot(ea), ea.dot(eb), eb.dot(eb));
    let det = aa * bb - ab * ab;
    if !(det > 1e-24 * aa * bb && det.is_finite()) {
        return None;
    }
    let da = ga - gv;
    if da * da >= aa {
        return None;
    }
    let t = (ab * da + (det * (aa - da * da)).sqrt()) / aa;
    let alpha = (bb * da - ab * t) / det;
    let beta = (aa * t - ab * da) / det;
    (beta > 0.0 && alpha <= 0.0 && alpha >= -beta && t.is_finite()).then_some((gv + t, (-alpha / beta).clamp(0.0, 1.0)))
}

/// The seams of one turn's laid triangles.
pub(super) struct Seams {
    /// Per half-edge `3 t + k`: the rules (bits S1-S5) that cut its edge; 0 = not cut.
    pub(super) cut: Vec<u8>,
}

/// The wedges and components of one turn's laid triangles.
pub(super) struct Wedges {
    /// Per corner `3 t + k` of a laid triangle: its wedge (`NONE` for other triangles).
    pub(super) corner: Vec<u32>,
    /// Vertex of each wedge.
    pub(super) vertex: Vec<u32>,
    /// Component of each laid triangle (`NONE` for others) and the component count.
    pub(super) comp: Vec<u32>,
    pub(super) comps: u32,
}

/// What the relaxation of one turn gives.
pub(super) struct Relaxed {
    /// Per stored triangle: local triangle, corners, corner depths `g`.
    pub(super) tris: Vec<(u32, [DVec2; 3], [f64; 3])>,
    /// Seam edges inside the picture (all rules) and per rule S1..S5.
    pub(super) seams: u32,
    pub(super) seams_by_rule: [u32; 5],
    /// Triangles with a seam edge inside the picture, relaxed triangles flipped inside the picture, CG iterations.
    pub(super) cut_tris: u32,
    pub(super) flipped: u32,
    pub(super) cg_iters: u32,
    /// check:A (round 9): per local triangle, laid with an edge cut by a closure (S1 / S2): where the ends or the
    /// halves meet (`lost_cut` does not count the picture beyond that line).
    pub(super) closure: Vec<bool>,
}

/// True when the 2D box of `q` meets `|x| <= rect[0], |y| <= rect[1]`.
pub(super) fn meets(q: &[DVec2], rect: [f64; 2]) -> bool {
    let (lo, hi) = q.iter().fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    lo.x <= rect[0] && hi.x >= -rect[0] && lo.y <= rect[1] && hi.y >= -rect[1]
}

/// The half-edges of the group of half-edge `he` that belong to laid triangles.
fn laid_members<'p>(patch: &'p Patch, laid: &'p [bool], he: usize) -> impl Iterator<Item = u32> + 'p {
    let (start, len) = patch.he_group[he];
    patch.edge_tris[start as usize..(start + len) as usize].iter().copied().filter(move |&h| laid[(h / 3) as usize])
}

/// True when `he` is the first laid half-edge of its group (each undirected laid edge is visited once).
fn first_laid(patch: &Patch, laid: &[bool], he: usize) -> bool {
    laid_members(patch, laid, he).next() == Some(he as u32)
}

/// Corner index of vertex `v` in local triangle `t`.
fn corner_of(patch: &Patch, t: u32, v: u32) -> usize {
    patch.tv[t as usize].iter().position(|&x| x == v).unwrap_or(0)
}

impl<'a> Layout<'a> {
    /// Laid triangles: every corner final and the triangle not degenerate.
    pub(super) fn laid(&self) -> Vec<bool> {
        let patch = self.patch;
        (0..patch.tv.len() as u32)
            .map(|t| !patch.degenerate(t) && patch.tv[t as usize].iter().all(|&x| self.state[x as usize] == State::Final))
            .collect()
    }

    /// S1 / S2 between vertices `a` and `b` (bits).
    fn closure_rules(&self, a: u32, b: u32, period: Option<f64>) -> u8 {
        let (a, b) = (a as usize, b as usize);
        let mut bits = 0;
        let l3 = self.patch.pos[a].distance(self.patch.pos[b]);
        let torn = || {
            let l2 = self.p2[a].distance(self.p2[b]);
            !(l2 <= TEAR_RATIO * l3 && l2 * TEAR_RATIO >= l3)
        };
        let df = (self.foot[a] - self.foot[b]).abs();
        match period {
            Some(p) if p > 0.0 => {
                if df > 0.5 * p {
                    bits |= S1;
                }
            }
            // check:A (round 8): the ends meet without the spine closing (the traces pass beside each other without
            // sharing a triangle: a tapering stem, a body of revolution that is not a cylinder): two neighbours whose
            // spine feet lie much farther apart along the spine than any surface path between those feet through them
            // (`g_a + |ab| + g_b`) were reached from the two ends of the spine. Without this the closure has no seam
            // and the relaxation tears the picture elsewhere (S5) and folds it (check_a8: a picture round the lathe
            // piece's stem had dup 58 %).
            _ => {
                if df > ENDS_MEET * (self.key[a] + self.key[b] + l3) && df > SEAM_NEAR * self.patch.max_edge[a].max(self.patch.max_edge[b]) && torn() {
                    bits |= S1;
                }
            }
        }
        // S2 also needs the seed to be torn across the edge (its seed length off the 3D length by more than
        // TEAR_RATIO): fronts that went round a bump on the spine meet with nearby seed positions (no cut line behind
        // each bump), fronts that met behind an object or a hole come from opposite sides of the picture.
        let far = |v: usize| self.key[v] > SEAM_NEAR * self.patch.max_edge[v];
        if (self.side[a] as i32) * (self.side[b] as i32) < 0 && far(a) && far(b) && torn() {
            bits |= S2;
        }
        bits
    }

    /// The early-out test (2.2-3) on the seed result `seed` (local triangle, corners, torn): every triangle meeting
    /// the picture rectangle `rect` (with its border) has all edge ratios within `1 +- ISO_TOL`, or is torn by a
    /// closure (its final corners differ by S1 / S2).
    pub(super) fn seed_is_exact(&self, seed: &[(u32, [DVec2; 3], bool)], rect: [f64; 2], period: Option<f64>) -> bool {
        let patch = self.patch;
        for (t, q, torn) in seed {
            if patch.degenerate(*t) || !meets(q, rect) {
                continue;
            }
            let c = patch.tv[*t as usize];
            // check:A (round 8): a halo triangle with one laid corner is placed by the frame rule of that corner on
            // either path (a projection, so it is "torn" wherever the surface turns); the relaxation cannot make it
            // better. It does not decide (WA16's block side faces, reached by the 1.7 x front: the exact seed was
            // relaxed for nothing, WA25). Halo triangles with two laid corners (hinged) still do.
            if c.iter().filter(|&&x| self.state[x as usize] == State::Final).count() < 2 {
                continue;
            }
            if *torn {
                let fin: Vec<u32> = c.iter().copied().filter(|&x| self.state[x as usize] == State::Final).collect();
                let closure = fin.iter().enumerate().any(|(i, &a)| fin[i + 1..].iter().any(|&b| self.closure_rules(a, b, period) != 0));
                if !closure {
                    if knob("WTE_TEST_RELAX_DEBUG", false) {
                        eprintln!("early-out: torn tri {} (finals {}), corners {q:?}, rect {rect:?}", patch.tris[*t as usize], fin.len());
                    }
                    return false;
                }
                continue;
            }
            let p3 = patch.corners(*t);
            for i in 0..3 {
                let j = (i + 1) % 3;
                let (l3, l2) = (p3[i].distance(p3[j]), q[i].distance(q[j]));
                if l3 > 0.0 && (l2 / l3 - 1.0).abs() > ISO_TOL {
                    if knob("WTE_TEST_RELAX_DEBUG", false) {
                        eprintln!("early-out: tri {} edge ratio {:.6}, corners {q:?}", patch.tris[*t as usize], l2 / l3);
                    }
                    return false;
                }
            }
        }
        true
    }

    /// Depth `g` of corner vertex `x` of triangle `t` for the store: its own when final or reached, else the nearest
    /// final corner's plus the 3D distance.
    pub(super) fn corner_g(&self, t: u32) -> [f64; 3] {
        let patch = self.patch;
        let c = patch.tv[t as usize];
        let fin: Vec<usize> = (0..3).filter(|&i| self.state[c[i] as usize] == State::Final).collect();
        std::array::from_fn(|i| {
            let x = c[i] as usize;
            if self.key[x].is_finite() && self.state[x] != State::Unknown {
                return self.key[x];
            }
            fin.iter().map(|&j| self.key[c[j] as usize] + patch.pos[c[j] as usize].distance(patch.pos[x])).fold(f64::INFINITY, f64::min)
        })
    }

    /// Steps 5-6 (3.2): seams over the laid triangles, wedges, LSCM then ARAP per component, the halo from the relaxed
    /// corners. `seed` is the seed result (local triangle, corners, torn); `rect` the picture rectangle with its
    /// border; `period` the closed spine's period (S1); `anchor` the anchor's local triangle.
    pub(super) fn relax(&mut self, seed: &[(u32, [DVec2; 3], bool)], rect: [f64; 2], period: Option<f64>, anchor: u32, cone_radius: f64) -> Relaxed {
        let patch = self.patch;
        let ntri = patch.tv.len();
        let laid = self.laid();
        let seams = self.seams(&laid, period, anchor, cone_radius);
        let wedges = wedges(patch, &laid, &seams);
        let nw = wedges.vertex.len();
        // Initial positions: the seed positions of the wedges' vertices.
        let mut x: Vec<DVec2> = wedges.vertex.iter().map(|&v| self.p2[v as usize]).collect();
        let mut pinned = vec![false; nw];
        let wedge_at = |t: u32, v: u32| wedges.corner[t as usize * 3 + corner_of(patch, t, v)];
        // Pins: each strip seed in the wedge of its strip triangle.
        for v in 0..patch.pos.len() {
            let st = self.seed_tri[v];
            if st == NONE || !laid[st as usize] {
                continue;
            }
            let w = wedge_at(st, v as u32);
            if w != NONE {
                pinned[w as usize] = true;
                x[w as usize] = self.p2[v];
            }
        }
        // Rest geometry per laid triangle: area and the gradient basis of its development (orientation of `orient`).
        let diag2 = patch.diag * patch.diag;
        let mut rest: Vec<Option<(f64, [DVec2; 3])>> = vec![None; ntri];
        for t in 0..ntri {
            if !laid[t] {
                continue;
            }
            let p = patch.corners(t as u32);
            let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
            let l1 = e1.length();
            let area = 0.5 * e1.cross(e2).length();
            if !(l1 > 0.0 && area >= 1e-12 * diag2 && area.is_finite()) || sliver(p) {
                continue;
            }
            let xh = e1 / l1;
            let e2x = e2.dot(xh);
            let s = if self.orient[t] < 0 { -1.0 } else { 1.0 };
            let e2y = s * (e2 - xh * e2x).length();
            let det = l1 * e2y;
            let g1 = DVec2::new(e2y, -e2x) / det;
            let g2 = DVec2::new(0.0, l1) / det;
            rest[t] = Some((area, [-(g1 + g2), g1, g2]));
        }
        // Components: strip components first (ascending id), then jumped parts in jump order, then the rest by their
        // lowest g.
        let mut comp_tris: Vec<Vec<u32>> = vec![Vec::new(); wedges.comps as usize];
        for t in 0..ntri {
            if wedges.comp[t] != NONE {
                comp_tris[wedges.comp[t] as usize].push(t as u32);
            }
        }
        let tri_g = |t: u32| patch.tv[t as usize].iter().map(|&v| self.key[v as usize]).fold(f64::NEG_INFINITY, f64::max);
        let mut order: Vec<(u8, f64, u32)> = Vec::with_capacity(comp_tris.len());
        for (c, tris) in comp_tris.iter().enumerate() {
            let strip = tris.iter().any(|&t| patch.tv[t as usize].iter().any(|&v| self.seed_tri[v as usize] != NONE && pinned_vertex(&wedges, &pinned, t, v, patch)));
            let jump = self.jump_log.iter().position(|&(_, lt)| wedges.comp.get(lt as usize).copied() == Some(c as u32));
            let low = tris.iter().map(|&t| tri_g(t)).fold(f64::INFINITY, f64::min);
            order.push(match (strip, jump) {
                (true, _) => (0, 0.0, c as u32),
                (false, Some(j)) => (1, j as f64, c as u32),
                _ => (2, low, c as u32),
            });
        }
        order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut strip_comp = vec![false; comp_tris.len()];
        for o in &order {
            strip_comp[o.2 as usize] = o.0 == 0;
        }
        let mut done_comp = vec![false; comp_tris.len()];
        let mut cg_iters = 0u32;
        for &(_, _, c) in &order {
            let tris = &comp_tris[c as usize];
            // A jumped part: its landing triangle re-placed from the source vertex's relaxed position and turn.
            if let Some(&(src, lt)) = self.jump_log.iter().find(|&&(_, lt)| wedges.comp.get(lt as usize).copied() == Some(c)) {
                let (pv_new, turn) = self.relaxed_pose(src, &laid, &wedges, &x, &done_comp, &rest);
                let pv_old = self.p2[src as usize];
                for &v in &patch.tv[lt as usize] {
                    let w = wedge_at(lt, v);
                    if w != NONE {
                        let d = self.p2[v as usize] - pv_old;
                        x[w as usize] = pv_new + DVec2::new(turn.x * d.x - turn.y * d.y, turn.y * d.x + turn.x * d.y);
                        pinned[w as usize] = true;
                    }
                }
            }
            // At least one triangle's worth of pins: else the component's lowest-g triangle keeps its seed corners.
            let mut wedge_list: Vec<u32> = tris.iter().flat_map(|&t| (0..3).map(move |k| t as usize * 3 + k)).map(|i| wedges.corner[i]).collect();
            wedge_list.sort_unstable();
            wedge_list.dedup();
            if wedge_list.iter().filter(|&&w| pinned[w as usize]).count() < 3
                && let Some(&t) = tris.iter().min_by(|&&a, &&b| tri_g(a).total_cmp(&tri_g(b)).then(a.cmp(&b)))
            {
                for k in 0..3 {
                    let w = wedges.corner[t as usize * 3 + k];
                    pinned[w as usize] = true;
                    x[w as usize] = self.p2[patch.tv[t as usize][k] as usize];
                }
            }
            // Round 9 (T4): a strip component starts from its Fermi layout (each free wedge at its vertex's spine
            // foot and signed geodesic distance), not from the seed (LSCM from the seed was folded next to the pinned
            // strip, D4a). Other components (jumped parts, parts without a strip) keep the seed start.
            if strip_comp[c as usize] && !knob("WTE_TEST_SEED_START", false) {
                for &w in &wedge_list {
                    if !pinned[w as usize] {
                        let f = self.fermi(wedges.vertex[w as usize]);
                        if f.is_finite() {
                            x[w as usize] = f;
                        }
                    }
                }
            }
            // The component's system over its own wedges.
            let local: std::collections::HashMap<u32, u32> = wedge_list.iter().enumerate().map(|(i, &w)| (w, i as u32)).collect();
            let mut sys_tris = Vec::new();
            let mut sys_rest = Vec::new();
            for &t in tris {
                if let Some(r) = rest[t as usize] {
                    sys_tris.push(std::array::from_fn::<u32, 3, _>(|k| local[&wedges.corner[t as usize * 3 + k]]));
                    sys_rest.push(r);
                }
            }
            let mut xs: Vec<DVec2> = wedge_list.iter().map(|&w| x[w as usize]).collect();
            let pins: Vec<bool> = wedge_list.iter().map(|&w| pinned[w as usize]).collect();
            let mut budget = RELAX_CG_BUDGET;
            cg_iters += relax_component_from(&sys_tris, &sys_rest, &pins, &mut xs, &mut budget, false);
            for (i, &w) in wedge_list.iter().enumerate() {
                if xs[i].is_finite() {
                    x[w as usize] = xs[i];
                }
            }
            done_comp[c as usize] = true;
        }
        // The stored triangles: laid ones from their wedges, the others (halo, degenerate) from the relaxed corners.
        let rep = self.representatives(&laid, &wedges);
        // A missing corner gets one position for every halo triangle round it (the first placed): no wedge without
        // surface opens between halo triangles (round 6 check:B's rule, on the relaxed layout).
        // Hinged placements (two final corners: the triangle unfolded about its edge) take precedence over the frame
        // rule (one final corner), which projects onto the corner's plane.
        let mut placed: Vec<Option<([DVec2; 3], bool)>> =
            seed.iter().map(|(t, _, _)| if laid[*t as usize] { None } else { self.halo_from_relaxed(*t, &rep, &x, &wedges, &laid) }).collect();
        let mut halo_at: Vec<Option<DVec2>> = vec![None; patch.pos.len()];
        for pass in [true, false] {
            for (i, (t, _, _)) in seed.iter().enumerate() {
                let Some((q, hinged)) = placed[i].as_mut() else { continue };
                if *hinged != pass {
                    continue;
                }
                for (k, &v) in patch.tv[*t as usize].iter().enumerate() {
                    if self.state[v as usize] == State::Final && rep[v as usize] != NONE {
                        continue;
                    }
                    match halo_at[v as usize] {
                        Some(p) => q[k] = p,
                        None => halo_at[v as usize] = Some(q[k]),
                    }
                }
            }
        }
        let mut tris = Vec::with_capacity(seed.len());
        let mut flipped = 0u32;
        for (i, (t, q_seed, _)) in seed.iter().enumerate() {
            let t = *t;
            let q: [DVec2; 3] = if laid[t as usize] {
                std::array::from_fn(|k| x[wedges.corner[t as usize * 3 + k] as usize])
            } else {
                placed[i].map_or(*q_seed, |(q, _)| q)
            };
            if !q.iter().all(|p| p.is_finite()) {
                continue;
            }
            if laid[t as usize] && rest[t as usize].is_some() && meets(&q, rect) {
                let area = cross2(q[1] - q[0], q[2] - q[0]);
                if area * (self.orient[t as usize] as f64) <= 0.0 {
                    flipped += 1;
                }
            }
            tris.push((t, q, self.corner_g(t)));
        }
        // Seam statistics inside the picture.
        let mut seams_by_rule = [0u32; 5];
        let mut seam_count = 0u32;
        let mut cut_tri = vec![false; ntri];
        for he in 0..ntri * 3 {
            if !laid[he / 3] || seams.cut[he] == 0 {
                continue;
            }
            let (t, k) = (he / 3, he % 3);
            let [a, b] = [(k + 1) % 3, (k + 2) % 3].map(|i| x[wedges.corner[t * 3 + i] as usize]);
            if !meets(&[a, b], rect) {
                continue;
            }
            cut_tri[t] = true;
            if first_laid(patch, &laid, he) {
                seam_count += 1;
                for (r, slot) in seams_by_rule.iter_mut().enumerate() {
                    if seams.cut[he] & (1 << r) != 0 {
                        *slot += 1;
                    }
                }
            }
        }
        let closure: Vec<bool> = (0..ntri).map(|t| laid[t] && (0..3).any(|k| seams.cut[t * 3 + k] & (S1 | S2) != 0)).collect();
        Relaxed {
            closure,
            tris,
            seams: seam_count,
            seams_by_rule,
            cut_tris: cut_tri.iter().filter(|&&c| c).count() as u32,
            flipped,
            cg_iters,
        }
    }

    /// Round 9 (T4): the Fermi position of vertex `v`: its continuous spine foot along the spine and its geodesic
    /// distance across it on its side, `e foot_c + rot90(e) side sqrt(g^2 - out^2)` (`out`: how far the foot lies
    /// beyond the spine's along range; a vertex beyond an end keeps its distance from that end).
    fn fermi(&self, v: u32) -> DVec2 {
        let v = v as usize;
        let (e, f) = (self.spine.e, self.foot_c[v]);
        let out = (self.spine.lo - f).max(f - self.spine.hi).max(0.0);
        let across = (self.key[v] * self.key[v] - out * out).max(0.0).sqrt();
        e * f + DVec2::new(-e.y, e.x) * (self.side[v] as f64 * across)
    }

    /// Position and turn (unit complex number) of vertex `v` after the relaxation: its representative wedge's
    /// position and the rotation between the seed and the relaxed layout of its representative triangle (identity when
    /// its component is not relaxed yet).
    fn relaxed_pose(&self, v: u32, laid: &[bool], wedges: &Wedges, x: &[DVec2], done: &[bool], rest: &[Option<(f64, [DVec2; 3])>]) -> (DVec2, DVec2) {
        let patch = self.patch;
        let tri = patch.vtris(v).iter().copied().find(|&t| laid[t as usize] && rest[t as usize].is_some());
        let Some(t) = tri else { return (self.p2[v as usize], DVec2::X) };
        let comp = wedges.comp[t as usize];
        if comp == NONE || !done[comp as usize] {
            return (self.p2[v as usize], DVec2::X);
        }
        let k = corner_of(patch, t, v);
        let pos = x[wedges.corner[t as usize * 3 + k] as usize];
        let g = rest[t as usize].map(|r| r.1).unwrap_or_default();
        let angle = |q: [DVec2; 3]| {
            let jx = g[0] * q[0].x + g[1] * q[1].x + g[2] * q[2].x;
            let jy = g[0] * q[0].y + g[1] * q[1].y + g[2] * q[2].y;
            (jy.x - jx.y).atan2(jx.x + jy.y)
        };
        let c = patch.tv[t as usize];
        let relaxed: [DVec2; 3] = std::array::from_fn(|i| x[wedges.corner[t as usize * 3 + i] as usize]);
        let seeded = c.map(|u| self.p2[u as usize]);
        let turn = angle(relaxed) - angle(seeded);
        let rot = DVec2::new(turn.cos(), turn.sin());
        (pos, if rot.is_finite() { rot } else { DVec2::X })
    }

    /// Per vertex: the wedge of its first laid triangle (ascending local triangle), `NONE` without one.
    fn representatives(&self, laid: &[bool], wedges: &Wedges) -> Vec<u32> {
        let patch = self.patch;
        (0..patch.pos.len() as u32)
            .map(|v| {
                patch.vtris(v).iter().copied().find(|&t| laid[t as usize]).map_or(NONE, |t| wedges.corner[t as usize * 3 + corner_of(patch, t, v)])
            })
            .collect()
    }

    /// Corners of a triangle that is not laid (halo, degenerate) from the relaxed positions of its final corners: the
    /// missing corners by the hinge (two final corners) or by the relaxed frame of the final corner (one).
    /// Also says whether the triangle was hinged (or complete) rather than placed by a frame.
    fn halo_from_relaxed(&mut self, t: u32, rep: &[u32], x: &[DVec2], wedges: &Wedges, laid: &[bool]) -> Option<([DVec2; 3], bool)> {
        let patch = self.patch;
        let c = patch.tv[t as usize];
        let known: [Option<DVec2>; 3] = c.map(|v| (self.state[v as usize] == State::Final && rep[v as usize] != NONE).then(|| x[rep[v as usize] as usize]));
        let count = known.iter().filter(|k| k.is_some()).count();
        match count {
            3 => Some((known.map(|k| k.unwrap_or_default()), true)),
            2 if !patch.degenerate(t) => {
                let w = (0..3).find(|&i| known[i].is_none())?;
                let (i1, i2) = ((w + 1) % 3, (w + 2) % 3);
                let (v, a) = if self.key[c[i1] as usize] <= self.key[c[i2] as usize] { (i1, i2) } else { (i2, i1) };
                let (pv, pa) = (known[v]?, known[a]?);
                let pw = self.hinge(t, c[v], c[a], c[w], pv, pa)?;
                let mut q = [DVec2::ZERO; 3];
                q[v] = pv;
                q[a] = pa;
                q[w] = pw;
                Some((q, true))
            }
            1..=2 => {
                // The known corner nearest the spine and its relaxed frame.
                let r = (0..3).filter(|&i| known[i].is_some()).min_by(|&i, &j| self.key[c[i] as usize].total_cmp(&self.key[c[j] as usize]))?;
                let v = c[r];
                let rt = patch.vtris(v).iter().copied().find(|&u| laid[u as usize])?;
                let rq: [DVec2; 3] = std::array::from_fn(|i| x[wedges.corner[rt as usize * 3 + i] as usize]);
                let frame = Frame::of_triangle(patch.corners(rt), rq).unwrap_or(self.frame[v as usize]);
                let (pr, qr) = (known[r]?, patch.pos[v as usize]);
                Some((std::array::from_fn(|i| if i == r { pr } else { known[i].unwrap_or_else(|| pr + frame.apply(patch.pos[c[i] as usize] - qr)) }), false))
            }
            _ => None,
        }
    }

    /// Round 9 (T3): the vertices reachable from `v` over edges of laid triangles through vertices within `radius`
    /// (3D distance) of `v`, `v` included, ascending. `stamp` marks visited vertices with `mark`.
    fn laid_within(&self, v: u32, radius: f64, laid: &[bool], stamp: &mut [u32], mark: u32) -> Vec<u32> {
        let patch = self.patch;
        let origin = patch.pos[v as usize];
        let mut out = vec![v];
        stamp[v as usize] = mark;
        let mut i = 0;
        while i < out.len() {
            let x = out[i];
            i += 1;
            for &t in patch.vtris(x) {
                if !laid[t as usize] {
                    continue;
                }
                for &y in &patch.tv[t as usize] {
                    if stamp[y as usize] != mark && patch.pos[y as usize].distance(origin) <= radius {
                        stamp[y as usize] = mark;
                        out.push(y);
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// S1-S5 over the laid triangles (`laid`).
    fn seams(&self, laid: &[bool], period: Option<f64>, anchor: u32, cone_radius: f64) -> Seams {
        let patch = self.patch;
        let ntri = patch.tv.len();
        let nv = patch.pos.len();
        let mut cut = vec![0u8; ntri * 3];
        let mark = |cut: &mut Vec<u8>, he: usize, bits: u8| {
            let (start, len) = patch.he_group[he];
            for &h in &patch.edge_tris[start as usize..(start + len) as usize] {
                cut[h as usize] |= bits;
            }
        };
        // Vertices whose fan is open (a triangle round it not laid, or a laid-set boundary / non-manifold edge).
        let mut open = vec![false; nv];
        for v in 0..nv as u32 {
            if patch.vtris(v).iter().any(|&t| !laid[t as usize]) {
                open[v as usize] = true;
            }
        }
        // S1, S2, S3.
        for he in 0..ntri * 3 {
            if !laid[he / 3] || !first_laid(patch, laid, he) {
                continue;
            }
            let members: Vec<u32> = laid_members(patch, laid, he).collect();
            let [a, b] = edge_of(patch.tv[he / 3], he % 3);
            if members.len() == 1 {
                open[a as usize] = true;
                open[b as usize] = true;
                continue;
            }
            if members.len() > 2 {
                mark(&mut cut, he, S3);
                open[a as usize] = true;
                open[b as usize] = true;
                continue;
            }
            let (h1, h2) = (members[0] as usize, members[1] as usize);
            let (o1, o2) = (self.orient[h1 / 3], self.orient[h2 / 3]);
            let [a2, _] = edge_of(patch.tv[h2 / 3], h2 % 3);
            let opposite = a2 == b;
            // check:A (round 8): a sliver's orientation comes from a normal that rounding can turn over; it is no
            // evidence of a Moebius flip (check_a8: 1e-6 slivers gave 4 S3 seam edges inside the picture).
            let slivers = sliver(patch.corners((h1 / 3) as u32)) || sliver(patch.corners((h2 / 3) as u32));
            if o1 != 0 && o2 != 0 && !slivers && ((opposite && o1 != o2) || (!opposite && o1 == o2)) {
                mark(&mut cut, he, S3);
            }
            let bits = self.closure_rules(a, b, period);
            if bits != 0 {
                mark(&mut cut, he, bits);
            }
        }
        let mut seam_vertex = vec![false; nv];
        for he in 0..ntri * 3 {
            if laid[he / 3] && cut[he] != 0 {
                for v in edge_of(patch.tv[he / 3], he % 3) {
                    seam_vertex[v as usize] = true;
                }
            }
        }
        // S4: cones, each with one cut along the steepest ascent of g to the laid set's boundary (or a seam).
        let mut cone = vec![false; nv];
        let limit = CONE_CUT_DEG.to_radians();
        let mut deficit = vec![0.0f64; nv];
        for v in 0..nv as u32 {
            if open[v as usize] || patch.vtris(v).len() < 3 {
                continue;
            }
            let mut sum = 0.0;
            for &t in patch.vtris(v) {
                let c = patch.tv[t as usize];
                let k = corner_of(patch, t, v);
                let (p, a, b) = (patch.pos[c[k] as usize], patch.pos[c[(k + 1) % 3] as usize], patch.pos[c[(k + 2) % 3] as usize]);
                let (u, w) = (a - p, b - p);
                sum += u.cross(w).length().atan2(u.dot(w));
            }
            deficit[v as usize] = std::f64::consts::TAU - sum;
        }
        // Round 9 (T3): a cone is a net curvature, not a vertex. A vertex whose |deficit| exceeds the limit cuts only
        // when the deficits of the laid interior vertices within `cone_radius` of it (3D distance, walked over laid
        // edges) also sum to more than the limit in absolute value, and only the strongest of such vertices within
        // one radius cuts (largest |deficit|, then lowest position rank). A cube corner, a cone tip still cut
        // (binding Q3); a rivet, a bevel, a groove do not.
        let strong: Vec<u32> = (0..nv as u32).filter(|&v| deficit[v as usize].abs() > limit).collect();
        if !strong.is_empty() {
            let mut stamp = vec![u32::MAX; nv];
            let mut near: Vec<Vec<u32>> = Vec::with_capacity(strong.len());
            let mut net_ok = vec![false; nv];
            for (round, &v) in strong.iter().enumerate() {
                // check:A (round 9): the net sums over at least the vertex's 1-ring (its longest laid edge): on a
                // coarse mesh a rivet's ring lies farther than the radius from its tip, and the tip alone cut the
                // picture as in round 8 (check_a9: a rivet next to a box corner, two S4 cuts, a cell 98 % lost). The
                // strongest-only rule keeps the radius (a coarse cube's corners each cut).
                let ring = patch
                    .vtris(v)
                    .iter()
                    .filter(|&&t| laid[t as usize])
                    .flat_map(|&t| patch.tv[t as usize])
                    .map(|u| patch.pos[u as usize].distance(patch.pos[v as usize]))
                    .fold(0.0f64, f64::max);
                // (A radius of 0, the test knob for round 8's rule, stays 0.)
                let radius = if cone_radius > 0.0 { cone_radius.max(ring * (1.0 + 1e-9)) } else { 0.0 };
                let found = self.laid_within(v, radius, laid, &mut stamp, round as u32);
                let net: f64 = found.iter().map(|&u| deficit[u as usize]).sum();
                net_ok[v as usize] = net.abs() > limit;
                let origin = patch.pos[v as usize];
                near.push(found.into_iter().filter(|&u| patch.pos[u as usize].distance(origin) <= cone_radius).collect());
            }
            let beats = |u: u32, v: u32| {
                let (du, dv) = (deficit[u as usize].abs(), deficit[v as usize].abs());
                du > dv || (du == dv && patch.rank[u as usize] < patch.rank[v as usize])
            };
            for (k, &v) in strong.iter().enumerate() {
                if net_ok[v as usize] && !near[k].iter().any(|&u| u != v && net_ok[u as usize] && beats(u, v)) {
                    cone[v as usize] = true;
                }
            }
        }
        for v in 0..nv as u32 {
            if !cone[v as usize] {
                continue;
            }
            let mut x = v;
            for _ in 0..nv {
                if open[x as usize] || (x != v && seam_vertex[x as usize]) {
                    break;
                }
                // The laid neighbour of steepest ascent over an uncut edge.
                let mut best: Option<(f64, u32, usize)> = None;
                for &t in patch.vtris(x) {
                    if !laid[t as usize] {
                        continue;
                    }
                    let c = patch.tv[t as usize];
                    for k in 0..3 {
                        let he = t as usize * 3 + k;
                        let [a, b] = edge_of(c, k);
                        let y = if a == x {
                            b
                        } else if b == x {
                            a
                        } else {
                            continue;
                        };
                        if cut[he] != 0 {
                            continue;
                        }
                        let (gx, gy) = (self.key[x as usize], self.key[y as usize]);
                        let len = patch.pos[x as usize].distance(patch.pos[y as usize]);
                        if !(gy > gx && len > 0.0) {
                            continue;
                        }
                        let slope = (gy - gx) / len;
                        if best.is_none_or(|(s, by, _)| slope > s || (slope == s && patch.rank[y as usize] < patch.rank[by as usize])) {
                            best = Some((slope, y, he));
                        }
                    }
                }
                let Some((_, y, he)) = best else { break };
                mark(&mut cut, he, S4);
                seam_vertex[x as usize] = true;
                let reached = seam_vertex[y as usize];
                seam_vertex[y as usize] = true;
                x = y;
                if reached {
                    break;
                }
            }
        }
        // S5: tree-cotree. A dual spanning tree grown in g order from the anchor triangle over uncut edges; the other
        // edges form the cut graph with the boundary and the seams; leaves are pruned (unless a cone); what is left
        // apart from the boundary and the seams is cut.
        // check:A (round 8): zero-area triangles whose corners are laid fill their slit for this rule (a T-junction
        // closed by a degenerate triangle is no hole: without this every such slit was cut to the boundary, check_a8's
        // slivered plate had 179 S5 seam edges inside the picture). Their own edges join the dual graph only where an
        // edge has fewer than two laid triangles.
        let fill: Vec<bool> = (0..ntri)
            .map(|t| !laid[t] && patch.degenerate(t as u32) && patch.tv[t].iter().all(|&x| self.state[x as usize] == State::Final))
            .collect();
        let node = |t: usize| laid[t] || fill[t];
        let topo_members = |he: usize| -> Vec<u32> {
            let (start, len) = patch.he_group[he];
            let all = &patch.edge_tris[start as usize..(start + len) as usize];
            let mut m: Vec<u32> = all.iter().copied().filter(|&h| laid[(h / 3) as usize]).collect();
            if m.len() < 2 {
                m.extend(all.iter().copied().filter(|&h| fill[(h / 3) as usize]));
            }
            m
        };
        // check:A (round 8): small holes of the laid set are filled for this rule by a virtual face: a crack or a
        // T-junction whose zero-area filler the import dropped, an unlaid vertex on a bump's top beyond the front's
        // stop. A sticker covers such a hole without a cut; cutting it to the boundary tore the picture (check_a8's
        // slivered plate: 179 S5 seam edges inside the picture). A hole is small when its boundary loop is at most
        // `HOLE_FILL` times the longest edge at its vertices; the longest loop (the laid set's rim) is never filled.
        let boundary: Vec<usize> = (0..ntri * 3).filter(|&he| node(he / 3) && topo_members(he).len() == 1).collect();
        let mut loops: Vec<u32> = (0..nv as u32).collect();
        for &he in &boundary {
            let [a, b] = edge_of(patch.tv[he / 3], he % 3);
            uf_join(&mut loops, a, b);
        }
        // Per loop root: length, longest edge at its vertices.
        let mut loop_len = vec![0.0f64; nv];
        let mut loop_edge = vec![0.0f64; nv];
        for &he in &boundary {
            let [a, b] = edge_of(patch.tv[he / 3], he % 3);
            let r = uf_find(&mut loops, a) as usize;
            loop_len[r] += patch.pos[a as usize].distance(patch.pos[b as usize]);
            loop_edge[r] = loop_edge[r].max(patch.max_edge[a as usize]).max(patch.max_edge[b as usize]);
        }
        let rim = (0..nv).filter(|&r| loop_len[r] > 0.0).max_by(|&a, &b| loop_len[a].total_cmp(&loop_len[b]).then(b.cmp(&a)));
        let mut hole_of_root = vec![NONE; nv];
        let mut hole_edges: Vec<Vec<usize>> = Vec::new();
        let mut hole_of = vec![NONE; ntri * 3];
        for &he in &boundary {
            let [a, _] = edge_of(patch.tv[he / 3], he % 3);
            let r = uf_find(&mut loops, a) as usize;
            if Some(r) == rim || loop_len[r] > HOLE_FILL * loop_edge[r] {
                continue;
            }
            if hole_of_root[r] == NONE {
                hole_of_root[r] = hole_edges.len() as u32;
                hole_edges.push(Vec::new());
            }
            hole_of[he] = hole_of_root[r];
            hole_edges[hole_of_root[r] as usize].push(he);
        }
        let tri_g = |t: u32| {
            if (t as usize) < ntri {
                patch.tv[t as usize].iter().map(|&v| self.key[v as usize]).fold(f64::NEG_INFINITY, f64::max)
            } else {
                hole_edges[t as usize - ntri].iter().flat_map(|&he| edge_of(patch.tv[he / 3], he % 3)).map(|v| self.key[v as usize]).fold(f64::NEG_INFINITY, f64::max)
            }
        };
        let mut visited = vec![false; ntri + hole_edges.len()];
        let mut tree = vec![false; ntri * 3];
        let mut starts: Vec<u32> = (0..ntri as u32).filter(|&t| node(t as usize)).collect();
        starts.sort_by(|&a, &b| tri_g(a).total_cmp(&tri_g(b)).then(a.cmp(&b)));
        if (anchor as usize) < ntri && laid[anchor as usize] {
            starts.insert(0, anchor);
        }
        #[derive(PartialEq)]
        struct Item(f64, u32);
        impl Eq for Item {}
        impl PartialOrd for Item {
            fn partial_cmp(&self, o: &Item) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Item {
            fn cmp(&self, o: &Item) -> Ordering {
                o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
            }
        }
        let mut heap = BinaryHeap::new();
        for s in starts {
            if visited[s as usize] {
                continue;
            }
            visited[s as usize] = true;
            heap.push(Item(tri_g(s), s));
            while let Some(Item(_, t)) = heap.pop() {
                if t as usize >= ntri {
                    // A filled hole: its neighbours across its loop's edges.
                    for &he in &hole_edges[t as usize - ntri] {
                        let u = (he / 3) as u32;
                        if cut[he] == 0 && !visited[u as usize] {
                            visited[u as usize] = true;
                            tree[he] = true;
                            heap.push(Item(tri_g(u), u));
                        }
                    }
                    continue;
                }
                for k in 0..3 {
                    let he = t as usize * 3 + k;
                    if cut[he] != 0 {
                        continue;
                    }
                    if hole_of[he] != NONE {
                        let u = ntri as u32 + hole_of[he];
                        if !visited[u as usize] {
                            visited[u as usize] = true;
                            tree[he] = true;
                            heap.push(Item(tri_g(u), u));
                        }
                        continue;
                    }
                    let members = topo_members(he);
                    if members.len() != 2 || !members.contains(&(he as u32)) {
                        continue;
                    }
                    let other = if members[0] as usize == he { members[1] } else { members[0] };
                    let u = other / 3;
                    if !visited[u as usize] {
                        visited[u as usize] = true;
                        tree[he] = true;
                        tree[other as usize] = true;
                        heap.push(Item(tri_g(u), u));
                    }
                }
            }
        }
        // The cut graph: (half-edge, ends, fixed).
        let mut edges: Vec<(usize, u32, u32, bool)> = Vec::new();
        for he in 0..ntri * 3 {
            if !node(he / 3) || tree[he] {
                continue;
            }
            let members = topo_members(he);
            if members.first() != Some(&(he as u32)) {
                continue;
            }
            let [a, b] = edge_of(patch.tv[he / 3], he % 3);
            let rim_or_fin = members.len() != 2 && hole_of[he] == NONE;
            edges.push((he, a, b, rim_or_fin || cut[he] != 0));
        }
        let mut degree = vec![0u32; nv];
        let mut incident: Vec<Vec<u32>> = vec![Vec::new(); nv];
        for (i, e) in edges.iter().enumerate() {
            degree[e.1 as usize] += 1;
            degree[e.2 as usize] += 1;
            incident[e.1 as usize].push(i as u32);
            incident[e.2 as usize].push(i as u32);
        }
        let mut alive = vec![true; edges.len()];
        let mut queue: std::collections::VecDeque<u32> = (0..nv as u32).filter(|&v| degree[v as usize] == 1).collect();
        while let Some(v) = queue.pop_front() {
            if degree[v as usize] != 1 || cone[v as usize] {
                continue;
            }
            let Some(&e) = incident[v as usize].iter().find(|&&e| alive[e as usize]) else { continue };
            let (_, a, b, fixed) = edges[e as usize];
            if fixed {
                continue;
            }
            alive[e as usize] = false;
            degree[a as usize] -= 1;
            degree[b as usize] -= 1;
            let other = if a == v { b } else { a };
            if degree[other as usize] == 1 {
                queue.push_back(other);
            }
        }
        for (i, &(he, _, _, fixed)) in edges.iter().enumerate() {
            if alive[i] && !fixed {
                mark(&mut cut, he, S5);
            }
        }
        Seams { cut }
    }
}

/// True when vertex `v` of laid triangle `t` sits in a pinned wedge.
fn pinned_vertex(wedges: &Wedges, pinned: &[bool], t: u32, v: u32, patch: &Patch) -> bool {
    let w = wedges.corner[t as usize * 3 + corner_of(patch, t, v)];
    w != NONE && pinned[w as usize]
}

/// Wedges (union of the corners round each vertex over uncut manifold edges) and components (triangles joined over
/// the same edges) of the laid triangles.
fn wedges(patch: &Patch, laid: &[bool], seams: &Seams) -> Wedges {
    let ntri = patch.tv.len();
    fn find(uf: &mut [u32], mut x: u32) -> u32 {
        while uf[x as usize] != x {
            uf[x as usize] = uf[uf[x as usize] as usize];
            x = uf[x as usize];
        }
        x
    }
    fn join(uf: &mut [u32], a: u32, b: u32) {
        let (ra, rb) = (find(uf, a), find(uf, b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            uf[hi as usize] = lo;
        }
    }
    let mut corners: Vec<u32> = (0..ntri as u32 * 3).collect();
    let mut tris: Vec<u32> = (0..ntri as u32).collect();
    for he in 0..ntri * 3 {
        if !laid[he / 3] || seams.cut[he] != 0 || !first_laid(patch, laid, he) {
            continue;
        }
        let members: Vec<u32> = laid_members(patch, laid, he).collect();
        if members.len() != 2 {
            continue;
        }
        let (t1, t2) = (members[0] / 3, members[1] / 3);
        let [a, b] = edge_of(patch.tv[t1 as usize], (members[0] % 3) as usize);
        for v in [a, b] {
            join(&mut corners, t1 * 3 + corner_of(patch, t1, v) as u32, t2 * 3 + corner_of(patch, t2, v) as u32);
        }
        join(&mut tris, t1, t2);
    }
    let mut corner = vec![NONE; ntri * 3];
    let mut vertex = Vec::new();
    let mut id_of = vec![NONE; ntri * 3];
    for i in 0..ntri * 3 {
        if !laid[i / 3] {
            continue;
        }
        let r = find(&mut corners, i as u32) as usize;
        if id_of[r] == NONE {
            id_of[r] = vertex.len() as u32;
            vertex.push(patch.tv[i / 3][i % 3]);
        }
        corner[i] = id_of[r];
    }
    let mut comp = vec![NONE; ntri];
    let mut comp_id = vec![NONE; ntri];
    let mut comps = 0u32;
    for t in 0..ntri {
        if !laid[t] {
            continue;
        }
        let r = find(&mut tris, t as u32) as usize;
        if comp_id[r] == NONE {
            comp_id[r] = comps;
            comps += 1;
        }
        comp[t] = comp_id[r];
    }
    Wedges { corner, vertex, comp, comps }
}

// ----- the solver -----

/// One component's energy terms: per triangle its unknowns, area and gradient basis `g_k` (the Jacobian of the map
/// from the triangle's development to the layout is `J = sum_k x_k g_k^T`).
struct System<'a> {
    tris: &'a [[u32; 3]],
    rest: &'a [(f64, [DVec2; 3])],
    /// Unknown -> its (triangle, corner) incidences, ascending (CSR).
    start: Vec<u32>,
    incid: Vec<u32>,
    diag: Vec<f64>,
    pinned: Vec<bool>,
    par: bool,
}

/// A sum over `0..n` added in fixed chunks of `PAR_CHUNK` (chunk sums in chunk order): the same bits with 1 or N
/// threads.
fn chunked_sum(n: usize, par: bool, f: impl Fn(usize) -> f64 + Sync) -> f64 {
    let chunks = n.div_ceil(PAR_CHUNK);
    let part = |c: usize| {
        let mut s = 0.0;
        for i in c * PAR_CHUNK..((c + 1) * PAR_CHUNK).min(n) {
            s += f(i);
        }
        s
    };
    let parts: Vec<f64> = if par { (0..chunks).into_par_iter().map(part).collect() } else { (0..chunks).map(part).collect() };
    parts.iter().sum()
}

impl<'a> System<'a> {
    fn new(n: usize, tris: &'a [[u32; 3]], rest: &'a [(f64, [DVec2; 3])], pinned: &[bool]) -> System<'a> {
        let mut start = vec![0u32; n + 1];
        for t in tris {
            for &w in t {
                start[w as usize + 1] += 1;
            }
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut incid = vec![0u32; start[n] as usize];
        let mut diag = vec![0.0f64; n];
        for (t, c) in tris.iter().enumerate() {
            let (area, g) = rest[t];
            for k in 0..3 {
                let w = c[k] as usize;
                incid[fill[w] as usize] = (t * 3 + k) as u32;
                fill[w] += 1;
                diag[w] += area * g[k].length_squared();
            }
        }
        // An unknown without energy keeps its position.
        let pinned = (0..n).map(|i| pinned[i] || !(diag[i] > 0.0 && diag[i].is_finite())).collect();
        System { tris, rest, start, incid, diag, pinned, par: n >= par_min() }
    }

    /// Per-triangle terms: `out[t][k]` = the term of corner `k` of triangle `t` in `H x` (`lscm`: the conformal
    /// energy's Hessian; else the ARAP global step's) or, with `rot`, in the ARAP right-hand side.
    fn terms(&self, lscm: bool, x: &[DVec2], rot: Option<&[DVec2]>, out: &mut [[DVec2; 3]]) {
        let term = |t: usize| -> [DVec2; 3] {
            let (area, g) = self.rest[t];
            let c = self.tris[t];
            if let Some(rot) = rot {
                let r = rot[t];
                let (rx, ry) = (DVec2::new(r.x, -r.y), DVec2::new(r.y, r.x));
                return std::array::from_fn(|k| DVec2::new(rx.dot(g[k]), ry.dot(g[k])) * area);
            }
            let xs = c.map(|w| x[w as usize]);
            if lscm {
                let r1: f64 = (0..3).map(|k| xs[k].x * g[k].x - xs[k].y * g[k].y).sum();
                let r2: f64 = (0..3).map(|k| xs[k].x * g[k].y + xs[k].y * g[k].x).sum();
                std::array::from_fn(|k| DVec2::new(r1 * g[k].x + r2 * g[k].y, r2 * g[k].x - r1 * g[k].y) * area)
            } else {
                let jx = g[0] * xs[0].x + g[1] * xs[1].x + g[2] * xs[2].x;
                let jy = g[0] * xs[0].y + g[1] * xs[1].y + g[2] * xs[2].y;
                std::array::from_fn(|k| DVec2::new(jx.dot(g[k]), jy.dot(g[k])) * area)
            }
        };
        if self.par {
            out.par_chunks_mut(PAR_CHUNK).enumerate().for_each(|(c, chunk)| {
                for (i, o) in chunk.iter_mut().enumerate() {
                    *o = term(c * PAR_CHUNK + i);
                }
            });
        } else {
            for (t, o) in out.iter_mut().enumerate() {
                *o = term(t);
            }
        }
    }

    /// `y[w]` = the sum of the terms of unknown `w` in incidence order (0 for a pinned unknown).
    fn gather(&self, terms: &[[DVec2; 3]], y: &mut [DVec2]) {
        let one = |w: usize| -> DVec2 {
            if self.pinned[w] {
                return DVec2::ZERO;
            }
            let mut s = DVec2::ZERO;
            for &i in &self.incid[self.start[w] as usize..self.start[w + 1] as usize] {
                s += terms[(i / 3) as usize][(i % 3) as usize];
            }
            s
        };
        if self.par {
            y.par_chunks_mut(PAR_CHUNK).enumerate().for_each(|(c, chunk)| {
                for (i, o) in chunk.iter_mut().enumerate() {
                    *o = one(c * PAR_CHUNK + i);
                }
            });
        } else {
            for (w, o) in y.iter_mut().enumerate() {
                *o = one(w);
            }
        }
    }

    fn dot(&self, a: &[DVec2], b: &[DVec2]) -> f64 {
        chunked_sum(a.len(), self.par, |i| a[i].dot(b[i]))
    }

    /// Preconditioned conjugate gradients on the free unknowns of `H x = b` (`lscm` picks `H`), warm-started from `x`;
    /// stops at a residual of `tol` times the right-hand side's (with the pins), after `cap` iterations or when the
    /// budget is spent. Returns the iterations.
    fn cg(&self, lscm: bool, b: &[DVec2], x: &mut [DVec2], tol: f64, cap: u32, budget: &mut u32) -> u32 {
        let n = x.len();
        let mut terms = vec![[DVec2::ZERO; 3]; self.tris.len()];
        let mut hx = vec![DVec2::ZERO; n];
        // The reference: the right-hand side with the pins moved over (the free unknowns at 0).
        let pins_only: Vec<DVec2> = (0..n).map(|i| if self.pinned[i] { x[i] } else { DVec2::ZERO }).collect();
        self.terms(lscm, &pins_only, None, &mut terms);
        self.gather(&terms, &mut hx);
        let free_b: Vec<DVec2> = (0..n).map(|i| if self.pinned[i] { DVec2::ZERO } else { b[i] - hx[i] }).collect();
        let reference = self.dot(&free_b, &free_b).sqrt();
        self.terms(lscm, x, None, &mut terms);
        self.gather(&terms, &mut hx);
        let mut r: Vec<DVec2> = (0..n).map(|i| if self.pinned[i] { DVec2::ZERO } else { b[i] - hx[i] }).collect();
        let precondition = |r: &[DVec2]| -> Vec<DVec2> { (0..n).map(|i| if self.pinned[i] { DVec2::ZERO } else { r[i] / self.diag[i] }).collect() };
        let mut z = precondition(&r);
        let mut p = z.clone();
        let mut rz = self.dot(&r, &z);
        let mut q = vec![DVec2::ZERO; n];
        let mut iters = 0;
        while iters < cap && *budget > 0 {
            let rr = self.dot(&r, &r).sqrt();
            if rr <= tol * reference || rr.is_nan() || !rr.is_finite() {
                break;
            }
            self.terms(lscm, &p, None, &mut terms);
            self.gather(&terms, &mut q);
            let pq = self.dot(&p, &q);
            if !(pq > 0.0 && pq.is_finite()) {
                break;
            }
            let alpha = rz / pq;
            for ((xi, ri), (pi, qi)) in x.iter_mut().zip(r.iter_mut()).zip(p.iter().zip(&q)) {
                *xi += *pi * alpha;
                *ri -= *qi * alpha;
            }
            iters += 1;
            *budget -= 1;
            z = precondition(&r);
            let rz_new = self.dot(&r, &z);
            let beta = rz_new / rz;
            if !beta.is_finite() {
                break;
            }
            for (pi, zi) in p.iter_mut().zip(&z) {
                *pi = *zi + *pi * beta;
            }
            rz = rz_new;
        }
        iters
    }
}

/// Test builds: a knob from the environment (experiments), else `default`.
#[cfg(test)]
pub(super) fn knob<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
#[cfg(not(test))]
pub(super) fn knob<T>(_: &str, default: T) -> T {
    default
}

/// Relaxes one component: LSCM from the warm start `x`, then ARAP iterations (round 8's solver; the solver tests of
/// check:A run it from a crumpled start).
#[cfg(test)]
fn relax_component(tris: &[[u32; 3]], rest: &[(f64, [DVec2; 3])], pins: &[bool], x: &mut [DVec2], budget: &mut u32) -> u32 {
    relax_component_from(tris, rest, pins, x, budget, true)
}

/// Relaxes one component: with `lscm` LSCM from the warm start `x` first, then ARAP iterations (3.1; round 9, T4: the
/// layout passes its Fermi start and no LSCM). `tris` index into `x`; `pins` marks fixed unknowns. Returns the CG
/// iterations spent.
fn relax_component_from(tris: &[[u32; 3]], rest: &[(f64, [DVec2; 3])], pins: &[bool], x: &mut [DVec2], budget: &mut u32, lscm: bool) -> u32 {
    let arap_iters: u32 = knob("WTE_TEST_ARAP_ITERS", RELAX_ARAP_ITERS);
    let arap_stop: f64 = knob("WTE_TEST_ARAP_STOP", ARAP_STOP);
    let debug: bool = knob("WTE_TEST_RELAX_DEBUG", false);
    *budget = knob("WTE_TEST_CG_BUDGET", *budget);
    if tris.is_empty() {
        return 0;
    }
    let sys = System::new(x.len(), tris, rest, pins);
    if sys.pinned.iter().all(|&p| p) {
        return 0;
    }
    // Round 9 (T4): no LSCM: the start is the Fermi layout (`Layout::relax`); round 8's LSCM start stays behind a
    // test knob (`WTE_TEST_LSCM=true`).
    let lscm: bool = knob("WTE_TEST_LSCM", lscm);
    let lscm_iters: u32 = knob("WTE_TEST_LSCM_ITERS", LSCM_ITERS);
    let arap_cg: u32 = knob("WTE_TEST_ARAP_CG", ARAP_ITERS);
    let zero = vec![DVec2::ZERO; x.len()];
    let mut iters = if lscm { sys.cg(true, &zero, x, LSCM_TOL, lscm_iters, budget) } else { 0 };
    if debug {
        eprintln!("relax: {} unknowns, {} triangles, {} pinned, LSCM {iters} iterations", x.len(), tris.len(), sys.pinned.iter().filter(|&&p| p).count());
    }
    let mut rot = vec![DVec2::X; tris.len()];
    let mut terms = vec![[DVec2::ZERO; 3]; tris.len()];
    let mut rhs = vec![DVec2::ZERO; x.len()];
    let mut prev = f64::INFINITY;
    for _ in 0..arap_iters {
        if *budget == 0 {
            break;
        }
        // Local step: the closest rotation per triangle; the energy of the current layout.
        let mut energy = 0.0;
        for (t, c) in tris.iter().enumerate() {
            let (area, g) = rest[t];
            let xs = c.map(|w| x[w as usize]);
            let jx = g[0] * xs[0].x + g[1] * xs[1].x + g[2] * xs[2].x;
            let jy = g[0] * xs[0].y + g[1] * xs[1].y + g[2] * xs[2].y;
            let (a, b, cc, d) = (jx.x, jx.y, jy.x, jy.y);
            let v = DVec2::new(a + d, cc - b);
            let r = if v.length() > 0.0 && v.is_finite() { v.normalize() } else { DVec2::X };
            rot[t] = r;
            energy += 0.5 * area * ((a - r.x).powi(2) + (b + r.y).powi(2) + (cc - r.y).powi(2) + (d - r.x).powi(2));
        }
        if debug {
            eprintln!("relax: ARAP energy {energy:.6e}");
        }
        if !energy.is_finite() || energy > prev * (1.0 - arap_stop) {
            break;
        }
        prev = energy;
        // Global step.
        sys.terms(false, x, Some(&rot), &mut terms);
        sys.gather(&terms, &mut rhs);
        let n = sys.cg(false, &rhs, x, ARAP_TOL, arap_cg, budget);
        if debug {
            eprintln!("relax: ARAP global {n} iterations");
        }
        iters += n;
    }
    iters
}

// ----- the owner grid (round 9, T1: the place owner) -----

/// Round 9 (T1): a fragment is the same surface place as its cell's owner when its 3D point lies within this many
/// cell diagonals (times the larger of the two stretches, `owner_scale`) of the owner's 3D point.
pub const OWNER_SAME: f32 = 3.0;
/// Round 9 (T1): the reach of the `buried` ray, times the model diagonal (`4 x GAP_JUMP`).
pub const BURY_REACH: f64 = 4.0 * super::GAP_JUMP;

/// One stored triangle as the owner grid sees it (round 9, T1).
pub(crate) struct OwnerTri {
    /// The stored (f32) layout corners.
    pub(crate) q: [DVec2; 3],
    /// Corner depths `g` (as stored).
    pub(crate) g: [f64; 3],
    /// The mesh triangle's corners (the f32 positions the bake interpolates).
    pub(crate) p3: [Vec3; 3],
    /// `owner_scale` of the triangle.
    pub(crate) s: f32,
    /// Part (`Patch::part`) and oriented unit normal (zero for a degenerate triangle): the `buried` test.
    pub(crate) part: u32,
    pub(crate) normal: DVec3,
    /// Laid (all corners final), not a halo triangle: only these count for `WrapStats::rejected`.
    pub(crate) laid: bool,
    /// check:A (round 9): laid next to a closure (S1 / S2, or a seed torn by one): `lost_cut` leaves out the picture
    /// beyond the line where the ends or the halves meet.
    pub(crate) closure: bool,
}

/// Round 9 (T1): the stretch of a stored triangle, `sqrt(3D area / layout area)` clamped to 1..4 (world units per
/// layout unit; a layout area of 0 gives 4). The bake and the grid compute it from the same f32 inputs.
pub(crate) fn owner_scale(p3: [Vec3; 3], q: [Vec2; 3]) -> f32 {
    let a3 = (p3[1] - p3[0]).cross(p3[2] - p3[0]).length();
    let a2 = (q[1] - q[0]).perp_dot(q[2] - q[0]).abs();
    let s = (a3 / a2).sqrt();
    if s.is_nan() { 4.0 } else { s.clamp(1.0, 4.0) }
}

/// The `buried` test (`wrap_param::buried_at`): `may(i)` is false when no point of stored triangle `i` can be buried
/// (no other entered part near the prism its normal sweeps), `at(i, p)` the test at 3D point `p` of triangle `i`.
pub(crate) struct Burial<'b> {
    pub(crate) may: &'b (dyn Fn(usize) -> bool + Sync),
    pub(crate) at: &'b (dyn Fn(usize, DVec3) -> bool + Sync),
}

/// Per layer, over the reach rectangle: per cell the **owner** of its centre, the stored triangle covering the centre
/// with the smallest `(buried, g, triangle)`, as its depth `g`, its 3D point and its stretch (round 9, T1; the owner
/// test of `bake_wrapped`). A cell whose centre no triangle covers has no owner (`g` infinite).
pub(crate) struct OwnerGrid {
    pub(crate) lo: [f32; 2],
    pub(crate) cell: f32,
    pub(crate) dims: [usize; 2],
    pub(crate) g: Vec<f32>,
    pub(crate) pos: Vec<[f32; 3]>,
    pub(crate) scale: Vec<f32>,
}

/// The barycentric weights of `c` in the layout triangle `q` when it lies inside (edges included), else `None`.
fn inside_weights(q: &[DVec2; 3], c: DVec2) -> Option<[f64; 3]> {
    let area = cross2(q[1] - q[0], q[2] - q[0]);
    if area == 0.0 || !area.is_finite() {
        return None;
    }
    let w = [cross2(q[2] - q[1], c - q[1]) / area, cross2(q[0] - q[2], c - q[2]) / area, cross2(q[1] - q[0], c - q[0]) / area];
    w.iter().all(|&x| x >= 0.0).then_some(w)
}

/// The owner build's rows per parallel chunk.
const OWNER_CHUNK_ROWS: usize = 16;
/// The owner build runs on the pool from this many triangles, its \`buried\` pass also from this many mixed cells.
const OWNER_PAR_TRIS: usize = 2048;
const OWNER_PAR_MIXED: usize = 20_000;

/// One cell during the owner build: the owner's depth, index and 3D point; the first candidate's part and point and
/// the `mixed` bits (1 another part, 2 another place).
#[derive(Clone, Copy)]
struct CellState {
    g: f64,
    best: u32,
    part0: u32,
    p: [f32; 3],
    first: [f32; 3],
    seen: u8,
    /// A closure triangle covers the centre (check:A, `lost_cut`).
    closure: bool,
}

impl Default for CellState {
    fn default() -> CellState {
        CellState { g: f64::INFINITY, best: NONE, part0: NONE, p: [0.0; 3], first: [0.0; 3], seen: 0, closure: false }
    }
}

impl OwnerTri {
    /// Depth and 3D point at the weights `w`.
    fn at(&self, w: [f64; 3]) -> (f64, DVec3) {
        let g = self.g[0] * w[0] + self.g[1] * w[1] + self.g[2] * w[2];
        let p = |i: usize| DVec3::new(self.p3[i].x as f64, self.p3[i].y as f64, self.p3[i].z as f64);
        (g, p(0) * w[0] + p(1) * w[1] + p(2) * w[2])
    }
}

impl OwnerGrid {
    /// The grid over `|x| <= half[0], |y| <= half[1]` with cells of at least `min_cell` (at most `OWNER_MAX_CELLS` on
    /// the long side) from the stored triangles `tris` (ascending triangle id). `buried` is asked only in cells whose
    /// candidates come from two or more parts. Also returns, of the (cell centre, covering triangle) pairs inside
    /// `picture`, those the owner test drops and all.
    pub(crate) fn build(tris: &[OwnerTri], half: [f64; 2], min_cell: f64, picture: [f64; 2], buried: &Burial<'_>) -> (OwnerGrid, (usize, usize), (usize, usize)) {
        let long = 2.0 * half[0].max(half[1]);
        let cell = min_cell.max(long / OWNER_MAX_CELLS as f64).max(1e-30);
        let dims = [((2.0 * half[0] / cell).ceil() as usize).clamp(1, 4 * OWNER_MAX_CELLS), ((2.0 * half[1] / cell).ceil() as usize).clamp(1, 4 * OWNER_MAX_CELLS)];
        let lo = [-half[0], -half[1]];
        let n = dims[0] * dims[1];
        let debug: bool = knob("WTE_TEST_OWNER_DEBUG", false);
        let t0 = std::time::Instant::now();
        let first = |a: f64, axis: usize| ((a - lo[axis]) / cell - 0.5).ceil().max(0.0);
        let last = |b: f64, axis: usize| ((b - lo[axis]) / cell - 0.5).floor().min(dims[axis] as f64 - 1.0);
        // The rows of cell centres a triangle may cover (`None` when none).
        let rows_of = |q: &[DVec2; 3]| -> Option<(usize, usize)> {
            if !q.iter().all(|p| p.is_finite()) {
                return None;
            }
            let (j0, j1) = (first(q[0].y.min(q[1].y).min(q[2].y), 1), last(q[0].y.max(q[1].y).max(q[2].y), 1));
            (j0 <= j1).then_some((j0 as usize, j1 as usize))
        };
        // The cell centres of rows `r0..r1` a triangle covers: f(cell, centre, weights).
        let visit = |q: &[DVec2; 3], r0: usize, r1: usize, f: &mut dyn FnMut(usize, DVec2, [f64; 3])| {
            let Some((j0, j1)) = rows_of(q) else { return };
            for j in j0.max(r0)..=j1.min(r1.saturating_sub(1)) {
                let y = lo[1] + (j as f64 + 0.5) * cell;
                let Some((sx0, sx1)) = row_span(q, y, y) else { continue };
                let (i0, i1) = (first(sx0, 0), last(sx1, 0));
                if i0.partial_cmp(&i1).is_none_or(|o| o == Ordering::Greater) {
                    continue;
                }
                for i in i0 as usize..=i1 as usize {
                    let c = DVec2::new(lo[0] + (i as f64 + 0.5) * cell, y);
                    if let Some(w) = inside_weights(q, c) {
                        f(j * dims[0] + i, c, w);
                    }
                }
            }
        };
        // Row chunks (processed in parallel, each cell by its own chunk only) and the triangles of each, ascending.
        let chunk_rows = OWNER_CHUNK_ROWS;
        let chunks = dims[1].div_ceil(chunk_rows);
        let mut bins: Vec<Vec<u32>> = vec![Vec::new(); chunks];
        for (i, t) in tris.iter().enumerate() {
            if let Some((j0, j1)) = rows_of(&t.q) {
                for bin in &mut bins[j0 / chunk_rows..=j1 / chunk_rows] {
                    bin.push(i as u32);
                }
            }
        }
        let width = dims[0];
        // Pass 1, per cell: the owner by (g, triangle); `mixed`: candidates of two or more parts and at two or more
        // places (one farther than half the owner test's smallest tolerance from the first candidate's point:
        // candidates all at one place need no choice, e.g. coplanar overlapping parts).
        let half_tol = 0.5 * OWNER_SAME as f64 * cell * std::f64::consts::SQRT_2;
        let mut cells: Vec<CellState> = vec![CellState::default(); n];
        // On the pool only for large inputs: a parameterization running on a busy pool (another bake, a test run)
        // would wait for it (same bits either way: each chunk writes only its own cells).
        let par = tris.len() >= OWNER_PAR_TRIS;
        let pass1 = |(c, out): (usize, &mut [CellState])| {
            let base = c * chunk_rows * width;
            let (r0, r1) = (c * chunk_rows, ((c + 1) * chunk_rows).min(dims[1]));
            for &i in &bins[c] {
                let t = &tris[i as usize];
                visit(&t.q, r0, r1, &mut |k, _, w| {
                    let (g, p) = t.at(w);
                    let p = [p.x as f32, p.y as f32, p.z as f32];
                    let s = &mut out[k - base];
                    if s.best == NONE {
                        *s = CellState { g, best: i, part0: t.part, p, first: p, seen: 0, closure: t.closure };
                        return;
                    }
                    s.closure |= t.closure;
                    if s.seen != 3 {
                        if s.part0 != t.part {
                            s.seen |= 1;
                        }
                        let d = Vec3::from(s.first).distance(Vec3::from(p)) as f64;
                        if d > half_tol {
                            s.seen |= 2;
                        }
                    }
                    if g < s.g {
                        s.g = g;
                        s.best = i;
                        s.p = p;
                    }
                });
            }
        };
        if par {
            cells.par_chunks_mut(chunk_rows * width).enumerate().for_each(pass1);
        } else {
            cells.chunks_mut(chunk_rows * width).enumerate().for_each(pass1);
        }
        let mixed = cells.iter().filter(|s| s.seen == 3).count();
        if debug {
            eprintln!("owner: {} x {} cells, {} triangles, pass 1 {:.1} ms, mixed {mixed}", dims[0], dims[1], tris.len(), t0.elapsed().as_secs_f64() * 1e3);
        }
        // Pass 2 (mixed cells only): the owner by (buried, g, triangle). The candidates in (g, triangle) order: the
        // first is pass 1's owner and stands unless it may be buried (`Burial::may`, once per triangle) and is;
        // then the next one not buried wins (else the first). `buried` is asked lazily in that order, chunks in
        // parallel: the same owners with 1 or N threads.
        if mixed > 0 {
            let mut used: Vec<u32> = cells.iter().filter(|s| s.seen == 3).map(|s| s.best).collect();
            used.sort_unstable();
            used.dedup();
            let flags: Vec<bool> = used.par_iter().map(|&i| (buried.may)(i as usize)).collect();
            let mut may = vec![None::<bool>; tris.len()];
            for (&i, &m) in used.iter().zip(&flags) {
                may[i as usize] = Some(m);
            }
            // The other candidates' `may`, for the triangles of the chunks that hold a mixed cell.
            let mut rest: Vec<u32> = Vec::new();
            for (c, bin) in bins.iter().enumerate() {
                let (a, b) = (c * chunk_rows * width, ((c + 1) * chunk_rows * width).min(n));
                if cells[a..b].iter().any(|s| s.seen == 3 && may[s.best as usize] == Some(true)) {
                    rest.extend(bin.iter().copied().filter(|&i| may[i as usize].is_none()));
                }
            }
            rest.sort_unstable();
            rest.dedup();
            let flags: Vec<bool> = rest.par_iter().map(|&i| (buried.may)(i as usize)).collect();
            for (&i, &m) in rest.iter().zip(&flags) {
                may[i as usize] = Some(m);
            }
            let asked = std::sync::atomic::AtomicUsize::new(0);
            let pass2 = |(c, out): (usize, &mut [CellState])| {
                let base = c * chunk_rows * width;
                if !out.iter().any(|s| s.seen == 3 && may[s.best as usize] == Some(true)) {
                    return;
                }
                let (r0, r1) = (c * chunk_rows, ((c + 1) * chunk_rows).min(dims[1]));
                // Cells whose pass 1 owner is buried.
                let mut needy = vec![false; out.len()];
                let mut count = 0;
                for (j, s) in out.iter().enumerate() {
                    if s.seen == 3 && may[s.best as usize] == Some(true) {
                        count += 1;
                        let p = Vec3::from(s.p);
                        needy[j] = (buried.at)(s.best as usize, DVec3::new(p.x as f64, p.y as f64, p.z as f64));
                    }
                }
                asked.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
                if !needy.iter().any(|&x| x) {
                    return;
                }
                let mut items: Vec<(usize, u32, f64, [f32; 3])> = Vec::new();
                for &i in &bins[c] {
                    let t = &tris[i as usize];
                    visit(&t.q, r0, r1, &mut |k, _, w| {
                        if needy[k - base] {
                            let (g, p) = t.at(w);
                            items.push((k - base, i, g, [p.x as f32, p.y as f32, p.z as f32]));
                        }
                    });
                }
                items.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.2.total_cmp(&b.2)).then(a.1.cmp(&b.1)));
                let mut j = 0;
                while j < items.len() {
                    let mut e = j + 1;
                    while e < items.len() && items[e].0 == items[j].0 {
                        e += 1;
                    }
                    let pick = items[j + 1..e]
                        .iter()
                        .position(|&(_, i, _, p)| {
                            let p = Vec3::from(p);
                            !(may[i as usize] == Some(true) && (buried.at)(i as usize, DVec3::new(p.x as f64, p.y as f64, p.z as f64)))
                        })
                        .map_or(j, |x| j + 1 + x);
                    let (k, i, g, p) = items[pick];
                    let s = &mut out[k];
                    s.best = i;
                    s.g = g;
                    s.p = p;
                    j = e;
                }
            };
            if par || mixed >= OWNER_PAR_MIXED {
                cells.par_chunks_mut(chunk_rows * width).enumerate().for_each(pass2);
            } else {
                cells.chunks_mut(chunk_rows * width).enumerate().for_each(pass2);
            }
            if debug {
                eprintln!("owner: pass 2 {:.1} ms, {} cells asked", t0.elapsed().as_secs_f64() * 1e3, asked.into_inner());
            }
        }
        let grid = OwnerGrid {
            lo: [lo[0] as f32, lo[1] as f32],
            cell: cell as f32,
            dims,
            g: cells.iter().map(|s| if s.best == NONE { f32::INFINITY } else { s.g as f32 }).collect(),
            pos: cells.iter().map(|s| s.p).collect(),
            scale: cells.iter().map(|s| if s.best == NONE { 1.0 } else { tris[s.best as usize].s }).collect(),
        };
        // Round 9 (A7): `lost_cut`, the picture's cells without an owner inside the laid set's outline: an uncovered
        // cell with covered cells on both sides in its row or in its column (a cut wedge, a shrunk seam, a hole), as
        // against picture beyond a mesh edge or beyond the spine's ends (open to the grid's border both ways).
        let covered = |k: usize| cells[k].best != NONE;
        let mut rspan = vec![(usize::MAX, 0usize); dims[1]];
        let mut cspan = vec![(usize::MAX, 0usize); dims[0]];
        for (j, r) in rspan.iter_mut().enumerate() {
            for (i, c) in cspan.iter_mut().enumerate() {
                if covered(j * width + i) {
                    r.0 = r.0.min(i);
                    r.1 = r.1.max(i);
                    c.0 = c.0.min(j);
                    c.1 = c.1.max(j);
                }
            }
        }
        // check:A (round 9): the picture beyond a closure (where the ends or the halves meet: a triangle-sized
        // tooth line, and with a helix the band past the meeting line) is not lost to a cut. Uncovered picture
        // cells 4-connected through uncovered picture cells to a cell a closure triangle covers are left out.
        let in_picture = |i: usize, j: usize| {
            let (x, y) = (lo[0] + (i as f64 + 0.5) * cell, lo[1] + (j as f64 + 0.5) * cell);
            x.abs() <= picture[0] && y.abs() <= picture[1]
        };
        let mut beyond = vec![false; n];
        let mut queue: Vec<usize> = Vec::new();
        let neighbours = |k: usize| {
            let (i, j) = (k % width, k / width);
            [(i > 0).then(|| k - 1), (i + 1 < width).then(|| k + 1), (j > 0).then(|| k - width), (j + 1 < dims[1]).then(|| k + width)]
        };
        for k in 0..n {
            if !covered(k) && in_picture(k % width, k / width) && neighbours(k).into_iter().flatten().any(|m| cells[m].closure) {
                beyond[k] = true;
                queue.push(k);
            }
        }
        while let Some(k) = queue.pop() {
            for m in neighbours(k).into_iter().flatten() {
                if !beyond[m] && !covered(m) && in_picture(m % width, m / width) {
                    beyond[m] = true;
                    queue.push(m);
                }
            }
        }
        let dump: String = knob("WTE_TEST_OWNER_DUMP", String::new());
        if !dump.is_empty() {
            // Test knob: the grid as a PGM (white covered in the picture, black lost_cut, dark grey beyond).
            let mut out = format!("P5 {} {} 255
", dims[0], dims[1]).into_bytes();
            for j in (0..dims[1]).rev() {
                for i in 0..dims[0] {
                    let k = j * width + i;
                    let (r, c) = (rspan[j], cspan[i]);
                    let inpic = in_picture(i, j);
                    out.push(if covered(k) {
                        if inpic { 255 } else { 190 }
                    } else if inpic && !beyond[k] && ((r.0 < i && i < r.1) || (c.0 < j && j < c.1)) {
                        0
                    } else if inpic {
                        90
                    } else {
                        40
                    });
                }
            }
            static DUMPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let _ = std::fs::write(format!("{dump}_{:02}.pgm", DUMPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), out);
        }
        let (mut lost, mut pic_cells) = (0usize, 0usize);
        for (j, r) in rspan.iter().enumerate() {
            let y = lo[1] + (j as f64 + 0.5) * cell;
            if y.abs() > picture[1] {
                continue;
            }
            for (i, c) in cspan.iter().enumerate() {
                let x = lo[0] + (i as f64 + 0.5) * cell;
                if x.abs() > picture[0] {
                    continue;
                }
                pic_cells += 1;
                if !covered(j * width + i) && !beyond[j * width + i] && ((r.0 < i && i < r.1) || (c.0 < j && j < c.1)) {
                    lost += 1;
                }
            }
        }
        drop(cells);
        // The rejected share at the cell centres inside the picture, of the laid triangles (`WrapStats::rejected`: the
        // laid picture area; a halo triangle hinged over a fold, e.g. a block's side hanging into a gap, is dropped
        // where another place owns its cells without counting). Per chunk, integer counts.
        let count = |c: usize| {
                let (r0, r1) = (c * chunk_rows, ((c + 1) * chunk_rows).min(dims[1]));
                let (mut total, mut rejected) = (0usize, 0usize);
                for &i in &bins[c] {
                    let t = &tris[i as usize];
                    if !t.laid {
                        continue;
                    }
                    visit(&t.q, r0, r1, &mut |k, c, w| {
                        if c.x.abs() > picture[0] || c.y.abs() > picture[1] {
                            return;
                        }
                        total += 1;
                        let (_, p) = t.at(w);
                        if !grid.keeps(k, Vec3::new(p.x as f32, p.y as f32, p.z as f32), t.s) {
                            rejected += 1;
                        }
                    });
                }
                (rejected, total)
        };
        let counts: Vec<(usize, usize)> = if par { (0..chunks).into_par_iter().map(count).collect() } else { (0..chunks).map(count).collect() };
        let rej = counts.iter().fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        if debug {
            eprintln!("owner: done {:.1} ms ({} x {} cells, {} triangles)", t0.elapsed().as_secs_f64() * 1e3, dims[0], dims[1], tris.len());
        }
        (grid, rej, (lost, pic_cells))
    }

    /// The owner test at cell index `k`: a fragment at 3D point `p` of a triangle with stretch `s` is kept when the
    /// cell has no owner or `p` is the owner's place (within `OWNER_SAME x cell diagonal x max(1, s, owner's s)`).
    #[inline]
    pub(crate) fn keeps(&self, k: usize, p: Vec3, s: f32) -> bool {
        if !self.g[k].is_finite() {
            return true;
        }
        let o = self.pos[k];
        let d = Vec3::new(p.x - o[0], p.y - o[1], p.z - o[2]).length();
        d <= OWNER_SAME * self.cell * std::f32::consts::SQRT_2 * s.max(self.scale[k]).max(1.0)
    }

    /// The owner's 3D point at cell index `k` (`None` without an owner).
    pub(crate) fn owner_at(&self, k: usize) -> Option<Vec3> {
        self.g[k].is_finite().then(|| Vec3::from(self.pos[k]))
    }

    /// The cell index of picture point `l` (`None` outside the grid).
    #[inline]
    pub(crate) fn index(&self, x: f32, y: f32) -> Option<usize> {
        let fx = (x - self.lo[0]) / self.cell;
        let fy = (y - self.lo[1]) / self.cell;
        if !(fx >= 0.0 && fy >= 0.0) {
            return None;
        }
        let (i, j) = (fx as usize, fy as usize);
        (i < self.dims[0] && j < self.dims[1]).then(|| j * self.dims[0] + i)
    }
}

/// The x span of the triangle `q` clipped to the band `y0 <= y <= y1` (`None` when it misses the band).
fn row_span(q: &[DVec2; 3], y0: f64, y1: f64) -> Option<(f64, f64)> {
    let (mut a, mut b) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut take = |x: f64| {
        a = a.min(x);
        b = b.max(x);
    };
    for i in 0..3 {
        let (p, r) = (q[i], q[(i + 1) % 3]);
        if p.y >= y0 && p.y <= y1 {
            take(p.x);
        }
        for y in [y0, y1] {
            if (p.y - y) * (r.y - y) < 0.0 {
                take(p.x + (r.x - p.x) * (y - p.y) / (r.y - p.y));
            }
        }
    }
    (a <= b).then_some((a, b))
}

#[cfg(test)]
mod tests {
    // check:A (round 8): the solver on its own: budgets and caps (CG non-convergence), slivers and huge gradients,
    // the parallel kernels against the serial ones, the fast-marching update.
    use super::*;

    /// A grid of `n` x `n` quads (two triangles each) over a gentle height field: per triangle its unknowns and rest
    /// terms (area, gradient basis), and per unknown its rest position.
    type Grid = (Vec<[u32; 3]>, Vec<(f64, [DVec2; 3])>, Vec<DVec2>);

    fn grid(n: usize) -> Grid {
        let at = |i: usize, j: usize| {
            let (x, y) = (i as f64 / n as f64, j as f64 / n as f64);
            DVec3::new(x, y, 0.05 * (6.0 * x).sin() * (5.0 * y).cos())
        };
        let id = |i: usize, j: usize| (j * (n + 1) + i) as u32;
        let (mut tris, mut rest) = (Vec::new(), Vec::new());
        for j in 0..n {
            for i in 0..n {
                for t in [[(i, j), (i + 1, j), (i + 1, j + 1)], [(i, j), (i + 1, j + 1), (i, j + 1)]] {
                    let p = t.map(|(a, b)| at(a, b));
                    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
                    let l1 = e1.length();
                    let xh = e1 / l1;
                    let e2x = e2.dot(xh);
                    let e2y = (e2 - xh * e2x).length();
                    let det = l1 * e2y;
                    let g1 = DVec2::new(e2y, -e2x) / det;
                    let g2 = DVec2::new(0.0, l1) / det;
                    tris.push(t.map(|(a, b)| id(a, b)));
                    rest.push((0.5 * e1.cross(e2).length(), [-(g1 + g2), g1, g2]));
                }
            }
        }
        let flat = (0..(n + 1) * (n + 1)).map(|k| DVec2::new((k % (n + 1)) as f64 / n as f64, (k / (n + 1)) as f64 / n as f64)).collect();
        (tris, rest, flat)
    }

    /// A crumpled start: the flat positions pushed around by a fixed pseudo-random pattern.
    fn crumpled(flat: &[DVec2]) -> Vec<DVec2> {
        let mut s = 12345u64;
        flat.iter()
            .map(|p| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let a = (s >> 33) as f64 / (1u64 << 31) as f64 - 0.5;
                let b = (s >> 11 & 0xffff) as f64 / 65536.0 - 0.5;
                *p + DVec2::new(a, b) * 0.3
            })
            .collect()
    }

    #[test]
    fn check_a8_the_cg_budget_and_caps_bound_the_work_and_keep_positions_finite() {
        let (tris, rest, flat) = grid(24);
        let n = flat.len();
        let mut pins = vec![false; n];
        pins[0] = true;
        pins[24] = true;
        for budget in [0u32, 1, 5, 40] {
            let mut x = crumpled(&flat);
            x[0] = flat[0];
            x[24] = flat[24];
            let start = x.clone();
            let mut left = budget;
            let iters = relax_component(&tris, &rest, &pins, &mut x, &mut left);
            assert!(iters <= budget && iters + left == budget, "budget {budget}: {iters} iterations, {left} left");
            assert!(x.iter().all(|p| p.is_finite()), "budget {budget}: non-finite positions");
            assert_eq!((x[0], x[24]), (flat[0], flat[24]), "a pin moved");
            if budget == 0 {
                assert!(x == start, "no budget, yet the positions changed");
            }
        }
        // Enough budget: the relaxed grid is close to its development (an isometry up to the gentle height field).
        let mut x = crumpled(&flat);
        x[0] = flat[0];
        x[24] = flat[24];
        let mut left = RELAX_CG_BUDGET;
        relax_component(&tris, &rest, &pins, &mut x, &mut left);
        let worst = tris.iter().zip(&rest).map(|(t, r)| {
            let q = t.map(|w| x[w as usize]);
            (0.5 * cross2(q[1] - q[0], q[2] - q[0]) / r.0 - 1.0).abs()
        }).fold(0.0f64, f64::max);
        println!("check:A8 solver: crumpled 25 x 25 grid relaxed, worst area ratio off by {worst:.4}");
        assert!(worst < 0.2, "{worst}");
        // Everything pinned: nothing to solve.
        let mut x = flat.clone();
        let mut left = 100;
        assert_eq!(relax_component(&tris, &rest, &vec![true; n], &mut x, &mut left), 0);
        assert!(x == flat);
    }

    #[test]
    fn check_a8_huge_gradients_and_nan_starts_stay_finite() {
        let (tris, mut rest, flat) = grid(8);
        // A near-sliver's gradients (1e9) next to normal ones.
        rest[10].1 = rest[10].1.map(|g| g * 1e9);
        rest[11].0 = 1e-30;
        let mut pins = vec![false; flat.len()];
        pins[0] = true;
        pins[8] = true;
        let mut x = crumpled(&flat);
        x[0] = flat[0];
        x[8] = flat[8];
        let mut left = RELAX_CG_BUDGET;
        relax_component(&tris, &rest, &pins, &mut x, &mut left);
        assert!(x.iter().all(|p| p.is_finite()), "huge gradients gave non-finite positions");
        // A non-finite start of a free unknown: the solver stops; the caller keeps the seed for non-finite results.
        let mut x = crumpled(&flat);
        x[0] = flat[0];
        x[8] = flat[8];
        x[40] = DVec2::new(f64::NAN, 0.0);
        let mut left = RELAX_CG_BUDGET;
        let iters = relax_component(&tris, &rest, &pins, &mut x, &mut left);
        println!("check:A8 solver: a NaN start stops after {iters} iterations");
        assert!(iters <= RELAX_CG_BUDGET);
    }

    #[test]
    fn check_a8_parallel_kernels_give_the_serial_bits() {
        // More than three chunks of unknowns and of triangles.
        let (tris, rest, flat) = grid(170);
        let n = flat.len();
        assert!(n > 3 * PAR_CHUNK);
        let mut pins = vec![false; n];
        pins[0] = true;
        pins[170] = true;
        let mut sys = System::new(n, &tris, &rest, &pins);
        let start = crumpled(&flat);
        let run = |sys: &System<'_>| {
            let mut x = start.clone();
            let mut budget = 60;
            sys.cg(true, &vec![DVec2::ZERO; n], &mut x, LSCM_TOL, 60, &mut budget);
            x
        };
        sys.par = false;
        let serial = run(&sys);
        sys.par = true;
        for threads in [1, 12] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            let par = pool.install(|| run(&sys));
            assert!(par.iter().zip(&serial).all(|(a, b)| a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits()), "{threads} threads differ from the serial kernels");
        }
    }

    #[test]
    fn check_a8_fast_marching_and_slivers() {
        // A plane wave from the edge (v, a) at depth 0 reaches w at its height.
        let (v, a, w) = (DVec3::ZERO, DVec3::X, DVec3::new(0.5, 0.75f64.sqrt(), 0.0));
        let g = fmm_update(v, a, w, 0.0, 0.0).unwrap();
        assert!((g - 0.75f64.sqrt()).abs() < 1e-12, "{g}");
        // A wave arriving along the edge (|ga - gv| = |a - v|) does not pass through the triangle.
        assert!(fmm_update(v, a, w, 0.0, 1.0).is_none());
        // An obtuse triangle where the wave would leave through the side: not causal.
        assert!(fmm_update(v, a, DVec3::new(3.0, 0.1, 0.0), 0.0, 0.0).is_none());
        assert!(!sliver([v, a, w]));
        assert!(sliver([v, a, DVec3::new(0.5, 1e-5, 0.0)]));
        assert!(sliver([v, a, DVec3::new(0.5, 0.0, 0.0)]));
        assert!(sliver([v, v, w]));
    }
}
