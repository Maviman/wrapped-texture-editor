# Round 9, package A (R9-1, R9-2: core wrap quality) — builder notes

Worktree `G:\software_projects\texture_wrapper_ap_r9a`, branch `round9-a` (from `round8-wip`). Builder: build:A (Opus).
Date 2026-11-22. Logs: `target\round9\` (git-ignored).

## Baseline (round8-wip on this machine)
`wa4_r8` (60 deg: ink 1.1360, cells 0.785..1.599), `wa20` (ink 1.6373, dup 10.684 %, scatter 4.077 %, cells
0.754..3.302, flipped 45, cg 1517), `wa22` (larger than the torus: dup 2.265 %), `wa24` (cube on floor: ink 1.3582,
dup 14.832 %) red, as the plan says. WA25: every case relaxed false, rejected 0.

## A1 place owner (T1)

Built (wrap_relax.rs `OwnerGrid`, wrap_param.rs `make_layer` / `OwnerCtx`, wrap_bake.rs, samples.rs, wrap_tests.rs):
- `OwnerGrid` per layer (as before: <= 1024 cells on the long side over the reach rectangle) now stores per cell the
  **owner of the cell centre**: the stored triangle covering the centre (barycentric inside test, edges included) with
  the smallest `(buried, g, triangle)`; per cell its `g` (f32), 3D point (`[f32; 3]`, from the stored f32 corners'
  barycentrics and the mesh's f32 positions) and stretch `s = sqrt(3D area / layout area)` clamped 1..4
  (`owner_scale`, the same f32 formula in the bake). A centre no triangle covers: no owner, every fragment kept.
- `keeps(k, p, s)`: `|p - owner point| <= OWNER_SAME (3) x cell diagonal x max(1, s, s_owner)`. No triangle size in
  the rule. `OWNER_TOL` removed; `OWNER_SAME`, `BURY_REACH = 4 x GAP_JUMP` added (pub).
- Bake: `WrapJob` carries the triangle's 3D corners and `s` (`depth`, `edge` gone; `WrapLayer::edge` gone, `depth`
  kept for `surface_of` without an owner). The fragment's 3D point = its barycentrics over the 3D corners.
- `surface_of` (outline, text-path handles): of the triangles containing the point, the one whose 3D point there lies
  nearest the cell owner's point; without an owner the lowest `g` (as before).
- `buried` (answer (a)-1), only in **mixed** cells: candidates of two or more parts **and** at two or more places (a
  candidate farther than half the smallest tolerance from the first). `OwnerCtx::buried_at(p, n, part)`: true when a
  triangle of another *entered* part touches `p` parallel to it (`query_sphere_dist` radius `1e-5 diag`, `|n . n'| >
  0.99`), or the ray from `p` along the oriented normal hits another entered part within `[1e-5, BURY_REACH] x diag`
  (`Bvh::raycast`, up to 16 hits skipped over own / not-entered parts).
- Cost control (needed: WA7's 200 ms limit, debug build): `may_bury(tri)` once per triangle (any triangle of another
  entered part meets the box of the prism the triangle's normal sweeps over `BURY_REACH`, grown by `1e-5 diag`); in a
  mixed cell pass 1's owner (lowest `(g, triangle)`) stands unless it may be buried **and** is; only then the cell's
  candidates are sorted by `(g, triangle)` and asked lazily (first not buried wins, else the first). The build runs
  per 16-row chunk in parallel (each cell written by its own chunk, triangles in ascending order per chunk; integer
  counts): the same owners with 1 or N threads.
- `rejected` = share of the (cell centre inside the picture, **laid** triangle covering it) pairs whose triangle is not
  the owner's place. Halo triangles do not count (see deviation 2).

Deviations / findings:
1. **Gutter fragments** (outside their triangle in UV space; barycentrics extrapolate) are tested at the triangle's
   nearest point (barycentrics clamped to >= 0 and renormalised; picture point and cell from those). With the
   extrapolated 3D point a fold (cube edge, thin slab, the gap of WA16) rejected the island padding: WA5 97.7 %, WA8
   98.5 %, WA16 99.25 % within 2/255 (limits 99.9 %). With the clamp all three are exact again (100 %).
2. **`rejected` counts laid triangles only.** WA16 (two blocks, Jump across the gap) is not injective: the blocks'
   inner side walls are halo triangles hinged down from the tops' edges, so in the layout B's -X wall lies under A's
   top and A's +X wall under B's top (round 6 and 8 painted the picture on those hidden walls too: the round-8
   tolerance in edge lengths kept them). The place owner now drops them (correct: a second copy), and counted over all
   stored triangles `rejected` was 0.5. `WrapStats::rejected` was documented in round 8 as "share of the laid picture
   area the owner test drops"; it now counts exactly that, and WA25 (assert unchanged, `rejected == 0`) passes. The
   hidden-wall drop is visible only in the bake (no change on the tops: WA16 100 % exact).
3. **WA17 / WA24 `ink >= 0.95` cannot be met by a picture that lies once** (assert kept, test red): WA17's picture
   (centre z = 1.2, half length 1.0 towards the cube) runs from z = 2.2 to z = 0.2, and the floor ends at z = 2.0, so
   0.2 of its 2.0 length (0.10 of its area) has no surface under it. The rest is 0.75 on the floor and 0.15 up the
   cube's side: at most 0.90 of the picture can lie once; measured **0.9146** (0.90 plus UV-gutter texels), dup 0 %,
   cube 13,561 texels, rejected 0.142 (the floor under the cube, buried). Round 6 (1.55) and round 8 (1.358, dup
   14.8 %) met 0.95 only with the second copy on the floor under the cube. WA24's grid half: not reached (the test
   stops at the first assert); WA31 covers the density question with a placement that lies wholly on surface.
4. **Welded contact**: with a floor grid whose vertices coincide with the cube's bottom corners (n = 8, 64 with
   uniform lines) the position weld makes floor and cube one part: no jump, no `buried` (one part), ink 1.23 with
   the cube's sides placed by the one-corner frame rule. `uv_cube_on_floor_n` therefore shifts its interior grid
   lines by 0.37 of a step. Open for check:A: a part that touches the floor at shared vertices is not a separate
   part, so "the piece takes its share" does not apply to it.

| Test | Plan limit | Measured (A1) |
|---|---|---|
| WA22 taller / larger than the torus | dup <= 1 %, scatter <= 3 % | dup 0.013 % / **0.000 %** (was 2.265 %), scatter 0.006 / 0.000 % — **green** |
| WA24 cube on floor | ink 0.95-1.10, dup <= 1 % | ink **0.9146** (geometric max ~0.90, deviation 3), dup 0.000 % — red |
| WA17 (round 6) | ink >= 0.95 | **0.915** — red (same cause; was green only through the duplicate) |
| WA25 | 0 rejected, not relaxed | 0 everywhere, not relaxed — green |
| WA31 floor 2 / 128 / 8,192 tris | dup <= 1 %, ink 0.95-1.10, cube receives, inks within 2 % | ink 1.0142 / 1.0187 / 1.0152, dup 0 / 0 / 0 %, cube 23,001 / 23,019 / 23,019 texels — green |
| WA20 | ink 0.85-1.15, cells 0.70-1.40, dup <= 1 % | ink 1.4669, dup 1.897 %, scatter 1.401 %, cells 0.752..2.714 — red (A4) |
| WA4 60 deg | ink <= 1.10, cells <= 1.30 | 1.1360, 0.785..1.599 (unchanged: injective, rejected 0) — red (A4) |
| WA7 slowest parameterization | 200 ms (debug) | 82 ms alone (overlapping_cubes x1, Jump + Overlap); 564 ms before the chunked / lazy build |

## A2 spine passing (T2)

Built (wrap_param.rs `trace_spine`, `wrap_param`; stamp_job.rs log lines; app `tool_options.rs` test literal: the
minimal fix for the new `WrapStats` fields):
- `Spine::passed`, `WrapStats::passed`, `WrapStats::ends_share` (`(end0+ + end0-) / (2 x needed along)` clamped to 1
  when the spine met itself or passed and one turn is laid; 1 for an open spine and for Overlap turns), and (measuring
  aid for A3, A7's field) `WrapStats::lost_cut`. The `wrap:` log lines (stamp_job.rs) end with `passed {}, ends {:.1}
  %, lost_cut {:.2} %`.
- The pass check runs at the top of every spine step while both traces are active on turn 0: gate `s+ + s- > across`
  and oriented normals agreeing (`nu+ . nu- > 0`); **armed** once the head distance `d` falls (below the previous
  check) under the bucketed across reach; **passed** when, armed and `d < across`, the - head's offset along the +
  trace's direction (`(q- - q+) . d+`) changes sign from > 0 to <= 0. The ends are set where that offset is zero
  (linear interpolation of the two traces' arc lengths between the two checks). Ends meet: both traces stop; Overlap:
  both start their next turn (`TurnSpan` from that arc length; the turn limit as `enter_turn`).
- A passed spine is **closed** like a met one: `period0 = end0+ + end0-` (S1 by the feet across the meeting line, the
  early-out's closure tears) and with Overlap the later turns are laid (`overlap_turns = overlap && (met || passed)`).

Deviations from T2 (with reasons):
1. **The gate is `s+ + s- > 1 x across`, not `4 x`.** WA32's own cylinder (turn 3.25, across reach 1.11) passes at
   `s+ + s- = 3.03 < 4 x 1.11`; the thin torus at 0.88 with across 0.55. The start is guarded instead by "armed only
   while the distance falls" (at the start it rises).
2. **Passing = the along offset changes sign, not "d rises after its minimum".** With the heads offset across the
   picture (the helix's pitch) the 3D head distance is smallest before the heads are level along the picture
   (`d^2 = chord^2 + dy^2`, `dy` grows while the chord shrinks): ends_share 0.6205 with the minimum rule against
   0.6183 measured / 0.6196 analytic with the sign rule on the cylinder sample. The ends meet where neither end lies
   ahead of the other.
3. **The analytic share is `P cos(tilt)`, not `P / cos(tilt)`.** Unrolled, the cylinder repeats with the shift
   (perimeter P, 0), i.e. `P cos t` along the picture and `P sin t` across it. A picture taller than `P sin t`
   (WA32: 1.8 against 0.81) lies once for a length of `P cos t`: longer and its ends overlap. The test uses that.
4. WA32 has a 64-row copy of the cylinder sample besides the sample itself: on the sample (one row of 4-unit
   triangles) round 8's triangle rule closes the spine first (`met_itself`) when the pass check is off; with it on the
   pass check fires first there too (both cases report `passed`). The torus value is the cylinder formula with the
   tube's girth and the starting angle (Clairaut moves the angle from 20 to 27.6 degrees on the inside): measured
   0.5588 against 0.5715 (2.2 % off).

| WA32 case | ends_share (analytic) | covered (>= 0.9 x ends) | ink | dup / scatter | Overlap turns |
|---|---|---|---|---|---|
| cylinder sample, 15 deg | 0.6183 (0.6196) | 0.6645 | 0.6651 | 0 / 0 % | 3 (ink 0.794, dup 0) |
| cylinder 64 rows, 15 deg | 0.6187 (0.6196) | 0.7920 | 0.7934 | 0 / 0 % | 3 (ink 0.853, dup 0.029 %) |
| thin torus R 1 r 0.15, 20 deg | 0.5588 (0.5715) | 0.7390 | 0.7364 | 0 / 0 % | 3 (ink 1.209, dup 25.1 %, not asserted) |

Other tests after A2: WA3, WA18, WA19, WA21, WA22 unchanged and green; the full core run has the same four red tests
as after A1 (`wa4_r8`, `wa20`, `wa17`, `wa24`); one run of `check_a_own_bad_meshes_never_fail` hit 220 ms (book x0.05,
Overlap) under the parallel test load; alone it is 51 ms (44 ms without A2): load, not the pass rule.

## A3 cones (T3)

Built (wrap_relax.rs `seams`, `laid_within`; `CONE_RADIUS = 0.1` pub; samples `riveted_plate(height)`,
`step_corner()`; WA33, WA34; the owner build runs on the pool only from 2,048 triangles / 20,000 mixed cells):
- S4 fires at an interior vertex `v` with `|deficit| > 45 deg` only when the deficits of the laid interior vertices
  reachable from `v` over laid edges within `CONE_RADIUS x bucketed across reach` (3D distance from `v`) also sum to
  more than 45 deg in absolute value; of such vertices within one radius only the strongest (largest `|deficit|`, then
  lowest position rank) cuts. Test knob `WTE_TEST_CONE_RADIUS` (0 = round 8's rule) for the before / after numbers.
- The owner build (A1) now runs its chunks serially below 2,048 triangles (pass 2 also below 20,000 mixed cells):
  on a busy pool a small parameterization waited for it (`check_a_own_bad_meshes_never_fail`, a wall-clock limit of
  200 ms, failed at 220-376 ms in full parallel runs; alone 51-59 ms). It still failed once more at 223 ms (cone fan
  200, x0.05) in a full run after this change, alone it passes: the limit is wall-clock under the parallel test load
  (my new tests add load). Open for check:A.

**What the helmet's lost 0.345 is made of** (exe `wrap_surface` on DamagedHelmet.glb, step 10, Jump off; `lost_cut`
= picture cells without an owner inside the laid set's outline):

| DamagedHelmet, Jump off | round 8 (r8 sweep) | A1+A2 (before T3) | A3 (after T3) |
|---|---|---|---|
| covered | 0.6548 | 0.6548 | 0.6542 |
| reached_edge | true | true | true |
| lost_cut (cut wedges, holes) | — | 1.69 % | 1.39 % |
| seams S1-S5 | 16 [0, 0, 5, 4, 7] | 16 [0, 0, 5, 4, 7] | 10 [0, 0, 5, 0, 5] |
| ink / picture, dup, scatter | 0.5065, 0 %, 0 % | 0.5064, 0 %, 0 % | 0.5060, 0 %, 0 % |

So of the 0.345 of the picture without surface under it, about 0.014-0.017 lies inside the laid outline (cut wedges
and holes); the rest, ~0.33, is beyond the edge of the part the picture is on (the visor part ends; `reached_edge`;
Jump off does not continue onto the next part). T3 removes the four S4 cuts but cannot give that third back: it is not
a cut. With Jump on: S4 28 -> 13, seams 140 -> 60, lost_cut 9.48 % -> 6.75 %, ink 0.7565 -> 0.7850 (round 8: 1.0941,
dup 0 %), rejected 24.4 % -> 21.4 %. **Open (check:A, gate 7.2 wants ink 0.90-1.3 with Jump on):** the place owner
drops 21 % of the laid (cell, triangle) pairs on the helmet with Jump on; the shot shows the right-hand part of the
picture (over the shell parts right of the visor) partly bare where round 8 painted it. Likely the `(buried, g)` order
picks a hidden part with a lower `g` that the `buried` ray does not catch (gap beyond `BURY_REACH`, or a hidden face
whose oriented normal points away from the covering part). Not traced further in this package.

| Test | Plan limit | before T3 (radius 0) | after T3 |
|---|---|---|---|
| WA33 riveted plate (80 x 80, 25 rivets, tip deficit 94.1 deg, h = 2 % of the picture) | seams 0, covered >= 0.99, ink 0.95-1.10, dup 0 | seams 205 [0,0,0,25,180], covered 1.0, ink 0.9946, dup 0.359 %, lost_cut 2.49 %, rejected 4.6 % | **seams 0, covered 1.0, ink 1.0135, dup 0 %, lost_cut 0, rejected 0** — green |
| WA33 cube corner (WA23's) | one S4 cut | S4 1 | S4 1 (dup 0.926 %, ink 0.9333) — green |
| WA34 step corner (+90 / -90 deg, 0.3 apart) | dup <= 1 %, scatter <= 3 %, lost_cut reported | seams [0,0,0,1,3] | same (the two vertices lie farther apart than the radius, 0.04): dup 0.140 %, scatter 0.113 %, ink 1.0009, lost_cut 0 — green |
| WA23 cone tip / cube corner | unchanged | — | unchanged, green |

## A4 Fermi start (T4)

Built (wrap_param.rs `Layout::foot_c`, `add_cand(.., foot_c)`; wrap_relax.rs `fmm_update_tau`, `Layout::fermi`,
`relax_component_from`):
- `foot_c` per vertex: seeds their (unclamped) along coordinate; the triangle update interpolates `foot_c(v) + tau
  (foot_c(a) - foot_c(v))` with `tau = -alpha / beta` of the fast-marching update, the edge update takes the nearer
  end's; the candidate that sets the key sets it (keys, positions, `foot` / `side` and so the seams stay bit-identical
  to round 8). Jump seeds inherit it from the source.
- Strip components start their free wedges at `e foot_c + rot90(e) side sqrt(g^2 - out^2)` (`out` = how far the
  foot lies beyond the spine's along range; deviation from the plan's `side x g`, which would put a vertex beyond an
  end of the spine sideways instead of beyond the end). Jumped parts and components without a strip keep the seed
  start (their pins are the landing triangle re-placed from the relaxed source / their lowest-g triangle's seed
  corners: a Fermi start from the strip's frame would not match those pins). Knob `WTE_TEST_SEED_START`.
- No LSCM: ARAP only. `RELAX_ARAP_ITERS` 12 (measured table below), stop when the energy drops < 0.05 %
  (`ARAP_STOP` 0.0005), <= 60 CG steps per solve (`ARAP_ITERS`), budget 2,500. LSCM stays behind `WTE_TEST_LSCM=true`
  and in `relax_component` (test-only now), which check:A's solver tests run from a crumpled start (from a crumpled
  start ARAP alone does not untangle a 25 x 25 grid pinned at two corners: worst area ratio 0.63 against its 0.2).

T4 table (debug build, `cells` = the 32 picture cells' ink over their share; `cg` = CG steps; plan's experiment:
ARAP 6 WA4 60 deg 1.116 / 0.760-1.445, WA20 ink 1.26 dup 0.99 % flipped 19 cells 0.637-3.149; ARAP 30 WA20 ink
1.19, WA4 60 deg 1.119 / 0.742-1.506):

| case (limit) | round 8 (LSCM + ARAP 6) | ARAP 6 | **ARAP 12 (chosen)** | ARAP 30 | ARAP 12, CG 200 | ARAP 30, CG 200 |
|---|---|---|---|---|---|---|
| WA4 30 deg (ink 0.90-1.10, cells 0.75-1.30) | 1.0281, 0.957..1.117, cg 1046 | 1.0241, 0.951..1.132, cg 360 | 1.0213, 0.948..1.116, cg 720 | 1.0269, 0.955..1.109, cg 1799 | 1.0209, 0.949..1.117 | 1.0261, 0.954..1.110 |
| WA4 45 deg (same) | 1.0742, 0.906..1.283, cg 1219 | 1.0654, 0.918..**1.309** | 1.0571, 0.899..1.258 | 1.0620, 0.897..1.247 | 1.0566, 0.899..1.258 | 1.0571, 0.895..1.242 |
| WA4 60 deg (ink <= 1.10, cells 0.75-1.30) | 1.1360, 0.785..1.599 | 1.1171, 0.759..1.449 | **1.1159, 0.745..1.481** | 1.1193, 0.742..1.507 | 1.1157, 0.745..1.479 | 1.1192, 0.742..1.507 |
| WA4b 4 periods 60 deg (0.85-1.15, 0.65-1.50) | 1.1230, 0.788..1.439 | 1.1242, 0.811..1.464 | 1.1111, 0.796..1.466 | 1.1100, 0.797..1.470 | 1.1113, 0.796..1.466 | 1.1101, 0.797..1.470 |
| WA20 (ink 0.85-1.15, cells 0.70-1.40, dup <= 1 %) | 1.4669 (A1), dup 1.897 %, 0.752..2.714, flipped 45, cg 1517 | 1.2441, dup 0.884 %, 0.647..2.767, flipped 18, cg 360 | **1.1890, dup 0.976 %, scatter 0.564 %, 0.665..2.886, flipped 15, cg 720** | 1.1725, dup **1.034 %**, 0.681..2.841, flipped 17, cg 1560 | 1.1885, dup 0.991 %, 0.656..2.879 | 1.1765, dup 1.041 %, 0.673..2.870 |
| WA21 (flipped <= 1 %, dup <= 1 %) | flipped 0, dup 0 % | flipped **1.746 %**, dup 0.819 % | flipped 0.044 %, dup 0.315 % | flipped 0.015 %, dup 0.014 % | 0.073 %, 0.477 % | 0.029 %, 0.388 % |
| WA22 taller / larger (dup <= 1 %) | 0.013 / 0.000 % | 0.013 / 0.026 % | 0.006 / 0.026 % | 0.038 / 0.026 % | — | — |

ARAP 6 fails WA4 45 deg (cells 1.309) and WA21 (flipped 1.75 %); ARAP 12 is the cheapest setting that meets WA4 30 /
45, WA4b, WA21, WA22, and it is the best on WA4 60 deg (ink 1.1159, top cell 1.481) and the only one of 12 / 30 that
keeps WA20's dup under 1 %; ARAP 30 has WA20's lower ink (1.1725) but dup 1.034 %. More CG steps per solve change
nothing that matters (the solves stop at the cap of 60, but the next iteration continues from there). Chosen:
**ARAP 12, CG 60 per solve**.

Remaining red (asserts unchanged): **WA4 60 deg** ink 1.1159 (<= 1.10), cells 0.745..1.481 (0.75-1.30); **WA20** ink
1.1890 (<= 1.15), cells 0.665..2.886 (0.70-1.40) — dup 0.976 % (<= 1 %), scatter 0.564 % (<= 2 %), seams 0, covered
1.0 meet their limits. As the planner measured: the start removes the fold next to the strip (WA20 flipped 45 -> 15,
dup 10.7 % (r8) -> 0.98 %) but does not bring the steep bumps inside round 8's area limits (D4b: with the centre line
exact an as-rigid-as-possible sticker takes up more surface than its own area there). WA17 / WA24 (A1 deviation 3)
are the other two red tests.

## A6 resolution (T6) — done before A5 (A5 is first in the cut order; A6 is "never cut")

Built (wrap_param.rs `reach_of`, `stop_of`, `max_gutter`, `WrapParam::{gutter, stop_bits}`, `serves_with`):
- `reach = [bucket(along), bucket(across)]` with `along` / `across` = the picture's half extents plus one source
  texel (no `2 x texel_world` before the bucketing). The front's stop is `max(1.7 x across reach, across + (gutter + 2)
  x texel_world)`; the candidate sphere is `along reach + stop` (+ the Jump term).
- `serves_with` keeps the texel key in the form that matters: it compares the bucketed reach and the stop computed
  with the current maps' largest texel size and gutter (stored `stop_bits`), so a reused parameterization stays
  bit-identical to a fresh one.
- Test helpers changed with the definition (not their asserts): WA11, WA27 and `check_a_covered_follows_a_reused_scale`
  build "a reach just above its bucket's start" as `diag 1.25^k 1.002 - 2 tw`; the `- 2 tw` is gone (else the size
  lies just below the bucket and the 1.2 scale crossed it: "a scale of 1.2 recomputed").
- WA36 (new): WA20's ball and the sphere sample with 256^2 and 2,048^2 maps: layers bit-equal, stats equal except
  `micros` — green. (Ball: 2,803 verts, 5,786 tris, flipped 15, stretch 0.728, rejected 0.0015 at both sizes.)
- The bucket change moves WA4 by a bucket on some angles: WA4 60 deg now ink 1.1177, cells 0.769..1.432 (A4: 1.1159,
  0.745..1.481); 45 deg 1.0414, 0.898..1.279; 30 deg 1.0204, 0.951..1.119; WA4b 1.1149, 0.799..1.481. WA20 unchanged
  (ink 1.1890).

## WA37 and WA26 / WA27 extended (tests for A7 / the table)

- `wa26_wa27_round9_cases_are_deterministic` (new; WA26 / WA27 themselves unchanged): WA24's cube on the floor,
  WA31's 8,192-triangle floor, WA32's thin torus: 1 / 2 / 5 threads and two runs byte-identical (Preview and Full);
  a scale x1.0001 reuses the cached parameterization (true in all three) and bakes like a fresh one — green.
- WA37 (new, plan limits): `covered >= 0.95 x ends_share` holds on all cases; `lost_cut <= 2 %` fails on two:

| WA37 case | covered | ends_share | lost_cut (<= 2 %) |
|---|---|---|---|
| WA4 30 / 45 / 60 deg, WA4b | 1.0 | 1.0 | 0 % |
| WA20 bumpy ball | 1.0 | 1.0 | 0 % |
| WA22 taller than the girth | 1.0 | 1.0 | **5.67 %** (red) |
| WA32 cylinder sample | 0.6645 | 0.6183 | 0 % |
| WA32 cylinder 64 rows | 0.8608 | 0.6187 | **2.48 %** (red) |
| WA32 thin torus | 0.7195 | 0.5588 | 0.34 % |
| WA33 riveted plate | 1.0 | 1.0 | 0 % |

  Both red cases are closures (WA22: the halves meet behind the tube, S2; WA32: the ends meet, S1): after the
  relaxation the two sides of the seam line no longer touch everywhere in the layout, and the picture cells in the gap
  are enclosed by covered cells on both sides, so `lost_cut` counts them. Not traced further (open for check:A: is it
  the seam opening under ARAP, or cells beyond the closure that the row/column rule calls "inside").

## A5 exact region (T5) — not started

Not built: A5 is the first item of the plan's cut order (section 10, item 6) and the account's usage was high; A6, A7,
the tests and the verification were done first. WaterBottle's body line (plan 7.2) and T5's speed gain are therefore
not met by this package; WA35 does not exist. The layout code is ready for it (seed, `laid`, pins per wedge).

## A7 and the verification

- `WrapStats::lost_cut` (built with A2, defined in the A6 section's WA37 table and ARCHITECTURE 6.5a "Round 9"),
  logged in both `wrap:` lines (stamp_job.rs).
- ARCHITECTURE.md: 1.6c (round-9 paragraph), 2 (wrap_tests row), 5.12 (constants, `WrapStats`, `serves`), 6.5a
  ("Round 9" block; round 8's owner test marked as replaced), 9.1 (round-9 tests).
- Release build of `7fcd2a1` (exe `target\round9\bin\wte_r9a.exe`, a copy of `target\release\wrapped_texture_editor.exe`).
- **Identity with `wte_r67.exe`** (both run here, `target\round9\ident\`): `stamp`, `text_warp`, `export_run`,
  `roundtrip` all ok, `shot.png` pixel-identical (0 differing pixels, no toast exception needed); export_run's 7
  export files byte-identical to r67's; `check_exports.py` on them: `RESULT: PASS`.
- `wrap_surface` on the built-in sample: ok, no errors.

### Sweep: `wrap_surface` step 10 (Jump off / on), r8 = today's round-8 sweep (`sweep\r8\<model>\shot.log`)

| Model | setting | ink / picture r8 -> r9a | dup r8 -> r9a | scatter r8 -> r9a | covered r8 -> r9a | seams [S1..S5] r8 -> r9a | other r9a |
|---|---|---|---|---|---|---|---|
| Avocado | off | 0.5288 -> 0.5302 | 0 -> 0 % | 0 -> 0 % | 0.532 -> 0.532 | 0 -> 0 | reached_edge; lost_cut 0 |
| Avocado | on | 1.5295 -> **0.9985** | **52.6 -> 0.809 %** | 5.865 -> 0.525 % | 1.0 -> 1.0 | 0 -> 0 | rejected 34.6 %, lost_cut 0 |
| torus_knot | off = on | 0.4051 -> 0.4545 | 0 -> 0 % | 0 -> 0 % | 0.404 -> 0.440 | [76,0,0,0,8] -> [48,0,0,0,0] | **passed, ends_share 0.430** (covered 1.02 x ends_share), lost_cut 0.92 % |
| Lantern | off = on | 0.8273 -> 0.7704 | **26.9 -> 1.676 %** | 10.6 -> 1.854 % | 1.0 -> 0.885 | 28 [0,0,0,19,9] -> 30 [0,0,0,11,19] | stretch 220 -> 31.2 %, flipped 11 -> 1, rejected 16.2 %, **lost_cut 11.2 %** |
| DamagedHelmet | off | 0.5065 -> 0.5094 | 0 -> 0 % | 0 -> 0 % | 0.655 -> 0.660 | 16 [0,0,5,4,7] -> 11 [0,0,5,0,6] | reached_edge; lost_cut 0.77 % |
| DamagedHelmet | on | 1.0941 -> **0.8293** | 0 -> 0.036 % | 0 -> 0 % | 1.0 -> 1.0 | 140 [0,0,5,28,107] -> 56 [0,0,5,10,41] | rejected 18.2 %, lost_cut 3.0 %, flipped 3 |

Against the gate's 7.2 lines (for check:A / the gate; package B and the gate re-run them): Avocado on: ink 0.9985
(0.90-1.15 ok), dup 0.81 % (<= 3 % ok). torus_knot: passed, covered 0.440 >= 0.85 x 0.430, ink 0.4545 >= 0.85 x
0.430 — ok. Lantern: dup 1.68 % (<= 3 % ok), scatter 1.85 % (<= 5 % ok), stretch 31 % (<= 60 % ok), **seams 30 (<= 6
not met)**, **lost_cut 11.2 % (<= 10 % not met)**. DamagedHelmet off: lost_cut 0.77 % (<= 10 % ok); **on: ink 0.829
(0.90-1.3 not met)**.

### Shots (r8 left, r9a right; `target\round9\runs_r9a\sheet.png`, `zoom.png`, `zoom2.png`; each shot is the last
state of the scenario, Jump small gaps on)
- **Avocado**: r8 shows the cells on the seed and again on the cut face round it (the left columns appear twice). r9a:
  one picture: the seed carries the middle cells, the flesh round it the rest, no repeated cell; a bare band of flesh
  is left at the top left between the seed's outline and the picture's left edge (those cells went to the seed's back
  side, hidden: the seed lies in a cavity, so its back is farther than `BURY_REACH` from the flesh and not `buried`).
- **torus_knot**: r8 shows a short dark sliver of the picture round the tube (covered 0.40, the helix cut to one
  pitch). r9a: a band of cells once round the tube over its whole height, ending where the ends meet (43 % of the
  picture's length fits once round; the rest is cut, ends_share 0.43).
- **Lantern** (the picture on the post's square base): r8: cells over the base's faces with smeared cells on the
  plinth below (dup 26.9 %). r9a: the base's front and right faces carry continuous cells meeting cleanly at the box
  edge; the plinth's top below them is mostly bare (lost_cut 11.2 %: picture cells inside the outline without an owner)
  and the cap on top is grey; no smear.
- **DamagedHelmet** (Jump on): the visor carries one continuous picture in both; at the right r8 shows a jumble of
  cells over the shell pieces (several parts painted under each other), r9a shows fewer, cleaner pieces there and some
  dark gaps at the lower right of the visor where cells were dropped to a hidden part (ink 0.83, rejected 18 %).

### Final state (`cargo test --workspace --no-fail-fast`, clippy `--workspace --all-targets` 0 warnings)
1,153 passed / 5 failed / 33 ignored (round8-wip: 1,147 / 4 / 33; +7 new tests). Failing, all with their asserts
unchanged: `wa4_r8` (60 deg only: ink 1.1177 > 1.10, cells 0.769..1.432 > 1.30), `wa20` (ink 1.1890 > 1.15, cells
0.665..2.886 outside 0.70-1.40; dup 0.976 %, scatter 0.564 %, seams 0, covered 1.0 pass), `wa17` and `wa24` (ink 0.915
against 0.95: geometric maximum ~0.90 for a picture lying once, A1 deviation 3), `wa37` (lost_cut 5.67 % on WA22
taller, 2.48 % on WA32's 64-row cylinder, limit 2 %). `check_a_own_bad_meshes_never_fail` (wall-clock 200 ms under the
parallel test load) failed in two of the six full core runs of this round and passed in the others and alone.

### Open items for check:A
1. Jump on: the `(buried, g)` owner drops picture to hidden surface that is not `buried` (Avocado's seed back in its
   cavity, the helmet's lower right: ink 0.83 against the gate's 0.90). A visibility rule beyond the ray (e.g. the
   candidate facing the projector, or a longer `BURY_REACH` for closed parts) is the next thing to try.
2. Parts welded to the floor at shared vertices are one part: no `buried` (A1 deviation 4).
3. WA17 / WA24 ink limits vs the floor edge (A1 deviation 3): the user should decide whether WA17's placement or the
   limit is meant.
4. `lost_cut` at closures (WA37, Lantern 11.2 %): seam opening under ARAP or the row/column rule.
5. A5 (exact region) not built.
6. `rejected` counts laid triangles only (A1 deviation 2).
