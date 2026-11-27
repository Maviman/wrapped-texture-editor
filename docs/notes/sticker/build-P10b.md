# Build note: P10b - Clipboard (STICKER_PLAN 7 "P10b", 6.3, the key part of 6.1; rev 3: 13.5, the P10b row of 13.9)

Builder, 2026-11-27, branch `stickers` (from 413cdea, the P10a check). Windows FFI (kind F). WTE now uses the Windows
clipboard for pictures, stickers and text: Copy / Cut / Paste / Paste at View Centre in the Edit menu and with
Ctrl+C / Ctrl+X / Ctrl+V (and Ctrl+Insert, Shift+Delete, Shift+Insert), text pasted into and copied out of egui's text
fields from and to other programs, and Help > Clipboard contents... Hand-written `raw-dylib` user32 calls, no new
crate, `Cargo.toml` / `Cargo.lock` unchanged. The user's workflow is not touched: nothing here reads or writes a link
file; a paste onto a linked sticker (a layer in the Sticker view) is an edit in WTE like any other (5.5 rules).

## What was built

| Item (plan 6.3, 6.1, 13.5, 7 "P10b") | Where | Notes |
|---|---|---|
| The clipboard behind a trait | new `ole/clipboard.rs` | `ClipFormat` (`WTE.Sticker.v1`, `PNG`, `CF_DIBV5`, `CF_DIB`, `CF_HDROP`, `CF_UNICODETEXT`), `PASTE_ORDER`, `trait Clipboard { write, read, format_names, is_memory }`, `MemClipboard` (named blocks; tests can add a foreign format by name), `clipboard_for(selftest)`; `real::RealClipboard` (`OpenClipboard` with the app window as owner, 5 tries 20 ms apart; `EmptyClipboard` + `SetClipboardData` per block, a refused block freed; `GetClipboardData` copied between `GlobalLock` / `GlobalUnlock`, never freed; `EnumClipboardFormats` + `GetClipboardFormatNameW`; always closed by a guard) is compiled only into a non-test Windows build and built only by `clipboard_for(false)`, like P9a's `RealLauncher`. `standard_format_name` / `unnamed_format` (13.5: standard formats by their constant names), `paste_item` (the read order of 6.3; a bad block falls through to the next format), `read_text` / `write_text`, `picture_entries` |
| The key subclass | new `ole/subclass.rs` | installed by `ole::install` (above the pen subclass, which `WteApp::new` installs first); `chord_of` / `route`: `WM_KEYDOWN` of V with Ctrl down and Shift and Alt up, or Insert with Shift down and Ctrl and Alt up, queues `OleEvent::Chord(Paste)`; every message is forwarded unchanged to the previous procedure and its result returned; `catch_unwind` around the chord; `WM_NCDESTROY` puts the previous procedure back |
| Queue and window | `ole/mod.rs` | `Chord { Copy, Cut, Paste }`, `OleEvent::Chord`, `take_chords` (only the chords; drop events stay for `dnd`), `window()` (the clipboard's owner, stored by `install`) |
| Copy / Cut / Paste | new `clip.rs` (+ `clip/tests.rs`) | `ClipAction { Copy, Cut, Paste, PasteAtViewCentre, ShowContents }`; `input_hook` / `route` (6.3 items 1-3), `after_ui` (item 4: `OutputCommand::CopyText`), copies taken on the UI thread and encoded on a worker (`ClipUi`, `poll`, `finish_copies`), `layer_pixels` (Sticker view: mask applied, cut to the selection), `paste_spot`, `nothing_text`, `show_contents` |
| Paste through the drop pipeline | `dnd.rs` | `start_items` (the worker part of `start_drop`, now shared), `is_placeable_path`; a `.wtes` with stamp styles gives them to the Stamp tool (`tool_styles_of`, as a library click does); `OleEvent::Chord` arriving late is left for the next frame's hook |
| App wiring | `app.rs`, `action.rs`, `dispatch.rs`, `state.rs`, `main.rs` | `raw_input_hook`: a self-test run discards the real OLE events, then queues the runner's paste chord, then `clip::input_hook`; `ui`: `clip::poll` after `dnd::handle`, `clip::after_ui` at the very end; `AppAction::Clip`; `AppState::clipboard`, `AppState::clip_ui`; the start log names the clipboard ("clipboard: in memory (nothing reaches Windows)" in a self-test run) |
| Menus | `panels/menu_bar.rs` | Edit: Cut (Ctrl+X; Sticker view only), Copy (Ctrl+C), Paste (Ctrl+V), Paste at View Centre; Help: Clipboard contents... |
| Shortcut tables | `shortcuts.rs` | the test `neither_table_binds_the_clipboard_chords` (Ctrl+C / X / V, Ctrl+ or Shift+Insert, Shift+Delete in neither table) |
| Self-test | `selftest/mod.rs` | `Step::Chord(Chord)`: Copy / Cut as `Event::Copy` / `Event::Cut` in a frame with Ctrl held, Paste as the subclass's chord handed over with its frame (`paste_with_next`, `take_chords`); `is_idle` waits for copies being encoded; unit test `chords_are_injected_as_real_use_produces_them` |
| Scenario | new `selftest/scenarios/sticker_clip.rs` | 69 scenarios; only `Step::Chord` for the chords (8.11 row) |
| Docs | ARCHITECTURE 1.3 (the "no OS clipboard" paragraph replaced), 1.12 status, 2.2, 3 (`raw-dylib` list, the eframe note), 8.1 (the clipboard facts of egui-winit 0.37 / egui 0.37 with file:line), 8.2, 8.3, 8.7, 8.10 (rule 8), 8.11 (`Step::Chord`, the `sticker_clip` row), 8.13 (pitfall 26), 8.17 (new part "Clipboard"), 9.2; HANDOFF 4.1, 4.2 (`WTE_OLE`), 5.4 and the branch table; `dist/HOW_TO_TEST.txt` chapter "Drag, drop, copy, paste" steps 11-19 | |

What Copy puts on the clipboard (6.3): a selected stamp (or the stamp of the active layer): its finished picture with
its styles (`Document::stamp_picture(id, true)`) as PNG + CF_DIBV5, plus WTE.Sticker.v1 (a `.wtes` with the stamp's
styles) for a sticker stamp and for an image stamp that has styles; no stamp: the active paint layer at texture size;
the Sticker view: the active picture layer's pixels with its mask applied and no layer styles, cut to the selection's
pixels. Paste reads WTE.Sticker.v1, PNG, CF_DIBV5, CF_DIB, then the first picture / PSD / `.wtes` file of CF_HDROP; a
DIB goes through the P10a codec with the 13.5 all-zero-alpha rule.

## Deviations (with reasons)

1. **`clip.rs` is a new module** (the plan lists `app.rs` for the hook logic and `tools/stamp_place.rs` for "paste as the
   tool source"). The routing is a pure function (`clip::route`) so the plan's tests (a)-(d) run on recorded inputs;
   `app.rs` only calls it. `tools/stamp_place.rs` needed no change: a paste goes through `dnd`'s "elsewhere" path,
   which already makes the picture (or the tray sticker) the Stamp tool's source and chooses the tool, and a click of
   the unchanged tool places it.
2. **A paste goes through the P10a drop pipeline** (`dnd::start_items`, the worker, `DropReady`, `dnd::apply`, one undo
   step) with a spot: `Elsewhere` (Paste), the view's centre (Paste at View Centre), the canvas's centre (Sticker
   view). Same decoding, same table, same undo rule as a drop; no second copy of that code. A side effect: a bad DIB on
   the clipboard is reported with the drop's sentence ("Cannot use the dropped picture: ...").
3. **Ctrl+Shift+V is not a paste.** The plan says "V with Ctrl down and Alt up"; the subclass also wants Shift up,
   because egui-winit's `is_paste_command` uses `command_only()`: with Shift it sends a key event and no paste, so the
   two would disagree. Shift+Insert wants Ctrl up (Ctrl+Shift+Insert is egui-winit's Copy).
4. **WTE.Sticker.v1 only where it keeps something editable.** A sticker stamp, and an image stamp with styles (as a
   one-layer sticker of its picture, so the styles stay editable); an image stamp without styles and a text stamp
   offer the picture only, so a paste in WTE gives back the same kind of stamp (an image stamp, not a sticker).
5. **The copied stamp's styles go to the Stamp tool on paste** (also for a dropped `.wtes`, `dnd::tool_styles_of`), as
   a click on a library entry does (P8): "copy a stamp, paste, click" places a stamp that looks the same.
6. **Copy and Cut in the Sticker view take a picture layer only** (text and shape layers: "Copy takes a picture layer's
   pixels: rasterize this layer first"). A locked or hidden picture layer can be copied (Cut's deletion keeps the paint
   rules: it is `StickerAction::DeleteSelected`, with its own entry "Delete Selected Pixels" and the shared-sticker
   question). Cut needs a selection ("Select an area first: Cut takes the selected pixels"); outside the Sticker view Cut
   only says "Cut takes selected pixels in the Sticker view; use Copy here" (the plan: Cut in Sticker mode only).
7. **The layer's mask is applied to a Sticker-view copy** ("the pixels being edited" as they show, without the layer
   styles), so pasting elsewhere gives what the layer shows.
8. **Encoding off the UI thread.** The pixels are taken on the UI thread (shared `Arc`s; a texture layer is not copied);
   a worker encodes PNG (`Fast`) + DIB and writes the `.wtes`; `clip::poll` writes the clipboard on the UI thread (the
   owner window's thread). A Paste first waits for a copy still being encoded, so Copy then Paste is always in order.
   The plan does not say where the encoding runs; a 4096 x 4096 layer would otherwise stall a frame for about a second
   (the reason of review A23 for drag out).
9. **Paste at View Centre in Split** uses the 3D half (the plan says "the active view"; WTE has no notion of an active
   view half). In the Sticker view both pastes add a layer at the canvas centre. Preview, or no laid-out view: a plain
   Paste.
10. **Paste as New Paint Layer** (6.3's list) is P12's (5.9 and the P12 file list); not built here.
11. **Text focus** is `text_edit_focused()` (as the shortcut table uses; `wants_keyboard_input` of the plan is true for
    any focused button) **or a selected label text** (`LabelSelectionState`), so Ctrl+C on selected label text still
    copies that text.
12. **While a dialog is open or a modal job runs** no `ClipAction` is queued; text fields (also in dialogs) still paste
    the clipboard's text.
13. **Help > Clipboard contents...** is a plain `Dialog::Message` (title "Clipboard contents"), not a new dialog kind.
14. **`Step::Chord(Paste)`** is handed to the app after `ole::discard` (`Runner::take_chords`): the runner's own chord
    would otherwise be dropped with the real ones in the next frame's discard. It goes with its injected frame (Ctrl
    held, no events), exactly where a real chord would arrive.

## Numbers

- `cargo test --workspace --no-fail-fast`: **1,874 passed, 1 failed (WA20, the known one), 65 ignored**
  (`target\sticker\p10b	est2.log`; `wte_app` 715 / 0 / 24 (P10a check: 694), `wte_core` lib 1,102 / 1 / 41,
  integration 15 + 9 + 8 + 25). New tests: `ole::clipboard::tests` (5), `ole::subclass::tests` (3), `clip::tests` (11),
  `shortcuts::tests::neither_table_binds_the_clipboard_chords`, `selftest::tests::chords_are_injected_as_real_use_produces_them`;
  the scenario count test now says 69.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 32,939,520 bytes, SHA-256
  `7b0dc114bb33fcebfe45997b624c8dd275c4e80c68ef5777e4168f4014ad2691` (copy `target\sticker\p10b\wte_p10b.exe`).
- Scenarios (`tools/objtabs/sweep.sh`, off-screen, no focus taken), all `"ok": true`: with the final exe
  `sticker_clip` (2-4 s, 172 frames), `sticker_drop`, `smoke` (`target\sticker\p10binal\`), and the same three with
  `WTE_OLE=1 WTE_PEN_SUBCLASS=1` (`final_ole\`); with the exe before the last refactor (a copied layer buffer shared
  instead of cloned on the worker; no other change) `sticker_clip`, `sticker_drop`, `sticker_view`, `sticker_link`,
  `smoke` plain (`runs2\`) and with `WTE_OLE=1` (`runs_ole\`), and `smoke`, `default`, `pen`, `companion`,
  `sticker_clip` with `WTE_PEN_SUBCLASS=1 WTE_OLE=1` (`runs_sub\`). Each `WTE_OLE=1` log says "ole: the key subclass is
  installed ..." after "pen input: window procedure subclassed ..." and "ole: WTE's drop target is registered", and
  every self-test log says "clipboard: in memory (nothing reaches Windows)". The `sticker_clip` shots with and without
  `WTE_OLE=1 WTE_PEN_SUBCLASS=1` are pixel-identical except the status bar of `clip_text` (the font-samples job's
  progress, timing).
- Shots of `sticker_clip`, each looked at against its `expect`: `clip_paste_source` (the disc sticker with its thin dark
  Outline on the upper left, still selected; the Stamp tool with "Sticker: Badge" and Style Outline; its yellow
  placement outline under the pointer on the lower right; matches), `clip_view_centre` (three disc stickers, the third
  in the middle of the view with the gizmo, three "Badge" rows; matches), `clip_sticker_layer` (the Sticker view: the
  disc with the cut square, the pasted 24 x 24 square at X 20 Y 20 covering it but for a strip along the top, two layers
  with "Pasted picture" active, the strip visible on the sphere; matches after the expect text was made exact),
  `clip_text` (the Text tool's field with "Copied out of WTE" highlighted and its yellow preview on the sphere; matches).
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\p10b\sweep\`): 21 / 21 `"ok": true`.
  `compare_baseline.py` against the P0 baseline: 214 the same, the two known `side_tabs` small shots (five tabs since
  P6b) and `shot_tab_sticker.png` (new since P6b), as in P10a; against P10a's sweep (`target\sticker\p10a\sweep\`):
  **217 / 217 the same**. No old project or flow changed.
- Programs started: cargo, Python (`-E -P`), and the WTE release exe only with `--screenshot ... --scenario ...`
  (off-screen, no focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No test and no
  self-test run opens the Windows clipboard (the real clipboard is not compiled into a test build; a self-test run gets
  `MemClipboard`); the tests' OS calls are `CallWindowProcW` (to a fake previous procedure, no window) and those of P10a.

## For the checker

- `unsafe` to read: `ole/clipboard.rs` `real` (open / close guard, `EmptyClipboard`, `SetClipboardData` ownership and the
  free of a refused block, `GetClipboardData` read between lock and unlock, `GetClipboardFormatNameW` buffer),
  `ole/subclass.rs` (`install`, `wndproc`: every message reaches the previous procedure; `WM_NCDESTROY`).
- The grep the plan asks for: the only `OpenClipboard` / `SetClipboardData` / `GetClipboardData` calls are in
  `ole/clipboard.rs` `mod real`, under `#[cfg(all(windows, not(test)))]`, built only by `clipboard_for(false)`
  (`AppState::new` with `selftest == false`, i.e. `WteApp::new` of an interactive run).
- Hand tests owed (8.7 hand tests 12, 13, 24; HOW_TO_TEST "Drag, drop, copy, paste" 11-19): the real clipboard with
  Paint, PhotoCraft, Photoshop, a browser, Explorer and Notepad, the transparency in each direction, AltGr+V.
