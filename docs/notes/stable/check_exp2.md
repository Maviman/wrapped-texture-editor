# check_exp2: independent check of `stable-fix-export`

Checker: Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_exp`, branch `stable-fix-export` at
`867d920` (4 commits on `stable` @ `b064fee`). Inputs: `critic.md`, `STABLE_GATE_REPORT.md`, `check_exp_2.md`,
`fix_export.md`, `git diff stable..HEAD` (20 files, +2,466 / -101). Evidence is under `target\stable\check_exp3\`
(git-ignored): `test_workspace.log`, `test_timing_alone.log`, `test_gpu.log`, `test_4096.log`, `clippy.log`,
`build.log`, `runs\summary.txt` and one folder per run, `verify_*.json`, `bin\` (private exe copies), `assets\`.

Goal: try to refute each "done" claim of the builder. **Verdict: no claim refuted; no major defect.** Three minor
defects (one fixed by me, a doc glitch) and two notes on test strength. The machine was shared with other agents'
cargo builds during this check (tasklist showed several `rustc` / `cargo-clippy` processes), which explains the
timing reds below.

## 1. Build and tests (run by me)

| Check | Result |
|---|---|
| `build --release -p wte_app` | OK. Exe SHA-256 `9c953770…a97f`, identical to the builder's final `wte_final2.exe`. All scenarios below ran on private copies (`bin\wte_c3_<tag>.exe`). |
| `test --workspace --no-fail-fast` | wte_app 377 / 0 / 19 ignored; wte_core lib **796 passed / 3 failed** / 21 ignored; brush_tips 15, end_to_end 9, project_roundtrip 23. Total 1,220 passed, 3 failed, 40 ignored. |
| the 3 reds | `wa20` (known: cells 0.690..1.986). `wa7_bad_meshes_and_bad_pictures_never_fail` 286.2 ms and `check_a_own_bad_meshes_never_fail` 269.7 ms against their 200 ms wall-clock limit: the critic's M6, under the load of the other agents' builds. **Both pass when run alone** (`test_timing_alone.log`). No wrap code changed on this branch. Limits not touched. |
| `test --workspace --no-fail-fast`, again at the end on an idle machine (`test_workspace2.log`) | **1,222 passed, 1 failed (`wa20`), 40 ignored**: wte_app 377 / 0 / 19, wte_core lib 798 / 1 / 21, 15 + 9 + 23. Same as the builder's count. |
| GPU `test -p wte_app -- --ignored --test-threads=1` | **19 / 19**, including the two new `export_dialog_replaces_a_zip_through_the_confirm` and `a_zip_export_cancelled_while_packing_leaves_nothing`. |
| `--ignored export_at_4096` (debug) | Files 16 files, 750,738,556 B in 17.7 s; Zip 16 entries, 702,934,108 B in 25.2 s (builder: 19.0 / 31.5 s). Entries equal the loose files. Peak memory not re-measured. |
| `clippy --workspace --all-targets` | **0 warnings** |
| removed asserts / limits | Four removed assert lines, each a spec change replaced by the new expectation (`model.zip` -> `model_selection.zip` twice, `sets == Some([set 0])` -> `None`, scenario count 42 -> 45). No limit loosened, no new `#[ignore]` besides the GPU / 4096 tests. No `Cargo.toml` / `Cargo.lock` change; Rust sources LF. |

## 2. Scenario runs (release exe, 1280x800, every run `"ok": true`)

`export_unsettled` on ABeautifulGame, high_poly_sphere and the sphere sample; `export_flow` on ABeautifulGame, the four
generated cubes, torus_knot, gltf_node_transforms, ngons_tiling and two cubes I generated (`assets\collide.obj`, and
`collide2.obj` whose two sets are named `Body` / `body`); `export_errors`, `export_nouv`, `multi`, `export_selected`,
`export_zip`, `export_run`, `export`, `layers`, `undo`, `roundtrip`, `final_bake_async`, `smoke`, `default`;
`fuzz_path` with **new seeds**: ngons_tiling seed 23 x 1,000 steps, Lantern seed 29 x 600 steps (0 ERROR lines, 0
panics, "the document survived"); and `export_run` / `export` / `multi` on the r9 exe for comparison.

## 3. The claims, one by one

1. **Switching back from "Export this layer".** Code: `switch_scope` swaps `ExportTicks` only when the dialog was
   opened for layers and the choice crosses Selected <-> Visible / All; the format fields are re-synced before the
   end-of-frame `apply_flat` (read in the same frame after the radio row). Unit test plus `export_flow` asserts (every
   set's flattened texture planned after Visible; preset back on Selected) passed in all my flow runs. **Holds.** The
   scenario calls `switch_scope` directly, not through a click on the radio; the UI path is the same function call.
2. **`<base>_selection.zip`.** In every flow run `_all` holds exactly `<base>.zip` and `<base>_selection.zip`; my
   Python (`tools/stable/check_exp2_verify.py`) found every selection entry byte-identical to the full zip's entry of
   that name. The dialog shot of `export_selected` (read at 3x) shows `sphere_selection.zip` next to "Pack into one .zip
   file". **Holds.**
3. **Designs across sets.** Checked independently on ABeautifulGame: the full zip holds nine design files
   (`_stamp_Logo` in 8 set/slots, `_text_WTE` in Chessboard). The selection zip holds all seven `_stamp_Logo` files of
   the projected stamp (Bishop_Black, Chessboard L03, Knight_Black L03, Pawn_Body_Black, Pawn_Body_White,
   Pawn_Top_Black, Queen_Black, each with ink) plus Paint 1, and not the wrapped stamp's `Knight_Black_L02` file nor the
   one-set text. That is exactly "2 selected + 6 linked". `multi` wrote both sets' layers ("Exported 2 files").
   **Holds.**
4. **Full-export names.** `export_run` on this exe and on the r9 exe: 7 / 7 files, same names, byte-identical, so the
   reference-plan naming changed no ordinary name. On `collide2.obj` (sets `Body` / `body`) the full zip carries the
   `_2` names (`collide2_body_2.png`, `..._L00_paint_Base_2.png`, ...) and every `_one` / `_sel` file exists in the full
   zip with the same bytes. Core tests cover Body/body, Mat:1/Mat_1, 64-char prefix. **Holds.**
5. **Multi-select Delete / Duplicate.** `export_flow` (Ctrl+J and Delete on the pair, one undo each, undone to the same
   document key) passed everywhere; the core and dispatch unit tests pass. The `export_selected` dialog shot shows the
   footer "Duplicate (2)" / "Delete (2)" and "2 layers selected · tools paint on Logo" (read at 4x). Fuzz with the new
   actions: 0 errors. **Holds.**
6. **Export right after a wrapped stamp.** My `export_unsettled` runs: ABeautifulGame Final landed after 5.8 s,
   **longest frame 20 ms** over 776 frames; high_poly_sphere Final after 19.8 s, **longest frame 67 ms** over 2,385
   frames; sphere 3 ms. Files byte-identical to the export after waiting and equal to the Final pixels. (The
   high_poly_sphere parameterization took 19.2 s here against the builder's 3.8 s for the same work, cg_iters 720:
   machine load, not export.) Code review: `pump` is not gated on a modal job, so the Final runs while the export
   waits; a modal job blocks shortcuts (`Jobs::busy` returns the modal job first), so the document cannot change while
   the job waits for its snapshot; cancel during the wait returns "cancelled". **Holds.** I did not re-run the
   "without the fix" debug build.
7. **Crashed exports.** I made a real crash: an `export_flow` copy of my exe killed (`taskkill /F`, my own process)
   while it packed `ABeautifulGame.zip`, leaving `ABeautifulGame.zip.tmp-88576` (33.7 MB) and the stage folder with 75
   files. I moved both into the fuzz run's export folder; the first fuzz export logged "removed ... left by process
   88576, which is not running" for both, and every later fuzz export asserted no `.tmp-` entry. **Holds.**
8. **Toast.** Logs show "Exported 1 file to shot_path_one", "Exported ABeautifulGame_selection.zip (8 files) to
   shot_path_all". **Holds.**
9. **Test gaps.** `export_errors`: the three blocked exports end in "Export failed" + error toast ("is a folder",
   os error 183), nothing written, then the stale leftovers of a dead pid are removed and `notes.txt` stays.
   `export_nouv` ok. GPU replace-zip and cancel-while-packing: green. 4096: re-run, numbers above. Fuzz: two new seeds
   green. **Holds** (read-only ACL still not exercised; the probe file stands in, as the builder says).
10. **Python check.** My own script (independent of the builder's `verify_zip_vs_loose.py`) on 9 flow folders: full
    zip == loose files (names and bytes), 1980 dates, every selection / `_one` / `_sel` file equal to the full zip's
    file of that name, flattened PNG vs. re-composite of the per-layer PNGs within **1.33/255** worst (ABeautifulGame,
    15 sets), ORA (`mimetype` first and stored, layer count, merged = flattened) and PSD headers, no `.tmp-` left.
    **ALL OK.**

## 4. Defects

### Major

None.

### Minor

- **m1 (fixed by me).** ARCHITECTURE.md 8.11: the sentence "Gate 3 added named mid-run captures (`Step::Shot`) for the
  report:" and its paragraph were glued onto the end of the new `export_errors` table row, so the Markdown table ran
  into the following paragraph. Moved back into its own paragraph after the table.
- **m2.** The dialog note `linked_extra` counts *layers* but says "+ their layers in N other texture sets". Two
  selected designs that span the same other sets give N = 2 x the number of sets. The file list under it is right;
  the line overstates. `linked_extra` has no unit test. Wording or a set count would fix it.
- **m3 (test strength).** `export_flow`'s `expect_layer_files` now takes the expected listing of the linked layers from
  `export::plan_contents`, the code under test. The independent oracles for item 3 are `multi` (`doc.stamp_layers`)
  and the core test; my ABeautifulGame file check (section 3, item 3) confirms the app result independently.
- **m4 (pre-existing, environment).** The wall-clock unit tests `wa7` and `check_a_own` go red under CPU load from
  other processes (M6 of the critic); they passed alone. Not caused by this branch; the decision on them is still the
  user's.
- **m5 (nit).** With several layers selected, the footer's Rasterize acts on the active layer without saying so (Merge
  Down has a tooltip and the menu says "Merge Down (active layer)"). The line "tools paint on Logo" names the active
  layer.

### Model-caused (not export)

- ABeautifulGame: the 30 % wrapped stamp inks 1.85 M texels of a 2048² set and the projected stamp inks up to 0.9 M
  texels per set: shared / mirrored UVs of the chess pieces, as check_exp_2 found. The export reproduces the layers
  exactly.

## 5. Not verified / open

- Save right after a wrapped stamp still settles on the UI thread (`start_save`; builder's open item).
- The dialog's flattened preview refreshing when a Final lands after the dialog opened (`prepare_export` no longer
  settles) was not looked at.
- The 4096 peak memory and the "without the fix: 1,832 ms" number were not re-measured.
- A real read-only (ACL) export folder.
