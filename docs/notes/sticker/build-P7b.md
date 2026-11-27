# Build note: P7b - Selections and cut-out (STICKER_PLAN 7 "P7b", 3.3, 3.4, 4.4, 4.5, 4.9)

Builder, 2026-11-26, branch `stickers` (from ded3658, the P7a check; plan revision 3). Kind U, with a small core
addition. The Sticker view can now select and cut out: the **Marquee**, **Lasso** and **Magic Wand** (Shift adds, Alt
subtracts, both intersect, or the Tool tab's New / Add / Subtract / Intersect row), the selection's dashed outline,
Ctrl+A / Ctrl+D / Ctrl+Shift+I, **Delete** of the selected pixels (never the stamp), the **Magic Eraser**, **Move
Selection** (a Move drag inside the selection cuts the pixels into a new layer and moves it), **Layer via Copy / Cut**,
**Crop to Selection**, Grow / Shrink / Feather / Color Range / From Layer Alpha, the Cut-out commands (**Remove
Background**, **Color to Alpha**, **Defringe**, Remove White / Black Matte) and the Filter commands (**Blur**,
**Sharpen**), the commands with numbers in small popovers with a live preview, and a **Sticker** menu in the menu bar
while the Sticker view is shown.

## What was built

| Item (plan P7b) | Where | Notes |
|---|---|---|
| Marquee (3.3) | `wte_app/src/sticker/tools/marquee.rs`, canvas | `CanvasDrag::Marquee`; the box / ellipse drawn while dragging; Feather, Fixed aspect W : H; a click drops the selection (New mode) |
| Lasso (3.3) | `tools/lasso.rs` | freehand drag (closes at the release) or polygon clicks closed by a double-click, a click on the first point, Enter or "Close lasso"; Esc / "Cancel" drop it; Feather |
| Magic Wand (3.3, 4.4) | `tools/wand.rs` | Tolerance, Contiguous, Anti-alias, "Sample all layers": the active layer alone on the artboard or the unstyled composite (new core read-outs) |
| Modes | `marquee::mode_of`, `tools::set_selection`, Tool tab row | the keys at the press (the press event's modifiers), else `StickerUi::select_mode` (tablet rule: every mode has a button) |
| Magic Eraser (3.3) | `tools/magic_eraser.rs` | one entry "Magic Eraser"; limited by the selection; "Into mask (keep pixels)" (adds a mask when there is none) |
| Move with a selection (3.3) | `tools/move_selection.rs`, canvas | a Move press inside the selection on a picture layer: the cut and every position fold into one entry "Move Selection"; the selection moves with the pixels; Esc / Undo in the drag take everything back |
| 3.4 | `outline.rs`, `state.rs` | the selection's own outline (edge runs at 128, cached per selection, white with black dashes, no animation); `StickerEdit::selection` is now `Option<Arc<Selection>>` |
| Selection commands | `commands.rs`, `tools/wand.rs`, `tools/move_selection.rs` | All, None, Invert, Grow / Shrink / Feather, Color Range..., From Layer Alpha, Delete ("Delete Selected Pixels"), Crop to Selection, Layer via Copy, Layer via Cut |
| Cut-out and Filter commands | `filters.rs` | Remove Background (the Tool tab's Tolerance / Follow gradients / Also enclosed areas / Into mask), Color to Alpha, Defringe, Remove White / Black Matte, Blur, Sharpen; one entry each; `compute` shared with the preview |
| Popovers with live preview | `popover.rs` | Color to Alpha (White / Black / Primary, colour button, a canvas click picks), Defringe, Blur, Sharpen, Color Range (live outline), Grow / Shrink / Feather (live outline); Apply / Cancel / Esc |
| The "Sticker" menu | `panels/menu_bar.rs` | Selection, Cut-out and Filter submenus from `filters::COMMANDS`; only in the Sticker view |
| Keys | `shortcuts.rs` | Ctrl+A / Ctrl+D / Ctrl+Shift+I in `STICKER_SHORTCUTS` (34 entries); Delete / Backspace clear the selected pixels; Enter closes a lasso; Esc: drag, popover, lasso, (dialog / pending), selection, leave |
| Tool tab | `sticker/layers_ui.rs` | options of the four new tools; collapsible Selection / Cut-out / Filter blocks at the end of the page |
| Core | new `wte_core/src/doc_sticker_cutout.rs` | `sticker_layer_on_artboard`, `sticker_unstyled_composite`, `sticker_picture_with` (read only, never a promotion) |
| Scenario | new `selftest/scenarios/sticker_cutout.rs` | NAMES 62 |
| Docs | ARCHITECTURE 1.12, 2.1, 5.20, 8.6, 8.7, 8.10, 8.11, 8.15, 9.1, 9.2; HANDOFF 4.1 (62 scenarios), 5.4, the branch table; `dist/HOW_TO_TEST.txt` "Stickers, part 3: selections and cut-out" | |

Tests added: core 4 (`k7b_*` in `doc_sticker_cutout_tests.rs`) and the ignored `perf_k7b_wand_sampling_and_preview`;
app 15 (`sticker/tools/tests_p7b.rs`: Marquee by keys and by the row, click drops, ellipse, fixed aspect, no entry and
no promotion; Lasso freehand / polygon / first-point close / Cancel / too few points; wand on the active layer and on
all layers, Shift adds, outside the artboard; Color Range picked on the canvas, previewed, applied; Magic Eraser in a
selection, promoted, undone, Into mask; Delete with and without a selection with the stamp selected, unchanged pixels
no entry, on a mask; Move Selection cancelled and whole; Layer via Copy / Cut; the selection commands and Crop to
Selection; the Cut-out / Filter commands with labels, promotion, no white halo, the refusal, Blur limited by the
selection; the Blur popover's preview equals Apply's result, Cancel restores the canvas, Color to Alpha picked on the
canvas; popovers close with another active layer; the outline runs, cache and fallback; the command list on a text
layer; the shared-picture question before Delete, Magic Eraser, Remove Background and Layer via Cut, then the waiting
edit). Changed: `shortcuts::tests::colliding_chords_run_the_action_of_their_mode` (+3 chords in both modes),
`sticker::tests::a_locked_layer_and_an_unbuilt_tool_do_nothing` (the unbuilt tool is now the Crop), the scenario count.

## Deviations from the plan, with reasons

1. **Files.** The plan lists `sticker/tools/{marquee,lasso,wand,magic_eraser,move_selection}.rs` and
   `sticker/commands.rs`: built as listed, plus `sticker/filters.rs` (the Cut-out and Filter commands and the one
   command list of the menu and the Tool tab), `sticker/popover.rs` and `sticker/outline.rs` (commands.rs would have
   grown to ~900 lines). Color Range lives in `wand.rs` (it is the wand with Contiguous off).
2. **Popovers are small non-modal `egui::Window`s over the canvas** with Apply / Cancel, not egui popups: a popup closes
   on any outside click, which would make "click the canvas to pick the colour" impossible. The live preview is drawn
   **in the canvas only** (the model changes at Apply): it runs the command's own `filters::compute` and the new core
   read-out `sticker_picture_with`, so the preview is byte-for-byte the result (test), and the document, its history
   and the pending-edit machinery are never touched by a preview (a pending Preview edit would have been flushed or
   cancelled by unrelated commands while the popover stays open).
3. **The preview runs on the UI thread**, at most every 100 ms while a slider is held. Measured (release): a Blur 16
   preview of a 2048 x 2048 sticker with a ClassicSticker layer costs **255 ms** (blur ~50 ms + the styled render); on
   the 256-px scenario sticker the `sticker.popover` span is under 4 ms. See "Open".
4. **Color to Alpha's "eyedropper"** is a click on the canvas while the popover is open (the plan: "pick the colour
   with the eyedropper or White / Black buttons"); a Primary button was added.
5. **Spelling**: the UI says "Color to Alpha" (as "Color Range", the "Color" tab, "Swap Colors"); the plan writes
   "Colour to Alpha". The core's Remove Background refusal sentence was changed to the same spelling (no test used it).
6. **Move Selection only starts inside the selection.** A Move press outside the selection (or on a text / shape /
   hidden / locked layer) is the ordinary layer Move, so layers can still be moved while a selection exists
   (Photoshop's behaviour). Auto-select is not asked for a press inside the selection: the active layer's pixels move.
7. **Move Selection is `set_sticker` with `Commit::Preview` calls**, not `replace_sticker_pixels`: the cut changes two
   layers (the source loses the pixels, a new layer gets them), so it is a structure entry; the Preview calls fold into
   it, and Esc / another command cancel the pending entry (cut included).
8. **The new layer of a cut** copies the source's attributes (opacity, blend, clip, styles, adjustments, mask,
   transform); on a layer that is only moved (scale 1, no rotation) its picture and mask are cropped to the cut pixels
   and its position adjusted (a small piece of a 4096-px photo stays small). On a scaled / turned layer it keeps the
   full size and the same transform (cropping would move the rotation centre).
9. **Layer via Copy / Cut and Crop to Selection drop the selection** (Photoshop drops it after Layer via Copy; Crop to
   Selection changes the artboard's size, which drops it by 3.4 anyway). Move Selection keeps it, shifted.
10. **Invert with nothing selected selects everything** (Photoshop disables the item); Select All is the same.
11. **Blur and Sharpen refuse pictures over 4096 px a side** (`MAX_FILTER_SIDE`): blur keeps three 8-byte planes (check
    P2 note 5, about 1.6 GB at 8192 px); the status line says so. Blur acts on the mask with "Paint on mask"
    (`blur_mask`, plan 4.9); Sharpen always on the picture (there is no mask sharpen in 4.9).
12. **The Sticker menu is shown only in the Sticker view**, so the menu bar of every identity-sweep shot is unchanged.
    Its P8 parts (Sticker: Trim, Padding, ...; Layer commands) come with P8.
13. **Command blocks at the end of the Tool tab page** (after Layer styles): placed before them they pushed the P6b
    controls down and `sticker_controls` failed to reach its stroke toggle; at the end every earlier control keeps its
    place.
14. **`tools::put` makes no entry for an unchanged buffer** (and so does not promote an image stamp), with the status
    line "<label>: nothing changed" - a second Delete of the same pixels, Defringe without an edge.
15. **An empty selection is no selection**: every command that makes one (Marquee, Lasso, wand, Select Rect, Invert,
    Grow ...) stores `None` when nothing is selected, so "Delete without a selection" says what to do.

## Numbers measured

- `cargo test --workspace --no-fail-fast` (final code): **1,742 passed, 1 failed (WA20 only:
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`), 64 ignored** (`wte_core` lib 1,084 / 1 /
  40, `wte_app` 601 / 0 / 24, integration 15 + 9 + 8 + 25). Against the P7a check (1,723 / 1 / 63): +4 core, +15 app,
  +1 ignored (perf). Log `target\sticker\p7b\test.log`.
- `cargo clippy --workspace --all-targets`: 0 warnings (`target\sticker\p7b\clippy.log`).
- `cargo build --release -p wte_app`: OK, exe 31,081,984 bytes, SHA-256
  `076c1e661499c7d094f06f76ac3482e8ca18a83ead908e9e19381823c2437bad`.
- `perf_k7b_wand_sampling_and_preview` (release, this machine, best of 3; 2048 x 2048 sticker of a white layer and a
  styled red square): the active layer alone on the artboard + the wand's region **21.6 ms**; the unstyled composite +
  the region **56.2 ms** (8.4: wand 60 ms); Blur 16 preview (blur + styled render) **255.2 ms** (UI thread; see Open).
- Scenarios with the final exe (`target\sticker\p7b\run3\`, off-screen, no focus taken, `WTE_FRAME_LOG=1`):
  `sticker_cutout` (285 frames), `sticker_view`, `sticker_keys`, `sticker_styles`, `sticker_tray`, `sticker_controls`,
  `sticker_edit`, `sticker_pen`, `smoke`: all `"ok": true`. (Run 2, with the command blocks above the position fields:
  `sticker_controls` failed, deviation 13.)
- Shots looked at against their `expect`: `cutout_delete` (the disc on white, the top-left corner transparent with the
  dashed outline, the sphere's stamp with the corner missing, the Marquee options) matches; `cutout_removed` (the disc
  alone on the checkerboard, no white rim; the sphere without the white square) matches; `cutout_color_to_alpha` (the
  Color to Alpha window at the canvas's top-left with White / Black / Primary, Apply, Cancel; the preview with the second
  layer's white gone, the disc a shade darker) matches; `cutout_die_cut` (the sphere with the disc as a die-cut sticker:
  white border and soft shadow; the moved piece, the lighter hole, the soft left edge; three layer rows) matches; the
  final shot (back in 3D, the image stamp as at the start, no Sticker menu) as expected. The P6b `controls_view` shot
  still shows Move options, X / Y, Mask and Layer styles as its expect says.
- Frame spans (`tools/sticker/frame_spans.py`, limit 50 ms): `sticker_cutout` worst `sticker.canvas` 2.51 ms,
  `sticker.popover` 1.96, `sticker.outline` 0.42, `sticker.render` 1.98; `sticker_edit` 5.56 / live 2.53 / region 0.76;
  `sticker_view` 8.91.
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with the final exe and `tools/sticker/jobs_identity.txt` into
  `target\sticker\p7b\sweep\`): **21 / 21 `"ok": true`**; `compare_baseline.py` against `target\sticker\baseline\sweep`:
  214 of 216 the same plus the NOTE of `side_tabs/shot_tab_sticker.png`; the two others,
  `side_tabs/shot_small_640.png` / `shot_small_900.png` (compared whole), are byte-identical to the P7a sweep (the mode
  strip's "Sticker" button and the two-row tab strip, outside the compared area of 8.5). Exports, saved layer PNGs and
  manifests: all the same (`target\sticker\p7b\compare.txt`).
- Programs started: cargo, Python and the WTE release exe (only with `--screenshot ... --scenario ...`, off-screen, no
  focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No new crate; `Cargo.toml` /
  `Cargo.lock` unchanged. New files LF; every edited file was LF and stays LF.

## Open (for the checker)

- **The popover preview runs on the UI thread**: 255 ms per update for a 2048-px styled sticker (throttled to one per
  100 ms while dragging, so the frame stalls that long each time). A job like `live.rs`'s restyle would keep the UI
  smooth; not built (deviation 3). Small stickers (the scenarios) are far below the 50 ms frame limit.
- The tablet rule's 24-pt hit targets (open since P6a) applies to the new buttons too.
- Grow / Shrink threshold at 128 (check-P2 note 4): a faint "From Layer Alpha" selection (alpha < 128 everywhere) has no
  outline (its bounds are drawn) and does not grow; unchanged.
- The outline is cached per selection `Arc`; a selection with more than 50,000 edge runs is drawn as its bounds.
- Magic Wand / Color Range on "active layer" sample the layer drawn alone with its mask and adjustments (not its
  opacity or blend); "Sample all layers" ignores layer styles (plan 4.4).
