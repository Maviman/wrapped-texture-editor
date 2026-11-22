# Round 7 gate report — Wrapped Texture Editor ("brush speed")

Date 2026-11-21. Plan: `target\round7_design\ROUND7_PLAN.md`. Notes: `notes\pkg.md` (builder), `notes\check.md` (check
and fix), `notes\gate.md` (this gate). Gate scripts, logs and analysis files: `target\round7\gate\`. Raw runs:
`target\round7\runs\gate\`.

## Verdict

**Round 7 does what it was for in kind, but not every number in the plan was reached. The pixels are exactly round
5's. On the chess set, the brush now does about half the work at the default size and about a third of it at Size
512. The two hardest lines were not met: the backlog limit (P4) and release to end at Size 512 (P6).**

- Pixels: all 38 sweep scenarios run. 35 shots are identical to round 5's. The 3 other shots differ only in toast
  timing or in a folder name shown in the export dialog. The brush shots are byte-identical with 1, 2 and 12 threads.
  All 54 bench hash checks are identical.
- ABeautifulGame, Size 50: no frame of stroke work is longer than 122 ms (round 5 in the same session: up to 199 ms,
  plus one stall of 1.25 s). Dabs are 28-44 ms (round 5: 53-164 ms). Stroke work is 0.42-0.56 x round 5. The paint
  still trails the pointer by 0.53-0.84 s at most (round 5: 1.0-2.6 s), so the 350-ms backlog target is not met.
- ABeautifulGame, Size 512: painting stops 2.1-3.8 s after release (round 5: 6.6-10.9 s). Stroke work is 0.29-0.38 x.
  Two of the three strokes meet the 3.0-s target in some runs. The Sphere stroke misses it in every run.
- Light models and the core bench are faster on every row, by 0.29-0.92 x. Nothing is slower.
- The fuzz run is clean.

**Measuring conditions.** Every measurement ran when the machine was idle by the plan's rule. There was no build
lock, no cargo or rustc, and no other wte exe, and no run had to wait. The background apps (bambu-studio, Discord,
Steam) used about 0.1-0.2 of a core. Free memory was the problem: during every ABeautifulGame run of **both** exes
it fell to 3-170 MB. The exe itself causes this pressure on a 7-GB machine. Stalls from paging (see P3) can happen in
either exe. The load of each run is in `gate\load_per_run.txt` and `gate\load_summary.txt`.

## Exe and tests

- Exe: `target\round7\wte_round7.exe`, **SHA-256 EBBB49604A96F03278CAC864E83AD72BDD969FD67A451E354F6B7C40516F1CA9**.
  It is byte-identical to the check agent's `runs\check\wte_check.exe`. It was built from `target\round7\work\merged`,
  a robocopy of the checked `work\pkg`. Nothing was changed in the gate.
- `cargo test --workspace` (merged): **1,076 passed, 32 ignored, 0 failed**: app 332 (16 ignored), core 699 (16
  ignored), brush_tips 15, end_to_end 9, project_roundtrip 21. Round 5 had 1,067 / 32. T1-T6,
  `folded_rows_are_the_rows_in_range` and the check agent's two adversarial tests all pass. Log `gate\test_ws.log`.
- `cargo clippy --workspace --all-targets`: 0 warnings (`gate\clippy.log`). There is no `todo!` or `unimplemented!`.
  The release build is ok (`gate\release.log`). The release core test exe for the bench is
  `gate\wte_core_bench_gate.exe`.
- The build lock was used for every compile step (`gate\lock_events.txt`). The gate waited once for a round-6 rustc
  while free memory was under 1.5 GB, and once for a lock held by `main-verify`.

## Acceptance lines

"r5" means `target\round5\wte_round5.exe` (SHA 5DA4E165...6568), measured in this session back to back, alternating
r5_1, r7_1, r5_2, r7_2. Size-50 numbers are for the three strokes after the warm-up stroke (Circular, Sphere,
Sphere/Cube). File names below are under `target\round7\`.

| # | Line | Round 7 | Round 5 (this session; gate-report value) | Verdict | From |
|---|---|---|---|---|---|
| P1 | Tests pass, T1-T6 pass, clippy 0 | 1,076 passed / 32 ignored / 0 failed; T1-T6 ok; clippy 0 | 1,067 / 32 | **met** | `gate\test_ws.log`, `gate\clippy.log` |
| P2 | Pixels identical: 38 sweep shots, `RAYON_NUM_THREADS=1` / `2`, bench r7 hash == r5 hash | 38 / 38 ok. 35 shots pixel-identical. `final_bake_async` and `workflow_path` differ in the toast area only (toast stack timing). `export` differs only in the dialog's output-folder text (it shows the run folder name: "...runs\..." against "...eep\..."). `brush_types` with 1 / 2 / 12 threads: byte-identical to each other and to round 5's shot. Bench: 54 of 54 hash lines identical | | **met** | `gate\shot_cmp_sweep.txt`, `gate\export_diff_crop.png`, `gate\toast_*.png`, `runs\gate\threads\`, `gate\bench_abc_u8.out` |
| P3 (B4) | ABG Size 50, both runs: longest frame of stroke work ≤ 140 ms, 0 drag frames over 140 ms, longest dab ≤ 40 ms | Longest frame: run 1 114 / 119 / 122 ms, run 2 115 / 116 / 114 ms (probe run 127 / 121 / 108). Drag frames over 140 ms: run 1 0 / 0 / 0, run 2 0 / **1** / 0 (probe run 0). Longest dab: run 1 30 / 36 / 29 ms, run 2 35 / **44** / 31 ms (probe run 34 / **44** / 28) | lf 127-199 ms (plus 1,252 in r5_1 Sphere/Cube); 0-3 drag frames over 140; dabs 53-164 ms (plus a 1,242-ms dab). Report: 123-156 ms; 0-4; 47-81 ms | **met in run 1; partly met in run 2.** The longest-frame part is met in every run. Run 2 has one 498-ms frame whose own UI work was 117 ms; it came at a moment when free memory was 170 MB. The Sphere stroke's longest dab is 44 ms | `gate\btp_cmp_all.txt`, `runs\gate\btp\..._r7_2\shot.log` line 471, `gate\load_samples.txt` 20:15:01-07 |
| P4 (B6) | Same strokes: largest backlog ≤ 350 ms, both runs | Run 1 580 / 667 / 608 ms, run 2 708 / 842 / 565 ms (probe 724 / 1,010 / 528) | 1,016-2,588 ms. Report: 760-1,683 ms | **not met** (about 0.4 x round 5) | `gate\btp_cmp_all.txt` |
| P5 | Same strokes: stroke work ≤ 0.50 x round 5, each stroke, 2 pairs | Pair 1: 0.513 / 0.461 / 0.312. Pair 2: 0.421 / 0.504 / 0.563. Round 7: 912-1,137 ms per stroke | 1,667-2,713 ms per stroke | **not met**: 3 of 6 strokes are at or under 0.50 x and 3 are 0.504-0.563 x. The 0.312 is flattered by a 1.24-s stall in r5_1 | `gate\btp_cmp_all.txt` |
| P6 (B5 / P8b) | ABG Size 512: release to end ≤ 3.0 s on each stroke, both runs; stroke work ≤ 0.50 x | Release to end: run 1 **3.27** / **3.78** / 2.54 s, run 2 2.81 / **3.55** / 2.63 s (probe 2.14 / **3.60** / 2.95 s). Stroke work 0.288 / 0.302 / 0.372 and 0.305 / 0.300 / 0.377 x | Release to end 6.62-10.94 s. Report: 6.7-11.9 s | **partly met**: stroke work is met everywhere. Release to end is met on Sphere/Cube in all runs and on Circular in run 2 and the probe run. Sphere 512 is 3.55-3.78 s in every run | `gate\btp_cmp_all.txt` |
| P7 (B5) | Size 50: release to end ≤ 400 ms; pointer step ≤ 1.10 x round 5 | Release to end 0 ms on every Size-50 stroke. Pointer step 0.39-0.74 x (warm-up 0.64-0.71) | Release to end 3-53 ms. Report: 2-14 ms | **met** | `gate\btp_cmp_all.txt` |
| P8 | Light models: stroke work ≤ 1.05 x round 5 (min of the runs); bench rows a and c ≤ 1.03 x the r5 row | tex_4096 0.555-0.840, ngons_tiling 0.797-0.904, tex_8192 0.453-0.921 (1 pair). Bench model a 0.37-0.77, model c 0.47-0.81 (model b 0.29-0.51) | | **met** | `gate\btp_cmp_all.txt`, `gate\bench_ratio.txt` |
| P9 | `workflow_path` ABG: every step within 15 % of round 5 or faster; the three brush strokes reported (expected ≤ 0.6 x) | Sum of steps 0.97 (min) / 0.91 (median). Steps over 1.15 x by min: open / import 1.18, 3D view and frame 1.21 (117-170 against 141-183 ms), export every format 1.16. By median, every step is within 1.13. Brush strokes 0.55 / 0.77 / 0.48 (pair 1) and 0.62 / 0.70 / 0.58 (pair 2); the "3D brush strokes" step is 0.61-0.71 x | Strokes 1,868-2,099 / 366-383 / 2,189-2,445 ms | **met by the median of 2 pairs; not met by the min on three non-brush steps.** None of those steps runs round-7 code (the brush index is built on the first stroke, not on import). Their spread is within round 5's own spread (round 5's gate, all exes: open / import 3,954-6,864 ms, 3D view and frame 133-315 ms, export 11.3-59.7 s). Strokes 1 and 3 are near 0.5-0.6 x. The short stroke 2 is 0.70-0.77 x | `gate\steps_abg.txt` |
| P10 (B7) | Fuzz seed 7: no `pointer/tools/hover` frame over 500 ms; 0 "panicked" / "Invalid parameter" | 0 frames over 500 ms (and 0 over 300 ms) in central / dispatch / pointer-tools-hover; 0 errors; ok | 0 | **met** | `gate\fuzz.txt` |

## Phase split of the heavy brush strokes

Source: `WTE_STROKE_LOG=1`, ABeautifulGame, `brush_types_path`, round-7 probe run (`gate\phase_r7_probe.txt`, from
`runs\gate\btp\brush_types_path_glb_samples_ABeautifulGame.glb_r7_probe\shot.log`). The table uses round 5's format
plus slots 10-14. The percentages are shares of stroke work. The pool has 12 threads, so band work / raster is the
effective parallel width.

| Stroke | Stroke work | Raster | Commit | Re-apply | Query | Setup | First touch ms | Texels touched | Pass-2 texels (x touched) | Pass-2 skipped | Bands | Band work (/ raster) | Candidates |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| warm-up 50 | 1,327 ms | 688 (52 %) | 11 (1 %) | 263 (20 %) | 123 (9 %) | 155 (12 %) | 84.2 | 19.99 M | 59.3 M (3.0 x) | 510,088 | 4,099 | 4,176 ms (6.1) | 332,570 |
| Circular 50 | 974 ms | 596 (61 %) | 10 (1 %) | 163 (17 %) | 93 (10 %) | 109 (11 %) | 0 | 19.99 M | 59.3 M (3.0 x) | 510,088 | 4,099 | 3,773 ms (6.3) | 332,570 |
| Sphere 50 | 1,126 ms | 748 (66 %) | 11 (1 %) | 168 (15 %) | 68 (6 %) | 129 (11 %) | 0 | 25.67 M | 89.4 M (3.5 x) | 677,489 | 4,570 | 5,073 ms (6.8) | 454,112 |
| Sphere/Cube 50 | 862 ms | 605 (70 %) | 8 (1 %) | 82 (9 %) | 85 (10 %) | 80 (9 %) | 0 | 11.57 M | 46.5 M (4.0 x) | 331,780 | 3,353 | 3,952 ms (6.5) | 223,977 |
| Circular 512 | 13,455 ms | 9,588 (71 %) | 32 (0 %) | 518 (4 %) | 1,777 (13 %) | 1,500 (11 %) | 37.3 | 341.3 M | 1,283 M (3.8 x) | 15.9 M | 14,548 | 89,628 ms (9.3) | 9.85 M |
| Sphere 512 | 21,259 ms | 16,630 (78 %) | 35 (0 %) | 483 (2 %) | 1,587 (7 %) | 2,521 (12 %) | 0.1 | 557.3 M | 2,584 M (4.6 x) | 24.8 M | 16,187 | 163,447 ms (9.8) | 16.03 M |
| Sphere/Cube 512 | 18,238 ms | 14,625 (80 %) | 29 (0 %) | 446 (2 %) | 1,557 (9 %) | 1,578 (9 %) | 0.1 | 371.6 M | 1,452 M (3.9 x) | 13.6 M | 12,644 | 148,542 ms (10.2) | 8.85 M |

The same strokes with round-5 code: round 5's gate-report table, and the builder's step-0 run of the same day
(`notes\pkg.md`, round-5 code plus the new slots).

| Stroke | Round 5 gate report | Step 0 (round-5 code, same day) |
|---|---|---|
| Circular 50 | 1,736 ms: raster 1,188 (68 %), commit 219 (13 %), re-apply 166 (10 %), query 85 (5 %), setup 76 (4 %) | 1,488 ms: raster 1,016, commit 184, re-apply 143, query 77, setup 65; pass-2 texels 15.4 x touched; band work / raster 5.3 |
| Sphere 50 | 2,506 ms: raster 1,761 (70 %), commit 316, re-apply 241, query 75, setup 109 | 1,883 ms; pass-2 16.1 x |
| Size 512 | raster 85-90 % of 44-67 s | raster 86-90 %; pass-2 21-26 x; band work / raster 9.5-9.7 |

What changed, and what remains:
- **Commit** dropped from 13 % (184-219 ms) to 1 % (10 ms). It now runs inside the bands.
- **Pass 2** looks at 3.0-4.6 x the touched texels, down from 15-26 x. Lean pass 2 skips 510 k-25 M calls per stroke.
- **Raster** fell from about 1,016-1,188 ms to 596 ms on the Circular stroke. It is still 61-80 % of the work. The
  remaining pass-2 work comes from triangles on island borders and near shared gutters, which still scan their whole
  box. At Size 50 the dab's bands keep about 6.3 of the 12 threads busy.
- **Re-apply** (A4 spans) stayed at about 150-170 ms at Size 50. On the chess pieces a Size-50 dab raises most texels
  of its rect, so the spans save little there. It is now 15-17 %, the second-largest share.
- **Setup and query** are 10-11 % each at Size 50. At Size 512, A3 brought query from 2.9 s (v1) down to 1.6-1.8 s per
  stroke.
- The touched-texel counts per stroke equal the step-0 run's (round-5 code) exactly.

## What was not met, and why

- **P4, backlog ≤ 350 ms (565-1,010 ms).** The backlog grows with the work per sample times the samples that queue.
  Stroke work halved, and the backlog roughly halved with it (round 5 in this session: 1,016-2,588 ms). The plan
  itself named P4 "the line most at risk". Reaching 350 ms needs about 4-5 x cheaper Size-50 dabs, not 2 x. Following
  plan 14.3, nothing was dropped or merged to make the number, because that would change pixels.
- **P5, ≤ 0.50 x per stroke (0.42-0.56).** On average it is about 0.5 x; three of six strokes land just over the line.
  What is left at Size 50 is raster (61-70 %), re-apply (15-17 %), setup (11 %) and query (6-10 %). The plan's estimate
  was 2.5-3 x cheaper dabs; the measured gain is about 2 x. The difference is pass 2 on border triangles (3.0-4.0 x the
  touched texels) and a re-apply that A4 barely shrinks on this model.
- **P6, release to end ≤ 3.0 s (2.1-3.8 s).** Release to end ≈ the dabs still queued × the time of one dab. A Size-512
  dab now takes 0.46-0.75 s (round 5: 1.3-2.2 s), which is 3 x faster, as the plan estimated. The Sphere stroke touches
  557 M texels, 1.6 x more than the Circular stroke, and its last dabs alone take 3.6 s. The plan's own estimate was
  "about 1.5-3 s". Sphere 512 lands just above that.
- **P3, run 2.** In run 2 one drag frame took 498 ms, although its own UI work was 117 ms. The longest dab of the
  Sphere stroke is 44 ms in run 2 and in the probe run (limit 40). Both happened while free memory was 70-170 MB. Run 1
  meets every part of P3.
- **P9 by min.** Three non-brush steps are 1.16-1.21 x by the min of two runs and within 1.13 x by the median. They are
  inside round 5's own run-to-run spread and do not run any changed code (details in the table).
- Plan section 4 says the pool has 6 threads. This machine has 6 cores and 12 logical processors, so the global rayon
  pool has 12 threads. Nothing depends on the count: band sizes and thresholds are constants, and the shots are
  identical with 1, 2 and 12 threads.

Possible next step (outside this plan, not built): in the lean pass 2, walk only each triangle's own gutter texels
when no shared block meets its box. That would need `gutter_by_tri` built in `prepare_brush_index`. It targets the
remaining 3-4 x pass-2 work, which is the largest piece of raster.

## Anything cut

**Nothing was cut.** Every run of plan 9.3 was done. That covers ABG 2 pairs plus the probe run; tex_4096 and
ngons_tiling 2 pairs each; tex_8192 1 pair; `workflow_path` 1 pair; fuzz seed 7; and the bench for models a, b and c.
The gate added one run beyond the plan: a second `workflow_path` pair, because the first pair left two non-brush steps
over 1.15 x. That is still at most 2 runs per exe. A3 and A4 were both built, as their rules required. The gate changed
no code.

## Re-test list after merging onto round 6 (plan section 11)

1. Three-way merge: base `target\round6\work\base`, ours `target\round7\work\merged`, theirs = the main tree after
   round 6. Round 7 changed only `crates\wte_core\src\{texel_map.rs, brush.rs, stroke.rs, bvh.rs}` and
   `crates\wte_app\src\tools\brush.rs` (`phase_text`). If round 6 touched any of these files, merge them by hand and
   say so. In `ARCHITECTURE.md`, round 7's hunks are in 4.6, 5.9, 5.19, 6.3 and 6.4. Paste the 1.7 paragraph and the
   9.1 lines from `notes\pkg.md`, plus the check agent's 9.1 lines from `notes\check.md`.
2. `cargo test --workspace`. Round 6's wrap tests run through `raster_tri`, which lives in `texel_map.rs`, a file round
   7 changed. Then clippy with 0 warnings, and a release build.
3. Scenarios at 1280 x 800:
   - `brush_types`, also with `RAYON_NUM_THREADS=1`. `shot.png` must be identical to this gate's (SHA-256
     F387E374...7680, `runs\gate\sweep\brush_types_builtin\shot.png`).
   - `brush_types_path` on ABeautifulGame, one run with `WTE_STROKE_LOG=1`. Stroke work must be within 15 % of this
     gate's probe run: 974 / 1,126 / 862 ms at Size 50, and 13.5 / 21.3 / 18.2 s at Size 512.
   - `workflow_path`, `wrap_surface`, `final_bake_async`, `stamp`.

## What the user should try by hand

1. Open ABeautifulGame. Make a fast stroke across the chess pieces at the default size. The view should stay
   responsive. The paint should follow about twice as closely as in round 5, but it can still lag by about half a
   second on a fast stroke.
2. Set Size 512 and stroke across several pieces. Note how long it keeps painting after you let go: about 2-4 s now,
   against 7-11 s in round 5. Say whether that is good enough (plan question 14.4: 3 s now, or a later round aimed at
   1 s, likely on the GPU).
3. Compare a stroke on a light model (any normal model) with round 5. It should feel the same or snappier.
4. Paint a stroke right after placing an image stamp on ABeautifulGame, while it still shows "Finishing stamp". The
   first dabs may be slower while the bake finishes. This was accepted in plan section 4 and was not measured.
5. Spot-check that undo and redo of brush strokes give exactly what was painted. The tests prove that the pixels equal
   round 5's, but a look on the real model does not hurt.

## Leftovers that can be deleted

- `target\round7\step0_src` and `target\round7\a12_src` (stale).
- `runs\check\wf_abg.wtep` (119 MB) and `runs\v3\wf_abg.wtep` (119 MB).
- The scenario outputs under `runs\gate\abg_wf\*\` (saved projects and exports).
- `work\pkg\target` (11.5 GB), once `merged` is landed. `work\merged` is the landing source; keep it until round 7
  lands.
