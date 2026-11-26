# Build note: P5c - the link logic of the editor link (STICKER_PLAN 7 "P5", 5.3-5.5, 13.1, 13.2, 13.9 "P5c")

Builder, 2026-11-26, branch `stickers` (from 206ac77, the P5b check; plan revision 3). Core logic (kind K), headless.
Nothing is user-visible yet (the watcher, launcher, dialogs and badge are P9a / P9b), so plan 8.2 lists no scenario
for P5c and plan 10 gives it no HANDOFF or `dist/HOW_TO_TEST.txt` edit; the app-level check is the identity sweep of
8.5. The user's workflow ("I would be able to maneuver the sticker in the wrapped app, and I can edit the sticker
itself in photoshop") is what the core now supports: placement edits in WTE never touch the link (a stamp, its
placement, its styles and the sticker's layer styles are not in the PSD and not in the sticker hash, so they are
neither "changed in WTE" nor a reason to write), and a Ctrl+S in Photoshop is applied to the sticker **in place**
(`apply_link_update` replaces the sticker value through `set_sticker`; the stamps keep their placements, and only a
canvas size change refits them by 2.9).

## What was built

| Item (13.9 "P5c") | Where | Notes |
|---|---|---|
| Link data, `StickerLink::editor` | new `crates/wte_core/src/link/mod.rs` | `StickerLink` (file, editor key, mode, scale, synced, synced_sticker, layer_map, next_lyid, `editor_native`, problem), `LinkFile`, `LinkMode`, `FileStamp`, `LinkLayer` (with the name the ids rules compare), `LinkProblem`, `SMART_FORCES_OUTSIDE_PHOTOCRAFT`, editor keys, `links_folder`, `link_file_name`, `archive_entry` |
| The round trip of 5.4 | new `link/map.rs` | `link_scale`, `export` (layer content without mask and styles, mask separately, ids kept, `next_lyid`), `import` (merged / layers, the table row by row), `import_fitted` ("Fit it into the sticker"), `mask_crc` |
| The three ids rules and the two matching passes of 13.2 | `link/map.rs` `match_layers` | rule 1 no known id, rule 2 renumbered, rule 3 one layer re-numbered, the "`lyid` reuse" rule, duplicate ids, pass 1 by (CRC, size) then name then order, pass 2 by a unique name; the log line |
| States and decisions of 5.5 | `link/mod.rs` | `LinkState` with the fixed badge texts, `link_state` (a pure function of the observed stamp and the sticker hash), `sync_action`, `StaleEvent` / `next_stale` (`stale_in_editor` as a pure function), `send_first_question`, `OpenedState` / `rewrite_warning` |
| Messages with the editor's name | new `link/texts.rs` | every sentence of 5.1-5.5 / 13.1 / 13.2 takes the editor name; only the MaxCompat sentence branches on the key (Photoshop / PhotoCraft / other); `offers_cli_convert` only for PhotoCraft |
| The refused-file states | `doc_links.rs` `link_file_refused`, `LinkProblem` | the 13.2 sentence on the badge while the file has that content; a truncated file is no problem, a damaged one is "could not be read" |
| The injected-rename write (13.1 A8) | new `link/file.rs` | `write_link_file`: temp file, old file copied aside, CRC registered before the rename (`LinkFileIo::before_rename`), three more tries 200 ms apart on sharing / lock / access errors, then nothing changed (temp files deleted, `rename_failed` takes the registration back), `<stem>.bak.psd` one generation; `LinkWriteError::sentence` "could not be replaced" |
| `Document` side | new `doc_links.rs`, `document.rs` | the link map in `StickerState` (outside the history snapshots), `links_revision` / `mark_links_saved` (counted by `is_modified`), `DocumentSnapshot::{links, links_revision}`, `sticker_link_hash`, `link_state`, `prepare_link_file`, `finish_link_write`, `unlink`, `set_link_file_and_editor`, `apply_link_update` ("Update Linked Sticker"), `apply_link_fit`, `link_file_refused`; `link_render_input` (fonts of hidden text layers too) |
| Render helpers | `sticker/render.rs` | `layer_content` / `mask_raster` split out of `layer_raster` (now `content_raster(.., with_mask)`); the render itself is unchanged (identity sweep below) |
| Project | `project/{mod,save,load,sticker_entries}.rs` | `LinkEntry` with the optional `editor` (serde default `"photocraft"`), `editor_native` (alias `pc_native`), `LinkLayerEntry::name`, `LinkEntry::{of, to_link}`; save: `write_links` + `archive_choice` (the save rule), `links/k<id:05>.psd` STORED, `SaveStats::warnings` / `SaveWarning::LinkBeingWritten`, `verify` checks link entries; load: step 8 `load_links` (only reads), `LoadReport::links` (`LinkAtLoad`, `LinkFileState`), `read_link_archive` |
| Tests | new `link/tests.rs` (23), `link/fixture_tests.rs` (4 + 1 ignored) | below; `project/sticker_tests.rs` P4 test adjusted to the renamed field |
| Fixture f15 | `crates/wte_core/tests/fixtures/psd/{f15_link_wte.psd, f15_link_pc.psd, f15_link_pc_merged.png, make_f15.sh}`, README | the link export of the test sticker (text, shape, mask), re-saved by the real PhotoCraft v0.2.0 CLI (headless) |
| ARCHITECTURE | 1.12 (P5c traceability), 2.1 (rows), 5.23 (`SaveStats::warnings`, `SaveWarning`, `LoadReport::links`, `read_link_archive`), 5.28 (title, the `link` API), new 6.13, 7.1 (table row, "Links" save rule, load step 8), 7.2 (`LinkEntry`), 9.1 (`k5_link_*`), 10 (P5c row) | |

Tests (`k5_link_*`): export and untouched import give the same sticker (ids 1..n, the same bytes twice, ids never
reused, `next_lyid` for a new layer); the empty-sticker placeholder; the mapping table row by row (moved, mask on a
whole-pixel picture kept and mapped, mask on a text -> picture, pixels changed -> picture without adjustments with
styles, new, deleted, clip / hidden / opacity / Multiply / rename, a disabled mask stable on the second read); each
forcing feature (ten) -> merged with its sentence for a Photoshop and a PhotoCraft link (SMART differs), needs-merged
without a merged picture; merged replaces the layers and undo restores them (then "changed in WTE", nothing applied
again); ids rewritten to unknown values and no ids; renumbered 1..n after a delete and an add (map 1, 2, 4, 5 -> file
1..4: all untouched, a text stays text), the same with either moved layer repainted (pass 2, styles kept); rule 3
alone; duplicate ids; `lyid` reuse = deleted + new; the Photoshop-like re-save simulation (extra resources and layer
blocks, rects one pixel larger with white RGB under alpha 0, names padded to 2, 40-byte ranges); scale 2 (text
untouched, a changed layer at scale 1/2 with the same on-model place and size, canvas -> artboard); a canvas over the
artboard limit (nothing applied, the badge, then Fit) and over the reader's limits; the scale rules; the state table
and the stale rule; every badge and every refusal sentence; folder, file names and the modified flag; the atomic
write, one `.bak.psd`, retries and the failed rename; the save rule (file bytes and time unchanged by saves, CRC
equal / different and readable / half written + warning, first save with a half-written file); `LoadReport::links`
(present, changed, a copy with another time present, missing and restorable, a missing external file, load writes
nothing, the manifest round trip, the sticker hash after save and load); entries that cannot be links. Fixtures:
f15 (PhotoCraft's re-save of a link export: every layer untouched, the sticker equal, text still text, shape still
shape, mask kept), f1 and f1w, f5 / f12 / f10 / f11 / f4 / f4b / f6 / f8 / f8b / f7 / f3 / f9 / f13, and the
Photoshop files (smart object per editor with MaxCompat on and off, type layers, effects / adjustment / group,
MaxCompat-off shape -> "needs a merged picture" with Photoshop's sentence word for word, MaxCompat-on shape, masks, 16
bit, a flat file, the profile note, Lab refused).

No new crate, no `Cargo.toml` / `Cargo.lock` change. Nothing in `psd/` changed. Every new file is LF; the edited CRLF
files (`project/mod.rs`, `project/save.rs`) stay CRLF, the LF ones LF. No ArtCraft name; "Photoshop" / "PhotoCraft"
appear only as plain text in the editor-specific MaxCompat sentences and in tests (the editor names are parameters
everywhere else).

## Deviations from the plan, with reasons

1. **`link/` is a module folder** (`mod.rs`, `map.rs`, `file.rs`, `texts.rs`), not one `link.rs`, and the `Document`
   side is a new `doc_links.rs` instead of more code in `doc_stickers.rs` (934 lines already): one file per concern,
   each under the 700-line target of ARCHITECTURE 2.
2. **Rule 2 of 13.2 also fires on the name**: a layer with a known id whose pixels and name differ from that entry
   while its pixels **or its name** equal another entry. With the plan's wording alone, "renumbered after a delete and
   an add, with one layer repainted" fails when the repainted layer is the one that moved onto another layer's id (the
   only trigger is that layer, and its pixels no longer match anything): it was read as "deleted + new" and its styles
   were lost. The name catches it; the PhotoCraft lowest-free-id case (f12) is unchanged (a new layer named like
   another WTE layer would only switch to content matching, which gives the same result). Both repaint cases are
   tested.
3. **The sticker hash covers only what goes into the PSD** (artboard rectangle, per layer content / transform / mask /
   adjustments / name / visibility / opacity / blend / clip), not layer styles, `locked`, the sticker's name, `keep` or
   the artboard shape. 5.3 says "every non-pixel field by value"; but styles never reach the file (5.4), so counting
   them would make a style change "changed in WTE", ask "Send WTE's changes first?" for nothing, and turn the next
   editor save into a conflict. It also serves the user's rule that work in WTE (placement, styles) never disturbs
   the link. The hash is built from the project-format entries, so it is the same after save and load.
4. **`file_changed` compares size and CRC-32, not the modification time** (`FileStamp::same_content`): a copy of the
   same content (restore from the archive, Save As) is not a change, as 5.6 asks for WTE's own writes.
5. **A sticker without layers is written with one empty layer "Layer 1"** (map entry with layer id 0,
   `PLACEHOLDER_LAYER`) that never comes back while it stays empty (P5b check finding 1: a PSD without layers would
   open in Photoshop as an opaque picture).
6. **`.bak.psd` is made by copying the old file aside before the rename and renaming the copy afterwards**, so a
   refused rename leaves the old backup untouched (13.1: "nothing is changed (file, `.bak.psd`, ...)"). The write takes
   `LinkFileIo` (rename, pause, `before_rename`, `rename_failed`), so the failure path is tested without a lock and the
   watcher's `own_writes` registration (P9a) is taken back on failure.
7. **The modified flag**: links are outside the history, so `Document` counts `links_revision` against `links_saved`;
   `mark_saved` does not touch them and the new `mark_links_saved(DocumentSnapshot::links_revision)` is for P9a's
   background save (so a link made during a save is not marked saved). `load` marks both. Without links nothing
   changes for the app.
8. **The save rule's "readable"** = `psd::read` accepts the bytes **or** refuses them as a whole (`Unsupported`,
   `TooLarge`): such a file is complete, only of a kind WTE does not apply. A missing `InFolder` file keeps the
   previous archive entry without a warning (the badge says "file missing"). The previous entry comes from the
   `PngReuse` file (the file the document was loaded from or last saved to, while unchanged).
9. **`LoadReport::links` also carries the expected path and the archive entry** (`LinkAtLoad { sticker, state, path,
   archive }`) and `project::read_link_archive` reads a stored copy, so P9a can restore a missing file without parsing
   the manifest again. `Present` = size and time equal, or the bytes' CRC equal.
10. **Link entries that cannot be links are left out at load with a log line** (an `InFolder` name with a folder part
    or a drive, a scale below 1 or not finite, two map entries with one id, a second link of a sticker, a link of a
    sticker that was not loaded). There is no `LoadWarning` for it: `error.rs` is a shared CRLF file and only a
    hand-edited file can contain such an entry.
11. **`ImportContext::blob_sizes`**: row 3 ("mask changed" on a whole-pixel picture) needs the picture's size; an
    unedited blob picture's size is decoded by the document and handed in, so `import` stays pure.
12. **More than 32 layers, or more picture bytes than a sticker may hold** (`MAX_OWNED_BYTES`), make the file come back
    merged with that reason in the toast ("more than 32 layers"), instead of failing.
13. **A problem is stored with the CRC of the content it concerns** (`LinkProblem`) and is not saved: the badge shows
    it while the file has that content; the next content is judged again.
14. **Opacity**: an untouched layer keeps WTE's exact `f32` when it rounds to the file's byte (otherwise 0.5 would come
    back as 128 / 255 and the sticker would differ from the one sent).
15. **f15 is a new fixture** (plan 8.3 names f1 for "the comparison rule on f1, text still text, shape still shape"):
    f1 and f1w hold only pixel layers written from pictures, so they cannot show that a WTE text or shape layer stays a
    text or shape. f15 is this build's link export of a sticker with a picture, a text, a shape and a masked picture,
    re-saved by the real PhotoCraft v0.2.0 CLI. f1 and f1w are tested too (as fresh links).

Not addressed (carried): P5a check note A (no total budget across all decoded layers; per layer and per canvas only),
B and C (unchanged reader rules). P5b check finding 2 (WTE's clipping rule is not Photoshop's for a semi-transparent or
non-Normal base) is the main session's decision; the link survives it (the layers come back unchanged).

## Numbers measured

- `cargo test --workspace --no-fail-fast`: **1,638 passed, 1 failed (WA20), 62 ignored** (`wte_core` lib 1,057 / 1 /
  38, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25). Against the P5b check (1,611 / 1 / 61): +27 passed (the 23
  `link::tests` and 4 `link::fixture_tests`) and +1 ignored (`k5_link_make_f15_input`). The only red test is
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`. The 27 `k5_link_*` tests take 1.9 s
  together in the debug test profile.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 29,317,120 bytes, SHA-256
  `6d1ace58fbc75e4835063420e45db84ecd68d353f7afe8f8aeab4ee58c25f252`.
- Fixture f15: `f15_link_wte.psd` 11,120 bytes (SHA-256 `0a477bec...08d4`); PhotoCraft v0.2.0 `convert` gives
  `f15_link_pc.psd` 11,152 bytes, the same SHA-256 on two runs (`dbb6bdaf...5df8`); PhotoCraft kept `lyid` 1..4, the
  names, the order, every layer's rect and bytes and the mask, so all four layers come back untouched.
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with that exe and `tools/sticker/jobs_identity.txt` into
  `target\sticker\p5c\sweep\`, off-screen, no focus taken): **21 / 21 `"ok": true`**; `compare_baseline.py`
  against the P0 baseline: 215 of 216 files the same; the one difference was `export_checks/shot.png` in the box of
  the third export toast ("Exported 3 files to shot_ec_sel", at most 8 levels: the toast's fade-in caught at another
  frame). `export_checks` run again alone (`target\sticker\p5c\sweep_rerun\`): its shot is pixel-identical to the
  baseline, and so are its exports. So: **216 / 216 the same** once the timing flake is set aside (`compare.txt`).
  Looked at `roundtrip` (Split view: the checker sphere with the logo, the WTE text and the two strokes, Layers WTE /
  Logo / Paint 1 / Base, "Saved shot.wtep") and the `export_checks` crop: as before. The saved projects carry no
  `links` key.
- Programs started: the WTE release exe (only with `--screenshot ... --scenario ...`, off-screen, no focus taken) and
  `photocraft-cli.exe` (headless, four runs for f15). Never `photocraft.exe` or `Photoshop.exe`.
