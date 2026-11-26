# PhotoCraft v0.2.0: CLI `convert` save vs GUI Ctrl+S (STICKER_PLAN P0, review B16)

**Verdict: the same.** In v0.2.0, `photocraft-cli convert in.psd out.psd` and a Ctrl+S in the GUI on a document opened
from a `.psd` both end in `photocraft_io::export(doc, path, &ExportOptions::default())` ->
`document_to_psd_with(doc, &PsdExportOptions { force_psb: false })` -> `PsdFile::to_bytes()` -> `std::fs::write`. Both
open the file through the same `photocraft_io::import`. Nothing that touches `lyid`, layer pixels, trimming or the merged
image differs between the two. Fixture f1 (`crates/wte_core/tests/fixtures/psd/f1_pc.psd`) therefore stands for a GUI
save of an **unedited** document. What f1 cannot show is what the user's own editing in the GUI does to a layer between
open and save (for example whether a tool replaces a layer and with it its `psd_id`); the code says painting keeps it
(below), but **hand test 1 stays the proof** that a WTE text layer comes back as text after a real Ctrl+S.

Source read: `G:\Claude_Resources\ref\scratch\pc020\` (PhotoCraft v0.2.0 source). Paths below are relative to it.
Nothing was run except the console `photocraft-cli.exe` (to make the fixtures); `photocraft.exe` was never started.

## 1. CLI: `photocraft-cli convert <in> <out>`

1. `apps/photocraft-cli/src/lib.rs:109` dispatches `"convert"` to `convert` (`:159-167`).
2. `:163` `files::open(Path::new(input))` = `crates/automation/src/files.rs:29-36`: reads the bytes, then
   `open_bytes` (`:19-25`): not `.pcraft` -> `photocraft_io::import(name, bytes)` (`crates/io/src/lib.rs:110-`): PSD by
   magic -> `PsdFile::from_bytes` -> `psd_to_document` (layer `psd_id` from `lyid`, `crates/io/src/psd_import.rs:190`).
   No session, no colour policy, no script events.
3. `:165` `files::save(&o.document, out, --format, &export_opts(a), None)` (`files.rs:46-69`): extension `psd` ->
   `photocraft_io::export(doc, &ext, opts)` (`:63`), then `std::fs::create_dir_all(parent)` and
   `std::fs::write(path, &r.bytes)` (`:64-67`).
4. `export_opts` (`lib.rs:139-146`) = `ExportOptions::default()`; `--quality` only sets the JPEG quality.
5. `photocraft_io::export` (`crates/io/src/lib.rs:132-149`): `psd`/`psb` -> `PsdExportOptions { force_psb:
   opts.force_psb || ext == "psb" }` (`:142`; `force_psb` is false by default), `document_to_psd_with(doc, &o)`
   (`crates/io/src/psd_export.rs:545`), `file.to_bytes()` (`crates/psd/src/file.rs:136`).

## 2. GUI: Ctrl+S on a document opened from a `.psd`

Opening (File > Open, a drop, or a path on the command line):

1. `crates/ui-egui/src/file_open.rs:29-40` `open_file(path, bytes)` -> `open_bytes` (`crates/ui-egui/src/lib.rs:418-`)
   -> `self.services.import` = `apps/photocraft/src/services.rs:99-101`: `photocraft_io::import(name, bytes)` (the same
   importer as the CLI).
2. `lib.rs:426` `self.session.open_document(doc, None)` = `crates/engine/src/color_cmds.rs:917-920`: applies the
   Colour Settings policy `open_policy` (`:236-276`) before the document is added. For an **untagged** file (WTE writes
   no ICC profile) it only *assigns* the working profile (`doc.icc_profile = Some(..)`, `:270-274`) when the user's
   working RGB space is not the built-in default; pixels are never converted for an untagged file. Default settings:
   `"untagged"`, nothing changes. A tagged file with a mismatching profile and policy "Convert" would be converted
   (`:260`); WTE's files are untagged.
3. `file_open.rs:36-38` remembers `path` as the document's own path; `automate_cmds::document_opened` fires the
   "Open Document" script event (only does something if the user bound a script).

Saving:

1. Ctrl+S: `crates/ui-egui/src/shortcuts.rs:194-224` finds `file.save` (`crates/ui-egui/src/menus.rs:14`, "Cmd+S" =
   Ctrl on Windows) and calls `menus::invoke(app, ctx, "file.save", {})` (`shortcuts.rs:224`).
2. `menus.rs:100-110` `invoke`: the unsaved-changes interceptor (`discard_ui::intercept`) only reacts to close / exit
   commands; then `invoke_unguarded`.
3. `menus.rs:220-227` `"file.save"`: `path` = the document's own path when `saves_in_place` (`:405-408`: extension
   `psd`, `psb` or `pcraft`), so a PSD is written back to itself without a dialog; then `app.save_as(path)`.
4. `crates/ui-egui/src/lib.rs:501-532` `save_as(Some(path))`: (an Edit Contents document would save into its smart
   object instead, `:502-506`; not our case) -> `self.services.export` (`:514-515`) with `ExportSettings::default()`
   (`jpeg_quality: None`) = `apps/photocraft/src/services.rs:103-108`: `ExportOptions::default()` ->
   `photocraft_io::export(doc, path, &opts)` inside `crash_guard::guard` -> the same function as the CLI's step 5.
5. `lib.rs:516-517` `self.services.write` = `services.rs:125` `std::fs::write(path, bytes)` (truncate and write in
   place; not a temporary file plus rename, the same as the CLI).
6. After the write: `saved_revision`, status text, and `automate_cmds::document_saved` (`crates/engine/src/
   automate_cmds.rs:269-272`): the "Save Document" script event and File > Generate > Image Assets. Neither touches
   the file just written unless the user bound a script that saves again.

## 3. Comparison

| Point | CLI `convert` | GUI Ctrl+S | Same? |
|---|---|---|---|
| Reader | `photocraft_io::import` | `photocraft_io::import` | yes |
| Between read and write | nothing | Colour Settings open policy (may add an ICC profile to an untagged file when the working space is not the default; resource only), the user's edits | pixels, `lyid`, merged: yes for an unedited file |
| Exporter | `photocraft_io::export(doc, "psd", ExportOptions::default())` | `photocraft_io::export(doc, path, ExportOptions::default())` | yes (`ext` from the name either way) |
| PSD options | `force_psb = false` | `force_psb = false` | yes |
| Layer ids | `psd_export.rs:577-590`: stored `psd_id` kept when unique; a layer without one gets the **lowest free** id counting from 1 (dividers come after all, `fresh_id` `:141-144`) | same function | yes |
| Trim / RGB under alpha 0 | `Surface::content_bounds` trim (`psd_export.rs:87, 107`) | same | yes |
| Merged image | built inside `document_to_psd_with` (matted on white, 3 channels when opaque) | same | yes |
| Compression | RLE for 8 / 16 bit | same | yes |
| Write | `std::fs::write` | `std::fs::write` | yes |
| After the write | nothing | script events, Image Assets (user opt-in) | no effect on the file |

Layer ids through editing (supports hand test 1, not a proof): `psd_id` is a field of the document's `Layer`
(`crates/doc/src/lib.rs:424`), set on import and cleared only when a layer is duplicated (`:512`, test `:882`). Painting
or adjusting a layer keeps the layer and its id; a new layer (also a new text layer) gets `psd_id = None` and at save the
**lowest id no other layer uses** (f5: the type layer got `lyid` 4 because 1..3 were taken). This is not "above the
largest": a WTE layer deleted in PhotoCraft frees its id, and a layer added in the same edit takes it (fixture f12:
`text` = `lyid` 2 deleted, the new `pc new` layer is `lyid` 2). HEAD does the same (`crates/io/src/psd_export/document.rs`
`index_layer_ids_and_text`). STICKER_PLAN 5.4 "`lyid` reuse" says what P5 does with it. (Corrected by the P0 check.)

## 4. Latest PhotoCraft (HEAD), for P9 (read only, not compared in full)

The latest save path is different code (`crates/ui-egui/src/app/files.rs`, `crates/ui-egui/src/menus/invoke.rs`), so
this note's verdict is for v0.2.0 only. Two things a builder of P9 should know (read at HEAD, not tried):

- File > Save writes back to the document's own layered file only when that format can hold the whole document
  (`menus/invoke.rs` `"file.save"`, `file_cmds::holds_at`); otherwise it posts a notice and opens Save As (#3806).
- **Before writing over its own file, latest PhotoCraft asks when another app changed that file since it was opened or
  last saved** (`crates/ui-egui/src/dialogs/disk_change_prompt.rs`, issue #6596: "Replace" or "Cancel"). So after WTE's
  "Send to PhotoCraft" rewrites a PSD that is open in the latest PhotoCraft, the user's next Ctrl+S there shows that
  question first. v0.2.0 has no such check (its `save_as` above writes at once). Hand tests 4 and 6 should note it.
