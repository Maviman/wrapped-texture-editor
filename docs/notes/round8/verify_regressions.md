# Round 8 verify: regressions and identity (skeptic, lens "regressions and identity")

Date 2026-11-22, about 05:15-06:00 local. I did not build round 8. No product code changed. Outputs:
`target\round8\verify_reg\` (driver `run.ps1`, file comparers `cmp_files.py` / `cmp_layers.py`, runs `r7\`, `r8\`,
`wsx\`, `wfw\`, `compat_*`, `core_tests\`, `clippy_verify.log`, crops `*.png`). Exes, run one at a time (the driver
waits while any `wte_*` / `wrapped_texture_editor` process runs; it had to wait twice, for another agent's
`wte_round8` run, about 50 s and 30 s):
- round 8 `target\round8\wte_round8.exe`, SHA-256 `19d7844b...e97e54b2a` = `work\merged\target\release\wrapped_texture_editor.exe` (checked);
- reference `target\round7\wte_round6_7.exe`, SHA-256 `2787c1b9...0f2bf8`.
Machine: free memory about 1.2-1.3 GB; no other cargo / rustc / Python process seen at my starts.

## Verdict for this lens

The report's identity claims for the **Project path, the brush, exports, the file format and reopened round-6
files** hold under my re-runs. Tests and clippy are as the report states (tests **not** green: 4 failures, said so).
Two statements are wrong or overstated:
1. "The water bottle label is **still exact**" (verdict bullet) is false: in round 8 the WaterBottle label is
   **relaxed** (wrap_surface step 10: relaxed true, stretch 2.1 %; workflow_path logo: relaxed true, stretch 10.3 %),
   and the same workflow placement differs from round 6 on 28.4 % of the logo's texels (71.6 % within 2/255, max 255)
   and 49.7 % of the "WTE" text's. The gate's own data (sweep table, compat table) say so.
2. "Exact cases unchanged" holds for the core tests against their analytic references (WA1/2/3/5/16/18/25 pass, same
   texel counts as round 6), but round 8's **un-relaxed** layout (relaxed false) is not the round-6 layout: the
   geodesic front / 1.7 x reach changes the laid set on flat or piecewise-flat models (details below). WA5's corner
   now has 2 cut triangles (round 6 notes: 1). The report does not list this as a behaviour change.

## 1. Source diff `work\base` -> `work\merged` (risky changes outside the plan)

- `merged\crates` = `pkg_b\crates` and `ARCHITECTURE.md` equal (diff -rq empty). `base\crates` and
  `ARCHITECTURE.md` = main tree. `Cargo.toml` / `Cargo.lock` unchanged: no new crate.
- Changed files are exactly the plan's lists (core: `doc_stamps.rs`, `document.rs`, `lib.rs`, `samples.rs`,
  `stamp_job.rs`, `wrap_bake.rs`, `wrap_param.rs`, `wrap_tests.rs`, new `wrap_relax.rs`; app: `app.rs`, `io_flows.rs`,
  `jobs.rs`, `panels\tool_options.rs`, `selftest\mod.rs`, `selftest\scenarios.rs`, `stamp_bakes.rs`, `state.rs`,
  `tools\transform.rs`). **Unchanged**: `texel_map.rs` (`raster_tri`), `bvh.rs`, `brush.rs`, `stroke.rs`,
  `tip_bake.rs`, `stamp_bake.rs`, `composite.rs`, `doc_build.rs`, the whole `project\` (format) and `export\` folders.
  Plan 3.6's `wrap_mesh.rs` and `doc_build.rs` edits do not exist (step 6 not built, as reported).
- Shared code read:
  - `stamp_job.rs`: `StampBakeJob::run` now takes its quality from the job kind; `Full` jobs (all Project / UV Finals)
    still pass `BakeQuality::Full` to `bake_projected` / `bake_uv`; Preview / ParamOnly jobs are made only for
    wrapped stamps (`next_wrap_preview_job`, `wrap_param_job` both require `wrapped_projector`). A ParamOnly job of a
    non-wrapped placement returns no pixels. No Project change.
  - `doc_stamps.rs`: `has_stale_previews` / `bake_stale_previews` skip only stamps that are wrapped **and** while
    `defer_wrap_previews` is on; `edit_serial` is a counter only. Projected stamps unchanged.
  - `wrap_bake.rs`: the owner test applies only when a layer has an owner grid (`layer.owner`); `raster_set` gained a
    parameter. Only the wrap path.
  - Job system (`jobs.rs`, `stamp_bakes.rs`): new silent jobs; `busy()` skips silent jobs; `any_running()` now also
    counts silent wrap jobs (used by `app.rs:354`, the close path, and tests). Final bakes wait for a wrap job only
    when that job's stamp is unbaked in the same scene generation. `needs_frame_after_dispatch` is unchanged when no
    wrapped stamp exists (`waits_for_wrap` false, `wrap_waiting` false).
  - `wrap_relax.rs`: env "knobs" (`WTE_TEST_*`) are compiled out of non-test builds (`#[cfg(not(test))]` returns the
    default); in test builds they can change test results if set in the environment (test hygiene only).
  - `wrap_param.rs`: the round-6 layout code itself changed (spine key, `trace_spine`, reach radius, `finish`,
    `make_layer`): this is why un-relaxed layouts can differ from round 6 (section 4).
- No removed or weakened old test: every removed line in the diff is product code or a reworded scenario log line;
  `wrap_tests.rs` and `samples.rs` only add lines.

## 2. Identity re-runs, 1280 x 800, `wte_round8.exe` vs `wte_round6_7.exe` (`verify_reg\r7`, `r8`, `shot_cmp.txt`)

| Scenario | shot.png | other files |
|---|---|---|
| stamp, text_warp, export_run, text_path, text_styles, roundtrip, paint3d, paint2d, brush_types, brush_shape, brush_hover, stamp_edge_path (Avocado), text_preview_path (Avocado) | identical (0 px) | export_run: 7 / 7 exported files byte-identical; `check_exports.py` on both: **PASS**; text_warp `shot_warp3d.png` identical; roundtrip `.wtep` differs only in `manifest.json` `saved_unix` |
| brush_types with `RAYON_NUM_THREADS=1` (both exes) | identical (0 px); also equal to round 8's 12-thread run | |
| final_bake_async (async) | 5,126 px in x 794-977, y 623-663 only (toast fade; crop `toast_final_bake_async.png`) | both export folders 4 + 4 files byte-identical; `.wtep` differs only in `saved_unix`; all 11 logged composite hashes (`scratch values`) equal |
| final_bake_async with `WTE_SYNC_FINAL_BAKES=1` | 6,650 px in the same toast box only | same as above; hashes equal to the async run and to round 6+7 |
| wrap_surface (built-in) | 3,517 px in x 375-1107, y 227-608 (the wrapped stamps, expected) | Project composite H0 `cd272dc06ef4e935` in both exes (steps 3, undo, redo); `.wtep` same entry list, same manifest key paths, format_version 3 / min_reader 3 in both; only the wrapped layers, flattened, thumbnail and manifest values differ |

Claims "Project byte-identical", "round-7 brush shots identical", "exports byte-identical", "H0 equal": **confirmed**.

## 3. Reopened round-6 projects (`verify_reg\compat_in`, `compat_open`, `compat_cmp.txt`)

Saved by `wte_round6_7.exe` in this run: `sphere_r67.wtep` (its `wrap_surface` `shot_wrap.wtep`: "cells" with Overlap
+ Jump and "WRAP AROUND", both wrapped), `waterbottle_r67.wtep` and `avocado_r67.wtep` (`workflow_path` with
`WTE_STAMP_MODE=wrap`: logo and "WTE" wrapped). `open_path` in both exes: all three `shot.png` **identical (0 px)**;
round 8's logs contain no `wrap:` / `wrap fit` / `wrap preview` / bake line, so the saved pixels are kept.
**Confirmed.** Limit of this check: the shot shows the 3D view (stamps visible on the sphere), not every texel; the
absence of any bake line is the stronger evidence. Not repeated: the tint-change check (gate's 71.6 % on WaterBottle,
reported as not met).

## 4. Exact / developable cases

Core tests run from the existing `merged` test binary `wte_core-e71b07d3f438e94f.exe` (built 03:43 after the last
source edit 03:30; sources equal to pkg_b), `core_tests\exact.log`: WA1 (429,308 / 428,824 texels, 100 % within 2,
max 1: the same counts as round 6's notes), WA2 (ink 1.0010, cells 0.970-1.023), WA3 (band 1.00000, smear 0.00021 %),
WA5 edge (100 %, max 1), WA5 corner (covered 1.0, **cut 2**; round 6 notes: cut 1), WA16, WA18 (all 8 cases share
1.00000, max 0-1), WA25 (all relaxed false, rejected 0): **7 passed**. So the limits hold; byte identity with round 6
cannot be shown from the tests (they compare with analytic references).

App level, same clicks in both exes (`wfw\`: `workflow_path` with `WTE_STAMP_MODE=wrap`, exported layer PNGs compared
texel by texel, `cmp_layers.py`; `wsx\`: `wrap_surface` with PATH, step 10's logged ink):

| Model | round 6+7 | round 8 | texels |
|---|---|---|---|
| flat_annulus (workflow) | 17 tris, covered 67.8 % | 21 tris, relaxed false, covered 67.8 % | stamp and text layers **identical** |
| tiny_cube (workflow) | 6 tris, cut 1 | 6 tris, cut 1, relaxed false | **identical** |
| flat_annulus (wrap_surface step 10, same placement [502.2 228.4]) | 16 verts / 19 tris, covered 59.83 %, ink area 1.527160 | 22 / 27, relaxed false, covered 59.95 %, ink area 1.528753 | **differs** (+0.10 % ink on a flat model; step 10 does not save, so not compared per texel) |
| ngons_tiling (workflow, text "WTE" on set does_not_exist) | covered 30.3 / 24.9 / 14.5 / 15.5 %, cut 2 / 0 / 1 / 1 | 29.0 / 24.5 / 15.3 / 15.5 %, cut 0, relaxed false | text layer **differs on 9.3 % of 1.0 M inked texels** (90.7 % within 2/255, max 255; alpha>0 941,995 -> 994,756); `ngons_text_cmp.png`, `ngons_3d_zoom.png`. ngons_tiling is not planar (pieces over z -0.1..6.1), so not a WA "exact" case |
| mobius (workflow) | 257 tris, covered 95.8 % | 432 tris, **relaxed true**, stretch 1.1 % | logo 78.6 % within 2/255; text 88.1 % |
| WaterBottle (workflow) | 333 tris, covered 99.1 %, cut 4 | 664 tris, covered 100 %, cut 0, **relaxed true**, stretch 10.3 % | logo 71.6 % within 2/255 (alpha>0 37,900 -> 39,273); text 50.3 % |

Reading: wherever round 6 left a torn (cut) triangle that is not a closure, `seed_is_exact` sends the layout to the
relaxation (WaterBottle's 4 cuts), and the round-6 layout code itself changed, so even un-relaxed layouts can lay
different triangles (flat_annulus at step 10, ngons_tiling). These are probably improvements (more coverage, no
tears), but they are behaviour changes on flat / developable / piecewise-flat models that the report neither lists
nor measured against round 6; the report's "still exact" for the WaterBottle label is wrong.

## 5. Tests and clippy

- Test binaries of gate 1 run again directly (no rebuild; `core_tests\*.log`): wte_app 357 passed / 16 ignored;
  wte_core lib **743 passed, 4 failed** (wa4_r8, wa20, wa22, wa24) / 17 ignored; brush_tips 15; end_to_end 9;
  project_roundtrip 23. Total **1,147 passed, 4 failed, 33 ignored**: as the report says. The plan's gate requirement
  "cargo test --workspace 0 failed" is **not met** (the report says so).
- Clippy: `cargo clippy --workspace --all-targets --offline` re-run in `work\merged` under `BUILD_LOCK` (label
  `round8-verify-regressions`, lock free before, deleted after; nothing else compiling). The restored sources' new
  mtimes made it re-check both crates ("Checking wte_core", "Checking wte_app"), so this is a fresh lint, not a cache
  replay: **0 warnings, 0 errors, exit 0, 1 min 35 s** (`clippy_verify.log`). Confirmed.

## 3b. Round-8 file in round 6+7

`r8\wrap_surface\shot_wrap.wtep` (saved by round 8) opened with `open_path` in both exes: `shot.png` identical, no
`wrap:` line in either log (`compat_back\`). No format change: confirmed.

## 6. Not done

- No cargo test rebuild (the existing binaries match the sources by time stamp and by gate 1's log).
- No per-texel round-6 comparison of WA exact cases (no round-6/7 core test binary exists: `round6\work`,
  `round7\work` hold no `target`; the main tree's binaries are from 2026-11-14/17).
- No request for files I do not own.
