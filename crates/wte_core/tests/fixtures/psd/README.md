# PSD fixtures (STICKER_PLAN 8.3)

Made in package P0 (2026-11-25) with the user's **PhotoCraft v0.2.0** console exe, run headless (no window, never
`photocraft.exe`):

- exe: `G:\Self-made_products\photocraft-0.2.0-windows-x86-portable\photocraft-cli.exe`, 22,375,288 bytes, dated
  2026-10-05, SHA-256 `04d554cd69ec9fa0c31baf553cb9655f8af8ad02e13460667c9a1b516e9fb5e8` (its `--help` text equals
  tag v0.2.0's `apps/photocraft-cli/src/lib.rs`).
- scratch folder: `G:\Claude_Resources\ref\scratch\stickerfix\` (outputs in `out\`, then copied here unchanged).
- everything is reproducible byte for byte: `make_fixtures.sh` was run twice and all 52 output files had the same
  SHA-256 both times (the P0 checker ran it twice again from another folder, with f12 added: 57 / 57 equal, and equal
  to the files here).

All canvases are 64 x 48, RGB. For every fixture `X.psd` there are:

- `X_merged.png`: PhotoCraft's own flattened picture of the file (`photocraft-cli convert X.psd X_merged.png`), the
  expected merged / composite result.
- `X_layers/*.png`: PhotoCraft's per-layer pictures (`photocraft-cli run X.psd --cmd file.export.layersToFiles
  --params {"dir":"<abs>/X_layers","format":"png","visibleOnly":false}`). Every file is canvas-sized (64 x 48, the
  layer at its place); the number in the name counts from the **top** layer (`0000` = topmost). These pictures have
  the layer's mask **and its effects applied** (f4's file shows the red stroke), and a group comes out as one picture
  (f6). f7's PNGs are 16-bit.

## How to make them again

1. f1's input `f1_wte_in.psd` is written by WTE's texture PSD writer (`wte_core::export::write_psd`, unchanged since
   v0.3.0) through the small program in `gen_f1/` (its own workspace, never built by `cargo test --workspace`). From
   the repository root, with the repository's `Cargo.lock` copied next to `gen_f1/Cargo.toml` first (same crate
   versions; delete the copy afterwards):

   ```
   G:\Claude_Resources\rust\cargo-gnu.cmd run --offline --release --manifest-path crates\wte_core\tests\fixtures\psd\gen_f1\Cargo.toml --target-dir G:\Claude_Resources\ref\scratch\stickerfix\gen_target -- G:\Claude_Resources\ref\scratch\stickerfix\f1_wte_in.psd
   ```

   `f1_wte_in.psd` SHA-256 `86edd6847bd556afa11febe6554641f8b7f696533aa9fdc80febdc4de128235e`.
2. Copy `make_fixtures.sh` and `f1_wte_in.psd` into an empty scratch folder and run `sh make_fixtures.sh` there from Git
   Bash; it writes `./out` (never run it inside the repository). The script holds every command line below, with `PC` =
   the exe above and `OUT` = the absolute path of `./out`.

## The fixtures

`tools/sticker/psd_dump.py FILE` prints the structure quoted below (rect = left, top, right, bottom).

| Id | File | Made with | What is in the file | Proves (plan 8.3) |
|---|---|---|---|---|
| f1 | `f1_wte_in.psd` -> `f1_pc.psd` | `convert f1_wte_in.psd f1_pc.psd` | in: WTE writer, 3 layers `photo` (soft-edged gradient disc, alpha 255 to radius 12, 0 at radius 20 around (22,24); **RGB kept under alpha 0** inside its content box), `text` ("WTE", WTE's default font, 18 px), `shape` (anti-aliased red disc r 9 at (50,36)); no `lyid`, no resource 1057, flags 0x08, empty blending ranges, merged RLE 4 channels. out: same names, order and rects, `lyid` **1, 2, 3** (bottom to top), `iOpa` 255, default blending ranges (32 bytes), resources 1005 1037 1049 1057 (hasRealMergedData 1), merged RLE 4 channels | PhotoCraft keeps names, order and rects, assigns `lyid` 1..3; RGB under alpha 0 (compare `f1_wte_in` and `f1_pc` layer channels) |
| f2 | `f2_feathered.psd` | `run --new {64x48 transparent} --cmd select.rect {x 12, y 4, 40x40, ellipse, feather 6} --cmd edit.fill {#3080e0} --cmd select.deselect --out` | one layer, feathered blue disc, rect [4,0,60,48]; merged 4 channels, layer count -1 | un-matting the merged picture from white |
| f3 | `f3_opaque.psd` | `run --new {64x48 white} ...` red rect + green ellipse | one `Background` layer (flags 0x01, `lspf`), layer count +1, **merged 3 channels** | 3-channel merged = opaque |
| f4 | `f4_mask_stroke.psd`, `f4b_mask.psd` | green rect (8,6 48x36); `select.rect` ellipse (14,10 36x28, feather 2); `layer.layerMask.revealSelection`; deselect; f4 only: `layer.layerStyle.stroke {size 3, outside, #ff0000}` | user mask (channel -2), mask rect [11,7,53,41], default 0; f4 also `lfx2` | f4: EFFECTS -> Merged, the merged picture includes the stroke; f4b: the mask comes back as a WTE mask |
| f5 | `f5_f1_plus_text.psd` | `run f1_pc.psd --cmd type.create {x 3, y 44, "PC", size 14, #102080, name "pc text"} --out` | f1's three layers with `lyid` 1..3 unchanged plus `pc text` (`lyid` 4, `TySh`, **with pixel channels**) on top | TEXT with pixels -> one new picture layer, the three WTE layers kept |
| f6 | `f6_group.psd` | two pixel layers (`Layer 1` red, `upper` blue), `layer.select {layer 2, add}`, `layer.groupLayers {name "group"}` | 4 records: `</Layer group>` divider (`lsct` 3, lyid 4), `Layer 1` (2), `upper` (3), `group` (`lsct` 1, blend `pass`, lyid 1) | GROUP -> Merged |
| f7 | `f7_16bit.psd` | `run --new {64x48 transparent, depth 16}` feathered orange disc | depth 16; layer info **inside the global `Lr16` block** (the plain layer section is empty); merged RLE 16-bit 4 channels | `Lr16`, 16 -> 8, un-matting at 16 bits |
| f8 | `f8_hidden_opacity_multiply.psd`, `f8b_color_dodge.psd` | f8: yellow base, `half` blue (`setProps {opacity 0.5}`), `multiply` red ellipse (`{blend "Multiply"}`), `hidden` green (`{visible false}`); f8b: brown base + `dodge` ellipse (`{blend "Color Dodge"}`) | f8: opacity 128, blend `mul `, flags 0x02 (hidden); f8b: blend `div ` | hidden / opacity / blend mapping; BLEND_OTHER -> Merged |
| f9 | `f9_alpha_channel.psd` | feathered purple disc; `select.rect` (0,0 32x24); `select.saveSelection {name "Alpha 1"}` | header channels 5; layer count -1; merged = R G B, transparency, `Alpha 1`; resources also 1006 1045 1077 | channel order (critic R1): the 4th merged channel is transparency, the 5th a saved alpha |
| f10 | `f10_f1.psb` | `convert f1_pc.psd f10_f1.psb` | version 2 (PSB), same layers and `lyid`s as f1_pc | PSB lengths |
| f11 | `f11_clipped.psd` | blue disc, new layer `clipped` (yellow band), `layer.createClippingMask` | `clipped` has clipping byte 1 | `clipped` -> `clip` |
| f12 | `f12_lyid_reuse.psd` (added by the P0 checker) | `run f1_pc.psd --cmd layer.select {layer 3} --cmd layer.delete --cmd layer.new.layer {name "pc new"}` + a cyan rect | `photo` `lyid` 1, `shape` `lyid` 3, `pc new` (new, on top) **`lyid` 2** = the deleted `text` layer's id: PhotoCraft gives a new layer the lowest free id, not one above the largest | STICKER_PLAN 5.4 "`lyid` reuse": a deleted WTE layer plus a new PhotoCraft layer that took its id |

Every fixture's layer records carry `luni`, `lyid` and `iOpa` = 255 and the default blending ranges
(`(channels + 1) x 00 00 FF FF 00 00 FF FF`), all channels PackBits (RLE); none uses ZIP.

## Differences from the plan's wording

- f4: the plan names `select.subject`; an elliptical `select.rect` with feather 2 was used instead so the mask is
  exact and does not depend on PhotoCraft's subject heuristics (the command is in v0.2.0's registry, the result would
  only be less predictable).
- f1 is made from today's texture PSD writer (no `lyid`); P5 adds a second f1 written by the new sticker writer and
  keeps this one (plan 8.3).
- Not made with the CLI (no PhotoCraft exe here can make them; built by the tests' byte builder in P5, plan 8.3):
  resource 1057 with `hasRealMergedData = 0`, ZIP / ZIP-prediction channels and a ZIP merged image, the 13 PSB long
  keys beyond what f10 uses, CMYK and 32-bit headers, truncated and corrupted files.
- `f1_pc.psd` is a CLI `convert`; whether a GUI Ctrl+S writes the same is answered in
  `docs/notes/sticker/pc-save-paths.md` (same exporter, same options).
