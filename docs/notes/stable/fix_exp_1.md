# fix_exp_1 — fix pass 1 on branch `export-select`

Builder: Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_exp`, branch `export-select`.
Input: `check_exp_1.md` (defects D1-D7). `docs/notes/stable/export.md` does not exist (the first builder returned
nothing), so this note and `check_exp_1.md` are the record of the feature's state. Evidence under
`target\stable\fix_exp_1\` (git-ignored): `runs\summary.txt`, one folder per run, `bin\wte_exp_fix1.exe`
(SHA-256 `0bd11643…`), and `target\stable\fix_test_full.log`, `fix_test_gpu_sweep.log`, `fix_clippy.log`,
`fix_build.log`.

## Commits

1. `9f2d2fb` — D1: the app-side half, committed exactly as the checker checked it (working-tree diff SHA-256
   `5badc2c8…`, the same as `check_exp_1\worktree_uncommitted.diff`), before any fix, so the fixes are separate
   diffs.
2. The fix commit (this note): D2, D3, D4, D5, D6.

## Defects

| # | Status | What was done |
|---|---|---|
| D1 uncommitted app side | fixed | commit `9f2d2fb` |
| D2 GPU test `export_dialog_controls_and_export` | fixed | The test unticks "Per-layer PNGs" and later reads every per-layer PNG. It now unticks it (asserts off), ticks it again (asserts on), so the stronger per-layer pixel check after the export runs. No assert removed. |
| D3 clippy 5 warnings | fixed | `std::slice::from_ref(&zip_path)` (export/mod.rs, 2x), `labels.contains(&want)` (scenarios.rs), doc line "+ Export" read as a list item -> "plus Export" (gpu_tests.rs). Clippy: 0 warnings. |
| D4 set name dropped under a selection | fixed (2nd attempt) | See below. |
| D5 HANDOFF / HOW_TO_TEST | fixed | `HANDOFF.md` 4.1: the three scenario names and what they do; `dist/HOW_TO_TEST.txt`: section "Exporting layers" (Ctrl / Shift / plain click, row menu "Export this layer..." / "Export selected layers... (N)", the zip tick). |
| D6 `export_flow PATH` on a saved `.wtep` | fixed (harness) | New setup step `flow_paint_layers` before the history is remembered: every set whose active layer is not a paint layer (stamp / text / none) gets one, as the brush's own steps (3)/(4) would on the first press. The stroke assert "one history entry" stays exactly 1. A locked / hidden active layer is reported as a scenario error, not hidden. A fresh import is unchanged (Paint 1 is already active in every set). |
| D7 numpy / Pillow "absent" | environment, clarified | They **are** installed for Python 3.11 (numpy 2.4.6, Pillow 11.3.0) in the **user** site-packages. `python -I` (as CLAUDE.md says to run tools) disables the user site, so they look absent. `python -c "import numpy, PIL"` from a neutral folder works. CLAUDE.md not edited (the main worktree's file; not this branch's job). Suggestion for the main session: run tools with `python -E -P` (no env vars, script folder not on the path) instead of `-I`, or install the two packages for all users. |

### D4: the naming rule

Before: `multi` (the `_<set>` part of file names) = the number of sets **left after** the layer selection dropped the
sets without a selected layer. So one layer of a multi-set model was named as if the model had one set, and two sets'
layers with the same index and name (ABeautifulGame `Knight_Black` / `Pawn_Body_Black` `L02_stamp_Logo`) got the
same file name.

Attempt 1: count the set scope (all sets or `opts.sets`) before the layer filter. Core tests passed, but
`export_flow` on ngons / ABeautifulGame still wrote `ABeautifulGame_L02_stamp_Logo.png`: the dialog opened from a layer
row (`dispatch::export_for_layers`) sets `opts.sets = Some([that layer's set])`, so the scope was one set again.

Attempt 2 (kept): `selected_sets` returns `multi = if opts.layers.is_some() { snapshot.sets.len() > 1 } else
{ <sets exported> > 1 }`. Under a layer selection the set name follows the model; without one nothing changes (a set
filter of one set still drops the set name, as in round 9). A selected layer's file now has the same name as in the
full export, and in the zip.

Byte identity: default and set-filtered exports are untouched (the rule only differs when `opts.layers` is `Some`,
which round 9 did not have). `export_run` new vs `wte_r9.exe`: all 7 files SHA-256 identical; `workflow_path` on the
r9-saved `lantern_r9.wtep`: all 11 exported files identical, screenshot identical.

Intended output change (listed as required): exporting one layer or a selection from a **multi-set** model now writes
`<base>_<set>_L..` names (before this fix, on the uncommitted branch only: `<base>_L..`). Single-set models are
unchanged.

Tests:
- core `export::tests::a_selection_leaves_out_sets_without_selected_layers`: updated to the new names (the one remaining
  set keeps `_<set>`); stated here because it changes an expected value.
- core `export::tests::a_selection_on_a_multi_set_model_keeps_the_set_name` (new): two sets' `Paint 1` layers at the same
  index give different names, each in the full export's plan; with `sets: Some([1])` too; the zip's entry name; a set
  filter without selection has no set name.
- app `dispatch::tests::export_for_layers_on_two_sets_keeps_the_set_names_apart` (new): the row-menu path
  (`export_for_layers` + `plan_contents`) on the TwoMaterial sample, two `Logo` layers at the same index -> two names,
  both in the full export. Fails under the old rule and under attempt 1.
- `end_to_end::two_material_pipeline` selection block: expects `layer_name(.., multi = true, ..)` and checks each name is
  in the full export's plan.
- `selftest::scenarios::selection_layer_name` (was `single_set_layer_name`): `multi = doc.sets.len() > 1`.

ARCHITECTURE.md: 5.24 (`naming.rs` comment), 7.4 (`multi` in the run line and the layer-scope bullet), 7.5 (flattened
naming note).

## Results (this machine, after the fix commit's code)

| Check | Result |
|---|---|
| `test --workspace --no-fail-fast` | **1,192 passed, 5 failed, 36 ignored** (wte_app 371/0/17; wte_core lib 774/5/19; brush_tips 15; end_to_end 9; project_roundtrip 23) |
| the 5 failures | the known round-9 wrap failures, same numbers as check_exp_1 and the r9 gate: `wa17_jump_small_gaps_climbs_a_cube_standing_on_the_floor` (ink/area < 0.95), `wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` (ink 1.1890), `wa24_jump_small_gaps_is_bounded_by_the_footprint` (ink 0.9146), `wa35_exact_parts_stay_exact` (92.37 % within 2/255, limit 99.9 %; max 51 > 8), `wa4_r8_bumpy_plates_receive_the_picture_once_whole_and_smooth` (60 deg: ink 1.1177 > 1.10, cells 0.769..1.432 > 1.30). No wrap code touched. |
| timing note | the first full run (attempt 1 code) also failed `wa7_bad_meshes_and_bad_pictures_never_fail` (366.5 ms > 200 ms, thin_slab) and `check_a_own_bad_meshes_never_fail` (276.5 ms > 200 ms, moebius) while another worktree's `wte_core` test process was running. Run alone: 169.0 ms and 197.7 ms, both pass; the second full run: both pass. CPU-contention timing, not this branch. `check_a_own` is within 2.3 ms of its limit on an idle machine. |
| `test -p wte_app -- --ignored control_sweep` (GPU) | **13 passed, 0 failed** |
| `clippy --workspace --all-targets` | **0 warnings** |
| `build --release -p wte_app` | OK, `bin\wte_exp_fix1.exe` SHA-256 `0bd116432a46e09c…` |

Scenarios (release exe, 1280x800), all `"ok": true`:
`export_selected`, `export_zip`, `export_run` (+ r9 copy for the hash compare), `export_flow` without PATH (OBJ / glTF /
GLB / ASCII FBX cube), `export_flow` on ngons_tiling.obj, Lantern.glb, ABeautifulGame.glb (25 s), and on the two
r9-saved projects `lantern_r9.wtep` and `roundtrip_r9.wtep` (D6: failed before), `workflow_path` on `lantern_r9.wtep`
(new and r9), `default`, `layers`, `stamp`, `roundtrip` (new and r9: screenshots byte-identical).
Written names: ABeautifulGame one layer `ABeautifulGame_Knight_Black_L02_stamp_Logo.png`, selection
`ABeautifulGame_Pawn_Body_Black_L01_paint_Paint 1.png` + `..._L02_stamp_Logo.png`; ngons one layer
`ngons_tiling_does_not_exist_L02_stamp_Logo.png`.

## Open (not changed here)

- check_exp_1 section 6 gaps (zip cancel / replace through the app, multi-select with Delete / Merge, a stamp spanning two
  sets exported from one row, a real binary FBX, a zip held open by another program) remain without tests.
- The 5 wrap test failures belong to the wrap branch (round 9), not to this feature.
