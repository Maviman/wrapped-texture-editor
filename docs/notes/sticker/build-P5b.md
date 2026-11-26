# Build note: P5b - the sticker PSD writer (STICKER_PLAN 7 "P5", 5.2, 13.3, 13.9 "P5b")

Builder, 2026-11-26, branch `stickers` (from e9298da, plan revision 3). Core logic (kind K), headless. Nothing is
user-visible yet (the link logic is P5c, the app side P9a / P9b), so plan 8.2 lists no scenario for P5b and plan 10
gives it no HANDOFF or `dist/HOW_TO_TEST.txt` edit; the app-level check is the identity sweep of 8.5.

## What was built

| Item (13.9 "P5b") | Where | Notes |
|---|---|---|
| `write_sticker` of 5.2 with the byte rules of 13.3 | new `crates/wte_core/src/psd/write.rs` | `StickerPsd { size, layers, id_seed }`, `StickerPsdLayer { name, lyid, pixels, mask, clipped, visible, opacity, blend }`, `write_sticker(w, &psd)`, `write_sticker_to_vec`, `normalise` (zero RGB under alpha 0, trim to alpha > 0; public for P5c's comparison rule), `composite` (the merged picture the file stores); header, resources 1005 / 1044 / 1057, layer info and section lengths multiples of 4, negative count with a transparent merged picture, records with channels -1 0 1 2 (-2), mask data 0 / 20 bytes, Photoshop's 40-byte default blending ranges, Windows-1252 Pascal names padded to 4, `luni` + `lyid`, PackBits channels (compression 0 and no bytes for an empty rectangle), merged picture RLE with all counts first, 3 channels when opaque else 4 matted on white; deterministic bytes |
| Error variants | `psd/mod.rs` | `PsdError::Invalid(&'static str)` (an input a PSD cannot hold) and `PsdError::Io { kind, message }` + `PsdError::io` |
| Strict structure walker | new `psd/strict.rs` (test-only) | 13.3: fails on any overlap or slack, one allowance (0-3 zero bytes after the layer info's channel data and at the section end); returns the offsets the fault tests patch |
| Pillow check | new `tools/sticker/psd_check.py` | second, independent reader: canvas, resources, Pascal names and rects of the layers Pillow lists, merged picture vs WTE's composite exactly (matted with the writer's formula) |
| Tests | new `psd/write_tests.rs` (16 tests + 3 ignored) | below |
| f1 rebuilt with this writer (8.3) | `tests/fixtures/psd/f1w_wte_in.psd`, `f1w_pc.psd`, `f1w_pc_merged.png`, `f1w_pc_layers/`, `make_f1w.sh`, README | the same three pictures as f1, `lyid` 1..3, re-saved by the real PhotoCraft v0.2.0 CLI (headless) |
| ARCHITECTURE | 1.12 (P5b traceability), 2.1 (rows for `psd/write.rs`, `strict.rs`, `write_tests.rs`), 5.28 (writer API, error variants, `psd::strict`), 7.7 (the two writers stay separate, the golden test unchanged), 7.9 ("Writer" table and "Strict walker" paragraph), 9.1 (the `k5_write_*` / `k5_strict_*` tests), 10 (P5b row) | |
| Test compositor | `psd/testkit.rs` (P5a's oracle) | a hidden base now hides its clipped layers (deviation 9) |

Tests (`psd::write_tests`; every file the writer produces in them also goes through the strict walker and through
`psd::read` with no re-sync): `k5_write_roundtrip_every_layer_feature`,
`k5_write_empty_layer_reads_as_an_empty_picture_layer_in_layers_mode`,
`k5_write_opaque_merged_has_three_channels_and_a_positive_count`, `k5_write_scales_1_2_4`,
`k5_write_normalises_layer_pixels_and_masks`, `k5_write_bytes_are_deterministic`, `k5_write_byte_rules_of_13_3`,
`k5_write_pascal_names_are_windows_1252`, `k5_write_clipping_and_hidden_bases`,
`k5_write_refuses_what_a_psd_cannot_hold_and_writes_nothing`, `k5_write_without_layers_is_a_flat_transparent_file`,
`k5_strict_accepts_every_photoshop_fixture`, `k5_strict_on_the_photocraft_fixtures`,
`k5_strict_rejects_a_seeded_fault_of_each_kind`, `k5_write_f1w_input_is_the_committed_fixture`,
`k5_write_f1w_photocraft_resave_keeps_ids_and_normalised_pixels`; ignored: `k5_write_make_f1w_input`,
`k5_write_dump_for_psd_check`, `perf_psd_write_and_read_2048_five_layers`.

No new crate, no `Cargo.toml` / `Cargo.lock` change. `export/psd.rs` and its golden test are untouched (the new writer
uses only `export::packbits::encode_row`). No PhotoCraft, Photoshop or ArtCraft name in UI text (core only). Every new
file is LF; every edited file was LF and stays LF.

## Deviations from the plan, with reasons

1. **The input types are designed here.** The plan names `StickerPsd` without fields. `StickerPsdLayer::pixels` is a
   `CroppedBuf` in canvas pixels; the mask reuses the reader's `PsdMask` (so a read mask can be written back; its
   `rendered` flag is not written, a WTE mask is always a pixel mask); `StickerPsd::id_seed` is resource 1044.
2. **`PsdError` gains `Invalid` and `Io { kind, message }`.** 5.2 gives `write_sticker` the reader's error type, which
   had no variant for a bad input or a failing output. `Io` keeps the `ErrorKind` (P5c's atomic write may want it);
   the type stays `Clone + PartialEq`. Nothing is written when an input check fails.
3. **The bottom layer is written unclipped.** A clipping byte on the bottom layer has nothing to clip to; WTE's render
   draws such a layer unclipped, and what Photoshop does with it is unknown. Writing 0 is the safe side; P5c should
   expect `clipped == false` back for the bottom layer.
4. **Masks are normalised too**: trimmed to the values that differ from the default (13.3 says only "mask rows use
   the mask rect"). Same bytes for the same mask whatever rectangle the caller used; a mask of only its default gets
   the empty rectangle (read back as a mask with `IRect::EMPTY`). A mask default other than 0 or 255 is refused
   (the format allows only those two).
5. **Layer parts outside the canvas are written**, not clipped (Photoshop writes such rects itself,
   `semi-transparent-layers.psd`). 5.4 has `link::export` clip to the canvas; the writer keeps what it is given. The
   merged picture covers the canvas only.
6. **Resource 1005** says "shown in inches" (unit 1), like WTE's texture exporter; Photoshop's fixtures say cm (2).
   Both are valid; 13.3 only fixes 72 dpi and pixels per inch.
7. **Resource 1044 = max(`id_seed`, largest `lyid`)**: the plan says `next_lyid - 1`; the max guards a caller that
   passes a seed below an id it writes (an editor would then number a new layer onto a used id).
8. **No layers** = layer info length 0 (a flat file; the reader reads the 4th merged channel as transparency).
9. **The compositing rule of a hidden clipping base.** The writer's merged picture hides layers clipped to a hidden
   base, as the sticker render (ARCHITECTURE 6.10) and Photoshop do. P5a's test compositor (`testkit::composite`)
   still drew them; it now follows the same rule. All P5a tests pass unchanged.
10. **The walker's rules were measured, not guessed.** Global tagged blocks in Photoshop's files have lengths that
    are not rounded and are followed by the zero bytes that pad them to 4 (`Lr16` 1514 + 2, `cinf` 499 + 1, ...);
    layer blocks always have lengths that are multiples of 4; the long mask data (56 bytes in `mask_parameters.psd`)
    is rect, default, flags, **real flags, real background, real rect, then** the parameters (flags bit 4), then
    padding. The walker follows exactly these. Calibration is on the 54 Photoshop-saved fixtures (the GIMP one is
    left out, as 13.9 says).
11. **PhotoCraft's own files fail the strict walker** (8 of 16: it rounds neither the layer-and-mask section nor an
    `lfx2` block). Not a fault of anything here: PhotoCraft's reader and WTE's accept them, and the walker states
    Photoshop's layout. `k5_strict_on_the_photocraft_fixtures` records which pass and the reason for each failure.
12. **`psd_check.py` is limited by Pillow 11**: Pillow skips layers with more than 4 channels (a masked layer), takes
    a layer's channels in sorted band order instead of file order (so its layer pictures are meaningless for any
    Photoshop-style file), and reads neither 16 / 32-bit files nor PSB. The script therefore checks names and rects
    of the layers without a mask (their count must match), resources 1005 / 1044 / 1057, the canvas, and the merged
    picture exactly; fixtures without an expectation file are only opened, 16 / 32-bit ones are reported as SKIP.
13. **Control characters in a name** become `?` in the Pascal name (the full name is in `luni`).
14. **Scale 1 / 2 / 4** is tested as the same sticker written with its canvas and layers scaled by 1, 2, 4 (the writer
    does not know the link scale; P5c picks it).

## Numbers measured

- f1w: the writer's f1 input is 12,222 bytes (SHA-256 `708962ff...a202`, asserted by a test); PhotoCraft v0.2.0
  `convert` gives `f1w_pc.psd`, 12,170 bytes, the same SHA-256 on two runs (`98d12188...652f`). PhotoCraft kept
  `lyid` 1, 2, 3, the names and the order, **and every layer's rect and bytes exactly** (its re-trim of a normalised
  layer finds nothing to trim): the comparison rule of 5.4 sees an untouched layer as unchanged without any
  tolerance. Its merged picture and `convert` PNG are within 1 level at alpha 255 of the writer's composite (its
  compositor differs by rounding), within the un-matting bound below; its layer PNGs equal the written layers exactly.
- Strict walker: 54 / 54 Photoshop-saved fixtures pass; 27 of them use the pad allowance after the layer info, none
  at the section end. PhotoCraft fixtures: 8 pass (`f1_pc`, `f2`, `f3`, `f5`, `f7`, `f9`, `f10`, `f1w_pc`), 8 fail
  as recorded. Every seeded fault fails: a channel length + 1, an extra length - 4, resource 1057 without its pad
  byte ("not padded to even"), a Pascal name padded to 2, a non-zero pad byte, 4 pad bytes ("at most 3 pad bytes"), a
  byte after the merged picture, a truncated merged picture.
- `psd_check.py` on the writer's outputs (`k5_write_dump_for_psd_check` -> `target\sticker\p5b\psd_check\`: `rich`,
  `f1w`, `opaque`, `big` 300 x 200): **4 / 4 PASS** (`target\sticker\p5b\psd_check_outputs.txt`); a tampered copy
  (a renamed layer, a changed merged pixel) FAILs with both reasons. On the fixture folder: 63 passed, 6 skipped
  (16 / 32-bit), 0 failed (`psd_check_fixtures.txt`).
- Performance (8.4, `perf_psd_write_and_read_2048_five_layers`, `--release`, three runs): write 94.1 to 95.8 ms
  (budget 150; 144 ms before the compositor was made row-parallel), read 82.8 to 83.3 ms (budget 150); 16,926,377
  bytes.
- Determinism: two writes and a write on a one-thread rayon pool are byte-identical (test).
- `cargo test --workspace --no-fail-fast`: **1,609 passed, 1 failed (WA20), 61 ignored** (`wte_core` lib 1,028 / 1 /
  37, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25). The only red test is
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`. (+18 passed and +4 ignored against
  the P5a build note: these 16 tests and 3 ignored ones, plus the P5a checker's 2 tests and 1 ignored one.) One run
  in between failed to link (rust-lld "undefined symbol" in the integration tests) after I had stopped a test build
  half way; `cargo clean -p wte_core -p wte_app` and a fresh run fixed it (no code change).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 29,157,888 bytes, SHA-256
  `7efdc8c5e114c5b90d019ae75438667232930ec54666166965eaf445b154fc5e`.
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with that exe and `tools/sticker/jobs_identity.txt` into
  `target\sticker\p5b\sweep\`, off-screen, no focus taken): **21 / 21 `"ok": true`**; `compare_baseline.py` against
  the P0 baseline: **216 / 216 files the same, PASS** (`target\sticker\p5b\compare.txt`). Looked at `roundtrip`
  (Split view, the checker sphere with the logo, the WTE text and two strokes, "Saved shot.wtep"): as before.
- No `Cargo.toml` / `Cargo.lock` line changed; `export/psd.rs` not touched. Programs started: the WTE exe (only with
  `--screenshot ... --scenario ...`) and `photocraft-cli.exe` (headless, twice, for f1w). Neither `photocraft.exe`
  nor `Photoshop.exe`.

## For the checker

- `python -E -P tools/sticker/psd_check.py target/sticker/p5b/psd_check` after running the ignored
  `k5_write_dump_for_psd_check` (`WTE_PSD_CHECK_OUT` picks another folder), and on `crates/wte_core/tests/fixtures/psd`.
- `make_f1w.sh` reproduces `f1w_pc.psd` from `f1w_wte_in.psd` (README step 3).
- P5c: `normalise` is the normaliser of the comparison rule; the bottom layer comes back unclipped (deviation 3); masks
  come back trimmed (deviation 4), so compare masks as functions over the layer's rect, not by rectangle.
