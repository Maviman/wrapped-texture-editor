# Stable, polish pass: notes

Opus, 2026-11-23. Worktree `G:\software_projects\texture_wrapper_ap_r9`, branch `stable`, on top of 0542f02 (fix pass 2
report on f889d69). Six small fixes from the fix pass 2 report's open items. Raw outputs (git-ignored) are in
`target\polish\`: logs, `runs\` (one folder per scenario run), `bin\`, the driver `tools\run_all.sh`. The user was in a
fullscreen game the whole time: every cargo command and every run used `start /b /low /wait`, and nothing measured
speed.

## The six items

1. **Export dialog, the note under "Selected"** (`dialogs/export.rs`). `linked_extra` now returns `LinkedExtra { designs,
   layers, sets }`, where `sets` counts the distinct texture sets that hold the added layers. `linked_note` builds the
   text: "its" for one design and "their" for several, "layer" for one layer and "layers" for more, then "in 1 other
   texture set" or "in N other texture sets". Two designs across the same two sets, both selected in set 0, read
   "+ their layers in 1 other texture set" (before: "in 2 other texture sets"). Test:
   `the_linked_note_counts_texture_sets_not_layers`.
2. **Save waits off the UI thread** (`io_flows::start_save`). Save no longer calls `stamp_bakes::settle`. It now
   waits the way export does. The export feed became `SnapshotFeed { tx, since, what }` in
   `WteApp::snapshot_feed`, and `feed_waiting_export` became `feed_waiting_snapshot`. While a Final is pending, the
   "Saving project" job starts at once and shows "Finishing stamps", and Cancel works. It gets the snapshot once the
   bakes have landed. The UI state is captured and the pending edit flushed when Save is pressed, as before. `settle`
   is now called only before Merge Down and Rasterize. New scenario **`save_unsettled [PATH]`** (46 scenarios now).
   It shares its first steps with `export_unsettled` (`place_wrapped_at_once`, `wrapped_stamp_pending`):
   - it places a wrapped stamp and runs Save As at once, then asserts that no frame took 300 ms or more;
   - it then runs Save As a second time after waiting;
   - `same_project` checks that the two manifests are equal apart from `saved_unix`. Each manifest entry carries the
     file's size and CRC-32. It also checks that both files load with the same sets, layer ids, names and pixels, that
     these equal the document's, and that the stamp's saved layers have ink.
3. **Wrap panel, the "Only N %" line** (`tool_options::largest_wrap_loss`). When the line shows and no cause line
   passed its threshold, it names the largest cause and its share. The causes are: lost to cuts, in the gaps between
   separate parts, where it meets a neighbouring part, beyond where the picture closed on itself (only when the spine
   met itself and no ends line shows), and the rest. The rest reads "running off the surface" when an edge was reached
   and "hidden or folded" otherwise. Nothing is named below 0.5 %. DamagedHelmet Jump on now reads "Only 97 % of the
   picture lies on the surface, mostly lost to cuts (2 %)". The panel-lines test has a case for each cause and a case
   with a cause line (no suffix). The contradiction sweep is unchanged. The ends cause is not worded "ends meet", so the
   sweep's "ends meet" checks still hold.
4. **The font sample timing test** became `perf_sample_job_for_the_machine_fonts`. It is ignored with the same
   reason text as WA7 and check:A, and the limit stays 0.5 s. The gate commands are in ARCHITECTURE.md section 9 and
   in HANDOFF.md 4.4:
   - `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1`;
   - `cargo test -p wte_app perf_ -- --ignored --test-threads=1`;
   - the GPU app tests: `cargo test -p wte_app -- --ignored --skip perf_`.
5. **Footer Rasterize with several layers selected** (`panels/layers.rs`, `rasterize_multi_hover`). The tooltip now
   names the active layer, as Merge Down's does. Enabled, it reads "Rasterizes the active layer, X, into a paint layer
   (not the other selected layers)". Disabled, it reads "Rasterize acts on the active layer, X, only (not the other
   selected layers): it is already a paint layer", or "...: it is locked". With one layer selected, the tooltip is
   unchanged. Test: `footer_rasterize_names_the_active_layer_when_several_are_selected`. It hovers the real button
   until egui shows the tooltip, which happens after 0.5 s of rest. The disabled case uses a paint layer and the
   enabled case a stamp layer. Clicking queues `RasterizeLayer` for the active layer only.
6. **The expected layers of export_flow** (`selftest/scenarios.rs`). `expected_selection_layers` works them out from
   the document's links: each selected stamp or text layer's `StampId`, through `Document::stamp_layers`.
   `expect_layer_files` now fails when the planned layers differ from these. It is used by `export_flow`,
   `export_selected` and `multi`. `export_unsettled` checks that the plan holds exactly the stamp's layers. Test:
   `expected_selection_layers_follow_the_stamp_links` (it agrees with `export::linked_selection` on a design that spans
   two sets).

## Each test fails without its fix

- Items 1, 3 and 5: I made three temporary mutations in one build and ran the three tests: **all 3 failed**. The
  mutations were: sets counted as layers, the cause suffix dropped, and the multi-select tooltip turned off. I then
  restored the files. Note that the restored files carried older mtimes, so the next `cargo test` reused the mutated
  binary and failed the same three. After a `touch` and a rebuild, all three pass.
- Item 2: I built `bin\wte_polish_oldsave.exe` from this tree with only one change, a `settle` call put back into
  `start_save`. This is the old behaviour. It still has stable-bg's background window. On ABeautifulGame,
  `save_unsettled` failed: "a frame took 1674 ms (limit 300 ms)", after "settle waited 1672 ms". With the fix, the
  longest frame was 11 ms over 822 frames.
- Item 6: the new check compares the plan against an independent list. Its unit test pins that list down. I did not
  make a mutation of `plan_contents` to show the scenario failing.
- Item 4 moves a test. It has no fail-without test of its own: the perf test is still the check.

## Results

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` | wte_app **380 / 0 / 19 ignored** (`test_app.log`, after the touch); wte_core lib 801 / **1 failed** (WA20, the known red) / 24 ignored; brush_tips 15, end_to_end 9, project_roundtrip 23. Total **1,228 passed, 1 failed, 43 ignored** |
| Serial perf tests | wte_core 2 / 2 (`perf_core.log`); wte_app 1 / 1 (`perf_app.log`). Pass or fail only: the machine was under load, so these are not speed results |
| GPU app tests `-- --ignored --skip perf_` | **18 / 18** (the 19th is now the perf test) |
| `cargo clippy --workspace --all-targets` | 0 warnings |
| `cargo build --release -p wte_app` | OK. The runs used `bin\wte_polish.exe`, SHA-256 906F9CE1…F871. The final rebuild after restoring the item-2 mutation is F3D88730…0EBD. The two differ only in the PE header's TimeDateStamp (offset 0x80, 2 bytes) |
| Focus probe (smoke) | the game (BlackOps3) kept the foreground: app in front 0 of 39 samples, the foreground changed 0 times |

All 11 scenario runs were `"ok": true` (`runs\summary.txt`):

- `save_unsettled` on ABeautifulGame: the Final landed 2.2 s after the save; the longest frame was 11 ms; both projects
  are equal (15 sets, 31 layers). On the sphere: the longest frame was 4 ms.
- `export_unsettled` on ABeautifulGame and on the sphere.
- `export_selected`, `multi`.
- `export_flow` on the cubes, ABeautifulGame and DamagedHelmet. ABeautifulGame: "the selection of 2 layer(s) brought 6
  linked layer(s)".
- `wrap_surface` on DamagedHelmet: the step-10 panel line reads "... mostly lost to cuts (2 %)". `wrap_surface` on the
  built-in sample.

I did not compare these runs against older outputs file by file. The only intended change in output is the panel line.

## Not done

- No speed measurement (the user was in a game).
- `docs/reports/STABLE_GATE_REPORT.md` is unchanged. Its open items 4, 6, 7, 8, 9 and 10 and user decisions 2, 5 and 6
  are handled here.
