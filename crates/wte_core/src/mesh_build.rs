// Owner: core-geometry (C1). Mesh normalization (ARCHITECTURE.md 5.7, algorithm 6.1) and the data derived from a
// normalized mesh: GPU vertices, wireframe edges and the UV-layout thumbnail.

// Thresholds are written as `!(value >= limit)` on purpose: a NaN then takes the rejecting branch.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{DVec3, Vec2, Vec3};
use rayon::prelude::*;

use crate::ids::TextureSetId;
use crate::math::{Aabb, fnv1a64};
use crate::mesh::{
    EdgeKind, MaterialSlot, MeshData, MeshReport, ObjectInfo, RawMaterial, RawMesh, RawObjects, RawTriangle,
    RenderVertex, SetInfo, TriFlags, WireEdge,
};
use crate::pixel::PixelBuf;

/// Options of `normalize`.
#[derive(Clone, Debug, Default)]
pub struct NormalizeOptions {
    pub generate_fallback_uvs: bool,
    /// Names the fallback-UV set and sets without texture or material name; "" => "Texture".
    pub model_name: String,
    /// File stems, index = `RawMaterial::texture`; a missing index falls back to the material name.
    pub texture_names: Vec<String>,
}

/// Triangles with a UV area below this (in UV units) count as unmapped.
const MIN_UV_AREA: f64 = 1e-12;
/// A triangle is reported as "wrapped" when a tile was subtracted or its UVs leave 0..1 by more than this.
const WRAP_REPORT_TOLERANCE: f32 = 1e-4;
/// FNV-1a offset basis: the first seed of `uv_hash`.
const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// One kept triangle while it travels through the pipeline.
struct Work {
    pos: [Vec3; 3],
    normals: Option<[Vec3; 3]>,
    uv: [Vec2; 3],
    material: u32,
    flags: TriFlags,
    /// `(p1 - p0) x (p2 - p0)`: direction = face normal, length = twice the area.
    face: DVec3,
    /// Raw object index (step 9): `RawObjects::tri_object[i]`, 0 when missing or out of range.
    object: u32,
}

/// Result of the UV step for a triangle that has usable UVs.
struct TileUv {
    uv: [Vec2; 3],
    /// The UV bounding box leaves `[0, 1]^2` after tile normalization.
    wraps: bool,
    /// The source UVs were outside 0..1 in a way worth reporting (`MeshReport::wrapped_tris`).
    folded: bool,
}

/// Steps 2 and 3 of section 6.1 for one triangle: subtract the tile of the UV centroid; `None` when the triangle
/// has no usable UVs (missing, non-finite or zero UV area).
fn tile_normalize(uvs: Option<[Vec2; 3]>) -> Option<TileUv> {
    let src = uvs?;
    if !src.iter().all(|c| c.is_finite()) {
        return None;
    }
    let mut out = [Vec2::ZERO; 3];
    let mut tiled = false;
    for axis in 0..2 {
        let c = [src[0][axis] as f64, src[1][axis] as f64, src[2][axis] as f64];
        // `+ 0.0` turns -0.0 into +0.0 so that equal coordinates have equal bit patterns.
        let shifted = |tile: f64| c.map(|v| (v - tile) as f32 + 0.0);
        let centroid = |v: [f32; 3]| (v[0] as f64 + v[1] as f64 + v[2] as f64) / 3.0;
        let mut tile = ((c[0] + c[1] + c[2]) / 3.0).floor();
        let mut v = shifted(tile);
        // Rounding to f32 can push the centroid just across a tile border: correct once.
        let cen = centroid(v);
        if cen < 0.0 {
            tile -= 1.0;
            v = shifted(tile);
        } else if cen >= 1.0 {
            tile += 1.0;
            v = shifted(tile);
        }
        tiled |= tile != 0.0;
        for k in 0..3 {
            out[k][axis] = v[k];
        }
    }
    if !out.iter().all(|c| c.is_finite()) {
        return None;
    }
    let d = out.map(|c| c.as_dvec2());
    let area = 0.5 * (d[1] - d[0]).perp_dot(d[2] - d[0]).abs();
    if !(area >= MIN_UV_AREA) {
        return None;
    }
    let outside = |tol: f32| out.iter().any(|c| c.x < -tol || c.y < -tol || c.x > 1.0 + tol || c.y > 1.0 + tol);
    Some(TileUv { uv: out, wraps: outside(0.0), folded: tiled || outside(WRAP_REPORT_TOLERANCE) })
}

/// `v` normalized in double precision; `None` for zero-length or non-finite input.
fn unit(v: Vec3) -> Option<Vec3> {
    let d = v.as_dvec3();
    let len = d.length();
    if len.is_finite() && len > 0.0 { Some((d / len).as_vec3() + Vec3::ZERO) } else { None }
}

fn bits3(v: Vec3) -> [u32; 3] {
    [(v.x + 0.0).to_bits(), (v.y + 0.0).to_bits(), (v.z + 0.0).to_bits()]
}

/// Chains `fnv1a64` over the little-endian bytes of a stream of `u32`s.
fn hash_u32s(mut h: u64, values: impl Iterator<Item = u32>) -> u64 {
    let mut buf = [0u8; 4096];
    let mut len = 0;
    for v in values {
        buf[len..len + 4].copy_from_slice(&v.to_le_bytes());
        len += 4;
        if len == buf.len() {
            h = fnv1a64(h, &buf);
            len = 0;
        }
    }
    fnv1a64(h, &buf[..len])
}

/// Signed area of a triangle in v-down UV space. Negative = the texture appears unmirrored on the front face.
fn signed_uv_area(uv: [Vec2; 3]) -> f64 {
    let d = uv.map(|c| c.as_dvec2());
    0.5 * (d[1] - d[0]).perp_dot(d[2] - d[0])
}

/// Runs the pipeline of section 6.1. Deterministic for identical input.
///
/// The report describes the *source* mesh: `no_uv_tris` and `no_uvs` count triangles that arrived without usable UVs
/// even when fallback UVs were generated for them afterwards (then `fallback_uvs_generated` is set and no triangle
/// of the result carries `NO_UV`). `wrapped_tris` counts triangles whose UVs were folded onto the 0..1 tile: a
/// non-zero tile was subtracted, or the UVs leave 0..1 by more than 1e-4. A triangle whose material index has no
/// entry in `raw.materials` is assigned to one extra material named "default". The mesh has one object (step 9).
pub fn normalize(raw: RawMesh, opts: &NormalizeOptions) -> (MeshData, MeshReport) {
    normalize_with_objects(raw, &RawObjects::default(), opts)
}

/// `normalize` with object data (section 6.1 step 9): `objects.tri_object` runs parallel to `raw.triangles`. The object
/// data never changes the triangle order, the vertex order or `uv_hash`; it only fills `MeshData::tri_object` and
/// `MeshData::objects`. A missing entry or an index past `objects.names` counts as raw object 0.
pub fn normalize_with_objects(raw: RawMesh, objects: &RawObjects, opts: &NormalizeOptions) -> (MeshData, MeshReport) {
    let RawMesh { triangles: raw_tris, materials: mut raw_materials } = raw;
    let raw_object = |i: usize| -> u32 {
        match objects.tri_object.get(i) {
            Some(&o) if (o as usize) < objects.names.len() => o,
            _ => 0,
        }
    };
    let mut report = MeshReport { total_tris: raw_tris.len().min(u32::MAX as usize) as u32, ..MeshReport::default() };

    // Step 1: drop triangles with non-finite positions or (almost) no area.
    let finite = |t: &RawTriangle| t.positions.iter().all(|p| p.is_finite());
    let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
    for t in raw_tris.iter().filter(|t| finite(t)) {
        for p in t.positions {
            lo = lo.min(p.as_dvec3());
            hi = hi.max(p.as_dvec3());
        }
    }
    let diag_sq = if lo.x <= hi.x { (hi - lo).length_squared() } else { 0.0 };
    let min_area = 1e-12 * diag_sq;

    let declared_materials = raw_materials.len() as u32;
    let mut uses_default_material = false;
    let mut work: Vec<Work> = Vec::with_capacity(raw_tris.len());
    for (raw_index, t) in raw_tris.into_iter().enumerate() {
        if !finite(&t) {
            report.dropped_degenerate += 1;
            continue;
        }
        let pos = t.positions.map(|p| p + Vec3::ZERO); // -0.0 -> +0.0: equal positions get equal bit patterns
        let d = pos.map(|p| p.as_dvec3());
        let face = (d[1] - d[0]).cross(d[2] - d[0]);
        if !(0.5 * face.length() > min_area) {
            report.dropped_degenerate += 1;
            continue;
        }
        let material = if t.material < declared_materials {
            t.material
        } else {
            uses_default_material = true;
            declared_materials
        };
        // Steps 2 and 3: tile normalization, missing / degenerate UVs.
        let (uv, flags) = match tile_normalize(t.uvs) {
            Some(tile) => {
                report.wrapped_tris += tile.folded as u32;
                (tile.uv, if tile.wraps { TriFlags::UV_WRAPS } else { TriFlags::empty() })
            }
            None => {
                report.no_uv_tris += 1;
                ([Vec2::ZERO; 3], TriFlags::NO_UV)
            }
        };
        work.push(Work { pos, normals: t.normals, uv, material, flags, face, object: raw_object(raw_index) });
    }
    if uses_default_material {
        raw_materials.push(RawMaterial {
            name: "default".to_owned(),
            base_color: [0.8, 0.8, 0.8, 1.0],
            double_sided: false,
            texture: None,
        });
    }
    let material_count = raw_materials.len();
    report.no_uvs = !work.is_empty() && report.no_uv_tris as usize == work.len();

    // Step 4: fallback UVs when nothing is mapped: a generated, non-overlapping box-projection atlas.
    let fallback = opts.generate_fallback_uvs && report.no_uvs;
    if fallback {
        let soup: Vec<[Vec3; 3]> = work.iter().map(|w| w.pos).collect();
        for (w, uv) in work.iter_mut().zip(crate::import::generate_atlas_uvs(&soup)) {
            w.uv = uv;
            w.flags = TriFlags::empty();
        }
        report.fallback_uvs_generated = true;
    }

    // Step 5: texture sets.
    let mut uv_tris = vec![0u32; material_count];
    let mut nouv_tris = vec![0u32; material_count];
    for w in &work {
        if w.flags.contains(TriFlags::NO_UV) {
            nouv_tris[w.material as usize] += 1;
        } else {
            uv_tris[w.material as usize] += 1;
        }
    }
    let fallback_name = if opts.model_name.is_empty() { "Texture" } else { opts.model_name.as_str() };
    let mut sets: Vec<SetInfo> = Vec::new();
    let mut material_set: Vec<Option<TextureSetId>> = vec![None; material_count];
    if fallback {
        let materials: Vec<u32> = (0..material_count).filter(|&m| uv_tris[m] > 0).map(|m| m as u32).collect();
        for &m in &materials {
            material_set[m as usize] = Some(TextureSetId(0));
        }
        sets.push(SetInfo { name: fallback_name.to_owned(), materials, texture: None, tri_count: work.len() as u32 });
    } else {
        let mut by_texture: BTreeMap<u32, usize> = BTreeMap::new();
        for m in 0..material_count {
            if uv_tris[m] == 0 {
                continue;
            }
            let texture = raw_materials[m].texture;
            let new_set = |sets: &mut Vec<SetInfo>| {
                sets.push(SetInfo { name: String::new(), materials: Vec::new(), texture, tri_count: 0 });
                sets.len() - 1
            };
            let index = match texture {
                Some(t) => *by_texture.entry(t).or_insert_with(|| new_set(&mut sets)),
                None => new_set(&mut sets),
            };
            sets[index].materials.push(m as u32);
            sets[index].tri_count += uv_tris[m];
            material_set[m] = Some(TextureSetId(index as u32));
        }
        for i in 0..sets.len() {
            let texture_name =
                sets[i].texture.and_then(|t| opts.texture_names.get(t as usize)).filter(|n| !n.is_empty());
            let material_name = sets[i].materials.first().map(|&m| &raw_materials[m as usize].name);
            let base: &str = match (texture_name, material_name) {
                (Some(name), _) => name,
                (None, Some(name)) if !name.is_empty() => name,
                _ => fallback_name,
            };
            let mut name = base.to_owned();
            let mut suffix = 2;
            while sets[..i].iter().any(|s| s.name == name) {
                name = format!("{base}_{suffix}");
                suffix += 1;
            }
            sets[i].name = name;
        }
    }

    // Step 6: normals. Smooth normals are accumulated over all triangles that meet in a position.
    let valid = |n: &Option<[Vec3; 3]>| n.is_some_and(|n| n.iter().all(|&v| unit(v).is_some()));
    let mut smooth: HashMap<[u32; 3], (DVec3, f64)> = HashMap::new();
    if work.iter().any(|w| !valid(&w.normals)) {
        for w in &work {
            let weight = w.face.length();
            for p in w.pos {
                let e = smooth.entry(bits3(p)).or_insert((DVec3::ZERO, 0.0));
                e.0 += w.face;
                e.1 += weight;
            }
        }
    }
    let mut normals: Vec<[Vec3; 3]> = Vec::with_capacity(work.len());
    for w in &work {
        let mut out = [Vec3::ZERO; 3];
        for k in 0..3 {
            out[k] = match w.normals.and_then(|n| unit(n[k])) {
                Some(n) => n,
                None => {
                    report.normals_generated = true;
                    let face = (w.face / w.face.length()).as_vec3() + Vec3::ZERO;
                    match smooth.get(&bits3(w.pos[k])) {
                        Some(&(sum, total)) if sum.length() > 1e-6 * total => unit(sum.as_vec3()).unwrap_or(face),
                        _ => face,
                    }
                }
            };
        }
        normals.push(out);
    }

    // Step 8 (order): stable sort by (material, NO_UV last). Done before unifying so vertices follow the index order.
    let mut order: Vec<usize> = (0..work.len()).collect();
    order.sort_by_key(|&i| (work[i].material, work[i].flags.contains(TriFlags::NO_UV)));

    // UV_MIRRORED: sign of the UV area against the area-weighted majority of the set. On an exact tie the
    // unmirrored orientation (negative in v-down UV space) is the majority.
    let mut set_area = vec![(0.0f64, 0.0f64); sets.len()]; // (negative, positive)
    let tri_set = |w: &Work| if w.flags.contains(TriFlags::NO_UV) { None } else { material_set[w.material as usize] };
    for w in &work {
        if let Some(set) = tri_set(w) {
            let a = signed_uv_area(w.uv);
            if a < 0.0 {
                set_area[set.index()].0 -= a;
            } else {
                set_area[set.index()].1 += a;
            }
        }
    }
    let mirrored: Vec<bool> = work
        .iter()
        .map(|w| match tri_set(w) {
            Some(set) => {
                let (negative, positive) = set_area[set.index()];
                let majority_negative = negative >= positive;
                (signed_uv_area(w.uv) < 0.0) != majority_negative
            }
            None => false,
        })
        .collect();

    // Step 7: unify indices. Insertion order (never iteration order) defines the vertex order.
    let mut lookup: HashMap<[u32; 9], u32> = HashMap::with_capacity(work.len());
    let mut mesh = MeshData {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::with_capacity(work.len() * 3),
        tri_material: Vec::with_capacity(work.len()),
        tri_flags: Vec::with_capacity(work.len()),
        materials: Vec::with_capacity(material_count),
        material_ranges: Vec::with_capacity(material_count),
        material_uv_ranges: Vec::with_capacity(material_count),
        material_nouv_ranges: Vec::with_capacity(material_count),
        sets,
        bounds: Aabb::EMPTY,
        uv_hash: 0,
        tri_object: Vec::with_capacity(work.len()),
        objects: Vec::new(),
    };
    for &i in &order {
        let w = &work[i];
        for (k, &n) in normals[i].iter().enumerate() {
            let (p, uv) = (w.pos[k], w.uv[k]);
            let (pb, nb) = (bits3(p), bits3(n));
            let key = [pb[0], pb[1], pb[2], nb[0], nb[1], nb[2], uv.x.to_bits(), uv.y.to_bits(), w.material];
            let next = mesh.positions.len() as u32;
            let index = *lookup.entry(key).or_insert_with(|| {
                mesh.positions.push(p);
                mesh.normals.push(n);
                mesh.uvs.push(uv);
                next
            });
            mesh.indices.push(index);
        }
        mesh.tri_material.push(w.material);
        mesh.tri_flags.push(if mirrored[i] { w.flags | TriFlags::UV_MIRRORED } else { w.flags });
    }

    // Step 9: objects, numbered densely in order of first appearance in the raw triangle order (kept triangles only;
    // `work` is in raw order). Not part of any sort or vertex key, so nothing above depends on them.
    let mut dense: HashMap<u32, u32> = HashMap::new();
    let mut raw_of_dense: Vec<u32> = Vec::new();
    for w in &work {
        dense.entry(w.object).or_insert_with(|| {
            raw_of_dense.push(w.object);
            raw_of_dense.len() as u32 - 1
        });
    }
    mesh.objects = object_infos(&raw_of_dense, objects, opts);
    mesh.tri_object.extend(order.iter().map(|&i| dense[&work[i].object]));
    for t in 0..mesh.tri_count() {
        let [a, b, c] = mesh.tri_positions(t);
        let info = &mut mesh.objects[mesh.tri_object[t as usize] as usize];
        info.tri_count += 1;
        for p in [a, b, c] {
            info.bounds.extend(p);
        }
    }

    // Step 8 (rest): materials, ranges, bounds, hash.
    let mut cursor = 0u32;
    for (m, raw_material) in raw_materials.into_iter().enumerate() {
        let (uv, nouv) = (uv_tris[m], nouv_tris[m]);
        mesh.material_ranges.push(cursor..cursor + uv + nouv);
        mesh.material_uv_ranges.push(cursor..cursor + uv);
        mesh.material_nouv_ranges.push(cursor + uv..cursor + uv + nouv);
        cursor += uv + nouv;
        mesh.materials.push(MaterialSlot {
            name: raw_material.name,
            base_color: raw_material.base_color,
            double_sided: raw_material.double_sided,
            texture_set: material_set[m],
        });
    }
    mesh.bounds = Aabb::from_points(&mesh.positions);
    let mut h = hash_u32s(FNV_BASIS, mesh.indices.iter().copied());
    h = hash_u32s(h, mesh.uvs.iter().flat_map(|uv| [uv.x.to_bits(), uv.y.to_bits()]));
    h = hash_u32s(h, (0..mesh.tri_count()).map(|t| mesh.tri_set(t).map_or(u32::MAX, |s| s.0)));
    mesh.uv_hash = h;

    (mesh, report)
}

/// Step 9 names: an empty name becomes "Object N" (N = 1-based dense index); a single unnamed object takes the model
/// name ("Model" when that is empty); then names are made unique in index order with `_2`, `_3`. Counts and bounds
/// are filled in by the caller.
fn object_infos(raw_of_dense: &[u32], objects: &RawObjects, opts: &NormalizeOptions) -> Vec<ObjectInfo> {
    let single = raw_of_dense.len() == 1;
    let mut used: HashSet<String> = HashSet::with_capacity(raw_of_dense.len());
    let mut out = Vec::with_capacity(raw_of_dense.len());
    for (i, &r) in raw_of_dense.iter().enumerate() {
        let given = objects.names.get(r as usize).map_or("", String::as_str);
        let base = if !given.is_empty() {
            given.to_owned()
        } else if single {
            if opts.model_name.is_empty() { "Model".to_owned() } else { opts.model_name.clone() }
        } else {
            format!("Object {}", i + 1)
        };
        let mut name = base.clone();
        let mut suffix = 2;
        while used.contains(&name) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        used.insert(name.clone());
        out.push(ObjectInfo { name, tri_count: 0, bounds: Aabb::EMPTY });
    }
    out
}

/// One side of a triangle edge, keyed for grouping: `key` = the two position bit patterns (smaller first).
/// `object` is the last field, so the derived order of the other fields is unchanged; it never takes part in the
/// grouping (`same_side`).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct HalfEdge {
    key: [u32; 6],
    /// `TextureSetId.0`, `u32::MAX` for none.
    set: u32,
    /// UV bit patterns at the two ends, in key order.
    uv: [u32; 4],
    /// The object of the triangle.
    object: u32,
}

impl HalfEdge {
    /// The same geometric edge with the same set and UVs (the object does not count).
    fn same_side(&self, other: &HalfEdge) -> bool {
        self.key == other.key && self.set == other.set && self.uv == other.uv
    }
}

impl MeshData {
    /// GPU vertices, 1:1 with positions/normals/uvs.
    pub fn render_vertices(&self) -> Vec<RenderVertex> {
        (0..self.positions.len())
            .map(|i| RenderVertex {
                pos: self.positions[i].to_array(),
                normal: self.normals.get(i).copied().unwrap_or(Vec3::Y).to_array(),
                uv: self.uvs.get(i).copied().unwrap_or(Vec2::ZERO).to_array(),
            })
            .collect()
    }

    /// Wireframe edges, de-duplicated, deterministic order (section 6.1).
    ///
    /// Triangle sides with bit-identical end positions are one geometric edge. One incident triangle => one
    /// `Boundary` entry; several whose set and UVs at both ends are identical => one `Interior` entry; otherwise
    /// one `Seam` entry per distinct (set, UV pair). `a` is the end with the smaller position bit pattern. Entries
    /// of triangles without UVs have `set == None` and zero UVs. Sorted by (set, kind, a, b, uv_a, uv_b) with sets
    /// ascending and `None` last, kinds in the order Interior, Seam, Boundary.
    pub fn wire_edges(&self) -> Vec<WireEdge> {
        let mut halves: Vec<HalfEdge> = Vec::with_capacity(self.indices.len());
        for t in 0..self.tri_count() {
            let tri = self.tri(t);
            let set = self.tri_set(t);
            let object = self.tri_object_of(t);
            for k in 0..3 {
                let (ia, ib) = (tri[k] as usize, tri[(k + 1) % 3] as usize);
                let (pa, pb) = (bits3(self.positions[ia]), bits3(self.positions[ib]));
                if pa == pb {
                    continue;
                }
                let uv_bits = |i: usize| match set {
                    Some(_) => [(self.uvs[i].x + 0.0).to_bits(), (self.uvs[i].y + 0.0).to_bits()],
                    None => [0, 0],
                };
                let (ua, ub) = (uv_bits(ia), uv_bits(ib));
                let (pa, pb, ua, ub) = if pa <= pb { (pa, pb, ua, ub) } else { (pb, pa, ub, ua) };
                halves.push(HalfEdge {
                    key: [pa[0], pa[1], pa[2], pb[0], pb[1], pb[2]],
                    set: set.map_or(u32::MAX, |s| s.0),
                    uv: [ua[0], ua[1], ub[0], ub[1]],
                    object,
                });
            }
        }
        if halves.len() >= 1 << 16 {
            halves.par_sort_unstable();
        } else {
            halves.sort_unstable();
        }

        let mut edges: Vec<WireEdge> = Vec::with_capacity(halves.len() / 2 + 1);
        let emit = |h: &HalfEdge, kind: EdgeKind| WireEdge {
            a: [f32::from_bits(h.key[0]), f32::from_bits(h.key[1]), f32::from_bits(h.key[2])],
            b: [f32::from_bits(h.key[3]), f32::from_bits(h.key[4]), f32::from_bits(h.key[5])],
            uv_a: [f32::from_bits(h.uv[0]), f32::from_bits(h.uv[1])],
            uv_b: [f32::from_bits(h.uv[2]), f32::from_bits(h.uv[3])],
            kind,
            set: if h.set == u32::MAX { None } else { Some(TextureSetId(h.set)) },
            object: h.object,
        };
        for group in halves.chunk_by(|x, y| x.key == y.key) {
            if group.len() == 1 {
                edges.push(emit(&group[0], EdgeKind::Boundary));
                continue;
            }
            // The group is sorted by (set, uv): distinct sides are runs of equal entries.
            let distinct = group.chunk_by(|x, y| x.same_side(y)).count();
            let kind = if distinct == 1 { EdgeKind::Interior } else { EdgeKind::Seam };
            for side in group.chunk_by(|x, y| x.same_side(y)) {
                edges.push(emit(&side[0], kind));
            }
        }

        let kind_rank = |k: EdgeKind| match k {
            EdgeKind::Interior => 0u8,
            EdgeKind::Seam => 1,
            EdgeKind::Boundary => 2,
        };
        let cmp_floats = |x: &[f32], y: &[f32]| -> Ordering {
            x.iter().zip(y).map(|(p, q)| p.total_cmp(q)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        let cmp = |x: &WireEdge, y: &WireEdge| -> Ordering {
            let set = |e: &WireEdge| e.set.map_or(u32::MAX, |s| s.0);
            set(x)
                .cmp(&set(y))
                .then(kind_rank(x.kind).cmp(&kind_rank(y.kind)))
                .then_with(|| cmp_floats(&x.a, &y.a))
                .then_with(|| cmp_floats(&x.b, &y.b))
                .then_with(|| cmp_floats(&x.uv_a, &y.uv_a))
                .then_with(|| cmp_floats(&x.uv_b, &y.uv_b))
        };
        if edges.len() >= 1 << 16 {
            edges.par_sort_unstable_by(cmp);
        } else {
            edges.sort_unstable_by(cmp);
        }
        edges
    }

    /// Number of UV-mapped triangles of a set.
    pub fn set_tri_count(&self, set: TextureSetId) -> u32 {
        self.sets.get(set.index()).map_or(0, |s| s.tri_count)
    }
}

/// Pixel coordinate of a UV coordinate in the layout image: 0 maps to the first pixel, 1 to the last one.
fn layout_px(uv: f32, side: u32) -> Option<i64> {
    let v = (uv as f64 * (side - 1) as f64).round();
    if v.is_finite() && v.abs() < 1e15 { Some(v as i64) } else { None }
}

/// UV layout of one set as an opaque `side x side` image (R2, import dialog): dark background (#202428), every
/// `wire_edges()` entry of the set drawn as a 1-px integer DDA line between its UV endpoints (wrapped into the image):
/// Interior grey (#8a8f98), Seam orange (#ff9d2e), Boundary cyan (#35d0e0); seams and boundaries are drawn last.
/// Cost O(edges * side); called once per set on the import worker, never per frame. For every set of a model use
/// `uv_layout_images`, which derives the edges once.
pub fn uv_layout_image(mesh: &MeshData, set: TextureSetId, side: u32) -> PixelBuf {
    let edges = mesh.wire_edges();
    draw_uv_layout(edges.iter().filter(|e| e.set == Some(set)), side)
}

/// `uv_layout_image` of every set (index = `TextureSetId.0`): `wire_edges` is computed once and the sets are drawn
/// in parallel (a model with many sets and millions of triangles took seconds per set before).
pub fn uv_layout_images(mesh: &MeshData, side: u32) -> Vec<PixelBuf> {
    let edges = mesh.wire_edges();
    // `wire_edges` is sorted by set: one contiguous run per set (`None` last).
    let mut runs: Vec<&[WireEdge]> = vec![&[]; mesh.sets.len()];
    for run in edges.chunk_by(|a, b| a.set == b.set) {
        if let Some(slot) = run[0].set.and_then(|s| runs.get_mut(s.index())) {
            *slot = run;
        }
    }
    runs.into_par_iter().map(|run| draw_uv_layout(run.iter(), side)).collect()
}

/// Draws the given edges (sorted by kind, Interior first) into a fresh layout image.
fn draw_uv_layout<'a>(edges: impl Iterator<Item = &'a WireEdge>, side: u32) -> PixelBuf {
    const BACKGROUND: [u8; 4] = [0x20, 0x24, 0x28, 0xff];
    const INTERIOR: [u8; 4] = [0x8a, 0x8f, 0x98, 0xff];
    const SEAM: [u8; 4] = [0xff, 0x9d, 0x2e, 0xff];
    const BOUNDARY: [u8; 4] = [0x35, 0xd0, 0xe0, 0xff];
    let mut img = PixelBuf::filled(side, side, BACKGROUND);
    if side == 0 {
        return img;
    }
    let n = side as i64;
    // `wire_edges` is sorted by (set, kind) with Interior first, so seams and boundaries overwrite interior lines.
    for e in edges {
        let color = match e.kind {
            EdgeKind::Interior => INTERIOR,
            EdgeKind::Seam => SEAM,
            EdgeKind::Boundary => BOUNDARY,
        };
        let (Some(x0), Some(y0), Some(x1), Some(y1)) =
            (layout_px(e.uv_a[0], side), layout_px(e.uv_a[1], side), layout_px(e.uv_b[0], side), layout_px(e.uv_b[1], side))
        else {
            continue;
        };
        let (dx, dy) = (x1 - x0, y1 - y0);
        let steps = dx.abs().max(dy.abs());
        // A line crossing many tiles covers every pixel long before it ends: bound the work per edge.
        let samples = steps.min(4 * n);
        for i in 0..=samples {
            let (x, y) = if samples == 0 {
                (x0, y0)
            } else {
                // Position `i * steps / samples` along the major axis, rounded to the nearest pixel on the minor one.
                let s = (i as i128 * steps as i128 / samples as i128) as i64;
                let along = |d: i64| {
                    let num = d as i128 * s as i128;
                    let den = steps as i128;
                    ((2 * num + den).div_euclid(2 * den)) as i64
                };
                (x0 + along(dx), y0 + along(dy))
            };
            img.set(x.rem_euclid(n) as u32, y.rem_euclid(n) as u32, color);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples;
    use crate::testutil::{XorShift, hash_pixels, mesh_of};
    use crate::texel_map::TexelMap;

    fn tri(p: [[f32; 3]; 3], uv: Option<[[f32; 2]; 3]>, material: u32) -> RawTriangle {
        RawTriangle {
            positions: p.map(Vec3::from_array),
            normals: None,
            uvs: uv.map(|uv| uv.map(Vec2::from_array)),
            material,
        }
    }

    fn mat(name: &str, texture: Option<u32>) -> RawMaterial {
        RawMaterial { name: name.to_owned(), base_color: [0.5, 0.6, 0.7, 1.0], double_sided: true, texture }
    }

    /// A unit right triangle in the XY plane shifted by `x`, with UVs in the lower-left half of the unit square.
    fn simple(x: f32, material: u32, with_uv: bool) -> RawTriangle {
        tri(
            [[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]],
            with_uv.then_some([[0.1, 0.9], [0.9, 0.9], [0.1, 0.1]]),
            material,
        )
    }

    /// Checks the structural invariants every consumer relies on.
    fn check_structure(mesh: &MeshData) {
        let n = mesh.positions.len();
        assert_eq!(mesh.normals.len(), n);
        assert_eq!(mesh.uvs.len(), n);
        assert_eq!(mesh.indices.len() % 3, 0);
        assert!(mesh.indices.iter().all(|&i| (i as usize) < n));
        let tris = mesh.tri_count() as usize;
        assert_eq!(mesh.tri_material.len(), tris);
        assert_eq!(mesh.tri_flags.len(), tris);
        let m = mesh.materials.len();
        assert_eq!((mesh.material_ranges.len(), mesh.material_uv_ranges.len(), mesh.material_nouv_ranges.len()), (m, m, m));
        let mut cursor = 0u32;
        for i in 0..m {
            let (all, uv, nouv) = (&mesh.material_ranges[i], &mesh.material_uv_ranges[i], &mesh.material_nouv_ranges[i]);
            assert_eq!(all.start, cursor);
            assert_eq!((uv.start, uv.end, nouv.end), (all.start, nouv.start, all.end), "ranges partition");
            for t in all.clone() {
                assert_eq!(mesh.tri_material[t as usize], i as u32);
                assert_eq!(mesh.tri_flags[t as usize].contains(TriFlags::NO_UV), nouv.contains(&t));
            }
            if mesh.materials[i].texture_set.is_none() {
                assert!(uv.is_empty());
            } else {
                assert!(!uv.is_empty());
            }
            cursor = all.end;
        }
        assert_eq!(cursor as usize, tris);
        for (i, set) in mesh.sets.iter().enumerate() {
            let id = TextureSetId(i as u32);
            assert!(!set.materials.is_empty());
            assert!(set.materials.iter().all(|&m| mesh.materials[m as usize].texture_set == Some(id)));
            let count = (0..mesh.tri_count()).filter(|&t| mesh.tri_set(t) == Some(id)).count() as u32;
            assert_eq!(set.tri_count, count);
            assert_eq!(mesh.set_tri_count(id), count);
            assert_eq!(mesh.sets.iter().filter(|s| s.name == set.name).count(), 1, "set names are unique");
        }
        assert_eq!(mesh.set_tri_count(TextureSetId(mesh.sets.len() as u32)), 0);
        for nrm in &mesh.normals {
            assert!((nrm.length() - 1.0).abs() < 1e-5, "normal {nrm:?} is not unit length");
        }
        for t in 0..mesh.tri_count() {
            if mesh.tri_flags[t as usize].contains(TriFlags::NO_UV) {
                continue;
            }
            let uv = mesh.tri_uvs(t);
            let c = (uv[0].as_dvec2() + uv[1].as_dvec2() + uv[2].as_dvec2()) / 3.0;
            assert!(c.x >= 0.0 && c.x < 1.0 && c.y >= 0.0 && c.y < 1.0, "centroid {c:?} outside the unit tile");
            let outside = uv.iter().any(|c| c.x < 0.0 || c.y < 0.0 || c.x > 1.0 || c.y > 1.0);
            assert_eq!(mesh.tri_flags[t as usize].contains(TriFlags::UV_WRAPS), outside);
        }
        assert_eq!(mesh.bounds, Aabb::from_points(&mesh.positions));
    }

    #[test]
    fn tile_normalization_keeps_centroids_in_the_unit_tile() {
        let mut rng = XorShift::new(11);
        let mut raw = RawMesh { triangles: Vec::new(), materials: vec![mat("m", None)] };
        let mut coord = |range: f32| (rng.next_f32() - 0.5) * 2.0 * range;
        for _ in 0..2000 {
            let base = [coord(6.0), coord(6.0)];
            let uv = [0, 1, 2].map(|_| [base[0] + coord(0.8), base[1] + coord(0.8)]);
            raw.triangles.push(tri([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], Some(uv), 0));
        }
        let source = raw.clone();
        let (mesh, report) = normalize(raw, &NormalizeOptions::default());
        assert_eq!(mesh.tri_count(), 2000);
        assert_eq!(report.no_uv_tris, 0);
        check_structure(&mesh);
        assert!(mesh.tri_flags.iter().any(|f| f.contains(TriFlags::UV_WRAPS)));
        assert!(mesh.tri_flags.iter().any(|f| !f.contains(TriFlags::UV_WRAPS)));
        // Every UV moved by a whole tile only, the same one for the three corners.
        for (t, src) in source.triangles.iter().enumerate() {
            let (out, src) = (mesh.tri_uvs(t as u32), src.uvs.unwrap());
            let shift = (src[0] - out[0]).round();
            for k in 0..3 {
                assert!((src[k] - out[k] - shift).length() < 1e-5);
            }
        }
        assert!(report.wrapped_tris > 0 && report.wrapped_tris <= 2000);

        // The wrapped sample: quad A straddles the borders, quad B is folded from tile 2.
        let (mesh, report) = normalize(samples::wrapped_quad(), &NormalizeOptions::default());
        check_structure(&mesh);
        assert_eq!(report.wrapped_tris, 4);
        assert_eq!(mesh.tri_flags.iter().filter(|f| f.contains(TriFlags::UV_WRAPS)).count(), 2);

        // UVs inside the unit square are untouched and nothing is reported.
        let (mesh, report) = normalize(samples::flat_quad(), &NormalizeOptions::default());
        assert_eq!(report, MeshReport { total_tris: 2, normals_generated: true, ..MeshReport::default() });
        let (_, report) = normalize(samples::seam_cube(1.0), &NormalizeOptions::default());
        assert_eq!(report, MeshReport { total_tris: 12, ..MeshReport::default() });
        assert!(mesh.tri_flags.iter().all(|f| f.is_empty()));
        assert!(mesh.uvs.contains(&Vec2::new(1.0, 1.0)) && mesh.uvs.contains(&Vec2::ZERO));
    }

    #[test]
    fn missing_uvs_are_sorted_last_within_their_material() {
        let raw = RawMesh {
            triangles: vec![
                simple(0.0, 1, false),
                simple(1.0, 0, true),
                simple(2.0, 1, true),
                simple(3.0, 0, false),
                simple(4.0, 1, false),
                simple(5.0, 0, true),
                simple(6.0, 2, false),
                // Degenerate UVs (zero area) and non-finite UVs count as missing.
                tri([[7.0, 0.0, 0.0], [8.0, 0.0, 0.0], [7.0, 1.0, 0.0]], Some([[0.2, 0.2], [0.4, 0.4], [0.6, 0.6]]), 1),
                tri([[8.0, 0.0, 0.0], [9.0, 0.0, 0.0], [8.0, 1.0, 0.0]], Some([[0.2, 0.2], [f32::NAN, 0.4], [0.6, 0.1]]), 1),
            ],
            materials: vec![mat("a", None), mat("b", None), mat("c", None), mat("unused", Some(0))],
        };
        let (mesh, report) = normalize(raw, &NormalizeOptions::default());
        check_structure(&mesh);
        assert_eq!(report.total_tris, 9);
        assert_eq!(report.no_uv_tris, 6);
        assert!(!report.no_uvs && !report.fallback_uvs_generated);
        assert_eq!(mesh.material_ranges, vec![0..3, 3..8, 8..9, 9..9]);
        assert_eq!(mesh.material_uv_ranges, vec![0..2, 3..4, 8..8, 9..9]);
        assert_eq!(mesh.material_nouv_ranges, vec![2..3, 4..8, 8..9, 9..9]);
        // Stable: source order is kept inside each group (x of the first vertex identifies the source triangle).
        let first_x: Vec<f32> = (0..mesh.tri_count()).map(|t| mesh.tri_positions(t)[0].x).collect();
        assert_eq!(first_x, vec![1.0, 5.0, 3.0, 2.0, 0.0, 4.0, 7.0, 8.0, 6.0]);
        // Materials without UV-mapped triangles get no set.
        assert_eq!(
            mesh.materials.iter().map(|m| m.texture_set).collect::<Vec<_>>(),
            vec![Some(TextureSetId(0)), Some(TextureSetId(1)), None, None]
        );
        assert_eq!(mesh.sets.len(), 2);
        assert_eq!(mesh.tri_set(2), None);
        assert_eq!(mesh.tri_set(3), Some(TextureSetId(1)));
        // NO_UV triangles carry zero UVs; material properties are copied.
        assert_eq!(mesh.tri_uvs(2), [Vec2::ZERO; 3]);
        assert_eq!(mesh.materials[0].base_color, [0.5, 0.6, 0.7, 1.0]);
        assert!(mesh.materials[0].double_sided);

        // The whole model without UVs.
        let (mesh, report) = normalize(samples::no_uv_tetra(), &NormalizeOptions::default());
        check_structure(&mesh);
        assert!(report.no_uvs && report.normals_generated && !report.fallback_uvs_generated);
        assert_eq!(report.no_uv_tris, 4);
        assert!(mesh.sets.is_empty());
        assert_eq!(mesh.materials[0].texture_set, None);
        assert_eq!(mesh.material_nouv_ranges, vec![0..4]);
    }

    #[test]
    fn degenerate_triangles_are_dropped_and_counted() {
        let raw = RawMesh {
            triangles: vec![
                simple(0.0, 0, true),
                // Zero area: two identical corners, three collinear corners.
                tri([[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]], None, 0),
                tri([[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]], None, 0),
                // Far below the relative threshold (the model is about 10 units across).
                tri([[5.0, 5.0, 0.0], [5.000001, 5.0, 0.0], [5.0, 5.000001, 0.0]], None, 0),
                // Non-finite positions.
                tri([[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], None, 0),
                tri([[0.0, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0], [0.0, 1.0, 0.0]], None, 0),
                simple(9.0, 0, true),
            ],
            materials: vec![mat("m", None)],
        };
        let (mesh, report) = normalize(raw, &NormalizeOptions::default());
        check_structure(&mesh);
        assert_eq!(report.total_tris, 7);
        assert_eq!(report.dropped_degenerate, 5);
        assert_eq!(mesh.tri_count(), 2);
        assert!(mesh.positions.iter().all(|p| p.is_finite()));
        assert_eq!(mesh.bounds.max, Vec3::new(10.0, 1.0, 0.0));

        // A tiny model keeps its triangles: the threshold is relative to the model size.
        let s = 1e-6f32;
        let raw = RawMesh {
            triangles: vec![tri([[0.0, 0.0, 0.0], [s, 0.0, 0.0], [0.0, s, 0.0]], None, 0)],
            materials: vec![mat("m", None)],
        };
        let (mesh, report) = normalize(raw, &NormalizeOptions::default());
        assert_eq!((mesh.tri_count(), report.dropped_degenerate), (1, 0));

        // Nothing left: an empty but well-formed mesh.
        let (mesh, report) = normalize(RawMesh::default(), &NormalizeOptions::default());
        check_structure(&mesh);
        assert_eq!(report, MeshReport::default());
        assert!(mesh.bounds.is_empty() && mesh.sets.is_empty() && mesh.materials.is_empty());
        assert!(mesh.wire_edges().is_empty() && mesh.render_vertices().is_empty());
        let all_bad = RawMesh { triangles: vec![tri([[0.0; 3]; 3], None, 0)], materials: vec![mat("m", Some(0))] };
        let (mesh, report) = normalize(all_bad, &NormalizeOptions { generate_fallback_uvs: true, ..Default::default() });
        check_structure(&mesh);
        assert_eq!((mesh.tri_count(), report.dropped_degenerate), (0, 1));
        assert!(!report.no_uvs && !report.fallback_uvs_generated && mesh.sets.is_empty());
    }

    #[test]
    fn generated_normals_are_unit_and_smooth() {
        // Tetrahedron without normals: every vertex normal is the normalized sum of its three face normals,
        // which for this regular tetrahedron points from the centre through the vertex.
        let (mesh, report) = normalize(samples::no_uv_tetra(), &NormalizeOptions::default());
        check_structure(&mesh);
        assert!(report.normals_generated);
        assert_eq!(mesh.positions.len(), 4, "smooth normals let the corners be shared");
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            assert!((p.normalize() - *n).length() < 1e-5, "{p:?} {n:?}");
        }

        // Area weighting: a large and a small face meeting in an edge.
        let raw = RawMesh {
            triangles: vec![
                tri([[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [0.0, 0.0, -3.0]], None, 0), // faces +Y, area 4.5
                tri([[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]], None, 0), // faces -X, area 0.5
            ],
            materials: vec![mat("m", None)],
        };
        let (mesh, _) = normalize(raw, &NormalizeOptions::default());
        let origin = mesh.positions.iter().position(|p| *p == Vec3::ZERO).unwrap();
        let expect = Vec3::new(-1.0, 9.0, 0.0).normalize();
        assert!((mesh.normals[origin] - expect).length() < 1e-5, "{:?}", mesh.normals[origin]);

        // Opposite faces cancel: fall back to the face normal (two vertices per position).
        let mut raw = samples::flat_quad();
        let mut back = raw.clone();
        samples::reverse_winding(&mut back);
        raw.triangles.extend(back.triangles);
        let (mesh, _) = normalize(raw, &NormalizeOptions::default());
        check_structure(&mesh);
        for t in 0..mesh.tri_count() {
            assert_eq!(mesh.tri_normals(t), [mesh.geo_normal(t); 3]);
        }

        // Provided normals are normalized; broken ones are replaced.
        let mut raw = samples::flat_quad();
        raw.triangles[0].normals = Some([Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 1.0)]);
        raw.triangles[1].normals = Some([Vec3::new(0.0, 3.0, 4.0); 3]);
        let (mesh, report) = normalize(raw, &NormalizeOptions::default());
        check_structure(&mesh);
        assert!(report.normals_generated);
        assert_eq!(mesh.tri_normals(0), [Vec3::Z; 3]);
        assert!((mesh.tri_normals(1)[0] - Vec3::new(0.0, 0.6, 0.8)).length() < 1e-6);
        let (_, report) = normalize(samples::seam_cube(1.0), &NormalizeOptions::default());
        assert!(!report.normals_generated);
    }

    fn count(edges: &[WireEdge], kind: EdgeKind) -> usize {
        edges.iter().filter(|e| e.kind == kind).count()
    }

    #[test]
    fn wire_edges_of_the_seam_cube() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let edges = mesh.wire_edges();
        assert_eq!(count(&edges, EdgeKind::Seam), 24, "12 geometric edges x 2 UV sides");
        assert_eq!(count(&edges, EdgeKind::Interior), 6, "the face diagonals");
        assert_eq!(count(&edges, EdgeKind::Boundary), 0);
        assert!(edges.iter().all(|e| e.set == Some(TextureSetId(0))));
        // Seam entries come in pairs with the same positions and different UVs.
        let seams: Vec<&WireEdge> = edges.iter().filter(|e| e.kind == EdgeKind::Seam).collect();
        for pair in seams.chunks(2) {
            assert_eq!((pair[0].a, pair[0].b), (pair[1].a, pair[1].b));
            assert_ne!((pair[0].uv_a, pair[0].uv_b), (pair[1].uv_a, pair[1].uv_b));
        }
        // Every entry's UVs belong to its positions: some triangle has both ends with exactly these attributes.
        for e in &edges {
            let found = (0..mesh.tri_count()).any(|t| {
                let (p, uv) = (mesh.tri_positions(t), mesh.tri_uvs(t));
                let has = |pos: [f32; 3], tex: [f32; 2]| (0..3).any(|k| p[k].to_array() == pos && uv[k].to_array() == tex);
                has(e.a, e.uv_a) && has(e.b, e.uv_b)
            });
            assert!(found);
        }
        // Sorted by kind inside the set, deterministic.
        let kinds: Vec<EdgeKind> = edges.iter().map(|e| e.kind).collect();
        assert!(kinds[..6].iter().all(|k| *k == EdgeKind::Interior) && kinds[6..].iter().all(|k| *k == EdgeKind::Seam));
        let again = mesh_of(samples::seam_cube(1.0)).wire_edges();
        assert_eq!(format!("{edges:?}"), format!("{again:?}"));
    }

    #[test]
    fn wire_edges_of_the_sphere_and_open_meshes() {
        let (seg, rings) = (12u32, 6u32);
        let mesh = mesh_of(samples::uv_sphere(seg, rings, 1.0));
        let edges = mesh.wire_edges();
        assert_eq!(count(&edges, EdgeKind::Boundary), 0, "the sphere is closed");
        // The seam column: `rings` geometric edges on the meridian x = 0, z < 0, each with a u = 0 and a u = 1 side.
        let on_seam = |p: [f32; 3]| p[0].abs() < 1e-6 && p[2] <= 1e-6;
        let column: Vec<&WireEdge> =
            edges.iter().filter(|e| e.kind == EdgeKind::Seam && on_seam(e.a) && on_seam(e.b)).collect();
        assert_eq!(column.len(), 2 * rings as usize);
        assert_eq!(column.iter().filter(|e| e.uv_a[0].max(e.uv_b[0]) == 1.0).count(), rings as usize);
        assert_eq!(column.iter().filter(|e| e.uv_a[0].min(e.uv_b[0]) == 0.0).count(), rings as usize);
        // The only other seams are the pole fans (each pole triangle has its own pole UV).
        let pole = |p: [f32; 3]| p[1].abs() == 1.0;
        let fans = edges.iter().filter(|e| e.kind == EdgeKind::Seam && !(on_seam(e.a) && on_seam(e.b))).count();
        assert_eq!(fans, 2 * 2 * (seg as usize - 1));
        assert!(edges.iter().filter(|e| e.kind == EdgeKind::Seam).all(|e| (on_seam(e.a) && on_seam(e.b)) || pole(e.a) || pole(e.b)));
        // Everything else is interior: the ring edges and the remaining meridian and diagonal edges.
        let rings_edges = (seg * (rings - 1)) as usize;
        let meridians = ((seg - 1) * (rings - 2)) as usize;
        let diagonals = (seg * (rings - 2)) as usize;
        assert_eq!(count(&edges, EdgeKind::Interior), rings_edges + meridians + diagonals);

        // An open quad: four boundary edges and the diagonal.
        let edges = mesh_of(samples::flat_quad()).wire_edges();
        assert_eq!((count(&edges, EdgeKind::Boundary), count(&edges, EdgeKind::Interior), edges.len()), (4, 1, 5));
        // Mirrored quads share their middle edge with identical UVs: interior, not a seam.
        let edges = mesh_of(samples::mirrored_quads()).wire_edges();
        assert_eq!((count(&edges, EdgeKind::Boundary), count(&edges, EdgeKind::Interior), count(&edges, EdgeKind::Seam)), (6, 3, 0));
        // Without UVs the entries have no set and zero UVs.
        let edges = mesh_of(samples::no_uv_tetra()).wire_edges();
        assert_eq!((edges.len(), count(&edges, EdgeKind::Interior)), (6, 6));
        assert!(edges.iter().all(|e| e.set.is_none() && e.uv_a == [0.0; 2] && e.uv_b == [0.0; 2]));
        // Two materials meeting in an edge: a seam between two sets even though the UVs agree.
        let raw = RawMesh {
            triangles: vec![
                tri([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], Some([[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]]), 0),
                tri([[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]], Some([[1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]), 1),
            ],
            materials: vec![mat("a", None), mat("b", None)],
        };
        let edges = mesh_of(raw).wire_edges();
        assert_eq!((count(&edges, EdgeKind::Boundary), count(&edges, EdgeKind::Seam)), (4, 2));
        let sets: Vec<Option<TextureSetId>> = edges.iter().map(|e| e.set).collect();
        assert!(sets.windows(2).all(|w| w[0] <= w[1]), "sorted by set");
    }

    #[test]
    fn materials_sharing_a_texture_share_a_set() {
        let raw = RawMesh {
            triangles: (0..6).map(|m| simple(m as f32 * 2.0, m, true)).collect(),
            materials: vec![
                mat("wood", Some(4)),
                mat("plain", None),
                mat("bark", Some(4)),
                mat("", Some(1)),
                mat("", None),
                mat("plain", None),
            ],
        };
        let opts = NormalizeOptions {
            generate_fallback_uvs: false,
            model_name: "tree".to_owned(),
            texture_names: vec![String::new(), "leaves".to_owned()],
        };
        let (mesh, _) = normalize(raw.clone(), &opts);
        check_structure(&mesh);
        // Set order = first appearance by material index.
        assert_eq!(mesh.sets.len(), 5);
        assert_eq!(mesh.sets[0], SetInfo { name: "wood".into(), materials: vec![0, 2], texture: Some(4), tri_count: 2 });
        assert_eq!(mesh.sets[1], SetInfo { name: "plain".into(), materials: vec![1], texture: None, tri_count: 1 });
        assert_eq!(mesh.sets[2], SetInfo { name: "leaves".into(), materials: vec![3], texture: Some(1), tri_count: 1 });
        assert_eq!(mesh.sets[3], SetInfo { name: "tree".into(), materials: vec![4], texture: None, tri_count: 1 });
        assert_eq!(mesh.sets[4], SetInfo { name: "plain_2".into(), materials: vec![5], texture: None, tri_count: 1 });
        assert_eq!(mesh.materials[2].texture_set, Some(TextureSetId(0)));
        assert_eq!(mesh.materials[2].name, "bark");

        // Texture stem wins over the material name; without any name the set is called "Texture".
        let opts = NormalizeOptions { texture_names: vec!["a".into(), "b".into(), "c".into(), "d".into(), "atlas".into()], ..Default::default() };
        let (mesh, _) = normalize(raw.clone(), &opts);
        assert_eq!(
            mesh.sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["atlas", "plain", "b", "Texture", "plain_2"]
        );
        // Uniquifying never collides with a later explicit name.
        let raw = RawMesh {
            triangles: (0..4).map(|m| simple(m as f32 * 2.0, m, true)).collect(),
            materials: vec![mat("x", None), mat("x", None), mat("x_2", None), mat("x", None)],
        };
        let mesh = mesh_of(raw);
        assert_eq!(mesh.sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["x", "x_2", "x_2_2", "x_3"]);
        check_structure(&mesh);

        // A triangle whose material index does not exist lands in an extra "default" material.
        let raw = RawMesh { triangles: vec![simple(0.0, 0, true), simple(2.0, 9, true)], materials: vec![mat("only", None)] };
        let mesh = mesh_of(raw);
        check_structure(&mesh);
        assert_eq!(mesh.materials.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["only", "default"]);
        assert_eq!(mesh.tri_material, vec![0, 1]);
        let mesh = mesh_of(RawMesh { triangles: vec![simple(0.0, 0, true)], materials: Vec::new() });
        assert_eq!((mesh.materials.len(), mesh.sets.len()), (1, 1));
        assert_eq!(mesh.sets[0].name, "default");
    }

    #[test]
    fn fallback_uvs_make_every_triangle_paintable() {
        for raw in [samples::no_uv_tetra(), {
            // An odd number of triangles spread over two materials, one of them with a (useless) texture.
            let mut soup = RawMesh { triangles: Vec::new(), materials: vec![mat("a", Some(0)), mat("b", None)] };
            for i in 0..37u32 {
                soup.triangles.push(simple(i as f32 * 1.5, i % 2, false));
            }
            soup
        }] {
            let tris = raw.triangles.len() as u32;
            let opts = NormalizeOptions { generate_fallback_uvs: true, model_name: "rock".into(), ..Default::default() };
            let (mesh, report) = normalize(raw.clone(), &opts);
            check_structure(&mesh);
            assert!(report.fallback_uvs_generated && report.no_uvs);
            assert_eq!(report.no_uv_tris, tris);
            assert_eq!(mesh.sets.len(), 1);
            assert_eq!(mesh.sets[0].name, "rock");
            assert_eq!(mesh.sets[0].texture, None);
            assert_eq!(mesh.sets[0].tri_count, tris);
            assert!(mesh.materials.iter().all(|m| m.texture_set == Some(TextureSetId(0))));
            assert!(mesh.tri_flags.iter().all(|f| f.is_empty()), "no NO_UV, no mirroring, no wrapping");
            assert!(mesh.material_nouv_ranges.iter().all(|r| r.is_empty()));
            // Paintable and non-overlapping: every triangle owns texels, none are claimed twice.
            let map = TexelMap::build(&mesh, TextureSetId(0), 256, 256, 1);
            assert_eq!(map.stats.overlapped_texels, 0);
            let mut owned = vec![0u32; tris as usize];
            for y in 0..256 {
                for x in 0..256 {
                    if let Some((t, false)) = map.owner(x, y) {
                        owned[t as usize] += 1;
                    }
                }
            }
            assert!(owned.iter().all(|&n| n > 50), "{owned:?}");
            // Without the option nothing is generated.
            let (plain, report) = normalize(raw, &NormalizeOptions::default());
            assert!(plain.sets.is_empty() && !report.fallback_uvs_generated);
        }
        // The option is ignored as soon as one triangle has UVs.
        let mut raw = samples::no_uv_tetra();
        raw.triangles[0].uvs = Some([Vec2::new(0.1, 0.1), Vec2::new(0.9, 0.1), Vec2::new(0.1, 0.9)]);
        let (mesh, report) = normalize(raw, &NormalizeOptions { generate_fallback_uvs: true, ..Default::default() });
        check_structure(&mesh);
        assert!(!report.fallback_uvs_generated && !report.no_uvs);
        assert_eq!(mesh.tri_flags.iter().filter(|f| f.contains(TriFlags::NO_UV)).count(), 3);
    }

    #[test]
    fn fallback_uvs_of_a_connected_surface_form_islands_that_strokes_can_cross() {
        // A sphere without UVs: the generated layout keeps neighbouring triangles together (few islands, shared UV
        // corners), never overlaps, mirrors nothing and gives every triangle texels at a modest resolution.
        let mut raw = samples::uv_sphere(32, 16, 1.0);
        for t in &mut raw.triangles {
            t.uvs = None;
        }
        let tris = raw.triangles.len() as u32;
        let opts = NormalizeOptions { generate_fallback_uvs: true, model_name: "ball".into(), ..Default::default() };
        let (mesh, report) = normalize(raw, &opts);
        check_structure(&mesh);
        assert!(report.fallback_uvs_generated);
        assert!(mesh.tri_flags.iter().all(|f| f.is_empty()));
        // Shared corners: far fewer UV-distinct vertices than three per triangle.
        assert!((mesh.positions.len() as u32) < tris * 3 / 2, "{} vertices for {tris} triangles", mesh.positions.len());
        let map = TexelMap::build(&mesh, TextureSetId(0), 1024, 1024, TexelMap::default_gutter(1024, 1024));
        assert_eq!(map.stats.overlapped_texels, 0);
        assert_eq!(map.stats.mirrored_tris, 0);
        assert!(map.stats.coverage > 0.3, "coverage {}", map.stats.coverage);
        // Texel density is uniform: equal 3D areas get equal UV areas within the box-projection stretch (<= 3x).
        let ratio: Vec<f64> = (0..mesh.tri_count())
            .map(|t| {
                let p = mesh.tri_positions(t).map(|v| v.as_dvec3());
                let a3 = (p[1] - p[0]).cross(p[2] - p[0]).length();
                signed_uv_area(mesh.tri_uvs(t)).abs() * 2.0 / a3
            })
            .collect();
        let (lo, hi) = ratio.iter().fold((f64::MAX, 0.0f64), |(lo, hi), &r| (lo.min(r), hi.max(r)));
        assert!(hi / lo <= 3.0 + 1e-6, "density varies {lo}..{hi}");
    }

    #[test]
    fn layout_images_of_all_sets_match_the_single_set_images() {
        for raw in [samples::two_material_model(), samples::seam_cube(1.0), samples::no_uv_tetra()] {
            let mesh = mesh_of(raw);
            let all = uv_layout_images(&mesh, 64);
            assert_eq!(all.len(), mesh.sets.len());
            for (s, image) in all.iter().enumerate() {
                assert_eq!(*image, uv_layout_image(&mesh, TextureSetId(s as u32), 64), "set {s}");
            }
        }
    }

    #[test]
    fn uv_hash_is_stable_and_sensitive() {
        let a = mesh_of(samples::two_material_model());
        let b = mesh_of(samples::two_material_model());
        assert_eq!(a.uv_hash, b.uv_hash);
        assert_ne!(a.uv_hash, 0);
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.uvs, b.uvs);

        // One changed UV coordinate.
        let mut raw = samples::two_material_model();
        raw.triangles[5].uvs.as_mut().unwrap()[1].x += 1e-4;
        assert_ne!(mesh_of(raw).uv_hash, a.uv_hash);
        // Same UVs, different set assignment.
        let mut raw = samples::two_material_model();
        raw.materials[1].texture = Some(0);
        let merged = mesh_of(raw);
        assert_eq!(merged.sets.len(), 1);
        assert_eq!(merged.uvs, a.uvs);
        assert_ne!(merged.uv_hash, a.uv_hash);
        // Positions are not part of the hash, names neither.
        let mut raw = samples::two_material_model();
        for t in &mut raw.triangles {
            for p in &mut t.positions {
                *p *= 2.0;
            }
        }
        raw.materials[0].name = "Renamed".into();
        assert_eq!(mesh_of(raw).uv_hash, a.uv_hash);
        // -0.0 and +0.0 are the same coordinate.
        let mut raw = samples::flat_quad();
        let plain = mesh_of(raw.clone());
        for t in &mut raw.triangles {
            for uv in t.uvs.as_mut().unwrap() {
                if uv.x == 0.0 {
                    uv.x = -0.0;
                }
            }
            for p in &mut t.positions {
                if p.z == 0.0 {
                    p.z = -0.0;
                }
            }
        }
        let negative_zero = mesh_of(raw);
        assert_eq!(negative_zero.uv_hash, plain.uv_hash);
        assert_eq!(negative_zero.positions.len(), 4);
        assert_eq!(negative_zero.wire_edges().len(), 5);
    }

    /// Pins the automatic UV layout of `IMPORTER_VERSION` 2 (section 6.1 item 4). If this fails, the generated layout
    /// or the normalisation changed: saved projects of models without UVs would open with their paint misplaced. Bump
    /// `IMPORTER_VERSION`, keep the version-2 path selectable by version, and only then add the new hash here.
    #[test]
    fn generated_uv_layout_of_importer_version_2_is_frozen() {
        assert_eq!(crate::import::IMPORTER_VERSION, 2, "a new importer version needs its own frozen hashes here");
        let opts = NormalizeOptions { generate_fallback_uvs: true, model_name: "rock".into(), ..Default::default() };
        let (tetra, report) = normalize(samples::no_uv_tetra(), &opts);
        assert!(report.fallback_uvs_generated);
        let mut sphere = samples::uv_sphere(32, 16, 1.0);
        for t in &mut sphere.triangles {
            t.uvs = None;
        }
        let (sphere, report) = normalize(sphere, &opts);
        assert!(report.fallback_uvs_generated);
        assert_eq!(
            [format!("{:016x}", tetra.uv_hash), format!("{:016x}", sphere.uv_hash)],
            ["e85730533e39fd3b", "6095e70cb3ed8972"]
        );
    }

    #[test]
    fn render_vertices_mirror_the_mesh() {
        let mesh = mesh_of(samples::two_material_model());
        check_structure(&mesh);
        let verts = mesh.render_vertices();
        assert_eq!(verts.len(), mesh.positions.len());
        for (i, v) in verts.iter().enumerate() {
            assert_eq!(v.pos, mesh.positions[i].to_array());
            assert_eq!(v.normal, mesh.normals[i].to_array());
            assert_eq!(v.uv, mesh.uvs[i].to_array());
        }
        // Vertices are shared inside a material: the sphere has far fewer vertices than triangle corners.
        let head = mesh.material_ranges[1].clone();
        assert_eq!(head.len(), 24 * 22);
        assert_eq!(mesh.positions.len(), 24 + 25 * 11 + 2 * 24);
        // No two vertices are identical.
        let mut keys: Vec<[u32; 9]> = (0..mesh.positions.len())
            .map(|i| {
                let (p, n, uv) = (bits3(mesh.positions[i]), bits3(mesh.normals[i]), mesh.uvs[i]);
                let material = (0..mesh.tri_count()).find(|&t| mesh.tri(t).contains(&(i as u32))).map(|t| mesh.tri_material[t as usize]);
                [p[0], p[1], p[2], n[0], n[1], n[2], uv.x.to_bits(), uv.y.to_bits(), material.unwrap()]
            })
            .collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), mesh.positions.len());
    }

    #[test]
    fn uv_layout_image_shows_the_islands() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let side = 256u32;
        let img = uv_layout_image(&mesh, TextureSetId(0), side);
        assert_eq!((img.width, img.height), (side, side));
        assert!(img.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "opaque");
        const ORANGE: [u8; 4] = [0xff, 0x9d, 0x2e, 0xff];
        const GREY: [u8; 4] = [0x8a, 0x8f, 0x98, 0xff];
        const DARK: [u8; 4] = [0x20, 0x24, 0x28, 0xff];
        let px = |uv: f32| (uv as f64 * (side - 1) as f64).round() as u32;
        for face in 0..6 {
            let [u0, u1, v0, v1] = samples::seam_cube_island(face);
            let (x0, x1, y0, y1) = (px(u0), px(u1), px(v0), px(v1));
            // All four borders are orange along their whole length.
            for x in x0..=x1 {
                assert_eq!(img.get(x, y0), ORANGE, "top border of island {face} at x = {x}");
                assert_eq!(img.get(x, y1), ORANGE, "bottom border of island {face}");
            }
            for y in y0..=y1 {
                assert_eq!(img.get(x0, y), ORANGE, "left border of island {face}");
                assert_eq!(img.get(x1, y), ORANGE, "right border of island {face}");
            }
            // The diagonal is grey, the rest of the island and the space between the islands is background.
            let inside: Vec<[u8; 4]> =
                (y0 + 1..y1).flat_map(|y| (x0 + 1..x1).map(move |x| (x, y))).map(|(x, y)| img.get(x, y)).collect();
            assert!(inside.iter().all(|p| *p == GREY || *p == DARK));
            assert!(inside.iter().filter(|p| **p == GREY).count() >= (y1 - y0 - 1) as usize);
            assert_eq!(img.get(x0 - 2, y0 - 2), DARK);
            assert_eq!(img.get(x1 + 2, y1 + 2), DARK);
        }
        assert_eq!(hash_pixels(&img), hash_pixels(&uv_layout_image(&mesh, TextureSetId(0), side)));

        // Boundaries are cyan; UV = 1 maps to the last pixel instead of wrapping to the first.
        let quad = mesh_of(samples::flat_quad());
        let img = uv_layout_image(&quad, TextureSetId(0), 64);
        const CYAN: [u8; 4] = [0x35, 0xd0, 0xe0, 0xff];
        for i in 0..64 {
            assert_eq!(img.get(i, 0), CYAN);
            assert_eq!(img.get(i, 63), CYAN);
            assert_eq!(img.get(0, i), CYAN);
            assert_eq!(img.get(63, i), CYAN);
        }
        assert_eq!(img.get(31, 32), GREY);
        // Wrapping UVs are folded into the image; an unknown set or an empty image is not an error.
        let wrapped = mesh_of(samples::wrapped_quad());
        let img = uv_layout_image(&wrapped, TextureSetId(0), 64);
        assert!(img.data.as_chunks::<4>().0.contains(&CYAN));
        assert!(uv_layout_image(&mesh, TextureSetId(7), 32).data.as_chunks::<4>().0.iter().all(|p| *p == DARK));
        assert_eq!(uv_layout_image(&mesh, TextureSetId(0), 0).byte_len(), 0);
        assert_eq!(uv_layout_image(&mesh, TextureSetId(0), 1).data, ORANGE);
    }

    // ----- objects-tabs WP1: step 9 (objects) -----

    /// `RawObjects` over `n` raw triangles: object `of(i)` for triangle `i`, names as given.
    fn raw_objects(n: usize, names: &[&str], of: impl FnMut(usize) -> u32) -> RawObjects {
        RawObjects { tri_object: (0..n).map(of).collect(), names: names.iter().map(|s| (*s).to_owned()).collect() }
    }

    fn named(model: &str) -> NormalizeOptions {
        NormalizeOptions { model_name: model.to_owned(), ..NormalizeOptions::default() }
    }

    fn names_of(mesh: &MeshData) -> Vec<&str> {
        mesh.objects.iter().map(|o| o.name.as_str()).collect()
    }

    /// Every object's triangle count and bounds agree with `tri_object`.
    fn check_objects(mesh: &MeshData) {
        assert_eq!(mesh.tri_object.len(), mesh.tri_count() as usize);
        assert!(!mesh.objects.is_empty());
        assert!(mesh.tri_object.iter().all(|&o| o < mesh.object_count()));
        assert_eq!(mesh.objects.iter().map(|o| o.tri_count).sum::<u32>(), mesh.tri_count());
        for (i, info) in mesh.objects.iter().enumerate() {
            let tris: Vec<u32> = (0..mesh.tri_count()).filter(|&t| mesh.tri_object_of(t) == i as u32).collect();
            assert_eq!(info.tri_count as usize, tris.len(), "object {i}");
            assert!(info.tri_count > 0, "object {i} has no triangle");
            let points: Vec<Vec3> = tris.iter().flat_map(|&t| mesh.tri_positions(t)).collect();
            assert_eq!(info.bounds, Aabb::from_points(&points), "object {i} bounds");
            assert!(!info.name.is_empty());
            assert_eq!(mesh.objects.iter().filter(|o| o.name == info.name).count(), 1, "object names are unique");
        }
    }

    #[test]
    fn objects_follow_their_triangles_through_the_sort() {
        // Two materials x two objects, interleaved; every fifth triangle has no UVs (sorted last in its material).
        let mut raw = RawMesh { triangles: Vec::new(), materials: vec![mat("a", Some(0)), mat("b", Some(1))] };
        for i in 0..40 {
            raw.triangles.push(simple(i as f32 * 2.0, (i % 2) as u32, i % 5 != 0));
        }
        let objects = raw_objects(40, &["Left", "Right"], |i| ((i / 2) % 2) as u32);
        let expected: HashMap<[u32; 3], u32> =
            raw.triangles.iter().enumerate().map(|(i, t)| (bits3(t.positions[0]), objects.tri_object[i])).collect();
        let (mesh, _) = normalize_with_objects(raw, &objects, &NormalizeOptions::default());
        check_structure(&mesh);
        check_objects(&mesh);
        assert_eq!(names_of(&mesh), ["Left", "Right"]);
        // The triangles were reordered (by material, NO_UV last); each one keeps the object of its raw triangle.
        assert_ne!(mesh.tri_object, objects.tri_object, "the sort moved triangles");
        for t in 0..mesh.tri_count() {
            assert_eq!(mesh.tri_object_of(t), expected[&bits3(mesh.tri_positions(t)[0])], "triangle {t}");
        }
    }

    #[test]
    fn object_data_never_changes_the_mesh() {
        let fixtures = [samples::two_material_model(), samples::seam_cube(1.0), samples::soup_sets(5, 600, 3), samples::uv_sphere(16, 8, 1.0)];
        for (f, raw) in fixtures.into_iter().enumerate() {
            let mut rng = XorShift::new(f as u64 + 3);
            // Indices past the names (99) count as object 0.
            let objects = raw_objects(raw.triangles.len(), &["a", "", "b", "a", "c"], |_| match rng.next_u32() % 7 {
                6 => 99,
                k => k % 5,
            });
            let (plain, plain_report) = normalize(raw.clone(), &named("model"));
            let (with, with_report) = normalize_with_objects(raw, &objects, &named("model"));
            assert_eq!(plain_report, with_report, "fixture {f}");
            assert_eq!(plain.positions, with.positions, "fixture {f}");
            assert_eq!(plain.normals, with.normals, "fixture {f}");
            assert_eq!(plain.uvs, with.uvs, "fixture {f}");
            assert_eq!(plain.indices, with.indices, "fixture {f}");
            assert_eq!(plain.tri_material, with.tri_material, "fixture {f}");
            assert_eq!(plain.tri_flags, with.tri_flags, "fixture {f}");
            assert_eq!(plain.material_ranges, with.material_ranges, "fixture {f}");
            assert_eq!(plain.sets, with.sets, "fixture {f}");
            assert_eq!(plain.bounds, with.bounds, "fixture {f}");
            assert_eq!(plain.uv_hash, with.uv_hash, "fixture {f}");
            check_objects(&with);
            assert!(with.objects.len() > 1, "fixture {f}: several objects");
            // The wireframe is the same apart from the object of each entry.
            let strip = |m: &MeshData| m.wire_edges().into_iter().map(|e| format!("{:?}", WireEdge { object: 0, ..e })).collect::<Vec<_>>();
            assert_eq!(strip(&plain), strip(&with), "fixture {f}");
        }
    }

    #[test]
    fn unnamed_and_duplicate_names() {
        let raw = RawMesh { triangles: (0..4).map(|i| simple(i as f32 * 2.0, 0, true)).collect(), materials: vec![mat("m", Some(0))] };
        let objects = raw_objects(4, &["", "Pawn", "Pawn", ""], |i| i as u32);
        let (mesh, _) = normalize_with_objects(raw.clone(), &objects, &named("chess"));
        assert_eq!(names_of(&mesh), ["Object 1", "Pawn", "Pawn_2", "Object 4"]);
        check_objects(&mesh);
        // A single unnamed object takes the model name, "Model" when there is none.
        let one = raw_objects(4, &[""], |_| 0);
        assert_eq!(names_of(&normalize_with_objects(raw.clone(), &one, &named("chess")).0), ["chess"]);
        assert_eq!(names_of(&normalize_with_objects(raw.clone(), &one, &named("")).0), ["Model"]);
        // A name that already ends in a suffix is not confused with a generated one.
        let tricky = raw_objects(4, &["A_2", "A", "A", "A"], |i| i as u32);
        assert_eq!(names_of(&normalize_with_objects(raw, &tricky, &named("")).0), ["A_2", "A", "A_3", "A_4"]);
    }

    #[test]
    fn objects_without_triangles_disappear() {
        let mut raw = RawMesh { triangles: (0..6).map(|i| simple(i as f32 * 2.0, 0, true)).collect(), materials: vec![mat("m", Some(0))] };
        // Object "Gone" (triangles 2 and 3) is all degenerate; "Unused" has no triangle at all.
        for t in &mut raw.triangles[2..4] {
            t.positions = [t.positions[0]; 3];
        }
        let objects = raw_objects(6, &["Unused", "First", "Gone", "Last"], |i| [1, 1, 2, 2, 3, 3][i]);
        let (mesh, report) = normalize_with_objects(raw, &objects, &NormalizeOptions::default());
        assert_eq!(report.dropped_degenerate, 2);
        assert_eq!(names_of(&mesh), ["First", "Last"]);
        assert_eq!(mesh.tri_object, [0, 0, 1, 1]);
        check_objects(&mesh);
    }

    #[test]
    fn no_object_data_is_one_object() {
        let mesh = mesh_of(samples::two_material_model());
        assert_eq!(mesh.objects.len(), 1);
        assert_eq!(mesh.objects[0].name, "Model");
        assert_eq!(mesh.objects[0].tri_count, mesh.tri_count());
        assert_eq!(mesh.objects[0].bounds, mesh.bounds);
        assert!(mesh.tri_object.iter().all(|&o| o == 0));
        assert_eq!(mesh.tri_object.len(), mesh.tri_count() as usize);
        assert_eq!(normalize(samples::flat_quad(), &named("quad")).0.objects[0].name, "quad");
        // An empty mesh has no object.
        let (empty, _) = normalize(RawMesh::default(), &NormalizeOptions::default());
        assert!(empty.objects.is_empty() && empty.tri_object.is_empty());
    }

    #[test]
    fn wire_edges_name_their_object() {
        // A quad split into one triangle per object (the diagonal is shared), plus a lone triangle of object 2.
        let quad = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let uv = |p: [f32; 3]| [p[0], 1.0 - p[1]];
        let corner = |idx: [usize; 3]| tri(idx.map(|i| quad[i]), Some(idx.map(|i| uv(quad[i]))), 0);
        let mut raw = RawMesh { triangles: vec![corner([0, 1, 2]), corner([0, 2, 3])], materials: vec![mat("m", Some(0))] };
        raw.triangles.push(simple(5.0, 0, true));
        let objects = raw_objects(3, &["B", "A", "C"], |i| i as u32);
        let (mesh, _) = normalize_with_objects(raw, &objects, &NormalizeOptions::default());
        let edges = mesh.wire_edges();
        assert_eq!(edges.len(), 8, "4 quad sides, the diagonal, 3 sides of the lone triangle");
        for e in &edges {
            let on_quad = e.a[0] <= 1.0 && e.b[0] <= 1.0;
            if !on_quad {
                assert_eq!(e.object, 2, "{e:?}");
                assert_eq!(e.kind, EdgeKind::Boundary);
                continue;
            }
            let diagonal = e.a == [0.0, 0.0, 0.0] && e.b == [1.0, 1.0, 0.0];
            if diagonal {
                // Shared by both objects: one Interior entry, owned by one of them (the smaller index).
                assert_eq!((e.kind, e.object), (EdgeKind::Interior, 0));
            } else {
                // Sides 0-1 and 1-2 belong to the first triangle, 2-3 and 3-0 to the second.
                let first = (e.a[1] == 0.0 && e.b[1] == 0.0) || (e.a[0] == 1.0 && e.b[0] == 1.0);
                assert_eq!(e.object, if first { 0 } else { 1 }, "{e:?}");
                assert_eq!(e.kind, EdgeKind::Boundary);
            }
        }
    }
}
