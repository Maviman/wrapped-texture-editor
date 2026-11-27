# Check note: P6b - Sticker tab, styles UI, tray

Checker, 2026-11-26, branch `stickers` at aa8e7be (the P6b build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P6b" and the sections it cites (3.6, 3.7, 3.8, 3.9, E6 in 3.1, 2.3, 3.10, 8.2, 8.5), with the
user's decisions and plan revision 3. Verdict: **pass after fixes** (no blocker; one hard-rule break in a test and two
small logic bugs fixed with tests; the minors left are listed at the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,696 passed / 1 failed (WA20 only) / 62 ignored (core 1,070 / 1 / 38, app
  569 / 0 / 24, integration 15 + 9 + 8 + 25), as the builder said.
- `cargo clippy --workspace --all-targets`: 0 warnings. Release build OK, exe SHA-256 `3f448649...ab029d12` (the
  builder's).
- Scenarios with that exe, off-screen, no focus taken (`target\sticker\p6b_check\run1\`): `sticker_styles`,
  `sticker_tray`, `sticker_controls`, `sticker_view`, `sticker_keys`, `smoke`, `side_tabs` at 1280x800, 900x600 and
  640x480: all `"ok": true`.
- Every new shot looked at against its `expect`: `styles_image` (logo with a thin white border and a soft shadow, WTE
  plain; Sticker tab in the second row, Die-cut contour and Drop shadow ticked), `styles_both` (WTE with border, dark
  stroke and shadow; the "dark box" behind WTE is the checker square and the gizmo's two rectangles: the pixels left
  of the W are the same as in `styles_image` (mean 152 / 152), so nothing was baked there), `tray_edit_before`,
  `tray_placed_twice`, `tray_question` (the dialog covers the two stamps of the 3D half), `tray_only_this` (the right
  disc lower than the left), `controls_tab` (every style on beside the plain second logo, nine boxes ticked),
  `controls_view` (Layer styles, Stroke unticked, undo label "Sticker Layer Style"), `side_tabs_tab_sticker` at the
  three sizes (640x480: Selected stamp, Sticker tray, New Sticker / From picture... inside the page). All match.
- Identity sweep (8.5, 21 jobs, `target\sticker\p6b_check\sweep\`): 21 / 21 `"ok": true`; `compare_baseline.py`
  against `target\sticker\baseline\sweep`: 214 files the same, `side_tabs/shot_tab_sticker.png` new, and
  `side_tabs/shot_small_640.png` / `shot_small_900.png` different. Checked with numpy: below the mode strip every
  differing pixel is at x >= 342 (640 wide) / x >= 601 (900 wide), i.e. inside the side panel; above it only the mode
  strip. Both areas are outside 8.5's comparison. Exports, saved layer PNGs and manifests: identical.

## Plan items, one by one

| Item (plan 7 "P6b" / cited section) | Found |
|---|---|
| 3.6 `StampToolSource { None, Blob, Sticker }` replacing `Option<BlobId>` (all users updated), `StampSettings::styles` (never serialized) | yes, tested |
| 3.6 thumbnail, "Edit before placing" (a blob source becomes a kept tray sticker first), style for new stamps | yes; styles as a list (deviation 7, accepted); `ToolSource` target replaced by `Tray(id)` (deviation 1, accepted: same behaviour) |
| 3.6 a `Blob` source with default styles places exactly today's `StampSource::Image` | yes (`k6b_1`: same pixels, object, history) |
| 3.7 Copy / Paste / Clear Style on the Sticker tab and in the Edit menu; `AppState::style_clipboard` (never the OS clipboard) | yes |
| 3.7 Paste onto every stamp of the selected layers, one entry; in the view onto the selected sticker layers | yes, tested |
| 3.7 one slider edits all selected stamps, the active one's values, a mixed hint | yes (whole stack sent, deviation 4, accepted) |
| 3.8 tray: thumbnails of every kept or used sticker, click = Stamp tool source (+ Stamp tool), Edit, Duplicate, Delete only without users, Export Picture, New Sticker (blank 512 / from a picture) | yes; Save to Library (P8) and from the clipboard (P10b) not yet, as the plan schedules |
| 3.8 Export Picture: PNG, styled from a stamp, file dialog, never in a self-test | yes (`resolve_path` ignores a dialog request in a self-test) |
| 3.9 five tabs, two rows `Tool | Color | Texture sets` / `Objects | Sticker` when one row does not fit; strip height in the split maths; `"sticker"`; `UpperTab::ALL` 0..=4 | yes, tested |
| 3.9 content: selected stamp (thumbnail, name, kind, Edit Sticker), Sticker style with presets and Copy / Paste / Clear, tray; Tool page image row `[Edit]` | yes; PSD handle (P11), Edit in PhotoCraft / link line (P9a/b), library (P8) later |
| 3.9 G1-G4 with five tabs in one row and in two rows at the three sizes | yes (one row checked at 1280x800 and 900x600 in a 420-pt panel; a 420-pt panel does not fit at 640x480: deviation 10, accepted) |
| E6 question before the first content edit of a shared sticker / picture; Edit all / Only this one / Cancel; the answer kept for the opening | yes (tray targets never ask: deviation 8; see minors) |
| 2.3 the Stroke / Outline note on text stamps | yes (`TEXT_OUTLINE_NOTE`) |
| App tests listed by the plan (control_sweep, G1-G4, indices, unknown prefs tab, no promotion, E6 answers, E5 paste one entry, Blob = Image, refitted depth) | all present |
| Scenarios `sticker_styles`, `sticker_tray` (incl. save / reopen / equality), `sticker_controls` | yes |
| ARCHITECTURE 8.3, 8.7, 8.8, 8.9, 8.15; HANDOFF 5 (+ 5.4), 6; HOW_TO_TEST "Stickers" part 1; README | yes |
| Line endings, no new crate, no Cargo change | all changed files LF; Cargo files untouched |

## Bugs found and fixed (commit of this check)

1. **Hard-rule break (plan 0.4 rule 5): a test wrote into the real `%LOCALAPPDATA%`.**
   `export_picture_writes_the_finished_picture` used `std::env::temp_dir()` (`%LOCALAPPDATA%\Temp`) for its PNGs; it
   was the only test of the workspace doing so. Now `wte_core::testutil::temp_dir` (under `target\wte-test-tmp\`).
2. **Minor: "Only this one" stored before the detach succeeded.** `answer_shared` set `share_answer = ThisStamp`
   first and then called `detach_sticker`; when the detach was refused (the edited stamp locked) the answer stayed,
   so every later edit of that opening went to the sticker the other stamp still shares, without asking. The answer
   is now stored only after the detach worked. Test `a_refused_detach_leaves_the_shared_picture_question_open`
   (fails on aa8e7be: "the question is still open").
3. **Minor: Clear Style on stamps was undone as "Paste Style".** `ClearStyle(Stamps)` used `set_stamps_styles`,
   whose entry is named "Paste Style". It now uses `set_styles_of` with no styles: one entry "Edit Sticker Style"
   (Paste keeps its name). Test `clear_style_on_stamps_is_one_style_entry` (fails on aa8e7be).
4. Cosmetic: a test line in `prefs.rs` had code and the next comment on one line.

ARCHITECTURE 8.15 (shared pictures, styles, tests) and the P6a paragraph's stale `ToolSource` updated.

## The tablet rule (3.10): every feature and its way without hover, right click, double click or a chord

| Feature | Way in |
|---|---|
| Sticker tab | the "Sticker" tab button (second row at 300 pt) |
| Edit Sticker / Export Picture... of the selected stamp | buttons on the Sticker tab; Edit also on the Tool page image row |
| Style presets (stamp and layer) | preset buttons |
| Copy / Paste / Clear Style | buttons on the Sticker tab and the Tool tab in the view; Edit menu items |
| A style on / off | the block's checkbox |
| A style's numbers | the block's arrow opens it (click); sliders drag, colour buttons click; stroke position three buttons |
| Tray: use a sticker | click its thumbnail |
| Tray: New Sticker, From picture..., Edit, Duplicate, Delete, Export Picture... | buttons (the item is selected by its click) |
| Edit before placing, style for new stamps | button and drop-down list in the Stamp tool options |
| Shared-picture answers | the dialog's three buttons (Esc and a click outside = Cancel as extras) |
| Layer styles in the view | Tool tab, under the layer's X / Y |

Only information is hover-only: a tray item's name and how often it is placed (the name is shown as "Selected: ..."
after a click; the selected stamp block shows "Its picture is shown by N stamps").

## Numbers after the fixes

- `cargo test --workspace --no-fail-fast`: **1,698 passed / 1 failed (WA20 only) / 62 ignored** (core 1,070 / 1 / 38,
  app 571 / 0 / 24: the two new tests, integration 15 + 9 + 8 + 25).
- `cargo clippy --workspace --all-targets`: 0 warnings. Release build OK, exe SHA-256
  `d182396112d28e7950172193d0c99ff6f9c0feebd82cc6557462677db5afebf2`.
- With that exe (`target\sticker\p6b_check\run2\`): `sticker_styles`, `sticker_tray`, `sticker_controls`,
  `sticker_view`, `smoke`, `side_tabs`: all `"ok": true`. The fixes touch only the E6 answer path and the Clear
  Style command (no core, layout or bake change), so the identity sweep above stands.
- Programs started: cargo, Python and the WTE release exe with `--screenshot ... --scenario ...` only (off-screen, no
  focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No Cargo file changed; every edited
  file was LF and stays LF.

## Left open (minor; not fixed here)

- Plan 3.10 "every hit target is at least 24 pt": the new buttons, checkboxes and sliders use egui's 20-pt
  `interact_size`, the style blocks' open arrows are about 14 pt (as P6a's rows and header). One app-wide decision
  (raise `interact_size.y` / `icon_width` for the sticker surfaces or for the whole panel); left to the main session.
- A tray sticker that stamps show, opened with the tray's Edit, is edited for every stamp without the E6 question
  (deviation 8: there is no "this one" in that case). A one-line "N stamps show this sticker" note in the header, or
  an "Edit all / Cancel" question, could be added with P8's tray work.
- `PictureThumbs` keeps the textures of deleted stickers until the document changes (about 100 x 100 px each).
- Export Picture encodes the PNG on the UI thread (a 4096-px styled sticker takes a moment; no progress); the other
  exports run on a job.
- The Sticker tab is long at 1280x800 (the tray sits below the nine style blocks): the builder's layout question
  stands.
- "Only this one" on a sticker stamp is two undo steps ("Detach Sticker" and the edit); 2.10 does not ask for one.
