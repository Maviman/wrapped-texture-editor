// Owner: core-geometry (C1). Bounding volume hierarchy and its queries (ARCHITECTURE.md 5.8, algorithm 6.2):
// nearest-hit raycast, any-hit occlusion test and the sphere query of the brush and stamp bakes.

// Range checks are written as `!(value >= limit)` on purpose: a NaN then takes the rejecting branch.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use glam::{Vec2, Vec3};

use crate::ids::{TextureSetId, TriId};
use crate::math::Ray;
use crate::mesh::{MeshData, TriFlags};

/// Face culling of a ray query.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Cull {
    None,
    Back,
}

/// Result of a ray query.
#[derive(Copy, Clone, Debug)]
pub struct Hit {
    pub tri: TriId,
    pub t: f32,
    /// Weights of v0, v1, v2.
    pub bary: [f32; 3],
    pub position: Vec3,
    /// From winding, unit (never flipped).
    pub geo_normal: Vec3,
    /// `dot(geo_normal, ray.dir) < 0`.
    pub front_face: bool,
    /// Interpolated shading normal, unit, negated iff `!front_face` (so it faces the viewer); `geo_normal` (same
    /// negation) if the interpolation is degenerate.
    pub normal: Vec3,
    /// None for NO_UV; not wrapped into 0..1.
    pub uv: Option<Vec2>,
    pub material: u32,
    /// None for NO_UV triangles and set-less materials.
    pub texture_set: Option<TextureSetId>,
}

impl Hit {
    /// The unflipped shading normal: `if front_face { normal } else { -normal }`. This is the reference normal of brush
    /// strokes (section 6.4); `normal` (viewer-facing) is the one used to orient stamp projectors.
    pub fn surface_normal(&self) -> Vec3 {
        if self.front_face { self.normal } else { -self.normal }
    }
}

/// `count == 0` => interior node with children at `left_first` and `left_first + 1`; otherwise a leaf covering
/// `tri_order[left_first .. left_first + count]`.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct BvhNode {
    min: [f32; 3],
    left_first: u32,
    max: [f32; 3],
    count: u32,
}

/// Bounding volume hierarchy over the triangles of one `MeshData`. Send + Sync.
pub struct Bvh {
    nodes: Vec<BvhNode>,
    tri_order: Vec<u32>,
}

/// A triangle during the build.
#[derive(Copy, Clone)]
struct Prim {
    min: [f32; 3],
    max: [f32; 3],
    centroid: [f32; 3],
    tri: u32,
}

const LEAF_MAX: usize = 4;
const BINS: usize = 16;
/// Subtrees above this many triangles are split with `rayon::join`.
const PARALLEL_TRIS: usize = 8192;
/// Size of the traversal stack. The build keeps the tree shallower than this (see `use_sah`).
const STACK: usize = 64;
/// Barycentric tolerance of the triangle test: edges are slightly inclusive so that a ray through a shared edge
/// cannot slip between two triangles.
const BARY_EPS: f32 = 1e-6;

fn bounds_of(prims: &[Prim]) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in prims {
        for a in 0..3 {
            lo[a] = lo[a].min(p.min[a]);
            hi[a] = hi[a].max(p.max[a]);
        }
    }
    (lo, hi)
}

fn leaf(prims: &[Prim], first: usize) -> BvhNode {
    let (min, max) = bounds_of(prims);
    BvhNode { min, left_first: first as u32, max, count: prims.len() as u32 }
}

fn half_area(lo: &[f32; 3], hi: &[f32; 3]) -> f32 {
    let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    d[0] * d[1] + d[1] * d[2] + d[2] * d[0]
}

/// Binned SAH is used only while the worst case of the remaining median splits still fits the traversal stack:
/// a median split halves the count, so a subtree of `n` triangles started at `depth` ends above
/// `depth + bit_length(n)`.
fn use_sah(n: usize, depth: u32) -> bool {
    depth + (usize::BITS - n.leading_zeros()) + 4 < STACK as u32
}

/// Partitions `prims` (more than `LEAF_MAX` of them) into two non-empty halves and returns the size of the first.
fn split(prims: &mut [Prim], depth: u32) -> usize {
    let n = prims.len();
    let (mut cmin, mut cmax) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in prims.iter() {
        for a in 0..3 {
            cmin[a] = cmin[a].min(p.centroid[a]);
            cmax[a] = cmax[a].max(p.centroid[a]);
        }
    }
    let extent = [cmax[0] - cmin[0], cmax[1] - cmin[1], cmax[2] - cmin[2]];

    if use_sah(n, depth) {
        // Bin on all three axes in one pass.
        let scale = extent.map(|e| if e > 0.0 && e.is_finite() { BINS as f32 / e } else { 0.0 });
        let bin_of = |p: &Prim, a: usize| (((p.centroid[a] - cmin[a]) * scale[a]) as usize).min(BINS - 1);
        let mut counts = [[0u32; BINS]; 3];
        let mut lo = [[[f32::INFINITY; 3]; BINS]; 3];
        let mut hi = [[[f32::NEG_INFINITY; 3]; BINS]; 3];
        for p in prims.iter() {
            for a in 0..3 {
                if scale[a] == 0.0 {
                    continue;
                }
                let b = bin_of(p, a);
                counts[a][b] += 1;
                for k in 0..3 {
                    lo[a][b][k] = lo[a][b][k].min(p.min[k]);
                    hi[a][b][k] = hi[a][b][k].max(p.max[k]);
                }
            }
        }
        // Sweep: cost of splitting after bin i = area(left) * n_left + area(right) * n_right.
        let mut best: Option<(f32, usize, usize)> = None;
        for a in 0..3 {
            if scale[a] == 0.0 {
                continue;
            }
            let mut right_area = [0.0f32; BINS];
            let (mut rlo, mut rhi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
            for i in (1..BINS).rev() {
                for k in 0..3 {
                    rlo[k] = rlo[k].min(lo[a][i][k]);
                    rhi[k] = rhi[k].max(hi[a][i][k]);
                }
                right_area[i] = half_area(&rlo, &rhi);
            }
            let (mut llo, mut lhi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
            let mut n_left = 0u32;
            for i in 0..BINS - 1 {
                for k in 0..3 {
                    llo[k] = llo[k].min(lo[a][i][k]);
                    lhi[k] = lhi[k].max(hi[a][i][k]);
                }
                n_left += counts[a][i];
                let n_right = n as u32 - n_left;
                if n_left == 0 || n_right == 0 {
                    continue;
                }
                let cost = half_area(&llo, &lhi) * n_left as f32 + right_area[i + 1] * n_right as f32;
                if cost.is_finite() && best.is_none_or(|(c, _, _)| cost < c) {
                    best = Some((cost, a, i));
                }
            }
        }
        if let Some((_, axis, bin)) = best {
            let (mut i, mut j) = (0, n);
            while i < j {
                if bin_of(&prims[i], axis) <= bin {
                    i += 1;
                } else {
                    j -= 1;
                    prims.swap(i, j);
                }
            }
            if i > 0 && i < n {
                return i;
            }
        }
    }

    // Median split on the widest centroid axis (all centroids in one bin, or the depth budget is used up).
    let mut axis = 0;
    for a in 1..3 {
        if extent[a] > extent[axis] {
            axis = a;
        }
    }
    let mid = n / 2;
    prims.select_nth_unstable_by(mid, |x, y| x.centroid[axis].total_cmp(&y.centroid[axis]).then(x.tri.cmp(&y.tri)));
    mid
}

/// Builds the subtree of `prims` with an explicit stack. Node 0 is the root; leaf offsets are relative to the
/// start of `prims`.
fn build_serial(prims: &mut [Prim], depth: u32) -> Vec<BvhNode> {
    let mut nodes = Vec::with_capacity(prims.len() / 2 + 1);
    nodes.push(leaf(prims, 0));
    let mut stack = vec![(0usize, 0usize, prims.len(), depth)];
    while let Some((node, start, end, depth)) = stack.pop() {
        if end - start <= LEAF_MAX {
            continue;
        }
        let mid = start + split(&mut prims[start..end], depth);
        let left = nodes.len();
        nodes.push(leaf(&prims[start..mid], start));
        nodes.push(leaf(&prims[mid..end], mid));
        nodes[node].left_first = left as u32;
        nodes[node].count = 0;
        stack.push((left + 1, mid, end, depth + 1));
        stack.push((left, start, mid, depth + 1));
    }
    nodes
}

/// Builds the subtree of `prims`, splitting large subtrees in parallel. The result does not depend on scheduling.
fn build_parallel(prims: &mut [Prim], depth: u32) -> Vec<BvhNode> {
    if prims.len() <= PARALLEL_TRIS {
        return build_serial(prims, depth);
    }
    let (min, max) = bounds_of(prims);
    let mid = split(prims, depth);
    let (left_prims, right_prims) = prims.split_at_mut(mid);
    let (left, right) =
        rayon::join(|| build_parallel(left_prims, depth + 1), || build_parallel(right_prims, depth + 1));

    // Layout: [root, left root, right root, rest of left, rest of right] keeps sibling pairs adjacent.
    let mut nodes = Vec::with_capacity(1 + left.len() + right.len());
    nodes.push(BvhNode { min, left_first: 1, max, count: 0 });
    let relocate = |mut n: BvhNode, child_shift: u32, prim_shift: u32| {
        n.left_first += if n.count == 0 { child_shift } else { prim_shift };
        n
    };
    let (left_shift, right_shift) = (2u32, left.len() as u32 + 1);
    nodes.push(relocate(left[0], left_shift, 0));
    nodes.push(relocate(right[0], right_shift, mid as u32));
    nodes.extend(left[1..].iter().map(|&n| relocate(n, left_shift, 0)));
    nodes.extend(right[1..].iter().map(|&n| relocate(n, right_shift, mid as u32)));
    nodes
}

/// Moeller-Trumbore. Returns `(t, u, v)` with `u`, `v` the weights of `b` and `c`; no range check on `t`.
#[inline]
fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3, cull: Cull) -> Option<(f32, f32, f32)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    // Reject when |det| < 1e-9 * |e1| * |e2| (compared squared, in f64, so tiny models do not underflow).
    let det64 = det as f64;
    if !(det64 * det64 >= 1e-18 * e1.length_squared() as f64 * e2.length_squared() as f64) || det == 0.0 {
        return None;
    }
    if cull == Cull::Back && det < 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(-BARY_EPS..=1.0 + BARY_EPS).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if !(v >= -BARY_EPS && u + v <= 1.0 + BARY_EPS) {
        return None;
    }
    let t = e2.dot(q) * inv;
    if t.is_finite() { Some((t, u, v)) } else { None }
}

/// A ray prepared for slab tests.
struct RayPrep {
    o: [f32; 3],
    d: [f32; 3],
    inv: [f32; 3],
}

impl RayPrep {
    fn new(ray: &Ray) -> RayPrep {
        let d = ray.dir.to_array();
        RayPrep { o: ray.origin.to_array(), d, inv: d.map(|c| 1.0 / c) }
    }

    /// Entry distance of the ray into the node's box within `[t0, t1]`, or `None` when it misses.
    #[inline]
    fn enter(&self, node: &BvhNode, mut t0: f32, mut t1: f32) -> Option<f32> {
        for a in 0..3 {
            if self.d[a] == 0.0 {
                // Parallel to this slab: inside (faces included) or never. Handled apart because the general form
                // would compute 0 * inf = NaN for an origin exactly on a face.
                if self.o[a] < node.min[a] || self.o[a] > node.max[a] {
                    return None;
                }
            } else {
                let ta = (node.min[a] - self.o[a]) * self.inv[a];
                let tb = (node.max[a] - self.o[a]) * self.inv[a];
                // NaN-ignoring form.
                t0 = t0.max(ta.min(tb));
                t1 = t1.min(ta.max(tb));
            }
        }
        // Conservative against rounding in the slab distances.
        if t0 <= t1 + t1.abs() * 4e-7 { Some(t0) } else { None }
    }
}

impl Bvh {
    /// Builds the hierarchy (binned SAH, section 6.2).
    pub fn build(mesh: &MeshData) -> Bvh {
        let mut prims: Vec<Prim> = (0..mesh.tri_count())
            .map(|tri| {
                let [a, b, c] = mesh.tri_positions(tri);
                let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
                // Two ulps of padding keep the slab test conservative for triangles lying in a box face.
                let pad = lo.abs().max(hi.abs()) * (2.0 * f32::EPSILON) + Vec3::splat(f32::MIN_POSITIVE);
                Prim {
                    min: (lo - pad).to_array(),
                    max: (hi + pad).to_array(),
                    centroid: ((a + b + c) / 3.0).to_array(),
                    tri,
                }
            })
            .collect();
        if prims.is_empty() {
            return Bvh { nodes: Vec::new(), tri_order: Vec::new() };
        }
        let nodes = build_parallel(&mut prims, 0);
        Bvh { nodes, tri_order: prims.iter().map(|p| p.tri).collect() }
    }

    /// Core traversal: nearest (or, with `any`, the first found) triangle hit with `t` in `[t_min, t_max]`.
    /// Ties in `t` go to the smaller triangle id, so the result does not depend on the tree shape.
    fn intersect(
        &self,
        mesh: &MeshData,
        ray: &Ray,
        t_min: f32,
        t_max: f32,
        cull: Cull,
        any: bool,
    ) -> Option<(TriId, f32, f32, f32)> {
        if self.nodes.is_empty() || !(t_min <= t_max) {
            return None;
        }
        let prep = RayPrep::new(ray);
        let (o, d) = (ray.origin, ray.dir);
        let mut best: Option<(TriId, f32, f32, f32)> = None;
        let mut best_t = t_max;
        let mut stack = [(0u32, 0.0f32); STACK];
        let mut sp = 0usize;
        let mut current = match prep.enter(&self.nodes[0], t_min, best_t) {
            Some(_) => 0u32,
            None => return None,
        };
        loop {
            let node = &self.nodes[current as usize];
            if node.count > 0 {
                let first = node.left_first as usize;
                for &tri in &self.tri_order[first..first + node.count as usize] {
                    let [a, b, c] = mesh.tri_positions(tri);
                    let Some((t, u, v)) = ray_triangle(o, d, a, b, c, cull) else { continue };
                    if !(t >= t_min && t <= best_t) {
                        continue;
                    }
                    if best.is_some_and(|(prev, ..)| t == best_t && prev < tri) {
                        continue;
                    }
                    best = Some((tri, t, u, v));
                    best_t = t;
                    if any {
                        return best;
                    }
                }
            } else {
                let (c0, c1) = (node.left_first, node.left_first + 1);
                let h0 = prep.enter(&self.nodes[c0 as usize], t_min, best_t);
                let h1 = prep.enter(&self.nodes[c1 as usize], t_min, best_t);
                match (h0, h1) {
                    (Some(e0), Some(e1)) => {
                        // Near child first; the far one waits on the stack with its entry distance.
                        let (near, far, far_entry) = if e0 <= e1 { (c0, c1, e1) } else { (c1, c0, e0) };
                        stack[sp] = (far, far_entry);
                        sp += 1;
                        current = near;
                        continue;
                    }
                    (Some(_), None) => {
                        current = c0;
                        continue;
                    }
                    (None, Some(_)) => {
                        current = c1;
                        continue;
                    }
                    (None, None) => {}
                }
            }
            // Pop the next node that can still contain a hit at least as near as the best one.
            loop {
                if sp == 0 {
                    return best;
                }
                sp -= 1;
                let (next, entry) = stack[sp];
                if entry <= best_t {
                    current = next;
                    break;
                }
            }
        }
    }

    /// Nearest hit with t in [t_min, t_max].
    pub fn raycast(&self, mesh: &MeshData, ray: &Ray, t_min: f32, t_max: f32, cull: Cull) -> Option<Hit> {
        let (tri, t, u, v) = self.intersect(mesh, ray, t_min, t_max, cull, false)?;
        Some(make_hit(mesh, ray, tri, t, u, v))
    }

    /// Any-hit with early out.
    pub fn occluded(&self, mesh: &MeshData, ray: &Ray, t_min: f32, t_max: f32) -> bool {
        self.intersect(mesh, ray, t_min, t_max, Cull::None, true).is_some()
    }

    /// `occluded` that also names the triangle it found (`Some` exactly when `occluded` is true).
    pub fn occluder(&self, mesh: &MeshData, ray: &Ray, t_min: f32, t_max: f32) -> Option<TriId> {
        self.intersect(mesh, ray, t_min, t_max, Cull::None, true).map(|hit| hit.0)
    }

    /// Triangles whose closest point to `center` is within `radius`. `out` is cleared, then filled sorted ascending.
    pub fn query_sphere(&self, mesh: &MeshData, center: Vec3, radius: f32, out: &mut Vec<TriId>) {
        out.clear();
        self.walk_sphere(mesh, center, radius, |tri, _| out.push(tri));
        out.sort_unstable();
    }

    /// As `query_sphere`, and also returns each triangle's squared closest distance to `center` (the value the
    /// acceptance test of `query_sphere` compares with `radius * radius`). Sorted by triangle id. `out` is cleared
    /// first. The brush asks this once per dab for all texture sets and filters the list per set (FEATURE_BRUSH 5.4).
    pub fn query_sphere_dist(&self, mesh: &MeshData, center: Vec3, radius: f32, out: &mut Vec<(TriId, f32)>) {
        out.clear();
        self.walk_sphere(mesh, center, radius, |tri, d2| out.push((tri, d2)));
        // Round 7 A3: a large result is sorted on the pool (the keys are unique, so the order is the same).
        if out.len() >= 32_768 {
            use rayon::slice::ParallelSliceMut;
            out.par_sort_unstable_by_key(|e| e.0);
        } else {
            out.sort_unstable_by_key(|e| e.0);
        }
    }

    /// The tree walk of the sphere queries: calls `found(tri, d2)` for every triangle whose closest point to
    /// `center` is within `radius` (`d2 <= radius * radius`), in tree order.
    fn walk_sphere(&self, mesh: &MeshData, center: Vec3, radius: f32, mut found: impl FnMut(TriId, f32)) {
        if self.nodes.is_empty() || !center.is_finite() || !(radius >= 0.0) {
            return;
        }
        let r2 = radius * radius;
        let c = center.to_array();
        let box_dist2 = |n: &BvhNode| -> f32 {
            let mut d2 = 0.0;
            for (a, &ca) in c.iter().enumerate() {
                let d = (n.min[a] - ca).max(ca - n.max[a]).max(0.0);
                d2 += d * d;
            }
            d2
        };
        if !(box_dist2(&self.nodes[0]) <= r2) {
            return;
        }
        // Both children are pushed, so the stack holds at most one sibling per level plus the current pair.
        let mut stack = [0u32; STACK + 1];
        let mut sp = 1usize;
        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            if node.count > 0 {
                let first = node.left_first as usize;
                for &tri in &self.tri_order[first..first + node.count as usize] {
                    let [a, b, c] = mesh.tri_positions(tri);
                    let d2 = (closest_point_on_triangle(center, a, b, c) - center).length_squared();
                    if d2 <= r2 {
                        found(tri, d2);
                    }
                }
            } else {
                for child in [node.left_first + 1, node.left_first] {
                    if box_dist2(&self.nodes[child as usize]) <= r2 {
                        stack[sp] = child;
                        sp += 1;
                    }
                }
            }
        }
    }

    /// Number of nodes of the hierarchy.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// Fills in the surface attributes of a hit (section 6.2).
fn make_hit(mesh: &MeshData, ray: &Ray, tri: TriId, t: f32, u: f32, v: f32) -> Hit {
    let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let bary = [(1.0 - u - v).max(0.0), u, v];
    let geo_normal = mesh.geo_normal(tri);
    let front_face = geo_normal.dot(ray.dir) < 0.0;
    let n = mesh.tri_normals(tri);
    let shading = (n[0] * bary[0] + n[1] * bary[1] + n[2] * bary[2]).normalize_or_zero();
    let shading = if shading == Vec3::ZERO || !shading.is_finite() { geo_normal } else { shading };
    let no_uv = mesh.tri_flags[tri as usize].contains(TriFlags::NO_UV);
    let uv = if no_uv {
        None
    } else {
        let uvs = mesh.tri_uvs(tri);
        Some(uvs[0] * bary[0] + uvs[1] * bary[1] + uvs[2] * bary[2])
    };
    Hit {
        tri,
        t,
        bary,
        position: ray.at(t),
        geo_normal,
        front_face,
        normal: if front_face { shading } else { -shading },
        uv,
        material: mesh.tri_material[tri as usize],
        texture_set: mesh.tri_set(tri),
    }
}

/// Closest point on a triangle (Ericson). Exposed for tests and the brush.
pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    // Real-Time Collision Detection, 5.1.5: classify `p` against the Voronoi regions of the triangle.
    let ratio = |num: f32, den: f32| if den != 0.0 { (num / den).clamp(0.0, 1.0) } else { 0.0 };
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * ratio(d1, d1 - d3);
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * ratio(d2, d2 - d6);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ratio(d4 - d3, (d4 - d3) + (d5 - d6));
    }
    let sum = va + vb + vc;
    if !(sum != 0.0 && sum.is_finite()) {
        // Degenerate triangle: fall back to the nearest corner.
        let (da, db, dc) = (ap.length_squared(), bp.length_squared(), cp.length_squared());
        return if da <= db && da <= dc { a } else if db <= dc { b } else { c };
    }
    let v = vb / sum;
    let w = vc / sum;
    a + ab * v + ac * w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{RawMaterial, RawMesh, RawTriangle};
    use crate::samples;
    use crate::testutil::{XorShift, mesh_of};
    use std::f32::consts::{PI, TAU};

    impl Bvh {
        /// (deepest leaf level with the root at 1, triangles referenced by leaves, largest leaf)
        fn shape(&self) -> (usize, usize, usize) {
            if self.nodes.is_empty() {
                return (0, 0, 0);
            }
            let (mut depth, mut tris, mut largest) = (0, 0, 0);
            let mut stack = vec![(0u32, 1usize)];
            while let Some((i, level)) = stack.pop() {
                let n = &self.nodes[i as usize];
                if n.count > 0 {
                    depth = depth.max(level);
                    tris += n.count as usize;
                    largest = largest.max(n.count as usize);
                } else {
                    stack.push((n.left_first, level + 1));
                    stack.push((n.left_first + 1, level + 1));
                }
            }
            (depth, tris, largest)
        }

        /// Every node's box contains its triangles and its children's boxes; every triangle is in exactly one leaf.
        fn check(&self, mesh: &MeshData) {
            let mut seen = vec![0u32; mesh.tri_count() as usize];
            for n in &self.nodes {
                if n.count > 0 {
                    for &t in &self.tri_order[n.left_first as usize..(n.left_first + n.count) as usize] {
                        seen[t as usize] += 1;
                        for p in mesh.tri_positions(t) {
                            for a in 0..3 {
                                assert!(n.min[a] <= p[a] && p[a] <= n.max[a]);
                            }
                        }
                    }
                } else {
                    for c in [n.left_first, n.left_first + 1] {
                        let child = &self.nodes[c as usize];
                        for a in 0..3 {
                            assert!(n.min[a] <= child.min[a] && child.max[a] <= n.max[a]);
                        }
                    }
                }
            }
            assert!(seen.iter().all(|&s| s == 1));
            let (depth, tris, largest) = self.shape();
            assert_eq!(tris, mesh.tri_count() as usize);
            assert!(largest <= LEAF_MAX);
            assert!(depth < STACK, "depth {depth} would overflow the traversal stack");
        }
    }

    fn brute_raycast(mesh: &MeshData, ray: &Ray, t_min: f32, t_max: f32, cull: Cull) -> Option<(TriId, f32)> {
        let mut best: Option<(TriId, f32)> = None;
        for tri in 0..mesh.tri_count() {
            let [a, b, c] = mesh.tri_positions(tri);
            if let Some((t, _, _)) = ray_triangle(ray.origin, ray.dir, a, b, c, cull)
                && t >= t_min
                && t <= t_max
                && best.is_none_or(|(_, bt)| t < bt)
            {
                best = Some((tri, t));
            }
        }
        best
    }

    fn random_dir(rng: &mut XorShift) -> Vec3 {
        loop {
            let v = Vec3::new(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5) * 2.0;
            if v.length_squared() > 0.01 && v.length_squared() <= 1.0 {
                return v.normalize();
            }
        }
    }

    /// A deterministic random triangle soup (no UVs, no normals).
    fn soup(count: usize, seed: u64, extent: f32, size: f32) -> RawMesh {
        let mut rng = XorShift::new(seed);
        let mut point = |scale: f32| Vec3::new(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5) * scale;
        let triangles = (0..count)
            .map(|_| {
                let c = point(extent);
                RawTriangle { positions: [c + point(size), c + point(size), c + point(size)], normals: None, uvs: None, material: 0 }
            })
            .collect();
        RawMesh {
            triangles,
            materials: vec![RawMaterial { name: "soup".into(), base_color: [1.0; 4], double_sided: true, texture: None }],
        }
    }

    #[test]
    fn random_rays_match_brute_force() {
        let mesh = mesh_of(samples::two_material_model());
        let bvh = Bvh::build(&mesh);
        bvh.check(&mesh);
        assert!(bvh.node_count() > mesh.tri_count() as usize / LEAF_MAX);
        let mut rng = XorShift::new(2024);
        let (mut hits, mut culled) = (0, 0);
        for i in 0..10_000 {
            // Origins around the model, aimed at a random point inside its bounds (most rays hit).
            let origin = mesh.bounds.center() + random_dir(&mut rng) * (1.5 + 2.0 * rng.next_f32());
            let target = Vec3::new(rng.next_f32() - 0.5, rng.next_f32() * 1.6 - 0.5, rng.next_f32() - 0.5);
            let ray = Ray::new(origin, target - origin);
            let cull = if i % 2 == 0 { Cull::None } else { Cull::Back };
            let expect = brute_raycast(&mesh, &ray, 0.0, f32::INFINITY, cull);
            let got = bvh.raycast(&mesh, &ray, 0.0, f32::INFINITY, cull);
            match (expect, got) {
                (None, None) => {}
                (Some((tri, t)), Some(hit)) => {
                    assert_eq!(hit.tri, tri, "ray {i}");
                    assert!((hit.t - t).abs() <= 1e-4);
                    hits += 1;
                }
                other => panic!("ray {i}: brute force and BVH disagree: {other:?}"),
            }
            assert_eq!(bvh.occluded(&mesh, &ray, 0.0, f32::INFINITY), brute_raycast(&mesh, &ray, 0.0, f32::INFINITY, Cull::None).is_some());
            assert_eq!(bvh.occluded(&mesh, &ray, 0.0, f32::INFINITY), bvh.raycast(&mesh, &ray, 0.0, f32::INFINITY, Cull::None).is_some());
            // A closed, consistently wound model seen from outside: culling changes nothing for the nearest hit.
            if cull == Cull::Back {
                let unculled = bvh.raycast(&mesh, &ray, 0.0, f32::INFINITY, Cull::None);
                culled += (unculled.map(|h| h.tri) != got.map(|h| h.tri)) as u32;
            }
        }
        assert!(hits > 5000, "only {hits} of 10000 rays hit");
        assert_eq!(culled, 0);
    }

    #[test]
    fn large_soup_matches_brute_force_and_builds_in_parallel() {
        // Above the parallel threshold, with overlapping triangles of very different sizes.
        let mut raw = soup(20_000, 5, 10.0, 0.4);
        raw.triangles.extend(soup(50, 6, 10.0, 8.0).triangles);
        let mesh = mesh_of(raw);
        assert!(mesh.tri_count() as usize > PARALLEL_TRIS);
        let bvh = Bvh::build(&mesh);
        bvh.check(&mesh);
        // Deterministic whatever the scheduling.
        let again = Bvh::build(&mesh);
        assert_eq!(bvh.tri_order, again.tri_order);
        assert_eq!(bvh.node_count(), again.node_count());

        let mut rng = XorShift::new(77);
        let mut found = Vec::new();
        for i in 0..1500 {
            let origin = random_dir(&mut rng) * (rng.next_f32() * 9.0);
            let ray = Ray::new(origin, random_dir(&mut rng));
            let (t_min, t_max) = if i % 3 == 0 { (0.5, 3.0) } else { (0.0, f32::INFINITY) };
            let expect = brute_raycast(&mesh, &ray, t_min, t_max, Cull::None);
            let got = bvh.raycast(&mesh, &ray, t_min, t_max, Cull::None);
            assert_eq!(expect.map(|e| e.0), got.map(|h| h.tri), "ray {i}");
            if let (Some((_, t)), Some(hit)) = (expect, got) {
                assert!((hit.t - t).abs() <= 1e-4);
                assert!(hit.t >= t_min && hit.t <= t_max);
            }
            assert_eq!(bvh.occluded(&mesh, &ray, t_min, t_max), expect.is_some());

            // Sphere query against brute-force closest points.
            let center = random_dir(&mut rng) * (rng.next_f32() * 6.0);
            let radius = rng.next_f32() * 1.2;
            bvh.query_sphere(&mesh, center, radius, &mut found);
            let brute: Vec<TriId> = (0..mesh.tri_count())
                .filter(|&t| {
                    let [a, b, c] = mesh.tri_positions(t);
                    (closest_point_on_triangle(center, a, b, c) - center).length_squared() <= radius * radius
                })
                .collect();
            assert_eq!(found, brute, "sphere {i}");
        }
    }

    /// FEATURE_BRUSH 8.1 C4: `query_sphere_dist` agrees with brute force on the random soup of the test above for
    /// 1,000 centres, returns the squared distance `query_sphere` compares, and lists the ids of `query_sphere` in
    /// the same order.
    #[test]
    fn sphere_query_with_distances_matches_brute_force_and_query_sphere() {
        let mut raw = soup(20_000, 5, 10.0, 0.4);
        raw.triangles.extend(soup(50, 6, 10.0, 8.0).triangles);
        let mesh = mesh_of(raw);
        let bvh = Bvh::build(&mesh);
        let mut rng = XorShift::new(4242);
        let (mut ids, mut with_dist) = (Vec::new(), vec![(7, 1.0)]);
        let mut found_total = 0usize;
        for i in 0..1000 {
            let center = random_dir(&mut rng) * (rng.next_f32() * 6.0);
            let radius = rng.next_f32() * 1.2;
            bvh.query_sphere(&mesh, center, radius, &mut ids);
            bvh.query_sphere_dist(&mesh, center, radius, &mut with_dist);
            let brute: Vec<(TriId, f32)> = (0..mesh.tri_count())
                .filter_map(|t| {
                    let [a, b, c] = mesh.tri_positions(t);
                    let d2 = (closest_point_on_triangle(center, a, b, c) - center).length_squared();
                    (d2 <= radius * radius).then_some((t, d2))
                })
                .collect();
            assert_eq!(with_dist, brute, "sphere {i}");
            assert_eq!(with_dist.iter().map(|e| e.0).collect::<Vec<_>>(), ids, "sphere {i}");
            found_total += ids.len();
        }
        assert!(found_total > 1000, "the centres must find triangles ({found_total})");
        // Invalid input: cleared and empty.
        bvh.query_sphere_dist(&mesh, Vec3::ZERO, f32::NAN, &mut with_dist);
        assert!(with_dist.is_empty());
        bvh.query_sphere_dist(&mesh, Vec3::new(f32::INFINITY, 0.0, 0.0), 1.0, &mut with_dist);
        assert!(with_dist.is_empty());
    }

    #[test]
    fn sphere_query_edge_cases() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let bvh = Bvh::build(&mesh);
        let mut out = vec![99, 98];
        // Far away: cleared and empty.
        bvh.query_sphere(&mesh, Vec3::splat(10.0), 1.0, &mut out);
        assert!(out.is_empty());
        // Around a corner: the triangles of the three faces meeting there that touch the corner, sorted. (The
        // corner is one end of the diagonal on +Z, so both of its triangles count; on +X and +Y one each.)
        bvh.query_sphere(&mesh, Vec3::splat(0.5), 0.05, &mut out);
        let touching: Vec<TriId> =
            (0..12).filter(|&t| mesh.tri_positions(t).iter().any(|p| *p == Vec3::splat(0.5))).collect();
        assert_eq!(out, touching);
        assert_eq!(out.len(), 4);
        let faces: std::collections::BTreeSet<u32> = out.iter().map(|t| t / 2).collect();
        assert_eq!(faces.into_iter().collect::<Vec<_>>(), vec![0, 2, 4]);
        // A tiny radius on the surface still finds the triangle; a huge radius finds everything.
        bvh.query_sphere(&mesh, Vec3::new(0.5, 0.1, 0.2), 1e-5, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0] / 2, 0);
        bvh.query_sphere(&mesh, Vec3::ZERO, f32::INFINITY, &mut out);
        assert_eq!(out, (0..12).collect::<Vec<_>>());
        bvh.query_sphere(&mesh, Vec3::ZERO, 0.49, &mut out);
        assert!(out.is_empty(), "the centre is 0.5 away from every face");
        // Invalid input gives nothing.
        bvh.query_sphere(&mesh, Vec3::ZERO, -1.0, &mut out);
        assert!(out.is_empty());
        bvh.query_sphere(&mesh, Vec3::ZERO, f32::NAN, &mut out);
        assert!(out.is_empty());
        bvh.query_sphere(&mesh, Vec3::new(f32::NAN, 0.0, 0.0), 1.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn analytic_sphere_hits() {
        let (seg, rings, r) = (96u32, 48u32, 2.0f32);
        let mesh = mesh_of(samples::uv_sphere(seg, rings, r));
        let bvh = Bvh::build(&mesh);
        bvh.check(&mesh);
        let mut rng = XorShift::new(9);
        for _ in 0..2000 {
            let dir = random_dir(&mut rng);
            // Stay away from the pole triangles, whose UVs are not equirect.
            if dir.y.abs() > 0.95 {
                continue;
            }
            let ray = Ray::new(dir * 5.0, -dir);
            let hit = bvh.raycast(&mesh, &ray, 0.0, 100.0, Cull::Back).expect("a ray at the centre hits the sphere");
            // The faceted sphere deviates from the true one by at most r * (1 - cos(pi / seg)) ~ 0.001.
            // (twice that in the middle of a quad's diagonal).
            assert!((hit.position.length() - r).abs() < 4e-3);
            assert!((hit.t - 3.0).abs() < 4e-3);
            assert!(hit.front_face);
            assert!((hit.normal - hit.position / r).length() < 4e-3, "smooth normal");
            assert!((hit.normal.length() - 1.0).abs() < 1e-5);
            assert!(hit.geo_normal.dot(dir) > 0.99);
            assert_eq!(hit.surface_normal(), hit.normal);
            assert!((hit.bary[0] + hit.bary[1] + hit.bary[2] - 1.0).abs() < 1e-5);
            assert!((hit.position - ray.at(hit.t)).length() < 1e-6);
            // Equirect formula of `samples::uv_sphere`.
            let uv = hit.uv.unwrap();
            let u = (-dir.x).atan2(-dir.z).rem_euclid(TAU) / TAU;
            let v = dir.y.acos() / PI;
            let du = (uv.x - u).abs();
            assert!(du.min(1.0 - du) < 3e-3, "u {} vs {}", uv.x, u);
            assert!((uv.y - v).abs() < 3e-3, "v {} vs {}", uv.y, v);
            assert_eq!(hit.material, 0);
            assert_eq!(hit.texture_set, Some(TextureSetId(0)));
            // From inside the far side is hit from behind.
            let inside = bvh.raycast(&mesh, &Ray::new(Vec3::ZERO, dir), 0.0, 100.0, Cull::None).unwrap();
            assert!(!inside.front_face);
            assert!(inside.normal.dot(dir) < -0.99, "the viewer-facing normal points back at the origin");
            assert!((inside.surface_normal() - inside.position / r).length() < 4e-3);
            assert!(bvh.raycast(&mesh, &Ray::new(Vec3::ZERO, dir), 0.0, 100.0, Cull::Back).is_none());
        }
    }

    #[test]
    fn axis_parallel_rays_and_origins_on_box_faces() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let bvh = Bvh::build(&mesh);
        bvh.check(&mesh);
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        // Axis-parallel rays (two infinite inverse direction components) through a grid of points, including
        // points on the cube's silhouette, i.e. exactly in the faces of the node boxes.
        for (ai, &axis) in axes.iter().enumerate() {
            let (u, v) = (axes[(ai + 1) % 3], axes[(ai + 2) % 3]);
            for sign in [1.0f32, -1.0] {
                for iu in -6..=6 {
                    for iv in -6..=6 {
                        let (fu, fv) = (iu as f32 * 0.125, iv as f32 * 0.125);
                        let ray = Ray::new(axis * (3.0 * sign) + u * fu + v * fv, -axis * sign);
                        let expect = brute_raycast(&mesh, &ray, 0.0, 10.0, Cull::None);
                        let got = bvh.raycast(&mesh, &ray, 0.0, 10.0, Cull::None);
                        assert_eq!(expect.map(|e| e.0), got.map(|h| h.tri), "axis {ai}, sign {sign}, ({fu}, {fv})");
                        let inside = fu.abs() < 0.5 && fv.abs() < 0.5;
                        if inside {
                            let hit = got.expect("a ray through the cube must hit");
                            assert!((hit.t - 2.5).abs() < 1e-5);
                            assert_eq!(hit.geo_normal, axis * sign);
                            assert!(hit.front_face);
                        } else if fu.abs() > 0.5 || fv.abs() > 0.5 {
                            assert!(got.is_none());
                        }
                        assert_eq!(bvh.occluded(&mesh, &ray, 0.0, 10.0), expect.is_some());
                    }
                }
            }
        }
        // Origin exactly on a face of the root box, looking inwards, outwards and along the face.
        let on_face = Vec3::new(0.5, 0.1, 0.2);
        let inwards = bvh.raycast(&mesh, &Ray::new(on_face, Vec3::NEG_X), 0.0, 10.0, Cull::None).unwrap();
        assert_eq!(inwards.t, 0.0, "the face the origin lies on");
        assert!(inwards.front_face && inwards.geo_normal == Vec3::X);
        let skipping = bvh.raycast(&mesh, &Ray::new(on_face, Vec3::NEG_X), 1e-3, 10.0, Cull::None).unwrap();
        assert!((skipping.t - 1.0).abs() < 1e-6 && skipping.geo_normal == Vec3::NEG_X);
        assert!(bvh.raycast(&mesh, &Ray::new(on_face, Vec3::X), 1e-3, 10.0, Cull::None).is_none());
        for dir in [Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
            let ray = Ray::new(on_face, dir);
            let expect = brute_raycast(&mesh, &ray, 1e-3, 10.0, Cull::None);
            assert_eq!(expect.map(|e| e.0), bvh.raycast(&mesh, &ray, 1e-3, 10.0, Cull::None).map(|h| h.tri));
        }
        // From the centre every axis direction hits its face from behind at 0.5.
        for &axis in &axes {
            for sign in [1.0f32, -1.0] {
                let hit = bvh.raycast(&mesh, &Ray::new(Vec3::ZERO, axis * sign), 0.0, 10.0, Cull::None).unwrap();
                assert!((hit.t - 0.5).abs() < 1e-6 && !hit.front_face);
            }
        }
        // A flat mesh has a zero-thickness box: rays in its plane miss, rays across it hit, also on its border.
        let quad = mesh_of(samples::flat_quad());
        let flat = Bvh::build(&quad);
        assert!(flat.raycast(&quad, &Ray::new(Vec3::new(-1.0, 0.5, 0.0), Vec3::X), 0.0, 10.0, Cull::None).is_none());
        let hit = flat.raycast(&quad, &Ray::new(Vec3::new(0.25, 0.5, 1.0), Vec3::NEG_Z), 0.0, 10.0, Cull::None).unwrap();
        assert_eq!((hit.t, hit.position), (1.0, Vec3::new(0.25, 0.5, 0.0)));
        assert_eq!(hit.uv, Some(Vec2::new(0.25, 0.5)));
        assert!(flat.raycast(&quad, &Ray::new(Vec3::new(1.0, 0.5, 1.0), Vec3::NEG_Z), 0.0, 10.0, Cull::None).is_some());
        assert!(flat.raycast(&quad, &Ray::new(Vec3::new(1.001, 0.5, 1.0), Vec3::NEG_Z), 0.0, 10.0, Cull::None).is_none());
    }

    #[test]
    fn t_range_and_degenerate_rays() {
        let mesh = mesh_of(samples::occluded_quads());
        let bvh = Bvh::build(&mesh);
        let ray = Ray::new(Vec3::new(0.5, 0.5, 2.0), Vec3::NEG_Z);
        // Front quad at t = 2, rear quad at t = 2.5.
        let near = bvh.raycast(&mesh, &ray, 0.0, 10.0, Cull::None).unwrap();
        assert_eq!((near.t, near.tri / 2), (2.0, 0));
        let far = bvh.raycast(&mesh, &ray, 2.1, 10.0, Cull::None).unwrap();
        assert_eq!((far.t, far.tri / 2), (2.5, 1));
        // The range is inclusive at both ends.
        assert!(bvh.raycast(&mesh, &ray, 2.0, 2.0, Cull::None).is_some());
        assert!(bvh.raycast(&mesh, &ray, 2.5, 2.5, Cull::None).is_some());
        assert!(bvh.raycast(&mesh, &ray, 2.01, 2.49, Cull::None).is_none());
        assert!(bvh.raycast(&mesh, &ray, 0.0, 1.99, Cull::None).is_none());
        assert!(!bvh.occluded(&mesh, &ray, 2.6, 10.0));
        assert!(bvh.occluded(&mesh, &ray, 2.1, 2.6));
        // Beside the front quad only the rear one is hit.
        let beside = Ray::new(Vec3::new(1.25, 0.5, 2.0), Vec3::NEG_Z);
        assert_eq!(bvh.raycast(&mesh, &beside, 0.0, 10.0, Cull::None).unwrap().tri / 2, 1);
        // Degenerate rays and ranges never hit and never panic.
        let zero = Ray::new(Vec3::new(0.5, 0.5, 2.0), Vec3::ZERO);
        assert!(bvh.raycast(&mesh, &zero, 0.0, 10.0, Cull::None).is_none());
        assert!(!bvh.occluded(&mesh, &zero, 0.0, 10.0));
        let nan = Ray { origin: Vec3::new(f32::NAN, 0.5, 2.0), dir: Vec3::NEG_Z };
        assert!(bvh.raycast(&mesh, &nan, 0.0, 10.0, Cull::None).is_none());
        assert!(bvh.raycast(&mesh, &ray, 5.0, 1.0, Cull::None).is_none());
        assert!(bvh.raycast(&mesh, &ray, f32::NAN, 10.0, Cull::None).is_none());
        // An empty mesh.
        let empty = mesh_of(RawMesh::default());
        let none = Bvh::build(&empty);
        assert_eq!(none.node_count(), 0);
        assert!(none.raycast(&empty, &ray, 0.0, 10.0, Cull::None).is_none());
        assert!(!none.occluded(&empty, &ray, 0.0, 10.0));
        let mut out = vec![1];
        none.query_sphere(&empty, Vec3::ZERO, 1.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn back_face_culling() {
        let mesh = mesh_of(samples::flat_quad());
        let bvh = Bvh::build(&mesh);
        let front = Ray::new(Vec3::new(0.3, 0.3, 1.0), Vec3::NEG_Z);
        let back = Ray::new(Vec3::new(0.3, 0.3, -1.0), Vec3::Z);
        assert!(bvh.raycast(&mesh, &front, 0.0, 10.0, Cull::Back).is_some());
        assert!(bvh.raycast(&mesh, &back, 0.0, 10.0, Cull::Back).is_none());
        let hit = bvh.raycast(&mesh, &back, 0.0, 10.0, Cull::None).unwrap();
        assert!(!hit.front_face);
        assert_eq!(hit.geo_normal, Vec3::Z);
        assert_eq!(hit.normal, Vec3::NEG_Z);
        assert!(bvh.occluded(&mesh, &back, 0.0, 10.0), "occlusion ignores the facing");
        // Through the cube: culling skips nothing from outside, and from inside everything.
        let cube = mesh_of(samples::seam_cube(1.0));
        let cube_bvh = Bvh::build(&cube);
        let through = Ray::new(Vec3::new(0.1, 0.2, 3.0), Vec3::NEG_Z);
        assert!((cube_bvh.raycast(&cube, &through, 0.0, 10.0, Cull::Back).unwrap().t - 2.5).abs() < 1e-6);
        assert!(cube_bvh.raycast(&cube, &through, 2.6, 10.0, Cull::Back).is_none());
        assert!((cube_bvh.raycast(&cube, &through, 2.6, 10.0, Cull::None).unwrap().t - 3.5).abs() < 1e-6);
    }

    #[test]
    fn reversed_winding_hit() {
        let mut raw = samples::flat_quad();
        samples::reverse_winding(&mut raw);
        let mesh = mesh_of(raw);
        let bvh = Bvh::build(&mesh);
        // The winding now faces -Z and so do the (generated) shading normals.
        let origin = Vec3::new(0.25, 0.5, 1.0);
        let ray = Ray::new(origin, Vec3::NEG_Z);
        let hit = bvh.raycast(&mesh, &ray, 0.0, 10.0, Cull::None).unwrap();
        let stored = mesh.tri_normals(hit.tri);
        assert_eq!(stored, [Vec3::NEG_Z; 3]);
        assert_eq!(hit.geo_normal, Vec3::NEG_Z);
        assert!(!hit.front_face);
        assert!(hit.normal.dot(origin - hit.position) > 0.0, "normal faces the ray origin");
        assert_eq!(hit.normal, Vec3::Z);
        assert_eq!(hit.surface_normal(), stored[0], "the stored shading normal");
        assert_eq!(hit.uv, Some(Vec2::new(0.25, 0.5)));
        assert!(bvh.raycast(&mesh, &ray, 0.0, 10.0, Cull::Back).is_none());
        // Seen from the other side it is an ordinary front face.
        let behind = bvh.raycast(&mesh, &Ray::new(Vec3::new(0.25, 0.5, -1.0), Vec3::Z), 0.0, 10.0, Cull::Back).unwrap();
        assert!(behind.front_face);
        assert_eq!(behind.normal, Vec3::NEG_Z);
        assert_eq!(behind.surface_normal(), Vec3::NEG_Z);

        // Explicit normals that disagree with the winding (a reversed cube keeps its outward normals): the flip of
        // `normal` is still decided by the winding alone, and `surface_normal()` returns the stored normal exactly.
        let mut raw = samples::seam_cube(1.0);
        samples::reverse_winding(&mut raw);
        let cube = mesh_of(raw);
        let cube_bvh = Bvh::build(&cube);
        let from_front = Ray::new(Vec3::new(0.1, 0.2, 3.0), Vec3::NEG_Z);
        let outside = cube_bvh.raycast(&cube, &from_front, 0.0, 10.0, Cull::None).unwrap();
        assert_eq!(outside.geo_normal, Vec3::NEG_Z);
        assert!(!outside.front_face);
        assert_eq!(cube.tri_normals(outside.tri), [Vec3::Z; 3]);
        assert_eq!(outside.normal, Vec3::NEG_Z);
        assert_eq!(outside.surface_normal(), Vec3::Z);
        assert!(cube_bvh.raycast(&cube, &from_front, 0.0, 3.0, Cull::Back).is_none());
    }

    #[test]
    fn hits_without_uvs_and_with_wrapping_uvs() {
        let tetra = mesh_of(samples::no_uv_tetra());
        let bvh = Bvh::build(&tetra);
        let hit = bvh.raycast(&tetra, &Ray::new(Vec3::splat(3.0), Vec3::splat(-1.0)), 0.0, 10.0, Cull::Back).unwrap();
        assert_eq!((hit.uv, hit.texture_set, hit.material), (None, None, 0));
        assert!((hit.normal.length() - 1.0).abs() < 1e-5);
        // UVs are reported unwrapped (tile-normalized per triangle, not folded into 0..1).
        let wrapped = mesh_of(samples::wrapped_quad());
        let bvh = Bvh::build(&wrapped);
        // Quad A has u = 0.5 + x, v = 1.5 - y; its first triangle was shifted by the tile (1, 1).
        let hit = bvh.raycast(&wrapped, &Ray::new(Vec3::new(0.2, 0.1, 1.0), Vec3::NEG_Z), 0.0, 10.0, Cull::None).unwrap();
        let uv = hit.uv.unwrap();
        assert_eq!(hit.tri, 0);
        assert!((uv - Vec2::new(-0.3, 0.4)).length() < 1e-5, "{uv:?}");
    }

    /// A `MeshData` assembled by hand from a triangle soup (no UVs), bypassing `normalize` and its degenerate filter.
    #[allow(clippy::single_range_in_vec_init)] // one triangle range per material, and there is one material
    fn hand_built(tris: &[[Vec3; 3]]) -> MeshData {
        let positions: Vec<Vec3> = tris.iter().flatten().copied().collect();
        let n = tris.len() as u32;
        MeshData {
            normals: vec![Vec3::Y; positions.len()],
            uvs: vec![Vec2::ZERO; positions.len()],
            indices: (0..n * 3).collect(),
            tri_material: vec![0; n as usize],
            tri_flags: vec![TriFlags::NO_UV; n as usize],
            materials: vec![crate::mesh::MaterialSlot {
                name: "m".into(),
                base_color: [1.0; 4],
                double_sided: false,
                texture_set: None,
            }],
            material_ranges: vec![0..n],
            material_uv_ranges: vec![0..0],
            material_nouv_ranges: vec![0..n],
            sets: Vec::new(),
            bounds: crate::math::Aabb::from_points(&positions),
            uv_hash: 0,
            positions,
        }
    }

    #[test]
    fn pathological_inputs_keep_the_tree_shallow() {
        // Centroids in a geometric progression along each axis in turn: every spatial split can only peel off the
        // few outermost triangles, so an unbounded SAH build would produce a chain deeper than the traversal stack.
        let mut tris = Vec::new();
        let tiny = 1e-18f32;
        for axis in 0..3 {
            for i in 0..62 {
                let mut c = Vec3::splat(tiny);
                c[axis] = tiny * 10f32.powf(0.58 * (i + 1) as f32);
                let s = c[axis] * 0.05;
                tris.push([c, c + Vec3::X * s, c + Vec3::Y * s]);
            }
        }
        let mesh = hand_built(&tris);
        let bvh = Bvh::build(&mesh);
        bvh.check(&mesh);
        let (depth, _, _) = bvh.shape();
        assert!(depth > 20, "the input is supposed to produce a deep tree, got {depth}");
        // Queries still agree with brute force on such a tree.
        for (i, t) in tris.iter().enumerate().step_by(7) {
            let centre = (t[0] + t[1] + t[2]) / 3.0;
            let far = Vec3::new(centre.x, centre.y, centre.z + 1.0);
            let ray = Ray::new(far, Vec3::NEG_Z);
            let expect = brute_raycast(&mesh, &ray, 0.0, f32::INFINITY, Cull::None);
            let got = bvh.raycast(&mesh, &ray, 0.0, f32::INFINITY, Cull::None);
            assert_eq!(expect.map(|e| e.0), got.map(|h| h.tri), "triangle {i}");
            let hit = got.expect("the ray is aimed at a triangle centre");
            assert_eq!((hit.uv, hit.texture_set), (None, None));
        }
        assert!(use_sah(410, 0) && use_sah(1 << 20, 30));
        assert!(!use_sah(5, 60) && !use_sah(usize::MAX, 0));

        // Many coincident triangles: every centroid falls into one bin (median split), and the nearest-hit tie goes
        // to the smallest triangle id.
        let mut raw = samples::flat_quad();
        for _ in 0..300 {
            raw.triangles.push(RawTriangle {
                positions: [Vec3::new(-5.0, 0.0, 0.0), Vec3::new(-4.0, 0.0, 0.0), Vec3::new(-5.0, 1.0, 0.0)],
                normals: None,
                uvs: None,
                material: 0,
            });
        }
        let mesh = mesh_of(raw);
        assert_eq!(mesh.tri_count(), 302);
        let bvh = Bvh::build(&mesh);
        bvh.check(&mesh);
        let ray = Ray::new(Vec3::new(-4.75, 0.25, 1.0), Vec3::NEG_Z);
        let hit = bvh.raycast(&mesh, &ray, 0.0, 10.0, Cull::None).unwrap();
        assert_eq!(Some((hit.tri, hit.t)), brute_raycast(&mesh, &ray, 0.0, 10.0, Cull::None));
        assert_eq!(hit.tri, 2);
        assert!(bvh.occluded(&mesh, &ray, 0.0, 10.0));
        let mut out = Vec::new();
        bvh.query_sphere(&mesh, Vec3::new(-4.75, 0.25, 0.0), 0.01, &mut out);
        assert_eq!(out, (2..302).collect::<Vec<_>>());
    }

    #[test]
    fn closest_point_regions() {
        let (a, b, c) = (Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 2.0, 0.0));
        let cp = |p: [f32; 3]| closest_point_on_triangle(Vec3::from_array(p), a, b, c);
        // Face region: projection onto the plane.
        assert_eq!(cp([0.5, 0.5, 3.0]), Vec3::new(0.5, 0.5, 0.0));
        assert_eq!(cp([0.5, 0.5, -3.0]), Vec3::new(0.5, 0.5, 0.0));
        // Vertex regions.
        assert_eq!(cp([-1.0, -1.0, 1.0]), a);
        assert_eq!(cp([3.0, -1.0, 0.0]), b);
        assert_eq!(cp([-1.0, 5.0, 0.0]), c);
        // Edge regions.
        assert_eq!(cp([1.0, -2.0, 0.5]), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(cp([-2.0, 1.5, 0.5]), Vec3::new(0.0, 1.5, 0.0));
        assert!((cp([2.0, 2.0, 0.0]) - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-6);
        // A point on the triangle is its own closest point.
        assert_eq!(cp([0.25, 1.0, 0.0]), Vec3::new(0.25, 1.0, 0.0));
        // Degenerate triangles do not produce NaN.
        let p = Vec3::new(1.0, 1.0, 1.0);
        assert_eq!(closest_point_on_triangle(p, a, a, a), a);
        let on_segment = closest_point_on_triangle(p, a, b, b);
        assert!(on_segment.is_finite() && (on_segment - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6);
        let collinear = closest_point_on_triangle(p, a, b, b * 2.0);
        assert!(collinear.is_finite() && (collinear - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6);

        // Random points against a dense sampling of the triangle.
        let mut rng = XorShift::new(31);
        let (a, b, c) = (Vec3::new(0.3, -0.2, 0.1), Vec3::new(1.7, 0.4, -0.6), Vec3::new(-0.5, 1.2, 0.9));
        for _ in 0..200 {
            let p = Vec3::new(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5) * 6.0;
            let q = closest_point_on_triangle(p, a, b, c);
            let d = (q - p).length();
            let mut best = f32::INFINITY;
            for i in 0..=60 {
                for j in 0..=(60 - i) {
                    let (u, v) = (i as f32 / 60.0, j as f32 / 60.0);
                    best = best.min((a + (b - a) * u + (c - a) * v - p).length());
                }
            }
            assert!(d <= best + 1e-5, "closest point is further than a sample: {d} > {best}");
            assert!(best <= d + 0.05, "sampling found nothing near the closest point");
            // The result lies in the triangle's plane.
            let n = (b - a).cross(c - a).normalize();
            assert!((q - a).dot(n).abs() < 1e-5);
        }
    }
}
