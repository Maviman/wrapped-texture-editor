# objects-tabs WP0 — baseline (branch `objects-tabs` @ a3fdd97 = `main` 1f0be41 + the plan)

Recorded 2026-11-24 by the WP0/1/2 builder before any code change. All outputs are under `target\objtabs\baseline\` in
the worktree `G:\software_projects\texture_wrapper_ap_ui` (not committed: `target/` is git-ignored).

## Release exe

- `cargo-gnu.cmd build --release -p wte_app` at a3fdd97 -> copied to `target\objtabs\baseline\wte_base.exe`
  (SHA-256 `2F48C0E81EDFE922563D36D65B18EE8234831AD03C0E3A9A3EB0980A00380D04`). This is the pixel / export reference of
  the later packages (WP3b: full-frame identity; WP4+: view-rect identity and `check_exports.py`).

## Workspace tests and clippy

- `cargo-gnu.cmd clippy --workspace --all-targets`: **0 warnings** (`target\objtabs\baseline\clippy.log`).
- `cargo-gnu.cmd test --workspace`: cargo stops after the first failing test binary (the `wte_core` lib, WA20), so the
  integration tests did not run in that command. Their baseline binaries (copied to `target\objtabs\baseline\`) were run
  directly afterwards. Totals, per binary:

  | Binary | Passed | Failed | Ignored |
  |---|---|---|---|
  | `wte_app` (`src/main.rs`) | 382 | 0 | 19 |
  | `wte_core` lib | 802 | 1 (`wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`, the known red) | 24 |
  | `tests/brush_tips.rs` | 15 | 0 | 0 |
  | `tests/end_to_end.rs` | 9 | 0 | 0 |
  | `tests/project_roundtrip.rs` | 24 | 0 | 0 |
  | doc tests | 0 | 0 | 0 |
  | **Total** | **1,232** | **1** | **43** |

  Same as the `stable` / `main` @ 1f0be41 numbers in CLAUDE.md. Use `cargo-gnu.cmd test --workspace --no-fail-fast`
  to see every binary in one run.

## Core perf tests (`cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`)

Run on the baseline test binary (`wte_core_lib_tests_base.exe perf_ --ignored --test-threads=1`, `crates\wte_core` as
cwd), idle machine apart from this run:

| Test | Slowest parameterization | Limit |
|---|---|---|
| `perf_check_a_own_meshes_within_200_ms` | 75.9 ms (torus x50) | 200 ms |
| `perf_wa7_parameterizations_within_200_ms` | 94.9 ms (doubled_cube x1, jump + overlap) | 200 ms |

Both passed; whole run 36.2 s.

## Scenario sweep (`--size 1280x800`, baseline exe)

Script `target\objtabs\run_sweep.sh EXE OUTDIR JOBS` (sequential, one folder per job with `shot.png/json/log`, named
shots and the exports the scenario writes), jobs `target\objtabs\jobs_baseline.txt`, summary
`target\objtabs\baseline\summary.txt`.

- All 40 scenarios of `NAMES` that run without a model path: **`"ok": true`** (empty, smoke, default, paint3d, paint2d,
  stamp, text, clay, preview, export, export_run, multi, roundtrip, undo, import, import_tex, import_nouv, unsaved,
  layers, pan2d, brush_types, brush_shape, brush_tab, text_gaps, text_styles, text_warp, text_path, text_tab,
  text_fonts, brush_hover, final_bake_async, wrap_surface, export_selected, export_zip, export_flow, export_unsettled,
  save_unsettled, export_nouv, export_errors, export_checks).
- The 7 scenarios that need a PATH, run on sample models (`G:\Claude_Resources\samples\khronos-gltf\`):
  `open_path` (ABeautifulGame -> `open_path_abg`, BoxTextured -> `open_path_box`), `workflow_path`, `fuzz_path`,
  `brush_types_path`, `text_preview_path` (BoxTextured), `stamp_edge_path` (Avocado), `import_save`
  (`test-assets\shapes\coincident_flipped_spheres.obj` -> `import_save_spheres`), plus `wrap_surface` with Avocado
  (`wrap_surface_avocado`): all `"ok": true`.
- Not a regression, recorded so nobody chases it: `import_save` needs a model with sidecar files and a surface under
  the scenario's click points. On BoxTextured.glb it fails ("the source bundle has 0 sidecar(s)"); on torus_knot,
  many_islands and nonmanifold it fails "the image stamp was placed: no stamp is selected" (the click misses). Those
  runs are not kept.
- Every job finished in 1-11 s except `fuzz_path` on BoxTextured (29 s).

The plan's new scenarios (`objects`, `objects_path`, `side_tabs`, section 8) do not exist yet; their references come
from the package that adds them.
