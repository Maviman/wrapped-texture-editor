# check_exp_1 — independent check of branch `export-select` (pass 1)

Checker: Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_exp`, branch `export-select`
(HEAD `e4e45a2`, base round9-wip `a8beea6`). Evidence under `target\stable\check_exp_1\` (git-ignored).
Goal: try to refute "import a model -> layer designs -> export one layer, a selection, or everything in one .zip".

## 0. State of the branch (read first)

- The builder returned nothing (`{"e1":null}`). Only the **core** part is committed (`e4e45a2`: `ExportOptions.layers`,
  `ExportPackage::Zip`, `plan_contents`, core tests). **Every app-side change is uncommitted** in the working tree
  (15 files, +3,296 / -1,515: `action.rs`, `dispatch.rs`, `state.rs`, `panels/layers.rs`, `menu_bar.rs`,
  `dialogs/export.rs`, `io_flows.rs`, `selftest/*`, `control_sweep/*`, `ARCHITECTURE.md`). I checked that working tree
  as found; its diff is saved in `worktree_uncommitted.diff` (SHA-256 `5badc2c8…`, unchanged from start to the release
  build). `HANDOFF.md` and `dist/HOW_TO_TEST.txt` were not updated (plan section 8 asked for both).
- No builder process was running in this worktree (the cargo/rustc seen at start belonged to `texture_wrapper_ap_r9`).

## 1. Build and tests (this worktree, this machine)

| Check | Result | Evidence |
|---|---|---|
| `cargo test --workspace --no-fail-fast` | **1,190 passed, 5 failed, 36 ignored** (wte_app 370/0/17; wte_core lib 773/5/19; brush_tips 15; end_to_end 9; project_roundtrip 23) | `test_full.log` |
| the 5 failures | `wa17_jump_small_gaps_climbs_a_cube_standing_on_the_floor` (ink 0.915 < 0.95), `wa20_a_bumpy_ball…` (ink 1.189), `wa24_jump_small_gaps_is_bounded_by_the_footprint` (ink 0.9146), `wa35_exact_parts_stay_exact` (92.37 % within 2/255, limit 99.9 %; max 51 > 8), `wa4_r8_bumpy_plates…` (60 deg: ink 1.1177 > 1.10, cells 0.769..1.432 > 1.30) — the same five, same numbers, as the round-9 gate (1,170 / 5 / 35). Not caused by this branch (no wrap code touched). | `test_full.log` |
| `test -p wte_app -- --ignored control_sweep` (GPU) | **12 passed, 1 failed**: `export_dialog_controls_and_export` panics at `gpu_tests.rs:326` (`NotFound`) | `test_gpu_sweep.log` |
| `clippy --workspace --all-targets` | **5 warnings (4 distinct)**, rule is 0: `export/mod.rs:1542`, `:1543` (unnecessary clone to a slice), `selftest/scenarios.rs:1233` (`contains()` instead of `iter().any()`), `control_sweep/gpu_tests.rs:360` (doc list indentation) | `clippy.log` |
| `build --release -p wte_app` | OK; exe copied to `bin\wte_exp.exe` (SHA-256 `8d895e8e…`) | `build.log` |

The GPU failure is a test bug the builder added: the test unticks "Per-layer PNGs" (line ~279) and later reads every
per-layer PNG from the export folder (new lines 321-328). The export itself is right (it wrote exactly the planned
files; the PSD assert before it passed).

## 2. Scenarios (release exe, 1280x800)

All `"ok": true` unless stated (`runs\summary.txt`, one folder per run under `runs\`).

| Run | Model | Result |
|---|---|---|
| `export_selected` | built-in sphere | ok (real Ctrl / Shift / plain / right clicks; hidden text layer selected and exported) |
| `export_zip` | built-in | ok; zip1 == zip2 byte for byte |
| `export_flow` (no PATH) | generated cube as OBJ+MTL+PNG, glTF, GLB, ASCII FBX | ok, all four |
| `export_flow` | Lantern.glb, ngons_tiling.obj (4 materials, a PNG name with a space, a missing texture, no map), BoxTextured.glb, Avocado.glb, gltf_node_transforms.gltf, **ABeautifulGame.glb** (15 sets, 84 files, 227 MB zip, 23 s) | ok, all |
| `export_flow` | r9-saved `.wtep` (Lantern run's `shot_wrap.wtep`, `ident/roundtrip/shot.wtep`) | **failed** at "the stroke painted: one history entry" (2 entries: the project's active layer is a stamp layer, so the brush creates a paint layer first — toast "Created paint layer"). Harness assumption, not a product bug (see D6). |
| `export_run` new vs `wte_r9.exe` | built-in | **all 7 files SHA-256 identical** (`runs\run_exp_hash.txt`, `run_r9_hash.txt`) |
| `workflow_path` on the r9 `.wtep`, new vs r9 | old project | **all 11 exported files identical**; the saved `.wtep` differs only in `manifest.json/saved_unix` |
| `layers`, `default`, `stamp`, `roundtrip` new vs r9 | built-in | screenshots byte-identical; `export` differs (Layers row replaces "Include hidden layers": intended) |

## 3. Independent checks of the files (Python, standard library only)

numpy / Pillow are **not installed** for any Python on this machine (3.9-3.13; CLAUDE.md says they are), so
`verify_exports.py` has its own PNG decoder, ORA / PSD parsers and a src-over recompositor. Outputs:
`runs\zip\verify.json`, `runs\flow_*\shot_*_all.verify.json`, `*.xcheck.json`.

- **Zip vs loose** (`export_zip`): 7 entries, all 7 byte-equal to `shot_export_ref`, none extra or missing, order =
  plan order, `testzip` clean, PNG / ORA stored, PSD deflated, every time stamp 1980-01-01 00:00:00, no folders, no
  duplicate names. Same structure in all 10 `export_flow` zips (incl. ABeautifulGame: 84 entries, 15 sets, PSD / ORA /
  per-layer counts agree per set).
- **Each per-layer PNG holds exactly its layer**: every per-layer PNG == the ORA layer at the same stack position
  (decoded, exact) in all zips; the flat PNG == ORA `mergedimage.png` (exact); full recomposite (src-over) of the ORA
  layers and of the per-layer PNGs == the flat PNG, **max diff 0** (`export_zip`).
- **One layer / selection files == the zip entries**: every `_one` / `_sel` PNG is byte-equal to the zip entry of the same
  layer (multi-set models: the entry of the right set, e.g. ngons `…_ngons b_L02_stamp_Logo.png`, ABeautifulGame
  `…_Knight_Black_L02_stamp_Logo.png`).
- **Selection with the flattened texture** (`export_selected`, `_export_selflat`): ink 27,489 texels = Paint 1 (19,191) +
  Logo (8,298), i.e. the Base is not in it.
- **Hidden but selected**: `sphere_L03_text_WTE.png` (hidden layer) written, ink 2,215.
- **ORA** readable: zip with `mimetype` first and stored (`image/openraster`), `stack.xml`, `mergedimage.png`.
  **PSD** readable: `8BPS` v1, RGB 8-bit, layer count = exported layers (record count -N, merged alpha), unicode names,
  hidden flags, blend `norm`.

## 4. Code review (diff `a8beea6..working tree`, `full_vs_r9.diff`)

- **Test cheating**: none found. Removed asserts are either replaced by stronger ones (`gpu_tests`: three `any(...)` ->
  `written == planned`) or belonged to the deleted app-side classifier (`classify_plan` / `kind_from_name`, replaced by
  the core's `plan_contents`); `scenario_names().len()` 39 -> 42 for the 3 new scenarios. The replacement test
  `the_list_kinds_come_from_the_core_plan` is weaker in one spot (`cache.kinds.len() == plan.len()` is tautological),
  acceptable since the classifier it checked no longer exists. No wrap / limit assert touched.
  `end_to_end.rs` and `control_sweep.rs` show huge diffs only because they were CRLF in `a8beea6` and are LF now
  (`git diff -w --ignore-cr-at-eol`: +53 and +5 lines). All changed files are LF.
- **Zip / staging path**: stage `<zip>.tmp-<pid>.d` and temp zip `<zip>.tmp-<pid>` in the output folder; guards drop in
  the right order (ZipWriter, then `TempFile`, then `StageDir`, then `CreatedDir`); rename replaces an existing zip
  only after `sync_all`. Two app instances use different pids; one instance cannot run two exports (`blocked_by_job`).
  Core tests cover cancel while staging and while packing (1 lane and many), stage / temp blocked, panic, a fresh folder
  removed again, the old zip unchanged byte for byte, replacement, determinism across lane counts. Staged files and
  zip entries are paired by position (`staged.files.zip(contents)`, only a `debug_assert` on the lengths); correct
  today because `run_files` reports files in plan order, and `zip_entries_are_the_loose_files` would catch a change.
  No race found.
- Selection rules (dispatch + `layer_selection`): normalised on read; not in `UiState`, `Prefs` or the project
  (`grep selected_layers`); no history entry / revision change (unit test + `export_selected` asserts with real clicks).

## 5. Defects

| # | Severity | What | Evidence |
|---|---|---|---|
| D1 | major | App-side half of the feature (UI, dispatch, dialog, scenarios, ARCHITECTURE) is **uncommitted**; the branch commit alone does not build the feature | `git status`; `worktree_uncommitted.diff` |
| D2 | major | GPU gate test `control_sweep::gpu_tests::export_dialog_controls_and_export` fails (test reads per-layer PNGs after unticking per-layer output) | `test_gpu_sweep.log`; `gpu_tests.rs:279` vs `:321-328` |
| D3 | major | clippy 5 warnings (rule: 0) | `clippy.log` |
| D4 | minor | Single-layer / selection export from a multi-set model drops the set name: `Knight_Black`'s and `Pawn_Body_Black`'s L02 Logo both become `ABeautifulGame_L02_stamp_Logo.png`; exported one after the other into one folder, the second replaces the first (the confirm step warns) | `runs\flow_ABeautifulGame\shot_path_one`, `shot_path_sel` |
| D5 | minor | `HANDOFF.md` 4.1 and `dist/HOW_TO_TEST.txt` not updated (plan section 8) | `git status` |
| D6 | minor | `export_flow PATH` cannot run on a saved `.wtep` whose active layer is a stamp (harness expects one undo entry for the stroke) | `runs\flow_old_lantern\shot.json` |
| D7 | minor | numpy / Pillow absent (environment, not the branch): CLAUDE.md's statement is wrong on this machine | section 3 |

## 6. Workflow gaps no test covers

1. Cancelling a **zip** export from the app (Cancel button during "Packing"): core-only coverage.
2. Replacing an existing zip **through the app**: the GPU test stops at the confirm step and cancels; replacement is
   core-only (`zip_replaces_an_existing_package_atomically`).
3. Delete / Merge Down / Duplicate with several layers selected act on the active layer only; no test that the other
   selected layers survive, and the only hint is the footer line.
4. A stamp spanning two materials: "Export this layer…" writes only the active set's part; no test, no hint.
5. A real binary FBX (none on this machine; only the generated ASCII cube).
6. A zip held open by another program (rename fails) and very long paths (the stage folder adds about 25 characters).
7. ABeautifulGame wrap note (model-caused, not export): the wrapped stamp file has 1,063,305 inked texels on a
   2048x2048 set (shared / mirrored UVs).

## 7. Verdict

The workflow works in the release exe built from the working tree: one layer, a selection (hidden included), and
everything as one zip export correctly on 11 models / formats, entries are byte-identical to loose files, every
per-layer PNG holds exactly its layer, ORA / PSD parse, default `export_run` and old-project exports are byte-identical
to round9-wip. Not gate-ready: D1 (uncommitted), D2 (red GPU test), D3 (clippy).
