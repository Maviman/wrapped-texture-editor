# Build note: P9b - the editor list (STICKER_PLAN 7 "P9", 13.1, 13.9 "P9b")

Builder, 2026-11-27, branch `stickers` (from 6d8dc7c, the P9a check; plan revision 3). App UI (kind U). P9a built the
link with the editor's name a parameter of every text; P9b adds Photoshop's discovery, the editor list, the chooser,
the default editor, Help > Editor locations..., the per-editor guard table and the question before a link is opened in
a second editor. The user's workflow holds: placement and style work in WTE never touches the link, and an editor's
Ctrl+S (now Photoshop's as well) updates the sticker in place, keeping every stamp's placement.

## What was built

| Item (plan 13.9 "P9b", 13.1) | Where | Notes |
|---|---|---|
| Prefs: three programs and the default editor, the old key still read | `state.rs` (`Prefs::photoshop_exe`, `other_editor_exe`, `default_editor`; lenient `editor_key`) | each written only when set; a P9a file (only `photocraft_exe`) and older files load unchanged; a `default_editor` that is not `photoshop` / `photocraft` / `other` reads as not chosen |
| Photoshop discovery: two keys, both registry views, injected reader | `link/launch.rs` (`RegView`, `PHOTOSHOP_KEYS`, `registry_keys`, `discover`; `WindowsRegistry` passes `RRF_SUBKEY_WOW6464KEY` / `RRF_SUBKEY_WOW6432KEY`) | App Paths `Photoshop.exe` (plain path) then `Applications\Photoshop.exe\shell\open\command` (quoted, `"%1"`); `HKCU` then `HKLM`; 64-bit view then 32-bit; the exe must exist; not found = not listed, nothing asked. PhotoCraft's keys are now also tried in both views |
| When discovery runs | `link/editor.rs` (`ensure_discovered`, `refresh_known`, `LinkUi::discovered`, `LinkUi::known`) | once per session, when an "Edit in" control is first drawn (Sticker tab, canvas header) or used, and again when Help > Editor locations... opens; found programs go into the prefs; `known` caches the editors whose program exists for the labels drawn every frame |
| The default editor, the chooser | `link/editor.rs` (`default_of`, `quiet_editor`), `link/flows.rs` (`choose_editor`), `dialogs/link.rs` (the box) | the request's editor, else the link's, else the default, else the only known one (it becomes the default, D1), else PhotoCraft (it can be located); two or more known and no default: "Edit in which program?" with **Always use this one** (ticked) and Cancel / Other program... / one button per known editor; a chooser button queues `SetDefaultEditor` first while the box is ticked (`link::button_actions`, also `link::press`) |
| "Edit in <default>" + the "Edit in..." list | `link/ui.rs` (`list_button`, `editor_list`, `listed_editors`, `LIST_LABEL`), `link/mod.rs` (`edit_in_label`, `editor_name_new`) | the main button: "Edit in <link's editor>", else "Edit in <default / only known>", else "Edit in..." (opens the chooser; the list button is then left out). The list: Photoshop (known), PhotoCraft (always), the other program (chosen), Other program..., a separator, Default: <name> radio items; `PanelHit::LinkEditInList`, `StickerHeaderEditInList`, `LinkEditorItem(i)` |
| Submenus | `panels/menu_bar.rs` (Layer > "Edit in <name>" + submenu "Edit in"), `panels/layers.rs` (row menu: the item + submenu "Edit in") | same `editor_list` |
| Help > Editor locations... | new `dialogs/editors.rs` (`Dialog::EditorLocations`), `LinkAction::{ShowEditorLocations, LocateEditor { editor, path }, ClearEditor, SetDefaultEditor}` | rows Photoshop / PhotoCraft / other program with their program or "not found" / "none chosen", Browse... / Clear; the default as radio items (Ask, then each editor whose program is known; PhotoCraft always); a note that the registry is only read; replaces P9a's "PhotoCraft location..." |
| Other program... | `LinkAction::PickOther` (`flows::run`: the file dialog "Choose the program to edit stickers in", then `EditIn` with `editor: other`) | D5: any exe, started with the file as its one argument; never discovered |
| Per-editor two-copies guard | `flows::guards_two_copies`, `edit_in` | PhotoCraft and another program: a kept child **of the same editor** that still runs asks "already open"; Photoshop never (A3) |
| Switch-editor question | `edit_in` (`QuestionKind::OtherEditor`) | a link opened in this session in editor A, then "Edit in" B: "A may still have this file open. Two programs saving the same file overwrite each other." **Open in B anyway** / Cancel (`EditRequest::switch_ok`); the launch then sets the link's editor to B |
| Texts with the editor's name | `link/mod.rs` (`editor_name` with the other program's stem, `editors_of`) | the close prompt and the Save As alias toast name every editor of their links ("Photoshop and PhotoCraft"); P9a used the first link's editor for all |
| MaxCompat sentence, status lines, "could not be replaced" (A8) | P9a code, now tested for Photoshop | Photoshop's sentence word for word in the Sticker tab (scenario); "Editor: Photoshop" + file name; a send whose file Photoshop holds without sharing: the OK message "<file> could not be replaced because another program has it open. Close it in Photoshop (or save it there), then choose Send to Photoshop again.", nothing changed |
| Import without a link | `flows::import_psd` | the editor key is the default editor's (`quiet_editor`); an unlinked import with PhotoCraft as that editor passes `other` so a smart object comes in merged (13.2: "a PSD imported without a link counts as not PhotoCraft"); a kept link follows the default editor |
| Own writes used up when seen (check-P9a open item 1) | `link/watch.rs` (`OwnWrites::take`, `judge`) | see deviation 4 |
| Test helper | `link/testkit.rs` (`with_foreign_layer`) | splices an editor's own layer record and channel data (every tagged block kept: `TySh`, `lyid` set above the seed) into a file WTE wrote: "Photoshop added its own type layer and saved" with Photoshop's bytes |
| Tests | new `link/editor_tests.rs` (13), `link/tests.rs` (+1), the menu sweep (`control_sweep/panel_tests.rs`: Help > Editor locations...) | below |
| Scenario | `selftest/scenarios/sticker_link.rs` (`photoshop_pass`, `StubPhotoshopRegistry`) | steps 1-3 and 6 again with the stub as Photoshop (no new scenario; the count stays 67) |
| Docs | ARCHITECTURE 2.x (file table), 3.3, 8.2 (state), 8.3 (menus), 8.8 (dialogs), prefs, 8.11 (`sticker_link` row), 8.16 (discovery, Edit in, new "Editor list", tests), 9.2; HANDOFF 4.1 and the branch table / status; `dist/HOW_TO_TEST.txt` "Working with PhotoCraft" items 1, 14, 15 | |

Tests (`link::editor_tests`): Photoshop discovered from the App Paths plain path, the quoted command, the 32-bit view
alone; 64 before 32, `HKCU` before `HKLM`, App Paths before the command; a path that does not exist skipped; nothing ->
none; Photoshop's keys never give PhotoCraft and another program is never discovered. Discovery once per session (the
registry is not read again) and again for Editor locations (a program that is gone is no longer known). `command_for`
for Photoshop (spaces in "Program Files", Cyrillic and accented file names) through the stub: one argument, the exe's
folder. The chooser only without a default and with two editors: its buttons, the box ticked, no file and no start
before the answer, "Edit in..." as the label; Photoshop with the box sets the default, links in Photoshop, the stub saw
Photoshop's exe, the texts name Photoshop; with a default no chooser; the box off: opened, no default; one known editor
becomes the default without a question; Cancel promotes nothing. The list (Photoshop, PhotoCraft, the other program
once chosen; PhotoCraft alone when nothing is known) and an explicit "Edit in" the other program (its exe, its stem in
the texts). A running child: PhotoCraft and another program ask "already open", Photoshop does not (and the second
start happens). A link opened in PhotoCraft asks before Photoshop (the exact sentence, "Open in Photoshop anyway", the
link's editor then Photoshop) and back; not when the link was not opened in this session. Texts: the "saved over"
question, Take Photoshop's version, Send to Photoshop, the MaxCompat sentence word for word, the close prompt naming
Photoshop and PhotoCraft. Editor locations: Browse with another file name (taken with the warning), Clear, Default /
Ask. Prefs: a P9a file and an older one load, the new keys round-trip, a bad default is none, an `other` default without
its program is none. A file held without sharing (`share_mode(0)`): the "could not be replaced" message, the file
byte-identical, no `.bak.psd`, the badge "changed in WTE", not stale, no temp file left. An unlinked import of
`photoshop\layers\smartobject-layer.psd` is merged with PhotoCraft or Photoshop as the default. The splice helper reads
back with the base layer untouched and the type layer (`TEXT`, its pixels, the new `lyid`). `link::tests`: an own write
is used up when seen, so the editor's later save of the very same bytes is reported.

## Deviations from the plan, with reasons

1. **While the chooser has to ask, the main button reads "Edit in..." and the small list button is left out.** Two
   buttons with the same label would be confusing; the chooser offers every known editor and (2) "Other program...",
   so nothing is lost. Once a default exists (or one editor is known) the main button names it and the list button
   appears beside it.
2. **The chooser has an "Other program..." button** (13.1 lists "one button per editor"): without it, "Other program..."
   would be unreachable from the Sticker tab while the chooser case hides the list button.
3. **"Always use this one" starts ticked.** D1 says no default until the user picks one in the chooser; ticked, the
   first pick is that choice (the chooser does not come back at every new link); unticking it keeps asking. Help >
   Editor locations... and the list's Default items change or clear it.
4. **Own-write CRCs are used up when the watcher sees them** (check-P9a's open item 1, a change in `link/watch.rs`,
   P9a's code). The Photoshop pass of `sticker_link` met it: the new link's file was the same logo picture as the
   PhotoCraft pass's, so the editor's save with a red bar was byte-identical to a file WTE had restored earlier in the
   run ("Restore from project"), and the watcher took it for WTE's own write for the rest of the session (the update
   never came). Real cases: the same edit on two links of one picture, or an editor saving bytes equal to something WTE
   sent. Now each registration is used once, when that content is seen settled (also when the list already has it as
   the synced content); a content WTE writes twice is registered twice. Test
   `an_own_write_is_used_up_when_seen_so_the_same_bytes_from_the_editor_are_reported`; the P9a watcher tests still
   pass (one assertion that counted the registration right after the write now allows the watcher to have used it
   already: it was a race).
5. **Step 6 of the Photoshop pass uses a sticker from Photoshop's own `layers\shape-layer.psd`, placed, and Photoshop's
   type layer is spliced into the linked file** (`testkit::with_foreign_layer`) rather than writing `type-layer.psd`
   over it: that file is one 32 x 32 text layer, so on its own it would replace the sticker's layers (the WTE layer
   "deleted"); the plan asks for "the added text layer", which is the splice with Photoshop's own bytes (its `TySh`
   block kept, its `lyid` above the seed as A1 assumes). The sticker is placed so the "needs a merged picture" state
   can be shown in the Sticker tab (the link block is drawn for the selected stamp). `outside-stroke.psd`,
   `smartobject-layer.psd` and `layers-minimal\shape-layer.psd` are written over the file as they are. The two merges
   are undone at the end (after the "needs a merged picture" file, which applies nothing; undoing first would make the
   next file a conflict).
6. **Step 1 of the Photoshop pass places two new logos** (the first pass's two logos are a sticker and a lone image
   stamp by then) and finds Photoshop through an injected registry (`StubPhotoshopRegistry`: the App Paths key names an
   empty `Photoshop.exe` in the run folder), so discovery, the chooser and the list run in the real window. A fresh
   `StubLauncher` counts the Photoshop pass's starts from zero.
7. **An import without a link** passes `other` instead of `photocraft` when PhotoCraft is the default editor (13.2 says
   such a file counts as "not PhotoCraft" for smart objects); the toast still names the default editor.
8. **Discovery also runs when the Sticker tab or the canvas header first draws an "Edit in" control** (13.1: "when Edit
   in is first used"): the label must already name the editor that a click would use. It reads a handful of registry
   values once per session.

Not built here (later packages): the PSD handle's caption and toast (P11), "Edit Layer in <name>" for paint layers (P12),
the Help table and the trademark sentence and HOW_TO_TEST "Working with Photoshop" (P13).

## Numbers measured

- `cargo test --workspace --no-fail-fast`: **1,821 passed, 1 failed (WA20), 65 ignored** (`wte_app` 663 / 0 / 24,
  `wte_core` lib 1,101 / 1 / 41, integration 15 + 9 + 8 + 25). The only red test is
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`. New: 13 `link::editor_tests` and one
  `link::tests` watcher test (`target\sticker\p9b_test.log`).
- `cargo clippy --workspace --all-targets`: 0 warnings (`OwnWrites::contains` is test-only now: the watcher uses `take`).
- `cargo build --release -p wte_app`: OK, exe 32,654,336 bytes, SHA-256
  `8e2552c6373e5831e4cfc7783c3b183bd46ccea86dacef1e4fbf092c328a145a`.
- Scenarios with that exe (`tools/objtabs/sweep.sh`, off-screen, no focus taken; `target\sticker\p9b\stickers\`), all
  `"ok": true`: `sticker_link` twice (27 and 29 s; the Photoshop pass adds about 11 s), `sticker_link_path` with
  `f1w_pc.psd` and with Photoshop's `layers	ype-layer.psd`, `smoke`, `sticker_view`, `sticker_keys`, `sticker_styles`,
  `sticker_tray`, `sticker_controls`, `sticker_edit`, `sticker_pen`, `sticker_cutout`, `sticker_text`, `sticker_layout`,
  `sticker_library`. Looked at every new shot of `sticker_link` and each matches its `expect` text: `link_pc_done`
  (Editor: PhotoCraft, Edit in PhotoCraft + Edit in...), `link_chooser` (the question, the ticked box, Cancel / Other
  program... / Photoshop / PhotoCraft over the sphere, the Sticker tab's main button "Edit in..."), `link_made_ps`
  ("Link: linked", "Editor: Photoshop", Logo_k13.psd, Edit in Photoshop / Edit in... / Update now / Send to Photoshop /
  Show file in Explorer / Unlink, the toast "Opening Logo_k13.psd in Photoshop"), `link_editor_list` (Photoshop,
  PhotoCraft, Other program..., a line, Default: Photoshop chosen, Default: PhotoCraft), `link_merged_ps` (the two
  merged toasts naming layer effects and smart objects in Photoshop), `link_needs_merged_ps` ("Link: needs a merged
  picture" in red, "Editor: Photoshop", Photoshop's sentence, no command-line button); the P9a shots are unchanged
  apart from the new "Edit in..." button beside "Edit in PhotoCraft" (`link_made`, `psd_placed`, `controls_tab`).
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\p9b\sweep\`): **21 / 21 `"ok": true`**.
  `compare_baseline.py` against the P0 baseline: 214 the same, the two known `side_tabs` small shots (five tabs since
  P6b) and `shot_tab_sticker.png` (new since P6b), as in P9a; against P9a's sweep (`target\sticker\p9a\sweep\`):
  **217 / 217 the same**. No old project or flow changed.
- Programs started: cargo, Python (`-E -P`), and the WTE release exe only with `--screenshot ... --scenario ...`
  (off-screen, no focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`; the stub launcher
  recorded every start (the Photoshop pass's "Photoshop.exe" is an empty file in the run folder); the real registry was
  read by no test and no scenario (`NoRegistry`, `ViewRegistry` / `MapRegistry` in tests, `StubPhotoshopRegistry` in the
  scenario); no prefs file was written (`save_prefs` is off in self-tests and test builds).
