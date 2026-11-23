// Owner: round 9, package B (R9-4, plan T9). Which triangles of a mesh use the same texture area as other triangles
// (mirrored halves, instanced pieces): per texture set a 1,024^2 count raster of the triangles' UV footprints; a
// triangle "shares texels" when most of its footprint is counted more than once. Paint on such a triangle shows on every
// copy, in any mode: the model's property, not the wrap's. The panel names it for a selected 3D stamp
// (`Document::stamp_shared_texels`). Built once per mesh, lazily, off the UI thread (the warm-up job,
// `WrapMeshCell::uv_share`).

use std::time::Instant;

use glam::Vec2;

use crate::ids::TriId;
use crate::mesh::MeshData;

/// Side of the count raster per texture set (texels); a set with many small footprints gets a finer raster (up to
/// `UV_SHARE_MAX_RES`) so that a footprint covers about `UV_SHARE_PX` pixel centres on average.
pub const UV_SHARE_RES: u32 = 1024;
pub const UV_SHARE_MAX_RES: u32 = 4096;
pub const UV_SHARE_PX: f64 = 8.0;
/// Most pixel-centre tests per texture set: a set whose footprints would take more (large or tiled UVs) is counted on
/// a coarser raster (half the side per step, at least 128).
pub const UV_SHARE_WORK: f64 = 64e6;
/// Least share of a selected stamp's area on triangles that share texels for which the panel warns.
pub const UV_SHARE_WARN: f32 = 0.05;
/// Stable (w1, item 5): a footprint whose UV box spans more than one texture (a tiled trim strip, u 0..12) is
/// rasterized wrapped into the unit square up to this many textures of box area; beyond, it counts everywhere when it
/// holds a whole texture (an inscribed circle of diameter sqrt 2), else only itself is marked.
pub const UV_SHARE_TILED_MAX: f64 = 64.0;

/// The shared-texel triangles of one mesh.
#[derive(Debug)]
pub struct UvShare {
    /// Per triangle: its UV footprint is mostly used by other triangles too.
    shares: Vec<bool>,
    /// Share of the mesh's UV-mapped surface (3D area) whose triangles share texels.
    mesh_share: f32,
    /// Build time (milliseconds).
    pub build_ms: f64,
}

/// The pixel centres (wrapped into the raster) the UV triangle `uv` covers, edges included; `f(index)` per centre (a
/// footprint wider than the raster visits a texel once per copy). Returns the number of centres visited; 0 for a
/// footprint whose box exceeds `max_box` rasters.
fn raster(uv: [Vec2; 3], res: u32, max_box: f64, f: &mut impl FnMut(usize)) -> usize {
    let r = res as f64;
    let p = uv.map(|q| [q.x as f64 * r, q.y as f64 * r]);
    let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
    if !(area.abs() > 0.0 && area.is_finite()) {
        return 0;
    }
    let s = area.signum();
    let (x0, x1) = (p.iter().map(|q| q[0]).fold(f64::INFINITY, f64::min), p.iter().map(|q| q[0]).fold(f64::NEG_INFINITY, f64::max));
    let (y0, y1) = (p.iter().map(|q| q[1]).fold(f64::INFINITY, f64::min), p.iter().map(|q| q[1]).fold(f64::NEG_INFINITY, f64::max));
    // A footprint over more than `max_box` whole rasters is not rasterized (the caller decides).
    if (x1 - x0) * (y1 - y0) > max_box * r * r || !(x1 - x0).is_finite() || !(y1 - y0).is_finite() {
        return 0;
    }
    let edge = |a: [f64; 2], b: [f64; 2], x: f64, y: f64| s * ((b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]));
    let mut n = 0;
    let (i0, i1) = ((x0 - 0.5).ceil() as i64, (x1 - 0.5).floor() as i64);
    let (j0, j1) = ((y0 - 0.5).ceil() as i64, (y1 - 0.5).floor() as i64);
    for j in j0..=j1 {
        let y = j as f64 + 0.5;
        for i in i0..=i1 {
            let x = i as f64 + 0.5;
            if edge(p[0], p[1], x, y) >= 0.0 && edge(p[1], p[2], x, y) >= 0.0 && edge(p[2], p[0], x, y) >= 0.0 {
                let (wi, wj) = (i.rem_euclid(res as i64) as usize, j.rem_euclid(res as i64) as usize);
                f(wj * res as usize + wi);
                n += 1;
            }
        }
    }
    n
}

/// Stable (w1, item 5): whether the UV triangle `uv` holds a whole texture (a unit square: its inscribed circle has a
/// diameter of at least sqrt 2), so that wrapped it covers every texel.
fn holds_a_texture(uv: [Vec2; 3]) -> bool {
    let p = uv.map(|q| glam::DVec2::new(q.x as f64, q.y as f64));
    let area = 0.5 * (p[1] - p[0]).perp_dot(p[2] - p[0]).abs();
    let perimeter = p[0].distance(p[1]) + p[1].distance(p[2]) + p[2].distance(p[0]);
    perimeter > 0.0 && 4.0 * area / perimeter >= std::f64::consts::SQRT_2
}

/// The pixel (wrapped) of the centroid of a UV triangle.
fn centroid_pixel(uv: [Vec2; 3], res: u32) -> Option<usize> {
    let c = (uv[0] + uv[1] + uv[2]) / 3.0;
    if !c.is_finite() {
        return None;
    }
    let (i, j) = ((c.x as f64 * res as f64).floor() as i64, (c.y as f64 * res as f64).floor() as i64);
    Some(j.rem_euclid(res as i64) as usize * res as usize + i.rem_euclid(res as i64) as usize)
}

impl UvShare {
    /// Builds the shares of `mesh` on the calling thread (serial).
    pub fn build(mesh: &MeshData) -> UvShare {
        UvShare::build_at(mesh, UV_SHARE_RES)
    }

    /// `build` with a raster of `res`^2 per set.
    pub fn build_at(mesh: &MeshData, res: u32) -> UvShare {
        let started = Instant::now();
        let ntri = mesh.tri_count() as usize;
        let mut shares = vec![false; ntri];
        let mut by_set: Vec<Vec<TriId>> = vec![Vec::new(); mesh.sets.len()];
        for t in 0..ntri as TriId {
            if let Some(s) = mesh.tri_set(t)
                && let Some(list) = by_set.get_mut(s.index())
            {
                list.push(t);
            }
        }
        // The UV bounding-box area of a triangle, in texture units.
        let box_area = |t: TriId| {
            let uv = mesh.tri_uvs(t);
            let (lo, hi) = (uv[0].min(uv[1]).min(uv[2]), uv[0].max(uv[1]).max(uv[2]));
            let a = ((hi.x - lo.x) * (hi.y - lo.y)) as f64;
            if a.is_finite() { a } else { 0.0 }
        };
        let mut count: Vec<u8> = Vec::new();
        for tris in by_set.iter().filter(|l| !l.is_empty()) {
            // The raster of this set: `res`, finer where the footprints are small (`UV_SHARE_PX` centres each on
            // average), coarser where they would take more than `UV_SHARE_WORK` tests (a footprint of a whole texture or
            // more is not rasterized: `huge`).
            let uv_area: f64 = tris.iter().map(|&t| {
                let uv = mesh.tri_uvs(t);
                (0.5 * (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]).abs() as f64).min(1.0)
            }).filter(|a| a.is_finite()).sum();
            let mut r = res;
            while r < UV_SHARE_MAX_RES && uv_area * (r as f64) * (r as f64) < UV_SHARE_PX * tris.len() as f64 {
                r *= 2;
            }
            let work = |r: u32| tris.iter().map(|&t| box_area(t).min(UV_SHARE_TILED_MAX)).sum::<f64>() * (r as f64) * (r as f64);
            while r > 128 && work(r) > UV_SHARE_WORK {
                r /= 2;
            }
            // Stable (w1, item 5): a footprint whose box spans more than one texture (`tiled`) is rasterized wrapped,
            // each texel once (it may cover a texel several times by itself: then it shares, with itself); round 9
            // counted every such footprint everywhere, so one trim strip (u 0..12, v 0.88..0.98) marked every other
            // triangle of its set as sharing. Beyond `UV_SHARE_TILED_MAX` textures: everywhere when it holds a whole
            // texture (`huge`), else not counted (only itself marked).
            #[derive(Clone, Copy, PartialEq)]
            enum Kind {
                Plain,
                Tiled,
                Huge,
                Unknown,
            }
            let kind: Vec<Kind> = tris
                .iter()
                .map(|&t| {
                    let a = box_area(t);
                    if a <= 1.0 {
                        Kind::Plain
                    } else if a <= UV_SHARE_TILED_MAX {
                        Kind::Tiled
                    } else if holds_a_texture(mesh.tri_uvs(t)) {
                        Kind::Huge
                    } else {
                        Kind::Unknown
                    }
                })
                .collect();
            let everywhere = kind.iter().filter(|&&k| k == Kind::Huge).count().min(255) as u8;
            count.clear();
            count.resize(r as usize * r as usize, 0);
            let mut seen: Vec<bool> = if kind.contains(&Kind::Tiled) { vec![false; r as usize * r as usize] } else { Vec::new() };
            let mut texels: Vec<usize> = Vec::new();
            // The distinct texels of a tiled footprint (in `texels`) and whether it covers one twice by itself.
            let distinct = |uv: [Vec2; 3], seen: &mut Vec<bool>, texels: &mut Vec<usize>| -> bool {
                texels.clear();
                let n = raster(uv, r, UV_SHARE_TILED_MAX, &mut |i| {
                    if !seen[i] {
                        seen[i] = true;
                        texels.push(i);
                    }
                });
                for &i in texels.iter() {
                    seen[i] = false;
                }
                n > texels.len()
            };
            let mut self_twice = vec![false; tris.len()];
            // Pass 1: every footprint counted, a tiled one once per texel (a huge one counts everywhere; one that covers
            // no pixel centre is not counted: it would make its neighbour's pixel look shared).
            for (k, (&t, &kd)) in tris.iter().zip(&kind).enumerate() {
                match kd {
                    Kind::Plain => {
                        raster(mesh.tri_uvs(t), r, 1.0, &mut |i| count[i] = count[i].saturating_add(1));
                    }
                    Kind::Tiled => {
                        self_twice[k] = distinct(mesh.tri_uvs(t), &mut seen, &mut texels);
                        for &i in &texels {
                            count[i] = count[i].saturating_add(1);
                        }
                    }
                    Kind::Huge | Kind::Unknown => {}
                }
            }
            // Pass 2: a triangle shares texels when more than half of its footprint (a tiled one: its distinct texels)
            // is counted twice or more, or (tiled) it covers texels twice by itself; one without a pixel centre when two
            // other footprints cover its centroid; a huge or unknown one always.
            for (k, (&t, &kd)) in tris.iter().zip(&kind).enumerate() {
                let uv = mesh.tri_uvs(t);
                shares[t as usize] = match kd {
                    Kind::Huge | Kind::Unknown => true,
                    Kind::Tiled => {
                        distinct(uv, &mut seen, &mut texels);
                        let twice = texels.iter().filter(|&&i| count[i].saturating_add(everywhere) >= 2).count();
                        self_twice[k] || 2 * twice > texels.len()
                    }
                    Kind::Plain => {
                        let mut twice = 0usize;
                        let n = raster(uv, r, 1.0, &mut |i| twice += usize::from(count[i].saturating_add(everywhere) >= 2));
                        if n == 0 { centroid_pixel(uv, r).is_some_and(|i| count[i].saturating_add(everywhere) >= 2) } else { 2 * twice > n }
                    }
                };
            }
        }
        let (mut all, mut shared) = (0.0f64, 0.0f64);
        for &t in by_set.iter().flatten() {
            let [a, b, c] = mesh.tri_positions(t);
            let area = 0.5 * (b - a).cross(c - a).length() as f64;
            if area.is_finite() {
                all += area;
                if shares[t as usize] {
                    shared += area;
                }
            }
        }
        let mesh_share = if all > 0.0 { (shared / all) as f32 } else { 0.0 };
        UvShare { shares, mesh_share, build_ms: started.elapsed().as_secs_f64() * 1e3 }
    }

    /// True when triangle `t` uses texels that other triangles use too.
    pub fn shares(&self, t: TriId) -> bool {
        self.shares.get(t as usize).copied().unwrap_or(false)
    }

    /// Share of the mesh's UV-mapped surface (3D area) on triangles that share texels.
    pub fn mesh_share(&self) -> f32 {
        self.mesh_share
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::mesh::{RawMesh, RawTriangle};
    use crate::samples;
    use crate::testutil::mesh_of;

    fn quad(out: &mut Vec<RawTriangle>, x: [f32; 2], u: [f32; 2], v: [f32; 2]) {
        let p = [Vec3::new(x[0], 0.0, 0.0), Vec3::new(x[1], 0.0, 0.0), Vec3::new(x[1], 1.0, 0.0), Vec3::new(x[0], 1.0, 0.0)];
        let q = [Vec2::new(u[0], v[1]), Vec2::new(u[1], v[1]), Vec2::new(u[1], v[0]), Vec2::new(u[0], v[0])];
        out.push(RawTriangle { positions: [p[0], p[1], p[2]], normals: None, uvs: Some([q[0], q[1], q[2]]), material: 0 });
        out.push(RawTriangle { positions: [p[0], p[2], p[3]], normals: None, uvs: Some([q[0], q[2], q[3]]), material: 0 });
    }

    /// WA39 (plan section 6): a mirrored-UV sample shares on half its surface; the sphere sample and the cube on the
    /// floor share nothing.
    #[test]
    fn wa39_shared_texels_are_found_where_the_model_reuses_its_texture() {
        // Four unit quads: the first two mirror one island (`samples::mirrored_quads`' island), the other two have their
        // own islands.
        let mut raw: RawMesh = samples::mirrored_quads();
        let mut extra = Vec::new();
        quad(&mut extra, [1.0, 2.0], [0.02, 0.2], [0.1, 0.9]);
        quad(&mut extra, [2.0, 3.0], [0.8, 0.98], [0.1, 0.9]);
        raw.triangles.extend(extra);
        let share = UvShare::build(&mesh_of(raw)).mesh_share();
        println!("WA39 mirrored halves + two own islands: share {share:.4}");
        assert!((share - 0.5).abs() <= 0.02, "mirrored sample: share {share}");
        let mirrored = UvShare::build(&mesh_of(samples::mirrored_quads()));
        assert!((0..4).all(|t| mirrored.shares(t)) && (mirrored.mesh_share() - 1.0).abs() < 1e-6, "{mirrored:?}");
        for (what, raw) in [("sphere sample", samples::uv_sphere(48, 24, 1.0)), ("uv_cube_on_floor", samples::uv_cube_on_floor())] {
            let s = UvShare::build(&mesh_of(raw));
            println!("WA39 {what}: share {:.4} ({:.1} ms)", s.mesh_share(), s.build_ms);
            assert!(s.mesh_share() == 0.0, "{what}: share {}", s.mesh_share());
        }
    }

    /// Stable (w1, item 5): a plate on its own island (u, v 0..0.5) and a tiled trim strip (u 0..12, v 0.88..0.98):
    /// the strip reuses its texels (by itself), the plate does not. Round 9: the strip's box (over one texture) counted
    /// it everywhere and the plate was marked too (the code skeptic's trim_strip: 100 % shared). A footprint over a
    /// whole texture still marks every triangle of its set; a huge sliver marks only itself.
    #[test]
    fn w1_a_tiled_trim_strip_does_not_mark_a_unique_island() {
        let mut t = Vec::new();
        quad(&mut t, [0.0, 1.0], [0.0, 0.5], [0.0, 0.5]);
        quad(&mut t, [2.0, 14.0], [0.0, 12.0], [0.88, 0.98]);
        let s = UvShare::build(&mesh_of(RawMesh { triangles: t.clone(), materials: samples::mirrored_quads().materials }));
        println!("W1 plate + trim strip: plate {} {}, strip {} {}, share {:.4} ({:.1} ms)", s.shares(0), s.shares(1), s.shares(2), s.shares(3), s.mesh_share(), s.build_ms);
        assert!(!s.shares(0) && !s.shares(1), "the plate's own island is marked shared: {s:?}");
        assert!(s.shares(2) && s.shares(3), "the tiled strip reuses its texels: {s:?}");
        assert!((s.mesh_share() - 12.0 / 13.0).abs() < 1e-3, "share {}", s.mesh_share());
        // A quad over 3 x 3 textures (holds whole textures) marks the plate; a 10,000-wide sliver marks only itself.
        let mut big = t[..2].to_vec();
        quad(&mut big, [2.0, 3.0], [0.0, 3.0], [0.0, 3.0]);
        let s = UvShare::build(&mesh_of(RawMesh { triangles: big, materials: samples::mirrored_quads().materials }));
        assert!(s.shares(0) && s.shares(1) && s.shares(2), "a footprint over whole textures shares with everything: {s:?}");
        let mut sliver = t[..2].to_vec();
        quad(&mut sliver, [2.0, 3.0], [0.0, 10000.0], [0.9, 0.91]);
        let s = UvShare::build(&mesh_of(RawMesh { triangles: sliver, materials: samples::mirrored_quads().materials }));
        assert!(!s.shares(0) && !s.shares(1) && s.shares(2), "a huge sliver marks only itself: {s:?}");
    }
}
