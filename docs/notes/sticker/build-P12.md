# Build note: P12 - Linked texture paint layers (STICKER_PLAN 7 "P12", 5.9; rev 3: the P12 row of 13.9)

Builder, 2026-11-27, branch `stickers` (from ab001de, the P11 check; plan revision 3). Kind K + U. A raster paint
layer can now be edited in PhotoCraft, Photoshop or another program the way a sticker is: "Edit Layer in <editor>"
writes a one-layer PSD at the texture's size next to the project, the editor's Ctrl+S updates the layer and the model
in place (one undo step "Update Linked Layer"), with the same watcher, states, conflict, Send, save, Save As and load
rules as a linked sticker. Plus Edit > Paste as New Paint Layer. No new crate, `Cargo.toml` / `Cargo.lock` unchanged.
The user's workflow is untouched: stamps, placements and styles are never in a layer's file, and nothing a placement
edit does reaches a link.

## What was built

| Item (plan 5.9, 13.9 "P12", 7 "P12") | Where | Notes |
|---|---|---|
| `PaintLink { file, editor, synced, synced_pixels, editor_layers, problem }`, the file name `<layer>_l<id>.psd`, the archive entry `links/l<id:05>.psd`, `normalised_crc`, `layer_psd` (one layer, `lyid` 1, opacity 255, Normal), `paint_source` (the source rule of 5.9 and the size refusal) | new `wte_core/src/link/paint.rs` | `PAINT_UPDATE_LABEL` = "Update Linked Layer"; `MAX_PAINT_LINK_SIDE` 8192, "This texture is too large to link." |
| State table for any link | `link/mod.rs` | `state_of(synced, problem, observed, wte_changed)` (`link_state` calls it); `send_first_text` (`send_first_question` calls it); `LinkProblem::SizeMismatch` / `LinkState::SizeMismatch` |
| Texts | `link/texts.rs` | `size_mismatch` ("<layer> is W x H in <editor>; the texture is w x h. Undo the size change in <editor>."), `layer_changed_while_closed`, `edit_layer_in` |
| `Document`: the layer links | new `doc_layer_links.rs`, `document.rs` | `layer_link(s)`, `layer_texture_size`, `layer_pixels_crc` (cached by layer version; during a texture stroke the badge does not hash the texture, it is "changed in WTE"), `layer_link_state`, `layer_link_changed_in_wte`, `prepare_layer_link_file` / `finish_layer_link_write`, `unlink_layer`, `set_layer_link_file_and_editor`, `apply_layer_link_update`, `layer_link_file_refused`; `StickerState::layer_links` (outside the history, sharing `links_revision`), `DocumentSnapshot::layer_links` |
| `replace_layer_pixels(layer, PixelBuf, label)` | `doc_layer_links.rs` | one `Edit::Pixels` with `TilePatch::diff` (only the changed 64-px tiles), dirty = the changed tiles, a new layer version; Ok(false) and no entry when nothing changed; an unallocated layer and an all-zero picture allocate nothing |
| `add_paint_layer_with_pixels`, `fit_centred` | `doc_layer_links.rs` | Paste as New Paint Layer: add + replace in one group entry; another size scaled to fit with the aspect kept (`pixel_ops::resize`), centred |
| Project | `project/{manifest,sticker_entries,save,load,mod}.rs` | `Manifest::layer_links: Vec<PaintLinkEntry>` (skipped when empty), `write_layer_links` (the save rule of `archive_choice`, only existing raster layers, `SaveWarning::LayerLinkBeingWritten`), `verify` checks its archive entries, `required_reader` 4 with a linked raster layer, `load_layer_links` + `LoadReport::layer_links` (`LayerLinkAtLoad`), the shared `file_state` |
| `LinkId { Sticker, Layer }` keyed sessions, watcher, questions, commands | `wte_app/src/link/{mod,watch,flows}.rs` | `common`, `is_live`, `link_name`, `wte_changed`; `link_path` / `file_name` / `editor_name` / `badge` / `busy_for` / `session` take `impl Into<LinkId>`; `LinkAction::{Send, UpdateNow, ShowFile, Unlink, Restore, Take, Later, UpdateClosed, IgnoreClosed}` carry a `LinkId`, `SendRequest::link`; `LinkQuestion::link` (Esc answers a layer's conflict / closed-change too) |
| "Edit Layer in <editor>" | `link/flows.rs` (`edit_layer_in`, shared `switch_question`, `existing_ready`, `open_in_editor`, `write_file(LinkId)`, `apply`, `read_now`, `conflict_question`, `restore`, `before_save`, `after_load_layers`, `layer_note`, `layer_refusal`) | the same editor choice, switch question, program lookup, send-first question, per-editor two-copies guard ("This layer is already open in ...") and start as a sticker; a stamp / text / locked layer is refused with a status line |
| UI | `link/ui.rs` (`layer_block_data`, `layer_block`, `edit_layer_label`, `layer_editable`, shared `link_buttons`), `panels/layers.rs`, `panels/menu_bar.rs`, `state.rs` (`PanelHit::LayerLink*`, `LayerMenuEditIn`, `LayersPasteAsLayer`) | Layer menu "Edit Layer in <editor>" + submenu "Edit Layer in" (editor list); a paint layer's row menu the same; the Layers tab shows a linked active layer's block at the top of the list's scroll area (badge with the refusal / size sentences, "Editor: <name> · <file>", needs-merged sentence, Edit Layer in / Edit in... / Update now / Send to / Show file / Unlink / Restore) |
| Paste as New Paint Layer | `clip.rs` (`ClipAction::PasteAsLayer`, `paste_as_layer`, `layer_picture`, `poll_layer`), `ole/clipboard.rs` (`picture_item`, `LAYER_PASTE_ORDER`) | Edit menu and the right-click menu of the Layers tab's "New"; decoded and fitted on a worker; one entry "Paste as New Paint Layer"; a document opened meanwhile gets nothing; `clip_ui.busy()` counts it (the self-test's idle check) |
| Load wiring | `jobs.rs` (`JobOutput::Built::layer_links`), `io_flows.rs` | `after_load_layers` after `after_load` |
| Scenario | new `selftest/scenarios/layer_link.rs`, `scenarios.rs` (71; long timeouts like `sticker_link`; shots of `layer_link*` must say what they show), `link/testkit.rs` (`clear_real_merged`) | PhotoCraft pass and Photoshop pass (below) |
| Docs | ARCHITECTURE 2.2 (files), 5.14, 5.28, 6.13, 7.1, 7.2, 7.9, 8.11 (the `layer_link` row), 8.16, 9.1, 9.2; HANDOFF (scenario list, the branch row, the P12 paragraph); `dist/HOW_TO_TEST.txt` "Working with PhotoCraft" 16-18 | |

Tests: core `doc_layer_links_tests.rs` (8 + the ignored `perf_k12`): `replace_layer_pixels` is one `Edit::Pixels` entry
with only the changed tile (1 of 12), undo / redo restore, nothing for the same or an all-zero picture, refusals; the PSD
of a layer reads back to the same (normalised) pixels and the layer's opacity is not sent; one-layer and multi-layer
PSDs choose the right source (the layer without merged data with its mask and opacity, a hidden second layer ignored,
a visible one = merged, none = needs merged, an effect = merged, a flat file); a size mismatch is refused with the
sentence and changes nothing (no entry, no revision bump); the state table of a layer link (linked, missing, changed in
<editor>, one update, undo = changed in WTE, conflict, needs merged, CMYK refusal, Unlink keeps pixels); **a project
without layer links writes no `layer_links` key and keeps reader version 1** (plus the identity sweep below); a layer
link survives save and load with `links/l<id>.psd`, Changed / Missing reported, a deleted layer's link not saved; paste
as a new layer at equal and at different sizes; the badge does not hash the texture during a stroke. App
`link/layer_tests.rs` (7): the PSD and the stub call; **a locked and a stamp layer are not offered and refused**; an
editor's save = one update, undo stays "changed in WTE", Update now; the conflict with Take and Keep (the warning, stale);
a size change and a CMYK file on the layer badge with Photoshop's wording; restore and "changed while closed" after a
load, Unlink; Paste as New Paint Layer through the in-memory clipboard (PNG at the texture's size, a DIB of another size,
nothing pasteable). Existing tests updated only for the `LinkId` types (no assertion changed).

Scenario `layer_link` (12 s): Paint 1 painted, saved, row menu "Edit Layer in PhotoCraft" (shot `layer_link_menu`), the
link (`layer_link_made`), the editor's blue band = one "Update Linked Layer" (`layer_link_updated`), undo + 2 s idle +
"changed in WTE" + the block's Update now, a WTE stroke and the editor's green band = the conflict question
(`layer_link_conflict`), Take, Save, reopen (`layer_link_reopened`); Photoshop pass (13.9): an injected registry names a
stub `Photoshop.exe`, Photoshop the default editor, Paint 2 linked from Layer > Edit Layer in Photoshop (the stub saw
Photoshop), "Updated from Photoshop", a 64-px wider canvas refused with Photoshop's size sentence on the badge
(`layer_link_ps_size`), two layers without merged data = "needs a merged picture" and Photoshop's MaxCompat sentence
word for word (`layer_link_ps_merged`). I looked at all seven shots; each matches its expect text (the first run showed
the block pushing the list's footer out of a small Layers area, so the block moved into the list's scroll area).

## Deviations (with reasons)

1. **`synced_pixels` is a CRC-32 of the layer's normalised pixels, not "the layer's version".** Layer versions are
   assigned afresh at load, so a version would make every reopened link "changed in WTE". The CRC is cached by
   version, so the badge costs nothing per frame; during an open texture stroke it is not computed at all.
2. **Manifest: its own list `layer_links`, not `links` entries with `"layer": id`.** A `LinkEntry` requires `sticker`
   and many sticker-only fields; a separate list keeps both strict, an older format-4 reader just ignores it, and a
   project without layer links writes exactly the bytes it did (no key).
3. **"+" menu: the Layers tab has no "+" button**; Paste as New Paint Layer is in the right-click menu of its "New"
   button (and the Edit menu), so the visible footer is unchanged.
4. **The layer's link block shows only for a linked active layer**, at the top of the layer list's scroll area; an
   unlinked layer is linked from the Layer menu or its row menu (the plan's two places). Showing an "Edit Layer in"
   row for every paint layer would change every Layers-tab screenshot.
5. **A file of a linked layer that cannot be applied also gives a warning toast** with the badge's sentence (a
   sticker's problem shows only on its badge): the layer's badge is visible only while that layer is active.
6. **Source rule detail:** "exactly one visible pixel layer" also accepts zero (a transparent picture) and requires the
   one layer not to be clipped onto a layer below; its mask and opacity are applied to alpha (a single layer over
   transparency looks the same in every blend mode). Several visible layers need the merged picture, as planned.
7. **No command-line conversion and no "Fit" for a layer link** (`Fit` / `Convert` stay sticker-only): a layer's file
   must keep the texture's size, and the size sentence says how; "needs a merged picture" says what to turn on.
8. **A locked layer receiving an update** is refused (`LayerLocked`, the error toast); the file stays "changed in
   <editor>" and Update now applies it after unlocking. The plan only says the command is not offered for locked
   layers.

## Numbers (this machine, release)

- `perf_k12` (4096 x 4096): prepare the PSD (CRC + writer, at "Edit Layer in" / Send) 254.8 ms; read it (watcher
  thread) 219.5 ms; **apply an update on the UI thread 61.5 ms** (`paint_source` + `replace_layer_pixels` + the new CRC);
  `fit_centred` of a 3000 x 2000 picture 45.6 ms (paste worker). 8.4 has no own budget for layers; the sticker update's
  1.5 s from the settled file to the model is met with room.
- `cargo test --workspace --no-fail-fast` (final run, `target\sticker\p12_test2.log`): **1,907 passed, 1 failed (WA20,
  the known one), 68 ignored** - wte_app 739 / 0 / 26, wte_core lib 1,111 / 1 / 42, integration 15 + 9 + 8 + 25.
- `cargo clippy --workspace --all-targets`: 0 warnings (`target\sticker\p12_clippy.log`).
- `cargo build --release -p wte_app`: OK, 33,765,888 bytes, SHA-256
  `d3a31d3ae31a1d3d5867e6847d9aa2f3c437c1fa28c05e3b1674d6a9c326707c` (copy `target\sticker\p12\wte_p12.exe`).
- Scenarios (`target\sticker\p12\runs\`, off-screen, no focus taken): `layer_link` ok (12 s; again with the final
  exe), `sticker_link` ok (28 s), `sticker_clip` ok, `smoke` ok.
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\p12\sweep\`, the exe before the last
  expect-text edit, same code): 21 / 21 `"ok": true`. Against the P0 baseline: 214 the same, the two known `side_tabs`
  small shots (five tabs since P6b) and `shot_tab_sticker.png` (new since P6b), as in P10a-P11; against the P11 check's
  sweep (`target\sticker\p11check\sweep\`): **217 / 217 the same**. No old project, export or flow changed.

## For the checker

- The `LinkId` refactor touched every generic link command; the sticker paths should behave exactly as before
  (`sticker_link` passes; the app tests changed only types).
- `replace_layer_pixels` stores the old tiles via `TilePatch::diff` against the old `Arc` buffer (no copy of the texture)
  and replaces the buffer; check undo / redo with `Edit::Pixels`' swap.
- `edit_layer_in` vs the menus' `layer_editable`; the Layer menu's submenu is named "Edit Layer in" (the stamp's stays
  "Edit in").
- Programs started: cargo, Python (`-E`), the WTE release exe only with `--screenshot ... --scenario ...`. Never
  `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe` (the scenario's are empty stub files; the launcher is the stub).
