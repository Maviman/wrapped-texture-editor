# Check note: P7a - Painting

Checker, 2026-11-26, branch `stickers` at 61c78ca (the P7a build). Independent and adversarial check of the package
against STICKER_PLAN 7 "P7a" and the sections it cites (3.3 rows Brush, Eraser, Fill, Gradient, Eyedropper and the
layer-kind rule, 3.4, 3.5, 3.10, 4.9, 8.4), with the user's decisions and plan revision 3. Verdict: **pass after
fixes** (one major fixed with a test; the minors left are listed at the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,719 passed / 1 failed (WA20 only) / 63 ignored (core 1,080 / 1 / 39, app
  582 / 0 / 24, integration 15 + 9 + 8 + 25), as the builder said.
- `cargo clippy --workspace --all-targets`: 0 warnings. Release build OK, exe SHA-256 `90c4a3ec...30bdf51d` (the
  builder's).
- `perf_k7a_dab_region_and_restyle` (release, ignored test): entering stroke mode 27.0 ms (budget 50), worst dab plus
  region 0.39 ms (2), Preview restyle job 56.4 ms (60), end restyle job 95.7 ms (300). The builder's numbers hold.
- Scenarios with that exe, off-screen, no focus taken (`target\sticker\p7a-check\run\`, `WTE_FRAME_LOG=1`):
  `sticker_edit`, `sticker_pen`, `sticker_view`, `sticker_keys`, `sticker_styles`, `sticker_tray`, `sticker_controls`:
  all `"ok": true`.
- Frame spans (`tools/sticker/frame_spans.py`, limit 50 ms of 8.4): `sticker_edit` worst `sticker.canvas` 3.02 ms,
  `sticker.live` 1.19, `sticker.region` 0.49, `sticker.render` 2.75; `sticker_pen` 1.46 / 1.06 / 0.02 / 0.81.
- Shots looked at against their `expect`: `edit_brush` (red line across the logo's upper part, Layer 2 active, the
  sphere shows it, Sticker Brush options in the Tool tab), `edit_styled` (the line cut by the eraser and the second
  line, both with a white die-cut border and a soft shadow, no stroke-mode note, the sphere the same), `edit_final`
  (yellow outside the ring, the bordered lines, a red-to-blue band inside the selection, three layer rows),
  `pen_canvas` (a thin and a wide blue line both cut by a vertical gap; zoomed on the sphere: the same two lines and the
  gap; "Pen detected", Size pressure on). All match.

## Plan items, one by one

| Item (plan 7 "P7a" / cited section) | Found |
|---|---|
| Brush (B): primary colour on the active picture layer, or its mask with Paint on mask; `BrushSettings` size, hardness, flow, opacity, pen pressure | yes, tested (pressure width, mask) |
| Eraser (E): erases alpha, hides on the mask; as Brush | yes, in `brush.rs` (deviation 2, accepted) |
| Fill (G): the clicked region by Tolerance / Contiguous / Anti-alias, or the selection | yes, tested |
| Gradient (Shift+G): press to release, limited by the selection, one entry "Gradient"; Primary to Secondary / to Transparent / 2..=8 stops; Opacity; Reverse | yes, tested |
| Eyedropper (I): the sticker's colour (all layers) into the primary colour | yes (opaque; nothing on a transparent pixel), tested; works on text / shape layers too (checker test) |
| Layer-kind rule: paint tools on a text or shape layer show "Rasterize this layer to paint on it" with a Rasterize button, never a silent rasterize | yes: warning toast + header button + Tool-tab button (toasts have no buttons, deviation 5, accepted); text tested by the builder, shape by the checker |
| Masks: Add from selection, Paint on mask, Invert, Apply, Delete; one entry each | yes, tested |
| 3.4 selection limits brush, eraser, fill, gradient; static dashed outline; dropped when the artboard changes size | yes (P7a draws the bounds; the Marquee is P7b), tested |
| 3.5 dirty rectangles from cached layer rasters with the active layer unstyled; clipped layers above in the rectangle too | yes (`render_region`, core test over 44 regions incl. clipping, mask, adjustments, ellipse artboard) |
| 3.5 stroke mode on a styled layer: one full recomposite, the note "styles update when you lift" | yes, tested and in `sticker_edit` |
| 3.5 the model: a Preview job at most every 100 ms, not queued twice; Final at the end | yes; the Preview bake itself stays on the UI thread (deviation 3, accepted: the same bake as a style-slider drag, a reported span) |
| 3.5 at the end: the restyle on a job, the unstyled raster stays until it lands, never the stale styled one | yes, tested; **but** a dropped restyle kept running beside the next one (finding 1, fixed) |
| 3.5 a layer without styles: dirty rectangles only (counter) | yes, tested (`full_composites`, `canvas_uploads`) |
| 3.10 pen pressure per event, eraser end selects the sticker Eraser, barrel / Secondary pans | yes, tested and in `sticker_pen` |
| App tests of review A6 (canvas = below + unstyled + above in every dab, no style result before the end, at most one restyle running, after the end = a fresh full render, no full recomposite without styles) | yes, except "at most one restyle running": the counter for it did not exist and `most_previews_running` was set to 1 without looking (finding 1) |
| Scenarios `sticker_edit` and `sticker_pen` as listed | yes, all through pointer / pen steps on `Target::StickerCanvas` |
| ARCHITECTURE 8.6, 8.7, 8.11, 8.15, 9.2; HOW_TO_TEST | yes |

The checker list of the plan: each tool on each layer kind (picture, text, shape: Rasterize offered, nothing painted;
Eyedropper picks), strokes across the artboard edge (a moved layer reaching past the edge and a turned / scaled layer:
canvas = the unstyled composite after every dab, the finished picture after the end), a selection that excludes the
stroke (nothing painted, no entry, the canvas unchanged), a tool switch in mid-drag (`SetStickerTool` finishes the
stroke; builder test), Esc (`escape` cancels a drag first; builder test of `cancel_drag`), the shots and the frame log
(above). New tests `k7a_check_*` in `sticker/tools/tests.rs`.

Code read beyond the tests: the row copy of `resample::copy_exact` (taken only for rotation exactly 0 and both axes
positive; with the 1e-9 scale tolerance and the 1e-4 centre check every pixel rounds to `x + ox` as in the per-pixel
loop, so the bytes are the same; the identity sweep is the builder's); `render_region` against `render_with` (clip base
hidden, clip cut from the live content, the shape step per pixel); `apply_sticker_preview` (stroke open, the render
dropped right after the bake, `previewed` set through `rebake_sticker_users`, so an unchanged stroke still re-bakes
Final); the guard (`drag_moved` true once a stroke painted, Undo cancels it alone; Save / Export finish it); Rasterize
(text / shape masks are never drawn, so dropping them keeps the look; versions renumbered by `set_sticker`).

## Fixed

1. **Major: render jobs piled up, and the test meant to catch it could not.** `live::start` and `live::cancelled`
   drop the receiver of a running restyle / Preview job, but the thread runs on to the end of its render. The next
   stroke's end then started another restyle at once, so a quick series of strokes on a styled layer ran one whole
   render with its style buffers per stroke at the same time (8.4: up to 256 MB extra per `apply_styles` at 4096 px,
   about 1.1 s each; three or four strokes a second on a large sticker gave several at once). Plan 7 "P7a" asks for "at
   most one restyle job is running" as an app test; there was no restyle counter, and `most_previews_running` was set
   to `max(1)` on every start, so the assertion `<= 1` could not fail. Now every job thread is counted while it runs
   (`RenderJobs::preview_alive` / `canvas_alive`, a drop guard that also asks for a frame, panic included), a Preview
   or restyle job starts only when no thread of its kind runs (a restyle waits for the dropped one; the unstyled picture
   stays meanwhile), `clear` keeps the counts, and `most_previews_running` / `most_restyles_running` are real counts.
   Test `k7a_check_quick_strokes_on_a_styled_layer_run_one_restyle_at_a_time` holds the job threads of that editor
   (`RenderJobs::hold`, test build only) through five strokes on a 256-px styled disc: with the waits taken out it
   fails ("stroke 1: 2 restyle threads"); with them it passes and the canvas ends as a fresh full render.

ARCHITECTURE 8.15 (live canvas paragraph and the tests list) updated.

## Numbers after the fix

- `cargo test --workspace --no-fail-fast`: **1,723 passed / 1 failed (WA20 only) / 63 ignored** (core 1,080 / 1 / 39,
  app 586 / 0 / 24: the four checker tests, integration 15 + 9 + 8 + 25).
- `cargo clippy --workspace --all-targets`: 0 warnings. Release build OK, exe 30,410,752 bytes, SHA-256
  `f49e3f45bd2b2c9ccc2b2ff92376ea6a495a81c16745b47830dffd9d4afae18b`.
- With that exe (`target\sticker\p7a-check\run2\`): `sticker_edit`, `sticker_pen`, `sticker_view`, `sticker_styles`,
  `sticker_tray`, `sticker_controls`, `sticker_keys`, `smoke`: all `"ok": true`; `edit_styled` looked at again (same as
  before). Frame spans: `sticker_edit` worst `sticker.canvas` 3.93 ms, `sticker.live` 1.20, `sticker.region` 0.49,
  `sticker.render` 3.79; `sticker_pen` 1.52 / 1.14 / 0.03 / 0.81 (limit 50).
- The fix touches only when the app starts its render threads (no core, render, bake or file-format change), so the
  builder's identity sweep (21 / 21, exports and manifests identical to P0) stands.
- Programs started: cargo, Python and the WTE release exe with `--screenshot ... --scenario ...` only (off-screen, no
  focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No Cargo file changed; every edited
  file was LF and stays LF.

## Left open (minor; not fixed here)

- Plan 3.10's 24-pt hit targets: the new buttons and checkboxes use egui's 20-pt rows (open since the P6a check; one
  app-wide decision for the main session).
- A Preview job snapshots the sticker, so the next dab copies the painted buffer once (copy-on-write): at most ten
  copies a second during a stroke. On a 4096-px layer that is about 64 MB per copy on the UI thread (some ms per
  100 ms); the plan's 40-ms budget names only the one copy at the stroke's start. Inherent to a snapshot every 100 ms;
  the gate should measure a stroke on a 4096-px layer.
- The Preview bake of a wrapped sticker stamp runs on the UI thread every 100 ms during a stroke (deviation 3); not
  measured on a large model. `sticker_flow_path` on ABeautifulGame (P13) is where 8.4 measures it.
- An Undo during a gradient drag (nothing changed yet) cancels the drag and undoes the entry before it (P6a's rule
  for an unmoved drag); a drag that already shows its line could count as "moved".
- "Paint on mask" is one switch for the editor, not per layer: after a switch to another layer that has a mask it
  paints that mask (the checkbox shows it).
- The mask commands on a hidden layer are refused with "The layer is hidden" (they share the paint tools' rules).
- For a short time after an Undo that follows a stroke on a styled layer (until its restyle lands), the canvas still
  shows the stroke: the live canvas owns the texture until then.
- The shared Size tooltip says "in texels"; in the Sticker view it is sticker pixels.
