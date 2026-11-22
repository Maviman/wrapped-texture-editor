# Round 9, check:A — notes

Checker: check:A (Opus), 2026-11-22. Worktree `G:\software_projects\texture_wrapper_ap_r9a`, branch `round9-a`. Logs under
`target\round9\check_a\` (git-ignored). Written as the work goes; the last section is the final state.

## 1. Verification of build:A's claims

| Claim (pkg_a.md) | Checked | Result |
|---|---|---|
| 1,153 passed / 5 failed / 33 ignored, clippy 0 | full `test --workspace --no-fail-fast` at `5acadac` | **confirmed** (357 + 749 + 15 + 9 + 23 passed; 16 + 17 ignored) |
| `wa4_r8` 60 deg: ink 1.1177, cells 0.769..1.432 | same run | confirmed |
| `wa20`: ink 1.189, cells 0.665..2.886, dup 0.976 %, scatter 0.564 %, seams 0, flipped 15 | same run | confirmed |
| `wa17` / `wa24`: ink 0.915 / 0.9146 vs >= 0.95 | same run | confirmed (cube 13,561 texels, floor 24,066, rejected 0.140, dup 0) |
| `wa37`: lost_cut 5.67 % (WA22 taller), 2.48 % (WA32 64 rows) | same run | confirmed |
| Determinism (1 vs N threads) | `RAYON_NUM_THREADS=1` vs default, `wrap_tests` with `--nocapture --test-threads=1`, every printed number compared (times masked) | **identical**. One extra finding: with one rayon thread `wa7` fails its wall-clock limit (308 ms, overlapping_cubes x1, Jump + Overlap; limit 200 ms debug); default threads pass |

## 2. WA17 / WA24: `ink >= 0.95` (decision for the user)

**build:A's claim is right.** Independent check of the geometry (`samples::uv_cube_on_floor`, the test's projector):
- `wrapped(position (0, 0, 1.2), normal +Y, up -X, half (1.0, 0.25))`: the picture's long axis is `up x normal` = +-Z,
  half length 1.0, so it runs from z = 0.2 to z = 2.2; its height runs over x in -0.25..0.25.
- The floor is the quad x, z in -2..2 at y = 0 (one sheet, no underside, no other surface beyond z = 2): the 0.2 of the
  picture's 2.0 length beyond z = 2.0 has no surface under it (`reached_edge: true`): 10 % of its area.
- The cube is x, z in -0.5..0.5, y in 0..1: from z = 2.0 to z = 0.5 the picture lies on the floor (1.5 of its length),
  then it climbs the cube's +Z side for the remaining 0.3 (the cube is 1 high). 1.5 + 0.3 = 1.8 of 2.0 = **0.90** is the
  largest share of the picture that can lie once; the floor under the cube (z 0.5..0.2) is the second copy.
- Measured: ink / picture **0.9146** (0.90 plus UV-gutter texels), dup 0.000 %, cube 13,561 texels. Round 6 (1.55) and
  round 8 (1.358, dup 14.8 %) passed `>= 0.95` only by painting the floor under the cube as well.
- So the limit `ink >= 0.95` contradicts "the picture lies once" for this placement. Not changed (binding: never
  weaken an assert, never change a setup without the user). **Decision for the user**: either move WA17 / WA24's
  picture so all of it has surface under it (WA31 does this: centre z = 0.95, ink 1.0142, dup 0) or lower the limit
  to the geometric 0.90 for this placement. WA24's second half (the 5 x 5 cube grid) is not reached while the first
  assert fails.

## 3. Break list (plan section 8) — tests `check_a9_*` in `wrap_tests.rs`

| Case | Before check:A | After | Test |
|---|---|---|---|
| Cube without a bottom face on a floor (WA31's placement, Jump on) | ink 0.9983, dup 0, cube 17,484 texels, floor under the cube bare | same (works: the spine climbs the side; the floor under the cube is not laid as a rival there) | `check_a9_a_piece_takes_its_share` |
| Closed cube (same placement) | ink 1.0124, dup 0, cube 18,859 texels | same | same |
| Two stacked boxes (picture on the lower top, onto the upper box) | ink 1.0138, dup 0, upper box 20,681 texels | same | same |
| Mug handle (half torus sunk into an open cylinder), Jump on | ink 1.0015, dup 0.014 %, covered 1.0, flipped 23, stretch 0.92 (the handle tube laid flat) | same | `check_a9_a_mug_handle_with_jump` |
| Helix on a cone (frustum, 15 deg, 1.5 x the way round; pitch ~0.73 between the picture's half height 0.4 and full height 0.8) | **bug**: the pass rule never armed (heads never within one across reach), the spine wound on 1.5 turns to the triangle rule (`met_itself`, ends 0.9565), S1 60 / S2 26 cut the overlapping band out: covered 0.867, **lost_cut 4.95 %, one cell 94 % lost** | passed at ends 0.549, covered 0.772, ink 0.774, dup 0.021 %, lost_cut 0.45 % | `check_a9_a_helix_on_a_cone` |
| Rivet next to a box corner (12 x 12 cube faces: the rivet's ring is one 0.125 edge from its tip, farther than `CONE_RADIUS` = 0.1 x across = 0.046) | **bug**: T3 saw the tip alone (net 74 deg): S4 2 (tip and corner), covered 0.909, lost_cut 9.2 %, a cell 98 % lost | S4 1 (the corner), covered 0.947, lost_cut 5.5 %; the corner alone: covered 0.972 | `check_a9_a_rivet_on_a_box_corner` |
| Picture smaller than one triangle (floor quad, cube face; Jump / Overlap on and off) | exact (seed, covered 1, no cut, not relaxed, dup 0); ink 0.90-0.92 and cells 0.3-2.6 are texel quantisation (the picture is 15 x 7 / 5 x 2.5 texels) | same | `check_a9_a_picture_smaller_than_one_triangle` |
| 1 vs 12 threads; 256^2 vs 2,048^2 maps (open cube, mug, cone helix, rivet corner) | byte-identical / layers and stats equal | same | `check_a9_threads_and_texture_sizes` |

Fixes:
1. **T2, pass distance** (`wrap_param.rs`, `PASS_NEAR = 2.0`): the traces have passed when their heads pass within two
   across reaches (the picture's full height: the bands overlap), not one. WA32's three cases are unchanged (same
   ends_share, their pitch is below the half height).
2. **T3, 1-ring** (`wrap_relax.rs` `seams`): the net deficit sums over at least the vertex's 1-ring (its longest laid
   edge) when that is farther than `CONE_RADIUS x across`; the strongest-only rule keeps `CONE_RADIUS` (a coarse cube's
   corners still each cut). WA33's plate (cells finer than the radius) is unchanged.
3. **T3 knob**: `WTE_TEST_CONE_RADIUS=0` still gives round 8's rule (the 1-ring is not added to a radius of 0).

## 4. Open items (4) and (6)

**(4) `lost_cut` at closures (WA37 red: WA22 taller 5.67 %, WA32 64 rows 2.48 %; Lantern 11.2 %).** Traced with a
test-only dump of the owner grid (a PGM per layer; the dump code was removed afterwards): every counted cell lay next to the
closure line. Where the halves (S2) or the ends (S1) meet, the line is triangle-jagged: the teeth between one side's
triangles are the other side's triangles, laid at the other end of the picture (the picture there shows its other
edge, once). With a helix the band past the meeting line is uncovered too. The row / column rule counted these as
"inside the outline" because the halo on the far side of the row closes the span. It is a measuring error, not a
seam opening under ARAP. Fix (`wrap_relax.rs` `OwnerGrid::build`, plumbing in `wrap_param.rs`): triangles next to a
closure (relaxed: an S1 / S2 cut edge; exact seed: torn) are flagged; uncovered picture cells 4-connected through
uncovered picture cells to a cell such a triangle covers are "beyond the meeting line", not `lost_cut`. Cut wedges
still count (rivet next to a corner 5.49 %, unchanged; box corner 1.88 %). WA37 now: WA22 taller 0 %, WA32 64 rows 0 %,
thin torus 0 % (was 0.34 %), all limits met — **green**.

**(6) Wall-clock flake.** `check_a_own_bad_meshes_never_fail` and WA7 take, for a case over 200 ms, the best of that
run and up to four more (`best_param_ms`); the limit stays 200 ms. Not a fix for the one-thread case: with
`RAYON_NUM_THREADS=1` WA7's overlapping_cubes (Jump + Overlap) takes 308 ms every time (reported in section 1).

## 5. A5 — exact region (T5), built by check:A

Built (`wrap_relax.rs` `Layout::relax` / `exact_region` / `exact_holds`, `EXACT_STRAIN = 0.25`;
`samples::capped_cylinder` (the WA1 cylinder's side in 16 rows plus a hemispherical dome); test
`wa35_exact_parts_stay_exact`; WA26/27 round-9 case added; ARCHITECTURE 1.6c, 5.12, 6.5a, 9.1):
- A laid vertex with **one wedge** whose laid triangles are all isometric in the seed (not torn, edge ratios within
  `ISO_TOL`), connected to a pinned strip seed through such vertices, is pinned at its seed position `p2` (the seed
  path's corners: bit-exact). The component loop is now a closure `solve(x0, pins)`, run with the exact pins; if a
  free laid triangle meeting the picture is flipped or the free triangles' 95th-percentile `|area ratio - 1|` exceeds
  `EXACT_STRAIN`, it runs again with the strip pins only (round 8). Deviation from T5's wording: "one wedge" (a vertex
  on a seam is left free; the plan does not say what a seam vertex's pin would be). Test-only thread-locals
  (`EXACT_REGION`, `EXACT_OUTCOME`) switch it off and report pins / fallback; `WTE_TEST_EXACT_DEBUG=true` prints the
  counts.
- No other test changed by it (wrap module: the same four red tests with the same numbers as before A5; WA4, WA20,
  WA21, WA22 unchanged: their seeds have no isometric region connected to the strip, or the strain check falls back).

| WA35 case | Plan limit | strip pins only (round 8) | exact region |
|---|---|---|---|
| capped cylinder, picture 0.7 on the side, 0.5 over the dome (face 20, y 0.3..1.5) | side texels 99.9 % within 2/255, max 8 | 82.02 %, max 255, cg 461 | **92.37 %, max 51** (49 exact wedges), cg 372 — **red** |
| same: seams / scatter | 0 / <= 2 % | — | 0 / 0 % — green |
| capped cylinder, picture just below the dome (y 0.25..0.95; added) | 99.9 % | seed exact (early-out): 100 %, max 1 | same |
| plate with one 45 degree bump in the picture (off the centre line) | flat texels 99.9 %, max 8; seams 0, scatter <= 2 % | — | **100 %, max 1** (4,417 exact wedges), seams 0, scatter 0 — green |
| 20 degree cone, apex in the picture | strain check trips, result = strip-only pins | — | fell back (769 exact wedges tried), layers bit-equal to the strip-only result — green |
| WA25 extended: rejected on the capped cylinder | 0 | — | 0 — green |
| WA26/27: capped cylinder 1 / 2 / 5 threads, reuse = fresh | byte-identical | — | green |

**Why the capped cylinder's side is not exact:** the seed (the front's layout) is itself not isometric on the side
next to the dome (`WTE_TEST_EXACT_DEBUG`): of 331 laid triangles 132 are isometric; side triangles at y 0.625..0.875
within five columns of the spine have seed edge ratios up to 1.23 (the dome's unfolding reaches the side's upper rows
through the front's candidates at the rim), and rows down to y 0.25 are just over `ISO_TOL` (1.0005). Only 57
vertices are exact; the rest is solved, and ARAP spreads the dome's strain into the side. T5 as specified pins the
**seed**; making the side exact here needs a better seed next to curved parts (the front's consensus at the rim), not
in this round. The assert stays (red), measured value above.

## 6. Open item (1) — hidden surfaces winning the owner choice (Jump on): tried, not fixed

Experiment: the `buried` ray's reach raised from `BURY_REACH x diag` to at least the bucketed across reach (the
picture's half height), so a hidden face farther from the part over it than 0.04 diag (Avocado's seed back in its
cavity) would count as buried. Release exe on Avocado / DamagedHelmet / Lantern: **no change** (Avocado on ink 0.9976
-> 0.9977, dup 0.787 -> 0.568 %, rejected 34.79 % both; helmet on ink 0.8424 -> 0.8423, rejected 16.95 % both;
Lantern identical). So the reach is not what decides these cells; the cause is elsewhere (likely the `(g)` order
between candidates that are not in a "mixed" cell, or halo triangles). Reverted (not committed). Open.

**Open item (2)** (a part welded to the floor at shared vertices is the same part, so `buried` never applies): not
attempted; it needs a notion of "part" finer than weld connectivity (feature edges, materials), not a contained fix.

**Lantern `lost_cut` 11.2 %** is not a closure (no S1 / S2 there: seams [0,0,0,11,19]): the closure rule of section 4
does not change it. Open (cut wedges at the box corners' S4 / S5 cuts on a very coarse mesh; gate limit 10 %).

## 7. Final state (HEAD after the last check:A commit)

- `cargo test --workspace --no-fail-fast`: **1,161 passed / 6 failed / 33 ignored** in the run that had my exe
  sweep running beside it; the extra failure was `check_a_own_bad_meshes_never_fail` (best of five runs 211.6 ms,
  cone fan x3 Jump + Overlap) under that load. Re-run of the core library without other load: 756 passed / **5
  failed** (alone the test's slowest case is 95-103 ms, with or without the exact region). Failing, asserts unchanged:
  - `wa4_r8`: 60 deg only, ink 1.1177 (<= 1.10), cells 0.769..1.432 (<= 1.30).
  - `wa20`: ink 1.1890 (<= 1.15), cells 0.665..2.886 (0.70-1.40); dup 0.976 %, scatter 0.564 %, seams 0 meet theirs.
  - `wa17`, `wa24`: ink 0.915 / 0.9146 (>= 0.95): geometric maximum 0.90 (section 2, decision for the user).
  - `wa35` (new, check:A): capped cylinder side 92.37 % within 2/255, max 51 (limit 99.9 %, max 8); the other WA35
    lines are green (section 5).
  - `wa37`: now green.
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: ok
  (`target\round9\bin\wte_c9a_final.exe`).
- Identity with `wte_r67.exe` (`target\round9\ident\c9a_final\`): `stamp`, `text_warp`, `export_run`, `roundtrip`
  ok, `shot.png` 0 differing pixels each; export_run's 7 export files byte-identical; `check_exports.py`: PASS.
- `wrap_surface` on the built-in sample: ok, no errors.

### Exe sweep, step 10 (round 8 -> now; `target\round9\check_a\runs_2750d96\`, the exe of the A5 commit; the final
exe differs only in test-only code)

| Model | Jump | ink | dup % | covered | seams | other now |
|---|---|---|---|---|---|---|
| Avocado | off | 0.5288 -> 0.5415 | 0 -> 0 | 0.532 -> 0.545 | 0 -> 0 | reached_edge |
| Avocado | on | 1.5295 -> 0.9976 | 52.6 -> 0.787 | 1.0 -> 1.0 | 0 -> 0 | rejected 34.8 %, scatter 0.50 % |
| torus_knot | off = on | 0.4051 -> 0.4545 | 0 -> 0 | 0.404 -> 0.440 | 84 -> 48 (S1) | passed, ends 0.430, lost_cut 0 |
| Lantern | off = on | 0.8273 -> 0.7704 | 26.9 -> 1.68 | 1.0 -> 0.885 | 28 -> 30 | stretch 220 -> 31 %, lost_cut 11.2 % |
| DamagedHelmet | off | 0.5065 -> 0.5094 | 0 -> 0 | 0.655 -> 0.660 | 16 -> 11 | lost_cut 0.77 % |
| DamagedHelmet | on | 1.0941 -> 0.8424 | 0 -> 0.036 | 1.0 -> 1.0 | 140 -> 69 | rejected 17.0 %, flipped 4 |
| WaterBottle | off = on | 1.0017 -> 1.0033 | 0 -> 0 | 1.0 -> 1.0 | 0 -> 0 | stretch 2.1 -> 3.8 % |
| klein_bottle | off = on | 1.105 -> 1.0132 | 4.13 -> 0 | 1.0 -> 1.0 | 0 -> 0 | stretch 46.8 -> 23.7 % |
| high_poly_sphere | off = on | 1.0427 -> 1.0977 | 0 -> 0 | 1.0 -> 1.0 | 176 -> 0 | param 11.0 -> 6.2 s |

Shots read (`target\round9\check_a\cmp_*.png`, `z_four.png`, `z_helmet.png`, round 8 left):
- **Avocado**: one picture: the seed carries the middle cells, the cut face to its right the rest; no cell twice
  (round 8 showed the left columns on the seed and again on the flesh). A bare strip of flesh is left at the top left
  between the seed's outline and the picture's left edge (open item 1).
- **torus_knot**: a band of cells once round the tube over its whole height, the ends meeting in one line (round 8: a
  thin dark sliver).
- **Lantern**: the base's front and side faces carry continuous cells meeting at the box edge; the plinth below and
  the cap above are partly bare (lost_cut 11.2 %); no smear (round 8: smeared cells on the plinth).
- **DamagedHelmet** (Jump on): the visor carries one continuous picture; to its right the shell pieces show fewer,
  cleaner cell pieces than round 8's jumble, with some bare gaps at the lower right (ink 0.84).
- **WaterBottle**: one whole 8 x 4 picture on the body, same as round 8.
- Parameterization times are often longer than round 8 on small models (Avocado 9.6 -> 38 ms, helmet on 60 -> 145
  ms; high_poly_sphere faster 11.0 -> 6.2 s): package B's subject.
