# Round 3 report — Wrapped Texture Editor (gate, 2026-11-18)

Paths are relative to `E:\claude_programs\wrapped_texture_editor\target\round3\` unless they start with `round3_audit\`
(= `…\target\round3_audit\`), `round2\` (= `…\target\round2\`) or `crates\`.

## Status
Twelve of the fourteen items meet every acceptance line. R3-1 meets its reduced lines (user decision 1). R3-12 and R3-13 meet
theirs but were checked only on generated fixtures. R3-4 roughly halves the stamp freezes, but four of its five acceptance
lines are still not met. R3-5 export time (6.0-8.2 s over 7 runs, target 12 s) is met with byte-identical output. 725 tests pass, clippy
is clean, all scenarios pass and the 159-model sweep is 151 ok against 150 in round 2, with no crash, panic or hang.

## Final exe
- `wte_round3.exe`, SHA-256 **7D0E0B571560DF4EA0FED4D08058C63B6C5B89F6B4E41038264A9B684E133E39**, built from
  `work\merged` (`cargo build --release --offline`; source unchanged since stage 2, rebuild reports nothing to compile).
  `wte_gate.exe` is the same file. Baseline: `round3_audit\wte.exe` (round 2), SHA-256 CEF11D8A…F87A.
- Runs on an older exe: the stage-1 runs (`runs_gate1\`) and the first stage-2 sweep (`sweep_exe3FC8.csv`,
  `runs_gate_exe3FC8\`) used exe 3FC84A93…6043. They are superseded: the stage-2 final sweep, fuzz and export checks, and every
  stage-3 run used 7D0E0B57. Package evidence (`runs_a\`, `runs_b\`, `runs_c\`) used package exes. This report takes none of
  its numbers from those runs, except the export-only memory figure marked below.

## Tests
| Check | Result | Evidence |
|---|---|---|
| `cargo test --workspace --offline` | 725 passed (wte_app 224, wte_core 488, end_to_end 8, project_roundtrip 5), 0 failed, 17 ignored (round 2: 670 / 15) | `notes\gate3_test.out` |
| `cargo clippy --workspace --all-targets --offline` | exit 0, 0 warnings | `notes\gate3_clippy.out` |
| `todo!` / `unimplemented!` in `crates\` | 0 | grep of `work\merged\crates` (gate3 note) |
| 42 scenario runs: 19 built-ins + pan2d, path scenarios, 3 sizes, 3 fuzz repros, open_path edge cases | 41 ok, 1 intended error (missing .glb) | `runs_gate3\batch1.csv`, `batch2.csv`, `runs_gate3\runs\` |
| Logs of those runs: "Unknown layer/stamp", "Invalid parameter", "panicked" | 0 / 0 / 0 | `runs_gate3\runs\*\shot.log` |
| Sweep, workflow_path on 159 models | 151 ok / 8 fail (round 2: 150 / 9) | `sweep.csv`, `sweep_3m.csv`, `notes\gate2.md` |
| Fuzz, 600 steps, seeds 3 and 11, 5 models | 10/10 exit 0, 0 ERROR lines, 0 panics | `fuzz_gate2.csv`, `fuzz_check_gate2.txt` |
| Exports (decision 3, Pillow + strict PSD reader) | PASS, 268 texture sets in 151 folders, 0 failing | `check_exports_gate2_all.txt`, script `check_exports.py` |

## Acceptance per item (plan section 3; section 6 overrides)
Unit tests named below are in `notes\gate3_test.out` (final source). "final 1/2" are two runs of the final exe, back to back with
the baseline (`perf3\`).

| Item | Acceptance line | Verdict | Number | Evidence |
|---|---|---|---|---|
| R3-1 | v1 manifest + wrong hash gets the new "older automatic texture layout" text | met | test ok | `project::load::tests::older_generated_layout_is_named_in_the_warning` |
| R3-1 | same manifest with the right hash loads without a warning | met | test ok | same test |
| R3-1 | frozen `uv_hash` constant pins the version-2 layout (`no_uv_tetra`) | met | test ok | `mesh_build::tests::generated_uv_layout_of_importer_version_2_is_frozen` |
| R3-1 | matching versions load and save as before | met | uv_hash equal to round 2 for 150 of 150 projects; no-UV save + reopen gives the same composite | `uvhash_gate2.txt`; `runs_gate3\runs\import_nouv\shot.log` |
| R3-1 | ARCHITECTURE rule: a `uv_hash` change must bump IMPORTER_VERSION | met | present | `work\merged\ARCHITECTURE.md` line 2593 |
| R3-2 | `layers`: Duplicate adds exactly one undo entry; undoing it gives no warning | met | 1 entry, 0 warnings at 1280x800, 900x600, 640x480 | `runs_gate3\runs\layers*\shot.log` |
| R3-2 | fuzz repros log no "Unknown layer" | met | 0 in s42 multi_uv_sets, s13 Fox, s13 huesitos | `runs_gate3\runs\fuzz_s*\shot.log` |
| R3-2 | `owed_final` dropped when its layer or stamp is gone (unit test) | met | test ok | `panels::tests::owed_final_is_dropped_when_its_layer_or_stamp_is_gone` |
| R3-3 | open_path on a saved no_uvs.obj project ends with no dialog | met | assert "no Warnings box" ok on 3 projects | `runs_gate3\runs\open_path_no_uvs_obj_wtep\`, `open_path_nouv_tetra_wtep\`, `open_path_r7_wtep\` |
| R3-3 | unit test on the warning list | met | tests ok | `project::load::tests::reopening_a_generated_layout_project_repeats_no_import_facts`, `model_facts_are_the_informational_import_warnings` |
| R3-4 | ngons_tiling "image stamp" <= 500 ms | **not met** | final 614 / 585 ms (base 1499 / 1614 ms) | `perf3\workflow_steps.csv` |
| R3-4 | ngons_tiling "transform the stamp" <= 1000 ms | **not met** | final 1666 / 1682 ms (base 3066 / 3183 ms) | `perf3\workflow_steps.csv` |
| R3-4 | fuzz seed 7 (1000 steps): no central/dispatch frame over 500 ms | **not met** | final, 6 runs: 5 / 4 / 5 / 6 / 6 / 5 frames, worst 1029-1194 ms (base 21 / 21, worst 2260 / 2253 ms) | `perf3\runs_final_*\…_s7\shot.log`, `verify_perf\runs_final\…_s7\shot.log`, `verify_final\runs_final_*\…_s7\shot.log` (`…` = `fuzz_path_shapes_ngons_tiling.obj`), `notes\gate3.md`, `notes\final_fix.md` |
| R3-4 | fuzz seed 7: at most 2 windows under 30 fps per quarter | **partly** | final Q1-Q4, 6 runs: 0/1/6/1, 0/1/5/2, 0/2/4/3, 0/3/5/3, 0/1/5/1, 0/1/4/4. Q1 always meets it; Q3 fails in 6 of 6 runs, Q4 in 3 of 6, Q2 in 1 of 6. Base 5/1/7/7 and 5/1/7/6 | same logs, `round3_audit\quarters.py` |
| R3-4 | ABeautifulGame "image stamp" <= 1500 ms | **not met** | final 1993 / 2242 ms (base 2100 / 2133 ms) | `perf3\workflow_steps.csv`, `perf3\bench_abg.out` |
| R3-4 | bakes bit-identical to round 2 for the existing tests + a new tiled-UV test | met | existing stamp_bake tests unchanged and passing; Full == plain reference on ABeautifulGame | `stamp_bake::tests::tiled_uv_bakes_equal_the_plain_reference`, `occlusion_shortcuts_agree_with_the_bvh`; `perf3\bench_abg.out` ("identical") |
| R3-5 | ABeautifulGame "export every format" <= 12 s | met | step 6650 / 6880 ms, job 6.51 / 6.68 s; 7 runs in all: job 6.04-8.24 s (base 27.04 / 27.04 s step, 26.92 / 26.91 s job) | `perf3\workflow_steps.csv` |
| R3-5 | exported files byte-identical to round 2 | met | export_run 7/7 against the round-2 audit run; ABeautifulGame 85/85, ngons 27/27, spider 24/24, DamagedHelmet 8/8, many_islands 8/8, tex_8192 8/8 against the baseline exe | `perf3\export_identity.txt` |
| R3-5 | memory peak stated | partly | whole workflow on ABeautifulGame: 1.90 GiB final, 2.21 GiB base (gate). Export-only: 1.27 / 1.29 GB in parallel against 1.05 / 1.08 GB serial (package A probe, measured under load) | `perf3\memory_abg.txt`; `runs_a\r35\*.ws.csv` |
| R3-6 | unit test replacing the "accepted simplification" | met | tests ok | `history::tests::is_modified_and_mark_saved`, `an_evicted_saved_state_stays_modified` |
| R3-6 | `unsaved`: save, stroke, undo -> OpenSample without the dialog; redo -> modified | met | asserts "undo back to the saved state: no star", "redo: star again", "Open Sample replaced the saved document without the prompt" | `runs_gate3\runs\unsaved\shot.log` |
| R3-7 | workflow_path: no toast intersects the side panel / `panel_rects` | met | assert ok in 151 sweep runs + 3 gate-3 runs | `runs_gate3\runs\workflow_path_*\shot.log`, `sweep.csv` |
| R3-7 | toast width <= 380 px | met | in the same assert (`toasts::MAX_WIDTH` = 380) | same; `toasts::tests::toasts_stay_in_the_view_corner_and_wrap_long_paths` |
| R3-8 | open_path on a missing .glb names the file | met | "Cannot open DoesNotExist.glb: file not found" | `runs_gate3\runs\open_path_missing_glb\shot.log` |
| R3-8 | fuzz logs contain no "Invalid parameter" | met | 0 in 4 gate-3, 10 gate-2 and 2 seed-7 fuzz runs (baseline seed 7: 7 per run) | `runs_gate3\runs\fuzz_*`, `fuzz_check_gate2.txt`, `perf3\runs_*_*\fuzz_*` |
| R3-9 | `text`: the layer is named "WTE 2026" after the change | met | assert ok at 3 sizes; screenshot shows layer and "Selected: WTE 2026" | `runs_gate3\runs\text\shot.log`, `shot.png` |
| R3-9 | a renamed layer keeps its name | met | test ok | `doc_stamps::tests::a_text_stamp_with_its_automatic_name_follows_its_text` |
| R3-10 | `layers`, 6 layers at 1280x800: Delete on screen, outside the list's scroll area | met | assert ok (also 900x600, 640x480) | `runs_gate3\runs\layers\shot.log` |
| R3-10 | 640x480 `stamp` screenshot without overlapping status text | met | status bar without overlap | `runs_gate3\runs\stamp_640x480\shot.png`; `panels::status_bar::tests::status_items_never_overlap` |
| R3-10 (dec. 2) | `stamp` and `text` at 1280x800 show the full picker and the Layers footer; Color rect inside the side panel | met | assert ok; screenshots read | `runs_gate3\runs\stamp\`, `text\` |
| R3-10 | Text options show the selected text; export folder shows the end of the path | met | screenshot; tests ok | `runs_gate3\runs\text\shot.png`; `dialogs::export::tests::long_folders_keep_their_end_and_the_middle_is_elided` |
| R3-11 | unit test on the clamp | met | tests ok | `view::view2d::tests::pan_of_ten_view_widths_keeps_32_points_of_the_texture_in_the_view`, `clamp_*` |
| R3-11 | a pan of 10 view widths leaves the texture in the view | met | overlap 32.0 pt in every pan2d case | `runs_gate3\runs\pan2d\shot.log` |
| R3-12 | ASCII FBX fixture (scale 2, offset 0.25) gives the uv_hash of the equivalent glTF | met | test ok | `import::fbx::tests::texture_uv_transform_gives_the_uv_hash_of_the_equivalent_gltf` |
| R3-12 | all 21 FBX samples keep their round-2 uv_hash | met | 21 / 21 | `import::fbx::tests::sample_uv_hashes_are_unchanged_by_texture_transforms`; `notes\gate2.md` section 6 |
| R3-13 | quantized cube imports like its float twin (within 1e-4) | met | test ok | `import::gltf::tests::quantized_meshes_import_like_their_float_twin` |
| R3-13 | meshopt and Draco are still refused by name | met | test ok | `import::gltf::tests::required_draco_extension_is_reported` |
| R3-14 | nonfinite.obj opens and passes workflow_path with one warning naming the line | met | ok; `BadObjNumbers { first_line: 7, lines: 34, dropped_faces: 119, degraded_faces: 22 }` | `runs_gate\workflow_path_nonfinite_nonfinite.obj\shot.log`; `import::obj::tests::bad_number_lines_keep_the_numbering` |

## Sweep by group (workflow_path, final exe, against round 2)
| Group | Models | Round 2 | Round 3 |
|---|---|---|---|
| glb_samples | 19 | 19 ok / 0 fail | 19 ok / 0 fail |
| fbx_samples | 22 | 21 ok / 1 fail | 21 ok / 1 fail |
| harness shapes | 12 | 12 ok / 0 fail | 12 ok / 0 fail |
| topologies (incl. mesh_3m x2, run alone) | 106 | 98 ok / 8 fail | 99 ok / 7 fail |
| **Total** | **159** | **150 ok / 9 fail** | **151 ok / 8 fail** |

The only change is nonfinite.obj, which now passes (R3-14). Every model that passed in round 2 still passes. The 8 failures are
the same as in round 2: 6 intended import errors (transparentTest.fbx, binary_garbage.glb, lying_length.glb, empty.obj,
lines_points_only.obj, zero_bytes.obj) and slivers.glb / slivers.obj, a scenario limit that was not planned for this round.
Sources: `sweep.csv`, `sweep_3m.csv`, `round2\sweep.csv`, `notes\gate2.md`.

## Performance before and after (idle machine, one process at a time; `perf3\`)
| Measure | Round 2 (base 1 / 2) | Round 3 (final 1 / 2) |
|---|---|---|
| ngons_tiling "image stamp" | 1499 / 1614 ms | 614 / 585 ms |
| ngons_tiling "transform the stamp" | 3066 / 3183 ms | 1666 / 1682 ms |
| ngons_tiling "export every format" | 1665 / 1934 ms | 417 / 398 ms |
| ABeautifulGame "image stamp" | 2100 / 2133 ms | 1993 / 2242 ms |
| ABeautifulGame "export every format" | 27038 / 27036 ms | 6650 / 6880 ms |
| ABeautifulGame save / reopen | 4476 / 6811 and 5183 / 8681 ms | 3525 / 4783 and 4210 / 4939 ms |
| ABeautifulGame save / reopen, all runs on the final exe (†) | base 6 runs: save 3758-5183, reopen 6811-9384 ms (median 8652) | final 7 runs: save 3525-4993, reopen 4783-10101 ms (median 6045) |
| ABeautifulGame whole run | 62.1 / 65.1 s | 37.7 / 39.2 s |
| ABeautifulGame whole run, all runs (†) | base 6 runs: 62.1-66.6 s | final 7 runs: 37.7-51.8 s (median 41.6) |
| ABeautifulGame "transform the stamp", all runs (†) | base 6 runs: 1334-1450 ms | final 7 runs: 1392-2419 ms (median 1482) |
| ABeautifulGame peak working set (whole run) | 2.21 GiB | 1.90 GiB |
| spider.fbx "image stamp" | 814 / 833 ms | 431 / 429 ms |
| tex_8192 "image stamp" / "fill" / "undo all / redo all" | 723 / 735, 2295 / 2230, 2702 / 2543 ms | 1581* / 783, 2563 / 1945, 2710 / 2670 ms |
| many_islands, DamagedHelmet "image stamp" | 368 / 348, 383 / 382 ms | 367 / 365, 565 / 383 ms |
| fuzz seed 7: central/dispatch frames > 500 ms | 21 / 21 (worst 2260 / 2253 ms) | 5 / 4 (worst 1113 / 1088 ms); 6 runs in all: 4-6 (worst 1029-1194 ms) |
| fuzz seed 7: windows < 30 fps Q1/Q2/Q3/Q4 | 5/1/7/7, 5/1/7/6 | 0/1/6/1, 0/1/5/2; 4 later runs 0/2/4/3, 0/3/5/3, 0/1/5/1, 0/1/4/4 |

(†) The single-pair numbers above are the gate's `perf3\` pair. The later runs of the same exe (`verify_perf\runs_final*`,
`verify_final\runs_<final|base>_<1-3>\`, see "After verification") show that save, reopen, transform and the whole run vary
much more from run to run than the first pair suggested: the "Opening project" job alone took 4.15-7.34 s on the final exe
and 5.44-7.30 s on the base exe. The final exe is still faster than the base in median for reopen and whole run.

\* The click took 91 ms. The rest of the 1581 ms is the scenario's own work before the click (stamp PNG write, ray grid),
so this is a scenario outlier (`perf3\runs_final_1\workflow_path_tex_8192_tex_8192.obj\shot.log`). Full step table:
`perf3\workflow_steps.csv`. Raw CSVs: `perf3\perf_final.csv`, `perf3\perf_base.csv`.

## Fixes made by the gate
| Stage | Fix | Regression test | Evidence |
|---|---|---|---|
| 1 | Added the missing scenario asserts: import_nouv save + reopen (no dialog, same composite), open_path "no Warnings box", workflow_path reopen asserts that no dialog is open | the asserts themselves | `notes\gate1.md` |
| 2 | Toasts overlapped the side panel by 7 pt when the Split view's set-tab row overflowed (tex_formats.obj). `App::ui` now anchors toasts to the area the central panel was given (`show_central`, `crates\wte_app\src\app.rs`); ARCHITECTURE 8.3 and 8.9 updated | `app::tests::central_area_ignores_content_that_overflows_into_the_side_panel`; tex_formats.obj sweep row | `notes\gate2.md` |
| 3 | None (no source change) | — | `notes\gate3.md` |
| After verification | None (no source change; two report corrections, see below) | — | `notes\final_fix.md` |

## Unresolved
1. **R3-4 targets** (numbers above). Causes measured in this stage:
   - ngons_tiling: one Full bake (~270-285 ms) per placement or drag release runs on the UI thread, on top of ~330 ms of fixed
     scenario time per step. The targets leave about 100-135 ms per bake. Meeting them needs Final bakes off the UI thread,
     which is an ARCHITECTURE 4.6 / dispatch change.
   - ABeautifulGame: the placement is one 1387 ms frame. On this model the occluder grids make the Full bake slower: 910 ms
     with them against 658 ms without, because building them costs 158 ms and there is little overdraw (0.38 fragments per
     texel, against about 10 on ngons) (`perf3\bench_abg.out`). Building the grid only when overdraw is high would save about
     250 ms. That still misses 1500 ms and would require re-running the whole stamp sweep, so it was not done in the gate.
   - fuzz seed 7: the remaining slow frames are the long text stamp's Full bake and **3D brush strokes on tiled UVs**
     (872 / 875 ms). The brush code is unchanged since round 2, and the baseline has the same kind of frame (994 ms). Stamp
     work alone cannot meet this line. Over 6 final-exe runs the "<= 2 windows under 30 fps per quarter" line fails in Q3
     every time (4-6 windows) and in Q4 in 3 of 6 runs (3-4 windows), not only in Q3.
   - The exact tint cache was not built.
2. R3-5: the export-only memory peak comes from package A's probe, measured under load. The gate measured only the
   whole-process peak.
3. R3-12: IMPORTER_VERSION was not bumped for FBX texture transforms. A project saved from such an FBX now shows
   MeshUvChanged; none of the 21 samples is affected. **The user should confirm this.** No real exporter file with a
   non-identity transform on its own UVs exists on disk. UVSwap, UV set selection and rotation about U/V are ignored.
4. R3-13: verified only on generated fixtures; integer attributes are honoured even without the extension declared.
5. R3-1: no version-1 no-UV project exists (decision 1), so the new warning is exercised only by edited manifests.
6. Pre-existing, not changed: the set-tab row is cut off in the Split view with many sets (tex_formats, many_materials); toast
   wording "Layer is locked" vs "The active layer is locked"; at 1280x800 with 5 or more layers the Tool options area is
   about 110 pt (allowed by decision 2).
7. PSD / ORA were never opened in a real editor (decision 3). The Pillow + spec checks pass.
8. Package A's change: exported files appear under their final names as each is written, and the disk flush happens once at
   the end ("Finishing" stage). After a power loss a renamed file can be incomplete (ARCHITECTURE 7.4).

## After verification
Three skeptics re-checked this report. Two claims were refuted, both by the "perf" skeptic, both minor, both about
measurement spread. Neither is a code defect, so no source file was changed and no regression test was added. The rebuild in
`work\merged` compiled nothing and gave the same exe, SHA-256 7D0E0B571560DF4EA0FED4D08058C63B6C5B89F6B4E41038264A9B684E133E39
(= `wte_round3.exe`). `cargo test --workspace --offline`: 725 passed, 0 failed, 17 ignored (`notes\final_fix_test.out`);
clippy exit 0, 0 warnings (`notes\final_fix_clippy.out`). The scenario, sweep and fuzz-repro runs of stages 2 and 3 used this
same exe, so they stand. To check the two claims, the gate ran on an idle machine, one process at a time
(`verify_final\run.ps1`): ABeautifulGame workflow_path final, base, final, base, final, base, then fuzz_path seed 7 (1000
steps) three times on the final exe. All 9 runs exit 0. The 3 fuzz logs contain 0 "Invalid parameter", "Unknown layer" or
"panicked".

| Refuted claim | Check | What was done |
|---|---|---|
| Fuzz seed 7: windows < 30 fps per quarter final 0/1/6/1 and 0/1/5/2, so only Q3 fails | `quarters.py` on the `perf3\` logs gives 0/1/6/1 and 0/1/5/2 again, so the old numbers are right for those two runs. On the skeptic's run it gives 0/2/4/3, and on 3 new runs 0/3/5/3, 0/1/5/1 and 0/1/4/4. Q4 (and once Q2) also goes over 2. Frames > 500 ms: 5 / 6 / 6 / 5 (worst 1154 / 1194 / 1029 / 1147 ms) | Report corrected: the acceptance row, the performance table and Unresolved 1 now give all 6 runs and say that Q3 fails every time and Q4 in 3 of 6 runs. Verdict stays "partly". |
| ABeautifulGame reopen 4783 / 4939 ms and whole run 37.7 / 39.2 s with the final exe | The two `perf3\` runs give these numbers, but they were the fast end. Final exe, 7 runs: reopen 4783-10101 ms (median 6045), save 3525-4993 ms, whole run 37.7-51.8 s (median 41.6 s), transform 1392-2419 ms. Base exe, 6 runs: reopen 6811-9384 ms (median 8652), whole run 62.1-66.6 s. The spread is in the background "Opening project" job (final 4.15-7.34 s, base 5.44-7.30 s) and in the GPU scene build after it (792 / 1233 ms in the fast and slow run that were read). Both exes show it, and the first run after a pause is the slowest (open / import 10.6 s in `verify_final\runs_final_1`), so the cause looks like disk cache and write-back of the ~300 MB just saved, not the code | Report corrected: ranges and medians were added next to the single pair (rows marked †) with a note. No acceptance line depends on these numbers. The export line R3-5 stays met over all 7 runs: job 6.04-8.24 s, step 6228-8383 ms, target 12 s. |

## How to repeat
Build (PowerShell, in `work\merged`): `cmd /c "E:\claude_programs\wrapped_texture_editor\cargo.cmd test --workspace --offline"`,
`… clippy --workspace --all-targets --offline`, `… build --release --offline`; then copy
`work\merged\target\release\wrapped_texture_editor.exe` and check the SHA-256 above.
- Scenarios: `powershell -ExecutionPolicy Bypass -File runs_gate1\run.ps1 -Jobs runs_gate3\jobs1.txt -Csv <csv> -Exe wte_round3.exe -Runs <dir> -Parallel 3`
  (put `tiny_cube.*` from `…\target\compat\harness\shapes` into `runs_gate3\r7_src` first). Then delete `r7_src` and run
  `runs_gate3\jobs2.txt` the same way.
- Performance: `powershell -ExecutionPolicy Bypass -File perf3\run_perf.ps1 -Passes 2` on an idle machine. It uses the jobs in
  `perf3\jobs_perf.txt` and runs final and base alternately, one at a time. Then run
  `python -I round3_audit\quarters.py <log>`, `python -I round3_audit\slow.py <log> 500` and `python -I perf3\lastact.py <log> 400`.
- Bake probe: in `work\merged`, set `$env:WTE_STAMP_BENCH_MODEL=<model>` and run
  `cargo.cmd test -p wte_core --release --offline --lib -- --ignored stamp_bench_tiled_model --nocapture`.
- Memory: start the exe on ABeautifulGame with `--scenario workflow_path` and sample `PeakWorkingSet64` (`perf3\memory_abg.txt`).
- Sweep: `round3_audit\sweep.ps1 -Jobs round2\jobs_wf.txt -Csv <csv> -Exe wte_round3.exe -Runs <dir> -Parallel 3 -TimeoutSec 900`.
  Run the mesh_3m models alone with `-Parallel 1 -TimeoutSec 3600`.
- Exports: `python -I check_exports.py <export folders>` (see `notes\gate2.md` section 5).
- Fuzz: set `WTE_FUZZ_SEED` and `WTE_FUZZ_STEPS` and use `--scenario fuzz_path` (the job format `scenario|model|seed|steps` in
  `sweep.ps1`).
