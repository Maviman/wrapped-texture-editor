# Round 3 plan — Wrapped Texture Editor

Audit of the round-2 build, 2026-11-17. Exe audited: `target\release\wrapped_texture_editor.exe`
(SHA-256 `CEF11D8A…F87A`, same as `dist\Wrapped Texture Editor.exe`), run from a copy at `target\round3_audit\wte.exe`.
No source file was changed. All paths below are relative to `target\round3_audit\` unless they start with `crates\`.

## 0. What was run

| Batch | Runs | Result | Table |
|---|---|---|---|
| Built-in scenarios (empty, smoke, default, paint3d, paint2d, stamp, text, clay, preview, export, export_run, multi, roundtrip, undo, import, import_tex, import_nouv, unsaved, layers) | 19 | 19 pass | `a_builtin.csv` |
| `stamp_edge_path` on Avocado.glb | 1 | pass | `a_builtin.csv` |
| `workflow_path` on 22 models (4 GLB samples, 9 topology models, 3 harness shapes, 6 FBX) | 22 | 21 pass, 1 intended error (transparentTest.fbx) | `a_builtin.csv`, `b.csv` |
| `fuzz_path`, 400 steps, new seeds (5, 13, 21, 42, 99) on BrainStem, no_uvs.glb, spider.fbx, uv_tiled_grid, Avocado, multi_uv_sets, crlf_bom.gltf, Fox, huesitos.fbx | 9 | 9 pass (no crash, panic or hang) | `c.csv` |
| Perf, one at a time: `fuzz_path` seed 7, 1000 steps on ngons_tiling; `workflow_path` on ABeautifulGame and tex_8192 | 3 | 3 pass | `e.csv` |
| `open_path` edge cases (missing file, non-model file, two saved no-UV projects) | 4 | 2 pass, 2 intended errors | `d.csv` |
| Small windows: `default` at 900x600, `stamp` at 640x480 | 2 | pass | `runs\size_small`, `runs\size_tiny` |
| Pre-round-2 exe (`target\compat\perf\exe_before`) on no-UV models | 3 | 2 refused (as that build did), 1 pass | `old.csv`, `runs_old\` |
| `cargo test --workspace --offline` | — | 670 passed, 0 failed, 15 ignored | `cargo_test.out` |

Every run's folder is `runs\<scenario>_<folder>_<model>[_s<seed>]\` with `shot.png`, `shot.json`, `shot.log`.
Contact sheets I read: `sheets\wf1.png`, `wf2.png`, `b1.png`, `fz1.png`, `fz2.png`. Helper scripts: `sweep.ps1`
(parallel runner), `slow.py` (slow frame -> preceding fuzz action), `quarters.py` (fps per quarter), `sheet.py`.

## 1. Round-2 claims re-checked

| Claim in ROUND2_REPORT | Verdict | Evidence |
|---|---|---|
| 670 tests pass | Holds | `cargo_test.out`: 201 + 456 + 8 + 5 |
| Stamp depth auto-fit on Avocado | Holds | `runs\stamp_edge_path_glb_samples_Avocado.glb\shot.log`: "edge stamp coverage: fitted 54178 … / 201272 …; old default 46576 / 186928" (same numbers). Screenshot shows the projection box, Depth front 2.2 % / back 15.8 % |
| 3D/2D badges, wrong-view hint | Holds | "3D stamp" / "3D text" in every workflow shot; "2D stamp" badge in `runs\fuzz_path_glb_samples_Fox.glb_s13\shot.png`; `stamp` scenario asserts pass |
| Text tool | Holds | `runs\text_builtin\shot.png` shows "WTE 2026" whole; workflow passes on many_islands and crlf_bom.gltf. Two rough edges: F8, F9 |
| Toast coalescing | Holds, with a layout problem | One "Redo: Edit Stamp (x11)" toast in all workflow shots. But see F6 |
| No-UV models, skinned bind pose | Holds for new imports | BrainStem, OrientationTest, no_uvs.obj/.glb, vertex_colors_only pass; BrainStem T-pose correct. Not for old projects (F1), and reopening nags (F4) |
| FBX import | Holds | 5 of 6 pass. transparentTest.fbx really is damaged: its NUL bytes are 0x20 (`xxd` of the header: `…Binary  \x20\x1a\x20…`), so the error is right |
| CR-only / UTF-16 OBJ | Holds | cr_only.obj, mobius_cr.obj, crlf_bom.obj, utf16le.obj pass |
| ngons_tiling "60/60/54/44 fps, slows late" | Numbers do not repeat; the diagnosis is wrong | Same seed, same exe, machine idle: median 60.5 / 60.0 / 54.2 / 57.8 fps (`quarters.py` on `runs\fuzz_path_shapes_ngons_tiling.obj_s7\shot.log`). It is not a slow-down that builds up. See F3 |
| ABeautifulGame: export 26 s, reopen 12.6 s, save 4.4 s | Export holds; reopen does not | Alone: export 28.1 s, save 3.8 s, reopen 4.7 s, whole workflow 63 s (`runs\workflow_path_glb_samples_ABeautifulGame.glb\shot.log`, "workflow step … done in … ms") |
| "Import warnings are not repeated" | Only after import | Every reopen of a saved no-UV project shows a modal Warnings box (F4) |
| Unresolved 5: "worst frames of 1 to 2 s, not investigated" | Now attributed | Stamp place / drag / tint (F3). The 0.6–1.0 s frames in the `selftest` segment are the scenario's own ray grid, not the app |

## 2. Findings

### F1 — Old no-UV projects open with a different UV layout (painted content lands in the wrong place)
- `crates\wte_core\src\project\load.rs:448-464`: the model is re-imported with `generate_fallback_uvs` from the manifest,
  but `manifest.import.importer_version` is never read. `normalize` (`mesh_build.rs:201-208`) always calls the new atlas
  generator `import\auto_uv.rs`. `IMPORTER_VERSION` is 2 (`import\mod.rs:57`).
- ARCHITECTURE 6.1 item 4 (line 1895): "Before importer version 2 this was one grid cell per triangle pair."
- So a version-1 project with generated UVs gets the version-2 atlas, the hash differs, `LoadWarning::MeshUvChanged`
  fires, and the layer pixels no longer match the triangles. The warning is the symptom; the paint is misplaced.
- Not reproduced end to end: no version-1 no-UV project exists on disk (all 19 projects with
  `generate_fallback_uvs: true` under `target\` are `importer_version: 2`), and the pre-round-2 exe refuses no-UV
  models in `workflow_path` (`old.csv`). The finding rests on the code and the architecture text.

### F2 — Duplicate Layer leaves a stray "Layer Opacity" undo entry; undoing it shows "Unknown layer"
- `runs\fuzz_path_multi_uv_sets_multi_uv_sets.glb_s42\shot.log`, fuzz steps 378–380: step 378 is
  `DuplicateLayer(LayerId(2))`, 379 an orbit drag, 380 two undos. The undos report "Undo: Layer Opacity", then
  "Undo: Duplicate Layer", then the warning toast "Unknown layer" (line 3061). Nothing set an opacity in between.
- Same sequence ("Undo: Layer Opacity" -> "Undo: Duplicate Layer" -> "Unknown layer") at line 2250 of that log, at
  `runs\fuzz_path_glb_samples_Fox.glb_s13\shot.log:3302` and `runs\fuzz_path_fbx_samples_huesitos.fbx_s13\shot.log:3297`.
  4 of 4 "Unknown layer" toasts in the audit follow this pattern. Visible in `sheets\fz2.png` (Fox, huesitos).
- Effects for a user: an extra undo step, the document marked modified, and a warning for a plain Ctrl+Z.

### F3 — Stamp place, drag and tint freeze the UI for 1–2.3 s on tiled-UV and large models
- ngons_tiling, seed 7 (`slow.py … 1000`): 16 frames over 1 s, every one right after an image/text stamp placement in
  3D, a gizmo drag, or `UpdateStampTint(…, Final)` (2011 ms and 2260 ms, segment `dispatch`). Worst frame 2260 ms.
  Per quarter: 5 / 1 / 7 / 8 two-second windows under 30 fps, 90th-percentile UI CPU 24 / 6 / 64 / 83 ms.
- `workflow_path` step times, ngons_tiling against the simple-model baseline (uv_tiled_grid, BrainStem, many_islands):
  "image stamp" 1485 ms against 366 ms; "transform the stamp" 3779 ms against 699 ms.
  spider.fbx "image stamp" 1033 ms; ABeautifulGame 3163 ms.
- This is the real content of the two open items "ngons fps drops late" and "3D stamp drags slow on tiled UVs".
  The median fps moves with where the stamp actions fall in the run, which is why round 2 saw 44 and this run 58.

### F4 — Reopening a project of a no-UV or no-normals model shows a modal "Warnings" box every time
- `runs\open_path_workflow_path_no_uvs_no_uvs.obj_shot.wtep\shot.png`: box "The model has no UV coordinates; a simple
  automatic UV layout was generated." over the reopened project.
- `runs\workflow_path_fbx_samples_embedded_override_uv.fbx\shot.log`, frame 484: "closing the message "Warnings": ["The
  model has no normals; smooth normals were generated.", "The model has no UV coordinates; …"]".
- Cause: `load.rs:465` copies every import warning into the load warnings; `io_flows.rs:155-158` shows any load
  warning as a dialog.

### F5 — Undo back to the saved state still shows the document as modified (known)
- `crates\wte_core\src\document.rs:158-170`: `is_modified` is `revision != saved_revision`, and every undo and redo
  bumps `revision`. `history.rs:1348` tests this as an "accepted simplification".

### F6 — Toasts cover the Layers panel, and a long path makes a toast 900 px wide
- Every workflow shot (`sheets\wf1.png`, `wf2.png`): "Exported 8 files to E:\…\shot_export" runs from x = 360 to 1268
  across the Layers rows. `toasts.rs:150` sets `max_width(380)`, but a path has no break points.
- `sheets\fz1.png`, `fz2.png`: toasts sit on the layer rows and on New / Duplicate / Merge Down / Rasterize / Delete
  for their 4 s (`toasts.rs:147`, anchored to the window's bottom-right corner).

### F7 — Error and warning texts that do not help
- Missing file: "Import failed: I/O error: The system cannot find the file specified. (os error 2)" — no file name
  (`runs\open_path_glb_samples_DoesNotExist.glb\shot.log`).
- "Invalid parameter: the layer is already a paint layer" (`doc_layers.rs:257`, seen in 3 fuzz logs and
  `sheets\fz1.png`, spider): internal wording for Rasterize on a paint layer.
- "Unknown layer" (F2) names nothing.

### F8 — A text layer keeps its first name after the text is changed
- `runs\text_builtin\shot.png`: the model shows "WTE 2026", the layer and "Selected:" still say "WTE".
  `tools\stamp_place.rs:75` (`text_name`) is only used at placement.

### F9 — Layers and side panel rough edges
- With 5 layers at 1280x800 the buttons New / Duplicate / Merge Down / Rasterize / Delete scroll out of sight
  (`runs\open_path_workflow_path_no_uvs_no_uvs.obj_shot.wtep\shot.png`; `panels\mod.rs:264` puts the whole Layers
  section, footer included, in one scroll area).
- The colour picker is cut off with no scroll cue whenever the tool options are long: Transform options on a selected
  stamp leave a 25 px strip of it (`runs\stamp_builtin\shot.png`), Brush options cut the palette row
  (`runs\unsaved_builtin\shot.png`).
- With a text stamp selected the Text options box is empty ("Type the text to place"); the text is behind "Edit"
  (`runs\text_builtin\shot.png`).
- 640x480: status bar items overlap ("Texel 696, 307024 x RGBA…", `runs\size_tiny\shot.png`).
- Export dialog: the folder field shows the start of the path and cuts the end (`runs\export_builtin\shot.png`).

### F10 — The 2D view can lose the texture
- Final Split shots of two fuzz runs have an empty 2D pane: `sheets\fz1.png` BrainStem (zoom 130 %) and Avocado
  (zoom 13 %). Pan is not limited, so the texture can be moved fully out of the view. Not verified beyond the two
  screenshots; Frame probably brings it back.

### F11 — Export of many sets is slow (known)
- ABeautifulGame: "Exporting" job 27.95 s for 85 files, 326 MB, 15 sets. `export\mod.rs:178-275` exports the sets one
  after the other; only the layers of one set run in parallel (line 238).

### F12 — Smaller known items, state unchanged
- FBX texture UV transforms are not read (`import\fbx.rs` has no `UVTranslation` / `UVScaling` / texture `Translation`
  handling). None of the 22 samples has a non-identity transform together with UVs, so a fixture is needed.
- `KHR_mesh_quantization` in `extensionsRequired` is refused (`import\gltf.rs:1307-1310`). No sample on disk.
- nonfinite.obj: one bad `v` line rejects the file.
- tex_8192: island fill 2.1 s, undo-all/redo-all 2.5 s, 3D strokes 3.2 s (`runs\workflow_path_tex_8192_tex_8192.obj\shot.log`).
  Usable; lowest priority.
- PSD / ORA: Pillow 12 opens both. PSD: 4 layers with the right names and bounding boxes. ORA: `mimetype` first and
  stored, valid `stack.xml`, zip test clean (`runs\export_run_builtin\shot_export\`). Still not opened in GIMP, Krita
  or Photoshop; that needs the user.
- slivers.obj/.glb: scenario limit, not an app bug (unchanged).

## 3. Round-3 items, by priority

### P1 — correctness

**R3-1 Old no-UV projects keep their layout (F1)** — REDUCED by user decision 1 in section 6; the text below is
kept for background only.
- Hypothesis: load ignores `importer_version`; the version-1 generator no longer exists.
- Fix direction: keep a `legacy` generator ("one grid cell per triangle pair", ARCHITECTURE 6.1 item 4 as it was before
  round 2) beside `auto_uv::generate`; pass the manifest's `importer_version` through `ImportOptions` /
  `NormalizeOptions`; a loaded project keeps the version it was saved with when it is saved again.
- Files: `wte_core\src\import\auto_uv.rs`, `import\mod.rs`, `mesh_build.rs`, `project\load.rs`, `project\save.rs`,
  `project\manifest.rs`.
- Risk: there is no version-1 no-UV project to check against. Ask the user for one. Without it, the legacy layout
  must be rebuilt from the architecture text and cannot be proven equal.
- Acceptance:
  - unit test: `normalize` with version 1 and version 2 gives different, each stable, `uv_hash` for `no_uv_tetra`;
  - `project_roundtrip`: a manifest with `importer_version: 1` + `generate_fallback_uvs: true` loads with the legacy
    layout and saves with version 1 again;
  - if the user supplies an old project: it opens without `MeshUvChanged`, and the screenshot shows the paint where
    it was;
  - if the hash still differs, the warning text says what happened and that nothing was overwritten.

**R3-2 No stray "Layer Opacity" entry after Duplicate Layer (F2)**
- Hypothesis: the Layers opacity slider (`panels\layers.rs:109-111`) and `continuous_edit` / `owed_final`
  (`panels\mod.rs:294-322`, `:125-133`) emit a `SetLayerOpacity(.., Final)` when the active layer changes under the
  slider (the slider's egui id is the same for every layer), and again for the removed copy after the undo.
- Files: `wte_app\src\panels\mod.rs`, `panels\layers.rs`, possibly `dispatch.rs`.
- Repro: `fuzz_path`, `WTE_FUZZ_SEED=42`, 400 steps, multi_uv_sets.glb (steps 378–380); seed 13 on Fox.glb.
- Acceptance:
  - new assert in the `layers` scenario: Duplicate adds exactly one undo entry; undoing it gives no warning toast;
  - the two fuzz repros log no "Unknown layer";
  - `owed_final` is dropped when its target layer or stamp no longer exists (unit test in `panels\mod.rs`).

**R3-3 Reopening a project does not nag (F4)**
- Hypothesis: `load.rs:465` forwards informational import warnings.
- Fix direction: on load, drop `NormalsGenerated` and `FallbackUvsGenerated` (they were shown at import); keep real
  ones (missing blob, font fallback, `MeshUvChanged`).
- Files: `wte_core\src\project\load.rs`.
- Acceptance: `open_path` on a saved no_uvs.obj project ends with `Dialog::None`; unit test on the warning list.

### P2 — responsiveness

**R3-4 Stamp place / drag / tint under 100 ms per step on ngons_tiling (F3)**
- Hypotheses, in order:
  1. `stamp_bake.rs:582-585`: a clip interval that wraps "forces the whole axis", so each tiled triangle is
     rasterized over the full texture and the scratch buffer is the full texture;
  2. every drag point runs `fit_projector_depth` (a 17x17 ray grid, up to 4 hits per ray, `stamp_bake.rs:391`; `projector_depth_is_auto` runs it a second time) plus a `Preview` bake on
     the UI thread (`doc_stamps.rs:594`, `:628`);
  3. a tint change re-bakes at `Full` quality with occlusion rays instead of re-tinting the cached coverage
     (`doc_stamps.rs:549`).
- Fix direction: fold tiled footprints per texel-map block (the `texel_map` block folding the perf package named),
  cache untinted coverage per stamp so tint is a multiply, throttle preview bakes to one per frame and skip the
  depth fit while the pointer is down.
- Files: `wte_core\src\stamp_bake.rs`, `texel_map.rs`, `doc_stamps.rs`; `wte_app\src\tools\transform.rs`,
  `tools\stamp_place.rs`.
- Acceptance (release exe, machine idle):
  - `workflow_path` on ngons_tiling: "image stamp" <= 500 ms, "transform the stamp" <= 1000 ms;
  - `fuzz_path` seed 7, 1000 steps, ngons_tiling: no frame over 500 ms in the `central` or `dispatch` segment, at
    most 2 two-second windows under 30 fps per quarter;
  - ABeautifulGame "image stamp" <= 1500 ms;
  - bake results bit-identical to round 2 for the existing `stamp_bake` tests, plus a new tiled-UV test.

**R3-5 Export of many sets in parallel (F11)**
- Hypothesis: serial loop over sets; PNG `Balanced` level and PSD writing dominate.
- Files: `wte_core\src\export\mod.rs`, `export\psd.rs`, `export\ora.rs`.
- Acceptance: ABeautifulGame "export every format" <= 12 s; exported files byte-identical to the round-2 exe's for
  the built-in `export_run` (or pixel-identical if the PNG level changes); memory peak stated in the report.

### P3 — usability

**R3-6 Undo to the saved state clears the modified mark (F5)**
- Fix direction: give each history state an id (the id of the entry on top of the undo stack); `mark_saved` stores
  it; `is_modified` compares ids. An evicted saved state stays "modified".
- Files: `wte_core\src\document.rs`, `history.rs`; `wte_app\src\io_flows.rs` (`has_unsaved_changes`).
- Acceptance: unit test replacing `history.rs:1348`; `unsaved` scenario variant: save, stroke, undo, then
  `OpenSample` opens without the Unsaved dialog; redo makes it modified again.

**R3-7 Toasts stay off the Layers panel (F6)**
- Fix direction: anchor the toast area to the bottom-right of the central view rect; show only the folder name
  ("Exported 8 files to shot_export"), full path on hover; wrap anywhere as a fallback.
- Files: `wte_app\src\toasts.rs`, `io_flows.rs:202`.
- Acceptance: in the `workflow_path` screenshot no toast intersects `state.panel_rects` (new assert); toast width
  <= 380 px.

**R3-8 Messages (F7)**
- Missing or unreadable file: "Cannot open <name>: file not found". Rasterize on a paint layer: do nothing (the
  button is already disabled). Stale layer: no toast.
- Files: `wte_app\src\io_flows.rs`, `dispatch.rs`. Do not reword `error.rs` for this.
- Acceptance: `open_path` on a missing .glb logs the file name; fuzz logs contain no "Invalid parameter".

**R3-9 Text layer follows its text (F8)**
- Rename the stamp inside `update_stamp_source` when the old name still equals the automatic one (one undo entry).
- Files: `wte_core\src\doc_stamps.rs`.
- Acceptance: `text` scenario asserts the layer is named "WTE 2026" after the change; a renamed layer keeps its name.

**R3-10 Side panel (F9)**
- Layers footer buttons outside the scroll area; Text options show the selected text ready to edit; status bar
  drops or elides items when narrow; export folder field shows the end of the path.
- Files: `wte_app\src\panels\mod.rs`, `layers.rs`, `tool_options.rs`, `status_bar.rs`, `dialogs\export.rs`.
- Acceptance: `layers` scenario with 6 layers at 1280x800: the Delete button rect is inside the window and not
  inside the scroll content; 640x480 `stamp` screenshot without overlapping status text.
- The colour picker being cut off is a layout decision for the user: see open questions.

**R3-11 2D view keeps the texture reachable (F10)**
- Clamp pan so at least 32 px of the texture stay in the view.
- Files: `wte_app\src\view\view2d.rs`, `view\camera.rs`.
- Acceptance: unit test on the clamp; a pan drag of 10 view widths leaves the texture rect intersecting the view.

### P4 — import breadth (known items)

**R3-12 FBX texture UV transforms** — `import\fbx.rs`: read `Translation` / `Rotation` / `Scaling` (and
`UVTranslation` / `UVScaling`) of the Texture node, apply like `KHR_texture_transform` (`gltf.rs:194`).
Acceptance: generated ASCII FBX fixture with scale 2, offset 0.25 gives the same `uv_hash` as the equivalent glTF;
all 21 FBX samples keep their round-2 `uv_hash`.

**R3-13 KHR_mesh_quantization** — `import\gltf.rs`: accept the extension, read normalized and non-normalized integer
POSITION / NORMAL / TEXCOORD accessors. Acceptance: generated quantized cube imports with the same mesh (within
1e-4) as its float twin; meshopt and draco are still refused by name.

**R3-14 Tolerant OBJ numbers** — `import\obj.rs`: skip a malformed or non-finite `v` line with a warning.
Acceptance: nonfinite.obj opens and passes `workflow_path`, with one warning naming the line.

Not planned for round 3: 8192 speed (usable), slivers scenario, PSD/ORA check in a real editor (user task; add the
step to `dist\HOW_TO_TEST.txt`).

## 4. Parallel packages (disjoint file ownership)

| Package | Items | Owns |
|---|---|---|
| A — stamps and performance | R3-4, R3-5, R3-9 | `wte_core\src\stamp_bake.rs`, `texel_map.rs`, `doc_stamps.rs`, `stamp.rs`, `export\*`; `wte_app\src\tools\transform.rs`, `tools\stamp_place.rs` |
| B — import and project | R3-1, R3-3, R3-12, R3-13, R3-14 | `wte_core\src\import\*`, `mesh_build.rs`, `mesh.rs`, `project\*`, `error.rs`; `wte_core\tests\project_roundtrip.rs` |
| C — UI and history | R3-2, R3-6, R3-7, R3-8, R3-10, R3-11 | `wte_app\src\panels\*`, `toasts.rs`, `io_flows.rs`, `dispatch.rs`, `dialogs\*`, `view\*`, `control_sweep\*`; `wte_core\src\document.rs`, `history.rs`, `doc_layers.rs` |
| D — integration gate (after A, B, C) | new scenario asserts, sweep, report | `wte_app\src\selftest\*`, `ARCHITECTURE.md`, `dist\*`, `target\round3\*`, the release build |

Rules:
- A, B and C add unit tests only in their own files. Scenario changes they need are written as a note for D, as in
  round 2.
- Shared types: if B needs a new field in `ImportOptions` and A or C would have to compile against it, B lands first
  or adds it with a default. C must not edit `error.rs`; it rewords at the app layer.
- `wte_core\src\lib.rs` re-exports: B owns it; A and C ask D for anything else.
- Each package builds into its own target dir (`--target-dir target\r3_<pkg>`) so three builds do not fight over
  one lock, all on E:.
- D runs: the full test suite, clippy, the 24 built-in scenarios, the 157-model sweep, the repros named in the
  acceptance lines above (F2 seeds, ngons seed 7, ABeautifulGame alone), then copies the exe to `dist\`.

## 5. Open questions for the user (answered, see section 6)

1. Do you have a project saved before round 2 from a model without UVs? R3-1 cannot be proven without one.
2. Colour picker cut off by long tool options: keep the single scroll area, or make Tool options / Color collapsible
   so the picker stays visible?
3. PSD / ORA: can you open `runs\export_run_builtin\shot_export\sphere.psd` and `sphere.ora` in your editor once?

## 6. User decisions, 2026-11-17 — these override the text above

**1. No project from before round 2 with generated UVs exists** (the only saved projects are test output). So R3-1
shrinks. Do not write a legacy UV generator. R3-1 is now:
- `project\load.rs` reads `manifest.import.importer_version`. When it is lower than `IMPORTER_VERSION`, the project
  uses generated UVs and the UV hash differs, the load warning says in plain words that the project was saved with an
  older automatic texture layout, that painted content may sit in the wrong place, and that nothing was overwritten.
  A hash difference for any other reason keeps the existing `MeshUvChanged` text.
- A project keeps loading and saving exactly as today when the versions match.
- ARCHITECTURE.md (5.21 / 6.1 / 7.3) gets the rule: a change to the generated layout or to mesh normalisation that
  changes `uv_hash` must bump `IMPORTER_VERSION` and keep the previous path selectable by version from then on.
- Acceptance: a unit test with a manifest edited to `importer_version: 1` and a wrong hash gets the new warning text;
  the same manifest with the right hash loads without a warning; a `uv_hash` stability test for `no_uv_tetra` pins
  the version-2 layout (a frozen hash constant), so a later change cannot slip through unnoticed.

**2. The colour picker stays visible.** R3-10 adds: at a window of 1280x800 or larger the whole Color section (RGB
row, colour bar, saturation/value square, hue bar, hex field, both swatch rows) is on screen without scrolling,
whatever tool is active and whatever is selected — including Transform options of a selected stamp and Text options.
Tool options and Texture sets scroll in their own area (with a visible scrollbar when they overflow); Color is pinned
below them; Layers keeps its own resizable area with the footer buttons outside its scroll area. In a smaller window
the picker may shrink, then scroll. Acceptance: `stamp` and `text` scenario screenshots at 1280x800 show the full
picker and the Layers footer buttons; a new assert checks that the Color section rect lies inside the side panel
rect in those two scenarios. (A separate Brush tab is planned for round 4; keep the layout code easy to extend, but
do not build tabs now.)

**3. No image editor is available for PSD / ORA.** The gate checks every exported PSD and ORA of the built-in
`export_run` and of one multi-set model with Pillow (`python -I`): the file opens, layer count, names, order and
bounding boxes match the document, each layer's pixels equal the matching per-layer PNG, and the flattened image
equals the flat PNG. The script is kept as `target\round3\check_exports.py`.
