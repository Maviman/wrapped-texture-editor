# WTE model and code for stickers / stamps / layers: where a sticker editor and a PhotoCraft link plug in

Research notes (2026-11-25) for the request "integrate sticker editing just like PhotoCraft ... drag and drop between
the apps and seamlessly link photos/layers". Source read: `G:\software_projects\texture_wrapper_ap_cmp` (branch
`companion` @ 90ada7b = main v0.2.0 + pen input + companion). Paths below are relative to that worktree unless they
start with `REG\` (= `G:\Claude_Resources\rust\cargo\registry\src\index.crates.io-1949cf8c6b5b557f\`) or `PC\`
(= `G:\Claude_Resources\ref\photocraft`, read with `git show v0.2.0:<path>`). Nothing was built or run; no GUI started.

Tags: **VERIFIED** = read in code at the cited line(s). **INFERRED** = my reasoning from the code, not executed.

---

## 1. Data model of stamps ("stickers")

### 1.1 StampObject and its source

- **VERIFIED** `crates/wte_core/src/stamp.rs:10-14`: `enum StampSource { Image { blob: BlobId }, Text(TextSpec) }`.
  An image sticker refers to a blob id only; there is no per-stamp image edit data (no crop, mask, filters,
  adjustments, outline, shadow).
- **VERIFIED** `stamp.rs:91-101`: `StampObject { id, name, source, placement, tint: [u8;4], home_set }`. The only
  appearance parameter of an image sticker is `tint` (multiplies the straight RGBA source, default `[255;4]`).
- **VERIFIED** `stamp.rs:84-88`: `enum Placement { Projected(Projector), Uv(UvPlacement) }`.
  - `Projector` (`stamp.rs:17-44`): origin, rotation quat, half_size, depth front/back, angle start/end, backface
    cull, occlusion, flip_x/flip_y, `mode: SurfaceMode { Project | Wrap }` (`:47-55`), `wrap: WrapOptions { jump_gaps,
    overlap }` (`:58-65`). "Wrapped stamps" are just `Projected` with `mode == Wrap` (round 6).
  - `UvPlacement` (`stamp.rs:67-81`): set, center_px, size_px, rotation (rad), flips, clip_to_islands (a "2D stamp").
  - **VERIFIED** user decision (HANDOFF.md section 6): 3D (projected) and 2D (UV) stamps stay separate kinds, not
    cross-editable.
- **VERIFIED** `crates/wte_core/src/doc_stamps.rs:43`: every `set_stamp_*` edit has the history label "Edit Stamp".

### 1.2 Image bytes: the BlobStore (embedded, not by path)

- **VERIFIED** `crates/wte_core/src/blob.rs:18-35`: `Blob { id, kind: Image|Font, name, bytes: Arc<[u8]>, crc32 }` in
  a `BlobStore` (BTreeMap). The **original encoded file bytes** are stored (PNG/JPEG/...), not decoded pixels.
- **VERIFIED** `blob.rs:44-67`: `insert` de-duplicates by kind + (len, crc32) + full compare, returns the existing id.
  `blob.rs:28-30`: blobs are never garbage-collected in memory (undo history may reference them).
- **VERIFIED** `doc_stamps.rs:211-216` `Document::add_image_blob(name, bytes)`: decodes (max side 8192,
  `doc_stamps.rs:45`), inserts the blob, builds `PremulMips` into `Document::image_mips` (`document.rs:70`,
  `HashMap<BlobId, Arc<PremulMips>>`). Not a history command.
- **VERIFIED** `doc_stamps.rs:1028-1037` `image_source(blob)`: mips cached per blob id; rebuilt from the blob bytes
  when missing (after load).
- The app reads the file from disk itself (**VERIFIED** `crates/wte_app/src/dispatch.rs:544-562` `add_stamp_image`:
  `std::fs::read(path)` then `add_image_blob(file_name, bytes)`). The path is **not** kept anywhere: no link to the
  file on disk after import (**VERIFIED**: `Blob` has only `name`; `StampObject` has no path).

### 1.3 Linked layers and the baked cache

- **VERIFIED** `crates/wte_core/src/layer.rs:27-35`: `LayerKind::{ Raster(Arc<PixelBuf>), Stamp { stamp, cache:
  Arc<SparseBuf> }, Text { stamp, cache } }`. A stamp owns one linked layer per texture set it covers (ARCHITECTURE
  1.2 D3). The layer holds the **baked result in UV/texture space** (a sparse cache), not the sticker picture.
- **VERIFIED** `layer.rs:46-61`: `Layer { id, name, visible, opacity, blend (7 modes, layer.rs:15-23), locked,
  alpha_lock, kind, version, content }`. No masks, groups, adjustment layers or layer effects exist.

### 1.4 Is the sticker non-destructive after placement? Yes (for the source image)

- **VERIFIED** `doc_stamps.rs:443-531` `set_stamp_source(id, source, commit)`: replaces the source and re-bakes all
  linked layers. Image -> Image: placement aspect re-fitted to the new image size (`refit_aspect`, ~`:503-508`),
  auto depth re-fitted. Kind switch Image <-> Text is refused (`InvalidParam`, ~`:455`). Locked stamp -> `LayerLocked`.
  `Commit::Preview` calls fold into one pending history entry; `Commit::Final` closes it (ARCHITECTURE 6.7 "Pending
  edits", lines 4071-4075).
- **VERIFIED** the app already exposes this: Tool page of a selected image stamp, row "Image: <name> [Choose...]"
  (`crates/wte_app/src/panels/tool_options.rs:646-655`) -> `AppAction::ReplaceStampImage(id, None)`
  (`action.rs:116`) -> file dialog -> `add_stamp_image` -> `set_stamp_source(id, Image{blob}, Final)`
  (`dispatch.rs:952-959`). So "edit the sticker image elsewhere and re-apply" is already supported as one undoable
  step, as long as the result arrives as encoded image bytes.
- **VERIFIED** `doc_layers.rs:316-347` `rasterize_layer`: converts a stamp/text layer to a raster layer (destructive;
  the stamp is removed when it has no linked layer left, `unlink_stamp` `:357-376`). Merge Down also unlinks.
- **INFERRED** consequence: a sticker editor should edit the *source image* (blob) and push the result through
  `set_stamp_source`; it never needs to touch the baked layer caches.
- **INFERRED** cost of live editing: each new edited picture would be a new blob (encoded bytes, kept for the
  whole session because blobs are never freed, `blob.rs:28-30`) plus a decode + mips build in `add_image_blob`. A live
  preview while dragging a slider would need a cheaper path (see gaps, section 9).

### 1.5 Text stickers

- **VERIFIED** `StampSource::Text(TextSpec)`; text engine in `crates/wte_core/src/text/` (layout, styled painter,
  warp, path, gaps, outline). `Document::text_preview(spec, max_pixels)` returns a `TextRaster`
  (`doc_stamps.rs:811`). Text effects already in WTE: outline, per-gap spacing, thickening (bold_em), slant, warp
  styles, text on arc/circle/curve (ARCHITECTURE 7.2, `TextFxEntry` lines 4365-4375).
- **VERIFIED** a text stamp cannot become an image stamp (`set_stamp_source` refuses the kind switch); "send a text
  sticker to PhotoCraft" therefore needs a new "convert text to image sticker" command (gap).

---

## 2. Paint layers and other layer data

- **VERIFIED** `layer.rs:27-30`: paint layer = dense `Arc<PixelBuf>` of exactly the set size, or `0x0` while never
  painted. Pixels are straight RGBA8 in texture (UV) space.
- **VERIFIED** core layer commands (`crates/wte_core/src/doc_layers.rs`): `add_raster_layer :51`, `remove_layer(s)
  :87/:116`, `duplicate_layer(s) :145/:184`, `move_layer :222`, `merge_down :245`, `rasterize_layer :316`, visibility /
  lock / alpha lock / blend / opacity / rename `:403-469`, `set_active_layer :487`, `layer_thumbnail :500`.
- **VERIFIED** (grep of `pub fn` in `wte_core/src`): there is **no** command to put external pixels into a paint layer
  (no "import image as layer", "replace layer pixels" or "paste"). Pixels only change through strokes
  (`stroke.rs`), fill (`fill.rs:234`) and undo.

---

## 3. Layer panel and multi-select

- **VERIFIED** `crates/wte_app/src/state.rs:960-962` `Session::selected_layers: Vec<LayerId>` (app state: not in the
  document, not in history, not saved); `Session::layer_selection()` `state.rs:1006-1012` normalises it (active layer
  always included). Click / Ctrl-click / Shift-click: `panels/layers.rs:27-39` `row_click_action`.
- **VERIFIED** row context menu `panels/layers.rs:41-60`: "Export selected layers... (N)" and "Export this layer..."
  only. Menu bar Layer menu `panels/menu_bar.rs:237-270` (New Paint Layer, Duplicate, Merge Down, Delete, Rasterize,
  Move Up/Down, Export Layer / Selected Layers).
- **VERIFIED** reordering is an in-window egui drag (`ui.dnd_drop_zone::<LayerId, ()>` `panels/layers.rs:213-241` ->
  `AppAction::MoveLayer`). It never leaves the window (egui DnD is in-process only).
- **VERIFIED** `Session::selected_stamp` (`state.rs:949`) is the stamp whose properties the Tool page shows
  (`tool_options.rs:578`); selecting a stamp makes its home-set layer active (`dispatch.rs:518-530`).

---

## 4. Project file `.wtep` (what is stored, versioning)

- **VERIFIED** ARCHITECTURE 7.1 (lines 4204-4325): ZIP with `mimetype`, `source/*` (original model + sidecars +
  chosen textures), `blobs/b<id:05>.<ext>` (stamp images and fonts, **original encoded bytes, only blobs referenced by
  a stamp**: `project/save.rs:277` `referenced_blobs`, `:316-325`), `sets/sNN/l<layer id>.png` (layer pixels: raster
  cropped to nonzero bounds; stamp/text caches cropped to their parts), optional `flattened.png`, `thumbnail.png`,
  `manifest.json` last.
- **VERIFIED** ARCHITECTURE 7.2 (lines 4327-4412): `StampEntry { id, name, home_set, tint, source:
  StampSourceEntry::{Image{blob}|Text(TextEntry)}, placement }`, `BlobEntry { id, kind, original_name, file }`,
  `ui_state: serde_json::Value` (app-owned, ignored by core).
- **VERIFIED** `crates/wte_core/src/project/mod.rs:22` `FORMAT_VERSION = 3`; `project/save.rs:116`
  `required_reader` (3 if any wrapped stamp / wrap option, 2 if text fx, else 1). ARCHITECTURE 7.3 (4414-4444):
  two-stage parse, `min_reader_version > FORMAT_VERSION` -> TooNew, newer format -> NewerFormat + forced Save As,
  `migrate::upgrade` chain. New optional fields must be `#[serde(default)]`; enum variant names never renamed.
- **VERIFIED** load step 5 (ARCHITECTURE 7.1 line ~4293): an image stamp whose blob is missing loads its layers as
  plain raster layers (no longer re-editable).
- **INFERRED** a per-stamp edit stack (non-destructive sticker edits) would be a new optional field on the Image
  source or the StampEntry -> FORMAT_VERSION 4 and `required_reader` 4 when used (older builds would otherwise drop
  the edits on save). Baking edits into a new blob instead needs **no format change** (the blob is just another
  image), but loses re-editability of the individual edits.
- A place for app-link metadata that does not need a version bump: `ui_state` (**VERIFIED** ignored by core,
  ARCHITECTURE 7.2 line ~4404) — but it is app UI state, not document data; a link record belongs in the manifest
  proper (INFERRED).

---

## 5. Import paths, drag-and-drop, clipboard

### 5.1 File dialog

- **VERIFIED** File > "Import Image as Stamp..." (`panels/menu_bar.rs:79`) and the Stamp tool's "Choose..."
  (`tool_options.rs:500-503`) -> `AppAction::ImportStampImage(None)` (`action.rs:114`) -> `io_flows::pick_image`
  (rfd, synchronous on the UI thread, ARCHITECTURE 1.2 D14) -> `add_stamp_image` -> sets the Stamp tool's source and
  switches to the Stamp tool; the user then clicks on the model / texture to place (`dispatch.rs:941-951`).

### 5.2 Drag and drop onto the window (files only)

- **VERIFIED** `crates/wte_app/src/main.rs:95-98`: `ViewportBuilder::with_drag_and_drop(true)` (single window; no
  egui multi-viewport anywhere: grep of `show_viewport|ViewportId` finds none).
- **VERIFIED** `crates/wte_app/src/dnd.rs:11-59`: reads `i.raw.hovered_files` / `i.raw.dropped_files` (paths only),
  draws a full-window "Drop to open" overlay, `plan_drop` (`:74-94`): one `.wtep` or model wins; images
  (`IMAGE_EXTENSIONS` `:101`: png jpg jpeg tga bmp tif tiff webp dds) become `AppAction::OpenPath` only when no
  model/project is in the drop. `OpenPath` of an image -> `ImportStampImage(Some(path))` (`dispatch.rs:609-620`).
- **VERIFIED** the drop position is not used: the image becomes the Stamp tool's source and the user clicks to place.
  **VERIFIED** `REG\winit-0.30.13\src\platform_impl\windows\drop_handler.rs:83,109,137`: winit ignores the drop point
  (`_pt`) and accepts only `CF_HDROP` (`:166`). So a drag that carries image data without a file (a browser image,
  `CF_DIB`, PNG stream, virtual files `CFSTR_FILEDESCRIPTOR`) is rejected by the cursor (DROPEFFECT_NONE).
- **INFERRED** while an OLE drag hovers, the window gets no mouse moves, so egui's pointer position is stale; a
  "drop at the cursor" placement needs our own `IDropTarget` (which gets the `POINTL`).

### 5.3 Clipboard

- **VERIFIED** ARCHITECTURE 1.3 (lines 66-69): no OS clipboard. eframe is built without its `clipboard` feature
  (`Cargo.toml` workspace deps: `eframe = { default-features = false, features = ["wgpu", "default_fonts",
  "accesskit"] }`); `REG\egui-winit-0.37.0\src\clipboard.rs:1-30` falls back to an in-app string clipboard.
  `arboard` is not in the registry (VERIFIED: `ls REG` has no `arboard*`).
- **VERIFIED** egui 0.37 has `Event::PasteImage` / `OutputCommand::CopyImage` (`REG\egui-winit-0.37.0\src\lib.rs:1149,
  1247`, `REG\egui-0.37.0\src\data\output.rs:137`), but they only reach the OS through arboard.
- **VERIFIED** WTE never copies or pastes anything (grep `copy_text|Event::Paste|CopyImage|clipboard` in `wte_app/src`:
  only GPU `copy_texture_to_buffer`). No Ctrl+C / Ctrl+V shortcuts (`shortcuts.rs`). Edit menu = Undo, Redo, Delete
  (`menu_bar.rs:129-136`).
- **VERIFIED** for contrast: PhotoCraft v0.2.0 uses arboard for image copy / paste
  (`PC\apps\photocraft\src\services.rs:136-140`, `apps/photocraft/Cargo.toml:32 arboard = "3"`), and opens dropped
  files as documents (`PC\crates\ui-egui\src\file_open.rs:78-88`, via `crates/ui-egui/src/lib.rs:664`). A grep of v0.2.0
  for `DoDragDrop|RegisterDragDrop` finds nothing in PhotoCraft's own code (it inherits winit's CF_HDROP-only target;
  INFERRED it cannot be a drag *source* to other apps). Details belong to the PhotoCraft notes.

---

## 6. Export

- **VERIFIED** ARCHITECTURE 1.8 / 7.4 / 7.5 (lines 175-202, 4446-4564): per set a flattened texture (PNG/JPEG/TGA/
  BMP/TIFF/WebP), per-layer **full-canvas** PNGs (`<base>[_<set>]_L<nn>_<kind>_<name>.png`), ORA and PSD; layer scope
  `ExportOptions::layers = Some(ids)` (one layer, a selection; `export::linked_selection` `export/mod.rs:125` adds the
  other sets' layers of the same stamp); `ExportPackage::Zip` (`<base>.zip` / `<base>_selection.zip`, deterministic,
  atomic). `export::run` `export/mod.rs:604`, ORA `export/ora.rs:122`, PSD `export/psd.rs:170`.
- **VERIFIED** `export/psd.rs:520` `read_psd` is `#[doc(hidden)]`, a strict reader for tests of WTE's own PSDs; there
  is no ORA reader. So WTE can write layered files other editors open but cannot read layered files back.
- **VERIFIED** exports are UV-space textures. The sticker's *source picture* is only written by File > "Extract
  Original Files" (blobs into `stamps\`, ARCHITECTURE 7.1 `extract_bundle`, line ~4318). There is no "export / copy
  this sticker's image" command.

---

## 7. Undo system

- **VERIFIED** ARCHITECTURE 6.7 (lines 4043-4099) and `crates/wte_core/src/history.rs`: two edit kinds, `Pixels`
  (64x64 tile diffs, `TilePatch` / `TileBackup` `history.rs:97-263`) and `Structure` (snapshot of layers + stamps,
  raster pixels pinned only for layers the command replaces). Swap semantics; pending `Commit::Preview` edits folded
  into one entry; budget-based eviction; history state ids for the modified flag.
- **VERIFIED** helpers: `group_edits(label, f)` (`history.rs:782`, several commands = one undo step),
  `push_structure_edit` (`:806`), `snapshot_structure(pin)` (`:651`).
- **INFERRED** a "replace paint layer pixels" command fits as `Structure` with the layer pinned (like
  `rasterize_layer`, `doc_layers.rs:330-346`) or as a `Pixels` edit through `TileBackup` (smaller undo cost when only
  part changed). A sticker image replacement already is one "Edit Stamp" entry (`set_stamp_source`).

---

## 8. Existing 2D image operations in wte_core (reusable for a sticker editor)

- **VERIFIED** `pixel.rs`: `PixelBuf` (`:8-90`: new, filled, from_raw, get/set, rows, view), `CroppedBuf` (`:96-160`),
  `SparseBuf` (`:167-360`), `PixelView` (`:405-430`).
- **VERIFIED** `pixel_ops.rs`: `premultiply`/`unpremultiply :15/:21`, `content_bounds :64`, `nonzero_bounds :69/:74`,
  `crop :92`, `crop_to_content :107`, `resize :368` (area/bilinear resample), `thumbnail :468`, `dilate_islands :478`.
- **VERIFIED** `image_io.rs`: `decode_image :74` (png jpeg tga bmp tiff webp dds via `image` 0.25.10, limits),
  `encode_png :144`, `encode_jpeg :165`, `encode_flat :230`.
- **VERIFIED** `blend.rs:89/:96` `blend_pixel` / `blend_row` (7 modes); `composite.rs:101/:107` `composite_rect` /
  `flatten_layers` (works on any `&[Layer]`, so a small sticker layer stack could reuse it).
- **VERIFIED** brush kernels usable on a plain buffer: `brush.rs:398 falloff`, `:436 apply_brush`, `:518
  StrokeScratch::new`, `:1830 dab_2d`. The full stroke API (`stroke.rs:373-741`) is bound to `Document` texture sets.
- **VERIFIED** text rendering to a picture: `Document::text_preview` (`doc_stamps.rs:811`), `text/raster.rs`,
  `text/styled.rs`.
- **VERIFIED** what is missing for photo-style editing (grep of `wte_core/src`): no colour adjustments
  (levels/curves/HSL/brightness), no blur/sharpen/filters, no selection or mask model, no transform-resample of a
  sub-region, no flood fill by colour tolerance (non-goal, ARCHITECTURE 1.3), no background removal, no outline /
  drop-shadow for images (text has outline only).

---

## 9. UI structure and where a "Sticker editor" fits

- **VERIFIED** frame layout `crates/wte_app/src/app.rs:361-404`: top menu `Panel::top("menu")`, status bar, left tool
  strip (44 pt), right side panel (`panels::show_side_panel`), central area (`view::central`), then
  `dialogs::show`, toasts, `dnd::handle`, shortcut actions, and `dispatch` of every queued `AppAction`.
  `raw_input_hook` `app.rs:297-315` (self-test injection, pen records, companion input, pen router).
- **VERIFIED** side panel = two tab groups (`panels/side_groups.rs:1-16`): upper `UpperTab::{Tool, Color,
  TextureSets, Objects}` (`state.rs:489-495`), lower `SideTab::{Layers, Brush, Text}` (`state.rs:441-446`), draggable
  split; layout guarantees G1-G4 tested in `control_sweep/panel_tests.rs` and the `side_tabs` scenario.
- **VERIFIED** central area modes `state.rs:33` `ViewMode { View3D, View2D, Split, Preview }`, switched in
  `view/mod.rs:80-83` (mode strip on top, `view/mod.rs:64-66`). `ViewId { Main3D, Main2D, Preview, ExportPreview }`
  (ARCHITECTURE 8.2 line 4691).
- **VERIFIED** dialogs: `state.rs:1127-1143` `enum Dialog { None, Import, Unsaved, Export, Message, About, Shortcuts,
  TabletHelp, Companion, CloseOpenings }`, drawn by `dialogs/mod.rs:46-77` as `egui::Modal`s; dialogs only push
  actions. Menus: `panels/menu_bar.rs:25-30` File / Edit / View / Layer / Object / Help.
- **VERIFIED** egui textures for small pictures are uploaded with `ctx.load_texture` (`panels/mod.rs:295-305`,
  `dialogs/import.rs:63`), so a sticker canvas can be a plain egui image (no wgpu target needed) — INFERRED.
- **INFERRED** options for the editor surface:
  1. A new `ViewMode::Sticker` (central area): full-size canvas of the selected sticker's source image with its own
     tools; the side panel's Tool page shows the sticker tool options; the 3D view could stay visible in a Split-like
     layout to show the re-baked result live. Best fit for the one-window app and the self-test runner (views have
     `ViewId`s that `Step::Pointer` can target).
  2. A modal `Dialog::StickerEditor` (simplest; but modal dialogs gate the views' input, `view/mod.rs:75-78`, so no
     live interaction with the model while open).
  3. A second OS window via egui viewports: not used anywhere today; would complicate focus (the user's rule: never
     steal focus) and the self-test runner (one window). Not recommended.
- **VERIFIED** self-test system: `selftest/scenarios.rs:38` `NAMES` (54 scenarios), `Step` enum
  `selftest/mod.rs:47-100` (`Do(AppAction)`, `Pointer`/`Click` in a `ViewId`, `ScreenPointer`/`ScreenClick`, `Key`,
  `Pen`, `Setup`/`Assert`/`Dyn`/`Until`, `Shot`, ...). `raw_input_hook` (`selftest/mod.rs:260-290`) drops all real
  input and clears `hovered_files` / `dropped_files`: drops are tested via `AppAction::OpenPath`, and dnd logic via a
  fake `egui::DroppedFile` in unit tests (`dnd.rs:123-162`). Runs are off-screen, never focused (`main.rs:99-110`).

---

## 10. Platform code already present (no new crates)

- **VERIFIED** `raw-dylib` FFI is the established pattern: `pen/win.rs:97-109` (user32: Get/SetWindowLongPtrW,
  CallWindowProcW, GetPointerPenInfo(History), SetWindowFeedbackSetting, SetPropW...), `crash.rs:304,333,346`
  (kernel32, dbghelp, user32), `gpu_check.rs:181`, `main.rs:209`, `companion/bt_win.rs:98` (ws2_32),
  `companion/crypto.rs:268` (bcrypt), `pen/wintab.rs:197` (kernel32 LoadLibrary for wintab32).
- **VERIFIED** HWND access: `pen/mod.rs:264-279` takes it from `cc.window_handle()` (`RawWindowHandle::Win32`) at
  startup; `pen/win.rs:198-240` installs a `GWLP_WNDPROC` subclass that forwards everything to winit's procedure,
  panics caught (`catch_unwind`, `pen/win.rs:9-10`).
- **VERIFIED** processes: `companion/adb.rs:25-26,116-121` spawns `adb` with `CREATE_NO_WINDOW`, stdin null, piped
  output (pattern for running `photocraft-cli.exe` without a console flash).
- **VERIFIED** companion server = in-process TCP server with its own protocol, threads, `Hub` (`companion/mod.rs:1-20`,
  `server.rs`); port 47800. A model for a local IPC channel, but PhotoCraft cannot be changed, so an IPC link to
  PhotoCraft is not possible without forking it (INFERRED).

### 10.1 OLE drag source / drop target / clipboard without new crates (INFERRED design, facts VERIFIED where marked)

- **VERIFIED** winit already calls `OleInitialize` and `RegisterDragDrop(hwnd, FileDropHandler)` when drag-and-drop
  is on (`REG\winit-0.30.13\src\platform_impl\windows\window.rs:1167-1194`); its handler is 237 lines of hand-written
  COM vtables (`drop_handler.rs`), a template for ours. The UI thread is therefore an OLE STA (required by
  `DoDragDrop` and `OleSetClipboard`).
- **Drop target (other formats + drop point)**: `RevokeDragDrop(hwnd)` then `RegisterDragDrop(hwnd, our IDropTarget)`
  after window creation (or `with_drag_and_drop(false)` and call `OleInitialize` ourselves). Our target accepts
  `CF_HDROP` (paths, as today), `CF_DIBV5`/`CF_DIB`, the registered "PNG" format, and optionally
  `FileGroupDescriptorW`+`FileContents`; it records the `POINTL` (screen -> client -> egui points via
  `pixels_per_point`) and queues results for `dnd::handle` (same queue pattern as `pen/win.rs` `SHARED`). Imports:
  ole32 (`RegisterDragDrop`, `RevokeDragDrop`, `ReleaseStgMedium`), shell32 (`DragQueryFileW`), kernel32
  (`GlobalLock`, `GlobalUnlock`, `GlobalSize`), user32 (`RegisterClipboardFormatW`, `ScreenToClient`).
- **Drag source (WTE -> PhotoCraft / Explorer)**: an `IDataObject` + `IDropSource` and `DoDragDrop`. Because
  PhotoCraft v0.2.0 opens only dropped *files* (CF_HDROP via winit), the data object must offer `CF_HDROP` pointing to a
  temp PNG written first (e.g. `%TEMP%\wte-link\<stamp>.png`); adding `CF_DIBV5`/"PNG" helps other apps. `DoDragDrop`
  runs a modal message loop until the drop; start it from an egui drag on a layer row / sticker thumbnail when the
  pointer leaves the window rect. Risk (INFERRED): re-entrancy of winit's event loop inside `DoDragDrop` called from
  `App::ui`; safer to start it from `App::logic` of the next frame and keep the frame loop idle meanwhile.
- **Clipboard**: user32 `OpenClipboard`/`EmptyClipboard`/`SetClipboardData`/`GetClipboardData`/
  `IsClipboardFormatAvailable`/`RegisterClipboardFormatW` + kernel32 `GlobalAlloc` is enough for copy / paste of
  images (`CF_DIBV5` + "PNG"), no COM needed. Encode / decode of DIBV5 (BGRA, bottom-up, alpha mask) is pure Rust and
  unit-testable.
- **Alternative**: `windows-sys 0.52.0` is already compiled into the app through winit with `Win32_System_Ole`,
  `Win32_System_Com`, `Win32_UI_Shell`, `Win32_Graphics_Gdi` (`REG\winit-0.30.13\Cargo.toml:525-552`), but **not**
  `Win32_System_DataExchange` / `Win32_System_Memory` (clipboard, GlobalAlloc). Making it a direct dependency adds no
  download but is a new direct dependency -> needs the user's OK under the no-new-crates rule (HANDOFF.md section 3
  item 3). raw-dylib keeps the rule untouched.
- **Testing constraints (INFERRED from the user's rules)**: no OS input injection and no focus stealing, so real
  drags cannot be automated. Unit tests can call our `IDropTarget::Drop` vtable with an in-process fake `IDataObject`,
  and test `IDataObject::GetData` of the drag source directly. `OleSetClipboard` / `SetClipboardData` in tests would
  overwrite the **user's real clipboard**: keep clipboard tests to the pure codec and leave the OS round trip to a
  hand test. The self-test runner must not install the OLE target (or must gate it like `WTE_PEN_SUBCLASS`).

---

## 11. Concrete integration points

| Need | Where it plugs in |
|---|---|
| Edit a placed sticker's image and re-apply | Produce PNG bytes -> `Document::add_image_blob` (`doc_stamps.rs:211`) -> `set_stamp_source(id, StampSource::Image{blob}, Commit::Final)` (`doc_stamps.rs:443`); app action exists: `AppAction::UpdateStampSource` (`action.rs:118`, `dispatch.rs:962`) / `ReplaceStampImage` (`dispatch.rs:952`) |
| Edit a sticker *before* placing | Stamp tool source `state.tools.stamp.source: Option<BlobId>` + `source_name` (`dispatch.rs:948-949`); edit -> new blob -> replace `source` |
| Open the editor | "Edit sticker..." next to "Image: Choose..." (`tool_options.rs:646-655`), a Layers row context item (`panels/layers.rs:41-60`), Layer menu (`menu_bar.rs:237-270`); new `AppAction` variants (`action.rs`), dispatched in `dispatch.rs` |
| Editor surface | new `ViewMode` arm in `view/mod.rs:80-83` (+ `ViewId`) or new `Dialog` variant (`state.rs:1127`, `dialogs/mod.rs:46-60`) |
| Image ops | new module in `wte_core` next to `pixel_ops.rs` (pure functions on `PixelBuf`, headless-testable); reuse `resize`, `crop`, `blend_row`, `dab_2d`/`apply_brush`, `decode_image`/`encode_png`, `text_preview` |
| Non-destructive edit stack (optional) | new field on `StampSource::Image` / `StampEntry` (`stamp.rs:10-14`, manifest 7.2), cache key in `image_mips` (`document.rs:70`, `doc_stamps.rs:1028`) becomes (blob, edits); `FORMAT_VERSION` 4, `required_reader` (`save.rs:116`), `migrate.rs` v3->v4 |
| Paint layer round trip | new core command "replace/import raster layer pixels" in `doc_layers.rs` (Structure edit pinning the layer, like `rasterize_layer :316`) + `AppAction`; full-canvas PNG export of a layer already exists (7.4) |
| Drop image files (today) | `dnd.rs:74-94` `plan_drop` (add "drop on a selected sticker = replace its image"), `dispatch.rs:609-620` |
| Drop non-file image data / drop point | new `crate::ole` (or `platform/win_ole.rs`) module, installed next to `pen::install` (`pen/mod.rs:264`), feeding a queue read by `dnd::handle` |
| Drag out to PhotoCraft / Explorer | egui drag on a layer row (`panels/layers.rs:213-241` already makes rows draggable) or the sticker thumbnail; when the pointer leaves the window -> write temp PNG -> `DoDragDrop` with CF_HDROP |
| Copy / paste image | new Edit menu items + Ctrl+C / Ctrl+V in `shortcuts.rs`; clipboard via user32 raw-dylib; paste = new image sticker (Stamp tool source) or replace selected sticker image |
| "Edit in PhotoCraft" (external editor) | write blob bytes to a temp file, spawn the user's `photocraft.exe <file>` (`Command`, as `companion/adb.rs:116`), poll the file's mtime in `App::logic` (`app.rs:317`), on change `add_image_blob` + `set_stamp_source(Final)`; path of the exe in `Prefs` (`state.rs:1146`) |
| Tests | unit tests in `wte_core` for every image op; app tests via `control_sweep` and a new scenario in `selftest/scenarios.rs` `NAMES` (`:38`) driving `AppAction`s; OLE/clipboard codec unit tests only |

## 12. Gaps (what does not exist today)

1. No sticker image editing at all: only tint, flips, placement; no crop, erase, paint, adjust, filter, outline,
   shadow, background removal on the source image (VERIFIED, sections 1.1, 8).
2. No API to put external pixels into a paint layer or create a layer from an image (VERIFIED, section 2).
3. No OS clipboard (VERIFIED, 5.3); no copy/paste of layers or stickers even inside the app.
4. Drops: files only (CF_HDROP), no drop position, images only become the Stamp tool source (VERIFIED, 5.2).
5. No drag *out* of the window (VERIFIED: egui DnD only; no DoDragDrop anywhere in WTE).
6. No link to the original file of a sticker (path not stored; VERIFIED 1.2) and no file watching; a "linked" sticker
   that follows an external file needs a new stored field (path + last mtime/crc) -> manifest change (INFERRED).
7. Text stickers cannot be turned into image stickers (VERIFIED `set_stamp_source` kind rule) -> needed to send text
   designs to PhotoCraft.
8. No reader for layered files (PSD reader is test-only, no ORA reader; VERIFIED 6): bringing a multi-layer
   PhotoCraft document back means flattened PNGs or a new reader.
9. Live preview of edits: `add_image_blob` encodes/decodes and blobs are never freed (VERIFIED 1.2); a live editor
   needs an in-memory "draft source" path (e.g. temporary mips not backed by a blob until Commit::Final) (INFERRED).
10. Exports are UV-space textures; the sticker's own picture is only reachable via "Extract Original Files"
    (VERIFIED 6). A "Copy / Export sticker image" command is missing.
11. HANDOFF.md section 6 records an earlier user decision "No image editor needed for PSD / ORA" (it was about
    validating exports); the new request supersedes it for stickers — worth confirming scope with the user (INFERRED).
