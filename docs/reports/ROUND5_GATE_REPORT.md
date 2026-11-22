# Round 5 gate report — Wrapped Texture Editor ("smoothness")

Date 2026-11-21. Plan: `target\round5_design\ROUND5_PLAN.md`. Notes: `notes\gate1.md` (merge, tests, scenarios, nothing
got worse), `notes\gate2.md` (measurements). Raw runs: `runs_gate\` (stage 1), `runs_gate2\` (stage 2; the analysis
files named below are in `runs_gate2\analysis\`).

## Verdict

**Round 5 does what it was for on the stamp and text side, and the font picker item is closed. The brush item is only
partly done.**

- Placing, moving, scaling, rotating and warping a stamp or text no longer freezes the app. On ABeautifulGame the
  1.6-second freeze when an image stamp is placed is gone: the frames stay under 100 ms while the worker bakes, and the
  sharp picture appears 1.4-1.7 s later. Warp edits that froze for 0.6-2.6 s in round 4 now leave every frame under
  300 ms. Pixels, exported files and saved projects are the same as round 4.
- The brush on the 1.5-million-triangle model is **not slower per stroke than round 3 or round 4**. Frames during a
  heavy stroke are shorter (no frame longer than 100 ms plus one dab). But the 140-ms cap and the backlog limit (B6) are
  not met. At Size 512 on that model the stroke now keeps painting for 7-12 s after the mouse is released. In round 4
  the app froze for 5-9 s per frame instead.
- The font picker opens in 1.4-2.1 ms side-panel frames (limit 16 ms).

Lines not met: A5 for one bake (text rotate on ngons_tiling, 1.31 / 1.40 x), plus two bakes in a run under heavy
unrelated load. A4 is met for the Final bakes, but the Preview frames during a drag (also in round 4) are still over
150 ms. B4 (the 140-ms cap) is met in 1 of 2 runs. B5 is broken at Size 512 (release to end). B6 is not met. B3 is met
on ABeautifulGame. It cannot be decided on tex_8192, because those runs had heavy unrelated load. No acceptance line
failed for a small, contained cause, so no code was changed in the gate.

**Measuring conditions.** The machine was never idle. An unrelated project's smoke-test server (Python, not ours)
used up to 3.5 cores in bursts during all of stage 2. Free memory often fell below 100 MB while our exe (2.0-2.8 GB)
ran beside it. Each run's load is in `analysis\load_per_run.txt`. Lines marked "loaded" below are indicative only.

## Exe and tests

- Final exe: `target\round5\wte_round5.exe`, **SHA-256 5DA4E1655152922D4E0E44807ECFBDDD608A1D5A6413DFB0C3DF17B125476568**.
  It is unchanged since stage 1, because no fix was needed.
- `cargo test --workspace` (merged): **1,067 passed, 32 ignored, 0 failed** (app 332 / 16 ignored, core 690 / 16, brush_tips
  15, end_to_end 9, project_roundtrip 21). Round 4: 1,040 / 32. `clippy --workspace --all-targets`: 0 warnings. There is
  no `todo!` or `unimplemented!`. Logs: `runs_gate\build_logs\test1.log`, `clippy1.log`, `release1.log`.

## Acceptance lines

"r4" = `target\round4\wte_round4.exe` measured in the same session. "P0" = `wte_p0_instrumented.exe` (round-3 code).

### R5-1: Final stamp bakes off the UI thread

| # | Line | Round 5 | Round 4 | Verdict | From |
|---|---|---|---|---|---|
| A1 | Tests of 1.7 pass, round-4 tests unchanged, clippy clean | 1,067 / 0 failed; clippy 0 | 1,040 | **met** | `runs_gate\build_logs\test1.log`, `clippy1.log` |
| A2 | Pixels equal round 4 | `stamp`, `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run`: shot.png pixel-identical. Exports byte-identical. `final_bake_async` passes with and without `WTE_SYNC_FINAL_BAKES=1`, and the two runs are equal | | **met** | `runs_gate\r4_builtin_compare.txt`, `fba_compare.txt` |
| A3 | ABeautifulGame `workflow_path`: no frame over 300 ms between `started` and `applied`; Final frame dispatch+central ≤ 60 ms | Longest frame between started and applied: image 38 / 91 ms, text 19 / 17 ms, transform ≤ 34 ms. Final frames 0.8-14.1 ms. The image placement's click frame (reported apart) is 3.5 / 10.4 ms | Image click frame 1,652 / 1,647 ms; release frames 58-101 ms | **met** (2 runs) | `analysis\bakes_abg.txt` |
| A4 | ngons_tiling: no frame over 150 ms in the three stamp steps | Final-related frames: Final frames ≤ 29.5 ms, longest frame while baking 33 ms. Still over 150 ms: 2 / 3 frames, all throttled **Preview** bakes during the drag (scene sync 149-198 ms) | 9 / 8 frames over 150 ms: release / click frames 148-394 ms, plus the same Preview frames (150-174 ms) | **met for the Final bakes; the line as written is not met** because of the Preview frames, which round 4 has too | `analysis\bakes_ngons.txt` |
| A5 | Final-to-swap (`applied after`) ≤ 1.3 x round 4's Final frame, back to back | ABeautifulGame pair 1: image 0.84, text 0.61, move 0.85, scale 0.98, rotate 1.03. Pair 2 (loaded): 1.03 / 0.90 / 1.26 / **1.97** / **2.23**. ngons: image 0.93 / 0.95, text 0.97 / 1.02, move 1.01 / 1.18, scale 1.08 / 1.17, rotate **1.31 / 1.40** | ABG: image 1,652 / 1,647, text 41 / 39, move 78 / 58, scale 101 / 96, rotate 87 / 82 ms. ngons: 392 / 349, 156 / 149, 155 / 154, 225 / 234, 293 / 287 ms | **partly met** (16 of 20 bakes) | `analysis\bakes_abg.txt`, `bakes_ngons.txt` |
| A6 | `text_warp` on ABeautifulGame: 0 frames over 300 ms around the Finals | 0 in both runs. Longest frame between a Final and its swap: 275 / 71 ms. Final frames ≤ 41 ms. Time to swap: 3D 629-1,075 ms, 2D 7-33 ms | Freezes of 641-2,618 ms in one frame | **met** | `analysis\bakes_warp.txt` |
| A7 | Fuzz seed 7, 1,000 steps, ngons: central/dispatch frames over 500 ms ≤ 2, none in a stamp/text step; 0 errors | Run 1: 1 frame (dispatch 658 ms on a MergeDown that was refused, a layer step; the same action took 16 ms in run 2, so probably paging). Run 2: 0. "Invalid parameter" / "Unknown layer" / "panicked": 0 / 0 | 5 / 8 frames, worst 1,011 / 2,149 ms | **met** | `analysis\fuzz.txt` |
| A8 | Stamp step totals ≤ 1.25 x round 4 (min over runs) | ABG: image 0.69, text 0.88, transform 1.11. ngons: image 0.80, text 1.11, transform 1.18 | ABG 3,077 / 3,507, 1,241 / 656, 1,533 / 1,372 ms. ngons 866 / 859, 434 / 406, 1,549 / 1,592 ms | **met** | `analysis\steps_abg.txt`, `steps_ngons.txt` |

Why A5 misses on the text rotate: round 5's worker needs 364 / 375 ms for that bake. Round 4's whole release frame
took 287-293 ms. The bake pool has one thread fewer (5 of 6), and the job starts one frame later. The plan's 1.3 x
allows for exactly that, and this bake lands just past it. The two ABeautifulGame misses (scale 188 ms, rotate 183 ms)
come from the run during which the unrelated server used about 3 cores (23:54:18-24). The clean pair is 0.98 / 1.03.

### R5-2: Brush on very heavy models

| # | Line | Round 5 | Reference | Verdict | From |
|---|---|---|---|---|---|
| B1 | Brush hash tests unchanged; new "one dab per frame = unpaced hash" test | Pass (part of the 1,067). C1 legacy = shared hashes identical in all 8 bench rows of both core exes | | **met** | `runs_gate\build_logs\test1.log`, `bench\stroke_bench_tips_*.txt` |
| B2 | ABeautifulGame `workflow_path` strokes 1-3, stroke work ≤ 1.10 x P0, back to back, 2 pairs | Pair 1: 1,856 / 360 / 2,049 ms against P0 2,241 / 492 / 2,463 = **0.83 / 0.73 / 0.83**. Pair 2: 2,097 / 356 / 2,014 against 1,855 / 392 / 2,393 = **1.13** / 0.91 / 0.84. Sum of the 3 strokes: 0.82 / 0.96 | Same walk: P0 234 / 126 / 131 steps; round 5 235 / 127 / 132 samples | **met** for strokes 2-3 and the sum. Stroke 1 of pair 2 is 1.13 (first use of 8 sets; that round-5 run had the heavier load) | `analysis\p0_strokes.txt`, `steps_abg.txt` |
| B3 | `brush_types_path` stroke work ≤ 1.05 x round 4, back to back; core `stroke_bench_tips` ≤ 1.02 x | ABG, 7 strokes x 2 pairs: 0.69-1.02. tex_4096: pair 1 0.70-1.11, pair 2 0.80-2.29. tex_8192: pair 1 1.00-1.93, pair 2 0.98-1.92 (strokes of 80-600 ms, single dabs of up to 647 ms from paging). Core, model b (ABG): 0.85-0.995 per row. Models a / c: 0.71-1.38 between the two runs, and the round-3 loop row, whose code did not change, moves just as much (0.73-1.26) | | **met on ABeautifulGame (app and core). Not decidable on tex_8192 / tex_4096**: every pair there ran under 1-3.5 cores of unrelated load | `analysis\btp_cmp.txt`, `bench_cmp.txt`, `load_per_run.txt` |
| B4 | ABG Size 50 after the warm-up stroke: longest frame of stroke work ≤ 100 ms + longest dab **and** ≤ 140 ms; 0 frames over 140 ms | Run 1: 126 / **156** / **142** ms (dabs 55 / 76 / 52); run 2: 134 / 140 / 140 (dabs 47 / 68 / 50); probe run 156 / 147 / 123. "100 ms + longest dab" holds everywhere. Drag frames over 140 ms: 0 / 4 / 1 and 1 / 1 / 1 | Round 4 (same session): 180 / 220 / 141 and 183 / 425 / 197 ms; drag frames over 140 ms 5 / 7 / 3 and 2 / 8 / 4 | **partly met**: better than round 4, but 140 ms is met only in run 2. A single dab of 50-80 ms plus the 100-ms budget can pass 140 ms | `analysis\btp_cmp.txt` |
| B5 | P8a-P8c stay met (release to end ≤ 400 ms at Size 50; pointer step ≤ 1.10 x r4) | Release to end at Size 50: 2-14 ms. Pointer step against r4: 0.48-1.09 at Size 50, 0.92-1.05 at Size 512. **But at Size 512 on ABG, release to end is 6.7-11.9 s** (P8b's Size-512 limit is 1,000 ms) | Round 4 at Size 512: release to end 0 ms, but frames of 4.9-8.7 s during the drag | **met at Size 50; P8b missed at Size 512 on ABeautifulGame** (tex models ≤ 31 ms) | `analysis\btp_cmp.txt` |
| B6 | P8d largest backlog, ABG Size 50 (limit 350 ms): reported with the phase split | Run 1: 946 / 1,139 / 1,516 / 976 ms; run 2: 1,026 / 915 / 1,295 / 760; probe run 850 / 1,180 / 1,683 / 1,125 | Round 4 (same session): 913-1,633 / 853-2,065 ms | **not met** (as the plan expected); split below | `btp\..._r5_probe\shot.log` |
| B7 | Fuzz seed 7: no `pointer/tools/hover` frame over 500 ms | 0 in both runs (0 over 300 ms) | | **met** | `analysis\fuzz.txt` |

### R5-3: Font picker's first open

| Line | Round 5 | Round 4 | Verdict | From |
|---|---|---|---|---|
| `text_fonts` x 3, the first right after an ABeautifulGame `workflow_path`: every side-panel frame between the markers ≤ 16 ms | Side panel max **2.14 / 1.58 / 1.38 ms**. The picker's own scopes ≤ 1.11 ms. Intervals ≤ 45 ms | 179.0 and 131.9 ms in run 1 of 5 | **met** | `analysis\fonts.txt` |
| `gate_font_picker` (ignored timing probe) ≤ 16 ms | Press 1.92 ms, opening 2.35 ms, open idle max 2.42 ms, scrolling max 2.94 ms (78 families) | | **met** | `runs_gate2\gate_font_picker.txt` |
| Picker tests pass; samples asked for once per catalog | Pass (`the_font_picker_filters_and_takes_a_font`, `the_font_button_rect_opens_the_picker`, `the_samples_are_asked_for_once_per_catalog_by_the_button`) | | **met** | `test1.log` |

## Is the brush slower per stroke than round 3? (B2)

**No.** On the same three ABeautifulGame strokes, back to back with the round-3 code (P0 exe), round 5's stroke work is
0.73-0.91 x on strokes 2 and 3, the ones the plan judges by, in both pairs. Over all three strokes it is 0.82 x and
0.96 x. Round 4 measured in the same session falls in between (2,054 / 377 / 2,364 and 1,947 / 504 / 2,451 ms).

The only value over 1.10 is stroke 1 of pair 2 (1.13). That stroke pays for the first use of 8 texture sets, and its
round-5 run had about 3 cores of unrelated load at the time. The two exes walk the same steps (234 / 126 / 131 against
235 / 127 / 132 samples, the press included).

Round 4's "1.8-1.9 x per walk step" (P7) compared a Size-50 stroke with a Size-32 baseline. Package B found that the
P0 labels were one size step off. So P7 was a measuring artifact and is closed.

## Phase split of the heavy brush frames (B6)

`WTE_STROKE_LOG=1`, ABeautifulGame, `brush_types_path` (loaded run; `btp\..._r5_probe\shot.log`).

Per stroke at Size 50 (317 dabs; 1,844-1,857 set dabs; buffer copies 0; first touch 1.3 ms in the warm-up stroke and 0
after):

| Stroke | Stroke work | Raster | Commit | Re-apply | Query | Setup | Texels touched |
|---|---|---|---|---|---|---|---|
| Circular (after warm-up) | 1,736 ms | 1,188 (68 %) | 219 (13 %) | 166 (10 %) | 85 (5 %) | 76 (4 %) | 20.0 M |
| Sphere | 2,506 ms | 1,761 (70 %) | 316 (13 %) | 241 (10 %) | 75 (3 %) | 109 (4 %) | 25.7 M |
| Cube | 1,712 ms | 1,275 (74 %) | 131 (8 %) | 114 (7 %) | 101 (6 %) | 89 (5 %) | 11.6 M |

The bursts are the samples of 20 ms or more: 18-30 per stroke, about 100 of the 317 dabs, 0.85-1.57 s (about half
of the stroke work). In those samples raster is 57-74 %, commit 7-11 %, re-apply 6-9 %, query 3-11 %, setup 4-6 %, and
they touch 4.8-14 M texels (about 50-140 thousand texels per dab).

At Size 512 (49 dabs, 283-330 rasterizing set dabs) raster is 85-90 % of 44-67 s.

**What a later round would have to change: rasterization in parallel inside one dab.** The bursts are almost all raster
work of single large dabs, 1.2-1.4 sets per dab, so painting the sets of one dab in parallel (B-4.2) would win little.
The phase split shows no copy and no first-use cost left to remove. Pacing by the dab cannot shrink the backlog,
because the work itself is the backlog.

## What was found for the font picker

Package B moved the sample job to the font button (it starts when the Text block is first shown, not when the picker
opens). It also cached each family's row data, so a row no longer scans the catalog each frame. The picker's own code
now costs under 1.2 ms per frame, even on the first open after a heavy ABeautifulGame run.

During package B's work, the one slow first-open frame (36.6 ms, under load) was inside egui's `Popup::show`, outside
every picker scope: cold pages, or egui laying out its first popup. In this gate's runs it did not appear (max
2.14 ms). In a run under the unrelated `find /` plus server load, one 32.9-ms frame appeared in run 3 (not run 1)
(`runs_gate2\fonts_loaded\`). The item is closed.

Side effect: choosing the Text tool starts the non-modal "Drawing font samples" job (48-73 ms on an idle-ish machine,
up to 0.5 s under load). `workflow_path`'s "text stamp" step waits for it. On these runs the step is within its line
(min ratio 0.88 on ABG, 1.11 on ngons). Stage 1's 1.37-2.61 x readings were taken under load.

## Stage-1 open points, settled

1. "Text stamp" step over 25 %: within the line on a calmer machine (above).
2. ABeautifulGame "2D view stroke" 1.38 x: **load artifact**. Round 5 measured 1,433 / 1,438 ms against round 4's 1,523 /
   1,658 ms (0.94 x). The 345-ms first 2D frame did not come back (`analysis\steps_frames.txt`).
3. Export frame of 3.8 s in round 5: not seen again. Round 5's longest export frame was 28 / 89 ms. Round 4 had a
   2.9-s frame in its loaded run. This is paging when free memory runs out. Export step times swing 12-130 s in
   **both** exes depending on memory pressure.
4. Not done in either stage: the 9 round-4 fuzz repro jobs (plan 5 item 4). Round 4's `jobs_fuzz.txt` and
   `sphere_sample.wtep` are gone and would have to be rebuilt from round 4's `notes\gate2.md` section 3.

## Not met, and why (summary)

- **A4 (as written)**: the Preview bakes during a drag on ngons_tiling are 148-198 ms (round 4: 150-174 ms). The plan did
  not change them (`bake_stale_previews` is unchanged). Every Final-related frame is ≤ 33 ms.
- **A5**: text rotate on ngons, 1.31 / 1.40 x. This is the one-thread-smaller pool plus one frame of latency on a
  290-ms bake. Two ABG transform bakes are at 1.97 / 2.23 x in the run with heavy unrelated load (clean pair: 0.98 /
  1.03).
- **B3 on tex_8192 / tex_4096**: cannot be decided. Every pair ran under 1-3.5 cores of unrelated load and showed
  paging dabs (up to 647 ms for a 50-texel dab). The ABeautifulGame pairs and the core bench on that model are within
  the line.
- **B4 (140 ms)**: met in 1 of 2 runs. It needs dabs under 40 ms; heavy dabs are 50-80 ms.
- **B5 / P8b at Size 512 on ABeautifulGame**: release to end is 6.7-11.9 s. This is the cost of pacing by the dab: frames
  are now one dab (1.4-2.1 s) instead of one whole sample (4.9-8.7 s), so the painting falls behind the pointer and
  finishes after the release. Total time is the same (pointer step 0.92-1.05 x round 4).
- **B6**: backlog 760-1,683 ms (limit 350). It needs cheaper dabs (see the phase split).
- **B-4.1 (reused candidate list)**: its 5 % gain was not shown. The core bench's in-run A/B on ABeautifulGame gives
  1.00 x at Size 50 and 0.995 x at Size 512, and other rows are noise. It changes no pixels and was kept. Taking it out
  is a small change if wanted (package B's notes describe how).

## Deliberate behaviour changes in round 5

1. **A new stamp or text shows its frame first and its picture when the worker bake lands.** On ABeautifulGame that is
   1.4-1.7 s for an image and 25-190 ms for a text. The status bar shows "Finishing stamp" / "Finishing text".
2. **After a move, scale, rotate, warp or style change**, the picture can show the last Preview of the drag until the
   bake lands. It then sharpens (0.6-1.1 s for a warped text on ABeautifulGame).
3. **Save, Save As, Export, the export dialog, Merge Down and Rasterize wait** for a pending bake. They never wait longer
   than round 4 froze for the same edit.
4. **Undo and redo never wait.** A state restored without its Full bake is baked again in the background. Undoing an
   unrelated entry made during a bake can briefly bring back the Preview picture (package A's notes).
5. **The brush paces by the dab.** A frame is about 100 ms plus at most one dab. On very heavy models at large sizes the
   stroke keeps painting after the release (Size 512 on ABeautifulGame: 7-12 s), where round 4 froze for seconds
   during the drag. A Size-50 stroke on ABeautifulGame `workflow_path` ends about 1.2 s after the release (round 4:
   0.8-0.9 s).
6. **Font samples are drawn when the Text block first appears**, not when the picker opens (a short background job).
7. **Layer ids**: in saved projects, the per-set layers of an image stamp on a multi-set model are numbered with the
   active set first. Content, order and names are unchanged.
8. New switches: `WTE_SYNC_FINAL_BAKES=1` (old synchronous bakes) and `WTE_STROKE_LOG=1` (brush phase probe). The
   brush log line has new fields: size, ring, dabs, longest dab.

## What to try by hand

1. On ABeautifulGame, place an image stamp, then drag it, scale it, rotate it. Keep working (orbit, pick another tool)
   while the picture sharpens. Nothing should freeze, and the status bar shows "Finishing stamp".
2. Add a text, give it an Arc or Flag warp and change the spacing. Keep typing while it sharpens.
3. **Undo right after a release** (before the picture sharpens), then Redo. The picture should come back right, with no
   wait.
4. **Save right after a release.** The save waits a moment, and the reopened project should look exactly like the
   screen.
5. **Merge Down or Rasterize a stamp layer right after a release.** The result should be the sharp picture.
6. **A fast stroke across the chess pieces** at the default size: the view should stay responsive and the paint
   should follow. Then try a very large brush (Size 512) and note how long it keeps painting after you let go. Say
   whether that feels better or worse than a frozen screen.
7. Open the font picker for the first time after loading a heavy model. It should open at once.
