# Check note: P10a - WTE's own Windows drop target

Checker, 2026-11-27, branch `stickers` at da4882e (the P10a build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P10a", 6.1 and 6.2 (the sections it cites), 13.9 (the all-zero-alpha rule the builder pulled
forward) and the user's decisions. Verdict: **pass after fixes** (no blocker, no major; five minors fixed, three with new tests and one a test flake,
the rest listed at the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,850 passed / 1 failed (WA20 only) / 65 ignored (app 691 / 0 / 24, core
  1,102 / 1 / 41, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\p10a_check\test.log`).
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: up to date.
- With that exe (off-screen, `--screenshot ... --scenario ...` only, no focus taken; `target\sticker\p10a_check\runs\`):
  `sticker_drop`, `sticker_view`, and with `WTE_OLE=1`: `sticker_drop`, `smoke`, `default`, `pen`, `companion`. All
  `"ok": true`; every `WTE_OLE=1` log has "ole: WTE's drop target is registered (winit's revoked: yes)"; the six
  `sticker_drop` shots are byte-identical with and without `WTE_OLE=1`.
- Every `sticker_drop` shot looked at:

| Shot | Seen |
|---|---|
| `drop_hover_picture` | the 3D half framed in the accent colour, a ring at the pointer on the sphere, "Drop to place the picture here" beside it; the Brush stays the tool (a hover changes nothing) |
| `drop_model` | the red picture centred on the ring's point (ring ~(302, 395), stamp centre ~(299, 396)), handles of the Transform tool, the UV copy in the 2D half, row "Dropped picture" selected, "Undo: Add Stamp" (one entry keeps its own label) |
| `drop_three` | one more stamp "one" placed, yellow (Badge, the `.wtes`) and blue (the 2D stamp) still there, toast "2 more added to the tray", "Undo: Drop" |
| `drop_canvas` | Sticker view, layer "badge" (24 x 24) over the 30 x 30 picture, at the drop point, "Undo: Add Picture Layer" |
| `drop_hover_project` | the full "Drop to open" overlay with the hint line |
| final | the start state after the undos (only Paint 1 / Base) |

## Plan 7 "P10a" item by item

| Item | Found |
|---|---|
| Files `ole/{mod,formats,com,drop_target,data_object}.rs`, `dnd.rs`, `app.rs`, `selftest/mod.rs` | present |
| 6.1 rules: `catch_unwind` in every callback; leaked box with an atomic count; `QueryInterface` = `IUnknown` + own, else `E_NOINTERFACE` | yes (see "Unsafe review" for what the app's panic hook does to the catch) |
| Callbacks only queue; `Drop` copies before return; caps 64 MiB / 256 MiB / 64; 1-MiB `IStream` chunks; `skipped` + one toast; no decoding in the callback | yes; decoding on a worker thread |
| `install`: interactive runs; self-test only with `WTE_OLE=1`, what it queues is dropped; `RevokeDragDrop` then `RegisterDragDrop`; failure logged, toast once; whether winit's handler can be put back: documented (it cannot) | yes |
| The overlay from `Hover` / `Leave`, egui's path kept where the target is not installed; one function hides the difference; old drop tests against both sources | yes (`handle_events` reads both; `the_overlay_is_the_same_from_both_sources`; the old egui tests unchanged) |
| 6.2 formats best first, `DROPEFFECT_COPY` / `NONE`, `Hover { pos, kinds }`, `ScreenToClient` / ppp | yes |
| 6.2 table, several pictures, no project open | yes, with the builder's deviations (accepted; see below) |
| Tests: codecs, layout asserts, the drop target in-process with the poisoning fake, `ReleaseStgMedium` on every path, caps, a caught panic, the WTE-origin marker, the table row by row, the overlay from both sources | yes |
| Scenario `sticker_drop` (3D +-2, 2D, canvas, row, miss, `.wtes`, three at once, `.wtep` hover) and again with `WTE_OLE=1` | yes, all eight parts |
| Docs: ARCHITECTURE 2.2, 3.3, 8.1, 8.3, 8.9, 8.11, 8.13, 8.17, 9.2; HANDOFF 4.2 `WTE_OLE`; HOW_TO_TEST "Drag, drop, copy, paste" | present |

## Findings and fixes

1. **Minor, fixed: a DIB mask with gaps overflowed** (`formats.rs` `Channel::get`). The channel's width is the mask's
   lowest run of bits, but the value was taken from every bit of the mask, so e.g. a red mask `0xFF00FF01` (a 1-bit
   run) gave values up to `0xFF00FF01` and `raw * 255` overflowed: a panic in a debug build ("attempt to multiply with
   overflow", reproduced by the new test before the fix), wrong colours in the release build. Another program's bytes
   decide the masks. Now only the run's bits count. Test `check_p10a_masks_with_gaps_never_overflow` (the gapped
   picture's exact pixels, and every single-run and gapped mask at every shift).
2. **Minor, fixed: a drop could land in a document opened while it was being prepared.** The spot (a stamp id, a
   canvas or view point) is found when the drop arrives; the worker may take a while (a large PSD); if another
   project was opened meanwhile, `apply` used the old spot on the new document (stamp ids are counted per document,
   so a row drop could change an unrelated stamp). `Prepared::generation` now carries `Session::scene_generation`
   (the id Save uses for the same purpose); a mismatch places nothing and says "The drop was not placed: another
   document was opened meanwhile". Test `check_p10a_a_drop_prepared_for_another_document_is_not_placed`.
3. **Minor, fixed: a virtual file in an `HGLOBAL` kept the block's slack.** `read_global` copies the whole
   `GlobalSize`, which may be larger than the data; for `FileContents` the descriptor's size (when given) is the
   file's, so the bytes are now cut to it (pictures and PSDs no longer get trailing bytes). Test
   `check_p10a_a_virtual_file_in_a_larger_hglobal_is_cut_to_its_size`.
4. **Minor, fixed: `GetData` success codes other than `S_OK` leaked the medium.** `get_bytes` treated only `S_OK` as
   success; a filled medium with another success code was neither read nor released. Now any success code reads and
   releases (`ReleaseStgMedium` on an empty `TYMED_NULL` medium does nothing).
5. **Minor, fixed (test flake): the two queue tests of `ole::tests` raced.** Both use the process-wide queue; one
   drains everything and pushes 266 events (past the 256 cap), so under the parallel test runner it could take or
   push out the other's three events. They now run one at a time (`queue_lock`).

Also re-checked and found right: `prepare` was split into `prepare` + `prepare_item` (no behaviour change).

## Unsafe review (Win32 contracts)

- `get_bytes`: `ReleaseStgMedium` runs on every path where `GetData` filled the medium (now: any success code),
  whatever the `tymed`; `pUnkForRelease` is left to `ReleaseStgMedium` (the tests' owner frees and poisons, the
  untracked medium is `GlobalFree`d by it). `read_global` reads between `GlobalLock` / `GlobalUnlock`, `GlobalSize`
  bytes, cap checked first; `read_stream` never trusts `pcbRead` beyond the buffer (`min(CHUNK)`), stops on a failure,
  0 bytes, or past the cap.
- The data object is used only during the call (no `AddRef` kept), as `IDropTarget` requires. `DragOver` uses the
  kinds of `DragEnter` (no data object there, as the interface says).
- Reference counts: `create` = 1; `QueryInterface` adds one; the window's target keeps its own reference for the
  process, so it is never freed while Windows may still call it; `TestDrag` releases both its
  objects in `Drop`. The tests check the counts return to their start.
- `DataObject::GetData` hands out a fresh `HGLOBAL` (caller frees through `ReleaseStgMedium`) or a fresh stream with one
  reference; `SHCreateStdEnumFmtEtc` copies the array before `list` is dropped. `global_from` frees the block when the
  lock fails.
- `catch_unwind`: in the tests a panic becomes `E_UNEXPECTED` (tested). **In the app, `main.rs`'s panic hook ends the
  process (error box, exit 101) before any unwinding starts**, so nothing can unwind into OLE either way; the catch is
  a second line. Written into ARCHITECTURE 8.17 (the builder's text implied the app survives such a panic).
- `POINTL` by value is 8 bytes in a register on x64, as declared; `ScreenToClient` gets a `POINT`-compatible
  `PointL`. Format ids from `RegisterClipboardFormatW` (0xC000-0xFFFF) fit `u16`; a failed registration gives 0 and
  `has` refuses format 0.

## Rules

- No new crate; `Cargo.toml` / `Cargo.lock` unchanged. No real clipboard, registry write, `%LOCALAPPDATA%` or prefs in
  any test; the tests' data objects are never registered with the OS. Only `--screenshot ... --scenario ...` runs of
  the WTE exe were started; no PhotoCraft, no Photoshop.
- Old projects: nothing of the project format, the renderer or the export changed (the history group is new API
  only). The builder's identity sweep (21 / 21, P0 and P9b) stands; the check's fixes touch only the drop path.

## Accepted deviations (builder's list) and one more

- All of the builder's deviations are reasonable and documented in `build-P10a.md`. One more, not in that list: a drop
  on the model uses the Stamp tool's own surface mode (Project or Wrap) and size, "exactly as a click"; the plan's
  table says "Project mode". Left as built (a drop is the click); a user with Wrap chosen gets a wrapped stamp.

## Left open (minor, not fixed)

- `DragEnter` copies a whole `WTE.Sticker.v1` block (up to 64 MiB) to read its 12-byte header (the process id). Only
  another WTE offers that format, and drag-out comes in P11: worth reading only the header there.
- `read_stream` does not seek a `FileContents` stream to its start; every known source hands it out at 0. A
  `Seek(0, SET)` (failure ignored) could be added when a real source needs it (hand test 7).
- A sticker block over the cap makes `read_payload` give up (`skipped: 1`) instead of trying `CF_HDROP` beside it.
- The view drop uses the camera at the time it is applied (after the worker); turning the view in between moves the
  landing point. The worker is quick for pictures.
- The 64-MiB-per-item cap text says "more than 64 MB" (MiB): wording only.
- Real Explorer / browser / PhotoCraft drags, pen through OLE, and the elevated case stay hand test 7.

## After the fixes

- `cargo test --workspace --no-fail-fast`: 1,853 passed / 1 failed (WA20 only) / 65 ignored (app 694 / 0 / 24: three
  new checker tests; core 1,102 / 1 / 41; integration 15 + 9 + 8 + 25). Logs `target\sticker\p10a_check\test2.log`,
  `test3_app.log` (the app crate again after the last `get_bytes` edit).
- `cargo clippy --workspace --all-targets`: 0 warnings. Release build OK: 32,722,432 bytes, SHA-256
  `2a41ee9dbb01ab121f3b4ee33a8bd9ebe78036d1b027061fc9b2110afac60b55`.
- With that exe (`target\sticker\p10a_check\runs_fixed\`): `sticker_drop`, and with `WTE_OLE=1` `sticker_drop`,
  `smoke`, `default`, `pen`, `companion`: all `"ok": true`, each `WTE_OLE=1` log has the registration line; the six
  `sticker_drop` shots are byte-identical to the builder exe's and with / without `WTE_OLE=1`.
