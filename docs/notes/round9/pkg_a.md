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
