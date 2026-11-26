# Build note: P1 - Styles and picture maths (STICKER_PLAN 7 "P1")

Builder, 2026-11-25, branch `stickers` (from 34b98d9). Core only (kind K): no `Document` change, nothing wired into the
app, so no new scenario; the identity sweep of 8.5 is the app-level check.

## What was built

| Item (plan P1) | Where | Notes |
|---|---|---|
| Module tree and limits | `crates/wte_core/src/sticker/mod.rs` | `MAX_ARTBOARD` 4096, `MAX_STICKER_LAYERS` 32, `MAX_PICTURE_SIDE` 8192, `MAX_CREATED_SIDE` 4096, `MAX_OWNED_BYTES` 512 MiB, `MAX_GRADIENT_STOPS` 8, `MAX_STYLE_LENGTH` 256; `LayerTransform` (identity, `moved`, `sanitize`, `apply`) because the resampler needs it |
| Styles (2.3) | `sticker/style.rs` | `StyleStack` and its nine entries, `Gradient` (`two`, `sample`, `sanitize`), `StrokePos`, `is_default`, `any_on`, `sanitize`, `margin`, crate-internal `scaled`; `StylePreset` (ClassicSticker, Outline, Glow, Badge, ShadowOnly, Clear; lengths for 512 px scaled by `max(content) / 512`) |
| `apply_styles` (4.3), distances and tent blur (4.2) | `sticker/fx.rs` | `apply_styles`, `tent` / `TentCell` (u8 and u16), `tent_width`, `dist_out`, `dist_in`; internal planes (contour, outside / inside stroke, drop shadow, glow, inner shadow), compose |
| `nearest_ink` | `text/edt.rs` | new function next to `distance_to_ink` (which is byte-for-byte unchanged: the diff only adds lines; the existing text tests pass untouched) |
| `AlphaBuf` | `pixel.rs` | `new`, `filled`, `from_raw`, `from_alpha`, `get`, `set`, `rect`, `byte_len` |
| Adjustments (2.4, 4.6) | `sticker/adjust.rs` | `Adjustment` (seven kinds), `sanitize`, `is_identity`, `lut`, `apply`; `apply_adjustments` |
| Resampler (4.7) | `sticker/resample.rs` | `transform_picture` (clip, mip level, exact-copy path), `transformed_bounds`, `MAX_RESAMPLE_PIXELS` |
| Shapes (4.8) | `sticker/shape.rs` | `ShapeSpec`, `ShapeKind`, `Paint`, `rasterize`, `polygon_coverage` (non-zero rule + edge distance, for P2's lasso too), `shape_points` |
| Exports | `lib.rs` | `pub mod sticker`; the types re-exported at the crate root |
| Tests | in-file `k1_*` tests, `sticker/fx_tests.rs`, `sticker/bench.rs` (ignored `perf_k1_*`) | 33 `k1_*` tests, 3 perf tests |
| ARCHITECTURE | 1.12 (skeleton), 2.1 (rows), 5.5 (`AlphaBuf`), new 5.27 (API part 1), 6.9 (pitfalls 11-14), new 6.11 (styles and picture maths), 9.1 (P1 tests), 10 (P1 row) | |

Licence: every file written with PhotoCraft's source open (`style.rs`, `fx.rs`, `adjust.rs`, `shape.rs`, `resample.rs`)
starts with `// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).` No ArtCraft name
anywhere. No new crate; `Cargo.lock` unchanged.

HANDOFF.md and `dist/HOW_TO_TEST.txt`: no change. Plan section 10 gives P1 no HANDOFF / HOW_TO_TEST edit (nothing is
user-visible before P6a / P6b).

## Deviations from the plan, with reasons

1. **Margin (2.3).** The plan's formula takes `max(contour reach, outside stroke)` and adds shadow reach **plus** glow
   reach. Built: contour reach **plus** the outside stroke (4.3 step 2 draws the outside stroke around the contour, so
   the plan's max is too small when both are on), and `max(shadow reach, glow reach)` (both are measured from the same
   outline shape, so the sum is never needed and only costs memory). Checked: `k1_the_margin_is_enough_for_200_random_stacks`
   and a one-off stress run of 3,000 random stacks (lengths up to 40, scales 0.3..2.3, not committed): the nearest
   non-zero pixel was never closer than 2 px to the border.
2. **Thresholds take the max with their source.** `clamp(solid + 0.5 - d)` is 0.5 (not 1) inside the shape when
   `solid = 0`, and the contour's `clamp(width + 0.5 - d)` likewise for width 0. Built: `max(S, threshold)` and
   `max(A, threshold)` (6.9 pitfall 12). The distance field of `S` / `1 - A` is computed only when some `solid >= 1`
   (ClassicSticker's spread 0 needs none: about 10 ms saved at 1024).
3. **Fill holes keeps the outer anti-aliased edge.** The plan sets every unreached pixel to 1; an edge pixel at >= 128
   is not reached either, so the die-cut's outer edge would lose its anti-aliasing (seen in the ring test). Built:
   unreached pixels that touch the reached outside keep their value; the rest (holes and the inside) become 255. Span
   (scanline) flood instead of a per-pixel stack (11 ms -> about 2 ms at 1164 x 1164).
4. **Bevel strength.** The plan's `n = (-dH/dx * depth, -dH/dy * depth, 1)` with `H` in 0..1 and `shade = n.L - sin e`
   used as coverage would give a 10-px bevel a slope of 0.1 at depth 1 and at most 50 % highlight at 30 degrees. Built
   as the reference: gradient times `depth * size` (depth 1 = slope about 1 at the edge, Photoshop's 100 %), highlight
   `min(shade / (1 - sin e), 1)`, shadow `min(-shade / sin e, 1)`. The flat interior is still exactly neutral (test).
5. **`Paint::Linear { gradient, angle_deg }`** instead of `Linear(Gradient)`: a gradient fill needs a direction (4.3a
   has one); 0 degrees = left to right.
6. **Gradient mapping by pixel centres.** `t` runs over the extreme pixel centres of the box (`half = ((w-1)|cos| +
   (h-1)|sin|) / 2`), so the first and last columns get the end stops exactly ("gradient endpoints exact").
7. **Resampler.** (a) Mip level chosen from the **larger** axis scale and only when both are below 0.5 (a mip level
   reduces both axes; this keeps either axis from being blurrier than needed). (b) The exact-copy path also covers
   quarter turns that land on the pixel grid: premultiply then un-premultiply is not exact for alpha < 255, so "rotate
   90 four times = identity" needs it. (c) A `clip` parameter (the render's output rect) and a cap of 2^26 output pixels.
8. **Shape raster** = a `CroppedBuf` in content pixels at the render scale (the content box `[0, size * s)`), grown by
   half the outline width plus one pixel, so a centred outline is not cut. `ShapeKind::Line` = a horizontal capsule
   through the box middle (the plan does not say where a line runs); the layer transform rotates it.
9. **Preset values** other than ClassicSticker are not in the plan; chosen: Outline = black outside stroke 6, Glow =
   warm outer glow 24 (80 %, spread 0.1), Badge = white contour 16 (smooth 0.7) + dark stroke 4 + shadow 35 % + bevel 6,
   ShadowOnly = shadow 50 % (distance 8, size 12). ClassicSticker's shadow: angle 120, distance 6, size 10, spread 0.
10. **A7 test reading.** "Inner shadow ... each change at least the border pixels": an inner shadow lit from 120 degrees
    cannot reach the bottom and right borders (the outside moves away from them), so the test checks the lit sides' (top
    and left) border pixels for the inner shadow and every border pixel for the inner glow and the bevel. "No value is
    NaN or infinite": the output is bytes; every non-finite distance becomes coverage 0 (`cov`), and the test runs scales
    0, -1, NaN, +inf and 1e-6 on a 1 x 1 picture without a panic.
11. **Perf row reading.** The 8.4 row says "ClassicSticker (contour 12, smooth 0.5, shadow)": budgeted as written (the
    512-px preset applied unscaled to 1024 / 2048 / 4096 bases). The preset scaled to the picture (what the Sticker tab
    will apply) is reported too, without a budget.
12. **Files.** The `fx` tests live in `sticker/fx_tests.rs` (a test-only child of `sticker`, like `wrap_tests.rs`), and
    the perf tests in `sticker/bench.rs`; `fx.rs` is 728 lines (target 700, ARCHITECTURE 2). `StyleStack::any_on`,
    `sanitized`, `scaled` (crate-internal) are extra helpers.

## Numbers measured

Perf (`cargo test -p wte_core --release --lib perf_k1 -- --ignored --test-threads=1 --nocapture`, this machine; the user
was running a game at the time, CPU load about 80 %, so earlier runs scattered: EDT 2048 between 29 and 123 ms):

| Operation (8.4) | Measured | Budget |
|---|---|---|
| `apply_styles` ClassicSticker 1024 x 1024 (margin 36) | 29.0 ms | 60 ms |
| the same 2048 x 2048 / 4096 x 4096 | 94.3 ms / 353.1 ms | 250 ms / 1.1 s |
| the same at the Preview size (652 px base, 724 x 724 out = 524,176 px) | 18.4 ms | 33 ms |
| peak memory of `apply_styles` at 4096 x 4096, ClassicSticker (counted allocations, output 66 MB included) | 150 MB | 256 MB |
| `distance_to_ink` 2048 x 2048 | 29.4 ms | 80 ms |
| reported only: ClassicSticker scaled to the picture, 1024 (margin 70) / 2048 (margin 138) | 33.0 ms / 116.8 ms | - |
| reported only: peak memory at 4096, preset scaled to the picture / every style on | 186 MB / 309 MB (outputs 82 / 95 MB) | - |

Before the optimisations of deviations 2 and 3 (plus parallel conversions and a no-division fast path in the compose)
the scaled 1024 case took 90-130 ms under the same load.

Accuracy (from the tests): contour of a radius-20 disc with width 10: every partially covered pixel puts the edge within
0.6 px of radius 30, the area radius within 0.6 px; stroke areas on a 40 x 40 square within 1 % (Outside 960 + 36 pi,
Inside 816, Center 480 + 9 pi / 444); ellipse, star, hexagon, rounded rectangle and capsule areas within 0.5 %; the
half-scale styled picture differs from the box-reduced full-scale one by at most 3 / 255 on average (premultiplied).

## Workspace runs (final state)

- `cargo test --workspace --no-fail-fast`: **1,444 passed, 1 failed (WA20), 51 ignored** (`wte_app` 524 / 0 / 24;
  `wte_core` 864 / 1 / 27; integration 15 + 9 + 8 + 24). Against P0 (1,411 / 1 / 48): +33 `k1_*` tests, +3 ignored
  `perf_k1_*`. The red test is `wrap_tests::wa20_...` only.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK; exe 28,025,344 bytes, SHA-256 `415d9726...1264` (names this build only;
  release builds here are not reproducible, see `baseline.md`).
- Identity sweep (8.5, `tools/objtabs/sweep.sh` with that exe, 21 jobs, off-screen, no focus) into
  `target\sticker\p1\sweep\`: **21 / 21 `"ok": true`**; `compare_baseline.py` against the P0 baseline: **216 / 216 files
  the same, PASS** (`target\sticker\p1\compare.txt`). Looked at `workflow_path` (Split view, chess set, UV view) and
  `stamp` (logo stamp with gizmo on the checker sphere): as before.
- Line endings: every new or changed file is LF (`git ls-files --eol`).
