# Check note: P13 - Docs, licences, version, the whole workflow

Checker, 2026-11-27, branch `stickers` at c9be8b7 (the P13 build). Plan: STICKER_PLAN 7 "P13", 4.0, 8.2, 8.5-8.7,
13.6, 13.8 D6, 13.9 "P13". Verdict: **pass after fixes** (one major in the scenario's shots, fixed; minors listed).

## Re-run (this machine)

| What | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` at c9be8b7 | app 749 / 0 / 26, core lib 1,113 / 1 (WA20) / 42, core integration 57 / 0 / 0 - as the builder said |
| after the fixes: `cargo test -p wte_app` | 749 / 0 / 26 (only the scenario file changed; the core is untouched) |
| `cargo test -p wte_app -- --ignored control_sweep` | 19 / 19 (About > Licences..., the Help dialog, the Shortcuts dialog's Sticker keys) |
| `cargo clippy --workspace --all-targets` | 0 warnings (before and after the fixes) |
| release build | c9be8b7: SHA-256 `e9c55709...` = the builder's; after the fixes `ba2588a9bf92e6f564b7b6c80bda721ad06e502d828a4cb6220882801d7789eb` |
| `sticker_flow_path` on ABeautifulGame.glb and DamagedHelmet.glb, plain and with `WTE_OLE=1 WTE_PEN_SUBCLASS=1` | `"ok": true` with the builder's exe and with the fixed one (`target\sticker\checkP13\`, final runs in `final\`) |
| the same with `WTE_FRAME_LOG=1`, `tools/sticker/frame_spans.py` | worst spans: ABeautifulGame `sticker.canvas` 0.63 ms, `sticker.render` 0.53, `ole.drop` 0.08; DamagedHelmet 0.79 / 0.66 / 0.62 ms (limit 50 ms) |
| `smoke` | ok |
| identity sweep (`tools/sticker/jobs_identity.txt`, fixed exe, `target\sticker\checkP13\sweep\`) | against the P12 check: 217 / 217 the same; against the P0 baseline: 214 the same, the two known `side_tabs` small-window shots, `shot_tab_sticker.png` only in the new sweep |
| `Cargo.lock` / `Cargo.toml` against `main` b2fb8f5 | only the two workspace versions 0.3.0 -> 0.4.0; no new `[dependencies]` line in the workspace (the fixture generator `gen_f1` of an earlier package is its own workspace) |
| LF audit | the `w/crlf` files are the ones that were CRLF before the branch (kept as they are) |
| image assets added outside the fixtures | none |
| "ArtCraft" over `crates/`, `dist/`, `installer/`, README (`grep -I`) | no match (only `licenses/photocraft-MIT.txt`) |
| "Adobe" (same, Photoshop fixture folder left out) | user-facing: only the trademark sentence (About > Licences, Help, README); test-only strings of earlier packages remain (minor 3) |
| "Edit in PhotoCraft" where the editor is variable | none in user-facing code; Help names PhotoCraft only in the PhotoCraft-only guard paragraph |

## Plan items (7 "P13" and 13.9 "P13")

| Item | Found |
|---|---|
| README (Stickers, Working with PhotoCraft and Photoshop, Licences) | present; claims checked against the code (seven blend modes = `psd/mod.rs` keys, the CLI button text, "needs a merged picture", the Help menu entries) |
| HOW_TO_TEST: every hand test of 8.7 with steps and expected result | 1, 2, 2b, 3-18 in "Stickers: the hand tests to report (1-25)", with the step numbers of the detailed chapters (checked: each number points at the matching step); 19-25 in "Working with Photoshop", word for word from 13.6; the landing list 1, 3, 4, 7, 8, 12, 15, 19, 21, 23, 24 |
| Help texts: what is lost, the guard's limits, no drag from PhotoCraft (and Photoshop), the 13.2 table, MaxCompat = Always, "Leave as is", tab bar, Export Clipboard, administrator note, trademark sentence | all in `dialogs/editor_help.rs`, tested by `the_help_says_what_the_plan_asks_for` and `the_photoshop_table_is_the_plans_table` |
| About > Licences... (`include_str!` of the licence file) | present; Back / Close / Esc / backdrop; test compares the constant with the file |
| installer `[Files]` line for `licenses\*` into `{app}\licenses` | present (Components: main); CI stages `licenses\*`; the smoke test checks `licenses\photocraft-MIT.txt` and its uninstall check ("nothing is left in the install folder") still holds; no installer built or run |
| release notes, workspace version 0.4.0, no tag | present; `the_version_is_0_4_0`; no tag on the branch |
| ARCHITECTURE 10, HANDOFF 1, 5, 5.4, 8 | present |
| scenario `sticker_flow_path` | present and green; see the finding below |
| `licenses/psd-tools-MIT.txt` only if fixtures ship | not added: correct (13.9) |

## Findings

1. **Major, fixed - shots that did not show what their `expect` says (plan 8.6 item 3 makes such a shot a gate red).**
   On ABeautifulGame:
   - `flow_style_pasted` expects "the blue square of Badge.png, with the same white die-cut border and shadow", but
     the badge was dropped at `flat_spots`' second point among the pieces: a projected stamp paints the first surface
     the viewer sees, so the pieces turned blue and no square or border could be seen.
   - `flow_saved` / `flow_reopened` expect "a wide blue band of the linked paint layer", but the stroke's first
     changed layer (King_Black's) was linked, and its rows 0.45-0.55 were nowhere to be seen.
   - On DamagedHelmet the band covered the badge, and the badge is blue like the band.

   Fix in `selftest/scenarios/sticker_flow.rs`:
   - `badge_spot`: the grid hit nearest the photo whose whole box (a 7 x 7 lattice of rays over the badge's
     estimated screen box) hits one texture set, apart from the photo's box. The Stamp tool takes the largest of the
     sizes 10 / 8 / 6 / 5 % that has such a point, chosen in a Setup and dropped by the next step. The old point is
     kept as a logged fallback.
   - `most_visible_changed`: the linked paint layer is the changed one whose set has the most grid hits.
   - `band_rows` and `editor_paints_band`: the band is a tenth of the height around the most visible rows of that
     set, clear of the rows the stickers are seen at when there are such rows. The stickers' texels are always left
     free (the helmet's scattered UV islands have no clear row). The band is now green; the badge is blue.
   - The wrap step's title said "its layer has ink" without asserting it; it now checks the home-set layer's ink.
   - The `flow_saved` / `flow_reopened` texts were updated ("the reopened shot's toast may still be fading" on a quick
     model: DamagedHelmet's reopen shot still shows the "Saved" toast).

   After the fix every shot of both models matches its `expect` (I looked at all of them):
   - ABeautifulGame: the badge sits on the board's right border, and the band runs across the board's front edge.
   - DamagedHelmet: the badge sits on the lower visor, and the band leaves both stickers free.

   ARCHITECTURE 8.11's `sticker_flow_path` row says how.
2. **Minor, not fixed - plan 8.2's "after undoing everything the document hash equals the start"** is not asserted in
   this scenario (builder deviation 4: it saves and reopens, link updates are outside the history). Each step asserts
   its own undo count, and `flow_key` asserts the reopen, so I accept the deviation. The gate should list it.
3. **Minor, not fixed (earlier packages) - "Adobe" in test-only strings.** Plan 8.6 item 7's grep over `crates/` still
   matches `link/editor_tests.rs` (a Photoshop install path) and three core tests (an ICC profile name read from a
   fixture). None of these is user-facing. The gate should either accept them as test data or have them reworded.
4. **Minor, not fixed - About > Licences' intro says the licence file is "in the program's folder"**. That is true for
   the installer. A bare exe handed out without the installer has no `licenses` folder, but the window shows the text
   itself anyway.

Nothing found in the P13 code itself (About / Licences, the Help dialog, the Shortcuts rows, the menu entry, the
installer lines, the version uses: the version string appears only in About, the crash report, `--gpu-check`, the
companion handshake as text and the project manifest's `app_version`, so no export changes with the bump).

## Rule audit

Only cargo, Python, sh and the WTE exe in `--screenshot` mode were started. No PhotoCraft, no Photoshop, no window
took the focus. Tests and scenarios use the stub launcher and the in-memory clipboard.
