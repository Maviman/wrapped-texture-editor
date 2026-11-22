# Round 8 plan — Wrapped Texture Editor ("wrap: once, whole, smooth — and fast")

Date: 2026-11-21. Planner: Fable. Builders: Opus. Source = the main tree `E:\claude_programs\wrapped_texture_editor`
(rounds 6 + 7 combined; reference exe `target\round7\wte_round6_7.exe`). `ARCHITECTURE.md` is the binding contract;
this plan says which sections change. Read first: `target\round6_design\ROUND6_PLAN.md` sections 2-4 and 11 (the
user's binding answers stay binding), `target\round6\GATE_REPORT.md`, `target\round6\notes\pkg_a.md`.

| Item | What | Package |
|---|---|---|
| R8-1 | Quality: the picture lies once, whole and continuous on curved / bumpy / closed surfaces | A (core) |
| R8-2 | Speed: the patch grows from the anchor; Preview wraps leave the UI thread | A (core part), B (app) |

The user's request (round 6): wrap text / images "physically around an object", "the full image on an uneven surface
without it tearing or dissapearing". Round 6 meets it on flat, cylindrical, conical and folded surfaces and fails it on
bumpy ones (WA4 at 45-60 degrees, high_poly_sphere, klein_bottle, Avocado's seed, torus_knot, Jump on DamagedHelmet /
ABeautifulGame), and is slow on ABeautifulGame. Both are fixed now. **Quality comes first.**

---

## 0. Facts every agent needs

- Build from the root of your copy: `cmd /c "E:\claude_programs\wrapped_texture_editor\cargo.cmd <cmd> --offline 2>&1"`
  (`test --workspace`, `clippy --workspace --all-targets`, `build --release -p wte_app`). Never edit `cargo.cmd` or
  `.cargo\config.toml`. **No new crates** (`glam`, `rayon`, `std` only; there is no sparse solver in `Cargo.lock`: the
  solver below is hand-written, matrix-free). One heavy cargo command at a time; wait when free memory < 1 GB.
- Machine: no GPU (CPU wgpu fallback), 6 cores / 12 threads, 7 GB RAM (often ~1 GB free), slow USB disk; a cold build
  is 20-40 min. Large outputs on E: only. Never scan `target\` recursively. Never kill a running `wrapped_texture_editor.exe`.
- Headless only: `<exe> [PATH] --screenshot <out.png> --scenario <name> [--size 1280x800]` writes `<out>.json` (`"ok"`).
  `WTE_FRAME_LOG=1` frame timing; `WTE_SYNC_FINAL_BAKES=1` synchronous Finals (and, from this round, synchronous wrap
  Previews); `WTE_STAMP_MODE=wrap` new stamps in Wrap.
- Copies: `target\round8\work\base` (main tree without `target\`), `pkg_a` (from base), `pkg_b` (from checked `pkg_a`;
  copy `pkg_a\target` along to save a cold build), `merged`. Notes `target\round8\notes\pkg_a.md`, `pkg_b.md`,
  `gate1.md`, `gate2.md`. Report `target\round8\GATE_REPORT.md`. Exe `target\round8\wte_round8.exe`.
- Tools: round 6 gate 2 `target\round6\runs_gate2\bakes6.py`, `g6table.py`; round 4 `target\round4\review\gate3\`
  (`frames.py`, `wf_steps.py`, `perf_driver.ps1`); round 3 `target\round3_audit\` (`sweep.ps1`, `slow.py`),
  `target\round3\shot_diff.py`, `check_exports.py`. Models: as round 6 section 0 (`target\compat\harness\shapes\`,
  `E:\claude_downloads\glb_samples\`). Core tests use `samples.rs` meshes only; file models in ignored probes and the gate.
- **Do not edit** (round 7's speed work and the Project path): `brush.rs`, `stroke.rs`, `texel_map.rs`, `bvh.rs`,
  `tip_bake.rs`, `stamp_bake.rs`. `raster_tri`, BVH queries and the `stamp_bake` helpers are used as they are.

## 1. What the user gets (plain words)

- **Bumpy and round things**: in Wrap the picture lies on the surface **once, whole and in one piece**, like a slightly
  stretchy sticker pressed on: over a bumpy ball, an avocado seed, a knot, a helmet, a chess piece. Over bumps the
  picture is gently stretched or squeezed (spread over the whole picture, not torn in one place). The picture's centre
  line still lies exactly along the surface (letters keep their size along a line of text).
- **Cut lines only where the shape forces them**: at a sharp corner or cone tip (one line, like a sticker pressed on a
  box corner), where the picture's own edges meet behind an object that is smaller than the picture ("Ends meet",
  now in both directions), and around a handle or a hole. Nowhere else.
- **Jump small gaps** continues only onto parts that lie under the picture, and no part of the picture ever appears
  twice (a piece standing in the picture takes its share of the picture; the board behind it shows the rest).
- **Unchanged**: flat, cylindrical, conical and folded surfaces (exact, as in round 6); Overlap; Project mode pixel for
  pixel; the brush; files (no format change).
- **Heavy models**: dragging, scaling or turning a wrapped picture stays as smooth as in Project mode. The picture
  follows the handles a moment later (the last finished picture stays visible until the new one lands, like round 5's
  Final bakes). Releases land faster.
- The panel says "Fitted to the curved surface: stretched up to P %" and "Cut where it cannot lie flat" when they apply.

## 2. Technique

### 2.1 Why round 6 fails (diagnosis from its code)

1. The front's **order and stop use the unfolded (layout) coordinates** (`SpineKey::key`): on a steep bump the rigid
   unfolding folds back, so surface that is far away *along the surface* lands inside the picture rectangle again and
   is laid too. That is the ink of 4.6-9.1 x: the same picture spots on several surface places.
2. A vertex reached from two sides of a bump takes one side's position ("nearest wins"); the triangles in between are
   then re-laid rigidly one by one (`finish` step 6): the scattered scraps and cut lines (15 % of WA4's triangles).
3. **Nothing forbids two surface places from receiving the same picture point.** With Jump small gaps every part in
   reach is laid on its own (DamagedHelmet 38 jumps, ABeautifulGame 50): the picture appears on every piece.
4. Speed: `wrap_param` collects **every triangle in a sphere** of the picture's reach and welds / sorts them all
   (hundreds of thousands on ABeautifulGame) even when 123 vertices are laid; and the Preview parameterization runs on
   the UI thread for every new spot.

### 2.2 Choice: geodesic-bounded front + seam-aware relaxation (LSCM, then ARAP) + an owner test

The round-6 unfolding stays as the **seed** (it is exact on developable surfaces). Four additions:

1. **Geodesic front** (fixes 2.1-1). The front pops vertices by `g` = their geodesic distance to the spine, computed
   by the fast-marching triangle update (planar unfolding of the triangle; Dijkstra edge update as fallback) from the
   strip seeds (`g` = |across|, exact). It stops at `g > STOP = WRAP_STOP_SLACK x across reach`. A vertex is laid only if
   the surface path to it is short, never because its *unfolded* position happens to land in the picture. The laid set
   is the geodesic tube around the spine. Each vertex also inherits two labels from its producer: `foot` (arc length of
   its spine point) and `side` (+/- of the spine).
2. **Seams** (where the sheet may be cut; one rule each, all deterministic): S1 ends meet (closed spine: feet more than
   half the period apart); S2 halves meet (opposite `side`, both farther than `SEAM_NEAR` from the spine: the sheet's
   top and bottom edges meeting behind an object, behind a hole, or beyond the spine's ends — outside the picture);
   S3 orientation flip or non-manifold edge (Möbius, Klein); S4 cone cut: from an interior vertex with |angle deficit|
   > `CONE_CUT` = 45 degrees (cube corner, cone tip) along steepest `g` ascent to the laid set's boundary; S5 topology:
   the tree-cotree remnant (dual spanning tree grown in `g` order from the anchor triangle, primal leaves pruned unless
   on the boundary, a seam or a cone) — it cuts only what S1-S4 left non-disk (a handle).
3. **Relaxation** (fixes 2.1-2). Unknowns = one 2D position per *wedge* (a vertex's fan of laid triangles between seam
   edges); so the layout is continuous everywhere except on seams, **by construction**. Pins: the strip seeds (the
   centre line stays exact) and each jumped part's landing triangle. First a **LSCM** solve (conformal least squares
   with the pins: smooth, no dependence on the crumpled seed, no collapse), then up to `RELAX_ARAP_ITERS` **ARAP**
   local/global iterations (per-triangle closest rotation by 2x2 polar decomposition; global step = the area-weighted
   Laplacian with the pins). Both solves: matrix-free conjugate gradients with a Jacobi preconditioner, warm-started,
   fixed iteration caps and a total budget. **Early-out**: when the seed is already an isometry where the picture lies
   (every non-seam triangle meeting the picture rectangle has all edge ratios within 1 ± `ISO_TOL` and no tear other
   than an S1 / S2 closure), the seed is kept as it is — round 6's exact result on flat, cylinder, cone, folds, box
   edges, WA16 parts and Overlap turns, with no solver cost.
4. **Owner test** (fixes 2.1-3, and guards 3 when relaxation is imperfect). Per layer an owner grid over the picture
   rectangle (at most `OWNER_MAX_CELLS` on the long side, cell >= one source pixel) stores the smallest `g` of the laid
   triangles covering each cell. In `bake_wrapped` a fragment at picture point `l` with interpolated depth `g_f` is kept
   only if `g_f <= g_min(cell) + OWNER_TOL x (cell diagonal + longest 3D edge of its triangle + of the cell's owner)`.
   Each picture point therefore goes to the surface place nearest the centre line along the surface — the sticker
   rule — and to no other place farther than the tolerance. On an injective layout the test rejects nothing (tested
   byte for byte). `surface_of` / the outline / text-path handles pick the lowest-`g` triangle too.
5. **Jump small gaps bound**: a front jump (and a spine jump) is taken only from a vertex whose layout position lies
   inside the picture rectangle grown by its border, and only if the landing point does too ("only where the picture's
   own footprint lies"). The owner test then keeps each picture point once across all parts.

### 2.3 Alternatives

| Option | For | Against | Verdict |
|---|---|---|---|
| **Chosen: geodesic front + seams + LSCM/ARAP + owner test** | Continuity by construction; "once" by construction; exact developable path kept (early-out); distortion spread smoothly; cuts only where topology / cones force them | A solver (hand-written CG, ~400 lines); cost grows with the laid area (tens of ms per 10k vertices) | Only option meeting all three of "once, whole, smooth" |
| Fermi / geodesic coordinates only (fast marching, no solver) | Cheap, injective near the spine | Behind every steep bump the distance field has a cut locus: a cut line behind each bump (the "torn" look stays) | No: that is what the user complained about |
| Heat-method geodesics | Smoother distances | Needs two sparse solves per parameterization; fast marching is enough for ordering and depth | No |
| LSCM alone | One solve | Conformal: cells shrink / grow with the curvature (area off by 30-60 % on a ball) | Used as the start of ARAP only |
| Tutte / harmonic start | Provably injective | Needs a fixed convex boundary; the tube's boundary is not natural; heavy edge distortion | No |
| Rejecting disagreeing vertices / coverage bounds only | Small | Leaves holes and scraps; the picture still appears in pieces | Part of the design only as the owner test (a guard) |
| Incremental parameterization during a drag (move the old one) | Fast | Not exact (the result depends on the anchor), reused != fresh bytes | No; Previews go to a worker instead |

### 2.4 What is guaranteed (numbers)

| Property | Guarantee | How |
|---|---|---|
| Developable surfaces (WA1, WA3, WA5 edge, WA16, WA18) | Exact as round 6: 99.9 % of texels within 2/255 of the analytic unrolling, max 8/255 (round 6 measured 100 % / max 1) | Early-out keeps the seed; owner test rejects 0 fragments there (asserted) |
| Continuity | One coordinate per wedge; a discontinuity only on S1-S5 seam edges; 0 seam edges inside the picture on WA4, the bumpy ball, the sphere sample | Wedge unknowns; tests count seam edges inside the picture |
| Once | No picture point on two surface places farther apart than the owner tolerance (~2 triangle edges + 2 cells), except Overlap layers (intended) | Owner test; `dup` metric <= 1 % in core tests, <= 3 % in the sweep |
| Bounded reach | Only surface within geodesic distance `STOP` of the centre line is laid; Jump enters only parts under the picture's footprint | Geodesic front; footprint rule |
| Ink | ink = mean area distortion of the relaxed layout over the picture; WA4 0.90-1.10; sweep <= 1.3 | ARAP + owner test (measured, not a hard bound) |
| Determinism | Byte-identical with 1 and N threads, repeat runs; reused = fresh | Serial or fixed-chunk kernels (8,192 rows; partial sums added in chunk order); gather per wedge in fixed order |
| Project, brush | Byte-identical to `wte_round6_7.exe` | Untouched code paths |

Not guaranteed: an injective layout (ARAP can fold locally near cones or on extreme bumps; the owner test makes the
fold harmless and `flipped` is reported); a distortion bound per cell (Gauss-Bonnet: a 60-degree bump carries about
pi of curvature that no flat sticker can absorb without stretch — WA4's cell limits are the measured target).

### 2.5 Speed technique

1. **Welded mesh graph, once per mesh** (`wrap_mesh.rs`): bit-exact position weld + the global boundary snap
   (`WELD_SNAP`), welded vertex -> triangles (CSR), half-edge groups, part ids and part boxes, position rank. Built lazily
   (`OnceLock`) by the first job that needs it, serially (no rayon inside the `OnceLock` init: the brush index's deadlock
   note in `brush.rs`), shared by `Arc` with jobs; dropped with the mesh. Expected on ABeautifulGame: ~0.3-0.8 s once,
   ~50 MB.
2. **Growth from the anchor**: the anchor triangle from a small growing BVH query at `origin` (radius 1e-3 diag x 4^k up
   to the reach); the spine and the front walk the welded graph; local indices are given on first touch. The cost
   follows the laid vertices (+ one halo ring), not the reach sphere. The same tie rules (position rank) keep the result
   independent of the visiting order.
3. **Previews off the UI thread** (B with A's API): one wrap Preview job in flight, latest wins; the stamp shows its
   last finished picture until the new one lands; a Final waits for its stamp's Preview job in flight and reuses its
   parameterization when it serves (no second build).
4. Relaxation kernels run in parallel (fixed chunks) from `PAR_MIN_UNKNOWNS` up. Total CG work is capped per
   component (deterministic: iteration counts, never time).

Expected: ABeautifulGame 20 % decal on the board (flat: early-out) ~2-10 ms; on a chess piece (~14k laid vertices,
relaxed) ~60-150 ms; high_poly_sphere at 40 % ~2-5 s (on the worker; the UI stays free). The user accepts CPU limits.

---

## 3. Package A — core (`wte_core`)

### 3.1 Constants (`wrap_param.rs` / `wrap_relax.rs`, public where tests need them)

`WRAP_STOP_SLACK = 1.3` (front stops at 1.3 x the bucketed across reach: relaxed layouts still cover the picture);
`ISO_TOL = 2e-4`; `CONE_CUT = 45 deg`; `SEAM_NEAR = 3` (x the longest incident edge); `OWNER_MAX_CELLS = 1024`;
`OWNER_TOL = 2.0`; `RELAX_ARAP_ITERS = 6` (stop earlier when the energy drops < 0.5 %); CG: LSCM rel. residual 1e-6,
<= 600 iterations; ARAP rel. 1e-5, <= 200 per solve; `RELAX_CG_BUDGET = 2,500` iterations per component and layer;
`PAR_MIN_UNKNOWNS = 16,384`, `PAR_CHUNK = 8,192`; cotangents clamped to |cot| <= 1e3; triangles with 3D area <
1e-12 diag^2 left out of the energies (their wedges keep the seed position when they have no other triangle). The round-6
constants stay.

### 3.2 Algorithm (per turn; `wrap_param`), all f64, deterministic

1. **Reach, anchor, spine**: as round 6 (4.3 steps 1-3, bucketed reach, anchor rule, frame, traces, Jump spine jumps,
   Overlap turns), but on the welded graph (2.5) instead of a candidate sphere. A spine jump additionally needs its
   arc length `s` <= the picture's half length + border (footprint rule).
2. **Front** (round 6 step 4 with these changes): heap key = `g` (2.2-1), stop `g > STOP`; candidate positions,
   consensus, `one_front`, never-fold and fallback rules unchanged; each finalized vertex stores `g`, `foot`, `side`
   (seeds: strip `s`, sign of the exact across, 0 when |across| <= 1e-9 diag; others: from the winning candidate's
   `from`; a neutral producer gives the sign of the vertex's own across). Jumps: footprint rule (2.2-5), landing `g =
   g_v + |q - p_v|`, labels from `v`.
3. **Halo and tears**: as round 6 steps 5-6 (this is the seed result). Record per triangle: torn or not, and whether a
   tear is a closure tear (its corners differ by an S1 / S2 rule).
4. **Early-out test** (2.2-3). If it holds: store the seed (go to 7). `WrapStats::relaxed = false`.
5. **Seams and wedges**: S1-S5 over the complete laid triangles (all three corners final, not degenerate). Wedges by
   union over the uncut edges around each vertex. Components = triangle sets connected over uncut edges.
6. **Relax** each component in entry order (spine component(s) first, then jumped parts in jump order): a jumped part's
   landing triangle is first re-placed from its source vertex's *relaxed* position and the rotation of the source's
   relaxed producing triangle, then pinned. A component without a pin is pinned by its lowest-`g` triangle (seed). LSCM,
   then ARAP (3.1). Halo triangles get their missing corners from the relaxed corners by the round-6 hinge / frame rule.
   Count `flipped` (relaxed triangles with reversed orientation inside the picture).
7. **Store**: per layer the triangles, f32 corners, f32 corner `g` (new), the owner grid (built here, so a reused
   parameterization carries it), stats. `covered` = rectangle cells with an owner / all cells (the developable path keeps
   round 6's area measure; both are fine for the tests' limits).

Overlap: steps 2-7 run per turn as in round 6 (each turn single-valued, its own seams, relaxation and owner grid; the
layers are composited as before). Across the spine there is no Overlap: the halves always meet (S2).

`WrapStats` gets `relaxed: bool`, `seams: u32` (seam edges inside the picture), `flipped: u32`, `stretch: f32` (95th
percentile of |area ratio - 1| of the triangles inside the picture), `rejected: f32` (share of laid picture area the
owner test drops), `cg_iters: u32`. `cut_tris` = seed path: torn triangles inside the picture (as today); relaxed path:
triangles with a seam edge inside the picture. The `wrap:` log line adds `relaxed`, `seams`, `stretch`, `rejected`,
`cg_iters`.

### 3.3 Measuring aid (public; used by tests, the scenario and the gate)

`Document::stamp_wrap_quality(id) -> Option<WrapQuality { ink_area: f32, dup: f32, scatter: f32, bins: u32 }>`: over
the stamp's inked texels (alpha >= 128) take the surface point (texel map `locate`) and its picture point
(`local_of`, top layer); bin picture points in cells of `c = 2 x max(source pixel in world units, texel_world)`.
`dup` = share of inked bins whose 3D points spread over more than `6c` (the same picture spot in two places); `scatter` =
share of 4-neighbour inked bin pairs whose 3D centroids lie more than `6c` apart (scraps; seams add a thin line).
**First step of the build**: measure round 6's behaviour with it (WA4 60 deg, the bumpy ball, the torus, the closed
body) and record the baseline in the notes (it must show the failure: dup >> 3 %).

### 3.4 Preview and param-only jobs (API for package B)

- `StampBakeJob` gets a kind: `Full` (today), `Preview`, `ParamOnly`, plus `Arc<WrapMeshCache>`. `run()` dispatches as
  today; `Preview` = parameterization (cached when it serves) + `bake_wrapped(.., Preview)`; `ParamOnly` = the
  parameterization only.
- `Document::set_defer_wrap_previews(on)` (default off): while on, `bake_stale_previews` leaves **wrapped** stamps in
  `preview_stale` (projected stamps unchanged: Project Previews stay on the UI thread, byte- and time-identical).
- `Document::next_wrap_preview_job() -> Option<StampBakeJob>` (lowest stale wrapped id; takes it out of
  `preview_stale`; records the pending-edit serial and the placement it was made for).
- `Document::apply_wrap_preview(out) -> WrapPreview { Applied { due_again: bool }, Stale }`: applied when the stamp
  exists, is in the same open pending edit (`preview_baked`, same serial), and the mesh and maps are the job's `Arc`s —
  **also when the placement moved since** (the last good picture); then the stamp is put back in `preview_stale` when
  its placement differs from the job's (`due_again`). The parameterization is cached when it serves the current
  placement. Esc, undo, redo, a Final (flush) or another document make an output `Stale`.
- `Document::wrap_param_job(id) -> Option<StampBakeJob>` (ParamOnly) and `apply_wrap_param(out)` (caches it when it
  serves; no pixels change) — the off-thread form of `ensure_wrap_param` (which stays for tests and the sync mode).
- `Document::wrap_mesh_prep() -> Option<WrapMeshPrep>` (`Send`; `run()` builds the welded graph if not built).
- Rule: synchronous mode (`set_defer_wrap_previews(false)`) gives the same Preview bytes as the job path (tested).

### 3.5 Tests package A adds (headless, in `wrap_tests.rs` / in-file; numbers are the acceptance)

Measured with the `cells` picture unless said; `dup`, `scatter` from 3.3; "seams" = `WrapStats::seams`.

| # | Test | Limit |
|---|---|---|
| WA4 | `bumpy_plate_periods(64, a, 2)` at 30, 45 and **60 deg**, picture 0.6 of the plate (round 6's setup) | ink 0.90-1.10; every cell 0.75-1.30; smear <= 1 %; covered >= 0.999; dup <= 1 %; scatter <= 2 %; seams 0; relaxed |
| WA4b | 4 periods at 60 deg (a bump spans ~1.7 picture cells, so a cell follows one bump's stretch instead of averaging) | ink 0.85-1.15; cells 0.65-1.50; smear <= 1 %; covered >= 0.999; dup <= 1 %; scatter <= 2 %; seams 0 |
| WA20 | New `samples::bumpy_sphere(subdiv 5, 24 bumps, slope 60 deg)` (the high_poly_sphere case, small), picture 40 % of the diagonal | ink 0.85-1.15; cells 0.70-1.40; dup <= 1 %; scatter <= 2 %; seams 0; covered >= 0.99 |
| WA21 | New `samples::lathe_piece()` (closed, pawn-like surface of revolution), picture 2 x its height: covered once with seams where the edges meet | inked share of the piece's area >= 0.95; dup <= 1 %; seams only S1 / S2 (counted per rule); flipped <= 1 % |
| WA22 | Torus sample, picture taller than the tube's girth; then larger than the whole torus (handle: S5 must cut) | dup <= 1 %; scatter <= 3 %; no panic; deterministic |
| WA23 | Cone tip (cone sample, picture centred on the tip) and `seam_cube` corner | one cut path from the tip / corner (S4); smear <= 0.5 %; covered >= 0.70; the round-6 WA5 corner limits hold |
| WA24 | Jump bounded: `uv_cube_on_floor` (WA17) and a new `samples::cubes_on_floor(5 x 5)` with the picture over 4 cubes | WA17: cube side receives, ink 0.95-1.10 (was 1.55), dup <= 1 %; grid: parts entered <= parts under the footprint, ink <= 1.15, dup <= 2 % |
| WA25 | Owner test inert where the layout is injective: WA1 (both cases), WA3, WA5 edge, WA16, WA18 | 0 fragments rejected; `relaxed == false`; their round-6 limits hold unchanged |
| WA26 | Determinism with relaxation and jumps: WA20, WA22, WA24 at 1 / 2 / 5 threads, Preview and Full, two runs | byte-identical |
| WA27 | Reuse after relaxation (WA11 on WA20): scale inside the bucket, tint, flip reuse; reused = fresh | byte-identical |
| WA28 | Growth: the parameterization touches <= 1.25 x (laid + halo) triangles; WA1-WA25 results unchanged by it | as stated |
| WA29 | Preview / param-only jobs (3.4): job Preview = synchronous Preview bytes; moved-since output applied with `due_again`; Esc, undo, Final, another document -> `Stale`; param-only caches without pixel change | as stated |
| WA30 | Old tests: every round-6 / round-7 test passes; only these expectations change, each listed in the notes: WA4 (now asserts 60 deg), WA17's ink (upper bound added), stats fields added | as stated |
| WA15 | Ignored probe (release, gate runs it): adds the piece spot, relaxed timing, the weld build time, `bumpy_sphere`, Jump at both spots | reported; limits G-lines 6.2 |

WA7 (bad meshes, <= 200 ms each) also runs the relaxation; if a relaxed case exceeds 200 ms in the debug test profile,
the builder measures it in release (limit 200 ms there) and notes it.

### 3.6 Files (package A)

New `wrap_relax.rs` (seams, wedges, LSCM / ARAP, CG kernels) and `wrap_mesh.rs` (welded graph cache). Edited:
`wrap_param.rs`, `wrap_bake.rs` (owner test in `WrapCtx::fragment`, depth per job), `stamp_job.rs` (kinds, cache Arc,
log line), `doc_stamps.rs` (3.3, 3.4, the cache plumbing), `document.rs` / `doc_build.rs` (`BakeSets` gets the
`WrapMeshCache`, replaced when the mesh `Arc` changes), `lib.rs`, `samples.rs` (`bumpy_sphere`, `lathe_piece`,
`cubes_on_floor`, `torus`, `cone` — `samples.rs` has no torus or cone today; check:A's test-local builders in
`wrap_tests.rs` may be moved there), `wrap_tests.rs`. `ARCHITECTURE.md`: 1.6 (round-8 paragraph), 2.1 (files),
5.12 (`WrapStats`, `WrapQuality`), 5.14 (`BakeSets`), 5.20 (job kinds, the Document functions of 3.3-3.4), 6.5a
(rewritten steps 2-7, owner test), 9.1 (tests). No app file; the app compiles unchanged (new fields only; it ignores them).

**Work order** (green tests between steps, notes after each; a context limit leaves usable work):
(1) 3.3 metric + baseline; (2) geodesic front, labels, footprint rule (all round-6 tests green); (3) owner grid + bake
test + `surface_of` (WA25); (4) seams, wedges, LSCM / ARAP, early-out, jumped-part re-landing (WA4, WA4b, WA20-WA24,
WA26-WA27); (5) 3.4 jobs (WA29) — **package B needs this**; (6) welded graph + growth (WA28, WA15); (7) ARCHITECTURE.md.

**Done means (A):** 3.5 passes (WA4 at 60 deg within limits, or the measured values recorded and flagged — never a
silently weakened assert); `cargo test --workspace` 0 failed, clippy 0 warnings; release build; the seven round-5
scenarios (`stamp`, `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run` + `check_exports.py`,
`final_bake_async`) `shot.png` identical to `wte_round6_7.exe` (toast area excepted); `wrap_surface` ok on the built-in
sample; WA15 run once and recorded.

---

## 4. Package B — app (`wte_app`), built on A's checked copy

### 4.1 Changes

- **`stamp_bakes.rs`**: a second slot `WrapPreviews { in_flight: Option<..>, held }` next to the Final slot. `pump`:
  when no wrap Preview job is in flight, take `doc.next_wrap_preview_job()` and spawn it (`Jobs::spawn_staged`, silent:
  no status-bar title; `run_isolated`); a Final job for a stamp whose Preview job is in flight starts after that job
  lands. `on_wrap_preview_output` -> `apply_wrap_preview` (generation check as Finals; held during a stroke like
  Finals); `due_again` asks for the next frame. `settle` never waits for Preview jobs (their outputs go stale).
- **`jobs.rs`**: `JobOutput::WrapPreview { generation, output }` (and `WrapParam` for param-only); routing in
  `io_flows::on_job_output` / `app.rs`.
- **`app.rs` / `state.rs`**: `set_defer_wrap_previews(true)` exactly when async Final bakes are on
  (`WTE_SYNC_FINAL_BAKES=1` keeps Previews synchronous: the comparison mode of the gate).
- **`tools\transform.rs`**: the selected wrapped stamp without a parameterization (after open / undo) asks for a
  param-only job instead of `ensure_wrap_param` on the UI thread; outline and stats appear when it lands ("Wrapping..."
  meanwhile, as during a drag). A warm-up job (`wrap_mesh_prep`) starts once per mesh when the Stamp / Text tool mode
  becomes Wrap, a wrapped stamp is selected, or a document with wrapped stamps is installed.
- **`panels\tool_options.rs`** (+ `text_block.rs` only if the stats show there): under "Wrapped over N triangles":
  "Fitted to the curved surface: stretched up to P %" when `relaxed` and `stretch >= 5 %`; "Cut where it cannot lie
  flat (a corner, a handle, or where its edges meet)" when `seams > 0`; existing warnings unchanged.
- **`selftest\scenarios.rs`** (+ `selftest\mod.rs`: `BakeWait` also waits for wrap Preview / param jobs):
  `wrap_surface` new steps — 2b (sphere sample, Wrap): `relaxed`, `seams == 0`, `dup <= 1 %`, `scatter <= 2 %`,
  logged; 6b (drag in async mode): no `wrap: ... param` line from the UI thread during the drag (the Preview lines name
  the job), the last Preview lands, the Final equals the synchronous run's composite; 10 (PATH): log `WrapStats`,
  `WrapQuality`, ink ratio, param and applied times for Jump off and on, then **assert ink <= 1.3, dup <= 3 %,
  scatter <= 5 %** (Ends meet). Scenario count unchanged.
- `ARCHITECTURE.md` 8.2 (env rule), 8.6 (Transform), 8.7 (panel lines), 8.9 (jobs), 8.11 (`wrap_surface` steps), 9.2.

### 4.2 Tests package B adds

A drag of 10 Preview moves on a wrapped stamp in async mode: at most one job in flight, no parameterization built on
the UI thread (core counter), the last applied Preview matches the last placement, the Final equals the synchronous
pixels; Esc / undo while a Preview job runs leaves the old pixels; a Preview output of another document is dropped; the
param-only job fills stats and outline without pixel change; the panel lines appear for a relaxed stamp; Project stamps
still bake Previews on the UI thread (unchanged).

**Done means (B):** 4.2 pass; workspace tests 0 failed, clippy 0 warnings; release build; `wrap_surface` ok on the
built-in sample and with `WaterBottle.glb`, `high_poly_sphere.obj`, and equal composites with `WTE_SYNC_FINAL_BAKES=1`;
the seven round-5 scenarios identical to `wte_round6_7.exe`; `wrap_surface`'s `shot.png` read and described in
`notes\pkg_b.md`.

---

## 5. Format and compatibility

No format change (`FORMAT_VERSION` stays 3, no new key): the parameterization is not saved, the baked caches are.
A round-6 project opens in round 8 with **identical pixels** (saved caches) until a wrapped stamp is re-baked (move,
scale, turn, tint, option, text edit, image swap, toggle). Then the round-8 wrap applies: on developable surfaces the
same picture (within 2/255, WA1 limits); on curved surfaces the improved one. After opening, the outline / stats come
from a round-8 parameterization and may not match old pixels on a curved surface until that stamp is re-baked. A
round-8 file opens in round 6 (same format, saved pixels); re-baked there it gets round 6's wrap. Acceptable: no data
is lost and nothing changes without an edit (question Q5).

## 6. Process and ownership

| Order | Agent | Works in | Output |
|---|---|---|---|
| 1 | build:A (Opus) | `work\pkg_a` | code, tests, `notes\pkg_a.md` (plan numbers next to measured ones) |
| 2 | check:A (Opus): diff vs 2-3; break it (a mug-like handle, a cone with a spine through the tip, a closed piece with Jump, Overlap on a bumpy band, slivers in the relaxation, undo while a Preview job runs, 1 vs 12 threads); fix; rerun tests + clippy | `work\pkg_a` | notes |
| 3 | build:B (Opus) | `work\pkg_b` = checked `pkg_a` | code, scenario, `notes\pkg_b.md` |
| 4 | check:B (Opus): diff vs 4; job races (Esc / undo / Final / open during a Preview job, a stroke during one); fixes | `work\pkg_b` | notes |
| 5 | gate 1 | `work\merged` | `gate1.md`, exe |
| 6 | gate 2 | runs under `target\round8\runs_gate*` | `gate2.md`, `GATE_REPORT.md` |

A owns the core and ARCHITECTURE 1.6 / 2.1 / 5.x / 6.5a / 9.1; B owns the app and 8.x / 9.2. B does not edit core
files: a needed core change is a request with exact code in `notes\pkg_b.md`; check:B or the gate applies it and reruns
the core tests. Before each agent: check weekly usage (87 % at planning; resets **2026-11-23 01:00 UTC**). If it would
pass ~95 % before build:B or a gate, **pause until the reset** rather than cutting the quality work.

## 7. Gate

### 7.1 Gate 1 (merge, tests, scenarios, identity, sweep)

1. `merged` = `pkg_b`; three-way diff against `base`: A's files by A, B's by B (+ applied requests). `cargo test
   --workspace` 0 failed (count >= base's; record it), clippy 0, no `todo!` / `unimplemented!`, release build ->
   `target\round8\wte_round8.exe`, SHA-256.
2. All scenarios at 1280 x 800 (`sweep.ps1`): every non-wrap scenario's `shot.png` identical to
   `wte_round6_7.exe`'s (round 7's known exceptions only: toast stack timing in `final_bake_async` / `workflow_path`,
   the export dialog's folder text). This includes **every brush scenario** (round 7 unchanged) and the **Project**
   identity: `stamp`, `text_warp`, `text_path`, `text_styles`, `roundtrip`, `export_run` (exports byte-identical,
   `check_exports.py` PASS), `final_bake_async`; `wrap_surface` step 1's Project composite hash equal in both exes.
   `wrap_surface` and `final_bake_async` again with `WTE_SYNC_FINAL_BAKES=1`: composites equal.
3. **Sweep** (G6): `wrap_surface` with PATH on the 13 models of round 6 (WaterBottle, Avocado, DamagedHelmet, Lantern,
   ABeautifulGame, ngons_tiling, torus_knot, mobius, klein_bottle, nonmanifold, many_islands, flat_annulus,
   high_poly_sphere), Jump off and on. Table: verts / tris / relaxed / seams / stretch / rejected / covered / ink / dup /
   scatter / jumps / param / applied. Limits: **ink <= 1.3, dup <= 3 %, scatter <= 5 % on every model, Jump off and
   on.** **Read the screenshots** of high_poly_sphere, klein_bottle, Avocado, DamagedHelmet (Jump on), ABeautifulGame
   (Jump on), torus_knot, WaterBottle and describe each: one whole picture — each cell once, rows continuous, cut lines
   only at a corner / cone / handle / where edges meet. Put round 6's and round 8's shots side by side in the report.
4. **Compatibility**: a project with wrapped stamps saved by `wte_round6_7.exe` (sphere sample + WaterBottle) opens in
   round 8 with an identical composite; after a tint change, the WaterBottle stamp is within 2/255 on >= 99.9 % of its
   texels, the sphere stamp differs (expected, report how).

### 7.2 Gate 2 (measurements; idle machine, release, `WTE_FRAME_LOG=1`, 2 runs per line, P and W alternating)

| # | Line | Limit |
|---|---|---|
| G5a | Sphere sample `stamp` | W Final applied <= 1.5 x P and <= 80 ms (second run); Preview landing <= 50 ms after the move |
| G5b | ngons_tiling `workflow_path` | as round 6 (applied <= 1.5 x P; 0 frames > 300 ms; drag frame <= P + max(30 ms, 25 %)) |
| G5c | ABeautifulGame `workflow_path` | **drag frame max <= P's + max(30 ms, 25 %)**; **Finals (image, text, move, scale, rotate) applied <= 1.5 x P** each; 0 frames > 300 ms started..applied; image worker <= 2.0 s; Preview landing time reported |
| G5d | ABeautifulGame `text_warp` in Wrap | 0 frames > 300 ms; swap <= 1.5 x P |
| G5e | WA15 probe | **20 % decal param <= 150 ms at the board and the piece spot** (fresh, welded graph built; its one-off build reported, <= 1.0 s); 100 % <= 3 s; Wrap Full <= 1.5 x projected Full |
| G5f | Jump small gaps (probe) | **param <= max(1.5 x Jump off, 150 ms)** at both spots (a ratio alone is meaningless when Jump off lays 123 flat vertices and Jump on adds whole pieces); Overlap 2.5 turns as round 6 |
| G6 | Sweep times (from 7.1.3) | high_poly_sphere Final applied <= 6 s (reported); others reported |
| G7 | ABeautifulGame `workflow_path` peak working set | W <= P + 300 MB |
| G8 | Fuzz seed 7, 1,000 steps, ngons_tiling, Wrap | 0 errors; central / dispatch frames > 500 ms <= 2 |

If a speed line fails, report the shares (`wrap:` lines: front, seams, relaxation, owner, bake) and do not change
product code in the gate unless the fix is small and contained (round-5 rule). Quality lines are never traded for speed.

### 7.3 Report

Verdict in plain words; exe SHA, test counts; every WA and G line with its number and source; the sweep table and the
screenshot readings (round 6 vs round 8); what was not met and why; behaviour changes; to try by hand (a picture over
Avocado's seed; a text across high_poly_sphere; Jump on across chess pieces; a box corner; a drag on ABeautifulGame;
reopen a round-6 project and nudge a stamp).

## 8. Acceptance summary

1. **Quality**: WA4 at 60 deg within ink 0.90-1.10, cells 0.75-1.30, smear <= 1 %, covered >= 0.999 (and WA4b's argued
   limits); WA20-WA24; sweep ink <= 1.3, dup <= 3 %, scatter <= 5 % on all 13 models, Jump off and on; the 5 failure
   screenshots show one whole picture.
2. **Exact stays exact**: WA1, WA3, WA5, WA16, WA18 within their round-6 limits; WA25 owner test inert there.
3. **Unchanged**: Project byte-identical; round-7 brush shots identical; all old tests pass; no format change.
4. **Deterministic**: WA26, WA27, WA9.
5. **Speed**: G5a-G5f, G7, G8.

## 9. Risks

| Risk | Mitigation |
|---|---|
| WA4 at 60 deg misses the cell limits (Gauss-Bonnet: ~pi of curvature per bump must become stretch) | LSCM start + ARAP; record measured values; the gate reports, never a silent weaker assert; WA4b separates the harder 4-period case |
| Relaxation too slow on large laid areas (ABeautifulGame pieces, high_poly_sphere) | Early-out on developable parts; parallel fixed-chunk kernels; CG budget; Previews on a worker; Final reuses the Preview's parameterization; G5e reports the shares |
| Spurious seams (S2 beyond the spine's ends, behind a hole) | S2 needs both ends far from the spine; only outside the picture or where topology needs a cut anyway; tests count seams inside the picture |
| Owner test drops wanted pixels (tie lines between a piece and the board, contact lines) | Tolerance in edges and cells; WA25 (0 rejected where injective), WA24 (cube side receives) |
| ARAP folds near cones / extreme bumps | Owner test keeps them single; `flipped` reported; WA21 <= 1 % |
| Welded graph memory / first-use time on ABeautifulGame (~50 MB, ~0.5 s) | Lazy, serial, on a worker; warm-up job; G5e reports it; G7 |
| Preview job races (stale outputs, Esc, undo, Final, stroke, new document) | Core apply rules (3.4) with a pending-edit serial; WA29; 4.2; check:B |
| CPU contention between a Preview job and software rendering | One job at a time; G5c measures frames |
| A is large for one builder | Work order with green states; metric first; B needs only step 5 |
| Round-7 / Project regressions | Do-not-edit list (section 0); gate 1 identity runs |

## 10. Cut order (if usage runs short; first cut first; never cut quality silently)

1. check:B folded into gate 1 (gate 1 reads B's diff and runs 4.2's races as scenario steps).
2. Gate 2 reduced: 1 run per line; sweep times only from gate 1.
3. The warm-up job (the first wrap on a heavy model then pays the welded-graph build in its first job).
4. The param-only job (after open / undo the parameterization is built on the UI thread, as round 6).
5. Package B's Preview jobs (drags on heavy models stay slow; G5c reported as not met). Package B then only does the
   panel lines and scenario steps.
6. A step 6, growth from the anchor (speed lines G5e / G5f reported as not met).

Never cut: A steps 1-4 (metric, geodesic front, owner test, seams + relaxation + early-out), WA4 / WA20-WA25, the sweep
with its screenshot reading, Project and brush identity, no format change. If A steps 1-4 cannot be finished, the round
pauses at the usage reset and resumes; it does not ship a partial quality fix as done.

## 11. Questions for the user (the build goes ahead with the defaults)

- **Q1 Where the picture is larger than the object** (a picture bigger than a chess piece, taller than a tube): the
  edges meet in a cut line at the back (default, like "Ends meet" round 6 already does along the line), or stop and
  leave the back uncovered?
- **Q2 Centre line exact or balanced**: keep the picture's centre line exactly along the surface (default; letters keep
  their size along a line, the stretch goes to the edges), or balance the stretch over the whole picture (centre line
  may stretch a few %)?
- **Q3 Corners**: cut at corners and tips sharper than 45 degrees (default: cube corner, cone tip; gentler bumps are
  stretched smoothly), or never cut (always stretch, even over a box corner)?
- **Q4 Previews on heavy models**: show the last finished picture while dragging and let it catch up (default, smooth
  frames), or keep the picture glued to the handles at the cost of stutters (round 6)?
- **Q5 Old projects**: keep saved pixels of round-6 wrapped stamps until they are edited (default, no surprise
  changes), or re-wrap every wrapped stamp on open with the new method (pixels change at once)?

## 12. User answers (2026-11-21, binding)

- Q1 picture larger than the object: the edges meet in one cut line at the back (default).
- Q2 stretch: keep the centre line exactly true; stretch goes to the top and bottom (default).
- Q3 corners and tips sharper than 45 degrees: cut there (default).
- Q4 heavy models while dragging: stay smooth, show the last finished picture and catch up (default).
- Q5 not asked: keep round-6 wrapped stamps' saved pixels until they are edited (default).
