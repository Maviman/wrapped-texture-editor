# Build note: P2 - Selection, cut-out and filter maths (STICKER_PLAN 7 "P2")

Builder, 2026-11-25, branch `stickers` (from 727ac6c). Core only (kind K): no `Document` change, nothing wired into the
app, so no new scenario (plan 8.2 lists none for P2); the identity sweep of 8.5 is the app-level check.

## What was built

| Item (plan P2) | Where | Notes |
|---|---|---|
| Selection (3.4 core part) | `crates/wte_core/src/sticker/select.rs` | `Selection { mask, bounds }` (`from_mask`, `none`, `all`, marquee `rect` / `ellipse`, lasso `polygon`, `from_alpha`, `combine` with `SelectMode` Replace / Add / Subtract / Intersect, `invert`, `grow`, `shrink`, `feather`, `limit`, `at`, `size`, `is_empty`), `mask_bounds` |
| Colour-tolerance regions (4.4) | `select.rs` | `color_diff`, `fringe_value`, `region` (wand / Fill / Magic Eraser: contiguous scanline flood or every match, anti-alias fringe), `color_range` (Color Range), `RegionOptions` (default 32, contiguous, anti-alias) |
| Applying a coverage | `select.rs` | `erase` / `erase_mask` (Magic Eraser, Delete, "Into mask"), `fill` / `fill_mask`, `extract` (Layer via Copy), `mix_px` / `mix_selected` / `mix_selected_mask` (limit a finished edit to a selection, premultiplied) |
| Cut-out helpers (4.5, not GrabCut) | `crates/wte_core/src/sticker/cutout.rs` | `unmix_pixel` / `color_to_alpha` (thresholds `t0`, `t1`), `remove_matte` (`Matte::White` / `Black`), `defringe` (1..=10), `remove_background` -> `BackgroundCut` (`apply`, `apply_to_mask`), `RemoveBgOptions` (tolerance 40, Follow gradients, Also enclosed areas), `RemoveBgRefusal` (`Empty`, `AlreadyTransparent`, `NoPlainBackground`, each with its UI message) |
| Blur, sharpen, gradient (4.9) | `crates/wte_core/src/sticker/filter.rs` | `gaussian_boxes`, `blur` / `blur_mask` (three boxes, premultiplied 16-bit, replicated edges), `sharpen` (unsharp mask), `draw_gradient` / `draw_gradient_mask`; limits `MAX_BLUR_RADIUS` 64, `MAX_SHARPEN_AMOUNT` 3, `MAX_SHARPEN_RADIUS` 16 |
| Exports | `sticker/mod.rs`, `lib.rs` | modules `cutout`, `filter`, `select`; `Selection`, `SelectMode`, `RegionOptions`, `Matte`, `RemoveBgOptions`, `RemoveBgRefusal`, `BackgroundCut` at the crate root |
| Shared helpers | `fx.rs`, `shape.rs` | `fx::cov`, `fx::over`, `shape::sd_round_box`, `shape::sd_ellipse` made `pub(crate)` (visibility only, no code change) |
| Tests | in-file `k2_*` tests in the three files; `sticker/bench.rs` (ignored `perf_k2_*`) | 23 `k2_*` tests, 3 perf tests |
| ARCHITECTURE | 1.3 (the non-goal names texture layers only), 1.12 (P2 traceability), 2.1 (three rows, bench row), 5.27 (API part 2), new 6.12, 9.1 (P2 tests), 10 (P2 row) | |

Every plan sentence of 7 "P2" has a test: wand two-colour at tolerance 0; tolerance edge (diff = tolerance is in);
contiguous vs not; anti-alias values on a ramp; transparent pixels match; magic eraser on white (object 255, background
0); fill honours a selection; colour to alpha (pure key -> 0, 50 % red over white -> red at 128 +- 1, opaque black
stays); mattes; defringe of a white halo on a red disc (no visible pixel keeps g > 40); remove background on flat white,
two-colour border and a vertical gradient with Follow gradients (IoU >= 0.98) and the "already transparent" case; grow
then shrink of a disc within 1 px; feather keeps the sum within 1 %; blur of a constant picture, premultiplied sum,
no colour bleed; sharpen amount 0 = identity; gradient endpoints exact and limited by a selection; thread-count
independence with local pools of 1 and 8 threads (one test per file).

Licence: the three new files start with `// Portions adapted from PhotoCraft (MIT licence, see
licenses/photocraft-MIT.txt).` (written with `pc020\crates\algo\src\selection.rs`, `erase.rs`, `layer_menu_cmds.rs`
and `PCHEAD:crates/algo/src/color_to_alpha.rs` open). No ArtCraft name anywhere. No new crate; `Cargo.lock` unchanged.

HANDOFF.md and `dist/HOW_TO_TEST.txt`: no change (plan section 10 gives P2 none; nothing is user-visible before P7b).

## Deviations from the plan, with reasons

1. **Feather extends the border (4.4).** The plan says "feather = tent blur of width n"; the tent's outside is 0, so a
   selection touching the artboard's border would fade there and Select All + Feather would no longer select the
   border pixels. Built: the mask is extended by its own border values (by the box width) before the tent and cropped
   back. Shrink likewise never treats the artboard border as an edge (`dist_in` has no pad). Test: Select All stays all
   after shrink and feather; the disc's sum stays within 1 %.
2. **Background colours of Remove Background (4.5 step 2).** The plan's rule ("up to 3 most frequent 4-bit cells that
   each hold 15 %") fails on a noisy mid-grey background whose values straddle a cell boundary (128 +- 12 spreads over
   8 cells, none with 15 %), and on a gradient it would cut colours at cell edges. Built: the most frequent cell (ties:
   lower index) only **seeds** a cluster: every not yet taken opaque frame pixel within `tolerance` of the cell's mean;
   the cluster's mean is a colour when it holds 15 % of the frame; up to three. Test
   `k2_remove_background_on_a_noisy_grey_background` asserts that no cell holds 15 % and that the cut still works.
   "Transparent" for step 1 = alpha < 128.
3. **Soft edge of Remove Background (4.5 step 4).** Plain colour to alpha against the background colour also hits the
   object's own opaque pixels inside the 2-px band: a red object on white got a rim about 12 % transparent with its
   colour shifted to (170, 0, 11). Built: the band pixel's colour-to-alpha opacity is divided by the opacity of the
   nearest "inside" pixel (not region, not band) against the same key, so an anti-aliased `k O + (1 - k) C` gives back
   `O` at alpha `k` and opaque object pixels stay opaque; plain colour to alpha when no inside pixel is within 4 px.
   The key is the background colour nearest to the closest region pixel; with Follow gradients that pixel's own colour
   (a gradient background has no single colour). "Nearest" = the first hit in a fixed disc of radius 4 (ordered by
   distance, row, column): band pixels are within 2 px of the region, and this avoids two more full distance
   transforms. A mutation (normalisation off) makes `k2_remove_background_on_three_synthetic_pictures` fail.
4. **Remove white / black matte test reading.** "Invert a matte within 1 level for alpha >= 32" cannot hold for the
   recovered colour itself: the stored `P` is rounded to a byte, and dividing by `a = 32 / 255` magnifies that half
   level to about 4 levels. Tested: matting the result again gives `P` within 1 level (the matte is inverted), and the
   colour is within the rounding of `P` (`ceil(0.5 / a) + 1` levels).
5. **Anti-alias fringe of a diagonal match.** In contiguous mode a matching pixel that touches the region only
   diagonally is not 4-connected; the plan's fringe formula gives it 255. Kept as the formula says (documented in 6.12).
6. **Blur at alpha 0.** A pixel whose blurred alpha rounds to 0 keeps its source RGB at alpha 0 (no division by zero,
   invisible data kept, as the adjustments do).
7. **Gradient of length 0** (press = release): `t = 1` everywhere (the last stop), not specified in the plan.
8. **API shape.** The plan names functions only by what they do. Built as pure functions over `PixelBuf` / `AlphaBuf` of
   one size; the caller (P3b `replace_sticker_pixels`, P7b tools) maps the layer to artboard space and picks the
   picture (active layer or composite for "Sample all layers"). A selection or coverage of another size counts as
   empty, so nothing changes rather than something wrong. Every destructive command takes `sel: Option<&AlphaBuf>` and
   mixes premultiplied (`mix_px`).
9. **Flood fill.** The match test runs once per pixel into a byte plane in parallel rows; the scanline flood then
   compares bytes (`flood_plane`, also used by Remove Background). A per-pixel test inside the flood made the comb case
   824 ms.
10. **Extra perf rows** (reported, not in 8.4): blur radius 64, sharpen, Remove Background with both options, and a
    4096 x 4096 comb (2,048 one-pixel teeth joined at the bottom: about 8 M one-pixel runs, the worst case for a span
    stack) held to the flood budget of 400 ms.

## Numbers measured

Perf (`cargo test -p wte_core --release --lib perf_k2 -- --ignored --test-threads=1 --nocapture`, this machine, median of
5 after a warm-up; the final run, `target\sticker\p2\perf.txt`; an earlier run gave the same numbers within 10 %):

| Operation (8.4) | Measured | Budget |
|---|---|---|
| wand (contiguous, anti-alias) 2048 x 2048, white background | 10.8 ms | 60 ms |
| magic eraser 2048 x 2048 (region + erase, a picture copy included) | 15.5 ms | 60 ms |
| remove background 2048 x 2048 (found and applied) | 46.5 ms | 150 ms |
| Gaussian blur radius 16, 2048 x 2048 | 51.0 ms | 150 ms |
| flood fill worst case, 4096 x 4096 one colour (anti-alias off / on) | 37.7 / 43.3 ms | 400 ms |
| reported only: flood 4096 x 4096 comb (2,048 one-pixel teeth, about 8 M runs) | 314.2 ms | (400 ms) |
| reported only: blur radius 64 / sharpen 100 % radius 4, 2048 x 2048 | 53.7 / 70.0 ms | - |
| reported only: remove background with Follow gradients and Also enclosed areas, 2048 x 2048 | 109.1 ms | - |

Before deviation 9 (the match test inside the flood) the comb took 823.5 ms and the plain remove background 132.1 ms.

Accuracy (from the tests): remove background IoU flat white 1.0000, two-colour border 1.0000 (colours found
(60, 160, 70) and (120, 170, 230)), vertical gradient with Follow gradients 0.9996 (three colours found); opaque object
pixels stay >= 250 and anti-aliased edge pixels get the object colour back within 12 levels; grow 8 then shrink 8 of a
radius-30 disc: no pixel moves by more than half coverage, the edge within 1 px; feather 10 keeps the disc's sum within
1 %; blur keeps the premultiplied sum of a random patch within 0.5 % and bleeds no colour from transparent pixels;
colour to alpha re-composited over the key reproduces 2,000 random pixels within 1.5 levels.

Mutation checks (made, run, reverted): soft-edge normalisation off -> `k2_remove_background_on_three_synthetic_pictures`
fails (edge pixel (169, 0, 11, 68)); blur without premultiplication -> `k2_blur_keeps_the_premultiplied_sum_and_bleeds_no_colour`
fails.

## Workspace runs (final state)

- `cargo test --workspace --no-fail-fast`: **1,470 passed, 1 failed (WA20), 54 ignored** (`wte_app` 524 / 0 / 24;
  `wte_core` 890 / 1 / 30; integration 15 + 9 + 8 + 24). Against P1's check (1,447 / 1 / 51): +23 `k2_*` tests, +3
  ignored `perf_k2_*`. The red test is `wrap_tests::wa20_...` only. Log `target\sticker\p2\test.txt`.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK; exe 28,040,704 bytes, SHA-256 `28bc4c92...98eb` (names this build only;
  release builds here are not reproducible, see `baseline.md`).
- Identity sweep (8.5, `tools/objtabs/sweep.sh` with that exe, `tools/sticker/jobs_identity.txt`, 21 jobs, off-screen,
  no focus) into `target\sticker\p2\sweep\`: **21 / 21 `"ok": true`**, exit 0 each; `compare_baseline.py` against
  the P0 baseline: **216 / 216 files the same, PASS** (`target\sticker\p2\compare.txt`). Looked at `workflow_path`
  (Split view, chess set, UV view) and `stamp` (logo stamp with its gizmo on the checker sphere): as before. P2 adds no
  scenario (plan 8.2 lists none for P2; nothing reaches the app before P3a).
- Line endings: every new or changed file is LF (`git ls-files --eol`). No `ArtCraft` anywhere; `Cargo.lock` unchanged.

## Open points for the checker / later packages

- P3b / P7b map a layer's content to artboard space and back; every P2 function takes buffers of one size and treats
  a selection of another size as empty (nothing changes). The tools must hand matching sizes.
- `region` holds a full-size byte plane (16 MB at 4096 x 4096) plus the output; Remove Background holds the region,
  one `f32` distance field and the keep plane (about 100 MB at 4096 x 4096). Not measured with the allocation counter.
- Remove Background's soft edge looks for the nearest region / inside pixel within 4 px only; an object thinner than
  that gets plain colour to alpha on its edge (by design, documented in 6.12).
