# Build note: S2 - More doors: the live PNG copy and the watched folder (STICKER_PLAN 7 "S2")

Builder, 2026-11-27, branch `stickers` (from b1c6536, the S1 check). Kind K + U, stretch. Two more doors between WTE
and PhotoCraft (or any program), both plain files: no FFI, nothing started, no new crate, `Cargo.toml` / `Cargo.lock`
unchanged.

- **(a) Live PNG copy** for PhotoCraft's File > Place Linked: "Keep a PNG copy up to date" of a sticker (Sticker tab:
  the selected stamp's sticker, the selected tray sticker) or of a raster paint layer (Layer menu and the row menu:
  "Keep a PNG Copy up to Date"; while on, a block at the top of the Layers tab's list) keeps
  `<project>_links\<name>_k<id>.png` / `<name>_l<id>.png` written 500 ms after the last change of its picture.
- **(b) Watched folder** for PhotoCraft's File > Generate > Image Assets (critic D1): File > Watch Folder for
  Stickers... (and the tray's "Watch a folder...") - every PNG that appears or changes there becomes, or updates, a
  kept tray sticker named after the file.

## What was built

| Item | Where | Notes |
|---|---|---|
| Core: a sticker's finished picture drawn off the UI thread | `wte_core/src/doc_stickers.rs` (`Document::sticker_picture_job`, `StickerPictureJob::run`; re-exported) | a cached render or the trivial picture is taken on the UI thread, else the render input; `run` (Send) = `sticker_picture(id, None, scale)`, same scale clamp / 4096 halving. ARCHITECTURE 5.20 |
| Core: the modified flag for settings saved outside the history | `wte_core/src/doc_links.rs` (`Document::mark_kept_settings_changed`) | bumps `links_revision` (what `is_modified` already compares); no entry. ARCHITECTURE 5.28 |
| The live copy | new `wte_app/src/doors/live.rs` | content key by value (`sticker::canvas::sticker_key` / `Layer::version` + texture size) plus the file's path; 500 ms after the key last changed (a copy just turned on or restored: at once); at most one write job; none starts during an interaction; the job draws, encodes (PNG Balanced), skips equal bytes, writes with `project::atomic::write_atomic`; a failed write retried after 2 s; status line; `saved` / `restore` |
| The folder worker | new `wte_app/src/doors/watch.rs` | one polling thread (`read_dir` + `metadata`, 1 s / 250 ms settle / poke on focus), `*.png` only, settled = two equal polls + a full read + a decode, 30 s give-up, `Known` content skipped, `Missing` / `Back`, at most 256 pictures and 64 MB each, Read again (`rescan`), counted repaints |
| Commands, frame hook, applying, controls | new `wte_app/src/doors/mod.rs` | `DoorAction`, `DoorsUi` (`AppState::doors`), `run`, `pump` (after `link::pump`), `interacting`, the batch `apply` (one history group; "Add Sticker" / "Update Sticker from Folder" / "Stickers from Folder"), `live_copy_row`, `folder_block` |
| Saved with the project | `io_flows.rs` (`UiState::live_copies`, `UiState::watch_folder`, `lenient`) | written only while used (an old project's bytes do not change), read leniently; reset and restored in `install_document` |
| Wiring | `action.rs` (`AppAction::Doors`), `dispatch.rs` (the folder dialog via `resolve_path`), `app.rs`, `main.rs`, `state.rs` (`AppState::doors`, 11 `PanelHit`s), `selftest/mod.rs` (idle check waits for a running write) | |
| Controls | `sticker/panels.rs` (selected stamp), `sticker/tray.rs` (selected tray sticker, the folder block), `panels/layers.rs` (row menu item, the Layers tab block, `live_copy_item`), `panels/menu_bar.rs` (File: Watch Folder for Stickers... / Stop Watching Folder; Layer: Keep a PNG Copy up to Date) | every control a button / box with a `PanelHit` |
| Scenarios | new `selftest/scenarios/sticker_doors.rs`; `scenarios.rs` (75); `sticker_tab.rs` (`doc_hash`, `reveal`, `hit_click` now `pub(super)`) | `sticker_live_copy`, `sticker_watch_folder` (below) |
| Docs | ARCHITECTURE 1 (1.12), 2.2, 5.20, 5.28, 7.2, 7.3, 8.11, **8.18** (new), 9.1, 9.2, 10; HANDOFF 4.1, 5.4; `dist/HOW_TO_TEST.txt` chapter "Two more doors to PhotoCraft" (6 steps) and an optional line under the hand-test list; README; release notes | |

## Deviations (with reasons)

1. **Where the settings live.** The plan does not say. Both are saved in `Document::ui_state` (app-owned JSON the core
   round-trips), not in the core manifest: no format or reader-version change, an old project's bytes stay the same,
   older builds ignore the keys. Changing them sets the modified flag (`mark_kept_settings_changed`), never a history
   entry (like link state, R6).
2. **A live copy needs a saved project**, as a link does (5.3): the copy lives in the links folder beside the project.
   Before the first save the box / item is disabled with the sentence `NEEDS_SAVE`.
3. **What is copied.** A sticker's own finished picture (its layers with their layer styles) at scale 1; stamp-level
   styles are left out (several stamps with different styles can show one sticker). Image and text stamps are not
   offered: they become a sticker first (the box says so). A paint layer: its pixels at the texture's size (locked
   layers too: the copy only reads them).
4. **The file name is fixed** when the copy is turned on (`<name>_k<id>.png`, `<name>_l<id>.png`): renaming the sticker
   must not break PhotoCraft's placed link. Turning it off keeps the file.
5. **Equal bytes are not written again**, so reopening a project (copies restored "at once") leaves the files and their
   times alone. A Save As writes the copies into the new links folder (the old ones stay).
6. **Matching a file to its sticker** is by a record (file name -> sticker and layer, the content last applied), not by
   searching the tray for a sticker of that name: an unrelated sticker that happens to have the name is never taken
   over. New stickers are named after the file stem, as the plan says. A file whose recorded sticker or layer is gone
   becomes a new sticker.
7. **A new size from the file.** When the sticker's artboard is still the size of the picture its layer shows now (as
   the file made it; after an undo the older picture's) the artboard follows the new size, so placements are refitted
   by 2.9 (same size: they stay exactly). An artboard the user changed in WTE is kept. (Image Assets trims each layer to
   its content, so sizes change between saves.)
8. **Undo and the folder.** One undo step per batch; undoing an update does not make the folder apply the file again
   (as R6); "Read again" does. A picture the sticker already shows makes no entry.
9. **Limits not in the plan**: only `*.png` directly in the folder (no subfolders, so Image Assets' `icons/a.png`
   naming is not followed), at most 256 pictures (by name, said once), 64 MB per file, the project's links folder
   refused as the watched folder (WTE's own copies would come back), one watched folder per project.
10. **Polling, not `ReadDirectoryChangesW`**: the same approach as the link watcher (no FFI, no directory handle), 1 s.
11. **Core API added**: `sticker_picture_job` instead of P7a's `sticker_canvas_job`, which always draws (a photo
    sticker would be rendered again) and has no 4096 px halving.
12. **Scenarios**: the folder is chosen with `DoorAction::WatchFolder(Some(path))` (a self-test ignores file dialogs, as
    every other scenario that opens files does); the watch scenario polls at 200 ms to keep it short. The idle check
    waits for a running write only, not for the 500 ms delay (old scenarios are unaffected; the new ones use `Until`).
13. **Hand tests**: the binding report list (1-25) is unchanged; S2's checks are a HOW_TO_TEST chapter with an
    "optional" line under the list.

## Tests

- Core `doc_stickers_tests::s2_sticker_picture_job_runs_on_a_worker_to_the_same_picture` (layered + styled text,
  cached render, scale 1/2, trivial blob / owned, over the 4096 limit, unknown id; `mark_kept_settings_changed`).
- App `doors/tests.rs` (16 + 1 ignored perf): live copy written at once then after the delay, no job while unchanged,
  off keeps the file; quick changes = one write; a paint layer's copy (and a never painted one); needs a saved project;
  restored copy with equal bytes not rewritten and bad saved entries dropped; waits while busy, follows Save As; a
  failed write retried, a deleted sticker `Gone` until undo; a renamed sticker keeps its file. Folder worker: new and
  changed PNGs once, no repaints while quiet, Read again; known content not reported; half-written / broken files, the
  folder's loss and return; the 256 cap. In the app: new / updated / resized / batch, the artboard after an undo;
  deleted sticker, locked user, bad bytes; choosing and stopping; the whole path through `pump`, held while busy, saved
  and restored, Read again of an unchanged picture makes no entry.
- `io_flows::tests::ui_state_keeps_the_live_copies_and_the_watched_folder`; `scenarios::tests` (75).

## Numbers measured (this machine, RTX 5070, release unless said)

- `cargo test --workspace --no-fail-fast`: **1,963 passed, 1 failed (WA20, the known one), 70 ignored**
  (`target\sticker\s2\test_full.log`; the perf test was added after that run: 71 ignored now). The doors tests again
  after the clippy fix: 16 / 16.
- `cargo clippy --workspace --all-targets`: **0 warnings** (one `clone_on_copy` in a test fixed).
- `cargo build --release -p wte_app`: OK; exe SHA-256
  `c260e36ccdd880be260b76de702e1f1b51464061b56ef4446963a17fa3e8016c` (copy `target\sticker\s2\wte_s2.exe`).
- `perf_s2_live_copy_jobs` (release, serial): a 2048 x 2048 sticker with the Classic sticker style on its layer - UI
  thread 30 ms for the first copy (the blob's one-time decode into the document's cache), 47 us after a change; the
  whole write (render, PNG, atomic write) 230 ms / 177 ms on the worker. A 1024 x 1024 paint layer: UI thread 52 us,
  write 107-177 ms.
- Scenarios (`target\sticker\s2\runs\`, `runs2\`; off-screen, no focus taken): `sticker_live_copy` ok, `sticker_watch_folder`
  ok (also with `WTE_OLE=1 WTE_PEN_SUBCLASS=1`); the affected ones ok: `sticker_tray`, `sticker_controls`, `sticker_styles`, `layer_link`, `sticker_link`,
  `sticker_view`, `sticker_keys`, `sticker_drop`, `sticker_dragout`, `smoke`, and `sticker_flow_path` on ABeautifulGame.glb
  and DamagedHelmet.glb (its Sticker tab now shows the unticked copy box under the link block). Shots looked at:
  `live_copy_on` (the tray box ticked, "PNG copy: badge_k3.png, up to date", Show file, Watch a folder...),
  `live_copy_edited` (the Sticker view with the badge nudged right, cut by the artboard), `live_copy_layer` (the red
  stroke, the Layers tab's block "PNG copy: Paint 1_l2.png, up to date" above the rows), `watch_two` (two tray
  thumbnails, "Watching: doc-assets", "2 pictures from it are in the tray", Read again / Show folder / Stop watching),
  `watch_updated` (the blue badge on the sphere, selected, the toast), `watch_reopened` (the wide magenta badge, the
  tray scrolled into view with the Watching lines). Each matches its expect text.
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` -> `target\sticker\s2\sweep\`): 21 / 21 `"ok": true`. Against
  S1's sweep: 216 the same, `export_checks/shot.png` differs only in the fading "Exported" toast box; a re-run of that
  job (`target\sticker\s2\sweep_rerun\`) is pixel-identical to S1's re-run: **217 / 217**. Against the P0 baseline: the
  known `side_tabs` small-window shots and `shot_tab_sticker.png` only (as since P6b); exports and manifests identical.

## For the checker

- The two threads: one write job at a time (`LiveCopies::job`), one folder worker per watched folder; a document change
  drops both (`DoorsUi::reset`; a running write still finishes its file).
- `apply` runs only when `interacting` is false; check the hold, the batch group and that the record's content is not
  rolled back by an undo.
- `UiState`: no key while unused (identity sweep), lenient read.
- Only cargo, Python and the WTE exe in `--screenshot` / `--scenario` mode were started; PhotoCraft, photocraft-cli and
  Photoshop never.
