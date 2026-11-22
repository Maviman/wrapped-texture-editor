# Round 6 gate report — Wrapped Texture Editor ("wrap on surface")

Date 2026-11-21. Plan: `target\round6_design\ROUND6_PLAN.md`. Notes: `notes\pkg_a.md`, `notes\pkg_b.md` (builders and
checks), `notes\gate1.md` (merge, tests, scenarios, pixel identity, compatibility), `notes\gate2.md` (measurements).
Raw runs: `runs_gate\` (stage 1) and `runs_gate2\` (stage 2; analyses in `runs_gate2\analysis\`).

## Verdict

**Wrap works and is delivered with both options, Project is unchanged, and files stay compatible. It is fast on light
and medium models. It is not fast enough on the 1.5-million-triangle chess model, and the picture breaks up on
strongly bumpy or knotted shapes.**

- **What works.** Every 3D stamp and 3D text has the switch **Project | Wrap**, plus the two Wrap options **Jump small
  gaps** and **Longer than the way round: Ends meet | Overlap**. Both options are saved, can be undone, are kept when
  switching to Project and back, and are greyed out in Project. On flat, cylindrical, conical and folded surfaces the
  picture lies exactly on the surface (core tests: 100 % of texels within 2/255 of the exact unrolling). It goes across
  UV seams without a visible line, wraps round a bottle like a label (WaterBottle), and winds like tape with Overlap.
  All 13 models of the sweep run without a panic or an error.
- **Unchanged.** Project mode gives the same pixels as round 5. Every 3D and 2D view pixel and every exported file of the
  seven round-5 scenarios is identical. Round-5 projects open unchanged. Round-6 projects without a wrapped stamp open
  in round 5. A project with a wrapped stamp is refused by round 5 with a readable message.
- **Not met: speed on ABeautifulGame.** Each move, scale or turn rebuilds the surface map (the "parameterization"), and
  on this model that costs 55-1,160 ms. Final bakes of a moved, scaled or turned text take 1.7-3.5 x as long as Project
  (limit 1.5 x). A drag has frames of about 140 ms (Project: about 50 ms; limit about 80 ms). The core probe's 20 %
  decal needs 150 ms on the board (limit 150) and 900-980 ms on a chess piece. Jump small gaps costs 2.5-2.7 x
  (limit 1.5 x). Lighter models are well inside every limit (built-in sphere, ngons_tiling, the sweep).
- **Not met: quality on strongly curved surfaces.** On a heavily bumped sphere (`high_poly_sphere`) the picture is
  scattered over the whole ball. On the Klein bottle it appears in many torn pieces. On Avocado's seed and on DamagedHelmet
  it shows cut lines. This is the known WA4 limit: the core meets WA4 at 30-degree bumps, not at the plan's 45-60 degrees.
  With Jump small gaps on, models made of many touching parts (DamagedHelmet, the chess set) get the picture on every
  part within reach.
- **Gate changes.** No product code was changed in this stage. Two pieces of test code were changed (details under
  "Gate changes"). No acceptance failure had a small, contained fix.

**Measuring conditions.** Another agent's round-7 measurements and builds ran in bursts during stage 2. The driver
waited for them (up to 5 minutes) and marks the runs they overlapped as "loaded". G5a, G5c and G5d ran on an idle
machine. G5b's first Project run, the second WA15 run and some sweep runs were loaded.

## Exe and tests

- Final exe: `target\round6\wte_round6.exe`, **SHA-256 6CE65B55F15861BF91AF2DA5FDFB493482F8F050CB51B801D042518E57FD5433**.
  It differs from gate 1's exe (F7CF77DF...31C8, kept as `runs_gate2\wte_round6_gate1.exe`) only in the `text_warp`
  self-test check (see "Gate changes"). G5a, G5b, G5c and the Project runs of G5d were measured with gate 1's exe. The
  Wrap runs of G5d, G6 and G8 were measured with the final exe.
- Tests (`merged`, after the gate changes; `runs_gate2\build_logs\test1.log`, 19:26-19:47): **1,107 passed, 0 failed,
  33 ignored** (wte_app 342 / 16, wte_core lib 718 / 17, brush_tips 15, end_to_end 9, project_roundtrip 23). `cargo clippy
  --workspace --all-targets`: **0 warnings** (`clippy1.log`). `cargo build --release -p wte_app`: ok (`release2.log`).
  WA15 (ignored) was run twice in release (`runs_gate2\wa15_run1.txt`, `wa15_run2.txt`). `text_warp` on the built-in
  sample with the final exe: ok, and `shot.png` / `shot_warp3d.png` are identical to gate 1's (0 pixels differ).
  Another agent's cargo build ran at the end of the test run (free memory 0.85 GB).
- Gate 1 (before the changes): 1,107 passed, 0 failed, 33 ignored; clippy 0 warnings; no `todo!` / `unimplemented!`.
  Round 5: 1,067 / 32.

## Acceptance lines

"P" = Project, "W" = Wrap (`WTE_STAMP_MODE=wrap`), same exe, same clicks. Times: "applied after" = Final edit to
swapped picture. Two runs per mode unless noted.

### Core accuracy and behaviour (package A, `wte_core::wrap_tests`; plan 4.7)

Numbers are from the builder's run (`notes\pkg_a.md`, acceptance table). check:A and check:B reran the full suite
(`notes\pkg_a.md` "Check and fix", `notes\pkg_b.md` "Check and fix"). Gate 1 and this stage reran it again (all
pass).

| # | What | Limit | Measured | Verdict |
|---|---|---|---|---|
| WA1 | Cylinder vs exact unrolling, off and on the UV seam | 99.9 % within 2/255, max 8 | 100 % within 2/255, max 1 (429k / 429k texels) | **met** |
| WA2 | Ink area / picture; per cell | 0.99-1.01; 0.97-1.03 | 1.0010; 0.970-1.023 (projected: 0.404) | **met** |
| WA3 | 1.5 x round a cylinder | band >= 99 %, met_itself, smear <= 0.5 %, cuts <= 2 per row | 100 %, yes, 0.0002 %, 2 | **met** |
| WA4 | Bumpy plate, max slope 60 degrees | ink 0.90-1.10, cells 0.75-1.30, smear <= 1 %, covered >= 0.999 | **60 and 45 degrees: ink 4.6-5.9 x, cells up to 13 x, ~15 % of triangles cut.** 30 degrees: ink 1.074, cells 1.00-1.27, smear 0, covered 1.0 | **not met** at the plan's slope (the test asserts 30 degrees) |
| WA5 | Cube: across an edge; over a corner | edge as WA1; corner smear <= 0.5 %, covered >= 0.70, cuts <= 8 | edge 100 % within 2, max 1; corner smear 0, covered 1.0, 1 cut | **met** |
| WA6 | UV seams: identical coordinates; sphere seam score | <= 1.25 x interior + 2 | identical; 0.44 vs 12.99 (app scenario: 0.428 vs 11.84) | **met** |
| WA7 | 19 bad meshes, bad pictures | no panic, finite, <= 200 ms | all finite, no panic, slowest 5.8 ms (check:A's own set: 72.8 ms under load) | **met** (its wall-clock assert failed once under heavy load in package B) |
| WA8 | Two blocks, cracked quads, thin slab | other block 0; WA1 limits | 0 texels; 100 % within 2, max 1 / max 0 | **met** |
| WA9 | 1 thread = N threads, repeat runs | equal | equal | **met** |
| WA10 | Project -> Wrap -> Project, undo / redo, duplicate, merge down, rasterize | equal | equal | **met** |
| WA11 | Reuse: scale 1.2, tint, flip reuse; move, turn recompute; reused = fresh | equal | as stated (a scale reuses only inside its reach bucket; a text edit is not tested separately) | **met** |
| WA12 | Deferred = synchronous Finals (wrapped image, wrapped Arc text) | equal | equal | **met** |
| WA13 | Format round trip, keys only when set, `required_reader`, unknown mode, round-5 fixtures | as stated | all pass | **met** |
| WA14 | Old tests unchanged | equal | all pass; 5 expectations follow FORMAT_VERSION 3, no pixel expectation changed | **met** |
| WA15 | Probe (gate runs it) | reported, limits G5e / G5f | below | see G5e / G5f |
| WA16 | Jump small gaps, coplanar blocks, gap 0.5 % | jumps 1; 99.9 % within 2, max 8; 2 % gap: on = off; one part: on = off | jumps 1; 100 % within 2, max 0; equal; equal | **met** |
| WA17 | Cube on floor, Jump small gaps | on: cube receives, ink >= 0.95, smear <= 0.5 %; off: nothing | 50,740 cube texels, ink 1.55, smear 0.005 %; off 0 | **met** |
| WA18 | Overlap on the cylinder, 1.5 x and 2.5 x | turns 3; 99.9 % within 2, max 8; flip_x; see-through; 0.9 x: equal bytes | turns 3; 100 % within 2, max 1 in all 8 cases; equal | **met** |
| WA19 | 20 x round, options together | turns 8, limited, <= 8 x one turn + 50 ms; deterministic; one history entry each | turns 8, limited, 53.8 ms vs 106 ms alone | **met** (its wall-clock assert now retries up to 4 times, gate 1) |

### Unchanged and compatible (gate 1, `notes\gate1.md`)

| # | Line | Measured | Verdict | From |
|---|---|---|---|---|
| G2 | Seven round-5 scenarios pixel-identical to `wte_round5.exe`, exports byte-identical | Every 3D / 2D view pixel identical; `export_run` 7 / 7 files byte-identical, `check_exports.py` PASS; `final_bake_async` exports identical; `.wtep` differs only in `format_version` 3 and `saved_unix`. The screenshots differ in the **side panel** only: `stamp` 4,576 px and `final_bake_async` 4,534 px (the new "On the model" row and the greyed Wrap option rows), `text_styles` 18 px (shorter scroll thumb) | **met for the views and files**; whole-screenshot identity is not met because of the new side-panel rows (by design) | `runs_gate\g2_compare.txt` |
| G3 | (not defined in the plan) | | | |
| G4 | Round-5 projects open in round 6; round-6 projects open in round 5; wrapped projects refused | identical shots; only the "newer version (format 3)" warning, composite equal (dim-screen fit, residual max 3 levels); refused with "newer version (format 3, supported 2)" | **met** | `runs_gate\g4_compare.txt`, `g4_dim_check.py` |

### Speed, memory, robustness (this stage, `notes\gate2.md`)

| # | Line | Wrap | Project | Verdict | From |
|---|---|---|---|---|---|
| G5a | Built-in sample, `stamp`: Wrap Full applied after <= 1.5 x P and <= 80 ms; Preview frames <= 30 ms | Run 2: 29 / 31 / 31 / 31 / 30 ms. Run 1: **84** / 33 / 31 / 31 / 32 ms (the first bake: a 49.7-ms first parameterization, 0.5 ms in run 2). Drag frames: max 8.6 / 5.2 ms | 30-32 ms | **met in run 2; run 1 misses on its first bake** (a one-off cold start; gate 1 saw one of 202 ms) | `analysis\g5a.txt` |
| G5b | ngons_tiling `workflow_path`: applied after <= 1.5 x P; no frame > 300 ms between started and applied; drag frame <= P + max(30 ms, 25 %) | image 32 / 30, text 70 / 69, move 65 / 68, scale 71 / 70, rotate 98 / 82 ms; 0 frames > 300 ms; drag max 64.4 / 64.7 ms | 1,014 / 281, 208 / 137, 251 / 149, 992 / 1,360, 448 / 332 ms (run 1 loaded); drag max 224.5 / 161.2 ms | **met** (Wrap is faster because it paints less: the tiles are separate parts, so the picture covers only 11-30 % of its rectangle) | `analysis\g5b.txt` |
| G5c | ABeautifulGame `workflow_path`, same three lines; image worker bake <= 2.0 s | image 942 / 930 (worker 927 / 885 ms); text 70 / 87; move 88 / 94; scale 161 / 155; rotate 166 / 164 ms; started..applied frames max 24.6 ms; **drag frames max 142.2 / 144.1 ms** | image 1,361 / 1,359; text 20 / 36; move 67 / 61; scale 103 / 93; rotate 98 / 114 ms; drag max 49.5 / 50.0 ms | **not met**: text 2.4-3.5 x, scale 1.67 x, rotate 1.67 x (move 1.44 x and image 0.69 x are met); drag frame limit about 80 ms. Met: no frame over 300 ms, image worker 0.9 s | `analysis\g5c.txt` |
| G5d | ABeautifulGame `text_warp` in Wrap: 0 frames > 300 ms around the Finals; time to swap <= 1.5 x P | 0 frames; Arc / Flag / Fisheye / Arc: 1,182 / 1,048 / 982 / 1,396 and 1,347 / 1,025 / 949 / 1,047 ms | 1,042 / 994 / 906 / 1,068 and 991 / 1,053 / 912 / 1,086 ms | **met** (0.98-1.19 x, worst pair 1.31 x) | `analysis\g5d.txt` |
| G5e | Core probe: Wrap Full <= 1.5 x projected Full; Wrap Preview <= projected + max(30 ms, 25 %); 20 % decal param on ABeautifulGame <= 150 ms; 100 % decal <= 3 s | Full and Preview: met in every Jump-off row (e.g. ABG board 20 %: Full 2.0 vs 199 ms; ngons 20 %: 27 vs 147 ms). Param, board spot: **150.3 ms** (run 1) / 507.6 ms (run 2, loaded); 100 %: 2.10 s / 7.25 s (loaded). Chess-piece spot: **976 / 900 ms**; 100 %: **3.07 / 3.89 s** | | **not met** for the 20 % param (150.3 ms at the limit on the board; 6 x over on a piece); 100 % met on the board only | `runs_gate2\wa15_run1.txt`, `wa15_run2.txt` |
| G5f | Jump small gaps on ABeautifulGame 20 %: param <= 1.5 x Jump off, jumps >= 1; Overlap 2.5 turns: param + bake <= (turns + 0.5) x Ends meet | Jump, board: **400.7 vs 150.3 ms = 2.67 x** (run 2: 2.47 x), jumps 3; piece: 1.37 x / 1.59 x, jumps 24. Overlap: 20.9 ms vs limit 123 ms (run 2: 201.7 vs 377 ms, loaded), turns 3 | | **Jump: not met** (jumps >= 1 met); **Overlap: met** | same |
| G6 | 13-model sweep, `wrap_surface` with PATH | 13 / 13 ok, 0 panics, 0 ERROR lines (table below) | | **met** (as written); the screenshots show the quality limits below | `analysis\g6.txt` |
| G7 | ABeautifulGame `workflow_path` peak working set: W <= P + 300 MB | working set 1,834 / 2,048 MB; private 3,383 / 3,386 MB | 2,064 / 2,048 MB; private 3,284 / 3,431 MB | **met** (at most +99 MB) | `runs_gate2\runs.csv` |
| G8 | Fuzz seed 7, 1,000 steps, ngons_tiling, Wrap: 0 errors; central / dispatch frames > 500 ms <= 2 | 0 / 0 / 0 errors in both runs; 0 frames over 500 ms (0 over 300 ms); 42 wrap bakes per run | | **met** | `analysis\g8.txt` |

**Where the ABeautifulGame time goes.** The Final lines (`wrap: stamp N param ...`) give the parameterization's share:
text place 55 of 62 ms of worker time, move 56 of 68, scale 133 of 147, rotate 125-129 of 142-147, image 809-843 of
885-927, warp 775-1,163 of 852-1,296 ms. That is 80-90 %. The drag frames of 138 ms are spent in `scene sync`. That is
the throttled Preview bake, which builds a fresh parameterization for each new spot. The Preview lines are debug-level
and not in the log, so the Final lines of the same placements (125-133 ms) are the estimate: about 90 % of the frame.
The parameterization's cost depends on how many triangles lie within the picture's reach, not on how much of the
picture lands. On the chess model a 20 % picture's reach holds hundreds of thousands of triangles, and every one is
welded and sorted. The plan foresaw this case (7.2: "report the param share"). The fix is not small: grow the patch
from the anchor instead of collecting a sphere, or move Preview bakes off the UI thread (plan risk table).

## Model sweep (G6)

`wrap_surface` with each model as PATH, step 10: the `cells` picture at 40 % of the model at the centre of the view, in
Wrap. Jump small gaps off, then on. Ink is the inked surface as a share of the picture's area. Over 1 means parts of the
picture are laid more than once or stretched. Param is the fresh build on the worker; "applied" is the Final swap.

| Model | Triangles | Jump off: verts / tris / cut / covered / ink / param | Jump on: jumps / covered / ink / param | Applied after (off / on) | Peak WS |
|---|---|---|---|---|---|
| WaterBottle | 4,510 | 71 / 151 / 0 / 1.00 / 1.00 / 1.5 ms | 0 / 1.00 / 1.00 / 2.1 ms | 32 / 32 ms | 462 MB |
| Avocado | 682 | 56 / 99 / 0 / 0.56 / 0.52 / 0.6 ms | 1 / 1.00 / 1.94 / 0.6 ms | 32 / 32 ms | 466 MB |
| DamagedHelmet | 15,452 | 275 / 525 / 135 / 1.00 / 1.02 / 5.7 ms | 38 / 1.00 / 5.56 / 20.2 ms | 65 / 131 ms | 498 MB |
| Lantern | 5,392 | 48 / 100 / 47 / 1.00 / 1.18 / 0.3 ms | 0 / 1.00 / 1.18 / 0.4 ms | 48 / 33 ms | 472 MB |
| ABeautifulGame (loaded) | 1,498,080 | 14,210 / 28,416 / 6,488 / 0.07 / 0.06 / 1,201 ms | 50 / 1.00 / 5.24 / 3,056 ms | 1,291 / 4,952 ms | 2,017 MB |
| ngons_tiling (loaded) | 756 | 11 / 9 / 0 / 0.07 / 0.006 / 1.5 ms | 0 / 0.07 / 0.006 / 17.9 ms | 31 / 49 ms | 439 MB |
| torus_knot | 28,800 | 2,526 / 4,914 / 827 / 1.00 / 1.87 / 14.2 ms | 0 / 1.00 / 2.35 / 27.8 ms | 22 / 65 ms | 424 MB |
| mobius | 1,920 | 134 / 264 / 0 / 0.57 / 0.57 / 0.9 ms | 0 / 0.57 / 0.57 / 0.8 ms | 31 / 32 ms | 431 MB |
| klein_bottle | 12,800 | 5,854 / 11,748 / 2,317 / 1.00 / 9.11 / 23.0 ms | 0 / 1.00 / 9.86 / 23.3 ms | 47 / 47 ms | 423 MB |
| nonmanifold | 48 | 16 / 14 / 0 / 0.63 / 0.63 / 0.1 ms | 0 / 0.63 / 0.63 / 0.1 ms | 31 / 31 ms | 414 MB |
| many_islands | 19,200 | 8 / 12 / 6 / 0.02 / 0.01 / 3.4 ms | 2 / 0.04 / 0.03 / 6.0 ms | 31 / 31 ms | 421 MB |
| flat_annulus | 80 | 16 / 19 / 0 / 0.60 / 0.60 / 0.2 ms | 0 / 0.60 / 0.60 / 0.1 ms | 31 / 32 ms | 423 MB |
| high_poly_sphere | 998,000 | 263,285 / 524,744 / 104,527 / 1.00 / **6.63** / 1,243 ms | 3 / 1.00 / **8.05** / 1,878 ms | 1,546 / 2,204 ms | 1,162 MB |

All 13 runs passed (steps 1-9 on the built-in sphere each time, then step 10 on the model). Steps 1-9 gave the same
values in all 13 runs and in gate 1 (ink 0.9854, seam score 0.428, Overlap 3 turns, composite H2 `ccc0244f4c7665fd`).

### The four screenshots the plan names

- **WaterBottle** (`runs_gate2\g6\wrap_surface_glb_samples_WaterBottle.glb_g6\shot.png`): the cells picture goes round
  the bottle body like a printed label. The rows stay straight and the cells keep their size all the way to where the
  body turns away. The white wrapped outline follows the curved top and bottom edges. The flat frame with its handles is
  wider than the bottle (the picture's sides go round the back). No tear, no gap. Panel: "On the model: Project
  [Wrap]", "Jump small gaps" ticked; the "Longer than the way round" buttons are below the visible part of the options
  box (it scrolls).
- **Avocado** (`...\wrap_surface_glb_samples_Avocado.glb_g6\shot.png`, Jump on): the picture is on the cut face and
  over the round seed. On the flat face the cells are regular. Over the seed they bend round it, and the picture shows
  **two horizontal cut lines** and a ragged top edge. The outline breaks into several pieces there. A separate strip
  of cells lies at the left edge (with Jump on, 1 other part was entered).
- **torus_knot** (`...\wrap_surface_shapes_torus_knot.obj_g6\shot.png`, 256 x 256 texture, so the cells are blurry):
  the picture follows the tube over the upper arc and comes back on two short lower stretches of the tube. Its height
  is greater than the tube's girth, so it wraps round the tube. The cells are smeared together and hard to recognise.
  Only fragments of the outline are drawn.
- **flat_annulus** (`...\wrap_surface_shapes_flat_annulus.obj_g6\shot.png`): the centre of the view is the hole, so the
  picture was placed on the ring at the top. It lies flat and undistorted. The part beyond the ring's outer edge is
  missing with a clean edge (covered 0.60), as a sticker over an edge would be.

## What degrades (screenshots)

| Case | Screenshot | What it shows |
|---|---|---|
| Bottle (works) | `runs_gate2\g6\wrap_surface_glb_samples_WaterBottle.glb_g6\shot.png` | A clean label round the body. Exact on this nearly cylindrical part (ink 1.00, 0 cuts) |
| Bump | `runs_gate2\g6\wrap_surface_glb_samples_Avocado.glb_g6\shot.png` | Cut lines where the picture goes over the seed |
| Strong bumps everywhere (fails) | `runs_gate2\g6\wrap_surface_shapes_high_poly_sphere.obj_g6\shot.png` | The bumpy ball is covered all over in scattered colour scraps. The picture's cells cannot be recognised, the outline is a tangle of straight lines, and ink is 6.6 x the picture with 104,527 cut triangles. Same cause as WA4 at 45-60 degrees: the unfolding fronts collide on steep bumps. |
| Self-crossing shape | `runs_gate2\g6\wrap_surface_shapes_klein_bottle.obj_g6\shot.png` | Cells repeated in torn patches all over the bottle (ink 9.1 x). The plan expected only cut lines |
| Hard edges, many parts (Jump on) | `runs_gate2\g6\wrap_surface_glb_samples_DamagedHelmet.glb_g6\shot.png` | With Jump small gaps on (38 jumps) the whole front of the helmet is covered in polygon-shaped fragments of the picture. With it off (not in a screenshot) the stats are fine: ink 1.02, covered 1.0, 135 cuts along hard edges |
| Hole / edge | `runs_gate2\g6\wrap_surface_shapes_flat_annulus.obj_g6\shot.png` | The part of the picture with no surface under it is missing; the rest stays in place |
| Separate parts (Jump on) | `runs_gate2\g6\wrap_surface_glb_samples_ABeautifulGame.glb_g6\shot.png` | The picture continues over half the board and onto every piece standing there (50 jumps). Each piece is painted all round. With Jump off it stays on the one piece that was clicked (6.5 % of the picture) |
| A corner | (no app screenshot) | Core WA5: a cube corner gives one clean cut line (1 cut, smear 0) |

## Not met, and why

1. **G5c (ABeautifulGame Finals and drags)** and **G5e (the 20 % parameterization)**. The parameterization is
   rebuilt for every new spot, size bucket or text frame. Its cost grows with the triangles inside the picture's reach
   sphere (weld, edge groups, sort), not with the picture's coverage. Text 2.4-3.5 x, scale and rotate 1.67 x; drag
   frames about 140 ms; 20 % parameterization 150 ms on the board and 900-980 ms on a piece. The plan's fallback is to
   report this, and no small fix exists.
2. **G5f, Jump small gaps** costs 2.5-2.7 x Jump off at the board spot. That spot's front enters 3 more parts, and
   every vertex near another part queries for a gap.
3. **G5a, run 1**: a single 84-ms first bake (a 49.7-ms first parameterization in a fresh process). Run 2 and every
   other bake are within the line. Gate 1 saw the same one-off (202 ms). It is probably the exe's code pages being
   read from the USB disk on first use (not proven).
4. **WA4 at 45-60 degrees** (package A, not changed since): ink 4.6-5.9 x on steep bumps. In the app this is the
   high_poly_sphere and Klein bottle results above. Fixing it needs a relaxation step or another front bound (check:A).
5. **G2 whole-screenshot identity**: only the side panel differs, because of the new rows (the views and files are
   identical). This is by design, but it needs the user's acceptance.
6. **G6's quality**: the line as written (all ok, no panics) is met. But on 5 of 13 models the picture is laid more
   than once or scattered (ink over 1.5 x: high_poly_sphere, klein_bottle, torus_knot, plus DamagedHelmet and
   ABeautifulGame with Jump on).

## Not delivered

Nothing of section 10 was cut. **Both Wrap options, Jump small gaps and Longer than the way round (Ends meet |
Overlap), are delivered**: controls in the Stamp and Text tool options and on the selected stamp, saved, undoable,
greyed in Project (WA16-WA19, `wrap_surface` steps 4b-4d and 9). Text-on-path handles on the surface, the wrapped
outline in 3D and 2D, parameterization reuse, the boundary snap and the spine's jump are all in.

## Gate changes (stage 2, `work\merged`)

1. `wrap_tests.rs::wrap_bench_models` (ignored probe, test only): it now also measures an "off-centre" spot, so
   ngons_tiling (whose centre rays miss) is measured, and it prints each spot. Package A's board spot is unchanged.
2. `selftest\scenarios.rs` (`text_warp`'s warp check, selftest only): for a wrapped text the check "the baked bounds
   changed" now checks that the baked pixels changed. A wrapped text on ABeautifulGame touches islands all over the
   2048 x 2048 set, so its bounds stay the whole set. `text_path` has the same bounds check; it was not run in Wrap and
   is not changed.
3. Gate 1's change stays: WA19's wall-clock assert retries up to 4 times (same limit).

## Behaviour changes in round 6

1. New switch **On the model: Project | Wrap** for every 3D stamp and 3D text (tool options for new ones, selected
   stamp for existing ones). New stamps start in Project. The tool keeps the chosen mode for the session.
2. **Jump small gaps** (off by default; fixed 1 % of the model size) and **Longer than the way round: Ends meet |
   Overlap** (Ends meet by default), per stamp, greyed in Project.
3. In Wrap the projection rows (depth, fit depth, angle limits, back-face cull, occlusion) are hidden. They are kept
   for switching back. "Project through" is greyed out. A stats line reads "Wrapped over N triangles", with warnings
   (runs off the surface, ends meet, wound N times, continued onto N parts, too large).
4. A selected wrapped stamp shows its outline on the surface (3D and 2D) and a faint flat frame with the handles.
   While a drag is in progress the stats line says "Wrapping..." and the outline waits for the Preview bake.
5. Project format 3. A file with a wrapped stamp, or with a Wrap option set, needs round 6. Other files open in
   round 5 with the "newer version" warning.
6. The side panel's stamp options are taller (new rows), so for example the Size field is lower down.
7. New measuring switch `WTE_STAMP_MODE=wrap`; new log lines `wrap: stamp N param ...`.
8. On a heavy model a wrapped stamp's move, scale or turn can freeze the view for 0.1-1 s per Preview, because the
   parameterization is built on the UI thread (ABeautifulGame: about 140 ms on a text, up to 1 s for a large picture
   on a piece).

## What to try by hand

1. **A text dragged round WaterBottle**: place a text in Wrap on the body, drag it round and up to the shoulder. It
   should stay a straight line on the label part and bend over the shoulder.
2. **1.5 x round with Ends meet, then Overlap**: scale a picture wider than the bottle's girth. With Ends meet the ends
   meet at the back. Switch to Overlap: the end of the picture should lie on top.
3. **Project -> Wrap -> Project on an old stamp** (open a round-5 project): the last state should look exactly like
   the first.
4. **Avocado's bumps, DamagedHelmet's edges**: move a picture over the seed and over the visor edge. Expect cut lines.
   Judge whether they are acceptable.
5. **A line across the chess pieces with Jump small gaps off, then on**: off stops at the piece's edge; on continues
   onto the neighbours and the board. Note how long each move takes on this model.
6. **Save, reopen, and open in round 5**: the reopened project should look the same. Round 5 should refuse the file
   with a "newer version" message.
7. **A large wrapped text on ABeautifulGame**: drag and scale it. Expect stutters of about 0.1-1 s (see "Not met").
   Say whether that is acceptable or whether moving Preview bakes off the UI thread should be the next round.
8. **A strongly bumpy model** (e.g. `high_poly_sphere.obj`): see how badly the picture breaks up. Decide whether
   Wrap should warn or refuse there.
