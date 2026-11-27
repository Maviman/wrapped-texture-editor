# Build note: P10a - Windows drop target (STICKER_PLAN 7 "P10a", 6.1 without the subclass, 6.2)

Builder, 2026-11-27, branch `stickers` (from 511157a, the P9b check; plan revision 3). Windows FFI (kind F). WTE now
registers its own OLE `IDropTarget` on the window in place of winit's (which reads `CF_HDROP` only and drops the
point), so a picture lands where it is let go, and picture data without a file (a browser's image, a virtual file)
arrives. Hand-written `raw-dylib` FFI, no new crate, `Cargo.toml` / `Cargo.lock` unchanged. The user's workflow is not
touched: a drop never writes or reads a link file; a drop on a linked sticker's stamp changes the sticker like any
other edit in WTE (the link's rules of 5.5 apply as for an edit in the Sticker view).

## What was built

| Item (plan 6.1, 6.2, 7 "P10a") | Where | Notes |
|---|---|---|
| Pure codecs | new `ole/formats.rs` | `dib_to_pixels` (`CF_DIB` / `CF_DIBV5`: 24-bit, 32-bit `BI_RGB`, `BI_BITFIELDS`, `BI_ALPHABITFIELDS`, top-down and bottom-up, masks of any width scaled to 8 bits, the masks some writers repeat after a V5 header, a colour table skipped; the 13.5 rule: an alpha that is zero in every pixel is opaque), `pixels_to_dibv5` (BI_BITFIELDS + alpha mask, sRGB, bottom-up: for P10b / P11), `DROPFILES` build / parse (wide and ANSI), `FILEGROUPDESCRIPTORW` build / parse, `CF_UNICODETEXT` both ways (P10b), `WTE.Sticker.v1` wrap / unwrap ("WTEs", version 1, the process id, the `.wtes` bytes). Every length is checked; nothing indexes past the end |
| Declarations | new `ole/com.rs` | `raw-dylib`: `ole32` (`RegisterDragDrop`, `RevokeDragDrop`, `ReleaseStgMedium`), `shell32` (`SHCreateStdEnumFmtEtc`), `user32` (`RegisterClipboardFormatW`, `ScreenToClient`), `kernel32` (`GlobalAlloc/Lock/Unlock/Size/Free`); `#[repr(C)]` `Guid`, `FormatEtc`, `StgMedium`, `PointL`, the vtables of `IUnknown`, `IDataObject`, `IDropTarget`, `IEnumFORMATETC`, `IStream`; the IIDs; `global_from`, `register_format`, `add_ref` / `release`; who frees what in the module comment |
| WTE's data object | new `ole/data_object.rs` | `DataObject` (a list of `Entry { format, lindex, tymed, bytes }`), `GetData` on `HGLOBAL` (a copy) or `IStream` (`MemStream`, read only, optionally endless), `QueryGetData`, `EnumFormatEtc` (shell32's enumerator), the rest `E_NOTIMPL` / `OLE_E_ADVISENOTSUPPORTED`; the tests' `pUnkForRelease` owner poisons (0xDD) and frees its `HGLOBAL` and counts; the last `Release` poisons the entries. P11 hands it to `DoDragDrop` |
| The drop target | new `ole/drop_target.rs` | `DropTarget` (`Formats` registered once, `Caps` 64 MiB / 256 MiB / 64 items, a `Sink`): `inspect` in `DragEnter` (the WTE-origin marker ignores the whole drag, R3; `CF_HDROP` names, picture data, a virtual file with a picture / PSD / `.wtes` name) -> `Hover`; `DragOver` -> `Hover`; `DragLeave` -> `Leave`; `Drop` -> `read_payload` (best format first, the next one if it cannot be read; copied before the return; `ReleaseStgMedium` on every path after a successful `GetData`) -> `Drop { pos, items, skipped }`. `catch_unwind` around every body; `QueryInterface` answers `IUnknown` and `IDropTarget` |
| Queue, install | new `ole/mod.rs` | `OleEvent { Hover, Leave, Drop }`, `DragKinds`, `DropItem`, `DataKind`; the queue (256 events, a repaint per push); `install(cc)` (`RevokeDragDrop` + `RegisterDragDrop`, the target leaked), `installed()`, `take_registration_lost()`; `TestDrag` (the self-test's drag through a drop target object and WTE's own data object), `kinds_of` |
| `dnd.rs` on both sources | `dnd.rs` (rewritten around the old code), `dnd/tests.rs` | hovers and drops from the OLE queue and from egui's file lists; `overlay_of` (`Open` / `Place` / `Blocked`, the same for both sources), the drop marker (a frame around the target and a sentence beside the pointer, `hover_text`), `spot_at` (a stamp's `LayerRow` inside the visible list, the Sticker tab's thumbnail, the sticker canvas, the 3D / 2D views; a dialog in front, the Preview view and the 3D half of the Sticker view are "elsewhere"), the worker (`prepare`: files read, PNG as is, a DIB made a PNG, `.wtes` and `.psd` parsed), `apply` (the 6.2 table, one undo step per drop, the rest of a drop to the tray). egui's dropped files still go through `plan_drop` and `OpenPath` exactly as before |
| `.wtes` files | `dnd::plan_drop`, `dispatch` (`OpenPath`) | a `.wtes` counts like an image or PSD; `OpenPath(.wtes)` adds it to the tray as the Stamp tool's source (`sticker::library::add`) |
| One undo step per drop | core `history.rs` (`EditGroup`, `Document::begin_edit_group` / `end_edit_group`) | for commands the app runs one by one (an import, then `StampPlaceTool::press`); as `group_edits` (one entry keeps its label, several become "Drop") |
| Sticker helpers | `sticker/mod.rs` (`add_layers_at`), `sticker/canvas.rs` (`layer_point`), `sticker/panels.rs` + `state.rs` (`PanelHit::StickerTabThumb`) | a sticker's layers added above the active layer, centred at a layer-space point, one entry |
| PSD import parts | `link/flows.rs` (`parse_psd`, `psd_sticker`, `psd_as_source`) | `import_psd` split so a PSD read on the worker (or dropped without a file) goes the same way; the toasts and their order are unchanged |
| App wiring | `app.rs` (`ole::install` in interactive runs and with `WTE_OLE=1`; `ole::discard` in a self-test's `raw_input_hook`), `action.rs` (`AppAction::DropReady`), `dispatch.rs` (guarded; `dnd::apply`), `state.rs` (`AppState::drop_ui`) | |
| Self-test | `selftest/mod.rs` | `Step::Drop { payload, target, at }`, `Step::DragHover`, `Step::DragLeave`, `Target::Screen` (screen points), `Active::Dropping` (queue, worker, dispatched actions, bakes), `is_idle` waits for the queue and the worker |
| Scenario | new `selftest/scenarios/sticker_drop.rs` | 68 scenarios |
| Docs | ARCHITECTURE 2.2 (file table), 3.3, 5.x (`begin_edit_group` / `end_edit_group`), 8.1 (the drop facts), 8.3, 8.9, 8.11 (steps and the `sticker_drop` row), 8.13 (pitfall 25), new 8.17, 9.2; HANDOFF 4.1, 4.2 (`WTE_OLE`), 5.4 and the branch table; `dist/HOW_TO_TEST.txt` chapter "Drag, drop, copy, paste" (the drop part) | |

Tests: `ole::formats::tests` (7), `ole::com::tests` (3), `ole::data_object::tests` (1), `ole::drop_target::tests` (4),
`ole::tests` (2), `dnd::tests` (13: the 3 old ones and 10 new), core `k10a_an_edit_group_is_one_undo_step`. What they
cover is listed in ARCHITECTURE 8.17 "Tests"; in short, the plan's list item by item:
- codecs: DIB 24 / 32, DIBV5 with masks, top-down, with and without alpha; DROPFILES with spaces and non-ASCII (and
  ANSI); file group descriptors; malformed inputs never panic (every truncation, 2,000 seeded byte flips, huge sizes,
  `i32::MIN` height, palettes, JPEG compression, a lying count);
- vtable layout asserts (x64 sizes and offsets of the SDK);
- the drop target driven in-process for each format with the fake data object; reference counts back at their start;
  `QueryInterface`; **`Drop` has copied the bytes before it returns** (the data object poisons its memory when released
  and the `pUnkForRelease` owner poisons each `HGLOBAL`; the queued payload is still right) and media released = media
  handed out on every path (also when the drag is refused); the caps (a 65-MiB item skipped with `skipped: 1`, an
  endless `IStream` stopped at the cap, 70 files cut to 64 with 6 skipped); a panic inside a callback caught (a hook per
  target object); the WTE-origin marker ignored, even beside `CF_HDROP`;
- the 6.2 table row by row (`dnd::tests`: the model, the 2D view, a miss, the canvas, rows of image / sticker / shared
  sticker stamps, `.wtes`, several pictures, a picture file elsewhere, an undecodable picture);
- the overlay from `Hover` / `Leave` events and from egui's `hovered_files` gives the same result.

## Deviations (with reasons)

1. **After a drop on the model the Transform tool takes over** (plan 6.2: "the Stamp tool becomes active with it
   selected"). The drop is the Stamp tool's click (`StampPlaceTool::press`), which ends with the new stamp selected and
   the Transform tool, so its handles show at once; a second behaviour for the same placement would be confusing. The
   dropped picture stays the Stamp tool's source, so pressing S and clicking places it again.
2. **A single picture dropped "elsewhere" becomes the Stamp tool's picture without a tray sticker** (plan: "and a tray
   sticker"). Rule 8 of 0.4: a file drop that misses the model behaves as before (`ImportStampImage`), and a kept tray
   sticker would turn a project that uses no new feature into a format-4 project. Further pictures of the same drop do
   go to the tray, as the plan says.
3. **A drop on a stamp's row never asks the shared-picture question** (plan: a `.wtes` "replaces the stamp's sticker
   (asks when shared)"). A drop aims at one stamp, so only that stamp changes: a `.wtes` / `.psd` becomes a new sticker
   shown by that stamp (the shared one stays for the others), and a picture added to a shared sticker detaches it
   first. "Edit all" is still one click away (Edit Sticker). Each is one undo step.
4. **A `.psd` on a row is treated like a `.wtes` (its layers), not "as picture".** An image stamp cannot switch to a
   sticker by `set_stamp_source`; the PSD's layers are what the user made, and the canvas row of the table does the
   same. An image stamp is promoted for it first (same entry).
5. **`Step::Drop` targets**: besides `Target::View` / `StickerCanvas`, `Target::Screen` (screen points) for a Layers row
   or the Sticker tab; and `Step::DragHover` / `Step::DragLeave` for the hover shots. All of them go through a drop
   target object and WTE's own data object (`ole::TestDrag`), so the scenario tests the codecs and `read_payload`
   too, not only the queue.
6. **Any `CF_HDROP` is accepted** (as winit did): an unusable file gets the "Not opened: ..." toast after the drop rather
   than a "no drop" cursor, so a model dropped together with its `.mtl` still opens.
7. **Picture data has no name**: the drop target calls it "Dropped picture.png" (the stamp is named "Dropped picture");
   a virtual file keeps its own name.
8. **`DragQueryFileW` is not used**: `DROPFILES` is parsed in Rust (`formats::parse_dropfiles`), which the tests reach
   without an OS call. `IEnumFORMATETC` comes from `SHCreateStdEnumFmtEtc` (no own enumerator). `IDropSource` is left
   to P11 (an unused declaration would be dead code).
9. **The worker is a plain thread with a channel** (`DropUi::jobs`), not a `Jobs` job: a drop never blocks the UI
   (`modal` stays off), and the self-test's idle check waits for it. Decoding of the picture itself (`add_image_blob`)
   stays on the UI thread as for `ImportStampImage`.
10. **The "texel +-2" check of the scenario** compares the projected stamp's origin with the drop point's hit (within
    1e-3 of the model's diagonal) and logs the texel: a projected decal has no single texel of its own. The 2D drop is
    checked in texels (+-2).
11. **winit's target cannot be put back** when `RegisterDragDrop` fails after the revoke (its object is not reachable
    through winit's API); the failure is logged and toasted once. `RegisterDragDrop` fails only for a wrong thread or no
    memory; it worked in every run here.
12. **The all-zero-alpha rule of 13.5** (a P10b row) is in the codec now, because a dropped DIB needs it already; P10b
    adds the clipboard side.
13. A new core API (`begin_edit_group` / `end_edit_group`) was needed for "one undo step per drop": the drop's
    commands need the app's state between them, which `group_edits` (one closure over the document) cannot give.

## Numbers

- `cargo test --workspace --no-fail-fast`: **1,850 passed, 1 failed (WA20, the known one), 65 ignored**
  (`target\sticker\p10a\test.log`; `wte_app` 691 / 0 / 24, `wte_core` lib 1,102 / 1 / 41, integration 15 + 9 + 8 + 25).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 32,721,408 bytes, SHA-256
  `e92c3132daaaf6bf69c1e844d5cdc70e5f1b64bed9aee2c6af2e1e203894ffe7`.
- Scenarios with that exe (`tools/objtabs/sweep.sh`, off-screen, no focus taken; `target\sticker\p10a\runs\` and
  `runs_ole\`), all `"ok": true`: `sticker_drop` (3 s, 141 frames), `sticker_view`, `sticker_link` (28 s); with
  `WTE_OLE=1`: `sticker_drop`, `smoke`, `default`, `pen`, `companion`. Each `WTE_OLE=1` log says "ole: WTE's drop target
  is registered (winit's revoked: yes)": the registration works on the real (off-screen) window. The five
  `sticker_drop` shots are **pixel-identical** with and without `WTE_OLE=1`.
- Shots of `sticker_drop`, each looked at against its `expect`: `drop_hover_picture` (the 3D half framed in blue, the
  ring at the pointer, "Drop to place the picture here"; matches), `drop_model` (the red picture on the sphere with the
  gizmo, the same on the texture, the row "Dropped picture" selected, "Undo: Add Stamp"; matches), `drop_three` (the
  red first picture, the yellow Badge, the new "one" with the gizmo, the blue UV stamp only in the 2D half, the toast
  "2 more added to the tray", "Undo: Drop"; matches), `drop_canvas` (the Sticker view zoomed onto the 30 x 30 purple
  picture with the green 24-px square right of and above its centre, two layers "badge" active, the sphere showing
  both, "Undo: Add Picture Layer"; matches after the expect text was made exact), `drop_hover_project` (the whole
  window darkened, "Drop to open" and the file kinds; matches); the final shot shows the start again after six undos.
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\p10a\sweep\`): 21 / 21 `"ok": true`.
  `compare_baseline.py` against the P0 baseline: 214 the same, the two known `side_tabs` small shots (five tabs since
  P6b) and `shot_tab_sticker.png` (new since P6b), as in P9a / P9b; against the P9b check's sweep
  (`target\sticker\check-p9b\sweep\`): **217 / 217 the same**. No old project or flow changed.
- Programs started: cargo, Python (`-E -P`), and the WTE release exe only with `--screenshot ... --scenario ...`
  (off-screen, no focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No test registers a
  drop target on a window or touches the clipboard; the tests' OS calls are `GlobalAlloc` & co., `ReleaseStgMedium`,
  `SHCreateStdEnumFmtEtc` and `RegisterClipboardFormatW` (a format name, not the clipboard).

## For the checker

- `unsafe` to read against the Win32 contracts: `drop_target.rs` (`get_bytes`, `read_global`, `read_stream`, the
  callbacks), `data_object.rs` (`GetData`, the owner's release), `com.rs` (`global_from`), `mod.rs` (`TestDrag`).
- Hand test owed (8.7 hand test 7, HOW_TO_TEST "Drag, drop, copy, paste" 1-10): a real Explorer and browser drag; the
  pen through OLE is not part of P10a (the drop target does not depend on the button state).
