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
