# Round 9 gate — skeptic notes, lens: regressions and identity

Date 2026-11-23 (00:30-00:50 local). Worktree `G:\software_projects\texture_wrapper_ap_r9`, `round9-wip` @ a8beea6
(= 42996df + gate docs). Exes: `target\round9\gate\bin\wte_r9.exe` (SHA-256 4F9E1422...), `wte_r67.exe` (9F4F93CB...,
equal to the scratchpad copy), `wte_r8.exe` (DB61E168..., equal to the scratchpad copy). No cargo run, no tracked file
edited. All outputs in this folder. (An earlier, unfinished attempt of this skeptic left `id_*`, `det_t1`, `det_tN`,
`save_*`, `open_*`, `compat_in` here on 2026-11-22 19:03-19:10; I reused only `compat_in` (the saved projects, checked
byte-equal to their `save_*` sources) and re-ran everything else.)

## 1. Diff review

- `git diff --ignore-cr-at-eol --stat`: r8..r9 touches 15 crate files, main..r9 touches 20; **none** of brush.rs,
  stroke.rs, texel_map.rs, bvh.rs, tip_bake.rs, stamp_bake.rs, composite.rs, project/, export/, Cargo files, .cargo,
  *.cmd (checked against `main` too: empty diff). `FORMAT_VERSION` 3.
- Removed asserts / limits: none. Removed lines in tests (r8..r9): only the WA15 bench probe, `wa11`,
  `check_a_covered_follows_a_reused_scale`, `wa27` `half_for` helpers (texel term dropped with T6, comment says so) and
  the wall-clock measurement of `wa7` / `check_a_own_bad_meshes_never_fail`, now `best_param_ms` = best of up to 5 runs
  when the first is over 200 ms (limit unchanged). That is a **changed test setup** (more lenient measurement); it is
  documented in `check_a.md` (6) but the gate report does not mention it when it says the wall-clock tests "passed under
  the game's load".
- `uv_cube_on_floor()` now calls `uv_cube_on_floor_n(1)`: for n = 1 the floor corners and island are exactly round 8's
  (line(0)=0, line(1)=4.0; island `[u0,u1,v0,v1]` passed verbatim). WA17/WA24's input unchanged.
- scenarios.rs r8..r9: additions only (step 10 coverage / placement bits / panel lines / `shot_off.png`); no assert
  removed.
- Env knobs: all new `WTE_TEST_*` knobs go through `relax::knob`, which is `#[cfg(test)]`; the release `knob` returns
  the default. `use_exact` is `true` in release. `WTE_PROBE_*` only in an ignored test. No new env read in the app.
  No release-behaviour knob added.
- Ownership: package A (7c86ba4) edited `wte_app/src/panels/tool_options.rs` (3 lines, test struct literal) although
  the plan says "No app file (the app compiles unchanged)"; check:B (159c70e, 18023e9) rewrote parts of
  `wrap_relax.rs` (owner grid, parallel kernels, `PAR_MIN_UNKNOWNS` 16,384 -> 8,192), code the plan reserves to A
  ("B does not touch ... relaxation or owner code"). Behaviour-preserving per check:B's fingerprints and my thread runs
  (section 4). Not mentioned in the gate report.
- Line endings: confirmed `wrap_param.rs` 3,421/3,421, `wrap_relax.rs` 2,247/2,247, `wrap_tests.rs` 3,559/3,559 CRLF in
  r9, 0 CR in round8-wip; `uv_share.rs`, `wrap_mesh.rs` LF. No other file changed EOL r8..r9. Small error in gate.md:
  "LF in `main`" for `wrap_relax.rs` — it does not exist in `main`.

## 2. Identity with wte_r67.exe (`runs_r9`, `runs_r67`, `cmp_identity.txt`, `tools\cmp_all.py`)

14 scenarios (stamp, text_warp, text_path, text_styles, export_run, roundtrip, paint3d, paint2d, brush_types,
brush_shape, brush_hover, final_bake_async, stamp_edge_path + text_preview_path on Avocado): all ok in both.
My comparator checks every PNG at any depth pixel-wise and every other file byte-wise, including `.wtep` entry by entry
(the gate's `shot_cmp.py` skips `.wtep`).
- Identical: all shot.png except below; all export files (export_run 7, final_bake_async 8) byte-identical;
  `check_exports.py` PASS on both (`check_exports2_*.txt`).
- `.wtep` (roundtrip, final_bake_async): only `manifest.json` differs, only in `saved_unix`.
- final_bake_async shot.png: 10,508 px in the toast box (x 686-975, y 660-700, max 7) — the known exception.
- brush_shape shot.png: first run 12,896 px (x 626-977, y 623-664) = a toast's fade state (`crop_brush_shape.png`);
  two reruns each: 0 px. Timing flake of the toast, not content.
- brush_shape shot_dialog.png: with both exes writing to the **same path** (`samepath_*`), only 12,758 px in the toast
  area (max 13) differ; `export` shot.png 0 px at the same path. So the gate's "path text" exceptions are exactly that.

## 3. Compatibility (`open2_*`, `compat_runs2.txt`)

Six projects with two wrapped stamps each (`"mode": "wrap"` x2): Avocado, sphere, WaterBottle saved by r67 and by r8.
Opened with `open_path` in r9, r67, r8: 18/18 ok; r9's shot.png byte-identical to r67's and r8's for all six; r9 logs
have 0 lines matching `wrap:|wrap phases|wrap fit|param `. The shot is sensitive to the wrap content (r67-file vs
r8-file in r9: 3,260-4,389 px differ). Manifest key sets of r9 / r8 / r67 saves equal, zip entries equal, format 3.
Not verified: that a *non-wrap* edit (paint, another layer) after loading leaves the old wrapped pixels (no scenario
opens a .wtep and edits something else without re-wrapping); exports of a loaded old wrapped project.

## 4. Determinism (`det2_*`, `det2_cmp.txt`, `det2_step10_cmp.txt`, `det2_sync_cmp.txt`)

wrap_surface on DamagedHelmet, high_poly_sphere, ABeautifulGame, WaterBottle with `RAYON_NUM_THREADS` 1 / 3 / default,
plus the earlier attempt's 1 / default runs (5 runs per model for 3 models). Composite hashes (14, all from steps 1-9
on the built-in sphere), wrap stat lines (times masked; only "fresh"/"reused" labels move with job timing), step-10
lines (stats incl. verts/tris/cut_tris/covered/ink/dup/scatter/seams/placement bits; `micros` masked) and shot.png /
shot_off.png: **equal / identical** in all runs, except UI-only pixels: 2 px of a scrollbar fade (ABG t3 shot_off,
x 1270-1271 y 47) and the earlier attempt's ABG default run whose failure screenshot caught the "Unsaved changes"
dialog of the exit (harness timing after the expected step-10 assert failure). Saved `.wtep`: only `saved_unix` differs.

**Caveat found:** in the default (async) mode wrap Previews and Finals run in `stamp_job::bake_pool()`, a pool of
`available_parallelism - 1` = 7 threads that ignores `RAYON_NUM_THREADS`. So RAYON_NUM_THREADS alone does not change the
threads of the step-10 wraps (check:B's app-level "1 vs 12 threads" in async mode only varied the global pool). I ran
`WTE_SYNC_FINAL_BAKES=1 RAYON_NUM_THREADS=1` (Finals on the UI thread, 1-thread global pool; high_poly_sphere param
6,209 ms vs 4,599 ms in the 7-thread pool; ABG Jump on 160,593 verts, 9.6 s, fresh): step-10 lines and both shots
identical to the default async run on all four models. So 1 vs 7 threads gives the same result on large meshes where the
parallel kernels (>= 8,192 unknowns) and the parallel owner grid are active.

## 5. Flat / developable models vs round 8 and main (`flat_r9|r8|r67\<model>`)

| Model | r67 step 10 (off) | r8 | r9 | shot.png r9 vs r8 (>2 levels) | In the report? |
|---|---|---|---|---|---|
| flat_annulus | ink 0.5965, 16 v / 19 t | 0.5972, 22 / 27 | 0.5972, 22 / 27 | 0 px | yes (D7 item 9) |
| ngons_tiling | 0.0061, 11 / 9 | 0.0061 | 0.0061 | 36,064 px: new orange shared-texel panel line + toast | yes (item 8, 100 %) |
| tiny_cube | 0.9938, 6 v / 6 t, cut 0 | 0.9938, 6 / 9 (on 10), cut 0, relaxed true | 0.9938, **5 / 6 (on 8), cut_tris 1, relaxed false** | 399 px: panel "Wrapped over 10 triangles" -> "8"; viewport identical | **no** (tiny_cube not in the sweep) |
| mobius | 0.5731, 134 / 264 | 0.5706, 232 / 434 | 0.5702, 232 / 434, bins 1652 vs 1648 | 1,247 px: cell borders shifted sub-texel on the strip | numbers yes; "unchanged to the eye" holds |
| WaterBottle | 0.9992, 71 / 151 | 1.0017 | 1.0033 | 1,062 px: thin band along the label's top edge | yes (1,052 px in the gate's run) |

Steps 1-9 shots on the built-in sphere differ from r8 (shot_wrapped 5,205 px, shot_text 4,383, shot_before_save 2,171,
shot_band 304; shot_overlap 0): the wrap of the sample changed; the gate counts these as "expected" without a reading.

## 6. Project-mode change on shared-UV models (new)

`Document::stamp_shared_texels` serves **projected (Project-mode) stamps too**, and selecting any 3D stamp starts the
uv_share warm-up thread. On DamagedHelmet in Project mode (`proj_*`: stamp_edge_path, text_preview_path, no
`WTE_STAMP_MODE`) r9 logs "warm-up started (graph: false) ... shared texels found in 40 ms (86.7 % of the surface)", and
shot.png differs from r67's by 12 px / 8 px = the side panel's scrollbar at x 1270-1271 (the Transform options content got
longer: the new line). This is plan T9 ("laid (Wrap) or hit (Project) triangles"), but the gate's identity set uses only
the built-in sphere (0 % shared) and Avocado (none), so it never exercised it; the verdict "Project mode ... untouched"
and "every non-wrap scenario's shot.png identical" do not hold for Project-mode scenarios on shared-UV models (UI only;
baked pixels identical as far as the shots show). Item 8 of the report lists the line but not that it shows in Project mode.
