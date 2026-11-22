// Owner: round 9, package B (R9-3, plan T7). The welded mesh graph of the wrap, built once per mesh: the bit-exact
// position weld and the boundary snap over the whole mesh, welded corners per triangle, vertex -> triangles (CSR), a
// global edge id per half-edge and the half-edges per edge (CSR). `Patch::from_graph` (wrap_param.rs) builds a
// parameterization's patch from it without hashing or sorting, and the growth from the anchor (`collect_grown`) walks
// it. A child module of `wrap_param` (it shares the weld's helpers).
//
// Same rules as round 8's per-patch weld (`Patch::build`, kept under `#[cfg(test)]` as the sphere path): vertices are
// welded by bit-exact position (`position_key`) in order of first use over ascending triangles; a vertex on a boundary
// edge is snapped to the lowest (`position_order`) already kept boundary vertex within `WELD_SNAP x diag`; a triangle
// whose snapped corners coincide is dropped. Round 8 did this per patch, where the sphere's cut made boundary edges of
// its own: the graph differs from a patch only where such a cut vertex lay within `WELD_SNAP` of another vertex.

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Instant;

use glam::DVec3;

use super::{NONE, PosMap, WELD_SNAP, dvec, edge_of, position_key, position_order};
use crate::ids::TriId;
use crate::mesh::MeshData;
use crate::uv_share::UvShare;

/// The welded graph of one mesh (plan T7). Immutable; shared by `Arc<WrapMeshCell>`.
pub struct WrapMesh {
    /// Per mesh triangle its welded corners after the snap; `[NONE; 3]` for a triangle the snap collapsed.
    pub(crate) corners: Vec<[u32; 3]>,
    /// Per welded vertex id (the id of first use before the snap; ids that were snapped away stay unused) its position.
    pub(crate) pos: Vec<DVec3>,
    /// Per welded vertex its rank in position order (the tie-break of every vertex comparison of a layout; unused
    /// ids rank last).
    pub(crate) rank: Vec<u32>,
    /// Vertex -> triangles (CSR, ascending triangle id; collapsed triangles left out).
    pub(crate) vt_start: Vec<u32>,
    pub(crate) vt: Vec<u32>,
    /// Half-edge `3 t + k` (the edge opposite corner k of mesh triangle t) -> its edge id (`NONE` when collapsed).
    pub(crate) he_edge: Vec<u32>,
    /// Edge -> its half-edges (CSR, ascending half-edge).
    pub(crate) edge_start: Vec<u32>,
    pub(crate) edge_he: Vec<u32>,
    /// The mesh diagonal the snap used.
    pub(crate) diag: f64,
    /// Time the build took (milliseconds).
    pub build_ms: f64,
}

impl std::fmt::Debug for WrapMesh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WrapMesh").field("tris", &self.corners.len()).field("verts", &self.pos.len()).field("edges", &self.edges()).finish()
    }
}

impl WrapMesh {
    /// Builds the graph of `mesh` on the calling thread (serial: it may run inside a `OnceLock` init, plan WA38).
    pub fn build(mesh: &MeshData) -> WrapMesh {
        let started = Instant::now();
        let diag = mesh.diag() as f64;
        let ntri = mesh.tri_count() as usize;
        // Weld by bit-exact position, ids in order of first use.
        let mut map: PosMap<u128, u32> = PosMap::with_capacity_and_hasher(mesh.positions.len(), Default::default());
        let mut pos: Vec<DVec3> = Vec::new();
        let mut tv: Vec<[u32; 3]> = Vec::with_capacity(ntri);
        for t in 0..ntri as TriId {
            tv.push(mesh.tri(t).map(|i| {
                let p = mesh.positions[i as usize];
                *map.entry(position_key(p)).or_insert_with(|| {
                    pos.push(dvec(p));
                    (pos.len() - 1) as u32
                })
            }));
        }
        drop(map);
        let n = pos.len();
        // Boundary vertices (on an edge with one half-edge, before the snap).
        let mut boundary = vec![false; n];
        let (sorted, keys) = sorted_half_edges(&tv, |_| true);
        let mut i = 0;
        while i < sorted.len() {
            let mut j = i + 1;
            while j < sorted.len() && keys[j] == keys[i] {
                j += 1;
            }
            if j - i == 1 {
                let he = sorted[i];
                let [a, b] = edge_of(tv[(he / 3) as usize], (he % 3) as usize);
                boundary[a as usize] = true;
                boundary[b as usize] = true;
            }
            i = j;
        }
        drop((sorted, keys));
        // Boundary snap (as round 8's `Patch::build`, over the whole mesh).
        let snap = WELD_SNAP * diag;
        let mut rep: Vec<u32> = (0..n as u32).collect();
        if snap > 0.0 && boundary.iter().any(|&b| b) {
            let cell = |p: DVec3| -> [i64; 3] { [(p.x / snap).floor() as i64, (p.y / snap).floor() as i64, (p.z / snap).floor() as i64] };
            let mut grid: PosMap<[i64; 3], Vec<u32>> = PosMap::default();
            for v in 0..n as u32 {
                if !boundary[v as usize] {
                    continue;
                }
                let p = pos[v as usize];
                let c = cell(p);
                let mut best: Option<u32> = None;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let Some(list) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else { continue };
                            for &u in list {
                                if pos[u as usize].distance(p) <= snap && best.is_none_or(|b| position_order(pos[u as usize], pos[b as usize]).is_lt()) {
                                    best = Some(u);
                                }
                            }
                        }
                    }
                }
                match best {
                    Some(u) => rep[v as usize] = u,
                    None => grid.entry(c).or_default().push(v),
                }
            }
        }
        let corners: Vec<[u32; 3]> = tv
            .iter()
            .map(|c| {
                let c = c.map(|v| rep[v as usize]);
                if c[0] == c[1] || c[1] == c[2] || c[0] == c[2] { [NONE; 3] } else { c }
            })
            .collect();
        drop(tv);
        // Vertex -> triangles.
        let mut vt_start = vec![0u32; n + 1];
        for c in corners.iter().filter(|c| c[0] != NONE) {
            for &v in c {
                vt_start[v as usize + 1] += 1;
            }
        }
        for i in 0..n {
            vt_start[i + 1] += vt_start[i];
        }
        let mut fill = vt_start.clone();
        let mut vt = vec![0u32; vt_start[n] as usize];
        for (t, c) in corners.iter().enumerate().filter(|(_, c)| c[0] != NONE) {
            for &v in c {
                vt[fill[v as usize] as usize] = t as u32;
                fill[v as usize] += 1;
            }
        }
        drop(fill);
        // Edges after the snap.
        let (sorted, keys) = sorted_half_edges(&corners, |c| c[0] != NONE);
        let mut he_edge = vec![NONE; corners.len() * 3];
        let mut edge_start = vec![0u32];
        let mut i = 0;
        while i < sorted.len() {
            let mut j = i + 1;
            while j < sorted.len() && keys[j] == keys[i] {
                j += 1;
            }
            let e = (edge_start.len() - 1) as u32;
            for &he in &sorted[i..j] {
                he_edge[he as usize] = e;
            }
            edge_start.push(j as u32);
            i = j;
        }
        let edge_he = sorted;
        drop(keys);
        // Position rank of the vertices in use.
        let used: Vec<bool> = {
            let mut u = vec![false; n];
            for c in corners.iter().filter(|c| c[0] != NONE) {
                for &v in c {
                    u[v as usize] = true;
                }
            }
            u
        };
        let mut order: Vec<u32> = (0..n as u32).filter(|&v| used[v as usize]).collect();
        order.sort_unstable_by(|&a, &b| position_order(pos[a as usize], pos[b as usize]).then(a.cmp(&b)));
        let mut rank = vec![u32::MAX; n];
        for (r, &v) in order.iter().enumerate() {
            rank[v as usize] = r as u32;
        }
        WrapMesh { corners, pos, rank, vt_start, vt, he_edge, edge_start, edge_he, diag, build_ms: started.elapsed().as_secs_f64() * 1e3 }
    }

    /// Number of edges.
    pub fn edges(&self) -> usize {
        self.edge_start.len() - 1
    }

    /// Number of mesh triangles.
    pub fn tri_count(&self) -> usize {
        self.corners.len()
    }

    /// The triangles of welded vertex `v`, ascending.
    pub(crate) fn vtris(&self, v: u32) -> &[u32] {
        &self.vt[self.vt_start[v as usize] as usize..self.vt_start[v as usize + 1] as usize]
    }

    /// The half-edges of edge `e`, ascending.
    pub(crate) fn edge_hes(&self, e: u32) -> &[u32] {
        &self.edge_he[self.edge_start[e as usize] as usize..self.edge_start[e as usize + 1] as usize]
    }

    /// True when the snap dropped triangle `t`.
    pub(crate) fn collapsed(&self, t: TriId) -> bool {
        self.corners[t as usize][0] == NONE
    }

    /// Approximate heap size in bytes.
    pub fn heap_bytes(&self) -> usize {
        self.corners.len() * 12
            + self.pos.len() * 24
            + self.rank.len() * 4
            + (self.vt_start.len() + self.vt.len() + self.he_edge.len() + self.edge_start.len() + self.edge_he.len()) * 4
    }
}

/// The half-edges of the triangles `keep` accepts, sorted by (undirected vertex pair, half-edge), with their keys.
fn sorted_half_edges(tv: &[[u32; 3]], keep: impl Fn(&[u32; 3]) -> bool) -> (Vec<u32>, Vec<u64>) {
    let key = |he: u32| -> u64 {
        let [a, b] = edge_of(tv[(he / 3) as usize], (he % 3) as usize);
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        ((lo as u64) << 32) | hi as u64
    };
    let mut keyed: Vec<(u64, u32)> = Vec::with_capacity(tv.len() * 3);
    for (t, c) in tv.iter().enumerate() {
        if keep(c) {
            for k in 0..3u32 {
                let he = t as u32 * 3 + k;
                keyed.push((key(he), he));
            }
        }
    }
    keyed.sort_unstable();
    keyed.into_iter().map(|(k, he)| (he, k)).unzip()
}

/// Graphs built through cells (`WrapMeshCell::get`), all threads (tests: plan WA38).
static CELL_BUILDS: AtomicU64 = AtomicU64::new(0);

/// The lazily built graph of one mesh, shared by `Arc` between a document (`BakeSets`) and its jobs (plan T7, T8).
/// Tied to the mesh it was made for (a `Weak`: the allocation cannot be reused while the cell lives).
pub struct WrapMeshCell {
    mesh: Weak<MeshData>,
    cell: OnceLock<WrapMesh>,
    /// Round 9 (T9): the mesh's shared-texel triangles (any stamp mode; built by the warm-up job).
    uv: OnceLock<UvShare>,
    builds: AtomicU64,
}

impl std::fmt::Debug for WrapMeshCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WrapMeshCell").field("built", &self.is_built()).finish()
    }
}

impl WrapMeshCell {
    /// An empty cell for `mesh` (nothing is built until `get`).
    pub fn new(mesh: &Arc<MeshData>) -> Arc<WrapMeshCell> {
        Arc::new(WrapMeshCell { mesh: Arc::downgrade(mesh), cell: OnceLock::new(), uv: OnceLock::new(), builds: AtomicU64::new(0) })
    }

    /// True when the cell was made for `mesh`.
    pub fn is_for(&self, mesh: &MeshData) -> bool {
        std::ptr::eq(self.mesh.as_ptr(), mesh)
    }

    /// The graph of `mesh` (the cell's mesh), built on the calling thread on first use (serially: no rayon inside the
    /// `OnceLock` init), together with the shared texels (`uv_share`).
    pub fn get(&self, mesh: &MeshData) -> &WrapMesh {
        assert!(self.is_for(mesh), "a wrap mesh graph used with another mesh");
        self.cell.get_or_init(|| {
            // The shared texels come with the graph (a wrap job that builds the graph leaves the warm-up nothing to do).
            self.uv_share(mesh);
            let g = WrapMesh::build(mesh);
            self.builds.fetch_add(1, AtomicOrdering::Relaxed);
            CELL_BUILDS.fetch_add(1, AtomicOrdering::Relaxed);
            log::info!("wrap mesh: graph built in {:.1} ms ({} triangles, {} edges, {:.1} MB)", g.build_ms, g.tri_count(), g.edges(), g.heap_bytes() as f64 / 1e6);
            g
        })
    }

    /// Round 9 (T9): the mesh's shared-texel triangles, built on the calling thread on first use.
    pub fn uv_share(&self, mesh: &MeshData) -> &UvShare {
        assert!(self.is_for(mesh), "a uv share used with another mesh");
        self.uv.get_or_init(|| {
            let u = UvShare::build(mesh);
            log::info!("wrap mesh: shared texels found in {:.1} ms ({:.1} % of the surface)", u.build_ms, u.mesh_share() * 100.0);
            u
        })
    }

    /// The shared-texel triangles when they are built.
    pub fn uv_built(&self) -> Option<&UvShare> {
        self.uv.get()
    }

    /// True once the graph is built.
    pub fn is_built(&self) -> bool {
        self.cell.get().is_some()
    }

    /// The graph when it is built.
    pub fn built(&self) -> Option<&WrapMesh> {
        self.cell.get()
    }

    /// How often this cell built its graph (0 or 1).
    pub fn builds(&self) -> u64 {
        self.builds.load(AtomicOrdering::Relaxed)
    }

    /// Graphs built through any cell so far (all threads).
    pub fn total_builds() -> u64 {
        CELL_BUILDS.load(AtomicOrdering::Relaxed)
    }
}

/// Round 9 (package B, T8): the warm-up job's input (`Document::wrap_mesh_warmup`): owned and `Send`, so the app runs it
/// as a silent job when a wrapped stamp is selected and the graph is not built yet.
pub struct WrapMeshWarmup {
    pub(crate) cell: Arc<WrapMeshCell>,
    pub(crate) mesh: Arc<MeshData>,
    /// Also the graph (a wrapped stamp), not only the shared texels.
    pub(crate) graph: bool,
}

impl WrapMeshWarmup {
    /// Finds the shared texels and (for a wrapped stamp) builds the graph, where no other thread did meanwhile; returns
    /// the milliseconds this call took.
    pub fn run(self) -> f64 {
        let started = Instant::now();
        self.cell.uv_share(&self.mesh);
        if self.graph {
            self.cell.get(&self.mesh);
        }
        started.elapsed().as_secs_f64() * 1e3
    }

    /// The cell it fills.
    pub fn cell(&self) -> &Arc<WrapMeshCell> {
        &self.cell
    }
}
