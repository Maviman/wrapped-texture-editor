# Part B — app: pressure settings and pressure through the stroke queue

Commit cda14ea "app: brush pressure settings and pressure through the stroke queue".

Built:

- `state.rs`: `BrushSettings` gains `pressure_size` (true), `pressure_size_min` (0.10), `pressure_flow` (false),
  `pressure_flow_min` (0.0); `sanitized` clamps both minimums to 0..=1, a non-finite value becomes the default;
  `BrushSettings::dynamics()`. `TabletInput { WindowsInk (default), WinTab }` (snake_case serde) and
  `Prefs::tablet_input` with a lenient reader (`lenient_prefs_serde::tablet_input`: anything that does not read is
  Windows Ink, as `upper_tab` does).
- `panels/tool_options.rs`: under the brush's Flow and Spacing rows a group "Pen pressure": checkbox Size + slider
  "min N %" (enabled when Size is on), checkbox Flow + slider "min N %"; hover texts of plan 3.2. Brush and Eraser each
  show their own settings. The pen line under it came with part C (`PenState::status_line`).
- `tools/brush.rs`: `QueuedSample::Hit(Hit, f32)`, `unresolved: VecDeque<(Pos2, ViewXform, f32)>`,
  `ActiveStroke { last_pressure, dynamics }`; `queue_position(pos, xf, pressure)`; the press paints its hit with
  `p.pressure`; `resolve` interpolates with `walk_pressure` and shrinks its step with
  `dynamics.size_factor(min(last_p, target_p))`; `paint` passes the sample's pressure to `stroke_to_3d_paced`;
  `BrushTool::params` fills `dynamics`.
- `tools/mod.rs` test support: `pointer_p(state, xf, pos, mods, pressure)` (`pointer` = `pointer_p(.., 1.0)`).

Tests: `a_mouse_stroke_is_unchanged_by_the_pressure_settings` (12 combinations, 3D and 2D, one hash each),
`pen_pressure_reaches_the_3d_and_2d_stroke`, `pressure_is_interpolated_along_the_walk`,
`a_pen_stroke_does_not_depend_on_the_pacing` (unlimited vs one dab per frame vs one raycast per frame);
`prefs::a10_brush_preferences_round_trip_and_read_leniently` extended (round trip of the four fields and
`tablet_input`, defaults from an old file, wild and non-finite values, unknown / wrong-typed `tablet_input`).

Check B: workspace tests 1,307 passed / 1 failed (WA20) / 48 ignored (baseline 1,294 / 1 / 48: +13 new); clippy 0.

Deviations:
- The resolve step uses the thinner end of the walk (see `decisions.md`).
- The plan's 3.4 test 1 says "every combination of the four settings": the two booleans in all four combinations, each
  with three pairs of minimums (12 settings).
- ARCHITECTURE 8.2 / 8.6 / 8.9 were updated in the part C commit.
