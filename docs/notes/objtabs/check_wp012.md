# objects-tabs — check of WP0, WP1, WP2 (core objects + visibility)

Independent checker, 2026-11-24. Builder commits 5c0c991 (WP0), a7b7e0a (WP1), eccefda (WP2), read against
`docs/plans/OBJECTS_TABS_PLAN.md` sections 2, 3, 11 and the ARCHITECTURE edits they name.

## Verified

- Builder's counts reproduced before any fix: `test --workspace --no-fail-fast` = 1,258 passed / 1 failed (WA20, the known
  red) / 43 ignored (app 382/0/19, core lib 820/1/24, brush_tips 15, end_to_end 9, objects 8, project_roundtrip 24).
- Plan 2.1-2.5 item by item: types, `normalize_with_objects` (object index outside the sort and vertex keys; dense
  renumbering in raw order over kept triangles; names; counts and bounds), `HalfEdge::object` as the last field with
  grouping by `same_side` (so the entries and their order are unchanged; only `object` is new), the three importers,
  `object_trio` / `to_obj_objects` / `SampleKind::Objects`, every test the plan lists. The frozen `uv_hash` tests are
  untouched; the only edited old test lines are the private-field rename in `bvh.rs` and a path in `import/mod.rs`.
- Plan 3.1-3.5: `TriMask`, `Bvh { tree: Arc<BvhTree>, hidden }`, the skip in both leaf loops (`intersect`, `walk_sphere`)
  — every public query (`raycast`, `occluded`, `occluder`, `query_sphere`, `query_sphere_dist`) goes through one of them.
  `Document::set_hidden_objects` and the bounds functions; `WrapCache::clear`; `collect_grown`.
- Non-BVH paths, looked for adversarially: the projected bake's occluder grid (`stamp_bake::Occluders::build`) and its
  hint are built from the masked sphere query and masked `occluder` calls, so a hidden triangle can never be a proven
  occluder; the brush's per-stroke candidate lists (`DabQuery`) live only for one stroke, and a stroke blocks
  `set_hidden_objects`; `stroke.rs` clones the BVH per dab; no whole-mesh triangle loop in `brush`, `stroke`,
  `stamp_bake`, `wrap_param`, `wrap_relax`, `wrap_bake` outside tests; `Bvh::build` runs once (`doc_build`) and nothing
  replaces `Document::mesh` / `bvh` except `set_hidden_objects`. The app queries the model only through
  `Document::raycast` (tools, text path, transform, view), so the mask applies there with no app change.
- Worker races: Final / Preview / ParamOnly jobs clone `Arc<Bvh>` when made; outputs carry it; a parameterization is
  cached only while it is still the document's BVH (builder's addition, tested). Old projects: `from_parts` is the only
  constructor and sets all-visible; the manifest is untouched (`FORMAT_VERSION` 3, `IMPORTER_VERSION` 2).

## Findings and fixes

1. **Fixed (major, perf): quadratic object naming.** `mesh_build::object_infos` restarted the `_2`, `_3`, ... search at 2 for
   every duplicate. An instanced glTF scene whose nodes are unnamed falls back to one mesh name for every node; 20,000
   such objects took **37 s** to import in the test build. The search now resumes per base after the last suffix it
   tried (same names: every smaller suffix is taken and names are never released). Test
   `many_objects_with_one_name_are_numbered_in_linear_time` (20,000 x "Leaf" plus an explicit "Leaf_3"; now
   milliseconds). ARCHITECTURE 6.1 step 9 and 9.1.
2. **Fixed (major, latent until WP3a): a visibility change made the app give up on a wrapped stamp's parameterization.**
   With the WP2 rule "a parameterization made with another BVH is not cached", a param-only job in flight across a hide /
   show lands with nothing cached; `stamp_bakes::apply_wrap` then saw "same placement, no stats" and set
   `param_failed` ("no surface to wrap onto: not asked for again"), so the stamp's outline / stats never came back until it
   was moved. The synchronous mode had the same trap in `TransformTool::wrap_failed`: a stamp whose surface was hidden
   was marked failed and not retried after Show all. Both marks now carry the document's `visibility_generation`
   (`WrapFlight::visibility`; a failure is recorded only when the generation did not change during the job). Tests
   `a_param_only_job_overtaken_by_a_visibility_change_is_asked_for_again`,
   `the_synchronous_wrap_failure_is_retried_after_a_visibility_change`. ARCHITECTURE 8.x (Transform tool text,
   `WrapJobs`) and 9.1.

## Not fixed (minor, noted)

- `Document::visible_bounds` allocates a `Vec` of every object index per call (only F / reset camera call it; harmless).
- The plan's determinism item asked to re-run an existing brush / wrap determinism test with a mask; the builder wrote a
  new one (`masked_results_do_not_depend_on_the_thread_count`, stroke + wrapped Jump bake, pools of 1 and 4 threads),
  which covers the same intent.
- An edge shared by two objects is drawn by one of them (plan 2.2 rule 6, documented); the Three Objects sample avoids
  shared vertices on purpose, real models (a piece standing on a board with coincident vertices) will show it.
- WP3a must still key the app's hover caches by `visibility_generation` (plan 4.3); not in this package.

## After the fixes

`test --workspace --no-fail-fast`: **1,261 passed / 1 failed (WA20, known red) / 43 ignored** (app 384, core lib 821, brush_tips 15,
end_to_end 9, objects 8, project_roundtrip 24). `clippy --workspace --all-targets`: 0 warnings.
