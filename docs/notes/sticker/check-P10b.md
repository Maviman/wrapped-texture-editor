# Check note: P10b - the clipboard

Checker, 2026-11-27, branch `stickers` at 909c0fb (the P10b build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P10b", 6.1 (the subclass part), 6.3, 13.5 / 13.9 (rev 3 row) and the user's decisions.
Verdict: **pass after fixes** (no blocker; one major and one minor fixed with tests, the rest noted below).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,874 passed / 1 failed (WA20 only) / 65 ignored (app 715 / 0 / 24, core
  1,102 / 1 / 41, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\check-p10b\test.log`).
- Clippy 0 warnings; release build OK.
- Release exe, off-screen (`--screenshot ... --scenario ...` only; no focus taken), `target\sticker\check-p10b\runs\`:
  `sticker_clip`, `sticker_drop`, `sticker_view`, `smoke` plain; with `WTE_PEN_SUBCLASS=1 WTE_OLE=1`: `sticker_clip`,
  `smoke`, `pen`, `companion`, `default` (the plan's checker item). All `"ok": true`. The `WTE_OLE=1` logs show the pen
  subclass first, then "ole: the key subclass is installed", and "clipboard: in memory (nothing reaches Windows)".

Every `sticker_clip` shot looked at:

| Shot | Expect text | Seen |
|---|---|---|
| `clip_paste_source` | Badge with dark outline upper left, Stamp tool "Sticker: Badge", Style Outline, yellow outline lower right, nothing new placed | matches (Undo: Add Sticker) |
| `clip_view_centre` | three discs, the middle one with the Transform gizmo, three Badge rows | matches; the status bar said "Undo: Drop" - see finding 2 |
| `clip_sticker_layer` | Sticker view, cut hole, pasted 24 x 24 layer at X 20 Y 20 with a transparent strip on top, "Pasted picture" on top and active | matches |
| `clip_text` | Text tool, field shows "Copied out of WTE" selected, yellow preview on the sphere | matches |

## Plan 7 "P10b" item by item

| Item | Found |
|---|---|
| Files `ole/{subclass,clipboard}.rs`, `app.rs` (`raw_input_hook`, end of `ui`), `menu_bar.rs`, `selftest/mod.rs` (`Step::Chord`) | present; `tools/stamp_place.rs` unchanged (a paste uses dnd's Elsewhere path, which already makes the Stamp tool's source: accepted) |
| 6.3 Edit menu: Cut (Sticker mode only), Copy, Paste, Paste at View Centre, Copy / Paste Style | present; **Paste as New Paint Layer** left to P12, whose file list names it (accepted) |
| 6.3 keys 1-5 (Copy / Cut events, subclass paste chord, text paste, `CopyText` at the end of `ui`, `Step::Chord` only) | yes; text focus = `text_edit_focused()` or a label selection instead of `wants_keyboard_input` (documented deviation, correct: the latter is true for any focused button) |
| Copy table (stamp with styles + `WTE.Sticker.v1`; paint layer at texture size; Sticker view selection without layer styles) | yes |
| Paste table (order `WTE.Sticker.v1`, PNG, DIBV5, DIB, HDROP; Stamp tool source, nothing placed until a click; View Centre; Sticker view new layer) | yes |
| `OpenClipboard` 5 x 20 ms, failure toast; `trait Clipboard`, real + in-memory | yes |
| rev 3: all-zero alpha = opaque; `format_names()`; log line; "Nothing WTE can paste" text; Help > Clipboard contents... | yes |
| ARCHITECTURE 1.3 (paragraph replaced), 8.1, 8.3, 8.10, 8.17, 9.2 | yes (also 2.2, 3, 8.2, 8.7, 8.11, 8.13) |
| Tests: subclass routing on a fake previous procedure (Ctrl+V, AltGr, Shift+Insert, other messages unchanged); (a)-(d); `CopyText`; neither table binds the chords; in-memory round trips; paste = source + click; View Centre places one | yes, all |
| Scenario `sticker_clip` | yes (Paste at View Centre via `Step::Do` of the menu action, which is how it is reached: no chord exists for it) |
| Checker: no test opens the real clipboard | confirmed: `RealClipboard` is compiled only under `cfg(all(windows, not(test)))`, built only by `clipboard_for(false)` (`AppState::new` with `selftest = false`, i.e. no `--screenshot`); no other `OpenClipboard` / `SetClipboardData` call in the tree; no `SendInput` / `keybd_event` anywhere |
| Checker: the subclass never returns without calling the previous procedure | confirmed: `route` always ends in `forward`; `wndproc` forwards with `CallWindowProcW` (or `DefWindowProcW` if no previous procedure is known), restores it at `WM_NCDESTROY` before forwarding |

## Findings

1. **Major, fixed: a slower earlier copy overwrote a later one; a held Ctrl+C encoded many pictures at once.**
   `ClipUi` kept a list of copy jobs and `poll` wrote each one when it finished, in the order the workers finished.
   Copy a 4096 x 4096 paint layer (about a second to encode PNG + DIBV5), then select a stamp and Copy again: the
   stamp's copy is written first, the layer's a moment later, and the clipboard ends up holding the older picture,
   which the user then pastes into Photoshop. A held Ctrl+C (egui-winit repeats `Event::Copy`) started one encoder
   thread per repeat, each allocating the PNG and a 64 MiB DIB at the same time. Fix (`clip.rs`): one `CopyJob` at a
   time; a new copy supersedes the running one (its receiver is dropped, so its result is never written, and an
   `AtomicBool` stops its worker before the next encoding step). `finish_copies` waits for the latest only. Test
   `clip::tests::check_p10b_a_later_copy_wins_over_a_slower_earlier_one` (a held first worker released after the
   second copy was written: the clipboard keeps the second, one write in all). ARCHITECTURE 8.17 updated.
2. **Minor, fixed: Paste at View Centre was labelled "Drop" in the history** ("Undo: Drop" in `clip_view_centre`),
   because a paste goes through the drop pipeline, whose edit group label was the literal "Drop". `dnd::start_items`
   now takes the label (`dnd::DROP_LABEL` for drops, `clip::PASTE_LABEL` = "Paste" for pastes; `Prepared::label`).
   Asserted in `paste_at_view_centre_places_one_stamp` (a PNG: one edit, its own label "Add Stamp"; a
   `WTE.Sticker.v1`: tray sticker + stamp in one step labelled "Paste") and in `sticker_clip` step (4), whose shot now
   reads "Undo: Paste". A paste that makes a single edit keeps that edit's own label, as drops do.

Noted, not changed:

- 6.3 item 3 as written: with the chord in a text field and no text on the Windows clipboard (a picture is there),
  egui-winit's in-app string is pasted - text that was copied in WTE earlier and is no longer on the clipboard. This is
  the plan's rule ("egui-winit's own event is put back"); harmless but surprising. Left for the user / P13.
- Ctrl+V held repeats the paste (one tray sticker or layer per repeat), as in Photoshop; not changed.
- `RealClipboard` opens and closes the clipboard once per format read during one Paste (another program could change
  it between reads); reads `GlobalSize` bytes (may be a few bytes longer than written: PNG and zip readers tolerate a
  tail). Hand tests 12, 13 and 24 cover the real clipboard; nothing automated can.
- `Document::stamp_picture(id, true)` (style rendering) runs on the UI thread at Copy; only the encoding is on the
  worker. Fine for stickers up to the 4096 limit in practice; noted for the gate's timing pass.
- Cut on a locked layer copies and then the deletion is refused by `DeleteSelected`'s own guard (the clipboard holds
  the copy, nothing is cut). Acceptable.

## After the fixes

- `cargo test --workspace --no-fail-fast`: app 716 / 0 / 24 (one new test), core 1,102 / 1 (WA20) / 41, integration
  15 + 9 + 8 + 25 (`test2.log` with the label test's first form, `app_tests3.log` final). Clippy 0 warnings
  (`clippy3.log`). Release build OK: 32,949,248 bytes, SHA-256 4e91d5ffea907a20...
- Off-screen with that exe (`target\sticker\check-p10buns_fixed\`): `sticker_clip`, `sticker_drop`, `sticker_view`,
  `smoke` plain and `sticker_clip`, `smoke`, `pen`, `companion` with `WTE_PEN_SUBCLASS=1 WTE_OLE=1`: all `"ok": true`.
  The `sticker_drop`, `sticker_view` and `smoke` shots are byte-identical to the run before the fixes (drops keep
  "Drop"); `clip_view_centre` now shows "Undo: Paste". No core code changed, so the P0 identity sweep is unaffected.
- Only cargo, Python and the WTE exe in `--screenshot` mode were started; no editor, no real clipboard.
