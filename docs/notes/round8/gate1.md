# Round 8 gate, stage 1 (plan section 7.1 items 1-4)

Date 2026-11-22, 04:05-04:45 local. Gate folder `target\round8\runs_gate\` (jobs files, build logs, runs, comparison
outputs, crops, tools). Work copy `target\round8\work\merged`. Exe `target\round8\wte_round8.exe`.
Nothing else of ours ran during the builds or runs; no unrelated cargo / rustc / Python process was seen at any lock
take or around the sweep (`lock_events.txt`, `sweep_g8_env.txt`). Free memory 0.9-1.6 GB throughout.

## Summary

| Item | Result |
|---|---|
| 1 merge, tests, clippy, release | merged = pkg_b (byte-identical sources). Tests **1,147 passed, 4 failed**, 33 ignored (base 1,116 / 0 / 33). Clippy 0 warnings. No `todo!` / `unimplemented!`. Exe SHA-256 `19D7844B4BDCD73ECE7327FB3F677CD58695A8314EA80F42CB194CDA97E54B2A` (= check:B's exe). The 4 failures are package A's unmet acceptance limits; **not fixed here** (below) |
| 2 scenarios, identity | 39 / 39 ok at 1280 x 800. 35 shots identical to `wte_round6_7.exe`; the rest are the known exceptions (toast area in `final_bake_async` and `workflow_path`, the export dialog's folder text in `export`) and `wrap_surface` (expected). Exports byte-identical, `check_exports.py` PASS. Sync = async for `wrap_surface` and `final_bake_async` |
| 3 sweep (G6 quality) | **Not met**: 5 of 13 models fail the plan's limits (Avocado Jump on, Lantern off/on, ABeautifulGame Jump on, klein_bottle off/on, nonmanifold off/on). Screenshots: high_poly_sphere, WaterBottle one whole picture; klein_bottle one piece on the visible side; Avocado, DamagedHelmet, ABeautifulGame, torus_knot not "one whole picture" (details below) |
| 4 compatibility | Round-6 projects open with identical shots (met). After a tint change the WaterBottle stamp is **not** within 2/255 on 99.9 % (71.6 %): the stamp lies partly on the bottle's curved shoulder, where round 8 relaxes. Sphere differs as expected (91.2 %, borders shifted by about one texel) |

## 1. Merge, three-way diff, tests, clippy, release

`merged` = robocopy of `work\pkg_b` including `target\` (minus `target\tmp`, `test_tmp`, `wte-test-tmp`): 10,369
files, 10.99 GB, 0 failed, 11 min 42 s (`robocopy_merged.log`; robocopy exit 1 = "files copied"). `tools\threeway.py
pkg_b merged`: no difference in `crates\` (without `target`), `ARCHITECTURE.md`, `Cargo.toml`, `Cargo.lock`.
`base` equals the main tree's `crates\`, `ARCHITECTURE.md`, `Cargo.*` (checked with `diff -rq` / `cmp`).

Three-way check (`threeway_pkg_b.txt`, base -> pkg_a -> pkg_b):

| Changed | Files |
|---|---|
| base -> A (core only) | `doc_stamps.rs`, `document.rs`, `lib.rs`, `samples.rs`, `stamp_job.rs`, `wrap_bake.rs`, `wrap_param.rs`, new `wrap_relax.rs`, `wrap_tests.rs`; `ARCHITECTURE.md` 1.6b, 2.1, 5.12, 5.14, 6.5a, 9.1. No app file. Do-not-edit files (`brush.rs`, `stroke.rs`, `texel_map.rs`, `bvh.rs`, `tip_bake.rs`, `stamp_bake.rs`) unchanged |
| A -> B, app (B's) | `app.rs`, `io_flows.rs`, `jobs.rs`, `panels\tool_options.rs`, `selftest\mod.rs`, `selftest\scenarios.rs`, `stamp_bakes.rs`, `state.rs`, `tools\transform.rs` |
| A -> B, core (applied requests, check:B) | `doc_stamps.rs` `apply_wrap_preview` only (request 1: a Stale Preview output caches its parameterization when it serves; the "no longer wrapped" fix), `wrap_param.rs` only `pub mod param_counter` + `param_counter::bump()` in `wrap_param` (request 2). Diff read: exactly what notes\pkg_b.md "Check and fix" items 1-3 say |
| A -> B, `ARCHITECTURE.md` | 6.5a (check:B's core change; 5.20's text lives there), 8.2, 8.6, 8.7, 8.9, 8.11, 9.2 |
| Cargo.toml / Cargo.lock | unchanged in A and B: no new crate |

No `todo!` / `unimplemented!` in `merged\crates` (0 hits).

Builds, each under `target\BUILD_LOCK` (label `round8-gate1`, never found held; `tools\locked_cargo.ps1`):

| Command (in merged) | Result | Log (`build_logs\`) |
|---|---|---|
| `cargo test --workspace --no-fail-fast` 04:18-04:21 (nothing recompiled: the copied fingerprints were current) | wte_app 357 passed / 16 ignored; wte_core lib **743 passed, 4 failed** / 17 ignored; brush_tips 15; end_to_end 9; project_roundtrip 23. **Total 1,147 passed, 4 failed, 33 ignored.** Base (round 6+7, `round7\verify\test.log`): 342 + 727 + 15 + 9 + 23 = 1,116 passed, 33 ignored. Count >= base: yes; 0 failed: **no** | `test1.log` |
| `cargo clippy --workspace --all-targets` | 0 warnings, 0 errors | `clippy1.log` |
| `cargo build --release -p wte_app` | ok (26 s, nothing rebuilt) | `release1.log` |

The 4 failing tests (messages identical to check:A / check:B):
- `wa4_r8_bumpy_plates...`: WA4 2 periods at 60 deg: ink 1.1360 (limit 0.90-1.10), cells 0.785-1.599 (limit 0.75-1.30);
  smear 0, covered 1.0, dup 0, scatter 0, seams 0 (those met).
- `wa20_a_bumpy_ball...`: ink 1.637 (limit 0.85-1.15) (check:A: cells 0.754-3.302, dup 10.7 %, scatter 4.08 %, 45 flipped).
- `wa22_a_torus_is_covered_once`: picture larger than the torus: dup 2.27 % (limit 1 %).
- `wa24_jump_small_gaps_is_bounded_by_the_footprint`: WA17 ink 1.358 (limit 0.95-1.10), dup 14.8 % (limit 1 %).

**Not fixed in merged, on purpose.** These are not broken tests but product quality limits the plan sets (sections 3.5,
8.1). The builder and check:A tried solver settings without success (notes\pkg_a.md, check:A section 4); WA24 needs a
visibility / tie-break rule for the owner test, which check:A identified as a planner decision. Weakening the asserts is
forbidden (plan 8, 10), and an algorithm change is not a "small and contained" gate fix (plan 7.2). They stay failing
with the plan's limits and go to the report as not met.

Exe: `target\round8\wte_round8.exe` (copy of `work\merged\target\release\wrapped_texture_editor.exe`, 26,395,648 bytes),
**SHA-256 19D7844B4BDCD73ECE7327FB3F677CD58695A8314EA80F42CB194CDA97E54B2A**, equal to check:B's pkg_b exe.

## 2. Scenarios at 1280 x 800, identity against `wte_round6_7.exe`

Reference: all 39 scenarios run now with `target\round7\wte_round6_7.exe` (`runs_ref\`, `ref_1280.csv`,
`sweep_ref.out`: 39 / 39 ok). Round 8: `sweep.ps1 -Jobs jobs_all.txt -Csv all_1280.csv -Runs runs -Parallel 1`
(round 7's 38 jobs + `wrap_surface|-`; models as round 7) -> **39 / 39 ok** (`sweep_all.out`). One exe at a time.
Comparison `tools\shot_cmp.py runs_ref runs` -> `shot_cmp_all.txt`:

| Scenario | shot.png vs wte_round6_7.exe |
|---|---|
| 35 scenarios, among them every brush scenario (brush_types, brush_shape, brush_tab, brush_types_path, brush_hover, paint3d, paint2d, pan2d, fuzz_path) and the Project ones `stamp`, `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run` | identical (0 px) |
| `final_bake_async` | 17,445 px in x 686-977, y 623-702: the toast stack only (round 7's known exception) |
| `workflow_path` (DamagedHelmet) | 9,313 px in x 735-977, y 697-739: toast area only (known exception) |
| `export` | 1,196 px in x 303-490, y 153-164: the export dialog's folder text ("...runs\export_builtin..." vs "..._ref\export_builtin...", crops `crops\export_runs*.png`; known exception) |
| `wrap_surface` | 3,517 px in x 375-1107, y 227-608: the wrapped stamps (expected; same numbers as packages A and B) |

Files: `export_run` `shot_export` 7 of 7 files byte-identical; `check_exports.py` on both folders **PASS** (sphere,
4 layers; `check_exports.txt`). `final_bake_async` both export folders 4 / 4 byte-identical; `workflow_path`
`shot_export` 8 / 8 and `shot_assets` identical. Saved projects: `roundtrip`, `final_bake_async`, `workflow_path`
`.wtep` differ only in `manifest.json` `saved_unix`; `wrap_surface`'s `shot_wrap.wtep` differs in the wrapped stamps'
caches, the flattened image, the thumbnail and the manifest's `sets` (expected).
`final_bake_async`: the sorted set of all logged 16-hex hashes is the same in the reference run, the async run and the
sync run (md5 of the list `ae610e6d...`).

`wrap_surface` step 1 Project composite H0 = `cd272dc06ef4e935` in both exes (met).

`WTE_SYNC_FINAL_BAKES=1` (`jobs_sync.txt`, `sync_1280.csv`, `runs_sync\`): `final_bake_async` ok, `wrap_surface` ok;
logs say "final stamp bakes: on the UI thread".
- `wrap_surface` async vs sync: all 14 logged composites equal in order (H0 cd272dc0..., H1 f3e5a387..., H2
  a84b65e3..., Overlap e4947f9d..., front f3e5a387..., H8 / saved df0ab0b7...); 6b composite after the Final
  424bc2ed5b3c53eb in both; `shot.png` and the five named shots 0 px apart. 2b (both): relaxed, seams 0, stretch 4.2 %,
  dup 0.000 %, scatter 0.000 %, 10,952 bins. 6b async: 7 Preview jobs started, 7 applied, 0 dropped, 0 UI-thread
  parameterizations; sync: 0 jobs.
- `final_bake_async` async vs sync: hash lists equal, both export folders byte-identical, `shot.png` 6,614 px in the
  toast box (x 794-977, y 623-663) only; 21 "applied after" lines async, 0 sync.

## 3. Sweep: `wrap_surface` with PATH on the 13 models (Jump off and on), `wte_round8.exe`

`jobs_sweep.txt`, `sweep_1280.csv`, `sweep_models.out`, runs `sweep\`, table `sweep_table.md` (`tools\sweep_table.py`).
04:27:51-04:30:39, one exe at a time, nothing else running. 8 ok, **5 `assert-failed` on step 10's plan limits** (no
panic, no other error). Numbers are step 10's log lines; verts / tris are the laid vertices / triangles; "applied" is
the Final's applied time (indicative).

| model | Jump | ok | verts / tris | relaxed | seams [S1..S5] | stretch | rejected | covered | ink | dup | scatter | jumps | flipped | param ms | applied ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| WaterBottle | off | yes | 225 / 482 | yes | 0 | 2.1 % | 0 | 1.000 | 1.0017 | 0.000 % | 0.000 % | 0 | 0 | 20.7 | 48 |
| WaterBottle | on | yes | 225 / 482 | yes | 0 | 2.1 % | 0 | 1.000 | 1.0017 | 0.000 % | 0.000 % | 0 | 0 | 22.1 | 36 |
| Avocado | off | no | 79 / 136 | yes | 0 | 12.7 % | 0 | 0.532 | 0.5288 | 0.000 % | 0.000 % | 0 | 0 | 10.3 | 47 |
| Avocado | on | **no** | 115 / 235 | yes | 0 | 12.6 % | 0 | 1.000 | **1.5295** | **52.591 %** | **5.865 %** | 1 | 0 | 21.6 | 45 |
| DamagedHelmet | off | yes | 233 / 457 | yes | 16 [0,0,5,4,7] | 16.9 % | 0 | 0.655 | 0.5065 | 0.000 % | 0.000 % | 0 | 0 | 25.4 | 81 |
| DamagedHelmet | on | yes | 1,060 / 1,956 | yes | 140 [0,0,5,28,107] | 43.6 % | 1.18 % | 1.000 | 1.0941 | 0.000 % | 0.000 % | 9 | 1 | 56.3 | 84 |
| Lantern | off | **no** | 52 / 100 | yes | 28 [0,0,0,19,9] | 220.2 % | 0 | 1.000 | 0.8273 | **26.860 %** | **10.578 %** | 0 | 11 | 14.0 | 63 |
| Lantern | on | **no** | 52 / 100 | yes | 28 [0,0,0,19,9] | 220.2 % | 0 | 1.000 | 0.8273 | **26.860 %** | **10.578 %** | 0 | 11 | 14.9 | 29 |
| ABeautifulGame | off | no | 14,210 / 28,416 | yes | 213 [213,0,0,0,0] | 130.3 % | 1.71 % | 0.053 | 0.0591 | 0.000 % (0 bins) | 0.000 % | 0 | 2,133 | 3,262 | 3,485 |
| ABeautifulGame | on | **no** | 160,593 / 323,172 | yes | 2,235 [0,823,52,7,1353] | 97.2 % | 26.34 % | 1.000 | **1.3045** | **10.505 %** | 1.130 % | 30 | 49,438 | 12,032 | 13,948 |
| ngons_tiling | off / on | yes | 11 / 9 | no | 0 | 0 | 0 | 0.074 | 0.0061 | 0 | 0 | 0 | 0 | 3.3 / 3.8 | 30 / 32 |
| torus_knot | off / on | yes | 1,272 / 2,599 | yes | 84 [76,0,0,0,8] | 18.3 % | 0 | 0.404 | 0.4051 | 0 | 0 | 0 | 0 | 79.1 / 84.3 | 97 / 114 |
| mobius | off / on | yes | 232 / 434 | yes | 0 | 1.1 % | 0 | 0.571 | 0.5706 | 0 | 0 | 0 | 0 | 13.7 / 15.7 | 31 / 49 |
| klein_bottle | off / on | **no** | 2,634 / 5,621 | yes | 0 | 46.8 % | 5.27 % | 1.000 | 1.1050 | **4.132 %** | 0.053 % | 0 | 2 | 203.8 / 204.8 | 230 / 232 |
| nonmanifold | off / on | **no** | 16 / 14 | no | 0 | 0 | 0 | 0.630 | 0.6292 | **84.186 %** | 1.672 % | 0 | 0 | 8.0 / 7.6 | 32 / 31 |
| many_islands | off | yes | 8 / 12 | yes | 8 [2,0,0,6,0] | 100.0 % | 0 | 0.018 | 0.0148 | 0 | 0 | 0 | 1 | 9.0 | 31 |
| many_islands | on | yes | 24 / 36 | yes | 21 [0,0,0,19,2] | 41.4 % | 0 | 0.034 | 0.0345 | 0 | 0 | 2 | 0 | 11.1 | 31 |
| flat_annulus | off / on | yes | 22 / 27 | no | 0 | 0 | 0 | 0.600 | 0.5972 | 0 | 0 | 0 | 0 | 7.6 / 10.0 | 31 / 32 |
| high_poly_sphere | off | yes | 122,496 / 246,559 | yes | 176 [0,49,0,0,127] | 34.9 % | 6.40 % | 1.000 | 1.0427 | 0.000 % | 0.103 % | 0 | 0 | 11,089 | 11,175 |
| high_poly_sphere | on | yes | same | yes | same | 34.9 % | 6.40 % | 1.000 | 1.0427 | 0.000 % | 0.103 % | 0 | 0 | 11,451 | 11,546 |

Limits (ink <= 1.3, dup <= 3 %, scatter <= 5 % on every model, Jump off and on): **not met** on Avocado (on), Lantern
(off, on), ABeautifulGame (on), klein_bottle (off, on), nonmanifold (off, on); met on the other 8. ABeautifulGame Jump
off passes only vacuously (the spot is on a piece's top: 5.3 % of the picture has surface, 0 inked bins at alpha >= 128;
213 S1 seams, stretch 130 %, 2,133 flipped). G6 (gate 2, reported here): high_poly_sphere Final applied 11.2 / 11.5 s
(limit 6 s); ABeautifulGame Jump on 13.9 s.

Round 6's numbers (`target\round6\runs_gate2\g6`, `r6_table.md`), ink with Jump off / on: WaterBottle 0.999 / 0.999;
Avocado 0.518 / 1.938; DamagedHelmet 1.019 / 5.563; Lantern 1.183 / 1.183; ABeautifulGame 0.064 / 5.236;
ngons_tiling 0.006 / 0.006; torus_knot 1.868 / 2.355; mobius 0.573 / 0.573; klein_bottle 9.109 / 9.862; nonmanifold
0.629 / 0.629; many_islands 0.015 / 0.035; flat_annulus 0.597 / 0.597; high_poly_sphere 6.626 / 8.051 (round 6 had no
dup / scatter measure). Round 8 lowers the ink on every curved model that failed in round 6.

### Screenshots (round 6 next to round 8; the shot is the state after Jump on)

| model | round 6 shot | round 8 shot |
|---|---|---|
| high_poly_sphere | `target\round6\runs_gate2\g6\wrap_surface_shapes_high_poly_sphere.obj_g6\shot.png` | `target\round8\runs_gate\sweep\wrap_surface_shapes_high_poly_sphere.obj_r8\shot.png` |
| klein_bottle | `...\g6\wrap_surface_shapes_klein_bottle.obj_g6\shot.png` | `...\sweep\wrap_surface_shapes_klein_bottle.obj_r8\shot.png` |
| Avocado | `...\g6\wrap_surface_glb_samples_Avocado.glb_g6\shot.png` | `...\sweep\wrap_surface_glb_samples_Avocado.glb_r8\shot.png` |
| DamagedHelmet | `...\g6\wrap_surface_glb_samples_DamagedHelmet.glb_g6\shot.png` | `...\sweep\wrap_surface_glb_samples_DamagedHelmet.glb_r8\shot.png` |
| ABeautifulGame | `...\g6\wrap_surface_glb_samples_ABeautifulGame.glb_g6\shot.png` | `...\sweep\wrap_surface_glb_samples_ABeautifulGame.glb_r8\shot.png` |
| torus_knot | `...\g6\wrap_surface_shapes_torus_knot.obj_g6\shot.png` | `...\sweep\wrap_surface_shapes_torus_knot.obj_r8\shot.png` |
| WaterBottle | `...\g6\wrap_surface_glb_samples_WaterBottle.glb_g6\shot.png` | `...\sweep\wrap_surface_glb_samples_WaterBottle.glb_r8\shot.png` |
| Lantern, nonmanifold, others | `...\g6\wrap_surface_<folder>_<model>_g6\shot.png` | `...\sweep\wrap_surface_<folder>_<model>_r8\shot.png` |

Enlarged crops of the round-8 shots: `runs_gate\crops\*_zoom.png`.

Readings:
- **high_poly_sphere**. Round 6: the whole ball covered with torn colour scraps, the picture appears many times, the
  outline a zig-zag of lines. Round 8: **one whole picture**. The 8 x 4 cells each appear once, the rows are continuous,
  and each cell bends over its bumps, so the cell edges wave. The outline is wavy, with no visible cut or scrap. The
  panel shows "Fitted to the curved surface: stretched up to 35 %" and "Cut where it cannot lie flat ...". The 176
  counted seam edges are not visible.
- **klein_bottle**. Round 6: picture pieces repeated all over the outer tube and the ring, torn. Round 8: one piece
  on the visible side. The left four columns lie on the outer tube with continuous rows, and the right columns curve
  round into the ring's hole, stretched. No cell repeats on the visible side. The outline has two jogs (lower left,
  right). The measured dup of 4.13 % (> 3 %) is not visible from this view, so the repeat must lie on the hidden part.
- **Avocado** (Jump on). Round 6: cells torn into strips that stick out beyond the fruit on the left, with repeats.
  Round 8: the visible picture is one piece. The left and right columns lie on the cut face, the middle columns over
  the seed's dome, and the rows continue across the seed's rim. The outline bends round the seed. Numerically it fails
  (ink 1.53, dup 52.6 %, scatter 5.9 %): the flesh hidden under the seed keeps its own copy of the seed's share
  (check:A open item 3, the owner test without a visibility rule).
- **DamagedHelmet** (Jump on). Round 6: the whole helmet covered with shredded colour triangles. Round 8: the visor
  carries most of the picture as large continuous cells. It is **not one whole picture**:
  - The left third of the visor shows a fold: the lilac cell is a thin wedge and the dark-red cell appears twice side
    by side.
  - The 9 jumps put scraps on the side parts: yellow, blue and magenta pieces on the right, a pink cell on the upper
    right part, blue and orange on the upper left knob.
  - The numbers pass (ink 1.094, dup 0, scatter 0), so the metric misses these scraps.
- **ABeautifulGame** (Jump on). Round 6: the picture smeared over the board and every piece in reach. Round 8: the
  board inside the outline shows the cells in continuous rows. But pawns, a bishop and the bases of pieces in and
  beside the footprint carry blobs of picture colours: purple, lime, magenta, cyan, orange. Some of these pieces stand
  right of the footprint. The outline is broken up. **Not one whole picture** (dup 10.5 %).
- **torus_knot**. Round 6: smeared colour along a long stretch of the knot. Round 8: the picture lies on one short
  tube section only, so the panel says "60 % of the picture runs off the surface". It is squeezed round the tube
  (76 S1 seams where the ends meet) and reads as a vertical band of the left columns' colours (black, purple,
  light blue, green, slate, violet) with blurred streaks. No repeated piece, but the picture is not recognisable as
  the 8 x 4 cells.
- **WaterBottle**. Round 6 and round 8 look alike: one whole 8 x 4 picture on the body above the holder, rows straight
  and continuous, columns narrowing evenly towards the sides, no cut, no repeat. Panel "Wrapped over 482 triangles"
  (stretch 2.1 %: no fitted line).
- **Lantern** (fails: dup 26.9 %, scatter 10.6 %). The picture lies on the post's base block. The front faces are
  continuous across the box edge. The sloped plinth and the top ledge carry extra patches: magenta on the ledge, as on
  the right face, and orange on the right of the plinth. The stretch is 220 %, with 19 S4 cone cuts on this 52-vertex
  block. Round 6 looks similar.
- **nonmanifold** (fails: dup 84.2 %). The "book" (several pages sharing one spine edge) carries the same cells on
  several pages (lilac and teal on two pages, olive and rose again), so the picture really appears twice there. The
  seed was kept (relaxed false), so S3's non-manifold cut does not apply on that path. The far bow-tie shows the
  pages' colours too, because the model's UVs overlap (50 UV triangles, 52 overlapping pairs, checked from the OBJ).
  That is a property of the texture layout, the same as round 6 (ink identical, 0.6292).

## 4. Compatibility

Inputs saved by `wte_round6_7.exe` (`runs_gate\compat\in\`):
- `sphere_r67.wtep`: the reference `wrap_surface` run's `shot_wrap.wtep`. Sphere sample with "cells" (Overlap + Jump
  small gaps) and the "WRAP AROUND" text, both wrapped.
- `waterbottle_r67.wtep`: a reference `workflow_path` run on WaterBottle.glb with `WTE_STAMP_MODE=wrap`
  (`compat\r67\`). The image stamp `wf_stamp.png` (the logo) and the text "WTE" are both `"mode": "wrap"`.

| Check | Result |
|---|---|
| Opens in round 8 with an identical composite | **met**. `open_path` in both exes (`compat\open_ref`, `compat\open_r8`), one at a time: both ok, `shot.png` identical (0 px) for both projects. No wrap or bake line in the round-8 logs, so the saved pixels are kept |
| After a tint change, the WaterBottle stamp within 2/255 on >= 99.9 % of its texels | **not met: 71.6 %** (logo) |
| The sphere stamp differs (expected); report how | differs: 91.2 % within 2/255 |

How the tint check was done: a gate probe (`runs_gate\tools\gate8_compat.rs`, run as an ignored integration test
in merged, then removed again; merged = pkg_b afterwards, checked). For each stamp it loads the project, changes the
tint and settles the Final bake (round 8 re-wraps), then sets the original tint back and bakes again. The re-baked
texels are compared with the round-6 cache as loaded, over the texels with alpha > 0 in either. With the changed
tint kept, all colour texels differ (as they should) and the alpha coverage is reported.

| Stamp | texels | within 2/255 | alpha within 2/255 | round-8 wrap | how it differs (`crops\compat_*.png`: round 6, round 8, red = differs) |
|---|---|---|---|---|---|
| WaterBottle logo `wf_stamp.png` | 39,299 | **71.57 %** | 72.30 % | relaxed, stretch 10.3 %, seams 0, dup 0 | The lower half (cylindrical body) matches. The upper ring and the band above the arrow moved by 1-3 texels, and the stamp covers 39,273 texels vs 37,900: the stamp's top lies on the bottle's shoulder, a curved surface, so the early-out does not hold and the relaxation spreads the picture slightly differently |
| WaterBottle text "WTE" | 8,763 | 50.30 % | 52.01 % | relaxed, stretch 65 %, dup 5.7 %, scatter 4.1 % | The letters shift by a few texels; round 8 adds small dark fragments on a separate UV island at the left |
| sphere "cells" | 29,727 | 91.17 % | 96.04 % | relaxed | The same picture in the same place. The differences are thin lines along the cell borders and the outer edges (shift of about one texel) |
| sphere "WRAP AROUND" | 6,162 | 78.77 % | 80.20 % | relaxed | The letters' edges shift by up to 69 levels |

Reading: the plan expected the WaterBottle stamp to be on a developable part (exact path). The workflow's logo spot
reaches the shoulder, so it is relaxed, and the 99.9 % / 2/255 limit is not met for this file. A stamp on the
cylindrical body alone was not available as a saved round-6 project: `wrap_surface`'s step 10 does not save, and
WaterBottle's body is relaxed there too (stretch 2.1 %). This is a behaviour change of round 8 that the report should
state: a round-6 WaterBottle stamp near the shoulder changes visibly (by a few texels) at its first edit.

## 5. Not met / open (for gate 2 and the report)

1. `cargo test --workspace` not green: 4 core acceptance tests fail with the plan's limits (WA4 60 deg, WA20, WA22,
   WA24). Not fixed in the gate (no weakening; needs design decisions). Count 1,147 >= base 1,116.
2. Sweep limits not met on 5 of 13 models: Avocado (Jump on: ink 1.53, dup 52.6 %, scatter 5.9 %); Lantern (dup
   26.9 %, scatter 10.6 %, both); ABeautifulGame (Jump on: ink 1.3045, dup 10.5 %); klein_bottle (dup 4.13 %, both);
   nonmanifold (dup 84.2 %, both: real repeats on the book's pages plus overlapping UVs).
3. Screenshot reading: one whole picture on high_poly_sphere, WaterBottle and klein_bottle's visible side. Not on
   DamagedHelmet (fold on the visor's left, scraps on 9 jumped parts; the numbers pass), ABeautifulGame (picture blobs
   on pieces), Avocado (visually one piece, numerically fails) or torus_knot (one piece squeezed into an unreadable
   band, 60 % off the surface).
4. Compatibility: the WaterBottle tint check is 71.6 % within 2/255 (limit 99.9 %), because of the stamp's relaxed
   upper part on the shoulder.
5. G6 (indicative): high_poly_sphere Final 11.2-11.5 s (limit 6 s); ABeautifulGame Jump on 13.9 s.
6. Not done here (gate 2): G5a-G5f, G7, G8, WA15 (not built by package A).
