# Stable, acceptance fixes (items A-F): notes

Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable`, on top of c82bff3. The items
come from the acceptance pass on exe 8328B74C (`target\accept\`: export_unseen, layer_save, test_gaps, verify_*). Raw
outputs (git-ignored) are in `target\accfix\`: `bin\` (the exes below), `before\`, `exp1..3\` (diagnostic runs),
`fix1\`, `fix2\` (the final runs), `mut\` (mutation runs), `oldproj\`, and the logs `v_test.txt`, `v_clippy.txt`,
`v_gpu.txt`. Every cargo command and every app run used `start /b /low /wait`; no run measured speed.

| Exe (`target\accfix\bin\`) | What | SHA-256 (first 16) |
|---|---|---|
| `wte_before.exe` | c82bff3 as it was (= the acceptance exe) | 8328b74cf99bd9fd |
| `wte_exp1/2/3.exe` | diagnostics (temporary logging, removed again) | - |
| `wte_fix2.exe` | the fixes (this commit's code) | a3d5e19575ced05f |
| `wte_final.exe` | the release build after the last source change (same sources as `wte_fix2`; the bytes differ, the build is not bit-reproducible here); smoke: `export_checks`, `wrap_surface` Lantern, `stamp_edge_path` WaterBottle, `final_bake_async` Avocado, `export_flow` BoxTextured (+ zip check PASS) all ok | 577fd91e16c99372 |
| `wte_mut.exe` | the fixes plus env-gated mutations (`WTE_MUT_*`, never committed) | - |

## A. Export dialog preview caption

`dialogs/export.rs::preview_caption(opts)` picks the caption by case:

| Layers | Flattened | Caption |
|---|---|---|
| Selected | on | "The preview shows the whole texture; the flattened file holds only the selected layers" (unchanged) |
| Selected | off (the "Export selected layers..." preset) | "The preview shows the whole texture; only the selected layers are exported, as separate files" |
| Visible / All | off | "The preview shows the layers together; no flattened texture is exported" |
| Visible / All | on | "Shows the flattened texture exactly as exported (seam padding and JPEG compression not shown)" |

All with the flattened texture on keeps "exactly as exported". The flattened texture is the visible layers under All
too (`ExportOptions::include_hidden`: "flattened always = visible only", ARCHITECTURE 5.x). All adds the hidden layers
to the per-layer, ORA and PSD files only. A first version of this fix said the flattened file "also holds the hidden
ones". `export_checks` (E1) showed that this was wrong: the All and the Visible flattened PNGs are byte-identical. It
was corrected before the commit.

- Unit test `the_preview_caption_says_what_the_export_writes` covers every case.
- Each frame, the dialog records the caption in `ToolProbe::export_caption`. `export_checks` asserts it in E2
  (selection, flattened off) and E3 (selection, flattened on).
- **Fails without the fix**: `WTE_MUT_A` (the old caption) fails `export_checks` at E2: "the caption is Some("Shows
  the flattened texture exactly as exported ...")".

## B. Side panel overlap

**Cause.** The Color section's `ScrollArea` had egui's default `min_scrolled_height` of 64 pt. A collapsed Color section
is about 28 pt tall (20 pt of content plus the margin). So the scroll area claimed 64 pt, and the frame reported a rect
of about 70 pt. The exact-size bottom panel then cut that rect down to 28 pt, keeping its bottom edge (egui's
`set_rect_size` for `Bottom`). The result was 533..561, where the section was actually drawn at 490..518. The panel
then moved the parent's cursor to 533, so the tool options' scroll area got 43 pt too many and ran over the "Color"
header and the Layers | Brush | Text strip.

The diagnostic logs showed this directly: inside the color panel `max_rect` was 495..516, while the response rect was
533..561.

It happens whenever the Color section is collapsed. It was visible only when the tool options were long enough to
reach the bottom: the wrapped-stamp panel on Lantern, or the Texture sets card on Avocado. It would also happen with the
section open in a window so small that the Color area falls under 64 pt.

**Fix.** `panels/mod.rs`: `.min_scrolled_height(0.0)` on the Color scroll area. The tool options' scroll area gets the
same, for the same class of problem in a very short window.

**Assert.** New `scenarios::assert_side_areas_apart` in `wrap_surface` (10), after the Color section is collapsed. It
checks three things, then runs `assert_side_panel_layout`:
- the tool area ends above the Color section;
- the Color section ends above the lower area;
- no tab button lies under either of them.

| Run | 1280x800 | 1024x640 |
|---|---|---|
| Assert without the fix (`wte_exp1`, and `wte_mut` with `WTE_MUT_B`) | fails on Lantern and Avocado: "the Color section ([[989 533] - [1272 561]]) runs 43.0 pt into the lower area ([[989 518] - ...])" | fails on Lantern: same, 43.0 pt |
| With the fix (`wte_fix2`) | passes on Lantern and Avocado | passes on Lantern and Avocado |

I looked at the screenshots myself: `fix2\side_pair.png` (Lantern 1280x800 and Avocado 1024x640) and `fix2\side_av.png`
(Avocado 1280x800, Texture sets card). The tool options end above the "Color" header, with their scroll bar showing.
The header and the tab strip are clear.

Note: `wrap_surface` on Lantern at **1024x640** still fails later in (10), on the wrap quality assert "Jump on: ink
0.4346, dup 3.401 %, scatter 2.797 %". `wte_before.exe` gives the same numbers. The view is smaller there, so the
picture lands elsewhere on the model; this is not related to the layout. One run under load (cargo test in parallel)
also failed (6b) "1 Preview jobs, 0 landed", which is timing. The rerun on an idle machine passed (6b). At 1280x800 both
models pass the whole scenario.

## C. final_bake_async step (k)

Since fix pass 2, Export no longer settles a pending Final: the export job waits. The new (k) works like this:
- it remembers `FinalBakes::settle_count`, a new counter of the `settle` calls that waited or baked on the UI thread;
- it runs the Final scale and `ShowExport` in the next frame;
- it asserts that the dialog is open and the count is unchanged (the UI was not blocked);
- after `WaitIdle` it asserts that nothing is unbaked and the dialog is still open, then `CloseDialog`.

The log also says whether the bake was still pending when the dialog opened. On Avocado it was. The doc comment and
ARCHITECTURE 8.11 are updated. "settle waited" now appears for (d) and (h) only: Save (c) and Export (f, k) no longer
settle. I checked this in the logs.

- `wte_before.exe` on Avocado: fails at (k) "the export dialog opened before the Final bake landed" (the old step).
- `wte_fix2.exe`: passes on the sphere, Avocado and WaterBottle.
- **Fails without the fix**: `WTE_MUT_C` (ShowExport calls `settle` again) fails the new (k) on the sphere and on
  Avocado: "opening the dialog waited for the Final bake on the UI thread (1 -> 2 settles)".

## D. Image stamp names without the extension

New `stamp_place::image_stamp_name(file_name)` gives the file stem: "wf_stamp.png" becomes "wf_stamp", " logo.v2.PNG "
becomes "logo.v2", an empty name becomes "Stamp", and ".png" stays ".png". A placed image stamp and its layers get this
name, so its per-layer export is `..._stamp_wf_stamp.png`, not `..._stamp_wf_stamp.png.png`.

`tools.stamp.source_name` keeps the full file name, because the Stamp options show it. The blob keeps the full file
name too (embedded file, Extract sources).

Tests:
- `an_image_stamp_is_named_after_the_file_stem` (new);
- `image_stamp_in_3d_becomes_a_layer_and_selects_it`, which now expects "ring";
- `project_roundtrip::an_image_stamp_named_with_its_extension_loads_unchanged`: a stamp and layers named "ring.png" are
  saved, loaded, and keep their names;
- `workflow_path` asserts that the placed stamp and its layers are named "wf_stamp".

**Old projects.** `oldproj\old_box.wtep` is a copy of the acceptance's `verify_layer-save_2\box_workflow_path.wtep`,
saved by 8328B74C, whose manifest has two "wf_stamp.png" names. I opened it with `workflow_path` on `wte_fix2`. The log
lists the layers as `"wf_stamp.png (stamp)"`, unchanged. The stamp the scenario places is named "wf_stamp".

**Fails without the fix**: `WTE_MUT_D` (the old naming) fails `workflow_path` on BoxTextured: 'the stamp is
"wf_stamp.png", its layers ["wf_stamp.png"]'.

## E. End-to-end export checks: new scenario `export_checks` (47 scenarios)

Setup: the sphere with the default content (Base, Paint 1 with the red and blue strokes, Logo, WTE). Logo is hidden,
and Paint 1 is at 50 % opacity in Multiply.

Every check reads the written files. The stack index comes from `_L<nn>_` in the file name. The checks compare against
`reference_composite`, which is ARCHITECTURE 6.6 written out in the test. It works in f64, quantizes the opacity and
the accumulator to u8 as documented, and applies the alpha-0 rule. Its input is the per-layer PNGs of the All export.
No export code is called.

- **E1.** Visible writes Base, Paint 1 and WTE. All writes those plus Logo, whose file equals Logo's layer pixels. The
  shared files are identical. Both flattened PNGs are byte-identical and leave Logo out (contract above): all 2611 of
  Logo's opaque texels differ from the stack with Logo.
- **E4.** Both flattened PNGs equal the reference of the visible layers: 0 texels over 2/255 (max 1/255). Against the
  same stack with Paint 1 as Normal 100 %, 18644 texels differ by more than 8/255, so the check does see the blend and
  the opacity.
- **E2.** The test clicks Paint 1's name, Shift-clicks WTE's name, right-clicks, and picks "Export selected layers...
  (3)". The selection is Paint 1, the hidden Logo and WTE.
  - The dialog had no hidden marker, so it got one. `PlanCache::hidden` (`hidden_layer_file`) puts a weak "(hidden)"
    next to the per-layer file of a hidden layer, under All and under a selection. The lines, as drawn, are recorded
    in `ToolProbe::export_files`.
  - The list marks exactly Logo's line. The export writes the three files, each equal to its layer's pixels; Logo's
    file has ink.
  - Unit tests: `the_list_for_a_selection_and_for_a_zip_package`, `plan_cache_tracks_the_options`.
- **E3.** `SelectLayer(Base)` + `ToggleLayerSelected(WTE)`, the dialog opened for them, the flattened PNG ticked. The
  flattened file equals the reference of Base + WTE (0 texels over 2/255, max 0). It differs from the visible stack in
  18536 texels, so Paint 1 is not in it.

**Mutations** (`wte_mut.exe`; with no variable set it passes `export_checks`):

| Variable | Mutation | Result |
|---|---|---|
| `WTE_MUT_E1` | hidden layers in Visible's per-layer files | E1 fails: "per-layer files: Visible [0, 1, 2, 3]" |
| `WTE_MUT_E4` | Multiply computed as Normal in the core | E4 fails: "Visible 18647 texels (max 59)" |
| `WTE_MUT_E2` | no "(hidden)" marker | E2 fails: "... is not marked hidden" |
| `WTE_MUT_E3` | the selection's flattened file is the whole stack | E3 fails: "18858 texels differ from Base + WTE (max 94/255)" |

The scenario passes at 1280x800 and at 1024x640. ARCHITECTURE 8.11 has its row, and the scenario count test is 47.

## F. stamp_edge_path on WaterBottle: a fitting bug, fixed

**Attribution.** I re-placed the stamp in four variants and logged (full-alpha texels / any-alpha texels):

| Variant | Result |
|---|---|
| fitted depth (front 0.0047, back 0.0458), angles 70/88 | 38460 / 95505 |
| fitted depth, the old angles 60/85 | 38272 / 95294 |
| depth h / h (0.0454), angles 70/88 | 40950 / 95519 |
| only the front set to h | 40950 / 95519 |

The whole difference is the **front** depth. The texel diagnostics showed:
- surface up to z = +0.0045 in front of the placement plane;
- 7625 texels in the front fade (z > 0.8 x 0.0047), all in the lower-centre of the footprint.

The screenshot shows why. The stamp sits where the bottle's black grip sleeve begins, and the sleeve stands about 4.5 mm
in front of the body. The body continues under the sleeve. The fit grows its surface from the centre, and on the
sleeve cells it picks the hit closest in depth to the neighbour, which is the hidden body. The first-hit rule
("within one half size") applied only to cells *off* the surface. So the visible sleeve never counted: hi was about
0.0002, the front was 0.0047, and the sleeve landed in the fade.

This is not a limit of the bottle's UVs or topology. The bake paints the visible sleeve (occlusion hides the body), and
the box must hold it. The limit is unchanged.

**Fix** (`stamp_bake::fit_projector_depth`). The first (visible) hit of *every* cell counts when it is within one half
size of the plane. For a single surface the first hit is that surface, so nothing changes there. A nearer part over the
grown surface now extends the front depth, as the old default (depth = half size) did. ARCHITECTURE 6.5 "Depth
auto-fit" and the doc comment are updated.

- New fixture `samples::sleeve_quads` and test `auto_fit_covers_a_visible_part_in_front_of_the_grown_surface`: the
  sleeve lies before the fade and is painted at full weight.
- **Fails without the fix**: with the old `near_parts` line the test panics at `stamp_bake.rs:2213`; with the fix it
  passes. The other two auto-fit tests pass both ways.

| Model | Before | After (`wte_fix2`) |
|---|---|---|
| WaterBottle | fitted 38460 / 95505 vs old default 40762 / 95308: **fails** | front 0.0100, fitted 40950 / 95519 vs 40762 / 95308: **passes** |
| Avocado | 54178 / 201272 vs 46576 / 186928 | unchanged, passes |
| Lantern | 43926 / 210412 vs 35784 / 202986 | unchanged, passes |

## Gate commands (all at low priority)

| Command | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` | 1232 passed, 1 failed, 43 ignored. The one failure is WA20 (`cells 0.689751..1.986138`), the known red with the same numbers. |
| `cargo clippy --workspace --all-targets` | 0 warnings |
| `cargo test -p wte_app -- --ignored --skip perf_` (GPU app tests) | 18 passed |
| `cargo build --release -p wte_app` | OK |

Scenarios on `wte_fix2.exe` (copied to `target\accfix\bin\` before the runs), all `"ok": true` unless noted:

| Scenario | Runs |
|---|---|
| `export_checks` | 1280x800 and 1024x640 |
| `export_flow` | BoxTextured, ABeautifulGame |
| `export_selected` | built-in |
| `wrap_surface` | Lantern, Avocado and the sphere at 1280x800; Avocado at 1024x640. Lantern at 1024x640 passes the new layout assert, then hits the earlier quality red that the old exe has too. |
| `final_bake_async` | sphere, Avocado, WaterBottle |
| `stamp_edge_path` | WaterBottle, Avocado, Lantern |
| `workflow_path` | BoxTextured, and the old project `old_box.wtep` |

`target\accept\my_zip_check.py` on the export_flow run folders:
- `fix2\ef_Box`: RESULT PASS (8 entries, composite worst 1/255, loose twin byte-equal);
- `fix2\ef_ABG`: RESULT PASS (84 entries, 15 composites checked, worst 1/255).
