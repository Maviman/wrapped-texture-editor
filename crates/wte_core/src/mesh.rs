//! Mesh data model (ARCHITECTURE.md 5.7): the normalized `MeshData`, its materials and texture-set descriptions, the
//! importer-side `RawMesh` triangle soup and the GPU-facing `RenderVertex` / `WireEdge` records.
//! The normalization pipeline and the derived-data builders live in `mesh_build.rs`.

use glam::{Vec2, Vec3};

use crate::ids::{TextureSetId, TriId};
use crate::math::Aabb;

bitflags::bitflags! {
    /// Per-triangle UV properties.
    #[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
    pub struct TriFlags: u8 {
        /// Missing UVs or zero UV area: never rasterized into a texture, still drawn and pickable.
        const NO_UV       = 1;
        /// Signed UV area has the opposite sign to the majority of its set.
        const UV_MIRRORED = 2;
        /// After tile normalization the triangle straddles a 0/1 border.
        const UV_WRAPS    = 4;
    }
}

/// One material of the model.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSlot {
    pub name: String,
    /// sRGB-encoded 0..1, used when the material has no texture set.
    pub base_color: [f32; 4],
    pub double_sided: bool,
    /// `None` => no UV-mapped geometry.
    pub texture_set: Option<TextureSetId>,
}

/// Description of one texture set (one paintable texture shared by one or more materials).
#[derive(Clone, Debug, PartialEq)]
pub struct SetInfo {
    /// Texture file stem, else first material name, else model name; unique (section 6.1 step 5).
    pub name: String,
    /// Material indices sharing this set.
    pub materials: Vec<u32>,
    /// Index into `ImportedModel::textures`.
    pub texture: Option<u32>,
    pub tri_count: u32,
}

/// UV coverage statistics of one texture set at one resolution.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UvStats {
    /// Texels owned by a triangle (gutter excluded).
    pub covered_texels: u64,
    /// Texels claimed by more than one triangle.
    pub overlapped_texels: u64,
    pub mirrored_tris: u32,
    /// `covered_texels / (w * h)`.
    pub coverage: f32,
    /// Mean world units per texel: `sqrt(total 3D area / total UV pixel area)`.
    pub texel_world: f32,
}

/// Normalized, immutable mesh. Unified index: one vertex per distinct (position, normal, uv, uv-tile) combination.
#[derive(Clone, Debug)]
pub struct MeshData {
    pub positions: Vec<Vec3>,
    /// Unit length.
    pub normals: Vec<Vec3>,
    /// v-down, per-triangle tile-normalized (section 6.1).
    pub uvs: Vec<Vec2>,
    /// 3 per triangle, CCW = front; triangles sorted by (material, NO_UV last), stable.
    pub indices: Vec<u32>,
    pub tri_material: Vec<u32>,
    pub tri_flags: Vec<TriFlags>,
    pub materials: Vec<MaterialSlot>,
    /// Triangle range per material (same order as `materials`).
    pub material_ranges: Vec<std::ops::Range<u32>>,
    /// Leading part of `material_ranges[m]`: UV-mapped triangles (drawn textured).
    pub material_uv_ranges: Vec<std::ops::Range<u32>>,
    /// Trailing part of `material_ranges[m]`: NO_UV triangles (drawn with `base_color` only).
    pub material_nouv_ranges: Vec<std::ops::Range<u32>>,
    /// Index == `TextureSetId.0`.
    pub sets: Vec<SetInfo>,
    pub bounds: Aabb,
    /// `fnv1a64` over indices, uv bits and per-triangle set ids.
    pub uv_hash: u64,
}

impl MeshData {
    /// Number of triangles.
    pub fn tri_count(&self) -> u32 {
        (self.indices.len() / 3) as u32
    }

    /// The three vertex indices of triangle `t`.
    #[inline]
    pub fn tri(&self, t: TriId) -> [u32; 3] {
        let i = t as usize * 3;
        [self.indices[i], self.indices[i + 1], self.indices[i + 2]]
    }

    /// The three corner positions of triangle `t`.
    #[inline]
    pub fn tri_positions(&self, t: TriId) -> [Vec3; 3] {
        let [a, b, c] = self.tri(t);
        [self.positions[a as usize], self.positions[b as usize], self.positions[c as usize]]
    }

    /// The three corner UVs of triangle `t`.
    #[inline]
    pub fn tri_uvs(&self, t: TriId) -> [Vec2; 3] {
        let [a, b, c] = self.tri(t);
        [self.uvs[a as usize], self.uvs[b as usize], self.uvs[c as usize]]
    }

    /// The three corner shading normals of triangle `t`.
    #[inline]
    pub fn tri_normals(&self, t: TriId) -> [Vec3; 3] {
        let [a, b, c] = self.tri(t);
        [self.normals[a as usize], self.normals[b as usize], self.normals[c as usize]]
    }

    /// The texture set triangle `t` paints into; `None` for NO_UV triangles and set-less materials.
    #[inline]
    pub fn tri_set(&self, t: TriId) -> Option<TextureSetId> {
        if self.tri_flags[t as usize].contains(TriFlags::NO_UV) {
            return None;
        }
        self.materials.get(self.tri_material[t as usize] as usize)?.texture_set
    }

    /// Unit geometric normal of triangle `t` from its winding; `Vec3::Y` for a degenerate triangle.
    #[inline]
    pub fn geo_normal(&self, t: TriId) -> Vec3 {
        let [a, b, c] = self.tri_positions(t);
        let n = (b - a).cross(c - a).normalize_or_zero();
        if n == Vec3::ZERO { Vec3::Y } else { n }
    }

    /// `bounds.diagonal().max(1e-12)`: the reference length of every epsilon.
    pub fn diag(&self) -> f32 {
        self.bounds.diagonal().max(1e-12)
    }
}

/// GPU vertex (32 bytes). Produced by the core so the app needs no knowledge of the mesh layout.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RenderVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// Classification of a wireframe edge.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum EdgeKind {
    Interior,
    Seam,
    Boundary,
}

/// One unique edge per UV side: a seam yields two entries (same positions, different UVs).
#[derive(Copy, Clone, Debug)]
pub struct WireEdge {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub uv_a: [f32; 2],
    pub uv_b: [f32; 2],
    pub kind: EdgeKind,
    pub set: Option<TextureSetId>,
}

/// Importer output before normalization: a triangle soup.
#[derive(Clone, Debug, Default)]
pub struct RawMesh {
    pub triangles: Vec<RawTriangle>,
    pub materials: Vec<RawMaterial>,
}

/// One triangle of a `RawMesh`.
#[derive(Clone, Debug)]
pub struct RawTriangle {
    pub positions: [Vec3; 3],
    pub normals: Option<[Vec3; 3]>,
    pub uvs: Option<[Vec2; 3]>,
    pub material: u32,
}

/// One material of a `RawMesh`.
#[derive(Clone, Debug)]
pub struct RawMaterial {
    pub name: String,
    pub base_color: [f32; 4],
    pub double_sided: bool,
    /// Index of the referenced texture (`ImportedModel::textures`), if any.
    pub texture: Option<u32>,
}

/// What `mesh_build::normalize` did to the raw mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshReport {
    pub total_tris: u32,
    pub dropped_degenerate: u32,
    pub no_uv_tris: u32,
    pub wrapped_tris: u32,
    pub normals_generated: bool,
    /// The whole model has no usable UVs.
    pub no_uvs: bool,
    pub fallback_uvs_generated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_mesh() -> MeshData {
        // Two triangles: a UV-mapped one in material 0 (set 0) and a NO_UV one in material 1 (no set).
        MeshData {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(2.0, 2.0, 2.0),
            ],
            normals: vec![Vec3::Z; 4],
            uvs: vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 0.0), Vec2::ZERO],
            indices: vec![0, 1, 2, 3, 3, 3],
            tri_material: vec![0, 1],
            tri_flags: vec![TriFlags::empty(), TriFlags::NO_UV],
            materials: vec![
                MaterialSlot { name: "a".into(), base_color: [1.0; 4], double_sided: false, texture_set: Some(TextureSetId(0)) },
                MaterialSlot { name: "b".into(), base_color: [1.0; 4], double_sided: false, texture_set: None },
            ],
            material_ranges: vec![0..1, 1..2],
            material_uv_ranges: vec![0..1, 1..1],
            material_nouv_ranges: vec![1..1, 1..2],
            sets: vec![SetInfo { name: "a".into(), materials: vec![0], texture: None, tri_count: 1 }],
            bounds: Aabb { min: Vec3::ZERO, max: Vec3::splat(2.0) },
            uv_hash: 0,
        }
    }

    #[test]
    fn accessors() {
        let m = tiny_mesh();
        assert_eq!(m.tri_count(), 2);
        assert_eq!(m.tri(0), [0, 1, 2]);
        assert_eq!(m.tri_positions(0)[1], Vec3::X);
        assert_eq!(m.tri_uvs(0)[2], Vec2::ZERO);
        assert_eq!(m.tri_normals(0), [Vec3::Z; 3]);
        assert_eq!(m.tri_set(0), Some(TextureSetId(0)));
        assert_eq!(m.tri_set(1), None);
        assert_eq!(m.geo_normal(0), Vec3::Z);
        assert_eq!(m.geo_normal(1), Vec3::Y, "degenerate triangle");
        assert!((m.diag() - 12f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn render_vertex_is_32_bytes() {
        assert_eq!(std::mem::size_of::<RenderVertex>(), 32);
        let v = RenderVertex { pos: [1.0, 2.0, 3.0], normal: [0.0, 1.0, 0.0], uv: [0.5, 0.25] };
        assert_eq!(bytemuck::bytes_of(&v).len(), 32);
        assert_eq!(TriFlags::default(), TriFlags::empty());
    }
}
