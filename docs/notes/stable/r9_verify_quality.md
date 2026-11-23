# Round 9 gate — skeptic notes, lens: picture quality

Skeptic (Opus), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9` (`round9-wip` @ a8beea6, gate commit on
42996df). Exe `target\round9\gate\bin\wte_r9.exe`, SHA-256 4F9E1422...B358B2 (re-hashed, equal to the report). No cargo
run, no tracked file edited. All outputs under `target\round9\verify_quality\`.

## What I ran

- `wrap_surface` with PATH on all 13 sweep models, one process at a time, `WTE_FRAME_LOG=1`, 1280 x 800
  (`tools\sweep.sh`, `sweep2\<model>\`, driver log `sweep2_driver.log`): 10 ok, 3 exit 2 (Lantern, ABeautifulGame,
  many_islands: the step-10 coverage assert, as the gate). high_poly_sphere passed step 6b (the gate's run-1 flake did not
  recur here).
- high_poly_sphere with the gate's 2,048 px checker model (`sweep2_2048\`): ok.
- Reference picture: `ref\cells_512x256.png` and `ref\cells_labelled.png` (Python port of `samples::cells_picture`,
  checked against `samples.rs:876-915`: same levels, stride-13 shuffle, gradient 8).
- Pixel tools: `tools\cmp_dirs.py`, `tools\pxdiff.py`, `tools\sheet.py`, `tools\cellgrid.py` (nearest cell colour per
  grid point, chromaticity), `tools\uv_mirror.py` (UV footprint raster of a GLB, mirror test of texel-sharing pairs).
  Crops in `crops\` (4-5 x unless noted).

## Reproducibility of the gate's sweep

- Every one of the 7 PNGs per model in `sweep2\` is pixel-identical (> 2 levels) to the gate's `sweep_r9\`, except
  ngons_tiling `shot.png` (6,806 px, toast box x 787-974, y 697-735) and ABeautifulGame `shot_off.png` (2 px at x
  1270-1271, y 47: UI edge). `sweep2_2048\` is identical to `sweep_r9_2048\` (all 7 shots).
- Parsed step-10 numbers (`sweep2_table.txt`, `tools\sweep_parse.py`): every quality field (verts, tris, seams, by rule,
  stretch, rejected, covered, reached_edge, ends, lost_cut, ink, dup, scatter, jumps, flipped, passed) equal to the gate's
  `logs\sweep_r9_table.txt` and to a re-parse of `sweep_r9\` on all 26 lines. The report's sweep table matches the raw
  logs line by line (checked every number of every row).
- 2,048 px sphere: ink 1.1024, dup 0, scatter 0, seams 0, rejected 0.00 %, covered 1.0 (equal to the report).

## Core WA numbers vs `logs\wa_tests_nocapture.log`

All quoted numbers match: WA4 30/45/60 deg, WA4b (1.1149, 0.799..1.481 against its own limits 0.85-1.15 / 0.65-1.50 in
`wrap_tests.rs:1981`; margin on cells only 0.019), WA20, WA21 (0.9966, 0.315 %, 3 of 6,816 flipped), WA22, WA23, WA31,
WA32 (0.6183 / 0.6187 / 0.5588, covered 0.664 / 0.790 / 0.720), WA33, WA34, WA35 (0.92368 max 51; 0.82017 max 255),
WA17 / WA24 (0.915 / 0.9146). Nit: the line printed by the WA4b case (`WA4 4 periods, 60 deg: ink / picture 1.1149
(limit 0.90-1.10), cells ... (limit 0.75-1.30)`, log line 105) shows the 2-period limits; the assert uses WA4b's own.
Read alone that line looks red.

## Shot readings (my own, crops in `crops\`)

| Model | Reading | Gate's claim |
|---|---|---|
| high_poly_sphere (`hps_3way.png`, `hps_left4x.png`, `hps_right4x.png`) | all 32 cells once, rows continuous, no cut, no bare gap, nothing outside the outline; 2,048 px shot same cells, same outline | confirmed |
| torus_knot (`tk_3way.png`) | column 5 (olive, tan, green, sea green) down the visible tube, column 4 left, a magenta sliver right; outline crosses the tube top and bottom; no repeat, no gap on the visible side. Meeting line on the back: not visible, not verified. A faint cyan haze above the top outline = 256 px texture filtering (round 8 has the same) | confirmed (meeting line not verifiable from the shot) |
| Avocado (`avo_on_4x.png`, `avo_off_4x.png`) | all 32 cells; cyan, tan, green cells continue across the seed's rim onto the flesh (one cell split, not twice); the light-blue cell shows as a wedge on the flesh and a patch on the seed with the cavity rim between: undecidable from pixels (dup 0.787 %) | confirmed, sliver caveat as the gate wrote |
| DamagedHelmet (`dh_shot_3x.png`, `dh_shot_off_3x.png`, `dh_r8_r9.png`) | visor centre c3-c6 continuous; a tilted black / purple / red copy over the c2-c3 area and row-1 cells above the outline: mirrored UVs. Independently checked: `uv_mirror.py` finds 7,765 texel-sharing triangle pairs, **99.4 % of them x-mirror images** (0.9 % same place); the log says 86.7 % of the surface shares texels. Right side: bare dark visor inside the outline where round 8 had cells (ink 1.09 -> 0.84), white bare bars inside the navy cell | confirmed (mirror = model property, verified) |
| ABeautifulGame (`abg_r8_r9.png`, `abg_on_board5x.png`, `abg_r67_r9off.png`, grid `cellgrid.py`) | columns 4-8 on the board, rows continuous — **but the picture on the board is turned about 90 deg clockwise against the stamp's frame**: row 1 (navy, green, pink) lies at the right, row 4 at the left, each column runs left-right across the screen (grid at y 412-437: c5r4 -> c5r3 left to right; c7r4, c7r3, c7r2, c8r2, c8r1 left to right). Round 8 and round 6+7 lay the rows parallel to the frame's top edge. All white pawns identical (instancing) | "board rows continuous" literally holds; the rotation is **missed** |
| Lantern (`lantern_3way.png`) | front and right faces continuous across the box edge, rows level; two dark wedge slivers inside the faces; cap top and plinth right bare; no repeated cell visible | confirmed |
| klein_bottle (`klein_3way.png`, `klein_r8_r9_4x.png`) | c1-c3 on the outer tube, c4-c7 into the hole, no repeat, no gap; nearly the same as round 8 (6,260 px differ) | confirmed |
| WaterBottle (`wb_top.png`) | one whole picture; top band differs from round 8 by 1,052 px, same cells | confirmed |
| nonmanifold (`nm_3way.png`, `nm_book5x.png`) | front-right page bare (stripes) where round 8 had pale cyan / pale yellow; top-left page also bare; nothing twice | confirmed (not met) |
| mobius, many_islands, flat_annulus, ngons_tiling | mobius 1,173 px vs round 8 (same cells), many_islands 112 px, flat_annulus and ngons_tiling 0 px vs round 8 | "no change for the worse" confirmed |

## Panel lines (`crops\panels.png`, Jump on state)

- Shared-texel line present on DamagedHelmet, ABeautifulGame, ngons_tiling; absent on Avocado, Lantern, klein,
  nonmanifold, high_poly_sphere, WaterBottle, many_islands: as claimed.
- "Longer than the way round": torus_knot 43 %, ABeautifulGame 59 %, many_islands 33 %: as claimed.
- **Missed**: the "runs off the surface: an edge, a hole or the end of the part" line fires on `covered < 0.98` alone
  (`tool_options.rs:143-146`), so it names the wrong cause on more models than the torus knot: Lantern "12 % ... an edge"
  with `reached_edge false` (the loss is cut wedges, lost_cut 11.23 %); many_islands shows four warnings at once ("97 %
  ... runs off", "The ends meet at the back", "Longer than the way round: 33 % shown", "Continued onto 2 separate
  parts"). Conversely the panel is silent where the owner leaves visible holes: DamagedHelmet Jump on (covered 1.0,
  rejected 16.95 %, ink 0.84: no line) and nonmanifold (says 37 % runs off; 70 % rejected and ink 0.19).

## Missed numbers

- ABeautifulGame Jump on **flipped 69,120 of 323,172 laid triangles (21 %)**, round 8 49,438: 40 % more folded
  triangles. The report's sweep table has no flipped column and no note mentions it (Jump off: 2,133 -> 6, better).
- Remaining duplicates (Avocado 0.787 %, Lantern 1.676 %, nonmanifold 1.523 %, ABeautifulGame 2.674 %): the report does
  not say whether they are the wrap's or the model's; I could not decide it either (not verified).

## Worse than round 8 in the pixels

ABeautifulGame Jump on (board picture turned ~90 deg and cut to columns 4-8), DamagedHelmet Jump on (bare visor inside the
outline), Lantern (wedge slivers, bare cap / plinth right), nonmanifold (bare front page). All but the ABeautifulGame
rotation are in the report.
