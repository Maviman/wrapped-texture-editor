# Round 9, package B (R9-3, R9-4: speed + a small app part) — builder notes

Worktree `G:\software_projects\texture_wrapper_ap_r9b`, branch `round9-b` (from checked `round9-a` @ 7c1e3b4). Builder:
build:B (Opus). Date 2026-11-22. Logs and runs: `target\round9\pkg_b\` (git-ignored; exe `bin\wte_r9b.exe`, sweeps
`sweep_b\` (B), `sweep_a\` (A's `wte_c9a_final.exe`, run while I compiled: its times are not clean), `time_b\` /
`time_a\` (B and A alone, sequential, idle machine), `ident\`, `syncchk\`, `wa15.log`, `parse.py`).

## Steps and commits

| Step | What | State | Commit |
|---|---|---|---|
| (1) | `wrap_mesh.rs` (WrapMesh: weld + snap per mesh, CSR, global edges), `Patch::from_graph`; round 8's `Patch::build` kept as `build_sphere` under `#[cfg(test)]` | done | fb4e3ab |
| (2) | growth from the anchor (`collect_grown`), Jump closure by part boxes; `Jumper::new` without the hash map | done | fb4e3ab |
| (3) | `BakeSets::wrap_mesh` (`WrapMeshCell`), jobs carry it, warm-up | done (warm-up reworked: background thread) | fb4e3ab, B7 commit |
| (4) | Final-wait rule (T8) | done | fb4e3ab |
| (5) | `uv_share.rs` + panel lines (T9; "Longer than the way round") | done | fb4e3ab (+ work cap in the B7 commit) |
| (6) | step 10: `shot_off.png`, coverage asserts, third log state (panel lines) | done | fb4e3ab |
| (7) | WA15 probe with phases, sweep, identity, timings, ARCHITECTURE | done | last commits |

Steps (1)-(6) went into one commit (fb4e3ab) because they were built and tested together (the doc plumbing changes the
callers of `wrap_param`); every later fix is its own commit.

## (1)+(2) Graph and growth (T7)

- `WrapMesh::build` (serial; `WrapMeshCell::get` = `OnceLock`): the same bit-exact weld in order of first use over
  ascending triangles, the same boundary snap (27-cell hash, lowest position wins, greedy in id order) and the same
  collapse rule as round 8's per-patch weld, over the whole mesh; CSR vertex -> triangles (ascending, collapsed left
  out); half-edge -> global edge id, edge -> half-edges (ascending); a global position rank (the patch's rank is only
  compared, never used as a number, so the global rank gives the same order).
- `Patch::from_graph`: local ids by first use over the candidates (zeroed scratch `Vec`s of mesh size: calloc, only
  touched pages cost), half-edge groups built from the global edge lists filtered to the patch (ascending = the old
  order inside a group; the group order itself is never used), parts by union-find (numbering by root = lowest
  vertex: no map), no sort, no hash.
- `collect_grown`: anchor with spheres `1e-3 diag x 4^k` up to the reach radius, the same tie rule; growth over welded
  vertices with the BVH's own closest-point test against the full sphere (incl. the anchor offset). Jump on: the whole
  sphere is queried once, its parts labelled over welded vertices with their boxes, then the closure of parts within
  `GAP_JUMP x diag` (the box test of `Jumper::new`) from the anchor's part.
- Deviation: `Jumper::nearest_other` (not in B's three regions) had one line changed: the landing's local triangle by
  binary search in `patch.tris` instead of the removed per-`Jumper` hash map (built twice per parameterization over all
  candidates).

**WA28 (plan limit / measured):**

| WA28 line | limit | measured |
|---|---|---|
| `cubes_on_floor(5)`, Jump off, picture on the floor | handed <= 1.25 x (laid + halo) | 2 handed / 2 stored (the sphere holds 282 triangles) — **green** |
| graph path = sphere path, WA1-WA5, WA4_r8, WA16-WA27, WA31-WA35, check_a_jump_across_three_parts + 5 extra placements x 3 option sets (cube grid Jump on/off/overlap, cube on floor, 64-row floor, three plates) | byte for byte | **135 parameterizations, 0 differ** (layers, corners bits, depth, clip/period, owner grids g/pos/scale bits, stats except `micros`, reuse keys) — green. Red tests (wa4_r8, wa17, wa20, wa24, wa35) compare only up to their failing assert; WA24's grid half is covered by the extra placements |
| (info) WA31's 8,192-triangle floor, Jump off | — | 1,370 handed / 531 stored: on a fine flat floor the reach *sphere* is larger than the laid band (disc vs stadium); not a WA28 case |

## (3)+(4) Plumbing, warm-up, Final-wait rule (T8)

- `BakeSets::wrap_mesh`, `Document::wrap_mesh_cell` (new cell when the mesh `Arc` changes; `Weak` identity),
  `StampBakeJob::graph`, `wrap_bake` / `wrap_param_only` build fresh parameterizations with `wrap_param_in(.., cell.get())`;
  `wrap_param(mesh, bvh, ..)` keeps its signature (builds a graph per call: A's tests and the probe use it).
- WA38: one cell per mesh, the Preview / ParamOnly / Full jobs hold the same `Arc`, `builds() == 1`, a new mesh gets a
  new unbuilt cell, a Project bake does not build it, 8 x 2 nested rayon tasks on a 2-thread pool getting the graph
  concurrently: one build, no deadlock; shared graph = per-call graph bytes — green.
- Warm-up: first built as a silent `Jobs` job (`JobOutput::WrapWarm`); that broke round 8's app test
  `a_selected_wrapped_stamp_without_parameterization_gets_it_from_a_param_only_job` (`running_count() == 1` saw the
  warm-up). Asserts unchanged; instead the warm-up is a named background thread outside `Jobs` (no `JobOutput`,
  `io_flows.rs` / `jobs.rs` back to round 8). Deviation from "a silent job": same effect, not counted by `Jobs`.
  It is asked for by the Transform tool for any selected 3D stamp (graph only for wrapped ones; the shared texels for
  both, the T9 line needs them in Project too).
- Shared texels were first built inside the graph's `OnceLock` (so a wrap job would leave the warm-up nothing to do):
  ngons_tiling's first Final waited 4.3 s for them (tiled UVs). Now: built only by the warm-up (graph first), a
  footprint of a whole texture or more is counted everywhere instead of rasterized, a set needing more than 32 M
  pixel tests is counted on a coarser raster (>= 128): ngons_tiling 4,312 -> 0.0 ms, ABeautifulGame ~1.0 s (25.5 %
  shared), high_poly_sphere 130 ms.
- T8: `final_waits_for_wrap_job` also requires `stamp.placement == flight.placement`. New app test
  `a_final_does_not_wait_for_a_wrap_preview_job_of_another_placement` (Final in flight in the pump of the release, two
  jobs running, the Preview lands stale, pixels = synchronous Final); round 8's `a_final_waits_for_its_stamps_wrap_preview_job`
  (same placement: waits and reuses) and the Esc / undo races pass unchanged.

## (5) Shared texels (T9) and the panel lines

- `uv_share.rs`: per texture set a count raster (1,024^2 u8, pixel centres, wrapped), pass 1 counts every footprint
  (centroid pixel for a footprint without a centre), pass 2: a triangle shares when more than half of its centres are
  counted twice or more. Share = 3D area of sharing triangles / UV-mapped area.
- WA39: mirrored halves + two own islands **0.5000** (limit 0.5 +- 0.02); `mirrored_quads` alone 1.0; sphere sample 0;
  `uv_cube_on_floor` 0 — green. (`samples::mirrored_quads` alone is all shared, 1.0: the 0.5 sample is built in the
  test from it plus two quads with their own islands.)
- Panel: the T9 line for any selected 3D stamp with `stamp_shared_texels >= 5 %` (16 x 16 picture points: Wrap via
  `surface_of`, Project via rays along `-out`); "Longer than the way round: P % shown" when `ends_share < 0.95`.
  App test: line shown on mirrored halves (share > 0.9) after the warm-up, none on the sphere, nothing before.
- First sweep: high_poly_sphere showed the line (22.7 % of its surface "shared"): a false positive. Its 1 M triangles
  are smaller than a pixel of a 1,024^2 raster, and a footprint without a pixel centre counted its centroid's pixel,
  which its neighbour also covered. Fixed: a set with small footprints gets a finer raster (about 8 centres per
  footprint, up to 4,096^2), and a footprint without a centre is not counted in pass 1 (it shares only where two other
  footprints cover its centroid). Now (`uv_b\`): high_poly_sphere 0.0 % (1.3 s in the warm-up thread), DamagedHelmet
  86.7 % of the surface (plan M1: 77.8 %; the helmet stamp 100 %: line), ABeautifulGame 23.1 % (the board stamp 20.1 %: line), ngons_tiling
  100 % (its triangles' UVs span more than one tile: real reuse; line), all other models 0 % (first sweep). ends lines:
  torus_knot 43 %, ABeautifulGame 59 %, many_islands 33 %.

## (6) Scenario step 10

`Shot("off")` after the Jump-off Final (`shot_off.png`), a `coverage:` line per setting, `panel lines:` log after the
Jump-on frames, and the assert `covered >= 0.85 x ends_share unless reached_edge, lost_cut <= 10 %` (both settings).
The new assert is red on three models (gate business, numbers equal to A's exe):
- ABeautifulGame Jump off: covered 0.0714 vs 0.85 x ends_share 0.3314 (passed = true; the 40 % picture on a piece).
- many_islands: off covered 0.0177 vs 0.85 x 0.1843; on 0.0333 vs 0.85 x 0.3318.
- Lantern: lost_cut 11.23 % (both) — A's open item.

## (7) Verification

**Tests** (`cargo test --workspace --no-fail-fast`): 1,167 passed / 5 failed / 33 ignored (A: 1,161 / 6 incl. the
flake). Failing = A's set with A's numbers: `wa4_r8` (60 deg ink 1.1177, cells 0.769..1.432), `wa20` (ink 1.1890),
`wa17` / `wa24` (ink 0.915 / 0.9146), `wa35` (side 92.3681 %, max 51). The flake passed. New: WA28 x2, WA38, WA39, 3
app tests. Clippy 0. Release build ok.

**Identity** (`ident\` vs A's r67 runs `..._r9a\target\round9\ident\r67`): `stamp`, `text_warp`, `export_run`,
`roundtrip` ok, `shot.png` 0 differing pixels each; export_run's 7 files byte-identical; `check_exports.py`: PASS.

**wrap_surface**: built-in sample, WaterBottle, high_poly_sphere ok, and with `WTE_SYNC_FINAL_BAKES=1` the same 22 / 24 /
24 logged composite hashes (`syncchk\`).

**Sweep, step 10, 13 models** (`sweep_b` vs `sweep_a` = A's exe): ink, dup, scatter, covered, reached_edge, seams,
rejected **identical on all 26 lines** (graph path = sphere path on the real models too). Round 8 values: see A's
tables (unchanged by B).

**Timings** (release, this machine; `param` = the `wrap:` line's fresh time, `applied` = "Final applied after";
round 8 = today's r8 sweep logs, which ran three models in parallel; A and B alone and sequential, one run each):

| line | round 8 | A (alone) | B (alone) | plan limit |
|---|---|---|---|---|
| T-a high_poly_sphere Final param / applied | 11.0 / 11.1 s | 7.7 / 7.8 s | **7.4 / 8.0 s** (off), 6.8 / 6.9 s (on) | <= 5 s and <= 0.5 x r8 — **not met** |
| T-b ABeautifulGame Jump off | 3.44 s | 3.37 s | **1.72 s** (applied 2.59 s incl. 0.78 s graph build) | <= 1.0 s — **not met** |
| T-b ABeautifulGame Jump on | 20.3 s (parallel) | 42.5 s | **32.3 s** | <= 6 s — **not met** |
| Avocado off / on | 9.6 / 20.8 ms | 36.2 / 63.0 ms | 42.4 / 64.6 ms | — |
| graph build (once) | — | — | ABeautifulGame 784 ms (1.50 M tris, 105 MB), high_poly_sphere 492 ms (1.0 M, 70 MB) | <= 1.0 s — met |

**Where the time goes now** (`wrap phases:` lines, B exe):

| case | collect | front | relax (seams + solve) | owner | candidates |
|---|---|---|---|---|---|
| ABeautifulGame Jump off | 5.1 ms | 22.6 ms | **1,631 ms** | 46 ms | 28,416 |
| ABeautifulGame Jump on | 278 ms | 596 ms | **12,974 ms** | **18,157 ms** | 1,426,489 |
| high_poly_sphere | 80-125 ms | 237-248 ms | **6,272-6,928 ms** | 62 ms | 513-632 k |
| Avocado off / on | 0.1 ms | 0.1-0.2 ms | 0.9-1.2 ms | **41-63 ms** | 158 / 612 |
| WaterBottle | 0.2-0.6 ms | 0.2 ms | 4.3 ms | **42.5 ms** | 1,200-3,317 |
| sphere sample (stamp) | 0.2 ms | 0.2 ms | 4 ms | **37-56 ms** | ~1,100 |

So the collection (round 8's 63-90 % on the chess set) is gone from the bill: on ABeautifulGame Jump off it went from
~1.7 s (A: 3.37 s total, B: 1.72 s with the same relax) to 5 ms. What is left belongs to package A's code: the
relaxation's solve on large laid sets (high_poly_sphere, the chess set) and the owner grid. On small models the owner
grid's fixed cost (~35-60 ms: 1,024 x 512 cells per layer whatever the model) is now nearly the whole parameterization
— the "2-4 x slower than round 8 on small models" of check:A is this (Avocado 9.6 -> 36-42 ms: owner 41 ms).

### Requests to package A's code (not applied; exact proposals)

1. **Owner grid size** (`wrap_relax.rs` `OwnerGrid::build`, `OWNER_MAX_CELLS = 1024`): the grid has `1024` cells on the
   picture's long side whatever the triangle count. Proposal: `let cell = min_cell.max(long / OWNER_MAX_CELLS as f64)
   .max((4.0 * half[0] * half[1] / (OWNER_CELLS_PER_TRI * tris.len().max(1) as f64)).sqrt())` with
   `OWNER_CELLS_PER_TRI = 64.0` (at least ~64 cells per stored triangle in the picture: the owner of a place is decided
   per triangle, so finer cells add nothing until triangles are smaller than cells), and re-measure WA22 / WA24 / WA31 /
   WA37 (rejected, lost_cut, dup) — a quality change: A or check:A must decide. Expected: Avocado 41 -> ~1 ms.
2. **Owner on injective layouts**: an exact seed with no jump and no closure (WA25's cases: rejected 0 by construction)
   could skip `buried` and the grid's pass 2; `lost_cut` still needs pass 1.
3. **Jump on, chess set**: owner 18.2 s — the `buried` ray casts in mixed cells (1.43 M candidates, 30 jumps). Sort
   candidate rays per cell once, or cap `BURY_REACH` rays per cell; needs A's judgement on `rejected` (45.65 %).
4. **Relaxation**: high_poly_sphere 1,440 CG steps over 77 k vertices = 6.3-6.9 s; ABeautifulGame off 1,440 steps =
   1.6 s. Both hit the ARAP budget (12 x 60 x 2). T-a / T-b need either fewer unknowns (T5's exact region pins nothing
   on a sphere) or a better preconditioner (Jacobi now); plan T8 names multigrid / Cholesky as "later".

## Deviations (summary)

1. One line in `Jumper::nearest_other` (binary search instead of the removed hash map).
2. Warm-up is a background thread, not a `Jobs` job (round 8 test counts jobs; asserts unchanged).
3. Phase timers (`phase_times`, four `Instant` pairs) in the body of `wrap_param_from` (A's region; no logic).
4. `WRAP_MAX_TRIS` (3 M) keeps the nearest *grown* candidates, round 8 the nearest in the sphere (never reached in tests).
5. The WA28 bound holds on its case; on a fine floor the sphere is up to 2.6 x the laid band (no tighter bound without
   the spine).
6. Steps (1)-(6) in one commit.

## Open items for check:B

1. Timing lines T-a, T-b (off and on) not met: the remaining time is A's relax / owner (requests above).
2. high_poly_sphere shows the T9 line (22.7 % of its surface shares texels): check the model's UVs (pole fans or a
   real overlap); a false positive would be a raster rule issue in `uv_share.rs`.
3. The three red step-10 asserts (ABeautifulGame off, many_islands, Lantern) are new plan-7.2 asserts, equal in A's exe.
4. The T-c piece spot and T-d (Jump on) are not met (A's relax / owner, table below).
5. Races: the warm-up thread and a wrap job share the cell's `OnceLock` (the job waits); checked by WA38's concurrency
   case only, not by an app-level race test.

## WA15 probe (release, `wa15.log`, alone; graph path vs round 8's sphere path in the same build, both `equal true`)

| spot | graph path (collect / front / relax / owner) | sphere path (collect) | candidates graph / sphere | limit (7.3) |
|---|---|---|---|---|
| graph build, ABeautifulGame (1.50 M tris) | **697 ms** once (105 MB); shared texels 895 ms (first version; now ~970 ms, background) | — | — | T-c <= 1.0 s — met |
| board 20 %, Jump off | **35.8 ms** (0.6 / 0.3 / 0.0 / 34.3) | 321 ms (274) | 1,150 / 403,388 | T-c <= 60 ms — **met** (old laptop 339-386) |
| piece 20 %, Jump off | **166 ms** (1.3 / 5.7 / 121.7 / 30.9) | 869 ms (644) | 7,392 / 568,033 | T-c <= 150 ms — **not met** (relax) |
| board 20 %, Jump on | 4,648 ms (127 / 64 / 1,495 / 2,903) | 4,992 ms (841) | 740,162 / 741,848 | T-d <= max(1.5 x off, 150) — not met (owner, relax) |
| piece 20 %, Jump on | 945 ms (107 / 29 / 456 / 315) | 1,598 ms (732) | 649,960 / 649,960 | T-d — not met |
| board 100 %, Jump off | 680 ms (3.6 / 15 / 611 / 31) | 2,637 ms (1,794) | 21,760 / 1,498,064 | — |
| board 100 %, Jump on | 104.8 s (306 / 2,764 / 45,863 / 54,559) | 110.8 s | whole mesh | — |
| piece 100 %, Jump off / on | 176 ms / 56.2 s | 2,110 ms / 60.2 s | 7,392 / whole mesh | — |
| sphere sample 20 % / 100 % | 35-37 ms (owner 96-97 %) / 81-97 ms (relax 61-68 %) | same | 400-570 / 3,968 | — |
| bumpy ball (WA20) | 135 ms incl. graph (relax 73 %, owner 22 %) | — | — | — |

Phase shares: with the graph the collection is 0-3 % everywhere except Jump on (the closure takes every part of the
board within the gap: 0.74-1.5 M candidates; 3-12 %). The rest is package A's: the owner grid (fixed ~30-35 ms per
layer; seconds with `buried` rays on Jump on) and the relaxation's solve.
