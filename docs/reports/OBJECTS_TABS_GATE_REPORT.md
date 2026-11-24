# objects-tabs gate report (work package 6)

Branch `objects-tabs` (worktree `G:\software_projects\texture_wrapper_ap_ui`), plan `docs/plans/OBJECTS_TABS_PLAN.md`
section 9. Gate run 2026-11-24 on the gate machine (RTX 5070, Vulkan). Gate commit: `fe855c4` (code) — this report and
nothing else is committed after it. Release exe of the gate (built from `fe855c4`):
`target\release\wrapped_texture_editor.exe`, SHA-256 `7B22B452DB6157501334205F451F3BE72282BFA96733EBCE8741987FA4F87BB7`
(copy `target\objtabs\gate\wte_gate.exe`). Baseline: `main` @ 1f0be41 (`target\objtabs\baseline\wte_base.exe`,
`docs/notes/objtabs/baseline.md`).

## Verdict

**PASS.** Every item of the plan's gate list is met, with these known red / open items (none caused by this branch):

| # | Item | Status |
|---|---|---|
| R1 | `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` fails | known red, the same on `main` (baseline 1,232 / **1** / 43) |
| R2 | `import_save` on BoxTextured.glb: `"ok": false` ("the source bundle has 0 sidecar(s)") | pre-existing, the same with the baseline exe (the scenario needs a model with sidecar files; on `coincident_flipped_spheres.obj` it passes) |
| R3 | Exports are byte-identical to the baseline **except 13 files** (`export_flow`: 12 zip / ora / psd; `fuzz_path_box`: its zip) | explained in WP4/5: in the baseline run the old side panel had grown to 304-316 pt, so the views were narrower and these two scenarios (view-relative clicks, random clicks) paint elsewhere. Byte-identical to the WP4/5 exe; `check_exports.py` PASS on every PSD / ORA folder |
| R4 | View-rect pixel identity: 10 of 73 comparable shots differ from the baseline | 7 are the WP4/5 differences (explained there, same cause as R3 or a moved pointer / popup); 3 lie only in the toast area (fade timing), which the identity check excepts. Against the WP4/5 exe only those 3 toast-area shots differ |
| O1 | Alt+H on Windows (system beep / menu?) | manual check by the user (in `dist/HOW_TO_TEST.txt`); fallback Ctrl+Shift+H |
| O2 | Double-click on the split edge with a real mouse | manual check (works headless and in `side_tabs`) |
| O3 | Plan 8.5 "no tint in Preview" | not looked at in a screenshot (the rule is covered by code review in WP3b and the highlight condition; no scenario takes a Preview shot with a selection) |

## 1. Workspace tests (`cargo-gnu.cmd test --workspace --no-fail-fast`, at `fe855c4`)

**1,292 passed / 1 failed / 47 ignored** (`target\objtabs\gate\test.log`).

| Binary | Passed | Failed | Ignored |
|---|---|---|---|
| `wte_app` | 415 | 0 | 23 |
| `wte_core` lib | 821 | 1 (WA20, R1) | 24 |
| `tests/brush_tips.rs` | 15 | 0 | 0 |
| `tests/end_to_end.rs` | 9 | 0 | 0 |
| `tests/objects.rs` (new) | 8 | 0 | 0 |
| `tests/project_roundtrip.rs` | 24 | 0 | 0 |

Baseline (`main`): 1,232 / 1 / 43. +60 passing tests, +4 ignored (GPU / perf), no new failure.

## 2. Clippy

`cargo-gnu.cmd clippy --workspace --all-targets`: **0 warnings** (`target\objtabs\gate\clippy.log`).

## 3. Release build

`cargo-gnu.cmd build --release -p wte_app`: OK (SHA-256 above).

## 4. Ignored test suites

| Command | Result | Baseline / earlier |
|---|---|---|
| `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1` | 2 passed; check:A slowest 66.6 ms, WA7 slowest 92.2 ms (limits 200 ms); 33.8 s | baseline 75.9 / 94.9 ms, 36.2 s (run 2: 66.7 / 87.6 ms, 34.0 s) |
| `cargo test -p wte_app perf_ -- --ignored --test-threads=1` | 2 passed (font sample job 197 ms warm; the visibility test needs `WTE_HEAVY_MODEL`) | — |
| same, `--release`, `WTE_HEAVY_MODEL` = ABeautifulGame.glb | isolate 2 objects: mask 2.6 ms + scene sync 30.7 ms = **33.3 ms**; hide 1: 20.2 ms; show all: 2.2 ms (budget 250 ms) | WP3b: 36.0 / 20.6 / 1.9 ms |
| `cargo test -p wte_app -- --ignored --skip perf_` (GPU) | **21 passed**, 0 failed | baseline 18 + 3 new |

Logs: `target\objtabs\gate\perf_core.log`, `perf_app.log`, `perf_toggle.log`, `gpu_tests.log`.

**Brush on the heavy model (risk 3):** `brush_types_path` on ABeautifulGame.glb with `WTE_STROKE_LOG=1`, baseline exe
vs gate exe, 7 strokes each (`target\objtabs\gate\stroke_wte_base`, `stroke_wte_gate`):

| Exe | setup sum | raster sum | BVH query sum | commit sum | candidates |
|---|---|---|---|---|---|
| baseline | 3,448 ms | 34,017 ms | 3,726 ms | 87 ms | 36,069,817 |
| gate | 3,456 ms | 34,487 ms | 3,728 ms | 90 ms | 36,069,817 |

Same work (identical candidate count), times within run-to-run noise (WP2 measured the same spread between two
baseline runs).

## 5. Scenario sweep — every scenario `"ok"`

`sh tools/objtabs/sweep.sh target/objtabs/gate/wte_gate.exe cmp/run tools/objtabs/jobs_gate.txt` (cwd
`target\objtabs`, the same output path as the baseline sweep so printed paths match), moved to
`target\objtabs\cmp\gate\` (`summary.txt`). 62 jobs, **61 `"ok": true`**, 1 `false` (R2, pre-existing):

- All 50 names of `NAMES` were run. Without PATH at 1280x800, all `"ok": true`: empty, smoke, default, paint3d,
  paint2d, stamp, text, clay, preview, export, export_run, multi, roundtrip, undo, import, import_tex, import_nouv,
  unsaved, layers, pan2d, brush_types, brush_shape, brush_tab, text_gaps, text_styles, text_warp, text_path, text_tab,
  text_fonts, brush_hover, final_bake_async, wrap_surface, export_selected, export_zip, export_flow, export_unsettled,
  save_unsettled, export_nouv, export_errors, export_checks, **side_tabs**, **objects**.
- With PATH: open_path (ABeautifulGame, BoxTextured), workflow_path (BoxTextured), fuzz_path (BoxTextured, 29 s),
  stamp_edge_path (Avocado), brush_types_path (BoxTextured), text_preview_path (BoxTextured), wrap_surface (Avocado),
  import_save (coincident_flipped_spheres.obj), **objects_path (ABeautifulGame and Lantern)**: all `"ok": true`.
  import_save on BoxTextured: false (R2).
- Small windows: brush_tab, text_tab, layers, **side_tabs** at 900x600 and 640x480: all `"ok": true`.

Measurements logged by the new scenarios:

| Run | Value |
|---|---|
| `objects_path` ABeautifulGame | 49 objects, 1,498,080 triangles; 21 rows laid out in a 438 pt list (limit 21); **isolate frame 5.6 ms** (budget 250 ms; mask 2.2 ms + visible geometry 2.1 ms in the log); Frame Selection: target on the selection's centre, distance 1.52 -> 0.79; the last row revealed by a Navigate pick after the list was scrolled to the top; timed stroke: longest frame 5.1 ms (budget 140 ms) |
| `objects_path` Lantern | 3 objects; all rows fit (the reveal check is skipped, logged); isolate frame 0.6 ms; stroke 4.5 ms |
| `objects` | the stamp's ink per region (Floor, Cube, Ball) = (12,404, 0, 0) with the Cube hidden; Frame Selection on the Ball: distance 12.84 -> 2.60, target exact; reopen toast "1 object is hidden: Cube (Object > Show All)" |

During the gate one scenario bug was found and fixed (`fe855c4`): `objects_path` asserted that the last row is out
of view after scrolling the list to the top, which cannot hold for a list that fits whole (Lantern). The check now
applies only when the list scrolls; ABeautifulGame still tests the reveal. The whole sweep was then re-run with the
fixed exe (the numbers above).

## 6. Pixel identity (plan 6.6: the view rect, toast area excepted)

`python -E -P tools/objtabs/compare_views.py cmp/base cmp/gate` (`cmp/base` = the baseline exe's sweep into the same
path; `target\objtabs\cmp\compare_views_gate.txt`): **63 identical, 10 different** among the 1280x800 shots both runs
have.

| Shot | Differs in | Cause |
|---|---|---|
| `export_flow\shot.png`, `export_selected\shot.png`, `export_selected\shot_dialog.png`, `fuzz_path_box\shot.png` | whole view | WP4/5: the baseline's side panel had grown to 304-316 pt (narrower view); view-relative or random clicks land elsewhere |
| `wrap_surface\shot.png`, `wrap_surface_avocado\shot.png` | the stamp | WP4/5: the removed "collapse the Color section" click used to move the pointer off the stamp (hover look only; documents identical) |
| `text_fonts\shot_picker.png` | the font popup | WP4/5: the popup hangs from the font button, which moved with the Tool page heading |
| `final_bake_async\shot.png`, `brush_shape\shot_dialog.png`, `export_checks\shot.png` | y 623-702, the bottom-right toast stack only | toast fade timing (looked at: the same toasts at another alpha); the toast area is excepted |

Against the WP4/5 builder's sweep (`cmp/wp45`, `compare_views_gate_wp45.txt`): 80 identical, 3 different — exactly the
three toast-area shots. So WP6 changed nothing in the views of the existing scenarios.

## 7. Exports

- `tools/round3/check_exports.py` on every folder of the gate sweep that holds a PSD or ORA (9 folders: export_flow
  fbx / glb / gltf / obj loose, export_nouv a / b, export_run, export_zip ref, workflow_path_box): **PASS (0 failing
  sets)** (`target\objtabs\cmp\check_exports_gate.txt`). (The PNG-only and zip-only folders have nothing for it to
  check; it reports them as "no .psd or .ora file", the same with the baseline exe.)
- File by file (`tools/objtabs/compare.py cmp/base cmp/gate`, `compare_files_gate.txt`): 13 exports differ (R3),
  every other export file is byte-identical. Against the WP4/5 sweep (`compare_files_gate_wp45.txt`): **no export
  differs**. `.wtep` files always differ (they hold the save time).

## 8. Screenshots looked at (Read tool)

- `objects`: `selected` (the Cube tinted blue, its row filled, Navigate active), `hidden` (no Cube and no Cube wires,
  wireframe on, the Ball's and Floor's wires drawn), `hidden_2d` (texture whole; the Cube's UV quadrant has no wires,
  the Ball's and Floor's have), `shown` (the Cube back, clean; the stamp gizmo behind it), `isolated` (only the Ball,
  tinted, framed close; Floor / Cube rows dimmed with closed eyes), final (Cube hidden after the reopen; the red stroke
  and the Logo stamp on the floor where the Cube stands; the toasts "Saved shot.wtep" and "1 object is hidden: Cube").
- `objects_path` ABeautifulGame: `isolated` (King_B and Bishop_W2 alone, tinted; header "49 objects · 47 hidden ·
  1,498,080 triangles", the list scrolled to Bishop_W2), final (every piece back, Bishop_W2 tinted and its row selected
  at the list's end, the green stroke on King_B; the green tops of the black pieces are the model's own texture — the
  baseline `open_path_abg` shot has them). Lantern final: three rows, the lantern selected and tinted, two green strokes.
- `side_tabs` 1280x800: `tab_color` (whole picker, 140 pt square, hex row, two swatch rows, no scroll bar),
  `tab_objects` (Cube selected and tinted, Ball hidden and gone, footer on one row clear of the split), `tab_text`
  (Color page above, Text tab below, nothing overlapping). 900x600 `tab_objects`, 640x480 `tab_brush` (the square
  shrinks, both strips visible, nothing drawn over the other group).
- The three toast-area diffs (before / after crops `target\objtabs\cmp\toast_*.png`).

## 9. Docs

ARCHITECTURE.md: new 1.9 (summary, the hidden-object rules, view state, no version change, the superseded colour
picker decision), 7.3 note, 8.11 (count 50, timeouts, `objects` / `objects_path` rows), 9.2 (WP6 paragraph, the gate
sweep tools), section 10 (objects-tabs package list). HANDOFF.md: 4.1 (scenario list incl. the three new names, the
multi-object PATH), 5 (history row), 6 (the colour picker decision struck through and marked superseded 2026-11-24
with its mitigations; the new tab / object decisions). `dist/HOW_TO_TEST.txt`: "Objects" and "Side panel tabs"
checklists including the Alt+H question. Builder notes: `docs/notes/objtabs/wp6.md`.

## 10. Commits of WP6 (all pushed to `origin/objects-tabs`)

- `aff96a9` scenarios `objects`, `objects_path`; `tools/objtabs/` (sweep, jobs, compare scripts); ARCHITECTURE 8.11
- `b95bbb9` ARCHITECTURE 1.9 / 7.3 / 9.2 / 10, HANDOFF, HOW_TO_TEST, notes
- `fe855c4` `objects_path`: the reveal is checked only when the list scrolls
- this report

Nothing was merged to `main`.

## Critic (completeness check, 2026-11-24)

The question: what of the request (1 select and hide objects of a multi-object file, 2 two tab groups with a draggable
split, 3 an object menu with Hide / Isolate / Show all / Frame; selecting must not limit painting; hidden objects are
not drawn and do not block painting, stamps or wraps) is missing or not proven? I read the plan, this report, the
builder and check notes, and the tests and scenarios behind each claim. Everything below was run on this machine from
this worktree (RTX 5070, Vulkan, off-screen runs only).

### What was missing

| # | Severity | Finding | Status |
|---|---|---|---|
| C1 | major | **The user's whole workflow on a real multi-object file was never run.** `objects` covers hide / paint / stamp / save / reopen on the built-in Three Objects sample (no wrap, no text, no export); `objects_path` covers the list, isolate, frame, a pick and a stroke on ABeautifulGame (no hiding of one object while painting another, no stamp, wrap or text, no save, reopen or export). Nothing showed that a project of a real glTF re-imports its objects under the same names on reopen, so that the stored hidden names find their object again. | fixed: new scenario `objects_flow_path` (commit `5e67b58`), below |
| C2 | minor | The contract "changing visibility never re-bakes existing stamps" (plan 3.4) had no test above the core. | fixed: asserted in `objects_flow_path` step 5 |
| C3 | minor | O3 ("no tint in Preview") was not looked at in a screenshot. | fixed: `objects_flow_path` takes a Preview shot while the pawn is selected (no tint, board left out; looked at) |

No test was weakened by this branch as far as I can tell: the edits to the panel and scenario tests in
`git diff 1f0be41..` keep the intent listed in plan 6.6 (the old "Color section" checks became the upper group's
checks; the two "collapse the Color section" clicks were removed because that header no longer exists). Every plan
item I spot-checked is present: Object > Objects Panel, the Navigate hint, double-click on a row to frame, a click
below the rows to deselect, the primary-colour chip opening the Color tab, the split double-click, tabs and split in
the prefs, `hiding_drops_cached_wrap_params`, the GPU tests of plan 5.6.

### `objects_flow_path` (PATH = `ABeautifulGame.glb`)

The scenario finds the board (the object with the largest x/z footprint: `Chessboard`, set 4, used by no other
object) and a pawn (`Pawn_Body_W1`, set 6, shared by the eight white pawn bodies). Steps and what each asserts:

1. Hide the board from its row (revealed in the list, H): 129 of 144 vertical rays over the board hit it while it is
   shown, **0** while it is hidden; revision, modified flag and history unchanged.
2. The pawn from its row, Shift+F (camera target on the pawn). A Navigate click on a point where only the hidden board
   is **deselects** (the hidden board is not picked); a click on the pawn selects it.
3. A brush stroke from the pawn's centre out to a point whose ray would hit the board if it were shown (checked with
   an unmasked copy of the BVH, so the check is real): the pawn's set changes, **set 4 does not**, one undo entry.
4. A projected image stamp (size 0.4, larger than the pawn, so the board behind it would be covered), a **wrapped**
   stamp with "jump small gaps" on, low on the pawn where the board touches its base, and a text: each one inks the
   pawn's set and puts **no ink in a layer of set 4**; set 4's composite never changes. The wrap jumped twice (onto
   the pawn's top), never onto the hidden board.
5. With the wrapped stamp selected (Transform), Show all, then the board hidden again: composites and history
   unchanged both times, so nothing was re-baked. The log shows how much the check means: with the board shown, the
   stamp's new parameterization covers 431,174 triangles (it jumps onto the board) instead of 49,440.
6. Navigate with the pawn selected: tint on (`highlight_shown`). Shots `designed` (tinted pawn, no board) and `preview`
   (no tint, no board).
7. Save As, reopen: the board is hidden again (the flags, and the BVH mask: 0 ray hits), one toast "1 object is hidden:
   Chessboard (Object > Show All)", not modified, nothing selected, structure and composites as saved.
8. Export one layer (the pawn's paint layer, per-layer PNG): exactly that file, equal to the layer, 20,584 inked
   texels. Export every set's flattened PNG: 15 files, each its set's composite (`verify_export`), the hidden board's
   set included.
9. Show all: the rays hit the board again. Then the pawn hidden and a short stroke on its screen centre: it paints the
   board behind it, the pawn's set is unchanged, the history ends with "Brush Stroke".

Result: `"ok": true` in two runs (`target\objtabs\critic\flow_abg\`, `flow_abg_run2\`, release exe of `5e67b58`,
SHA-256 `6EC2A1E0BE6A5CDA0DDECF86A52F0FF3E63F551F91C79EE806D8D6B624030D11`), about 25 s each. Screenshots looked at:
`board_hidden` (pieces stand on nothing, Chessboard row dimmed with a closed eye, header "1 hidden"),
`piece_selected` (the pawn tinted blue, its row selected), `designed` (stroke, stamps and text on the white pawns,
which share one texture, and no board), `preview` (no tint, no board), `reopened` (same picture, the toast),
final (board shown again and clean: no paint or stamp ink on it).

The scenario needs a model whose board-like object has a texture set of its own and fails with a clear message
otherwise. It is in `tools/objtabs/jobs_gate.txt`, ARCHITECTURE 8.11 / 9.2 and HANDOFF 4.1 (scenario count 51).

### Notes for the user (not changed)

- **After Show all, a selected wrapped stamp's outline describes the new surface, not its pixels.** In step 5 the
  Transform tool rebuilt the parameterization for the new visibility (6.7 s on a worker for the board). The pixels
  stay as baked; the next edit of that stamp bakes it again with what is visible then. This is the planned rule (plan
  0.1 item 2, 3.4), but on a real model it shows: a wrap placed with the board hidden spreads onto the board once it is
  shown and the stamp is touched.
- **The Layers panel does not follow the object selection.** Selecting the pawn leaves "Texture set: King_Black" in
  the Layers tab. Not part of the plan; a possible later convenience.
- **Seen, not caused by this branch:** a 3D brush stroke first adds an empty paint layer to every set whose active
  layer is a stamp or text layer (Brush row step 4), so such a stroke makes more than one undo entry; and a plain
  Transform click on a stamp (no drag) baked it again and left an "Edit Stamp" undo entry in one of my runs.
  `tools/transform.rs` changed only in its wrap-failure key here, so this is older behaviour; not investigated
  further.
- Still manual (unchanged): O1 Alt+H on Windows, O2 the split double-click with a real mouse.

### Checks after the critic's change (`5e67b58`)

`cargo-gnu.cmd test --workspace --no-fail-fast`: **1,292 passed / 1 failed (WA20, known) / 47 ignored** (same as the
gate; `scenarios::tests` now counts 51 names); `test -p wte_app` again after the last edit: 415 passed / 23 ignored.
`clippy --workspace --all-targets`: 0 warnings. Release build OK. Logs `target\objtabs\critic_*.log`.

**Verdict of the critic:** the gate's PASS stands. The one major gap (the end-to-end workflow on a real multi-object
model) is closed by `objects_flow_path`, which found no defect in the feature.
