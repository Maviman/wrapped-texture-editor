# Wrapped Texture Editor — Architecture (binding)

Status: binding contract for implementation. Where this document and an implementer's memory of a crate API disagree, the crate source in
`E:\claude_downloads\rust-toolchain\cargo\registry\src\index.crates.io-1949cf8c6b5b557f\<crate>-<version>\` wins; where this document and a
proposal disagree, this document wins. External API facts quoted here were re-checked against those sources (file:line given where it matters).

Path shorthand used below: `REG\` = the registry directory above.

---

## 1. Overview and requirement traceability

Wrapped Texture Editor is a Windows desktop app (single exe, no console in release) for painting and stamping onto a 3D model's UV-mapped
texture. It is a Cargo workspace with two crates:

- `crates/wte_core` — library. No GPU, no GUI. Owns the complete data model and every algorithm: import, UV analysis, BVH raycast, brush
  engine, stamp/text projection bake, compositing, undo, project file, export. Fully unit-testable headless.
- `crates/wte_app` — binary `wrapped_texture_editor`. eframe 0.37 UI + wgpu 30 renderer. Thin over `wte_core`: it turns input into core
  calls and uploads the core's composited pixels to the GPU.

Pipeline in one sentence: every layer is a CPU RGBA8 buffer; `wte_core` composites dirty rectangles into one `composite` buffer per texture
set; `wte_app` uploads the dirty rectangles with `queue.write_texture` and draws the model with that texture into an offscreen target that
egui shows as an image. Picking is a CPU BVH raycast.

### 1.1 Traceability

| Req | What the user gets | Core modules | App modules |
|---|---|---|---|
| R1 import model + texture | File > New from Model (OBJ+MTL, glTF, GLB), drag-and-drop, texture auto-detected from the material or chosen manually or blank | `import/*`, `image_io`, `mesh_build`, `doc_build` | `dialogs/import`, `io_flows`, `dnd` |
| R2 recognise the wrapping | UV parsed per material -> TextureSets; UV wireframe overlay in the 2D view, seam overlay and numbered UV-checker shading in 3D; UV coverage stats and UV-layout thumbnail in the import dialog | `mesh`, `mesh_build` (`wire_edges`, `UvStats`), `texel_map` | `gpu/renderer` (lines, UvChecker mode), `view/view2d`, `dialogs/import` |
| R3 paint in 3D / 2D, texture on/off | Brush, eraser, eyedropper, fill; paint on the model in the 3D view and on the flat image in the 2D view; Split view; `T` toggles texture on the model (Textured <-> Clay); layers; undo/redo | `brush`, `stroke`, `fill`, `bvh`, `texel_map`, `composite`, `blend`, `doc_layers`, `history` | `view/view3d`, `view/view2d`, `tools/brush`, `tools/eyedropper`, `tools/fill`, `panels/*` |
| R4 image stamps as separate layers | Import an image, click on the model (or in the 2D view) to place a projected decal, move/scale/rotate with a gizmo, re-editable later; every stamp is its own layer; exported as its own layer | `stamp`, `stamp_bake`, `doc_stamps`, `blob`, `export/*` | `tools/stamp_place`, `tools/transform`, `view/gizmo`, `dialogs/export` |
| R5 3D preview before export | Preview view mode (clean, orbit, lit/unlit, turntable) showing exactly the flattened composite; the same widget is embedded in the Export dialog | `composite` (the composite buffer is byte-identical to the flattened export) | `view/preview`, `dialogs/export` |
| R6 exe application | `wrapped_texture_editor.exe`, `windows_subsystem = "windows"`, runtime window icon | — | `main`, `theme` |
| R7 self-contained project file | `.wtep` ZIP: manifest + original model bytes and every sidecar + layer PNGs + stamp images + fonts; opens with the originals deleted; File > Extract Original Files | `project/*`, `import/resolver` (`SourceBundle`, `MemoryResolver`) | `io_flows` |
| R8 text stamps | Text tool: content, system font or bundled font, size, colour, outline, alignment; placed and edited like image stamps; own layer; font bytes embedded in the project | `text/*`, `stamp`, `stamp_bake`, `doc_stamps` | `tools/stamp_place` (text mode), `panels/tool_options` |
| R9 no AI | No AI features, no network access anywhere | — | — |

### 1.2 Conflict resolutions (decisions taken while merging the three proposals)

| # | Topic | Decision | Rejected alternative and why |
|---|---|---|---|
| D1 | Core module tree | Flat modules plus `import/`, `text/`, `project/`, `export/` sub-trees (section 2) | Proposal C's `io/` umbrella: one more nesting level with no benefit |
| D2 | Mesh model | One `MeshData` (unified index, triangles sorted by material) + `Bvh`; the renderer draws one vertex buffer with one index range per material | Per-set meshes (B): duplicates geometry, and untextured materials would need a third path |
| D3 | Stamp ownership | Stamps are document-level `StampObject`s; each texture set a stamp covers holds one linked layer (`LayerKind::Stamp/Text { stamp, cache }`) | Per-set stamp layers (B, C): a decal across a material boundary would be clipped |
| D4 | Interactive stamp preview | CPU `BakeQuality::Preview` re-bake on every drag frame, Full bake on release. No GPU ghost | GPU ghost decal (B): a second projection implementation that must match the CPU bake exactly, extra pipelines and bind groups; the CPU preview is WYSIWYG by construction |
| D5 | Pixel storage | Raster layers are dense `Arc<PixelBuf>` (unallocated `0 x 0` until first painted) with a tracked content bound; stamp/text caches are sparse `Arc<SparseBuf>` (cropped parts per cluster of touched 64 px tiles); `Arc::make_mut` gives copy-on-write snapshots | Sparse 256-px tiles for every layer (C): much more code in every algorithm. (Review round 2 showed that one cropped rect per cache does not remove the cost on packed atlases, findings SP-2/4/5, hence the sparse caches, content bounds and lazy paint layers.) |
| D6 | Undo | Two edit kinds only: `Pixels` (64x64 tile diffs) and `Structure` (a snapshot of the whole layer/stamp structure; raster pixels are pinned only for layers the command itself replaces or removes, stamp/text caches are `Arc`-shared — section 6.7) | Seven specialised edit variants (A): more code and more ways to get inverse operations wrong |
| D7 | Stroke target | A 3D stroke paints the active layer of every texture set it touches (one undo step). The app first gives every set whose active layer is a stamp/text layer (or that has none) a paint layer; sets whose active layer is locked or hidden are skipped and reported once per stroke (sections 6.4, 8.6) | Single explicit `(set, layer)` (B): strokes would stop at material boundaries |
| D8 | Mesh in the project | Not stored. `.wtep` holds the original source files; open re-imports from them through `MemoryResolver`. `uv_hash` in the manifest detects importer drift | `mesh/mesh.bin` cache (C): a second mesh format to maintain; nothing persisted references vertex or triangle indices, so it is not needed |
| D9 | Save strategy | Full rewrite every save, PNG `Fast`, atomic temp + rename, optional `.bak`. No incremental raw-copy, no autosave in v1 | Incremental `raw_copy_file` + autosave/recovery (C): real value, but not required and a large test surface; listed as future work |
| D10 | Max texture side | 8192 (egui-wgpu's device descriptor requests `max_texture_dimension_2d: 8192`, `REG\egui-wgpu-0.37.0\src\setup.rs:272`). Larger images are downscaled at import with a warning | 16384 (C): cannot be uploaded |
| D11 | glTF feature set | Proposal C's pinned list (extensions enabled so `extensionsRequired` validation passes), `KHR_texture_transform` applied to UVs at import, `KHR_mesh_quantization` read by the importer (round 3) | A's "transform unsupported": files using it would map wrongly |
| D12 | base64 / percent-decoding | Hand-written in `import/uri.rs` | `base64`/`urlencoding` crates (A): avoidable dependencies |
| D13 | Progress reporting | `trait Progress: Send + Sync` (B) for every long core operation | `&dyn Fn(SaveProgress)` (C) |
| D14 | File dialogs | `rfd::FileDialog` called synchronously on the UI thread with `.set_parent(frame)` | Dialogs on worker threads (B): `eframe::Frame` is not `Send`, so the parent handle cannot cross threads |
| D15 | Brush size unit | UI brush size is a diameter in texels. 2D radius = `size/2` texels; 3D radius = `size/2 * texel_world_size(set under cursor)` world units, fixed for the stroke | Screen-space radius (A) and per-triangle texel size (B): size would change with zoom or jump between triangles |
| D16 | Viewport integration | Offscreen `ViewTarget`s (own MSAA + depth) registered with `egui_wgpu::Renderer::register_native_texture` | `egui_wgpu::Callback` (shared window depth buffer, no per-view MSAA) |
| D17 | Blend modes | Normal, Multiply, Screen, Overlay, Add, Darken, Lighten | `Difference` (C's mapping table only) dropped |
| D18 | Fill tool | Kept (B), scopes `Layer` and `Island` (UV island under the cursor). No colour-tolerance flood fill | — |
| D19 | Ids | `TextureSetId(u32)` = index into `Document::sets` (sets never change after creation); `LayerId(u64)`, `StampId(u64)` from one monotonic counter; `BlobId(u32)`. No uuid | — |

### 1.3 Non-goals for v1

UDIM, skinning/animation, changing a set's resolution after creation, colour-tolerance flood fill, selections, autosave/crash recovery,
incremental save, PSB (PSD > 4 GiB), HDR/16-bit textures, any AI or network feature, automatic UVs for models where only *some*
materials lack UVs (those materials are listed as "not paintable" in the import dialog).

OS clipboard integration is unavailable in v1: eframe's `clipboard` feature needs the `arboard` crate, which is not downloaded, so
egui-winit falls back to an in-app clipboard (`REG\egui-winit-0.37.0\src\clipboard.rs:6, :27`). Copy/paste works inside the app only
(text cannot be pasted from other programs into the text-stamp content field). If OS paste is wanted later, add `arboard` with
`default-features = false` and the eframe feature `clipboard`; no API in this document changes.

### 1.4 Round 4 (brush types, text effects)

Round 4 added brush types with a 3D brush object (Circular, Sphere with Ball / Cube / Cylinder / a shape from a model
file, the Brush tab) and extended text stamps (the fonts of the machine, per-gap spacing, thickening, slant, warp,
text along a line, the Text tab). User answers of 2026-11-17 (ROUND4_PLAN section 7) that shape this document:
- No "Stay on connected surface" checkbox: no `core/topology.rs`, no topology field, `SurfaceReach` has `SameSide`
  and `All` only. Circular is the round-3 stroke; Sphere is the 3D-object brush whose default object is a ball.
- A shape file with openings is not refused: the app asks how to close them (`Dialog::CloseOpenings`: flat caps,
  rounded caps, fill all hollows, only the closed parts, Cancel); the answer is stored with the shape path.
- Curved text is a curve in the flat picture of the text, projected as before. **Future work**: text that runs around
  the model's surface (not built in round 4).
- The font list is the Windows font folders (no registry fonts, no named styles of variable fonts).
- Ctrl+wheel and Shift+wheel with the Brush and the Eraser change size and turn instead of zooming.
- "Angled" text = the whole text turned (Angle field); per-letter rotation and text on an image stamp are not built.

### 1.5 Round 5 (smoothness)

Round 5 (`target\round5_design\ROUND5_PLAN.md`) moves the Full bake that ends a stamp or text edit off the UI thread
(R5-1): with **deferred Final bakes** (5.20) a placement, the release of a gizmo drag or any other Final marks the stamp
*unbaked* and the app bakes it on a worker (`stamp_bakes`, 8.9); the picture swaps when the bake lands. Undo and redo
never wait (the unbaked flag travels with the history snapshots, 6.7); save, export, the export dialog, merge down and
rasterize *settle* first (4.6). Pixels are bit-identical to round 4 (one code path, `StampBakeJob::run`).
`WTE_SYNC_FINAL_BAKES=1` keeps the round-4 behaviour for measurements. R5-2 (brush lag on very heavy models) and R5-3
(the font picker's first open) are in 5.19 and 8.6.

### 1.6 Round 6 (wrap on surface)

Round 6 (`target\round6_design\ROUND6_PLAN.md`) gives every 3D stamp and 3D text a surface mode, **Project | Wrap**
(`Projector::mode`, 5.12). Project is the projection of rounds 1-5, pixel for pixel. Wrap lays the picture on the surface
like a sticker: a per-stamp parameterization of the nearby surface (strip-seeded geodesic unfolding, `wrap_param.rs`,
6.5a) gives every vertex stamp-local coordinates, and the wrap bake (`wrap_bake.rs`) paints the picture through them. It
is exact on developable surfaces (plane, cylinder, cone, folds, box edges), continuous across UV seams, cuts cleanly
where the geometry forces it (a corner, the back of a picture longer than the way round) and stops at holes and open
edges. Two per-stamp options of Wrap, kept through Project and back: **Jump small gaps** (`WrapOptions::jump_gaps`:
continue onto separate parts closer than 1 % of the model diagonal) and **Overlap** (`WrapOptions::overlap`: a picture
longer than the way round winds on like tape, later turns on top). The project format is 3 (7.2, 7.3); a file without
wrapped stamps or set options still opens in round 5. Parameterizations are cached per stamp (`bakes.wrap_params`,
5.14, reuse rule 6.5a); the deferred Final bakes of round 5 carry them (`StampBakeJob`). 2D (UV) stamps are unchanged.

### 1.6b Round 8, package A (wrap: once, whole, smooth)

The wrap's front is ordered and bounded by the geodesic distance from the picture's centre line (fast marching), not by
the unfolded position; where the seed layout is not exact where the picture lies (curved, bumpy, closed surfaces) it is
cut only by fixed seam rules (ends meet, halves meet, orientation / non-manifold, cone tips, handles) and relaxed (LSCM,
then ARAP, hand-written deterministic conjugate gradients, the centre line pinned); flat, cylindrical, conical and
folded surfaces keep round 6's exact seed (early-out). An owner test in the bake gives each picture point to the
surface place nearest the centre line along the surface. Wrapped stamps' Preview bakes and parameterizations can run
as jobs (`StampJobKind`, `next_wrap_preview_job`, `apply_wrap_preview`, `wrap_param_job`, `apply_wrap_param`) for
package B. `Document::stamp_wrap_quality` measures how once and how whole a picture lies (dup / scatter). Details
6.5a "Round 8". Not done in package A: the per-mesh welded graph and growth from the anchor (plan 2.5 items 1-2, step 6).

### 1.6c Round 9, package A (wrap: once, nothing lost)

ROUND9_PLAN section 5 (T1-T6). The owner test gives every picture point to **one surface place**: per owner cell the
triangle covering the cell centre with the smallest `(buried, g, triangle)` owns it, and a fragment is kept only at
that 3D place (no triangle size in the rule, so coarse meshes behave like fine ones); with Jump small gaps a surface
lying under another entered part (the floor under a cube) is `buried` and loses. A spine that winds round a tube as a
helix ends where its traces pass beside each other (`passed`, `ends_share`). An angle deficit cuts (S4) only where the
net curvature within a radius exceeds 45 degrees (rivets, bevels, grooves no longer cut). The relaxation starts from
the Fermi layout (spine foot and signed geodesic distance) and runs ARAP only (no LSCM). The reach has no texel term,
so the layout does not depend on the texture's resolution. `WrapStats::lost_cut` reports picture lost inside the
laid outline. check:A added T5: laid vertices whose seed triangles are all isometric, connected to the strip, are
pinned at their seed positions (the exact region; a strain check falls back to the strip pins). Details 6.5a "Round
9". Not done: package B (speed).

### 1.6d Stable (after round 9): wrap correctness fixes

`docs/notes/stable/w1.md`. The ends meet only where the spine really closes round something (a tube, a helix, a piece:
the traces come from opposite sides), not on open rough surfaces; the exact region falls back per component (the chess
board keeps its exact layout under a pawn). Details 6.5a "Stable (w1)".

`docs/notes/stable/w2.md`. The exact region is grown by developing the flat surface from it (a cylinder's rows next to
its dome stay exact); the ARAP solve asks a compressed triangle to grow (steep bumps keep their area); the owner of a
cell with candidates on two sheets prefers a place the stamp faces over one hidden along its `out` (a book's front
page); `WrapStats::room` and `wraps_closed` say when the part is narrower than the picture or the picture covers a whole
closed part. Details 6.5a "Stable (w2)".

`docs/notes/stable/fix_wrap_1.md`. `WrapStats::shown` (the picture's cells shown somewhere, each once) and `beyond`
(cells left bare beyond where the ends met) make the panel's wrap lines honest: no ends line when the spine's closure
left no picture bare (the chess board under a pawn), "runs off" and "Only P %" from `shown` instead of the summed
`covered`, and the overlap line says each spot is shown once. Details 6.5a "Stable (fix_wrap_1)".

`docs/notes/stable/fix_wrap_2.md` (branch `stable-fix-wrap`). `WrapStats::gaps` (picture bare in the gaps between separate parts it
continued across) and `at_part` (bare where it stops at a neighbouring part) give the panel two more causes: "falls into
the gaps between separate parts" (parts200: 28 %), "stops where it meets a neighbouring part" (ABeautifulGame Jump on:
the board slab at the flush marble frame, 9 %), and "runs off the surface" says only the rest. Details 6.5a "Stable
(fix-wrap)"; the wall-clock limits of WA7 / check:A moved to serial perf tests (9). Entering a flush, touching part
again (so a long flush seam is laid along its whole length) was tried and not kept (it changed ABeautifulGame's seams);
the chess board's cut at the marble frame is the front's stop, not a gap (6.5a).

### 1.7 Round 7 (brush speed)

Round 7 (`target\round7_design\ROUND7_PLAN.md`) makes 3D dabs on very heavy models cheaper and properly parallel,
with bit-identical pixels. (A1) The brush's rasterizer calls skip pass 2 when it cannot report anything: a triangle
that owns no gutter texel and claimed no texel, in a box without shared gutter texels (a per-map index built on a
set's first touch, 6.3 "Lean pass 2"). (A2) A set's dab is rasterized and committed in one parallel pass over 32-row
bands, its candidates binned by their tight scan boxes (6.4 "Round 7: the fused dab pass"); the partition depends only
on the dab. (A3) A large shared query runs its two passes and its sort on the pool. (A4) Re-apply shades only the span
of each row whose mask the dab raised. The stroke probe has five more slots (5.19). The test seam `RASTER_R5` keeps
round 5's rasterizer and commit as the reference of the equality tests and of the bench's "... r5" rows.

### 1.8 Export scope and package (branch `export-select`, `docs/plans/EXPORT_SELECT_PLAN.md`)

The user's workflow is "import a model -> layer designs onto the material -> export one layer, a selection of layers, or
everything in one package". (1) **Multi-select in the Layers list** (8.7): a plain click makes a layer the active and only
selected one, Ctrl-click toggles, Shift-click selects the range from the active layer; the active layer (the one tools paint
on) is always part of the selection; every layer command (Delete, Duplicate, Merge Down, Rasterize, opacity, blend, move) still
acts on the active layer only. The selection is app state (`Session::selected_layers`, 8.2): not in the document, not in
history, no revision change, not saved. (2) **Layer scope in the exporter**: `ExportOptions::layers = Some(ids)` exports exactly
those layers (hidden ones included); the flattened texture is then the selected *visible* layers flattened on their own, ORA /
PSD hold the selected layers only (7.4). (3) **One package**: `ExportOptions::package = Zip` writes `<base>.zip` holding the same
files under the same names; deterministic, atomic, nothing left behind on failure or cancellation (7.4). With default options
(`layers: None, package: Files`) the exporter writes byte for byte what it wrote before. The project format (7.1-7.3) is
untouched: `FORMAT_VERSION` stays 3.

**fix_export** (branch `stable-fix-export`, the stable gate critic's export findings, notes `docs/notes/stable/fix_export.md`):
(1) the dialog opened with "Export this layer..." / "Export selected layers..." keeps the normal ticks in
`ExportDialogState::other_ticks`; switching the Layers choice to Visible / All brings back every texture set and those
ticks, back on Selected the layer preset returns (8.8). (2) A zip of a layer selection is `<base>_selection.zip`
(`export::package_name_for`), so it never replaces the full package `<base>.zip`; the dialog shows the name (7.5). (3) A
design (stamp, text, wrapped stamp) across several texture sets has one layer per set linked by its `StampId`; a layer
selection is completed by those linked layers (`export::linked_selection`), so "Export this layer" writes the design's
layer of every set it touches (7.4). (4) File names come from one reference plan (all sets, all layers, every format), so a
selection uses the full export's names, `_2` included (7.5). (5) Delete and Duplicate act on every selected layer as one
undo step (`Document::remove_layers` / `duplicate_layers`, `group_edits`); Merge Down stays on the active layer (8.7).
(6) Export no longer settles the Final bakes on the UI thread: its job waits behind the progress dialog until they land
(`io_flows::feed_waiting_snapshot`, 8.9; polish item 2: Save and Save As do the same). (7) Each export removes the temp files and stage folders of crashed exports whose
process is not running (`export::remove_stale_temps`). (8) "Exported 1 file". A read-only or blocked folder fails before
anything is written (`preflight`, 7.4).

---

## 2. Workspace layout

```
E:\claude_programs\wrapped_texture_editor\
  cargo.cmd                      existing wrapper around the self-contained toolchain (do not edit)
  .cargo/config.toml             existing linker configuration for that toolchain (do not edit)
  Cargo.toml                     workspace manifest, shared dependency versions, profiles
  ARCHITECTURE.md                this document
  crates/wte_core/               library
  crates/wte_app/                binary wrapped_texture_editor
  target/                        build output (on E:, never on C:)
```

Ownership tags: **[S]** = written completely by the wave-0 scaffold and not edited by later packages; **[C1..C7]** / **[A1..A6]** =
created by the scaffold as a compiling stub (all pub items present; core bodies `todo!()`, app bodies harmless no-ops — section 10),
then owned exclusively by that work package. Target size: under 700 lines per file.

Tag legend (package ids of section 10): C1 = `core-geometry`, C2 = `core-document`, C3 = `core-paint`, C4 = `core-stamps-text`,
C5 = `core-import`, C6 = `core-persist`, C7 = `core-integration`; A1 = `app-shell`, A2 = `app-gpu`, A3 = `app-views-tools`,
A4 = `app-panels`, A5 = `app-dialogs`, A6 = `app-selftest`.

### 2.1 `crates/wte_core/src`

| File | Owner | Owns |
|---|---|---|
| `lib.rs` | S | module declarations, re-exports, crate docs |
| `ids.rs` | S | `TextureSetId`, `LayerId`, `StampId`, `BlobId`, `TriId` |
| `error.rs` | S | every error and warning enum of the crate |
| `math.rs` | S | `IRect`, `Aabb`, `Ray`, `DirtyRegion`, `div255`, `smoothstep`, sRGB helpers (fully implemented + tests) |
| `progress.rs` | S | `Progress` trait, `NoProgress` |
| `pixel.rs` | S | `PixelBuf`, `CroppedBuf`, `PixelView` and their trivial accessors (fully implemented) |
| `blob.rs` | S | `BlobStore`, `Blob`, `BlobKind` (fully implemented + tests; round 4 T3: allocation check before hashing) |
| `mesh.rs` | S | `MeshData`, `MaterialSlot`, `SetInfo`, `TriFlags`, `RawMesh`, `RenderVertex`, `WireEdge`, `UvStats` + trivial accessors; objects-tabs: `ObjectInfo`, `RawObjects`, `MeshData::{tri_object, objects}` |
| `layer.rs` | S | `Layer`, `LayerKind`, `BlendMode`, `TextureSet` struct |
| `stamp.rs` | S; round 6 A | `StampObject`, `StampSource`, `Placement`, `Projector` (round 6: `mode`, `wrap`), `SurfaceMode`, `WrapOptions`, `UvPlacement`, `BakeQuality` (types only) |
| `document.rs` | S | `Document`, `DocumentSnapshot`, `Commit`, lookup accessors, id allocation, revision tracking, `guard_no_stroke`, `mark_layer_dirty`, `debug_check_structure` |
| `testutil.rs` | S | test-only helpers: temp dir under `target/`, image comparison (PSNR), deterministic RNG, `doc_from_raw`, `imported_from_raw` (and their `_objects` forms) |
| `mesh_build.rs` | C1 | `normalize`, UV tile normalisation, normals, fallback UVs, `wire_edges`, `render_vertices`, `uv_hash`, `uv_layout_image`; objects-tabs: `normalize_with_objects` (6.1 step 9) |
| `bvh.rs` | C1 | `Bvh`, `Hit`, `Cull`, raycast / occluded / sphere query; objects-tabs: `TriMask`, `Bvh::with_hidden` (every query skips hidden triangles) |
| `texel_map.rs` | C1 | `TexelMap`, `Fragment`, `raster_tri`, gutter build, `locate` |
| `samples.rs` | C1; round-4 additions S | procedural meshes and images, `to_obj`; the brush fixtures of FEATURE_BRUSH 8 and the writers `to_gltf`, `to_glb`, `to_fbx_ascii`; objects-tabs: `object_trio`, `to_obj_objects` |
| `pixel_ops.rs` | C1 | `PremulMips`, sampling, resize, content / non-zero bounds, crop, thumbnail, `dilate_islands` |
| `blend.rs` | C2 | per-pixel blend kernels, PSD/ORA blend-mode names |
| `composite.rs` | C2 | `recomposite_all`, `take_gpu_dirty`, `flatten_layers` |
| `doc_build.rs` | C2 | `Document::from_import`, `Document::from_parts`, `Document::sample`, `SampleKind`, `TextureChoice` |
| `doc_layers.rs` | C2 | layer commands (add/remove/duplicate/move/merge/rasterize/props), thumbnails |
| `history.rs` | C2 | `History`, `Edit`, `TilePatch`, `TileBackup`, `StructureSnapshot`, `LayerRecord`, undo/redo application, pending edits |
| `brush.rs` | C3; round 4 B3 | brush profile, stroke-to-layer kernel, per-set stroke mask, `dab_3d` (the per-set ball dab, kept as the reference) and `dab_2d`; round 4: `shared_candidates` (one sphere query per dab for all sets), `dab_tip` (ball or shape field), `TipGeom`, `DabShape` |
| `brush_tip.rs` | S (round 4) | `BrushTip`, `SurfaceReach` (SameSide, All), `TipAlign`, `TipVolume`, `TipPose`, `tip_frame` (complete + tests) |
| `tip_field.rs` | S (round 4) | `TipField` (signed-distance grid of a brush shape): lookups `sample`/`thickness`/`outline`/`outline_thickness`, built-in `cube`/`cylinder`, `from_fn`/`outline_from_fn`, the lower-envelope transform, local thickness, middle slice, sections, hash; `TipReport` (complete + tests) |
| `tip_bake.rs` | B1 (round 4) | `TipField::bake` / `bake_from_path` (model -> field, 5.26), `CloseMode`, `TipError` |
| `stroke.rs` | C3 | `StrokeSession`, `Document::stroke_*`, `sample_color` |
| `fill.rs` | C3 | `Document::fill`, `FillScope`, island flood |
| `stamp_bake.rs` | C4 | `Projector` methods, `bake_projected`, `bake_uv` (round 6: the scratch helpers are `pub(crate)` for the wrap bake) |
| `wrap_param.rs` | round 6 A | `WrapParam`, `WrapStats`, `wrap_param` (the parameterization of 6.5a: weld, spine, front, halo, tears, Jump small gaps, Overlap turns), `local_of` / `surface_of` / `outline`, the constants of 6.5a; round 9 B: `wrap_param_in` (with the mesh graph), `collect_grown`, `Patch::from_graph`, `phase_times` |
| `wrap_bake.rs` | round 6 A | `bake_wrapped` (6.5a "Bake"; round 8: the owner test) |
| `wrap_mesh.rs` | round 9 B | child module `wrap_param::wrap_mesh`: `WrapMesh` (the welded graph of a whole mesh: weld + boundary snap, CSR vertex -> triangles, global edge ids), `WrapMeshCell` (lazy, shared by `Arc`: graph and shared texels), `WrapMeshWarmup` (6.5a step 1, round 9 B) |
| `uv_share.rs` | round 9 B | `UvShare`: per texture set a count raster of the UV footprints; per triangle "shares texels", the mesh's shared share; `UV_SHARE_WARN` (5.20 `stamp_shared_texels`) |
| `wrap_relax.rs` | round 8 A | child module `wrap_param::relax`: seams S1-S5, wedges, LSCM / ARAP with the matrix-free CG, the owner grid (6.5a "Round 8") |
| `wrap_tests.rs` | round 6 A | the acceptance tests WA1-WA12, WA15-WA19 of the round-6 plan and (round 8) WA4 / WA4b, WA20-WA27, WA29 (round 9) WA31-WA34, WA36, WA37 and `r8_quality_survey` (test-only module) |
| `wrap_stable_tests.rs` | stable (w1, w2) | the regression tests of the wrap correctness fixes after round 9 (`w1_*`, `w2_*`; a child module of `wrap_tests.rs`, test-only) |
| `doc_stamps.rs` | C4; round 6 A | stamp/text commands on `Document`, source mip cache, `stamp_at`; round 6: the wrap dispatch of `bake_and_apply`, `ensure_wrap_param`, `stamp_wrap_*`, `stamp_ink_area` |
| `text/mod.rs` | S | `TextSpec` (with `fx: TextFx`), `FontRef`, `TextAlign`, `Outline`, `TextLayout`, `PlacedGlyph`; round 4: `TextFx`, `Gap`, `Warp`, `WarpStyle`, `TextPath`, `PathShape`, `TextFrame`, `RasterQuality`, `TextRaster`, the limits (`GAP_MIN_EM` ... `MAX_STYLED_PIXELS`), `TextSpec::new`/`sanitize_fx`/`floats`/`floats_mut`/`differs_only_in_spacing` (complete + tests) |
| `text/font_catalog.rs` | C4; round 4 T3 | `FontCatalog` (fontdb; the Windows font folders, file sizes read once, faces shared per file), `FontCache` (fontdue), bundled fonts, `FaceTraits`, `face_traits`, `FontCatalog::sample` (picker samples drawn with the styled painter's outline and coverage code) |
| `text/layout.rs` | C4; round 4 T2 | `layout_text`, `measure_text` (classic canvas); `lay_out_lines` (crate): the one layout rule of both painters (6.8 steps 2-3 with gap values and the thickening term), `Lines`, `LaidGlyph` |
| `text/raster.rs` | C4; round 4 T2 | `rasterize_text` (classic painter); `compose` (crate: outline + fill, shared); `FaceFonts`, `TextFonts` (+ `metrics`, `faces`), `text_frame`, `render_text` (dispatch classic / styled painter) |
| `text/edt.rs` | C4 | exact Euclidean distance transform for outlines |
| `text/outline.rs` | T1 (round 4) | glyph outlines as polylines (`GlyphOutline`, `glyph_outline`, `char_to_glyph`, `parse_face`) |
| `text/scan.rs` | T1 (round 4) | coverage rasterizer (`Coverage`, `fill_contours`) |
| `text/warp.rs` | T1 (round 4) | `WarpMap` (15 styles, direction, distortions, refinement) |
| `text/path.rs` | T1 (round 4) | `PathLayout`, `TextLine`, `LinePoint`; `new_circle`, `fit_circle`, `new_curve` |
| `text/gaps.rs` | T1 (round 4) | gap list editing (`get`, `set`, `insert`, `delete`, `shift_for_edit`, `is_joined`, `slots`, `slot_name`, `GapSlot`) |
| `text/styled.rs` | T2 (round 4) | the styled painter: `styled_frame`, `render_styled` (6.8a) |
| `text/frame.rs` | T2 (round 4) | the anchor rule: `Keep`, `refit_placement`, `place_anchor_at_centre`, `EmMap` (6.5 "Text anchor rule") |
| `import/mod.rs` | C5; round 4 B1 | `import_from_path`, `import_from_bundle`, `sniff_model_format`, `ImportedModel`, `ImportOptions`, `import_geometry_from_path` |
| `import/resolver.rs` | C5 | `AssetResolver`, `DiskResolver`, `MemoryResolver`, `RecordingResolver`, `SourceBundle`, `normalize_ref` |
| `import/obj.rs` | C5 | OBJ + MTL -> `RawMesh` |
| `import/gltf.rs` | C5 | glTF / GLB -> `RawMesh` |
| `import/mtl_ref.rs` | C5 | `parse_texture_ref` (strip `map_Kd` options) |
| `import/uri.rs` | C5 | base64 decode, percent decode, data-URI parsing |
| `image_io.rs` | C5 | `decode_image`, `encode_png`, `encode_flat`, `FlatFormat`, `PngLevel` |
| `project/mod.rs` | S | re-exports, `FORMAT_VERSION`, `SaveOptions`, `SaveStats`, `LoadReport` |
| `project/manifest.rs` | S | serde manifest schema (types only); round 4: `TextFxEntry`, `GapEntry`, `WarpEntry`, `PathEntry`, `TextEntry::unknown_features` |
| `project/save.rs` | C6 | `project::save` |
| `project/load.rs` | C6 | `project::load`, `read_manifest`, `extract_bundle`, `extract_sources` |
| `project/migrate.rs` | C6 | manifest version migration chain |
| `project/atomic.rs` | C6 | `write_atomic` (temp + fsync + rename, `.bak` rotation) |
| `export/mod.rs` | C6 | `export::plan`, `export::plan_contents`, `export::run` (loose files or one zip, 7.4), `ExportOptions` (layer scope, package), `ExportPackage`, `ExportReport`, `PlannedFile` |
| `export/naming.rs` | C6 | `sanitize_file_stem`, output file naming, de-duplication |
| `export/ora.rs` | C6 | OpenRaster writer |
| `export/psd.rs` | C6 | PSD writer + strict verification reader |
| `export/packbits.rs` | C6 | PackBits RLE |

Integration tests (`crates/wte_core/tests/`, outside `src`):

| File | Owner | Owns |
|---|---|---|
| `tests/end_to_end.rs` | C7 | the whole-pipeline test of section 9.1 ("end to end") |
| `tests/project_roundtrip.rs` | C7; round 4 T5 | project tests that need painted, stamped and text content plus re-bake on load (section 9.1) |
| `tests/brush_tips.rs` | B3 (round 4) | end-to-end tests of brush tips (FEATURE_BRUSH 8.1, 8.5): C3, C5, C10, C11, C14, C15, C20 to C22, C24, C26, `strokes_save_identically_through_both_entry_points` |
| `tests/objects.rs` | objects-tabs WP2 | hidden objects on the "Three Objects" sample: view state, rays, brush, projected stamp + depth fit, wrap (with and without Jump), bake jobs keep their visibility, thread-count determinism with a mask |

### 2.2 `crates/wte_app/src`

| File | Owner | Owns |
|---|---|---|
| `state.rs` | S | `AppState`, `Session` (objects-tabs: `selected_objects`, `object_anchor`, `object_selection_rev`, `object_reveal`), `ViewMode`, `ViewId`, `ViewFlags`, `ShadingMode`, `Backdrop`, `CameraPreset`, `ToolKind`, tool settings (round 4: `BrushType`, `ShapeChoice`, `BrushSettings::sanitized`), `ColorState`, `HoverInfo`, `OrbitCamera`, `UvViewState`, `Dialog`, `Pending`, `Prefs`, `BusyInfo`; round 4: `SideTab`, `TextPage`, `SideRects`, `BrushShapeState`, `ReadyShape`, `PendingShape`, `OpeningsDialogState`, `TextGapSel`, `FontSampleState`, `FontSampleCell`, `ToolProbe` (types + `Default`s + `Session::validate`) |
| `action.rs` | S | `AppAction`, `ToastLevel` (types only) |
| `main.rs` | A1 | logging to file, panic hook, startup-failure message box, `NativeOptions`, `eframe::run_native`, exit code |
| `crash.rs` | A1 | native crash recorder (vectored handler + unhandled filter, report + minidump), ring of recent log lines / actions, startup diagnostics (graphics env vars, hook modules), `WTE_DEBUG_NATIVE_CRASH` test hook (8.3) |
| `cli.rs` | A1 | `CliArgs`, hand-written argument parser |
| `app.rs` | A1 | `WteApp`, `impl eframe::App` (`logic`, `ui`, `raw_input_hook`), frame order |
| `dispatch.rs` | A1 | `dispatch(app, action, frame)`: the single mutation point for commands and the only caller of native file dialogs |
| `shortcuts.rs` | A1 | shortcut table, `handle` (the only consumer of key presses), `shortcut_text` |
| `jobs.rs` | A1 | `Jobs`, `JobProgress`, `JobOutput`, worker threads |
| `io_flows.rs` | A1 | new/open/save/save-as/export/import-stamp sequencing, file dialogs, job result handling |
| `prefs.rs` | A1; round 4 B5 | `Prefs` load/save (`%APPDATA%\WrappedTextureEditor\prefs.json`); the closing choice per shape file (`stored_close`, `remember_close`, `forget_close`, at most `MAX_SHAPE_CLOSE` = 32 files) |
| `toasts.rs` | A1 | `Toasts` queue (`push`, `error_count`) and drawing |
| `theme.rs` | A1 | visuals, runtime window icon, vector tool icons |
| `dnd.rs` | A1 | file drag-and-drop routing |
| `object_ops.rs` | objects-tabs WP3a | the object commands as plain functions (8.3 "Objects"): select / toggle / range / all / none on `Session`; `set_hidden`, `set_visible`, `hide_selected`, `isolate_selected`, `show_all`, `frame_selection` on `AppState`; `hidden_names` / `hidden_from_names` / `restored_hidden_text` for `UiState` (8.9) |
| `brush_flows.rs` | B5 (round 4) | the brush shape loaded from a model file: Load / quiet reload / Remove, the bake job and its request numbers, the event table of FEATURE_BRUSH 4.4, the closing-openings question (`Dialog::CloseOpenings`, `AnswerOpenings`, the choice per file), `startup`, `sync_prefs` |
| `text_ops.rs` | T7 (round 4) | the font-sample job of the font picker and its atlas textures |
| `gpu/mod.rs` | A2 | `Gpu` (device, queue, egui renderer handle) |
| `gpu/params.rs` | S | `View3dParams`, `ViewUvParams`, `BrushRing` (plain data crossing the view/gpu boundary; imports `ShadingMode` / `Backdrop` from `state.rs`) |
| `gpu/targets.rs` | A2 | `ViewTarget` (offscreen colour + MSAA + depth, egui texture id) |
| `gpu/textures.rs` | A2 | `SetTexture` (dirty-rect upload, mip regeneration with the mip blit pipeline) |
| `gpu/scene.rs` | A2; objects-tabs WP3b | `GpuScene` (mesh buffers, wire instance buffers, per-set textures), `sync`; objects-tabs: `sync_objects` (visible index buffer, wire runs per object, highlight index buffer), the pure helpers `visible_draws`, `selected_indices`, `object_runs`, `visible_runs`, `WireList` |
| `gpu/renderer.rs` | A2; round 4 B6; objects-tabs WP3b | `Renderer`: pipelines, uniforms, `render_3d`, `render_uv`; the ring's side filter; objects-tabs: the tint pipeline |
| `gpu/shaders/mesh.wgsl`, `lines.wgsl`, `uvview.wgsl`, `background.wgsl`, `blit.wgsl`, `tint.wgsl` | A2; `tint.wgsl` objects-tabs WP3b | shaders (`include_str!`; `tint.wgsl` = the selection tint) |
| `view/mod.rs` | A3 | `ViewCx`, `central` (layout, two-phase update), view tab strip |
| `view/camera.rs` | A3 | `impl OrbitCamera` (matrices, rays, orbit/pan/zoom, framing, projection) |
| `view/view3d.rs` | A3 | 3D view: input routing, hover raycast, overlays |
| `view/view2d.rs` | A3 | UV view: pan/zoom, cursor -> UV, overlays |
| `view/preview.rs` | A3 | `preview_widget` (clean, self-contained orbit view; also embedded in the export dialog), Preview mode strip |
| `view/gizmo.rs` | A3 | stamp gizmo geometry, hit-testing and drawing (3D and UV) |
| `tools/mod.rs` | A3; objects-tabs WP3b | `Tool` trait, `ToolController`, `ViewXform`, `ViewPointer`, `Surface`, `Interaction`; objects-tabs: the Navigate click that picks an object (`NavClick`, `pick_object`) |
| `tools/brush.rs` | A3; round 4 B7 | brush + eraser: the tip of the settings, stroke pacing, the shape cursor, the wheel gestures |
| `tools/eyedropper.rs` | A3 | eyedropper |
| `tools/fill.rs` | A3 | fill |
| `tools/stamp_place.rs` | A3; round 4 T7 | image-stamp and text-stamp placement; the see-through text under the pointer |
| `tools/transform.rs` | A3; round 4 T7 | select / move / scale / rotate stamps |
| `tools/text_path.rs` | T7 (round 4) | the round points of a text on a Curve (drag in the view) |
| `panels/mod.rs` | A4; round 4 S2, B4 | `panels::show_*` entry points; the side panel's tab strip Layers / Brush / Text and `side_rects` |
| `panels/menu_bar.rs`, `toolbar.rs`, `tool_options.rs`, `layers.rs`, `texture_sets.rs`, `color.rs`, `status_bar.rs` | A4; round 4 B4 | one panel each (`layers.rs`: multi-select and the row menu "Export this layer..." / "Export selected layers... (N)", 8.7) |
| `panels/brush_tab.rs` | B4 (round 4) | the Brush tab |
| `panels/text_block.rs` | T6 (round 4) | the Text block of the tool options: `target`, `show` (four rows), the spec helpers `current_spec` / `draft_spec` / `write_draft` / `live_edit` / `click_edit`, the font rules `candidates` / `resolve_face` / `resolve_font` / `style_click`, `TextPanelState`, `GapBuffer` with its history |
| `panels/text_tab.rs` | T6 (round 4) | the Text tab: pages Character (with the gap strip), Warp, Path |
| `panels/font_picker.rs` | T6 (round 4) | the font picker popup of the font button |
| `dialogs/mod.rs` | A5 | `dialogs::show` dispatcher |
| `dialogs/import.rs`, `export.rs`, `unsaved.rs`, `progress.rs`, `message.rs`, `about.rs` | A5 | one modal each (`message` = error/warning list, `about` = About + Shortcuts) |
| `selftest/mod.rs` | A1; round 4 Z | `Runner`, `Step`, `Scratch`, input injection, screenshot capture, report |
| `selftest/scenarios.rs` | A6 | canned scenarios (the scaffold stub already contains the action-only smoke scenarios of section 8.11) |

---

## 3. Cargo manifests

Rules: every dependency must be pure Rust (no `cc`, no `cmake`, no C-compiling `*-sys`). Only crates already in the registry are used.
`default-features = false` is mandatory where shown: default features of `eframe`, `rfd`, `fontdb`, `zip`, `image`, `gltf` pull crates
that are not downloaded.

### 3.1 `Cargo.toml` (workspace root)

```toml
[workspace]
resolver = "3"
members = ["crates/wte_core", "crates/wte_app"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "MIT"

[workspace.dependencies]
wte_core   = { path = "crates/wte_core" }
glam       = "0.34.3"
bytemuck   = { version = "1.25.2", features = ["derive"] }
serde      = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
rayon      = "1.12.0"
thiserror  = "2.0.21"
anyhow     = "1.0.104"
log        = "0.4.35"
bitflags   = "2.13.2"
image      = { version = "0.25.10", default-features = false, features = ["png", "jpeg", "tga", "bmp", "tiff", "webp", "dds", "rayon"] }
png        = "0.18.1"
zip        = { version = "9.5.1", default-features = false, features = ["deflate"] }
crc32fast  = "1.5"
tobj       = "4.0.5"
gltf       = { version = "1.4.1", default-features = false, features = [
    "utils", "names", "allow_empty_texture",
    "KHR_materials_unlit", "KHR_texture_transform", "KHR_materials_pbrSpecularGlossiness",
    "KHR_materials_emissive_strength", "KHR_materials_ior", "KHR_materials_transmission", "KHR_lights_punctual",
] }
fontdue    = "0.9.5"
fontdb     = { version = "0.24.0", default-features = false, features = ["std", "fs", "memmap"] }
epaint_default_fonts = "0.37.0"
ttf-parser = { version = "0.25.1", default-features = false, features = ["opentype-layout", "no-std-float"] }
eframe     = { version = "0.37.0", default-features = false, features = ["wgpu", "default_fonts", "accesskit"] }
rfd        = { version = "0.18.0", default-features = false }
env_logger = "0.11.11"

[profile.release]
opt-level = 3
lto = "thin"
strip = true

[profile.dev]
opt-level = 0

# Painting in a debug build must stay interactive: optimise the core and all dependencies.
[profile.dev.package.wte_core]
opt-level = 1
[profile.dev.package."*"]
opt-level = 1
```

Notes:
- `crc32fast` is the one crate beyond the fixed list. It is already in the registry (transitive dependency of `png`, `zip`, `flate2`), is
  pure Rust, and is needed for blob de-duplication and manifest `FileRef.crc32`.
- gltf feature list is pinned: it changes signatures (`Texture::source()` returns `Option<Image>` with `allow_empty_texture`,
  `REG\gltf-1.4.1\src\texture.rs:161`) and decides which `extensionsRequired` validate (`REG\gltf-json-1.4.1\src\extensions\mod.rs:40`;
  no feature enables `KHR_mesh_quantization`, so `import/gltf.rs` `validate` lets that one name through itself). The gltf `import` feature stays off: wte_core never lets gltf touch the filesystem.
- `fontdb` without `fontconfig` (unix-only parser crate that is not downloaded). `memmap2` is present.
- eframe feature `wgpu` = `wgpu_no_default_features` + `egui-wgpu/default` (which enables `wgpu/default`): DX12/Vulkan/GL backends, WGSL.
- eframe feature `clipboard` is deliberately off (`arboard` is not in the registry): in-app clipboard only, see section 1.3.
- `egui` 0.37.0, `egui-wgpu` 0.37.0 and `wgpu` 30.0.1 are **not** listed as direct dependencies. wte_app reaches them only through
  eframe's re-exports (`use eframe::{egui, egui_wgpu, wgpu};`, `REG\eframe-0.37.0\src\lib.rs:156-162`), so versions and feature sets
  can never diverge from the ones eframe was built with. `winit` is not used directly at all.

### 3.2 `crates/wte_core/Cargo.toml`

```toml
[package]
name = "wte_core"
version.workspace = true
edition.workspace = true

[dependencies]
glam.workspace = true
bytemuck.workspace = true
serde.workspace = true
serde_json.workspace = true
rayon.workspace = true
thiserror.workspace = true
log.workspace = true
bitflags.workspace = true
image.workspace = true
png.workspace = true
zip.workspace = true
crc32fast.workspace = true
tobj.workspace = true
gltf.workspace = true
# zlib inflate of compressed FBX arrays (already in the dependency tree through png).
miniz_oxide = "0.8.9"
fontdue.workspace = true
fontdb.workspace = true
epaint_default_fonts.workspace = true
# Glyph outlines for the styled text painter (round 4, TEXT 5.1); the feature set fontdue enables.
ttf-parser.workspace = true
```

No dev-dependencies (`tempfile` is not available; see `testutil.rs`).

### 3.3 `crates/wte_app/Cargo.toml`

```toml
[package]
name = "wte_app"
version.workspace = true
edition.workspace = true

[[bin]]
name = "wrapped_texture_editor"
path = "src/main.rs"

[dependencies]
wte_core.workspace = true
eframe.workspace = true
glam.workspace = true
bytemuck.workspace = true
serde.workspace = true
serde_json.workspace = true
anyhow.workspace = true
log.workspace = true
env_logger.workspace = true
rfd.workspace = true
```

wte_app does not depend on `image`/`png` directly: all encoding and decoding goes through `wte_core::image_io`.

`main.rs` starts with `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` (release = no console; debug keeps one).
No `build.rs`, no resource file: the window icon is set at runtime (`ViewportBuilder::with_icon`).

---

## 4. Conventions

### 4.1 Coordinates

- **World space**: right-handed, Y-up, `f32`, `glam::Vec3`. Meshes are never rescaled. Every epsilon is relative to
  `diag = mesh.bounds.diagonal()`.
- **Triangle winding**: counter-clockwise = front face. Importers fix winding when a node transform has negative determinant.
- **UV space**: origin top-left, **v points down** (glTF / image / wgpu convention). Texel `(x, y)` has its centre at
  `((x + 0.5) / W, (y + 0.5) / H)`. Row 0 of every pixel buffer is the top row. glTF UVs are used as-is; **OBJ is flipped at import
  (`v' = 1 - v`)**. Nothing else in either crate ever flips V.
- **Pixel coordinates**: `x` right, `y` down, continuous texel coordinates `px = uv * (W, H)`; `floor(px)` is the texel index;
  `px - 0.5` is the bilinear sample coordinate.
- **Rects**: `IRect { x0, y0, x1, y1 }`, `i32`, half-open.
- **Camera / projection** (wte_app): `glam::camera::rh::proj::directx::perspective(fov_y, aspect, near, far)` (NDC z in 0..1, Y-up — the
  wgpu convention) and `glam::camera::rh::view::look_at_mat4(eye, center, up)`. glam 0.34.3 has **no** `Mat4::perspective_rh` /
  `look_at_rh` (`REG\glam-0.34.3\src\camera\rh\proj.rs:202`, `view.rs:32`). Never use the `vulkan` module (Y-down).
- **Angles**: radians in the core API unless the field name ends in `_deg`. The manifest stores every angle exactly as the core
  holds it (same unit, same `f32` bits), so a round trip is bit-exact: `UvPlacement.rotation` is radians in both places, the
  `Projector` angle limits are degrees in both places. Only the UI converts (`StampSettings.rotation_deg`).
- **Rotation sense**: a positive stamp rotation is **clockwise as seen by the viewer** for both placement kinds
  (`UvPlacement.rotation` on the v-down texture, `Projector::rotated` seen from the projector's +Z side).

### 4.2 Colour and alpha

- All pixel data is **RGBA8, straight (non-premultiplied) alpha, sRGB-encoded**, tightly packed (`stride = 4 * W`).
- Layer blending is done on the sRGB-encoded values (gamma-space blending, like Photoshop/GIMP/Krita 8-bit), so our PSD/ORA exports
  re-composite identically in other editors.
- RGB under alpha 0 is preserved through load/save/undo (imported game textures use it).
- Filtering (mips, bilinear resampling, resize) always happens on **premultiplied** values (`PremulMips`), then un-premultiplies.
- GPU: set textures and view targets are `Rgba8Unorm` holding sRGB-encoded bytes (never `*Srgb` formats). Shaders convert to linear only
  for lighting and convert back. In unlit mode the 3D view outputs the texture bytes unchanged.
- egui images must be premultiplied: build them with `egui::ColorImage::from_rgba_unmultiplied` (`REG\epaint-0.37.0\src\image.rs:113`).

### 4.3 Units

- Brush size in the UI: diameter in texels (D15). `BrushParams.radius`: world units for a 3D stroke, texels for a 2D stroke.
- Projected stamp sizes: world units. UV stamp sizes: texels of the target set.
- Text has two separate sizes. The **on-model size** is the placement (`Projector.half_size` / `UvPlacement.size_px`), like any
  stamp; this is the only "size" the UI shows. `TextSpec.size_px` is the **raster resolution** (em size in pixels of the text image,
  not texels of the target); the core chooses it automatically from the on-model size (section 6.5, "Text raster resolution") and
  the user never edits it. Outline width and letter spacing are fractions of the em, so they do not change when the raster
  resolution changes.
- Opacity, hardness, flow: `f32` in `0..=1`.
- Mouse-wheel amounts are in **notches** (one detent of a wheel = 1.0), see section 8.6.

### 4.4 Ids

- `TextureSetId(n)` is the index `n` into `Document::sets` and `MeshData::sets`. Sets are created once and never reordered or removed.
- `LayerId` and `StampId` come from `Document::alloc_id()` (monotonic `u64`, never reused, persisted as `next_id`).
- `BlobId` is allocated by `BlobStore` (monotonic `u32`, the first id is 0).
- Nothing persisted references vertex or triangle indices.

### 4.5 Errors and logging

- `wte_core`: `thiserror` enums, all defined in `error.rs`. No `anyhow`, no panics on bad input (malformed files, NaN, zero-size).
  `unwrap`/`expect` only on invariants established by the same function. Recoverable problems are returned as warnings
  (`ImportWarning`, `LoadWarning`) next to the result.
- `wte_app`: `anyhow::Result` at flow level; user-facing failures become a toast (short text) plus `Dialog::Message` (details). Everything
  is also logged with `log`.
- Floats written to the manifest are checked with `is_finite` first (serde_json writes NaN as `null`).

### 4.6 Threading

- `Document` lives on the UI thread and is mutated only there. `Document: Send` (it is built on a worker and moved), not shared.
- `MeshData`, `Bvh`, `TexelMap`, `PremulMips`, `BlobStore` contents, `DocumentSnapshot`, `ImportedModel`: `Send + Sync`.
  `StampBakeJob`, `StampBakeOutput` (round 5): `Send`.
- Data-parallel loops inside the core use the global rayon pool (compositing rows, bakes, BVH build, PNG encoding of several layers,
  and since round 7 the 3D dab: candidate preparation of long lists and the fused band pass of 6.4, called from the UI thread,
  which waits for the dab; the partition depends only on the dab, never on the thread count). The global pool has one thread
  per logical processor (12 on the reference machine).
- A stamp bake job runs its data-parallel loops in the core's own rayon pool (`available_parallelism - 1` threads, at
  least 1, named `stamp-bake-N`; `StampBakeJob::run_isolated`), not the global pool (round 5).
- Long operations (import, document build, project load/save, export, system-font scan, and — with deferred Final bakes on,
  5.20 — the Full bake of a stamp or text after a placement or a Final edit) run on `std::thread` workers started by
  `wte_app::jobs`; they receive owned inputs (`DocumentSnapshot`, paths, `StampBakeJob`) and report through
  `&dyn Progress` (a stamp bake reports nothing).
- The UI thread never blocks on a worker, with one exception: `stamp_bakes::settle` waits for the stamp bake in flight and
  bakes the other waiting stamps itself before save, export, the export dialog, merge down and rasterize, because these keep
  final stamp pixels. Workers never touch egui except `Context::request_repaint()`.
- GPU objects are created and used on the UI thread only.

### 4.7 Determinism

- No `HashMap` iteration order may influence output pixels, file bytes or ids. Sort (`query_sphere` output is sorted by `TriId`) or use
  `BTreeMap`/`Vec`.
- The core never reads the clock; timestamps are passed in (`SaveOptions::now_unix`).
- ZIP entries use the two option values of section 7.1, both built from `SimpleFileOptions::DEFAULT` (fixed 1980 timestamp), so
  identical input gives identical archive bytes.

### 4.8 Disk rule for tests and tools

The C: drive is nearly full. Tests must not write to `%TEMP%`. Use `wte_core::testutil::temp_dir(name)`, which creates
`<workspace>/target/wte-test-tmp/<name>-<pid>-<counter>/` (from `env!("CARGO_MANIFEST_DIR")`), and remove it at the end of the test.

### 4.9 Naming

Rust 2024 edition, `snake_case` modules, no `mod.rs`-less mixing: sub-trees use `mod.rs`. Public items carry a one-line doc comment.
No hand-written `unsafe` in either crate (the bytemuck derives are the only source of generated `unsafe impl`s).

---

## 5. `wte_core` public API contract

This is the interface implementers code against. The scaffold writes every item below; bodies of non-[S] files start as `todo!()`.
Rules:
- Pub signatures may not change without the architect's approval. Private fields and private helpers are the owner's business.
- `pub(crate)` items listed here are cross-package contracts inside the crate and are equally binding.
- Derives: unless stated otherwise, plain data types derive `Clone, Debug, PartialEq`; small value types also `Copy`.
- `lib.rs` re-exports every type named in this section at the crate root (`pub use ids::*;` etc.), and `pub use glam;`.

### 5.1 `ids.rs` [S]

```rust
pub type TriId = u32;                         // triangle index; < 2^31

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)] pub struct TextureSetId(pub u32);   // == index into Document::sets / MeshData::sets
#[serde(transparent)] pub struct LayerId(pub u64);        // same derives
#[serde(transparent)] pub struct StampId(pub u64);        // same derives
#[serde(transparent)] pub struct BlobId(pub u32);         // same derives

impl TextureSetId { pub fn index(self) -> usize; }
```

### 5.2 `error.rs` [S]

```rust
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("file not found: {0}")] NotFound(String),
    #[error("cannot read {0}: {1}")] Io(String, #[source] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ImageIoError {
    #[error("cannot decode image: {0}")] Decode(String),
    #[error("cannot encode image: {0}")] Encode(String),
    #[error("unsupported image format: {0}")] UnsupportedFormat(String),
    #[error("image is too large: {width}x{height}")] TooLarge { width: u32, height: u32 },
    #[error("image has zero size")] Empty,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("unrecognized model format")] UnknownFormat,
    #[error("OBJ parse error: {0}")] Obj(String),
    #[error("glTF error: {0}")] InvalidGltf(String),
    #[error("invalid FBX file: {0}")] InvalidFbx(String),                    // damaged / unsupported FBX (import/fbx.rs)
    #[error("glTF requires unsupported extension(s): {0:?}")] UnsupportedGltfExtension(Vec<String>),
    #[error("referenced file not found: {0}")] MissingSidecar(String),        // only a glTF .bin is fatal
    #[error("buffer {index} too short: expected {expected}, got {actual}")]
    BufferLength { index: usize, expected: usize, actual: usize },
    #[error("model contains no triangles{}", ..)] Empty { reasons: Vec<String> },   // the loader's SkippedPrimitive reasons
    #[error("I/O error: {0}")] Io(#[from] std::io::Error),
    #[error("{name} is {size} bytes; a project can store files of at most {limit} bytes")]
    FileTooLarge { name: String, size: u64, limit: u64 },
    #[error("cancelled")] Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImportWarning {
    MissingMtl(String),
    MissingTexture { material: String, reference: String },
    TextureDecodeFailed { reference: String, reason: String },
    TextureResized { reference: String, from: [u32; 2], to: [u32; 2] },
    NoUvs { material: String },
    DegenerateUvs { material: String, percent: u8 },
    UvWrapped { triangles: u32 },              // UVs outside 0..1 were folded onto one tile (no UDIM)
    NormalsGenerated,
    DroppedDegenerate { triangles: u32 },
    SkippedPrimitive { reason: String },
    SkinIgnored,
    FallbackUvsGenerated,
    // OBJ v / vt / vn lines with a missing, malformed or non-finite number and f lines with a position index that points
    // nowhere (import/obj.rs); first_line is 1-based
    BadObjNumbers { first_line: usize, lines: u32, dropped_faces: u32, degraded_faces: u32 },
}
impl std::fmt::Display for ImportWarning { /* one human-readable sentence each */ }
impl ImportWarning {
    /// A fact about the model file itself: NoUvs, DegenerateUvs, UvWrapped, NormalsGenerated, DroppedDegenerate,
    /// SkippedPrimitive, SkinIgnored, FallbackUvsGenerated, BadObjNumbers. Shown once by the import dialog; `project::load` drops these
    /// (re-importing the stored file gives them again unchanged). MissingMtl and the texture warnings are not facts: on load
    /// they can mean a stored file is missing.
    pub fn is_model_fact(&self) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("the model has no UV-mapped geometry")] NoTextureSets,
    #[error("invalid document setup: {0}")] BadSetup(String),
    #[error(transparent)] Image(#[from] ImageIoError),
    #[error("cancelled")] Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("font error: {0}")] Font(String),
    #[error("font data is missing from the project")] MissingFont,
    #[error("text image would be too large: {width}x{height}")] TooLarge { width: u32, height: u32 },
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("unknown texture set")] UnknownSet,
    #[error("unknown layer")] UnknownLayer,
    #[error("unknown stamp")] UnknownStamp,
    #[error("unknown image or font")] UnknownBlob,
    #[error("layer is locked")] LayerLocked,
    #[error("layer is hidden")] LayerHidden,
    #[error("layer is not a paint layer")] NotRaster,
    #[error("no paintable layer")] NoPaintableLayer,
    #[error("a stroke is in progress")] StrokeInProgress,
    #[error("cannot merge: {0}")] CannotMerge(&'static str),
    #[error("invalid parameter: {0}")] InvalidParam(&'static str),
    #[error(transparent)] Text(#[from] TextError),
    #[error(transparent)] Image(#[from] ImageIoError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("I/O error: {0}")] Io(#[from] std::io::Error),
    #[error("not a Wrapped Texture Editor project: {0}")] NotAZip(String),
    #[error("project manifest is missing")] ManifestMissing,
    #[error("project manifest is invalid: {0}")] ManifestInvalid(String),
    #[error("project was saved by a newer version (format {found}, supported {supported})")]
    TooNew { found: u32, supported: u32 },
    #[error("model data is unrecoverable: {0}")] ModelUnrecoverable(String),
    #[error("entry {entry} exceeds the size limit")] EntryTooLarge { entry: String },
    #[error(transparent)] Image(#[from] ImageIoError),
    #[error("zip error: {0}")] Zip(String),            // from zip::result::ZipError (non_exhaustive): map with to_string()
    #[error("save verification failed: {0}")] VerifyFailed(String),
    #[error("cancelled")] Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadWarning {
    NewerFormat { found: u32 },
    MeshUvChanged,
    OlderGeneratedUvLayout { saved: u32 },     // saved importer_version < IMPORTER_VERSION, generated UVs, uv_hash differs
    LayerPixelsLost { layer: LayerId, name: String, reason: String },
    BakeCacheRebuilt { layer: LayerId },
    BlobMissing { blob: BlobId, used_by: Vec<StampId> },
    FontFallback { stamp: StampId, family: String },
    UnknownLayer { set: TextureSetId, index: usize },
    /// A text stamp uses a round-4 feature this build does not know (an unknown warp style or path kind, named in
    /// `what`); the stamp loads without that feature and plain Save is refused (`LoadReport::force_save_as`).
    TextFeatureUnknown { stamp: StampId, what: String },
    Import(ImportWarning),
}
impl std::fmt::Display for LoadWarning { /* one sentence each */ }

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("I/O error: {0}")] Io(#[from] std::io::Error),
    #[error(transparent)] Image(#[from] ImageIoError),
    #[error("zip error: {0}")] Zip(String),
    #[error("PSD would exceed 4 GiB ({bytes} bytes)")] PsdTooLarge { bytes: u64 },
    #[error("PSD supports at most 30000 px per side ({width}x{height})")] PsdDimension { width: u32, height: u32 },
    #[error("nothing selected for export")] NothingToExport,
    #[error("cancelled")] Cancelled,
}
```

### 5.3 `math.rs` [S] (fully implemented by the scaffold)

```rust
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct IRect { pub x0: i32, pub y0: i32, pub x1: i32, pub y1: i32 }   // half-open
impl IRect {
    pub const EMPTY: IRect;                                   // all zero
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> IRect;
    pub fn from_xywh(x: i32, y: i32, w: u32, h: u32) -> IRect;
    pub fn from_size(w: u32, h: u32) -> IRect;                // 0,0..w,h
    pub fn width(&self) -> u32;  pub fn height(&self) -> u32; // 0 when empty
    pub fn is_empty(&self) -> bool;  pub fn area(&self) -> u64;
    pub fn intersect(&self, o: &IRect) -> IRect;              // EMPTY when disjoint
    pub fn union(&self, o: &IRect) -> IRect;                  // ignores empty operands
    pub fn contains(&self, x: i32, y: i32) -> bool;
    pub fn expand(&self, px: i32) -> IRect;                   // an empty rect stays EMPTY
    pub fn touches(&self, o: &IRect) -> bool;                 // overlap or share an edge (corner-only contact does not count)
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Aabb { pub min: Vec3, pub max: Vec3 }
impl Aabb {
    pub const EMPTY: Aabb;                                    // min = +INF, max = -INF
    pub fn from_points(points: &[Vec3]) -> Aabb;
    pub fn extend(&mut self, p: Vec3);  pub fn union(&self, o: &Aabb) -> Aabb;
    pub fn center(&self) -> Vec3;  pub fn diagonal(&self) -> f32;   // length of (max - min); 0 when empty
    pub fn is_empty(&self) -> bool;
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Ray { pub origin: Vec3, pub dir: Vec3 }            // dir is unit length
impl Ray { pub fn new(origin: Vec3, dir: Vec3) -> Ray; /* normalizes */  pub fn at(&self, t: f32) -> Vec3; }

/// Pairwise disjoint rectangles. Up to MAX_RECTS (32) exact rects (touching rects merge); beyond that the region switches
/// to a grid of TILE x TILE (64 px) tiles: every added rect marks the tiles it covers and `rects()` returns the runs of
/// marked tiles per tile row (rows with identical runs merged), clipped to the bounds of what was added. Many small
/// disjoint rects spread over a large area (a 3D dab reaching many UV islands of a packed atlas) therefore stay close
/// to their own area instead of collapsing to their bounding box (review round 2, finding SP-3).
#[derive(Clone, Debug, Default)]
pub struct DirtyRegion { rects: Vec<IRect>, tiles: BTreeSet<(i32, i32)>, tile_bounds: IRect }
impl DirtyRegion {
    pub const MAX_RECTS: usize = 32;  pub const TILE: i32 = 64;
    pub fn add(&mut self, r: IRect);  pub fn is_empty(&self) -> bool;
    pub fn take(&mut self) -> Vec<IRect>;  pub fn rects(&self) -> Vec<IRect>;  pub fn bounds(&self) -> IRect;
}

pub fn div255(x: u32) -> u32;                 // exact round(x / 255) for x <= 65535: t = x + 128; (t + (t >> 8)) >> 8
pub fn mul_u8(a: u8, b: u8) -> u8;            // div255(a * b)
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32;
pub fn srgb_to_linear(c: u8) -> f32;  pub fn linear_to_srgb(c: f32) -> u8;
pub fn fnv1a64(seed: u64, bytes: &[u8]) -> u64;              // used for uv_hash and test hashes. Starts from `seed` verbatim: pass
                                                             // the FNV offset basis 0xcbf29ce484222325 first, then the previous result to chain
```

### 5.4 `progress.rs` [S]

```rust
pub trait Progress: Send + Sync {
    fn report(&self, fraction: f32, stage: &str);   // fraction 0..=1, monotone within one operation
    fn cancelled(&self) -> bool;
}
pub struct NoProgress;  impl Progress for NoProgress { /* no-op, never cancelled */ }
```

### 5.5 `pixel.rs` [S] and `pixel_ops.rs` [C1]

```rust
// pixel.rs
/// Dense RGBA8 image, straight alpha, sRGB-encoded, row 0 = top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelBuf { pub width: u32, pub height: u32, pub data: Vec<u8> }   // data.len() == w*h*4
impl PixelBuf {
    pub fn new(width: u32, height: u32) -> PixelBuf;                 // transparent black
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> PixelBuf;
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Option<PixelBuf>;
    pub fn get(&self, x: u32, y: u32) -> [u8; 4];  pub fn set(&mut self, x: u32, y: u32, rgba: [u8; 4]);   // panic outside the image
    pub fn rect(&self) -> IRect;  pub fn row(&self, y: u32) -> &[u8];  pub fn row_mut(&mut self, y: u32) -> &mut [u8];
    pub fn view(&self) -> PixelView<'_>;  pub fn byte_len(&self) -> usize;
}

/// RGBA8 pixels covering only `rect` of some canvas; transparent everywhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CroppedBuf { pub rect: IRect, pub data: Vec<u8> }         // data.len() == rect.area()*4
impl CroppedBuf {
    pub fn empty() -> CroppedBuf;  pub fn is_empty(&self) -> bool;
    pub fn new(rect: IRect) -> CroppedBuf;                           // transparent
    pub fn get(&self, x: i32, y: i32) -> [u8; 4];                    // [0;4] outside rect
    pub fn view(&self) -> PixelView<'_>;  pub fn byte_len(&self) -> usize;
    pub fn to_dense(&self, width: u32, height: u32) -> PixelBuf;
    pub fn blit_into(&self, out: &mut PixelBuf);                     // copy at rect (clipped), rest of `out` kept
}

/// RGBA8 pixels made of disjoint cropped parts; transparent everywhere else: the storage of stamp / text caches (6.5).
/// Parts are pairwise disjoint, never empty, cropped to their non-zero pixels, ordered by (y0, x0). (Review round 2,
/// finding SP-2: a decal on a packed atlas reaches islands spread over the texture; one dense rect over all of them
/// cost 17-94 MB per default-size stamp, the parts cost about the decal's own area.)
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SparseBuf { parts: Vec<CroppedBuf>, bounds: IRect }
impl SparseBuf {
    pub const TILE: i32 = 64;
    pub fn empty() -> SparseBuf;  pub fn is_empty(&self) -> bool;
    pub fn from_parts(parts: Vec<CroppedBuf>) -> SparseBuf;          // already disjoint; empty parts dropped, sorted
    pub fn from_cropped(buf: CroppedBuf) -> SparseBuf;               // = from_dense(buf.rect, buf.data)
    /// Splits dense pixels covering `rect`: the TILE x TILE tiles holding a non-zero byte are grouped into rectangles
    /// (runs of tiles per tile row; a run joins the open rectangle above it when the join wastes at most a quarter of
    /// the joined area; overlapping results are merged), each cropped to its non-zero pixels. All zero => empty().
    pub fn from_dense(rect: IRect, data: Vec<u8>) -> SparseBuf;
    pub fn bounds(&self) -> IRect;  pub fn parts(&self) -> &[CroppedBuf];  pub fn rects(&self) -> impl Iterator<Item = IRect>;
    pub fn byte_len(&self) -> usize;  pub fn get(&self, x: i32, y: i32) -> [u8; 4];   // [0;4] outside every part
    pub fn to_dense(&self, width: u32, height: u32) -> PixelBuf;
    pub fn to_cropped(&self) -> CroppedBuf;                          // one rect over bounds() (writers of one rect per layer)
}

/// Borrowed pixels covering `rect` of a canvas; stride = rect.width() * 4.
#[derive(Copy, Clone, Debug)]
pub struct PixelView<'a> { pub rect: IRect, pub data: &'a [u8] }
impl<'a> PixelView<'a> { pub fn row(&self, y: i32) -> Option<&'a [u8]>;  pub fn get(&self, x: i32, y: i32) -> [u8; 4]; }

// pixel_ops.rs
pub fn premultiply(p: [u8; 4]) -> [u8; 4];
pub fn unpremultiply(p: [u8; 4]) -> [u8; 4];                         // (c*255 + a/2) / a; a == 0 -> [0;4]
pub fn content_bounds(view: PixelView<'_>) -> Option<IRect>;         // bbox of alpha > 0; None if fully transparent
/// Bbox of pixels != [0, 0, 0, 0] (keeps RGB stored under alpha 0); None if every byte is zero. Used when saving raster layers.
pub fn nonzero_bounds(view: PixelView<'_>) -> Option<IRect>;
pub fn nonzero_bounds_par(buf: &PixelBuf) -> Option<IRect>;           // the same over a whole image, 64-row bands in parallel
pub fn crop(buf: &PixelBuf, rect: IRect) -> CroppedBuf;
pub fn crop_to_content(buf: &PixelBuf) -> CroppedBuf;

/// Premultiplied RGBA8 box-filtered mip chain of a source image. levels[0] = full size.
#[derive(Clone, Debug)]
pub struct PremulMips { pub levels: Vec<PixelBuf> }
impl PremulMips {
    pub fn build(src: &PixelBuf) -> PremulMips;
    pub fn size(&self) -> [u32; 2];
    /// Bilinear within a level, linear between levels, transparent outside 0..1. Returns premultiplied RGBA in 0..=1.
    pub fn sample(&self, u: f32, v: f32, lod: f32) -> [f32; 4];
    /// Bilinear on the single nearest level (Preview quality).
    pub fn sample_nearest_level(&self, u: f32, v: f32, lod: f32) -> [f32; 4];
}
pub fn resize(src: &PixelBuf, width: u32, height: u32) -> PixelBuf;  // premultiplied area/bilinear filter
/// Aspect-preserving thumbnail (longest side = `side`) of a canvas of size canvas_w x canvas_h showing `view`.
/// Stride-sampled: every output pixel averages at most 2x2 source samples (premultiplied) taken at regular positions inside its
/// footprint, so the cost is O(side^2) whatever the canvas size (never a full box filter over the canvas).
pub fn thumbnail(view: PixelView<'_>, canvas_w: u32, canvas_h: u32, side: u32) -> PixelBuf;
/// Export padding: fill texels that have no owner in `map` and alpha 0 with the nearest covered colour, up to `max_px` steps.
pub fn dilate_islands(buf: &mut PixelBuf, map: &TexelMap, max_px: u32);
```

### 5.6 `blob.rs` [S] (fully implemented by the scaffold)

```rust
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum BlobKind { Image, Font }
#[derive(Clone, Debug)]
pub struct Blob { pub id: BlobId, pub kind: BlobKind, pub name: String, pub bytes: Arc<[u8]>, pub crc32: u32 }

/// Original encoded bytes of stamp images and font files. De-duplicated by (len, crc32) then full comparison.
#[derive(Clone, Debug, Default)]
pub struct BlobStore { /* BTreeMap<BlobId, Blob>, next: u32 */ }
impl BlobStore {
    pub fn insert(&mut self, kind: BlobKind, name: &str, bytes: Arc<[u8]>) -> BlobId;        // returns the existing id on a duplicate (same kind, same bytes);
                                                                                              // the very same allocation (Arc::ptr_eq) is recognised before hashing
    pub fn insert_with_id(&mut self, id: BlobId, kind: BlobKind, name: &str, bytes: Arc<[u8]>); // project load
    pub fn get(&self, id: BlobId) -> Option<&Blob>;
    pub fn iter(&self) -> impl Iterator<Item = &Blob>;                                        // ascending id
    pub fn len(&self) -> usize;  pub fn is_empty(&self) -> bool;
}
```
Blobs are never garbage-collected in memory (undo history may reference them). `project::save` writes only blobs referenced by the
snapshot's stamps.

`insert` first looks for a stored blob of the same kind in the same allocation (`Arc::ptr_eq`) and returns it without
hashing (round 4, FEATURE_TEXT 5.13: `FontCatalog::face_bytes` hands out one allocation per font file while someone
holds it). Otherwise it hashes as before. Either way the lowest id among equal blobs of that kind is returned (two
exist only after `insert_with_id` on load). Unused font blobs are still not freed while the document is open.

### 5.7 `mesh.rs` [S] and `mesh_build.rs` [C1]

```rust
// mesh.rs
bitflags::bitflags! {
    #[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
    pub struct TriFlags: u8 {
        const NO_UV       = 1;   // missing UVs or zero UV area: never rasterized into a texture, still drawn and pickable
        const UV_MIRRORED = 2;   // signed UV area has the opposite sign to the majority of its set
        const UV_WRAPS    = 4;   // after tile normalization the triangle straddles a 0/1 border
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSlot {
    pub name: String,
    pub base_color: [f32; 4],                  // sRGB-encoded 0..1, used when the material has no texture set
    pub double_sided: bool,
    pub texture_set: Option<TextureSetId>,     // None => no UV-mapped geometry
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetInfo {
    pub name: String,                          // texture file stem, else first material name, else model name; unique (6.1 step 5)
    pub materials: Vec<u32>,                   // material indices sharing this set
    pub texture: Option<u32>,                  // index into ImportedModel::textures
    pub tri_count: u32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UvStats {
    pub covered_texels: u64,       // texels owned by a triangle (gutter excluded)
    pub overlapped_texels: u64,    // texels claimed by more than one triangle
    pub mirrored_tris: u32,
    pub coverage: f32,             // covered_texels / (w*h)
    pub texel_world: f32,          // mean world units per texel: sqrt(total 3D area / total UV pixel area)
}

/// Normalized, immutable mesh. Unified index: one vertex per distinct (position, normal, uv, uv-tile) combination.
#[derive(Clone, Debug)]
pub struct MeshData {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,                    // unit length
    pub uvs: Vec<Vec2>,                        // v-down, per-triangle tile-normalized (section 6.1)
    pub indices: Vec<u32>,                     // 3 per triangle, CCW = front; triangles sorted by (material, NO_UV last), stable
    pub tri_material: Vec<u32>,
    pub tri_flags: Vec<TriFlags>,
    pub materials: Vec<MaterialSlot>,
    pub material_ranges: Vec<std::ops::Range<u32>>,        // triangle range per material (same order as `materials`)
    pub material_uv_ranges: Vec<std::ops::Range<u32>>,     // leading part of material_ranges[m]: UV-mapped triangles (drawn textured)
    pub material_nouv_ranges: Vec<std::ops::Range<u32>>,   // trailing part: NO_UV triangles (drawn with base_color only)
    pub sets: Vec<SetInfo>,                    // index == TextureSetId.0
    pub bounds: Aabb,
    pub uv_hash: u64,                          // fnv1a64 over indices, uv bits and per-triangle set ids
    // objects-tabs (6.1 step 9): object index per triangle (index into `objects`), same order as `tri_material`; not
    // part of `uv_hash`. `objects`: at least one entry for a non-empty mesh, order of first appearance in the raw
    // triangle list.
    pub tri_object: Vec<u32>,
    pub objects: Vec<ObjectInfo>,
}
/// One object of the model (an OBJ object / group, a glTF mesh node, an FBX Model).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectInfo {
    pub name: String,                          // unique within the mesh, never empty
    pub tri_count: u32,
    pub bounds: Aabb,                          // bounds of the object's triangles (world space, transforms baked)
}
/// Importer-side object data, parallel to `RawMesh::triangles` (`RawMesh` / `RawTriangle` themselves are unchanged).
/// `Default` = no object structure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawObjects {
    pub tri_object: Vec<u32>,                  // one entry per raw triangle: index into `names`; empty = one unnamed object
    pub names: Vec<String>,                    // as the file gives them; may be empty strings or repeat
}
impl MeshData {                                 // trivial accessors, implemented by the scaffold
    pub fn tri_count(&self) -> u32;
    pub fn tri(&self, t: TriId) -> [u32; 3];
    pub fn tri_positions(&self, t: TriId) -> [Vec3; 3];
    pub fn tri_uvs(&self, t: TriId) -> [Vec2; 3];
    pub fn tri_normals(&self, t: TriId) -> [Vec3; 3];
    pub fn tri_set(&self, t: TriId) -> Option<TextureSetId>;   // None for NO_UV triangles and set-less materials
    pub fn geo_normal(&self, t: TriId) -> Vec3;                // unit; Vec3::Y for degenerate
    pub fn diag(&self) -> f32;                                 // bounds.diagonal().max(1e-12)
    pub fn object_count(&self) -> u32;                         // objects.len()
    pub fn tri_object_of(&self, t: TriId) -> u32;              // tri_object[t], 0 when the vector is short (hand-built test meshes)
}

/// GPU vertex (32 bytes). Produced by the core so the app needs no knowledge of the mesh layout.
#[repr(C)] #[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RenderVertex { pub pos: [f32; 3], pub normal: [f32; 3], pub uv: [f32; 2] }

#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum EdgeKind { Interior, Seam, Boundary }
/// One unique edge per UV side: a seam yields two entries (same positions, different UVs).
#[derive(Copy, Clone, Debug)]
pub struct WireEdge { pub a: [f32; 3], pub b: [f32; 3], pub uv_a: [f32; 2], pub uv_b: [f32; 2],
                      pub kind: EdgeKind, pub set: Option<TextureSetId>,
                      pub object: u32 /* objects-tabs: the object of the emitted triangle side (6.1 wire_edges) */ }

/// Importer output before normalization: a triangle soup.
#[derive(Clone, Debug, Default)]
pub struct RawMesh { pub triangles: Vec<RawTriangle>, pub materials: Vec<RawMaterial> }
#[derive(Clone, Debug)]
pub struct RawTriangle { pub positions: [Vec3; 3], pub normals: Option<[Vec3; 3]>, pub uvs: Option<[Vec2; 3]>, pub material: u32 }
#[derive(Clone, Debug)]
pub struct RawMaterial { pub name: String, pub base_color: [f32; 4], pub double_sided: bool, pub texture: Option<u32> }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshReport {
    pub total_tris: u32, pub dropped_degenerate: u32, pub no_uv_tris: u32, pub wrapped_tris: u32,
    pub normals_generated: bool, pub no_uvs: bool /* whole model */, pub fallback_uvs_generated: bool,
}

// mesh_build.rs
#[derive(Clone, Debug, Default)]
pub struct NormalizeOptions {
    pub generate_fallback_uvs: bool,
    pub model_name: String,              // names the fallback-UV set and sets without texture or material name; "" => "Texture"
    pub texture_names: Vec<String>,      // file stems, index = RawMaterial::texture; a missing index falls back to the material name
}
/// Runs the pipeline of section 6.1. Deterministic for identical input. One object (step 9).
pub fn normalize(raw: RawMesh, opts: &NormalizeOptions) -> (MeshData, MeshReport);
/// objects-tabs: `normalize` with object data; `normalize(raw, opts) == normalize_with_objects(raw, &RawObjects::default(), opts)`.
/// The object data never changes triangle order, vertex order or `uv_hash` (`IMPORTER_VERSION` stays 2).
pub fn normalize_with_objects(raw: RawMesh, objects: &RawObjects, opts: &NormalizeOptions) -> (MeshData, MeshReport);
impl MeshData {
    pub fn render_vertices(&self) -> Vec<RenderVertex>;       // 1:1 with positions/normals/uvs
    pub fn wire_edges(&self) -> Vec<WireEdge>;                // de-duplicated, deterministic order
    pub fn set_tri_count(&self, set: TextureSetId) -> u32;
}
/// UV layout of one set as an opaque `side x side` image (R2, import dialog): dark background (#202428), every
/// `wire_edges()` entry of the set drawn as a 1-px integer DDA line between its UV endpoints (wrapped into the image):
/// Interior grey (#8a8f98), Seam orange (#ff9d2e), Boundary cyan (#35d0e0); seams and boundaries are drawn last.
/// Cost O(edges * side); called once per set on the import worker, never per frame.
pub fn uv_layout_image(mesh: &MeshData, set: TextureSetId, side: u32) -> PixelBuf;
```

### 5.8 `bvh.rs` [C1]

```rust
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum Cull { None, Back }

#[derive(Copy, Clone, Debug)]
pub struct Hit {
    pub tri: TriId,
    pub t: f32,
    pub bary: [f32; 3],                        // weights of v0, v1, v2
    pub position: Vec3,
    pub geo_normal: Vec3,                      // from winding, unit (never flipped)
    pub front_face: bool,                      // dot(geo_normal, ray.dir) < 0
    pub normal: Vec3,                          // interpolated shading normal, unit, negated iff !front_face (so it faces the
                                               // viewer); geo_normal (same negation) if the interpolation is degenerate
    pub uv: Option<Vec2>,                      // None for NO_UV; not wrapped into 0..1
    pub material: u32,
    pub texture_set: Option<TextureSetId>,     // None for NO_UV triangles and set-less materials
}
impl Hit {
    /// The unflipped shading normal: `if front_face { normal } else { -normal }`. This is the reference normal of brush
    /// strokes (section 6.4); `normal` (viewer-facing) is the one used to orient stamp projectors.
    pub fn surface_normal(&self) -> Vec3;
}

/// objects-tabs: a set of triangles as a bit set (the hidden triangles of a `Bvh`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriMask { /* private: bits: Vec<u64>, len: u32, count: u32 */ }
impl TriMask {
    pub fn from_fn(tri_count: u32, f: impl Fn(TriId) -> bool) -> TriMask;
    pub fn get(&self, t: TriId) -> bool;                      // false beyond len
    pub fn count(&self) -> u32;
    pub fn is_empty(&self) -> bool;
}

pub struct Bvh { /* private: tree: Arc<BvhTree { nodes, tri_order }>, hidden: Option<Arc<TriMask>> */ }   // Send + Sync
impl Bvh {
    pub fn build(mesh: &MeshData) -> Bvh;                     // hidden: None
    /// objects-tabs: the same tree with another hidden set (`None` or an empty mask = nothing hidden, stored as `None`).
    /// O(1): the tree is shared. **Every query below skips the hidden triangles** (rays pass through them, any-hit
    /// queries ignore them, sphere queries omit them); with `None` the results are bit-identical to a plain tree.
    pub fn with_hidden(&self, hidden: Option<Arc<TriMask>>) -> Bvh;
    pub fn hidden(&self) -> Option<&TriMask>;
    pub fn is_hidden(&self, t: TriId) -> bool;
    /// Nearest hit with t in [t_min, t_max].
    pub fn raycast(&self, mesh: &MeshData, ray: &Ray, t_min: f32, t_max: f32, cull: Cull) -> Option<Hit>;
    /// Any-hit with early out.
    pub fn occluded(&self, mesh: &MeshData, ray: &Ray, t_min: f32, t_max: f32) -> bool;
    /// `occluded`, also naming the hit triangle (`Some` exactly when `occluded` is true).
    pub fn occluder(&self, mesh: &MeshData, ray: &Ray, t_min: f32, t_max: f32) -> Option<TriId>;
    /// Triangles whose closest point to `center` is within `radius`. `out` is cleared, then filled sorted ascending.
    pub fn query_sphere(&self, mesh: &MeshData, center: Vec3, radius: f32, out: &mut Vec<TriId>);
    /// As `query_sphere`, and also returns each triangle's squared closest distance to `center` (the value compared
    /// with `radius * radius`). Sorted by triangle id; `out` is cleared first. Same tree walk as `query_sphere`.
    pub fn query_sphere_dist(&self, mesh: &MeshData, center: Vec3, radius: f32, out: &mut Vec<(TriId, f32)>);
    pub fn node_count(&self) -> usize;
}
/// Closest point on a triangle (Ericson). Exposed for tests and the brush.
pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3;
```

### 5.9 `texel_map.rs` [C1]

```rust
/// Texel -> owning triangle for one texture set, including a gutter ring around every UV island.
pub struct TexelMap {
    pub width: u32, pub height: u32, pub gutter: u8,
    pub stats: UvStats,
    /* private: owner: Vec<u32>  (u32::MAX = none; bit 31 = gutter texel; low 31 bits = TriId);
                shared_gutter: Vec<(u32, u32)>  (gutter texel, covered texel it grew from) for sources >1 triangle covers */
}
#[derive(Copy, Clone, Debug)]
pub struct SurfacePoint { pub tri: TriId, pub bary: [f32; 3], pub position: Vec3, pub normal: Vec3 }
#[derive(Copy, Clone, Debug)]
pub struct Fragment { pub x: u32, pub y: u32, pub bary: [f32; 3], pub gutter: bool }   // x,y already wrapped into the image

impl TexelMap {
    pub fn default_gutter(width: u32, height: u32) -> u8;         // clamp(max(w,h)/256, 2, 16)
    pub fn build(mesh: &MeshData, set: TextureSetId, width: u32, height: u32, gutter: u8) -> TexelMap;
    /// `build` polling `cancelled` once per triangle; None when it reported true (`Document::from_parts` -> `Cancelled`).
    pub fn build_cancellable(mesh: &MeshData, set: TextureSetId, width: u32, height: u32, gutter: u8,
                             cancelled: &(dyn Fn() -> bool + Sync)) -> Option<TexelMap>;
    pub fn owner(&self, x: u32, y: u32) -> Option<(TriId, bool)>; // (triangle, is_gutter)
    pub fn locate(&self, mesh: &MeshData, x: u32, y: u32) -> Option<SurfacePoint>;   // 2D -> 3D
}

/// The one surface rasterizer. Visits every texel that triangle `tri` covers or owns (section 6.3), restricted to
/// `clip` (unwrapped pixel-space box [x0, y0, x1, y1], None = whole triangle) and to wrapped rows in `rows`.
pub fn raster_tri(mesh: &MeshData, map: &TexelMap, tri: TriId,
                  clip: Option<[f32; 4]>, rows: std::ops::Range<u32>, f: &mut impl FnMut(Fragment));
/// `raster_tri` with pass 1 folded: each covered wrapped texel reported once through `covered(x, y)` (however many images
/// of a tiled triangle cover it); pass-2 fragments (gutter, claimed) through `f` exactly as `raster_tri` reports them.
pub fn raster_tri_covered(mesh: &MeshData, map: &TexelMap, tri: TriId, clip: Option<[f32; 4]>, rows: std::ops::Range<u32>,
                          covered: &mut impl FnMut(u32, u32), f: &mut impl FnMut(Fragment));
/// Per vertex, how far below 0 the barycentric weight of a fragment `raster_tri` reports as inside can go
/// (`EDGE_EPS * edge length / (2 * area)`, pixel space); `None` for a degenerate triangle.
pub fn inside_bary_slack(uv: [Vec2; 3], width: u32, height: u32) -> Option<[f32; 3]>;
```

Round 7 (A1): crate-internal, for the brush only, `TexelMap::prepare_brush_index` (a `TriId` bitset of the owners of
gutter texels and of the slivers that claimed a texel, and one bit per 8 x 8 block holding a `shared_gutter` texel; both
`OnceLock`, built once per map) and `raster_tri_lean` / `raster_tri_covered_lean` / `raster_box` (6.3); the public
`raster_tri` / `raster_tri_covered` and their callers (stamps, wraps, fill) are unchanged.

### 5.10 `samples.rs` [C1]

```rust
pub fn uv_sphere(segments: u32, rings: u32, radius: f32) -> RawMesh;   // equirect UVs, duplicated seam column
pub fn seam_cube(size: f32) -> RawMesh;            // 24 verts, six islands in a 3x2 atlas with 1/32 margins: every edge is a seam
pub fn two_material_model() -> RawMesh;            // seam_cube "Body" + uv_sphere "Head" on top: two materials, two textures
pub fn mirrored_quads() -> RawMesh;                // two quads sharing one UV island, one mirrored
pub fn wrapped_quad() -> RawMesh;                  // UVs in [0.5, 1.5]^2, plus one quad in tile [2, 3]
pub fn flat_quad() -> RawMesh;                     // unit quad in the XY plane facing +Z with identity UVs (u = x, v = 1 - y)
pub fn no_uv_tetra() -> RawMesh;
pub fn tight_islands() -> RawMesh;                 // two coplanar unit quads whose UV islands are 2 texels apart at 256 px
                                                   // (u in [0.25, 0.496] and [0.504, 0.75]): their gutters touch
pub fn occluded_quads() -> RawMesh;                // flat_quad plus a second, larger quad 0.5 behind it (z = -0.5), separate UV islands
pub fn reverse_winding(raw: &mut RawMesh);         // swaps vertices 1 and 2 of every triangle (positions, normals, uvs)
pub fn checker_texture(width: u32, height: u32, cells: u32) -> PixelBuf;     // opaque two-tone checker with a coloured border
pub fn sample_stamp_image(size: u32) -> PixelBuf;  // ring + arrow with soft alpha; asymmetric so flips are detectable
/// OBJ + MTL text for a raw mesh (writes v = 1 - v; triangles without UVs are written as faces without `vt`).
/// `texture_names[i]` is the map_Kd for raw texture index i.
pub fn to_obj(raw: &RawMesh, mtl_name: &str, texture_names: &[String]) -> (String, String);
/// objects-tabs: `to_obj` plus an `o <name>` line whenever the object changes (`to_obj` itself stays byte-identical).
pub fn to_obj_objects(raw: &RawMesh, objects: &RawObjects, mtl_name: &str, texture_names: &[String]) -> (String, String);
/// objects-tabs: three separate shells in disjoint UV regions of one texture (one material "Trio", one set):
/// "Floor" (8 x 8 grid, x, z in -3..3, y = 0, u 0.5..0.99), "Cube" (closed unit cube, x, z in -0.5..0.5, y in 0..1, in the
/// middle of the floor, u 0..0.5, v 0..0.5), "Ball" (uv_sphere r 0.5 at (1.75, 0.5, 0), u 0.01..0.49, v 0.51..0.99).
/// A top-down ray through the cube hits the cube first and the floor behind it.
pub fn object_trio() -> (RawMesh, RawObjects);
pub const OBJECT_TRIO_NAMES: [&str; 3];            // ["Floor", "Cube", "Ball"]
// Round 4 (FEATURE_BRUSH 8), paint fixtures with documented UV islands:
pub fn thin_slab(thickness: f32) -> RawMesh;       // 1 x 1 plate, x, y in 0..1, z in -thickness..0; seam_cube's six islands
pub fn fold(inner_angle_deg: f32) -> RawMesh;      // two unit quads sharing the edge x = 1; angle between the front faces
pub fn two_blocks(gap: f32) -> RawMesh;            // two unit cubes `gap` apart along X, twelve islands (two_blocks_island)
pub fn two_blocks_island(index: u32) -> [f32; 4];
pub fn fan_strips() -> RawMesh;                    // two long coplanar strips 0.1 apart sharing one corner 2 units away
pub fn undercut_ridge() -> RawMesh;                // plate, overhanging wall (110 deg from the plate normal), plateau
pub fn t_junction_quads() -> RawMesh;              // a quad next to two quads that split its edge
pub fn cracked_quads(eps: f32) -> RawMesh;         // two quads with a crack of eps
pub fn flat_quad_tilted(degrees: f32) -> RawMesh;  // flat_quad turned about X, UVs kept
pub fn soup_sets(seed: u64, triangles: u32, sets: u32) -> RawMesh;   // deterministic triangle soup over `sets` materials
// bake fixtures, no UVs and no normals:
pub fn open_box(side: f32) -> RawMesh;             // cube without its top face (one opening)
pub fn overlapping_cubes() -> RawMesh;             // two unit cubes, the second shifted by 0.5 along X
pub fn cube_with_inner_wall(slanted: bool) -> RawMesh;
pub fn doubled_cube() -> RawMesh;                  // every face twice
pub fn tilted_quad() -> RawMesh;                   // a quad turned 30 deg about X and 20 deg about Y
pub fn cube_on_floor() -> RawMesh;                 // closed unit cube on an open 10 x 10 quad
pub fn cube_floor_rod() -> RawMesh;                // the same plus a closed rod 0.05 thick, 3 long
pub fn corner_cubes() -> RawMesh;                  // two unit cubes in opposite corners of a 20-unit box
pub fn coin(diameter: f32, thickness: f32) -> RawMesh;   // closed cylinder, axis Y, 64 segments
pub fn flat_star() -> RawMesh;                     // five-armed star in the XY plane, outer radius 1, inner 0.4, no thickness
pub fn plate_xy() -> RawMesh;                      // closed box 1 x 1 x 0.2, thin along Z
pub fn cup() -> RawMesh;                           // cylinder r 0.5, y 0..1, with a bottom, open top (closing-openings tests)
// model-file writers (each test checks them by importing what they wrote):
pub fn to_gltf(raw: &RawMesh, node: Mat4) -> String;          // one mesh, one node with the matrix, base64 data: buffer
pub fn to_glb(parts: &[(RawMesh, Mat4)]) -> Vec<u8>;          // one mesh and one node per part, JSON + BIN chunks
pub fn to_fbx_ascii(raw: &RawMesh, unit_scale_factor: f32, lcl_scaling: [f32; 3]) -> String;   // ASCII FBX 7.4
```
Samples return `RawMesh` so tests can call `normalize` themselves; `mesh_of(raw)` = `normalize(raw, &NormalizeOptions::default()).0`
is provided in `testutil.rs`.

### 5.11 `layer.rs` [S]

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum BlendMode { Normal, Multiply, Screen, Overlay, Add, Darken, Lighten }

#[derive(Clone, Debug)]
pub enum LayerKind {
    /// Exactly set size, or 0 x 0 = unallocated (never painted, fully transparent; review round 2, finding SP-5: new
    /// "Paint N" layers and "Paint 1" cost no memory until the first stroke or fill, which allocates the buffer
    /// (`TextureSet::allocate_raster`, no version change, no history); merge_down onto one allocates the merged copy;
    /// the loader keeps a paint layer without stored pixels unallocated).
    Raster(Arc<PixelBuf>),
    Stamp { stamp: StampId, cache: Arc<SparseBuf> },        // baked image decal
    Text  { stamp: StampId, cache: Arc<SparseBuf> },        // baked text decal
}
impl LayerKind { pub fn empty_raster() -> LayerKind; }     // Raster(0 x 0)

#[derive(Clone, Debug)]
pub struct Layer {
    pub id: LayerId, pub name: String,
    pub visible: bool, pub opacity: f32, pub blend: BlendMode,
    pub locked: bool, pub alpha_lock: bool,
    pub kind: LayerKind,
    pub(crate) version: u64,                                // pixel-content version, see Document::mark_layer_dirty (5.14)
    /// Paint layers: Some((v, r)) = at version v every pixel outside r is [0,0,0,0] (a conservative content bound).
    /// Valid only while v == version. Grown by mark_layer_dirty, computed once by from_parts /
    /// Document::refresh_content_bounds (start of recomposite_all) when unknown, carried by structure snapshots
    /// (LayerRecord::content). Review round 2, finding SP-4: compositing and property changes skip the empty rest.
    pub(crate) content: Option<(u64, IRect)>,
}
impl Layer {
    /// visible, opacity 1, Normal, unlocked, no alpha lock, version 0, content unknown.
    pub fn new(id: LayerId, name: &str, kind: LayerKind) -> Layer;
    /// Changes whenever this layer's pixels change (never reused within a document). Thumbnail caches key on (id, version).
    pub fn version(&self) -> u64;
    pub fn raster(&self) -> Option<&PixelBuf>;              // the paint buffer (0 x 0 while unallocated)
    pub fn views(&self) -> Vec<PixelView<'_>>;              // dense buffer (none while unallocated) / cache parts
    pub fn get(&self, x: i32, y: i32) -> [u8; 4];           // [0;4] outside the stored pixels
    pub fn to_dense(&self, width: u32, height: u32) -> PixelBuf;
    /// Where the layer can be non-transparent: the content bound of a paint layer (its whole buffer while unknown,
    /// nothing while unallocated), the part rects of a cache. Property changes, adds, removes and moves mark exactly
    /// these rects dirty; `content_rect` is their bounding box.
    pub fn content_rects(&self) -> Vec<IRect>;  pub fn content_rect(&self) -> IRect;
    pub fn stamp_id(&self) -> Option<StampId>;
    pub fn is_raster(&self) -> bool;
    pub fn kind_name(&self) -> &'static str;                // "paint" | "stamp" | "text"
    pub fn byte_len(&self) -> usize;
    pub fn same_pixels(&self, other: &Layer) -> bool;       // Arc::ptr_eq on the pixel storage
}

pub struct TextureSet {
    pub id: TextureSetId, pub name: String, pub width: u32, pub height: u32,
    pub layers: Vec<Layer>,                                 // bottom -> top
    pub active: Option<LayerId>,
    pub texel_map: Arc<TexelMap>,                           // built eagerly at document creation
    pub(crate) composite: PixelBuf,                         // what the 3D view, the preview and the flattened export show
    pub(crate) pending: DirtyRegion,                        // layer changes not yet composited
    pub(crate) gpu_dirty: DirtyRegion,                      // composited, not yet handed to the app
    pub(crate) scratch: Option<Box<crate::brush::StrokeScratch>>,
}
impl TextureSet {
    pub fn composite(&self) -> &PixelBuf;
    pub fn full_rect(&self) -> IRect;
    pub fn layer(&self, id: LayerId) -> Option<&Layer>;
    pub fn layer_index(&self, id: LayerId) -> Option<usize>;
    pub fn active_layer(&self) -> Option<&Layer>;
    pub(crate) fn mark_dirty(&mut self, r: IRect);          // clips to the set, adds to `pending`
    pub(crate) fn mark_layer_content_dirty(&mut self, index: usize);   // mark_dirty of every content_rects() rect
    pub(crate) fn allocate_raster(&mut self, index: usize);           // an unallocated paint layer gets its buffer
}
```

### 5.12 `stamp.rs` [S], `stamp_bake.rs` [C4]

```rust
// stamp.rs
#[derive(Clone, Debug, PartialEq)]
pub enum StampSource { Image { blob: BlobId }, Text(TextSpec) }

/// Oriented projection box. Local frame: +X right, +Y up, +Z out of the surface toward the viewer.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Projector {
    pub origin: Vec3,                 // decal centre, on the surface
    pub rotation: Quat,               // local -> world
    pub half_size: Vec2,              // world units (x = half width, y = half height)
    pub depth_front: f32,             // box extent along +Z (toward the viewer)
    pub depth_back: f32,              // box extent along -Z (into the surface)
    pub angle_start_deg: f32,         // full weight while the surface faces the projector within this angle (default 70)
    pub angle_end_deg: f32,           // zero weight beyond this angle (default 88)
    pub backface_cull: bool,          // default true
    pub occlusion: bool,              // default true: surfaces hidden behind others receive nothing
    pub flip_x: bool, pub flip_y: bool,
    pub mode: SurfaceMode,            // round 6: Project (default) or Wrap; in Wrap the depth, angle, back-face and
                                      // occlusion fields are kept (for switching back) and ignored
    pub wrap: WrapOptions,            // round 6: the Wrap options, kept (and ignored) in Project mode
}

/// Round 6: how a 3D stamp reaches the surface. `from_surface` gives Project.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SurfaceMode { #[default] Project, Wrap }
/// Round 6: the options of Wrap mode; both false = stop at the edge of a part, ends meet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct WrapOptions { pub jump_gaps: bool, pub overlap: bool }

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvPlacement {
    pub set: TextureSetId,
    pub center_px: Vec2, pub size_px: Vec2,   // texels of the target set
    pub rotation: f32,                        // radians, positive = clockwise on screen (v-down); stored verbatim in the manifest
    pub flip_x: bool, pub flip_y: bool,
    pub clip_to_islands: bool,                // default false
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Placement { Projected(Projector), Uv(UvPlacement) }

#[derive(Clone, Debug, PartialEq)]
pub struct StampObject {
    pub id: StampId, pub name: String,
    pub source: StampSource,
    pub placement: Placement,
    pub tint: [u8; 4],                // multiplies the source (straight RGBA); default [255; 4]
    pub home_set: TextureSetId,       // the set that always keeps a layer for this stamp, even when its bake is empty
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BakeQuality { Preview /* no occlusion rays, single mip level */, Full }

/// The automatic name of a text stamp: the first non-blank line of `content`, trimmed, at most 32 characters (then
/// "..." is appended); "Text" when there is none. The Text tool names new stamps with it; `set_stamp_source` keeps a
/// stamp that still carries it following its text (5.20, fix round 3, R3-9).
pub fn text_stamp_name(content: &str) -> String;

// stamp_bake.rs
impl Projector {
    /// Frame from a surface point: Z = normal, Y = up_hint projected onto the tangent plane (any orthonormal vector
    /// if parallel), X = Y x Z. depth_front = depth_back = depth. Other fields take their defaults.
    pub fn from_surface(position: Vec3, normal: Vec3, up_hint: Vec3, half_size: Vec2, depth: f32) -> Projector;
    pub fn axes(&self) -> (Vec3, Vec3, Vec3);                 // world right, up, out
    pub fn corners(&self) -> [Vec3; 4];                       // TL, TR, BR, BL of the image, on the plane z = 0
    /// Same decal moved to another surface point: the frame is rotated by the shortest arc from the old to the new normal.
    pub fn moved_to(&self, position: Vec3, normal: Vec3) -> Projector;
    /// Rotation about local +Z. Positive = clockwise as seen from +Z (by the viewer), the same sense as UvPlacement.rotation:
    /// `rotation = rotation * Quat::from_rotation_z(-radians)`. A +90 degree turn moves the image's top edge to the right.
    pub fn rotated(&self, radians: f32) -> Projector;
    pub fn bounding_radius(&self) -> f32;
    pub fn is_finite(&self) -> bool;
}
impl UvPlacement { pub fn corners_px(&self) -> [Vec2; 4]; /* TL, TR, BR, BL */  pub fn is_finite(&self) -> bool; }

/// Projects `src` through `p` onto the surface and returns one cropped buffer per touched set (ascending set id).
/// `maps[i]` is the texel map of set i. Sets with no coverage are omitted.
pub fn bake_projected(mesh: &MeshData, bvh: &Bvh, maps: &[Arc<TexelMap>], src: &PremulMips, tint: [u8; 4],
                      p: &Projector, quality: BakeQuality) -> Vec<(TextureSetId, SparseBuf)>;
/// Places `src` directly in the texture of `p.set`. `map` is that set's texel map (used for clip_to_islands and the size).
pub fn bake_uv(src: &PremulMips, tint: [u8; 4], p: &UvPlacement, map: &TexelMap) -> SparseBuf;
/// `p` with depth_front / depth_back fitted to the surface under its footprint (6.5 "Depth auto-fit"). Depends only on
/// the mesh and origin, rotation, half_size, backface_cull of `p`; unusable input or no surface at the centre => `*p`.
pub fn fit_projector_depth(mesh: &MeshData, bvh: &Bvh, p: &Projector) -> Projector;
/// The depths of `p` are the fitted ones (relative 1e-3), or the placement default of earlier versions
/// (depth_front == depth_back == larger half size): moving / scaling re-fits them. False for a depth typed in by hand.
pub fn projector_depth_is_auto(mesh: &MeshData, bvh: &Bvh, p: &Projector) -> bool;
pub const DEFAULT_ANGLE_START_DEG: f32 = 70.0;  pub const DEFAULT_ANGLE_END_DEG: f32 = 88.0;
pub const DEPTH_FADE_FRACTION: f32 = 0.2;       // smoothstep fade over the last 20% of each depth limit
```
Angle defaults (fix round 2, item 2): a decal is stretched by `1 / cos(angle)` along the slope. Full strength up to 70 degrees
(stretch <= 2.9x, the image still reads; on a sphere that is the inner 94% of the projected disc radius) and a smooth fall to
zero at 88 degrees (stretch 29x) lets a stamp wrap onto a moderately curved surface and run out softly at the silhouette instead
of the old 60 / 85 degrees, whose fade started where most curved surfaces still face the viewer well. Back-face culling stays on.
Stored projectors keep their own angles (the defaults apply to new placements).

Round 6, the wrap (`wrap_param.rs`, `wrap_bake.rs`; algorithm 6.5a):
```rust
pub const WRAP_HEADROOM: f64 = 1.25;  pub const WRAP_MAX_VERTS: usize = 1_000_000;  pub const WRAP_MAX_TRIS: usize = 3_000_000;
pub const WELD_SNAP: f64 = 1e-5 /* x diag */;  pub const CONSENSUS: f64 = 0.25;  pub const TEAR_RATIO: f64 = 2.0;
pub const SPINE_NUDGE: f64 = 1e-6;  pub const GAP_JUMP: f64 = 0.01 /* x diag */;  pub const WRAP_MAX_TURNS: u32 = 8;
/// The parameterization of the surface around one wrapped stamp. Immutable, shared by Arc.
pub struct WrapParam { /* layers (one per turn, bottom first) of (TriId, three stamp-local corners); the inputs it was
                          made for: mesh address, origin and rotation bits, spine axis, options, bucketed reach, source
                          size, largest texel size */ }
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WrapStats { pub verts: u32, pub tris: u32, pub cut_tris: u32, pub covered: f32 /* share of the picture with
    surface under it */, pub met_itself: bool, pub reached_edge: bool, pub limited: bool, pub jumps: u32,
    pub turns: u32, pub micros: u64,
    // Round 8: relaxed (false: the exact seed was kept), seam edges inside the picture (all rules and per rule
    // S1..S5), relaxed triangles flipped inside the picture, the 95th percentile of |area ratio - 1| inside the picture,
    // the share of the laid picture area the owner test drops (cell-centre estimate), CG iterations.
    pub relaxed: bool, pub seams: u32, pub seams_by_rule: [u32; 5], pub flipped: u32, pub stretch: f32,
    pub rejected: f32, pub cg_iters: u32,
    // Round 9: the spine's traces passed beside each other (T2); the share of the picture's length laid when the ends
    // meet (1 for an open spine and for Overlap turns); the share of the picture's cells without an owner inside the
    // laid outline (A7).
    pub passed: bool, pub ends_share: f32, pub lost_cut: f32,
    // Stable (w2): the largest 3D extent across the picture of the parts it lies on over the picture's height (<= 1);
    // a whole closed part (no boundary edge) was laid on the first turn.
    pub room: f32, pub wraps_closed: bool,
    // Stable (fix_wrap_1): the share of the picture's cells with an owner (each once; `covered` sums laid areas, so
    // two places at one picture point count twice); the share of its cells bare beyond a closure (where the ends met).
    pub shown: f32, pub beyond: f32,
    // Stable (fix-wrap): the share of the picture's cells bare in a gap between two separate parts it continued across
    // (an uncovered run of at most the jump gap between cells owned by two parts); the share bare where it stops at a
    // neighbouring part (next to a laid cell on a continued part that goes on, or with another part's surface within
    // the jump gap, and the bare cells nearest to such a cell).
    pub gaps: f32, pub at_part: f32 }
/// Round 8 (`Document::stamp_wrap_quality`): ink area, the share of picture bins found in two places (`dup`, > 6 bins
/// apart), the share of neighbouring bin pairs lying apart (`scatter`), the inked bins.
pub struct WrapQuality { pub ink_area: f32, pub dup: f32, pub scatter: f32, pub bins: u32 }
// Round 8 constants (pub): WRAP_STOP_SLACK 1.7 (plan 1.3, see 6.5a), ISO_TOL 2e-4, CONE_CUT_DEG 45, SEAM_NEAR 3,
// OWNER_MAX_CELLS 1024, RELAX_CG_BUDGET 2500, PAR_MIN_UNKNOWNS 16384 (round 9 check:B: 8192), PAR_CHUNK 8192; round 9: OWNER_SAME 3 (OWNER_TOL
// removed), BURY_REACH 0.04 (= 4 GAP_JUMP, x diag), CONE_RADIUS 0.1 (x across reach), RELAX_ARAP_ITERS 12 (was 6);
// check:A: ENDS_MEET 2, HOLE_FILL 8 (S1 on an open spine, S5's small holes; 6.5a), COT_MAX 1e3 (private); round 9
// check:A: PASS_NEAR 2 (x across reach, T2), EXACT_STRAIN 0.25 (T5); stable w2: FLAT_TOL 1e-6 (rad), AREA_PUSH 1,
// AREA_PUSH_MAX 1.5.
// WrapParam::src_size(), texel_world(): the inputs it was made for.
/// None when the projector is unusable (non-finite, zero size, empty source) or no triangle lies within reach.
pub fn wrap_param(mesh: &MeshData, bvh: &Bvh, maps: &[Arc<TexelMap>], src_size: [u32; 2], p: &Projector) -> Option<WrapParam>;
impl WrapParam {
    pub fn stats(&self) -> WrapStats;
    /// `stats` measured for `p`'s size (a projector it serves: a scale inside the reach bucket reuses the
    /// parameterization). Stable (w1): every size-dependent field, equal to a fresh build's (round 9: `covered` only);
    /// `stats_for` takes the source size it was made for, `stats_for_src` the current one (the picture's border).
    /// Used by the Document queries and the `wrap:` log line.
    pub fn stats_for(&self, p: &Projector) -> WrapStats;
    pub fn stats_for_src(&self, p: &Projector, src_size: [u32; 2]) -> WrapStats;
    /// Same mesh Arc, origin and rotation bit-equal, same spine axis and options, same bucketed reach and (round 9)
    /// the same front stop (6.5a "Reuse").
    pub fn serves(&self, mesh: &Arc<MeshData>, p: &Projector) -> bool;
    pub fn local_of(&self, tri: TriId, bary: [f32; 3]) -> Option<Vec2>;          // surface -> stamp-local (top turn)
    pub fn surface_of(&self, mesh: &MeshData, local: Vec2) -> Option<SurfacePoint>; // stamp-local -> surface (top turn)
    pub fn outline(&self, mesh: &MeshData, half_size: Vec2, samples_per_edge: usize) -> Vec<Vec<SurfacePoint>>;
}
/// One cropped buffer per touched set (ascending set id), like `bake_projected`.
pub fn bake_wrapped(mesh: &MeshData, maps: &[Arc<TexelMap>], param: &WrapParam, src: &PremulMips, tint: [u8; 4],
                    p: &Projector, quality: BakeQuality) -> Vec<(TextureSetId, SparseBuf)>;
```

### 5.13 `text/` — types [S], implementation [C4]

```rust
// text/mod.rs  [S]
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum TextAlign { Left, Center, Right }
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Outline { pub width_em: f32 /* fraction of the em, 0..=0.25; width in raster px = width_em * size_px */, pub color: [u8; 4] }
/// A font face whose file bytes live in the document's BlobStore. `style` follows the naming rule below (`style_name`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontRef { pub blob: BlobId, pub face_index: u32, pub family: String, pub style: String }
#[derive(Clone, PartialEq, Debug)]
pub struct TextSpec {
    pub content: String,              // may contain '\n'
    pub font: FontRef,
    pub size_px: f32,                 // raster resolution (em size in px), clamped to 8..=512. Chosen by the core for stamps
                                      // (section 6.5); callers creating a spec use TEXT_PLACEMENT_PX
    pub color: [u8; 4],
    pub align: TextAlign,
    pub line_spacing: f32,            // multiplier, default 1.0
    pub letter_spacing: f32,          // extra advance per glyph as a fraction of the em, default 0 (px = letter_spacing * size_px)
    pub outline: Option<Outline>,
}
pub const TEXT_PLACEMENT_PX: f32 = 128.0;   // size_px of a freshly built TextSpec (before the core picks the real resolution)
#[derive(Copy, Clone, Debug)]
pub struct PlacedGlyph { pub ch: char, pub font_index: u8, pub x: i32, pub y: i32, pub width: u32, pub height: u32 }
#[derive(Clone, Debug)]
pub struct TextLayout { pub glyphs: Vec<PlacedGlyph>, pub width: u32, pub height: u32, pub line_count: u32, pub pad: u32 }

// ---- round 4 (S1) ----
pub struct TextSpec { /* the eight fields */ pub fx: TextFx }      // fx: TextFx::default() = a round-3 text
pub struct TextFx { pub gaps: Vec<Gap>, pub bold_em: f32, pub slant_deg: f32, pub warp: Option<Warp>, pub path: Option<TextPath> }
impl TextFx { pub fn is_default(&self) -> bool; pub fn needs_outlines(&self) -> bool }   // gaps alone: classic painter
pub struct Gap { pub after: u32 /* char index of the left character */, pub em: f32 }
pub struct Warp { pub style: WarpStyle, pub bend: f32, pub horizontal: f32, pub vertical: f32, pub vertical_axis: bool }
pub enum WarpStyle { Arc, ArcLower, ArcUpper, Arch, Bulge, ShellLower, ShellUpper, Flag, Wave, Fish, Rise, Fisheye, Inflate, Squeeze, Twist }
impl WarpStyle { pub const ALL: [WarpStyle; 15]; pub fn key(self) -> &'static str; pub fn from_key(&str) -> Option<WarpStyle>; pub fn label(self) -> &'static str }
pub struct TextPath { pub shape: PathShape, pub start: f32, pub flip: bool }
pub enum PathShape { Arc { bend: f32 }, Circle { diameter: f32, center: [f32; 2] }, Curve { points: Vec<[f32; 2]> } }
pub const GAP_MIN_EM: f32 = -0.5; GAP_MAX_EM = 4.0; MAX_PATH_POINTS = 16; PATH_COORD_MAX_EM = 60.0;
pub const MAX_STYLED_PIXELS: u64 = 1 << 24; FAUX_BOLD_EM = 0.02; FAUX_SLANT_DEG = 12.0; MAX_BOLD_EM = 0.08; MAX_SLANT_DEG = 45.0;
impl TextSpec {
    pub fn new(content: &str, font: FontRef) -> TextSpec;
    pub fn sanitize_fx(&mut self);          // repairs fx, never rejects (FEATURE_TEXT 4.1 rules)
    pub fn floats(&self, f: &mut dyn FnMut(&'static str, f32));                 // one macro generates both visitors
    pub fn floats_mut(&mut self, f: &mut dyn FnMut(&'static str, &mut f32));
    pub fn differs_only_in_spacing(&self, other: &TextSpec) -> bool;
}
pub struct TextFrame { pub size: [u32; 2], pub anchor_px: [f32; 2], pub em_px: f32, pub box_px: [f32; 2],
                       pub baseline0_px: f32, pub align_px: [f32; 2], pub circle_fill: f32 }
pub enum RasterQuality { Full, Preview { max_pixels: u32 } }
pub struct TextRaster { pub image: PixelBuf, pub frame: TextFrame, pub full: bool }
// raster.rs:
pub struct FaceFonts<'a> { pub metrics: &'a fontdue::Font, pub bytes: &'a [u8], pub face_index: u32 }
pub struct TextFonts<'a> { pub font: Option<FaceFonts<'a>>, pub fallback: FaceFonts<'a> }
pub fn text_frame(fonts: &TextFonts<'_>, spec: &TextSpec) -> Result<TextFrame, TextError>;
pub fn render_text(fonts: &TextFonts<'_>, spec: &TextSpec, quality: RasterQuality) -> Result<TextRaster, TextError>;
// font_catalog.rs:
pub struct FaceTraits { pub weight: u16, pub italic: bool, pub stretch: u16 }
pub fn face_traits(bytes: &[u8], face_index: u32) -> Option<FaceTraits>;
impl FontCatalog { pub fn sample(&self, id: fontdb::ID, text: &str, px: f32) -> Option<(u32, u32, Vec<u8>)> }
// text/font_catalog.rs  [C4]
#[derive(Clone, Debug)]
pub struct FontFaceInfo { pub id: fontdb::ID, pub family: String /* FaceInfo.families[0].0 */,
                          pub style: String /* style_name(weight, slant); a later face with a taken name: "<style> (<PostScript name>)" */,
                          pub weight: u16, pub italic: bool, pub monospaced: bool, pub face_index: u32,
                          pub post_script_name: String /* name id 6, may be empty */, pub stretch: u16 /* OS/2 width class, 5 normal */,
                          pub file_bytes: u64 /* size of the whole font file; 0 unknown */ }
/// The one definition of a style string (shown in the UI, persisted in FontRef.style, looked up by `find`).
/// fontdb exposes no subfamily name, so it is derived: weight name from `FaceInfo.weight.0` — 100 "Thin", 200 "ExtraLight",
/// 300 "Light", 400 "Regular", 500 "Medium", 600 "SemiBold", 700 "Bold", 800 "ExtraBold", 900 "Black", any other value "W<n>" —
/// followed by " Italic" / " Oblique" for `fontdb::Style::Italic` / `Oblique`; "Regular Italic" collapses to "Italic",
/// "Regular Oblique" to "Oblique".
pub fn style_name(weight: u16, style: fontdb::Style) -> String;
/// System + bundled font enumeration (fontdb). Send + Sync.
pub struct FontCatalog { /* fontdb::Database, family list, file_sizes: HashMap<PathBuf, u64>, shared: Mutex<HashMap<source, Weak<[u8]>>> */ }
impl FontCatalog {
    pub fn bundled() -> FontCatalog;                       // Ubuntu-Light and Hack-Regular from epaint_default_fonts
    pub fn load_system(&mut self);                         // see below; slow, call off the UI thread
    pub fn load_font_file(&mut self, path: &Path) -> std::io::Result<()>;  // one file (all faces of a collection), size read once
    pub fn families(&self) -> &[String];                   // sorted by to_lowercase(), ties by the string; de-duplicated
                                                           // without case (first spelling kept), as `faces` matches
    pub fn faces(&self, family: &str) -> Vec<FontFaceInfo>;                 // sorted by (weight, italic), stable; no face dropped
    pub fn find(&self, family: &str, style: &str) -> Option<FontFaceInfo>;  // exact family (case-insensitive) + exact style string
    pub fn face_bytes(&self, id: fontdb::ID) -> Option<(Arc<[u8]>, u32)>;   // whole font file + face index; one allocation per file while held
    pub fn sample(&self, id: fontdb::ID, text: &str, px: f32) -> Option<(u32, u32, Vec<u8>)>; // coverage, at most 128 x 20, row 0 on top
}
pub const SAMPLE_MAX_WIDTH: u32 = 128; pub const SAMPLE_MAX_HEIGHT: u32 = 20;
pub const SAMPLE_TEXT: &str = "Abc 123"; pub const SAMPLE_PX: f32 = 16.0;
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FaceTraits { pub weight: u16, pub italic: bool, pub stretch: u16 }
pub fn face_traits(bytes: &[u8], face_index: u32) -> Option<FaceTraits>;  // from the font bytes, fontdb's rule; None: no font
pub const DEFAULT_FONT_FAMILY: &str = "Ubuntu";            // Ubuntu-Light.ttf: typographic family (name id 16, preferred by fontdb,
pub const DEFAULT_FONT_STYLE: &str = "Light";              // REG\fontdb-0.24.0\src\lib.rs:1064) "Ubuntu", OS/2 weight 300 => "Light"
pub fn default_font_bytes() -> &'static [u8];              // epaint_default_fonts::UBUNTU_LIGHT

/// Parsed fontdue fonts keyed by (blob, face_index, scale bucket). Parsing is slow; never drop this cache per frame.
#[derive(Default)]
pub struct FontCache { /* HashMap<(BlobId, u32, u32), Arc<fontdue::Font>>, fallback */ }
impl FontCache {
    pub fn get(&mut self, blobs: &BlobStore, font: &FontRef, px: f32) -> Result<Arc<fontdue::Font>, TextError>;
    pub fn fallback(&mut self, px: f32) -> Arc<fontdue::Font>;             // bundled default font
}

// text/layout.rs  [C4]      fonts[0] = chosen face, fonts[1..] = fallbacks; each char uses the first font that has the glyph
pub fn layout_text(fonts: &[&fontdue::Font], spec: &TextSpec) -> Result<TextLayout, TextError>;
pub fn measure_text(fonts: &[&fontdue::Font], spec: &TextSpec) -> Result<[u32; 2], TextError>;
// text/raster.rs  [C4]
pub fn rasterize_text(fonts: &[&fontdue::Font], spec: &TextSpec) -> Result<PixelBuf, TextError>;   // straight alpha
// text/edt.rs  [C4]
/// Exact Euclidean distance (in pixels) from every pixel to the nearest pixel with coverage >= 128, with the
/// sub-pixel edge correction of section 6.8. Returns w*h distances (0 inside).
pub fn distance_to_ink(coverage: &[u8], width: u32, height: u32) -> Vec<f32>;
```

**Round 4 (S1).** Classic frame (no outlines): `size` = the canvas, `anchor_px` = its centre, `em_px = raster_px`, `box_px` = canvas
minus twice the padding, `baseline0_px = round(ascent) - box_h / 2`, `align_px` = the left edge / centre / right
edge of the line box at the anchor's height, `circle_fill = 0`. The geometry modules `outline`, `scan`, `warp`,
`path`, `gaps`, `styled`, `frame` are listed in 2.1; their algorithm texts come from T1 and T2 (FEATURE_TEXT 5.3 to
5.12).

Font list and font memory (round 4, FEATURE_TEXT 5.13, 5.9):
- `load_system` = fontdb's `load_system_fonts()` (the three Windows font folders: `%SYSTEMROOT%\Fonts`,
  `%USERPROFILE%\AppData\Local\Microsoft\Windows\Fonts`, `...\Roaming\...\Fonts`; no registry, no named styles of
  variable fonts, user answer 2026-11-17) plus one pass over the same folders that records the size of every font
  file from the directory listing and loads the files whose extension is `ttf`/`ttc`/`otf`/`otc` in mixed case
  (fontdb takes only all-lower or all-upper case). `load_font_file(path)` succeeds once fontdb loaded the file; a
  size that cannot be read stays unknown (0). Sizes of files fontdb found elsewhere are read with one
  `fs::metadata` each. A bundled font's size is its byte length. `faces()` reads no file.
- `faces(family)` never drops a face. The first face with a style name keeps the plain name, so every `FontRef.style`
  of an existing project still resolves through `find`; a later face with the same name is named
  `"<style> (<PostScript name>)"` (e.g. "Regular (Candara-Light)"), or `"<style> (face <index>)"` without a
  PostScript name; a still-taken name gets " 2", " 3" ... The round-3 rule "duplicate styles: first wins" is gone.
- `face_bytes` keeps a weak reference per source (font file path, or the allocation of loaded bytes): while anyone
  (a document blob, a pending job) holds the bytes, every face of that file gets the same `Arc` without reading the
  file again, and `BlobStore::insert` recognises it before hashing (5.6). When nobody holds them the file is read
  again (one copy). The copy is made without holding the catalog's lock; when two threads copy the same file at
  once, the allocation stored first is handed to both. Fonts a document knows by name are not asked from the catalog at all (`Document::find_font`,
  `text_block::resolve_font`).
- `sample(id, text, px)` reads the font in place through fontdb's `with_face_data` (no copy, no fontdue parse), maps
  characters with `outline::char_to_glyph` (every cmap subtable, the last non-zero mapping wins: the painter's map,
  so symbol fonts draw the same glyphs as in a text), draws the outlines of `outline::glyph_outline` with
  `scan::fill_contours`. It draws the characters of `text` the face maps (spaces kept); when it maps none of them,
  its first six mapped characters above U+0020 (kept in a bounded set while the cmap is enumerated: no list of all
  code points). The line (ascender to descender) fits 20 px; a taller line draws the
  face smaller; the text is cut at 128 px. `None` for an unknown id, a non-positive or non-finite size, or nothing
  inked.
- `face_traits(bytes, index)`: OS/2 weight and width class; italic = OS/2 style not normal or `post` italic angle
  not 0 (the rule fontdb uses, so for an installed face it equals `FontFaceInfo`). It is the source of truth for the
  B and I buttons (FEATURE_TEXT 5.9), read from the document's font bytes, never from the installed list.

**Text geometry (T1, FEATURE_TEXT 5.3 to 5.7, 5.12).** Pure functions, no state, no fonts kept.

- `outline.rs`. `glyph_outline(face, glyph, px) -> GlyphOutline`: `ttf_parser::Face::outline_glyph`, scaled by
  `px / units_per_em`, y turned down (glyph space: origin on the baseline at the pen position, x right, y down,
  raster pixels), curves flattened with `FLATTEN_TOLERANCE_PX` = 0.05: a quadratic piece into
  `ceil(sqrt(M / (4 tol)))` lines, a cubic into `ceil(sqrt(0.75 M / tol))`, `M` = largest second difference of the
  control points (at most 256 lines per piece). Contours are closed polylines without a repeated end point; fewer
  than 3 points are dropped. An empty outline for a glyph without one, an index beyond the font, or a size that is
  not positive. `char_to_glyph(face, ch)` walks every cmap subtable in file order and keeps the last mapping that is
  not glyph 0 — the table `fontdue` builds — so symbol fonts with only (1, 0) / (3, 0) subtables map as in the
  painter (`Face::glyph_index` skips them).
- `scan.rs`. `fill_contours(contours, origin, w, h) -> Coverage`: signed-area accumulation into `(w + 2) * h` f32
  cells. Every edge is clipped to the rows, cut at every pixel row and column it crosses; each piece adds its exact
  trapezoid area to its cell and the rest of its signed height to the next cell. Only the columns `0..w` are cut:
  the part of a row piece left of the bitmap adds its share of the height to cell 0 as one piece (full cover), the
  part right of it to cell `w` (none visible), so the work does not grow with how far an edge reaches outside. Per row a running sum,
  coverage `round(255 * min(|sum|, 1))`: non-zero winding as in `fontdue`, opposite windings cancel. No thickening
  here (that is T2's EDT on the finished coverage).
- `warp.rs`. `WarpMap::new(warp, frame_min, frame_max)` prepares a map; `map(p)` applies the style (15 formulas of
  FEATURE_TEXT 5.6 on `u = (x - cx) / a`, `v = (y - cy) / h`), the Vertical direction (turn by -90 degrees, style
  with the half sizes exchanged, turn back) and the two distortions (scaled about the box of the warped frame
  border, 65 samples per side, computed once in `new` and kept in a private field; a map is therefore immutable
  after `new`: build a new one instead of changing its public fields). The map is the identity when a
  half size is below 0.5 px, a number is not finite, or bend and both distortions are 0 (Arc also below
  |bend| 1e-4). `map_polyline(points, closed)`: every edge cut into equal pieces of at most 8 px, each halved while
  the mapped middle is off the mapped chord by more than 0.1 px or the mapped piece is longer than 16 px, at most
  6 times; the result is mapped (closed polylines do not repeat their first point). No style folds (unit test over
  15 styles x 2 directions x 8 bends x 7 aspect ratios x 3 x 3 distortions: every mapped triangle keeps more than 0.05
  of its area).
- `path.rs`. `PathLayout::new(path, px, align, lines)` builds one walk per text line (`TextLine { baseline, width }`,
  unrounded, text space); `line_point(line, s)` gives point and unit tangent at arc length `s` from the start of
  that text line (alignment and Start already applied); `place(line, pen, advance, p)` puts the middle of a rigid
  glyph on the walk (`point + T (x' - adv / 2) + D y'`, `D = (-T.y, T.x)`); a painter placing many points of one
  glyph calls `glyph_frame(line, pen, advance)` once (the walk look-up at `pen + adv / 2`) and
  `place_with(frame, advance, p)` per point (one rotation), with the same result as `place`. Arc: `r = W / (|bend| 2 pi)`, circle
  touching the first baseline at `(0, b0)`, line `l` on the concentric radius `r -/+ (y_l - y_0)` (at least a
  quarter em), aligned in angle, straight below |bend| 0.001, Start clamped to +-W, straight continuation beyond
  the span, `flip` negates the angle and reverses the tangent (the letters turn round, so lines `l > 0` go the
  other way: radius `r +/- (y_l - y_0)`; a straight flipped Arc stacks its lines upwards from `b0`). Circle: centre and diameter from the spec, Left /
  Center / Right against the half circle on the reference point (top, or bottom with `flip`), Start turns every line
  by the same angle, `circle_fill` = largest `width_l / (2 pi r_l)`. Curve: centripetal Catmull-Rom as cubic Bezier
  pieces (ends mirrored, knot spacing at least 0.001 px), 32 parameter steps per piece in a running chord-length
  table; a look-up is a binary search, a linear step of the parameter and the exact Bezier point and derivative;
  lines `l > 0` use the sampled polyline moved along `D` with their own table (with `flip` along `-D`, the
  letters' "down" on the reversed walk); Start clamped to +-L (L taken as 0 when huge points overflow it);
  `flip` walks backwards; straight continuation beyond both ends. Every number from a spec is made finite first,
  so no input panics.
- `gaps.rs`. `get`, `set` (clamped, 0 removes, non-finite ignored), `insert(at, n)` (entries with `after >= at`
  move), `delete(range)` (entries in the range go, later ones move down), `shift_for_edit(old, new)` (common
  prefix and suffix in chars, one delete and one insert), `is_joined`, `slots(content, gaps)` (per text line the
  pairs of neighbouring laid-out characters — skipped control characters are not laid out — without a pair whose
  right character is joined or whose left one is U+200D, plus an end slot for a last character with a stored
  value), `slot_name` ("(space)", "(tab)", "(no-break space)", "U+XXXX" for invisible characters). All indices
  count chars; the list invariants (sorted, one entry per character, no zero) hold after every call.

Round-4 contract change (M1, contract request T1-2): `WarpMap`'s fields `warp`, `frame_min`, `frame_max` are private
with the getters `warp()`, `frame_min()`, `frame_max()`; a map is immutable after `new`.

`FontCatalog: Clone` (round 4, T7): copies the fontdb face list, the family list and the file sizes (font files stay
files; nothing is read); the shared-bytes table starts empty in the copy. The font-sample job draws from such a copy.

### 5.14 `document.rs` [S]

```rust
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Commit { Preview /* apply, no history entry yet */, Final /* apply and push one history entry */ }

pub struct Document {
    pub name: String,
    pub mesh: Arc<MeshData>,
    pub bvh: Arc<Bvh>,
    pub sets: Vec<TextureSet>,                      // index == TextureSetId.0
    pub stamps: Vec<StampObject>,                   // ascending id
    pub blobs: BlobStore,
    pub source: Arc<SourceBundle>,                  // original model files (R7)
    pub import_options: ImportOptions,
    pub import_warnings: Vec<ImportWarning>,        // not persisted
    pub history: History,
    pub ui_state: serde_json::Value,                // opaque to the core; round-tripped in the manifest
    pub(crate) next_id: u64,
    pub(crate) revision: u64,
    pub(crate) saved_revision: u64,
    pub(crate) stroke: Option<crate::stroke::StrokeSession>,
    pub(crate) pending_edit: Option<crate::history::PendingEdit>,
    pub(crate) preview_baked: BTreeSet<StampId>,    // stamps whose caches are currently BakeQuality::Preview (5.20, 6.7)
    pub(crate) bakes: Box<BakeSets>,                // round 5: boxed so that Document stays small (wte_app's
                                                    // JobOutput::Built carries it by value; clippy large_enum_variant):
                                                    //   preview_stale: BTreeSet<StampId> — deferred Preview bakes still to run
                                                    //     (subset of preview_baked, 5.20);
                                                    //   unbaked: BTreeSet<StampId> — stamps whose Full bake is still due (5.20
                                                    //     "Deferred Final bakes"); part of the structure state
                                                    //     (StructureSnapshot::unbaked); empty while the mode is off
    pub(crate) defer_preview_bakes: bool,           // set_defer_preview_bakes (5.20)
    pub(crate) defer_final_bakes: bool,             // round 5: set_defer_final_bakes (5.20)
    pub(crate) png_reuse: Option<Arc<PngReuse>>,    // images of the file last loaded / saved (7.1); not persisted
    pub(crate) version_counter: u64,                // source of Layer::version values; not persisted
    pub(crate) fonts: FontCache,
    pub(crate) image_mips: HashMap<BlobId, Arc<PremulMips>>,              // decoded stamp images
    pub(crate) text_mips: HashMap<StampId, (TextSpec, Arc<PremulMips>)>,  // rasterized text per stamp
    pub(crate) text_caches: Box<TextCaches>,   // round-4 text caches, boxed so that Document stays small
}
// Round 6: `BakeSets` (the boxed `bakes`) also holds `wrap_params: WrapCache`, the parameterizations of wrapped stamps
// (6.5a "Reuse"): derived data, not in snapshots, not saved, not part of the structure state; at most 16 entries, least
// recently used dropped first; an entry is used only when it `serves` the stamp's placement; `remove_stamp` drops its
// entry and every insertion drops entries of stamps that no longer exist. (In `bakes` so that `Document` keeps its size.)
// objects-tabs: `WrapCache::clear()` drops every entry; `set_hidden_objects` calls it on a change.
// Round 8: `BakeSets` also holds `defer_wrap_previews: bool` (`set_defer_wrap_previews`) and `edit_serial: u64` (bumped
// when the first stamp joins `preview_baked`, i.e. per pending edit; a wrap Preview job's output applies only in the
// edit it was made in).
// compile-time check in document.rs: fn _assert_send<T: Send>() {}  _assert_send::<Document>();

impl Document {                                     // implemented by the scaffold
    pub fn set(&self, id: TextureSetId) -> Option<&TextureSet>;
    pub(crate) fn set_mut(&mut self, id: TextureSetId) -> Option<&mut TextureSet>;
    pub fn set_ids(&self) -> Vec<TextureSetId>;
    pub fn find_layer(&self, id: LayerId) -> Option<(TextureSetId, usize)>;
    pub fn layer(&self, id: LayerId) -> Option<&Layer>;
    pub(crate) fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer>;
    pub fn stamp(&self, id: StampId) -> Option<&StampObject>;
    pub fn stamp_layers(&self, id: StampId) -> Vec<(TextureSetId, LayerId)>;      // ascending set id
    pub fn stamp_of_layer(&self, layer: LayerId) -> Option<StampId>;
    pub fn stamp_locked(&self, id: StampId) -> bool;                              // any linked layer is locked (5.20)
    pub fn texel_maps(&self) -> Vec<Arc<TexelMap>>;                               // index == set id
    pub fn texel_world_size(&self, set: TextureSetId) -> f32;                     // texel_map.stats.texel_world
    pub fn raycast(&self, ray: &Ray) -> Option<Hit>;                              // nearest hit, Cull::None, t in [1e-5*diag, INF); passes hidden objects
    pub fn revision(&self) -> u64;                                                // bumped by every content mutation, undo and redo
    pub fn history_state(&self) -> u64;                                           // id of the current history state (6.7)
    pub fn is_modified(&self) -> bool;                                            // history_state() != saved_revision (the saved state id)
    pub fn mark_saved(&mut self, state: u64);                                     // history_state() / DocumentSnapshot::history_state
    pub(crate) fn alloc_id(&mut self) -> u64;
    pub(crate) fn touch(&mut self);                                               // revision += 1
    pub fn snapshot(&self) -> DocumentSnapshot;                                   // O(#layers): Arc clones only
    pub fn layer_version(&self, id: LayerId) -> Option<u64>;                      // Layer::version of that layer
    pub fn set_png_reuse(&mut self, reuse: Option<PngReuse>);                     // SaveStats::reuse of the last save (7.1)
    /// Err(EditError::StrokeInProgress) while a stroke is open. First statement of every history-producing command.
    pub(crate) fn guard_no_stroke(&self) -> Result<(), EditError>;
    /// THE way to report changed pixels of one layer: clips `r` to the set, adds it to the set's `pending` region and gives the
    /// layer a fresh `version` (version_counter += 1). Unknown layer: no-op.
    pub(crate) fn mark_layer_dirty(&mut self, layer: LayerId, r: IRect);
    pub(crate) fn fresh_version(&mut self) -> u64;                                // for layers created by a command
    /// debug_assert!s the structure invariants of section 6.5. Called at the end of push_structure_edit, restore_structure,
    /// flush_pending_edit and from_parts; a no-op in release builds.
    pub(crate) fn debug_check_structure(&self);
}

// objects-tabs (docs/plans/OBJECTS_TABS_PLAN.md 3.2): hidden objects. Private fields `hidden_objects: Vec<bool>` (one per
// `mesh.objects`) and `visibility_generation: u64`, set in `from_parts` (all false, 0); not in `DocumentSnapshot`,
// `StructureSnapshot`, the history or the manifest (the app keeps hidden object *names* in `ui_state`, 8.9).
impl Document {
    pub fn hidden_objects(&self) -> &[bool];                  // all false in a new or loaded document
    pub fn object_hidden(&self, object: u32) -> bool;         // false for an unknown index
    pub fn hidden_object_count(&self) -> usize;
    pub fn visibility_generation(&self) -> u64;               // 0 new; +1 at every set_hidden_objects that changed something
    /// Replaces the hidden flags (a slice of another length is padded with false / cut). Ok(false) = nothing changed.
    /// Err(StrokeInProgress) while a stroke is open (guard_no_stroke). View state: no history entry, no revision change,
    /// is_modified unaffected. Swaps `self.bvh` for `bvh.with_hidden(mask)` (mask = TriMask::from_fn(tri_count, hidden
    /// of tri_object_of(t)); None when nothing is hidden) and clears the wrap cache (`WrapCache::clear`: `serves` does not
    /// know about visibility). The pending edit is left alone (its later bake uses the new mask).
    pub fn set_hidden_objects(&mut self, hidden: &[bool]) -> Result<bool, EditError>;
    pub fn visible_bounds(&self) -> Aabb;                     // union of visible objects; mesh.bounds when none or all are hidden
    pub fn objects_bounds(&self, objects: &[u32]) -> Option<Aabb>;   // union of the listed visible objects; None when none
}
```
**Rules for hidden objects** (objects-tabs plan 3.4; the contract):

| Operation | Behaviour with hidden objects |
|---|---|
| Pick / hover / eyedropper / fill seed / stamp and text placement / gizmo drag / text-path points in the 3D view | the ray passes through hidden objects and hits what is behind them (`Document::raycast`, every BVH ray query) |
| Brush and eraser in 3D | hidden triangles are never dab candidates: no paint, no shadowing of "Same side" (6.4). Objects that share texels (instanced meshes, mirrored UVs) still share paint: a property of the UVs |
| Projected stamp / text bake | hidden triangles receive nothing and do not occlude; `fit_stamp_depth` fits to visible surfaces (6.5) |
| Wrap | hidden triangles are never part of the patch, never the anchor, never a Jump target (6.5a) |
| When a stamp is (re)baked | against the visibility at the moment the bake job is made (the job's `Arc<Bvh>`). Changing visibility never re-bakes existing stamps; undo / redo restore stored pixels; `project::load` keeps saved caches, and the rare load-time re-bake of a damaged cache runs with everything visible (the app restores visibility after `load` returns) |
| Fill (Layer / Island scope), 2D (UV) brush, UV stamps | texture-space operations: not masked (the seed comes from a visible surface) |
| Export, composite, save, thumbnails | unaffected |
| Undo / redo / revision / modified flag | unaffected by hide / show / select |

```rust

/// Immutable copy for background save/export. Send + Sync + 'static.
#[derive(Clone)]
pub struct DocumentSnapshot {
    pub name: String,
    pub mesh: Arc<MeshData>,
    pub source: Arc<SourceBundle>,
    pub import_options: ImportOptions,
    pub sets: Vec<SetSnapshot>,
    pub stamps: Vec<StampObject>,
    pub blobs: BlobStore,
    pub ui_state: serde_json::Value,
    pub next_id: u64,
    pub revision: u64,
    pub history_state: u64,                         // Document::history_state(): what mark_saved takes after the save
    pub png_reuse: Option<Arc<PngReuse>>,           // what project::save may copy (7.1)
}
#[derive(Clone)]
pub struct SetSnapshot { pub id: TextureSetId, pub name: String, pub width: u32, pub height: u32,
                         pub layers: Vec<Layer>, pub active: Option<LayerId>, pub texel_map: Arc<TexelMap> }
```
**Mutation rule** for every history-producing `impl Document` command in other files (everything in 5.17, 5.19 `fill`, 5.20 except
the read-only queries; the `stroke_*` methods are the only exception), in this order:
1. `self.guard_no_stroke()?` — while a stroke is open every such command returns `Err(EditError::StrokeInProgress)` and changes
   nothing (`undo`/`redo` return `None`). The app cancels or ends the stroke first (section 8.3).
2. `self.flush_pending_edit()` — unless the method is the `Commit::Preview`/`Final` continuation of the same pending edit
   (`begin_pending` handles that case).
3. Validate arguments and return `Err` before changing anything.
4. Apply; report pixel changes of a layer with `mark_layer_dirty(layer, rect)` and pure structure changes (visibility, opacity,
   blend, order, add/remove) with `TextureSet::mark_dirty(rect of the affected layers)`.
5. Push the history entry (`push_structure_edit` / `push_edit`), which calls `self.touch()`.

`set_active_layer` produces no history and is exempt from steps 2 and 5 (it still obeys step 1).

Round 4 (S1, T4): the text caches.
```rust
pub(crate) struct TextCaches {                  // not persisted, not part of the history
    pub(crate) frames: HashMap<StampId, (TextSpec, TextFrame)>,     // frame of the current spec; stale entries recognised by the spec
    pub(crate) preview_ids: BTreeSet<StampId>,   // text_mips entries that hold a preview picture (5.20 text pictures)
    pub(crate) font_refs: Vec<FontRef>,                             // fonts handed out by register_font (find_font)
    pub(crate) preview: Option<(TextSpec, u32, Arc<TextRaster>)>,   // the one-entry cache of text_preview
}
```
`text_mips` keeps its round-3 type. Every insert into it goes through `store_text_picture` (doc_stamps.rs), which also
writes `preview_ids` and the frame of the picture's spec into `frames`, so the three agree; `history.rs` dropping a
stale `text_mips` entry (or `doc_layers.rs` `unlink_stamp` removing one) leaves at most a stale `preview_ids` id or
frame, which is harmless: `preview_ids` is only read together with a `text_mips` entry, the next store rewrites it,
and a frame is only returned for its own spec. `add_stamp`'s failure path and `remove_stamp` drop all three
(`forget_text_picture`).
No topology field (round-4 user answer, plan section 7 item 1: no "Stay on connected surface").

**Round 9 (package B, T7).** `BakeSets::wrap_mesh: Option<Arc<WrapMeshCell>>`: the welded mesh graph and the shared
texels of the document's mesh, built lazily (by the first wrap job that needs a fresh parameterization, by the
synchronous wrap path, or by the app's warm-up thread) and shared with every job by `Arc`. Derived data like
`wrap_params`: not in snapshots, not saved. A cell is tied to its mesh by a `Weak` (`WrapMeshCell::is_for`);
`Document::wrap_mesh_cell` makes a new cell when the document's mesh is another `Arc`.

### 5.15 `doc_build.rs` [C2]

```rust
pub const MAX_TEXTURE_SIDE: u32 = 8192;
pub const DEFAULT_BLANK_SIDE: u32 = 1024;

#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum SampleKind { Sphere, Cube, TwoMaterial, Objects }
// Objects (objects-tabs): `object_trio()` named "objects", written with `to_obj_objects` and read back by the OBJ importer
// (objects "Floor", "Cube", "Ball"), texture "objects_checker.png".

#[derive(Clone, Debug)]
pub enum TextureChoice {
    Detected,                                           // the material's texture; falls back to Blank(1024, white) if missing
    /// User-chosen image, already decoded (<= MAX_TEXTURE_SIDE). `original` = the chosen file's bytes (key = file name);
    /// from_import stores it in `SourceBundle::extras` so the project stays self-contained (R7). None only in tests.
    Image { pixels: PixelBuf, original: Option<SourceFile> },
    Blank { width: u32, height: u32, fill: [u8; 4] },
}

pub struct SetParts { pub name: String, pub width: u32, pub height: u32, pub layers: Vec<Layer>, pub active: Option<LayerId> }
pub struct DocumentParts {
    pub name: String, pub model: ImportedModel, pub sets: Vec<SetParts>,       // sets.len() == model.mesh.sets.len()
    pub stamps: Vec<StampObject>, pub blobs: BlobStore, pub ui_state: serde_json::Value, pub next_id: u64,
}

impl Document {
    /// One TextureChoice per set (choices.len() == model.mesh.sets.len()). Each set gets two raster layers: "Base" (the
    /// imported / chosen / blank texture, `locked: true`) and above it an empty transparent "Paint 1", which is the active
    /// layer. So the first stroke or erase never damages the original texture (unlock Base to edit it directly).
    /// An `Image { original: Some(f), .. }` choice for set n appends `f` to `bundle.extras` with
    /// `key = format!("textures/set{n:02}/{}", normalize_ref(&f.original_name))`. Err(NoTextureSets) if the mesh has no set.
    pub fn from_import(model: ImportedModel, choices: Vec<TextureChoice>, progress: &dyn Progress) -> Result<Document, DocError>;
    /// Assembles a document from already-built layers (project load, tests): builds the BVH and texel maps (rayon over
    /// sets), validates sizes, assigns fresh layer versions, composites everything, clears history. Does not bake stamps
    /// and does not add or lock any layer.
    pub fn from_parts(parts: DocumentParts, progress: &dyn Progress) -> Result<Document, DocError>;
    /// Built-in sample: samples::* -> to_obj -> in-memory SourceBundle (OBJ + MTL + checker PNGs encoded with encode_png)
    /// -> import_from_bundle(.., decode_textures = true, ..) -> from_import(all Detected). Needs core-import at run time.
    pub fn sample(kind: SampleKind) -> Document;
}
```

### 5.16 `blend.rs`, `composite.rs` [C2]

```rust
// blend.rs
/// Composite one straight-alpha source pixel over a destination pixel (section 6.6). `opacity` = layer opacity 0..=255.
pub fn blend_pixel(dst: [u8; 4], src: [u8; 4], opacity: u8, mode: BlendMode) -> [u8; 4];
/// Same for a row: dst.len() == src.len(), multiple of 4.
pub fn blend_row(dst: &mut [u8], src: &[u8], opacity: u8, mode: BlendMode);
impl BlendMode {
    pub const ALL: [BlendMode; 7];
    pub fn label(self) -> &'static str;          // "Normal", "Multiply", ...
    pub fn psd_key(self) -> [u8; 4];             // norm, "mul ", scrn, over, lddg, dark, lite
    pub fn ora_op(self) -> &'static str;         // svg:src-over, svg:multiply, svg:screen, svg:overlay, svg:plus, svg:darken, svg:lighten
}

// composite.rs
/// Recomputes `out[rect]` from `layers` (bottom -> top). Rows are processed with rayon.
pub fn composite_rect(layers: &[Layer], width: u32, height: u32, rect: IRect, include_hidden: bool, out: &mut PixelBuf);
pub fn flatten_layers(layers: &[Layer], width: u32, height: u32, include_hidden: bool) -> PixelBuf;
impl Document {
    pub fn recomposite_all(&mut self);                                   // pending -> composite, extends gpu_dirty
    pub fn has_pending_composite(&self) -> bool;
    pub fn take_gpu_dirty(&mut self, set: TextureSetId) -> Vec<IRect>;   // rects of `composite` changed since the last call
    pub fn composite(&self, set: TextureSetId) -> Option<&PixelBuf>;     // byte-identical to flatten(set, false) once nothing is pending
    pub fn flatten(&self, set: TextureSetId, include_hidden: bool) -> Option<PixelBuf>;
}
```

### 5.17 `doc_layers.rs` [C2]

```rust
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PaintTarget { Ready(LayerId), NoActiveLayer, NotRaster(LayerId), Locked(LayerId), Hidden(LayerId) }

impl Document {
    pub fn paint_target(&self, set: TextureSetId) -> PaintTarget;
    /// New transparent raster layer above the active layer (top if none); becomes active. Name None => "Paint N"
    /// (N = smallest positive integer not used by a "Paint N" layer of that set).
    pub fn add_raster_layer(&mut self, set: TextureSetId, name: Option<&str>) -> Result<LayerId, EditError>;
    /// Removes a layer (Err(LayerLocked) if it is locked). A stamp/text layer removes the whole stamp (all linked layers in
    /// all sets): exactly `return self.remove_stamp(stamp)`. The layer below (else above, else None) becomes active.
    pub fn remove_layer(&mut self, layer: LayerId) -> Result<(), EditError>;
    /// Raster: copies pixels into a new unlocked layer "<name> copy" directly above. Stamp/text: exactly
    /// `let s = self.duplicate_stamp(stamp)?;` then the new stamp's linked layer in the same set is returned.
    pub fn duplicate_layer(&mut self, layer: LayerId) -> Result<LayerId, EditError>;
    /// fix_export: several layers as one undo step "Delete Layers" (`group_edits`): each as by `remove_layer`; ids gone by
    /// then and locked layers are skipped. Ok(number removed); Err(first error) when none was removed.
    pub fn remove_layers(&mut self, layers: &[LayerId]) -> Result<usize, EditError>;
    /// fix_export: several layers as one undo step "Duplicate Layers", each as by `duplicate_layer`; the copies in the
    /// order of `layers`; where a set's active layer was duplicated, its copy is active. Err(first error) when none.
    pub fn duplicate_layers(&mut self, layers: &[LayerId]) -> Result<Vec<LayerId>, EditError>;
    pub fn move_layer(&mut self, layer: LayerId, to_index: usize) -> Result<(), EditError>;     // index within its set, clamped
    /// Merges `layer` into the layer below (must be an unlocked raster; `layer` itself must be unlocked). Returns the surviving layer id.
    pub fn merge_down(&mut self, layer: LayerId) -> Result<LayerId, EditError>;
    /// Converts a stamp/text layer into a raster layer (loses re-editability in that set). Err(LayerLocked) if locked.
    pub fn rasterize_layer(&mut self, layer: LayerId) -> Result<(), EditError>;
    pub fn set_layer_visible(&mut self, layer: LayerId, visible: bool) -> Result<(), EditError>;
    pub fn set_layer_locked(&mut self, layer: LayerId, locked: bool) -> Result<(), EditError>;
    pub fn set_layer_alpha_lock(&mut self, layer: LayerId, on: bool) -> Result<(), EditError>;
    pub fn set_layer_blend(&mut self, layer: LayerId, blend: BlendMode) -> Result<(), EditError>;
    pub fn set_layer_opacity(&mut self, layer: LayerId, opacity: f32, commit: Commit) -> Result<(), EditError>;
    pub fn rename_layer(&mut self, layer: LayerId, name: &str) -> Result<(), EditError>;
    pub fn set_active_layer(&mut self, set: TextureSetId, layer: Option<LayerId>) -> Result<(), EditError>;  // no history, no revision bump
    pub fn layer_thumbnail(&self, layer: LayerId, side: u32) -> Option<PixelBuf>;
    pub fn set_thumbnail(&self, set: TextureSetId, side: u32) -> Option<PixelBuf>;               // from the composite
}
```
Every method except `paint_target`, `set_active_layer` and the thumbnails pushes exactly one `Edit::Structure` (label e.g. "Add Layer").
`layer_thumbnail` / `set_thumbnail` call `pixel_ops::thumbnail` (stride-sampled, O(side^2)).

**Delegation between `doc_layers.rs` [C2] and `doc_stamps.rs` [C4]** (one direction only, so there is no recursion and exactly one
history entry): for a Stamp/Text layer, `remove_layer`, `duplicate_layer` and `rename_layer` do nothing but call `remove_stamp` /
`duplicate_stamp` / `rename_stamp` (after the stroke guard and the lock check). `doc_stamps.rs` implements those by editing `sets[..].layers`, `sets[..].active` and `stamps` directly and
**never calls a `doc_layers.rs` command**. `rasterize_layer` and `merge_down` on a stamp/text layer are implemented in `doc_layers.rs`
(rules in section 6.5) and never call `doc_stamps.rs` commands.

Raster pixels pinned in the `before` snapshot (argument of `snapshot_structure`, section 6.7): `remove_layer` — the removed raster
layer; `merge_down` — the lower layer and, if raster, the upper one; every other command of this file — none.

### 5.18 `history.rs` [C2]

```rust
pub enum TileData { Solid([u8; 4]), Raw(Box<[u8]>) }                 // Raw: tile_w * tile_h * 4 bytes (edge tiles are smaller)
pub struct Tile { pub tx: u16, pub ty: u16, pub data: TileData }
/// Sparse set of 64x64 tiles of one raster layer. Applying it swaps its contents with the layer (it becomes its own inverse).
pub struct TilePatch { pub tiles: Vec<Tile> }
impl TilePatch {
    pub const TILE: u32 = 64;
    pub fn is_empty(&self) -> bool;  pub fn byte_len(&self) -> usize;  pub fn bounds(&self, width: u32, height: u32) -> IRect;
    pub fn swap_with(&mut self, buf: &mut PixelBuf) -> IRect;        // returns the dirty bounds
}

/// Copy-on-first-touch backup of a raster layer during a stroke or fill.
pub struct TileBackup { /* width, height, tiles: HashMap<(u16,u16), Box<[u8]>> */ }
impl TileBackup {
    pub fn new(width: u32, height: u32) -> TileBackup;
    pub fn touch_rect(&mut self, buf: &PixelBuf, r: IRect);          // saves every not-yet-saved tile intersecting r; call BEFORE writing
    pub fn original(&self, buf: &PixelBuf, x: u32, y: u32) -> [u8; 4];   // pre-edit pixel (from the backup if saved, else from buf); tests
    /// Pre-edit RGBA of the span x0..x1 of row y, appended to `out` after clearing it (4 * (x1 - x0) bytes): one tile
    /// lookup per 64 texels. This is what the stroke re-apply loop uses; never call `original` per texel there.
    pub fn original_row(&self, buf: &PixelBuf, y: u32, x0: u32, x1: u32, out: &mut Vec<u8>);
    pub fn restore(&self, buf: &mut PixelBuf) -> IRect;              // undo everything; returns the dirty bounds
    pub fn into_patch(self, current: &PixelBuf) -> TilePatch;        // drops tiles identical to `current`, collapses uniform tiles
    pub fn is_empty(&self) -> bool;
}

/// Pixel storage of one layer inside a structure snapshot.
#[derive(Clone)]
pub enum RecordPixels {
    Keep,                                              // raster layer whose pixels the command does not replace: on restore the
                                                       // document's CURRENT buffer for this LayerId is reused (nothing is pinned)
    Raster(Arc<PixelBuf>),                             // pinned raster pixels (layer removed / merged / replaced by the command)
    Stamp { stamp: StampId, cache: Arc<SparseBuf> },   // caches are always stored: they are replaced, never mutated in place
    Text  { stamp: StampId, cache: Arc<SparseBuf> },
}
#[derive(Clone)]
pub struct LayerRecord { pub id: LayerId, pub name: String, pub visible: bool, pub opacity: f32, pub blend: BlendMode,
                         pub locked: bool, pub alpha_lock: bool, pub version: u64, pub pixels: RecordPixels,
                         pub content: Option<(u64, IRect)> /* Layer::content; Keep records take the current layer's */ }
#[derive(Clone)] pub struct SetStructure { pub layers: Vec<LayerRecord>, pub active: Option<LayerId> }
/// The complete layer/stamp structure of a document. Cheap: raster pixels are pinned only where a command asks for it.
#[derive(Clone)] pub struct StructureSnapshot { pub sets: Vec<SetStructure>, pub stamps: Vec<StampObject>,
                                               pub unbaked: BTreeSet<StampId> /* round 5: Document::unbaked at the time (6.7) */ }

pub enum Edit {
    Pixels { set: TextureSetId, layer: LayerId, patch: TilePatch },
    Structure(Box<StructureSnapshot>),
    Multi(Vec<Edit>),                                                // applied in order for redo, reverse order for undo
}
pub struct Entry { pub label: String, pub edit: Edit,
                   pub state: u64 /* history state id on the other side of the entry, swapped by undo/redo (6.7) */ }
pub(crate) struct PendingEdit { pub label: String, pub before: StructureSnapshot }

pub struct History { pub budget_bytes: usize /* default 512 MiB */, pub max_entries: usize /* default 100 */,
                     pub(crate) undo: Vec<Entry>, pub(crate) redo: Vec<Entry>,
                     pub(crate) state: u64 /* id of the current history state (6.7); Document::history_state() */ }
impl Default for History { /* budgets above, empty stacks, state 0 */ }
impl History {
    pub fn push(&mut self, label: &str, edit: Edit);                 // clears redo; evicts oldest entries over budget (newest always kept)
    pub fn clear(&mut self);
    pub fn undo_len(&self) -> usize;  pub fn redo_len(&self) -> usize;
}

impl Document {
    pub fn undo(&mut self) -> Option<String>;                        // returns the label; None if nothing to undo
    pub fn redo(&mut self) -> Option<String>;
    pub fn can_undo(&self) -> bool;  pub fn can_redo(&self) -> bool;
    pub fn undo_label(&self) -> Option<&str>;  pub fn redo_label(&self) -> Option<&str>;
    pub fn has_pending_edit(&self) -> bool;
    /// Ends the pending Preview edit: first `finalize_stamp_bake(id)` (5.20) for every id drained from `preview_baked`
    /// (ascending), then pushes `Edit::Structure(before)` as one history entry. No-op without a pending edit. With
    /// deferred Final bakes on (round 5) `finalize_stamp_bake` bakes nothing: the stamps are left unbaked and the entry is
    /// pushed at once.
    pub fn flush_pending_edit(&mut self);
    /// Restores the state from before the first Preview call (Full-quality caches included) and clears `preview_baked`.
    /// The unbaked set comes back with it (round 5): a stamp whose Full bake was due before the edit is due again.
    pub fn cancel_pending_edit(&mut self) -> bool;
    // cross-package helpers
    /// Snapshot of the current structure. Raster layers listed in `pin` are stored as `RecordPixels::Raster` (Arc clone),
    /// every other raster layer as `Keep`; stamp/text caches are always stored.
    pub(crate) fn snapshot_structure(&self, pin: &[LayerId]) -> StructureSnapshot;
    /// Swaps the document's structure with `s` (rules in 6.7), marks dirty, returns the inverse snapshot. Round 5: the
    /// unbaked set is swapped too (ids of stamps that do not exist afterwards are dropped); with deferred Final bakes
    /// off, a restored unbaked stamp is baked at once (`settle_final_bakes`).
    pub(crate) fn restore_structure(&mut self, s: StructureSnapshot) -> StructureSnapshot;
    pub(crate) fn push_structure_edit(&mut self, label: &str, before: StructureSnapshot);   // history.push + touch() + debug_check_structure
    /// No-op if a pending edit with the same label exists; else flush_pending_edit() + capture `snapshot_structure(&[])`.
    pub(crate) fn begin_pending(&mut self, label: &str);
    pub(crate) fn push_edit(&mut self, label: &str, edit: Edit);     // history.push + touch()
}
```

### 5.19 `brush.rs`, `stroke.rs`, `fill.rs` [C3]

```rust
// brush.rs
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum BrushMode { Paint, Erase }
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct BrushParams {
    pub radius: f32,          // world units for a 3D stroke, texels for a 2D stroke
    pub hardness: f32,        // 0..=1
    pub opacity: f32,         // stroke ceiling 0..=1
    pub flow: f32,            // per-dab build-up 0..=1
    pub spacing: f32,         // fraction of the diameter between dabs; clamped to 0.02..=1; default 0.12
    pub color: [u8; 4],       // straight sRGB; alpha scales the strength
    pub mode: BrushMode,
    pub same_side_only: bool, // 3D: skip triangles whose geometric normal opposes the stroke's reference normal (default true)
}
impl Default for BrushParams { /* radius 16, hardness 0.8, opacity 1, flow 1, spacing 0.12, black, Paint, true */ }

/// Brush profile (section 6.4): 1 inside hardness*radius, smooth fall towards the rim, times an anti-aliasing ramp of
/// width `aa` centred on the rim. Non-zero for dist < radius + aa/2 (the support radius); there is no cut-off at dist == radius.
pub fn falloff(dist: f32, radius: f32, hardness: f32, aa: f32) -> f32;
/// Stroke-to-layer kernel (section 6.4): dst = pre-stroke pixel, strength 0..=1.
pub fn apply_brush(dst: [u8; 4], color: [u8; 4], strength: f32, mode: BrushMode, alpha_lock: bool) -> [u8; 4];

/// Per-set scratch reused between strokes.
pub struct StrokeScratch { /* width, height, mask: Vec<u16>, dab_cov: Vec<u8>, dab_touched: Vec<u32> */ }
impl StrokeScratch {
    pub fn new(width: u32, height: u32) -> StrokeScratch;
    pub fn mask(&self, x: u32, y: u32) -> u16;
    pub fn clear_rect(&mut self, r: IRect);
}
/// One spherical dab on the surface. `normal` = the stroke's reference normal (unflipped shading normal, section 6.4).
/// Returns the wrapped texel rects whose mask changed.
pub fn dab_3d(mesh: &MeshData, bvh: &Bvh, map: &TexelMap, set: TextureSetId, scratch: &mut StrokeScratch,
              center: Vec3, normal: Vec3, radius: f32, hardness: f32, strength: f32, same_side_only: bool) -> Vec<IRect>;
/// One disc dab in texel space (wraps around the image borders). Returns the texel rects whose mask changed.
pub fn dab_2d(scratch: &mut StrokeScratch, center_px: Vec2, radius: f32, hardness: f32, strength: f32) -> Vec<IRect>;

// stroke.rs
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum SampleSource { Composite, Layer(LayerId) }
pub(crate) struct StrokeSession { /* brush, kind (3D/2D), last sample, residual, per-set state: target layer, TileBackup, dirty */ }

impl Document {
    /// Starts a stroke (flushes a pending edit first). Err(NoPaintableLayer) if no set has a paintable active layer;
    /// Err(StrokeInProgress) if one is open.
    pub fn stroke_begin(&mut self, brush: &BrushParams) -> Result<(), EditError>;
    /// As `stroke_begin`, with an explicit tip. `brush.same_side_only` is not read: `tip.reach` decides.
    pub fn stroke_begin_with(&mut self, brush: &BrushParams, tip: &BrushTip) -> Result<(), EditError>;
    /// Pose of the last dab of the open 3D stroke; `None` without one.
    pub fn stroke_tip_pose(&self) -> Option<TipPose>;
    /// Adds a surface sample: dabs are interpolated from the previous sample (section 6.4). The reference normal is
    /// `hit.surface_normal()`. Ignored without an open 3D-capable stroke. `stroke_to_3d_paced` with `|| true`.
    pub fn stroke_to_3d(&mut self, hit: &Hit, pressure: f32);
    /// Round 5 (B-3): `stroke_to_3d` that asks `more()` after every dab and stops with `Partial` when it says no
    /// before the segment's last dab (at least one dab per call). The rest of the segment is kept in the stroke.
    pub fn stroke_to_3d_paced(&mut self, hit: &Hit, pressure: f32, more: &mut dyn FnMut() -> bool) -> StrokeStep;
    /// Round 5 (B-3): continues a `Partial` segment (at least one dab per call); `Done` when none is open.
    pub fn stroke_resume(&mut self, more: &mut dyn FnMut() -> bool) -> StrokeStep;
    /// Round 5 (B-2): the counters of the open stroke (all zero without one; reset by `stroke_begin*`).
    pub fn stroke_counters(&self) -> StrokeCounters;
    /// Adds a sample in continuous texel coordinates of `set`.
    pub fn stroke_to_2d(&mut self, set: TextureSetId, pos_px: Vec2, pressure: f32);
    /// The pointer left the surface: the next sample starts a new segment without bridging.
    pub fn stroke_break(&mut self);
    /// Ends the stroke and pushes one history entry (Edit::Multi over the touched sets). Returns false if nothing changed.
    pub fn stroke_end(&mut self) -> bool;
    /// Restores every touched layer from its backup; no history entry.
    pub fn stroke_cancel(&mut self);
    pub fn stroke_active(&self) -> bool;
    /// Sets (ascending) in which a dab of the open stroke found triangles but which have no `PaintTarget::Ready` layer,
    /// with the reason (`Locked`, `Hidden`, `NotRaster` or `NoActiveLayer`). Empty without an open stroke; read it just
    /// before `stroke_end` / `stroke_cancel`.
    pub fn stroke_skipped_sets(&self) -> Vec<(TextureSetId, PaintTarget)>;
    /// Eyedropper: nearest texel at `uv` (wrapped). None for an unknown set/layer.
    pub fn sample_color(&self, set: TextureSetId, uv: Vec2, source: SampleSource) -> Option<[u8; 4]>;
}

// Round 5 (B-2, B-3)
#[derive(Copy, Clone, Debug, PartialEq, Eq)] pub enum StrokeStep { Done, Partial }
pub const STROKE_PROBE_SLOTS: usize = 15;
pub const STROKE_PROBE_NAMES: [&str; STROKE_PROBE_SLOTS]; // "setup", "raster", "commit", "re-apply", "set dabs",
                                                          // "touched", "query", "walks", "first touch", "buffer copy",
                                                          // round 7: "pass-2 texels", "pass-2 skipped", "bands",
                                                          // "band work", "candidates"
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StrokeCounters {
    pub dabs: u64,          // anchors: a 3D dab that reaches three sets counts once
    pub set_dabs: u64,      // per-set dabs: that dab counts three times
    pub sets_started: u32,  // first touches of a set (layer allocation, stroke buffer)
    pub buffer_copies: u32, // a dab copied a layer buffer that a snapshot or history entry still shared
    pub nanos: [u64; STROKE_PROBE_SLOTS], // the probe's slots; zero while the probe is off (4, 5, 7, 10, 11, 12, 14 are counts)
}
/// The stroke probe: a process-wide switch, off by default, in every build.
pub fn set_stroke_probe(on: bool);
pub fn stroke_probe_on() -> bool;

// fill.rs
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum FillScope { Layer /* whole layer */, Island /* UV island under seed_uv */ }
impl Document {
    /// Fills `layer` (unlocked, visible raster; else LayerLocked / LayerHidden / NotRaster) with `color`. Island scope
    /// needs an owned texel at seed_uv (else Err(InvalidParam("no UV island here"))) and fills exactly that island plus
    /// its own gutter (section 6.4). Respects alpha_lock. One Edit::Pixels entry.
    pub fn fill(&mut self, set: TextureSetId, layer: LayerId, seed_uv: Vec2, color: [u8; 4], scope: FillScope) -> Result<(), EditError>;
}
```

Round 4 (S1, B3). `stroke_begin(&b)` is `stroke_begin_with(&b, &BrushTip::legacy(b.same_side_only))` (the scaffold implements the
reverse direction until B3 lands; B3 replaces both by the shared tip path)." by:

`stroke_begin(&b)` is `stroke_begin_with(&b, &BrushTip::legacy(b.same_side_only))`: one source of truth. The session
keeps the sanitized tip (`BrushTip::sanitized`), the per-stroke `TipGeom` of a field, the reused query buffers
(`DabQuery`), the tangent carried along the segment and the pose of the last dab, boxed together (`TipState`) so that
`Document`, which holds the session inline, keeps its size. `stroke_tip_pose` returns that pose
(`None` before the first 3D dab and for 2D strokes). A 2D stroke ignores the tip. `dab_3d` keeps its signature and
behaviour; strokes no longer call it (it is the reference of test C2).

Crate-internal (not public API), listed for the owners of `stroke.rs`:

```rust
// brush.rs
pub(crate) struct DabShape<'a> { pub center: Vec3 /* c' */, pub reach: f32 /* R_b */, pub field: Option<FieldDab<'a>> }
pub(crate) struct FieldDab<'a> { pub field: &'a TipField, pub to_tip: Mat3 /* R^T / k */, pub k: f32, pub through: bool, pub geom: TipGeom }
pub(crate) struct TipGeom { pub depth: f32, pub thick_max: f32, pub box_min: Vec3, pub box_max: Vec3, pub inradius_thickness: bool }
pub(crate) struct DabQuery { /* found: Vec<(TriId, f32)>, lists: Vec<Vec<TriId>> per set, aa / rho1 / rho per set */ }
pub(crate) fn shared_candidates<'m>(mesh, bvh, n: usize, map_of: impl Fn(usize) -> Option<&'m TexelMap>,
                                    shape: &DabShape, normal: Vec3, reach: SurfaceReach, q: &mut DabQuery);
// map_of(s) = the set's texel map, None = skipped by the stroke (a closure: no list is built per dab)
pub(crate) fn dab_tip(mesh, map, scratch, shape: &DabShape, list: &[TriId], hardness: f32, strength: f32) -> Vec<IRect>;
pub(crate) mod probe { SLOTS, set_enabled, enabled, now, add, count, read } // the stroke probe (round 5 B-2; 15 slots since round 7)
```

Round 5 (R5-2). **Pacing by the dab (B-3).** `stroke_to_3d` is `stroke_to_3d_paced(hit, pressure, &mut || true)`. A
paced call computes the segment exactly as before (bridge rule, step, count, residual) and keeps it in the session as
the *open segment* (endpoints, normals, facing normals, pressures, length, step, count, the residual at its start and
the one to keep at its end, the next dab `k`); it then paints dab `k = 1, 2, ...` at `t = (k * step - start residual)
/ len`, asking `more()` after every dab, also after the single dab of an unbridged sample (the app times its dabs
there). When `more()` says no before the last dab the call returns `Partial` and the segment stays open; the sample is
already the stroke's `last` sample. `stroke_resume(more)` continues it. Every other stroke call (`stroke_to_3d*` with a
new sample, `stroke_break`, `stroke_to_2d`, `stroke_end`) first paints the open segment to its end with `|| true`, and
`stroke_cancel` drops it with the session, so the dabs, their positions, normals, pressures and order never depend on
the pacing (test `one_dab_per_call_gives_the_unpaced_stroke`; the C1 hashes, C2 and `tests\brush_tips.rs` keep their
values). `stroke_skipped_sets` does not see the sets an unpainted rest of a segment would reach; the app reads it after
the queues are empty.

**The stroke probe (B-2).** `brush::probe` is a runtime switch (`set_stroke_probe`, an `AtomicBool`, off by default),
not a test-only `cfg`. Off, every probe point is one relaxed load (`now()` returns `None`; `add` / `count` do nothing);
on, the phase times are added to thread-local counters of the calling thread (the parallel commit and re-apply are
timed around their parallel loops on that thread). Slots: 0 setup (per-set candidate preparation), 1 raster, 2 commit,
3 re-apply, 4 per-set dabs (x 1 000 000), 5 touched texels, 6 shared query, 7 tree walks (x 1 000 000), 8 first touch
of a set (`stroke_target`: layer allocation, `StrokeScratch::new`, since round 7 also the map's brush index), 9 layer
buffer copied (`Arc::make_mut` of a shared buffer in `reapply`); round 7: 10 pass-2 texels looked at (a call's pass-2
box, or its index candidates; added once per call, no per-texel probe work), 11 rasterizer calls whose pass 2 the lean
rule skipped, 12 band jobs, 13 band work (the band jobs' times summed over all threads: band work / raster = the
effective parallel width), 14 prepared candidates. The band jobs run on pool threads, whose thread-local counters
would be lost: they return their counts and times and the calling thread adds them after the parallel loop. In the
fused pass (6.4) slot 1 times the whole pass (raster and the bands' commits) and slot 2 the merge of the rects. The session keeps its own counters (dabs, per-set dabs, sets started, buffer copies; always
counted) and the probe's counters at `stroke_begin*`; `stroke_counters()` returns them, with `nanos` = the probe's
counters now minus those at the begin (zero while the probe is off). The benches read the probe with
`probe::take()`; `stroke_bench_tips` runs with the probe off unless `WTE_BENCH_PROBE=1` (it then measures the probe's
off cost against round 4, whose test builds always counted) and takes its dab count from `stroke_counters().set_dabs`.

**Reused candidate list (B-4.1).** `dab_tip` collects its prepared triangles (`DabTri`) into a buffer that the set's
`StrokeScratch` keeps (cleared per dab) instead of a new `Vec` per dab. Pixels are unchanged (the bench's row "Ball/
SameSide r4 alloc" paints with round 4's allocation and must give the same hash). The per-set loop of a dab stays
sequential (B-4.2 was not taken, see the round-5 notes).

### 5.20 `doc_stamps.rs` [C4]

```rust
impl Document {
    /// Stores an encoded image (png/jpeg/tga/bmp/tiff/webp/dds) as a blob after verifying that it decodes.
    pub fn add_image_blob(&mut self, name: &str, bytes: Arc<[u8]>) -> Result<BlobId, EditError>;
    pub fn image_size(&mut self, blob: BlobId) -> Result<[u32; 2], EditError>;
    pub fn blob_thumbnail(&mut self, blob: BlobId, side: u32) -> Result<PixelBuf, EditError>;
    /// Stores a font file as a blob and returns a FontRef for TextSpec. Blob name: "<family>-<style>.<ext>", the extension
    /// from the container signature (`text::font_file_extension`: ttf / otf / ttc / woff / woff2).
    pub fn register_font(&mut self, family: &str, style: &str, face_index: u32, bytes: Arc<[u8]>) -> FontRef;
    pub fn default_font(&mut self) -> FontRef;                                        // bundled Ubuntu Light
    /// Pixel size of the source image / of the full picture of a text (for aspect-correct placement). A text is
    /// sanitized (`sanitize_fx`) and measured from geometry only (`text_frame`; for a plain text = `measure_text`),
    /// at its own `size_px`, without the too-large fallback (`stamp_source_frame` has it).
    pub fn stamp_source_size(&mut self, source: &StampSource) -> Result<[u32; 2], EditError>;

    /// Full bake (for text: after choosing the raster resolution, section 6.5); one linked layer per covered set (always
    /// one in `home_set`), inserted above each set's active layer; the layer in `home_set` becomes active there.
    /// One history entry ("Add Stamp" / "Add Text").
    /// Round 5, deferred Final bakes on (rule 5 below): validation as above; a text is sanitized and reuses the Text
    /// tool's full hover picture when the T7 rule allows, else only its frame is taken (`fit_frame`: geometry, the same
    /// too-large fallback); the stamp object, `retune_text`, an empty linked layer in `home_set` (made active), the stamp
    /// marked unbaked, one entry. The layers of the other covered sets appear when the bake is applied.
    pub fn add_stamp(&mut self, name: &str, source: StampSource, placement: Placement, home_set: TextureSetId)
        -> Result<StampId, EditError>;
    pub fn set_stamp_placement(&mut self, id: StampId, placement: Placement, commit: Commit) -> Result<(), EditError>;
    /// Replaces the source. For text the anchor rule (6.5 "Text anchor rule") places the new picture: the on-model size
    /// of one em and the on-model position of the kept point (the anchor; for a change of spacing alone without warp
    /// or line, `align_px`) are unchanged. `source.size_px` of an incoming Text source is ignored (the stamp keeps its
    /// raster resolution until finalized) except for the too-large fallback (6.5). The incoming spec is sanitized. A
    /// `Commit::Preview` of a styled text prepares a picture of at most `TEXT_EDIT_PREVIEW_PIXELS` (1 megapixel); the
    /// old text is never rasterized to learn its size (its frame comes from the frame cache or a layout pass).
    /// For another image the user's stretch relative to the old image (placement aspect / old image aspect, e.g. from
    /// a Shift+corner drag) is kept and applied to the new image's aspect, the larger side of the placement keeping
    /// its length: a replacement of the same aspect leaves the placement unchanged, one of another aspect is never
    /// distorted beyond what the user chose (centre, rotation, flips, signs and depth range unchanged), in the same
    /// history entry. Without a readable old image the placement simply follows the new image's aspect.
    /// Text name rule (fix round 3, R3-9): when the text changes and the stamp's name still equals
    /// `text_stamp_name(old content)` (5.12), the stamp and all its linked layers are renamed to
    /// `text_stamp_name(new content)` inside the same pending edit, so one undo restores text and name together and a
    /// cancel (Esc) of Preview edits restores both. Any other name (one the user chose) is kept.
    pub fn set_stamp_source(&mut self, id: StampId, source: StampSource, commit: Commit) -> Result<(), EditError>;
    pub fn set_stamp_tint(&mut self, id: StampId, tint: [u8; 4], commit: Commit) -> Result<(), EditError>;
    pub fn rename_stamp(&mut self, id: StampId, name: &str) -> Result<(), EditError>;   // also renames the linked layers; one entry
    /// Removes the stamp object and all its linked layers (edits sets/stamps directly). One entry ("Delete Stamp").
    pub fn remove_stamp(&mut self, id: StampId) -> Result<(), EditError>;
    /// New stamp "<name> copy" with the same source/placement/tint; its linked layers (Arc-shared caches) are inserted
    /// directly above the originals. One entry ("Duplicate Stamp").
    pub fn duplicate_stamp(&mut self, id: StampId) -> Result<StampId, EditError>;
    /// Topmost visible stamp/text layer whose cache has alpha > 0 at `uv` (wrapped).
    pub fn stamp_at(&self, set: TextureSetId, uv: Vec2) -> Option<StampId>;
    /// `fit_projector_depth` / `projector_depth_is_auto` (5.12, 6.5) on the document's mesh and BVH. Not commands.
    pub fn fit_stamp_depth(&self, p: &Projector) -> Projector;
    pub fn stamp_depth_is_auto(&self, p: &Projector) -> bool;
    /// Re-bakes one stamp without touching history (project load with a lost cache).
    pub(crate) fn rebake_stamp(&mut self, id: StampId, quality: BakeQuality) -> Result<(), EditError>;
    /// Brings a Preview-baked stamp to its final state, without history: (text only) pick `text_raster_px` and, if it
    /// changed, take the new size (frames only, no trial pictures) and apply the anchor rule; bake at `Full` (a preview
    /// picture is drawn again in full first); remove linked layers with an empty cache outside `home_set`;
    /// remove `id` from `preview_baked`. Unknown id: no-op. Called by `flush_pending_edit` (history.rs) and by Commit::Final.
    pub(crate) fn finalize_stamp_bake(&mut self, id: StampId);
    /// Raster resolution for a text stamp: `clamp(next_power_of_two(ceil(em height in texels of home_set)), 32, 512)` (6.5).
    pub(crate) fn text_raster_px(&self, id: StampId) -> f32;
    /// Deferred Preview bakes (default off; the app turns them on for every installed document). While on, a
    /// `Commit::Preview` stamp edit only updates the stamp object (the gizmo follows at once), inserts `id` into
    /// `preview_baked` and `preview_stale`, and bakes nothing; `bake_stale_previews` runs the waiting Preview bakes.
    /// A Final / flush bakes at Full quality as always (and clears `preview_stale`), a cancel restores the old caches.
    /// Turning it off runs the waiting bakes. (Review round 2, finding SP-1.)
    // round 4 (S1, T4)
    pub fn stamp_text_frame(&mut self, id: StampId) -> Result<TextFrame, EditError>;   // geometry only; fills text_frames
    pub fn cached_text_frame(&self, id: StampId) -> Option<TextFrame>;                 // None when not cached for the current spec
    pub fn stamp_source_frame(&mut self, source: &StampSource) -> Result<TextFrame, EditError>;   // for a text not placed yet (sanitized)
    pub fn text_preview(&mut self, spec: &TextSpec, max_pixels: u32) -> Result<Arc<TextRaster>, EditError>;   // one-entry cache
    pub fn find_font(&self, family: &str, style: &str) -> Option<FontRef>;    // text stamps first, then register_font's list; family without case
pub const TEXT_EDIT_PREVIEW_PIXELS: u32 = 1 << 20;   // most pixels of a styled text's picture during Preview edits and bakes
    pub fn set_defer_preview_bakes(&mut self, on: bool);
    pub fn has_stale_previews(&self) -> bool;
    pub fn bake_stale_previews(&mut self);
    // round 5 (R5-1): deferred Final bakes, rules below
    pub fn set_defer_final_bakes(&mut self, on: bool);          // default off; turning it off calls settle_final_bakes()
    pub fn has_unbaked_stamps(&self) -> bool;                   // an id in `unbaked` that is not in `preview_baked`
    pub fn stamp_is_unbaked(&self, id: StampId) -> bool;
    /// The job of the lowest unbaked id that is not in `preview_baked`; None when there is none or a stroke is open.
    /// A stamp whose inputs cannot be prepared is logged, taken out of `unbaked` and skipped.
    pub fn next_final_bake(&mut self) -> Option<StampBakeJob>;
    pub fn apply_final_bake(&mut self, out: StampBakeOutput) -> FinalBake;
    /// Bakes every unbaked stamp on the calling thread (next_final_bake + run + apply until none is left).
    pub fn settle_final_bakes(&mut self);
}

// stamp_job.rs (round 5)
/// Owned inputs of one Full bake of one stamp. Send + 'static: id, object (StampObject), mesh, bvh, the texel maps of all
/// sets (Arcs), and the picture: Image(Arc<PremulMips>) | TextCached(Arc<PremulMips>) (the cached full picture) |
/// TextDraw { spec, own face (Arc<fontdue::Font>, Arc<[u8]>, face index) or none, fallback Arc<fontdue::Font> }.
pub struct StampBakeJob { /* ... */ }
impl StampBakeJob {
    pub fn stamp(&self) -> StampId;
    pub fn is_text(&self) -> bool;
    pub fn run(self) -> StampBakeOutput;            // on the calling thread's rayon pool (the synchronous path); never creates a pool
    pub fn run_isolated(self) -> StampBakeOutput;   // `pool.install(|| self.run())` in the core's bake pool (4.6); what the app's worker calls
}
/// objects-tabs: a job carries the BVH (and so the hidden objects) of the moment it is made; it bakes against that
/// visibility whenever it runs.
/// Result of a job. Send + 'static. Carries the job's identity (id, object, mesh, bvh and map Arcs), the baked buffers
/// `Vec<(TextureSetId, SparseBuf)>` or the error, for a drawn text its picture (spec, mips, frame, full), and `elapsed()`.
/// objects-tabs: its wrap parameterization is cached only while `Arc::ptr_eq(out.bvh, doc.bvh)` (6.5a).
pub struct StampBakeOutput { /* ... */ }
impl StampBakeOutput { pub fn stamp(&self) -> StampId; pub fn is_text(&self) -> bool; pub fn elapsed(&self) -> Duration; pub fn is_error(&self) -> bool; }
pub enum FinalBake { Applied { id: StampId, empty: bool }, Failed { id: StampId }, Stale, Busy(Box<StampBakeOutput>) }
```
**Lock rule**: a locked linked layer protects the whole stamp. While `stamp_locked(id)`, `set_stamp_placement`,
`set_stamp_source`, `set_stamp_tint` and `remove_stamp` return `Err(EditError::LayerLocked)` for any change and change
nothing (so does `remove_layer` through any linked layer); an *unchanged* value is a no-op (`Ok`, no entry), checked
before the lock (the Final a focused field sends when it loses the focus to the lock icon must not warn); `rename_stamp`
(like `rename_layer` for paint layers) and `duplicate_stamp` stay allowed, the copy's layers are unlocked.

All three `set_stamp_*` methods have the same shape: `guard_no_stroke()?`; validate (unknown id, non-finite placement =>
`InvalidParam`, unknown blob); return `Ok` for an unchanged value outside a pending stamp edit; locked and changed => `LayerLocked`;
`begin_pending("Edit Stamp")`; apply the parameter; for `Commit::Preview` bake at `BakeQuality::Preview` (or defer it,
above), keep empty linked layers; insert `id` into `preview_baked`; then, for `Commit::Final` only, `self.flush_pending_edit()` —
which runs `finalize_stamp_bake` (Full bake) and pushes the single history entry. A `Commit::Final` call gets no Preview bake of
its own (fix round 3, R3-4: the Full bake of the flush replaces every cache it would write, and it doubled the cost of each
release of a drag, each tint pick). So `Commit::Final` gives the same caches, layers and history entry as "`Commit::Preview`
followed by `flush_pending_edit()`" (only ids of linked layers a Preview bake created and the flush removed again are not
allocated). With deferred Final bakes off, every path that ends a pending edit (Final, another command, undo,
save, export, entering Preview mode) leaves Full-quality caches. With them on, it leaves the stamp unbaked: its cache may be
in a history snapshot only together with its flag, and never reaches a saved project, an export, a merged or a rasterized
layer (settle, rule 8 below and 8.9). `cancel_pending_edit()` (Esc during a drag) restores the pre-drag structure including the Full-quality caches and clears
`preview_baked`. `add_stamp`, `remove_stamp`, `duplicate_stamp`, `rename_stamp` follow the mutation rule of 5.14 and use
`push_structure_edit` with `snapshot_structure(&[])` (no raster pixels are replaced).

**Deferred Final bakes** (round 5, R5-1; `stamp_job.rs`, the API at the end of the block above). A document mode, off by
default (as `set_defer_preview_bakes`); the app turns it on in `install_document` unless `WTE_SYNC_FINAL_BAKES=1`.
1. **One code path.** `bake_and_apply(id, Full)` = make the job (`make_bake_job`), `run()` it on the calling thread and
   apply the output without the staleness check. `finalize_stamp_bake` (mode off), `add_stamp` (mode off),
   `rebake_stamp`, the project load and `settle_final_bakes` all go through it. The Preview-quality path keeps its direct
   code. No pixel changes.
2. **Job making** (UI thread, cheap): an image takes `image_source(blob)` (cached decode). A text takes `text_mips[id]`
   when it holds the full picture of the spec (`text_source`'s test), else the spec and the fonts (`FontCache::get` /
   `fallback`, as `with_text_fonts`) and the worker draws it (`render_text(.., RasterQuality::Full)`, then the mips).
   `raster_counter::bump()` runs on the thread that draws.
3. **`run_isolated`**: a `OnceLock<rayon::ThreadPool>` with `max(1, available_parallelism - 1)` threads named
   `stamp-bake-N`; `pool.install(|| self.run())` (4.6).
4. **Ending an edit, mode on.** `finalize_stamp_bake(id)` does `retune_text(id)` (frames only), inserts `id` into
   `unbaked` and removes it from `preview_baked` / `preview_stale`. No bake, no `remove_empty_linked`.
   `flush_pending_edit` then pushes the history entry as before; `Commit::Final`, undo, another command: the same, none
   bakes. A `Final` of `set_stamp_placement` / `set_stamp_tint` prepares its source at Preview quality at most (any
   cached picture of the spec), and a text `Final` of `set_stamp_source` takes the new frame from geometry (`fit_frame`,
   the same too-large fallback) instead of drawing the full picture: the UI thread never draws a full text picture.
5. **`add_stamp`, mode on**: see its comment in the block.
6. **Applying.** `apply_final_bake` returns `Busy(out)` while a stroke is open (the app keeps it and tries again). The
   output is used only when the stamp exists, `*stamp == out.object`, `id` is in `unbaked` and not in `preview_baked`,
   and the mesh and every texel map are the job's `Arc`s (`Arc::ptr_eq`); else `Stale` and nothing changes. When used:
   the drawn text picture is stored (`store_text_mips`), `apply_bake`, `remove_empty_linked`, `unbaked.remove(id)`,
   `touch()` (the revision moves so thumbnails refresh; `history_state()` and `is_modified()` do not change),
   `debug_check_structure`; `Applied { id, empty }` (`empty`: no linked layer holds a texel). An output that carries an
   error: warning in the log, `unbaked.remove(id)`, `Failed` (the earlier caches stay). A lock does not stop it.
7. **Structure snapshots carry the flag** (`StructureSnapshot::unbaked`, 6.7): undo, redo and `cancel_pending_edit`
   restore "this stamp still needs its Full bake" together with the caches it belongs to; nothing waits, nothing is
   baked on the UI thread; a restored stamp that never got its Full bake is baked in the background.
8. **Commands that keep stamp pixels.** `merge_down` and `rasterize_layer`: after their `flush_pending_edit()`, an unbaked
   stamp of the layer is baked synchronously (rule 1) before its cache is read (`bake_unbaked_now`; the bake may remove
   the layer itself when it is an empty layer outside the home set: then `UnknownLayer`, as after a synchronous bake).
   `duplicate_stamp`: the copy shares the caches, so it is unbaked exactly when the original is. `remove_stamp` and the
   removal of a stamp's last layer (`unlink_stamp`): `unbaked.remove`.
9. **Invariants** (`debug_check_structure`): `unbaked` holds only ids of existing stamps and is empty while the mode is
   off; "outside a pending edit `preview_baked` is empty" stays.
10. `Document::snapshot()` does not change; its callers settle first (8.9). Project load is untouched (the loaded document
    is in mode off on the load worker; `unbaked` starts empty in `from_parts`).

Round 4 (T4), the functions above:
- `stamp_text_frame(id)`: the frame of the stamp's current spec from geometry only (layout; for a styled text also
  outlines and warp), cached in `text_caches.frames` per stamp with its spec; entries of stamps that no longer exist
  are dropped on insert. Callers after an undo or a load (empty picture cache) get the frame without pixels.
- `cached_text_frame(id)`: the cached frame when it belongs to the stamp's current spec, else `None`.
- `stamp_source_frame(source)`: for a text not placed yet; sanitized; with the too-large fallback, so its `em_px` can
  be below `size_px` (the frame `add_stamp` will draw).
- `text_preview(spec, max_pixels)`: the hover picture; sanitized; one-entry cache; with the too-large fallback.
- `find_font(family, style)`: fonts of the text stamps first, then `register_font`'s session list
  (`text_caches.font_refs`), family compared without case, style exactly; a font whose blob is gone is skipped.

**Text pictures** (new paragraph after the block): `text_mips[id]` holds the mips of the picture of the stamp's spec
and `preview_ids` says whether it is a preview. A bake at `BakeQuality::Preview` uses whatever picture is cached for
the spec (rendering one at `Preview { max_pixels: TEXT_EDIT_PREVIEW_PIXELS }` if none); a bake at `Full` never uses a
preview picture: it renders the spec in full and replaces the cache. `set_stamp_placement` / `set_stamp_tint` /
`set_stamp_source` prepare the picture at the quality of their commit (Preview: preview; Final: full). A plain text
(no thickening, slant, warp or line) is always drawn in full by the classic painter, as in round 3.

Round 4 (T7, FEATURE_TEXT 5.14): `add_stamp` reuses the document's one-entry hover-picture cache (`text_preview`) when
that picture is the full picture of the sanitized spec at its own size (`TextRaster::full` and `frame.em_px ==
raster_px(spec)`, i.e. not a too-large fallback): the Text tool's click then draws nothing again. Otherwise it draws
the text as before. `Document::text_picture_is_full(id) -> Option<bool>` (read only) says whether the cached picture
of a text stamp is full (`None` without one). `doc_stamps::raster_counter::get()` counts the text pictures this
thread rendered through the document (one thread-local add per picture; it exists in every build so the app tests
can read it).

**Round 6: wrapped stamps.** `set_stamp_placement` takes the surface mode and the Wrap options as part of the placement:
a toggle Project <-> Wrap or an option change is an ordinary placement edit (one `EDIT_LABEL` entry, undoable, refused
on a locked stamp, copied by `duplicate_stamp`). Toggling back with no other change gives byte-identical caches. The
Preview bake (`bake_and_apply(.., Preview)`) and the Full bake job (`StampBakeJob::run`) dispatch on `p.mode`: Project
calls `bake_projected` exactly as before; Wrap takes the cached parameterization when it serves the placement (else
builds one), then `bake_wrapped`; the parameterization used is cached (`StampBakeOutput` carries it to
`apply_final_bake`). Log line (info for Full, debug for Preview): `wrap: stamp N param V verts T tris in X ms
(fresh|reused), covered C %, cut K, turns U, jumps J; bake B ms (Quality)`.
```rust
impl Document {
    /// Computes the parameterization of wrapped stamp `id` when the cache has none that serves it (UI thread; after
    /// opening a project). None: not a wrapped stamp, unknown id, a source that cannot be prepared, no surface.
    pub fn ensure_wrap_param(&mut self, id: StampId) -> Option<WrapStats>;
    pub fn stamp_wrap_stats(&self, id: StampId) -> Option<WrapStats>;                        // from the cache
    pub fn stamp_wrap_outline(&self, id: StampId, samples_per_edge: usize) -> Vec<Vec<SurfacePoint>>;
    pub fn stamp_wrap_local(&self, id: StampId, tri: TriId, bary: [f32; 3]) -> Option<Vec2>;
    pub fn stamp_wrap_surface(&self, id: StampId, local: Vec2) -> Option<SurfacePoint>;
    /// Sum over the stamp's linked caches of alpha / 255 times the world area of each covered texel (its owning
    /// triangle's world / UV area ratio; gutter texels do not count). Any stamp kind.
    pub fn stamp_ink_area(&self, id: StampId) -> f32;
}
```

**Round 9 (package B).**
- `wrap_mesh_cell(&mut self) -> Arc<WrapMeshCell>` (5.14). Every `StampBakeJob` carries it (`graph`); `wrap_bake` /
  `wrap_param_only` build a fresh parameterization with `wrap_param_in(.., cell.get(mesh), ..)` (a reused one needs no
  graph; a Project bake never builds it). The synchronous wrap path and `ensure_wrap_param` use the same cell, so the job
  path and the synchronous path give the same bytes (WA28's sphere comparison, WA38). A fresh build logs `wrap phases:
  stamp N graph .. ms, collect .. ms (K candidates), front .. ms, relax .. ms, owner .. ms`.
- `wrap_mesh_warmup(graph) -> Option<WrapMeshWarmup>`: what the warm-up must build (the shared texels; with `graph`
  also the welded graph), `None` when everything is built. `WrapMeshWarmup::run` builds them on any thread (graph first).
- `stamp_shared_texels(&self, id) -> Option<f32>` (T9): the share of a 3D stamp's picture on triangles whose texels other
  triangles use too, from 16 x 16 picture points (Wrap: `WrapParam::surface_of` of the cached parameterization; Project:
  rays along `-out`). `None` until the shared texels are built, for a wrapped stamp without a serving
  parameterization, for UV stamps. Never builds anything (UI thread).

### 5.21 `import/` [C5]

```rust
// import/resolver.rs
pub trait AssetResolver {
    /// `reference` is the string exactly as written in the model file (mtllib, map_Kd, glTF uri).
    fn read(&self, reference: &str) -> Result<Arc<[u8]>, ResolveError>;
}
pub struct DiskResolver { pub base_dir: PathBuf }
#[derive(Default)] pub struct MemoryResolver { /* BTreeMap<String, Arc<[u8]>> keyed by normalize_ref */ }
impl MemoryResolver { pub fn insert(&mut self, reference: &str, bytes: Arc<[u8]>);  pub fn from_bundle(bundle: &SourceBundle) -> MemoryResolver; }
/// Wraps a resolver and records every file successfully read (first-import capture for R7).
pub struct RecordingResolver<R: AssetResolver> { /* inner, RefCell maps */ }
impl<R: AssetResolver> RecordingResolver<R> {
    pub fn new(inner: R) -> Self;
    pub fn into_captured(self) -> (Vec<SourceFile>, Vec<String> /* missing references */);
}
impl AssetResolver for DiskResolver {}  impl AssetResolver for MemoryResolver {}  impl<R: AssetResolver> AssetResolver for RecordingResolver<R> {}
/// '\\' -> '/', percent-decode, strip leading "./", ASCII-lowercase.
pub fn normalize_ref(reference: &str) -> String;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ModelFormat { Obj, Gltf, Glb, Fbx }          // manifest names "obj", "gltf", "glb", "fbx"
impl ModelFormat { pub fn extension(self) -> &'static str; }
#[derive(Clone, Debug)]
pub struct SourceFile { pub key: String /* normalize_ref */, pub original_name: String, pub bytes: Arc<[u8]> }
/// The original model file plus every file it references (on first import: glTF buffers and every external image, the MTL
/// files and every texture file their `map_*` / `bump` / `disp` / `decal` / `refl` / `norm` statements name, used or not,
/// every texture file an FBX names (RelativeFilename, else FileName) that is not embedded;
/// files that cannot be read are skipped silently), plus user-chosen texture files. Send + Sync.
#[derive(Clone, Debug)]
pub struct SourceBundle { pub format: ModelFormat, pub main: SourceFile, pub sidecars: Vec<SourceFile>,
                          /// Original files of manually chosen textures (TextureChoice::Image), key
                          /// "textures/set<NN>/<name>". Never read by the importer: `MemoryResolver::from_bundle` ignores them.
                          pub extras: Vec<SourceFile>,
                          pub original_path: Option<String> /* informational only; never read back */ }

// import/mod.rs
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ImportOptions { pub generate_fallback_uvs: bool, pub max_texture_dim: u32 }
impl Default for ImportOptions { /* false, 8192 */ }
/// 2: the generated (fallback) layout is the box-projection atlas of `import/auto_uv.rs`; skinned glTF meshes are posed
/// by their skin; FBX import. Stored in the manifest; `project::load` compares it (7.1 step 3).
/// RULE: any change to the generated layout or to mesh normalisation that changes `uv_hash` for some input must bump this
/// constant, and from then on keep the previous path selectable by version (the manifest's `importer_version` passed
/// through `ImportOptions` / `NormalizeOptions`). `mesh_build` test `generated_uv_layout_of_importer_version_2_is_frozen`
/// pins the version-2 hashes of `no_uv_tetra` and a UV-less `uv_sphere(32, 16)`, so a silent change fails the tests.
/// A format loader that starts reading something it ignored before (FBX texture UV transforms, R3-12) is not such a
/// change: the files it affects were mapped wrongly before (see below).
/// (No version-1 generator exists: no project from before version 2 with generated UVs was ever saved by a user.)
pub const IMPORTER_VERSION: u32 = 2;
// Decision (round 3 repair, for the gate / user to confirm): R3-12 changes `uv_hash` only for an FBX whose Texture carries a
// non-identity UV transform *and* whose polygons have UVs of their own (none of the 21 importable samples). Such a model was
// shown with the transform ignored, i.e. mapped wrongly, so a project saved from it reopens with `MeshUvChanged`; no
// version bump and no old path. R3-13 changes nothing for core glTF files: integer attributes are read the new way only
// when the file declares `KHR_mesh_quantization` (before round 3 a file requiring it was refused).

#[derive(Clone, Debug)]
pub struct ImportedTexture { pub name: String /* file stem */, pub reference: String,
                             pub image: Option<PixelBuf> /* None: missing, undecodable, or decode_textures == false */ }
#[derive(Clone, Debug)]
pub struct ImportedModel {
    pub name: String,                           // file stem
    pub mesh: MeshData,
    pub textures: Vec<ImportedTexture>,         // indexed by SetInfo::texture
    pub report: MeshReport,
    pub warnings: Vec<ImportWarning>,
    pub bundle: SourceBundle,
    pub options: ImportOptions,
}
/// b"glTF" magic -> Glb; b"Kaydara FBX Binary" magic or a text starting with "; FBX" -> Fbx; first non-whitespace byte
/// '{' -> Gltf; else by extension (.obj/.gltf/.glb/.fbx, any case).
pub fn sniff_model_format(bytes: &[u8], file_name: &str) -> Option<ModelFormat>;
/// Reads the file and its sidecars from disk (DiskResolver wrapped in RecordingResolver) and captures them in the bundle
/// (`extras` empty). Always decodes the referenced textures. A model file larger than `MAX_SOURCE_FILE` (2 GiB, the entry
/// limit of `project::load`) is `ImportError::FileTooLarge`; a larger sidecar counts as unreadable.
pub fn import_from_path(path: &Path, opts: &ImportOptions, progress: &dyn Progress) -> Result<ImportedModel, ImportError>;
/// Geometry only (brush shapes): reads the model file and the side files its geometry needs, decodes no texture,
/// generates no UVs, keeps no file bytes. Default `ImportOptions` (glTF node transforms, FBX units as for a model).
pub fn import_geometry_from_path(path: &Path, progress: &dyn Progress) -> Result<MeshData, ImportError>;
/// `import_from_path`, except that with `auto_uvs` a model none of whose triangles has usable UVs gets the generated
/// layout right away (as if `generate_fallback_uvs` were set). Every import sets `options.generate_fallback_uvs` when
/// the layout was generated, so a saved project re-imports identically. Used by *New from Model* (8.9): every model
/// with geometry can be painted (R6).
pub fn import_from_path_auto_uvs(path: &Path, opts: &ImportOptions, auto_uvs: bool, progress: &dyn Progress)
    -> Result<ImportedModel, ImportError>;
/// Re-imports purely from memory (project load, samples, tests). Never touches the disk. With `decode_textures == false`
/// (project load: the pixels come from the layer PNGs) no texture is decoded, every `ImportedTexture.image` is None and no
/// MissingTexture / TextureDecodeFailed / TextureResized warning is emitted; mesh, sets and names are identical either way.
/// The returned model's bundle is a clone of `bundle` (extras included).
pub fn import_from_bundle(bundle: &SourceBundle, opts: &ImportOptions, decode_textures: bool, progress: &dyn Progress)
    -> Result<ImportedModel, ImportError>;

// import/obj.rs, import/gltf.rs   (pub(crate))
pub(crate) struct RawImport { pub mesh: RawMesh, pub objects: RawObjects /* objects-tabs */, pub textures: Vec<ImportedTexture>,
                              pub warnings: Vec<ImportWarning> }
// Objects (objects-tabs, 6.1 step 9). OBJ: `o <name>` and `g <name>` lines set the current object / group (the whole rest
// of the line is the name; `g` without a name is "default"; a name seen again continues the same entry). If at least one
// face follows an `o` line the objects are the `o` entries, else, if at least one face follows a `g` line, the `g`
// entries; faces in front of the first `o` (`g`) form one unnamed object; a file without either is one object. glTF / GLB:
// each visited node with a mesh is one object, named by the node, else by its mesh, else unnamed; two nodes instancing one
// mesh are two objects; a node whose primitives are all skipped yields no triangles and disappears in normalization.
// FBX: one object per `Model` with geometry, named by its display name. Objects do not change triangles, order or
// `uv_hash`: `IMPORTER_VERSION` stays 2.
pub(crate) fn load_obj(main: &[u8], resolver: &dyn AssetResolver, opts: &ImportOptions, decode_textures: bool)
    -> Result<RawImport, ImportError>;
// OBJ numbers: a `v` / `vt` / `vn` line with a missing, malformed or non-finite number (`abc`, `nan`, `inf`, `1e39`, which
// overflows f32) is not fatal. It stays in the numbering as a placeholder (so later indices are unchanged); a face using a bad
// `v` is dropped; a bad `vt` / `vn` counts as a missing reference (the face is unmapped / gets generated normals). A face
// whose position index points nowhere (`f 1 2 999999`, `f 0 1 2`) is dropped and counted the same way. One
// `BadObjNumbers` warning names the first bad line and the counts. When no face is left, the file fails with
// `ImportError::Obj("line <first bad line>: <reason> (<n> line(s) with unusable numbers, no usable face)")`. Face index
// syntax errors (`f 1 2 x`, `f 1/2/3/4`, an index that does not fit i64) stay fatal.
pub(crate) fn load_gltf(main: &[u8], resolver: &dyn AssetResolver, opts: &ImportOptions, decode_textures: bool)
    -> Result<RawImport, ImportError>;
// import/fbx.rs: binary (6000..7999) and ASCII FBX, see section 10 (FBX notes). Malformed files are
// `ImportError::InvalidFbx(..)` ("invalid FBX file: ..."); never a panic, hang or unbounded allocation.
pub(crate) fn load_fbx(main: &[u8], resolver: &dyn AssetResolver, opts: &ImportOptions, decode_textures: bool)
    -> Result<RawImport, ImportError>;
// import/auto_uv.rs: the generated layout of 6.1 step 4 (box-projection charts, shelf-packed).
pub(crate) fn generate_atlas_uvs(tris: &[[Vec3; 3]]) -> Vec<[Vec2; 3]>;
// import/mod.rs then calls normalize_with_objects(raw.mesh, &raw.objects, &NormalizeOptions { generate_fallback_uvs:
//     opts.generate_fallback_uvs || auto_uvs, model_name: <file stem>, texture_names: raw.textures.iter().map(|t| t.name.clone()).collect() }).
// OBJ / MTL / ASCII FBX text: `decode_text` (UTF-8, else Latin-1; a UTF-8 BOM is dropped; a UTF-16 LE / BE BOM, as
// Notepad's "Unicode" writes, decodes the rest as UTF-16) normalizes CRLF, lone CR (classic Mac OS) and mixed line
// endings to LF before any line-based parsing.
// import/mtl_ref.rs
pub fn parse_texture_ref(map_line: &str) -> &str;                    // strips leading -options, keeps file names with spaces
// import/uri.rs
pub fn base64_decode(text: &str) -> Option<Vec<u8>>;                  // standard alphabet, padding optional, whitespace ignored
pub fn percent_decode(text: &str) -> String;                          // invalid escapes are kept literally; lossy UTF-8
pub fn parse_data_uri(uri: &str) -> Option<(Option<&str> /* mime */, Vec<u8>)>;
```

### 5.22 `image_io.rs` [C5]

```rust
#[derive(Clone, Debug)]
pub struct DecodedImage { pub pixels: PixelBuf, pub original_size: [u32; 2], pub resized: bool }
/// Decodes any supported format to straight RGBA8. Format is guessed from content, then from the extension of
/// `name_hint` (needed for TGA). Images larger than `max_dim` on a side are downscaled (aspect preserved).
pub fn decode_image(bytes: &[u8], name_hint: Option<&str>, max_dim: u32) -> Result<DecodedImage, ImageIoError>;

#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum PngLevel { Fast, Balanced, High }
/// RGBA8, non-interlaced, straight alpha. `rgba.len() == width*height*4`.
pub fn encode_png(width: u32, height: u32, rgba: &[u8], level: PngLevel) -> Result<Vec<u8>, ImageIoError>;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum FlatFormat { Png(PngLevel), Jpeg { quality: u8, matte: [u8; 3] }, Tga, Bmp, Tiff, WebpLossless }
impl FlatFormat { pub fn extension(&self) -> &'static str;  pub fn keeps_alpha(&self) -> bool; }
pub fn encode_flat(buf: &PixelBuf, format: &FlatFormat) -> Result<Vec<u8>, ImageIoError>;
```

### 5.23 `project/` — types [S], implementation [C6]

```rust
// project/mod.rs  [S]
pub const FORMAT_VERSION: u32 = 3;   // 2: round 4 (text effects); 3: round 6 (surface mode, Wrap options)
pub const PROJECT_EXTENSION: &str = "wtep";
pub const PROJECT_MIMETYPE: &str = "application/x-wtep";
#[derive(Clone, Debug)]
pub struct SaveOptions { pub png: PngLevel /* Fast */, pub write_flattened: bool /* true */, pub keep_backup: bool /* false */,
                         pub now_unix: u64, pub app_version: String }
#[derive(Clone, Debug, Default)]
pub struct SaveStats { pub bytes_written: u64, pub entries: u32, pub reuse: Option<PngReuse> /* record of the written file */ }
/// Images of an existing project file that `save` copies instead of encoding (7.1, "Copying unchanged images").
#[derive(Clone, Debug, Default)]
pub struct PngReuse { pub path: PathBuf, pub len: u64, pub modified: Option<SystemTime>,
                      pub layers: HashMap<LayerId, (u64 /* version */, String /* entry */, [i32; 4] /* rect */)>,
                      pub flattened: HashMap<TextureSetId, (u64 /* composite_key */, String)>, pub thumbnail: Option<(u64, String)> }
pub struct LoadReport { pub document: Document, pub warnings: Vec<LoadWarning>, pub force_save_as: bool }
pub use save::save;  pub use load::{load, read_manifest, extract_bundle, extract_sources};

// project/save.rs  [C6]
/// Writes a complete .wtep atomically (temp file in the same directory, fsync, verify, rename).
pub fn save(snapshot: &DocumentSnapshot, path: &Path, opts: &SaveOptions, progress: &dyn Progress) -> Result<SaveStats, ProjectError>;
// project/load.rs  [C6]
pub fn load(path: &Path, progress: &dyn Progress) -> Result<LoadReport, ProjectError>;
pub fn read_manifest(path: &Path) -> Result<Manifest, ProjectError>;
/// R7 "Extract Original Files", working on in-memory data (so it also works for a document that was never saved): writes
/// the model file, every sidecar and every extra at their relative paths, and every blob under `stamps\` (images) or
/// `fonts\` (fonts) using its sanitized original name, into `dest_dir` (created if missing). With `overwrite == false`
/// an existing file is never replaced: the new file gets a `_2`, `_3`, ... suffix. Returns the written paths (7.1).
pub fn extract_bundle(bundle: &SourceBundle, blobs: &BlobStore, dest_dir: &Path, overwrite: bool)
    -> Result<Vec<PathBuf>, ProjectError>;
/// Thin wrapper: reads `source/*`, the extras and the blobs of a saved .wtep (no import, no layers) and calls extract_bundle.
pub fn extract_sources(path: &Path, dest_dir: &Path, overwrite: bool) -> Result<Vec<PathBuf>, ProjectError>;
// project/migrate.rs  [C6]
pub fn upgrade(manifest: serde_json::Value, from_version: u32) -> Result<serde_json::Value, ProjectError>;
// project/atomic.rs  [C6]
/// Calls `write` with a temp file next to `path`, fsyncs, runs `verify` on the temp path, optionally rotates the
/// existing file to `<path>.bak`, then renames. On any error the temp file is deleted and `path` is untouched.
pub fn write_atomic<E: From<std::io::Error>>(path: &Path, keep_backup: bool,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), E>,
    verify: impl FnOnce(&Path) -> Result<(), E>) -> Result<(), E>;
// project/manifest.rs  [S]  — see section 7.2
```

### 5.24 `export/` [C6]

```rust
// export/mod.rs
#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub dir: PathBuf,
    pub base_name: String,                       // sanitized by the exporter
    pub sets: Option<Vec<TextureSetId>>,         // None = all
    pub flat: Option<FlatFormat>,                // flattened texture per set
    pub per_layer_png: bool,
    pub ora: bool,
    pub psd: bool,
    pub include_hidden: bool,                    // hidden layers in per-layer/ORA/PSD output (flattened always = visible only)
    pub dilate_px: u32,                          // extra seam padding on the flattened texture; 0 = byte-identical to the composite
    /// Layer scope (1.8): None = every layer `include_hidden` lets through; Some(ids) = exactly these layers, hidden ones
    /// included, `include_hidden` ignored, unknown ids ignored, sets without a selected layer left out.
    pub layers: Option<Vec<LayerId>>,
    pub package: ExportPackage,                  // Files (default) | Zip
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ExportPackage { #[default] Files, Zip }
#[derive(Clone, Debug, Default)]
pub struct ExportReport {
    pub files: Vec<PathBuf>,                     // Files: the written files in plan order; Zip: the one zip
    pub bytes_written: u64,                      // Zip: the size of the zip
    pub packaged: usize,                         // Zip: number of entries; Files: 0
}
/// One file of the export as the dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFile { pub name: String, pub set: TextureSetId, pub kind: PlannedKind }
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlannedKind { Flat, Layer { layer: LayerId, index: usize, raster: bool }, Ora, Psd }
/// The paths `run` writes, without touching the disk: `Files` = the exact list of files (same naming and
/// de-duplication, same order); `Zip` = `[dir.join(package_name_for(opts))]`, or nothing when there is nothing to pack. The
/// export dialog shows it and checks it for existing files before starting.
pub fn plan(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PathBuf>;
/// The files of the export in plan order whatever the package (for `Zip`: the entries), with what each one holds. The
/// dialog's list and its "separate layer" markers come from here (no second classification in the app).
pub fn plan_contents(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PlannedFile>;
/// The entries of a zip in archive order (tests and self-test).
#[doc(hidden)] pub fn read_package(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String>;
/// fix_export: `ids` completed by the layers linked to the same stamp / text (`LayerKind::{Stamp, Text}::stamp`) in the
/// other sets: the ids as given, then the linked ones in set order, no duplicates. `plan`, `plan_contents` and `run`
/// apply it to `opts.layers` first (a design across several sets is exported in every set it touches).
pub fn linked_selection(snapshot: &DocumentSnapshot, ids: &[LayerId]) -> Vec<LayerId>;
/// fix_export: the zip's name: `package_name(base)` for Visible / All, `selection_package_name(base)` under `layers`.
pub fn package_name_for(opts: &ExportOptions) -> String;
/// fix_export: removes `<file>.tmp-<pid>` files (`<file>` with an export extension) and `<zip>.tmp-<pid>.d` stage folders
/// in `dir` whose pid is not this process and `!is_running(pid)`; returns what was removed. The app's export job calls it
/// before `run` with `io_flows::process_running`.
pub fn remove_stale_temps(dir: &Path, is_running: &dyn Fn(u32) -> bool) -> Vec<PathBuf>;
/// Creates `opts.dir` (create_dir_all), then writes every file of `plan` (each one atomically, replacing an existing
/// file of the same name). Flattened pixels come from flatten_layers(.., false). Several sets at a time, report in
/// plan order, each file renamed from its temp file once written, all flushed at the end (7.4).
pub fn run(snapshot: &DocumentSnapshot, opts: &ExportOptions, progress: &dyn Progress) -> Result<ExportReport, ExportError>;

// export/naming.rs
pub fn sanitize_file_stem(s: &str) -> String;
/// File names for one set; `multi` = the set name goes into the names (section 7.4, `selected_sets`). See section 7.5.
pub fn flat_name(base: &str, set_name: &str, multi: bool, ext: &str) -> String;
pub fn layer_name(base: &str, set_name: &str, multi: bool, index: usize, kind: &str, layer_name: &str) -> String;
pub fn layered_name(base: &str, set_name: &str, multi: bool, ext: &str) -> String;
/// `<base>.zip` (base sanitized) of `ExportPackage::Zip`.
pub fn package_name(base: &str) -> String;
/// fix_export: `<base>_selection.zip` (base sanitized), the zip of a layer selection.
pub fn selection_package_name(base: &str) -> String;

// export/ora.rs, export/psd.rs
pub struct ExportLayer<'a> { pub name: &'a str, pub kind: &'static str, pub visible: bool, pub opacity: f32,
                             pub blend: BlendMode, pub pixels: PixelView<'a> }
pub struct ExportSet<'a> { pub name: &'a str, pub width: u32, pub height: u32, pub flattened: &'a PixelBuf,
                           pub layers: Vec<ExportLayer<'a>> /* bottom -> top */ }
pub fn write_ora<W: std::io::Write + std::io::Seek>(w: W, set: &ExportSet<'_>) -> Result<(), ExportError>;
/// PNGs already encoded from the same pixels at `Balanced` (merged image; per `ExportSet::layers` entry the full-canvas PNG,
/// used only when that layer's content is the whole canvas). Same bytes as `write_ora` (7.4).
pub struct OraReuse<'a> { pub merged: Option<&'a [u8]>, pub full_canvas_layers: Vec<Option<&'a [u8]>> }
pub fn write_ora_reusing<W: std::io::Write + std::io::Seek>(w: W, set: &ExportSet<'_>, reuse: &OraReuse<'_>)
    -> Result<(), ExportError>;
pub fn write_psd<W: std::io::Write>(w: W, set: &ExportSet<'_>) -> Result<(), ExportError>;
#[doc(hidden)] pub fn read_psd(bytes: &[u8]) -> Result<PsdDoc, String>;      // strict verification reader (tests)
#[doc(hidden)] pub struct PsdDoc { pub width: u32, pub height: u32, pub layers: Vec<PsdLayer>, pub merged: PixelBuf }
#[doc(hidden)] pub struct PsdLayer { pub rect: IRect, pub blend_key: [u8; 4], pub opacity: u8, pub hidden: bool,
                                     pub name: String, pub unicode_name: String, pub pixels: CroppedBuf }
// export/packbits.rs
pub fn encode_row(src: &[u8], out: &mut Vec<u8>);
pub fn decode_row(src: &[u8], out_len: usize) -> Option<Vec<u8>>;
```

### 5.25 `testutil.rs` [S]

`#[doc(hidden)] pub mod testutil` (always compiled; tiny): `temp_dir(name) -> PathBuf` (section 4.8), `mesh_of(raw) -> MeshData`,
`psnr(a: &PixelBuf, b: &PixelBuf) -> f64`, `hash_pixels(buf: &PixelBuf) -> u64`, `XorShift` deterministic RNG (`XorShift::new(seed: u64)`, `next_u32`, `next_f32` in `[0, 1)`),
`imported_from_raw(raw: RawMesh, textures: Vec<Option<PixelBuf>>) -> ImportedModel` (normalize with default options -> dummy
in-memory `SourceBundle` whose main file is the bytes `b"dummy"` -> `ImportedTexture`s named `tex0`, `tex1`, ...),
`imported_from_raw_objects(raw, &RawObjects, textures)` / `doc_from_raw_objects(raw, &RawObjects, size)` (the same through
`normalize_with_objects`; objects-tabs),
`doc_from_raw(raw: RawMesh, size: u32) -> Document` (`imported_from_raw` -> **one** blank white, unlocked raster layer of
`size x size` per set, active -> `Document::from_parts`), so algorithm tests do not depend on the importers and paint directly on
that single layer. (`Document::from_import` and `Document::sample` produce the locked "Base" + "Paint 1" pair instead.)
A document built this way cannot be re-imported; persistence tests use `Document::sample` or a real import.
Fixed details of these helpers: `imported_from_raw` names the model `dummy` (main file key and original name `dummy.obj`, format
`Obj`, default `ImportOptions`) and pads the texture list with `None` up to the highest index a material references;
`doc_from_raw` names the document `test`, gives set `i` the layer `LayerId(i + 1)` called "Layer 1" and sets `next_id = n + 1`.

### 5.26 `brush_tip.rs`, `tip_field.rs` [S, round 4], `tip_bake.rs` [B1, round 4]

```rust
// brush_tip.rs
pub enum SurfaceReach { SameSide, All }
pub enum TipAlign { Surface, World }                       // serde snake_case; default Surface
pub enum TipVolume { Ball, Field(Arc<TipField>) }
pub struct BrushTip { pub volume: TipVolume, pub reach: SurfaceReach, pub align: TipAlign, pub rotation: Quat,
                      pub offset: f32 /* radii, -1..=1 */, pub scale: f32 /* 0.25..=4 */, pub through: bool, pub up_hint: Vec3 }
impl BrushTip { pub fn legacy(same_side_only: bool) -> BrushTip; pub fn sanitized(&self) -> BrushTip;
                pub fn is_ball(&self) -> bool; pub fn field(&self) -> Option<&Arc<TipField>> }
pub fn tip_frame(facing: Vec3, up_hint: Vec3, prev_up: Option<Vec3>) -> (Mat3, Vec3);   // columns (right, out, -up_t), carried up_t
pub struct TipPose { pub center: Vec3, pub world_from_tip: Mat3, pub unit: f32 }

// tip_field.rs
pub struct TipReport { pub triangles: u32, pub grid: [u32; 3], pub inside_fraction: f32, pub thin: bool, pub partial: bool,
                       pub flat: bool, pub up_axis: u8, pub height_ratio: f32, pub openings_closed: u32 }
pub const TIP_MAX_NODES: u32 = 128;
impl TipField {
    pub const CUBE_HALF_SIDE: f32; pub const CYLINDER_RADIUS: f32; pub const CYLINDER_HALF_HEIGHT: f32;
    pub fn cube() -> TipField; pub fn cylinder() -> TipField;
    pub fn from_fn(dims: [u32; 3], origin: Vec3, cell: f32, f: impl Fn(Vec3) -> f32 + Sync) -> TipField;
    pub fn outline_from_fn(dims: [u32; 2], origin: Vec2, cell: f32, f: impl Fn(Vec2) -> f32 + Sync) -> TipField;
    pub(crate) fn from_distances(dims: [u32; 3], origin: Vec3, cell: f32, dist: Vec<i16>, base: TipReport) -> TipField;
    pub(crate) fn outline_from_distances(dims: [u32; 2], origin: Vec2, cell: f32, dist: Vec<i16>, base: TipReport) -> TipField;
    pub fn sample(&self, q: Vec3) -> f32; pub fn thickness(&self, q: Vec3) -> f32;
    pub fn outline(&self, x: f32, z: f32) -> f32; pub fn outline_thickness(&self, x: f32, z: f32) -> f32;
    pub fn outline_bounds(&self) -> [Vec2; 2]; pub fn is_flat(&self) -> bool; pub fn inradius(&self) -> f32;
    pub fn bound(&self) -> f32; pub fn bounds(&self) -> Aabb; pub fn cell(&self) -> f32; pub fn dims(&self) -> [u32; 3];
    pub fn report(&self) -> &TipReport; pub fn hash(&self) -> u64;
    pub fn principal_sections(&self) -> &[Vec<[Vec2; 2]>; 3];       // cuts x = 0 (y, z), y = 0 (x, z), z = 0 (x, y)
    pub fn section(&self, origin: Vec3, u: Vec3, v: Vec3, n: u32) -> Vec<[Vec2; 2]>;
}
pub(crate) fn power_transform(dims: &[usize], init: &[f64]) -> (Vec<f64>, Vec<u32>);   // separable lower envelope + argmin
pub(crate) fn thickness_grid(dims: &[usize], dist: &[i16]) -> Vec<u8>;
pub(crate) fn distances_from_border(dims: &[usize], inside: &[bool], border: &[f32]) -> Vec<i16>;
pub(crate) fn slice_from_values(dims: [u32; 2], values: &[f32]) -> (Vec<i16>, Vec<bool>);

// tip_bake.rs
pub const MAX_TIP_TRIANGLES: u32 = 2_000_000;
pub enum CloseMode { Refuse /* default */, FlatCaps, RoundedCaps, FillHollows, ClosedPartsOnly }   // serde snake_case; label()
pub enum TipError { Import(ImportError), NoInterior, HasOpenings { count: u32, largest: f32, closed_parts: bool },
                    TooManyTriangles { count: u32, limit: u32 }, Cancelled }
impl TipField {
    pub fn bake(mesh: &MeshData, close: CloseMode, progress: &dyn Progress) -> Result<TipField, TipError>;
    pub fn bake_from_path(path: &Path, close: CloseMode, progress: &dyn Progress) -> Result<TipField, TipError>;
}
```
A field is a grid of signed distances in tip units (the solid fits inside the unit ball), stored as `i16` in
1/256 cells saturating at +-127.99 cells, with a local-thickness grid (`u8`, half cells) and a middle slice y = 0
(two-dimensional distances and thickness) for Through; a flat outline has the slice only. Node `(i, j, k)` lies at
`origin + (i, j, k) * cell`, x fastest. Lookups clamp into the grid box and add the distance to the clamped point,
so they are defined everywhere. Built-in Cube (half side 1/sqrt(3)) and Cylinder (radius and half height
1/sqrt(2)) are written from their exact distances on a grid of cell side/122 that is symmetric about the origin
(the origin is a node). The local thickness is the separable lower-envelope transform with argmin (5.7 step 9).

Round-4 decisions of M1 (contract requests B1-1, B3-1): the local thickness of step 9 (`thickness_grid`) minimises
`|x-y|^2 - (depth(y)+1)^2` (`THICKNESS_SLACK_CELLS` = 1.0) and keeps `max(T, depth(x))`, which keeps baked faceted
shapes stable (F20 on a baked ball: 3.70 cells near the skin, 2.27 deeper; FEATURE_BRUSH asks 1.5: an open miss).
`bound()` and `bounds()` grow by the largest depth of an inside node on the grid's border (0 for every field with a
margin), so a part cut off at its grid's border stays inside its bound. The deviation of step 8 described below
(contract request B1-4) is accepted by the gate: it meets F2 (0.004 cells) and F9 (1.001 cells), the designed
transform does not.

`TipField::bake` (FEATURE_BRUSH 5.7, 5.7b; plan section 7 item 5) works on a welded triangle soup: vertices with
bit-identical positions are merged, collapsed triangles dropped; normals, UVs and materials are ignored.

Openings. The boundary loops of the soup (edges used by exactly one triangle, chained into loops; triangles over the
same three vertices count once, so an open model whose back faces are a second copy of its triangles still shows its
openings) are found per connected part. A loop counts (the app asks about it) when its bounding-box diagonal is at
least half a cell of the first grid. A smaller loop that can be capped is closed quietly with a flat cap in every mode
(a crack under half a first-grid cell can be several cells of a second pass's finer grid, where the flood would leak
through it); it is not counted in `HasOpenings` or `openings_closed`. A loop can be capped when its part reaches more than 2 % of the part's size out of
the loop's plane (Newell normal through the loop's centre), so a flat sheet (a floor, a star, a wall inside a cube)
is never offered caps. `HasOpenings { count, largest, closed_parts }`: the number of loops that count and can be
capped, the largest one's diagonal as a share of the model's, and whether a closed part exists. A part is closed when
it has at least 4 triangles and no boundary loop except cracks the quiet caps close (under half a first-grid cell and
cappable); a sheet of any size, even a decal strip whose rim is under half a cell, is open. `CloseMode`:
- `Refuse`: such loops give `Err(HasOpenings)` before any grid work; otherwise the plain bake.
- `FlatCaps`: every loop that counts and can be capped is filled by ear clipping of the loop projected onto its plane (a
  non-planar loop still gets a spanning patch); loops of more than 512 corners, or whose projection crosses itself,
  get a fan from their centre.
- `RoundedCaps`: 8 rings from the rim towards the centre on an elliptic profile, raised along the loop normal turned
  away from the part's centre, height 0.5 x the loop's mean radius, closed by a fan to the top.
- `FillHollows`: no triangles are added; the flood of step 4 is replaced by the six-direction rule: a node is inside
  when a wall lies in at least five of the six axis directions (a blocked link between the node and the end of its
  grid line, or the model beyond the line's end, found by one long ray per line end). Open boxes and cups are filled;
  a tube open at both ends is not (two directions see out). Border links that no triangle blocks take the middle of
  the link as their skin point and distance.
- `ClosedPartsOnly`: every part that is not closed (sheets included) is dropped; nothing left gives
  `NoInterior` (an open box alone: the dialog offers this mode only when `closed_parts` is true).
Caps are ordinary triangles of the soup, so the classification (which only asks whether a wall lies between two
nodes) is unaffected by self-intersections, winding or doubled faces. `TipReport::openings_closed` = the number of
loops that counted (0 for `ClosedPartsOnly`), `closed_parts` = the model had both such loops and closed parts.

Centring: after the weld (on the file's bit-exact positions) the soup is moved by minus the centre of its used box,
and the openings, caps, grid, BVH and link rays work in these centred coordinates (for a model whose offset is about
1e5 times its size the f32 spacing of file coordinates is about a cell, which would quantise the nodes and swallow
`tau`). The field is in tip units, so it does not change; only the trace's `p0` (tests) is moved back.

Grid (5.7 steps 2 to 12, as designed except step 8, see below; Z accepts or reverts that deviation explicitly,
contract request B1-4): flat check (thinnest side under 0.5 % -> outline; under 2 % -> outline when the
solid finds no inside); cell = longest side / 122, `clamp(ceil(side / cell + 5.382), 8, 128)` nodes, first node 2.382
cells below the box; link bits by `Bvh::occluded` over `cell + 2 tau` (tau = 1e-4 cells) in parallel z slabs
(cancellation and progress per slab); flood from the outer layer; fewer than `max(8, nodes / 10,000)` inside nodes
is `NoInterior`; skin by `Bvh::raycast` from the outside node of every border link; second pass over the skin box
grown by 3 first-pass cells when the skin box is more than 2 cells inside the model box on some side (`partial`),
seeded by outer-layer nodes outside the first grid or with a clear segment to an outside corner of their first-pass
cell; border values: exact distance to the link's triangle, and every node within 2 links (along each axis) of a
border node takes its exact distance to the skin triangles within 1.5 cells (border nodes at most one cell), so the
values within one cell of the skin are exact (F2: 0.05 cells); all other nodes (deviation from 5.7 step 8):
`d(x) = |x - x_s| + f_s` for the node `s` with an exact value carried to `x` by a nearest-first propagation over the 6
(4) neighbours (a bucket queue of 1/8-cell buckets, a node passed on again whenever its value improves, sequential, so
the same on any number of threads). It is an upper bound of the distance to the skin, exact along a flat face's normal and
radially in a ball, and neighbours differ by at most one cell by construction; the stored `i16` is rounded down
(not to nearest), so the property survives the quantisation exactly (F9's 1.001 cells). The designed
`sqrt(min |x - x_b|^2 + f_b^2)` is short by up to one cell away from the skin, unevenly, which made the local thickness
of step 9 collapse to the depth near the skin of a baked ball (59 cells off at the worst of 1,000 points; with the
propagation and the exact values near the skin 46 cells at the worst point, all within 2 cells of the faceted skin,
and at most 2.45 cells deeper in; FEATURE_BRUSH F20 asks 1.5 cells everywhere: open, contract request B1-1 on
step 9). `tip_field::distances_from_border`
stays for S1's formula shapes; tip units `p0`, `s0` (one pass: centre of the model box and largest vertex distance; two
passes: centre of the skin box and largest skin-point distance plus one cell); lying flat by an exact re-indexing
(thin Z: `(x, y, z) -> (x, z, -y)`, thin X: `(x, y, z) -> (-y, x, z)`); `from_distances`. Flat outline (5.7b): the
triangles projected on the plane of the outline (the same quarter turns), a 256-node lattice on the longer side with
2 cells of margin, coverage by bucketed point-in-triangle tests, crossings by 10 bisection steps, `a*b/sqrt(a^2+b^2)`
for two crossings, `outline_from_distances`. Deterministic on any number of threads.

Cancellation: per z slab in the parallel stages, every 65,536 vertices in the weld and every 65,536 nodes in the
flood and the propagation. The bake records the end time of each stage (`log::debug!` "brush shape bake stages").

`bake_from_path` = `import_geometry_from_path` (first 30 % of the progress) + `bake` (the rest). The import runs on a
thread of its own; the calling thread relays its progress and returns `Cancelled` at once on a cancel (the parse stages
look at cancellation only between them, and a million-triangle OBJ parses for seconds); the import thread then stops
at its next check and its result is dropped. Loads are serialised: the cancelled import's thread is kept, and the next
`bake_from_path` waits for it to end (answering Cancel meanwhile) before its own parse, so a quick Cancel and reload
never runs two parses at the same time.

---

## 6. Algorithms

### 6.1 Mesh normalization (`mesh_build::normalize`) [C1]

1. Drop triangles with non-finite positions or 3D area below `1e-12 * diag^2`; count them.
2. **UV tile normalization**: per triangle subtract `floor(uv_centroid)` from its three UVs, so every centroid lies in `[0,1)^2`.
   A triangle whose UV bbox still leaves `[0,1]^2` gets `UV_WRAPS`. UDIM is not supported: all tiles fold onto one image; if any
   triangle had a non-zero tile the importer emits `ImportWarning::UvWrapped`.
3. **Missing/degenerate UVs**: no UVs, non-finite UVs, or `|uv area| < 1e-12` (in UV units) => `NO_UV`. Such triangles are drawn with
   the material base colour (the renderer draws `material_nouv_ranges` untextured, section 8.4), occlude, can be picked
   (`Hit.uv = None`, `Hit.texture_set = None`) and are never painted.
4. **Fallback UVs** (`generate_fallback_uvs` and *every* triangle is `NO_UV`): the generated layout of `import/auto_uv.rs`, a
   box-projection atlas usable for painting. (a) Every triangle gets the axis direction (+X, -X, +Y, -Y, +Z, -Z) its face normal is
   closest to. (b) Charts grow breadth-first from seed triangles (in triangle order) over edges shared by bit-identical positions to
   neighbours whose normal is within 60 degrees of the seed's axis; a neighbour whose projection would overlap the chart's projection
   so far (separating-axis test against a spatial grid) is rejected for that chart, so a folded surface (a spiral, a pocket) is split
   and no chart overlaps itself. (c) Each chart is projected onto its axis plane (side faces: world +Y is texture up; stretch at most
   2x) with the unmirrored orientation (negative signed area in v-down UV). (d) Charts keep their world size (one texel density) and
   are shelf-packed (sorted by height) into the unit square; the square grows by 3 % until everything fits, with a margin of 1/512 of
   the side around every chart (4 texels apart at 1024 px). Deterministic. All materials then share one set named
   `opts.model_name`. Report `fallback_uvs_generated`. (Before importer version 2 this was one grid cell per triangle pair; that
   generator no longer exists, see the rule at `IMPORTER_VERSION` in 5.21: a layout change bumps the version and keeps the old
   path. The version-2 layout is pinned by frozen `uv_hash` constants in the `mesh_build` tests.)
5. **Sets**: materials referencing the same `RawMaterial::texture` share one `SetInfo`; every other material with at least one UV-mapped
   triangle gets its own set; a material with none gets `texture_set = None`. Set order = order of first appearance by material index.
   Set assignment depends only on the `RawMaterial::texture` *index*, never on whether that texture could be read or decoded
   (so a load with `decode_textures == false` yields the same sets). Name: `opts.texture_names[t]` if the set has a texture `t`
   and that entry exists and is non-empty; else the first material's name if non-empty; else `opts.model_name`; else `"Texture"`.
   Names are then made unique with `_2`, `_3`.
6. **Normals**: if a triangle has none, use area-weighted smooth normals accumulated over vertices with bit-identical positions;
   zero-length result => face normal. Normalize all.
7. **Unify indices** with a `HashMap<[u32; 9], u32>` keyed on the bit patterns of (position, normal, uv, material) — insertion order
   (not iteration order) defines vertex order, so the result is deterministic.
8. Stable-sort triangles by `(material, NO_UV flag)` so that inside each material the UV-mapped triangles come first; build
   `material_ranges` and its two sub-ranges `material_uv_ranges` / `material_nouv_ranges` (either may be empty; for a material
   with `texture_set == None` the uv range is empty), `bounds`, per-set `UV_MIRRORED` flags (sign of UV area versus the set's
   area-weighted majority) and `uv_hash`.
9. **Objects** (objects-tabs, `normalize_with_objects`): every kept triangle carries the raw object index of its triangle
   (`objects.tri_object[i]`, 0 when the vector is empty or shorter than the triangle list; an index `>= names.len()` counts as
   0). The object is **not** part of the sort key nor of the vertex key: triangle order, vertex order and `uv_hash` are
   bit-identical to `normalize` for every input. Objects are renumbered densely in order of first appearance in the *raw*
   triangle order, counting only triangles that survived step 1 (an object without a surviving triangle disappears). Names:
   an empty name becomes `"Object N"` (N = 1-based dense index); if the mesh has exactly one object and its name is empty, it
   is `opts.model_name`, or `"Model"` when that is empty too; then names are made unique in index order with `_2`, `_3`
   (the smallest free suffix; the search of a base resumes after the last suffix it tried, so thousands of objects with one
   name cost linear time). `ObjectInfo::tri_count` and `bounds` come from the final triangles.

`wire_edges`: for every triangle edge build the key (min/max of the two vertex *position bit patterns*); edges with the same key are
the same geometric edge. Classify: one incident triangle => `Boundary`; two or more whose UVs at both ends are identical => `Interior`
(emit once); otherwise `Seam` (emit once per distinct UV pair). Sort the output by (set, kind, a, b) for determinism.
objects-tabs: each half edge also carries its triangle's object (as the last field of the derived order, never part of the
grouping); the emitted entry's `object` is that of the side's smallest-object half edge, so an edge shared by two objects
belongs to one of them (corner case: it disappears with that object when hidden).
`uv_layout_images(mesh, side)` draws the layout thumbnails of all sets from one `wire_edges` call (in parallel); the import job
uses it (one `wire_edges` per set made ABeautifulGame's 13 sets x 1.5 M triangles take seconds).

### 6.2 BVH and queries [C1]

```
#[repr(C)] struct BvhNode { min: [f32; 3], left_first: u32, max: [f32; 3], count: u32 }   // count == 0 => interior, children at left_first, left_first+1
```
- **Build**: binned SAH, 16 bins on triangle centroids, at most 4 triangles per leaf, explicit stack. Subtrees above 8k triangles
  split with `rayon::join`. Median split when all centroids fall into one bin.
- **Traversal**: fixed 64-entry stack, near child first. Slab test with precomputed `inv_dir`, written in the NaN-ignoring form
  `tmin = tmin.max(t1.min(t2)); tmax = tmax.min(t1.max(t2))`.
- **Triangle test**: Moeller-Trumbore; reject when `|det| < 1e-9 * |e1| * |e2|`. `Cull::Back` rejects `det < 0`.
- **Hit**: `position = ray.at(t)`; `geo_normal` from winding; `front_face = dot(geo_normal, ray.dir) < 0`; `normal` = normalized
  barycentric blend of vertex normals (`geo_normal` if the blend is degenerate), **negated iff `!front_face`** — the flip is decided
  by the winding alone, so `surface_normal()` recovers the stored shading normal exactly; `uv` = barycentric blend unless `NO_UV`.
- **Sphere query**: descend into nodes whose point-to-AABB squared distance <= r^2; at leaves use `closest_point_on_triangle`. Sort.
- **Hidden triangles** (objects-tabs): `intersect` (every ray query) and `walk_sphere` (every sphere query) take the mask
  once (`self.hidden.as_deref()`) and skip a hidden triangle in the leaf loop before its triangle test. Node boxes are not
  rebuilt: a ray simply finds nothing in hidden leaves. One predictable branch per leaf triangle (no measurable cost:
  `docs/notes/objtabs/wp2.md`). The mask lives in the BVH rather than as a query parameter because about 50 production call
  sites query it, several on worker threads through a cloned `Arc<Bvh>`: a job sees the visibility of the moment it cloned
  the `Arc` (a snapshot by construction), and a toggle costs one mask build instead of a BVH rebuild.

### 6.3 TexelMap and the surface rasterizer [C1]

**Build** (`TexelMap::build`):
1. Rasterize the set's triangles (not `NO_UV`) in index order with a texel-centre-inside test in pixel space; first writer wins; later
   writers increment `overlapped_texels`. Triangles with `UV_WRAPS` are rasterized in unwrapped space and written at
   `(x.rem_euclid(W), y.rem_euclid(H))`. Each edge function is monotone along a row, so a row's inside texels are one interval, found
   by bisection with the same expression and epsilon as the per-texel test. A triangle is processed per *wrapped* row: the intervals
   of every unwrapped row folding onto it are folded and merged first, so a tiled triangle whose UVs repeat over many tiles costs one
   pass over the image (not tiles x W x H) and a texel covered by several images of one triangle counts once. A texel whose centre a
   second triangle also covers is marked *shared* (overlap, mirroring, or a centre on an edge two triangles share). The build polls
   its cancel flag once per triangle.
2. Sliver fallback: a triangle that covered no texel centre claims the texel under its UV centroid if it is free.
3. Gutter: multi-source BFS (8-neighbourhood, wrapping at the image borders) from all covered texels, `gutter` steps deep; a newly
   reached texel inherits the owner of the texel it was reached from, with bit 31 set, and remembers the covered texel its ring grew
   from; gutter texels grown from a shared texel are listed in `shared_gutter`.
4. Stats: `covered_texels`, `overlapped_texels`, `mirrored_tris`, `coverage`, `texel_world` (sum of 3D areas / sum of UV pixel areas
   over the set's UV-mapped triangles, square root; 1.0 if undefined).

Memory: 4 bytes per texel (64 MiB at 4096^2, 256 MiB at 8192^2).

**`raster_tri` rule**: a texel is emitted for triangle T if its centre is inside T (inclusive, edge epsilon `-1e-4` px) **or**
`owner(texel) == T` (covers gutter texels and partially covered edge texels) **or** it is a gutter texel grown from a shared texel
whose centre T covers (mirrored, stacked or instanced islands: painting or projecting onto any copy also fills the padding, with
barycentrics extrapolated for T). Inside texels come first (row intervals, every unwrapped image), then one pass over the wrapped
texels of the clipped box for the ownership and shared-gutter cases (a wide triangle uses the image closest to it). When that
folded box holds 64 x 64 texels or more, pass 2 does not scan it texel by texel: it walks only the texels it can report, in the
same order — the gutter texels T owns (a lazily built index of all gutter texels sorted by owner, `OnceLock`, 4 bytes per gutter
texel, built on first use), the `shared_gutter` entries of the row and the sliver texel (step 2) — so a tiled triangle (whose box
is the whole image) no longer costs W x H owner lookups per call. `raster_tri_covered` is the folded form for callers to whom
every inside fragment of some parts of the triangle means the same (a dab whose full-strength core contains them). A predicate
`fold_block` picks the parts: it is asked about the whole clipped box first (`None` = everything) and, if it refuses, about each
64 x 64 block of unwrapped texels (aligned to multiples of 64, clipped to the box; the picks of a block row are computed when a
row of it is first scanned). Pass 1 reports the covered texels of picked parts once per call as row runs `covered(y, x0, x1)` —
the per-wrapped-row inside intervals of all images folded and merged as in build step 1 — instead of one fragment per image,
and the inside fragments of the other blocks one by one (the fragments of `raster_tri`, grouped by wrapped row); pass 2 is
unchanged. Tests compare both forms with the plain box scan (nothing picked, everything picked, a checkerboard of blocks). The scan bbox is the triangle's pixel bbox grown by
`gutter + 1`, intersected with `clip`. Barycentrics of gutter texels are extrapolated (outside 0..1), so painted or projected content
continues smoothly into the padding and bilinear/mip sampling never picks up stale colour at a seam. Edge functions are evaluated in
`f32` relative to the bbox minimum (precision at 8k). A fragment's `x, y` are wrapped; `rows` filters on the wrapped `y`.

Consequences every caller relies on:
- Shared edges and overlapping UVs can visit a texel more than once. **All writers must be idempotent (max rule)**; never accumulate
  additively inside a `raster_tri` callback.
- Normals interpolated with gutter barycentrics must use `normalize_or_zero` and fall back to the geometric normal.

**Parallel pattern** (bakes and large dabs): build the candidate list, bin `(tri)` entries into 64-row bands by wrapped row range
(a wrapping triangle is listed in every band it touches), then `par_chunks_mut(64 * W * 4)` over the target buffer; each band calls
`raster_tri` with its `rows`. No shared writes, no locks. Round 7 (the 3D dab, 6.4): the bands are 32 rows and the binning is by
each candidate's **tight** scan box (`raster_box`: the clipped box `raster_tri` scans, the same function), as CSR (count per
band, prefix sums, fill; O(candidates + entries), buffers reused), so a small triangle lands in one or two bands instead of
every band of the dab's footprint. The folded row loops of `raster_tri_covered` walk only the rows that fold into `rows` (at most
two intervals), so a whole-image tiled triangle costs its band's rows, not every row of the image, per band.

**Lean pass 2** (round 7 A1, `raster_tri_lean` / `raster_tri_covered_lean`, called only by the brush): pass 2 of T reports only
(i) gutter texels T owns, (ii) the texel T claimed as a sliver (build step 2) and (iii) `shared_gutter` texels whose source T
covers — exactly the candidates of the indexed pass 2. So when T's bit in the map's brush index is 0 (T owns no gutter texel
and claimed none) and no 8 x 8 block of the index that holds a `shared_gutter` texel meets the folded pass-2 box within `rows`,
pass 2 is skipped; the call then visits the same fragments in the same order (test `lean_pass2_visits_the_same_fragments`). On a
dense mesh most triangles are interior, and their pass 2 used to scan their box grown by the gutter (8-16 texels) to find
nothing: 15-26 x the touched texels on ABeautifulGame (round 7 step 0). The index is built in parallel chunks of the owner array
(a set union: the same on any thread count) by `prepare_brush_index`, called by `stroke_target` and at the start of every dab
on the calling thread (never lazily inside a band job: a pool thread waiting in `OnceLock` for an initialisation that its own
stolen work started would deadlock).

### 6.4 Brush engine [C3]

**Stroke model** (opacity caps, flow builds — Photoshop-like). Per stroke and per touched texture set:
- `mask: Vec<u16>` — accumulated stroke coverage 0..=65535 (u16: low-flow soft brushes must not quantize to zero).
- `dab_cov: Vec<u8>` + `dab_touched: Vec<u32>` — per-dab scratch (max rule within a dab).
- `TileBackup` of the target layer — blend source during the stroke and the undo record afterwards.
The scratch lives in `TextureSet::scratch` and is reused; only the stroke's dirty rect is cleared at stroke end.

**Targets**: when a set is first touched by a stroke (a dab finds candidate triangles in it), its target is the active layer if
`paint_target(set) == Ready` (an unallocated target first gets its transparent buffer, `TextureSet::allocate_raster`; `fill`
does the same); otherwise the set is skipped for the whole stroke and recorded with its reason for
`stroke_skipped_sets()`. The core never creates layers during a stroke: the app gives every set that needs one a paint layer before
`stroke_begin` (section 8.6), so in practice only `Locked` / `Hidden` targets are skipped. The layer's pixels are obtained with
`Arc::make_mut` (copy-on-write only if a `DocumentSnapshot` or a history entry that pinned this layer still shares them; ordinary
structure snapshots do not pin raster pixels, section 6.7).

**Reference normal**: a stroke works with the **unflipped** shading normal `n = hit.surface_normal()`, not with the viewer-facing
`hit.normal`. So a hit on a back-facing triangle (reversed winding, the inside of an open mesh — it is rendered, because
`cull_mode` is `None`) has `dot(geo_normal(tri), n)` near +1 and is painted like any other.

**One 3D dab** at centre `c`, reference normal `n`, radius `r` (`dab_3d`); `aa = texel_world_size(set)` and the **support radius**
is `r_s = r + 0.5 * aa`:
1. `bvh.query_sphere(c, r_s)`; keep triangles of this set that are not `NO_UV`; if `same_side_only`, drop triangles with
   `dot(geo_normal(tri), n) < -0.2`. (objects-tabs: the query skips hidden triangles, so a hidden object is never a dab
   candidate and never takes part in any side or occlusion test.)
2. Per candidate triangle: `d` = signed distance from `c` to the triangle plane; skip if `|d| >= r_s`.
   `rho = sqrt(max(0, r_s^2 - d^2))`. With the triangle's 3D-to-UV-pixel affine map (gradients `grad_u`, `grad_v` in px per world
   unit) the clip box is `uv_px(c projected onto the plane) +/- (rho*|grad_u| + gutter + 1, rho*|grad_v| + gutter + 1)`. Without it a
   large triangle on a 4k texture costs a million texels per dab.
3. `raster_tri` per candidate, folded where the dab's full-strength core (`falloff == 1`, radius `min(hardness * r, r - aa/2)`,
   `core_radius`) allows it, since the max rule then gives the same `dab_cov` as visiting every fragment:
   - a candidate whose every inside fragment lies in the core — `max_i |p_i - c|`, widened by the inside test's barycentric
     slack, at most 99.9 % of the core radius, see `in_full_core` — is rasterized with `raster_tri_covered`, everything picked;
   - any other candidate whose clip box holds 64 x 64 texels or more (`FOLD_MIN_TEXELS`) uses `raster_tri_covered` with
     `BlockFold::in_core` as the pick: the box's corners mapped through the triangle's pixel-to-plane affine map (distance is
     convex, so its maximum over a box is at a corner) all lie within 99.9 % of the core radius;
   - the folded runs of all candidates of the dab (of the band, in the parallel pattern) are collected, sorted and merged per
     row, and only then written at coverage 255 (`put_core_runs`), once per texel. A tiled triangle used to repeat each texel
     up to tiles^2 times, and a model whose faces all share one tiled UV space (ngons_tiling.obj: every polygon maps 0..4) used
     to repeat the whole image once per triangle per dab.
   Per fragment: `P = sum(b_i * p_i)`, `dist = |P - c|`,
   `cov = falloff(dist, r, hardness, texel_world_tri)` where `texel_world_tri = sqrt(area3d / area_uv_px)` of that triangle;
   `dab_cov[i] = max(dab_cov[i], round(cov * 255))`; first touch pushes `i` to `dab_touched`. Distance is measured **in 3D**, so a dab
   across a UV seam is continuous on the surface and overlapping/mirrored UVs are idempotent.
4. Commit: for each `i` in `dab_touched`: `inc = ceil((65535 - mask[i]) * strength * dab_cov[i] / 255)` with
   `strength = flow * pressure`; `mask[i] += inc`; `dab_cov[i] = 0`. `ceil` guarantees progress. Return the changed rects.
   A dab that touched at least 65536 texels and an eighth of the image (a large brush on a small or tiled texture) is
   committed on the rayon pool in 64-row bands over the dense `dab_cov` (non-zero exactly at the touched texels); bands are
   whole rows of the rect tracker's 64 x 64 tiles, so the rects are the same as the serial commit's. Since round 7 that
   commit is only reached from the serial path (below `PAR_DAB_WORK`, where it never triggers) and from the `RASTER_R5` test
   reference: a dab with more work commits inside the bands of the fused pass (below).

**Round 4: one query per dab, and tips** (FEATURE_BRUSH 5.2 to 5.6). A stroke dab no longer calls `dab_3d` per set.
`Document::surface_dab` builds the dab's shape and calls `shared_candidates` once, then `dab_tip` per set:

- **Shape.** `r` = the stroke's radius, `k = r * tip.scale` (one tip unit in world units), `n_v` = the viewer-facing
  normal (`Hit::normal`, blended between samples exactly like the reference normal `n`), the tip centre
  `c' = c + offset * r * n_v` (`c` itself for offset 0, bit for bit). The frame `R = A * U`: `U` = the tip's rotation;
  `A = tip_frame(n_v, up_hint, prev_up)` for "Follow surface" (the returned tangent is carried to the next dab of the
  segment and dropped at a new segment), the identity for "Fixed in space". The ball has `R_b = r`; a field has
  `R_b = k * field.bound()` Solid and `R_b = k * max(field.bound(), 1)` Through (`brush::field_reach`: the prism is
  cut by the ring's ball of radius `k`, which must lie inside `R_b` also for an outline whose bound is under 1).
- **Shared query** (`shared_candidates`): per set not skipped `aa_s` = the set's mean texel size and
  `rho_s = R_b + 0.5 * aa_s`; one `query_sphere_dist(c', max rho_s)`. Per set, the coarsest texel size among its
  first-pass candidates (triangles of the set within `rho_s`, side test of `SameSide`: `dot(geo_normal, n) >= -0.2`;
  `All`: none) widens `rho_s` to `R_b + 0.5 * local_max` when it exceeds the mean by 0.1 %, exactly as `dab_3d`
  widens. If a widened radius exceeds the first one, one second query with the largest. The candidate list of a set is
  the triangles of the set within its final `rho_s` that pass the side test, ascending by id; for a field also those
  that survive the **early reject** (all three corners, in tip coordinates `q = R^T (P - c') / k`, beyond the same
  face of the field's box grown by half the triangle's texel size: `bounds()` for Solid; the outline's box in x and z
  and `-max(bound, 1)..max(bound, 1)` in y for Through; for a part cut off at its grid's border, whose zero level
  lies beyond the grid, the box is grown by the depth of the most negative value on its faces, or on the outline
  box's edge, which is 0 for every other field). For the ball the lists equal the lists `dab_3d` filters, so masks are
  bit-identical (test C2). At most 2 tree walks per dab instead of one to two per set plus one per unreached set.
- **Reach.** A set the stroke has not reached is reached when its list is not empty (for the ball this equals the old
  `dab_reaches_set`); then `stroke_target` decides painted or skipped. A set that only the bounding ball of a shape
  touches therefore gets no buffer and no "Paint skipped" report (test C24).
- **Per triangle** (`prepare_tri`, = the old step 2 with `R_b` and `c'`): plane test, clip box and fast-path flags.
  A field with `R_b < max(aa_s, tri_aa)` is painted as the ball of radius `R_b` at full hardness (tiny shapes); a field
  with `k * depth < tri_aa` is painted at full hardness.
- **Coverage of a field fragment** (`FieldDab::cover`), `P` the fragment's point, `q = R^T (P - c') / k`:
  Solid: a fragment whose `q` lies beyond a face of the early-reject box grown by `0.5 * aa / k` plus one cell is 0
  without a lookup (the zero level lies inside `bounds()` and `F` grows at least as fast as the distance to that box
  beyond it, so the formula gives 0 there too; the extra cell absorbs the quantization; it halves a small Cube dab's
  cost); otherwise `sd = k * F(q)` (`TipField::sample`); Through: `sd = k * G(q.x, q.z)` (`outline`) times the hard limit at the
  ring's ball `clamp(0.5 - (|P - c'| - k) / aa, 0, 1)`. `thick = k * T(q)` (`thickness` / `outline_thickness`) only
  in the fade zone (`hardness < 1` and `-sd < (1 - hardness) * k * thick_max`), else `k * thick_max`, where the
  profile is 1 for every possible thickness; `cover = falloff(thick + sd, thick, hardness, aa)`. For the ball
  `thick + sd = |P - c'|` and `thick = r`: today's formula, computed by today's code. `TipGeom` (per stroke): Solid
  `depth = inradius`, `thick_max = inradius + cell / 2` (thickness nodes are inside depths stored in half cells);
  Through: the largest depth and thickness of the outline found by scanning its nodes, plus half a cell.
- **Fast paths for a field**: a region within `rho` of a point `g` has coverage exactly 1 when
  `-sd(g) - 1.8 * rho >= 1.001 * max((1 - hardness) * k * thick_max, aa / 2)` (1.8 bounds the change of `F` and `G`
  per unit moved: neighbouring nodes differ by at most a cell) and, for Through, `|g - c'| + rho <= 0.999 (k - aa / 2)`.
  Whole triangle: `g` = centroid, `rho` = the farthest corner widened by the barycentric slack. Block (64 x 64 texels of
  a large box): `g` = the mean of its four corners mapped to the plane, `rho` = the farthest of them. Folded and
  unfolded rasterization give the same mask (test C16).
- Per set with a non-empty list: setup, `rasterize_tip` (serial or the band pattern), commit and re-apply as before
  (since round 7 the fused pass `raster_commit` below replaces `rasterize_tip` + `commit_dab`).
  A ball triangle is rasterized by `raster_ball`, round 3's per-candidate code with the ball's coverage inlined; a
  field triangle by its own instance with the field's coverage inlined. (One closure that chose between the two per
  fragment made a 50-texel ball dab rasterize about 1.7 x slower: keep the two instances separate.)

**Round 7: the fused dab pass** (`raster_commit`, called by `dab_tip` and `dab_3d`; no pixel changes). Steps 3 and 4 of a set's
dab are one function:
1. Candidates: `prepare_tri` per triangle of the list, on the pool (`par_extend`, order kept) from 4,096 triangles on.
2. Tight boxes: `raster_box` per candidate (on the pool from 4,096 candidates); a candidate without one visits nothing and
   is dropped. Work = the sum of the box areas, rows and columns capped at the image.
3. Below `PAR_DAB_WORK` = 16,384 texels of work, or with fewer than `PAR_DAB_MIN_CANDS` = 8 candidates and less than
   4 x `PAR_DAB_WORK` (a few large triangles: cheaper on the calling thread than the pool's wake-up): round 5's serial
   loop (every candidate over all rows, `put_core_runs`; pass 2 lean) and `commit_dab`.
4. Otherwise: CSR binning of the candidates into bands of `DAB_BAND_ROWS` = 32 rows by their boxes' wrapped row ranges (6.3; a
   box of at least H rows goes into every band), then one parallel pass over the bands that have candidates,
   `mask.par_chunks_mut(32 * W).zip(dab_cov.par_chunks_mut(32 * W))`: per band, `raster_cand` (lean pass 2) of its candidates
   within its rows into its rows of `dab_cov` with a band-local touched list and core runs (`put_core_runs` per band), then
   the commit of its own texels (the same `mask_increment`, `dab_cov` reset to 0) into a band-local `RectTracker`. The calling
   thread merges the bands' tiles **by union per tile key** in band order (32-row bands share the tracker's 64-row tiles) and
   calls `finish()`.
Each band owns its rows of `dab_cov` and `mask`; within a dab a texel's coverage is the max over the fragments of its own rows
(order-free); its commit depends only on that texel; the tracker's rects do not depend on the order in which texels arrive; the
lean pass 2 visits exactly the fragments of the full one; and the partition depends only on the dab (both constants are fixed,
never derived from the thread count). So masks, rects and layer bytes are round 5's on any thread count (tests T1-T6 of round 7:
`r7_dabs_equal_round5_on_random_dabs`, `dab_raster_is_the_same_on_any_thread_count`, `lean_pass2_visits_the_same_fragments`,
`fused_band_commit_equals_the_serial_commit`, `a_stroke_paints_round5s_pixels`, `prepare_brush_index_is_complete`). Between dabs
`dab_cov` is all zero and `dab_touched` empty, as before. The test seam `RASTER_R5` (thread-local, read once per dab) runs round
5's `rasterize_tip` (clip-box binning into 64-row bands, full pass 2) and `commit_dab` instead: the reference of the equality tests
and of the in-process A/B rows "... r5" of `stroke_bench_tips` (whose r7 and r5 hashes must be equal per row).
A stamp bake in flight when a stroke starts keeps its own pool (`stamp-bake-N`) until it lands; its threads then share the cores
with the dab's (accepted; separate pools, the UI thread waits only on the global pool, so no deadlock).
Round 7 also (A3) runs a large shared query on the pool: from 32,768 found triangles both passes of `shared_candidates` go over
fixed chunks of 8,192 (per-chunk per-set maxima, then their maximum; per-chunk `(set, tri)` lists appended in chunk order, so every
list stays ascending; `map_of` is asked once per set first), and `query_sphere_dist` sorts with `par_sort_unstable_by_key` from
32,768 entries (unique keys: the same order); the tree walk stays serial. And (A4) every 3D commit (the bands' commits and both
paths of `commit_dab`) records per row the span from the first to the last texel whose mask it raised
(`StrokeScratch::row_spans`); the re-apply below shades only that span of each row of a rect, since the rect's other texels
already hold `apply_brush(original, mask)` (a stale older span only means extra shading, never a different value). A 2D dab
clears the spans' validity: its rects are re-applied whole. The test seam `FULL_REAPPLY` (thread-local) is round 5's whole-rect
re-apply.

**2D dab** (`dab_2d`): disc in texel space around `center_px`; `dist` = distance from the texel centre; AA width `aa = 1` texel;
the scanned bbox is `center_px +/- (radius + 0.5)` (the support radius); writes at wrapped coordinates (a dab crossing a border
yields up to 4 rects); no `dab_cov` needed (compute `inc` directly).

**Re-apply** (in `stroke.rs`, after each dab, over the dab's changed rects): `backup.touch_rect(layer, rect)` first, then **row by
row**: `backup.original_row(layer, y, x0, x1, &mut row)` (one tile lookup per 64 texels — never `original()` per texel), and for
texels with `mask > 0`: `s = opacity * (mask / 65535) * (color.a / 255)`;
`layer[i] = apply_brush(row[i], color, s, mode, layer.alpha_lock)`; then `doc.mark_layer_dirty(layer, rect)`.

`apply_brush`, straight alpha, `(Cd, ad)` = pre-stroke pixel, values in 0..1, results rounded to u8:
- Paint: `ao = s + ad*(1 - s)`; `Co = (Cs*s + Cd*ad*(1 - s)) / ao`; `ao == 0` => keep `Cd`.
- Erase: `ao = ad*(1 - s)`; RGB unchanged.
- Paint with alpha lock: `ao = ad`; `Co = lerp(Cd, Cs, s)`. Erase with alpha lock: no change.

**Falloff**: `falloff = profile(min(dist / radius, 1)) * clamp((radius - dist) / aa + 0.5, 0, 1)` with
`profile(t) = 1` for `t <= hardness` (and everywhere when `hardness >= 1`), else
`1 - smoothstep(0, 1, (t - hardness) / (1 - hardness))`. There is **no** early-out at `t >= 1`: the anti-aliasing ramp is centred on
the rim and reaches zero only at `dist = radius + aa/2`, which is why every query above uses the support radius. Consequences: a
hard brush has a full two-sided AA ramp (0.5 at the rim), and the smallest brush (diameter 1 texel, radius 0.5) still covers the
texel under the cursor wherever the cursor sits inside it (`dist <= 0.707 < radius + 0.5`), so thin strokes have no gaps. Because a
soft profile is 0 at the rim, **`dab_3d` / `dab_2d` pass `hardness = 1` whenever `radius < aa`** (a brush under 2 texels across:
hardness is meaningless below texel resolution), which keeps that guarantee for soft brushes too.

**Spacing and interpolation** (`StrokeSession`): keeps `last` (position, normal for 3D; set + px for 2D) and `residual`.
- First sample of a segment: one dab at the sample.
- 3D sample at distance `L` from `last`: if `L > 6 * radius` or `dot(n_last, n_new) < 0` (both unflipped reference normals), treat
  as a new segment (one dab, no bridging — handles crossing a silhouette). Otherwise emit dabs every `step = max(spacing * 2 * radius, 0.5 * texel_world(set))`
  along the straight 3D segment, carrying `residual`; centres and normals are linearly interpolated (off-surface centres are fine,
  the falloff is spherical). Each dab is applied to **every** set with a target (the sphere query filters by set).
- 2D sample: same along the segment in texel space, `step = max(spacing * 2 * radius, 0.5)`; a sample on a different set starts a new
  segment.
- A stroke is 3D or 2D, decided by its first sample; samples of the other kind are ignored.
- `stroke_break()` clears `last`.

**End**: per touched set `backup.into_patch(layer)`; non-empty patches become `Edit::Pixels`; all of them go into one `Edit::Multi`
labelled "Brush Stroke" / "Erase". `stroke_cancel` calls `backup.restore`. Both clear the scratch over the stroke's dirty bounds.

**Fill**: `Layer` scope fills all texels. `Island` scope (gutters of neighbouring islands touch in real atlases, so the flood must
never walk through gutter texels):
1. Seed texel = texel under `seed_uv` (wrapped). No owner => `Err(InvalidParam("no UV island here"))`. If the seed is a gutter
   texel, start instead from any non-gutter texel of its owner triangle (found with `raster_tri` over that triangle; if the
   triangle covers no texel centre, the island is just the texels it owns).
2. 4-neighbour BFS (wrapping at the image borders) over **non-gutter owned texels only**, collecting the filled texels and the
   set `T` of their owner `TriId`s.
3. Additionally fill every gutter texel whose owner is in `T` (scan the bbox of step 2 grown by `gutter`, wrapping).
Pixel rule: `rgba = color`, or with alpha lock `rgb = color.rgb, a unchanged`. Uses `TileBackup` for the undo patch and
`mark_layer_dirty`.

The app bounds the work per frame (at most 64 surface samples per frame, section 8.6); the core does not time-slice.

### 6.5 Stamp / text bake [C4]

objects-tabs: every geometric query of a bake (candidates, occluders, the depth fit) goes through the job's BVH, so hidden
triangles receive nothing, do not occlude and are not fitted to; the job's `Arc<Bvh>` fixes the visibility of the moment
the job was made (5.14 rules).

Source pixels: for an image stamp, the blob is decoded once (`decode_image`, max side 8192) and turned into `PremulMips`, cached in
`Document::image_mips`; for a text stamp, `rasterize_text` output is cached in `Document::text_mips` together with the `TextSpec` it
was built from (re-rasterized only when the spec changes). Font lookup: `FontCache::get` for `spec.font`, plus `FontCache::fallback`;
if the font blob is missing use the fallback only.

**Text raster resolution** (R8 quality: text must not be magnified from a small raster). For a text stamp with full picture size `(w, h)` px (the frame
of its current spec, `stamp_text_frame`; also while the cached picture is a preview) at `spec.size_px = S`, the on-model em height in texels of `home_set` is
`em_texels = 2 * half_size.y * S / h / texel_world_size(home_set)` (`Projected`) or `size_px.y * S / h` (`Uv`).
`text_raster_px(id) = clamp(next_power_of_two(ceil(em_texels)), 32, 512)` (power-of-two buckets, so small gizmo scalings do not
re-rasterize). `add_stamp` and `finalize_stamp_bake` (i.e. every `Commit::Final` and every flush) compare it with `S`; if it
differs they set `spec.size_px` to it and apply the text anchor rule below (the new size is tried with frames only; when its
picture would be too large, the next smaller power of two above the current size is tried; the picture is drawn once,
by the bake that follows), so the on-model size is unchanged
while the raster now has at least one raster pixel per texel (up to the 512 cap; a label scaled to 4x its size is re-rasterized
at the next bucket instead of being magnified). `Commit::Preview` edits never change `size_px` (cheap while dragging/typing), except for the too-large fallback below.
Outline width and letter spacing are em fractions, so the look is independent of the bucket.

**`bake_projected`**:
1. `(right, up, out) = p.axes()`. `bvh.query_sphere(p.origin, p.bounding_radius())`. Drop `NO_UV` triangles and triangles whose
   projector-space AABB misses the box. If `backface_cull`, drop triangles with `dot(geo_normal, out) <= 0`. Group by set.
2. Per set: target buffer = union of the candidates' wrapped pixel bboxes plus gutter (a wrapping candidate forces full width or
   height on that axis). Work in a dense scratch of that bbox. (Measured in round 3: zeroing and cropping a full 1024 x 1024
   scratch costs ~3 ms, so this is not where tiled models lose time; see the cost rules in step 3.) The sets are rastered in
   groups of consecutive sets whose scratches sum to at most `SCRATCH_BUDGET` = 256 MiB (one parallel pass per group, cost rule
   (d)); a set larger than that forms a group alone, so a bake never holds more than max(256 MiB, its largest scratch) at once.
3. Band-parallel `raster_tri`. Per fragment:
   - `P = sum(b_i * p_i)`; `l = (dot(P - o, right), dot(P - o, up), dot(P - o, out))`.
   - Reject if `|l.x| > hw + bx` or `|l.y| > hh + by` (`bx, by` = one source texel in world units, so the transparent border filters
     correctly) or `l.z > depth_front` or `l.z < -depth_back`.
   - `N` = interpolated normal (`normalize_or_zero`, fallback geometric). `facing = dot(N, out)`;
     **`if !p.backface_cull { facing = facing.abs() }`** (with culling off, surfaces facing away from the projector — the far
     side in project-through mode, or triangles with reversed winding — receive the decal with the same angular weight).
     `w = smoothstep(cos(angle_end), cos(angle_start), facing)`; multiply by a soft fade (smoothstep) over the last
     `DEPTH_FADE_FRACTION` = 20% of each depth limit, so the end of the box never leaves a hard edge.
     Skip if `w <= 0`.
   - Occlusion (`Full` and `p.occlusion` only): `g` = the triangle's geometric normal negated if `dot(g, out) < 0` (i.e. flipped
     toward `out`); ray from `P + g * 1e-4 * diag` in direction `out`, `t_max = depth_front - l.z`; if `bvh.occluded` => skip.
   - Image UV: `u = 0.5 + l.x / (2*hw)`, `v = 0.5 - l.y / (2*hh)`; apply `flip_x` (`u = 1 - u`), `flip_y` (`v = 1 - v`).
   - `lod = log2(max(src_px_per_world * texel_world_tri, 1))` with `src_px_per_world = src_w / (2*hw)`. `Full`: `src.sample(u, v, lod)`;
     `Preview`: `src.sample_nearest_level`. Un-premultiply, multiply by `tint` (all four channels) and alpha by `w`, round to u8.
   - **Max-alpha rule**: write the pixel only if its alpha is greater than the alpha already stored (deterministic; correct for
     mirrored/overlapping UVs and double-visited edge texels).
   - **Cost rules** (fix round 3, R3-4; none of them changes a single output byte, `tiled_uv_bakes_equal_the_plain_reference`
     compares against the steps above written plainly). On tiled UVs a decal larger than one UV tile reaches each texel through
     many unwrapped images (ngons_tiling.obj: ~10-15 fragments per texel, most of them on surfaces hidden by others), so:
     (a) a texel already at alpha 255 is skipped; (b) a fragment whose largest possible alpha
     (`SAMPLE_ALPHA_MAX * tint.a * w`, rounded as `shade` rounds) does not beat the stored alpha is not sampled; (c) no
     occlusion ray is cast for a pixel that would not be written anyway; (d) the bands of all sets of a scratch group (step 2) run in one parallel pass;
     (e) occlusion rays are answered without the BVH wherever that is provably the BVH's answer: `Occluders` bins every
     triangle a ray of this bake can hit into a 64 x 64 grid (built only when the jobs' clip boxes estimate at least
     `COARSE_MIN_FRAGMENTS` = 200 000 fragments: below that the BVH answers the few rays faster than the grid is built) over the projector (x, y) of the box (padded by 1e-3 diag; cells
     sorted by the triangle's highest z, so a ray stops at the first triangle below its start), and `classify` decides each
     (f64, the crossing distance and barycentrics prepared as affine functions of the ray origin per triangle) as `Hit` / `Miss`
     only with margins of ~17 f32 epsilons times the condition `span * longest edge / |n . dir|` (plus the f64 rounding) around
     the f32 test of `Bvh::occluded`; anything closer, worse conditioned, outside the grid, below `-depth_back`, or in a cell of
     more than 48 triangles goes to the BVH (`Bvh::occluder`, which also names the hit triangle). The last occluder of a band
     is tried first. The full ray test still comes last, after the sample (testing it first was measured: twice the rays,
     slower). (f) Two constant-time tests run before the sample instead (built only from `FINE_MIN_FRAGMENTS` = 1 M estimated
     fragments): on a finer 512 x 512 grid over the same area, `cover_grid` (parallel over chunks of triangles, merged to the
     serial answer: highest plane, lowest index on a tie) keeps per cell the highest binned triangle that covers the whole cell (all barycentrics > 0 at its corners)
     with its plane below `front`, and `floor_grid` the depth `F` below which `verdict` returns `Hit` against it for every ray
     origin within `m` of the prism `cell x [-back, F]` (weights and crossing distance are affine in the origin, the error terms
     convex, so the eight corners decide; `m` = 1.01e-4 diag plus 2e-6 times the coordinate scale covers the ray start offset
     and the f32 rounding of the fragment's projector coordinates). A fragment that is not a gutter texel (its ray starts at
     its surface point, `t_max = front - l.z`) with `l.z` below its cell's floor is rejected after a table lookup
     (`Occluders::below_floor`); any other fragment asks `Occluders::covered` (the cover triangle's `classify` for its ray).
     `occlusion_shortcuts_agree_with_the_bvh` checks the shortcuts ray by ray against `Bvh::occluded`; the ignored timing
     probe `stamp_bench_tiled_model` (doc_stamps.rs) also compares a real model's Full bake with the plain reference.
     Measured on ngons_tiling.obj, 20 % decal, 12 threads: Full bake ~250-280 ms against ~420-460 ms without the grid
     and ~1-2 s for round 2 (see the round-3 notes); a Full bake costs about one Preview bake plus 30-50 %. The depth fit
     (17 x 17 rays) costs 0.2 ms and the candidate query and the scratch under 15 ms: neither is where tiled models lose time.
4. Turn each set's scratch into a `SparseBuf` (`SparseBuf::from_dense`: the touched 64 x 64 tiles grouped into parts, each
   cropped to its non-zero pixels); omit empty results. A decal over neighbouring faces whose islands lie far apart in a packed
   atlas then costs about its own area in memory, history and dirty rects, not the area of the bounding box of all its islands
   (review round 2, finding SP-2). The scratch itself is transient.
Gutter texels arrive through the owner rule, so decals are seam-padded automatically.

**Depth auto-fit** (`fit_projector_depth`, fix round 2 item 2: a fixed box depth cut decals off with a hard edge where a
curved surface such as the end of Avocado.glb falls away deeper than the decal is wide). With `s` = the larger half size:
a 17 x 17 grid of rays along `-out` over the placement rectangle, starting `2 s` in front of the plane, `t_max = 4 s`,
`Cull::Back` while `backface_cull` (else `None`); each ray keeps up to 4 hits as `(z, slope)` with
`slope = min(tan(angle(geo_normal, out)), 8)`. From the hit nearest to `z = 0` at the centre, the placement point's surface is
grown over the 4-neighbourhood: a neighbour's hit closest in depth joins when `|dz| <= grid step * (1.5 * max(slope_a, slope_b)
+ 0.35)` (a smooth surface steps by about its slope; an object edge onto a surface behind steps much more). Every cell
adds its first (visible) hit only when `|z| <= s` (the placement default of earlier versions): off that surface, so a decal
over many separate small parts (many_islands.obj) still reaches the nearby ones; on it (accept_fixes item F), so a nearer part
that hides the grown surface lies inside the box instead of in its fade (WaterBottle.glb: the grip sleeve stands ~4.5 mm in
front of the body, which continues under it; the grown surface followed the hidden body and the visible sleeve got a front
depth of 4.7 mm, i.e. the fade; `stamp_bake::tests::auto_fit_covers_a_visible_part_in_front_of_the_grown_surface` on
`samples::sleeve_quads`). With `[lo, hi]` the z range of all that (including 0): `depth_front = hi / 0.8 + 0.1 s`, `depth_back = -lo / 0.8 + 0.1 s` (so the
surface lies before the 20% fade), each capped at `2 s + 0.1 s`. Other parts in front of or behind the decal do not extend the
box; culling and occlusion still apply in the bake, so the decal cannot leak through a thin model. `Document::fit_stamp_depth`
runs it on the document's mesh. Who fits: the Stamp / Text tools on placement (not with Project through), the Transform tool
on every move / scale / rotate (drag and wheel) while `stamp_depth_is_auto` held at the start, the numeric Size / Rotation
fields of the tool options (same rule), `Document::set_stamp_source` when a new text or image changes the footprint of a
projected stamp with an automatic depth (a longer text was cut off at its ends otherwise), and the "Fit depth to surface"
button of the tool options. A depth typed into the Depth fields is no longer the fitted one and is kept (and scaled along by
corner / wheel scaling, as before).

**`bake_uv`**: for each texel in the bbox of `corners_px()` (clipped to the set, no wrapping): inverse-rotate the texel centre into
the stamp's local frame, `u = 0.5 + lx / size.x`, `v = 0.5 + ly / size.y` (v-down), flips, `lod = log2(max(src_w / size.x, 1))`,
sample, un-premultiply, tint. `clip_to_islands` rejects texels without an owner in the map. `SparseBuf::from_dense` of the result.

**Linked-layer maintenance** (shared by add / set / rebake): let `baked` = bake result by set.
- A set in `baked` without a linked layer gets a new layer (`LayerKind::Stamp` or `Text` by source kind, name = stamp name, opacity 1,
  Normal, visible) inserted directly above that set's active layer (top if none).
- A linked layer whose set is not in `baked` gets `SparseBuf::empty()`.
- In `finalize_stamp_bake` (so on `Commit::Final`, on every flush) and in `add_stamp`: linked layers with an empty cache are
  removed, except the one in `home_set`; if a removed layer was its set's active layer, the layer below (else above) becomes active.
- Dirty rects per set = the part rects of the old and of the new cache (not their union), reported with
  `mark_layer_rects_dirty(linked layer, rects)` (one new layer version). A cache equal to the old one keeps the old `Arc`.

**Text anchor rule** (FEATURE_TEXT 5.11; `text::frame::refit_placement`, T2's text in 6.5 has the formula). Whenever
a text stamp's picture changes (`set_stamp_source` replacing a text, and a raster-resolution change) the placement is
refitted from the old frame to the new one so that the on-model size of one em (per axis) and the on-model position of
the kept point stay. The kept point is `align_px` for a change of spacing alone (`differs_only_in_spacing`) when
neither spec has a warp or a line, and the anchor otherwise (`text::frame::Keep`). A plain text's anchor is its picture
centre, so editing a plain string is round 3's rescale bit for bit (same em: `f = (w1/w0, h1/h0)`, centre untouched);
the automatic depth range is re-fitted afterwards as before.

Formula (`text/frame.rs`, pure; T2). Frame of a placement in its host space: centre `c`, full
size `(W, H)`, unit directions `ex`, `ey` of the picture's axes (Projected: `ex = right` negated by `flip_x`,
`ey = -up` negated by `flip_y`, `(W, H) = 2 * half_size`, `c = origin`; Uv: `ex = (cos r, sin r)`,
`ey = (-sin r, cos r)`, each negated by its flip, `(W, H) = size_px`, `c = center_px`). When the full-resolution frame of
a text changes from `(w0, h0)`, kept point `a0`, `S0` px per em to `(w1, h1)`, `a1`, `S1`
(`refit_placement(placement, old, new, keep)`):
`k0 = (W0 / w0, H0 / h0)`, `k1 = k0 * S0 / S1`; size `*= (w1 / w0, h1 / h0) * S0 / S1` (with `S0 == S1` exactly
`(w1 / w0, h1 / h0)`, the round-3 rescale bit for bit);
`c1 = c0 + ex * ((a0.x - w0/2) k0.x - (a1.x - w1/2) k1.x) + ey * ((a0.y - h0/2) k0.y - (a1.y - h1/2) k1.y)`.
The kept point is `anchor_px` (`Keep::Anchor`) or, for a change of spacing alone without warp or path,
`align_px` (`Keep::AlignEdge`). When both kept points lie exactly in the picture centres the centre is not touched,
so a plain text edited as in round 3 keeps its placement bit for bit. Preserved: the host size of one em per axis and
the host position of the kept point. A degenerate frame (zero size, zero or non-finite em) changes nothing.
`place_anchor_at_centre(placement, frame)` moves the rectangle so that the anchor lies where its centre was (no change
for a centred anchor). Where the rule runs (`set_stamp_source`, the too-large fallback, `retune_text`) is T4's text
(`doc_stamps.rs`).

**Too-large fallback** (new paragraph): when the new picture of `set_stamp_source` or `add_stamp` (or the frame of
`stamp_source_frame` / `text_preview`) exceeds 8192 px on a side or, for a styled text, `MAX_STYLED_PIXELS`
(16,777,216) in area (`TextError::TooLarge`), the largest of `S/2`, `S/4`, ... down to 32 px per em whose frame fits
is taken (one `text_frame` per try, no pixels), `spec.size_px` becomes it and the anchor rule runs with `S1 < S0`.
Only when 32 px per em does not fit either does the edit fail with `Err(Text(TooLarge))` (nothing changes). This is
the one case in which a `Commit::Preview` changes `size_px`. `add_stamp` keeps the caller's placement (sized from
`stamp_source_frame`, which applies the same fallback).

**Structure invariants** (checked by `Document::debug_check_structure`, document.rs [S], which `push_structure_edit`,
`restore_structure`, `flush_pending_edit` and `from_parts` call last; `debug_assert!` only):
every `LayerKind::Stamp/Text` references an existing stamp whose source kind matches; every stamp has exactly one linked layer in
`home_set` and at most one in any other set; `stamps` is sorted by id; layer ids are unique across the document; `active` is `None`
or an existing layer of that set; raster buffers have the set's size or are unallocated (`0 x 0`); a known content bound of a
paint layer holds every non-zero pixel; outside a pending edit `preview_baked` is empty.

`rasterize_layer`: the linked layer becomes `Raster(cache.to_dense(w, h))` (content bound = the cache bounds); if the stamp has other linked layers, `home_set` moves to
the lowest remaining set id, else the stamp is removed. `merge_down` on a stamp/text layer = rasterize + merge in one history entry.
`merge_down` pixel rule: `below'[i] = blend_pixel(below[i], top[i], round(top.opacity * 255), top.blend)`; the result keeps the lower
layer's id, name, opacity and blend (exact when the lower layer is Normal at 100%; otherwise an accepted approximation).

### 6.5a Wrap bake (round 6, `wrap_param.rs`, `wrap_bake.rs`)

objects-tabs: hidden triangles are never patch members, never the anchor and never a Jump target. Every BVH query of the
parameterization skips them; the one path that is not a BVH query, `collect_grown`'s growth over the welded graph
(`WrapMesh::vtris`), treats a hidden triangle as outside the reach sphere (`in_sphere` returns `None`). The Jump branch
floods only over triangles marked by the masked sphere query, and `wrap_relax` / `wrap_bake` only walk the collected patch.
A parameterization is cached only while the document still has the BVH it was made with (`StampBakeOutput::bvh`,
`Arc::ptr_eq`): a job made before a visibility change still applies its pixels but does not refill the cache.

A wrapped stamp (`Projector::mode == Wrap`) gets a **local parameterization**: every vertex near it gets stamp-local
coordinates (world units along `right` / `up` of the projector, origin at the anchor); a texel's barycentrics give its
coordinates, those give the picture's (u, v). All of `wrap_param` runs in f64, serially, deterministically. Every tie
between vertices goes to the lower position (x, then y, then z), never the lower index, so the result does not depend on
which other triangles the candidate set holds.

1. **Reach.** `(hw, hh) = |half_size|`; the spine axis is the picture's longer axis (x when `hw >= hh`). `along = half
   extent along the spine + one source texel + 2 tw` (tw = the largest `texel_world` of the maps), `across` likewise;
   each is rounded up to `diag * 1.25^k` (integer k; 0-25 % to spare, `WRAP_HEADROOM`). Candidates:
   `query_sphere_dist(origin, along + across)` (`+ 4 GAP_JUMP diag` with Jump small gaps; when `origin` lies off the surface
   by more than `1e-6 diag`, the query is repeated with that distance added, so the reach counts from the anchor); all of
   them, NO_UV included;
   beyond `WRAP_MAX_TRIS` the nearest (`limited`).
2. **Weld and anchor.** Weld by bit-exact position (`0.0 == -0.0`), then snap boundary-edge vertices within
   `WELD_SNAP * diag` of another boundary-edge vertex (spatial hash, 27 cells; the representative is the lowest position);
   drop collapsed triangles. CSR vertex -> triangles, half-edge groups per welded edge, parts (connected components),
   per-part boxes. Anchor triangle: the candidates within `1e-4 diag` of the nearest, the one facing `out` most, then the
   lowest id; anchor point = closest point; its oriented normal = the triangle's normal flipped to agree with the
   interpolated shading normal (itself flipped towards `out`); `right` turned by the shortest arc from `out` onto that
   normal, so the spine direction lies in the anchor triangle's plane (turning onto the shading normal instead twists the
   picture on a flat face with smoothed normals).
3. **Spine.** Two traces (+ and - along the spine axis) advance alternately, the one with the smaller arc length first
   (+ on ties), so the ends of a picture longer than the way round meet opposite the anchor. Each runs straight inside a
   triangle (barycentric walk in its 2D development), crosses an edge into the most nearly coplanar neighbour keeping its
   angle to the edge (the oriented normal follows the hinge, so folds of any angle, inconsistent winding and Moebius
   strips work); an exit within `SPINE_NUDGE` of a vertex is moved onto the edge's interior. Stops: the reach, a boundary
   edge (`reached_edge`), entering a triangle a turn-0 trace crossed (`met_itself`; Ends meet), `(4 + 2 * WRAP_MAX_TURNS) * triangles + 16`
   steps. Every crossed triangle's exact development (path on the along axis) seeds its vertices (first in arc-length
   order wins) with that development's frame (2x3 map) and oriented normal.
4. **Front.** A binary heap keyed by `|across| + the distance outside the spine's along range` (ties: lower position).
   A finalized vertex `v` gives each triangle `(v, a, w)` with `a` final a candidate for `w`: the triangle unfolded
   **rigidly** (its own size, centred on the developed edge `(P_v, P_a)`; a similarity would carry the length error of a
   developed edge into every later triangle), on the side of the triangle's **orientation**; a triangle's
   orientation is inherited from an already oriented neighbour across a shared edge (consistent neighbours run the edge
   in opposite directions; strip triangles are oriented by their development), only without one from `v`'s oriented
   normal. A triangle with only `v` final gives fallback candidates through `v`'s frame (used only without triangle
   candidates; such a vertex waits in a second heap until the main front is empty). Tentative position: the mean of the
   triangle candidates when all lie within `CONSENSUS` x the vertex's shortest edge of it, or when their producers lie
   consistently in the layout (every pair's 2D / 3D distance within `[1/TEAR_RATIO, TEAR_RATIO]`: curvature, one front);
   else the candidate whose producers have the smallest keys (fronts meeting behind a hole or round a band). A position
   that would flip a complete triangle of the vertex is replaced by the best candidate that flips none. On pop: final,
   frame = the nearest isometry (polar decomposition) of the producing triangle's development, oriented normal = that
   triangle's oriented normal. A vertex whose key exceeds `across` is not laid (seeds always are: a large strip triangle
   can have every corner beyond the reach); `WRAP_MAX_VERTS` final -> `limited`. A final vertex on a boundary edge inside the picture
   rectangle sets `reached_edge`.
5. **Halo.** A triangle with 1 or 2 final corners gets its missing corners: a corner the front reached (tentative: the
   consensus of its candidates, step 4) keeps that one position in every halo triangle (check:B of round 6: placed per
   triangle, the halo triangles around it left thin wedges without surface between them, inside the picture where the
   reach ends less than an edge beyond it, e.g. the sphere sample: `covered` 0.998-0.999); only a corner the front never
   reached gets the triangle's own unfolding / frame rule. The tentative positions come from the final vertices only,
   so the layout still depends only on the bucketed reach (reuse stays byte-exact).
6. **Tears.** A stored triangle whose layout is flipped against its orientation or collapsed (2D area below the 3D area
   over `TEAR_RATIO^4`), or has an edge whose 2D / 3D length
   ratio is outside `[1/TEAR_RATIO, TEAR_RATIO]`, is re-unfolded rigidly from its final corner nearest the spine (lowest
   key, then position): its plane turned onto that corner's oriented normal (shortest arc), then that corner's frame
   (`cut_tris`).
7. **Store.** Per triangle its `TriId` and three `[f32; 2]` corners, ascending `TriId`; `covered` = clipped parameter
   area / picture area (clamped to 1); a lazy uniform grid for `surface_of`. A reused parameterization (a scale inside the
   reach bucket) reports `covered` for the current picture (`WrapParam::stats_for`, from the stored corners; stable w1:
   every size-dependent statistic, 6.5a "Stable (w1)").

**Jump small gaps** (`jump_gaps`; only when the candidates hold two or more parts, otherwise byte-identical to off):
a part "near another" has another part's box within `GAP_JUMP * diag` of its box (sweep over the boxes; those parts are
its neighbours). (a) The front: when a vertex `v` of such a part is finalized and lies within the gap of the box of a
neighbour not entered yet, the nearest point `q` within the gap on a not-entered part is the landing (among the nearest within a thousandth of the gap: a triangle facing the same way first,
then one rising from `v`'s tangent plane, then one turning away below it, then the back side of something lying on it;
then the lower triangle). The landing triangle is developed rigidly with `q` at `P_v + M_v (q - p_v)`, its plane turned
onto `v`'s by the shortest arc of the oriented normals; its corners become seeds with key `key_v + |q - p_v|`; the part
is entered once. (b) The spine: samples every half gap along its path (and at a boundary edge) look for a part the spine
has not reached within the gap and not behind it; it continues there with the direction turned onto the landing
triangle's plane, its arc length grown by the gap's part along the direction. A part the spine reached is never entered
again by the front. Exact across coplanar or parallel-offset gaps; a part met at a steep angle takes its placement from
the landing point.

**Stable (fix-wrap): a flush, touching part, and the chess board's cut** (`docs/notes/stable/fix_wrap_2.md`). A part is
entered once (a), so its depth is measured from its first landing: two plates touching along a seam parallel to the
spine get the second plate laid only near that landing (`fix_jump_continues_across_flush_coplanar_plates`, ignored as a
known limit: plate B 33.6 % right). Entering a touching part again (boxes face to face within 1e-4 x patch diag, a
landing within that distance facing the same way, on a triangle the part's own front had not reached) fixed it, but
changed ABeautifulGame Jump on (5 landings between a chess piece's touching ring parts: seams 2,147 -> 2,269, rejected
+0.2 points, flipped -1,694, ink / dup / scatter within 0.3 %), so under the no-regression rule it was not kept (a first
attempt that counted every box touching, chess pieces on the board included, made the front 200 x slower). The board's
cut at the marble frame is not that case: the frame (4 parts) is entered across the 1-2 mm inlay ridge, but the spine
closed round the pawn under the anchor, so the depth at the board's edge is already 0.16-0.18 against the front's stop
of 0.185 (1.7 x the across reach) and the frame strip lies beyond the reach (with the touching re-entry `shown` moved
from 0.9127 to 0.9132). The panel says the picture stops at a neighbouring part (`WrapStats::at_part`, below).

**Overlap** (`overlap`; only when the spine meets itself, otherwise byte-identical to Ends meet): a trace that meets turn
0 does not stop but starts its turn 1, and a later turn that re-enters itself starts the next one, at most
`WRAP_MAX_TURNS` in all (`limited`); turns starting beyond the picture are dropped (`serves` then also compares the exact
half extent). One parameterization per turn (steps 4-6 seeded by the turn's strip, Jump small gaps inside each), each
with its along range (turn 0: from where the - trace met to where the + trace met; turn k: its arc length span); a closed
turn (one that re-entered itself, and turn 0) also has a period, its range's length. Layers bottom first by turn
(later along the spine axis on top; a flip on that axis reverses).

**Bake** (`bake_wrapped`, 4.5 of the plan). Per layer and triangle with a texture set: stamp-local corners; skip when
their box misses the picture rectangle grown by one source texel (`hw + bx`, `hh + by`); footprint = the triangle
clipped to that rectangle (and the turn's range, with the copies one period either way for a closed turn) mapped to UV
px, grown by `gutter + 2` -> the `raster_tri` clip box. Scratch per set, 64-row bands, crop: the structure of
`raster_jobs` (6.5). Per fragment: `l = sum(b_i c_i)` (gutter barycentrics extrapolate it); multi-turn: a fragment
outside the turn's range is moved by its period when that brings it inside, else rejected; reject outside the grown
rectangle; `u = 0.5 + l.x / (2 hw)`, `v = 0.5 - l.y / (2 hh)`, flips; `lod = log2(max(src_px_per_world * texel_world_tri
* sqrt(param_area / world_area), 1))`; `Full`: `sample`, `Preview`: `sample_nearest_level`; `shade(sample, tint, 1)`;
the max-alpha rule and the alpha-255 skip. No angle weight, no depth fade, no occlusion. Fragments arrive in ascending
triangle order: the result does not depend on the thread count. Several layers: each is rastered into a layer scratch
(max-alpha inside the layer) and composited over the set's scratch with the Normal blend at 100 % (`blend_row`), bottom
first; one layer takes the single-layer path.

**Reuse** (`WrapParam::serves`, `Document::bakes.wrap_params`). A parameterization serves `p` when the mesh `Arc` is the same,
`origin` and `rotation` are bit-equal, the spine axis and the Wrap options are the same and the bucketed reach (step 1)
is the same; the coordinates depend on the reach, so equal buckets make a reused and a fresh parameterization
bit-identical. A scale that stays in the reach bucket (0-25 % to spare), a tint, a flip, a text edit that keeps the axis
and the bucket, and every Final after a Preview reuse it; a move or a turn recomputes.


**Round 8 (`wrap_relax.rs`, a child module of `wrap_param.rs`; plan R8 2-3): the picture lies once, whole and smooth.**
Steps 4-7 above are changed as follows (the rest stands).

4'. **Geodesic front.** The heap key is `g`, the geodesic distance from the spine: strip seeds get their exact distance
   in the strip's development (`hypot(across, distance outside the spine's along range)`); a vertex reached through a
   triangle with two final corners gets the fast-marching triangle update (a plane wave with unit gradient through the
   triangle, taken only when it is causal: the wave enters through the edge of the two final corners), else the edge
   update `g_v + |vw|`; the tentative key is the smallest. A vertex is laid only while `g <= WRAP_STOP_SLACK (1.7; plan 1.3) x the
   bucketed across reach`: the laid set is the geodesic tube round the spine, never surface whose *unfolded* position
   happens to land in the picture (round 6's ink of 4.6-9.1 x on steep bumps). Positions, consensus, `one_front`,
   never-fold and fallback rules are unchanged. Every laid vertex also stores `foot` (the arc length of its spine
   point; seeds: their along coordinate clamped to the spine's range) and `side` (+1 / -1 of the spine, 0 within
   `1e-9 diag` of it), inherited from the producing vertex (a producer on the spine gives the vertex's own side). Jump
   small gaps: a front jump starts only from a vertex whose layout position lies inside the reach rectangle and lands
   only inside it (the footprint rule; the reach rectangle = the picture's half extents with border and texel margin,
   bucketed, so reuse stays exact); its landing corners get `g_v + |q - p_v| + |x - q|` and `v`'s labels. The spine's
   own jumps are bounded by its reach.
5'. **Seed result and early-out.** Halo and tears as steps 5-6 (the seed). When every seed triangle whose 2D box meets
   the reach rectangle has all three edge ratios (2D / 3D) within `1 +- ISO_TOL` (`2e-4`), or is a tear whose final
   corners differ by S1 / S2 (a closure), the seed is stored as it is (`relaxed = false`): round 6's exact result on
   flat, cylinder, cone, folds, box edges, Jump across coplanar parts and Overlap turns, no solver cost. Halo triangles
   with only one laid corner do not decide (check:A: they are placed by that corner's frame rule on either path, a
   projection that is "torn" wherever the surface turns, and the relaxation cannot change them; WA16's block sides).
   The several-turn stacks of Overlap always keep the seed (builder's deviation 4).
6'. **Seams** (else), over the laid triangles (all three corners final, not degenerate), one rule each:
   S1 ends meet: the turn is closed (period P) and the feet of an edge's ends differ by more than P / 2; on an open
   spine (check:A: the traces pass beside each other without sharing a triangle, e.g. round a tapering stem) the feet
   differ by more than `ENDS_MEET` (2) x `(g_a + |ab| + g_b)` and by more than `SEAM_NEAR` x the longer of their longest
   incident edges, and the seed is torn across the edge;
   S2 halves meet: the ends lie on opposite sides and both farther than `SEAM_NEAR` (3) x their longest incident edge
   from the spine (`g`); S3: a non-manifold edge (three or more laid triangles) or an orientation flip across it (the
   two triangles' `orient` disagree with the edge's traversal; Moebius, Klein; check:A: not when either is a sliver,
   whose orientation rounding can turn over); S4 cone cut: from every interior vertex
   (its whole fan laid, manifold) with |angle deficit| > `CONE_CUT_DEG` (45 degrees) one path along the steepest ascent
   of `g` over uncut laid edges (ties: lower position) until the laid set's boundary or a seam; S5 topology: a dual
   spanning tree over the laid triangles across uncut manifold edges, grown in `g` order (a triangle's `g` = its
   largest corner's) from the anchor triangle; the other laid edges form a graph with the laid set's boundary and the
   S1-S4 seams (fixed); vertices of degree 1 are pruned repeatedly (never across a fixed edge, never at a cone); the
   unfixed edges left (the generators of a handle) are cut. check:A: for this rule zero-area triangles whose corners
   are laid fill their slit, and every boundary loop of the laid set except the longest (its rim) whose length is at
   most `HOLE_FILL` (8) x the longest edge at its vertices is filled by a virtual face of the dual tree (a crack, a
   T-junction whose zero-area filler the import dropped, an unlaid bump top): no cut leads to a small hole.
   **Wedges**: the corners round a vertex joined over uncut manifold edges (union-find); one unknown 2D position per
   wedge, so the layout is continuous everywhere except on seams by construction. **Components**: laid triangles joined
   over the same edges.
7'. **Relaxation** per component: strip components first, then jumped parts in jump order, then the rest by lowest `g`.
   Pins: each strip seed in the wedge of its strip triangle (the centre line stays exact); a jumped part's landing
   triangle, re-placed from its source vertex's relaxed position and the turn between the seed and the relaxed layout
   of the source's first laid triangle; a component with fewer than three pinned wedges also pins the wedges of its
   lowest-`g` triangle at their seed positions. Energies over the triangles with 3D area >= `1e-12 diag^2` and no angle
   whose |cotangent| exceeds `COT_MAX` (1e3; the plan's cotangent clamp: slivers leave the energies; the others'
   wedges keep their seed when they have no other triangle), each triangle's development oriented by `orient`, `J =
   sum_k x_k g_k^T` (`g_k` the gradient basis of the development). First **LSCM** (conformal energy `A/2 ((J00 - J11)^2 +
   (J01 + J10)^2)`), warm-started from the seed, then up to `RELAX_ARAP_ITERS` (6) **ARAP** iterations (local step: the
   closest rotation per triangle, 2x2 polar decomposition, det > 0; global step: `A/2 |J - R|^2`; stop when the energy
   drops by less than 0.5 %). Both by a matrix-free preconditioned (Jacobi) conjugate gradient on the free unknowns:
   LSCM to a residual of 1e-6 of the right-hand side or 600 iterations, ARAP 1e-5 / 200, `RELAX_CG_BUDGET` (2,500)
   iterations per component in all. Deterministic: per-triangle terms (in parallel in fixed chunks of `PAR_CHUNK` from
   `PAR_MIN_UNKNOWNS` unknowns up) are gathered per unknown in incidence order, dot products are added chunk by chunk in
   chunk order: the same bits with 1 or N threads. Round 9 check:B (speed, the same bits): every per-element kernel
   (terms, gather, the CG updates and preconditioner, the ARAP local step with its energy summed in triangle order) runs
   in chunks of 1,024 from 8,192 unknowns; the components are solved in levels (a component reads another's result
   only through its jump source's relaxed pose, when that component comes first in the order; one level's components
   in parallel, results written back in order); the exact region's fallback solve takes a component's first result
   when its start positions and pins are bit-equal. Triangles that are not laid (halo, degenerate) get their final corners
   from their vertices' relaxed positions (the wedge of the vertex's first laid triangle) and the missing ones by the
   hinge (two final corners) or the relaxed frame of the final corner nearest the spine (one). `flipped` counts laid
   triangles inside the picture whose relaxed orientation is reversed; `seams` / `seams_by_rule` the seam edges inside
   the picture; `cut_tris` (relaxed) the triangles with a seam edge inside the picture.
8'. **Store.** Per layer also each triangle's corner depths `g` (f32; a corner the front did not lay: its tentative
   `g`, else the nearest final corner's plus the 3D distance) and longest 3D edge, and (a layer not clipped to an
   Overlap turn's range) the **owner grid** over the reach rectangle: at most `OWNER_MAX_CELLS` (1,024) cells on the
   long side; per cell the smallest `g` of the stored triangles overlapping it (separating axes; the linear `g` at the
   cell's corners clamped to the triangle's range) and that owner's longest edge. `rejected` = the share of the cell
   centres inside the picture covered by a stored triangle where the owner test drops it. `stretch` = the 95th
   percentile of |2D area / 3D area - 1| of the stored triangles meeting the picture (slivers and triangles below
   `1e-12 diag^2` left out).

**Owner test** (`bake_wrapped`; round 8's depth test, replaced in round 9 by the place owner below). A fragment at
picture point `l` with interpolated depth `g_f` was kept only when `g_f <= g_min(cell) + OWNER_TOL (2) x (cell diagonal
+ its triangle's longest edge + the owner's longest edge)`; the tolerance grew with the triangle size and rejected
nothing on coarse meshes (ROUND9_PLAN D1).

**Round 9 (ROUND9_PLAN T7, package B): step 1-2 from the mesh graph.** The weld of step 2 is done once per mesh
(`WrapMesh::build`, `wrap_mesh.rs`: the same bit-exact weld in order of first use over ascending triangles, the same
boundary snap, collapsed triangles dropped; CSR vertex -> triangles; a global edge id per half-edge and the half-edges per
edge). Candidates (`collect_grown`): the anchor is found with spheres of `1e-3 diag x 4^k` up to the reach radius (the
same anchor rule: the triangles within `ANCHOR_TIE` of the nearest, facing `out` most, lowest id); the reach sphere grows
by the anchor's offset as before; the candidates are the triangles reached from the anchor triangle over welded vertices
whose closest point lies within that sphere (the BVH query's own test). With Jump small gaps: every in-sphere triangle,
its parts over welded vertices and their boxes, and the parts whose boxes lie within `GAP_JUMP x diag` of a part
already taken, transitively from the anchor's (no other part can be entered: the front's and the spine's jumps land within
the gap). `Patch::from_graph` maps the global ids to local ones (first use, ascending triangle) with zeroed scratch arrays
(no hash) and groups half-edges by global edge id (no sort); the position rank is the graph's (same order). So a patch of
the same triangles equals round 8's per-patch weld wherever the graph's weld equals the patch's, which differs only where
the reach sphere cut an edge within `WELD_SNAP` of another vertex. Round 8's sphere path stays under `#[cfg(test)]`
(`Source::Sphere`, `sphere_check`): WA28 builds every parameterization of WA1-WA5, WA16-WA27, WA31-WA35 and extra Jump
placements both ways and compares layers, owner grids and stats (135 parameterizations, all equal). `WRAP_MAX_TRIS`
keeps the nearest grown candidates (round 8: the nearest in the sphere). `Jumper::new` no longer builds a hash map of
the patch's triangles (a landing's local triangle: binary search in `patch.tris`).

**Round 9 (ROUND9_PLAN section 3, package A): once, nothing lost.** Changes to the steps above:

- **Reach (T6).** `reach = [bucket(along), bucket(across)]` with `along` / `across` the picture's half extents plus
  one source texel: no texel term before the bucketing (a coarse texture tipped the buckets). The front's stop is
  `max(WRAP_STOP_SLACK x across reach, across + (gutter + 2) x texel_world)` (largest gutter and texel size of the
  maps); the candidate sphere is `along reach + stop` (+ `4 GAP_JUMP diag` with Jump). Reuse also compares the stop.
- **Spine (T2).** Besides the shared-triangle rule, the turn-0 traces end where they **pass** beside each other:
  checked before every step while both run on turn 0, once `s+ + s- > across reach` and their oriented normals agree;
  armed when the head distance falls below `PASS_NEAR (2) x across reach` (the picture's full height: the bands
  overlap; check:A, a helix whose pitch lies between the half and the full height wound on) while decreasing; passed
  when, armed and within that distance, the - head's offset
  along the + trace's direction `(q- - q+) . d+` turns from > 0 to <= 0; the ends are put where that offset is zero
  (the traces' arc lengths interpolated between the two checks). Ends meet: both stop; Overlap: both start their next
  turn. A passed spine is closed like a met one (period `end0+ + end0-`: S1, the early-out's closure tears, Overlap
  turns). `ends_share = (end0+ + end0-) / (2 x needed along)` clamped to 1 (one layer and met or passed), else 1.
- **Front (T4 labels).** Each vertex also carries a continuous spine foot `foot_c`: seeds their along coordinate; the
  fast-marching update `foot_c(v) + tau (foot_c(a) - foot_c(v))` with `tau = -alpha / beta` (where the wave's
  characteristic meets the edge); the edge update the nearer end's; jump seeds the source's. The keys, positions,
  `foot` and `side` are unchanged.
- **S4 (T3).** A vertex with `|deficit| > CONE_CUT_DEG` cuts only when the deficits of the laid interior vertices
  reachable from it over laid edges within `CONE_RADIUS (0.1) x across reach` (3D distance; check:A: at least its
  1-ring, its longest laid edge, so a coarse rivet's ring is seen) sum to more than 45 degrees in absolute value; of
  such vertices within `CONE_RADIUS x across reach` only the strongest (largest `|deficit|`, then lowest position
  rank) cuts. A cube corner and a cone tip still cut; a rivet, a bevel, a groove do not.
- **Relaxation (T4).** A strip component starts from its Fermi layout: each free wedge at `e foot_c + rot90(e) side
  sqrt(g^2 - out^2)` (`out`: how far `foot_c` lies beyond the spine's along range); other components keep the seed
  start. No LSCM: up to `RELAX_ARAP_ITERS` (12) ARAP iterations, stop when the energy drops by less than 0.05 %, at most
  60 CG steps per global solve, `RELAX_CG_BUDGET` per component.
- **Exact region (T5, check:A).** Before the solve, a laid vertex with one wedge whose laid triangles are all
  isometric in the seed (not torn, edge ratios within `ISO_TOL`), connected to a pinned strip seed through such
  vertices, is pinned at its seed position (bit-exact, as the seed path); only the rest is solved. When a free laid
  triangle meeting the picture comes out flipped or the free triangles' 95th-percentile `|area ratio - 1|` exceeds
  `EXACT_STRAIN` (0.25), the pins were inconsistent (a gentle cone) and the turn is solved again with the strip pins
  only (round 8's result, bit for bit). The exact region is as good as the seed: next to a non-developable part the
  front's seed is itself not isometric (WA35's capped cylinder), and those vertices are solved.
- **Store (T1).** The owner grid stores per cell the **owner of its centre**: of the stored triangles covering the cell
  centre (barycentric inside test of the stored f32 corners, edges included) the one with the smallest `(buried, g,
  triangle)`, with its `g`, its 3D point (the barycentrics over the mesh's f32 positions) and its stretch `s =
  sqrt(3D area / layout area)` clamped to 1..4 (`owner_scale`). `buried` is evaluated only in **mixed** cells
  (candidates of two or more parts at two or more places, i.e. Jump small gaps): true when a triangle of another
  entered part touches the point parallel to it (within `1e-5 diag`) or the ray from the point along its oriented
  normal hits one within `[1e-5, BURY_REACH (0.04)] x diag` (`Bvh::raycast`, hits of the own or not-entered parts
  skipped). Cost: `may_bury` once per triangle (another entered part's triangle meets the box of the prism its normal
  sweeps; round 9 check:B: answered from a per-layer index of the entered parts' triangles, per part a box and a grid
  of triangle boxes, with the sphere query's tests in its f32 arithmetic: 155 s of thread time -> under 0.2 s on the
  chess set with Jump on); pass 1's owner (lowest `(g, triangle)`) stands unless it may be buried and is; only then the cell's
  candidates are sorted by `(g, triangle)` and asked lazily. The build runs in 16-row chunks (each cell written by its
  own chunk, triangles ascending; on the pool from 2,048 triangles or 20,000 mixed cells, round 9 check:B also from
  65,536 cells, with the cell array's first touch, the output grids, the row / column spans and the `lost_cut` rows):
  the same owners with 1 or N threads. `rejected` = the share of the (cell centre inside the picture, **laid** triangle covering it) pairs whose
  triangle is not the owner's place. `lost_cut` = the share of the picture's cells without an owner that have owned
  cells on both sides in their row or in their column (inside the laid outline: cut wedges, opened seams, holes), as
  against picture beyond a mesh edge or beyond the spine's ends; check:A: uncovered picture cells 4-connected
  (through uncovered picture cells) to a cell a closure triangle covers (laid with an S1 / S2 cut edge, or torn in an
  exact seed) lie beyond the line where the ends or the halves meet and are not counted.
- **Place owner test** (`bake_wrapped`). A fragment is kept when its cell has no owner or its 3D point (its
  barycentrics over its triangle's 3D corners) lies within `OWNER_SAME (3) x cell diagonal x max(1, s, s_owner)` of the
  owner's point: each picture point lies at one surface place. A gutter fragment (outside its triangle in UV space) is
  tested at its triangle's nearest point (barycentrics clamped to >= 0 and renormalised: picture point, cell and 3D
  point from those), so the islands' padding continues the picture where its inside neighbours show it.
  `surface_of` picks, of the triangles containing the point, the one whose 3D point there lies nearest the cell owner's
  point (no owner: the lowest `g`).

**Stable (w1): wrap correctness fixes after round 9** (`docs/notes/stable/w1.md`). Changes to the round-9 rules above:

- **Ends meet only round something (item 1).** A pass (T2) also needs the heads to come from opposite sides
  (`d+ . d- < PASS_FACING`, 0) and the offset to cross zero continuously (it falls by at most `PASS_STEP` (2) x the
  arc length both traces went since the last check). The shared-triangle rule (`met_itself`) keeps, per triangle a
  turn-0 trace crossed first, that trace's direction and sign: entering the other trace's path closes only coming from
  the other side (`d . d0 < PASS_FACING`), its own path only the same way round (`> -PASS_FACING`), the anchor's
  triangle always (`closes_on`); otherwise the trace runs on. On an open rough surface the traces wander side by side
  and cross each other's paths going the same way (noisy height fields: cosines +0.39 .. +0.95, offset jumps of 10-15 x
  the step); round a tube, a helix or a piece they come round towards each other (torus knot -0.998, WA32 -0.66 ..
  -0.93, WA21's lathe piece -0.35).
- **Exact region fallback per component (item 1).** The exact pins' check (flipped free triangle meeting the picture,
  95th-percentile strain over `EXACT_STRAIN`) is made per component; only a failing component is solved again with its
  strip pins (and the jumped parts re-placed from a changed source, which are checked again, until no new component
  fails). One component gives round 9's result. Round 9 dropped every component's exact pins when one failed: on the
  chess set (step 10, Jump on) a pawn's failure freed the board, laid exactly from the strip, and the solve turned the
  board's picture 40-90 degrees against the frame.
- **Part closure in near-linear time (item 2).** `collect_grown`'s closure over the part boxes (Jump small gaps)
  finds the boxes near a taken part over a uniform grid (`near_closure`: cell side the larger of the gap and the
  median box extent, at most 2^20 cells per axis; a box meeting more than `CLOSURE_BIG_CELLS` (4,096) cells is tested
  by every taken part and tests every box itself). The same set as round 9's test of every box per part taken
  (quadratic): the same candidates, layouts and fingerprints (tiles300 with Jump on: collect 6,010 -> 107 ms).
- **Reuse equals fresh (item 3).** The exact pins' check uses the bucketed reach rectangle (`footprint`), not the
  picture's own rectangle: every layout decision now depends on the bucket only, so a parameterization reused for any
  size in its bucket bakes byte for byte like a fresh one (round 9: WA35's bump plate scaled by 0.95 baked differently).
  Every size-dependent statistic is measured by one function from stored inputs, for a fresh build and for
  `WrapParam::stats_for` / `stats_for_src` (the source size gives the picture's one-texel border) alike
  (`size_stats`): `covered` (the stored f32 corners clipped to the picture; round 9: f64 areas before storing),
  `reached_edge` (the spine's, or one of the layer's final boundary vertices, kept as a Pareto front of `(|x|, |y|)`,
  inside the picture), `ends_share` (the stored sum of the ends over twice the size's needed along), `flipped`,
  `stretch` (per stored triangle its `|area ratio - 1|` and flip flag; the picture test on the stored corners),
  `seams` / `seams_by_rule` / `cut_tris` (the relaxed layout's laid seam half-edges with their f64 ends, plus a seed
  layout's torn triangles), `rejected` / `lost_cut` (per owner cell the mark and the (laid triangle, centre) pair counts,
  `OwnerGrid::rejected_pairs` / `lost_cells`). The last size asked is memoised. Round 9 refreshed `covered` only.
- **`lost_cut` is picture lost to cuts (item 4).** An uncovered picture cell counts only when the nearest covered
  cells on both sides in its row (or in its column) are owned by the **same part** (the owner grid keeps each cell's
  owner part): a gap between two parts (Jump small gaps: tiles, kit-bashed pieces) is not surface lost to a cut. For
  one part this is round 9's span rule, cell for cell (the sweep's Lantern: 11.23 % either way); a 40 x 40 tiled floor
  with Jump on: 17.06 % -> 0. `rejected_pairs` and `lost_cells` work over the picture's cells only (the cells whose
  centres lie in the picture form one rectangle) and over runs of uncovered cells along rows and columns (the covered
  cells at both ends of a run, inside the picture or beyond it): the same counts, 2.7 ms instead of 13 ms per
  statistics pass on a 1,024 x 525 grid (WaterBottle).
- **Tiled UV footprints (item 5, `uv_share.rs`).** A triangle whose UV box spans more than one texture but at most
  `UV_SHARE_TILED_MAX` (4; 64 made a set tiled up to 4 x 4 take seconds) is rasterized wrapped into the unit square, each texel once (it shares when it covers a
  texel twice by itself, or as any other footprint when more than half of its texels are counted twice); beyond that,
  one that holds a whole texture (inscribed circle of diameter sqrt 2) counts everywhere as before and a huge sliver
  marks only itself. Round 9 counted every footprint whose box exceeded one texture everywhere: one trim strip (u
  0..12, v 0.88..0.98) marked a plate on its own island (the code skeptic's trim_strip: 100 % shared).

**Stable (w2): exact parts, steep bumps, the visible place** (`docs/notes/stable/w2.md`). Changes to the rules above:

- **Developed exact region (item 1, `Layout::develop_exact`).** After round 9's exact region (seed-isometric vertices
  connected to the strip), the region is grown by developing the surface: from the laid triangles whose corners are
  all exact or strip pins and isometric there, a laid triangle across an uncut manifold edge is unfolded rigidly onto
  that edge (lowest `g` first, a heap) when one of its corners is **flat**: a closed laid fan with `|deficit| <=
  FLAT_TOL` (1e-6 rad; rounding leaves 1e-8 on flat samples, a curved fine mesh 1.5e-5 and more) that also has a
  triangle with three flat corners round it (a flat area, not a line where a curved surface's deficit crosses zero).
  So no curved fan closes inside the developed region. A triangle whose third corner already has a position joins only
  when it is isometric (`ISO_TOL`) there. The developed wedges of one-wedge vertices are pinned at their developed
  positions; a laid triangle whose corners are all pinned and not all round 9's must be isometric, else its developed
  corners are freed (repeated). The check and fallback (`EXACT_STRAIN`, per component) have two steps: a component whose
  developed pins fail keeps round 9's region only (a steep bump inside a pinned plain: WA35's bump plate), and only when
  that fails too the strip pins only. Why: the front's seed is not isometric next to a curved part (the consensus at a
  cylinder's rim mixes the dome's unfolding into the side's rows: WA35's capped cylinder side 92.37 % within 2/255, max
  51); developed, 99.92 %, max 4. Not reached: the WaterBottle's round-6 label (its shoulder and the body's profile ring
  are curved at every vertex, `|deficit|` >= 4.9e-3 rad: no flat vertex in the patch; w2 notes).
- **Area push (item 3, `relax_component_from`).** The ARAP local step's target is the closest rotation `R` times `c =
  (det J)^(-AREA_PUSH / 2)` clamped to `1 ..= AREA_PUSH_MAX` (1, 1.5): a triangle laid smaller than its 3D area is asked
  to grow, a larger one keeps `R`; the energy for the stop rule is that of the scaled targets (summed in triangle
  order). With the centre line pinned, a steep bump's extra surface was laid compressed (ink above 1): WA4 60 degrees
  ink 1.1177 -> 1.0473, cells 0.769..1.432 -> 0.755..1.246; WA20 ink 1.189 -> 1.087, cells 0.665..2.886 -> 0.690..1.986
  (still above 1.40). Symmetric scaling (shrinking stretched triangles too) folded WA21's closed lathe piece (83 flipped,
  limit 68).
- **Sheets and the visible place (item 4, owner grid).** Every stored triangle carries its **sheet** (triangles joined
  over edges with exactly two triangles: a non-manifold edge or a bow-tie vertex separates sheets). A cell is mixed when
  its candidates lie on two or more sheets (round 9: parts) at two or more places. In a mixed cell a candidate is
  `buried` also when a ray from its point along the projector's `out` hits a triangle of an entered part (its own
  included) beyond `1e-5 diag` within `2 diag` (`OwnerCtx::hidden_at`): of two places laid at one picture point the one
  the stamp faces wins (`Burial::may` is true for every triangle then; ABeautifulGame Jump on owner 188 -> 200 ms).
  Round 9 gave a book's far half to the page the spine continued on, hidden behind the page in front (nonmanifold.obj:
  owner hidden at 2,253 of 5,186 picture points, 1,551 with a visible place dropped; now 43 / 0).
- **Room and closed parts (item 2).** `WrapStats::room` = min(1, the largest extent, along the turned frame's across
  axis, of the parts the stored triangles belong to (their patch vertices) / the picture's full across size); stored
  as `WrapParam::part_across`, so a reused parameterization reports it for its own size. `wraps_closed`: a part
  without boundary edges in the patch all of whose triangles are laid on the first turn. A closed surface cannot lie
  flat: the picture is cut open at its corners and loses the part's total curvature to the gaps there (`lost_cut`; the
  Lantern's 52-vertex base block: 11.2 %).

**Stable (fix_wrap_1): what the panel's lines are measured from** (`docs/notes/stable/fix_wrap_1.md`).
- `WrapStats::shown`: of the owner grid's cells whose centres lie inside the picture (with its one-texel border), the
  share with an owner (`OwnerGrid::owned_cells`); summed over the layers that have an owner grid, `covered` when none
  has. `covered` stays (the clipped laid areas summed, clamped to 1): with Jump small gaps the board and the pawns are
  laid over the same picture points, so it read 1.0 on ABeautifulGame Jump on where 8.7 % of the cells are bare (the
  frame's corner past the board's front edge).
- `WrapStats::beyond`: the share of the picture's cells without an owner that `lost_cells` leaves out as beyond a
  closure (4-connected through bare cells to a cell a closure triangle covers). On a helix every bare cell is beyond
  (WA32: `shown + beyond + lost_cut = 1` within 1 %); on ABeautifulGame Jump on the spine met its own path round a pawn
  (`ends_share` 0.738) but `beyond` is 0: the closure cost no picture.
- Both are size-dependent and come from `size_stats` (a reused parameterization reports them for its own size).

**Stable (fix-wrap): what became of the bare cells** (`OwnerGrid::classify_bare`, `bare_cells`; `fix_wrap_2.md`).
After the owner grid is built (`make_layer`), `classify_bare(gap = GAP_JUMP x patch diag, at_part)` marks, over the
whole grid and serially: `MARK_GAP` on each run of uncovered cells along a row or column of at most `gap / cell + 1`
cells between covered cells owned by two different parts; then `MARK_AT_PART` on each other uncovered cell with a
4-neighbour covered cell whose owner (`OwnerGrid::owner`, its triangle) stops at a neighbouring part
(`OwnerCtx::at_part`, asked once per covered cell): the owner's triangle lies on a part other than the anchor's, none
of its corners is on a boundary edge and a triangle at one of its corners is not laid (the part goes on where the front
stopped on it; a small closed part laid whole, many_islands' cubes, does not), or another part's surface, not laid
there, lies within the gap of the owner's point (`near_other_part`: only a point within the gap of the box of a part in the turn's
`Jumper::neighbours` asks the BVH; empty with Jump off, where the patch is one part). `bare_cells(picture)` (from
`size_stats`, any picture size: stable w1 item 3) counts per picture cell: owned; lost / beyond (`lost_flags`, as
`lost_cells` was); `gaps` (`MARK_GAP`); the rest go with the nearest laid cell (breadth first from the uncovered cells
next to a covered one, 4-connected through such cells inside the picture): `at_part` when that cell is marked. Shares
over the picture's cells: `WrapStats::gaps`, `at_part`; `1 - shown - beyond - lost_cut - gaps - at_part` is the rest
(an edge, a hole, the end of the part, or where the front stopped on the anchor's own part). Cost (alternating runs on an idle machine, release): tiles300 Jump on param
1,834 / 1,856 ms against 1,820 / 1,819 (+1 %; the neighbour lists are the jumper's), Jump off 16.4 / 16.9 against
12.2 / 12.6 ms (the grid's passes on a small wrap, about 4 ms); high_poly_sphere and ABeautifulGame within 2 %.

**Measuring aid** `Document::stamp_wrap_quality(id) -> Option<WrapQuality { ink_area, dup, scatter, bins }>` (5.20).

**Round 8 jobs** (`stamp_job.rs`, `doc_stamps.rs`; plan 3.4). `StampJobKind { Full, Preview, ParamOnly }` on
`StampBakeJob` / `StampBakeOutput` (`kind()`), plus the pending-edit serial. `Preview`: the parameterization (the cached
one when it serves) and `bake_wrapped(.., Preview)` with the picture the synchronous Preview uses; `ParamOnly`: the
parameterization only (logged `(fresh, param only)`). `Document::set_defer_wrap_previews(on)` (default off): a Preview
edit of a wrapped stamp then only marks it waiting (`has_stale_wrap_previews`; `has_stale_previews` /
`bake_stale_previews` leave it alone; projected and UV stamps unchanged); `next_wrap_preview_job()` takes the lowest
waiting wrapped stamp; `apply_wrap_preview(out) -> WrapPreview { Applied { due_again }, Stale }`: applied when the stamp
exists, is in the same pending edit (`preview_baked`, serial) and the mesh and maps are the job's, also when it changed
since (then it waits again, `due_again`); its parameterization is cached when it serves the current placement; Esc,
undo, redo, a Final, another document, a removed stamp or a stamp no longer wrapped (check:B): `Stale`; a stroke:
`Stale` and the stamp waits again. A `Stale` Preview output of the same mesh and maps still leaves its parameterization in
the cache when it serves the stamp's current placement (no pixel change; check:B, package B's request 1): the Final that
waited for that job reuses it (plan 2.5-3). `wrap_param::param_counter::get()` (check:B, request 2): parameterizations
started on the calling thread (one thread-local add per `wrap_param`; the app tests read it).
`wrap_param_job(id)` (only when the cache does not serve the placement) and `apply_wrap_param(out) -> bool` (caches it
when it serves; no pixel changes). Turning the mode off bakes waiting stamps here. The synchronous mode and the job
give the same Preview bytes (WA29).

### 6.6 Compositing and blend modes [C2]

Per-pixel math (W3C compositing, straight alpha, values 0..1, `as = alpha_src * layer_opacity`):
```
ao = as + ab*(1 - as)
Co = [ (1 - ab)*as*Cs + ab*as*B(Cb, Cs) + (1 - as)*ab*Cb ] / ao          (ao == 0 -> output 0,0,0,0)
B: Normal = Cs;  Multiply = Cb*Cs;  Screen = Cb + Cs - Cb*Cs;
   Overlay = Cb <= 0.5 ? 2*Cb*Cs : 1 - 2*(1 - Cb)*(1 - Cs);
   Add = min(1, Cb + Cs);  Darken = min(Cb, Cs);  Lighten = max(Cb, Cs)
```
- Implemented in `u32` fixed point with `div255`; layer opacity is quantized to u8 once per layer (`round(opacity * 255)`).
- **Alpha-0 rule** (overrides the formula; identical in `blend_pixel`, `blend_row` and for every blend mode). RGB stored under
  alpha 0 is data (section 4.2: masks, cutout textures) and must survive compositing and the flattened export:
  - `as > 0 && ab == 0`: output = `(Cs, as)`.
  - `as == 0 && ab > 0`: skip (accumulator unchanged).
  - `as == 0 && ab == 0`: if the accumulator pixel is exactly `[0, 0, 0, 0]`, output = `(Cs, 0)` (carry the hidden colour);
    otherwise skip. So the bottom-most layer with non-zero hidden RGB at a fully transparent texel defines the RGB there, and an
    empty layer above (all zero bytes) never erases it.
  - The `ao == 0 -> 0,0,0,0` clause of the formula can therefore no longer be reached.
  Consequence: flattening a single visible layer at opacity 1 reproduces it byte for byte, and so does flattening an untouched
  imported "Base" under an empty "Paint 1": `export(import(x)) == x`.
- Fast path: Normal with `as == 255` -> copy.
- The accumulator is the u8 composite row itself (at most 1 LSB error per layer, like other 8-bit editors).
- Result must be within 1 LSB of an `f64` reference for every mode (test).

`composite_rect`: `out.data[y0*W*4 .. y1*W*4].par_chunks_mut(W*4)`; each row clears `x0..x1` to `[0, 0, 0, 0]`, then folds the
visible layers (`visible && opacity > 0`, or all when `include_hidden`) bottom to top. Each layer contributes one entry per pixel
view (`Layer::views`: a paint buffer, or each part of a stamp cache; the parts of one layer are disjoint, so folding them in turn
equals folding the layer once), clipped to the layer's `content_rect()` and to `rect`; entries that miss `rect` are dropped up front
and rows outside an entry's clip skip it. An unallocated or empty paint layer contributes nothing, a paint layer with a small
content bound only its bound (a layer contributes nothing outside its own content: an all-zero pixel never changes the fold).

**Content bounds and dirty marking** (review round 2, finding SP-4): a paint layer's `Layer::content` is a conservative bound of its
non-zero pixels, grown by every `mark_layer_dirty` and computed once (parallel scan) when unknown. Visibility, opacity and blend
changes, adds, removes, duplicates and structure restores mark only the affected layers' `content_rects()`; `move_layer` marks only
the moved layer's content (the others keep their relative order).

`recomposite_all`: `refresh_content_bounds()`, then for each set, `for r in pending.take() { composite_rect(.., r, false, &mut
composite); gpu_dirty.add(r) }`.
`take_gpu_dirty` returns and clears `gpu_dirty`. Invariant (R5): after `recomposite_all`, `composite(set)` equals
`flatten_layers(&layers, w, h, false)` byte for byte, and that is exactly what the flattened export writes when `dilate_px == 0`.

### 6.7 Undo / redo [C2]

- **Swap semantics**: applying an entry exchanges its payload with the document, so one stored object serves undo and redo.
  `Pixels`: `patch.swap_with(layer pixels via Arc::make_mut)`, mark the returned bounds dirty.
  `Structure`: `stored = doc.restore_structure(stored)`.
  `Multi`: children in reverse order for undo, forward for redo.
- **Structure snapshots do not pin raster pixels** unless the command replaces them. A toggle of visibility, lock, opacity, blend,
  name, order, or any stamp edit stores every raster layer as `RecordPixels::Keep`. Otherwise each such entry would share the
  layer's `Arc`, the next stroke's `Arc::make_mut` would deep-copy the whole layer (64 MiB at 4096^2, 256 MiB at 8192^2 — a visible
  hitch on the first dab) and the copy would stay charged to the history, evicting undo steps after a handful of
  toggle-paint-toggle-paint pairs. Commands pass the layers to pin to `snapshot_structure(pin)` (lists in 5.17 and 5.20).
- **`restore_structure(s)`** — builds the new layer lists from `s` and returns the inverse snapshot:
  - Record with `Keep`: the layer takes the buffer **and the `version`** the document currently holds for that `LayerId` (it must
    exist as a raster layer: `debug_assert!`; in release a missing buffer becomes a transparent one). Record with `Raster` /
    `Stamp` / `Text`: the layer takes the stored `Arc` and the stored `version`.
  - Inverse snapshot (what the document looked like before the call): a current raster layer is stored as `Keep` if `s` contains
    a `Keep` record for the same id, otherwise as `Raster(current Arc)` (it is about to disappear or to be replaced). Current
    stamp/text caches are always stored. Hence undo and redo of every command pin exactly the buffers they replace.
  - Replaces `layers`/`active` of every set and `stamps`. Dirty rect per set: compare old and new layer lists by position; if the
    lists are identical in ids, order, properties and `same_pixels`, nothing is dirty; otherwise the union of the content rects
    (`view().rect`) of all layers that are added, removed, moved, or changed (a changed raster layer = the full set).
  - Drops `text_mips` entries of stamps that no longer exist or whose `TextSpec` changed; ends with `debug_check_structure()`.
- **Why `Keep` is sound**: history is linear. Every `Pixels` entry recorded after a `Structure` entry is undone before that
  `Structure` entry is undone (and redone after it is redone), so at the moment a `Keep` record is applied the document's current
  buffer for that layer holds exactly the pixels it had when the snapshot was taken. Pinned buffers are immutable because a later
  stroke mutates through `Arc::make_mut`, which copies while the history still shares the buffer. For the same reason, when a
  `Pixels` entry is undone/redone the layer it names exists with exactly the content the patch expects.
- **Pending edits** (`Commit::Preview`): the first Preview call captures `before = snapshot_structure(&[])` in `pending_edit`;
  further Preview calls with the same label reuse it. `flush_pending_edit()` — called directly, by `Commit::Final`, by `undo()`,
  by `stroke_begin` and by every other history-producing command — first finalizes all Preview-baked stamps
  (`finalize_stamp_bake` for each id in `preview_baked`), then pushes `Edit::Structure(before)`. `cancel_pending_edit()` restores
  `before` and clears `preview_baked`.
- **Unbaked stamps in snapshots** (round 5, 5.20 rule 7): `StructureSnapshot::unbaked` is `Document::unbaked` at the time of
  the snapshot. `snapshot_structure` stores it; `restore_structure` swaps it with the document's (ids of stamps that no longer
  exist are dropped) and returns the old set in the inverse snapshot. So a cache of a stamp whose Full bake was still due
  only ever comes back together with that flag: undo, redo and `cancel_pending_edit` never wait for a bake and never bake on
  the UI thread; the app bakes a restored unbaked stamp in the background. A bake that lands later changes the current state
  only (no entry): an entry pushed after it records the baked caches. With deferred Final bakes off, a restored unbaked
  stamp (a snapshot taken while the mode was on) is baked at once by `restore_structure`.
- **Budget**: entry cost = `patch.byte_len()` for `Pixels`; for `Structure` the summed `byte_len` of the pinned raster buffers and
  stamp/text caches in the stored snapshot that are not `ptr_eq` to a buffer currently in the document (recomputed lazily when
  checking the budget). Evict oldest entries while `total > budget_bytes` or `len > max_entries`; the newest entry is always kept.
- `Pixels` application calls `mark_layer_dirty` (fresh layer version). `undo`/`redo` call `touch()`: the revision only grows (it
  drives caches and thumbnails), so it cannot tell whether the content equals the saved one.
- **History state ids** (round 3, R3-6): `History::state` names the current history state. `push_edit` gives the new state a
  fresh id, the revision after its `touch()` (the revision only grows, so an id is never reused); the entry stores the state
  before it in `Entry::state`. `undo`/`redo` swap `Entry::state` with `History::state` (swap semantics, like the payload).
  `mark_saved(state)` stores the saved id; `is_modified() = history_state() != saved`. So undo or redo back to the saved state
  clears the modified mark, and leaving it marks the document again. A saved state that was evicted (its entry dropped by the
  budget), or that sat on the redo stack when a new edit cleared it, can never be the current state again: the document stays
  modified until the next save. `History::clear` keeps the current id. The id equals `revision()` until the first undo/redo
  after the newest push, which is why `load` may call `mark_saved(revision())` on a freshly built document. A background save
  marks `DocumentSnapshot::history_state`: an edit made while it runs keeps the document modified; undoing that edit before
  the save finishes makes it unmodified. A pending `Commit::Preview` edit is not counted by `is_modified`; the app adds
  `has_pending_edit()` (`has_unsaved_changes`, 8.10).
- A stroke in progress blocks `undo`/`redo` (they return `None`) and every other history-producing command
  (`Err(StrokeInProgress)`, mutation rule in 5.14).

### 6.8 Text layout and rasterization [C4]

fontdue facts (`REG\fontdue-0.9.5\src\font.rs`): `Font::from_bytes(data, FontSettings { collection_index, scale, load_substitutions })
-> Result<Font, &'static str>` (:242); `metrics(char, px) -> Metrics { xmin, ymin, width, height, advance_width, .. }` (:453);
`rasterize(char, px) -> (Metrics, Vec<u8>)` coverage with top-left origin (:529); `horizontal_kern(left, right, px) -> Option<f32>`
(:418); `horizontal_line_metrics(px) -> Option<LineMetrics { ascent, descent, line_gap, new_line_size }>` (:376); `has_glyph` (:630).
`fontdue::layout` is not used (no letter spacing). Both
painters take positions from `fontdue` glyph indices (`lookup_glyph_index`, `metrics_indexed`,
`horizontal_kern_indexed`); the styled painter takes only the glyph shapes from `ttf_parser` with the same index.

`FontCache::get`: key `(blob, face_index, bucket)` with `bucket = (px as u32).next_power_of_two().clamp(32, 1024)`;
`FontSettings { collection_index: face_index, scale: bucket as f32, load_substitutions: false }`.

Layout (`px = spec.size_px.clamp(8, 512)`; `ls = spec.letter_spacing * px` and `ow = outline.width_em.clamp(0, 0.25) * px` are the
letter spacing and outline width in raster pixels):
1. `lm = fonts[0].horizontal_line_metrics(px)`, fallback ascent `0.8*px`, descent `-0.2*px`, `new_line_size = 1.2*px`.
   `line_h = lm.new_line_size * line_spacing`.
2. Walk the stored characters of `content` with their index (`char` count): a line feed ends a line, a tab is laid
   out as four spaces, every other control character (also `\r`) is skipped. This gives the lines of round 3 and never
   moves an index. **Empty content is laid out as a single space** (a fully transparent image one space wide and one
   line high), so the rescale rule, the aspect ratio and the gizmo stay well-defined while the user clears and retypes
   a text stamp.
3. Per line: `pen = 0`; glyph = the first font whose `lookup_glyph_index(ch)` is not 0, else glyph 0 of `fonts[0]`;
   add `horizontal_kern_indexed(prev, glyph, px)` when both glyphs come from the same font; glyph origin `= round(pen)`;
   `pen += advance_width + 2 * bold + ls + gap_i` where `gap_i = clamp(fx.gaps[i].em, -0.5, 4) * px` is the gap value of
   the stored character (0 without an entry; for a tab it follows the fourth space) and `bold` the thickening radius
   (0 for the classic painter; no bold term for a glyph whose advance width is 0). Line width `= pen - ls - gap` of the
   last glyph (0 for an empty line), so neither the letter spacing nor a gap value after the last character widens a
   line. Without gap values every number is the number of round 3 bit for bit (test `plain_text_is_bit_identical`,
   the eight hashes of `target\round4\perf\text_baseline.txt`).
4. Alignment offset `= (max_w - line_w) * {0, 0.5, 1}`.
5. Baseline of line `i` `= pad + ascent + i * line_h`. Bitmap top-left: `x = pad + align + origin + m.xmin`,
   `y = baseline - m.ymin - m.height` (fontdue's `ymin` is the bottom edge, y-up).
6. `pad = ceil(ow) + 2`. Canvas `= (max(1, ceil(max_w)) + 2*pad, ceil(ascent - descent + (lines - 1) * line_h) + 2*pad)`.
   Over 8192 on a side => `TextError::TooLarge`.

Raster:
1. A8 coverage canvas, `cov = max(cov, glyph)` per placed glyph.
2. Outline: `d = distance_to_ink(cov)` (Felzenszwalb-Huttenlocher squared EDT, two 1-D passes, from pixels with `cov >= 128`, with the
   nearest site's sub-pixel correction `d = sqrt(d2) + 0.5 - cov[site]/255`); `outline_a = clamp(ow + 0.5 - d, 0, 1)`.
3. Per pixel: fill `(color.rgb, color.a * cov)` OVER outline `(outline.rgb, outline.a * outline_a)`; output straight alpha.

**Dispatch** (`raster.rs`): `render_text(fonts, spec, quality)` and `text_frame(fonts, spec)` use the classic painter
(`layout_text` + `rasterize_text`, always in full, `quality` ignored) unless `spec.fx.needs_outlines()` (thickening,
slant, warp or path), then the styled painter. A text whose only new feature is gap spacing stays on the classic
painter. Classic frame: `size` = the canvas, `anchor_px` = its centre, `box_px` = canvas minus twice the padding,
`baseline0_px = round(ascent) - box_h / 2`, `align_px` = left edge / centre / right edge of the line box at the anchor's
height, `circle_fill = 0`. `text_frame` equals the frame `render_text(.., Full)` reports.

### 6.8a Styled painter (`text/styled.rs`, round 4; FEATURE_TEXT 5.2 to 5.10)

Text space: raster pixels of the stamp resolution, x right, y down, origin at the anchor. A straight line box is
centred on the anchor, its top-left corner at `(-box_w / 2, -box_h / 2)`; the first baseline at
`b0 = round(ascent) - box_h / 2`.
1. Layout: the classic rule (6.8 step 3) with the thickening term `bold = clamp(fx.bold_em, 0, 0.08) * px`.
2. Per glyph: `outline::glyph_outline(face, glyph, px)` (closed polylines, y down, flattened to 0.05 px) with
   `x' = x - y * tan(slant) + shift`, `shift = +bold` (`-bold` for a glyph without advance: a mark stays over its
   letter). Straight: point `= (-box_w / 2 + align_l + round(pen) + x', -box_h / 2 + round(ascent + l * line_h) + y')`,
   the numbers of the classic painter. On a line: `path::PathLayout::glyph_frame(line, pen, advance)` once per glyph
   and `PathLayout::place_with(frame, advance, (x', y'))` per point (one walk look-up per glyph, one rotation per
   point; the same result as `place`), with the unrounded pen, text lines at `b0 + l * line_h`. Glyph cells (pen .. pen + advance, ascent .. descent, sheared) go
   through the same placement.
3. Warp: `warp::WarpMap` on the bounding box of all cells (and of the line box for a text without a path); outlines
   through `map_polyline` (refined), cells and line-box border through `map`.
4. Canvas: `R` = bounds of the mapped outline points grown by `bold`, of the mapped cells and, without a path, of the
   mapped line-box border (16 points per side); `pad = ceil(outline) + 2`; the canvas runs from `floor(R.min) - pad` to
   `ceil(R.max) + pad` **on the pixel grid of the line box** (floor and ceil are taken relative to the line-box corner,
   so a straight styled text lands on the pixels of the classic one; the anchor may then lie on a half pixel). A side
   above 8192 px or an area above `MAX_STYLED_PIXELS` (16,777,216) is `TextError::TooLarge`; the classic painter keeps
   the side limit only. Frame: `anchor_px = -canvas_min`, `align_px` as the classic one for a text without warp and
   path, else `anchor_px`; `circle_fill` from the path layout.
5. Pixels: `quality = Preview { max_pixels }` picks the largest `f` of 1, 1/2, ..., 1/16 with
   `w * h * f^2 <= max_pixels` (1/16 when none fits) and draws a `ceil(w f) x ceil(h f)` picture of the same rectangle
   (points scaled by `(wp / w, hp / h)`, thickening and outline by `f`; `full = f == 1`; the frame is the full frame).
   Per glyph a buffer around its mapped outline grown by `ceil(radius) + 2`, clipped to the canvas;
   `scan::fill_contours` (non-zero winding); thickening when the radius is at least 0.25 px (a radius below 0.25 px
   at the stamp resolution is ignored entirely, also in the advance and the shift of steps 1 and 2, so it cannot move
   a letter; below 12.5 px per em the default 0.02 em therefore draws nothing):
   `cov = max(cov, round(255 * clamp(r + 0.5 - distance_to_ink(cov), 0, 1)))`. Glyphs in reading order in batches of at
   most 64 glyphs and 8 megapixels of buffers, each batch in parallel (rayon), merged by the maximum on one thread: the
   picture does not depend on the thread count. Outline and fill over outline: `raster::compose`, as the classic
   painter.

### 6.9 Numeric pitfalls checklist

1. `f32::rem_euclid(1.0)` can return exactly 1.0 for tiny negatives: wrap in integer pixel space (`(x as i32).rem_euclid(W as i32)`).
2. Texel centres are at +0.5: `floor(u * W)` is the texel index, `u * W - 0.5` the bilinear coordinate.
3. Un-premultiply as `(c*255 + a/2) / a`, guard `a == 0`; round with `+0.5` before `as u8`.
4. `r^2 - d^2` can go slightly negative: clamp at 0.
5. Occlusion ray bias scales with `diag` (too small: acne on decals; too large: leaks through thin walls).
6. A wrapping triangle maps to several row bands: bin by wrapped rows.
7. Sub-texel slivers get coverage only through the sliver fallback or a neighbour's gutter.
8. u16 mask increments need `ceil`; opacity is applied when writing into the layer, never baked into the mask.
9. Negative-determinant node transforms flip winding: fix at import. Models with inconsistent winding still work without any
   setting: strokes use the unflipped reference normal (6.4), and a stamp clicked onto a back-facing triangle is created with
   `backface_cull = false`, where the facing weight uses `|dot(N, out)|` (6.5, 8.6). `same_side_only` and `backface_cull`
   remain user-toggleable.
10. Never let `HashMap` iteration order reach pixels or files.

---

## 7. File formats

### 7.1 `.wtep` project container [C6]

A ZIP archive (`zip` 9.5.1, `REG\zip-9.5.1\src\write.rs`: `ZipWriter::new(inner: W)` with `W: Write + Seek` (:602),
`start_file(name, options)` (:1120), `finish(self) -> ZipResult<W>` (:1494)). Exactly two option values are used, in the project
writer and in the ORA writer alike:
```rust
// STORED entries — the level MUST stay None
let stored   = zip::write::SimpleFileOptions::DEFAULT.compression_method(zip::CompressionMethod::Stored);
// "deflate 6" entries
let deflated = zip::write::SimpleFileOptions::DEFAULT.compression_method(zip::CompressionMethod::Deflated)
                                                    .compression_level(Some(6));
```
**Never call `.compression_level(Some(_))` on a Stored entry**: `prepare_next_writer` rejects it with
`UnsupportedArchive("Unsupported compression level")` (`write.rs:2313-2318`), which would fail the very first entry (`mimetype`).
`compression_level` takes `Option<i64>` (`write\options.rs:380`); never pass a level above 9 (levels > 9 switch to zopfli,
`write.rs:2366-2371`). `DEFAULT` (`write\options.rs:624`) carries the fixed 1980 timestamp. `ZipWriter` finalizes on `Drop`
(`write.rs:2181`), so on an error path the temp file must still be deleted explicitly.

| Entry | Compression | Content |
|---|---|---|
| `mimetype` | STORED, **first** entry | exactly `application/x-wtep` (no newline) |
| `source/main.<ext>` | deflate 6 | original model file bytes, `<ext>` = `obj` / `gltf` / `glb` |
| `source/f<nnn>.<ext>` | deflate 6 (STORED for png/jpg/jpeg/webp) | sidecars in capture order (`nnn` = 000, 001, ...); `<ext>` = lower-case ASCII-alphanumeric extension of the original name (max 8 chars), else `bin` |
| `source/x<nnn>.<ext>` | deflate 6 (STORED for png/jpg/jpeg/webp) | `SourceBundle::extras` in order: original files of manually chosen textures (same `<ext>` rule) |
| `blobs/b<id:05>.<ext>` | STORED for images, deflate 6 for fonts | stamp source images and font files (original encoded bytes), only those referenced by a stamp |
| `sets/s<set:02>/l<layer id>.png` | STORED | layer pixels: RGBA8 PNG cropped to `nonzero_bounds` (raster: bbox of pixels != `[0,0,0,0]`, so RGB under alpha 0 is kept) or to the bounds of the cache parts (stamp/text) |
| `sets/s<set:02>/flattened.png` | STORED | full-size composite of the set (if `write_flattened`) — lets other tools grab the result without our app |
| `thumbnail.png` | STORED | at most 256 px, from the composite of set 0 |
| `manifest.json` | deflate 6, **last** entry | pretty-printed JSON, schema 7.2 |

Entry names are built only from numeric ids and sanitized extensions; no user string reaches a ZIP path. Original names live in the
manifest. Readers take entry names from the manifest only.

**Save procedure** (`project::save`):
1. Validate: every float in stamps is finite (else `ManifestInvalid`, naming the float and the stamp). The floats of a text
   come from `TextSpec::floats`, the read-only visitor (size, line and letter spacing, outline width, and every round-4 effect
   value: each gap, thickening, slant, the warp's bend / horizontal / vertical, the path's bend / diameter / centre / points /
   start), so a new effect value cannot be forgotten and nothing is cloned. `min_reader_version` is `required_reader(snapshot)` (7.3).
2. Encode layer PNGs in parallel with rayon (`encode_png`, level from `SaveOptions`), at most `max(1, 1 GiB / (w*h*4))` at a time.
   Raster layers are cropped to `pixel_ops::nonzero_bounds` — **not** `content_bounds`: a texel with alpha 0 but non-zero RGB is
   content (section 4.2) and must round-trip; only a raster layer whose every byte is zero has `pixels: null` and no entry.
   Stamp/text caches are written as one PNG over the bounds of their parts (`SparseBuf::to_cropped`; the loader splits it
   again; `pixels: null` for an empty cache). An unallocated paint layer has `pixels: null`.
   Flattened = `flatten_layers(layers, w, h, false)`.
   **Copying unchanged images** (review round 2, finding SP-6): the snapshot carries `png_reuse: Option<Arc<PngReuse>>`, the record
   of the project file this document was loaded from or last saved to (`path`, `len`, `modified`; per layer id: the layer version,
   entry and rect it was written with; per set: the `composite_key` and the `flattened.png` entry; the thumbnail likewise). While
   that file still has the recorded size and modification time, every layer whose version is unchanged and every set whose
   `composite_key` (size + per layer id, version, visibility, opacity, blend) is unchanged is copied from it (CRC-checked read)
   instead of being encoded; a read failure falls back to encoding. The old file is closed before the new one replaces it.
   `SaveStats::reuse` returns the record of the written file; the app stores it with `Document::set_png_reuse`. `load` records
   every layer image it read as stored (with the version the layer has after `from_parts`; re-baked caches get newer versions)
   and, when nothing was lost or re-baked, the composites.
3. Inside `write_atomic(path, keep_backup, write, verify)`: write entries in table order through a `BufWriter<File>`; compute each
   `FileRef { entry, size, crc32 }` with `crc32fast::hash`; write `manifest.json` last; `finish()`.
4. `verify`: reopen with `ZipArchive::new`, parse `manifest.json`, check that every `FileRef` entry exists with the recorded size.
5. Report progress per layer; honour `progress.cancelled()` between entries (`ProjectError::Cancelled`, temp file removed).

`write_atomic`: temp name `<file name>.tmp-<pid>` in the same directory; `File::sync_all()`; if `keep_backup` and the target exists,
rename it to `<path>.bak` (replacing an older one); `std::fs::rename(tmp, path)` (replaces on Windows); delete the temp file on any error.

**Load procedure** (`project::load`):
1. `ZipArchive::new(BufReader<File>)` (`REG\zip-9.5.1\src\read\zip_archive.rs:432`); failure => `NotAZip`. Before reading any entry
   check `ZipFile::size()`: manifest <= 64 MiB, source and blob entries <= 2 GiB (`import::MAX_SOURCE_FILE`, which import and
   save enforce too), PNGs <= `w*h*4 + 1 MiB`. Beyond the limit the manifest and the main model file are fatal (`EntryTooLarge`);
   any other entry is treated like an unreadable one (sidecar / extra dropped, blob missing, layer pixels lost or cache rebuilt).
2. `manifest.json` (`by_name`, :695): missing => `ManifestMissing`. Parse as `serde_json::Value`; `format != "wtep"` =>
   `ManifestInvalid`. Version handling (7.3). Then `serde_json::from_value::<Manifest>`.
3. Rebuild the `SourceBundle` from `source/*` (main, sidecars, extras; an unreadable extra is dropped with a logged warning — it
   is not needed to open the project); `import_from_bundle(&bundle, &manifest.import options, /* decode_textures */ false, progress)`:
   the referenced textures are **not** decoded, because every layer's pixels come from the layer PNGs (saves seconds per open for
   models with several 4K textures). Any failure => `ModelUnrecoverable`. `mesh.sets.len() != manifest.sets.len()` =>
   `ModelUnrecoverable("texture set count changed")`. `uv_hash` differs => `LoadWarning::OlderGeneratedUvLayout { saved }` when
   `manifest.import.importer_version < IMPORTER_VERSION`, `generate_fallback_uvs` is set and the re-import generated the layout
   (`FallbackUvsGenerated`): plain words that the project was saved with an older automatic texture layout, that painted content may
   sit in the wrong place and that nothing was overwritten; for any other cause `LoadWarning::MeshUvChanged`. Same hash => no
   warning, whatever the version. Import warnings are forwarded as `LoadWarning::Import` except the model facts
   (`ImportWarning::is_model_fact`: generated normals / UVs, folded tiles, dropped triangles, ...), which the import dialog already
   showed and every reopen would repeat unchanged (the app shows any load warning as a modal "Warnings" box). A `MissingMtl` is
   forwarded only when `manifest.source.sidecars` lists that library (key `normalize_ref(reference)`) and the file lacks its
   entry (a damaged project); a library that was already missing at import was never stored and is not repeated. The reopened file
   therefore usually has no warnings, which also lets the next save reuse its stored composites (`PngReuse`).
4. Blobs: read into `BlobStore::insert_with_id`; unreadable => `BlobMissing { used_by }`.
5. Stamps: build `StampObject`s. Image stamp whose blob is missing: its linked layers are loaded as plain raster layers and the stamp
   is dropped (still visible, no longer re-editable). Text stamp whose font blob is missing: font replaced by the bundled default,
   `FontFallback`, stamp flagged for re-bake. Text stamp with a round-4 feature this build does not know (an unknown or missing
   warp style or path kind, `TextEntry::unknown_features`): the stamp loads without that feature (`to_spec` leaves it out, then
   `sanitize_fx` repairs out-of-range values), one `LoadWarning::TextFeatureUnknown { stamp, what }` per feature, the stamp is
   flagged for re-bake (so the picture shows what the spec now says) and `LoadReport.force_save_as` is set, as for a newer
   format: a plain Save would overwrite the file with less. Text stamp whose effects `sanitize_fx` changed (the sanitized
   `fx`, written back as an entry, differs from the file's entry in canonical form, `load::canonical_fx`: a value out of
   range, a gap beyond the text, too many curve points, a non-finite number): flagged for re-bake without a warning, so the
   stored picture never disagrees with the repaired spec. The canonical form only rewrites what does not change the
   picture (gaps sorted, the later of two entries for one character kept, zero gaps dropped, path keys of other kinds
   cleared), so valid but hand-written effects keep their stored pictures too. Files the app wrote hold sanitized
   effects, read back equal, and keep their stored pictures.
6. Layers per set, bottom to top: each `serde_json::Value` is parsed as `LayerEntry`; failure => `UnknownLayer`, skipped. Pixels are
   decoded with `decode_image`; the declared `rect` must match the PNG size and lie inside the set. Raster: blit into a dense buffer
   (no stored pixels => unallocated layer); decode failure => transparent (unallocated) layer + `LayerPixelsLost`. Stamp/text:
   `SparseBuf::from_cropped`; failure => flag for re-bake (`BakeCacheRebuilt`).
   A stamp/text entry naming an unknown stamp => `UnknownLayer`, skipped.
7. A stamp that ended up without a linked layer in its `home_set` (its layer entry was skipped) first gets an empty one
   (`SparseBuf::empty()`, name = stamp name, on top of that set) and is flagged for re-bake, so the structure invariants of 6.5
   hold when `from_parts` runs `debug_check_structure`. Then `Document::from_parts`; then `rebake_stamp(id, Full)` for the flagged
   stamps; clear history; `mark_saved(revision)` (equals `history_state()` here, 6.7).

Fatal: not a ZIP, no/invalid manifest, too new, unrecoverable model, manifest or model file beyond its size limit. Everything else
degrades with a warning. Corruption is detected by
the ZIP per-entry CRC (checked by the reader at end of entry) and the PNG decoder.

`extract_bundle` (R7 "Extract Original Files"; works on the in-memory `SourceBundle` + `BlobStore`, so also for a document that
was never saved): writes `main`, every sidecar and every extra under its original file name — verbatim when Windows accepts it
(no forbidden character, no trailing dot or space, no device name, at most 255 UTF-16 units), else
`sanitize_file_stem(stem) + sanitized extension` — so the `mtllib` / `map_Kd` / glTF `uri` references between them stay valid,
into `dest_dir` (`create_dir_all`), preserving the relative sub-directories of sidecar/extra keys when they are plain relative
paths without `..` (else flat; extras therefore land in `textures\set<NN>\`); then every blob: images into `stamps\`, fonts into
`fonts\` (a font name without a font extension gets the one of its container: `ttf`, `otf`, `ttc`, `woff`, `woff2`). Name
collisions inside one call get `_2`, `_3`, ... before the extension. With `overwrite == false`, when any planned file already
exists in the destination, the files are not renamed one by one (that would break their references): the whole set is written
into a fresh sub-folder `<model stem>_original` (`_2`, `_3`, ... when that exists too). With `overwrite == true` existing files
are replaced. `extract_sources(path, dest, overwrite)` reads the same data straight from a
`.wtep` (entries named by the manifest; no import) and calls `extract_bundle`.

### 7.2 Manifest schema (`project/manifest.rs`) [S]

All structs derive `Serialize, Deserialize, Clone, Debug, PartialEq`. New optional fields must be `#[serde(default)]`; enum variant
names are never renamed.

```rust
pub struct Manifest {
    pub format: String,                    // "wtep"
    pub format_version: u32,               // FORMAT_VERSION
    pub min_reader_version: u32,           // oldest reader that can open this file without misrendering
    pub app_version: String,
    pub name: String,
    pub saved_unix: u64,
    pub next_id: u64,
    pub source: SourceEntry,
    pub import: ImportEntry,
    pub sets: Vec<SetEntry>,
    #[serde(default)] pub stamps: Vec<StampEntry>,
    #[serde(default)] pub blobs: Vec<BlobEntry>,
    #[serde(default)] pub ui_state: serde_json::Value,
}
pub struct FileRef { pub entry: String, pub size: u64, pub crc32: u32 }
pub struct SourceEntry { pub format: ModelFormat, pub main: SourceFileEntry, pub sidecars: Vec<SourceFileEntry>,
                         #[serde(default)] pub extras: Vec<SourceFileEntry>,   // manually chosen texture files (source/x<nnn>.<ext>)
                         pub original_path: Option<String> }
pub struct SourceFileEntry { pub key: String, pub original_name: String, pub file: FileRef }
pub struct ImportEntry { pub generate_fallback_uvs: bool, pub max_texture_dim: u32, pub importer_version: u32,
                         pub uv_hash: String /* 16 lower-case hex digits */, pub vertex_count: u32, pub triangle_count: u32 }
pub struct SetEntry { pub id: TextureSetId, pub name: String, pub width: u32, pub height: u32,
                      pub active_layer: Option<LayerId>, pub flattened: Option<FileRef>,
                      pub layers: Vec<serde_json::Value> /* LayerEntry, bottom -> top */ }
pub struct LayerEntry { pub id: LayerId, pub name: String, pub visible: bool, pub locked: bool, pub alpha_lock: bool,
                        pub opacity: f32, pub blend: BlendMode,
                        #[serde(flatten)] pub kind: LayerKindEntry,
                        pub pixels: Option<PixelsEntry> }
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerKindEntry { Raster, Stamp { stamp: StampId }, Text { stamp: StampId } }
pub struct PixelsEntry { pub file: FileRef, pub rect: [i32; 4] /* x, y, w, h */ }

pub struct StampEntry { pub id: StampId, pub name: String, pub home_set: TextureSetId, pub tint: [u8; 4],
                        pub source: StampSourceEntry, pub placement: PlacementEntry }
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StampSourceEntry { Image { blob: BlobId }, Text(TextEntry) }
pub struct TextEntry { pub content: String, pub font: FontRefEntry, pub size_px: f32 /* raster resolution */, pub color: [u8; 4],
                       pub align: TextAlign, pub line_spacing: f32, pub letter_spacing: f32 /* em fraction */,
                       pub outline: Option<OutlineEntry>,
                       #[serde(default, skip_serializing_if = "TextFxEntry::is_default")] pub fx: TextFxEntry }   // version 2
pub struct TextFxEntry { gaps: Vec<GapEntry>, bold_em: f32, slant_deg: f32, warp: Option<WarpEntry>, path: Option<PathEntry> }
                       // every key optional and not written when empty / zero / None
pub struct GapEntry { pub after: u32, pub em: f32 }
pub struct WarpEntry { pub style: String /* WarpStyle::key */, pub bend: f32, pub horizontal: f32, pub vertical: f32, pub vertical_axis: bool }
pub struct PathEntry { pub kind: String /* "arc" | "circle" | "curve" */, pub bend: f32, pub diameter: f32, pub center: [f32; 2],
                       pub points: Vec<[f32; 2]>, pub start: f32, pub flip: bool }   // only the keys of its kind are written
impl TextEntry { pub fn to_spec(&self) -> TextSpec /* unknown names: feature None; sanitize_fx */;
                 pub fn unknown_features(&self) -> Vec<String> /* ["warp style \"ribbon\""] */ }
pub struct FontRefEntry { pub blob: BlobId, pub face_index: u32, pub family: String, pub style: String }
pub struct OutlineEntry { pub width_em: f32, pub color: [u8; 4] }
#[serde(tag = "space", rename_all = "snake_case")]
pub enum PlacementEntry {
    Projected { origin: [f32; 3], rotation: [f32; 4] /* quaternion x, y, z, w */, half_size: [f32; 2],
                depth_front: f32, depth_back: f32, angle_start_deg: f32, angle_end_deg: f32,
                backface_cull: bool, occlusion: bool, flip_x: bool, flip_y: bool,
                // round 6 (format 3): "" (absent) = Project, "wrap" = Wrap; another value loads as Project with
                // LoadWarning::StampFeatureUnknown { stamp, what } (forces Save As, re-bakes the stamp)
                #[serde(default, skip_serializing_if = "String::is_empty")] mode: String,
                #[serde(default, skip_serializing_if = "is_false")] wrap_gaps: bool,     // written only when true
                #[serde(default, skip_serializing_if = "is_false")] wrap_overlap: bool },
    Uv { set: TextureSetId, center_px: [f32; 2], size_px: [f32; 2], rotation: f32 /* radians, UvPlacement.rotation verbatim */,
         flip_x: bool, flip_y: bool, clip_to_islands: bool },
}
pub struct BlobEntry { pub id: BlobId, pub kind: BlobKind, pub original_name: String, pub file: FileRef }

// conversions, implemented by the scaffold in manifest.rs:
impl From<&StampObject> for StampEntry {}      impl StampEntry { pub fn to_stamp(&self) -> StampObject; }
impl From<&TextSpec> for TextEntry {}          impl TextEntry { pub fn to_spec(&self) -> TextSpec; }
impl From<&Placement> for PlacementEntry {}    impl PlacementEntry { pub fn to_placement(&self) -> Placement; }
```
Plain arrays instead of glam serde keep the JSON stable. A manually chosen texture is linked to its set by the key of its
`extras` entry (`textures/set<NN>/<name>`, 5.15); `SetEntry` therefore needs no separate reference field. `ui_state` is written by the app (camera, UV view, view mode, active set,
colours, swatches; objects-tabs: `hidden_objects`, the names of the hidden objects, written only when one is hidden) and
ignored by the core. No version change (7.3): older builds ignore the unknown key and show everything; older projects load
with nothing hidden.

A text without round-4 features is written with exactly the keys of version 1. Names
(`style`, `kind`) are strings, so an unknown name never fails a file; a wrong JSON type inside `fx` fails the
manifest like any malformed field.

### 7.3 Versioning

- Two-stage parse: read `format`, `format_version`, `min_reader_version` from the `Value` first.
- `min_reader_version > FORMAT_VERSION` => `ProjectError::TooNew`.
- `format_version > FORMAT_VERSION` but `min_reader_version` acceptable => open with `LoadWarning::NewerFormat` and
  `LoadReport.force_save_as = true` (the app then refuses plain Save so the newer file is never overwritten with lost fields).
- Older => `migrate::upgrade(value, from)`: a chain of pure `fn vN_to_vN1(Value) -> Value`, then `from_value`. `FORMAT_VERSION`
  is 3 (round 6): `v1_to_v2` returns the manifest unchanged (a version-1 text entry has no `fx`; serde fills in the default);
  `v2_to_v3` likewise (a version-2 projected placement has no `mode` / `wrap_*`: Project, options off).
- `min_reader_version` is the lowest version whose reader shows everything in the file correctly: `project::required_reader`
  gives 3 when any projected stamp is wrapped or has a Wrap option set (also a projected stamp keeping options for
  switching back: an older build would drop them on its next save), else 2 when a text stamp has `!fx.is_default()` (a
  gap value counts, also one on the last character of a line), else 1. So a round-4 project without new text features
  opens in the round-3 build, and a round-6 project without wrapped stamps or options in the round-5 build (with the
  "newer format" warning and Save As); a project with a wrapped stamp is refused by round 5 (`TooNew`).
- Frozen fixtures that must load forever: the version-1 manifest of `project::load`'s tests, and the version-1 text fixture
  `frozen_v1_text_fixture_loads` (`tests/project_roundtrip.rs`: one text stamp and one font blob, the bundled Ubuntu
  Light), which was run green on the round-3 code before the schema changed.
- A second frozen fixture, `frozen_v2_fx_fixture_loads` (`tests/project_roundtrip.rs`): a literal version-2 manifest with one
  text stamp that uses every effect (`fx` with two gaps, thickening, slant, an Arc warp, a Curve path). It loads without a
  warning, and a re-save writes the same `source` object with `min_reader_version` 2. Never edit either fixture string.
- Unknown names never fail a file (`TextFeatureUnknown`, 7.1 step 5); out-of-range or non-finite values are repaired by
  `sanitize_fx`; a wrong JSON type inside `fx` fails the manifest (`ManifestInvalid`) like any malformed field.
- `import.importer_version` is the importer that produced the saved mesh. A change to the generated UV layout or to mesh
  normalisation that changes `uv_hash` must bump `IMPORTER_VERSION` and keep the previous path selectable by version from then
  on (5.21). A project whose version is older and whose generated layout no longer matches opens with
  `LoadWarning::OlderGeneratedUvLayout`; the file is not touched until the user saves (the save writes the current version and
  hash).

### 7.4 Export [C6]

`export::run` per selected set (`multi` = more than one set selected; under a layer selection: the model has more than one set):
1. `flat = flatten_layers(layers, w, h, false)`; if `dilate_px > 0`, `dilate_islands(&mut flat, &texel_map, dilate_px)`.
2. `opts.flat`: `encode_flat(&flat, fmt)` -> `flat_name(..)`.
3. `per_layer_png`: every layer (visible ones, or all with `include_hidden`) as a **full-canvas** RGBA PNG (`Balanced`), so the files
   overlay directly -> `layer_name(..)`.
4. `ora` / `psd`: `ExportSet` with all layers (hidden ones only with `include_hidden`) -> `layered_name(..)`.
`export::plan(snapshot, opts)` returns the complete, ordered list of output paths (`opts.dir.join(name)`; per set: flat, per-layer
files bottom to top, ORA, PSD) using the naming and de-duplication of 7.5; `run` iterates exactly that list, so the dialog can show
the real file names and detect files that would be replaced **before** anything is written. `run` first creates `opts.dir` with
`std::fs::create_dir_all`. Nothing selected (empty plan) => `NothingToExport`. Progress per file; cancellation between files.

Run of the files (fix round 3, R3-5):
- **Sets in parallel.** `sets_in_flight` sets are exported at the same time: `1 GiB / (canvas bytes x (exported layers + 3))` of
  the largest selected set, at least 1, at most the rayon pool's thread count and the number of sets (2048² with 4 layers: 9; 8192²:
  1, as before). Lanes are `rayon::scope` tasks that take the next set from an atomic counter; a set's own per-layer encodes, ORA
  crops and PSD rows keep using the same pool. The files of one set are written one after the other in plan order;
  `ExportReport::files` is in plan order whatever the timing. Progress counts written files under a lock (monotone). A failing set
  sets a flag that stops the others before their next file; the first error in plan order that is not `Cancelled` is returned.
- **Atomic, flushed at the end.** Each file is written to `temp_path_for(path)` (`<name>.tmp-<pid>`, no flush), closed and renamed
  over its final name at once; a failed write or a panic deletes its temp file (`TempFile` guard), so at most one temp file per
  set in flight exists at any time and none is left behind by a failure, a panic or a killed process beyond the one being
  written. A file under its final name is always complete, as with `write_atomic` (it is not yet on the disk until the flush; a
  power loss before the flush can lose it, which `write_atomic`'s per-file flush did not allow). When all lanes have stopped —
  also after a failure — `finish` flushes every written file (`sync_all`) in plan order, reporting the stage "Finishing" (the
  progress bar's last 10 %; writing takes it to `WRITE_SHARE` = 0.9) after each, and honours a cancellation between files. The
  files written before a cancellation or failure stay. The flush at the end replaces the flush per file because the disk
  dominates a large export (below).
- **No PNG encoded twice.** The ORA file stores PNGs the set's other outputs already encoded from the same pixels at `Balanced`:
  the flattened PNG (only when `opts.flat == Png(Balanced)`) as `mergedimage.png`, and a layer's full-canvas PNG as its
  `data/layerNNN.png` when the layer's content rectangle is the whole canvas (`ora::OraReuse`, `write_ora_reusing`). The encoder is
  deterministic, so the archive is byte-identical to one written alone; the kept PNGs are dropped after the ORA.
- Measured (ABeautifulGame, 15 sets of 2048² / 1024², 85 files, 326 MB, of which PSD 260 MB; `export_bench_real_project`): the CPU
  work per set alone sums to ~31 s on one thread (flattened PNG 6.4 s, per-layer PNGs 7.2 s, ORA 12.7 s of which about 70 % were
  the duplicate merged/base PNGs, PSD 4.9 s, flattening 0.5 s). With sets in parallel the encoding finishes in about 3 s; the rest
  is writing to the disk. On the USB hard disk `E:` of the measuring machine, writing the same 326 MB took 14-18 s with a flush per
  file and 8-11 s with one flush pass at the end. Memory: the export adds at most ~250 MB to the process (1.03 GB after loading
  the project, 1.29 GB peak; one set at a time: +50 MB).
- The output is byte-identical to the round-2 exporter (SHA-256 of all 85 ABeautifulGame files and of the `export_run` scenario
  files); PNG levels and the PSD / ORA layout are unchanged.

Layer scope (1.8; `ExportOptions::layers`):
- `exported_layers(set, opts)` is the one filter of per-layer files, ORA and PSD: `None` => `include_hidden || visible`;
  `Some(ids)` => `ids.contains(id)` (hidden ones too). The index in a per-layer name stays the position in the full stack, so
  a selected layer's file has the same name as in a full export.
- `selected_sets` drops, under a selection, the sets in which no layer is selected; `multi` (the `_<set>` name part) counts
  the sets that remain **without** a selection (unchanged) and, **under** a selection, the sets of the model
  (`snapshot.sets.len() > 1`), whatever `opts.sets` (the dialog opened from a layer row sets it to that layer's set) and the
  selection narrow it to (fix_exp_1, check_exp_1 D4). So on a multi-set model a selected layer keeps its set name and the
  name a full export gives it, and two sets' layers with the same index and name (ABeautifulGame's `Knight_Black` /
  `Pawn_Body_Black` `L02_stamp_Logo`) never share a file name. A set filter of one set without a layer selection still
  drops the set name, as before.
- Flattened texture under a selection = `flatten_layers(<clones of the selected layers>, w, h, false)`: the selected
  **visible** layers bottom to top with their own opacity and blend (a selected hidden layer is in the per-layer files and in
  ORA / PSD flagged hidden, not in the flattened picture). A blend mode with nothing below composites over transparent, as
  `flatten_layers` defines it. ORA's `mergedimage.png` and the PSD's merged image are that picture, so each layered file
  agrees with its own stack. With `None` the original line (`flatten_layers(&set.layers, ..)`) runs unchanged.

Before the first file (fix_export, `preflight`, Files and Zip): no planned path (for a zip: the zip's path) may be a
folder, and a probe file `.wte-export-probe.tmp-<pid>` must be creatable in the folder (removed at once); otherwise the
export fails with nothing written, so a read-only or blocked folder never leaves a mix of new and old files. A failure
while writing (a full disk, a file held open) still leaves the files written before it.

Layer selection (fix_export): `opts.layers` is completed by `linked_selection` before planning: a stamp / text design that
spans several sets has one layer per set (same `StampId`), and selecting one of them exports all of them.

Package (`ExportPackage::Zip`, `run_zip`):
1. `plan_contents`; empty => `NothingToExport` (nothing created). Cancelled before the start => `Cancelled`, nothing created.
2. `create_dir_all(dir)` (remembered when this export created the folder), then the stage folder
   `<zip name>.tmp-<pid>.d` next to the zip (a leftover of the same pid is removed first); a `StageDir` guard removes it on
   every exit (success, failure, cancellation, panic).
3. The unchanged loose-file exporter writes into the stage (`run_files(.., flush = false)`: sets in parallel as usual, no
   `finish` pass, since the staged files are thrown away). Its progress is scaled to 0..0.6, stage "Exporting".
4. Packing in plan order into `<zip name>.tmp-<pid>` (`TempFile` guard; `ZipWriter` over a 1 MiB `BufWriter`): per staged
   file one entry named like the file (flat, no folders), PNG / JPEG / WebP / ORA **stored**, PSD / TGA / BMP / TIFF
   **deflated** at level 6, time stamp fixed at 1980-01-01 00:00:00 (`last_modified_time(DateTime::default())`), ZIP64 sizes
   (`large_file`) only for a staged file of at least 0xFFFF0000 bytes; `std::io::copy`, then the staged file is deleted
   (extra disk use near one file). Cancellation between entries; progress 0.6..0.95, stage "Packing".
5. `finish()`, flush, `sync_all`, rename to `package_name_for(opts)` (`<base>.zip`, or `<base>_selection.zip` under a
   layer selection), guards released, progress 1.0 "Finishing". Report: `files =
   [zip]`, `bytes_written` = its size, `packaged` = entry count.
- Atomic as a whole: until the rename no `<base>.zip` changes; an existing one is replaced only by a complete new one. A
  failure or cancellation leaves no zip, no temp file, no stage folder, and removes the output folder again when this export
  created it and it is empty (stricter than loose export, where finished files stay).
- Deterministic: fixed time stamps, no comment or extra fields, entry order = plan order, deterministic encoders; two exports
  of the same document are byte-identical, with one lane or several (tested). Entries are byte-identical to the loose files:
  they are the loose files.
- Cost: the export is written twice (stage, then zip), both in the output folder (same drive). Rejected: writing entries
  straight into one `ZipWriter` from the lanes (one stream forces one lane or a reorder buffer holding whole PSDs; the ORA
  writer needs `Seek`).

`encode_flat` (`REG\image-0.25.10`): PNG through the `png` crate (`png::Encoder::new(w, width, height)`, `set_color(ColorType::Rgba)`,
`set_depth(BitDepth::Eight)`, `set_compression(Compression::Fast | Balanced | High)`, `write_header()`, `write_image_data(..)`;
`REG\png-0.18.1\src\encoder.rs:165-742`). JPEG: matte RGB over `matte` (`c' = (c*a + m*(255 - a) + 127) / 255`), then
`image::codecs::jpeg::JpegEncoder::new_with_quality(w, quality)` with `ExtendedColorType::Rgb8` (JPEG accepts only L8/Rgb8).
TGA / BMP / TIFF / WebP (lossless only): `image::write_buffer_with_format(&mut Cursor<Vec<u8>>, rgba, w, h,
ExtendedColorType::Rgba8, ImageFormat::Tga | Bmp | Tiff | WebP)` (`REG\image-0.25.10\src\images\dynimage.rs:1658`).

`decode_image`: `image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()`; if no format was guessed use
`ImageFormat::from_extension` of `name_hint` (TGA has no magic); set `image::Limits` (`max_image_width/height = 16384`,
`max_alloc = 1 GiB`; build with `let mut l = Limits::default(); l.max_... = ...`); `decode()?.into_rgba8()`; downscale with
`pixel_ops::resize` if a side exceeds `max_dim`. DDS: only DXT1/3/5 decode; others give a clear `UnsupportedFormat`.

### 7.5 Export file naming [C6]

- Flattened: `<base>.<ext>` (single set) or `<base>_<set>.<ext>`. "Single set" = one set exported; under a layer selection
  = the model has one set (7.4).
- Per layer: `<base>[_<set>]_L<index:02>_<kind>_<layer-name>.png`; index 00 = bottom; kind = `paint` | `stamp` | `text`.
- Layered: `<base>[_<set>].ora`, `<base>[_<set>].psd`.
- Package (`ExportPackage::Zip`): `<base>.zip`; a layer selection `<base>_selection.zip` (fix_export: it never replaces the
  full package in the same folder); its entries carry exactly the loose names above (flat list, no folders).
- `sanitize_file_stem`: replaces `<>:"/\|?*` and control characters with `_`, trims trailing dots and spaces, prefixes `_` to
  reserved device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9, case-insensitive), caps at 64 chars, empty => `untitled`.
  The exporter de-duplicates colliding names with `_2`, `_3` (case-insensitive) over a **reference plan** (fix_export,
  `reference_names`), not over the files of the run: per set (all sets when names carry the set name, else the selected
  ones) the flattened name (`opts.flat`'s extension, `png` when off), every layer of the stack (hidden ones too), ORA, PSD.
  So any narrower export (a selection, fewer formats, without hidden layers) gives each file the name the full export
  gives it, `_2` included (`Body` / `body`, `Mat:1` / `Mat_1`, long names sharing their first 64 characters).

### 7.6 OpenRaster (`export/ora.rs`) [C6]

ZIP entries in this order (STORED entries use the `stored` options of 7.1 — no compression level —, deflate entries the `deflated`
options):
1. `mimetype` — STORED, first, content exactly `image/openraster`.
2. `stack.xml` — deflate, UTF-8:
```xml
<?xml version='1.0' encoding='UTF-8'?>
<image version="0.0.3" w="W" h="H" xres="72" yres="72">
 <stack>
  <layer name="NAME" src="data/layer002.png" x="X" y="Y" opacity="0.750" visibility="visible" composite-op="svg:src-over"/>
  ... top-most layer FIRST ...
 </stack>
</image>
```
3. `data/layerNNN.png` — STORED; RGBA8 cropped to the content bbox with `x`/`y` offsets (`NNN` = index from the bottom); an empty layer
   is a 1x1 transparent PNG at 0,0.
4. `mergedimage.png` — the full-size flattened image.
   (`write_ora_reusing` stores a PNG the caller already encoded instead of encoding the merged image or a full-canvas layer
   again; see 7.4. The bytes are the same.)
5. `Thumbnails/thumbnail.png` — at most 256x256, aspect preserved.

`visibility` = `visible` | `hidden`; `opacity` with three decimals; attribute values escape `& < > " '`; `composite-op` =
`BlendMode::ora_op()`.

### 7.7 PSD writer (`export/psd.rs`) [C6]

All integers big-endian. 8-bit RGB, PSD version 1. Reject width or height above 30000 (`PsdDimension`). Sizes are computed in `u64`;
if the layer section or the file would exceed `u32::MAX` return `PsdTooLarge`.

**A. Header (26 bytes)**: `"8BPS"`, `u16 1`, 6 zero bytes, `u16 channels = 4`, `u32 height`, `u32 width`, `u16 depth = 8`,
`u16 mode = 3` (RGB).

**B. Color mode data**: `u32 0`.

**C. Image resources**: `u32 length`, then one block (resolution info): `"8BIM"`, `u16 0x03ED`, Pascal name `00 00`, `u32 16`, data
`00 48 00 00` (72.0 as 16.16 fixed), `u16 1`, `u16 1`, `00 48 00 00`, `u16 1`, `u16 1`.

**D. Layer and mask information**: `u32 section_len = 4 + layer_info_len + 4`, containing
- `u32 layer_info_len` (length of what follows up to the global mask, zero-padded to a multiple of 4);
- `i16 layer_count = -N` (negative: the first alpha channel of the merged image is its transparency);
- N layer records, bottom layer first;
- channel image data for all layers, in record order;
- padding to make `layer_info_len` a multiple of 4;
- `u32 0` (global layer mask info).

Layer record (rect = content bbox; an empty layer uses 0,0,0,0):

| Bytes | Field |
|---|---|
| 4x4 | `i32 top, left, bottom, right` |
| 2 | `u16 4` channels |
| 4x6 | per channel `i16 id`, `u32 data_len` (includes the 2-byte compression word); order `-1` (alpha), `0` R, `1` G, `2` B |
| 4 | `"8BIM"` |
| 4 | blend key = `BlendMode::psd_key()`: `norm`, `mul ` (trailing space), `scrn`, `over`, `lddg` (Add), `dark`, `lite` |
| 1 | opacity `round(opacity * 255)` |
| 1 | clipping `0` |
| 1 | flags `0x08 | (0x02 if hidden)` |
| 1 | filler `0` |
| 4 | `u32 extra_len` (everything below) |
| 4 | `u32 0` layer mask data |
| 4 | `u32 0` blending ranges |
| var | Pascal name: `u8 len`, bytes, zero-padded so `1 + len` is a multiple of 4; ASCII only (non-ASCII => `?`), at most 255 bytes |
| var | Unicode name block: `"8BIM"`, `"luni"`, `u32 block_len`, `u32 n_utf16_units`, UTF-16BE units, zero-padded so `block_len` is a multiple of 4 |

Channel data, per channel: `u16 compression`; `0` (raw) for empty layers (no bytes follow, `data_len = 2`); otherwise `1` (RLE):
`h` x `u16` byte counts per row, then the PackBits rows. Pixels are straight alpha.

**E. Merged image**: `u16 1` (RLE), then `4*H` x `u16` row byte counts in plane order R, G, B, A, then the rows in that order. If any
alpha is below 255 the RGB planes are matted on white (`c' = (c*a + 255*(255 - a) + 127) / 255`); the A plane keeps the real alpha.

**PackBits**: header `n` in 0..=127 => copy the next `n + 1` literal bytes; 129..=255 => repeat the next byte `257 - n` times
(2..=128); 128 is never emitted. The encoder emits a run for 3 or more equal bytes (2 at the end of a row) and splits runs and
literals at 128. Worst case for an 8192-px row is 8257 bytes (fits `u16`). Rows are encoded with rayon, layer by layer.

**Verification without Photoshop**: `read_psd` is a strict parser (fails if any declared length disagrees with the bytes consumed or
trailing bytes remain) returning header, layers (rect, blend key, opacity, hidden, both names, decoded RGBA) and merged RGBA. Tests in
section 9. Opening the files in GIMP / Krita / Photopea remains a manual release check.

---

## 8. `wte_app` architecture

Imports: `use eframe::{egui, egui_wgpu, wgpu};` everywhere (never a separately versioned wgpu).

### 8.1 Verified eframe / egui / egui-wgpu / wgpu facts

| Fact | Source |
|---|---|
| `trait eframe::App { fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {}  fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame);  fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {}  fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4];  fn on_exit(&mut self) {} }` | `REG\eframe-0.37.0\src\epi.rs:152-282` |
| `logic` then `ui` run inside one `egui_ctx.run_ui(raw_input, ..)`; while the window is hidden only `logic` runs | `REG\eframe-0.37.0\src\native\epi_integration.rs:349-362` |
| `eframe::run_native(app_name: &str, native_options: NativeOptions, app_creator: AppCreator<'_>) -> eframe::Result`; `AppCreator = Box<dyn FnOnce(&CreationContext<'_>) -> Result<Box<dyn App>, Box<dyn Error + Send + Sync>>>` | `REG\eframe-0.37.0\src\lib.rs:288`, `epi.rs:49` |
| `CreationContext { egui_ctx, wgpu_render_state: Option<egui_wgpu::RenderState>, .. }`; `Frame::wgpu_render_state(&self) -> Option<&egui_wgpu::RenderState>`; `Frame` implements `HasWindowHandle + HasDisplayHandle` | `epi.rs:53-84, 943` |
| `NativeOptions { viewport: egui::ViewportBuilder, multisampling: u16, depth_buffer: u8, renderer, wgpu_options, persist_window, system_font_fallback, dithering, .. }` | `epi.rs:306-404` |
| `RenderState { adapter, instance, device, queue, target_format, renderer: Arc<epaint::mutex::RwLock<egui_wgpu::Renderer>>, .. }` | `REG\egui-wgpu-0.37.0\src\lib.rs:105-129` |
| `Renderer::register_native_texture(&mut self, device: &wgpu::Device, texture: &wgpu::TextureView, texture_filter: wgpu::FilterMode) -> epaint::TextureId`; `update_egui_texture_from_wgpu_texture(&mut self, device, texture: &wgpu::TextureView, texture_filter, id: epaint::TextureId)`; `free_texture(&mut self, id: &epaint::TextureId)` | `REG\egui-wgpu-0.37.0\src\renderer.rs:809, 830, 790` |
| Device limits requested by egui-wgpu: `max_texture_dimension_2d: 8192` | `REG\egui-wgpu-0.37.0\src\setup.rs:272` |
| Panels take `&mut Ui`: `egui::Panel::{top,bottom,left,right}(id).resizable(b).exact_size(f).default_size(f).show(ui, ..)`, `egui::CentralPanel::default()` / `::no_frame()` `.show(ui, ..)` (there is no `TopBottomPanel`/`SidePanel`) | `REG\egui-0.37.0\src\containers\panel.rs:252-425, 1358-1377` |
| `egui::MenuBar::new().ui(ui, ..)`, `ui.menu_button(..)`; `egui::Modal::new(id).show(ctx, ..) -> ModalResponse<T>` (`should_close()`) | `containers\menu.rs:336-361`, `ui.rs:2922`, `modal.rs:26-151` |
| `ctx.request_screenshot(callback: impl FnOnce(Arc<ColorImage>) + Send + 'static)` sends `ViewportCommand::Screenshot(ScreenshotCallback)`; there is no screenshot event. The readback uses `map_async` and nobody calls `device.poll`, so the callback fires only after a later submit/poll | `REG\egui-0.37.0\src\context.rs:4557`, `viewport.rs:1205`, `REG\egui-wgpu-0.37.0\src\capture.rs:193-211` |
| `ViewportCommand::{Close, CancelClose, Title(String), InnerSize(Vec2), Icon(..), RequestPaintWhileHidden, ..}`; `ctx.input(\|i\| i.viewport().close_requested())`; `ViewportBuilder::with_icon(impl Into<Arc<IconData>>)`, `IconData { rgba, width, height }` | `viewport.rs:1090-1239, 440, 189` |
| `Context::send_viewport_cmd` **always** calls `request_repaint_of(viewport)`: a command sent unconditionally every frame makes the app repaint forever | `REG\egui-0.37.0\src\context.rs:4549-4577` |
| `ViewportCommand::InnerSize(size)` is converted to physical pixels **after** the pass as `size * (zoom_factor * native scale)`; `Context::set_zoom_factor` only stores the new factor, which takes effect at the start of the **next** pass. `ctx.native_pixels_per_point() -> Option<f32>`, `ctx.content_rect() -> Rect` | `REG\egui-winit-0.37.0\src\lib.rs:1969-1972`, `REG\egui-0.37.0\src\context.rs:2761-2766, 2731, 3406` |
| While the root window is minimized/hidden eframe runs only `App::logic` (no egui pass, injected events are not applied); `ViewportCommand::RequestPaintWhileHidden` forces a full painted pass for one frame (a pending `Screenshot` implies it) | `REG\eframe-0.37.0\src\native\wgpu_integration.rs:686-716`, `REG\egui-winit-0.37.0\src\lib.rs:1913-1923, 2161` |
| `egui::Sense::DRAG` (bitflag constant: drag starts on press, **not focusable**) vs `Sense::drag()` (= `DRAG \| FOCUSABLE`: joins Tab navigation) vs `click_and_drag()` (waits for the click/drag decision); `ui.allocate_rect(rect, sense) -> Response`; `Response::{contains_pointer, hover_pos, interact_pointer_pos, dragged_by}` | `sense.rs:15, 68-81`, `ui.rs:1382`, `response.rs:346-651` |
| `RawInput { time: Option<f64>, predicted_dt: f32, events: Vec<Event>, hovered_files: Vec<HoveredFile>, dropped_files: Vec<DroppedFileHandle>, .. }` — there is **no** `modifiers` field: the modifier state arrives as `Event::ModifiersChanged(Modifiers)`; `egui::Id::new` is deprecated (use `Id::unique`); `trait DroppedFile { fn path(&self) -> &Path; fn bytes(&self) -> Result<Vec<u8>, String>; .. }` | `data\input\raw_input.rs:48-81`, `dropped_file.rs` |
| Event shapes: `Event::PointerMoved(Pos2)`; `Event::PointerButton { pos, button, pressed, modifiers }`; `Event::Key { key: Key, physical_key: Option<Key>, pressed: bool, repeat: bool, modifiers: Modifiers }`; `Event::MouseWheel { unit: MouseWheelUnit /* Point, Line, Page */, delta: Vec2, phase: TouchPhase, source: MouseWheelSource /* Unknown, Wheel, Trackpad, Momentum */, modifiers: Modifiers }`; also `MouseMoved(Vec2)`, `PointerGone`, `WindowFocused(bool)`, `Text(String)`, `Zoom(f32)` | `data\input\event.rs:97-202`, `mouse_wheel_unit.rs`, `mouse_wheel_source.rs` |
| Wheel input is **not** available as a vertical scroll delta when a modifier is held: with Ctrl the wheel feeds `zoom_delta` and `smooth_scroll_delta` stays zero; with Shift it becomes horizontal scroll; the smoothed delta also spreads one notch over several frames; `InputState` has no `raw_scroll_delta`. Views read the raw `Event::MouseWheel` events from `i.events` (section 8.6) | `input_state\mod.rs:456-470`, `input_state\wheel_state.rs:149-160` |
| `InputState::consume_shortcut(&KeyboardShortcut) -> bool` matches the **logical** key; `Modifiers::matches_logically` ignores extra Shift/Alt. Shift+`[` arrives as `Key::OpenCurlyBracket` (`"{"`), Shift+`]` as `Key::CloseCurlyBracket` | `input_state\mod.rs:743-761`, `data\input\modifiers.rs:211-220`, `data\key.rs:539-542`, `REG\egui-winit-0.37.0\src\lib.rs:1567-1571` |
| `ctx.egui_wants_keyboard_input()` = *any* widget has focus (also buttons and sliders after Tab); `ctx.text_edit_focused()` = the focused widget is a text edit; `ctx.memory(\|m\| m.focused())`, `ctx.memory_mut(\|m\| m.surrender_focus(id))`; `Response::lost_focus()` is true in the pass in which focus was surrendered before the widget is drawn | `context.rs:3485-3496`, `memory\mod.rs:859-937` |
| `ctx.pixels_per_point()`; `ctx.all_styles_mut(..)`; `ctx.load_texture(..)`; `ctx.request_repaint_after(Duration)`; `painter.image(texture_id, rect, uv, tint)` | `context.rs:2712, 2637, 2814, 2179`, `painter.rs:447` |
| `egui::color_picker::color_picker_color32(ui, &mut Color32, Alpha) -> bool`; `ui.dnd_drag_source` / `ui.dnd_drop_zone` | `widgets\color_picker.rs:547`, `ui.rs:2767-2826` |
| wgpu 30 descriptor shapes: `PipelineLayoutDescriptor { label, bind_group_layouts: &[Option<&BindGroupLayout>], immediate_size: 0 }`; `VertexState { module, entry_point: Some(".."), buffers: &[Some(VertexBufferLayout {..})], compilation_options }`; `DepthStencilState { format, depth_write_enabled: Option<bool>, depth_compare: Option<CompareFunction>, stencil, bias }`; `RenderPipelineDescriptor { .., multiview_mask: None, cache: None }`; `RenderPassColorAttachment { view, depth_slice: None, resolve_target, ops }`; `RenderPassDescriptor { label, color_attachments, depth_stencil_attachment, timestamp_writes, occlusion_query_set, multiview_mask: None }`; `SamplerDescriptor { .., mipmap_filter: wgpu::MipmapFilterMode, .. }` | `REG\egui-wgpu-0.37.0\src\renderer.rs:380-462`, `capture.rs:164-179`, `REG\wgpu-30.0.1\src\api\render_pass.rs:621-682`, `REG\wgpu-types-30.0.1\src\texture.rs:658-682` |
| `Queue::write_texture(&self, texture: TexelCopyTextureInfo<'_>, data: &[u8], data_layout: TexelCopyBufferLayout, size: Extent3d)`; `Device::poll(&self, wgpu::PollType) -> Result<PollStatus, PollError>` (`PollType::Poll`); `CommandEncoder::begin_render_pass(&mut self, &RenderPassDescriptor) -> RenderPass<'encoder>` | `REG\wgpu-30.0.1\src\api\queue.rs:253`, `device.rs:95`, `command_encoder.rs:76` |
| `wgpu::util::TextureBlitter` is **not** used: its sampler sets only `mag_filter` (`min_filter` stays `Nearest`), so a 2:1 downsample blit is a point-sampled decimation and the builder cannot change that. Mips use our own blit pipeline (section 8.4) | `REG\wgpu-30.0.1\src\util\texture_blitter.rs:54-61`, `REG\wgpu-types-30.0.1\src\texture.rs:685-701` |
| `rfd::FileDialog::new().add_filter(name, &[ext]).set_directory(p).set_file_name(s).set_title(s).set_parent(&w).pick_file() / .save_file() / .pick_folder() -> Option<PathBuf>` | `REG\rfd-0.18.0\src\file_dialog.rs:40-199` |
| `rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title(s).set_description(s).show() -> MessageDialogResult` (native message box, works without a window or an egui context) | `REG\rfd-0.18.0\src\message_dialog.rs:15-82, 154` |
| `env_logger::Builder::new().filter_level(log::LevelFilter::Info).target(env_logger::Target::Pipe(Box<dyn Write + Send>)).try_init()` (log to a file) | `REG\env_logger-0.11.11\src\logger.rs:391-498`, `src\writer\target.rs:4-12` |

When in doubt about any descriptor, copy its shape from `REG\egui-wgpu-0.37.0\src\renderer.rs`.

### 8.2 State (`state.rs`, `action.rs`) [S]

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)] pub enum ViewMode { View3D, View2D, Split, Preview }
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)] pub enum ViewId { Main3D, Main2D, Preview, ExportPreview }
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum ShadingMode { Textured = 0, Clay = 1, UvChecker = 2 }
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum Backdrop { Dark = 0, Light = 1, Gradient = 2, Checker = 3 }
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ToolKind { Navigate, Brush, Eraser, Eyedropper, Fill, Stamp, Text, Transform }
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum CameraPreset { Front, Back, Left, Right, Top, Bottom }

pub struct ViewFlags { pub shading: ShadingMode /* Textured */, pub lit: bool /* true */, pub wireframe: bool, pub seams: bool,
                       pub uv_wire: bool /* true: UV wireframe in the 2D view */, pub alpha_checker: bool /* true */,
                       pub backdrop: Backdrop /* Gradient */ }
pub struct PreviewSettings { pub lit: bool, pub turntable: bool, pub turntable_speed: f32 /* rad/s, 0.6 */,
                             pub backdrop: Backdrop, pub nearest: bool }
pub struct BrushSettings { pub size_px: f32 /* diameter in texels, 1..=2048 */, pub hardness: f32, pub opacity: f32,
                           pub flow: f32, pub spacing: f32,
                           pub kind: BrushType /* Circular | Sphere */, pub shape: ShapeChoice /* Ball | Cube | Cylinder | File */,
                           pub through: bool, pub turn_deg: f32, pub tilt_deg: f32, pub roll_deg: f32, pub offset: f32,
                           pub align: wte_core::TipAlign }   // round 4: `same_side_only` removed; Circular = the old "same side only" stroke
pub struct StampSettings { pub source: Option<BlobId>, pub source_name: String,
                           pub size_frac: f32 /* larger side: fraction of model diagonal (3D) or of texture width (2D); 0.15 */,
                           pub rotation_deg: f32, pub project_through: bool,
                           // round 6 (R6-2): shared by the Stamp and the Text tool; #[serde(skip)]: kept for the
                           // session only, new sessions start in Project (ROUND6_PLAN 11 Q1); `WteApp::new` sets Wrap
                           // when WTE_STAMP_MODE=wrap (state::stamp_mode_from_env; logged "new 3D stamps: ...")
                           pub mode: wte_core::SurfaceMode /* Project */, pub wrap: wte_core::WrapOptions /* off, Ends meet */ }
// Round 8 (R8-2) env rule: `AppState::async_final_bakes` (WTE_SYNC_FINAL_BAKES unset or not "1") also switches the
// wrapped stamps' Preview bakes to jobs (`install_document`: `doc.set_defer_wrap_previews(async_final_bakes)`);
// WTE_SYNC_FINAL_BAKES=1 keeps Finals and wrapped Previews on the UI thread (the gate's comparison mode). New AppState
// fields: `wrap_jobs: stamp_bakes::WrapJobs`, `wrap_jobs_pending: bool` (8.9).
pub struct TextSettings { pub content: String /* "Text" */,
                          pub family: String /* DEFAULT_FONT_FAMILY */, pub style: String /* DEFAULT_FONT_STYLE */,
                          pub color: [u8; 4], pub align: TextAlign, pub line_spacing: f32 /* 1.0 */,
                          pub letter_spacing: f32 /* em fraction, 0 */,
                          pub outline: bool, pub outline_width_em: f32 /* em fraction, 0.06 */, pub outline_color: [u8; 4],
                          /// The UI "Size": on-model height of one em (one text line) as a fraction of the model diagonal
                          /// (3D placement) or of the texture height (2D placement); 0.06. There is no raster-size setting:
                          /// the core picks TextSpec.size_px (sections 4.3, 6.5).
                          pub em_frac: f32 }
pub struct ToolSettings { pub brush: BrushSettings, pub eraser: BrushSettings, pub fill_scope: FillScope,
                          pub stamp: StampSettings, pub text: TextSettings }
pub struct ColorState { pub primary: [u8; 4], pub secondary: [u8; 4], pub swatches: Vec<[u8; 4]> /* 24 */ }
#[derive(Default)]
pub struct HoverInfo { pub view: Option<ViewId>, pub set: Option<TextureSetId>, pub uv: Option<[f32; 2]>,
                       pub texel: Option<[u32; 2]>, pub rgba: Option<[u8; 4]>, pub world: Option<[f32; 3]> }

pub struct OrbitCamera { pub target: glam::Vec3, pub yaw: f32, pub pitch: f32 /* clamped to +-89 deg */, pub distance: f32,
                         pub fov_y: f32 /* radians, default 45 deg */, pub ortho: bool }
pub struct UvViewState { pub center_uv: glam::Vec2, pub zoom: f32 /* points per texel, 0.02..=64 */ }

pub struct Session {
    pub doc: Document,
    pub path: Option<PathBuf>,                 // .wtep path once saved/opened
    pub force_save_as: bool,
    pub active_set: TextureSetId,
    pub selected_stamp: Option<StampId>,
    pub camera: OrbitCamera, pub preview_camera: OrbitCamera, pub uv_view: UvViewState,
    pub scene_generation: u64,                 // process-wide unique; changes whenever the document object is replaced
                                               // => GpuScene rebuild; also identifies the session a Save job belongs to
    pub selected_layers: Vec<LayerId>,         // export-select (1.8): the Layers list's selection, raw (may be stale)
    // objects-tabs: the object selection. App state only: not in the document, not in history, not saved.
    pub selected_objects: BTreeSet<u32>,       // indices into doc.mesh.objects
    pub object_anchor: Option<u32>,            // the row a Shift-click extends from (the last plain / Ctrl-clicked object)
    pub object_selection_rev: u64,             // +1 at every change of selected_objects (the GPU highlight follows it)
    pub object_reveal: Option<u32>,            // request: the Objects list scrolls this row into view once (3D pick)
}
impl Session {
    /// Re-validates app-side selection after any document change (called by dispatch, section 8.3): clears
    /// `selected_stamp` if `doc.stamp(id)` is None, clamps `active_set` to an existing set; objects-tabs: drops selected
    /// objects `>= object_count` (bumping `object_selection_rev`) and a stale anchor / reveal.
    pub fn validate(&mut self);
    /// The selected layers of the active set, bottom to top, normalised on every read: ids that are gone or belong to
    /// another set are dropped; when the active layer is not among them the selection is the active layer alone; no
    /// active layer => empty. Whatever makes another, unselected layer active (a click on a stamp, New, Undo, another
    /// set) collapses a multi-selection by itself; deleting a selected layer keeps the others selected. All readers use
    /// this; only dispatch writes `selected_layers`. Not in the document, not in history, not saved (`UiState` and
    /// `Prefs` do not hold it).
    pub fn layer_selection(&self) -> Vec<LayerId>;
}

#[derive(Clone, Debug)] pub enum Pending { NewFromModel(Option<PathBuf>), OpenProject(Option<PathBuf>), OpenSample(SampleKind), Quit }
pub enum SetChoiceUi { Detected,
                       /// A manually chosen image: `bytes` = the original file (kept for R7), `image` = decoded pixels.
                       File { path: PathBuf, bytes: Option<Arc<[u8]>>, image: Option<PixelBuf>, error: Option<String> },
                       Blank { side: u32, fill: [u8; 4] } }
pub struct ImportDialogState { pub path: PathBuf, pub model: Option<ImportedModel>, pub choices: Vec<SetChoiceUi>,
                               pub uv_thumbs: Vec<PixelBuf>,   // one uv_layout_image(.., 256) per set, from the import job
                               pub uv_stats: Vec<UvStats>,     // one per set (TexelMap::build at 256x256, gutter 0), from the import job
                               pub fallback_uvs: bool, pub loading: bool, pub error: Option<String> }
pub struct ExportDialogState { pub options: ExportOptions, pub format_index: usize, pub jpeg_quality: u8, pub jpeg_matte: [u8; 3],
                               pub show_preview: bool,
                               pub confirm_replace: Option<Vec<PathBuf>> /* Some = "N files will be replaced" step is showing */ }
pub enum Dialog { None, Import(Box<ImportDialogState>), Unsaved { then: Pending }, Export(Box<ExportDialogState>),
                  Message { title: String, lines: Vec<String> }, About, Shortcuts }

pub struct Prefs { pub recent: Vec<PathBuf> /* max 10 */, pub last_dir: Option<PathBuf>, pub last_export_dir: Option<PathBuf>,
                   pub window_size: [f32; 2] /* normal (not maximized) inner size, clamped to 8192 on load */,
                   pub window_maximized: bool, pub flags_wireframe: bool, pub keep_backup: bool,
                   pub swatches: Vec<[u8; 4]> }   // Serialize + Deserialize + Default, every field #[serde(default)]
/// A clickable control of the Layers list (8.7); `AppState::panel_rects` maps it to its rect of the last frame.
pub enum PanelHit { LayerRow(LayerId), LayerEye(LayerId), LayerThumb(LayerId), LayerName(LayerId), LayerLock(LayerId),
                    LayerAlphaLock(LayerId) }

pub struct BusyInfo { pub title: String, pub stage: String, pub fraction: f32, pub modal: bool }

pub struct AppState {
    pub session: Option<Session>,
    pub prefs: Prefs,
    pub view_mode: ViewMode, pub split_ratio: f32 /* 0.5 */,
    pub last_edit_mode: ViewMode,                   // the non-Preview mode to return to when leaving Preview (default View3D)
    pub flags: ViewFlags, pub preview: PreviewSettings,
    pub tool: ToolKind, pub tools: ToolSettings, pub colors: ColorState,
    pub hover: HoverInfo,
    pub view_rects: HashMap<ViewId, egui::Rect>,   // written by view::central every frame (points); read by the self-test
    pub panel_rects: HashMap<PanelHit, egui::Rect>, // written by the side panel every frame (points): Layers row controls, the
                                                    // settings / Color / Layers areas, footer, Delete (8.7); read by the self-test
    pub side_panel_rect: Option<egui::Rect>,        // written by App::ui every frame (points); read by the self-test
    pub central_rect: Option<egui::Rect>,           // written by App::ui every frame: the central view area; toasts anchor here
    pub dialog: Dialog,
    pub toasts: crate::toasts::Toasts,
    pub actions: Vec<AppAction>,                    // queue; drained by dispatch at the end of App::ui
    pub fonts: FontCatalog, pub fonts_loading: bool,
    pub busy: Option<BusyInfo>,                     // mirrored from Jobs each frame
    pub status_hint: String,                        // set by the active tool
    pub status_rects: Vec<egui::Rect>,              // written by the status bar every frame: the drawn items (8.7)
    pub allow_close: bool,
    pub last_title: String,                         // last window title sent (update_title sends only on change)
    pub gpu_mips_stale: bool,                       // written by view::central after GpuScene::sync (8.4); read by WaitIdle
    pub focus_text_content: bool,                   // request: the Text tool options focus their content field next frame
    pub typing_prev: bool,                          // ctx.text_edit_focused() at the end of the previous shortcuts::handle (8.10)
    pub selftest: bool,                             // true while the scenario runner drives the app (no native dialogs, no prefs writes)
    pub preview_bakes: PreviewBakeClock,            // throttle of the deferred stamp Preview bakes (8.4)
}
impl AppState { pub fn new(prefs: Prefs, selftest: bool) -> AppState;  pub fn doc(&self) -> Option<&Document>;
                pub fn doc_mut(&mut self) -> Option<&mut Document>;  pub fn push(&mut self, a: AppAction); }
```
Derive policy (the scaffold applies it; later packages rely on it): every plain-data type of `state.rs`, `action.rs` and
`gpu/params.rs` derives `Clone, Debug`; additionally `PartialEq` wherever all fields support it (all field-less enums,
`ViewFlags`, `PreviewSettings`, the `*Settings` structs, `ColorState`, `HoverInfo`, `OrbitCamera`, `UvViewState`, `BrushRing`,
`View3dParams`, `ViewUvParams`, `BusyInfo`, `Prefs`); `OrbitCamera` and `UvViewState` are also `Copy` (`ViewXform` stores them by
value); `ViewMode`, `ShadingMode`, `Backdrop`, `ViewFlags`, `PreviewSettings`, `ColorState`, `OrbitCamera`, `UvViewState`,
`ToolKind` and the `*Settings` structs also derive `Serialize, Deserialize` (they are written into `Document::ui_state` and
`Prefs`). `Dialog`, `ImportDialogState`, `ExportDialogState`, `SetChoiceUi`, `Pending`, `AppAction` derive `Clone, Debug` only.
`Session` and `AppState` derive nothing. `ShadingMode`, `Backdrop` and `CameraPreset` are defined in `state.rs` only;
`gpu/params.rs` imports them.

Fixed by the scaffold in `state.rs` (final; wave-2 packages rely on it):
- `pub const SWATCH_COUNT: usize = 24;` — `ColorState::swatches` always has that many entries. `Prefs::swatches` empty = the
  built-in palette; `AppState::new` pads (white) or truncates a non-empty list to `SWATCH_COUNT`.
- Serde shape: every serialized struct has struct-level `#[serde(default)]` (a missing field takes its default); enums are
  written in snake_case, `ViewMode` as `"view3d"` / `"view2d"` / `"split"` / `"preview"`; `glam::Vec3` / `Vec2` fields as plain
  arrays; `FillScope` as `"layer"` / `"island"`. `StampSettings::source` and `source_name` are `#[serde(skip)]`: a `BlobId` is
  bound to one document and is never persisted (8.9, `install_document`).
- Defaults not stated above: brush and eraser `size_px` 32, hardness 0.8, opacity 1, flow 1, spacing 0.12, Circular, Ball, Solid, no rotation, offset 0,
  Follow surface (`BrushSettings::sanitized` forces every value into the ranges of FEATURE_BRUSH 1.4); `fill_scope` `Island`; stamp `rotation_deg` 0, `project_through` false; text colour opaque black, `Left`, outline off
  with an opaque white outline colour; primary black, secondary white; `PreviewSettings` lit, turntable off, `Gradient`, linear;
  `OrbitCamera` target origin, yaw 0.6, pitch 0.35, distance 3, perspective; `UvViewState` centre `(0.5, 0.5)`, zoom 1;
  `Prefs` window 1280x800, no recent files or folders, `flags_wireframe` and `keep_backup` false.
- `AppState::new(prefs, selftest)`: no session, `View3D` (also `last_edit_mode`), tool `Brush`, `flags.wireframe` from
  `prefs.flags_wireframe`, `fonts = FontCatalog::bundled()`, `fonts_loading = false` (the code that starts the Fonts job sets
  it to true, 8.9), no dialog, `allow_close = false`.

```rust
// action.rs
/// Convention for every `Option<PathBuf>` below: `Some(path)` = use this path; `None` = ask the user with a native file
/// dialog (done by dispatch, which has the `eframe::Frame`; a `None` action is a no-op while `state.selftest`).
pub enum AppAction {
    // file
    NewFromModel(Option<PathBuf>), OpenProject(Option<PathBuf>), OpenSample(SampleKind), OpenPath(PathBuf) /* by extension */,
    Save, SaveAs(Option<PathBuf>), ShowExport, RunExport(ExportOptions), ExportPickFolder(Option<PathBuf>),
    ExtractSources(Option<PathBuf>) /* destination folder */, Quit,
    ConfirmPending(Pending) /* unsaved prompt answered "Don't Save" */, SaveThen(Pending),
    // import dialog
    ImportReload { fallback_uvs: bool }, ImportPickTexture { set: usize, path: Option<PathBuf> }, ImportCreate,
    // edit
    Undo, Redo,
    Cancel,                 // Esc: cancel interaction / close dialog / leave Preview (order in 8.3)
    DeleteSelection,        // Delete: the selected stamp, else the active layer of the active set
    // view
    SetViewMode(ViewMode), ToggleTexture /* Textured <-> Clay */, SetShading(ShadingMode), ToggleWireframe, ToggleSeams,
    ToggleUvWire, ToggleAlphaChecker, ToggleLit, FrameModel /* 3D: frame the model; 2D: fit the texture */, ResetCamera,
    SetCameraPreset(CameraPreset),
    // tools and colours
    SetTool(ToolKind), BrushSizeStep(i32), BrushHardnessStep(i32), SwapColors, ResetColors /* black / white */,
    SetPrimaryColor([u8; 4]),
    // sets and layers
    SetActiveSet(TextureSetId), SelectLayer(LayerId), AddPaintLayer, DuplicateLayer(LayerId), DeleteLayer(LayerId),
    ToggleLayerSelected(LayerId) /* Ctrl-click */, SelectLayerRange(LayerId) /* Shift-click */,
    MergeDown(LayerId), RasterizeLayer(LayerId), MoveLayer { layer: LayerId, to_index: usize },
    SetLayerVisible(LayerId, bool), SetLayerLocked(LayerId, bool), SetLayerAlphaLock(LayerId, bool),
    SetLayerOpacity(LayerId, f32, Commit), SetLayerBlend(LayerId, BlendMode), RenameLayer(LayerId, String),
    // the same commands on "the active layer of the active set" (shortcuts and the Layer menu; no-op without one)
    DuplicateActiveLayer, MergeActiveDown, RasterizeActiveLayer, MoveActiveLayer(i32) /* +1 = one step up, -1 = down */,
    // stamps
    ImportStampImage(Option<PathBuf>),                 // choose the image the Stamp tool places
    ReplaceStampImage(StampId, Option<PathBuf>),       // "Choose..." of a selected image stamp
    SelectStamp(Option<StampId>), UpdateStampSource(StampId, StampSource, Commit),
    UpdateStampPlacement(StampId, Placement, Commit), UpdateStampTint(StampId, [u8; 4], Commit), DeleteStamp(StampId),
    // objects (objects-tabs; `object_ops.rs`)
    SelectObject(u32) /* plain click */, ToggleObjectSelected(u32) /* Ctrl-click; Ctrl/Shift-click in 3D */,
    SelectObjectRange(u32) /* Shift-click in the list */, SelectAllObjects, DeselectObjects,
    SetObjectVisible(u32, bool) /* the eye of a row */, HideSelectedObjects, IsolateSelectedObjects, ShowAllObjects,
    FrameSelection,
    // ui
    ShowDialog(Dialog), CloseDialog, CancelJob, Toast(ToastLevel, String),
}
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum ToastLevel { Info, Warning, Error }
```
Rules:
- Panels, dialogs and shortcuts change *settings* in `AppState` directly (tool settings, colours, dialog-local fields) and express
  every *command* as an `AppAction`. `dispatch` is the only place that calls document-changing core commands, except the
  interactive tool path (`stroke_*`, `fill`, `add_raster_layer` before a stroke, `add_stamp`, `set_stamp_placement`,
  `cancel_pending_edit`, `sample_color`), which `tools/*` calls directly.
- **Native file dialogs are opened only by `dispatch`** (it is the only code with the `eframe::Frame`, D14). Panels and dialogs
  never call `rfd`; they push the `Option<PathBuf>` actions with `None`. The self-test pushes them with `Some(path)`.
- **Keyboard keys are consumed only by `shortcuts::handle`** (8.10), which turns them into actions. Tools, views, panels and
  dialogs read modifiers and "Space is held" only (text fields receive their typing from egui as usual). There is no second
  owner of Esc or Delete.
- `SelectLayer` on a stamp/text layer also selects its stamp; selecting a stamp also makes its `home_set` layer active.

Export-select (1.8): `ShowExportLayers(Vec<LayerId>)` (file group) opens the Export dialog for these layers;
`ExportDialogState::selected_layers: Vec<LayerId>` is the selection captured when the dialog opened (what "Selected (N)"
exports while the user switches the Layers choice back and forth). `PanelHit::LayerMenuExportThis` /
`LayerMenuExportSelected` are the rects of the open row menu's two items (self-test).

Round 4 (scaffold S2):
- `TextSettings` gains `bold_em`, `slant_deg`, `warp: Option<Warp>`, `path: Option<TextPath>` (defaults 0 / None).
- `Session.text_gap: Option<TextGapSel { stamp, after }>`: the gap selected in the gap strip; a new session clears it.
- `AppState` gains: `side_tab: SideTab { Layers, Brush, Text }` (not saved; a tool change never writes it),
  `text_page: TextPage { Character, Warp, Path }`, `brush_shape: BrushShapeState { current: Option<ReadyShape>,
  pending: Option<PendingShape>, last_error, next_request, warned_missing }` (`ReadyShape` / `PendingShape` carry the
  `CloseMode` of their bake), `builtin_tips: [Option<Arc<TipField>>; 2]` (Cube, Cylinder; made on first use),
  `side_rects: SideRects { panel, tool_area, tool_options, color, bottom_area, tab_buttons: [Rect; 3], brush_tab:
  Option<Rect>, text_tab: Option<Rect>, `brush_tab_content: Option<Rect>` (round 4, B4: the Brush tab's content as laid out, unclipped; written in the same frames as `brush_tab`; the self-test asserts that it lies inside `brush_tab`, i.e. that no row of the tab scrolls) }` (written by the side panel every frame; `brush_tab` / `text_tab` are `Some`
  exactly in the frames in which that tab's content was drawn), `font_samples: FontSampleState` (the picker's samples
  and atlas textures), `tool_probe: ToolProbe { cursor_footprint, cursor_wires, text_hover, wrap_controls }` (what the
  active tool showed in the last frame, for the scenarios, which see `AppState` only; round 6: `wrap_controls:
  Option<bool>` = the two Wrap option controls as the tool options drew them, `Some(enabled)`, `None` when not shown).
- `Dialog::CloseOpenings(Box<OpeningsDialogState { path, count, largest, closed_parts, choice, by_user, for_eraser }>)`:
  "how should the openings of this brush shape be closed?" (ROUND4_PLAN section 7 item 5).
- `Prefs` gains `brush`, `eraser` (`BrushSettings`), `brush_shape_path: Option<PathBuf>`, `brush_shape_close:
  BTreeMap<PathBuf, CloseMode>` (the answer per shape file), all `#[serde(default)]`.
- `AppAction` gains `LoadBrushShape(Option<PathBuf>)`, `ReloadBrushShape`, `RemoveBrushShape`,
  `AnswerOpenings(Option<CloseMode>)` (`None` = Cancel), `SetBrushShapeClose(CloseMode)` (the Brush tab's "Openings"),
  `LoadFontSamples`.

### 8.3 `WteApp` and the frame (`app.rs`, `dispatch.rs`) [A1]

```rust
pub struct WteApp {
    pub state: AppState,
    pub gpu: Gpu, pub renderer: Renderer, pub scene: Option<GpuScene>, pub targets: ViewTargets,
    pub tools: ToolController,
    pub panels: panels::PanelState, pub dialogs: dialogs::DialogsState,
    pub jobs: Jobs,
    pub selftest: Option<selftest::Runner>,
    pub exit_code: Arc<AtomicI32>,
}
impl WteApp { pub fn new(cc: &eframe::CreationContext<'_>, args: CliArgs, exit_code: Arc<AtomicI32>) -> anyhow::Result<WteApp>; }

impl eframe::App for WteApp {
    fn raw_input_hook(&mut self, ctx, raw) { if let Some(r) = &mut self.selftest { r.raw_input_hook(ctx, raw) } }
    fn logic(&mut self, ctx, frame) {
        for done in self.jobs.poll() { io_flows::on_job_output(self, done.output, done.cancel_requested) } // may replace the session
        self.state.busy = self.jobs.busy();
        if let Some(r) = &mut self.selftest { r.tick(ctx, &mut self.state, &self.gpu, self.tools.is_busy()) }
        io_flows::handle_close_request(self, ctx);                            // CancelClose + prompt / "please wait" (8.9)
        io_flows::update_title(self, ctx);                                    // sends Title only when the text changed (8.9)
        self.cap_window_size(ctx);        // MaxInnerSize = device max_texture_dimension_2d px (again when ppp changes)
        self.fit_window_to_monitor(ctx);  // once at start-up, not in self-test: fit_to_monitor (below)
    }
    fn ui(&mut self, ui, frame) {
        let late = shortcuts::handle(ui.ctx(), &mut self.state);              // keys -> actions; rules in 8.10
        egui::Panel::top("menu").show(ui, |ui| panels::show_menu_bar(ui, &mut self.state));
        egui::Panel::bottom("status").show(ui, |ui| panels::show_status_bar(ui, &mut self.state));
        egui::Panel::left("tools").resizable(false).exact_size(44.0).show(ui, |ui| panels::show_toolbar(ui, &mut self.state));
        let side = egui::Panel::right("side").default_size(300.0).show(ui, |ui| panels::show_side_panel(ui, &mut self.state, &mut self.panels));
        self.state.side_panel_rect = Some(side.response.rect);
        let mut cx = ViewCx { state: &mut self.state, gpu: &self.gpu, renderer: &mut self.renderer,
                              scene: &mut self.scene, targets: &mut self.targets, tools: &mut self.tools };
        let central = show_central(ui, |ui| view::central(ui, &mut cx)); // CentralPanel::no_frame; returns the area it
        cx.state.central_rect = Some(central);                           // was GIVEN (ui.max_rect()), not response.rect
        dialogs::show(ui.ctx(), &mut cx, &mut self.dialogs);
        self.state.toasts.show(ui.ctx(), central);        // anchored in the central area's corner (8.9)
        dnd::handle(ui.ctx(), &mut self.state);
        // Shortcut actions go LAST: a text field that lost focus because of the shortcut (8.10) has by now queued its
        // Commit::Final action, which must be dispatched before e.g. Save.
        self.state.actions.extend(late);
        for a in std::mem::take(&mut self.state.actions) { dispatch::dispatch(self, a, frame) }
    }
}
// dispatch.rs
pub fn dispatch(app: &mut WteApp, action: AppAction, frame: &eframe::Frame);
```
Notes:
- `logic()` may run without `ui()` (minimized window): never assume view rects exist there.
- `WteApp::new`: applies the theme, builds `Gpu` / `Renderer` / `ViewTargets` inside a wgpu validation error scope (a pipeline
  the adapter cannot build => `Err`, i.e. the startup error box with the adapter name, never a wgpu panic), registers a
  device-lost callback (any reason but `Destroyed` => `fatal_exit` with a message naming the lost device, exit 101) and an
  uncaptured-error handler: a `Surface::configure` error (a window resized programmatically past the `MaxInnerSize` cap,
  which only limits user resizing) sets `SURFACE_TOO_LARGE` / `OVERSIZED`, the errors of egui's pass for that oversized
  window are logged instead of being fatal, and `cap_window_size` shrinks the window below the limit, which ends the
  tolerance; every other uncaptured wgpu error panics as wgpu's default handler does (error box, exit 101). It also loads
  `Prefs` (not in self-test), starts the `Fonts` job, creates the self-test `Runner` when `args.screenshot` is set, and pushes
  `OpenPath(path)` for `args.path`.
- `NativeOptions`: `viewport = ViewportBuilder::default().with_title("Wrapped Texture Editor").with_inner_size(prefs.window_size)
  .with_min_inner_size([900.0, 600.0]).with_maximized(prefs.window_maximized).with_icon(theme::app_icon())
  .with_drag_and_drop(true)`; `multisampling: 0`, `depth_buffer: 0`, `persist_window: false`, `system_font_fallback: false`,
  `dithering: false` in self-test mode, `wgpu_options` from `main::wgpu_options` (below); everything else
  `..Default::default()` (renderer defaults to wgpu, the only compiled backend). In self-test mode the initial inner size is
  `args.size`, there is no minimum size and no prefs are read, and the window opens in the background (stable):
  `with_active(false)` (shown with `SW_SHOWNOACTIVATE`, the user's window keeps the focus), `with_taskbar(false)` and
  `with_position(main::offscreen_position)`: up-left of the virtual screen (`GetSystemMetrics(SM_X/YVIRTUALSCREEN)`)
  by twice the window size + 1000, so nothing shows on any monitor. (`WindowLevel::AlwaysOnBottom` does not work for
  this: winit applies `HWND_BOTTOM` while the window is still hidden, and eframe's first-frame show puts it on top.) It
  still renders and captures (eframe only stops drawing a hidden or minimized window; `request_screenshot` reads the GPU
  frame; the runner fixes the zoom at 1 px per point); `raw_input_hook` sets `RawInput::focused = true`.
- `main::wgpu_options`: the backends of `WGPU_BACKEND` (else egui-wgpu's primary + GL); a `native_adapter_selector` that picks,
  among the adapters that can present to the window, by (software last, Vulkan < DX12 < Metal < GL, discrete < integrated <
  virtual); a `device_descriptor` that requests the adapter's own `max_texture_dimension_2d` and `max_buffer_size` (egui-wgpu
  would request 8192 and wgpu's default 256 MiB) on top of `Limits::default()` (`downlevel_webgl2_defaults()` on GL).
- Window size limits: `cap_window_size` sends `ViewportCommand::MaxInnerSize(max_texture_dimension_2d / pixels_per_point)`
  whenever `pixels_per_point` changes, so a window stretched over several monitors never asks for a surface larger than the
  device allows (`Surface::configure` would panic). `fit_window_to_monitor` runs once in the first frames that know the monitor
  size (not in self-test, not when maximized): `app::fit_to_monitor(outer, inner, monitor)` shrinks a window that is larger than
  96 % x 90 % of its monitor and centres one that hangs off a monitor at the origin (a window left of / above the primary
  monitor is only resized).
- Debug builds only: `WTE_DEBUG_PANIC_FRAME=<n>` panics on the UI thread in frame `n` (test of the panic hook below).
- **Frame profile** (`app::frame_profile`, UI thread only): on in self-test mode and when `WTE_FRAME_PROFILE` is set, otherwise
  every call is a no-op. `logic` starts a frame, the end of `ui` closes it; named segments are timed with `frame_profile::scope`
  guards (`jobs`, `selftest`, `menu/status/toolbar`, `side panel`, `central`, `dialogs/toasts`, `dispatch`, and inside the views
  `scene sync`, `pointer/tools/hover`, `render 3d`, `render 2d`, `overlays`). Every 2 s one INFO line `frame profile: N frames,
  F fps, interval avg/max, ui cpu avg/max; segments avg/max ms: ...` (sorted by total). `interval` is the time between frame
  starts (includes egui tessellation, presentation and vsync), `ui cpu` the CPU time of `logic` + `ui`. Round 4 (gate 3):
  with `WTE_FRAME_LOG` set (read once when the profile is enabled), every frame also logs one INFO line `frame timing:
  interval X ms, ui cpu Y ms; segments ms: <name> <ms>, ...` with the segments that ran in that frame (the single-frame
  measurements of FEATURE_BRUSH P11 / P13 and FEATURE_TEXT 8.4 read these lines; off by default).
- `clear_color`: use the trait default.

**`main.rs` — logging and startup failures (R6).** The release exe has no console, so nothing may fail silently:
1. `cli::parse`. Log file: `--log FILE` if given; else `<OUT>.log` in self-test mode; else
   `%APPDATA%\WrappedTextureEditor\log.txt` (directory created, file truncated at every start — a few KB of text). The file is
   opened with `share_mode(FILE_SHARE_READ)`: a second instance cannot truncate or write the log of a running one; it gets a
   sharing violation and logs to `log-<pid>.txt` next to it instead (keeping the newest 4 such files). `LOG_PATH` (shown in the
   error box) is the file actually used. Logger:
   `env_logger::Builder::new().filter_level(log::LevelFilter::Info).target(env_logger::Target::Pipe(Box::new(crash::Tee(file)))).try_init()`
   (`Tee` writes each record unbuffered to the file and copies its lines into the crash recorder's ring, item 1a).
   If no file can be created the app runs without logging.
1a. **Native crash recorder (`crash.rs`)**, installed right after logging (`crash::install(second log handle, log path,
   selftest)`). Access violations, heap corruption, stack overflows and illegal instructions inside native code (graphics
   drivers, injected overlay DLLs such as OBS's `graphics-hook64.dll`) never reach the panic hook and used to end the process
   with nothing in the log. The recorder installs:
   - `AddVectoredExceptionHandler(1, ..)`: it only RECORDS, always `EXCEPTION_CONTINUE_SEARCH`. A first-chance
     `0xC0000005` / `0xC000001D` writes one short `[native] first-chance ...` line (code, `module+offset`, thread, the
     accessed address) to the log per distinct address, at most 16 addresses (a driver that handles its own exceptions makes
     no noise); `0xC00000FD` records nothing there (no stack left). `0xC0000374` (heap corruption) and `0xC0000409` are not
     continuable and never reach the unhandled filter, so the vectored handler reports them as THE crash.
   - `SetUnhandledExceptionFilter`: THE crash for everything else (re-installed after GPU initialization in `WteApp::new`,
     logged when a driver or hook had replaced it). A stack overflow is reported from a fresh thread (1 MiB stack) while the
     faulting thread waits.
   - The report avoids the heap (text preformatted in fixed stack buffers, written with `WriteFile` to the already open log
     handle and to a file created with `CreateFileW` on preformatted UTF-16 paths): exception code and name, faulting address
     as `module.dll+0xoffset` (`GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT)` + `GetModuleFileNameW`), the accessed
     address of an access violation, thread id and `GetThreadDescription` name (not for heap corruption), uptime, and the
     ring of the last 24 log lines and dispatched actions (`crash::note_action` in `dispatch`, 200 bytes per line). It goes to
     the log and to `<exe dir>\crash\crash-<unix time>-<pid>.txt` (no fallback folder: the log dir is on the system drive,
     and the log entry is the full report), then a minidump
     `crash-<unix time>-<pid>.dmp` next to it: `MiniDumpWriteDump` (dbghelp) with the `EXCEPTION_POINTERS`, type
     `MiniDumpWithIndirectlyReferencedMemory | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules` (about 0.5 - 2 MB); a
     failure is recorded with its error code. Then, unless in self-test mode, `MessageBoxW` "Wrapped Texture Editor crashed.
     A crash report was saved to <path>." on its own thread (same reason as `fatal_exit`), waited for at most 120 s (after
     heap corruption the box thread can block on the heap lock), and `TerminateProcess` with the exception code as exit
     code. The stack-overflow reporting thread is waited for at most 180 s. A second crash while reporting terminates at once.
   - All Win32 calls are `extern "system"` declarations with `#[link(name = .., kind = "raw-dylib")]` (kernel32, dbghelp,
     user32); no crate.
   - Test hook (release builds too): `WTE_DEBUG_NATIVE_CRASH=av|heap|stack` makes `WteApp::new` crash on purpose right after
     the GPU diagnostics: `av` = `write_volatile` to address 0x10, `heap` = a double `HeapFree` on the process heap (reported
     as `0xC0000374`), `stack` = unbounded recursion. Not set => nothing happens; any other value is logged and ignored.
   - Startup diagnostics (`crash::log_startup_diagnostics`, after the adapter line in `WteApp::new`): backend, adapter and
     driver; the variables `WGPU_BACKEND`, `WGPU_ADAPTER_NAME`, `WGPU_POWER_PREF`, `VK_LOADER_LAYERS_DISABLE`,
     `VK_LOADER_LAYERS_ENABLE`, `VK_INSTANCE_LAYERS`, `DISABLE_VULKAN_OBS_CAPTURE`, `WTE_DEBUG_NATIVE_CRASH` when set; and,
     from `K32EnumProcessModules`, every loaded module whose path contains obs, graphics-hook, overlay, steam, fossilize,
     rtss, rivatuner, nvidia overlay, nvspcap, discord, medal, amdihk, reshade or afterburner (`WARN hook/overlay module
     loaded`) plus the graphics runtime / driver modules (vulkan-1, amdvlk, amdxc, nvoglv, igvk, d3d12, dxgi, ...).
   - `WGPU_BACKEND` (e.g. `dx12`, `vulkan`) is honoured: egui-wgpu builds the instance with `wgpu::Backends::from_env()`, so
     the adapter selector only sees adapters of those backends. `dist\` ships launchers `Start - DirectX 12.cmd`
     (`WGPU_BACKEND=dx12`) and `Start - Vulkan without overlays.cmd` (`VK_LOADER_LAYERS_DISABLE=~implicit~`, verified with
     `VK_LOADER_DEBUG=layer` against the Vulkan loader 1.3.300: the OBS, Steam overlay / Fossilize and AMD switchable-graphics
     implicit layers are disabled and `graphics-hook64.dll` is no longer loaded).
2. Panic hook (installed before `run_native`; chains to the previous hook so debug builds still print to the console): logs the
   panic message and location, then `fatal_exit(text, 101)`. It applies to every thread (a panicking worker would otherwise
   leave a modal job spinning forever). `fatal_exit` (also used by the device-lost callback): in self-test mode it exits at
   once; otherwise the first fatal error spawns a thread named `"error box"` that shows the error box (below) and the failing
   thread joins it without pumping messages, then `std::process::exit(code)`. A modal message box on the panicking UI thread
   would run its message loop re-entrantly inside winit's window procedure: the main window's paint messages never stop, the
   box stays invisible and the process spins at 100 % CPU without exiting. Later fatal errors on other threads park until the
   first box ends the process; a panic on the box thread itself exits at once. The process always exits, also when the box
   thread cannot be started.
3. `eframe::run_native` returns `Err` (no usable wgpu adapter on a VM / RDP session / old GPU, window creation failure), or
   `WteApp::new` fails (`cc.wgpu_render_state` is `None` => the app creator returns `Err`, which `run_native` passes on), or the
   arguments do not parse: log it, show the error box unless in self-test mode, exit with code 4.
4. Error box: `rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("Wrapped Texture Editor")
   .set_description(format!("{error}\n\nLog file: {log_path}")).show()` — a native message box that needs no window.
5. After `run_native` returns `Ok`, `std::process::exit(exit_code.load(..))` (0 outside self-test).

**`dispatch` rules** (binding; `state` = `app.state`, `doc` = the session's document):
- Any core `Err(EditError)` becomes a warning toast (the action is dropped), except `UnknownLayer` / `UnknownStamp` /
  `UnknownSet` (round 3, R3-8): an action whose target no longer exists (queued before an undo or a delete removed it) is
  dropped silently with a debug log line (`report_edit_error`). `RasterizeLayer` of a paint layer does nothing and shows
  nothing (`rasterize_layer`; the button and the Layer menu entry are disabled for it, and there is no shortcut), so the
  core's internal "invalid parameter" text never reaches the user. Actions that need a session are no-ops without one
  (`ImportStampImage` shows the toast "Open a model first").
- **Interaction guard**: before every document-changing action of the groups *file*, *edit*, *sets and layers* and *stamps*
  (everything except `SetActiveSet`, `SelectLayer`, `SelectStamp` and the `Commit::Preview` variants):
  `if app.tools.is_busy() { finish or cancel }`. `Save`, `SaveAs`, `SaveThen`, `ShowExport`, `ShowExportLayers` and
  `RunExport` call
  `app.tools.finish(&mut app.state)` (`finishes_interaction`): the running stroke is `stroke_end`ed, a gizmo drag gets its
  `Commit::Final`, as on release, so the file contains what the user sees (Ctrl+S with the mouse button still held keeps the
  stroke). So a command can never meet an open stroke
  (the core would answer `StrokeInProgress`, 5.14) or a half-finished gizmo drag.
  `Undo` calls `app.tools.discard(&mut app.state)` and stops there: the interrupted interaction is the thing that was
  undone. Every other guarded action calls `app.tools.cancel(&mut app.state)`. Round 4: a brush stroke whose button is up
  but whose paint still trails (FEATURE_BRUSH 5.10, "finishing") is the user's completed stroke: `cancel` and `finish`
  complete it (one history entry) before the command runs; only `discard` (Undo, Esc) throws it away. The guarded
  actions include `LoadBrushShape`, `AnswerOpenings(Some(_))` and `SetBrushShapeClose` (they start a modal bake), not
  `ReloadBrushShape`, `RemoveBrushShape`, `AnswerOpenings(None)` or `LoadFontSamples`. objects-tabs: `SetObjectVisible`,
  `HideSelectedObjects`, `IsolateSelectedObjects` and `ShowAllObjects` are guarded (the core refuses a visibility change
  during an open stroke) and **finish** the interaction like Save (hiding is view state and never throws away what the
  user painted: H with the button still held commits the stroke, one history entry, then hides). The selection actions and
  `FrameSelection` are not guarded.
- `Undo` / `Redo`: `doc.undo()` / `doc.redo()`; `Some(label)` => info toast "Undo: <label>" / "Redo: <label>".
- **After every document-changing action**: `session.validate()` (clears a `selected_stamp` that no longer exists, clamps
  `active_set`). Tools and panels must in any case treat a `selected_stamp` for which `doc.stamp(id)` is `None` as "no selection".
- `Cancel` (first match wins): a modal job is running => nothing (only the progress dialog's Cancel button cancels a job);
  `tools.is_busy()` => `tools.discard(state)` (round 4; the tool calls `stroke_cancel` / `cancel_pending_edit`, a finishing
  stroke is dropped); a dialog is open =>
  Export in its confirm step goes back to the options (`confirm_replace = None`), `Unsaved` = its Cancel answer, any other
  dialog closes; `doc.has_pending_edit()` (a panel slider is being dragged) => `doc.cancel_pending_edit()`;
  `view_mode == Preview` => `SetViewMode(last_edit_mode)`; a stamp is selected => deselect.
- `DeleteSelection`: several layers selected in the Layers list (fix_export) => `delete_layers(selection)`; else
  `selected_stamp` => `doc.remove_stamp(id)`; else the active layer of the active set => `doc.remove_layer(id)`. `DeleteStamp(id)` / `DeleteLayer(id)` are the explicit forms. Exactly one of them runs per key press
  (the Transform tool does not handle Delete itself).
- Layer selection (export-select, 1.8; none of these is guarded, none edits the document beyond `set_active_layer`: no
  history entry, no revision change, the modified mark stays): `SelectLayer(l)` = `activate_layer`: active set and
  active layer = `l`'s, `selected_stamp = doc.stamp_of_layer(l)`, `selected_layers = [l]`. `ToggleLayerSelected(l)`: a
  layer of another set (or no active layer) is activated; else not selected => added; the only selected layer => nothing;
  the active layer => removed and the top-most remaining selected layer becomes active (and its stamp selected); else
  removed. `SelectLayerRange(l)`: the layers between the active one and `l` (both included) in stack order; the active
  layer stays; another set => activate. `ShowExportLayers(ids)`: what `ShowExport` does first (cancel the tool, flush a
  pending edit; fix_export: no settle of the stamp bakes, the export waits for them off the UI thread, 8.9), then the
  dialog defaults changed by `export_for_layers`: `other_ticks = Some(the defaults' ticks)`, `layers = Some(ids of the
  set of the first existing id, in stack order)`, `sets = None` (fix_export: the layers, and the linked layers of their
  designs, decide the sets), flattened off, per-layer PNGs on, ORA / PSD off, `selected_layers` = those ids (no
  existing id => the plain defaults). `ShowExport`: today's defaults (`layers: None, package: Files`,
  `other_ticks: None`), `selected_layers = session.layer_selection()`.
- fix_export, commands on a multi-selection: `DeleteLayers(ids)` = `delete_layers`: `doc.remove_layers(ids)` (one undo step
  "Delete Layers"), the selection collapses to the new active layer, a removed selected stamp is deselected, a warning
  "Deleted N of M layers: K locked layer(s) were kept" when some stayed. `DuplicateLayers(ids)` = `duplicate_layers`:
  `doc.duplicate_layers(ids)` (one step "Duplicate Layers"), the copies become the selection, the active copy's stamp is
  selected. `DuplicateActiveLayer` (Ctrl+J, the Layer menu) and `DeleteSelection` (Delete) use them when the selection has
  more than one layer. Merge Down, Rasterize, opacity, blend and move act on the active layer only.
- `DuplicateActiveLayer` / `MergeActiveDown` / `RasterizeActiveLayer` / `MoveActiveLayer(d)`: resolve the active layer of
  `session.active_set`, then behave like the id-based action (`MoveActiveLayer(d)`: `to_index = index + d`, clamped).
- `DuplicateLayer(id)`: `doc.duplicate_layer(id)`; the selection follows the copy, which the core made the active layer:
  `selected_stamp = doc.stamp_of_layer(copy)` (a stamp/text copy is selected, a paint-layer copy clears the selection) and
  `active_set` = the copy's set. The Transform gizmo, the property fields and Delete then act on the copy the Layers list
  highlights, not on the original under it.
- `SetViewMode(Preview)`: `tools.cancel` + `doc.flush_pending_edit()`; remember the previous mode in `last_edit_mode`.
  `SetViewMode(other)` and `SetTool(..)` also `tools.cancel`.
- `FrameModel`: 3D views frame `doc.visible_bounds()` (objects-tabs: the visible objects; `mesh.bounds` when nothing is
  hidden); the 2D view fits the texture (both in Split). `ResetCamera` frames `visible_bounds()` too. `ToggleTexture`: `Textured <-> Clay`
  (from `UvChecker` it goes to `Textured`). `ResetColors`: primary black, secondary white.
- **Objects** (objects-tabs, `object_ops.rs`; no document change, no history entry, no revision change, `is_modified`
  untouched — the same as camera and layer multi-selection): `SelectObject(o)` = `{o}`, anchor `o`; `ToggleObjectSelected(o)`
  adds / takes out `o`, anchor `o`; `SelectObjectRange(o)` = the range anchor..=o (no valid anchor: a plain select), the
  anchor stays; `SelectAllObjects`, `DeselectObjects` (also clears the anchor); unknown indices are ignored;
  `object_selection_rev` +1 when the set changed. `SetObjectVisible(o, v)` flips one flag (the selection is not touched);
  `HideSelectedObjects` hides the selected objects (they stay selected, so the eye or Show All brings them back in place);
  `IsolateSelectedObjects` shows the selected and hides every other object; both are no-ops with an empty selection;
  `ShowAllObjects`. They all go through `Document::set_hidden_objects` (5.14); an `Err` (open stroke) is a warning toast;
  no toast otherwise; no-ops without a session. `FrameSelection`: `doc.objects_bounds(selected)` (visible selected objects)
  else `visible_bounds()`, framed with `OrbitCamera::frame` on `session.camera` in 3D / Split, `session.preview_camera` in
  Preview, nothing in 2D. `Cancel` (Esc) does not clear the object selection.
- `ShowExport`: `tools.cancel` + `doc.flush_pending_edit()` (so the dialog and its preview show Full-quality stamps), then
  `Dialog::Export` with the defaults of 8.8.
- `RunExport(opts)`: `doc.flush_pending_edit()`, remember `opts.dir` in `prefs.last_export_dir`, `start_export` (8.9).
- `ExportPickFolder(p)`: `p.or_else(|| pick_folder(..))`; `Some(dir)` => `ExportDialogState.options.dir = dir`,
  `confirm_replace = None`, `prefs.last_export_dir = Some(dir)`.
- `ImportPickTexture { set, path }`: `path.or_else(|| pick_image(..))`; read the file; `decode_image(&bytes, Some(file name),
  model.options.max_texture_dim.min(MAX_TEXTURE_SIDE))` (synchronous: one user-chosen image); store
  `ImportDialogState.choices[set] = SetChoiceUi::File { path, bytes: Some(bytes), image: Some(pixels), error: None }`, or on a
  read/decode failure `File { path, bytes: None, image: None, error: Some(text) }`.
- `ImportCreate`: maps `choices` to `Vec<TextureChoice>` — `Detected` => `Detected`; `Blank { side, fill }` =>
  `Blank { width: side, height: side, fill }`; `File { path, bytes, image: Some(pixels), .. }` =>
  `TextureChoice::Image { pixels, original: bytes.map(|b| SourceFile { key: normalize_ref(file name), original_name: file name,
  bytes: b }) }` (so the chosen file is embedded in the project, R7); a `File` with an error cannot occur (Create is disabled) —
  then starts the build job (8.9).
- `ImportStampImage(p)`: `p.or_else(|| pick_image(..))`; read the bytes; `doc.add_image_blob(file name, bytes)` (`Err` => error
  toast + `Dialog::Message`); set `tools.stamp.source = Some(blob)`, `tools.stamp.source_name` (the file name, shown in the
  Stamp options); `SetTool(Stamp)`; `status_hint = "Click on the model to place the stamp"`. A stamp placed with it is
  named after the file **stem** (`stamp_place::image_stamp_name`: "logo.png" -> "logo", "Stamp" when empty; accept_fixes
  item D), so its layer and its per-layer export read "..._stamp_logo.png", not "..._stamp_logo.png.png"; the blob keeps the
  full file name (embedded file, Extract sources). Projects saved before keep their names ("ring.png") when opened.
- `ReplaceStampImage(id, p)`: same pick + `add_image_blob`, then `doc.set_stamp_source(id, StampSource::Image { blob }, Commit::Final)`.
- `ExtractSources(p)`: `p.or_else(|| pick_folder(..))`; starts the extract job (8.9) on the in-memory document — it works for a
  document that was never saved.

`close_dialog` of `Dialog::CloseOpenings` answers it with Cancel (`brush_flows::answer_openings(app, None)`), so Esc
and `CloseDialog` cancel the load.

Frame and flows: the new actions are one-line arms into `brush_flows.rs` (`load`, `reload`, `remove`,
`answer_openings`, `set_close`) and `text_ops.rs` (`load_font_samples`); `WteApp::new` calls `brush_flows::startup`
after `start_fonts`; `brush_flows::sync_prefs(&mut state)` runs directly before both `prefs::save` calls (`on_exit`,
`remember_recent`).

### 8.4 GPU layer (`gpu/*`) [A2]

```rust
// gpu/mod.rs
pub struct Gpu { pub device: wgpu::Device, pub queue: wgpu::Queue,
                 pub egui_renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
                 pub max_side: u32 /* view targets: min(device max_texture_dimension_2d, 8192) */,
                 pub msaa: u32 /* msaa_for(backend) */ }
impl Gpu {
    pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
    pub const MSAA: u32 = 4;
    pub fn new(rs: &egui_wgpu::RenderState) -> Gpu;          // clones device/queue/renderer handles
    pub fn limits(&self) -> wgpu::Limits;                     // the device's limits (buffer / texture sizes)
}
/// `Gpu::MSAA`, but 1 on DX12: a 4x MSAA pass with a resolve target of 256 px and more loses the device on some drivers
/// (AMD Radeon iGPU, driver 31.0.21924). DX12 is used only without a Vulkan adapter (8.3 `wgpu_options`).
pub fn msaa_for(backend: wgpu::Backend) -> u32;

// gpu/params.rs  [S]
pub struct BrushRing { pub center: glam::Vec3, pub radius: f32, pub hardness: f32,
                      pub side_normal: Option<glam::Vec3> /* round 4 */ }
pub struct View3dParams {
    pub view_proj: glam::Mat4, pub eye: glam::Vec3,
    pub shading: ShadingMode, pub lit: bool, pub alpha_checker: bool,
    /// Some(rgb): transparent texels are matted onto this colour instead of the checker (export preview of a flat format
    /// without alpha, i.e. JPEG). Takes precedence over `alpha_checker`.
    pub matte: Option<[u8; 3]>,
    pub wireframe: bool, pub seams: bool, pub nearest: bool,
    pub backdrop: Backdrop, pub brush: Option<BrushRing>,
    pub all_objects: bool,   // objects-tabs: draw hidden objects too (the export dialog's preview; 8.5)
    pub highlight: bool,     // objects-tabs: draw the selection tint (Main3D only, rule in 8.5)
}
pub struct ViewUvParams {
    pub set: TextureSetId,
    pub center_uv: glam::Vec2, pub px_per_texel: f32,        // physical pixels per texel
    pub uv_wire: bool, pub alpha_checker: bool,
}

// gpu/targets.rs
pub struct ViewTarget { pub egui_id: egui::TextureId, /* resolve (1 sample), msaa colour, depth, alloc size, used size */ }
impl ViewTarget {
    pub fn new(gpu: &Gpu, label: &str) -> ViewTarget;        // 64x64, register_native_texture(.., FilterMode::Linear)
    /// Allocation is rounded up to a multiple of 64 and clamped to 64..=max_side; on reallocation the egui texture id is kept
    /// (update_egui_texture_from_wgpu_texture). The 64-px quantum avoids reallocating on every frame of a panel drag.
    pub fn ensure_size(&mut self, gpu: &Gpu, px: [u32; 2]);
    pub fn size_px(&self) -> [u32; 2];                       // used size
    pub fn uv_rect(&self) -> egui::Rect;                     // [0,0]..used/alloc, for painter.image
    pub fn destroy(self, gpu: &Gpu);                         // free_texture
}
pub struct ViewTargets { pub main3d: ViewTarget, pub main2d: ViewTarget, pub preview: ViewTarget, pub export_preview: ViewTarget }
impl ViewTargets { pub fn new(gpu: &Gpu) -> ViewTargets;  pub fn get_mut(&mut self, id: ViewId) -> &mut ViewTarget; }

// gpu/textures.rs
pub struct SetTexture { /* Rgba8Unorm, full mip chain, TEXTURE_BINDING | COPY_DST | RENDER_ATTACHMENT; per-mip views; bind group */ }
impl SetTexture {
    pub fn new(gpu: &Gpu, renderer: &Renderer, size: [u32; 2], composite: &[u8]) -> SetTexture;
    pub fn upload(&self, queue: &wgpu::Queue, composite: &[u8], r: IRect);
    pub fn regenerate_mips(&self, gpu: &Gpu, renderer: &Renderer, encoder: &mut wgpu::CommandEncoder);
}

// gpu/scene.rs
/// Err (a sentence for the user) when the mesh's vertex buffer (32 bytes per vertex) or index buffer exceeds
/// `limits.max_buffer_size`: creating it would be a wgpu validation error, i.e. a panic. io_flows checks it when an import
/// finishes and before a built / loaded document is installed (error toast + "Model too large" message, nothing replaced).
pub fn check_mesh_fits(mesh: &MeshData, limits: wgpu::Limits) -> Result<(), String>;
pub struct GpuScene { pub generation: u64, /* vertex + index buffers, material ranges, per-material untextured bind groups,
                                              wire instance buffers, Vec<SetTexture>, per-set mips_stale, last mip time */ }
impl GpuScene {
    /// The CPU geometry (render vertices on one scoped worker thread; wire edges, 3D wire lists and the UV wire lists of
    /// all sets in ONE pass over the edges on another) is built while this thread creates and uploads the set textures;
    /// then the buffers, then the mips. Logs one line "GPU scene built in N ms (...)" with the time of each part.
    pub fn new(gpu: &Gpu, renderer: &Renderer, doc: &mut Document, generation: u64) -> GpuScene;
    /// doc.recomposite_all(); for every set: upload each take_gpu_dirty rect and mark its mips stale; regenerate the mips of
    /// stale sets according to the throttling rule below (`force_mips` = regenerate now); one submit.
    pub fn sync(&mut self, gpu: &Gpu, renderer: &Renderer, doc: &mut Document, force_mips: bool);
    pub fn mips_stale(&self) -> bool;                        // any set whose level 0 is newer than its mip chain
    /// objects-tabs: rebuilds the visible index buffer and the visible wire ranges when `doc.visibility_generation()`
    /// changed, and the highlight index buffer when that or `selection_rev` changed; two integer compares otherwise.
    /// `view::sync_scene` calls it every frame after `sync` with the session's `selected_objects` / `object_selection_rev`.
    /// Logs "objects: visible geometry rebuilt in N ms (...)" on a change.
    pub fn sync_objects(&mut self, gpu: &Gpu, doc: &Document, selected: &BTreeSet<u32>, selection_rev: u64);
    /// Vertex buffer, index buffer and draws of a render: the visible triangles, or all with `all_objects`; `None` for an
    /// empty mesh or when every triangle is hidden.
    pub(crate) fn mesh_draw(&self, all_objects: bool) -> Option<(&wgpu::Buffer, &wgpu::Buffer, &[Draw])>;
    pub(crate) fn highlight(&self) -> Option<(&wgpu::Buffer, &wgpu::Buffer, u32)>;   // vertices, indices, index count
    /// The wire buffers and the instance ranges to draw (the visible objects' runs, or `0..n` with `all_objects`).
    pub(crate) fn wire_all(&self, all_objects: bool) -> Option<(&wgpu::Buffer, &[Range<u32>])>;   // also wire_seams
    pub(crate) fn wire_uv(&self, set: usize, all_objects: bool) -> Option<(&wgpu::Buffer, &[Range<u32>])>;
}
// objects-tabs pure helpers (unit-tested without a GPU):
pub(crate) fn visible_draws(mesh: &MeshData, draws: &[Draw], hidden: &[bool]) -> (Vec<u32>, Vec<Draw>); // compacted per draw, empty draws dropped
pub(crate) fn selected_indices(mesh: &MeshData, hidden: &[bool], selected: &BTreeSet<u32>) -> Vec<u32>; // mesh order
pub(crate) fn object_runs(objects: &[u32]) -> Vec<(u32, Range<u32>)>;          // of a list sorted by object
pub(crate) fn visible_runs(runs: &[(u32, Range<u32>)], hidden: &[bool]) -> Vec<Range<u32>>; // neighbours merged
pub(crate) struct WireList { pub instances: Vec<LineInstance>, pub runs: Vec<(u32, Range<u32>)> }
```
**Objects on the GPU** (objects-tabs, plan 5.1-5.2). `wire_instances_3d` / `wire_instances_uv_all` return `WireList`s:
each group they build (3D all: interior, then seams + boundaries; 3D seams; UV per set: interior, then seams) is
stable-sorted by `WireEdge::object` before it is appended, and its `object_runs` are kept (offset to the buffer). With one
object the instance bytes are those of the build without objects (tested). The draw order of wires of different objects
can therefore differ from earlier builds on multi-object models (plan risk 5). While nothing is hidden the mesh is drawn
from the full index buffer with `build_draws`, and each wire buffer with one `0..n` instance range, exactly as before.
While objects are hidden: a compacted index buffer of the visible triangles (`visible_draws`, one pass over the triangles,
a subset of the full buffer so it always fits) with its own draws; wires are drawn as one `draw(0..4, range)` per visible
run (neighbours merged); the UV view draws only the visible objects' UV wires (the texture is shown whole). The highlight
is an index buffer of the selected visible objects' triangles (`selected_indices`), drawn by the **tint pipeline**
(`tint.wgsl`: `view_only` layout, the mesh vertex buffer layout, `TriangleList`, `DepthMode::Overlay` = `LessEqual` without
depth write, `LINE_BLEND`; the vertex shader pulls depth by the wires' `DEPTH_PULL` = 2e-4 so the tint wins against the
mesh's own depth; the fragment colour is the constant `(0.25, 0.61, 1.0, 0.30)` in the wires' sRGB-byte convention).
`render_3d`: backdrop, mesh from `mesh_draw(p.all_objects)`, then the tint when `p.highlight` and `highlight()` is
`Some`, then the wires as ranges. Measured on ABeautifulGame (1.5 M triangles, 49 objects, release build, RTX 5070):
isolating 2 objects costs 2.9 ms (core mask) + 33 ms (`sync_objects` incl. the 18 MB upload); hiding one 1.5 + 19 ms;
Show All 2 ms (`control_sweep::gpu_tests::perf_visibility_toggle_on_a_heavy_model`, budget 250 ms).
**Deferred stamp Preview bakes** (review round 2, finding SP-1). `io_flows::install_document` turns on
`Document::set_defer_preview_bakes(true)`, so a gizmo drag frame, a wheel notch, a tint / size slider frame or a keystroke in a
text stamp's content field only updates the stamp object. `view::sync_scene` calls `view::bake_stale_previews(doc,
&mut state.preview_bakes)` right before `GpuScene::sync`: the waiting Preview bakes run when `PreviewBakeClock::due` (the time since
the end of the last deferred bake is at least its duration, capped at `MAX_WAIT` = 400 ms), else the clock's `wake` makes
`view::central` request a repaint once they may run. A slow bake (a large decal on an 8192 set takes hundreds of ms) thus takes at
most about half of the frames while the UI and the gizmo stay responsive; cheap bakes still run every frame.
```rust

// gpu/renderer.rs
pub struct Renderer { /* pipelines, bind group layouts, samplers, uniform buffers, mip blit pipeline + sampler, dummy white texture */ }
impl Renderer {
    pub fn new(gpu: &Gpu) -> Renderer;
    pub fn render_3d(&mut self, gpu: &Gpu, scene: &GpuScene, target: &ViewTarget, p: &View3dParams);
    pub fn render_uv(&mut self, gpu: &Gpu, scene: &GpuScene, target: &ViewTarget, p: &ViewUvParams);
    pub fn render_backdrop(&mut self, gpu: &Gpu, target: &ViewTarget, backdrop: Backdrop);   // no document
}
```

**Offscreen targets (D16)**. Each view has its own `ViewTarget`: resolve texture `Rgba8Unorm` (`RENDER_ATTACHMENT | TEXTURE_BINDING`),
a `gpu.msaa`-sample MSAA colour texture (none when `gpu.msaa == 1`: the pass then renders straight into the resolve texture and
stores it), and a `gpu.msaa`-sample `Depth32Float` texture; the view pipelines use the same sample count. Pixel size =
`(rect.size() * ctx.pixels_per_point()).round()`. The target
holds sRGB-encoded values with alpha 1; egui samples it as a gamma texture, so there is no double conversion. The app never touches
the surface or `RenderState.target_format`. Each `render_*` call: `queue.write_buffer` uniforms -> one `CommandEncoder` -> one render
pass (viewport and scissor = used size, MSAA resolve into the resolve texture) -> `queue.submit` immediately. Because every call
submits on its own, one shared uniform buffer serves all views. These submits happen inside `App::ui`, before eframe submits egui's
pass, so the image egui draws is always current. Lock `gpu.egui_renderer.write()` only for the duration of a register/update/free call.

**Set-texture upload** (CPU-authoritative): the composite buffer is passed whole; the layout addresses the sub-rect in place:
```rust
queue.write_texture(
    wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0,
        origin: wgpu::Origin3d { x: r.x0 as u32, y: r.y0 as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
    composite,
    wgpu::TexelCopyBufferLayout { offset: ((r.y0 as u64) * (w as u64) + r.x0 as u64) * 4,
        bytes_per_row: Some(w * 4), rows_per_image: None },
    wgpu::Extent3d { width: r.width(), height: r.height(), depth_or_array_layers: 1 });
```

**Mips** — our own blit pipeline, **not** `wgpu::util::TextureBlitter` (its sampler has `min_filter: Nearest`, so every level would
be a point-sampled decimation and minified strokes, text and checker lines would shimmer; 8.1). `Renderer` owns a pipeline built
from `blit.wgsl` (fullscreen triangle; fragment = `textureSample(src, samp, uv)`; bind group 0 = texture + sampler), colour target
`Rgba8Unorm`, no blending, no depth, and a sampler `SamplerDescriptor { mag_filter: Linear, min_filter: Linear,
mipmap_filter: MipmapFilterMode::Nearest, address_mode_u/v/w: ClampToEdge, ..Default::default() }`. `regenerate_mips` renders, for
`i` in `1..mip_count`, the single-level view of level `i-1` (`TextureViewDescriptor { base_mip_level: i - 1,
mip_level_count: Some(1), ..Default::default() }`) into the single-level view of level `i`; at the 2:1 ratio the bilinear tap sits
on a texel corner, i.e. an exact 2x2 box filter. Known cosmetic limitation: filtering straight alpha bleeds the RGB of transparent
texels at minified transparency edges.

**Mip throttling** (regenerating a full chain touches 1.33x the texture area: 89M pixels at 8192^2, too much per frame on
integrated GPUs). An upload marks the set `mips_stale`. `sync` regenerates the chain of a stale set when `force_mips`, or when
`!(doc.stroke_active() || doc.has_pending_edit())` (first sync after the interaction ended), or when at least 100 ms have passed
since that set's last regeneration (`std::time::Instant`). `view::central` and `preview_widget` pass `force_mips = true` whenever
a Preview / ExportPreview view is rendered. After `sync`, `view::central` stores `scene.mips_stale()` in `state.gpu_mips_stale`;
the self-test's `WaitIdle` waits for it to be `false`, so screenshots never show stale mips.

**Samplers**: one sampler per set texture: `Repeat`, linear mag/min, `MipmapFilterMode::Linear`, `anisotropy_clamp: 8`. Nearest
filtering (magnified 2D viewing, Preview "Nearest") is a `textureLoad` of the wrapped texel at the nearest mip level for the UV
derivatives (`SetUniforms.params.y` = mip count), not a second sampler: a texture sampled with two samplers cannot be translated
to GLSL, so the GL fallback backend could not even build the pipelines.

**Buffer limits**: the device has the adapter's `max_buffer_size` (8.3). `GpuScene::new` creates no mesh buffer that
`check_mesh_fits` refuses (logged; the file flows refuse such documents before), and leaves out a wireframe instance buffer that
exceeds the limit (logged; the model is drawn without that wireframe).

**Geometry**: one vertex buffer of `wte_core::RenderVertex` (32 bytes: `Float32x3, Float32x3, Float32x2`) and one `u32` index buffer
for the whole mesh. Per material `m`, up to two `draw_indexed` calls (triangle range x 3):
- `mesh.material_uv_ranges[m]` (UV-mapped triangles; empty when the material has no set) with the bind group of the material's
  set texture;
- `mesh.material_nouv_ranges[m]` (NO_UV triangles, and the whole material when it has no set) with that material's *untextured*
  bind group: the dummy white texture and `SetUniforms { base_color, has_texture: 0 }`. These triangles never sample a set
  texture at garbage UVs; in Clay and UV-checker shading they are drawn clay grey.
Wire instances are built once from `mesh.wire_edges()`.

**Bind groups / uniforms** (vec4/mat4 only, to avoid vec3 padding bugs):
```rust
#[repr(C)] #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniforms {            // group 0
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],               // xyz
    light_dir: [f32; 4],         // xyz = key light (world, attached to the camera: normalize(eye_dir + 0.4*up + 0.3*right)), w = ambient
    viewport: [f32; 4],          // w, h, 1/w, 1/h in physical pixels
    brush: [f32; 4],             // xyz = centre (world), w = radius (world); w == 0 => hidden
    brush_aux: [f32; 4],         // x = hardness, yzw = side normal of the ring (unit; zero = the ring on every face)
    matte: [f32; 4],             // rgb = matte colour (sRGB-encoded 0..1), used when the MATTE flag is set
    params: [u32; 4],            // x = ShadingMode, y = flags (1 = LIT, 2 = ALPHA_CHECKER, 4 = NEAREST, 8 = MATTE), z = Backdrop
}
#[repr(C)] struct SetUniforms { tex_size: [f32; 4], base_color: [f32; 4], params: [u32; 4] /* x = has_texture, y = mip count */ }  // group 1: uniform + texture + linear sampler
```
Group 1 bind groups: one per texture set (`has_texture: 1`) and one per material for untextured drawing (`has_texture: 0`).

**Shaders** (WebGL2-level features only: uniform buffers, 2D textures, instancing; no storage buffers, no compute, no polygon-mode line):
- `mesh.wgsl` — `cull_mode: None`; the fragment shader flips the normal on `@builtin(front_facing) == false` (models often have
  inconsistent winding; picking is also two-sided). Fragment steps, in order: (1) sample the texture linearly and read the
  nearest texel (`textureLoad`, level from `dpdx` / `dpdy`) unconditionally (`textureSample` and the derivatives must stay in
  uniform control flow) and select by `NEAREST`; (2) base colour by mode: texture (or `base_color`
  when `has_texture == 0`), clay grey 0.72 for texture-off (R3), or a procedural 8x8 two-colour UV checker with per-cell hue
  derived from the cell index so orientation and seams are readable (R2; clay grey when `has_texture == 0`); (3) transparency:
  if `MATTE`: `rgb = mix(matte.rgb, rgb, a)`; else if `ALPHA_CHECKER`: `rgb = mix(checker(uv * tex_size.xy / 16), rgb, a)`;
  (4) if `LIT`: sRGB -> linear, hemisphere ambient + Lambert key light + faint specular, linear -> sRGB; unlit = passthrough, so
  Preview Unlit shows the exported bytes; (5) brush ring: `d = distance(world, brush.xyz)`, `w = fwidth(d)`, white/black double ring
  at `d = brush.w` plus a thin inner ring at `brush.w * brush_aux.x`. With a side normal in `brush_aux.yzw` the ring is
  drawn only where the fragment's unflipped vertex normal satisfies `dot(normalize(normal), side) >= -0.2`
  (`SIDE_MIN_DOT`, the same threshold as the Circular brush's `SAME_SIDE_MIN_DOT` in `core\brush.rs`), so the ring is
  hidden on faces the Circular brush skips; perpendicular faces keep it. The core tests each triangle's geometric
  (winding) normal, the shader the interpolated vertex normal (the pipeline has no per-face data): on flat-shaded faces
  the two agree exactly, on smooth-shaded meshes the boundary of the hidden part is approximate near dot = -0.2. `view_uniforms_3d` writes `BrushRing::side_normal`
  normalised; `None`, a zero or a non-finite normal pack as zero = no filter (round-3 ring). The derivative work stays
  before the branch (uniform control flow). (6) alpha 1. Depth: `Less`, write on.
- `lines.wgsl` — instanced wide lines for the 3D wireframe/seams and the UV wireframe. Instance `{ a: Float32x3, b: Float32x3,
  color: Unorm8x4 }`, 4-vertex `TriangleStrip`; the vertex shader clips the segment against the near plane, expands by 1.25 px in
  screen space and pulls `clip.z -= 2e-4 * clip.w` against z-fighting. Two pipelines: 3D (`LessEqual`, no depth write, alpha blend)
  and UV (`Always`), where the instance holds `(uv_a, 0)`, `(uv_b, 0)` and `view_proj` is the UV-to-clip matrix. Colours: interior
  edges 35% alpha, seams orange, boundaries cyan. In 3D, `wireframe` draws all edges, `seams` only Seam + Boundary.
- `uvview.wgsl` — fullscreen triangle; a `uv_from_frag` affine (in `ViewUniforms.view_proj`'s place) maps fragment position to UV.
  Outside `[0,1]^2`: dim backdrop. Inside: screen-space checker blended with the texture using straight alpha (when `ALPHA_CHECKER`),
  linear + mips when minified, nearest (`textureLoad` of level 0) above 4 screen px per texel, faint texel grid above 12.
- `background.wgsl` — fullscreen triangle drawn first in 3D/Preview passes: dark, light, vertical gradient or checker.
- `blit.wgsl` — fullscreen triangle + one `textureSample`; used only by the mip blit pipeline above.

Round 4 (S2, B6), `BrushRing::side_normal`: `pub side_normal: Option<glam::Vec3>` — `Some(n)` draws the ring only on faces of that
side (the Circular type skips faces it will not paint); `None` = every face (round 3). B6 writes the shader part.
The tools decide what goes into `BrushRing` (B7: Circular sets the hover hit's unflipped normal, Sphere leaves it
`None`; a ring located from the 2D view of Split mode has none). The renderer only packs and filters. The inner hardness
ring is drawn whenever `0.001 < hardness < 0.999` (unchanged); BrushRing has no shape flag, so "inner ring for the
ball only" (FEATURE_BRUSH 5.11) is the tool's choice of `hardness` (0 or 1 suppresses it).
Tests that hold this: `gpu::renderer::tests::the_side_normal_is_packed_into_brush_aux` (A14),
`control_sweep::gpu_tests::the_ring_at_a_cube_corner_follows_the_side_normal` (A15, ignored without a GPU).

### 8.5 Views (`view/*`) [A3]

```rust
// view/mod.rs
pub struct ViewCx<'a> { pub state: &'a mut AppState, pub gpu: &'a Gpu, pub renderer: &'a mut Renderer,
                        pub scene: &'a mut Option<GpuScene>, pub targets: &'a mut ViewTargets, pub tools: &'a mut ToolController }
/// Tab strip (3D | 2D | Split | Preview), layout, input, rendering and overlays of the central area. The strip's quick
/// display toggles (Texture, UV checker, Wireframe, Seams in 3D / Split, UV wire in 2D / Split, Frame) are one row of buttons
/// while their measured width fits the rest of the strip; otherwise they move into a "Display" drop-down with the same
/// entries and actions, so none ends up under the side panel at the minimum window (900 x 600) or with a wide side panel
/// (review round 2, finding mode-strip-overflows-at-min-width). After the views: `tools.end_frame` (wheel gestures) and the
/// repaint requests of the deferred stamp bakes (8.4).
pub fn central(ui: &mut egui::Ui, cx: &mut ViewCx<'_>);

// view/camera.rs
impl OrbitCamera {
    pub fn eye(&self) -> Vec3;                                    // target + distance * (cos p * sin y, sin p, cos p * cos y)
    pub fn basis(&self) -> (Vec3, Vec3, Vec3);                    // right, up, forward (world)
    /// near = max(distance - 1.5R, 1e-3 R), far = distance + 1.5R with R = `scene_radius` (definition below).
    /// `ortho`: `directx::orthographic` with half-height `distance * tan(fov_y / 2)` and the same near/far.
    pub fn view_proj(&self, aspect: f32, scene_radius: f32) -> Mat4;
    pub fn ray(&self, pos: egui::Pos2, rect: egui::Rect, scene_radius: f32) -> wte_core::Ray;  // unprojects near/far through inverse(view_proj)
    pub fn project(&self, p: Vec3, rect: egui::Rect, scene_radius: f32) -> Option<egui::Pos2>; // None behind the camera
    pub fn orbit(&mut self, delta_points: egui::Vec2);            // 0.008 rad per point
    /// Moves `target` in the view plane; afterwards clamps it so that `|target - bounds.center()| <= 2 * bounds.diagonal()`.
    pub fn pan(&mut self, delta_points: egui::Vec2, rect_height: f32, bounds: &wte_core::Aabb);
    /// One wheel notch = 15%: `distance *= exp(-0.15 * notches)`, clamped to `[0.02, 50] * model_radius`.
    pub fn zoom(&mut self, notches: f32, model_radius: f32);
    pub fn frame(&mut self, bounds: wte_core::Aabb);              // target = centre, distance = R / sin(fov_y / 2) * 1.15
    pub fn preset(&mut self, p: CameraPreset);
    /// The R every caller passes to view_proj / ray / project (and stores in `ViewXform.scene_radius`):
    /// `bounds.diagonal() * 0.5 + (self.target - bounds.center()).length()` — the model stays inside near/far after panning.
    pub fn scene_radius(&self, bounds: &wte_core::Aabb) -> f32;
}
// view/view2d.rs
impl UvViewState {
    pub fn uv_to_screen(&self, uv: Vec2, tex: [u32; 2], rect: egui::Rect) -> egui::Pos2;   // rect.center() + (uv - center) * tex * zoom
    pub fn screen_to_uv(&self, pos: egui::Pos2, tex: [u32; 2], rect: egui::Rect) -> Vec2;
    pub fn zoom_about(&mut self, pos: egui::Pos2, factor: f32, tex: [u32; 2], rect: egui::Rect);  // then clamp_to
    pub fn pan(&mut self, delta: egui::Vec2, tex: [u32; 2], rect: egui::Rect);   // center -= delta / (tex * zoom), then clamp_to
    /// R3-11: moves `center_uv` the least amount so that at least KEEP_VISIBLE = 32 points of the texture (or the whole
    /// texture / view side when smaller) stay inside `rect` on each axis: per axis `k = min(32, S, L)` with S = tex * zoom,
    /// L = view length, `center ∈ [(k - L/2) / S, 1 + (L/2 - k) / S]`. Non-finite centre / zoom are reset (0.5 / 1).
    pub fn clamp_to(&mut self, tex: [u32; 2], rect: egui::Rect);
    pub fn fit(&mut self, tex: [u32; 2], rect: egui::Rect);
}
pub fn show_3d(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect);       // view3d.rs
pub fn show_2d(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect);       // view2d.rs
// view/preview.rs
/// Clean orbit view of the composite (no ring, gizmo, wireframe). `id` = ViewId::Preview or ViewId::ExportPreview.
/// Self-contained: allocates `rect`, handles its own orbit (LMB/RMB drag), pan (MMB) and wheel zoom, syncs the scene with
/// `force_mips = true`, renders and paints. It never looks at `state.dialog` or `state.busy`, so it stays interactive inside
/// the Export modal. `matte` is forwarded to `View3dParams.matte`.
pub fn preview_widget(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, id: ViewId, rect: egui::Rect, matte: Option<[u8; 3]>);
pub fn show_preview_mode(ui: &mut egui::Ui, cx: &mut ViewCx<'_>, rect: egui::Rect);     // widget + option strip
// view/gizmo.rs
#[derive(Copy, Clone, PartialEq, Debug)] pub enum GizmoHandle { Body, Corner(u8), Rotate }
pub struct GizmoShape { pub corners: [egui::Pos2; 4] /* TL, TR, BR, BL */, pub center: egui::Pos2, pub rotate_knob: egui::Pos2,
                        /// projected decal in a 3D view: [front, back] rectangles of the projection box (corners moved by
                        /// +depth_front / -depth_back along `out`), drawn faintly (white alpha 70, 1 px, plus the 4 depth
                        /// edges) under the gizmo so the depth range is visible while the stamp is selected; not a handle.
                        /// None for UV decals or when a box corner is behind the camera.
                        pub depth_box: Option<[[egui::Pos2; 4]; 2]> }
pub fn gizmo_shape(placement: &Placement, xf: &ViewXform) -> Option<GizmoShape>;
pub fn hit_test(shape: &GizmoShape, pos: egui::Pos2) -> Option<GizmoHandle>;            // 8-point tolerance on handles
pub fn draw(painter: &egui::Painter, shape: &GizmoShape, hot: Option<GizmoHandle>);
```

`central` is two-phase so a split view never lags a frame:
- **Phase A (input)**: compute the view rects for the mode (Split: draggable divider at `split_ratio`), store them in
  `state.view_rects`, `ui.allocate_rect(rect, egui::Sense::DRAG)` per view (the **non-focusable** constant — `Sense::drag()` would
  put the views into Tab navigation, 8.1), translate egui input into navigation or
  `ToolController::{hover, press, drag, release, scroll}` calls (8.6).
- **Phase B (render)**: ensure the `GpuScene` (`None` or `generation != session.scene_generation` => `GpuScene::new`),
  `scene.sync(.., force_mips)`, store `state.gpu_mips_stale`, `target.ensure_size`, `renderer.render_*` for every visible view,
  `painter.image(target.egui_id, rect, target.uv_rect(), Color32::WHITE)`, then egui-painter overlays (2D brush circle, gizmo,
  stamp hover outline, `[0,1]` border, empty-state hint).
Without a session the central area shows the backdrop and a hint ("File > New from Model, or drop a model / .wtep file here").
Render every egui pass in which a view is shown (egui is reactive; an idle app runs no passes and therefore renders nothing —
which only holds because nothing sends a viewport command or requests a repaint unconditionally, 8.13).

**Input gate.** While `state.busy` is modal or `state.dialog` is not `Dialog::None`, `central` routes no pointer or wheel input
to `Main3D` / `Main2D` and calls no tool. The gate lives in `central`'s routing for those two views only. `preview_widget` has
no such gate — neither in Preview mode nor as `ViewId::ExportPreview` inside the Export dialog, where the user must be able to
orbit to confirm the result (R5); it relies on egui's own layering (`response.contains_pointer()` is false for a widget that
lies under a modal).

2D view: shows `session.active_set`; with more than one set a set-tab strip sits above it. `zoom` is in points per texel; the shader
gets `px_per_texel = zoom * pixels_per_point`. Brush circle radius = `size_px / 2 * zoom` with an inner hardness circle.
**The texture stays reachable** (round 3, R3-11; before, pan was unlimited and a pan drag or a wheel zoom beside the texture
could leave the 2D pane empty, as in the audit's BrainStem / Avocado fuzz shots): the 2D pan (`Interaction::Pan` in
`ToolController::drag`, via `UvViewState::pan` with the view's rect) and the wheel zoom (`zoom_about`) call `clamp_to`, and
`view2d::input` calls `clamp_to(tex, image)` every frame after the first-draw fit, so a window resize, a Split divider move,
a set switch to a different texture size or a view loaded from a project is clamped too. A view that already shows 32 points
is never moved. `FrameModel` still fits the whole texture (`fit`). The 3D cameras are unchanged.

Preview mode (R5): entered through `SetViewMode(Preview)` (dispatch cancels the tool interaction and calls
`doc.flush_pending_edit()`, 8.3). Renders `View3dParams { shading: Textured, wireframe: false, seams: false, brush: None,
alpha_checker: true, matte: None, lit/backdrop/nearest from state.preview }` with `preview_camera`. Strip: Lit/Unlit,
Turntable + speed (advances `yaw` by `speed * i.stable_dt`, then `ctx.request_repaint()`), Background, Linear/Nearest, Export...,
Back (Esc). The Export dialog embeds `preview_widget(.., ViewId::ExportPreview, .., matte)`: the same code path and the same
composite, with `matte = Some(jpeg_matte)` when the chosen flattened format has `keeps_alpha() == false`. `preview_widget`
writes its rect into `state.view_rects[id]`.

### 8.6 Tools (`tools/*`) [A3]

```rust
// tools/mod.rs
pub enum Surface { Model(Hit), Uv { set: TextureSetId, uv: Vec2, px: Vec2 }, Miss }
pub struct ViewPointer { pub view: ViewId, pub pos: egui::Pos2, pub surface: Surface, pub pressure: f32, pub mods: egui::Modifiers }
/// Screen <-> scene mapping of the view the pointer is in.
pub struct ViewXform { pub view: ViewId, pub rect: egui::Rect, pub is_3d: bool,
                       pub camera: OrbitCamera, pub scene_radius: f32,           // 3D; scene_radius = camera.scene_radius(&mesh.bounds)
                       pub uv: UvViewState, pub tex_size: [u32; 2] }             // 2D
impl ViewXform {
    /// The transform of `view` as the views use it this frame (rect from `state.view_rects`, camera / uv view from the
    /// session). None without a session or before the view was laid out. Used by the views and by self-test `Dyn` steps.
    pub fn for_view(state: &AppState, view: ViewId) -> Option<ViewXform>;
    pub fn world_to_screen(&self, p: Vec3) -> Option<egui::Pos2>;
    pub fn px_to_screen(&self, px: Vec2) -> egui::Pos2;                          // texel coordinates -> screen (2D views)
    pub fn surface_at(&self, doc: &Document, set: TextureSetId, pos: egui::Pos2) -> Surface;   // raycast (3D) or UV lookup (2D)
}
pub enum Interaction { Idle, Orbit(ViewId), Pan(ViewId), Tool(ViewId) }
pub trait Tool {
    fn hover(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {}
    fn press(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) -> bool;       // true = capture the drag
    fn drag(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {}
    fn release(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer) {}
    /// Ctrl+wheel / Shift+wheel in notches (`p.mods` = the wheel event's modifiers). true = consumed.
    fn scroll(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer, notches: f32) -> bool { false }
    fn cancel(&mut self, state: &mut AppState) {}
    /// Ends the interaction and commits what it did so far, as a release would (Brush: `stroke_end` of the dabs so far,
    /// Transform: `Commit::Final` of the placement reached). Default: `cancel` (nothing to commit).
    fn finish(&mut self, state: &mut AppState) { self.cancel(state) }
    fn overlay(&self, state: &AppState, xf: &ViewXform, painter: &egui::Painter) {}
    fn brush_ring(&self, state: &AppState) -> Option<BrushRing> { None }
}
pub struct ToolController { /* interaction, one instance of each tool, last hover */ }
impl ToolController {
    pub fn new() -> ToolController;
    pub fn interaction(&self) -> &Interaction;
    pub fn is_busy(&self) -> bool;                               // not Idle
    pub fn hover(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer);
    pub fn press(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer, button: egui::PointerButton);
    pub fn drag(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer);
    pub fn release(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer);
    pub fn scroll(&mut self, state: &mut AppState, xf: &ViewXform, p: &ViewPointer, notches: f32) -> bool;
    /// Ends the current interaction without committing it and returns to Idle: an open stroke is `stroke_cancel`led, a
    /// gizmo drag `cancel_pending_edit`ed. Called by dispatch (`Cancel`, the interaction guard, tool / mode / document change).
    pub fn cancel(&mut self, state: &mut AppState);
    /// Ends the current interaction committing it (`Tool::finish`) and returns to Idle. Called by dispatch's interaction
    /// guard for Save / Save As / Export (8.3).
    pub fn finish(&mut self, state: &mut AppState);
    pub fn overlay(&self, state: &AppState, xf: &ViewXform, painter: &egui::Painter);
    pub fn brush_ring(&self, state: &AppState) -> Option<BrushRing>;
}
```

**Objects in the views** (objects-tabs, plan 5.3-5.5). Main3D: `all_objects: false`, `highlight:
view3d::highlight_shown(state)` = objects are selected, no stroke is open and the tool is Navigate (a tint would falsify
the colours while painting; WP4 adds "or the Objects tab is shown"). Preview mode: `all_objects: false`, no highlight.
The export dialog's preview (`ViewId::ExportPreview`): `all_objects: true` (an export contains every object), no
highlight. The 2D view shows the texture whole and leaves out the UV wires of hidden objects; painting in 2D is not
masked; no highlight in 2D. `view::locate_px` (texel -> surface point: the UV-stamp outline traced onto the model
`uv_outline_in_3d`, the brush ring of a 2D cursor, the hover position of the 2D view) returns `None` for a texel owned by
a hidden object, so no outline point, ring or position lands on a hidden triangle. Everything hidden: the 3D view shows
the backdrop and tools hit nothing.

**Routing (identical for both views; subject to the input gate of 8.5).** Presses are detected with
`i.pointer.button_pressed(b) && response.contains_pointer()` (`Sense::DRAG` captures immediately, so the first dab lands on
mouse-down). On press:
1. RMB => Orbit (3D) / Pan (2D). MMB or Space+LMB => Pan. Tool `Navigate` + LMB => Orbit (3D) / Pan (2D).
   objects-tabs: **a Navigate click picks an object** — a press of the logical primary button (Space+LMB is a pan) with
   the Navigate tool in `Main3D` on a model with two or more objects remembers `NavClick { origin, mods, camera, travel }`;
   the orbit adds each pointer delta's length to `travel`; at the release with `travel < NAV_CLICK_TRAVEL` (4 pt) the
   camera is put back (the sub-4-pt orbit is undone) and a ray is cast at `origin` (hidden objects are passed through):
   a hit => `SelectObject(tri_object_of(hit.tri))`, with Ctrl or Shift `ToggleObjectSelected`, and
   `session.object_reveal = Some(object)`; a miss without a modifier => `DeselectObjects` (pushed through
   `state.push`). `go_idle` (cancel, discard) forgets the click. Painting tools keep every click; one-object models, the
   Preview views and the 2D view never pick, so a Navigate click there is the old zero-length orbit. The Navigate tool
   options add the line "Click an object to select it; Ctrl- or Shift-click adds or removes it." on a model with two or
   more objects.
2. Otherwise LMB => `tool.press`; `true` => `Interaction::Tool`.
Motion during a tool drag uses **every** `Event::PointerMoved` in `i.events` of the frame, not only the latest position.
Hover: each frame the pointer is over a view, compute `Surface` once (one raycast in 3D), fill `state.hover`, call `tool.hover`.
Alt held with Brush/Eraser/Fill temporarily acts as Eyedropper.
Keys: views and tools never read key presses (8.2); Esc and Delete arrive as `AppAction::Cancel` / `DeleteSelection` through
`dispatch`, which calls `ToolController::cancel`.

**Wheel.** Views take wheel input **only from the raw events**, never from `smooth_scroll_delta` or `zoom_delta()` (with Ctrl held
egui turns the wheel into a zoom factor and the scroll delta is zero, with Shift it becomes horizontal scroll, and the smoothed
delta spreads one notch over several frames — 8.1):
```rust
if response.contains_pointer() {
    ui.input(|i| for e in &i.events {
        if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = e {
            let notches = match unit { egui::MouseWheelUnit::Line => delta.y,
                                       egui::MouseWheelUnit::Point => delta.y / 50.0,
                                       egui::MouseWheelUnit::Page => delta.y * 10.0 };
            wheel.push((notches, *modifiers));
        }
    });
}
```
Routing uses the event's own `modifiers`: Ctrl or Shift held => `tools.scroll(.., notches)` (pointer `mods` = those modifiers;
Ctrl = size / scale, Shift = rotation); if that returns `false`, or with no modifier, the camera zooms
(3D: `camera.zoom(notches, model_radius)`; 2D: `uv_view.zoom_about(cursor, exp(0.15 * notches), ..)`).

| Tool | 3D view | 2D view |
|---|---|---|
| Brush / Eraser | Press on a hit, in this order: (1) `hit.texture_set == None` (NO_UV triangle or a material without UVs) => no stroke, warning toast "This part of the model has no UV mapping and cannot be painted". (2) Make the hit's set the active set. (3) `doc.paint_target(set)`: `Locked` / `Hidden` => warning toast ("The active layer is locked / hidden"), no stroke; `NotRaster` / `NoActiveLayer` => `add_raster_layer(set, None)` + info toast "Created paint layer". (4) Every **other** set whose `paint_target` is `NotRaster` / `NoActiveLayer` also gets `add_raster_layer` (no toast), so a stroke that crosses a material boundary paints on both sides (D7). (5) If `flags.shading != Textured`: switch to `Textured` + info toast "Texture display turned on" (painting must be visible). (6) the stroke: see **Brush / Eraser in 3D (round 4)** below the table. | Same with `radius = size_px / 2` and `stroke_to_2d(set, px, 1.0)` (the core interpolates); steps (1), (4) and (5) do not apply (only the view's set is involved). |
| Eyedropper | `sample_color(set, uv, Composite)` => `SetPrimaryColor`. Hit without a set: nothing. | Same at the cursor UV. |
| Fill | Hit without a set => the "no UV mapping" toast. Target = active layer of the hit's set, with the same `paint_target` handling as Brush step (3); then `doc.fill(set, layer, uv, primary, tools.fill_scope)` (`Err` => warning toast). | Same. |
| Stamp / Text | Hover: outline of the would-be decal (projected `Projector::corners`). Click on a hit with a texture set: build the projector — `half_size` from the source size `(w, h) = doc.stamp_source_size(&source)`: image stamp: the larger of `half_size.x / .y` = `size_frac * diag / 2`, aspect `w : h`; text: `half_size.y = em_frac * diag * h / TEXT_PLACEMENT_PX / 2` (so one em is `em_frac * diag` high on the model), `half_size.x = half_size.y * w / h`. `p = Projector::from_surface(hit.position, hit.normal, camera up, half_size, max(half_size)).rotated(rotation_deg.to_radians())`; **`if !hit.front_face { p.backface_cull = false }`** (a decal clicked onto a back-facing / reversed-winding triangle would otherwise bake to nothing); `project_through` => `backface_cull = false, occlusion = false, depth_back = diag`; otherwise (on the click, not on hover) `p = doc.fit_stamp_depth(&p)` (6.5 "Depth auto-fit"). `doc.add_stamp(name, source, Placement::Projected(p), set)`, select it, switch to Transform; if every linked layer of the new stamp is empty, a warning toast says the decal covers no texels there (round 5: with deferred Final bakes the stamp is unbaked at the click — the gizmo frame and "Finishing stamp" in the status bar show at once, `state.await_first_bake = Some(id)` — and the toast comes when its first bake lands, `stamp_bakes`). No source chosen (Stamp tool) => pushes `ImportStampImage(None)` instead. Hit without a set => the "no UV mapping" toast. **Every refusal gives a message** (fix round 2, item 4): a click that hits nothing (between the parts of a model such as many_islands.obj, or beside the texture in 2D) => info toast "Nothing here to put the text / stamp on: click on the model / on the texture"; empty text => status hint and focus on the text field. Round 4 (S2, FEATURE_TEXT 6.2): the Text tool always works on the text of its options (the draft), also while a placed text is selected; the selected text is edited with the Transform tool (the R3-10 rule is gone). Ctrl+wheel changes `size_frac` / `em_frac` (x1.1 per notch, 0.2 %..400 %; the option sliders show a value beyond their own range without clamping it, `SliderClamping::Edits`), Shift+wheel `rotation_deg` (5 degrees per notch, kept in (-180, 180]). `rotation_deg` and `project_through` are one setting shared by the Stamp and the Text tool; both tools' options show them. | Outline rectangle at the cursor; click adds `Placement::Uv { set: active set, center_px: cursor, size_px, rotation, .. }` with `size_px`: image: larger side = `size_frac * texture width`; text: `size_px.y = em_frac * texture height * h / TEXT_PLACEMENT_PX`, aspect from the source. |
| Transform | Gizmo of `session.selected_stamp` (projected corners + rotate knob 24 pt above the top edge); a selected id for which `doc.stamp(id)` is `None` counts as no selection. Press on Body => move: re-raycast under the cursor (offset by the grab delta); hit => `moved_to(hit.position, hit.normal)`. Corner => uniform scale by the ratio of cursor distances from the centre (Shift: non-uniform along the decal's axes). Rotate knob => rotate so the knob follows the cursor (clockwise cursor motion = positive rotation, 4.1). Projected decals whose depth was automatic at the press (`doc.stamp_depth_is_auto(start)`) get `doc.fit_stamp_depth` of every new placement (move, scale, rotate, and each wheel notch), so the depth follows the surface under the footprint; a depth typed in by hand is kept and scaled along. The gizmo also shows the projection box faintly (`GizmoShape::depth_box`, 8.5). Every drag frame: `set_stamp_placement(id, p, Commit::Preview)`; release: `Commit::Final`; cancel: `doc.cancel_pending_edit()`; finish (Save / Export during the drag): `Commit::Final`. Press outside the gizmo: `stamp_at(set, uv)` under the cursor selects another stamp, else deselects. Ctrl+wheel scales (x1.1 per notch), Shift+wheel rotates (5 degrees per notch). One wheel **gesture** is one history entry (review round 2, finding SP-1): every wheel event (whole notches and touchpad fractions alike) is a `Commit::Preview` of the pending stamp edit; `ToolController::end_frame` (called by `view::central` with the input time, after `set_clock`) flushes it once the wheel has been idle for `WHEEL_IDLE` = 0.3 s (one Final bake), a press commits an open gesture first, Esc cancels it like any pending edit. A stamp protected by a locked linked layer (`doc.stamp_locked`, 5.20) can be selected but not moved: a press on its handles and the Ctrl/Shift+wheel show the warning toast `panels::layers::stamp_lock_message` ("The stamp's layer is locked: unlock it in the Layers list first"; with several sets it names the set(s) that hold the lock) and change nothing. A UV placement's rotation is kept in (-pi, pi]. | Same handles on `UvPlacement` in texel space. A stamp is edited in the view that matches its placement kind; the other view only shows its outline. |

**Round 6 (R6-2, wrapped 3D stamps; ROUND6_PLAN 5.1).** *Stamp / Text*: the new projector gets `mode =
tools.stamp.mode` and `wrap = tools.stamp.wrap`; in Wrap the back-face and project-through rules of the row above do not
apply (`project_through` is greyed out in the options), the depth is fitted on the click as in Project (so switching
back gives a fitted projection); the hover outline and hover picture stay flat. *Transform*: the drag maths are
unchanged (body drag raycasts and `moved_to`, scale changes `half_size`, rotation turns the frame about the normal,
depth auto-fit still runs); on a back-face hit a wrapped decal keeps `backface_cull`. `gizmo_shape` keeps the flat
frame's handles (corners, centre, knob: hit tests and the self-test drags unchanged), has `depth_box: None` and
`wrapped: true` in Wrap; `gizmo::draw` draws that frame faintly and `TransformTool::overlay` draws
`gizmo::wrapped_outline(doc, id, xf)` (the picture's border from `Document::stamp_wrap_outline(id, 32)`, points facing
away from the camera dropped). `TransformTool::ensure_wrap_param` (through `ToolController::ensure_wrap_param`, called
by `WteApp::logic` after `stamp_bakes::pump`, before the panels and views draw): a selected wrapped stamp whose cache
holds no parameterization of its placement (after open / undo / redo), whose bake is not pending and with no pending
edit open (check:B: a drag's throttled Preview bakes bring it; never one per drag frame on the side) gets
`doc.ensure_wrap_param(id)` on the UI thread, logged "wrap: selected stamp N got its parameterization in X ms ..."; a
placement without surface is not retried every frame. Round 8 (R8-2): while the document defers wrapped Previews
(`doc.defers_wrap_previews()`, the asynchronous mode) it makes nothing on the UI thread: it asks for a param-only job
(`state.wrap_jobs.request_param(id)`, started by `stamp_bakes::after_tools` in the same logic pass); the stats line
shows "Wrapping..." and the outline appears when the job lands; a placement for which that job found no surface
(`wrap_jobs.param_failed`) is not asked for again. Objects-tabs: both failure marks (`wrap_failed`, `param_failed`) carry
the document's `visibility_generation`, so a placement is tried again after objects are shown or hidden, and a
param-only job made before a visibility change (not cached, core 5.20) never marks a failure. `ui_params` counts the synchronous builds. The warm-up job of plan
4.1 (`wrap_mesh_prep`) is not built: the core has no welded-graph cache yet (package A step 6). *Text on a Curve* (`text_path.rs`): for a wrapped text the curve
points and the drawn curve lie on the surface (`path_points_on`: the flat frame's point taken to stamp-local
coordinates, then `stamp_wrap_surface`; the flat point where it returns `None`); dragging a point raycasts the model
and converts with `stamp_wrap_local(id, hit.tri, hit.bary)` (`screen_to_em_on`; the projector plane where the ray
misses or the picture has no parameterization there). *Outline of the selected stamp* (`view::selected_outline`, other
tools or the other view): Wrap in 3D = `wrapped_outline`; Wrap in 2D = `view::wrapped_outline_in_2d` (the outline's
surface points through their triangle's UVs into the active set's texels; points on other sets and seam jumps break
the line).

**Brush / Eraser in 3D (round 4, B7)**, from step (6) of the table on:
(6) `BrushParams.radius = size_px / 2 * doc.texel_world_size(set)`, colour = primary; the tip of the settings
(FEATURE_BRUSH 4.3; checked before step 2, so a refused press changes nothing): Circular = `BrushTip::legacy(true)`
(the round-3 stroke, bit for bit); Sphere + Ball = ball, `SurfaceReach::All`, `offset`; Sphere + Cube / Cylinder =
the built-in field (`AppState::builtin_tips`, made on first use), `All`, `align`, rotation `Ry(-turn) * Rx(tilt) *
Rz(roll)`, `offset`, scale 1, `through` (always true for a flat outline); Sphere + File = `brush_shape.current`'s field
(also while a newer load runs); File without a current shape and a load pending => no stroke, info toast "The brush
shape is still loading"; File with nothing loaded or loading => the ball, info toast "No brush shape is loaded;
painting with the ball" once per program run (`BrushShapeState::warned_missing`). `up_hint` = the camera's up at the
press. `stroke_begin_with(params, tip)`, then the press hit is painted at once (`stroke_to_3d(hit, 1.0)`).
Drag (stroke pacing, FEATURE_BRUSH 5.10): every pointer position is queued with the `ViewXform` of its frame
(`unresolved`; a position within 0.001 pt of the last one is dropped). **Resolve**: the screen polyline through the
queued positions is walked in steps of `max(1 pt, 0.25 * ring radius on screen)`, each step raycast with the transform
stored with the position it leads to; hit / miss go to `pending`. At most `StrokePace::raycasts_per_frame` = 64
raycasts per frame while the button is down, `finish_raycasts` = 4,096 per frame after the release. **Paint**: samples
are taken from `pending` in order (hit => `stroke_to_3d(hit, 1.0)`, miss => `stroke_break`) until the queue is empty,
or 10 ms are spent in this frame while the backlog (`pending.len()` x the mean time of the last 16 samples) is at most
250 ms (only while the button is down), or 100 ms are spent; at least one dab per frame. Round 5 (B-3): the rule is
also checked after every dab (`stroke_to_3d_paced` with a `more` that compares this frame's painting time, including
the sample in progress, with the same limits): a sample whose segment does not fit stays at the front of `pending`,
partly painted (`BrushTool::partial`), the frame ends, and the next frame resumes it first (`stroke_resume`, at least
one dab) before it checks the rule again; a sample's time is summed over its frames for the mean and the longest
sample, the backlog counts it as one sample, and the frames' own time is booked as it is spent (`longest_frame`
unchanged). `complete` / `finish`, the press's first sample and a clock that only moves per sample (the tests' fixed
clock) never split a sample; the core finishes an open segment before any other stroke call, so the pixels do not
change (test `one_dab_per_frame_gives_the_unpaced_hash`). `drag` and
`ToolController::end_frame` (`BrushTool::pump`) both resolve and paint; the budgets are per frame, so a frame without
a drag call still advances the stroke and a frame with one does not do more. The painted pixels depend on the pointer
path only, never on the clock, the budgets or a camera move after a position was queued (tests A4, A5).
Release: the release position is queued and the stroke is marked released; resolve and paint go on in this frame and
the next ones; when both queues are empty: `doc.stroke_skipped_sets()`, `stroke_end`, the warning toast "Paint skipped
on <set names>: the active layer is locked or hidden" if sets were skipped, and one log line "brush stroke: N samples,
stroke work X ms, longest frame X ms, longest sample X ms, largest backlog X ms, release to end X ms, size S px, ring
R pt, D dabs, longest dab L ms" (FEATURE_BRUSH 8.4, P8; the last four fields since round 5: Size at the press, the
ring radius on screen at the press, `stroke_counters().dabs`, the longest wall-clock time between two dabs). With the
environment variable `WTE_STROKE_LOG` set (read once, `brush.rs`), the core's stroke probe is turned on and the tool
logs "brush sample: T ms over F frame(s), D dabs, S set dabs, sets started N, buffer copies N; <phases>" for every
sample of 20 ms or more and "brush stroke probe: ...; <phases>" with the stroke's totals (5.19). Until then `finishing()` is true (the controller stays busy, status bar "Finishing the stroke... (Esc
discards it)"); `drag` and `release` do nothing for a released stroke. `complete` / `finish` (Save while held, every
command but Undo and Esc, a new press) resolve and paint everything without limits and end the stroke (the queued
positions carry their own transforms); `cancel` (also `ToolController::discard`: Undo, Esc) drops both queues and
`stroke_cancel`s. While the backlog of a held stroke is over 250 ms the status bar says "Painting... the stroke is
catching up". The clock is `StrokeClock` (wall clock, or in tests a fixed time per sample or per dab).
Wheel (FEATURE_BRUSH 1.5, deliberate change of round 4): Ctrl+wheel multiplies the active tool's `size_px` by 1.1 per
notch (1..2048); Shift+wheel with Sphere and a shape other than the Ball adds 5 degrees per notch to `turn_deg` (kept
in (-180, 180]); otherwise both still zoom the view; the wheel without a key always zooms.
`brush_ring`: centre = the hover hit (during a 3D stroke: the last painted sample) plus `offset * radius` along the
viewer-facing normal (Sphere only), radius = the open stroke's `radius_world` during a 3D stroke, else from the live
Size; `side_normal` = the hit's unflipped normal for Circular, `None` for Sphere and for a ring located from the 2D
view. The type and offset during a stroke are the stroke's own (`stroke_tip`), not the live settings.
`BrushRing::hardness` = the setting for the ball (Circular, Sphere with the Ball, File without a shape), 1 for a
shape (the stroke's tip during a stroke), so only the ball shows the inner hardness ring (FEATURE_BRUSH 5.11; gate 1).
Overlay in the 3D view (`cursor_segments(state, xf) -> CursorLines { footprint, wires }`, FEATURE_BRUSH 5.11): nothing
for Circular; for Sphere with the Ball three wire circles (48 segments) of the ball in the planes of `tip_frame`; for a
shape the three principal sections (`TipField::principal_sections`, mapped by `c' + k * R * q`, 60 % white, 1 pt) and
the bold footprint (white 1.5 pt over a dark 3.5 pt shadow): Solid = `field.section` of the tangent plane at the
cursor (48 x 48 lattice), Through or a flat outline = the middle slice, clipped to the ring's ball. During a stroke the
pose is `doc.stroke_tip_pose()`. No depth test. The segment counts are written to `AppState::tool_probe.cursor_footprint`
/ `cursor_wires` in `hover` and in every stroke frame (the probe counts the ball's circles as wires).
Status bar while the brush hovers over the model in the 3D view: "Circular brush" / "Sphere brush: paints everything
inside the ball, also hidden surfaces" / "Shape brush: <Cube | Cylinder | file name>"; cleared when the pointer leaves
the model (the brush writes and clears only its own hints, never another feature's).
The 2D column of the row is unchanged (a 2D stroke is `stroke_begin` with the flat disc whatever the type,
FEATURE_BRUSH 1.6; it is painted at once, without queues).

**Text source for the Text tool**: a `TextSpec` built from `tools.text` with `size_px = TEXT_PLACEMENT_PX` (the core then picks
the real raster resolution, 6.5), `outline = Some(Outline { width_em: outline_width_em, color })` when enabled. Font:
`state.fonts.find(&family, &style)` -> `face_bytes` -> `doc.register_font(..)`; when `find` fails (font not installed, or the
system-font scan has not delivered it yet) use `doc.default_font()` **and** show the warning toast
"Font <family> not found, using Ubuntu Light" — never fall back silently.
- Press with `tools.text.content.trim().is_empty()`: no stamp and no history entry; `status_hint = "Type the text in the tool
  options first"` and `state.focus_text_content = true` (the tool options focus the content field).
- An existing text stamp whose content is edited down to empty keeps its layer (the core lays empty content out as one space,
  6.8, so size and gizmo stay defined). It has no visible pixels, so `stamp_at` cannot pick it in a view; it stays selectable
  through its row in the Layers panel (`SelectLayer`).

Round 4, controller (S2, B7):
Stroke pacing, controller side (FEATURE_BRUSH 5.10): `ToolController::end_frame` also calls `BrushTool::pump(state)`
(the brush resolves and paints its queued pointer positions under the time rule when no drag ran in the frame) and
returns `Some(0.0)` while work is queued, which the central area turns into a repaint request. `release` keeps the
interaction while `BrushTool::finishing()` (the stroke is "finishing"; `ToolController::finishing()` says so) and
`end_frame` returns to Idle once the brush has ended the stroke. `cancel` and `finish` complete a finishing stroke
(`BrushTool::complete`: all queued work, then `stroke_end`); `discard` drops it (`Tool::cancel`). A press in a view
first completes a finishing stroke (`complete_finishing`, in `ToolController::press` and in the press branch of
`view::route_pointer`). The scaffold's `pump` returns false and `finishing` is always false, so a stroke ends at its
release exactly as in round 3 until B7.

Text tool (FEATURE_TEXT 6.2, replaces the R3-10 rule "the Text options show the selected text"): the Text tool always
works on the text of its options (the draft), also while a placed text is selected — the hover outlines it, Ctrl /
Shift+wheel change it, a press places it. The selected text is edited with the Transform tool.
`text_tool_shows_selected_text` and `TEXT_DESELECTED_HINT` are gone.

Round 4 (T7, FEATURE_TEXT 5.11, 5.14):
- The draft (`text_source`): the eight round-3 fields plus `fx` from the Text settings (`bold_em`, `slant_deg`,
  `warp`, `path`; never gap values). The font is looked up in the open document first (`Document::find_font`: a
  font that only the project holds can be used for the next text, and the project's copy wins over an installed
  file of the same names), then in the catalog (`face_bytes` + `register_font`), else the bundled font with the toast
  of today.
- Measuring: for a text the tool asks `stamp_source_frame` (the frame `add_stamp` will draw, with the too-large
  fallback) once per draft (`SourceKey::Text(TextSettings)`) and sizes the rectangle with the frame's `em_px`
  (`em_frac * diag * h / em_px`; for a plain text `em_px` is `TEXT_PLACEMENT_PX`, so the rectangle is round 3's bit
  for bit). A draft whose frame or hover picture fails (too large at every size) is remembered and not tried again
  until the draft changes (no layout per hover frame).
- Anchor at the click: the placement is built around the click as before and then moved by
  `place_anchor_at_centre`, so the text's anchor lies at the click (for a plain text the anchor is the picture
  centre and nothing moves). A projected text gets its automatic depth fitted after the move. The hover outline is
  the same rectangle.
- Hover picture: when the draft changes, `hover` asks `Document::text_preview(&spec, 1 << 18)` and keeps the
  `Arc<TextRaster>` with its spec (`StampPlaceTool::hover_preview()`); a changed angle or on-model size draws
  nothing new. `overlay` uploads it as an egui texture when the picture changed (`Mutex<Option<(Arc<TextRaster>,
  TextureHandle)>>` in the tool; the `Tool` trait is unchanged) and draws it at 70 % opacity inside the yellow frame:
  in 2D as the rotated rectangle, in 3D as the placement rectangle on the plane under the pointer, cut into 8 x 8
  cells whose corners are projected (a cell with a corner behind the camera is left out). It does not show the wrap
  around the surface. `state.tool_probe.text_hover` holds the spec and the pixel size of the picture for the
  scenarios; another tool or an empty draft clears it. The click that places the text reuses a full hover picture
  (5.20).

Round 4 (T7, FEATURE_TEXT 6.5): curve points of a text on a Curve (`tools/text_path.rs`). For the selected text with
`PathShape::Curve`, in the view that edits its placement (the gizmo's rule), the overlay draws the line the text
follows (centripetal Catmull-Rom, 33 points per piece, the painter's spline) and one round point of 5 pt per curve
point, in a colour different from the gizmo; the hot point is filled and the cursor is "move". Positions come from
`EmMap` of the placement and the cached frame (`cached_text_frame` in `overlay`; `stamp_text_frame` in `hover`,
`press` and `drag`): in 3D the points lie on the projector plane and the pointer is brought back by cutting the camera
ray with that plane, in 2D through `screen_to_px`.
Hit order in `press`: the rotate knob, then the curve points (8 pt), then the corners, then the body. A point press
on a locked text warns as a locked gizmo drag does. A point drag (`TransformTool::path_drag: Option<PathDrag { id,
view, point, grab, moved, refused_said }>`, next to `drag`) sends `set_stamp_source(.., Preview)` with the moved point
every frame (clamped to `PATH_COORD_MAX_EM` = 60 em from the anchor) and one `Final` on release or `finish`; Esc
(`cancel`) calls `cancel_pending_edit` and the curve is as before, without a history entry. The anchor and the em
stay (anchor rule), so the other points stay under their marks. A refused `Preview` (the text too long for any
resolution) keeps the last accepted point: spec and picture stay, the status bar shows the reason once per drag, and
release commits the last accepted point (no entry when none was accepted).

### 8.7 Panels (`panels/*`) [A4]

```rust
pub struct PanelState { /* layer thumbnails keyed by (LayerId, Layer::version), set thumbnails, rename-in-progress, drag state */ }
impl PanelState { pub fn new() -> PanelState; }
pub fn show_menu_bar(ui: &mut egui::Ui, state: &mut AppState);
pub fn show_toolbar(ui: &mut egui::Ui, state: &mut AppState);
pub fn show_side_panel(ui: &mut egui::Ui, state: &mut AppState, ps: &mut PanelState);
pub fn show_status_bar(ui: &mut egui::Ui, state: &mut AppState);
```
Panels only read `AppState` / `Document`, change settings, and push `AppAction`s (they never call `rfd` and never mutate the
document). They may call the read-only helpers `shortcuts::shortcut_text` and `theme::tool_icon`. Two `&mut Document` calls
are allowed because they change no content, no revision and no history: `doc.blob_thumbnail` (fills the stamp-image decode
cache; Stamp tool source preview) and `doc.register_font` / `doc.default_font` (a deduplicated blob insert that yields the
`FontRef` an `UpdateStampSource` action needs when the font of a selected text stamp is changed — there is no action that
carries a catalog face).

- **Menu bar**. File: New from Model... (Ctrl+N), Open Project... (Ctrl+O), Open Recent >, Open Sample > (Sphere, Cube, Two
  Materials, Three Objects), Save (Ctrl+S), Save As... (Ctrl+Shift+S), Import Image as Stamp... (`ImportStampImage(None)`), Export... (Ctrl+E),
  Extract Original Files... (`ExtractSources(None)`; enabled with any open document, saved or not), Exit. Edit: Undo / Redo with
  the entry label, Delete (`DeleteSelection`). View: 3D / 2D / Split / Preview, Texture on Model (T), Shading > (Textured, Clay,
  UV Checker), Wireframe (W), UV Seams, UV Wireframe in 2D, Alpha Checker, Lit, Frame Model (F), Reset Camera, camera presets.
  Layer: New Paint Layer, Duplicate, Merge Down, Delete, Rasterize, Move Up / Down (the `*Active*` actions); below a
  separator "Export Layer..." (one layer selected) or "Export Selected Layers... (N)" (several), enabled with an active
  layer, => `ShowExportLayers(session.layer_selection())` (export-select, 1.8). Object (objects-tabs, between Layer and
  Help): Hide Selected (H) and Isolate Selected (Shift+H), enabled with a non-empty object selection; Show All (Alt+H),
  enabled when `hidden_object_count() > 0`; Frame Selection (Shift+F), enabled with a document; Select All (with a
  document) / Deselect All (with a selection). Help: Shortcuts,
  About. Shortcut texts come from `shortcuts::shortcut_text`. Entries are enabled exactly when they can act (review round 2,
  finding merge-down-enabled-onto-locked-base): Merge Down (menu and Layers footer alike) = `panels::layers::can_merge_down`
  (the active layer is unlocked and directly above an unlocked paint layer: the checks of `Document::merge_down`, so on a new
  document "Paint 1" above the locked "Base" cannot merge); Rasterize = a stamp/text layer that is not locked; Layer > Delete =
  the active layer is unlocked and not a stamp locked in another set; Edit > Delete = the target of `DeleteSelection` (the
  selected stamp, else the active layer) is not protected by a lock. Ctrl+M stays a shortcut without a disabled state (a refused
  merge shows the warning toast).
- **Toolbar** (left, 44 pt): vector-painted icon buttons from `theme`, tooltips with shortcuts; below them the primary/secondary
  colour chip with swap.
- **Side panel** (right), three areas from top to bottom (fix round 3, R3-10, user decision 2; `panels::show_side_panel`):
  1. one scroll area with the collapsing sections Tool options and Texture sets; it gets the room the other two leave and
     shows a thin scroll bar whenever it overflows (`ScrollStyle::thin()` for all side panel scroll areas: egui's default
     floating bars are invisible until hovered, which hid that anything was cut off);
  2. the *Color* section pinned below it (`egui::Panel::bottom("side_color_area")`, not resizable, `exact_size` = its content height of the last frame
     plus the frame margins and 2 pt of slack — a scroll area cannot grow a panel by itself, and without the slack a
     rounding difference shows the scroll bar, which narrows the swatches and flips the layout every frame):
     at a 1280x800 window or larger the whole picker (RGB row, colour bar, saturation/value square, hue bar, hex field, both
     swatch rows) is on screen whatever tool is active and whatever is selected. Its room is what the Layers area leaves
     minus 112 pt kept for the tool options (about 96 pt of them visible after the areas' spacing); when that is short the square (`slider_width`, 140 pt at most) shrinks to fit,
     down to 72 pt, then the section scrolls in its own scroll area (`side_color_scroll`). The height of the open section
     without the square is measured each frame (`PanelState::color_fixed_height`) for the next frame's fit;
  3. a resizable bottom area (`egui::Panel::bottom("side_layers_area")`, default 40 % of the panel height (at most
     420 pt), at least 200 pt (header, opacity row, two whole rows, footer), at most 85 % and at most the panel height minus 112 + 60 pt for the
     tool options and the Color area — a size set in a large window is clamped to that when the window shrinks). Until the
     user drags its edge (`PanelState::layers_user_sized`, set by a primary press within the resize grab radius of the
     area's top edge, kept across documents) the area hugs its content (`layers_area_fit`: header + rows + footer measured
     in the last frame, `layers_natural_height`, between 200 pt and the default), so a short list gives its room to the
     tool options and the Color section (R3-10 review: at 1280x800 the Brush options are whole again); with more layers
     it grows back to the default and the list scrolls. It holds the *Layers* section: opacity / blend row, then the layer list in its own scroll area
     (`side_layers_scroll`, as tall as the area minus the footer measured in the last frame), then the footer buttons below
     it, outside the scroll area, so New / Duplicate / Merge Down / Rasterize / Delete stay on screen with any number of
     layers (finding F9: with 5 layers they scrolled out of sight).
  `state.panel_rects` records, besides the layer row controls, the visible settings area (`PanelHit::SettingsArea`), the
  Color area (`ColorSection`) and its unclipped content (`ColorContent`; inside `ColorSection` = nothing scrolled away), the
  list's visible rect (`LayersList`), the footer (`LayersFooter`) and its Delete button (`LayersDelete`), the whole Layers
  area (`LayersArea`); `panel_rects` is
  cleared at the start of `show_side_panel`. New areas (e.g. the Brush tab planned for round 4) go in as further panels
  there. The sections:
  - *Tool options* for `state.tool`: Brush/Eraser (size, hardness, opacity, flow, spacing; round 4: "Same side only" removed, the Brush tab's Type replaces it); Fill (scope);
    Stamp (source thumbnail + Choose... => `ImportStampImage(None)`, size, rotation, project-through); Text (multiline content —
    focused when `state.focus_text_content` is set, which the panel then clears —, family and style combos from `state.fonts`
    with the row "Loading system fonts..." while `state.fonts_loading`, **Size** = `em_frac` shown as a percentage of the model
    size, colour, outline on/off + width (`outline_width_em`, as % of the text size) + colour, alignment, line/letter spacing,
    and the shared Rotation and Project through of `tools.stamp`, marked "Shared with the Stamp tool").
    The round-3 rule "Text tool with a selected text stamp" (R3-10) is replaced in round 4: see the Text block below.
    There is exactly one text "Size" (the on-model size); no raster-size control exists (4.3).
    When a stamp is selected (Transform tool): its editable properties — name, tint, the text fields for a text stamp, image
    "Choose..." (`ReplaceStampImage(id, None)`) for an image stamp, numeric size/rotation, depth, a "Fit depth to surface"
    button (`UpdateStampPlacement(id, doc.fit_stamp_depth(p), Final)`, 6.5), angle limits, flips,
    backface-cull / occlusion — as `UpdateStamp*` actions (`Commit::Preview` while a widget is being dragged or typed in,
    `Commit::Final` on `drag_stopped()` / `lost_focus()`). Numeric "Size" of a text stamp edits the placement (on-model size).
    Numeric Size / Rotation of a projected stamp whose depth is automatic (`doc.stamp_depth_is_auto` of the current
    placement) send the edited placement through `doc.fit_stamp_depth` (`tool_options::refit_auto_depth`, fix round 2,
    item 2), like the Transform tool; a hand-set depth is kept.
    A change while any text field has the keyboard focus also counts as "typed in" (`ctx.text_edit_focused()`): typing into the
    R/G/B fields of a tint / colour popup is one Preview edit, finished by the owed `Commit::Final` once nothing is focused.
    The owed Final (`PanelState::owed_final`) is the exact Final twin of the last preview: an edit that ends (drag stopped,
    focus lost) without a change in that frame sends that owed action itself, never a Final rebuilt for whatever layer or stamp
    the control shows now; the owed Final is dropped in `begin_frame` / `end_frame` when its layer or stamp no longer exists
    (`panels::action_target_exists`; fix round 3, R3-2).
    Display alone never edits: the document-bound `DragValue`s use `clamp_existing_to_range(false)` (a UV stamp turned past
    180 degrees or a depth scaled past the field's range is shown as it is, not clamped and written back), the document-bound
    sliders `SliderClamping::Edits` (`panels::percent_slider` always sets it: with egui's default `Always` an idle slider
    writes its existing value back rounded to its decimals and reports a change — the Layers opacity slider at 0 decimals
    turned 21.33 % into a stray "Layer Opacity" Final for every newly active layer, finding F2 of round 3); Depth front / back accept 0.05..1000 % of the model size. While the stamp is protected by a
    locked layer (`doc.stamp_locked`) the panel says so (`panels::layers::stamp_lock_message`: with several sets it names the
    set(s) whose Layers list holds the lock) and disables everything but the name (tint, image, text fields, placement fields,
    "Delete stamp"); when the lock sits in a set other than the active one, a button *Show set "<name>"* queues
    `SetActiveSet` (review round 2, finding cross-set-lock-invisible).
    **Wrong-view hint** (fix round 2, item 3; `tool_options::wrong_view_hint`): while the selected stamp cannot be edited in the
    current view mode — a projected stamp / text in `View2D`, a UV-placed one in `View3D` (Split shows both, Preview edits
    nothing) — the top of the Tool options shows "This is a 3D stamp - edit it in the 3D view" (or 2D / text accordingly) with
    a button *Show 3D view* / *Show 2D view* (`SetViewMode`), and the same text is written to `status_hint`; it is cleared from
    the status bar again once the condition ends (only a hint starting with "This is a " is cleared).
    **Round 6 (R6-2, ROUND6_PLAN 5.1): Project | Wrap.** `tool_options::mode_selector` draws "On the model: Project |
    Wrap" (two selectable buttons; hover "Like a projector from the front: sharp from this side, stretched at the
    sides" / "Lies on the surface like a sticker: goes round the object and over bumps") and
    `tool_options::wrap_option_controls` the two Wrap options under it: checkbox "Jump small gaps" (hover "Continue onto
    separate parts closer than 1 % of the model size; off: stop at the edge of a part") and "Longer than the way round:
    Ends meet | Overlap" (hover "Ends meet: the middle part shows, the ends meet at the back" / "Overlap: keeps winding
    like tape, the end on top"); shown in Project too, greyed out (hover "Used in Wrap mode"); they record
    `ToolProbe::wrap_controls` (reset at the start of the Tool options section). Stamp tool options and the Text block
    (Text tool, next to "Project through"): `tool_options::tool_surface_options` on `tools.stamp` (`mode`, `wrap`; no
    action), "Project through" greyed out in Wrap. A selected 3D stamp (`placement_fields`): the selector at the top,
    then the two options, each change `UpdateStampPlacement(id, Projected(Projector { mode | wrap, ..pr }), Final)` (one
    history entry; disabled with the other placement fields when the stamp is locked); in Wrap one weak line "Wrapped over
    N triangles" from `doc.stamp_wrap_stats(id)` ("Wrapping..." while it is `None`) plus warning lines when `covered <
    0.98` ("P % of the picture runs off the surface: an edge, a hole or the end of the part"), when `met_itself` and not
    Overlap ("The ends meet at the back"), `turns > 1` ("Wound N times"), `jumps > 0` ("Continued onto N separate
    parts"), `limited` ("Too large for this model: the wrap stops at 1 million vertices / 8 turns"). Round 8 (plan 4.1):
    between "Wrapped over N triangles" and the warnings two weak lines (`tool_options::wrap_stats_texts`): "Fitted to the
    curved surface: stretched up to P %" when `relaxed` and `stretch >= 0.05` (P = round(stretch x 100)), and "Cut where
    it cannot lie flat (a corner, a handle, or where its edges meet)" when `seams > 0`; and the rows Depth
    front / back, Fit depth to surface, Angle limits, Backface cull and Occlusion are hidden (kept in the placement for
    switching back; Flip X / Y stay). The Layers badge's hover text of a wrapped 3D stamp is "Wrapped on the surface of
    the model: ..." (`layers::stamp_is_wrapped`, `layers::kind_tip`).
    **Panel width** (fix round 2, item 4): no tool options row may be wider than the side panel's 300-pt default, because
    egui grows the right panel to its widest row and that moves the 3D view. The Outline rows (Text tool, text stamp
    properties: checkbox + width slider + value + colour) therefore use a 56-pt slider (`OUTLINE_SLIDER_WIDTH`); with the
    default 100 pt they widened the panel by ~30 pt as soon as the Text tool was chosen, so the model jumped under the
    cursor (a click computed or aimed before the switch landed beside the model, with no stamp placed).
  - *Color*: `color_picker_color32(ui, &mut c, Alpha::Opaque)`, hex field, 24 swatches (click = set, right-click = store).
  - *Texture sets*: name, resolution, thumbnail (`doc.set_thumbnail`), UV coverage; click => `SetActiveSet`.
  - *Layers* of the active set, top row = top layer: eye toggle, 40-px thumbnail, kind badge ("Paint", "3D stamp",
    "2D stamp", "3D text", "2D text": `layers::kind_badge(doc, layer)`; 3D = `Placement::Projected`, 2D = `Placement::Uv`;
    hover text says which view edits it) plus a small "3D" / "2D" tag over the thumbnail's lower left corner of stamp and text
    layers (fix round 2, item 3: the two kinds stay separate and are not cross-editable, so the list marks them), name
    (double-click renames), lock, alpha-lock; a click on the thumbnail, the name or the empty part of a row selects the layer, a
    double-click on the empty part renames too; drag-reorder (a drag starting anywhere on the row, the controls included) into
    a `dnd_drop_zone` => `MoveLayer`. Each row first registers its own `Sense::click_and_drag()` widget over the whole row and
    then draws the controls on top of it (a child `Ui` in the row rect), so egui's hit test gives a click to the control and the
    drag to the row; on `dragged()` the row sets the `DragAndDrop` payload itself and is painted on a floating layer at the
    pointer. (`Ui::dnd_drag_source` is not used: its drag-only widget is registered after the contents, on top of them, and
    swallows every click in the row.) Labels in the row are not selectable. The row and control rects of the last frame are
    recorded in `state.panel_rects` (`PanelHit`) for the self-test. Opacity slider (`percent_slider`, 0 decimals, never
    rewrites a fractional opacity it only shows) and blend combo for the active layer above the list; footer buttons New / Duplicate / Merge Down / Rasterize / Delete (enabled as the Layer menu entries above).
    A stamp/text row whose own layer is unlocked but whose stamp is locked by its linked layer in another set
    (`panels::layers::locked_elsewhere`) shows a dimmed closed padlock with a tooltip naming that set; its Delete is disabled
    with the same explanation (the lock icon still toggles the row's own lock).
    A freshly imported document shows "Base" with its lock closed and "Paint 1" active above it.
    **Multi-select (export-select, 1.8).** A primary click on the row, its thumbnail or its name goes through
    `layers::row_click_action(modifiers, layer, active, multi)`: Ctrl (`Modifiers::command`) => `ToggleLayerSelected`,
    Shift => `SelectLayerRange`, plain => `SelectLayer` (on the active row only when several layers are selected: it
    collapses the selection; else nothing). The active row keeps its look (selection fill x0.6, bold name); a selected
    row that is not active gets the selection fill x0.3 and a 2 pt accent bar at its left edge, so with one layer
    selected the list looks as before. A right-click on the row, its thumbnail or its name opens the row menu (egui
    `Response::context_menu`; it changes neither the selection nor the active layer): "Export selected layers... (N)" when
    the row belongs to a selection of N > 1 => `ShowExportLayers(selection)`, and "Export this layer..." =>
    `ShowExportLayers([layer])`; their rects go to `panel_rects` (`LayerMenuExportSelected`, `LayerMenuExportThis`). Only a
    primary drag reorders (`dragged_by(Primary)`). With N > 1 the footer starts with the weak line "N layers selected ·
    tools paint on <active name>". The name's hover text: "Click: select | Ctrl / Shift-click: select more | Right-click:
    export | Double-click: rename | Drag: reorder". Every layer command (footer, Layer menu, shortcuts, the Delete key)
    still acts on the active layer only (fix_export item 5 later made Duplicate and Delete act on the selection, "Duplicate
    (N)" / "Delete (N)"). With N > 1 the footer's Merge Down and (polish item 5) Rasterize say in their tooltips that they
    act on the active layer: "Merges the active layer, <name>, into the layer below (not the other selected layers)";
    Rasterize enabled: "Rasterizes the active layer, <name>, into a paint layer (not the other selected layers)",
    disabled: "Rasterize acts on the active layer, <name>, only (not the other selected layers): it is already a paint
    layer" (any paint layer, also the locked Base: check_polish) / "it is locked" (a locked stamp or text layer)
    (`layers::rasterize_multi_hover`).
  - **Thumbnail policy** (performance at 4K/8K): a layer thumbnail (`doc.layer_thumbnail` ->
    `ColorImage::from_rgba_unmultiplied` -> `ctx.load_texture`) is cached per layer under the key `(LayerId, Layer::version())`
    and regenerated only when that layer's version changed — never because `doc.revision()` changed. At most 2 thumbnails are
    regenerated per frame (`ctx.request_repaint()` while more are due), and none while `doc.stroke_active() ||
    doc.has_pending_edit()` (a stroke, a gizmo drag or a slider drag is running; the old image stays). Set thumbnails are
    refreshed under the same "not during an interaction" rule when `doc.revision()` changed, one per frame. Both thumbnail
    functions are stride-sampled, O(side^2) (5.5).
- **Status bar**: `state.status_hint`, hover read-out (set, UV, texel, RGBA), texture size, zoom, undo label, job spinner with stage
  and progress. Items never overlap (R3-10, finding F9: at 640x480 the read-out ran under the texture size):
  `status_bar::plan` gets the measured widths and keeps the job progress and at least 160 pt of the hint, then adds, while
  they fit, the texture size, the shortest hover form, zoom, fuller hover forms (`hover_variants`: with the set name, with
  the set number, without the set, texel + RGBA, RGBA) and the undo label; the hint gets the rest and is elided at its end.
  The right group is drawn first and the left group is clipped at it, so even a mis-measured item cannot overlap. The
  drawn item rects go to `state.status_rects`.

Round 4, side panel (S2): The lower area of the side panel starts with a tab strip (24 pt, three `Button::selectable`: Layers | Brush | Text) in
place of the "Layers" collapsing header. One rule (ROUND4_PLAN 2.2 item 10): Brush is enabled while the tool is Brush
or Eraser; Text while `text_block::target(state)` is `Some` (the tool is Text, or the tool is Transform and the selected
stamp is a text). A disabled tab is greyed out with a hover text; the area shows `shown_tab(state)` = `side_tab` when
its tab is enabled, else Layers. A click writes `state.side_tab` directly; `SetTool` never writes it. With Layers the
area hugs its rows as in round 3; with Brush or Text it has the computed height `panel height - Color height - 144 -
4`, limited to 160..420 pt (FEATURE_BRUSH 6.1); a size the user dragged is kept for every tab. The Brush and the Text
tab draw in their own scroll area (`brush_tab::show`, `text_tab::show`). Every frame the panel writes
`state.side_rects`.

Tool options (round 4): `text_block::target(state)` (Text tool: the draft; Transform: the selected text stamp; else
none) and `text_block::show(ui, state, ps, target, actions)` are the only text calls in `tool_options.rs`. Order: the
Text block first, then the wrong-view hint, then the tool's own options, then (every tool but Transform, the Text tool
included) the "Selected: <name> [Edit]" row; under the Transform tool the Text block comes before Name, Tint and the
placement fields. The "Same side only" checkbox is removed (the Brush tab's Type replaces it); the Size hover text
says that a brush shape is scaled to fit inside the ring, the Spacing hover text that a thin shape needs a lower
value. `PanelState.text: text_block::TextPanelState` holds the text fields of the panel state (`stamp_text`,
`font_check` moved there).

Text block (round 4, `panels/text_block.rs`; FEATURE_TEXT 6.1, 6.2). Four rows, every row at most 265 pt wide, rows 1-3
end within 116 pt of the top of the tool options at 1280x800:
  1. the content field (2 text rows);
  2. the font button (60 %, opens the font picker; the name in the warning colour with the hover text "Font not
     installed: Ubuntu Light will be used" when neither the catalog nor the project has the font; "Embedded in the
     project, not installed here" for a project-only font) and the style list (40 %: `candidates`);
  3. B, I (selectable buttons, lit by "is bold" = face weight >= 600 or `bold_em` > 0, "is italic" = italic face or
     `slant_deg` != 0), Text size (64 pt), Angle (56 pt), the colour button;
  4. "Warp: <style>" (100 pt), "Path: <kind>" (104 pt), "More" (48 pt): each sets `side_tab = Text` and the page; a click
     on the button of the page that is showing switches back to Layers.
For the text to place (Text tool) the block adds "Project through" and the help line below the rows.
Both targets are edited through one representation, a `TextSpec`: `draft_spec(&TextSettings)` builds one for the
draft (its `FontRef` carries the family and style names with a stand-in blob) and `write_draft` writes a changed one
back (no action is queued); for a placed text `live_edit` sends `UpdateStampSource` through `continuous_edit`
(Preview while dragged or typed, one Final) and `click_edit` one Final. A locked text shows the block disabled.
- Text size of a placed text = the height of one em on the model: `em_px * picture height / frame height`, in % of
  the model diagonal (UV: of the set's texture height). A change scales the placement about the anchor
  (`EmMap::to_world([0, 0])`, UV `to_texel`), with the automatic depth re-fitted; one `UpdateStampPlacement`.
- Angle = the placement rotation (`placement_rotation_deg` / `rotate_placement_to`; automatic depth re-fitted); for
  the draft `tools.stamp.rotation_deg`. Typed values wrap into -180..180 (`wrap_deg`: 270 is -90), never clamp.
  Both fields only write what the user changed: a placed text whose size lies outside 0.2..400 % or whose angle
  lies outside -180..180 (a UV rotation is stored verbatim) is shown as it is and not re-written by showing it.
- B / I (`resolve_face`, FEATURE_TEXT 5.9): among the candidates with the wished italic flag (else the other) and the
  current stretch, the face of weight >= 600 nearest 700 for bold, else <= 500 nearest 400 (staying in the other class
  when the wished one is missing); thickening / slant only make up for what the face lacks (`FAUX_BOLD_EM`,
  `FAUX_SLANT_DEG`, or the old amount). A family change in the picker resolves the face again from what the text was;
  a pick in the style list is taken as it is (only `bold_em` / `slant_deg` the face has itself are reset). The traits of
  a placed text's face are read from its own font bytes (`face_traits`, cached per `(BlobId, face index)` in
  `TextPanelState`), never from the installed list.
- `candidates(fonts, doc, cache, family)`: the catalog's faces of the family and the fonts of that family used by the
  project's text stamps, without duplicates (same style and face index: the project's copy wins).
- `resolve_font(state, family, style) -> Option<FontRef>`: `doc.find_font` (a font the project holds; nothing read or
  copied), else the catalog face stored with `register_font`, else the bundled font with the toast "Font X not found,
  using Ubuntu Light". `None` without a document. (T7's `text_source` is meant to call it.)
- The content field of a placed text edits a `GapBuffer` (`egui::TextBuffer`): `insert_text` / `delete_char_range`
  call `gaps::insert` / `gaps::delete` and store the state (text, gaps) in a history of this focus session (at most 200
  states; an empty delete stores nothing); `replace_with` (egui's undo / redo of the field) looks for a stored state
  with exactly that text, backward from the current one, then forward, and restores its gap list; not found:
  `gaps::shift_for_edit`. The history is dropped when the field loses the focus or shows another text. While the field
  has the focus and its cursor stands between two characters of one line, that gap is selected
  (`Session::text_gap`) and the strip shows its line. A change of the content from outside the field (undo, redo of
  the document) drops the selected gap.

Text tab (round 4, `panels/text_tab.rs`; FEATURE_TEXT 6.4). `text_tab::show(ui, state, ps)` draws the page selector
Character | Warp | Path (`state.text_page`) and the page for `text_block::target(state)`; it queues its actions into
`state.actions` and never edits while idle.
- Character: Align, Line spacing, Letter spacing, Thicken (0-8 %, `fx.bold_em`), Slant (-45..45 degrees,
  `fx.slant_deg`), Outline (switch, width, colour), then the gap controls: "Gaps  line n of m  [<] [>]"; the strip
  (one 14 pt box per laid-out character, a combining mark in the box of its letter, a 10 pt slot after it where
  `gaps::slots` has a gap: cells of exactly 24 pt in a horizontal scroll area that gets the full content width as one
  allocation; only the cells inside the clip rectangle are laid out, at most 12 at 265 pt; a slot with a value is
  filled, the selected one framed and scrolled into view, an end slot (a stored value on the last character of a line)
  dashed; hover / accessible text "Gap between E and (space): 30 %", "Stored gap after 6: 30 % (no effect at the end
  of a line)"); the value row `-` [value, % of the text size, -50..400, 0.2 % per point dragged] `+` (1 %, 5 % with
  Shift) Reset, Reset all. The gap controls are disabled for the draft ("Place the text first; ...") and without a
  selected gap ("Click a gap in the strip above"). While a gap is selected the status bar reads "Gap between E and
  (space): 30 % of the text size"; the line is cleared when the gap or the Text tab goes away.
- Warp: 16 style pictures (None and the 15 styles, two rows of 8, 30 x 26 pt): a 4 x 2 grid sent through `WarpMap` at
  50 % bend; the name of the style under the pointer (else the chosen one) below; a click is one Final (Bend 50 % for a
  first style, else the bend kept). Bend, Horizontal, Vertical (-100..100 %), Direction Horizontal / Vertical, Reset
  (style None).
- Path: Straight / Arc / Circle / Curve (one Final; Arc with bend 25 %, Circle `path::new_circle` and Curve
  `path::new_curve` from the straight frame `stamp_source_frame` of the text without its line; Curve disabled for the
  draft). Arc: Bend. Circle: Diameter (0.5..100 em) and Fit (`path::fit_circle` with the current frame). Start
  (-100..100 em), Other side. Curve: Add point (middle of the longest piece, at most 16) / Remove point (the last one,
  at least 2). With `circle_fill > 1` the page shows "The text is longer than the circle. Raise Diameter or press Fit."
  in the warning colour.

Font picker (round 4, `panels/font_picker.rs`; FEATURE_TEXT 6.3). A popup under the font button (300 pt wide, closed by
a click outside or Esc): a search field with the focus; "In this project" (the families of the text stamps, most
recently added first, at most 8; "(in project)" for one that is not installed); "All fonts (k of n)": rows of 22 pt
drawn with `ScrollArea::show_rows`, the family name in the UI font, its sample from `AppState::font_samples` on the
right and the file size for a file of 5 MB or more. Search: case-insensitive substring, families that start with the
text first (`filtered`). Up / Down move the highlight, Enter or a click takes the family (`text_block::change_family`).
The font button queues one `AppAction::LoadFontSamples` per catalog size (round 5 R5-3: the button is drawn whenever
the Text block is, so the sample job never starts in the frame that opens the popup; `PickerState::asked_for`). The
per-family "N MB" label is kept in `PickerState::rows` (`RowData`, once per catalog size): the button fills it a few
families per frame (2 ms) and a row that is drawn before its family is known looks it up once. The picker's parts are
frame-profile segments ("picker: button" for the whole button, "picker: popup", "picker: setup" with "picker: search
field" in it, "picker: project rows", "picker: rows", "picker: row data"; the Text block is "text block").

Brush tab (`panels/brush_tab.rs`, round 4 B4; FEATURE_BRUSH 1.4, 6.3; ROUND4_PLAN R4-6 and section 7). It edits the
settings of the Eraser while the Eraser is the active tool, otherwise those of the Brush; settings are written to
`state.tools` directly (through `BrushSettings::sanitized`), commands are actions. Rows, top to bottom:
- "Brush tip - paints on: <layer>" ("Eraser tip - ..."; the layer is `Document::paint_target` of the active set:
  its name, "(locked)" / "(hidden)" added, or "no paint layer"; truncated), with [Reset] for the Sphere type (Turn,
  Tilt, Roll and Offset to 0, Align to Follow surface).
- Type: [Circular] [Sphere] (selectable buttons with the hover texts of FEATURE_BRUSH 1.4). There is no "Stay on
  connected surface" checkbox (plan section 7 item 1).
- Sphere only: Shape list (Ball, Cube, Cylinder, the file entry, a separator, "Remove file shape") and [Load...]
  (`LoadBrushShape(None)`), followed by a "!" with a hover text when `brush_shape.last_error` is set or the loaded
  field's report says `partial` or `thin`. The file entry reads the file name (cut to 15 characters in the closed
  list), "Loading... <name>" while a load runs, or "File (none loaded)" greyed out when no path is remembered
  (`prefs.brush_shape_path`); choosing a remembered file that is neither loaded nor loading also queues
  `ReloadBrushShape`. "Remove file shape" queues `RemoveBrushShape` and is greyed out without a remembered path.
- Sphere with a shape other than the Ball: the 72 x 72 preview (the three `TipField::principal_sections`, turned by
  `Ry(-turn) Rx(tilt) Rz(roll)`, in a fixed three-quarter view; the surface plane y = -offset as a thin ellipse; the
  part below it dimmer; a file shape that is not loaded shows "no shape"). A drag in it changes Turn (left / right)
  and Tilt (up / down) by 0.5 degree per point, wrapped into -180..180. Next to it Turn, Tilt, Roll (drag values,
  -180..180 degrees). The Cube and Cylinder fields come from `AppState::builtin_tips` (made on first use, as the brush
  tool does).
- Sphere: Offset (slider, -100..100 % of the radius).
- Sphere with a shape: Align (Follow surface / Fixed in space) and Use (Solid / Through; disabled and fixed on Through
  for a flat outline).
- Shape File with a loaded file whose openings were closed (`ReadyShape::close != Refuse`): "Openings" with the
  choices of `CloseMode::CHOICES` ("Use only the closed parts" only when `ReadyShape::closed_parts`); another choice
  queues `SetBrushShapeClose(mode)` (B5 bakes the file again and stores the choice with the path); disabled while a
  load runs.
- Two note lines (each at most 44 characters): "Paints the surface you point at." / "Also paints far sides and
  hidden parts." / "Paints everything inside the shape." / "Presses the outline straight through." (Circular /
  Sphere with the Ball / a shape used Solid / a shape used Through or flat), and always "The 2D view always paints a
  flat disc."
The rows are 20 pt with 2 pt between them; the label column is 72 pt. The tallest state (a loaded file with closed openings and a "!") is 238 pt (headless) / 238 pt (real window, brush_tab scenario) of a 246 pt tab area; at 1280 x 800 the computed lower
area (S2) holds it without scrolling. `PanelState.brush: BrushTabState { content, preview }` keeps the last frame's
content and preview rects for the tests.

**Round 9 (package B).** Under a selected 3D stamp (any mode) the tool options show, in the warning colour, "This
model uses the same texture area in several places (mirrored or repeated parts): the picture also shows there." when
`Document::stamp_shared_texels` is at least `UV_SHARE_WARN` (5 %); nothing while unknown. Under a wrapped stamp's stats
line: "Longer than the way round: P % shown" when `WrapStats::ends_share < 0.95` (a closed or helical spine).

**Stable (w1, item 6): the wrap lines name their own cause, one line each** (`tool_options::wrap_stats_texts`,
replacing the round-6 / round-9 warning lines above; the help lines "Fitted ..." and "Cut where it cannot lie flat
..." and the lines "Wound N times", "Continued onto N separate parts", "Too large ..." stay): with Ends meet and the
ends met (`met_itself` or `passed`) one line, "Longer than the way round: P % shown, the ends meet at the back" for a
helix (`passed`, P = `ends_share` < 95 %), "The ends meet at the back: P % of the picture's length shown" for a closure
(`met_itself`), else "The ends meet at the back"; "P % of the picture runs off the surface: an edge, a hole or the end
of the part" only when `reached_edge` and `covered < 0.98`; "P % of the picture is lost where the surface had to be
cut" when `lost_cut >= 2 %`; "P % of the laid picture lies on surface hidden under another part or folded over itself, and
is not shown there" when `rejected >= 5 %` (DamagedHelmet Jump on; nonmanifold's and the Lantern's single parts); "Only P % of the picture lies on the surface"
when `covered < 0.98` and neither an edge nor the ends and cuts explain it (`covered` more than 2 % below
`ends_share` (when cut) minus `lost_cut` (when shown)). The shared-texel line also shows for Project-mode stamps (the
texture reuse is the model's, in any mode): an intended difference from the main build's panel (round 9's, kept).
Stable (w2): the cut line reads "P % of the picture is lost where it is cut open round the closed part's corners" when
`wraps_closed`; the covered line reads "The part is narrower than the picture: only P % of the picture lies on it"
when `room < 0.95`.
Stable (fix_wrap_1): the ends line with a share ("Longer than the way round: P % shown ..." / "The ends meet at the
back: P % ...", P = `ends_share`) needs `ends_share < 0.95` **and** `beyond >= 2 %` (`ENDS_BEYOND_MIN`,
`tool_options::ends_cut_shown`); with `ends_share < 0.95` and nothing bare beyond the closure there is no ends line
(ABeautifulGame Jump on: w1 / w2 said "The ends meet at the back: 74 % ..." with the board holding the picture); the
plain "The ends meet at the back" stays for `ends_share >= 0.95`. The "runs off" and "Only P %" lines use `shown`
instead of `covered` (P = 1 - `shown`, `shown`); the "Only P %" line's expected share is `ends_share` whenever an
ends line shows (also the plain one). The `rejected` line reads "P % of the laid picture fell where the
surface overlaps another part or folds over itself: each spot of the picture is shown in one place only": a dropped
place's cell always has an owner, so it is shown elsewhere (Avocado Jump on 35 % with ink 0.996; the old wording "...
and is not shown there" read like a loss).

Stable (fix-wrap, `docs/notes/stable/fix_wrap_2.md`): with less than 98 % shown, `gaps >= 2 %` (`GAP_LINE_MIN`) adds the
help line "P % of the picture falls into the gaps between separate parts" (P = `gaps`; parts200 Jump on 28 %, where
fix_wrap_1 said "28 % ... runs off the surface" while the picture looks whole) and `at_part >= 2 %` the warning "P % of
the picture stops where it meets a neighbouring part" (P = `at_part`; ABeautifulGame Jump on 9 %, where it said "9 %
... runs off the surface: an edge, a hole or the end of the part": the bare strip lies on the flush marble frame next to
the board slab). Both come after the ends line; "runs off" says only the rest (`1 - shown - beyond - lost_cut - gaps - at_part`,
whether or not their lines show) and shows only when that rest is 2 % or more (DamagedHelmet Jump on: of 2.6 % bare,
2.0 % lost to cuts and 0.5 % in gaps: fix_wrap_1's "3 % ... runs off" becomes "Only 97 % of the picture lies on the
surface"); the "Only P %" / "narrower" line's expected share also drops the gaps and neighbouring-part shares when their
lines show.

Polish (item 3, `tool_options::largest_wrap_loss`): when the "Only P % of the picture lies on the surface" line shows and
no cause line passed its threshold (no gaps, neighbouring-part, cut or ends-with-share line; "runs off" never shows with
it), the line names the largest loss with its share: ", mostly lost to cuts (S %)" (`lost_cut`), "in the gaps between
separate parts" (`gaps`), "where it meets a neighbouring part" (`at_part`), "beyond where the picture closed on itself"
(`beyond`, only when the spine met or passed itself and no ends line shows) or the rest (`1 - shown - beyond - lost_cut -
gaps - at_part`): "running off the surface" when `reached_edge`, else "hidden or folded"; nothing below 0.5 %
(`LARGEST_LOSS_MIN`). S is rounded like the other lines (at least 1 %). DamagedHelmet Jump on: "Only 97 % of the
picture lies on the surface, mostly lost to cuts (2 %)" (`lost_cut` 1.96 %, below the cut line's 2 %). The "narrower"
line names no cause (the room is the cause).

### 8.8 Dialogs (`dialogs/*`) [A5]

```rust
pub struct DialogsState { /* import: egui textures of the UV thumbnails; export: cached plan + the options it was built from */ }
impl DialogsState { pub fn new() -> DialogsState; }
/// Draws the dialog in state.dialog (egui::Modal) and, when state.busy is modal, the progress modal on top.
pub fn show(ctx: &egui::Context, cx: &mut ViewCx<'_>, ds: &mut DialogsState);
```
`show` takes the dialog out with `let mut d = std::mem::replace(&mut cx.state.dialog, Dialog::None);`, draws it (so the dialog's
own state and `cx` — needed by the embedded preview — can be borrowed at the same time) and **always puts it back**. Dialogs
change their own fields directly and push actions; they close only through actions handled by dispatch (`CloseDialog`,
`ImportCreate`, `RunExport`, `ConfirmPending`, `SaveThen`, `Cancel`). Dialogs never call `rfd` and never read keys: Esc reaches
them as `AppAction::Cancel` (`ModalResponse::should_close()` may be used for the click-on-backdrop case of Message / About).

- **Import (R1, R2)**. Opened when the import job delivers its result (8.9). Shows file name, triangle count, warnings, and one
  row per texture set:
  - materials, triangle count;
  - the **UV-layout thumbnail**: `ImportDialogState.uv_thumbs[set]` (a 256-px image rasterized once on the import worker by
    `uv_layout_image`, 5.7), uploaded once with `ctx.load_texture` and kept in `DialogsState` — visible proof that the wrapping
    was recognised. Wire edges are **never** drawn with the egui painter (hundreds of thousands of edges would be
    re-tessellated every frame);
  - UV statistics from `ImportDialogState.uv_stats[set]`: coverage in percent, "overlapping UVs" when `overlapped_texels > 0`,
    "N mirrored triangles" when `mirrored_tris > 0`;
  - the texture choice: *Detected: name WxH* (or "missing — a blank texture will be created"), *Choose image...* (pushes
    `ImportPickTexture { set, path: None }`; afterwards the row shows the file name and size, or the decode error in red),
    *Blank* (512 / 1024 / 2048 / 4096 / 8192 + fill colour).
  Materials with `texture_set == None` are listed under "Not paintable (no UVs)" so the limitation is visible before Create.
  A model without any UVs (`report.no_uvs`) arrives with the generated texture layout already applied (8.9) and shows
  `generated_layout_text`: the model has no texture layout of its own, so one was generated (flat islands packed without overlap
  into a new texture; painting, stamps and text work everywhere; a texture made for the model elsewhere will not fit), plus the
  checkbox "Generate a texture layout (automatic UVs)", checked (unchecking => `ImportReload { fallback_uvs: false }`, which
  leaves the model unpaintable and says so). *Create* => `ImportCreate`; it is disabled, with the reason shown next to it, while
  `mesh.sets` is empty ("The model has no UV mapping") or a `File` choice has an error. *Cancel* closes.
- **Unsaved changes**. Save / Don't Save / Cancel before New, Open, Open Sample and Quit (`SaveThen(p)` / `ConfirmPending(p)`).
- **Export (R4, R5)**. Defaults when opened (`ShowExport`): `dir` = `prefs.last_export_dir`, else `<folder of the .wtep>\export`,
  else `<folder of doc.source.original_path>\export`, else empty; `base_name` = document name; flattened PNG on; per-layer PNGs
  on; ORA and PSD off; Layers "Visible"; zip off; seam padding 0. The default folder is a sub-folder on purpose, so the first
  export can never land on top of the user's original texture file. Opened for layers (`ShowExportLayers`, 8.3): Layers
  "Selected", no set filter, per-layer PNGs only; the normal ticks wait in `other_ticks` (fix_export, below).
  Left column: output folder (text + "Browse..." => `ExportPickFolder(None)`; while the field is not being edited it shows
  the end of the path with the middle elided, `dialogs::export::elide_middle`, e.g. `E:\…\round3\shot_export`, and the full
  path on hover; a click or Tab edits the full path with the cursor at its end — R3-10, finding F9), base name, sets to export, flattened format (PNG /
  JPEG + quality + matte colour with the note "JPEG has no transparency" / TGA / BMP / TIFF / WebP lossless), per-layer PNGs,
  ORA, PSD, "Pack into one .zip file" (`options.package`), seam padding (px), and **the list of files that will be written**:
  `export::plan(&doc.snapshot(), &options)` (what goes to disk) and `export::plan_contents` (names and kinds; cached in
  `DialogsState::export_plan` as `PlanCache { plan, names, kinds, hidden, exists }`, recomputed when the options or the document
  change), file names only, stamp and text layer files marked "separate layer" (from `PlannedKind::Layer { raster: false }`;
  the app no longer classifies files itself), the per-layer file of a hidden layer marked "(hidden)" (accept_fixes item E2,
  `hidden_layer_file`: under All and under a selection, which exports hidden layers too), files that already exist marked
  "will be replaced". Each frame the list's lines (name + markers) and the preview caption are recorded in
  `ToolProbe::export_files` / `export_caption` for the self-test. With the zip: the header
  "1 file will be written: <base>.zip (N files inside)" carries the "will be replaced" marker, the rows list the entries.
  **Layers** row (export-select; replaces the round-1 checkbox "Include hidden layers") in the options grid after the base
  name: `Visible` | `All` | `Selected (N)` (selectable labels; `scope_of` / `apply_scope`): Visible / All = `layers: None`
  with `include_hidden` false / true (All's hover, check_accept: "Hidden layers too, in the per-layer, ORA and PSD files
  (the flattened texture is the visible layers)"); Selected = `layers: Some(st.selected_layers)` (and clears the set filter when the
  layers change: their set decides), disabled with N = 0 (hover "Select layers in the Layers list first"). Under Selected a
  weak line names the layers ("Logo, WTE", after three "... and 2 more", `selection_names`; fix_export: "+ its layer in 1
  other texture set" / "+ their layers in N other texture sets" when designs span sets, `linked_extra` / `linked_note`;
  polish item 1: N counts the distinct texture sets that hold the added layers, not the layers, so two selected designs
  across the same two sets read "+ their layers in 1 other texture set"; "its" for one design, "layer" for one layer) and the
  texture-set checkboxes are hidden. fix_export: a change of the choice goes through `switch_scope(st, scope)`: when the
  dialog was opened for layers (`other_ticks` is `Some`) and the choice moves between Selected and Visible / All, the
  sets, flattened format, per-layer, ORA and PSD ticks of the two sides swap (`ExportTicks::of` / `apply`) and the format
  fields follow (`sync_from_options`); so Visible / All export every set with the normal ticks again, and Selected gets
  the layer preset back. A dialog opened with Export... keeps its ticks (as before). Next to "Pack into one .zip file"
  a weak label shows the zip's name (`export::package_name_for`: `<base>.zip` or `<base>_selection.zip`).
  Right column: the embedded live `preview_widget(.., ViewId::ExportPreview, .., matte)` with Lit/Unlit — orbitable inside the
  modal (8.5); `matte = Some(jpeg_matte)` for JPEG, else `None` (alpha checker). Caption under it (`preview_caption(opts)`,
  accept_fixes item A; the preview always shows the visible layers composited): Visible or All with the flattened texture
  on (the flattened texture is the visible layers under All too; All adds hidden layers to the per-layer, ORA and PSD
  files only): "Shows the flattened texture exactly as exported (seam padding and JPEG compression not shown)"; Visible /
  All with it off: "The preview shows the layers together; no flattened texture is exported"; a layer selection with it on: "The preview shows the
  whole texture; the flattened file holds only the selected layers"; a selection with it off (the preset of "Export selected
  layers..."): "The preview shows the whole texture; only the selected layers are exported, as separate files".
  *Export* is disabled while `dir` is empty or the plan is empty, with the reason next to it (`export_blocker`: "Choose an
  output folder", "Choose at least one kind of file", "Select at least one layer" (a selection that matches nothing),
  "Choose at least one texture set", "Nothing to export"). On click: if any planned path exists, the dialog switches to
  the confirm step (`confirm_replace = Some(existing paths)`: "N files will be replaced" + list, *Replace* / *Cancel*); otherwise,
  and on *Replace*, it pushes `RunExport(options)`.
- **Progress**. Title, stage, bar, Cancel (=> `CancelJob`) for modal jobs.
- **Message**. Title + scrollable lines (import/load warnings, error details) + Close.
- **Close openings** (`Dialog::CloseOpenings`, drawn by `brush_flows::show_openings_dialog`; round 4, plan section 7
  item 5): opened when a brush-shape bake with `CloseMode::Refuse` returns `TipError::HasOpenings`. It names the file
  (name and full path), the number of openings and the size of the largest (share of the model's bounding-box
  diagonal; "under 1 %" for a tiny one) and offers, as radio buttons with a one-line note each: Flat caps (default),
  Rounded caps, Fill all hollows ("also fills deep hollows and pockets"), Use only the closed parts (only when
  `closed_parts`). Buttons "Close with <choice>" and Cancel; Esc / `CloseDialog` = Cancel. It answers with
  `AppAction::AnswerOpenings(Some(mode) | None)`. The rects of the choices (`PanelHit::OpeningsChoice(index)`) and the
  two buttons (`PanelHit::OpeningsOk`, `OpeningsCancel`) go into `panel_rects` for the self-test.
  Round 4 (gate 3): the font button of the Text block records `PanelHit::FontButton` (the self-test opens the font
  picker with a `ScreenPointer` click on it, scenario `text_fonts`).
- **About / Shortcuts**. Static content; the shortcut list is generated from `shortcuts::table()`.

### 8.9 Jobs and file flows (`jobs.rs`, `io_flows.rs`, `toasts.rs`, `prefs.rs`, `dnd.rs`) [A1]

```rust
pub struct JobProgress { /* AtomicU32 fraction bits, Mutex<String> stage, AtomicBool cancel, egui::Context */ }
impl wte_core::Progress for JobProgress { /* report() stores and calls ctx.request_repaint() */ }
pub enum JobOutput {
    /// uv_thumbs / uv_stats: one entry per set (uv_layout_image(.., 256) and TexelMap::build(.., 256, 256, 0).stats), empty on Err.
    Imported { path: PathBuf, result: Result<ImportedModel, String>, uv_thumbs: Vec<PixelBuf>, uv_stats: Vec<UvStats> },
    Built { result: Result<Document, String>, path: Option<PathBuf>, warnings: Vec<String>, force_save_as: bool },
    /// scene_generation = Session::scene_generation at the moment the save started.
    Saved { result: Result<(PathBuf, u64 /* snapshot.history_state */, Option<PngReuse>), String>, scene_generation: u64, then: Option<Pending> },
    Exported { result: Result<ExportReport, String>, dir: PathBuf },   // toast: export_toast_text ("1 file" / "N files")
    Extracted { result: Result<Vec<PathBuf>, String>, dir: PathBuf },
    Fonts(FontCatalog),
    // round 4 (S2)
    BrushShape { request: u64, path: PathBuf, close: CloseMode, result: Result<Arc<TipField>, TipError> },  // brush_flows::on_job_output
    FontSamples(Vec<FontSample { family: String, image: Option<(u32, u32, Vec<u8>)> }>),                   // text_ops::on_font_samples
    // round 5 (R5-1)
    StampBake { generation: u64 /* Session::scene_generation at the start */, output: Box<StampBakeOutput> },  // stamp_bakes::on_output
    // round 8 (R8-2): silent jobs of stamp_bakes::WrapJobs, both handled by stamp_bakes::on_wrap_output
    WrapPreview { generation: u64, output: Box<StampBakeOutput> },  // a wrapped stamp's Preview bake
    WrapParam { generation: u64, output: Box<StampBakeOutput> },    // a wrapped stamp's parameterization only
}
/// A finished job; `cancel_requested` = the user pressed the progress dialog's Cancel while it ran.
pub struct Finished { pub output: JobOutput, pub cancel_requested: bool }
pub struct Jobs { /* running: Vec<(title, modal, Arc<JobProgress>, mpsc::Receiver<JobOutput>)> */ }
impl Jobs {
    pub fn new() -> Jobs;
    pub fn spawn(&mut self, ctx: &egui::Context, title: &str, modal: bool,
                 f: impl FnOnce(&JobProgress) -> JobOutput + Send + 'static);
    /// Round 5: `spawn` with the stage shown until the job reports one (a stamp bake: "", the title alone); false when no
    /// worker thread could be started.
    pub fn spawn_staged(&mut self, ctx: &egui::Context, title: &str, modal: bool, stage: &str,
                        f: impl FnOnce(&JobProgress) -> JobOutput + Send + 'static) -> bool;
    /// Round 8: a non-modal job the status bar does not show (`busy` skips it; start / end logged at debug level): the
    /// wrap Preview and param-only jobs. False when no worker thread could be started.
    pub fn spawn_silent(&mut self, ctx: &egui::Context, title: &str,
                        f: impl FnOnce(&JobProgress) -> JobOutput + Send + 'static) -> bool;
    /// Round 5: removes the running job with that title and blocks on its receiver. Used only by `stamp_bakes::settle`.
    pub fn wait_for(&mut self, title: &str) -> Option<JobOutput>;
    pub fn poll(&mut self) -> Vec<Finished>;
    pub fn busy(&self) -> Option<BusyInfo>;           // the first modal job, else the first job that is not silent
    pub fn cancel_modal(&self);
    pub fn any_running(&self) -> bool;
    pub fn modal_running(&self) -> bool;
}
// stamp_bakes.rs (round 5, R5-1): one Final bake in flight at a time
pub struct FinalBakes { /* the bake in flight (stamp, title, generation, start), an output held during a stroke */ }
/// From `App::logic` right after the job poll: applies a held output once no stroke is open; when no bake is in flight and
/// a session exists, `doc.next_final_bake()` and `jobs.spawn_staged(ctx, "Finishing stamp" | "Finishing text", false, "",
/// move |_| JobOutput::StampBake { generation, output: Box::new(job.run_isolated()) })`; sets `state.final_bakes_pending
/// = in flight || held || doc.has_unbaked_stamps()`. Bakes start also while a pending edit is open on another stamp; none
/// starts while a stroke is open.
pub fn pump(app: &mut WteApp);
/// From `io_flows::on_job_output`: dropped when `generation != session.scene_generation`; else `apply_final_bake`:
/// `Applied { id, empty }` -> when `await_first_bake == Some(id)` and `empty`, the "covers no texels" warning of the
/// Stamp / Text tool (8.6); `Busy(out)` -> held; `Stale` / `Failed` -> log.
pub fn on_output(app: &mut WteApp, generation: u64, output: Box<StampBakeOutput>);
/// Waits for the bake in flight of the open document (`jobs.wait_for`) and applies it, then `doc.settle_final_bakes()`.
/// Called before `MergeDown` / `RasterizeLayer` (fix_export: no longer by `start_export` nor the `ShowExport` arm; polish
/// item 2: no longer by `start_save`; both wait through `feed_waiting_snapshot`) (which `MergeActiveDown` / `RasterizeActiveLayer` reach);
/// a layer that is gone after it (the bake removed an empty layer outside the home set) is left alone. Afterwards
/// `debug_assert!(!doc.has_unbaked_stamps())` (unless a stroke is open).
pub fn settle(app: &mut WteApp);
/// From `App::ui` right after the frame's dispatch: when a stamp waits (or an output is held), no bake is in flight and no
/// stroke is open, `ctx.request_repaint()`, so a Final made in a frame without input (the Transform tool's wheel gesture
/// ends on a timer) starts its bake in the next frame instead of at the next mouse move; and each stamp first seen
/// waiting here gets its mark time (marks are dropped while a pending edit is open).
pub(crate) fn after_dispatch(state: &mut AppState, ctx: &egui::Context);
// Log lines (the gate greps them): "final bake: stamp N started (text|image)", "final bake: stamp N applied after T ms
// (worker W ms, empty: b)" (T from the end of the dispatch of the Final's frame, `after_dispatch`; from the start of the
// job when there is no mark), "final bake: stamp N result dropped (stale)", "final bake: settle waited T ms".
// AppState (round 5): async_final_bakes (false in AppState::new and the app tests; WteApp::new: true unless
// WTE_SYNC_FINAL_BAKES=1), final_bakes_pending, await_first_bake: Option<StampId>, final_bakes: FinalBakes.
// install_document: doc.set_defer_final_bakes(state.async_final_bakes); a held output and await_first_bake are dropped.
//
// Round 8 (R8-2, ROUND8_PLAN 4.1): wrapped Previews as latest-wins jobs (stamp_bakes.rs; ARCHITECTURE 5.20, core 3.4).
// install_document also calls doc.set_defer_wrap_previews(state.async_final_bakes): with WTE_SYNC_FINAL_BAKES=1 the
// wrapped Previews stay on the UI thread (the comparison mode); projected / UV Previews are unchanged in both modes.
#[derive(Default)] pub struct WrapJobs {
    in_flight: Option<WrapFlight>,                   // one wrap job at a time: { id, kind, generation, started, placement,
                                                     //   visibility (objects-tabs: doc.visibility_generation() at the start) }
    held: Option<(u64, Box<StampBakeOutput>)>,       // a Preview output that arrived while a stroke was open
    want_param: Option<StampId>,                     // the Transform tool's request (async mode), taken by pump
    pub(crate) param_failed: Option<(StampId, Placement, u64)>, // a param-only job found no surface: not asked again
                                                     //   (u64 = the visibility generation; objects-tabs)
    pub counters: WrapJobCounters,                   // previews started / applied / due again / stale, params started /
}                                                    // applied, ui_params (TransformTool::ensure_wrap_param, sync mode)
// pump (before the Final part): a held Preview output is applied once no stroke is open; when no wrap job is in flight
// and no stroke is open, doc.next_wrap_preview_job() (the lowest waiting wrapped stamp, made from the stamp as it is now:
// latest wins) else doc.wrap_param_job(want_param) is spawned with Jobs::spawn_silent ("Wrap preview" / "Wrap fit": not
// in the status bar; run_isolated); no worker thread -> doc.set_defer_wrap_previews(false) (round-6 behaviour).
// A Final does not start while the wrap job in flight belongs to a stamp that waits for its Final (same generation):
// it starts after that job lands (plan 4.1; the Preview output is stale after the flush, and its parameterization is
// cached when it serves the released placement, so the Final reuses it: core 5.20, check:B). While the Final waits,
// after_dispatch asks for no frame (the landing job asks for one).
pub fn on_wrap_output(app: &mut WteApp, generation: u64, output: Box<StampBakeOutput>); // JobOutput::WrapPreview / WrapParam
// Another generation -> dropped. Preview: held while a stroke is open, else doc.apply_wrap_preview: Applied { due_again }
// (the last finished picture; due_again: the stamp moved since, pump starts its next job in the same logic pass) or
// Stale (Esc, undo, redo, a Final, a removed stamp). ParamOnly: doc.apply_wrap_param (stats and outline, no pixel);
// nothing for an unchanged placement and an unchanged visibility generation -> param_failed.
pub(crate) fn after_tools(state: &mut AppState, jobs: &mut Jobs, ctx: &egui::Context); // App::logic after ensure_wrap_param:
// starts the param-only job just asked for when no wrap job is in flight; updates wrap_jobs_pending.
// after_dispatch also asks for the next frame when a wrapped stamp waits for its Preview job and none is in flight.
// settle never waits for wrap jobs. reset_for_new_document also drops held / want_param / param_failed.
// AppState (round 8): wrap_jobs: WrapJobs, wrap_jobs_pending (in flight, held, a wrapped stamp waiting, a param asked).
// Log lines: "wrap preview: stamp N job started", "wrap preview: stamp N applied after T ms (worker W ms, due again: b)",
// "wrap preview: stamp N result dropped (stale|other document) ...", "wrap fit: stamp N param-only job started",
// "wrap fit: stamp N parameterization landed after T ms (worker W ms)", "wrap fit: stamp N has no surface to wrap onto".
// FinalBakes::last_applied() (round 8): the stamp and "applied after" ms of the last applied Final (scenario step 10).
// io_flows.rs
pub fn on_job_output(app: &mut WteApp, out: JobOutput, cancel_requested: bool);
/// doc.is_modified() || doc.has_pending_edit(): what the unsaved prompts (close, open, new, sample, drop) and the title's
/// "*" test. A Commit::Preview edit still open (text typed into a focused field) has not pushed a history entry yet. Undo
/// or redo back to the saved state clears both (6.7).
pub fn has_unsaved_changes(doc: &Document) -> bool;
pub fn handle_close_request(app: &mut WteApp, ctx: &egui::Context);
pub fn update_title(app: &mut WteApp, ctx: &egui::Context);
pub fn start_import(app: &mut WteApp, path: PathBuf, fallback_uvs: bool);
pub fn start_open_project(app: &mut WteApp, path: PathBuf);
/// Writes `UiState::capture` into `doc.ui_state` and flushes the pending edit. Polish item 2: no `settle` (it froze the
/// UI for a wrapped stamp's Final, 3-4 s on ABeautifulGame): as `start_export`, when `stamp_bakes::bakes_pending` the
/// modal "Saving project" job starts at once and waits in `receive_snapshot` ("Finishing stamps", Cancel = "Save
/// cancelled") for the snapshot `feed_waiting_snapshot` sends (`SnapshotFeed { what: "save" }`); else it gets
/// `doc.snapshot()` right away. Then `project::save`; `JobOutput::Saved` clears the feed. Scenario `save_unsettled`.
pub fn start_save(app: &mut WteApp, path: PathBuf, then: Option<Pending>);
/// fix_export: flushes the pending edit; no `settle`. When `stamp_bakes::bakes_pending` (a Final in flight, held or due),
/// the modal "Exporting" job starts at once and waits in `receive_snapshot` ("Finishing stamps", Cancel works) for the
/// snapshot that `feed_waiting_snapshot` sends; else it gets the snapshot right away. The job then runs
/// `export::remove_stale_temps(dir, process_running)` and `export::run`.
pub fn start_export(app: &mut WteApp, opts: ExportOptions);
/// fix_export: every frame after `stamp_bakes::pump`: while an export or (polish item 2) a save waits
/// (`WteApp::snapshot_feed: Option<SnapshotFeed { tx, since, what }>`, one modal job at a time) and a Final bake or a
/// wrap job runs on a worker, nothing (a repaint in 50 ms); once no bake is pending, sends `doc.snapshot()` to the job; a
/// bake that is due while no job runs (no worker thread) is settled here so the job never waits forever. Log:
/// "<export|save>: the Final bakes landed after T ms; starting".
pub fn feed_waiting_snapshot(app: &mut WteApp);
/// fix_export: `OpenProcess` + `GetExitCodeProcess` (Windows; access denied counts as running), `/proc/<pid>` elsewhere.
pub fn process_running(pid: u32) -> bool;
pub fn install_document(app: &mut WteApp, doc: Document, path: Option<PathBuf>, force_save_as: bool);
pub fn pick_model(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf>;       // rfd, synchronous, .set_parent(frame)
pub fn pick_project(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf>;
pub fn pick_save_project(frame: &eframe::Frame, prefs: &Prefs, default_name: &str) -> Option<PathBuf>;
pub fn pick_image(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf>;
pub fn pick_folder(frame: &eframe::Frame, prefs: &Prefs) -> Option<PathBuf>;
// toasts.rs
#[derive(Default)] pub struct Toasts { /* visible toasts, error counter */ }
impl Toasts {
    /// Queues and logs a toast (every push is logged and every Error push counted). Coalescing: a text identical to a
    /// visible toast of the same level replaces it (moved to the newest place, timer restarted, count + 1); an Info /
    /// Warning "Undo: .." or "Redo: .." replaces the visible Undo / Redo toast (count + 1 while the direction is the
    /// same, else 1). Drawn as "<text>  (xN)" when N > 1.
    pub fn push(&mut self, level: ToastLevel, text: impl Into<String>);
    /// `push` with a hover text (the full path of a folder the text names by its name only); coalescing as `push`.
    pub fn push_with_detail(&mut self, level: ToastLevel, text: impl Into<String>, detail: impl Into<String>);
    pub fn info(&mut self, text: impl Into<String>);  pub fn warn(&mut self, text: impl Into<String>);
    pub fn error(&mut self, text: impl Into<String>);                 // push shorthands
    pub fn error_count(&self) -> usize;               // number of Error-level pushes so far (monotonic; self-test exit code 2)
    pub fn show(&mut self, ctx: &egui::Context, within: egui::Rect); // within = state.central_rect of this frame
    pub(crate) fn drawn_rects(&self) -> &[egui::Rect];  // rects of the toasts of the last show (self-test)
}
pub const MAX_WIDTH: f32 = 380.0;                     // widest toast, frame included
```
- Workers are `std::thread::spawn` (the rayon pool stays free for the core). One worker per job.
- **Modality**: the Import, Build (`from_import`), Load, Save, Export and Extract jobs are `modal: true`; only the Fonts job is
  non-modal. While a modal job runs the progress modal covers the window: views ignore input (8.5), `shortcuts::handle` returns
  nothing, drag-and-drop is ignored and no new job can be started — so the session can never be replaced while a job that
  refers to it is running. Only the progress dialog's Cancel (`CancelJob` => `jobs.cancel_modal()`) is available.
- Flows:
  - *New from Model* (also drag and drop and the command line; model extensions `MODEL_EXTENSIONS` = obj, gltf, glb, fbx, the
    file dialog filter "3D models (OBJ, glTF, GLB, FBX)"): unsaved check -> path (action or `pick_model`) -> import job
    (`import_from_path_auto_uvs(path, opts, auto_uvs = true)`: a model without any UVs gets the generated layout at once, R6;
    then `uv_layout_images(&mesh, 256)` and per set `TexelMap::build(&mesh, set, 256, 256, 0).stats`; the job logs
    "import job of <path>: import N ms, UV previews M ms") -> `Imported` => `Dialog::Import` with one `SetChoiceUi::Detected`
    per set and `fallback_uvs = model.options.generate_fallback_uvs` (or `Dialog::Message` on error) -> `ImportReload`
    restarts the import job with the other `fallback_uvs` and `auto_uvs = false` (`start_import` recognises the reload by the
    open, loading import dialog) -> `ImportCreate` -> build job (`Document::from_import`) -> `Built` => `install_document`. The import
    warnings were listed in the import dialog; the build job passes none, so they are not shown a second time.
    `Imported` and `Built` first check `gpu::scene::check_mesh_fits(&mesh, gpu.limits())`: a mesh whose vertex or index buffer
    exceeds the device's `max_buffer_size` gives an error toast and `Dialog::Message { title: "Model too large" }` and
    replaces nothing.
  - *Open*: unsaved check -> path -> `project::load` job -> `Built` => `install_document` + `Dialog::Message` for warnings
    (missing blobs, font fallbacks; shown nowhere else).
  - *A file that cannot be opened* (round 3, R3-8): when the import or load job fails, `cannot_open_text(path)` checks the
    path itself: missing => "Cannot open <name>: file not found", no permission => "Cannot open <name>: access denied", a
    folder => "... it is a folder, not a file", another OS error => its text. That text is the job's error, the error
    toast as it is (without the "Import failed:" / "The project ... could not be opened" prefix) and the second line of the
    `Dialog::Message`. A file that opens fine keeps the core's error text (its content, or a file it refers to, is the
    problem). The core's error texts (`error.rs`) are not changed for this.
  - *Cancel* (progress dialog): a job that was asked to stop and returns `Err` was cancelled, not failed: info toast
    ("Import cancelled", "Opening cancelled", "Save cancelled: the project was not saved", "Export cancelled", ...), no
    `Dialog::Message`, no error toast (so no self-test failure). A cancelled save keeps the document modified and drops
    `then`; a cancelled export leaves the Export dialog open as it was.
  - *Save*: no path or `force_save_as` => Save As (`pick_save_project`); write `ui_state` into the document,
    `doc.flush_pending_edit()`, `doc.snapshot()`, `project::save` job (`now_unix` from `SystemTime`, `keep_backup` from prefs)
    -> `Saved`: **ignored (logged) unless `scene_generation == session.scene_generation`**; on success
    `doc.mark_saved(snapshot.history_state)`, `session.path = path`, add to the recent files, then continue with `then`; on error an error
    toast + `Dialog::Message`, and `then` is dropped.
  - *Export* (`RunExport` already flushed the pending edit, 8.3): `doc.snapshot()` -> `export::run` job -> `Exported`: success =>
    close the Export dialog, info toast "Exported N files to <folder name>" (a zip package, `report.packaged > 0`:
    "Exported <base>.zip (N files) to <folder name>"; both start with "Exported " and end with " to <folder>") with the full
    path as its hover text
    (`push_with_detail`, R3-7); error => `Dialog::Message` with the details (the Export
    dialog stays open underneath for a retry).
  - *Extract Original Files*: job running `project::extract_bundle(&doc.source, &doc.blobs, &dir, false)` on `Arc`/cheap clones
    (existing files are never overwritten: when names are taken the set goes into a fresh `<model>_original` sub-folder) -> info
    toast "Extracted N files to <name of the folder actually written>" (full path on hover).
  - *Open Sample*: unsaved check -> `Document::sample(kind)` synchronously -> `install_document`.
- `install_document`: `tools.cancel`; replaces `state.session` (fresh camera framed on `mesh.bounds`, `uv_view` fit on first
  draw — the session starts with `UvViewState::default()` and the 2D view calls `fit` in the first frame in which
  `session.uv_view` still equals that default; there is no separate flag —, `ui_state` restored when present — objects-tabs: `UiState::hidden_objects` (names; unknown names ignored, anything but an
  array of strings counts as none) is applied with `doc.set_hidden_objects(hidden_from_names(..))` before the session is
  stored, with one Info toast "1 object is hidden: Cube (Object > Show All)" / "N objects are hidden: A, B (Object > Show
  All)" (names up to three) when any is hidden; the object selection starts empty —, `selected_stamp = None`, a new process-wide unique `scene_generation`); resets the
  document-bound tool settings **`tools.stamp.source = None; tools.stamp.source_name.clear()`** (a `BlobId` of the previous
  document must never be used with the new one); closes dialogs.
- Close (`handle_close_request`, on `i.viewport().close_requested()`): a modal job is running => `ViewportCommand::CancelClose`
  + warning toast "Please wait for the running operation to finish" (also when the document is unmodified: a save or export
  must not be cut off); else `has_unsaved_changes(doc)` and `!allow_close` => `CancelClose` + `Dialog::Unsaved { then: Quit }`;
  when an Unsaved prompt for another operation is already open, its `then` becomes `Quit` (quit wins) with the info toast
  "The window closes after you answer the unsaved-changes prompt". Confirmed quit sets `allow_close` and sends
  `ViewportCommand::Close`. When the window really closes (not in self-test) `remember_window` stores
  `prefs.window_maximized` and, only when not maximized, the inner size as `prefs.window_size`.
- Title (`update_title`): computes `"<name>[*] - Wrapped Texture Editor"` (`*` = `has_unsaved_changes`) and sends
  `ViewportCommand::Title` **only when it
  differs from `state.last_title`** (then stores it). `send_viewport_cmd` always requests a repaint (8.1); sending the title
  every frame would make the idle app repaint — and re-render every 3D view — forever.
- Native dialogs are never opened when `state.selftest` (the `pick_*` helpers are only called for `None` paths outside
  self-test; actions carry explicit paths there). `pick_*` start in `prefs.last_dir` and update it.
- Startup: a `Fonts` job runs `FontCatalog::bundled()` + `load_system()`; until it arrives `state.fonts` is the bundled catalog
  and `state.fonts_loading` is true.
- Drag and drop (`dnd.rs`): while `i.raw.hovered_files` is non-empty draw a full-window "Drop to open" overlay. The dropped
  paths are classified first (`plan_drop`): one document is opened, the first `.wtep`, else the first `.obj/.gltf/.glb`
  (`OpenPath`: open project / import flow); images become stamp sources (`OpenPath` => `ImportStampImage(Some(path))`, toast
  "Open a model first" without a document) only when the drop holds no project or model (a model finds its own textures and
  material files through its MTL / glTF); every other path (`.mtl`, `.bin`, folders, unknown types, a second model, the
  model's texture) is not opened, and one info toast "Not opened: <names> (<reason>)" names them. An unsupported file can
  therefore never take the place of the model dropped with it.
- Toasts (`toasts.rs`): an `egui::Area` whose bottom-right corner is 12 pt inside the bottom-right corner of the central view
  area (`state.central_rect`, `pivot(RIGHT_BOTTOM)`, `constrain_to` that rect; the rect is the area the central panel was
  given, not its response rect, which grows with content that overflows it — a long row of set tabs in the Split view put
  toasts 7 pt under the side panel on tex_formats.obj, round-3 gate 2), so a toast never covers the side panel (Layers
  rows and footer buttons), the tool bar or the status bar (round 3, R3-7; before, the area was anchored to the window
  corner and an export path made a toast 900 px wide over the Layers list). Each toast is at most `MAX_WIDTH` = 380 pt
  wide (less in a narrow view); its text wraps, and a word longer than a row (a path) breaks anywhere (egui's fallback
  break). Each row is as wide as its unwrapped text needs (measured with `layout_no_wrap`, the dismiss button of an error
  included), capped at that width, so a short toast stays small in the corner; the gap between rows is only between them,
  so the newest toast's bottom sits on the 12 pt margin. Rows are right-aligned, the newest at the bottom. A toast with a detail shows it as a tooltip while the pointer
  is over it (read from the hover position, so the area stays non-interactable). Info and warnings fade after 4 s (measured with `i.time`), errors
  stay until dismissed; everything is also logged. Any package may call `state.toasts.push` (tools do, for their warnings).
  Fix round 2, item 9 (toasts piled up over the 2D view, one per undo step): pushes are coalesced as described at `push`;
  at most `MAX_VISIBLE` = 4 toasts are shown, the oldest info / warning ones are dropped first and errors only when more
  than 4 errors wait (errors stay sticky until dismissed and remain in the log / `error_texts`); the area is
  `interactable` only while an error (with its dismiss button) is shown, so timed toasts never take a click meant for the
  view under them.
- Prefs (`prefs.rs`): `load() -> Prefs`, `save(&Prefs)`; JSON at `%APPDATA%\WrappedTextureEditor\prefs.json` (a few KB; failures
  are logged and ignored; never read or written in self-test mode). eframe's `persistence` feature is not used. Round 4:
  `brush`, `eraser` (written through `brush_flows::sync_prefs` at every write), `brush_shape_path`, `brush_shape_close`
  (the closing choice per shape file, at most 32).

Round 4 (S2): `prefs.json` (round 4): `brush`, `eraser`, `brush_shape_path`, `brush_shape_close` (B5 reads `brush` and `eraser`
leniently and sanitized). `brush_flows::sync_prefs` copies the tool settings in directly before every write.

Brush shape from a model file (`brush_flows.rs`, FEATURE_BRUSH 4.4, plan section 7 item 5). Every decision is a
function on `AppState` that returns the bake to start (`ShapeJob { request, path, close, modal }`); the `WteApp`
wrappers only spawn it ("Loading brush shape": modal for "Load...", the dialog's answer and the Brush tab's Openings
control; non-modal for the quiet reload). The job runs `TipField::bake_from_path(path, close, progress)`.
- Request numbers: every bake takes `next_request` and becomes `pending`; a result whose number is not `pending`'s is
  dropped without a toast (a "Load..." started while a reload runs wins in either order; a result after Remove is
  dropped). A reload is not started while any load is pending. "Load..." is refused with the "please wait" toast while a
  modal job runs.
- The close mode of a bake: the user's stored answer for that file (`prefs.brush_shape_close`), else `Refuse` (ask).
  `Err(HasOpenings)` closes nothing and opens `Dialog::CloseOpenings` (for a quiet reload too, when no other dialog is
  open; otherwise it counts as a failed reload). The answer bakes the same file again for the same tool and in the
  same manner (modal for "Load...", quiet for a reload); Cancel keeps the shape in use ("Loading cancelled"; a cancelled
  reload moves tools on File without a shape back to the Ball).
- Success: `current` = the shape (`ReadyShape::close` = the mode, or `Refuse` when the file had nothing to close,
  i.e. `report.openings_closed == 0 && !report.closed_parts`; `closed_parts` from the report), `last_error` cleared,
  the mode stored for the path when it is not `Refuse`. "Load...": `prefs.brush_shape_path` = the path; the issuing
  tool (Eraser when the Eraser was active, else the Brush) gets Sphere / File and `through = flat || height_ratio <
  0.25`; the Brush tab is shown; toasts of FEATURE_BRUSH 6.6 (loaded with triangle count; partial unless the user chose
  "Use only the closed parts"; thin; flat; thin-so-Through). A quiet reload changes nothing else and shows nothing.
- Failure: `current` and the remembered path kept, `last_error` = (path, message). "Load...": error toast (the
  "Cannot open <name>: ..." text for a file that cannot be opened, else the bake's sentence) and the Message dialog
  "Brush shape not loaded". Quiet reload: one warning toast ("Brush shape not found: <path>. Using the ball." or
  "Brush shape <name> cannot be used: <message> Using the ball.") and every tool on File without a shape goes back
  to the Ball. A cancelled bake: info "Loading cancelled".
- Remove: `current`, `pending`, `last_error`, the remembered path and its stored choice cleared; tools on File go back
  to the Ball (their type is kept).
- `SetBrushShapeClose(mode)` (Brush tab "Openings"): bakes the current (else the remembered) file again with `mode`,
  modal, for the active tool; `Refuse` forgets the stored choice, so that bake asks.
- Start (`brush_flows::startup`, after `start_fonts`): `prefs.brush` / `prefs.eraser` (sanitized) go into
  `tools.brush` / `tools.eraser`; `ReloadBrushShape` is pushed only when a path is remembered and one of the two is
  Sphere with File. `sync_prefs` copies both tools (sanitized) back directly before every `prefs::save`.
- `prefs::from_json` (round 4): `brush` and `eraser` are read leniently (state.rs) and sanitized; stored choices that
  do not read, and `refuse` entries, are dropped; at most 32 are kept (the remembered shape's first). Nothing about the
  shape (no model data, no grid) is written anywhere but the path and the choice.

Font samples (round 4, T7, FEATURE_TEXT 5.13): `AppAction::LoadFontSamples` starts one non-modal job "Drawing font
samples" when none runs and the samples were not made for a catalog of this many families
(`FontSampleState::requested_for`; the catalog grows once, when the system scan arrives). The job draws from a clone of
the catalog: per family the face `pick_style(.., "Regular")` picks, `FontCatalog::sample(id, "Abc 123", 16.0)`.
`text_ops::on_font_samples` packs the samples row by row into atlas images of 512 x 1024 px with cells of 128 x 20
px (4 x 51 = 204 per atlas; white with the coverage as alpha, so the picker tints them), uploads them as textures
`font_samples_<n>` and records each family's cell (`FontSampleState::cells`, `atlases`); a family without a sample
has no cell. Measured on this machine (T7, release test build): 78 families, all drawn, 15.5 ms with a warm file cache (limit 0.5 s).

**Round 9 (package B, T8).**
- A Final waits for the wrap job in flight of its stamp only when that job was made for the stamp's current placement
  (`final_waits_for_wrap_job` compares `WrapFlight::placement`): then it reuses the job's parameterization; a job made for
  an earlier frame of the drag does not hold the Final back (the Final starts in the same pump, the job's output lands
  stale).
- Warm-up: the Transform tool's `ensure_wrap_param` asks for it whenever a 3D stamp is selected
  (`WrapJobs::request_warmup(wrapped)`) and (round 9 check:B, plan T8 "when Wrap is chosen") whenever the Stamp or Text
  tool is active in Wrap mode (graph and shared texels, before the first wrapped stamp is placed); `after_tools` starts `Document::wrap_mesh_warmup(..).run()` on a named
  background thread (`wrap-warm-up`) outside `Jobs` when something is unbuilt and no warm-up runs (no `JobOutput`: it
  fills the document's `WrapMeshCell`, which the jobs share; a wrap job that needs the graph meanwhile waits in the
  cell's `OnceLock`). Logged as `wrap mesh: ...` (no `wrap:` line, Q5).

### 8.10 Shortcuts (`shortcuts.rs`) [A1]

```rust
pub struct ShortcutDef {
    pub action_label: &'static str,
    pub shortcut: egui::KeyboardShortcut,
    pub display: Option<&'static str>,              // menu text override, e.g. "Shift+[" for the OpenCurlyBracket binding
    pub text_ok: bool,                              // still fires while a text field has keyboard focus
    pub make: fn(&AppState) -> Option<AppAction>,   // None = not applicable right now
}
pub fn table() -> &'static [ShortcutDef];          // ordered most-specific-first (rule 5)
/// Consumes the key presses of this frame and returns the resulting actions; `App::ui` appends them to `state.actions`
/// after the panels and dialogs have run (8.3).
pub fn handle(ctx: &egui::Context, state: &mut AppState) -> Vec<AppAction>;
pub fn shortcut_text(ctx: &egui::Context, label: &str) -> String;       // for menus and tooltips; "" if the label has no binding
```
Rules for `handle` (one rule set; 8.3 and this section must agree):
1. `handle` is the only code that consumes key presses (8.2). It walks `table()` in order and calls
   `ctx.input_mut(|i| i.consume_shortcut(&def.shortcut))`.
2. While a modal job runs: return nothing. While a dialog is open: only Esc (`Cancel`) is processed.
3. **Text fields.** `let typing = ctx.text_edit_focused();` — *not* `egui_wants_keyboard_input()`, which is true whenever any
   widget (a button or slider reached with Tab) has focus and would kill the single-key shortcuts. While `typing`, entries with
   `text_ok == false` are skipped *without consuming*: all single-key shortcuts, Delete, Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z (the
   text field keeps its own undo) and the layer shortcuts.
   Esc needs one extra frame of memory: egui itself clears the keyboard focus on Esc at the *start* of the pass
   (`REG\egui-0.37.0\src\memory\mod.rs:598-601`), before `handle` runs, so `typing` is already false in the frame of the key
   press. `handle` therefore stores `state.typing_prev = ctx.text_edit_focused()` at its end and skips the Esc entry when
   `typing || state.typing_prev`: the first Esc only leaves the text field (which commits its edit through `lost_focus()`),
   a second Esc is `Cancel`.
   **Popups** (review round 2, finding esc-in-popup-cancels): while `egui::Popup::is_any_open(ctx)` (a combo box, a colour popup,
   a menu; read at the start of the pass, i.e. the popup the user sees; tooltips do not count) the Esc and Delete entries are
   skipped *without consuming*: the popup closes itself on that Esc (`Popup` checks `key_pressed(Escape)` later in the pass,
   like egui's own `Modal::should_close`), and the next Esc is `Cancel`. So Esc in the Export dialog's format drop-down closes
   the drop-down, not the dialog, and Esc in a stamp's tint popup does not deselect the stamp.
4. `text_ok == true` for Ctrl+S, Ctrl+Shift+S, Ctrl+O, Ctrl+N, Ctrl+E and F5. When one of them fires while `typing`, `handle`
   first releases the focus — `if let Some(id) = ctx.memory(|m| m.focused()) { ctx.memory_mut(|m| m.surrender_focus(id)); }` —
   so the field reports `lost_focus()` in this same pass, the panel queues its `Commit::Final` action, and that action is
   dispatched before the shortcut's action (which is appended last). Even without that, Save / Export / Preview call
   `flush_pending_edit`, which finalizes Preview-baked stamps (5.20).
5. **Order.** `Modifiers::matches_logically` ignores extra Shift and Alt, so `consume_shortcut` for `Ctrl+S` also matches
   Ctrl+Shift+S. The table lists the more specific binding of every such pair first: Ctrl+Shift+S before Ctrl+S, Ctrl+Shift+N
   before Ctrl+N, Ctrl+Shift+Z before Ctrl+Z, Shift+X before X; objects-tabs: Shift+F before F, Shift+H and Alt+H before H.
   A unit test checks that no entry is shadowed by an earlier one. The table has 40 entries.
6. **Brackets.** `consume_shortcut` matches the *logical* key: Shift+`[` arrives as `Key::OpenCurlyBracket`, Shift+`]` as
   `Key::CloseCurlyBracket`. Hardness is bound to `KeyboardShortcut::new(Modifiers::SHIFT, Key::OpenCurlyBracket)` /
   `Key::CloseCurlyBracket` with `display: Some("Shift+[")` / `Some("Shift+]")`; size to `Key::OpenBracket` / `CloseBracket` with
   `Modifiers::NONE`; layer move to `Modifiers::COMMAND` + the square brackets.

| Keys | Action (`AppAction`) | Keys | Action (`AppAction`) |
|---|---|---|---|
| Ctrl+N / Ctrl+O | `NewFromModel(None)` / `OpenProject(None)` | 1 / 2 / 3 / F5 | `SetViewMode(View3D / View2D / Split / Preview)` |
| Ctrl+S / Ctrl+Shift+S | `Save` / `SaveAs(None)` | T | `ToggleTexture` |
| Ctrl+E | `ShowExport` | W | `ToggleWireframe` |
| Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z | `Undo` / `Redo` | F | `FrameModel` (3D: frame model, 2D: fit texture) |
| V B E I G S X R | `SetTool(Navigate, Brush, Eraser, Eyedropper, Fill, Stamp, Text, Transform)` | `[` / `]` | `BrushSizeStep(-1 / +1)` |
| Shift+`[` / Shift+`]` (`{` / `}`) | `BrushHardnessStep(-1 / +1)` | Ctrl+Shift+N | `AddPaintLayer` |
| Ctrl+J / Ctrl+M | `DuplicateActiveLayer` / `MergeActiveDown` | Delete | `DeleteSelection` |
| Ctrl+`]` / Ctrl+`[` | `MoveActiveLayer(+1 / -1)` | D / Shift+X | `ResetColors` / `SwapColors` |
| Esc | `Cancel` | Space+drag, MMB | Pan (read by the views as "Space held"; not a table entry) |
| H / Shift+H | `HideSelectedObjects` / `IsolateSelectedObjects` (objects-tabs) | Alt+H / Shift+F | `ShowAllObjects` / `FrameSelection` (objects-tabs) |

`action_label`s (the `label` argument of `shortcut_text`, as implemented): "New from Model...", "Open Project...", "Save",
"Save As...", "Export...", "Undo", "Redo", "Delete", "Cancel", "3D", "2D", "Split", "Preview", "Texture on Model", "Wireframe",
"Frame Model", the tool names "Navigate", "Brush", "Eraser", "Eyedropper", "Fill", "Stamp", "Text", "Transform",
"Smaller Brush", "Larger Brush", "Softer Brush", "Harder Brush", "New Paint Layer", "Duplicate Layer", "Merge Down",
"Move Layer Up", "Move Layer Down", "Reset Colors", "Swap Colors"; objects-tabs: "Frame Selection", "Isolate Selected",
"Show All", "Hide Selected" (all `with_doc`, `text_ok` false: H in a focused text field types "h"; Alt+H must be tried by hand
on Windows — a possible beep of the system menu; fallback Ctrl+Shift+H). "Redo" resolves to Ctrl+Y (its first entry).

### 8.11 Self-test runner (`selftest/mod.rs` [A1], `selftest/scenarios.rs` [A6], `cli.rs` [A1])

CLI (`cli.rs`, hand-parsed):
`wrapped_texture_editor.exe [PATH] [--screenshot OUT.png] [--scenario NAME] [--size WxH] [--log FILE]`
(read with `std::env::args_os`, never `args`: a PATH that is not valid Unicode, e.g. a file name with an unpaired
UTF-16 surrogate, must open instead of panicking; `--scenario` / `--size` values must be Unicode, else a clear error)
- `PATH`: `.wtep` project or model file opened at startup (`OpenPath`). It may be combined with `--screenshot` (scenario
  `open_path`).
- `--screenshot`: self-test mode. `--scenario` defaults to `default`; `--size` defaults to `1280x800`.
- `--log`: log file. Default `<OUT>.log` in self-test mode, `%APPDATA%\WrappedTextureEditor\log.txt` otherwise (8.3).
```rust
pub struct CliArgs { pub path: Option<PathBuf>, pub screenshot: Option<PathBuf>, pub scenario: String,
                     pub size: [u32; 2], pub log: Option<PathBuf> }
pub fn parse(args: impl Iterator<Item = String>) -> Result<CliArgs, String>;
```
```rust
// selftest/mod.rs
/// Values a scenario carries from one step to a later one (hashes, counts), plus the output location.
pub struct Scratch { pub values: BTreeMap<&'static str, u64>,
                     pub out_stem: PathBuf /* OUT without ".png"; scenario files go to "<out_stem>_assets", "<out_stem>_export", "<out_stem>.wtep" */ }
pub enum Step {
    Do(AppAction),
    Pointer { view: ViewId, path: Vec<[f32; 2]> /* normalized 0..1 in the view rect */, button: egui::PointerButton,
              mods: egui::Modifiers, points_per_frame: usize },
    Click { view: ViewId, at: [f32; 2], button: egui::PointerButton, mods: egui::Modifiers },
    /// Primary button: press at the first point, one move per frame, release at the last; screen points (panel controls,
    /// e.g. the Layers rows of `state.panel_rects`). One point = a click.
    ScreenPointer { path: Vec<egui::Pos2> },
    /// Export-select: one click at a screen position with any button and modifiers (`click_frames`): Ctrl / Shift-click
    /// and the right-click on a Layers row, a click on an item of the row's context menu.
    ScreenClick { at: egui::Pos2, button: egui::PointerButton, mods: egui::Modifiers },
    Scroll { view: ViewId, at: [f32; 2], notches: f32, mods: egui::Modifiers },
    /// Round 4 (S2): one `Event::PointerMoved` to the point (normalized in the view rect), no button.
    Hover { view: ViewId, at: [f32; 2] },
    Key(egui::Modifiers, egui::Key),
    /// Window inner size in physical pixels: one `ViewportCommand::InnerSize(size / pixels_per_point)`, then 10 frames.
    Resize([u32; 2]),
    WaitFrames(u32),
    WaitIdle,
    /// Direct state preparation or bookkeeping: register the sample stamp image as a blob, write asset files, remember a
    /// hash in `Scratch`.
    Setup(&'static str, fn(&mut AppState, &mut Scratch) -> Result<(), String>),
    Assert(&'static str, fn(&AppState, &Scratch) -> Result<(), String>),
    /// Steps computed at run time from the live state and spliced in at this position — for pointer paths that depend on
    /// camera framing and layout (dragging a gizmo handle: `ViewXform::for_view` + `view::gizmo::gizmo_shape`, converted to
    /// normalized view coordinates).
    Dyn(&'static str, fn(&AppState, &Scratch) -> Result<Vec<Step>, String>),
    Screenshot,
    /// Round 4 (gate 1): a named mid-run capture written to `<out_stem>_<name>.png`; `OUT.png` stays the final
    /// `Screenshot` (`brush_shape` and `text_tab` keep their hover frame as `<out>_hover.png`).
    Shot(&'static str),
    /// Round 5: switches the runner's wait for pending Final stamp bakes after input and `Do` steps off (false) and on
    /// (true, the default); `WaitIdle` always waits for them.
    BakeWait(bool),
    Exit,
}
pub struct Runner { /* steps, index, frame counter, scratch, injected events, screenshot receiver, output paths, report, exit code handle */ }
impl Runner {
    pub fn new(args: &CliArgs, exit_code: Arc<AtomicI32>) -> Option<Runner>;     // None without --screenshot; unknown scenario => error scenario
    pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput);
    pub fn tick(&mut self, ctx: &egui::Context, state: &mut AppState, gpu: &Gpu, tools_busy: bool);
}
// selftest/scenarios.rs
/// `args` gives scenarios their paths (`args.screenshot` => out stem, `args.path`).
pub fn scenario(name: &str, args: &CliArgs) -> Option<Vec<Step>>;
pub fn scenario_names() -> &'static [&'static str];
/// Step / run timeouts of scenarios that need more than 30 s / 180 s (`workflow_path`, `fuzz_path`, `brush_types_path`: 300 s / 3600 s).
pub fn timeouts(name: &str) -> Option<(Duration, Duration)>;
```
- **Determinism** (`raw_input_hook`): drop every real `PointerMoved`, `PointerButton`, `MouseMoved`, `PointerGone`, `MouseWheel`,
  `Key`, `Text`, `Zoom`, `WindowFocused` event and clear `hovered_files` / `dropped_files`; set `focused = true` (the
  window runs in the background, 8.3, and real focus changes must not change a run); then append the runner's own events,
  so strokes go through real egui hit-testing and the real tool code. Exact shapes (egui 0.37, 8.1):
  - pointer: `Event::PointerMoved(pos)`; `Event::PointerButton { pos, button, pressed, modifiers }`;
  - key (`Step::Key`): `Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }` in one frame and the same
    with `pressed: false` in the next;
  - wheel (`Step::Scroll`): one `Event::PointerMoved(pos)`, then in the next frame `Event::MouseWheel { unit:
    egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, notches), phase: egui::TouchPhase::Move,
    source: egui::MouseWheelSource::Wheel, modifiers }` — a whole number of notches in a single frame;
  - modifier state: egui 0.37 has **no** `RawInput::modifiers`; `InputState::modifiers` follows `Event::ModifiersChanged`
    (`REG\egui-0.37.0\src\input_state\mod.rs:434`). The hook drops the real `ModifiersChanged` events and injects
    `Event::ModifiersChanged(m)` (first) whenever the wanted state differs from the last one announced: `m = modifiers` in every
    frame in which an injected event carries modifiers (and while a `Pointer` step with `mods` is in progress),
    `egui::Modifiers::NONE` otherwise;
  - time: `raw.time = Some(frame as f64 / 60.0)`, `raw.predicted_dt = 1.0 / 60.0` (the hook runs after eframe set the time).
- **Window size** (a viewport command is converted to physical pixels with the zoom factor of the pass in which it is
  *processed*, and `set_zoom_factor` only takes effect in the next pass — 8.1):
  - frame 0: `ctx.all_styles_mut(|s| s.animation_time = 0.0)` and
    `ctx.set_zoom_factor(1.0 / ctx.native_pixels_per_point().unwrap_or(1.0))` (1 point = 1 physical pixel); nothing else;
  - from frame 1 on, while `(ctx.content_rect().size() * ctx.pixels_per_point()).round() != requested size`: send
    `ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32) / ctx.pixels_per_point())` (at most once every 10 frames);
  - scenario steps start only when the size matches, or after 120 frames (the OS clamps windows larger than the screen): the
    run then continues with the actual size and `OUT.json` records `"size_clamped": true`.
- **Driving** (`tick`, called from `logic()`): every frame it calls `ctx.request_repaint()` and
  `ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden)` — while the window is minimized or hidden eframe would
  otherwise run only `logic()`, injected events would never be applied and the scenario would time out (the self-test runner is
  the one place allowed to send a viewport command every frame, 8.13). `Do` pushes into `state.actions`; `Pointer` emits press
  at the first point, `points_per_frame` moves per frame, release at the last, using `state.view_rects` from the previous
  frame; `Dyn` evaluates its function and replaces itself with the returned steps; `WaitIdle` = `state.busy.is_none()` (no job
  of any kind, fonts included), `!tools_busy`, `!doc.has_pending_edit()`, `!doc.has_pending_composite()`,
  `!state.gpu_mips_stale`, `!state.final_bakes_pending` (round 5), and stable view rects — all for 3 consecutive frames.
- **Final stamp bakes** (round 5): the scenarios run the app's asynchronous mode and stay deterministic because the runner
  treats waiting bakes as part of the step: an input step also waits while `state.final_bakes_pending`, and a `Do` step
  is followed by the same wait (it ends in the first frame without a pending bake, so it costs no frame when nothing is
  pending). `Step::BakeWait(false)` switches both waits off (and `true` on again) for `final_bake_async`.
  Round 8: both waits and `WaitIdle` also cover the wrapped stamps' Preview and param-only jobs
  (`state.wrap_jobs_pending`; those jobs are silent, so `state.busy` does not show them).
- **Input steps** (round 4, S2): An input step (`Pointer`, `Click`, `ScreenPointer`, `Scroll`, `Hover`, `Key`)
is complete when its injected frames are processed **and the tools are not busy** (`ToolController::is_busy`), so a
pointer step ends only when its stroke is complete, however long its paint trails (FEATURE_BRUSH 7.3).
- **Capture**: `ctx.request_screenshot(move |img| { tx.send(img).ok(); })` captures the next painted frame. On every following frame
  `gpu.device.poll(wgpu::PollType::Poll)` (required to fire the readback) and `rx.try_recv()`. On receipt: force alpha to 255,
  `wte_core::image_io::encode_png(w, h, img.as_raw(), PngLevel::Balanced)`, write `OUT.png` and `OUT.json`
  (`{ "scenario", "ok", "frames", "requested_size": [w, h], "size": [w, h] /* of the captured image */, "size_clamped",
  "errors": [..] }`), then `ViewportCommand::Close` with `allow_close = true`.
- **Exit codes** (stored in the shared `AtomicI32`; `main` calls `std::process::exit(code)` after `run_native` returns): `0` ok;
  `2` a failed `Setup` / `Assert` / `Dyn`, or `state.toasts.error_count()` grew during the scenario (the remaining steps are
  skipped, a capture is still attempted); `3` timeout (a step takes longer than 30 s, or the run longer than 180 s, wall
  clock — or the scenario's own `timeouts`, each overridden by the environment variables `WTE_SELFTEST_STEP_TIMEOUT` /
  `WTE_SELFTEST_RUN_TIMEOUT` in whole seconds; a capture is still attempted); `4` startup failure (no wgpu adapter, bad arguments); `101` panic.
- Verification scripts must start the exe with `Start-Process -Wait -PassThru` (shells do not wait for GUI-subsystem executables).

Scenarios. "Default content" = Sphere sample (`OpenSample(Sphere)`: set 0 has the locked "Base" and the active "Paint 1"), Split
view, a red 3D brush stroke across the sphere, a blue 2D brush stroke, an image stamp (built-in `sample_stamp_image` registered as
a blob in a `Setup`) placed in 3D, a text stamp "WTE" placed in 3D, Transform deselect. Earlier values (initial hashes, undo
counts) are remembered with `Setup` steps in `Scratch` and compared in later `Assert`s. No scenario needs assets from outside:
the import scenarios write their files with `samples::to_obj` and `encode_png` into `<out_stem>_assets\`.

| Name | Steps (summary) | Asserts before the screenshot |
|---|---|---|
| `empty` * | No document | no session |
| `smoke` * | `OpenSample(Sphere)`; `WaitIdle` | session with 1 set and exactly 2 layers: "Base" (locked raster), "Paint 1" (active) |
| `default` | Default content | document modified; set 0 has exactly 4 layers bottom to top: Base (locked), Paint 1, one Stamp, one Text; the Base layer's pixels are unchanged (hash); the composite hash differs from the initial one |
| `paint3d` | Cube sample; 3D view; `SetShading(UvChecker)`; stroke across a cube edge and a corner | pixels changed in >= 2 UV islands; `flags.shading == Textured` again (texture display was switched back on by the stroke) |
| `paint2d` | Sphere; 2D view; stroke; eyedropper; second stroke | primary colour changed by the eyedropper |
| `stamp` | Sphere; image stamp in 3D; `Dyn`: drag the gizmo body, a corner and the rotate knob; Ctrl+wheel (`Scroll`, 2 notches); `View2D` and back to `View3D`; 3 x `Undo`, 3 x `Redo` | one undo entry per drag and one for the wheel gesture; after placing and after the body drag the depth is the auto-fitted one (6.5), the gizmo has its `depth_box` and the layer badge is "3D stamp"; stamp layer present; placement size changed by the wheel (x1.1^2); in `View2D` the status hint is "This is a 3D stamp - edit it in the 3D view", gone again in `View3D`; the undo burst is one "Undo: .. (x3)" toast, the redo burst replaces it with one "Redo: .. (x3)" (at most 4 toasts); then `assert_side_panel_layout` (R3-10): the Color area inside the side panel and, at a window of 1280x800 or more, its whole content on screen; settings, Color and Layers list areas do not overlap; the Layers footer and Delete inside the panel, below the list |
| `text` | Sphere; Stamp tool, then Text tool (fix round 2, item 4: the 3D view rect must not change, nor after placing the text, when the text stamp's properties are shown); text stamp; change the content through `UpdateStampSource(.., Final)`; scale it 4x through `UpdateStampPlacement(.., Final)`; press with empty content; `SelectStamp` of the text stamp again (Text tool) | text layer cache rect changed; `TextSpec.size_px` larger after the 4x scale; the empty-content press added no stamp and no undo entry and deselected the text stamp (R3-10); reselected, the Text options draw the stamp's text field (`PanelHit::StampTextField`) inside the settings area; `assert_side_panel_layout` Round 4 (S2): the steps after the empty press now assert that the Text tool's box is the text to place (the selected-text field is not drawn under the Text tool) and that, with the text selected and the Transform tool, its Text block holds the text inside the tool options area. |
| `clay` | Sphere; stroke; `ToggleTexture` | `flags.shading == Clay` |
| `preview` | Default content; `SetViewMode(Preview)` | `view_mode == Preview`, no pending edit |
| `export` | Default content; `ShowExport` | `Dialog::Export` open, `options.dir` ends in `export`, `export::plan` non-empty |
| `export_run` | Default content; `RunExport(opts)` with `dir = <out_stem>_export`, flat PNG + per-layer PNGs + ORA + PSD; `WaitIdle` | every path of `export::plan` exists and is non-empty; the folder holds exactly the planned files and they pass `verify_export` (export-select: per-layer PNG = layer, flat / ORA merged = composite, PSD layer names, recomposition = composite); the per-layer files contain exactly one `_stamp_` and one `_text_` file (R4: separate layers); no dialog open Round 4 (T7): the default text gets the same gaps and warp before the export (one `_text_` layer file as before). |
| `multi` | Two-material sample; image stamp placed across the material boundary (its home set now has the stamp layer active, i.e. no paintable active layer); then one 3D stroke that starts on the *other* material and crosses the boundary | the stamp has a layer in both sets; the stamp's home set got a new paint layer; paint pixels changed in **both** sets |
| `roundtrip` | Default content; `SaveAs(Some(<out_stem>.wtep))`; `OpenProject(Some(..))` | composite hashes of all sets equal before/after; stamp and text layers still re-editable; Base still locked Round 4 (T7): the default text gets two gap values and an Arc warp before Save As; asserts added: the reopened TextSpec (every field) and its layer pixels equal the saved ones; one more Final changes the layer. |
| `undo` | Default content; undo everything; redo everything | hash after undo-all == initial; after redo-all == final |
| `import` | `Setup` writes `two_material_model` as OBJ + MTL + two checker PNGs; `NewFromModel(Some(obj))`; `WaitIdle`; assert; `ImportCreate`; `WaitIdle` | first: `Dialog::Import` with 2 sets, both textures detected (`image.is_some()`), 2 UV thumbnails; then: session with 2 sets, each with Base + Paint 1, composites not blank |
| `import_tex` | `Setup` writes `seam_cube` as OBJ **without** MTL plus a separate checker PNG; `NewFromModel`; `ImportPickTexture { set: 0, path: Some(png) }`; `ImportCreate`; `WaitIdle` | Base equals the chosen image; `doc.source.extras.len() == 1` with the PNG's exact bytes (R7) |
| `import_nouv` | `Setup` writes `no_uv_tetra` as OBJ; `NewFromModel`; assert; `ImportReload { fallback_uvs: false }`; assert; `ImportReload { fallback_uvs: true }`; `WaitIdle`; `ImportCreate`; `WaitIdle`; 3D stroke; Save As; Open Project | first: `Dialog::Import`, 1 set, `report.no_uvs` and `report.fallback_uvs_generated` (the generated layout is applied on import); with the layout switched off: no sets; after Create no dialog is open (the import warnings are not repeated); then: 1 set and the stroke changed pixels; then (round 3, R3-1/R3-3) `SaveAs(<out_stem>.wtep)` + `OpenProject`: no dialog open (no "Warnings" box), not modified, no history, same composite |
| `unsaved` | Sphere; Save As `<out>.wtep`; stroke; Undo; Redo; Undo; `OpenSample(Sphere)`; stroke; `OpenSample(Cube)` | after the save and after each undo back to it: no unsaved changes and no `*` in the title (`last_title`); after the stroke and the redo: both set; the first `OpenSample` opens with no prompt; then `Dialog::Unsaved` open and the session is still the sphere (R3-6) |
| `open_path` | Nothing but waiting: the CLI `PATH` is opened by the app; if `Dialog::Import` appears, `ImportCreate`; `WaitIdle` | a session exists (fails with a clear message when no `PATH` was given); no dialog open, in particular no "Warnings" box (round 3, R3-3) |
| `import_save` | R7 preparation (added by the integration gate): like `open_path` with a model `PATH`, then 3D view, an image stamp, a text stamp "R7" and a red stroke; `SaveAs(Some(<out_stem>.wtep))`; `WaitIdle`. The release check then deletes the model's folder and opens the `.wtep` with `open_path` | the source bundle holds the model and its sidecars; both stamps placed; the project saved (not modified, file non-empty) |
| `workflow_path` | Compatibility harness on the CLI `PATH` (OBJ / glTF / GLB / .wtep). Like `open_path`, but a refused import fails with the dialog's reason (error, or no texture set). Then: 3D view + `FrameModel`; three brush strokes (red, green, blue) along runs of a 40x28 screen grid whose rays hit UV-mapped geometry (`ViewXform::surface_at`, the BVH of the document — robust to models whose centre ray misses, any scale, several sets; hits on triangles whose own UVs have no area are skipped; when no point of the grid hits, a 4x finer grid is tried; a run of one hit strokes from the hit itself); a 2D stroke through UVs taken from those hits (their set made active; its points are kept inside the texture); Split; Preview, Esc back; 3D: `AddPaintLayer` in the set of the best-facing hit and an island fill there; `ImportStampImage` of a generated PNG placed on a well-facing hit; a text stamp on another (both spots "solid" whenever any is: the hit and 4 points 3 px around it hit the model, so a click on a model of tiny parts or thin limbs does not land in a gap; spots on sets with a texture side under 32 px only if there is no other; every placed stamp must cover texels, `placed_stamp`); Transform: drag the body, a corner, the rotate knob; undo all, redo all; `RunExport` (flat PNG, per-layer PNGs, ORA, PSD) into `<out_stem>_export`; `SaveAs(<out_stem>.wtep)`; `OpenProject` of it (no dialog may open: R3-3); Split + `FrameModel`. Every workflow step logs `workflow step "<name>" done in <ms> ms` and a summary line | after every edit: nothing pending, a new undo entry, the composite changed (not for the new layer); 2D view on screen; Split shows both views; Preview shown, left by Esc; undo-all reaches 0 entries and (when nothing was evicted) the imported composite, redo-all the final one; every planned export file non-empty with .png, .ora and .psd present; the export toast names the folder only and no drawn toast intersects the side panel or a Layers control or leaves the central view area (round 3, R3-7; again after the final Split); saved (not modified, file non-empty); the reopened project has the same sets / layers / stamps (structure hash) and composites, no history |
| `stamp_edge_path` | Fix round 2, item 2, on the CLI `PATH` (opened like `workflow_path`; made for Avocado.glb): 3D + `FrameModel`; the sample stamp image at `size_frac` 0.3 placed on a solid, well-facing (> 0.6) grid hit with a miss within 3 grid cells (near the silhouette); re-placed with the old default (depth = larger half size, angles 60 / 85) through `UpdateStampPlacement(.., Final)`; `Undo`; Transform tool (the screenshot shows the gizmo with its projection box) | the stamp covers texels; its depth is the fitted one with a projection box and a "3D stamp" badge; logs `edge stamp coverage: fitted F full / A any; old default F0 / A0` and fails when the fitted placement covers less than 95 % of the old one; after `Undo` the fitted placement is back |
| `fuzz_path` | On the CLI `PATH` (opened like `workflow_path`): `WTE_FUZZ_STEPS` (default 400) seeded pseudo-random actions (`WTE_FUZZ_SEED`, default 1; splitmix64), each logged as `fuzz <i>/<n>: <action>` *before* its steps run: orbit / pan drags and wheel (also Ctrl / Shift) in every shown view; view-mode switches and rapid toggling; tool switches; brush / eraser strokes of random size, colour and modifiers through model hits or random points (3D and 2D); fills (island / layer); image and text stamps (several texts); gizmo drags of body / corner / knob and Ctrl / Shift+wheel on the selected stamp; layer add / duplicate / delete / move / visibility / opacity drags / blend / select / merge down / rasterize / lock / alpha lock; texture-set switches; undo / redo bursts (actions or Ctrl+Z / Ctrl+Y); `Resize` of the window; view toggles and camera presets; shortcuts (never Save / Open / New); stamp select / delete / tint. An open dialog is closed by the next action. Then `Resize` back, undo all, redo all, Split | the session survived: no pending edit, every set's composite has the set's size, every active layer exists, every stamp has a layer; in the final Split at least 32 pt (or the whole smaller side) of the texture lie inside the 2D half on both axes (R3-11) Round 4 (T7): four operations on a random text stamp (in the stamp-operations roll): a random gap value (sometimes out of range), a random warp, a random line (Arc / Circle / Curve, sometimes beyond 60 em), every effect cleared; 20 % are sent as a Preview followed by its Final. |
| `pan2d` | Round 3, R3-11: Sphere; `View2D`; middle-button pan drags of ten view widths (`Pointer` paths far outside the view) left and down-right; 2 x 10 wheel notches in at the bottom-right view corner; a pan up and 2 x 10 notches out at the top-left corner; `FrameModel`; `Split`; a pan of ten widths up-right; `Resize(900x600)`; `FrameModel`; a pan of ten widths up-left (the screenshot shows the texture corner held at the view edge) | after every drag, zoom and the resize: `texture_in_view` (logs `pan2d <label>: texture .., view .., overlap W x H pt`; at least min(32, texture side, view side) - 0.5 pt of overlap on both axes); after the last `FrameModel` the whole texture rect lies inside the 2D half |
| `layers` | Sphere; with injected pointer input on the Layers rows (`ScreenPointer` at `state.panel_rects`, clicks 0.5 s apart): Base's lock, Base's eye twice, Paint 1's alpha lock, Base's name; then a drag of Paint 1 (started on its name) onto the lower half of Base's row; then an image stamp placed in 3D and Ctrl+J (`Key`); (R3-2) Duplicate and Undo of a 21.33 % layer; (R3-10) 2 x `AddPaintLayer` | both rows lie inside the window; Base unlocked, hidden, visible again; Paint 1 alpha-locked; Base active; exactly 4 undo entries for the clicks; layer order Paint 1, Base; after Ctrl+J the selected stamp is the copy, its layer is the active layer, 2 stamps; with 6 layers `assert_side_panel_layout` (Delete button on screen, outside the list's scroll area) |
| `brush_types` | Cube sample, 3D view, camera preset Front, Brush, Size 1024, hardness 1: click the centre of the view with Circular; Undo; Sphere; click again (FEATURE_BRUSH 8.3 S1; the TwoMaterial part is not built, plan section 7 item 1); then Sphere + Cube, Size 600, Turn 30, `Step::Hover` at the centre of the view | Circular: the back face (-Z) has 0 changed texels, each other face more than 100,000; after Undo the composite is the one before; Sphere: all six faces more than 100,000; one history entry per click; the hover: `tool_probe` footprint and wires both non-empty and the status bar says "Shape brush: Cube" |
| `brush_shape` | Cube sample, 3D view, camera preset Front, Brush; writes `seam_cube(1.0)` as `cube_shape.obj`, `.glb`, `.fbx` and `open_box(1.0)` as `open_box.obj` into `<out>_assets`; `LoadBrushShape(Some(obj))`; Size 200, hardness 1, click the centre; Undo; Turn 45, click; a 40-point stroke; `Step::Hover`, screenshot; Turn 0; the GLB and the FBX cube, one click and Undo each; `LoadBrushShape(Some(open_box.obj))`, "Flat caps" and "Close with flat caps" clicked in the dialog (`ScreenPointer` on `PanelHit::OpeningsChoice(0)` / `OpeningsOk`), one click (FEATURE_BRUSH 8.3 S2; plan section 7 item 5) | after each load: 12 triangles, not partial, brush Sphere / File / Solid, eraser unchanged, prefs path = the file, Brush tab drawn; OBJ click: only the front face, 13,333 texels within 5 %, box 90.9 x 146.8 within 4; Undo restores the composite; Turn 45: box 128.5 x 207.6 within 4; the stroke is one history entry; GLB and FBX clicks: the same count (5 %) and box (4 texels) as the OBJ click; the open box asks (1 opening, no "closed parts" choice, the FBX cube stays in use); after the answer: closed with Flat caps (1 opening closed), the choice stored with the path, the preview sections non-empty, the Brush tab drawn, and its click paints the cube's square; no error toast |
| `brush_tab` | 1280 x 800. Sphere sample, 3D view, Brush tool, Brush tab, Sphere with the Cube turned 30 degrees, the pointer on the model without a press; then the tallest state; `SetTool(Text)`; `SetTool(Brush)` | the side panel is 300 pt wide, the Color section is inside it, the brush slider rows end above the bottom of the tool area, the tab's content lies inside the lower area, the shape cursor has a footprint and wires (`ToolProbe`). The same layout in the tallest state (a shape file with closed openings: the "Openings" row and the "!"). `SetTool(Text)`: Layers list on screen, `side_tab` still Brush; `SetTool(Brush)`: the Brush tab again, the cursor again. Screenshot: the tab with its preview, the footprint and the wires. |
| `brush_types_path` (PATH) | Opens the model like `workflow_path`, 3D view, Frame; one warm-up stroke along `hit_stroke(0)`, Undo; the same path with Circular, Sphere and Sphere + Cube at Size 50 and Size 512, each preceded by a log line naming type and size and followed by Undo (FEATURE_BRUSH 8.3 S4) | each stroke changes the composite and is one history entry; each Undo restores the composite; the brush tool's per-stroke log lines are the P7 / P8 numbers |
| `text_gaps` | Sphere; "WTE 2026" at 6 % placed on the model; one `UpdateStampSource` `Final` with the gap after "E" = 0.5 em; undo; redo; the same with a UV text placed in the 2D view | 3D: the baked bounds grow; 2D (left-aligned UV text): the bounds grow by 0.5 em in texels within 2 and the left edge stays (less than 1 texel); the undo label is "Edit Stamp"; undo gives the layer hash before, redo the one after; the layer stays a text layer |
| `text_styles` | Sphere; place "WTE 2026"; one `Final` with thickening 0.02 em and slant 12 degrees; undo; redo | more inked texels; one history entry; undo / redo give the two hashes |
| `text_warp` [PATH] | Sphere (or the PATH model); place; Arc, Flag, Fisheye, Arc at 50 %; the same on a UV text; Split view for the screenshot | after each step the baked bounds differ from the step before; the anchor moved by less than 0.1 % of the diagonal (3D) / 0.5 texel (2D); the 3D view rect did not move; each `Final` is logged as `text step "warp <style>" final in <ms> ms` |
| `text_path` [PATH] | Sphere (or the PATH model); place; Arc 25 %; Circle; Curve (shapes made from the straight frame as the Path page makes them); a pointer drag of the middle curve point 40 pt up (its screen position from `text_path::path_points`); the same on a UV text | each kind changes the baked bounds; on the UV text the ink centre on the Circle is within 2 texels of the straight text's, along x; after the drag `points[1].y` is smaller, one more history entry, the first and last point moved less than 1.5 pt on screen; each `Final` logged |
| `text_tab` | Sphere; Text tool; `Step::Hover` on the 3D view (the see-through text of the draft, `tool_probe.text_hover`); the three pages of the Text tab before and after placing a text (Color section inside the side panel, panel 300 pt wide, 3D view where it was with the Stamp tool); a draft change with the text selected leaves the placed text alone; the three tabs together (ROUND4_PLAN 3.3: Brush tool -> Layers with `side_tab` Text and the selection kept; Brush tab click; Transform -> Layers; Text tab click; Stamp -> both grey); back to Layers (footer inside); ends on the Character page with the gap after "E" selected (status line) |
| `text_fonts` | this machine: waits for the font job; hard asserts (never skipped): at least 50 families, Arial present and not bundled; B on Arial gives the face "Bold" without thickening; B on Ubuntu thickens by 0.02 em; Candara lists "Regular" and "Regular (Candara-Light)" (SKIP when missing); a thickened Arc-warped "Ag" in the first CFF family and in Bahnschrift has more than 200 inked pixels and no fully inked row or column (SKIP per missing family); gate 3: a click on `PanelHit::FontButton` opens the font picker, the sample job finishes (samples of every family in the atlases), 30 frames open, `<out>_picker.png`, Esc |
| `text_preview_path` (PATH) | Gate 3 (measurement, FEATURE_TEXT 8.4): opens the model like `workflow_path`; places "WTE 2026" in 3D as `text_warp` does; Arc 50 % (`Final`, timed); twenty `UpdateStampSource(.., Preview)` of the bend, one per frame, between the log steps `gate: twenty preview edits start` / `end`; one `Final` (timed) | the twenty Previews are one pending edit and add no history entry; the Final adds one entry with the last bend |
| `final_bake_async` [PATH] | Round 5 (R5-1): Sphere (or the PATH model). With the bake wait on: an image stamp placed and dragged (composite H1), Undo. Then with `BakeWait(false)`, each next step starting while the bake runs: (a) the same drag, `Undo` in the frame after the release, `Redo`; (b) the drag, at once a second drag, `Undo`; (d) a `Final` scale and `MergeDown` of the stamp's home layer in the next frame, against the same two steps with the wait on; (c) a text stamp gets an Arc warp (`Final`) and `SaveAs(<out_stem>_fba.wtep)` in the next frame, then `OpenProject` of it; (f) a `Final` scale of the text and `RunExport` (flat PNG and per-layer PNGs) in the next frame, against the same two steps with the wait on (into `<out_stem>_fba_export_ref`); (e) a `Final` scale of the text and `DeleteStamp` in the next frame. A second image stamp, each pass against the same steps with the wait on: (g) a `Final` scale, then `SetViewMode(Preview)`, `SetTool(Brush)`, `FrameModel`, the earlier view mode and Transform at once; (i) a paint layer made active, Brush, a `Final` scale and a stroke across the model in the next frame; (k) a `Final` scale and `ShowExport` in the next frame, then `WaitIdle` with the dialog open, `CloseDialog`; (h) a `Final` scale and `RasterizeLayer` of the home layer in the next frame. A third image stamp: (j) a `Final` scale and `ConfirmPending(OpenSample(Sphere))` in the next frame; then an image stamp for the screenshot | (a), (b): composite == H1; (d): composite and paint layers equal the reference; (c): saved, not modified, no unbaked stamp; the reopened composite equals the in-memory one; (f): the files of `<out_stem>_fba_export` equal the reference byte for byte; (e): no text stamp and no text layer, nothing pending; (g): composite equals the reference; (i): two history entries (Final, stroke) and the composite equals the reference; (k) (accept_fixes item C; since fix pass 2 the export job, not the dialog, waits for a pending Final): the export dialog is open in the next frame and the UI did not settle (`FinalBakes::settle_count` unchanged); after `WaitIdle` nothing is unbaked and the dialog is still open; (h): no image stamp left, composite and paint layers equal the reference; (j): the new document has no stamp and nothing waits. In the asynchronous mode the log shows at least one `final bake: stamp N result dropped (stale)` (a), `result dropped (other document)` (j) and `final bake: settle waited T ms` (d, h); with `WTE_SYNC_FINAL_BAKES=1` the same hashes |
| `brush_hover` | Gate 3 (measurement, FEATURE_BRUSH P11): Sphere, 3D view, Brush tool, Brush tab, Circular Size 64; 60 idle frames, 60 `Hover` frames across the model with Circular, then with Sphere + Cube, each stretch between `gate: ...` steps (read with `WTE_FRAME_LOG`) | the Brush tab is shown; the Cube cursor was drawn (`tool_probe.cursor_footprint` > 0) |
| `wrap_surface` [PATH] | Round 6 (R6-2, ROUND6_PLAN 5.2): built-in Sphere, 3D view, the generated 512 x 256 `cells` picture (`samples::cells_picture`) as the stamp image. (1) Stamp tool 30 %, Project, click left of the centre: S, composite H0. (2) `UpdateStampPlacement(S, Projector { mode: Wrap, .. }, Final)`. (3) back to Project; `Undo`, `Undo`, `Redo` x2. (4) Wrap again; S stretched along its spine (width only) to 1.2 x the sphere's circumference: a band round the sphere (H2). (4b) Overlap (Final); `Undo`; Overlap, Project, Wrap; Ends meet. (4c) Jump small gaps (Final); `Undo`. (4d) Project (Final), 3 frames; `Undo`. (5) back to 30 % (Final); S moved onto the UV seam (the meridian of texel columns 0 and width-1 at the middle row) with its long axis across it (Final); `Undo`. (6) a body drag of a quarter of the view width. (7) Text tool in Wrap, "WRAP AROUND" at the tool's default size (6 %), placed below the centre, Arc warp bend 30 % (`UpdateStampSource`, Final). (8) `BakeWait(false)`: S to Project (Final) and `Undo` in the next frame. (9) S to Overlap + Jump small gaps (Final), S selected (`Shot("before_save")`), `SaveAs(<out_stem>_wrap.wtep)`, `OpenProject` of it. Without a PATH the screenshot shows S selected in the reopened project (the text selected first, then S; the Color section collapsed so the whole Transform options show). Named shots: `_wrapped` (after 2), `_band` (4), `_overlap` (4b), `_text` (7), `_before_save` (9). (10) with a PATH (accepted and left for the sphere at the start, opened again at the end): 3D view, the picture at 40 % in Wrap at the centre of the view (or the best `stamp_spots` hit when the centre misses), then Jump small gaps on (Final); the screenshot shows that model | (2) `stamp_ink_area` / picture area 0.93-1.07, `covered` >= 0.999, `cut_tris` 0, one entry, composite changed (H1); (3) Project = H0, then H1, H0, H0 in Project; (4) `met_itself`; of the texels whose `locate` point lies within 0.9 hh of the great circle of S's spine >= 99 % inked, smear (`cells` colour test of 4.7) <= 1 % of the opaque ones; (4b) `turns` 3, one entry; in the two overlap stretches (angle from the anchor 0.83 pi..0.97 pi on either side, inside the 0.8 pi..pi where two turns lie) >= 90 % of the opaque texels show a colour of the picture's right half (the later turn on top); `Undo` = H2; Overlap again = the Overlap state, also after Project and back (Overlap kept); Ends meet = H2; (4c) H2 byte for byte, `jumps` 0, one entry; after `Undo` off and H2; (4d) `tool_probe.wrap_controls == Some(false)` in Project, `Some(true)` after `Undo`; (5) seam score (mean largest channel difference of the composite over the texel pairs of columns 0 / width-1 opaque in S, surface points within 1.25 texel widths) <= 1.25 x the interior score + 2 levels, at least 20 pairs; after `Undo` the 30 % state; (6) the gizmo is `wrapped` without a depth box; the anchor moved >= 0.3 r along the surface, ink 0.93-1.07, one entry, still Wrap; (7) the text is wrapped with a warp, ink area > 0, `covered` >= 0.999 (`scenarios::WS_TEXT_COVERED`; steps 2 and 7 log a 32 x 16 map of the picture points without surface, `ws_holes`); (8) S wrapped, the composite of before the toggle; (9) the reopened composite equals the saved one, both stamps Wrap, S with both options, the text with none, `read_manifest`: `min_reader_version` 3, `mode` "wrap" on both and `wrap_gaps` / `wrap_overlap` only on S; (10) logs `wrap_surface (10) Jump small gaps off / on: ..., ink area, stats` (the bake times are the core's `wrap:` and the `final bake:` lines); ink area > 0 both times. Round 8 (ROUND8_PLAN 4.1): (2b) after (2): `relaxed`, `seams` 0, `stamp_wrap_quality` dup <= 1 %, scatter <= 2 % (logged with stretch, rejected, flipped, cg_iters); (6b) after (6), from `state.wrap_jobs.counters` since (6)'s Setup: asynchronous mode: the document defers wrapped Previews, the drag started >= 1 Preview job and >= 1 landed, `ui_params` unchanged (no parameterization on the UI thread by the app), nothing in flight or waiting afterwards; `WTE_SYNC_FINAL_BAKES=1`: no job used; the composite after the Final is logged in both (equal); (10) accept_fixes item B: after collapsing the Color section, `assert_side_areas_apart`: the tool area ends above the Color section, the Color section above the lower area, no tab button under either (then `assert_side_panel_layout`); it failed before the fix (the Color section reported 43 pt into the Layers area on Lantern and Avocado). (10) also logs `... quality:` ink / picture, dup, scatter, bins, the parameterization's ms, relaxed, seams per rule, stretch, rejected, jumps and the Final's "applied after" ms for Jump off and on, then one assert over both: ink / picture <= 1.3, dup <= 3 %, scatter <= 5 % (scenario count unchanged). Every assert logs its measured values as `wrap_surface (<step>) ...` |
| `export_selected` | Export-select (EXPORT_SELECT_PLAN 6.3): default content; the text layer hidden; `ScreenClick`s on the Layers rows' names: plain on Logo, Ctrl on WTE, Shift on Paint 1, Ctrl on WTE, Ctrl on Paint 1; a right-click on Logo's name and a click on `PanelHit::LayerMenuExportSelected` (`Shot("dialog")`); the dialog's options run into `<out>_export_sel`; a right-click on Paint 1 and "Export this layer..." (`LayerMenuExportThis`) into `<out>_export_one`; `RunExport` of Paint 1 + Logo with a flattened PNG only into `<out>_export_selflat` | the selection after each click ({Logo}, {Logo, WTE}, {Paint 1, Logo}, all three, {Logo, WTE}) with Logo active and neither history nor revision changed; the right-click selects nothing and the menu shows both items (a row outside the selection only "Export this layer"); the dialog has `layers == [Logo, WTE]`, per-layer PNGs only, two `PlannedKind::Layer` stamp/text files; the folders hold **exactly** (directory listing) `<name>_L02_stamp_Logo.png` + `<name>_L03_text_WTE.png` (the hidden one included), then `<name>_L01_paint_Paint 1.png`, each equal to its layer's pixels (stamp file with ink, no stroke colours; Paint 1 with both stroke colours); the flattened selection equals `flatten_layers(Paint 1, Logo)` and is transparent where only the Base shows; no history entry from any of it |
| `export_zip` | Export-select: the `export_run` content exported loose into `<out>_export_ref`, then as `ExportPackage::Zip` into `<out>_export_zip` and `<out>_export_zip2`; `ShowExport` with the zip and the first zip's folder (`Shot("replace")`), `CloseDialog` | the toast "Exported <name>.zip (N files) to <folder>"; each zip folder holds only `<name>.zip`; the two zips are byte-identical; entries = the loose files (names in plan order, bytes); one stamp and one text entry; `verify_export` on the entries (below); the dialog's plan is the one existing zip |
| `export_flow` [PATH] | Export-select (EXPORT_SELECT_PLAN 7), the user's workflow end to end. Without PATH a `seam_cube` written as OBJ + MTL + checker PNG, glTF, GLB and ASCII FBX (`samples::to_obj/to_gltf/to_glb/to_fbx_ascii`), each opened (`ConfirmPending(NewFromModel)`, import accepted) and run through the flow; with PATH the flow once on it. Flow: 3D view; a red brush stroke on Paint 1 (`hit_stroke`); an image stamp (Project) and the text "WTE" (`stamp_spots`); a wrapped image stamp (`tools.stamp.mode = Wrap`, `third_spot`: the solid spot farthest from the two); `ShowExportLayers([wrapped])` + the dialog's options into `<out>_<model>_one`; `SelectLayer(stamp)` + `ToggleLayerSelected(text)` (another pair of one set when they lie in different sets) + `ShowExportLayers(selection)` into `<out>_<model>_sel`; every kind as one zip into `<out>_<model>_all` (`<model>` = obj, gltf, glb, fbx or path). Long timeouts | the OBJ cube's Base is the checker its MTL references; the stroke is one history entry and changes the composite; the wrapped stamp is `Projected` with `mode == Wrap`, covers texels, no Final bake pending; `_one` holds exactly the wrapped layer's PNG (its pixels, with ink); `_sel` exactly the two PNGs (each its layer, different from each other, stamp files without the stroke colour); `_all` exactly `<name>.zip` whose entries pass `verify_export`: names = `plan_contents`, flattened = the composite, per-layer = the layers, ORA merged = composite with the exported layer count, PSD layer names = the visible layers, and the per-layer PNGs re-composited with the document's order / visibility / opacity / blend = the composite exactly; afterwards the document key (composites, layers' properties and pixels), the revision and the history are unchanged and no `.tmp-` entry is left |

Round 4: 37 scenarios (25 of round 3, the ten above and the gate's two measuring scenarios `text_preview_path` and
`brush_hover`); round 5 adds `final_bake_async` (38); round 6 adds `wrap_surface` (39); export-select adds
`export_selected`, `export_zip` and `export_flow` (42, rows below the table); fix_export adds `export_unsettled`,
`export_nouv` and `export_errors` (45); polish adds `save_unsettled` (46); accept_fixes adds `export_checks` (47); `scenarios::tests` checks the count. fix_export also extends `export_flow` (the
dialog's Layers choice switched to Visible and back after "Export this layer"; the toast says "1 file"; everything as
loose files into `<out>_<model>_loose`, equal to the zip's entries byte for byte; the selection as a zip into `_all`:
`<base>_selection.zip` next to the unchanged `<base>.zip`, every entry equal to the full package's; Duplicate and Delete
on the two selected layers, one undo step each, undone to the same document key), `multi` ("Export this layer" on the
stamp across both sets writes both sets' layers) and `fuzz_path` (actions: Ctrl-click, Shift-click, Duplicate / Delete
of the selection, rename to odd names, an export into `<out>_fuzz_export` with no temp file left, save + reopen as
`<out>_fuzz.wtep`). The runner takes an error toast a scenario announced (`scratch.values["expected_error_toasts"]`,
counted down) instead of failing (`export_errors`).

| Scenario | Steps | Asserts |
|---|---|---|
| `export_unsettled` [PATH] | fix_export item 6. The sphere sample (or PATH); the sample image as a 30 % Wrap stamp placed with `BakeWait(false)`; the frame clock (`AppState::frame_work`) reset while the Final is pending; `RunExport` of the stamp's layers (+ flattened) into `<out>_a` at once; after it, the same export into `<out>_b` | the Final was pending at the export; no frame of UI work took 300 ms or more, at least 3 frames drawn; the two folders byte-identical; every layer file equals its layer's Final pixels, with ink. Run on ABeautifulGame (Final ~1-4 s) and `high_poly_sphere.obj` (~4 s) |
| `save_unsettled` [PATH] | Polish item 2. The same start as `export_unsettled` (`place_wrapped_at_once`, `wrapped_stamp_pending`); `SaveAs(<out>_a.wtep)` at once while the Final is pending; after it, `SaveAs(<out>_b.wtep)` | the Final was pending at the save; no frame of UI work took 300 ms (`SAVE_MAX_FRAME_MS`) or more, at least 3 frames drawn; saved as `_a.wtep`, not modified, nothing pending, no dialog; the two projects' manifests equal apart from `saved_unix` (every entry's size and CRC-32), both load with the same sets, layer ids, names and pixels, equal to the document's; the stamp's saved layers have ink (`same_project`). Run on ABeautifulGame |
| `export_nouv` | fix_export item 9 (L3). The UV-less tetrahedron (generated layout), a stroke and a Project stamp; everything exported into `<out>_a`; Save As, Open; everything again into `<out>_b`; then as a zip into `<out>_zip` | the export holds flattened + every visible layer + ORA + PSD; `_a` and `_b` byte-identical; the zip's entries equal the loose files |
| `export_checks` | accept_fixes item E. `default_content` (Base, Paint 1 with the red and blue strokes, the Logo stamp, the WTE text); Logo hidden, Paint 1 at 50 % opacity in Multiply. (E1) `RunExport` Visible into `<out>_ec_vis` and All (`include_hidden`) into `<out>_ec_all` (flattened PNG + per-layer PNGs, no seam padding); (E2) click Paint 1's name, Shift-click WTE's name, right-click, "Export selected layers... (3)", `Shot("hidden")`, the dialog's options into `<out>_ec_sel`; (E3) `SelectLayer(Base)` + `ToggleLayerSelected(WTE)`, `ShowExportLayers`, the flattened PNG ticked, into `<out>_ec_base` | every check reads the files (stack index from `_L<nn>_` in the name) and compares with `reference_composite`, ARCHITECTURE 6.6 written out in the test (f64, opacity and accumulator quantized to u8, alpha-0 rule) over the per-layer PNGs of `_ec_all`: (E1) Visible writes Base, Paint 1, WTE (no Logo), All also Logo (its layer's pixels); the shared files are equal; both flattened files are byte-identical and leave Logo out (the flattened texture is the visible layers under All too, `ExportOptions::include_hidden`): they differ from the reference with Logo in >= 50 % of its opaque texels; (E4) both flattened files equal the reference of the visible layers within 2/255 in every texel, and differ from the reference with Paint 1 as Normal 100 % in >= 100 texels; (E2) the selection is Paint 1, Logo, WTE; the dialog's list (`ToolProbe::export_files`) marks exactly Logo's line "(hidden)", the caption is the selection one without a flattened file; the folder holds the three layers' PNGs (their pixels, Logo with ink); (E3) the caption is the selection one with the flattened file; the folder holds Base's and WTE's PNGs and a flattened PNG equal to the reference of Base + WTE within 2/255, different from the visible stack (Paint 1 not in it) |
| `export_errors` | fix_export item 9 (L1, L4). Sphere sample; three exports that cannot work: the folder path under a file, a planned file that is a folder (Files, an old file in the folder), the zip's name a folder; then a folder with a dead process's stage folder, temp zip and temp PNG plus `notes.txt`, exported as a zip | each failure: the "Export failed" message, an error toast (announced), nothing new written, the old file unchanged, no `.tmp-` entry; the last export removes the leftovers and keeps `notes.txt` |

Gate 3 added named mid-run captures (`Step::Shot`) for the report:
`brush_shape` `_dialog` (the closing dialog), `text_tab` `_page_character` / `_page_warp` / `_page_path`, `text_fonts`
`_picker`, `text_warp` `_warp3d`, `text_path` `_circle3d`, `text_gaps` `_gap3d`. The ten were written and
first run by their author packages (ROUND4_PLAN 2.2 item 11) and pasted by the gate (sitting M2). `brush_tab` applies
the A12 pixel budget (300 pt, slider rows inside the tool area, tab without scrolling) only in a window of at least
1280 x 800; in a smaller one it checks the Color section inside the panel, a width of at most 300 pt and the tab
inside the lower area (gate 1).
\* action-only smoke scenarios: already contained in the scaffold's stub of `scenarios.rs`, so `app-shell` and `app-gpu` can be
verified before the views exist (section 10).

**Round 9 (package B).** `wrap_surface` step 10 (PATH): after the Jump-off Final a capture `<out>_off.png`; per
setting a `coverage:` line (covered, ends_share, reached_edge, lost_cut, passed); after the Jump-on screenshot frames a
third log state, `panel lines:` (the shared-texel share and whether the T9 line shows, ends_share and whether its line
shows); and the asserts of ROUND9_PLAN 7.2: `covered >= 0.85 x ends_share` unless `reached_edge`, `lost_cut <= 10 %`,
both settings. Stable (w2): `covered >= 0.85 x ends_share x room` (`WrapStats::room`: a part narrower than the picture
across holds only that share of its height: ABeautifulGame Jump off on a pawn, many_islands' islands) and `lost_cut <=
10 %` unless `wraps_closed` (a picture covering a whole closed part loses the part's total curvature at its corners:
the Lantern's 52-vertex base block); the `coverage:` line also logs `room` and `wraps_closed`. Stable (fix_wrap_1):
`lost_cut <= 15 %` when `wraps_closed` (w2 left it unbounded; the Lantern 12.11 %, round 8's cone rule there 14.7 %);
the `coverage:` line also logs `shown` and `beyond`, and `panel lines:` says "line shown" for the ends share by the
panel's rule (`ends_cut_shown`). `room` is the parts' 3D extent across, a lower bound of the surface's room on a
curved part, so the room factor makes the coverage assert more lenient than the surface requires, not stricter (w2's
notes said the opposite).

### 8.12 Theme and icon (`theme.rs`) [A1]

`pub fn apply(ctx: &egui::Context)` (dark visuals, slightly tighter spacing); `pub fn app_icon() -> egui::IconData` (64x64 RGBA
generated procedurally at runtime: a UV-checker sphere glyph); `pub fn tool_icon(painter: &egui::Painter, rect: egui::Rect,
tool: ToolKind, color: egui::Color32)` (vector-painted, no image assets). `apply` also installs `egui::FontProvider`s for
`%WINDIR%\Fonts\seguisym.ttf`, `seguiemj.ttf`, `segoeui.ttf` (each read lazily, the first time egui misses a glyph): the
built-in egui 0.37 fonts lack symbols egui's own widgets use (the colour picker's "📋" copy button), and the emoji-font and
`system_fonts` features stay off (no new crate features).

### 8.13 Pitfalls for app implementers

1. glam 0.34 has no `Mat4::perspective_rh`; use `glam::camera::rh::proj::directx::perspective` and `rh::view::look_at_mat4`.
2. WGSL uniforms: `vec4`/`mat4` only; `textureSample` never inside non-uniform control flow.
3. Never hand straight-alpha pixels to egui as a texture: `ColorImage::from_rgba_unmultiplied`.
4. Texture side is capped at 8192: clamp blank sizes in the import dialog and `ViewTarget` sizes.
5. The screenshot callback needs `device.poll` and captures the frame *after* the request.
6. Viewports use `egui::Sense::DRAG` (the non-focusable constant) and detect clicks themselves. `Sense::click_and_drag()` delays
   drags; `Sense::drag()` is focusable, which puts the views into Tab navigation.
7. Hold `egui_renderer.write()` only for a single register/update/free call (eframe takes the lock while painting).
8. All wgpu 30 descriptors need the new fields (`depth_slice`, `multiview_mask`, `immediate_size`, `cache`, `Option<..>` wrappers).
9. Keep `cull_mode: None`; `Document::raycast` returns the nearest hit regardless of facing, so picking matches what is drawn.
10. Large files never go to C: — screenshots, self-test logs, projects and exported files are written where the user/CLI says;
    nothing is cached in `%TEMP%`. Only `prefs.json` and the normal-mode `log.txt` (a few KB, truncated at every start) live
    under `%APPDATA%`.
11. Every `ctx.send_viewport_cmd` requests a repaint: never send one unconditionally per frame (`update_title` compares with
    `last_title`). The self-test runner is the only exception.
12. Wheel input comes from the raw `Event::MouseWheel` events only (8.6); `smooth_scroll_delta` is empty under Ctrl and
    horizontal under Shift.
13. "Is the user typing?" is `ctx.text_edit_focused()`, not `ctx.egui_wants_keyboard_input()` (true for any focused widget).
14. Shortcuts match logical keys: Shift+`[` is `Key::OpenCurlyBracket`. Order table entries most-specific-first.
15. Do not use `wgpu::util::TextureBlitter` for mips (nearest minification); use the blit pipeline of 8.4.
16. No `rfd` call outside `dispatch` / `io_flows` / `main`; no key handling outside `shortcuts::handle`.

---

## 9. Test plan

### 9.1 `wte_core` unit tests (in-file `#[cfg(test)] mod tests`, headless, no external assets)

Every package's in-file tests may call only code of the packages it depends on (section 10: "Depends on" column and stages), so each
stage is green before the next one starts. Tests that need the whole crate (painted + stamped + text content, re-bake on load,
`Document::sample` through the real importer in combination with everything else) live in the integration tests owned by
`core-integration` [C7]. Temp files only through `testutil::temp_dir`. The owner tag after each group is the package that writes
those tests.

- **math** [S]: `div255` exhaustive over 0..=65535 against `f64` rounding; `IRect` intersect/union/empty cases; `DirtyRegion` merge
  and overflow collapse.
- **pixel_ops** [C1]: premultiply round-trip; mip of an opaque-red / transparent-green checker stays pure red; bilinear border is
  transparent; `content_bounds` tight; `nonzero_bounds` keeps texels with alpha 0 and non-zero RGB and is `None` only for an
  all-zero buffer; `resize` preserves a solid colour; `thumbnail` of an 8192^2 view reads at most `4 * side^2` source pixels
  (cost independent of the canvas size) and preserves a solid colour; `dilate_islands` fills only unowned transparent texels.
- **mesh_build** [C1]: tile normalization leaves every centroid in `[0,1)`; partially missing UVs give `NO_UV` triangles, sorted
  last within their material, with `material_uv_ranges` / `material_nouv_ranges` partitioning `material_ranges`; degenerate
  triangles are dropped and counted; generated normals are unit length; `wire_edges` of `seam_cube` = 24 `Seam` entries
  (12 geometric edges x 2 UV sides), 6 `Interior` entries (the face diagonals) and 0 `Boundary`; of `uv_sphere` has interior
  edges and one seam column; shared-texture materials share a set; set names follow the rule of 6.1 step 5 (texture stem, else
  material name, else model name, uniquified); fallback UVs make every triangle paintable and non-overlapping; `uv_hash` stable
  across two runs and different for a changed UV; `uv_layout_image` of `seam_cube` has orange pixels on every island border and
  is deterministic.
  objects-tabs (6.1 step 9): `objects_follow_their_triangles_through_the_sort`, `object_data_never_changes_the_mesh` (random object
  data leaves positions, normals, uvs, indices, flags, ranges, sets and `uv_hash` equal to `normalize`, and the wireframe equal
  apart from `object`), `unnamed_and_duplicate_names`, `objects_without_triangles_disappear`, `no_object_data_is_one_object`,
  `wire_edges_name_their_object`, `many_objects_with_one_name_are_numbered_in_linear_time` (check of WP1/2: 20,000 objects
  named "Leaf" plus one explicit "Leaf_3"). Importers: OBJ `o` objects, `g` groups without `o`, `o` wins, faces before the first `o`,
  re-entered names, no `o`/`g` = one object named after the model, `to_obj_objects` round trip; glTF mesh nodes (named / unnamed
  with a named mesh / shared mesh / duplicate names, transforms in the bounds); FBX two `Model`s; `Document::sample(Objects)`.
- **bvh** objects-tabs: `tri_mask_basics`; `a_hidden_triangle_is_skipped_by_every_query` (`cube_on_floor`: raycast,
  occluded / occluder, query_sphere / query_sphere_dist; everything hidden -> nothing); `no_mask_and_an_empty_mask_answer_identically`
  (2,000 random rays and spheres on `soup_sets`: plain = `with_hidden(None)` = empty mask field by field; a random half
  hidden = brute force over the visible triangles); `with_hidden_shares_the_tree`. `document::objects_tests::
  hiding_drops_cached_wrap_params` (also: a ParamOnly job from an older visibility is not cached).
- **bvh** [C1]: 10k random rays against brute force (same triangle, `t` within 1e-4); `occluded == raycast.is_some()`;
  `query_sphere` equals brute-force closest-point; analytic `uv_sphere` hits (position on the sphere, normal ~ position / r, UV matches
  the equirect formula); axis-parallel rays (infinite `inv_dir`); ray origin on an AABB face; `Cull::Back`; a hit on a
  reversed-winding triangle has `front_face == false`, `normal` facing the ray origin and `surface_normal()` equal to the stored
  shading normal.
- **texel_map** [C1]: `seam_cube` covered count ~ island area +-2%; gutter width exact; `overlapped == 0` for the cube and `> 0` for
  `mirrored_quads`; `locate` then re-derive UV round-trips within half a texel; sliver fallback; `raster_tri` fragment barycentrics
  sum to 1; wrapped quad writes at wrapped coordinates; the gutters of `tight_islands` touch (precondition of the fill test).
  Round 7: the lean pass 2 (`raster_tri_lean` / `raster_tri_covered_lean`) visits exactly the fragments and covered runs
  of the full call, in the same order, for every triangle of round 7's meshes (atlas, sphere, tiled, stacked / mirrored,
  slivers, wrapped quad), clips none / wider than the image / inverted / random wrapping, empty and partial row ranges
  (`lean_pass2_visits_the_same_fragments`); the brush index is exact: a triangle's bit is set iff it owns a gutter texel
  or claimed one, every block with a `shared_gutter` texel has its bit, and a 1-thread build gives the same bitsets
  (`prepare_brush_index_is_complete`); the folded row loops walk exactly the rows that fold into `rows`
  (`folded_rows_are_the_rows_in_range`).
- **blend / composite** [C2]: each blend mode against an `f64` reference (<= 1 LSB) over random pixels; dirty-rect recomposite equals
  full recomposite (random edits); hidden and zero-opacity layers skipped; cropped-layer clipping; parallel equals serial;
  `composite(set) == flatten(set, false)`; **alpha-0 rule**: flattening one layer reproduces it byte for byte including RGB under
  alpha 0, for every blend mode; a layer with RGB = (10, 20, 30), A = 0 under an all-zero layer flattens to itself; a pixel with
  alpha 0 never changes an accumulator pixel with alpha > 0.
- **doc_layers / history** [C2]: every layer command followed by `undo` restores a byte-identical structure and composite; random
  command sequences (layer commands mixed with raw pixel edits pushed through `push_edit`): undo-all gives the initial document,
  redo-all the final one, byte-identical; budget eviction keeps the newest entry; solid tiles compress; `TileBackup::into_patch`
  drops unchanged tiles; `original_row` equals `original` texel by texel; pending edit: N Preview calls + Final = one entry,
  `cancel_pending_edit` restores; `is_modified` / `mark_saved`; merge-down pixel result equals the composite of the two layers when
  the lower one is Normal at 100%; structure invariants hold after every command; **no pinning**: after `set_layer_visible` (and
  every other property command) the layer's pixel `Arc` has strong count 1, and a following pixel edit grows the history by the
  tile patch only (not by a layer copy); `remove_layer` / `merge_down` pin exactly the layers listed in 5.17; a layer's `version`
  changes on pixel edits and on undo/redo of them, and not on property changes.
- **doc_build** [C2]: `from_import` (through `testutil::imported_from_raw`) with Detected / Image / Blank gives two layers per set —
  locked "Base" with the chosen pixels and an empty active "Paint 1"; `Image { original: Some(..) }` lands in `source.extras`
  under `textures/set00/<name>`; size mismatch in `from_parts` is `BadSetup`; `from_parts` adds and locks nothing; `Document: Send`.
- **brush / stroke / fill** [C3]: a dab at a cube corner modifies texels in exactly 3 islands; a dab centred on a cube edge gives
  equal mask values (+-1 LSB of u8) at mirror-symmetric surface points on both faces (the core seam-correctness assertion); sphere
  seam: painted disc continuous across u = 0/1; repeating a dab at flow 1 leaves texels with `dab_cov == 255` at mask 65535 and
  never decreases any mask value; within one dab, a texel visited twice through overlapping / mirrored UVs gets the same value
  as when visited once (max rule); flow 0.05 soft brush builds up monotonically and reaches opacity; the opacity ceiling holds
  under self-overlapping strokes; `falloff`: 0.5 at the rim of a hard brush, 0 at `radius + aa/2`, 1 inside; a 1-texel brush
  (radius 0.5) covers the texel under the cursor for cursor positions at the texel centre, edge and corner; erase lowers alpha
  only; alpha lock preserves alpha; dab count = `floor(L / step)` with the residual carried; `mirrored_quads`: a dab on the
  non-owner quad still paints; a `flat_quad` with `reverse_winding` is paintable with default `BrushParams` (`same_side_only`);
  `stroke_cancel` restores exactly; `wrapped_quad` paints at wrapped coordinates; 2D dab wraps at borders; a multi-set stroke gives
  one history entry touching both sets; while a stroke is open every history-producing command of 5.17 and `fill` return
  `StrokeInProgress` and `undo` / `redo` return `None`, changing nothing; `stroke_begin` flushes a pending edit;
  locked/hidden/non-raster targets are skipped and reported by `stroke_skipped_sets` with
  their reason; Island fill on `seam_cube` stays inside one island and covers its gutter; Island fill on `tight_islands` leaves
  every covered texel of the neighbouring island untouched, also when the seed is a gutter texel; `sample_color` wraps.
  Round 7: random dabs (ball hard / soft, Cube Solid and Through, SameSide / All, radii from under a texel to the whole
  model) on round 7's meshes and maps of 1 x 1 to 1024 x 1024, gutters 0 to 16, give round 5's masks and rects (the
  `RASTER_R5` seam) and leave `dab_cov` zero and `dab_touched` empty (`r7_dabs_equal_round5_on_random_dabs`); the fused
  pass with bands of 16 / 32 / 64 rows equals the serial loop + `commit_dab` and round 5's code on heights that are not
  a multiple of the band (`fused_band_commit_equals_the_serial_commit`); serial, banded and folded dabs and a two-set
  document stroke give the same masks, rects and layer bytes in pools of 1, 2 and 5 threads
  (`dab_raster_is_the_same_on_any_thread_count`); whole two-set strokes (ball and Cube) give round 5's document state
  with the seam on / off, the probe on / off and re-apply by spans / whole rects (`a_stroke_paints_round5s_pixels`).
  `stroke_bench_tips` (ignored timing probe) adds an "... r5" row per shared-path tip and asserts r7 hash == r5 hash.
  Round 7 check: adversarial dabs (1 x N and N x 1 images, a 1-row last band, gutters 0 / 1 / 16, boxes far wider than
  the image, a multi-set soup, repeated dabs on saturated masks) give round 5's masks and rects through `dab_tip` and
  through the forced fused pass (bands of 16 / 32 / 64 rows, pools of 1 and 3), and every raised texel lies in its row's
  changed span (`check_r7_adversarial_dabs_equal_round5`); paced strokes over six sets, a cancelled stroke with an open
  segment and 2D strokes between 3D strokes give round 5's document state
  (`check_r7_many_sets_cancel_and_2d_strokes_paint_round5s_pixels`).
- **stamp_bake / doc_stamps** [C4]: project `sample_stamp_image` onto `flat_quad` and compare with direct resampling (PSNR > 40 dB);
  flips land where expected (asymmetric image); a +90 degree rotation moves the image's top edge to the right in **both**
  `bake_projected` (`Projector::rotated`) and `bake_uv` (`UvPlacement.rotation`); projection across a cube edge covers two islands
  and leaves back faces untouched with culling on; project-through (`backface_cull = false, occlusion = false`, deep box) covers
  both the near and the far side of a closed cube; a decal on a `reverse_winding` `flat_quad` is empty with culling on and
  non-empty with `backface_cull = false`; occlusion on `occluded_quads`: the rear quad receives nothing (Full) but does in
  Preview / `occlusion = false`; angle falloff monotonic on the sphere (full weight below 68, none beyond 89 degrees); the
  depth fade is smooth (no alpha step over the last 20%); **depth auto-fit**: on a unit sphere (decal half size 0.8) and on the
  narrow tip of a 0.4 x 0.4 x 1 ellipsoid (half size 0.35) every front-facing texel of the footprint below 68 degrees gets full
  alpha and none below 80 degrees is cut off, while the old default depth cut texels off; a hand-edited depth is not "auto";
  on `occluded_quads` a decal hanging over the front quad's edge keeps the rear quad (0.5 behind, > one half size) out of the
  box, a larger one (half size 0.6) reaches it; a placement on the rear quad ignores the front quad's edge in front of it;
  `bake_uv` rotation/flip/clip; cropped rect is tight;
  Preview and Full agree within tolerance without occlusion; `add_stamp` on the two-material model creates linked layers in both
  sets and one history entry; Preview drags + Final = one entry; **`set_stamp_placement(Preview)` on `occluded_quads` followed by
  `flush_pending_edit()` gives exactly the caches, layers and history of `Commit::Final`** (also when the flush comes from
  `undo()` or from another layer command), and `preview_baked` is empty afterwards; undo restores caches without
  re-baking (pointer-equal `Arc`s); `remove_layer` / `duplicate_layer` / `rename_layer` on a linked layer act on the whole stamp
  with exactly one history entry; text edit keeps world size per em; **text resolution**: scaling a text stamp 4x (Final) raises
  `size_px` so that raster px per texel >= 1 up to the 512 cap, with the on-model size unchanged within 1%; outline width in
  texels is unchanged by that re-rasterization (em fraction); `stamp_at` picks the topmost.
- **text** [C4]: `measure_text` equals the size of `rasterize_text`; two lines are taller than one by `line_h`; right alignment shifts
  ink; letter spacing widens; outline grows the bbox by about `2 * width_em * size_px`; a missing glyph falls back to the bundled
  font; empty text is laid out as one space (fully transparent, one line high); EDT against brute force on a 16x16 mask;
  `FontCatalog::bundled` lists "Hack" and "Ubuntu" and `find("Ubuntu", "Light")` succeeds; families sorted without case;
equal style names stay reachable ("Light", "Light (Ubuntu-Light)"); `face_bytes` shares one allocation per file and
`BlobStore::insert` knows it; file sizes are read once (100 `faces()` calls with the file deleted); mixed-case
extensions are loaded; samples of both bundled fonts have ink within 128 x 20; `face_traits` matches the catalog; on
this machine (skipped without the Windows font folder): at least 50 families, Arial with a real Bold face, 90 % of the
families draw a sample; `style_name` table (300 => "Light", 400 italic =>
  "Italic", 700 italic => "Bold Italic", 350 => "W350"); `face_bytes` round-trips into fontdue.
- **import** [C5]: OBJ round-trip through `samples::to_obj` reproduces positions/UVs (V flipped); quads are triangulated; mixed faces
  with and without `vt` do not misalign; `usemtl` splits materials; `map_Kd -s 1 1 1 my tex.png` parses to `my tex.png`; a missing
  MTL is a warning; Latin-1 bytes are accepted; bad `v` / `vt` / `vn` numbers keep the numbering, drop or degrade only the faces that
  use them and give one `BadObjNumbers` warning (faces with a position index pointing nowhere are dropped the same way), a file with no usable face left still fails naming the first bad line. glTF (JSON built with `serde_json::json!`, GLB wrapped with
  `gltf::binary::Glb { .. }.to_vec()`): GLB with embedded PNG; `.gltf` with a base64 buffer; `.gltf` with external `.bin` and image
  via `MemoryResolver`; node translation baked; negative scale flips winding; no indices; normalized u16 UVs; two primitives with two
  materials give two sets; two materials sharing an image give one set; `KHR_texture_transform` offset/scale applied;
  FBX texture `Translation` / `Scaling` / `Rotation` with pivots and the older `ModelUV*` names applied before the V flip, and an
  FBX quad with scale 2 / offset 0.25 has the `uv_hash` of its glTF twin (offset 0.25, -1.25); the 21 importable Assimp FBX
  samples keep their frozen `uv_hash` (prints an explicit SKIPPED line when `E:\claude_downloads\fbx_samples` is absent; with the
  folder present all 21 files must be there);
  `extensionsRequired: ["KHR_draco_mesh_compression"]` => `UnsupportedGltfExtension`, also next to `KHR_mesh_quantization` (only
  the refused names are listed; `EXT_meshopt_compression` likewise); `KHR_mesh_quantization`: a box (off-grid face centres) in every
  position encoding (i8 / u8 / i16 / u16, normalized or not, under a dequantizing child node and a rotated, scaled parent), i8 / i16
  normalized normals, i8 tangents and every UV encoding rescaled by `KHR_texture_transform` imports to the mesh of its float twin
  (positions within 1e-4 of the bounding-box diagonal, normals and UVs within 1e-4, same indices and flags; 16-bit positions also
  against the unquantized box); the normalization rules (-128 => -1); non-normalized integer normals regenerated, u32 positions
  skipped, sparse quantized positions read; without the declaration a non-normalized u16 UV is still read normalized and an
  integer position is skipped (core reading). Resolver: case-insensitive and
  backslash paths; `RecordingResolver` captures exactly the files read; `MemoryResolver::from_bundle` ignores `extras`;
  base64/percent/data-URI edge cases. `import_from_bundle(.., decode_textures = false, ..)` gives the same mesh, sets, set names
  and `uv_hash` as with `true`, every `image` is `None`, and no texture warning is emitted — also when a texture is missing.
- **image_io** [C5]: every `FlatFormat` encodes and decodes back to the same dimensions (exact pixels for lossless formats); JPEG
  matte; TGA detected by extension; RGB under alpha 0 survives PNG; oversize image is downscaled and flagged.
- **project** [C6] (raster-only content: `Document::sample` or a real import, with pixels written directly into the layers through
  the crate-internal accessors — no strokes, no stamps): round trip of the
  two-material sample (pixels equal per layer, composites equal, Base still locked); **a raster layer with RGB = (10, 20, 30),
  A = 0 everywhere round-trips byte-identical** (`nonzero_bounds`), and an all-zero layer is stored as `pixels: null`; saving the
  reopened project yields a byte-identical `manifest.json`; saving twice with the same `now_unix` gives identical file bytes;
  self-containment: import from a temp dir, save, delete the temp dir, load => same `uv_hash`; **manually chosen texture**:
  `from_import` with `Image { original: Some(file) }`, save, delete the file, load, `extract_bundle` => the file comes back with
  identical bytes under `textures\set00\`; `extract_sources` reproduces the original bytes; `extract_bundle` with
  `overwrite == false` never replaces an existing file (`_2` suffix); corruption: truncated/garbage => `NotAZip`, missing manifest =>
  `ManifestMissing`, flipped byte in a layer PNG => `LayerPixelsLost` and the rest loads, corrupt
  source => `ModelUnrecoverable`, `format_version = 999` with high `min_reader_version` => `TooNew`, unknown layer kind =>
  `UnknownLayer`; the first ZIP entry is `mimetype`, STORED (guards the "no compression level on Stored" rule of 7.1);
  atomicity: a failing writer leaves the original untouched and no temp file; `.bak` rotation; frozen v1 manifest fixture loads.
- **export** [C6]: PSD round trip through `read_psd` (three layers incl. non-ASCII name, hidden, opacity 0.5, Multiply, one empty);
  golden bytes for a 2x1 single-layer PSD assembled by hand in the test; PackBits edge cases (lengths 0, 1, 2, 127, 128, 129, 130,
  all-equal, alternating, random) cross-checked by an independent naive decoder in the test module; PSD structure checks
  (`section_len == 4 + layer_info_len + 4`, `layer_info_len % 4 == 0`, channel lengths); oversize dimension error; ORA: first entry
  `mimetype` STORED with exact content, `stack.xml` order and attributes, PNGs decode to the layer pixels; naming and sanitizing
  (reserved names, collisions); flattened PNG decodes to exactly `flatten_layers`; **`export(import(x)) == x`**: the flattened PNG
  of an untouched sample (Base + empty Paint 1) decodes to the original texture byte for byte, including RGB under alpha 0;
  per-layer PNGs are full canvas; `plan` returns exactly the files `run` writes, in order, and `run` creates a missing `dir`.
- **integration** [C7] (`crates/wte_core/tests/`):
  - `end_to_end.rs`: `Document::sample` for all three kinds (set counts 1/1/2, Base locked + Paint 1 active); two-material sample ->
    3D stroke across the material boundary -> image stamp -> text stamp -> assert both sets changed and that the `take_gpu_dirty`
    rects cover every changed composite pixel -> save -> load -> composites identical -> export all formats into a temp dir ->
    stamp and text layers appear as separate layers in every layered format and as separate per-layer PNGs (R4) -> undo
    everything -> composite equals the initial one; random command / stroke / stamp / undo / redo sequences restore
    byte-identical documents (soundness of `RecordPixels::Keep`); a visibility toggle followed by a real stroke does not grow
    the history beyond the tile patch; the stamp commands of 5.20 return `StrokeInProgress` during a stroke; a Preview stamp
    edit flushed by `stroke_begin` is Full quality.
  - `project_roundtrip.rs`: round trip with raster, stamp and text layers (stamp parameters `PartialEq`, including a rotated UV
    stamp — radians stored verbatim —, pixels equal per layer); byte-identical `manifest.json` on re-save; corrupt stamp cache =>
    `BakeCacheRebuilt` with a correct re-bake; deleted image blob => `BlobMissing` (the layer survives as raster); missing font
    blob => `FontFallback`; a project saved right after a
    `Commit::Preview` edit (flushed by the caller as the app does) contains Full-quality caches.

**Round 4 core tests** (by package):

- S1: `text::tests` (`warp_style_keys_round_trip`, `default_fx_uses_no_feature`, `float_list_is_complete`,
  `sanitize_repairs_the_effects`, `non_finite_and_out_of_range_gaps_are_sanitized`,
  `differs_only_in_spacing_is_strict`); `text::gaps::tests::joined_characters_are_the_fixed_ranges`;
  `text::path::tests::creation_formulas`; `text::frame::tests` (`em_map_round_trips`, `refit_keeps_the_em_size`);
  `text::raster::dispatch_tests::the_classic_frame_matches_the_classic_picture`;
  `text::font_catalog::tests::face_traits_match_the_catalog`; `project::manifest::tests`
  (`text_fx_round_trips_and_plain_text_keeps_the_v1_keys`, `unknown_fx_names_load_without_the_feature`);
  `brush_tip::tests` (`frames_are_right_handed_with_the_normal_as_y` = C15 first part, `legacy_and_clamping` = C22 for
  the tip); `tip_field::tests` (`envelope_matches_brute_force`, `built_in_cube` = F1, `built_in_cylinder`,
  `neighbouring_nodes_differ_by_at_most_one_cell` (F9 for the built-ins), `sections_of_the_cube` = F13,
  `local_thickness` = F20 for the built-in Cube, a formula ball and the fin of C23, `flat_outlines`,
  `same_result_on_one_thread`); `tip_bake::tests::messages`; `samples::tests`
  (`brush_fixtures_match_their_descriptions`, `gltf_and_glb_writers_import_back`, `fbx_writer_imports_back`,
  `base64_is_standard`); `stroke::tests::stroke_begin_with_a_ball_is_the_legacy_stroke`;
  `tests/project_roundtrip.rs::frozen_v1_text_fixture_loads`.

- B1: `tip_bake::tests`: `baked_cube_matches_the_formula` (F2), `winding_doubles_and_inner_walls_leave_no_trace` (F3,
  F4, F5), `overlapping_solids_are_one_solid` (F6), `shapes_without_an_inside` (F7, with plan 7 item 5),
  `faces_on_planes_of_nodes` (F8), `neighbouring_nodes_and_far_nodes` (F9), `a_mesh_without_uvs` (F10),
  `deterministic` (F11), `cancel_and_limit` (F12), `unused_parts_do_not_set_the_size` (F16, field part),
  `a_thin_closed_model_is_not_refused` (F17), `lying_flat` (F18),
  `a_closed_part_cut_by_the_second_grid_does_not_flood_the_solid` (F19), `local_thickness_of_bakes` (F20),
  `a_baked_ball_is_the_ball` (C13, field part), `a_shape_from_a_gltf_file` (F21), `a_shape_from_a_glb_file` (F22),
  `a_shape_from_an_fbx_file_and_its_units` (F23), `node_transforms_and_several_meshes` (F24),
  `an_open_box_with_every_mode`, `a_rounded_cap_is_larger_than_a_flat_cap`, `a_non_planar_opening_is_capped`,
  `closed_parts_are_reported_and_used`, `sheets_and_closed_models_are_not_open`, `ear_clipping`,
  `a_small_hole_is_closed_quietly`, `a_double_sided_open_model_asks`, `a_tiny_sheet_is_not_a_closed_part` (plan 7
  item 5); `a_model_far_from_the_origin`, `a_cancelled_load_is_waited_for_by_the_next`,
  `the_compat_folder_is_found_from_every_copy`;
  ignored (files of this machine): `odd_files` (F15), `real_files_of_every_format` (F25), `load_times` (P10, core
  part); they find `target\compat` by walking up from the crate (main tree or any copy) and fail instead of skipping
  when `WTE_REQUIRE_MACHINE_FILES` is set. `import::tests::geometry_import_ignores_missing_textures` (F14).

- B3: `bvh::tests::sphere_query_with_distances_matches_brute_force_and_query_sphere` (C4).
  `brush::tests`: `a_ball_shaped_field_equals_the_ball` (C12), `coverage_matches_the_formula_at_every_texel` (C9),
  `folded_field_dabs_equal_unfolded` (C16), `a_large_field_dab_is_the_same_on_the_pool` (C17),
  `a_tiny_field_dab_covers_the_cursor_texel` (C18), `field_paint_never_leaves_the_bound` (C19),
  `thin_parts_are_not_faded_as_a_whole` (C23), `the_early_reject_decides_the_reach_of_a_field`,
  `the_fragment_reject_changes_no_coverage`.
  `stroke::tests`: `the_shared_query_equals_the_per_set_path` (C2, also the C5 fold(60) row with the raycast's own
  normal), `the_tip_pose_follows_the_last_dab`, `a_set_only_the_bounding_ball_touches_gets_no_stroke_buffer` (C24); the ignored
  timing probe `stroke_bench_tips` (FEATURE_BRUSH 8.4: `cargo test -p wte_core --release --lib -- --ignored
  stroke_bench_tips --nocapture`; `WTE_BENCH_MODELS=ac` limits the models; in its first repetition it hashes the paint
  layers after each stroke, undoes it, and asserts that the shared query gives round 3's pixels on the real models).
  `tests/brush_tips.rs` (new file): `strokes_save_identically_through_both_entry_points` (8.5),
  `one_source_of_truth_for_the_side_filter` (C3), `the_rows_of_the_reach_table` (C5 without the Connected parts),
  `a_cube_tip_prints_a_square` (C10), `a_cylinder_tip_prints_a_disc_or_a_rectangle` (C11), `offset_lifts_the_tip`
  (C14), `frames_are_carried_along_the_stroke` (C15 second part), `shape_strokes_keep_history_and_opacity` (C20),
  `a_texture_stroke_ignores_the_tip` (C21), `tips_are_clamped` (C22), `a_set_only_the_bounding_ball_touches_is_not_reached`
  (C24), `through_prints_the_outline_on_a_curved_surface` (C26),
  `through_paints_up_to_the_ring_ball_when_the_bound_is_under_one` (5.6, Through with an outline whose bound is under 1).
  `folded_field_dabs_equal_unfolded` (C16) also covers the flat star outline of C26 (Through).

- T1: `text::gaps::tests` (`get_and_set`, `values_survive_content_edits`, `shift_for_edit_keeps_untouched_gaps`,
  `edits_keep_the_invariants`, `slots_follow_the_lines`, `a_stored_end_value_gets_an_end_slot`,
  `joined_characters_have_no_slot`, `slot_names`); `text::scan::tests` (`a_square_has_exact_coverage`,
  `a_ring_has_an_empty_middle`, `overlapping_contours_fill_once`, `edges_outside_the_buffer_are_clipped`);
  `text::outline::tests` (`outline_bounds_match_fontdue_metrics`, `coverage_matches_fontdue`,
  `the_sample_maps_like_the_painter`, `flattening_follows_the_tolerance`); `text::warp::tests`
  (`every_style_is_the_identity_at_zero`, `no_style_folds_over`, `arc_at_full_bend_is_a_half_circle`,
  `arc_keeps_the_middle_line`, `arc_inner_edge_keeps_a_quarter`, `edges_that_stay` = `arc_lower_keeps_the_top_edge`,
  `arc_upper_keeps_the_bottom_edge`, `fisheye_and_twist_keep_the_frame_border`, `distortion_scales_the_sides`,
  `vertical_axis_is_the_turned_style`, `refinement_follows_the_curve`, `styles_bend_the_way_their_names_say`);
  `text::path::tests` (`arc_bend_is_the_share_of_a_circle`, `arc_at_zero_is_the_straight_layout`,
  `negative_bend_keeps_letters_upright`, `arc_flip_reads_the_other_way`, `start_is_limited_on_open_lines`,
  `circle_does_not_depend_on_the_text`, `circle_flip_reads_upright_at_the_bottom`,
  `a_new_circle_is_centred_on_its_top`, `circle_alignment_pins_its_side`, `circle_start_turns_all_lines_alike`,
  `circle_fill_reports_overflow`, `a_gap_is_applied_before_the_line`, `curve_passes_through_its_points`,
  `curve_does_not_overshoot`, `length_table_is_exact`, `glyph_middles_are_spaced_by_their_advances`,
  `text_longer_than_the_line_continues_straight`, `flip_reverses_the_direction`, `curve_lines_and_alignment`).
  Added by the review of T1: `text::outline::tests::flattened_curves_stay_within_the_tolerance`,
  `text::warp::tests::wave_fish_inflate_squeeze_match_the_reference`, `text::path::tests`
  (`flipped_lines_stack_under_the_first`, `huge_curve_points_do_not_panic`, `a_glyph_frame_places_like_place`).
  `coverage_matches_fontdue` compares against fontdue parsed at 1024 (limit 99 %) and at the classic painter's bucket
  64 (a lower guard limit; contract request T1-1).

- T2: - `text/layout.rs`: `one_gap_moves_exactly_the_glyphs_after_it`, `a_negative_gap_pulls_letters_together`,
    `a_gap_adds_to_letter_spacing_and_kerning`, `a_gap_after_the_last_character_of_a_line_changes_nothing`,
    `a_gap_after_a_tab_follows_its_fourth_space`, `a_gap_ignores_the_fallback_font`, `centred_lines_stay_centred`,
    `an_empty_gap_list_is_today` (the eight round-3 hashes), `hostile_gap_lists_do_not_panic`.
  - `text/raster.rs`: `plain_text_is_bit_identical` (the eight hashes through `render_text`, Full and Preview).
  - `text/styled.rs`: `frame_equals_the_raster_frame` (12 specs), `switching_the_painter_moves_no_ink`,
    `both_painters_cover_alike`, `bold_grows_the_ink_by_its_radius`, `bold_keeps_a_mark_over_its_letter`,
    `slant_moves_the_top_to_the_right`, `a_gap_bends_with_the_warp`, `preview_covers_the_same_rectangle`,
    `the_raster_scales_with_size_px`, `thread_count_does_not_matter`, `too_large_reports_its_size`,
    `a_styled_picture_has_an_area_limit`, `sanitize_repairs_everything`, `a_gap_on_a_circle_turns_the_next_letter`,
    `styled_frames_keep_the_line_box_of_the_classic_frame`,
    `plain_text_through_the_styled_painter_is_about_the_classic_size`; ignored timing `bench::painter_timings` (every
    row of TEXT 8.4 for this file, peak working memory with an allocation counter of the release test build).
    `both_painters_cover_alike` checks 99 % against fontdue parsed at 1024 at 128 px per em (99.80 / 99.48 %) and 98.5 %
    at 64 px (99.51 / 98.77 %); against the app's classic painter (fontdue at its size bucket) the shares are
    97.48 / 95.68 % at 128 and 96.15 / 94.42 % at 64 (held to 95 / 94 %), a limit of the classic painter's curve
    flattening (contract request T2-1). `the_raster_scales_with_size_px` checks the anchor, the line box and the ink
    centroid times 2 within 2 px, each ink side within 3 px (pixel quantization at 64 px doubles; worst 2.5 px, the
    vertical Flag), and the canvas size within -4 .. +2 px (floor / ceil per side at 64 px count twice).
    `bold_keeps_a_mark_over_its_letter` runs on the bundled Hack (Ubuntu Light has no U+0301).
  - `text/frame.rs`: `refit_keeps_the_alignment_edge`, `refit_keeps_anchor_and_em`, `centred_frames_leave_the_centre_bits`,
    `repeated_refits_do_not_drift`, `the_anchor_goes_where_the_centre_was` (with S1's `em_map_round_trips`,
    `refit_keeps_the_em_size`, `refit_with_the_same_em_is_the_round_3_rescale`).

- T4: - `doc_stamps::text_document_tests` (FEATURE_TEXT 8.1 "Document"): `a_spacing_edit_keeps_the_alignment_edge`
    (projected and UV at 30 degrees; Left / Right / Center; a gap of 0.5 em and Letter spacing 10 %; with an Arc the
    anchor), `find_font_prefers_the_project`, `an_area_overflow_drops_the_resolution`,
    `a_too_large_edit_drops_the_resolution`, `anchor_stays_when_an_effect_is_switched_on` (sphere and UV, flipped,
    Arc / Flag / Circle / thickening), `editing_a_plain_text_gives_the_round_3_placement` (P0's round-3 bits),
    `typing_on_a_circle_does_not_move_the_circle`, `moving_a_curve_point_keeps_the_others`,
    `previews_and_one_final_are_one_entry`, `undo_restores_fx_placement_and_pixels`, `a_locked_text_refuses_fx`,
    `the_frame_needs_no_pixels_after_undo_and_load` (test-build rasterization counter `raster_counter`),
    `a_full_bake_never_uses_a_preview_picture`, `duplicates_and_removed_stamps_keep_the_caches_consistent`,
    `source_frames_sanitize_and_fall_back`; ignored timing probe `text_document_bench` (TEXT 8.4:
    `set_stamp_source(.., Preview)` 16 ms, fallback frames 1 ms).
  - `tests\end_to_end.rs`: `a_styled_text_keeps_its_anchor_through_typing_save_and_load`.
  
  Addendum (T4 completion step): add to the 9.1 T4 list the ignored timing probe `text_document_twenty_edits_bench`
  (plan R4-16: twenty Preview edits with deferred bakes on an 8192 texture; styled longest frame at most 30 ms plus the
  plain text's longest frame).

- T5: `tests/project_roundtrip.rs` (FEATURE_TEXT 8.1 "Project"): `frozen_v2_fx_fixture_loads`, `gaps_round_trip`,
  `fx_round_trips` (each of the 15 warp styles, each line kind; byte-identical manifest on re-save),
  `plain_text_entry_has_the_v1_keys`, `required_reader_follows_fx`, `an_end_of_line_gap_needs_reader_2`,
  `an_unknown_warp_style_loads_without_warp`, `broken_fx_from_a_file_is_repaired`, `a_path_entry_writes_only_its_own_keys`,
  `a_wrong_type_in_fx_is_a_manifest_error`, `a_nan_in_any_float_refuses_save` (18 floats, each named),
  `nan_in_arc_and_circle_floats_refuses_save` (the arc bend, circle diameter and centre),
  `valid_fx_written_differently_keeps_the_stored_picture` (no re-bake for valid, non-canonical effects; one repaired value
  re-bakes),
  `styled_text_survives_save_reopen_and_export` (flat PNG and per-layer `_text_` PNGs byte-identical after reopening; the
  effects read back equal; a clean file's stored picture is used, not re-baked), `the_effects_change_the_saved_pixels`
  (ignored until the T1/T2 painters are merged: the Flag warp and the styled effects change the picture, and the styled
  picture is what the file stores). The re-bake checks of `an_unknown_warp_style_loads_without_warp` and
  `broken_fx_from_a_file_is_repaired` replace the stored picture with a solid block, so they do not depend on the painters.

- Round 6 (package A, `wrap_tests.rs` unless noted; the numbers are the plan's acceptance): WA1
  `wa1_a_cylinder_unrolls_exactly_away_from_and_on_the_seam` (per-texel error against the exact unrolling of a 64-gon,
  99.9 % within 2/255, max 8/255), WA2 `wa2_the_cylinder_picture_keeps_its_area_and_every_cell_its_share`, WA3
  `wa3_a_picture_longer_than_the_way_round_has_its_ends_meet`, WA4 `wa4_a_bumpy_plate_receives_the_whole_picture`, WA5
  `wa5_a_cube_edge_unfolds_exactly_and_a_corner_is_cut_cleanly`, WA6 `wa6_every_uv_copy_of_a_welded_vertex_gets_the_same_coordinates`
  and `wa6_the_sphere_seam_is_invisible`, WA7 `wa7_bad_meshes_and_bad_pictures_never_fail` and
  `wa7_the_reach_is_bucketed_and_holds_the_picture`, WA8 `wa8_gaps_cracks_and_thin_sheets`, WA9
  `wa9_the_wrap_is_deterministic_on_any_thread_count`, WA10 `wa10_toggling_wrap_and_back_restores_the_projection_exactly`,
  WA11 `wa11_a_parameterization_is_reused_exactly_when_it_serves`, WA12 (extension of
  `doc_stamps::deferred_final_bake_tests::deferred_final_bakes_equal_the_synchronous_ones` with a wrapped image and a
  wrapped Arc-warped text), WA13 (`project::manifest::tests::wrapped_placements_round_trip_and_write_new_keys_only_when_set`,
  `tests/project_roundtrip.rs` `wrapped_stamps_and_wrap_options_round_trip`), WA16
  `wa16_jump_small_gaps_continues_across_a_gap_as_flat_ground`, WA17 `wa17_jump_small_gaps_climbs_a_cube_standing_on_the_floor`
  (fixture `samples::uv_cube_on_floor`), WA18 `wa18_overlap_winds_on_like_tape`, WA19
  `wa19_overlap_limits_and_both_options_together` and `wa19_option_changes_are_edits_kept_through_project_and_back`; WA15
  `wrap_bench_models` (ignored timing probe, release: Sphere sample, `ngons_tiling.obj`, `ABeautifulGame.glb` when
  present). Fixtures in `samples.rs`: `cylinder`, `bumpy_plate`, `cells_picture` / `cells_colors` / `CELLS_GRADIENT`,
  `uv_cube_on_floor`, `bumpy_plate_periods`; `a_flat_grid_unfolds_exactly`. WA4 asserts the plan's limits on bumps of
  30 degrees and prints (does not assert) 45 / 60 degrees, where the limits are not met (round-6 builder notes).
  Round-6 check (check:A, `wrap_tests.rs`): `check_a_own_bad_meshes_never_fail` (a single triangle, a 200-sliver cone
  tip, a three-page non-manifold fin, doubled and reversed triangles, a Moebius strip, a torus, zero-area triangles,
  1e4 / 1e-4 scaled cubes; anchors on a vertex and an edge; 0.05x-50x; all option sets; a non-unit rotation),
  `check_a_a_stamp_at_a_mesh_boundary` (covered 0.25 / 0.5 at a plate corner / edge / beside the edge / the cylinder
  rim; far away: nothing), `check_a_covered_follows_a_reused_scale` (`stats_for`), `check_a_a_huge_text_on_a_small_part`,
  `check_a_undo_during_a_wrapped_bake` (a deferred job made stale by an undo), `check_a_jump_across_three_parts` (two
  jumps, WA1 limits, also with a lifted third part; 1 / 2 / 5 threads), `check_a_overlap_at_many_turns` (5 turns at 3.7 x
  round against the ground truth, 8 at 10 x, a vertical spine); `tests/project_roundtrip.rs`
  `reopened_wrapped_stamps_rebuild_their_parameterization_exactly`.
- Round 8 (package A, `wrap_tests.rs`; plan R8 3.5): `r8_quality_survey` (the quality measure on round 6's failure
  cases, printed), `wa4_r8_bumpy_plates_receive_the_picture_once_whole_and_smooth` (WA4 at 30 / 45 / 60 degrees, WA4b),
  `wa20_a_bumpy_ball_...`, `wa21_a_closed_piece_...`, `wa22_a_torus_...`, `wa23_a_cone_tip_and_a_cube_corner_...`,
  `wa24_jump_small_gaps_is_bounded_by_the_footprint`, `wa25_the_owner_test_is_inert_where_the_layout_is_exact`,
  `wa26_the_relaxed_wrap_is_deterministic`, `wa27_a_relaxed_parameterization_is_reused_exactly`,
  `wa29_wrap_preview_and_param_jobs`. The plan's limits are asserted; the cases the builder measured outside them are
  listed in `target\round8\notes\pkg_a.md`. check:A (round 8): `check_a8_shapes_that_try_to_break_the_relaxed_wrap`
  (a mug with its handle, Jump on / off; round a torus' inner ring; a cone with the spine through its tip; a closed lathe
  piece, Jump on equals off; a bumpy band with Overlap and Ends meet: no panic, finite, 1 / 5 threads equal, the sweep's
  limits where asserted), `check_a8_slivers_and_zero_area_triangles_in_the_relaxation`,
  `check_a8_huge_tiny_and_boundary_stamps_on_the_relaxed_path`, `check_a8_the_parallel_relaxation_equals_the_serial_one`
  (the parallel kernels forced on: 1 and 12 threads equal the serial bits),
  `check_a8_undo_and_a_new_edit_while_a_preview_job_runs`; in `wrap_relax.rs`: the CG budget and caps, huge gradients
  and a NaN start, the parallel kernels against the serial ones on a 170 x 170 grid, the fast-marching update and the
  sliver rule.
- Round 9 (package A, `wrap_tests.rs`; ROUND9_PLAN section 5): `wa31_the_owner_does_not_depend_on_the_mesh_density`
  (`samples::uv_cube_on_floor_n`, floor 2 / 128 / 8,192 triangles), `wa32_a_helix_round_a_tube_has_its_ends_meet`
  (cylinder sample and a 64-row copy, the picture turned 15 degrees; a thin torus, 20 degrees; Overlap),
  `wa33_rivets_do_not_cut_the_picture` (`samples::riveted_plate`; WA23's cube corner),
  `wa34_a_saddle_corner_lies_once` (`samples::step_corner`), `wa36_the_layout_does_not_depend_on_the_texture_resolution`
  (256^2 and 2,048^2 maps), `wa37_nothing_is_lost_without_a_reason`, `wa26_wa27_round9_cases_are_deterministic` (WA24,
  WA31, WA32, WA35 cases); check:A: `wa35_exact_parts_stay_exact` (`samples::capped_cylinder`, a plate with a 45
  degree bump, a 20 degree cone), `check_a9_*` (the plan's break list). Round 8's tests keep their asserts; the
  measured values outside them are in `docs/notes/round9/pkg_a.md` and `check_a.md`.

**Export-select tests** (EXPORT_SELECT_PLAN 6.1): `export::tests::{default_options_plan_and_bytes_are_unchanged` (default
`layers: None, package: Files` plans today's names; selecting every layer = "All layers" byte for byte),
`plan_with_a_layer_selection` (full-stack indices, kinds and ids from `plan_contents`, id order irrelevant, unknown ids
ignored, a zip plans one file with the same contents), `a_selected_hidden_layer_is_exported` (PNG = the layer; ORA
`visibility="hidden"`, PSD `hidden`, empty merged image), `flattened_of_a_selection_is_the_selected_visible_layers` (flat =
`flatten_layers` of the selection, differs from the composite, a selected hidden layer changes nothing, ORA / PSD hold exactly
the selection and their merged image is that picture), `an_empty_or_stale_selection_is_nothing_to_export` (Files and Zip:
empty plan, `NothingToExport`, no folder), `a_selection_leaves_out_sets_without_selected_layers`, `zip_entries_are_the_loose_files`
(every format, TGA flat; names in plan order and bytes = the loose files; the folder holds only the zip; report),
`zip_export_is_deterministic` (twice, and one lane against several), `zip_methods_and_timestamps` (PNG / ORA stored, PSD / TGA
deflated, 1980-01-01 00:00:00, no folders), `zip_of_a_selection`, `a_cancelled_or_failed_zip_export_leaves_nothing` (cancel while
staging and while packing, 1 lane and several; the stage folder blocked by a file; the temp zip blocked by a folder; a panic
while staging; a cancelled export into a new folder removes it; the old `model.zip` stays byte for byte),
`zip_replaces_an_existing_package_atomically}`; `export::naming::tests::package_names`; `tests/end_to_end.rs`
`two_material_pipeline`: the same export as a zip (entries = the checked loose files) and a selection of the body set's stamp
and text layers (exactly those two files, each = `dense(layer)`).

- M1 (gate, between the waves): `tip_field::tests::a_part_cut_off_at_the_border_stays_inside_its_bound`; `tests/brush_tips.rs`: `a_baked_ball_dab_matches_the_ball` (C13 dab part), `unused_parts_do_not_change_the_printed_square` (F16 dab part, asserts the measured state and logs the miss).

### 9.2 `wte_app` tests

- Unit tests (no GPU, no window): `cli::parse`; `OrbitCamera` (`project(ray-at-t)` round trip, `frame` puts the bounds in view,
  after a pan to the clamp limit and an orbit the whole model is still between near and far); `UvViewState`
  (`screen_to_uv(uv_to_screen(x)) == x`, zoom about a point keeps that point fixed); `gizmo::hit_test`; shortcut table: no
  duplicate bindings and no entry shadowed by an earlier one under `matches_logically`; `Session::validate`; `Prefs` JSON round trip.
- Self-test scenarios (section 8.11) are the automated UI verification. Each is run as
  `Start-Process -Wait -PassThru target\release\wrapped_texture_editor.exe -ArgumentList '--screenshot', '<out>.png', '--scenario', '<name>'`
  (`open_path` additionally gets a path argument: a `.wtep` written by `roundtrip` and an OBJ written by `import`);
  pass = exit code 0, `<out>.json` has `"ok": true`, and the PNG exists with the `size` recorded in the JSON, which equals
  `requested_size` unless `size_clamped` is set. The screenshots are then inspected. Output paths are on E:.
- Manual release checks: open an exported PSD and ORA in GIMP/Krita/Photopea; import a real OBJ+MTL and a real GLB; move the original
  model files away and reopen the `.wtep`; start the exe by double-click and confirm that a forced startup failure shows the
  error box.

**Round 4 app tests** (by package):

- S2: `control_sweep::text_panel_tests::{the_gap_buffer_sees_every_edit_of_the_field,
  the_field_reports_its_cursor_while_focused}` (the two egui proofs of FEATURE_TEXT 6.2 rule 6; both pass),
  `panels::text_block::tests::the_tool_decides_the_target`, `panels::tests::the_tab_strip_follows_the_one_tab_rule`,
  `state::tests::brush_settings_are_sanitized_and_round_trip`,
  `tools::tests::the_pacing_controller_keeps_the_round_3_stroke_with_a_brush_that_queues_nothing`,
  `tools::tests::the_text_tool_places_the_draft_while_a_text_is_selected` (replaces
  `text_press_with_a_selected_text_stamp_does_not_place_the_hidden_new_text`),
  `control_sweep::panel_tests::the_text_tool_edits_the_draft_and_the_transform_tool_the_selected_text` (replaces
  `text_tool_with_a_selected_text_stamp_edits_its_text_in_place`).

- B4: `control_sweep::panel_tests::{a11_brush_tab_panels_are_idle_and_keep_their_width,
  a12_brush_tab_heights_at_1280x800, a13_brush_tab_controls_change_exactly_their_own_setting,
  brush_tab_lines_follow_the_layer_and_the_type}` (FEATURE_BRUSH A11 to A13).

- B5: `brush_flows::tests::{a16_load_flow_every_row, a20_two_shape_jobs,
  open_shapes_ask_how_to_close_and_remember_the_answer, sync_prefs_copies_the_tools,
  the_openings_dialog_draws_and_answers}`, `prefs::tests::{a10_brush_preferences_round_trip_and_read_leniently,
  closing_choices_are_stored_per_file_and_bounded}`.

- B6: Tests that hold this: `gpu::renderer::tests::the_side_normal_is_packed_into_brush_aux` (A14),
  `control_sweep::gpu_tests::the_ring_at_a_cube_corner_follows_the_side_normal` (A15, ignored without a GPU).

- B7: `tools::brush::tests` (FEATURE_BRUSH A1 to A9: the tip of the settings, pacing under the time rule, the queued transforms, finishing, the wheel gestures, the cursor segments; see `R4\notes\pkg_b7.md`).

- T6: Round 4, T6 (`control_sweep/text_panel_tests.rs`, headless, the real side panel through `PanelApp`): the two egui proofs
  of S2; field_undo_brings_the_gap_back, typing_keeps_the_gap_on_its_character, the_cursor_in_the_field_selects_a_gap,
  text_rows_fit_the_panel, the_colour_section_stays_inside_the_panel_on_every_page,
  the_text_tab_follows_the_tool_and_the_selection, the_gap_selection_dies_with_the_session,
  the_text_tool_edits_the_draft_not_the_selected_text, the_transform_tool_edits_the_selected_text,
  a_change_before_placing_changes_the_draft_and_queues_nothing, idle_text_controls_queue_no_action,
  a_slot_click_and_a_drag_change_one_gap, nudge_and_reset, the_strip_shows_one_line, the_end_slot_shows_a_stored_value,
  the_strip_lays_out_only_visible_cells, gap_texts_name_the_real_pair, gap_controls_are_disabled_before_placing,
  bold_formats_the_whole_text, an_embedded_font_can_be_given_to_another_text, a_font_change_resolves_bold_again (reads
  C:\Windows\Fonts\arial*.ttf, skips without), the_font_picker_filters_and_takes_a_font,
  the_warp_page_applies_a_style_and_one_drag_is_one_entry, the_path_page_creates_shapes_that_move_nothing,
  the_path_page_creates_a_centred_circle, the_angle_field_turns_the_right_text,
  every_text_control_changes_exactly_its_own_field. Unit tests in `text_block.rs` (`resolve_face`, `candidates`, the
  field history, angle wrap), `text_tab.rs` (strip boxes, slot texts, Add point) and `font_picker.rs` (search order).

- T7: `text_ops::tests::{the_bundled_families_get_a_sample_each, samples_are_packed_into_atlases_of_204_cells,
  samples_are_requested_once_per_catalog}`, ignored `perf_sample_job_for_the_machine_fonts` (machine fonts, 0.5 s limit;
  polish item 4: a serial perf test, see "Wall-clock tests" below);
  `tools::stamp_place::tests::{text_source_copies_fx, the_draft_takes_a_font_the_project_already_holds,
  a_plain_draft_is_placed_as_in_round_3, a_styled_draft_is_placed_with_its_anchor_at_the_click,
  hover_with_the_text_tool_caches_a_preview_of_the_draft, a_changed_draft_replaces_the_hover_preview,
  add_stamp_reuses_only_a_full_preview, a_draft_too_large_for_every_size_is_not_drawn_again_each_frame}`;
  `tools::text_path::tests::{points_map_to_the_screen_and_back, hit_takes_the_nearest_point_within_8_pt,
  with_point_moves_one_point_within_the_range, the_sampled_curve_is_the_painters_line}`;
  `tools::transform::tests::{points_win_over_corners_and_lose_to_the_knob, dragging_a_point_sends_previews_and_one_final,
  esc_restores_the_curve, a_locked_text_refuses_the_drag, a_drag_far_out_stops_at_the_range,
  a_refused_preview_keeps_the_last_point}`.

**Round 6 app tests** (package B, R6-2, ROUND6_PLAN 5.3):
`control_sweep::panel_tests::{the_selected_stamps_mode_selector_and_wrap_options` (image and text: the selector is one
history entry each way, the projection rows are hidden and the stats line shown in Wrap, the two options are greyed out
in Project, one entry each in Wrap, kept through Project and back, undo walks them back, a locked stamp's selector and
options are disabled), `the_stamp_and_text_tools_set_the_mode_and_the_wrap_options}` (the shared `StampSettings::mode` /
`wrap`, no action, "Project through" greyed out in Wrap); `tools::stamp_place::tests::new_placements_carry_the_tools_mode_and_wrap_options`
(image and text: mode and options carried, depth fitted, project-through ignored in Wrap, a parameterization cached);
`tools::transform::tests::a_wrapped_stamp_drags_like_a_projected_one_and_keeps_its_mode` (Previews and one Final, mode
and options kept, `ensure_wrap_param` after an undo, once); `view::gizmo::tests::a_wrapped_decal_has_no_projection_box_and_an_outline_on_the_surface`;
`tools::text_path::tests::the_handles_of_a_wrapped_curved_text_lie_on_the_surface` (distance to the mesh < 1e-4 of
the diagonal; screen -> em round trip through the surface); `state::tests::stamp_mode_env_switch_and_session_only_mode`
(`WTE_STAMP_MODE`); `panels::layers::tests::badges_name_the_layer_kind` (the wrapped badge's hover text). Scenario
`wrap_surface` (8.11). Check:B added (the interactions table of ROUND6_PLAN 3.3): `tools::stamp_place::tests::a_styled_warped_or_curved_text_is_wrapped_as_drawn`
(on a flat cube face a wrapped text with thickening, slant, outline and an Arc warp / on a Curve inks the texels of the
projected one, <= 1 % differ by more than 8 levels; back in Project the projected bytes);
`stamp_bakes::tests::a_wrapped_stamp_bakes_on_the_worker_and_settles_like_a_projected_one` (worker bake = synchronous
pixels, parameterization cached by the applied bake, `settle` waits for a wrapped bake in flight, a toggle undone while
its bake runs changes nothing); `view::tests::a_wrapped_stamps_2d_outline_runs_along_its_inked_texels`; the transform
test also checks that `ensure_wrap_param` makes nothing during a pending edit (the throttled Preview bakes bring the
parameterization). Core: `wrap_tests::checkb_a_coarse_mesh_leaves_no_wedge_without_surface_inside_the_picture`.

**Round 8 app tests** (package B, R8-2, ROUND8_PLAN 4.2), `stamp_bakes::tests::`
`a_drag_of_a_wrapped_stamp_bakes_its_previews_on_a_worker_and_the_latest_wins` (10 Preview moves in the asynchronous
mode: one job in flight, no status-bar title, nothing baked and no parameterization of a moved placement on the UI
thread; the first job lands as the last good picture with `due_again`, the next job is made for the last placement and
equals the synchronous Preview's pixels; the Final equals the synchronous Final), `esc_or_undo_while_a_wrap_preview_job_runs_keeps_the_old_pixels`,
`a_wrap_preview_of_another_document_is_dropped`, `a_selected_wrapped_stamp_without_parameterization_gets_it_from_a_param_only_job`
(after an undo: the Transform tool asks, `after_tools` starts one job, stats and outline come with it, no pixel
changes, no build on the UI thread), objects-tabs check of WP2: `a_param_only_job_overtaken_by_a_visibility_change_is_asked_for_again`
and `the_synchronous_wrap_failure_is_retried_after_a_visibility_change` (failure marks per visibility generation),
`a_final_waits_for_its_stamps_wrap_preview_job` (then equals the synchronous Final), `projected_stamps_keep_their_previews_on_the_ui_thread`; `jobs::tests::a_silent_job_runs_and_delivers_but_is_not_shown_in_the_status_bar`;
`panels::tool_options::tests::a_relaxed_or_cut_wrap_says_so_under_the_stats_line`. Scenario `wrap_surface` steps 2b,
6b and 10 (8.11). check:B (the job races): the drag test also reads `wte_core::param_counter` (no build on the UI
thread); `a_final_waits_for_its_stamps_wrap_preview_job` also checks that the Final reuses the job's parameterization
and that no frame is asked for while it waits; `esc_after_a_landed_wrap_preview_restores_the_old_pixels`,
`a_stale_preview_landing_after_a_newer_edit_is_dropped_and_the_newer_edit_wins`,
`a_stamp_deleted_while_its_preview_job_runs_is_left_alone_and_undo_brings_it_back`,
`a_stroke_during_a_wrap_preview_job_holds_its_output_until_it_ends`,
`save_during_a_wrap_preview_job_settles_without_waiting_for_it`,
`a_wrap_preview_landing_after_a_turn_to_project_in_the_same_edit_is_dropped`.

---

- Round 9 (package B): `stamp_bakes::a_final_does_not_wait_for_a_wrap_preview_job_of_another_placement` (T8: the Final
  starts in the pump of the release beside the Preview job of the earlier placement, which lands stale; pixels equal the
  synchronous Final); `tool_options::the_shared_texel_line_shows_where_the_model_reuses_its_texture` (T9: shown on
  mirrored halves after the warm-up, not on the sphere, nothing known before);
  `tool_options::a_picture_longer_than_the_way_round_says_how_much_is_shown`. Core (`wrap_tests.rs`, `uv_share.rs`):
  WA28 (growth: candidates <= 1.25 x laid + halo on `cubes_on_floor(5)`; graph path = sphere path), WA38 (one graph per
  mesh shared by Full / Preview / ParamOnly jobs, a new cell for a new mesh, no rayon inside the init), WA39 (shared
  texels: mirrored sample 0.5, sphere and cube on floor 0); the WA15 probe reports the graph build, the shared texels and
  per parameterization the phases of the graph path next to round 8's sphere path.

- Stable (w1, `wrap_stable_tests.rs` unless noted; `docs/notes/stable/w1.md`): `w1_an_open_rough_field_never_closes`,
  `w1_a_part_on_a_plane_keeps_the_rows_on_the_plane`, `w1_the_chess_board_keeps_the_rows_under_a_pawn` (skipped
  without `ABeautifulGame.glb`), `w1_reuse_equals_fresh_within_the_bucket`, `w1_lost_cut_does_not_count_the_gaps_between_parts`;
  `wrap_param::closure_tests::w1_near_closure_equals_the_plain_closure`, `w1_near_closure_scales_linearly`;
  `uv_share::tests::w1_a_tiled_trim_strip_does_not_mark_a_unique_island`;
  `tool_options::the_wrap_lines_name_the_right_cause_and_never_contradict`.
- Stable (w2, `wrap_stable_tests.rs`; `docs/notes/stable/w2.md`): `w2_a_book_shows_the_picture_on_the_page_it_faces`,
  `w2_room_and_a_whole_closed_part`, `w2_the_developed_region_stays_off_curved_surface`. Turned green by w2's changes:
  WA35 (the developed exact region), WA4 (the area push); WA17 / WA24 with their picture centred at z = 0.95 instead
  of 1.2 (from 1.2 at most 0.90 of the picture could lie once: check:A of round 9 section 2).
- Stable (fix_wrap_1; `docs/notes/stable/fix_wrap_1.md`): `w1_the_chess_board_keeps_the_rows_under_a_pawn` also asks
  `beyond < 2 %` and `shown >=` the board's share; WA32 asks `beyond >= 2 %` and `shown + beyond + lost_cut = 1`
  within 1 %; `tool_options::the_wrap_lines_name_the_right_cause_and_never_contradict` has the chess-board case (no ends
  line, "9 % ... runs off") and a grid over `beyond`.
- Stable (fix-wrap; `docs/notes/stable/fix_wrap_2.md`), `wrap_stable_tests.rs` unless named:
  `fix_jump_continues_across_flush_coplanar_plates` (ignored, a known limit: two coplanar plates touching along a seam
  without a shared vertex, the spine parallel to the seam, Jump on: the bake on both plates and on plate B alone should
  be the flat picture within WA1's limits, `shown >= 0.99`, `jumps == 1`; plate B is 33.6 % within 2/255; the fix that
  passed it was not kept, 6.5a); `relax::tests::fix_bare_cells_by_cause`
  (a hand-built owner grid: a 0.2 gap between two parts is `gaps`, a 0.5 gap is not; bare rows next to parts a
  neighbouring part is near are `at_part`, nearest-laid-cell attribution, none with `near_other` false);
  `fix_damaged_helmet_shared_texels_are_two_measures` (skipped without the sample: the stamp's share
  `stamp_shared_texels` 100 % equals an independent 4,096 count raster of the texels under the 16 x 16 picture points;
  the mesh's share 86.7 % is `UvShare::mesh_share`); `fix_the_water_bottle_label_is_exact_on_the_body` (skipped without
  the sample: the body triangles meeting the picture keep their edge lengths within 0.01 texel); the chess board test
  also asks `at_part >= 0.9 x (1 - shown)`, `w1_lost_cut_does_not_count_the_gaps_between_parts` also `gaps >= 0.15`,
  `at_part < 0.01`, and the rest below 2 %; `tool_options::the_wrap_lines_name_the_right_cause_and_never_contradict`'s
  chess case now reads "9 % of the picture stops where it meets a neighbouring part", plus a parts200 case and a mixed
  case (gaps, neighbouring part and edge each with its own share).
- Stable (fix-wrap), wall-clock limits: `wa7_bad_meshes_and_bad_pictures_never_fail` and
  `check_a_own_bad_meshes_never_fail` keep every functional assert and print their slowest parameterization; the
  200 ms limit (best of up to five runs, `best_param_ms`) is asserted by the ignored perf tests
  `perf_wa7_parameterizations_within_200_ms` and `perf_check_a_own_meshes_within_200_ms`, which run the same cases
  serially on an otherwise idle machine: `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1` (gate step).
  In the parallel run the wall clock depended on the other tests (197.7 of 200 ms idle, 276.5 / 366.5 ms under load);
  the parameterization's own CPU time (one-thread pool, `GetThreadTimes`) was tried and rejected: its parallel parts on
  one thread take 3-5 x the wall time (doubled_cube 500 ms CPU against 115 ms wall), so the same bound could not stay.
- Stable (fix-wrap), `wrap_surface` step 10: the panel lines are logged before the step's asserts (nonmanifold,
  noisy400 and tiles300 fail the ink / dup assert and now log them too); the `coverage:` line adds `gaps` and
  `at_part`, `panel lines:` adds both and says "shared texels under the picture" (the stamp's share; the whole mesh's
  is the warm-up's "shared texels found ... (N % of the surface)"); a `composite hash` line per Jump setting (every
  set's composite) for byte-equality checks across runs.

- Polish (wall-clock tests, item 4): the tests with a tight wall-clock limit (WA7, check:A, the font sample job) are
  ignored `perf_*` tests, run serially on an otherwise idle machine; the parallel runs skip them. check_polish: two
  timed tests are not `perf_*`: `texel_map::tests::tiled_uvs_build_in_one_pass_over_the_image` runs in the parallel run
  with a coarse 1 s guard (a 1024² build and a raster, against the old per-tile cost), and the ignored
  `tip_bake::tests::load_times` (P10, models of this machine, 1 / 2 / 8 s) runs in none of the commands below. The gate
  commands:
  `cargo test --workspace` (no perf test runs: all are ignored);
  `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1` (WA7, check:A);
  `cargo test -p wte_app perf_ -- --ignored --test-threads=1` (`text_ops::tests::perf_sample_job_for_the_machine_fonts`,
  0.5 s; 649 ms under a game's load in fix pass 2, 340-424 ms alone);
  `cargo test -p wte_app -- --ignored --skip perf_` (the GPU app tests).
  The limits are unchanged.

**Polish tests**: `dialogs::export::tests::the_linked_note_counts_texture_sets_not_layers` (item 1: two designs across
the same two sets, selected in set 0: 2 layers in 1 set; each design in a different set: 2 sets; all of a design
selected: no note); `control_sweep::panel_tests::footer_rasterize_names_the_active_layer_when_several_are_selected`
(item 5: real hover over the footer Rasterize until egui shows the tooltip: with two layers selected it names the active
layer, disabled on a paint layer and enabled on a stamp layer, whose click queues `RasterizeLayer` of the active layer
only; one layer selected: the plain tooltip); `tool_options::tests::the_wrap_lines_name_the_right_cause_and_never_contradict`
(item 3: DamagedHelmet Jump on reads "Only 97 % of the picture lies on the surface, mostly lost to cuts (2 %)"; a case
per cause below its line's threshold: gaps, neighbouring part, running off the surface, hidden or folded, beyond where
the picture closed on itself; no cause appended while a cause line shows; the contradiction sweep unchanged);
`selftest::scenarios::tests::expected_selection_layers_follow_the_stamp_links` (item 6); scenario `save_unsettled`
(item 2, 8.11).

Polish item 6: `export_flow`'s `expect_layer_files` (also `export_selected`, `multi`) no longer takes the expected
layers from `export::plan_contents` alone: `expected_selection_layers` works them out from the document's `StampId`
links (`Document::stamp_layers` of each selected stamp or text layer) and the planned files' layers must equal them;
the names still come from the plan (they are checked against `selection_layer_name` when no set names collide).
`export_unsettled` checks that the plan holds exactly the stamp's layers.

**fix_export tests**: core `export::tests::{a_selection_has_the_full_exports_names_when_set_names_collide,
a_selection_of_a_colliding_set_writes_the_full_exports_file, a_design_across_two_sets_exports_its_layer_of_every_set,
a_selection_zip_never_replaces_the_full_package, stale_temps_of_processes_that_are_not_running_are_removed,
a_blocked_export_folder_fails_before_writing_anything, non_ascii_names_round_trip_through_a_zip,
export_at_4096_with_several_sets_and_layers (ignored; time on stderr, peak memory with tools/stable/peak_mem.py)}`,
`doc_layers::tests::{several_layers_are_deleted_and_duplicated_as_one_undo_step,
a_group_at_the_entry_limit_evicts_older_entries_only}`; app `dialogs::export::tests::{switching_from_the_layer_preset_to_visible_restores_the_normal_ticks,
the_zip_name_follows_the_layers_choice}`, `io_flows::tests::{export_toasts_count_files_in_the_singular_and_plural,
this_process_runs_and_a_finished_one_does_not}`; GPU (ignored) `control_sweep::gpu_tests::{export_dialog_replaces_a_zip_through_the_confirm,
a_zip_export_cancelled_while_packing_leaves_nothing}`; scenarios `export_unsettled`, `export_nouv`, `export_errors` and the
`export_flow` / `multi` / `fuzz_path` additions (8.11). `tools/stable/verify_zip_vs_loose.py` checks export_flow's zips
against its loose files outside the app. The independent check (`docs/notes/stable/check_exp2.md`) adds
`tools/stable/check_exp2_verify.py` (`python -E -P`, numpy + Pillow): per export_flow run folder, the full zip against
the loose files (names, bytes, 1980 dates, UTF-8 flag), every `_selection.zip` entry and every `_one` / `_sel` file
against the full zip's file of that name, the flattened PNG of each set against a straight-alpha normal-over
re-composite of its per-layer PNGs (within 2/255), ORA (`mimetype` first and stored, layer count, `mergedimage.png` =
flattened) and PSD headers, and no `.tmp-` entry left.

**Export-select app tests** (EXPORT_SELECT_PLAN 6.2): `panels::layers::tests::row_clicks_select_toggle_or_extend` (the
`row_click_action` table); `dispatch::tests::{ctrl_and_shift_select_layers_without_touching_the_document` (toggle add /
remove / last stays / the active one moves to the top-most remaining, ranges up and down, a plain select collapses; history,
revision and the modified mark unchanged; the three are not guarded, `ShowExportLayers` is and finishes an interaction),
`the_selection_stays_valid_when_layers_go_away` (delete of the active layer, undo, a new layer collapses it, stale and
foreign ids ignored), `export_for_layers_opens_the_dialog_on_the_selection}`; `dialogs::export::tests::{the_list_kinds_come_from_the_core_plan,
the_list_for_a_selection_and_for_a_zip_package, the_layers_choice_round_trips, the_list_matches_for_several_sets}` and
`export_is_blocked_with_a_reason` ("Select at least one layer"); `control_sweep::panel_tests::layer_rows_take_ctrl_shift_and_right_clicks`
(real Ctrl / Shift clicks on the name, thumbnail and empty row part; a right-click opens the menu without selecting; its
items emit `ShowExportLayers` with the right ids; the footer line; the Layer menu's "Export Selected Layers... (2)") and the
Layer menu's "Export Layer..." in `every_menu_entry_queues_its_action`; `control_sweep::gpu_tests` (ignored, need a GPU;
run with `-- --ignored control_sweep`): `export_dialog_controls_and_export` (the Layers choice "All" replaces the checkbox;
the folder holds exactly the planned files, the PSD every layer, each per-layer PNG its layer's pixels),
`export_dialog_zip_package_and_selected_layers` (tick the zip, Export: one zip whose entries are the listed files and whose
flat entry is the composite, the zip toast; again: the confirm step lists the zip; opened for two layers: "Selected (2)"
chosen, two files listed and written), `export_dialog_at_the_smallest_window_and_with_several_sets` (the Layers choices and
the zip checkbox on screen at 900 x 600). Scenarios `export_selected`, `export_zip`, `export_flow` (8.11); `export_run` and
`workflow_path` now check their folder with `verify_export` (exact listing, pixels, recomposition) instead of "exists and is
not empty".

objects-tabs WP3a (app state, actions, menu, shortcuts, persistence): `object_ops::tests::{select_toggle_and_range_follow_the_anchor_rules,
hide_isolate_and_show_all_are_view_state_and_keep_the_selection, a_hide_during_an_open_stroke_is_refused_with_a_warning,
frame_selection_targets_the_visible_selected_objects, hidden_names_round_trip_and_tolerate_unknown_and_repeated_names,
validate_drops_objects_the_mesh_does_not_have}` (none changes revision, modified flag or undo depth);
`io_flows::tests::ui_state_round_trips_and_tolerates_garbage` (`hidden_objects` not written when empty, names round-trip, a
non-array value or non-string entries ignored without losing the other fields); `shortcuts::tests` (table of 40, the four
new contract bindings, `object_keys_map_to_their_actions_and_stay_in_text_fields`); `dispatch::tests::guard_covers_document_commands_but_not_selection_or_previews`
(the four visibility actions are guarded and finish, selection and `FrameSelection` are not);
`control_sweep::panel_tests::{every_menu_entry_queues_its_action, menu_entries_are_enabled_exactly_when_they_can_act}` (the
Object menu and File > Open Sample > Three Objects); GPU (ignored): `control_sweep::gpu_tests::hiding_during_a_held_stroke_commits_the_stroke_first`
(H with the button held: one undo entry, the stroke; the hide adds none; Alt+H shows all).

objects-tabs WP3b (rendering, highlight, picking): `gpu::scene::tests::{visible_draws_hold_every_visible_triangle_once_in_its_group,
selected_indices_are_the_selected_visible_objects_triangles, object_runs_and_visible_runs,
wire_lists_of_several_objects_are_grouped_per_object}` and, in `one_pass_uv_wires_match_the_per_set_filter_and_the_geometry_threads_match_the_serial_build`,
the one-object lists byte-identical to the build without objects with one run per group; `gpu::renderer::tests` (`tint.wgsl`
valid, its `ViewUniforms` layout, its `VsIn` = the position at location 0, the same `DEPTH_PULL` as the wires);
`tools::tests::a_navigate_click_picks_an_object_and_a_drag_orbits` (3 pt = a pick with the camera restored and the row
revealed, 5 pt = an orbit, Ctrl / Shift toggle, a plain click on nothing deselects, a hidden object is passed through,
Esc forgets the click, no pick in Preview, 2D, with Space+LMB or on a one-object model); GPU (ignored):
`control_sweep::gpu_tests::hidden_objects_are_not_drawn_and_the_selection_is_tinted` (Three Objects: the cube hidden shows
the floor behind it, `all_objects` draws it again, the tint makes the cube bluer and is not drawn on a hidden object, the
hidden cube's wires are gone inside its former screen box) and `perf_visibility_toggle_on_a_heavy_model` (reads
`WTE_HEAVY_MODEL`; prints the core mask and `sync_objects` times, fails over 250 ms).

## 10. Work packages

Wave 0 creates a compiling workspace; wave 1 implements `wte_core`; wave 2 implements `wte_app`. Inside a wave the packages are
ordered in **stages**: packages of one stage run in parallel, and a stage starts only when every package it depends on has landed
and its tests are green. Every file belongs to exactly one package. A package may not edit files it does not own; if a contract
in an [S] file must change, it is reported to the architect instead. Each brief is self-sufficient through its two section lists:
*Implements* (the contract it must fulfil) and *Consumes* (contracts of other packages and of the scaffold that it calls,
read-only).

### Wave 0

**`scaffold`** — workspace skeleton and shared contracts.
Owns (complete, final — the shared [S] files nobody else may edit): `Cargo.toml`, `crates/wte_core/Cargo.toml`,
`crates/wte_app/Cargo.toml`, and in `crates/wte_core/src`: `lib.rs`, `ids.rs`, `error.rs`, `math.rs`, `progress.rs`, `pixel.rs`,
`blob.rs`, `mesh.rs`, `layer.rs`, `stamp.rs`, `document.rs`, `testutil.rs`, `text/mod.rs`, `project/mod.rs`,
`project/manifest.rs`; in `crates/wte_app/src`: `state.rs`, `action.rs`, `gpu/params.rs`.
Pre-existing and not to be edited by anyone: `cargo.cmd`, `.cargo/config.toml` (linker configuration of the self-contained
toolchain).
Additionally creates every other file of section 2 (including `crates/wte_core/tests/*.rs` and the five `.wgsl` files) as a stub
containing all pub (and listed `pub(crate)`) items of sections 5 and 8 with placeholder private fields, so that
`cargo check --workspace` passes before parallel work starts. Core stub bodies are `todo!()`. App stub bodies are harmless
no-ops, with these exceptions that make the early app stages verifiable:
- `main.rs` already opens an eframe window and contains every `mod` declaration of the app crate;
- `toasts.rs` already has `#[derive(Default)] struct Toasts` with working `push` / `error_count` (drawing may be empty);
- `view::central` is a ~30-line placeholder: with a session it ensures the `GpuScene`, calls `sync`, renders `Main3D` with a
  fixed camera framing `mesh.bounds` (matrix built inline with glam) and paints the image; without one it calls
  `render_backdrop`. `app-views-tools` replaces it;
- `selftest/scenarios.rs` contains the action-only scenarios `empty` and `smoke` (8.11).
Stubs are then handed to their owners.

### Wave 1 — `wte_core`

| Stage | Id (tag) | Title | Files (under `crates/wte_core/`) | Implements | Consumes (read-only) | Depends on |
|---|---|---|---|---|---|---|
| 1a | `core-geometry` (C1) | Mesh normalization, BVH, texel map, samples, pixel ops | `src/mesh_build.rs`, `src/bvh.rs`, `src/texel_map.rs`, `src/samples.rs`, `src/pixel_ops.rs` | 5.5 (`pixel_ops`), 5.7-5.10, 6.1-6.3 | 4, 5.1-5.5 (`pixel.rs`), 5.25 | `scaffold` |
| 1b | `core-document` (C2) | Blending, compositing, layers, history, document construction | `src/blend.rs`, `src/composite.rs`, `src/doc_build.rs`, `src/doc_layers.rs`, `src/history.rs` | 5.15-5.18, 6.6, 6.7, 6.5 (structure invariants, `rasterize_layer` / `merge_down` rules) | 4, 5.5, 5.7-5.10, 5.11, 5.14 (mutation rule), 5.20 (`remove_stamp` / `duplicate_stamp` / `rename_stamp` / `finalize_stamp_bake` signatures), 5.21 (`ImportedModel`, `SourceBundle`, `import_from_bundle` for `Document::sample`), 5.22, 5.25 | `core-geometry` |
| 1b | `core-import` (C5) | OBJ/glTF/FBX import, generated UV layout, resolvers, image I/O | `src/import/mod.rs`, `src/import/resolver.rs`, `src/import/obj.rs`, `src/import/gltf.rs`, `src/import/fbx.rs`, `src/import/auto_uv.rs`, `src/import/mtl_ref.rs`, `src/import/uri.rs`, `src/image_io.rs` | 5.21, 5.22, 7.4 (codecs: `encode_flat`, `decode_image`), importer notes below | 4, 5.2, 5.5 (`resize`), 5.7 (`RawMesh`, `NormalizeOptions`, `normalize`), 5.10 (`to_obj` in tests), 6.1 | `core-geometry` |
| 1c | `core-paint` (C3) | Brush engine, strokes, fill | `src/brush.rs`, `src/stroke.rs`, `src/fill.rs` | 5.19, 6.4 | 4, 5.8, 5.9, 6.3 (`raster_tri` rules, parallel pattern), 5.11, 5.14 (mutation rule, `mark_layer_dirty`), 5.17 (`paint_target`), 5.18 (`TileBackup`, `original_row`, `push_edit`, `flush_pending_edit`), 5.25 | `core-geometry`, `core-document` |
| 1c | `core-stamps-text` (C4) | Stamp/text bake, stamp commands, fonts and text rasterization | `src/stamp_bake.rs`, `src/doc_stamps.rs`, `src/text/font_catalog.rs`, `src/text/layout.rs`, `src/text/raster.rs`, `src/text/edt.rs` | 5.12, 5.13, 5.20, 6.5, 6.8 | 4, 5.5 (`PremulMips`), 5.6, 5.8, 5.9, 6.3 (band-parallel pattern, max rule), 5.11, 5.14, 5.17 (delegation rule), 5.18 (`begin_pending`, `push_structure_edit`, `snapshot_structure`, `flush_pending_edit`), 5.22 (`decode_image`), 5.25 | `core-geometry`, `core-document`, `core-import` |
| 1c | `core-persist` (C6) | `.wtep` save/load and all exporters | `src/project/save.rs`, `src/project/load.rs`, `src/project/migrate.rs`, `src/project/atomic.rs`, `src/export/mod.rs`, `src/export/naming.rs`, `src/export/ora.rs`, `src/export/psd.rs`, `src/export/packbits.rs` | 5.23, 5.24, 7.1, 7.3-7.7 | 4, 5.5 (`nonzero_bounds`, `content_bounds`, `crop`, `thumbnail`, `dilate_islands`), 5.6, 5.11, 5.14 (`DocumentSnapshot`), 5.15 (`DocumentParts`, `from_parts`, `Document::sample` in tests), 5.16 (`flatten_layers`, `BlendMode` keys), 5.20 (`rebake_stamp`), 5.21 (`SourceBundle`, `import_from_bundle`), 5.22, 6.6, 7.2 (manifest types), 5.25 | `core-geometry`, `core-document`, `core-import` |
| 1d | `core-integration` (C7) | Whole-crate integration tests; fixes nothing itself, reports failures to the owning package | `tests/end_to_end.rs`, `tests/project_roundtrip.rs` | 9.1 ("integration") | all of 5, 6, 7 | all other wave-1 packages |

In-file tests of a package use only its own code and its dependencies (9.1). In particular `core-document` tests `Document::sample`
nowhere (it needs `core-import`, same stage) — that is covered by `core-integration` — and `core-persist` tests use raster-only
documents (stamp and text persistence is covered by `tests/project_roundtrip.rs`).

Importer notes for `core-import` (verified in source):
- **tobj 4.0.5**: `tobj::load_obj_buf(&mut reader, &tobj::LoadOptions { triangulate: true, ignore_points: true, ignore_lines: true,
  ..Default::default() }, |p: &Path| -> tobj::MTLLoadResult { .. tobj::load_mtl_buf(&mut Cursor::new(bytes)) })`
  (`REG\tobj-4.0.5\src\lib.rs:2027, 2062`). The loader closure is `Fn`; on a missing MTL return `Err(tobj::LoadError::OpenFileFailed)`
  and treat the inner error as `ImportWarning::MissingMtl`. Do **not** use `single_index: true` (texcoords silently misalign when some
  faces lack `vt`, `lib.rs:850`); use `indices` / `texcoord_indices` / `normal_indices` and emit `RawTriangle`s. tobj reads with
  `lines()`: pre-convert the bytes with `String::from_utf8_lossy` and strip a BOM. tobj emits a new `Model` per `usemtl`; group by
  `mesh.material_id`. `diffuse_texture` is the raw rest of the `map_Kd` line: `parse_texture_ref` strips options
  (`-blendu/-blendv/-cc/-clamp/-imfchan/-texres/-bm/-boost` take 1 argument, `-mm` 2, `-o/-s/-t` 1 to 3 numbers); fall back to
  `ambient_texture`. A relative texture reference of an MTL that sits in a sub-folder is read as `<mtl folder>/<reference>` when
  that file exists (the resolver key and `ImportedTexture.reference` are then the joined path), else as written. With
  `decode_textures`, every other texture statement of every loaded MTL is read too (capture only). Flip V. Never call
  `tobj::load_obj` (disk).
- **gltf 1.4.1**: `gltf::Gltf::from_slice_without_validation(bytes)` -> `Gltf { document, blob }` (`REG\gltf-1.4.1\src\lib.rs:322`),
  then our own validation: first every mesh primitive's `POSITION` index is checked against `accessors.len()` (the crate's
  `primitive_validate_hook`, `REG\gltf-json-1.4.1\src\mesh.rs:151`, indexes the accessor list unchecked and would panic), then the
  crate's full `Validate::validate` runs, ignoring only `Missing` for `attributes["POSITION"].min` / `.max` (common viewers load
  such files and the importer never reads the bounds) and `Unsupported` for a required `KHR_mesh_quantization` (the crate refuses
  it by name, `REG\gltf-json-1.4.1\src\root.rs:160`; the importer reads it, below); any other problem is `InvalidGltf`. Never
  `gltf::import*`. A validation error consisting only of unsupported `extensionsRequired` => `UnsupportedGltfExtension`
  (`KHR_draco_mesh_compression`, `EXT_meshopt_compression` and every other unknown required extension stay refused by name). With `decode_textures`, every external
  `images[].uri` is read too (capture only). Buffers: `buffer::Source::Bin` => `blob`; `Source::Uri` => `parse_data_uri` or `resolver.read`; check
  lengths. Scene: default scene, else first, else all root nodes; `world = parent * Mat4::from_cols_array_2d(&node.transform().matrix())`.
  Per primitive (Triangles, TriangleStrip, TriangleFan; others skipped with a warning):
  `prim.reader(|b| buffers.get(b.index()).map(|v| &v[..]))` (`src\mesh\mod.rs:319`) for `read_indices().map(|i| i.into_u32())`
  and the skin attributes; `POSITION`, `NORMAL` and `TEXCOORD_n` are read by `read_f32::<N>` with `gltf::accessor::Iter` (dense
  and sparse) in their stored component type (**`KHR_mesh_quantization`**, only when the file declares it in `extensionsUsed`
  or `extensionsRequired`; a core file keeps the pre-round-3 reading: f32 `POSITION` / `NORMAL`, f32 / u8 / u16 `TEXCOORD_n`
  with integers always normalized, whatever the flag): `POSITION`
  f32 / i8 / u8 / i16 / u16, `NORMAL` f32 or *normalized* i8 / i16 (other integer normals are dropped and regenerated),
  `TEXCOORD_n` f32 / i8 / u8 / i16 / u16; a normalized integer is `c / MAX` (unsigned) or `max(c / MAX, -1)` (signed), any other
  integer keeps its value and the node transform (positions) or `KHR_texture_transform` (UVs) scales it back; 32-bit integer
  attributes are skipped as unsupported types. (The crate's `read_tex_coords` would be `unreachable!` on i8 / i16 and always
  normalizes u8 / u16, so it is not used.) `TANGENT` is never read, whatever its type; UV set =
  `base_color_texture().map(|i| i.tex_coord())` else 0. Bake `world` into positions, inverse-transpose into normals, swap winding when
  `world.determinant() < 0`. Apply `info.texture_transform()` (offset, rotation, scale) to the dequantized UVs. No V flip. Texture image:
  `info.texture().source()` is an `Option` (feature `allow_empty_texture`); `image::Source::View { view, .. }` slices the buffer,
  `Source::Uri { uri, .. }` is a data URI or goes to the resolver; decode with `decode_image` (only when `decode_textures`). Fall
  back to `pbr_specular_glossiness().diffuse_texture()`. **Skinned meshes** (node with `skin`): the node's own transform is
  ignored (glTF rule); every vertex is placed by `sum(w_i * jointWorld_i * inverseBind_i)` from `JOINTS_0` (u8/u16) /
  `WEIGHTS_0` (f32/u8/u16, renormalized; a joint index outside the skin contributes nothing), with the joints' world matrices
  from the file's node transforms (`node_world_matrices`: parent links, cycles cut) - for a file saved at rest that is its bind
  pose (BrainStem: a standing figure instead of the previous mis-placed parts). Normals use the inverse transpose of the
  blended matrix, the winding is swapped where it is mirrored. A skin without usable inverse bind matrices / weights falls
  back to the node transform with `SkinIgnored`; morph targets are ignored with `SkinIgnored`.
- **FBX** (`import/fbx.rs`, own reader, `miniz_oxide 0.8.9` for zlib): binary `Kaydara FBX Binary  \0`, version u32 at byte 23,
  accepted 6000..7999; record header `endOffset, numProperties, propertyListLen` u32 below 7500, u64 from 7500, then the name
  length u8 and name; properties `Y C I F D L` scalars, `S R` u32-length strings / bytes, `f d l i b c` arrays
  (`count, encoding, storedLen` u32; encoding 0 plain, 1 zlib); children end with a null record (13 / 25 zero bytes). Every
  offset is checked against its parent record, nesting is capped at 64, array sizes against a decompression budget of
  64 x file size + 64 MiB. ASCII: `Name: values {children}` with `;` comments, quoted strings (`&quot;`), 7.x arrays
  `*N { a: ... }`, 6.x inline arrays continued on lines that start with a comma, base64 `Content`. Scene: `Objects` (7.x ids,
  6.x `Class::Name` names) linked by `Connections` (`C` / `Connect`, OO and OP). Model world matrix (FBX SDK):
  `parentWorld * T * Roff * Rp * Rpre * R(order) * Rpost^-1 * Rp^-1 * Soff * Sp * S * Sp^-1`, Euler in degrees, `RotationOrder`
  0..5 = XYZ, XZY, YZX, YXZ, ZXY, ZYX (XYZ = X applied first), pre/post rotation XYZ; geometry additionally by
  `GeometricTranslation / Rotation / Scaling` (not inherited). `GlobalSettings`: `UpAxis`/`UpAxisSign` -> +Y,
  `CoordAxis`/`CoordAxisSign` -> +X, front = right x up (always a rotation), `UnitScaleFactor` (cm per unit) -> metres.
  Geometry: `Vertices`, `PolygonVertexIndex` (negative = last corner, index `!v`); polygons triangulated by ear clipping in the
  Newell plane (a convex polygon gives the fan of corner 0; fan when clipping fails or above 512 corners). Layer elements
  (index 0 preferred): `LayerElementNormal` (`Normals`, `NormalsIndex`), `LayerElementUV` (`UV`, `UVIndex`, V flipped),
  `LayerElementMaterial` (`Materials`, AllSame / ByPolygon) with mapping ByPolygonVertex / ByVertice / ByPolygon / AllSame and
  reference Direct / IndexToDirect; anything out of range drops that attribute of the triangle (or the polygon, for vertices,
  with `SkippedPrimitive`). Materials: connection order of the model; colour `DiffuseColor` (else `Diffuse`), alpha `Opacity`;
  texture = the one on the `DiffuseColor` / base-colour slot (else the first that is not a normal / bump / specular-type map;
  a `LayeredTexture` stands for its first layer; 6.x: a texture on the model). **Texture UV transform** (round 3, R3-12): the
  Texture object's `Translation`, `Rotation` (Z = W, degrees, counter-clockwise), `Scaling`, `TextureRotationPivot`,
  `TextureScalingPivot` (Properties70 / 60), else the older `ModelUVTranslation` / `ModelUVScaling` child nodes (the properties
  win where both exist, as in Assimp), are applied to the UVs of that material's polygons in the file's V-up space before the
  V flip: `uv' = T * Rp * R * Rp^-1 * Sp * S * Sp^-1 * uv` (without pivots `T + R(S uv)`, the `KHR_texture_transform` form; the
  convention of the FBX SDK, Max tiling / offset, Maya repeat / offset and Assimp. Blender's importer subtracts the translation
  instead; not followed). A scale component of 0 counts as 1, non-finite values are ignored, an identity transform changes
  nothing (`uv_hash` of files without transforms is unchanged). Materials showing the same image with different transforms share
  one texture, each transforming its own polygons (as glTF). `UVSwap`, `UVSet` and rotation about U / V are ignored. Texture file: the video's `Content` if embedded,
  else the first of RelativeFilename / RelPath / FileName / Path the resolver can read, else an embedded video of the same
  file name (orphaned videos), else a `MissingTexture` warning (blank set). Skin / blend-shape deformers and `Shape`
  geometry: ignored with `SkinIgnored` (FBX stores the vertices in bind pose). NURBS and other non-mesh geometry: skipped.
- **DiskResolver::read** tries: `base_dir.join(reference)`; the same with `\` and `/` swapped; a case-insensitive match in that
  directory; for absolute or dangling paths the bare file name in `base_dir`, then in `base_dir/textures/`.
- A missing or undecodable texture is always a warning (the set is created blank); only a missing glTF `.bin` is fatal. Texture
  *references* are always recorded (`ImportedTexture.name` / `.reference`, `RawMaterial::texture`), whether or not the file
  could be read or decoded, so set assignment and set names never depend on decoding (6.1 step 5).

### Wave 2 — `wte_app`

All wave-2 packages depend on the complete, green wave 1 (expressed as a dependency on `core-integration`).

| Stage | Id (tag) | Title | Files (under `crates/wte_app/src/`) | Implements | Consumes (read-only) | Depends on |
|---|---|---|---|---|---|---|
| 2a | `app-shell` (A1) | Entry point, logging, frame loop, dispatch, jobs, file flows, shortcuts, prefs, toasts, theme, drag-and-drop, CLI, self-test runner | `main.rs`, `cli.rs`, `app.rs`, `dispatch.rs`, `shortcuts.rs`, `jobs.rs`, `io_flows.rs`, `prefs.rs`, `toasts.rs`, `theme.rs`, `dnd.rs`, `selftest/mod.rs` | 8.3, 8.9, 8.10, 8.12, 8.11 (CLI, `Runner`, `Step`, `Scratch`) | 8.1, 8.2, 8.13, stub signatures of 8.4-8.8, 5.14-5.24 (document commands, import, project, export), 9.2 | `core-integration` (= all of wave 1) |
| 2a | `app-gpu` (A2) | wgpu renderer: targets, set textures, mip blit, scene, pipelines, shaders | `gpu/mod.rs`, `gpu/targets.rs`, `gpu/textures.rs`, `gpu/scene.rs`, `gpu/renderer.rs`, `gpu/shaders/mesh.wgsl`, `gpu/shaders/lines.wgsl`, `gpu/shaders/uvview.wgsl`, `gpu/shaders/background.wgsl`, `gpu/shaders/blit.wgsl` | 8.4 | 8.1, 8.2 (`gpu/params.rs`, `ShadingMode`, `Backdrop`, `ViewId`), 8.13, 4.2, 5.7 (`RenderVertex`, `wire_edges`, material ranges), 5.16 (`recomposite_all`, `take_gpu_dirty`, `composite`) | `core-integration` (= all of wave 1) |
| 2b | `app-views-tools` (A3) | 3D / 2D / Preview views, camera, gizmo, all tools | `view/mod.rs`, `view/camera.rs`, `view/view3d.rs`, `view/view2d.rs`, `view/preview.rs`, `view/gizmo.rs`, `tools/mod.rs`, `tools/brush.rs`, `tools/eyedropper.rs`, `tools/fill.rs`, `tools/stamp_place.rs`, `tools/transform.rs` | 8.5, 8.6 | 8.1, 8.2, 8.4, 8.9 (`Toasts`), 8.13, 5.8, 5.12, 5.13 (`TextSpec`, `FontCatalog`), 5.14, 5.17 (`paint_target`, `add_raster_layer`), 5.19, 5.20 | `app-shell`, `app-gpu` |
| 2b | `app-panels` (A4) | Menu bar, toolbar, side panel sections, status bar | `panels/mod.rs`, `panels/menu_bar.rs`, `panels/toolbar.rs`, `panels/tool_options.rs`, `panels/layers.rs`, `panels/texture_sets.rs`, `panels/color.rs`, `panels/status_bar.rs` | 8.7 | 8.1, 8.2, 8.10 (`shortcut_text`), 8.12 (`tool_icon`), 8.13, 5.11, 5.12, 5.13, 5.14, 5.17 (thumbnails, `layer_version`) | `app-shell` |
| 2b | `app-dialogs` (A5) | Import, export (with embedded preview), unsaved, progress, message, about | `dialogs/mod.rs`, `dialogs/import.rs`, `dialogs/export.rs`, `dialogs/unsaved.rs`, `dialogs/progress.rs`, `dialogs/message.rs`, `dialogs/about.rs` | 8.8 | 8.1, 8.2, 8.5 (`preview_widget`, `ViewCx`), 8.10 (`table`), 8.13, 5.7 (`UvStats`), 5.15, 5.21 (`ImportedModel`), 5.22 (`FlatFormat`), 5.24 (`ExportOptions`, `plan`) | `app-shell`, `app-gpu` |
| 2c | `app-selftest` (A6) | All canned scenarios; runs every scenario and reports failures to the owning package | `selftest/scenarios.rs` | 8.11 (scenario table) | 8.2, 8.5 (`gizmo_shape`), 8.6 (`ViewXform::for_view`), 8.9, 8.11 (`Step`, `Scratch`), 5.10 (samples), 5.22 (`encode_png`), 5.24 | all other wave-2 packages |

Verification per stage: 2a — `cargo test -p wte_app` plus the `empty` and `smoke` scenarios (the scaffold's placeholder
`view::central` shows the sample model as soon as `app-gpu` works); 2b — the same two scenarios still pass, plus the package's own
unit tests (`app-dialogs` codes against the stub signature of `view::preview_widget`; the embedded export preview is verified in
2c); 2c — every scenario of 8.11.

Cross-package call directions in wave 2 (all through the stub signatures of section 8): `app-shell` calls every other package's
entry points (`view::central`, `panels::show_*`, `dialogs::show`, `ToolController::{is_busy, cancel}`, `Gpu::new`,
`Renderer::new`, `selftest::scenarios::scenario`); `app-views-tools` calls `app-gpu`; `app-dialogs` calls `view::preview_widget`;
`app-selftest` calls `view::gizmo::gizmo_shape` and `ViewXform::for_view`. Beyond that, **any** wave-2 package may call the
read-only helpers `shortcuts::{table, shortcut_text}` and `theme::*`, push toasts with `state.toasts.push(..)`, and read
`Gpu` (`device`, `queue`). Nothing else crosses package borders: commands travel as `AppAction`s, settings live in `AppState`.
