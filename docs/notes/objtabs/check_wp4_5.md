# Check of WP4 + WP5 (tab groups, Objects tab) — builder commit 1c62a8d

Independent check-and-fix pass (HANDOFF section 7). Plan: `docs/plans/OBJECTS_TABS_PLAN.md` sections 6 and 7.

## What was checked

- Plan 6.1-6.6 and 7.1-7.2 item by item against the diff: `UpperTab`, `SideRects` fields and their `Rect::NOTHING`
  rule, `PanelHit` additions, the removed `side_color_area` and collapsing headers, the lower group's unchanged rules,
  `tab_area_height` / `upper_keep` / `layers_area_max`, the split (press, drag, double-click), the focus and chip
  exceptions, Object > Objects Panel, prefs (lenient read, clamp, `with_prefs`, `sync_prefs` at exit, never in
  self-test), every test edit of the 6.6 table (each keeps its intent; G1 is now asserted with every lower tab, which
  is stronger than before), the five 7.2 tests, `header_text` / `thousands` / `row_click_action` unit tests.
- Nothing in the app outside tests reads `SettingsArea` / `ColorSection` / `side_rects`, so the plan's "guard reads
  against `Rect::NOTHING`" needs no code. An owed `Final` of a text field whose page is switched away is still sent by
  `PanelState::end_frame`.
- Tests at 1c62a8d: 1,290 passed / 1 failed (WA20, known) / 47 ignored; clippy 0 — as the builder reported.
- Screenshots (release exe of this worktree, off-screen): `side_tabs` started at 1280x800, 900x600 and 640x480 (every
  upper and lower tab), and a temporary local scenario (not committed) opening `ABeautifulGame.glb` with each upper tab,
  two objects selected, one hidden, a reveal of the last row, and the Brush tab below: `target\objtabs\check45\shots\`
  (before the fixes) and `shots2\` (after). All runs `"ok": true`.

## Findings

1. **Major (fixed): the automatic split had a memory.** With the Brush or the Text tab below, `upper_keep` used the Color
   page height *as last measured*. After the square shrank in a small window (e.g. 640x480), the page measured shorter,
   so back in a larger window the split kept the small room, the square stayed small and measured small again: a stable
   wrong state. At 1280x700 the square stayed 86 pt (fresh start: 140 pt), at 1366x768 118 pt; at 1280x800 the 420 pt
   cap of the lower group happened to hide it. Fix: the room kept is the page with a full-size square
   (`PanelState::color_page_full_height` = measured page beside its square + `COLOR_SQUARE_MAX`). Test
   `the_automatic_split_does_not_depend_on_the_window_sizes_seen_before` (failed before: `(86.0, 380.5)` vs
   `(140.0, 326.5)`).
2. **Minor (fixed): revealing a row near the end of the Objects list overscrolled.** The offset that centres the row was
   clamped at 0 only; egui treats an offset past the end as an overscroll and springs it back, so the list showed empty
   room under the last row (still visible 4 frames later in the ABeautifulGame run). Fix: `objects::reveal_offset`
   clamps to `0..=content - view`. Unit test `reveal_offsets_stay_in_the_scrollable_range`; the 2,000-object test now
   also reveals the last row and checks it ends at the list's bottom.
3. **Minor (fixed): the Objects footer touched the split edge.** The footer's bottom was the lower group's top (546 =
   546 in the ABeautifulGame run), so a press on the lower 8 pt of Hide / Isolate / Show all / Frame lay in the split's
   grab band and silently made the split the user's (and egui's own resize handle covers 5 pt of it). Fix: the list
   leaves `objects::footer_clearance` (grab band + 4 pt) below the footer; the footer test asserts it.
4. **Minor (fixed): `remember_recent` saved the prefs without `panels::sync_prefs`**, contrary to the "directly before
   every `prefs::save`" rule the code states; now synced there too.

## Not fixed / notes

- The builder's deviations (focus rule only for the Text tool, double-click only on the same row, `in_row` filter,
  overlay scroll bars in the upper group, 18 pt indent kept, `side_tabs` added early, non-identical exports of
  `export_flow` / `fuzz_path_box` because the old panel had grown wider) are reasonable and match the plan's intent.
- The general press band of the split (8 pt above the edge) still covers the bottom of the Tool / Color / Texture sets
  pages when they are scrolled to the end; that was so before WP4 (the old Color area sat on the same edge).
- A pick in the 3D view made while another upper tab is shown is revealed when the Objects tab is next opened (as
  ARCHITECTURE says); an out-of-range `object_reveal` is ignored but not cleared (harmless).
- The double-click on the split edge with a real mouse is still a manual check (open issue of the builder).
