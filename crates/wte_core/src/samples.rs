// Owner: core-geometry (C1). Procedural meshes and images of ARCHITECTURE.md 5.10: the built-in samples and the
// fixtures of the algorithm tests. Everything here is deterministic.
//
// Conventions shared by all meshes: world space is right-handed and Y-up, counter-clockwise triangles are front
// faces, UVs are v-down (section 4.1). Quads are split along the diagonal from their bottom-left to their top-right
// corner (as seen from the front).

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::fmt::Write as _;

use glam::{Vec2, Vec3};

use crate::mesh::{RawMaterial, RawMesh, RawObjects, RawTriangle};
use crate::pixel::PixelBuf;

fn material(name: &str, base_color: [f32; 4], texture: Option<u32>) -> RawMaterial {
    RawMaterial { name: name.to_owned(), base_color, double_sided: false, texture }
}

/// Appends a quad as two triangles. Corners are given counter-clockwise as seen from the front, starting at the
/// bottom-left one: (BL, BR, TR, TL) -> triangles (BL, BR, TR) and (BL, TR, TL). `normal == None` leaves the normals
/// to `normalize`, which derives them from the winding.
fn push_quad(out: &mut Vec<RawTriangle>, p: [Vec3; 4], uv: [Vec2; 4], normal: Option<Vec3>, material: u32) {
    for [a, b, c] in [[0usize, 1, 2], [0, 2, 3]] {
        out.push(RawTriangle {
            positions: [p[a], p[b], p[c]],
            normals: normal.map(|n| [n; 3]),
            uvs: Some([uv[a], uv[b], uv[c]]),
            material,
        });
    }
}

/// An axis-aligned quad in the plane `z = const` facing +Z, covering `x0..x1`, `y0..y1`, mapped to the UV rectangle
/// `u0..u1`, `v0..v1` (v0 = top). The flat fixtures carry no explicit normals: their shading normals always follow
/// the winding, also after `reverse_winding`.
fn push_xy_quad(out: &mut Vec<RawTriangle>, x: [f32; 2], y: [f32; 2], z: f32, u: [f32; 2], v: [f32; 2], material: u32) {
    push_quad(
        out,
        [Vec3::new(x[0], y[0], z), Vec3::new(x[1], y[0], z), Vec3::new(x[1], y[1], z), Vec3::new(x[0], y[1], z)],
        [Vec2::new(u[0], v[1]), Vec2::new(u[1], v[1]), Vec2::new(u[1], v[0]), Vec2::new(u[0], v[0])],
        None,
        material,
    );
}

/// UV sphere with equirect UVs and a duplicated seam column.
///
/// Centred at the origin, north pole at +Y. A surface point with unit direction `d` has `u = atan2(-d.x, -d.z) / TAU`
/// (wrapped into 0..1) and `v = acos(d.y) / PI`: u = 0.5 faces +Z, u grows towards +X there, and the seam (u = 0 and
/// u = 1, two vertex columns with identical positions) runs down the -Z side. The pole triangles use
/// `u = (i + 0.5) / segments` at the pole. Smooth normals. One material "Sphere" referencing texture 0.
/// `segments` is clamped to at least 3, `rings` to at least 2.
pub fn uv_sphere(segments: u32, rings: u32, radius: f32) -> RawMesh {
    let seg = segments.max(3);
    let rings = rings.max(2);
    // Unit direction of grid point (i, j); column `seg` is bit-identical to column 0.
    let dir = |i: u32, j: u32| -> Vec3 {
        if j == 0 {
            return Vec3::Y;
        }
        if j >= rings {
            return Vec3::NEG_Y;
        }
        let theta = PI * j as f32 / rings as f32;
        let phi = TAU * (i % seg) as f32 / seg as f32;
        let (st, ct) = theta.sin_cos();
        let (sp, cp) = phi.sin_cos();
        Vec3::new(-st * sp, ct, -st * cp)
    };
    let uv = |i: u32, j: u32| Vec2::new(i as f32 / seg as f32, j as f32 / rings as f32);
    let pole_uv = |i: u32, j: u32| Vec2::new((i as f32 + 0.5) / seg as f32, j as f32 / rings as f32);

    let mut triangles = Vec::with_capacity((seg * (2 * rings - 2)) as usize);
    let mut push = |corners: [(Vec3, Vec2); 3]| {
        triangles.push(RawTriangle {
            positions: corners.map(|(d, _)| d * radius),
            normals: Some(corners.map(|(d, _)| d)),
            uvs: Some(corners.map(|(_, t)| t)),
            material: 0,
        });
    };
    for j in 0..rings {
        for i in 0..seg {
            // Quad corners in the texture: A top-left, B top-right, C bottom-left, D bottom-right.
            let a = (dir(i, j), uv(i, j));
            let b = (dir(i + 1, j), uv(i + 1, j));
            let c = (dir(i, j + 1), uv(i, j + 1));
            let d = (dir(i + 1, j + 1), uv(i + 1, j + 1));
            if j == 0 {
                push([(Vec3::Y, pole_uv(i, 0)), c, d]);
            } else if j == rings - 1 {
                push([a, (Vec3::NEG_Y, pole_uv(i, rings)), b]);
            } else {
                push([a, c, d]);
                push([a, d, b]);
            }
        }
    }
    RawMesh { triangles, materials: vec![material("Sphere", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// UV rectangle `[u0, u1, v0, v1]` of island `face` (0..6) of `seam_cube`: cell `face % 3`, `face / 3` of the 3x2
/// atlas, inset by 1/32 on every side.
pub(crate) fn seam_cube_island(face: u32) -> [f32; 4] {
    const MARGIN: f32 = 1.0 / 32.0;
    let (col, row) = ((face % 3) as f32, (face / 3) as f32);
    [col / 3.0 + MARGIN, (col + 1.0) / 3.0 - MARGIN, row / 2.0 + MARGIN, (row + 1.0) / 2.0 - MARGIN]
}

/// Cube with 24 verts, six islands in a 3x2 atlas with 1/32 margins: every edge is a seam.
///
/// Centred at the origin with edge length `size`. Faces in atlas order (row-major, 3 columns): +X, -X, +Y, -Y, +Z,
/// -Z; every island shows its face upright and unmirrored as seen from outside (for the side faces "up" is +Y, for
/// +Y it is -Z, for -Y it is +Z). Explicit flat normals (hard edges). One material "Cube" referencing texture 0.
pub fn seam_cube(size: f32) -> RawMesh {
    let h = size * 0.5;
    // (normal, right, up) with right x up = normal.
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    let mut triangles = Vec::with_capacity(12);
    for (f, (n, r, u)) in faces.into_iter().enumerate() {
        let [u0, u1, v0, v1] = seam_cube_island(f as u32);
        let corner = |sx: f32, sy: f32| (n + r * sx + u * sy) * h;
        push_quad(
            &mut triangles,
            [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)],
            [Vec2::new(u0, v1), Vec2::new(u1, v1), Vec2::new(u1, v0), Vec2::new(u0, v0)],
            Some(n),
            0,
        );
    }
    RawMesh { triangles, materials: vec![material("Cube", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// `seam_cube` "Body" + `uv_sphere` "Head" on top: two materials, two textures.
///
/// Body: `seam_cube(1.0)` (material 0, texture 0). Head: `uv_sphere(24, 12, 0.35)` centred at (0, 0.8, 0), so it is
/// sunk 0.05 into the top face and meets it in a circle of radius ~0.18 (material 1, texture 1).
pub fn two_material_model() -> RawMesh {
    let mut model = seam_cube(1.0);
    model.materials = vec![
        material("Body", [0.75, 0.75, 0.78, 1.0], Some(0)),
        material("Head", [0.85, 0.8, 0.7, 1.0], Some(1)),
    ];
    let offset = Vec3::new(0.0, 0.8, 0.0);
    for mut t in uv_sphere(24, 12, 0.35).triangles {
        for p in &mut t.positions {
            *p += offset;
        }
        t.material = 1;
        model.triangles.push(t);
    }
    model
}

/// Two quads sharing one UV island, one mirrored.
///
/// Both lie in the plane z = 0 and face +Z, `y` in -0.5..0.5: quad A (triangles 0, 1) covers `x` in -1..0, quad B
/// (triangles 2, 3) covers `x` in 0..1 and is the mirror image of A in the plane x = 0. Both map onto the UV square
/// 0.25..0.75: A unmirrored, B mirrored (u decreases with x), so the UVs agree along the shared edge.
pub fn mirrored_quads() -> RawMesh {
    let mut triangles = Vec::with_capacity(4);
    push_xy_quad(&mut triangles, [-1.0, 0.0], [-0.5, 0.5], 0.0, [0.25, 0.75], [0.25, 0.75], 0);
    push_xy_quad(&mut triangles, [0.0, 1.0], [-0.5, 0.5], 0.0, [0.75, 0.25], [0.25, 0.75], 0);
    RawMesh { triangles, materials: vec![material("Mirrored", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// UVs in [0.5, 1.5]^2, plus one quad in tile [2, 3].
///
/// Quad A (triangles 0, 1): `x`, `y` in 0..1, z = 0, facing +Z, `u = 0.5 + x`, `v = 1.5 - y` — it straddles all four
/// tile borders. Quad B (triangles 2, 3): `x` in 1.5..2.5, same plane, UVs 2.25..2.75 in both directions.
pub fn wrapped_quad() -> RawMesh {
    let mut triangles = Vec::with_capacity(4);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [0.5, 1.5], [0.5, 1.5], 0);
    push_xy_quad(&mut triangles, [1.5, 2.5], [0.0, 1.0], 0.0, [2.25, 2.75], [2.25, 2.75], 0);
    RawMesh { triangles, materials: vec![material("Wrapped", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// Unit quad in the XY plane facing +Z with identity UVs (u = x, v = 1 - y).
///
/// `x`, `y` in 0..1 at z = 0. Like all flat fixtures it has no explicit normals (they are generated from the winding).
pub fn flat_quad() -> RawMesh {
    let mut triangles = Vec::with_capacity(2);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [0.0, 1.0], [0.0, 1.0], 0);
    RawMesh { triangles, materials: vec![material("Quad", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// Tetrahedron without UVs (and without normals): vertices (1,1,1), (1,-1,-1), (-1,1,-1), (-1,-1,1), outward winding.
pub fn no_uv_tetra() -> RawMesh {
    let v = [Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, -1.0), Vec3::new(-1.0, 1.0, -1.0), Vec3::new(-1.0, -1.0, 1.0)];
    let triangles = [[0usize, 1, 2], [0, 3, 1], [0, 2, 3], [1, 3, 2]]
        .into_iter()
        .map(|[a, b, c]| RawTriangle { positions: [v[a], v[b], v[c]], normals: None, uvs: None, material: 0 })
        .collect();
    RawMesh { triangles, materials: vec![material("Tetra", [0.7, 0.72, 0.8, 1.0], None)] }
}

/// Two coplanar unit quads whose UV islands are 2 texels apart at 256 px (u in [0.25, 0.496] and [0.504, 0.75]):
/// their gutters touch.
///
/// Both face +Z in the plane z = 0, `y` in 0..1: quad A (triangles 0, 1) covers `x` in 0..1, quad B (triangles 2, 3)
/// covers `x` in 1.5..2.5. Both islands span `v` in 0.25..0.75.
pub fn tight_islands() -> RawMesh {
    let mut triangles = Vec::with_capacity(4);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [0.25, 0.496], [0.25, 0.75], 0);
    push_xy_quad(&mut triangles, [1.5, 2.5], [0.0, 1.0], 0.0, [0.504, 0.75], [0.25, 0.75], 0);
    RawMesh { triangles, materials: vec![material("Islands", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// `flat_quad` plus a second, larger quad 0.5 behind it (z = -0.5), separate UV islands.
///
/// Front quad (triangles 0, 1): `x`, `y` in 0..1 at z = 0, UV island 0.05..0.45 in both directions. Rear quad
/// (triangles 2, 3): `x`, `y` in -0.5..1.5 at z = -0.5, UV island 0.55..0.95. Both face +Z.
pub fn occluded_quads() -> RawMesh {
    let mut triangles = Vec::with_capacity(4);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [0.05, 0.45], [0.05, 0.45], 0);
    push_xy_quad(&mut triangles, [-0.5, 1.5], [-0.5, 1.5], -0.5, [0.55, 0.95], [0.55, 0.95], 0);
    RawMesh { triangles, materials: vec![material("Quads", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// A body with a sleeve over its lower half (accept_fixes item F, like WaterBottle's grip): the unit square in the plane
/// z = 0 (facing +Z, UV 0.05..0.45) continues under a second quad, x 0..1, y 0..0.5, 0.05 in front of it (UV
/// 0.55..0.95). Rays through the lower half hit the sleeve first and the hidden body behind it.
pub fn sleeve_quads() -> RawMesh {
    let mut triangles = Vec::with_capacity(4);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [0.05, 0.45], [0.05, 0.45], 0);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 0.5], 0.05, [0.55, 0.95], [0.55, 0.95], 0);
    RawMesh { triangles, materials: vec![material("Sleeve", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// A fragmented atlas: the unit square in the plane z = 0 (facing +Z) cut into 4 x 4 quads, each mapped to its own
/// small UV island (1/16 of the atlas wide, oriented like `flat_quad`) and the islands scattered over the atlas, so
/// that neighbours on the surface lie far apart in UV (like an xatlas / Smart-UV packing).
pub fn scattered_islands() -> RawMesh {
    // Island slot (column, row) of every quad on an 8 x 8 grid of slots: a fixed shuffle.
    const SLOTS: [(u32, u32); 16] = [
        (0, 0), (7, 6), (3, 1), (5, 7),
        (1, 5), (6, 0), (2, 3), (4, 4),
        (7, 2), (0, 7), (5, 1), (2, 6),
        (4, 0), (1, 2), (6, 4), (3, 7),
    ];
    let mut triangles = Vec::with_capacity(32);
    for (i, &(sx, sy)) in SLOTS.iter().enumerate() {
        let (qx, qy) = ((i % 4) as f32 * 0.25, (i / 4) as f32 * 0.25);
        let (u0, v0) = (sx as f32 / 8.0 + 1.0 / 64.0, sy as f32 / 8.0 + 1.0 / 64.0);
        push_xy_quad(&mut triangles, [qx, qx + 0.25], [qy, qy + 0.25], 0.0, [u0, u0 + 1.0 / 16.0], [v0, v0 + 1.0 / 16.0], 0);
    }
    RawMesh { triangles, materials: vec![material("Scattered", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// Swaps vertices 1 and 2 of every triangle (positions, normals, uvs).
///
/// Explicit normals are only reordered, never negated: a mesh that carries them (cube, sphere) afterwards has shading
/// normals pointing against its winding, while a mesh without them (the flat fixtures) gets normals that follow the
/// new winding.
pub fn reverse_winding(raw: &mut RawMesh) {
    for t in &mut raw.triangles {
        t.positions.swap(1, 2);
        if let Some(n) = &mut t.normals {
            n.swap(1, 2);
        }
        if let Some(uv) = &mut t.uvs {
            uv.swap(1, 2);
        }
    }
}

/// Opaque two-tone checker with a coloured border.
///
/// `cells` x `cells` grey cells (the top-left one is light). The border is `max(1, min(w, h) / 64)` pixels wide and
/// tells the sides apart: top red, bottom blue, left green, right yellow (top and bottom win in the corners).
pub fn checker_texture(width: u32, height: u32, cells: u32) -> PixelBuf {
    const LIGHT: [u8; 4] = [212, 212, 212, 255];
    const DARK: [u8; 4] = [120, 120, 124, 255];
    const TOP: [u8; 4] = [228, 70, 58, 255];
    const BOTTOM: [u8; 4] = [58, 108, 228, 255];
    const LEFT: [u8; 4] = [70, 188, 88, 255];
    const RIGHT: [u8; 4] = [238, 198, 48, 255];
    let cells = cells.max(1) as u64;
    let border = (width.min(height) / 64).max(1);
    let mut img = PixelBuf::new(width, height);
    for y in 0..height {
        let cy = y as u64 * cells / height as u64;
        let row = img.row_mut(y);
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = x as u32;
            let color = if y < border {
                TOP
            } else if y >= height - border {
                BOTTOM
            } else if x < border {
                LEFT
            } else if x >= width - border {
                RIGHT
            } else if (x as u64 * cells / width as u64 + cy).is_multiple_of(2) {
                LIGHT
            } else {
                DARK
            };
            *px = color;
        }
    }
    img
}

/// Ring + arrow with soft alpha; asymmetric so flips are detectable.
///
/// On a transparent background: a soft white glow inside a blue ring, a red arrow in the middle pointing **up**
/// (towards row 0) and a yellow dot to the lower **right** of the centre. Straight alpha, anti-aliased edges.
pub fn sample_stamp_image(size: u32) -> PixelBuf {
    let mut img = PixelBuf::new(size, size);
    if size == 0 {
        return img;
    }
    let s = size as f32;
    // Coverage of a shape from its signed distance in pixels (negative inside): a two-pixel smooth edge.
    let coverage = |sd_px: f32| -> f32 {
        let t = ((1.0 - sd_px) * 0.5).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    // Signed distance to the half plane left of the directed line a -> b (v-down image coordinates).
    let half_plane = |p: Vec2, a: Vec2, b: Vec2| -> f32 {
        let d = (b - a).normalize_or_zero();
        (p - a).perp_dot(d)
    };
    for y in 0..size {
        for x in 0..size {
            let p = Vec2::new((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let r = (p - Vec2::splat(0.5)).length();
            // Layers bottom to top: (rgb, alpha).
            let glow_t = ((0.36 - r) / 0.16).clamp(0.0, 1.0);
            let glow = 0.35 * glow_t * glow_t * (3.0 - 2.0 * glow_t);
            let ring = coverage(((r - 0.36).abs() - 0.045) * s);
            let shaft = {
                let q = (p - Vec2::new(0.5, 0.55)).abs() - Vec2::new(0.035, 0.17);
                q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0)
            };
            let head = {
                let (apex, left, right) = (Vec2::new(0.5, 0.2), Vec2::new(0.38, 0.4), Vec2::new(0.62, 0.4));
                half_plane(p, apex, right).max(half_plane(p, right, left)).max(half_plane(p, left, apex))
            };
            let arrow = coverage(shaft.min(head) * s);
            let dot = coverage(((p - Vec2::new(0.7, 0.62)).length() - 0.05) * s);
            let layers: [([f32; 3], f32); 4] = [
                ([255.0, 255.0, 255.0], glow),
                ([40.0, 110.0, 220.0], ring),
                ([225.0, 60.0, 50.0], arrow),
                ([245.0, 200.0, 40.0], dot),
            ];
            let (mut rgb, mut alpha) = ([0.0f32; 3], 0.0f32);
            for (color, a) in layers {
                // Straight-alpha "over".
                let out = a + alpha * (1.0 - a);
                if out > 0.0 {
                    for c in 0..3 {
                        rgb[c] = (color[c] * a + rgb[c] * alpha * (1.0 - a)) / out;
                    }
                }
                alpha = out;
            }
            let a8 = (alpha * 255.0 + 0.5) as u8;
            if a8 > 0 {
                img.set(x, y, [(rgb[0] + 0.5) as u8, (rgb[1] + 0.5) as u8, (rgb[2] + 0.5) as u8, a8]);
            }
        }
    }
    img
}

/// Name of material `index` as written by `to_obj`: the material's name with whitespace replaced by `_`, or
/// `material_<index>` when it is empty or the index has no entry in `raw.materials`.
fn obj_material_name(raw: &RawMesh, index: u32) -> String {
    match raw.materials.get(index as usize) {
        Some(m) if !m.name.trim().is_empty() => {
            m.name.trim().chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect()
        }
        _ => format!("material_{index}"),
    }
}

/// OBJ + MTL text for a raw mesh (writes v = 1 - v; triangles without UVs are written as faces without `vt`).
/// `texture_names[i]` is the map_Kd for raw texture index i.
///
/// Triangles keep their order; a `usemtl` line is written whenever the material changes. Identical positions, UVs
/// and normals are written once and shared through the face indices. An empty `mtl_name` omits the `mtllib` line.
/// Materials are written to the MTL in index order as `newmtl`, `Kd`, `d` and (when the material has a texture with
/// a non-empty name in `texture_names`) `map_Kd`.
pub fn to_obj(raw: &RawMesh, mtl_name: &str, texture_names: &[String]) -> (String, String) {
    write_obj(raw, None, mtl_name, texture_names)
}

/// `to_obj` plus an `o <name>` line in front of a face whenever its object (`objects.tri_object`, 0 when missing)
/// changes, so that the OBJ importer reads the objects back (objects-tabs plan 2.4).
pub fn to_obj_objects(raw: &RawMesh, objects: &RawObjects, mtl_name: &str, texture_names: &[String]) -> (String, String) {
    write_obj(raw, Some(objects), mtl_name, texture_names)
}

fn write_obj(raw: &RawMesh, objects: Option<&RawObjects>, mtl_name: &str, texture_names: &[String]) -> (String, String) {
    // Index of a value in its OBJ list (1-based), adding it on first use. Insertion order defines the file order.
    fn intern<const N: usize>(map: &mut HashMap<[u32; N], usize>, list: &mut Vec<[f32; N]>, value: [f32; N]) -> usize {
        let key = value.map(|c| (c + 0.0).to_bits());
        *map.entry(key).or_insert_with(|| {
            list.push(value);
            list.len()
        })
    }
    let (mut v_map, mut vt_map, mut vn_map) = (HashMap::new(), HashMap::new(), HashMap::new());
    let mut v_list: Vec<[f32; 3]> = Vec::new();
    let mut vt_list: Vec<[f32; 2]> = Vec::new();
    let mut vn_list: Vec<[f32; 3]> = Vec::new();
    let mut faces = String::new();
    let mut current: Option<u32> = None;
    let mut current_object: Option<u32> = None;
    for (i, t) in raw.triangles.iter().enumerate() {
        if let Some(objects) = objects {
            let object = objects.tri_object.get(i).copied().unwrap_or(0);
            if current_object != Some(object) {
                current_object = Some(object);
                let name = objects.names.get(object as usize).map_or("", String::as_str);
                let _ = writeln!(faces, "o {name}");
            }
        }
        if current != Some(t.material) {
            current = Some(t.material);
            let _ = writeln!(faces, "usemtl {}", obj_material_name(raw, t.material));
        }
        faces.push('f');
        for k in 0..3 {
            let v = intern(&mut v_map, &mut v_list, t.positions[k].to_array());
            let vt = t.uvs.map(|uv| intern(&mut vt_map, &mut vt_list, [uv[k].x, 1.0 - uv[k].y]));
            let vn = t.normals.map(|n| intern(&mut vn_map, &mut vn_list, n[k].to_array()));
            let _ = match (vt, vn) {
                (Some(vt), Some(vn)) => write!(faces, " {v}/{vt}/{vn}"),
                (Some(vt), None) => write!(faces, " {v}/{vt}"),
                (None, Some(vn)) => write!(faces, " {v}//{vn}"),
                (None, None) => write!(faces, " {v}"),
            };
        }
        faces.push('\n');
    }

    let mut obj = String::with_capacity(faces.len() + v_list.len() * 96);
    obj.push_str("# Wrapped Texture Editor sample mesh\n");
    if !mtl_name.is_empty() {
        let _ = writeln!(obj, "mtllib {mtl_name}");
    }
    for [x, y, z] in &v_list {
        let _ = writeln!(obj, "v {x} {y} {z}");
    }
    for [u, v] in &vt_list {
        let _ = writeln!(obj, "vt {u} {v}");
    }
    for [x, y, z] in &vn_list {
        let _ = writeln!(obj, "vn {x} {y} {z}");
    }
    obj.push_str(&faces);

    let mut mtl = String::from("# Wrapped Texture Editor sample materials\n");
    for (i, m) in raw.materials.iter().enumerate() {
        let [r, g, b, a] = m.base_color;
        let _ = writeln!(mtl, "newmtl {}", obj_material_name(raw, i as u32));
        let _ = writeln!(mtl, "Kd {r} {g} {b}");
        let _ = writeln!(mtl, "d {a}");
        if let Some(name) = m.texture.and_then(|t| texture_names.get(t as usize)).filter(|n| !n.is_empty()) {
            let _ = writeln!(mtl, "map_Kd {name}");
        }
        mtl.push('\n');
    }
    (obj, mtl)
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4 (scaffold S1): the generators of FEATURE_BRUSH section 8 and the three model-file writers. The bake
// fixtures (`open_box` to `cup`) carry no UVs and no normals.
// ---------------------------------------------------------------------------------------------------------------------

/// Names of the three objects of `object_trio`, in object order.
pub const OBJECT_TRIO_NAMES: [&str; 3] = ["Floor", "Cube", "Ball"];

/// Objects-tabs plan 2.4: three separate shells with UVs in disjoint regions of one texture (one material "Trio"
/// referencing texture 0, one set). Object 0 **"Floor"**: an 8 x 8 grid of quads (`x`, `z` in -3..3, `y` = 0, facing
/// +Y; the grid lines are shifted so that no floor vertex meets a cube or ball vertex), UV `u` 0.5..0.99, `v`
/// 0.01..0.99. Object 1 **"Cube"**: a closed unit cube (`x`, `z` in -0.5..0.5, `y` in 0..1) standing on the floor in
/// the middle of the model, its faces (in `push_box` order) the cells of a 2 x 3 grid in `u` 0..0.5, `v` 0..0.5.
/// Object 2 **"Ball"**: `uv_sphere(24, 12, 0.5)` centred at (1.75, 0.5, 0) with its UVs scaled into `u` 0.01..0.49,
/// `v` 0.51..0.99. A top-down ray through the cube hits the cube's top face first and the floor behind it.
pub fn object_trio() -> (RawMesh, RawObjects) {
    let mut triangles = Vec::new();
    let mut tri_object = Vec::new();
    // Floor.
    let n = 8u32;
    let (u0, u1, v0, v1) = (0.5f32, 0.99f32, 0.01f32, 0.99f32);
    let (du, dv, step) = ((u1 - u0) / n as f32, (v1 - v0) / n as f32, 6.0 / n as f32);
    let line = |k: u32| if k == 0 { 0.0 } else if k == n { 6.0 } else { step * (k as f32 + 0.37) };
    for j in 0..n {
        for i in 0..n {
            let (xa, xb) = (-3.0 + line(i), -3.0 + line(i + 1));
            let (za, zb) = (3.0 - line(j), 3.0 - line(j + 1));
            let floor = [Vec3::new(xa, 0.0, za), Vec3::new(xb, 0.0, za), Vec3::new(xb, 0.0, zb), Vec3::new(xa, 0.0, zb)];
            let island = [u0 + du * i as f32, u0 + du * (i + 1) as f32, v1 - dv * (j + 1) as f32, v1 - dv * j as f32];
            push_island_quad(&mut triangles, floor, island, 0);
        }
    }
    tri_object.resize(triangles.len(), 0);
    // Cube.
    let cell = |f: usize| {
        let (col, row) = ((f % 2) as f32, (f / 2) as f32);
        let (w, h) = (0.25, 0.5 / 3.0);
        [col * w + 0.01, (col + 1.0) * w - 0.01, row * h + 0.01, (row + 1.0) * h - 0.01]
    };
    push_box(&mut triangles, Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5), Some(std::array::from_fn(cell)), 0);
    tri_object.resize(triangles.len(), 1);
    // Ball.
    let centre = Vec3::new(1.75, 0.5, 0.0);
    for mut t in uv_sphere(24, 12, 0.5).triangles {
        t.positions = t.positions.map(|p| p + centre);
        t.uvs = t.uvs.map(|uv| uv.map(|c| Vec2::new(0.01 + c.x * 0.48, 0.51 + c.y * 0.48)));
        t.material = 0;
        triangles.push(t);
    }
    tri_object.resize(triangles.len(), 2);
    let raw = RawMesh { triangles, materials: vec![material("Trio", [0.8, 0.8, 0.8, 1.0], Some(0))] };
    (raw, RawObjects { tri_object, names: OBJECT_TRIO_NAMES.map(str::to_owned).to_vec() })
}

/// The atlas cell `[u0, u1, v0, v1]` of island `index` in a grid of `cols x rows` cells, inset by 1/64 of a cell
/// width on every side.
fn atlas_cell(index: u32, cols: u32, rows: u32) -> [f32; 4] {
    let (col, row) = ((index % cols) as f32, (index / cols) as f32);
    let (w, h) = (1.0 / cols as f32, 1.0 / rows as f32);
    let m = w.min(h) / 64.0;
    [col * w + m, (col + 1.0) * w - m, row * h + m, (row + 1.0) * h - m]
}

/// The six faces of an axis-aligned box from `min` to `max`, in the order and orientation of `seam_cube` (+X, -X,
/// +Y, -Y, +Z, -Z; each island upright and unmirrored as seen from outside), face `f` mapped to `islands[f]`
/// (`[u0, u1, v0, v1]`, or no UVs with `None`). Outward winding, no explicit normals.
fn push_box(out: &mut Vec<RawTriangle>, min: Vec3, max: Vec3, islands: Option<[[f32; 4]; 6]>, material: u32) {
    let (c, e) = ((min + max) * 0.5, (max - min) * 0.5);
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    for (f, (n, r, u)) in faces.into_iter().enumerate() {
        let corner = |sx: f32, sy: f32| c + (n + r * sx + u * sy) * e;
        let p = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
        match islands {
            Some(islands) => {
                let [u0, u1, v0, v1] = islands[f];
                push_quad(out, p, [Vec2::new(u0, v1), Vec2::new(u1, v1), Vec2::new(u1, v0), Vec2::new(u0, v0)], None, material);
            }
            None => push_bare_quad(out, p, material),
        }
    }
}

/// A quad without UVs and normals (corners counter-clockwise from the front, as `push_quad`).
fn push_bare_quad(out: &mut Vec<RawTriangle>, p: [Vec3; 4], material: u32) {
    for [a, b, c] in [[0usize, 1, 2], [0, 2, 3]] {
        out.push(RawTriangle { positions: [p[a], p[b], p[c]], normals: None, uvs: None, material });
    }
}

/// A quad with UVs given as an island rectangle (BL corner at `(u0, v1)`, as `push_xy_quad`).
fn push_island_quad(out: &mut Vec<RawTriangle>, p: [Vec3; 4], island: [f32; 4], material: u32) {
    let [u0, u1, v0, v1] = island;
    push_quad(out, p, [Vec2::new(u0, v1), Vec2::new(u1, v1), Vec2::new(u1, v0), Vec2::new(u0, v0)], None, material);
}

/// A mesh of bare geometry (one material, no UVs, no normals).
fn bare(triangles: Vec<RawTriangle>, name: &str) -> RawMesh {
    RawMesh { triangles, materials: vec![material(name, [0.8, 0.8, 0.8, 1.0], None)] }
}

/// A plate of 1 x 1 x `thickness`: `x`, `y` in 0..1, `z` in `-thickness..0`, so its front face is `flat_quad`'s
/// square. Six islands in the 3 x 2 atlas of `seam_cube` (`seam_cube_island`): front (+Z) is island 4, back (-Z)
/// island 5.
pub fn thin_slab(thickness: f32) -> RawMesh {
    let mut triangles = Vec::with_capacity(12);
    let islands = std::array::from_fn(|f| seam_cube_island(f as u32));
    push_box(&mut triangles, Vec3::new(0.0, 0.0, -thickness), Vec3::new(1.0, 1.0, 0.0), Some(islands), 0);
    RawMesh { triangles, materials: vec![material("Slab", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// Two unit quads sharing the edge `x = 1`: quad A (triangles 0, 1) is `flat_quad`'s square facing +Z; quad B
/// (triangles 2, 3) runs from the edge in the direction `(-cos a, 0, sin a)` for `y` in 0..1, so the angle between
/// the two front faces is `inner_angle_deg` (180 = flat, 90 = a right-angled valley, below 90 B leans over A). B's
/// front faces `(-sin a, 0, -cos a)`. Islands: A u 0.05..0.45, B u 0.55..0.95, both v 0.3..0.7.
pub fn fold(inner_angle_deg: f32) -> RawMesh {
    let a = inner_angle_deg.to_radians();
    let d = Vec3::new(-a.cos(), 0.0, a.sin());
    let mut triangles = Vec::with_capacity(4);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [0.05, 0.45], [0.3, 0.7], 0);
    let hinge = Vec3::new(1.0, 0.0, 0.0);
    let p = [hinge, hinge + d, hinge + d + Vec3::Y, hinge + Vec3::Y];
    push_island_quad(&mut triangles, p, [0.55, 0.95, 0.3, 0.7], 0);
    RawMesh { triangles, materials: vec![material("Fold", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// Two unit cubes side by side along X with `gap` between them: block A `[0, 1]^3` (triangles 0..12), block B `x` in
/// `1 + gap..2 + gap` (triangles 12..24). Twelve islands in a 3 x 4 atlas: face `f` of block `b` is cell
/// `f + 6 b` (`[u0, u1, v0, v1]` from `two_blocks_island`).
pub fn two_blocks(gap: f32) -> RawMesh {
    let mut triangles = Vec::with_capacity(24);
    for b in 0..2u32 {
        let x0 = b as f32 * (1.0 + gap);
        let islands = std::array::from_fn(|f| two_blocks_island(f as u32 + 6 * b));
        push_box(&mut triangles, Vec3::new(x0, 0.0, 0.0), Vec3::new(x0 + 1.0, 1.0, 1.0), Some(islands), 0);
    }
    RawMesh { triangles, materials: vec![material("Blocks", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// UV rectangle `[u0, u1, v0, v1]` of island `index` (0..12) of `two_blocks`.
pub fn two_blocks_island(index: u32) -> [f32; 4] {
    atlas_cell(index, 3, 4)
}

/// Two long coplanar strips in the plane z = 0, facing +Z, that share only the corner `(2, 0.15, 0)`: strip A
/// (triangle 0) from the edge `y` 0..0.1 at `x = 0`, strip B (triangle 1) from the edge `y` 0.2..0.3 at `x = 0`, so
/// they are 0.1 apart at `x = 0`. Islands: A u 0.05..0.45, B u 0.55..0.95.
pub fn fan_strips() -> RawMesh {
    let s = Vec3::new(2.0, 0.15, 0.0);
    let tri = |a: Vec3, b: Vec3, uv: [Vec2; 3]| RawTriangle { positions: [a, s, b], normals: None, uvs: Some(uv), material: 0 };
    let triangles = vec![
        tri(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.1, 0.0), [Vec2::new(0.05, 0.9), Vec2::new(0.45, 0.5), Vec2::new(0.05, 0.1)]),
        tri(Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.0, 0.3, 0.0), [Vec2::new(0.55, 0.9), Vec2::new(0.95, 0.5), Vec2::new(0.55, 0.1)]),
    ];
    RawMesh { triangles, materials: vec![material("Fan", [0.8, 0.8, 0.8, 1.0], None)] }
}
/// A plate with an undercut ridge, extruded along Y (`y` in 0..1). Profile in x/z: the plate z = 0 for `x` in 0..1
/// (front +Z, island 0); a front wall from `(1, 0)` up to `(1 - 0.3 cos 70deg, 0.3 sin 70deg)` that overhangs the
/// plate, its face normal leaning 110 degrees away from the plate normal (island 1); the plateau on top of the
/// ridge, z = 0.3 sin 70deg, up to `x = 1.6` (front +Z, island 2); a back wall straight down at `x = 1.6` (front +X,
/// island 3). Islands are the cells of a 2 x 2 atlas (`atlas_cell(i, 2, 2)`).
pub fn undercut_ridge() -> RawMesh {
    let lean = 70f32.to_radians();
    let top = Vec3::new(1.0 - 0.3 * lean.cos(), 0.0, 0.3 * lean.sin());
    let h = top.z;
    let at = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    let mut triangles = Vec::with_capacity(8);
    push_island_quad(&mut triangles, [at(0.0, 0.0, 0.0), at(1.0, 0.0, 0.0), at(1.0, 1.0, 0.0), at(0.0, 1.0, 0.0)], atlas_cell(0, 2, 2), 0);
    // Front wall: from the foot (x = 1) up to the overhanging top edge; its front faces -X and down.
    push_island_quad(&mut triangles, [at(1.0, 1.0, 0.0), at(1.0, 0.0, 0.0), at(top.x, 0.0, h), at(top.x, 1.0, h)], atlas_cell(1, 2, 2), 0);
    push_island_quad(&mut triangles, [at(top.x, 0.0, h), at(1.6, 0.0, h), at(1.6, 1.0, h), at(top.x, 1.0, h)], atlas_cell(2, 2, 2), 0);
    push_island_quad(&mut triangles, [at(1.6, 0.0, h), at(1.6, 0.0, 0.0), at(1.6, 1.0, 0.0), at(1.6, 1.0, h)], atlas_cell(3, 2, 2), 0);
    RawMesh { triangles, materials: vec![material("Ridge", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// A unit quad (`x`, `y` in 0..1, triangles 0, 1) next to two quads `x` in 1..2 that split its right edge at
/// `y = 0.5` (triangles 2, 3: `y` 0..0.5; triangles 4, 5: `y` 0.5..1), all in z = 0 facing +Z. Three islands: the
/// cells 0, 1, 2 of a 2 x 2 atlas.
pub fn t_junction_quads() -> RawMesh {
    let mut triangles = Vec::with_capacity(6);
    let island = |i: u32| atlas_cell(i, 2, 2);
    let [a, b, c] = [island(0), island(1), island(2)];
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [a[0], a[1]], [a[2], a[3]], 0);
    push_xy_quad(&mut triangles, [1.0, 2.0], [0.0, 0.5], 0.0, [b[0], b[1]], [b[2], b[3]], 0);
    push_xy_quad(&mut triangles, [1.0, 2.0], [0.5, 1.0], 0.0, [c[0], c[1]], [c[2], c[3]], 0);
    RawMesh { triangles, materials: vec![material("TJunction", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// Two unit quads in z = 0 facing +Z with a crack of `eps` between them: A `x` in 0..1 (triangles 0, 1), B `x` in
/// `1 + eps..2 + eps` (triangles 2, 3), `y` in 0..1. They share no vertex position (for `eps > 0`). Islands: A u
/// 0.05..0.45, B u 0.55..0.95, both v 0.3..0.7.
pub fn cracked_quads(eps: f32) -> RawMesh {
    let mut triangles = Vec::with_capacity(4);
    push_xy_quad(&mut triangles, [0.0, 1.0], [0.0, 1.0], 0.0, [0.05, 0.45], [0.3, 0.7], 0);
    push_xy_quad(&mut triangles, [1.0 + eps, 2.0 + eps], [0.0, 1.0], 0.0, [0.55, 0.95], [0.3, 0.7], 0);
    RawMesh { triangles, materials: vec![material("Cracked", [0.8, 0.8, 0.8, 1.0], None)] }
}

/// `flat_quad` turned by `degrees` about the X axis (through the origin); UVs kept.
pub fn flat_quad_tilted(degrees: f32) -> RawMesh {
    let mut raw = flat_quad();
    let q = glam::Quat::from_rotation_x(degrees.to_radians());
    for t in &mut raw.triangles {
        for p in &mut t.positions {
            *p = q * *p;
        }
    }
    raw
}

/// A deterministic soup of `triangles` small triangles in the unit cube, spread over `sets` materials (triangle `i`
/// uses material `i % sets`), each triangle with its own UV triangle in its own cell of its material's atlas (no
/// overlaps). Materials have no texture. `seed` picks the sequence.
pub fn soup_sets(seed: u64, triangles: u32, sets: u32) -> RawMesh {
    let sets = sets.max(1);
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    let per_set = triangles.div_ceil(sets).max(1);
    let cells = (per_set as f32).sqrt().ceil().max(1.0) as u32;
    let mut out = Vec::with_capacity(triangles as usize);
    for i in 0..triangles {
        let a = Vec3::new(next(), next(), next());
        let b = a + (Vec3::new(next(), next(), next()) - Vec3::splat(0.5)) * 0.1;
        let c = a + (Vec3::new(next(), next(), next()) - Vec3::splat(0.5)) * 0.1;
        let k = i / sets;
        let (cx, cy) = ((k % cells) as f32, (k / cells) as f32);
        let w = 1.0 / cells as f32;
        let uv = [
            Vec2::new((cx + 0.1) * w, (cy + 0.9) * w),
            Vec2::new((cx + 0.9) * w, (cy + 0.9) * w),
            Vec2::new((cx + 0.5) * w, (cy + 0.1) * w),
        ];
        out.push(RawTriangle { positions: [a, b, c], normals: None, uvs: Some(uv), material: i % sets });
    }
    let materials = (0..sets).map(|s| material(&format!("Soup {s}"), [0.8, 0.8, 0.8, 1.0], None)).collect();
    RawMesh { triangles: out, materials }
}

/// A cube of edge `side` centred at the origin without its top face (+Y): five faces, one opening.
pub fn open_box(side: f32) -> RawMesh {
    let mut closed = Vec::with_capacity(12);
    push_box(&mut closed, Vec3::splat(-side / 2.0), Vec3::splat(side / 2.0), None, 0);
    // Faces in `push_box` order: +X, -X, +Y, -Y, +Z, -Z; drop +Y (triangles 4, 5).
    let triangles = closed.into_iter().enumerate().filter(|(i, _)| !(4..6).contains(i)).map(|(_, t)| t).collect();
    bare(triangles, "OpenBox")
}

/// Two unit cubes, the second shifted by 0.5 along X: `[-0.5, 0.5]^3` and `x` in 0..1.
pub fn overlapping_cubes() -> RawMesh {
    let mut triangles = Vec::with_capacity(24);
    push_box(&mut triangles, Vec3::splat(-0.5), Vec3::splat(0.5), None, 0);
    push_box(&mut triangles, Vec3::new(0.0, -0.5, -0.5), Vec3::new(1.0, 0.5, 0.5), None, 0);
    bare(triangles, "Overlap")
}

/// The unit cube `[-0.5, 0.5]^3` with an inner wall across it: the quad x = 0 spanning the whole cross-section, or,
/// with `slanted`, a quad from `x = -0.1` at `z = -0.5` to `x = 0.1` at `z = 0.5`, so it meets the skin at an angle.
pub fn cube_with_inner_wall(slanted: bool) -> RawMesh {
    let mut triangles = Vec::with_capacity(14);
    push_box(&mut triangles, Vec3::splat(-0.5), Vec3::splat(0.5), None, 0);
    let s = if slanted { 0.1 } else { 0.0 };
    let wall = [
        Vec3::new(-s, -0.5, -0.5),
        Vec3::new(s, -0.5, 0.5),
        Vec3::new(s, 0.5, 0.5),
        Vec3::new(-s, 0.5, -0.5),
    ];
    push_bare_quad(&mut triangles, wall, 0);
    bare(triangles, "Walled")
}

/// The unit cube `[-0.5, 0.5]^3` with every face twice (24 triangles).
pub fn doubled_cube() -> RawMesh {
    let mut triangles = Vec::with_capacity(24);
    push_box(&mut triangles, Vec3::splat(-0.5), Vec3::splat(0.5), None, 0);
    let copy = triangles.clone();
    triangles.extend(copy);
    bare(triangles, "Doubled")
}

/// A unit quad centred at the origin in the XY plane, turned 30 degrees about X and then 20 degrees about Y: a flat
/// sheet that lies in none of the main planes.
pub fn tilted_quad() -> RawMesh {
    let q = glam::Quat::from_rotation_y(20f32.to_radians()) * glam::Quat::from_rotation_x(30f32.to_radians());
    let p = [Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 0.0), Vec3::new(-0.5, 0.5, 0.0)].map(|p| q * p);
    let mut triangles = Vec::with_capacity(2);
    push_bare_quad(&mut triangles, p, 0);
    bare(triangles, "Tilted")
}

/// A closed unit cube (`x`, `z` in -0.5..0.5, `y` in 0..1) standing on an open 10 x 10 floor quad (y = 0, facing
/// +Y).
pub fn cube_on_floor() -> RawMesh {
    let mut triangles = Vec::with_capacity(14);
    push_box(&mut triangles, Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5), None, 0);
    let floor = [Vec3::new(-5.0, 0.0, 5.0), Vec3::new(5.0, 0.0, 5.0), Vec3::new(5.0, 0.0, -5.0), Vec3::new(-5.0, 0.0, -5.0)];
    push_bare_quad(&mut triangles, floor, 0);
    bare(triangles, "CubeOnFloor")
}

/// `cube_on_floor` plus a closed rod 0.05 thick and 3 long that sticks out of the cube: `x` in 0..3, `y` in
/// 0.475..0.525, `z` in -0.025..0.025.
pub fn cube_floor_rod() -> RawMesh {
    let mut raw = cube_on_floor();
    push_box(&mut raw.triangles, Vec3::new(0.0, 0.475, -0.025), Vec3::new(3.0, 0.525, 0.025), None, 0);
    raw
}

/// Two closed unit cubes in opposite corners of a 20-unit box: `[0, 1]^3` and `[19, 20]^3`.
pub fn corner_cubes() -> RawMesh {
    let mut triangles = Vec::with_capacity(24);
    push_box(&mut triangles, Vec3::ZERO, Vec3::ONE, None, 0);
    push_box(&mut triangles, Vec3::splat(19.0), Vec3::splat(20.0), None, 0);
    bare(triangles, "Corners")
}

/// Points of a circle of radius `r` at height `y` in the XZ plane, `segments` of them, counter-clockwise seen from
/// +Y.
fn ring(r: f32, y: f32, segments: u32) -> Vec<Vec3> {
    (0..segments)
        .map(|i| {
            let a = TAU * i as f32 / segments as f32;
            Vec3::new(r * a.cos(), y, -r * a.sin())
        })
        .collect()
}

/// The side, and the chosen caps, of a cylinder with axis Y: radius `r`, `y` in `y0..y1`, 64 segments; outward
/// winding.
fn cylinder_parts(r: f32, y0: f32, y1: f32, bottom: bool, top: bool) -> Vec<RawTriangle> {
    const SEGMENTS: u32 = 64;
    let (lo, hi) = (ring(r, y0, SEGMENTS), ring(r, y1, SEGMENTS));
    let mut out = Vec::new();
    let tri = |a: Vec3, b: Vec3, c: Vec3| RawTriangle { positions: [a, b, c], normals: None, uvs: None, material: 0 };
    for i in 0..SEGMENTS as usize {
        let j = (i + 1) % SEGMENTS as usize;
        // Seen from outside: lo[i], lo[j] run counter-clockwise around +Y, so (lo[i], lo[j], hi[j]) faces outward.
        out.push(tri(lo[i], lo[j], hi[j]));
        out.push(tri(lo[i], hi[j], hi[i]));
        if top {
            out.push(tri(Vec3::new(0.0, y1, 0.0), hi[i], hi[j]));
        }
        if bottom {
            out.push(tri(Vec3::new(0.0, y0, 0.0), lo[j], lo[i]));
        }
    }
    out
}

/// A closed cylinder (a coin) with axis Y: `diameter` across, `thickness` high, centred at the origin.
pub fn coin(diameter: f32, thickness: f32) -> RawMesh {
    bare(cylinder_parts(diameter / 2.0, -thickness / 2.0, thickness / 2.0, true, true), "Coin")
}

/// A five-armed star polygon in the XY plane (z = 0, facing +Z), outer radius 1, inner radius 0.4, the first arm on
/// +X; no thickness. Ten triangles fanned from the centre.
pub fn flat_star() -> RawMesh {
    let point = |i: u32| {
        let r = if i.is_multiple_of(2) { 1.0 } else { 0.4 };
        let a = TAU * i as f32 / 10.0;
        Vec3::new(r * a.cos(), r * a.sin(), 0.0)
    };
    let triangles = (0..10)
        .map(|i| RawTriangle { positions: [Vec3::ZERO, point(i), point((i + 1) % 10)], normals: None, uvs: None, material: 0 })
        .collect();
    bare(triangles, "Star")
}

/// A closed box 1 x 1 x 0.2 centred at the origin, thin along Z.
pub fn plate_xy() -> RawMesh {
    let mut triangles = Vec::with_capacity(12);
    push_box(&mut triangles, Vec3::new(-0.5, -0.5, -0.1), Vec3::new(0.5, 0.5, 0.1), None, 0);
    bare(triangles, "Plate")
}

/// A cup for the closing-openings tests (plan section 7 item 5): a cylinder with axis Y, radius 0.5, `y` in 0..1,
/// with a bottom and without a top (one circular opening of diameter 1). Zero wall thickness.
pub fn cup() -> RawMesh {
    bare(cylinder_parts(0.5, 0.0, 1.0, true, false), "Cup")
}

/// A cup with a non-planar opening for the closing-openings tests (plan section 7 item 5: "a mesh with a non-planar
/// hole loop caps without self-intersection failures"): `cup()` whose rim is a saddle, `y = 1 + 0.25 cos 2a` around
/// the axis, so the rim runs from `y = 0.75` to `y = 1.25` and its 64 points lie in no plane. Bottom closed, zero
/// wall thickness, outward winding.
pub fn saddle_cup() -> RawMesh {
    const SEGMENTS: usize = 64;
    let (lo, hi) = (ring(0.5, 0.0, SEGMENTS as u32), ring(0.5, 1.0, SEGMENTS as u32));
    let hi: Vec<Vec3> = hi
        .into_iter()
        .enumerate()
        .map(|(i, p)| Vec3::new(p.x, 1.0 + 0.25 * (2.0 * TAU * i as f32 / SEGMENTS as f32).cos(), p.z))
        .collect();
    let tri = |a: Vec3, b: Vec3, c: Vec3| RawTriangle { positions: [a, b, c], normals: None, uvs: None, material: 0 };
    let mut triangles = Vec::with_capacity(3 * SEGMENTS);
    for i in 0..SEGMENTS {
        let j = (i + 1) % SEGMENTS;
        triangles.push(tri(lo[i], lo[j], hi[j]));
        triangles.push(tri(lo[i], hi[j], hi[i]));
        triangles.push(tri(Vec3::ZERO, lo[j], lo[i]));
    }
    bare(triangles, "SaddleCup")
}

// ----- round 6: wrap fixtures (plan 4.6) -----

/// An open cylinder (an extruded regular polygon, no caps) with axis Y: `segments` sides (at least 3) of a polygon of
/// circumradius `radius`, `y` in `-height / 2..height / 2`, outward winding, smooth normals (from `normalize`). Ring
/// vertex `i` lies at angle `TAU * i / segments` from +X towards -Z (counter-clockwise seen from +Y). One UV island:
/// `u = 0.02 + 0.96 * i / segments` around, `v = 0.02 + 0.96 * (height / 2 - y) / height` (top up); its seam (u =
/// 0.02 against u = 0.98, two vertex columns with identical positions) runs down the +X side. One material
/// "Cylinder" referencing texture 0.
pub fn cylinder(segments: u32, radius: f32, height: f32) -> RawMesh {
    let seg = segments.max(3);
    let (y0, y1) = (-height / 2.0, height / 2.0);
    let point = |i: u32, y: f32| {
        let a = TAU * (i % seg) as f32 / seg as f32;
        Vec3::new(radius * a.cos(), y, -radius * a.sin())
    };
    let uv = |i: u32, top: bool| Vec2::new(0.02 + 0.96 * i as f32 / seg as f32, if top { 0.02 } else { 0.98 });
    let mut triangles = Vec::with_capacity(2 * seg as usize);
    for i in 0..seg {
        push_quad(
            &mut triangles,
            [point(i, y0), point(i + 1, y0), point(i + 1, y1), point(i, y1)],
            [uv(i, false), uv(i + 1, false), uv(i + 1, true), uv(i, true)],
            None,
            0,
        );
    }
    RawMesh { triangles, materials: vec![material("Cylinder", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// A square plate `x`, `y` in -0.5..0.5 facing +Z, displaced along Z by the height field
/// `z = amplitude * sin(8 pi x) * sin(8 pi y)` (four bumps per side; the steepest slope is `amplitude * 8 pi`, so
/// `amplitude = tan(60 deg) / (8 pi)` gives 60 degrees), sampled on a grid of `cells` x `cells` quads (at least 1).
/// One UV island: `u = 0.02 + 0.96 * (x + 0.5)`, `v = 0.02 + 0.96 * (0.5 - y)`. Smooth normals. One material "Bumps"
/// referencing texture 0.
pub fn bumpy_plate(cells: u32, amplitude: f32) -> RawMesh {
    bumpy_plate_periods(cells, amplitude, 4.0)
}

/// `bumpy_plate` with `periods` sine periods per side (the steepest slope is `amplitude * 2 pi * periods`).
pub fn bumpy_plate_periods(cells: u32, amplitude: f32, periods: f32) -> RawMesh {
    let n = cells.max(1);
    let at = |i: u32, j: u32| {
        let (x, y) = (i as f32 / n as f32 - 0.5, j as f32 / n as f32 - 0.5);
        let z = amplitude * (periods * TAU * x).sin() * (periods * TAU * y).sin();
        (Vec3::new(x, y, z), Vec2::new(0.02 + 0.96 * (x + 0.5), 0.02 + 0.96 * (0.5 - y)))
    };
    let mut triangles = Vec::with_capacity(2 * (n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let c = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            push_quad(&mut triangles, c.map(|c| c.0), c.map(|c| c.1), None, 0);
        }
    }
    RawMesh { triangles, materials: vec![material("Bumps", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// The 32 cell colours of `cells_picture` (8 columns x 4 rows, row-major from the top left): channel levels 30, 100,
/// 170 and 240, combinations whose level indices sum to an even number, so two cells differ by at least 70 in two
/// channels or 140 in one.
pub fn cells_colors() -> [[u8; 3]; 32] {
    const LEVELS: [u8; 4] = [30, 100, 170, 240];
    let mut out = [[0u8; 3]; 32];
    let mut k = 0;
    for (r, &lr) in LEVELS.iter().enumerate() {
        for (g, &lg) in LEVELS.iter().enumerate() {
            for (b, &lb) in LEVELS.iter().enumerate() {
                if (r + g + b) % 2 == 0 && k < 32 {
                    out[k] = [lr, lg, lb];
                    k += 1;
                }
            }
        }
    }
    // A fixed shuffle (stride 13 is coprime to 32) keeps similar colours out of neighbouring cells.
    std::array::from_fn(|i| out[(i * 13) % 32])
}

/// Largest deviation per channel of the gradient inside a cell of `cells_picture`.
pub const CELLS_GRADIENT: u8 = 8;

/// The wrap test picture (plan 4.7): `width` x `height` opaque pixels in 8 x 4 cells of the colours of
/// `cells_colors`, each with a smooth gradient inside the cell (every channel shifted by up to `CELLS_GRADIENT`
/// levels: up along x, down along y, from the cell's top-left corner).
pub fn cells_picture(width: u32, height: u32) -> PixelBuf {
    let colors = cells_colors();
    let mut out = PixelBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = ((x as f32 + 0.5) * 8.0 / width as f32, (y as f32 + 0.5) * 4.0 / height as f32);
            let (cx, cy) = ((fx as u32).min(7), (fy as u32).min(3));
            let (tx, ty) = (fx - cx as f32, fy - cy as f32);
            let shift = (CELLS_GRADIENT as f32 * (tx - ty)).round() as i32;
            let c = colors[(cy * 8 + cx) as usize];
            let ch = |v: u8| (v as i32 + shift).clamp(0, 255) as u8;
            out.set(x, y, [ch(c[0]), ch(c[1]), ch(c[2]), 255]);
        }
    }
    out
}

/// `cube_on_floor` with UVs (round 6, Jump small gaps): a closed unit cube (`x`, `z` in -0.5..0.5, `y` in 0..1)
/// standing on a separate 4 x 4 floor quad (y = 0, facing +Y); the two parts share no vertex. One material "Floor"
/// referencing texture 0. Islands: the cube's faces in `push_box` order (+X, -X, +Y, -Y, +Z, -Z) are the cells of a
/// 2 x 3 grid in `u` 0..0.5; the floor is `u` 0.5..0.99, `v` 0.01..0.99.
pub fn uv_cube_on_floor() -> RawMesh {
    uv_cube_on_floor_n(1)
}

/// Round 9 (WA31): `uv_cube_on_floor` with the floor as `n` x `n` quads (`2 n^2` triangles; the island split to
/// match, uniformly in UV). `n = 1` is `uv_cube_on_floor` exactly.
pub fn uv_cube_on_floor_n(n: u32) -> RawMesh {
    let n = n.max(1);
    let mut triangles = Vec::with_capacity(12 + 2 * (n * n) as usize);
    let cell = |f: usize| {
        let (col, row) = ((f % 2) as f32, (f / 2) as f32);
        let (w, h) = (0.25, 1.0 / 3.0);
        [col * w + 0.01, (col + 1.0) * w - 0.01, row * h + 0.01, (row + 1.0) * h - 0.01]
    };
    push_box(&mut triangles, Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5), Some(std::array::from_fn(cell)), 0);
    let (u0, u1, v0, v1) = (0.5f32, 0.99f32, 0.01f32, 0.99f32);
    let (du, dv, step) = ((u1 - u0) / n as f32, (v1 - v0) / n as f32, 4.0 / n as f32);
    // The interior grid lines are shifted by 0.37 of a step: no floor vertex coincides with a cube corner (a
    // coincident one welds the floor and the cube into one part).
    let line = |k: u32| if k == 0 { 0.0 } else if k == n { 4.0 } else { step * (k as f32 + 0.37) };
    for j in 0..n {
        for i in 0..n {
            let (xa, xb) = (-2.0 + line(i), -2.0 + line(i + 1));
            let (za, zb) = (2.0 - line(j), 2.0 - line(j + 1));
            let floor = [Vec3::new(xa, 0.0, za), Vec3::new(xb, 0.0, za), Vec3::new(xb, 0.0, zb), Vec3::new(xa, 0.0, zb)];
            let island = [u0 + du * i as f32, u0 + du * (i + 1) as f32, v1 - dv * (j + 1) as f32, v1 - dv * j as f32];
            push_island_quad(&mut triangles, floor, if n == 1 { [u0, u1, v0, v1] } else { island }, 0);
        }
    }
    RawMesh { triangles, materials: vec![material("Floor", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// Round 8 (wrap quality): a bumpy ball. An icosphere subdivided `subdiv` times (20 x 4^subdiv triangles) whose radius
/// is `1 + h`, `h` the sum of `bumps` Gaussian bumps (width 0.18 rad) centred on a Fibonacci spiral of the sphere, of a
/// height that gives a steepest slope of about `slope_deg` against the sphere. Six UV islands (gnomonic projection of
/// each triangle onto the cube face of its centroid direction, cells of a 3 x 2 atlas). No explicit normals. One
/// material "Bumps" referencing texture 0.
pub fn bumpy_sphere(subdiv: u32, bumps: u32, slope_deg: f32) -> RawMesh {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut dirs: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdiv.min(7) {
        let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next = Vec::with_capacity(faces.len() * 4);
        let mut midpoint = |a: u32, b: u32, dirs: &mut Vec<Vec3>| -> u32 {
            let key = (a.min(b), a.max(b));
            *mid.entry(key).or_insert_with(|| {
                dirs.push(((dirs[a as usize] + dirs[b as usize]) * 0.5).normalize());
                (dirs.len() - 1) as u32
            })
        };
        for &[a, b, c] in &faces {
            let ab = midpoint(a, b, &mut dirs);
            let bc = midpoint(b, c, &mut dirs);
            let ca = midpoint(c, a, &mut dirs);
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    let n = bumps.max(1);
    let golden = PI * (3.0 - 5f32.sqrt());
    let centres: Vec<Vec3> = (0..n)
        .map(|k| {
            let y = 1.0 - 2.0 * (k as f32 + 0.5) / n as f32;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let phi = k as f32 * golden;
            Vec3::new(r * phi.cos(), y, r * phi.sin())
        })
        .collect();
    let sigma = 0.18f32;
    // Steepest slope of a * exp(-(d / sigma)^2) is a * sqrt(2) * exp(-1/2) / sigma.
    let amplitude = slope_deg.clamp(0.0, 89.0).to_radians().tan() * sigma / (2f32.sqrt() * (-0.5f32).exp());
    let radius = |d: Vec3| -> f32 {
        1.0 + centres
            .iter()
            .map(|c| {
                let ang = d.dot(*c).clamp(-1.0, 1.0).acos();
                amplitude * (-(ang / sigma) * (ang / sigma)).exp()
            })
            .sum::<f32>()
    };
    let positions: Vec<Vec3> = dirs.iter().map(|&d| d * radius(d)).collect();
    // Cube faces in `push_box` order (+X, -X, +Y, -Y, +Z, -Z): (normal, right, up).
    let axes: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    let mut triangles = Vec::with_capacity(faces.len());
    for &[a, b, c] in &faces {
        let centroid = dirs[a as usize] + dirs[b as usize] + dirs[c as usize];
        let f = (0..6).max_by(|&i, &j| centroid.dot(axes[i].0).total_cmp(&centroid.dot(axes[j].0))).unwrap_or(0);
        let (nrm, right, up) = axes[f];
        let [u0, u1, v0, v1] = atlas_cell(f as u32, 3, 2);
        let uv = |v: u32| {
            let d = dirs[v as usize];
            let depth = d.dot(nrm).max(1e-3);
            let (x, y) = (d.dot(right) / depth, d.dot(up) / depth);
            Vec2::new(u0 + (0.5 + 0.4 * x) * (u1 - u0), v0 + (0.5 - 0.4 * y) * (v1 - v0))
        };
        triangles.push(RawTriangle {
            positions: [positions[a as usize], positions[b as usize], positions[c as usize]],
            normals: None,
            uvs: Some([uv(a), uv(b), uv(c)]),
            material: 0,
        });
    }
    RawMesh { triangles, materials: vec![material("Bumps", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// Round 8 (wrap quality): a closed, pawn-like surface of revolution about +Y (base disk of radius 0.5 at y = 0, a
/// stem, a collar and a round head up to y = 1.34), 48 segments round, profile steps of at most 0.04. Outward
/// winding, poles as triangle fans. One UV island: `u` round (seam column duplicated on the +X side), `v` along the
/// profile from the top. No explicit normals. One material "Piece" referencing texture 0.
pub fn lathe_piece() -> RawMesh {
    let seg = 48u32;
    // Profile (r, y) from the bottom pole to the top pole.
    let mut corners: Vec<(f32, f32)> =
        vec![(0.0, 0.0), (0.5, 0.0), (0.5, 0.08), (0.42, 0.14), (0.3, 0.22), (0.22, 0.4), (0.18, 0.6), (0.17, 0.75), (0.3, 0.8), (0.3, 0.84), (0.17, 0.88)];
    let (hc, hr) = (1.1f32, 0.24f32);
    let phi0 = -(0.17f32 / hr).acos();
    for k in 0..=12 {
        let phi = phi0 + (PI / 2.0 - phi0) * k as f32 / 12.0;
        let r = if k == 12 { 0.0 } else { hr * phi.cos() };
        corners.push((r, hc + hr * phi.sin()));
    }
    let mut profile: Vec<(f32, f32)> = vec![corners[0]];
    for w in corners.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let steps = (len / 0.04).ceil().max(1.0) as u32;
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            profile.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    let mut arc = vec![0.0f32; profile.len()];
    for k in 1..profile.len() {
        let (a, b) = (profile[k - 1], profile[k]);
        arc[k] = arc[k - 1] + ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    }
    let total = arc[arc.len() - 1];
    let point = |k: usize, i: u32| {
        let a = TAU * (i % seg) as f32 / seg as f32;
        let (r, y) = profile[k];
        Vec3::new(r * a.cos(), y, -r * a.sin())
    };
    let uv = |k: usize, i: f32| Vec2::new(0.02 + 0.96 * i / seg as f32, 0.02 + 0.96 * (1.0 - arc[k] / total));
    let mut triangles = Vec::new();
    for k in 0..profile.len() - 1 {
        for i in 0..seg {
            let (fi, fj) = (i as f32, (i + 1) as f32);
            if profile[k].0 == 0.0 {
                // Bottom pole: the quad's one non-degenerate half, (pole, i + 1, i), faces down (outward).
                let pole = (point(k, 0), uv(k, fi + 0.5));
                triangles.push(RawTriangle {
                    positions: [pole.0, point(k + 1, i + 1), point(k + 1, i)],
                    normals: None,
                    uvs: Some([pole.1, uv(k + 1, fj), uv(k + 1, fi)]),
                    material: 0,
                });
            } else if profile[k + 1].0 == 0.0 {
                let pole = (point(k + 1, 0), uv(k + 1, fi + 0.5));
                triangles.push(RawTriangle {
                    positions: [point(k, i), point(k, i + 1), pole.0],
                    normals: None,
                    uvs: Some([uv(k, fi), uv(k, fj), pole.1]),
                    material: 0,
                });
            } else {
                push_quad(
                    &mut triangles,
                    [point(k, i), point(k, i + 1), point(k + 1, i + 1), point(k + 1, i)],
                    [uv(k, fi), uv(k, fj), uv(k + 1, fj), uv(k + 1, fi)],
                    None,
                    0,
                );
            }
        }
    }
    RawMesh { triangles, materials: vec![material("Piece", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// Round 8 (Jump small gaps bounded): `n x n` separate closed cubes of side 0.5, one unit apart (centres at
/// `(i - (n - 1) / 2, 0.25, j - (n - 1) / 2)`), standing on a separate floor quad (y = 0, facing +Y, half extent
/// `n / 2 + 0.5`); no two parts share a vertex. Islands: the cubes' faces (`push_box` order) in a 10 x 16 grid of
/// `u` 0..0.5; the floor `u` 0.5..0.99, `v` 0.01..0.99. One material "Floor" referencing texture 0.
pub fn cubes_on_floor(n: u32) -> RawMesh {
    let n = n.clamp(1, 5);
    let mut triangles = Vec::new();
    let off = (n as f32 - 1.0) * 0.5;
    for j in 0..n {
        for i in 0..n {
            let c = Vec3::new(i as f32 - off, 0.0, j as f32 - off);
            let base = (j * n + i) * 6;
            let cell = |f: usize| {
                let [u0, u1, v0, v1] = atlas_cell(base + f as u32, 10, 16);
                [u0 * 0.5, u1 * 0.5, v0, v1]
            };
            push_box(&mut triangles, c + Vec3::new(-0.25, 0.0, -0.25), c + Vec3::new(0.25, 0.5, 0.25), Some(std::array::from_fn(cell)), 0);
        }
    }
    let e = n as f32 * 0.5 + 0.5;
    let floor = [Vec3::new(-e, 0.0, e), Vec3::new(e, 0.0, e), Vec3::new(e, 0.0, -e), Vec3::new(-e, 0.0, -e)];
    push_island_quad(&mut triangles, floor, [0.5, 0.99, 0.01, 0.99], 0);
    RawMesh { triangles, materials: vec![material("Floor", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

// ----- model-file writers -----

/// Standard base64 with padding.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The glTF JSON for `parts` (one mesh and one node per part, the node carrying the matrix) and its binary buffer:
/// per part non-indexed positions (and UVs when every triangle has them) followed by `u32` indices, each view
/// 4-byte aligned. `uri`: the buffer's URI (a `data:` URI for `.gltf`, none for `.glb`).
/// Turns the binary buffer into the buffer's URI.
type BufferUri<'a> = &'a dyn Fn(&[u8]) -> String;

fn gltf_document(parts: &[(&RawMesh, glam::Mat4)], uri: Option<BufferUri<'_>>) -> (String, Vec<u8>) {
    use serde_json::{Value, json};
    let mut bin: Vec<u8> = Vec::new();
    let (mut views, mut accessors, mut meshes, mut nodes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut push_view = |bin: &mut Vec<u8>, bytes: &[u8], target: u32| -> usize {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len(), "target": target }));
        bin.extend_from_slice(bytes);
        views.len() - 1
    };
    for (index, (raw, matrix)) in parts.iter().enumerate() {
        let positions: Vec<Vec3> = raw.triangles.iter().flat_map(|t| t.positions).collect();
        let count = positions.len();
        let (lo, hi) = positions.iter().fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let bytes: Vec<u8> = positions.iter().flat_map(|p| p.to_array()).flat_map(f32::to_le_bytes).collect();
        let view = push_view(&mut bin, &bytes, 34962);
        accessors.push(json!({
            "bufferView": view, "componentType": 5126, "count": count, "type": "VEC3",
            "min": (if count > 0 { lo.to_array().to_vec() } else { vec![0.0; 3] }),
            "max": (if count > 0 { hi.to_array().to_vec() } else { vec![0.0; 3] }),
        }));
        let mut attributes = serde_json::Map::new();
        attributes.insert("POSITION".to_owned(), Value::from(accessors.len() - 1));
        if !raw.triangles.is_empty() && raw.triangles.iter().all(|t| t.uvs.is_some()) {
            let uvs: Vec<u8> =
                raw.triangles.iter().flat_map(|t| t.uvs.unwrap_or_default()).flat_map(|uv| uv.to_array()).flat_map(f32::to_le_bytes).collect();
            let view = push_view(&mut bin, &uvs, 34962);
            accessors.push(json!({ "bufferView": view, "componentType": 5126, "count": count, "type": "VEC2" }));
            attributes.insert("TEXCOORD_0".to_owned(), Value::from(accessors.len() - 1));
        }
        let indices: Vec<u8> = (0..count as u32).flat_map(u32::to_le_bytes).collect();
        let view = push_view(&mut bin, &indices, 34963);
        accessors.push(json!({ "bufferView": view, "componentType": 5125, "count": count, "type": "SCALAR" }));
        meshes.push(json!({ "name": format!("mesh{index}"), "primitives": [ { "attributes": attributes, "indices": accessors.len() - 1, "mode": 4 } ] }));
        nodes.push(json!({ "name": format!("node{index}"), "mesh": index, "matrix": matrix.to_cols_array().to_vec() }));
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut buffer = json!({ "byteLength": bin.len() });
    if let Some(uri) = uri {
        buffer["uri"] = Value::from(uri(&bin));
    }
    let doc = json!({
        "asset": { "version": "2.0", "generator": "Wrapped Texture Editor samples" },
        "scene": 0,
        "scenes": [ { "nodes": (0..parts.len()).collect::<Vec<_>>() } ],
        "nodes": nodes,
        "meshes": meshes,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [ buffer ],
    });
    (doc.to_string(), bin)
}

/// A `.gltf` text with one mesh (non-indexed positions, UVs when every triangle has them, `u32` indices), one node
/// that carries the 4 x 4 matrix `node`, and the vertex data as a base64 `data:` buffer. Materials are not written.
pub fn to_gltf(raw: &RawMesh, node: glam::Mat4) -> String {
    let uri = |bin: &[u8]| format!("data:application/octet-stream;base64,{}", base64(bin));
    gltf_document(&[(raw, node)], Some(&uri)).0
}

/// A binary `.glb` (12-byte header, one JSON chunk, one BIN chunk) with one mesh and one node per entry of `parts`,
/// so that one file can hold several meshes with different node transforms.
pub fn to_glb(parts: &[(RawMesh, glam::Mat4)]) -> Vec<u8> {
    let refs: Vec<(&RawMesh, glam::Mat4)> = parts.iter().map(|(raw, m)| (raw, *m)).collect();
    let (json, bin) = gltf_document(&refs, None);
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// An ASCII FBX 7.4 text with one mesh model: the global setting `UnitScaleFactor` (centimetres per file unit; the
/// importer turns positions into metres with `unit_scale_factor * 0.01`), Y up, and the model property
/// `Lcl Scaling`. Positions are written as they are; UVs (when every triangle has them) per polygon vertex with
/// `v` flipped to FBX's v-up.
pub fn to_fbx_ascii(raw: &RawMesh, unit_scale_factor: f32, lcl_scaling: [f32; 3]) -> String {
    let numbers = |values: &mut dyn Iterator<Item = String>| values.collect::<Vec<_>>().join(",");
    let positions: Vec<f32> = raw.triangles.iter().flat_map(|t| t.positions).flat_map(|p| p.to_array()).collect();
    let count = raw.triangles.len() * 3;
    let indices = numbers(&mut (0..count as i64).map(|i| if i % 3 == 2 { (-i - 1).to_string() } else { i.to_string() }));
    let mut text = String::new();
    let _ = write!(
        text,
        "; FBX 7.4.0 project file\n\
         FBXHeaderExtension:  {{\n\tFBXVersion: 7400\n}}\n\
         GlobalSettings:  {{\n\tProperties70:  {{\n\
         \t\tP: \"UpAxis\", \"int\", \"Integer\", \"\",1\n\
         \t\tP: \"UpAxisSign\", \"int\", \"Integer\", \"\",1\n\
         \t\tP: \"CoordAxis\", \"int\", \"Integer\", \"\",0\n\
         \t\tP: \"CoordAxisSign\", \"int\", \"Integer\", \"\",1\n\
         \t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",{unit_scale_factor}\n\
         \t}}\n}}\n\
         Objects:  {{\n\
         \tGeometry: 1, \"Geometry::Shape\", \"Mesh\" {{\n\
         \t\tVertices: *{} {{\n\t\t\ta: {}\n\t\t}}\n\
         \t\tPolygonVertexIndex: *{count} {{\n\t\t\ta: {indices}\n\t\t}}\n",
        positions.len(),
        numbers(&mut positions.iter().map(|v| v.to_string())),
    );
    if !raw.triangles.is_empty() && raw.triangles.iter().all(|t| t.uvs.is_some()) {
        let uvs: Vec<f32> = raw.triangles.iter().flat_map(|t| t.uvs.unwrap_or_default()).flat_map(|uv| [uv.x, 1.0 - uv.y]).collect();
        let _ = write!(
            text,
            "\t\tLayerElementUV: 0 {{\n\t\t\tMappingInformationType: \"ByPolygonVertex\"\n\
             \t\t\tReferenceInformationType: \"Direct\"\n\t\t\tUV: *{} {{\n\t\t\t\ta: {}\n\t\t\t}}\n\t\t}}\n",
            uvs.len(),
            numbers(&mut uvs.iter().map(|v| v.to_string())),
        );
    }
    let [sx, sy, sz] = lcl_scaling;
    let _ = write!(
        text,
        "\t}}\n\
         \tModel: 2, \"Model::Shape\", \"Mesh\" {{\n\t\tProperties70:  {{\n\
         \t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",{sx},{sy},{sz}\n\t\t}}\n\t}}\n\
         }}\n\
         Connections:  {{\n\t;Geometry::Shape, Model::Shape\n\tC: \"OO\",1,2\n\t;Model::Shape, Model::RootNode\n\tC: \"OO\",2,0\n}}\n"
    );
    text
}

// ----- round 9 (wrap quality) -----

/// A grid face of `nu` x `nv` quads from `origin` along `eu`, `ev` (the face's normal is `eu x ev`), its UV island
/// `[u0, u1, v0, v1]` split uniformly.
fn push_grid_face(out: &mut Vec<RawTriangle>, origin: Vec3, eu: Vec3, ev: Vec3, nu: u32, nv: u32, island: [f32; 4]) {
    let [u0, u1, v0, v1] = island;
    let at = |i: u32, j: u32| origin + eu * (i as f32 / nu as f32) + ev * (j as f32 / nv as f32);
    let uv = |i: u32, j: u32| Vec2::new(u0 + (u1 - u0) * i as f32 / nu as f32, v1 - (v1 - v0) * j as f32 / nv as f32);
    for j in 0..nv {
        for i in 0..nu {
            push_quad(out, [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)], [uv(i, j), uv(i + 1, j), uv(i + 1, j + 1), uv(i, j + 1)], None, 0);
        }
    }
}

/// Round 9 (WA33, T3): a flat plate `x`, `y` in -0.5..0.5 facing +Z, 80 x 80 quads (cell 0.0125), with 5 x 5
/// rivets: the grid vertices at `x`, `y` in {-0.2, -0.1, 0, 0.1, 0.2} raised by `height` (a pyramid of six faces
/// whose tip deficit exceeds 45 degrees for `height` around the cell size; the ring round it carries the opposite
/// deficit, so the rivet's net curvature is zero). One UV island `u = 0.02 + 0.96 (x + 0.5)`, `v = 0.02 + 0.96 (0.5 -
/// y)`. One material "Rivets" referencing texture 0.
pub fn riveted_plate(height: f32) -> RawMesh {
    let n = 80u32;
    let rivet = |i: u32| (24..=56).contains(&i) && (i - 24).is_multiple_of(8);
    let at = |i: u32, j: u32| {
        let z = if rivet(i) && rivet(j) { height } else { 0.0 };
        Vec3::new(i as f32 / n as f32 - 0.5, j as f32 / n as f32 - 0.5, z)
    };
    let uv = |i: u32, j: u32| Vec2::new(0.02 + 0.96 * i as f32 / n as f32, 0.02 + 0.96 * (1.0 - j as f32 / n as f32));
    let mut triangles = Vec::with_capacity((2 * n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            push_quad(&mut triangles, [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)], [uv(i, j), uv(i + 1, j), uv(i + 1, j + 1), uv(i, j + 1)], None, 0);
        }
    }
    RawMesh { triangles, materials: vec![material("Rivets", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// Round 9 (WA34, T3): a box corner pushed into a floor, one connected surface: the floor `y = 0` over `x`, `z` in
/// -1..1 without the quadrant `x, z > 0`, where a block of height 0.3 stands (its top `y = 0.3` and its two walls `x =
/// 0`, `z = 0`; no other side). The block's top corner `(0, 0.3, 0)` has an angle deficit of +90 degrees, the floor's
/// corner `(0, 0, 0)` -90 degrees. 16 quads per unit on every face, so the faces share their edge vertices. Six
/// islands (3 x 2 atlas): three floor quadrants, the top, the wall `x = 0`, the wall `z = 0`. One material "Step"
/// referencing texture 0.
pub fn step_corner() -> RawMesh {
    let h = 0.3f32;
    let mut triangles = Vec::new();
    let cell = |f: u32| atlas_cell(f, 3, 2);
    // Floor quadrants (normal +Y: eu along +Z, ev along +X).
    for (f, (x0, z0)) in [(-1.0f32, -1.0f32), (0.0, -1.0), (-1.0, 0.0)].into_iter().enumerate() {
        push_grid_face(&mut triangles, Vec3::new(x0, 0.0, z0), Vec3::Z, Vec3::X, 16, 16, cell(f as u32));
    }
    // The top.
    push_grid_face(&mut triangles, Vec3::new(0.0, h, 0.0), Vec3::Z, Vec3::X, 16, 16, cell(3));
    // The wall x = 0 (normal -X: eu along +Z, ev along +Y) and the wall z = 0 (normal -Z: eu along +Y, ev along +X).
    push_grid_face(&mut triangles, Vec3::ZERO, Vec3::Z, Vec3::Y * h, 16, 5, cell(4));
    push_grid_face(&mut triangles, Vec3::ZERO, Vec3::Y * h, Vec3::X, 5, 16, cell(5));
    RawMesh { triangles, materials: vec![material("Step", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

/// Round 9 (WA35, T5): a capped cylinder: the side of `cylinder(64, 1.0, 2.0)` (the same positions per column, `y` in
/// -1..1, here in 16 rows; `u = 0.02 + 0.96 i / 64`, `v` 0.5 at the top to 0.98 at the bottom) with a hemispherical
/// dome on top (8 rings to the pole at `y = 2`, sharing the side's top ring; `v` 0.48 at its rim to 0.02 at the pole,
/// the same `u` per segment). One material "Capped" referencing texture 0. The first `2 x 64 x 16` triangles are the
/// side.
pub fn capped_cylinder() -> RawMesh {
    let (seg, rows, rings) = (64u32, 16u32, 8u32);
    let ring = |i: u32, k: u32| {
        let a = TAU * (i % seg) as f32 / seg as f32;
        let phi = 0.5 * PI * k as f32 / rings as f32;
        let (r, y) = if k == 0 { (1.0f32, 1.0f32) } else { (phi.cos(), 1.0 + phi.sin()) };
        Vec3::new(r * a.cos(), y, -r * a.sin())
    };
    let side = |i: u32, j: u32| {
        let a = TAU * (i % seg) as f32 / seg as f32;
        Vec3::new(a.cos(), -1.0 + 2.0 * j as f32 / rows as f32, -a.sin())
    };
    let u = |i: u32| 0.02 + 0.96 * i as f32 / seg as f32;
    let side_v = |j: u32| 0.98 - 0.48 * j as f32 / rows as f32;
    let dome_v = |k: u32| 0.48 - 0.46 * k as f32 / rings as f32;
    let mut triangles = Vec::new();
    for j in 0..rows {
        for i in 0..seg {
            push_quad(
                &mut triangles,
                [side(i, j), side(i + 1, j), side(i + 1, j + 1), side(i, j + 1)],
                [Vec2::new(u(i), side_v(j)), Vec2::new(u(i + 1), side_v(j)), Vec2::new(u(i + 1), side_v(j + 1)), Vec2::new(u(i), side_v(j + 1))],
                None,
                0,
            );
        }
    }
    for k in 0..rings - 1 {
        for i in 0..seg {
            push_quad(
                &mut triangles,
                [ring(i, k), ring(i + 1, k), ring(i + 1, k + 1), ring(i, k + 1)],
                [Vec2::new(u(i), dome_v(k)), Vec2::new(u(i + 1), dome_v(k)), Vec2::new(u(i + 1), dome_v(k + 1)), Vec2::new(u(i), dome_v(k + 1))],
                None,
                0,
            );
        }
    }
    let pole = Vec3::new(0.0, 2.0, 0.0);
    for i in 0..seg {
        let k = rings - 1;
        triangles.push(RawTriangle {
            positions: [ring(i, k), ring(i + 1, k), pole],
            normals: None,
            uvs: Some([Vec2::new(u(i), dome_v(k)), Vec2::new(u(i + 1), dome_v(k)), Vec2::new(0.5 * (u(i) + u(i + 1)), 0.02)]),
            material: 0,
        });
    }
    RawMesh { triangles, materials: vec![material("Capped", [0.8, 0.8, 0.8, 1.0], Some(0))] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::TriFlags;
    use crate::testutil::{hash_pixels, mesh_of};

    fn geo_normal(t: &RawTriangle) -> Vec3 {
        (t.positions[1] - t.positions[0]).cross(t.positions[2] - t.positions[0]).normalize()
    }

    fn uv_signed_area(t: &RawTriangle) -> f32 {
        let uv = t.uvs.unwrap();
        0.5 * (uv[1] - uv[0]).perp_dot(uv[2] - uv[0])
    }

    #[test]
    fn sphere_is_closed_outward_and_equirect() {
        let (seg, rings, r) = (16u32, 8u32, 2.0f32);
        let raw = uv_sphere(seg, rings, r);
        assert_eq!(raw.triangles.len() as u32, seg * (2 * rings - 2));
        assert_eq!(raw.materials.len(), 1);
        assert_eq!(raw.materials[0].texture, Some(0));
        for t in &raw.triangles {
            let centre = (t.positions[0] + t.positions[1] + t.positions[2]) / 3.0;
            assert!(geo_normal(t).dot(centre.normalize()) > 0.9, "outward winding");
            // Front faces show the texture unmirrored: negative signed area in v-down UV space.
            assert!(uv_signed_area(t) < 0.0);
            let (n, uv) = (t.normals.unwrap(), t.uvs.unwrap());
            for k in 0..3 {
                let p = t.positions[k];
                assert!((p.length() - r).abs() < 1e-5);
                assert!((n[k] - p / r).length() < 1e-5);
                assert!((0.0..=1.0).contains(&uv[k].x) && (0.0..=1.0).contains(&uv[k].y));
                // The equirect formula of the doc comment (poles and the seam column excepted).
                let d = p / r;
                let v = d.y.clamp(-1.0, 1.0).acos() / PI;
                assert!((uv[k].y - v).abs() < 1e-5);
                if d.y.abs() < 0.999 && uv[k].x > 0.0 && uv[k].x < 1.0 {
                    let u = (-d.x).atan2(-d.z).rem_euclid(TAU) / TAU;
                    assert!((uv[k].x - u).abs() < 1e-5, "u {} vs {}", uv[k].x, u);
                }
            }
        }
        // u = 0.5 faces +Z and u grows towards +X there.
        let m = mesh_of(raw);
        let front = m.uvs.iter().position(|uv| (uv.x - 0.5).abs() < 1e-6 && (uv.y - 0.5).abs() < 1e-6).unwrap();
        assert!((m.positions[front] - Vec3::new(0.0, 0.0, r)).length() < 1e-5);
        let right = m.uvs.iter().position(|uv| (uv.x - 0.75).abs() < 1e-6 && (uv.y - 0.5).abs() < 1e-6).unwrap();
        assert!((m.positions[right] - Vec3::new(r, 0.0, 0.0)).length() < 1e-5);
        // Duplicated seam column: (rings - 1) ring positions appear with u = 0 and with u = 1.
        let at_u = |u: f32| m.uvs.iter().filter(|uv| uv.x == u).count();
        assert_eq!(at_u(0.0), (rings - 1) as usize);
        assert_eq!(at_u(1.0), (rings - 1) as usize);
        // Minimum tessellation is enforced.
        assert_eq!(uv_sphere(0, 0, 1.0).triangles.len(), 3 * 2);
    }

    #[test]
    fn cube_has_six_upright_islands() {
        let raw = seam_cube(2.0);
        assert_eq!(raw.triangles.len(), 12);
        let mesh = mesh_of(raw.clone());
        assert_eq!(mesh.positions.len(), 24);
        assert_eq!(mesh.tri_count(), 12);
        assert!(mesh.tri_flags.iter().all(|f| f.is_empty()));
        for (i, t) in raw.triangles.iter().enumerate() {
            let centre = (t.positions[0] + t.positions[1] + t.positions[2]) / 3.0;
            let n = geo_normal(t);
            assert!(n.dot(centre) > 0.0, "outward winding");
            assert_eq!(t.normals.unwrap(), [n; 3]);
            assert!(uv_signed_area(t) < 0.0, "unmirrored");
            let [u0, u1, v0, v1] = seam_cube_island(i as u32 / 2);
            for (p, uv) in t.positions.iter().zip(t.uvs.unwrap()) {
                assert!(p.abs().max_element() == 1.0);
                assert!(uv.x >= u0 && uv.x <= u1 && uv.y >= v0 && uv.y <= v1);
            }
        }
        // Islands leave 1/32 to their cell border, hence 1/16 between neighbours.
        let a = seam_cube_island(0);
        let b = seam_cube_island(1);
        let d = seam_cube_island(3);
        assert!((a[0] - 1.0 / 32.0).abs() < 1e-7 && (a[2] - 1.0 / 32.0).abs() < 1e-7);
        assert!((b[0] - a[1] - 1.0 / 16.0).abs() < 1e-6);
        assert!((d[2] - a[3] - 1.0 / 16.0).abs() < 1e-6);
        assert!((seam_cube_island(5)[1] - (1.0 - 1.0 / 32.0)).abs() < 1e-6);
        // "Up" in the island is +Y on the side faces: the top-left UV corner of +Z is the vertex (-1, 1, 1).
        let front = &raw.triangles[9]; // second triangle of +Z: (BL, TR, TL)
        let [u0, _, v0, _] = seam_cube_island(4);
        assert_eq!(front.positions[2], Vec3::new(-1.0, 1.0, 1.0));
        assert_eq!(front.uvs.unwrap()[2], Vec2::new(u0, v0));
    }

    #[test]
    fn two_material_model_has_two_sets() {
        let raw = two_material_model();
        assert_eq!(raw.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["Body", "Head"]);
        assert_eq!(raw.materials.iter().map(|m| m.texture).collect::<Vec<_>>(), vec![Some(0), Some(1)]);
        assert_eq!(raw.triangles.iter().filter(|t| t.material == 0).count(), 12);
        assert_eq!(raw.triangles.iter().filter(|t| t.material == 1).count(), 24 * 22);
        let mesh = mesh_of(raw);
        assert_eq!(mesh.sets.len(), 2);
        assert_eq!(mesh.sets[0].name, "Body");
        assert_eq!(mesh.sets[1].name, "Head");
        // The head sits on top of the body.
        assert!((mesh.bounds.max.y - 1.15).abs() < 1e-5 && (mesh.bounds.min.y + 0.5).abs() < 1e-6);
    }

    #[test]
    fn small_fixtures_match_their_descriptions() {
        // flat_quad: identity UVs.
        let quad = flat_quad();
        for t in &quad.triangles {
            assert_eq!(geo_normal(t), Vec3::Z);
            for (p, uv) in t.positions.iter().zip(t.uvs.unwrap()) {
                assert_eq!((uv.x, uv.y), (p.x, 1.0 - p.y));
                assert_eq!(p.z, 0.0);
            }
        }

        // mirrored_quads: same island, opposite UV orientation, UVs agree on the shared edge x = 0.
        let m = mirrored_quads();
        assert!(uv_signed_area(&m.triangles[0]) < 0.0 && uv_signed_area(&m.triangles[1]) < 0.0);
        assert!(uv_signed_area(&m.triangles[2]) > 0.0 && uv_signed_area(&m.triangles[3]) > 0.0);
        for t in &m.triangles {
            assert_eq!(geo_normal(t), Vec3::Z);
            for (p, uv) in t.positions.iter().zip(t.uvs.unwrap()) {
                assert!((uv.x - (0.75 - 0.5 * p.x.abs())).abs() < 1e-6);
                assert!((uv.y - (0.75 - 0.5 * (p.y + 0.5))).abs() < 1e-6);
            }
        }
        let mesh = mesh_of(m);
        assert_eq!(mesh.tri_flags[0], TriFlags::empty());
        assert_eq!(mesh.tri_flags[2], TriFlags::UV_MIRRORED);

        // wrapped_quad: both triangles of quad A wrap, quad B folds onto 0.25..0.75.
        let (mesh, report) = crate::mesh_build::normalize(wrapped_quad(), &Default::default());
        assert_eq!(report.wrapped_tris, 4);
        assert!(mesh.tri_flags[0].contains(TriFlags::UV_WRAPS) && mesh.tri_flags[1].contains(TriFlags::UV_WRAPS));
        assert!(!mesh.tri_flags[2].contains(TriFlags::UV_WRAPS) && !mesh.tri_flags[3].contains(TriFlags::UV_WRAPS));
        for t in 2..4 {
            assert!(mesh.tri_uvs(t).iter().all(|uv| uv.x >= 0.25 && uv.x <= 0.75 && uv.y >= 0.25 && uv.y <= 0.75));
        }

        // no_uv_tetra: closed, outward, no UVs, no normals.
        let tetra = no_uv_tetra();
        assert_eq!(tetra.triangles.len(), 4);
        for t in &tetra.triangles {
            assert!(t.uvs.is_none() && t.normals.is_none());
            let centre = (t.positions[0] + t.positions[1] + t.positions[2]) / 3.0;
            assert!(geo_normal(t).dot(centre) > 0.0);
        }

        // tight_islands: at 256 px the islands leave exactly the texel columns 127 and 128 free.
        let tight = tight_islands();
        let max_a = tight.triangles[..2].iter().flat_map(|t| t.uvs.unwrap()).fold(0.0f32, |m, uv| m.max(uv.x));
        let min_b = tight.triangles[2..].iter().flat_map(|t| t.uvs.unwrap()).fold(1.0f32, |m, uv| m.min(uv.x));
        assert!(126.5 < max_a * 256.0 && max_a * 256.0 < 127.5);
        assert!(128.5 < min_b * 256.0 && min_b * 256.0 < 129.5);

        // occluded_quads: the rear quad is larger, 0.5 behind, and its island is disjoint from the front one.
        let occ = occluded_quads();
        assert!(occ.triangles[..2].iter().all(|t| t.positions.iter().all(|p| p.z == 0.0)));
        assert!(occ.triangles[2..].iter().all(|t| t.positions.iter().all(|p| p.z == -0.5)));
        assert!(occ.triangles.iter().all(|t| geo_normal(t) == Vec3::Z));
        assert!(occ.triangles[..2].iter().flat_map(|t| t.uvs.unwrap()).all(|uv| uv.x <= 0.45 && uv.y <= 0.45));
        assert!(occ.triangles[2..].iter().flat_map(|t| t.uvs.unwrap()).all(|uv| uv.x >= 0.55 && uv.y >= 0.55));
        for (quad_tri, front_tri) in flat_quad().triangles.iter().zip(&occ.triangles) {
            assert_eq!(quad_tri.positions, front_tri.positions);
        }
    }

    #[test]
    fn reverse_winding_flips_the_face_only() {
        // A mesh with explicit per-vertex normals: they are reordered with their vertices, not negated.
        let mut raw = uv_sphere(8, 4, 1.0);
        let before = raw.clone();
        reverse_winding(&mut raw);
        for (a, b) in before.triangles.iter().zip(&raw.triangles) {
            assert!((geo_normal(b) + geo_normal(a)).length() < 1e-6);
            assert_eq!((a.positions[0], a.positions[1], a.positions[2]), (b.positions[0], b.positions[2], b.positions[1]));
            let (ua, ub) = (a.uvs.unwrap(), b.uvs.unwrap());
            assert_eq!((ua[0], ua[1], ua[2]), (ub[0], ub[2], ub[1]));
            let (na, nb) = (a.normals.unwrap(), b.normals.unwrap());
            assert_eq!((na[0], na[1], na[2]), (nb[0], nb[2], nb[1]));
            assert_eq!(a.material, b.material);
        }
        reverse_winding(&mut raw);
        for (a, b) in before.triangles.iter().zip(&raw.triangles) {
            assert_eq!(a.positions, b.positions);
        }
        // The flat fixtures have no explicit normals, so their shading normals follow the new winding.
        let mut quad = flat_quad();
        assert!(quad.triangles.iter().all(|t| t.normals.is_none()));
        assert!(mesh_of(quad.clone()).normals.iter().all(|n| *n == Vec3::Z));
        reverse_winding(&mut quad);
        let reversed = mesh_of(quad);
        assert!(reversed.normals.iter().all(|n| *n == Vec3::NEG_Z));
        assert_eq!(reversed.geo_normal(0), Vec3::NEG_Z);
        let mut tetra = no_uv_tetra();
        reverse_winding(&mut tetra);
        assert!(tetra.triangles.iter().all(|t| t.uvs.is_none() && t.normals.is_none()));
    }

    #[test]
    fn checker_is_opaque_with_distinct_sides() {
        let img = checker_texture(128, 64, 8);
        assert!(img.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        let (top, bottom, left, right) = (img.get(64, 0), img.get(64, 63), img.get(0, 32), img.get(127, 32));
        let all = [top, bottom, left, right];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // Two tones alternate from cell to cell.
        let a = img.get(8, 4);
        let b = img.get(24, 4);
        assert_ne!(a, b);
        assert_eq!(img.get(40, 4), a);
        assert_eq!(img.get(8, 12), b);
        assert!(!all.contains(&a) && !all.contains(&b));
        assert_eq!(hash_pixels(&img), hash_pixels(&checker_texture(128, 64, 8)));
        // Degenerate sizes do not panic.
        assert_eq!(checker_texture(0, 5, 4).byte_len(), 0);
        assert_eq!(checker_texture(1, 1, 0).data, [228, 70, 58, 255]);
        assert_eq!(checker_texture(3, 2, 100).byte_len(), 24);
    }

    #[test]
    fn stamp_image_is_soft_and_asymmetric() {
        let size = 64u32;
        let img = sample_stamp_image(size);
        assert_eq!((img.width, img.height), (size, size));
        assert_eq!(img.get(0, 0), [0; 4], "transparent corner");
        assert!(img.data.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
        assert!(img.data.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[3] < 255), "soft alpha");
        // Blue ring on the left and right, red arrow head above the centre, yellow dot right of the shaft.
        let ring = img.get(9, 32);
        assert!(ring[2] > 180 && ring[0] < 90 && ring[3] > 200, "{ring:?}");
        let head = img.get(32, 20);
        assert!(head[0] > 200 && head[2] < 90 && head[3] == 255, "{head:?}");
        let dot = img.get(45, 40);
        assert!(dot[0] > 220 && dot[1] > 170 && dot[2] < 90, "{dot:?}");
        // Not symmetric under either flip.
        let flipped = |fx: bool, fy: bool| {
            let mut out = PixelBuf::new(size, size);
            for y in 0..size {
                for x in 0..size {
                    out.set(if fx { size - 1 - x } else { x }, if fy { size - 1 - y } else { y }, img.get(x, y));
                }
            }
            out
        };
        assert_ne!(flipped(true, false), img);
        assert_ne!(flipped(false, true), img);
        assert_ne!(flipped(true, true), img);
        assert_eq!(hash_pixels(&img), hash_pixels(&sample_stamp_image(size)));
        assert_eq!(sample_stamp_image(0).byte_len(), 0);
        assert_eq!(sample_stamp_image(1).byte_len(), 4);
    }

    /// Minimal OBJ reader for the test: returns per face corner (position, uv, normal).
    type Corner = (Vec3, Option<Vec2>, Option<Vec3>);
    fn parse_obj(text: &str) -> (Vec<[Corner; 3]>, Vec<String>, Option<String>) {
        let (mut v, mut vt, mut vn) = (Vec::new(), Vec::new(), Vec::new());
        let (mut faces, mut usemtl, mut mtllib) = (Vec::new(), Vec::new(), None);
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let nums = |it: std::str::SplitWhitespace<'_>| it.map(|s| s.parse::<f32>().unwrap()).collect::<Vec<_>>();
            match it.next() {
                Some("v") => v.push(Vec3::from_slice(&nums(it))),
                Some("vt") => vt.push(Vec2::from_slice(&nums(it))),
                Some("vn") => vn.push(Vec3::from_slice(&nums(it))),
                Some("usemtl") => usemtl.push(it.next().unwrap().to_owned()),
                Some("mtllib") => mtllib = Some(it.next().unwrap().to_owned()),
                Some("f") => {
                    let corners: Vec<Corner> = it
                        .map(|c| {
                            let mut parts = c.split('/');
                            let idx = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(|s| s.parse::<usize>().unwrap() - 1);
                            let p = v[idx(parts.next()).unwrap()];
                            let t = idx(parts.next()).map(|i| vt[i]);
                            let n = idx(parts.next()).map(|i| vn[i]);
                            (p, t, n)
                        })
                        .collect();
                    faces.push([corners[0], corners[1], corners[2]]);
                }
                _ => {}
            }
        }
        (faces, usemtl, mtllib)
    }

    #[test]
    fn to_obj_round_trips() {
        let raw = two_material_model();
        let (obj, mtl) = to_obj(&raw, "model.mtl", &["body.png".to_owned(), "head.png".to_owned()]);
        let (faces, usemtl, mtllib) = parse_obj(&obj);
        assert_eq!(mtllib.as_deref(), Some("model.mtl"));
        assert_eq!(usemtl, vec!["Body", "Head"]);
        assert_eq!(faces.len(), raw.triangles.len());
        for (face, t) in faces.iter().zip(&raw.triangles) {
            for (k, &(p, uv, n)) in face.iter().enumerate() {
                assert_eq!(p, t.positions[k]);
                assert_eq!(n, Some(t.normals.unwrap()[k]));
                let uv = uv.unwrap();
                assert_eq!(uv.x, t.uvs.unwrap()[k].x);
                assert!((1.0 - uv.y - t.uvs.unwrap()[k].y).abs() < 1e-6, "V is flipped");
            }
        }
        // Shared values are written once: 24 cube corners use 8 positions.
        assert_eq!(to_obj(&seam_cube(1.0), "", &[]).0.lines().filter(|l| l.starts_with("v ")).count(), 8);

        let lines: Vec<&str> = mtl.lines().collect();
        let body = lines.iter().position(|l| *l == "newmtl Body").unwrap();
        let head = lines.iter().position(|l| *l == "newmtl Head").unwrap();
        assert!(body < head);
        assert!(lines[body..head].contains(&"map_Kd body.png"));
        assert!(lines[head..].contains(&"map_Kd head.png"));
        assert!(lines[body..head].iter().any(|l| l.starts_with("Kd 0.75 0.75 0.78")));
        assert!(lines[body..head].contains(&"d 1"));
    }

    #[test]
    fn to_obj_handles_missing_attributes_and_names() {
        // Mixed faces: with and without UVs / normals.
        let mut raw = flat_quad();
        for t in &mut raw.triangles {
            t.normals = Some([Vec3::Z; 3]);
        }
        raw.triangles[1].uvs = None;
        raw.triangles.extend(no_uv_tetra().triangles.into_iter().map(|mut t| {
            t.material = 7; // no such material
            t
        }));
        raw.materials[0].name = "  my material ".to_owned();
        raw.materials[0].texture = Some(3);
        let (obj, mtl) = to_obj(&raw, "", &["a.png".to_owned()]);
        assert!(!obj.contains("mtllib"));
        let (faces, usemtl, _) = parse_obj(&obj);
        assert_eq!(usemtl, vec!["my_material", "material_7"]);
        assert!(faces[0].iter().all(|c| c.1.is_some() && c.2.is_some()));
        assert!(faces[1].iter().all(|c| c.1.is_none() && c.2.is_some()));
        assert!(faces[2..].iter().all(|f| f.iter().all(|c| c.1.is_none() && c.2.is_none())));
        let face_lines: Vec<&str> = obj.lines().filter(|l| l.starts_with("f ")).collect();
        assert!(face_lines[1].contains("//"));
        assert!(!face_lines[2].contains('/'));
        // The texture index has no name: no map_Kd line.
        assert!(mtl.contains("newmtl my_material"));
        assert!(!mtl.contains("map_Kd"));
        // An unnamed material gets a generated name in both files.
        raw.materials[0].name.clear();
        let (obj, mtl) = to_obj(&raw, "x.mtl", &[]);
        assert!(obj.contains("usemtl material_0") && mtl.contains("newmtl material_0"));
        // An empty mesh still gives valid text.
        let (obj, mtl) = to_obj(&RawMesh::default(), "x.mtl", &[]);
        assert!(obj.contains("mtllib x.mtl") && !obj.contains("\nf "));
        assert!(!mtl.contains("newmtl"));
    }

    // ----- round 4: the generators of FEATURE_BRUSH 8 and the file writers -----

    /// Six times the signed volume enclosed by the triangles (positive for a closed mesh wound outward).
    fn signed_volume6(raw: &RawMesh) -> f32 {
        raw.triangles.iter().map(|t| t.positions[0].dot(t.positions[1].cross(t.positions[2]))).sum()
    }

    /// Edges used by exactly one triangle (the openings of the mesh), with positions compared bit for bit.
    fn open_edges(raw: &RawMesh) -> usize {
        let key = |p: Vec3| p.to_array().map(|c| (c + 0.0).to_bits());
        let mut count: HashMap<([u32; 3], [u32; 3]), i32> = HashMap::new();
        for t in &raw.triangles {
            for k in 0..3 {
                let (a, b) = (key(t.positions[k]), key(t.positions[(k + 1) % 3]));
                *count.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
            }
        }
        count.values().filter(|&&c| c == 1).count()
    }

    #[test]
    fn brush_fixtures_match_their_descriptions() {
        // Closed solids wound outward, with their volumes.
        for (raw, volume) in [
            (thin_slab(0.05), 0.05),
            (two_blocks(0.01), 2.0),
            (overlapping_cubes(), 2.0),
            (doubled_cube(), 2.0),
            (corner_cubes(), 2.0),
            (plate_xy(), 0.2),
            (coin(150.0, 1.0), std::f32::consts::PI * 75.0 * 75.0),
        ] {
            assert_eq!(open_edges(&raw), 0);
            let v = signed_volume6(&raw) / 6.0;
            assert!((v - volume).abs() < 0.01 * volume, "{} {v} vs {volume}", raw.materials[0].name);
        }
        assert!((signed_volume6(&coin(2.0, 1.0)) / 6.0 - std::f32::consts::PI).abs() < 0.01);
        // Open shapes: one opening each.
        assert_eq!(open_edges(&open_box(1.0)), 4);
        assert_eq!(open_box(1.0).triangles.len(), 10);
        assert_eq!(open_edges(&cup()), 64);
        assert!(cup().triangles.iter().all(|t| t.positions.iter().all(|p| p.y >= 0.0 && p.y <= 1.0)));
        // The saddle cup: one opening of 64 edges whose rim points lie in no plane (heights 0.75 to 1.25).
        let saddle = saddle_cup();
        assert_eq!(open_edges(&saddle), 64);
        let mut rim: Vec<Vec3> = Vec::new();
        for p in saddle.triangles.iter().flat_map(|t| t.positions).filter(|p| p.y > 0.5) {
            if !rim.contains(&p) {
                rim.push(p);
            }
        }
        assert_eq!(rim.len(), 64);
        let (lo, hi) = rim.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
        assert!((lo - 0.75).abs() < 1e-5 && (hi - 1.25).abs() < 1e-5, "{lo} {hi}");
        let normal = (rim[1] - rim[0]).cross(rim[2] - rim[0]).normalize();
        assert!(rim.iter().any(|p| (*p - rim[0]).dot(normal).abs() > 0.1), "the rim is planar");
        assert!(saddle.triangles.iter().all(|t| t.uvs.is_none() && t.normals.is_none()));
        assert_eq!(open_edges(&cube_on_floor()), 4);
        assert_eq!(open_edges(&cube_floor_rod()), 4);
        assert_eq!(cube_with_inner_wall(false).triangles.len(), 14);
        let slanted = cube_with_inner_wall(true);
        assert!(slanted.triangles[12..].iter().flat_map(|t| t.positions).any(|p| p.x.abs() > 0.05));
        // Bake fixtures have neither UVs nor normals.
        for raw in [open_box(1.0), overlapping_cubes(), doubled_cube(), tilted_quad(), cube_on_floor(), corner_cubes(), flat_star(), plate_xy(), cup()] {
            assert!(raw.triangles.iter().all(|t| t.uvs.is_none() && t.normals.is_none()));
        }
        // The tilted quad lies in none of the main planes; the star lies in z = 0 facing +Z.
        let n = geo_normal(&tilted_quad().triangles[0]);
        assert!(n.x.abs() > 0.1 && n.y.abs() > 0.1 && n.z.abs() > 0.1, "{n:?}");
        let star = flat_star();
        assert_eq!(star.triangles.len(), 10);
        assert!(star.triangles.iter().all(|t| (geo_normal(t) - Vec3::Z).length() < 1e-6 && t.positions.iter().all(|p| p.z == 0.0)));

        // The paint fixtures: UVs everywhere, unmirrored islands on the front faces.
        for raw in [thin_slab(0.05), fold(90.0), two_blocks(0.01), fan_strips(), undercut_ridge(), t_junction_quads(), cracked_quads(1e-6), flat_quad_tilted(30.0)] {
            for t in &raw.triangles {
                assert!(uv_signed_area(t) < 0.0, "{}", raw.materials[0].name);
            }
            let mesh = mesh_of(raw);
            assert!(mesh.sets.len() == 1 && mesh.tri_flags.iter().all(|f| !f.contains(TriFlags::UV_MIRRORED)));
        }
        // fold: the angle between the two front faces.
        for angle in [60.0f32, 90.0, 180.0] {
            let raw = fold(angle);
            let (a, b) = (geo_normal(&raw.triangles[0]), geo_normal(&raw.triangles[2]));
            assert!((a.dot(b) - (180.0 - angle).to_radians().cos()).abs() < 1e-5, "{angle}");
        }
        // The undercut wall leans 110 degrees away from the plate normal.
        let ridge = undercut_ridge();
        assert!((geo_normal(&ridge.triangles[2]).dot(Vec3::Z) - 110f32.to_radians().cos()).abs() < 1e-5);
        assert_eq!(geo_normal(&ridge.triangles[4]), Vec3::Z);
        // fan_strips share exactly one corner; cracked quads share none.
        let fan = fan_strips();
        let corners = |t: &RawTriangle| t.positions.map(|p| p.to_array().map(f32::to_bits));
        let shared = corners(&fan.triangles[0]).iter().filter(|c| corners(&fan.triangles[1]).contains(c)).count();
        assert_eq!(shared, 1);
        // The long edge of A is not split: it and the two short edges of B and C are open (a T-junction).
        assert_eq!(open_edges(&t_junction_quads()), 10);
        // Islands of two_blocks are disjoint.
        for i in 0..12 {
            for j in i + 1..12 {
                let (a, b) = (two_blocks_island(i), two_blocks_island(j));
                assert!(a[1] <= b[0] || b[1] <= a[0] || a[3] <= b[2] || b[3] <= a[2]);
            }
        }
        // flat_quad_tilted keeps the UVs and turns the plane.
        let tilted = flat_quad_tilted(30.0);
        assert_eq!(tilted.triangles[0].uvs, flat_quad().triangles[0].uvs);
        assert!((geo_normal(&tilted.triangles[0]).dot(Vec3::Z) - 30f32.to_radians().cos()).abs() < 1e-5);
        // soup_sets: deterministic, spread over its sets, one UV cell per triangle.
        let soup = soup_sets(1, 2000, 4);
        assert_eq!(soup.triangles.len(), 2000);
        assert_eq!(soup.materials.len(), 4);
        let positions = |raw: &RawMesh| raw.triangles.iter().map(|t| (t.positions, t.uvs, t.material)).collect::<Vec<_>>();
        assert_eq!(positions(&soup), positions(&soup_sets(1, 2000, 4)));
        assert_ne!(positions(&soup), positions(&soup_sets(2, 2000, 4)));
        let mesh = mesh_of(soup);
        assert_eq!(mesh.sets.len(), 4);
    }

    /// The triangles of an imported model as sorted centroid keys (to compare with what was written).
    fn centroids(mesh: &crate::mesh::MeshData) -> Vec<[i64; 3]> {
        let mut out: Vec<[i64; 3]> = (0..mesh.tri_count())
            .map(|t| {
                let [a, b, c] = mesh.tri_positions(t);
                ((a + b + c) / 3.0).to_array().map(|v| (v * 1e4).round() as i64)
            })
            .collect();
        out.sort();
        out
    }

    fn expected_centroids(raw: &RawMesh, m: glam::Mat4) -> Vec<[i64; 3]> {
        let mut out: Vec<[i64; 3]> = raw
            .triangles
            .iter()
            .map(|t| {
                let c = (t.positions[0] + t.positions[1] + t.positions[2]) / 3.0;
                m.transform_point3(c).to_array().map(|v| (v * 1e4).round() as i64)
            })
            .collect();
        out.sort();
        out
    }

    fn import(bytes: Vec<u8>, name: &str, format: crate::import::ModelFormat) -> crate::import::ImportedModel {
        use crate::import::{SourceBundle, SourceFile, import_from_bundle, normalize_ref};
        let main = SourceFile { key: normalize_ref(name), original_name: name.to_owned(), bytes: bytes.into() };
        let bundle = SourceBundle { format, main, sidecars: Vec::new(), extras: Vec::new(), original_path: None };
        import_from_bundle(&bundle, &Default::default(), false, &crate::progress::NoProgress).unwrap()
    }

    fn close(a: &[[i64; 3]], b: &[[i64; 3]]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (0..3).all(|k| (x[k] - y[k]).abs() <= 2))
    }

    #[test]
    fn gltf_and_glb_writers_import_back() {
        use crate::import::{ModelFormat, sniff_model_format};
        use glam::{Mat4, Quat};
        let cube = seam_cube(1.0);
        let scaled = Mat4::from_scale(Vec3::new(1.5, 1.0, 1.0));
        let text = to_gltf(&cube, scaled);
        assert_eq!(sniff_model_format(text.as_bytes(), "cube.gltf"), Some(ModelFormat::Gltf));
        let model = import(text.into_bytes(), "cube.gltf", ModelFormat::Gltf);
        assert_eq!(model.mesh.tri_count(), 12);
        assert!(close(&centroids(&model.mesh), &expected_centroids(&cube, scaled)));
        // UVs were written: the cube keeps its one textured set and is paintable.
        assert_eq!(model.mesh.sets.len(), 1);

        // A GLB with two meshes under different nodes, one of them without UVs.
        let turned = Mat4::from_rotation_translation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::new(0.5, 0.0, 0.0));
        let parts = vec![(seam_cube(1.0), Mat4::IDENTITY), (no_uv_tetra(), turned)];
        let glb = to_glb(&parts);
        assert_eq!(&glb[..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize, glb.len());
        assert_eq!(sniff_model_format(&glb, "shape.bin"), Some(ModelFormat::Glb));
        let model = import(glb, "shape.glb", ModelFormat::Glb);
        assert_eq!(model.mesh.tri_count(), 16);
        let mut expected = expected_centroids(&parts[0].0, parts[0].1);
        expected.extend(expected_centroids(&parts[1].0, parts[1].1));
        expected.sort();
        assert!(close(&centroids(&model.mesh), &expected));
    }

    #[test]
    fn fbx_writer_imports_back() {
        use crate::import::{ModelFormat, sniff_model_format};
        use glam::Mat4;
        let cube = seam_cube(1.0);
        for (unit, scaling) in [(1.0f32, [1.0f32, 1.0, 1.0]), (100.0, [1.0, 1.0, 1.0]), (1.0, [1.5, 1.0, 1.0])] {
            let text = to_fbx_ascii(&cube, unit, scaling);
            assert_eq!(sniff_model_format(text.as_bytes(), "cube.fbx"), Some(ModelFormat::Fbx));
            let model = import(text.into_bytes(), "cube.fbx", ModelFormat::Fbx);
            assert_eq!(model.mesh.tri_count(), 12, "{unit} {scaling:?}");
            // UnitScaleFactor is centimetres per file unit; Lcl Scaling scales the model.
            let m = Mat4::from_scale(Vec3::from_array(scaling) * unit * 0.01);
            assert!(close(&centroids(&model.mesh), &expected_centroids(&cube, m)), "{unit} {scaling:?}");
        }
        // A bake fixture without UVs imports too.
        let text = to_fbx_ascii(&open_box(1.0), 1.0, [1.0; 3]);
        assert_eq!(import(text.into_bytes(), "box.fbx", ModelFormat::Fbx).mesh.tri_count(), 10);
    }

    #[test]
    fn base64_is_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(crate::import::base64_decode(&base64(&data)).unwrap(), data);
    }
}

