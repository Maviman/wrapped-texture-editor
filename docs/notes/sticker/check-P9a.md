# Check note: P9a - the editor link in the app (PhotoCraft)

Checker, 2026-11-27, branch `stickers` at 08e1e2d (the P9a build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P9" / 13.9 "P9a" and the sections it cites (5.1, 5.3 app parts, 5.5, 5.6, 5.7, 5.8, 8.2), with
the user's decisions and plan revision 3. Verdict: **pass after fixes** (one major fixed with tests, two minors fixed
with tests, the checker's links-folder deletion added to `sticker_link`; the minors left are listed at the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,804 passed / 1 failed (WA20 only) / 65 ignored (core 1,101 / 1 / 41, app
  646 / 0 / 24, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\check-p9a\test1.log`).
- The builder's release exe (SHA-256 `82f042f4...`, as reported): `sticker_link` **three times**, all `"ok": true`
  (about 16 s each), and `sticker_link_path` with `crates\wte_core\tests\fixtures\psd\f1w_pc.psd`, `"ok": true`
  (`target\sticker\check-p9a\orig\`). Every shot looked at:

| Shot | expect | Seen |
|---|---|---|
| `link_shared` | "Shared picture" question with Cancel / Only this one / Edit all over the sphere with two logos | matches |
| `link_made` | Sticker tab: "Sticker, 1 layer", "Link: linked", "Editor: PhotoCraft", Logo_k7.psd, the five buttons | matches (plus the styles toast and "Opening Logo_k7.psd in PhotoCraft") |
| `link_updated` | the left logo with a red bar, the right one unchanged, "Sticker, 2 layers", toast | matches |
| `link_held_applied` | Sticker view: only the red bar, no dashed selection, one layer "red bar", the toast | matches |
| `link_merged` | "Updated from PhotoCraft" and the layer-effects toast; 3D unchanged | matches |
| `link_conflict` | "Both WTE and PhotoCraft changed Logo." with the three answers over the Sticker view | matches (badge "conflict" in red in the header and tab) |
| `link_saved_over` | "PhotoCraft saved over the changes WTE sent to Logo." with the three answers | matches |
| `link_changed_closed` | "Linked sticker Logo changed while the project was closed." Ignore / Update over the reopened sphere | matches |
| `psd_placed` | the PSD placed in the middle, "Sticker, 3 layers", Edit in PhotoCraft, no link lines, both toasts | matches |

## Plan items, one by one

| Item (plan 7 "P9" / 13.9 "P9a") | Found |
|---|---|
| `link/{mod,watch,launch,ui,editor}.rs`, `app.rs` drains events (`link::pump` in `logic`) | yes |
| Watcher (5.6): one thread from the first link to the last, list through a channel, 1 s poll, Poke on focus / Update now, 250 ms settle with two equal polls + `psd::read`, `Truncated` waits, 30 s -> one `Unreadable`, own writes by CRC (64, registered before the rename), `Missing`, one counted repaint per round | yes, read line by line; tests cover each; idle check in the scenario |
| Alias after Save As (5.3) | yes: copied over the file (temp + rename), reported once; dropped when the kept child exited |
| Launcher (5.7, 13.1): `command_for(editor, exe, file)` one argument + exe folder; real: stdio null, `CREATE_NO_WINDOW`, child kept; stub everywhere in self-tests and test builds | yes; `RealLauncher` is `#[cfg(not(test))]` and only reachable from `launcher_for(false)`; `AppState::new(.., selftest)` passes the self-test flag |
| Discovery: registry only (HKCU, HKLM; two keys), injected reader, exe must exist; "Where is photocraft.exe?" once; warn on another name; Help > PhotoCraft location... | yes; `NoRegistry` in self-tests and tests; `raw-dylib` `RegGetValueW` read only |
| Edit in (5.1): save first, shared-picture question before any file, text stamp converted after a question, send-first question, two-copies guard (not for Photoshop), `stale_in_editor` cleared for an exited child | yes; **"Save..." could leave the edit waiting forever (major, fixed)** |
| Send (5.5): warning when opened, `.bak.psd`, `stale_in_editor` | yes |
| Conflict / saved over / changed while closed, in-window, wait; Esc = Decide later / Ignore | yes |
| Updates held while the view is busy (5.6), applied after (selection dropped, active layer re-picked, toast) | yes (scenario holds with a focused field: builder deviation 1, accepted, the plan's brush-drag hold contradicts its own 5.5 table; unit test covers a canvas drag and a tool interaction) |
| Badge texts, Sticker tab block, canvas header, Layer menu, row menu | yes; the badge "menu" is a row of buttons (deviation 3, accepted: tablet rule 3.10) |
| Project flows: save writes no link file and archives; Save As copies + aliases + toast; load restores / asks; close prompt line | yes; **a link made while a save runs was dropped from the close prompt's line (minor, fixed)** |
| Import PSD as sticker (5.8), drop, OpenPath | yes (menu items instead of a checkbox: deviation 6, accepted); **a PSD over 4096 px was refused with the update question's text (minor, fixed)** |
| Command-line conversion | yes, a polled child, 30 s and kill (deviation 7, accepted) |
| Relink of a broken external link | not built (deviation 5): "file missing" + Unlink, re-import with Keep Linked. Accepted as a minor gap, listed below |
| Editor name a parameter of every text | yes for the link texts; P9b's checker greps the rest |
| ARCHITECTURE 3.3, 4.6, 8.3, 8.8, 8.9, 8.13, 8.16, 9.2; HOW_TO_TEST "Working with PhotoCraft" | yes |
| Tests listed in the plan | all present (`link::tests`, 23 + the editor test) |
| Scenarios `sticker_link` 1-9 and `sticker_link_path` | yes |

## Checker duties of the plan

- **Spawns:** `grep -rn "Command::new|ShellExecute|CreateProcess|.spawn()" crates`: only `link/launch.rs`'s `real`
  module (the editor, the CLI conversion, `explorer.exe /select` on the user's click), `companion/adb.rs` (earlier
  package) and a pre-existing `io_flows` unit test that runs `cmd /c exit`. Nothing names photocraft outside the real
  launcher; the real launcher is not compiled into a test build (`#[cfg(not(test))]`) and a self-test run gets the stub
  (`launcher_for(selftest)`, unit test `self_tests_and_test_builds_get_only_the_stub`).
- **Focus, flash, beep in `link/`:** none (`grep` for focus / Flash / Beep / ViewportCommand / attention finds only the
  read of `viewport().focused` that pokes the watcher, and comments).
- **`sticker_link` three times:** three passes, above.
- **The links folder deleted in mid-run:** added to `sticker_link` itself (after step 4): the whole `<run>_links\`
  folder is removed while WTE runs; the watcher reports it (`observed` = missing), the badge reads "file missing", the
  Sticker tab's **Restore from project** writes the archived copy back with the same CRC, nothing is applied, no question
  opens, no undo entry, the badge reads "linked" again (the restored content is registered in `own_writes`).
- No `Cargo.toml` / `Cargo.lock` change; new files LF, changed files keep their line endings.

## Findings

### Major (fixed): "Save..." of the save-first question could open the editor at a later, unrelated save

`LinkAction::SaveThenEdit` stored the edit in `LinkUi::after_save` and queued `Save`; only a save that ran and failed
cleared it (`save_failed`). A save that never started kept it: the Save As file dialog cancelled (the usual case on a
never-saved project), a running job (`blocked_by_job` / `refuse_while_busy`), and in a self-test any save without a
path. The next successful save, minutes or hours later, then queued "Edit in" and started the editor (an outside
program coming to the front unasked) and wrote a link file. Fix: the request waits in `save_then_edit`; `SaveThenEdit`
queues `Save` and right behind it `LinkAction::DropUnstartedEdit` (dispatch runs the queue in order); `before_save`,
which `start_save` calls only when the job really starts, moves the request to `after_save`; the drop discards whatever
is left. Tests: `an_unsaved_project_asks_to_save_first_and_writes_nothing` (the started save continues the edit) and
`check_p9a_a_save_that_did_not_start_drops_the_waiting_edit` (a later save opens nothing, the stub saw no call).

### Minor (fixed): a link made while a save ran was taken out of the close prompt's line

`after_save` cleared `made_since_save` entirely. A link made after the save's snapshot is not in the `.wtep`, and the
document stays modified (its `links_revision` is newer), so the close prompt appeared without the line that names the
sticker. Fix: `before_save` records the links the save holds (`saving_made`) and `after_save` removes only those. Test
`check_p9a_a_link_made_during_a_save_stays_in_the_close_prompt`.

### Minor (fixed): a PSD larger than a sticker could not be imported

`Document::add_sticker_from_psd` ran `link::import` with an empty map; a canvas over 4096 px (an ordinary photo-sized
PSD) returned `CanvasTooLarge`, shown as "<name> is now W x H in PhotoCraft; a sticker can be at most 4096 px.", the
text of a linked update, and nothing was imported. Fix: the import falls back to `import_fitted` (what "Fit it into the
sticker" does: one merged picture, nothing thrown away) with a toast "<file> is W x H; a sticker can be at most 4096
px, so it came in as one picture of w x h."; over the reader's limits (8192) it is still refused. Test
`check_p9a_a_psd_larger_than_a_sticker_is_imported_fitted` (6000 x 100 -> 4096 x 69, Merged, the toast; 9000 px
refused with the "too large" sentence). ARCHITECTURE 5.28, HOW_TO_TEST item 11.

## Left open (minor, not fixed)

1. **Own-write CRCs are remembered for the session (64 entries).** If the editor saves bytes identical to something WTE
   wrote earlier (for example the user undoes back to exactly WTE's sent version and the editor writes it with WTE's
   writer byte for byte), the watcher takes it as WTE's own write and reports nothing. Real editors write their own
   bytes (PhotoCraft re-trims, Photoshop adds its resources), so this needs identical output from a different writer;
   "Update now" applies the file in any case. A cure would be to forget a CRC once it has been seen settled.
2. **Relink** for a broken external link is not built (builder deviation 5): Unlink and "Import Sticker from PSD, Keep
   Linked..." do the same in two steps.
3. **"Keep WTE's version" + the send warning + Cancel** ends the conflict like "Decide later" (the held change is
   dropped, the badge still reads "conflict"); "Update now" or "Send to" settles it. Consistent, but the user might
   expect the conflict question back.
4. `hold()` sets `StickerEdit::held_update` on whatever sticker the view shows, also when the held change is of another
   sticker (busy by a 3D interaction elsewhere); the flag is only read by the scenario.
5. The watcher thread is restarted only when the watch list changes; a thread that ended by itself (it does not panic
   on any path read here) would stay down until a link changes.
6. Real-launcher details no test can run without starting a program: `explorer.exe /select,"<file>"` quoting, the
   child's working folder, `RegGetValueW` for a `REG_EXPAND_SZ` command value (refused by `RRF_RT_REG_SZ`; the
   installers write `REG_SZ`). Hand tests 1-6 and 14 of plan 8.7 are owed (HOW_TO_TEST "Working with PhotoCraft").

## Numbers after the fixes

- `cargo test --workspace --no-fail-fast`: **1,807 passed / 1 failed (WA20 only) / 65 ignored** (core 1,101 / 1 / 41,
  app 649 / 0 / 24, integration 15 + 9 + 8 + 25; `target\sticker\check-p9a	est2.log`). The three new app tests are
  `check_p9a_*`.
- `cargo clippy --workspace --all-targets`: 0 warnings (after one `as_chunks_mut` lint in the new test).
- `cargo build --release -p wte_app`: OK, SHA-256 `359ddef4a2276a373431fefc3d01bf36243fd82ead786afe686948ec9f8a3c15`.
- With that exe, off-screen, no focus taken (`target\sticker\check-p9aixed\`): `sticker_link` three times (with the
  links-folder deletion: the watcher reported it in about 0.8 s, Restore wrote it back), `sticker_link_path`
  (`f1w_pc.psd`), `smoke`, `sticker_view`, `sticker_tray`, `sticker_controls`, `sticker_styles`, `sticker_text`: all
  `"ok": true`. The fixes touch only link state, the save flow's link hooks and `add_sticker_from_psd`; a project
  without links goes through none of them (the builder's identity sweep stands).
- Programs started: cargo, Python (`-E -P`) and the WTE exe with `--screenshot ... --scenario ...` only. Never
  `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`; the registry was not read.
