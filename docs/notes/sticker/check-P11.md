# Check note: P11 - drag out

Checker, 2026-11-27, branch `stickers` at fe41619 (the P11 build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P11", 6.4, 6.5, 13.5 and the P11 row of 13.9, and the user's decisions. Verdict: **pass after
fixes** (no blocker; one major and two minors fixed with tests, the rest noted below).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,890 passed / 1 failed (WA20 only) / 66 ignored (app 731 / 0 / 25, core
  1,102 / 1 / 41, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\p11check\test.log`).
- Clippy 0 warnings; release build OK.

## Plan 7 "P11" item by item

| Item | Found |
|---|---|
| Files: `ole/{subclass,drag_source}.rs`, `panels/layers.rs`, Sticker tab thumbnails and the PSD handle, `app.rs` (`raw_input_hook` pointer-up injection), the drag-out job, `dragout\manifest.json` | present (`dragout.rs`, `dragout/files.rs`, `sticker/{panels,tray,layers_ui}.rs`) |
| 6.4 sources: Layers rows (texture and sticker layers), thumbnails (selected stamp, tray), PSD handle; rows still reorder inside the window | yes (scenario steps 2, 4, 6, 7, 9; unit test for a Sticker-view row) |
| 6.4 what is handed over: CF_HDROP (one PNG) + PNG + CF_DIBV5 (+ WTE.Sticker.v1 for stickers); the PSD handle CF_HDROP with the linked PSD; `DROPEFFECT_COPY` only | yes (the recorder reads the real data object; formats and bytes asserted) |
| 6.4 the job at recognition (A23), `PngLevel::Fast`; post at once when ready + outside, or in the frame the job finishes; a drag that never left is cancelled and its file removed | yes, but the styles of a stamp were drawn on the UI thread at the drag's start: **finding 1** (fixed) |
| 6.4 folder + manifest + start-up cleanup (B7): only listed, unchanged, older than a day; never the folder | yes (`the_cleanup_removes_only_old_unchanged_files_of_wtes`) |
| 6.4 "How" (C4, R3): never `DoDragDrop` in `update`; static slot; registered message; the subclass handles it; `DRAG_ACTIVE`; `DragEnded`; `QueryContinueDrag` Esc / button up / continue; `GiveFeedback` default cursors | yes |
| 6.4 after the drag: injected release + `PointerGone`, the row drag cancelled | yes (also checked against winit 0.30.13: `WM_CAPTURECHANGED` from `DoDragDrop`'s capture resets winit's own capture count, so the eaten `WM_LBUTTONUP` leaves no stuck capture) |
| 6.4 pen: no workaround, a hand test | yes (HOW_TO_TEST 27; Help line left to P13: accepted, P13 owns the Help texts) |
| rev 3 / 13.9: caption "drag to your editor: stays linked"; the toast once per session after a PSD-handle drag; the link's editor = the default editor | yes (toast also requires Photoshop known or the link in Photoshop: accepted, the toast is about Photoshop) |
| ARCHITECTURE 8.3, 8.7, 8.13, 8.17, 9.2 | yes (also 1, 2.2, 8.2, 8.11) |
| Tests: data object per kind (PNG with the stamp styles, `DROPFILES` path exists), the state machine, release first, a second request while `DRAG_ACTIVE`, `DragEnded` injection, in-window reorder, cleanup (B7) cases, the PSD handle (save first, shared, link + PSD), `DoDragDrop` never called by a test | yes, all |
| Scenario `sticker_dragout` (row drag leaves the window, one recorded drag with the formats, the next move drags nothing) | yes, and more (reorder, paint layer, Esc, tray, PSD handle twice) |
| Checker: the subclass forwards every message but the drag message unchanged; `pen` with `WTE_PEN_SUBCLASS=1 WTE_OLE=1` | read: `route` answers only `msg == registered drag id` (0 = none registered never matches), everything else goes to `forward` even when looking at it panics; `pen`, `companion`, `default`, `smoke`, `sticker_clip`, `sticker_dragout` all ok with both flags; the log shows the pen subclass, the drag message id, then the key subclass and the drop target |

## Findings

1. **Major (fixed): every Layers-row drag of a styled stamp drew the stamp's styles on the UI thread.** `dragout::begin`
   calls `clip::stamp_take`, which ran `Document::stamp_picture(id, true)` (the tint and the styles at full resolution)
   before the job started (build deviation 1). Every reorder of a stamp layer's row starts that drag, so an ordinary
   in-window reorder froze the UI: measured (release) on a 2048 x 2048 image stamp, 34 ms (Outline), 51 ms (Glow), 77 ms
   (Classic sticker); about four times that at 4096. The builder's `perf_k11` measured only a paint layer, which is not
   rendered. Plan 6.4 / A23 and 8.4 ("the UI thread never waits for it"). Fix: core `Document::stamp_picture_parts`
   returns a `StampPicture` (the base picture from the caches + the tint / styling), whose `finish()` (`Send`) draws them;
   `stamp_picture` is now `parts.finish()` (the same code, byte-identical). `clip::stamp_take` takes the parts on the UI
   thread and finishes them in the worker part, so Copy (P10b) gains the same; the status line's size ("Copied the stamp
   "Badge" (70 x 70 pixels)") is now added by the worker (`start_copy(.., sized, ..)`). Now 0.05-0.17 ms. Tests:
   `check_p11_stamp_picture_parts_finish_on_a_worker_to_the_same_picture` (core: plain, tinted, styled, styled + tinted,
   without styles, finished on another thread = `stamp_picture`) and the ignored release test
   `perf_k11_a_styled_stamps_drag_starts_without_drawing_its_styles` (begin < 10 ms for three presets; the job's PNG is
   the styled picture).
2. **Minor (fixed): a drop that a program answered with "no effect" removed the PNG at once.** `DoDragDrop` returns
   `DRAGDROP_S_DROP` with `DROPEFFECT_NONE` both when the target refused the hover and when a program's `Drop` reported
   no effect; a program can still read the file after it returns (Chromium-based targets read dropped files
   asynchronously; what Photoshop CC 2019 and the latest PhotoCraft report is not known here). Removing the file then
   makes that open fail with no explanation, in exactly the hand tests 8 and 23. Fix: `ole::DragEnded` carries `dropped`
   (the result was `DRAGDROP_S_DROP`); only a drag that ended without a drop (Esc) removes its PNG at once; a dropped
   but untaken PNG stays listed in the manifest and goes with the day-old cleanup. Test:
   `check_p11_a_drop_without_an_effect_keeps_the_file_for_the_cleanup`; the Esc tests still remove the file.
3. **Minor (fixed): a large layer's drag held its PNG and DIB twice while the drag ran.** `post` cloned the entries into
   the data object (`data_entries(&[..])` + `bytes.clone()`) and kept the originals in `Phase::Posted(offer)` for the
   whole modal drag: about 85 MB extra for a 4096 x 4096 layer. The entries are now moved into the data object
   (`DragPort::post` takes the `Vec`).
4. Docs nit (fixed): HOW_TO_TEST's drag-out folder paragraph ran "dragout(with" into one long line; it now also says an
   Esc'd drag removes its file at once.

## Looked at, not changed

- A tray drag still renders (cached) and clones the sticker's picture on the UI thread (`sticker_picture`, a memcpy of
  at most 4096 x 4096 x 4 bytes): small, and only for a drag of a tray item, not for every reorder.
- A tiny race in `dragout::cancel`: a worker that read "not cancelled" just before the flag was set can send its result
  after the receiver was looked at and before it is dropped; its PNG is then neither removed nor listed (a stray file in
  WTE's own folder, never the user's). Not worth a lock.
- Two WTE instances share `manifest.json` (read-modify-write without a lock): a lost entry only means a file of WTE's is
  never cleaned up. Never a removal of something not WTE's.
- The PSD handle silently switches an existing link's editor to the default editor (13.9 says so); "Edit in" asks the
  P9b question before a link opens in a second editor, the handle cannot ask mid-gesture. Accepted as the plan's rule.
- A self-test run's drag-out folder is the relative `<out_stem>_dragout`, so its CF_HDROP paths are relative there; an
  interactive run uses the absolute `%LOCALAPPDATA%` path. Only the recorder ever reads the self-test's.
- The posted-message race the builder names (button released between the post and `DoDragDrop`): the posted message
  is dispatched before the queued button-up, so `DoDragDrop` then drops at once where the pointer is, which is what a
  release outside the window means anyway.
- Rule checks: no `SendInput` / `keybd_event` / `mouse_event`, no registry write, no real clipboard in tests; the real
  `DoDragDrop` is compiled only into a non-test build and reached only through a `WindowPort` (never in a self-test run:
  `TestPort` never posts a window message); no ArtCraft name; touched files keep LF; `Cargo.toml` / `Cargo.lock`
  unchanged.

## Results after the fixes

- `cargo test --workspace --no-fail-fast`: **1,892 passed / 1 failed (WA20 only) / 67 ignored** (app 732 / 0 / 26, core
  1,103 / 1 / 41, integration 15 + 9 + 8 + 25) (`target\sticker\p11check\test2.log`).
- Release `perf_k11` tests: the 4096 x 4096 layer's job 202 ms off the UI thread (start 0.09 ms, worst step 0.40 ms); a
  styled 2048 x 2048 stamp's drag start 0.05-0.07 ms.
- Clippy 0 warnings (`clippy.log`). Release build OK: exe 33,104,896 bytes, SHA-256
  `63b27813d1fd51b3ebf18d0b05b467cfb823852d1fe902b780e48f38f0a49fdd` (`target\sticker\p11check\wte_p11check.exe`).
- Scenarios (off-screen through `tools/objtabs/sweep.sh`, no focus taken), all `"ok": true`: `sticker_dragout`,
  `sticker_drop`, `sticker_clip`, `sticker_view`, `sticker_link`, `smoke` (`runs\`); with `WTE_OLE=1
  WTE_PEN_SUBCLASS=1`: `sticker_dragout`, `smoke`, `default`, `pen`, `companion`, `sticker_clip` (`runs_ole\`).
- Every `sticker_dragout` shot looked at:

| Shot | Expect text | Seen |
|---|---|---|
| `dragout_after_row` | disc sticker with thin dark outline, selected; Layers in usual order, no floating row or line; status "Dragged out Badge.png" | matches |
| `dragout_sticker_tab` | Sticker tab, "Badge", thumbnail, the "PSD" handle with grip dots and the caption under it, Edit Sticker, Export Picture... | matches (status "Dragged out Paint 1.png" from step 5) |
| `dragout_save_first` | "Save first" with Cancel and Save... over the sphere with the disc | matches |
| `dragout_psd_toast` | the tab-bar toast; "Link: linked", "Editor: Photoshop" | matches (Badge_k3.psd, Edit in Photoshop, Send to Photoshop; status "Dragged out Badge_k3.psd: Ctrl+S in Photoshop updates the sticker") |

- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\p11check\sweep\`): 21 / 21 `"ok": true`.
  Against P11's sweep: **217 / 217 the same** (`compare_p11.txt`), so the `stamp_picture` split changed no export, no
  saved project and no shot. Against the P0 baseline: 214 the same, the two known `side_tabs` small shots (five tabs
  since P6b) and `shot_tab_sticker.png` (new since P6b), as in P10a / P10b / P11 (`compare_p0.txt`).

## Still owed (not verifiable here)

- Hand tests 8, 9, 10, 11 and 23 (HOW_TO_TEST "Drag, drop, copy, paste" 20-27): real drags into Explorer, PhotoCraft
  v0.2.0 and the latest, Photoshop's picture and tab bar, the PSD handle followed by Ctrl+S, Esc, the pen; whether
  `DoDragDrop`'s modal loop and eframe's frames get along on the real window.
- Help text for drag out (P13).

Programs started: cargo, Python (`-E -P`), and the WTE release exe only with `--screenshot ... --scenario ...`
(off-screen). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`.
