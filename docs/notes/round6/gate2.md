# Round 6 gate, stage 2 (plan sections 7.2 and 7.3)

Date 2026-11-21, 17:53-19:50. Raw runs and logs: `target\round6\runs_gate2\` (`driver.ps1`, `driver.log`, `runs.csv`,
`load_samples.txt`, run folders `g5a` .. `g8`, `wa15_run1.txt` / `wa15_run2.txt`, analyses in `analysis\`, build logs
in `build_logs\`). Work copy: `target\round6\work\merged`. Report: `target\round6\GATE_REPORT.md`.

## How the runs were made

- Exe: `target\round6\wte_round6.exe`, release, 1280 x 800, `WTE_FRAME_LOG=1`, one process at a time
  (`driver.ps1` -> `sweep_peak.ps1`, a copy of `round3_audit\sweep.ps1` that also polls the app's peak working set and
  peak private bytes every 250 ms). Wrap = `WTE_STAMP_MODE=wrap`; order Wrap, Project, Project, Wrap (2 runs per mode
  per line). `wrap_surface` (G6) sets Wrap itself; G8 runs in Wrap.
- Before each run the driver waits (at most 5 min) while a cargo / rustc / clippy-driver or another app exe
  (`wte_*`, `wrapped_texture_editor`) runs, then logs CPU, free memory and what runs (`driver.log`). A sampler logged
  CPU, free memory and the top processes every 5 s (`load_samples.txt`).
- Machine: another agent (round 7, `target\round7\runs\v2|v3\wte_v2.exe / wte_v3.exe`, `wte_core_bench_v3.exe`, and its
  cargo builds) ran ABeautifulGame measurements and builds in bursts during this stage. Runs it overlapped are marked
  "loaded" below. bonewright was not seen. Free memory 0.16-3.0 GB (lowest, 158 MB, during WA15 run 0 at 18:04 with
  `wte_v2.exe` running; that run was discarded).
- Tools: `bakes6.py` (adapted from round 5's `runs_gate2\bakes.py`: per worker bake, per pointer step, wrap lines,
  frames > 300 ms), `g6table.py` (the sweep table), round 5's `fuzzframes.py`, round 3's `slow.py`.

## Changes made in `work\merged` in this stage

1. **Probe spot (test-only, `wrap_tests.rs::wrap_bench_models`)**: package A's probe skipped `ngons_tiling.obj` (its three
   centre rays miss). The probe now measures two spots per model: "centre" (package A's rule, unchanged: on
   ABeautifulGame the +Z centre ray passes between the pieces, so it is the board seen from above, triangle 222051) and
   "off-centre" (the first of the four other +Z rays of `stamp_bench_tiled_model` that hits: on ABeautifulGame a chess
   piece, triangle 3066; on ngons_tiling a tile). It prints the spot. A first version that tried the extra rays before
   the +Y ray changed ABeautifulGame's spot to the piece (that is why runs 0 and the first try differed from
   package A: 3,698 verts instead of 123); package A's own probe exe (13:13) run at 18:17 confirmed the board spot
   (123 verts, 1,262,364 projected texels). Ignored test; no product code.
2. **`text_warp` scenario in Wrap (selftest code, `scenarios.rs` `t7_warp_steps` / `t7_remember_step`)**: the G5d runs
   in Wrap failed at "the baked bounds changed and the anchor stayed": a wrapped text on ABeautifulGame touches UV
   islands all over the 2048 x 2048 set, so its bounds were the whole set before and after the Arc warp (logged
   `IRect {0,0,2048,2048}`). For a wrapped text the assert now requires the baked pixels (the ink hash it already
   computes) to change; projected texts keep the bounds check (unchanged). Product code unchanged. The two failed
   Wrap runs' folders were deleted before the rerun by mistake; their failure lines are in `driver.log` (18:51 and
   18:52) and the bake lines were read before deletion (first Final 876 ms, Arc Final 1,143 ms).
   `text_path` has the same bounds check (`t7_kind_changed`); it was not run in Wrap and not changed.
3. Release build after change 2 (4 min 00 s, nothing else running). New exe copied to `target\round6\wte_round6.exe`,
   **SHA-256 6CE65B55F15861BF91AF2DA5FDFB493482F8F050CB51B801D042518E57FD5433**. The gate-1 exe (F7CF77DF...31C8) is
   kept as `runs_gate2\wte_round6_gate1.exe`. G5a, G5b, G5c and the two Project G5d runs were made with the gate-1 exe;
   the Wrap G5d reruns, G6 and G8 with the new one. The two differ only in selftest code.

## Results (plan number -> measured)

See GATE_REPORT.md for the full tables; the analysis files hold every value.

- G5a (`analysis\g5a.txt`, idle): Wrap applied after 84 / 33 / 31 / 31 / 32 ms (run 1) and 29-31 ms (run 2); Project
  30-32 ms. Run 1's first bake had a 49.7-ms first parameterization (57 verts; 0.5 ms in run 2; gate 1 saw 202 ms
  once): the first call of the wrap code in a process whose exe pages were not cached (inferred, not proven). Preview
  frames during the drags: max ui 8.6 / 5.2 ms (limit 30). Met in run 2; run 1 misses 80 ms / 1.5 x on that first bake.
- G5b (`analysis\g5b.txt`; p1 loaded by `wte_core_bench_v3.exe`): Wrap applied after 30-98 ms, Project 137-1,360 ms;
  0 frames over 300 ms between started and applied; drag max frame Wrap 64.4 / 64.7 ms, Project 224.5 (loaded) / 161.2.
  Met. The wrapped picture covers only 11-30 % of its rectangle there (ngons_tiling's tiles are separate parts).
- G5c (`analysis\g5c.txt`, idle): image W 942 / 930 vs P 1,361 / 1,359 ms (worker 927 / 885 <= 2.0 s); text W 70 / 87
  vs P 20 / 36; move 88 / 94 vs 67 / 61; scale 161 / 155 vs 103 / 93; rotate 166 / 164 vs 98 / 114. Not met for
  text, scale, rotate (1.67-3.5 x). Started-applied frames max 24.6 ms (met). Drag: W 142.2 / 144.1 ms vs P 49.5 /
  50.0 ms (limit about 80): not met; the long frame is `scene sync` 138 ms = the throttled Preview bake, which rebuilds
  the parameterization (the Finals of the same placements took 125-133 ms of param: about 90 % of the frame).
- G5d (`analysis\g5d.txt`, idle): 0 frames over 300 ms between any Final and its swap; time to swap Wrap 1,182 / 1,048 /
  982 / 1,396 and 1,347 / 1,025 / 949 / 1,047 ms vs Project 1,042 / 994 / 906 / 1,068 and 991 / 1,053 / 912 / 1,086 ms
  (min/min 0.98-1.19 x; worst pair 1.31 x). Met. Param 775-1,163 ms of each Final (85-90 %). The text covers 16-28 % of
  its rectangle (it lies on one chess piece).
- G5e / G5f (`wa15_run1.txt` mostly idle for its first ~30 s, then `wte_v3.exe` ran; run 2 loaded throughout):
  ABeautifulGame centre 20 % param 150.3 ms (run 1) / 507.6 ms (run 2, loaded); package A's exe at 18:17 (other cargo
  running) 240 ms. 100 %: 2.10 s / 7.25 s (loaded). Off-centre (a piece): 20 % 976 / 900 ms, 100 % 3.07 / 3.89 s.
  Jump on, centre 20 %: 400.7 / 1,252.5 ms = 2.67 / 2.47 x Jump off (limit 1.5 x), jumps 3. Overlap 2.5 turns: 20.9 ms vs
  limit 123 ms; 201.7 vs 377 (loaded); turns 3. Wrap Full / Preview vs projected: met in every Jump-off row.
- G6 (`analysis\g6.txt`): 13 / 13 ok, 0 panics, 0 ERROR lines (table in the report). Loaded: Lantern (end),
  ABeautifulGame, ngons_tiling (another agent's cargo build); waits of 300 / 300 / 165 / 90 / 165 s before some runs.
- G7 (`runs.csv`): ABeautifulGame `workflow_path` peak working set Wrap 1,834 / 2,048 MB vs Project 2,064 / 2,048 MB;
  peak private bytes 3,383 / 3,386 vs 3,284 / 3,431 MB. Met (+99 MB at most).
- G8 (`analysis\g8.txt`): both runs ok (203 / 201 s), 0 panicked / Invalid parameter / Unknown layer, 0 central /
  dispatch / pointer frames over 500 ms (0 over 300 ms); 42 wrap bakes per run. Met.

## Tests after the changes

- `cargo test --workspace --no-fail-fast` (19:26-19:47; nothing else at the start, another agent's cargo at the end;
  free 0.85 GB): **1,107 passed, 0 failed, 33 ignored** (wte_app 342 / 16, wte_core lib 718 / 17, brush_tips 15,
  end_to_end 9, project_roundtrip 23). Log `runs_gate2\build_logs\test1.log`. WA19's retry was not needed (no failure).
- `cargo clippy --workspace --all-targets`: 0 warnings, 0 errors (2 min 57 s; `clippy1.log`).
- `cargo build --release -p wte_app`: ok, 4 min 00 s (`release2.log`); made before the test run, no source change
  after it. Exe SHA-256 6CE65B55F15861BF91AF2DA5FDFB493482F8F050CB51B801D042518E57FD5433.
- WA15 probe release builds: `probe_build*.log` (3 min 35 s to 7 min 57 s, partly with other agents' builds running).
- Check: `text_warp` on the built-in sample with the final exe: ok; `shot.png` and `shot_warp3d.png` 0 pixels differ
  from gate 1's run (`runs_gate2\check_final\`).

## Not done / open

1. No product-code fix: G5c, G5e and G5f (parameterization cost on ABeautifulGame) and the quality on strongly curved
   models (WA4 family: high_poly_sphere, klein_bottle, torus_knot) have no small, contained fix.
2. G5a's run-1 first-bake outlier (49.7 ms first parameterization) is explained only by inference (cold code pages).
3. No app screenshot of a corner, and none of DamagedHelmet with Jump off (the scenario's last state is Jump on).
4. `text_path`'s bounds check would fail the same way in Wrap on a model whose text spreads over the whole set; not
   run, not changed.
5. Side panel: the "Longer than the way round" buttons are below the visible part of the options box in the G6 shots
   (the box scrolls), and the 306-pt panel width after reopening with a text selected (check:B) is still open.
6. No request for files I do not own.
