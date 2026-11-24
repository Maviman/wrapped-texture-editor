# pen-input gate report (gate + completeness critic)

Branch `pen-input` (worktree `G:\software_projects\texture_wrapper_ap_ui`), plan `docs/plans/PEN_INPUT_PLAN.md`. The
user's request: "make sure this app is drawing pad compatible". Gate run 2026-11-24 on the gate machine (RTX 5070,
Vulkan, no tablet). Gated: `f751aa4` (builder parts A-G + the checker's fixes), then the gate's own fix `499e321`;
this report is committed after it and changes nothing else.

- Release exe of the gate fix (`499e321`): `target\release\wrapped_texture_editor.exe`, SHA-256
  `94A742DD68A38CA9802ACE9B965EAFE1481960561C1D6A269CAD9173C0E64F5C` (copy `target\pen\gate\wte_gate2.exe`).
- Exe of `f751aa4`: SHA-256 `52E125E957CF8F8C83391145F6AEA8666258E857ECEFE3869F3B1E40BAA5F51C` (copy
  `target\pen\gate\wte_gate.exe`, the same hash as the checker's).
- Baseline: `main` @ 1b589a1, `target\pen\baseline\wte_base.exe`.
- Notes of this gate: `docs/notes/pen/gate.md`. Raw outputs: `target\pen\gate\`.

## Verdict

**PASS**, with one gap the gate cannot close: no part of it has been tried with a real tablet. Every claim that can be
checked without one is met; the hardware-only parts are the hand tests in section 6.

| # | Item | Status |
|---|---|---|
| R1 | `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` fails | known red, the same on `main` |
| R2 | `import_save` on BoxTextured.glb: `"ok": false` ("the source bundle has 0 sidecar(s)") | pre-existing, the same with the baseline exe (OBJECTS_TABS R2: the scenario needs a model with sidecar files; on `coincident_flipped_spheres.obj` it passes) |
| O1 | The Windows Ink message path, the Ink feedback / gesture settings, tap accuracy, tooltips under a pen, the barrel mapping of each driver | hardware only: hand tests 1-7, 11 |
| O2 | WinTab (opt-in): a driver press that arrives before the first tip-down packet paints the first dab at full pressure (builder G.md, checker) | hardware only: hand test 9 |
| O3 | Status-bar "Pen N %" is the first item dropped (by plan), so at 1280 pt with the full hover read-out it is not shown | by design; Help and hand test 1 now point to the Pen line under Pen pressure |
| O4 | Ring / stroke position on a second monitor or with the tablet mapped to part of the desktop | positions are winit's (himetric through `GetPointerDeviceRects`), unchanged by this branch; hand test 1 |

## 1. Workspace tests (`cargo-gnu.cmd test --workspace --no-fail-fast`)

| Commit | Passed | Failed | Ignored | Log |
|---|---|---|---|---|
| `f751aa4` (as handed over) | 1,343 | 1 (WA20) | 48 | `target\pen\gate\test.log` |
| `499e321` (gate fix) | **1,344** | **1 (WA20)** | **48** | `target\pen\gate\test2.log` |

Baseline at 1b589a1: 1,294 / 1 / 48. +50 passing tests (builder 45, checker 4, gate 1), no new failure. Per binary at
`499e321`: `wte_app` 458 / 0 / 24, `wte_core` lib 830 / 1 / 24, `brush_tips` 15, `end_to_end` 9, `objects` 8,
`project_roundtrip` 24. GPU test `control_sweep::gpu_tests::about_shortcuts_and_message_dialogs_close_by_button_backdrop_and_esc`
(`--ignored`, opens Help > Tablet setup and checks the Tablet input combo): passed (`target\pen\gate\test_gpu.log`).

## 2. Clippy

`cargo-gnu.cmd clippy --workspace --all-targets`: **0 warnings** at `f751aa4` (`clippy.log`) and at `499e321`
(`clippy2.log`).

## 3. Scenarios (all of `NAMES`, the path ones on sample models)

Job list `tools/pen/jobs_gate.txt` = the objects-tabs gate list (`tools/objtabs/jobs_gate.txt`, 63 jobs: every name of
`NAMES` without a model, the `_path` scenarios on ABeautifulGame / BoxTextured / Avocado / Lantern, the side panel at
900x600 and 640x480) plus `pen` at 1280x800, 900x600 and 640x480: **66 jobs**. Runner `tools/objtabs/sweep.sh` (off-screen,
no focus, no input injection).

| Exe | ok | not ok | Output |
|---|---|---|---|
| `f751aa4` | 65 | 1 (`import_save_box`, R2) | `target\pen\gate\runs` |
| `499e321` | 65 | 1 (`import_save_box`, R2) | `target\pen\gate\runs2` |
| baseline 1b589a1 (63 jobs, no `pen`) | 62 | 1 (`import_save_box`, R2) | `target\pen\gate\base_runs` |
| `499e321` with `WTE_PEN_SUBCLASS=1`: smoke, pen, paint3d | 3 | 0 | `target\pen\gate\runs2_sub` (log: "window procedure subclassed; Ink pen feedback off for 5 of 5 kinds; pen service property set") |

`pen` passes at all three sizes. Its measurements at `499e321` are those of the builder and the checker: 2D ramp column
heights 12 texels at 20 % (expected 11.2 +- 2), 36 at 90 % (36.4 +- 2), 38 at the end (40 +- 3); 3D light stroke under
half the texels of the full one; mouse == pen at full pressure in 2D and 3D; eraser end erases and the Brush returns;
barrel drag orbits without paint; a pen tap selects a layer; the read-out shows during contact.

## 4. Mouse identity against the baseline (`python -E -P tools/pen/compare_runs.py`)

`base_runs` vs `runs2` (the final exe), 63 common jobs:
- **164 output files byte-identical** (every export of every export scenario: layer PNGs, flattened PNGs, PSD, ORA, the
  zips, model files, assets). The 14 that differ are all `.wtep` projects; unzipped, every entry is identical except
  `manifest.json` `saved_unix` (the save time) and, in `export_nouv` / `import_nouv`, `source.original_path` (the run
  folder). So mouse strokes paint the same pixels.
- **100 screenshots pixel-identical.** The others differ only
  - in the side panel: the new Pen pressure rows under Brush / Eraser options (y 188-276 at 1280x800; at 900x600 and
    640x480 the same rows of the narrower panel, whose left edge is at x 600 / 340, so `compare_runs.py`'s fixed split
    at x 980 labels them "central");
  - in dialogs that print the output folder (`export`, `brush_shape\shot_dialog.png`);
  - in the toast corner (`final_bake_async`, `objects_flow_path_abg`): toasts fade by wall-clock time and these runs
    were slower under load (36 s vs 28 s); looked at side by side (`target\pen\gate\cmp_*.png`).
  **No pixel of a 3D or 2D view changed.**

`runs` (`f751aa4`) vs `runs2` (`499e321`): 164 files byte-identical (14 `.wtep` by save time), 140 screenshots
identical; only `pen*\shot_tablet_help.png` (the new help text) and the same path / toast items differ.

Screenshots looked at: `runs\pen\shot_pen_ramp.png` (2D stroke thin to thick), `runs\pen\shot.png` (3D ramp thin to
thick, the light and the full stroke, the erased band), `runs2\pen\shot_tablet_help.png` and
`runs2\pen_640\shot_tablet_help.png` (six paragraphs, the combo, scrolls at 640x480).

## 5. Completeness critique (what a real tablet user meets)

Checked against the plan and against the sources of winit 0.30.13, egui-winit 0.37, egui 0.37 and
accesskit_windows 0.35.2:

- **Plan items:** A-G all present (the checker's list holds). Plan risk "taps miss buttons with pen jitter": not
  built (by plan only on a report); hand test 3.
- **Fixed here (`499e321`) — tooltips under a still pen.** A hovering pen keeps reporting at the digitizer's rate,
  and egui restarts the tooltip delay on every `PointerMoved`, so no tooltip would ever show under a pen (hand test 3
  expected them). The router now drops a pen's hover moves within 1 pt of the last one passed; contact moves are
  never dropped; mouse frames are untouched. Test `route::tests::a_still_hovering_pen_sends_no_moves`. Classified
  major: certain from the egui rule, not confirmable without a tablet.
- **Help / README precision:** the barrel button acts as the right button only while the tip touches (Windows Ink,
  decision D6); a pen display's touch acts as a mouse (egui-winit emulates the first touch; unchanged from `main`):
  turn touch off in the driver or Windows' "Ignore touch input when I'm using my pen". How to turn Windows Ink on per
  driver (Wacom Mapping > "Use Windows Ink", Huion "Enable Windows Ink", XP-Pen "Windows Ink", per-application lists) was
  already in Help > Tablet setup and README.
- **Window subclass vs accesskit:** eframe installs accesskit's subclass before `App::new`, so the pen subclass sits on
  top and forwards to it; accesskit's own uninstall at teardown drops the pen subclass, which is harmless then.
- **Installer:** nothing to stage. No new file or DLL; `wintab32.dll` comes with tablet drivers and is loaded at run
  time only in WinTab mode; `installer/smoke-test.ps1` runs `--screenshot` runs, which never install the subclass. The
  user guide `dist/HOW_TO_TEST.txt` is not shipped by the installer (unchanged); the in-app Help > Tablet setup is the
  user-facing text.
- **Claims without an automated test** (all in the hand tests): the effect of `SetWindowFeedbackSetting` / the gesture
  answer / the pen service property; per-sample pressure from real `GetPointerPenInfoHistory` data; barrel in 2D (pan)
  and on a layer row (menu) — the router's Secondary button is unit-tested and orbit is shown in the scenario; WinTab
  against a real driver.
- **Not fixed, noted:** `WM_POINTERLEAVE` starts no frame, so the eraser override and the "Pen" read-out can lag until
  the next mouse move or pen sample (nothing paints in between).

## 6. Hand tests for the user (`dist/HOW_TO_TEST.txt`, "Pen tablets")

With the tablet driver's Windows Ink option on:
1. Hover over the model: the brush ring sits exactly under the pen tip (no offset, also at the screen edges and on a
   second monitor) and the status bar shows "Pen" (in a narrow window: the Pen line under Tool tab > Brush options >
   Pen pressure).
2. Paint in 3D and 2D, light to hard: thin to thick, the status bar shows the percentage; fast curves are smooth; a
   stroke starts where the tip touches.
3. Taps on toolbar buttons, layer rows, tabs, sliders, menus: one click each. Hold the pen still above a toolbar button
   for a second: its tooltip appears.
4. Pen pressure: Size off + Flow on: constant width, paint builds with pressure; Size on with min 50 %: light strokes are
   half as wide.
5. Turn the pen around: the Eraser is selected and erases; turn it back: the previous tool returns; Ctrl+Z undoes one
   eraser stroke.
6. Hold the lower pen button and touch, then drag: 3D orbits, 2D pans; a tap with the button held on a layer row opens
   its menu.
7. Press and hold the pen still on the model: no ring animation, no right-click menu, no ripple on taps.
8. Restart: the pressure settings are remembered.
9. With Windows Ink off in the driver: no pressure; then Help > Tablet setup > Tablet input: WinTab: pressure works,
   each stroke appears once, the eraser end erases, no fat blob at the end of a stroke lifted while moving, the first
   dab is not bigger than the rest (O2). Set it back to Windows Ink.
10. Mouse painting feels and looks as before.
11. Pen display with touch: rest the hand on the screen while drawing; if it paints or moves things, turn touch off in
    the driver or Windows' "Ignore touch input when I'm using my pen" on.

Note the tablet model and driver version with anything that fails, and send `%APPDATA%\WrappedTextureEditor\log.txt`
(it says whether the pen window hook was installed).

## 7. Commits

| Commit | What |
|---|---|
| 0558d20 ... 67f94ad | plan and parts A-G (builder) |
| f751aa4 | checker: five fixes with tests |
| 499e321 | gate: hover dead band, tablet help / README / hand tests, ARCHITECTURE / HANDOFF, `tools/pen/jobs_gate.txt` |
| (this) | this report |

Nothing merged to `main`; only `pen-input` pushed.
