# Check note: P1 - Styles and picture maths (STICKER_PLAN 7 "P1")

Independent checker, 2026-11-25, branch `stickers`, builder commit d2f5d25. Verdict: **pass after fixes** (two majors
and one related major in the tent blur found and fixed, each with a test that fails on d2f5d25).

## Item by item (plan 7 "P1", its checker line, and the sections it cites)

| Plan item | Checked how | Result |
|---|---|---|
| Files: `sticker/{mod,style,fx,adjust,shape,resample}.rs`, `AlphaBuf`, `nearest_ink`, `lib.rs` exports, no `Document` change | `git show --stat d2f5d25`; read every file | present; extra `fx_tests.rs`, `bench.rs` (test-only). No `Document` / app change |
| 2.3 `StyleStack` and entries, limits, `sanitize`, `is_default`, presets | read `style.rs` against 2.3 | as planned; `margin` deviation (contour + stroke added, `max` of shadow / glow) is sound (the stroke runs around the contour; shadow and glow are both measured from the outline) |
| 2.4 / 4.6 adjustments | read `adjust.rs`, recomputed the hand values of the test | OK (brightness curve clamped, tables monotone; colorize and saturation rules as 4.6) |
| 4.2 distances, tent, spread split | read `fx.rs`, `edt.rs` diff | `distance_to_ink` untouched (diff adds lines only; text tests pass). **The tent was wrong next to the buffer edge (finding 2)** |
| 4.3 `apply_styles` steps 1-8 | read each step against the formulas | order of compose, overlays, stroke planes, contour, drop shadow, bevel OK; **inner shadow / inner glow saw only a one-pixel outside (finding 1)** |
| 4.7 resampler | read `resample.rs` | OK: premultiplied bilinear, mip level choice, exact-copy path, clip and output cap |
| 4.8 shapes | read `shape.rs`, brute-force distances | fill OK; **thick polygon / star outlines were cut to about 2 px (finding 3)** |
| `nearest_ink` vs brute force | read the test and the transform | OK (site of the winning parabola, signed column offsets as `distance_to_ink`) |
| Unit tests of the plan list (`k1_*`) | matched each sentence of plan 7 "P1" to a test | all present. The A7 test only asserted "border pixel changed", which passed while finding 1 made the inner glow about 10 x too weak on an opaque photo |
| Checker line: fuzz `apply_styles` with empty, 1x1, opaque, transparent | new test `k1_checker_fuzz_degenerate_and_extreme_inputs` (240 random stacks, lengths up to 1000 before sanitize, scales 0.02..1, 0x0 / 1x1 / 1xN / Nx1 / opaque / transparent bases) | no panic, sizes `(w + 2m) x (h + 2m)`, borders transparent, a transparent base stays transparent |
| Checker line: licence header on adapted files | `head -1` | the five adapted files carry the exact notice line; `licenses/photocraft-MIT.txt` present; no "ArtCraft" in code or ARCHITECTURE |
| No new crate, LF | `git diff 34b98d9 d2f5d25 -- Cargo.lock` empty; `git ls-files --eol` | OK |
| ARCHITECTURE 1.12, 2.1, 5.5, 5.27, 6.9, 6.11, 9.1, 10 | read the diff | present; 6.11 and 9.1 updated by the checker for the fixes |

## Findings

1. **Major (fixed).** Inner shadow and inner glow were blurred on `1 - A` with the tent's "outside = 0", so only the
   one-pixel working pad counted as outside. For the usual case, a fully opaque photo (review A7), the inner glow of
   size 20 reached 43 / 255 at the edge instead of about 185 / 255 (the same square inside a transparent frame); an
   inner shadow also lost the artwork moved past the plane's edge, so a shadow appeared along the far edges. The A7 test
   passed because it only checked that border pixels changed. Fix (`fx.rs` `inner_plane`): work on the complement (the
   artwork eroded by `solid`), move it into a buffer larger by the move on every side, tent-blur with outside 0, crop,
   invert: everything outside the picture now counts as transparent. Test `k1_inner_styles_see_the_whole_outside`:
   opaque 64 x 64 photo vs the same square in a 200 x 200 transparent picture, inner glow (spread 0 / 0.5), inner
   shadow (spread 0 / 0.4), bevel: equal within 1 level (was 179 levels off on d2f5d25).
2. **Major (fixed).** `tent` cut its first box pass at the buffer edge, so the result next to the border was not the tent
   of the zero-extended picture (at a straight edge 105 instead of 134 for width 20). It changed the bevel heights of an
   opaque photo, and finding 1 could not be fixed without it. Outer styles were not affected (the margin keeps them away
   from the border; the identity of those results is shown by the unchanged 200-stack and half-scale tests). Fix: the
   first pass of each axis is kept `b - 1` pixels beyond the buffer (`box_line` / `box_cols` take a start offset and an
   output longer than the input). Test: in `k1_tent_blur_keeps_the_sum_and_is_symmetric`, a random 37 x 29 picture
   blurred alone equals the same picture blurred in the middle of a larger zero buffer, widths 2 / 5 / 12 / 40, u8 and
   u16. Timing A/B on this machine (game running, CPU about 75 %): old vs new interleaved, ClassicSticker 1024 39.7 /
   41.7 ms vs 29.6 / 42.0 ms; Preview size 21.7 / 20.2 ms vs 15.7 / 24.0 ms: no measurable cost.
3. **Major (fixed).** `polygon_row` computed exact edge distances only within 1.5 px of the row, enough for the fill
   but not for an outline: a Polygon or Star outline of width 10 was cut to about 2 px (pixels farther than 1.5 px from
   an edge got distance 1e6, so no outline). The outline test only used a Rect (analytic distance). Fix: a `reach`
   parameter (1.5 for the fill and `polygon_coverage`, `outline width / 2 + 1` in `rasterize`). Test
   `k1_thick_polygon_and_star_outlines_follow_the_edge_distance`: hexagon width 10, star width 8, triangle width 24
   against brute-force edge distances within 1 level (was 255 levels off).
4. **Minor (not changed, for P3a).** `apply_styles` allocates `(w + 2m) x (h + 2m)` with no cap of its own; the margin
   is bounded only by `min(1e6)`. With the 256-px length limit, the margin is up to about `1280 * scale + 2`, so a large
   `scale` (a text stamp drawn at 512 px per em has a reference scale of 4) can ask for hundreds of MB. P3a's
   working-size rule must bound `scale` / the output size before calling it (the builder's open issue says the same
   for the 309 MB "every style on" case).
5. **Minor (not changed).** A stroke or contour that is on with width 0 still gives a margin of 2 (an empty frame of
   2 px). Harmless: the margin is known and nothing is drawn there.
6. **Minor (not changed).** Mip-level sampling uses `level.width / src.width` as the axis factor; for odd sizes the
   `PremulMips` level is not exactly half, so a shrunk odd-sized picture can be placed up to a quarter pixel off.
   Only for scales below 0.5; no test asks for sub-pixel placement there.

7. **Minor (pre-existing, not changed; for the gate).** `wrap_surface` step (6b) needs at least one of the drag's
   wrapped Preview jobs to land; under heavy load the only job started can be superseded (started 1, applied 0,
   dropped 1) and the scenario fails although the result is the same. Seen once in four runs of this exe. If the gate
   sees it, the check could accept "applied 0, dropped >= 1" or the drag could last until one job has landed.

Deviations of the builder (build note 1-13): all read and accepted. The bevel normalisation (4), threshold-with-source
(2) and fill-holes edge rule (3) are improvements over the plan's literal formulas and are documented in 6.11.

## Runs (after the fixes)

- `cargo test --workspace --no-fail-fast`: **1,447 passed, 1 failed (WA20 only), 51 ignored** (`wte_app` 524 / 0 / 24,
  `wte_core` 867 / 1 / 27, integration 15 + 9 + 8 + 24). Builder's state reproduced first: 1,444 / 1 / 51.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- Perf (`perf_k1_*`, release, serial; the machine was loaded, a game running, so numbers scatter 2 x between runs):
  ClassicSticker 1024 / 2048 / 4096 = 48.1 / 147.5 / 638.0 ms (budgets 60 / 250 / 1100); Preview size 23.8 ms (33);
  peak memory at 4096 150 MB (256); `distance_to_ink` 2048 48.6 ms (80). One further run under the same load had the
  Preview case at 51.2 ms (over 1.5 x); the interleaved A/B above (15.7-24.0 ms) shows that this is load, not the code.
  **The gate must re-measure on an idle machine.**
- Release build OK, exe SHA-256 `aef9bbbd10c6910f5e9b7fcb46b24dce01293b44c8a09a909610b443badeff86`.
- Identity sweep (8.5, `tools/objtabs/sweep.sh`, off-screen, no focus) into `target\sticker\check-p1\sweep\`:
  21 jobs. First run (absolute output path) differed only in the export dialog's Folder field, which shows the
  sweep's own path (`export`, `export_checks`); rerun with the relative path the baseline used. In that run
  `wrap_surface` failed once: step (6b) "the drag started 1 Preview jobs, 0 landed" (dropped 1), composite hash equal
  to the good runs (`0b0f7479b610f4c7`); rerun alone twice: `"ok": true` both times. P1 touches no app code (the app
  does not use `wte_core::sticker`), so this is a pre-existing timing race of the scenario on a loaded machine (a
  game was running): finding 7. With the rerun's `wrap_surface` folder: **21 / 21 `"ok": true`, `compare_baseline.py`
  216 / 216 files the same, PASS** (`target\sticker\check-p1\compare.txt`; the failed run is kept in
  `target\sticker\check-p1\wrap_surface_failed_run\`). Looked at the `stamp` shot: unchanged. P1 adds no scenario
  (plan 8.2 lists none).
