# Windows drag and drop and linking between WTE and PhotoCraft

Topic: how WTE (Wrapped Texture Editor) and PhotoCraft can exchange images/layers on Windows by drag and drop,
clipboard, linked files or a control channel, under WTE's "no new crates" rule (FFI through `raw-dylib`, as
`crates/wte_app/src/pen/win.rs` already does). Written 2026-11-25.

Sources read:
- WTE: `G:\software_projects\texture_wrapper_ap_cmp` (branch `companion`), `Cargo.lock`, `crates/wte_app/src/{dnd.rs,main.rs,app.rs,pen/win.rs,pen/mod.rs}`.
- winit / egui / eframe as WTE builds them: `REG\winit-0.30.13`, `REG\egui-winit-0.37.0`, `REG\egui-0.37.0`, `REG\eframe-0.37.0`
  (`REG` = `G:\Claude_Resources\rust\cargo\registry\src\index.crates.io-1949cf8c6b5b557f`).
- PhotoCraft: `G:\Claude_Resources\ref\photocraft` at tag `v0.2.0` (the user's installed build) and `HEAD` (b5c7620, latest main).
- PhotoCraft CLI v0.2.0 binary, run headless on a 32x16 PNG in `G:\Claude_Resources\ref\scratch\windnd\`.

Tags: **VERIFIED** = read in code or ran; **INFERRED** = Win32/COM knowledge or reasoning, not checked on this machine.

Side note (VERIFIED): the first `git grep` on the reference clone printed "Auto packing the repository" (git's automatic
gc ran once); later commands used `-c gc.auto=0 -c maintenance.auto=false` and `GIT_NO_LAZY_FETCH=1`. The clone is a
blobless partial clone (`remote.origin.partialclonefilter = blob:none`) and some greps that touch `HEAD` print
"commit-graph ... not in the object database"; trees and blobs of `v0.2.0` and `HEAD` were readable.

---

## 1. Versions in play

| Item | WTE (`_cmp` Cargo.lock) | PhotoCraft v0.2.0 (Cargo.lock) | PhotoCraft HEAD (Cargo.lock) |
|---|---|---|---|
| winit | 0.30.13, crates.io | 0.30.13, crates.io | **fork** `storytold/winit` rev a517b38 (`[patch.crates-io]`, Cargo.toml:377) |
| egui / egui-winit / eframe | 0.37.0, crates.io | 0.36.2, crates.io | **fork** `storytold/egui` rev b15ffe9 (Cargo.toml:363-376) |
| clipboard | none (no arboard, no clipboard-win in the lock) | arboard 3.6.1 (+ clipboard-win 5.4.1) | arboard 3.7.0 |
| windows-sys | 0.52.0 (via winit), 0.61.2 (eframe, egui-winit, rfd, zip, anstyle) | 0.52 / 0.59 / 0.60 / 0.61 | same |
| windows | 0.62.2 (via wgpu-hal, gpu-allocator, accesskit_windows) | 0.62.2 | 0.62.2 |

All VERIFIED (Cargo.lock / `cargo tree --locked --offline -i ...`). PhotoCraft HEAD's Cargo.toml comment (line 350):
"Keep the complete egui family and native event bridge on reviewed fork commits." The forks are not cloned here, so
what HEAD's winit fork changed in drag and drop is **unknown** (no network).

---

## 2. winit 0.30.13 on Windows: the drop target it installs

### 2.1 Registration (VERIFIED)
- `REG\winit-0.30.13\src\platform_impl\windows\window.rs:1166-1197` (`create_window_data`): only when
  `platform_specific.drag_and_drop` is true (default true, `mod.rs:49`; set by `with_drag_and_drop`,
  `platform/windows.rs:566`), winit calls `OleInitialize(null)` on the window's thread (panics on `OLE_E_WRONGCOMPOBJ`
  or `RPC_E_CHANGED_MODE`, i.e. if the thread was already initialised as MTA), creates a `FileDropHandler` and
  `assert_eq!(RegisterDragDrop(hwnd, handler), S_OK)`.
- `event_loop.rs:1260-1262`: `WM_DESTROY` calls `RevokeDragDrop(window)`.
- Doc (`platform/windows.rs:490-497`): drag and drop "will interfere with other crates that use multi-threaded COM ... on
  the same thread".
- egui maps `ViewportBuilder::with_drag_and_drop` to it: `REG\egui-winit-0.37.0\src\lib.rs:2444-2445`. WTE sets it:
  `crates/wte_app/src/main.rs:98` `.with_drag_and_drop(true)`. PhotoCraft too: v0.2.0 `apps/photocraft/src/main.rs:110`,
  HEAD `apps/photocraft/src/main.rs:192`.

### 2.2 What it reports (VERIFIED, `drop_handler.rs`)
- Only `CF_HDROP` is ever asked for (`iterate_filenames`, lines 161-210: `FORMATETC{cfFormat: CF_HDROP, tymed: TYMED_HGLOBAL}`
  then `DragQueryFileW`). Any data object without `CF_HDROP` gets `DV_E_FORMATETC` and is ignored with a debug log
  (line 201-205): **no in-memory images (CF_DIBV5 / "PNG"), no virtual files (`FileGroupDescriptorW` / `FileContents`),
  no text/URLs** reach the app.
- Events: `HoveredFile(PathBuf)` per file on `DragEnter` (79-104), `HoveredFileCancelled` on `DragLeave` (120-131),
  `DroppedFile(PathBuf)` per file on `Drop` (133-155). `DragOver` only repeats the cursor effect (106-118).
- The drop **point is ignored**: `_pt` in `DragEnter`, `DragOver` and `Drop`, `_grfKeyState` too. The effect is
  always `DROPEFFECT_COPY` when there is a CF_HDROP, regardless of what the source allows.
- `QueryInterface` is `unimplemented!()` (line 53-61): it would panic across FFI if OLE ever called it (it does not in
  practice for a registered drop target).
- egui-winit turns them into `RawInput::hovered_files` (path only, `mime` empty) and `RawInput::dropped_files`
  (`Arc<NativeFile>` built from the path) (`REG\egui-winit-0.37.0\src\lib.rs:513-535`). egui 0.37's `DroppedFile` is a
  trait with `path()` and a native `bytes()` that reads the file (`REG\egui-0.37.0\src\data\input\dropped_file.rs:11-31`).
- INFERRED: during an OLE drag started in another process the target window gets no `WM_MOUSEMOVE` (the source's modal
  loop owns the mouse), so egui's pointer position is stale while files hover; combined with the ignored `pt`, a winit
  app **cannot know where** in its window a file was dropped. (PhotoCraft HEAD does know: `DropTo::Canvas { at }`,
  `crates/ui-egui/src/file_open.rs:28-40`, which suggests its winit/egui fork adds a position; not checkable here.)

### 2.3 WTE today (VERIFIED)
`crates/wte_app/src/dnd.rs:11-59`: full-window "Drop to open" overlay while `hovered_files` is non-empty; dropped paths go
through `plan_drop` (one `.wtep` or model per drop, images become stamp sources when no document is in the drop,
everything else ignored with a toast). Image extensions: png, jpg, jpeg, tga, bmp, tif, tiff, webp, dds (`dnd.rs:101`).
So **Explorer -> WTE already works** for files, but not for in-memory images and not at a drop position.

### 2.4 Replacing or extending the drop target (INFERRED, mechanism verified against the code above)
A window has exactly one registered `IDropTarget`. Two ways for WTE to own it:
1. Keep `with_drag_and_drop(true)` (winit has then done `OleInitialize` on the UI thread, which WTE needs anyway for
   a drag source), and after the window exists call `RevokeDragDrop(hwnd)` + `RegisterDragDrop(hwnd, wte_target)` from
   `WteApp::new` (same place `pen::install` gets the HWND: `crates/wte_app/src/pen/mod.rs:264-279`,
   `cc.window_handle()` -> `RawWindowHandle::Win32`). winit's handler object stays allocated until the window drops;
   winit's `RevokeDragDrop` on `WM_DESTROY` then revokes WTE's target, which is fine. WTE's object must live for the
   process (static / leaked box), like the pen subclass state.
2. `with_drag_and_drop(false)` and call `OleInitialize(null)` + `RegisterDragDrop` itself. Equivalent; option 1 keeps
   the self-test path (which clears `hovered_files`/`dropped_files`, `selftest/mod.rs:286-287`) unchanged if the swap is
   skipped in self-test runs.

WTE's own `IDropTarget` can then:
- read `CF_HDROP` (as now), the registered `"PNG"` format (`RegisterClipboardFormatW(L"PNG")`, TYMED_HGLOBAL or ISTREAM),
  `CF_DIBV5` / `CF_DIB`, and virtual files (`CFSTR_FILEDESCRIPTORW` + `CFSTR_FILECONTENTS`, TYMED_ISTREAM or HGLOBAL:
  browsers, Outlook, zip folders) and a private WTE format (e.g. `"WTE.Layer.v1"` carrying a link id);
- convert `pt` (screen physical pixels) with `ScreenToClient` and divide by `pixels_per_point` to place a stamp
  exactly where it was dropped (3D view hit test);
- honour `grfKeyState` (Ctrl/Shift/Alt modifiers to choose "new stamp" vs "replace layer").
Hand-off to the UI: the callbacks run on the UI thread (STA; OLE dispatches cross-process calls through the thread's
message loop) from inside winit's `DispatchMessageW`, i.e. outside eframe's frame; they must only push to a static
`Mutex<VecDeque<..>>` and call `ctx.request_repaint()` (egui `Context` is `Send + Sync`), then `raw_input_hook`
(`crates/wte_app/src/app.rs:297`) or `dnd::handle` drains the queue. Every callback body inside `catch_unwind`, as
`pen/win.rs` does. Hand-written vtable: winit's own `drop_handler.rs` (~240 lines) plus `definitions.rs` vtables is a
ready model; WTE needs `ole32` (`RegisterDragDrop`, `RevokeDragDrop`, `ReleaseStgMedium`), `shell32` (`DragQueryFileW`),
`user32` (`ScreenToClient`, `RegisterClipboardFormatW`), `kernel32` (`GlobalLock`/`GlobalUnlock`/`GlobalSize`) via
`raw-dylib`. Optional: `IDropTargetHelper` (`CoCreateInstance(CLSID_DragDropHelper)`) to show the source's drag image
over WTE.

---

## 3. WTE as an OLE drag SOURCE

### 3.1 Threading and blocking (VERIFIED code, INFERRED consequences)
- `DoDragDrop` (ole32) is synchronous and modal: it runs its own message loop on the calling thread until the drop or
  cancel, and must be called on an OLE-initialised STA thread while the mouse button is down (INFERRED, Win32 contract).
  The UI thread qualifies because winit already called `OleInitialize` (2.1).
- **It must not be called from inside `eframe::App::update` (or anything eframe calls).** winit's Windows runner
  (`REG\winit-0.30.13\src\platform_impl\windows\event_loop\runner.rs:208-238`): a `RedrawRequested` produced while the
  app's handler is running calls `call_event_handler`, which does
  `self.event_handler.take().expect("either event handler is re-entrant (likely) ...")`. A `WM_PAINT` dispatched by
  DoDragDrop's nested loop while `update` is on the stack would hit that `expect` (panic in the event loop); other
  events would be buffered and the window would not repaint during the drag. VERIFIED (code); the panic path is INFERRED
  (not run).
- Safe pattern (INFERRED): `update` detects "drag left the layer row/viewport with the button held" and does
  `PostMessageW(hwnd, WM_APP + n, ..)`; a window-procedure subclass (WTE already has one: `pen/win.rs:215-240`, chain
  through `CallWindowProcW`) handles that message by calling `DoDragDrop`/`SHDoDragDrop`. That runs from winit's
  `DispatchMessageW`, outside the event handler, so nested `WM_PAINT`s are delivered normally (`send_event` ->
  `call_event_handler` with the handler free), the app keeps rendering during the drag (like the modal resize loop).
  The mouse button is still down a frame later in practice; if released, the default drop source just drops/cancels.
- After the drag: OLE took mouse capture, winit sees `WM_CAPTURECHANGED` and resets its `capture_count` to 0
  (`event_loop.rs:1927-1935`, VERIFIED) but **no `WM_LBUTTONUP` reaches winit**, so egui still believes the primary
  button is down (INFERRED). WTE must inject `egui::Event::PointerButton { pressed: false, .. }` (or `PointerGone`) via
  `raw_input_hook` after DoDragDrop returns, otherwise the next move "drags" whatever is under the pointer.
- Pen (RISK, INFERRED): WTE's pen path feeds egui from `WM_POINTER*` messages that winit never passes to `DefWindowProc`
  (`pen/win.rs:12-13`), so Windows does not promote pen contact to mouse messages for this window. The default drop
  source's `QueryContinueDrag` polls the left-button key state; with a pen it may see "button up" immediately and drop
  at once, or OLE may not track the pen at all. Must be tested on a real tablet (user's Galaxy/S Pen via the companion is
  a different path entirely: companion input is injected in-process and has no OS button state, so drag-out from the
  tablet companion is **not possible** by OLE).

### 3.2 Data object without hand-writing IDataObject (INFERRED, documented shell32 APIs)
- `SHCreateDataObject(pidlFolder, cidl, apidl, null, IID_IDataObject, &pdo)` (shell32, Vista+) returns the shell's own
  `IDataObject`. Built from the PIDL of a real file (`SHParseDisplayName(path)` + `ILFindLastID` / `SHCreateDataObject`
  with the parent folder PIDL), it already offers `CF_HDROP`, `Shell IDList Array`, etc. Extra formats can be added with
  its `SetData(&FORMATETC{"PNG", TYMED_HGLOBAL}, &STGMEDIUM, fRelease=TRUE)` (vtable slot 7) and `CF_DIBV5` likewise.
- `SHDoDragDrop(hwnd, pdo, pdsrc = null, DROPEFFECT_COPY, &effect)` (shell32) accepts a null `IDropSource` (the shell
  supplies a default one that follows the mouse buttons and Esc) and sets up the drag image helper. Result: **WTE only
  calls COM methods by vtable offset; it implements no COM object at all** for the drag source. If a custom
  `IDropSource` is wanted (e.g. pen-aware `QueryContinueDrag`), it is 3+2 methods (~80 lines).
- The alternative, a hand-written `IDataObject` + `IEnumFORMATETC` (`SHCreateStdEnumFmtEtc` from shell32 can supply the
  enumerator), is ~400-600 lines of unsafe vtable code.

### 3.3 Formats to offer (INFERRED)
- `CF_HDROP` of a **real file on disk**: the only format that both PhotoCraft (any version, 4) and Explorer accept.
  Explorer copies it (`DROPEFFECT_COPY`; never offer MOVE for a temp file). The file must outlive the drop: Explorer and
  PhotoCraft read it after `DoDragDrop` returns (PhotoCraft v0.2.0 reads it in the next frame, 4.1). Keep the files in a
  WTE folder (e.g. `%LOCALAPPDATA%\WrappedTextureEditor\links\`) rather than deleting them on return.
- `"PNG"` + `CF_DIBV5` (premultiplied rules: DIBV5 with BI_BITFIELDS + alpha mask; PNG is the lossless choice): for
  Office, browsers, Krita/GIMP and any app that takes in-memory images. Not used by PhotoCraft's drop path.
- Private `"WTE.Layer.v1"` (UTF-8 JSON: project path, layer id, link file path) for WTE -> WTE drops between windows /
  instances.
- Avoid offering virtual files only: winit-based targets (PhotoCraft) ignore them.

### 3.4 Dependency question (VERIFIED tree, decision for the user)
- `windows` 0.62.2 is already in WTE's lock with `Win32_System_Com`, `Win32_System_Ole` enabled (via wgpu-hal /
  accesskit_windows), and `windows-implement` 0.60.2 / `windows-interface` 0.59.3 (the `#[implement]` COM macros) are in
  the lock too. `windows-sys` 0.52.0 (via winit) has `Win32_System_Ole`, `Win32_System_Com`, `Win32_UI_Shell` enabled;
  0.61.2 has `Win32_System_Com`, `Win32_UI_Shell`.
- Using either directly = a new **direct** dependency of `wte_app` (no new crate in the lock, possibly extra features
  such as `Win32_UI_Shell`, `Win32_System_Memory`, `Win32_System_SystemServices` for `windows`). Under the round-4 rule
  this still needs the user's OK. `windows-sys` gives only function/struct declarations (COM interfaces are opaque
  pointers, vtables still by hand), so it saves little over `raw-dylib`. `windows` 0.62 with `#[implement]` would remove
  the hand-written vtables (INFERRED: the macro is re-exported through `windows-core`; needs a feature check).
- Without any of them: `raw-dylib` imports from `ole32`, `shell32`, `user32`, `kernel32` (pattern of `pen/win.rs:97`,
  `gpu_check.rs:181`, `companion/bt_win.rs:98`) plus `#[repr(C)]` vtables copied from the SDK headers. Feasible; winit's
  `definitions.rs` shows the exact `IDataObjectVtbl` / `IDropTargetVtbl` layouts.

---

## 4. What PhotoCraft accepts and offers

### 4.1 PhotoCraft v0.2.0 (the user's installed build)
- Drop target (VERIFIED): stock winit 0.30.13 + egui 0.36.2, so CF_HDROP paths only (section 2.2).
  `crates/ui-egui/src/lib.rs:664` calls `open_dropped(ctx.input(|i| i.raw.dropped_files.clone()))` every frame;
  `crates/ui-egui/src/file_open.rs:76-89`: each dropped file is read (`read_dropped` -> `DroppedFile::bytes`, `lib.rs:741`)
  and **opened as a new document** with its absolute path (`open_file(path, bytes)`), like File > Open. No place-as-layer
  on drop, no drop position, no in-memory images.
- Document path after a drop = the dropped file's path (VERIFIED test `file_open/tests.rs:64-78` for the open dialog;
  drop uses the same `open_file`). File > Save writes in place only for `psd`, `psb`, `pcraft`
  (`crates/ui-egui/src/menus.rs:220-226, 405-408`); a dropped PNG goes through Save As (test
  `pcraft_documents_save_in_place_but_flat_files_ask`, `file_open/tests.rs:80-92`). So: **if WTE drops/opens a `.psd`
  or `.pcraft`, Ctrl+S in PhotoCraft writes straight back to WTE's file** — the basis of a link (section 5.1).
- Drag source (VERIFIED by absence): no `DoDragDrop`, `IDataObject`, `RegisterDragDrop` or any OLE code anywhere in the
  v0.2.0 tree (`git grep` over all `*.rs`). **PhotoCraft cannot drag a layer or image out to another app.**
- Clipboard (VERIFIED): arboard 3.6.1. `apps/photocraft/src/services.rs:135-142`: `clipboard_set_image` / `get_image`
  with RGBA8. `crates/ui-egui/src/lib.rs:965-998`: copy mirrors the session clipboard to the OS clipboard;
  `import_os_clipboard` takes an OS image that PhotoCraft did not put there ("so Ctrl+V pastes screenshots and images
  copied in other apps"). INFERRED (arboard source not on this machine): on Windows arboard 3.x writes `CF_DIBV5` and a
  registered `"PNG"` format, and reads PNG first then DIB(V5). So **WTE <-> PhotoCraft copy/paste works if WTE writes
  `"PNG"` + `CF_DIBV5` and reads them**.
- Command line (VERIFIED, `apps/photocraft/src/main.rs:1-9, 40-60`): `photocraft [--control <port>] [--control-token ...]
  [--automation-read-root <dir>] [--automation-write-root <dir>] [files...]`; files open as documents. No
  single-instance in v0.2.0 (no `single_instance.rs`; HEAD has one): every launch is a new process/window.
- Control channel (VERIFIED, `crates/ui-egui/src/control.rs:1-22, 354-374`): localhost JSON-lines server only when
  started with `--control <port>` (or `PHOTOCRAFT_CONTROL_PORT`), first line must authenticate with a 64-hex token;
  methods `engine.execute {command, params}` (any engine command, e.g. `file.placeLinked`,
  `layer.smartObjects.updateModifiedContent`), `app.open {path}` / `app.save {path}` (relative to the automation roots),
  `ui.*` (inspect, synthetic input, screenshot — `ui.screenshot`/`ui.focus` raise the window: focus risk). WTE cannot
  attach to a PhotoCraft the user started normally; it would have to launch PhotoCraft itself with `--control`.
- Linked smart objects (VERIFIED): `file.placeLinked` exists (`crates/engine/src/file_cmds.rs:1040`), and
  `layer.smartObjects.updateModifiedContent` / `updateAllModifiedContent` (`crates/engine/src/smart_cmds.rs:847-859`) —
  manual refresh only in v0.2.0 (no watcher in the tree).
- CLI (VERIFIED, ran `photocraft-cli.exe --help` and `convert in.png doc.pcraft`, `convert in.png doc.psd`,
  `info doc.pcraft --compact`, `convert doc.psd back.png`, all exit 0): headless `convert`, `info` (layer tree JSON),
  `run --cmd`, `batch`, `serve` (headless JSON-lines session on stdio or 127.0.0.1), `mcp`. WTE could use it to turn a
  `.psd`/`.pcraft` saved by PhotoCraft back into a PNG without a PSD reader of its own (spawn with `CREATE_NO_WINDOW`).

### 4.2 PhotoCraft HEAD (latest main)
- Drop target: forked winit/egui (section 1). `crates/ui-egui/src/canvas/window_drop.rs:1-3`: files dropped on the
  canvas are **placed as layers** (Place Embedded, in Free Transform), on the tab strip opened at a slot, elsewhere
  opened (`file_open.rs:26-40` `DropTo`); `file_open.rs:108`: "Dropped onto the window (desktop): read from its path, or
  from the bytes the OS gave" (VERIFIED text). Whether the fork accepts in-memory images (CF_PNG/DIB) is unknown;
  `window_drop/tests.rs:14-24` models a desktop drop as path-only (VERIFIED), so assume CF_HDROP.
- Drag source: still no OLE drag code in the tree (VERIFIED by `git grep`; the winit fork is not inspectable).
- Linked smart objects **follow their files automatically**: `crates/engine-core/src/link_watch.rs:1-21` (VERIFIED):
  size + mtime polled every 2 s and on window activation, on a worker thread; one undo step per changed file.
- Clipboard: arboard 3.7 with more fallbacks (`apps/photocraft/src/services.rs:251-553`).

---

## 5. Alternatives to direct drag (for the PhotoCraft -> WTE direction and for "linking")

### 5.1 Linked file round trip ("Edit in PhotoCraft") — works with v0.2.0
1. WTE writes the layer (or stamp source) to a stable path, e.g. `%LOCALAPPDATA%\WrappedTextureEditor\links\<project>\<layer>.psd`
   (WTE would need a small single-layer PSD writer, ~150-250 lines, or run `photocraft-cli convert x.png x.pcraft`,
   VERIFIED to work), then starts `photocraft.exe <path>` (user-initiated, opens a window: never in tests) or the user
   drags that file (CF_HDROP) onto PhotoCraft.
2. PhotoCraft opens it with that path; Ctrl+S saves in place because the extension is psd/pcraft (VERIFIED, 4.1).
3. WTE notices the change and re-imports: polling `std::fs::metadata` (size + mtime, every 1-2 s, PhotoCraft HEAD's own
   approach) needs no FFI at all; or `FindFirstChangeNotificationW(dir, FALSE, FILE_NOTIFY_CHANGE_LAST_WRITE |
   FILE_NOTIFY_CHANGE_FILE_NAME)` / `ReadDirectoryChangesW` (kernel32, raw-dylib) on a worker thread. Watch the
   **directory**, not the file (other savers write a temp file and rename), and debounce until the file can be
   opened and decoded: PhotoCraft v0.2.0 saves documents with a plain in-place `std::fs::write`
   (`apps/photocraft/src/services.rs:125`, VERIFIED), so WTE can see a half-written file (its `write_atomic`,
   `services.rs:71-78` temp + rename, is used for prefs only).
4. Re-import: `.psd` composite -> WTE needs a PSD composite reader (or `photocraft-cli convert link.psd link.png`);
   `.pcraft` only through the CLI. Effort: 2-4 days incl. tests (all in-process testable, no GUI).

### 5.2 WTE output linked into PhotoCraft
WTE exports a layer PNG to a stable path; in PhotoCraft File > Place Linked that PNG. v0.2.0: refresh with Layer >
Smart Objects > Update Modified Content (manual); HEAD: automatic within ~2 s. WTE side: "keep exported link up to date"
= re-export on every layer change (debounced). Effort small (WTE already exports single layers).

### 5.3 Clipboard
WTE "Copy layer" / "Paste as stamp" with `user32` `OpenClipboard`/`EmptyClipboard`/`SetClipboardData`/
`GetClipboardData`/`RegisterClipboardFormatW("PNG")` + `kernel32` `GlobalAlloc`/`GlobalLock` via raw-dylib (~200-300
lines). Works both ways with PhotoCraft v0.2.0 (4.1, arboard). Risks: `OpenClipboard` contention (retry loop), tests
must not touch the user's real clipboard (unit-test the DIBV5/PNG encode/decode; any real-clipboard test `#[ignore]`).
`OleSetClipboard` with the same data object as the drag source is an alternative that shares code with 3.2.

### 5.4 PhotoCraft control channel
Only if WTE launches PhotoCraft itself with `--control <port> --control-token-file <f> --automation-read-root/-write-root`.
Gives WTE: open a file, run `file.placeLinked` / `layer.smartObjects.updateModifiedContent`, save to a path, inspect
layers. Does not work with a PhotoCraft the user opened from the Start menu; TCP on 127.0.0.1 only (no firewall
prompt for loopback, INFERRED). Medium effort (JSON-lines client, token handling), some coupling to PhotoCraft's
command ids that change between versions (`v0.2.0` vs HEAD differ, e.g. `link_watch`, `updateChanged`).

### 5.5 Forking PhotoCraft (user decision, not assumed)
Only a fork could add a real drag source (layer -> OLE data object) or accept a private WTE format. Out of scope unless
the user chooses it.

---

## 6. Feasibility by direction

| Direction | Today | Feasible? | How | Effort (INFERRED) | Main risks |
|---|---|---|---|---|---|
| Explorer -> WTE | works for files (VERIFIED `dnd.rs`) | yes, extend | own `IDropTarget` (2.4) for drop position, in-memory PNG/DIB, virtual files (browser/Outlook drags) | 2-3 days with tests | vtable/refcount bugs (crash across FFI), self-test must skip the swap, HiDPI point conversion |
| WTE -> Explorer | no | yes | CF_HDROP of a real file + `SHCreateDataObject`/`SHDoDragDrop` from a posted `WM_APP` message in the wndproc subclass (3.1-3.3); inject pointer-up afterwards | 3-5 days incl. temp-file policy | re-entrancy panic if called from `update`; stuck pointer state; pen may not drive OLE; files must outlive the drop |
| WTE -> PhotoCraft | no | yes (same code as WTE -> Explorer) | CF_HDROP. v0.2.0: opens as a **new document** (not a layer); HEAD: dropped on canvas = placed as layer. Use `.psd`/`.pcraft` to get in-place Save back | +0.5 day over the above | v0.2.0 cannot place into the open document; PNG drops need Save As in PhotoCraft |
| PhotoCraft -> WTE | no | **not by drag** (PhotoCraft has no drag source, v0.2.0 or HEAD, VERIFIED) | linked file + watcher (5.1), clipboard (5.3), or the user drags the saved file from Explorer (already works) | 2-4 days (5.1), 1-2 days (5.3) | detecting complete writes, PSD/pcraft decoding (CLI dependency), version drift of PhotoCraft |
| WTE <-> WTE (two windows) | no | yes | private format in the same data object | small once source + target exist | none special |

Testing constraint (project rules): no OS mouse/keyboard injection and no GUI runs that take focus, so an end-to-end
drag can never be automated here. What can be tested in-process: the data object's formats (`GetData` on the object
WTE builds), WTE's `IDropTarget` driven by calling its vtable with a `SHCreateDataObject`-made object, DIBV5/PNG
encoding, the link watcher (write a file, see the reload), the pointer-release injection. A real drag between the two
apps, with mouse and with pen, stays a hand test for the user (like `dist/HOW_TO_TEST.txt` for the pen).

Other risks (INFERRED): UIPI blocks drag and drop from a non-elevated to an elevated process (both apps must run at
the same integrity level); OLE does not activate the target window, so drops do not steal focus (good for the user's
rule); a drag that starts from the 3D view must not conflict with painting/orbit gestures (needs a modifier or a drag
handle on the layer row).
