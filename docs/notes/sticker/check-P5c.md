# Check note: P5c - the link logic of the editor link (STICKER_PLAN 7 "P5", 5.3-5.5, 13.1, 13.2, 13.9 "P5c")

Checker, 2026-11-26, branch `stickers` @ 8c9265c (builder commit). Independent, adversarial check of `link/`,
`doc_links.rs`, the project side (save rule, `LoadReport::links`, `LinkEntry`) and fixture f15. Outputs under
`target\sticker\p5c_check\`.

## Verdict

**Pass after fixes.** One major found and fixed (a "linked" badge over a sticker the file does not show, after an undo
of an update or a conflict answered "Take <editor>'s version"), plus two minors fixed (a refused file marked the project
modified; a PSD held without read sharing gave a disk-error sentence instead of "could not be replaced"). Eight checker
tests added (`link/check_tests.rs`, `k5_link_check_*`).

## Findings

### 1. Major (fixed): "Update now" after an undo, and "Take <editor>'s version", kept WTE's layers under a "linked" badge

The map entries describe each layer **as it was at the sync**, but `import` compared the file only with the map and
then kept the sticker's **current** layer for every "untouched" row. When the sticker had changed in WTE since the sync,
that current layer is not what the file shows:

- Plan 5.5 / R6: after an undo of "Update Linked Sticker" the badge reads "changed in WTE" and **Update now** must
  apply the file again. The link (outside the history) still holds the post-update map, so a layer the update had
  repainted or moved now matched its entry exactly ("untouched") and the undone, old layer was kept. Result: the
  editor's change was silently dropped, and the badge then said **"linked"** (`synced_sticker` = hash of the old
  sticker). Reproduced by `k5_link_check_update_now_after_an_undo_gives_the_files_content_again` (photo repainted and
  shape moved in the editor: after undo + Update now the photo was the old picture at the old place).
- Plan 5.5 conflict, **Take <editor>'s version** ("apply the file; WTE's changes since the last sync are one undo
  away"): a layer WTE changed and the editor did not was kept as WTE's version, and the badge went to "linked" while the
  file shows something else; the next editor save kept it too. Reproduced by
  `k5_link_check_take_the_editors_version_of_a_conflict_drops_wtes_layer_changes` (WTE moved the shape and changed the
  text's words; after "take" the shape stayed moved).

Fix (no format break, no change in the normal flow): each `LinkLayer` carries `wte: Option<LinkLayerWte { hash, at }>`,
the sticker layer at the sync: `Document::link_layer_hash` (the project-format entry without styles, name, visibility,
opacity, blend, clip and translate: content, mask, adjustments, scale, rotation; pictures by CRC) and its translate
relative to the artboard's top-left. `Document` fills it after every export (`prepare_link_file`) and every applied
import (`apply_link`); `apply_link` passes the current layers' hashes in `ImportContext::layer_hashes`. In `layers()`, a
matched layer whose current hash differs from `wte.hash` takes the "pixels differ" row (the file's pixels, styles kept);
one with the same hash at another place goes back to `at` (then the row's move applies). Entries without `wte` (or a
pure `import` call with no hashes, as the builder's unit tests do) are trusted as before. Saved as the optional
`wte` object of `LinkLayerEntry` (serde default; absent = trusted). In the normal flow (no WTE change since the sync)
every hash is equal and nothing changes: all builder tests pass unchanged except one assertion that compared the pure
import's map with the document's map (now compared without `wte`). The untouched text of the undo test stays a text.
`k5_link_check_the_sync_info_survives_save_and_load` proves the hashes are equal after save and load (an untouched
file applied after a load changes nothing).

### 2. Minor (fixed): a refused / unreadable / too-large file marked the project modified

`put_link` bumped `links_revision` for a change of `problem` alone, so `is_modified()` became true and the app would
ask "save changes?" although `problem` is never saved. `put_link` now ignores `problem` when comparing.
Test: `k5_link_check_a_refused_file_does_not_mark_the_project_modified`.

### 3. Minor (fixed): a PSD held without read sharing gave "could not be written: ... os error 32"

`write_link_file` copies the old file aside before the rename; an editor holding the PSD with share mode 0 makes that
copy fail first, and the error was `Io`, not `Locked`, so the user would not get the 13.1 A8 sentence. A copy that fails
with a sharing / lock / access error is now `Locked` (nothing changed, temp files removed). Test with a real share-mode-0
handle on a temp file under `target\wte-test-tmp\` (no other program):
`k5_link_check_a_file_held_without_sharing_gives_could_not_be_replaced` (Windows only).

### Notes, not fixed (for the main session / P9a)

- Plan 5.3 says the link file is archived "streamed"; `write_links` reads it whole into memory, and a file whose CRC
  differs from `synced` is fully decoded by `psd::read` during the save (the save rule asks for exactly that). With the
  reader's limits this is bounded but can add seconds and hundreds of MB to a save of a large, changed link. Acceptable
  for now; P9a's background save should not hold the UI on it.
- The colour-profile note names Photoshop's menu even on a PhotoCraft link (the plan's sentence, 13.2).
- Builder deviation 2 (rule 2 of 13.2 also fires on the name): I checked the cases where a PhotoCraft default name
  coincides with a WTE layer's name; content matching then gives the same result as the reuse rule. Accepted.
- A layer without a usable id (none, or a later duplicate) gets a fresh map id that the file does not carry; on the next
  read it is matched by rule 3 (tested by the builder), so it is not re-created each time.

## Plan items (13.9 "P5c", P5 list), one by one

| Item | Result |
|---|---|
| `StickerLink::editor`, `editor_native` (alias `pc_native`), manifest `editor` with default `"photocraft"` | present; P4 files load (builder test + `entries_that_cannot_be_links`) |
| `SMART_FORCES_OUTSIDE_PHOTOCRAFT`, forcing per editor with its sentence | present, ten features x two editors tested; SMART differs |
| the three ids rules and two matching passes, duplicate ids, lyid reuse (f12) | present and tested; checker's own patterns added (below) |
| comparison rule on f1 (text still text, shape still shape) | f1 / f1w (pixel layers only) and the new f15 (text, shape, mask; real PhotoCraft v0.2.0 CLI re-save); checked by hand (below) |
| mapping table row by row | builder tests every row; three rows re-derived by hand from fixture bytes (below) |
| scale 1 / 2 / 4, canvas -> artboard, 4096 limit, Fit, too large | tested; a layer that came back at scale 1/2 is sent again pixel for pixel (`k5_link_check_a_changed_layer_at_scale_two_is_sent_back_with_the_same_pixels`: copy-exact, no resampling) |
| state table, undo does not re-trigger, stale rule, "nothing written at save" | tested; `write_links` has no writer (compile-time); **Update now after the undo was broken: finding 1** |
| `.bak.psd`, injected rename, "could not be replaced" | tested; copy-lock path fixed (finding 3) |
| save rule (equal / readable / truncated + warning; first save) | tested |
| `LoadReport::links` present / changed / missing, external missing, load writes nothing | tested (directory listing before / after) |
| every refusal sentence on the badge | tested |
| ARCHITECTURE 5.28, 6.13, 7.1, 7.2, 9.1 | updated by the builder; I updated 5.28 / 6.13 / 7.2 / 9.1 for the fixes |
| Checker: nothing in `psd/` changed | `git diff 206ac77..8c9265c -- crates/wte_core/src/psd`: empty. No `Cargo.toml` / `Cargo.lock` change since 9cd4e0f |

### Three rows re-derived by hand from fixture bytes

A small Python walker (independent of WTE's reader: header, sections, layer records, `lyid` block, RLE decoder written
from the format document) parsed `f15_link_pc.psd` and `f15_link_wte.psd`. Both give the same four layers:

| lyid | name | record | rect (t, l, b, r) | normalised origin, size | CRC-32 of normalised RGBA |
|---|---|---|---|---|---|
| 1 | photo | 222 | 5, 3, 33, 31 | (3, 5), 28 x 28 | 0x64668350 |
| 2 | text | 388 | 7, 25, 21, 62 | (25, 7), 37 x 14 | 0x65068780 |
| 3 | shape | 550 | 28, 38, 42, 56 | (38, 28), 18 x 14 | 0xeffe4c02 |
| 4 | masked | 716 | 32, 4, 44, 24 | (4, 32), 20 x 12 | 0x5b45600e (mask 32, 4, 44, 24, default 255) |

PhotoCraft changed only the flags (0x08 -> 0x00) and the resources; every rect and CRC equals what WTE wrote. WTE's map
holds exactly these values (`k5_link_check_rows_derived_by_hand_from_f15_bytes`). Then, by patching the record bytes at
the offsets of the hand parse and importing:

1. **Moved**: the shape's rect +3, -2 -> `Moved(shape)`, still a shape, translate (38, 28) -> (41, 26); every other
   layer untouched.
2. **Untouched with properties from the file**: photo blend `mul `, opacity 128, flags bit 1 (hidden); text clipping
   byte 1 -> all untouched; photo Multiply, 128 / 255, hidden, same picture; text clipped and still a text.
3. **A mask marked disabled**: masked's mask flags bit 1 -> `MaskChanged { kept: true }`, no mask, same picture; applied
   once, the next read of the same file is untouched (the disabled mask's CRC is remembered).

### Renumbering patterns the builder did not test

On the builder's map 1, 2, 4, 5 (photo, text, masked, added): (a) photo deleted in the editor and the rest renumbered
1..3 by position -> rule 2, three layers untouched, photo deleted, text still text; (b) text and masked swapped in the
editor and every layer renumbered from the top down (4..1) -> rule 2, all untouched, new order, every layer equal to its
WTE layer (`k5_link_check_renumbered_after_a_delete_or_a_reorder_in_the_editor`).

## Numbers measured (this checker, after the fixes)

- `cargo test --workspace --no-fail-fast`: **1,646 passed, 1 failed (WA20 only:
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`), 62 ignored** (`wte_core` lib 1,065 / 1 /
  38, `wte_app` 524 / 0 / 24, integration 15 + 9 + 8 + 25). Against the builder (1,638 / 1 / 62): +8 checker tests.
  Before the fixes I reproduced the builder's numbers exactly (`test1.log`).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, `targetelease\wrapped_texture_editor.exe` 29,395,968 bytes, SHA-256
  `c90fb4659004b8bbe86da69b5612047e3d81f1d2f7c97d910cf039182f059771` (only test code changed after this build).
- Identity sweep (8.5, P5c has no scenario of its own): `tools/objtabs/sweep.sh` with that exe and
  `tools/sticker/jobs_identity.txt` into `target\sticker\p5c_check\sweep\`, off-screen, no focus taken: **21 / 21
  `"ok": true`**; `compare_baseline.py` against `target\stickeraseline\sweep`: **216 / 216 files the same, PASS**
  (`compare.txt`; no toast flake this time). Looked at `roundtrip/shot.png`: Split view, the checker sphere with the
  arrow logo, the WTE text and the two strokes, layers WTE / Logo / Paint 1 / Base, "Saved shot.wtep" - as before. The
  four saved `.wtep` of the sweep have no `links` key and keep their reader versions (1, 2, 1, 3).
- Line endings: every touched file is LF as before (`project/save.rs` and `project/mod.rs`, the CRLF ones, are not
  touched by the fixes); the new `link/check_tests.rs` and this note are LF.

## Programs started

`cargo` (test, clippy, release build), Python (`-E -P`, the hand parser), and the WTE release exe only with
`--screenshot ... --scenario ...` through `tools/objtabs/sweep.sh` (off-screen, no focus taken). Never `photocraft.exe`,
`photocraft-cli.exe` or `Photoshop.exe`.
