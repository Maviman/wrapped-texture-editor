# Build note: P11 - Drag out (STICKER_PLAN 7 "P11", 6.4, 6.5; rev 3: 13.5, the P11 row of 13.9)

Builder, 2026-11-27, branch `stickers` (from 3a55742, the P10b check; plan revision 3). Windows FFI (kind F). WTE's
pictures can now leave the window by drag: a Layers row, a row of the Sticker view's Layers list, the Sticker tab's
thumbnail of the selected stamp and a tray thumbnail become an OS drag when the pointer leaves the window with the
button held, and the new "PSD" handle hands out the stamp's linked PSD. Hand-written `raw-dylib` FFI, no new crate,
`Cargo.toml` / `Cargo.lock` unchanged. The user's workflow ("I would be able to manuver the sticker in the wrapped app,
and I can edit the sticker itself in photoshop") is kept: a PSD-handle drag only makes / hands out the link (the
placement on the model is not touched), and Ctrl+S in the editor updates the sticker in place through the P9 watcher as
before.

## What was built

| Item (plan 6.4, 6.5, 13.5, 13.9, 7 "P11") | Where | Notes |
|---|---|---|
| Declarations | `ole/com.rs` | `IDropSourceVtbl`, `IID_IDROPSOURCE`, `DRAGDROP_S_DROP / CANCEL / USEDEFAULTCURSORS`, `MK_LBUTTON`, `RegisterWindowMessageW`, `PostMessageW`; layout test |
| The drag source | new `ole/drag_source.rs` | `DragJob` in a static slot, `DRAG_ACTIVE`, `request` (a request while a drag runs is dropped: `Refused::Busy`; a failed post takes the job back), `post_to_window` (the registered message `WTE.OleDrag.v1`), `handle_message` (takes the job, sets the flags, `DoDragDrop` through the `DoDrag` seam, releases both objects, queues `ole::DragEnded`), WTE's `IDropSource` (Esc cancels, the primary button up drops, `GiveFeedback` default cursors, `catch_unwind`), `window_seam` (the real `DoDragDrop` only in a non-test build), `Recorder` / `RecordedDrag` (tests and self-test runs: reads the data object like a drop target, asks the drop source its three questions, answers a canned effect), `own_drag_on_this_thread` |
| The subclass | `ole/subclass.rs` | the drag message is the one message it answers itself (0, not forwarded); every other message still reaches the previous procedure unchanged |
| Queue, install | `ole/mod.rs` | `DragEnded { tag, effect }` with its own queue (`push_drag_ended`, `take_drag_ended`), the message registered at install, `drag_window()` (the window when the subclass is installed) |
| Drop target | `ole/drop_target.rs` | takes nothing while this thread runs one of WTE's own drags (R3, whatever the drag carries); `DragEnter` reads only the 12-byte header of a `WTE.Sticker.v1` block (`get_prefix`; check P10a's open point) |
| The state machine | new `dragout.rs` | `DragSource { Layer, StickerLayer, Stamp, Tray, PsdHandle }`, `DragOutUi` (`AppState::drag_out`), `DragPort` (`TestPort`, `WindowPort`), `begin` / `start_job` (the job at recognition: the PNG (Fast) as a new file + `CF_HDROP` + `PNG` + `CF_DIBV5` + `WTE.Sticker.v1` for stickers), `step` / `pump` (the result, a release in the window cancels and removes the file, the post when ready + held + outside, also in the frame a late job finishes), `input_hook` (the injected release + `PointerGone`, egui's payload cleared, `stop_dragging`, `released_this_frame`), `finish` (a PNG nobody took is removed; a taken PSD marks the link opened; the Photoshop toast once), `install` (folder, start-up cleanup, port), `deliver_test_post` |
| Folder and manifest | new `dragout/files.rs` | `%LOCALAPPDATA%\WrappedTextureEditor\dragout\` (self-test: `<out_stem>_dragout`), `manifest.json` (name, size, mtime ns), `write_new` (never over a file: "Badge (2).png"), `record`, `remove_unused`, `cleanup` (only listed, unchanged, older than a day; changed ones forgotten; never the folder), `file_name` (Windows-safe, 80 characters) |
| The PSD handle | `sticker/panels.rs` (`psd_handle`), `link/flows.rs` (`prepare_handle`, `HandleReady`, `handle_editor`, `HANDLE_TEXT_HINT`), `link/mod.rs` (`EditRequest::prepare_only`, `for_handle`) | a 48 x 24-pt "PSD" box with grip dots, the caption "drag to your editor: stays linked" under it; a drag makes the link (save first; the shared picture; the image stamp promoted; a new link's PSD written; a missing file written again), the link's editor = the default editor; a click only shows how it is used |
| Drag sources | `panels/layers.rs`, `sticker/layers_ui.rs` (also: a reorder skips the injected release), `sticker/panels.rs` (thumbnail `Sense::drag`), `sticker/tray.rs` (`Sense::click_and_drag`) | `AppAction::DragOut(source)` at `drag_started_by(Primary)` |
| Copy shared | `clip.rs` | `stamp_take` / `Picked` / `Take` (Copy and drag out take a stamp's picture the same way) |
| App wiring | `app.rs`, `action.rs`, `dispatch.rs` (guarded only for the PSD handle), `state.rs` (`drag_out`, `PanelHit::StickerTabPsdHandle`), `main.rs` | |
| Self-test | `selftest/mod.rs` (`Step::ScreenHold`, `is_idle` waits for `drag_out.busy()`), new `selftest/scenarios/sticker_dragout.rs` | 70 scenarios |
| Docs | ARCHITECTURE 1 (status), 2.2 (files), 8.2, 8.3, 8.7, 8.11 (`ScreenHold`, the `sticker_dragout` row), 8.13 (pitfall 27), 8.17 (new part "Drag out"), 9.2; HANDOFF (scenario list, `WTE_OLE`, the branch table, 5.4, the hand-test line); `dist/HOW_TO_TEST.txt` "Drag, drop, copy, paste" 20-27 | |

Tests (all new or extended): `ole::drag_source::tests` (4), `ole::subclass::tests::the_drag_message_runs_the_drag_and_is_not_forwarded`,
`ole::com::tests` (the `IDropSource` layout and id), `dragout::tests` (10 + the ignored `perf_k11`), and
`panels::layers::tests::every_row_control_takes_its_click_and_the_row_still_drags` now expects the `DragOut` before the
`MoveLayer` (the reorder itself unchanged). The plan's test list item by item:
- the data object of each drag kind (`the_data_object_of_each_drag_kind`): the thumbnail of a styled sticker stamp offers
  exactly CF_HDROP, PNG, CF_DIBV5, WTE.Sticker.v1; the PNG decodes to `stamp_picture(id, true)` (with the stamp styles),
  the DIB holds the same pixels, the sticker block is this process's and keeps the styles, the `DROPFILES` path exists
  in the folder and is the PNG, the file is in the manifest; the stamp's row the same (a second file "Logo (2).png"); a
  painted paint layer at texture size without a sticker; a tray sticker without stamp styles; a sticker layer's pixels;
- the state machine (`a_drag_posts_once_when_the_pointer_has_left_and_the_job_is_done`, with a job held on a channel):
  the job starts at recognition; nothing is posted inside, nor when the pointer leaves before the job is done; posted in
  the frame it finishes; exactly one post over ten more frames; the end injects the release at the pointer and
  `PointerGone`, clears egui's drag payload (the row drag) and marks the frame, the next frame injects nothing;
- releasing first posts nothing and removes the file (`releasing_first_posts_nothing_and_removes_the_file`), also while
  the job still runs (the worker removes its own file);
- a second request while `DRAG_ACTIVE` is set is dropped (`a_second_request_while_a_drag_runs_is_dropped`: from inside a
  running drag, the request is `Busy` and a message finds no job; a failed post; a panic in the seam);
- cleanup (B7, `the_cleanup_removes_only_old_unchanged_files_of_wtes`): a listed file two days old is removed, an hour-old
  one kept, a listed file saved over by the user kept and forgotten, a file not in the manifest kept, a manifest name
  pointing outside the folder ignored, the folder and the manifest kept, no manifest = nothing removed;
- the PSD handle (`the_psd_handle_asks_first_then_links_and_hands_out_the_psd`): unsaved -> "Save first", no link, no
  drag; a picture two stamps show -> the shared question, no drag; "Only this one" -> the link record and its PSD, the
  editor = the default editor, no editor started; the drag hands out CF_HDROP with the PSD only; the link counts as
  opened; the default editor changed to Photoshop -> the link's editor follows at the next press;
- the toast fires once (`the_tab_bar_toast_needs_photoshop_and_a_taken_drag` and the handle test): not with PhotoCraft
  alone, not after an Esc'd drag, once after a taken drag with Photoshop, not again;
- `DoDragDrop` itself is never called by a test (the recorder seam; the real one is not compiled into a test build).

## Deviations (with reasons)

1. **A stamp's styled picture is rendered on the UI thread at the drag's start, as Copy does** (plan A23: "a job renders
   and encodes"). `Document::stamp_picture` needs `&mut Document` (its caches); the same split as P10b's Copy keeps one
   way of taking a stamp's picture (`clip::stamp_take`). The expensive part for big pictures, the PNG encode and the
   file write, is on the job; a paint layer is not rendered at all (its shared buffer goes to the job). Measured: a
   4096 x 4096 layer's job 199 ms off the UI thread, the UI thread's calls 0.1 / 0.4 ms (`perf_k11`).
2. **`DragEnded` has its own small queue** (`ole::take_drag_ended`) instead of an `OleEvent` in the drop events' queue:
   `dnd` drains that queue every frame and it is capped at 256 events (a stream of hovers could push the end out);
   the end must reach `raw_input_hook` before egui's frame. Tests on many threads cannot take each other's ends either.
3. **WTE's own drop target ignores every drag WTE itself runs** (a thread-local flag set around `DoDragDrop`), not only
   drags with the `WTE.Sticker.v1` marker of this process: a paint layer's PNG has no marker and would otherwise be
   imported again when dropped back on WTE (R3). The P10a marker rule stays for stickers of this process.
4. **The PSD handle makes the link when its drag is recognised** (egui's `drag_started`), not at the bare press: a click
   on the handle then never writes a file by accident; it only shows "Drag the PSD handle onto your editor (in
   Photoshop: onto the tab bar) ...". A question on the way (Save first, the shared picture) stops that drag; its
   answer makes the link (`EditRequest::prepare_only`, routed by `edit_in` to `prepare_handle`) with the status line
   "<file> is ready: drag the PSD handle to <editor>", and the user drags again.
5. **The handle's editor without a default** (13.9 says "the default editor"): with two known editors and no default the
   chooser cannot be asked in the middle of a drag; the order is the default (or the only known) editor, else the
   link's own, else the first known one, else PhotoCraft (`handle_editor`).
6. **The Photoshop toast** comes after a taken PSD-handle drag when the link's editor is Photoshop **or Photoshop is
   known on this machine** (WTE cannot see where the drop landed; a user with both editors may drop into Photoshop).
   Not after an Esc'd drag. Once per session.
7. **A linked file without WTE's latest changes is handed out as it is**, with the status line "The file does not have
   WTE's latest changes yet: Send to <editor> first to include them" (no send-first question in the middle of a drag;
   "Edit in" still asks it).
8. **A sticker layer row** hands out a picture layer's pixels with its mask applied and without layer styles (as Copy in
   the Sticker view); a text or shape layer is drawn alone (`sticker_layer_on_artboard`) and cut to what it covers.
   `WTE.Sticker.v1` only for a sticker stamp, a styled image stamp (as Copy, P10b deviation 4) and a tray sticker.
9. **A paint layer without pixels** (an unallocated buffer) is not dragged out: status line "This layer has no pixels
   yet: nothing to drag out".
10. **A PNG nobody took is removed at once** (Esc, or a place that takes no files: the effect is none); a taken one
    stays until the day-old cleanup (Explorer copied it; PhotoCraft opens it from the folder).
11. **Help's "drag out with the mouse" line** is left to P13 (its file list has the Help texts); HOW_TO_TEST 27 says it.
12. **`Step::ScreenHold`** (new): a real drag out never delivers the release to winit (OLE takes it), so the scenario's
    pointer steps must not either; the app's own injected release is what the scenario then sees.
13. The P10a check's open point "drag enter copies the whole sticker block" is fixed here (`get_prefix`), since WTE's own
    drag out now offers such blocks to other WTEs.

## Numbers

- `cargo test --workspace --no-fail-fast`: **1,890 passed, 1 failed (WA20, the known one), 66 ignored**
  (`target\sticker\p11\test.log` for the whole workspace, `test_app2.log` for `wte_app` after the layer test's update:
  `wte_app` 731 / 0 / 25, `wte_core` lib 1,102 / 1 / 41, integration 15 + 9 + 8 + 25).
- `cargo clippy --workspace --all-targets`: 0 warnings (`target\sticker\p11\clippy.log`).
- `cargo build --release -p wte_app`: OK, exe 33,152,512 bytes, SHA-256
  `6261c978ab0c81d5d9b2adb8a6f840e536bc49e0f1c53667721edb08eec92f3a` (copy `target\sticker\p11\wte_p11.exe`).
- `perf_k11` (`--release --ignored`): the 4096 x 4096 layer's drag-out job 198.6 ms on its worker; `start_job` 0.09 ms
  and the worst `step` 0.41 ms on the UI thread (8.4: "the UI thread never waits for it").
- Scenarios (`tools/objtabs/sweep.sh`, off-screen, no focus taken), all `"ok": true`: `sticker_dragout` (3 s, 213 frames),
  `sticker_drop`, `sticker_clip`, `sticker_view`, `sticker_link` (29 s), `smoke` (`target\sticker\p11\runs\`); with
  `WTE_OLE=1 WTE_PEN_SUBCLASS=1`: `sticker_dragout`, `smoke`, `default`, `pen`, `companion`, `sticker_clip`
  (`runs_ole\`). The `WTE_OLE=1` logs show the pen subclass, then "ole: the drag-out message WTE.OleDrag.v1 is 0xc395",
  the key subclass and the drop target registered. The five `sticker_dragout` shots are **pixel-identical** with and
  without `WTE_OLE=1 WTE_PEN_SUBCLASS=1`.
- Shots of `sticker_dragout`, each looked at against its `expect`: `dragout_after_row` (the disc sticker with its outline
  selected on the sphere, the Layers rows Badge / Paint 1 / Base in order, no floating row or line, status "Dragged out
  Badge.png"; matches), `dragout_sticker_tab` (the Sticker tab: "Badge", "Sticker, 1 layer", the PSD handle with its
  grip dots and the caption under it, Edit Sticker, Export Picture..., Edit in PhotoCraft, Edit in...; matches),
  `dragout_save_first` ("Save first" with Cancel and Save... over the red sphere; matches), `dragout_psd_toast` (the
  toast "In Photoshop the sticker stays linked only when it opened as its own tab. Drop it on the tab bar, not on a
  picture.", the Sticker tab with "Link: linked", "Editor: Photoshop", Badge_k3.psd, Edit in Photoshop, Send to
  Photoshop; status "Dragged out Badge_k3.psd: Ctrl+S in Photoshop updates the sticker"; matches); the final shot the
  same window after the sixth drag.
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\p11\sweep\`): 21 / 21 `"ok": true`.
  `compare_baseline.py` against the P0 baseline: 214 the same, the two known `side_tabs` small shots (five tabs since
  P6b) and `shot_tab_sticker.png` (new since P6b), as in P10a / P10b; against P10b's sweep (`target\sticker\p10b\sweep\`):
  **217 / 217 the same**. No old project or flow changed (no job writes a drag-out file; the folder is not even made).
- Programs started: cargo, Python (`-E -P`), and the WTE release exe only with `--screenshot ... --scenario ...`
  (off-screen, no focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe` (the scenario's
  "Photoshop.exe" is an empty stub file in the run folder, never started; the launcher is the stub). No test or self-test
  run calls `DoDragDrop`, posts a window message or touches `%LOCALAPPDATA%` (tests: `target\wte-test-tmp\dragout\`;
  self-test runs: `<out_stem>_dragout`).

## For the checker

- `unsafe` to read: `ole/drag_source.rs` (`handle_message`'s object lifetimes, `DropSource`'s reference count, the
  recorder's `GetData` / `ReleaseStgMedium` / enumerator release), `ole/drop_target.rs` (`get_prefix`,
  `read_global_prefix`, `read_stream_prefix`), `ole/subclass.rs` (the drag message branch).
- The subclass: every message but `WTE.OleDrag.v1` and the forwarded key-downs reaches the previous procedure untouched;
  the pen subclass still gets its messages (scenario `pen` with `WTE_PEN_SUBCLASS=1 WTE_OLE=1`: ok above).
- Hand tests owed (plan 8.7 hand tests 8, 9, 10, 11; 13.6 hand test 23; `dist/HOW_TO_TEST.txt` "Drag, drop, copy,
  paste" 20-27): real drags to Explorer, PhotoCraft v0.2.0 and the latest, Photoshop's picture and tab bar, the PSD
  handle with Ctrl+S, Esc, the pen. Not verifiable here: whether Explorer / PhotoCraft / Photoshop accept WTE's data
  object as built (they read `CF_HDROP`, offered first; `SetData` answers `E_NOTIMPL`, which Explorer tolerates for
  plain file drags) and whether `DoDragDrop`'s modal loop and eframe's frames get along on the real window.
