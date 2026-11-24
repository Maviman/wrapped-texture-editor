# objects-tabs WP2 — core visibility (plan section 3)

Builder notes, 2026-11-24.

## What was built

- `bvh.rs`: `TriMask` (bit set: `from_fn`, `get`, `count`, `is_empty`); `Bvh` is now `{ tree: Arc<BvhTree { nodes,
  tri_order }>, hidden: Option<Arc<TriMask>> }` with `with_hidden` (O(1), shares the tree; an empty mask is stored as
  `None`), `hidden`, `is_hidden`. `intersect` and `walk_sphere` take the mask once and skip a hidden triangle in the
  leaf loop before the triangle test. `Bvh::build` returns `hidden: None`. Still `Send + Sync` (the existing
  `texel_map` compile-time check covers it).
- `document.rs`: private `hidden_objects: Vec<bool>`, `visibility_generation: u64` (set in `from_parts`, not in any
  snapshot, history or manifest); `hidden_objects`, `object_hidden`, `hidden_object_count`, `visibility_generation`,
  `set_hidden_objects` (guard_no_stroke; pad / cut; `Ok(false)` when unchanged; swaps `self.bvh` for
  `bvh.with_hidden(mask)`; `WrapCache::clear()`), `visible_bounds`, `objects_bounds`. `WrapCache::clear`.
- `wrap_param::collect_grown`: `in_sphere` returns `None` for `bvh.is_hidden(t)` (the growth over `WrapMesh::vtris` is
  the only non-BVH path).
- `stamp_job.rs` / `doc_stamps.rs` (addition, see decision 1): `StampBakeOutput::bvh`; a wrap parameterization from an
  output is cached only while `Arc::ptr_eq(out.bvh, self.bvh)` (`apply_bake_output`, both branches of
  `apply_wrap_preview`, `apply_wrap_param`). Pixels still apply as before.
- ARCHITECTURE.md: 2.1 (`bvh.rs` row, `tests/objects.rs` row), 5.8 (`TriMask`, `with_hidden` ...), 5.14 (API + the rules
  table of plan 3.4), 5.20 (job carries the BVH; caching rule; `WrapCache::clear`), 6.2 (mask in the leaf loops), 6.4,
  6.5, 6.5a (one paragraph each), 9.1.

## Builder checks

1. **`collect_grown` Jump branch (plan 3.3): confirmed.** Its part flood (`for &u in g.vtris(v)`, wrap_param.rs ~2835)
   only enters `u` with `mark[u] == 1 && comp[u] == 0`, and `mark == 1` is set only for triangles returned by the masked
   `query_sphere_dist(.., full_r, ..)`. No `is_hidden` test is needed there. The non-Jump flood uses `in_sphere`, which
   now rejects hidden triangles (marked 2, never entered). The anchor comes from `near`, itself from masked queries.
2. **grep `g.vtris(` / `0..mesh.tri_count()` in `wrap_param` / `wrap_relax` / `wrap_bake`:** `g.vtris(` occurs only in
   `collect_grown` (the two floods above). Every other `vtris` call is `patch.vtris` (local to the collected patch).
   No `0..mesh.tri_count()` loop in the three files; `wrap_mesh.rs` builds the whole-mesh graph (topology only, no
   visibility meaning). The test-only `Source::Sphere` path (`collect_sphere`) uses `query_sphere_dist` only (masked).
   `wrap_param`'s remaining raycasts / sphere queries (2080, 3293, 3348, 3372, 3380) are BVH queries (masked).
3. Paint paths: no whole-mesh triangle scans outside tests in `brush.rs`, `stroke.rs`, `stamp_bake.rs`, `stamp_job.rs`
   (candidates, occluders and the depth fit all come from the BVH).

## Decisions where the plan left room

1. **Stale wrap parameterizations.** The plan clears the wrap cache on a visibility change, but a Preview / ParamOnly /
   Final job made before the change can land after it and would put a parameterization built with the old visibility
   back into the cache (which `serves` cannot detect). The output now carries the job's `Arc<Bvh>`; its
   parameterization is cached only when that is still the document's BVH. The pixels of such an output still apply
   (rule "a bake uses the visibility of the moment the job was made"). Tested in
   `document::objects_tests::hiding_drops_cached_wrap_params`.
2. `visible_bounds` when every object is hidden returns `mesh.bounds` (plan); `objects_bounds` ignores unknown indices
   and objects with empty bounds.
3. `set_hidden_objects(&[])` = show all (a short slice is padded with false).
4. `TriMask::from_fn` is serial (1.5 M triangles: one pass, a few ms; measured cost of the whole toggle is for WP3b's
   scenario log).
5. One existing test line changed (`bvh.rs` `large_soup_matches_brute_force_and_builds_in_parallel`: `bvh.tri_order` ->
   `bvh.tree.tri_order`, a private field that moved). No assertion changed.

## Tests

`bvh`: `tri_mask_basics`, `a_hidden_triangle_is_skipped_by_every_query`, `no_mask_and_an_empty_mask_answer_identically`
(plain = `None` = empty mask on 2,000 random rays / spheres, and a random half hidden = brute force over the visible
triangles), `with_hidden_shares_the_tree` (through a `#[cfg(test)]` `shares_tree_with` = `Arc::ptr_eq`).
`document::objects_tests::hiding_drops_cached_wrap_params`.
New integration file `crates/wte_core/tests/objects.rs` (on `Document::sample(SampleKind::Objects)`):
`hiding_is_view_state` (revision, modified, undo label, layer pixels and composite unchanged; generation +1; same flags
-> `Ok(false)`; pad / cut; undo / redo leave visibility alone; refused during an open stroke), `visible_bounds_and_object_bounds`,
`rays_pass_through_a_hidden_object`, `a_brush_stroke_paints_under_a_hidden_object_and_not_on_it` (cube hidden: floor
region changes, cube and ball regions byte-identical; shown: cube changes, floor clean; label "Brush Stroke"),
`a_projected_stamp_skips_a_hidden_object` (no cube ink, more floor ink; depth fit placed on the cube's top reaches the
floor only once the cube is hidden), `a_wrap_never_enters_a_hidden_object` (Jump off and on: no cube ink,
`stamp_wrap_local` is `None` on every cube triangle after `ensure_wrap_param`; with the cube shown Jump does reach it),
`a_bake_job_keeps_the_visibility_it_was_made_with` (deferred Final job made with the cube shown, cube hidden, job run and
applied: equals the bake with everything shown; a new bake differs and has no cube ink),
`masked_results_do_not_depend_on_the_thread_count` (stroke + wrapped Jump bake with the cube hidden, pools of 1 and 4
threads: identical pixels).

Workspace (`test --workspace --no-fail-fast`): app 382 / 0 / 19; core lib **820 passed, 1 failed (WA20, known red),
24 ignored**; integration brush_tips 15, end_to_end 9, **objects 8**, project_roundtrip 24 -> **1,258 passed, 1 failed
(WA20), 43 ignored** (WP1 1,245 + 13). Clippy `--workspace --all-targets`: 0 warnings.

## Perf (plan 3.5 check, risk 3)

`cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1` (debug test profile; whole-run time / slowest
parameterization of check:A and WA7; 200 ms limits):

| Run | Whole run | check:A slowest | WA7 slowest |
|---|---|---|---|
| baseline exe, run 1 (WP0) | 36.2 s | 75.9 ms | 94.9 ms |
| baseline exe, run 2 (between the WP2 runs) | 34.0 s | 66.7 ms | 87.6 ms |
| WP2, run 1 | 35.0 s | 67.4 ms | 88.7 ms |
| WP2, run 2 | 36.6 s | 132.0 ms (single triangle x3: an outlier, a 1-triangle mesh) | 94.6 ms |

Same range as the baseline (run-to-run noise of several ms; the 132 ms is a scheduling outlier on a one-triangle mesh,
far from any BVH work). Brush on the heavy model, the hot path the mask touches (`WTE_STROKE_LOG=1`,
`brush_types_path` on ABeautifulGame.glb, 7 strokes incl. Size 512, two runs per exe, release builds):

| Exe | BVH query time, sum over strokes | setup sum | raster sum | candidates |
|---|---|---|---|---|
| baseline (`wte_base.exe`) | 3,955 / 3,915 ms | 3,845 / 3,832 ms | 37,378 / 37,554 ms | 36,069,817 |
| WP2 (`target\objtabs\wp2\wte_wp2.exe`) | 3,869 / 3,886 ms | 3,820 / 3,844 ms | 37,791 / 38,127 ms | 36,069,817 |

No slowdown in the query (the only changed code); identical candidate counts (same work).

## Behaviour check of the release exe (nothing hidden = nothing changed)

WP2 release exe (`target\objtabs\wp2\wte_wp2.exe`) on the whole baseline job list (`target\objtabs\wp2\sweep\`):
49 / 50 `"ok": true`, the same 1 expected failure as the baseline (`import_save` on BoxTextured.glb, no sidecars).
Compared with the baseline outputs file by file (233 PNG / export files): 216 byte- or pixel-identical; the other 17 are 12
`.wtep` files (save time in the manifest) and 5 screenshots whose only differences are the output path printed in a
dialog (`...\baseline\...` vs `...\wp2\sweep\...`) and the fade of a toast (`target\objtabs\wp2\diff_crops.png`).
All exports (PNG / PSD / ORA / ZIP) are byte-identical.
