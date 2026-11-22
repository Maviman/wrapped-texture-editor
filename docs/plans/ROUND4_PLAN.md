# Round 4 plan — Wrapped Texture Editor

Written 2026-11-17. This is a plan, not a result: nothing was built, run or measured for it, and no source file was
changed. It turns two finished design documents into work packages for agents that build in parallel:

- `target\round4_design\FEATURE_BRUSH.md` (2,247 lines) — brush types Circular and Sphere, a brush shape loaded from
  a model file, a Brush tab. Called "BRUSH" below; "BRUSH 5.7" means its section 5.7.
- `target\round4_design\FEATURE_TEXT.md` (2,523 lines) — fonts of the machine, bold, slant, warp, text on a line,
  spacing between single characters, a Text tab. Called "TEXT" below.

How to read this plan:

- All paths are below `E:\claude_programs\wrapped_texture_editor\`. `core\` means `crates\wte_core\src\`, `app\`
  means `crates\wte_app\src\`.
- The two design documents are the specification. This plan does not repeat their algorithms; it says who builds
  what, in which files, in which order, and how it is checked. Where the two documents disagree, section 2.2 of this
  plan decides.
- A statement about today's code carries a `file:line` reference that was read for this plan in the main tree on
  2026-11-17 (the state before round 3). Numbers taken from the design documents and not re-read are marked
  "(design)". All sizes and times are estimates until the gate measures them.
- Words: a "package" is one agent working in its own full copy of the source. The "scaffold" is the package that
  writes every type and file that more than one other package needs. The "gate" is the last package: it merges,
  measures and reports.

A completeness check on 2026-11-17 compared this plan and the two designs with the request and found 18 gaps
(7 major, 11 minor). All were taken over; none was rejected. Section 6 lists what was changed for each. The two
design documents were changed in the same pass, so plan and designs agree again.

The user's request, word for word:

> BRush type: circular vs sphere, where circular is what you have and sphere will glide along an object and paint
> whatever is in its sphere. i want to be able to upload and orient the shape and size of the "sphere" into any
> compatible model type. can be its own tab like in photoshop.
>
> Stamp type; text: being able to choose from fonts on the machine to put onto the stamp. the text can be warped,
> curved along a line, angled, slanted, bolded, or segmented like in photoshop.

---

## 0. The plan on one page

1. **19 items**, R4-1 to R4-19: one shared (the tab strip), eight for the brush, nine for the text, one for the gate.
2. **17 packages** in five steps: a baseline measurement (P0), two scaffold packages one after the other (S1 core,
   S2 app), eight core packages in parallel (wave 1), six app packages in parallel (wave 2), the gate (Z). After the
   scaffold no file has two owners. Every self-test scenario is written and first run by the package that builds
   the feature; the gate only pastes and registers them (2.2 item 11).
3. **Nothing that reads or changes source starts before round 3 is merged into the main tree.** On 2026-11-17 round 3
   was still running: `target\round3\notes\` holds notes of packages B and C only, and `target\round3\work\` has no
   `merged` copy. Section 2.6 lists what waits and what does not.
4. **Size:** about 16,950 lines new or changed, tests included (brush about 6,750, text about 10,200; the table
   is in 5.1). The largest packages are T6 (text panels, 2,360 lines), S1 (core scaffold, 1,860) and T1 (text
   geometry, 1,550).
5. **One new dependency line, no new package:** `ttf-parser` 0.25.1 is already in `Cargo.lock` (`Cargo.lock:3540`)
   through `fontdue`; `crates\wte_core\Cargo.toml` does not name it today (its dependency list ends with `fontdue`,
   `fontdb`, `epaint_default_fonts`). S1 proves a real offline build before anything else starts.
6. **Project format:** `FORMAT_VERSION` goes from 1 (`core\project\mod.rs:19`) to 2, caused by the text feature
   alone. A project without a new text feature stays readable by the round-3 build. The brush changes nothing in
   `.wtep`.
7. **"Segmented" is decided** and is not asked again (section 4). Eight questions are open; each has a default.
   None blocks P0 or S1. Question 2 is needed before S2 starts and question 6 before wave 1 starts (section 4
   says what happens without an answer).
8. **Nothing the user asked for in words is cut without asking.** If the round must shrink, a cut that removes
   a requested phrase (for example shapes from model files) is put to the user as a question first (5.3).

---

## 1. Items

Each item says what the user gets, which design sections it implements, the files, and the acceptance tests. Test
ids (C1, F2, A5, S1, P3 ...) are the ids of BRUSH section 8; quoted names are the test names of TEXT section 8.
An item is done when its tests pass in the merged tree and the gate has read the screenshots.

### Shared

**R4-1 Tabs in the lower part of the side panel: Layers | Brush | Text**
- User gets: a row of three tabs where the "Layers" header is today. Brush can be opened while the Brush or Eraser
  tool is active, Text while the Text tool is active, or while the Transform tool is active and the selected stamp
  is a text (decision 2.2 item 10). A tab that cannot be used is grey and the area shows Layers. Changing the tool never changes which tab was last chosen. The colour picker stays fully
  visible at 1280 x 800.
- Design: BRUSH 6.1, 6.2; TEXT 6.1, 7.3.
- Files: `app\state.rs` (`SideTab` with three values, `side_tab`, `side_rects`), `app\panels\mod.rs`,
  `app\panels\layers.rs`.
- Acceptance: A11, A12 (BRUSH 8.2); `the_text_tab_follows_the_tool_and_the_selection`,
  `the_colour_section_stays_inside_the_panel_on_every_page` (TEXT 8.2); scenarios `brush_tab` and `text_tab`; one
  added cross-feature step in `text_tab` (section 3.3). The default height of the lower area is computed from the
  measured height of the Color section (BRUSH 6.1), not a fixed share.

### Brush

**R4-2 Brush type: Circular or Sphere, with Offset**
- User gets: a Type choice in the Brush tab. Circular paints exactly what the brush paints today. Sphere paints
  every texel inside the ball, on any side and any part, hidden or not. Offset lifts the ball off the surface or
  sinks it in. The checkbox "Same side only" (`app\panels\tool_options.rs:165`, setting at `app\state.rs:164`, used
  at `app\tools\brush.rs:69`) is removed; Type replaces it.
- Design: BRUSH 1.1, 1.4, 1.7, 4.1 to 4.3, 5.2 to 5.4, 5.9.
- Files: `core\brush_tip.rs` (new), `core\brush.rs`, `core\stroke.rs`, `app\tools\brush.rs`, `app\state.rs`.
- Acceptance: C1 (every existing brush test passes unedited, apart from the one `fx` line of 2.2 item 13), the save
  test `strokes_save_identically_through_both_entry_points` (BRUSH 8.5, written by B3), C3, C5, C9, C14, C19, C21, C22; A1, A2, A7; scenario
  `brush_types` (after the Circular click one cube face has 0 changed texels and each of the other five more than
  100,000; after the Sphere click all six have more than 100,000).

**R4-3 "Stay on connected surface" for Circular**
- User gets: a checkbox, off by default. When on, Circular paints only the piece of surface under the cursor, not
  nearby separate parts.
- Design: BRUSH 5.5.
- Files: `core\topology.rs` (new), `core\brush.rs`, `core\document.rs` (the table, built at first use).
- Acceptance: T1 to T4, C6, C7, C8, C25; A3; the TwoMaterial part of `brush_types`; P4 (at most 1.15 x the cost of
  Circular without the checkbox), P9 (first connected press at most 0.4 s and 70 MiB on the 1.5-million-triangle
  model).
- Dropped as a whole (package B2 and the flood code in B3) if the user answers question 3 with "no checkbox".

**R4-4 A shape instead of the ball: built-in Cube and Cylinder, orientation, Solid or Through**
- User gets: a Shape list (Ball, Cube, Cylinder, File). Turn, Tilt and Roll angles; Align "Follow surface" or
  "Fixed in space"; Use "Solid" (paint where the surface passes through the shape) or "Through" (the outline pressed
  straight through the surface). Hardness fades against the local thickness, so thin parts are painted as solidly as
  thick ones. Size stays the diameter of the ring; the shape is scaled to fit inside it. With the Brush or the
  Eraser, Ctrl+wheel changes Size and Shift+wheel turns the shape, as both do for stamps today.
- **Changes existing behaviour without having been asked for:** Ctrl+wheel and Shift+wheel zoom the view today
  with these two tools (BRUSH 1.5); the wheel without a key keeps zooming. Question 9 asks the user; the gate
  report names it as a deliberate change. Built by B7 alone (one `scroll` function), so a "no" removes it
  without touching another package.
- Design: BRUSH 1.2, 1.5, 5.2, 5.3, 5.6, 5.8.
- Files: `core\tip_field.rs` (new), `core\brush.rs`, `core\stroke.rs`, `app\tools\brush.rs`.
- Acceptance: F1, F13, F20; C10, C11, C12, C15 to C18, C20, C23, C24, C26; A6; P5 and P6 (Cube against the ball: at
  most 1.5 x at 50 texels, at most 3.0 x at 512 and 2048 texels; at most 6 ms per dab at 512 texels and 60 ms at
  2048 texels on the 8192 texture).

**R4-5 A shape loaded from a model file**
- User gets: "Load..." opens a file dialog for OBJ, glTF, GLB and FBX. The model is converted in a background job
  into a grid of at most 128 x 128 x 128 distance values (6 MiB with the thickness grid). Only the closed solid of
  the file is used and sets the size; a flat sheet in a main plane becomes an outline; a model that encloses
  nothing is refused with a message that says what to change. The path is remembered in `prefs.json` and the shape
  is read again, quietly, the first time it is needed after a start. "Remove file shape" forgets it.
- Design: BRUSH 4.2, 4.4, 5.7, 5.7b.
- Files: `core\tip_bake.rs` (new), `core\import\mod.rs`, `app\brush_flows.rs` (new), `app\prefs.rs`, `app\jobs.rs`,
  `app\action.rs`.
- Acceptance: F2 to F12, F14 to F19, C13; A9, A10, A16, A20; scenario `brush_shape`; P10 (load to ready: at most
  1.0 s for the cube, 8 s for the 500,000-face sphere, 2.0 s for a two-pass file; Cancel ends within 1 s).
- **Every format, not only OBJ** (BRUSH 8.1, added by the completeness check): F21 bakes the cube from a `.gltf`
  file, F22 from a `.glb`, F23 from an ASCII `.fbx` (also with the unit setting 100 and with a scaled model), each
  with the same inside fraction (within 2 %) and inradius (within one grid cell) as the OBJ cube; F24 bakes a GLB
  with a scaled and a turned node (the baked extents follow the node transform), a GLB with two overlapping
  cubes and one with two far cubes; F25 bakes one real `.glb`, `.gltf` and `.fbx` of this machine. A16 loads the
  cube in all four formats through the app's load flow; `brush_shape` loads and paints with the GLB and the FBX
  cube after the OBJ one. The test files are written by three new writers in `core\samples.rs` (S1). B1 writes
  F21 to F25, B5 the A16 part and the scenario.
- Never cut without the user's yes (5.3).

**R4-6 The Brush tab**
- User gets: the controls of R4-2 to R4-5 in one tab with a small preview picture of the tip that can be dragged to
  turn it, a Reset button, a line that names the layer that will be painted, and a note that the 2D view always
  paints a flat disc.
- Design: BRUSH 1.3, 1.4, 6.3, 6.4.
- Files: `app\panels\brush_tab.rs` (new), `app\panels\tool_options.rs`, `app\control_sweep\panel_tests.rs`.
- Acceptance: A11, A12, A13; scenario `brush_tab`; P11 (side panel with the tab idle at most 1.5 ms per frame).

**R4-7 Brush cursor and hints**
- User gets: the ring of today, now hidden on faces Circular will skip; for a shape a bold outline of its footprint
  and three thin cross-sections; the status bar names the type while the pointer is over the model.
- Design: BRUSH 5.11, 6.5, 6.6.
- Files: `app\gpu\renderer.rs`, `app\gpu\shaders\mesh.wgsl`, `app\gpu\params.rs`, `app\tools\brush.rs`.
- Acceptance: A8, A14, A15, A18; the screenshots of `brush_shape` and `brush_tab`; P11 (overlays plus side panel at
  most 1 ms above their values without a shape).

**R4-8 Strokes that keep the window responsive**
- User gets: no part of a fast stroke is skipped or lands in the wrong place when the camera moves. Stroke work
  takes 10 ms per frame while it keeps up and up to 100 ms when more than 250 ms of work waits. A stroke that still
  trails is finished, visibly, in the frames after the release. Only Undo and Esc throw it away; every other
  command first completes it.
- Design: BRUSH 5.10.
- Files: `app\tools\brush.rs`, `app\tools\mod.rs` (`end_frame` at `:325`, `press` at `:351`, `release` at `:409`,
  `cancel` at `:431` today), `app\view\mod.rs`, `app\dispatch.rs`, `app\selftest\mod.rs` (an input step ends only
  when the tools are idle).
- Acceptance: A4, A5, A17, A19; P8a (no frame does more than 100 ms plus one sample of stroke work, the release
  frame included), P8b (release to end of stroke: at most 400 ms at Size 50, 1,000 ms at Size 512), P8c, P8d;
  each on the 8192 texture, the 4096 texture and the 1.5-million-triangle model.
- Known side effect: a scenario that asserts an exact hash after a long drag may get a new expected value, because
  the new walk follows the whole pointer path (BRUSH 8.5). The gate lists every such scenario.

**R4-9 One search per dab instead of one per texture set, measured first**
- User gets: the same pixels, faster on models with several texture sets; an honest statement of the speed on the
  1.5-million-triangle model.
- Design: BRUSH 5.4, 5.12, 8.4.
- Files: `core\bvh.rs`, `core\brush.rs`, `core\stroke.rs`.
- Acceptance: C2, C4; P1 (at most 1.05 x baseline on the 8192 and on the 4096 texture), P13 (an absolute line
  for the 1.5-million-triangle model: the median painting frame at Size 50 with Circular and with Sphere is at
  most the baseline median, both printed in milliseconds; on the two texture models at most 16.7 ms or the
  baseline median), P2 (query phase at most 0.35 x baseline on
  the 1.5-million-triangle model), P3 (total at most 1.00 x, or 0.85 x when the baseline shows that queries are at
  least 30 % of the frame), P7, P12 (every frame over 100 ms listed). Said plainly: on the large model the limits promise "not slower than
  today" and a responsive window through pacing (R4-8), not a faster brush; P13 makes a miss visible as a number.

### Text

**R4-10 Spacing between single characters ("segmented")**
- User gets: a strip in the Text tab that shows the characters of the selected text with a clickable gap between
  each two; a value field, nudge buttons, "reset this gap" and "reset all". The value is a fraction of the text
  size and adds to Letter spacing and to the kerning of the font. Values stay with their characters when the text
  is edited; Ctrl+Z in the content field brings them back. A spacing change keeps the alignment side of the text in
  place (this also changes today's Letter spacing behaviour for left- and right-aligned placed texts).
- Design: TEXT 4.1 (`TextSpec.fx.gaps`; `TextSpec` today ends with `letter_spacing` and `outline`,
  `core\text\mod.rs:61-75`), 5.2, 5.11, 5.12, 6.2.
- Files: `core\text\mod.rs`, `core\text\gaps.rs` (new), `core\text\layout.rs`, `app\panels\text_block.rs` (new),
  `app\panels\text_tab.rs` (new).
- Acceptance: `one_gap_moves_exactly_the_glyphs_after_it` and the other layout tests of TEXT 8.1 "Gap spacing"; the
  tests of "Gap values and edits"; `plain_text_is_bit_identical` (a text without gap values gives today's pixels);
  `the_gap_buffer_sees_every_edit_of_the_field`, `field_undo_brings_the_gap_back`,
  `the_strip_lays_out_only_visible_cells`; scenario `text_gaps` (bounds wider by 0.5 em within 2 texels, left edge
  moved by less than 1 texel, one undo entry "Edit Stamp"); "WTE 2026" with 3 gap values within 5 % of baseline B1.
- This item is never cut. It needs only S1, S2, the gap parts of T1, T2 and T6, and T5.

**R4-11 Font picker**
- User gets: one button that opens a list of every font family of the machine with a search field, a sample of each
  family in its own face, every face of a family, and an order that ignores upper and lower case. A font that
  exists only inside the open project can be picked too (the project is searched first, then the machine).
- Design: TEXT 3.1, 5.13, 6.3.
- Files: `core\text\font_catalog.rs`, `core\blob.rs`, `app\panels\font_picker.rs` (new), `app\text_ops.rs` (new).
- Acceptance: the "Catalog" tests of TEXT 8.1; the picker tests of TEXT 8.2; scenario `text_fonts`, which now
  has two asserts that are never skipped (the catalog holds at least 50 families; Arial is among them; the
  round-3 run logged 78 families, `target\round3_audit\runs\text_builtin\shot.log:26`); sample job for
  all families at most 0.5 s with a warm file cache; no frame over 16 ms in the side panel with the picker open;
  the font scan at start at most 0.05 s slower than in round 3.

**R4-12 Bold and slant**
- User gets: B and I buttons, a Thicken value and a Slant angle. B and I take the real bold or italic face when the
  family has one and thicken or slant the letters only when it has none. Both apply to the whole text.
- Design: TEXT 5.3, 5.4, 5.7, 5.9, 5.10.
- Files: `core\text\outline.rs`, `scan.rs`, `styled.rs` (all new), `core\text\raster.rs`,
  `core\text\font_catalog.rs` (`face_traits`), `app\panels\text_block.rs`.
- Acceptance: the "Rasterizer and outlines" and "Painters" tests of TEXT 8.1; scenario `text_styles`; the panel
  sweep `every_text_control_changes_exactly_its_own_field` (TEXT 8.2; it operates B, I, Thicken, Slant and every
  control of the Warp and Path pages and checks that only the control's own field changes, with one undo entry;
  it is also acceptance for R4-13 and R4-14); in `text_fonts`: Arial gets the face "Bold" and no thickening, Ubuntu gets a thickening of 0.02 em.

**R4-13 Warp**
- User gets: the 15 style names of Photoshop's Warp Text with Bend, Horizontal, Vertical and a direction switch. The
  letter shapes bend. The formulas are our own.
- Design: TEXT 5.6.
- Files: `core\text\warp.rs` (new), `core\text\styled.rs`, `app\panels\text_tab.rs`.
- Acceptance: the "Warp" tests of TEXT 8.1, among them `no_style_folds_over`; scenario `text_warp` (bounds change
  with each style, the anchor moves by less than 0.5 texel); final picture of "WTE 2026" with Arc 50 % at 512 px
  per em at most 40 ms; every `Final` step of `text_warp` on the 1.5-million-triangle model at most 1,500 ms.

**R4-14 Text along a line: Arc, Circle, Curve**
- User gets: a Path page. Arc has one Bend slider. Circle has a fixed diameter, starts centred on its top, and shows
  a hint with a Fit button when the text is longer than the circle. Curve runs through 2 to 16 points that are
  dragged in the view. Letters keep their shape and turn with the line.
- Design: TEXT 5.5, 6.4, 6.5.
- Files: `core\text\path.rs` (new), `core\text\styled.rs`, `app\panels\text_tab.rs`, `app\tools\text_path.rs` (new),
  `app\tools\transform.rs`.
- Acceptance: the "Line" tests of TEXT 8.1; `esc_restores_the_curve`; scenario `text_path` (one history entry per
  point drag; the first and last point move by less than 1.5 pt on screen); the Circle `Final` on the
  1.5-million-triangle model at most 1,500 ms.

**R4-15 Text options in four rows, the Text tab, the Angle field**
- User gets: the Text options shrink to four rows (content; font and style; B, I, size, Angle, colour; the buttons
  Warp, Path, More). Everything else is in the Text tab with the pages Character, Warp and Path. "Angle" shows the
  rotation of the whole text, which exists today in four other places. The Text tool always edits the text to
  place; the Transform tool edits the selected text.
- Design: TEXT 1.2, 6.1, 6.2, 6.4.
- Files: `app\panels\text_block.rs`, `app\panels\text_tab.rs` (T6). `app\panels\tool_options.rs` is edited for
  this item by S2 only (the call order of TEXT 6.1 and the move of the text functions); T6 does not touch it
  (2.4).
- Acceptance: `text_rows_fit_the_panel` and the other tests of TEXT 8.2; `the_angle_field_turns_the_right_text`
  (with the Text tool a typed 30 changes only the rotation of the text to place and queues nothing; with the
  Transform tool it sends one placement change for the selected text and makes one undo entry; the existing
  Rotation field and the gizmo knob show the same value; 270 becomes -90); scenario `text_tab`; the existing scenario
  assert "the Text tool options leave the 3D view where it was" still passes; the Character page with 1,000
  characters: no frame over 16 ms in the side panel.

**R4-16 The text stays where it is; bounded cost while editing**
- User gets: when a text changes shape (warp, line, bold, a new resolution) its anchor point and the size of one em
  stay fixed on the model. While a control is dragged a styled text is drawn with at most 1 megapixel, at the end
  once in full; nothing moves when the final picture replaces the preview. A styled picture has at most
  16.8 megapixels; path coordinates are limited to 60 em.
- Design: TEXT 5.8, 5.10, 5.11.
- Files: `core\text\frame.rs` (new), `core\doc_stamps.rs`, `core\document.rs`, `core\doc_build.rs`.
- Acceptance: the "Frame" and "Document" tests of TEXT 8.1, among them
  `editing_a_plain_text_gives_the_round_3_placement`; preview picture of "WTE 2026" with Arc 50 % at most 8 ms;
  `set_stamp_source(.., Preview)` for 40 styled letters at most 16 ms; twenty `Preview` edits on an 8192 texture:
  longest frame at most 30 ms plus the round-3 preview-bake time; peak working memory of the largest final picture
  with an outline at most 250 MB.

**R4-17 The text is visible before it is placed**
- User gets: with the Text tool, a see-through picture of the text follows the pointer inside the yellow frame of
  today.
- Design: TEXT 5.14, 6.5.
- Files: `app\tools\stamp_place.rs`.
- Acceptance: three unit tests in `stamp_place.rs` (TEXT 8.2): `hover_with_the_text_tool_caches_a_preview_of_the_draft`
  (a picture of at most 262,144 pixels whose spec equals the text to place), `a_changed_draft_replaces_the_hover_preview`,
  `add_stamp_reuses_only_a_full_preview`. In scenario `text_tab`: one `Step::Hover` with the Text tool, the assert
  that the tool holds a preview of the draft, and a screenshot of that frame. First on the cut line (section 5.3).

**R4-18 Project format version 2**
- User gets: texts with the new features save and reopen; old projects open unchanged; a project that uses no new
  text feature can still be opened by the round-3 build; a newer project with an unknown warp or path name opens
  with a warning instead of failing.
- Design: TEXT 4.3, 4.4.
- Files: `core\project\manifest.rs`, `mod.rs`, `migrate.rs`, `save.rs`, `load.rs`, `core\error.rs`
  (`LoadWarning::TextFeatureUnknown`), `crates\wte_core\tests\project_roundtrip.rs`.
- Acceptance: the "Project" tests of TEXT 8.1 (`required_reader`, round trips, the second frozen fixture);
  `frozen_v1_text_fixture_loads`, which S1 adds and runs green **before** it changes the manifest (2.5);
  scenarios `roundtrip` and `export_run` extended.
- The brush check of BRUSH 8.5 is not part of this item any more: it is test
  `strokes_save_identically_through_both_entry_points`, written by B3 in `crates\wte_core\tests\brush_tips.rs`
  (a document painted through the old and through the new entry point saves to byte-identical files whose
  `min_reader_version` is 1). It is listed under R4-2.

### Gate

**R4-19 Merge, scenarios, measurements, architecture, report** — section 3.

---

## 2. Packages

### 2.1 Names

The two design documents use overlapping letters (both have an "A" and a "G"). This plan renames:

| Plan name | Name in the design | Content |
|---|---|---|
| P0 | BRUSH "step 0", TEXT 7.5 gates 2, 3, 5 | baseline and rebase check, no feature code |
| S1 | core half of BRUSH "scaffold S" and of TEXT "S0" | shared core contracts |
| S2 | app half of both | shared app contracts, the tab strip |
| B1 | BRUSH A core-bake | model to distance grid |
| B2 | BRUSH C core-topology | neighbour table |
| B3 | BRUSH E core-paint | the paint engine |
| B4 | BRUSH D1 app-panels | Brush tab, panel layout |
| B5 | BRUSH D2 app-flows | load flow, preferences |
| B6 | BRUSH G app-gpu | ring side filter |
| B7 | BRUSH F app-tools | brush tool, pacing, cursor |
| T1 to T7 | TEXT T1 to T7 | unchanged |
| Z | BRUSH "Gate", TEXT "G shared gate" | integration gate |

### 2.2 Decisions where the two designs disagree or leave a gap

1. **The tab strip is built once, by S2**, with three tabs. BRUSH still lists "tab strip" under D1 and defines
   `SideTab { Layers, Brush }` (BRUSH 4.2, 7.3). For this round that row reads: S2 builds the strip, the enable
   conditions, the one tab rule, the container and `side_rects`; `SideTab` is `{ Layers, Brush, Text }`; B4 fills
   its own tab function in `brush_tab.rs`. This is the edit TEXT 7.3 asks for.
2. **Panel files, one owner each.** After S2: T6 owns `app\panels\text_block.rs`, `text_tab.rs`, `font_picker.rs`
   and `app\control_sweep\text_panel_tests.rs`. B4 owns every other file of `app\panels\` and
   `app\control_sweep\panel_tests.rs` and `app\control_sweep.rs`. B6 owns `app\control_sweep\gpu_tests.rs`.
3. **The height of the lower area** is the computed one of BRUSH 6.1 (`panel height - Color height - 144 - 4`,
   limited to 160..420 points). TEXT 6.1 mentions a 37 % share as a possibility; it is not used. Both tabs fit: the
   Brush tab needs at most 250 points, the tallest Text page 218 points plus its 24-point page row (design).
4. **The scaffold is two packages in sequence** (S1 core, then S2 app). The designs describe one; together it
   would be about 2,450 lines in 45 files, too much for one agent to hold. S2 compiles against the types S1 wrote.
5. **`core\document.rs` and `core\doc_build.rs`**: S1 writes the brush's topology field, its accessor and its
   initial value, and the stub fields and functions of the text design. From then on both files belong to T4. B2
   and B3 do not edit them.
6. **`app\selftest\*`**: S2 makes the two runner changes: an input step ends only when the tools are idle (BRUSH
   7.3), and the step `Step::Hover { view, at }` (about 15 lines, BRUSH 7.3 "Gate"; moved from Z to S2 so that
   wave-2 packages can run scenarios that hover). After that the folder belongs to Z in the merged tree: no
   package's copy of `scenarios.rs` is ever copied or merged (this is stricter than round 3, where any package
   could edit the file, because 14 packages would otherwise all merge into it). How the scenarios are still
   written and run early is item 11.
7. **`ARCHITECTURE.md`** belongs to Z. Every package writes the new text for its sections as a file
   `target\round4\notes\arch_<package>.md` (section number, replaced text, new text). Z applies them. The sections
   are listed in BRUSH 4.7 and TEXT 7.7; both change 2.1, 2.2, 5.14, 8.2, 8.6, 8.7, 8.9, 8.11 and 9, which is why
   one owner applies them.
8. **`TextSpec` literals.** TEXT 7.1 counts 11 places that need one more line. A search for `TextSpec {` on
   2026-11-17 finds 18 occurrences besides the definition, in 10 files (`core\doc_stamps.rs` 3,
   `core\project\manifest.rs` 3, `tests\project_roundtrip.rs` 3, `core\text\layout.rs` 2, `tests\end_to_end.rs` 2,
   `core\history.rs`, `app\panels\layers.rs`, `app\panels\tool_options.rs`, `app\tools\stamp_place.rs`,
   `app\control_sweep\panel_tests.rs` 1 each); some may be patterns. S1 and S2 re-count in the round-3 tree and
   adjust every one.
9. **Scenario count.** The count test says 24 today (`app\selftest\scenarios.rs:3063`). Round 4 adds ten names
   (four brush, six text), so the number becomes the round-3 count plus ten.
10. **The tab rule, one rule, decided here before S2 starts.** The Brush tab is enabled when the tool is Brush or
    Eraser. **The Text tab is enabled when the tool is Text, or when the tool is Transform and the selected stamp
    is a text.** In every other case the tab is grey and the area shows Layers, whatever `side_tab` says and
    whatever is selected. `SetTool` writes neither `side_tab` nor the selection (`app\dispatch.rs:631-635` only
    cancels the running tool and sets `state.tool`; it stays as it is). The first version of TEXT 6.1 enabled the
    Text tab "while a text stamp is selected"; with a selection that survives a tool change this contradicted
    its own test. TEXT 6.1, 7.3 and 8.2 now state the rule above, which is also row 3 of the target table in TEXT
    6.2 ("any other tool: the Text tab is not enabled"). The other way out, clearing the selection on a tool
    change, was not taken: it would change `SetTool`, break the "Selected: ... Edit" row
    (`app\panels\tool_options.rs:91-101`) and undo the round-3 behaviour around the selected text.
11. **Every scenario has an author package that writes and first runs it.** The author adds the scenario to
    `scenarios.rs` **in its own tree**, runs it there, and delivers the function as the file
    `target\round4\notes\scenario_<name>.rs` together with the log of its run. Its copy of `scenarios.rs` is
    thrown away; Z pastes the delivered functions into the merged file and registers the names (sitting M2,
    3.3). A scenario failure therefore shows up inside the package that can fix it, not at the very end.

    | Scenario | Author | What can run in the author's own tree | What runs first at M2 |
    |---|---|---|---|
    | `brush_types`, `brush_types_path` | B7 | everything (the engine comes with `base4_core`; the setup writes the settings directly) | — |
    | `brush_shape` | B5 | the three loads and their asserts | the click and stroke asserts, which need B7's tool |
    | `brush_tab` | B4 | the panel asserts and the tab rule | the cursor assert, which needs B7 |
    | `text_gaps`, `text_styles`, `text_warp`, `text_path`; the added steps of `roundtrip`, `export_run`, `fuzz_path` | T7 | everything (they send actions; the painters come with `base4_core`) | — |
    | `text_tab` | T6 | every page, the colour section, the cross-feature step of 3.3 | the hover assert, which needs T7 |
    | `text_fonts` | T6 | everything | — |

    The re-recording of exact hashes after long drags (R4-8) is done by B7 in its own tree, with the list of
    scenarios and old and new values in its note; Z repeats the run on the merged tree and compares.
12. **Text edits in `app\panels\tool_options.rs`.** The file belongs to B4 after S2, but TEXT 6.1 needs text
    edits in it. S2 makes all of them: the text functions move to `text_block.rs` (among them `text_options`,
    which holds the "Font not installed" line of `tool_options.rs:220-222`, so that line leaves the file with
    it); the call order becomes "Text block first, then the wrong-view hint, then the 'Selected: ... Edit' row"
    (`tool_options.rs:91-101`), and under the Transform tool the Text block comes before Name, Tint and the
    placement fields. What stays in `tool_options.rs` is two calls with fixed signatures,
    `text_block::target(state)` and `text_block::show(ui, state, ps, target, actions)`. T6 changes only the
    bodies behind them. If T6 finds that it needs another call site, that is a contract request (2.5 item 3).
13. **One-line `TextSpec` edits are an allowed exception** to "files nobody touches" and to brush test C1: S1
    adds `fx: TextFx::default()` to the literal in the test of `core\history.rs:1537` and to the literals in
    `crates\wte_core\tests\end_to_end.rs` and `project_roundtrip.rs`. No other line of those files and no
    expected value changes in S1. TEXT 7.1 and BRUSH 8.1 (C1) say the same.

### 2.3 Order

```
round 3 merged into the main tree
   |
P0  baseline + rebase check                      (no feature code; machine idle)
   |
S1  core scaffold                                (alone)
   |
S2  app scaffold                                 (alone)      -> snapshot "base4"
   |
wave 1, core, in parallel:   B1  B2  B3  T1  T2  T3  T4  T5   (build and test only -p wte_core)
   |
M1  merge of wave 1 onto base4, core tests green -> snapshot "base4_core"      (done by Z's agent)
   |
wave 2, app, in parallel:    B4  B5  B6  B7  T6  T7           (start from base4_core)
   |
Z   merge, scenarios, measurements, ARCHITECTURE.md, report
```

Why two waves instead of one: the app packages compile against stubs, so all 14 could run at once, but (a) B7 needs
the real engine of B3 for tests A2 and A3, B5 must be merged after B1, T7 needs the real `text_preview` of T4 to
show a picture; (b) wave 1 builds only `wte_core`, which is quick, while 14 full app builds at once on a machine
with 12 logical processors would make every package slow and every timing useless; (c) the middle merge finds core
conflicts before the app packages build on them. T2 finishes after T1 inside wave 1 (it starts on the stubs).

A wave-2 package may be started early on `base4` when the machine is free; it then re-bases onto `base4_core` by
copying in the core files, which it does not own.

### 2.4 Ownership

"Owns" means: only this package changes the file after the scaffold. Every file not in this table has no owner and
is changed only as a listed cross-package edit (rule of `target\round3\ROUND3_BUILD_RULES.md`, section 5: minimal,
local, listed in the note with the reason, merged three-way).

**S1 core scaffold** (writes, then hands over where a later owner is named)

| Files | What S1 writes | Later owner |
|---|---|---|
| `Cargo.toml`, `crates\wte_core\Cargo.toml`, `Cargo.lock` | the `ttf-parser` line of TEXT 5.1; first task: a real offline build and the `wte_core` test suite with it | none |
| `core\lib.rs`, `core\error.rs` | module lines and re-exports for both features; `LoadWarning::TextFeatureUnknown` | none |
| `core\brush_tip.rs`, `core\tip_field.rs`, `core\samples.rs` | complete, with tests (BRUSH 4.1, 5.8, the generators of BRUSH 8 and the three file writers `to_gltf`, `to_glb`, `to_fbx_ascii`, each checked by importing what it wrote) | none |
| `core\tip_bake.rs` | `TipError`, the two `bake` signatures returning `Err(NoInterior)` | B1 |
| `core\topology.rs` | public signatures and a correct serial `build` | B2 |
| `core\stroke.rs` | `stroke_begin_with`, `stroke_tip_pose` (behaving as `stroke_begin` for a ball) | B3 |
| `core\text\mod.rs` | the types and functions of TEXT 4.1 with their tests, `sanitize_fx` | none |
| `core\text\outline.rs`, `scan.rs`, `warp.rs`, `path.rs`, `gaps.rs` | compiling stubs with final signatures | T1 |
| `core\text\styled.rs`, `frame.rs` | stubs (`render_text` = today's painter, centre as anchor) | T2 |
| `crates\wte_core\tests\project_roundtrip.rs` | **first project task:** the test `frozen_v1_text_fixture_loads` (TEXT 4.3 item 5), run green on the still unchanged manifest code; its output goes into S1's note | T5 |
| `core\project\manifest.rs`, `mod.rs`, `migrate.rs` | only after that test is green: `fx` in the text entry, version 2 with the empty step; the fixture test must still pass | none |
| `core\document.rs`, `core\doc_build.rs`, `core\doc_stamps.rs` | topology field and accessor; stub text fields and document functions | T4 |
| every core and core-test file with a `TextSpec` literal, `core\history.rs:1537` included | one line each (2.2 item 13) | their owners; `history.rs` keeps having none |

**S2 app scaffold**

| Files | What S2 writes | Later owner |
|---|---|---|
| `app\state.rs`, `app\action.rs` | all new types, settings and actions of BRUSH 4.2 and TEXT 4.2. Both files say in their header that only the scaffold writes them (`app\state.rs:3`, `app\action.rs:4`) | none |
| `app\dispatch.rs`, `app\io_flows.rs`, `app\app.rs`, `app\jobs.rs` | one-line arms into `brush_flows.rs` and `text_ops.rs`; the guard entry; `discard` for Undo and Esc; `sync_prefs` before both preference writes | none |
| `app\tools\mod.rs`, `app\view\mod.rs` | the controller side of the pacing (BRUSH 5.10): `end_frame` pumps the brush, `release` keeps a finishing stroke, `discard`, `finishing` | none |
| `app\gpu\params.rs`, `app\gpu\mod.rs` | `BrushRing::side_normal`, with `None` at today's three literals (`app\gpu\mod.rs:248`, `app\gpu\renderer.rs:861`, `app\tools\brush.rs:299`) | none |
| `app\panels\mod.rs`, `app\panels\layers.rs` | the three-tab strip, tab rule, container, `side_rects`, the computed height | B4 |
| `app\panels\tool_options.rs`, `app\control_sweep.rs`, `app\control_sweep\panel_tests.rs` | checkbox removed with its tests; text functions moved out; target rule of TEXT 6.2; **every text-related edit that stays in `tool_options.rs`** (the call order of TEXT 6.1 and the two fixed calls into `text_block.rs`, 2.2 item 12) | B4 |
| `app\panels\text_block.rs` | the text option functions of today, moved unchanged, with `TextPanelState`; `target(state)` and `show(..)` with their final signatures; the recording part of `GapBuffer` that the two proofs need (about 60 lines) | T6 |
| `app\panels\brush_tab.rs` | stub (an empty tab with its name) | B4 |
| `app\panels\text_tab.rs`, `font_picker.rs` | stubs | T6 |
| `app\control_sweep\text_panel_tests.rs` | the two proofs of the egui behaviour, `the_gap_buffer_sees_every_edit_of_the_field` and `the_field_reports_its_cursor_while_focused` (TEXT 6.2 rule 6, 8.2), written and run **before** `TextPanelState` and `text_gap` are fixed. If one fails, S2 switches the contract to the designed fallback (text comparison, gaps selected in the strip only), writes that into its note, and T1 builds `gaps.rs` without the field history | T6 |
| `app\brush_flows.rs`, `app\prefs.rs` | functions with empty bodies; `..Prefs::default()` at the literals | B5 |
| `app\text_ops.rs`, `app\tools\text_path.rs`, `app\tools\stamp_place.rs`, `app\tools\transform.rs` | stubs; the `TextSpec` line | T7 |
| `app\tools\brush.rs` | `pump` that does nothing, `finishing` that returns false, the adjusted literals | B7 |
| `app\selftest\mod.rs` | input steps end when the tools are idle; `Step::Hover` | Z |

After S2 the workspace builds, every existing test passes, clippy is clean, and the app behaves as after round 3
with two visible differences: the tab strip exists with two empty tabs, and "Same side only" is gone.

**Wave 1 — core**

| Package | Items | Owns |
|---|---|---|
| B1 brush bake | R4-5 (with the format tests F21 to F25) | `core\tip_bake.rs`, `core\import\mod.rs` |
| B2 brush topology | R4-3 | `core\topology.rs` |
| B3 brush paint | R4-2, R4-3, R4-4, R4-9; the save test of BRUSH 8.5 | `core\brush.rs`, `core\stroke.rs`, `core\bvh.rs`, `crates\wte_core\tests\brush_tips.rs` |
| T1 text geometry | R4-10, R4-12, R4-13, R4-14 | `core\text\outline.rs`, `scan.rs`, `warp.rs`, `path.rs`, `gaps.rs` |
| T2 text painters | R4-10, R4-12, R4-13, R4-14, R4-16 | `core\text\layout.rs`, `raster.rs`, `styled.rs`, `frame.rs` |
| T3 fonts | R4-11, R4-12 | `core\text\font_catalog.rs`, `core\blob.rs` |
| T4 text document | R4-16 | `core\doc_stamps.rs`, `core\document.rs`, `core\doc_build.rs`, `crates\wte_core\tests\end_to_end.rs` |
| T5 text project | R4-18 | `core\project\save.rs`, `load.rs`, `crates\wte_core\tests\project_roundtrip.rs` |

**Wave 2 — app**

| Package | Items | Owns |
|---|---|---|
| B4 brush panels | R4-1, R4-6; scenario `brush_tab` | `app\panels\*` except T6's three files; `app\control_sweep.rs`, `app\control_sweep\panel_tests.rs` |
| B5 brush flows | R4-5; scenario `brush_shape` | `app\brush_flows.rs`, `app\prefs.rs` |
| B6 brush gpu | R4-7 | `app\gpu\renderer.rs`, `app\gpu\shaders\mesh.wgsl`, `app\control_sweep\gpu_tests.rs` |
| B7 brush tool | R4-2, R4-3, R4-4 (with the wheel gestures of BRUSH 1.5), R4-7, R4-8; scenarios `brush_types`, `brush_types_path` | `app\tools\brush.rs` |
| T6 text panels | R4-10 to R4-15; scenarios `text_tab`, `text_fonts` | `app\panels\text_block.rs`, `text_tab.rs`, `font_picker.rs`, `app\control_sweep\text_panel_tests.rs` |
| T7 text tools | R4-11, R4-14, R4-17; scenarios `text_gaps`, `text_styles`, `text_warp`, `text_path` and the added steps of `roundtrip`, `export_run`, `fuzz_path` | `app\tools\stamp_place.rs`, `transform.rs`, `text_path.rs`, `app\text_ops.rs` |

A scenario in this table is delivered as a note file, not as an edit of `scenarios.rs` in the merged tree (2.2
item 11).

**Z gate** owns `app\selftest\*`, `ARCHITECTURE.md`, `target\round4\*`, the merged copy and the release build.

Files both designs promise not to touch, and that therefore have no owner: `core\stamp_bake.rs`, `texel_map.rs`,
`stamp.rs`, `history.rs`, `doc_layers.rs`, `core\export\*`, `core\text\edt.rs`, `app\view\gizmo.rs`,
`app\shortcuts.rs`, `app\toasts.rs`, `app\dialogs\*`. An edit there is a sign that a package left its design; the
gate reviews each one. One edit there is planned and allowed: the single `fx` line S1 adds to the `TextSpec`
literal in the test at `core\history.rs:1537` (2.2 item 13).

### 2.5 Order in which shared types land

1. S1, in this order inside the package: the dependency line and its build proof; the frozen version-1 text
   fixture `frozen_v1_text_fixture_loads`, green on the unchanged project code (it is the safety net for the
   step to version 2, so it must exist before the manifest changes); `core\text\mod.rs`
   (`TextFx`, `Warp`, `TextPath`, gap list) and `core\brush_tip.rs` (`BrushTip`, reach, `TipPose`); `tip_field.rs`;
   the manifest entry and version 2; the document fields; all stubs; the literals.
2. S2: the two proofs of the egui behaviour first (they decide the shape of `TextPanelState` and `text_gap`,
   2.4); then `state.rs` and `action.rs` (they need the S1 types), then `jobs.rs`, `dispatch.rs`, `io_flows.rs`,
   `app.rs`, then the controller files, then the panel container, then the stubs.
3. A package that finds a contract wrong or missing does not change the scaffold file on its own. It writes the
   needed change into `target\round4\notes\contract_requests.md`; the agent that runs the round applies it to the
   base snapshot and tells the running packages. A purely additive field with a default may instead be made as a
   listed cross-package edit.
4. Merge order inside a wave does not matter for owned files (they are copied). Listed cross-package edits are
   merged three-way against the wave's base; B5 is merged after B1 and B7 after B3, as BRUSH 7.3 asks.

### 2.6 What must wait for the end of round 3

Waits until round 3 is merged into the main tree and its gate report exists:

- The snapshot the round-4 copies are made from. Round 3 owns, until then, every file S1 and S2 edit outside
  `core\brush.rs`, `stroke.rs`, `bvh.rs` and `core\text\*` (`target\round3\ROUND3_BUILD_RULES.md`, section 5).
- P0 as a whole: its numbers must come from the code round 4 starts from.
- Re-reading every `file:line` reference of both designs; they are from the tree before round 3.
- Five facts that only the round-3 result can give, each needed by a named package:

| Fact | Round-3 item | Needed by |
|---|---|---|
| The visible height of the tool options and the height of the Color section at 1280 x 800; which rectangles the panel already records for the Color assert | R3-10 and user decision 2 | S2, B4, T6 |
| How the text picture and the tint coverage are cached per stamp; the preview-bake throttle; the rule for the depth fit while the pointer is down | R3-4 | T4, T7 |
| Where a text stamp is renamed inside the source update | R3-9 | T4 |
| The load-warning list after the changes to old-layout and informational warnings | R3-1, R3-3 | S1, T5 |
| The scenario count, the test count (670 before round 3 per the build rules), the limits round 3 reached, and whether `target\round3\check_exports.py` exists | round-3 gate | Z |

Does not wait:

- Putting the questions of section 4 to the user. They are asked now. P0 and S1 start without the answers; S2
  needs the answer to question 2 and wave 1 the answer to question 6 (section 4).
- Writing `target\round4\ROUND4_BUILD_RULES.md` (round 3's rules with the tables of section 2.4 of this plan and
  the stricter rules for `scenarios.rs` and `ARCHITECTURE.md`).
- Nothing else. In particular no package may start "on the round-2 tree to save time": S2 rewrites the panel
  container that R3-10 is changing right now.

---

## 3. The gate (package Z)

Z works in three sittings: P0 before the scaffold, M1 between the waves, the gate proper at the end.

### 3.1 P0 — before any feature code

1. Check every `file:line` reference of the sections the scaffold needs against the round-3 tree; write the changed
   ones to `target\round4\notes\rebase.md`.
2. Brush baseline (BRUSH 8.4 step 0): `workflow_path` on the five performance models (ABeautifulGame.glb,
   tex_4096.obj, tex_8192.obj, mesh_1m.glb, mesh_3m.glb); one instrumented throw-away copy that splits the stroke
   frame into its phases; the ignored test `stroke_bench_tips` with today's brush at 50, 512 and 2048 texels on
   the 8192 texture, the 4096 texture and ABeautifulGame. Output: `target\round4\perf\baseline.txt`, with the P3
   target and the two P13 medians (painting frames at Size 50 on ABeautifulGame, "Same side only" on and off)
   written out as numbers in milliseconds.
3. Text baseline (TEXT 7.5 gate 5): the eight FNV-1a hashes of plain texts, the placement after one content edit of
   a projected and of a UV text, the times B1 to B4. Output: `target\round4\perf\text_baseline.txt`.
4. Measure the two heights of section 2.6 on a screenshot of the `text` and `paint3d` scenarios at 1280 x 800.
   If fewer than 116 points are visible for tool options, T6 builds the content field with one text row.
5. Three checks of things this plan depends on and did not know:
   - **The export check script.** `export_run` needs the Pillow check of round 3. A search of `target\` on
     2026-11-17 found no `check_exports.py` (round 3 was still running). P0 finds the script the round-3 gate
     used and writes its path into `rebase.md`; if there is none, the script becomes a task of Z (about 60
     lines of Python, outside the Rust build) and is listed in 5.1.
   - **Disk space.** The size of one full target folder of a package copy is measured (it was not measured for
     this plan; one attempt on a round-3 copy did not finish in 100 s). Needed: that size times 17 package
     copies, plus 20 %. E: had 745 GB free on 2026-11-17. If the need is larger than what is free, the copies of
     finished packages are deleted before the next wave starts, and the plan says in which order.
   - **The model sizes behind the limits.** `target\compat\harness\shapes\high_poly_sphere.obj` has 500,000
     faces (its `f` lines were counted on 2026-11-17), so the 8 s limit of P10 stands. P0 also writes the triangle
     counts of the five performance models and the paths of the three real files of test F25 into
     `baseline.txt`.

The frozen version-1 text fixture is **not** a P0 task (an earlier version of risk 10 said so): P0 merges no line.
It is the first project task of S1 (2.5).

### 3.2 M1 — between the waves

Copy the owned files of the eight wave-1 packages onto `base4`, merge the listed cross-package edits three-way,
then: `cargo test -p wte_core --offline`, clippy on the workspace, and the app test suite (it must still pass on
the stubs of wave 2). Run `stroke_bench_tips` and the text timing tests once, so that a missed core limit is known
before the app packages finish. Output: the snapshot `base4_core` and a note per conflict.

### 3.3 The gate proper

**Sitting M2, first thing after the wave-2 merge and before any measurement:** Z pastes the delivered scenario
functions (`target\round4\notes\scenario_<name>.rs`, 2.2 item 11) into `scenarios.rs`, registers the ten names,
updates the count test, and runs every scenario once. A scenario that passed in its author's tree and fails here
points at a cross-package problem; the parts listed in the last column of the table in 2.2 item 11 run here for
the first time.

**Scenarios to add** (ten names; steps and asserts as written in BRUSH 8.3 and TEXT 8.3; the author of each is in
2.2 item 11):

| Scenario | Proves |
|---|---|
| `brush_types` | Circular against Sphere on the cube; the connected checkbox on TwoMaterial; undo |
| `brush_shape` | loading an OBJ cube as a shape, a click, Turn 45, a 40-point stroke, the cursor; then the same cube loaded from a GLB and from an FBX file, one click each |
| `brush_tab` | the pixel budget at 1280 x 800; the tab rule when the tool changes |
| `brush_types_path` (needs a model path) | stroke timing per type and size on a given model |
| `text_gaps` | one gap widens the ink, keeps the left edge, one undo entry; 3D and 2D |
| `text_styles` | thickening and slant, undo, redo |
| `text_warp` | three warp styles; the anchor stays |
| `text_path` | Arc, Circle, Curve and one point drag |
| `text_tab` | the see-through text under the pointer before placing (one hover step, an assert, a screenshot); every page with the colour picker inside the panel; the draft does not change the placed text; the three tabs together |
| `text_fonts` (this machine; logs SKIP per missing family, but two asserts are never skipped: at least 50 families, Arial present) | the font list really holds the fonts of the machine; real bold face against thickening; CFF and variable fonts render |

Also: `roundtrip`, `export_run` and `fuzz_path` extended as in TEXT 8.3 (a text with two gap values and a warp is
saved, reopened and exported; four new fuzz operations). The runner has `Step::Hover` from S2 (2.2 item 6).

One step this plan adds, because neither design tests the three tabs together. It follows the one tab rule of
2.2 item 10, and every line is an unconditional assert through `side_rects` (S2 records `side_rects.text_tab`
next to `side_rects.brush_tab`). At the end of `text_tab`, with `side_tab = Text`, the placed text selected and
the Transform tool active (the Text tab is shown):

1. `SetTool(Brush)`: the area shows Layers (`text_tab` and `brush_tab` are both `None`); `side_tab` is still
   Text; `selected_stamp` is still the text.
2. A click on the Brush tab: the Brush tab is shown (`brush_tab` is `Some`).
3. `SetTool(Transform)`: the area shows Layers, because `side_tab` is now Brush and Brush is not enabled.
4. A click on the Text tab: the Text tab is shown (`text_tab` is `Some`), because the tool is Transform and the
   selected stamp is a text.
5. `SetTool(Stamp)`: the area shows Layers; both tabs are grey.

**Runs to repeat from round 3** (to prove nothing got worse):

- `cargo test --workspace --offline` and `cargo clippy --workspace --all-targets --offline`.
- Every built-in scenario of the round-3 gate, each screenshot read by the agent.
- The 157-model sweep with `workflow_path` (`target\round2\jobs_wf.txt`), exit codes and times compared with the
  round-3 table. A model that got more than 10 % slower in the brush or text step is listed.
- The round-3 repros: `fuzz_path` seed 42 on multi_uv_sets.glb and seed 13 on Fox.glb (no "Unknown layer"), seed 7
  with 1,000 steps on ngons_tiling (the round-3 frame limits).
- The round-3 stamp and export limits on ngons_tiling and ABeautifulGame (image stamp, transform, export every
  format), with whatever numbers the round-3 gate reached as the new "not worse than" line.
- The Pillow check of every exported PSD and ORA (round-3 user decision 3), now with a styled text layer.

**Performance runs, new** (release build, machine idle, one run at a time, 1280 x 800):

- Brush: P1 to P13 of BRUSH 8.4, from `stroke_bench_tips` and from `brush_types_path` on the 8192 texture, the
  4096 texture and ABeautifulGame.
- Text: every row of the table in TEXT 8.4 (median of 5 runs), and its seven in-app checks.
- `fuzz_path` with the new operations: 3 seeds of 1,000 steps on the Sphere sample and on ngons_tiling.

**A limit that is missed is reported with its number.** It is not waved through and it is not hidden by changing
the test. The designs name the first fallback for the likeliest misses: a grid of 96 instead of 128 nodes for P5
and P6; a half-resolution distance transform for thickening and for the outline of a styled text.

**The report** (`target\round4\GATE_REPORT.md`) contains:

1. Per item R4-1 to R4-19: done, partial or not done, with the test names and scenario runs that show it.
2. A table of every performance limit: limit, measured value, baseline value, pass or miss.
3. For the 1.5-million-triangle model, in plain words: frames per second while painting before and after (the
   two medians of P13 next to their baseline values), the longest frame, how long a stroke trails after the
   release, and every frame over 100 ms with its cause. One sentence says whether the brush got faster there or
   only stayed responsive.
4. Every scenario whose expected hash was re-recorded because of the new stroke walk, with the old and new value.
5. Every cross-package edit and every contract request, with the reason.
6. The sweep table and the list of models that got slower or changed result.
7. Screenshots at 1280 x 800 of: the Brush tab with a loaded shape and its cursor; each page of the Text tab; the
   font picker; a warped text, a text on a circle and a text with one widened gap on the model.
8. What the user should try by hand, because no test can judge it: whether the warp shapes look right, whether the
   trail of a fast stroke feels acceptable, whether the picker shows the fonts they use.
9. Open defects and what was cut (section 5.3), with the user's answer for every cut that needed one.
10. The questions of section 4 that are still unanswered and the default that was built.
11. Deliberate changes of existing behaviour, each in one sentence: Ctrl+wheel and Shift+wheel with the Brush and
    the Eraser no longer zoom (BRUSH 1.5, question 9); a spacing change keeps the alignment side of a placed text
    fixed (risk 11); "Same side only" is replaced by Type; a fast stroke may finish after the release (R4-8).

Only after the report: the agent that started the round copies the merged source into the main tree and the exe to
`dist\`. Packages never write to the main tree or to `dist\`.

---

## 4. Questions for the user

**Question 1 — "segmented": decided, not to be asked again.** The user was asked on 2026-11-17 at 13:00 and
answered, word for word: **"segmented means editing the space between each character"**. The plan builds exactly
that as R4-10: one spacing value for every gap between two neighbouring characters. Not built, because not asked
for: own font, size or colour for parts of a text; letters as separately movable pieces; a stencil look.

Eight questions are open. All are put to the user now, in the order 3, 2, 6, 7, 4, 5, 9, 8 (the first decides
whether the two brush types look different at first try; the next two can throw work away).

**When an answer is needed.** P0 and S1 start without any answer. **Question 2 (tab or floating panel) is needed
before S2 starts**, because S2 builds the container. **Question 6 (a curved text on one side, or around the
model) is needed before wave 1 starts**, because the other answer postpones the whole Path page (R4-14) that T1,
T6 and T7 would build. If the user has been asked and has not answered when that point is reached, the default
is built, and this plan accepts in writing what that can cost: for question 2 the container and its tests are
rebuilt (about one day); for question 6 the Path work of this round (`path.rs` in T1, the Path page in T6,
`text_path.rs` in T7; about 1,300 lines by the estimates of TEXT 7.1, not counted exactly) may be set aside
unused. Every other question changes one default or one package and can be answered at any time before the gate.

| # | Question | Default the plan assumes | What changes with another answer |
|---|---|---|---|
| 2 | The brush settings and the extra text settings each get a tab next to Layers. While such a tab is open you see the name of the active layer but not the layer list. Is that acceptable, or do you prefer a small floating panel over the 3D view? | Tabs. | Floating panel: only the container changes (S2, B4, T6), about one more day for its tests; it would be the first floating window of the app and covers part of the model. |
| 3 | **With the planned default, Circular and Sphere look the same at first try:** at the default Size of 32 px both paint the same pixels on every built-in sample and on any plain solid model (BRUSH 1.7). They differ only on thin walls, sharp folds and with large brushes. The reason is that today's brush already colours other parts that lie inside its ring when they face the same way, even hidden ones (painting the foot of a chess piece also colours the board under it). Should Circular keep doing exactly that, with a checkbox "Stay on connected surface" that is off? Or should the checkbox be on from the start, so that the two types differ at once on every model with more than one part? Or do you not need the checkbox at all? | Exactly as today; the checkbox exists and is off. | On from the start: one default value; the two types then differ at first try on every model with more than one part. No checkbox: package B2 and the flood code in B3 are dropped (about 550 lines less). |
| 4 | Size is the diameter of the ring, and a loaded shape is scaled to fit inside the ring, so a cube's side is 58 % of Size. Is that right, or should Size be the shape's longest side? | Fit inside the ring. | Longest side: one line in B7; the shape then reaches up to 1.73 times beyond the ring and large shape brushes are about 3 times slower. |
| 5 | What should happen with a shape file that is not a closed solid? Planned: a model with an opening (a cup, a head open at the neck) is refused with a message, and only closed parts of a file are used; a flat shape such as a star is used as a cookie cutter whose outline is pressed straight through the surface. | As described. | Close openings automatically: about 150 more lines in B1, and deep hollows of open models get filled too. Wrap a flat shape onto the surface like a stamp: not in this round; that is what the Stamp tool does. |
| 6 | Is a curved text on one side of the model enough (the line lies in the flat picture of the text, which is then projected as today), or must a text be able to run around the model, for example once around a bottle? | One side. | Around the model: a new kind of stamp with one projection per letter, about 3 weeks and its own design; the Path page (R4-14) would be postponed, everything else stays. |
| 8 | Is a font you use missing from the font list of the app today? Two kinds are known not to appear: fonts that Windows knows only through its registry (installed from a folder outside the three Windows font folders), and the named styles of variable fonts (Bahnschrift, Segoe UI Variable and Sitka show one face each instead of "Light", "SemiBold" and so on). | Both stay as they are. | Registry fonts or loading a font file by hand: about 1 day in T3 and T7. Named styles of variable fonts: about 3 days and a change in how the font rasterizer is built; not recommended (TEXT section 2). |
| 9 | With the Brush or the Eraser, Ctrl+wheel will change the brush size and Shift+wheel will turn a loaded shape, as both already do for stamps and texts. Today both zoom the view with these two tools; the wheel without a key keeps zooming. Is that change welcome, or should Ctrl+wheel and Shift+wheel keep zooming? | The change is built. | "Keep zooming": one function less in B7; size and turn stay reachable through the sliders, the `[` `]` keys and the preview. |
| 7 | Two words of the text request were read as what the app already does: "angled" = the whole text is turned (exists today; an Angle field is added), and "stamp type: text" = a text is its own stamp and layer, as the Text tool makes today. Are both right? | Yes to both. | Each letter turned on the spot: half a day (one number, T2 and T6). Text written onto an image stamp so that both move as one: a new design, about one week, not in this round. |

Smaller questions of the two designs are **not** put to the user now; their defaults stand and each can be changed
later without touching another package:

- Brush: Sphere also paints surfaces you cannot see (yes); the centre of the ball rides on the surface, Offset moves
  it (yes); "upload" means "load a file from this computer" (yes); the shape file is remembered by its path, no
  shape library (BRUSH Q4); the file's +Y axis points away from the surface (BRUSH Q7); the four small extras Cube
  and Cylinder, Offset, "Fixed in space", dragging the preview are built (BRUSH Q6); no jitter, scattering or
  texture settings.
- Text: a click with the Text tool does not select an
  existing text, text settings are not remembered between runs, no rescan button (TEXT Q6, each half a day or less
  if wanted); typing into a placed text grows it about its centre as today (TEXT Q8).

---

## 5. Size and risks

### 5.1 Lines per package (new or changed, tests included; estimates)

| Package | Lines | Of which tests (about) | Basis |
|---|---|---|---|
| P0 | 0 merged (throw-away instrumentation only) | — | BRUSH 8.4 |
| S1 core scaffold | 1,860 | 560 | BRUSH 7.2 rows for `brush_tip`, `tip_field`, `samples`, stubs: 1,150; the three file writers: 120; core half of TEXT S0: 550; the frozen fixture test: 40 |
| S2 app scaffold | 885 | 320 | BRUSH 7.2 app contract rows and controller: 320 plus the strip; `Step::Hover`: 15; app half of TEXT S0: 400; the two egui proofs with the recording buffer: 120 (moved from T6) |
| B1 brush bake | 920 | 450 | plus F21 to F25: 150 |
| B2 brush topology | 150 | 60 | |
| B3 brush paint | 1,510 | 640 | plus the save test of BRUSH 8.5: 40 |
| B4 brush panels | 730 | 220 | plus scenario `brush_tab`: 100 |
| B5 brush flows | 550 | 190 | plus the formats in A16: 40; scenario `brush_shape`: 130 |
| B6 brush gpu | 60 | 25 | |
| B7 brush tool | 850 | 250 | plus scenarios `brush_types`, `brush_types_path`: 230 |
| T1 text geometry | 1,550 | 650 | TEXT 7.1 |
| T2 text painters | 1,400 | 650 | |
| T3 fonts | 550 | 250 | |
| T4 text document | 950 | 500 | |
| T5 text project | 470 | 350 | |
| T6 text panels | 2,360 | 890 | minus the two proofs (120, now S2); plus the Angle test 70, the control sweep 190, scenarios `text_tab`, `text_fonts`: 120 |
| T7 text tools | 1,460 | 480 | plus the three hover tests: 130; scenarios `text_gaps`, `text_styles`, `text_warp`, `text_path` and the extensions: 330 |
| Z gate | 700 | — | `ARCHITECTURE.md` 300 + about 300; pasting and registering the scenarios: about 100. The 910 scenario lines are now in the author packages |
| **Total** | **about 16,950** | | brush about 6,750, text about 10,200. This is 900 more than before the completeness check: the added tests and writers (about 800) and the registering work |

"Scenario" lines in a package are the delivered note file; they are counted once, with their author.

### 5.2 Main risks

| # | Risk | What the plan does about it |
|---|---|---|
| 1 | Round 3 is not finished. The panel layout, the stamp caches and the load warnings that round 4 builds on are not known. | Nothing starts before the round-3 merge; P0 re-reads the references and measures the two panel heights (2.6, 3.1). |
| 2 | The scaffold is 2,450 lines in about 45 files and everything waits for it. A wrong contract costs every later package. | Two scaffold packages with a review after each; stubs carry final signatures; the contract-request file (2.5). |
| 3 | T6 is the largest package (2,360 lines), and the gap buffer of the content field rests on egui behaviour that was read, not tried. | The two proofs of TEXT 8.2 are written by S2, before `TextPanelState` and `text_gap` are fixed and before T1 builds `gaps.rs`; a failed proof switches to the designed fallback (text comparison) at that point. If T6 slips, the cut line of 5.3 keeps the gap strip. T6 writes the control sweep last, because it needs every page. |
| 4 | Nobody has measured where the 35 to 59 ms per frame go when painting on the 1.5-million-triangle model (design). The single query may not help there. | P0 measures before B3 starts; P3 is set from the measurement; the report says in plain words if the model stays at its present speed. |
| 5 | No text time was measured; all are estimates. An outline on a 16.8-megapixel styled text is estimated at 0.7 s and 134 MB on the UI thread per final edit (design). | The limits of TEXT 8.4 are measured at M1, early enough for the half-resolution fallback. |
| 6 | Compiling `wte_core` directly against `ttf-parser` was checked by dependency resolution only, not by a build (TEXT 5.1). | First task of S1. Named fallback: read outlines with `skrifa`, which changes one file. |
| 7 | 14 packages produce 14 hand-over notes, about 30 architecture sections and many cross-package edits to merge; scenario failures would otherwise show only at the very end. | One owner for `ARCHITECTURE.md` and for the merged `scenarios.rs`; two merges (M1, final) instead of one; owned files are copied, not merged; every scenario is written and first run by its author package and only pasted by Z (2.2 item 11, sitting M2). |
| 8 | Machine load: wave 1 runs 8 builds at once on 12 logical processors, each in its own target folder on E:. | Wave 1 builds only `wte_core`; wave 2 has 6 app builds. Package-phase timings are never trusted; only P0, M1 and Z measure, alone. P0 measures the size of one full target folder and compares 17 times that, plus 20 %, with the free space on E: (745 GB on 2026-11-17) before the copies are made (3.1 item 5). |
| 9 | The stroke pacing changes when pixels arrive: scenarios with exact hashes after long drags may change, and a stroke can finish after the release. | The runner waits for idle tools (S2); the gate lists every re-recorded hash; tests A4, A5 and A19 prove that pacing never changes pixels and never loses a stroke. |
| 10 | Format version 2: a mistake would make projects unreadable. | S1 adds the frozen version-1 text fixture as its first project task and runs it green on the unchanged code, before the `fx` field and version 2 are written (2.5); the test must pass unchanged from then on and belongs to T5. `min_reader_version` rises only when a text uses a new feature; round-trip tests in T5; B3's test that a painted document saves byte-identically through both stroke entry points with `min_reader_version` 1. |
| 11 | Spacing changes now keep the alignment side fixed, which changes today's Letter spacing behaviour for left- and right-aligned placed texts. | Stated in the report as a deliberate change; typing still grows a text about its centre (question TEXT Q8 is not asked now). |
| 12 | The Brush and Text tabs hide the layer list. | Question 2; the Brush tab names the active layer; the area shows Layers whenever the tab's tool is not in use. |
| 13 | Open or flat shape files behave in ways the user may not expect (refused, or used as an outline). | Question 5; messages that say what to change; the preview and the cursor show the result before painting. |
| 14 | Shape files other than OBJ could fail in ways no test saw (node transforms, several meshes, FBX units). How the FBX importer applies a model's own scaling was not verified for the design. | Tests F21 to F25, the four formats in A16 and in `brush_shape` (R4-5). A format that fails is reported by name; it is not dropped silently from the file dialog. |
| 15 | With the default settings the two brush types paint the same pixels on simple models, so the user may think Sphere does nothing. | Question 3 says so in plain words and is asked first; the scenario `brush_types` shows the difference on the cube with a large brush; the Brush tab's preview shows the tip. |
| 16 | Ctrl+wheel and Shift+wheel stop zooming with the Brush and the Eraser, which nobody asked for. | Question 9; one function in B7; named in the report. |
| 17 | Work on the Path page may be thrown away if the user wants text around the model. | Question 6 is asked before wave 1 starts; the cost of building the default without an answer is written down in section 4. |

### 5.3 Cut line, if the round must shrink

Dropped in this order; each cut removes work from the named packages only and leaves the file format as designed:

1. R4-17, the picture of the text before placing (T7).
2. The samples in the font picker (T3, T7); search and names stay.
3. The Curve kind with its points in the view (T7, parts of T1 and T6); Arc and Circle stay.
4. The direction switch of the warp (T1, T6).
5. R4-3, "Stay on connected surface" (B2, part of B3), if question 3 allows it.
6. Shapes from model files, R4-5 (B1, B5); the built-in Cube and Cylinder stay. **Only with the user's yes** (see
   below).

**A cut that removes something the user asked for in words is never made silently.** Cuts 1, 2, 4 and 5 remove
comforts the request does not name, and cut 3 leaves "curved along a line" true through Arc and Circle; the
agent that runs the round may make them and reports them. The following are **never cut without the user's
yes**; before such a cut the user is asked one question that names what would be missing and what it would cost
to keep, and the round waits for the answer:

- R4-5, a shape loaded from a model file ("upload and orient the shape and size of the 'sphere' into any
  compatible model type"), and any single one of its four formats;
- the last remaining path kind of R4-14 (Arc and Circle may not both go: "curved along a line");
- R4-13 warp ("warped"), R4-12 bold and slant ("slanted, bolded"), the Angle field of R4-15 ("angled"), the font
  picker's list of machine fonts in R4-11 ("choose from fonts on the machine");
- the Brush tab of R4-6 ("its own tab").

Never cut at all: the scaffold, the tab strip, Circular and Sphere, the stroke pacing, gap spacing (the one text
feature the user defined in their own words), the anchor rule, version 2 of the project format.

---

## 6. What the completeness check of 2026-11-17 changed

Each gap was checked against the documents and, where it cites code, against the source. All 18 were confirmed.

| # | Severity | Gap | Checked | What was changed, and where |
|---|---|---|---|---|
| 1 | major | Every test that loads a brush shape used OBJ. | BRUSH 8.1 to 8.4 as they were: F14, F15, A16, S2, P10 name OBJ only. `target\compat` holds no small glTF, GLB or FBX cube. | BRUSH section 8: three file writers; F21 to F25; A16 and S2 extended. Plan: R4-5, S1 and B1 rows, risk 14. |
| 2 | major | The see-through text before placing had no test. | TEXT 8.2 and 8.3 as they were. | TEXT 5.14 (`hover_preview`), 8.2 (three tests), 8.3 (`text_tab` with a hover step). Plan: R4-17; `Step::Hover` moved to S2. |
| 3 | major | The Angle field had no test. | TEXT 8.2 as it was. | TEXT 8.2 `the_angle_field_turns_the_right_text`. Plan: R4-15. |
| 4 | major | The tab rule contradicted itself. | `app\dispatch.rs:631-635`: `SetTool` keeps the selection. | One rule, plan 2.2 item 10; TEXT 6.1, 7.3, 8.2; the cross-feature step of 3.3 is five unconditional asserts. |
| 5 | major | Text edits in `tool_options.rs` had no package. | `app\panels\tool_options.rs:85-101`, `:203`, `:220-222`: the warning line is inside `text_options`, which moves out. | Plan 2.2 item 12, 2.4 (S2 row), R4-15 Files; TEXT 6.1. |
| 6 | major | The cut line could drop a requested phrase without asking. | Plan 5.3 as it was. | Plan 5.3 and section 0 item 8: a list of what is never cut without the user's yes. |
| 7 | major | The frozen version-1 fixture had no owner and no point in time. | Plan 3.1, 5.1, risk 10 as they were; TEXT 7.5. | S1's first project task (plan 2.4, 2.5, 3.1, risk 10; TEXT 4.3 item 5, 7.5). |
| 8 | minor | `history.rs` "not touched" against a `TextSpec` literal in it. | `core\history.rs:1537`. | Allowed one-line exception: plan 2.2 item 13, 2.4; TEXT 7.1; BRUSH C1. |
| 9 | minor | The brush save test had no author and a false clause. | BRUSH 8.5 as it was; plan section 0 item 6 (version 2). | B3, `brush_tips.rs`, asserts `min_reader_version` 1 (BRUSH 8.5; plan R4-18, 2.4, risk 10). |
| 10 | minor | The changed wheel gestures were in no item, question or report. | BRUSH 1.5; `app\tools\mod.rs:191` (the default `scroll` returns false). | BRUSH 1.5 and Q9; plan R4-4, question 9, report item 11, risk 16. |
| 11 | minor | No limit on the 4096 texture; only relative limits on the large model. | BRUSH 8.4 as it was. | BRUSH 8.4: model (c), P1, P8, new P13. Plan: R4-8, R4-9, 3.1, report item 3. |
| 12 | minor | The two brush types look the same with the defaults. | BRUSH 1.7 and section 2 (design). | No design change. Question 3 says it in its first sentence and is asked first; risk 15. |
| 13 | minor | Q5 (missing fonts) was not asked; `text_fonts` could pass without a machine font. | `target\round3_audit\runs\text_builtin\shot.log:26`: "font catalog: 78 families". | Question 8; two hard asserts in `text_fonts` (TEXT 8.3; plan R4-11, 3.3). |
| 14 | minor | Many text controls were tested in the core only. | TEXT 8.2 "Other controls" as it was. | TEXT 8.2 `every_text_control_changes_exactly_its_own_field`; plan R4-12. |
| 15 | minor | The egui proofs came after the contract they decide. | TEXT 6.2 rule 6; plan order. | The proofs move to S2 (plan 2.4, 2.5, risk 3; TEXT 6.2, 8.2, 9.1). |
| 16 | minor | Scenarios were written only by the gate. | Plan 2.2 item 6 as it was. | Author packages and sitting M2 (plan 2.2 items 6 and 11, 2.4, 3.3, 5.1, risk 7). |
| 17 | minor | Two totals; unverified inputs. | The table of 5.1 summed to 16,050. `high_poly_sphere.obj`: 500,000 `f` lines counted. No `check_exports.py` found under `target\`. Free space on E: 745 GB; the size of a target folder could not be measured in 100 s. | One total (16,950 after the additions); three P0 checks (3.1 item 5); risk 8. |
| 18 | minor | TEXT 7.5 wanted answers before any package; the plan said nothing blocks. | TEXT 7.5 gate 1; plan section 4 as it was. | Question 2 before S2, question 6 before wave 1, with the written cost of building the default (plan section 4, 2.6; TEXT 7.5). |

---

## 7. User answers of 2026-11-17 (binding; they override sections 0 to 6 and both FEATURE documents)

Answers, word for word where quoted, and what they mean for the build.

1. Q3 (Circular vs Sphere): "circular the way it looks is fine, the "sphere" is a way to have different paint brushes
   via a 3D object; the default 3D object is a sphere. they might paint the same on most surfaces, and that is to be
   expected." -> Circular stays exactly as today. The "Stay on connected surface" checkbox is **not built**: drop
   package B2 and the flood code in B3 (about 550 lines). Sphere = the 3D-object brush; its default object is a ball.
2. Q2 (tabs): "agree" -> Brush and Text tabs next to Layers, as planned.
3. Q4 (size): "whatever is inside the 3D brush object is painted" -> default stands: the shape fits inside the ring;
   everything inside the shape volume is painted.
4. Q6 (curved text): "for now, just flat editing like in photoshop. but that can be something new to add later" ->
   default stands: a curve in the flat picture of the text, projected as today (Path page R4-14 is built). Text
   around the model is a later round; record it in TEXT 9.2 as future work.
5. Q5 (open shapes) — CHANGED: "the app should ask to close openings, and how they should be closed." -> A loaded
   shape file with openings is no longer simply refused. The app finds the openings (boundary loops of the mesh) and
   shows a dialog that names the file, the number of openings and the size of the largest, and asks how to close
   them:
   - **Flat caps** (default): each opening is filled with a flat patch across its edge loop.
   - **Rounded caps**: each opening is filled with a patch that bulges outward along the average normal of its edge.
   - **Fill all hollows**: the solid is closed from outside (the six-direction rule of BRUSH section 2 row 8); also
     fills deep hollows; the dialog says so.
   - **Use only the closed parts** (only offered when the file has closed parts).
   - **Cancel**.
   The choice is shown in the preview before painting, is stored with the remembered shape path (so a reload closes
   the same way without asking again), and can be changed later from the Brush tab ("Openings: Flat caps ▾").
   Flat sheets keep the planned outline ("Through") handling; a sheet is not offered caps. Owners: the capping and
   the hollow fill in B1 (`core\tip_bake.rs`, new `CloseMode` input to `bake`; `Err(NoInterior)` becomes
   `Err(HasOpenings { count, largest })` when caps could help); the dialog and the stored choice in B5
   (`app\brush_flows.rs`, `app\prefs.rs`); the tab control in B4. Tests: `open_box(1.0)` with each mode gives `Ok`
   with the expected inside fraction (flat caps = the closed box within 2 %); a cup with a rounded cap is larger than
   with a flat cap; a mesh with a non-planar hole loop caps without self-intersection failures (bake still `Ok`);
   scenario `brush_shape` loads an open model, picks Flat caps through the dialog and asserts the preview is
   non-empty. Estimate: about 400 lines more (B1 250, B5 100, B4 50).
6. Q8 (fonts): "the font list should just take from what the computer has in its fonts folder" -> default stands:
   the Windows font folders; no registry fonts, no variable-font named styles.
7. Q9 (mouse wheel): "looks good" -> Ctrl+wheel size and Shift+wheel turn with Brush/Eraser, as planned.
8. Q7 (angled, stamp type text): "if its already there then its already there" -> "angled" = the whole text turned
   (exists; the Angle field is added); a text is its own stamp, as today. Per-letter rotation and text on an image
   stamp are not built.

Net size change: about -550 (B2 and flood) +400 (closing openings) = about 16,800 lines.
