# Build note: P7a - Painting (STICKER_PLAN 7 "P7a", 3.3, 3.5, 3.10, 8.4)

Builder, 2026-11-26, branch `stickers` (from bb5ceed, the P6b check; plan revision 3). Kind U, with core additions.
The Sticker view can now be painted in: the sticker **Brush** and **Eraser** (the paint tools' size, hardness, opacity,
flow, spacing and pen-pressure settings; the pen's pressure and eraser end), **Fill**, **Gradient** and **Eyedropper**,
**layer masks** (Add / Add from selection, Invert, Apply, Delete, "Paint on mask"), and **Rasterize** of text and shape
layers (the paint tools offer it and never paint them). The canvas draws only what a dab changed; a layer with a style
is shown unstyled during the stroke ("styles update when you lift") and restyled on a job after it; the model follows
the stroke through a Preview job at most every 100 ms.

## What was built

| Item (plan P7a) | Where | Notes |
|---|---|---|
| Brush, Eraser (3.3) | `wte_app/src/sticker/tools/brush.rs` | press opens `sticker_stroke_begin` (image stamps promoted inside it), every pointer event is one `sticker_stroke_dab` with its pressure, the release ends it: one entry "Sticker Brush" / "Sticker Eraser"; `BrushSettings` of the Brush / the Eraser, size = diameter in layer-space px; the primary colour (a mask: its grey) |
| Fill, Gradient, Eyedropper (3.3) | `sticker/tools/{fill,gradient,eyedropper}.rs` | Fill: the colour region under the click (Tolerance, Contiguous, Anti-alias) or the selection; Gradient: primary to secondary / to transparent / custom stops (2..=8), Opacity, Reverse, limited by the selection; both one `replace_sticker_pixels` entry ("Fill", "Gradient"); Eyedropper: the finished sticker's colour (opaque), nothing on a transparent point |
| Layer-kind rules (3.3) | `sticker/tools/mod.rs` `paint_target` | picture layer, unlocked, shown; text / shape: status line + warning toast "Rasterize this layer to paint on it" + the header's Rasterize button; the E6 question before a stroke on a shared sticker; Paint on mask; the selection seen from the layer (`selection_for`) |
| Masks (3.2 "Mask") | `sticker/commands.rs`, Tool tab `layers_ui::mask_block` | Add (from the selection when there is one; turns Paint on mask on), Invert (tile patch), Apply, Delete; one entry each; Paint on mask needs a mask |
| Rasterize (pulled from P8) | core `rasterize_sticker_layer`, `commands::rasterize`, `StickerAction::Rasterize` | one entry "Rasterize Sticker Layer", the same look; also Layer > Rasterize in the view |
| Selection command | `StickerAction::SelectRect` / `SelectNone`, canvas outline | the Marquee of P7b makes the same selection; the scenario's "marquee made by a command"; dropped when the artboard changes size (3.4) |
| Stroke-mode canvas (3.5, review A6) | new `sticker/live.rs`, `canvas.rs` | `LiveStroke`, dirty rectangles via `Document::sticker_canvas_region` + `set_partial`; stroke mode (one full recomposite to enter it, header note); the restyle job at the end (shown only when the sticker is still the value it drew); an unstyled layer never gets a full recomposite; `LiveCounters` |
| The model during a stroke (3.5) | `live::preview`, core `sticker_preview_job` / `apply_sticker_preview` | a Preview job at most every 100 ms after new paint, never two at once; the Final at the end (deferred bake jobs) |
| Pen (3.10) | `canvas::handle_input` | every event's pressure from `PenFrame::cursor` (as `route_pointer`); the eraser end selects the sticker Eraser (P6a) and the self-test probe counts it; the barrel button / companion finger pans |
| Guard rules | `sticker/mod.rs` `cancel_drag` / `finish_drag` / `drag_moved`, `dispatch.rs` | `CanvasDrag::{Stroke, Gradient, Pick}`; Undo during a stroke takes back the stroke only; Save keeps it; other commands and Esc cancel it; a tool switch and leaving the view keep it |
| Tool tab / Brush tab (3.2) | `sticker/layers_ui.rs`, `panels/brush_tab.rs`, `panels/tool_options.rs` | the paint tools' options; Mask block / Rasterize; Brush tab in the view: the tip line, Type greyed (Circular) with the hint; `[` `]` act on the sticker tool's settings (`dispatch::active_brush`) |
| Core | new `doc_sticker_paint.rs`; `sticker/render.rs` `render_region`; `sticker/select.rs` `selection_in_content`; `sticker/resample.rs` row copy; `counters::region_renders` | see deviations 6-8 |
| Self-test | `selftest/mod.rs` (`is_idle` waits for `live::pending`; the pen probe counts the sticker Eraser), new `selftest/scenarios/sticker_paint.rs` | scenarios `sticker_edit`, `sticker_pen` (NAMES 61) |
| Tool | new `tools/sticker/frame_spans.py` | the worst `sticker.*` / `link.*` / `ole.*` spans of a frame log (plan 8.4 names it; no package had built it) |
| Docs | ARCHITECTURE 1.12, 2.1, 5.20 ("Painting"), 5.27, 6.10 ("Region render"), 6.11, 8.6, 8.7, 8.11, 8.15 ("Stickers P7a"), 9.1, 9.2; HANDOFF 4.1 (61 scenarios), 5.4; `dist/HOW_TO_TEST.txt` "Stickers, part 2: painting" | |

Tests added: core 10 (`k7a_*` in `doc_sticker_paint_tests.rs`: region = whole render in 44 regions of a sticker with
every layer kind, clipping, blends, mask, adjustment, ellipse artboard; trivial path; the region read-out during a
stroke without a whole render; the canvas job's snapshot and the one buffer copy after it; the Preview job applied only
during the stroke, no entry, no stale render left in the cache; `sticker_color_at`; layer read-outs; Rasterize; the
selection seen from a moved / turned layer; `resample::k7a_a_plain_move_copies_rows_with_the_per_pixel_bytes`) and the
ignored `perf_k7a_dab_region_and_restyle`. App 11 (`sticker/tools/tests.rs`: the brush stroke on an image stamp; **stroke
mode** - after every dab the canvas equals "below + unstyled layer + above", no restyle during the stroke, at most one
Preview job, the finished picture after the restyle, and for an unstyled layer no full recomposite and no upload; pressure
width and the eraser; Esc / Save / Undo / tool switch; locked / hidden / text layers with the Rasterize offer for each
paint tool; the E6 question before a stroke; Fill; Gradient with Reverse and the stop editor; Eyedropper; masks and Paint
on mask; the selection dropped with the artboard); `sticker::tests::a_locked_layer_and_an_unbuilt_tool_do_nothing` now
uses the Marquee as the unbuilt tool.

## Deviations from the plan, with reasons

1. **The live canvas is `sticker/live.rs`**, not `canvas.rs` (the plan's file for "dirty rectangles, stroke mode, the
   restyle job"): canvas.rs keeps the drawing and the routing; live.rs owns the texture updates and the jobs.
2. **The Eraser is in `tools/brush.rs`** (the plan lists `eraser.rs`): it differs from the Brush only by the mode and its
   settings.
3. **The model's Preview "job" renders off the UI thread; the bake itself stays on the UI thread.** The job renders the
   sticker with its layer styles at the users' Preview scale (and its mips); `apply_sticker_preview` then runs the
   existing synchronous Preview bake (`rebake_sticker_users(id, Preview)`: stamp styles at the Preview working size and
   the projection) from that picture. A bake job for a sticker Preview would need a new kind of stamp job and its own
   freshness rules; plan 8.4 limits the restyle (off the UI thread: it is) and only reports the preview-bake spans.
   Measured: worst `sticker.live` 1.25 ms per frame on the sphere (the bake of a projected stamp); a wrapped stamp's
   Preview bake on a large model was not measured.
4. **Jobs are plain named threads with a channel** (like `stamp_bakes::warm_up`), not `Jobs` / `JobOutput`: they have
   no progress, change nothing and are dropped when they no longer fit.
5. **No button inside the toast.** Toasts have no buttons and their area takes no input while only timed toasts are
   shown (8.9, so they never swallow a click on the view). The offer is a warning toast plus the **Rasterize** button in
   the canvas header (`PanelHit::StickerHeaderRasterize`) and in the Tool tab (`StickerRasterize`); never a silent
   rasterize.
6. **Rasterize is built now** (P8's command list): the paint tools must offer a working Rasterize. Core
   `rasterize_sticker_layer` (the look is unchanged: test k7a) and `StickerAction::Rasterize` (also Layer > Rasterize in
   the view, which said "not available yet").
7. **Core additions in a U package** (as P6a / P6b): `doc_sticker_paint.rs` (region read-out, render jobs, colour and
   layer read-outs, Rasterize), `render::render_region`, `select::selection_in_content`, and a **row copy in
   `resample::copy_exact`** for plain moves: entering stroke mode on a 2048-px sticker took 84 ms with a cold layer cache
   because the per-pixel copy of a whole-pixel move costs the whole source; the row copy gives the same bytes (test
   k7a, identity sweep unchanged) and 27 ms.
8. **`render_region` equals the whole render on every pixel that is seen**; a fully transparent pixel may carry another
   colour (a layer clipped to the live layer is left out where the live layer draws nothing in the region, the whole
   render composites it at alpha 0). The canvas shows premultiplied pixels, so nothing visible differs; the tests compare
   "seen" pixels.
9. **Selections**: the Marquee and its outline are P7b's. P7a adds the commands `SelectRect` / `SelectNone` (the
   scenario's "marquee made by a command") and draws the selection's bounds as a static two-colour dashed rectangle.
10. **Small rules the plan does not fix**: a hidden layer is not painted (status line, as Photoshop); "Add mask" turns
    "Paint on mask" on; Fill with a selection fills the selection (the plan's "or the selection when there is one");
    the Eyedropper takes the colour opaque and picks nothing on a transparent point; a sticker tool switch or leaving the
    view in mid-stroke keeps the stroke (Esc, Undo and other commands cancel it); a gradient drag under 0.5 px draws
    nothing.
11. **Brush tab in the view**: the Type row is greyed with the hint; the Sphere / shape rows are not shown (they never
    apply to the sticker brush) rather than greyed one by one.

## Numbers measured

- `cargo test --workspace --no-fail-fast`: **1,719 passed, 1 failed (WA20 only:
  `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth`), 63 ignored** (`wte_core` lib 1,080 / 1 /
  39, `wte_app` 582 / 0 / 24, integration 15 + 9 + 8 + 25). Against the P6b check (1,698 / 1 / 62): +10 core, +11 app,
  +1 ignored (perf). Log `target\sticker\p7a\test.log`.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo build --release -p wte_app`: OK, exe 30,409,216 bytes, SHA-256
  `90c4a3eceb84558d3178e0bee183c35930fff7ff01d16c75b7dd9c30c1bdf51d`.
- `perf_k7a_dab_region_and_restyle` (release, this machine; 2048 x 2048 sticker, a noise picture under a styled layer,
  the layer cache cold as in the app after a render-cache hit): entering stroke mode (one full recomposite) **27.1 ms**
  (budget 50; 84 ms before the row copy), one dab of size 64 plus its dirty-rectangle composite worst of 50 **0.19 ms**
  (budget 2), the Preview restyle job **51.1 ms** (budget 60, off the UI thread), the end restyle job **97.7 ms** (budget
  300, off the UI thread; 212 ms before the row copy).
- Scenarios with that exe (`target\sticker\p7a\run1\`, off-screen, no focus taken): `sticker_edit` (234 frames) and
  `sticker_pen` (174) `"ok": true`. Re-run of the earlier sticker scenarios (`target\sticker\p7a\run2\`):
  `sticker_view`, `sticker_keys`, `sticker_styles`, `sticker_tray`, `sticker_controls` all `"ok": true`.
- Shots looked at against their `expect`: `edit_brush` (the logo with a straight red line across its upper part, Layer 2
  active, the sphere with the line, the Brush options in the Tool tab) matches; `edit_styled` (the cut line and the second
  line with white die-cut borders and a soft shadow, no stroke-mode note, the sphere with the same borders) matches;
  `edit_final` (the logo on a yellow square, the two bordered lines, a red-to-blue band at the bottom, three layer rows
  with Layer 3 above Logo) matches; `pen_canvas` (a thin and a wide blue line, both cut by a vertical gap, the Brush
  highlighted, "Pen detected") matches; the final shot of `sticker_edit` (back in 3D after undoing everything: the plain
  logo on the sphere) is as expected.
- Frame spans (`WTE_FRAME_LOG=1`, `tools/sticker/frame_spans.py`): `sticker_edit` worst `sticker.canvas` 2.70 ms,
  `sticker.live` 1.25 ms (the Preview bake included), `sticker.region` 0.48 ms, `sticker.render` 2.59 ms; `sticker_pen`
  worst 1.66 / 1.14 / 0.02 / 0.87 ms; limit 50 ms (8.4).
- Identity sweep (8.5; `tools/objtabs/sweep.sh` with the final exe and `tools/sticker/jobs_identity.txt` into
  `target\sticker\p7a\sweep\`): **21 / 21 `"ok": true`**; `compare_baseline.py` against `target\sticker\baseline\sweep`:
  214 of 216 files the same plus the NOTE of the P6b shot `side_tabs/shot_tab_sticker.png`; the two others,
  `side_tabs/shot_small_640.png` / `shot_small_900.png` (compared whole), are byte-identical to the P6b and P6b-check
  sweeps (the mode strip's "Sticker" button and the two-row tab strip, outside the compared area of 8.5). Exports, saved
  layer PNGs and manifests: all the same (`target\sticker\p7a\compare.txt`).
- Programs started: cargo, Python and the WTE release exe (only with `--screenshot ... --scenario ...`, off-screen, no
  focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No new crate; `Cargo.toml` /
  `Cargo.lock` unchanged. New files LF; every edited file was LF and stays LF.

## Open (for the checker)

- The tablet rule's 24-pt hit targets (open since the P6a check) applies to the new buttons too (egui's 20-pt rows).
- An Undo pressed during a gradient drag (which has changed nothing yet) cancels the drag and undoes the entry before
  it, like a Move press without motion (P6a's rule: only a drag that changed something is undone alone).
- The Brush options' Size tooltip still says "in texels" (the shared widget); in the view the size is in sticker pixels.
- The model's Preview bake of a **wrapped** sticker stamp during a stroke runs on the UI thread every 100 ms; not measured
  on a large model (the scenarios use the sphere and a projected stamp).
