# objects-tabs — check of WP3a / WP3b (app state, actions, menu, shortcuts, persistence; rendering, highlight, picking)

Independent check of the builder's commits 8a126d4 (WP3a) and 7013e79 (WP3b) on branch `objects-tabs`, against plan
sections 4 and 5 and the rules of 3.4.

## What was verified

- **Diff read item by item** against plan 4.1-4.7 and 5.1-5.6: Session fields and `validate`; the ten actions and their
  dispatch; `object_ops` rules (anchor, no-ops without a session / with an empty selection, hide keeps the selection,
  isolate, show all, frame selection over visible selected objects, 2D frames nothing); guard + finish for the four
  visibility actions; `FrameModel` / `ResetCamera` on `visible_bounds()`; the four shortcuts in "most specific first"
  order (table 40); the Object menu and its enable rules; File > Open Sample > Three Objects; `UiState::hidden_objects`
  (skip when empty, lenient read); restore in `install_document` with the Info toast; `GpuScene` helpers,
  `sync_objects` keyed by (visibility generation, selection revision) and reset with every new scene (new document);
  `mesh_draw` / wire ranges (one `0..n` range and the full index buffer while nothing is hidden); the tint pipeline;
  `View3dParams::{all_objects, highlight}` per view (Main3D / Preview / export preview); UV wires of hidden objects left
  out; `locate_px` skips hidden owners; Navigate click-select (logical primary button, Main3D, >= 2 objects, 4 pt,
  camera restored, Ctrl/Shift toggle, miss deselects, `go_idle` forgets it). The press/release routing
  (`view::route_pointer`) works on raw button events, so a click without motion does reach `press` / `release`.
- **Races**: a visibility change is refused by the core during an open stroke and the dispatch guard finishes the
  interaction first; wrap param jobs in flight across a change are not cached (WP0-2 check, `WrapFlight::visibility`);
  bake jobs carry their `Arc<Bvh>`; no wrap or picking cache in the app is keyed without the visibility generation.
- **Edge cases**: unknown / repeated stored names (ignored), a stored list that names nothing of the model (nothing
  hidden, no toast), everything hidden (no mesh draw, tools hit nothing, `visible_bounds` = model bounds), one-object
  models (no pick; menu entries still act), `hidden_objects()` always has `object_count` entries (one constructor,
  `set_hidden_objects` pads / cuts), indices beyond the mesh ignored everywhere.
- **Tests**: `cargo-gnu.cmd test --workspace --no-fail-fast` at 7013e79: 1,273 passed / 1 failed (WA20, known red) /
  46 ignored (app 396, core lib 821 + 1 failed, brush_tips 15, end_to_end 9, objects 8, project_roundtrip 24).
  `cargo test -p wte_app -- --ignored --skip perf_`: 20 passed. Clippy 0 warnings.
- **Pixel identity** (my own run, release exe of 7013e79 = SHA-256 AA4DF3D1…, identical to the builder's `wte_wp3.exe`):
  `default`, `paint3d`, `stamp`, `text`, `clay`, `preview`, `wrap_surface` (6 shots), `layers` and `open_path` on
  ABeautifulGame against `target\objtabs\baseline\`: every screenshot differs only in the same 383 pixels in the box
  x 153..229, y 7..19 (the new "Object" menu title, Help moved right); 0 differing pixels below the menu bar. All 9 jobs
  `"ok": true`. Outputs `target\objtabs\check3\run\`, script `target\objtabs\check3\pixcmp.py`.
- **Visibility toggle cost** re-measured (release test build, ABeautifulGame, 1,498,080 triangles / 49 objects):
  isolate 2 = 2.4 + 34.1 ms, hide 1 = 1.5 + 18.6 ms, show all = 1.8 ms (core mask + `sync_objects`); budget 250 ms.
  Log `target\objtabs\check3\perf_toggle_check.log`.

## Findings

1. **Missing test (minor, fixed): the five tool rays through a hidden object at the app level.** The core tests
   (`tests/objects.rs`) cover brush, projected stamp and wrap on the document API, and the Navigate test covers the pick,
   but nothing drove the eyedropper, the fill seed or a stamp placement through `ToolController` with a hidden object.
   Added `tools::tests::every_tool_ray_passes_through_a_hidden_object`: eyedropper (the cube's top painted red: shown
   picks red, hidden picks the floor's colour), Island fill, brush stroke, projected and wrapped image stamp, each with
   the cube shown (acts on the cube) and hidden (acts on the floor under it, zero cube texels changed).
2. **Missing test (minor, fixed): restoring stored hidden objects on open.** `install_document`'s restore and toast had
   no test (the serde part had). Added GPU test `control_sweep::gpu_tests::a_document_opens_with_its_stored_hidden_objects`
   (known + unknown name, one toast, not modified, no history, empty selection, first frames draw without the cube; a
   list naming nothing hides nothing and shows no toast).
3. **Plan deviation accepted (no change): full-frame pixel identity** cannot hold because of the planned Object menu
   title; confirmed it is the only difference (finding of the builder, verified above).
4. **Plan deviation accepted (no change): visibility actions finish a held stroke** instead of cancelling it — required
   by the plan's own test (one undo entry for the stroke); verified by the GPU test
   `hiding_during_a_held_stroke_commits_the_stroke_first`.
5. **Open (not code, unchanged): Alt+H on Windows** cannot be checked headless (system-menu beep); manual check by the
   user, fallback Ctrl+Shift+H. **Risk 5** (wire draw order on multi-object models with Wireframe on) is not exercised
   by any existing scenario; WP6's `objects` scenario turns Wireframe on for the "hidden" shot.

No blocker or major defect found in WP3a / WP3b.

## After the check

`cargo-gnu.cmd test --workspace --no-fail-fast`: **1,274 passed, 1 failed (WA20, known red), 47 ignored** (app 397 / 23
ignored). `cargo test -p wte_app a_document_opens_with_its_stored_hidden_objects -- --ignored`: ok. Clippy
`--workspace --all-targets`: 0 warnings. ARCHITECTURE 9.2 lists the two new tests.
