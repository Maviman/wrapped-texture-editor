# Round 7 check-and-fix (brush speed): notes

Agent: round7-check (Opus), 2026-11-21. Copy: `target\round7\work\pkg` (the builder's). Base for diffs:
`target\round6\work\base`. Plan: `target\round7_design\ROUND7_PLAN.md`. Diffs written to `target\round7\check\*.diff`.

## Checked (diff review against the plan)

- **File ownership**: `git diff --no-index` over `crates\` shows exactly five changed files: `wte_core\src\{texel_map.rs,
  brush.rs, stroke.rs, bvh.rs}` and `wte_app\src\tools\brush.rs` (`phase_text` only: slots 10, 11, 12, 14 printed as
  counts). All in round 7's column of plan section 5; none in round 6's. `Cargo.toml` / `Cargo.lock` identical to base:
  no new crate. `bvh.rs`: only `query_sphere_dist` (A3 sort). `ARCHITECTURE.md` hunks lie in 4.6, 5.9 (before 5.10),
  5.19 and 6.3 / 6.4 only; 1.7 and 9.1 not edited (texts in `notes\pkg.md`).
- **No `unsafe`** in any of the diffs (grep of the diffs: 0 hits). Public API: `raster_tri` / `raster_tri_covered` keep
  their signatures and pass `lean = false` (stamps, wraps, fill unchanged); the new texel-map functions are
  `pub(crate)`; `STROKE_PROBE_SLOTS` 10 -> 15 and five names appended (plan 3.0).
- **A1 (lean pass 2)** reviewed against the full pass 2 case by case: pass 2 reports (a) gutter texels the triangle owns
  (bit set: every gutter owner is in `pass2_tris`), (b) a covered texel it owns whose centre test fails: only a sliver's
  claimed texel (step 1 owns exactly the texels whose centre `span` finds inside, with the same expression and
  thresholds as `inside`; in the wide / ambiguous case `best_image` picks an inside image), so `claimed` covers it,
  (c) gutter texels of other owners with a `shared_gutter` source: they lie in `cols x rows`, whose 8 x 8 blocks are
  tested. The bitset is built after the build's final owner pass (GUTTER_BIT then means gutter only). The block-range
  bit test (`any`) handles full words and partial words correctly. Same rule as round 5's indexed path.
- **Deadlock rule**: the only lean callers are `raster_cand(.., lean = true, ..)` from `raster_commit`, which calls
  `prepare_brush_index()` on the calling thread before any band job; `stroke_target` builds it on a set's first touch.
  `gutter_by_tri` (lazy, inside band jobs since round 5) initialises serially, so it cannot deadlock.
- **A2 (fused pass)**: CSR binning by `raster_box` (the same `scan_box` the rasterizer uses); `box_bands` handles boxes
  of >= H rows, wrapping boxes and the merge of two wrapped ranges; every fragment of a band's call lies in its rows
  (the `rows` argument), so `cov[i - base]` cannot underflow; tiles merged by union per key; constants fixed (16,384 /
  32 / 8 / 4,096), never derived from the thread count. Dropped candidates (no box) visit nothing in round 5 either.
- **A3**: chunks of 8,192 fixed; per-chunk maxima of `f32::max` (order-free; NaN-safe as in the serial loop); the
  second pass's per-chunk lists are appended in chunk order (lists stay ascending); `maps.len() == n` guards the case
  where the second query grows past 32,768 while the first did not (then serial). `set_of_par` equals `set_of`
  (`maps.get(s)` is `None` for `s >= n`). The BVH walk reports each triangle once, so the parallel unstable sort keeps
  the order.
- **A4 (span re-apply)**: `reapply` runs right after every dab with that dab's rects and the stroke's fixed
  `BrushParams` (colour, opacity, mode, alpha lock are per stroke), so a texel outside the dab's changed spans holds
  `apply_brush(original, mask)` already (or the original when its mask is 0). Stale spans (rows of bands without
  candidates, rows the serial commit did not record, spans left from an earlier stroke on a reused scratch) are only
  ever supersets: extra shading of an unchanged texel gives the same value. A 2D dab clears `spans_valid` (whole rects).
  Cancel / end clear the mask over dirty rects; spans need no reset.
- **Probe fields**: slots 10-14 as the plan says; band jobs return counts / times and the calling thread adds them; with
  the probe off each call is one relaxed load.
- **Determinism**: band partition depends only on the dab; T2 runs pools of 1, 2 and 5 threads.

## Tried to break it (new tests, in round 7's files)

- `brush::tests::check_r7_adversarial_dabs_equal_round5`: r7 test meshes + `tiled_strip(5, 3.3)` + a 3-set soup
  (`soup_sets(11, 400, 3)`); images 1 x 97 (g0), 97 x 1 (g1), 2 x 300 (g16), 333 x 7 (g3), 129 x 65 (g1: a 1-row last
  band at 32 / 64 rows), 64 x 33 (g16), 200 x 97 (g0); radii 0.1 texel, 1, 6, 40 texels, 0.3 x and 3 x the model
  diagonal (boxes far wider than the image); ball hard / soft, Cube Solid / Through; SameSide / All; every second dab
  repeated on the same spot (saturated masks, increments of 0, strength 1.0 on the last). Each dab: `dab_tip` ==
  round 5 (`RASTER_R5`), masks and rect sets; the forced fused pass (`par_from` 0) with bands of 16 / 32 / 64 rows in
  pools of 1 and 3 threads == round 5 and leaves `dab_cov` zero / `dab_touched` empty; every texel whose mask rose lies
  inside `changed_span` of its row (A4 invariant).
- `stroke::tests::check_r7_many_sets_cancel_and_2d_strokes_paint_round5s_pixels`: a 6-set soup (3,000 triangles,
  128² per set); five strokes paced with `stroke_to_3d_paced` (Partial segments), varying pressure; a 0.6 ball that
  starts >= 5 sets in one stroke, a 0.45 stroke cancelled halfway with a segment still open, a 2D stroke on set 2
  after strokes 1 and 3 (A4's spans after a 2D dab and stale spans on the reused scratch), Cube tips at 0.15 and 0.004;
  document state == round 5's (`RASTER_R5` + `FULL_REAPPLY`).
- Brush / stroke / texel_map tests (84 tests, incl. T1-T6 and the two above) rerun with `RAYON_NUM_THREADS=1` and `=2`:
  all pass.
- Cross-check in the app: the ABeautifulGame `brush_types_path` probe's "touched" counts per stroke (19,993,452 /
  19,993,452 / 25,670,217 / 11,571,945 / 341.3 M / 557.3 M / 371.6 M) are the step-0 run's (round-5 code) exactly.

## Found

- No pixel, determinism, ownership, `unsafe` or crate problem. Both new tests passed on the builder's code as it was.
- Documentation gap: `ARCHITECTURE.md` (copy) described A1 / A2 but not **A3** (parallel shared query and sort) nor
  **A4** (span re-apply and its `FULL_REAPPLY` seam), although both were built and kept.
- Minor, not changed: the lean check allocates one small `Vec` (`in_rows`) per call; the forced fused path (`par_from`
  0, tests only) would panic in `par_chunks_mut(0)` on a 0-width map (unreachable from the app: `par_from` is 16,384
  there and an empty dab returns before it).

## Fixed

- `ARCHITECTURE.md` 6.4, in round 7's "Round 7" paragraph before "2D dab": one paragraph on A3 (chunks of 8,192 from
  32,768 found triangles, chunk-order lists, parallel sort with unique keys, serial walk) and A4 (`row_spans`, why a
  stale span is only extra work, 2D dabs re-apply whole rects, the `FULL_REAPPLY` seam).
- Added the two tests above.

## Reruns (this copy, after the changes)

- Build lock taken for the compile steps only (`check\build_c1.log`, `check\build_c2.log`); a round-6 `rustc` was running
  at the first compile with 1.9 GB free (above the 1.5 GB rule), so no wait.
- `cargo test --workspace`: **1,076 passed, 32 ignored, 0 failed** (332 + 699 + 15 + 9 + 21; builder 1,074; round 5
  1,067). Log `target\round7\check\test_ws_c.log`.
- `cargo clippy --workspace --all-targets`: 0 warnings. Release build ok.
- Exe `target\round7\runs\check\wte_check.exe` = `work\pkg\target\release\wrapped_texture_editor.exe`, SHA-256
  EBBB49604A96F03278CAC864E83AD72BDD969FD67A451E354F6B7C40516F1CA9 (differs from v3 only by the rebuild; no app code
  changed).
- Smoke (`runs\smoke_check.ps1`, `runs\check\smoke.log`, one process at a time, no cargo / rustc running): `brush_types`
  ok, with `RAYON_NUM_THREADS=1` and `=2` ok; the three shots are byte-identical to each other, to v3's and to round 5's
  sweep shot (`target\round5\runs_gate\runs\brush_types_builtin\shot.png`). `brush_types_path` on ABeautifulGame
  (`WTE_STROKE_LOG=1`) ok, on ngons_tiling ok; `workflow_path` on ABeautifulGame ok. 0 "panicked" / "Invalid parameter"
  in the logs.

## Lines for 9.1 (add to the builder's C3 text in `notes\pkg.md`)

```
  Round 7 check: adversarial dabs (1 x N and N x 1 images, a 1-row last band, gutters 0 / 1 / 16, boxes far wider than
  the image, a multi-set soup, repeated dabs on saturated masks) give round 5's masks and rects through `dab_tip` and
  through the forced fused pass (bands of 16 / 32 / 64 rows, pools of 1 and 3), and every raised texel lies in its row's
  changed span (`check_r7_adversarial_dabs_equal_round5`); paced strokes over six sets, a cancelled stroke with an open
  segment and 2D strokes between 3D strokes give round 5's document state
  (`check_r7_many_sets_cancel_and_2d_strokes_paint_round5s_pixels`).
```

## Still open (for the gate / later)

- P3-P10 timings are the gate's; the builder's indicative P4 miss (Size-50 backlog 453-773 ms vs 350 ms) and the
  borderline P5 / P6 stand. Nothing here changes speed.
- Pass 2 still looks at about 3 x the touched texels at Size 50 (island-border triangles scan their whole box). A
  next step outside this plan: in the lean call, walk only the triangle's own gutter texels when no shared block meets
  the box; it would need `gutter_by_tri` built in `prepare_brush_index` (its lazy serial sort inside a band job would
  stall the first dab of a large set).
- Plan section 4 says 6 pool threads; the machine's global pool has 12 (12 logical processors). Doc 4.6 says so.
- `brush_types_path` needs a PATH (as in round 5); not runnable on the built-in sample.
- Snapshot folders `target\round7\step0_src`, `target\round7\a12_src` (stale) can be deleted; `runs\check\wf_abg.wtep`
  (119 MB) is a scenario output and can go too.
