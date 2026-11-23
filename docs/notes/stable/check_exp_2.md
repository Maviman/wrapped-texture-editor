# check_exp_2: independent check of the export branch (pass 2)

Checker: Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_exp`, branch `export-select` at
`80f73ca` (round9-wip `a8beea6` + `46c5096`, `e4e45a2`, `252de2e`, `9f2d2fb`, `80f73ca`). I did not edit any source.
Evidence is under `target\stable\check_exp_2\` (git-ignored): `test_full.log`, `test_core_tests.log`,
`test_gpu_sweep.log`, `clippy.log`, `build.log`, `runs\summary.txt`, one folder per run, `verify.py` and
`verify.json`, and `bin\wte_exp_c2.exe` / `bin\wte_r9_copy.exe`.

Goal: try to refute that a user can import a model, layer designs on it, and then export one layer, a selection, or
everything as one .zip. **I could not refute it.** No new major defect was found, and every earlier defect (D1-D7) is
fixed or explained.

## 1. Build and tests (run by me on this machine)

| Check | Result |
|---|---|
| `test --workspace` | wte_app 371 passed / 0 failed / 17 ignored; wte_core lib **774 passed / 5 failed** / 19 ignored. The run stops at the first failing crate. |
| `test -p wte_core --tests --no-fail-fast` | lib as above; brush_tips 15/0, end_to_end 9/0, project_roundtrip 23/0 |
| Total | **1,192 passed, 5 failed, 36 ignored**. This is the same as the builder's count. |
| `test --workspace --doc` | 0 doc tests |
| The 5 failures | All are known round-9 wrap failures with the same numbers: `wa17_jump_small_gaps_climbs_a_cube_standing_on_the_floor` (ink/area >= 0.95 failed), `wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` (ink 1.18898), `wa24_jump_small_gaps_is_bounded_by_the_footprint` (ink 0.9146), `wa35_exact_parts_stay_exact` (92.3681 % within 2/255 vs 99.9 %; max 51 > 8), `wa4_r8_bumpy_plates_receive_the_picture_once_whole_and_smooth` (60 deg: ink 1.1177 > 1.10, cells 0.769..1.432). This branch touches no wrap code. The timing tests wa7 and check_a_own passed in this run. |
| GPU `test -p wte_app -- --ignored control_sweep` | **13 passed / 0 failed**. These include `export_dialog_controls_and_export` (D2) and `export_dialog_zip_package_and_selected_layers`. |
| `clippy --workspace --all-targets` | **0 warnings** |
| `build --release -p wte_app` | OK. The exe SHA-256 is `0bd11643…`, identical to the builder's `wte_exp_fix1.exe`. |

## 2. Scenario runs (release exe, 1280x800, every run `"ok": true`)

| Run | Model | Notes |
|---|---|---|
| sel | (sphere sample) export_selected | Ctrl, Shift, plain and right clicks; hidden WTE text selected and exported (2,215 inked texels); one layer; flattened selection |
| zip | export_zip | 7 entries, 240,057 bytes |
| run_exp / run_r9 | export_run | 7/7 files SHA-256 identical to wte_r9.exe, same names |
| flow_gen | generated cube: OBJ+MTL+PNG, glTF, GLB, ASCII FBX | |
| flow_ngons | test-assets ngons_tiling.obj (4 sets, one set name with a space) | |
| flow_Lantern | Lantern.glb | |
| flow_old_lantern / flow_old_roundtrip | r9-saved `lantern_r9.wtep`, `roundtrip_r9.wtep` | D6 fixed |
| flow_ABeautifulGame | ABeautifulGame.glb (15 sets) | 84 files verified, zip 226,958,408 bytes, 23 s |
| **flow_DamagedHelmet** (new) | DamagedHelmet.glb | 8 entries, 43 MB zip |
| **flow_WaterBottle** (new) | WaterBottle.glb | |
| **flow_gltf_nodes** (new) | test-assets gltf_node_transforms.gltf | |
| **flow_torus_knot** (new) | test-assets torus_knot.obj | |
| wf_{lantern,roundtrip}_{exp,r9} | workflow_path on both old .wtep | 11/11 exported files identical to r9 for each; screenshots identical |
| id_{default,layers,stamp,roundtrip}_{exp,r9} | | all 4 screenshots byte-identical to r9 |

## 3. Independent verification with Python (numpy and Pillow; `verify.py`, results in `verify.json`)

I ran `python -E -P`. numpy 2.4.6 and Pillow 11.3.0 are in the user site-packages, and `-I` hides them (D7 is an
environment issue and is confirmed).

- **Zip vs loose files** (export_zip): there are 7 entries for 7 loose files, with the same names, **byte-for-byte
  identical**. Exporting the zip twice gives identical zips. `testzip()` is clean. Methods are stored or deflated, and
  every entry is dated 1980-01-01.
- **Selection and one-layer files vs the full zip** (every export_flow run, 13 models/formats): every `_one` and
  `_sel` PNG has a name that exists in the full-export zip, and its bytes are **identical** to that entry. Under a
  selection, multi-set names carry the set (D4 is fixed): for example
  `ABeautifulGame_Knight_Black_L02_stamp_Logo.png`, `ngons_tiling_ngons b_L01_paint_Paint 1.png`. The two selected
  files are always different pictures.
- **Flattened vs a re-composite of the per-layer PNGs** (straight-alpha normal over, compared premultiplied): every set
  of every zip and loose export is within at most 1.33/255. Across all runs 0 texels differ by more than 2.
- **Flattened selection** (export_selected `_export_selflat`): it equals Paint 1 + Logo re-composited from the
  independent export_run files to 3e-14, and it is transparent where only the Base shows.
- The **selected and one-layer files** in export_selected are byte-identical to the export_run and export_zip loose
  files of the same name (Logo, Paint 1). The WTE text differs only because export_run and export_zip use the
  T7-style text. The scenario itself checks WTE against its layer.
- **ORA**: for every set, `mimetype` is the first entry and is stored. The stack.xml layer count equals the number of
  per-layer PNGs. `mergedimage.png` equals the flattened PNG exactly, and every ORA layer, cropped at its x/y, equals
  its per-layer PNG. My first script version reported mismatches and one texel off by 64. Both came from my script
  (two layers named "Logo"; the RGB of an alpha-0 texel). After I fixed the script, there are 0 mismatches.
- **PSD**: signature 8BPS, version 1, 4 channels, 8-bit RGB. Width and height equal the flattened image. The layer
  count equals the number of per-layer PNGs in every set.
- No `.tmp-` file or stage folder was left in any output folder.

## 4. Code review of the diff a8beea6..HEAD

- **No test cheating found.** I listed the removed lines with `assert`. The removed asserts belong to the removed
  `classify_plan`/`kind_from_name` and were replaced by equal or stronger checks (`PlanCache` names, kinds and plan
  equality). The GPU test's `.jpg/.ora/.psd` existence asserts became "written == planned" plus PSD layer and
  per-layer pixel checks, which is stronger. `scenario_names().len()` went from 39 to 42 for the 3 new scenarios. The
  only new `#[ignore]` is the new GPU test ("needs a GPU"), which passes. D6's harness fix adds paint layers *before*
  the history is remembered, and the "one history entry" assert is still 1.
- The large line counts for `end_to_end.rs`, `layers.rs` and `control_sweep.rs` come from a CRLF-to-LF conversion: the
  base had CRLF and the branch has LF, which follows the repo rule. Whitespace-insensitive, end_to_end changed by +54
  lines.
- **Zip and staging path (`run_zip`):** the stage folder is `<zip>.tmp-<pid>.d` inside the output folder and is
  removed by a drop guard on success, error, cancel and panic. The temp zip has a guard. `zip` is dropped before the
  guard, so the handle is closed before the delete on Windows. `sync_all` then `rename` replaces the old zip
  atomically, and the old zip survives every failure (core test `a_cancelled_or_failed_zip_export_leaves_nothing`
  covers cancel while staging, cancel while packing, a blocked stage folder, a blocked temp zip, a panic, and a fresh
  folder removed again). Staged files are paired with entry names by position. That is correct because
  `SetReport.files` is pushed in plan order and the lanes are collected in set order. The 15-set parallel
  ABeautifulGame export confirms it: every entry matches its own layer. Two app copies get different pids, and
  `blocked_by_job` stops a second export in one app. **No race found.**
- **Multi-select rules** (`row_click_action`, `toggle_layer_selected`, `select_layer_range`, `Session::layer_selection`):
  Ctrl toggles and wins over Shift. Shift selects a range from the active layer. A plain click activates the layer and
  collapses the selection. The active layer is always in the selection, and the last selected layer cannot be removed.
  The selection is normalised on every read, so stale or other-set ids are dropped. It is app state only: it is not in
  `UiState`, history or the file, and unit tests assert unchanged undo length, revision and modified flag. Layer
  commands (New, Delete, Merge Down, the tools) stay on the active layer, and the footer says "N layers selected ·
  tools paint on X".
- No FORMAT_VERSION or project/save change and no new crate (Cargo.toml and Cargo.lock are unchanged). `zip` was
  already a dependency.

## 5. Earlier defects (check_exp_1): status

| # | Status |
|---|---|
| D1 app side uncommitted | fixed (`9f2d2fb`; `git status` is clean) |
| D2 GPU test export_dialog_controls_and_export | fixed (13/13 GPU) |
| D3 clippy 5 warnings | fixed (0) |
| D4 set name dropped under a selection | fixed (verified on ABeautifulGame and ngons, names = full-export names, bytes equal) |
| D5 HANDOFF 4.1 / HOW_TO_TEST | fixed |
| D6 export_flow on saved .wtep | fixed (both old projects ok) |
| D7 numpy/Pillow | environment: they are present for Python 3.11 user site. CLAUDE.md's `python -I` hides them, and `-E -P` works. Not a branch defect. |

## 6. Not verified at runtime and gaps without tests (no defect claimed)

- A zip held open by another program: from the code, the rename fails, the result is an error toast, and the temp
  zip and stage are removed. I did not run it, because no scenario can do this without OS-level interference.
- Cancel of a zip export *through the app* (only the core test, plus the generic progress-dialog cancel GPU test).
- A stage folder left behind after a crash: on the next run it is cleaned only if the pid is the same.
- Observations (UX, not bugs): a dialog opened with "Export this layer..." keeps `sets = [that set]` and the flattened
  texture unticked when the user switches Layers to Visible/All, so it is not "everything" until the user ticks those
  again (the file list shows this). A zip of a selection is also named `<base>.zip`, the same as the full package, so
  in the same folder it replaces the full zip after the "will be replaced" confirm. "Export this layer..." on a stamp
  that spans two sets exports only the clicked set's layer. Multi-select Delete and Merge act on the active layer only.

## 7. Model-caused (not export)

On ABeautifulGame the wrapped stamp's file has 1,063,305 inked texels on a 2048² set. This comes from the model's
shared or mirrored UVs (as in check_exp_1), and the export reproduces the layer exactly.
