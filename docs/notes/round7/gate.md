# Round 7 gate (brush speed): notes

Agent: round7-gate (Opus), 2026-11-21. Copy: `target\round7\work\merged` = robocopy of the checked `work\pkg` (with its
`target\`). Plan: `target\round7_design\ROUND7_PLAN.md` section 9. Gate files: `target\round7\gate\` (scripts, logs,
analysis), runs under `target\round7\runs\gate\`.

## 1. Copy, tests, clippy, release

- 19:19 start. Machine at start: `wte_round6.exe` (round 6's gate, ngons_tiling) running, free memory 1.48 GB,
  `BUILD_LOCK` absent. Robocopy `pkg` -> `merged` (log `gate\copy_merged.log`).
- Copy done 19:36 (12,448 files, 11.55 GB, robocopy exit 1 = copied, 0 failed).
- No `todo!` / `unimplemented!` in `merged\crates` (grep: 0 hits).
- `test --workspace --no-run` under the lock: waited for round 6 first (a round-6 `rustc` with 684 MB free at 19:36:48,
  3-min waits), lock taken about 19:45, released 19:48 (3 min 13 s compile; the lock-event lines of that run were lost:
  my own `tail -f` held the event file, fixed). Tests ran without the lock.
- `cargo test --workspace`: **1,076 passed, 32 ignored, 0 failed** (app 332 / 16 ignored, core 699 / 16, brush_tips 15,
  end_to_end 9, project_roundtrip 21). T1-T6, `folded_rows_are_the_rows_in_range` and the check agent's two tests: ok.
  Log `gate\test_ws.log`.
- Lock events `gate\lock_events.txt`: lock held by `main-verify 2026-11-21 19:51` at 19:52, waited 2 min; clippy
  19:54:37-19:54:54, release build 19:54:55-19:56:19, release core test exe 19:56:20-20:00:50.
- `cargo clippy --workspace --all-targets`: **0 warnings** (`gate\clippy.log`). Release build ok (`gate\release.log`).
- Exe `target\round7\wte_round7.exe`, **SHA-256 EBBB49604A96F03278CAC864E83AD72BDD969FD67A451E354F6B7C40516F1CA9**
  (byte-identical to the check agent's `runs\check\wte_check.exe`). Release core test exe for the bench:
  `gate\wte_core_bench_gate.exe` (SHA-256 4A62F504...0CAD).

## 2. Pixel sweep

- 20:02 sweep of round 5's 38-name job list (`gate\jobs_all.txt` = `round5\runs_gate\jobs_all.txt`), `-Parallel 1`,
  1280 x 800, runs `runs\gate\sweep\`, csv `gate\sweep_all.csv`.
- Result: **38 of 38 ok** (`gate\sweep_all.out`). `gate\shot_cmp.py` (RGBA, whole image) against
  `round5\runs_gate\runs\` (`gate\shot_cmp_sweep.txt`): **35 identical**; `final_bake_async` and `workflow_path`
  (DamagedHelmet) differ only in the toast area (toast stack timing: a "Saved shot_fba.wtep" / "Redo: Edit Stamp (x11)"
  toast present in one and not the other; crops `gate\toast_final_bake.png`, `gate\toast_workflow_p.png`); `export`
  differs in 1,172 px at x 304-490 y 153-164 = the export dialog's output-folder text, which shows the run folder
  ("...runs\export_builtin\shot_export" vs "...eep\export_builtin\shot_export"; crop `gate\export_diff_crop.png`): a
  path artefact of the folder name, not a rendering change. Every brush scenario shot (brush_types, brush_shape,
  brush_tab, brush_hover, brush_types_path tex_4096, paint3d, paint2d, fuzz_path Avocado) is pixel-identical.
  No round-5 exe run was needed (every name exists in round 5's sweep). Round 5's `final_bake_async_builtin_sync` and its
  job-file slip `import_save_glb_samples_Avocado.glb` have no round-7 counterpart (not in the 38-name list).
- `brush_types` with `RAYON_NUM_THREADS=1` and `=2` (`runs\gate\threads\`): ok; both `shot.png` byte-identical
  (SHA-256 F387E374...7680) to the default-pool sweep shot and to round 5's sweep shot. Read the shot: the cube face is
  painted red under the rotated Cube tip outline (Size 600 px), layers Paint 1 / Base, toast "Undo: Brush Stroke",
  status bar "Shape brush: Cube": as round 5.

## 3. Measurements (plan 9.3)

- Driver `gate\driver.ps1` (round 5's driver pattern: `sweep.ps1` one job at a time, `-Parallel 1`, 1280 x 800), log
  `gate\driver.log`; per-run load `gate\load_per_run.txt` (top-5 CPU processes over 3 s before and after each run, free
  memory, busy processes) and `gate\load_samples.txt` (5-s sampler); summary `gate\load_summary.txt`
  (`gate\load_windows.py`). Idle rule checked before every run (no `BUILD_LOCK`, no cargo / rustc, no other wte /
  wrapped_texture_editor exe; 5-min re-checks, 3-h budget): **the machine was idle by that rule for every run; no run
  waited, none is marked loaded** (`gate\waited_total.txt` = 0). Background on every run: bambu-studio, Discord, Steam
  (0.1-0.2 core on average, peaks 94 % total CPU during the ABG runs = our own exe). Free memory fell to 3-170 MB during
  every ABeautifulGame run of both exes (7 GB machine; the exe itself is the pressure); light-model runs 418-1,365 MB.
- Runs 20:06-20:35: ABG `brush_types_path` r5_1, r7_1, r5_2, r7_2, r7_probe (`WTE_STROKE_LOG=1`); tex_4096 r5/r7 x 2,
  ngons_tiling r5/r7 x 2, tex_8192 r5/r7 x 1; `workflow_path` ABG r5_1, r7_1; fuzz seed 7 ngons r7_1; bench; then a
  second `workflow_path` pair r7_2, r5_2 (added: the first pair had two non-brush steps over 1.15 x). All runs ok.
  Runs under `runs\gate\{btp,abg_wf,fuzz}`.
- Analysis: `gate\btp_cmp7.py` (round 5's `btp_cmp.py`, r7 vs r5) -> `gate\btp_cmp_all.txt`; `gate\phase.py` ->
  `gate\phase_r7_probe.txt`; `gate\steps7.py` -> `gate\steps_abg.txt`; round 5's `fuzzframes.py` -> `gate\fuzz.txt`;
  bench `gate\bench_abc.out` (UTF-16; `bench_abc_u8.out`), rows `gate\bench_abc_rows.txt`, ratios `gate\bench_ratio.txt`.
- ABG Size 50 after the warm-up (strokes Circular, Sphere, Sphere/Cube), r7_1 / r7_2 / probe: longest frame of stroke
  work 114-122 / 114-116 / 108-127 ms; drag frames over 140 ms 0 / 1 / 0 (r7_2 Sphere: one frame of 498 ms interval with
  117 ms UI CPU, at 20:15:06 when free memory was 170 MB); longest dab 30-36 / 31-44 / 28-44 ms; largest backlog
  580-667 / 565-842 / 528-1,010 ms; release to end 0 ms. Round 5 same session: lf 121-1,252 ms, 0-3 frames over 140,
  dabs 53-1,242 ms (r5_1 Sphere/Cube had one 1.24-s dab), backlog 1,016-2,588 ms.
  Stroke work r7/r5: pair 1 0.513 / 0.461 / 0.312, pair 2 0.421 / 0.504 / 0.563. Pointer step r7/r5 0.39-0.74.
- ABG Size 512: release to end r7 3.27 / 3.78 / 2.54 s and 2.81 / 3.55 / 2.63 s (probe 2.14 / 3.60 / 2.95 s); round 5
  6.95 / 10.84 / 6.62 and 6.91 / 10.94 / 6.63 s. Stroke work r7/r5 0.288-0.377. Longest dab 464-750 ms (round 5
  1,320-2,197).
- Light models (min r7 / min r5 stroke work per stroke): tex_4096 0.555-0.840, ngons_tiling 0.797-0.904, tex_8192
  0.453-0.921. Pointer steps 0.44-1.25 (the 1.25 = r5_2 vs r7_2 on ngons Sphere/Cube: r7_2 had one 226-ms frame).
- Bench (one run, 360 s): 54 hash lines (C1 + P2 r7 == r5), **all identical**. r7 / r5 ms per dab: model a 0.37-0.77,
  model b (ABG) 0.29-0.51, model c 0.47-0.81. Every row at or below its r5 row.
- `workflow_path` ABG (2 pairs): sum of steps r7/r5 0.97 (min) / 0.91 (median). Steps over 1.15 x by min: open / import
  1.18 (r5 5,037 / 6,602, r7 5,959 / 5,991 ms; median 1.03), 3D view and frame 1.21 (117 / 170 vs 141 / 183 ms; median
  1.13), export every format 1.16 (10,455 / 9,217 vs 10,876 / 10,678 ms; median 1.10). By median every step is within
  1.13. Brush strokes r7/r5: 0.55 / 0.77 / 0.48 and 0.62 / 0.70 / 0.58; "3D brush strokes" step 0.61-0.71.
- Fuzz seed 7 ngons, 1,000 steps: 0 frames over 500 ms (0 over 300 ms) in central / dispatch / pointer-tools-hover;
  0 "panicked" / "Invalid parameter"; ok.
- Probe phase split (r7_probe), Circular 50 after the warm-up: 974 ms = raster 596 (61 %), re-apply 163 (17 %), setup
  109 (11 %), query 93 (10 %), commit 10 (1 %); pass-2 texels 3.0 x touched; band work / raster 6.3. Size 512: raster
  71-80 %, query 7-13 %, setup 9-12 %. Touched texels per stroke equal round 5's step-0 counts exactly.
- 20:36 sampler stopped. No lock held at any measuring time.

## 4. Report

- `target\round7\GATE_REPORT.md` written. Verdict: pixel-exact speed-up, about 0.5 x stroke work at Size 50 and about
  0.3 x at Size 512 on ABeautifulGame. P1, P2, P7, P8, P10 met. P3 met in run 1 and partly in run 2. P4 not met
  (565-1,010 ms). P5 not met (0.42-0.56 x, 3 of 6 strokes at or under 0.50 x). P6 partly met (stroke work met; release
  to end 2.1-3.8 s, Sphere 512 over 3.0 s in every run). P9 met by median, not by min on three non-brush steps. Nothing
  cut; no code changed in the gate.
