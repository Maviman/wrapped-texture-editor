# Build note: P8 - Layout tools, text, shapes, clipping, adjustments, library (STICKER_PLAN 7 "P8", 3.2, 3.3, 3.8)

Builder, 2026-11-26, branch `stickers` (from cb5bfcd, the P7b check; plan revision 3). Kind U with a core part. The
Sticker view can now lay a sticker out: the Move tool's **transform handles** and numbers (W, H, Angle, Flip H / V,
Reset), the **Crop** tool with the artboard shapes Rectangle / Rounded / Ellipse, the **Text** and **Shape** tools (text
layers edited by the Text block and the Text tab), the whole-sticker commands **Trim, Add Padding, Canvas Size, Resize,
Rotate 90 CW / CCW / 180, Rotate..., Flip, Flatten**, the layer commands **Merge Down** and **Clip to Layer Below**, the
**Adjustments** block, the **sticker library** of the Sticker tab (Save to Library, add from the library), **Export as
PSD** and the Sticker tab's **Convert to layered sticker** for text stamps.

## What was built

| Item (plan P8) | Where | Notes |
|---|---|---|
| Layout maths | new core `sticker/layout.rs` | `LayerBox` (where a layer's content box lands: picture, shape, text frame; exactly the placement `render` draws), `resize`, `rotate`, `flip`, `canvas_size`, `pad`, `limited` |
| Layout commands | new core `doc_sticker_layout.rs` | `trim_sticker`, `pad_sticker`, `canvas_size_sticker`, `crop_sticker` (shape, Delete cropped pixels), `resize_sticker`, `rotate_sticker`, `flip_sticker`, `flatten_sticker`, `merge_down_sticker_layer`; read-outs `sticker_layered`, `layer_box_of` / `sticker_layer_box`, `sticker_psd_bytes`; one entry each through `set_sticker_with(.., Refit)` (refactored out of `set_sticker`, which is `Refit::Artboard`) |
| Library preview | core `sticker/file.rs` | `read_wtes_preview` (name + `preview.png`, never the layers) |
| Move / Transform (3.3) | `sticker/tools/transform.rs`, canvas | eight scale handles + rotate handle; corners keep the aspect (Keep aspect, Shift inverts), edges one axis, Shift = 15 degree steps; `CanvasDrag::Transform` (Preview calls, one Final "Transform Sticker Layer"; Esc / Undo / Save rules of a Move drag); W / H / Angle / Flip H / Flip V / Reset in the Tool tab |
| Crop (3.3, E2) | `sticker/tools/crop.rs` | box with eight handles, move inside, new box outside, aspect presets 1:1 / 4:3 / 16:9, shapes, Delete cropped pixels (off), Enter / Apply crop, Esc / Cancel |
| Text (3.3) | `sticker/tools/text.rs`, `panels/text_block.rs`, `text_tab.rs` | a click adds a text layer (Text tool settings, primary colour, em = shorter side / 4) and focuses its content field; a click on a letter selects; `TextTarget::Layer` for the Text block and Text tab (`SetLayerText`: one entry per typing burst / drag; Size in px per em; Angle = the layer rotation); the layer name follows its text |
| Shape (3.3) | `sticker/tools/shape.rs` | drag adds a shape layer (kind, fill none / colour / gradient, outline, radius, sides / points / inner, line width); the options edit an active shape layer |
| Sticker and Layer commands | `sticker/layout.rs`, `filters.rs` (`COMMANDS` 46, groups Sticker and Layer, `Needs::Below` / `Vector`), `popover.rs` (Trim..., Add Padding..., Canvas Size..., Resize..., Rotate...), menu (`Group::MENU`) | Merge Down (Ctrl+M, the footer's Merge, the row menu), Clip (footer Clip / Unclip, row menu), Bring Forward / Send Backward, Copy / Paste / Clear Layer Style also in the Layer group |
| Style editor per layer, inner shadow, gradient overlay, bevel | already built by P6b (`styles_ui.rs` has all nine blocks) | checked, nothing to add |
| Adjustments UI (3.2) | `sticker/adjust_ui.rs` | list with sliders, Up / Down / Remove, "Add adjustment..." (7 kinds), at most 16, one entry per drag |
| Library (3.8) | `sticker/library.rs`, Sticker tab, tray | `Prefs::sticker_library_dir` (default `%APPDATA%\WrappedTextureEditor\stickers`, never in a self-test / unit test), listing on a job thread, one thumbnail per frame, Choose folder... ("(on the PC)" with a companion), Refresh, Save to Library (tray and the Sticker group), a click adds to the tray (kept, one entry, the Stamp tool's source and the file's styles) |
| Export as PSD (3.8) | `sticker/layout.rs`, `io_flows::pick_save_psd` | `sticker_psd_bytes`: the link writer at the link scale, no link made |
| `convert_text_stamp` button | Sticker tab (`StickerTabConvertText`) | the canvas header and Layers tab had it since P6a; now also outside the Sticker view |
| Layers tab | `layers_ui.rs` | footer Text, Shape, Clip, Merge; the row context menu (Duplicate, Delete, Merge Down, Rasterize, Clip, Copy / Paste Style) |
| Canvas | `canvas.rs` | the routing of the four tools, the transform box, the crop box with the darkened outside, the shape drag box; the camera is fitted again when the artboard changes size (`fitted_size`) |
| Scenarios | new `selftest/scenarios/sticker_layout.rs` | `sticker_text`, `sticker_layout`, `sticker_library` (NAMES 65); `sticker_keys` updated (Ctrl+M merges now) |
| Docs | ARCHITECTURE 1 (summary, file tables), 5.20 "Layout", 5.27, 8.6, 8.7, 8.9, 8.10, 8.11, 8.15, 9.1, 9.2; HANDOFF 4.1 (65 scenarios), the branch table, 5.4; `dist/HOW_TO_TEST.txt` "Stickers, part 4" | |

Tests added: core 14 (`k8_*` in `sticker::layout` 5 and `doc_sticker_layout_tests.rs` 8, plus the ignored
`perf_k8_layout_commands`); app 18 (`sticker/tests_p8.rs` 12; `tools::crop::tests`, `tools::shape::tests`,
`tools::text::tests`, `adjust_ui::tests`, `library::tests` 2). Changed: `sticker::tests::layer_commands_are_one_entry_each_and_keep_their_rules`
(Ctrl+M merges), `sticker::tests::a_locked_layer_and_an_unbuilt_tool_do_nothing` (every tool is built; the Crop starts its
box), the scenario count (65), `sticker_keys` (the Ctrl+M step and its shot's expect).

## Deviations from the plan, with reasons

1. **Files.** Plan: `sticker/tools/{crop,text,shape,transform}.rs`, `sticker/{adjust_ui,library}.rs`. Built as listed,
   plus `sticker/layout.rs` (the Sticker / Layer commands and Export as PSD) and the core `sticker/layout.rs` +
   `doc_sticker_layout.rs` (the commands are document commands, so they are core and unit-tested headless).
2. **Resize keeps the placement** (`Refit::Scaled`). 2.9 lists resize among the refits, but the refit keeps the on-model
   size of one layer-space pixel, so a 200 % resize would have doubled the sticker on the model, and the scenario asks
   "the marked point stays on its texel". Resize therefore keeps every user's placement and scales the users'
   stamp-level style lengths by the mean factor (their reference size is the artboard, 2.3), so the model looks the same
   and the sticker has more pixels. `set_sticker` was split into `set_sticker_with(.., Refit)`; its behaviour for every
   existing caller is unchanged (`Refit::Artboard`).
3. **Rotate's mark.** A turn moves the artwork on the model (that is what a turn is for), so in `sticker_layout` the mark
   is asserted to move by at least 4 texels after Rotate 90 CW and to be back on its texel after Rotate 90 CCW. Quarter
   turns use a pivot half a pixel off the centre when the sides differ in parity (rule in ARCHITECTURE 5.20), so the new
   artboard has whole-pixel corners and CW / CCW / four turns are exact (tests).
4. **Shift on the corner handles.** The plan says "Shift keeps aspect, on by default": built as a "Keep aspect" option
   (on by default) that Shift inverts while dragging (Photoshop CC 2019+ behaviour); edges always scale one axis.
5. **Trim = visible content inside the artboard** (the finished picture's alpha > 0), as Photoshop's Trim; content
   outside the artboard is not looked at.
6. **Merge Down / Flatten bake** what they render at scale 1 over the artboard: content outside the artboard is not
   kept; Merge Down renders the two layers alone, so the lower layer's own blend mode against the layers below and its
   clipping to a base further down are baked as drawn alone (documented in ARCHITECTURE 5.20). Flatten keeps the
   artboard shape and renders without it, so its picture is byte-identical (test).
7. **Crop "Delete cropped pixels"** cuts only picture layers that are just moved (scale 1, no turn, whole-pixel
   position); a scaled or turned layer keeps its pixels (cutting it would need a resample).
8. **Text layers in the Text tab:** the per-gap selection and the gap strip's per-gap editing stay for placed texts
   (`TextTarget::Layer` has no `TextGapSel`); a text layer's content field is a plain text buffer, and gap values
   whose characters go are dropped by `sanitize_fx` (a placed text's `GapBuffer` moves them with their characters).
9. **The Text tool focuses the content field in the Tool tab** (the Text block), not the Text tab: the Layers tab stays
   visible. Typing goes into the text; Esc leaves the field (the scenario presses Esc before the next tool key).
10. **"Help text"**: there is no in-app Help page for stickers yet (P13 writes Help); the help of P8 is the Tool tab's
    hint lines, the tooltips and `dist/HOW_TO_TEST.txt` "Stickers, part 4".
11. **The numbers commands use popovers** (P7b's small window, no preview) rather than modal dialogs: Trim... (padding),
    Add Padding..., Canvas Size... (W, H, a 3 x 3 anchor), Resize... (percent or W / H, Keep aspect), Rotate...
12. **Library: a stamp's styles.** "Save to Library" of the edited sticker keeps its stamp's styles in the file (2.12);
    adding such a file sets them as the Stamp tool's styles (the next placed stamp looks as saved).
13. **Layer group in the Sticker menu** has its own New Picture Layer / Duplicate / Delete...; the main Layer menu keeps
    mapping its items to the sticker in the view (P6a check), unchanged.

## Numbers measured

- `cargo test --workspace --no-fail-fast` (final code): **1,776 passed, 1 failed (WA20 only:
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`), 65 ignored** (`wte_core` lib 1,097 / 1 /
  41, `wte_app` 622 / 0 / 24, integration 15 + 9 + 8 + 25). Against the P7b check (1,745 / 1 / 64): +13 core, +18 app,
  +1 ignored (perf). Log `target\sticker\p8	est2.log`.
- `cargo clippy --workspace --all-targets`: 0 warnings (`target\sticker\p8\clippy.log`).
- `cargo build --release -p wte_app`: OK, exe 31,782,400 bytes, SHA-256
  `8aa78f3af729355373e1c120c416b709de9442c8ce5b22d9746ab081c949cfb5`.
- `perf_k8_layout_commands` (release, best of 3; 2048 x 2048 sticker: a white picture and a styled red square): Trim
  35.7 ms, Rotate 90 < 0.1 ms, Resize < 0.1 ms (numbers only; the users' re-bakes are jobs), Merge Down 104.2 ms,
  Flatten 104.3 ms. All are one-click commands on the UI thread (like Crop to Selection); 8.4 sets no budget for them.
- Scenarios with the final exe (`target\sticker\p8\run2\`, off-screen, no focus taken, `WTE_FRAME_LOG=1`): `smoke`,
  `sticker_view`, `sticker_keys`, `sticker_styles`, `sticker_tray`, `sticker_controls`, `sticker_edit`, `sticker_pen`,
  `sticker_cutout`, `sticker_text`, `sticker_layout`, `sticker_library`: all `"ok": true`.
- Shots looked at against their `expect`: `text_clipped` (yellow star on the left, "H I" filled with magenta / cyan
  stripes, nothing between the letters; four rows, the top one "clipped") matches; `text_warped` (the striped "H I !"
  bent by the Arc warp, the star, the Shape options in the Tool tab) matches; `text_converted` (the sphere with both
  stickers, the Sticker tab "Sticker, 1 layer" for WTE) matches; `layout_crop_box` (the box with eight handles and the
  ellipse, the Crop options with Ellipse) matches; `layout_rotated` (the ellipse upright, the sphere turned the same way)
  matches; `layout_done` (one "Flattened" row, the sphere unchanged, the camera fitted to the 360 x 200 artboard)
  matches; `library_saved` (tray thumbnail "LIB", the Library section with the folder and one thumbnail) matches;
  `library_placed` (the "LIB" sticker on the sphere, the Sticker tab with the stamp) matches; `keys_inside` (one row
  after the merge, the transform box with its handles) matches the updated expect.
- Frame spans (`tools/sticker/frame_spans.py`, limit 50 ms): `sticker_text` worst `sticker.canvas` 2.57 ms,
  `sticker_layout` 2.31, `sticker_library` 3.64, `sticker_view` 0.86, `sticker_edit` 2.74 (render).
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with the final exe and `tools/sticker/jobs_identity.txt` into
  `target\sticker\p8\sweep\`): **21 / 21 `"ok": true`**; `compare_baseline.py` against `target\sticker\baseline\sweep`:
  214 of 216 the same plus the NOTE of `side_tabs/shot_tab_sticker.png`; the two known whole-picture differences
  `side_tabs/shot_small_640.png` / `shot_small_900.png` (mode strip, two-row tab strip) are byte-identical to the P7b
  check's sweep; `shot_tab_sticker.png` differs from P7b's by the new tray button "Save to Library" and the Library
  section ("No library folder: choose one.": a self-test never lists the real folder). Exports, saved layer PNGs and
  manifests: all the same (`target\sticker\p8\compare.txt`).
- Programs started: cargo, Python and the WTE release exe (only with `--screenshot ... --scenario ...`, off-screen, no
  focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No new crate; `Cargo.toml` /
  `Cargo.lock` unchanged. New files LF; every edited file kept its line endings (all were LF).

## Open (for the checker)

- Merge Down / Flatten / Trim render on the UI thread (about 100 ms at 2048 px, more at 4096 px); a job like the
  popover preview's would keep the frame smooth for big stickers.
- The 24-pt tablet hit targets (open since P6a) apply to the new handles too: the transform and crop handles are 8-pt
  squares with a 9-pt grab radius (18 pt across), below 24 pt.
- The Text tab's per-gap editing is not offered for text layers (deviation 8).
- The Text tool's click selects a text layer only on its ink (`sticker_layer_at`); a click between letters adds a new
  text layer.
- Resize changes the users' stamp-level style lengths in the same entry (deviation 2): a stamp sharing the sticker gets
  them too (it shows the same sticker, so it should).
