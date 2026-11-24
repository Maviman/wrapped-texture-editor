# OBJECTS_TABS_PLAN — Objects (select / hide / isolate / frame) and two tab groups in the side panel

Branch `objects-tabs` (worktree `G:\software_projects\texture_wrapper_ap_ui`, cut from `main` @ 1f0be41). Planned 2026-11-24.
This plan is for a builder who has not seen the planning conversation. Read `HANDOFF.md` and `ARCHITECTURE.md` section 1 first, then the sections named in section 10 below.

---

## 0. Summary

Two features:

1. **Objects.** Importers keep the object structure of a file (OBJ `o` / `g`, glTF mesh nodes, FBX `Model`s). The mesh carries an object index per triangle. The user can select objects (list, or a click in the 3D view with the Navigate tool), **Hide selected, Isolate, Show all, Frame selection**. Hidden objects are not drawn and rays pass through them (painting, eyedropper, fill seed, stamps, wraps, gizmo). Selecting never restricts painting.
2. **Tabs.** The right side panel becomes two Photoshop-style tab groups with a draggable split: upper **Tool | Color | Texture sets | Objects**, lower **Layers | Brush | Text** (the existing strip). One page per group.

Key design choices (justified in the sections):

| Topic | Choice |
|---|---|
| Object identity | `MeshData::tri_object` + `MeshData::objects`; triangle order, vertex order and `uv_hash` do **not** change, `IMPORTER_VERSION` stays 2 |
| Skipping hidden objects | a hidden-triangle bit mask **inside `Bvh`** (cheap clone sharing the tree). Every ray and sphere query skips hidden triangles with zero call-site changes; worker jobs get a consistent snapshot through the `Arc<Bvh>` they already clone |
| Rendering hidden | a compacted "visible" index buffer rebuilt when visibility changes; wire instances grouped per object and drawn as visible runs |
| Highlight | one translucent tint pass over the selected triangles (new tiny pipeline + shader) |
| Viewport select | a click (no drag) with the **Navigate** tool; no modifier scheme on painting tools |
| Persistence | hidden object *names* in the app's `UiState` inside `Document::ui_state` (already round-tripped, already ignored by older builds). `FORMAT_VERSION` stays 3 |
| Undo / dirty | view state: not in history, no revision change, does not mark the document modified (same as camera and layer multi-selection, ARCHITECTURE 1.8) |
| Tabs | lower group = today's resizable bottom area, unchanged rules; upper group = the rest. No new crates, plain egui |

### 0.1 Three things to confirm with the user before or during the build

These are not blockers for work packages 1-3, but the user should see them:

1. **The colour picker is no longer always visible.** `HANDOFF.md` section 6 lists a binding decision: "The colour picker stays visible in the side panel whatever tool is active." The user's new decision (Color is one tab of the upper group) replaces it. Mitigations in this plan: a click on the toolbar's primary colour chip shows the Color tab; a tool change never leaves the Color tab (5.4); at 1280x800 the whole picker is on screen whenever the Color tab is shown. HANDOFF section 6 must record that the old decision is superseded.
2. **Stamps are baked against what is visible when they are baked** (3.4). A stamp placed while a pawn is hidden lands on the board under it; if the user later shows the pawn and edits that stamp, the re-bake sees the pawn. Saved projects keep the saved pixels (load does not re-bake a healthy cache).
3. **Shortcuts**: H = Hide Selected, Shift+H = Isolate Selected, Alt+H = Show All, **Shift+F = Frame Selection** (F stays "Frame Model", which now frames the visible objects). Alt+H must be tried by hand on Windows (possible system beep on Alt+letter; fallback Ctrl+Shift+H).

---

## 1. What I verified in the code (so the builder can trust it)

- `crates/wte_core/src/mesh.rs`: `RawMesh { triangles, materials }`, `RawTriangle { positions, normals, uvs, material }`; no object identity. There are **135** `RawMesh { .. }` literals and **4** `MeshData { .. }` literals in the workspace (`bvh.rs:1074`, `mesh.rs:216`, `mesh_build.rs:332`, `tip_bake.rs:622`). So `RawMesh` / `RawTriangle` must not get new fields; `MeshData` may.
- `mesh_build::normalize` (`mesh_build.rs:139`): drops degenerate triangles, stable-sorts by (material, NO_UV), unifies vertices with a key of (position, normal, uv, material). `uv_hash` = indices + uv bits + per-triangle set ids. Carrying an object index along does not touch any of these.
- Importers: `import/obj.rs` (`sanitize_obj` builds `TriInfo` per output triangle in file order; `o` and `g` are dropped at line 113 / the `_ => {}` arm), `import/gltf.rs` (`Loader::add_mesh` per mesh node, world transform baked, depth-first node walk around line 760), `import/fbx.rs` (`load_fbx` loops over `Model` objects, `add_geometry(geometry, &model_name, ..)`). All three return `RawImport { mesh, textures, warnings }` (`import/mod.rs:343`), 3 literal sites. `import/mod.rs:288` builds `NormalizeOptions` and calls `normalize`.
- `Bvh` (`bvh.rs`): private `nodes: Vec<BvhNode>`, `tri_order: Vec<u32>`; all ray queries go through `intersect` (leaf loop at ~line 371), all sphere queries through `walk_sphere` (leaf loop at ~line 488).
- Every production BVH query: `brush.rs` (574, 729, 738, 1510, 1562), `stroke.rs` (via `doc.bvh`), `stamp_bake.rs` (499-536 occlusion, 765 and 1284 candidates, 1139 depth fit), `wrap_param.rs` (2080, 2664, 2675, 2732, 2747, 2806, 3293, 3348, 3372, 3380), `document.rs:237` (`Document::raycast`), app: `tools/mod.rs:148` (`ViewXform::surface_at`), `tools/text_path.rs:168`, `tools/transform.rs:362`, `view/mod.rs:595` (`projected_outline_in_2d`). `tip_bake.rs` builds its own BVH for the brush-shape model and is not affected. **Candidates of brush dabs, projected bakes and wraps all come from BVH sphere queries**; the one exception is `wrap_param::collect_grown` (`wrap_param.rs:2722`), which grows over the welded graph `WrapMesh::vtris` after the anchor query.
- Stamp bake jobs clone `Arc<Bvh>` (`doc_stamps.rs:1391`, `1731`), so a mask stored in the `Bvh` travels with the job.
- `project::load` re-bakes a stamp only when its cache is missing / damaged / repaired (`load.rs:217-311`, `473`, `652`); normally saved pixels are kept.
- `Manifest::ui_state: serde_json::Value` is opaque to the core; the app writes `UiState` (`io_flows.rs:37`, `#[serde(default)]`, no `deny_unknown_fields`) at save (`start_save`, `io_flows.rs:470`) and restores it in `install_document` (`io_flows.rs:690`).
- Renderer: `GpuScene` (`gpu/scene.rs`) holds one vertex buffer, one index buffer, `draws: Vec<Draw>` (triangle ranges per material), `wire_all`, `wire_seams`, `wire_uv[set]` instance buffers. `Renderer::render_3d` (`renderer.rs:544`) draws background, mesh draws, wires. Pipelines are made by `view_pipeline(device, samples, PipelineSpec)` with `DepthMode::{Opaque, Overlay, Ignore}`; `lines.wgsl` pulls depth by `DEPTH_PULL`. `mesh.wgsl` does not use the primitive index.
- Input: `ToolController::press` (`tools/mod.rs:396`): with `ToolKind::Navigate` the primary button starts `Interaction::Orbit`; a click without a drag has no meaning today. `slot_for(Navigate) == None`.
- Shortcuts (`shortcuts.rs`): table of 36; H, Shift+H, Alt+H, Shift+F are free. F = "Frame Model". The tests `no_duplicate_bindings` and `no_entry_is_shadowed_by_an_earlier_one` enforce "most specific first".
- Side panel: `panels::show_side_panel` (`panels/mod.rs:303`): bottom panel `side_layers_area` (resizable, tab strip, hug-content rule, `layers_user_sized`), then bottom panel `side_color_area`, then the settings scroll area. `SideTab { Layers, Brush, Text }`, `shown_tab` (one tab rule), `SideRects`, `PanelHit` in `state.rs`.
- Self-test runs use `Prefs::default()` (`app.rs:144`) and never write prefs, so new prefs fields cannot disturb scenarios.
- `Document::sample(SampleKind)` (`doc_build.rs:376`) builds samples by writing an in-memory OBJ and importing it.
- Sample models exist at `G:\Claude_Resources\samples\khronos-gltf\` (ABeautifulGame.glb, Avocado, BoxTextured, DamagedHelmet, Lantern, WaterBottle).

Not verified (the builder must check; each is called out again where it matters): the Jump branch of `collect_grown` floods only over marked triangles; hover caches in `tools/stamp_place.rs` / `tools/brush.rs`; `view::uv_outline_in_3d`; exact text widths of the four upper tab buttons at 300 pt; Alt+H behaviour on Windows.

---

## 2. Core: object data model (work package 1)

### 2.1 Types (`crates/wte_core/src/mesh.rs`, ARCHITECTURE 5.7)

```rust
/// One object of the model (an OBJ object / group, a glTF mesh node, an FBX Model).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectInfo {
    /// Unique within the mesh, never empty.
    pub name: String,
    pub tri_count: u32,
    /// Bounds of the object's triangles (world space, transforms baked).
    pub bounds: Aabb,
}

/// Importer-side object data, parallel to `RawMesh::triangles`. `Default` = no object structure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawObjects {
    /// One entry per raw triangle: index into `names`. Empty = every triangle belongs to one unnamed object.
    pub tri_object: Vec<u32>,
    /// Names as the file gives them; may be empty strings or repeat.
    pub names: Vec<String>,
}

pub struct MeshData {
    // ... existing fields unchanged ...
    /// Object index per triangle (index into `objects`), same order as `tri_material`.
    pub tri_object: Vec<u32>,
    /// At least one entry for a non-empty mesh; order of first appearance in the raw triangle list.
    pub objects: Vec<ObjectInfo>,
}
impl MeshData {
    pub fn object_count(&self) -> u32;
    #[inline] pub fn tri_object_of(&self, t: TriId) -> u32;   // tri_object[t], 0 when the vector is short (hand-built test meshes)
}
```

`RawMesh` and `RawTriangle` are **not** changed. Patch the 4 `MeshData { .. }` literals (`tri_object: vec![0; n]`, one `ObjectInfo`).

Add to `WireEdge`: `pub object: u32` (2 literal sites: `mesh_build.rs:450`, `gpu/scene.rs:478`).

### 2.2 Normalize (`mesh_build.rs`, ARCHITECTURE 6.1 gets a step 9)

```rust
/// `normalize` with object data. `normalize(raw, opts)` == `normalize_with_objects(raw, &RawObjects::default(), opts)`.
pub fn normalize_with_objects(raw: RawMesh, objects: &RawObjects, opts: &NormalizeOptions) -> (MeshData, MeshReport);
```

Rules (step 9, "Objects"):

1. Each `Work` item carries the raw object index of its triangle (`objects.tri_object[i]`, or 0 when the vector is empty or shorter than the triangle list; an index `>= names.len()` counts as 0 with an empty name).
2. The object index is **not** part of the sort key and **not** part of the vertex key. Triangle order, vertex order and `uv_hash` are bit-identical to today for every input. (Test: the frozen `uv_hash` constants in the `mesh_build` tests stay green untouched.)
3. After the sort, objects are renumbered densely in order of first appearance **in the raw triangle order** (not the sorted order), counting only triangles that survived step 1. Objects with no surviving triangle disappear.
4. Names: an empty name becomes `"Object N"` (N = 1-based dense index). If the mesh ends up with exactly one object and its name is empty, the name is `opts.model_name`, or `"Model"` when that is empty too. Names are then made unique in index order with `_2`, `_3` (the rule sets already use in step 5).
5. `ObjectInfo::tri_count` and `bounds` are computed from the final triangles.
6. `wire_edges`: `HalfEdge` gets `object: u32` as its **last** field (so the derived ordering of the existing fields is unchanged); the emitted `WireEdge::object` is the object of the half-edge that is emitted. An edge shared by two objects therefore belongs to one of them (documented corner case: it disappears with that object).

No change to `MeshReport`.

### 2.3 Importers (ARCHITECTURE 5.21)

`RawImport` gets `pub objects: RawObjects`; `import/mod.rs:294` calls `normalize_with_objects(raw.mesh, &raw.objects, &normalize_opts)`.

- **OBJ** (`import/obj.rs`): `TriInfo` gets `o: u32` and `g: u32` (indices into two name lists kept in `Prepass`, `u32::MAX` = none yet). `o <name>` and `g <name>` lines set the current one (the whole rest of the line is the name; `g` with no name = `"default"`). The same name seen again continues the same entry (OBJ group semantics), so a name is one object however often it is re-entered. After the pre-pass: if the file has at least one `o` line with faces after it, objects are the `o` entries (faces before the first `o` form an unnamed object); otherwise, if it has `g` lines, objects are the `g` entries (faces before the first `g` form an unnamed object); otherwise `RawObjects::default()`. Update the doc comment at line 113 ("`o`, `g` are recorded as objects").
- **glTF / GLB** (`import/gltf.rs`): `Loader` gets `objects: RawObjects`. Each visited node with a mesh is one object: name = node name, else mesh name, else empty. `add_mesh` pushes the current object index once per pushed triangle. Two nodes instancing the same mesh are two objects (different transforms). A node whose primitives are all skipped yields no triangles and disappears in normalize.
- **FBX** (`import/fbx.rs`): one object per `Model` with geometry, name = `display_name(model)`; `Builder::add_geometry` pushes the object index per triangle.
- `testutil::imported_from_raw` and friends keep calling `normalize` (one object).

### 2.4 Samples (`samples.rs`, `doc_build.rs`, ARCHITECTURE 5.10 / 5.15)

- `pub fn object_trio() -> (RawMesh, RawObjects)`: three separate shells with UVs in disjoint regions of one texture: **"Floor"** (a plate under everything), **"Cube"** (standing on the floor, centre of the model), **"Ball"** (a `uv_sphere` beside the cube). One material, one texture set. The cube must sit between a top-down ray and the floor (the tests below rely on it).
- `pub fn to_obj_objects(raw, objects, mtl_name, texture_names) -> (String, String)`: `to_obj` plus an `o <name>` line whenever the object changes. `to_obj` itself stays byte-identical.
- `SampleKind::Objects` → `Document::sample` builds it through `to_obj_objects` and the OBJ importer (named "objects", texture "objects_checker.png"), so the sample also exercises the importer. Menu: File > Open Sample > "Three Objects".

### 2.5 Unit tests (wte_core)

`mesh_build` tests:
- `objects_follow_their_triangles_through_the_sort`: two materials x two objects interleaved; every final triangle's object matches its raw triangle (compare by position bits); `objects[i].tri_count` sums to `tri_count()`; bounds match.
- `object_data_never_changes_the_mesh`: for `two_material_model`, `seam_cube`, `soup_sets(…)`: `normalize` and `normalize_with_objects` with random object indices give equal `positions`, `indices`, `uvs`, `tri_material`, `uv_hash`.
- `unnamed_and_duplicate_names`: names `["", "Pawn", "Pawn", ""]` → `["Object 1", "Pawn", "Pawn_2", "Object 4"]`; a single unnamed object takes `model_name`; empty `model_name` → `"Model"`.
- `objects_without_triangles_disappear`: an object whose triangles are all degenerate is dropped and indices stay dense.
- `no_object_data_is_one_object`: `normalize` → `objects.len() == 1`, `tri_object` all 0.
- `wire_edges_name_their_object`.

`import/obj.rs` tests: `o` objects; `g` groups when there is no `o`; `o` wins over `g`; faces before the first `o`; a re-entered name continues its object; a file without `o`/`g` → one object named after the model (through `import_from_bundle`).
`import/gltf.rs` tests: three nodes (two named, one unnamed with a named mesh, two sharing one mesh) → objects and names; node transforms baked into `ObjectInfo::bounds`; duplicate node names get `_2`.
`import/fbx.rs` test: a two-`Model` ASCII document → two objects with the model names.
`doc_build` test: `Document::sample(SampleKind::Objects)` has objects `Floor`, `Cube`, `Ball`.

**Check after WP1:** `cargo-gnu.cmd test -p wte_core` green (no existing test edited except the 4 `MeshData` literals and the 3 `RawImport` literals); clippy 0 warnings; the frozen `uv_hash` tests untouched.

---

## 3. Core: hidden objects (work package 2)

### 3.1 `TriMask` and the BVH (`bvh.rs`, ARCHITECTURE 5.8 / 6.2)

```rust
/// A set of triangles as a bit set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriMask { /* bits: Vec<u64>, len: u32, count: u32 */ }
impl TriMask {
    pub fn from_fn(tri_count: u32, f: impl Fn(TriId) -> bool) -> TriMask;
    #[inline] pub fn get(&self, t: TriId) -> bool;      // false beyond len
    pub fn count(&self) -> u32;
    pub fn is_empty(&self) -> bool;
}

pub struct Bvh { /* tree: Arc<BvhTree { nodes, tri_order }>, hidden: Option<Arc<TriMask>> */ }
impl Bvh {
    /// The same tree with another hidden set (`None` or an empty mask = nothing hidden). O(1): the tree is shared.
    pub fn with_hidden(&self, hidden: Option<Arc<TriMask>>) -> Bvh;
    pub fn hidden(&self) -> Option<&TriMask>;
    #[inline] pub fn is_hidden(&self, t: TriId) -> bool;
}
```

- `intersect` and `walk_sphere`: hoist `let hidden = self.hidden.as_deref();` out of the loops; in the leaf loop `if hidden.is_some_and(|m| m.get(tri)) { continue; }` before the triangle test. With `None` the results are bit-identical to today (the gate checks this with the existing test suite and the pixel-identity sweep). Node boxes are not rebuilt; rays simply find nothing in hidden leaves.
- `Bvh::build` returns `hidden: None`. `with_hidden(Some(empty))` stores `None`.
- `Bvh` stays `Send + Sync`.

Why the mask lives in the BVH: about 50 production call sites query the BVH, several of them on worker threads with a cloned `Arc<Bvh>`. A mask parameter would touch every one of them and their tests; a filtered BVH rebuild costs a full build of 1.5 M triangles per toggle. A shared tree plus a bit set costs one predictable branch per leaf triangle and makes "what a job sees" a snapshot by construction.

### 3.2 `Document` API (`document.rs`, ARCHITECTURE 5.14)

```rust
impl Document {
    /// One flag per object (`mesh.objects`), all false in a new or loaded document.
    pub fn hidden_objects(&self) -> &[bool];
    pub fn object_hidden(&self, object: u32) -> bool;
    pub fn hidden_object_count(&self) -> usize;
    /// 0 in a new document; +1 at every call of `set_hidden_objects` that changed something.
    pub fn visibility_generation(&self) -> u64;
    /// Replaces the hidden flags (a slice of another length is padded with false / cut). `Ok(false)` = nothing changed.
    /// `Err(EditError)` from `guard_no_stroke` while a stroke is open. View state: no history entry, no revision change,
    /// `is_modified` unaffected. Swaps `self.bvh` for `bvh.with_hidden(..)` and drops every cached wrap parameterization.
    pub fn set_hidden_objects(&mut self, hidden: &[bool]) -> Result<bool, EditError>;
    /// Union of the bounds of the visible objects; `mesh.bounds` when nothing is hidden or everything is.
    pub fn visible_bounds(&self) -> Aabb;
    /// Union of the bounds of the listed objects that are visible; `None` when there is none.
    pub fn objects_bounds(&self, objects: &[u32]) -> Option<Aabb>;
}
```

- New private fields `hidden_objects: Vec<bool>`, `visibility_generation: u64`; initialised in `doc_build` (`from_parts` / `from_import`), **not** in `DocumentSnapshot`, `StructureSnapshot` or the manifest.
- `set_hidden_objects` builds the mask with `TriMask::from_fn(tri_count, |t| hidden[mesh.tri_object_of(t)])`.
- Wrap cache: add `WrapCache::clear()` and call it (the cache key `WrapParam::serves` does not know about visibility; `ensure_wrap_param` rebuilds the selected stamp's parameterization on the next frame, as after open / undo).
- The pending edit is left alone (a slider drag keeps running; its later bake uses the new mask).

### 3.3 The one non-BVH path: wrap growth (`wrap_param.rs`, ARCHITECTURE 6.5a)

In `collect_grown`, the closure `in_sphere` returns `None` for `bvh.is_hidden(t)` (so hidden triangles are marked 2 and never entered), and the anchor loop skips hidden triangles (they cannot be in `near` anyway, since `near` comes from the masked query). **Builder: read the Jump branch (from line ~2803)** and confirm its part flood only visits triangles with `mark == 1` (which come from the masked query); if it can step to an unmarked neighbour through `g.vtris`, add the same `is_hidden` test there. The test-only `Source::Sphere` path uses masked queries only. Nothing else in `wrap_param` / `wrap_relax` / `wrap_bake` enumerates triangles outside the collected patch — confirm with a grep for `g.vtris(` and `0..mesh.tri_count()` in those files.

### 3.4 Rules for hidden objects (the contract; goes into ARCHITECTURE 5.14, 6.4, 6.5, 6.5a)

| Operation | Behaviour with hidden objects |
|---|---|
| Pick / hover / eyedropper / fill seed / stamp and text placement / gizmo drag / text-path points in the 3D view | the ray passes through hidden objects and hits what is behind them |
| Brush and eraser in 3D | hidden triangles are never dab candidates: they get no paint and do not shadow "Same side" tests. (Objects that share texels — instanced meshes, mirrored UVs — still share paint; that is a property of the UVs.) |
| Projected stamp / text bake | hidden triangles receive nothing and do not occlude; `fit_stamp_depth` fits to visible surfaces |
| Wrap | hidden triangles are never part of the patch, never the anchor, never a Jump target |
| When a stamp is (re)baked | against the visibility at the moment the bake job is made. Changing visibility never re-bakes existing stamps. Undo / redo restore stored pixels. `project::load` keeps saved caches; the rare load-time re-bake of a damaged cache runs with everything visible (the app restores visibility after `load` returns) |
| Fill (Layer / Island scope), 2D (UV) brush, UV stamps | texture-space operations: **not masked**. The seed comes from a visible surface; "Layer" fills the whole layer as its name says |
| Export, composite, save, thumbnails | unaffected |
| Undo / redo / revision / modified flag | unaffected by hide / show / select |

### 3.5 Unit tests (wte_core)

`bvh` tests:
- `a_hidden_triangle_is_skipped_by_every_query`: `occluded_quads` or `cube_on_floor`: with the front part hidden `raycast` returns the part behind, `occluded` / `occluder` ignore it, `query_sphere` / `query_sphere_dist` omit it; everything hidden → `None` / empty.
- `no_mask_and_an_empty_mask_answer_identically`: random rays and spheres on `soup_sets`, compare `Hit`s field by field with `with_hidden(None)`, `with_hidden(Some(empty))` and the plain tree.
- `with_hidden_shares_the_tree` (`node_count` equal; pointer equality of the tree `Arc` through a `#[cfg(test)]` accessor).

`document` / integration tests (new file `crates/wte_core/tests/objects.rs`, using `Document::sample(SampleKind::Objects)` or `object_trio`):
- `hiding_is_view_state`: `set_hidden_objects` changes neither `revision()`, `is_modified()`, `undo_label()` nor any layer pixel; `visibility_generation` goes up by one; a second identical call returns `Ok(false)`; refused during an open stroke.
- `rays_pass_through_a_hidden_object`: top-down ray hits Cube; Cube hidden → hits Floor; Show all → Cube again.
- `a_brush_stroke_paints_under_a_hidden_object_and_not_on_it`: stroke over the cube's footprint with the cube hidden: the Floor texels under the cube change, the pixels of the Cube's UV region are byte-identical; the same stroke with everything visible paints the cube and leaves the floor under it clean.
- `a_projected_stamp_skips_a_hidden_object`: same shape of assertion for `add_stamp` (Project, occlusion on).
- `a_wrap_never_enters_a_hidden_object`: wrapped stamp anchored on the Floor next to the Cube: with the Cube hidden no baked texel lies in the Cube's UV region and the parameterization covers no Cube triangle (`stamp_wrap_local(tri of Cube, ..) == None`); also with Jump small gaps on.
- `hiding_drops_cached_wrap_params` (through `wrap_cache_probe`).
- `a_bake_job_keeps_the_visibility_it_was_made_with`: `make_bake_job`, then change visibility, run the job: output equals the bake with the old visibility.
- Determinism: one existing brush / wrap determinism test re-run with a mask for `RAYON_NUM_THREADS` 1 and N (results equal).

**Check after WP2:** `cargo-gnu.cmd test --workspace` green; `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1` not slower than on `main` beyond noise (the mask branch is the only hot-path change); clippy 0.

---

## 4. App: state, commands, menu, shortcuts, persistence (work package 3a)

### 4.1 State (`state.rs`, ARCHITECTURE 8.2)

```rust
pub struct Session {
    // ...
    /// Selected objects (indices into `doc.mesh.objects`). App state: not in the document, not in history, not saved.
    pub selected_objects: std::collections::BTreeSet<u32>,
    /// The row a Shift-click extends from (the last plain / Ctrl-clicked object).
    pub object_anchor: Option<u32>,
    /// +1 at every change of `selected_objects` (the GPU highlight buffer follows it).
    pub object_selection_rev: u64,
    /// Request: the Objects list scrolls this row into view once (set by a pick in the 3D view).
    pub object_reveal: Option<u32>,
}
```
`Session::validate` drops selected indices `>= object_count` and clears a stale anchor.

### 4.2 Actions (`action.rs`)

```rust
// ---- objects ----
/// Plain click: this object only.
SelectObject(u32),
/// Ctrl-click (list) / Ctrl- or Shift-click (3D view): add or take out.
ToggleObjectSelected(u32),
/// Shift-click in the list: the range from `object_anchor` to this row.
SelectObjectRange(u32),
SelectAllObjects,
DeselectObjects,
/// The eye of one row.
SetObjectVisible(u32, bool),
HideSelectedObjects,
/// Hide every object that is not selected (and show the selected ones).
IsolateSelectedObjects,
ShowAllObjects,
/// Frame the selected visible objects in the 3D view (the visible model when nothing visible is selected).
FrameSelection,
```

### 4.3 Dispatch (`dispatch.rs` + new file `crates/wte_app/src/object_ops.rs`, ARCHITECTURE 8.3)

`object_ops.rs` holds the logic as plain functions on `AppState` / `Session` so it is unit-testable without a GPU:

```rust
pub(crate) fn select(session: &mut Session, object: u32);
pub(crate) fn toggle(session: &mut Session, object: u32);
pub(crate) fn select_range(session: &mut Session, to: u32);
pub(crate) fn select_all(session: &mut Session);
pub(crate) fn deselect(session: &mut Session);
/// Applies new hidden flags through `Document::set_hidden_objects`; reports an error as a toast.
pub(crate) fn set_hidden(state: &mut AppState, hidden: Vec<bool>);
pub(crate) fn set_visible(state: &mut AppState, object: u32, visible: bool);
pub(crate) fn hide_selected(state: &mut AppState);
pub(crate) fn isolate_selected(state: &mut AppState);
pub(crate) fn show_all(state: &mut AppState);
pub(crate) fn frame_selection(state: &mut AppState);
/// Hidden object names for `UiState`, and back (unknown names are ignored).
pub(crate) fn hidden_names(doc: &Document) -> Vec<String>;
pub(crate) fn hidden_from_names(doc: &Document, names: &[String]) -> Vec<bool>;
```

Rules:
- Selection actions: no interaction guard, no document change.
- `SetObjectVisible`, `HideSelectedObjects`, `IsolateSelectedObjects`, `ShowAllObjects` are added to `needs_interaction_guard` (a held stroke or gizmo drag is ended the usual way first; a finishing brush stroke is completed). They are no-ops (no toast) without a session; Hide / Isolate are no-ops with an empty selection.
- Hiding keeps the selection (the hidden rows stay selected in the list so the eye or Show All brings them back in place); hidden objects are never highlighted or framed.
- After a visibility change: `app.tools` hover caches must not show stale results. **Builder: check `tools/stamp_place.rs` (the see-through text picture) and `tools/brush.rs` (cursor / candidates caches)**; where a cache is keyed by pointer position or `doc.revision()` only, add `doc.visibility_generation()` to the key.
- `FrameModel` (existing, F): in the 3D branches use `session.doc.visible_bounds()` instead of `mesh.bounds` (identical when nothing is hidden). `ResetCamera` likewise.
- `FrameSelection`: `doc.objects_bounds(selected)` else `visible_bounds()`; frames `session.camera` in 3D / Split and `session.preview_camera` in Preview; nothing in 2D. `OrbitCamera::frame` is used as is (verified: zoom floor `0.02 R` and `near = 1e-3 R` leave room for an object 2 % of the model size).
- `Cancel` (Esc) is unchanged; it does not clear the object selection.
- Toast on visibility changes: none. On opening a project that restores hidden objects: one Info toast "N objects are hidden (Object > Show All)".

### 4.4 Shortcuts (`shortcuts.rs`, ARCHITECTURE 8.10)

Add five entries (table size 36 → 41), each more specific one before its base key:

| Label | Keys | Action (`with_doc`) | Position in the table |
|---|---|---|---|
| "Frame Selection" | Shift+F | `FrameSelection` | directly before "Frame Model" |
| "Isolate Selected" | Shift+H | `IsolateSelectedObjects` | new "objects" block |
| "Show All" | Alt+H | `ShowAllObjects` | after Shift+H |
| "Hide Selected" | H | `HideSelectedObjects` | after Alt+H |

(That is four; no fifth binding: Select All has no shortcut — Ctrl+A is left free for a later round. Table size 40.) `text_ok` false for all. Extend `the_contract_bindings_exist`; the two table-order tests must pass unchanged. The Shortcuts dialog lists the table, so it picks the entries up; check its layout still fits.

### 4.5 Menu (`panels/menu_bar.rs`, ARCHITECTURE 8.7 "Menu bar")

New menu **Object** between Layer and Help:

- Hide Selected (H) — enabled with a non-empty selection
- Isolate Selected (Shift+H) — enabled with a non-empty selection
- Show All (Alt+H) — enabled when `hidden_object_count() > 0`
- —
- Frame Selection (Shift+F) — enabled with a document
- —
- Select All / Deselect All — enabled with a document / a non-empty selection
- —
- Objects Panel — sets `state.upper_tab = UpperTab::Objects` (a setting, like the Background entries; added in WP4, a disabled placeholder is not needed before)

File > Open Sample gains "Three Objects".

### 4.6 Persistence (`io_flows.rs`, ARCHITECTURE 7.2 note, 8.9)

- `UiState` gets `hidden_objects: Vec<String>` (`#[serde(default)]` via the struct attribute). `capture` writes `object_ops::hidden_names(&session.doc)`.
- `install_document`: after building the session and before storing it, `doc.set_hidden_objects(&hidden_from_names(..))`; queue the Info toast when at least one object is hidden.
- `FORMAT_VERSION` stays 3, `required_reader` unchanged, no migration. Older builds ignore the unknown `ui_state` key (verified: `UiState` has no `deny_unknown_fields`); older projects load with nothing hidden. A project saved here and opened in an older build shows everything.
- Selection is not saved.

### 4.7 Tests (wte_app, no GPU)

`object_ops` unit tests: select / toggle / range (anchor rules: plain and Ctrl set the anchor, Shift keeps it; range with no anchor = plain select); hide keeps the selection; isolate shows the selected and hides the rest; show all; `frame_selection` targets the union bounds and ignores hidden objects; `hidden_names` ↔ `hidden_from_names` round trip, unknown and duplicate names tolerated; none of them changes `doc.revision()` or `is_modified()`.
`io_flows`: extend `ui_state_round_trips_and_tolerates_garbage` (new field; a `ui_state` without it; a non-array value → ignored with the warning path).
`shortcuts`: table tests; H / Shift+H / Alt+H / Shift+F produce their actions; H in a focused text field types "h".
`control_sweep/panel_tests.rs`: `every_menu_entry_queues_its_action` and `menu_entries_are_enabled_exactly_when_they_can_act` get the Object menu and the new sample entry.
`dispatch` test (headless app harness used by `app_frame`): `HideSelectedObjects` during a held stroke ends the stroke first and leaves one undo entry (the stroke), none for the hide.

**Check after WP3a:** workspace tests and clippy green; the whole feature is drivable by actions and keys even though it is not drawn yet (ray tests prove hiding).

---

## 5. App: rendering, highlight, picking (work package 3b)

### 5.1 `GpuScene` (`gpu/scene.rs`, ARCHITECTURE 8.4)

New pure helpers (unit-tested without a GPU):

```rust
/// Indices of the visible triangles, compacted per draw, and the draws over that compacted list (empty draws dropped).
pub(crate) fn visible_draws(mesh: &MeshData, draws: &[Draw], hidden: &[bool]) -> (Vec<u32>, Vec<Draw>);
/// Indices of the triangles of the selected, visible objects (mesh order).
pub(crate) fn selected_indices(mesh: &MeshData, hidden: &[bool], selected: &BTreeSet<u32>) -> Vec<u32>;
/// Instance runs per object of a list that is sorted by object: `(object, start..end)`, empty runs omitted.
pub(crate) fn object_runs(objects: &[u32]) -> Vec<(u32, Range<u32>)>;
/// The runs of visible objects, neighbours merged (one range when nothing is hidden).
pub(crate) fn visible_runs(runs: &[(u32, Range<u32>)], hidden: &[bool]) -> Vec<Range<u32>>;
```

Changes:
- **Wire lists grouped per object.** `wire_instances_3d` and `wire_instances_uv_all` additionally return, per list, the object of every instance; each group they build today (3D: interior, then seams + boundaries; UV per set: interior, then seams) is stable-sorted by `WireEdge::object` before it is concatenated, and `object_runs` is taken per group. With one object the instance bytes are identical to today (assert this in the existing byte-comparison test). `SceneGeometry` and `GpuScene` keep the run lists: `runs_all`, `runs_seams`, `runs_uv[set]` (two groups each for `all` and `uv`: keep them as one `Vec` of runs in buffer order).
- **Visibility.** New fields `indices_visible: Option<wgpu::Buffer>` (`None` = nothing hidden: the full buffer and `draws` are used), `draws_visible: Vec<Draw>`, `wire_draw_all / wire_draw_seams: Vec<Range<u32>>`, `wire_draw_uv: Vec<Vec<Range<u32>>>`, `highlight: Option<(wgpu::Buffer, u32)>`, `objects_key: (u64, u64)`.

```rust
/// Rebuilds the visible index buffer / wire ranges when `visibility_generation` changed and the highlight index buffer
/// when it or `selection_rev` changed. Cheap no-op otherwise. Buffers larger than `max_buffer_size` cannot occur
/// (they are subsets of the full index buffer).
pub fn sync_objects(&mut self, gpu: &Gpu, doc: &Document, selected: &BTreeSet<u32>, selection_rev: u64);
/// Vertex buffer, index buffer and draws for a render: the visible set, or everything with `all_objects`.
pub(crate) fn mesh_draw(&self, all_objects: bool) -> Option<(&wgpu::Buffer, &wgpu::Buffer, &[Draw])>;
pub(crate) fn highlight(&self) -> Option<(&wgpu::Buffer, u32)>;
/// `wire_all` / `wire_seams` / `wire_uv` now return the instance ranges to draw as well.
pub(crate) fn wire_all(&self, all_objects: bool) -> Option<(&wgpu::Buffer, &[Range<u32>])>;   // same shape for seams and uv
```
`view::sync_scene` calls `sync_objects` every frame after `sync` (cost when nothing changed: two integer compares). Cost when it changes on ABeautifulGame (1.5 M triangles): one pass over the triangles and one 18 MB buffer upload — measure and log it (`log::info!`, like "GPU scene built in").

### 5.2 Renderer (`gpu/renderer.rs`, `gpu/params.rs`, new `gpu/shaders/tint.wgsl`)

- `View3dParams` gets `all_objects: bool` (draw hidden objects too) and `highlight: bool`. Callers: Main3D → `all_objects: false`, `highlight` per 5.3; Preview mode → `false, false`; the export dialog's preview → `true, false` (an export contains every object).
- `render_3d`: mesh from `scene.mesh_draw(p.all_objects)`; then, if `p.highlight` and `scene.highlight()` is `Some`, the tint pass; then the wires as ranges (`pass.draw(0..4, r.clone())` per range).
- `render_uv`: UV wires as ranges from `scene.wire_uv(set, false)`.
- **Tint pipeline** `tint_pipeline = view_pipeline(.., PipelineSpec { label: "tint", module: tint, layout: &view_only, buffers: &mesh_buffers, topology: TriangleList, depth: DepthMode::Overlay, blend: Some(LINE_BLEND) })`. `tint.wgsl`: the `ViewUniforms` struct as in the other shaders; `vs_main` = `view.view_proj * vec4(pos, 1)` with the same `DEPTH_PULL` as `lines.wgsl` (so it wins the `LessEqual` test against the mesh's own depth); `fs_main` returns a constant accent colour `vec4(0.25, 0.61, 1.0, 0.30)` written the way `lines.wgsl` writes its instance colour (follow that shader's colour-space convention). Add the shader to `every_shader_is_valid_wgsl` and the vertex layout test.

### 5.3 When the highlight is drawn (`view/view3d.rs`)

`highlight = selection non-empty && no stroke open && (shown upper tab is Objects || state.tool == ToolKind::Navigate)`.
Reason: a tint over the surface would falsify colours while painting; with the Brush active and any other tab shown the model is clean. (Until WP4 exists the condition is `tool == Navigate`.) A silhouette outline instead of a tint is a possible later refinement, not part of this plan.

### 5.4 Selecting in the 3D view (`tools/mod.rs`, ARCHITECTURE 8.6)

Rule: **a primary click without a drag, with the Navigate tool, in the Main3D view, on a model with at least two objects.** Painting tools keep every click; Navigate had no click meaning. Single-object models are excluded so existing scenarios that click with Navigate change nothing.

Implementation in `ToolController`:
- `press`: when the Navigate branch starts an `Orbit` with `button == Primary` (the logical button, so Space+LMB pan is excluded) and `xf.view == ViewId::Main3D`, remember `nav_click = Some(NavClick { origin: p.pos, mods: p.mods, camera: session.camera, travel: 0.0 })`.
- `drag`: add the pointer delta length to `travel`.
- `release`: if `nav_click` is set and `travel < 4.0` points: restore `session.camera` (undo the sub-4-pt orbit), cast `xf.surface_at` at `origin`; hit → object `mesh.tri_object_of(hit.tri)`: plain = `SelectObject`, Ctrl or Shift = `ToggleObjectSelected`, and set `session.object_reveal`; miss with no modifier = `DeselectObjects`. Actions go through `state.push`.
- `go_idle` clears `nav_click`.
- Status hint for Navigate (where the tool hints are set): append "Click an object to select it" when the model has two or more objects.

### 5.5 Other view details

- `view/view2d.rs`: the texture is shown whole (it is one image shared by all objects); **UV wires of hidden objects are not drawn**; painting in 2D is not masked (3.4). No highlight in 2D.
- `view::uv_outline_in_3d` / `locate_px` (outline of a selected UV stamp traced onto the model): **builder: check** whether it can place outline points on hidden triangles; if so skip points whose owning triangle is hidden (`doc.bvh.is_hidden(tri)`).
- Everything hidden: the view shows the backdrop; tools hit nothing; allowed.

### 5.6 Tests

`gpu/scene.rs` unit tests (no GPU): `visible_draws` (every visible triangle exactly once, groups preserved, hidden ones absent, nothing hidden → equals the full index list and `build_draws`); `selected_indices`; `object_runs` / `visible_runs` (merging, all hidden → empty); wire lists of a multi-object mesh are grouped per object and with one object are byte-identical to the old lists.
`tools/mod.rs` unit tests: `NavClick` travel rule (3 pt = click and camera restored, 5 pt = orbit), modifiers map to the three actions, no pick on a single-object model, none in Preview / 2D.
GPU test (`control_sweep/gpu_tests.rs`, `#[ignore = "needs a GPU"]`): render `SampleKind::Objects` with the Cube hidden → the pixel at the cube's centre equals the floor render; with the Cube selected and `highlight: true` that pixel is bluer than without; wireframe on + Cube hidden → no dark wire pixels inside the cube's former silhouette above the floor grid lines (compare against a render of a floor-only reference); `all_objects: true` draws the cube again.

**Check after WP3b:** workspace tests + clippy; `cargo test -p wte_app -- --ignored --skip perf_` green on the GPU machine; headless run `--scenario default` and `paint3d` still `"ok": true` and **pixel-identical to the `main` exe** (nothing in the side panel has changed yet — this is the last point where a full-frame identity check is possible, use it).

---

## 6. The two tab groups (work package 4)

### 6.1 Types and state (`state.rs`)

```rust
/// The tab of the upper group of the side panel. Starts on Tool.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpperTab { #[default] Tool, Color, TextureSets, Objects }
impl UpperTab {
    pub const ALL: [UpperTab; 4];
    pub fn index(self) -> usize;
    pub fn label(self) -> &'static str;     // "Tool", "Color", "Texture sets", "Objects"
}
```
`AppState::upper_tab: UpperTab`. `SideTab`, `state.side_tab`, `shown_tab`, `brush_tab_enabled`, `text_tab_enabled` keep their names and rules (81 scenario references stay valid).

`SideRects` — existing fields keep their names with these meanings, plus new ones:

| Field | Meaning now |
|---|---|
| `panel` | unchanged |
| `upper_area` (new) | the whole upper group (strip + page) |
| `upper_tab_buttons: [Rect; 4]` (new) | indexed by `UpperTab::index` |
| `upper_page` (new) | the visible page rect below the strip |
| `tool_area` | = `upper_page` in frames where the Tool page is shown, else `Rect::NOTHING` |
| `tool_options` | the Tool page content as laid out, in the same frames |
| `color` | = `upper_page` in frames where the Color page is shown, else `Rect::NOTHING` |
| `sets_tab`, `objects_tab: Option<Rect>` (new) | `Some(visible rect)` in frames where that page is shown |
| `bottom_area`, `tab_buttons`, `brush_tab`, `brush_tab_content`, `text_tab` | unchanged |

`PanelHit`: `SettingsArea` is recorded only while the Tool page is shown; `ColorSection` / `ColorContent` only while the Color page is shown. New: `UpperTabButton(UpperTab)`, and for WP5 `ObjectRow(u32)`, `ObjectEye(u32)`, `ObjectsList`, `ObjectsFooter`, `ObjectsHide`, `ObjectsIsolate`, `ObjectsShowAll`, `ObjectsFrame`.

### 6.2 Layout (`panels/mod.rs` + new `panels/side_groups.rs`, ARCHITECTURE 8.7 "Side panel" rewritten)

Move the sizing constants / functions and the new upper-group code into `panels/side_groups.rs` (`panels/mod.rs` is already over the 700-line target).

```
+--------------------------------------+
| Tool | Color | Texture sets | Objects |  upper strip, 24 pt
|--------------------------------------|
| page of the shown upper tab          |  own vertical scroll area per page (thin visible bars)
|                                      |
|======================================|  draggable edge (the existing resize edge of the lower area)
| Layers | Brush | Text                 |  lower strip, 24 pt (one tab rule unchanged)
| page of the shown lower tab          |
+--------------------------------------+
```

- **Lower group** = today's `egui::Panel::bottom("side_layers_area")`, code and rules unchanged except its automatic height for the Brush / Text tab (below). It still hugs its rows with the Layers tab until the user drags the edge (`layers_user_sized`), min `LAYERS_MIN_HEIGHT.min(height * 0.5)`.
- **The pinned Color panel (`side_color_area`) is removed.**
- **Upper group** = the remaining `ui`: strip (`upper_tab_strip`, same widget style as `tab_strip`: `Button::selectable`, all four always enabled, a click writes `state.upper_tab`), then the page:
  - Tool: `tool_options::show` in `ScrollArea` `side_settings_scroll` (same id as today).
  - Color: `color::show(ui, state, ps, square)` in `ScrollArea` `side_color_scroll`; `square = color_square_side(width, page_height - COLOR_AREA_MARGIN, ps.color_fixed_height)` (72..=140 pt as today, then the page scrolls).
  - Texture sets: `texture_sets::show` in `ScrollArea` `side_sets_scroll`.
  - Objects: `objects::show` (WP5; until then the label "No objects panel yet" is not acceptable — WP4 and WP5 may land together, or WP4 ships the page with the read-only header line of 7.1).
- **Section headers**: the collapsing headers "Color" and "Texture sets" are removed (the tab is the header); `color::show` then always returns `Some(square)`. The Tool page keeps one heading line "<Tool> options" as a plain strong label (not collapsible), so the page says which tool it shows.
- **Automatic height of the lower group** (not user-sized):
  - Layers tab: `layers_area_fit` as today (hug, at most 40 % of the panel, 420 pt).
  - Brush / Text tab: `tab_area_height(height, upper_keep)` with `upper_keep = TAB_STRIP_HEIGHT + 4 + max(BRUSH_OPTIONS_HEIGHT, ps.color_content_height + COLOR_AREA_MARGIN)`: `(height - upper_keep).clamp(TAB_AREA_MIN.min(height * 0.5), TAB_AREA_MAX).min(layers_area_max(height))`. `ps.color_content_height` is the last measured Color page height (first-frame guess as today minus the removed header). Re-measure `BRUSH_OPTIONS_HEIGHT` for the header-less Tool page.
- **Limits**: `layers_area_max(height) = (height * 0.85).min(height - UPPER_MIN_HEIGHT).max(LAYERS_MIN_HEIGHT)` with `UPPER_MIN_HEIGHT = TAB_STRIP_HEIGHT + SETTINGS_MIN_HEIGHT` (24 + 112). `COLOR_MIN_HEIGHT` goes away.
- **Draggable split**: the lower area's top edge, as today. A press on it sets `layers_user_sized`; from then on the height applies to every lower tab and is clamped to `min..=layers_area_max(height)`. New: a **double-click on the edge returns to automatic** (`layers_user_sized = false`).
- `state.panel_rects.clear()` at the start and the `SideRects` record at the end stay.

### 6.3 Guarantees (replace the old "three areas" guarantees; tests in 9)

At a window of 1280x800 or larger (side panel at least 740 pt high), split not user-sized:

- **G1 (Color tab)**: with any tool, any selection and any lower tab, the whole Color page is on screen without scrolling (`ColorContent` inside `ColorSection`) and the square is 140 pt.
- **G2 (Tool tab)**: with the Brush or Eraser, the five slider rows (Size … Spacing) are inside `tool_area` without scrolling, with the Layers tab **and** with the Brush tab below (better than today, where only the Layers case was guaranteed).
- **G3 (lower group)**: unchanged — Layers footer and Delete below the list, inside the panel; the Brush tab's rows inside the lower area (A12); side panel at most 300 pt wide.
- **G4**: the four upper tab buttons fit in one row at 300 pt; the two groups never overlap; no strip button lies outside its group.

At 900x600 and 640x480: the groups stay inside the panel and do not overlap; the upper page keeps at least about 96 pt; pages scroll; the Color square shrinks to 72 pt before the page scrolls.

### 6.4 Tab behaviour

- **Lower group: unchanged.** The one tab rule (`shown_tab`): Brush only with Brush / Eraser, Text only with a text target, else Layers; a tool change never writes `side_tab`.
- **Upper group**: all four tabs always available; **a tool change does not write `upper_tab`.** Reason: with tabs, colour changes are the most frequent trip; if `SetTool` jumped to the Tool tab, every B / E key press would throw the user out of the Color tab. Two targeted exceptions:
  1. `state.focus_text_content` (the Text tool asks for keyboard focus in its content field) sets `upper_tab = Tool` at the start of `show_side_panel`, otherwise typing would go nowhere.
  2. A click on the toolbar's **primary colour chip** sets `upper_tab = Color` (`toolbar.rs`: the chip becomes `Sense::click()`, hover text "Primary colour: click to show the Color tab").
  - `Object > Objects Panel` sets `upper_tab = Objects`.
- Without a document every page shows its existing "No document open" line.

### 6.5 Persistence (`state.rs` `Prefs`, `prefs.rs`, ARCHITECTURE 8.9)

New prefs fields, all `#[serde(default)]` through the struct attribute and read leniently (an unknown tab name falls back to the default; follow `lenient_prefs_serde`):
`side_upper_tab: UpperTab`, `side_lower_tab: String` (`"layers" | "brush" | "text"`; `SideTab` itself stays non-serde), `side_split: Option<f32>` (the user-sized lower height in points; `None` = automatic).
- Start: `AppState::new` copies them into `upper_tab`, `side_tab`, and `PanelState` starts with `layers_user_sized = side_split.is_some()` using `default_size(side_split)`.
- Exit: next to `brush_flows::sync_prefs` in `WteApp::on_exit` (a small `panels::sync_prefs(state, ps)`), never in self-test mode (already the rule). `from_json` clamps `side_split` to a finite value in `100..=2000`.
- The `SideTab` doc comment "Not saved" is updated.

### 6.6 What breaks and how it is updated (keep each test's intent)

`control_sweep/panel_tests.rs`:

| Test | Change |
|---|---|
| `the_picker_and_the_layers_footer_stay_on_screen_with_long_tool_options` | becomes two loops over the same tools x window sizes: with `upper_tab = Color` assert G1 (and inside-panel / no-overlap at small sizes); with `upper_tab = Tool` assert `settings.height() >= 90`, footer / Delete checks, list height; "areas do not overlap" is now `upper_area.bottom() <= bottom_area.top() + 0.5` |
| `a_short_layer_list_gives_its_room_to_the_tool_options` | same intent; `panel` height = `bottom_area.bottom() - upper_area.top()`; add the double-click-returns-to-automatic step |
| `a12_brush_tab_heights_at_1280x800`, `a11_brush_tab_panels_are_idle_and_keep_their_width` | A12 budget re-stated with G2 (slider rows inside `tool_area` with the Brush tab shown); new expected lower height from the new `tab_area_height` |
| `colour_section_edits_the_primary_colour_and_the_swatches` | set `upper_tab = Color` first |
| `collapsing_headers_and_the_colour_picker_value_fields` | rename to `…the_colour_picker_value_fields`; drop the header clicks; add "the Tool page has no collapsible header" |
| `texture_set_rows_activate_their_set` | set `upper_tab = TextureSets` first |
| `idle_panels_with_every_kind_of_selection_queue_nothing` | run for each of the four upper tabs |
| `toolbar_buttons_select_tools_and_colour_chips_swap_and_reset` | add: primary chip click shows the Color tab |
| tool-option tests (`brush_and_eraser_options…`, `fill_and_stamp_options`, `text_tool_options…`, `selected_*_stamp_*`, mode / wrap option tests) | unchanged (default tab is Tool); fix only if they clicked a section header |
| new `upper_tabs_show_one_page_each` | click each strip button through AccessKit; exactly one of `tool_area` / `color` / `sets_tab` / `objects_tab` is drawn; G4 |
| new `a_tool_change_keeps_the_upper_tab_and_text_focus_shows_the_tool_tab` | 6.4 |
| new `the_split_is_dragged_kept_and_reset` | drag, resize window (clamp), double-click |

`control_sweep/text_panel_tests.rs` (26 references): tests that check the Text block's rects against `SettingsArea` / `side_rects.tool_area` keep working with the default Tool tab; those that assert "the Color section is inside the panel" switch to `upper_area`.

`panels/text_block.rs`, `panels/tool_options.rs`, `brush_flows.rs` read `side_rects` / `SettingsArea` in a few places (11 references): they run only while the Tool page is drawn; guard any read against `Rect::NOTHING`.

`selftest/scenarios.rs`:

| Helper / place | Change |
|---|---|
| `assert_side_panel_layout` (3525) | groups inside the side panel and apart (`upper_area`, `bottom_area`); strip buttons inside `upper_area`; footer / Delete / list checks unchanged; `if Color page shown && side.height() >= 740` → content not cut (G1); `if Tool page shown` → `SettingsArea` at least 90 pt |
| `assert_side_areas_apart` (6450) | "tool options end above the Color section, Color above the lower area" becomes "upper group ends above the lower group; no lower strip button above `upper_area.bottom()`" |
| `brush_tab_layout` (5059), `text_page_layout` (7340) | "Color section inside the side panel" → "upper group inside the side panel"; the A12 budget uses G2 |
| unit test with hand-built `SideRects` (9260-9285) | fill the new fields; expectations follow the new helper |
| `wrap_surface` steps "collapse the Color section" (8710, 8769) | delete both steps (the Transform options have the whole Tool page now) |
| line 671 (`StampTextField` inside `SettingsArea`) | unchanged |
| scenarios that assert tab rules (`brush_tab`, `text_tab`, round-4 3.3 block) | unchanged (lower group rules are the same) |

**Pixel identity:** from WP4 on, every screenshot differs from `main` in the side panel. The gate's identity check is restricted to (a) the view rect(s) from `<out>.json` / `state.view_rects`, toast area excepted, for scenarios whose view size did not change (the side panel stays 300 pt wide, so the central area is unchanged), and (b) exports byte-identical (`tools/round3/check_exports.py` must PASS).

**Check after WP4:** workspace tests + clippy; run every existing scenario at 1280x800 (and `brush_tab`, `text_tab`, `layers` also at 900x600 and 640x480): all `"ok": true`; view-rect pixel identity against the WP3b exe; look at one screenshot per upper tab.

---

## 7. The Objects tab (work package 5)

New file `crates/wte_app/src/panels/objects.rs` (`pub(super) fn show(ui, state, ps)`), UI state `ObjectsTabState` in `PanelState` (`footer_height` measured last frame).

### 7.1 Layout

```
32 objects · 3 hidden · 1,523,104 triangles          <- one weak line (no document: "No document open")
+--------------------------------------------------+
| (eye) Board                              12,480  |  rows, 22 pt each, virtualized
| (eye) Pawn_White_1                       48,112  |  selected rows: selection fill
| ( - ) Pawn_White_2  (weak text)          48,112  |  hidden: closed eye, weak name
| ...                                              |
+--------------------------------------------------+
[ Hide ] [ Isolate ] [ Show all ] [ Frame ]           <- footer, pinned below the list
```

- **List**: `egui::ScrollArea::vertical().id_salt("side_objects_scroll").auto_shrink([false, false]).max_height(page - header - footer).show_rows(ui, ROW_HEIGHT, object_count, |ui, range| …)`. Only the rows in `range` are laid out, so thousands of objects cost nothing per frame. Each row: `allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click())`; the eye is a 20 pt sub-rect at the left with its own `ui.interact(.., Sense::click())` (pattern of `layers::row`; reuse `layers::paint_eye` — make it `pub(super)`); name painted left, elided with "…" to the room before the triangle count; triangle count right-aligned, weak, with thousands separators. Hover text: full name, triangle count, "Hidden" when hidden.
- **Row clicks** (pure function `row_click_action(mods, object) -> AppAction`, like `layers::row_click_action`): plain → `SelectObject`, Ctrl → `ToggleObjectSelected`, Shift → `SelectObjectRange`. Double-click → `SelectObject` + `FrameSelection`. Eye click → `SetObjectVisible(i, hidden)` (does not change the selection). A click on the empty list area below the rows → `DeselectObjects`.
- **Footer buttons** with the shortcut in the hover text (`shortcut_text`): Hide (`HideSelectedObjects`), Isolate (`IsolateSelectedObjects`), Show all (`ShowAllObjects`), Frame (`FrameSelection`); enabled states as in the menu (4.5).
- **Reveal**: when `session.object_reveal` is `Some(i)` the scroll offset is set so that row `i` is in view (`ScrollArea::vertical_scroll_offset(i as f32 * row_pitch - visible / 2)` clamped at 0) and the request is cleared.
- **Hit rects**: `ObjectRow(i)` / `ObjectEye(i)` for the laid-out rows only, `ObjectsList` (visible list rect), `ObjectsFooter` and the four buttons.
- The panel never mutates the document: everything is an `AppAction` (rule of 8.7).

### 7.2 Tests

`panels/objects.rs` unit tests: `row_click_action`; the header text (singular / plural, no "hidden" part when none); number formatting.
`control_sweep/panel_tests.rs` (headless, `SampleKind::Objects`, `upper_tab = Objects`):
- `object_rows_take_plain_ctrl_and_shift_clicks` (through `ObjectRow` rects; resulting `selected_objects`).
- `the_eye_hides_one_object_and_keeps_the_selection`.
- `objects_footer_buttons_queue_their_actions_and_are_enabled_when_they_can_act`.
- `a_long_object_list_draws_only_the_rows_in_view`: a document built with 2,000 objects (a `RawObjects` over `soup_sets`, through `normalize_with_objects` and `testutil`): the number of recorded `ObjectRow` rects is at most `list height / ROW_HEIGHT + 2`; the footer is inside the page; setting `object_reveal = Some(1500)` brings `ObjectRow(1500)` into `ObjectsList` within two frames.
- `objects_tab_is_idle` (no actions queued without input, with a selection and hidden objects).

**Check after WP5:** workspace tests + clippy; by hand in a headless run of the `objects` scenario draft: the tab with three rows, one hidden, in the screenshot.

---

## 8. Scenarios (work package 6, `selftest/scenarios.rs`, ARCHITECTURE 8.11)

Add to `NAMES` and the dispatcher: `objects`, `objects_path`, `side_tabs`. All assertions are on state (the runner has no pixel read-back mid-run); the named shots are for the reviewer.

### 8.1 `objects` (no PATH; `OpenSample(SampleKind::Objects)`, 1280x800)

1. Assert: three objects `Floor`, `Cube`, `Ball`; triangle counts sum to the mesh; nothing hidden; `upper_tab == Tool`.
2. `Setup`: `upper_tab = Objects`. `SetTool(Navigate)`. `Dyn`: project the Cube's bounds centre to the screen (`ViewXform`), `ScreenClick` there → assert `selected_objects == {Cube}`, `ObjectRow(Cube)` drawn. `Shot("selected")` (reviewer: cube tinted blue, row highlighted).
3. `Setup`: remember the ray through that point and that it hits a Cube triangle; remember `doc.revision()`, `is_modified()`, undo depth.
4. `Key(NONE, H)` → assert Cube hidden; the remembered ray now hits a **Floor** triangle; revision / modified / undo depth unchanged. `Shot("hidden")` (reviewer: no cube, no cube wires — turn Wireframe on for this shot, then off).
5. `SetTool(Brush)`; a 3D stroke across the cube's former footprint → assert (as the core test, on the live document): Floor texels under the cube changed, the Cube's UV region byte-identical. One undo entry ("Brush Stroke").
6. Place an image stamp (the sample stamp image, Project) at the same point, `WaitIdle` → assert the Cube's UV region still byte-identical and the stamp layer has ink in the Floor's region.
7. `Key(ALT, H)` → nothing hidden; the ray hits the Cube again; the paint of step 5 is on the floor (still), the cube is clean. `Shot("shown")`.
8. `ScreenClick` on `ObjectRow(Ball)`, `Key(SHIFT, H)` → hidden == {Floor, Cube}. `Key(SHIFT, F)` → camera target ≈ Ball bounds centre (1e-3 of the diagonal) and distance smaller than before. `Shot("isolated")`.
9. Eye click on `ObjectEye(Floor)` → Floor visible, selection still {Ball}.
10. Ctrl-click `ObjectRow(Cube)`, Shift-click row range → selection sets as expected; click the footer "Show all".
11. Hide the Cube again; save to the scenario's temp path, reopen (the pattern of `roundtrip`) → Cube hidden after load, one Info toast names it, `is_modified()` false, selection empty.
12. `assert_side_panel_layout`; final `Screenshot`.

### 8.2 `objects_path` (PATH = a multi-object model, e.g. `G:\Claude_Resources\samples\khronos-gltf\ABeautifulGame.glb`)

1. Assert: more than one object; names unique and non-empty; counts sum to the mesh. Log the count.
2. `upper_tab = Objects`: at most `list height / 22 + 2` rows recorded.
3. Select the first object by a row click, Ctrl-click a second; Isolate; assert only those two visible; log the UI-thread time of the frame (`state.frame_work`) — **budget: the visibility change frame under 250 ms on ABeautifulGame**; Frame Selection; `Shot("isolated")`.
4. Navigate click in the view on a visible piece → its row becomes selected and is revealed in the list.
5. Brush stroke across the isolated pieces → `"ok"` and no texel changed outside the texture sets those objects use is **not** asserted (shared texels); assert only that the stroke made one undo entry and the frame times stay in the brush budget of the report.
6. Show All; final shot. Fails with a clear message when the model has one object.

### 8.3 `side_tabs` (no PATH, built-in sample, 1280x800)

1. Defaults: `upper_tab == Tool`, lower shows Layers; `assert_side_panel_layout`.
2. Click each upper strip button (`ScreenPointer` at `upper_tab_buttons[i]`): exactly that page is drawn; `Shot` per tab (`tab_tool`, `tab_color`, `tab_sets`, `tab_objects`).
3. G1: Color tab with tools Transform (text selected), Text, Brush (lower tab Brush), Stamp → content not cut, square at least 139 pt.
4. G2: Tool tab + Brush tool with lower tabs Layers and Brush → the "Distance between dabs" row inside `tool_area`.
5. Tool change keeps the tab: on Color, `SetTool(Eraser)` → still Color. Toolbar primary chip click from Tool → Color. Text tool click in the view (places a text) → `upper_tab == Tool` and the content field focused.
6. Drag the split edge 80 pt up → lower area 80 pt taller, upper shorter, both inside the panel; switch lower tab → same height; double-click the edge → back to the automatic height.
7. `Resize([900, 600])`, `Resize([640, 480])`: `assert_side_panel_layout` each; Color page scrolls or shrinks, nothing overlaps; `Shot` each.
8. Lower one tab rule still holds (re-use `tabs_shown` / `tab_click`).

### 8.4 Existing scenarios

All existing names must still report `"ok": true` with the helper updates of 6.6. `tools/round3_audit/sweep.ps1` job files get the three new names (`objects_path` with the ABeautifulGame path).

### 8.5 What to look at in the screenshots

- Upper strip: four buttons in one row, the shown one highlighted, nothing clipped at 300 pt; lower strip unchanged.
- Color tab: whole picker, 140 pt square, hex row, two swatch rows, no scroll bar at 1280x800.
- Objects tab: header line, eye icons, selected row fill, hidden row dimmed, footer buttons on one row and not clipped.
- 3D view: hidden object gone including its wireframe / seams; tint only on the selected object and only with the Objects tab or Navigate; no tint in Preview.
- 2D view of the trio with the Cube hidden: texture whole, the Cube's UV wires absent.
- 640x480: both strips visible, pages scroll, nothing drawn over the other group.

---

## 9. Test plan summary

| Layer | Where | New / changed |
|---|---|---|
| core unit | `mesh_build.rs`, `import/{obj,gltf,fbx}.rs`, `doc_build.rs`, `bvh.rs` | 2.5, 3.5 |
| core integration | new `crates/wte_core/tests/objects.rs` | 3.5 |
| app unit (no GPU) | `object_ops.rs`, `io_flows.rs`, `shortcuts.rs`, `gpu/scene.rs`, `tools/mod.rs`, `prefs.rs`, `panels/objects.rs`, `panels/side_groups.rs` (sizing functions: `tab_area_height`, `layers_area_max`, clamps) | 4.7, 5.6, 6.5, 7.2 |
| control sweep | `control_sweep/panel_tests.rs`, `text_panel_tests.rs` | 6.6 table, 7.2 |
| GPU (ignored) | `control_sweep/gpu_tests.rs` | 5.6 |
| scenarios | `selftest/scenarios.rs` | 8 |
| gate | all of: `cargo-gnu.cmd test --workspace`; `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`; `cargo test -p wte_app perf_ -- --ignored --test-threads=1`; `cargo test -p wte_app -- --ignored --skip perf_`; `cargo-gnu.cmd clippy --workspace --all-targets` = 0 warnings; release build; scenario sweep; view-rect pixel identity; `check_exports.py` PASS | |

The known red on `stable` (WA20) is not part of `main`; on this branch (cut from `main`) the baseline is whatever `cargo-gnu.cmd test --workspace` reports before WP1 — **record that baseline first** and keep it.

---

## 10. ARCHITECTURE.md and HANDOFF.md edits

ARCHITECTURE.md (binding; update in the same commits as the code):

| Section | Edit |
|---|---|
| 1 | new **1.9 Objects and tab groups (branch `objects-tabs`, `docs/plans/OBJECTS_TABS_PLAN.md`)**: the summary of section 0, the rules table of 3.4, "view state: not in history, not saved in the document, hidden names in `ui_state`", "`FORMAT_VERSION` stays 3, `IMPORTER_VERSION` stays 2", and that the round-3 decision "colour picker always visible" is replaced |
| 2.1 | `mesh.rs` row: `ObjectInfo`, `RawObjects`; `bvh.rs` row: `TriMask`, the hidden mask; `mesh_build.rs`: `normalize_with_objects`; `samples.rs`: `object_trio`, `to_obj_objects`; integration test row `tests/objects.rs` |
| 2.2 | new rows `object_ops.rs`, `panels/objects.rs`, `panels/side_groups.rs`, `gpu/shaders/tint.wgsl`; `state.rs` row: `UpperTab`, the object selection fields; `panels/mod.rs` row: two tab groups |
| 5.7 | the types and functions of 2.1 / 2.2; `WireEdge::object`; `MeshData` doc line unchanged about ordering |
| 5.8 | `TriMask`, `Bvh::with_hidden / hidden / is_hidden`; "every query skips hidden triangles" |
| 5.10, 5.15 | `object_trio`, `to_obj_objects`, `SampleKind::Objects` |
| 5.14 | the `Document` API of 3.2 and the rules table of 3.4 |
| 5.20 | "a bake job carries the BVH (and so the visibility) of the moment it is made"; `WrapCache::clear` on a visibility change |
| 5.21 | `RawImport::objects`; object rules per format (2.3) |
| 6.1 | step 9 "Objects"; `wire_edges` object rule |
| 6.2 | the mask test in the leaf loops; identical results without a mask |
| 6.4, 6.5, 6.5a | one sentence each: hidden triangles are never candidates / occluders / patch members; `collect_grown` rule |
| 7.2 | `ui_state` note: adds `hidden_objects` (names); 7.3: no version change, older builds ignore it |
| 8.2 | `UpperTab`, `SideRects` (table of 6.1), `PanelHit` additions, `Session` fields, the actions of 4.2 |
| 8.3 | guard list additions; `FrameModel` uses visible bounds; `FrameSelection`; where `sync_prefs` for tabs runs |
| 8.4 | visible index buffer, wire runs, tint pipeline, `View3dParams::{all_objects, highlight}`, `sync_objects` |
| 8.5 | which view draws what (Main3D / Preview / export preview / 2D); highlight rule |
| 8.6 | Navigate click-select (5.4) |
| 8.7 | Menu bar: Object menu, Three Objects sample; **Side panel rewritten** (6.2-6.4, guarantees G1-G4 replace the "three areas" text and the "whole picker on screen whatever tool is active" sentence); Objects tab (7.1); toolbar chip click |
| 8.9 | `Prefs` fields, `UiState::hidden_objects`, `install_document` restore + toast |
| 8.10 | the four bindings; table size |
| 8.11 | scenarios `objects`, `objects_path`, `side_tabs`; updated helper descriptions |
| 9.1, 9.2 | the tests of section 9 |
| 10 | a short "objects-tabs" package list (this plan's WP1-WP6) |

HANDOFF.md:
- 4.1: add the three scenario names; note that `objects_path` needs a multi-object model.
- 5: history row "objects-tabs: objects (select / hide / isolate / frame) and two tab groups — state".
- 6: new decisions dated 2026-11-23/24: two tab groups and their tabs; object rules (hide = not drawn and rays pass through; selecting never restricts painting); **"The colour picker stays visible …" marked as superseded by the tab decision**, with the mitigations.
- `dist/HOW_TO_TEST.txt`: a short "Objects" and "Tabs" checklist for the user (including "press Alt+H: does Windows beep?").
- Save this file as `docs/plans/OBJECTS_TABS_PLAN.md`; builder notes go to `docs/notes/objects_tabs/`.

---

## 11. Work packages, order, checks

Each package: one builder, then one check-and-fix pass (HANDOFF section 7). Sources stay LF; no new crates (everything here uses egui, wgpu, glam, serde already in `Cargo.lock`); the headless runner's window behaviour is not touched (the app must never take the user's focus); never drive the app with OS-level input.

| # | Package | Files (main ones) | Check |
|---|---|---|---|
| 0 | Baseline | — | record test counts, clippy, and reference screenshots + exports of all scenarios from the `main` exe |
| 1 | **Core objects** (section 2) | `mesh.rs`, `mesh_build.rs`, `import/{mod,obj,gltf,fbx}.rs`, `samples.rs`, `doc_build.rs` | `test -p wte_core` green; frozen `uv_hash` tests untouched; importing ABeautifulGame through a throw-away test or the log shows a plausible object count and names; clippy 0 |
| 2 | **Core visibility** (section 3) | `bvh.rs`, `document.rs`, `doc_build.rs`, `doc_stamps.rs` (cache clear), `wrap_param.rs`, `tests/objects.rs` | workspace tests green; perf tests not slower; clippy 0 |
| 3a | **App state, actions, menu, shortcuts, persistence** (section 4) | `state.rs`, `action.rs`, `dispatch.rs`, `object_ops.rs`, `shortcuts.rs`, `panels/menu_bar.rs`, `io_flows.rs` | workspace tests green; hide / show provable by ray assertions through actions |
| 3b | **Rendering, highlight, picking** (section 5) | `gpu/scene.rs`, `gpu/renderer.rs`, `gpu/params.rs`, `gpu/shaders/tint.wgsl`, `view/{mod,view3d,view2d,preview}.rs`, `dialogs/export.rs` (preview params), `tools/mod.rs` | GPU tests green; all existing scenarios `"ok"` and **full-frame pixel-identical to the baseline**; exports byte-identical |
| 4 | **Tab groups** (section 6) | `panels/mod.rs`, `panels/side_groups.rs`, `panels/{color,texture_sets,tool_options,toolbar}.rs`, `state.rs`, `prefs.rs`, `app.rs` (`on_exit`), the test files of 6.6 | all tests green after the listed updates; all scenarios `"ok"`; view-rect identity; screenshots of the four tabs |
| 5 | **Objects tab** (section 7) | `panels/objects.rs`, `panels/layers.rs` (`paint_eye` visibility), `panels/mod.rs` | tests of 7.2; screenshot |
| 6 | **Scenarios, docs, gate** (sections 8-10) | `selftest/scenarios.rs`, `ARCHITECTURE.md`, `HANDOFF.md`, `dist/HOW_TO_TEST.txt`, sweep job files | the full gate list of section 9; `objects`, `objects_path` (ABeautifulGame), `side_tabs` at 1280x800, `side_tabs` also started at 900x600 and 640x480; gate report in `docs/reports/OBJECTS_TABS_GATE_REPORT.md` |

Packages 4 and 5 touch the same panel files and may be built by one builder back to back; 1 → 2 → 3a → 3b are strictly ordered.

---

## 12. Risks and open points

1. **Colour picker visibility** (0.1 item 1) — a user decision replaced; confirm the mitigations are enough. If the user wants the picker always visible after trying it, the cheapest fallback is a compact primary-colour row (chip + hex) at the top of the Tool page; not planned.
2. **Stamp bakes depend on visibility at bake time** (3.4). Simple and predictable, but a stamp edited after "Show all" changes where it lands. The alternative (store the hidden set per stamp) needs a project format change and was rejected for this round.
3. **Brush speed.** The mask adds one branch per BVH leaf triangle. Expected to be noise; the round-7 timing is sensitive, so the perf tests and a `WTE_STROKE_LOG=1` run on ABeautifulGame are part of the WP2 / gate check.
4. **Visibility toggle cost on heavy models**: mask build + index compaction + 18 MB upload on 1.5 M triangles; budget 250 ms (8.2). If it is over, compact on a worker and swap the buffer when ready (the BVH mask stays immediate).
5. **Wire order on multi-object models**: grouping wire instances per object changes the draw order where a seam and a boundary of different objects cross; a few pixels may differ from `main` with Wireframe / Seams on. Single-object models are byte-identical. Accept and note in the gate report.
6. **`collect_grown` Jump branch, tool hover caches, `uv_outline_in_3d`** — the three places marked "builder: check" (3.3, 4.3, 5.5).
7. **Alt+H on Windows** may beep or be eaten by the system menu handling in winit; cannot be seen headless. Manual check by the user; fallback binding Ctrl+Shift+H.
8. **Upper strip width**: four buttons at 300 pt were estimated to fit (about 242 pt) but not measured. If they do not fit, shorten "Texture sets" to "Textures" (hover text "Texture sets") rather than wrapping the strip; the G4 test decides.
9. **egui details**: `ScrollArea::show_rows` with a custom row allocation and a per-row sub-interaction (eye) — follow the `layers::row` pattern (allocate the row first, interact with the sub-rect after) so the eye wins the click; a double-click on the panel resize edge must be detected from raw pointer input like the existing press detection, since the panel's own resize handle consumes the drag.
10. **Test churn in WP4** is the largest cost of the plan (about 30 + 26 references in the panel tests and 81 in the scenarios). The mitigations are structural: field and enum names are kept, the default tabs are Tool + Layers so most tests see the widgets they saw before, and the lower group's code is untouched.
11. **Objects that share texels** (instanced chess pieces): hiding one does not protect its texels from paint on its twin. Documented, not solvable at this level.
12. Not in this round: object name filter in the list, context menu on object rows, Alt-click on an eye to isolate, object name in the status bar hover, a silhouette outline instead of the tint, selection highlight in the 2D view, renaming objects.

---

### Critical Files for Implementation
- G:\software_projects\texture_wrapper_ap_ui\crates\wte_core\src\mesh_build.rs
- G:\software_projects\texture_wrapper_ap_ui\crates\wte_core\src\bvh.rs
- G:\software_projects\texture_wrapper_ap_ui\crates\wte_app\src\panels\mod.rs
- G:\software_projects\texture_wrapper_ap_ui\crates\wte_app\src\gpu\scene.rs
- G:\software_projects\texture_wrapper_ap_ui\crates\wte_app\src\selftest\scenarios.rs

Other files the plan touches heavily: `crates\wte_core\src\{mesh.rs, document.rs, wrap_param.rs, samples.rs, doc_build.rs}`, `crates\wte_core\src\import\{mod.rs, obj.rs, gltf.rs, fbx.rs}`, `crates\wte_app\src\{state.rs, action.rs, dispatch.rs, shortcuts.rs, io_flows.rs, prefs.rs}`, `crates\wte_app\src\gpu\{renderer.rs, params.rs}`, `crates\wte_app\src\tools\mod.rs`, `crates\wte_app\src\panels\{menu_bar.rs, color.rs, texture_sets.rs, tool_options.rs, toolbar.rs, layers.rs}`, `crates\wte_app\src\control_sweep\{panel_tests.rs, text_panel_tests.rs, gpu_tests.rs}`, `ARCHITECTURE.md`, `HANDOFF.md`.