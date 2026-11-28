# Check of package S2 (more doors: the live PNG copy and the watched folder)

Checker: independent, adversarial. Base: `b1c6536` (S1 check); checked commit `ab0af65`.

## Plan items (STICKER_PLAN 7 "S2")

| Item | Where | Result |
| --- | --- | --- |
| (a) per sticker or layer "Keep a PNG copy up to date" | Sticker tab (selected stamp's sticker, selected tray sticker), Layer menu, layer row menu, Layers tab block | present |
| written into the links folder 500 ms after the last change | `doors/live.rs` (`LIVE_COPY_DELAY`, content key by value, one job at a time, waits for interactions) | present, tested |
| for PhotoCraft's File > Place Linked | file name fixed when turned on (`<name>_k<id>.png` / `_l<id>.png`), identical bytes not rewritten | present; untried with a real PhotoCraft (hand test) |
| (b) File > Watch Folder for Stickers... | File menu + tray "Watch a folder..." | present |
| every PNG that appears or changes becomes / updates a tray sticker of that name | `doors/mod.rs` `apply` / `apply_one`, `doors/watch.rs` worker | present, tested |
| PhotoCraft's Image Assets folder `<document>-assets` | top-level `*.png` only (PhotoCraft's `default` variants in subfolders are not followed; noted by the builder) | present with that limit |
| tests and one scenario each | `doors::tests` (16 + perf), core `s2_*`, `io_flows` UiState test; scenarios `sticker_live_copy`, `sticker_watch_folder` | present |

Nothing missing.

## What I re-ran

- `cargo test --workspace --no-fail-fast` on `ab0af65`: 1,963 passed, 1 failed (WA20, the known one), 71 ignored.
- clippy `--workspace --all-targets`: 0 warnings. Release build OK, exe SHA-256 c260e36c... (as the builder said).
- Scenarios (1280x800, off-screen, no focus), all `"ok": true`: sticker_live_copy, sticker_watch_folder, smoke,
  sticker_tray, sticker_controls, layer_link, sticker_link, sticker_view, sticker_flow_path on DamagedHelmet.glb.
  Outputs `target\sticker\check_s2\runs\`.
- Shots looked at: live_copy_on (ticked box, "PNG copy: badge_k3.png, up to date", Show file, Watch a folder...),
  live_copy_layer (red stroke on the sphere, Layers tab block "Paint 1_l2.png, up to date"), watch_updated (blue badge
  placed, gizmo, toasts "2 new tray stickers" / "Updated the sticker of badge.png"; the stamp's copy box is disabled
  because that run's project is not saved yet - correct), watch_reopened (magenta badge on the sphere, tray with the
  wide badge and the green star, "Watching: doc-assets, 2 pictures from it are in the tray", Read again / Show folder /
  Stop watching). Each matches its expect text (watch_reopened's "selected" is the active Layers row; no gizmo is drawn
  after a reopen - wording only).

## What I looked for

- Back-compat: `UiState` has `#[serde(default)]`, both new fields `skip_serializing_if` and read leniently; an old
  project writes no new key. Old WTE builds ignore unknown keys. No manifest or reader change. OK.
- `mark_kept_settings_changed` bumps `links_revision`; `after_save` marks the snapshot's revision saved; nothing else
  reads `links_revision` for behaviour (close-prompt link lines use `made_since_save`). OK.
- Undo inside `apply`: `begin_edit_group` lifts the history limits, so the "Same" detection by `undo_len` is exact even
  at the 100-entry cap. A batch with no change leaves no entry. OK.
- Blob reuse: `BlobStore::insert` de-duplicates by bytes, so "Read again" of an unchanged picture is no edit. OK.
- Ids: sticker ids come from the persisted monotonic `next_id`, sticker layer ids from the per-sticker `next_layer`:
  a record can never point at an unrelated sticker after a delete + reopen. OK.
- Live copy of a raster layer: buffers are either 0 x 0 (transparent copy) or the texture's size; straight RGBA. OK.
- Links folder: nothing in WTE prunes it; the PSD link (`.psd`), the conversion (`.converted.png`) and the copy
  (`.png`) never share a name; the links folder is refused as a watched folder (by canonical path). OK.
- Threads: the folder worker ends when its channel closes (drop); a copy job's thread finishes its file after a reset
  (atomic write, the target is never half-written). No FFI, no window, nothing started; Show file / the folder dialog
  are user-initiated and go through the stub launcher in self-tests. OK.
- Line endings: every touched file is LF in the index (`git ls-files --eol`).

## Findings

1. **minor, fixed** - the folder's record (file -> sticker, last applied size / CRC) is saved with the project in
   `ui_state`, outside the history, but applying a picture did not mark the kept settings changed. After a folder
   update, an undo back to the saved state made the project unmodified while the record already held the new CRC; a
   close without saving (no prompt) dropped the record, and the reopened project then applied the undone file again by
   itself - against "undoing it does not make the folder apply the file again". Fix: `apply_one` calls
   `Document::mark_kept_settings_changed` when the record of a file changes (not for an unchanged picture or a refused
   one). Test `doors::tests::check_s2_a_changed_folder_record_keeps_the_project_modified_after_an_undo`. ARCHITECTURE
   8.18 says so.
2. **minor, not fixed** - `apply_one` decodes each settled picture again on the UI thread (`add_image_blob` decodes and
   builds the image mips; the worker had decoded it already to validate it). A 4096 px asset costs a few hundred ms of
   UI time per changed file; "Read again" of a large unchanged folder decodes every file before finding it the same.
   This is the existing pattern of the drop path (`dnd::apply`) and the link update (PSD parsed on the UI thread); not a
   correctness issue. A later change could add a core API that takes pre-decoded pixels.
3. **note** - a deleted live-copy file is not written again until its picture changes or the project is reopened
   (`written` equals the key). Harmless for Place Linked (PhotoCraft keeps its embedded copy).

## After the fix

- `cargo test -p wte_app --no-fail-fast`: 778 passed, 0 failed, 27 ignored (the new test included; `wte_core` is
  unchanged since the full run above). The new test fails without the fix (the project is unmodified after the undo).
- clippy `--workspace --all-targets`: 0 warnings. Release build OK (exe SHA-256 f7939144...).
- sticker_live_copy, sticker_watch_folder, smoke: `"ok": true` (`target\sticker\check_s2\runs2\`).

## Verdict

pass_after_fixes (one minor fixed with a test; no blocker or major found).
