// Written by the scaffold S1 of round 4, complete; no later owner (a defect is fixed as a listed cross-package edit).
// A brush shape as a grid of signed distances (FEATURE_BRUSH 4.1, 5.6, 5.7 steps 8-12, 5.8): the lookups `F`
// (`sample`), `T` (`thickness`), `G` (`outline`) and `T2` (`outline_thickness`), the local-thickness transform, the
// middle slice, the sections, the built-in Cube and Cylinder and the formula test seams. No mesh code: the bake of a
// model (`tip_bake.rs`) computes the distances and builds the field with `from_distances` /
// `outline_from_distances`, which run the same thickness, slice and summary code as the built-in shapes.
//
// Units: "tip units", in which the solid fits inside the unit ball around the origin; +Y is "up". Distances are
// stored as `i16 = round(d / cell * 256)`, saturating at +-127.99 cells; local thickness as `u8` in half cells.
// Node `(i, j, k)` lies at `origin + (i, j, k) * cell`; index `i + nx * (j + ny * k)` (x fastest).

use std::fmt;

use glam::{Vec2, Vec3};
use rayon::prelude::*;

use crate::math::{Aabb, fnv1a64};

/// Largest stored distance, in 1/256 cells (127.99 cells).
const SATURATED: i16 = i16::MAX;
/// Lattice size of the principal sections (`principal_sections`).
const SECTION_LATTICE: u32 = 128;
/// Largest number of nodes per axis of a baked grid (FEATURE_BRUSH 5.7 step 3).
pub const TIP_MAX_NODES: u32 = 128;

/// What a field is and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct TipReport {
    /// Triangles of the model the field was baked from (0 for a built-in or formula shape).
    pub triangles: u32,
    /// Nodes per axis of the three-dimensional grid (`[0, 0, 0]` for a flat outline).
    pub grid: [u32; 3],
    /// Share of the grid nodes that lie inside (of the slice nodes for a flat outline).
    pub inside_fraction: f32,
    /// Some part is thinner than the grid can show (`inradius` under 2 cells).
    pub thin: bool,
    /// The file held more than the closed solid; only the solid is used (FEATURE_BRUSH 5.7 step 6).
    pub partial: bool,
    /// A flat outline without a volume (5.7b); usable Through only.
    pub flat: bool,
    /// The file axis that became tip +Y: 1 = Y (unchanged), 0 = X, 2 = Z (5.7 step 11).
    pub up_axis: u8,
    /// Extent along tip Y / largest extent along tip X or Z; 0 for a flat outline.
    pub height_ratio: f32,
    /// Openings of the model that the bake closed (plan section 7 item 5, `CloseMode`); 0 when it had none.
    pub openings_closed: u32,
    /// The model has openings and also closed parts (as `TipError::HasOpenings::closed_parts`), so
    /// `CloseMode::ClosedPartsOnly` can be offered; false when it had no openings. Filled by `tip_bake` (B1).
    pub closed_parts: bool,
}

impl Default for TipReport {
    fn default() -> TipReport {
        TipReport {
            triangles: 0,
            grid: [0; 3],
            inside_fraction: 0.0,
            thin: false,
            partial: false,
            flat: false,
            up_axis: 1,
            height_ratio: 0.0,
            openings_closed: 0,
            closed_parts: false,
        }
    }
}

/// The middle slice y = 0 (or the whole shape of a flat outline): two-dimensional distances and local thickness.
#[derive(Clone, PartialEq)]
struct Slice2 {
    /// Nodes along x and along z.
    dims: [u32; 2],
    /// Position (x, z) of node (0, 0).
    origin: Vec2,
    cell: f32,
    dist: Vec<i16>,
    thick: Vec<u8>,
}

/// A solid as a grid of signed distances in tip units (the shape fits inside the unit ball around the origin).
/// Negative inside. Immutable; shared through `Arc`; `Send + Sync`.
#[derive(Clone, PartialEq)]
pub struct TipField {
    /// Nodes per axis; `[0, 0, 0]` for a flat outline (no three-dimensional grid).
    dims: [u32; 3],
    origin: Vec3,
    cell: f32,
    dist: Vec<i16>,
    thick: Vec<u8>,
    slice: Slice2,
    inradius: f32,
    bound: f32,
    bounds: Aabb,
    sections: [Vec<[Vec2; 2]>; 3],
    report: TipReport,
    hash: u64,
}

impl fmt::Debug for TipField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TipField")
            .field("dims", &self.dims)
            .field("slice_dims", &self.slice.dims)
            .field("cell", &self.cell)
            .field("inradius", &self.inradius)
            .field("bound", &self.bound)
            .field("report", &self.report)
            .field("hash", &format_args!("{:016x}", self.hash))
            .finish()
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Quantization and the lower-envelope transform
// ---------------------------------------------------------------------------------------------------------------------

/// The slack of step 9 in cells (contract request B1-1).
const THICKNESS_SLACK_CELLS: f64 = 1.0;

/// A distance in cells as stored: `round(d * 256)`, saturating at +-127.99 cells (NaN counts as far outside).
fn quantize(d_cells: f64) -> i16 {
    if d_cells.is_nan() {
        return SATURATED;
    }
    (d_cells * 256.0).round().clamp(-(SATURATED as f64), SATURATED as f64) as i16
}

/// A local thickness in cells as stored: half cells, at most 255.
fn quantize_thick(t_cells: f64) -> u8 {
    if t_cells.is_nan() { 0 } else { (t_cells * 2.0).round().clamp(0.0, 255.0) as u8 }
}

/// One-dimensional lower envelope of parabolas (Felzenszwalb and Huttenlocher): `out[q] = min_p (q - p)^2 + f[p]`
/// and the minimizing `p`. Entries of `f` that are not finite take no part; without any finite entry every `out` is
/// infinite and every `arg` is `u32::MAX`.
fn envelope_1d(f: &[f64], out: &mut [f64], arg: &mut [u32]) {
    let n = f.len();
    let mut v: Vec<usize> = Vec::with_capacity(n);
    let mut z: Vec<f64> = Vec::with_capacity(n + 1);
    for q in 0..n {
        if !f[q].is_finite() {
            continue;
        }
        let fq = f[q] + (q * q) as f64;
        loop {
            let Some(&p) = v.last() else {
                v.push(q);
                z.clear();
                z.push(f64::NEG_INFINITY);
                break;
            };
            let s = (fq - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
            if s <= *z.last().unwrap_or(&f64::NEG_INFINITY) && v.len() > 1 {
                v.pop();
                z.pop();
            } else if s <= *z.last().unwrap_or(&f64::NEG_INFINITY) {
                // The only parabola is dominated everywhere: replace it.
                v.clear();
                v.push(q);
                z.clear();
                z.push(f64::NEG_INFINITY);
                break;
            } else {
                v.push(q);
                z.push(s);
                break;
            }
        }
    }
    if v.is_empty() {
        out.fill(f64::INFINITY);
        arg.fill(u32::MAX);
        return;
    }
    let mut k = 0;
    for q in 0..n {
        while k + 1 < v.len() && z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let d = q as f64 - p as f64;
        out[q] = d * d + f[p];
        arg[q] = p as u32;
    }
}

/// The separable transform of FEATURE_BRUSH 5.7 steps 8 and 9 on a grid of 2 or 3 axes (x fastest): for every node
/// `x`, `min over y (|x - y|^2 + init[y])` in squared cells and the index of the minimizing node. Nodes with a
/// non-finite `init` take no part. Every grid line is independent, so the result does not depend on the number of
/// threads.
pub(crate) fn power_transform(dims: &[usize], init: &[f64]) -> (Vec<f64>, Vec<u32>) {
    let total = init.len();
    debug_assert_eq!(dims.iter().product::<usize>(), total);
    let mut val = init.to_vec();
    let mut arg: Vec<u32> = (0..total).map(|i| if init[i].is_finite() { i as u32 } else { u32::MAX }).collect();
    let mut stride = 1usize;
    for &n in dims {
        if n == 0 {
            return (val, arg);
        }
        let lines = total / n;
        let (val_ref, arg_ref) = (&val, &arg);
        let results: Vec<(Vec<f64>, Vec<u32>)> = (0..lines)
            .into_par_iter()
            .map(|line| {
                let base = (line % stride) + (line / stride) * stride * n;
                let f: Vec<f64> = (0..n).map(|q| val_ref[base + q * stride]).collect();
                let mut out = vec![f64::INFINITY; n];
                let mut p = vec![u32::MAX; n];
                envelope_1d(&f, &mut out, &mut p);
                let src = (0..n)
                    .map(|q| if p[q] == u32::MAX { u32::MAX } else { arg_ref[base + p[q] as usize * stride] })
                    .collect();
                (out, src)
            })
            .collect();
        for (line, (out, src)) in results.into_iter().enumerate() {
            let base = (line % stride) + (line / stride) * stride * n;
            for q in 0..n {
                val[base + q * stride] = out[q];
                arg[base + q * stride] = src[q];
            }
        }
        stride *= n;
    }
    (val, arg)
}

/// Local thickness (FEATURE_BRUSH 5.7 step 9) of a grid of 2 or 3 axes, from its stored distances: for every inside
/// node the depth of the inside node `y*` that minimises `|x - y|^2 - (depth(y) + 1)^2` (one cell of slack, contract
/// request B1-1: without it a node near a facet vertex of a baked shape wins against the deep node whose ball should
/// cover it), never less than the node's own depth; an outside node next to an inside node copies the largest value
/// of its inside neighbours; every other outside node holds the inradius. Half cells.
pub(crate) fn thickness_grid(dims: &[usize], dist: &[i16]) -> Vec<u8> {
    let depth = |i: usize| -(dist[i] as f64) / 256.0;
    let init: Vec<f64> = (0..dist.len())
        .map(|i| if dist[i] < 0 { -((depth(i) + THICKNESS_SLACK_CELLS) * (depth(i) + THICKNESS_SLACK_CELLS)) } else { f64::INFINITY })
        .collect();
    let (_, arg) = power_transform(dims, &init);
    let inradius = (0..dist.len()).filter(|&i| dist[i] < 0).map(depth).fold(0.0f64, f64::max);
    let inside_t: Vec<f64> =
        (0..dist.len()).map(|i| if dist[i] < 0 && arg[i] != u32::MAX { depth(arg[i] as usize) } else { -1.0 }).collect();
    let strides: Vec<usize> = dims.iter().scan(1usize, |s, &n| {
        let here = *s;
        *s *= n;
        Some(here)
    }).collect();
    (0..dist.len())
        .into_par_iter()
        .map(|i| {
            if dist[i] < 0 {
                return quantize_thick(inside_t[i].max(depth(i)));
            }
            // The largest value of the inside neighbours (6 or 4 of them).
            let mut best = -1.0f64;
            let mut rest = i;
            for (axis, &n) in dims.iter().enumerate() {
                let c = rest % n;
                rest /= n;
                let s = strides[axis];
                if c > 0 && dist[i - s] < 0 {
                    best = best.max(inside_t[i - s]);
                }
                if c + 1 < n && dist[i + s] < 0 {
                    best = best.max(inside_t[i + s]);
                }
            }
            quantize_thick(if best >= 0.0 { best } else { inradius })
        })
        .collect()
}

/// Signed distances (stored) of a grid of 2 or 3 axes from its border nodes (FEATURE_BRUSH 5.7 step 8):
/// `d(x) = sqrt(min over border nodes b (|x - x_b|^2 + f_b^2))`, negative for inside nodes. `border[i]` = `f_b` in
/// cells for a border node, NaN for every other node.
pub(crate) fn distances_from_border(dims: &[usize], inside: &[bool], border: &[f32]) -> Vec<i16> {
    let init: Vec<f64> =
        border.iter().map(|&f| if f.is_finite() { (f as f64) * (f as f64) } else { f64::INFINITY }).collect();
    let (val, _) = power_transform(dims, &init);
    val.par_iter()
        .zip(inside.par_iter())
        .map(|(&v, &inside)| {
            let d = if v.is_finite() { v.max(0.0).sqrt() } else { f64::INFINITY };
            if inside { quantize(-d) } else { quantize(d) }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Grid layout of the built-in shapes
// ---------------------------------------------------------------------------------------------------------------------

/// A grid symmetric about the origin for a shape with half extents `half` (tip units): cubic cells of
/// `2 * max(half) / 122` (the cell of FEATURE_BRUSH 5.7 step 3) and an odd number of nodes per axis with at least 2
/// cells of margin, so the origin is a node. Returns `(dims, origin, cell)`.
fn symmetric_grid(half: Vec3) -> ([u32; 3], Vec3, f32) {
    let cell = 2.0 * half.max_element() / 122.0;
    let per_axis = |h: f32| -> u32 {
        let n = (h / cell).ceil() as u32 + 2;
        (2 * n + 1).clamp(9, TIP_MAX_NODES - 1)
    };
    let dims = [per_axis(half.x), per_axis(half.y), per_axis(half.z)];
    let origin = Vec3::new(
        -((dims[0] / 2) as f32) * cell,
        -((dims[1] / 2) as f32) * cell,
        -((dims[2] / 2) as f32) * cell,
    );
    (dims, origin, cell)
}

// ---------------------------------------------------------------------------------------------------------------------
// TipField
// ---------------------------------------------------------------------------------------------------------------------

impl TipField {
    /// Half side of the built-in Cube: its corners lie on the unit ball.
    pub const CUBE_HALF_SIDE: f32 = 0.577_350_3;
    /// Radius and half height of the built-in Cylinder (axis Y): its rims lie on the unit ball.
    pub const CYLINDER_RADIUS: f32 = std::f32::consts::FRAC_1_SQRT_2;
    pub const CYLINDER_HALF_HEIGHT: f32 = std::f32::consts::FRAC_1_SQRT_2;

    /// The built-in Cube: half side `1/sqrt(3)`, written from the exact box distance at the nodes of a grid of
    /// cell `side / 122` (FEATURE_BRUSH 5.8).
    pub fn cube() -> TipField {
        let h = TipField::CUBE_HALF_SIDE;
        let (dims, origin, cell) = symmetric_grid(Vec3::splat(h));
        TipField::from_fn(dims, origin, cell, move |p| {
            let q = p.abs() - Vec3::splat(h);
            q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
        })
    }

    /// The built-in Cylinder: axis Y, radius and half height `1/sqrt(2)`, from the exact capped-cylinder distance.
    pub fn cylinder() -> TipField {
        let (r, h) = (TipField::CYLINDER_RADIUS, TipField::CYLINDER_HALF_HEIGHT);
        let (dims, origin, cell) = symmetric_grid(Vec3::new(r, h, r));
        TipField::from_fn(dims, origin, cell, move |p| {
            let d = Vec2::new(Vec2::new(p.x, p.z).length() - r, p.y.abs() - h);
            d.max_element().min(0.0) + d.max(Vec2::ZERO).length()
        })
    }

    /// Test seam and formula shapes: a field from a signed distance `f` (tip units, negative inside) evaluated at
    /// the nodes of the grid `dims` (each clamped to `2..=2 * TIP_MAX_NODES`), `origin`, `cell`. The thickness grid, the
    /// middle slice and the summary come from the same code as a baked field. `f` should change by at most the
    /// distance moved (an exact or conservative distance), so that the neighbour property of 5.6 holds.
    pub fn from_fn(dims: [u32; 3], origin: Vec3, cell: f32, f: impl Fn(Vec3) -> f32 + Sync) -> TipField {
        let dims = dims.map(|n| n.clamp(2, 2 * TIP_MAX_NODES));
        let cell = if cell.is_finite() && cell > 0.0 { cell } else { 1.0 / 64.0 };
        let [nx, ny, nz] = dims.map(|n| n as usize);
        let dist: Vec<i16> = (0..nx * ny * nz)
            .into_par_iter()
            .map(|i| {
                let (x, y, z) = (i % nx, (i / nx) % ny, i / (nx * ny));
                let p = origin + Vec3::new(x as f32, y as f32, z as f32) * cell;
                quantize(f(p) as f64 / cell as f64)
            })
            .collect();
        TipField::from_distances(dims, origin, cell, dist, TipReport::default())
    }

    /// Test seam and formula outlines: a flat outline from a two-dimensional signed distance `f` (in the plane
    /// (x, z), tip units) evaluated at the nodes of the slice grid. Usable Through only.
    pub fn outline_from_fn(dims: [u32; 2], origin: Vec2, cell: f32, f: impl Fn(Vec2) -> f32 + Sync) -> TipField {
        let dims = dims.map(|n| n.clamp(2, TIP_MAX_NODES * 2));
        let cell = if cell.is_finite() && cell > 0.0 { cell } else { 1.0 / 64.0 };
        let [nx, nz] = dims.map(|n| n as usize);
        let dist: Vec<i16> = (0..nx * nz)
            .into_par_iter()
            .map(|i| {
                let p = origin + Vec2::new((i % nx) as f32, (i / nx) as f32) * cell;
                quantize(f(p) as f64 / cell as f64)
            })
            .collect();
        TipField::outline_from_distances(dims, origin, cell, dist, TipReport::default())
    }

    /// A field from finished stored distances (`i16` in 1/256 cells, negative inside) of a three-dimensional grid,
    /// for the bake (FEATURE_BRUSH 5.7: steps 9, 10 and 12 run here). `base` carries what the bake knows
    /// (`triangles`, `partial`, `up_axis`, `openings_closed`, `closed_parts`); `grid`, `inside_fraction`, `thin`, `flat` and
    /// `height_ratio` are computed.
    pub(crate) fn from_distances(dims: [u32; 3], origin: Vec3, cell: f32, dist: Vec<i16>, base: TipReport) -> TipField {
        let udims = dims.map(|n| n as usize);
        debug_assert_eq!(dist.len(), udims.iter().product::<usize>());
        let thick = thickness_grid(&udims, &dist);
        let mut field = TipField {
            dims,
            origin,
            cell,
            dist,
            thick,
            slice: Slice2 { dims: [0, 0], origin: Vec2::ZERO, cell, dist: Vec::new(), thick: Vec::new() },
            inradius: 0.0,
            bound: 0.0,
            bounds: Aabb::EMPTY,
            sections: [Vec::new(), Vec::new(), Vec::new()],
            report: base,
            hash: 0,
        };
        field.slice = field.middle_slice();
        field.summarize();
        field
    }

    /// A flat outline from finished stored distances of the slice grid (FEATURE_BRUSH 5.7b), for the bake.
    pub(crate) fn outline_from_distances(
        dims: [u32; 2],
        origin: Vec2,
        cell: f32,
        dist: Vec<i16>,
        base: TipReport,
    ) -> TipField {
        let udims = dims.map(|n| n as usize);
        debug_assert_eq!(dist.len(), udims[0] * udims[1]);
        let thick = thickness_grid(&udims, &dist);
        let mut field = TipField {
            dims: [0; 3],
            origin: Vec3::ZERO,
            cell,
            dist: Vec::new(),
            thick: Vec::new(),
            slice: Slice2 { dims, origin, cell, dist, thick },
            inradius: 0.0,
            bound: 0.0,
            bounds: Aabb::EMPTY,
            sections: [Vec::new(), Vec::new(), Vec::new()],
            report: base,
            hash: 0,
        };
        field.summarize();
        field
    }

    // ----- lookups -----

    /// Signed distance to the skin at a point in tip units; defined for every `q` (`F` of FEATURE_BRUSH 5.6): the
    /// trilinear blend of the 8 nodes around the clamped point plus the distance from `q` to the clamped point. For
    /// a flat outline: `outline(q.x, q.z)`.
    pub fn sample(&self, q: Vec3) -> f32 {
        if self.is_flat() {
            return self.outline(q.x, q.z);
        }
        let (value, outside) = self.trilinear(&self.dist, q);
        value * self.cell / 256.0 + outside
    }

    /// Local thickness at `q` in tip units, never less than the depth of `q` (`T` of FEATURE_BRUSH 5.2). For a flat
    /// outline: `outline_thickness(q.x, q.z)`.
    pub fn thickness(&self, q: Vec3) -> f32 {
        if self.is_flat() {
            return self.outline_thickness(q.x, q.z);
        }
        let (value, _) = self.trilinear(&self.thick, q);
        (value * self.cell / 2.0).max(-self.sample(q))
    }

    /// Signed distance to the outline of the middle slice y = 0, in the plane (`G` of FEATURE_BRUSH 5.6).
    pub fn outline(&self, x: f32, z: f32) -> f32 {
        let (value, outside) = self.bilinear(&self.slice.dist, Vec2::new(x, z));
        value * self.slice.cell / 256.0 + outside
    }

    /// Local thickness of the outline at (x, z), never less than the depth (`T2`).
    pub fn outline_thickness(&self, x: f32, z: f32) -> f32 {
        let (value, _) = self.bilinear(&self.slice.thick, Vec2::new(x, z));
        (value * self.slice.cell / 2.0).max(-self.outline(x, z))
    }

    /// Box `[min, max]` of the outline in (x, z): the inside nodes of the slice grown by one cell (an empty box at
    /// the origin when the slice holds no inside node).
    pub fn outline_bounds(&self) -> [Vec2; 2] {
        let s = &self.slice;
        let mut lo = Vec2::splat(f32::INFINITY);
        let mut hi = Vec2::splat(f32::NEG_INFINITY);
        for (i, &d) in s.dist.iter().enumerate() {
            if d < 0 {
                let p = s.origin + Vec2::new((i % s.dims[0] as usize) as f32, (i / s.dims[0] as usize) as f32) * s.cell;
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        if lo.x > hi.x { [Vec2::ZERO, Vec2::ZERO] } else { [lo - Vec2::splat(s.cell), hi + Vec2::splat(s.cell)] }
    }

    /// True for a flat outline without a volume (FEATURE_BRUSH 5.7b): usable Through only.
    pub fn is_flat(&self) -> bool {
        self.dist.is_empty()
    }

    /// Largest depth inside, tip units.
    pub fn inradius(&self) -> f32 {
        self.inradius
    }

    /// Radius of a ball around the origin that holds the zero level: the largest distance of an inside node plus
    /// `sqrt(3) * cell`, plus the largest depth of an inside node on the grid's border (a part cut off there).
    pub fn bound(&self) -> f32 {
        self.bound
    }

    /// Box of the inside nodes, grown by one cell and by the largest depth of an inside node on the grid's border,
    /// tip units. For a flat outline: the outline's box in x and z (grown the same way) and one cell either way in y.
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// Edge length of one grid cell, tip units.
    pub fn cell(&self) -> f32 {
        self.cell
    }

    /// Nodes per axis of the three-dimensional grid (`[0, 0, 0]` for a flat outline).
    pub fn dims(&self) -> [u32; 3] {
        self.dims
    }

    pub fn report(&self) -> &TipReport {
        &self.report
    }

    /// FNV-1a 64 over the dimensions and every grid (distances, thickness, slice).
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Outline segments of the cuts x = 0 (plane coordinates (y, z)), y = 0 ((x, z)) and z = 0 ((x, y)), for the wire
    /// cursor and the preview.
    pub fn principal_sections(&self) -> &[Vec<[Vec2; 2]>; 3] {
        &self.sections
    }

    /// Outline of the cut with the plane through `origin` spanned by the unit vectors `u`, `v`: marching squares on
    /// an `(n+1) x (n+1)` lattice over `[-bound, bound]^2`, as segments in plane coordinates (point =
    /// `origin + a * u + b * v`). The end points of neighbouring segments are bit-identical, so a closed outline
    /// gives closed loops.
    pub fn section(&self, origin: Vec3, u: Vec3, v: Vec3, n: u32) -> Vec<[Vec2; 2]> {
        let n = n.clamp(2, 4096) as usize;
        let b = self.bound.max(self.cell);
        let coord = |i: usize| -b + 2.0 * b * i as f32 / n as f32;
        let values: Vec<f32> = (0..(n + 1) * (n + 1))
            .into_par_iter()
            .map(|idx| {
                let (i, j) = (idx % (n + 1), idx / (n + 1));
                self.sample(origin + u * coord(i) + v * coord(j))
            })
            .collect();
        let at = |i: usize, j: usize| values[i + j * (n + 1)];
        // The crossing on the lattice edge from (i0, j0) to (i1, j1), always computed in the same direction.
        let crossing = |i0: usize, j0: usize, i1: usize, j1: usize| -> Vec2 {
            let (s0, s1) = (at(i0, j0), at(i1, j1));
            let t = if s0 != s1 { (s0 / (s0 - s1)).clamp(0.0, 1.0) } else { 0.5 };
            let p0 = Vec2::new(coord(i0), coord(j0));
            let p1 = Vec2::new(coord(i1), coord(j1));
            p0 + (p1 - p0) * t
        };
        let mut out = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let inside = [at(i, j) < 0.0, at(i + 1, j) < 0.0, at(i + 1, j + 1) < 0.0, at(i, j + 1) < 0.0];
                let bottom = (inside[0] != inside[1]).then(|| crossing(i, j, i + 1, j));
                let right = (inside[1] != inside[2]).then(|| crossing(i + 1, j, i + 1, j + 1));
                let top = (inside[3] != inside[2]).then(|| crossing(i, j + 1, i + 1, j + 1));
                let left = (inside[0] != inside[3]).then(|| crossing(i, j, i, j + 1));
                match (bottom, right, top, left) {
                    (Some(b), Some(r), Some(t), Some(l)) => {
                        let centre = (at(i, j) + at(i + 1, j) + at(i + 1, j + 1) + at(i, j + 1)) / 4.0;
                        if (centre < 0.0) == inside[0] {
                            out.push([b, r]);
                            out.push([l, t]);
                        } else {
                            out.push([l, b]);
                            out.push([r, t]);
                        }
                    }
                    (b, r, t, l) => {
                        let ends: Vec<Vec2> = [b, r, t, l].into_iter().flatten().collect();
                        if let [p, q] = ends[..] {
                            out.push([p, q]);
                        }
                    }
                }
            }
        }
        out
    }

    // ----- internals -----

    /// Trilinear blend of a node grid at `q` (the point clamped into the grid box) and the distance from `q` to the
    /// clamped point. Grid units (the caller scales).
    fn trilinear<T: Copy + Into<f32>>(&self, grid: &[T], q: Vec3) -> (f32, f32) {
        let [nx, ny, nz] = self.dims.map(|n| n as usize);
        let max = Vec3::new((nx - 1) as f32, (ny - 1) as f32, (nz - 1) as f32);
        let raw = (q - self.origin) / self.cell;
        let g = if raw.is_finite() { raw.clamp(Vec3::ZERO, max) } else { Vec3::ZERO };
        let outside = if raw.is_finite() { ((raw - g) * self.cell).length() } else { f32::INFINITY };
        let i0 = g.floor().min(max - Vec3::ONE).max(Vec3::ZERO);
        let t = g - i0;
        let (x, y, z) = (i0.x as usize, i0.y as usize, i0.z as usize);
        let v = |dx: usize, dy: usize, dz: usize| -> f32 { grid[(x + dx) + nx * ((y + dy) + ny * (z + dz))].into() };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let c00 = lerp(v(0, 0, 0), v(1, 0, 0), t.x);
        let c10 = lerp(v(0, 1, 0), v(1, 1, 0), t.x);
        let c01 = lerp(v(0, 0, 1), v(1, 0, 1), t.x);
        let c11 = lerp(v(0, 1, 1), v(1, 1, 1), t.x);
        (lerp(lerp(c00, c10, t.y), lerp(c01, c11, t.y), t.z), outside)
    }

    /// Bilinear blend of a slice grid at `p` = (x, z), as `trilinear`.
    fn bilinear<T: Copy + Into<f32>>(&self, grid: &[T], p: Vec2) -> (f32, f32) {
        let s = &self.slice;
        let [nx, nz] = s.dims.map(|n| n as usize);
        if nx < 2 || nz < 2 || grid.len() < nx * nz {
            return (SATURATED as f32, 0.0);
        }
        let max = Vec2::new((nx - 1) as f32, (nz - 1) as f32);
        let raw = (p - s.origin) / s.cell;
        let g = if raw.is_finite() { raw.clamp(Vec2::ZERO, max) } else { Vec2::ZERO };
        let outside = if raw.is_finite() { ((raw - g) * s.cell).length() } else { f32::INFINITY };
        let i0 = g.floor().min(max - Vec2::ONE).max(Vec2::ZERO);
        let t = g - i0;
        let (x, z) = (i0.x as usize, i0.y as usize);
        let v = |dx: usize, dz: usize| -> f32 { grid[(x + dx) + nx * (z + dz)].into() };
        let a = v(0, 0) + (v(1, 0) - v(0, 0)) * t.x;
        let b = v(0, 1) + (v(1, 1) - v(0, 1)) * t.x;
        (a + (b - a) * t.y, outside)
    }

    /// The middle slice y = 0 (FEATURE_BRUSH 5.7 step 10): the sign of `F` on the plane, sampled on a square lattice
    /// with the grid's cell over the grid's x and z extent; the crossing on every lattice link between two signs by
    /// linear interpolation; the distance of each border node to its crossings; the two-dimensional transform and
    /// local thickness.
    fn middle_slice(&self) -> Slice2 {
        let dims = [self.dims[0], self.dims[2]];
        let origin = Vec2::new(self.origin.x, self.origin.z);
        let cell = self.cell;
        let [nx, nz] = dims.map(|n| n as usize);
        let values: Vec<f32> = (0..nx * nz)
            .into_par_iter()
            .map(|i| {
                let p = origin + Vec2::new((i % nx) as f32, (i / nx) as f32) * cell;
                self.sample(Vec3::new(p.x, 0.0, p.y))
            })
            .collect();
        let (dist, _) = slice_from_values(dims, &values);
        let thick = thickness_grid(&[nx, nz], &dist);
        Slice2 { dims, origin, cell, dist, thick }
    }

    /// Step 12 of FEATURE_BRUSH 5.7: inradius, bound, bounds, sections, report, hash.
    fn summarize(&mut self) {
        let cell = self.cell;
        if self.is_flat() {
            let s = &self.slice;
            let nx = s.dims[0] as usize;
            let nz = s.dims[1] as usize;
            let mut inside = 0usize;
            let mut deepest = 0i16;
            let mut far = 0.0f32;
            // Deepest inside node on the border of the grid (contract request B3-1): a part cut off there has its
            // zero level up to that depth beyond the grid.
            let mut cut = 0i16;
            for (i, &d) in s.dist.iter().enumerate() {
                if d < 0 {
                    inside += 1;
                    deepest = deepest.min(d);
                    let (ix, iz) = (i % nx, i / nx);
                    let p = s.origin + Vec2::new(ix as f32, iz as f32) * s.cell;
                    far = far.max(p.length());
                    if ix == 0 || iz == 0 || ix + 1 == nx || iz + 1 == nz {
                        cut = cut.min(d);
                    }
                }
            }
            let cut = -(cut as f32) * s.cell / 256.0;
            self.inradius = -(deepest as f32) * s.cell / 256.0;
            self.bound = far + 3f32.sqrt() * s.cell + cut;
            let [lo, hi] = self.outline_bounds();
            let (lo, hi) = (lo - Vec2::splat(cut), hi + Vec2::splat(cut));
            self.bounds = Aabb { min: Vec3::new(lo.x, -s.cell, lo.y), max: Vec3::new(hi.x, s.cell, hi.y) };
            self.report.grid = [0; 3];
            self.report.inside_fraction = inside as f32 / s.dist.len().max(1) as f32;
            self.report.thin = self.inradius < 2.0 * s.cell;
            self.report.flat = true;
            self.report.height_ratio = 0.0;
        } else {
            let [nx, ny, nz] = self.dims.map(|n| n as usize);
            let mut inside = 0usize;
            let mut deepest = 0i16;
            let mut far = 0.0f32;
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            // Deepest inside node on the border of the grid (contract request B3-1): a closed part cut off there
            // (FEATURE_BRUSH 5.6) has its zero level up to that depth beyond the grid, so `bound` and `bounds` grow
            // by it. 0 for a field with a margin of outside nodes (every built-in and well-baked shape).
            let mut cut = 0i16;
            for (i, &d) in self.dist.iter().enumerate() {
                if d < 0 {
                    inside += 1;
                    deepest = deepest.min(d);
                    let (ix, iy, iz) = (i % nx, (i / nx) % ny, i / (nx * ny));
                    let p = self.origin + Vec3::new(ix as f32, iy as f32, iz as f32) * cell;
                    far = far.max(p.length());
                    lo = lo.min(p);
                    hi = hi.max(p);
                    if ix == 0 || iy == 0 || iz == 0 || ix + 1 == nx || iy + 1 == ny || iz + 1 == nz {
                        cut = cut.min(d);
                    }
                }
            }
            let cut = -(cut as f32) * cell / 256.0;
            self.inradius = -(deepest as f32) * cell / 256.0;
            self.bound = far + 3f32.sqrt() * cell + cut;
            self.bounds = if inside > 0 {
                Aabb { min: lo - Vec3::splat(cell + cut), max: hi + Vec3::splat(cell + cut) }
            } else {
                Aabb { min: Vec3::ZERO, max: Vec3::ZERO }
            };
            self.report.grid = self.dims;
            self.report.inside_fraction = inside as f32 / self.dist.len().max(1) as f32;
            self.report.thin = self.inradius < 2.0 * cell;
            self.report.flat = false;
            self.report.height_ratio = if inside > 0 {
                let extent = hi - lo + Vec3::splat(cell);
                extent.y / extent.x.max(extent.z)
            } else {
                0.0
            };
        }
        self.sections = [
            self.section(Vec3::ZERO, Vec3::Y, Vec3::Z, SECTION_LATTICE),
            self.section(Vec3::ZERO, Vec3::X, Vec3::Z, SECTION_LATTICE),
            self.section(Vec3::ZERO, Vec3::X, Vec3::Y, SECTION_LATTICE),
        ];
        let mut h = fnv1a64(0xcbf2_9ce4_8422_2325, &[self.dims, [self.slice.dims[0], self.slice.dims[1], 0]].concat()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>());
        h = fnv1a64(h, bytemuck::cast_slice::<i16, u8>(&self.dist));
        h = fnv1a64(h, &self.thick);
        h = fnv1a64(h, bytemuck::cast_slice::<i16, u8>(&self.slice.dist));
        h = fnv1a64(h, &self.slice.thick);
        self.hash = h;
    }
}

/// The two-dimensional distances (stored) of a slice from the values of a signed function at its nodes: border
/// nodes get the distance to their crossings (`a * b / sqrt(a^2 + b^2)` when links in both directions cross),
/// then the transform of step 8. Returns the distances and the inside flags.
pub(crate) fn slice_from_values(dims: [u32; 2], values: &[f32]) -> (Vec<i16>, Vec<bool>) {
    let [nx, nz] = dims.map(|n| n as usize);
    let inside: Vec<bool> = values.iter().map(|&v| v < 0.0).collect();
    // Distance in cells from node `i` to the crossing on its link to node `j` (linear interpolation).
    let crossing = |i: usize, j: usize| -> f32 {
        let (a, b) = (values[i], values[j]);
        if a == b { 0.5 } else { (a / (a - b)).clamp(0.0, 1.0) }
    };
    let border: Vec<f32> = (0..nx * nz)
        .map(|i| {
            let (x, z) = (i % nx, i / nx);
            let mut along_x = f32::INFINITY;
            let mut along_z = f32::INFINITY;
            if x > 0 && inside[i - 1] != inside[i] {
                along_x = along_x.min(crossing(i, i - 1));
            }
            if x + 1 < nx && inside[i + 1] != inside[i] {
                along_x = along_x.min(crossing(i, i + 1));
            }
            if z > 0 && inside[i - nx] != inside[i] {
                along_z = along_z.min(crossing(i, i - nx));
            }
            if z + 1 < nz && inside[i + nx] != inside[i] {
                along_z = along_z.min(crossing(i, i + nx));
            }
            match (along_x.is_finite(), along_z.is_finite()) {
                (true, true) => {
                    let (a, b) = (along_x.max(1e-6), along_z.max(1e-6));
                    a * b / (a * a + b * b).sqrt()
                }
                (true, false) => along_x,
                (false, true) => along_z,
                (false, false) => f32::NAN,
            }
        })
        .collect();
    (distances_from_border(&[nx, nz], &inside, &border), inside)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    use super::*;

    fn cube() -> &'static TipField {
        static CUBE: OnceLock<TipField> = OnceLock::new();
        CUBE.get_or_init(TipField::cube)
    }

    fn cylinder() -> &'static TipField {
        static CYLINDER: OnceLock<TipField> = OnceLock::new();
        CYLINDER.get_or_init(TipField::cylinder)
    }

    /// The fin shape of test C23: a ball of radius 0.6 and a slab with half sizes 0.95 x 0.06 x 0.3.
    fn fin() -> TipField {
        TipField::from_fn([129, 129, 129], Vec3::splat(-1.04), 2.08 / 128.0, |p| {
            let ball = p.length() - 0.6;
            let q = p.abs() - Vec3::new(0.95, 0.06, 0.3);
            let slab = q.max(Vec3::ZERO).length() + q.max_element().min(0.0);
            ball.min(slab)
        })
    }

    /// A deterministic pseudo-random point in [-1, 1]^3.
    fn points(seed: u64, count: usize) -> Vec<Vec3> {
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        (0..count).map(|_| Vec3::new(next(), next(), next())).collect()
    }

    #[test]
    fn envelope_matches_brute_force() {
        let f = [f64::INFINITY, 3.0, -2.0, f64::INFINITY, 0.5, 7.0, f64::INFINITY, -1.0];
        let mut out = [0.0; 8];
        let mut arg = [0u32; 8];
        envelope_1d(&f, &mut out, &mut arg);
        for (q, &value) in out.iter().enumerate() {
            let best = (0..8)
                .filter(|&p| f[p].is_finite())
                .map(|p| ((q as f64 - p as f64).powi(2) + f[p], p))
                .fold((f64::INFINITY, 0), |a, b| if b.0 < a.0 { b } else { a });
            assert!((value - best.0).abs() < 1e-9, "q {q}: {value} vs {}", best.0);
        }
        let none = [f64::INFINITY; 4];
        let (mut out, mut arg) = ([0.0; 4], [0u32; 4]);
        envelope_1d(&none, &mut out, &mut arg);
        assert!(out.iter().all(|v| v.is_infinite()) && arg.iter().all(|&a| a == u32::MAX));
        // The 3D transform equals brute force on a small grid.
        let dims = [5usize, 4, 3];
        let init: Vec<f64> = (0..60).map(|i| if i % 7 == 0 { -((i % 5) as f64) } else { f64::INFINITY }).collect();
        let (val, arg) = power_transform(&dims, &init);
        for x in 0..60usize {
            let px = [x % 5, (x / 5) % 4, x / 20];
            let best = (0..60usize)
                .filter(|&y| init[y].is_finite())
                .map(|y| {
                    let py = [y % 5, (y / 5) % 4, y / 20];
                    let d2: f64 = (0..3).map(|a| (px[a] as f64 - py[a] as f64).powi(2)).sum();
                    d2 + init[y]
                })
                .fold(f64::INFINITY, f64::min);
            assert!((val[x] - best).abs() < 1e-9);
            let y = arg[x] as usize;
            let py = [y % 5, (y / 5) % 4, y / 20];
            let d2: f64 = (0..3).map(|a| (px[a] as f64 - py[a] as f64).powi(2)).sum();
            assert!((d2 + init[y] - best).abs() < 1e-9, "the argument attains the minimum");
        }
    }

    /// F1: the built-in Cube.
    #[test]
    fn built_in_cube() {
        let c = cube();
        assert!((c.sample(Vec3::ZERO) + 0.577).abs() < 0.005, "{}", c.sample(Vec3::ZERO));
        assert!(c.sample(Vec3::new(0.57735, 0.0, 0.0)).abs() < 0.005);
        assert!((c.sample(Vec3::new(0.9, 0.0, 0.0)) - 0.323).abs() < 0.01, "{}", c.sample(Vec3::new(0.9, 0.0, 0.0)));
        assert!((c.inradius() - 0.577).abs() < 0.01);
        assert!(!c.is_flat() && c.dims().iter().all(|&n| n <= TIP_MAX_NODES));
        assert!(c.bound() > 0.99 && c.bound() < 1.03, "{}", c.bound());
        let b = c.bounds();
        assert!((b.max.x - (0.57735 + c.cell())).abs() < 1.5 * c.cell() && (b.min.y + b.max.y).abs() < 1e-4);
        let r = c.report();
        assert!(!r.flat && !r.partial && !r.thin && r.up_axis == 1 && r.triangles == 0);
        assert!((r.height_ratio - 1.0).abs() < 0.02);
        // Inside fraction: the cube's volume share of the grid box.
        let side = (c.dims()[0] - 1) as f32 * c.cell();
        assert!((r.inside_fraction - (2.0 * 0.57735f32 / side).powi(3)).abs() < 0.06, "{}", r.inside_fraction);
        // Deterministic, and every query point defined.
        assert_eq!(TipField::cube().hash(), c.hash());
        for p in points(7, 200) {
            assert!(c.sample(p * 3.0).is_finite() && c.thickness(p * 3.0).is_finite());
        }
        assert!(c.sample(Vec3::new(f32::NAN, 0.0, 0.0)).is_infinite());
    }

    #[test]
    fn built_in_cylinder() {
        let c = cylinder();
        let r = TipField::CYLINDER_RADIUS;
        assert!((c.sample(Vec3::ZERO) + r).abs() < 0.006, "{}", c.sample(Vec3::ZERO));
        assert!(c.sample(Vec3::new(r, 0.0, 0.0)).abs() < 0.006);
        assert!(c.sample(Vec3::new(0.0, r, 0.0)).abs() < 0.006);
        assert!((c.sample(Vec3::new(0.0, 0.0, 0.9)) - (0.9 - r)).abs() < 0.01);
        assert!((c.outline(0.5, 0.5) - (0.5f32.hypot(0.5) - r)).abs() < 0.01);
        assert!((c.report().height_ratio - 1.0).abs() < 0.02);
    }

    /// The neighbour property of FEATURE_BRUSH 5.6 for the built-in shapes (the bake's shapes are tested by F9).
    #[test]
    fn neighbouring_nodes_differ_by_at_most_one_cell() {
        for field in [cube(), cylinder()] {
            let [nx, ny, nz] = field.dims.map(|n| n as usize);
            let at = |x: usize, y: usize, z: usize| field.dist[x + nx * (y + ny * z)] as i32;
            let mut worst = 0;
            for z in 0..nz {
                for y in 0..ny {
                    for x in 0..nx {
                        if x + 1 < nx {
                            worst = worst.max((at(x, y, z) - at(x + 1, y, z)).abs());
                        }
                        if y + 1 < ny {
                            worst = worst.max((at(x, y, z) - at(x, y + 1, z)).abs());
                        }
                        if z + 1 < nz {
                            worst = worst.max((at(x, y, z) - at(x, y, z + 1)).abs());
                        }
                    }
                }
            }
            assert!(worst as f32 <= 1.001 * 256.0, "F: {worst}");
            let s = &field.slice;
            let [sx, sz] = s.dims.map(|n| n as usize);
            let mut worst = 0;
            for z in 0..sz {
                for x in 0..sx {
                    let v = s.dist[x + sx * z] as i32;
                    if x + 1 < sx {
                        worst = worst.max((v - s.dist[x + 1 + sx * z] as i32).abs());
                    }
                    if z + 1 < sz {
                        worst = worst.max((v - s.dist[x + sx * (z + 1)] as i32).abs());
                    }
                }
            }
            assert!(worst as f32 <= 1.001 * 256.0, "G: {worst}");
        }
    }

    /// Every end point of a closed outline is shared by exactly two segments.
    fn assert_closed(segments: &[[Vec2; 2]]) {
        assert!(!segments.is_empty());
        let mut count: HashMap<(u32, u32), u32> = HashMap::new();
        for s in segments {
            for p in s {
                *count.entry((p.x.to_bits(), p.y.to_bits())).or_default() += 1;
            }
        }
        assert!(count.values().all(|&c| c == 2), "open ends: {:?}", count.iter().filter(|(_, c)| **c != 2).count());
    }

    /// F13: sections.
    #[test]
    fn sections_of_the_cube() {
        let c = cube();
        let cut = &c.principal_sections()[1];
        assert_closed(cut);
        for p in cut.iter().flatten() {
            assert!((p.x.abs().max(p.y.abs()) - 0.577).abs() < 0.015, "{p:?}");
        }
        let (s, cs) = std::f32::consts::FRAC_PI_4.sin_cos();
        let turned = c.section(Vec3::ZERO, Vec3::new(cs, 0.0, -s), Vec3::Y, 96);
        assert_closed(&turned);
        for k in 0..3 {
            assert_closed(&c.principal_sections()[k]);
        }
    }

    /// F20 (built-in and formula parts): local thickness.
    #[test]
    fn local_thickness() {
        let c = cube();
        for p in points(11, 1000) {
            let q = p * 0.57;
            if c.sample(q) < 0.0 {
                assert!(c.thickness(q) >= -c.sample(q) - 0.5 * c.cell(), "{q:?}");
            }
        }
        assert!((c.thickness(Vec3::ZERO) - c.inradius()).abs() < 1.5 * c.cell());

        // A ball: the thickness is the inradius everywhere inside.
        let ball = TipField::from_fn([97, 97, 97], Vec3::splat(-1.05), 2.1 / 96.0, |p| p.length() - 0.9);
        assert!((ball.inradius() - 0.9).abs() < 1.5 * ball.cell());
        for p in points(13, 1000) {
            let q = p * 0.55;
            if ball.sample(q) < 0.0 {
                assert!(ball.thickness(q) >= -ball.sample(q) - 0.5 * ball.cell());
                assert!((ball.thickness(q) - ball.inradius()).abs() < 1.5 * ball.cell(), "{q:?}: {}", ball.thickness(q));
            }
        }

        // The fin of C23: 0.06 on the fin's middle plane beyond 0.75 from the centre, 0.6 at the centre.
        let f = fin();
        let tol = 1.5 * f.cell();
        for x in [0.76f32, 0.8, 0.85, 0.9, -0.8] {
            for z in [0.0f32, 0.1, -0.2] {
                let t = f.thickness(Vec3::new(x, 0.0, z));
                assert!((t - 0.06).abs() < tol, "({x}, {z}): {t}");
            }
        }
        assert!((f.thickness(Vec3::ZERO) - 0.6).abs() < tol, "{}", f.thickness(Vec3::ZERO));
        // The outline of the fin's middle slice: inside on the fin, outside beside it.
        assert!(f.outline(0.9, 0.0) < 0.0 && f.outline(0.0, 0.7) > 0.0, "{} {}", f.outline(0.9, 0.0), f.outline(0.0, 0.7));
        // In the plane y = 0 the fin is 0.3 wide (half size along z): its outline thickness there is about 0.3.
        assert!((f.outline_thickness(0.9, 0.0) - 0.3).abs() < 3.0 * f.cell(), "{}", f.outline_thickness(0.9, 0.0));
    }

    #[test]
    fn flat_outlines() {
        // A five-armed star (outer radius 0.95, inner 0.4) as a polygon distance.
        let star: Vec<Vec2> = (0..10)
            .map(|i| {
                let r = if i % 2 == 0 { 0.95 } else { 0.4 };
                let a = std::f32::consts::TAU * i as f32 / 10.0;
                Vec2::new(a.cos(), a.sin()) * r
            })
            .collect();
        let field = TipField::outline_from_fn([129, 129], Vec2::splat(-1.04), 2.08 / 128.0, |p| polygon_sd(&star, p));
        assert!(field.is_flat() && field.report().flat && field.report().grid == [0; 3]);
        assert_eq!(field.report().height_ratio, 0.0);
        // On an arm axis at 0.8: inside; on a bisector at 0.8: outside.
        assert!(field.outline(0.8, 0.0) < 0.0);
        let bisector = Vec2::new(36f32.to_radians().cos(), 36f32.to_radians().sin()) * 0.8;
        assert!(field.outline(bisector.x, bisector.y) > 0.0);
        assert_eq!(field.sample(Vec3::new(0.8, 0.5, 0.0)), field.outline(0.8, 0.0), "sample is G for a flat outline");
        let [lo, hi] = field.outline_bounds();
        assert!(lo.x < -0.7 && hi.x > 0.9 && lo.y < -0.85 && hi.y > 0.85);
        assert!(field.bound() > 0.95 && field.bound() < 1.05);
        assert!(field.inradius() > 0.3 && field.inradius() < 0.42, "{}", field.inradius());
        assert!(field.outline_thickness(0.0, 0.0) >= field.inradius() - 1.5 * field.cell());
        assert_closed(&field.principal_sections()[1]);
        // A solid plate of the same star: its middle slice is the star.
        let plate = TipField::from_fn([129, 129, 129], Vec3::splat(-1.04), 2.08 / 128.0, |p| {
            polygon_sd(&star, Vec2::new(p.x, p.z)).max(p.y.abs() - 0.05)
        });
        assert!(!plate.is_flat() && plate.report().height_ratio < 0.1);
        for (x, z) in [(0.8f32, 0.0f32), (bisector.x, bisector.y), (0.0, 0.0), (0.3, -0.6)] {
            assert!((plate.outline(x, z) - field.outline(x, z)).abs() < 1.5 * plate.cell(), "({x}, {z})");
        }
        assert_ne!(plate.hash(), field.hash());
    }

    /// Signed distance to a closed polygon (negative inside).
    fn polygon_sd(poly: &[Vec2], p: Vec2) -> f32 {
        let mut d = f32::INFINITY;
        let mut inside = false;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let e = b - a;
            let t = ((p - a).dot(e) / e.length_squared()).clamp(0.0, 1.0);
            d = d.min((p - (a + e * t)).length());
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        if inside { -d } else { d }
    }

    /// Contract request B3-1 (applied at M1): a closed part cut off at its grid's border (a box of half extents
    /// (0.62, 0.3, 0.3) on a grid ending at x = +-0.5, border depth 0.12 = 7.2 cells) keeps its whole zero level
    /// inside `bound()` and `bounds()`; a field with a margin of outside nodes keeps the bound of step 12.
    #[test]
    fn a_part_cut_off_at_the_border_stays_inside_its_bound() {
        let cut = TipField::from_fn([61, 41, 41], Vec3::new(-0.5, -1.0 / 3.0, -1.0 / 3.0), 1.0 / 60.0, |p| {
            let q = p.abs() - Vec3::new(0.62, 0.3, 0.3);
            q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
        });
        assert!(cut.sample(Vec3::new(0.6, 0.0, 0.0)) < 0.0, "the zero level reaches beyond the grid");
        assert!(cut.bound() >= 0.62, "bound {}", cut.bound());
        let b = cut.bounds();
        assert!(b.max.x >= 0.62 && b.min.x <= -0.62, "{b:?}");
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        for _ in 0..20_000 {
            let q = Vec3::new(next(), next() * 0.5, next() * 0.5);
            if cut.sample(q) < 0.0 {
                assert!(q.length() <= cut.bound(), "{q}");
                assert!(q.cmpge(b.min).all() && q.cmple(b.max).all(), "{q}");
            }
        }
        // The built-in cube has a margin: its bound is the farthest inside node plus sqrt(3) cells, about 1.0.
        assert!(cube().bound() > 0.99 && cube().bound() < 1.03, "{}", cube().bound());
    }

    #[test]
    fn same_result_on_one_thread() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let serial = pool.install(TipField::cylinder);
        assert_eq!(serial.hash(), cylinder().hash());
        assert!(serial == *cylinder());
    }
}
