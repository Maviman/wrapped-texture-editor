# Build note: P6b - Sticker tab, styles UI, tray (STICKER_PLAN 7 "P6b", 3.6-3.9, E3-E7)

Builder, 2026-11-26, branch `stickers` (from 9f51638, the P6a check; plan revision 3). Kind U. The side panel has a
fifth upper tab, **Sticker** (two rows of tabs at the default 300-pt panel): the selected stamp (thumbnail, kind, Edit
Sticker, Export Picture...), its **stamp-level styles** for image, text and sticker stamps (preset row, Copy / Paste /
Clear Style, the nine-block style editor; one slider edits every selected stamp) and the **sticker tray** (New Sticker,
from a picture, click = the Stamp tool's source, Edit, Duplicate, Delete, Export Picture...). The Stamp tool gets
**Edit before placing** and a **Style** for new stamps; the Sticker view asks **"Edit all / Only this one"** before
the first content edit of a shared sticker or picture (E6); in the view the Tool tab edits the active layer's styles.

## What was built

| Item (plan P6b) | Where | Notes |
|---|---|---|
| Five tabs, two rows (3.9, review A13) | `panels/side_groups.rs`, `state.rs` | `UpperTab::Sticker` ("sticker", index 4); `upper_tabs_fit_one_row` measures the labels with the button font; else the fixed rows `UPPER_ROWS` (Tool / Color / Texture sets, Objects / Sticker); the strip's height (24 / 51 pt) feeds `upper_keep`, `layers_area_max`, `tab_area_height`, `upper_min_height`; `SideRects::{upper_tab_buttons: [Rect; 5], upper_strip_height, sticker_tab}` |
| The Sticker tab (3.9) | new `sticker/panels.rs` | selected-stamp block, Sticker style block, tray; every control has a `PanelHit` |
| Style editor (2.3, 3.2, 3.7) | new `sticker/styles_ui.rs` | nine blocks with an on/off box in the header (`CollapsingState`), numbers in the body; first tick = defaults scaled by reference side / 512, untick keeps the numbers; all sliders `SliderClamping::Edits` + `continuous_edit`; the text stamp's Stroke note of 2.3; preset row |
| Stamp styles commands | `sticker/mod.rs` | `SetStampStyles` (never promotes, E3), `ApplyPreset` (scaled per stamp), `CopyStyle` / `PasteStyle` / `ClearStyle` with `StyleTarget::{Stamps, Layers}`, `AppState::style_clipboard`, `selected_stamps` |
| Layer styles in the view (3.2) | `sticker/layers_ui.rs` | "Layer styles" under X / Y in the Tool tab: presets (`ApplyLayerPreset`), Copy / Paste / Clear (layers), editor (`SetLayerStyles`, a content edit) |
| Tray, Export Picture (3.8) | new `sticker/tray.rs`, `io_flows::pick_save_png` | `tray_stickers` (kept or used), New Sticker (512 x 512, one empty layer), From picture..., `TrayUse`, Duplicate, Delete (refused while used), `write_picture` (stamp: "what you see"; tray sticker: scale 1); thumbnails `PictureThumbs` (two per frame, value keys, dropped per document) |
| Edit before placing (3.6, E4) | `state.rs` (`StampToolSource`, `StampSettings::styles`), `tools/stamp_place.rs`, `panels/tool_options.rs`, `tray.rs` | the source is `None / Blob / Sticker`; a `Blob` with default styles places today's `StampSource::Image`; a sticker source places `StampSource::Sticker`; every Stamp-tool placement goes through `add_stamp_styled`; "Edit before placing" makes a picture source a kept tray sticker, then opens the view on it (`StickerTarget::Tray`, "Not placed yet") |
| Shared-picture question (E6) | `sticker/mod.rs` (`shared_users`, `ask_shared`, `answer_shared`), new `dialogs/sticker_shared.rs`, `dispatch::close_dialog` | content commands wait for the answer; Edit all / Only this one (detach first for a sticker stamp) / Cancel; answer kept for the opening of the editor |
| Edit menu (3.7) | `panels/menu_bar.rs` | Copy Style / Paste Style / Clear Style (layers in the view, stamps outside) |
| Tool page image row | `panels/tool_options.rs` | "Image: name [Choose...] [Edit]" |
| Core additions | `doc_stamps.rs`, `doc_stickers.rs` | `add_stamp_styled` (`add_stamp` = it with default styles), `set_styles_of` / `set_styles_each` (`set_stamp_styles` = `set_styles_of(&[id])`), `stamp_style_reference_size` |
| Runner | `selftest/mod.rs` | `Step::ScreenScroll { at, notches }` (scenarios scroll a side-panel page to a control) |
| Scenarios | new `selftest/scenarios/sticker_tab.rs` | `sticker_styles`, `sticker_tray`, `sticker_controls` (NAMES 59); `side_tabs` clicks the fifth tab too (shot `tab_sticker`) |
| Split clearance (found while building) | `panels/side_groups.rs` | pages end 6 pt above the split edge (see deviation 9) |
| Docs | ARCHITECTURE 1.11, 2.1, 2.2, 5.20, 8.2, 8.3, 8.7, 8.8, 8.9, 8.11, 8.15, 9.1, 9.2; HANDOFF 4.1 (59 scenarios), 5 (row "stickers", new 5.4 with the landing rule and the owed hand tests), 6 (the sticker decisions, the PSD-editor note, five tabs); `dist/HOW_TO_TEST.txt` "Stickers, part 1"; README "Stickers" | |

Tests added: core 4 (`k6b_1`..`k6b_4`); app 21: `sticker::tests_p6b` (9), `control_sweep::sticker_panel_tests` (5),
`control_sweep::panel_tests::the_two_row_strip_keeps_the_side_panel_guarantees` (and `upper_tabs_show_one_page_each`
rewritten for five tabs, one row at 420 pt and two rows at 300 pt), `styles_ui` (2), `dialogs::sticker_shared` (1),
`tools::stamp_place` (1), `prefs` / `side_groups` / `state` / `selftest` tests extended (unknown tab name -> Tool as on
the P0 code; "sticker" reads back; indices 0..=4; source and styles never serialized; `ScreenScroll`).

## Deviations from the plan, with reasons

1. **`StickerTarget::ToolSource` is not built.** The plan itself turns a blob source into a kept tray sticker first, so
   the tool's unplaced sticker *is* a tray sticker: Edit before placing opens `Tray(id)`. One target kind less, same
   behaviour.
2. **Not in the Sticker tab yet:** "Edit in PhotoCraft", the link status line (P9a / P9b), the "PSD" drag handle (P11),
   the library and Save to Library (P8), New Sticker from the clipboard (P10b). Nothing disabled is shown for them.
3. **Thumbnails show the base picture without stamp styles** (sticker: `sticker_picture(.., None, ..)`; image: the
   blob; text: a "T"): the styled picture of a large stamp would be a full style pass on the UI thread. Export Picture
   writes the styled picture.
4. **"Apply to all selected" sends the whole edited stack** to every selected stamp (they become equal); the plan
   leaves open whether only the changed field is merged. A warning line says so when their styles differ.
5. **A preset on several stamps is scaled per stamp** (each picture's reference size), so a new core command
   `set_styles_each` was added next to `set_styles_of`.
6. **Core additions in a U package** (as P6a): `add_stamp_styled` (placing with the tool's styles in one entry; the
   automatic depth fitted to the shadow), `set_styles_of` / `set_styles_each` (one entry over several stamps; the
   existing `set_stamp_styles` delegates, behaviour unchanged: the P3a tests pass), `stamp_style_reference_size`.
7. **The Stamp tool's styles are a list** (None and the presets, "Custom" when edited elsewhere) instead of a second
   preset row: the Tool page is narrow; the full editor is on the Sticker tab for placed stamps.
8. **E6 details the plan does not fix:** the waiting edit runs after the answer as a Final (its drag is over); a Move
   drag's Preview is not replayed (the user drags again); an unchanged name never asks; a tray target never asks.
9. **Split clearance (a pre-existing bug the two-row strip exposed).** `side_tabs` at 640x480 failed: the split did
   not move when dragged from its edge. The scroll area of an upper page that overflows senses drags and took the press
   in the 3 pt of the resize handle above the edge; with one row the Tool page fit at 640x480, with two rows it
   overflows. It was reproduced with one row too (any overflowing page). Fix: every `page()` ends 6 pt above the edge
   and `upper_keep` adds 6 pt (G1 / G2 still hold: all panel tests pass). Test
   `the_split_drags_from_its_edge_while_the_upper_page_overflows`.
10. **G1-G4 in one row** are checked in a 420-pt panel at 1280x800 and 900x600 only: a 420-pt panel does not fit beside
    the views of a 640-pt window. The 640x480 page minimum holds with two rows, so the plan's fallback (the split one row
    lower) is not needed.
11. **`sticker_styles` checks the artwork box of the stored placement, not its depth**: an automatic depth is refitted
    to the shadow by plan 2.6, which writes the depth into the stored placement.
12. **The Edit menu's style items are not clicked in `sticker_controls`** (menus have no `panel_rects`); the control
    sweep test clicks them through AccessKit.
13. **Paste / Clear on layers** use the labels "Paste Layer Style" / "Clear Layer Style"; on stamps the core's "Paste
    Style" (one entry); a paste of the same styles is no entry.

## Numbers measured

- `cargo test --workspace --no-fail-fast` (full run before the last two app-only changes): **1,695 passed, 1 failed
  (WA20 only), 62 ignored** (`wte_core` lib 1,070 / 1 / 38, `wte_app` 568 / 0 / 24, integration 15 + 9 + 8 + 25);
  against the P6a check (1,673 / 1 / 62): +4 core, +18 app. After the split-clearance fix and the boxed style
  actions, `wte_app` again: **569 passed / 0 / 24** (+1 test); the core did not change after the full run.
- `cargo clippy --workspace --all-targets`: 0 warnings (final source; an `AppAction` size warning from the first
  version was fixed by boxing the `StyleStack` of `SetStampStyles` / `SetLayerStyles`).
- `cargo build --release -p wte_app`: OK, exe SHA-256 `3f4486494d61962cdd49de252b6a1c1666f6d2f0f28d9613cf689253ab029d12`.
- Scenarios with that exe (`target\sticker\p6b\final\`, off-screen, no focus taken), all `"ok": true`:
  `sticker_styles` (138 frames), `sticker_tray` (205), `sticker_controls` (744), `side_tabs` at 1280x800 (298),
  900x600 and 640x480, `sticker_view` (191), `sticker_keys` (516), `smoke`.
- Shots looked at against their `expect`: `styles_image` (logo with a thin white border and a soft shadow, WTE plain;
  the Sticker tab in the second row, Classic sticker, Die-cut contour and Drop shadow ticked) matches;
  `styles_both` (WTE with a white border, a dark stroke around it and a shadow; Selected stamp WTE, Text stamp,
  contour / stroke / shadow ticked) matches - the faint box around WTE is the gizmo's projection box (front and back
  rectangles), which grew with the refitted depth, not baked pixels (checked: the linked layer has no alpha there);
  `tray_edit_before` (logo + red disc on the canvas, plain sphere, "Not placed yet", two rows, Layer styles in the Tool
  tab) matches; `tray_placed_twice`, `tray_question` (the dialog lies over the two stamps of the 3D half, which was
  first mistaken for a missing render: a shot three frames after entering shows both stamps), `tray_only_this`,
  `controls_tab` (every style on), `controls_view` (Layer styles, Stroke unticked), `side_tabs_tab_sticker`,
  `side_tabs_small_640` match.
- Frame spans (`WTE_FRAME_LOG=1`): worst `sticker.canvas` 2.47 ms, `sticker.render` 2.37 ms, side panel (Sticker tab
  with thumbnails) 3.13 ms; worst whole-frame UI CPU 28.7 ms (a frame with a style Preview); limit 50 ms (8.4).
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with the final exe and `tools/sticker/jobs_identity.txt` into
  `target\sticker\p6b\sweep\`): **21 / 21 `"ok": true`**; `compare_baseline.py` against `target\sticker\baseline\sweep`:
  214 of 216 files the same, plus the new `side_tabs/shot_tab_sticker.png`. The two others are
  `side_tabs/shot_small_640.png` / `shot_small_900.png` (compared whole): 0 differing pixels in the central view below
  the mode strip (numpy); the differences are the mode strip (rows 25..50, its "Sticker" button since P6a) and the side
  panel (the two-row strip and the split clearance), both outside the compared area of 8.5. Exports, saved layer PNGs
  and manifests: all the same (`target\sticker\p6b\compare.txt`).
- Programs started: cargo, Python and the WTE release exe (P6b's and the P0 baseline exe, only with `--screenshot ...
  --scenario ...`, off-screen, no focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No new
  crate; `Cargo.toml` / `Cargo.lock` unchanged. New files LF; every edited file was LF and stays LF.

## Open (for the checker)

- The tablet rule's 24-pt hit targets (P6a check's open item) still applies to the new buttons (egui's 20-pt height).
- The Sticker tab is long: at 1280x800 the tray sits below the nine style blocks (the page scrolls); a collapsible
  "Sticker style" section or the tray above the editor could be considered.
