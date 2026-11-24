# Pen input: decisions (branch `pen-input`)

The decisions of `docs/plans/PEN_INPUT_PLAN.md` section 1, as built. ARCHITECTURE.md 1.10 holds the same table.

| # | Decision | Why | Where |
|---|---|---|---|
| D1 | A pen is "a mouse with pressure". The pen's `Event::Touch` events are removed from the raw input after their data is taken; finger touches are left alone. | Any `Event::Touch` turns on egui's touch-screen mode (drag-to-scroll, labels not selectable, long press = secondary click). Tap = click, hover and tooltips come from egui-winit's emulated pointer events. | `pen/route.rs` |
| D2 | Pen flags and exact per-sample pressure come from a window-procedure subclass on the app's HWND. | winit drops the flags and gives every coalesced sample of a message the latest pressure. | `pen/win.rs` (part D) |
| D3 | Defaults for both Brush and Eraser: pressure -> Size on, minimum 10 %; pressure -> Flow off, minimum 0 %. | The usual default of painting apps. | `state.rs` `BrushSettings::default` |
| D4 | The core default stays "pressure scales flow, size fixed". | Every existing core test and hash keeps its value; the app always passes its own settings. | `wte_core::PressureDynamics::default` |
| D5 | No pressure curve, tilt, rotation or pressure -> opacity ceiling. | The tablet driver has the curve; the rest is outside "tablets work well". | — |
| D6 | Barrel button + tip = Secondary; a third pointer button = Middle. | Windows Ink semantics: barrel-drag orbits in 3D, pans in 2D, opens context menus. | `pen/route.rs` |
| D7 | The eraser end switches any tool to the Eraser while the inverted end is in range or contact, then restores the previous tool. | Requested. | `pen::eraser_override`, `app.rs` `logic` |
| D8 | WinTab opt-in, last separate commit (part G). | Double input impossible by construction; one commit to revert. | `pen/wintab.rs` (part G) |
| D9 | The brush ring keeps showing the full (maximum) size. | Simple and predictable. | unchanged ring code |
| D10 | No version bump, no release. | The main session's call. | — |

Choices made while building (not in the plan, or made more precise):

- **Pressure pairing by event order, not by position alone.** The plan's `PenFrame::cursor().take(pos)` matched the next
  tag by position. A hover `PointerMoved` at the same position as the following tagged contact event (the hover sample
  right before the tip goes down, which every real pen has) would then take the contact's note and shift every pressure
  of the frame by one. Built instead: `PenFrame::pointer` has one entry per routed `PointerMoved` / `PointerButton`
  (`Some(pressure)` for a pen in contact, `None` otherwise), and `route_pointer` asks the walker for every such event in
  order; a note is used only when its position also matches bit for bit, else 1.0. egui removes only key and AccessKit
  events from `InputState::events` (`count_and_consume_key`, `consume_accesskit_action_requests`), never pointer events,
  so the order is that of `raw_input_hook`. A frame without pen input has no notes: every event reads 1.0.
- **Records are paired by kind, other ids keep their place.** The router searches the feed for the first record of the
  touch's id whose kind matches the phase; records of the same id before it are applied (state, Leave handling) on the
  way; with no match it builds a fallback from winit's `force`. Records left at the end of the frame are applied
  (state only, Leave handling) and the feed is empty after every route: the window procedure and winit see the same
  messages between two frames, so nothing belongs to the next frame.
- **The resolve step shrinks with the thinner end** of the walk (`size_factor(min(last_p, target_p))`) instead of "the
  pressure at the step's end", which depends on the step itself. Denser is never wrong; for a mouse both ends are 1.0.
- **Eraser override hold-off.** A manual tool change while the eraser end hovers drops the override (as planned); without
  more, the next frame would engage it again and fight the user. `PenState::eraser_hold_off` keeps it off until the
  eraser end leaves the range or turns around.
- **The Pen pressure rows sit in the Tool tab** (Brush / Eraser options), under Flow and Spacing, where those rows are
  drawn (plan 3.2: "put the rows where the Flow and Spacing rows of the brush are drawn"). The plan's help text and hand
  test say "Brush tab"; the texts written here say "Tool tab > Brush options > Pen pressure".
