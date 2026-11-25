# PhotoCraft interop doors (v0.2.0 vs latest main)

Research note for the WTE "sticker editing + PhotoCraft link" request, 2026-11-25.
Scope: every way another app can exchange images or layers with PhotoCraft.

Legend: **VERIFIED** = read in source/docs at the cited place, or ran with the user's v0.2.0 `photocraft-cli.exe`
(headless, console only, scratch dir `G:\Claude_Resources\ref\scratch\pcinterop\`). **INFERRED** = deduced (from strings
in the v0.2.0 binary, from upstream crate behaviour, or from code that is not available locally).

Sources:
- Reference clone `G:\Claude_Resources\ref\photocraft` (HEAD b5c762017, 2026-11-25, docs equal to tag v0.24.0-rc.2;
  latest stable tag v0.23.1, 2026-11-11). "latest" below = HEAD.
- v0.2.0 = tag v0.2.0 (ad8632173, 2026-10-05, "Release 0.2.0"); the user's portable exes are dated 2026-10-05 and
  `photocraft-cli --help` matches the tag's `apps/photocraft-cli/src/lib.rs` usage text (VERIFIED).
- **Caveat (VERIFIED):** the clone is a partial (promisor) clone and 587 of the 828 v0.2.0 blobs are missing locally,
  including all of `crates/ui-egui`, `crates/engine`, `crates/io`, `crates/psd`, `crates/format` at v0.2.0. Readable at
  v0.2.0: `apps/photocraft/src/*`, `apps/photocraft-cli/src/*`, `crates/automation/src/*`, `docs/control-protocol.md`,
  `book/src/**`. Older tags (v0.10 ... v0.23.1) are mostly missing too, so "when was X added" is only known where a
  tree entry exists. v0.2.0 UI behaviour (drop, paste, save) was therefore checked against strings extracted from the
  user's `photocraft.exe` plus headless CLI runs. Use `GIT_NO_LAZY_FETCH=1` when grepping the clone (otherwise git tries
  to fetch missing blobs and prints promisor errors).
- Nothing was built; no GUI exe was started. CLI runs wrote nothing to `%APPDATA%\Photocraft` (still absent, VERIFIED;
  the user has apparently never run the v0.2.0 GUI on this machine, INFERRED).

---

## 1. Command line, single instance, file associations

### v0.2.0
- `photocraft [--control <port>] [--control-token <hex> | --control-token-file <path>] [--automation-read-root <dir>]
  [--automation-write-root <dir>] [--version] [files...]`. Every other argument is a file; files are opened at start
  with `app.open_paths(&files)`. VERIFIED: `git show v0.2.0:apps/photocraft/src/main.rs` lines 3-5, 47-63, 146.
- **No single-instance logic.** Every launch is a new process and window; a second `photocraft.exe x.png` does not hand
  the file to the running app. VERIFIED: v0.2.0 `main.rs` has no IPC (whole file, 150 lines); the app crate has only
  `apple_events.rs control_server.rs crash_guard.rs main.rs monitor_profile.rs services.rs`.
- **No file associations**: the portable zip has only the two exes, README and licences; no installer, nothing written to
  the registry (VERIFIED: folder listing; no `HKEY_CLASSES_ROOT`/`RegSetValue` strings in `photocraft.exe`). No
  `portable.txt` either, so v0.2.0 keeps settings in `%APPDATA%\Photocraft` (VERIFIED v0.2.0 `docs/control-protocol.md`,
  Preferences section).
- The GUI is a Windows-subsystem exe (`#![windows_subsystem = "windows"]`, v0.2.0 `main.rs` line 13): starting it opens a
  window that takes focus (never do that in tests).

### latest
- clap-based `LaunchArgs`: `files...`, `--safe-gpu`, `--in-window-menus`, `--new-instance`, `--present-mode`,
  `--frame-latency`, `--stroke-trace`, `--control <port>` + auth/automation flags. VERIFIED
  `apps/photocraft/src/launch_args.rs` lines 24-63. No "place as layer" or "open into document X" flag.
- **Single instance (Windows/Linux, since <= v0.10.0, tree entry exists at tag v0.10.0):** first launch locks
  `instance.lock` in the settings folder, listens on a loopback port, writes port + 256-bit token to `instance.json`; a
  later launch sends its absolute paths there and exits; the primary opens each as a **new tab** and brings the window
  forward. VERIFIED `apps/photocraft/src/single_instance.rs` lines 1-20 (module doc), `main.rs` lines 247-290.
- Skipped (separate process) when: settings folder is not the per-user one (**portable copy** or `PHOTOCRAFT_CONFIG_DIR`),
  `--new-instance`/`PHOTOCRAFT_NEW_INSTANCE`, any `--control` launch, non-Unicode paths, per-process options. VERIFIED
  `single_instance.rs` lines 185-203. The latest Windows portable zip ships `portable.txt` (VERIFIED
  `packaging/windows/package.ps1` line 8, `packaging/windows/portable.txt`; `apps/photocraft/src/app_dirs.rs` lines 6-8),
  so **a portable latest copy does NOT forward files**; only the MSI install does.
- The `instance.json` protocol is private (token + JSON line of paths); in principle WTE could speak it, but it only
  opens files as new documents, so `photocraft.exe <file>` already gives the same result. INFERRED.
- **File associations (MSI only):** ProgIds `Photocraft.PhotoshopDocument` / `Photocraft.Image`, `OpenWithProgids` for
  psd/psb/pdn/ora/kra/png/jpg/tif/webp/gif/bmp/tga/exr/hdr/avif/qoi/ico..., and `.pcraft` with PhotoCraft as default app.
  VERIFIED `packaging/windows/install-policy.json` (registry list), `packaging/windows/photocraft.wxs` lines 75-77, 110.
  Command line is `"photocraft.exe" "%1"`.

---

## 2. OS drag and drop INTO PhotoCraft

### v0.2.0
- Window built with `.with_drag_and_drop(true)` (VERIFIED v0.2.0 `main.rs` line 110). winit 0.30.13 registers an OLE drop
  target that accepts **files only** (CF_HDROP via `DragQueryFileW`); non-file data is rejected with
  "item is not a file". VERIFIED as strings in `photocraft.exe` (`RegisterDragDrop`, `DragQueryFileW`,
  `winit-0.30.13\src\platform_impl\windows\drop_handler.rs`, "Error occurred while processing dropped/hovered item:
  item is not a file."). So a drag of raw bitmap data (CF_DIB) or a virtual file (FileGroupDescriptor) is refused;
  `FileGroupDescriptor` does not occur in the binary (VERIFIED).
- What a drop does: the home screen string is "Drop an image or PSD anywhere to open it." (VERIFIED, binary string). The
  v0.2.0 UI code is not available locally, so whether a drop on an open canvas places a layer is **unknown**; most likely
  every drop opens a new document (INFERRED: the web shell of the same tag feeds drops into the "inbox" that opens
  documents, `git show v0.2.0:apps/photocraft-web/src/web.rs` lines 98-123).
- Accepted formats = File > Open formats: pcraft, psd, psb, png, jpg/jpeg, tif/tiff, webp, gif, bmp, tga, ico, qoi, exr,
  hdr, pbm/pgm/ppm/pam/pfm, ... VERIFIED `git show v0.2.0:apps/photocraft/src/services.rs` lines 15-18. No ORA in v0.2.0.

### latest
- Routing (VERIFIED `crates/ui-egui/src/file_open.rs` lines 1-6, 25-56, 404-430, 663-666):
  - drop on the **active document's canvas** -> **Place Embedded** (smart object layer, Free Transform starts), one per
    file; centred on the selection, else on the visible part of the canvas; a drop point on a Frame layer fills that
    frame. Never "Place Linked".
  - drop on the **tab strip** -> opened as documents at that tab position.
  - anywhere else -> opened like File > Open.
  - brush/gradient preset files go to their libraries wherever dropped.
- Still file-path drops only (framework `craft::ui::runtime::drag::OsFile`, path or bytes; INFERRED that Windows gives
  paths, since winit only supports CF_HDROP). Folders are refused with a message (`folder_error`, lines 77-87).
- Long places run on a worker; the layer is inserted when the job lands (VERIFIED `docs/control-protocol.md` line 322).
- Elevation note: an admin PhotoCraft does not receive drops from a non-admin Explorer (VERIFIED comment
  `file_open.rs` lines 89-92); same applies to drops from WTE if integrity levels differ (INFERRED, standard UIPI).

---

## 3. Drag OUT of PhotoCraft

- **None, in both versions.** No OLE drag source: `DoDragDrop`/`IDropSource` absent from v0.2.0 `photocraft.exe`
  (VERIFIED strings). Latest source has no drag-out code (VERIFIED grep for DoDragDrop/IDropSource/drag-out in
  `crates/` and `apps/`; the craft-libs framework is not cloned, so INFERRED for that part). winit 0.30 / egui provide
  no drag source either (INFERRED, upstream).
- Consequence: "drag a layer from PhotoCraft into WTE" cannot work with unmodified PhotoCraft. The reverse (WTE as drag
  source -> PhotoCraft) works only if WTE offers real files (CF_HDROP of a temp PNG/PSD).

---

## 4. Clipboard

### v0.2.0
- OS clipboard via `arboard` 3.6.1 (VERIFIED: `arboard-3.6.1` path string in the exe; v0.2.0 `services.rs` lines
  135-142): `set_image(width, height, RGBA8)` and `get_image() -> RGBA8`. Images only; no text/file-list paste path.
- Windows formats (INFERRED from arboard 3.6 behaviour, consistent with exe strings "Cannot register PNG clipboard
  format", "failed to read clipboard PNG data", "When reading the DIBV5 data..."): copy writes **CF_DIBV5 (32-bit with
  alpha) + registered "PNG"**; paste reads "PNG" first, else CF_DIBV5/CF_DIB. Transparency survives through the "PNG"
  format (and DIBV5 alpha for readers that honour it).
- Preference `general.exportClipboard` defaults to **true** (VERIFIED: `photocraft-cli run --new ... --cmd prefs.get
  {"path":"general"}` printed `"exportClipboard":true`), i.e. Edit > Copy mirrors the session clipboard to the OS.
- Commands: `edit.copy`, `edit.copyMerged`, `edit.cut`, `edit.paste {center?}`, `edit.pasteSpecial.pasteInPlace`,
  `pasteInto`, `pasteOutside`, `edit.purge.clipboard` (VERIFIED: `photocraft-cli commands --json`, saved as
  `scratch\pcinterop\cmds.json`).

### latest
- Same arboard path (now arboard 3.7, VERIFIED `apps/photocraft/Cargo.toml` line 50). Copy mirrors the **clipboard
  bounds** as RGBA8 when `exportClipboard` is on (default true, VERIFIED `crates/engine-core/src/prefs.rs` line 322;
  `crates/ui-egui/src/app/clipboard.rs` lines 80-99).
- Paste: if the OS clipboard holds an image that PhotoCraft did not put there, it becomes the session clipboard and is
  pasted (as a new layer). VERIFIED `clipboard.rs` lines 100-106. New since v0.2.0: paste of **copied files**
  (Explorer Copy = file list; first decodable image is used, VERIFIED `apps/photocraft/src/services.rs` lines 265-285,
  529) and SVG text (`clipboard_get_svg`).
- No custom clipboard format carrying layer name/position: pixels only, position lost (INFERRED from the RGBA8-only
  `ClipboardSetFn(w, h, px)` signature).

---

## 5. File watching, linked smart objects, Edit Contents, saving

### Linked smart objects (both versions)
- Commands exist in v0.2.0 (VERIFIED `cmds.json`): `file.placeEmbedded {path, scale?, fit=true, center?}`,
  `file.placeLinked {path, scale?, fit=true, center?}`, `layer.smartObjects.updateModifiedContent {layer?}`,
  `updateAllModifiedContent`, `relinkToFile {layer?, path}`, `replaceContents {layer?, path}`,
  `exportContents {layer?, path}`, `convertToLinked {layer?, path}` (writes contents there), `convertToEmbedded`,
  `editContents {layer?}` -> opens contents as a new document, `saveContents {}` (in that document).
- **Ran (v0.2.0 CLI, VERIFIED):**
  1. `run --new {256x256 transparent} --cmd file.placeLinked {path:sticker.png, center:[100,80]}` -> smart object layer
     at the given centre. `.pcraft` manifest stores `"source":{"kind":"linked","path":"G:/.../sticker.png"}`
     (absolute path, as given) plus a cached render and a 2x3 transform matrix.
  2. Changed `sticker.png` on disk (64x48 red ellipse -> 80x40 green rectangle), reopened the `.pcraft`:
     the composite still showed the old pixels until `layer.smartObjects.updateAllModifiedContent` -> `{"updated":[5]}`;
     afterwards the new pixels, new size 80x40, **top-left kept** at (68,56) (not re-centred).
  3. `editContents` on the linked layer opened `sticker.png` as document 1; `image.adjustments.invert` +
     `saveContents` -> parent updated, but the **file on disk was not written** and the layer became
     `"source":{"kind":"embedded"}`. So Edit Contents does not round-trip into the linked file in v0.2.0.
- **No automatic reload** of linked files in either version: no file-watcher APIs in the v0.2.0 exe
  (`ReadDirectoryChangesW`/`FindFirstChangeNotification` absent, VERIFIED strings); latest has no watcher for linked
  sources (VERIFIED grep: only the Home screen recent-file thumbnails recheck on focus,
  `crates/ui-egui/src/home_recent_ui.rs` lines 86-87). "Update Modified Content" is a manual menu command (latest
  `crates/engine-core/src/cmd/smart_cmds.rs` line 269, enabled only when the document has linked layers).
- Latest has one live external-editor round trip, **SVG only**: an owned temp SVG is polled every 200 ms and saved back
  (VERIFIED `apps/photocraft/src/external_svg.rs` lines 1-4, 25-29). Not for raster layers.
- Over the control channel and headless `serve`, all `layer.smartObjects.*` except `convertToSmartObject` and all
  `file.*` (place/export) are refused in v0.2.0 (see 6). In latest, in-memory `editContents`/`convertToLayers`/
  `saveContents` are allowed, sources needing a disk read stay refused (VERIFIED `docs/control-protocol.md` lines 261-272).

### Generate > Image Assets (v0.2.0 and latest)
- `file.generate.imageAssets {on?, dir?}`: layers named like `foo.png`, `200% foo@2x.png`, `48x48 icons/a.png8`,
  `photo.jpg80%` are exported into `<document>-assets` "now and after each save" (VERIFIED command doc in `cmds.json`).
- Ran (VERIFIED): on a PSD with a layer named `sticker2.png` it wrote `doc-assets\sticker2.png`, **trimmed to the
  layer's bounds (33x33), no position**. The "after each save" part in the GUI is INFERRED.
- Gotcha: WTE stamp layers are named after their image file (`wf_stamp.png` in a WTE PSD, VERIFIED below), so they
  would match Image Assets naming if the user turns Generate on.

### Script Events Manager (v0.2.0 and latest)
- `file.scripts.scriptEventsManager {enabled?, add:{event, script?|steps?, name?}}`, events `startApplication`,
  `newDocument`, `openDocument`, `saveDocument`, `closeDocument`, `print`, `export`, `everything` (VERIFIED by running
  it headless; stored under preference `scriptEvents`). A user-configured `saveDocument` -> `file.export.layersToFiles
  {dir: <exchange folder>}` binding would make every GUI save drop per-layer PNGs for WTE (INFERRED that the GUI runs
  file.* steps for a user binding; automation opens/saves never fire script events, VERIFIED latest
  `docs/control-protocol.md` line 179). Automation cannot set `scriptEvents` (`prefs.set` refused, VERIFIED v0.2.0
  `crates/automation/src/workspace.rs` lines 209-221).

### Saving
- `.pcraft` save keeps everything incl. links; PSD save of v0.2.0 **flattens smart objects to pixels** (VERIFIED: CLI
  warning `layer "sticker": smart object written as pixels`, no `SoLd`/`PlLd`/`lnk2` in the file).
- Latest PSD export keeps smart objects as smart objects (`SoLd`/`PlLd`, contents embedded in `lnk2`), also for sources
  read from a link (VERIFIED test `crates/engine/tests/it/smart_psd.rs` lines 1-3, 79-104;
  `crates/io/src/psd_export/smart.rs`). The PSD writer itself is now in craft-libs `craft-psd` (not cloned).
- File > Save target: latest writes back only to the document's own PSD/PSB/.pcraft/ORA, or to its own flat file while
  the document is still a single plain layer; otherwise it asks (Save As). VERIFIED `crates/ui-egui/src/app/files.rs`
  lines 504, 554-563. v0.2.0 behaviour unknown (UI code missing; INFERRED similar but unverified).
- `file.revert {}` "reloads the saved file as one undoable step" exists in v0.2.0 (VERIFIED `cmds.json`): the only
  "reload from disk", manual.
- Autosave/crash recovery writes `.pcraft` into the settings `Recovery` folder (VERIFIED v0.2.0 `services.rs`
  lines 155-176); not an exchange path.

---

## 6. Control channel, headless server, MCP, CLI

### v0.2.0 desktop control (VERIFIED `git show v0.2.0:docs/control-protocol.md`, `apps/photocraft/src/main.rs` 64-90,
### 136-140, `apps/photocraft/src/control_server.rs`)
- Only when the GUI is **launched with `--control <port>`** (or `PHOTOCRAFT_CONTROL_PORT`). The preference
  `integrations.controlPort` exists (CLI `prefs.get` -> `{"allowAgentControl":true,"controlPort":0}`) but v0.2.0
  `main.rs` never reads it; `control_server::start` is called only from `main.rs` line 138 (VERIFIED).
- Transport: TCP `127.0.0.1:<port>`, JSON lines `{"id","method","params"}` -> `{"id","ok","result"|"error"}`;
  1 MiB request line, 16 connections, 30 s socket timeout.
- Auth: first frame `{"method":"auth","params":{"token":"<64 hex>"}}`. Token from `--control-token`,
  `--control-token-file <path>` (created with a fresh 256-bit token if missing, reused if present) or the env
  equivalents; without any, a per-launch token is printed to stderr (useless for a GUI-subsystem exe: no console).
- Methods: `engine.execute {command, params}`, `engine.commands`, `ui.inspect`, `ui.set`, `ui.menu.invoke/list`,
  `ui.dialog.*`, `ui.window.open/close`, `ui.pointer`, `ui.key`, `ui.type`, `ui.resize`, `ui.screenshot {path?,
  focus?}` (raises the window by default), `ui.focus`, `app.open {path}`, `app.save {path}`, `app.quit`.
  **No events / subscriptions / change notifications** (request-reply only); poll `document.inspect` (`revision`,
  `history`) or `session.list`-like data from `ui.inspect`.
- File access is capability-scoped: `app.open`/`app.save` take relative paths under `--automation-read-root` /
  `--automation-write-root` (fail closed without roots). `engine.execute` refuses every `file.*` command except
  new/close/closeAll/closeOthers/fileInfo/fitImage/conditionalModeChange/flatten scripts/deleteAllEmptyLayers/
  exportPreferences, every `layer.smartObjects.*` except `convertToSmartObject`, `layer.exportAs`/`quickExportAsPng`
  with a path, colour-profile and scriptEvents changes. VERIFIED `git show v0.2.0:crates/automation/src/workspace.rs`
  lines 131-167, 182-232. So over control: **no Place, no Place Linked, no Export Layers, no Image Assets**.
- What does work over control (VERIFIED with the same engine in headless `serve`, section below): open a PNG/PSD from
  the read root as a second document, `select.all` + `edit.copy`, switch document, `edit.pasteSpecial.pasteInPlace`,
  `layer.setProps {name}` -> a new named pixel layer in the target document. In the desktop app `edit.copy` would also
  overwrite the user's OS clipboard (exportClipboard = true) (INFERRED). `app.save {path:"x.psd"}` writes a layered PSD,
  `x.png` the flattened composite.

### v0.2.0 headless (no window; `photocraft-cli.exe` is a console exe, VERIFIED ran without any window)
- `photocraft-cli serve [--port <p>] [--control-token...] [--automation-read-root] [--automation-write-root]`: one
  engine session, JSON lines on stdio (no auth on stdio) or loopback TCP (auth as above). Methods (VERIFIED by running
  `methods`): `engine.execute, engine.commands, session.list, doc.open, doc.new, doc.save, doc.inspect, doc.render,
  doc.select, doc.close, batch, methods`. `doc.save` = `.pcraft` native else export by extension; `doc.render` returns
  base64 PNG or writes a file. Same `file.*` gate: `file.export.layersToFiles` and `file.placeLinked` were refused
  ("uses ambient filesystem paths and is disabled") although roots were given (VERIFIED run).
- `photocraft-cli run (<file> | --new <json>) --cmd <id> [--params <json>] ... [--out <file>] [--format <ext>]` uses
  `Headless::trusted_local()`, **no gate**: every engine command incl. place/placeLinked/layersToFiles/updateModified
  works with absolute paths (VERIFIED `git show v0.2.0:apps/photocraft-cli/src/lib.rs` lines 209-238,
  `crates/automation/src/headless.rs` lines 179-185, and the runs in section 5). Also `convert`, `info --compact`
  (layer tree JSON), `batch --actions --in --out`, `droplet`, `commands --json`, `mcp`.
- Speed (VERIFIED): `run` on a 256x256 5-layer PSD incl. Layers to Files ~0.1 s; `info` on a 135 MB 4096x4096 WTE PSD
  ~2.4 s.
- MCP: `photocraft-cli mcp` headless or `--bridge 127.0.0.1:<port> --control-token-file <path>` to a `--control` GUI;
  tools `command_run`, `command_list`, `command_batch`, `doc_new/inspect/open/save/export/render_preview`,
  `doc_select/close` (headless only), `ui_*` and `control_call` (bridge only). VERIFIED v0.2.0 control-protocol doc,
  "MCP bridge". For WTE (a Rust app) MCP adds nothing over the raw JSON-lines protocol.

### latest additions (VERIFIED `docs/control-protocol.md`, `docs/mcp.md` at HEAD)
- Control without a command line: `integrations.connectAgents` / a saved Control Port (or the ArtCraft Launcher switch
  with the `AI-MCP` flag) makes the running app serve on loopback and publish `agent-connection.json`
  (`{version, pid, port, tokenFile, app}`) next to `agent-control.token` in its settings folder, removed on quit
  (control-protocol lines 20-55). A client reads the file, checks the pid, connects. This is the only way to reach an
  app the user started normally, and it needs a user opt-in in Preferences > AI Integrations.
- Auth: new `auth.challenge` (HMAC-SHA256 both ways, never sends the token) plus the old single-frame token auth
  (lines 57-76). 60 s reply deadline with `timeout: not run` vs `timeout: no reply in 60s`.
- `app.save` without `path` writes back to the document's own PSD/PSB/.pcraft; extra options for TGA/DDS/JPEG/TIFF/PSD
  compatibility (line 179). Still no push events.
- Desktop `file.*` gate unchanged: `doc_place` -> `file.placeEmbedded` and `doc_export_layers` ->
  `file.export.layersToFiles` are mapped in bridge mode but the desktop refuses ambient-path commands (lines 261,
  457-459). Headless `serve` gained capability-scoped `doc.place`, `doc.replaceSmartObject`, `doc.exportLayers`,
  `doc.exportDataSets`, `doc.importDataSets`, jobs (lines 555-575). Directory-bundle `.pcraft` via `app.open`.

---

## 7. File formats for a WTE <-> PhotoCraft layer link

### PSD (8-bit RGB, both directions) - best common denominator
- v0.2.0 writer (VERIFIED on `linked.psd` written by the CLI): `8BPS` version 1, 4 channels, RGB, 8 bit, per-layer RLE
  (PackBits), layer records with Unicode names (`luni`) and ids (`lyid`), per-layer **trimmed bounds** (a layer holds
  only its non-transparent rectangle; empty layers get 0x0 bounds), straight (non-premultiplied) RGBA. Smart objects
  flattened to pixels (warning).
- **WTE -> PhotoCraft v0.2.0 works today (VERIFIED):** a real WTE export
  (`texture_wrapper_ap_r9\target\accept\verify_layer-save_2\box_workflow_path_export\BoxTextured.psd`, 256x256, layers
  Base / Paint 1 / Paint 2 / `wf_stamp.png` / WTE) opened with names, order, bounds, blend Normal, opacity 1, visibility
  intact; re-saved as PSD by PhotoCraft, every layer exported identical (max abs diff 0 on all 5 layers, compared via
  `file.export.layersToFiles` of both files).
- PhotoCraft -> WTE: WTE's PSD reader is described as a "strict verification reader" of its own files
  (WTE `ARCHITECTURE.md` line 392); it would have to accept PhotoCraft's PSDs (RLE, trimmed bounds, extra tagged
  blocks, groups `lsct`, adjustment/type/shape layers, 16/32-bit or CMYK documents) or ask PhotoCraft to flatten them.
  INFERRED; not checked here.
- PSD loses PhotoCraft-only state (effects are kept as PSD effects, adjustment/type layers as PSD equivalents per
  README; smart objects flattened in v0.2.0).

### Per-layer PNGs via Layers to Files - simplest PhotoCraft -> WTE path
- `file.export.layersToFiles {dir, format="png", prefix=<doc name>, visibleOnly=true, quality?}` -> `{files}`
  writes `<prefix>_<NNNN>_<layer name>.png`, **full canvas size**, RGBA, position preserved, top layer = 0000
  (VERIFIED run; names keep a trailing `.png` from the layer name, e.g. `wte_box_0001_wf_stamp.png.png`). Top-level
  layers only (latest doc: "one file per exported top-level layer", control-protocol line 562). For WTE (canvas =
  texture) these PNGs drop straight into texture layers.
- `layer.exportAs {layer?, path, scale}` / `layer.quickExportAsPng {layer?, path}` export one layer (v0.2.0, VERIFIED
  in `cmds.json`; trimmed vs canvas size not tested).

### .pcraft (native)
- ZIP (stored, 1980 timestamps) with `manifest.json`, `thumb.png`, `composite/preview.png`, content-addressed
  `tiles/<blake3>.zst` (zstd) and `blobs/<blake3>.zst`; manifest `format_version: 1`, `generator: "photocraft-format
  0.2.0"`, document tree as serde JSON of `photocraft-doc` (layers with `content.kind` raster/smart/..., smart
  `source.kind` linked{path}/embedded, `transform.m` 2x3) (VERIFIED: unzipped `linked.pcraft`; v0.2.0
  `book/src/formats/pcraft.md`).
- Latest adds optional `composite/full.png` (off by default) and directory bundles with `save.lock`; FORMAT_VERSION is
  still 1 with an empty migration list (VERIFIED `crates/format/src/migrate.rs` lines 8-10, latest
  `book/src/formats/pcraft.md`). Whether v0.2.0 opens a latest .pcraft (new fields) is untested (INFERRED risky).
- Documented only at the container level; the manifest is an internal serde dump that changes with `photocraft-doc`
  ("When fields are added to photocraft-doc, photocraft-format intentionally requires corresponding manifest and
  conversion updates"). Reading it needs zip + zstd + knowledge of the tile layout; WTE has `zip` 9.5.1 (deflate only)
  and no zstd crate (VERIFIED WTE `Cargo.lock`), so .pcraft reading needs a new crate (user OK required). Writing
  .pcraft from WTE is not advisable. INFERRED: not a stable interchange format.
- Its one unique value: a **linked smart object with an absolute path** survives in .pcraft (not in v0.2.0 PSD).

### ORA
- Not readable by v0.2.0 (not in OPEN_EXTS); readable/writable in latest (`crates/ui-egui/src/file_dialog/filters.rs`
  line 13; Save writes back to ORA "only when it holds everything"). WTE already writes ORA (ARCHITECTURE 7.4).

### Recommendation for the link format (INFERRED)
- Document level: **PSD 8-bit RGB** both ways (works with v0.2.0 and latest; WTE already writes it).
- Single-layer level: **PNG at texture size** (WTE -> PhotoCraft as placed/pasted layer; PhotoCraft -> WTE via
  Layers to Files or a per-layer export).
- Live "linked" layer: PhotoCraft **Place Linked** to a PNG that WTE owns + manual "Update Modified Content" in
  PhotoCraft; for the opposite direction WTE watches a PNG/PSD that PhotoCraft saves. Needs the document to be saved
  as .pcraft to keep the link (v0.2.0 PSD drops it).

---

## 8. Conclusion: mechanisms WTE can use

### TODAY with the user's v0.2.0 (unmodified PhotoCraft)
1. **Launch with files** (user-initiated, e.g. "Edit in PhotoCraft"): WTE writes `<layer>.png` or a layered `.psd` to
   an exchange folder and starts `photocraft.exe <file>` (always a new window, no single instance, no associations).
   VERIFIED args; window/focus behaviour INFERRED.
2. **File round trip**: user saves in PhotoCraft (Ctrl+S on the opened PSD; behaviour for flat PNG unverified); WTE
   watches the file (WTE-side watcher) and re-imports changed layers. PSD layers come back by name; PhotoCraft gives no
   notification, so WTE must poll/watch.
3. **Headless conversion with `photocraft-cli.exe run`** (console exe, no window, ~0.1 s): WTE can drive the full
   engine without the gate: `file.export.layersToFiles` (canvas-size PNG per layer), `info --compact` (layer tree),
   `file.placeLinked/placeEmbedded`, `layer.smartObjects.updateAllModifiedContent`, `convert` PSD/.pcraft/PNG.
   Requires knowing where the user's PhotoCraft is (settings field). VERIFIED.
4. **OS drag WTE -> PhotoCraft**: WTE must be an OLE drag source offering CF_HDROP of a real temp file; PhotoCraft opens
   it as a document (canvas placement in v0.2.0 unverified). PhotoCraft -> WTE drag: impossible (no drag source).
5. **Clipboard**: WTE copy as "PNG" + CF_DIBV5 with alpha -> Ctrl+V in PhotoCraft pastes a layer (INFERRED arboard
   behaviour). PhotoCraft Ctrl+C -> OS clipboard as "PNG" + CF_DIBV5 -> WTE paste. Pixels only (no name/position).
   WTE currently has no OS clipboard (eframe `clipboard` feature off, arboard not in the registry: WTE ARCHITECTURE
   lines 69-72) -> needs arboard or Win32 clipboard code (user OK for crates).
6. **Control channel**: only if WTE itself launches `photocraft.exe --control <port> --control-token-file <file>
   --automation-read-root <x> --automation-write-root <x>`. Gives `app.open`, `app.save` (layered PSD / PNG),
   document/layer inspection, and in-memory copy/paste of a layer between documents, but no Place, no Export Layers,
   no linked-object commands, no events. A PhotoCraft the user started normally cannot be reached.
7. **User-configured automation in PhotoCraft**: Script Events `saveDocument` -> Layers to Files into the exchange
   folder, or Generate > Image Assets for layers named `*.png` (trimmed PNGs) - turns every PhotoCraft save into files
   WTE can watch. Needs the user to set it up in PhotoCraft's UI.

### With the latest release (v0.23.1 / main ~ v0.24.0-rc.2), additionally
- **Single instance** for the MSI install: `photocraft.exe <file>` from WTE adds a tab to the running window instead of a
  second window (portable zip still standalone). File associations for psd/png/.pcraft etc. (Open With).
- **Canvas drop = Place Embedded** as a smart-object layer in Free Transform: a WTE drag source (CF_HDROP temp PNG) can
  drop a sticker straight into the open document. Tab-strip drop = open.
- **Paste of copied files** (Explorer-style file list) in addition to bitmaps.
- **Reach the user's running app**: if the user enables agents (Preferences > AI Integrations Control Port, or the
  launcher switch), WTE reads `agent-connection.json` + `agent-control.token` from PhotoCraft's settings folder and
  connects with `auth.challenge` (HMAC-SHA256) or token auth. Same method set as above, `app.save` without path saves
  the document in place; still no place/export-layers on the desktop, still no events.
- Headless `serve` gains scoped `doc.place`, `doc.replaceSmartObject`, `doc.exportLayers`.
- PSD export keeps smart objects (embedded in lnk2) instead of flattening them; ORA read/write; DDS/TGA export.
- Still missing in latest: drag-out from PhotoCraft, push notifications, automatic linked-file refresh (manual
  Update Modified Content), raster external-editor round trip (SVG only), any layer-level clipboard metadata.

### What only a PhotoCraft fork could add (for the user's decision)
- OLE drag source for layers, a file watcher that auto-runs Update Modified Content, Edit Contents writing back to the
  linked file, control events (document saved / layer changed), a `place`/`exportLayer` method on the desktop control
  channel that is scoped to a WTE exchange folder.
