# Part A — core: pressure dynamics and per-dab radius

Commit ce4c008 "core: pressure dynamics (size, flow) with per-dab radius".

Built (`wte_core/src/brush.rs`, `stroke.rs`):

- `PressureDynamics { size, size_min, flow, flow_min }`, `Default` (size off, flow on from 0 = round 3), `OFF`,
  `size_factor`, `flow_factor`, `sanitized`; the exact formulas of plan 2.1. `BrushParams.dynamics`, default
  `PressureDynamics::default()`. `sanitize` keeps `dynamics.sanitized()`.
- `surface_dab`, `surface_dab_legacy`, `texture_dab`: radius `brush.radius * size_factor(p)`, strength
  `brush.flow * flow_factor(p)`. With the default dynamics both are bit for bit the old values (`x * 1.0 == x`,
  `flow_factor(p) == p`).
- Checked before relying on a per-dab radius: `shared_candidates` recomputes `aa`, `rho1`, `rho` from `shape.reach` on
  every call; `dab_tip` takes the shape per dab; the round-7 brush index (`prepare_brush_index`) is per map and has no
  radius; `TipGeom::new(field, through)` has none. Nothing was cached per stroke by radius; nothing had to be keyed.
- Pressure-sized segments (plan 2.3): `pressure_sized`, `step_at`, `var_bound`; `OpenSegment.var: Option<VarWalk>`;
  `run_var_segment` (paced, 3D) and the inline 2D walk. The `None` branch of `run_segment` and the 2D constant-step loop
  are the old code, unchanged.
- `tests/end_to_end.rs` `random_brush` builds a full `BrushParams` literal: it gained `dynamics:
  PressureDynamics::default()` (a compile fix; its values are unchanged).

Tests (stroke.rs, plan 2.4): `size_and_flow_factors`, `full_pressure_ignores_the_dynamics`, `pressure_sizes_the_stroke_2d`,
`pressure_spacing_follows_the_radius`, `pressure_sized_stroke_is_pacing_independent`,
`pressure_sized_stroke_is_thread_independent`, `eraser_follows_pressure_size`, `flow_min_and_size_min_are_sanitized`,
`too_many_pressure_dabs_fall_back_to_one` (9 tests; plan item 3 is the unchanged existing hashes: C1, C2,
`tests\brush_tips.rs`, `one_dab_per_call_gives_the_unpaced_stroke`, `pressure_scales_the_flow` all pass with no edits).

Check A: `cargo test --workspace` (with part B) 1,307 passed, 1 failed (WA20, known), 48 ignored; clippy 0 warnings.

Deviation: ARCHITECTURE 5.19 / 6.4 were updated in the part C commit, not in this one.
