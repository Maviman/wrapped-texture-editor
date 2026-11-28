# Sticker gate report (STICKER_PLAN 7 "Gate", 8.6)

Branch `stickers` (worktree `G:\software_projects\texture_wrapper_ap_stk`), plan `docs/plans/STICKER_PLAN.md` (revision 3).
Gated commit: **`8c8a297`** (the S2 check, the last package commit: P0-P13, S1 and S2 built and checked). Base: `main` on
GitHub = `b2fb8f5` (v0.3.0); the local `main` ref of this clone is older (`1b589a1`), so every comparison below is made
against `b2fb8f5`. Gate run 2026-11-27 on the user's desktop (RTX 5070, Vulkan). This report changes no code. A
completeness critic appends its own section below this report (plan 7 "Gate").

- Release exe: `target\release\wrapped_texture_editor.exe`, 33,910,784 bytes, SHA-256
  `F7939144C878D2BC7A2159F207E65D97BE002B0AF974132C01F0B263A690AB70`. (Release builds of this toolchain are not
  reproducible byte for byte, see `docs/notes/sticker/baseline.md`; the hash only names the exe that was tested.)
- Baseline: the P0 exe `target\sticker\baseline\wte_p0_baseline.exe` (= `b2fb8f5`) and its sweep
  `target\sticker\baseline\sweep\`.
- Raw outputs of this gate: `target\sticker\gate\` (git-ignored): `test.log`, `clippy.log`, `build.log`,
  `perf_core.log`, `perf_app.log`, `gpu_tests.log`, sweeps `all\`, `ole\`, `identity\`, `frames\`, `small\`, `p0_all\`,
  `p0_small\`, `identity_compare.txt`, `shots_expect.tsv`, `sheets\` (the shots looked at), `focus\probe.txt`.
- Job lists (committed with this report): `tools/sticker/jobs_gate.txt` (every name of `NAMES`, the `*_path` ones with a
  model, 82 jobs) and `tools/sticker/jobs_gate_ole.txt` (the 21 new scenarios, 23 jobs).

## Verdict

**PASS.** Every gate criterion of 8.6 is met. The three red items are all pre-existing and red with the P0 exe as well;
none comes from this branch. Nothing here has been tried with the real PhotoCraft, the real Photoshop, a real drag between
two programs or the real clipboard: those are the hand tests of section 9, and per plan 8.6 the branch lands only after
the user reports hand tests 1, 3, 4, 7, 8, 12, 15, 19, 21, 23 and 24 (or says "land, tests owed").

| # | Item | Status |
|---|---|---|
| R1 | `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` fails | the known red (the same at P0: 1,411 / 1 / 48) |
| R2 | scenario `import_save` on BoxTextured.glb: `"ok": false` ("the source bundle has 0 sidecar(s)") | pre-existing, the same with the P0 exe (`p0_all`); the scenario needs a model with sidecar files and passes on `coincident_flipped_spheres.obj` (earlier gate reports R2) |
| R3 | scenario `companion_host`: `"ok": false` after 300 s ("the expected sessions drew a pen stroke...") | pre-existing, the same with the P0 exe; it waits for a real tablet or the emulator and was left out of the companion gate's sweep for that reason |

Gate findings (none is a blocker; not fixed, report only):

| # | Severity | Finding |
|---|---|---|
| G1 | minor | Three check notes carry a stray carriage return in a path (`docs/notes/sticker/check-P10b.md` "`check-p10b\runs_fixed`", `check-P5c.md` "`target\release\`", `check-P8.md` "`p8check\run2`": a `\r` was written as a control character). Git therefore classifies them as binary (`i/-text`) and the path reads wrong. The S1 and S2 notes had the same slip and were fixed; these three were not. |
| G2 | minor (layout) | At 1280x800 the link block of a linked active paint layer pushes the layer rows down: one row stays visible in `flow_saved` / `layer_link_made`, and **none** in `layer_link_ps_merged`, where the Photoshop "needs a merged picture" sentence is shown (the list scrolls). Known since the P12 check ("about 1.5 visible layer rows"); a layout decision for the user. |
| G3 | wording | `sticker_view`'s shot `sticker_small` says "all 14 sticker tools"; since S1 there are 15 (Cut Out Box). The shot shows all 15. |
| G4 | note | `sticker_watch_folder` `watch_reopened`: when the watched file becomes wider (96 -> 160 px, the picture centred in it), the disc on the model moves 32 layer pixels to the right. This is plan 2.9's refit (layer space is anchored, the artboard grows to the right) and S2 deviation 7; a user re-exporting a wider asset may expect it centred. |
| G5 | note | `perf_k7a` Preview restyle job: 66.8 ms in the first serial run (budget 60, 11 % over, off the UI thread); three reruns 56.6 / 58.5 / 57.0 ms. Counted as within budget; it sits near the limit. |
| G6 | note | `crates/wte_core/tests/fixtures/psd/gen_f1/Cargo.toml` has a `[dependencies]` block (`wte_core` by path, `fontdue`). It is P0's fixture generator in its own `[workspace]`, not a member, never built by `cargo test --workspace` (P0 deviation 2); `Cargo.lock` is unchanged apart from the version. Read against 8.6 item 5's "no new `[dependencies]` line anywhere" literally, this is one; accepted as a test tool. |

## 1. Workspace tests (`cargo-gnu.cmd test --workspace --no-fail-fast`)

| Commit | Passed | Failed | Ignored |
|---|---|---|---|
| P0 baseline `7969fcb` (= `b2fb8f5` + plan) | 1,411 | 1 (WA20) | 48 |
| **gate `8c8a297`** | **1,964** | **1 (WA20)** | **71** |

Per binary: `wte_app` 778 / 0 / 27; `wte_core` lib 1,129 / 1 / 44; `brush_tips` 15; `end_to_end` 9; `objects` 8;
`project_roundtrip` 25; doc tests 0. +553 passing tests (plan 8.1 expected about 330-430). The identity tests of plan
8.6 item 8 are in this run and pass: `k4_a_v3_project_of_the_p0_exe_loads_as_before` (the v3 fixture written by the P0
exe opens and its layer PNG entries are byte-equal on a re-save) and
`k4_check_the_p0_version_check_refuses_a_sticker_project_and_opens_a_plain_one` (the P0 version check, copied verbatim
from `b2fb8f5`, refuses a v4 project with a sticker with "project was saved by a newer version").

Ignored GPU / control-sweep app tests (`cargo test -p wte_app -- --ignored --skip perf_ --skip write_icon_png
--test-threads=1`, headless wgpu, no window): **21 / 21 passed** (`gpu_tests.log`; `write_icon_png` is the installer's
icon export tool and writes a file, so it was skipped).

## 2. Clippy and release build

- `cargo-gnu.cmd clippy --workspace --all-targets`: **0 warnings**.
- `cargo-gnu.cmd build --release -p wte_app`: OK (the exe above).

## 3. Scenarios

All runs at 1280x800 (unless named), off-screen, no focus taken, through `tools/objtabs/sweep.sh`.

| Sweep | Jobs | ok | not ok | Output |
|---|---|---|---|---|
| every name of `NAMES` (75 names; `*_path` ones on ABeautifulGame, BoxTextured, Avocado, Lantern, DamagedHelmet, and `sticker_link_path` on `f1w_pc.psd` and Photoshop's `layers/type-layer.psd`) | 82 | **80** | 2 (R2, R3) | `all\` |
| the 21 new scenarios with `WTE_OLE=1 WTE_PEN_SUBCLASS=1` (8.6 item 3) | 23 | **23** | 0 | `ole\` |
| side panel at 900x600 and 640x480 (`brush_tab`, `text_tab`, `layers`, `side_tabs`, `pen`) | 10 | **10** | 0 | `small\` |
| P0 exe: the 59 jobs of the old scenarios | 59 | 57 | 2 (R2, R3: the same two) | `p0_all\` |
| P0 exe: the small sizes | 10 | 10 | 0 | `p0_small\` |

So every old scenario that passed at P0 passes now, and every new one passes, with and without WTE's own OLE drop target
and the pen subclass. Every `WTE_OLE=1` log has "ole: WTE's drop target is registered (winit's revoked: yes)" and "the key
subclass is installed". Run times of the new scenarios: 1-11 s each, `sticker_link` 28 s, `sticker_flow_path` 39 s on
ABeautifulGame and 11 s on DamagedHelmet.

### 3.1 Every new shot against its `expect` (8.2, 8.6 item 3)

91 shots with an `expect` text (the 21 new scenarios; `shots_expect.tsv`). I looked at every one (two-up sheets in
`sheets\`, and full-size crops where the detail is small). **91 / 91 match**; four remarks are in the last column.

| Scenario | Shots | Result |
|---|---|---|
| `layer_link` | layer_link_menu, _made, _updated, _conflict, _reopened, _ps_size, _ps_merged | match (the row menu also has S2's "Keep a PNG Copy up to Date"; G2: no layer row visible under the Photoshop sentence in _ps_merged) |
| `sticker_clip` | clip_paste_source, clip_view_centre, clip_sticker_layer, clip_text | match |
| `sticker_controls` | controls_tab, controls_view | match |
| `sticker_cutout` | cutout_delete, cutout_removed, cutout_color_to_alpha, cutout_die_cut | match |
| `sticker_dragout` | dragout_after_row, dragout_sticker_tab, dragout_save_first, dragout_psd_toast | match |
| `sticker_drop` | drop_hover_picture, drop_model, drop_three, drop_canvas, drop_hover_project | match |
| `sticker_edit` | edit_brush, edit_styled, edit_final | match |
| `sticker_flow_path` (ABeautifulGame) | flow_dropped, _sticker_view, _styled_wrapped, _linked, _updated, _style_pasted, _saved, _reopened | match (the wrapped sticker is small on the board: the white border was checked on a crop; the badge's projection box reaches the pieces beside it, P13 deviation 5) |
| `sticker_flow_path` (DamagedHelmet) | the same 8 | match (the photo also shows on the left of the shell, where the helmet's texture is mirrored: a UV effect of the model, as the P13 builder noted) |
| `sticker_grabcut` | grabcut_cut, grabcut_mask, grabcut_die_cut | match |
| `sticker_keys` | keys_inside | match |
| `sticker_layout` | layout_crop_box, layout_rotated, layout_done | match |
| `sticker_library` | library_saved, library_placed | match |
| `sticker_link` | link_shared, _made, _updated, _held_applied, _merged, _conflict, _saved_over, _changed_closed, _pc_done, _chooser, _made_ps, _editor_list, _merged_ps, _needs_merged_ps | match |
| `sticker_link_path` | psd_placed (f1w_pc.psd: "Sticker, 3 layers"; type-layer.psd: "Sticker, 1 layer") | match |
| `sticker_live_copy` | live_copy_on, live_copy_edited, live_copy_layer | match |
| `sticker_pen` | pen_canvas | match |
| `sticker_styles` | styles_image, styles_both | match |
| `sticker_text` | text_clipped, text_warped, text_converted | match |
| `sticker_tray` | tray_edit_before, tray_placed_twice, tray_question, tray_only_this | match |
| `sticker_view` | sticker_enter, _moved, _reordered, _small (900x600), _left | match (G3: 15 tools, the text says 14) |
| `sticker_watch_folder` | watch_two, watch_updated, watch_reopened | match (G4; no gizmo after the reopen, "selected" = the active row, as the S2 checker noted) |

The user's own workflow ("I would be able to manoeuvre the sticker in the wrapped app, and I can edit the sticker itself in
Photoshop") is what `sticker_flow_path` and `sticker_link` show: the sticker is moved and resized in WTE after the link is
made (`flow_linked` -> `flow_updated`: the badge stays "linked"), and the editor's save updates the sticker in place,
keeping that placement (`flow_updated`, `link_updated`, `link_merged_ps`: "its placement unchanged since it was placed").

## 4. Identity checks (8.5) against the P0 baseline

`sh tools/objtabs/sweep.sh target/release/wrapped_texture_editor.exe target/sticker/gate/identity tools/sticker/jobs_identity.txt`
(21 jobs, `workflow_path` on ABeautifulGame): **21 / 21 `"ok": true`**. `python -E -P tools/sticker/compare_baseline.py
target/sticker/baseline/sweep target/sticker/gate/identity` (`identity_compare.txt`):

- **214 of 216 files identical**: every export file byte for byte, every saved `.wtep`'s layer PNG entries and manifest
  (`format_version`, `min_reader_version`, `app_version`, `saved_unix` aside), every 1280x800 screenshot's view rectangle.
- New: `side_tabs/shot_tab_sticker.png` (the Sticker tab, which does not exist at P0).
- Different: `side_tabs/shot_small_640.png` and `shot_small_900.png`, which the tool compares whole because they are not
  1280x800. Checked with numpy: rows 0-50 differ only in the mode strip (its "Sticker" button), and below it every
  differing pixel is at x >= 342 (640 wide) / x >= 601 (900 wide), i.e. inside the side panel (the two-row tab strip). Both
  areas are excluded by 8.5. The same result as every package check since P6b.

The tool's verdict line reads "FAIL" because of these two whole-picture compares; by 8.5's rule the identity check passes.

## 5. Performance (8.4): ignored `perf_*` tests, serial, release, idle machine

`cargo test --release -p wte_core --lib perf_ -- --ignored --test-threads=1 --nocapture` (17 tests, all pass) and the
same for `-p wte_app` (5 tests, all pass). Logs `perf_core.log`, `perf_app.log`.

| Operation (8.4) | Budget | Measured |
|---|---|---|
| `apply_styles` ClassicSticker 1024 | 60 ms | **25.2 ms** |
| the same 2048 / 4096 | 250 ms / 1.1 s | **75.5 ms / 271.9 ms** |
| the same at the Preview working size (724 x 724 out) | 33 ms | **11.6 ms** |
| peak extra memory of `apply_styles` at 4096 | 256 MB | **150 MB** (ClassicSticker; scaled to the picture 186 MB and every style on 309 MB are reported only) |
| EDT 2048 | 80 ms | **19.8 ms** |
| `sticker::render` 3 layers + style, 1024: cold / one layer changed | 120 / 50 ms | **31.5 / 4.1 ms** |
| Preview-quality render, one layer changed | 33 ms | **4.4 ms** |
| one brush dab 64 px on 2048 + dirty-rectangle composite | 2 ms | **0.11 ms** (dab), 0.23-0.28 ms (dab + region, `perf_k7a`) |
| `sticker_stroke_begin` on a shared 4096 picture | 40 ms | **12.3 ms** |
| entering stroke mode on a styled 2048 sticker | 50 ms | **30.7 ms** (reruns 27.6-28.2) |
| restyle job during a stroke (Preview) / at the end (2048) | 60 / 300 ms, off the UI thread | **66.8 ms** first run, reruns **56.6-58.5** (G5) / **114.3 ms** |
| wand / magic eraser / remove background 2048 | 60 / 60 / 150 ms | **13.6 / 18.4 / 51.4 ms** |
| Gaussian blur r 16, 2048 | 150 ms | **61.0 ms** |
| flood fill worst case 4096 | 400 ms | **39.4 ms** one colour; the 2,048-teeth comb **354.1 ms** |
| `psd::read` / `psd::write_sticker`, 2048, 5 layers | 150 / 150 ms | **90.6 / 102.8 ms** |
| a settled file change -> the sticker updated on the model | 1.5 s | **1.17-1.23 s** in `sticker_link` ("Until(the update arrives)", 256-px logo stickers: the watcher's settle time dominates); not measured on a 1024 picture |
| watcher repaint requests while nothing changes | 0 in 3 s | **0** (asserted in `sticker_link`: "3 s later: no second update, the same picture, no repaint asked by the watcher") |
| drag-out PNG of a 4096 layer | on a job | job **244 ms** off the UI thread; UI thread `start_job` 0.11 ms, worst step 0.43 ms (`perf_k11`) |
| GrabCut (S1) 2048 | 3 s, on a job | **687 ms** (box), 1,019 ms (whole picture), 505 ms (fragmented worst case) |

Reported without a budget: Merge Down / Flatten at 2048 109 ms each, Trim 38.5 ms (UI thread, P8 open item); linked layer
at 4096: make the PSD 263.6 ms, read 218.4 ms, apply an update 67.3 ms on the UI thread (P12 open item); blur 16 popover
preview of a 2048 styled sticker 170 ms (UI thread, P7b); live PNG copy of a sticker: UI thread 32 ms the first time,
0.06 ms after a change (S2); wrap WA7 137.7 ms and check:A 90.5 ms (limit 200 ms, unchanged); font sample job 211.5 ms.
`perf_visibility_toggle_on_a_heavy_model` measured nothing (`WTE_HEAVY_MODEL` not set; objects-tabs test, not this plan).

**Frame-span limit (8.4)**: `WTE_FRAME_LOG=1` runs of `sticker_view`, `sticker_edit`, `sticker_styles` and
`sticker_flow_path` on ABeautifulGame, `python -E -P tools/sticker/frame_spans.py frames/*/shot.log`: exit 0, **no span
over 50 ms**. Worst values: `sticker.canvas` 4.33 ms, `sticker.render` 4.19 ms, `sticker.live` 1.23 ms, `sticker.region`
0.52 ms, `sticker.outline` 0.05 ms, `ole.drop` 0.08 ms. (`sticker_view`, `sticker_edit` and `sticker_styles` open the
sphere sample themselves; only `sticker_flow_path` takes ABeautifulGame.) Not limited by 8.4, recorded: the worst frame of
the flow run is 3.5 s of the `selftest` segment (the scenario's own save / reopen / hash steps) with a 745 ms `scene sync`
when the reopened project loads the model.

## 6. Audits

| 8.6 item | Check | Result |
|---|---|---|
| 5 | `git diff b2fb8f5 -- Cargo.lock`: only `wte_app` / `wte_core` 0.3.0 -> 0.4.0; workspace `Cargo.toml` only the version; no other manifest changed | pass (G6: the fixture generator's own manifest) |
| 6 | no process started for an editor: `tasklist` for `photocraft.exe`, `photocraft-cli.exe`, `Photoshop.exe` before and after all sweeps: none; every one of the 82 sweep logs says "editor link: the stub launcher (nothing is started)"; no "starting <exe>" line in any log | pass |
| 6 | the real launcher exists only in a non-test build and only for `launcher_for(false)` (a self-test passes `true`); it spawns with stdio null and `CREATE_NO_WINDOW`; adb (companion) also uses `CREATE_NO_WINDOW`; "Show file in Explorer" starts `explorer.exe /select,` without that flag, by design (it opens a window at the user's click, never in a self-test); the only other spawn is the pre-existing test `cmd /c exit` in `io_flows.rs` | pass |
| 6 | focus: `tools/stable/focus_probe.ps1` (read-only sampling of the foreground window every 25 ms) on `sticker_view`, `sticker_link`, `sticker_dragout`, `layer_link`: **the app was in the foreground in 0 of 1,368 samples** (the foreground changes counted there were the user's own windows); no `FlashWindow`, `SetForegroundWindow`, `MessageBeep`, `PlaySound`, `BringWindowToTop`, `SetActiveWindow` or `AllowSetForegroundWindow` anywhere in `crates/wte_app/src` | pass |
| 6 | clipboard: the Windows clipboard code is `#[cfg(all(windows, not(test)))]`; every sweep log says "clipboard: in memory (nothing reaches Windows)" | pass |
| 0.4 rule 5 | registry: read only (`RegGetValueW`), `NoRegistry` in every self-test and test build; prefs: `%APPDATA%\WrappedTextureEditor` holds only a `log.txt` of 2026-11-25, untouched by the sweeps; `%LOCALAPPDATA%\WrappedTextureEditor` (the drag-out folder) does not exist: self-test runs write the drag-out folder next to their output | pass |
| 7 | LF: of the 330 files the branch adds, 200 are LF text, 130 binary (72 `.psd`, 4 `.psb`, 50 `.png`, 1 `.wtep`, and the three notes of G1); of the 74 edited files the six CRLF ones (`error.rs`, `project/{manifest,migrate,mod,save}.rs`, `text/frame.rs`) stayed CRLF, with every added line CRLF; the repository has 29 `w/crlf` files, as at the base | pass (G1) |
| 7 | licence: `licenses/photocraft-MIT.txt` (MIT text, the copyright line of plan 4.0); the installer installs `licenses\*` into `{app}\licenses`, the CI stages it, the smoke test checks the file; About > Licences... shows it; the notice "Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt)." heads the ported files `sticker/{adjust,cutout,filter,fx,grabcut,resample,select,shape,style}.rs`; the Photoshop fixtures carry `LICENSE-psd-tools.txt` in the test folder and are never shipped | pass |
| 7 | "ArtCraft" (`grep -rIil` over `crates`, `dist`, `installer`, `README*`, `licenses`, `.github`): only `licenses/photocraft-MIT.txt` | pass |
| 7 | "Adobe" (`grep -rIi`, Photoshop fixture folder left out): the trademark sentence (`README.md`, `dialogs/about.rs` twice) and test data (the Photoshop install path in `link/editor_tests.rs`; the ICC profile name "Adobe RGB (1998)" read from a fixture in three core tests). No image asset was added outside the test fixtures | pass (test data accepted, as P13 deviation 8 proposes) |
| 8 | v3 fixture opens and exports identically; a v4 sticker project refused by the P0 check | pass (section 1, and the `roundtrip` / `workflow_path` / `objects` / `wrap_surface` projects of section 4) |
| 9 | the hand-test sheet: `dist/HOW_TO_TEST.txt` "Stickers: the hand tests to report (1-25)" lists 1-25 (with 2b) and the landing list; the chapters "Stickers", "Working with PhotoCraft", "Working with Photoshop", "Drag, drop, copy, paste" hold the steps | pass; all owed (section 9) |

## 7. Deviations from the plan (collected from the build and check notes)

Each is described with its reason in the named note (`docs/notes/sticker/build-<P>.md`, "Deviations"). None was found
wrong by its checker; together they are the plan as built.

- **P0**: f4's mask from an elliptical `select.rect` instead of `select.subject`; f1's input written by a separate
  generator `gen_f1/` (own workspace, G6); `compare_baseline.py` / `jobs_identity.txt` written in P0; volatile screenshot
  areas masked (companion dialog port / code, export folder field).
- **P1**: style margin = contour + outside stroke, plus max(shadow, glow) (the plan's max was too small); thresholds take
  the max with their source; fill-holes keeps the outer anti-aliased edge; bevel strength as the reference; `Paint::Linear`
  carries an angle; gradient by pixel centres; resampler details (mip choice by the larger axis, exact copy for grid
  quarter turns, a clip and a 2^26 cap); shape raster bounds, `Line` = horizontal capsule; preset values chosen; the A7 test
  reading; perf row budgeted as written.
- **P2**: feather / shrink treat the artboard border as content; Remove Background's background colours by seeded clusters;
  its soft edge normalised by the nearest inside pixel; matte test bound; diagonal-only wand fringe kept as the formula;
  blur keeps RGB at alpha 0; zero-length gradient = last stop; pure functions over equal-size buffers; flood via a byte
  plane.
- **P3a**: "styled" = any style on; `bake_placement` follows the current picture's level; working-size limits (4096 in,
  6144 out); render scale computed per bake; promoted images use their own mips; `LayerKey` without sticker id; render
  cache 8 entries and 512 MiB; text layers within 8-512 px per em; `StyledEntry` caches tint and quality;
  `apply_wrap_preview` of an older sticker applies as last good; save refused stickers until P4 (then allowed).
- **P3b**: `PixelData` instead of `TileData`; patches generic by a const parameter; `TileBuf for Arc<T>`;
  `Edit::StickerPixels` target in the patch; a new picture version per dab; in-place writes when unshared; own dab kernel
  without UV wrapping (no pressure-sized walk); empty strokes leave nothing; blob picture edits are structure entries; dab
  rectangles in content pixels; checkpoints include sticker stamps; `sticker_stroke_break`; bakes wait during a sticker
  stroke; the "before" snapshot only on a structure change.
- **P4**: existing CRLF files kept CRLF; picture / mask entries are `FileRef`s; shape geometry under `form`; stamps of a degraded
  sticker re-baked at load; an invalid sticker entry lost with `StickerLost`; links parsed, used from P5; `.wtes` details;
  extract renders on the calling thread; v3 hashes recorded on P3b (identical reader); fixture added with `git add -f`;
  the 200 random save sequences as one chain; font fallback in sticker text layers.
- **P5a**: `PsdError::Unsupported(PsdRefusal)` (+ `Truncated`, `Corrupt`, `TooLarge`); more `PsdDoc` fields; GROUP forces
  merged also when hidden; `lsct` blend key replaces the record's; VECTOR_MASK never on shape / fill layers; padding
  re-sync; `Lr16`/`Lr32`/`Layr` read only without layers; per-pixel un-matting bound against PhotoCraft's PNG; layer opacity
  applied in PhotoCraft's layer PNGs.
- **P5b**: writer input types designed; `PsdError::{Invalid, Io}`; bottom layer written unclipped; masks normalised;
  layer parts outside the canvas kept; resource 1005 in inches; resource 1044 = max(seed, largest id); no layers = flat;
  hidden clipping base hides its clipped layers; walker rules measured on 54 Photoshop files; PhotoCraft's own files fail
  the strict walker (recorded); `psd_check.py` limited by Pillow; control characters -> `?`.
- **P5c**: `link/` as a folder and `doc_links.rs`; the renumber rule also matches by name; the sticker hash covers only
  what goes into the PSD (styles never disturb the link: the user's rule); change = size + CRC, not mtime; an empty sticker
  is written with one placeholder layer; `.bak.psd` by copy then rename; links outside the history with their own
  revision; "readable" includes a whole-file refusal; `LinkAtLoad` carries path and archive; bad link entries dropped with
  a log line; `blob_sizes` handed in; > 32 layers or too many bytes come back merged; problems stored with their CRC;
  untouched opacity keeps WTE's exact value; new fixture f15.
- **P6a**: 25 scenario literals got `.into()` and old shots `expect: ""`; canvas rect outside `view_rects`; tools not built
  yet shown dimmed; no row menu / multi-select yet (came later); E6 moved to P6b; whole-texture upload (partial in P7a);
  read-only core additions; arrow nudge, menu mapping, Backspace, Enter added; `prefs.rs` unchanged; `view3d.rs` (CRLF)
  not edited; Shift+E / Shift+G outside the view unchanged.
- **P6b**: no `StickerTarget::ToolSource` (a tray sticker instead); link / handle / library / clipboard parts left to their
  packages; thumbnails without stamp styles; "Apply to all selected" sends the whole stack; presets scaled per stamp
  (`set_styles_each`); core additions; Stamp tool styles as a list; E6 details; the split clearance fix
  (`side_tabs` 640x480); G1-G4 checked at 1280x800 / 900x600; `sticker_styles` checks the box, not the depth; Edit-menu
  style items clicked through AccessKit; "Paste / Clear Layer Style" labels.
- **P7a**: `sticker/live.rs`; the Eraser inside `brush.rs`; the model's Preview renders off the UI thread but the bake runs
  on it; jobs as plain threads; no button in toasts (Rasterize in the header and Tool tab); Rasterize built now; core
  additions and a row copy in `copy_exact`; `render_region` equals the render on seen pixels; selection commands for the
  scenario; small rules (hidden layers not painted, "Add mask" turns Paint on mask on, ...); the Brush tab in the view.
- **P7b**: extra files `filters.rs`, `popover.rs`, `outline.rs`; popovers as non-modal windows with a canvas-only preview;
  the preview on the UI thread (255 ms at 2048 with a styled layer); Color to Alpha picks by a canvas click (+ Primary);
  "Color" spelling; Move Selection only inside the selection; Move Selection as `set_sticker` Previews; a cut's new layer
  copies attributes; Layer via Copy / Cut and Crop drop the selection; Invert with nothing selects all; Blur / Sharpen
  refuse > 4096 px; the Sticker menu only in the view; command blocks at the end of the Tool tab; unchanged results make
  no entry; an empty selection is none.
- **P8**: layout commands in the core (`sticker/layout.rs`, `doc_sticker_layout.rs`); Resize keeps the placement
  (`Refit::Scaled`); quarter turns with a half-pixel pivot; Keep aspect option inverted by Shift; Trim = visible content;
  Merge Down / Flatten bake at scale 1 inside the artboard; Crop deletes pixels only of plain moved layers; text layers
  without per-gap editing; the Text tool focuses the Tool tab field; Help left to P13; numeric commands in popovers; the
  library keeps a stamp's styles; Layer group in the Sticker menu.
- **P9a**: scenario step 5 holds the update with a focused text field; the first-time note is a toast; badge "menu" as a
  row of buttons (tablet rule); the editor's exe is looked for before the shared question; no Relink (Unlink, import
  again); two File > Import items instead of a checkbox dialog; the CLI conversion as a polled child beside the PSD;
  `opened` flag; "Update now" is an explicit answer; Esc on a link question = Decide later / Ignore; the f1w-based tray
  sticker in step 6; `*_path` scenarios counted with a PATH.
- **P9b**: the chooser case shows "Edit in..." without the list button; the chooser has "Other program..."; "Always use
  this one" starts ticked; own-write CRCs are used up when seen (fix of a real stuck-update case); the Photoshop pass
  splices Photoshop's own type layer into the linked file; Photoshop found through an injected stub registry; an unlinked
  import counts as "other" for smart objects; discovery when an "Edit in" control first draws.
- **P10a**: after a drop on the model the Transform tool takes over; a single picture dropped elsewhere becomes the Stamp
  tool's picture without a tray sticker (rule 8 of 0.4); a drop on a row never asks the shared question; a `.psd` on a row
  as layers; `Target::Screen`, `DragHover`, `DragLeave` steps through a real data object; any `CF_HDROP` accepted;
  "Dropped picture.png"; `DROPFILES` parsed in Rust; the drop worker as a thread; the texel check of a projected drop by its
  origin; winit's target cannot be put back (logged); the all-zero-alpha DIB rule moved here; `begin_edit_group` /
  `end_edit_group`. The P10a check adds: a drop uses the Stamp tool's own surface mode (Wrap when chosen), not "Project".
- **P10b**: `clip.rs`; a paste goes through the drop pipeline; Ctrl+Shift+V is not a paste; `WTE.Sticker.v1` only where it
  keeps something editable; pasted stamps bring their styles to the Stamp tool; Copy / Cut in the view take picture layers
  only, with the mask applied; encoding on a worker; Paste at View Centre in Split uses the 3D half; Paste as New Paint
  Layer moved to P12; text focus includes selected label text; no clipboard action while a dialog or modal job runs; Help >
  Clipboard contents... as a message; `Step::Chord(Paste)` after `ole::discard`.
- **P11**: the styled picture of a stamp is taken on the UI thread, encoded on the job (later moved fully to the worker by
  the P11 check); `DragEnded` in its own queue; WTE ignores every drag it runs itself; the PSD handle makes the link when
  the drag is recognised; the handle's editor order without a default; the Photoshop toast when Photoshop is known; a link
  file without WTE's latest changes handed out as it is (status line); sticker layer rows hand out masked pixels without
  styles; an empty paint layer is not dragged; a PNG nobody took is removed at once; Help line left to P13;
  `Step::ScreenHold`; `get_prefix` for drag enter.
- **P12**: `synced_pixels` is a pixel CRC, not a version; manifest `layer_links` as its own list; no "+" button (Paste as
  New Paint Layer in the New button's right-click menu and the Edit menu); the link block only for a linked active layer;
  a refused layer file also gives a warning toast; source rule details; no CLI conversion and no Fit for layer links; a
  locked layer refuses an update.
- **P13**: the text layer added with the Text tool's content; drop points from `flat_spots`; the linked paint layer is the
  first one the stroke changed; **"after undoing everything the document equals the start" is not asserted in
  `sticker_flow_path`** (it saves and reopens; each step asserts its own undo count) - accepted by the P13 checker; the
  second sticker stays projected; Help > Shortcuts lists the Sticker keys; `licenses\psd-tools-MIT.txt` not added (the
  fixtures are not shipped); "Adobe" only in the trademark sentence and test data.
- **S1**: GrabCut on a window of the box + margin (not the whole picture); ring fallback; early stop; transparent pixels
  fixed background; the soft-edge key from the nearest background pixel; a 4096-px limit; key Shift+W; turned layers use
  the bounding box; cancel between steps.
- **S2**: settings in `ui_state` (no format change); a live copy needs a saved project; what is copied (the sticker's own
  picture without stamp styles; paint layers at texture size); fixed file names; equal bytes not rewritten; files matched
  by a record, not by name; a new size from the file refits (G4); one undo step per batch; limits (top-level PNGs only, 256
  pictures, 64 MB, the links folder refused, one folder); polling, not `ReadDirectoryChangesW`;
  `sticker_picture_job`; folder chosen by an action in scenarios; the hand tests stay 1-25 (S2's are optional).

## 8. Open issues (carried from the package checks; none a blocker)

Decisions for the main session / the user:

1. **24-pt tablet hit targets** (plan 3.10): the Sticker tab, style editor, tray, eye / lock icons, transform and crop
   handles use egui's 20-pt rows (block arrows about 14 pt). One app-wide decision (P6a, P6b, P7a, P8).
2. **Clipping over a base that is not opaque, below 100 % or not Normal**: WTE's rule (ARCHITECTURE 6.10) differs from
   Photoshop and PhotoCraft, so such stickers look different there; the link is not broken by it (P5b).
3. **No total memory budget across all decoded PSD layers** (per layer and per canvas only) (P5a, P5c).
4. **The layer link block's height** at 1280x800 (G2, P12).
5. **Ctrl+V in a text field** when the clipboard holds a picture and no text pastes text copied earlier in WTE (plan 6.3's
   rule; P10b).
6. **Wider watched-folder pictures move on the model** (G4).

Work on the UI thread without a plan budget (smoothness, not correctness): Merge Down / Flatten / Trim (about 110 ms at
2048), the linked layer's PSD at 4096 (264 ms) and update (67 ms), Export Picture's encode, the blur popover preview of a
large styled sticker (170-255 ms), decoding a dropped / linked / watched picture (`apply_one`), a tray drag's picture clone,
the Preview bake of a wrapped sticker stamp on a large model (not measured; the flow run's sticker spans stay under 5 ms).

Smaller items: thumbnail textures of deleted stickers kept until the document changes; "Only this one" on a sticker stamp
is two undo steps; a tray sticker edited while several stamps show it changes all without the E6 question (documented);
own-write CRCs kept for the session (an editor save byte-equal to an earlier WTE write is not reported; Update now
applies it); no Relink for a broken external link; "Keep WTE's version" then Cancel on the send warning acts like Decide
later; `REG_EXPAND_SZ` registry values are refused (this machine's are `REG_SZ`); the PSD handle switches an existing
link's editor to the default without asking (plan 13.9); two WTE instances share the drag-out `manifest.json` without a
lock; a tiny cancel race can leave an unlisted PNG in the drag-out folder; a Text tool click between letters adds a new
text layer; GrabCut keys the soft edge on a transparent neighbour's colour, drops alpha-anti-aliased edge pixels below
128, and does not see Cancel inside one max-flow; Image Assets files in subfolders are not followed; the About > Licences
intro names the `licenses` folder that only the installer provides; the progress note of S1's GrabCut is caught by no shot;
the Cut Out Box has no hand-test number; the canvas zoom is kept on a window resize.

Not run by any agent (by rule): the installer build; any start of `photocraft.exe`, `Photoshop.exe` or the file dialogs
of "Other program..." / "Browse..."; the real Windows clipboard; a real drag between programs; a real tablet.

## 9. Hand tests (8.7, 13.6): owed

No hand test has been reported by the user yet: **all are owed**. **Bold** = needed before landing (8.6, 13.8 D7).

| # | Hand test | Status |
|---|---|---|
| **1** | PhotoCraft v0.2.0 round trip; "Edit all / Only this one" | owed |
| 2 | a layer effect in PhotoCraft -> the finished picture with the note | owed |
| 2b | a text and a mask added in PhotoCraft -> new layer, mask, WTE's layers kept | owed |
| **3** | Edit in PhotoCraft twice -> the "already open" question | owed |
| **4** | conflict question, both answers; Send while open -> warning, then WTE asks | owed |
| 5 | changed while closed; close without saving | owed |
| 6 | the latest PhotoCraft: 1, 2, 2b again, Maximize Compatibility off | owed |
| **7** | drop from Explorer, a browser, onto the Sticker canvas | owed |
| **8** | drag a thumbnail / layer row to the desktop, PhotoCraft v0.2.0, the latest PhotoCraft | owed |
| 9 | the PSD handle onto PhotoCraft, Ctrl+S | owed |
| 10 | after a drag out: nothing drags; files you saved in the drag-out folder stay | owed |
| 11 | the same drags with the pen | owed |
| **12** | Ctrl+C / Ctrl+V both ways with PhotoCraft; Paint, a browser | owed |
| 13 | text paste / copy with Notepad (Ctrl+V, Shift+Insert, Ctrl+C) | owed |
| 14 | Ctrl+S in PhotoCraft or a full-screen game: WTE never comes forward, flashes or beeps | owed |
| **15** | tablet companion in the Sticker view (S Pen pressure, erase, pan, pinch, style, Revert, Done) | owed |
| 16 | a v0.3.0 project opens and exports as in v0.3.0 | owed (automated equivalent passes: section 4) |
| 17 | Edit Layer in PhotoCraft on a paint layer | owed |
| 18 | Sticker-view keys V, W, H, F, X, Delete, Esc inside and outside the view | owed |
| **19** | Photoshop round trip (opens as a tab without a warning; text stays text) | owed |
| 20 | Photoshop: new / text / renamed / deleted / reordered layers | owed |
| **21** | Photoshop: layer style -> merged path; shape layer; smart object | owed |
| 22 | Photoshop: Maximize Compatibility Never / Always | owed |
| **23** | drag into Photoshop (document, tab bar), the PSD handle + Ctrl+S | owed |
| **24** | copy / paste both ways with Photoshop; Help > Clipboard contents... | owed |
| 25 | Photoshop: twice, conflict, Send while open, 16 bits, CMYK, Edit Layer in Photoshop, no focus taken | owed |

Optional (HOW_TO_TEST): S1's Cut Out Box on real photos; S2's live PNG copy with PhotoCraft's Place Linked and the watched
Image Assets folder.

### What no automated test proves (8.6 item 10)

- **The real PhotoCraft GUI save.** P0 showed (by reading the v0.2.0 source) that the CLI `convert` and the GUI Ctrl+S use
  the same exporter with the same options, so fixture f1 stands for a GUI save of an unedited document; what a real edit
  does between open and save is hand test 1.
- **Everything about the latest PhotoCraft** (0.23.x / HEAD) was read in its source at HEAD, never run: hand tests 6, 8.
- **A real drag between two programs** (into and out of WTE): hand tests 7-11, 23. Whether `DoDragDrop`'s modal loop and
  eframe's frames get along on the real window is also untested.
- **The real Windows clipboard**: hand tests 12, 13, 24 (every test and scenario uses the in-memory clipboard).
- **Photoshop itself.** Assumptions A1-A8 of 13.7 are unverified until the hand test named:

| # | Assumption | Settled by |
|---|---|---|
| A1 | Photoshop keeps the `lyid`s WTE wrote and numbers new layers above the seed (also: 255-byte Pascal names) | 19b, 20 |
| A2 | `Photoshop.exe <file>` opens the file (a tab when Photoshop runs) | 19a |
| A3 | opening an already open path brings that tab forward, no second copy | 25 |
| A4 | Photoshop does not reload a document changed on disk | 25 |
| A5 | Photoshop opens WTE's PSD without a warning | 19a |
| A6 | Ctrl+S saves in place as PSD, no Save As dialog | 19b |
| A7 | "Export Clipboard" is on, so Ctrl+C reaches the Windows clipboard | 24 |
| A8 | Photoshop does not lock the open PSD, so Send can rename over it | 25 |

Also unverified: the real App Paths discovery on another machine (`REG_EXPAND_SZ` values), the explorer `/select,` quoting,
pen behaviour with a real tablet and the S Pen through the companion, and WTE's clipping rule as seen in Photoshop.

## Critic

Completeness critic (plan 7 "Gate"), 2026-11-27, on `22d83bc` (the gate report). What I read: this report, plan sections
0-0.4, 3.3, 5.4, 6.2, 8.2, 8.4-8.6 and 13, the build and check notes where a claim led there, the scenario sources of
`sticker_flow_path`, `sticker_link` and `layer_link`, and the hand-test sheet. What I looked at: every
`sticker_flow_path` shot on both models (zoomed crops of the sticker in `target\sticker\critic\`), `sticker_styles`,
`sticker_link`'s Photoshop shots and `layer_link_ps_merged`.

**Verdict: PASS after fixes.** The user's request is built as the plan and its accepted defaults say; no red was found.
The fixes below are a test of the user's own Photoshop workflow, one hand-test step and four text corrections. After them:
`cargo test --workspace` 1,965 passed / 1 failed (WA20) / 71 ignored (`target\sticker\critic\test.log`), clippy 0 warnings.
The release exe was not rebuilt: only a test and three `expect` texts changed, so the gate's sweep outputs keep the old
texts.

### The user's workflow, end to end

`sticker_flow_path` walks the whole workflow on ABeautifulGame and DamagedHelmet, and its shots show what they claim: a
photo dropped on the model; Remove Background (the cut-out); a "GO!" text layer; Classic sticker (die-cut contour + drop
shadow); Wrap; Save As; "Edit in PhotoCraft" through the stub; the sticker moved and scaled in WTE (the badge stays
"linked" and the PSD is untouched); the editor's simulated save (`testkit::save_as_editor`, temp file + rename) updates
it in place as one "Update Linked Sticker", keeping the placement, the styles and the text; Copy / Paste Style; a linked
paint layer updated; save; reopen; export of one layer, a selection and a zip, each checked file by file.

**Gap: the user's own words name Photoshop, but the flow uses PhotoCraft.** The user: "I would be able to manuver the
sticker in the wrapped app, and I can edit the sticker itself in photoshop". `sticker_flow_path` runs with PhotoCraft as
the default editor. `sticker_link`'s Photoshop pass never moves the sticker between the link and Photoshop's save: its
"placement unchanged" means unchanged since it was placed. Hand test 19 did not ask for it either. The code path does not
depend on the editor, but nothing proved it for Photoshop. **Fixed:**

- New test `link::editor_tests::critic_photoshop_placement_edits_keep_the_link_and_each_save_updates_the_sticker_in_place`.
  It makes a Photoshop link, then in WTE moves, turns (0.4 rad), scales (x1.4), mirrors and wraps the sticker, gives it
  the Classic sticker styles and a tint. It asserts that the badge stays "linked", the PSD is byte-identical and nothing
  is started again. Then Photoshop's save adds Photoshop's own type layer (`type-layer.psd`'s bytes, spliced in): one
  step "Update Linked Sticker", two layers, placement, styles and tint exactly as the user left them, "linked", toast
  "Updated from Photoshop". The sticker is moved again and saved again: the newer placement is kept. An undo of the update
  does not move the sticker. It passes.
- `dist/HOW_TO_TEST.txt` hand test 19 gains step (c): move, scale and turn the sticker in WTE, add a style and switch
  Wrap on between two Photoshop saves; the badge stays "linked" and the update lands where the user put it.

### Fixed small items

| # | Item | Fix |
|---|---|---|
| C1 | G1: the three check notes with a raw carriage return (`check-P10b.md`, `check-P5c.md`, `check-P8.md`) | the byte replaced by the two characters `\r`; git now sees them as LF text |
| C2 | G3: `sticker_view`'s `sticker_small` expect says "all 14 sticker tools" | "all 15 sticker tools (S1's Cut Out Box included)" |
| C3 | `flow_updated`'s expect says the red bar lies "partly over the disc". On DamagedHelmet the link's scale is 2 (the PSD is 384 x 512 against 192 x 256 on the chess set), so the stub's fixed-pixel bar lands half size, beside the disc. The shot is right; the text was too narrow | "(partly over the disc, or just beside it when the link's scale is 2)" |
| C4 | the user's Photoshop workflow was not shown with placement edits (above) | test + hand test 19 (c) |

### Scenarios that prove less than they say (not fixed; minor)

1. **The drop shadow is hard to see in the flow's shots.** `flow_styled_wrapped` and later shots say "a soft dark
   shadow". On both models the sticker is small, and the shadow is faint even in a 3x crop; the white die-cut border is
   clear. The step asserts the shadow in state ("the stamp has the white contour and the drop shadow"). The shadow shows
   plainly in `sticker_styles` and `sticker_edit`.
2. **"Change colours" (plan 0.3) appears in no shot.** The adjustments (Brightness / Contrast, Hue / Saturation, Levels,
   Invert, Black & White, Posterize, Threshold; the Tool tab's Adjustments block) are covered by unit tests
   (`sticker::tests_p8`) and the control sweeps, not by a scenario picture.
3. **"After undoing everything the document equals the start" (plan 8.2, "in each")** is asserted in neither
   `sticker_flow_path` (reported as P13's deviation) nor `sticker_link`, `layer_link` or `sticker_dragout`. Those three
   assert the undo of each update instead, and `sticker_link` asserts that an undone update does not come back by itself.
   This report names only the flow.
4. **"Every 8.4 budget met" is slightly more than was measured.** The Preview restyle job's first serial run was 66.8 ms
   against 60 (G5; 8.4 lets the gate decide misses under 50 %, and it did). The 1.5 s "settled change -> updated" budget
   is for a 1024 x 1024 sticker but was measured only on 256-px stickers (1.17-1.23 s, mostly the watcher's settle time).
   So that budget is unverified at the size it names.

### Still owed or half there against the user's request (for the main session)

- **"Sticker editing just like PhotoCraft ... plenty to include".** The editor has 15 tools, cut-out commands, blur /
  sharpen, layout commands, text, shapes, clipping, masks, 7 adjustments, layer and stamp styles and a library. The parts
  of PhotoCraft left for later (plan 0.2, the list the user saw in section 11) are the visible distance from "just like":
  - layer groups;
  - Curves and colour balance;
  - filters other than blur and sharpen;
  - quick selection and refine edge;
  - satin and pattern overlays;
  - several strokes per layer and per-style blend modes;
  - perspective, warp and liquify;
  - healing and clone;
  - brush presets;
  - a history panel.

  If the user expects any of these now, it is new work.
- **"Drag and drop between the apps"** works in one direction only. Neither PhotoCraft nor Photoshop can drag into
  another program, so the substitutes are Ctrl+S on a link, Ctrl+C / Ctrl+V and a drag from Explorer (Help says so).
  The direction that is built, out of WTE (rows, thumbnails, the PSD handle), has never run against a real program:
  hand tests 8-11 and 23. Whether `DoDragDrop`'s modal loop and eframe's frames get along on the real window is
  untested.
- **"Seamlessly link photos / layers"** is built: sticker links, linked paint layers, the live PNG copy and the watched
  folder. Every statement about how Photoshop and the latest PhotoCraft behave rests on assumptions A1-A8 and on reading
  source code (section 9). Until hand tests 19-25 and 6 are reported, the link has been tested only against files WTE's
  tests wrote the way an editor would.
- **G2 matters for the user's main workflow.** In `layer_link_ps_merged`, the exact Photoshop case (a linked paint layer
  that needs a merged picture), no layer row is visible at 1280x800 until the list is scrolled or the split is dragged.
  The upper group (Tool tab) has about 180 pt of empty space above the split in that shot. This is a layout decision
  for the user (open issue 4).
- Hand tests 1, 3, 4, 7, 8, 12, 15, 19, 21, 23 and 24 are owed before landing (8.6). No tag, release or merge was made.
