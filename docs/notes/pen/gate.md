# Gate + completeness critic of pen-input (2026-11-24)

Gate of `pen-input` @ f751aa4 (builder parts A-G + the checker's fixes), then the fixes below. Report:
`docs/reports/PEN_INPUT_GATE_REPORT.md`. Outputs: `target\pen\gate\`.

## Choices made here

- **Hover dead band (fix, `pen/route.rs` `HOVER_DEADBAND` = 1 pt).** A hovering Windows Ink pen reports at the
  digitizer's rate (100-300 Hz) even when held still, with sub-pixel noise (winit computes positions from the himetric
  location, so almost every report is a new `f64` position). egui sets `PointerState::last_move_time` on **every**
  `PointerMoved` (egui 0.37 `input_state/mod.rs` 1155), and a tooltip needs `time_since_last_movement() >=
  tooltip_delay` and `is_still()` (`containers/tooltip.rs` 408-436). So with a pen held over a button no tooltip would
  ever show (hand test 3 expects them). Not confirmable without a tablet, but the egui rule is certain and the fix is
  local: the router drops a pen's hover `PointerMoved` closer than 1 pt to the last one it passed. Contact moves are
  never dropped; the anchor is cleared at the press, at the tip lift / `PointerGone`, at a Leave and at any mouse pointer
  event, so the first hover move after those always passes. A mouse frame is untouched (the router's mouse-only path).
  Test `route::tests::a_still_hovering_pen_sends_no_moves`. Effect on the hover ring: it lags the pen by under 1 pt.
- **Help > Tablet setup text (6 paragraphs instead of 5).** "The lower pen button is the right mouse button" read as if
  pressing it while hovering clicks; with Windows Ink the button counts only while the tip touches (decision D6). Now:
  "Hold the lower pen button while the tip touches the tablet for the right mouse button". The status-bar paragraph now
  points to the Pen line under Pen pressure for narrow windows (the status item is dropped first, by plan). New
  paragraph: on a pen display with touch a resting hand acts as a mouse (egui-winit emulates a mouse for the first
  touch; unchanged from `main`), so turn touch off in the driver or turn on Windows' "Ignore touch input when I'm using
  my pen". README matches.
- **Hand tests (`dist/HOW_TO_TEST.txt`)** made precise: no offset between ring and pen tip (also on a second monitor:
  winit maps the himetric location through `GetPointerDeviceRects`, never tried with a tablet mapped to part of the
  desktop), the tooltip check, a stroke starts where the tip touches, undo of an eraser stroke, the barrel button held
  while the tip touches, the WinTab specifics the checker left open, and a new test 11 (pen display + touch).

## Looked at, no change

- Status-bar priority: kept lowest (plan). Raising it above the undo label would make the bar's undo label flicker
  whenever a pen comes into or leaves range; the Tool tab line always shows the pressure.
- `WM_POINTERLEAVE` starts no frame (winit sends no event for it), so the Leave record waits for the next frame (the
  next mouse move or pen sample). The eraser override and the "Pen" read-out can therefore lag until something moves;
  nothing paints in between. Not worth a repaint hook in the window procedure.
- Installer: nothing new to stage (no DLL, no file; `wintab32.dll` comes with the tablet driver and is loaded at run
  time only in WinTab mode). `installer/smoke-test.ps1` runs `--screenshot` runs, which never install the subclass.
- accesskit: `accesskit_winit` subclasses the window before `WteApp::new`, so the pen subclass sits on top of it and
  forwards to it; at `WM_NCDESTROY` the pen subclass restores accesskit's procedure first.
