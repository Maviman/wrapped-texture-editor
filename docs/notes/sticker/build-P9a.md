# Build note: P9a - the editor link in the app (STICKER_PLAN 7 "P9", 5.1, 5.3 app parts, 5.5-5.8, 13.1, 13.9 "P9a")

Builder, 2026-11-26/27, branch `stickers` (from 59c7848, the P8 check; plan revision 3). App UI (kind U). The editor's
name is a parameter of every text from the start (`link::editor_name`, `editor::name_of_key`); PhotoCraft is the only
discoverable editor and the editor of every new link (P9b adds Photoshop, the editor list and the default editor).
The user's workflow holds: placement and style work in WTE never touches the link (stamps, placements and styles are not
in the PSD and not in the sticker hash), and the editor's Ctrl+S is applied to the sticker in place through
`apply_link_update`, so every stamp keeps its placement.

## What was built

| Item (plan P9 / 13.9 "P9a") | Where | Notes |
|---|---|---|
| Watcher | new `crates/wte_app/src/link/watch.rs` | one thread `wte-link-watcher`, started by `link::pump` with the first link, ended when dropped with the last (channel closed); list replaced through a channel (sent only when it changed); 1 s poll, `Poke` on focus gain and Update now, 250 ms settle (two equal polls + `psd::read`), `Truncated` waits, 30 s -> one `Unreadable`; own writes by CRC (`OwnWrites`, 64 max, registered before the rename by `OwnWritesIo`, taken back on a failed write) or the synced CRC; `Missing`, `Refused`; alias changes copied over the file (temp + rename) and reported once; one repaint request per round with events, counted in `LinkUi::repaints` |
| Launcher | new `link/launch.rs` | `command_for(editor, exe, file)` (exe + one argument + the exe's folder as `current_dir`; pure), `convert_command`; `Launcher` / `ChildProc` (Running / Exited / Unknown, `kill`); `StubLauncher` (records `(key, exe, file)`, optional fake running child, optional PNG for the conversion); the real launcher only in a non-test build (`#[cfg(not(test))]`) and only from `launcher_for(false)`: stdio null, `CREATE_NO_WINDOW`, the child kept; "Show file in Explorer" = `explorer.exe /select,"<file>"` (only on the user's click) |
| Discovery | `link/launch.rs` | injected `RegistryReader` (`WindowsRegistry`: `advapi32` `RegGetValueW` through `raw-dylib`, read only; `NoRegistry` in self-tests and tests); HKCU then HKLM, the two PhotoCraft `shell\open\command` keys, the quoted / up-to-".exe" path must exist; else the file dialog "Where is photocraft.exe?" once (another file name: a warning, taken anyway); Help > "PhotoCraft location..."; `Prefs::photocraft_exe` (written only when set) |
| Editor type | new `link/editor.rs` | `ExternalEditor { PhotoCraft, Photoshop, Other(PathBuf) }`, `name()`, `key()`, `from_key`, `exe_name`, `name_of_key` |
| Session state, pump, events | new `link/mod.rs` | `LinkUi` (`AppState::link_ui`), `LinkSession` (child, `opened`, `opened_editor`, `stale_in_editor`, `alias`, `held`, `observed`, `closed_change`, `conflict_open`, `ignore_crc`, `unreadable`, `profile_noted`), `pump` (every frame in `App::logic`), `on_event`, `busy_for` (3D tool interaction, a pending Preview edit or an open sticker stroke anywhere; in the Sticker view on that sticker a canvas drag or a focused text field), `retry_held`, `decide` (the 5.5 table through the core's `sync_action`), `badge`, `ask` (in-window question now or queued), `press`, `unsaved_line` |
| Commands | new `link/flows.rs` | Edit in (save-first with "Save..." continuing after the save; exe; a text stamp converted after a question; the shared-picture question before any file; promotion / detach; the link file written into `<project>_links\`; a missing file written again; send-first; the two-copies guard; `stale_in_editor` cleared for an exited child; launch), Send (the rewrite warning when opened; `.bak.psd`; stale set), Update now, Take / Keep / Decide later, Update / Ignore after load, Fit, Unlink, Show file, Restore, the command-line conversion (`photocraft-cli convert`, 30 s, PNG -> merged update), Import PSD as sticker (+ keep linked), `before_save` / `after_save` / `save_failed` / `after_load` |
| Dialogs | new `dialogs/link.rs`, `dispatch::close_dialog`, `dialogs/unsaved.rs` | `Dialog::Link(Box<LinkQuestion>)` for save first, shared picture, convert text, send first, send warning, already open, conflict / saved over, changed while closed, canvas too large, OK messages; Esc = Decide later / Ignore; the unsaved prompt's "Stickers linked to <editor> since the last save: ..." line |
| UI | new `link/ui.rs`; `sticker/panels.rs`, `sticker/canvas.rs`, `panels/menu_bar.rs`, `panels/layers.rs` | Sticker tab block (badge, "Editor: <name>", file, Edit in / Update now / Send to / Show file in Explorer / Unlink / Restore from project / Convert with the command-line tool, the MaxCompat sentence for "needs a merged picture"); canvas header badge + "Edit in <editor>"; Layer menu and a stamp layer's row menu "Edit in <editor>"; File > Import Sticker from PSD... and ..., Keep Linked...; Help > PhotoCraft location... |
| Project flows | `io_flows.rs`, `jobs.rs` | `JobOutput::Built::links` (`LoadReport::links`), `jobs::Saved` (links revision and the save's warnings), `install_document` resets the link state, Save As copies the link files first and sets aliases on success, load restores / asks |
| Drops and Open | `dnd.rs`, `dispatch.rs` | a `.psd` / `.psb` drop (without a model or project) and `OpenPath` import it as a tray sticker (5.8) |
| Core | `doc_links.rs` | `Document::add_sticker_from_psd` + `PsdSticker` (5.8: `link::import` with an empty map into a new kept tray sticker, one "Add Sticker" entry, optional `External` link in sync) |
| Tests | new `link/tests.rs` (23), `link/editor.rs` (1), `dnd.rs` (extended), `selftest/scenarios.rs` count test | below |
| Scenarios | new `selftest/scenarios/sticker_link.rs` | `sticker_link` (steps 1-9 of the plan), `sticker_link_path` |
| Test helper | new `link/testkit.rs` | "the editor saved this": a read PSD written back with the sticker writer with one change, via temp file + rename |
| Docs | ARCHITECTURE 1.12, 2.1, 2.2, 3.3, 4.6, 5.28, 8.2, 8.3, 8.8, 8.9, 8.11, 8.13 (pitfall 24), new 8.16, 9.2; HANDOFF 4.1, 5, 5.4; `dist/HOW_TO_TEST.txt` chapter "Working with PhotoCraft" | |

Tests (`link::tests`): `command_for` with spaces, non-ASCII, UNC and `&` / `%` paths and `convert_command`, spawning
nothing; the stub in self-test and test builds (`launcher_for(false)` is a stub in a test build: the real launcher is not
compiled there); discovery from an injected registry (key 1 quoted, key 2 unquoted, HKCU before HKLM, a missing exe
skipped, neither, Photoshop not discovered in P9a) and `exe_of_command`; the watcher: a change reported once with
`repaints` 0 over the idle polls before it and exactly 1 after, a half-written file waits, given up after the timeout
(once), a deleted file once, a refused file, WTE's own write with the old list (no event, no repaint), an alias change
copied over the file and reported once, the thread ends when dropped, `pump` starts and stops it; Edit in: save-first
writes nothing and "Save..." continues after the save, promotion + file + stub call + badge + close-prompt line + the
styles note, the shared-picture question before any file ("Only this one" keeps the other stamp an image stamp), a text
stamp converted after its question (a text layer), the two-copies guard and the stale flag cleared by an exited child;
an update applied once, an undo not re-applied by itself ("changed in WTE"), Update now; an update held by a canvas drag
and by a tool interaction, applied after (selection gone, top layer active); both changed -> Take; again -> Keep -> the
warning -> Send anyway (`.bak.psd`, opacity 128, stale) -> the editor saves over -> "saved over" -> Decide later;
send-first with both answers; a project save leaves the link file's bytes and time alone and archives it, Save As copies
the file with the old path as alias and one toast; after a load a missing file restored, a changed one asked about (the
watcher's event held until the answer; Ignore keeps the same content from applying); Import PSD as sticker (two layers,
keep linked = External link watched at the file, a refused file adds nothing); the command-line conversion as a merged
update. Temp files under `target\wte-test-tmp\link\<name>-<pid>`, removed by a guard (ARCHITECTURE 4.8).

## Deviations from the plan, with reasons

1. **Scenario step 5 holds the update with a focused text field, not a held brush drag.** The self-test runner's pointer
   steps always release in the same step (there is no press-and-hold step), and a brush drag that painted is a change in
   WTE: by the table of 5.5 the update then correctly becomes a conflict, not an apply. A focused text field of the
   Sticker view is one of the plan's busy reasons (5.6) and changes nothing: the scenario clicks into the sticker's name
   field (new `PanelHit::StickerNameField`), lets the editor save, waits 2.5 s (nothing applied, `held_update` set, the
   selection still there), clicks into the 3D half, and the update lands (selection gone, top layer active, the toast).
   The unit test `an_update_waits_while_the_sticker_view_is_busy` covers the canvas drag and a tool interaction.
2. **The first-time note is a toast, not a dialog**: "WTE's layer styles and stamp styles are not in the file: <editor>
   shows the artwork without them, and WTE adds them again to whatever comes back." (once per session, at the first
   link). A modal right as the editor starts would sit behind the editor's window and wait for an answer that changes
   nothing.
3. **The badge "menu" is a row of buttons** under the badge in the Sticker tab (Update now, Send to, Show file in
   Explorer, Unlink, Restore, Convert) plus "Edit in" in the canvas header, Layer menu and row menu: the tablet rule of
   3.10 (nothing only by right click or hover).
4. **Edit in looks for the editor's program before the shared-picture question** (5.1 lists the question first): a
   cancelled "Where is photocraft.exe?" then leaves the document as it was (the promotion or detach of the shared
   question is an undo step). The question still comes before any file is written.
5. **No "Relink" for a broken external link** (5.3): the badge reads "file missing" with Unlink; the file can be
   imported again with "Import Sticker from PSD, Keep Linked...". An `InFolder` file gets "Restore from project".
6. **File > Import has two items** ("Import Sticker from PSD..." and "..., Keep Linked...") instead of a dialog with the
   checkbox "Keep linked to this file" (5.8): no extra dialog for one choice. A dropped or opened `.psd` / `.psb` is
   imported unlinked. This needed one core function, `Document::add_sticker_from_psd` (`link::import` with an empty map
   into a new kept tray sticker), documented in 5.28.
7. **The command-line conversion runs as a polled child, not a job thread**: `photocraft-cli convert <psd>
   <name>.converted.png` beside the PSD (the links folder, not `%TEMP%`), polled every frame by `link::pump` (30 s, then
   killed), the PNG read, deleted and applied as a merged update with the PSD's stamp. Nothing blocks the UI.
8. **`LinkSession::opened_at` is a flag `opened`** (plus the editor key it was opened in): only "opened in this session"
   is ever asked.
9. **"Update now" is an explicit answer**: it clears `stale_in_editor`, a pending "changed while closed", an "Ignore"
   and a held change, then reads the file now and applies it (5.5: "Update now applies the file again").
10. **Esc on a link question** = Decide later (conflict) or Ignore (changed while closed); a button closes the dialog
    with `ShowDialog(Dialog::None)` so `CloseDialog`'s cancel meaning does not run after a real answer.
11. **The sticker in step 6 of `sticker_link` is a tray sticker imported from fixture `f1w_pc.psd`** (PhotoCraft's own
    re-save of the f1 pictures) and linked through the canvas-header path (`LinkTarget::Sticker`), so PhotoCraft's real
    files f5 (f1 + a TEXT layer with pixels) and f4 (a layer effect) apply to it exactly; the fixtures are read from the
    repository at run time (`CARGO_MANIFEST_DIR`).
12. **Scenario-count test**: a `sticker*_path` scenario is checked with a PATH (it has its steps only with one).

Not built here (P9b): the editor list, Photoshop discovery, the default editor and chooser, Help > Editor locations...,
the per-editor guard table beyond "never for Photoshop" (already in `edit_in`), the switch-editor question.

## Numbers measured

- `cargo test --workspace --no-fail-fast`: **1,804 passed, 1 failed (WA20), 65 ignored** (`wte_core` lib 1,101 / 1 /
  41, `wte_app` 646 / 0 / 24, integration 15 + 9 + 8 + 25). The only red test is
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`. The 24 `link::*` tests take about 2 s
  together (debug).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 32,396,288 bytes, SHA-256
  `82f042f4ec1b08df4ec432960ada1284cb9323e41afc8c841b56474b0be953f7`.
- Scenarios with that exe (`tools/objtabs/sweep.sh`, off-screen, no focus taken; `target\sticker\p9a\stickers\`):
  every `sticker_*` scenario `"ok": true` (view, keys, styles, tray, controls, edit, pen, cutout, text, layout, library),
  `sticker_link` twice (16 s each) and `sticker_link_path` with `f1w_pc.psd` (3 layers photo / text / shape, 64 x 48).
  Looked at every shot of `sticker_link` (`link_shared`, `link_made`, `link_updated`, `link_held_applied`, `link_merged`,
  `link_conflict`, `link_saved_over`, `link_changed_closed`, the final one), `psd_placed` and `controls_tab`: each matches
  its `expect` text (the questions over the window, the badge texts "linked" / "conflict" / "changed in PhotoCraft",
  the red bar on the model, the merged toast). `sticker_link`'s idle check: the watcher made no repaint request in
  the 3 s after the undo.
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\p9a\sweep\`): **21 / 21 `"ok": true`**.
  `compare_baseline.py` against the P0 baseline: 214 the same, the two known `side_tabs` small shots (five tabs since
  P6b) and the new `shot_tab_sticker.png`; against the P8 check's sweep: 216 of 217 the same, the one difference
  `export_checks/shot.png` in the box of an export toast (its fade caught at another frame). `export_checks` run again
  alone (`target\sticker\p9a\sweep_rerun\`): its shot is pixel-identical to the P0 baseline and to the P8 check.
  So no old project or flow changed.
- Programs started: cargo, Python (`-E -P`), and the WTE release exe only with `--screenshot ... --scenario ...`
  (off-screen, no focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`; the registry was not
  read by any test or scenario (the self-test registry is `NoRegistry`).
