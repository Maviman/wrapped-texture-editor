//! GPU resources of one document (ARCHITECTURE.md 8.4): mesh buffers, per-material draw ranges and untextured bind
//! groups, wire instance buffers and one `SetTexture` per texture set, kept in sync with the CPU composites.

use std::ops::Range;
use std::time::{Duration, Instant};

use eframe::wgpu;
use eframe::wgpu::util::DeviceExt;
#[cfg(test)]
use wte_core::TextureSetId;
use wte_core::{Document, EdgeKind, MeshData, WireEdge};

use crate::gpu::renderer::LineInstance;
use crate::gpu::{Gpu, Renderer, SetTexture};

/// Minimum time between two mip regenerations of one set while an interaction is running.
pub(crate) const MIP_INTERVAL: Duration = Duration::from_millis(100);

/// Interior wire colour in the 3D views (35% alpha).
const WIRE_INTERIOR_3D: [u8; 4] = [12, 12, 14, 89];
/// Interior wire colour in the UV view (35% alpha).
const WIRE_INTERIOR_UV: [u8; 4] = [235, 238, 245, 89];
const WIRE_SEAM: [u8; 4] = [255, 157, 46, 255];
const WIRE_BOUNDARY: [u8; 4] = [53, 208, 224, 255];

/// The mip throttling rule of section 8.4.
pub(crate) fn should_regenerate_mips(stale: bool, force: bool, interacting: bool, since_last: Duration) -> bool {
    stale && (force || !interacting || since_last >= MIP_INTERVAL)
}

/// Which group-1 bind group a draw uses.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum DrawGroup {
    /// The texture of set `n`.
    Set(usize),
    /// The untextured group of material `n`.
    Untextured(usize),
}

/// One `draw_indexed` call: a triangle range and its bind group.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Draw {
    pub tris: Range<u32>,
    pub group: DrawGroup,
}

/// Per material: the UV-mapped range with the set texture, then the NO_UV range (or the whole material when it has no
/// set) untextured. Empty ranges are skipped.
pub(crate) fn build_draws(mesh: &MeshData) -> Vec<Draw> {
    let mut draws = Vec::new();
    let tri_count = mesh.tri_count();
    let clip = |r: &Range<u32>| r.start.min(tri_count)..r.end.min(tri_count);
    for (m, material) in mesh.materials.iter().enumerate() {
        let whole = mesh.material_ranges.get(m).map(clip).unwrap_or(0..0);
        match material.texture_set {
            Some(set) => {
                let uv = mesh.material_uv_ranges.get(m).map(clip).unwrap_or(0..0);
                let nouv = mesh.material_nouv_ranges.get(m).map(clip).unwrap_or(0..0);
                if !uv.is_empty() {
                    draws.push(Draw { tris: uv, group: DrawGroup::Set(set.index()) });
                }
                if !nouv.is_empty() {
                    draws.push(Draw { tris: nouv, group: DrawGroup::Untextured(m) });
                }
            }
            None if !whole.is_empty() => draws.push(Draw { tris: whole, group: DrawGroup::Untextured(m) }),
            None => {}
        }
    }
    draws
}

fn wire_color(kind: EdgeKind, interior: [u8; 4]) -> [u8; 4] {
    match kind {
        EdgeKind::Interior => interior,
        EdgeKind::Seam => WIRE_SEAM,
        EdgeKind::Boundary => WIRE_BOUNDARY,
    }
}

/// 3D wire instances: (all edges, seams + boundaries only). Interior edges come first so seams draw on top.
pub(crate) fn wire_instances_3d(edges: &[WireEdge]) -> (Vec<LineInstance>, Vec<LineInstance>) {
    let to_instance = |e: &WireEdge| LineInstance { a: e.a, b: e.b, color: wire_color(e.kind, WIRE_INTERIOR_3D) };
    let interior = edges.iter().filter(|e| e.kind == EdgeKind::Interior).map(to_instance);
    let seams: Vec<LineInstance> = edges.iter().filter(|e| e.kind != EdgeKind::Interior).map(to_instance).collect();
    let all = interior.chain(seams.iter().copied()).collect();
    (all, seams)
}

/// UV wire instances of one set: `(uv_a, 0)` -> `(uv_b, 0)`, interior edges first.
#[cfg(test)]
pub(crate) fn wire_instances_uv(edges: &[WireEdge], set: TextureSetId) -> Vec<LineInstance> {
    wire_instances_uv_all(edges, set.index() + 1).swap_remove(set.index())
}

/// UV wire instances of every set `0..set_count` in one pass over the edges (a document with many sets used to scan
/// all edges once per set): per set `(uv_a, 0)` -> `(uv_b, 0)`, interior edges first, each group in edge order.
/// Edges of other sets (or none) are ignored.
pub(crate) fn wire_instances_uv_all(edges: &[WireEdge], set_count: usize) -> Vec<Vec<LineInstance>> {
    let to_instance = |e: &WireEdge| LineInstance {
        a: [e.uv_a[0], e.uv_a[1], 0.0],
        b: [e.uv_b[0], e.uv_b[1], 0.0],
        color: wire_color(e.kind, WIRE_INTERIOR_UV),
    };
    let mut interior: Vec<Vec<LineInstance>> = vec![Vec::new(); set_count];
    let mut seams: Vec<Vec<LineInstance>> = vec![Vec::new(); set_count];
    for e in edges {
        let Some(set) = e.set.map(|s| s.index()).filter(|&s| s < set_count) else { continue };
        let bucket = if e.kind == EdgeKind::Interior { &mut interior[set] } else { &mut seams[set] };
        bucket.push(to_instance(e));
    }
    interior
        .into_iter()
        .zip(seams)
        .map(|(mut all, seams)| {
            all.extend(seams);
            all
        })
        .collect()
}

/// The CPU-side geometry of a scene: render vertices and the wire instance lists (3D all, 3D seams, UV per set).
struct SceneGeometry {
    vertices: Vec<wte_core::RenderVertex>,
    wire_all: Vec<LineInstance>,
    wire_seams: Vec<LineInstance>,
    wire_uv: Vec<Vec<LineInstance>>,
}

impl SceneGeometry {
    /// Builds the geometry on worker threads (vertices on one, edges and wire lists on another) while `meanwhile` runs
    /// on the calling thread; for a large mesh this overlaps the edge sort with the texture uploads.
    fn build_with<R>(mesh: &MeshData, set_count: usize, meanwhile: impl FnOnce() -> R) -> (SceneGeometry, R) {
        std::thread::scope(|scope| {
            let vertices = scope.spawn(|| mesh.render_vertices());
            let wires = scope.spawn(|| {
                let edges = mesh.wire_edges();
                let (all, seams) = wire_instances_3d(&edges);
                (all, seams, wire_instances_uv_all(&edges, set_count))
            });
            let r = meanwhile();
            let vertices = vertices.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
            let (wire_all, wire_seams, wire_uv) = wires.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
            (SceneGeometry { vertices, wire_all, wire_seams, wire_uv }, r)
        })
    }
}

/// Bytes of the mesh vertex buffer (one `RenderVertex` per mesh vertex), of the index buffer, and the most the 3D
/// wireframe instance buffer can need (one `LineInstance` per triangle side).
pub(crate) fn mesh_buffer_bytes(mesh: &MeshData) -> (u64, u64, u64) {
    let vertices = mesh.positions.len() as u64 * size_of::<wte_core::RenderVertex>() as u64;
    let indices = mesh.indices.len() as u64 * size_of::<u32>() as u64;
    let wire = mesh.indices.len() as u64 * size_of::<LineInstance>() as u64;
    (vertices, indices, wire)
}

/// `Err` (a sentence for the user) when the mesh's vertex or index buffer exceeds the device's `max_buffer_size`:
/// creating it would be a wgpu validation error, which panics. The wireframe alone being too large is no error,
/// `GpuScene::new` leaves the wireframe out then.
pub fn check_mesh_fits(mesh: &MeshData, limits: wgpu::Limits) -> Result<(), String> {
    let (vertices, indices, _) = mesh_buffer_bytes(mesh);
    let largest = vertices.max(indices);
    if largest > limits.max_buffer_size {
        let mib = |b: u64| b.div_ceil(1 << 20);
        return Err(format!(
            "The model is too large for this graphics adapter: {} triangles and {} vertices need a {} MiB buffer, the \
             adapter allows {} MiB. Reduce the model in a 3D program first.",
            mesh.tri_count(),
            mesh.positions.len(),
            mib(largest),
            mib(limits.max_buffer_size)
        ));
    }
    Ok(())
}

/// Vertex buffer of line instances, `None` when there are none or they exceed the device's `max_buffer_size` (the
/// wireframe is then not drawn; logged).
fn instance_buffer(gpu: &Gpu, label: &str, instances: &[LineInstance]) -> Option<(wgpu::Buffer, u32)> {
    if instances.is_empty() {
        return None;
    }
    let bytes = std::mem::size_of_val(instances) as u64;
    if bytes > gpu.device.limits().max_buffer_size {
        log::warn!("{label}: {} line instances ({bytes} bytes) exceed the buffer size limit; not drawn", instances.len());
        return None;
    }
    let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(instances),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Some((buffer, instances.len() as u32))
}

/// One texture set on the GPU and its mip bookkeeping.
struct SceneSet {
    texture: SetTexture,
    mips_stale: bool,
    last_mips: Instant,
}

/// GPU resources of one document: mesh buffers, material ranges, wire instance buffers and the per-set textures.
pub struct GpuScene {
    /// `Session::scene_generation` this scene was built for.
    pub generation: u64,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    draws: Vec<Draw>,
    /// Per material: `(uniform buffer, untextured bind group)`.
    untextured: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    wire_all: Option<(wgpu::Buffer, u32)>,
    wire_seams: Option<(wgpu::Buffer, u32)>,
    /// Per set: UV wireframe instances.
    wire_uv: Vec<Option<(wgpu::Buffer, u32)>>,
    sets: Vec<SceneSet>,
}

impl GpuScene {
    /// Builds all GPU resources of `doc`. The CPU geometry (render vertices, wire edges and instance lists) is built on
    /// worker threads while the set textures are created and uploaded here; the time of each part is logged.
    pub fn new(gpu: &Gpu, renderer: &Renderer, doc: &mut Document, generation: u64) -> GpuScene {
        let start = Instant::now();
        doc.recomposite_all();
        let mesh = std::sync::Arc::clone(&doc.mesh);
        let composited = start.elapsed();

        let set_count = doc.sets.len();
        let (geometry, (sets, textures_time)) = SceneGeometry::build_with(&mesh, set_count, || {
            let t = Instant::now();
            let now = Instant::now();
            let mut sets = Vec::with_capacity(set_count);
            for id in doc.set_ids() {
                // The whole composite is uploaded below; earlier dirty rects are covered by it.
                doc.take_gpu_dirty(id);
                let set = &doc.sets[id.index()];
                let texture = SetTexture::new(gpu, renderer, [set.width, set.height], &set.composite().data);
                sets.push(SceneSet { texture, mips_stale: true, last_mips: now });
            }
            (sets, t.elapsed())
        });
        let geometry_ready = start.elapsed();

        let index_count = mesh.indices.len() - mesh.indices.len() % 3;
        let too_large = check_mesh_fits(&mesh, gpu.device.limits());
        let (vertices, indices) = if geometry.vertices.is_empty() || index_count == 0 {
            (None, None)
        } else if let Err(e) = too_large {
            // The file flows refuse such a document before it gets here (io_flows); never a wgpu panic.
            log::error!("mesh not uploaded: {e}");
            (None, None)
        } else {
            let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh_vertices"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh_indices"),
                contents: bytemuck::cast_slice(&mesh.indices[..index_count]),
                usage: wgpu::BufferUsages::INDEX,
            });
            (Some(vb), Some(ib))
        };

        let untextured =
            mesh.materials.iter().map(|m| renderer.create_untextured_group(gpu, m.base_color)).collect();
        let wire_all = instance_buffer(gpu, "wire_all", &geometry.wire_all);
        let wire_seams = instance_buffer(gpu, "wire_seams", &geometry.wire_seams);
        let wire_uv = geometry.wire_uv.iter().map(|w| instance_buffer(gpu, "wire_uv", w)).collect();

        let mut scene = GpuScene {
            generation,
            vertices,
            indices,
            draws: build_draws(&mesh),
            untextured,
            wire_all,
            wire_seams,
            wire_uv,
            sets,
        };
        let buffers_ready = start.elapsed();
        scene.regenerate_stale(gpu, renderer, |_| true);
        log::info!(
            "GPU scene built in {:.0} ms ({} triangles, {} set(s)): composite {:.0} ms, textures {:.0} ms in parallel with \
             geometry (until {:.0} ms), buffers {:.0} ms, mips {:.0} ms",
            start.elapsed().as_secs_f64() * 1000.0,
            mesh.tri_count(),
            set_count,
            composited.as_secs_f64() * 1000.0,
            textures_time.as_secs_f64() * 1000.0,
            geometry_ready.as_secs_f64() * 1000.0,
            (buffers_ready - geometry_ready).as_secs_f64() * 1000.0,
            (start.elapsed() - buffers_ready).as_secs_f64() * 1000.0,
        );
        scene
    }

    /// `doc.recomposite_all()`; for every set: upload each `take_gpu_dirty` rect and mark its mips stale; regenerate
    /// the mips of stale sets according to the throttling rule of section 8.4 (`force_mips` = regenerate now); one
    /// submit.
    pub fn sync(&mut self, gpu: &Gpu, renderer: &Renderer, doc: &mut Document, force_mips: bool) {
        doc.recomposite_all();
        for id in doc.set_ids() {
            let rects = doc.take_gpu_dirty(id);
            let Some(set) = doc.set(id) else { continue };
            let size = [set.width.max(1), set.height.max(1)];
            let composite = &set.composite().data;
            match self.sets.get_mut(id.index()) {
                Some(entry) if entry.texture.size() == size => {
                    for r in rects.iter().filter(|r| !r.is_empty()) {
                        entry.texture.upload(&gpu.queue, composite, *r);
                        entry.mips_stale = true;
                    }
                }
                Some(entry) => {
                    // Defensive: a set never changes size, but if it did the texture is rebuilt from scratch.
                    entry.texture.destroy();
                    entry.texture = SetTexture::new(gpu, renderer, size, composite);
                    entry.mips_stale = true;
                }
                None => {
                    log::error!("texture set {} has no GPU texture (scene built for another document?)", id.0);
                }
            }
        }
        let interacting = doc.stroke_active() || doc.has_pending_edit();
        let now = Instant::now();
        self.regenerate_stale(gpu, renderer, |s| {
            should_regenerate_mips(s.mips_stale, force_mips, interacting, now.saturating_duration_since(s.last_mips))
        });
    }

    /// True when level 0 of any set is newer than its mip chain.
    pub fn mips_stale(&self) -> bool {
        self.sets.iter().any(|s| s.mips_stale)
    }

    /// Regenerates the mip chains of the sets `pick` selects, in one encoder and one submit.
    fn regenerate_stale(&mut self, gpu: &Gpu, renderer: &Renderer, pick: impl Fn(&SceneSet) -> bool) {
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        let now = Instant::now();
        for set in self.sets.iter_mut().filter(|s| pick(s)) {
            let enc = encoder.get_or_insert_with(|| {
                gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mips") })
            });
            set.texture.regenerate_mips(gpu, renderer, enc);
            set.mips_stale = false;
            set.last_mips = now;
        }
        if let Some(encoder) = encoder {
            gpu.queue.submit([encoder.finish()]);
        }
    }

    /// Vertex and index buffer of the whole mesh (`None` for an empty mesh).
    pub(crate) fn mesh_buffers(&self) -> Option<(&wgpu::Buffer, &wgpu::Buffer)> {
        Some((self.vertices.as_ref()?, self.indices.as_ref()?))
    }

    /// The draw calls of the mesh, in material order.
    pub(crate) fn draws(&self) -> &[Draw] {
        &self.draws
    }

    /// The GPU texture of set `index`.
    pub(crate) fn set_texture(&self, index: usize) -> Option<&SetTexture> {
        self.sets.get(index).map(|s| &s.texture)
    }

    /// The untextured bind group of material `index`.
    pub(crate) fn untextured_group(&self, index: usize) -> Option<&wgpu::BindGroup> {
        self.untextured.get(index).map(|(_, g)| g)
    }

    /// All 3D wire instances (interior, seams, boundaries).
    pub(crate) fn wire_all(&self) -> Option<(&wgpu::Buffer, u32)> {
        self.wire_all.as_ref().map(|(b, n)| (b, *n))
    }

    /// Seam and boundary 3D wire instances.
    pub(crate) fn wire_seams(&self) -> Option<(&wgpu::Buffer, u32)> {
        self.wire_seams.as_ref().map(|(b, n)| (b, *n))
    }

    /// UV wire instances of set `index`.
    pub(crate) fn wire_uv(&self, index: usize) -> Option<(&wgpu::Buffer, u32)> {
        self.wire_uv.get(index)?.as_ref().map(|(b, n)| (b, *n))
    }
}

#[cfg(test)]
mod tests {
    use wte_core::{MaterialSlot, NormalizeOptions, RawMaterial, RawMesh, RawTriangle};

    use super::*;

    #[test]
    fn mip_throttling_rule() {
        let short = Duration::from_millis(10);
        let long = MIP_INTERVAL;
        // Nothing stale: never.
        assert!(!should_regenerate_mips(false, true, false, long));
        // Forced: always.
        assert!(should_regenerate_mips(true, true, true, short));
        // Interaction over: at once.
        assert!(should_regenerate_mips(true, false, false, short));
        // During an interaction: at most every 100 ms.
        assert!(!should_regenerate_mips(true, false, true, short));
        assert!(should_regenerate_mips(true, false, true, long));
    }

    fn tri(material: u32, uvs: bool, offset: f32) -> RawTriangle {
        RawTriangle {
            positions: [
                glam::Vec3::new(offset, 0.0, 0.0),
                glam::Vec3::new(offset + 1.0, 0.0, 0.0),
                glam::Vec3::new(offset, 1.0, 0.0),
            ],
            normals: None,
            uvs: uvs.then_some([glam::Vec2::new(0.1, 0.1), glam::Vec2::new(0.9, 0.1), glam::Vec2::new(0.1, 0.9)]),
            material,
        }
    }

    fn material(name: &str, texture: Option<u32>) -> RawMaterial {
        RawMaterial { name: name.to_owned(), base_color: [0.5, 0.25, 1.0, 1.0], double_sided: false, texture }
    }

    #[test]
    fn draws_split_materials_into_textured_and_untextured_ranges() {
        // Material 0: textured, one UV triangle and one NO_UV triangle; material 1: no texture.
        let raw = RawMesh {
            triangles: vec![tri(0, true, 0.0), tri(0, false, 2.0), tri(1, true, 4.0)],
            materials: vec![material("a", Some(0)), material("b", None)],
        };
        let opts = NormalizeOptions { model_name: "t".to_owned(), texture_names: vec!["tex".to_owned()], ..Default::default() };
        let (mesh, _) = wte_core::normalize(raw, &opts);
        let draws = build_draws(&mesh);
        let total: u32 = draws.iter().map(|d| d.tris.len() as u32).sum();
        assert_eq!(total, mesh.tri_count(), "every triangle is drawn exactly once");
        for d in &draws {
            assert!(!d.tris.is_empty());
            match d.group {
                DrawGroup::Set(s) => {
                    assert!(s < mesh.sets.len());
                    for t in d.tris.clone() {
                        assert_eq!(mesh.tri_set(t).map(|s| s.index()), Some(s), "textured draw holds a set-less tri");
                    }
                }
                DrawGroup::Untextured(m) => {
                    for t in d.tris.clone() {
                        assert_eq!(mesh.tri_material[t as usize] as usize, m);
                        assert!(mesh.tri_set(t).is_none(), "untextured draw holds a UV-mapped tri");
                    }
                }
            }
        }
        assert!(draws.iter().any(|d| matches!(d.group, DrawGroup::Set(_))));
        assert!(draws.iter().any(|d| matches!(d.group, DrawGroup::Untextured(_))));
    }

    #[test]
    fn draws_of_a_set_less_material_cover_the_whole_material() {
        let mut mesh = wte_core::normalize(
            RawMesh { triangles: vec![tri(0, true, 0.0)], materials: vec![material("a", Some(0))] },
            &NormalizeOptions { model_name: "t".to_owned(), texture_names: vec!["tex".to_owned()], ..Default::default() },
        )
        .0;
        // Detach the set: the whole range is drawn untextured even if the uv range was not emptied.
        mesh.materials[0] = MaterialSlot { texture_set: None, ..mesh.materials[0].clone() };
        assert_eq!(build_draws(&mesh), vec![Draw { tris: 0..1, group: DrawGroup::Untextured(0) }]);
    }

    fn edge(kind: EdgeKind, set: Option<u32>, x: f32) -> WireEdge {
        WireEdge {
            a: [x, 0.0, 0.0],
            b: [x, 1.0, 0.0],
            uv_a: [x * 0.1, 0.0],
            uv_b: [x * 0.1, 0.5],
            kind,
            set: set.map(TextureSetId),
            object: 0,
        }
    }

    #[test]
    fn wire_instances_order_and_filter() {
        let edges = vec![
            edge(EdgeKind::Interior, Some(0), 1.0),
            edge(EdgeKind::Seam, Some(0), 2.0),
            edge(EdgeKind::Boundary, Some(0), 3.0),
            edge(EdgeKind::Interior, Some(1), 4.0),
            edge(EdgeKind::Seam, Some(1), 5.0),
            edge(EdgeKind::Boundary, None, 6.0),
        ];
        let (all, seams) = wire_instances_3d(&edges);
        assert_eq!(all.len(), 6);
        assert_eq!(seams.len(), 4);
        // Interior first, then seams/boundaries; alpha 35% for interior.
        assert!(all[..2].iter().all(|i| i.color == WIRE_INTERIOR_3D));
        assert!(all[2..].iter().all(|i| i.color == WIRE_SEAM || i.color == WIRE_BOUNDARY));
        assert_eq!(WIRE_INTERIOR_3D[3], (0.35f32 * 255.0).round() as u8);
        assert!(seams.iter().all(|i| i.color != WIRE_INTERIOR_3D));

        let uv0 = wire_instances_uv(&edges, TextureSetId(0));
        assert_eq!(uv0.len(), 3);
        assert_eq!(uv0[0].color, WIRE_INTERIOR_UV);
        assert_eq!(uv0[0].a, [0.1, 0.0, 0.0]);
        assert_eq!(uv0[0].b, [0.1, 0.5, 0.0]);
        assert_eq!(wire_instances_uv(&edges, TextureSetId(1)).len(), 2);
        assert!(wire_instances_uv(&edges, TextureSetId(7)).is_empty());
    }

    /// The reference of `wire_instances_uv_all`: one filter pass over all edges per set (the former per-set build).
    fn wire_uv_per_set(edges: &[WireEdge], set: TextureSetId) -> Vec<LineInstance> {
        let of_set = || edges.iter().filter(move |e| e.set == Some(set));
        of_set()
            .filter(|e| e.kind == EdgeKind::Interior)
            .chain(of_set().filter(|e| e.kind != EdgeKind::Interior))
            .map(|e| LineInstance { a: [e.uv_a[0], e.uv_a[1], 0.0], b: [e.uv_b[0], e.uv_b[1], 0.0], color: wire_color(e.kind, WIRE_INTERIOR_UV) })
            .collect()
    }

    #[test]
    fn one_pass_uv_wires_match_the_per_set_filter_and_the_geometry_threads_match_the_serial_build() {
        let doc = Document::sample(wte_core::SampleKind::TwoMaterial);
        let edges = doc.mesh.wire_edges();
        let sets = doc.sets.len();
        assert!(sets >= 2);
        let all = wire_instances_uv_all(&edges, sets);
        assert_eq!(all.len(), sets);
        let bytes = |v: &[LineInstance]| bytemuck::cast_slice::<LineInstance, u8>(v).to_vec();
        for (i, got) in all.iter().enumerate() {
            let want = wire_uv_per_set(&edges, TextureSetId(i as u32));
            assert!(!want.is_empty());
            assert_eq!(bytes(got), bytes(&want), "set {i}");
        }
        // Edges of sets beyond the count are ignored, not a panic.
        assert_eq!(wire_instances_uv_all(&edges, 1).len(), 1);

        let (geometry, marker) = SceneGeometry::build_with(&doc.mesh, sets, || 7);
        assert_eq!(marker, 7);
        let (all3d, seams3d) = wire_instances_3d(&edges);
        assert_eq!(bytes(&geometry.wire_all), bytes(&all3d));
        assert_eq!(bytes(&geometry.wire_seams), bytes(&seams3d));
        assert_eq!(geometry.wire_uv.len(), sets);
        assert_eq!(
            bytemuck::cast_slice::<wte_core::RenderVertex, u8>(&geometry.vertices),
            bytemuck::cast_slice::<wte_core::RenderVertex, u8>(&doc.mesh.render_vertices())
        );
    }

    #[test]
    fn meshes_beyond_the_buffer_limit_are_refused_before_any_buffer_is_made() {
        let doc = Document::sample(wte_core::SampleKind::Sphere);
        let (vertices, indices, wire) = mesh_buffer_bytes(&doc.mesh);
        assert_eq!(vertices, doc.mesh.positions.len() as u64 * 32, "RenderVertex is 32 bytes");
        assert_eq!(indices, doc.mesh.indices.len() as u64 * 4);
        assert_eq!(wire, doc.mesh.indices.len() as u64 * size_of::<LineInstance>() as u64);
        // wgpu's default limit (256 MiB) holds the sample; a limit below its vertex buffer refuses it with a sentence.
        assert!(check_mesh_fits(&doc.mesh, wgpu::Limits::default()).is_ok());
        let tight = wgpu::Limits { max_buffer_size: vertices.max(indices) - 1, ..wgpu::Limits::default() };
        let err = check_mesh_fits(&doc.mesh, tight).unwrap_err();
        assert!(err.contains("too large for this graphics adapter") && err.contains("triangles"), "{err}");
        // At the default 256 MiB a vertex buffer of more than 8.39 million vertices (about 2.8 million unwelded
        // triangles) does not fit.
        assert_eq!(wgpu::Limits::default().max_buffer_size / 32, 8_388_608);
    }
}
