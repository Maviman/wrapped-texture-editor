# Round 8 gate report: skeptic check, picture quality

Date 2026-11-22, about 05:15-05:45 local. Checker: verify_quality (independent; did not build round 8). No product code
changed, no cargo command run (so no BUILD_LOCK taken). Exe `target\round8\wte_round8.exe`, SHA-256 checked:
`19D7844B4BDCD73ECE7327FB3F677CD58695A8314EA80F42CB194CDA97E54B2A`. Own runs: `target\round8\verify_quality\`.
Helper scripts (crops, cells reference, UV-overlap counter) in my scratchpad, not in the tree.

## What I ran

| Run | Model (PATH) | Result | Notes |
|---|---|---|---|
| 1 | `high_poly_sphere.obj` (256 x 256 texture, as in the sweep) | ok, 38 s | `shot.png` pixel-identical to the gate's sweep shot |
| 2 | `Avocado.glb` | assert-failed (step 10), 8 s | identical shot; Jump on ink 1.5295, dup 52.591 %, scatter 5.865 % (same as gate) |
| 3 | `DamagedHelmet.glb` | ok, 8 s | identical shot; Jump on ink 1.0941, dup 0.000 %, scatter 0.000 %, seams 140, rejected 1.18 % |
| 4 | `high_poly_sphere.obj` copied to `verify_quality\hps2048_model\` with the same checker scaled to 2048 x 2048 (test input only) | ok, 28 s | see below |

Nothing of ours ran alongside. During run 4 a `wte_round6_7.exe` (pid 27460, not started by me, not touched) was running,
so run 4's times are indicative only; free memory 0.9-1.3 GB. Quality numbers are deterministic (runs 1-3 equal the gate).

Log lines, step 10 (Jump off / on):
- high_poly_sphere 256: ink 1.0427 / 1.0427, dup 0.000 %, scatter 0.103 %, **bins 1,020**, seams 176 [S2 49, S5 127],
  rejected 6.40 %, 122,496 verts, param 11,462 / 11,983 ms, applied 11,542 / 12,077 ms.
- high_poly_sphere 2048 (same spot, same frame on screen): ink 1.0516, dup 0.000 %, scatter 0.000 %, **bins 32,768**,
  **seams 0, rejected 0.00 %**, 77,721 verts, stretch 30.5 %, param 6,017 / 6,370 ms, applied 6,108 / 6,454 ms.
- Avocado: off ink 0.5288 dup 0 scatter 0 (17,533 bins); on ink 1.5295 dup 52.591 % scatter 5.865 % (32,768 bins).
- DamagedHelmet: off ink 0.5065, covered 0.655 (round 6: 1.0185, covered 1.000); on ink 1.0941 dup 0 scatter 0.

## Claim by claim (pixels read at 4-5 x, next to a rendered reference of the 8 x 4 `cells_colors`)

1. **high_poly_sphere "one whole picture, each cell once, rows continuous"**: **confirmed on the visible side.** All 32
   cells once in the right order, rows continuous, cells bent over the bumps, no hole, no scrap. The 2048 run shows the
   same, crisp, with dup / scatter 0 at full resolution. Caveats: (a) the sweep's 256 x 256 texture gives about 9 texels
   per cell and the dup metric's bins are about one cell wide (1,020 bins, threshold 6c = about 1.07 cell widths), so the
   sweep alone could not see a sub-cell tear; (b) at 256 x 256 the same spot is laid with 176 seam edges inside the
   picture (127 S5 "handle" cuts and 49 S2 cuts on a ball with no handle, plan 2.4 says 0 for the bumpy ball) and 6.4 %
   of the laid area rejected, while at 2048 x 2048 it is 0 / 0 with 37 % fewer vertices and half the time. The
   parameterization depends on the texture resolution; the report does not say so. Not visible in either shot. The
   back of the ball is not visible in either.
2. **Avocado (Jump on) "the visible picture is one piece ... the flesh hidden under the seed keeps its own copy"**:
   **refuted.** In the shot (crop at 5 x) the cut face *above and left of the seed* shows light blue (cell 1), orange
   (2), cyan (3), purple (8), red (16) and crimson (24), and the seed dome shows the same six cells again (light blue,
   orange, cyan at its top, purple / red / crimson down its left flank). The copy is visible from the default view, not
   hidden under the seed. Avocado's UVs do not overlap (1,024^2 raster: 7 of 678,224 texels covered twice), so this is
   the wrap's own duplicate (dup 52.6 % agrees). "What to try by hand" 1 sets a wrong expectation.
3. **DamagedHelmet (Jump on) "not one whole picture"**: verdict **confirmed**, cause **refuted / incomplete.** On the
   visor's left the black cell appears twice, purple twice, red twice, a pink wedge, and cells continue outside the
   outline on the left flank. The report blames a fold plus 9 jumps and says the metric misses them. The model's UVs
   are **mirrored**: 6,942 pairs of triangles have identical UVs with positions mirrored across x = 0 (77.8 % of the
   surface area, 73.8 % of the central |x| < 0.4 part); on a 1,024^2 raster 708,657 of 709,453 covered texels belong to
   exactly two triangles. Every texel the stamp paints shows on both halves; `stamp_wrap_quality` takes one surface
   point per texel (`locate`), so dup / scatter cannot see it. No wrap change fixes this; the planner should know.
   Also: Jump off covered 0.655 / ink 0.5065 vs round 6's 1.000 / 1.0185: with Jump off about a third of the picture
   now has no surface on the helmet (no shot of that state exists; the sweep's ink limit has no lower bound).
4. **ABeautifulGame (Jump on) "not one whole picture; pieces in and beside the footprint carry blobs"**: verdict
   **confirmed**, cause **incomplete.** The glTF instances its pieces: `Pawn_Body_Shared` and `Pawn_Top_Shared` are used
   by 8 nodes each, castles / knights / bishops by 2. In the shot all eight white pawns carry the same lime / purple /
   green pattern, both white bishops are cyan and both white castles have an orange base, also those far right of the
   footprint. Those blobs come from shared texels, not from Jump leaving the footprint. The board inside the outline:
   rows continuous (confirmed).
5. **klein_bottle "one piece on the visible side"**: **confirmed for what is visible**, but the visible part is not the
   whole picture: column 8 and the top row of columns 5-7 are not visible (inside the hole / behind), and the outline
   runs across the middle of the visible picture with a hook enclosing bare blue tube on the right. "The 4.1 % repeat
   presumably lies on the hidden side" is an assumption with no evidence (no other view exists); treated as unverified.
6. **WaterBottle "one whole picture"**: **confirmed** (8 x 4, rows straight, no cut, no repeat). The verdict's "the water
   bottle label is still exact" is **not supported**: the sweep stamp is relaxed (stretch 2.1 %, ink 1.0017), not the
   exact path, and gate 1's compat check found the logo moves 1-3 texels (71.6 % within 2/255).
7. **torus_knot**: the report's reading (one short tube section, 60 % of the picture off the surface, not recognisable)
   is **confirmed**. Covered fell from 1.000 (round 6) to 0.404; the picture largely disappears, the opposite of the
   user's request, yet the sweep table marks it "met (numbers)" and the verdict's five failing models omit it and
   DamagedHelmet. By plan 8.1 / 7.1.3 (screenshots must show one whole picture) at least 7 of 13 models fail.
8. **"The ink ratio dropped on every curved model that failed in round 6"**: numerically true, but misleading as a
   quality statement: on torus_knot (1.87 -> 0.41) and DamagedHelmet Jump off (1.02 -> 0.51) the drop is the picture
   losing surface, not fewer repeats.
9. **WA4 at 60 degrees asserts the original limits**: **confirmed.** `wa4_r8_bumpy_plates_receive_the_picture_once_whole_and_smooth`
   (`work\merged\crates\wte_core\src\wrap_tests.rs` 1918-1943) uses round 6's setup (`bumpy_plate_periods(64, a, 2)`,
   a = tan(60 deg) / (2 pi x 2), half 0.3 x 0.15, sample unchanged vs the main tree) and asserts ink 0.90-1.10, every
   cell 0.75-1.30, smear <= 1 %, covered >= 0.999, dup <= 1 %, scatter <= 2 %, seams 0, relaxed (relaxed exempt only at
   30 deg). It collects failures and asserts at the end, so it really fails (gate log `test1.log` 1180). Note: the
   report's "a cell 1.6 x" understates it: 9 of 32 cells exceed 1.30 (1.31, 1.59, 1.34, 1.35, 1.60, 1.35, 1.38, 1.35,
   1.27 ok), max 1.599. The round-6 test `wa4_a_...` still exists and still asserts 30 deg only (harmless).
   WA20's test asserts the plan's limits too (ink first; it stops there). The plan's own proxy for high_poly_sphere,
   WA20, fails "once": dup 10.7 %, scatter 4.08 %, a cell 3.3 x its share, 45 flipped, 22.2 % of the laid area rejected.
10. **WA23 "met"**: the cone's covered limit is asserted as 0.95 x 0.447 = 0.425, not the plan's 0.70. The geometric
    argument (161 deg round the tip) is plausible, but it is a changed limit and should be listed as "plan limit not
    asserted", not "met".

## Not done
- No view of the back side of any model (the scenario has a fixed camera; no OS input allowed).
- Lantern, nonmanifold, ngons, mobius, many_islands, flat_annulus shots not re-read in depth (report already fails or
  they are flat cases).
- No requests for files I do not own.
