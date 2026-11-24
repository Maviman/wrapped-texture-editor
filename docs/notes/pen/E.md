# Part E — readout and help

Built:

- Status bar (`panels/status_bar.rs`): `StatusWidths::pen`, `StatusPlan::pen`; the pen read-out (`PenFrame::readout`:
  "Pen 47 %" / "Pen (eraser)" / "Pen") is measured only while a pen is in range and planned last, and only when the undo
  label was taken (so it is always dropped first, never shown while a higher item is missing); drawn at the left end of
  the right group with the hover text "Pen pressure (Help > Tablet setup)". Without a pen the plan and the bar are
  unchanged.
- Help > Tablet setup (`dialogs/tablet.rs`, `Dialog::TabletHelp`, menu entry between Shortcuts and About): the five
  paragraphs of plan section 7 (`TABLET_HELP`) with the Pen pressure location written as "Tool tab > Brush options (and
  Eraser options) > Pen pressure" (where the rows are; the plan said "Brush tab"), the pen line, Close; closes by Close,
  backdrop click and Esc. The "Tablet input" choice is added by part G.
- README.md: section "Pen tablets" (five points). `dist/HOW_TO_TEST.txt`: section "Pen tablets (new in this build)" with
  the ten hand tests of plan section 12 (item 9 is completed by part G).

Tests:
- `status_bar::tests::a_wide_bar_shows_everything_and_a_narrow_one_drops_the_least_important_first`: the pen item in
  the widths; at 1400 pt everything incl. the pen, without a pen the old plan; at 960 pt the undo label stays and the pen
  goes; at 900 pt both go; the width loop includes the pen.
- `status_items_never_overlap`: with a pen in contact at 1280 / 900 / 640; at 1280 the full hover read-out leaves no room
  for the pen (6 items, the pen dropped first), with the hover cleared the pen read-out is drawn (5 items), no overlaps.
- `control_sweep::panel_tests` menu sweep: Help > Tablet setup queues `ShowDialog(TabletHelp)`.
- GPU test `about_shortcuts_and_message_dialogs_close_by_button_backdrop_and_esc` (ignored by default, run here with
  `--ignored`: passed): the dialog shows its heading and the pen line, closes by Close, backdrop and Esc.
- Scenario `pen` step 10: `ShowDialog(TabletHelp)`, assert open, `Shot("tablet_help")` (looked at: all five paragraphs,
  "Pen detected", Close), `CloseDialog`, assert closed. `"ok": true`.

Check E: `cargo test -p wte_app` 444 passed + the fixed status test (5 of 5 status tests pass); clippy 0 warnings;
release build; scenario `pen` ok.
