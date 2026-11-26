# Check note: P5b - the sticker PSD writer (STICKER_PLAN 7 "P5", 5.2, 13.3, 13.9 "P5b")

Checker, 2026-11-26, branch `stickers` @ 1c43534 (builder commit). Independent, adversarial check of the writer, the
strict walker, `psd_check.py` and fixture f1w. Outputs under `target\sticker\p5b_check\`.

## Verdict

**Pass.** No blocker or major found. Two minor points for P5c / the main session (below), not fixed here because they
are outside the writer's contract. Two checker tests and one PhotoCraft-made fixture added.

## Plan items (13.9 "P5b", P5 list, 13.3), one by one

| Item | Result |
|---|---|
| `write_sticker` of 5.2: header, resources 1005 / 1044 / 1057, no ICC | read the code byte by byte against 13.3 and against what Photoshop writes in the fixtures (my own parser): 1057 strings without terminator exactly like Photoshop's ("Adobe Photoshop" + reader name, 93 bytes); 1005 equal except the display unit (Photoshop cm, WTE inches; both valid) |
| lengths rounded to 4, 0-3 pad bytes | layer info padded, section = 4 + info + 4; walker confirms on every output |
| 40-byte default ranges, flags `0x08` / `0x0A`, filler 0, clipping 0 / 1 | as Photoshop; bottom layer forced unclipped, same rule as the render (6.10: base falls back to layer 0) |
| mask data 20 bytes, an all-default mask = rect 0, 0, 0, 0 + channel -2 of 2 bytes | **exactly what Photoshop writes** for a reveal-all mask (`layers\solid-color-fill.psd`, `gradient-fill.psd`, `4x4_8bit_grayscale.psd`: rect 0 0 0 0, default 255, channel -2 length 2) |
| Pascal name Windows-1252, `?`, <= 255, padded to 4; `luni` padded to 4 | `luni` lengths match Photoshop's rule (`Background` 24, `Background copy` 36) |
| negative count with a transparent merged picture; merged RLE, all counts first, matted on white | yes; un-matting bound holds in every round trip |
| normalised pixels (A11), deterministic bytes | `normalise` = PhotoCraft's re-trim (f1w and f14w: PhotoCraft's bytes equal the written ones) |
| round trip incl. empty layer, names non-ASCII and > 31 bytes, hidden, opacity, 7 blends, mask, clip, outside part, opaque vs transparent, scale 1 / 2 / 4 | present (`k5_write_*`); I added a 300-case random round trip (below) |
| strict walker calibrated on 54 Photoshop fixtures, seeded faults | 54 / 54 pass, every fault kind rejected (wrong length, odd padding, non-zero pad, 4 pad bytes, trailing byte, truncation) |
| fixture f1 rebuilt (8.3) | `f1w_wte_in.psd` -> `f1w_pc.psd` by the real CLI; ids, names, order and bytes kept; the old f1 kept |
| ARCHITECTURE 5.28, 7.7, 7.9 (+1.12, 2.1, 9.1, 10) | updated and consistent with the code |
| Checker: `psd_check.py` on round-trip outputs and fixtures | outputs: **4 / 4 PASS** (`psd_check_outputs.txt`); fixture folder: **63 passed, 6 skipped (16 / 32-bit), 0 failed** (`psd_check_fixtures.txt`) |
| Checker: diff two writes of one sticker | `k5_write_dump_for_psd_check` run twice in two processes into `dump_a` / `dump_b`: all 12 files byte-identical |
| Checker: `export/psd.rs` and its golden test untouched; no `Cargo.toml` / `Cargo.lock` change | `git diff b2fb8f5..HEAD` on `crates/wte_core/src/export/`, `Cargo.toml`, `Cargo.lock`: nothing (the only Cargo.toml in the range is the P0 fixture generator `gen_f1`) |

Also checked the builder's claim that Pillow cannot judge layer pixels: true, and not WTE-specific: Pillow 11 builds
1 tile for the second layer and none for the third of `f1_pc.psd` (PhotoCraft), `f1w_pc.psd`, and of Photoshop's own
`stroke.psd` (`R G`, then nothing). So the layer pixels need another judge; the f14w test below is that judge.

## Added by the checker

- **Fixture f14w** (`tests/fixtures/psd/f14w_rich_pc.psd`, `make_f14w.sh`, README row): the writer's `rich_sticker()`
  (12 layers: Cyrillic / umlaut / emoji / 63-character names, `lyid`s 1-3, 10-16, 30, 31, hidden, opacities, the seven
  blend keys, a clipped layer, a soft mask with default 0, a layer partly outside the canvas, an empty layer) re-saved
  headless by the real PhotoCraft v0.2.0 CLI (`convert`; `photocraft-cli.exe` only). Test
  `k5_check_rich_sticker_survives_a_photocraft_resave`: a second editor reads **every** feature the writer writes as
  WTE means it: names, ids, order, hidden, opacity, blend, clipping, no feature flag, Layers mode, and every layer's
  pixels and mask on the canvas equal to the written (normalised) ones.
- `k5_check_random_stickers_round_trip`: 300 random stickers (fixed seed): canvases 1..40, layers wholly outside the
  canvas, 1 x 1 and empty, alpha with holes and RGB under alpha 0, masks of default 0 / 255, all-default, wholly
  outside, disabled, clipping anywhere (bottom layer, chains), hidden bases, every blend and opacity. Each file passes
  the strict walker and reads with no re-sync; layers and masks come back as written; the reader's composite equals
  the writer's exactly; merged within the un-matting bound. All pass.
- PhotoCraft's own compositor on the WTE file (`convert rich.psd rich_direct_merged.png`, and after its re-save):
  equal to WTE's composite within **1 level everywhere except the clipped band** (see minor 2).

## Findings

1. **Minor (P5c, not fixed).** A sticker with **no layers** is written as a flat file (layer info length 0) with a
   4-channel merged picture. WTE's reader takes the 4th channel as transparency, but Photoshop treats an extra channel
   of a flat file as a saved alpha channel ("Alpha 1") over an opaque Background: the empty sticker would open as
   opaque white, and a Ctrl+S there would bring an opaque white picture back. Only an empty sticker is affected. P5c
   should never hand the writer zero layers (write one empty layer, e.g. "Layer 1", with a fresh `lyid`), or refuse
   to link an empty sticker.
2. **Minor, design observation (plan 4.1 / ARCHITECTURE 6.10, not P5b).** WTE's clipping rule (the clipped layer's
   alpha times the base's coverage, then composited over everything below, base opacity and blend not applied to the
   group) is not Photoshop's / PhotoCraft's (a clipping group is composited *atop* the base and takes the base's
   opacity and blend mode). They agree when the base is opaque where the clipped layer draws and the base has opacity
   100 % and Normal. Measured on `rich_sticker()` (base alpha 220 with a soft mask): PhotoCraft's composite differs
   by up to 61 levels in the clipped band, <= 1 elsewhere. The writer is correct to its contract (merged = WTE's
   composite) and the link survives (the layers come back unchanged), but what the user sees in Photoshop differs from
   WTE for such stickers. For the main session to decide (keep, document in Help, or change the render rule).
3. Nit (not a fault): the strict walker takes the global layer mask info without checking its contents; the writer
   writes length 0 there, and Photoshop's fixtures pass.

## Runs (this checker, this machine)

- `cargo test --workspace --no-fail-fast` on 1c43534: 1,609 passed, 1 failed (WA20 only), 61 ignored (same as the
  builder). With the checker's two tests: **1,611 passed, 1 failed (WA20 only), 61 ignored** (`wte_core` lib
  1,030 / 1 / 37, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK; the exe is byte-identical to the builder's (SHA-256 `7efdc8c5...5fc5e`,
  29,157,888 bytes: test-only changes). Smoke scenario off-screen: `"ok": true`, the screenshot is the usual start view.
- `perf_psd_write_and_read_2048_five_layers --release`: write 94.6 ms, read 79.4 ms (budgets 150 / 150).
- P5b adds no scenario (core only); the exe does not call the writer yet, so the builder's identity sweep (21 / 21,
  216 / 216 files equal to P0) stands for this exe.
- Programs started: cargo, python (`-E -P`), `photocraft-cli.exe` (headless, `convert` / `run`, stdin closed). Not
  `photocraft.exe`, not `Photoshop.exe`; no window, no focus taken.
