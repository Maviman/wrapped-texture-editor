# Check note: P6a - Sticker view and canvas

Checker, 2026-11-26, branch `stickers` at 1d30b48 (the P6a build). Independent and adversarial check of the
package against STICKER_PLAN 7 "P6a" and the sections it cites (3.1, 3.2, 3.3, 3.5, 3.10, 8.2), with the user's
decisions and plan revision 3. Verdict: **pass after fixes** (four confirmed bugs fixed with tests, a key scenario
added; the minors left are listed at the end).

## What was re-run on the builder's commit

- `cargo test --workspace --no-fail-fast`: 1,669 passed / 1 failed (WA20 only) / 62 ignored, as the builder said.
- Release build OK; scenario `sticker_view` 1280x800: `"ok": true`, 192 frames. Every shot looked at against its
  `expect` text (`target\sticker\p6a_check\run1\`): `sticker_enter`, `sticker_moved`, `sticker_reordered`,
  `sticker_small` (900x600: header Done / Revert / Fit / 100 %, the 14 tools and the colour chip; the sticker larger
  than the narrower canvas, the builder's open issue) and `sticker_left` all show what they claim.

## Plan items, one by one

| Item (plan 7 "P6a" / cited section) | Found |
|---|---|
| `ViewMode::Sticker`, `StickerEdit` (all fields of 3.1), `sticker_canvas_rect`, `PanelHit::StickerCanvas` | yes |
| Never in prefs / `ui_state` (`remembered_view_mode`, `install_document` filter) | yes, tested |
| Ways in: Tool page, Layer menu, row menu, double-click thumbnail, Enter, key 4, mode strip, View menu | yes (Sticker tab, tray and "Edit before placing" are P6b by the plan) |
| Leaving: Done, Esc (after drag / dialog / pending / selection), another mode; flush | yes |
| Revert (B12): one entry, undoable, enabled only when different | yes |
| What the canvas shows per kind (sticker, image stamp virtual + promotion in the first edit, text read-only + Convert) | yes |
| Checkerboard, artboard outline, pixel grid from 800 %, texture by value key | yes; `set_partial` deferred to P7a (deviation 6, accepted: nothing paints in P6a) |
| 3.2 header (name, Done, Revert, zoom, Fit, 100 %) | yes (link badge is P9) |
| 3.2 tool strip replaces the paint tools, paint tool back on leaving | yes |
| 3.2 Layers tab: eye, thumbnail, name, lock, clipped, drag reorder, add / delete / reorder / show-hide / opacity / blend | yes; multi-select and the row context menu deferred (deviation 4) |
| 3.3 key table, routing rule, `NOT_IN_STICKER`; Move (move only) and Hand / zoom | yes; checked key by key in the real window (below) |
| 3.5 structure edits one entry each, Preview folds; `frame_profile` spans | yes |
| 3.10 pen eraser branch, barrel = Secondary pans, companion fingers via the canvas rect, "(on the PC)" | yes (rect through `finger_views`, deviation 2, accepted) |
| 8.2 `Target`, `Step::WaitMs`, shots with `expect` in `OUT.json` | yes |
| App tests listed by the plan | all present |
| Scenario `sticker_view` as the plan describes | yes |
| ARCHITECTURE 1.12, 2.2, 8.2, 8.3, 8.5, 8.7, 8.10, 8.11, 8.13, 8.15; HANDOFF 4.1 | yes |

## Bugs found and fixed (commit of this check)

1. **Major: Layer > Delete in the Sticker view deleted the stamp being edited.** Entering the view calls
   `select_stamp`, which makes the stamp's own layer the active texture layer. The Layer menu's Delete pushes
   `DeleteLayer(active texture layer)` (or `DeleteLayers`), which `in_sticker_view` did not map, so it removed that layer
   and with it the stamp and the open sticker (plan 3.3: "never deletes the stamp or a texture layer"). Same hole for
   Rasterize (`RasterizeActiveLayer`: the stamp layer turned into paint) and New Paint Layer. Fix: `in_sticker_view`
   also maps `DeleteLayer(_)` / `DeleteLayers(_)` -> sticker Delete Layer, `DuplicateLayer(_)` / `DuplicateLayers(_)`,
   `MergeDown(_)`, `AddPaintLayer` -> New picture layer, `RasterizeActiveLayer` -> new `StickerAction::Rasterize` (a
   status line until P8). Test: `dispatch::tests::the_sticker_view_maps_the_layer_commands_to_the_sticker` (mapping,
   and the mapped Delete on the real state leaves the stamp and its layer).
2. **Major: Enter that confirmed a text field opened the Sticker view.** A one-line `TextEdit` drops the focus on
   Enter inside the pass (`surrender_focus`, egui `builder.rs:1331`), before `shortcuts::handle` runs, so `typing` was
   already false and the new global Enter = "Edit Sticker" fired. Renaming a stamp in its Name field (or a layer by
   double-click) and pressing Enter jumped into the Sticker view. Fix: the Enter entries are skipped when
   `state.typing_prev`, like the first Esc. Test: `shortcuts::tests::enter_that_ends_typing_in_a_field_does_not_open_the_sticker_view`
   (fields first, `handle` after them, as in the real frame); it fails without the fix.
3. **Minor-major: an unchanged layer name promoted the image stamp.** The Layers tab's Name field sends
   `RenameLayer` at every focus loss; `set_sticker` with a promotion records an entry even when nothing changes, so a
   click into the Name field of an image stamp's virtual layer and away turned the stamp into a sticker with an empty
   "Rename Sticker Layer" entry and enabled Revert. Fix: `RenameLayer` with the current name does nothing. Test:
   `sticker::tests::an_unchanged_layer_name_is_no_edit_and_does_not_promote` (fails without the fix).
4. **Minor-major: the canvas drag did not follow the interaction guard's rules.** Every guarded command cancelled it:
   Ctrl+Z during a Move drag cancelled the drag *and* undid the entry before it (two steps for one key), and Save during
   a drag saved the layer at its old place. Fix: dispatch finishes the drag for the finishing commands
   (`sticker::finish_drag`, one entry where it is shown), cancels it otherwise, and returns after an Undo that met a drag
   which had moved something (`sticker::drag_moved`), as for `ToolController`. Test:
   `sticker::tests::a_drag_is_finished_by_save_and_undone_alone_by_undo`.
5. **Minor: caches outlived the view.** `StickerUi::canvas` (a whole sticker at scale 1, up to the artboard limit, kept
   as a GPU texture) and the thumbnails stayed after leaving. Their keys hold picture versions that
   `Document::next_pixel_version` numbers per document (the model's doc says "for the life of the process", but the
   counter is a `Document` field), so after a document change a different sticker with the same ids and versions could
   have shown the old picture. Fix: `leave_in` drops both (every document change leaves the view first). Test:
   `sticker::tests::leaving_the_view_frees_the_canvas_and_thumbnail_caches`.

## Every key of the table, in both modes, in the real window

New scenario `sticker_keys` (kept as a regression test; NAMES 56, ARCHITECTURE 8.11, HANDOFF 4.1): outside the view
B E G I X S V Shift+E Shift+G R give their paint tools, W toggles the wireframe, M L C U H change nothing; Enter opens
the view; inside, all 14 tool keys give their sticker tools with the paint tool and the wireframe untouched, S and R do
nothing, F refits, Delete / Backspace leave the stamp with the status line, Enter does nothing, Ctrl+J / Ctrl+[ /
Ctrl+] act on the sticker layers (one entry each), Ctrl+M only says it is not available, the eight arrows nudge by 1 /
10 (one entry each), Shift+X and T run their global actions, Ctrl+Z undoes the last nudge, Esc and 1 / 2 / 3 / F5
leave with the paint tool kept, 4 opens the view again. Result `"ok": true` (516 frames); shot `keys_inside` looked at
(two rows, the copy on top and highlighted, Move highlighted, the Merge Down status line). The shortcut-to-action
mapping itself was already covered by the builder's unit test `colliding_chords_run_the_action_of_their_mode`.

## Numbers after the fixes

- `cargo test --workspace --no-fail-fast`: **1,673 passed, 1 failed (WA20 only), 62 ignored** (`wte_core` lib 1,066 / 1 /
  38, `wte_app` 550 / 0 / 24, integration 15 + 9 + 8 + 25): +4 app tests, one extended.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- Release build OK, exe SHA-256 `5fdb2fde45e28d3be999ac0901b57f68b5c83bb403cde87e411810e22623b22c`.
- Scenarios with that exe: `sticker_view` ok, `sticker_keys` ok, `smoke` ok (`target\sticker\p6a_check\run2`, `run3`).
- Identity sweep (8.5) with that exe (`tools/objtabs/sweep.sh` + `tools/sticker/jobs_identity.txt` into
  `target\sticker\p6a_check\sweep\`): 21 / 21 `"ok": true`; `compare_baseline.py` against
  `target\sticker\baseline\sweep`: 213 of 216 files the same. `side_tabs/shot_small_640.png` / `shot_small_900.png`
  differ only in rows 25..51 (the mode strip with its new "Sticker" tab, excluded by 8.5), as for the builder.
  `export_checks/shot.png` differed in the toast stack: the oldest of four toasts ("Exported 4 files to shot_ec_vis")
  was fading. That is the toast's 4 s wall-clock life against the run's speed, not the code: three more runs of the
  same exe gave two pictures identical to the baseline and one with the same fading toast
  (`target\sticker\p6a_check\export_checks_reruns\`). Every export, saved layer PNG and manifest is the same.
- Programs started: cargo, Python and the WTE release exe with `--screenshot ... --scenario ...` only (off-screen, no
  focus taken). Never `photocraft.exe`, `photocraft-cli.exe` or `Photoshop.exe`. No Cargo file changed. All edited
  files were LF and stay LF.

## Left open (minor; not fixed here)

- Plan 3.10 "every hit target is at least 24 pt": the sticker rows' eye / lock icons are 18 pt wide, and the header and
  footer buttons use the app's 20-pt `interact_size`. P6b's checker lists the tablet rule; one app-wide decision.
- Plan 3.2 "every control gets a `PanelHit`": the sticker Opacity slider, Blend combo, the two Name fields, Auto-select
  and the X / Y fields have none yet (P6b's `control_sweep` work).
- `differs_from_entry` runs every frame (Revert's enabled state) and compares owned pictures byte by byte when their
  versions differ: harmless now, a cost per frame for large painted pictures in P7a (cache it by the document
  revision then).
- The arrow-key nudge works with every sticker tool (the plan places it under Move); Enter on a keyboard-focused
  non-text widget (a button reached with Tab) both presses it and opens the view; Space held in a text field still
  makes a canvas press pan. Small, left as they are.
- The builder's open issues stand (canvas zoom kept on a window resize; frame_spans.py not written yet).
