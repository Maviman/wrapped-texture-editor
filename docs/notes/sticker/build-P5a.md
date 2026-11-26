# Build note: P5a - the PSD / PSB reader (STICKER_PLAN 7 "P5", 13.2, 13.4, 13.9 "P5a")

Builder, 2026-11-26, branch `stickers` (from 9cd4e0f, plan revision 3). Core logic (kind K), headless. Nothing is
user-visible yet (the link logic is P5c, the app side P9a / P9b), so plan 8.2 lists no scenario for P5a and plan 10 gives
it no HANDOFF or `dist/HOW_TO_TEST.txt` edit; the app-level check is the identity sweep of 8.5.

## What was built

| Item (13.9 "P5a") | Where | Notes |
|---|---|---|
| Reader types | new `crates/wte_core/src/psd/mod.rs` | `PsdDoc`, `PsdLayer`, `PsdMask`, `PsdBlend`, `PsdFeatures` (bitflags, `FORCING`, `NATIVE`, `words()`), `PsdSection`, `PsdColorMode`, `PsdLimits`, `PsdError`, `PsdRefusal` with `sentence(name)`; `PsdDoc::{forcing(smart_forces), editor_native, profile_note, is_flat}` |
| The reader of 5.2 with every rule of 13.2 | new `psd/read.rs` | header (PSD / PSB; RGB, Grayscale expanded; Bitmap, Indexed, CMYK, Multichannel, Duotone, Lab, other modes, 32-bit and other depths refused, each with its own sentence), resources 1057 and 1039 (ICC `desc` v2 / `mluc` v4, never applied; grey files exempt from the note), layer info in the section or in `Lr16` / `Lr32` / `Layr`, "flat" counted after that block, the 14 PSB long keys (with `cinf`), records (empty and inverted rects, channels in any order, no -1 = opaque, -3 stepped over), mask data (bits 1, 3, 4), all-default blending ranges of any count, UTF-8-then-1252 Pascal names, `luni` with surrogates, `lsct` types (0 = ordinary layer), `artb`, flags bit 4 and the shape / fill rule, the NO_PIXELS rule, `iOpa`, blend keys, tagged blocks with padding search and re-sync, channels raw / PackBits / ZIP / ZIP with prediction at 8 and 16 bit, the merged picture (one ZIP stream for all channels, all PackBits counts first) un-matted from white at the file's depth |
| The descriptor walker | new `psd/descriptor.rs` | parses the whole descriptor into a light tree; steps over `Objc GlbO VlLs doub UntF UnFl TEXT enum long comp bool type GlbC alis tdta Pth ObAr obj ` and the seven reference items `prop Clss Enmr rele Idnt indx name`; `effects_enabled`: `masterFXSwitch` and `enab`, unreadable = on; depth and item counts bounded |
| The test byte builder | new `psd/testkit.rs` (test-only) | `File` / `Layer` / `Chan` builder (any channel order, compression, padding, PSB, `Lr16`, global blocks, resources), `Desc` descriptor builder, the test compositor (WTE's blend kernel, masks, clipping, opacity), `max_diff`, `mode` (the P5c rule: Layers / Merged / NeedsMerged), `Rng` |
| Tests | new `psd/read_tests.rs` (21 tests), `psd/fixture_tests.rs` (39 tests + 1 ignored) | below |
| Fixtures | `crates/wte_core/tests/fixtures/psd/photoshop/` | the 55 files of `repo_set.txt` copied unchanged (1,970,184 bytes, asserted), `LICENSE-psd-tools.txt` (the MIT text verbatim), `README.md` (source, commit, "licence inferred", test data only and never shipped, the table of 13.4) |
| ARCHITECTURE | 1.12 (title "Stickers and outside editors (PhotoCraft, Photoshop)" and the P5a traceability), 2.1 (the `psd/` modules), new 5.28 (`psd` API), 7.7 (one line: the sticker link has its own reader / writer), new 7.9 (the reader subset), 9.1 (`k5_*`), 10 (P5a row) | |

No new crate, no `Cargo.toml` / `Cargo.lock` change (inflate = `miniz_oxide`, already a direct dependency of `wte_core`).
`export/psd.rs` is untouched (its golden test runs unchanged); the reader uses only `export::packbits::decode_row`.
No PhotoCraft, Photoshop or ArtCraft name in code or UI text beyond the refusal sentences of 13.2 (which name
Photoshop's menus as plain text, as the plan words them). Every new file is LF; `lib.rs` (LF) got one line.

Tests (`k5_*`, 60 new + 1 ignored): one `k5_ps_*` per row of the 13.4 table (32 tests covering the 55 files, plus
`k5_ps_every_fixture_is_present_and_readable_or_refused`); `k5_pc_layer_pixels_equal_photocrafts_pngs`,
`k5_pc_merged_equals_photocrafts_convert_png`, `k5_pc_fixture_features` (the P0 fixtures of 8.3); the truncation sweep,
the ignored `k5_ps_sweep_large`, 5,000 byte flips; `k5_synth_*` (20) for every row of 13.4's "built by the tests" table
that needs no writer and the reader bullets of the P5 list (ZIP / PSB / 1057 / 16-bit / refusal synthetics);
`k5_desc_walker_steps_over_every_item_type`.

## Deviations from the plan, with reasons

1. **`PsdError::Unsupported(PsdRefusal)`, not `Unsupported(&'static str)`.** The sentences of 13.2 start with the
   file's name ("<name> is in CMYK Color mode. ..."), which the core does not know; `PsdRefusal::sentence(name)` builds
   them, and P5c / P9b get a value they can match on. Also `PsdError::{Truncated, Corrupt(&'static str),
   TooLarge { width, height }}` (a canvas side above 30000).
2. **`PsdDoc` has more fields than 5.2 lists:** `psb`, `mode`, `too_large` ("the caller reports too large"),
   `icc_description` (for the note of 13.2) and `PsdLayer::{section, flags, fill_opacity}`; `PsdMask::rendered` (mask
   flags bit 3). `pc_native()` is `editor_native()` as rev 3 says.
3. **GROUP counts on hidden layers too** in `forcing` (5.2 says "on any visible layer"): a group's end marker is always
   hidden, and a hidden group's children carry their own visibility flags, so importing them layer by layer would show
   layers Photoshop does not draw. Merged is the safe side. Every other forcing feature counts on visible layers only.
4. **`pass` and `lsct` blend keys.** The blend key stored in an `lsct` block (Photoshop writes `pass` there, the record
   says `norm`) replaces the record's key, so a pass-through group shows `PsdBlend::Other(pass)` and BLEND_OTHER as 5.2
   asks. A group forces anyway; the sentence will list "groups" and "other blend modes" for it.
5. **VECTOR_MASK is never set on a shape / fill layer** (flags bit 4 + a fill key), with or without pixels. 13.2 says
   VECTOR_MASK is for ordinary pixel layers only; an empty-rect shape layer (MaxCompat off) is NO_PIXELS, and the sentence
   would otherwise also name "vector masks", which the user did not add. A layer with a rect but no colour channel is
   NO_PIXELS too (it has no picture; not seen in any file).
6. **Name padding.** The padding after the Pascal name is not skipped by count; the block walk looks for the first
   signature within the next 3 bytes, so names padded to 2 (some writers) read the same. Non-zero bytes skipped this
   way, or a scan for the next signature, count as a re-sync (`read::RESYNCS`, test-only), which the PSB and artboard
   tests assert is 0 for Photoshop's files.
7. **`Lr16` / `Lr32` / `Layr` are read only when the section's layer info holds no layers.** A PSB global block with
   one of those keys next to real layers (the long-keys test) is stepped over instead of read as a second layer list.
8. **Merged versus PhotoCraft's `convert` PNG: not "within 1 level for alpha >= 16".** PhotoCraft stores the merged
   colours matted on white in 8 bits, so one stored level hides 255 / a levels of the straight colour; measured errors
   were up to 7 levels at alpha 16 (f1, f2, f5, f12), the same at both ends (the layers composite to the PNG exactly).
   The test states the bound per pixel, as the plan allows: alpha exact, colour within 1 level at alpha 255 and within
   1 + ceil(127.5 / a) below (9 at alpha 16). 16-bit files are un-matted at 16 bits and are within 1 (synthetic test).
9. **PhotoCraft's layer PNGs have the layer opacity applied** (f8 "half": alpha 128 where the channel has 255), as
   well as the mask; the comparison applies both. f4 (stroke effect in the PNG) and f6 (a group exports as one PNG) are
   checked through the merged picture and their features only.
10. **`layers/pattern-fill.psb` composite vs merged: 0** (13.4 asked the builder to measure it); asserted exactly.
11. **Smallest fixture for "every prefix"** is the smallest of all readable fixtures (Photoshop and PhotoCraft):
    `transparentbg-gimp.psd`, 2,912 bytes. Every fixture's data ends exactly at its last byte (asserted), so every
    shorter prefix is half-written and must be an error.
12. **The sweep is fast**: the capped sweep takes about 1.1 s in the debug test run (limit 60 s), so the 2,000 stays;
    the ignored `k5_ps_sweep_large` (full 2,000 on the five large files) takes 0.3 s in `--release`.

## Numbers measured

- Fixture facts (`dump` of the reader over the 55 files, matching 13.4 row by row): composite of the layers = merged
  exactly on `layers/pixel-layer`, `type-layer`, `smartobject-layer`, `shape-layer`, `solid-color-fill`,
  `gradient-fill`, `pattern-fill.psb`, `mask`, `mask-disabled`, `hidden-layer`, `semi-transparent-layers`,
  `transparentbg-gimp`, `16bit5x5` (PSD and PSB); within 1 on `stroke.psd`, `gradient-sizes.psd`, the two grey and
  profile files; 2 to 3 on the six blend-mode files (WTE's blend kernel against Photoshop's). `outside-stroke.psd`:
  224 of 1,024 pixels differ (the stroke). `north_america_newspaper.psd`: profile "Adobe RGB (1998)".
- Byte flips: 5,000 in 0.4 s (debug); 3,670 still read, 1,330 refused; no panic.
- `cargo test --workspace --no-fail-fast`: **1,591 passed, 1 failed (WA20), 57 ignored** (`wte_core` lib 1,010 / 1 /
  33, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25). The only red test is
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`. The new `psd` tests take about 1.5 s
  together in the debug test profile.
- `cargo clippy --workspace --all-targets`: 0 warnings (constant-size chunks use `as_chunks`, as the rest of the crate).
- `cargo build --release -p wte_app`: OK, exe 29,121,024 bytes, SHA-256
  `e6ecb78a242dd8e951e31f2c4255f00ee5446c3351a9fda265bbef74e4acdb16`.
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with that exe and `tools/sticker/jobs_identity.txt`, relative output
  folder `target/sticker/p5a/sweep`, off-screen, no focus taken): **21 / 21 `"ok": true`**; `compare_baseline.py`
  against the P0 baseline: **216 / 216 files the same, PASS** (`target\sticker\p5a\compare.txt`). Looked at
  `roundtrip` (Split view, the checker sphere with the logo, WTE text and two strokes, "Saved shot.wtep"): as before.
- No `Cargo.toml` / `Cargo.lock` line changed (`git diff --stat` empty for them). No program other than the WTE exe
  (with `--screenshot ... --scenario ...`) was started; neither `photocraft.exe` nor `Photoshop.exe`.

## For the checker

- The 8 outside samples (13.4) were not read by this builder; `dump`-style reading of them is the checker's step.
- `read::parse` returns `Parsed { doc, flag_offsets, end }` (test-visible) and `read::RESYNCS` counts re-syncs in
  tests; both help byte patches and exact-walk assertions.
