# Part C — pen module, pure parts, wiring, scenario `pen`

Built:

- `pen/mod.rs`: `PenKind`, `PenRecord`, `PenFrame` (pointer notes, `cursor`, `held_pressure`, `readout`),
  `PenCursor::take`, `PenState` (`route`, `status_line`), the eraser override as a pure step (`eraser_step`) plus
  `eraser_override` (state update, returns the tool to set), `drain_window_records` (empty until part D), `FEED_CAP` 8192.
- `pen/decode.rs`: `RawPen` -> `PenRecord` with the constants of plan 4.2 (used from part D; marked
  `allow(dead_code)` in this commit).
- `pen/route.rs`: `PenRouter::process` as in plan 4.3 with the changes listed in `decisions.md` (notes by event order;
  kind-matched record search; leftover records applied at the end of the frame). The End / Cancel of a pen touch
  releases the remembered button (Secondary after a barrel press) even when egui-winit sent only `PointerGone` (Cancel);
  after it the router's "pointer is the pen" flag is cleared, so a later Leave adds no second `PointerGone`.
- `app.rs`: `raw_input_hook` = self-test hook (returns the frame's records) -> `drain_window_records` -> `PenState::route`;
  `logic` starts with the eraser override (`dispatch(SetTool)` when it says so; `logic` now names its `frame` argument).
- `view/mod.rs`: `PointerEvent` carries the pressure of `PenFrame::cursor().take(pos)`; `pointer_at` takes a pressure;
  the drag without motion, the wheel and the hover use `PenFrame::held_pressure()`.
- `panels/tool_options.rs`: the pen line ("Pen: 47 %" / "Pen detected" / "No pen seen yet (Help > Tablet setup)") under
  the Pen pressure rows.
- `selftest/mod.rs`: `Step::Pen`, `PenPoint`, `FrameInput::pen`, `pen_frames` / `pen_up` / `pen_touch` (what egui-winit's
  `on_touch` pushes for a pen, plus the records), `Runner::raw_input_hook -> Vec<PenRecord>`, `Runner::probe_pen` (scratch
  values `pen_contact_frames`, `pen_eraser_frames`, `pen_readout_ok`).
- `selftest/scenarios/pen.rs`: the scenario `pen` (plan 4.5 steps 1-9; ARCHITECTURE 8.11 table row). Step 8 (tap = click)
  uses `Step::Pen` with normalized coordinates computed from the Base row's rect relative to the 3D view (values outside
  0..1); no `ScreenPen` variant was needed. Step 9's readout check is the runner's probe (`PenFrame::readout`, which the
  status bar of part E shows).
- `scenarios.rs`: `mod pen`, `NAMES` + "pen", the count test 51 -> 52.

Tests: `pen::decode::tests` (4), `pen::route::tests` (10, the list of plan 4.3), `pen::tests` (4: cursor, readout and pen
line, eraser switch / restore, manual change drops the override), `selftest::tests::pen_frames_hover_down_moves_up_leave`
(the recorded pen routed gives the mouse drag's events plus `PointerGone`, with the pressures).

Check C:
- `cargo test -p wte_app`: 440 passed, 0 failed, 24 ignored (421 + 19). Core unchanged since part B.
- clippy 0 warnings.
- release build; `--scenario pen` ok (482 frames): ramp column heights 12 (20 %; plan value 11.2), 36 (90 %; 36.4), 38 at
  the end (40 +- 3); pen at 1.0 == mouse in 2D and 3D; flow row 0.2 fainter than 1.0; 3D light < half of full; eraser end
  erased with the Eraser in every contact frame and the Brush back after the Leave; barrel orbited without paint; the tap
  selected Base. Screenshots `target\pen\c_runs\pen\shot_pen_ramp.png` (thin-to-thick 2D stroke), `shot_pen_3d.png`
  (light and full 3D strokes), `shot.png` (3D ramp) looked at: pressure visibly changes the width.
- Regression sweep (`tools/pen/jobs_regress.txt`: smoke default paint3d paint2d stamp text wrap_surface brush_types
  brush_tab brush_shape brush_hover layers undo multi pan2d export export_run export_flow side_tabs objects
  brush_types_path + workflow_path on BoxTextured.glb) all `"ok": true`; compared with the baseline exe's runs by
  `tools/pen/compare_runs.py`: 82 output files byte-identical (every export: PNG / PSD / ORA / ZIP), 3 `.wtep` differ only
  in `manifest.json` `saved_unix` (the save time); 28 screenshots pixel-identical; the others differ only in the side
  panel's new Pen pressure rows (y 188-276 at 1280x800) and, in `export` and `brush_shape`'s dialog shot, in the output
  folder path printed in a dialog (`baseline\runs\...` vs `c_runs\regress\...`). No central-view pixel changed.
