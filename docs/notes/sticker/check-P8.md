# Check note: P8 - Layout tools, text, shapes, clipping, adjustments, library

Checker, 2026-11-26, branch `stickers` at ead3747 (the P8 build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P8" and the sections it cites (3.3 rows Move / Transform, Crop, Text, Shape and the Sticker /
Layer commands; 2.9; 2.10; 3.8; 3.9 item 4; 4.6; 4.8; 8.2; 8.4; 8.5), with the user's decisions and plan revision 3.
Verdict: **pass after fixes** (one major fixed with a test; three minors fixed with tests; the minors left are listed at
the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,776 passed / 1 failed (WA20 only) / 65 ignored (core 1,097 / 1 / 41, app
  622 / 0 / 24, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\p8check\test.log`).
- `cargo clippy --workspace --all-targets`: 0 warnings. Release build: SHA-256 `8aa78f3a...`, as the builder said.
- Scenarios with that exe, off-screen (`target\sticker\p8check\run\`): `smoke`, `sticker_text`, `sticker_layout`,
  `sticker_library`, `sticker_keys`, `sticker_view`, `sticker_edit`, `sticker_cutout`, `sticker_styles`, `sticker_tray`,
  `sticker_controls`, `sticker_pen`: all `"ok": true`. Every new shot looked at (below).

## Plan items, one by one

| Item (plan 7 "P8" / cited section) | Found |
|---|---|
| Move / Transform (V): handles scale (aspect kept by default), rotate handle with Shift = 15 degrees, one entry per drag; numeric fields; Flip H / V; Reset; Auto-select (P6a); arrows (P6a) | yes; "Keep aspect" option that Shift inverts (deviation 4, accepted); W / H / Angle with 0, NaN, infinite and huge values tested; Esc / Undo / Save mid-drag follow the Move drag's guard |
| Crop (C): edges / new box, Enter applies one entry, Delete cropped pixels off by default, Rectangle / Rounded / Ellipse, aspect presets | yes, tested; Delete cropped pixels cuts only moved-only picture layers (deviation 7, accepted: no resample) |
| Text (X): click adds a text layer at the point (primary colour, Text tab settings), focuses the content field; click on a text layer selects | yes (the field is in the Tool tab's Text block: deviation 9, accepted); a click selects only on the letters' ink (open, minor) |
| Shape (U): drag adds a shape layer; kind, fill none / solid / gradient, outline colour and width, radius, sides / points | yes, the options also edit an active shape layer, tested |
| Sticker commands: Trim (padding), Add Padding, Canvas Size, Resize (percent or pixels; transforms, text sizes, shape sizes, style lengths; no resample), Rotate 90 CW / CCW / 180, Rotate... (any angle, bounding box), Flip H / V, Flatten | yes, one `Structure` entry each with the promotion inside (2.10), tested; Resize keeps the placements (deviation 2, accepted: the 2.9 refit would double the sticker on the model, against "the marked point stays on its texel") |
| Layer commands: Merge Down, Rasterize (P7a), Clip to Layer Below, Bring Forward / Send Backward, Copy / Paste / Clear Layer Style | yes; **Merge Down of two layers clipped to one base lost the upper one's pixels (major, fixed)**; Merge Down of a hidden layer threw pixels away (minor, fixed) |
| 2.9 refit for crop, trim, padding, canvas size, rotate | yes (`Refit::Artboard`, the old path of `set_sticker`); tests keep a layer-space point on its texel; Rotate turns about the centre |
| The style editor per layer; inner shadow, gradient overlay, bevel | built in P6b (`styles_ui.rs`, nine blocks), checked |
| Adjustments UI (2.4, 4.6) | yes, 7 kinds, at most 16, one entry per drag, tested |
| Library (3.8): folder in the prefs, default `%APPDATA%\...\stickers`, listed lazily on a job, at most one file per frame, click = add to the tray (kept), Save to Library, self-tests never the real folder | yes; **the job read every file whole and each entry kept its 512-px preview in RAM and on the GPU (minor, fixed)**; self-tests and unit tests get no folder without one in the prefs (checked: a self-test starts from `Prefs::default()` and never saves prefs) |
| Export as PSD (3.8) | yes (`sticker_psd_bytes`, the link writer without a link), tested by reading it back |
| `convert_text_stamp` button | yes (Sticker tab, also outside the view), scenario `sticker_text` |
| ARCHITECTURE 8.6, 8.7, 8.9, 8.11, 8.15, 9.2; HOW_TO_TEST; Help text | yes; the in-app Help chapter is P13's (deviation 10, accepted: P13's file list has the Help texts) |
| Scenarios `sticker_text`, `sticker_layout`, `sticker_library` | yes, with the plan's assertions; the text change and the warp are sent as the Text tab's action, not by clicking the Text tab (the Text block's typing is a unit test) |
| Checker: every numeric field with 0, negative, huge and non-numeric input; a sticker at each limit | DragValues clamp their ranges and ignore non-numbers; the core commands checked headless with a new test (below) |

## Findings

### Major (fixed): Merge Down of two layers clipped to the same base lost pixels

`merge_down_sticker_layer` drew the pair alone with the lower layer unclipped and the upper layer **clipped to the lower
one**. When both layers are clipped (two patterns clipped to one text: the common "pattern inside text" set-up), the
upper one was clipped to the base further down, not to the lower layer, so every pixel of the upper layer outside the
lower one vanished on Merge Down. Fixed: the upper layer is drawn clipped only when it is clipped and the lower one is
not; the merged layer keeps the lower one's clip flag, so the base cuts both again. Test
`k8_check_merging_two_layers_clipped_to_one_base_keeps_the_look` (two non-overlapping squares clipped to a white base:
the finished picture is byte-identical after the merge; it fails on the builder's code: the yellow square vanished).

### Minor (fixed): Merge Down of a hidden layer threw its pixels away

A hidden layer is not drawn, so merging a hidden upper layer dropped it and merging onto a hidden lower layer replaced
the lower one by the upper one, made visible. Now refused (no entry, the toast "a hidden layer cannot be merged: show it
first"; Photoshop refuses a Merge Down onto a hidden layer too). Test `k8_check_a_hidden_layer_is_never_merged`.

### Minor (fixed): the library read whole files and kept big previews

The listing job read every `.wtes` file completely (`std::fs::read`, up to 1 GiB each, layers and all) to get its
512-px preview, and every entry kept that preview as a `PixelBuf` beside its texture: up to 500 x 1 MB in RAM and
500 x 1 MB on the GPU for 52-pt thumbnails. Now `wte_core::read_wtes_preview_from` reads through a `BufReader` of the
file (the zip directory, the manifest and `preview.png` only), the job makes the preview at most 128 px a side
(`library::THUMB_PX`, `wte_core::thumbnail`), and an entry keeps the texture and its aspect only. Tests
`library_thumbnails_are_small` (a 512 x 256 preview listed as a 128 x 64 texture, aspect 2) and
`k8_check_a_library_preview_is_read_from_a_file_without_its_layers`.

### Minor (fixed): `sticker::layout::limited` overflowed

`limited` computed `b - a` in `i32`: a crop rectangle from the public API spanning more than `i32` holds panicked in a
debug build (wrapped in release). The UI cannot make one (`crop::box_rect` limits to 1e6), but `crop_sticker` is public.
Now in `i64`. Covered by `k8_check_every_command_at_the_limits_and_with_bad_numbers`, which also puts a 1 x 1 and a
4096 x 4096 sticker through Rotate (0, -90, 90, 180, 270, 360, 45, -1e9, 1e30; NaN and infinities refused without an
entry), Resize and Canvas Size (0, `u32::MAX`, NaN / infinite anchors), Add Padding and Trim (0 .. `u32::MAX`), Crop
(`i32::MIN..i32::MAX`, a NaN radius, Delete cropped pixels), Flip and Flatten: no panic, the artboard always within
1..=4096 a side, and undoing everything gives the start.

### Looked at and accepted (no change)

- Rotate with the 2.9 refit: the quarter turns' half-pixel pivot for sides of different parity is right (checked by
  hand for 101 x 60: both corner sums whole; CW and CCW share the pivot).
- `LayerBox::mirrored` / `flipped` / `with_scale` / `with_rotation` / the handle maths (`transform::dragged` in the
  layer's own frame) are right for turned and flipped layers.
- Resize's `Refit::Scaled`: stamp style lengths are in reference pixels (`stamp_style_reference_size` = the artboard at
  the reference scale), so scaling them by the mean factor keeps the look; the existing callers of `set_sticker` keep
  `Refit::Artboard` (identity sweep unchanged).
- The transform box cache compares the layer content: `PicturePixels` equality is a version / pointer test first, and
  the cache stores an `Arc` clone, so the frames after a change compare pointers.
- Crop, Trim and Add Padding re-derive an Ellipse / Rounded artboard shape from the new artboard (the shape belongs to
  the artboard, E2): in `layout_done` the ellipse is the padded artboard's, so the orange box shows flat sides. By design.
- The warped "H I !" in `text_warped` bends each stem by the envelope (Arc, bend 0.5): the same `render_text` path as a
  text stamp, not P8's.
- The new `Prefs::sticker_library_dir` is skipped when `None` (an old prefs file reads and writes the same).

## Screenshots looked at

`text_clipped` (yellow star; "H I" with magenta / cyan stripes and a thin white outline, nothing between the letters;
four rows, the top one "clipped"), `text_warped` (the arc-warped striped text), `text_converted` (the sphere with both
stickers; "Sticker, 1 layer"), `layout_crop_box` (the box with eight handles over the whole artboard and the ellipse;
Crop options with Ellipse), `layout_rotated` (the upright ellipse, the sphere turned the same way; the thin vertical
lines left and right are the active layer's outline, which is taller than the canvas), `layout_done` (one "Flattened"
row, the camera fitted to the 360 x 200 artboard), `library_saved` (tray "LIB", the Library section with the folder and
one thumbnail), `library_placed` (the "LIB" sticker on the sphere), `keys_inside` (one row after Ctrl+M). Each matches
its expect text.

## Numbers after the fixes

- `cargo test --workspace --no-fail-fast`: core 1,101 passed / 1 failed (WA20 only) / 41 ignored (+4 checker tests),
  integration 15 + 9 + 8 + 25 (`target\sticker\p8check	est2.log`); that run found one app test that merged a hidden
  layer (`sticker::tests::layer_commands_are_one_entry_each_and_keep_their_rules`: it hid layer 1 earlier and Ctrl+M'd
  it); the test now shows the layer first (one more "Show Sticker Layer" step, the merge itself unchanged) and the app
  tests are 623 passed / 0 failed / 24 ignored (+1 `library_thumbnails_are_small`; `test_app3.log`). Whole workspace:
  1,781 passed / 1 failed (WA20) / 65 ignored.
- `cargo clippy --workspace --all-targets`: 0 warnings (`clippy2.log`).
- `cargo build --release -p wte_app`: OK, exe 31,864,320 bytes, SHA-256
  `bc535a5d2dd70ec62b8bafafd284e46e035f0489cee1f00e667fcd9d7e8eec33`.
- Scenarios with the fixed exe (`target\sticker\p8checkun2\`, off-screen, no focus taken, `WTE_FRAME_LOG=1`): `smoke`,
  `sticker_text`, `sticker_layout`, `sticker_library`, `sticker_keys`, `sticker_view`, `sticker_edit`, `sticker_cutout`,
  `sticker_styles`, `sticker_tray`, `sticker_controls`, `sticker_pen`: all `"ok": true`. Frame spans
  (`tools/sticker/frame_spans.py`, limit 50 ms): worst `sticker.canvas` 3.90 ms (`sticker_library`). `library_saved`
  looked at again: the 128-px thumbnail fills its 52-pt slot as before.
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with the fixed exe and `tools/sticker/jobs_identity.txt` into
  `target\sticker\p8check\sweep\`): 21 / 21 `"ok": true`. `compare_baseline.py` against `target\stickeraseline\sweep`
  (`compare.txt`): the known `side_tabs/shot_small_640.png` / `shot_small_900.png` (byte-identical to the builder's P8
  sweep) and the NOTE `shot_tab_sticker.png` (byte-identical to the builder's P8 sweep), plus `export_checks/shot.png`
  with a toast caught half faded (box at the bottom right of the view); that job run again alone
  (`sweep_ec\`): 0 pixels differ from the baseline in the view rectangle. Exports, saved layer PNGs and manifests: all
  the same.
- Programs started: cargo, Python and the WTE release exe (only with `--screenshot ... --scenario ...`). Never
  `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No new crate; `Cargo.toml` / `Cargo.lock` unchanged; every
  edited file kept its LF endings.

## Left open (minor)

- Merge Down, Flatten and Trim render on the UI thread (builder: about 100 ms at 2048 px; a 4096-px sticker freezes a
  click for several hundred ms). 8.4 sets no budget; a job like the popover preview's would keep the frame smooth.
- The transform and crop handles are 8-pt squares with a 9-pt grab radius, below the 24-pt tablet target (open since
  P6a).
- A Text tool click selects a text layer only on its ink; a click between letters adds a new text layer.
- Text layers have no per-gap editing in the Text tab (deviation 8).
- Merge Down bakes the upper layer's blend against the lower layer only (documented approximation).
- No in-app Help chapter for stickers yet (P13).
