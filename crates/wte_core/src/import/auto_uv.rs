// Owner: core-import (C5). Generated texture layout for models without UVs (ARCHITECTURE.md 6.1 step 4).
//
// Box projection with chart growing and shelf packing:
// 1. every triangle gets the axis direction (+X, -X, +Y, -Y, +Z, -Z) its face normal is closest to;
// 2. charts grow from a seed triangle over shared edges (by position) to neighbours facing within 60 degrees of the
//    seed's axis; a triangle whose projection would overlap the chart's projection so far starts or joins another
//    chart, so no chart overlaps itself (a folded surface is split);
// 3. each chart is projected onto the plane of its axis, oriented so that the world's up direction (+Y) points up
//    in the texture on the side faces, and so that no triangle is mirrored (stretch at most 2x);
// 4. the charts keep their world size (one texel density for the whole model) and are shelf-packed into the unit
//    square with a margin of 1/512 of the atlas side around every chart (4 texels apart at 1024 px).
//
// Deterministic: hash maps are only used for lookups, every iteration follows the triangle order.

use std::collections::{HashMap, VecDeque};

use glam::{DVec2, DVec3, Vec2, Vec3};

/// Margin around every chart as a fraction of the atlas side (charts are at least 4 texels apart at 1024 px).
const MARGIN: f64 = 1.0 / 512.0;
/// A triangle may join a chart when its normal is within 60 degrees of the chart's axis.
const JOIN_COS: f64 = 0.5;
/// Unit vectors of the six directions of `direction_of`.
const AXES: [DVec3; 6] = [DVec3::X, DVec3::NEG_X, DVec3::Y, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z];
/// A triangle whose bounding box spans more grid cells than this is kept in a list tested against every candidate.
const MAX_GRID_CELLS: i64 = 1024;

/// One generated chart: its triangles (indices into the input) and their projected corners.
struct Chart {
    tris: Vec<u32>,
    min: DVec2,
    max: DVec2,
}

/// The axis direction (0..6: +X, -X, +Y, -Y, +Z, -Z) a face normal is closest to.
fn direction_of(normal: DVec3) -> u8 {
    let a = normal.abs();
    let (axis, value) = if a.x >= a.y && a.x >= a.z {
        (0, normal.x)
    } else if a.y >= a.z {
        (1, normal.y)
    } else {
        (2, normal.z)
    };
    axis * 2 + u8::from(value < 0.0)
}

/// Projection of a point onto the plane of `direction`, in v-down texture orientation: on the side faces world +Y
/// is up in the texture, on the top / bottom faces world -Z / +Z is up; triangles facing `direction` come out with
/// a negative signed area in v-down UV space (the unmirrored orientation of `mesh_build`).
fn project(direction: u8, p: DVec3) -> DVec2 {
    match direction {
        0 => DVec2::new(-p.z, -p.y),
        1 => DVec2::new(p.z, -p.y),
        2 => DVec2::new(p.x, p.z),
        3 => DVec2::new(p.x, -p.z),
        4 => DVec2::new(p.x, -p.y),
        _ => DVec2::new(-p.x, -p.y),
    }
}

/// True when the interiors of two 2D triangles overlap by more than `eps` along every separating-axis candidate
/// (edges or corners shared with a neighbour only touch: no overlap).
fn overlaps(a: &[DVec2; 3], b: &[DVec2; 3], eps: f64) -> bool {
    for tri in [a, b] {
        for k in 0..3 {
            let edge = tri[(k + 1) % 3] - tri[k];
            let axis = edge.perp();
            let len = axis.length();
            if len <= 0.0 {
                continue;
            }
            let axis = axis / len;
            let range = |t: &[DVec2; 3]| {
                let d = t.map(|p| p.dot(axis));
                (d[0].min(d[1]).min(d[2]), d[0].max(d[1]).max(d[2]))
            };
            let (a0, a1) = range(a);
            let (b0, b1) = range(b);
            if a1.min(b1) - a0.max(b0) <= eps {
                return false;
            }
        }
    }
    true
}

/// Position bits for edge matching (`-0.0` and `+0.0` are the same position).
fn bits(p: Vec3) -> [u32; 3] {
    [(p.x + 0.0).to_bits(), (p.y + 0.0).to_bits(), (p.z + 0.0).to_bits()]
}

/// The spatial grid of the chart being grown.
struct Grid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<u32>>,
    /// Triangles too large for the grid: tested against every candidate.
    large: Vec<u32>,
}

impl Grid {
    fn range(&self, t: &[DVec2; 3]) -> (i64, i64, i64, i64) {
        let lo = t[0].min(t[1]).min(t[2]) / self.cell;
        let hi = t[0].max(t[1]).max(t[2]) / self.cell;
        (lo.x.floor() as i64, lo.y.floor() as i64, hi.x.floor() as i64, hi.y.floor() as i64)
    }

    fn insert(&mut self, tri: u32, t: &[DVec2; 3]) {
        let (x0, y0, x1, y1) = self.range(t);
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > MAX_GRID_CELLS {
            self.large.push(tri);
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells.entry((x, y)).or_default().push(tri);
            }
        }
    }

    /// True when `t` overlaps a triangle of the grid (`projected` holds the projections of the chart's triangles).
    fn collides(&self, t: &[DVec2; 3], projected: &[[DVec2; 3]], eps: f64) -> bool {
        if self.large.iter().any(|&o| overlaps(t, &projected[o as usize], eps)) {
            return true;
        }
        let (x0, y0, x1, y1) = self.range(t);
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > MAX_GRID_CELLS {
            // A huge candidate: test the chart's triangles directly instead of walking its cells.
            return self.cells.values().flatten().any(|&o| overlaps(t, &projected[o as usize], eps));
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(list) = self.cells.get(&(x, y))
                    && list.iter().any(|&o| overlaps(t, &projected[o as usize], eps))
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Generates a non-overlapping, unmirrored texture layout inside `(0, 1)^2` for the given triangles (all finite
/// and of non-zero area). One entry per input triangle, in v-down UV space.
pub(crate) fn generate(tris: &[[Vec3; 3]]) -> Vec<[Vec2; 3]> {
    let n = tris.len();
    if n == 0 {
        return Vec::new();
    }
    let pos: Vec<[DVec3; 3]> = tris.iter().map(|t| t.map(|p| p.as_dvec3())).collect();
    let normal: Vec<DVec3> = pos.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()).collect();
    let direction: Vec<u8> = normal.iter().map(|&n| direction_of(n)).collect();
    // Projection of every triangle onto the plane of every direction it may join (its own and, near a box edge,
    // a neighbouring one): computed on demand.
    let project_tri = |t: usize, d: u8| -> [DVec2; 3] { pos[t].map(|p| project(d, p)) };
    let mut projected: Vec<[DVec2; 3]> = (0..n).map(|t| project_tri(t, direction[t])).collect();
    let mean_edge = {
        let total: f64 = pos.iter().map(|t| (t[1] - t[0]).length() + (t[2] - t[1]).length() + (t[0] - t[2]).length()).sum();
        (total / (3 * n) as f64).max(1e-30)
    };
    let eps = mean_edge * 1e-6;

    // Edge adjacency by position.
    let mut edge_tris: HashMap<([u32; 3], [u32; 3]), Vec<u32>> = HashMap::with_capacity(n * 2);
    for (t, tri) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (bits(tri[k]), bits(tri[(k + 1) % 3]));
            let key = if a <= b { (a, b) } else { (b, a) };
            edge_tris.entry(key).or_default().push(t as u32);
        }
    }
    let neighbours = |t: usize, out: &mut Vec<u32>| {
        out.clear();
        let tri = &tris[t];
        for k in 0..3 {
            let (a, b) = (bits(tri[k]), bits(tri[(k + 1) % 3]));
            let key = if a <= b { (a, b) } else { (b, a) };
            if let Some(list) = edge_tris.get(&key) {
                out.extend(list.iter().copied().filter(|&o| o as usize != t));
            }
        }
    };

    // Chart growing.
    const UNASSIGNED: u32 = u32::MAX;
    let mut chart_of = vec![UNASSIGNED; n];
    let mut rejected_by = vec![UNASSIGNED; n];
    let mut charts: Vec<Chart> = Vec::new();
    let mut queue: VecDeque<u32> = VecDeque::new();
    let mut around: Vec<u32> = Vec::new();
    for seed in 0..n {
        if chart_of[seed] != UNASSIGNED {
            continue;
        }
        let id = charts.len() as u32;
        let label = direction[seed];
        let axis = AXES[label as usize];
        let mut grid = Grid { cell: mean_edge * 2.0, cells: HashMap::new(), large: Vec::new() };
        let mut chart = Chart { tris: Vec::new(), min: DVec2::INFINITY, max: DVec2::NEG_INFINITY };
        let mut accept = |t: u32, tri: [DVec2; 3], chart_of: &mut [u32], projected: &mut [[DVec2; 3]], grid: &mut Grid| {
            chart_of[t as usize] = id;
            projected[t as usize] = tri;
            grid.insert(t, &tri);
            chart.tris.push(t);
            for p in tri {
                chart.min = chart.min.min(p);
                chart.max = chart.max.max(p);
            }
        };
        accept(seed as u32, projected[seed], &mut chart_of, &mut projected, &mut grid);
        queue.clear();
        queue.push_back(seed as u32);
        while let Some(t) = queue.pop_front() {
            neighbours(t as usize, &mut around);
            for &o in &around {
                let oi = o as usize;
                // A triangle rejected for this chart (overlap) is not tried again for it. Triangles facing up to
                // 60 degrees away from the chart's axis may join it: fewer, larger charts along the box edges.
                if chart_of[oi] != UNASSIGNED || rejected_by[oi] == id || normal[oi].dot(axis) < JOIN_COS {
                    continue;
                }
                let tri = project_tri(oi, label);
                if grid.collides(&tri, &projected, eps) {
                    rejected_by[oi] = id;
                    continue;
                }
                accept(o, tri, &mut chart_of, &mut projected, &mut grid);
                queue.push_back(o);
            }
        }
        charts.push(chart);
    }

    // Packing: shelves of charts sorted by height (then width, then creation order), atlas side grown until the
    // shelves fit into a square.
    let size = |c: &Chart| (c.max - c.min).max(DVec2::splat(0.0));
    let mut order: Vec<usize> = (0..charts.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (size(&charts[a]), size(&charts[b]));
        sb.y.total_cmp(&sa.y).then(sb.x.total_cmp(&sa.x)).then(a.cmp(&b))
    });
    let area: f64 = charts.iter().map(|c| size(c).x * size(c).y).sum();
    let largest = charts.iter().map(|c| size(c).max_element()).fold(0.0f64, f64::max).max(mean_edge * 1e-3);
    let mut side = (area / 0.8).sqrt().max(largest);
    let mut offsets = vec![DVec2::ZERO; charts.len()];
    for _ in 0..400 {
        let margin = side * MARGIN;
        let (mut x, mut y, mut shelf) = (0.0f64, 0.0f64, 0.0f64);
        let mut fits = true;
        for &c in &order {
            let s = size(&charts[c]) + DVec2::splat(2.0 * margin);
            if x > 0.0 && x + s.x > side {
                y += shelf;
                x = 0.0;
                shelf = 0.0;
            }
            if x + s.x > side || y + s.y > side {
                fits = false;
                break;
            }
            offsets[c] = DVec2::new(x + margin, y + margin);
            x += s.x;
            shelf = shelf.max(s.y);
        }
        if fits {
            break;
        }
        side *= 1.03;
    }

    let mut out = vec![[Vec2::ZERO; 3]; n];
    for (c, chart) in charts.iter().enumerate() {
        for &t in &chart.tris {
            let uv = projected[t as usize].map(|p| (p - chart.min + offsets[c]) / side);
            // Clamp against rounding at the far border; the margin keeps every corner inside (0, 1).
            out[t as usize] = uv.map(|p| p.clamp(DVec2::ZERO, DVec2::ONE).as_vec2());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples;

    fn uv_area2(t: &[Vec2; 3]) -> f64 {
        let d = t.map(|c| c.as_dvec2());
        (d[1] - d[0]).perp_dot(d[2] - d[0])
    }

    /// Every corner inside the unit square, every triangle unmirrored with a positive area, no two triangles
    /// overlapping (brute force).
    fn assert_valid_layout(tris: &[[Vec3; 3]], uvs: &[[Vec2; 3]]) {
        assert_eq!(tris.len(), uvs.len());
        for uv in uvs {
            assert!(uv.iter().all(|c| c.x > 0.0 && c.x < 1.0 && c.y > 0.0 && c.y < 1.0), "{uv:?}");
            assert!(uv_area2(uv) < 0.0, "mirrored or empty: {uv:?}");
        }
        let d: Vec<[DVec2; 3]> = uvs.iter().map(|t| t.map(|c| c.as_dvec2())).collect();
        for i in 0..d.len() {
            for j in i + 1..d.len() {
                assert!(!overlaps(&d[i], &d[j], 1e-7), "triangles {i} and {j} overlap: {:?} {:?}", d[i], d[j]);
            }
        }
    }

    fn soup(raw: &crate::mesh::RawMesh) -> Vec<[Vec3; 3]> {
        raw.triangles.iter().map(|t| t.positions).collect()
    }

    #[test]
    fn a_cube_becomes_six_square_charts_of_equal_size() {
        let tris = soup(&samples::seam_cube(2.0));
        let uvs = generate(&tris);
        assert_valid_layout(&tris, &uvs);
        // Texel density: every triangle keeps its 3D area ratio (all faces are equal: equal UV areas).
        let areas: Vec<f64> = uvs.iter().map(|t| uv_area2(t).abs()).collect();
        let (lo, hi) = areas.iter().fold((f64::MAX, 0.0f64), |(lo, hi), &a| (lo.min(a), hi.max(a)));
        assert!(hi / lo < 1.0001, "{areas:?}");
        // The two triangles of each face share their diagonal in UV space too (one chart per face).
        let shared = |a: &[Vec2; 3], b: &[Vec2; 3]| a.iter().filter(|c| b.contains(c)).count();
        for face in 0..6 {
            assert_eq!(shared(&uvs[2 * face], &uvs[2 * face + 1]), 2, "face {face}");
        }
        // Reasonable use of the square: six squares fill more than a third of it.
        let used: f64 = areas.iter().sum::<f64>() / 2.0;
        assert!(used > 0.33, "coverage {used}");
    }

    #[test]
    fn a_sphere_keeps_connected_charts_and_no_overlap() {
        let tris = soup(&samples::uv_sphere(24, 12, 1.0));
        let uvs = generate(&tris);
        assert_valid_layout(&tris, &uvs);
        // Box projection of a sphere: a handful of large charts, not one chart per triangle.
        let mut corners: Vec<[u32; 2]> = uvs.iter().flatten().map(|c| [c.x.to_bits(), c.y.to_bits()]).collect();
        corners.sort_unstable();
        corners.dedup();
        assert!(corners.len() < tris.len(), "{} distinct UV corners for {} triangles", corners.len(), tris.len());
        // Deterministic.
        assert_eq!(generate(&tris), uvs);
    }

    #[test]
    fn a_folded_surface_is_split_instead_of_overlapping() {
        // A spiral ramp of one and a half turns: every triangle faces +Y and is connected to the next, but the
        // second turn lies above the first one, so a single projected chart would overlap itself.
        let steps = 48;
        let point = |i: usize, r: f32| {
            let a = i as f32 * 3.0 * std::f32::consts::PI / steps as f32;
            Vec3::new(r * a.cos(), 0.05 * a, r * a.sin())
        };
        let mut tris = Vec::new();
        for i in 0..steps {
            for t in [[point(i, 0.5), point(i, 1.0), point(i + 1, 1.0)], [point(i, 0.5), point(i + 1, 1.0), point(i + 1, 0.5)]] {
                let n = (t[1] - t[0]).cross(t[2] - t[0]);
                tris.push(if n.y > 0.0 { t } else { [t[0], t[2], t[1]] });
            }
        }
        let normals: Vec<DVec3> = tris.iter().map(|t| (t[1] - t[0]).cross(t[2] - t[0]).as_dvec3()).collect();
        assert!(normals.iter().all(|&n| direction_of(n) == 2), "all triangles face +Y");
        let uvs = generate(&tris);
        assert_valid_layout(&tris, &uvs);
    }

    #[test]
    fn scattered_triangles_and_tiny_inputs() {
        let one = [[Vec3::ZERO, Vec3::X, Vec3::Y]];
        assert_valid_layout(&one, &generate(&one));
        assert!(generate(&[]).is_empty());
        // Wildly different sizes: still inside the square and without overlap.
        let tris: Vec<[Vec3; 3]> = (0..40)
            .map(|i| {
                let s = if i % 7 == 0 { 50.0 } else { 0.01 * (i as f32 + 1.0) };
                let o = Vec3::new(i as f32 * 100.0, (i % 3) as f32, (i % 5) as f32 * 3.0);
                let n = i % 6;
                match n {
                    0 => [o, o + Vec3::X * s, o + Vec3::Y * s],
                    1 => [o, o + Vec3::Y * s, o + Vec3::X * s],
                    2 => [o, o + Vec3::Z * s, o + Vec3::X * s],
                    3 => [o, o + Vec3::X * s, o + Vec3::Z * s],
                    4 => [o, o + Vec3::Y * s, o + Vec3::Z * s],
                    _ => [o, o + Vec3::Z * s, o + Vec3::Y * s],
                }
            })
            .collect();
        assert_valid_layout(&tris, &generate(&tris));
    }

    #[test]
    fn side_faces_keep_world_up_as_texture_up() {
        // A wall facing +Z: the higher corner must get the smaller v (v-down).
        let tris = [[Vec3::ZERO, Vec3::X, Vec3::new(0.0, 1.0, 0.0)]];
        let uv = generate(&tris)[0];
        assert!(uv[2].y < uv[0].y, "{uv:?}");
        assert!(uv[1].x > uv[0].x, "{uv:?}");
    }
}
