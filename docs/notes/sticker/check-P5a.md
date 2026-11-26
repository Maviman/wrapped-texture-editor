# Check note: P5a - the PSD / PSB reader (STICKER_PLAN 7 "P5", 13.2, 13.4, 13.9 "P5a")

Checker, 2026-11-26, branch `stickers`, builder commit 097fde3 (from 9cd4e0f, plan revision 3). Independent and
adversarial: every claim of `build-P5a.md` was re-run or re-read; nothing was taken on trust.

## Verdict

**pass after fixes.** One minor bug fixed (with a test); no blocker or major found. Three minor observations are left as
notes for P5c / P9 (below); none of them is a wrong result on any file of clear origin.

## Plan items (7 "P5", 13.9 "P5a") against the commit

| Item | Found | Status |
|---|---|---|
| `psd/{mod,read,descriptor}.rs`, test-only `testkit.rs` | present; `lib.rs` one line | ok |
| 55 fixtures + `LICENSE-psd-tools.txt` + README | the 55 files of `repo_set.txt` compared byte for byte with `G:\Claude_Resources\ref\psd-samples\psd-tools\` (55 / 55 equal); the licence equals the repository's `LICENSE` exactly; the README's first lines say source, commit, "licence: MIT, inferred", test data only and not shipped; `2layers.psd` and `cactus_top.psd` are not in the repo | ok |
| Reader of 5.2 with every rule of 13.2 | read line by line: modes and depth sentences (mode checked before depth, depth 1 = Bitmap), `lsct` types 0-3 and `lsdk`, `artb`, flags bit 4 and the shape / fill rule, NO_PIXELS only for an empty rect with bits 3 + 4 or a FILL / ADJUSTMENT key and never on a marker, mask flags bits 1, 3, 4, channel -3, empty and inverted rects, channels in any order, all-default ranges of any count, flat counted after `Lr16` / `Lr32` / `Layr`, UTF-8-then-1252 names, `luni` with surrogates, ICC `desc` / `mluc` with the grey exemption, the 14 long keys, padding re-sync | ok |
| Walker (`masterFXSwitch` / `enab`, `lrFX` rule, `obj ` items) | every item type and the seven reference items stepped over; depth 32, counts bounded by the bytes left; unreadable = on; `lfx2` / `lmfx` decide alone | ok |
| `forcing(smart_forces)`, `editor_native` | as 13.2; GROUP also on hidden layers (deviation 3, agreed: a hidden group would otherwise show its children) | ok |
| Tests: one `k5_ps_*` per row of 13.4, the P5 reader bullets, sweep, flips | all present; each 13.4 row asserted on the parsed document | ok |
| ARCHITECTURE 5.28 (new), 7.9 (new), 1.12, 2.1, 7.7, 9.1, 10 | present and match the code | ok |
| No `Cargo.toml` / `Cargo.lock` change | `git diff 9cd4e0f..097fde3` empty for every manifest and the lock; also after this check | ok |
| Checker duties: arithmetic on lengths, sweep under `--release`, one more CLI fixture, the 8 outside samples | below | done |

## Arithmetic on lengths (read for every multiplication and cast)

All file lengths are `u64` until compared against the bytes left (`Cur::take_u64`); `usize` casts happen only after
that check. Rect widths are computed in `i64` (`IRect::width`), so `(i32::MIN, i32::MAX)` rects cannot overflow; a layer
is decoded only when `rect.area() <= max_decode_pixels`, which bounds `w * bps * h`. The merged picture's
`row * h * channels` is at most 30000 x 2 x 30000 x 56 (about 1e11, `checked_mul`, fits `usize` on x64). PackBits: every
row needs at least `2 * ceil(row / 128)` bytes before anything is allocated (correct: a packet gives at most 128 bytes and
takes at least 2). ZIP: the output limit is the channel size and a size above 1,100 x the input is refused (deflate's
ceiling is about 1,032:1). Descriptor counts are bounded by the bytes left; `UnFl`'s `n * 8` cannot overflow after that.
ICC offsets use `checked_add`. No panic path found.

**Finding 1 (minor, fixed).** `read_layer_info` refused a layer count when `count * 46 > bytes left`. The smallest legal
record is 43 bytes (34 fixed + mask length 4 + ranges length 4 + name length byte 1), and the reader itself accepts names
padded to 2 (deviation 6), so 4 records without channels, with empty ranges and an empty name padded to 0 or 2 bytes
(43 / 44 bytes each) were refused as `Corrupt("layer info")`. Now `MIN_RECORD = 43`. Test
`k5_check_layer_count_floor_is_the_smallest_record` (fails on 097fde3, passes now). No file of the collection had hit it.

## Sweep and byte flips

- `cargo test -p wte_core --release --lib psd:: -- --include-ignored`: **72 passed**, 1.3 s in all, including
  `k5_ps_sweep_large` (2,000 prefixes of each of the five large files) and `k5_check_outside_samples`.
- Debug run: `k5_ps_truncation_sweep` and the 5,000 byte flips pass (no prefix gives a document, no panic).

## One more PhotoCraft CLI fixture (f13)

Made with the user's v0.2.0 `photocraft-cli.exe` only (headless, `< /dev/null`, from
`G:\Claude_Resources\ref\scratch\p5a_check\`; `photocraft.exe` and `Photoshop.exe` were never started):
`crates/wte_core/tests/fixtures/psd/f13_check_16bit.psb`, `_merged.png`, `_layers/` and `make_f13_check.sh`; README row
f13. It is what the builder's fixtures did not have: a **16-bit PSB** (layers in `Lr16` with an 8-byte length,
PackBits rows with 4-byte counts at 16 bits, layer count -2), a soft user mask at 16 bits, and a layer whose Pascal name
PhotoCraft wrote as `???? ?` while `luni` holds "Слой ü" (opacity 0.6 = 153, Screen). Test
`k5_check_pc_16bit_psb_against_photocrafts_pngs`: no re-sync, the walk ends at the last byte, names, `lyid`, mask rect
[6,2,50,40] default 0, Layers mode; each layer equals PhotoCraft's 16-bit layer PNG within 1 level (mask and opacity
applied); the merged picture equals the `convert` PNG (alpha within 1, colour within the un-matting bound); composite
of the layers vs merged within 2. Passes on the builder's reader unchanged.

## The 8 samples outside the repository (13.4)

Read from `G:\Claude_Resources\ref\psd-samples\psd-tools\`; the ignored test `k5_check_outside_samples` asserts all of
this when the folder is present. Every readable one: the walk ends exactly at the last byte, 0 re-syncs.

| Sample | What the reader says | Mode (Photoshop link) |
|---|---|---|
| `2layers.psd` (unknown writer) | 101 x 55, two layers "Фон" and "Слой" (UTF-8 Pascal names decoded right), no `lyid`, no 1057 = real; composite vs merged 1 | Layers |
| `third-party-psds\cactus_top.psd` | 16 x 16, channel -1 last, `lsct` type 0 on both layers (no GROUP), hidden "Background", sRGB profile (no note); header has 4 channels but a **positive** count, so the merged picture is read as opaque (5.2), see note C | Layers |
| `empty-layer.psd` | "empty" = empty picture layer; shape "normal" (flags 0x18, `vmsk`) is FILL with pixels, its rendered mask dropped; pass-through group: GROUP + BLEND_OTHER; composite vs merged 0 | Merged |
| `blend-modes\normal.psd` | three shape layers with pixels (FILL, no VECTOR_MASK) at opacity 128; composite vs merged 2 | Layers |
| `colormodes\4x4_32bit_grayscale.psd` | refused, `Depth32` (the 32-bit sentence) | - |
| `layers\hue-saturation.psd` | ADJUSTMENT + NO_PIXELS (empty rect, flags 0x18) | Merged |
| `layers-minimal\gradient-fill.psd`, `layers-minimal\pattern-fill.psd` | FILL + NO_PIXELS, no real merged picture | needs a merged picture |

## Notes for later packages (not fixed; no wrong result on any real file)

- **A. Total memory across layers is not bounded.** Each layer is limited to 67 MP, but every layer is decoded and
  kept: 100 full-canvas layers of an 8192 x 8192 document would need about 27 GB (an allocation failure aborts WTE).
  The plan's limits are per canvas and per layer only. A real document that large would be just as large inside WTE;
  P5c / P9b may want a total budget ("too large" when the sum of decoded layer areas exceeds, say, 8 x the canvas
  limit). Left as is: it changes the plan's limit semantics.
- **B. The merged ZIP stream is inflated for all header channels** (up to 56), although at most 4 are used. Bounded
  by the 1,100:1 rule; stopping early would lose the "a truncated file is never a picture" guarantee, so it stays.
- **C. Positive layer count with an extra merged channel** (`cactus_top.psd`): the plan's rule reads the merged
  picture as opaque. Layers mode does not use it; if such a third-party file ever came back Merged, its picture would
  be opaque on white. Plan rule kept.

## Runs (this checker, after the fix)

- `cargo test --workspace --no-fail-fast`: **1,593 passed, 1 failed (WA20 only, known), 58 ignored** (`wte_core` lib
  1,012 / 1 / 34, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25). Before the fix the builder's numbers were
  reproduced exactly (1,591 / 1 / 57).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 29,121,024 bytes, SHA-256
  `df49f840bee339c97b8c5a9cea4580246dc810626f61e1b2cf5b7d68eeea30f2`.
- P5a adds no scenario (core only; the reader is not yet called by the app). Identity sweep (8.5,
  `tools/objtabs/sweep.sh` with that exe, `tools/sticker/jobs_identity.txt`, off-screen, no focus taken) into
  `target\sticker\p5a_check\sweep\`: 21 / 21 `"ok": true`. `compare_baseline.py` first gave one difference,
  `export_checks/shot.png`: the upper of two "Exported ... files" toasts was caught half faded (looked at both crops:
  same text, only the toast's fade differs; a timing race of the shot, not this package). The job rerun alone:
  `"ok": true`, and with it **216 / 216 files the same, PASS** (`target\sticker\p5a_check\compare.txt`; the first
  run's folder is kept as `export_checks_first_run`).
- Only the WTE exe (with `--screenshot ... --scenario ...`) and `photocraft-cli.exe` (headless, for f13) were started.
