# Build note: P13 - Docs, licences, version, the whole workflow (STICKER_PLAN 7 "P13"; rev 3: the P13 row of 13.9)

Builder, 2026-11-27, branch `stickers` (from 6115e9a, the P12 check; plan revision 3). Kind U. Docs, the Help and
About texts, the installer's licence folder, the workspace version 0.4.0 (no tag, no release, no merge) and the
scenario `sticker_flow_path` that runs the user's whole sticker workflow on a real model. No new crate; `Cargo.lock`
changes only by the version of the two workspace crates (0.3.0 -> 0.4.0, the one change plan 0.4 rule 1 allows).

## What was built

| Item (plan 7 "P13", 13.9 "P13", 4.0, 13.8 D6) | Where | Notes |
|---|---|---|
| Workspace version 0.4.0 | `Cargo.toml`, `Cargo.lock` (only the two `wte_*` package versions) | About, the crash report, `--gpu-check`, the companion handshake and the manifest's `app_version` read it; test `the_version_is_0_4_0` |
| About > **Licences...** | `dialogs/about.rs` (`show_licences`, `PHOTOCRAFT_MIT = include_str!("../../../../licenses/photocraft-MIT.txt")`, `LICENCES_INTRO`, `TRADEMARK`), `Dialog::Licences` | a scrolling window: what the adapted code covers and where the file is, the licence word for word (monospace, selectable), the trademark sentence of 13.8 D6; Back (to About), Close, backdrop, Esc |
| Help > **Working with PhotoCraft and Photoshop** | new `dialogs/editor_help.rs` (`LINK_HELP`, `PHOTOSHOP_TABLE`, `PHOTOSHOP_HELP`, `DND_HELP`), `Dialog::EditorHelp`, `panels/menu_bar.rs` | three chapters: the link (Edit in and the links folder; **placement edits never touch the link or the file**; what comes back; **what is lost**; conflicts and no focus; **the guard's limits, said as plan 5.7 says them**; the latest PhotoCraft's MaxCompat and the CLI conversion; linked paint layers; **no drag from PhotoCraft**), Photoshop (**the table "In Photoshop you... / In WTE" of 13.2**, **MaxCompat = Always**, **"Leave as is"** for profile questions (and the grayscale exemption), **the tab bar** for the PSD handle, **Export Clipboard**, **the administrator note**, 16-bit / refused modes, **no drag from Photoshop**), drag and drop / copy and paste (incl. **"drag out with the mouse"** of 6.4 and the companion), then the trademark sentence |
| Help > Shortcuts: the Sticker view's keys | `dialogs/about.rs` (`show_shortcuts`, `sticker_label`, `STICKER_KEY_TEXT`) | "In the Sticker view (key 4)": `sticker_table()` through `shortcut_rows`; Delete / Backspace, Esc and Enter are labelled by what they do. Hand test 18 says "the key table of Help": before P13 the Shortcuts dialog listed only the global table |
| Installer licence folder | `installer/WrappedTextureEditor.iss` (`[Files]` `Source: "{#StageDir}\licenses\*"; DestDir: "{app}\licenses"; ... Components: main`), `.github/workflows/installer.yml` (the Stage step copies `licenses\*`), `installer/smoke-test.ps1` (checks `licenses\photocraft-MIT.txt` is installed) | no installer was built or run (CI does that on a tag; no tag) |
| README | `README.md` | "Stickers (version 0.4.0)", "Working with PhotoCraft and Photoshop", "Licences" (with the trademark sentence); the old "in progress / coming" text replaced |
| Release notes | `installer/RELEASE_NOTES.md` | "What's new in 0.4.0" |
| HOW_TO_TEST | `dist/HOW_TO_TEST.txt` | new chapter **"Working with Photoshop"** = hand tests 19-25 **word for word from plan 13.6** (markdown bold removed), the 13.2 table, the no-drag and administrator lines; new chapter **"Stickers: the hand tests to report (1-25)"** = every hand test of plan 8.7 (1, 2, 2b, 3-18) with its steps and expected result and the step numbers of the detailed chapters, 19-25 pointing to the Photoshop chapter, the landing list; a last line to try About > Licences..., the Help dialog and the version |
| Scenario `sticker_flow_path` | new `selftest/scenarios/sticker_flow.rs`, `scenarios.rs` (`NAMES` 72, dispatch, the long timeouts of `sticker_link`) | below |
| Tests | `dialogs/about.rs` tests (3 new), `dialogs/editor_help.rs` tests (3), `control_sweep/panel_tests.rs` (the menu item), `control_sweep/gpu_tests.rs` (About > Licences... shows the licence text and the trademark sentence; Back / Close / Esc; the Help dialog; the Shortcuts dialog's Sticker keys), the scenario count test (72) | |
| Docs | ARCHITECTURE 2.2 (two files), 8.2 (the two dialogs), 8.7 (the Help item), 8.8 (About / Licences / Shortcuts, the Help dialog), 8.11 (the `sticker_flow_path` row), 9.2, **10 (rows P6a-P13 added to the stickers table, which stopped at P5c)**; HANDOFF 1 (a stickers / format 4 paragraph), 4.1 (72 scenarios, the scenario, its PATH note), 5 (the branch row), 5.4 (P13 and the list to report from), 8 (item 7: what stays open) | |

### `sticker_flow_path` (PATH = a model; gate: ABeautifulGame.glb, DamagedHelmet.glb)

The user's whole workflow in the real window (stub launcher, in-memory clipboard, `Step::Drop` into WTE's own drop
pipeline), one undo step per user action asserted at every step, no Error toast at any step:

1. `Photo.png` (an orange disc on white) **dropped from Explorer** (`DropItem::File`) on the model: a new image stamp
   "Photo" at the drop point, one entry.
2. The Sticker view: **Remove Background** (the white is transparent, the disc opaque; the stamp is a sticker now),
   **a text layer** "GO!" (`AddTextLayer` with the Text tool's content); Done.
3. **Die-cut + shadow**: the Classic sticker preset from the Sticker tab.
4. **Wrap it**: the stamp's placement switched to Wrap; its Final bake landed.
5. Save As, then **"Edit in <default editor>"** from the Sticker tab: PhotoCraft is the default editor; the label is
   asserted to be "Edit in " + the default editor's name (the default-editor wording of 13.9) and is clicked: the PSD in
   the links folder (two layers), the stub saw it.
6. **The user's own words**: the sticker is moved and scaled on the model in WTE: one entry, the badge stays "linked",
   the PSD's bytes and modification time are unchanged, nothing is started.
7. **A file update**: the editor saves the PSD with one more layer: one "Update Linked Sticker"; the sticker updates
   in place - the placement of step 6 (depth aside), the styles and the text layer (still text) are kept.
8. **Copy / paste the style to a second sticker**: `Badge.png` dropped at a second point; Copy Style on the photo
   sticker, Paste Style on the badge: one "Paste Style", equal styles.
9. **Link a paint layer and update it**: a brush stroke, "Edit Layer in PhotoCraft" on the first paint layer it
   changed (the label asserted), the one-layer PSD at the texture's size, the editor paints a band: one "Update Linked
   Layer", the layer holds the file's picture.
10. **Save**: neither PSD is rewritten (bytes and time).
11. **Reopen**: the same composites, stamps (placement, styles), sticker layers, links ("linked") and Wrap.
12. **Export one layer, a selection and everything as a zip** with the `export_flow` checks (`expect_layer_files`,
    `verify_export`).

Shots (each with its `expect`): `flow_dropped`, `flow_sticker_view`, `flow_styled_wrapped`, `flow_linked`,
`flow_updated`, `flow_style_pasted`, `flow_saved`, `flow_reopened`.

## Deviations (with reasons)

1. **The text layer is added with the Text tool's content** (`AddTextLayer` after setting the tool's text to "GO!"),
   not typed into the layer's content field: with the Sticker tab in front the Text tab's field is not shown, so it
   never takes the focus, and an Esc meant for the field left the Sticker view (seen in the first run). The field
   itself is covered by `sticker_text`.
2. **Drop points** come from a new helper `flat_spots` (the grid hit with the most neighbours of its own texture set
   facing the viewer), not `stamp_spots`: on ABeautifulGame `stamp_spots` picks a pawn, where a 15 % sticker covers
   several pieces and the wrapped result is unreadable in the shots. With `flat_spots` the photo sticker lands on the
   board (ABeautifulGame) and on the helmet's shell (DamagedHelmet).
3. **The linked paint layer is the first paint layer the stroke changed**: a 3D stroke on ABeautifulGame crosses up to
   eight texture sets; the scenario links one of them (logged).
4. **"After undoing everything the document hash equals the start" (8.2) is not asserted** in this scenario: it saves
   and reopens the project (the history starts empty after the reopen) and its link updates are outside the history
   (R6). Every step asserts its own undo count instead; the round trip is asserted by `flow_key` after the reopen.
   `objects_flow_path` and `export_flow`, the other whole-workflow scenarios, do the same.
5. **The second sticker stays in Project mode** (only the first one is wrapped): the plan's flow wraps "it" (the first
   sticker). On ABeautifulGame the badge's projection box therefore also reaches the pieces next to it (visible in
   `flow_style_pasted` / `flow_saved`); this is how projected stamps work, not a sticker flaw.
6. **Help > Shortcuts now lists the Sticker view's keys** (not in the P13 file list): hand test 18 asks the user to
   compare against "the key table of Help", which did not exist. Small, static, tested.
7. **`licenses\psd-tools-MIT.txt` is not added**: as 13.9 says, the Photoshop fixtures stay in the test folder (with
   their own `LICENSE-psd-tools.txt`) and the installer ships no fixture.
8. **"Adobe" outside the trademark sentence**: the only other occurrences under `crates/` (fixture folder left out,
   `grep -I`) are test data from earlier packages - the Photoshop install path in `link/editor_tests.rs` (P9b) and the
   ICC profile name "Adobe RGB (1998)" read from a fixture in three core tests (P5a / P5c). None is user-facing text.
   The P13 test that checks the Help texts takes the company name from the trademark sentence, and the copyright
   holder's name from the licence file, so neither is spelled in code.

## Numbers (this machine, release, 1280 x 800, off-screen)

| What | Result |
|---|---|
| `sticker_flow_path` on ABeautifulGame.glb | `"ok": true`, 8 shots, about 36 s; zip 214,779,232 bytes, 84 files verified (15 flattened, 39 per-layer, 30 layered, 15 sets recomposed exactly) |
| the same with `WTE_OLE=1 WTE_PEN_SUBCLASS=1` | `"ok": true` |
| `sticker_flow_path` on DamagedHelmet.glb | `"ok": true`, 8 shots, about 10 s; zip 40,568,498 bytes, 8 files verified |
| the same with `WTE_OLE=1 WTE_PEN_SUBCLASS=1` | `"ok": true` |
| Frame spans (8.4: no frame over 50 ms in `sticker.` / `link.` / `ole.` spans), `WTE_FRAME_LOG=1`, `tools/sticker/frame_spans.py`, final exe | ABeautifulGame (3,624 frames): worst `sticker.canvas` 0.71 ms, `sticker.render` 0.58 ms, `ole.drop` 0.08 ms; DamagedHelmet (1,188 frames): 0.85 / 0.73 / 0.15 ms. No `link.` span appears in these runs (the link's work runs on the watcher and job threads) |
| Each sticker edit on ABeautifulGame re-bakes the stamp's Final on the worker | ~1.35 s per edit (log "final bake ... applied after 1372 ms"), off the UI thread |

## Results

- `cargo test --workspace --no-fail-fast`: **1,919 passed, 1 failed (WA20, the known red), 68 ignored** (app 749 / 0 /
  26, core lib 1,113 / 1 / 42, core integration 57 / 0 / 0). P12 check: 1,913; the 6 new are the about and
  editor_help tests. The app tests were run again after the last edit: 749 / 0 / 26.
- `cargo test -p wte_app -- --ignored control_sweep` (the GPU control sweep, headless wgpu device, no window): 19 / 19
  passed, including the extended dialog test (Licences, the Help dialog, the Shortcuts dialog's Sticker keys).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK; exe SHA-256 `e9c55709a8fa82bae14a960048af914496116710308b7b8dfa8a78e6565603df`.
- Scenarios with that exe: `sticker_flow_path` on ABeautifulGame.glb and DamagedHelmet.glb, each plain and with
  `WTE_OLE=1 WTE_PEN_SUBCLASS=1`, and again with `WTE_FRAME_LOG=1`: all `"ok": true`, 8 shots each; `smoke` ok. I
  looked at every shot of both models: each matches its `expect` (on DamagedHelmet the projected photo sticker also
  shows at the left of the shell, where the helmet's texture is mirrored: a UV effect of the model, not a sticker flaw;
  on ABeautifulGame the badge's projection box reaches the pieces next to it, see deviation 5).
- Identity sweep (8.5, `tools/sticker/jobs_identity.txt` into `target\sticker\p13\sweep\`): against the P12 check's
  sweep **217 / 217 the same**; against the P0 baseline the same result as the P12 check (214 the same, the two
  known `side_tabs` small-window shots that P6b's fifth tab changed, `shot_tab_sticker.png` only in the new sweep).
  The version bump changes only the manifests' `app_version`, which the comparison leaves out (plan 8.5).

## For the checker

- Every hand test of plan 8.7 (1, 2, 2b, 3-18) and 13.6 (19-25) is in `dist/HOW_TO_TEST.txt`: the chapter "Stickers:
  the hand tests to report (1-25)" lists each with steps and the expected result; 19-25 are the chapter "Working with
  Photoshop", word for word from 13.6.
- The installer script lists the licence folder (`[Files]` and the header comment); CI stages it; the smoke test checks
  it. Nothing was built or run for the installer.
- Greps (plan 8.6 item 7, `grep -I`, the fixture folder left out): "ArtCraft" only in `licenses/photocraft-MIT.txt`
  (and the notes); "Adobe" in user-facing text only in the trademark sentence (About > Licences, Help, README); no image
  asset added on the branch outside the test fixtures.
- No text says "Edit in PhotoCraft" where the editor is variable: the new Help says "Edit in <editor>" for the general
  case and names PhotoCraft only in the PhotoCraft-only guard paragraph; the scenario asserts the label from the
  default editor.
- Outputs: `target\sticker\p13\` (`flow_ABeautifulGame*`, `flow_DamagedHelmet*`, `flow_ole_*`, `flowlog_*`).
- Only cargo, Python and the WTE exe in `--screenshot` mode were started; no PhotoCraft, no Photoshop, no window took
  the focus.
