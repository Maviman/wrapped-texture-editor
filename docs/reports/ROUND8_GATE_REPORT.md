# Round 8 gate report — Wrapped Texture Editor ("wrap: once, whole, smooth — and fast")

Date 2026-11-22. Plan: `target\round8_design\ROUND8_PLAN.md` (section 12: the user's answers, all defaults). Notes:
`notes\pkg_a.md`, `notes\pkg_b.md` (builders and checks), `notes\gate1.md` (merge, tests, scenarios, identity, sweep,
compatibility), `notes\gate2.md` (measurements). Raw runs: `runs_gate\` (stage 1) and `runs_gate2\` (stage 2, analyses in
`runs_gate2\analysis\`).

## Verdict

**Round 8 does not pass its gate.** The picture now lies much better on round and bumpy things, Project mode and the
brush are unchanged, and old files still open the same. But four core acceptance tests fail, five of the thirteen
sweep models miss the quality limits, Jump small gaps still repeats picture pieces across separate parts, and wrapped
pictures on heavy models are slower to finish than in round 6.

- **Better than round 6.** On the bumpy ball (`high_poly_sphere`) the picture is now one whole piece: every cell once,
  rows continuous, each cell bent over its bumps (round 6: scraps all over the ball, ink 6.6-8.1 x). The Klein bottle
  shows one piece on its visible side (round 6: torn repeats, ink 9.1 x). The water bottle label is still exact. The
  ink ratio dropped on every curved model that failed in round 6. The panel now says "Fitted to the curved surface:
  stretched up to P %" and "Cut where it cannot lie flat ...".
- **Smoother drags.** Wrapped Previews are made on a worker: on the chess set a drag of a wrapped text now has frames of
  at most 23 ms (Project: up to 66 ms; round 6 Wrap: 142 ms). While dragging, the last finished picture stays and
  catches up after (user answer Q4).
- **Unchanged.** Project pixels, every brush scenario and the exported files are identical to round 6+7
  (`wte_round6_7.exe`). Round-6 projects open with identical pixels. No format change.
- **Not met: quality.** WA4 at 60 degrees (ink 1.136, a cell 1.6 x), the WA20 bumpy ball (ink 1.64, 10.7 % of the picture
  twice), WA22 (picture larger than a torus: 2.3 % twice) and WA24 (Jump onto a cube: ink 1.36, 14.8 % twice) fail
  their limits. In the sweep, Avocado (Jump on), Lantern, ABeautifulGame (Jump on), klein_bottle and nonmanifold fail
  (ink up to 1.53, picture twice on up to 84 %). On the chess set with Jump on, the pieces in and next to the picture
  carry blobs of picture colours; on DamagedHelmet the visor's left has a fold and the side parts get scraps.
- **Not met: speed on heavy models.** A wrapped Final on the chess set takes 0.3-3.1 s (Project 0.04-1.7 s: 1.7-10.8 x,
  limit 1.5 x); a warped text 2.0-2.8 s (Project about 0.9 s). The core's 20 % picture needs 339-386 ms on the board
  and about 1.06 s on a piece (limit 150 ms); Jump small gaps 1.6-6.8 x more. The bumpy ball's Final takes 11.4-11.6 s
  (limit 6 s). On the built-in sphere a moved stamp's Final takes 47-64 ms against Project's 30 ms (limit 1.5 x). The
  cause: package A did not build "growth from the anchor" (step 6), so 63-90 % of the time on the chess set goes to
  collecting every triangle within reach; on bumpy spheres 74-78 % goes to the relaxation solve.
- **Gate changes.** None to product code. The core probe was extended and timed with a test-only phase clock for two
  runs, then reverted (`work\merged` equals `work\pkg_b` again).

**Measuring conditions.** No build or other app ran during any measurement (driver log: "running before: []" for all 32
runs; rustc only during my two probe builds, between measurements). The user's bambu-studio, Discord and Claude ran
lightly in the background. Free memory was 0.9-2.2 GB, but during the chess-set `workflow_path` runs (G5c, both modes) it
fell to 8-400 MB and Windows trimmed the app's working set, so those times are from a machine under memory pressure.

## Exe and tests

- Exe `target\round8\wte_round8.exe`, **SHA-256 19D7844B4BDCD73ECE7327FB3F677CD58695A8314EA80F42CB194CDA97E54B2A** (gate 1's
  release build of `work\merged`, = check:B's exe). Every scenario and measurement of both stages used it.
- Tests (gate 1, `runs_gate\build_logs\test1.log`; no source change since): **1,147 passed, 4 failed, 33 ignored**
  (wte_app 357 / 16, wte_core lib 743 passed, 4 failed / 17, brush_tips 15, end_to_end 9, project_roundtrip 23). Base
  (round 6+7) 1,116 / 33. The 4 failures are package A's acceptance limits (below), asserted unchanged. `cargo clippy
  --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: ok. No `todo!` / `unimplemented!`.
- Scenarios (gate 1): 39 / 39 ok at 1280 x 800. 35 shots identical to `wte_round6_7.exe` (every brush scenario, `stamp`,
  `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run`); the rest only round 7's known exceptions (toast
  area, export dialog folder text) and `wrap_surface` (expected). Exports byte-identical, `check_exports.py` PASS. Project
  composite H0 `cd272dc06ef4e935` in both exes. `WTE_SYNC_FINAL_BAKES=1` gives the same composites.

## Acceptance lines

"P" = Project, "W" = Wrap (`WTE_STAMP_MODE=wrap`), same exe, same clicks. Two values = run 1 / run 2.

### Core quality (package A, plan 3.5; `wte_core::wrap_tests`)

Numbers from `notes\pkg_a.md` (builder and check:A tables) and gate 1's run (`runs_gate\build_logs\test1.log`).

| # | Test | Limit | Measured | Verdict |
|---|---|---|---|---|
| WA1-WA3, WA5, WA16, WA18 | Exact cases (cylinder, 1.5 x round, cube edge / corner, coplanar blocks, Overlap) | round-6 limits: 99.9 % within 2/255, max 8 | 100 % within 2/255, max 0-1 in every case; WA2 ink 1.0010, cells 0.970-1.023 | **met** |
| WA4 30 deg | Bumpy plate, 2 periods | ink 0.90-1.10; cells 0.75-1.30; smear <= 1 %; covered >= 0.999; dup <= 1 %; scatter <= 2 %; seams 0 | ink 1.028; cells 0.957-1.117; dup 0; scatter 0; seams 0 | **met** |
| WA4 45 deg | same | same | ink 1.074; cells 0.906-1.283; dup 0; seams 0 | **met** |
| WA4 60 deg | same | same | **ink 1.136; cells 0.785-1.599**; smear 0; covered 1.0; dup 0; scatter 0; seams 0 (round 6: ink 4.62, dup 89 %) | **not met** (test fails) |
| WA4b | 4 periods, 60 deg | ink 0.85-1.15; cells 0.65-1.50 | ink 1.123; cells 0.788-1.439; dup 0 | **met** |
| WA20 | Bumpy ball, 40 % | ink 0.85-1.15; cells 0.70-1.40; dup <= 1 %; scatter <= 2 %; seams 0; covered >= 0.99 | **ink 1.637; cells 0.754-3.302; dup 10.7 %; scatter 4.08 %**; seams 0; covered 1.0; 45 flipped (round 6: ink 3.53, dup 84.6 %) | **not met** (test fails) |
| WA21 | Closed lathe piece, picture 2 x its height | inked share >= 0.95; dup <= 1 %; seams S1 / S2 only; flipped <= 1 % | share 1.001; dup 0; S1 130 only; flipped 0 | **met** |
| WA22 | Torus: taller than the girth / larger than the torus | dup <= 1 %; scatter <= 3 % | taller: dup 0.013 %, scatter 0.006 %; **larger: dup 2.27 %**, scatter 0 | **not met** (larger case; test fails) |
| WA23 | Cone tip; cube corner | one S4 cut; smear <= 0.5 %; covered >= 0.70 | cone: one closure cut, smear 0, covered 0.461 (the cone has 161 deg of surface round its tip: at most 0.447 is possible; the test asserts 0.95 x that bound); cube: S4 1, smear 0, covered 1.0 | **met** except the plan's 0.70 on the cone, which the geometry rules out |
| WA24 | Jump bounded: WA17 cube on floor; 5 x 5 cubes | WA17 ink 0.95-1.10, dup <= 1 %; grid ink <= 1.15, dup <= 2 % | **WA17 ink 1.358, dup 14.8 %**; grid not reached (the test stops at the first assert) | **not met** (test fails) |
| WA25 | Owner test inert where injective (WA1 x2, WA3, WA5 edge, WA16, WA18) | 0 rejected; relaxed false | all 8: relaxed false, rejected 0 (after check:A's fix) | **met** |
| WA26 | Determinism WA20 / WA22 / WA24 at 1 / 2 / 5 threads, Preview and Full | byte-identical | equal; parallel kernels equal on 1 and 12 threads | **met** |
| WA27 | Reuse after relaxation (scale, tint, flip) | reused = fresh | equal | **met** |
| WA28 | Growth from the anchor | <= 1.25 x (laid + halo) | not built | **not done** |
| WA29 | Preview / param-only jobs | as stated | all as stated | **met** |
| WA30 | Old tests pass | as stated | every round-6 / round-7 test passes unchanged | **met** |
| WA15 | Probe | reported; G5e / G5f | run in this gate (below); the welded-graph build time does not exist | partly (step 6 not built) |
| 4.2 | Package B's app tests (worker Previews, races, panel lines) | pass | 357 app tests pass (15 new: 9 by build:B, 6 by check:B) | **met** |

### Unchanged and compatible (gate 1, `notes\gate1.md`)

| Line | Measured | Verdict |
|---|---|---|
| Project and brush identity | 35 of 39 shots identical to `wte_round6_7.exe`, the rest the known exceptions; exports byte-identical; H0 equal | **met** |
| Round-6 projects open in round 8 with identical pixels (sphere sample, WaterBottle) | 0 px differ | **met** |
| After a tint change the WaterBottle stamp within 2/255 on >= 99.9 % | **71.6 %** (logo); its top lies on the curved shoulder, which round 8 relaxes (stretch 10.3 %), so rows shift 1-3 texels there | **not met** |
| Sphere stamp after a tint change differs (expected) | 91.2 % within 2/255; the same picture in the same place, cell borders shifted by about one texel | as expected |

### Speed, memory, robustness (this stage, `notes\gate2.md`)

| # | Line | Wrap | Project | Verdict | From |
|---|---|---|---|---|---|
| G5a | Sphere sample `stamp`: W Final applied <= 1.5 x P and <= 80 ms (run 2); Preview landing <= 50 ms | Run 2: 47 / 50 / 64 / 64 / 28 ms. Run 1: 160 (first parameterization in a fresh process, 96 ms) / 46 / 63 / 82 / 31 ms. Preview landing (job start to applied) 32-50 ms in run 2, 31-51 ms in run 1. Drag frames <= 19.2 ms | 29-34 ms | **not met**: 4 of 5 run-2 Finals are 1.5-2.1 x P (<= 80 ms met); landing met in run 2, one 51 ms in run 1 | `analysis\g5a.txt` |
| G5b | ngons_tiling `workflow_path`: applied <= 1.5 x P; 0 frames > 300 ms; drag <= P + max(30 ms, 25 %) | image 31 / 139, text 83 / 117, move 148 / 114, scale 145 / 147, rotate 98 / 131 ms; max frame during bakes 45.8 ms; drag max 21.6 / 29.0 ms | 313 / 315, 150 / 150, 182 / 171, 254 / 247, 365 / 364 ms; drag max 184.2 / 184.4 ms | **met** | `analysis\g5b.txt` |
| G5c | ABeautifulGame `workflow_path`: drag frame <= P + max(30 ms, 25 %); Finals <= 1.5 x P; 0 frames > 300 ms; image worker <= 2.0 s | drag max 22.6 / 19.6 ms; image 3,073 / 2,503 (worker 2,960 / 2,468), text 317 / 304, move 616 / 615, scale 1,014 / 981, rotate 1,086 / 1,013 ms; max frame during bakes 42.6 ms; Preview jobs landed 281-615 ms after their start, all after the release (dropped) | drag max 65.6 / 52.4 ms; image 1,655 / 1,453, text 35 / 38, move 67 / 68, scale 132 / 112, rotate 118 / 94 ms | **drag met; 0 long frames met; Finals not met** (1.7-10.8 x); **image worker not met** (2.5-3.0 s) | `analysis\g5c.txt` |
| G5d | ABeautifulGame `text_warp` in Wrap: 0 frames > 300 ms; swap <= 1.5 x P | 0 frames > 300 ms (max 134.6 ms); swap 2,498 / 2,501 / 2,015 / 2,498 and 2,458 / 2,214 / 2,115 / 2,775 ms | 946 / 947 / 817 / 951 and 1,044 / 932 / 780 / 932 ms | **frames met; swap not met** (2.4-2.9 x; round 6: 1.0-1.3 x) | `analysis\g5d.txt`, `g5d_swap.txt` |
| G5e | Probe: 20 % param <= 150 ms at the board and the piece; welded-graph build <= 1.0 s; 100 % <= 3 s; Wrap Full <= 1.5 x projected Full | 20 %: board 385.9 / 339.0 ms, piece 1,054.6 / 1,070.7 ms; welded graph not built; 100 %: board 3,301.6 / 2,910.9 ms, piece 2,292.0 / 2,392.5 ms; Full: e.g. board 20 % 2.2 vs 253.8 ms, piece 20 % 34.6 vs 755.6 ms | | **20 % not met** (2.3-7.1 x the limit); 100 % met on the piece, board only in run 2; **Full met** in every row | `wa15_run1.txt`, `wa15_run2.txt`, `analysis\wa15.txt` |
| G5f | Jump on <= max(1.5 x Jump off, 150 ms) at both spots; Overlap 2.5 turns as round 6 | board 2,432.3 / 2,308.1 vs limit 578.9 / 508.5 ms (jumps 3; 14,345 of 49,746 triangles flipped); piece 1,690.3 / 2,102.4 vs 1,581.9 / 1,606.1 ms (jumps 6); Overlap 19.5 / 18.5 vs 36.8 / 37.1 ms, turns 3 | | **Jump not met** (board 6.3-6.8 x, piece 1.6-2.0 x); **Overlap met** | same |
| G6 | Sweep times; high_poly_sphere Final <= 6 s | high_poly_sphere 11,360 / 11,561 ms (Jump off / on; gate 1: 11,175 / 11,546); ABeautifulGame 3,486 / 13,147; klein_bottle 229; torus_knot 100 / 116; DamagedHelmet 80 / 85; the rest 15-66 ms | | **not met** (high_poly_sphere 1.9 x the limit) | `analysis\g6_table.md`, gate 1 |
| G7 | ABeautifulGame `workflow_path` peak working set W <= P + 300 MB | 1,464 / 1,950 MB (private 3,411 / 3,402 MB) | 1,995 / 1,916 MB (private 3,312 / 3,421 MB) | **met** (at most +99 MB private; working sets trimmed under memory pressure) | `runs_gate2\runs.csv` |
| G8 | Fuzz seed 7, 1,000 steps, ngons_tiling, Wrap: 0 errors; long frames <= 2 | both ok (190 s); 0 errors; 0 central / dispatch / pointer frames over 500 ms (0 over 300 ms); 42 wrap Finals and 38-39 Preview jobs per run | | **met** | `analysis\g8.txt` |

**Where the time goes** (`wrap:` lines; inside the parameterization from a test-only phase clock, probe run 2):

| Case | Total param | collect + weld | front | seams + wedges | LSCM + ARAP solve | owner grid (+ stretch stats) | other |
|---|---|---|---|---|---|---|---|
| ABG board, 20 % (flat: seed kept) | 339 ms | 307 (90 %) | 5 | - | - | 13 | 14 |
| ABG piece, 20 % (3,698 verts, relaxed) | 1,071 ms | 677 (63 %) | 43 | 32 | 266 (25 %) | 17 | 36 |
| ABG board, 20 %, Jump on (24,817 verts) | 2,308 ms | 872 (38 %) | 111 | 102 | 1,080 (47 %) | 42 | 101 |
| high_poly_sphere, 40 % (118k verts) | 8,774 ms | 1,019 (12 %) | 244 | 332 | 6,850 (78 %) | 117 | 212 |
| bumpy ball (WA20), 40 % | 199 ms | 8 | 3 | 6 | 147 (74 %) | 31 (15 %) | 4 |
| sphere sample, 20 % | 15 ms | 0.5 | 0.1 | 0.2 | 0.5 | 13.7 (90 %) | 0.3 |

In the app: the wrapped Finals on the chess set are 92-96 % parameterization. A Final released while the drag's Preview
job still runs waits for that job (plan 4.1, binding): 238-534 ms of the move / scale / rotate times on the chess set and
up to ~50 ms on the sphere sample. The job's parameterization is reused only when it was made for the released
placement (sphere sample: yes; chess set: no, the drags are shorter than one job).

## Model sweep (G6 quality, gate 1; this stage's rerun gives identical numbers and identical shots)

`wrap_surface` with each model as PATH, step 10: the `cells` picture at 40 % of the model in Wrap, Jump small gaps off
then on. Ink = inked surface / picture area; dup = share of the picture found in two places; scatter = scraps.
Limits: **ink <= 1.3, dup <= 3 %, scatter <= 5 %** (both Jump settings).

| Model | Jump | verts / tris | relaxed | seams | stretch | covered | ink (round 6) | dup | scatter | jumps | param / applied ms | Limits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| WaterBottle | off / on | 225 / 482 | yes | 0 | 2.1 % | 1.000 | 1.0017 (0.999 / 0.999) | 0 | 0 | 0 | 20 / 49, 21 / 36 | met |
| Avocado | off | 79 / 136 | yes | 0 | 12.7 % | 0.532 | 0.529 (0.518) | 0 | 0 | 0 | 9 / 48 | met |
| Avocado | on | 115 / 235 | yes | 0 | 12.6 % | 1.000 | **1.530** (1.938) | **52.6 %** | **5.87 %** | 1 | 29 / 60 | **not met** |
| DamagedHelmet | off | 233 / 457 | yes | 16 | 16.9 % | 0.655 | 0.507 (1.019) | 0 | 0 | 0 | 25 / 80 | met |
| DamagedHelmet | on | 1,060 / 1,956 | yes | 140 | 43.6 % | 1.000 | 1.094 (5.563) | 0 | 0 | 9 | 56 / 85 | met (numbers; see the shot) |
| Lantern | off / on | 52 / 100 | yes | 28 | 220 % | 1.000 | 0.827 (1.183) | **26.9 %** | **10.6 %** | 0 | 14 / 66, 14 / 24 | **not met** |
| ABeautifulGame | off | 14,210 / 28,416 | yes | 213 | 130 % | 0.053 | 0.059 (0.064) | 0 (0 bins) | 0 | 0 | 3,361 / 3,486 | met (vacuously: 5 % of the picture has surface) |
| ABeautifulGame | on | 160,593 / 323,172 | yes | 2,235 | 97.2 % | 1.000 | **1.3045** (5.236) | **10.5 %** | 1.13 % | 30 | 12,144 / 13,147 | **not met** |
| ngons_tiling | off / on | 11 / 9 | no | 0 | 0 | 0.074 | 0.006 (0.006) | 0 | 0 | 0 | 3 / 31 | met |
| torus_knot | off / on | 1,272 / 2,599 | yes | 84 | 18.3 % | 0.404 | 0.405 (1.868 / 2.355) | 0 | 0 | 0 | 78 / 100, 87 / 116 | met (numbers; see the shot) |
| mobius | off / on | 232 / 434 | yes | 0 | 1.1 % | 0.571 | 0.571 (0.573) | 0 | 0 | 0 | 13 / 31, 17 / 50 | met |
| klein_bottle | off / on | 2,634 / 5,621 | yes | 0 | 46.8 % | 1.000 | 1.105 (9.109 / 9.862) | **4.13 %** | 0.05 % | 0 | 209 / 229 | **not met** |
| nonmanifold | off / on | 16 / 14 | no | 0 | 0 | 0.630 | 0.629 (0.629) | **84.2 %** | 1.67 % | 0 | 7 / 30 | **not met** |
| many_islands | off / on | 8-24 / 12-36 | yes | 8 / 21 | 100 / 41 % | 0.018 / 0.034 | 0.015 / 0.035 (same) | 0 | 0 | 0 / 2 | 8-11 / 33 | met |
| flat_annulus | off / on | 22 / 27 | no | 0 | 0 | 0.600 | 0.597 (0.597) | 0 | 0 | 0 | 8 / 31 | met |
| high_poly_sphere | off / on | 122,496 / 246,559 | yes | 176 | 34.9 % | 1.000 | 1.043 (6.626 / 8.051) | 0 | 0.10 % | 0 | 11,279 / 11,360, 11,471 / 11,561 | met |

8 of 13 models meet the limits with both settings; 5 do not.

### Screenshots, round 6 next to round 8 (the shot shows the state after Jump on)

Round 6: `target\round6\runs_gate2\g6\wrap_surface_<folder>_<model>_g6\shot.png`. Round 8: `target\round8\runs_gate\sweep\
wrap_surface_<folder>_<model>_r8\shot.png` (identical to `target\round8\runs_gate2\g6\wrap_surface_<folder>_<model>_g6\
shot.png`). Enlarged crops: `target\round8\runs_gate\crops\*_zoom.png`. Readings from gate 1, checked again here on
the high_poly_sphere and ABeautifulGame crops.

| Model | Round 6 | Round 8 |
|---|---|---|
| high_poly_sphere | Torn colour scraps over the whole ball, the picture many times, the outline a zig-zag | **One whole picture**: the 8 x 4 cells each once, rows continuous, each cell bent over its bumps (wavy edges), wavy outline, no visible cut or scrap |
| klein_bottle | Picture pieces repeated all over the outer tube and the ring | One piece on the visible side: the left columns on the outer tube, the right ones curving into the ring's hole, stretched; two jogs in the outline. The 4.1 % repeat is not visible from this view, so it presumably lies on the hidden side |
| Avocado (Jump on) | Cells torn into strips sticking out to the left, repeats | The visible picture is one piece over the cut face and the seed's dome, rows continuous across the rim. Fails numerically: the flesh hidden under the seed keeps its own copy of the seed's share |
| DamagedHelmet (Jump on) | Shredded colour triangles over the whole helmet | Large continuous cells on the visor, but **not one whole picture**: a fold on the visor's left (a thin lilac wedge, a dark-red cell twice), and the 9 jumps put scraps on the side parts. The numbers pass; the metric misses these |
| ABeautifulGame (Jump on) | Picture smeared over the board and every piece in reach | Continuous rows on the board, but pawns, a bishop and piece bases in and beside the footprint carry blobs of picture colours: **not one whole picture** |
| torus_knot | Smeared colour along a long stretch | The picture on one short tube section, squeezed round the tube into a band of the left columns' colours; "60 % of the picture runs off the surface"; not recognisable as the cells, but nothing repeated |
| WaterBottle | One whole picture on the body | The same: one whole 8 x 4 picture, rows straight, columns narrowing evenly to the sides, no cut, no repeat |
| Lantern | Similar patches | The front faces continuous across the box edge, extra patches on the sloped plinth and the top ledge (19 cone cuts on a 52-vertex block, stretch 220 %) |
| nonmanifold | Same | The "book" pages carry the same cells on several pages; the model's UVs also overlap (52 overlapping pairs) |

## What was not met, and why

1. **Four core tests** (WA4 60 deg, WA20, WA22 larger than the torus, WA24). WA4 and WA20: with the picture's centre
   line pinned exactly to the surface (user answer Q2), a 60-degree bump's curvature must go into stretch at the
   picture's top and bottom; LSCM + ARAP spread it, but not within the limits, and on the ball the relaxation folds (45
   flipped triangles). check:A tried more iterations and other starts: none met both WA4 and WA20 without breaking WA21.
   WA24: the owner test cannot give a standing part (a cube, a chess piece, Avocado's seed) its share of the picture,
   because the floor under it ties with it in surface distance; a visibility or tie-break rule is a planner decision.
   WA22: not investigated further. The asserts keep the plan's limits.
2. **Sweep quality on 5 models** (Avocado Jump on, Lantern, ABeautifulGame Jump on, klein_bottle, nonmanifold), and the
   screenshots of DamagedHelmet, ABeautifulGame and torus_knot are not "one whole picture". Same causes as 1 (owner
   test with jumped parts; cone cuts and stretch on a coarse box; the non-manifold "book" keeps the round-6 seed).
3. **Speed: G5a, G5c Finals and image worker, G5d swap, G5e 20 %, G5f Jump, G6.** Package A did not build step 6
   (welded graph, growth from the anchor, warm-up job): the parameterization still collects and welds every triangle
   within a reach sphere, now 1.7 x the picture's half height across (plan 1.3; package A's deviation 1, needed for
   WA4). On the chess set that is 63-90 % of the time. On curved parts the new relaxation adds 25-78 %. A Final waits
   for the drag's Preview job (plan 4.1). No small, contained fix exists, so nothing was changed in the gate.
4. **Compatibility tint check** on WaterBottle: 71.6 % within 2/255 (limit 99.9 %). The plan expected the stamp to lie on
   a cylinder part; it reaches the shoulder, which round 8 relaxes.
5. **Not built**: package A step 6 (WA28, the welded-graph time of G5e) and package B's warm-up job; Overlap stacks of
   several turns on curved bands keep round 6's seed (check:A open item 4).

## Behaviour changes in round 8

1. On curved and bumpy surfaces a wrapped picture is relaxed: stretched smoothly instead of torn; cut lines only at
   sharp corners or tips (> 45 degrees), where the picture's edges meet, or round a handle. The panel shows "Fitted to the
   curved surface: stretched up to P %" (from 5 %) and "Cut where it cannot lie flat (a corner, a handle, or where its
   edges meet)".
2. Jump small gaps continues only onto parts under the picture's own footprint.
3. Wrapped Previews are made on a worker (one at a time, latest wins). During a drag the last finished picture stays and
   catches up afterwards. A Final waits for its stamp's Preview job in flight. Project Previews stay on the UI thread,
   unchanged. `WTE_SYNC_FINAL_BAKES=1` now also keeps wrapped Previews synchronous.
4. After opening a project or an undo, a selected wrapped stamp gets its outline and stats from a background job
   ("Wrapping..." meanwhile).
5. Round-6 wrapped stamps keep their saved pixels until edited; at the first edit on a curved part they change
   slightly (WaterBottle logo near the shoulder: rows move 1-3 texels; the sphere stamp's cell borders about 1 texel).
6. Wrapped Finals on heavy models take longer than in round 6 (chess set: text move 0.6 s vs 0.09 s; warped text 2.0-2.8
   s vs 1.0-1.4 s; bumpy ball 11.4 s vs 1.5-2.2 s), while drags are smoother (23 ms vs 142 ms frames).
7. Log: `wrap:` lines add `relaxed`, `seams`, `stretch`, `rejected`, `cg_iters`; new lines `wrap preview: stamp N ...` and
   `wrap fit: stamp N ...`. No file-format change.

## What to try by hand

1. **A picture over Avocado's seed** (Wrap, Jump small gaps on): it should lie over the seed's dome in one piece. Look
   whether the cut face under the seed shows the seed's part of the picture again when the seed is hidden or rotated
   away.
2. **A text across high_poly_sphere**: the letters should follow the bumps without breaks; note how long the release
   takes (expect 2-11 s depending on the size; the drag itself stays smooth).
3. **Jump small gaps on across chess pieces** (ABeautifulGame): expect picture colours on pawns and piece bases next to
   the picture. Say whether pieces should take the picture at all.
4. **A box corner** (any cube or the Lantern's base): one cut line from the corner; on the Lantern's sloped plinth
   expect extra patches.
5. **A drag on ABeautifulGame** (wrapped text): frames stay smooth, the picture follows after the release; the release
   takes 0.3-1.1 s. Say whether that delay is acceptable.
6. **Reopen a round-6 project and nudge a stamp** (e.g. a WaterBottle label): unchanged until the nudge, then re-wrapped;
   near the shoulder the rows move by a few texels.
