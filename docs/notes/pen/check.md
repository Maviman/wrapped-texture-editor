# Check of pen-input (independent checker, 2026-11-24)

Checked `main..pen-input` @ 67f94ad against `docs/plans/PEN_INPUT_PLAN.md` and hunted bugs in the subclass, the router,
the eraser override, the core walk and the WinTab path. Plan items A-G are all present (deviations as the builder
listed them; `docs/notes/pen/F.md` does not exist, part F's notes are in `build.md`).

## Fixed here

1. WinTab: the driver's mouse press of the eraser end ended the eraser override. In WinTab mode every press is a
   passthrough mouse press for the router, so `PenFrame::mouse_press` was true on the eraser's touch-down and
   `eraser_step` restored the previous tool in the same frame: the WinTab eraser painted with the Brush. `wintab::apply`
   now clears `mouse_press` while the WinTab pen is in range or down. Test
   `wintab::tests::the_wintab_eraser_press_keeps_the_eraser`.
2. WinTab: the frame whose newest packet says "tip up" still carries the driver's last mouse moves and the release of
   the stroke. They read pressure 1.0, so with size dynamics every WinTab stroke lifted while moving ended in a
   full-size blob. That frame's untagged notes now keep the last pressure. Test
   `wintab::tests::the_moves_that_come_with_the_tip_up_packet_keep_the_last_pressure`.
3. WinTab: `open` / `close` / `on_message(WM_ACTIVATE)` hold the WinTab mutex while calling into the driver. A driver
   that *sends* (not posts) `WT_PROXIMITY` / `WT_PACKET` from inside `WTOpenW` / `WTEnable` / `WTOverlap` would re-enter
   `on_message` on the same thread and deadlock the UI on its own lock. `on_message` now uses `try_lock` and skips such
   a message. Test `wintab::ffi::tests::a_message_sent_from_inside_a_wintab_call_does_not_deadlock`.
4. Eraser override: a mouse press restored the previous tool while the eraser end still lay in range, and the next
   idle frame engaged the Eraser again, so the tool flipped after every mouse stroke. A restore by a mouse press now
   sets `eraser_hold_off` (as a manual tool change does) until the eraser end leaves or turns back. Extended
   `pen::tests::the_eraser_end_switches_and_restores`.
5. Without the subclass (`touch_is_pen`, install failed) a second finger while the first one painted got a synthesized
   press, moves and release of its own (two "mice" pressing; the stroke zigzags between the fingers). A second touch
   while a contact is held is now dropped in that mode. Test `route::tests::without_the_subclass_a_second_finger_does_not_paint_too`.

## Looked at, no change

- Subclass: `extern "system"`, `GWLP_WNDPROC` via `Get/SetWindowLongPtrW` (x86-64 only; the app ships x64), the
  previous procedure stored before the swap, every message forwarded except `WM_TABLET_QUERYSYSTEMGESTURESTATUS`,
  `catch_unwind` around the work, restore at `WM_NCDESTROY`, single UI thread, no extra native viewports. Positions are
  never taken from the subclass, so DPI and monitors only matter to winit (unchanged). Records are queued before the
  forward, and winit sends its `Touch` events synchronously from the same message, so records and touches of one
  message reach the same `raw_input_hook`; a late touch (winit buffering) falls back to winit's force.
- Pointer ids: the records carry the message's id (`LOWORD(wParam)`), the router matches by `Touch` id = winit's
  `pointerId`; `WM_POINTERLEAVE` / `CAPTURECHANGED` use the same `LOWORD(wParam)`.
- Pressure 0 at touch-down: decoded as 1/1024 (Ink); `force: None` on Ended uses the held pressure; NaN is mapped to 0
  by the core's `unit` and cannot hang the walk (step floor > 0, `MAX_DABS_PER_SAMPLE` guard).
- Barrel pressed mid-stroke: the contact keeps its button (press and release match). Eraser end via another pointer id:
  per-id records, Leave of the tip id, Update of the eraser id.
- Undo: one pen stroke = one history entry, as a mouse stroke.
- Self-test runs: the runner still strips every real event (`Touch` included) and the subclass's queue is discarded.
- Known, not fixed (WinTab, hardware needed): a driver press that arrives before the first tip-down packet paints its
  first dab at full pressure (G.md, hand test 9). Status-bar "Pen N %" is the first item dropped, so at 1280 pt with
  a full hover read-out it does not show (by plan); the Tool tab line still shows it.
