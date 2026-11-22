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

/// Side of the count raster per texture set (texels).
pub const UV_SHARE_RES: u32 = 1024;
/// Least share of a selected stamp's area on triangles that share texels for which the panel warns.
pub const UV_SHARE_WARN: f32 = 0.05;

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

/// The pixel centres (wrapped into the raster) the UV triangle `uv` covers, edges included; `f(index)` per centre.
/// Returns the number of centres visited.
fn raster(uv: [Vec2; 3], res: u32, f: &mut impl FnMut(usize)) -> usize {
    let r = res as f64;
    let p = uv.map(|q| [q.x as f64 * r, q.y as f64 * r]);
    let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
    if !(area.abs() > 0.0 && area.is_finite()) {
        return 0;
    }
    let s = area.signum();
    let (x0, x1) = (p.iter().map(|q| q[0]).fold(f64::INFINITY, f64::min), p.iter().map(|q| q[0]).fold(f64::NEG_INFINITY, f64::max));
    let (y0, y1) = (p.iter().map(|q| q[1]).fold(f64::INFINITY, f64::min), p.iter().map(|q| q[1]).fold(f64::NEG_INFINITY, f64::max));
    // A footprint over more than four whole rasters (broken UVs) is not counted.
    if (x1 - x0) * (y1 - y0) > 4.0 * r * r {
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
        let mut count = vec![0u8; res as usize * res as usize];
        for tris in by_set.iter().filter(|l| !l.is_empty()) {
            count.fill(0);
            // Pass 1: every footprint counted (a triangle that covers no pixel centre counts its centroid's pixel).
            for &t in tris {
                let uv = mesh.tri_uvs(t);
                let n = raster(uv, res, &mut |i| count[i] = count[i].saturating_add(1));
                if n == 0
                    && let Some(i) = centroid_pixel(uv, res)
                {
                    count[i] = count[i].saturating_add(1);
                }
            }
            // Pass 2: a triangle shares texels when more than half of its footprint is counted twice or more.
            for &t in tris {
                let uv = mesh.tri_uvs(t);
                let mut twice = 0usize;
                let n = raster(uv, res, &mut |i| twice += usize::from(count[i] >= 2));
                shares[t as usize] = if n == 0 { centroid_pixel(uv, res).is_some_and(|i| count[i] >= 2) } else { 2 * twice > n };
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
}
