# Check note: P2 - Selection, cut-out and filter maths (STICKER_PLAN 7 "P2")

Independent checker, 2026-11-25, branch `stickers`, builder commit 78da4d1. Read: plan 0, 1, 2.7, 3.4, 4.0, 4.4, 4.5,
4.9, 7 "P2"; the builder note; the three new files line by line; the ARCHITECTURE diff (1.3, 1.12, 2.1, 5.27, 6.12,
9.1, 10); PhotoCraft's defringe (`pc020\crates\engine\src\layer_menu_cmds.rs:225-264`) for comparison.

## Plan 7 "P2" item by item

| Item | Where | Verdict |
|---|---|---|
| Files `sticker/{select,cutout,filter}.rs` | as named | present, licence notice line 1 in all three |
| 3.4 core (`Selection { mask, bounds }`, limit) | `select.rs` | ok |
| 4.4 match / contiguous / non-contiguous / magic eraser / fill / grow / shrink / feather / invert / marquee / lasso | `select.rs` | ok, **except anti-alias** (finding 3, fixed) |
| 4.5 colour to alpha, mattes | `cutout.rs` | ok (formulas checked against the plan) |
| 4.5 defringe | `cutout.rs` | **wrong on soft shadows / glows** (finding 1, fixed; deviation from the plan's formula) |
| 4.5 remove background (steps 1-5) | `cutout.rs` | ok with the builder's documented deviations (cluster-seeded colours, normalised soft edge); I agree with both |
| 4.9 blur (three boxes, premultiplied u16, replicated edges), sharpen, gradient | `filter.rs` | ok (u16 range: `c * a <= 65,025`, running sums in u32; box rule = the standard one) |
| "With a selection the result is mixed with the original by mask / 255" | `mix_px` | **unchanged pixels drifted** (finding 2, fixed) |
| ARCHITECTURE 1.3, 5.27, new 6.12, 9.1 | | present; updated for the fixes below |
| Every `k2_` test sentence of 7 "P2" | 23 tests | present; the matte deviation (re-matte within 1 level) is sound: at alpha 32 the byte rounding of P alone is ~4 levels of Q |
| Checker: own pictures (noisy white background) | new `k2c_remove_background_on_a_noisy_white_background` | passes: light yellow, dark blue and grey objects on white with 20 levels of per-channel noise, IoU >= 0.98, no background speck left, no object pixel below 250 |
| Checker: 4096 x 4096 one-colour flood completes (perf) | `perf_k2_flood_fill_worst_case_4096` | 39 / 44 ms (budget 400); comb worst case 316 ms |
| Checker: determinism | the three thread-pool tests (1 vs 8 threads; all three pictures are above `PAR_PIXELS`, so the parallel paths run) | ok; the new `mix_px` is integer only |

## Findings

1. **Major, fixed - Defringe recoloured soft shadows and glows anywhere in the picture.** The plan's rule (edge =
   alpha > 0 and `dist_in <= n` with `dist_in` on the 128 threshold; interior = alpha >= 250) makes **every** pixel with
   alpha < 128 an edge pixel at any depth (`dist_in = 0`). Probe: a red square and, 15 px away, a soft black shadow
   (alpha 100): Defringe 1 px turned all 1,600 shadow pixels red. A glow touching the object, an isolated faint stroke:
   the same. Fix (deviation from the plan formula, same idea as PhotoCraft's defringe): depth `d0` = distance to a
   **fully transparent** pixel (no pad: the picture border is still not an edge); edge = alpha > 0 and `d0 <= n`;
   interior = alpha > 0 and `d0 > n`; an edge pixel takes the nearest interior pixel's RGB only when it is at most
   `n + 2` px away. The plan's halo test still passes unchanged; new test
   `k2c_defringe_leaves_shadows_glows_and_faint_strokes_alone` (shadow, glow, faint stroke untouched; a 2-px white rim
   on a red square is replaced). Fails on 78da4d1.
2. **Minor, fixed - `mix_px` changed unchanged pixels.** It mixed 8-bit premultiplied values and un-premultiplied
   them, so `mix_px(p, p, k) != p` for 3,605 of 5,080 probed faint pixels (colour off by up to 12 levels at alpha 10):
   every destructive command limited by a soft (feathered / anti-aliased) selection recoloured faint pixels it did not
   change. Now exact integer maths on `c * a` (u32): `before == after` returns `before`; `k = 0 / 255` exact as before;
   the existing expectations are unchanged. Test `k2c_mix_px_keeps_unchanged_pixels_and_faint_colours` (all alphas,
   all `k`, plus a feathered-selection blur of a constant faint picture = the same picture). Fails on 78da4d1.
3. **Minor, fixed - wand anti-alias gave 255 to matching pixels the contiguous flood did not reach.** Plan 4.4 gives
   the partial value to pixels that **do not match**; the builder applied the formula to every neighbour (note
   deviation 5), so a matching pixel touching the region only diagonally was fully selected (a checkerboard wand picked
   an X of five pixels). Now such pixels stay 0; non-matching neighbours get the fringe as before. Test
   `k2c_wand_anti_alias_gives_no_value_to_unreached_matching_pixels`. Fails on 78da4d1.
4. **Minor, not fixed (plan formula, documented here) - grow / shrink of a faint selection.** Grow and shrink threshold
   at 128 (`dist_out` / `dist_in`): a selection whose values are all below 128 does not grow, and shrinking it empties
   it. Only reachable with hand-made masks (a feather of a large selection keeps values >= 128 inside). For P7b: if a
   "Selection from layer alpha" of a faint layer becomes common, grow / shrink should threshold at half the mask's
   maximum.
5. **Minor, not fixed - memory.** Blur holds three u16x4 planes at peak (about 400 MB at 4096 x 4096, 8 bytes per pixel
   each, plus input and output); the P2 API takes artboard-size buffers (<= 4096), so 8192 decode-size pictures do not
   reach it unless a caller passes a raw layer. Remove Background with Follow gradients uses a depth-first stack of
   pixel indices (up to 8 bytes per pixel in the worst case). Both linear; recorded for P3b / P7b (pass artboard-space
   buffers only).

Nothing found in: `flood_plane` (each run re-checked at pop, terminates, linear); `box_pass` running sums (no u32
underflow: the subtracted value is in the sum); `gaussian_boxes` (standard rule; radius 0.5 = no blur, documented
range); transpose blocking; `remove_background` refusals and frame of 1-px pictures; tiny pictures (1x1, 1x50, 50x1,
2x3 blur / sharpen / mask blur); selection-size mismatch handling (always "nothing changes"); `over` used by Fill and
the gradient; licence lines; no `ArtCraft` under `crates/`; `Cargo.lock` unchanged; LF everywhere.

## Runs (after the fixes)

- `cargo test --workspace --no-fail-fast`: **1,474 passed, 1 failed (WA20 only), 54 ignored** (`wte_app` 524 / 0 / 24; `wte_core` 894 / 1 /
  30; integration 15 + 9 + 8 + 24). Builder: 1,470 / 1 / 54; +4 `k2c_*` tests. Log `target\sticker\check-p2	est.txt`.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK; exe 28,041,728 bytes, SHA-256 `97d2184d...3438` (names this build only).
- Perf (release, serial, `perf_k2`): wand 11.6 ms (60), magic eraser 16.7 ms (60), remove background 46.0 ms (150),
  blur radius 16 at 2048^2 56.1 ms (150), flood 4096^2 one colour 39.2 / 44.0 ms (400), comb 315.6 ms; Follow
  gradients + enclosed 110.9 ms (reported only). `target\sticker\check-p2\perf.txt`.
- Identity sweep (8.5; P2 has no scenario of its own): `tools/objtabs/sweep.sh` with that exe and `tools/sticker/jobs_identity.txt` (off-screen, no
  focus) into `target\sticker\check-p2\sweep\`: **21 / 21 `"ok": true`**, exit 0 each; `compare_baseline.py` against the
  P0 baseline: **216 / 216 files the same, PASS**. Looked at `stamp` (logo stamp and gizmo on the checker sphere) and
  `workflow_path` (Split view, chess set, UV view): unchanged. P2 touches no `Document` code, so this is the expected
  result.

Verdict: **pass after fixes** (one major and two minors fixed with tests; two minors recorded for later packages).
