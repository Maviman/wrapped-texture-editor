//! GPU resources of one document (ARCHITECTURE.md 8.4): mesh buffers, per-material draw ranges and untextured bind
//! groups, wire instance buffers and one `SetTexture` per texture set, kept in sync with the CPU composites.
//! objects-tabs: wire instances are grouped per object; while objects are hidden a compacted index buffer of the visible
//! triangles and the visible wire runs are drawn instead; the selected visible objects have a highlight index buffer.

use std::collections::BTreeSet;
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

/// Indices of the visible triangles, compacted per draw, and the draws over that compacted list (empty draws dropped);
/// `hidden` holds one flag per object (objects-tabs). Triangle ranges of the draws beyond the index list are clipped.
pub(crate) fn visible_draws(mesh: &MeshData, draws: &[Draw], hidden: &[bool]) -> (Vec<u32>, Vec<Draw>) {
    let is_hidden = |t: u32| hidden.get(mesh.tri_object_of(t) as usize).copied().unwrap_or(false);
    let tri_count = (mesh.indices.len() / 3) as u32;
    let mut indices = Vec::new();
    let mut out = Vec::new();
    for draw in draws {
        let start = (indices.len() / 3) as u32;
        for t in draw.tris.start.min(tri_count)..draw.tris.end.min(tri_count) {
            if !is_hidden(t) {
                let i = t as usize * 3;
                indices.extend_from_slice(&mesh.indices[i..i + 3]);
            }
        }
        let end = (indices.len() / 3) as u32;
        if end > start {
            out.push(Draw { tris: start..end, group: draw.group });
        }
    }
    (indices, out)
}

/// Indices of the triangles of the selected, visible objects, in mesh order (objects-tabs: the highlight).
pub(crate) fn selected_indices(mesh: &MeshData, hidden: &[bool], selected: &BTreeSet<u32>) -> Vec<u32> {
    let count = mesh.object_count() as usize;
    let mut want = vec![false; count];
    for &o in selected {
        if (o as usize) < count && !hidden.get(o as usize).copied().unwrap_or(false) {
            want[o as usize] = true;
        }
    }
    if !want.contains(&true) {
        return Vec::new();
    }
    let tri_count = (mesh.indices.len() / 3) as u32;
    let mut indices = Vec::new();
    for t in 0..tri_count {
        if want.get(mesh.tri_object_of(t) as usize).copied().unwrap_or(false) {
            let i = t as usize * 3;
            indices.extend_from_slice(&mesh.indices[i..i + 3]);
        }
    }
    indices
}

/// Instance runs per object of a list that is sorted by object: `(object, start..end)`, empty runs omitted.
pub(crate) fn object_runs(objects: &[u32]) -> Vec<(u32, Range<u32>)> {
    let mut runs: Vec<(u32, Range<u32>)> = Vec::new();
    for (i, &o) in objects.iter().enumerate() {
        let i = i as u32;
        match runs.last_mut() {
            Some((last, r)) if *last == o && r.end == i => r.end = i + 1,
            _ => runs.push((o, i..i + 1)),
        }
    }
    runs
}

/// The runs of the visible objects, neighbours merged (one range when nothing is hidden and the runs are contiguous).
pub(crate) fn visible_runs(runs: &[(u32, Range<u32>)], hidden: &[bool]) -> Vec<Range<u32>> {
    let mut out: Vec<Range<u32>> = Vec::new();
    for (o, r) in runs {
        if r.is_empty() || hidden.get(*o as usize).copied().unwrap_or(false) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == r.start => last.end = r.end,
            _ => out.push(r.clone()),
        }
    }
    out
}

/// One wire instance list and the object runs of its instances, in buffer order (objects-tabs). A list may hold
/// several groups (3D all: interior, then seams + boundaries; UV: interior, then seams); each group is stable-sorted
/// by object, so an object has one run per group.
#[derive(Clone, Debug, Default)]
pub(crate) struct WireList {
    pub instances: Vec<LineInstance>,
    pub runs: Vec<(u32, Range<u32>)>,
}

impl WireList {
    /// Appends a group of `(object, instance)` pairs, stable-sorted by object (a group of one object keeps its order:
    /// the bytes of a one-object model are those of the build without objects).
    fn push_group(&mut self, mut group: Vec<(u32, LineInstance)>) {
        group.sort_by_key(|(o, _)| *o);
        let base = self.instances.len() as u32;
        let objects: Vec<u32> = group.iter().map(|(o, _)| *o).collect();
        self.runs.extend(object_runs(&objects).into_iter().map(|(o, r)| (o, r.start + base..r.end + base)));
        self.instances.extend(group.into_iter().map(|(_, i)| i));
    }
}

fn wire_color(kind: EdgeKind, interior: [u8; 4]) -> [u8; 4] {
    match kind {
        EdgeKind::Interior => interior,
        EdgeKind::Seam => WIRE_SEAM,
        EdgeKind::Boundary => WIRE_BOUNDARY,
    }
}

/// 3D wire instances: (all edges, seams + boundaries only). Interior edges come first so seams draw on top; each group
/// is stable-sorted by object (objects-tabs).
pub(crate) fn wire_instances_3d(edges: &[WireEdge]) -> (WireList, WireList) {
    let to_pair = |e: &WireEdge| (e.object, LineInstance { a: e.a, b: e.b, color: wire_color(e.kind, WIRE_INTERIOR_3D) });
    let interior: Vec<(u32, LineInstance)> = edges.iter().filter(|e| e.kind == EdgeKind::Interior).map(to_pair).collect();
    let seams: Vec<(u32, LineInstance)> = edges.iter().filter(|e| e.kind != EdgeKind::Interior).map(to_pair).collect();
    let mut all = WireList::default();
    all.push_group(interior);
    all.push_group(seams.clone());
    let mut seam_list = WireList::default();
    seam_list.push_group(seams);
    (all, seam_list)
}

/// UV wire instances of one set: `(uv_a, 0)` -> `(uv_b, 0)`, interior edges first.
#[cfg(test)]
pub(crate) fn wire_instances_uv(edges: &[WireEdge], set: TextureSetId) -> Vec<LineInstance> {
    wire_instances_uv_all(edges, set.index() + 1).swap_remove(set.index()).instances
}

/// UV wire instances of every set `0..set_count` in one pass over the edges (a document with many sets used to scan
/// all edges once per set): per set `(uv_a, 0)` -> `(uv_b, 0)`, interior edges first, each group in edge order and
/// then stable-sorted by object (objects-tabs). Edges of other sets (or none) are ignored.
pub(crate) fn wire_instances_uv_all(edges: &[WireEdge], set_count: usize) -> Vec<WireList> {
    let to_pair = |e: &WireEdge| {
        let instance = LineInstance {
            a: [e.uv_a[0], e.uv_a[1], 0.0],
            b: [e.uv_b[0], e.uv_b[1], 0.0],
            color: wire_color(e.kind, WIRE_INTERIOR_UV),
        };
        (e.object, instance)
    };
    let mut interior: Vec<Vec<(u32, LineInstance)>> = vec![Vec::new(); set_count];
    let mut seams: Vec<Vec<(u32, LineInstance)>> = vec![Vec::new(); set_count];
    for e in edges {
        let Some(set) = e.set.map(|s| s.index()).filter(|&s| s < set_count) else { continue };
        let bucket = if e.kind == EdgeKind::Interior { &mut interior[set] } else { &mut seams[set] };
        bucket.push(to_pair(e));
    }
    interior
        .into_iter()
        .zip(seams)
        .map(|(interior, seams)| {
            let mut list = WireList::default();
            list.push_group(interior);
            list.push_group(seams);
            list
        })
        .collect()
}

/// The CPU-side geometry of a scene: render vertices and the wire instance lists (3D all, 3D seams, UV per set).
struct SceneGeometry {
    vertices: Vec<wte_core::RenderVertex>,
    wire_all: WireList,
    wire_seams: WireList,
    wire_uv: Vec<WireList>,
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

/// A wire instance buffer with its object runs (objects-tabs) and the instance ranges to draw.
struct WireBuffer {
    buffer: wgpu::Buffer,
    runs: Vec<(u32, Range<u32>)>,
    /// Every instance (the export preview, and the draw while nothing is hidden: one call, as before objects).
    whole: [Range<u32>; 1],
    /// The runs of the visible objects (`whole` while nothing is hidden).
    visible: Vec<Range<u32>>,
}

impl WireBuffer {
    fn ranges(&self, all_objects: bool) -> &[Range<u32>] {
        if all_objects { &self.whole } else { &self.visible }
    }

    fn set_hidden(&mut self, hidden: Option<&[bool]>) {
        self.visible = match hidden {
            Some(hidden) => visible_runs(&self.runs, hidden),
            None => self.whole.to_vec(),
        };
    }
}

/// Vertex buffer of line instances, `None` when there are none or they exceed the device's `max_buffer_size` (the
/// wireframe is then not drawn; logged).
fn instance_buffer(gpu: &Gpu, label: &str, list: &WireList) -> Option<WireBuffer> {
    let instances = &list.instances;
    if instances.is_empty() {
        return None;
    }
    let bytes = std::mem::size_of_val(instances.as_slice()) as u64;
    if bytes > gpu.device.limits().max_buffer_size {
        log::warn!("{label}: {} line instances ({bytes} bytes) exceed the buffer size limit; not drawn", instances.len());
        return None;
    }
    let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(instances),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let whole = 0..instances.len() as u32;
    Some(WireBuffer { buffer, runs: list.runs.clone(), whole: [whole.clone()], visible: vec![whole] })
}

/// An index buffer of `indices` (never empty).
fn index_buffer(gpu: &Gpu, label: &str, indices: &[u32]) -> wgpu::Buffer {
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(indices),
        usage: wgpu::BufferUsages::INDEX,
    })
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
    wire_all: Option<WireBuffer>,
    wire_seams: Option<WireBuffer>,
    /// Per set: UV wireframe instances.
    wire_uv: Vec<Option<WireBuffer>>,
    sets: Vec<SceneSet>,
    /// objects-tabs: `Some` while objects are hidden: the index buffer of the visible triangles (`None` when every
    /// triangle is hidden) and the draws over it.
    visible: Option<(Option<wgpu::Buffer>, Vec<Draw>)>,
    /// objects-tabs: the index buffer of the selected visible objects' triangles and its index count.
    highlight: Option<(wgpu::Buffer, u32)>,
    /// `(visibility_generation, object selection revision)` the two above were made for; `None` before the first
    /// `sync_objects`.
    objects_key: Option<(u64, u64)>,
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
            visible: None,
            highlight: None,
            objects_key: None,
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

    /// objects-tabs (8.4): rebuilds the visible index buffer and the visible wire ranges when the document's
    /// `visibility_generation` changed, and the highlight index buffer when that or `selection_rev` changed. Two integer
    /// compares otherwise. The new buffers are subsets of the full index buffer, so they always fit the device.
    pub fn sync_objects(&mut self, gpu: &Gpu, doc: &Document, selected: &BTreeSet<u32>, selection_rev: u64) {
        let generation = doc.visibility_generation();
        let visibility_changed = self.objects_key.is_none_or(|(g, _)| g != generation);
        let selection_changed = visibility_changed || self.objects_key.is_none_or(|(_, r)| r != selection_rev);
        if !selection_changed {
            return;
        }
        self.objects_key = Some((generation, selection_rev));
        let mesh = &doc.mesh;
        let hidden = doc.hidden_objects();
        let any_hidden = hidden.contains(&true);
        if visibility_changed {
            let start = Instant::now();
            let mask = any_hidden.then_some(hidden);
            for wires in self.wire_all.iter_mut().chain(self.wire_seams.iter_mut()).chain(self.wire_uv.iter_mut().flatten()) {
                wires.set_hidden(mask);
            }
            self.visible = None;
            if any_hidden && self.indices.is_some() {
                let (indices, draws) = visible_draws(mesh, &self.draws, hidden);
                let shown = indices.len() / 3;
                let buffer = (!indices.is_empty()).then(|| index_buffer(gpu, "mesh_indices_visible", &indices));
                self.visible = Some((buffer, draws));
                log::info!(
                    "objects: visible geometry rebuilt in {:.1} ms ({shown} of {} triangles, {} of {} objects hidden)",
                    start.elapsed().as_secs_f64() * 1000.0,
                    mesh.tri_count(),
                    doc.hidden_object_count(),
                    mesh.object_count()
                );
            } else if generation > 0 {
                log::info!("objects: everything visible again ({:.1} ms)", start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        self.highlight = None;
        if !selected.is_empty() && self.indices.is_some() {
            let indices = selected_indices(mesh, hidden, selected);
            if !indices.is_empty() {
                let count = indices.len() as u32;
                self.highlight = Some((index_buffer(gpu, "mesh_indices_highlight", &indices), count));
            }
        }
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

    /// Vertex buffer, index buffer and draw calls (in material order) of a render: the visible triangles, or every
    /// triangle with `all_objects` (objects-tabs). `None` for an empty mesh or when every triangle is hidden.
    pub(crate) fn mesh_draw(&self, all_objects: bool) -> Option<(&wgpu::Buffer, &wgpu::Buffer, &[Draw])> {
        let vertices = self.vertices.as_ref()?;
        match &self.visible {
            Some((indices, draws)) if !all_objects => Some((vertices, indices.as_ref()?, draws)),
            _ => Some((vertices, self.indices.as_ref()?, &self.draws)),
        }
    }

    /// objects-tabs: vertex buffer, index buffer and index count of the selection highlight (the selected visible
    /// objects' triangles); `None` without one.
    pub(crate) fn highlight(&self) -> Option<(&wgpu::Buffer, &wgpu::Buffer, u32)> {
        let (indices, count) = self.highlight.as_ref()?;
        Some((self.vertices.as_ref()?, indices, *count))
    }

    /// The GPU texture of set `index`.
    pub(crate) fn set_texture(&self, index: usize) -> Option<&SetTexture> {
        self.sets.get(index).map(|s| &s.texture)
    }

    /// The untextured bind group of material `index`.
    pub(crate) fn untextured_group(&self, index: usize) -> Option<&wgpu::BindGroup> {
        self.untextured.get(index).map(|(_, g)| g)
    }

    /// All 3D wire instances (interior, seams, boundaries) and the instance ranges to draw: those of the visible
    /// objects, or all with `all_objects`.
    pub(crate) fn wire_all(&self, all_objects: bool) -> Option<(&wgpu::Buffer, &[Range<u32>])> {
        self.wire_all.as_ref().map(|w| (&w.buffer, w.ranges(all_objects)))
    }

    /// Seam and boundary 3D wire instances and the ranges to draw (as `wire_all`).
    pub(crate) fn wire_seams(&self, all_objects: bool) -> Option<(&wgpu::Buffer, &[Range<u32>])> {
        self.wire_seams.as_ref().map(|w| (&w.buffer, w.ranges(all_objects)))
    }

    /// UV wire instances of set `index` and the ranges to draw (as `wire_all`).
    pub(crate) fn wire_uv(&self, index: usize, all_objects: bool) -> Option<(&wgpu::Buffer, &[Range<u32>])> {
        self.wire_uv.get(index)?.as_ref().map(|w| (&w.buffer, w.ranges(all_objects)))
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
        let (all, seams) = (all.instances, seams.instances);
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
            assert_eq!(bytes(&got.instances), bytes(&want), "set {i}");
            assert_eq!(visible_runs(&got.runs, &[false]), vec![0..want.len() as u32], "one object: the runs cover the list");
        }
        // Edges of sets beyond the count are ignored, not a panic.
        assert_eq!(wire_instances_uv_all(&edges, 1).len(), 1);

        let (geometry, marker) = SceneGeometry::build_with(&doc.mesh, sets, || 7);
        assert_eq!(marker, 7);
        let (all3d, seams3d) = wire_instances_3d(&edges);
        assert_eq!(bytes(&geometry.wire_all.instances), bytes(&all3d.instances));
        assert_eq!(bytes(&geometry.wire_seams.instances), bytes(&seams3d.instances));
        // objects-tabs: one object => the lists are those of the build without objects (interior in edge order, then
        // seams + boundaries in edge order), byte for byte, each with one run per group.
        let to_instance = |e: &WireEdge| LineInstance { a: e.a, b: e.b, color: wire_color(e.kind, WIRE_INTERIOR_3D) };
        let old_seams: Vec<LineInstance> = edges.iter().filter(|e| e.kind != EdgeKind::Interior).map(to_instance).collect();
        let old_all: Vec<LineInstance> =
            edges.iter().filter(|e| e.kind == EdgeKind::Interior).map(to_instance).chain(old_seams.iter().copied()).collect();
        assert_eq!(bytes(&all3d.instances), bytes(&old_all));
        assert_eq!(bytes(&seams3d.instances), bytes(&old_seams));
        let interior = (old_all.len() - old_seams.len()) as u32;
        assert_eq!(all3d.runs, vec![(0, 0..interior), (0, interior..old_all.len() as u32)]);
        assert_eq!(visible_runs(&all3d.runs, &[false]), vec![0..old_all.len() as u32], "neighbours merge");
        assert_eq!(geometry.wire_uv.len(), sets);
        assert_eq!(
            bytemuck::cast_slice::<wte_core::RenderVertex, u8>(&geometry.vertices),
            bytemuck::cast_slice::<wte_core::RenderVertex, u8>(&doc.mesh.render_vertices())
        );
    }

    /// objects-tabs: the Three Objects sample (Floor 0, Cube 1, Ball 2; one material, so one draw).
    fn trio() -> Document {
        let doc = Document::sample(wte_core::SampleKind::Objects);
        assert_eq!(doc.mesh.object_count(), 3);
        doc
    }

    /// The triangles (as index triples) a draw list covers over an index list.
    fn drawn(indices: &[u32], draws: &[Draw]) -> Vec<([u32; 3], DrawGroup)> {
        let mut out = Vec::new();
        for d in draws {
            for t in d.tris.clone() {
                let i = t as usize * 3;
                out.push(([indices[i], indices[i + 1], indices[i + 2]], d.group));
            }
        }
        out
    }

    #[test]
    fn visible_draws_hold_every_visible_triangle_once_in_its_group() {
        for doc in [trio(), Document::sample(wte_core::SampleKind::TwoMaterial)] {
            let mesh = &doc.mesh;
            let mut draws = build_draws(mesh);
            if draws.len() == 1 {
                // Split the one draw in three so that groups and their order are exercised.
                let d = draws[0].clone();
                let (a, b) = (d.tris.start + d.tris.len() as u32 / 3, d.tris.start + 2 * d.tris.len() as u32 / 3);
                draws = vec![
                    Draw { tris: d.tris.start..a, group: d.group },
                    Draw { tris: a..b, group: DrawGroup::Untextured(0) },
                    Draw { tris: b..d.tris.end, group: d.group },
                ];
            }
            let n = mesh.object_count() as usize;
            // Nothing hidden: the full index list and the same draws.
            let (indices, visible) = visible_draws(mesh, &draws, &vec![false; n]);
            assert_eq!(indices, mesh.indices[..mesh.indices.len() / 3 * 3]);
            assert_eq!(visible, draws);
            // Every single object hidden, and all but the first.
            let mut cases: Vec<Vec<bool>> = (0..n).map(|o| (0..n).map(|i| i == o).collect()).collect();
            cases.push((0..n).map(|i| i != 0).collect());
            for hidden in cases {
                let (indices, visible) = visible_draws(mesh, &draws, &hidden);
                assert!(visible.iter().all(|d| !d.tris.is_empty()), "empty draws are dropped");
                let want: Vec<([u32; 3], DrawGroup)> = draws
                    .iter()
                    .flat_map(|d| d.tris.clone().map(move |t| (t, d.group)))
                    .filter(|(t, _)| !hidden[mesh.tri_object_of(*t) as usize])
                    .map(|(t, g)| {
                        let i = t as usize * 3;
                        ([mesh.indices[i], mesh.indices[i + 1], mesh.indices[i + 2]], g)
                    })
                    .collect();
                assert_eq!(drawn(&indices, &visible), want, "hidden {hidden:?}");
                assert_eq!(indices.len() / 3, want.len());
            }
            // Everything hidden: nothing to draw.
            let (indices, visible) = visible_draws(mesh, &draws, &vec![true; n]);
            assert!(indices.is_empty() && visible.is_empty());
        }
    }

    #[test]
    fn selected_indices_are_the_selected_visible_objects_triangles() {
        let doc = trio();
        let mesh = &doc.mesh;
        let tris_of = |objects: &[u32]| -> Vec<u32> {
            (0..mesh.tri_count())
                .filter(|t| objects.contains(&mesh.tri_object_of(*t)))
                .flat_map(|t| mesh.indices[t as usize * 3..t as usize * 3 + 3].to_vec())
                .collect()
        };
        let none = [false; 3];
        assert!(selected_indices(mesh, &none, &BTreeSet::new()).is_empty());
        assert_eq!(selected_indices(mesh, &none, &BTreeSet::from([1])), tris_of(&[1]));
        assert_eq!(selected_indices(mesh, &none, &BTreeSet::from([2, 1, 9])), tris_of(&[1, 2]), "mesh order, unknown ignored");
        assert_eq!(selected_indices(mesh, &[false, true, false], &BTreeSet::from([1, 2])), tris_of(&[2]), "hidden: no highlight");
        assert!(selected_indices(mesh, &[false, true, false], &BTreeSet::from([1])).is_empty());
    }

    #[test]
    fn object_runs_and_visible_runs() {
        assert!(object_runs(&[]).is_empty());
        assert_eq!(object_runs(&[0, 0, 2, 2, 2, 5]), vec![(0, 0..2), (2, 2..5), (5, 5..6)]);
        let runs = vec![(0, 0..2), (1, 2..5), (2, 5..6), (0, 6..9), (2, 9..10)];
        assert_eq!(visible_runs(&runs, &[false, false, false]), vec![0..10], "nothing hidden: one range");
        assert_eq!(visible_runs(&runs, &[]), vec![0..10], "no flags: nothing hidden");
        assert_eq!(visible_runs(&runs, &[false, true, false]), vec![0..2, 5..10]);
        assert_eq!(visible_runs(&runs, &[true, false, true]), vec![2..5]);
        assert!(visible_runs(&runs, &[true, true, true]).is_empty());
    }

    /// objects-tabs: the wire lists of a multi-object mesh are grouped per object inside each group (3D all: interior,
    /// then seams + boundaries; UV: interior, then seams), keeping the edge order within an object; the runs name the
    /// object of every instance.
    #[test]
    fn wire_lists_of_several_objects_are_grouped_per_object() {
        let doc = trio();
        let edges = doc.mesh.wire_edges();
        assert!(edges.iter().any(|e| e.object != edges[0].object), "edges of several objects");
        let (all, seams) = wire_instances_3d(&edges);
        let to_instance = |e: &WireEdge| LineInstance { a: e.a, b: e.b, color: wire_color(e.kind, WIRE_INTERIOR_3D) };
        let bytes = |v: &[LineInstance]| bytemuck::cast_slice::<LineInstance, u8>(v).to_vec();
        let grouped = |interior: bool| -> Vec<LineInstance> {
            let mut objects: Vec<u32> = edges.iter().map(|e| e.object).collect();
            objects.sort_unstable();
            objects.dedup();
            objects
                .iter()
                .flat_map(|&o| edges.iter().filter(move |e| e.object == o && (e.kind == EdgeKind::Interior) == interior))
                .map(to_instance)
                .collect()
        };
        let want_seams = grouped(false);
        let want_all: Vec<LineInstance> = grouped(true).into_iter().chain(want_seams.iter().copied()).collect();
        assert_eq!(bytes(&all.instances), bytes(&want_all));
        assert_eq!(bytes(&seams.instances), bytes(&want_seams));
        // The runs name the object of every instance: per group, the sorted objects of its edges.
        let objects_of = |interior: bool| -> Vec<u32> {
            let mut o: Vec<u32> = edges.iter().filter(|e| (e.kind == EdgeKind::Interior) == interior).map(|e| e.object).collect();
            o.sort_unstable();
            o
        };
        let interior_count = objects_of(true).len() as u32;
        let seam_runs = object_runs(&objects_of(false));
        let want_runs: Vec<(u32, Range<u32>)> = object_runs(&objects_of(true))
            .into_iter()
            .chain(seam_runs.iter().map(|(o, r)| (*o, r.start + interior_count..r.end + interior_count)))
            .collect();
        assert_eq!(all.runs, want_runs);
        assert_eq!(seams.runs, seam_runs);
        assert!(all.runs.len() >= 4, "several objects: several runs {:?}", all.runs);
        let floor_only = visible_runs(&all.runs, &[false, true, true]);
        let floor_edges = edges.iter().filter(|e| e.object == 0).count() as u32;
        assert_eq!(floor_only.iter().map(|r| r.len() as u32).sum::<u32>(), floor_edges);
        let uv = wire_instances_uv_all(&edges, doc.sets.len());
        assert_eq!(uv.iter().map(|l| l.instances.len()).sum::<usize>(), edges.iter().filter(|e| e.set.is_some()).count());
        for list in &uv {
            assert_eq!(list.runs.iter().map(|(_, r)| r.len()).sum::<usize>(), list.instances.len());
        }
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
