# Export: layer selection and single-package (.zip) export — plan

Branch `export-select` (off `round9-wip` @ a8beea6). Planned by reading code and docs only; nothing was built or run.
Line numbers refer to that commit.

Goal (the user's definition of "works as intended"): **import a model -> layer designs onto the material -> export
either one layer, a selection of layers, or everything in a single package.**

Hard limits: no new crates; `FORMAT_VERSION` stays 3 and old projects load unchanged; with default options the exporter
behaves exactly as today (the `export_run` scenario's files stay byte-identical); the app is driven only through the
headless runner.

---

## 1. What is there today (verified)

| Fact | Where |
|---|---|
| `ExportOptions { dir, base_name, sets, flat, per_layer_png, ora, psd, include_hidden, dilate_px }`, no `Default`, no serde | `crates/wte_core/src/export/mod.rs:34` |
| `build_plan` -> `plan` (paths only) -> `run` / `run_in_lanes`; several sets in parallel lanes; each file temp + rename; one flush pass at the end | same file, 96-497 |
| Per-layer file index = position in the full stack, also when hidden layers are left out | `exported_layers`, `mod.rs:90`; test `per_layer_pngs_are_full_canvas` |
| `zip` 9.5.1 (`default-features = false, features = ["deflate"]`) is a `wte_core` dependency; **not** a `wte_app` dependency | `Cargo.toml:24`, `crates/wte_core/Cargo.toml` |
| Without the `time` feature `DateTime::default_for_write()` is 1980-01-01, so archives are already deterministic (`.wtep`, ORA) | `REG\zip-9.5.1\src\datetime.rs:198-202`; `project/save.rs` test `saving_is_deterministic_and_a_resave_keeps_the_manifest` |
| `zip_stored()`, `zip_deflated()` (level 6), `source_options(ext)` (stored for png/jpg/jpeg/webp) | `crates/wte_core/src/project/save.rs:35-65` |
| Export dialog: options, cached plan (`PlanCache`), "will be replaced" markers, confirm step; `classify_plan` **repeats the core's layer filter in the app** and falls back to guessing from file names with only a `log::warn` | `crates/wte_app/src/dialogs/export.rs` |
| `ExportDialogState` and `ExportOptions` are **not persisted** anywhere. Only `Prefs::last_export_dir` is. `UiState` (project `ui_state`) holds cameras, view mode, active set, colours | `state.rs:793`, `state.rs:825`, `io_flows.rs:38` |
| The active layer lives in the document: `TextureSet::active`, set by `Document::set_active_layer` (no history, no revision bump) | `doc_layers.rs:419`, test `set_active_layer_is_not_history` |
| App-side selection today: `Session::selected_stamp` only; `AppAction::SelectLayer` sets active set, active layer and the selected stamp | `state.rs:722`, `dispatch.rs:717` |
| Layer rows: a `click_and_drag` row widget with the controls on top; three click sites (row, thumbnail, name) push `SelectLayer`; **no context menu anywhere on a row** | `panels/layers.rs:233-325` |
| Scenarios drive the app with `Step::Do(AppAction)`, `Step::ScreenPointer { path }` (primary button, no modifiers) at `state.panel_rects`, `Step::Click` / `Pointer` in a view (with button + modifiers), `Step::Key`, `Setup` / `Assert` / `Dyn` | `selftest/mod.rs:48-84` |
| Importers: OBJ, glTF, GLB, FBX (ASCII and binary) | `crates/wte_core/src/import/{obj,gltf,fbx}.rs`, `mod.rs:94-113` |
| Models on this machine: six Khronos **GLB** files (`samples\`), OBJ shapes plus one `.gltf` in `test-assets\shapes\`. **No FBX file exists on disk.** Scenarios can write a cube as OBJ / glTF / GLB / ASCII FBX (`samples::to_obj`, `to_gltf`, `to_glb`, `to_fbx_ascii`; done by `bs_write_assets` for brush shapes) | `samples.rs:386,1255,1262,1287`; `scenarios.rs:3925` |

---

## 2. Decisions

1. **Selection is app state, not document state.** `Session::selected_layers: Vec<LayerId>`. Not in the document, not in
   history, no revision bump, not in `UiState`, not in `Prefs`. Nothing about the project file changes.
2. **Active vs selected.** The active layer (the one tools paint on, `TextureSet::active`) is always part of the
   selection. Plain click = active + the only selected one. Ctrl-click toggles without changing the active layer
   (except when the active layer itself is toggled off: the top-most remaining selected layer becomes active; the last
   selected layer cannot be toggled off). Shift-click selects the range from the active layer to the clicked one; the
   active layer stays. The selection is limited to the active texture set (the panel shows only that set).
   Every existing layer command (Delete, Duplicate, Merge Down, Rasterize, opacity, blend, move, the Delete key) keeps
   acting on the **active layer only**. Multi-selection is used by export only in this feature.
3. **Layer scope in the exporter = a new field** `layers: Option<Vec<LayerId>>`. `None` = today's behaviour
   (`include_hidden` decides). `Some(ids)` = exactly those layers, hidden ones included; `include_hidden` is ignored.
4. **Flattened texture / ORA / PSD under a layer selection = the selected layers only** (recommendation).
   - Flattened = `flatten_layers` of the selected layers, bottom to top, with their own opacity and blend, **visible ones
     only** — the same rule as today ("flattened always = visible only"). A selected hidden layer is written as its
     per-layer PNG and as a hidden layer inside ORA / PSD, and does not contribute to the flattened picture.
   - Why not the full visible composite: (a) ORA and PSD carry a merged image that must equal their own layer stack,
     so a file with three layers and a merged picture of seven would be inconsistent; (b) the full composite is
     already what scope Visible / All gives, so repeating it adds nothing, while "these decals on transparent" is a
     useful result nothing else gives; (c) one rule for all three outputs is easier to explain in the dialog.
   - Consequence handled in the dialog: opened for a selection, the defaults are per-layer PNGs on and flattened / ORA
     / PSD off, and the preview caption says the preview shows the whole texture (section 5.3).
5. **Package = `<base>.zip`** in the output folder, holding the same files under the same names (flat list, no
   folders). Built by exporting into a staging folder next to the zip with the unchanged exporter, then packing the
   files in plan order (section 4). Entries are byte-identical to the loose files by construction, the sets keep
   exporting in parallel, and entry order does not depend on timing.
6. **Stored vs deflate.** PNG, JPEG, WebP and ORA entries are stored (already compressed; deflating them costs time
   for under 1 %). PSD, TGA, BMP and TIFF entries are deflated at level 6 (PSD is only run-length coded; on
   ABeautifulGame it is 260 of 326 MB). Both are deterministic.
7. **The dialog's "Include hidden layers" checkbox becomes a three-way "Layers" choice**: Visible / All /
   Selected (N). Visible and All map to `include_hidden` false / true as before.

---

## 3. Core changes (`crates/wte_core`)

### 3.1 `src/export/mod.rs`

```rust
/// How the output is written.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ExportPackage {
    /// Loose files in `dir` (as before).
    #[default]
    Files,
    /// One `<base>.zip` in `dir` holding those files.
    Zip,
}

pub struct ExportOptions {
    // ... the nine existing fields, unchanged ...
    /// Layer scope. `None` = every layer that `include_hidden` lets through (as before). `Some(ids)` = exactly these
    /// layers, hidden ones included; `include_hidden` is ignored; ids that do not exist are ignored; a set that holds
    /// none of them is left out completely.
    pub layers: Option<Vec<LayerId>>,
    pub package: ExportPackage,
}

pub struct ExportReport {
    pub files: Vec<PathBuf>,       // Files: the written files. Zip: the one zip.
    pub bytes_written: u64,        // Zip: size of the zip.
    /// Zip: number of files inside. Files: 0.
    pub packaged: usize,
}

/// One file of the export as the dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFile { pub name: String, pub set: TextureSetId, pub kind: PlannedKind }
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlannedKind { Flat, Layer { layer: LayerId, index: usize, raster: bool }, Ora, Psd }

/// The files of the export in plan order, whatever the package (file names only).
pub fn plan_contents(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PlannedFile>;
/// The paths written to disk: `Files` = `dir.join(name)` of every planned file (as before); `Zip` = the one zip
/// path, or nothing when there is nothing to pack.
pub fn plan(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PathBuf>;
/// Test and self-test helper: the entries of a package in archive order.
#[doc(hidden)] pub fn read_package(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String>;
```

Edits, in order:

1. `exported_layers(set, opts)` takes the options instead of `include_hidden`:
   `match &opts.layers { None => include_hidden || layer.visible, Some(ids) => ids.contains(&layer.id) }`. All five
   call sites (`build_plan`, `export_set` twice, `sets_in_flight`, the `OraReuse` list) pass `opts`. The returned
   index stays the position in the full stack, so a selected layer's file has **the same name** as in a full export.
2. `selected_sets`: with `layers: Some(..)`, drop the sets in which `exported_layers` is empty. `multi` (the
   `_<set>` name part) is computed from what remains, as today.
3. `build_plan` unchanged apart from (1) and (2). `plan_contents` maps its items to `PlannedFile`. `plan` returns the
   item paths for `Files` and `[dir.join(package_name(base))]` for `Zip` when there is at least one item.
4. `export_set`: when `opts.layers.is_some()`, the flattened picture is
   `flatten_layers(&selected, w, h, false)` where `selected: Vec<Layer>` holds clones of the exported layers (cheap:
   pixels are behind `Arc`). With `None` the existing line runs unchanged (`flatten_layers(&set.layers, ..)`), so the
   default path does not change by a single instruction that matters. `dilate_islands` applies as before.
   Nothing else in `export_set` changes: `ExportSet::layers`, `OraReuse::full_canvas_layers` and `layer_pngs` already
   go through `exported_layers`.
5. `run` dispatches on `opts.package`: `Files` -> `run_in_lanes` exactly as today; `Zip` -> `run_zip` (section 4).
6. `ExportReport.packaged` (0 for `Files`).

Blend modes over nothing: a selected Multiply layer without a backdrop composites over transparent, as `flatten_layers`
defines it. No special case; stated in ARCHITECTURE 7.4.

### 3.2 `src/export/naming.rs`

`pub fn package_name(base: &str) -> String` = `sanitize_file_stem(base) + ".zip"`. Exported next to `flat_name`.

### 3.3 `src/lib.rs`

Re-export `ExportPackage`, `PlannedFile`, `PlannedKind` next to `ExportOptions` (line 92).

### 3.4 Call sites that build `ExportOptions` by literal (add `layers: None, package: ExportPackage::Files`)

`export/mod.rs` tests `options()` (the other 22 uses are `..options(&dir)` updates), `tests/end_to_end.rs` (3 literals;
line 490 is an update), `tests/project_roundtrip.rs` (1), `wte_app/src/dispatch.rs:327`,
`wte_app/src/dialogs/export.rs` tests `options()`, `wte_app/src/selftest/scenarios.rs:2055`.

---

## 4. The ZIP writer (`run_zip` in `export/mod.rs`, about 100 lines)

```
zip_path = dir.join(package_name(base))
stage    = dir.join("<zip name>.tmp-<pid>.d")      // a folder; removed by a guard on every exit
tmp_zip  = temp_path_for(&zip_path)                // "<zip name>.tmp-<pid>"
```

1. `build_plan`; empty => `NothingToExport` (nothing created).
2. `create_dir_all(dir)`, create `stage`. A `StageDir` guard (`Drop` = `remove_dir_all`) covers success, failure,
   cancellation and panic.
3. Run the existing exporter into `stage`: `run_in_lanes(snapshot, &ExportOptions { dir: stage, package: Files,
   ..opts.clone() }, &scaled_progress, usize::MAX)` with a new private flag that **skips the `finish` flush pass**
   (staged files are thrown away; the zip is flushed instead). `scaled_progress` maps 0..1 to 0..0.6, stage
   "Exporting", and forwards `cancelled()`.
4. Pack, in plan order: `ZipWriter::new(BufWriter::with_capacity(1 << 20, File::create(tmp_zip)))` under the existing
   `TempFile` guard. Per staged file: entry name = file name; options = `package_options(ext)` — `zip_stored()` for
   `png | jpg | jpeg | webp | ora`, `zip_deflated()` otherwise — with `.last_modified_time(zip::DateTime::default())`
   set explicitly (so a later `time` feature somewhere in the tree cannot make archives differ) and
   `.large_file(true)` only when the staged file is larger than `u32::MAX` bytes; `start_file`, `std::io::copy`,
   then delete the staged file (keeps the extra disk use near one file, not the whole export). Cancellation is
   checked between entries. Progress 0.6..0.95, stage "Packing".
5. `zip.finish()`, flush the `BufWriter`, `sync_all`, close, `rename(tmp_zip, zip_path)`, keep the guard. Progress 1.0,
   stage "Finishing".
6. Report: `files = [zip_path]`, `bytes_written` = zip size, `packaged` = entry count.

Properties:
- **Atomic as a whole**: until the rename there is no `<base>.zip`; an existing one is replaced only by a complete new
  one. A cancelled or failed zip export leaves nothing behind (stricter than loose export, where finished files stay).
- **Deterministic**: fixed timestamps, no comment, no extra fields, entry order = plan order, deterministic PNG and
  deflate encoders. Two exports of the same document are byte-identical (tested).
- **Entries = loose files**: they are the loose files, produced by the same code.
- **Cost**: the export is written twice (stage, then zip). The stage lives in the output folder, so it is on the same
  drive and never on C: unless the user exports there. Peak extra disk = about the size of the export.

Rejected: writing entries straight into one `ZipWriter` from the lanes. A single stream forces either one lane
(ABeautifulGame: about 31 s of encoding on one thread against about 3 s in lanes, 7.4) or a reorder buffer holding
whole PSDs in memory; and the ORA writer needs `Seek`, so it would need a spool file anyway.

To check while building (not verified by reading alone): `SimpleFileOptions::large_file` and
`last_modified_time` signatures in `REG\zip-9.5.1\src\write.rs`; that the crate switches the central directory to
ZIP64 by itself when the archive passes 4 GiB.

---

## 5. App changes (`crates/wte_app`)

### 5.1 `src/state.rs`

- `Session` gains `pub selected_layers: Vec<LayerId>` (raw, may be stale) and

  ```rust
  /// The selected layers of the active set, bottom to top. Always normalised on read: ids that are gone or belong to
  /// another set are dropped; if the active layer is not among them the selection is just the active layer; no active
  /// layer => empty.
  pub fn layer_selection(&self) -> Vec<LayerId>;
  ```

  Normalising on read needs no hook: whatever changes the active layer (placing a stamp, New, Delete, Undo, Redo,
  `SetActiveSet`, a click on a stamp in a view) collapses the selection by itself. All readers use
  `layer_selection()`; only dispatch writes `selected_layers`.
  The four `Session { .. }` literals (`io_flows.rs:564`, three in tests) get `selected_layers: Vec::new()`.
- `ExportDialogState` gains `pub selected_layers: Vec<LayerId>` = the selection captured when the dialog opened
  (what "Selected (N)" refers to while the user switches scope back and forth).
- `PanelHit` gains `LayerMenuExportThis` and `LayerMenuExportSelected` (rects of the open context menu's items, for
  the self-test).

### 5.2 `src/action.rs`, `src/dispatch.rs`

New actions (all drivable with `Step::Do`):

```rust
/// Ctrl-click on a layer row.
ToggleLayerSelected(LayerId),
/// Shift-click on a layer row: the range from the active layer to this one.
SelectLayerRange(LayerId),
/// Open the Export dialog with the layer scope set to these layers.
ShowExportLayers(Vec<LayerId>),
```

Dispatch:
- `SelectLayer(l)`: as today, plus `selected_layers = vec![l]`. Body moved into `fn activate_layer(session, l)`.
- `ToggleLayerSelected(l)`: `l` outside the active set => same as `SelectLayer(l)`. Otherwise with
  `sel = layer_selection()`: not in `sel` => add; in `sel` and `sel.len() == 1` => nothing; in `sel` and it is the
  active layer => remove it and `activate_layer` the top-most remaining one (keeping the rest selected); else remove.
- `SelectLayerRange(l)`: no active layer or another set => `SelectLayer(l)`; else the ids between the two stack
  indices, inclusive; active layer unchanged.
- None of the three calls a document edit other than `set_active_layer`: no history entry, no revision bump, the
  modified mark does not change. They are **not** in `needs_interaction_guard` (like `SelectLayer`, line 53).
- `ShowExportLayers(ids)`: the `ShowExport` body (cancel tool, flush pending edit, `stamp_bakes::settle`) then
  `export_defaults` adjusted: `layers = Some(ids in stack order)`, `sets = Some([their set])`, `flat = None`,
  `per_layer_png = true`, `ora = psd = false`; `selected_layers` = the ids. Added to both lists that name
  `ShowExport` (`dispatch.rs:63` and `:106`).
- `ShowExport`: unchanged defaults (`layers: None`, `package: Files`); `selected_layers = session.layer_selection()`
  so "Selected (N)" can be chosen there too.

### 5.3 `src/dialogs/export.rs`

- "Include hidden layers" checkbox -> a row **Layers**: `Visible` | `All` | `Selected (N)` (three
  `selectable_label`s; N = `st.selected_layers.len()`; with N = 0 the third is disabled, hover "Select layers in the
  Layers list first"). Writes `options.include_hidden` and `options.layers` (`Some(st.selected_layers.clone())` for
  Selected, else `None`). Pure helper `scope_of(&ExportOptions) -> LayerScope` + `apply_scope(..)`, unit-tested.
- While the scope is Selected: a weak line under it naming the layers ("Logo, WTE" — elided after three: "Logo, WTE
  and 2 more"), and the texture-set checkboxes are hidden (the set is given by the layers).
- New checkbox under the format rows: **"Pack into one .zip file"** -> `options.package`.
- File list: built from `export::plan_contents` (names and kinds) and `export::plan` (what is checked on disk).
  `classify_plan` and `kind_from_name` are **deleted**; `FileKind` is mapped from `PlannedKind`
  (`Layer { raster: false }` => "separate layer"). `PlanCache` holds `contents`, `disk` (paths) and `exists` per disk
  path.
  - `Files`: header and rows as today.
  - `Zip`: header "1 file will be written: `<base>.zip` (N files inside)" with the "will be replaced" marker on that
    line, then the N entry names as today's rows without markers.
- Confirm step unchanged (it lists `existing`, here the one zip).
- `export_blocker`: when the plan is empty and `opts.layers.is_some()` => "Select at least one layer"; the other
  reasons as today.
- Preview caption when `opts.layers.is_some() && opts.flat.is_some()`: "The preview shows the whole texture; the
  flattened file holds only the selected layers". Otherwise the present caption.

### 5.4 `src/panels/layers.rs`

- Pure helper `row_click_action(mods: egui::Modifiers, layer: LayerId, active: bool, multi: bool) -> Option<AppAction>`:
  `mods.command` => `ToggleLayerSelected`; `mods.shift` => `SelectLayerRange`; else `SelectLayer` when
  `!active || multi` (a plain click on the active row of a multi-selection collapses it), else `None`.
  Used by the three click sites (row, thumbnail, name) in place of `if resp.clicked() && !active`.
- Row background: active as today; selected but not active = `selection.bg_fill.linear_multiply(0.3)` plus a 2 pt
  accent bar at the left edge. The name is bold for the active layer only.
- Context menu on the row widget, the thumbnail and the name (`Response::context_menu`,
  `REG\egui-0.37.0\src\response.rs:1112`; the three widgets are separate hit targets, so each gets it; call it before
  `on_hover_text` consumes the response). Shared `fn row_menu(ui, selection, layer, actions, hits)`:
  - the row is in a selection of N > 1: "Export selected layers… (N)" => `ShowExportLayers(selection)`, rect recorded
    as `PanelHit::LayerMenuExportSelected`;
  - always: "Export this layer…" => `ShowExportLayers(vec![layer])`, rect as `PanelHit::LayerMenuExportThis`.
  A right-click does not change the selection or the active layer.
- `drag_row`: `row_resp.dragged()` -> `row_resp.dragged_by(egui::PointerButton::Primary)` so a right-button press
  that moves a little cannot start a reorder.
- Footer: with N > 1 one weak line "N layers selected · tools paint on <active name>". (It adds to
  `layers_footer_height`, which is measured each frame; `assert_side_panel_layout` must still pass in `layers`.)
- Hover text of the name: "Click: select  |  Ctrl / Shift-click: select more  |  Double-click: rename  |  Drag: reorder".

### 5.5 `src/panels/menu_bar.rs`

Layer menu: "Export Layer…" with one layer selected, "Export Selected Layers… (N)" with more; enabled with an
active layer; => `ShowExportLayers(session.layer_selection())`.

### 5.6 `src/io_flows.rs`

Success toast: `report.packaged == 0` => the present text, unchanged ("Exported N files to <folder>");
otherwise "Exported <zip name> (N files) to <folder>". Both still start with "Exported " and end with
" to <folder>", which `workflow_path` asserts.

### 5.7 `src/selftest/mod.rs`

`Step::ScreenClick { at: egui::Pos2, button: egui::PointerButton, mods: egui::Modifiers }` = `click_frames(at,
button, mods)` (the helper exists, line 643). Needed for Ctrl-click, Shift-click and the right-click on a row.
`ScreenPointer` stays as it is.

---

## 6. Tests

### 6.1 Core unit tests (`export/mod.rs`, on `layered_doc()` / `two_set_doc()`)

| Test | Checks |
|---|---|
| `default_options_plan_and_bytes_are_unchanged` | `layers: None, package: Files`: `plan` equals the names listed in the existing tests; guards the "nothing changes by default" rule together with the existing 20 export tests, which must pass **unedited** apart from `options()` |
| `plan_with_a_layer_selection` | `Some([paint, top])`: per-layer names are exactly `model_L01_…`, `model_L03_…` (full-stack indices); `plan_contents` kinds and layer ids; unknown ids ignored; order of the id list does not matter |
| `a_selected_hidden_layer_is_exported` | `Some([hidden])`, `include_hidden: false`: `model_L02_paint_Hidden_ notes.png` is written and decodes to the layer; in ORA `visibility="hidden"`, in PSD `hidden` |
| `flattened_of_a_selection_is_the_selected_visible_layers` | flat PNG == `flatten_layers` of the selected layers; differs from the document composite; a hidden selected layer does not change it; ORA `mergedimage.png` and PSD merged equal it; ORA / PSD hold exactly the selected layers |
| `an_empty_or_stale_selection_is_nothing_to_export` | `Some(vec![])` and `Some([LayerId(9999)])`: `plan` empty, `run` => `NothingToExport`, the folder is not created — for `Files` and `Zip` |
| `a_selection_leaves_out_sets_without_selected_layers` | two sets, a layer of set 1 selected: only set 1's files, without the `_<set>` name part |
| `zip_entries_are_the_loose_files` | every format on: export loose into A, zip into B; `read_package` names == loose names in plan order; each entry's bytes == the loose file's bytes; B holds exactly `model.zip` (no stage folder, no temp file); `report.files == plan == [zip]`, `packaged` == entry count, `bytes_written` == file size |
| `zip_export_is_deterministic` | two zip exports of the same snapshot are byte-identical; also with 1 lane against many |
| `zip_methods_and_timestamps` | via `zip::ZipArchive`: png / ora entries Stored, psd / tga Deflated; every entry's time is 1980-01-01 00:00:00 |
| `zip_of_a_selection` | `layers: Some(..)` + `Zip`: entries == the selection's loose files |
| `a_cancelled_or_failed_zip_export_leaves_nothing` | cancel during staging and during packing (the `CancelAfter` progress of the existing tests); failure injected like `a_failing_set_stops_the_export_and_its_error_is_returned`: no zip, no stage folder, no temp file; **an existing `model.zip` is still the old one, byte for byte** |
| `zip_replaces_an_existing_package_atomically` | a second export over an existing zip succeeds and the result equals a fresh export |

`tests/end_to_end.rs`, `two_material_pipeline`: after the existing export block add a zip export of the same options
and compare entries with the files just checked; add a selection export (the stamp layer and the text layer of the
body set) and check both PNGs against `dense(layer, ..)`.

`naming.rs`: `package_name` (sanitising, reserved names).

### 6.2 App tests (run by plain `cargo test`)

- `panels/layers.rs`: `row_click_action` table (plain / Ctrl / Shift, active / not, multi / not).
- `state.rs`: `layer_selection` normalisation — stale id, id of another set, active layer missing from the list,
  no active layer, order bottom to top.
- `dispatch.rs` (next to the existing action tests, line ~880): Toggle add / remove / last one stays / toggling the
  active layer moves active to the top-most remaining; Range up and down; `SelectLayer` collapses; **history length,
  `revision()` and `is_modified()` are unchanged by all three**; after `DeleteLayer` + `Undo` the selection is a
  valid single layer; the three are not interaction-guarded.
- `dialogs/export.rs`: `scope_of` / `apply_scope` round trip; `export_blocker` reasons including "Select at least
  one layer"; the list model for `Zip` (one disk path, N entries) and for a selection; "separate layer" markers from
  `PlannedKind`. The existing tests of `classify_plan` / `kind_from_name` are rewritten against `plan_contents`.
- `control_sweep/panel_tests.rs` (`PanelApp`, records actions, no GPU): Ctrl-click and Shift-click on a row emit
  `ToggleLayerSelected` / `SelectLayerRange`; `right_click` on a row name shows "Export this layer…", and with two
  selected also "Export selected layers… (2)"; clicking them emits `ShowExportLayers` with the right ids; the Layer
  menu item's label and action (extend the table at line 124).
- `control_sweep/gpu_tests.rs` (`#[ignore = "needs a GPU"]`; **this machine has one — the gate runs them with
  `-- --ignored control_sweep`**): update `export_dialog_controls_and_export` and
  `export_dialog_at_the_smallest_window_and_with_several_sets` for the Layers row in place of "Include hidden
  layers"; add: tick "Pack into one .zip file", click Export => one zip on disk; Export again => the confirm step
  lists the zip; open through `ShowExportLayers` => "Selected (2)" is chosen and the list shows two files.

### 6.3 New self-test scenarios (`selftest/scenarios.rs`, `NAMES`, and the 8.11 table)

All three decode what they wrote. `wte_core::image_io::decode_image` and `flatten_layers` are public in core; if
they are not re-exported at the crate root, re-export them (no new dependency for `wte_app`). Zips are read with
`wte_core::export::read_package`.

**`export_selected`** — default content (Sphere: Base, Paint 1, stamp "Logo", text "WTE"), then:

1. `SetLayerVisible(text, false)` (the hidden-selected case). Remember history length and revision.
2. `ScreenClick` primary on Logo's name => active Logo, selection [Logo].
3. `ScreenClick` Ctrl on the text row => selection {Logo, WTE}; **active still Logo**; history and revision unchanged.
4. `ScreenClick` Shift on Paint 1 => {Paint 1, Logo}; Ctrl on the text row => three; Ctrl on Paint 1 => {Logo, WTE}.
5. `ScreenClick` secondary on Logo's name; `Dyn`: `ScreenClick` on `panel_rect(LayerMenuExportSelected)`.
6. Assert: `Dialog::Export` with `layers == Some([logo, text])`, per-layer on, flat `None`, `plan_contents` = two
   `Layer` entries. `Shot("dialog")`.
7. `Setup`: empty `<out>_export_sel`, write it into the dialog's `options.dir`; `Dyn`: `RunExport(dialog options)` —
   the action the Export button pushes. `WaitIdle`.
8. Assert: the folder holds **exactly** `<name>_L02_stamp_Logo.png` and `<name>_L03_text_WTE.png` (directory
   listing, not `plan`); each decodes to that layer's pixels (`cache.to_dense`); the stamp file has ink and none of
   the red or blue stroke colours; no dialog open; history length unchanged since step 1.
9. Single layer: `ScreenClick` secondary on Paint 1's row, click `LayerMenuExportThis`, same run into
   `<out>_export_one` => exactly `<name>_L01_paint_Paint 1.png`, equal to the layer's raster, and it holds both
   strokes' colours.
10. Selection with flattened on (`Do(RunExport(..))` built in a `Dyn`): the flat PNG equals `flatten_layers` of the
    visible selected layers and is transparent where the Base alone shows.

**`export_zip`** — the content of `export_run` (default content + `t7_style_default_text`):

1. `RunExport(export_options)` into `<out>_export_ref` (loose; the same files `export_run` writes).
2. `RunExport({ package: Zip, dir: <out>_export_zip, ..same })`; again into `<out>_export_zip2`.
3. Assert: each zip folder holds exactly `<name>.zip`; entry names == the loose file names in `plan_contents` order;
   every entry's bytes == the loose file; zip 1 == zip 2 byte for byte; the flat PNG entry decodes to
   `doc.composite(set)`; exactly one `_stamp_` and one `_text_` entry and each equals its layer; the toast reads
   "Exported <name>.zip (N files) to <folder>".
4. `ShowExport`, set `package = Zip` and the zip folder in the dialog state, one frame, assert the dialog's cached
   list reports the zip as existing (the "will be replaced" path); `Shot("replace")`; `CloseDialog`.

**`export_flow`** — the end-to-end acceptance check, section 7.

`timeouts()`: `export_flow` gets the long limits of `workflow_path`.

### 6.4 Identity gate (what must not change)

- `export_run` on the new exe: SHA-256 of every file in `<out>_export` equals the `round9-wip` exe's (the round 3-9
  practice; `tools/round3/check_exports.py` must PASS on it and on the unpacked `export_zip` package).
- Every other existing scenario still `"ok": true`; `layers`, `stamp`, `default` screenshots pixel-identical (the
  Layers rows look the same with one layer selected; toast area excepted as usual).
- A `.wtep` saved by the `round9-wip` exe opens in the new one and the other way round (`roundtrip`,
  `wrap_surface`'s reopen step; nothing in `project/` is touched — confirm with `git diff --stat`).

---

## 7. End-to-end acceptance check of the whole workflow

Scenario **`export_flow [PATH]`**.

- Without PATH: writes into `<out>_assets` a `seam_cube` as **OBJ + MTL + a checker PNG** (`to_obj` with a texture
  name), **glTF** (`to_gltf`), **GLB** (`to_glb`) and **ASCII FBX** (`to_fbx_ascii`), and runs the flow below once per
  file (`NewFromModel(Some(path))`, later ones through `ConfirmPending(Pending::NewFromModel(..))`, the import dialog
  accepted by `accept_import`).
- With PATH: the flow once on that model. Gate runs: `samples\BoxTextured.glb`, `Avocado.glb`, `WaterBottle.glb`,
  `DamagedHelmet.glb`, `test-assets\shapes\torus_knot.obj`, `test-assets\shapes\gltf_node_transforms.gltf`.

Flow per model (helpers of `workflow_path` are reused: `model_hits`, `hit_stroke`, `stamp_spots`, `placed_stamp`):

| Step | Action | Assert |
|---|---|---|
| import | open, accept the dialog | a document with at least one set; Base equals the imported texture where there is one |
| paint | Brush, `hit_stroke` on Paint 1 | composite changed, one history entry |
| stamp | Stamp tool, mode Project, click a solid spot | a stamp layer that covers texels |
| text | Text tool "WTE", click another spot | a text layer that covers texels |
| wrapped stamp | Stamp tool with `tools.stamp.mode = SurfaceMode::Wrap`, click a third spot | a stamp layer whose placement is `Projected` with `mode == Wrap`, covers texels, `WaitIdle` (Final bake landed) |
| remember | — | per set: composite hash; per layer: pixel hash |
| export one layer | `ShowExportLayers([wrapped])` => `RunExport(dialog options)` into `<out>_<fmt>_one` | folder listing == exactly that one PNG; decodes to the layer's pixels; has ink |
| export a selection | `SelectLayer(stamp)`, `ToggleLayerSelected(text)`, `ShowExportLayers(selection)` => run into `<out>_<fmt>_sel` | exactly two PNGs, each == its layer; the two differ from each other and neither contains the stroke colour |
| export everything, one package | `RunExport` with every kind on, `include_hidden: false`, `package: Zip` into `<out>_<fmt>_all` | exactly one zip; entries == `plan_contents`; per set the flat PNG == `doc.composite(set)` exactly; **recomposition**: the per-layer PNGs of a set, decoded and put back into `Layer`s with the document's opacity / blend / order, `flatten_layers` to exactly the composite; ORA `mergedimage.png` == composite; the PSD parses with `read_psd` and its layer count == the set's visible layers |
| unchanged | — | the document's composite and layer hashes equal the remembered ones; history length unchanged by the three exports; no dialog open; no `*.tmp-*` anywhere under the output folders |

What this proves that nothing proves today: an imported model of each format, carrying all four kinds of content
including a wrapped stamp, exports to files that are (a) there, (b) one per layer, (c) pixel-equal to what the app
shows.

### 7.1 What existing tests already cover

| Part of the workflow | Covered by | How strongly |
|---|---|---|
| Import OBJ through the dialog | `import`, `import_tex`, `import_nouv` | good (sets, textures, layers) |
| Import glTF / GLB / FBX as the model | core import unit tests; `open_path` / `workflow_path` **only when someone passes a PATH** | no argument-free scenario imports anything but OBJ; FBX appears in scenarios only as a brush shape (`brush_shape`) |
| Paint, stamp, text | `default`, `paint3d`, `paint2d`, `stamp`, `text`, `workflow_path` | good |
| Wrapped stamp | `wrap_surface` | good for the wrap itself; **never exported** |
| Export everything, loose | `export_run`, `workflow_path`; core `two_material_pipeline` | scenarios: files exist and are non-empty, names contain `_stamp_` / `_text_`. Core: pixel-exact for every format, but on the built-in sample, Project stamps only |
| Export dialog | `export` (dialog opens, plan non-empty); GPU test `export_dialog_controls_and_export` | the GPU test clicks Export and checks files, but is `#[ignore]` |
| Export one layer / a selection / a package | nothing | — |

### 7.2 Obvious failures the existing tests would miss

1. **Wrong pixels in an app export.** `assert_exported` and `workflow_path` accept any non-empty file: a blank PNG,
   the wrong layer's pixels, a stamp exported at Preview quality or before its Final bake, or a flattened texture
   that differs from the screen all pass. (`final_bake_async` (f) compares two exports with each other, so both being
   wrong the same way passes.)
2. **A wrapped stamp missing from, or stale in, the export.** No test exports a document with a Wrap stamp.
3. **A GLB / glTF / FBX import that cannot be exported** (or exports the wrong base texture). No scenario without a
   PATH argument does it, so the routine sweep never sees it; and there is **no real FBX file on this machine** — the
   only FBX input is the generated ASCII cube. A real binary FBX from the user would close that.
4. **Stray or missing files.** The checks ask `export::plan` which files to look for — the same function that decided
   what to write. A leftover temp file, an extra file, or plan and run being wrong together are invisible. The new
   scenarios list the folder instead.
5. **The Export button itself.** Only an ignored GPU test presses it. Plain `cargo test` and the scenario sweep both
   go through `RunExport` directly. The gate must run the ignored `control_sweep` tests on this machine.
6. **The dialog's file list disagreeing with the exporter.** `classify_plan` re-implements the filter and on a
   mismatch only logs a warning. Removed by `plan_contents`.
7. **Per-layer files that do not add up to the texture.** Nothing recomposes exported layers and compares with the
   composite in the app (the core test compares each layer with itself, which is weaker when blend or opacity is
   involved).
8. **Layers of a multi-set model.** `workflow_path` on a multi-set PATH checks existence only; set-qualified names
   and per-set pixels are unchecked in the app.
9. **Hidden layers.** No scenario exports with a hidden layer; only core unit tests do.
10. **Export changing the document.** Nothing asserts that composite, history and the modified mark are the same
    after an export.

Items 1, 2, 3 (except a real FBX), 4, 7, 9 and 10 are closed by `export_flow`, `export_selected` and `export_zip`;
5 by running the ignored tests at the gate; 6 by the code change; 8 by running `export_flow` on a multi-set model
(add `samples\ABeautifulGame.glb` to the gate list if its run time is acceptable on this machine).

---

## 8. ARCHITECTURE.md sections to update

| Section | Change |
|---|---|
| 1 (new 1.8 "Export scope and package") | the feature in five lines, the decisions of section 2 here |
| 2.1 / 2.2 tables | `export/mod.rs` (`plan_contents`, `ExportPackage`), `panels/layers.rs` (multi-select, row menu) |
| 5.24 | `ExportOptions.layers`, `.package`, `ExportPackage`, `ExportReport.packaged`, `PlannedFile` / `PlannedKind`, `plan_contents`, new meaning of `plan` for `Zip`, `package_name`, `read_package` |
| 7.4 | layer scope (filter, set skipping, flattened = selected visible layers, ORA / PSD = selected layers); the package run (stage folder, pack order, methods, timestamps, atomic rename, nothing left after cancel / failure, cost) |
| 7.5 | `<base>.zip`; entry names = loose names, flat, no folders |
| 8.2 | `Session::selected_layers` + `layer_selection()` and its normalisation rule; "not saved, not history"; `ExportDialogState::selected_layers`; the three actions; the two `PanelHit`s |
| 8.3 | dispatch of the three actions; `ShowExportLayers` in the interaction-guard lists |
| 8.7 | Layers rows: Ctrl / Shift / plain click, active vs selected, row colours, context menu, footer line; Layer menu item; "commands act on the active layer" |
| 8.8 | Export dialog: Layers scope row (replaces "Include hidden layers"), "Pack into one .zip file", list for `Zip`, defaults when opened for layers, the caption under a selection, the new blocker text |
| 8.9 | the zip toast text |
| 8.11 | `Step::ScreenClick`; rows `export_selected`, `export_zip`, `export_flow` in the scenario table; `NAMES` |
| 9.1 / 9.2 | the tests of 6.1 / 6.2 |
| 7.1-7.3 | **no change** (state explicitly in 1.8 that the project format is untouched) |

`HANDOFF.md` 4.1 scenario list: add the three names. `dist/HOW_TO_TEST.txt`: three lines on Ctrl / Shift-click,
right-click > Export, and the zip checkbox.

---

## 9. Build order (one builder, one check-and-fix, one gate)

1. Core: 3.1-3.4 and section 4 with the tests of 6.1. `cargo test -p wte_core` green; the existing export tests
   unedited except `options()`.
2. App state, actions, dispatch, panel (5.1, 5.2, 5.4, 5.5) with the non-GPU tests of 6.2.
3. Dialog and toast (5.3, 5.6), dialog tests, GPU tests updated.
4. Runner step and the three scenarios (5.7, 6.3, section 7).
5. ARCHITECTURE.md, HANDOFF.md, HOW_TO_TEST.txt.
6. Gate: `test --workspace`, `test -p wte_app -- --ignored control_sweep`, clippy 0 warnings, release build, full
   scenario sweep, the identity checks of 6.4, `export_flow` on the PATH list of section 7, screenshots of
   `export_selected` (`_dialog` shot: two rows highlighted, scope "Selected (2)", two files listed) and `export_zip`
   (`_replace` shot) looked at by the main session.

Rough size: core ~250 lines + ~350 of tests; app ~350 lines + ~300 of tests; scenarios ~500 lines.

---

## 10. Open points for the user (defaults chosen; none blocks the build)

1. **Flattened / ORA / PSD under a selection = the selected layers only** (decision 4). The alternative "always the
   whole texture" is a two-line change in `export_set`.
2. **A stamp that spans two materials** has one layer per texture set. "Export this layer…" exports the layer of the
   active set only. Exporting its sibling layers too would be a small addition (`doc.stamp_layers`), left out for now.
3. **Layer commands with several layers selected** (Delete, Merge, Duplicate) still act on the active layer only.
   Making them act on the selection is a separate feature (history entries for multi-layer edits).
4. **The zip is flat** (no folder inside) and named `<base>.zip`; the dialog does not remember the zip tick between
   openings (like every other option except the folder).
5. **A real FBX model** for the acceptance run: none is on this machine; the check uses a generated ASCII FBX cube.
6. Should a zip export also be offered for "Save" style workflows (project + exports in one file)? Not planned.
