# Pen input — build notes, measurements and comparisons (branch `pen-input`)

Builder: one agent, parts A-G of `docs/plans/PEN_INPUT_PLAN.md` in order, each its own commit, pushed after each.
Per-part details: `A.md` ... `G.md`; decisions and deviations: `decisions.md`.

## Baseline (main @ 1b589a1)

- Release exe built from 1b589a1 before any code change, kept as `target\pen\baseline\wte_base.exe`
  (SHA-256 a41924c102aed4a5ee0aac1fc6375453b5380a4a8420c34c174d3d0d16223f29).
- Baseline sweep `target\pen\baseline\runs` with `tools/objtabs/sweep.sh` and `tools/pen/jobs_regress.txt`: smoke default
  paint3d paint2d stamp text wrap_surface brush_types brush_tab brush_shape brush_hover layers undo multi pan2d export
  export_run export_flow side_tabs objects, and brush_types_path / workflow_path on
  `G:\Claude_Resources\samples\khronos-gltf\BoxTextured.glb` — 22 runs, all `"ok": true`.
- Test baseline (from the task): 1,294 passed / 1 failed (WA20) / 48 ignored; clippy 0.

## Commits

| Commit | Part |
|---|---|
| 0558d20 | plan saved verbatim (`docs/plans/PEN_INPUT_PLAN.md`) |
| ce4c008 | A: core pressure dynamics, per-dab radius, variable spacing |
| cda14ea | B: brush pressure settings, pressure through the stroke queue, prefs |
| 062b276 | C: pen module, router, eraser end, barrel button, views, `Step::Pen`, scenario `pen` (+ ARCHITECTURE for A-C) |
| dbb55f9 | D: Windows subclass, Ink feedback / gestures off |
| e769f96 | E: status read-out, Help > Tablet setup, README, HOW_TO_TEST |
| (F) | final doc pass: ARCHITECTURE 9.1 / 9.2 / 10, HANDOFF, FEATURE_BRUSH, these notes |
| (G) | WinTab opt-in, see `G.md` |

## Final checks (part F, before part G)

- `cargo test --workspace --no-fail-fast`: **1,331 passed, 1 failed (WA20, the known red), 48 ignored** (baseline 1,294 /
  1 / 48: +37 new tests: core 9, app brush 4, pen decode 4, router 11, pen 4, win 4, selftest 1; the prefs and
  status-bar tests were extended, not added).
- GPU test `about_shortcuts_and_message_dialogs_close_by_button_backdrop_and_esc` (`--ignored`): passed.
- `cargo clippy --workspace --all-targets`: 0 warnings.
- Release build: `target\release\wrapped_texture_editor.exe`, SHA-256
  302a087ee268c844e56465060864b4ed548c348be08ec4b030293386351461c2 (before part G).
- Scenario sweep `target\pen\f_runs` (`tools/pen/jobs_final.txt` = `pen` + the 22 baseline jobs; the plan's final list
  `pen smoke default paint3d paint2d stamp text wrap_surface brush_types brush_tab layers undo multi` is a subset):
  23 runs, all `"ok": true`.

## Identity with main (mouse strokes, exports, central view)

`python -E -P tools/pen/compare_runs.py target\pen\baseline\runs target\pen\f_runs`:

- **82 output files byte-identical**: every export of `export`, `export_run`, `export_flow` (layer PNGs, flattened PNG,
  PSD, ORA, the zips of every model format) and the assets the scenarios write. Brush strokes are in these exports, so
  mouse strokes paint the same pixels.
- 3 `.wtep` projects differ only in `manifest.json` `"saved_unix"` (the save time; every other entry of the zips is
  byte-identical, checked by unzipping).
- **28 screenshots pixel-identical**; the others differ only
  - in the side panel rows y 188-276 (1280x800; at 900x600 in `pan2d` the same rows of the narrower panel): the new
    "Pen pressure" group under the Brush / Eraser options (`multi` the same group in another panel state);
  - in `export\shot.png` and `brush_shape\shot_dialog.png` inside a dialog: the output folder path printed there
    (`...\baseline\runs\...` vs `...\f_runs\...`; looked at side by side).
  No pixel of a 3D / 2D view changed.
- Scenario `pen` proves mouse == pen at pressure 1.0 in the real app, 2D and 3D (paint hash).

## Measurements of the scenario `pen` (1280x800, Sphere sample, size 40, hardness 1, Size from 0)

- 2D ramp 0.1 -> 1.0: painted column heights 12 texels at 20 % of the path (expected 2 * 20 * 0.28 = 11.2), 36 at 90 %
  (36.4), 38 at the end (40 +- 3).
- 3D: a stroke at 0.25 covers less than half the texels of the same stroke at 1.0 (assert passed).
- Flow on (min 0), flow 0.5: centre alpha of the 0.2 row below the 1.0 row and above 0.
- Eraser end: centre alpha 0, the Eraser in every contact frame, history label "Erase", the Brush back one frame after
  the Leave. Barrel: camera turned, paint hash unchanged. Pen tap on the Base row: Base active.
- Screenshots looked at: `target\pen\f_runs\pen\shot_pen_ramp.png` (2D stroke thin to thick), `shot_pen_3d.png`
  (a full and a light 3D stroke), `shot.png` (3D ramp thin to thick, the partly erased band of step 6),
  `shot_tablet_help.png` (the dialog).

## Subclass on a real window (part D)

`smoke`, `pen`, `paint3d` without and with `WTE_PEN_SUBCLASS=1` (`target\pen\d_runs\plain`, `...\subclass`): all ok; the
log of every subclassed run says "window procedure subclassed; Ink pen feedback off for 5 of 5 kinds; pen service
property set"; the five screenshots of the two sweeps are pixel-identical. Windows off-screen and unfocused; no input
injection of any kind was used anywhere (pens are recorded samples fed through `raw_input_hook`).

## Not verifiable here (no tablet on this machine)

- The real Windows Ink message path: `GetPointerPenInfoHistory` on real pen messages, the flags a given driver sets,
  coalesced histories, `WM_POINTERLEAVE` / capture changes in practice.
- The effect of `SetWindowFeedbackSetting`, the gesture answer and the pen service property (no ring / ripple / flick).
- Tap accuracy with pen jitter (egui's click distance), tooltips under the pen, the barrel-button mapping of each driver.
- WinTab (part G) with a real driver.
All of these are the hand tests in `dist/HOW_TO_TEST.txt` ("Pen tablets").
