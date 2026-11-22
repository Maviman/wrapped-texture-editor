# Round 6 plan — Wrapped Texture Editor ("wrap on surface")

Date: 2026-11-21. Planned by Opus at the user's choice. Source = the main tree `E:\claude_programs\wrapped_texture_editor`
(round 5's final source, exe `target\round5\wte_round5.exe`, SHA 5DA4E165...6568). `ARCHITECTURE.md` is the binding
contract; this plan says which of its sections change. File:line references are to the main tree as it is today.

One item, two packages (built one after the other), one gate in two stages:

| Item | What | Package |
|---|---|---|
| R6-1 | Surface parameterization and the wrap bake (core) | A |
| R6-2 | The toggle, the tools and the scenario (app) | B (starts from A's checked copy) |

Request: wrap 3D text and 3D image stamps "physically around an object", movable over the surface, the full image
on an uneven surface "without it tearing or dissapearing", as a toggle; projection stays. 2D (UV) stamps stay as they
are and stay a separate kind. The user's answers to the plan's questions are binding (section 11).

---

## 0. Facts every agent needs

- Build from the root of your copy: `cmd /c "E:\claude_programs\wrapped_texture_editor\cargo.cmd <cmd> --offline 2>&1"`
  (`test --workspace`, `clippy --workspace --all-targets`, `build --release -p wte_app`). Never edit `cargo.cmd` or
  `.cargo\config.toml`. No new crates: everything below uses `glam`, `rayon`, `std` (already dependencies). There is
  no sparse linear solver in `Cargo.lock` (no nalgebra / faer / sprs); the design needs none.
- Machine: 6 cores, 7 GB RAM, slow USB disk; a cold build of one copy is about 20 minutes. One heavy cargo build at a
  time; when free memory is low, wait. Never scan `target\` with recursive finds. Large outputs on E: only.
- Testing is headless: `<exe> [PATH] --screenshot <out.png> --scenario <name> [--size 1280x800]` writes `<out>.json`
  with `"ok"`. No OS mouse/keyboard injection. Never kill a running `wrapped_texture_editor.exe`. Never run downloaded
  binaries. `WTE_FRAME_LOG=1` gives per-frame timing lines; `WTE_SYNC_FINAL_BAKES=1` keeps Final bakes on the UI
  thread (round 5).
- Copies: `target\round6\work\base` (main tree without `target\`), `pkg_a` (from base), `pkg_b` (from `pkg_a` after
  its check; copying `pkg_a\target` along saves a cold build, E: has 530 GB free), `merged`. Notes
  `target\round6\notes\pkg_a.md`, `pkg_b.md`, `gate1.md`, `gate2.md`; report `target\round6\GATE_REPORT.md`; exe
  `target\round6\wte_round6.exe`.
- Tools: `target\round4\review\gate3\` (`frames.py`, `wf_steps.py`, `perf_driver.ps1`), `target\round3_audit\`
  (`sweep.ps1`, `slow.py`), `target\round3\shot_diff.py` and `check_exports.py`. Reference: `wte_round5.exe`.
- Models: built-in Sphere sample; `target\compat\harness\shapes\` (`ngons_tiling.obj`, `torus_knot`, `mobius`,
  `klein_bottle`, `nonmanifold`, `many_islands`, `flat_annulus`, `high_poly_sphere`, `tiny_cube`);
  `E:\claude_downloads\glb_samples\` (`ABeautifulGame.glb` 1.5 M triangles, `WaterBottle.glb`, `Avocado.glb`,
  `DamagedHelmet.glb`, `Lantern.glb`). Core tests use only `samples.rs` meshes; file models only in ignored probes
  and the gate.

---

## 1. What the user gets (plain words)

Every 3D stamp and 3D text gets a switch **Project | Wrap** (selected stamp's options; Stamp / Text tool options for
new ones). **Project** is today's behaviour, pixel for pixel: a slide projector from the front, stretched at the
sides, nothing on the back. **Wrap** lays the picture on the surface like a sticker: its long centre line runs
straight along the surface, the rest follows outwards; it goes round a bottle or a chess piece, follows bumps, and
continues across UV seams and hard edges. Dragging moves it over the surface (it re-wraps where it lies); corners
scale it, the knob turns it about the normal. Switching back gives the old projection (its settings were kept). The
panel says when part of the picture has no surface under it. Images and texts with every text effect (the text
picture is drawn as today, then wrapped).
- Two per-stamp options of Wrap (images and texts; greyed out in Project, kept when switching back and forth):
  - **Jump small gaps** (off by default): off, the sticker stops at the edge of a part; on, it continues onto
    separate parts closer than 1 % of the model size (a letter line across chess pieces standing on the board, a
    helmet's separate visor), as if the surface ran on flat over the gap.
  - **Longer than the way round: Ends meet | Overlap** (Ends meet by default): Ends meet shows the middle part and
    the two ends meet at the back; Overlap keeps winding like tape, the picture's end lying on top of the turn
    below (a see-through text shows the turn below through its gaps).

## 2. Technique

### 2.1 Choice: strip-seeded geodesic unfolding ("Fermi coordinates" of the picture's centre line)

Each wrapped stamp gets a **local parameterization**: every vertex near it gets 2D coordinates (along, across) in
world units in the stamp's frame; a texel's barycentrics give its coordinates, those give the picture's (u, v), so the
picture is painted through each triangle's UVs. (1) **Weld** the nearby triangles by position: both UV copies of a seam
vertex get one coordinate, so seams are invisible. (2) **Spine**: trace the straightest path over the surface from the
anchor both ways along the picture's longer axis; the strip of crossed triangles unfolds **exactly** into the plane.
(3) **Grow outwards**, nearest first: each new vertex is placed by unfolding a triangle it shares with two placed ones
(exact on developable surfaces: plane, cylinder, cone, folds, box edges); disagreeing unfoldings are averaged when
within a quarter edge, else the one nearer the spine wins (fronts meeting behind a corner or hole never smear).
(4) **Tears**: a triangle flipped or stretched over 2x is re-unfolded rigidly from its vertex nearest the spine: a
clean cut line, never a smear. Details in 4.3; the two options in 4.3a / 4.3b.

### 2.2 Why this and not the alternatives

| Option | For | Against | Verdict |
|---|---|---|---|
| **Strip-seeded unfolding (chosen)** | Exact on developable surfaces (cylinder, cone, folded box): the bottle / chess-piece / label cases. The text's centre line keeps its length and letters keep their height along a long line, even on a ball. Local (cost grows with the stamp's area, not the model). No solver. Deterministic, serial, robust to bad meshes (a front never needs a disk topology). | Distortion away from the centre line on doubly curved surfaces (2.3). | Best fit for "text around an object". |
| Discrete exponential map (Schmidt 2006: polar geodesic coordinates from the centre) | The classic interactive decal method, same machinery. | On a ball the letters of a long line shrink in height towards its ends (sin d / d: 36 % smaller at a quarter turn). Polar coordinates suit a round logo, not a line of text. | The same engine with a point seed; not needed separately. |
| Local conformal (LSCM) / ARAP flattening of the covered patch | Lower overall distortion on a bump or a ball. | Needs a sparse solver (none in `Cargo.lock`; a hand-written CG of a few hundred lines); the patch must be cut to a disk (a ring around a bottle is not); every move re-solves a global system (tens to hundreds of ms on a heavy patch); the result is not exact on a cylinder unless pinned. | Too slow and too large for this round; can refine the chosen method later. |
| Cylinder / sphere projection primitives; projection without culling ("project through") | Trivial / exists. | Only fits objects that are that primitive; projection still stretches at the sides, prints both sides, never goes round. | No (Project mode stays as it is). |

### 2.3 What is guaranteed and what degrades

| Surface under the picture | Result |
|---|---|
| Flat, cylinder, cone, folded sheet, box faces across an edge | **Exact**: the picture lies undistorted (lengths and angles kept). Tested against the analytic unrolling (WA1, WA5). |
| UV seams anywhere under the picture | **Continuous**: one coordinate per welded vertex (bit-identical on every UV copy); gutter texels extrapolate the same coordinates. |
| Bumps, a ball, a saddle (curved both ways) | Whole picture, continuous, no holes. Lengths along the centre line exact; away from it widths change: on a ball of radius R at a distance d from the centre line by cos(d/R): 3 % at d = 0.25 R, 12 % at 0.5 R, 46 % at 1 R. |
| Longer than the way around (a band around a bottle) | Ends meet (default): the middle part shows, the two ends meet in one line opposite the anchor (the spine stops when it meets itself); no overlap, no smear. Overlap: every turn is laid (at most 8), later along the picture on top; exact on a cylinder (WA18). On a non-cylindrical band the turns follow the real straightest path, so they may drift sideways (spiral) as real tape does. |
| A box corner, a cone tip | The wedge that cannot lie flat is cut along one line (like a sticker pressed onto a corner). |
| A hole, an open edge, the end of a part | No surface there: that part of the picture is missing (like a sticker over a hole); the rest stays in place, the front goes round a hole (stretched there; a cut line may show behind it). The panel reports the share that runs off. |
| Separate parts of the model (not connected by shared vertices) | Jump small gaps off (default): not reached (Project mode still reaches them). On: a part within 1 % of the model size of the reached surface (and inside the picture's reach) gets the picture continued across the gap, which counts as flat ground in the picture's direction (WA16 exact on coplanar parts); a part reached in several places takes its placement from the first (nearest the spine). Wider gaps still stop it. Cracks narrower than 1e-5 of the model size are closed by the weld in both settings. T-junctions stop the wrap where their two sides are separate parts, unless Jump small gaps is on (then they are crossed as a gap of 0). |
| Thin parts (a sheet with two sides) | Folds round the rim onto the other side, as a sticker would. |
| Tiled, mirrored or stacked UVs | Every texel copy shows the stamp (max-alpha rule), as with the brush. |
| Very heavy model, very large stamp | At most 1,000,000 welded vertices per stamp (beyond: the wrap stops, the panel says so). The Final bake runs on the worker as in round 5. |
| Non-manifold edges, Möbius strip, Klein bottle | Works, deterministic; cut lines where the front closes on itself. |

---

## 3. Data model, project format, interactions

### 3.1 Data model (decision: a mode of the 3D stamp, not a new kind)

`stamp.rs`: `#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)] pub enum SurfaceMode { #[default] Project, Wrap }`
and a new last field `pub mode: SurfaceMode` on `Projector` (`Projector::from_surface` sets `Project`). Reasons: the
Transform tool, gizmo, text anchor rule (`text::frame`), depth fit and the manifest all work on `Projector` already;
`Placement` keeps its two kinds (the binding "3D and 2D stay separate kinds" holds); the toggle is one field of a
placement edit; switching back keeps the projection's depth and angle settings. Full `Projector { .. }` literals exist
only in the core (`stamp_bake.rs:161`, `manifest.rs:502`, `:539`); the app uses `from_surface` and struct update
syntax, so the app compiles unchanged after package A. In Wrap mode the fields `depth_front`, `depth_back`,
`angle_*`, `backface_cull`, `occlusion` are kept and ignored; `origin` (anchor), `rotation` (frame), `half_size`,
`flip_x`, `flip_y` are used.

The two Wrap options are a second new field: `pub wrap: WrapOptions` with
`#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)] pub struct WrapOptions { pub jump_gaps: bool, pub overlap: bool }`
(both `false` = Stop at the edge, Ends meet). They live on the placement in both modes, so a toggle to Project and
back keeps them, an option change is an ordinary placement edit (one history entry, undoable, refused when locked,
copied by duplicate), and they are ignored in Project mode. The gap distance is **fixed** at 1 % of the model's
bounding-box diagonal (`GAP_JUMP`, 4.2), shown in the option's hover text; a slider is not needed for the request
and would add a stored float, a UI row and more tests (a later round can add it as `gap: f32` with the same format
rule).

### 3.2 Project format

- `PlacementEntry::Projected` gets `#[serde(default, skip_serializing_if = "String::is_empty")] mode: String`:
  `""` (absent) = Project, `"wrap"` = Wrap; and `#[serde(default, skip_serializing_if = "is_false")] wrap_gaps:
  bool`, `wrap_overlap: bool` (written only when `true`). A file without wrapped stamps or set options writes no new
  key.
- `FORMAT_VERSION` 2 → **3**; `migrate.rs` gets an identity step `v2_to_v3` (the `STEPS.len()` test follows).
  `save::required_reader` returns **3** when any stamp is wrapped or has a Wrap option set (also a projected stamp
  that keeps options for switching back: a round-5 save would drop them silently), else what it returns today.
- Older builds: a project without wrapped stamps written by round 6 opens in round 5 with the existing
  `NewerFormat` warning (as round-4 files did in round 3); a project with a wrapped stamp is refused by round 5 with
  `TooNew` (a plain Save there would silently turn it into a projected stamp). Round-5 and older projects open in
  round 6 unchanged (no `mode` key = Project).
- An unknown `mode` string (a later round) loads as Project with a new `LoadWarning::StampFeatureUnknown { stamp,
  what }` (error.rs, Display text like `TextFeatureUnknown`), and counts as "cannot keep" for the plain-Save rule at
  `load.rs:661`.
- The baked caches are saved as today, so a reopened wrapped stamp is pixel-identical without a re-bake.

### 3.3 Interactions (all specified; none changes Project mode)

| Feature | In Wrap mode |
|---|---|
| Toggle | `set_stamp_placement(id, Projected(Projector { mode, ..p }), Final)`: one history entry (`EDIT_LABEL`), undoable; refused on a locked stamp like any placement edit. Toggling Wrap → Project with no other change gives byte-identical caches to before. |
| Jump small gaps / Overlap | Placement edits like the toggle (one entry each, undoable, kept through Project and back); changing one recomputes the parameterization. With one part under the picture, Jump on = off byte for byte; with a spine that never meets itself, Overlap = Ends meet byte for byte. |
| Warps, text on a path, styles, outline, gaps, bold, slant | Change the picture only; the picture is wrapped as drawn. Text raster resolution rule (6.5) unchanged (world units are surface units). |
| Text anchor rule (`refit_placement`) | Moves `origin` in the tangent plane as today; the wrap anchors at the surface point nearest `origin` (4.3 step 2). Exact for centred anchors (plain text); a strongly off-centre anchor on a curved surface drifts by at most ~7 % of its shift at half the curvature radius. |
| Transform drag / scale / rotate / wheel / numeric fields | Unchanged code: body drag raycasts and `moved_to` (anchor follows the cursor over the surface, frame carried along); scale changes `half_size`; rotation turns the frame about the normal. Depth auto-fit still runs (0.2 ms) so that switching back gives a fitted projection. |
| Deferred Final bakes (round 5), settle before save / export / merge down / rasterize | Same path: `StampBakeJob` carries the placement (with mode) and an optional cached parameterization; `run()` dispatches. |
| Preview bakes while dragging | Same throttle (`bake_stale_previews`); a Preview wrap bake = parameterization + nearest-level sampling. |
| Undo / redo / duplicate / delete / lock / rename / replace image | Unchanged (mode is part of the placement; duplicate copies it). |
| Merge down, rasterize, export, save | Work on the caches, unchanged. |
| Project through (tool option) | Ignored in Wrap (disabled in the UI). |
| 2D view | Shows the baked pixels; outline of the selected wrapped stamp from the parameterization (5.2), no gizmo (as for projected stamps). |
| `stamp_at` (click to select) | Uses the caches, unchanged. |
| Brush, fill, eyedropper | Unaffected. |

---

## 4. Package A — core (`wte_core`)

### 4.1 New public items

```rust
// stamp.rs
pub enum SurfaceMode { Project, Wrap }               // default Project; Projector gets `pub mode: SurfaceMode`
pub struct WrapOptions { pub jump_gaps: bool, pub overlap: bool }   // default false/false; Projector gets `pub wrap`

// wrap.rs (new; the builder may split it into wrap_param.rs + wrap_bake.rs)
/// The parameterization of the surface around one wrapped stamp. Immutable, shared by Arc.
pub struct WrapParam { /* see 4.3; plus the inputs it was made for: mesh Arc pointer, origin, rotation bits,
                          spine axis, reach (with headroom) */ }
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WrapStats {
    pub verts: u32, pub tris: u32,
    pub cut_tris: u32,          // triangles re-unfolded as tears (4.3 step 6)
    pub covered: f32,           // share of the picture rectangle that has surface under it (param areas, 0..1)
    pub met_itself: bool,       // the spine closed on itself (longer than the way around)
    pub reached_edge: bool,     // the front met a boundary edge inside the rectangle
    pub limited: bool,          // WRAP_MAX_VERTS or WRAP_MAX_TURNS reached
    pub jumps: u32,             // separate parts entered across a gap (Jump small gaps)
    pub turns: u32,             // layers laid (1 unless Overlap and the spine met itself)
    pub micros: u64,            // time to build
}
/// None when the projector is unusable (non-finite, zero size) or no triangle lies within reach of `origin`.
pub fn wrap_param(mesh: &MeshData, bvh: &Bvh, maps: &[Arc<TexelMap>], src_size: [u32; 2], p: &Projector) -> Option<WrapParam>;
impl WrapParam {
    pub fn stats(&self) -> WrapStats;
    /// True when this parameterization serves `p` on `mesh` (4.4).
    pub fn serves(&self, mesh: &Arc<MeshData>, p: &Projector) -> bool;
    /// Surface -> stamp-local coordinates (world units along the projector's right / up, origin at the anchor).
    pub fn local_of(&self, tri: TriId, bary: [f32; 3]) -> Option<Vec2>;
    /// Stamp-local coordinates -> surface point (`texel_map::SurfacePoint`); None where no surface lies under them.
    pub fn surface_of(&self, mesh: &MeshData, local: Vec2) -> Option<SurfacePoint>;
}
/// The wrap bake: one cropped buffer per touched set (ascending set id), like `bake_projected`.
pub fn bake_wrapped(mesh: &MeshData, maps: &[Arc<TexelMap>], param: &WrapParam, src: &PremulMips, tint: [u8; 4],
                    p: &Projector, quality: BakeQuality) -> Vec<(TextureSetId, SparseBuf)>;

// doc_stamps.rs (Document)
pub fn ensure_wrap_param(&mut self, id: StampId) -> Option<WrapStats>;   // computes if missing (UI thread)
pub fn stamp_wrap_stats(&self, id: StampId) -> Option<WrapStats>;         // from the cache
pub fn stamp_wrap_outline(&self, id: StampId, samples_per_edge: usize) -> Vec<Vec<SurfacePoint>>; // picture border on the surface, split where it leaves the surface
pub fn stamp_wrap_local(&self, id: StampId, tri: TriId, bary: [f32; 3]) -> Option<Vec2>;
pub fn stamp_wrap_surface(&self, id: StampId, local: Vec2) -> Option<SurfacePoint>;
/// Sum over the stamp's linked caches of alpha / 255 times the world area of each texel (from its owning triangle's
/// world / UV area ratio). For tests and the scenario: compare with the picture's opaque area.
pub fn stamp_ink_area(&self, id: StampId) -> f32;
```

### 4.2 Constants

`WRAP_HEADROOM = 1.25` (reach computed with 25 % to spare, 4.4), `WRAP_MAX_VERTS = 1_000_000`, `WRAP_MAX_TRIS =
3_000_000` (candidate cap), `WELD_SNAP = 1e-5 * diag`, `CONSENSUS = 0.25` (of the shortest incident edge),
`TEAR_RATIO = 2.0` (edge stretch outside [1/2, 2] or a flip = tear), `SPINE_NUDGE = 1e-6` (of the edge length),
`GAP_JUMP = 0.01 * diag` (Jump small gaps), `WRAP_MAX_TURNS = 8` (Overlap).

### 4.3 Algorithm (`wrap_param`), all in f64, serial, deterministic

1. **Reach.** `(hw, hh)` = |half_size|; border `bx, by` = one source texel in world units and `tw` = largest
   `texel_world` of the maps (as `projected_jobs`). Spine axis = the picture's longer axis (x when `hw >= hh`).
   `along = (half extent along the spine + border + 2 tw) * WRAP_HEADROOM`, `across` likewise. Every point of the
   picture is within geodesic distance `along + across` of the anchor, and Euclidean <= geodesic, so
   `bvh.query_sphere(origin, along + across)` returns every triangle that can matter (all of them, NO_UV included:
   the sticker passes over them; they only receive no pixels). More than `WRAP_MAX_TRIS` → keep the nearest ones
   (`query_sphere_dist`), `limited`. With Jump small gaps the radius grows by `4 * GAP_JUMP` (a jump counts only the
   gap's part along the surface direction, 4.3a). With Overlap the along reach is not cut at one turn (4.3b).
2. **Weld and anchor.** Weld the candidates by bit-exact position (`0.0 == -0.0`; a fast hasher like `tip_bake.rs`'s
   `PositionHasher`, copied — `tip_bake.rs` is not edited); then snap each boundary-edge vertex to another
   boundary-edge vertex within `WELD_SNAP` (spatial hash, 27 cells); drop triangles that collapse. CSR adjacency
   vertex → triangles; connected parts by union-find over the welded vertices (`part[v]`, used by 4.3a). Anchor triangle: the candidate(s) nearest `origin` (within `1e-4 diag` of the nearest), the one
   with the largest `dot(geo_normal, out)`, then the lowest id. Anchor point = closest point on it; normal `n` = its
   interpolated shading normal flipped towards `out`. Frame: `right` turned by the shortest arc from `out` to `n`
   (as `moved_to`), `up' = n x right'`.
3. **Spine.** From the anchor trace a straightest path in `+spine` and `-spine`: straight inside a triangle to its exit
   edge; across an edge, continue in the neighbour unfolded about that edge (same angle to the edge); an exit within
   `SPINE_NUDGE` of a vertex is moved along the edge towards its lower-index end (deterministic). Stop at the reach,
   at a boundary edge (`reached_edge`), when entering a triangle either trace already crossed (`met_itself`), or after
   `4 * patch triangles` steps. The crossed triangles form a strip whose planar development has the path on the
   `along` axis: their vertices get exact (along, across) coordinates and become the seeds (a vertex of several strip
   triangles takes its first, the nearest to the anchor in trace order). Seeds also get a frame (the 2x3 map of
   their strip triangle's development) and an oriented normal (shading normal flipped to agree with `n` at the
   anchor and then along the strip).
4. **Front.** Binary heap keyed by `|across| + max(0, |along| - spine length on that side)` (ties: lower local vertex
   index). When vertex `v` is finalized, for each triangle `(v, a, w)` with `a` final and `w` not: candidate for `w` =
   the triangle unfolded onto `(P_v, P_a)` by the similarity of its own development, on the side whose orientation
   matches the triangle's orientation relative to `v`'s oriented normal (not the winding: works on inconsistent
   winding and Möbius strips). A triangle with only `v` final gives `w` (and `a`) a fallback candidate through
   `v`'s frame (`P_v + M_v (p - p_v)`), used only when no triangle candidate exists. Tentative `P_w` = mean of the
   triangle candidates if all lie within `CONSENSUS` of it, else the candidate whose producing vertices have the
   smallest key. On pop: `P_w` final, frame `M_w` from the triangle that produced its candidate, oriented normal
   flipped to agree with the producer's. Stop popping when the key exceeds `across` (with headroom) or
   `WRAP_MAX_VERTS` are final (`limited`).
5. **Halo.** Each triangle with 1 or 2 final vertices gets its missing corners by the same unfolding / frame rule,
   stored per triangle (not propagated), so triangles straddling the reach are complete.
6. **Tears.** For every patch triangle: flipped param orientation (relative to the oriented normals) or an edge
   whose param / 3D length ratio is outside `[1/TEAR_RATIO, TEAR_RATIO]` → re-unfold it rigidly from its vertex with
   the lowest key through that vertex's frame; count `cut_tris`.
7. **Store** per patch triangle: `TriId` and three `[f32; 2]` stamp-local coordinates (spine axis swapped back to
   x = right, y = up); `covered` = area of the param triangles clipped to the picture rectangle / its area (clamped
   to 1); a lazy (`OnceLock`) uniform grid over the param bounding box for `surface_of`. With Overlap the store
   holds one such triangle list per turn (4.3b); `local_of` / `surface_of` / the outline use the top turn.

### 4.3a Jump small gaps (`p.wrap.jump_gaps`)

Off: steps 1-7 exactly as above (bit-identical output; WA16 checks it). On, and only when the candidates contain more
than one part (`part[]` has two or more values; with one part there is no extra cost and the output is byte-identical
to off):

- **Where a jump starts.** When a vertex `v` is finalized (front or seed) and some part other than `part[v]` has not
  been entered yet, `bvh.query_sphere_dist(p_v, GAP_JUMP)` (candidates only) → the nearest point `q` on a triangle
  `t` of a not-yet-entered part (`closest_point_on_triangle`, bvh.rs; ties: lower triangle id). The query is skipped
  for vertices whose part has no other part within `GAP_JUMP` of its bounding box (a box test per part pair, built
  once in step 2), so a model of well separated parts pays nothing per vertex.
- **Where the front lands.** `P_q = P_v + M_v (q - p_v)`: the gap is crossed as flat ground in `v`'s tangent plane
  (its part along the surface counts, its part along the normal does not, so the picture does not shift sideways
  over a step). Triangle `t` is developed rigidly with `q` at `P_q`, its plane turned onto `v`'s tangent plane by the
  shortest arc between its oriented normal (flipped to agree with `v`'s) and `v`'s; its three vertices become seeds of
  the front with key `key_v + |q - p_v|`, frames from that development; the part is marked entered. The front then
  grows over the new part as over any surface (steps 4-6). A part is entered once, from the first jump in key order
  (deterministic); later jumps into it are ignored, so a part is never laid twice.
- **The spine jumps too**, so a line of text keeps its line across parts: when a trace leaves through a boundary edge
  at point `e`, it looks for the nearest point `q` on another part within `GAP_JUMP` of `e`, continues from `q` in the
  trace direction turned onto `q`'s triangle plane (shortest arc of the oriented normals), and its arc length grows by
  the tangential part of `q - e`. Without such a point the trace stops as before (`reached_edge`).
- **Guaranteed**: exact continuation across a gap between coplanar or parallel-offset parts (WA16); continuous along
  the spine across any jump; never a smear (tears and consensus rules apply unchanged). **Degrades**: a part met at a
  steep angle or only at one corner gets its placement from that one point, so its distortion follows its own shape;
  a gap wider than `GAP_JUMP` still stops it; parts stacked on top of each other (gap along the normal) both receive
  the picture. **Cost**: one small BVH query per front vertex of a part that has another part within `GAP_JUMP`;
  measured by WA15 (limit G5f).

### 4.3b Overlap (`p.wrap.overlap`)

Ends meet (off): steps 1-7 as above. Overlap (on), and only when a trace meets itself (else byte-identical to off):

- **Turns.** The two traces do not stop at a self-meeting; each runs on to its reach (`along`), at most
  `WRAP_MAX_TURNS` turns in all (`limited` beyond, the rest of the picture is dropped). The traces are split into
  **turns** at their meeting points: turn 0 = the Ends-meet spine (both traces up to where they meet), turn +1 = the
  `+` trace from that meeting point until it re-enters triangles of turn +1 itself or reaches its reach, turn +2 the
  next piece, and likewise -1, -2 for the `-` trace. Each turn keeps the trace's arc length as its `along` values,
  so the picture continues without a jump from turn to turn (the real straightest path: on a cylinder it closes
  exactly, on a bottle shoulder it may spiral, as tape would).
- **One parameterization per turn**: steps 3 (seeds = that turn's strip), 4, 5 and 6 run per turn, each
  single-valued over the patch (a turn's front covers the band once, with a cut where its own ends meet, as in Ends
  meet). Jump small gaps applies inside each turn. The result is a stack of layers `k = -K..K` (`turns` = count).
- **Which is on top**: the layer whose picture coordinate is later along the picture's x axis (`flip_x` reverses it),
  like tape wound from the picture's start: its end lies on top. For a picture whose spine is its y axis (taller
  than wide) the same rule uses y, top of the picture last.
- **Painting** (`bake_wrapped`, 4.5): layers are rastered in back-to-front order (earliest first), each into its own
  layer scratch with the max-alpha rule inside the layer (idempotent for gutter and tiled double visits); after each
  layer the layer scratch is composited **over** the set scratch with the core's Normal blend at 100 %
  (`blend_pixel`, blend.rs; straight alpha, deterministic) and cleared. An opaque picture therefore shows the top turn
  only; a see-through text shows the turn below through its gaps. Memory: one extra scratch per set group.
- **Guaranteed**: on a cylinder every texel shows exactly the composite of the turns that lie over it in the stated
  order (WA18); the spine's arc length is continuous across turns. **Degrades**: on surfaces where the path spirals,
  turns drift sideways; turns beyond 8 are dropped (stats and panel say so). **Cost**: about one parameterization and
  one raster per turn (limit G5f).

### 4.4 Reuse of a parameterization

`Document` gets `wrap_params: HashMap<StampId, Arc<WrapParam>>` (derived data: not in snapshots, not saved, dropped
when the mesh `Arc` changes and when a stamp is removed; at most 16 entries, least recently used dropped first).
`serves(mesh, p)`: same mesh `Arc` (pointer), `origin` and `rotation` bit-equal, same spine axis, same `WrapOptions`,
and the needed
reach <= the stored reach. So scaling up to 125 %, tint, flips, text edits that keep the spine axis and every Final
after a Preview reuse it; a move or a turn recomputes. Bakes from a reused and a fresh parameterization are
byte-identical (the parameterization depends only on the inputs `serves` compares plus the reach, and the reach only
limits where the front stops — the builder makes sure the coordinates of a vertex do not depend on the stop; if that
cannot be kept exact, `serves` requires an equal reach and the 25 % headroom is dropped; say which in the notes).

### 4.5 `bake_wrapped`

Per patch triangle with a texture set (skip NO_UV / set-less): stamp-local corners `c0..c2`; skip when their box
misses the picture rectangle grown by the border. Footprint: clip the local triangle against the rectangle (the 2D
form of `Poly::clipped` in `stamp_bake.rs`), map the clipped polygon's barycentrics to UV px, grow by `gutter + 2` →
the `raster_tri` clip box. Scratch per set, bands, scratch groups, crop: the same structure and helpers as
`raster_jobs` (`BAND_ROWS`, `SCRATCH_BUDGET`, `scratch_groups`, `crop_scratch`, `shade`, `tint_factors`,
`SAMPLE_ALPHA_MAX` become `pub(crate)` in `stamp_bake.rs`: visibility only, no logic change). Per fragment:
`l = sum(b_i c_i)` (gutter barycentrics extrapolate it: seam padding as for projected stamps); reject unless
`|l.x| <= hw + bx`, `|l.y| <= hh + by`; `u = 0.5 + l.x / (2 hw)`, `v = 0.5 - l.y / (2 hh)`, flips; `lod =
log2(max(src_px_per_world * texel_world_tri * sqrt(param_area / world_area), 1))`; `Full`: `src.sample`, `Preview`:
`src.sample_nearest_level`; `shade(sample, tint, 1.0)`; max-alpha rule and the alpha-255 skip as in 6.5 step 3. No
angle weight, no depth fade, no occlusion. Fragments of a texel arrive in ascending triangle order: the result does
not depend on the thread count. With Overlap and more than one turn: per layer as above, then composited over (4.3b);
with one turn the code path is the single-layer one (no compositing step), so Ends meet and Overlap without a
self-meeting give identical bytes.

### 4.6 Document and job integration

- `stamp_job.rs`: `StampBakeJob` gets `wrap: Option<Arc<WrapParam>>` (the cached one when it `serves`), `run()` matches
  `Placement::Projected(p)` on `p.mode`: `Project` → `bake_projected` exactly as today; `Wrap` → reuse or
  `wrap_param(..)`, then `bake_wrapped(.., Full)`. `StampBakeOutput` carries the parameterization it used;
  `apply_final_bake` stores it when it applies the output. `make_bake_job` fills `wrap`.
- `doc_stamps.rs` `bake_and_apply(.., Preview)` (`:1113-1117`): the same dispatch (cache, compute and store when
  missing). `set_stamp_placement` / `add_stamp`: nothing new beyond the field. `remove_stamp`, mesh replacement:
  drop the cache entry. `refit_auto_depth` unchanged.
- Log lines (the gate greps them): `wrap: stamp N param V verts T tris in X ms (fresh|reused), covered C %, cut K`
  (info for Full bakes, debug for Preview) and the existing `final bake:` lines.
- `samples.rs`: `cylinder(segments, radius, height)` (open prism, one UV island with its seam on one side, 64 gon in
  tests) and `bumpy_plate(cells, amplitude)` (height field of sine bumps, max slope parameterized).

### 4.7 Tests package A adds (all in-file or in `tests\`, headless; numbers are the acceptance)

Test picture `cells(512, 256)`: 8 x 4 cells of distinct, far-apart colours with a smooth gradient inside each cell.
"Ground truth" = the same `PremulMips::sample` call at the analytically unrolled coordinates with the same `lod`.
"Smear" = an inked texel whose colour is within 12 levels of no cell colour and of no 50/50 mix of two neighbouring
cells. Measured on texels whose analytic coordinates lie at least 2 source texels inside the picture.

| # | Test | Limit |
|---|---|---|
| WA1 | Cylinder (64 gon, r 1, h 2, 1024² set), picture 0.9 of the way round x 0.5 h, centred away from and on the UV seam: per-texel error vs the polygon's exact unrolling (perimeter arc length) | 99.9 % of texels within 2/255 per channel, max 8/255, in both cases |
| WA2 | Same, coverage: `stamp_ink_area / picture opaque area`; per cell area / expected | 0.99-1.01; each cell 0.97-1.03. The projected bake of the same projector is logged for comparison |
| WA3 | Same cylinder, picture 1.5 x the way round | band texels inked >= 99 %; `met_itself`; smear <= 0.5 % of inked; `cut_tris` <= 2 per triangle row of the band |
| WA4 | `bumpy_plate` (64 x 64 cells, max slope 60°), picture 0.6 of the plate | ink area 0.90-1.10 of the picture; every cell 0.75-1.30; smear <= 1 %; `covered` >= 0.999 |
| WA5 | `seam_cube` (six UV islands): picture across one edge — error vs the unfolded cube net; picture over a corner | edge: as WA1; corner: smear <= 0.5 %, `covered` >= 0.70, `cut_tris` <= 8 |
| WA6 | UV seams: every UV copy of a welded vertex gets bit-identical coordinates (unit test). Sphere sample (`uv_sphere` seam): seam score <= 1.25 x interior score + 2 levels (seam score: mean max-channel difference over texel pairs on the two sides of the seam whose `locate` points lie within 0.75 texel of each other; interior: the same over horizontally adjacent texels of one island under the stamp) | as stated |
| WA7 | Robustness: `thin_slab`, `fold`, `two_blocks`, `t_junction_quads`, `cracked_quads`, `open_box`, `overlapping_cubes`, `doubled_cube`, `coin`, `cup`, `saddle_cup`, `soup_sets`, `no_uv_tetra`, `mirrored_quads`, `wrapped_quad`, reversed winding, degenerate triangles; pictures 0.01x-1000x the model, zero / NaN size and origin | no panic, no non-finite coordinate, every pixel finite, <= 200 ms each |
| WA8 | `two_blocks(gap)`: the other block receives nothing; `cracked_quads(1e-6 diag)`: continuous (ground truth as WA1); `thin_slab`: a picture at the rim continues onto the back face (ground truth: unfolded slab) | as WA1 where a ground truth exists |
| WA9 | Determinism: 1-thread pool vs the default pool, Preview and Full, byte-identical; two runs identical | equal |
| WA10 | Toggle: Project → Wrap → Project (Finals) leaves caches byte-identical to the start; undo / redo restore each; duplicate keeps the mode; merge down and rasterize of a wrapped stamp equal its cache | equal |
| WA11 | Reuse (4.4): scale x1.2, tint, flip and a text edit reuse (`fresh` count unchanged); move and turn recompute; reused and fresh bakes byte-identical | equal |
| WA12 | Deferred: `deferred_final_bakes_equal_the_synchronous_ones` extended with a wrapped image and a wrapped styled text (Arc warp) | equal |
| WA13 | Format: round trip of a wrapped stamp (mode and both options kept, bit-exact floats); options written only when `true`; `required_reader` 3 with a wrapped stamp or a set option (also on a projected stamp), unchanged otherwise; a manifest without `mode` = Project; unknown `mode` → Project + `StampFeatureUnknown`; the round-5 fixtures load unchanged | as stated |
| WA14 | Unchanged: every existing test passes with unchanged expected values (8 layout hashes, `plain_text_is_bit_identical`, stamp_bake tests, `tiled_uv_bakes_equal_the_plain_reference`) | equal |
| WA16 | Jump small gaps: `two_blocks(gap)` with coplanar tops, gap 0.5 % of the diagonal, picture across both. Off: second block nothing, bytes equal to the default design. On: `jumps == 1`; error vs the flat unrolling with the gap's width counted: 99.9 % of texels within 2/255, max 8/255; a text line across the gap: its baseline row continuous (same ground truth). Gap 2 %: on = off. One-part meshes (sphere, cylinder): on = off byte for byte | as stated |
| WA17 | Jump small gaps on `cube_on_floor` (cube and floor separate, touching), picture from the floor up the cube's side: on: the cube's side receives the continuation, ink area of the whole picture >= 0.95 of its opaque area, smear <= 0.5 %; off: the cube receives nothing. Determinism (1 vs N threads) with jumps | as stated |
| WA18 | Overlap on the WA1 cylinder, picture 1.5 x and 2.5 x the way round: `turns` 3 / 3 (centred: turn 0 plus one each side); every texel equals the ground truth composite of the turns over it (analytic `along = arc + k * C`, layers later along x on top, Normal blend): 99.9 % within 2/255, max 8/255; with `flip_x` the other end is on top; a see-through text (alpha 0 between letters) shows the turn below in its gaps (same ground truth). Picture 0.9 x the way round: Overlap = Ends meet byte for byte | as stated |
| WA19 | Overlap limits and options together: 20 x the way round → `turns == 8`, `limited`, no panic, <= 8 x the one-turn time + 50 ms; Overlap + Jump on `two_blocks` and on the cylinder: no panic, deterministic; option changes are one history entry each, undo / redo, kept through Project and back (caches byte-identical after the round trip) | as stated |
| WA15 | Ignored probe `wrap_bench_models` (also with Jump on, and Overlap at 2.5 turns on a cylinder) (release, gate runs it): Sphere sample, `ngons_tiling.obj`, `ABeautifulGame.glb` (skipped when absent), a 20 % decal at the spot `stamp_bench_tiled_model` uses and a 100 % one: param fresh / reused, Preview, Full, against `bake_projected` Preview / Full of the same projector | reported; limits in 8.3 |

### 4.8 Files (package A)

Core: new `wrap.rs` (or `wrap_param.rs` + `wrap_bake.rs`), `stamp.rs`, `stamp_bake.rs` (`from_surface` sets the mode;
`pub(crate)` on the helpers of 4.5; no other change), `stamp_job.rs`, `doc_stamps.rs`, `document.rs` (cache field,
invariant: only ids of existing stamps), `lib.rs`, `samples.rs`, `error.rs` (`StampFeatureUnknown`),
`project\manifest.rs`, `project\save.rs`, `project\load.rs`, `project\migrate.rs`, `project\mod.rs`,
`tests\project_roundtrip.rs`; `blend.rs` (`blend_pixel`) and `bvh.rs` (`closest_point_on_triangle`) are used, not edited. `ARCHITECTURE.md`: 1.6 (new "Round 6" paragraph), 2.1 (file list), 5.12
(`SurfaceMode`, the field, the wrap API), 5.14 (`wrap_params`), 5.20 (the Document functions of 4.1, the mode in
`set_stamp_placement`'s doc), new 6.5a "Wrap bake" (4.3-4.5), 7.2 / 7.3 (`mode`, FORMAT_VERSION 3,
`required_reader`), 9.1 (tests). No app file.

**Done means (A):** 4.7 all pass; `cargo test --workspace` and clippy clean (the app untouched and still green);
release build; `stamp`, `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run`, `final_bake_async` run
with the release exe and their `shot.png` equal `wte_round5.exe`'s (`shot_diff.py`, toast area excepted); WA15 run
once and recorded in `notes\pkg_a.md`.

---

## 5. Package B — app (`wte_app`), built on A's checked copy

### 5.1 Changes

- **`state.rs`**: `StampSettings::mode: SurfaceMode` and `StampSettings::wrap: WrapOptions` (shared by the Stamp and
  Text tools, like `project_through`), defaults Project / off / Ends meet in `AppState::new`.
- **`app.rs`** (`WteApp::new`): `WTE_STAMP_MODE=wrap` sets the default to Wrap (measuring aid, logged like
  `WTE_SYNC_FINAL_BAKES`).
- **`panels\tool_options.rs`**: `stamp_options`: a two-button selector "On the model: Project | Wrap" (hover texts:
  "Like a projector from the front: sharp from this side, stretched at the sides" / "Lies on the surface like a
  sticker: goes round the object and over bumps"); "Project through" disabled in Wrap. `placement_fields` of a
  selected projected stamp: the same selector at the top, sending `UpdateStampPlacement(id, Projected(Projector {
  mode, ..pr }), Final)`; in Wrap the rows Depth front / back, Fit depth, Angle limits and the Backface cull /
  Occlusion checkboxes are hidden (Flip X / Y stay); below the selector one weak line from `stamp_wrap_stats`:
  "Wrapped over N triangles", plus a warning line when `covered < 0.98` ("P % of the picture runs off the surface:
  an edge, a hole or the end of the part"), when `met_itself` and not Overlap ("The ends meet at the back"), when
  `turns > 1` ("Wound N times"), when `jumps > 0` ("Continued onto N separate parts"), when `limited` ("Too large for
  this model: the wrap stops at 1 million vertices / 8 turns").
  **The two Wrap options**, under the selector in both places (tool options for new stamps and the selected stamp),
  shown in Project mode too but greyed out (hover: "Used in Wrap mode"): checkbox **"Jump small gaps"** (hover:
  "Continue onto separate parts closer than 1 % of the model size; off: stop at the edge of a part") and a two-button
  selector **"Longer than the way round: Ends meet | Overlap"** (hover: "Ends meet: the middle part shows, the ends
  meet at the back" / "Overlap: keeps winding like tape, the end on top"). On a selected stamp each sends
  `UpdateStampPlacement(id, Projected(Projector { wrap, ..pr }), Final)` (one history entry; disabled when locked).
- **`panels\text_block.rs`** (`:101`): the same selector and the two options for the Text tool next to "Project
  through".
- **`tools\stamp_place.rs`**: new 3D placements get `mode = state.tools.stamp.mode` and `wrap = state.tools.stamp.wrap`; depth fitted as today (also in
  Wrap, for switching back); hover outline and hover picture stay flat (5.3 default 6).
- **`tools\transform.rs`**: no change in the drag maths. In Wrap: no `backface_cull = false` on a back-face hit;
  `overlay` draws the wrapped outline (below). On selecting a wrapped stamp whose cache has no parameterization (after
  opening a project) call `doc.ensure_wrap_param(id)` once (logged with its time).
- **`view\gizmo.rs`**: `gizmo_shape` unchanged for handles (corners, centre, knob from the flat frame: hit tests and
  the scenarios' `selected_gizmo` keep working); `depth_box = None` in Wrap. New `wrapped_outline(doc, id, xf) ->
  Vec<Vec<egui::Pos2>>` from `stamp_wrap_outline(id, 32)` (points facing away from the camera dropped).
- **`view\mod.rs`** `selected_outline` (`:643`, `:653`): Wrap in 3D → the wrapped outline; Wrap in 2D → the outline's
  surface points mapped to the active set's UV px (skip other sets).
- **`tools\text_path.rs`**: in Wrap the path point handles are drawn at `stamp_wrap_surface(id, local)` (flat frame
  where it returns None); dragging a handle raycasts and converts with `stamp_wrap_local(id, hit.tri, hit.bary)`.
- **`panels\layers.rs`**: the 3D badge's hover text says "wrapped on the surface" for Wrap.
- **`selftest\scenarios.rs`** (+ `selftest\mod.rs` only if a new step kind is needed): scenario `wrap_surface` (5.2);
  `scenario_names().len()` 38 → 39.
- `ARCHITECTURE.md` 8.2 (`StampSettings::mode`, env), 8.6 (Stamp, Text, Transform rows), 8.7 (tool options, text
  block, layers badge), 8.11 (`wrap_surface`, 39 names), 9.2 (tests).

### 5.2 Scenario `wrap_surface` (PATH optional)

Built-in Sphere sample, 3D view, 1280 x 800. A generated 512 x 256 `cells` picture registered as the stamp image
(Setup step, PNG bytes from a `PixelBuf`). Each assert prints its measured value to the log.

1. Stamp tool, mode Project, size 30 %, click left of centre → stamp S. Wait. Remember composite H0.
2. Toggle S to Wrap (`UpdateStampPlacement`, Final), wait. Assert: `stamp_ink_area / opaque area` in 0.93-1.07;
   `covered >= 0.999`; `cut_tris == 0`; composite changed. Remember H1.
3. Toggle back to Project, wait → composite == H0. Undo → H1; Undo → H0; Redo twice → H0 (Project).
4. Wrap again; scale S so its width is 1.2 x the sphere's circumference → `met_itself`; equator band (texels whose
   `locate` point lies within 0.9 hh of the spine's great circle) inked >= 99 %; smear <= 1 %. Remember H2.
   4b. Switch S to **Overlap** (Final) → `turns == 3` (0, +1, -1), one history entry; in the two overlap stretches of
   the band (where two turns lie; found from the turns' `along` ranges) >= 90 % of inked texels carry a colour of the
   picture's right half (the turn later along x on top; the wrong order would show left-half cells); Undo →
   composite == H2. Overlap again, toggle to Project and back to Wrap → still Overlap, composite equals the Overlap
   state. Switch back to Ends meet → H2.
   4c. **Jump small gaps** on (Final): the sphere is one part → composite == H2 byte for byte, `jumps == 0`, one
   history entry; Undo. (The real jump is tested in the core, WA16 / WA17, and in the gate sweep on ABeautifulGame.)
   4d. In Project mode the two option controls are present and disabled (`AppState` panel probe or the control
   sweep's widget lookup).
5. Scale back to 30 %, rotate so the picture covers the sphere's UV seam (the builder finds the seam meridian with
   `locate`) → WA6 seam score assertion on the composite.
6. Drag the body by 25 % of the view (runner `drag` over the model) → origin moved by >= 0.3 r along the surface;
   ink ratio still 0.93-1.07; one history entry for the drag.
7. Text tool, mode Wrap, "WRAP AROUND", Arc warp bend 30, place; Final → ink area > 0, `covered >= 0.999`.
8. `BakeWait(false)`: toggle S, `Undo` in the next frame, wait → composite equals the state before the toggle.
9. Set S to Overlap + Jump small gaps, then save to a temp `.wtep`, reopen → composite equal (document pixels, not
   the screenshot), both stamps Wrap, S keeps both options, manifest `min_reader_version == 3` and has
   `wrap_overlap` / `wrap_gaps` only on S.
10. With a PATH: open it, place the `cells` picture at 40 % at the centre of the view in Wrap, wait; log
    `WrapStats` and the bake times; then the same with Jump small gaps on (log `jumps`); assert ink area > 0 and no
    error in the log (no coverage limit: open models).

### 5.3 Tests package B adds

`tool_options`: the selector toggles a selected stamp with one history entry, is disabled when locked, hides the
projection rows in Wrap; the Stamp / Text tool selectors set `StampSettings::mode` and new placements carry it; the
two option controls are disabled in Project, each change on a selected wrapped stamp is one history entry, and new
placements carry `StampSettings::wrap`.
`transform`: a body drag of a wrapped stamp gives Previews and one Final, the mode survives; `gizmo`: `depth_box`
None in Wrap, outline non-empty after `ensure_wrap_param`. `text_path`: a handle of a wrapped curved text lies on the
surface (distance to the mesh < 1e-4 diag). The env switch.

**Done means (B):** 5.3 pass; workspace tests and clippy clean; release build; `wrap_surface` ok on the built-in
sample and with `WaterBottle.glb`, and equal with `WTE_SYNC_FINAL_BAKES=1`; the seven round-5 scenarios of 4.8 still
pixel-identical to `wte_round5.exe`; `shot.png` of `wrap_surface` read and described in `notes\pkg_b.md`.

---

## 6. Process

| Order | Agent | Works in | Output |
|---|---|---|---|
| 1 | build:A (Opus) | `work\pkg_a` | code, tests, notes `pkg_a.md` |
| 2 | check:A (Opus): reads the diff against 2-4, tries to break it (WA7-type meshes of its own, a stamp at a mesh boundary, a huge text on a small part, undo during a bake, reopen, Jump across three parts, Overlap at 10 turns), fixes, reruns tests and clippy | `work\pkg_a` | notes |
| 3 | build:B (Opus) | `work\pkg_b` = copy of checked `pkg_a` | code, scenario, notes `pkg_b.md` |
| 4 | check:B (Opus): diff against 3 and 5, the interactions table 3.3 row by row (as tests or scenario steps), fixes | `work\pkg_b` | notes |
| 5 | gate 1 (merge, tests, scenarios) | `work\merged` | `gate1.md`, exe |
| 6 | gate 2 (measurements, report) | runs under `target\round6\runs_gate*` | `gate2.md`, `GATE_REPORT.md` |

Ownership: A owns the core and its `ARCHITECTURE.md` sections (4.8); B owns the app and sections 8.x / 9.2. B does not
edit core files: a needed core change goes into `notes\pkg_b.md` as a request with the exact code; check:B or the
gate applies it (and reruns the core tests). Builders write the plan's numbers next to their measured values in
their notes. The builder of A works in this order, so a context limit leaves usable work: (1) data model + format +
`from_surface` (all green), (2) `wrap_param` with WA1 / WA3 / WA5 / WA6 / WA7, (3) `bake_wrapped` + job / doc
dispatch, (4) cache, queries, WA2 / WA4 / WA8-WA14, (5) the two options (4.3a, 4.3b, WA16-WA19), (6) `ARCHITECTURE.md`
(6.5a includes both options). B's work order puts the option controls right after the toggle.

---

## 7. Gate

### 7.1 Gate 1 (merge, tests, scenarios)

1. `merged` = `pkg_b` (B started from A); check with a three-way diff against `base` that B changed only B's files
   (plus applied requests). `cargo test --workspace` (round 5: 1,067 passed, 32 ignored — the count only grows,
   0 failed), clippy `--workspace --all-targets` 0 warnings, no `todo!` / `unimplemented!`, release build; copy to
   `target\round6\wte_round6.exe`, record SHA-256.
2. All 39 scenarios at 1280 x 800 (`sweep.ps1` jobs file as round 5 + `wrap_surface`), and `final_bake_async` and
   `wrap_surface` again with `WTE_SYNC_FINAL_BAKES=1` (composites equal). Read `shot.png` of `stamp`, `text_warp`,
   `wrap_surface`.
3. **Pixel identity** (G2): `stamp`, `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run`,
   `final_bake_async` on the built-in sample: `shot.png` equal to `wte_round5.exe`'s (toast area excepted), exported
   files byte-identical (`check_exports.py` PASS).
4. **Compatibility** (G4): a project saved by `wte_round5.exe` opens in round 6 with an equal composite; a round-6
   project without wrapped stamps opens in `wte_round5.exe` (`open_path` scenario) with only the `NewerFormat`
   warning and an equal composite; a round-6 project with a wrapped stamp is refused by round 5 with `TooNew` and a
   readable message.

### 7.2 Gate 2 (measurements; idle machine, one process at a time, release, `WTE_FRAME_LOG=1`, 2 runs per line)

"Wrap" = the same exe with `WTE_STAMP_MODE=wrap` (identical clicks, so the stamp steps compare like for like),
alternating with the default (Project) run.

| # | Line | How |
|---|---|---|
| G5a | Built-in sample, `stamp` scenario: Wrap Full bake (`applied after`) <= **1.5 x** Project and <= **80 ms**; Preview bake frames <= **30 ms** | log, `frames.py` |
| G5b | `ngons_tiling` `workflow_path`, the three stamp steps: Wrap `applied after` <= **1.5 x** Project; no frame over **300 ms** between `started` and `applied`; longest frame during the drag <= Project's longest **+ max(30 ms, 25 %)** (Project today: Preview bake frames 148-198 ms) | `frames.py`, `wf_steps.py` |
| G5c | ABeautifulGame `workflow_path`, same three lines; plus the image stamp's worker bake <= **2.0 s** (Project: 1.4-1.7 s) | same |
| G5d | ABeautifulGame `text_warp` in Wrap: 0 frames over **300 ms** around the Finals; time to swap <= **1.5 x** Project (Project 3D: 629-1,075 ms) | `frames.py` |
| G5e | Core probe WA15: Wrap Full <= **1.5 x** projected Full; Wrap Preview <= projected Preview **+ max(30 ms, 25 %)**; param of the 20 % decal on ABeautifulGame <= **150 ms** fresh; 100 % decal param <= **3 s** | `cargo test --release -p wte_core -- --ignored wrap_bench_models` |
| G5f | Options (WA15 probe and logs): Jump small gaps on ABeautifulGame (chess pieces on the board, separate parts), 20 % decal across a piece and the board: param <= **1.5 x** the same with Jump off, `jumps >= 1`; Overlap at 2.5 turns on the cylinder: param + bake <= **(turns + 0.5) x** the Ends-meet time | probe output |
| G6 | Model sweep: `wrap_surface` with PATH (its step 10 runs Jump off and on) on WaterBottle, Avocado, DamagedHelmet, Lantern, ABeautifulGame, ngons_tiling, torus_knot, mobius, klein_bottle, nonmanifold, many_islands, flat_annulus, high_poly_sphere: all ok, 0 panics, table of verts / tris / covered / cut / jumps / times; read 4 screenshots (WaterBottle, Avocado, torus_knot, flat_annulus) | 13 runs, 1 each |
| G7 | Memory: ABeautifulGame `workflow_path` peak working set in Wrap <= Project + **300 MB** | process sampling as round 5's load log |
| G8 | Fuzz seed 7, 1,000 steps, ngons_tiling, with `WTE_STAMP_MODE=wrap`: 0 "panicked" / "Invalid parameter" / "Unknown layer"; central / dispatch frames over 500 ms <= 2 | `slow.py` |

About 30 runs. If a speed line fails only on ABeautifulGame because the parameterization is rebuilt on every Preview
of a drag, report the param share from the `wrap:` lines; do not change code in the gate unless the cause is small
and contained (the round-5 rule).

### 7.3 Report `target\round6\GATE_REPORT.md`

Verdict in plain words; exe SHA, test counts; every line WA1-WA19, G2-G8 with its number and source; the sweep table;
screenshots of what degrades (WaterBottle, a bump, a corner, a hole); what was not met and why; behaviour changes;
**to try by hand**: a text dragged round WaterBottle; 1.5 x round with Ends meet, then Overlap (end on top); Project →
Wrap → Project on an old stamp (identical); Avocado's bumps, DamagedHelmet's edges; a line across chess pieces with
Jump small gaps on / off; save, reopen, and open in round 5 (refused); a large wrapped text on ABeautifulGame (smooth
frames). Any cut item of section 10 is listed under "not delivered" (the two Wrap options are never cut silently).

---

## 8. Acceptance summary (what "done" means for the round)

1. Core accuracy WA1-WA6 (exact on developable surfaces, continuous across seams, whole picture on cylinder and
   bumps, clean cuts only where geometry forces them), robustness WA7-WA9, behaviour WA10-WA14.
2. The two Wrap options: Jump small gaps WA16-WA17, Overlap WA18-WA19; both saved (WA13), undoable, kept through
   Project and back, greyed in Project, settable on existing stamps (5.3, `wrap_surface` 4b-4d and 9).
3. App: toggle on existing and new 3D stamps and texts, gizmo / outline, scenario `wrap_surface` ok in both bake modes.
4. Unchanged: 1,067 existing tests, the seven round-5 scenarios pixel-identical, exports byte-identical, round-5
   projects open identically, round-6 files without wrap or options open in round 5.
5. Speed: G5a-G5f; memory G7; fuzz G8.

---

## 9. Risks

| Risk | Mitigation |
|---|---|
| Parameterization quality on bad meshes (cracks, T-junctions, non-manifold, inconsistent winding) | Weld + boundary snap; orientation from propagated normals, not winding; consensus and tear rules; WA7 / WA8 and the G6 sweep. |
| The spine through vertices or degenerate triangles | Nudge rule, step cap, degenerate input in WA7. |
| Heavy models: a large patch built on the UI thread during a drag | Cost grows with the stamp's area, not the model; vertex cap; reuse (4.4); Final on the worker. G5c / G5e; if too slow, a later round moves Preview bakes off the UI thread. |
| Jump small gaps lands a part badly (steep angle, one-corner contact) or costs a query per vertex | Placement from the first, nearest jump only; per-part box pre-test so separated models pay nothing; WA16-WA17, G5f, the ABeautifulGame sweep. |
| Overlap multiplies the work (one parameterization and raster per turn) and adds a compositing path | At most 8 turns; one-turn case uses the plain path (byte-identical); WA18-WA19 ground truth; G5f. |
| Memory (7 GB machine, ABeautifulGame at 2-2.8 GB) | ~100-150 MB peak per 1 M vertices, freed after the bake; Overlap keeps one turn's working set at a time; cache f32 per triangle, at most 16 stamps; G7. |
| Projected pixels change by accident | `stamp_bake.rs` gets visibility changes only; `Project` arm is the old call; WA14 + G2. |
| Format bump confuses round-5 users | Only files with wrapped stamps or set options need round 6; G4. |
| Reuse not exactly equal to a fresh build | WA11 compares bytes; fallback in 4.4. |
| Builder context limits (A is the large package) | Work order in 6 with green states between steps; options last in A's order. |
| Weekly usage (74 % at planning, reset 2026-11-23 01:00 UTC) | Six agents; cut order below; check usage before each agent. |

## 10. Cut order (if the weekly limit runs short; first cut first)

1. check:B folded into gate 1 (gate 1 reads B's diff against 5 and runs 3.3 as scenario steps).
2. Text-on-path handles on the surface (they stay on the flat frame; handles may float off the surface).
3. Gate 2 sweep reduced to WaterBottle, ABeautifulGame, ngons_tiling, torus_knot, 1 run per speed line.
4. Wrapped outline in the 3D gizmo and the 2D view (flat rectangle instead; the stats line still warns).
5. Parameterization reuse (4.4): every bake rebuilds it (slower Previews on heavy models; pixels the same).
6. Boundary snap in the weld (cracks then stop the wrap like a hole).
7. The spine's own jump in 4.3a (the front still jumps; a text line may bend slightly across a gap).
8. **Overlap**, then 9. **Jump small gaps**: last, because the user asked for them. Cutting one is allowed only after
   1-7 and only by an explicit decision recorded in the notes; the control is then not shown (no dead switch), the
   format keys stay reserved (read and kept), and `GATE_REPORT.md` lists it under "not delivered" with the reason.

Never cut: 3.1-3.2, 4.3, 4.5, 4.6, the toggle (5.1 tool options / text block / stamp place), `wrap_surface` steps
1-3 and 9, WA1-WA6, WA10, WA13-WA14, G2, G4.

---

## 11. Decisions (binding: the user's answers of 2026-11-21) and defaults

Answers to the four planning questions:

- **Q1 (binding)**: new texts and new images start in **Project**; the Stamp / Text tool options keep the chosen mode
  (and the two Wrap options) for the session. Existing stamps switch with the selector of the selected stamp.
- **Q2 (binding)**: **Jump small gaps** is a per-stamp option for texts and images in Wrap, off by default (= stop at
  the edge of a part); on = continue across gaps between separate parts up to a fixed 1 % of the model's diagonal
  (3.1, 4.3a).
- **Q3 (binding)**: **Longer than the way round: Ends meet | Overlap** is a per-stamp option for texts and images in
  Wrap, Ends meet by default; Overlap winds on like tape, the later part of the picture on top (4.3b).
- **Q4 (binding)**: the sticker **folds over every edge**; at a corner that cannot lie flat it is cut along one line.
- Both options: saved only when not default (3.2), undoable, kept when switching to Project and back, greyed out in
  Project, settable on an existing stamp.

Further defaults (the build goes ahead with these):

1. **The picture's long centre line lies exactly along the surface**; letters keep their size along a line of text,
   also on a ball. Far from that line on a ball the picture narrows (12 % at half the radius).
2. **Holes and open edges stop the wrap**; the panel says how much of the picture has no surface under it. Project
   mode still reaches separate parts.
3. **The hover outline before placing stays flat** (the wrapped picture appears after the click, with the round-5
   "Finishing stamp" status); the selected stamp shows its wrapped outline.
4. **Wrapped stamps ignore depth, angle, back-face and occlusion settings** (kept for switching back, hidden in Wrap).
5. **Files**: a project with a wrapped stamp or a set Wrap option needs round 6 to open; others open in round 5.
6. The gap distance is fixed (not a slider); a slider can be added later with the same format rule.
