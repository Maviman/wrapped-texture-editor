# Photoshop-saved PSD fixtures (STICKER_PLAN 13.4)

- Source: https://github.com/psd-tools/psd-tools , directory `tests/psd_files/`, commit
  `b5637927626c7e577ffb5b8925901fc9d06c69f9` (same relative paths here).
- Licence: MIT, **inferred** from the repository's `LICENSE` file ("Copyright (c) 2019 Kota Yamaguchi"; the text is
  `LICENSE-psd-tools.txt` here, verbatim). The repository has one licence file and no separate notice for its test
  files; the origin of the individual files was not checked.
- These files are test data only. They are read by `wte_core`'s PSD reader tests (`crates/wte_core/src/psd/fixture_tests.rs`)
  and are never shipped (the installer contains no fixture).

55 files, 1,970,184 bytes, copied unchanged from `G:\Claude_Resources\ref\psd-samples\psd-tools\` (the list is that
folder's `repo_set.txt`) on 2026-11-26. Two files of the collection that look contributed by users
(`2layers.psd`, `third-party-psds\cactus_top.psd`) are **not** here; what they show is built by the tests' byte builder
(`psd/testkit.rs`). 54 of the 55 were last saved by Photoshop (the reader name of image resource 1057: CS3 4, CC 1,
CC 2017 2, CC 2018 17, CC 2019 22, 2020 5, 2022 1, 2026 2), one by GIMP. None was saved by Photoshop CC 2019 on
Windows and none is a WTE-written file re-saved by Photoshop: those are hand tests (STICKER_PLAN 13.6).

"MaxCompat off" = Photoshop's "Maximize PSD and PSB File Compatibility" was off at save: resource 1057 says the merged
picture is not real.

| File | Last saved by | The `k5_ps_*` test asserts |
|---|---|---|
| `layers/pixel-layer.psd` | CC 2018 | 1 layer, `lyid` 3, rect (1, 1, 30, 30); composite = merged within 1; Layers mode |
| `layers-minimal/pixel-layer.psd` | CC 2019, MaxCompat off | no real merged picture; Layers mode still imports the layer |
| `layers/type-layer.psd`, `layers-minimal/type-layer.psd` | CC 2018, CC 2019 (MaxCompat off) | TEXT with pixels, not forcing, `editor_native`; one picture layer |
| `layers/smartobject-layer.psd`, `layers-minimal/smartobject-layer.psd` | CC 2018, CC 2019 (MaxCompat off) | SMART with pixels; forcing for a Photoshop link (Merged / "needs a merged picture"), Layers mode for a PhotoCraft link |
| `layers/shape-layer.psd` | CC 2018 | FILL with pixels and flags bit 4: a picture layer, no VECTOR_MASK |
| `layers-minimal/shape-layer.psd` | CC 2019, MaxCompat off | NO_PIXELS, no real merged picture |
| `stroke.psd` | CC 2019 | five shape layers with shape strokes, opacity 194 / 148: picture layers; composite = merged within 2; the Background is opaque and not BLEND_IF |
| `layers/solid-color-fill.psd`, `layers/gradient-fill.psd` | CC 2018, CC 2019 | fill layer with pixels and a user mask (40-byte ranges: not BLEND_IF); Layers mode |
| `layers-minimal/solid-color-fill.psd` | CC 2019, MaxCompat off | NO_PIXELS, no real merged picture |
| `layers/pattern-fill.psb` | 2020 | an 8-bit PSB: 4-byte row counts, the `8B64cinf` block with an 8-byte length stepped over exactly; a pattern-fill picture layer with a mask; composite = merged exactly |
| `layers/group.psd`, `masks3.psd` | CC 2018, CC 2019 | `lsct` 1 and 3 -> GROUP -> Merged; the end marker is not NO_PIXELS |
| `gradient-sizes.psd` | CC 2019 | three artboards (`artb`) -> GROUP; `artd` / `cinf` skipped; three shape layers reaching outside the canvas; transparent merged picture |
| `layers/levels.psd`, `layers/curves.psd`, `clip-adjustment.psd` | CC 2019, CC 2019, CC 2018 | ADJUSTMENT -> Merged; a clipped adjustment over a text layer |
| `effects/effects-enabled.psd` | 2022, MaxCompat off | the descriptor walker per layer (only `lfx2` decides); with "Layer 1" hidden by a byte patch: Layers mode |
| `effects/outside-stroke.psd` | 2026 | an enabled stroke -> EFFECTS -> Merged; the merged picture differs from the layer in 224 of 1,024 pixels |
| `transparency/clip-opacity.psd`, `transparency/fill-opacity.psd` | 2020 | `iOpa` 128 / 0 -> FILL_OPACITY -> Merged |
| `mask.psd`, `mask-disabled.psd` | CS3, 2020 | user mask (rect, default colour); flags bit 1 = disabled; Layers mode |
| `mask_parameters.psd` | CC 2018, MaxCompat off | mask flags bit 4 and channel -3 -> MASK_EXTRA |
| `masks/2.psd` | CC 2019, MaxCompat off | vector masks on pixel layers -> VECTOR_MASK; a shape without pixels; opacity 128 |
| `clipping-mask3.psd` | 2020 | clipping byte -> `clipped` |
| `hidden-layer.psd`, `semi-transparent-layers.psd` | CS3, CC | hidden flag; a Background without channel -1; a rect reaching outside the canvas |
| `blend-modes/{multiply,screen,overlay,linear-dodge,darken,lighten}.psd` | CC 2019 | each key maps to WTE's mode; Layers mode; WTE's blend kernel = merged within 3 |
| `blend-modes/color-dodge.psd`, `blend-modes/pass-through.psd` | CC 2019 | `div ` -> BLEND_OTHER; `pass` group -> Merged |
| `layer-name-emoji.psd` | CC 2018, MaxCompat off | `luni` with a surrogate pair; raw channels; `lddg`, opacity 128 |
| `transparentbg-gimp.psd` | GIMP (no 1057) | no `lyid`, no 1057 (= real); a UTF-8 Pascal name without `luni` |
| `16bit5x5.psd`, `16bit5x5.psb` | CS3, CC 2017 | 16-bit, layers in `Lr16` (not flat), ZIP with prediction; PSD and PSB give the same document |
| `colormodes/4x4_16bit_rgb.psd`, `colormodes/4x4_16bit_grayscale.psd` | CC 2018 | an ordinary empty layer; a fill layer without pixels -> Merged from the 16-bit merged picture; grey expanded |
| `colormodes/4x4_8bit_grayscale.psd` | CC 2018 | grey expanded; 2- and 3-channel layers not BLEND_IF; Layers mode; no profile note |
| `colorprofiles/north_america_newspaper.psd` | CC 2019 | a non-sRGB profile ("Adobe RGB (1998)") -> the profile note |
| `32bit5x5.psd`, `colormodes/4x4_8bit_lab.psd`, `gradients/noise-gradient-cmyk.psd`, `colormodes/4x4_8bit_index_color.psd`, `colormodes/4x4_1bit_bitmap.psd`, `colormodes/4x4_8bit_duotone.psd`, `colormodes/4x4_16bit_multichannel.psd` | CS3, CC 2018, 2026, CC 2018 x 4 | refused with their own sentence (32 bits, Lab, CMYK, Indexed, Bitmap, Duotone, Multichannel) |
| `slices.psd` | CC 2017 | 0 layers: one picture from the merged data |
| `vector-mask2.psd` | CC 2019 | a layer rect (0, 0, -1, 0) is empty, not an error; ten layers |

Every file also goes through the truncation sweep and the byte flips (`k5_ps_truncation_sweep`,
`k5_ps_byte_flips_never_panic`, ignored `k5_ps_sweep_large`).
