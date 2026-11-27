# Check note: P7b - Selections and cut-out

Checker, 2026-11-26, branch `stickers` at 94086f9 (the P7b build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P7b" and the sections it cites (3.3 rows Marquee, Lasso, Magic Wand, Magic Eraser, Move with a
selection, the keys table and the Commands list; 3.4; 4.4; 4.5; 4.9; 8.4), with the user's decisions and plan revision
3. Verdict: **pass after fixes** (one major fixed with a test, two minors fixed with tests; the minors left are listed
at the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,742 passed / 1 failed (WA20 only) / 64 ignored (core 1,084 / 1 / 40, app
  601 / 0 / 24, integration 15 + 9 + 8 + 25), as the builder said (`target\sticker\check-p7b\test.log`).
- The builder's shots in `target\sticker\p7b\run3\` and the code paths behind them; then everything below again with
  the fixed code.

## Plan items, one by one

| Item (plan 7 "P7b" / cited section) | Found |
|---|---|
| Marquee (M): rectangle or ellipse; Shift adds, Alt subtracts; Feather; fixed aspect | yes (both keys = intersect, plus the New / Add / Subtract / Intersect row for a tablet), tested |
| Lasso (L): polygon (clicks; double-click or Enter closes), freehand drag; Feather | yes (also a click on the first point, Close lasso), tested |
| Magic Wand (W): Tolerance 32, Contiguous, Anti-alias, Sample active layer / all layers | yes (all layers = the unstyled composite, 4.4), tested |
| Magic Eraser (Shift+E): Tolerance, Contiguous, Anti-alias, Into mask | yes, tested (one entry, promotion, undo, a mask added when none) |
| Move with a selection on a picture layer: Layer via Cut + move, one entry "Move Selection" | yes: `set_sticker` Preview calls fold into one entry (deviation 7, accepted: the cut changes two layers); Esc / Undo mid-drag take everything back, tested |
| 3.4: limits brush, eraser, fill, magic eraser, delete, destructive commands; static dashed outline; dropped when the artboard changes size | yes (outline of merged runs, cached per `Arc`; bounds fallback over 50,000 runs) |
| Keys: Ctrl+A / Ctrl+D / Ctrl+Shift+I; Delete / Backspace clear the selected pixels and never the stamp; Esc order; Enter closes a lasso | yes; the colliding-chords test has the three chords; Delete / Backspace through the real key table: checker test (below) |
| Cut-out: Remove Background (Tool tab options, one click), Color to Alpha (eyedropper / White / Black, live preview), Defringe 1..=10, Remove White / Black Matte | yes, one entry each, tested; the eyedropper is a canvas click while the popover is open (deviation 4, accepted) |
| Filter: Blur 0.5..=64 with live preview, Sharpen 0..=300 % / 0.5..=16; one entry each; mixed by the selection | yes, tested (right half byte-identical in the scenario); over 4096 px a side refused (deviation 11, accepted) |
| Selection: All, None, Invert, Grow / Shrink, Feather, Color Range (live outline), Delete, Crop to Selection, Layer via Copy, Layer via Cut, From Layer Alpha | yes, tested |
| Layer-kind rule for Color to Alpha, Defringe, Blur, Sharpen (Rasterize offer, never silent) | yes (`paint_target` with the Brush's rule), tested on a text layer |
| The Sticker menu, the commands also in the Tool tab | yes (menu only in the view, deviation 12, accepted: identity shots) |
| ARCHITECTURE 8.6, 8.7, 8.11, 8.15, 9.2; HOW_TO_TEST | yes ("Stickers, part 3") |
| Scenario `sticker_cutout` with every step and assert of the plan (corner alpha 0, object alpha 255, halo bound, die-cut shot) | yes; shots looked at (below) |
| Checker: Delete / Backspace with and without a selection, with a stamp selected | done: `k7b_check_delete_and_backspace_keys_with_a_stamp_selected` (both keys through `shortcuts::handle` in the view) |
| 8.4: Preview-quality render during a slider drag 33 ms; frame limit 50 ms | **missed** by the popover preview: finding 1 |

## Findings

1. **Major, fixed: the popover's live preview ran on the UI thread** (`popover::frame`: the command's maths plus
   `sticker_picture_with`, a full styled render at scale 1 without caches). The builder measured 255 ms per update for
   a 2048-px sticker with a ClassicSticker layer (Blur 16); 8.4 allows 33 ms for a preview render during a slider drag
   and 50 ms per frame, and a miss over 50 % is a gate fail. Every change of a number, a White / Black click or a
   canvas pick froze the window that long (throttled to one per 100 ms while a slider is held, so a drag ran at about
   3 frames a second); a 4096-px sticker would freeze it for over a second. Fix: `filters::compute` is split into
   `prepare` (UI thread: the target, `Arc`s of the pixels, mask and selection) and `Prepared::math` (any thread); core
   `Document::sticker_picture_with` into `sticker_picture_with_job` (UI thread: the render input, `Arc`s) and
   `StickerPictureWith::run` (any thread, `Send`; `sticker_picture_with` is now the two halves, so its tests cover
   both). `popover::frame` starts one job thread (`StickerUi::popover_job`, counted in `popover_alive` until the thread
   ends, a dropped one included, so jobs never pile up), takes its picture when it lands for the key still shown, drops
   one for older numbers and draws the newer numbers next; the canvas keeps the last preview meanwhile. Apply still runs
   the command itself (one entry), so the result does not depend on the job. `popover::pending` is part of
   `live::pending` (the self-test idle check); the scenario waits with `WaitIdle` instead of three frames. The
   selection previews (Color Range, Grow / Shrink / Feather) stay in the frame: they are the wand's work and a distance
   transform, within their own 8.4 budgets. Test `k7b_check_the_popover_preview_runs_on_a_job` (held job threads: the
   frame that starts the job leaves the canvas untouched; no second job beside a running one; the stale result
   dropped; a closed popover's job never lands; the landed picture equals the result of the command). The
   `sticker.popover` span in `sticker_cutout` went from 1.96 ms to 0.18 ms worst.
2. **Minor, fixed: an open polygon lasso outlived a tool change.** It stayed drawn under the next tool, and Enter there
   closed it into a selection. `sticker::tool_changing` (called by `SetStickerTool`) drops it unless the Lasso is chosen
   again. Test `k7b_check_another_tool_drops_an_open_lasso`.
3. **Minor (test gap), closed: Delete / Backspace were tested only through `run_in(DeleteKey)`.** The checker test
   presses both keys through the real shortcut table in the Sticker view, with and without a selection, with the stamp
   selected: the selected pixels go (one entry each), the stamp, its selection and the texture layers stay.

Looked at and found right: Move Selection's crop of the cut piece on a plain-moved layer (no rotation centre involved
at scale 1), the new layer's mask cropped with it and the source keeping its own; the selection shifted by the same
whole-pixel offset; `version_new_buffers` re-versions every new `Owned { version: 0 }` buffer of a `set_sticker`, so no
render cache key is reused; `put` makes no entry for an unchanged buffer; the shared-picture question clears the canvas
drag, so a Move Selection drag never commits after the dialog; Esc order (drag, popover, lasso, then the rest); the
outline's edge runs at the bounds (exclusive `x1` / `y1`); the identity of old projects (below).

## Re-run with the fixes

- `cargo test --workspace --no-fail-fast`: **1,745 passed / 1 failed (WA20 only) / 64 ignored** (core 1,084 / 1 / 40,
  app 604 / 0 / 24, integration 15 + 9 + 8 + 25): +3 checker tests (`target\sticker\check-p7b\test2.log`).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 31,111,168 bytes, SHA-256
  `e1c8f87cfebb435c7db06508243023b930a9d61e7ebbfa066b0f0a1b901d5426`.
- Scenarios with that exe, off-screen, no focus taken, `WTE_FRAME_LOG=1` (`target\sticker\check-p7b\run\`):
  `sticker_cutout` (302 frames), `sticker_view`, `sticker_keys`, `sticker_styles`, `sticker_tray`, `sticker_controls`,
  `sticker_edit`, `sticker_pen`, `smoke`: all `"ok": true`.
- Frame spans (`tools/sticker/frame_spans.py`, limit 50 ms): `sticker_cutout` worst `sticker.canvas` 15.25 ms (a full
  render after an undo), `sticker.render` 15.09, `sticker.outline` 0.80, `sticker.popover` 0.18; `sticker_edit` 6.79 /
  live 2.14 / region 0.46; `sticker_view` 3.25.
- Shots looked at against their `expect`: `cutout_delete` (the corner checkerboard with the dashed outline, the
  sphere's stamp missing the corner, the Marquee options), `cutout_removed` (the disc alone, no white rim; the sphere
  without the white square), `cutout_color_to_alpha` (the window at the canvas's top-left with White / Black / Primary,
  Apply, Cancel; the preview drawn by the job: the second layer's white gone, the model still unchanged),
  `cutout_die_cut` (the sphere with the die-cut disc, white border and shadow, the moved piece and the soft left edge;
  three layer rows), P6b's `controls_view` (Move options, X / Y, Mask, Layer styles). All match.
- Identity sweep (8.5): 21 / 21 ok, nothing new differs (last section).
- Programs started: cargo, Python and the WTE release exe (only with `--screenshot ... --scenario ...`, off-screen).
  Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No new crate; `Cargo.toml` / `Cargo.lock`
  unchanged; every edited file was LF and stays LF.

## Left open (minors, not fixed)

- A popover stays open while another tool paints: a brush stroke draws its dirty rectangles over the preview picture
  until the stroke ends and the preview is drawn again (the document is right; only the canvas mixes the two for the
  stroke's length). Photoshop's filter dialogs are modal; closing the popover at a stroke's start would be the simple
  rule.
- The selection previews (Color Range, Grow / Shrink / Feather) and the outline of a new selection are computed in the
  frame: at 2048 px about the wand's 60 ms (8.4) per change, more at 4096 px. Within the wand / EDT budgets, but over
  the 50 ms frame limit for big stickers; the same job pattern would fit if the gate wants it.
- As the builder listed: the 24-pt tablet hit targets (open since P6a), Grow / Shrink at the 128 threshold, the bounds
  fallback of the outline, "active layer" sampling without opacity / blend.
- An "Into mask" Magic Eraser click whose region is empty on a layer without a mask still adds an all-white mask (one
  entry that changes nothing visible).

## Identity sweep result

`tools/objtabs/sweep.sh` with the fixed exe and `tools/sticker/jobs_identity.txt` into `target\sticker\check-p7b\sweep\`:
21 / 21 `"ok": true`. `compare_baseline.py` against `target\sticker\baseline\sweep`: 214 of 216 files the same, plus the
known NOTE (`side_tabs/shot_tab_sticker.png`, only in the new run); the two known whole-picture differences
`side_tabs/shot_small_640.png` / `shot_small_900.png` (the mode strip's Sticker button and the two-row tab strip,
outside the compared area of 8.5) are byte-identical to the P7a and P7b sweeps, and so is `shot_tab_sticker.png`.
Exports, saved layer PNGs and manifests: all the same (`target\sticker\check-p7b\compare.txt`).
