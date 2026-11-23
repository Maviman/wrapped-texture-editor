# Stable gate report

Gate: Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable` = `stable-wrap`
(3125bd8: w1, w2, fix_wrap_1 and their checks) merged with `export-select` (e70b8ea: layer selection and zip export,
off round9-wip a8beea6) as merge commit a6fa80f. Nothing was merged into `main`. Raw outputs (git-ignored) are under
`target\stable\gate\`: `logs\`, `runs_st\` / `runs_r9\` (all scenarios), `compat\` (old projects), `flow\` (export
workflow), `sweep_st\` (wrap sweep), `sheets\` (shot crops), `timing\`, `fuzz\`, scripts in `tools\`, exes in `bin\`.

## Verdict in plain words

**The workflow works.** On the merged build a user can import a model (OBJ, glTF, GLB, ASCII FBX), paint on it, place
an image stamp, a text and a wrapped stamp, and then export one layer, a selection of layers, or everything as one .zip.
Every scenario that drives this ran green on 12 models and formats plus 4 old saved projects, and I checked the
written files myself (zip entries, per-layer PNGs, flattened texture).

**The build is stable, with one known red unit test.** 1,208 tests pass, 1 fails, 37 are ignored; the ignored GPU app
tests pass 17 / 17; clippy has 0 warnings; the release build is fine. The one failing test (WA20, a wrap quality limit
on a synthetic bumpy ball) was already red on the wrap branch and is unchanged. No scenario crashed; the only red
scenario runs are three wrap-quality asserts on two synthetic stress models and one deliberately broken model
(nonmanifold), all already known and all model-caused.

**Nothing that should stay the same changed.** Brush, project-mode stamps, text, default export output and old projects
are byte-identical or pixel-identical to round 9 (`wte_r9.exe`). The only screenshot differences are the new export
dialog controls (intended), file path text and toast timing.

## 1. Merge

`git checkout -b stable stable-wrap; git merge --no-ff export-select`. One conflict: `ARCHITECTURE.md` section 9 (test
lists), where both branches appended a block at the same spot. Both blocks were kept (stable wrap tests w1 / w2 /
fix_wrap_1, then the export-select app tests). All other files, including `scenarios.rs` (both sides add scenarios and
steps), `HANDOFF.md` and the app panels, merged without conflict. `NAMES` now has 42 scenarios (39 of round 9 +
`export_selected`, `export_zip`, `export_flow`). Every file changed since round9-wip is LF (`git ls-files --eol`: all
`w/lf`). No `Cargo.toml` / `Cargo.lock` change, `FORMAT_VERSION` = 3.

## 2. Tests, clippy, release

| Check | Result |
|---|---|
| `cargo test --workspace` (`test.log`) | wte_app 372 passed / 0 failed / 17 ignored; wte_core lib **789 passed / 1 failed** / 20 ignored |
| integration tests (`test_int.log`; cargo stops after the failing lib) | brush_tips 15, end_to_end 9, project_roundtrip 23, all green |
| **Total** | **1,208 passed, 1 failed, 37 ignored** |
| The failure | `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`: `cells 0.689751..1.986138` against the limit 0.70..1.40 (`wrap_tests.rs:2035`). Same numbers as on `stable-wrap` (check_wrap_2). Ink, dup, scatter and seams of WA20 are within their limits. |
| ignored GPU app tests `cargo test -p wte_app -- --ignored` (`test_gpu.log`) | **17 passed / 0 failed**, incl. `export_dialog_controls_and_export`, `export_dialog_zip_package_and_selected_layers`, `export_dialog_at_the_smallest_window_and_with_several_sets` |
| `cargo clippy --workspace --all-targets` | **0 warnings** |
| `cargo build --release -p wte_app` | ok; `target\stable\gate\bin\wte_stable.exe`, **SHA-256 9EB3F988C9A058E440687A0A145FB624E7DFF31D6F60F69AD8A0BCB40FD41121** |

The counts add up: stable-wrap had 1,186 passed (wte_core lib 775) and export-select added 14 core and 8 app tests.
The four other round-9 wrap failures the export branch still showed (WA4, WA17, WA24, WA35) are green here because the
wrap branch fixed them.

## 3. All scenarios, identity with round 9, old projects

**All 42 scenarios ok** with `wte_stable.exe` (`logs\scen_st.txt`; models as in the round-9 gate: Avocado for
`open_path` / `fuzz_path` / `stamp_edge_path` / `text_preview_path`, DamagedHelmet for `workflow_path`, tiny_cube for
`import_save`, ABeautifulGame for `brush_types_path`). The 38 non-new, non-wrap scenarios also ran with a copy of
`wte_r9.exe` (all ok) and were compared file by file (`logs\shot_cmp.txt`):

- **35 scenarios: every file identical**, `shot.png` included: every brush scenario, stamp, text (all 8), clay,
  preview, multi, roundtrip, undo, the imports, layers, open_path, fuzz_path, stamp_edge_path, pan2d.
- **`export_run`: the 7 exported files byte-identical** to wte_r9 (sphere.png, .ora, .psd, 4 per-layer PNGs), and its
  shot identical. **`workflow_path`** (DamagedHelmet): all 8 export files and the shot identical.
- `export` shot.png: 23,593 px (x 198-597, y 153-529) = the Export dialog's new "Layers: Visible / All / Selected" row
  and "Pack into one .zip file" checkbox, which push the rest of the dialog down (`logs\export_dialog_r9_vs_st.png`).
  **Intended change** (export-select).
- `brush_shape` `shot_dialog.png`: 11,344 px = the dialog's file path text (`runs_r9` vs `runs_st`) and a toast's fade
  (`logs\brush_shape_dialog_r9_vs_st.png`). Known kind.
- `final_bake_async` shot.png: 9,441 px in the toast box (x 686-975, y 660-700). Known timing kind; its exports identical.

**Old projects** (`compat\`): made fresh with the reference exes: `wb_r67.wtep` / `wb_r8.wtep` (`workflow_path` on
WaterBottle with `WTE_STAMP_MODE=wrap`, so they hold wrapped stamps) and `sphere_r67.wtep` / `sphere_r8.wtep`
(`wrap_surface`'s saved project, two wrapped stamps and text). Each opened with `open_path` in stable, r9 and the exe
that made it: 12 / 12 ok, **stable's shot identical (0 px) to both** for all four; no `wrap:` line in stable's logs (the
saved pixels are kept). `workflow_path` on `wb_r67` and `wb_r8`: ok in stable and r9, the 12 exported files of each
**byte-identical** to r9's, shots identical. `export_flow` on all four old projects: ok (section 4).

## 4. The user's workflow end to end (`export_flow`)

`export_flow` imports the model, accepts the import dialog, paints a stroke, places an image stamp, a text and a
wrapped stamp, exports the wrapped stamp's layer alone ("Export this layer"), selects the stamp and Ctrl-selects the
text and exports the selection, then exports everything as one zip; it checks names, listings, pixels and the
recomposition itself.

| Model | Result | Zip |
|---|---|---|
| generated cube: OBJ+MTL+PNG, glTF, GLB, ASCII FBX (no PATH) | ok | 138 KB (OBJ, 512 px) to 476 KB (FBX, 1024 px), 8 entries each |
| WaterBottle.glb | ok | 13.2 MB, 8 entries |
| Avocado.glb | ok | 21.1 MB |
| DamagedHelmet.glb | ok | 43.2 MB |
| BoxTextured.glb | ok | 0.2 MB |
| Lantern.glb | ok | 27.7 MB |
| ngons_tiling.obj (3 texture sets + 1 without a map, a set name with a space) | ok | 1.2 MB, 25 entries |
| klein_bottle.obj | ok | 0.15 MB |
| old projects sphere_r67, sphere_r8, wb_r67, wb_r8 | ok | 11-12 entries |

My own check (`tools\spot_zip.py`, `logs\spot_zip.txt`) on all 15 zips: `testzip` clean; per texture set the per-layer
PNGs composited bottom-up equal the flattened PNG within **at most 1.22 / 255** (18 sets); every "one layer" and
"selection" file exists in the zip under the same name with **identical bytes** (45 / 45). I looked at the DamagedHelmet
zip's six PNGs side by side (`logs\zip_sheet_DamagedHelmet.png`): flattened = base + red stroke + the projected logo
(scattered over the UV islands it covers) + the text + the wrapped logo (one compact piece on the cheek); each per-layer
PNG holds only its own layer on transparency. The final screenshot shows the toasts "Exported 1 files to
shot_path_one", "Exported 2 files to shot_path_sel", "Exported DamagedHelmet.zip (8 files) to shot_path_all" and the
footer "2 layers selected · tools paint on Logo".

Observed, **model-caused**: on ngons_tiling the stamp layers of two sets are inked on every texel (1,048,576 of
1,048,576) because the model's UVs run 0..4 (the texture tiles 4 x 4), so a stamp in UV space repeats over the whole
texture. The app already says so in the panel ("This model uses the same texture area in several places").

Minor wording seen in the toast: "Exported 1 files" (plural with one file). Cosmetic, not fixed.

## 5. Wrap sweep (wrap_surface, 13 models + noisy400, tiles300, parts200, trim_strip)

Method as in `docs/notes/round9/gate.md` section 4 (one process at a time, `WTE_FRAME_LOG=1`, 1280 x 800), parsed
with fix_wrap_1's parser (`logs\table_st.txt`). The r9 comparison uses check_wrap_2's sweep made with a hash-checked
copy of `wte_r9.exe` (4F9E1422...; `logs\table_r9.txt`, re-parsed and equal to check_wrap_2's table): the exe is the same
and the runs are deterministic, so I did not run it again.

**The merged build's wrap is exactly the wrap branch's:** the stable table equals check_wrap_2's `table_c2.txt` field
for field (times aside), and 118 of 119 shots are pixel-identical to check_wrap_2's (noisy400 `shot_off.png` differs
only in the toast box). Exit codes: 14 ok; red on nonmanifold, noisy400, tiles300 (r9: red on Lantern, ABeautifulGame,
many_islands, noisy400, tiles300, parts200).

Jump small gaps on (step 10; `shown` = share of the picture shown, `beyond` = runs past the end of a tube):

| Model | ink | dup % | scatter % | covered | shown | lost_cut % | seams | rejected % | flipped | r9 ink / dup |
|---|---|---|---|---|---|---|---|---|---|---|
| WaterBottle | 1.0020 | 0 | 0 | 1.0000 | 1.0 | 0 | 0 | 0 | 0 | 1.0033 / 0 |
| Avocado | 0.9963 | 0.671 | 0.622 | 1.0000 | 1.0 | 0 | 0 | 34.81 | 0 | 0.9976 / 0.787 |
| DamagedHelmet | 0.8286 | 0.125 | 0 | 1.0000 | 0.974 | 1.96 | 70 | 17.95 | 9 | 0.8424 / 0.036 |
| Lantern | 0.7502 | 1.572 | 1.796 | 0.9029 | 0.731 | 12.11 | 30 | 18.37 | 1 | 0.7704 / 1.676 |
| ABeautifulGame | 0.9055 | 1.886 | 1.151 | 1.0000 | 0.913 | 0.12 | 2,147 | 50.33 | 70,583 | 0.8463 / 2.674 |
| ngons_tiling | 0.0061 | 0 | 0 | 0.0736 | 0.073 | 0 | 0 | 0 | 0 | 0.0061 / 0 |
| torus_knot | 0.4492 | 0 | 0 | 0.4441 | 0.442 (beyond 0.558) | 0 | 48 | 0.03 | 0 | 0.4545 / 0 |
| mobius | 0.5700 | 0 | 0 | 0.5702 | 0.568 | 0 | 0 | 0 | 0 | 0.5702 / 0 |
| klein_bottle | 0.9824 | 0 | 0 | 1.0000 | 1.0 | 0 | 0 | 0 | 0 | 1.0132 / 0 |
| nonmanifold | 0.1881 | **3.414** | 1.996 | 0.6303 | 0.178 | 0 | 0 | 70.27 | 0 | 0.1873 / 1.523 |
| many_islands | 0.0317 | 0 | 0 | 0.0338 | 0.031 | 0.50 | 15 | 6.42 | 7 | 0.0313 / 0 |
| flat_annulus | 0.5972 | 0 | 0 | 0.5995 | 0.598 | 0 | 0 | 0 | 0 | 0.5972 / 0 |
| high_poly_sphere | 1.0643 | 0 | 0 | 1.0000 | 1.0 | 0 | 0 | 0 | 0 | 1.0977 / 0 |
| noisy400 | 0.4720 | **5.124** | 2.847 | 0.8620 | 0.473 (beyond 0.502) | 2.54 | 2,623 | 44.98 | 21 | 0.3036 / 9.971 |
| tiles300 | **1.3286** | 2.052 | 0.024 | 0.9372 | 0.832 | 0 | 0 | 5.46 | 0 | 1.3286 / 2.052 |
| parts200 | 0.9541 | 0 | 0 | 0.7312 | 0.723 | 0 | 0 | 0.37 | 0 | 0.9541 / 0 |
| trim_strip | 0.3188 | 0 | 0 | 0.3188 | 0.316 | 0 | 0 | 0 | 0 | 0.3188 / 0 |

Jump off differs from Jump on only where there is a second part: Avocado (0.5393, the seed is separate), DamagedHelmet
(0.5080), ABeautifulGame (0.0639: the picture is larger than the pawn it lands on, ends meet), many_islands (0.0123),
tiles300 / parts200 (one tile only).

Panel lines (Jump on), checked against the numbers:
- WaterBottle: none (nothing to explain). klein_bottle, high_poly_sphere: "Fitted to the curved surface: stretched up
  to 27 %".
- Avocado: stretched 14 %; "35 % of the laid picture fell where the surface overlaps another part or folds over itself:
  each spot of the picture is shown in one place only"; "Continued onto 1 separate part".
- DamagedHelmet: stretched 50 %; cut; "3 % of the picture runs off the surface: an edge, a hole or the end of the part";
  overlap 18 %; 9 parts; shared-texel line shown (100 %).
- Lantern: stretched 21 %; cut; "12 % of the picture is lost where it is cut open round the closed part's corners";
  overlap 18 %; "The part is narrower than the picture: only 73 % of the picture lies on it".
- ABeautifulGame: stretched 92 %; cut; "9 % ... runs off"; overlap 50 %; 30 parts; shared-texel line (10.7 %).
- torus_knot: "Longer than the way round: 43 % shown, the ends meet at the back".
- mobius 43 %, flat_annulus 40 %, ngons_tiling 93 %, trim_strip 68 %, parts200 28 %: "... runs off the surface".
- many_islands: stretched 90 %; cut; overlap 6 %; "narrower than the picture: only 3 %"; 2 parts.
- nonmanifold, noisy400, tiles300 stop at the quality assert before the panel step is logged. noisy400's panel on the
  final screenshot reads stretched 34 %, cut, "3 % of the picture is lost where the surface had to be cut", "45 % of the
  laid picture fell where the surface overlaps ...".

### Shots read at 3-8 x (`sheets\pair_*.png` = viewport stable | r9; `sheets\z_<model>.png` = zoom, stable first)

| Model | What the user sees | As expected? | Cause if not |
|---|---|---|---|
| WaterBottle | one whole 8 x 4 picture on the label band, rows level, columns narrowing evenly to the sides | yes | |
| Avocado | the whole picture: the middle columns over the seed's dome, the outer ones on the flesh, rows continuous across the seed's rim; column 1 is narrow on the flesh left of the seed | yes (better than r9, where column 1 is bare below its top cell) | |
| DamagedHelmet | whole picture across the visor; the two left columns kink along the visor/shell crease, the right columns break over the raised side plates; teal bits left of the frame are the picture shown again through mirrored UVs | mostly; the kink and the plates follow the surface | model: hard creases and separate plates; mirrored UVs (panel line shown) |
| Lantern | the picture wraps round the plinth block, cells continue over its corners; bits on the plinth's base slab and cap; two thin bare slivers on the front faces | mostly | model/topology: a coarse closed block (100 triangles); the panel names the 12 % cut loss and 73 % room |
| ABeautifulGame | the picture lies flat on the board, rows parallel to the board edge, all 8 columns and 4 rows, the pawns in it carry their share; the left column is cut where the slab meets the flush marble frame; colour bits on other pieces | mostly | the left cut is **code** (minor, check_wrap_2: Jump small gaps does not continue onto the flush frame, 1-2 mm away); the bits on other pieces are model (shared UVs, panel line shown) |
| ngons_tiling | hardly any picture: it lands on one small n-gon tile; the rest of the frame is gaps and other tiles | yes for this model | model: separate tiles with gaps larger than "small", UVs tiled 4 x 4 (panel: 93 % runs off, shared texels) |
| torus_knot | a strip of the picture round the tube, once, ends meeting at the back | yes | the tube is thinner than the picture (panel: 43 % shown) |
| mobius | columns follow the band; the cells outside the band are cut by its edge | yes | the band is narrower than the picture (43 % runs off) |
| klein_bottle | one whole picture on the outer tube, columns 5-8 curving into the hole | yes | |
| nonmanifold | the front page of the book carries cells over about two thirds of its width, the rest striped; the duplicated triangle shows a stripe; the double cone shows its own texture | acceptable | model: seven pages through one edge and a duplicated triangle; dup 3.414 % > 3 % is the step-10 red |
| many_islands | the picture on two tiny cubes (purple and green), nothing else | yes for this model | model: cubes far smaller than the picture and far apart (panel: only 3 %) |
| flat_annulus | the picture on the ring's top, the top row cut by the outer edge | yes | placement past the edge (40 % runs off) |
| high_poly_sphere | one whole picture following the bumps, no repeat | yes | |
| noisy400 | a shattered star of strips with fragments beyond the frame | no | model: synthetic random height field (features smaller than a cell); not a crash, the panel names cut and overlap; r9 is as bad (dup 9.97 %) |
| tiles300 | one whole picture over 300 x 300 separate tiles, rows continuous | yes visually | model: synthetic; ink 1.3286 > 1.3 because the picture is counted on tile sides; red, unchanged from r9 |
| parts200 | one whole picture over 200 x 200 separate quads | yes | panel says "28 % runs off" (the gaps between the quads); literally true, looks whole at screen size |
| trim_strip | the picture on the square part, the strip beside it bare | yes | model: a tiled trim strip (68 % runs off) |

Model-caused or synthetic: nonmanifold, noisy400, tiles300 (the three reds), ngons_tiling, many_islands, trim_strip,
mobius / flat_annulus (placement). Code-caused, minor and not a regression: the ABeautifulGame flush-frame cut.

## 6. Timing (idle machine)

Before each run the driver logged the CPU load and any cargo / rustc / wte / game process (`timing\driver.log`): no
other build or app copy ran, no game ran (Steam was open, idle), **CPU load 0-15 % before every run (median 4 %)**. CPU
i7-9700K (8 cores). Two runs each, exes alternating (three for exports).

| Line | stable | r9 | r8 | stable vs r9 |
|---|---|---|---|---|
| high_poly_sphere Final, Jump off: param / applied | 3,038 / 3,403; 3,069 / 3,439 ms | 2,996 / 3,372; 2,902 / 3,280 | 7,700 / 7,775; 7,699 / 7,772 | +3 % |
| high_poly_sphere, Jump on | 3,075 / 3,146; 3,041 / 3,111 | 2,944 / 3,018; 2,915 / 2,998 | 7,924 / 7,998; 7,886 / 7,959 | +4 % |
| ABeautifulGame Final, Jump off | 512 / 1,095; 512 / 1,107 | 408 / 1,003; 413 / 1,012 | 2,230 / 2,320; 2,222 / 2,319 | param +25 %, applied +9 % |
| ABeautifulGame Final, Jump on | 3,363 / 4,096; 3,353 / 4,048 | 4,273 / 4,958; 4,257 / 4,944 | 10,146 / 10,814; 10,116 / 10,776 | **-18 %** |
| tiles300 Jump on (21,860 parts) | 1,784 / 1,825; 1,766 / 1,818 | 5,407 / 5,445; 5,413 / 5,462 | 2,083 / 2,138; 2,074 / 2,130 | **-67 %** |
| ABeautifulGame `workflow_path` (Wrap): text placed | 67; 65 ms | 65; 65 | 234; 222 | same |
| text move | 187; 187 | 170; 169 | 482; 484 | +10 % |
| text scale | 282; 282 | 232; 233 | 798; 779 | +21 % |
| text rotate | 218; 220 | 201; 204 | 856; 858 | +8 % |
| image stamp placed | 844; 824 | 878; 885 | 1,934; 1,938 | -5 % |
| drag frame max (move / scale / rotate) | 16.3-19.1 ms | 15.9-20.0 | 16.0-20.2 | same |
| export of the 4-layer sphere document, plain (first export of the session) | 0.39 / 0.45 / 0.42 s | 0.39 / 0.41 / 0.45 | 0.40 / 0.58 / 0.48 | same |
| same document as one zip (stable only; 2nd and 3rd export in the session) | 0.10 / 0.09 / 0.10 s (and 0.09-0.10 again) | - | - | |
| DamagedHelmet 2,048 px: one layer / two layers / everything as zip | 0.06 / 0.13 / 1.96 s | - | - | |

The zip export is not slower than the plain one; it is faster in these runs only because it was the second export of the
session (the first one pays a warm-up). In `workflow_path` the stable runs have one extra frame over 300 ms (2.55 s):
it is the scenario's own new export verification (`verify_export` re-reads and recomposites the exported files inside an
Assert step on the UI thread), not app behaviour. Wrap finals: 0 frames over 300 ms in every exe.

The slowdowns against r9 on small and medium wraps (ABG Jump off param, the text Finals) are w2's owner-visibility
rays, already reported by check_wrap_1 / check_wrap_2; they add 20-100 ms and every one stays well below r8. The heavy cases (ABG Jump on, tiles300) are
faster than r9.

## 7. Fuzz (Wrap mode, 1,000 steps)

`fuzz_path` with `WTE_FUZZ_STEPS=1000`, `WTE_STAMP_MODE=wrap` (the log confirms "Wrap (WTE_STAMP_MODE=wrap)"),
`WTE_FRAME_LOG=1` (`fuzz\<model>_<seed>\`):

| Run | Result | Errors | Frames (> 500 ms / > 300 ms) | Wrapped bakes (`wrap:` lines / Finals applied) | Undo all / redo all | Time |
|---|---|---|---|---|---|---|
| ngons_tiling, seed 7 | ok | **0** | 10,965 (0 / 0) | 40 / 73 | 99 / 100 | 171 s |
| WaterBottle, seed 7 | ok | **0** | 10,463 (0 / 0) | 47 / 67 | 100 / 100 | 160 s |
| ngons_tiling, seed 11 | ok | **0** | 11,683 (0 / 0) | 73 / 65 | 99 / 99 | 180 s |
| WaterBottle, seed 11 | ok | **0** | 10,712 (0 / 0) | 42 / 52 | 100 / 100 | 163 s |

0 ERROR lines and 0 panics in all four. The WARN lines are the app's own guard toasts ("Layer is locked", "Cannot
merge: the layer below is not a paint layer", "Paint skipped on ...: the active layer is locked or hidden", "No UV island
here") and the crash module's note about screen-capture overlay DLLs loaded into the process (OBS / Overwolf / NVIDIA),
which are on this machine. Each run ends with "the document survived the fuzz run".

## 8. What does not work, and why

| # | What | Severity | Cause |
|---|---|---|---|
| 1 | Unit test WA20 fails on cells 0.690..1.986 vs 0.70..1.40 (a synthetic bumpy ball; ink, dup, scatter, seams pass) | minor | code: the area term of the relaxation on the steepest bumps; two attempts in w2 did not move it |
| 2 | wrap_surface step 10 red on nonmanifold: dup 3.414 % > 3 % (r9 1.523 %) | minor | model: a seven-page book on one edge plus a duplicated triangle; side effect of w2's owner-visibility rule |
| 3 | wrap_surface step 10 red on noisy400: dup 5.124 % (r9 9.971 %); the picture is a shattered star | minor | model: synthetic random field |
| 4 | wrap_surface step 10 red on tiles300: ink 1.3286 > 1.3 (unchanged from r9) | minor | model: synthetic tiles |
| 5 | ABeautifulGame Jump on: the left column is cut where the board slab meets the flush marble frame; the panel says "9 % runs off" | minor | code: Jump small gaps does not continue onto a coplanar touching part; not a regression (r9 showed less) |
| 6 | parts200 panel "28 % runs off" while the picture looks whole (gaps between 200 x 200 quads) | cosmetic | literally true; synthetic model |
| 7 | toast "Exported 1 files" | cosmetic | wording |
| 8 | Small wraps slower than r9 (ABG Jump off param 512 vs 410 ms, text scale 282 vs 233 ms) | minor | w2's owner visibility rays; heavy wraps are faster |

Nothing of this crashes, writes garbage files or shows a wrong panel line. No assert or limit was weakened in this gate
(I changed no product code: the only edit is the conflict resolution in `ARCHITECTURE.md`).

## Fix pass 2

Gate: Opus, 2026-11-23. Branch `stable` at **f889d69**: `stable-fix-wrap` (251ebcb + check b2c5a14, merge a1346a2),
`stable-fix-export` (867d920 + check c077314, merge 5c5f2ef) and `stable-bg` (1728708, merge f889d69). I did not merge
again and changed no Rust code. Raw outputs (git-ignored) are under `target\stable\gate3\`: `logs\`, `runs\` (all
scenarios), `flow\`, `fuzz\`, `sweep\`, `focus\`, scripts in `tools\`, exes in `bin\`. Notes:
`docs/notes/stable/gate_fix2.md`. `target\stable\gate2\` is a stopped earlier attempt built before f889d69: stale, not
used.

### Verdict in plain words

**The workflow works and the build is stable.** Every scenario ran green on the f889d69 exe. That covers importing a
model, painting, image stamps, text and wrapped stamps, and exporting one layer, a selection, or everything as one .zip
(or as loose files). My own Python checks found every exported file correct. The only red test is the known WA20.

**A run of the app no longer takes the focus.** While I ran the gate, the user was in a fullscreen game. A probe run
sampled the foreground window every 25 ms: the game kept it for the whole run, and the app was never in front (see
below). One caveat: an earlier, stopped attempt of this step had already started a batch of runs with an **older exe
that still takes the focus**. That batch kept going until 09:27 (open item 1).

### Build and tests (f889d69)

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` (`test.log`, again at the end `test_final.log`) | wte_app 377 / 0 / 19 ignored; wte_core lib **801 / 1 failed** / 24 ignored; brush_tips 15, end_to_end 9, project_roundtrip 23. **Total 1,225 passed, 1 failed, 43 ignored**, the same both times. |
| The failure | `wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`: `cells 0.689751..1.986138`. These are the same numbers as before (known red). |
| Serial perf tests `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1` (`perf.log`) | 2 / 2 pass. WA7 slowest 132.9 ms, check:A 100.1 ms, against 200 ms. |
| GPU app tests `cargo test -p wte_app -- --ignored` | First run (`test_gpu.log`): 18 / 19. `text_ops::tests::sample_job_for_the_machine_fonts` took 649 ms against 0.5 s while a game and a second app copy were running. Alone it passed 3 of 3 (340, 360, 424 ms; `test_gpu_fonts_alone.log`). The full rerun (`test_gpu2.log`): **19 / 19**. |
| `cargo clippy --workspace --all-targets` | **0 warnings** (both runs) |
| `cargo build --release -p wte_app` | OK. `target\stable\gate3\bin\wte_stable2.exe`, **SHA-256 21CA9303AFD2C89541D1271F247D5127B2D348185A75AEDD977993BB56DE4B98**. The final rebuild gives the same hash. |

`Cargo.toml` and `Cargo.lock` are unchanged. No test limit was changed.

### Focus (the user's request)

`tools/stable/focus_probe.ps1` starts one `--screenshot` run and samples `GetForegroundWindow` every 25 ms. It only
reads; it sends no input. I ran it on a `smoke` run while the user's game (BlackOps3) was in front: 50 samples, the
app in front 0 times, the foreground changed 0 times, and the game was still in front afterwards (`focus_probe.txt`).
Every scenario, flow, fuzz and sweep run of this pass used a copy of the 21CA9303 exe.

### All scenarios

All 46 runs were `"ok": true` (`logs\scen.txt`). That is the 45 names in `NAMES`, with `export_unsettled` run twice
(sphere and ABeautifulGame). I ran no old exe, so I compared against stored outputs instead (`cmp_*.txt`):

- **Against the stable gate's `runs_st`:** 33 scenarios are identical file for file. Four runs are new since the gate
  (export_errors, export_nouv, and export_unsettled twice). The other 9 (brush_shape, export, export_flow,
  export_selected, export_zip, fuzz_path, multi, text_fonts, workflow_path) differ for reasons the fix branches
  intended:
  - export_flow, multi and fuzz_path now write more files (loose export, selection zip, both sets' layers of a design,
    fuzz exports);
  - the export dialog changed;
  - toast timing;
  - the text_fonts focus outline, from stable-bg.
- **Against check_exp2's runs (exe 9c953770):** default, smoke, export_errors, export_nouv, export_run,
  export_selected, export_zip, layers, multi, roundtrip and undo are all identical. export_flow on the cubes is
  identical in all 59 files. `export` differs only in the export folder's path text. `final_bake_async` differs only
  in the toast box.
- **Against stable-bg's runs:** text_fonts, workflow_path, brush_types_path and wrap_surface are identical.
  brush_shape differs only in a toast's fade. fuzz_path differs because the fuzz now has more kinds of action.

### Export (`export_flow`)

| Model | Result | Zip |
|---|---|---|
| generated cube: OBJ+MTL+PNG, glTF, GLB, ASCII FBX | ok | 8 entries each, selection zip 2 entries |
| WaterBottle.glb | ok | 8 entries |
| DamagedHelmet.glb | ok | 8 entries |
| ABeautifulGame.glb (15 sets) | ok, 49 s | 84 entries, 227.0 MB; selection zip 8 entries, 1.2 MB |
| cube without UVs (`models\nouv_cube.obj`, 12 triangles without UVs, generated layout) | ok | 8 entries, 336 KB |

- `tools/stable/verify_zip_vs_loose.py`: **ALL OK**. Each full zip equals the loose files byte for byte, each selection
  zip uses the full export's names and bytes, and no `.tmp-` file was left.
- `tools/stable/check_exp2_verify.py`: **ALL OK**. Each set's flattened PNG matches a re-composite of its per-layer
  PNGs within **1.33 / 255** at worst (ABeautifulGame); the one-layer and selection files equal their zip entries; the
  ORA and PSD files check out.
- **ABeautifulGame is identical in all 96 files** to check_exp2's run. On DamagedHelmet and ABeautifulGame, every file
  that also exists in check_wrap2's export_flow (exe 97A70242) is byte-identical to it.
- The toasts now read "Exported 1 file" and "Exported 2 files".
- `export_unsettled` on ABeautifulGame: the Final landed after 3,072 ms. The longest frame was 21 ms. Both exports
  were byte-identical and hold the Final's pixels.

### Fuzz (Wrap mode, 1,000 steps, `WTE_FRAME_LOG=1`)

| Run | Result | Errors / panics | Frames (interval > 500 / > 300 ms) | export / save+reopen / Ctrl- or Shift-click / selection Dup-Del | Undo all / redo all |
|---|---|---|---|---|---|
| ngons_tiling, seed 7 | ok | 0 / 0 | 13,229 (0 / 0) | 15 / 8 / 16 / 10 | 80 / 81 |
| WaterBottle, seed 7 | ok | 0 / 0 | 12,534 (0 / 0) | 15 / 7 / 15 / 9 | 60 / 62 |
| ngons_tiling, seed 11 | ok | 0 / 0 | 14,465 (1 / 2) | 12 / 14 / 12 / 3 | 3 / 3 |
| WaterBottle, seed 11 | ok | 0 / 0 | 13,605 (0 / 0) | 14 / 16 / 10 / 5 | 8 / 8 |

- The fuzz now exercises the new features: export, save and reopen, multi-select and selection Duplicate / Delete.
  Each run asserts that no temp file was left and ends with "the document survived the fuzz run".
- The undo counts are lower than in the gate because a save and reopen starts a new history.
- The two slow frames on ngons seed 11 are the scenario's own window resizes (542 ms and 399 ms between frames). The
  UI's CPU time was 2.8 ms and 124 ms. These are not app work.

### Wrap sweep (wrap_surface; ABeautifulGame, parts200, high_poly_sphere, torus_knot)

All four runs were ok.

- **The step-10 table equals fix-wrap's final sweep (`fixwrap\sweep_fw7`, exe wte_fw7) field for field, times
  aside.** That includes `gaps`, `at_part` and the panel lines. 27 of the 28 shots are pixel-identical. parts200's
  `shot.png` differs only in the toast box.
- The panel lines:
  - ABeautifulGame Jump on: "9 % of the picture stops where it meets a neighbouring part", stretched 92 %, cut, 50 %
    overlap, 30 parts, and the shared-texel line (10.7 %).
  - parts200: "28 % of the picture falls into the gaps between separate parts".
  - torus_knot: "Longer than the way round: 43 % shown, the ends meet at the back".
  - high_poly_sphere: stretched 27 %.

  Each line matches its numbers (ABeautifulGame `at_part` 8.60 %, parts200 `gaps` 27.69 %, torus_knot `ends_share`
  43.0 %).
- **Not a speed result.** The user was playing a game during the sweep, with CPU load at 65-76 %. The param times
  were 1.3-3.7 x the idle gate's: ABeautifulGame Jump off 1,904 against 517 ms, Jump on 6,912 against 3,411 ms;
  high_poly_sphere 6,911 against 3,050 ms. I could not separate the game's load from any effect of the window now
  running in the background (Windows may throttle a process with no window in front). Timing is therefore unmeasured
  in this pass.

### Open items

1. **Old exe runs during this step.** The stopped earlier attempt of this step left its driver script running. From
   09:15 to 09:27 it ran `target\stable\gate2\bin\wte_g2_flow.exe` (SHA-256 1B0C7255…, built before f889d69). That
   covered export_flow ABeautifulGame and nouv_cube, the four fuzz runs and the four sweeps, while the user was in a
   game. That exe can take the focus. I tried to stop the script, but the permission system refused, so it ran until
   it finished on its own. Nothing of this pass used its outputs.
2. **WA20** is red: cells 0.690..1.986 against 0.70..1.40.
3. **The wrap_surface step-10 reds on nonmanifold, noisy400 and tiles300** (model-caused) were not re-run in this pass.
   Fix-wrap's sweep shows them unchanged, now with panel lines.
4. **DamagedHelmet Jump on reads "Only 97 % of the picture lies on the surface" and names no cause** (check_wrap2 m1).
   Cuts take 1.96 %, just under the 2 % threshold for their own line.
5. **ABeautifulGame flush-frame cut.** The picture still stops at the marble frame. The panel now names it correctly.
   fix-wrap's attempt 2 was reverted because it added 5.7 % seams. The flush-plates test stays `#[ignore]` as a known
   limit.
6. **The export dialog's "+ their layers in N other texture sets" note** (`dialogs/export.rs::linked_extra`) counts
   layers, not sets, when two selected designs span the same sets. The file list is right, and the note has no unit
   test.
7. **Test strength.** export_flow takes its expected linked layers from `export::plan_contents`, which is the code
   under test. The `multi` scenario, the core test and the Python checks are the independent ones.
8. With several layers selected, the footer's Rasterize acts on the active layer without saying so (nit).
9. **Save right after a wrapped stamp** still waits on the UI thread for the Final. Export no longer does.
10. **Wall-clock tests depend on load.** `sample_job_for_the_machine_fonts` (0.5 s) failed once under load. WA7 and
    check:A now run only as serial perf tests.
11. **Speed** was not measured on an idle machine in this pass (see the sweep), and I compared it against no old exe.

### User decisions

1. Land `stable` on `main` with WA20 as a known red, or try a third approach.
2. Should the DamagedHelmet line ("Only 97 % ...") name its largest cause even below 2 %?
3. ABeautifulGame flush frame: keep the cut, which the panel now explains, or take fix-wrap's attempt 2 at +5.7 %
   seams?
4. Keep the three model-caused wrap_surface reds as known reds?
5. Should Save, like Export, wait for a pending Final off the UI thread?
6. Keep wall-clock tests in the default run (the font sample job), or move them to serial perf tests like WA7?
7. A real binary FBX from Blender as a test asset (critic H1, still open).
8. Accept the wrap slowdown against r9 on small and medium wraps (section 6). It was not re-measured here.
