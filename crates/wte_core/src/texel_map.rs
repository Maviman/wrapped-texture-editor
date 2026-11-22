// Owner: core-geometry (C1). Texel ownership map and the one surface rasterizer (ARCHITECTURE.md 5.9, algorithm 6.3).
//
// Everything that decides whether a texel centre lies inside a triangle goes through `TriSetup`, which evaluates the
// edge functions relative to the triangle's own pixel bounding box. `TexelMap::build`, `raster_tri` and
// `TexelMap::locate` therefore agree texel for texel, whatever clip box or row band a caller passes.

use glam::{Vec2, Vec3};

use crate::ids::{TextureSetId, TriId};
use crate::mesh::{MeshData, TriFlags, UvStats};

/// Texel -> owning triangle for one texture set, including a gutter ring around every UV island.
pub struct TexelMap {
    pub width: u32,
    pub height: u32,
    pub gutter: u8,
    pub stats: UvStats,
    // u32::MAX = none; bit 31 = gutter texel; low 31 bits = TriId.
    owner: Vec<u32>,
    // The set the map was built for: `raster_tri` ignores triangles of other sets.
    set: TextureSetId,
    // (gutter texel, covered texel it was grown from), sorted by gutter texel, for the gutter texels whose source
    // centre more than one triangle covers (mirrored, stacked or instanced UVs, or an edge shared by two triangles). `raster_tri` emits such a gutter texel
    // for every triangle that covers its source, not only for the owner.
    shared_gutter: Vec<(u32, u32)>,
    // Every gutter texel, sorted by (owning triangle, texel index); built on first use by `raster_tri` (pass 2 of a
    // triangle whose box is large: tiled UVs make every box the whole image). Lets pass 2 visit the gutter texels a
    // triangle owns instead of scanning its whole box.
    gutter_by_tri: std::sync::OnceLock<Vec<u32>>,
    // The sliver triangles that claimed a texel in build step 2, ascending (round 7 A1).
    claimed: Vec<TriId>,
    // The brush index (round 7 A1, `prepare_brush_index`): a bitset over `TriId` of the triangles whose pass 2 can
    // report a texel they own (the owners of gutter texels and `claimed`), and one bit per `SHARED_BLOCK` x
    // `SHARED_BLOCK` texel block that holds a `shared_gutter` texel (row-major, `width.div_ceil(SHARED_BLOCK)` per row).
    pass2_tris: std::sync::OnceLock<Vec<u64>>,
    shared_blocks: std::sync::OnceLock<Vec<u64>>,
}

/// Pass 2 of `raster_tri` uses the gutter index instead of scanning the box from this many texels in the folded box on.
const INDEXED_PASS2_TEXELS: i64 = 64 * 64;
/// Side of the blocks `raster_tri_covered` decides to fold or not (unwrapped texels).
const FOLD_BLOCK: i64 = 64;
/// Side of the texel blocks of the brush index's `shared_blocks` bitset.
const SHARED_BLOCK: i64 = 8;

/// A point on the surface found from a texel (2D -> 3D).
#[derive(Copy, Clone, Debug)]
pub struct SurfacePoint {
    pub tri: TriId,
    pub bary: [f32; 3],
    pub position: Vec3,
    pub normal: Vec3,
}

/// One texel visited by `raster_tri`; `x`, `y` are already wrapped into the image.
#[derive(Copy, Clone, Debug)]
pub struct Fragment {
    pub x: u32,
    pub y: u32,
    pub bary: [f32; 3],
    pub gutter: bool,
}

const GUTTER_BIT: u32 = 1 << 31;
const TRI_MASK: u32 = !GUTTER_BIT;
/// A texel centre counts as inside a triangle up to this distance (in pixels) outside an edge.
const EDGE_EPS: f64 = 1e-4;
/// A later triangle counts as overlapping an owned texel only when the centre lies at least this far (in pixels)
/// inside all of its edges, so centres on an edge shared by two neighbours are not reported as overlap.
const OVERLAP_MARGIN: f64 = 1e-3;
/// UVs further than this many image sizes from the unit tile are not rasterized (guards absurd input).
const MAX_TILES: i64 = 4;
/// Axial neighbours first, so that a straight step wins over a diagonal one.
const NEIGHBOURS: [(i64, i64); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)];
const CENTROID: [f32; 3] = [1.0 / 3.0; 3];

/// A triangle prepared for rasterization in pixel space.
struct TriSetup {
    /// Edge function `i` (zero on the edge opposite vertex `i`, positive inside, equal to twice the triangle area at
    /// vertex `i`): `a[i] * x + b[i] * y + c[i]` with `x`, `y` relative to `(ox, oy)`.
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
    /// Lengths of the three edges in pixels.
    len: [f64; 3],
    inv_area2: f64,
    /// Origin of the relative coordinates: the floor of the bounding box minimum.
    ox: i64,
    oy: i64,
    /// Integer bounding box of the vertices: `[floor(min), ceil(max)]`.
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl TriSetup {
    fn new(uv: [Vec2; 3], width: u32, height: u32) -> Option<TriSetup> {
        let (w, h) = (width as f64, height as f64);
        let px = [uv[0].x as f64 * w, uv[1].x as f64 * w, uv[2].x as f64 * w];
        let py = [uv[0].y as f64 * h, uv[1].y as f64 * h, uv[2].y as f64 * h];
        if !px.iter().chain(&py).all(|v| v.is_finite()) {
            return None;
        }
        let lo = |v: &[f64; 3]| v[0].min(v[1]).min(v[2]);
        let hi = |v: &[f64; 3]| v[0].max(v[1]).max(v[2]);
        let (wi, hi_px) = (width as i64, height as i64);
        // Float-to-int casts saturate; the clamp bounds the scan for UVs far outside the unit tile.
        let clamp_x = |v: f64| (v as i64).clamp(-MAX_TILES * wi, (MAX_TILES + 1) * wi);
        let clamp_y = |v: f64| (v as i64).clamp(-MAX_TILES * hi_px, (MAX_TILES + 1) * hi_px);
        let (x0, x1) = (clamp_x(lo(&px).floor()), clamp_x(hi(&px).ceil()));
        let (y0, y1) = (clamp_y(lo(&py).floor()), clamp_y(hi(&py).ceil()));
        let (ox, oy) = (x0, y0);
        let rx = px.map(|v| v - ox as f64);
        let ry = py.map(|v| v - oy as f64);
        let area2 = (rx[1] - rx[0]) * (ry[2] - ry[0]) - (ry[1] - ry[0]) * (rx[2] - rx[0]);
        if !(area2.abs() > 0.0 && area2.is_finite()) {
            return None;
        }
        let s = area2.signum();
        let (mut a, mut b, mut c, mut len) = ([0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3]);
        for i in 0..3 {
            let (j, k) = ((i + 1) % 3, (i + 2) % 3);
            let (dx, dy) = (rx[k] - rx[j], ry[k] - ry[j]);
            a[i] = -dy * s;
            b[i] = dx * s;
            c[i] = -(a[i] * rx[j] + b[i] * ry[j]);
            len[i] = dx.hypot(dy);
        }
        Some(TriSetup { a, b, c, len, inv_area2: 1.0 / area2.abs(), ox, oy, x0, y0, x1, y1 })
    }

    /// The row terms of the edge functions for the texel row `y` (unwrapped).
    #[inline]
    fn row(&self, y: i64) -> [f64; 3] {
        let qy = (y - self.oy) as f64 + 0.5;
        [self.b[0] * qy + self.c[0], self.b[1] * qy + self.c[1], self.b[2] * qy + self.c[2]]
    }

    /// Edge functions at the centre of texel `x` (unwrapped) of a row prepared with `row`.
    #[inline]
    fn eval(&self, x: i64, row: &[f64; 3]) -> [f64; 3] {
        let qx = (x - self.ox) as f64 + 0.5;
        [self.a[0] * qx + row[0], self.a[1] * qx + row[1], self.a[2] * qx + row[2]]
    }

    /// Texel-centre-inside test: inclusive, with the edge epsilon of section 6.3.
    #[inline]
    fn inside(&self, e: &[f64; 3]) -> bool {
        e[0] >= -EDGE_EPS * self.len[0] && e[1] >= -EDGE_EPS * self.len[1] && e[2] >= -EDGE_EPS * self.len[2]
    }

    /// The centre lies inside by at least `OVERLAP_MARGIN` pixels (`span` with `well_inside_min` is the row form).
    #[cfg(test)]
    #[inline]
    fn well_inside(&self, e: &[f64; 3]) -> bool {
        e[0] >= OVERLAP_MARGIN * self.len[0]
            && e[1] >= OVERLAP_MARGIN * self.len[1]
            && e[2] >= OVERLAP_MARGIN * self.len[2]
    }

    /// How far (in pixels) the point lies outside the triangle, as the largest distance to an edge line; <= 0 inside.
    #[inline]
    fn outside_distance(&self, e: &[f64; 3]) -> f64 {
        let d = |i: usize| if self.len[i] > 0.0 { -e[i] / self.len[i] } else { f64::NEG_INFINITY };
        d(0).max(d(1)).max(d(2))
    }

    /// Barycentric weights (of v0, v1, v2) from edge function values; extrapolated outside the triangle.
    /// Falls back to the centroid if the triangle is too thin for finite weights.
    #[inline]
    fn bary(&self, e: &[f64; 3]) -> [f32; 3] {
        let b0 = e[0] * self.inv_area2;
        let b1 = e[1] * self.inv_area2;
        let b = [b0 as f32, b1 as f32, (1.0 - b0 - b1) as f32];
        if b.iter().all(|v| v.is_finite()) { b } else { CENTROID }
    }

    /// The texels `x` in `[lo, hi)` of a row prepared with `row` whose three edge functions reach `min` (the same
    /// expression `eval` computes, compared the same way as `inside` / `well_inside`). Each edge function is monotone
    /// in `x`, so the set is one interval; it is found by bisection instead of evaluating every texel of the row.
    fn span(&self, row: &[f64; 3], lo: i64, hi: i64, min: [f64; 3]) -> (i64, i64) {
        let (mut lo, mut hi) = (lo, hi);
        for i in 0..3 {
            if lo >= hi {
                break;
            }
            let ok = |x: i64| self.a[i] * ((x - self.ox) as f64 + 0.5) + row[i] >= min[i];
            if self.a[i] > 0.0 {
                lo = first_true(lo, hi, ok);
            } else if self.a[i] < 0.0 {
                hi = first_true(lo, hi, |x| !ok(x));
            } else if !ok(lo) {
                hi = lo;
            }
        }
        (lo, hi.max(lo))
    }

    /// Thresholds of `inside` for `span`.
    fn inside_min(&self) -> [f64; 3] {
        [-EDGE_EPS * self.len[0], -EDGE_EPS * self.len[1], -EDGE_EPS * self.len[2]]
    }

    /// Thresholds of `well_inside` for `span`.
    fn well_inside_min(&self) -> [f64; 3] {
        [OVERLAP_MARGIN * self.len[0], OVERLAP_MARGIN * self.len[1], OVERLAP_MARGIN * self.len[2]]
    }

    /// Unwrapped texel range `[x0, x1) x [y0, y1)` to scan: the bounding box grown by `grow` texels (plus the one
    /// texel a centre-inside test can reach beyond the integer box).
    fn scan(&self, grow: i64) -> (i64, i64, i64, i64) {
        (self.x0 - grow - 1, self.x1 + grow + 1, self.y0 - grow - 1, self.y1 + grow + 1)
    }
}

/// First `x` in `[lo, hi)` for which the monotone (false, then true) predicate holds; `hi` if there is none.
fn first_true(mut lo: i64, mut hi: i64, p: impl Fn(i64) -> bool) -> i64 {
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if p(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// Folds the unwrapped range `[lo, hi)` onto `[0, size)`: one or two ranges, or the whole axis.
fn fold(lo: i64, hi: i64, size: i64, out: &mut Vec<(i64, i64)>) {
    if lo >= hi {
        return;
    }
    if hi - lo >= size {
        out.push((0, size));
        return;
    }
    let a = lo.rem_euclid(size);
    let b = a + (hi - lo);
    if b <= size {
        out.push((a, b));
    } else {
        out.push((a, size));
        out.push((0, b - size));
    }
}

/// Sorts ranges and merges the ones that overlap or touch.
fn merge(ranges: &mut Vec<(i64, i64)>) {
    if ranges.len() < 2 {
        return;
    }
    ranges.sort_unstable();
    let mut k = 0;
    for i in 1..ranges.len() {
        if ranges[i].0 <= ranges[k].1 {
            ranges[k].1 = ranges[k].1.max(ranges[i].1);
        } else {
            k += 1;
            ranges[k] = ranges[i];
        }
    }
    ranges.truncate(k + 1);
}

/// The texel under a triangle's UV centroid (the sliver fallback of build step 2), wrapped into the image.
fn centroid_texel(uv: [Vec2; 3], width: u32, height: u32) -> Option<(i64, i64)> {
    let cx = ((uv[0].x as f64 + uv[1].x as f64 + uv[2].x as f64) / 3.0 * width as f64).floor();
    let cy = ((uv[0].y as f64 + uv[1].y as f64 + uv[2].y as f64) / 3.0 * height as f64).floor();
    if !(cx.is_finite() && cy.is_finite()) || width == 0 || height == 0 {
        return None;
    }
    Some(((cx as i64).rem_euclid(width as i64), (cy as i64).rem_euclid(height as i64)))
}

/// The signed difference `to - from` of two wrapped coordinates, taking the shorter way around.
#[inline]
fn wrap_delta(from: i64, to: i64, size: i64) -> i64 {
    let d = (to - from).rem_euclid(size);
    if d * 2 > size { d - size } else { d }
}

impl TexelMap {
    /// Value of a texel without an owner in the raw owner array.
    pub(crate) const NO_OWNER: u32 = u32::MAX;

    /// The raw owner array (`width * height` entries; `NO_OWNER`, or a triangle id with bit 31 marking a gutter texel).
    pub(crate) fn owner_raw(&self) -> &[u32] {
        &self.owner
    }

    /// Default gutter width: `clamp(max(w, h) / 256, 2, 16)`.
    pub fn default_gutter(width: u32, height: u32) -> u8 {
        (width.max(height) / 256).clamp(2, 16) as u8
    }

    /// Builds the map of one set at the given resolution (section 6.3).
    pub fn build(mesh: &MeshData, set: TextureSetId, width: u32, height: u32, gutter: u8) -> TexelMap {
        Self::build_cancellable(mesh, set, width, height, gutter, &|| false).expect("never cancelled")
    }

    /// `build` that polls `cancelled` once per triangle and returns `None` as soon as it reports true.
    pub fn build_cancellable(
        mesh: &MeshData,
        set: TextureSetId,
        width: u32,
        height: u32,
        gutter: u8,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Option<TexelMap> {
        let (w, h) = (width as i64, height as i64);
        let wu = width as usize;
        let n = wu * height as usize;
        let mut owner = vec![Self::NO_OWNER; n];
        let mut stats = UvStats { texel_world: 1.0, ..UvStats::default() };
        let (mut area_world, mut area_px) = (0.0f64, 0.0f64);
        let mut slivers: Vec<TriId> = Vec::new();
        // Folded texel ranges of the current wrapped row: centres inside the triangle, and well inside it.
        let mut inside: Vec<(i64, i64)> = Vec::new();
        let mut well: Vec<(i64, i64)> = Vec::new();
        // Texels whose centre more than one triangle covers (mirrored, stacked or instanced UVs, and centres on an
        // edge two triangles share): their gutter belongs to every triangle covering them.
        let mut shared: Vec<u64> = vec![0; n.div_ceil(64)];
        let mut any_shared = false;

        // Step 1: rasterize in index order; the first writer owns a texel. Bit 31 temporarily marks texels that
        // a later triangle also covers, so each of them is counted once.
        //
        // A triangle is processed one wrapped row at a time: the inside texels of every unwrapped row that folds onto
        // it form one interval (found by bisection), and the intervals of all images are folded and merged first. A
        // texel then costs the same whether one or many images of a repeating (tiled) triangle cover it, and a
        // triangle spanning many tiles costs at most one pass over the image.
        for tri in 0..mesh.tri_count() {
            if mesh.tri_set(tri) != Some(set) {
                continue;
            }
            if cancelled() {
                return None;
            }
            if mesh.tri_flags[tri as usize].contains(TriFlags::UV_MIRRORED) {
                stats.mirrored_tris += 1;
            }
            let uv = mesh.tri_uvs(tri);
            let [p0, p1, p2] = mesh.tri_positions(tri).map(|p| p.as_dvec3());
            area_world += 0.5 * (p1 - p0).cross(p2 - p0).length();
            let d = uv.map(|c| c.as_dvec2());
            area_px += 0.5 * (d[1] - d[0]).perp_dot(d[2] - d[0]).abs() * width as f64 * height as f64;
            if n == 0 {
                continue;
            }
            let Some(s) = TriSetup::new(uv, width, height) else { continue };
            let (in_min, well_min) = (s.inside_min(), s.well_inside_min());
            let (x0, x1, y0, y1) = s.scan(0);
            let mut covers_a_centre = false;
            for r in 0..(y1 - y0).min(h) {
                inside.clear();
                well.clear();
                let mut y = y0 + r;
                while y < y1 {
                    let row = s.row(y);
                    let (lo, hi) = s.span(&row, x0, x1, in_min);
                    if lo < hi {
                        fold(lo, hi, w, &mut inside);
                        let (a, b) = s.span(&row, lo, hi, well_min);
                        fold(a, b, w, &mut well);
                    }
                    y += h;
                }
                if inside.is_empty() {
                    continue;
                }
                covers_a_centre = true;
                merge(&mut inside);
                merge(&mut well);
                let base = (y0 + r).rem_euclid(h) as usize * wu;
                let mut k = 0;
                for &(a, b) in &inside {
                    for x in a..b {
                        while k < well.len() && well[k].1 <= x {
                            k += 1;
                        }
                        let i = base + x as usize;
                        let o = owner[i];
                        if o == Self::NO_OWNER {
                            owner[i] = tri;
                            stats.covered_texels += 1;
                        } else if o & TRI_MASK != tri {
                            shared[i / 64] |= 1 << (i % 64);
                            any_shared = true;
                            if o & GUTTER_BIT == 0 && k < well.len() && well[k].0 <= x {
                                owner[i] = o | GUTTER_BIT;
                                stats.overlapped_texels += 1;
                            }
                        }
                    }
                }
            }
            if !covers_a_centre {
                slivers.push(tri);
            }
        }
        if stats.overlapped_texels > 0 {
            for o in owner.iter_mut().filter(|o| **o != Self::NO_OWNER) {
                *o &= TRI_MASK;
            }
        }
        let is_shared = |i: usize| any_shared && shared[i / 64] & (1 << (i % 64)) != 0;

        // Step 2: a triangle that covered no texel centre claims the texel under its UV centroid if it is free.
        let mut claimed: Vec<TriId> = Vec::new();
        for tri in slivers {
            let Some((cx, cy)) = centroid_texel(mesh.tri_uvs(tri), width, height) else { continue };
            let i = cy as usize * wu + cx as usize;
            if owner[i] == Self::NO_OWNER {
                owner[i] = tri;
                stats.covered_texels += 1;
                claimed.push(tri);
            }
        }

        // Step 3: gutter. Breadth-first from all covered texels, one ring per step; a reached texel inherits the
        // owner of the texel it was reached from. Within a ring straight steps are taken before diagonal ones, so a
        // gutter texel continues the covered texel nearest to it. Each gutter texel remembers the covered texel it was
        // grown from; the ones grown from a shared texel are listed in `shared_gutter`.
        let mut shared_gutter: Vec<(u32, u32)> = Vec::new();
        if gutter > 0 && stats.covered_texels > 0 && stats.covered_texels < n as u64 {
            let hu = height as usize;
            // (texel, the covered texel it was grown from)
            let mut frontier: Vec<(usize, usize)> = Vec::new();
            // First ring: every free texel looks for a covered neighbour (texels claimed in this pass carry the
            // gutter bit and are not sources).
            for y in 0..hu {
                let rows = [(y + hu - 1) % hu * wu, y * wu, (y + 1) % hu * wu];
                for x in 0..wu {
                    if owner[rows[1] + x] != Self::NO_OWNER {
                        continue;
                    }
                    let cols = [(x + wu - 1) % wu, x, (x + 1) % wu];
                    for (dx, dy) in NEIGHBOURS {
                        let src = rows[(dy + 1) as usize] + cols[(dx + 1) as usize];
                        let o = owner[src];
                        if o != Self::NO_OWNER && o & GUTTER_BIT == 0 {
                            owner[rows[1] + x] = o | GUTTER_BIT;
                            frontier.push((rows[1] + x, src));
                            break;
                        }
                    }
                }
            }
            // Further rings grow from the previous one.
            let neighbour = |i: usize, (dx, dy): (i64, i64)| -> usize {
                let x = ((i % wu) as i64 + dx).rem_euclid(w);
                let y = ((i / wu) as i64 + dy).rem_euclid(h);
                y as usize * wu + x as usize
            };
            for ring in 1..=gutter {
                let grown_from_shared = frontier.iter().filter(|&&(_, src)| is_shared(src));
                shared_gutter.extend(grown_from_shared.map(|&(i, src)| (i as u32, src as u32)));
                if ring == gutter || frontier.is_empty() {
                    break;
                }
                let mut next = Vec::new();
                for directions in [&NEIGHBOURS[..4], &NEIGHBOURS[4..]] {
                    for &(i, src) in &frontier {
                        let o = owner[i];
                        for &d in directions {
                            let j = neighbour(i, d);
                            if owner[j] == Self::NO_OWNER {
                                owner[j] = o;
                                next.push((j, src));
                            }
                        }
                    }
                }
                frontier = next;
            }
            shared_gutter.sort_unstable();
        }

        // Step 4: statistics.
        if n > 0 {
            stats.coverage = stats.covered_texels as f32 / n as f32;
        }
        let texel_world = (area_world / area_px).sqrt();
        if area_world > 0.0 && area_px > 0.0 && texel_world.is_finite() && texel_world > 0.0 {
            stats.texel_world = texel_world as f32;
        }
        Some(TexelMap {
            width,
            height,
            gutter,
            stats,
            owner,
            set,
            shared_gutter,
            gutter_by_tri: std::sync::OnceLock::new(),
            claimed,
            pass2_tris: std::sync::OnceLock::new(),
            shared_blocks: std::sync::OnceLock::new(),
        })
    }

    /// Owner of a texel: (triangle, is_gutter).
    #[inline]
    pub fn owner(&self, x: u32, y: u32) -> Option<(TriId, bool)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let o = self.owner[y as usize * self.width as usize + x as usize];
        if o == Self::NO_OWNER { None } else { Some((o & TRI_MASK, o & GUTTER_BIT != 0)) }
    }

    /// The covered texel a gutter texel was grown from, when more than one triangle covers that texel's centre.
    #[inline]
    fn shared_source(&self, i: usize) -> Option<usize> {
        if self.shared_gutter.is_empty() {
            return None;
        }
        let k = self.shared_gutter.binary_search_by_key(&(i as u32), |e| e.0).ok()?;
        Some(self.shared_gutter[k].1 as usize)
    }

    /// Builds the brush index of round 7 A1 (`pass2_tris`, `shared_blocks`) unless it exists: a few milliseconds for a
    /// 4096 x 4096 set, once per map. The stroke calls it when it first touches a set; the lean rasterizer builds it
    /// on first use otherwise.
    pub(crate) fn prepare_brush_index(&self) {
        self.pass2_tris();
        self.shared_blocks();
    }

    /// The bitset over `TriId` of the triangles that own a gutter texel or claimed a texel (a set union built in
    /// parallel chunks of the owner array, so the same on any thread count). Bits beyond its end are 0.
    fn pass2_tris(&self) -> &[u64] {
        self.pass2_tris.get_or_init(|| {
            use rayon::prelude::*;
            let gutter_owner = |o: u32| (o != Self::NO_OWNER && o & GUTTER_BIT != 0).then_some(o & TRI_MASK);
            let max_tri = self
                .owner
                .par_chunks(1 << 16)
                .map(|c| c.iter().filter_map(|&o| gutter_owner(o)).max())
                .reduce(|| None, |a, b| a.max(b))
                .max(self.claimed.last().copied());
            let Some(max_tri) = max_tri else { return Vec::new() };
            let words = (max_tri as usize + 1).div_ceil(64);
            let chunk = (self.owner.len() / 16).max(1 << 16);
            let mut bits = self
                .owner
                .par_chunks(chunk)
                .map(|c| {
                    let mut bits = vec![0u64; words];
                    for t in c.iter().filter_map(|&o| gutter_owner(o)) {
                        bits[t as usize / 64] |= 1 << (t % 64);
                    }
                    bits
                })
                .reduce(
                    || vec![0u64; words],
                    |mut a, b| {
                        a.iter_mut().zip(&b).for_each(|(a, b)| *a |= b);
                        a
                    },
                );
            for &t in &self.claimed {
                bits[t as usize / 64] |= 1 << (t % 64);
            }
            bits
        })
    }

    /// One bit per `SHARED_BLOCK` x `SHARED_BLOCK` block of texels that holds a `shared_gutter` texel (empty when
    /// there is none).
    fn shared_blocks(&self) -> &[u64] {
        self.shared_blocks.get_or_init(|| {
            if self.shared_gutter.is_empty() {
                return Vec::new();
            }
            let bw = self.shared_blocks_per_row();
            let bh = self.height.div_ceil(SHARED_BLOCK as u32) as usize;
            let mut bits = vec![0u64; (bw * bh).div_ceil(64)];
            let width = self.width as usize;
            for &(i, _) in &self.shared_gutter {
                let (x, y) = (i as usize % width, i as usize / width);
                let b = y / SHARED_BLOCK as usize * bw + x / SHARED_BLOCK as usize;
                bits[b / 64] |= 1 << (b % 64);
            }
            bits
        })
    }

    /// Blocks of `shared_blocks` per block row.
    #[inline]
    fn shared_blocks_per_row(&self) -> usize {
        self.width.div_ceil(SHARED_BLOCK as u32) as usize
    }

    /// False when pass 2 of `tri` over the wrapped columns `cols` and the wrapped rows `rows` (folded ranges inside
    /// the image) cannot report anything: `tri` owns no gutter texel and claimed no texel, and no `shared_gutter`
    /// texel lies in a block the box meets. Pass 2 reports only those three kinds of texels (the candidates of
    /// `pass2_row_candidates`), so a call that skips it when this is false visits the same fragments.
    fn pass2_may_report(&self, tri: TriId, cols: &[(i64, i64)], rows: &[(i64, i64)]) -> bool {
        let tris = self.pass2_tris();
        if tris.get(tri as usize / 64).is_some_and(|w| w & (1 << (tri % 64)) != 0) {
            return true;
        }
        if self.shared_gutter.is_empty() {
            return false;
        }
        let blocks = self.shared_blocks();
        let bw = self.shared_blocks_per_row();
        // Any bit set in the half-open bit range [a, b).
        let any = |mut a: usize, b: usize| -> bool {
            while a < b {
                let word = a / 64;
                let (lo, hi) = (a % 64, (b - word * 64).min(64));
                let mask = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
                if blocks[word] & mask != 0 {
                    return true;
                }
                a = word * 64 + hi;
            }
            false
        };
        for &(ra, rb) in rows.iter().filter(|r| r.0 < r.1) {
            for by in (ra / SHARED_BLOCK)..=((rb - 1) / SHARED_BLOCK) {
                let base = by as usize * bw;
                for &(ca, cb) in cols.iter().filter(|c| c.0 < c.1) {
                    if any(base + (ca / SHARED_BLOCK) as usize, base + ((cb - 1) / SHARED_BLOCK) as usize + 1) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The gutter index (built on first use): every gutter texel, sorted by (owning triangle, texel index).
    fn gutter_by_tri(&self) -> &[u32] {
        self.gutter_by_tri.get_or_init(|| {
            let mut texels: Vec<u32> = (0..self.owner.len() as u32)
                .filter(|&i| {
                    let o = self.owner[i as usize];
                    o != Self::NO_OWNER && o & GUTTER_BIT != 0
                })
                .collect();
            // Stable: texels of one triangle stay in ascending index order.
            texels.sort_by_key(|&i| self.owner[i as usize] & TRI_MASK);
            texels
        })
    }

    /// The gutter texels `tri` owns, ascending.
    fn gutter_of(&self, tri: TriId) -> &[u32] {
        let index = self.gutter_by_tri();
        let key = |i: &u32| self.owner[*i as usize] & TRI_MASK;
        let a = index.partition_point(|i| key(i) < tri);
        let b = a + index[a..].partition_point(|i| key(i) <= tri);
        &index[a..b]
    }

    /// Pass-2 candidates of `tri` in the wrapped row `wy`, ascending and unique: the gutter texels it owns, the gutter
    /// texels grown from a shared covered texel (any owner), and `sliver` (the texel a sliver triangle claimed).
    /// Every other texel of the row is one pass 2 would skip.
    fn pass2_row_candidates(&self, own: &[u32], wy: i64, sliver: Option<(i64, i64)>, out: &mut Vec<u32>) {
        out.clear();
        let width = self.width as u64;
        let (lo, hi) = (wy as u64 * width, (wy as u64 + 1) * width);
        let row = |v: &[u32]| {
            let a = v.partition_point(|&i| (i as u64) < lo);
            let b = v.partition_point(|&i| (i as u64) < hi);
            (a, b)
        };
        let (a, b) = row(own);
        out.extend(own[a..b].iter().map(|&i| i - lo as u32));
        if !self.shared_gutter.is_empty() {
            let a = self.shared_gutter.partition_point(|e| (e.0 as u64) < lo);
            let b = self.shared_gutter.partition_point(|e| (e.0 as u64) < hi);
            out.extend(self.shared_gutter[a..b].iter().map(|e| e.0 - lo as u32));
        }
        if let Some((sx, sy)) = sliver
            && sy == wy
        {
            out.push(sx as u32);
        }
        out.sort_unstable();
        out.dedup();
    }

    /// The unwrapped position of the wrapped texel `(wx, wy)` that lies closest to the triangle, among the images
    /// `(wx + i * width, wy + j * height)` inside the triangle's scan range; ties go to the smaller `y`, then `x`.
    fn best_image(&self, s: &TriSetup, wx: i64, wy: i64) -> (i64, i64) {
        let (w, h) = (self.width as i64, self.height as i64);
        let (x0, x1, y0, y1) = s.scan(self.gutter as i64);
        let mut best: Option<(f64, i64, i64)> = None;
        for j in (y0 - wy + h - 1).div_euclid(h)..=(y1 - 1 - wy).div_euclid(h) {
            let y = wy + j * h;
            let row = s.row(y);
            for i in (x0 - wx + w - 1).div_euclid(w)..=(x1 - 1 - wx).div_euclid(w) {
                let x = wx + i * w;
                let d = s.outside_distance(&s.eval(x, &row));
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, x, y));
                }
            }
        }
        match best {
            Some((_, x, y)) => (x, y),
            // Not in the scan range at all (cannot happen for a texel the triangle owns): the image nearest to the box.
            None => {
                let nearest =
                    |lo: i64, hi: i64, v: i64, size: i64| v + ((lo + hi) / 2 - v + size / 2).div_euclid(size) * size;
                (nearest(s.x0, s.x1, wx, w), nearest(s.y0, s.y1, wy, h))
            }
        }
    }

    /// 2D -> 3D: the surface point under the centre of an owned texel.
    ///
    /// For a covered texel the point lies on the owning triangle. For a gutter texel the barycentric weights are
    /// extrapolated (outside 0..1) exactly as `raster_tri` reports them, so `position` continues the triangle's
    /// plane beyond its edge. `normal` is the interpolated shading normal, or the geometric normal where the
    /// interpolation degenerates.
    pub fn locate(&self, mesh: &MeshData, x: u32, y: u32) -> Option<SurfacePoint> {
        let (tri, is_gutter) = self.owner(x, y)?;
        if tri >= mesh.tri_count() {
            return None;
        }
        let bary = match TriSetup::new(mesh.tri_uvs(tri), self.width, self.height) {
            Some(s) => {
                let (ux, uy) = self.best_image(&s, x as i64, y as i64);
                let e = s.eval(ux, &s.row(uy));
                // A texel claimed through the sliver fallback is neither inside nor gutter: use the centroid.
                if is_gutter || s.inside(&e) { s.bary(&e) } else { CENTROID }
            }
            None => CENTROID,
        };
        let p = mesh.tri_positions(tri);
        let n = mesh.tri_normals(tri);
        let position = p[0] * bary[0] + p[1] * bary[1] + p[2] * bary[2];
        let normal = (n[0] * bary[0] + n[1] * bary[1] + n[2] * bary[2]).normalize_or_zero();
        let normal = if normal == Vec3::ZERO || !normal.is_finite() { mesh.geo_normal(tri) } else { normal };
        Some(SurfacePoint { tri, bary, position, normal })
    }
}

/// The one surface rasterizer. Visits every texel that triangle `tri` covers or owns (section 6.3), restricted to
/// `clip` (unwrapped pixel-space box [x0, y0, x1, y1], None = whole triangle) and to wrapped rows in `rows`.
///
/// A texel is visited when its centre lies inside the triangle (`gutter == false`) or when the map says the triangle
/// owns it: gutter texels (`gutter == true`, barycentrics extrapolated beyond the edge) and the single texel a sliver
/// triangle claimed through the fallback (`gutter == false`, barycentrics of the centroid). `clip` selects the texels
/// whose unit square touches the box. Triangles that do not belong to the map's set (or have no UVs) visit nothing.
/// A gutter texel grown from a texel that several triangles cover (mirrored or stacked UVs) is also visited for each
/// of those triangles, with barycentrics extrapolated for that triangle, so painting on any copy reaches the padding.
/// Inside texels arrive first (in row order of the unwrapped scan), then the gutter and claimed texels; a texel can be
/// visited more than once when UVs overlap themselves through wrapping, so writers must be idempotent.
pub fn raster_tri(
    mesh: &MeshData,
    map: &TexelMap,
    tri: TriId,
    clip: Option<[f32; 4]>,
    rows: std::ops::Range<u32>,
    f: &mut impl FnMut(Fragment),
) {
    raster_tri_with(mesh, map, tri, clip, rows, INDEXED_PASS2_TEXELS, None::<&mut fn(u32, u32, u32)>, None, false, f);
}

/// `raster_tri` for a caller to whom every inside fragment of `tri` in some parts of its UV box means the same (a dab
/// whose full-strength core contains those parts of the triangle). `fold_block` picks the parts: it is asked about
/// the clipped box and, if it refuses that, about its blocks of 64 x 64 unwrapped texels, each as the integer box
/// `[x0, y0, x1, y1)` (texel centres lie inside it); `None` picks everything. Pass 1 reports each texel covered in a
/// picked part ONCE per call, as runs `covered(y, x0, x1)` (texels `x0..x1` of wrapped row `y`; the runs of a row
/// ascending and disjoint), however many unwrapped images of a tiled triangle cover it, and the inside fragments of
/// the other blocks to `f` (same fragments as `raster_tri`, grouped by wrapped row); pass 2 (gutter and claimed
/// texels) reports fragments to `f` exactly as `raster_tri` does. The covered texels plus the pass-1 fragment
/// positions are exactly the pass-1 fragment positions of `raster_tri`.
#[allow(clippy::too_many_arguments)]
pub fn raster_tri_covered(
    mesh: &MeshData,
    map: &TexelMap,
    tri: TriId,
    clip: Option<[f32; 4]>,
    rows: std::ops::Range<u32>,
    fold_block: Option<&dyn Fn([i64; 4]) -> bool>,
    covered: &mut impl FnMut(u32, u32, u32),
    f: &mut impl FnMut(Fragment),
) {
    raster_tri_with(mesh, map, tri, clip, rows, INDEXED_PASS2_TEXELS, Some(covered), fold_block, false, f);
}

/// What pass 2 of one rasterizer call cost (the stroke probe's slots 10 and 11, round 7): the texels of the pass-2
/// box looked at (texel by texel, or the index candidates of a large box), and 1 when a lean call skipped pass 2.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct Pass2Cost {
    pub(crate) texels: u64,
    pub(crate) skipped: u64,
}

impl std::ops::AddAssign for Pass2Cost {
    fn add_assign(&mut self, o: Pass2Cost) {
        self.texels += o.texels;
        self.skipped += o.skipped;
    }
}

/// `raster_tri` for the brush (round 7 A1): with `lean`, pass 2 is skipped when it cannot report anything (see
/// `TexelMap::pass2_may_report`); the fragments are exactly those of `raster_tri`, in the same order.
pub(crate) fn raster_tri_lean(
    mesh: &MeshData,
    map: &TexelMap,
    tri: TriId,
    clip: Option<[f32; 4]>,
    rows: std::ops::Range<u32>,
    lean: bool,
    f: &mut impl FnMut(Fragment),
) -> Pass2Cost {
    raster_tri_with(mesh, map, tri, clip, rows, INDEXED_PASS2_TEXELS, None::<&mut fn(u32, u32, u32)>, None, lean, f)
}

/// `raster_tri_covered` with the lean pass 2 of `raster_tri_lean`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn raster_tri_covered_lean(
    mesh: &MeshData,
    map: &TexelMap,
    tri: TriId,
    clip: Option<[f32; 4]>,
    rows: std::ops::Range<u32>,
    fold_block: Option<&dyn Fn([i64; 4]) -> bool>,
    lean: bool,
    covered: &mut impl FnMut(u32, u32, u32),
    f: &mut impl FnMut(Fragment),
) -> Pass2Cost {
    raster_tri_with(mesh, map, tri, clip, rows, INDEXED_PASS2_TEXELS, Some(covered), fold_block, lean, f)
}

/// Per vertex, how far below 0 the barycentric weight of a fragment `raster_tri` reports as inside `uv` (a `width` x
/// `height` image) can go: `EDGE_EPS * edge length / twice the area`, in pixel space; `None` for a degenerate triangle.
pub fn inside_bary_slack(uv: [Vec2; 3], width: u32, height: u32) -> Option<[f32; 3]> {
    let s = TriSetup::new(uv, width, height)?;
    Some([0, 1, 2].map(|i| (EDGE_EPS * s.len[i] * s.inv_area2) as f32))
}

/// The offsets `r` in `0..min(y1 - y0, h)`, ascending, whose wrapped row `(y0 + r) mod h` lies in
/// `rows_start..rows_end` (a range inside `0..h`): at most two intervals. The folded loops of pass 1 walk only these
/// (round 7: with 32-row bands a whole-image tiled triangle would otherwise loop over every row in every band).
fn folded_rows(y0: i64, y1: i64, h: i64, rows_start: i64, rows_end: i64) -> impl Iterator<Item = i64> {
    let n = (y1 - y0).min(h);
    let a = y0.rem_euclid(h);
    // wy = a + r for r < h - a, a + r - h after.
    let first = (rows_start - a).max(0)..(rows_end - a).clamp(0, n);
    let second = (rows_start - a + h).clamp(0, n)..(rows_end - a + h).clamp(0, n);
    first.chain(second)
}

/// The clipped unwrapped scan box `[x0, y0, x1, y1)` of a prepared triangle: its bounding box grown by the gutter
/// (and the one texel a centre test can reach), cut to the texels whose unit square touches `clip`; `None` when empty.
fn scan_box(s: &TriSetup, gutter: u8, clip: Option<[f32; 4]>) -> Option<[i64; 4]> {
    let (mut x0, mut x1, mut y0, mut y1) = s.scan(gutter as i64);
    if let Some(c) = clip {
        if !(c[0] <= c[2] && c[1] <= c[3]) {
            return None;
        }
        // Saturating float-to-int casts keep huge boxes harmless.
        x0 = x0.max(c[0].floor() as i64);
        y0 = y0.max(c[1].floor() as i64);
        x1 = x1.min((c[2].floor() as i64).saturating_add(1));
        y1 = y1.min((c[3].floor() as i64).saturating_add(1));
    }
    (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
}

/// The clipped unwrapped box `[x0, y0, x1, y1)` that `raster_tri` scans for `tri` and `clip` (round 7 A2: the brush
/// bins its candidates by it); `None` when the call visits nothing (empty box, a degenerate triangle, a triangle of
/// another set). Every fragment of the call lies in it (at its wrapped position).
pub(crate) fn raster_box(mesh: &MeshData, map: &TexelMap, tri: TriId, clip: Option<[f32; 4]>) -> Option<[i64; 4]> {
    if map.width == 0 || map.height == 0 || tri >= mesh.tri_count() || mesh.tri_set(tri) != Some(map.set) {
        return None;
    }
    let s = TriSetup::new(mesh.tri_uvs(tri), map.width, map.height)?;
    scan_box(&s, map.gutter, clip)
}

/// `raster_tri` with the box size (texels) from which pass 2 walks the gutter index instead of the box; both ways
/// visit the same fragments in the same order (tests compare them).
#[allow(clippy::too_many_arguments)]
fn raster_tri_with(
    mesh: &MeshData,
    map: &TexelMap,
    tri: TriId,
    clip: Option<[f32; 4]>,
    rows: std::ops::Range<u32>,
    indexed_from: i64,
    covered: Option<&mut impl FnMut(u32, u32, u32)>,
    fold_block: Option<&dyn Fn([i64; 4]) -> bool>,
    lean: bool,
    f: &mut impl FnMut(Fragment),
) -> Pass2Cost {
    let mut cost = Pass2Cost::default();
    let (w, h) = (map.width as i64, map.height as i64);
    if w == 0 || h == 0 || tri >= mesh.tri_count() || mesh.tri_set(tri) != Some(map.set) {
        return cost;
    }
    let Some(s) = TriSetup::new(mesh.tri_uvs(tri), map.width, map.height) else { return cost };
    let Some([x0, y0, x1, y1]) = scan_box(&s, map.gutter, clip) else { return cost };
    let (gx0, gx1, gy0, gy1) = s.scan(map.gutter as i64);
    let (rows_start, rows_end) = (rows.start.min(map.height) as i64, rows.end.min(map.height) as i64);
    if rows_start >= rows_end {
        return cost;
    }
    // Wider than the image: a wrapped texel has several images in the scan range and only the one closest to the
    // triangle may be reported through the ownership rule.
    let ambiguous = gx1 - gx0 > w || gy1 - gy0 > h;
    let width = map.width as usize;

    // Pass 1: the texels whose centre lies inside the triangle, every unwrapped image of them. Each row's inside
    // texels form one interval, so texels outside the triangle cost nothing here.
    let in_min = s.inside_min();
    // Folding (`raster_tri_covered`): everything, nothing, or the blocks of `FOLD_BLOCK` x `FOLD_BLOCK` unwrapped
    // texels (aligned to multiples of `FOLD_BLOCK`, clipped to the box) that `fold_block` picks. A predicate that
    // picks the whole box picks each of its blocks (callers test convex bounds).
    enum Folding<'p> {
        Nothing,
        All,
        Blocks(&'p dyn Fn([i64; 4]) -> bool),
    }
    let folding = match (covered.is_some(), fold_block) {
        (false, _) => Folding::Nothing,
        (true, None) => Folding::All,
        (true, Some(pick)) if pick([x0, y0, x1, y1]) => Folding::All,
        (true, Some(pick)) => Folding::Blocks(pick),
    };
    match (covered, folding) {
        (Some(covered), Folding::All) => {
            // Per wrapped row, the inside intervals of all its unwrapped images, folded and merged (as in step 1 of
            // the build), so each covered texel is reported once.
            let mut spans: Vec<(i64, i64)> = Vec::new();
            for r in folded_rows(y0, y1, h, rows_start, rows_end) {
                let wy = (y0 + r).rem_euclid(h);
                spans.clear();
                let mut y = y0 + r;
                while y < y1 {
                    let (lo, hi) = s.span(&s.row(y), x0, x1, in_min);
                    fold(lo, hi, w, &mut spans);
                    y += h;
                }
                merge(&mut spans);
                for &(a, b) in &spans {
                    covered(wy as u32, a as u32, b as u32);
                }
            }
        }
        (Some(covered), Folding::Blocks(pick)) => {
            // As above for the picked blocks; the inside texels of the other blocks are reported one by one (per
            // wrapped row, every image), the picks of a block row computed when a row of it is first scanned.
            let (bx0, bx1) = (x0.div_euclid(FOLD_BLOCK), (x1 - 1).div_euclid(FOLD_BLOCK));
            let by0 = y0.div_euclid(FOLD_BLOCK);
            let mut picks: Vec<Option<Vec<bool>>> = vec![None; ((y1 - 1).div_euclid(FOLD_BLOCK) - by0 + 1) as usize];
            let mut spans: Vec<(i64, i64)> = Vec::new();
            for r in folded_rows(y0, y1, h, rows_start, rows_end) {
                let wy = (y0 + r).rem_euclid(h);
                spans.clear();
                let mut y = y0 + r;
                while y < y1 {
                    let row = s.row(y);
                    let (lo, hi) = s.span(&row, x0, x1, in_min);
                    if lo < hi {
                        let by = y.div_euclid(FOLD_BLOCK);
                        let row_picks = picks[(by - by0) as usize].get_or_insert_with(|| {
                            let (ya, yb) = (y0.max(by * FOLD_BLOCK), y1.min((by + 1) * FOLD_BLOCK));
                            (bx0..=bx1)
                                .map(|bx| pick([x0.max(bx * FOLD_BLOCK), ya, x1.min((bx + 1) * FOLD_BLOCK), yb]))
                                .collect()
                        });
                        let mut x = lo;
                        while x < hi {
                            let bx = x.div_euclid(FOLD_BLOCK);
                            let end = hi.min((bx + 1) * FOLD_BLOCK);
                            if row_picks[(bx - bx0) as usize] {
                                fold(x, end, w, &mut spans);
                            } else {
                                for ux in x..end {
                                    let e = s.eval(ux, &row);
                                    f(Fragment { x: ux.rem_euclid(w) as u32, y: wy as u32, bary: s.bary(&e), gutter: false });
                                }
                            }
                            x = end;
                        }
                    }
                    y += h;
                }
                merge(&mut spans);
                for &(a, b) in &spans {
                    covered(wy as u32, a as u32, b as u32);
                }
            }
        }
        _ => {
            for tile_y in y0.div_euclid(h)..=(y1 - 1).div_euclid(h) {
                let ys = y0.max(tile_y * h + rows_start);
                let ye = y1.min(tile_y * h + rows_end);
                for y in ys..ye {
                    let wy = (y - tile_y * h) as u32;
                    let row = s.row(y);
                    let (lo, hi) = s.span(&row, x0, x1, in_min);
                    let mut x = lo;
                    while x < hi {
                        let tile_x = x.div_euclid(w);
                        let end = hi.min((tile_x + 1) * w);
                        for ux in x..end {
                            let e = s.eval(ux, &row);
                            f(Fragment { x: (ux - tile_x * w) as u32, y: wy, bary: s.bary(&e), gutter: false });
                        }
                        x = end;
                    }
                }
            }
        }
    }
    // Pass 2: owned texels whose centre is outside (gutter texels, the texel a sliver claimed) and gutter texels grown
    // from a texel this triangle shares with their owner. Every wrapped texel of the clipped box is looked at once.
    let (mut cols, mut wrapped_rows) = (Vec::new(), Vec::new());
    fold(x0, x1, w, &mut cols);
    fold(y0, y1, h, &mut wrapped_rows);
    if lean {
        // Round 7 A1: the brush skips a pass 2 that cannot report anything (O(1) for a triangle of a dense mesh).
        let in_rows: Vec<(i64, i64)> =
            wrapped_rows.iter().map(|&(a, b)| (a.max(rows_start), b.min(rows_end))).filter(|r| r.0 < r.1).collect();
        if !map.pass2_may_report(tri, &cols, &in_rows) {
            cost.skipped = 1;
            return cost;
        }
    }
    let sliver_texel = if ambiguous { centroid_texel(mesh.tri_uvs(tri), map.width, map.height) } else { None };
    // The unwrapped image of a wrapped texel: the closest one for a wide triangle, else the only one in the box.
    let image = |wx: i64, wy: i64| -> (i64, i64) {
        if ambiguous {
            map.best_image(&s, wx, wy)
        } else {
            (x0 + (wx - x0).rem_euclid(w), y0 + (wy - y0).rem_euclid(h))
        }
    };
    let in_box = |(x, y): (i64, i64)| (x0..x1).contains(&x) && (y0..y1).contains(&y);
    // A large box is not scanned texel by texel: only the texels pass 2 can report are visited (the gutter index),
    // in the same order. The sliver texel is the only covered texel pass 2 may report (step 2 of the build).
    let box_texels = cols.iter().map(|c| c.1 - c.0).sum::<i64>() * wrapped_rows.iter().map(|r| r.1 - r.0).sum::<i64>();
    let indexed = box_texels >= indexed_from;
    let own = if indexed { map.gutter_of(tri) } else { &[] };
    let sliver = if indexed { centroid_texel(mesh.tri_uvs(tri), map.width, map.height) } else { None };
    let mut candidates: Vec<u32> = Vec::new();
    let col_texels = cols.iter().map(|c| c.1 - c.0).sum::<i64>() as u64;
    for &(ra, rb) in &wrapped_rows {
        for wy in ra.max(rows_start)..rb.min(rows_end) {
            let owners = &map.owner[wy as usize * width..(wy as usize + 1) * width];
            if indexed {
                map.pass2_row_candidates(own, wy, sliver, &mut candidates);
                cost.texels += candidates.len() as u64;
                if candidates.is_empty() {
                    continue;
                }
            } else {
                cost.texels += col_texels;
            }
            let mut visit = |wx: i64| {
                let o = owners[wx as usize];
                if o == TexelMap::NO_OWNER {
                    return;
                }
                let gutter = o & GUTTER_BIT != 0;
                if o & TRI_MASK == tri {
                    // A covered texel of a wide triangle is inside one of its images (pass 1) unless the triangle
                    // is a sliver that claimed it: skip the image search for all others.
                    if ambiguous && !gutter && sliver_texel != Some((wx, wy)) {
                        return;
                    }
                    let (x, y) = image(wx, wy);
                    if !in_box((x, y)) {
                        return;
                    }
                    let e = s.eval(x, &s.row(y));
                    if s.inside(&e) {
                        return;
                    }
                    let bary = if gutter { s.bary(&e) } else { CENTROID };
                    f(Fragment { x: wx as u32, y: wy as u32, bary, gutter });
                } else if gutter {
                    let Some(src) = map.shared_source(wy as usize * width + wx as usize) else { return };
                    let (sx, sy) = ((src % width) as i64, (src / width) as i64);
                    let (dx, dy) = (wrap_delta(wx, sx, w), wrap_delta(wy, sy, h));
                    // The image of the source this triangle covers, and the gutter texel's image beside it.
                    let (ux, uy) = if ambiguous {
                        map.best_image(&s, sx, sy)
                    } else {
                        let (gx, gy) = image(wx, wy);
                        (gx + dx, gy + dy)
                    };
                    if !s.inside(&s.eval(ux, &s.row(uy))) {
                        return;
                    }
                    let (gx, gy) = (ux - dx, uy - dy);
                    if !in_box((gx, gy)) {
                        return;
                    }
                    let e = s.eval(gx, &s.row(gy));
                    f(Fragment { x: wx as u32, y: wy as u32, bary: s.bary(&e), gutter: true });
                }
            };
            for &(ca, cb) in &cols {
                if indexed {
                    let a = candidates.partition_point(|&x| (x as i64) < ca);
                    let b = candidates.partition_point(|&x| (x as i64) < cb);
                    candidates[a..b].iter().for_each(|&x| visit(x as i64));
                } else {
                    (ca..cb).for_each(&mut visit);
                }
            }
        }
    }
    cost
}

/// The meshes of round 7's equality tests (T1, T3, T6): an atlas with seams, a sphere, tiled UVs, stacked and mirrored
/// copies (shared gutters), slivers (one claims a texel, one is hidden) and a wrapped quad. All of set 0.
#[cfg(test)]
pub(crate) fn r7_test_meshes() -> Vec<(&'static str, crate::mesh::RawMesh)> {
    use crate::mesh::{RawMaterial, RawMesh, RawTriangle};
    use crate::samples;
    let one = |triangles: Vec<RawTriangle>| RawMesh {
        triangles,
        materials: vec![RawMaterial { name: "m".into(), base_color: [1.0; 4], double_sided: false, texture: None }],
    };
    // A triangle of world size 1 at x, with the given UVs.
    let tri = |uv: [[f32; 2]; 3], x: f32, z: f32| RawTriangle {
        positions: [Vec3::new(x, 0.0, z), Vec3::new(x + 1.0, 0.0, z), Vec3::new(x, 1.0, z)],
        normals: None,
        uvs: Some(uv.map(Vec2::from_array)),
        material: 0,
    };
    // Quads of one world unit whose UVs repeat 3 times across each quad.
    let mut tiled = Vec::new();
    for q in 0..4 {
        let x = q as f32;
        let (a, b, c, d) = (Vec3::new(x, 0.0, 0.0), Vec3::new(x + 1.0, 0.0, 0.0), Vec3::new(x + 1.0, 1.0, 0.0), Vec3::new(x, 1.0, 0.0));
        let t = |p: [Vec3; 3], uv: [[f32; 2]; 3]| RawTriangle { positions: p, normals: None, uvs: Some(uv.map(Vec2::from_array)), material: 0 };
        tiled.push(t([a, b, d], [[0.0, 0.0], [3.0, 0.0], [0.0, 3.0]]));
        tiled.push(t([b, c, d], [[3.0, 0.0], [3.0, 3.0], [0.0, 3.0]]));
    }
    let stacked = one(vec![
        tri([[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]], 0.0, 0.0),
        tri([[0.4, 0.1], [0.1, 0.1], [0.1, 0.4]], 1.5, 0.0), // mirrored copy: shared gutter
        tri([[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]], 3.0, 0.0), // stacked copy
        tri([[0.55, 0.6], [0.9, 0.62], [0.6, 0.95]], 0.0, 1.0),
        tri([[0.55, 0.6], [0.9, 0.62], [0.6, 0.95]], 1.5, 1.0), // a second stacked pair
        tri([[0.85, 0.05], [1.2, 0.1], [0.9, 0.3]], 3.0, 1.0), // crosses the right border
    ]);
    let slivers = one(vec![
        tri([[0.05, 0.45], [0.45, 0.45], [0.05, 0.05]], 0.0, 0.0),
        tri([[0.52, 0.704], [0.92, 0.704], [0.52, 0.701]], 2.0, 0.0), // claims its centroid texel
        tri([[0.1, 0.404], [0.3, 0.404], [0.1, 0.401]], 4.0, 0.0), // hidden under the first
        tri([[0.7, 0.2], [0.7004, 0.2], [0.7, 0.2003]], 6.0, 0.0), // a tiny sliver
    ]);
    vec![
        ("seam_cube", samples::seam_cube(1.0)),
        ("uv_sphere", samples::uv_sphere(16, 8, 1.0)),
        ("tiled_strip", one(tiled)),
        ("mirrored_quads", samples::mirrored_quads()),
        ("stacked", stacked),
        ("slivers", slivers),
        ("wrapped_quad", samples::wrapped_quad()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{RawMaterial, RawMesh, RawTriangle};
    use crate::samples;
    use crate::testutil::mesh_of;
    use std::collections::{BTreeMap, BTreeSet};

    const SET: TextureSetId = TextureSetId(0);

    fn all_fragments(mesh: &MeshData, map: &TexelMap, tri: TriId) -> Vec<Fragment> {
        let mut out = Vec::new();
        raster_tri(mesh, map, tri, None, 0..map.height, &mut |f| out.push(f));
        out
    }

    fn counts(map: &TexelMap) -> (u64, u64) {
        let (mut covered, mut gutter) = (0, 0);
        for y in 0..map.height {
            for x in 0..map.width {
                match map.owner(x, y) {
                    Some((_, false)) => covered += 1,
                    Some((_, true)) => gutter += 1,
                    None => {}
                }
            }
        }
        (covered, gutter)
    }

    fn uv_tri(uv: [[f32; 2]; 3], x: f32) -> RawTriangle {
        RawTriangle {
            positions: [Vec3::new(x, 0.0, 0.0), Vec3::new(x + 1.0, 0.0, 0.0), Vec3::new(x, 1.0, 0.0)],
            normals: None,
            uvs: Some(uv.map(Vec2::from_array)),
            material: 0,
        }
    }

    fn one_material(triangles: Vec<RawTriangle>) -> RawMesh {
        RawMesh {
            triangles,
            materials: vec![RawMaterial { name: "m".into(), base_color: [1.0; 4], double_sided: false, texture: None }],
        }
    }

    /// The UV a set of barycentric weights stands for.
    fn bary_uv(mesh: &MeshData, tri: TriId, bary: [f32; 3]) -> Vec2 {
        let uv = mesh.tri_uvs(tri);
        uv[0] * bary[0] + uv[1] * bary[1] + uv[2] * bary[2]
    }

    /// Distance in pixels between a UV and a texel centre, modulo whole tiles.
    fn centre_error(uv: Vec2, x: u32, y: u32, w: u32, h: u32) -> (f32, f32) {
        let dx = (uv.x * w as f32 - (x as f32 + 0.5)).rem_euclid(w as f32);
        let dy = (uv.y * h as f32 - (y as f32 + 0.5)).rem_euclid(h as f32);
        (dx.min(w as f32 - dx), dy.min(h as f32 - dy))
    }

    #[test]
    fn default_gutter_follows_the_resolution() {
        assert_eq!(TexelMap::default_gutter(1, 1), 2);
        assert_eq!(TexelMap::default_gutter(256, 256), 2);
        assert_eq!(TexelMap::default_gutter(1024, 512), 4);
        assert_eq!(TexelMap::default_gutter(512, 2048), 8);
        assert_eq!(TexelMap::default_gutter(4096, 4096), 16);
        assert_eq!(TexelMap::default_gutter(8192, 8192), 16);
    }

    #[test]
    fn cube_coverage_matches_the_island_area() {
        let mesh = mesh_of(samples::seam_cube(2.0));
        for (w, h) in [(256u32, 256u32), (512, 256), (300, 420)] {
            let map = TexelMap::build(&mesh, SET, w, h, 2);
            let island_area: f64 = (0..6)
                .map(|f| {
                    let [u0, u1, v0, v1] = samples::seam_cube_island(f);
                    (u1 - u0) as f64 * (v1 - v0) as f64
                })
                .sum();
            let expected = island_area * w as f64 * h as f64;
            let (covered, _) = counts(&map);
            assert_eq!(covered, map.stats.covered_texels);
            assert!((covered as f64 - expected).abs() <= 0.02 * expected, "{w}x{h}: {covered} vs {expected}");
            assert!((map.stats.coverage as f64 - covered as f64 / (w as f64 * h as f64)).abs() < 1e-6);
            assert_eq!(map.stats.overlapped_texels, 0, "texel centres on the diagonals are not overlap");
            assert_eq!(map.stats.mirrored_tris, 0);
            // Each face of the 2-unit cube has area 4 and covers (u1 - u0) * (v1 - v0) * w * h texels.
            let expect_texel_world = (24.0 / expected).sqrt();
            assert!((map.stats.texel_world as f64 - expect_texel_world).abs() < 1e-4 * expect_texel_world);
            // Every covered texel's centre is inside the island of its owner and both triangles of a face own texels.
            let mut per_tri = [0u32; 12];
            for y in 0..h {
                for x in 0..w {
                    if let Some((tri, false)) = map.owner(x, y) {
                        per_tri[tri as usize] += 1;
                        let [u0, u1, v0, v1] = samples::seam_cube_island(tri / 2);
                        let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                        assert!(u >= u0 - 1e-5 && u <= u1 + 1e-5 && v >= v0 - 1e-5 && v <= v1 + 1e-5);
                    }
                }
            }
            for pair in per_tri.chunks(2) {
                let (a, b) = (pair[0] as f64, pair[1] as f64);
                assert!((a - b).abs() < 0.05 * (a + b), "the diagonal splits an island in two halves: {a} / {b}");
            }
        }
    }

    #[test]
    fn gutter_width_is_exact() {
        // tight_islands at 64 px, v in 0.25..0.75: covered rows 16..=47; island A covers columns 16..=31, B 32..=47
        // (0.496 * 64 = 31.74, 0.504 * 64 = 32.26).
        let mesh = mesh_of(samples::tight_islands());
        for gutter in [0u8, 1, 3, 5] {
            let map = TexelMap::build(&mesh, SET, 64, 64, gutter);
            assert_eq!(map.gutter, gutter);
            let g = gutter as i32;
            for y in 0..64i32 {
                for x in 0..64i32 {
                    let inside = (16..48).contains(&x) && (16..48).contains(&y);
                    let ring = (16 - g..48 + g).contains(&x) && (16 - g..48 + g).contains(&y);
                    let expect = if inside { Some(false) } else if ring { Some(true) } else { None };
                    assert_eq!(map.owner(x as u32, y as u32).map(|o| o.1), expect, "gutter {gutter} at ({x}, {y})");
                }
            }
            // A gutter texel belongs to the island it is nearest to: left of column 32 island A (triangles 0, 1).
            if gutter > 0 {
                assert!(map.owner(15, 20).unwrap().0 < 2);
                assert!(map.owner(48, 20).unwrap().0 >= 2);
                assert!(map.owner(20, 15).unwrap().0 < 2);
                assert!(map.owner(40, 48).unwrap().0 >= 2);
            }
            let (covered, ring) = counts(&map);
            assert_eq!(covered, 32 * 32);
            assert_eq!(ring, ((32 + 2 * g as u64) * (32 + 2 * g as u64)) - 32 * 32);
            assert_eq!(map.stats.covered_texels, 32 * 32, "the gutter is not part of the coverage");
        }
        // Out-of-range texels have no owner.
        let map = TexelMap::build(&mesh, SET, 64, 64, 2);
        assert_eq!(map.owner(64, 0), None);
        assert_eq!(map.owner(0, 64), None);
        assert_eq!(map.owner(u32::MAX, u32::MAX), None);
        // A set id the mesh does not have: an empty map.
        let none = TexelMap::build(&mesh, TextureSetId(3), 8, 8, 2);
        assert_eq!(counts(&none), (0, 0));
        assert_eq!(none.stats, UvStats { texel_world: 1.0, ..UvStats::default() });
    }

    #[test]
    fn gutter_wraps_at_the_image_borders() {
        // An island touching the left border (columns 0..=7, rows 8..=23 at 32 px): its gutter continues on the
        // right side of the image.
        let raw = one_material(vec![
            uv_tri([[0.0, 0.75], [0.25, 0.75], [0.0, 0.25]], 0.0),
            uv_tri([[0.25, 0.75], [0.25, 0.25], [0.0, 0.25]], 2.0),
        ]);
        let mesh = mesh_of(raw);
        let map = TexelMap::build(&mesh, SET, 32, 32, 2);
        assert_eq!(map.owner(0, 16).map(|o| o.1), Some(false));
        assert_eq!(map.owner(31, 16).map(|o| o.1), Some(true));
        assert_eq!(map.owner(30, 16).map(|o| o.1), Some(true));
        assert_eq!(map.owner(29, 16), None);
        assert_eq!(map.owner(9, 16).map(|o| o.1), Some(true));
        assert_eq!(map.owner(10, 16), None);
        // A gutter texel continues the covered texel straight beside it.
        for y in 8..24 {
            assert_eq!(map.owner(31, y).map(|o| o.0), map.owner(0, y).map(|o| o.0));
            assert_eq!(map.owner(30, y).map(|o| o.0), map.owner(0, y).map(|o| o.0));
            assert_eq!(map.owner(8, y).map(|o| o.0), map.owner(7, y).map(|o| o.0));
        }
        // The wrapped gutter texels are reported with barycentrics extrapolated to the left of the island.
        let (tri, _) = map.owner(31, 16).unwrap();
        let frags = all_fragments(&mesh, &map, tri);
        let wrapped: Vec<&Fragment> = frags.iter().filter(|f| f.x >= 30).collect();
        assert!(!wrapped.is_empty() && wrapped.iter().all(|f| f.gutter));
        for f in &wrapped {
            let u = bary_uv(&mesh, tri, f.bary).x;
            assert!((u * 32.0 - (f.x as f32 + 0.5 - 32.0)).abs() < 1e-3, "u = {u} for texel {}", f.x);
        }
        // `locate` agrees: the surface point continues the triangle to the left of the island.
        let point = map.locate(&mesh, 31, 16).unwrap();
        assert_eq!(point.tri, tri);
        assert!((bary_uv(&mesh, tri, point.bary).x * 32.0 + 0.5).abs() < 1e-3);
    }

    #[test]
    fn mirrored_and_stacked_uvs_are_reported_as_overlap() {
        let mesh = mesh_of(samples::mirrored_quads());
        let map = TexelMap::build(&mesh, SET, 64, 64, 2);
        assert_eq!(map.stats.covered_texels, 32 * 32);
        // Quad B covers the same texels as quad A; only the centres on B's diagonal are not counted.
        assert_eq!(map.stats.overlapped_texels, 32 * 32 - 32);
        assert_eq!(map.stats.mirrored_tris, 2);
        // First writer wins: every covered texel belongs to quad A.
        for y in 0..64 {
            for x in 0..64 {
                if let Some((tri, false)) = map.owner(x, y) {
                    assert!(tri < 2);
                }
            }
        }
        // The non-owner quad is still rasterized over the same texels, and reaches the same gutter.
        let texels = |tris: [TriId; 2], gutter: bool| -> BTreeSet<(u32, u32)> {
            tris.iter()
                .flat_map(|&t| all_fragments(&mesh, &map, t))
                .filter(|f| f.gutter == gutter)
                .map(|f| (f.x, f.y))
                .collect()
        };
        let (a, b) = (texels([0, 1], false), texels([2, 3], false));
        assert_eq!(a.len(), 32 * 32);
        assert_eq!(a, b);
        let (a_gutter, b_gutter) = (texels([0, 1], true), texels([2, 3], true));
        assert_eq!(a_gutter.len(), 36 * 36 - 32 * 32);
        assert_eq!(a_gutter, b_gutter);

        // No overlap in an ordinary atlas, also at resolutions where texel centres fall on shared edges.
        let sphere = mesh_of(samples::uv_sphere(16, 8, 1.0));
        for (w, h) in [(64u32, 64u32), (128, 64), (96, 80)] {
            let map = TexelMap::build(&sphere, SET, w, h, 2);
            assert_eq!(map.stats.overlapped_texels, 0, "{w}x{h}");
            assert_eq!(map.stats.mirrored_tris, 0);
        }
        // Three stacked triangles: each texel is counted once.
        let stacked = mesh_of(one_material(vec![
            uv_tri([[0.1, 0.9], [0.9, 0.9], [0.1, 0.1]], 0.0),
            uv_tri([[0.1, 0.9], [0.9, 0.9], [0.1, 0.1]], 2.0),
            uv_tri([[0.1, 0.9], [0.9, 0.9], [0.1, 0.1]], 4.0),
        ]));
        let map = TexelMap::build(&stacked, SET, 64, 64, 0);
        assert!(map.stats.overlapped_texels > 0 && map.stats.overlapped_texels <= map.stats.covered_texels);
        assert_eq!(counts(&map), (map.stats.covered_texels, 0), "the temporary overlap marks are cleared");
    }

    /// The per-texel scan the interval rasterizer replaced: every unwrapped texel of the box, first writer wins.
    /// Returns (owner array after step 1 without gutter, covered, overlapped).
    fn reference_step1(mesh: &MeshData, width: u32, height: u32) -> (Vec<u32>, u64, u64) {
        let (w, h) = (width as i64, height as i64);
        let mut owner = vec![TexelMap::NO_OWNER; (width * height) as usize];
        let (mut covered, mut overlapped) = (0, 0);
        for tri in 0..mesh.tri_count() {
            if mesh.tri_set(tri) != Some(SET) {
                continue;
            }
            let Some(s) = TriSetup::new(mesh.tri_uvs(tri), width, height) else { continue };
            let (x0, x1, y0, y1) = s.scan(0);
            for y in y0..y1 {
                let row = s.row(y);
                for x in x0..x1 {
                    let e = s.eval(x, &row);
                    if !s.inside(&e) {
                        continue;
                    }
                    let i = (y.rem_euclid(h) * w + x.rem_euclid(w)) as usize;
                    let o = owner[i];
                    if o == TexelMap::NO_OWNER {
                        owner[i] = tri;
                        covered += 1;
                    } else if o & GUTTER_BIT == 0 && o != tri && s.well_inside(&e) {
                        owner[i] = o | GUTTER_BIT;
                        overlapped += 1;
                    }
                }
            }
        }
        for o in owner.iter_mut().filter(|o| **o != TexelMap::NO_OWNER) {
            *o &= TRI_MASK;
        }
        (owner, covered, overlapped)
    }

    /// The per-texel `raster_tri` the interval rasterizer replaced (owner rule only, no shared gutter).
    fn reference_raster(mesh: &MeshData, map: &TexelMap, tri: TriId, clip: Option<[f32; 4]>) -> Vec<(u32, u32, bool)> {
        let (w, h) = (map.width as i64, map.height as i64);
        let mut out = Vec::new();
        let Some(s) = TriSetup::new(mesh.tri_uvs(tri), map.width, map.height) else { return out };
        let (gx0, gx1, gy0, gy1) = s.scan(map.gutter as i64);
        let (mut x0, mut x1, mut y0, mut y1) = (gx0, gx1, gy0, gy1);
        if let Some(c) = clip {
            x0 = x0.max(c[0].floor() as i64);
            y0 = y0.max(c[1].floor() as i64);
            x1 = x1.min(c[2].floor() as i64 + 1);
            y1 = y1.min(c[3].floor() as i64 + 1);
        }
        let ambiguous = gx1 - gx0 > w || gy1 - gy0 > h;
        for y in y0..y1 {
            let row = s.row(y);
            for x in x0..x1 {
                let (wx, wy) = (x.rem_euclid(w), y.rem_euclid(h));
                let e = s.eval(x, &row);
                if s.inside(&e) {
                    out.push((wx as u32, wy as u32, false));
                    continue;
                }
                let o = map.owner[(wy * w + wx) as usize];
                if o == TexelMap::NO_OWNER || o & TRI_MASK != tri {
                    continue;
                }
                if ambiguous && map.best_image(&s, wx, wy) != (x, y) {
                    continue;
                }
                out.push((wx as u32, wy as u32, o & GUTTER_BIT != 0));
            }
        }
        out.sort();
        out
    }

    /// A row of unit quads whose UVs repeat `span` times in both directions (tiled walls and floors).
    fn tiled_quads(quads: usize, span: f32) -> RawMesh {
        let mut tris = Vec::new();
        for q in 0..quads {
            let x = 2.0 * q as f32;
            tris.push(uv_tri([[0.0, 0.0], [span, 0.0], [0.0, span]], x));
            let mut t = uv_tri([[span, 0.0], [span, span], [0.0, span]], x);
            t.positions = [Vec3::new(x + 1.0, 0.0, 0.0), Vec3::new(x + 1.0, 1.0, 0.0), Vec3::new(x, 1.0, 0.0)];
            tris.push(t);
        }
        one_material(tris)
    }

    #[test]
    fn interval_rasterizer_matches_the_per_texel_scan() {
        let meshes = [
            samples::seam_cube(1.0),
            samples::uv_sphere(16, 8, 1.0),
            samples::wrapped_quad(),
            samples::tight_islands(),
            samples::two_material_model(),
            tiled_quads(2, 3.0),
            tiled_quads(1, 2.37),
            one_material(vec![
                uv_tri([[0.1, 0.1], [7.3, 0.4], [0.2, 0.15]], 0.0),
                uv_tri([[-2.2, 0.3], [0.4, 3.9], [0.3, 0.2]], 2.0),
                uv_tri([[0.05, 0.45], [0.45, 0.45], [0.05, 0.05]], 4.0),
            ]),
        ];
        for raw in meshes {
            let mesh = mesh_of(raw);
            for (w, h, g) in [(64u32, 64u32, 2u8), (37, 23, 3), (8, 4, 16)] {
                let map = TexelMap::build(&mesh, SET, w, h, g);
                let (owner, covered, overlapped) = reference_step1(&mesh, w, h);
                assert_eq!((map.stats.covered_texels, map.stats.overlapped_texels), (covered, overlapped), "{w}x{h}");
                for (i, &o) in owner.iter().enumerate() {
                    if o != TexelMap::NO_OWNER {
                        assert_eq!(map.owner[i], o, "texel {i} at {w}x{h}");
                    }
                }
                for tri in 0..mesh.tri_count() {
                    if mesh.tri_set(tri) != Some(SET) {
                        continue;
                    }
                    for clip in [None, Some([-5.3f32, 2.0, 20.7, 30.1]), Some([-90.0, -70.0, 10.0, 10.0])] {
                        let mut frags: Vec<(u32, u32, bool)> = Vec::new();
                        raster_tri(&mesh, &map, tri, clip, 0..h, &mut |f| frags.push((f.x, f.y, f.gutter)));
                        frags.sort();
                        let mut expect = reference_raster(&mesh, &map, tri, clip);
                        if !map.shared_gutter.is_empty() {
                            // Shared gutter texels are additionally visited by the non-owners.
                            frags.retain(|&(x, y, g)| !g || map.owner(x, y).unwrap().0 == tri);
                            expect.retain(|&(x, y, g)| !g || map.owner(x, y).unwrap().0 == tri);
                        }
                        assert_eq!(frags, expect, "triangle {tri} at {w}x{h}, clip {clip:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn tiled_uvs_build_in_one_pass_over_the_image() {
        // One quad whose UVs repeat 20 times: before, every triangle cost (tiles covered) x w x h texel tests.
        let mesh = mesh_of(tiled_quads(1, 20.0));
        let start = std::time::Instant::now();
        let map = TexelMap::build(&mesh, SET, 1024, 1024, TexelMap::default_gutter(1024, 1024));
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs_f64() < 1.0, "build took {elapsed:?}");
        assert_eq!(map.stats.covered_texels, 1024 * 1024);
        // Painting and island lookups on such a triangle stay cheap too: one pass over the image, not over every tile.
        let start = std::time::Instant::now();
        let mut n = 0u64;
        raster_tri(&mesh, &map, 0, Some([100.0, 100.0, 140.0, 140.0]), 0..1024, &mut |_| n += 1);
        assert!(n > 0);
        for tri in 0..2 {
            let p = map.locate(&mesh, 5, 7 + tri).unwrap();
            assert!(p.bary.iter().all(|b| b.is_finite()));
        }
        assert!(start.elapsed().as_secs_f64() < 1.0, "raster took {:?}", start.elapsed());
        // A cancelled build stops and reports it.
        let calls = std::sync::atomic::AtomicU32::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 1;
        assert!(TexelMap::build_cancellable(&mesh, SET, 1024, 1024, 4, &cancel).is_none());
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2, "polled once per triangle");
    }

    #[test]
    fn mirrored_halves_both_reach_the_shared_gutter() {
        // mirrored_quads at 64 px: island columns and rows 16..=47, quad B (triangles 2, 3) lies on top of quad A.
        let mesh = mesh_of(samples::mirrored_quads());
        let map = TexelMap::build(&mesh, SET, 64, 64, 2);
        assert_eq!(map.owner(15, 32), Some((1, true)), "the gutter belongs to the first writer");
        for tri in [2u32, 3] {
            for f in all_fragments(&mesh, &map, tri).iter().filter(|f| f.gutter) {
                // The barycentrics are extrapolated for this triangle and still reproduce the texel centre.
                let (dx, dy) = centre_error(bary_uv(&mesh, tri, f.bary), f.x, f.y, 64, 64);
                assert!(dx < 1e-2 && dy < 1e-2, "{f:?} of triangle {tri}");
                assert!(f.bary.iter().any(|b| *b < 0.0));
            }
        }
        // Left gutter columns 14 and 15 of row 32 are visited by quad B as well.
        let b: BTreeSet<(u32, u32)> =
            [2, 3].iter().flat_map(|&t| all_fragments(&mesh, &map, t)).map(|f| (f.x, f.y)).collect();
        assert!(b.contains(&(14, 32)) && b.contains(&(15, 32)));
        // Row bands still partition the fragments.
        for tri in 0..4 {
            let mut all: Vec<_> = all_fragments(&mesh, &map, tri).iter().map(|f| (f.y, f.x, f.gutter)).collect();
            let mut banded = Vec::new();
            for band in 0..4u32 {
                raster_tri(&mesh, &map, tri, None, band * 16..(band + 1) * 16, &mut |f| banded.push((f.y, f.x, f.gutter)));
            }
            all.sort();
            banded.sort();
            assert_eq!(all, banded);
        }
    }

    #[test]
    fn locate_round_trips_to_the_texel_centre() {
        for raw in [
            samples::seam_cube(1.0),
            samples::uv_sphere(24, 12, 1.5),
            samples::wrapped_quad(),
            samples::two_material_model(),
        ] {
            let mesh = mesh_of(raw);
            let (w, h) = (128u32, 96u32);
            let map = TexelMap::build(&mesh, SET, w, h, 3);
            let (mut covered, mut gutter) = (0, 0);
            for y in 0..h {
                for x in 0..w {
                    let Some((tri, is_gutter)) = map.owner(x, y) else {
                        assert!(map.locate(&mesh, x, y).is_none());
                        continue;
                    };
                    let p = map.locate(&mesh, x, y).unwrap();
                    assert_eq!(p.tri, tri);
                    assert!((p.bary[0] + p.bary[1] + p.bary[2] - 1.0).abs() < 1e-4);
                    assert!((p.normal.length() - 1.0).abs() < 1e-4);
                    // Re-derive the UV from the barycentrics: the texel centre, up to whole tiles.
                    let (dx, dy) = centre_error(bary_uv(&mesh, tri, p.bary), x, y, w, h);
                    assert!(dx < 0.5 && dy < 0.5, "texel ({x}, {y}) of triangle {tri}: off by ({dx}, {dy})");
                    assert!(dx < 1e-2 && dy < 1e-2);
                    let pos = mesh.tri_positions(tri);
                    let expect = pos[0] * p.bary[0] + pos[1] * p.bary[1] + pos[2] * p.bary[2];
                    assert!((p.position - expect).length() < 1e-5);
                    if is_gutter {
                        gutter += 1;
                        assert!(p.bary.iter().any(|b| *b < 0.0), "gutter texels lie outside their triangle");
                    } else {
                        covered += 1;
                        assert!(p.bary.iter().all(|b| *b >= -1e-3 && *b <= 1.0 + 1e-3));
                    }
                }
            }
            // (The wrapped quad covers the whole texture and leaves no room for a gutter.)
            assert!(covered > 1000 && (gutter > 100 || covered == w * h), "{covered} covered, {gutter} gutter");
        }
        // 2D -> 3D on the flat quad: u = x, v = 1 - y.
        let quad = mesh_of(samples::flat_quad());
        let map = TexelMap::build(&quad, SET, 16, 16, 2);
        let p = map.locate(&quad, 3, 12).unwrap();
        assert!((p.position - Vec3::new(3.5 / 16.0, 1.0 - 12.5 / 16.0, 0.0)).length() < 1e-6);
        assert_eq!(p.normal, Vec3::Z);
        assert!(map.locate(&quad, 16, 0).is_none());
    }

    #[test]
    fn sliver_fallback_claims_the_centroid_texel() {
        // A normal triangle, a sliver thinner than a texel that covers no centre, and a sliver hidden under the first.
        let raw = one_material(vec![
            uv_tri([[0.05, 0.45], [0.45, 0.45], [0.05, 0.05]], 0.0),
            uv_tri([[0.52, 0.704], [0.92, 0.704], [0.52, 0.701]], 2.0),
            uv_tri([[0.1, 0.404], [0.3, 0.404], [0.1, 0.401]], 4.0),
        ]);
        let mesh = mesh_of(raw);
        assert!(mesh.tri_flags.iter().all(|f| f.is_empty()), "thin, but not degenerate");
        let map = TexelMap::build(&mesh, SET, 64, 64, 2);
        // Row 44 has its centre at 44.5 / 64 = 0.695, row 45 at 0.711: the sliver at v ~ 0.703 lies between them.
        let sliver_texels: Vec<(u32, u32)> = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|&(x, y)| map.owner(x, y) == Some((1, false)))
            .collect();
        // Its centroid is (0.653, 0.703) -> texel (41, 44).
        assert_eq!(sliver_texels, vec![(41, 44)]);
        // The hidden sliver's centroid texel already belongs to triangle 0: it gets nothing.
        assert!((0..64).all(|y| (0..64).all(|x| map.owner(x, y).is_none_or(|o| o.0 != 2))));
        assert_eq!(map.owner(8, 25), Some((0, false)));
        assert_eq!(map.owner(10, 25), Some((0, false)));
        // The claimed texel is reported by raster_tri with the centroid's barycentrics, and it has a gutter.
        let frags = all_fragments(&mesh, &map, 1);
        let own: Vec<&Fragment> = frags.iter().filter(|f| !f.gutter).collect();
        assert_eq!(own.len(), 1);
        assert_eq!((own[0].x, own[0].y), (41, 44));
        assert_eq!(own[0].bary, CENTROID);
        assert_eq!(frags.iter().filter(|f| f.gutter).count(), 24, "a 5x5 block around the claimed texel");
        assert!(frags.iter().all(|f| f.bary.iter().all(|b| b.is_finite())));
        let p = map.locate(&mesh, 41, 44).unwrap();
        assert_eq!((p.tri, p.bary), (1, CENTROID));
        assert!((p.position - Vec3::new(2.0 + 1.0 / 3.0, 1.0 / 3.0, 0.0)).length() < 1e-6);
        // The hidden sliver still rasterizes nothing it does not cover.
        assert!(all_fragments(&mesh, &map, 2).is_empty());
        assert_eq!(map.stats.covered_texels, counts(&map).0);
    }

    #[test]
    fn fragment_barycentrics_sum_to_one() {
        for raw in [samples::seam_cube(1.0), samples::uv_sphere(12, 6, 1.0), samples::wrapped_quad()] {
            let mesh = mesh_of(raw);
            let map = TexelMap::build(&mesh, SET, 96, 64, 2);
            let mut visited: BTreeSet<(u32, u32)> = BTreeSet::new();
            for tri in 0..mesh.tri_count() {
                for f in all_fragments(&mesh, &map, tri) {
                    assert!(f.x < 96 && f.y < 64);
                    assert!((f.bary[0] + f.bary[1] + f.bary[2] - 1.0).abs() < 1e-5, "{f:?}");
                    // The barycentrics reproduce the texel centre (modulo whole tiles).
                    let (dx, dy) = centre_error(bary_uv(&mesh, tri, f.bary), f.x, f.y, 96, 64);
                    assert!(dx < 1e-2 && dy < 1e-2, "{f:?} of triangle {tri}");
                    if f.gutter {
                        // A gutter texel belongs to its owner, and to every other triangle covering its source texel.
                        let (owner, is_gutter) = map.owner(f.x, f.y).unwrap();
                        assert!(is_gutter);
                        if owner != tri {
                            let src = map.shared_source((f.y * 96 + f.x) as usize).expect("a shared source");
                            let (sx, sy) = ((src % 96) as u32, (src / 96) as u32);
                            let covers = all_fragments(&mesh, &map, tri).iter().any(|c| !c.gutter && (c.x, c.y) == (sx, sy));
                            assert!(covers, "{f:?}: triangle {tri} does not cover the source ({sx}, {sy})");
                        }
                        assert!(f.bary.iter().any(|b| *b < 0.0));
                    } else {
                        assert!(f.bary.iter().all(|b| *b >= -1e-3));
                        assert!(matches!(map.owner(f.x, f.y), Some((_, false))), "covered texels are never gutter");
                    }
                    visited.insert((f.x, f.y));
                }
            }
            // Together the triangles of the set visit exactly the owned texels.
            for y in 0..64 {
                for x in 0..96 {
                    assert_eq!(map.owner(x, y).is_some(), visited.contains(&(x, y)), "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn clip_and_rows_restrict_the_scan() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let map = TexelMap::build(&mesh, SET, 128, 128, 2);
        for tri in [0u32, 5, 11] {
            let key = |f: &Fragment| (f.y, f.x, f.bary.map(f32::to_bits), f.gutter);
            let all: Vec<_> = all_fragments(&mesh, &map, tri).iter().map(key).collect();
            assert!(all.len() > 500);
            // Row bands partition the fragments; each band reports exactly its rows, with identical barycentrics.
            let mut banded = Vec::new();
            for band in 0..8u32 {
                let rows = band * 16..(band + 1) * 16;
                let mut part = Vec::new();
                raster_tri(&mesh, &map, tri, None, rows.clone(), &mut |f| part.push(f));
                assert!(part.iter().all(|f| rows.contains(&f.y)));
                banded.extend(part.iter().map(key));
            }
            let (mut a, mut b) = (all.clone(), banded);
            a.sort();
            b.sort();
            assert_eq!(a, b);
            // A clip box selects the texels touching it, again bit-identical to the unclipped run.
            let clip = [20.3f32, 10.0, 31.9, 47.2];
            let mut clipped = Vec::new();
            raster_tri(&mesh, &map, tri, Some(clip), 0..128, &mut |f| clipped.push(key(&f)));
            let mut expect: Vec<_> =
                all.iter().copied().filter(|&(y, x, _, _)| (20..=31).contains(&x) && (10..=47).contains(&y)).collect();
            clipped.sort();
            expect.sort();
            assert_eq!(clipped, expect);
            assert_eq!(tri == 0, !clipped.is_empty(), "the box lies in island 0");
            // Empty and invalid restrictions visit nothing.
            let mut none = 0;
            raster_tri(&mesh, &map, tri, None, 0..0, &mut |_| none += 1);
            raster_tri(&mesh, &map, tri, None, 200..300, &mut |_| none += 1);
            raster_tri(&mesh, &map, tri, Some([500.0, 500.0, 600.0, 600.0]), 0..128, &mut |_| none += 1);
            raster_tri(&mesh, &map, tri, Some([30.0, 30.0, 20.0, 40.0]), 0..128, &mut |_| none += 1);
            raster_tri(&mesh, &map, tri, Some([f32::NAN, 0.0, 10.0, 10.0]), 0..128, &mut |_| none += 1);
            assert_eq!(none, 0);
            // A huge box and an over-long row range are the same as no restriction.
            let mut huge = Vec::new();
            let everything = Some([-1e30, -1e30, 1e30, f32::INFINITY]);
            raster_tri(&mesh, &map, tri, everything, 0..u32::MAX, &mut |f| huge.push(key(&f)));
            assert_eq!(huge, all);
        }
        // Unknown triangles and triangles of another set or without UVs visit nothing.
        let mut n = 0;
        raster_tri(&mesh, &map, 12, None, 0..128, &mut |_| n += 1);
        raster_tri(&mesh, &map, u32::MAX, None, 0..128, &mut |_| n += 1);
        let two = mesh_of(samples::two_material_model());
        let body = TexelMap::build(&two, SET, 64, 64, 2);
        raster_tri(&two, &body, 12, None, 0..64, &mut |_| n += 1); // a "Head" triangle
        let tetra = mesh_of(samples::no_uv_tetra());
        let empty = TexelMap::build(&tetra, SET, 16, 16, 2);
        raster_tri(&tetra, &empty, 0, None, 0..16, &mut |_| n += 1);
        assert_eq!(n, 0);
        assert!(body.stats.covered_texels > 0);
        // Each set has its own map: the head's map is owned by head triangles only.
        let head = TexelMap::build(&two, TextureSetId(1), 64, 64, 2);
        assert_eq!(head.owner(1, 20).map(|o| o.0 >= 12), Some(true));
        assert!(!all_fragments(&two, &head, 12).is_empty());
        assert!(all_fragments(&two, &head, 0).is_empty());
    }

    #[test]
    fn wrapped_quad_writes_at_wrapped_coordinates() {
        let mesh = mesh_of(samples::wrapped_quad());
        let (w, h) = (64u32, 64u32);
        let map = TexelMap::build(&mesh, SET, w, h, 2);
        // Quad A spans u, v in 0.5..1.5: folded onto the tile it covers every texel exactly once.
        assert_eq!(map.stats.covered_texels, 64 * 64);
        assert!((map.stats.coverage - 1.0).abs() < 1e-6);
        assert_eq!(counts(&map), (64 * 64, 0));
        for y in 0..h {
            for x in 0..w {
                assert!(map.owner(x, y).unwrap().0 < 2, "quad A was rasterized first");
            }
        }
        // Quad B (0.25..0.75 after folding) lies completely under A.
        assert_eq!(map.stats.overlapped_texels, 32 * 32 - 32, "B's texels except the centres on its diagonal");
        // Each triangle of A reports wrapped coordinates with barycentrics of the unwrapped position.
        let mut seen: BTreeSet<(u32, u32)> = BTreeSet::new();
        for tri in 0..2 {
            let frags = all_fragments(&mesh, &map, tri);
            assert!(frags.iter().all(|f| !f.gutter && f.x < w && f.y < h));
            for f in &frags {
                // The barycentrics describe a point of the triangle in its own (partly negative or > 1) UV range ...
                assert!(f.bary.iter().all(|b| *b >= -1e-3 && *b <= 1.0 + 1e-3));
                // ... which folds onto the reported texel.
                let u = bary_uv(&mesh, tri, f.bary);
                assert_eq!(((u.x * w as f32).floor() as i32).rem_euclid(w as i32) as u32, f.x);
                assert_eq!(((u.y * h as f32).floor() as i32).rem_euclid(h as i32) as u32, f.y);
                seen.insert((f.x, f.y));
            }
            // Both sides of the wrap are present.
            assert!(frags.iter().any(|f| f.x < 8) && frags.iter().any(|f| f.x >= 56));
            assert!(frags.iter().any(|f| f.y < 8) && frags.iter().any(|f| f.y >= 56));
        }
        assert_eq!(seen.len(), 64 * 64);
        // A wrapping triangle is found in every row band it touches.
        for band in 0..4u32 {
            let mut n = 0;
            raster_tri(&mesh, &map, 0, None, band * 16..(band + 1) * 16, &mut |_| n += 1);
            assert!(n > 0, "band {band}");
        }
        // The clip box is in the triangle's unwrapped space: triangle 0 has u in -0.5..0.5 (pixels -32..32).
        let mut left = Vec::new();
        raster_tri(&mesh, &map, 0, Some([-32.0, -40.0, -0.5, 40.0]), 0..h, &mut |f| left.push(f));
        assert!(!left.is_empty() && left.iter().all(|f| f.x >= 32), "negative u folds onto the right half");
        // `locate` resolves the wrap too.
        let p = map.locate(&mesh, 60, 3).unwrap();
        let (dx, dy) = centre_error(bary_uv(&mesh, p.tri, p.bary), 60, 3, w, h);
        assert!(dx < 1e-2 && dy < 1e-2);
    }

    #[test]
    fn gutters_of_tight_islands_touch() {
        let mesh = mesh_of(samples::tight_islands());
        let gutter = TexelMap::default_gutter(256, 256);
        assert_eq!(gutter, 2);
        let map = TexelMap::build(&mesh, SET, 256, 256, gutter);
        for y in 64..192 {
            // Island A ends at column 126, island B starts at column 129; the two free columns are gutter.
            assert_eq!(map.owner(126, y).map(|o| (o.0 < 2, o.1)), Some((true, false)));
            assert_eq!(map.owner(127, y).map(|o| (o.0 < 2, o.1)), Some((true, true)));
            assert_eq!(map.owner(128, y).map(|o| (o.0 >= 2, o.1)), Some((true, true)));
            assert_eq!(map.owner(129, y).map(|o| (o.0 >= 2, o.1)), Some((true, false)));
        }
        assert_eq!(map.stats.overlapped_texels, 0);
        // Each island's fragments (covered and gutter) stay on its own side.
        for tri in 0..4 {
            for f in all_fragments(&mesh, &map, tri) {
                assert_eq!(f.x <= 127, tri < 2, "{f:?} of triangle {tri}");
            }
        }
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        for (w, h) in [(0u32, 0u32), (0, 16), (16, 0)] {
            let map = TexelMap::build(&mesh, SET, w, h, 2);
            assert_eq!(map.stats.covered_texels, 0);
            assert_eq!(map.stats.coverage, 0.0);
            assert_eq!(map.owner(0, 0), None);
            assert!(map.locate(&mesh, 0, 0).is_none());
            let mut n = 0;
            raster_tri(&mesh, &map, 0, None, 0..16, &mut |_| n += 1);
            assert_eq!(n, 0);
        }
        // 1x1: no triangle covers the single centre (it lies between the island rows), so the first sliver claims it.
        let map = TexelMap::build(&mesh, SET, 1, 1, 2);
        assert_eq!(map.stats.covered_texels, 1);
        assert!(map.locate(&mesh, 0, 0).is_some());
        // Tiny maps with large gutters: the gutter stops when the image is full.
        let map = TexelMap::build(&mesh, SET, 8, 4, 16);
        assert_eq!(counts(&map).0 + counts(&map).1, 32);
        for tri in 0..12 {
            let frags = all_fragments(&mesh, &map, tri);
            assert!(frags.iter().all(|f| f.x < 8 && f.y < 4 && f.bary.iter().all(|b| b.is_finite())));
            // The same wrapped texel is never reported twice through the ownership rule, even though the scan
            // range is wider than the image.
            let mut gutter_seen = BTreeSet::new();
            for f in frags.iter().filter(|f| f.gutter) {
                assert!(gutter_seen.insert((f.x, f.y)), "gutter texel reported twice for triangle {tri}");
            }
        }
        // Every owned texel is still reported by its owner.
        let mut reported: BTreeMap<(u32, u32), TriId> = BTreeMap::new();
        for tri in 0..12 {
            for f in all_fragments(&mesh, &map, tri) {
                if map.owner(f.x, f.y).map(|o| o.0) == Some(tri) {
                    reported.insert((f.x, f.y), tri);
                }
            }
        }
        assert_eq!(reported.len(), 32);
        // UVs far outside the supported range are ignored instead of looping forever.
        let far = mesh_of(one_material(vec![uv_tri([[0.0, 0.0], [3.0e6, 0.2], [0.1, 2.0e6]], 0.0)]));
        let map = TexelMap::build(&far, SET, 16, 16, 2);
        assert!(map.stats.covered_texels <= 256);
        let mut n = 0u32;
        raster_tri(&far, &map, 0, None, 0..16, &mut |_| n += 1);
        assert!(n <= 16 * 16 * 100);
    }

    /// The fragments of `raster_tri_with` as comparable tuples.
    fn fragments_with(mesh: &MeshData, map: &TexelMap, tri: TriId, clip: Option<[f32; 4]>, rows: std::ops::Range<u32>, indexed_from: i64) -> Vec<(u32, u32, [u32; 3], bool)> {
        let mut out = Vec::new();
        raster_tri_with(mesh, map, tri, clip, rows, indexed_from, None::<&mut fn(u32, u32, u32)>, None, false, &mut |f| out.push((f.x, f.y, f.bary.map(f32::to_bits), f.gutter)));
        out
    }

    #[test]
    fn indexed_pass2_visits_exactly_what_the_box_scan_visits() {
        // Tiled quads (boxes wider than the image), mirrored / stacked copies (shared gutters), slivers and a plain
        // atlas, with whole-triangle, partial and wrapping clip boxes and row bands.
        let mut tris = vec![
            uv_tri([[0.05, 0.05], [3.3, 0.15], [0.1, 0.25]], 0.0), // tiled along u, wider than the image
            uv_tri([[0.6, 0.7], [2.9, 0.72], [0.62, 0.95]], 2.0),
            uv_tri([[-0.3, 0.45], [2.4, 0.5], [-0.2, 0.55]], 4.0), // negative UVs, wraps
            uv_tri([[0.7, -0.6], [0.75, 1.9], [0.8, -0.5]], 6.0), // tiled along v
        ];
        tris.push(uv_tri([[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]], 10.0));
        tris.push(uv_tri([[0.4, 0.1], [0.1, 0.1], [0.1, 0.4]], 11.0)); // mirrored copy: shared gutter
        tris.push(uv_tri([[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]], 12.0)); // stacked copy
        tris.push(uv_tri([[0.3, 0.6], [0.3004, 0.6], [0.3, 0.6003]], 13.0)); // sliver
        let mesh = mesh_of(one_material(tris));
        let (mut gutter_fragments, mut mixed) = (0usize, 0usize);
        for (size, gutter) in [(64u32, 2u8), (37, 3), (128, 4)] {
            let map = TexelMap::build(&mesh, SET, size, size, gutter);
            let f = size as f32;
            let clips = [
                None,
                Some([-5.0, -5.0, 3.0 * f, 2.5 * f]),
                Some([0.3 * f, 0.2 * f, 0.9 * f, 0.7 * f]),
                Some([-0.4 * f, 0.6 * f, 0.2 * f, 1.3 * f]),
            ];
            for tri in 0..mesh.tri_count() {
                for clip in clips {
                    for rows in [0..size, size / 3..size / 2 + 1] {
                        let scan = fragments_with(&mesh, &map, tri, clip, rows.clone(), i64::MAX);
                        let indexed = fragments_with(&mesh, &map, tri, clip, rows.clone(), 0);
                        assert_eq!(indexed, scan, "size {size}, tri {tri}, clip {clip:?}, rows {rows:?}");
                        // The folded form: the same pass-2 fragments, and each pass-1 position once.
                        let (mut once, mut pass2) = (Vec::new(), Vec::new());
                        raster_tri_covered(&mesh, &map, tri, clip, rows.clone(), None, &mut |y, a, b| once.extend((a..b).map(|x| (x, y))), &mut |f| {
                            pass2.push((f.x, f.y, f.bary.map(f32::to_bits), f.gutter))
                        });
                        let unique_once: BTreeSet<(u32, u32)> = once.iter().copied().collect();
                        assert_eq!(unique_once.len(), once.len(), "a covered texel was reported twice");
                        let inside: BTreeSet<(u32, u32)> = scan.iter().filter(|f| !f.3 && f.2 != CENTROID.map(f32::to_bits)).map(|f| (f.0, f.1)).collect();
                        assert_eq!(unique_once, inside, "size {size}, tri {tri}, clip {clip:?}, rows {rows:?}");
                        let scan_pass2: Vec<_> = scan.iter().copied().filter(|f| f.3 || f.2 == CENTROID.map(f32::to_bits)).collect();
                        assert_eq!(pass2, scan_pass2);
                        // Folding only some blocks: no block folded gives the fragments of `raster_tri`; a checkerboard
                        // of folded blocks reports every pass-1 position through one of the two sinks, each folded one
                        // once, and the unfolded fragments are fragments of `raster_tri` (barycentrics included).
                        let picks: [fn([i64; 4]) -> bool; 2] = [|_| false, |_| true];
                        for pick in picks {
                            let (mut once, mut frags) = (Vec::new(), Vec::new());
                            raster_tri_covered(&mesh, &map, tri, clip, rows.clone(), Some(&pick), &mut |y, a, b| once.extend((a..b).map(|x| (x, y))), &mut |f| {
                                frags.push((f.x, f.y, f.bary.map(f32::to_bits), f.gutter))
                            });
                            if pick([0; 4]) {
                                assert_eq!(once.iter().copied().collect::<BTreeSet<_>>(), inside);
                                assert_eq!(frags, scan_pass2);
                            } else {
                                // Pass 1 grouped by wrapped row, then pass 2 in order.
                                assert!(once.is_empty());
                                let mut pass1: Vec<_> = frags[..frags.len() - scan_pass2.len()].to_vec();
                                let mut want: Vec<_> = scan[..scan.len() - scan_pass2.len()].to_vec();
                                pass1.sort_unstable();
                                want.sort_unstable();
                                assert_eq!(pass1, want);
                                assert!(frags.ends_with(&scan_pass2));
                            }
                        }
                        let checker = |b: [i64; 4]| {
                            b[2] - b[0] <= FOLD_BLOCK && b[3] - b[1] <= FOLD_BLOCK && (b[0].div_euclid(FOLD_BLOCK) + b[1].div_euclid(FOLD_BLOCK)).rem_euclid(2) == 0
                        };
                        let (mut once, mut frags) = (Vec::new(), Vec::new());
                        raster_tri_covered(&mesh, &map, tri, clip, rows.clone(), Some(&checker), &mut |y, a, b| once.extend((a..b).map(|x| (x, y))), &mut |f| {
                            frags.push((f.x, f.y, f.bary.map(f32::to_bits), f.gutter))
                        });
                        let unique_once: BTreeSet<(u32, u32)> = once.iter().copied().collect();
                        assert_eq!(unique_once.len(), once.len(), "a covered texel was reported twice");
                        let all: BTreeSet<_> = scan.iter().copied().collect();
                        assert!(frags.iter().all(|f| all.contains(f)), "a fragment raster_tri does not report");
                        let frag_inside: BTreeSet<(u32, u32)> = frags.iter().filter(|f| !f.3 && f.2 != CENTROID.map(f32::to_bits)).map(|f| (f.0, f.1)).collect();
                        assert_eq!(unique_once.union(&frag_inside).copied().collect::<BTreeSet<_>>(), inside);
                        assert!(frags.ends_with(&scan_pass2));
                        mixed += usize::from(!once.is_empty() && !frag_inside.is_empty());
                        gutter_fragments += scan.iter().filter(|f| f.3).count();
                    }
                }
            }
            assert!(!map.shared_gutter.is_empty(), "the copies share gutter texels");
        }
        assert!(gutter_fragments > 1000, "pass 2 reported only {gutter_fragments} gutter fragments");
        assert!(mixed > 10, "only {mixed} calls folded some blocks and not others");
    }

    #[test]
    fn texel_world_is_the_mean_density() {
        // flat_quad: 1 world unit maps to the whole texture.
        let quad = mesh_of(samples::flat_quad());
        let map = TexelMap::build(&quad, SET, 256, 256, 2);
        assert!((map.stats.texel_world - 1.0 / 256.0).abs() < 1e-7);
        assert_eq!(map.stats.covered_texels, 256 * 256);
        assert!((map.stats.coverage - 1.0).abs() < 1e-6);
        // Non-square textures use the geometric mean of the two densities.
        let map = TexelMap::build(&quad, SET, 512, 128, 2);
        assert!((map.stats.texel_world - 1.0 / 256.0).abs() < 1e-7);
        // Independent of the gutter.
        let sphere = mesh_of(samples::uv_sphere(32, 16, 1.0));
        let a = TexelMap::build(&sphere, SET, 256, 256, 0).stats;
        let b = TexelMap::build(&sphere, SET, 256, 256, 8).stats;
        assert_eq!(a, b);
        // Sphere area ~ 4 pi over ~ 256^2 texels.
        let expect = (4.0 * std::f32::consts::PI / 65536.0).sqrt();
        assert!((a.texel_world - expect).abs() < 0.05 * expect, "{} vs {expect}", a.texel_world);
        // The shared, immutable data of a document can cross threads.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TexelMap>();
        assert_send_sync::<crate::bvh::Bvh>();
        assert_send_sync::<crate::pixel_ops::PremulMips>();
        assert_send_sync::<MeshData>();
    }

    // ----- Round 7 (A1: the lean pass 2 and the brush index) -----

    /// T6: on the maps of round 7's meshes the brush index is complete (and exact): a triangle's bit is set iff it
    /// owns a gutter texel or claimed a texel, and every block with a `shared_gutter` texel has its bit.
    #[test]
    fn prepare_brush_index_is_complete() {
        let mut checked = (0usize, 0usize, 0usize);
        for (name, raw) in r7_test_meshes() {
            let mesh = mesh_of(raw);
            for (w, h, gutter) in [(1u32, 1u32, 0u8), (3, 5, 2), (64, 64, 8), (37, 53, 3), (300, 420, 16), (128, 128, 0)] {
                let map = TexelMap::build(&mesh, SET, w, h, gutter);
                map.prepare_brush_index();
                let tris = map.pass2_tris.get().expect("built");
                let bit = |t: TriId| tris.get(t as usize / 64).is_some_and(|w| w & (1 << (t % 64)) != 0);
                let mut want: BTreeSet<TriId> = map.claimed.iter().copied().collect();
                for &o in &map.owner {
                    if o != TexelMap::NO_OWNER && o & GUTTER_BIT != 0 {
                        want.insert(o & TRI_MASK);
                    }
                }
                for t in 0..mesh.tri_count() + 64 {
                    assert_eq!(bit(t), want.contains(&t), "{name} {w}x{h} g{gutter}: triangle {t}");
                }
                checked.0 += want.len();
                // The claimed triangles are exactly the owners of a covered texel whose centre none of their images
                // covers (the sliver fallback).
                for &t in &map.claimed {
                    assert!(map.owner.contains(&t), "{name}: claimed triangle {t} owns no covered texel");
                }
                checked.1 += map.claimed.len();
                let blocks = map.shared_blocks.get().expect("built");
                let bw = map.shared_blocks_per_row();
                for &(i, _) in &map.shared_gutter {
                    let (x, y) = (i as usize % w as usize, i as usize / w as usize);
                    let b = y / SHARED_BLOCK as usize * bw + x / SHARED_BLOCK as usize;
                    assert!(blocks[b / 64] & (1 << (b % 64)) != 0, "{name}: shared gutter texel ({x}, {y})");
                    checked.2 += 1;
                }
                // Built again: the same (a set union does not depend on the chunks or the thread count).
                let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
                let again = pool.install(|| TexelMap::build(&mesh, SET, w, h, gutter));
                pool.install(|| again.prepare_brush_index());
                assert_eq!(again.pass2_tris.get(), map.pass2_tris.get());
                assert_eq!(again.shared_blocks.get(), map.shared_blocks.get());
            }
        }
        assert!(checked.0 > 100 && checked.1 > 0 && checked.2 > 100, "{checked:?}");
    }

    /// T3: for every triangle of round 7's meshes, random clips (wrapping, wider than the image, empty) and row
    /// ranges (empty ones too), the lean call reports exactly the fragments and covered runs of the full call, in the
    /// same order; and it skips pass 2 for most triangles of a dense mesh.
    #[test]
    fn lean_pass2_visits_the_same_fragments() {
        let mut rng = crate::testutil::XorShift::new(0x7a3);
        let (mut skipped, mut scanned, mut fragments) = (0u64, 0u64, 0usize);
        type Frag = (u32, u32, [u32; 3], bool);
        let run = |mesh: &MeshData, map: &TexelMap, tri: TriId, clip, rows: std::ops::Range<u32>, mode: u8, lean: bool| {
            let (mut frags, mut runs): (Vec<Frag>, Vec<(u32, u32, u32)>) = (Vec::new(), Vec::new());
            let mut f = |f: Fragment| frags.push((f.x, f.y, f.bary.map(f32::to_bits), f.gutter));
            let checker = |b: [i64; 4]| {
                b[2] - b[0] <= FOLD_BLOCK && b[3] - b[1] <= FOLD_BLOCK && (b[0].div_euclid(FOLD_BLOCK) + b[1].div_euclid(FOLD_BLOCK)).rem_euclid(2) == 0
            };
            let cost = match mode {
                0 => raster_tri_lean(mesh, map, tri, clip, rows, lean, &mut f),
                1 => raster_tri_covered_lean(mesh, map, tri, clip, rows, None, lean, &mut |y, a, b| runs.push((y, a, b)), &mut f),
                _ => raster_tri_covered_lean(mesh, map, tri, clip, rows, Some(&checker), lean, &mut |y, a, b| runs.push((y, a, b)), &mut f),
            };
            (frags, runs, cost)
        };
        for (name, raw) in r7_test_meshes() {
            let mesh = mesh_of(raw);
            for (w, h, gutter) in [(64u32, 64u32, 2u8), (37, 53, 3), (16, 9, 0), (128, 96, 8)] {
                let map = TexelMap::build(&mesh, SET, w, h, gutter);
                let (fw, fh) = (w as f32, h as f32);
                for tri in 0..mesh.tri_count() {
                    for k in 0..6 {
                        let clip = match k {
                            0 => None,
                            1 => Some([-0.5 * fw, -0.5 * fh, 2.5 * fw, 2.5 * fh]), // wider than the image
                            2 => Some([5.0, 5.0, 1.0, 9.0]),                         // empty (inverted)
                            _ => {
                                let (cx, cy) = ((rng.next_f32() * 2.0 - 0.5) * fw, (rng.next_f32() * 2.0 - 0.5) * fh);
                                let (rx, ry) = (rng.next_f32() * 0.7 * fw, rng.next_f32() * 0.7 * fh);
                                Some([cx - rx, cy - ry, cx + rx, cy + ry])
                            }
                        };
                        let rows = match rng.next_u32() % 4 {
                            0 => 0..h,
                            1 => {
                                let a = rng.next_u32() % h;
                                a..a // empty
                            }
                            _ => {
                                let a = rng.next_u32() % h;
                                a..(a + 1 + rng.next_u32() % h).min(h)
                            }
                        };
                        for mode in 0..3u8 {
                            let full = run(&mesh, &map, tri, clip, rows.clone(), mode, false);
                            let lean = run(&mesh, &map, tri, clip, rows.clone(), mode, true);
                            assert_eq!(lean.0, full.0, "{name} {w}x{h}: tri {tri}, clip {clip:?}, rows {rows:?}, mode {mode}");
                            assert_eq!(lean.1, full.1, "{name} {w}x{h}: tri {tri}, clip {clip:?}, rows {rows:?}, mode {mode}");
                            assert_eq!(full.2.skipped, 0);
                            skipped += lean.2.skipped;
                            scanned += u64::from(lean.2.skipped == 0);
                            fragments += full.0.len();
                        }
                    }
                }
            }
        }
        assert!(skipped > 1000 && scanned > 1000 && fragments > 10_000, "{skipped} skipped, {scanned} scanned, {fragments}");
    }

    /// The folded row loops walk exactly the offsets whose wrapped row lies in the range, ascending.
    #[test]
    fn folded_rows_are_the_rows_in_range() {
        for h in [1i64, 5, 64] {
            for y0 in [-130i64, -64, -3, 0, 2, 63, 64, 200] {
                for len in [0i64, 1, 3, h - 1, h, h + 7, 3 * h] {
                    for (rs, re) in [(0, h), (0, 0), (1, h.min(3)), (h / 2, h), (h - 1, h)] {
                        let want: Vec<i64> = (0..len.min(h)).filter(|r| (rs..re).contains(&(y0 + r).rem_euclid(h))).collect();
                        assert_eq!(folded_rows(y0, y0 + len, h, rs, re).collect::<Vec<_>>(), want, "h {h} y0 {y0} len {len} rows {rs}..{re}");
                    }
                }
            }
        }
    }
}
