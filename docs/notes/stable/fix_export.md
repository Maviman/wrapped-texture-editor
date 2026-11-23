# fix_export: the stable critic's export findings, fixed

Builder (Opus), 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_exp`, branch `stable-fix-export`
(from `stable` @ b064fee). Inputs: `critic.md` (H2, M1, M2, M3, M5, L1-L4, C5, item 7 of the report),
`STABLE_GATE_REPORT.md`, `check_exp_2.md` section 6. No wrap code changed, no crate added, Rust sources LF.
Evidence: `target\stable\fix_export\` (git-ignored): `bin\` (exes), `runs\` (new scenarios), `flow\` (export_flow +
`verify.json`), `fuzz\`, `nofix\` (item 6 without the fix), `mem4096.txt`.

## What changed, per item

| # | Finding | Fix | Test that fails without it |
|---|---|---|---|
| 1 | H2: "Export this layer", then Visible / All kept one set and per-layer PNGs only | `ExportDialogState::other_ticks` holds the normal ticks when the dialog opens for layers; `dialogs::export::switch_scope` swaps sets + flat + per-layer + ORA + PSD between the Selected side and the Visible / All side (and re-syncs the format fields, else the end-of-frame `apply_flat` turned a TGA back into PNG). `export_for_layers` sets `sets = None`. | unit `switching_from_the_layer_preset_to_visible_restores_the_normal_ticks`; export_flow asserts (Visible: every set's flattened texture, normal formats; back on Selected: the preset) |
| 2 | H2: a selection zip replaced `<base>.zip` | `export::package_name_for`: `<base>_selection.zip` under a layer selection; shown next to "Pack into one .zip file" and in the list header | core `a_selection_zip_never_replaces_the_full_package`, dialog `the_zip_name_follows_the_layers_choice`; export_flow: both zips side by side, the full one unchanged, every selection entry equal to the full package's |
| 3 | M1: a design across sets exported only the clicked set's layer | Designs are modelled as one layer per set linked by the same `StampId` (`LayerKind::{Stamp,Text}::stamp`, `Document::stamp_layers`). `export::linked_selection` completes a layer selection with those layers; `plan`, `plan_contents`, `run` apply it. The dialog says "+ its layer in 1 other texture set". | core `a_design_across_two_sets_exports_its_layer_of_every_set`; `multi` scenario (TwoMaterial: both sets' files, each its layer) |
| 4 | M5: `_2` names differed between a selection and the full export | Names come from `reference_names`: the deduper runs over the full reference plan (all sets when names carry the set, every layer incl. hidden, flat, ORA, PSD), and each export picks its files' names from that map | core `a_selection_has_the_full_exports_names_when_set_names_collide` (Body/body, Mat:1/Mat_1, two 69-char names sharing 64; PNG / TGA / no flat) and `a_selection_of_a_colliding_set_writes_the_full_exports_file` (bytes) |
| 5 | M3: Delete / Merge / Duplicate on the active layer only | `Document::remove_layers` / `duplicate_layers` (one undo step via `history::group_edits`, which merges the entries into `Edit::Multi` and evicts once at the end). App: `DeleteLayers` / `DuplicateLayers`; footer "Duplicate (N)" / "Delete (N)" with tooltips, Layer menu "Duplicate N Layers" / "Delete N Layers" / "Merge Down (active layer)", Delete key and Ctrl+J act on the selection; Merge Down's tooltip names the active layer. A locked layer stays with a warning. | core `several_layers_are_deleted_and_duplicated_as_one_undo_step`, `a_group_at_the_entry_limit_evicts_older_entries_only`; app `duplicate_and_delete_act_on_every_selected_layer_in_one_undo_step`; export_flow (Ctrl+J and Delete on the pair, one undo each, undone to the same document key) |
| 6 | M2: export right after a wrapped stamp settled on the UI thread | `start_export` no longer calls `settle`. When a Final is due the modal "Exporting" job starts at once and waits ("Finishing stamps", Cancel works) for the snapshot `io_flows::feed_waiting_export` sends once `pump` has landed the Finals (a due bake with no job running is settled as before, so it never hangs). `prepare_export` (opening the dialog) no longer settles either. | new scenario `export_unsettled` (frame work < 300 ms, export byte-identical to one after waiting, files = Final pixels). Without the fix (`nofix\`, the old `settle` put back into a debug build): ABeautifulGame **fails, a frame took 1,832 ms**. |
| 7 | L4: a crashed zip export leaves `<base>.zip.tmp-<pid>.d` | `export::remove_stale_temps(dir, is_running)` removes `<file>.tmp-<pid>` (export extensions) and `<zip>.tmp-<pid>.d` whose pid is not running and not ours; the export job calls it with `io_flows::process_running` (`OpenProcess` + `GetExitCodeProcess` over kernel32 FFI, as `crash.rs` does; no crate) | core `stale_temps_of_processes_that_are_not_running_are_removed`, app `this_process_runs_and_a_finished_one_does_not`; scenario `export_errors` (last part) |
| 8 | "Exported 1 files" | `io_flows::export_toast_text` | unit `export_toasts_count_files_in_the_singular_and_plural`; export_flow / multi / export_nouv assert no "1 files" toast |
| 9 | test gaps | see below | |
| 10 | export_flow on ABeautifulGame, gltf_node_transforms, torus_knot with the final exe; zip vs loose in Python | export_flow now also writes everything as loose files (`<out>_<model>_loose`); `tools/stable/verify_zip_vs_loose.py` | see "Runs" |

Item 9:
- **Model without UVs** through export and through save, reopen, export: scenario `export_nouv` (flattened, layers, ORA,
  PSD byte-identical before and after the round trip; the zip equal to the loose files).
- **Non-ASCII** model, set and layer names in a zip, read back: core `non_ascii_names_round_trip_through_a_zip` (CJK set
  name, emoji + katakana layer, katakana base; entries = loose names and bytes; the raw entry name is UTF-8).
- **4096** with two sets x five layers, every format, loose and zip: core `export_at_4096_with_several_sets_and_layers`
  (ignored), measured with `tools/stable/peak_mem.py` (numbers below).
- **Read-only / blocked folder through the app**: new `preflight` in the core (no planned path may be a folder, a probe
  file must be creatable) runs before the first file, so such a folder fails with nothing written; core
  `a_blocked_export_folder_fails_before_writing_anything`; scenario `export_errors` (folder path under a file, a planned
  file that is a folder with an old file next to it, the zip name a folder: the "Export failed" message, an error toast,
  nothing new written, the old file unchanged, no temp file, the app goes on). A real read-only ACL was not set up (it
  would change folder permissions); the probe covers it. A failure *while* writing (full disk, a file held open by a
  viewer) still leaves the files written before it (documented, 7.4).
- **Replacing an existing zip through the app's confirm**: GPU `export_dialog_replaces_a_zip_through_the_confirm`.
- **Cancelling a zip during Packing through the app**: GPU `a_zip_export_cancelled_while_packing_leaves_nothing` (12
  noise UV stamps; the test waits for the stage "Packing", then clicks the progress dialog's Cancel: no zip, no stage
  folder, the created folder removed, "Export cancelled", revision unchanged). Ran 4 times, green each time.
- **Fuzz**: `fuzz_path` gained Ctrl-click, Shift-click, Duplicate / Delete of the selection, rename (odd and non-ASCII
  names), export to `<out>_fuzz_export` (random scope, formats, zip or files; asserts no temp file is left) and save +
  reopen. The first runs found that a fuzz export with no file kind fails ("nothing selected for export", which the
  dialog prevents with "Choose at least one kind of file"); the fuzz now follows the dialog's rule.

The self-test runner gained `scratch.values["expected_error_toasts"]`: a scenario that drives a failure on purpose
announces its error toasts, every other error toast still fails the run.

## Runs

Exes: `wte_final.exe` (SHA-256 5fbb78fe…eb20) for the fuzz and the scenario regression set; the final release build of
HEAD, `wte_final2.exe` (SHA-256 9c953770…a97f), differs from it only in `#[cfg(test)]` code (a test assert and a
dispatch test) and was used for the export_flow, export_unsettled, export_nouv, export_errors and multi reruns. Every
scenario ran on a copy of the exe under its own name.

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` (`test_workspace.log`) | **1,222 passed, 1 failed, 40 ignored**: wte_app 377/0/19, wte_core lib 798/1/21, brush_tips 15, end_to_end 9, project_roundtrip 23. The one red is the known `wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` (no wrap code touched). |
| GPU tests `--ignored` (app) | **19 / 19** (17 before + 2 new); the packing-cancel test 4 / 4 on repeat |
| `clippy --workspace --all-targets` | **0 warnings** |
| `build --release -p wte_app` | OK |
| export_flow (final exe) on ABeautifulGame, gltf_node_transforms, torus_knot, the four generated cubes | all `"ok": true`. ABeautifulGame: `ABeautifulGame.zip` 227,008,447 B, 84 entries verified (15 sets recomposed exactly); the two-layer selection brought **6 linked layers of other sets** (8 files), `ABeautifulGame_selection.zip` 8 entries all equal to the full package's |
| `tools/stable/verify_zip_vs_loose.py` on those 7 flows (`flow2erify.json`) | **ALL OK**: testzip clean, entry names = loose names, every entry byte-identical to its loose file, 1980-01-01 dates, PNG/ORA stored, PSD deflated; every `_selection.zip` entry equals the full export's file of that name; no `.tmp-` anywhere |
| export_unsettled (item 6) | ABeautifulGame: Final landed after 1.0-1.3 s, **longest frame 11 ms**; high_poly_sphere: Final 4.1-5.4 s, **longest frame 38-40 ms**; sphere sample: 2-3 ms; exports byte-identical to the export after waiting and equal to the Final pixels. **Without the fix** (`nofix\`, old `settle` restored in a debug build): ABeautifulGame **fails, one frame 1,832 ms** |
| export_nouv, export_errors, multi | ok |
| scenario regression set (final exe, `final_sc\`) | export_unsettled, export_nouv, export_errors, export_selected, export_zip, export_run, export, multi, final_bake_async, layers, undo, roundtrip, import_nouv, smoke, default, workflow_path (DamagedHelmet): all ok |
| fuzz_path, 1,000 steps, Wrap mode (`fuzz\`) | ngons_tiling seed 7 / 11, WaterBottle seed 7 / 11: all `"ok": true`, **0 ERROR lines, 0 panics**, "the document survived". New actions per run (7/11): ngons 15/12 exports, 8/14 save + reopen, 10/5 Ctrl-clicks, 6/7 Shift-clicks, 10/3 Duplicate/Delete of a selection, 9/6 renames; WaterBottle 15/14 exports, 7/16 save + reopen, 8/4 Ctrl, 7/6 Shift, 9/5 selection commands, 9/8 renames; every export left no temp file |
| 4096 export (`mem4096.txt`, core test, debug-profile test binary) | 2 sets x 4096², 5 layers each, every format: loose 16 files, 750,738,556 B in 19.0 s; zip 702,934,108 B in 31.5 s; **peak working set 2,426 MB** for the whole test process (the document's own buffers included) |

## Not done / open

- Save right after a wrapped stamp still settles on the UI thread (`start_save`); only export was in scope.
- Merge Down, Rasterize, opacity, blend and move stay on the active layer (as the task says for Merge Down).
- CLAUDE.md still says `python -I` (critic C7); `-E -P` is what works here. Not changed (project instructions).
- Wrap findings of the critic (WA20, panel wording C4, FBX H1, timing margins M6) are outside this task.
