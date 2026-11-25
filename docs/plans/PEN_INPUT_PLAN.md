# Pen / drawing-tablet input plan (`docs/plans/PEN_INPUT_PLAN.md`)

Branch `pen-input`, worktree `G:\software_projects\texture_wrapper_ap_ui`, cut from `main` @ 1b589a1.
Goal: graphics tablets and pen displays (Wacom, Huion, XP-Pen, Surface) work well on Windows 10/11: pressure, eraser end, barrel button, pen clicks, no Windows Ink disturbances, a readout and setup help.
Nothing here can be tried on a real tablet by the builder; section 12 lists what the user must try by hand.

## 0. Facts this plan rests on

Checked in the sources on 2026-11-24. Re-check any you rely on.

**winit 0.30.13** (`src/platform_impl/windows/event_loop.rs` 1985-2142)
- It handles `WM_POINTERDOWN/UPDATE/UP`, returns 0 without `DefWindowProc`, and sends one `WindowEvent::Touch` per history entry, oldest first.
- Positions are per history entry, but pen pressure is read with `GetPointerPenInfo(pointerId)`, the *current* state. Every coalesced sample of one message therefore carries the same (latest) pressure.
- `normalize_pointer_pressure` maps `1..=1024` to `Some(p/1024)` and 0 to `None`, so hover updates arrive as `Moved` with `force: None`.
- It skips an entry when `GetPointerDeviceRects` or `ScreenToClient` fails.
- It does not handle `WM_POINTERLEAVE` or `WM_POINTERCAPTURECHANGED`, and ignores pen flags and button flags.

**egui-winit 0.37** (`on_touch`, lib.rs 960-1018)
- Per touch it pushes `Event::Touch`, then for the emulated id: `Started` gives `PointerMoved` + `PointerButton{Primary, pressed}`; `Moved` gives `PointerMoved`; `Ended` gives `PointerButton{Primary, released}` + `PointerGone`.
- A hover `Moved` with no touch being translated also gives `PointerMoved`, so hover already works.
- If an `Ended` never arrives, `pointer_touch_id` stays set and later touches with a new id are not emulated.

**egui 0.37**
- Any `Event::Touch` makes `has_touch_screen()` true: scroll areas become drag-to-scroll, labels stop being drag-selectable, windows drag from anywhere. `is_long_touch` turns a long press into a secondary click.

**eframe 0.37 / wgpu 30**
- `App::raw_input_hook(&mut self, ctx, raw)` exists (epi.rs 282) and the app already implements it for the self-test (`app.rs` 281).
- `CreationContext` implements `HasWindowHandle` (epi.rs 101). The trait is reachable as `eframe::wgpu::rwh` (wgpu-30.0.1 lib.rs 329), so no new direct dependency is needed.

**Core (`wte_core/src/stroke.rs`)**
- Strength is `brush.flow * pressure` in `surface_dab` (739) and `texture_dab` (804).
- The radius is read from `session.brush.radius` per dab. `TipGeom::new(field, through)` does not depend on the radius.
- The test `the_shared_query_equals_the_per_set_path` (1480) already changes `s.brush.radius` between dabs of one stroke, so the dab path has no per-stroke radius cache.
- Spacing uses a constant step: `dabs_on_segment`, `OpenSegment{step, count, start_residual, next}`, and the 2D inline loop (526).

**App**
- `ViewPointer.pressure` exists but is always 1.0 (`view/mod.rs` 480, `tools/mod.rs` 695).
- `route_pointer` (`view/mod.rs` 352) already forwards every `PointerMoved` of a frame to `tools.drag`.
- The 3D brush queues `(pos, xf)`, walks them in `resolve` with raycasts, and paints `QueuedSample::Hit(hit)` with pressure literal `1.0` (`tools/brush.rs` 641). The 2D path passes `p.pressure`.
- Brush and Eraser each have their own `BrushSettings` (`state.rs` 227, `#[serde(default)]`, `sanitized()`), stored in `Prefs`.
- `A::SetTool` (`dispatch.rs` 851) does `cancel_tool` + sets `state.tool`.
- `selftest::Runner::raw_input_hook` strips all real input including `Touch`, then appends injected `FrameInput.events`.
- FFI style: `#[link(name = "user32", kind = "raw-dylib")]` blocks (`main.rs` 207, `crash.rs`, `gpu_check.rs`).

## 1. Decisions (record in `docs/notes/pen/decisions.md`)

| # | Decision | Why |
|---|---|---|
| D1 | A pen is "a mouse with pressure". The pen's `Event::Touch` events are removed from the raw input after their data is taken; finger touches are left alone. | Avoids egui's touch-screen mode (see section 0). Tap = click, hover and tooltips come from the emulated pointer events. |
| D2 | Pen flags and exact per-sample pressure come from a window-procedure subclass on the app's HWND. | winit drops the flags and gives one pressure per message. |
| D3 | Defaults for both Brush and Eraser: pressure → Size on, minimum size 10 %; pressure → Flow off, minimum flow 0 %. | Matches the usual default of painting apps: size follows pressure, opacity does not. |
| D4 | The core default stays "pressure scales flow, size fixed". | Every existing core test and hash keeps its value. The app always passes its own settings. |
| D5 | No pressure curve, tilt, rotation or pressure → opacity ceiling. | Tablet drivers have a pressure curve; the rest is outside "tablets work well". |
| D6 | Barrel button + tip = Secondary button; a third pointer button = Middle. | Windows Ink semantics. Barrel-drag orbits in 3D, pans in 2D and opens context menus. Buttons the driver maps to middle click, keys or scroll already arrive as ordinary mouse/keyboard input. |
| D7 | Eraser end switches any tool to Eraser while the inverted end is in range or contact, then restores the previous tool. | Requested behaviour. |
| D8 | WinTab is included as opt-in, as the last separate commit (part G). Pref "Tablet input: Windows Ink (default) / WinTab". It supplies only pressure and the eraser flag; positions and buttons always come from the one egui pointer stream. | Double input is impossible by construction. Reverting the one commit removes it. |
| D9 | The brush ring keeps showing the full (maximum) size. | Simple and predictable. |
| D10 | No version bump, no release. | The main session's call. |

## 2. Part A — core: pressure dynamics and per-dab radius (`wte_core`)

### 2.1 API (ARCHITECTURE 5.19)

```rust
// brush.rs
/// How pen pressure (0..=1) drives a stroke. Default = round 3 behaviour: flow follows pressure, size fixed.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct PressureDynamics {
    pub size: bool,      // radius follows pressure
    pub size_min: f32,   // 0..=1: fraction of the radius at pressure 0
    pub flow: bool,      // per-dab strength follows pressure
    pub flow_min: f32,   // 0..=1: fraction of the flow at pressure 0
}
impl Default for PressureDynamics { /* size false, size_min 0.0, flow true, flow_min 0.0 */ }
impl PressureDynamics {
    pub const OFF: PressureDynamics;                 // size false, flow false
    pub fn size_factor(&self, pressure: f32) -> f32; // exactly 1.0 when !size or pressure >= 1.0
    pub fn flow_factor(&self, pressure: f32) -> f32; // exactly 1.0 when !flow or pressure >= 1.0
}
pub struct BrushParams { /* existing fields */ pub dynamics: PressureDynamics }
```

Exact formulas. Bit-identity matters here.
- `size_factor(p)`: `if !size || p >= 1.0 { 1.0 } else { 1.0 - (1.0 - size_min) * (1.0 - p) }`.
- `flow_factor(p)`: `if !flow || p >= 1.0 { 1.0 } else if flow_min == 0.0 { p } else { flow_min + (1.0 - flow_min) * p }`.
  The `flow_min == 0.0` branch returns `p` itself, so `brush.flow * flow_factor(p)` is bit-equal to today's `brush.flow * pressure`.
- `sanitize` (stroke.rs 231) clamps `size_min` and `flow_min` to 0..=1, NaN → 0.
- `BrushParams::default()` gets `dynamics: PressureDynamics::default()`. The only full literal in the app is `BrushTool::params`.

### 2.2 Dabs
- `surface_dab`: `let r = brush.radius * dynamics.size_factor(pressure);` Everything derived from `r` follows: `k = r * tip.scale`, the offset `tip.offset * r`, `DabShape.reach`, `field_reach(field, k, ..)`. Strength is `brush.flow * dynamics.flow_factor(pressure)`.
- `surface_dab_legacy` (test seam) and `texture_dab`: same two substitutions.
- Before relying on it, read `brush::shared_candidates`, `dab_tip` and the round-7 texel-map brush index. Confirm nothing is cached per stroke by radius (`DabQuery`'s aa/rho, the index). The 1480 test suggests so. If something is, key it per dab and note it.
- No radius floor: the falloff's anti-aliasing ramp makes a sub-texel dab a faint taper. A field tip with `k == 0` already returns early.

### 2.3 Spacing from the current radius (ARCHITECTURE 6.4 "Spacing and interpolation")

A segment is **pressure-sized** iff `dynamics.size && (from_pressure < 1.0 || pressure < 1.0)`. Pressures are those after `unit()`.
Every other segment uses today's constant-step code unchanged. That covers every mouse segment and every stroke with `size == false`.

For a pressure-sized segment:
- `step_at(p) = (spacing * 2.0 * radius * size_factor(p)).max(floor)`. `floor` is `0.5 * texel` in 3D (the value the existing code computes) and `0.5` in 2D.
- `p(t) = from_pressure + (pressure - from_pressure) * t`.
- First dab at `d = (step_at(from_pressure) - residual).max(0.0)` from the segment start.
- While `d <= len`: paint at `t = (d / len).clamp(0, 1)` with `p(t)`, set `last_d = d`, then `d += step_at(p(t))`.
- After the segment: `session.residual = (len - last_d).max(0.0)`. With no dab painted, `last_d = -start_residual`.
- Guard: `bound = floor((residual + len) / step_at(min(from_pressure, pressure)))`. If `bound > MAX_DABS_PER_SAMPLE`, use the existing single-dab fallback (one dab at the sample, residual 0).
- `residual` keeps its meaning (distance since the last dab), so constant and pressure-sized segments mix freely in one stroke.

Pacing: `OpenSegment` gets `var: Option<VarWalk { next_d: f32, last_d: f32, floor: f32 }>`.
- `run_segment` branches on it. The `None` branch is today's code, untouched.
- The `Some` branch runs the walk above and asks `more()` after each dab. It returns `Partial` (segment kept with its `next_d` / `last_d`) when `more()` says no and `next_d <= len`.
- The dab sequence must not depend on pacing.

2D (`stroke_to_2d`): the same walk inline (it is not paced), with the same segment rule.

Unchanged: the bridge rule (`MAX_BRIDGE_RADII * radius` with the base radius), the opacity ceiling, `stroke_break`, history, counters.

### 2.4 Core tests (stroke.rs / brush.rs, ARCHITECTURE 9.1)
1. `size_and_flow_factors`: table test of both factors. Exactly 1.0 at `p = 1` for several `size_min` / `flow_min` (including 0.1, 0.3); `flow_factor(p) == p` bit-equal for `flow_min == 0`; monotonic in `p`; `OFF` gives 1.0 everywhere.
2. `full_pressure_ignores_the_dynamics` (mouse identity): the same 3D and 2D strokes at pressure 1.0 with `size: true, size_min: 0.1, flow: true, flow_min: 0.2` and with `PressureDynamics::default()` give identical layer hashes and identical `stroke_counters().dabs`.
3. Existing hashes: C1, C2, `tests\brush_tips.rs`, `one_dab_per_call_gives_the_unpaced_stroke`, `pressure_scales_the_flow` keep their values with no edits. This is the proof that the default is untouched.
4. `pressure_sizes_the_stroke_2d`: hardness 1, radius 20, `size: true, size_min: 0, flow: false`, a straight 2D stroke with pressure ramping 0.1 → 1.0. The painted column height near 20 % of the path is within ±2 texels of `2 * 20 * p(0.2)`, near 90 % within ±2 of `2 * 20 * p(0.9)`. With `flow: false` the mask is full in the stroke centre at both places.
5. `pressure_spacing_follows_the_radius`: with flow 0.1 and constant pressure 0.25, the dab count over a long segment equals `floor(len / step_at(0.25))` ±1, about 4× the count at pressure 1.
6. `pressure_sized_stroke_is_pacing_independent`: 3D stroke with varying pressures, painted unpaced, with `|| false` (one dab per call + `stroke_resume`) and with a mixed pattern. Identical hashes and dab counts.
7. `pressure_sized_stroke_is_thread_independent`: the same stroke inside rayon pools of 1 and 3 threads (pattern: brush.rs 3151). Identical hashes.
8. `eraser_follows_pressure_size`: `BrushMode::Erase` with size dynamics over a filled layer. The erased width follows the pressure as in test 4.
9. `flow_min_and_size_min_are_sanitized`: NaN / out-of-range values behave as clamped.
10. `too_many_pressure_dabs_fall_back_to_one`: tiny pressure with a huge segment hits the `MAX_DABS_PER_SAMPLE` guard and does not hang.

**Check A:** `cargo test -p wte_core`, clippy 0. Commit "core: pressure dynamics (size, flow) with per-dab radius", push.

## 3. Part B — app: pressure through the tools, settings, prefs

### 3.1 Settings (`state.rs`, `prefs.rs`)
`BrushSettings` gains four fields; `#[serde(default)]` at struct level already makes old prefs load.
```rust
pub pressure_size: bool,       // default true
pub pressure_size_min: f32,    // 0..=1, default 0.10
pub pressure_flow: bool,       // default false
pub pressure_flow_min: f32,    // 0..=1, default 0.0
```
- `sanitized()` clamps both minimums to 0..=1; non-finite → default.
- `BrushTool::params` fills `dynamics` from these.
- `Prefs` gains `tablet_input: TabletInput { WindowsInk (default), WinTab }` with lenient serde like the other enums (unknown word → default; follow how `"kind": "spiral"` is handled). It is stored now and used in part G.

### 3.2 UI (`panels/tool_options.rs` and/or `panels/brush_tab.rs`)
Put the rows where the Flow and Spacing rows of the brush are drawn; locate them first. Under them, a group "Pen pressure":
- checkbox "Size" + slider "Min size" 0–100 % (enabled when Size is on);
- checkbox "Flow" + slider "Min flow" 0–100 % (enabled when Flow is on);
- one status line: "Pen: 47 %" while a pen touches, "Pen detected" while it hovers or once one was seen in this run, else "No pen seen yet (Help > Tablet setup)".

Hover texts:
- Size: "Press harder for a wider stroke. A mouse always paints at full size."
- Flow: "Press harder for more paint per dab."

Both tools (brush and eraser settings) show the group. Update `control_sweep` panel tests and any Brush-tab layout assertion that counts rows.

### 3.3 Pressure through the brush tool (`tools/brush.rs`)
- `unresolved: VecDeque<(Pos2, ViewXform, f32)>`; `ActiveStroke` gains `last_pressure: f32`; `QueuedSample::Hit(Hit, f32)`.
- `queue_position(pos, xf, pressure)`; `drag` / `release` pass `p.pressure`. `press_3d` paints the first sample with `p.pressure` and sets `last_pressure`.
- `resolve`: for a step of length `s` towards a target at distance `len`, `pressure = if next == target { target_p } else { last_p + (target_p - last_p) * (s / len) }`, then `st.last_pressure = pressure`. For a mouse this is `1 + 0 * f == 1.0` exactly.
- `resolve` step length: `(0.25 * st.ring_px * factor).max(1.0)` with `factor = dynamics.size_factor(pressure at the step's end)`. A thin pen stroke then gets surface samples as dense relative to its width as a full-size one; otherwise chords of a curved surface leave gaps. For a mouse `factor == 1.0`: identical raycasts.
- `paint`: `doc.stroke_to_3d_paced(&hit, pressure, &mut more)`.
- 2D path: already passes `p.pressure`; nothing else.

### 3.4 Tests (tools/brush.rs tests, prefs.rs)
1. `a_mouse_stroke_is_unchanged_by_the_pressure_settings`: `drag_path` in 3D and 2D with every combination of the four settings gives the same `paint_hash` (pointer pressure 1.0).
2. `pen_pressure_reaches_the_3d_and_2d_stroke`: add a test helper `pointer_p(state, xf, pos, mods, pressure)`. A 2D drag at 0.25 with Size on paints fewer texels than at 1.0, and a narrower column. Same for 3D by texel count.
3. `pressure_is_interpolated_along_the_walk`: unit test of the `resolve` interpolation (extract a small pure fn `walk_pressure(last_p, target_p, step, len)`).
4. A4-style: a pen stroke with varying pressure is identical with unlimited pace and one dab per frame.
5. prefs: extend `a10_brush_preferences_round_trip_and_read_leniently`. New fields round-trip; a file without them gives the defaults (Size on, 10 %, Flow off); wild values are clamped; `tablet_input` round-trips; unknown word → `WindowsInk`.

**Check B:** workspace tests (only WA20 red), clippy 0. Commit "app: brush pressure settings and pressure through the stroke queue", push.

## 4. Part C — pen module, pure parts and wiring (`crates/wte_app/src/pen/`)

Files:
- `pen/mod.rs`: types, `PenState`.
- `pen/decode.rs`: pure flag decoding.
- `pen/route.rs`: pure event router.
- `pen/win.rs`: Windows glue (part D).
- `pen/wintab.rs`: part G.

### 4.1 Types
```rust
/// What the window procedure saw for one history entry of a pen pointer message (or one synthetic test record).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PenRecord {
    pub pointer_id: u32,
    pub kind: PenKind,        // Down, Update, Up, Leave (left range / capture lost)
    pub in_contact: bool,
    pub pressure: f32,        // 0..=1; 1.0 for a pen without pressure
    pub barrel: bool,         // PEN_FLAG_BARREL or POINTER_FLAG_SECONDBUTTON
    pub third: bool,          // POINTER_FLAG_THIRDBUTTON
    pub inverted: bool,       // PEN_FLAG_INVERTED or PEN_FLAG_ERASER
}
/// Rebuilt by every `raw_input_hook`.
pub struct PenFrame {
    pub tagged: Vec<(egui::Pos2, f32)>, // pen-made PointerMoved / PointerButton events of this frame, in order, with pressure
    pub in_range: bool, pub in_contact: bool, pub inverted: bool, pub barrel: bool,
    pub pressure: f32,                  // of the last contact sample; 1.0 without contact
}
pub struct PenState {                   // AppState.pen
    pub feed: VecDeque<PenRecord>,      // drained from the window procedure's queue, plus the self-test's records
    pub router: PenRouter,              // persists across frames
    pub frame: PenFrame,
    pub eraser_restore: Option<ToolKind>,
    pub seen: bool,
}
```

### 4.2 `decode.rs` (pure)
`RawPen { pointer_flags: u32, pen_flags: u32, pen_mask: u32, pressure: u32 }` → `PenRecord` fields.
- Constants: `POINTER_FLAG_INRANGE 0x2`, `INCONTACT 0x4`, `FIRSTBUTTON 0x10`, `SECONDBUTTON 0x20`, `THIRDBUTTON 0x40`, `CANCELED 0x8000`, `DOWN 0x10000`, `UPDATE 0x20000`, `UP 0x40000`, `CAPTURECHANGED 0x200000`; `PEN_FLAG_BARREL 1`, `INVERTED 2`, `ERASER 4`; `PEN_MASK_PRESSURE 1`.
- `kind`: DOWN → Down, UP → Up, UPDATE → Update, else `None` (winit skips those entries too).
- `pressure`: without `PEN_MASK_PRESSURE` → 1.0; with it → `raw.clamp(1, 1024) / 1024` in contact, 0.0 not in contact.
- Tests: tip down; hover; barrel hover; barrel + contact (SECONDBUTTON); inverted hover; eraser contact; no pressure mask; pressure 0 in contact → 1/1024; pressure > 1024 clamps; an entry with no DOWN/UP/UPDATE → `None`.

### 4.3 `route.rs` (pure): `PenRouter::process(&mut self, events: &mut Vec<egui::Event>, feed: &mut VecDeque<PenRecord>) -> PenFrame`

Walk `events` in order and build the new list. For each `Event::Touch { id, phase, pos, force, .. }`:

1. **Is it a pen?** Yes if the feed holds a record with `pointer_id == id`, or the id is in `router.known_pen_ids` (small set, updated from the feed, cleared for an id on Leave). Otherwise it is a finger: copy it and its companions unchanged.
2. **Match a record.** Pop feed records of that id until one whose kind matches the phase (Started ↔ Down, Moved ↔ Update, Ended ↔ Up, Cancelled ↔ Up or Leave). Skipped records still update the pen state.
   - No match: build a fallback record: pressure `force.unwrap_or(1.0)` in contact, flags as last known.
   - This resyncs at every stroke start and end, so a skipped winit entry costs at most one sample.
3. **Drop the Touch event** (D1).
4. **Companions.** The events egui-winit put right after it are handled as follows.
   - `Started`: expect `PointerMoved(pos)` then `PointerButton{Primary, pressed: true}`. The button becomes `Secondary` if the record has `barrel`, else `Middle` if `third`, else it stays `Primary`. Remember it as `router.contact = Some((id, button))`. Tag both events with the record's pressure.
   - `Moved`: expect `PointerMoved(pos)`. Tag with the pressure while in contact; hover moves are not tagged.
   - `Ended`: expect `PointerButton{Primary, pressed: false}` then `PointerGone`. Rewrite the button to the remembered one; clear `router.contact`.
   - **Missing companions** (egui-winit's emulation is stuck on an older id): synthesize exactly the events listed above, with the modifiers of the last `PointerButton` / `ModifiersChanged` seen, or `Modifiers::NONE`. Unit-test this; it is what keeps the pen alive after a lost `Ended`.
5. Feed records of kind `Leave` (they have no Touch event):
   - If `router.contact` is that id (capture lost during contact): append `PointerButton{remembered button, pressed: false}` at the last pen position and `PointerGone`; clear the contact.
   - If not in contact: append `PointerGone` only when the last pointer event in this frame's list was pen-made, so hover highlights go away when the pen leaves.
   - Forget the id in `known_pen_ids`.
6. `PenFrame`: `tagged` as built; `in_range / in_contact / inverted / barrel / pressure` from the last record seen (state persists in the router across frames); `seen` set once.
7. Cap the feed at 8192 records (drop oldest) so it cannot grow when frames are not running.

All non-touch events pass through untouched and in order. A frame without pen touches and with an empty feed returns the event list bit-for-bit unchanged: mouse-only behaviour and all existing scenarios are unaffected.

Router tests:
- tap (Down + Up in one frame) → PointerMoved, Primary press, Primary release, PointerGone, and no Touch left;
- a move burst of N samples with N different pressures → N tags in order;
- barrel tap → Secondary press and release; third button → Middle;
- eraser hover then contact → `inverted` true and the button stays Primary;
- finger touch (no record) → events unchanged, Touch kept;
- record / touch count mismatch → resync, no panic, fallback pressure from `force`;
- stuck emulation (Touch events without companions) → synthesized press, moves, release;
- Leave during contact → release + PointerGone;
- Leave while hovering → PointerGone;
- mouse-only event list → identical output.

### 4.4 Wiring
- `WteApp::raw_input_hook`:
  1. self-test hook if present. It returns the injected `Vec<PenRecord>` of the frame, pushed into `state.pen.feed`.
  2. else (interactive) `pen::win::drain_into(&mut state.pen.feed)` (part D).
  3. `state.pen.frame = state.pen.router.process(&mut raw.events, &mut state.pen.feed)`.
- `WteApp::logic`, before the tools and panels run: `pen::apply_eraser_override(app)`.
  - When `frame.inverted && frame.in_range`, no override is active and the tools are not busy: remember `state.tool` in `eraser_restore` and run `A::SetTool(ToolKind::Eraser)` through the normal dispatch (skip if the tool is already Eraser).
  - When an override is active and the pen is seen non-inverted, or has left range, or a non-pen `PointerButton` press is in the frame, and the tools are not busy: if `state.tool` is still `Eraser`, `A::SetTool(previous)`; clear the override either way.
  - A manual tool change during the override simply drops it. Test this.
  - The hook runs before `logic` in the same frame, so a pen that comes down inverted erases from its first sample.
- `view/mod.rs`: `route_pointer` and `pointer_at` take the pressure.
  - Add `PenFrame::cursor()` returning a small walker with `take(pos) -> f32`. If the next untaken tag's position equals `pos` bit-for-bit, return its pressure and advance; else return 1.0.
  - `route_pointer` creates one walker per call and asks it for every `PointerMoved` / `PointerButton` event it sees. Each view has its own walker, so Split works.
  - The "drag without motion this frame" call and the hover use `if frame.in_contact { frame.pressure } else { 1.0 }`.
  - `tools/mod.rs` 695 (test helper) keeps 1.0.
  - Nothing else in the routing changes. A barrel press reaches `ToolController::press` as `Secondary` (orbit in 3D, pan in 2D) through the existing rules. Layer-row context menus open through egui's own secondary click.
- A pen tap in a view paints one dab (the press paints the first sample), as a mouse click does.

### 4.5 Self-test hook and scenario `pen` (ARCHITECTURE 8.11)
- `Step::Pen { view, samples: Vec<PenPoint>, inverted: bool, barrel: bool, points_per_frame: usize }` with `PenPoint { at: [f32; 2], pressure: f32 }`.
  - Pressure 0 = hover sample. The first sample with pressure > 0 is the Down, the last contact sample is followed by an Up, and a `Leave` record ends the step.
- `FrameInput` gains `pen: Vec<PenRecord>`. `pen_frames(...)` builds per frame exactly what egui-winit would push (Touch with `force`, PointerMoved, PointerButton, PointerGone) plus the matching records. This is the recorded-pen-state test hook: in-process, no OS input.
  - Use a fixed `TouchId` / `pointer_id` (e.g. 77) and a fixed `TouchDeviceId`.
- Scenario `pen` on the built-in sample model the `paint3d` / `paint2d` scenarios use. Add it to `NAMES` and update any test or doc that counts or lists the names. Steps, each with `Assert`s on layer pixels read from the document:
  1. Set brush: size 40 texels, hardness 1, flow 1, pressure Size on (min 0), Flow off.
  2. **2D ramp:** `Pen` straight horizontal stroke in `Main2D`, pressure 0.1 → 1.0. Assert the painted column height near 20 % of the stroke is less than half the height near 90 %, and the height near the end is 40 ± 3 texels.
  3. **2D identity:** undo; draw the same path with `Step::Pointer` (mouse) and remember the hash; undo; draw it with `Pen` at pressure 1.0 and the same `points_per_frame`. Assert equal hashes.
  4. **2D flow:** Size off, Flow on (min 0), brush flow 0.5, constant-pressure strokes at 0.2 and at 1.0 on two rows. Assert centre alpha of the first is below the second and above 0.
  5. **3D size:** `Pen` stroke in `Main3D` at constant pressure 0.25, count changed texels; undo; the same at 1.0. Assert `0 < count(0.25) < 0.5 * count(1.0)`. Then the 3D identity as in step 3 (mouse vs pen at 1.0).
  6. **Eraser end:** paint a patch; `Pen { inverted: true }` with two hover samples, then a stroke across the patch. Assert alpha is 0 in the stroke centre and the tool was Eraser during it (record `state.tool` in `Scratch` with a `Setup` between frames, or assert on the history label "Erase"). After the step and one `WaitFrames`, assert `state.tool == Brush`.
  7. **Barrel:** `Pen { barrel: true }` drag in `Main3D`. Assert the camera changed and the paint hash did not.
  8. **Tap = click:** `Pen` with one contact sample on a Layers row or toolbar button (position from `state.panel_rects`, as `ScreenPointer` users do; add a `ScreenPen` variant only if the view-relative step cannot reach panels). Assert the selection or tool changed.
  9. Assert the status readout text was "Pen …" during a contact frame (capture it in `Scratch`). `Screenshot`.
- Existing scenarios inject no pen records: their event lists pass the router unchanged.

**Check C:** workspace tests, clippy 0, release build. Run `--scenario pen` with output under `target\pen\` and require `"ok": true`. Rerun `smoke default paint3d paint2d stamp text wrap_surface brush_types brush_tab layers undo`, all `"ok": true`. Commit "pen: router, eraser end, barrel button, pressure into the views; scenario pen", push.

## 5. Part D — Windows glue (`pen/win.rs`, `#[cfg(windows)]`)

Installed from `WteApp::new` only when the run is **not** a self-test (`--screenshot`) and not `--gpu-check`.

### 5.1 HWND
`use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};` then `cc.window_handle().ok().map(|h| h.as_raw())`; `RawWindowHandle::Win32(h)` gives `h.hwnd.get()`. Any failure logs a warning and the app runs without pen extras; the router's fallback still gives a working pen with winit's pressure.

### 5.2 Subclass
- `SetWindowLongPtrW(hwnd, GWLP_WNDPROC = -4, pen_wndproc)`. Keep the previous procedure in a `static PREV: AtomicIsize`. `pen_wndproc` always ends with `CallWindowProcW(prev, hwnd, msg, wparam, lparam)` and returns its result unchanged. On `WM_NCDESTROY`, put the old procedure back first.
- User32 only: no new DLL, no crate. winit keeps its own state in `GWL_USERDATA` and its class procedure, so replacing `GWLP_WNDPROC` does not disturb it.
- `WteApp::new` runs on the window's thread, as required.
- Messages:
  - `WM_POINTERDOWN 0x0246`, `WM_POINTERUPDATE 0x0245`, `WM_POINTERUP 0x0247`: `id = LOWORD(wparam)`. If `GetPointerType(id) == PT_PEN (3)`: call `GetPointerPenInfo` for `historyCount`, then `GetPointerPenInfoHistory(id, &mut n, buf)` into a `Vec` of that size. Push the entries **oldest first** (the API returns newest first) through `decode` into the queue.
  - `WM_POINTERLEAVE 0x024A` and `WM_POINTERCAPTURECHANGED 0x024C` for a pen id: push a `Leave` record.
  - `WM_TABLET_QUERYSYSTEMGESTURESTATUS 0x02CC`: return the flags of 6.2 directly (the one message not forwarded).
- Shared state: `static QUEUE: Mutex<VecDeque<PenRecord>>` (const-constructed, cap 8192).
  - The window procedure and `raw_input_hook` both run on the UI thread, so the lock is never contended. A Mutex keeps it sound if that ever changes.
  - Use `lock().unwrap_or_else(PoisonError::into_inner)`.
  - Wrap the whole body of `pen_wndproc` before the forwarding call in `std::panic::catch_unwind`: a panic must never cross the FFI boundary.
- `drain_into(feed)` moves the queue's records into `PenState.feed`.
- Structs, `#[repr(C)]`. Add `const _: () = assert!(size_of::<PointerPenInfo>() == 120)` under `cfg(target_pointer_width = "64")`.
  - `POINTER_INFO { pointerType u32, pointerId u32, frameId u32, pointerFlags u32, sourceDevice isize, hwndTarget isize, ptPixelLocation POINT, ptHimetricLocation POINT, ptPixelLocationRaw POINT, ptHimetricLocationRaw POINT, dwTime u32, historyCount u32, InputData i32, dwKeyStates u32, PerformanceCount u64, ButtonChangeType i32 }`
  - `POINTER_PEN_INFO { pointerInfo, penFlags u32, penMask u32, pressure u32, rotation u32, tiltX i32, tiltY i32 }`
- FFI (`#[link(name = "user32", kind = "raw-dylib")] unsafe extern "system"`): `SetWindowLongPtrW`, `CallWindowProcW`, `GetPointerType`, `GetPointerPenInfo`, `GetPointerPenInfoHistory`, `SetWindowFeedbackSetting`, `SetPropW`, `GlobalAddAtomW` (kernel32). All exist on Windows 8+.

### 5.3 What the builder can and cannot test here
- Unit-testable: struct sizes, `decode`, queue cap and drain, oldest-first ordering (a pure helper `push_history(newest_first: &[RawPen], ..)`).
- Not testable without a pen: the message path itself.
- Do test that installing the subclass on a real window does no harm. Add env switch `WTE_PEN_SUBCLASS=1` that installs it in a self-test run too, and run `smoke` and `pen` once with it, expecting `"ok": true`. The window stays off-screen and unfocused; this is not input injection. Document the switch in 8.11.

**Check D:** tests, clippy 0, release build; scenarios `smoke`, `pen`, `paint3d` with and without `WTE_PEN_SUBCLASS=1`. Commit "pen: Windows pointer subclass (flags, per-sample pressure), Ink feedback off", push. (Section 6 goes in the same commit.)

## 6. Windows Ink feedback and gestures (in part D)

### 6.1 `SetWindowFeedbackSetting`
Call `SetWindowFeedbackSetting(hwnd, kind, 0, 4, &FALSE)` for:
- `FEEDBACK_PEN_BARRELVISUALIZATION 2`
- `FEEDBACK_PEN_TAP 3`
- `FEEDBACK_PEN_DOUBLETAP 4`
- `FEEDBACK_PEN_PRESSANDHOLD 5`
- `FEEDBACK_PEN_RIGHTTAP 6`

Leave touch feedback as it is. Log failures, never fail startup.

### 6.2 System gestures (what other drawing apps and Qt also do)
- Answer `WM_TABLET_QUERYSYSTEMGESTURESTATUS` with `TABLET_DISABLE_PRESSANDHOLD 0x1 | TABLET_DISABLE_PENTAPFEEDBACK 0x8 | TABLET_DISABLE_PENBARRELFEEDBACK 0x10 | TABLET_DISABLE_FLICKS 0x10000`.
- Set the window property `"MicrosoftTabletPenServiceProperty"` (`SetPropW`) to the same value.
- Effect: no press-and-hold ring or right-click delay, no ripple on tap, no flicks while painting.

### 6.3 Also relevant, no code needed (say so in ARCHITECTURE 8.13)
- winit already makes the process per-monitor DPI aware.
- winit reads the full pointer history, so no samples are lost.
- winit does not pass pointer messages to `DefWindowProc`, so Windows does not create duplicate mouse messages for the pen.
- `EnableMouseInPointer` is not used and must not be.

## 7. Part E — readout and help

- **Status bar** (`panels/status_bar.rs`): a new item with the lowest priority in `StatusPlan` (dropped first). Shown only while `state.pen.frame.in_range`:
  - "Pen 47 %" in contact,
  - "Pen" hovering,
  - "Pen (eraser)" when inverted.
  - Nothing is shown without a pen, so the bar is unchanged for mouse users and existing scenarios. Extend the plan tests (`status_items_never_overlap`, the wide/narrow test) with the item present.
- **Brush tab line**: see 3.2.
- **Help > Tablet setup** (`Dialog::TabletHelp`, new `dialogs/tablet.rs`, menu entry between Shortcuts and About; update `control_sweep`). Text:
  - Works with any tablet whose driver has **Windows Ink** turned on (the default): Wacom Tablet Properties > Mapping > "Use Windows Ink"; Huion > Pen settings > "Enable Windows Ink"; XP-Pen / Pentablet > "Windows Ink".
  - If the stroke does not follow pressure: turn that option on (drivers allow it per application: add Wrapped Texture Editor to the driver's application list), or choose Tablet input: WinTab (part G).
  - Pressure options are in the Brush tab under "Pen pressure". The pressure curve (soft/firm) is set in the tablet driver.
  - Turn the pen around to erase. The lower pen button is the right mouse button (orbit in 3D, pan in 2D, context menus). Other buttons do what the driver maps them to (middle click pans, keys work as shortcuts).
  - The status bar shows "Pen N %" while the pen touches the tablet.
- The "Tablet input" combo (Windows Ink / WinTab) lives in this dialog. It is shown once part G lands.
- **README.md**: a short "Drawing tablets" section with the same content in five lines.
- **`dist/HOW_TO_TEST.txt`**: the hand-test list of section 12.

**Check E:** tests, clippy; scenario `pen` and a scenario or control-sweep test that opens the new dialog. Commit, push.

## 8. Part G — WinTab, opt-in (last, own commit)

Used only when `prefs.tablet_input == WinTab`, in interactive runs only. Never loaded otherwise. Everything in `pen/wintab.rs`.

- **Loading:** `LoadLibraryW("wintab32.dll")` + `GetProcAddress` for `WTInfoW`, `WTOpenW`, `WTClose`, `WTPacketsGet`, `WTEnable`, `WTOverlap`. Must be run-time loading: the DLL is absent without a tablet driver, and a `raw-dylib` import would stop the exe from starting. Missing DLL or function gives the toast "No WinTab tablet driver found; using Windows Ink" and behaves as Ink.
- **Context:**
  - `WTInfoW(WTI_DEFSYSCTX = 4, 0, &mut LOGCONTEXTW)` (struct 212 bytes; assert the size).
  - `lcOptions |= CXO_MESSAGES (0x4)`, keeping `CXO_SYSTEM`: the driver keeps moving the system cursor and making mouse clicks.
  - `lcPktData = lcMoveMask = PK_CURSOR 0x20 | PK_BUTTONS 0x40 | PK_NORMAL_PRESSURE 0x400`, `lcPktMode = 0`, `lcMsgBase = 0x7FF0`.
  - `WTOpenW(hwnd, &lc, TRUE)`.
  - Pressure range from `WTInfoW(WTI_DEVICES = 100, DVC_NPRESSURE = 15, &mut AXIS)`; `axMax <= 0` → 1023.
- **Packets:** the subclass of part D handles `WT_PACKET 0x7FF0` (`lParam` = context) by draining `WTPacketsGet(ctx, 64, buf)` into the last state: `{ cursor: u32, buttons: u32, pressure: u32 }` (12-byte packet for that mask).
  - `WT_PROXIMITY 0x7FF5` sets `in_range` from `LOWORD(lParam)`.
  - On `WM_ACTIVATE`: `WTEnable(ctx, active)` and, when active, `WTOverlap(ctx, TRUE)`.
  - `WTClose` on `WM_NCDESTROY` and when the pref changes back.
- **Pure, unit-tested part:** `wintab_state(packets: &[Packet], pressure_max: u32, in_range: bool) -> WintabPen { in_range, tip_down: buttons & 1 != 0, pressure: p / max clamped, inverted: cursor % 3 == 2 }`.
  - `spread(prev: f32, now: f32, n: usize) -> Vec<f32>` gives linearly spaced pressures for the n pointer-move events of a frame, last = `now`.
- **Use in the router:** when a WinTab context is open and `tip_down`, the frame's pointer events that are not Ink-pen-made are tagged with `spread(prev_frame_pressure, pressure, n)` (press events with the first value). `PenFrame.in_contact / pressure / inverted / in_range` come from the WinTab state. Eraser override and readout work as for Ink.
- **No double input:**
  - Positions and buttons always come from the single egui pointer stream; WinTab never creates pointer events.
  - If the driver delivers both Ink pointer messages and WinTab packets, the Ink path still makes the events (D1). In WinTab mode with `tip_down`, the WinTab pressure replaces the Ink pressure of the tags. Without packets, the Ink pressure is used, so a wrong pref still works.
  - Barrel buttons are not read from WinTab; the driver's own mouse-button mapping acts through `CXO_SYSTEM`.
- **UI:** the combo "Tablet input: Windows Ink (recommended) / WinTab" in Help > Tablet setup, with the line "Use WinTab only if your driver's Windows Ink option is off. Takes effect at once." Opening or closing the context happens on change.
- **Tests:** struct sizes; `wintab_state` (tip, eraser cursor, pressure scaling, zero max); `spread`; router tests with a synthetic WinTab state (mouse events get pressures; an Ink pen plus WinTab gives one stroke's worth of events with WinTab pressure; no WinTab state leaves mouse events untagged); prefs round trip.
- The FFI path cannot be exercised without a driver: say so in the notes and the hand-test list.

**Check G:** tests, clippy 0, release build; scenarios `smoke pen paint3d paint2d` ok. Commit "pen: opt-in WinTab pressure source", push. If something in G cannot be made sound, leave G out entirely (the combo is not shown), keep the pref field, and write the reason in `docs/notes/pen/wintab.md` and in the Help text ("turn Windows Ink on in the driver").

## 9. Documentation edits (in the same commits as the code they describe)

**ARCHITECTURE.md**
- New 1.10 "Pen input (branch `pen-input`)": the decisions table of section 1 and the traceability.
- 2.2: the `pen/` files and `dialogs/tablet.rs`.
- 3.3: no new dependency; `eframe::wgpu::rwh`.
- 5.19: `PressureDynamics`, `BrushParams.dynamics`, the two factor functions, `OpenSegment.var`.
- 6.4 "Spacing and interpolation": the pressure-sized segment rule, the walk, the guard, the identity guarantees.
- 8.2: `PenState`, `BrushSettings` fields, `Prefs.tablet_input`.
- 8.3: order inside `raw_input_hook` and the eraser override in `logic`.
- 8.5 / 8.6: pressure pairing in `route_pointer`, the barrel remap, the brush queue with pressure, the `resolve` step.
- 8.7: Pen pressure group, status item, Help entry.
- 8.8: Tablet setup dialog.
- 8.9: prefs.
- 8.11: `Step::Pen`, scenario `pen`, `WTE_PEN_SUBCLASS`.
- 8.13 pitfalls: never pass pointer messages to `DefWindowProc`; never call `EnableMouseInPointer`; nothing may panic in the window procedure; pen Touch events must not reach egui.
- 9.1 / 9.2: the new tests.
- 10: a "pen-input" work-package list.

**docs/plans/FEATURE_BRUSH.md** (~373, ~2120): mark pen pressure as done, pointing to this plan.

**HANDOFF.md**: a short "Pen input" entry: state, the hand tests still owed by the user, WinTab status.

**README.md, dist/HOW_TO_TEST.txt**: section 7 and section 12.

**docs/notes/pen/**: `<part>.md` per part: what was built, deviations, anything unclear and the choice made.

## 10. Work order

| Step | Content | Check before moving on |
|---|---|---|
| A | Core dynamics, per-dab radius, variable spacing | `test -p wte_core` green, existing hashes untouched, clippy 0 |
| B | Settings, UI, prefs, pressure through the brush tool | workspace tests (WA20 only red), clippy 0 |
| C | `pen` module pure parts, wiring, eraser override, `Step::Pen`, scenario `pen` | tests, clippy, release build, `pen` + regression scenarios ok |
| D | Windows subclass, feedback / gesture settings | tests, clippy, build, scenarios with and without `WTE_PEN_SUBCLASS=1` |
| E | Status item, Help dialog, README, HOW_TO_TEST | tests, clippy, scenario `pen` |
| F | Final doc pass (ARCHITECTURE, HANDOFF, FEATURE_BRUSH), notes | full `test --workspace --no-fail-fast` = baseline + new tests, 1 failed (WA20); clippy 0; release build; every scenario in the list below ok |
| G | WinTab opt-in | as F, own commit |

Final scenario list: `pen smoke default paint3d paint2d stamp text wrap_surface brush_types brush_tab layers undo multi`, each `"ok": true`, outputs in `target\pen\`. Check the exe exists before starting it.

Report at the end: test counts, clippy, exe SHA-256, scenario results, commit list, what was not verifiable.

## 11. Risks

| Risk | Mitigation |
|---|---|
| A changed default silently alters mouse strokes | Core default = old behaviour; factors are exactly 1.0 at pressure 1; constant-step code path untouched; identity tests in core, tool and scenario (mouse hash == pen at 1.0) |
| Record ↔ Touch matching drifts | Resync at Down / Up, fallback to winit's `force`, feed cap; unit tests with mismatched counts |
| egui-winit emulation stuck after a lost `Ended` | Router synthesizes companions and releases on `Leave`; unit tests |
| Subclass breaks the window or panics in FFI | Always forwards unchanged; `catch_unwind`; restore on `WM_NCDESTROY`; `WTE_PEN_SUBCLASS=1` scenario runs |
| Pen tip jitter exceeds egui's click distance, so taps miss buttons | Hand test 3. If reported, raise `input_options.max_click_dist` for pen contacts only (set per frame from `PenFrame.in_contact`) |
| Eraser override fights tools with drafts (Stamp / Text placement) | Uses the same `SetTool` path as the E shortcut, only when tools are idle; a manual tool change drops the override |
| Thin strokes on curved surfaces get gaps | `resolve` step scales with `size_factor` |
| Many dabs at low pressure with size on are slow | Step floor of half a texel, `MAX_DABS_PER_SAMPLE` guard, existing frame pacing |
| Brush tab gets taller, layout asserts or screenshots change | Expected; update only assertions that count rows; painted-pixel asserts of other scenarios must not change |
| WinTab driver quirks | Opt-in, pressure and eraser only, falls back to Ink pressure when no packets, own commit |
| No hardware test by the builder | Section 12; the status readout lets the user confirm in seconds |

## 12. What the user must try with a real tablet (goes into `dist/HOW_TO_TEST.txt`)

1. Hover over the model: the brush ring follows the pen and the status bar shows "Pen".
2. Paint in 3D and in 2D, light to hard: the stroke goes thin to thick and the status bar shows the percentage. Fast curved strokes are smooth, not polygonal.
3. Tap toolbar buttons, layer rows, tabs, sliders and menus with the pen: each tap is one click. Tooltips appear when hovering.
4. Brush tab > Pen pressure: turn Size off and Flow on: width is constant and paint builds with pressure. Min size 50 %: light strokes are half width.
5. Turn the pen around: the tool switches to Eraser and erases; turn it back: the previous tool returns.
6. Hold the lower pen button and drag: 3D orbits, 2D pans; on a layer row it opens the context menu.
7. Press and hold the pen still on the model: no ring animation, no right-click menu, no ripple on taps.
8. Restart the app: the pressure settings are remembered.
9. If the tablet driver's Windows Ink option is off: confirm there is no pressure, then Help > Tablet setup > Tablet input: WinTab, and check that pressure works and each stroke appears once.
10. Mouse painting feels and looks as before.
