# Build note: S1 - GrabCut box cut-out (STICKER_PLAN 7 "S1", 4.5 "GrabCut box", 8.4)

Builder, 2026-11-27, branch `stickers` (from 9425652, the P13 check). Kind K + U, stretch. A new sticker tool **Cut Out
Box** (Shift+W): drag a box around the object of a photo on the active picture layer and GrabCut cuts it out of its
background - colour models and a minimum graph cut, no AI, no network, deterministic. It runs on a job with a progress
note and Cancel / Esc, has the option "Into mask (keep pixels)", is limited by the selection like every cut-out command
and lands as one undo step "Cut Out Box" (an image stamp is promoted inside it). No new crate, `Cargo.toml` /
`Cargo.lock` unchanged. Nothing of a project, an export or a link changes: the tool only makes a pixel or mask edit
through the existing `replace_sticker_pixels` / `set_sticker` paths.

## What was built

| Item (plan 7 "S1", 4.5, 8.4) | Where | Notes |
|---|---|---|
| GrabCut box: 5-component full-covariance GMMs per side (deterministic k-means++ start), 8-neighbour grid with contrast-sensitive weights (`GAMMA = 50`), Boykov-Kolmogorov max-flow, 5 iterations on a copy of at most 512 px, one re-cut at full size in a band of 6 px around the result, the soft edge of step 4 | new `wte_core/src/sticker/grabcut.rs` (`cut_out_box`, `BoxCutRefusal`; re-exported at the crate root) | MIT notice of plan 4.0 at the top (ported with PhotoCraft's `segment/{grabcut,gmm,maxflow}.rs` open). Details and the deviations below; ARCHITECTURE 6.12 "GrabCut box" |
| Shared soft edge | `sticker/cutout.rs` | Remove Background's step 4 moved unchanged into `pub(crate) fn soft_edge(src, region, key_of)`; `remove_background` calls it (its `k2_*` tests pass unchanged). GrabCut's key = the colour of the closest background pixel (a photo has no plain background colour) |
| Tool "Cut Out Box" | `wte_app/src/state.rs` (`StickerTool::CutOutBox`, in `ALL` after the Magic Eraser, `paints()`), `theme.rs` (icon: a dashed box around a figure), `shortcuts.rs` (Shift+W in `STICKER_SHORTCUTS`, before W) | the tool strip has 15 buttons (fits: the strip sizes its buttons from the height, as before) |
| The drag | `sticker/canvas.rs` (`CanvasDrag::CutBox`, press / motion / release / draw / cursor), `sticker/mod.rs` (`StickerAction::CutOutBox([x0, y0, x1, y1])`, `CancelCutOutBox`) | the press checks `paint_target` (picture layer, shown, unlocked; text / shape: the Rasterize offer); a box of at least 2 x 2 layer px pushes the action; it is a content edit (the shared-picture question first) |
| The job | new `sticker/tools/cut_box.rs` | `start` cancels a running cut and queues the box; `after_dispatch` (app, after the frame's dispatch; also every canvas frame) starts it when no cut thread runs (at most one, a cancelled one included); inputs as `Arc`s; the thread runs the cut and `filters::apply_cut`; `frame` takes the result (waits for a drag / stroke; drops it with `CHANGED_NOTE` when the layer no longer holds the same pixels and mask; refusals as a status line + info toast; else `tools::put` / `put_new_mask`); `pending` is part of `live::pending` (the idle check); leaving the view / another target stops it |
| Progress note, Cancel | `sticker/canvas.rs` header ("Cutting out... N %" + Cancel, `PanelHit::StickerCutBoxCancel`), `sticker/layers_ui.rs` (Tool tab: "Into mask (keep pixels)" `PanelHit::StickerCutBoxInto`, a progress bar, help), Esc in `sticker::escape` | plan 8.4: "on a job with a progress note" |
| `filters::apply_cut` | `sticker/filters.rs` | the pixels / existing mask / new mask change of a found cut, shared by Remove Background and the Cut Out Box |
| Scenario `sticker_grabcut` | new `selftest/scenarios/sticker_grabcut.rs`, `scenarios.rs` (73) | below |
| Docs | ARCHITECTURE 1 (overview), 2.2 (files), 5.27 (API), 6.12 (GrabCut box), 8.11 (the row), 8.15 (Cut Out Box), 9.1, 9.2, 10 (S1 row); HANDOFF 4.1 (list, 73, the entry) and 5.4; `dist/HOW_TO_TEST.txt` "Stickers, part 3" step 13; README; `installer/RELEASE_NOTES.md` | |

## Deviations (with reasons)

1. **Window instead of the whole picture.** The plan says "computed on a copy scaled to at most 512 px". The copy is of
   the box plus a margin (15 % of the box's longer side, at least 8 px; clipped), not of the whole layer: pixels farther
   out are background anyway, and the background models learn the colours near the object (the reference does the
   same). The scale is a whole step (box filter, alpha-weighted), so the longer side of the window is at most 512.
2. **Ring fallback.** A box over the whole picture leaves no background outside it; then a ring of 1/50 of the shorter
   working side is background (the reference's rule). Without any opaque background: `NoBackground`.
3. **Early stop.** The iterations stop when no label changed (the result is the same as running all 5).
4. **Transparent pixels** are fixed background, are not samples of either model and have no smoothness link.
5. **Soft edge key.** Step 4's key is the closest background pixel's own colour (as Remove Background's "Follow
   gradients"), since a photo's background has no plain colour.
6. **Picture size limit.** The tool works on pictures up to 4,096 px a side (like Blur / Sharpen; several full-size
   planes at a few bytes a pixel); larger ones say so. The plan sets no limit.
7. **Key.** The plan gives the tool no key; Shift+W (W is the Magic Wand, as Shift+E is the Magic Eraser next to E) so
   that `every_sticker_tool_has_a_key` holds and the tool strip's tooltip names it. Outside the Sticker view Shift+W
   stays the logical match of W (Wireframe), tested in `colliding_chords_run_the_action_of_their_mode` and in the
   `sticker_keys` scenario.
8. **Turned layers.** The box is taken in the layer's content pixels as the bounds of its four corners through the
   inverse transform, so on a turned layer the cut's box is the (larger) bounding box of the turned box.
9. **Cancel granularity.** A cancel is seen between the cut's steps (the longest step is one max-flow, about 0.15 s at
   512 px); the cancelled thread is counted until it ends and a new box waits for it.

## Tests

- Core `sticker/grabcut/tests.rs` (`k_s1_*`, 14): IoU >= 0.9 on three synthetic photo-like scenes with textured
  backgrounds - a shaded ball on grass 400 x 300 (**0.9990**), a mug with a red band and a handle on wood under a wall
  640 x 480 (**0.9968**), a star with a rim and a blue centre on a purple / teal carpet 1200 x 900 through the scaled
  copy and the full-size re-cut (**0.9965**); everything outside the box cleared, the edge soft; the same result for 1
  and 8 threads and for two runs; refusals (empty, box under 4 x 4, outside, transparent picture, cancelled at once and
  midway); a box over plain grass keeps little; progress 0 .. 100 rising; a box over the whole picture (ring) 0.9990;
  transparent pixels stay background; apply / apply_to_mask; the grid cut equals the brute-force minimum on 40 random
  small grids; fixed neighbours fold into the terminals; the colour models.
- Core perf (ignored, release): `perf_s1_grabcut_box_2048`.
- App `sticker/tools/tests_s1.rs` (11): the canvas drag = one entry "Cut Out Box" that promotes (alpha checks, kept
  area within 10 % of the ball), undo; Into mask (new mask; then on a layer that has a mask); the selection limits it;
  a click cuts nothing; Cancel; a second box while one runs waits for its thread, only the last lands, never two
  threads; a layer changed meanwhile drops the result; a result waits for a running drag; leaving the view stops it;
  refusals; `content_rect` of a moved and scaled layer.
- `shortcuts::tests::colliding_chords_run_the_action_of_their_mode` (Shift+W row), `scenarios::tests` (73), the
  `sticker_keys` scenario (Shift+W row).

## Numbers measured (this machine, release)

- `cargo test --workspace --no-fail-fast`: **1,944 passed, 1 failed (WA20, the known one), 69 ignored**
  (`target\sticker\s1	est_full.log`). The S1 tests again after the clippy fixes: core 14 / 14, app 11 / 11.
- `cargo clippy --workspace --all-targets`: **0 warnings** (after fixing four in the new code).
- `cargo build --release -p wte_app`: OK; exe SHA-256 `01644da67ed760e5995c9f0e7766f3aea3621d893ff796026316235440e59fa1`
  (copy `target\sticker\s1\wte_s1.exe`).
- `perf_s1_grabcut_box_2048` (release, serial): the 2048 x 2048 star scene with its box **862 ms**, a box over the
  whole 2048 x 2048 picture **1,215 ms** (plan 8.4: 3 s, on a job). Measured before the clippy fixes (an iterator
  instead of an index loop and a NaN test spelled differently: no change in the work).
- IoU: ball on grass 0.9990, mug on wood 0.9968, star on carpet 0.9965 (through the re-cut), whole-picture box 0.9990;
  the scenario's photo >= 0.9 (asserted).
- Scenarios (`target\sticker\s1\runs\`, off-screen, no focus taken): `sticker_grabcut` ok (4 s; again with
  `WTE_OLE=1 WTE_PEN_SUBCLASS=1`: ok), `sticker_keys` ok (Shift+W row), `sticker_cutout` ok (Remove Background after the
  `soft_edge` move, the halo bound holds), `sticker_view` ok, `smoke` ok. Shots looked at: `grabcut_cut` (the ball
  alone on the checkerboard, round clean edge, the sphere shows the ball without grass, the Tool tab shows "Into mask"),
  `grabcut_mask` (the same picture, Into mask ticked, the Mask block with Invert / Apply / Delete), `grabcut_die_cut`
  (the ball as a die-cut sticker with a white border and shadow on the sphere), `keys_inside` (15 tools in the strip,
  the new icon between Magic Eraser and Eyedropper). Each matches its expect text.
- Identity sweep (8.5; `tools/sticker/jobs_identity.txt` into `target\sticker\s1\sweep\`): 21 / 21 `"ok": true`.
  Against the P13 check's sweep: 216 the same, `export_checks/shot.png` differed only by a fading "Exported 4 files"
  toast; a re-run of that job (`target\sticker\s1\sweep_rerun\`) is pixel-identical: **217 / 217**. Against the P0
  baseline: the same plus the two known `side_tabs` small-window shots and `shot_tab_sticker.png` (since P6b). No old
  project, export or flow changed.

## For the checker

- `cut_box.rs` is the only new thread; check the cancel / queue rules (at most one cut thread) and that a result never
  lands on another layer or document (`frame` compares the target and the very pixels and mask).
- `soft_edge` is a move of Remove Background's step 4 (no behaviour change): the `k2_*` tests and the P2 halo checks of
  `sticker_cutout` still pass.
- The scenario does not show the progress note in a shot (the cut of a 320 x 240 photo ends within a frame or two);
  the note, Cancel and the queue are covered by the held-thread unit tests.
- Only cargo, Python and the WTE exe in `--screenshot` / `--scenario` mode were started; PhotoCraft and Photoshop never.
