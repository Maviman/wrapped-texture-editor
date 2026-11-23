# Round 9, check:B — notes

Checker: check:B (Opus), 2026-11-22/23. Worktree `G:\software_projects\texture_wrapper_ap_r9b`, branch `round9-b`. Logs and
runs under `target\round9\check_b\` (git-ignored): `test*.log`, `fp_*.log` (fingerprints), `t_*\<model>\shot.log` (exe
runs), `probe.sh` / `sweep.sh` / `parse.py`, `bin\` (exes). Written as the work goes; the last section is the final state.

## 1. Verification of build:B's claims

| Claim (pkg_b.md) | Checked | Result |
|---|---|---|
| 1,167 passed / 5 failed / 33 ignored, clippy 0 | full `test --workspace --no-fail-fast` at `7201033`; clippy `--workspace --all-targets` | **confirmed** (360 + 760 + 15 + 9 + 23 passed; 16 + 17 ignored); clippy 0 warnings |
| Red set = A's with A's numbers | same run | confirmed: `wa4_r8` 60 deg ink 1.1177, cells 0.769..1.432; `wa20` ink 1.1890, cells 0.665..2.886 (dup 0.976 %, scatter 0.564 %); `wa17` / `wa24` ink 0.915 / 0.9146; `wa35` side 92.3681 % within 2/255, max 51 |
| WA28 compares bytes | read `sphere_check::fingerprint` | **yes**: stats except `micros` (Debug), reach / needed_along / spine_y / src_size / stop / half bits, per layer the triangle ids, corner and depth bits, clip / period / axis, owner grid lo / cell / dims and every cell's g / pos / scale bits. Only parameterizations made on the test's own thread are compared (the thread-count tests' pools are not), 135 in all |
| ABeautifulGame Jump on 32.3 s, slower than round 8 | `wte_r8.exe` and build:B's exe alone, sequential, idle | confirmed slower: round 8 alone **13.5 s** param (14.2 s applied; the plan's "20 s" was three runs in parallel), build:B 29.0 s (owner 16.5 s, relax 11.3 s) |
| Jump off 1.72 s | same | 1.36 s param, 2.15 s applied (the first Final pays the graph: 0.71 s); round 8 alone 2.77 s |
| Avocado 42 / 65 ms (owner ~ all of it) | same | 34.5 / 60.0 ms; round 8 alone 8.4 / 19.3 ms |
| high_poly_sphere 7.4 / 6.8 s | same | 6.39 / 6.17 s |

## 2. Speed (package A's code, check:B's authority from plan section 6)

Rule kept: **byte-identical results** (no quality number may move). Tools: `check_b_fingerprints` (ignored test: every
parameterization the wrap tests make on their thread, 601, hashed over everything but the build time; before / after
compared), `check_b_probe` (ignored, release: one sweep parameterization rebuilt from the scenario's new `placement bits`
log line, timed by phase, with its fingerprint hash).

Where the time went (release probe, ABeautifulGame step 10):
- Jump on, owner 20.0 s: `may_bury` 262,053 calls, **155 s of thread time** (each a BVH sphere query collecting the
  thousands of triangles round the prism box, then filtered to other entered parts); `buried_at` 144,381 calls 0.3 s.
- Jump on, relax 10.8 s: seams + wedges 0.5 s; the exact region (T5) tried 2,021 pins and **fell back**, so every one of
  the 30 components was solved twice, one after another, each with kernels that ran on one thread below 16,384 unknowns.
- high_poly_sphere: one component, solved twice (exact region of 2 wedges, fell back), 2.7 ms per CG step.
- Avocado: the owner grid (1,024 x 525 cells) built on one thread: 10 ms pass 1 (a third of it the page faults of the
  25 MB cell array), 20 ms the spans, `lost_cut`, the output grids and the count pass.

Changes (all byte-identical; fingerprints 601 / 601 equal, probe hashes equal):

| # | Change | Where |
|---|---|---|
| S1 | `may_bury` answered from a per-layer index of the entered parts' triangles (per part a box and a uniform grid of triangle boxes, big triangles listed apart): the same two tests (box overlap, closest point within the circumscribed sphere) in the same f32 arithmetic, `any` over a superset found by grid cells | `wrap_param.rs` `BuryIndex`, `OwnerCtx::may_bury` |
| S2 | Solve by levels: a component reads another's result only through `relaxed_pose` of its jump source (when that component comes first in `order`); components of one level run on the pool, each from the positions / pins the sequential loop would give it, results written back in order | `wrap_relax.rs` `Layout::relax` (`solve`, `source_comp`) |
| S3 | The exact region's fallback reuses a component's first result when its start positions and pins are bit-equal (`CompRun`) | same |
| S4 | Per-element kernels (terms, gather, CG updates, preconditioner, ARAP local step) on the pool in chunks of 1,024 (`each`); the energy summed in triangle order; sums keep `PAR_CHUNK`; `PAR_MIN_UNKNOWNS` 16,384 -> 8,192 (bits do not depend on it: check:A's test) | `wrap_relax.rs` `System`, `relax_component_from` |
| S5 | Owner grid on the pool from 65,536 cells (not only from 2,048 triangles): first touch of the cell array, output grids, row / column spans, `beyond` seeds, `lost_cut` rows (integer sums, min / max merges) | `wrap_relax.rs` `OwnerGrid::build` |
| S6 | Owner grid outputs and a compact per-cell mark (covered, closure) in one pass; the spans, seeds and `lost_cut` read the marks instead of the 48-byte cell states (Avocado owner 16 -> 13 ms) | same |

Not applied (they change results; a quality decision for package A / the gate, not a speed fix): the owner grid sized by
the triangle count (build:B request 1), skipping the grid on injective layouts (request 2: `keeps` and `lost_cut` still
read it), fewer ARAP / CG steps or another preconditioner (request 4).

Checks of S1-S5: `check_b_fingerprints` 601 / 601 hashes equal (default pool) and equal with `RAYON_NUM_THREADS=1`
(WA7's wall-clock retries add repeats there: 319.9 ms one-thread, red as check:A found, 308 ms); full
`test --workspace --no-fail-fast`: 1,167 passed / 5 failed (the same five, the same numbers) / 35 ignored (+2: the two
check:B tools); clippy 0.

Release probe (`check_b_probe`, idle machine, this machine's 8 threads), before -> after, fingerprint hash unchanged:

| case | before (build:B) | after S1-S5 | hash |
|---|---|---|---|
| ABeautifulGame Jump on | 32.7 s (owner 20.6, relax 10.9) | 5.6-6.3 s (owner 0.21-0.23, relax 4.3-4.9) | `0e04f53c2484ec79` both |
| ABeautifulGame Jump off | 1,255 ms (relax 1,165) | 615-756 ms (relax 523-660) | `7497da3c18a487b4` both |
| high_poly_sphere | 6.0-6.4 s (exe) | 4.66-4.94 s (relax 4.2-4.5: one component solved twice, the exact region of 2 wedges falls back) | stats equal |
| Avocado off / on | 32 / 65 ms (owner 31 / 63) | 17 / 43 ms (owner 16 / 40) | `34dd04da03ee66ad` / `ea75788a41fd9ef0` both |

## 3. Races at app level (break list, part 1)

New app tests (`stamp_bakes.rs`), both green:
- `the_warm_up_and_the_first_wrap_final_build_one_graph`: the selection's warm-up thread and the first wrap Final start
  in the same frame on a fresh document; one graph (`builds() == 1`, the job waits in the cell or the warm-up does),
  Final pixels = the synchronous Final, shared texels known afterwards, no second warm-up.
- `a_new_document_or_a_close_during_the_warm_up_is_safe`: New / Open while the old document's warm-up runs (the new
  document's warm-up starts once the old thread ended, the tool asks every frame; its own cell, built once, shared
  texels known); a third document closed during its warm-up (the thread finishes on its own, is joined, nothing new
  starts without a document).
- Finding (plan T8 says the graph is warmed up "when Wrap is chosen or a wrapped stamp is selected / loaded"; build:B
  warmed up only for a selected stamp, so the first wrapped Final on a heavy model paid the graph build: ABeautifulGame
  Jump off applied 2,025 ms for a 900 ms parameterization). Fixed: the Stamp or Text tool in Wrap mode asks for the
  warm-up every frame (`TransformTool::ensure_wrap_param`); test `choosing_wrap_in_the_stamp_tool_warms_up_the_graph`.
  No pixel can change (the warm-up only fills the shared cell). App tests 363 passed.

## 4. The three red step-10 asserts (plan 7.2: `covered >= 0.85 x ends_share unless reached_edge`, `lost_cut <= 10 %`)

The scenario's assert is the plan's wording, letter for letter (`!reached_edge && covered < 0.85 x ends_share`, and
`lost_cut > 0.10`), so there was nothing to correct to the wording. Verdict per model (numbers equal in build:B's, A's
and this branch's exe):

| Model | Numbers | Verdict | Action |
|---|---|---|---|
| ABeautifulGame, Jump off | covered 0.0714, ends_share 0.331, reached_edge false, passed, seams [334, 46, 0, 0, 0], lost_cut 0 | **not a loss: the picture (40 % of the diagonal) is larger than the part in both directions.** It lands on a white pawn; the pawn's whole part is laid (28,416 candidates = 28,416 stored triangles, 14,210 vertices), the ends meet round it (S1) and the halves meet over its top (S2). The pawn is closed, so `reached_edge` stays false, and `ends_share` only measures the along direction. `shot_off.png`: the pawn fully covered (all eight white pawns show it: shared texels, panel line). | assert left as worded (red); gate / user: the 7.2 exemption should also cover a part smaller than the picture across (halves meet, S2), e.g. "unless reached_edge or the halves meet" |
| many_islands | off: covered 0.0177 vs 0.85 x 0.184 (one cube island: 8 vertices, 12 triangles, all laid; S1 2, S4 6); on: 0.0333 vs 0.85 x 0.332 (three cubes, 2 jumps) | **same case: islands by design, each a closed cube far smaller than the picture**; nothing more of the picture has surface (lost_cut 0 / 1.48 %) | same as above |
| Lantern (off = on) | lost_cut 11.23 % (limit 10 %), covered 0.885, seams [0, 0, 0, 11, 19], 52 vertices / 100 triangles laid, stretch 31 % | **real, but by design**: the picture sits on the post's square base; the coarse box has corners (S4 11) and the base round the post is not a disk (S5 19, tree-cotree); the cut wedges have no surface and the panel names the cut ("Cut where it cannot lie flat ... 12 % of the picture runs off the surface"). Not a closure (check:A's rule does not apply), not a measuring error. | no code change (package A's open item since check:A); the 10 % line stays red |

## 5. Graph path vs sphere path on the 13 sweep models (break list, part 2)

`probe_all.sh` (release probe with `WTE_PROBE_SPHERE=true`, the placement of each model's step 10 from the sweep's
`placement bits` line): the graph path's fingerprint hash equals round 8's sphere path's on **all 26 lines** (13 models
x Jump off / on), e.g. ABeautifulGame `7497da3c18a487b4` / `0e04f53c2484ec79`, high_poly_sphere `03ffb430ad5e9cb8`
(`target\round9\check_b\probe_all.txt`). So WA28's equality also holds on the real models, as a whole parameterization
(layers, owner grids, stats), not only as sweep numbers.

## 6. Threads, identity, synchronous Finals (exe `wte_cb3`, `target\round9\check_b\`)

- **1 vs 12 threads** (`RAYON_NUM_THREADS=1` / `12` vs the default 8; `s_t1`, `s_t12` vs `s_cb3`; Avocado,
  DamagedHelmet, Lantern, ABeautifulGame, torus_knot, klein_bottle): every logged `wrap_surface: composite` hash (14
  per model), every step-10 quality / coverage line and all 7 shots per model pixel-identical (`cmp_runs.py`).
- **Before / after check:B's speed changes** (build:B's `sweep_b` vs `s_cb3`, all 13 models): composites, step-10 lines
  and the 7 shots per model identical.
- **Identity with `wte_r67.exe`** (A's r67 runs in `..._r9a\target\round9\ident\r67`): `stamp`, `text_warp`,
  `export_run`, `roundtrip` ok, `shot.png` 0 differing pixels each; export_run's 7 files byte-identical;
  `check_exports.py`: PASS.
- **`WTE_SYNC_FINAL_BAKES=1`**: `wrap_surface` on the built-in sample, WaterBottle, high_poly_sphere ok; the 14 composite
  hashes each equal the asynchronous run's and build:B's synchronous run's.

## 7. Timings (release, this machine: 8 threads; alone, sequential)

Exe sweep step 10 (`param` = the `wrap:` line, `applied` = "Final applied after"; round 8 alone = `wte_r8.exe` run here
alone, round 8 sweep = today's r8 logs with three runs in parallel):

| line | round 8 | build:B | check:B (`s_cb2` / `s_cb3`) | plan 7.3 |
|---|---|---|---|---|
| T-a high_poly_sphere param / applied | 11.0 / 11.1 s (sweep) | 6.4 / 6.9 s | 5.6 / 6.1 s; 6.1 / 6.6 s | <= 5 s and <= 0.5 x r8: **not met** (relax 86-90 %: one 78k-wedge component, solved twice because its 2-wedge exact region falls back; ~2.7 ms per CG step, memory-bound) |
| T-b ABeautifulGame Jump off param / applied | 2.77 / 2.87 s alone (3.44 s sweep) | 1.36 / 2.15 s | 0.90 / 2.03 s; 1.13 / 2.05 s | <= 1.0 s: param met in one of two runs; applied includes the first wrap's graph wait (0.8 s) |
| T-b ABeautifulGame Jump on | 13.5 / 14.2 s alone (20.3 s sweep) | 29.0 / 29.7 s | 7.2 / 8.1 s; 6.6 / 7.3 s (probe 5.6-7.2 s) | <= 6 s: **not met** (ratio to round 8 alone 0.49-0.53; relax ~75 %) |
| Avocado off / on param | 8.4 / 19.3 ms alone | 34.5 / 60.0 ms | 21.5 / 41.0; 19.1 / 36.4 ms (probe after the last owner change: 14 / 49 ms) | - |

WA15 probe (release `wrap_bench_models`, `wa15.log`), graph path:

| spot | build:B | check:B | limit 7.3 |
|---|---|---|---|
| graph build, ABeautifulGame | 697 ms | 682 ms | T-c <= 1.0 s: met |
| board 20 %, Jump off | 35.8 ms | **22.2 ms** (owner 96 %) | T-c <= 60 ms: met |
| piece 20 %, Jump off | 166 ms | **140.8 ms** (relax 76 %) | T-c <= 150 ms: **met** (was not) |
| board 20 %, Jump on | 4,648 ms | 861 ms (relax 68 %) | T-d <= max(1.5 x off, 150 ms) = 150 ms: not met |
| piece 20 %, Jump on | 945 ms | 546 ms (relax 54 %) | T-d <= 211 ms: not met |
| board 100 %, Jump on | 104.8 s | 26.9 s | - |
