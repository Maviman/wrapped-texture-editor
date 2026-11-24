# objects-tabs WP1 — core objects (plan section 2)

Builder notes, 2026-11-24.

## What was built

- `mesh.rs`: `ObjectInfo { name, tri_count, bounds }`, `RawObjects { tri_object, names }`, `MeshData::{tri_object,
  objects}`, `MeshData::{object_count, tri_object_of}`, `WireEdge::object`. `RawMesh` / `RawTriangle` unchanged.
- `mesh_build.rs`: `normalize_with_objects` (6.1 step 9); `normalize` = `normalize_with_objects(.., &RawObjects::default(), ..)`.
  `HalfEdge::object` is the last field; the grouping of `wire_edges` compares sides with `HalfEdge::same_side` (key, set,
  uv), never the object, so Interior / Seam / Boundary classification and the emitted entries are unchanged.
- Importers: `RawImport::objects`; OBJ (`o` / `g`), glTF (mesh nodes), FBX (`Model`s with geometry) as in plan 2.3;
  `import/mod.rs` calls `normalize_with_objects`.
- `samples.rs`: `object_trio()` (+ `OBJECT_TRIO_NAMES`), `to_obj_objects`; `to_obj` routes through the same writer with no
  objects and stays byte-identical (its existing round-trip tests are untouched and green).
- `doc_build.rs`: `SampleKind::Objects` (built through `to_obj_objects` and the OBJ importer: named "objects", texture
  "objects_checker.png").
- `testutil.rs`: `imported_from_raw_objects`, `doc_from_raw_objects` (the plain helpers call them with no objects).
- ARCHITECTURE.md: 2.1 rows, 5.7, 5.10, 5.15, 5.21, 5.25, 6.1 (step 9 + wire_edges object rule), 9.1.
- The four `MeshData { .. }` literals (mesh.rs test, bvh.rs test, tip_bake.rs, mesh_build.rs) and the one test
  `WireEdge` literal (`gpu/scene.rs`) were patched; no existing test was edited otherwise. The frozen `uv_hash` tests are
  untouched and green.

## Decisions where the plan left room

1. **Out-of-range raw object index** ("counts as 0 with an empty name"): read as "counts as raw object 0"; the name is
   `names[0]`, which is empty only when `names` is empty. (When the vector is empty everything is object 0 with no name.)
2. **Edge shared by two objects**: the emitted entry takes the object of the side's first half edge in the derived sort
   order, i.e. the smallest object index (deterministic).
3. **OBJ `o` with nothing after it** is a name `""` (an unnamed object, later "Object N" / the model name); `g` alone is
   `"default"` as the plan says.
4. **glTF node name ""** counts as no name (falls back to the mesh name), like a missing one.
5. **FBX**: a `Model` without geometry gets no `names` entry at all (it would vanish in normalization anyway).
6. Notes live in `docs/notes/objtabs/` (the workflow's instruction) rather than the plan's `docs/notes/objects_tabs/`.
7. `object_trio`: the floor is an 8 x 8 grid (not one quad) so that wraps and brush tests have triangles to work with; its
   grid lines are shifted (as `uv_cube_on_floor_n`) so no floor vertex meets a cube or ball vertex (the shells never weld).
   The cube's bottom face lies in the floor plane (as in `uv_cube_on_floor`).
8. Line endings: the repo has CRLF files (`import/obj.rs`, `samples.rs`, `gpu/scene.rs`, ...). Edits keep each file's own
   line endings (no whole-file EOL changes in the diff).

## Tests (all green)

`mesh_build`: `objects_follow_their_triangles_through_the_sort`, `object_data_never_changes_the_mesh` (two_material_model,
seam_cube, soup_sets, uv_sphere; random objects incl. out-of-range indices; also the wireframe equal apart from `object`),
`unnamed_and_duplicate_names`, `objects_without_triangles_disappear`, `no_object_data_is_one_object`,
`wire_edges_name_their_object`. `import::obj`: `o_lines_are_objects`, `g_groups_are_objects_when_there_is_no_o`,
`a_file_without_o_or_g_is_one_object_named_after_the_model` (also: `o` lines change neither `indices` nor `uv_hash`),
`to_obj_objects_round_trips_the_object_trio`. `import::gltf`: `mesh_nodes_are_objects`. `import::fbx`:
`models_are_objects`. `doc_build`: `the_objects_sample_has_floor_cube_and_ball`.

Workspace after WP1 (`test --workspace --no-fail-fast`): app 382 / 0 / 19, core lib 815 passed / 1 failed (WA20) / 24
ignored, integration 15 + 9 + 24 -> **1,245 passed, 1 failed (WA20), 43 ignored** (baseline 1,232 + 13 new). Clippy 0.

## Real-model check (throw-away test, release build, deleted after the run)

`import_from_path` on `G:\Claude_Resources\samples\khronos-gltf\ABeautifulGame.glb`: 1,498,080 triangles, **49 objects**,
15 sets, import 0.95 s. First names: `King_B` (40,480 tris), `King_W`, `Queen_B`, `Queen_W` (34,368), `Chessboard`
(92,416), `Pawn_Body_W1` (47,232), `Pawn_Top_W1` (2,208), ... `Pawn_Body_B8`, `Pawn_Top_B8`, `Castle_B1` .. `Castle_W2`
(30,528), `Knight_B1` .. `Knight_W2` (11,184), `Bishop_B1` .. `Bishop_W2` (74,520). Bounds are per piece on the board
(e.g. `Pawn_Body_W1` x 0.197..0.239, z -0.177..-0.135). Names unique, counts sum to the mesh.
Other samples: DamagedHelmet 1 (`node_damagedHelmet_-6514`), Avocado 1, WaterBottle 1, Lantern 3
(`LanternPole_Body`, `LanternPole_Chain`, `LanternPole_Lantern`), BoxTextured 1 (`Mesh`).
Full list: `target\objtabs\abg_probe_objects.txt`.
