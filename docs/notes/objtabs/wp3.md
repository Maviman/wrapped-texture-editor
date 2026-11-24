# objects-tabs WP3a / WP3b — builder notes

Branch `objects-tabs`, worktree `G:\software_projects\texture_wrapper_ap_ui`. Plan sections 4 (WP3a) and 5 (WP3b).

## WP3a — app state, actions, object_ops, menu, shortcuts, persistence

What is in place:

- `Session::{selected_objects, object_anchor, object_selection_rev, object_reveal}` (state.rs); `Session::validate` drops
  selected objects beyond `object_count` (bumping the revision) and a stale anchor / reveal.
- `AppAction::{SelectObject, ToggleObjectSelected, SelectObjectRange, SelectAllObjects, DeselectObjects, SetObjectVisible,
  HideSelectedObjects, IsolateSelectedObjects, ShowAllObjects, FrameSelection}`.
- New `crates/wte_app/src/object_ops.rs` with the plan's functions plus `selection_bounds` and `restored_hidden_text`.
  `set_hidden` logs `objects: N of M hidden (mask in X ms)` on a change.
- dispatch: the object actions; `FrameModel` and `ResetCamera` frame `doc.visible_bounds()`.
- Shortcuts (table 36 -> 40): Shift+F "Frame Selection" directly before F; Shift+H "Isolate Selected", Alt+H "Show All",
  H "Hide Selected" (a new "objects" block after "Frame Model"). All `with_doc`, `text_ok` false.
- Menu "Object" between Layer and Help (Hide Selected, Isolate Selected, Show All, Frame Selection, Select All, Deselect
  All); File > Open Sample > "Three Objects". The "Objects Panel" entry is WP4's.
- `UiState::hidden_objects: Vec<String>` (io_flows.rs): captured at save, restored in `install_document` with one Info
  toast.

Decisions where the plan was open or in tension (each chosen for the plan's stated intent):

1. **The visibility actions finish a held interaction instead of cancelling it.** Plan 4.3 says "a held stroke or gizmo
   drag is ended the usual way first", but its test (4.7) wants "HideSelectedObjects during a held stroke ends the stroke
   first and leaves one undo entry (the stroke)". The usual way for a guarded action is `cancel`, which discards a held
   stroke (`stroke_cancel`, no undo entry). So `SetObjectVisible`, `HideSelectedObjects`, `IsolateSelectedObjects` and
   `ShowAllObjects` are in `finishes_interaction` (like Save): hiding is view state and should never throw paint away.
2. **`UiState::hidden_objects` is not written when nothing is hidden** (`skip_serializing_if = "Vec::is_empty"`), so a
   project without hidden objects has the same `ui_state` as before. It is read leniently (`lenient_names`): any value
   that is not an array counts as nothing hidden *and the rest of the view state still loads* (the plan's "non-array
   value -> ignored with the warning path": the warning is logged, but I did not let one bad key drop the camera,
   colours and view mode); non-string entries are skipped.
3. **The Info toast on load** names the hidden objects when there are at most three ("1 object is hidden: Cube (Object >
   Show All)", "2 objects are hidden: Floor, Ball (Object > Show All)"), else "N objects are hidden (Object > Show All)".
   (Plan 4.3 gives the N form; scenario 8.1 step 11 wants the toast to name the Cube.)
4. `SelectObjectRange` replaces the selection with the range (like the Layers list's Shift-click), the anchor stays.
   `SelectAllObjects` keeps the anchor; `DeselectObjects` clears it.
5. The dispatch test with a held stroke needs `ToolController` and `dispatch`, i.e. a `WteApp`, which needs a GPU: it is
   `control_sweep::gpu_tests::hiding_during_a_held_stroke_commits_the_stroke_first` (ignored, run with `--ignored`; passes
   on this machine).
6. `SelectObjectRange` and `SetObjectVisible` carry `#[allow(dead_code)]` until the Objects tab (WP5) queues them.

Tests after WP3a (`cargo-gnu.cmd test --workspace --no-fail-fast`): app 391 / 0 / 20, core lib 821 / 1 (WA20, the known
red) / 24, brush_tips 15, end_to_end 9, objects 8, project_roundtrip 24 -> **1,268 passed, 1 failed (WA20), 44 ignored**.
Clippy `--workspace --all-targets`: 0 warnings. GPU: `hiding_during_a_held_stroke_commits_the_stroke_first` ok.

## WP3b — rendering, highlight, picking

What is in place:

- `gpu/scene.rs`: the pure helpers `visible_draws`, `selected_indices`, `object_runs`, `visible_runs` and `WireList`
  (instances + object runs). `wire_instances_3d` / `wire_instances_uv_all` stable-sort each group (3D: interior, then
  seams + boundaries; UV per set: interior, then seams) by object. `GpuScene::sync_objects` (called by
  `view::sync_scene` every frame after `sync`) rebuilds the visible index buffer + draws and the visible wire ranges when
  `visibility_generation` changes, and the highlight index buffer when that or `object_selection_rev` changes.
  `mesh_draw(all_objects)`, `highlight()`, `wire_all/wire_seams/wire_uv(.., all_objects)` return what to draw.
  While nothing is hidden the draw calls are exactly the old ones (full index buffer, `build_draws`, one `0..n` wire
  range).
- `gpu/shaders/tint.wgsl` + `Renderer::tint_pipeline` (view_only layout, mesh vertex layout, Overlay depth, LINE_BLEND,
  DEPTH_PULL 2e-4 like the wires, colour (0.25, 0.61, 1.0, 0.30) in the wires' sRGB-byte convention).
- `View3dParams::{all_objects, highlight}`: Main3D `false` / `view3d::highlight_shown` (selection non-empty, no open
  stroke, tool Navigate — the "Objects tab shown" half comes with WP4); Preview mode `false, false`; the export dialog's
  preview `true, false` (`preview_widget` decides by `ViewId::ExportPreview`).
- `render_uv` draws only the visible objects' UV wires.
- Navigate click-select (`tools/mod.rs`): `NavClick`, `NAV_CLICK_TRAVEL` = 4 pt, `pick_object`. Only a logical primary
  press with the Navigate tool in Main3D on a model with >= 2 objects; `go_idle` forgets it. The Navigate tool options
  gain the line "Click an object to select it; Ctrl- or Shift-click adds or removes it." on such models (this is where
  the Navigate hint lives: the status bar has no Navigate hint).
- `io_flows::install_document` logs `objects: N (first 24 names ...)`.

### The three "builder: check" items

1. **Tool hover caches** (`tools/stamp_place.rs`, `tools/brush.rs`): nothing to key. `StampPlaceTool::size_cache`,
   `hover_pic`, `size_failed`, `picture_failed` are keyed by the source settings (`SourceKey`: image blob or text
   settings) and hold the picture's size / raster, which does not depend on the surface; the placement outline
   (`preview`) is recomputed from the hit on every hover. `BrushTool::ring` / `cursor_uv` are set from the current hover
   and cleared in `begin_frame`; no brush cache is keyed by position or revision. Also looked at: `transform.rs` (its
   `wrap_failed` mark is keyed by the visibility generation since check WP0-2; `cached_text_frame` is the text's frame,
   independent of visibility) and `text_path.rs` ("the flat point ... is cached" in a doc comment means "falls back to",
   nothing is stored).
2. **`view::uv_outline_in_3d` / `locate_px`**: it *could* place outline points on hidden triangles (`TexelMap::locate`
   knows nothing of visibility). Fixed in `locate_px` itself: a texel whose owning triangle belongs to a hidden object
   gives `None`. That covers the UV-stamp outline in 3D, the brush ring of a 2D cursor drawn in 3D and the 2D hover's
   world position.
3. **`collect_grown` Jump branch**: confirmed by the WP2 builder and checker (see wp2.md / check_wp012.md).

### Measured cost of a visibility change (plan 5.1, risk 4; budget 250 ms)

`control_sweep::gpu_tests::perf_visibility_toggle_on_a_heavy_model` with `WTE_HEAVY_MODEL` = ABeautifulGame.glb
(1,498,080 triangles, 49 objects), release test build, RTX 5070 Vulkan (`target\objtabs\perf_toggle_release.log`):

| Change | core mask (`set_hidden_objects`) | `sync_objects` (compaction + upload + highlight) | total |
|---|---|---|---|
| isolate 2 objects | 2.9 ms | 33.1 ms | 36.0 ms |
| hide 1 object | 1.5 ms | 19.1 ms | 20.6 ms |
| show all | 0.0 ms | 1.9 ms | 1.9 ms |

Well inside the budget; no worker thread needed.

### Pixel / export identity against the baseline (plan 5 check)

Both exes were run on the whole baseline job list (`target\objtabs\jobs_baseline.txt`, 1280x800) **into the same folder
one after the other** (`target\objtabs\cmp\run_both.sh`: baseline exe `target\objtabs\baseline\wte_base.exe`, then the
WP3b exe `target\objtabs\wp3\wte_wp3.exe`, SHA-256 `AA4DF3D1…`, results moved to `cmp\base` and `cmp\wp3`), so the
output paths printed in dialogs are the same. Compared with `target\objtabs\cmp\compare.py` (Pillow / numpy), result in
`target\objtabs\cmp\compare_wp3.txt`:

- Summary: every job `"ok": true` with both exes except `import_save` on BoxTextured.glb (fails the same way with both:
  no sidecar files; known since WP0).
- **Exports**: every PNG / PSD / ORA / ZIP the scenarios write is **byte-identical** (147 files byte-identical in all, every export among them).
  The 12 `.wtep` files differ only by their save time (as in WP2).
- **Screenshots**: 72 differ **only inside the menu bar** — the same 383 pixels in the box x 153..229, y 7..19: the new
  "Object" menu title between Layer and Help (WP3a, plan 4.5) and Help moved right. The full-frame identity the plan
  asks for at this point cannot hold with the planned Object menu; with the menu bar row excluded they are
  pixel-identical. 2 more differ in the menu bar and in one toast each (`brush_shape\shot_dialog.png`: the "Brush shape
  loaded" toast, `final_bake_async\shot.png`: the "Exported 4 files" toast; max channel delta 5 and 21 — the toast's
  fade-in timing, as seen in WP2). Nothing else differs anywhere.
- Risk 5 (wire order on multi-object models) is not exercised by the existing scenarios (none turns the wireframe on
  for a multi-object model); ABeautifulGame (`open_path_abg`) is identical apart from the menu bar.

Headless checks: `open_path` on ABeautifulGame.glb starts, imports and logs
`objects: 49 (King_B, King_W, Queen_B, Queen_W, Chessboard, Pawn_Body_W1, ... and 25 more)`; on BoxTextured.glb (one
object) `objects: 1 (Mesh)`; both `"ok": true`.

### Tests after WP3b

`cargo-gnu.cmd test --workspace --no-fail-fast`: app 396 / 0 / 22, core lib 821 / 1 (WA20, known red) / 24,
brush_tips 15, end_to_end 9, objects 8, project_roundtrip 24 -> **1,273 passed, 1 failed (WA20), 46 ignored**.
`cargo test -p wte_app -- --ignored --skip perf_`: **20 passed** (includes the two new GPU tests). Clippy
`--workspace --all-targets`: 0 warnings.

Deviation notes:

- `GpuScene::mesh_buffers()` / `draws()` were replaced by `mesh_draw(all_objects)`; `highlight()` returns the vertex
  buffer too.
- The plan's GPU test asks for "the pixel at the cube's centre equals the floor render": compared against a render with
  Cube and Ball hidden (floor only), and the wire check compares only pixels where the Ball does not change the picture.
- The ignored perf test is not a `perf_` test of the gate list's `wte_core`, but `cargo test -p wte_app perf_ --
  --ignored` runs it; without `WTE_HEAVY_MODEL` it only prints that nothing was measured.
