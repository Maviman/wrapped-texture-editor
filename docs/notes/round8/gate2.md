# Round 8 gate, stage 2 (plan sections 7.2 and 7.3)

Date 2026-11-22, 04:40-05:30 local. Raw runs and logs: `target\round8\runs_gate2\` (`driver.ps1`, `driver.log`,
`runs.csv`, `load_samples.txt`, `lock_events.txt`, run folders `g5a`, `g5b`, `g5c`, `g5d`, `g6`, `g8`, probe outputs
`wa15_run1.txt` / `wa15_run2.txt`, analyses in `analysis\`, build logs in `build_logs\`, tools in `tools\`).
Exe measured: `target\round8\wte_round8.exe`, SHA-256 `19D7844B4BDCD73ECE7327FB3F677CD58695A8314EA80F42CB194CDA97E54B2A`
(gate 1's exe; unchanged in this stage). Report: `target\round8\GATE_REPORT.md`.

## How the runs were made

- `driver.ps1` (round 6's driver with round 8's paths) -> `sweep_peak.ps1` (round 6's copy of `round3_audit\sweep.ps1`
  that also polls peak working set and private bytes every 250 ms). Release exe, 1280 x 800, `WTE_FRAME_LOG=1`, one
  process at a time. W = `WTE_STAMP_MODE=wrap`. Order W, P, P, W for G5a-G5d (2 runs per mode). G6 = `wrap_surface` with
  each of the 13 models (Jump off and on inside one run). G8 = `fuzz_path` seed 7, 1,000 steps, ngons_tiling, W, 2 runs.
- Before each run the driver waits while cargo / rustc / clippy-driver / another `wte*` or `wrapped_texture_editor` runs
  (it never had to wait) and logs CPU, free memory and what ran (`driver.log`: "running before: []" for all 32 runs).
  `sampler.ps1` logged CPU, free memory and the top processes every 5 s (`load_samples.txt`).
- Machine: idle as far as builds go (no cargo / rustc / other app exe during any measurement; rustc only during my own
  two probe builds, 04:58-05:01 and 05:04-05:07, between measurements). In the background: bambu-studio (the user's
  app, ~0.2-0.3 s CPU per 5 s), Discord, Claude. One short Python parse of mine (< 1 s) overlapped G5a's W run 2
  (04:42:00-04:42:04). Free memory 0.9-2.2 GB, but during the ABeautifulGame `workflow_path` runs (G5c) it fell to
  8-400 MB and the app's working set was trimmed (e.g. ws 379 MB with 2.3 GB private): those runs (W and P alike) ran
  under memory pressure.
- Tools: `bakes8.py` (round 6's `bakes6.py` plus the round-8 job lines `wrap preview: ...` / `wrap fit: ...`),
  `tools\fuzzframes.py` (round 5), `tools\sweep_table.py` (gate 1's), `tools\locked_cargo.ps1` (gate 1's, label
  `round8-gate2`, log `lock_events.txt`).
- WA15 probe (G5e / G5f): `wrap_tests::wrap_bench_models` (round 6's ignored probe), release test build
  `cargo test --release -p wte_core --lib --no-run` under the build lock (3 min 31 s, then 2 min 27 s), the test exe
  run directly (`--ignored --nocapture --test-threads 1`), nothing else running. Probe exes kept as
  `runs_gate2\wa15_probe_a.exe` (run 1) / `wa15_probe_b.exe` (run 2).

## Temporary changes in `work\merged` (all reverted; merged = pkg_b again, `diff -rq` of `crates\` empty)

1. Probe (test only, `wrap_tests.rs`): added `high_poly_sphere.obj` (at 20 % and 40 %, not 100 %) and the WA20 bumpy
   ball (`r8_bumpy_ball`, 40 %, Jump off / on) to `wrap_bench_models`. Used for runs 1 and 2.
2. Run 2 only: a test-only phase clock (`#[cfg(test)] mod phase_clock` in `wrap_param.rs`, `#[cfg(test)]` timing
   statements in `wrap_param` and `relax`): per-phase times "collect+weld" (candidate sphere query + `Patch::build`),
   "anchor+spine", "front", "seed halo+tears", "relax: seams+wedges", "relax: LSCM+ARAP solve", "relax: halo+seam
   stats", "early-out test+relax total", "stretch stats+owner grid". Compiled out of non-test builds; the app exe was
   not rebuilt.
3. Originals were backed up in `runs_gate2\tools\*.orig` and copied back after run 2; `work\merged\target\release\
   wrapped_texture_editor.exe` is still `19D7844B...` (checked). Because the restored files have new mtimes, the next
   cargo command in merged recompiles wte_core.

No product-code change was made in this stage (see "Fixes considered"). Tests, clippy and the release build were
therefore not rerun; gate 1's results stand: 1,147 passed, 4 failed (wa4_r8, wa20, wa22, wa24), 33 ignored; clippy 0.

## Results (plan line -> measured; W / P; run 1 / run 2)

### G5a, sphere sample `stamp` (`analysis\g5a.txt`)
- Final applied after, W run 2: image 47, scale 50, move 64, rotate 64, scroll 28 ms. W run 1: 160 (first bake: a
  96-ms first parameterization in a fresh process; 25.7 ms in run 2), 46, 63, 82, 31 ms. P: 31 / 34 / 30 / 32 / 32 and
  29 / 31 / 31 / 31 / 32 ms.
- Limit "<= 1.5 x P and <= 80 ms (second run)": <= 80 ms met in run 2 (max 64); <= 1.5 x P (~44-48 ms) **not met** for
  4 of 5 Finals of run 2 (1.5-2.1 x).
- Why: the reused Finals' worker time is 2-3 ms; the rest is the wait for the drag's last Preview job (plan 4.1: a Final
  waits for its stamp's Preview job in flight; that job was dropped as stale after 49-51 ms and its parameterization
  was reused, request 1). The 17-point drag's last job was made for an earlier placement: the Final built a fresh one
  (35.6 ms). The probe shows the owner grid (always ~1024 x 512 cells, two passes) is 13.7-18 ms of the 15-19 ms
  parameterization of a 20 % picture on the sphere.
- Preview landing (job start -> applied): run 1 32, 31, 34, 48, 49, 51 ms; run 2 32, 32, 33, 36, 47, 50 ms. Limit 50 ms:
  met in run 2, one 51-ms landing in run 1. Measured from the job's start, not from the pointer event (a move made
  while a job runs waits for it: latest wins).
- Drag frames: W max 17.1-19.2 ms, P max 17.4-20.9 ms. 0 frames > 300 ms.

### G5b, ngons_tiling `workflow_path` (`analysis\g5b.txt`)
- Applied after W: image 31 / 139, text 83 / 117, move 148 / 114, scale 145 / 147, rotate 98 / 131 ms. P: 313 / 315,
  150 / 150, 182 / 171, 254 / 247, 365 / 364 ms. All W <= 1.5 x P: **met**.
- 0 frames > 300 ms between started and applied (max 45.8 ms): **met**. Drag max W 21.6 / 29.0 ms vs P 184.2 / 184.4 ms:
  **met**. (W2's first parameterization 86 ms vs 5.5 ms in W1: first call in a fresh process again.)

### G5c, ABeautifulGame `workflow_path` (`analysis\g5c.txt`; memory pressure, see above)
- Drag frame max (transform steps): W 19.5 / 22.6 / 18.7 and 18.9 / 19.6 / 19.6 ms; P 29.7 / 62.2 / 65.6 and 31.2 /
  52.4 / 51.3 ms. **Met** (W below P).
- Finals applied after, W vs P: image 3,073 / 2,503 vs 1,655 / 1,453 ms (1.86 / 1.72 x); text 317 / 304 vs 35 / 38
  (9.1 / 8.0 x); move 616 / 615 vs 67 / 68 (9.2 / 9.0 x); scale 1,014 / 981 vs 132 / 112 (7.7 / 8.8 x); rotate 1,086 /
  1,013 vs 118 / 94 (9.2 / 10.8 x). Limit 1.5 x: **not met** for all five.
- Image worker 2,960 / 2,468 ms: limit 2.0 s **not met**.
- 0 frames > 300 ms between started and applied (max 42.6 ms): **met**. The other frames > 300 ms (20-23 W, 12-13 P)
  are self-test checkpoint steps, model load, reopen and exit.
- Preview jobs: one per drag, landing 299 / 486 / 615 and 281 / 466 / 567 ms after its start, each after the release,
  dropped as stale (the drags last 150-275 ms in P). No Preview landed during a drag; the last finished picture stayed
  (user answer Q4).
- Shares (`wrap:` lines): param 2,777 / 2,376 of 2,960 / 2,468 worker ms for the image (94-96 %; 14,210 verts, relaxed,
  599 seams, stretch 95 %, covered 13.4 %); text 300 / 276 of 314 / 289 ms; move 341 / 363 of 353 / 377 ms plus 263 /
  238 ms waiting for the stale Preview job; scale 519 / 502 plus 478 / 464 ms wait; rotate 534 / 512 plus 534 / 484 ms
  wait. Inside the parameterization (probe, the piece spot, 20 %, 3,698 verts): collect+weld 677 ms (63 %), solve 266 ms
  (25 %), front 43, seams+wedges 32, owner grid 17 ms.

### G5d, ABeautifulGame `text_warp` in Wrap (`analysis\g5d.txt`, `g5d_swap.txt`)
- 0 frames > 300 ms between each Final's start and its swap (max 134.6 ms at the first one): **met**.
- Time to swap (scenario's "final in"), Arc / Flag / Fisheye / Arc: W 2,498 / 2,501 / 2,015 / 2,498 and 2,458 / 2,214 /
  2,115 / 2,775 ms; P 946 / 947 / 817 / 951 and 1,044 / 932 / 780 / 932 ms. 2.4-2.9 x; limit 1.5 x **not met**. (Round
  6: W 0.95-1.40 s.) The 2D text's swaps (stamp 33 / 42) are 110-117 ms in both.
- Shares: param 1,818-2,420 ms of 1,916-2,568 worker ms (92-95 %; 14,210 verts on a piece, relaxed, 256-553 seams,
  stretch 95-128 %, covered 17.5-28.2 %); bake 78-109 ms.

### G5e / G5f, WA15 probe (`wa15_run1.txt`, `wa15_run2.txt` with phases, `analysis\wa15.txt`)
| line | run 1 | run 2 | limit | verdict |
|---|---|---|---|---|
| ABG board (centre) 20 % param, Jump off | 385.9 ms | 339.0 ms (collect+weld 306.7) | 150 ms | not met |
| ABG piece (off-centre) 20 % param, Jump off | 1,054.6 ms | 1,070.7 ms (collect+weld 677, solve 266) | 150 ms | not met |
| welded graph one-off build | - | - | reported, <= 1.0 s | not built (package A step 6) |
| ABG board 100 % | 3,301.6 ms | 2,910.9 ms (collect+weld 1,829, solve 770) | 3 s | run 1 not met, run 2 met |
| ABG piece 100 % | 2,292.0 ms | 2,392.5 ms | 3 s | met |
| Wrap Full <= 1.5 x projected Full (Jump-off rows) | board 20 %: 2.2 vs 253.8; 100 %: 59.3 vs 3,388.6; piece 20 %: 34.6 vs 755.6; 100 %: 47.3 vs 2,879 (run 2) | same order | 1.5 x | met everywhere (also ngons, sphere, high_poly_sphere) |
| G5f board: Jump on <= max(1.5 x off, 150 ms) | 2,432.3 vs 578.9 ms (6.3 x off) | 2,308.1 vs 508.5 ms (6.8 x) | | not met (jumps 3; 24,817 verts relaxed, 14,345 of 49,746 triangles flipped) |
| G5f piece | 1,690.3 vs 1,581.9 ms (1.60 x) | 2,102.4 vs 1,606.1 ms (1.96 x) | | not met (jumps 6) |
| G5f Overlap 2.5 turns: param + bake <= (turns + 0.5) x Ends meet | 0.8 + 18.7 = 19.5 vs 3.5 x 10.5 = 36.8 ms | 0.7 + 17.8 = 18.5 vs 37.1 ms | | met (turns 3) |
Extra rows (not limits): high_poly_sphere 20 % 1.55-1.66 s, 40 % 8.59-8.97 s (solve 6.60-6.85 s = 77 %, collect+weld
0.96-1.17 s, seams+wedges 0.33 s, front 0.24-0.28 s); bumpy ball (WA20) 40 % 199-216 ms (solve 147-155 ms = 74 %,
owner grid 31 ms); sphere sample 20 % 12-19 ms (owner grid 13.7-18.0 ms), 100 % 98-117 ms (solve 62-81 ms); ABG board
100 % Jump on 43.0 / 40.7 s (739k verts, 110 jumps, 413k flipped).

### G6, sweep times (gate 1's sweep plus this stage's rerun, `analysis\g6_table.md`)
The rerun (04:54-04:58, idle) gives the same quality numbers and pixel-identical `shot.png` for all 13 models as gate 1
(the sweep is deterministic). high_poly_sphere Final applied 11,360 / 11,561 ms (Jump off / on; gate 1 11,175 / 11,546);
limit 6 s **not met**; param 11,279 / 11,471 ms of it. Others (off / on, ms): ABeautifulGame 3,486 / 13,147; klein_bottle
229 / 229; torus_knot 100 / 116; DamagedHelmet 80 / 85; Lantern 66 / 24; Avocado 48 / 60; WaterBottle 49 / 36; mobius 31
/ 50; ngons_tiling, nonmanifold, many_islands, flat_annulus 15-33.

### G7, ABeautifulGame `workflow_path` peak memory (`runs.csv`, G5c runs)
Peak working set W 1,464 / 1,950 MB vs P 1,995 / 1,916 MB; peak private W 3,411 / 3,402 MB vs P 3,312 / 3,421 MB. Limit
W <= P + 300 MB: **met** (at most +99 MB private). The working sets were trimmed under memory pressure (above).

### G8, fuzz seed 7, 1,000 steps, ngons_tiling, Wrap (`analysis\g8.txt`)
Both runs ok (190.4 / 190.2 s), 0 errors, 0 panicked / Invalid parameter / Unknown layer / ERROR lines; 0 central,
dispatch or pointer frames over 500 ms (0 over 300 ms); 42 wrap Finals, 38 / 39 Preview jobs (29 / 30 applied, 10
dropped stale) per run. **Met.**

## Fixes considered (plan 7.2: only small and contained)

- G5a: the excess over 1.5 x P is the Final's wait for the drag's Preview job, which is the plan's binding rule (4.1,
  check:B open item 3), plus the owner grid's fixed ~0.5M cells (plan constant `OWNER_MAX_CELLS`). Changing either
  changes behaviour or results; not a gate fix.
- G5c, G5d, G5e, G5f, G6: 63-90 % of the parameterization on ABeautifulGame is the candidate-sphere collection and weld
  (package A step 6, "growth from the anchor", not built), the rest the relaxation solve; on the bumpy spheres the
  solve is 74-78 %. No small fix; reported with the shares above.
- No product code changed; quality limits untouched.

## Not done / open

1. Package A step 6 (welded graph, growth, `wrap_mesh_prep`, warm-up job, WA28) is not built: the welded-graph one-off
   time of G5e cannot be measured.
2. The plan's WA15 additions were partly made here (piece spot already existed; bumpy ball and high_poly_sphere added;
   per-phase shares measured with a temporary test-only clock). They are not kept in the tree.
3. G5a's Preview landing is measured from the job's start, not from the pointer event.
4. No request for files I do not own.
