# Round 7 package P (brush speed): builder notes

Builder: round7-build (Opus), 2026-11-21. Copy: `target\round7\work\pkg` (robocopy of round 6's `base`, the round-5
final source). Plan: `target\round7_design\ROUND7_PLAN.md`.

## Setup

- `pkg` had no `target\`. Instead of a cold build, the dependency artifacts of the main tree's `target\release` and
  `target\debug` (`deps`, `build`, `.fingerprint`, without `wte_*` / `wrapped_texture_editor*` / test exes / pdb) were
  robocopied into `pkg\target` (same `Cargo.lock`, hash checked). Cargo reused them: the first release build compiled
  only `wte_core` and `wte_app`. (Copied only from the main tree, nothing from round 6's folders.)
- Source snapshots: `target\round7\step0_src\` (round-5 code + probe slots 10-14 = the step-0 build),
  `target\round7\a12_src\` (working copy kept while the step-0 sources were swapped in for the measuring build).

## Step 0: where the raster time is (round-5 code + probe slots 10-14)

Build: `step0_src` (round-5 rasterizer unchanged; `raster_tri_with` only reports its pass-2 cost; slots appended
after slot 9). Exe `target\round7\runs\step0\wte_step0.exe` (SHA-256 0E8EDCDE...7310). Machine: 6 cores, **12 logical
processors** (the global rayon pool has 12 threads, not 6 as plan section 4 says). No other build ran during the app
run (two idle `cargo.exe` of another agent were present; no `rustc`). Indicative, single run.

### App: `brush_types_path` on ABeautifulGame, `WTE_STROKE_LOG=1` (`runs\step0\btp_abg.log`, ok, 716 frames)

| stroke | stroke work ms | longest frame / dab ms | backlog ms | release to end ms | set dabs (probe) | touched | pass-2 texels | pass-2 / touched | candidates | cands / set dab | raster ms | band work ms | band work / raster | bands |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| warm-up Size 50 | 1,710 | 137 / 51 | 856 | 5 | 394 | 19.99 M | 308.2 M | **15.4** | 332,570 | 844 | 1,145 | 6,104 | 5.3 | 3,753 |
| Circular 50 | 1,488 | 134 / 47 | 920 | 3 | 394 | 19.99 M | 308.2 M | **15.4** | 332,570 | 844 | 1,016 | 5,368 | 5.3 | 3,753 |
| Sphere 50 | 1,883 | 132 / 66 | 1,460 | 5 | 404 | 25.67 M | 413.6 M | **16.1** | 454,112 | 1,124 | 1,325 | 7,594 | 5.7 | 4,083 |
| Sphere/Cube 50 | 1,375 | 146 / 50 | 922 | 4 | 370 | 11.57 M | 205.8 M | **17.8** | 223,977 | 605 | 1,028 | 5,020 | 4.9 | 3,441 |
| Circular 512 | 43,132 | 1,386 / 1,386 | 50,092 | 7,295 | 330 | 341.3 M | 8,872 M | **26.0** | 9.85 M | 29,842 | 37,156 | 354,554 | 9.5 | 9,584 |
| Sphere 512 | 67,837 | 2,013 / 2,013 | 80,436 | 10,698 | 330 | 557.3 M | 14,433 M | **25.9** | 16.03 M | 48,578 | 61,029 | 592,149 | 9.7 | 9,695 |
| Sphere/Cube 512 | 50,258 | 1,787 / 1,787 | 63,002 | 7,358 | 283 | 371.6 M | 8,005 M | **21.5** | 8.85 M | 31,264 | 43,503 | 422,452 | 9.7 | 8,196 |

Phase split of the Size-50 strokes (Circular): setup 65, raster 1,016, commit 184, re-apply 143, query 77 ms (raster
68 %); Size 512: raster 86-90 %, setup 4-5 %, query 5-7 %, commit 1.5 %.

Reading against plan 3.0's rules:
- **pass-2 texels / touched = 15-18 at Size 50, 21-26 at Size 512, far above 5: A1 is the main win (as expected).**
- band work / raster = 4.9-5.7 at Size 50 and 9.5-9.7 at Size 512 (12 threads), above 3. The rule "< 3 -> finer bands
  are a main win" is **not** met: round 5's bands already keep about 5 of 12 threads busy at Size 50. A2 is still
  built (plan: never cut), its expected gain is mostly the parallel commit/setup and the dropped per-band setups of
  clip-box binning, not the width. (Note: band work includes the empty pass-2 scans, so the band jobs themselves
  are mostly pass-2 waste; after A1 they shrink and the serial parts weigh more.)
- Texture sizes of the sets: as in round 5 (ABeautifulGame: 15 sets, 2048 x 2048 and 4096 x 4096 textures; gutter 8 / 16).
- A3 / A4 are decided after A1 + A2 (query 5-7 % of Size-512 stroke work here; re-apply 6-10 % at Size 50).

### Core bench: `stroke_bench_tips`, `WTE_BENCH_MODELS=b WTE_BENCH_PROBE=1`, step-0 release test exe

`runs\step0\bench_b.out` (rows joined by `runs\bench_rows.py` into `bench_b_rows.txt`), one thread of the test
harness, 317 s, no other build running. Per dab (median of 5 reps):

| diameter | tip | ms/dab | raster | touched/dab | pass-2 texels/dab | pass-2 / touched | bands/dab | band work / raster | cands/dab |
|---|---|---|---|---|---|---|---|---|---|
| 50 | Ball/SameSide | 0.584 | 0.464 | 2,905 | 63,753 | **21.9** | 2.2 | 1.6 | 60 |
| 50 | Ball/All | 0.597 | 0.479 | 2,905 | 63,753 | 21.9 | 2.2 | 1.6 | 60 |
| 512 | Ball/SameSide | 71.6 | 61.4 | 802,025 | 11.70 M | **14.6** | 26.0 | 9.8 | 11,706 |
| 512 | Ball/All | 114.8 | 102.1 | 1,235,995 | 18.52 M | 15.0 | 26.2 | 10.0 | 18,726 |

(The bench's 50-texel dabs are light: 60 candidates and 2.9 k texels; most still take round 5's band path, with 2.2
bands and an effective width of 1.6. The app's Size-50 dabs on the chess pieces are heavier: 844 candidates and
51 k texels per set dab.) C1 hashes identical (legacy = shared) at both sizes.

**Decision:** A1 first (pass 2 is 15-26 x the touched texels: by far the largest waste), then A2 as planned. A3 / A4
by their rules after A1 + A2.

## Implementation (A1 + A2, the seam, T1-T6)

Files changed (all inside round 7's ownership): `wte_core\src\texel_map.rs`, `brush.rs`, `stroke.rs`,
`wte_app\src\tools\brush.rs` (`phase_text` only), and the copy's `ARCHITECTURE.md` (4.6 bullet 3, 5.9 one paragraph after
the code block, 5.19 probe slots + prose, 6.3 "Parallel pattern" + new "Lean pass 2" paragraph, 6.4 commit paragraph + new
"Round 7: the fused dab pass" paragraph before "2D dab"). `bvh.rs` not touched (A3 not built, see below). No new crate, no
`unsafe`, no public API change (`raster_box`, `raster_tri_lean`, `raster_tri_covered_lean`, `prepare_brush_index` are
`pub(crate)`; `STROKE_PROBE_SLOTS` 10 -> 15 and `STROKE_PROBE_NAMES` got five names).

- **Probe** (step 0): slots 10 pass-2 texels, 11 pass-2 skipped, 12 bands, 13 band work, 14 candidates. `raster_tri_with`
  returns a `Pass2Cost` (texels looked at, skipped flag); band jobs return counts and times, the calling thread adds them.
- **A1** (`texel_map.rs`): `TexelMap::claimed` (slivers that claimed a texel, recorded in build step 2), `pass2_tris`
  (`OnceLock<Vec<u64>>` bitset: owners of gutter texels plus `claimed`, built with `par_chunks` + OR-reduce),
  `shared_blocks` (`OnceLock`, 8 x 8 blocks holding a `shared_gutter` texel), `prepare_brush_index()`,
  `pass2_may_report(tri, cols, rows)` (word-masked range test of the blocks). `raster_tri_with(.., lean, ..)` skips pass 2
  when the triangle's bit is 0 and no shared block meets the folded pass-2 box within `rows`. New `raster_box` = the
  clipped scan box exactly as `raster_tri_with` computes it (one shared `scan_box` function). Folded row loops
  (`Folding::All`, `Folding::Blocks`) now walk `folded_rows(..)` (at most two intervals, ascending) instead of every box row
  with a `continue`. Public `raster_tri` / `raster_tri_covered` pass `lean = false`: unchanged for stamps, wraps, fill.
- **A2** (`brush.rs`): `raster_commit` = plan 3.2 steps 1-6 (`PAR_DAB_WORK` = 16,384, `DAB_BAND_ROWS` = 32,
  `PAR_PREPARE_TRIS` = 4,096; CSR buffers `boxes` / `band_start` / `band_items` reused in `StrokeScratch`; tile merge by
  union per key). `dab_tip` (parallel `prepare_tri` from 4,096 triangles) and `dab_3d` both go through it. Round 5's
  `rasterize_tip` / `rasterize_dab` / `DabTri::extent` are now `#[cfg(test)]` references; `commit_dab` stays (serial path).
- **Seam**: `#[cfg(test)] thread_local! RASTER_R5` (read once per dab): `dab_tip` / `dab_3d` run round 5's code (serial
  prepare, `rasterize_tip` with full pass 2, `commit_dab`).
- **Deadlock found and fixed during the build**: first version built the brush index lazily inside `pass2_may_report`,
  i.e. inside band jobs on pool threads. `OnceLock::get_or_init` running a rayon reduction there deadlocks (a thread
  that waits in `join` for the init steals another band job, which waits on the same `OnceLock`). Symptom: the brush
  tests hung. Fix: `raster_commit` calls `map.prepare_brush_index()` on the calling thread before any band job (and
  `stroke_target` calls it on a set's first touch). After the fix the brush / texel_map / stroke tests run in 6 s.
- **Bench**: `stroke_bench_tips` adds a row "<tip> r5" after every shared-path tip (mode 3 = `RASTER_R5`), asserts r7 hash
  == r5 hash per row (and prints `P2: ... identical`); ABeautifulGame also gets diameter 200; phase columns extended to
  slots 10-14.
- `stroke.rs`: `stroke_target` calls `prepare_brush_index` ("first touch" slot); tests that switch the process-wide probe
  hold a test mutex `PROBE_TESTS` (the existing `stroke_counters` probe test and T5), so one cannot switch the probe off
  while another reads it.

Tests added (core, in-file):
- T1 `brush::tests::r7_dabs_equal_round5_on_random_dabs` (7 meshes x maps 1x1 / 3x5 / 64² g8 / 300x420 g16 / 1024² g2 and
  gutter 0; ball hard / soft, Cube Solid / Through; SameSide / All; radii 0.3 texel .. whole model; masks, rects as sets,
  empty scratch; asserts >= 10 dabs took the banded path).
- T2 `brush::tests::dab_raster_is_the_same_on_any_thread_count` (pools of 1, 2, 5: serial, banded, full-core folded,
  core-block folded dabs via `dab_3d` and a Cube `dab_tip`; plus a two-set document stroke's layer hashes).
- T3 `texel_map::tests::lean_pass2_visits_the_same_fragments` (every triangle of T1's meshes, 4 map sizes incl. gutter 0,
  6 clips (none, wider than the image, inverted, random wrapping), random / empty row ranges; plain, all-folded and
  checkerboard-folded forms; fragments and runs bit for bit; asserts > 1000 skipped and > 1000 scanned calls).
- T4 `brush::tests::fused_band_commit_equals_the_serial_commit` (5 meshes, heights 75 / 131 / 200 / 33 / 45, sparse / dense
  dabs, bands 16 / 32 / 64 forced, against the serial path and round 5's code).
- T5 `stroke::tests::a_stroke_paints_round5s_pixels` (two_material_model 256², cube path + a path over head and body,
  4 strokes: small / large ball, Cube 0.25 / 0.03; seam on / off x probe on / off; `doc_state` equal).
- T6 `texel_map::tests::prepare_brush_index_is_complete` (bits exact: set iff gutter owner or claimed; every shared block
  set; built again in a 1-thread pool gives the same bitsets).
- plus `texel_map::tests::folded_rows_are_the_rows_in_range` (the folded-row helper against the filtered loop).

`cargo test --workspace` (debug, first run with all of the above): **1,074 passed, 32 ignored, 0 failed**
(332 + 697 + 15 + 9 + 21; round 5: 1,067 / 32). Log `target\round7\test_ws1.log`. (That run held the build lock also
while the tests ran, about 2 minutes after the compile; later runs compile with `--no-run` under the lock and run
without it.)

## After A1 + A2 (v1 exe, `runs\v1\`): app run and the A3 / A4 decision

Indicative, single run, no build running (`runs\v1\btp_abg.log`, ok, 643 frames; v1 = A1 + A2, before A3 / A4).
`brush_types_path` on ABeautifulGame, `WTE_STROKE_LOG=1`, compared with the step-0 run of the same day (round-5 code):

| stroke | stroke work ms (step 0 -> v1) | ratio | longest frame ms | longest dab ms | backlog ms | release to end ms |
|---|---|---|---|---|---|---|
| warm-up 50 | 1,710 -> 956 | 0.56 | 137 -> 112 | 51 -> 42 | 856 -> 536 | 5 -> 0 |
| Circular 50 | 1,488 -> 751 | **0.50** | 134 -> 102 | 47 -> 24 | 920 -> 453 | 3 -> 0 |
| Sphere 50 | 1,883 -> 955 | 0.51 | 132 -> 115 | 66 -> 31 | 1,460 -> 677 | 5 -> 0 |
| Sphere/Cube 50 | 1,375 -> 817 | 0.59 | 146 -> 104 | 50 -> 27 | 922 -> 485 | 4 -> 0 |
| Circular 512 | 43,132 -> 13,551 | **0.31** | 1,386 -> 473 | 1,386 -> 473 | 50,092 -> 15,190 | 7,295 -> 2,198 |
| Sphere 512 | 67,837 -> 20,070 | 0.30 | 2,013 -> 617 | 2,013 -> 617 | 80,436 -> 23,243 | 10,698 -> 3,654 |
| Sphere/Cube 512 | 50,258 -> 18,009 | 0.36 | 1,787 -> 634 | 1,787 -> 634 | 63,002 -> 21,990 | 7,358 -> 3,075 |

Probe (v1): pass-2 texels per stroke 308 M -> 59 M at Size 50 (3.0 x touched instead of 15.4 x; 510 k pass-2 calls
skipped), 8,872 M -> 1,283 M at Size 512; band work / raster 6.7 (Size 50), 9.5 (Size 512).
Phase split v1, Circular 50: setup 84, raster 442, commit 10 (now inside the bands), re-apply 144, query 70, total
751 ms -> **re-apply 19 % (>= 15 %: A4)**, query 9 %. Circular 512: setup 1,389, raster 8,816, commit 34, re-apply
424, query 2,857 of 13,551 ms -> **query 21 % (>= 10 %: A3)**. Both rules fire, so A3 and A4 were built (below).

### Bench artefact found and fixed (not a pixel change)

The first v1 bench run on models a, b, c failed its new P2 assert on tex_8192 at 50 texels: "Cube" r7 a437fd4c... vs
"Cube r5" e01715314e.... Diagnosis (temporary ignored test, removed again; logs `runs\diag\diag2.out`, `diag3.out`):
the masks of r7 and r5 are identical after every one of the 200 samples, and the layer hash is the same for r7 and r5
in either order. The cause was the bench itself: the Cube stroke (default colour) over the warm-up stroke (same colour,
same path) changed no pixel, so it left no history entry and its `undo` removed the **warm-up** stroke; the next row
then painted on another base. Fix in `stroke_bench_tips`: the warm-up stroke is undone right after it (every hashed
row starts from the same layer). Round 5 never compared Cube rows, so it did not see this.

## A3 and A4 (both rules fired after A1 + A2)

- **A3** (`brush.rs` `shared_candidates`, `bvh.rs` `query_sphere_dist`): from 32,768 found triangles both passes run on
  the pool in fixed chunks of 8,192 (per-chunk per-set maxima, then their maximum; per-chunk `(set, tri)` lists appended
  in chunk order, so every list stays ascending). `map_of` (a closure of the caller, not `Sync`) is asked once per set
  into a `Vec` first. `query_sphere_dist` sorts with `par_sort_unstable_by_key` from 32,768 entries (unique keys: same
  order). The tree walk stays serial. Effect: shared query at Size 512 on ABeautifulGame 2.9 s -> 1.5-1.6 s per stroke
  (app, `runs\v2\btp_abg_b.log` vs `runs\v1\btp_abg_b.log`), bench b-512 3.45 -> 1.82 ms/dab (step-0 bench vs v2).
- **A4** (`stroke.rs` `reapply_rect`, `brush.rs` commits): every 3D commit (the band commits of the fused pass and both
  paths of `commit_dab`) records per row the span from the first to the last texel whose mask it raised
  (`StrokeScratch::row_spans`; a band resets its own rows, the serial commit resets the rows it set last time; a stale
  older span only means extra shading, never a different value). `reapply_rect` shades only that span of each row of a
  rect (serial and parallel forms); the other texels already hold `apply_brush(original, mask)`. A 2D dab clears
  `spans_valid` (its rects are re-applied whole). Test seam `FULL_REAPPLY` (thread-local) = round 5's whole-rect
  re-apply; T5 also compares A4 on / off, and the bench's "... r5" rows run with it on.
  **Rule "keep only if re-apply drops >= 30 % on the bench": met on most rows** (v2 bench, r7 row vs its r5 row, re-apply
  ms/dab): a-512 0.344 / 0.559 (-38 %), a-2048 4.38 / 6.90 (-37 %), c-512 0.332 / 0.535 (-38 %), c-2048 5.79 / 9.10
  (-36 %), b-50 0.028 / 0.052 (-46 %), Cube rows -35 to -45 %; less on b-200 (-23 %) and b-512 (-8 %). **Kept.** (In the
  app at Size 50 the re-apply moved little, 150 -> 138 ms in the alternating pair: the Size-50 dabs on the chess
  pieces still raise most texels of their rects.)

## v2 bench (A1 + A2 + A3 + A4; `runs\v2\bench_abc.out`, rows `bench_abc_rows.txt`; indicative, in-process A/B)

Every P2 line "r7 == r5" **identical** (models a, b, c; all tips; 50 / 200 / 512 / 2048 texels) and every C1 line
identical. ms/dab, r7 row / its "r5" row in the same process (r5 = `RASTER_R5` + `FULL_REAPPLY`):

| model diam | Ball/SameSide | Ball/All | Ball/All h1 | Cube | Cube h1 | Cube Through |
|---|---|---|---|---|---|---|
| a 50 | 0.508 / 0.474 | 0.465 / 0.448 | 0.375 / 0.428 | 0.468 / 0.626 | 0.514 / 0.517 | 0.568 / 0.630 |
| a 512 | 1.373 / 2.461 | 1.332 / 2.559 | 1.014 / 2.091 | 2.202 / 3.071 | 1.645 / 2.459 | 2.215 / 2.964 |
| a 2048 | 14.11 / 31.54 | 13.90 / 31.55 | 9.05 / 24.50 | 19.74 / 29.15 | 9.98 / 18.36 | 17.64 / 26.00 |
| b 50 | **0.297 / 0.567** | 0.291 / 0.566 | 0.267 / 0.536 | | | |
| b 200 | **1.297 / 2.742** | 1.656 / 3.546 | 1.535 / 3.392 | | | |
| b 512 | **20.89 / 69.06** | 31.99 / 108.72 | 27.12 / 90.08 | | | |
| c 50 | 0.136 / 0.184 | 0.137 / 0.183 | 0.128 / 0.174 | 0.190 / 0.230 | 0.165 / 0.205 | 0.190 / 0.232 |
| c 512 | 1.338 / 2.495 | 1.305 / 2.534 | 0.999 / 2.112 | 2.427 / 3.023 | 1.644 / 2.474 | 2.221 / 2.882 |
| c 2048 | 17.19 / 25.61 | 17.29 / 25.11 | 11.28 / 18.68 | 23.31 / 34.68 | 11.52 / 22.40 | 19.58 / 30.89 |

ABeautifulGame: 0.52 x at 50 texels, 0.47 x at 200, 0.30 x at 512. The light models gain at 512 / 2048 (the commit and
re-apply moved into the bands / spans) and at c-50; **a-50 Ball/SameSide and Ball/All were 7 % / 4 % slower** (the
same code's "r4 alloc" row: 0.472, so within the noise, but P8 asks <= 1.03 x). Cause found in the probe columns: those
dabs have 3 candidates and ~1,300 touched texels, but ~20,000 texels of boxes (the big triangles' boxes are the whole
dab footprint plus gutter), so they took the pool (0.9 band jobs per set dab) where round 5 stayed serial; the pool's
wake-up costs more than the dab. **Change after this bench** (v3): a dab with fewer than `PAR_DAB_MIN_CANDS` = 8
candidates stays serial up to 4 x `PAR_DAB_WORK` (65,536 texels of box work, round 5's own threshold).

## For the main session at landing (plan 5: not edited in the copy)

### New section 1.7 (insert after 1.6, round 6's)

```
### 1.7 Round 7 (brush speed)

Round 7 (`target\round7_design\ROUND7_PLAN.md`) makes 3D dabs on very heavy models cheaper and properly parallel,
with bit-identical pixels. (A1) The brush's rasterizer calls skip pass 2 when it cannot report anything: a triangle
that owns no gutter texel and claimed no texel, in a box without shared gutter texels (a per-map index built on a
set's first touch, 6.3 "Lean pass 2"). (A2) A set's dab is rasterized and committed in one parallel pass over 32-row
bands, its candidates binned by their tight scan boxes (6.4 "Round 7: the fused dab pass"); the partition depends only
on the dab. (A3) A large shared query runs its two passes and its sort on the pool. (A4) Re-apply shades only the span
of each row whose mask the dab raised. The stroke probe has five more slots (5.19). The test seam `RASTER_R5` keeps
round 5's rasterizer and commit as the reference of the equality tests and of the bench's "... r5" rows.
```

### Lines for 9.1

Append to the **texel_map** [C1] bullet:

```
  Round 7: the lean pass 2 (`raster_tri_lean` / `raster_tri_covered_lean`) visits exactly the fragments and covered runs
  of the full call, in the same order, for every triangle of round 7's meshes (atlas, sphere, tiled, stacked / mirrored,
  slivers, wrapped quad), clips none / wider than the image / inverted / random wrapping, empty and partial row ranges
  (`lean_pass2_visits_the_same_fragments`); the brush index is exact: a triangle's bit is set iff it owns a gutter texel
  or claimed one, every block with a `shared_gutter` texel has its bit, and a 1-thread build gives the same bitsets
  (`prepare_brush_index_is_complete`); the folded row loops walk exactly the rows that fold into `rows`
  (`folded_rows_are_the_rows_in_range`).
```

Append to the **brush / stroke / fill** [C3] bullet:

```
  Round 7: random dabs (ball hard / soft, Cube Solid and Through, SameSide / All, radii from under a texel to the whole
  model) on round 7's meshes and maps of 1 x 1 to 1024 x 1024, gutters 0 to 16, give round 5's masks and rects (the
  `RASTER_R5` seam) and leave `dab_cov` zero and `dab_touched` empty (`r7_dabs_equal_round5_on_random_dabs`); the fused
  pass with bands of 16 / 32 / 64 rows equals the serial loop + `commit_dab` and round 5's code on heights that are not
  a multiple of the band (`fused_band_commit_equals_the_serial_commit`); serial, banded and folded dabs and a two-set
  document stroke give the same masks, rects and layer bytes in pools of 1, 2 and 5 threads
  (`dab_raster_is_the_same_on_any_thread_count`); whole two-set strokes (ball and Cube) give round 5's document state
  with the seam on / off, the probe on / off and re-apply by spans / whole rects (`a_stroke_paints_round5s_pixels`).
  `stroke_bench_tips` (ignored timing probe) adds an "... r5" row per shared-path tip and asserts r7 hash == r5 hash.
```

## v3 = the hand-off state (A1 + A2 + A3 + A4 + the few-candidates rule)

Exe `target\round7\runs\v3\wte_v3.exe` = `work\pkg\target\release\wrapped_texture_editor.exe`
(SHA-256 EB1FD59D2657FFBA6280B7A70B0F0F36922C77C4E158182D5B904608C596076A). Release core test exe
`runs\v3\wte_core_bench_v3.exe`.

- `cargo test --workspace`: **1,074 passed, 32 ignored, 0 failed** (332 + 697 + 15 + 9 + 21; round 5: 1,067 / 32; +7 =
  T1-T6 and `folded_rows_are_the_rows_in_range`). Log `target\round7\test_ws3.log` (compiled under the lock with
  `--no-run`, `build_v3.log`; run without it).
- `cargo clippy --workspace --all-targets`: **0 warnings** (`build_v3.log`). Release build ok (`build_v3.log`).
- Smoke runs (`runs\v3\smoke.log`, script `runs\smoke_v3.ps1`, one process at a time, no cargo / rustc running):
  - `brush_types` (built-in sample): ok; with `RAYON_NUM_THREADS=1` and `=2`: ok, `shot.png`s **byte-identical** to the
    default run; and **pixel-identical to round 5's sweep shot** `target\round5\runs_gate\runs\brush_types_builtin\shot.png`
    (0 differing pixels, RGBA compared).
  - `brush_types_path` **without a PATH** (built-in sample): not runnable by design, as in round 5: the scenario asserts
    "the open_path scenario needs a model or .wtep PATH" (`btp_sample.json`, ok=false). Ran it on ABeautifulGame
    (`btp_abg`, ok, `WTE_STROKE_LOG=1`) and on `ngons_tiling.obj` (`btp_ngons`, ok) instead.
  - `workflow_path` on ABeautifulGame: ok (`wf_abg`).
- Bench v3 (`runs\v3\bench_abc.out`, `bench_abc_rows.txt`; in-process A/B, indicative): **all 54 hash lines identical**
  (C1 and P2 "r7 == r5", models a, b, c, every tip and diameter), 0 different. **Every r7 row is at or below its r5 row**:

| model diam | Ball/SameSide r7 / r5 ms/dab | ratio | worst ratio of the other tips |
|---|---|---|---|
| a 50 | 0.103 / 0.160 | 0.64 | 0.75 (Cube Through) |
| a 512 | 1.394 / 2.492 | 0.56 | 0.74 (Cube Through) |
| a 2048 | 13.91 / 31.34 | 0.44 | 0.69 (Cube) |
| b 50 | 0.296 / 0.564 | **0.52** | 0.52 |
| b 200 | 1.295 / 2.740 | **0.47** | 0.47 |
| b 512 | 21.27 / 69.67 | **0.31** | 0.30 |
| c 50 | 0.103 / 0.180 | 0.57 | 0.70 (Cube Through) |
| c 512 | 1.364 / 2.493 | 0.55 | 0.82 (Cube) |
| c 2048 | 17.31 / 25.22 | 0.69 | 0.70 (Ball/All h1) |

  The a / c 50-texel dabs (3-4 candidates) now stay serial (0 band jobs) and are 25-43 % faster than round 5 (lean
  pass 2 + span re-apply). P8's "r7 <= 1.03 x r5" holds on every row of this run.
- App, ABeautifulGame `brush_types_path`, `WTE_STROKE_LOG=1` (`runs\v3\btp_abg.log`; single run, indicative; free RAM
  was ~1-2 GB, the warm-up stroke's first dabs pay page faults of the new scratches and indexes and vary from run to
  run: its longest dab 42 / 28 ms in the two v1 runs, 165 / 165 / 209 ms in the v2 / v3 runs, with "first touch"
  13-131 ms):

| stroke | stroke work ms | longest frame ms | longest dab ms | backlog ms | release to end ms |
|---|---|---|---|---|---|
| warm-up 50 | 1,820 | 209 | 209 | 1,033 | 0 |
| Circular 50 | 927 | 125 | 32 | 487 | 0 |
| Sphere 50 | 1,034 | 106 | 34 | 773 | 0 |
| Sphere/Cube 50 | 878 | 114 | 30 | 636 | 0 |
| Circular 512 | 15,051 | 504 | 504 | 17,012 | 3,392 |
| Sphere 512 | 23,153 | 695 | 695 | 27,220 | 4,055 |
| Sphere/Cube 512 | 18,985 | 627 | 627 | 21,587 | 2,773 |

  Same-day alternating pair v1 / v2 (`runs\v1\btp_abg_b.log`, `runs\v2\btp_abg_b.log`): Size 50 779-977 / 828-1,138 ms,
  Size 512 13.6-20.2 / 12.9-20.2 s: the run-to-run spread on this machine is larger than the A3 / A4 differences in the
  app; the bench above is the reliable A/B. Against the step-0 run (round-5 code, same day): Size 50 stroke work about
  0.5-0.65 x, longest dab after the warm-up 47-66 -> 24-34 ms, longest frame <= 125 ms, backlog 920-1,460 -> 453-773 ms;
  Size 512 about 0.3-0.35 x, release to end 7.3-10.7 s -> 2.2-4.1 s.

### Where the plan's lines stand (indicative only; the gate measures)

- P1, P2: met here (tests, clippy, shots, all bench hashes).
- P3 (longest frame <= 140 ms, longest dab <= 40 ms after the warm-up): met in every v1-v3 run after the warm-up.
- **P4 (largest backlog <= 350 ms at Size 50): not met in these runs (453-773 ms).** P5 (<= 0.50 x): about 0.50-0.65 x
  here, borderline. P6 (Size 512 release to end <= 3.0 s): 2.2-4.1 s, borderline; stroke work 0.30-0.36 x (met).
  The remaining Size-50 cost (v3 Circular 50): raster 573 ms (band work 3,547 ms, width 6.2 of 12), re-apply 149, setup
  104, query 89. Pass 2 still looks at 59 M texels for 20 M touched (3.0 x, down from 15.4 x): the triangles that own
  gutter texels (island borders) or meet shared gutter blocks still scan their whole box. A next step would be to walk
  only their own gutter texels (`gutter_of`) in the lean call when no shared block meets the box; not in this plan, not
  built.
- The plan's thread count: the global pool has 12 threads here (6 cores, 12 logical processors), not 6.

### Jobs that can overlap a stroke (plan section 4)

Non-modal jobs (`app.jobs.spawn(.., modal: false)`): "Loading system fonts" (`io_flows.rs`, font scan, no rayon),
"Drawing font samples" (`text_ops.rs`; `text\raster.rs` / `styled.rs` use rayon, so these share the global pool with a
dab), stamp Final bakes (`stamp_bakes.rs`, their own `stamp-bake-N` pool; not started while a stroke is open) and the
custom tip-shape load when no modal job runs (`brush_flows.rs`, modal flag per job; `tip_bake.rs` / `tip_field.rs` use
rayon on the global pool). Import, document build, project open / save, export and extraction are modal (the UI waits
behind the progress dialog, no stroke can start). Nothing was changed for these.

### Files and folders

- Source changes: `crates\wte_core\src\{texel_map.rs, brush.rs, stroke.rs, bvh.rs}`, `crates\wte_app\src\tools\brush.rs`,
  `ARCHITECTURE.md` (copy). No other file.
- `target\round7\step0_src\` = the step-0 measuring sources; `target\round7\a12_src\` = an intermediate snapshot (stale,
  before A3 / A4 and the fixes); both only for the record. `runs\step0`, `runs\v1`, `runs\v2`, `runs\v3`, `runs\diag`:
  the runs above (exes copied there; `pkg\target` holds the builds).
