# Check note: P12 - linked texture paint layers

Checker, 2026-11-27, branch `stickers` at 3e2b0df (the P12 build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P12", 5.9 (and the 5.3-5.7 rules it reuses), the P12 row of 13.9, and the user's decisions.
Verdict: **pass after fixes** (no blocker, no major; five minors fixed with tests, three noted).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,907 passed / 1 failed (WA20 only) / 68 ignored (app 739 / 0 / 26, core
  1,111 / 1 / 42, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\p12check\test1.log`).
- Clippy 0 warnings; release build OK, SHA-256 d3a31d3a... = the builder's exe.
- Scenarios `layer_link`, `sticker_link`, `smoke`: `"ok": true` (`target\sticker\p12check\runs\`). I looked at the
  seven `layer_link_*` shots: the menu with "Edit Layer in PhotoCraft" / "Edit Layer in >" on Paint 1's row; the blue
  band on the sphere with the block "Link: linked" and "Undo: Update Linked Layer"; the conflict dialog "Both WTE and
  PhotoCraft changed Paint 1." with the three answers and the badge "conflict"; the reopened project with the green and
  blue bands and "linked"; Photoshop's size sentence on the badge and as a warning toast; "needs a merged picture" with
  Photoshop's MaxCompat sentence. Each shows what its expect text says.

## Plan 7 "P12" item by item

| Item | Found |
|---|---|
| `replace_layer_pixels` (a tile patch through `Edit::Pixels`, one undo step, caches update as after a stroke) | yes: `TilePatch::diff` against the old buffer, `mark_layer_dirty` with the changed tiles (a fresh version), `push_edit`; undo / redo swap through the existing `apply_edit` (checked) |
| `PaintLink` (file, editor, synced, synced_pixels), export / import of the one-layer PSD | yes; `synced_pixels` is a CRC of the normalised pixels (deviation 1: accepted, versions are renumbered at load) |
| `project/*`: entries, archive `links/l<id:05>.psd`, the save rule, `LoadReport` | yes; own list `layer_links` (deviation 2: accepted, absent when empty) |
| App `link/*` keyed by `LinkId { Sticker, Layer }` (watcher, launcher, dialogs) | yes; I read every generic command (Send, UpdateNow, Take, Later, UpdateClosed, IgnoreClosed, Unlink, ShowFile, Restore, Save As aliases, after_load, Esc of a question): the sticker paths do what they did, `sticker_link` passes |
| Layer menu, row context menu, Edit menu (Paste as New Paint Layer) | yes; "+" menu = the right-click menu of "New" (deviation 3: accepted) |
| 5.9: not offered for stamp-linked and locked layers | yes (`layer_editable` gates both menus; `edit_layer_in` refuses with a status line; test) |
| 5.9: file at the texture's size, opacity 255 / Normal, merged = the same pixels; 8192 limit and its message | yes; textures are capped at 8192 (`MAX_TEXTURE_SIDE`), so "This texture is too large to link." guards only; checker test for the 8192 boundary |
| 5.9: source rule (one visible pixel layer, else the real merged picture, else "needs a merged picture"); a size change refused with the sentence | yes (deviation 6 detail accepted) |
| 5.9: conflicts, guard, watcher, Send (5.5-5.7) | yes, except an update during an open texture stroke: finding 1 (fixed) |
| 5.9: Paste as New Paint Layer, same size as is, else fitted with the aspect kept, centred (the 4.7 resampler) | yes, except a picture ready during a stroke: finding 3 (fixed) |
| 13.9: "Edit Layer in <name>", the same submenu and guard table, `PaintLink::editor`, refusal sentences on the layer badge, `layer_link` with the Photoshop stub | yes; the chooser's sentence spoke of a sticker: finding 4 (fixed) |
| ARCHITECTURE 5.14, 5.28, 6.13, 7.1, 7.9, 8.16, 9.1, 9.2 | yes (also 2.2, 7.2, 8.11) |
| Tests of the plan's list, incl. "a project without layer links saves the same bytes" | yes (no key, reader 1) plus the identity sweep below |
| Checker: regression set 8.5 with attention to `layers`, `export`, `roundtrip`; locked and stamp layers not offered | sweep below (all three the same as P0 / P11); the test `locked_and_stamp_layers_are_not_offered_and_refused` |

## Findings

1. **Minor (fixed): a layer's update that arrived during an open texture stroke the tools did not report was lost.**
   `busy_for` held an update only for a tool interaction, a pending Preview edit or a sticker stroke; with a texture
   stroke open in the document `replace_layer_pixels` refuses with `StrokeInProgress`, `apply` showed an error toast
   and the change was not applied until "Update now". Now a layer link's update also waits for `Document::stroke_active`
   and is applied at the first idle frame. Test `check_p12_an_update_waits_for_an_open_texture_stroke`.
2. **Minor (fixed): an editor's save of the unchanged picture added an invisible "Update Linked Layer" undo step.**
   WTE's eraser keeps RGB under alpha 0 (`brush.rs`: Erase changes only alpha); the link file holds normalised pixels,
   so a Ctrl+S in the editor without any change replaced the layer's stored bytes (one undo step, a new layer version,
   the 3D textures re-uploaded) though nothing visible changed. `apply_layer_link_update` now skips the replace when
   the picture equals the layer up to RGB under alpha 0 (`same_normalised`, exact, no allocation); the link is still in
   sync with the new file. Test `check_p12_an_unchanged_picture_over_rgb_under_alpha_0_is_no_undo_step` (a one-pixel
   change is still applied).
3. **Minor (fixed): Paste as New Paint Layer was lost when its picture was ready during an open stroke** (the new
   layer refused with `StrokeInProgress`, the picture dropped). `poll_layer` now waits for the stroke's end. Test
   `check_p12_paste_as_a_layer_waits_for_an_open_stroke`.
4. **Minor (fixed): the chooser of "Edit Layer in..." (two editors, no default) said "WTE opens the sticker's layers
   in it; Ctrl+S there updates the sticker on the model."** and was not tied to the layer. It now says "WTE opens the
   layer's pixels in it; Ctrl+S there updates the layer on the model." with `LinkQuestion::link` = the layer. Test
   `check_p12_the_layer_chooser_speaks_of_the_layer`.
5. **Minor (fixed): the close prompt named a linked paint layer deleted since its link was made** ("layer (paint
   layer)", since its name is gone), though a save leaves that link out. `unsaved_line` (and its editor list) now skips
   a layer link whose layer is not a live raster layer; an undo of the delete names it again. Test
   `check_p12_a_deleted_linked_layer_is_not_named_by_the_close_prompt`.
6. **Noted (not changed): the link block takes room from the layer rows.** At 1280 x 800 with the default split the
   Layers area shows about one and a half rows below a linked active layer's block (shots `layer_link_updated`,
   `_reopened`); the rows scroll and the footer stays visible (the builder's fix of the first run). The split between
   the two tab groups can be dragged. A layout question for the gate / the user, not a defect of the package.
7. **Noted (open, for the gate): UI-thread costs.** Making the PSD at "Edit Layer in" / Send: 255 ms at 4096 (about
   1 s at 8192); applying an update: 61.5 ms at 4096 (about 4x at 8192). Plan 8.4 has no budget for layer links; both
   are one-off explicit actions or one per editor save. The builder recorded them; I agree they are not a P12 defect.
8. **Noted: `hold` sets the Sticker view's `held_update` for a layer link too** (the flag only drives the view's
   re-pick after an update of the sticker it shows; `retry_held` clears it). Harmless.

Also checked and found right: undo / redo of `replace_layer_pixels` on an unallocated layer and of Paste as New Paint
Layer (the structure snapshot taken at undo time holds the allocated buffer, so the redo's pixel swap has a target);
the CRC cache keyed by the global layer version (a structure undo that restores an old version recomputes); links of
deleted layers kept for an undo, not saved, not watched; `has_layer_links` / `required_reader` 4 only with a linked
existing raster layer; the archive entries are verified at save; load reads the files only; no write of a link file
by a save or a load; file names `<name>_l<id>.psd` cannot meet a sticker's `_k<id>.psd`; Esc on a layer's conflict =
Decide later; nothing starts a program but the stub launcher in tests and the scenario (`photocraft.exe`,
`Photoshop.exe` there are empty stub files); no clipboard, registry write or `%LOCALAPPDATA%` touch in tests; no new
crate; LF line endings on every new file.

## After the fixes

- `cargo test --workspace --no-fail-fast` (`target\sticker\p12check\test3.log`): **1,913 passed, 1 failed (WA20),
  68 ignored** - app 743 / 0 / 26 (+4 checker tests), core 1,113 / 1 / 42 (+2), integration 15 + 9 + 8 + 25.
- Clippy 0 warnings (`clippy3.log`). Release build OK, SHA-256
  `9bd886962fd4d83c6abc36f02ed37ddcf0e62ec18dfdd8e50665a62e2ca89479` (copy `target\sticker\p12check\wte_p12check.exe`).
- Scenarios on that exe (`runs2\`): `layer_link`, `sticker_link`, `sticker_clip`, `sticker_drop`, `smoke` all
  `"ok": true`; the `layer_link` shots look as before.
- Identity sweep (8.5, `target\sticker\p12check\sweep\`): 21 / 21 ok. Against the P0 baseline
  (`compare_p0.txt`): 214 the same, the two known `side_tabs` small shots and `shot_tab_sticker.png` (since P6b), as
  in every check since P10a. Against the P11 check's sweep (`compare_p11.txt`): **217 / 217 the same**.
- Docs: ARCHITECTURE 6.13 (the no-op rule), 8.16 (busy: an open texture stroke for a layer link), 8.16 Paste (waits
  for a stroke), 9.2 (the checker tests).
- Programs started: cargo, Python (`-E -P`), the WTE exe only with `--screenshot ... --scenario ...`.
