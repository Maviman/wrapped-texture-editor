# Part G — WinTab, opt-in

Built (`crates/wte_app/src/pen/wintab.rs`; hooks in `pen/win.rs`, `pen/mod.rs`, `app.rs`, `dialogs/tablet.rs`):

- Pure, on every platform: `Packet` (12 bytes: cursor, buttons, normal pressure), `WintabPen`, `wintab_state` (the
  newest packet decides: tip = button 0, pressure / max with `max <= 0` -> 1023, clamped; eraser = cursor % 3 == 2),
  `spread(prev, now, n)` (linear, last = now), `apply(frame, events, tab, ramp)`: while the tip is down every pointer
  note of the frame gets the spread pressures in order (a press the first value; a new contact starts at its own
  pressure; an Ink pen's notes are replaced; notes are built for a mouse-only frame); `in_range` / `in_contact` /
  `pressure` / `inverted` follow WinTab while it is in range or down; without a WinTab pen nothing changes.
- Windows FFI (`wintab::ffi`): `wintab32.dll` loaded at run time (`LoadLibraryW` + `GetProcAddress` of the six
  functions, kernel32 `raw-dylib`; a load-time import would stop the exe from starting without a driver); `open`
  (`WTInfoW(WTI_DEFSYSCTX)` with the size checked first, `CXO_MESSAGES` added, `CXO_SYSTEM` kept, packet mask
  `PK_CURSOR | PK_BUTTONS | PK_NORMAL_PRESSURE`, absolute mode, message base 0x7FF0, pressure range from
  `WTI_DEVICES / DVC_NPRESSURE`, `WTOpenW(hwnd, &lc, TRUE)`), `close`, `poll`, `status`, `on_message` (`WT_PACKET`:
  `WTPacketsGet(ctx, 64)` until fewer than 64 come; `WT_PROXIMITY`: range; `WM_ACTIVATE`: `WTEnable`, `WTOverlap`).
  `LOGCONTEXTW` = 212 bytes and `AXIS` = 16 asserted at compile time, field offsets tested.
- The subclass forwards `WT_PACKET` / `WT_PROXIMITY` / `WM_ACTIVATE` to `on_message` (in `catch_unwind`) and closes the
  context at `WM_NCDESTROY`. `win::hwnd()` gives the subclassed window.
- `PenState::sync_tablet_input` (in `logic`, interactive runs only) opens / closes on a change of `prefs.tablet_input`
  and returns the toast text when WinTab cannot be used ("No WinTab tablet driver found; using Windows Ink", "The WinTab
  driver did not open a tablet context; ...", "WinTab cannot be used without the pen window hook; ..."), shown as an Info
  toast once per change. `PenState::apply_wintab` (in `raw_input_hook`, interactive only, after the router).
- Help > Tablet setup: the combo "Tablet input: Windows Ink (recommended) / WinTab" (writes the preference; saved at exit
  like every pref), the line "Use WinTab only if your driver's Windows Ink option is off. Takes effect at once.", the
  WinTab state line ("WinTab: off" / "on" / "no WinTab driver found (Windows Ink is used)" / ...); paragraph 2 of the
  help now ends "Or choose Tablet input: WinTab below." README and hand test 9 mention it.

No double input, by construction: WinTab creates no events; positions and buttons always come from the egui pointer
stream (the driver's own mouse emulation via `CXO_SYSTEM`, or the Ink path); a pen driver that sends both Ink pointer
messages and WinTab packets gives one stroke (the Ink path's events) with WinTab's pressure. Without packets (wrong
pref, no driver) the Ink pressure / a mouse's 1.0 is used.

Tests: `wintab::tests::the_newest_packet_decides`, `spread_ends_at_now`,
`mouse_events_get_wintab_pressures_while_the_tip_is_down`, `an_ink_pen_with_wintab_gives_one_stroke_with_wintab_pressure`,
`without_a_wintab_pen_nothing_changes`; `wintab::ffi::tests::layouts`, `closed_context_reports_nothing`;
`pen::tests::the_tablet_input_preference_is_applied_once`; the GPU dialog test sees the combo; the prefs round trip of
`tablet_input` (part B).

Not verifiable here: everything behind `LoadLibraryW("wintab32.dll")` (this machine has no tablet driver): the context
options a real driver accepts, packet timing relative to the driver's mouse clicks (a press that arrives before the
first packet of a contact paints its first dab at full pressure — hand test 9 would show it), proximity messages,
cursor numbering of non-Wacom drivers. Hand test 9 in `dist/HOW_TO_TEST.txt`.

Check G: see `build.md` "Part G".
