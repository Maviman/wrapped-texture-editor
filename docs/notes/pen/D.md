# Part D — Windows glue (subclass, Ink feedback off, system gestures)

Built (`crates/wte_app/src/pen/win.rs`, `#[cfg(windows)]`; `pen/mod.rs` `install`, `drain_window_records`,
`discard_window_records`; `app.rs`):

- HWND from `cc.window_handle()` (`eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle::Win32}`); no new dependency.
- `install`: `SetWindowFeedbackSetting` off for the five pen feedback kinds (2-6), `SetPropW("MicrosoftTabletPenServiceProperty",
  0x10019)`, then the subclass (`GetWindowLongPtrW` stored before `SetWindowLongPtrW(GWLP_WNDPROC)`, so a message
  dispatched right after the swap always finds the previous procedure).
- `pen_wndproc`: `WM_TABLET_QUERYSYSTEMGESTURESTATUS` answered directly with 0x10019; pointer DOWN / UPDATE / UP of a
  `PT_PEN` pointer read with `GetPointerPenInfo` + `GetPointerPenInfoHistory` (at most 1,024 entries; the current entry
  alone if the history call fails), decoded oldest first into the shared queue; `WM_POINTERLEAVE` / `WM_POINTERCAPTURECHANGED`
  queue a Leave; `WM_NCDESTROY` restores the old procedure and removes the property first; every message is forwarded
  with `CallWindowProcW` and its result returned. The message work runs in `catch_unwind`.
- Structs `POINTER_INFO` (96 bytes) and `POINTER_PEN_INFO` (120) with a compile-time size assert on 64-bit and offset tests.
- `WteApp::new`: installed in interactive runs; in a self-test run only with `WTE_PEN_SUBCLASS=1`. `--gpu-check` exits in
  `main` before the app exists, so it never installs.

Deviations / choices:

- `GlobalAddAtomW` (plan 5.2 FFI list) is not used: `SetPropW` takes the property name as a string and adds the atom
  itself. `RemovePropW` is used at `WM_NCDESTROY` (a window must remove its properties before it is destroyed).
- `WM_POINTERCAPTURECHANGED` queues a Leave only while that pen is in contact. Windows can send it when the implicit
  capture ends at a normal Up; a Leave then would mark the hovering pen out of range between strokes (and flip the eraser
  override off and on). Contact state per pointer id is kept in the shared state.
- If the install fails (no Win32 handle, `SetWindowLongPtrW` fails) the app logs it and sets `PenRouter::touch_is_pen`:
  every touch is then routed as a pen with winit's `force` as the pressure (and leaves range at its End, since no Leave
  comes). The plan's "the router's fallback still gives a working pen with winit's pressure" needs this: without
  records a pen's touch cannot be told from a finger's. On a touch screen with a failed install, fingers then act as a
  mouse too (the app has no touch gestures).
- In a self-test run with `WTE_PEN_SUBCLASS=1`, records the subclass queued are dropped every frame
  (`discard_window_records`): a real pen used meanwhile must not reach a scenario.

What could and could not be tested here:

- Unit tests: struct layout (`struct_layouts`), oldest-first decoding with a skipped entry and the contact bookkeeping
  (`the_history_is_queued_oldest_first`), Leave vs lost capture (`leave_and_lost_capture`), the queue cap; the router's
  `without_the_subclass_every_touch_is_a_pen_with_winits_pressure`.
- Not testable without a pen: the message path (`GetPointerType` / `GetPointerPenInfoHistory` on real pen messages),
  the effect of the feedback settings and of the gesture answer. Hand tests 1-7 of `dist/HOW_TO_TEST.txt`.
- The subclass on a real window does no harm: release exe, scenarios `smoke`, `pen`, `paint3d` without and with
  `WTE_PEN_SUBCLASS=1` all `"ok": true`; the log says "window procedure subclassed; Ink pen feedback off for 5 of 5
  kinds; pen service property set" in every subclassed run; the five screenshots of the two sweeps are pixel-identical
  (`tools/pen/compare_runs.py target\pen\d_runs\plain target\pen\d_runs\subclass`). The windows stayed off-screen and
  unfocused; no input was injected.

Check D: `cargo test -p wte_app` 445 passed (440 + 5), 0 failed; clippy 0 warnings; release build.
