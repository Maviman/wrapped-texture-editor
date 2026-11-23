# Round 9 gate — skeptic notes, lens: code review (verify_code)

Date 2026-11-23. Read `git diff --ignore-cr-at-eol round8-wip..round9-wip -- crates` (saved as `full.diff` here), the gate
report and `docs/notes/round9/gate.md`. No cargo run, no tracked file touched. Exe used: `target\round9\gate\bin\wte_r9.exe`
(SHA-256 4F9E14222A17C9E1..., checked) and `wte_r8.exe` for comparison, one process at a time (two `wte_r9.exe` runs of
another skeptic, pids 70216 / 47624, were running at the same time; not touched). Times below are therefore not clean
timings, but the ratios quoted are large.

Folders: `runs2\` = my runs (this session). `runs\`, `models\parts*`, `models\noisy*`, `models\uv_sliver*`, `gen.py`,
`gen_sliver.py`, `diff.patch`, `wrap_*.diff` were already here (an earlier, interrupted run of this role, 2026-11-22
~19:06 local); I re-ran the ones I rely on (`noisy400`) and only quote the others as supporting data, marked "(earlier run)".
My generators: `py\gen_tiles.py`, the inline scripts that wrote `models\uv_long.obj` and `models\trim_strip.obj`;
`py\uvshare.py` = an independent UV-overlap counter for GLBs.

## New findings

### F1 (major, shown) — Jump on: the part closure in `collect_grown` is quadratic in the number of parts
`crates/wte_core/src/wrap_param.rs:2678-2686`: for every part taken, a loop over **all** in-sphere part boxes
(`for j in 0..boxes.len()`), so P parts in the reach sphere cost O(P^2) box tests. Round 8's `Jumper::new` (still at
`wrap_param.rs:1926-1944`) uses an x-sweep for the same test.
- `runs2\tiles300_r9` vs `runs2\tiles300_r8` (`models\tiles300.obj`: 90,000 separate 0.9 x 0.9 quads, gaps 0.1,
  wrap_surface step 10, Jump on): round 9 `wrap phases: ... collect 4039.1 ms (160635 candidates), front 1093.6 ms,
  relax 0.0 ms, owner 160.1 ms`, param **5,925 ms**; round 8 param **2,508 ms** for the same layout (83,785 verts,
  43,718 tris, 21,860 jumps in both). Round 9 is 2.4 x slower; collect is 68 % of it.
- (earlier run) `runs\parts100_r9` (10,000 parts) collect 43.3 ms; `runs\parts200_r9` (40,000 parts) collect 694.4 ms:
  16 x for 4 x the parts. Round 8 parts200 param 538 ms, round 9 1,021 ms.
- So the report's "heavy wraps about twice as fast as round 8" does not hold for models made of many parts with Jump on
  (foliage, tiles, kit-bashed scenes); the sweep has no such model. Not a pixel change (same verts/tris/jumps).

### F2 (major, shown) — `passed` fires on an open surface: the picture is cut and the panel names wrong causes
`runs2\noisy400_r9` (`models\noisy400.obj`: a 400 x 400 open height field, z noise +-0.6 per unit step, no tube, no
hole): both Jump settings `covered 0.6749, ends_share 0.3639, reached_edge false, lost_cut 0.20 %, passed true`, seams
2,015 `[173 S1, 86 S2, 0, 125, 1703]`. The shot (`runs2\noisy400_r9\crop.png`) shows the picture torn into a star of
strips and the panel says "33 % of the picture runs off the surface: an edge, a hole or the end of the part" and
"Longer than the way round: 36 % shown" — neither applies to an open plane. The step-10 ink assert is red (ink 0.3036,
dup 9.971 %, scatter 5.796 %); round 8 is also red there (`runs\noisy400_r8`: ink 0.7672, dup 39.9 %), so this is no
regression in the asserts, but it shows that the T2 "traces passed each other" test misfires away from tubes. It
supports reading the gate's decision 6 (ABeautifulGame Jump on cut to 59 % round a pawn) as a defect of the `passed`
detection rather than an intended behaviour. (A milder field, `runs\noisy200a15_r9`, earlier run: passed false, ok.)

### F3 (major, code only, not shown with the exe) — reuse != fresh: the exact region's fallback depends on the unbucketed picture
A parameterization is reused for any size whose bucketed reach and stop are equal (`serves_with`,
`wrap_param.rs:3271-3290`), and every layout decision is supposed to use the bucketed reach so that reuse is
bit-identical to a fresh build (comment at `wrap_param.rs` "Round 8: every layout decision that a footprint bounds uses
the bucketed reach"; plan 4.4). Round 9's T5 breaks that: `layout.relax(&seed, picture, ...)` passes the **unbucketed**
`picture = [hw + 2hw/src, hh + 2hh/src]`, and `Layout::relax` uses it as `rect` in `exact_holds`
(`wrap_relax.rs:559`, `733-760`: only free triangles meeting `rect` enter the flipped / 95th-percentile strain test).
Whether the exact pins are kept or the turn falls back to strip pins therefore depends on the exact picture size, not
its bucket: two sizes in one bucket can give different layouts, and which one the user gets depends on history (a cached
parameterization from an earlier size vs a fresh one after open / undo / another stamp). The round-9 reuse test
`wa26_wa27_round9_cases_are_deterministic` scales by 1.0001 only (`wrap_tests.rs:2819`), and WA27 by 1.2 on the bumpy
ball where the exact region does not decide anything, so neither would catch it. (In round 8 `relax` also received
`picture`, but used it only for statistics.)
Related, stats only: `stats_for` recomputes only `covered` (`wrap_param.rs:3217-3250`); `ends_share` (panel line "P %
shown"), `lost_cut`, `rejected`, `flipped`, `seams` of a reused parameterization are those of the size it was built for.

### F4 (minor, shown) — `lost_cut` counts the gaps between jumped parts as picture lost to cuts
`OwnerGrid::build` (`wrap_relax.rs:1897-2003`) counts an uncovered picture cell with covered cells on both sides of its
row or column as lost, unless it is next to a closure. With Jump small gaps every gap between parts is such a cell.
`runs2\tiles300_r9` Jump on: lost_cut **16.85 %** (the gap share of the tile grid is 1 - 0.9^2 = 19 %), no cut at all
(seams 0, relaxed false); (earlier run) parts100 / parts200 Jump on 25.74 % / 27.69 %, which turned the new step-10 assert
red ("Jump on: lost_cut 25.74 % > 10 %"). The new assert and the stat are therefore red / inflated for any multi-part
model with Jump on, independent of cuts (DamagedHelmet Jump on's 2.61 % may include gaps too).

### F5 (minor, shown) — one tiled UV triangle marks the whole texture set as "shared texels"
`uv_share.rs:125-126, 141, 146-147`: a triangle whose UV **bounding box** exceeds 1 is "huge" and adds 1 to every pixel's
count of its set (`everywhere`), so with one such triangle every other triangle of the set counts as covered twice and
"shares". A box > 1 does not mean the triangle covers the texture: a long trim strip (u 0..12, v 0.88..0.98) covers a
10 % band. `runs2\trim_strip_r9` (`models\trim_strip.obj`: a plate on its own island u, v 0..0.5 plus such a strip):
`shared texels found ... (100.0 % of the surface)`, step 10 `panel lines: shared texels 100.0 % (line shown)` for a
picture on the plate, whose texels nothing else uses. Panel-only false warning (no pixels change). An attempt at an
unbounded raster loop with a 1e7-long UV sliver (`models\uv_long.obj`, `runs2\uv_long_r9`) did not reproduce: the importer
wraps such triangles (`wrapped_tris: 1`) and the build took 9-17 ms; the same strip heuristic then flagged 100 % too.

### F6 (minor, shown) — many-part tiles: duplicates up with Jump on
`runs2\tiles300`: Jump on dup **2.052 %** in round 9 vs **0.047 %** in round 8 (scatter 0.024 % vs 0), same layout size;
within the 3 % limit. Ink 1.3286 vs 1.3280: the step-10 ink assert is red in both exes (not new).

## Checked and confirmed (code or data)
- Exe SHA-256 4F9E14222A17C9E1... (gate bin) matches the report.
- Line endings: `wrap_param.rs`, `wrap_relax.rs`, `wrap_tests.rs` are `i/crlf w/crlf` in round9-wip and LF (0 CR) in
  round8-wip; `uv_share.rs`, `wrap_mesh.rs` are `i/lf w/lf` (`git ls-files --eol`).
- DamagedHelmet "shared texels" is real, not a false positive: an independent count (`py\uvshare.py`, 1,024^2, same
  majority rule) gives 15,040 of 15,452 triangles = **86.7 %** of the area, equal to the app's log; its UVs are mirrored
  (708,770 texels covered twice or more, 684 once; v range 1..2).
- Warm-up / jobs / Final / close: no deadlock path found. `WrapMeshCell::get` and `uv_share` initialise serially
  (`wrap_mesh.rs:55, 274-293`; `UvShare::build` has no rayon); the warm-up is a plain thread outside `Jobs`
  (`stamp_bakes.rs` `warm_up`); a job or the warm-up that finds the cell being built blocks on the `OnceLock` while the
  builder runs serial code. `OwnerCtx::bury_index` is initialised inside `used.par_iter()` workers
  (`wrap_relax.rs:1794`, `wrap_param.rs:3006`) but `BuryIndex::build` is serial, so the blocked workers cannot be needed
  by the initialiser (the brush.rs hazard needs a parallel init). Latent: making `BuryIndex::build` parallel would
  deadlock.
- Mesh identity: `WrapMeshCell` holds a `Weak<MeshData>` (address cannot be reused while the cell lives); the mesh is never
  mutated in place (no `make_mut` / `get_mut` on it); `stamp_shared_texels` filters with `is_for`. Texel-map changes do
  not touch the graph or the shared texels (both depend on the mesh only). No stale-cache path found.
- Close / new document during a warm-up: the thread owns `Arc<MeshData>` + `Arc<WrapMeshCell>` and ends on its own; the
  old mesh and graph stay alive until then (<= 2 s on the chess set). `reset_for_new_document` clears `want_warm`;
  `ensure_wrap_param` re-requests every frame (`app.rs:293`).
- Determinism: sums in `chunked_sum` are fixed `PAR_CHUNK` chunks added in order for 1 or N threads; per-element kernels
  (`each`) write one element each; owner-grid passes are row-chunked with integer counts; the level-parallel component
  solve reads only its own wedges and the source component's (`relaxed_pose` reads one triangle's component, the same as
  `source_comp`), and the fallback memo is keyed by bit-equal start positions and pins. Hash maps in the new code are
  used for lookups only (no iteration). Phase times are thread-local log data only.
- Degenerate input: empty mesh, 1 triangle, zero-area triangles, NaN positions (total_cmp orders, NaN distances fail the
  snap / sphere tests) and a mesh without UVs (zero-area UV footprints are skipped) take no panicking path in
  `WrapMesh::build`, `collect_grown`, `Patch::from_graph`, `UvShare::build`, `BuryIndex`. u32 ids: half-edge `3 t + k`
  overflows only beyond ~1.43 G triangles; CSR offsets <= 3 x triangles. Memory: graph 104.9 MB for 1.5 M triangles (log);
  transient sort keys 16 B x 3 x triangles; owner grid <= ~1,025^2 cells (48 B per cell during the build, 21 B kept per
  cell per layer); shared-texel raster <= 4,096^2 bytes.
- Project mode / brush: no round-9 code runs in a Project or brush bake except `graph: wrap_mesh_cell()` (an `Arc` clone,
  nothing built) and the shared-texel warm-up on a background thread (no pixels); the owner test in `wrap_bake.rs` is
  wrap-only.
- The high_poly_sphere 6b flake: the one Preview job spent `owner 334.5 ms` on the 3,968-triangle sample sphere (typical
  13 ms), worker 397 ms (`gate\sweep_r9\high_poly_sphere_run1_6bfail\shot.log`): consistent with the gate's "load".

## Not verified
- F3 was not shown with the exe (no scenario scales within a bucket on a model whose exact region decides).
- Timing numbers of the gate (another skeptic's lens).
