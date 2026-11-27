# Check note: P9b - the editor list (Photoshop, PhotoCraft, another program)

Checker, 2026-11-27, branch `stickers` at 648ad24 (the P9b build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P9" (rev 3 note), 13.1, 13.9 "P9b" and the sections they cite (5.5, 5.6, 5.7, 13.2 for the
MaxCompat sentence and the unlinked import, 13.6 / 13.7 for what stays a hand test), with the user's decisions and the
user's workflow ("manoeuvre the sticker in WTE, edit the sticker itself in Photoshop"). Verdict: **pass after fixes**
(no blocker, no major; two minors fixed with tests, one expect text corrected; the minors left are listed at the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,821 passed / 1 failed (WA20 only) / 65 ignored (app 663 / 0 / 24, core
  1,101 / 1 / 41, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\check-p9b\test1.log`).
- `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: up to date, SHA-256
  `8e2552c6...` as reported.
- With that exe (`tools/objtabs/sweep.sh`, off-screen, no focus taken; `target\sticker\check-p9b\runs\`):
  `sticker_link` **three times** (28 s each), `sticker_link_path` with Photoshop's `layers\type-layer.psd` and with
  `layers\smartobject-layer.psd` (not run by the builder: imported merged, one layer, the smart-object toast), `smoke`,
  `sticker_controls`: all `"ok": true`.
- Every new shot looked at (the builder's and mine):

| Shot | expect | Seen |
|---|---|---|
| `link_chooser` | "Edit in which program?", the ticked "Always use this one", Cancel / Other program... / Photoshop / PhotoCraft over the sphere | matches; the Sticker tab's main button reads "Edit in..." with no second list button |
| `link_made_ps` | "Link: linked", "Editor: Photoshop", Logo_k13.psd, Edit in Photoshop / Edit in... / Update now / Send to Photoshop / Show file in Explorer / Unlink, toast "Opening Logo_k13.psd in Photoshop" | matches |
| `link_editor_list` | Photoshop, PhotoCraft, Other program..., a line, Default: Photoshop (chosen), Default: PhotoCraft | matches |
| `link_merged_ps` | the merged toasts naming layer effects and smart objects in Photoshop; the sticker shows the smart object's picture | matches, but the expect placed the sticker "at the upper left of the sphere": it sits left of / at the centre over a logo (both runs). The placement is the same in this shot, in `link_needs_merged_ps` and in the final shot after the two undos (the user's rule: Photoshop's saves never move it). Expect text corrected |
| `link_needs_merged_ps` | "Link: needs a merged picture" in red, "Editor: Photoshop", the file, Photoshop's MaxCompat sentence word for word, no command-line button | matches |
| `link_pc_done` | the PhotoCraft link's block with "Edit in PhotoCraft" + "Edit in..." | matches |
| final shot | after two undos: the shape-layer sticker with 2 layers (the shape and Photoshop's "A"), badge still "needs a merged picture" (the file on disk is still the MaxCompat-off one) | as designed |

## Plan items, one by one (13.9 "P9b", 13.1)

| Item | Found |
|---|---|
| Prefs: `photocraft_exe`, `photoshop_exe`, `other_editor_exe`, `default_editor`; an older file loads | yes; each skipped when `None`; `default_editor` lenient (an unknown key or a number reads as not chosen; `other` without its program is no default). Tested |
| Photoshop discovery: App Paths (plain path) then `Applications\Photoshop.exe\shell\open\command` (quoted + `"%1"`), HKCU then HKLM, 64-bit then 32-bit view, the exe must exist, injected reader | yes; `RRF_SUBKEY_WOW6464KEY` / `RRF_SUBKEY_WOW6432KEY` with `RRF_RT_REG_SZ`; `HKEY` constants sign-extended correctly; tests cover every row. **Checker, read only:** a throw-away ignored test (removed again, never committed) ran `WindowsRegistry` against this machine's real registry: HKCU has neither key; HKLM gives `C:\Program Files\Adobe\Adobe Photoshop CC 2019\Photoshop.exe` from App Paths and the quoted command, in both views, and `discover` returns that path. `reg query` agrees (both values are `REG_SZ`). Nothing was written, nothing started |
| Discovery once per session and again for Help > Editor locations... | yes (`LinkUi::discovered`; `ShowEditorLocations` forces it); tested with a counting reader |
| "Edit in <default>" + "Edit in..." list, submenus in the Layer menu and the row menu | yes; the list: Photoshop when known, PhotoCraft always, the other program when chosen, Other program..., Default radio items. Builder deviation 1 (no list button while the main button itself is "Edit in...") accepted: the chooser offers every editor and "Other program..." |
| The chooser only without a default and with two or more known; one known = used and made the default (D1) | yes; tested (with, without default, box off, one editor, Cancel). Deviation 3 ("Always use this one" starts ticked) accepted: the first pick is the user's choice D1 asks for, unticking keeps asking |
| Help > Editor locations... (Browse / Clear, the default) | yes (`dialogs/editors.rs`); the menu sweep opens it; no screenshot of it (minor, the sweep covers it) |
| Per-editor two-copies guard; never for Photoshop | yes; `guards_two_copies`; a child of another editor does not trigger the "already open" question; tested for all three kinds |
| The switch-editor question | yes, only for a link opened in this session in another editor; "Open in <new> anyway" sets the link's editor; tested both ways and for a link not opened this session |
| `stale_in_editor`, the warning before a rewrite, "saved over" for every editor | from P9a; the texts take the link's editor; tested for Photoshop |
| Editor-specific MaxCompat sentence | yes, word for word in a test and the scenario's Sticker tab shot. (The scenario's own Assert only calls the pure text function; the shot is what shows the tab, and it does) |
| Status lines `Editor: <name>` + file name | yes (shots) |
| "could not be replaced" (A8) | yes for Photoshop; **for another program it said "Close it in program ... Send to program again"** (minor, fixed below) |
| Tests of 13.9 (discovery rows, chooser, `command_for` for Photoshop with spaces and non-ASCII names spawning nothing, Photoshop running child asks nothing / PhotoCraft does, switching asks, old prefs) | all present (`link::editor_tests`, 13) and meaningful: each asserts on the state, not only on "no panic" |
| Scenario: `sticker_link` steps 1-3 and 6 again with the stub as Photoshop and the 13.4 fixtures | yes (deviation 5, the type layer spliced into WTE's file with Photoshop's own bytes, is the faithful reading of "the added text layer"; deviation 6, the injected registry, is what rule 5 of 0.4 requires) |
| Checker greps | spawns: `std::process::Command` only in `RealLauncher` (`#[cfg(not(test))]`, reached only through `launcher_for(false)`), the companion's adb and one test helper (`cmd /c exit`); no `ShellExecute`; nothing names `Photoshop.exe` outside discovery keys, the expected exe name and texts. No user-visible literal names an editor where the editor is variable (app and core greps: only `name()`, the Editor locations row labels and the file-dialog filter "Photoshop document") |
| ARCHITECTURE / HANDOFF / HOW_TO_TEST | updated (8.16 "Editor list", 8.8, 8.3, prefs, 9.2; HOW_TO_TEST items 1, 14, 15). The watcher's "used up" rule was missing from 8.16: added with the fix below |

## Findings

1. **Minor, fixed: a new link's first write (and a restored file) left its own-write registration behind.** The
   builder fixed check-P9a's open item 1 by using a registration up when the watcher *judges* that content. But the
   watcher's first look at a new entry stops early when the file is at the synced stamp (`poll_path`: `last_seen` none
   and meta = synced) and never judges it, so the registration of every new link's first file stayed (up to 64). After
   an editor save moved the synced content on, the editor saving exactly those first bytes again was taken for WTE's
   own write and never reported - the very bug the builder fixed, still there on the most common path (whether a
   restored file was judged depended on timing). Fix (`link/watch.rs`): that first look uses up one registration of
   the synced CRC, for the main path only (the alias keeps its rule). Test
   `link::tests::a_new_links_first_write_is_used_up_at_the_first_look` (fails without the fix: the last save is not
   reported). Real editors rarely write WTE's exact bytes, hence minor; the scenario's stub editor uses WTE's writer, so
   it matters for scenario robustness.
2. **Minor, fixed: the write error of another program's link named "program".** `flows::write_file` passed no exe to
   `name_of_key`, so "could not be replaced ... Close it in program (or save it there), then choose Send to program
   again." Now the other program's stem ("gimp-2.10"). The locked-file test runs for Photoshop and for another program
   (the latter fails without the fix).
3. **Expect text corrected:** `link_merged_ps` (see the shot table).

## Re-run after the fixes

- `cargo test --workspace --no-fail-fast`: **1,822 passed / 1 failed (WA20 only) / 65 ignored** (app 664 / 0 / 24)
  (`target\sticker\check-p9b\test2.log`). `cargo clippy --workspace --all-targets`: 0 warnings. Release build OK,
  exe SHA-256 `3a9ead5538d0a8e6c4fce01096c45135eb53802c2f1a61e5402ebc20225ad66a`.
- Scenarios with that exe (`target\sticker\check-p9b\runs2\`): `sticker_link` three times (28-29 s), `sticker_link_path`
  with Photoshop's `layers	ype-layer.psd` and with `f1w_pc.psd`, `smoke`, `sticker_controls`, `sticker_view`: all
  `"ok": true`; the Photoshop pass's shots look as before.
- Identity sweep (`tools/sticker/jobs_identity.txt` into `target\sticker\check-p9b\sweep\`), compared with P9b's sweep:
  **21 / 21 `"ok": true`, 217 / 217 files the same** (`compare_p9b.txt`, RESULT: PASS). No old
  project or flow changed.

## Minors left (not fixed; for the main session / later packages)

- **Clear does not stick for a discoverable editor.** Help > Editor locations... "Clear" forgets Photoshop's program,
  but discovery runs again whenever that dialog opens (and in the next session), so Photoshop comes back. The plan
  asks for exactly this discovery; a user who wants Photoshop out of the list can only pick another default.
- **A link whose editor is `other` on a machine without the other program** reads "Edit in program" but the click
  opens the default editor (the link's editor is not usable), without saying so.
- **"Other program..." from the chooser with "Always use this one" ticked** sets the default to `other` before the
  file dialog; a cancelled dialog leaves `default_editor = "other"` with no program, which reads as not chosen
  (harmless; Editor locations then shows no radio selected rather than "Ask").
- **An unlinked import names the default editor** in the merged toast ("...which only PhotoCraft can show... keep
  editing its layers in PhotoCraft") although nothing is linked (builder deviation 7; seen in the
  `sticker_link_path` run with `smartobject-layer.psd`). Wording only.
- **Own-write CRCs are global, not per path:** two links whose files hold identical bytes could use up each other's
  registration. Needs byte-identical files on two links at the same moment; left.
- Still owed, by rule: real Photoshop / PhotoCraft launches, the file dialogs of Other program... and Browse..., hand
  tests 19-25 (13.6), A1-A8 unverified. `REG_EXPAND_SZ` values are still refused by `RRF_RT_REG_SZ` (this machine's
  values are `REG_SZ`).

Programs started by the checker: cargo, Python (`-E -P`), `reg query` (read only), and the WTE release exe only with
`--screenshot ... --scenario ...` (off-screen, no focus taken). Never `Photoshop.exe`, `photocraft.exe` or
`photocraft-cli.exe`; nothing written to the registry or the user's prefs.
