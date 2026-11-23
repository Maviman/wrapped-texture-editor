# Stable gate: completeness critic

Critic (Opus), 2026-11-23. Branch `stable` at e1bf12f, worktree `G:\software_projects\texture_wrapper_ap_r9`.
Read-only: I edited only this file. Inputs: `docs/reports/STABLE_GATE_REPORT.md`, every note in `docs/notes/stable/`,
the export code (`crates/wte_core/src/export/`, `dialogs/export.rs`, `dispatch.rs`, `io_flows.rs`,
`stamp_bakes.rs::settle`), the scenarios (`export_flow`, `export_zip`, `final_bake_async`, `fuzz_path`) and the GPU
export tests. Gate evidence I opened is under `target\stable\gate\`.

## What I checked myself

- **The merge.** `git diff a8beea6 e1bf12f` shows no `Cargo.toml` or `Cargo.lock` change. Every changed file is
  `w/lf`. `NAMES` has 42 entries (scenarios.rs lines 34-75).
- **The ngons_tiling stamp layer inked on every texel.** I opened
  `flow\ngons_tiling\shot_path_one\ngons_tiling_does_not_exist_L02_stamp_Logo.png`. It is 1024x1024 with alpha > 0 on
  100 % of texels and alpha 255 on 90.9 %. It has 3,716 distinct colours. The OBJ's `vt` range is u 0..4, v 0..4. This
  means the picture is repeated, not a solid fill or garbage, so the "model-caused" label holds. Note that this file is
  the *wrapped* stamp (the flow log says covered 10 %, 6 tris), not the projected one.
- **The export dialog's Layers choice** (`dialogs/export.rs::apply_scope`). Visible and All clear `layers` but keep
  `sets`. Selected clears `sets`. `dispatch::export_for_layers` sets `sets = [that layer's set]`, `flat = None`,
  `ora = psd = false`. check_exp_2's observation is therefore confirmed in code (see gap H2).
- **`run_zip`.** Entry names come from `plan_contents`. The plan depends only on the document, never on the disk, so
  the staged names and the entry names cannot diverge. Only the first set of a multi-set layer list is used
  (`export_for_layers`: `find_map` takes the first layer's set).

## 1. Claims that rest on one source, or were never checked

1. **export_flow never ran on ABeautifulGame on the merged build.** This model has 15 sets, a 227 MB zip and the most
   changed wrap. It ran only on the pre-merge export-select exe (check_exp_1/2, exe 0bd11643). The gate's flow list
   is the 4 cube formats plus 7 models plus 4 old projects. gltf_node_transforms and torus_knot were also run only
   before the merge.
2. **The post-merge export files were verified only by the gate's own script.** This covers `spot_zip.py`,
   1.22/255, and 45/45 byte-equal. check_exp_2's independent `verify.py` (ORA crops, PSD headers, flattened
   selection) ran on the pre-merge exe, not on 9EB3F988. The risk is low, because the merge touched no export code.
   Still, no second agent has looked at a file written by the shipped exe.
3. **The old-project compatibility claims are gate only.** These are 12/12 ok, 0 px against both r9 and the
   creating exe, and 12-file byte identity. The four `.wtep` files were made by the gate itself, and no checker
   re-ran any of it.
4. **The post-merge non-wrap identity is gate only** (35/38 scenarios identical). The checkers compared 15 scenarios
   on stable-wrap and about 6 on export-select, never on the merged exe.
5. **The ignored GPU tests (17/17) on the merged build are gate only.** check_exp_2 ran 13 of them (`control_sweep`)
   pre-merge.
6. **The whole timing section is gate only,** with two runs per line, apart from one-run numbers in
   check_wrap_1/2. "The zip export is not slower than the plain one" was never measured like for like: every zip run
   was a warm second or third export, and every plain run was a cold first export. The DamagedHelmet 2048 numbers
   (0.06 / 0.13 / 1.96 s) are single runs.
7. **The fuzz says nothing about the new features.** It ran 4 runs, all by the gate. `fuzz_action` never uses
   ToggleLayerSelected, range select, export, rename, save or reopen. "0 errors" covers painting, stamping, layers and
   undo only.
8. **The panel lines of the three red sweep models were never seen.** nonmanifold, noisy400 and tiles300 stop at the
   step-10 assert before the panel step. fix_wrap_1's "would read 82 % runs off / Only 47 %" is derived, not
   observed. noisy400's lines come from one final screenshot.
9. **The ABeautifulGame flipped count is attributed by one source.** fix_wrap_1 alone attributes +11,979 flipped
   triangles (70,583) to w2's area push, and check_wrap_2 says "not re-verified by me".
10. **The WA20 cause has not been shown.** "The area term on the steepest bumps" is w2's explanation, and no
    experiment isolated it. Both attempts failed, so the cause is a hypothesis.
11. **The owner-ray slowdown attribution is single source.** Avocado's owner phase went from 22 to 54 ms, and only
    fix_wrap_1's probe shows this.
12. **"Nothing shows a wrong panel line" rests on the gate's reading,** and two lines are arguably misleading (see
    contradiction C4).
13. **The merged build's determinism was not re-run.** check_wrap_2 ran it (sync bakes, 1 vs N threads) on
    stable-wrap only.
14. **Items still open from r9_critic that nobody revisited on stable:**
    - the WaterBottle exact-label tint check (68.14 % within 2/255, one probe);
    - the torus_knot meeting line, which is on the hidden back in every shot;
    - the Project-mode shared-texel line and warm-up on shared-UV models (DamagedHelmet). Stable equals r9 here, but
      the r9 decision on it was never taken.
15. **The merge itself is the gate's word.** The claim that the only conflict was ARCHITECTURE.md section 9 cannot be
    checked after the fact. I verified the result instead: no Cargo change, LF, 42 names.

## 2. What a user would plausibly do that no test covers (only what can really break)

### High

**H1. A real binary FBX from Blender, Maya or 3ds Max.** Binary FBX is the default FBX export of every DCC tool,
and it is a supported import format. Coverage is only `binary_quad_in_every_encoding`, a quad made by the test's own
minimal writer, plus the generated ASCII cube. No real binary file exists on this machine (check_exp_1 gap 5). The
parser has never been tested on real node layouts: several UV sets, ByPolygonVertex/IndexToDirect layers, embedded
Video content, material mapping and FBX 7.4-7.7. A parse error means the user's model does not import at all.

**H2. "Export this layer...", then the user switches to Visible / All and the zip, expecting "everything".**
The options keep `sets = [that set]` and keep the flattened texture, ORA and PSD unticked. On a multi-set model the
"everything" zip then holds one set's per-layer PNGs only. The zip is named `<base>.zip`, the same as the full
package, so after the "will be replaced" confirm it overwrites a real full package in that folder. The Texture-sets
checkboxes do appear with one ticked, so the cause is visible in the dialog. Nothing tests this sequence, and the
report does not mention it, although check_exp_2 section 6 does.

### Medium

**M1. Export one layer of a stamp that spans two materials.** A stamp across a material boundary has one layer per
set (sample `multi`). "Export this layer..." and the selection use only the clicked set's layer
(`export_for_layers` takes the first layer's set), so half the design is missing from the file. There is no test
and no hint in the UI. On a multi-material model like ABeautifulGame this is the normal "export one layer" case.

**M2. Export right after placing or editing a wrapped stamp.** In practice this means clicking Export before the
Final has landed. `start_export` calls `stamp_bakes::settle`, which waits for the Final, or runs it synchronously,
on the UI thread. The one test of this path (`final_bake_async` (f)/(k)) uses a Project-mode text on the sphere.
`export_flow` waits idle and even asserts that no Final is pending before it exports. With a wrapped stamp on
ABeautifulGame (Final 3.4-4.1 s), the UI could freeze for seconds with no progress dialog. A bug in settling a
wrap Final would export stale or preview pixels. Untested.

**M3. Multi-select followed by Delete, Merge Down, Duplicate or Undo, then "Export selected".** The selection is
normalised on read (unit tested), but the fuzz never selects more than one layer. No scenario deletes or undoes
with several layers selected and then exports. Users will expect Delete to act on all selected layers; it acts on
the active one only, and only the footer says so.

**M4. Large textures: 4096 / 8192 sets, several layers, several sets.** `MAX_TEXTURE_SIDE` is 8192. The largest
export tested is 2048 (DamagedHelmet; ABeautifulGame is 15 x 2048, pre-merge only). An 8192 RGBA layer is 256 MB.
Sets are exported in parallel lanes, and each lane builds PNG, PSD and ORA. Peak memory, the time to deflate a
multi-GB PSD in the single-threaded packer (cancel is checked only between files), and a zip over 4 GB are all
untested. `LARGE_ENTRY` handles one entry of 4 GB or more; archive offsets over 4 GB are left to the `zip` crate
and never exercised.

**M5. Two material or set names that sanitize to the same stem.** Examples: `Body` / `body` (the deduper ignores
case), `Mat:1` / `Mat_1`, and long DCC names that share their first 64 characters. The full export de-duplicates
the second set's files to `_2`. A selection holding only the second set's layer has no collision and gets no `_2`.
So D4's rule ("a selected layer's file has the same name as in the full export and in the zip") breaks, and two
exports into one folder can silently pair the wrong files. There is no test with colliding set stems.

**M6. A wall-clock unit test sits at its limit.** `check_a_own_bad_meshes_never_fail` took 197.7 ms against a
200 ms limit on an idle machine. Both it and `wa7_...` failed under CPU contention in fix_exp_1 (276.5 and 366.5 ms),
and both now take the best of 5 runs. `cargo test` going green is therefore load-dependent, against the user's
"all tests green, stable build". This is not in the report.

### Low (could break, but unlikely or with a clear error)

**L1. An export folder that is read-only, on a full disk, or holding a file locked by a viewer.** The core tests
block the temp file and the stage folder. The app's error path ("Export failed: ...", CreatedDir cleanup) has never
been driven end to end. In Files mode a failure part-way leaves a mix of new and old files from earlier exports,
which is by design but never shown to the user. The default export folder is the model's folder plus `\export`,
which can be read-only (a downloaded model in a protected location).

**L2. Non-ASCII layer, set or model names in the zip** (CJK material names in glTF, an emoji in a renamed layer).
`sanitize_file_stem` keeps them, and the `zip` crate sets the UTF-8 flag. No test writes a non-ASCII name into a
zip or reads it back.

**L3. A model without UVs (generated layout) taken through export, and through save, reopen and export.**
`import_nouv` paints but never exports. `export_flow` never ran on such a model.

**L4. A crashed zip export leaves `<base>.zip.tmp-<pid>.d`.** It is cleaned up only by a later export with the same
pid, so it stays in the user's folder.

**L5. Long paths.** The stem is capped at 64 characters, so a file name can reach about 210 characters. The stage
folder adds about 25. This is fine for the app (std uses verbatim paths), but Explorer's "Extract all" may fail.

**Already covered:** hidden layers (selected hidden exported; Visible mode), two exports in a row (zip twice is
byte-identical; the Files confirm step), undo after export (the revision is unchanged), exporting after reopening a
saved r67/r8/r9 project, multi-set models (ngons 4 sets, ABeautifulGame 15 sets pre-merge), and odd characters
in layer names (the sanitize unit tests and "Hidden: notes" in PSD). One gap remains here: replacing a zip *through
the app* is never confirmed (the GPU test cancels at the confirm step), and neither is cancelling a zip during
"Packing".

## 3. Contradictions between the notes and the report

- **C1. The DamagedHelmet shared-texel share.** The report (section 5, panel lines) says "shared-texel line shown
  (100 %)". check_wrap_2 says "shared texels (86.7 %, model)". One of them is wrong, or they measure different things.
- **C2. The model count.** The verdict says export_flow ran green "on 12 models and formats". The table lists 11:
  4 cube formats plus WaterBottle, Avocado, DamagedHelmet, BoxTextured, Lantern, ngons_tiling and klein_bottle. 11
  plus 4 old projects gives the 15 zips the spot check counts.
- **C3. The ngons_tiling set count.** The report says "3 texture sets + 1 without a map". Its own flow log says
  "4 flattened ... 4 set(s) recomposed", and check_exp_2 says 4 sets. The fully-inked one-layer file is the
  *wrapped* stamp in the `does_not_exist` set. The report's explanation ("a stamp in UV space repeats") fits, but it
  calls these files "the stamp layers of two sets" without saying that one of them is the wrapped stamp.
- **C4. Panel lines.** The report's closing line says "Nothing of this ... shows a wrong panel line". Its own item 5
  and check_wrap_2 say that ABeautifulGame's "9 % runs off the surface: an edge, a hole or the end of the part"
  describes a picture that would continue onto a flush, touching part. That is the case Jump small gaps exists for,
  and the line names the wrong cause from the user's point of view. Item 6's parts200 "28 % runs off" also reads
  wrong on screen. "Literally true" is not "accurate, explains what happened", which is the user's standard.
- **C5. Omissions against "the workflow works".** The report's section 8 table leaves out check_exp_2's four
  observations:
  - the dialog trap (H2);
  - a selection zip that replaces the full package;
  - one-set-only export of a two-set stamp (M1);
  - multi-select Delete and Merge acting on the active layer only (M3).

  It also leaves out fix_exp_1's timing-test margin (M6).
- **C6. "Zip export not slower".** The report says the zip "is not slower than the plain one" while admitting that
  every zip run skipped the warm-up. Its evidence supports only "no slower than a warm export, unmeasured cold".
- **C7. Python.** CLAUDE.md says to run tools with `python -I` and that numpy and Pillow are present.
  check_exp_1, fix_exp_1 D7 and check_wrap_2 show that `-I` hides them (they are user site-packages) and that
  `-E -P` works. CLAUDE.md has not been corrected.

## 4. What the user must decide

1. **Whether to land `stable` on `main` with WA20 red** (cells 0.690..1.986 against 0.70..1.40). w2 made two attempts
   (the no-loop rule stops further work). The other options are a third approach in a new round, or an accepted
   known red.
2. **What the export dialog should do after "Export this layer...".** Should switching the Layers choice to
   Visible / All reset the texture sets and formats? Should a selection zip get its own name (for example
   `<base>_selection.zip`) instead of `<base>.zip` (H2)?
3. **Whether "Export this layer" on a stamp that spans materials should include that stamp's layers in every set
   (M1).**
4. **Multi-select semantics:** should Delete, Merge and Duplicate act on all selected layers (M3)?
5. **The ABeautifulGame flush-frame cut:** make Jump small gaps continue onto a coplanar touching part, or accept
   the cut and change the panel wording.
6. **Whether to accept the small and medium wrap slowdown against r9** (ABeautifulGame Jump off param +25 %, text
   scale Final +21 %) as the price of w2's owner visibility.
7. **Whether the three wrap_surface reds** (nonmanifold dup 3.414 %, noisy400 dup 5.124 %, tiles300 ink 1.3286)
   stay as known model-caused reds in the sweep. They cannot be loosened under the rules.
8. **Whether to get a real binary FBX** (from Blender) as a test asset before calling import "done" (H1).
9. **How to handle the near-limit wall-clock tests** (M6): keep them as is (risking flaky reds under load), or run
   them in a separate, serial test target.
10. **Whether to fix CLAUDE.md's `python -I` instruction (C7) and the "Exported 1 files" toast.**
