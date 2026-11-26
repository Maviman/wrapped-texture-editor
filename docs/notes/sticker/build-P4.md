# Build note: P4 - Project format 4 and `.wtes` (STICKER_PLAN 7 "P4")

Builder, 2026-11-26, branch `stickers` (from bdec1ec, the P3b check). Core logic (kind K), headless. An earlier build
session stopped while working on P4 and left its work uncommitted; this builder reviewed it with `git diff`, kept what
was sound (most of it: the entry types, the save and load code, `.wtes`), finished it (tests, fixture hashes, load
re-bake of degraded stickers, docs) and fixed what was not (line endings, a test that expected the P3a refusal, a test
that asserted format 3). Nothing is user-visible yet (the tray and the library are P6b / P8), so plan 8.2 lists no
scenario for P4; the app-level check is the identity sweep of 8.5.

## What was built

| Item (plan P4) | Where | Notes |
|---|---|---|
| `FORMAT_VERSION = 4`, `v3_to_v4` (no change) | `project/{mod,migrate}.rs` | an old project loads into exactly the document it gave before |
| Format-4 entries (2.11) | new `project/sticker_entries.rs`, `project/manifest.rs` | `StickerEntry`, `StickerLayerEntry` (with `clip`), `StickerContentEntry`, `ShapeEntry`, `AdjustmentEntry`, `StyleStackEntry` + members, `ArtboardShapeEntry`, `LinkEntry` (plain data); `Manifest::{stickers, links}`, `StampEntry::styles`, `StampSourceEntry::Sticker`; every new field `#[serde(default)]`, not written when empty / default |
| Save (2.11) | `project/save.rs` | which stickers (shown or kept, `saved_stickers`); float check of stamp styles and every sticker float, named (`sticker_floats`, `style_floats`, `shape_floats`, `adjustment_floats`); a sticker stamp without its sticker refused; `stickers/k<id:05>/p<layer:03>.png` (RGBA) and `m<layer:03>.png` (8-bit grey, new `image_io::encode_png_gray`), encoded in parallel once per version; blobs of sticker layers (pictures, fonts) saved |
| `required_reader` | `project/save.rs` | 4 for a sticker stamp, non-default stamp styles (an off entry counts) or a saved sticker; unchanged otherwise (links: P5) |
| Save reuse by version (review A8) | `PngReuse::stickers` (`project/mod.rs`), `save.rs`, `load.rs` | version -> entry; copied when recorded, encoded otherwise; `load` records what it read as stored with the fresh versions of `from_parts`; `project::counters::sticker_encodes` (thread-local) |
| Load (2.11) | `project/load.rs` | step 4b `load_stickers` (PNGs read one by one, decoded in parallel); lost picture -> empty layer + `StickerPixelsLost`; lost or wrong-size mask -> empty mask (hidden) + `StickerPixelsLost { mask: true }`; missing font -> font of the file / bundled font + `StickerFontFallback`; invalid sticker entry -> lost; stamps of a missing / lost sticker -> raster layers + `StickerLost { used_by }`; stamps of a degraded sticker re-baked; sticker ids count for `next_id` |
| `Document::from_parts` with stickers | `doc_build.rs` | `DocumentParts::stickers`; fresh picture versions; validation (a stamp's sticker exists, limits) |
| Extract Original Files | `project/load.rs` `extract_bundle(.., stickers, ..)`, `sticker/file.rs` `sticker_extract_pictures`, `wte_app/src/io_flows.rs` | `stickers\<name>.png`, scale 1, no stamp styles |
| `.wtes` (2.12) | new `sticker/file.rs` | `write_wtes` / `read_wtes`, `WtesManifest { format: 1, sticker, styles, blobs }`, `WtesFile`, fixed time stamps, `Document::{saved_sticker_ids, sticker_wtes, import_wtes}` |
| Errors | `error.rs` | `LoadWarning::{StickerLost, StickerPixelsLost, StickerFontFallback}` with plain messages |
| Tests | new `project/sticker_tests.rs` (10 `k4_*`), `tests/project_roundtrip.rs` (`k4_a_v3_project_of_the_p0_exe_loads_as_before`), fixture `tests/fixtures/project/v3_roundtrip.wtep` | see below |
| ARCHITECTURE | 1.12 (traceability), 2.1 (modules), 5.20 (save note), 5.23, 7.1, 7.2 ("Format 4"), 7.3, new 7.8, 9.1, 10 | |

HANDOFF.md and `dist/HOW_TO_TEST.txt`: no change (plan section 10 gives P4 none; nothing is user-visible). No new
crate; `Cargo.lock` unchanged. No PhotoCraft or ArtCraft name in code or UI text added.

Tests (`k4_*`): required_reader table (each feature alone and undone, a leftover sticker, older features keep their
numbers); full round trip of a sticker with every layer kind (blob picture + mask, owned picture with clip, all seven
adjustments, a layer style, rotation and flip, text, star with gradient and outline, rounded rect, rounded artboard off
the origin) shared by two stamps (one styled), an image stamp with styles, a kept tray sticker and an unused one (not
written), equal after reload, equal bakes, a re-save copies every sticker PNG; lost picture / mask / sticker / artboard /
font; unknown extra fields everywhere new (and a `links` entry); non-finite floats refused with named messages (stamp and
every sticker float kind, `.wtes` too); save reuse (copy without encode, stroke + undo encodes again, stale record
unused); the property test (200 random edit / save steps: strokes on pictures and masks, replaced pictures of the same
and another size, replaced masks, undo, redo, detached copies sharing a buffer, reloads; every saved PNG compared with
the document); `.wtes` round trip, deterministic bytes, import into another document, refusals, a lost picture in a
`.wtes`; Extract writes the sticker pictures; the v3 fixture.

## Deviations from the plan, with reasons

1. **Line endings of existing files kept.** The earlier session had rewritten five files that are CRLF in the index
   (`error.rs`, `project/{manifest,migrate,mod,save}.rs`) as LF, which made a 3,000-line diff of unchanged code. They
   are CRLF again (so the diff shows only P4's changes and later merges with `main` stay clean, as the P0 checker's
   "`git ls-files --eol` unchanged" rule asks); every new file is LF.
2. **Picture and mask entries are `FileRef`s** (entry, size, CRC-32), not the bare `Option<String>` file names of plan
   2.11: the same form as the layer images, so the save's verify step and `file_refs` check them.
3. **The shape's geometry is the key `form`** (`ShapeEntry { form: {"type": ..}, .. }`): the content tag of
   `StickerContentEntry` is already `kind`, and a struct inside an internally tagged enum cannot have a field of the same
   name. Paint and form use the inner tag `type`.
4. **Stamps of a degraded sticker are re-baked at load** (not in the plan): a sticker that lost a picture, a mask or a
   font no longer looks like the stored picture of its stamps, so they are flagged like a text stamp with a font
   fallback (7.1 step 5). A lost picture is a 1 x 1 transparent owned picture (draws nothing); a lost mask is an empty
   mask of the picture's size (hides the layer, the warning says so).
5. **A sticker entry that is invalid as a whole** (artboard outside 1..=4096, two layers with one id, the limits of
   `check_limits`) is lost with `StickerLost`, and its stamps degrade as for a missing sticker.
6. **Links are parsed but not used yet.** `Manifest::links` / `LinkEntry` exist and parse (unknown keys ignored, test);
   save writes none and load ignores them; `required_reader` for links, the archive copy `links/k<id>.psd` and
   `LoadReport::links` are P5's (plan 5.3 assigns their meaning there).
7. **`.wtes` details.** Blob entries are `blobs/b<id:05>.<ext>` as in a project; the sticker's id in the file is 0;
   `read_wtes` reuses the project's `load_stickers` (same degradation, `WtesFile::warnings`); new `Document::sticker_wtes`
   (file of a document sticker with its finished picture as preview) and `import_wtes` (adds the blobs, de-duplicated by
   bytes, and the sticker as a kept tray sticker through `add_sticker`, one entry "Add Sticker").
8. **Extract renders the sticker pictures on the calling thread** (`Document::sticker_extract_pictures` needs
   `&mut Document` for the render caches) and hands them to `extract_bundle`, which got a `stickers` parameter. For a
   document without stickers nothing is rendered and the job is unchanged.
9. **The v3 fixture's hashes were recorded on HEAD bdec1ec (P3b), not on the P0 commit.** The P0 project reader and
   bake are unchanged there (the P3a and P3b identity sweeps matched the P0 baseline byte for byte), and building the P0
   commit again would have cost a full build. The test also compares directly with the P0 exe: a re-save with every image
   encoded again writes layer PNGs byte-equal to those the P0 exe wrote. The loaded and the re-baked hashes are equal
   (the debug test build bakes exactly what the P0 release exe stored).
10. **The fixture** is the P0 baseline sweep's `roundtrip/shot.wtep` (406,872 bytes; `*.wtep` is git-ignored, so it was
    added with `git add -f`).
11. **"200 random edit / save sequences"** is one random chain of 200 edit groups (1-3 edits each) with a save after each
    group and random reloads (one step in 25 on average), alternating between two files (each save copies from the previous one): this exercises
    the copy path as much as the encode path (asserted: both above 100), which 200 fresh documents would not.
12. **Font fallback in a sticker text layer** reuses a font blob of the file with the bundled font's bytes when there is
    one (the existing `fallback_font` rule), else adds the bundled font above every id the manifest mentions (the
    "floor" now also counts sticker layers, `blob_floor`).
13. **`k3a_finished_pictures_paste_style_and_save_refusal`** became `k3a_finished_pictures_paste_style_and_save`: the
    save now succeeds with `min_reader_version` 4. `required_reader_follows_fx` asserts `FORMAT_VERSION == 4`.

## Numbers measured

- `cargo test --workspace --no-fail-fast`: **1,529 passed, 1 failed (WA20), 56 ignored** (`wte_core` lib 948 / 1 / 32,
  `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25). Against the P3b check (1,518 / 1 / 56): +10 `k4_*` unit tests,
  +1 integration test. The red test is `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` only.
- The `k4_*` unit tests take about 21 s together in the debug test profile, almost all in the property test (200 saves
  and its reloads).
- v3 fixture hashes (pre-P4 code): loaded = re-baked = `(0xa53ce7bbf4a59917, 0x428ae025ace1c186)`.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 29,443,072 bytes, SHA-256
  `d244088de542fc2de4d32d5ece08458d240e5d52cc21b9f20073bf3e3d5e8187`.
- Identity sweep (8.5, `tools/objtabs/sweep.sh` with that exe and `tools/sticker/jobs_identity.txt`, off-screen, no focus
  taken) into `target\sticker\p4\sweep\`: **21 / 21 `"ok": true`**; `compare_baseline.py` against the P0 baseline:
  **216 / 216 files the same, PASS** (`target\sticker\p4\compare.txt`; the saved projects differ only in
  `format_version` 4, with `min_reader_version` 2 and 1 as before and no `stickers` / `links` keys). Looked at
  `roundtrip` (Split view: the checker sphere with the logo, the WTE text and the two strokes, "Saved shot.wtep"): as
  before.

Plan revision 3 (Photoshop as a second editor) appeared in the worktree while P4 was being finished (an uncommitted
change of `docs/plans/STICKER_PLAN.md` by the main session; not part of this commit). For P4 it changes nothing: the
link's `editor` field and the rename `pc_native` -> `editor_native` are P5's (rev 3 says "added in P5, so P4's files
load unchanged"); `LinkEntry` keeps `pc_native` until then, and no P4 file contains a link.
