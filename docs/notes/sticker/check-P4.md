# Check note: P4 - Project format 4 and `.wtes` (STICKER_PLAN 7 "P4")

Checker, 2026-11-26, branch `stickers`, builder commit 84158b6 (from bdec1ec, the P3b check). Independent and
adversarial: every plan item compared with the code (`git diff bdec1ec..84158b6`), the builder's numbers re-run, the two
checker items of the plan done, extra tests written to try to break the loader.

**Verdict: pass after fixes.** One major found and fixed (a damaged sticker entry could crash the load or put its
pictures into the next sticker); one cheap minor fixed (a mask on an unedited picture decoded the image once per
layer). No blocker remains.

## Results re-run by the checker

| What | Result |
|---|---|
| `cargo test --workspace --no-fail-fast`, builder's source | 1,529 passed / 1 failed (WA20) / 56 ignored, as the builder reported (`wte_core` 948 / 1 / 32, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25) |
| the same with the checker's fix and tests | **1,531 passed / 1 failed (WA20 only) / 56 ignored** (`wte_core` 950 / 1 / 32) |
| `cargo clippy --workspace --all-targets` | 0 warnings |
| `cargo build --release -p wte_app` | OK; exe 29,444,096 bytes, SHA-256 `dca76dbc7e31251894d35b989e61a52e813075e0c613975ccbddf8601b7b8d41` (with the fix) |
| identity sweep (8.5), `tools/objtabs/sweep.sh` + `tools/sticker/jobs_identity.txt`, off-screen, no focus taken | 21 / 21 `"ok": true`; `compare_baseline.py` against the P0 baseline: **216 / 216 files the same, PASS** (`target\sticker\check-p4\sweep\`, `compare.txt`). A first run with an absolute output folder showed 2 "differences" in the export dialog's Folder field (it displays the sweep's own path, `G:/...k-p4/...` instead of `target/.../`); with a relative folder, as the baseline was made, all 216 are the same. The saved `roundtrip` project says `format_version` 4, `min_reader_version` 2, no `stickers` / `links` / `styles` keys |
| screenshots looked at | `roundtrip` (Split view: checker sphere with the red and blue strokes, the logo stamp and the WTE text, Layers WTE / Logo / Paint 1 / Base, toast "Saved shot.wtep"): as before; the export dialog crops compared above |

## The plan's checker items

1. **A v4 manifest with a sticker fed to a copy of the P0 version check** (unit test, no exe run):
   `k4_check_the_p0_version_check_refuses_a_sticker_project_and_opens_a_plain_one` copies `parse_manifest`'s version
   handling from b2fb8f5 (format 3) verbatim. A save with a sticker stamp, and a save with only a kept tray sticker, are
   both refused with `TooNew { found: 4, supported: 3 }` ("project was saved by a newer version"); a save without a new
   feature is opened (as a newer, readable format) and its manifest has no `stickers` / `links` key.
2. **Byte compare of the layer PNG entries of a re-saved v3 fixture:** done twice. `k4_a_v3_project_of_the_p0_exe_loads_as_before`
   compares every layer PNG entry of a copied and of a fully re-encoded save with the P0 exe's fixture (I read the test:
   it compares the raw ZIP entry bytes, not decoded pixels). Independently, `compare_baseline.py` on my own sweep compares
   every entry of the `roundtrip` (and every other) saved `.wtep` except `manifest.json` byte for byte against the P0
   baseline, and the manifest with only `format_version` / `app_version` / `saved_unix` removed (so `min_reader_version`
   is checked to be unchanged).

## Plan items (7 "P4", 2.11, 2.12) against the code

| Item | Found | Note |
|---|---|---|
| `FORMAT_VERSION = 4`, `v3_to_v4` identity, `STEPS.len() == FORMAT_VERSION - 1` | yes | |
| manifest additions `#[serde(default)]`, skipped when empty | yes | `stickers`, `links`, `StampEntry::styles` (None for the default stack), `StampSourceEntry::Sticker`; enum names unchanged |
| entries mirror the model | yes | picture / mask as `FileRef` (deviation 2, fine: verified by the save's verify step via `file_refs`); shape geometry key `form` (deviation 3, needed with the `kind` tag) |
| archive entries `stickers/k<id:05>/p<layer:03>.png` / `m<layer:03>.png`, grey masks, `Fast` | yes | new `encode_png_gray`; blob pictures and text fonts of stickers in `referenced_blobs` |
| which stickers are saved (shown or kept) | yes | `saved_stickers`; leftovers dropped (tested) |
| `required_reader` 4 for each feature alone, unchanged otherwise | yes | table test incl. an off style entry and a kept tray sticker; links left to P5 (deviation 6) |
| load: lost picture -> empty layer + warning; lost sticker -> raster layers | yes | plus lost mask (hides the layer), font fallback, whole-entry loss; the stamps of a degraded sticker are re-baked (deviation 4, reasonable) |
| save reuse by version (review A8) | yes | see "Reuse by version" below |
| property test over 200 sequences | yes, as one chain | deviation 11 accepted: one chain of 200 edit groups with reloads and two files exercises both paths (asserted > 100 each) |
| `.wtes` round trip, deterministic bytes, reading assigns new ids | yes | fixed time stamps, entry order fixed, blobs remapped and de-duplicated on import |
| unknown extra fields still load | yes | |
| non-finite floats refused with a named message | yes | stamp styles and every sticker float kind, `.wtes` too |
| `extract_bundle` writes `stickers\<name>.png` | yes | rendered on the UI thread before the job (deviation 8); documents without stickers unchanged |
| ARCHITECTURE 5.23, 7.1-7.3, new 7.8, 9.1 | yes | |
| no new crate, `Cargo.lock` unchanged, LF for new files, no ArtCraft name | yes | the five CRLF files were CRLF at bdec1ec already (checked the raw bytes) |

### Reuse by version (the data-loss trap of review A8)

Read with the question "can a version ever stand for two different byte contents?":

- Pictures and masks draw from one counter (`Document::next_pixel_version`), so a picture and a mask never share a number,
  and the record (`PngReuse::stickers`, version -> entry) needs no kind.
- Every pixel change takes a new version: stroke begin, **every dab**, end / cancel, undo / redo of a patch
  (`StickerPatch::swap_with` then a new version), `replace_sticker_pixels`, conversion. All go through `Arc::make_mut`,
  so a snapshot taken by a save in between keeps its own buffer and version.
- `version_new_buffers` gives a fresh number to every buffer it does not know as (same `Arc`, same version) of the old
  sticker; two layers sharing one `Arc` share one number, which is correct (same bytes).
- `from_parts` gives loaded buffers fresh numbers after `load_stickers` set them all to 0, and the load record is keyed
  by those; `import_wtes` goes through `add_sticker`, so the version-0 pictures of a read `.wtes` never reach a save.
- Versions are per document and the record is per document, path, size and modification time.

No hole found; the builder's property test and the copy / encode counters agree.

## Findings

| # | Severity | What | Where | Fixed |
|---|---|---|---|---|
| 1 | major | A sticker entry that is invalid as a whole only at a later layer (two layers with one id) had already queued the PNGs of its earlier layers with the index `docs.len()`, which the **next** sticker of the file then gets. Result: the load panicked on a layer index (reproduced: `load.rs:441` "index out of bounds"), or, when the next sticker had enough layers, that sticker silently received the lost sticker's pictures and masks. Only a damaged or hand-edited file triggers it (save never writes duplicate layer ids), but opening a damaged project must degrade, not crash. | `project/load.rs` `load_stickers` | yes: on `Err` the PNGs and the layer warnings this entry added are taken back (`StickerLost` covers them). Test `k4_check_a_sticker_lost_after_queuing_its_pictures_leaves_the_next_sticker_alone` (failed with the panic before, passes now; also checks the next sticker's picture and mask are its own) |
| 2 | minor | A mask on an unedited (blob) picture decoded the whole image at load to check the mask's size, once per masked layer (the builder's open issue). | `project/load.rs` | yes: the size is decoded once per blob per load |
| 3 | minor | `Manifest::links` is parsed with P4's schema but ignored: a file with links (only a future P5 build writes them) would lose them on a plain save from this build, and a P5 schema change (rev 3: `editor`, `editor_native`) could make the whole manifest fail to parse here. P4 builds are never released, so this only matters if P5 forgets to set `force_save_as` / keep the parse lenient. | `project/sticker_entries.rs`, `load.rs` | no (P5's job; noted for the P5 checker) |
| 4 | minor | `read_wtes` reports a newer sticker file with `ProjectError::TooNew`, whose text says "project was saved by a newer version". | `sticker/file.rs` | no (wording; the app's `.wtes` UI is P6b / P8, which can word it) |
| 5 | minor | `import_wtes` adds the file's blobs before `add_sticker`; when `add_sticker` refuses (limits), the blobs stay in the store unused (never saved, since save writes only referenced blobs). | `sticker/file.rs` | no (harmless) |

Carried over and still open (not P4's): the sticker render LRU keeps pictures of earlier versions outside the memory
budget (check-P3b), and the app does not check `sticker_stroke_active()` before Save / Extract (P6a / P7a).

## Tests added by the checker

- `project::sticker_tests::k4_check_a_sticker_lost_after_queuing_its_pictures_leaves_the_next_sticker_alone`
- `project::sticker_tests::k4_check_the_p0_version_check_refuses_a_sticker_project_and_opens_a_plain_one`
