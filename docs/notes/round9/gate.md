# Round 9 gate — notes

Gate: round 9 gate agent (Opus), 2026-11-22, 17:50-19:10 local. Worktree `G:\software_projects\texture_wrapper_ap_r9`,
branch `round9-wip` at `42996df` (= `round9-b`: package A + check:A + package B + check:B). Raw outputs under
`target\round9\gate\` (git-ignored): `logs\`, `runs_r9\`, `runs_r67\`, `runs_sync\`, `compat\`, `sweep_r9\`,
`sweep_r9_2048\`, `timing1\`, `sheets\`, `ref_shots\`, `tools\`, `bin\`. No product source was changed. Report:
`docs/reports/ROUND9_GATE_REPORT.md`.

## Measuring conditions (read first)

**The machine was not idle.** The user's game `BlackOps3.exe` (pid 36672, started 11:43, 6.5 GB working set) ran the whole
time. Before the timing runs it used about 35 % of the 8 cores alone (process CPU delta), total CPU 45-70 % before any
of my processes started. During the timing session (18:28-18:53) a sampler (`timing1\load_samples.txt`, every 10 s, 147
samples) saw the game in every sample: game median 36 % of the 8 cores (max 50 %), total CPU median 73 % (42-100 %),
free memory 37-39 GB. I did not stop it (never kill a process I did not start). No cargo / rustc / other app process ran
during the timing session (`driver.log` lists the running processes before each run: only the game). So every time in
section 7.3 was measured with roughly 3 of 8 cores taken by another program; the round-8 baseline was re-measured
under the same load, alternating with round 9, so the ratios are fair to each other but the absolute numbers are
pessimistic. CPU: Intel i7-9700K, 8 cores / 8 threads (the plan says "fast CPU"); GPU RTX 5070.

## 0. Line endings

The six files the task named (`crates/wte_core/src/uv_share.rs`, `crates/wte_core/src/wrap_mesh.rs`,
`docs/notes/round9/{pkg_a,check_a,pkg_b,check_b}.md`) are **already LF** in the working tree and in `HEAD` (0 CR bytes,
`git ls-files --eol`: `i/lf w/lf`). Git Bash's `grep -c $'\r$'` reports every line of them as CRLF, which is wrong; a
byte count with Python shows 0 CR. Nothing to convert, so no commit was made for step 0.

What **is** CRLF: three existing files that were LF in `round8-wip` and in `main` were converted to CRLF in round 9 by
check:A (whole file, every line): `crates/wte_core/src/wrap_param.rs` (3,421 / 3,421 lines CRLF; first CRLF commit
`ce7f656`), `wrap_tests.rs` (3,559 / 3,559; `ce7f656`), `wrap_relax.rs` (2,247 / 2,247; `591617d`). That is why
`git diff --stat round8-wip..round9-wip` shows 5,898 / 3,748 / 6,037 changed lines for them; with
`--ignore-cr-at-eol` the real change is +1,058 / -114, +922 / -176, +1,090 / -9. The task forbids touching other files'
line endings, so they are left as they are: **a decision for the main session before landing** (convert them back to
LF so the history shows the real diff). `samples.rs` is CRLF in `main` already (not a round-9 change).

## 1. Tests, clippy, todo, release, exe

- `cargo test --workspace --no-fail-fast` (`logs\test1.log`, 7 min 22 s incl. compile, game running): **1,170 passed, 5
  failed, 35 ignored** (wte_app 363 / 16 ignored; wte_core lib 760 passed, 5 failed / 19 ignored; brush_tips 15;
  end_to_end 9; project_roundtrip 23). Same as check:B. Failing (asserts untouched): `wa4_r8` (60 deg: ink 1.1177,
  cells 0.769..1.432), `wa20` (ink 1.1890, cells 0.665..2.886; dup 0.976 %, scatter 0.564 %, seams 0, covered 1.0
  pass), `wa17` (ink 0.915 >= 0.95 fails; cube 13,561 texels, floor 24,066, smear 0.003 %), `wa24` (WA17 half: ink
  0.9146, dup 0; the grid half is not reached), `wa35` (capped cylinder side 92.3681 % within 2/255, max 51; strip pins
  only would give 82.02 %, max 255). The wall-clock tests (`wa7`, `check_a_own_bad_meshes_never_fail`) passed under the
  game's load.
- Re-run of `wrap_tests::wa` with `--nocapture` (`logs\wa_tests_nocapture.log`): 34 passed, the same 5 failed, numbers as
  above; the passing WA lines' numbers are quoted in the report.
- `cargo clippy --workspace --all-targets` (`logs\clippy.log`): 0 warnings.
- `todo!` / `unimplemented!` in `crates\`: 0.
- `cargo build --release -p wte_app` (`logs\release.log`): ok, 2 min 49 s. Exe copied to
  `target\round9\gate\bin\wte_r9.exe`, 26,677,760 bytes, **SHA-256
  4F9E14222A17C9E1CF096FB7281822EC8E4E70E40DD6E9D8744CB999F0B358B2** (equal to `target\release\wrapped_texture_editor.exe`).
  It differs from check:B's `wte_cb_final.exe` (3541FB3F...), which was built in another worktree path; the sweep
  numbers and shots are identical to check:B's (below).
- Reference exes copied into `bin\`: `wte_r67.exe` (9F4F93CB...) and `wte_r8.exe` (DB61E168...).

## 2. All 39 scenarios, identity with `wte_r67.exe`

`tools\run_all.sh` with `tools\jobs_all.txt` (round 6/7/8 gates' model choices: Avocado for `open_path`, `fuzz_path`,
`stamp_edge_path`, `text_preview_path`; DamagedHelmet for `workflow_path`; tiny_cube.obj for `import_save`;
ABeautifulGame for `brush_types_path`, because `tex_4096.obj` does not exist on this machine). One process at a time.
`wte_r9.exe` 39 / 39 ok (`logs\scen_r9.txt`), `wte_r67.exe` 39 / 39 ok (`logs\scen_r67.txt`).

`tools\shot_cmp.py runs_r67 runs_r9` (`logs\shot_cmp_all.txt`): every top-level PNG pixel-compared, every other file
(exports, assets) byte-compared:
- 36 scenarios all files identical, `shot.png` included; among them every brush scenario (`paint3d`, `paint2d`,
  `brush_types`, `brush_shape`'s `shot.png`, `brush_tab`, `brush_types_path`, `brush_hover`, `pan2d`, `fuzz_path`) and
  the Project ones (`stamp`, `text_*`, `roundtrip`, `export_run`, `workflow_path` with its 8 export files and assets,
  `final_bake_async` with both export folders). This time `final_bake_async` and `workflow_path` had no toast difference.
- `export` `shot.png`: 870 px in x 313-445, y 153-164 = the export dialog's folder text (`..._r67/export/...` vs
  `..._r9/...`, crop `logs\export_folder.png`): known exception.
- `brush_shape` `shot_dialog.png` (a named extra shot, not `shot.png`): 11,934 px in x 417-976, y 333-663 = the open-shape
  dialog's file path text (`runs_r67\...` vs `runs_r9\...`, shifted line) and a toast's fade state (crop
  `logs\brush_shape_dialog.png`). Same kind as the folder-text exception.
- `wrap_surface`: 6 shots differ (expected: the wrap changed).
- `export_run`: the 7 exported files byte-identical; `check_exports.py` on both folders **PASS** (`logs\check_exports_r67.txt`,
  `_r9.txt`).
- `wrap_surface` step 1 Project composite `ws_h0` = `cd272dc06ef4e935` in both exes.
- `WTE_SYNC_FINAL_BAKES=1` (`runs_sync\`): `wrap_surface` ok, the ordered list of logged composites equal to the
  asynchronous run (md5 of the list equal) and all 6 shots identical; `final_bake_async` ok, its 12 scratch hashes equal
  in r67, r9 async and r9 sync, both export folders identical, `shot.png` differs only in the toast box (10,739 px, x
  686-977, y 660-702; 21 "applied after" lines async, 0 sync).

## 3. Format and compatibility

- `FORMAT_VERSION` = 3 (`crates/wte_core/src/project/mod.rs:22`). Manifest key paths of a round-9 save
  (`runs_r9\wrap_surface\shot_wrap.wtep`) = those of the r67 and the r8 save (recursive key set: no key added or
  missing). `workflow_path` saves have `format_version` 3 in both exes.
- Inputs (`compat\in\`): `sphere_r67.wtep` (r67 `wrap_surface` `shot_wrap.wtep`), `sphere_r8.wtep` (r8 `wrap_surface`),
  `wb_r67.wtep` and `wb_r8.wtep` (`workflow_path` on WaterBottle.glb with `WTE_STAMP_MODE=wrap`); each has 2 stamps in
  `"wrap"` mode. Each opened with `open_path` in r9, r67 and r8: all 12 runs ok; r9's `shot.png` identical (0 px) to
  r67's and r8's for all four; r9 logs have no `wrap:` / `wrap phases` / `wrap fit` line (saved pixels kept).
- Tint check (plan 7.2, WaterBottle row): an out-of-tree probe (`compat_probe\`, a separate cargo package with a path
  dependency on `crates/wte_core`, built into `probe_target\`; no product file touched) loads a project, changes each
  stamp's tint (Final), settles, sets the tint back (Final), settles and compares the re-baked layer cache with the
  cache as loaded, over texels with alpha > 0 in either. "Body" = texels whose surface normal is perpendicular to the
  bottle's axis (|n . Y| < 0.05). Self-check (`--rebase`: one cycle first, then compare a second cycle): 100.00 %, max 0.
  Results (`logs\compat_tint.txt`, images `compat\dump\*.png`: loaded | re-baked | green = within 2/255, red = not):

  | Project | Stamp | all texels within 2/255 | body (cylinder) | rest (shoulder etc.) |
  |---|---|---|---|---|
  | wb_r67 (round 6+7) | logo `wf_stamp.png` | 71.79 % of 39,362 (max 255) | **68.14 % of 9,187** (max 255) | 72.90 % of 30,175 |
  | wb_r67 | text "WTE" | 50.61 % of 8,101 | 53.26 % of 5,822 | 43.84 % of 2,279 |
  | sphere_r67 | "cells" / "WRAP AROUND" | 84.09 % / 69.75 % | - | - |
  | wb_r8 (round 8) | logo / "WTE" | 83.34 % / 61.80 % | 87.69 % / 59.71 % | 82.02 % / 68.16 % |

  The diff image of the logo shows the differences along every colour edge (the picture moved by about a texel all
  over), not only near the shoulder. Most of the logo lies on the shoulder (|n . Y| 0.1-0.7 for 30,175 of its texels).
  Limit "label texels on the cylinder body 99.9 % within 2/255": **not met (68.14 %)**.

## 4. Sweep (7.2)

`tools\sweep.sh` (wrap_surface, PATH, 1280 x 800, `WTE_FRAME_LOG=1`), one at a time, `sweep_r9\<model>\`. Parsed with
`tools\sweep_parse.py` (`logs\sweep_r9_table.txt`). Every quality number (ink, dup, scatter, covered, ends, lost_cut,
seams, rejected, verts, tris, flipped, jumps) equals check:B's final sweep (`..._r9b\target\round9\check_b\s_final`) on
all 26 lines, and all 7 shots per model are pixel-identical to it except ngons_tiling's `shot.png` (6,937 px in the toast
area x 787-977, y 697-738).

Exit codes: 9 ok; **4 assert-failed**, all on the new step-10 assert "covered >= 0.85 x ends_share unless reached_edge,
lost_cut <= 10 %" except one:
- Lantern: lost_cut 11.23 % > 10 % (both settings).
- ABeautifulGame: Jump off covered 0.0714 < 0.85 x 0.3314 (the picture is larger than the pawn it lands on).
- many_islands: off 0.0177 < 0.85 x 0.1843; on 0.0333 < 0.85 x 0.3318 (cube islands far smaller than the picture).
- high_poly_sphere, run 1: failed at step **6b** ("the drag's wrapped Previews ran as jobs and landed": 1 job started,
  0 landed; the one Preview job on the built-in sphere took 397 ms of worker time instead of ~20 ms, so the drag's other
  moves never started a job). Kept as `sweep_r9\high_poly_sphere_run1_6bfail\`. Run 2 (and the two timing runs, and the
  2,048 run) passed 6b with 5 / 4 jobs started / landed. A timing-dependent assert that flaked once under the game's
  load; not seen in check:B's runs.
The ink / dup / scatter assert (ink <= 1.3, dup <= 3 %, scatter <= 5 %) passed on all 13 models with both settings.

high_poly_sphere with a 2,048 x 2,048 checker (`hps2048_model\`: the OBJ, MTL and the 256 px checker scaled by nearest
neighbour; `sweep_r9_2048\`): ok; 77,721 verts / 156,680 tris (same as 256 px), seams 0, rejected 0.00 %, ink 1.1024
(256 px: 1.0977), dup 0, scatter 0, stretch 33.8 % (same).

Screenshot sheets: `sheets\<model>_view.png` (viewport, r6+7 | r8 | r9, half size), `sheets\<model>_3way_zoom.png`
(3-4 x), single 4 x crops `sheets\<model>_r9_zoom.png`, `_r8_zoom.png`, `_r67_zoom.png`, `_r9off_zoom.png` (Jump off),
and closer crops `Avocado_r9_topleft.png` (10 x), `ABeautifulGame_r9_board.png` (7 x), `Lantern_r9_wedge.png` (12 x),
`high_poly_sphere_2048_zoom.png`, `panel_lines.png`. Reference shots copied to `ref_shots\r67|r8\<model>\`. Readings are
in the report.

## 5. Timing (7.3), `timing1\`

Driver `tools\timing.sh` (one process at a time; `tools\run_peak.ps1` polls peak working set and private bytes every
250 ms; `driver.log`). Order: wrap_surface high_poly_sphere and ABeautifulGame, r8 then r9, two rounds; WA15 probe r8,
r9, r8, r9; ABeautifulGame `workflow_path` r9 W, r9 P, r8 W, r8 P, r9 P, r9 W, r8 P, r8 W; sphere sample `stamp` r9 W,
r9 P, r8 W, r9 P, r9 W, r8 W; fuzz seed 7, 1,000 steps, ngons_tiling, r9 W, twice. `WTE_FRAME_LOG=1` everywhere.

WA15 probe: `wrap_tests::wrap_bench_models` in release. r9: `cargo test --release -p wte_core --lib --no-run` in this
worktree (exe copied to `bin\wa15_r9.exe`). r8: `git archive round8-wip` extracted to `r8src\` (rev
`f907612454d6c375ba7f4283e12fd14a572b315f`), the same command there with its own target dir (`bin\wa15_r8.exe`). Run as
`<exe> wrap_tests::wrap_bench_models --exact --ignored --nocapture --test-threads 1` with
`WTE_GLB_SAMPLES=G:/software_projects/texture_wrapper_ap/samples` (ngons_tiling is skipped by the probe: its
`target/compat/harness/shapes` folder does not exist here). Outputs `wa15_r8_1.txt` .. `wa15_r9_2.txt`, table
`wa15_table.txt`.

Workflow analysis `tools\wf_analyze.py` (`wf_analysis.txt`): per Final its "applied after" and worker time and any frame
> 300 ms between its start and its application; drag frames = frames from the step's first `Pointer` step to the
Final's start (the first frame of a step is the idle gap after the previous step: 205-229 ms in every exe and mode, not
a drag frame).

All numbers are in the report's 7.3 table.

## 6. Findings beyond the plan's lines

1. ABeautifulGame Jump on: `passed` with ends_share 0.588: the spine round the pawn under the anchor passes itself, so
   the whole picture is cut to 59 % of its length although the board has room; the board shows only columns 4-8 and the
   panel says "Longer than the way round: 59 % shown" (round 8: most of the picture on the board).
2. nonmanifold: ink 0.629 (r6, r8) -> 0.187, rejected 70 %: the page facing the viewer is bare where round 8 showed the
   picture's middle column; the visible picture is in two pieces with a bare page between.
3. Lantern: two thin bare slivers inside the front and right face cells (cut wedges), besides the bare plinth and cap.
4. torus_knot panel: both "56 % of the picture runs off the surface: an edge, a hole or the end of the part" and
   "Longer than the way round: 43 % shown"; the first line names a cause that does not apply.
5. The high_poly_sphere step-6b flake (section 4).
6. Small models' parameterizations are slower than round 8 in the sweep logs (under load): mobius 111-134 vs 12-13 ms,
   nonmanifold 136-238 vs 7 ms, klein_bottle 396-493 vs 212-225 ms, ngons_tiling 11-12 vs 3-4 ms.
7. CRLF conversion of three existing core files (section 0).
