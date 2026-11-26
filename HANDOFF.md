# Wrapped Texture Editor — handoff to the new machine

This file is for the Claude Code session that continues the project on the new (faster, GPU) computer. It
covers the state of the code, how it was built and tested, the user's binding decisions, the rules of the
previous sessions, what is still open, and a checklist of **things to look over and confirm** first.

Written 2026-11-22 by the previous session (on a 6-core, 7 GB laptop without a usable GPU, slow USB disk).

---

## 1. What this is

A Rust desktop app (egui 0.37 + wgpu) for painting textures directly on 3D models: import OBJ / glTF / GLB / FBX,
paint in 3D / 2D / Split views, layers, brushes (Circular, Sphere, Cube, Cylinder, model-file shapes), image stamps
and text stamps (system fonts, warp, text on a path, styles, per-gap spacing), 3D (projected) and 2D (UV) stamps,
and since round 6 **Wrap on surface** for 3D text and image stamps. Exports PNG / PSD / ORA and saves `.wtep`
projects (format version 3).

- `crates/wte_core` — document model, mesh/UV/texel maps, brush, stroke, stamps, text, wrap, project I/O, export.
- `crates/wte_app` — the egui/wgpu app, tools, panels, jobs, and the **headless self-test runner** (`selftest/`).
- `ARCHITECTURE.md` — the **binding design contract** (≈580 KB). Every round updates its own sections. Read
  section 1 (history of rounds 1.4-1.7) and the sections a task touches before changing code.

## 2. Repository layout

| Path | What |
|---|---|
| `crates/`, `Cargo.toml`, `Cargo.lock`, `ARCHITECTURE.md` | the source (branch `main` = the landed build) |
| `.cargo/config.toml` | linker setting for the `x86_64-pc-windows-gnu` target only (rust-lld, self-contained) |
| `cargo.cmd` | **old-machine only**: wrapper that put a toolchain on `E:\claude_downloads\rust-toolchain` on PATH |
| `cargo-gnu.cmd`, `rust-toolchain.toml`, `tools/setup-toolchain.ps1`, `TOOLCHAIN.md` | the portable toolchain setup for any machine (section 3) |
| `tools/fetch-samples.ps1` | downloads the Khronos sample models used by the gates into `samples/` (git-ignored) |
| `dist/` | launcher scripts and `HOW_TO_TEST.txt` for the user's test build (the `.exe` is not committed) |
| `docs/plans/` | feature briefs (`FEATURE_*.md`), round plans `ROUND3_PLAN.md` … `ROUND8_PLAN.md` |
| `docs/reports/` | gate reports of rounds 3-8 (`ROUND<n>_GATE_REPORT.md`, `ROUND3_REPORT.md`) |
| `docs/notes/round6..8/` | builder / check-and-fix / gate / skeptic notes of the last three rounds |
| `tools/` | gate tooling: `round3/` (`check_exports.py` = validate PSD/ORA exports with Pillow, `shot_diff.py`, `uvhash_compare.py`, `fuzz_check.py`), `round3_audit/` (`sweep.ps1` runs scenario jobs files, `slow.py` finds slow frames), `round4_gate3/` (`frames.py`, `btp.py`, `wf_steps.py`, `perf_driver.ps1`, `core_benches.ps1`) |
| `test-assets/shapes/` | awkward test meshes (torus_knot, mobius, klein_bottle, nonmanifold, many_islands, ngons_tiling, flat_annulus, tiny_cube, …) |
| `test-assets/make_shapes.py` | generator of those shapes; `high_poly_sphere.obj` (55 MB, 1 M triangles) is **not committed** — regenerate with `python -I test-assets/make_shapes.py <out_dir>` (writes all shapes into `<out_dir>`) |

**Path mapping.** The plans, reports and notes refer to the old machine's paths. Translate:
`target\roundN_design\ROUNDN_PLAN.md` → `docs/plans/ROUNDN_PLAN.md`; `target\roundN\GATE_REPORT.md` →
`docs/reports/ROUNDN_GATE_REPORT.md`; `target\roundN\notes\*.md` → `docs/notes/roundN/`;
`target\compat\harness\shapes\` → `test-assets/shapes/`; `target\round3\*.py` → `tools/round3/`;
`target\round3_audit\` → `tools/round3_audit/`; `target\round4\review\gate3\` → `tools/round4_gate3/`;
`E:\claude_downloads\glb_samples\*.glb` → see 4.3. Run folders (`runs_*`), screenshots and work copies were not
uploaded (too large).

## 3. Building

**Full details: [`TOOLCHAIN.md`](TOOLCHAIN.md).** The compiler is not in the repo (its files are 205-341 MB, over
GitHub's 100 MB limit), but it is pinned and reproducible:

1. `powershell -ExecutionPolicy Bypass -File tools\setup-toolchain.ps1 -InstallRustup` installs rustup (if missing),
   the exact toolchain **rustc 1.99.1 `x86_64-pc-windows-gnu`** with clippy / rustfmt / llvm-tools, and rebuilds the
   **dlltool shim** in `.toolchain\dlltool-shim` (the toolchain's `llvm-ar.exe` under the names `dlltool.exe` and
   `llvm-dlltool.exe`, plus two MinGW DLLs). No Visual Studio needed; linking uses the bundled `rust-lld`
   (`.cargo/config.toml`). `rust-toolchain.toml` pins the version.
2. `.\cargo-gnu.cmd test --workspace` · `.\cargo-gnu.cmd clippy --workspace --all-targets` (must be 0 warnings) ·
   `.\cargo-gnu.cmd build --release -p wte_app` → `target/release/wrapped_texture_editor.exe`.
   (`cargo-gnu.cmd` is the portable wrapper; `cargo.cmd` only works on the original laptop.)
   `tools\fetch-samples.ps1` downloads the Khronos sample models into `samples\`; the gate tools need Python 3 with
   Pillow and numpy.
   (The old session always ran with `--offline` because of a slow network; a first online build fetches crates.)
3. No new crates were allowed in rounds 4-8 (everything uses what `Cargo.lock` already has: egui/eframe 0.37,
   wgpu, glam, rayon, ttf-parser, image, …). Keep that rule unless the user agrees otherwise.

## 4. Testing

### 4.1 Headless self-test runner (the only way the app was driven)

```
wrapped_texture_editor.exe [MODEL_OR_WTEP_PATH] --screenshot <out.png> --scenario <name> [--size 1280x800]
```

It plays a scripted scenario, writes `<out>.png` (plus named extra shots) and `<out>.json` with `"ok": true/false`,
and a log `<out>.log`. Scenario names (`crates/wte_app/src/selftest/scenarios.rs`, `NAMES`):

`empty smoke default paint3d paint2d stamp text clay preview export export_run multi roundtrip undo import
import_tex import_nouv unsaved open_path import_save layers workflow_path fuzz_path stamp_edge_path pan2d
brush_types brush_shape brush_tab brush_types_path text_gaps text_styles text_warp text_path text_tab text_fonts
text_preview_path brush_hover final_bake_async wrap_surface export_selected export_zip export_flow export_unsettled
save_unsettled export_nouv export_errors export_checks side_tabs objects objects_path objects_flow_path pen companion
companion_host sticker_view` (55; `scenarios::tests` checks the count and that every shot of a `sticker*` scenario says
what it must show; `pen` = branch `pen-input`, recorded pen samples fed in-process, see 5.2; `sticker_view` = branch
`stickers` P6a: the Sticker view on the sample logo, shots `sticker_enter`, `sticker_moved`, `sticker_reordered`,
`sticker_small` (900x600 inside the run), `sticker_left`; each shot's `expect` text is in `<out>.json` under `"shots"`)

`*_path` scenarios need a PATH argument (a model); `objects_path` needs a model with **several objects** (gate:
`ABeautifulGame.glb`, 49 objects; also `Lantern.glb`, 3) and fails with a clear message on a one-object model; `objects_flow_path` (the whole user workflow: hide the board,
paint / stamp / wrap / text a pawn, save, reopen, export) needs a model whose board-like object has its own texture set
(gate: `ABeautifulGame.glb`). Branch
`objects-tabs`: `side_tabs` (the two tab groups of the side panel; also run at `--size 900x600` and `640x480`) and
`objects` (the built-in Three Objects sample) need no PATH. `tools/objtabs/sweep.sh EXE OUT tools/objtabs/jobs_gate.txt`
runs the whole gate sweep (sequential, off-screen). `export_flow` takes an optional PATH (a model or a saved `.wtep`):
without one it writes a cube as OBJ+MTL+PNG, glTF, GLB and ASCII FBX and runs the whole flow on each (paint, stamp,
text, wrapped stamp, then export one layer, a selection and everything as one zip, checked file by file);
`export_selected` drives Ctrl / Shift / plain / right clicks in the Layers panel and the dialog's "Selected (N)";
`export_zip` checks the zip against the loose files and the replace step. `export_checks` (accept_fixes item E) checks
the written files against a reference compositor written out in the test (ARCHITECTURE 6.6): hidden layers under
Visible / All, a Shift-click range across a hidden layer (the dialog marks its file "(hidden)"), Base + one layer
flattened, 50 % Multiply within 2/255. `tools/round3_audit/sweep.ps1` runs a jobs file of
`scenario[,path]` lines.

### 4.2 Switches (environment variables)

| Variable | Effect |
|---|---|
| `WTE_FRAME_LOG=1` | per-frame timing lines (used by `tools/round4_gate3/frames.py`) |
| `WTE_STROKE_LOG=1` | brush phase probe: one line per slow sample and a `brush stroke probe:` line per stroke |
| `WTE_SYNC_FINAL_BAKES=1` | old behaviour: Final stamp bakes on the UI thread (round 5 moved them to a worker) |
| `WTE_STAMP_MODE=wrap` | new stamps start in Wrap (for like-for-like timing runs) |
| `WTE_COMPANION_PORT`, `WTE_COMPANION_TEST_CODE`, `WTE_COMPANION_AUTO_ALLOW=1`, `WTE_COMPANION_EXPECT_SESSIONS`, `WTE_COMPANION_ADB_SERIAL`, `WTE_COMPANION_LINGER_MS` | branch `companion`, scenario `companion_host` only (`--screenshot` runs): test port (47811), fixed pairing code, allow confirm requests at once, sessions to wait for, the one adb serial the watcher may address, ms to keep serving after the asserts (`docs/notes/companion/desktop.md`) |
| `WTE_PEN_SUBCLASS=1` | branch `pen-input`: installs the pen window-procedure subclass in a self-test run too (it is always installed in a normal run); proves it does no harm; what it queues is dropped |
| `RAYON_NUM_THREADS=n` | pool size; brush and wrap results must be identical for any n |
| `WGPU_BACKEND=dx12` / Vulkan | renderer choice (see `dist/*.cmd`) |

### 4.3 Sample models (not in the repo)

The gates used Khronos glTF sample models: **ABeautifulGame** (chess set, 1.5 M triangles, 15 texture sets — the
heavy-model benchmark), DamagedHelmet, Avocado, WaterBottle, Lantern, BoxTextured, plus others. Get them from the
Khronos `glTF-Sample-Assets` repository on GitHub (`Models/<Name>/glTF-Binary/<Name>.glb`). The "topology" models
`tex_4096` / `tex_8192` used in brush timing were generated locally and are not included (optional).

### 4.4 Test counts and identity checks

- Landed build (rounds 6+7): `cargo test --workspace` **1,116 passed, 33 ignored, 0 failed**; clippy 0 warnings.
- Wall-clock tests (branch `stable`, polish item 4): the tests with a tight time limit are ignored `perf_*` tests and
  run serially on an otherwise idle machine, never in the parallel run (exceptions: texel_map's 1 s guard
  `tiled_uvs_build_in_one_pass_over_the_image` in the parallel run, and the ignored P10 `tip_bake::tests::load_times`).
  Gate commands:
  `cargo test --workspace`; `cargo test -p wte_core --lib perf_ -- --ignored --test-threads=1` (WA7, check:A, 200 ms);
  `cargo test -p wte_app perf_ -- --ignored --test-threads=1` (font sample job, 0.5 s); the GPU app tests
  `cargo test -p wte_app -- --ignored --skip perf_`. ARCHITECTURE.md section 9 has the details.
- Branch `stable` after the acceptance fixes (`docs/notes/stable/accept_fixes.md`): `cargo test --workspace` 1,232
  passed, 1 failed (WA20, the known red), 43 ignored; clippy 0 warnings; GPU app tests 18 passed; 47 scenarios.
- Every round checked that unrelated scenarios stay **pixel-identical** to the previous exe (`shot.png`, toast
  area excepted) and that exports are byte-identical (`tools/round3/check_exports.py` must PASS).

## 5. History and current state

| Round | What | State | Exe SHA-256 (old machine) |
|---|---|---|---|
| 1-3 | core app, import, paint, layers, stamps, export, crash recorder | landed | round 3: 7D0E0B57… |
| 4 | brush types / shapes (incl. model-file shapes), text effects (fonts, warp, path, styles, gaps), project format 2 | landed 2026-11-20 | 7695C240… |
| 5 | smoothness: Final stamp/text bakes on a worker, brush pacing by dab, font picker first-open fix | landed 2026-11-21 | 5DA4E165… |
| 6 | **Wrap on surface** (Project / Wrap toggle, Jump small gaps, Ends meet / Overlap), project format 3 | landed 2026-11-21 | 6CE65B55… |
| 7 | brush speed on heavy models (lean pass 2, fused parallel band raster + commit), pixels unchanged | landed on top of 6 | **2787C1B9…2BF8** (= `main`) |
| 8 | Wrap quality on bumpy surfaces + Wrap speed | **NOT landed** — branch `round8-wip` | — |
| pen-input | drawing tablets: pen pressure (size / flow), eraser end, barrel button, Windows Ink feedback off, status read-out, Help > Tablet setup; plan `docs/plans/PEN_INPUT_PLAN.md`, notes `docs/notes/pen/` | built on branch `pen-input` (2026-11-24), **not merged to `main`**; hand tests with a real tablet owed (5.2) | — |
| objects-tabs | objects (select / hide / isolate / frame, hidden objects skipped by every ray; Objects tab) and two tab groups in the side panel (Tool / Color / Texture sets / Objects above, Layers / Brush / Text below, draggable split); plan `docs/plans/OBJECTS_TABS_PLAN.md`, notes `docs/notes/objtabs/`, report `docs/reports/OBJECTS_TABS_GATE_REPORT.md` | built and gated on branch `objects-tabs` (2026-11-24), **not merged to `main`** | — |

| companion | Android tablet companion: in-process server (USB through adb reverse, Wi-Fi with a pairing code, Bluetooth RFCOMM), the window mirrored as JPEG tiles, the tablet's pen / fingers / keys fed in-process; Help > Tablet companion, status chip; plan `docs/plans/COMPANION_PLAN.md`, wire spec `docs/protocol/`, notes `docs/notes/companion/`; the Android app is in https://github.com/Maviman/tablet-companion | desktop part D0-D11 built on branch `companion` (2026-11-24), **not merged to `main`**; Android part next, then the gate; hand tests owed (5.3) | — |
Reports: `docs/reports/`. Plans: `docs/plans/`.

### 5.2 Pen input (branch `pen-input`)

Plan `docs/plans/PEN_INPUT_PLAN.md`, decisions and per-part notes `docs/notes/pen/` (`build.md` = the measurements and
comparisons), ARCHITECTURE 1.10. A pen is "a mouse with pressure": the pen's touch events never reach egui, a
window-procedure subclass (`pen/win.rs`) reads exact per-sample pressure and the barrel / eraser flags, Size follows
pressure from 10 % by default (Tool tab > Brush / Eraser options > Pen pressure), the eraser end switches to the
Eraser and back, the barrel button is the right button, Windows Ink's press-and-hold / tap / flick feedback is off, the
status bar shows "Pen N %", Help > Tablet setup explains the driver settings. Mouse strokes, exports and the central
view are identical to `main` (checked against a 1b589a1 exe).

**Owed by the user** (the builder had no tablet): the eleven hand tests in `dist/HOW_TO_TEST.txt` ("Pen tablets"), with
Windows Ink on; the WinTab path (part G, opt-in: Help > Tablet setup > Tablet input: WinTab; `wintab32.dll` loaded at
run time, pressure and eraser only) can only be checked with a driver whose Windows Ink option is off.

Independent check (2026-11-24, `docs/notes/pen/check.md`): plan complete; five fixes with tests (WinTab eraser press
ended the eraser override, WinTab stroke-end blob, WinTab lock re-entry, the eraser flipping back after every mouse
stroke, a second finger without the subclass).

Gate + completeness critic (2026-11-24, `docs/reports/PEN_INPUT_GATE_REPORT.md`, notes `docs/notes/pen/gate.md`):
**PASS**; one fix (a pen held still above the tablet no longer sends hover moves within 1 pt, so tooltips can show),
Help > Tablet setup / README made precise (barrel button counts while the tip touches; touch on pen displays), hand
tests extended to 11 (ring offset on every monitor, tooltips, pen display touch, the WinTab specifics).

### 5.3 Tablet companion (branch `companion`)

Plan `docs/plans/COMPANION_PLAN.md` (section 10 = work order; the user later moved the Android app to its own repo
https://github.com/Maviman/tablet-companion, app name "Tablet Companion", package io.github.maviman.tabletcompanion),
wire spec `docs/protocol/COMPANION_PROTOCOL.md` + fixtures (copied byte for byte into the tablet repo's `protocol/`;
change both together, `docs/notes/companion/protocol-changes.md`), ARCHITECTURE 1.11 and 8.14, notes
`docs/notes/companion/` (`desktop.md` = the hand-over to the Android side: how to run the desktop server headless).
Desktop part done (D0-D11): server, handshake / pairing, JPEG tile encoder, capture, remote input through the pen
router, dialog, status chip, adb watcher (checked against the emulator only), RFCOMM (compiled, never run). Scenarios
`companion` (loopback test tablet) and `companion_host` (the Android end-to-end run's desktop side); with every link off
the app is unchanged (regression shots identical to the e9c6567 exe). No new crate.

**Owed:** the Android app and the end-to-end run (tablet repo), the gate, and the user's hand tests with the real
tablet (`dist/HOW_TO_TEST.txt` "Tablet companion" + the tablet repo's `docs/HOW_TO_TEST.md`), incl. Bluetooth (never
run by agents), the Windows Firewall prompt for Wi-Fi, the Samsung USB driver and Air command on the S Pen button.

### 5.1 Round 8 (branch `round8-wip`) — failed its gate, kept for a round 9

Plan `docs/plans/ROUND8_PLAN.md` (user answers in its section 12), report `docs/reports/ROUND8_GATE_REPORT.md`,
notes `docs/notes/round8/` (including two independent skeptic reviews `verify_quality.md`, `verify_regressions.md`,
both "do not land"). The branch holds round 8's merged source (packages A + B).

Gains: the bumpy ball (`high_poly_sphere`) now shows one whole picture bent over the bumps (round 6: scraps all
over); the Klein bottle's visible side is one piece; wrapped drags on ABeautifulGame are smooth (max ~23 ms, Previews
on a worker, last finished picture shown until the new one lands); Project / brush / exports / file format identical
to the landed build.

Failures: 4 core tests red (WA4 at 60° ink 1.136, a cell 1.6×; WA20 bumpy ball 10.7 % of the picture twice; WA22
picture larger than a torus 2.3 % twice; WA24 Jump onto a cube 14.8 % twice — the owner test by geodesic depth needs
a visibility tie-break); **regressions vs round 6**: on torus_knot 60 % of the picture disappears (covered 1.0 → 0.40),
DamagedHelmet with Jump off loses a third, Avocado shows six cells twice, the WaterBottle label is no longer exact
(relaxed; 28 % of texels change), un-relaxed layouts changed (flat_annulus, ngons_tiling, mobius; WA5 cuts 1 → 2);
**slower** Finals on heavy models (step 6 "grow the patch from the anchor over a welded graph" was not built:
ABeautifulGame Finals 1.7-10.8× Project, 20 % decal 340-1,070 ms, bumpy ball Final 11.5 s).
Not wrap bugs (confirmed by the skeptic): DamagedHelmet's doubles come from mirrored UVs (77.8 % of its surface
shares texels with its mirror half); ABeautifulGame's blobs on other pieces come from instanced meshes sharing texels.

## 6. The user's binding decisions (keep them)

- 3D (projected) and 2D (UV) stamps stay **separate kinds**, not cross-editable ("3D stamp" / "2D stamp" badges).
- ~~The colour picker stays visible in the side panel whatever tool is active.~~ **Superseded 2026-11-24** by the
  tab decision below (Color is one tab of the upper group). Mitigations: a click on the toolbar's primary colour chip
  shows the Color tab, a tool change never leaves the Color tab, and at 1280x800 and above the whole picker is on
  screen whenever the Color tab is shown (ARCHITECTURE 1.9, 8.7 guarantee G1).
- No support for pre-round-2 projects or the legacy no-UV layout.
- No image editor needed for PSD / ORA: validate exports with Pillow (`tools/round3/check_exports.py`).
- Round 4 (`docs/plans/ROUND4_PLAN.md` section 7): no "connected surface" checkbox; opening an open shape file asks
  how to close its openings; fonts from the Windows fonts folder; "segmented" text = per-gap character spacing.
- Round 6 (`docs/plans/ROUND6_PLAN.md` section 11): new stamps start in **Project**; Wrap folds over every edge;
  **Jump small gaps** and **Ends meet / Overlap** are per-stamp toggles for text and images.
- Round 8 (`docs/plans/ROUND8_PLAN.md` section 12): larger than the object → edges meet in one cut at the back; keep
  the text's centre line true (stretch to the edges); cut at corners/tips sharper than 45°; on heavy models drags stay
  smooth and the picture catches up; round-6 wrapped stamps keep their saved pixels until edited.
- Objects and tabs (2026-11-23/24, `docs/plans/OBJECTS_TABS_PLAN.md`): the side panel is **two tab groups** with a
  draggable split, upper **Tool | Color | Texture sets | Objects**, lower **Layers | Brush | Text** (one page per group;
  the lower group keeps the round-4 one tab rule). Objects come from the file (OBJ `o` / `g`, glTF mesh nodes, FBX
  Models); **hide = not drawn and rays pass through** (painting, stamps, text, wraps, eyedropper, fill seed, gizmo);
  **selecting never restricts painting**; hiding / selecting is view state (no undo entry, not "modified"); hidden
  object names are saved in the project, the selection is not. Keys H / Shift+H / Alt+H / Shift+F (Alt+H to be tried
  by hand on Windows; fallback Ctrl+Shift+H). Stamps bake against what is visible when they are baked.
- Brush speed (after round 7): on the old CPU-only laptop the remaining lag (Size 50 trails 0.5-0.8 s, Size 512
  keeps painting 2-4 s after release on ABeautifulGame) was accepted ("only so much you can do without a GPU").
  **The new machine has a GPU — ask the user whether a GPU brush is now wanted** before planning one.

## 7. How the previous sessions worked (the user's process rules)

- **Plan first, then build.** The user prefers planning a round with Fable and building with Opus (rounds 6-8 were
  planned with Opus by the user's choice because Fable's weekly limit was used up).
- **Lean build**: per package one builder agent, then one check-and-fix agent; then a gate (merge, tests, clippy,
  release build, scenarios, pixel identity, measurements, `GATE_REPORT.md`). Fewer packages, smaller rounds.
  Check weekly plan usage before starting a round.
- **Nothing is "landed" until the main session itself verified it**: read the report, rerun the tests, run key
  scenarios and look at the screenshots, check the main tree did not change underneath; only then copy the merged
  source into the main tree and the exe to `target/release` and `dist`.
- Never use OS-level mouse/keyboard injection (the user may be using the PC); drive the app only via the headless
  runner. Never kill a running copy of the app the user may have open (copy the exe if it is locked).
- Don't change system or overlay settings. Don't run downloaded binaries.
- Keep large outputs off a nearly full system drive (old machine: everything on E:). Delete a landed round's build
  waste; keep toolchains and sample models.
- Treat the user's name and email as private (this repo is public: commits use the GitHub noreply address).

## 8. Open issues (beyond round 8)

1. **Wrap quality / speed** — round 8's failures above; the natural round 9 continues from `round8-wip`.
2. **Brush on very heavy models** — see section 6 (GPU question). Round 7's report lists the remaining pass-2 work
   on island-border triangles as the next CPU step.
3. **FBX texture-transform `IMPORTER_VERSION`** was never bumped (round 3 open point; the user never confirmed; no
   sample is affected).
4. Round 5 minor items: Preview stamp bakes during a drag on ngons_tiling are 148-198 ms (unchanged since round 4);
   text-rotate Final 1.31-1.40× round 4; classic painter 1.03-1.05× round 3; "Layer is locked" wording.
5. The 9 round-4 fuzz repro jobs were never rebuilt (round 5 report, "Stage-1 open points" 4).
6. All timings in the reports were measured on the old laptop under memory pressure (often < 1 GB free); none of
   them hold on the new machine.

## 9. Things for the new session to look over and confirm

1. **Clean build from this repo**: `git clone`, run `tools\setup-toolchain.ps1 -InstallRustup`,
   `.\cargo-gnu.cmd test --workspace` → expect 1,116
   passed / 33 ignored / 0 failed on `main`; clippy 0 warnings; release build. Note any difference (a different
   toolchain or target may change warnings or test timing asserts such as WA19's wall-clock check).
2. **GPU rendering**: start the app normally and with `dist/Start - DirectX 12.cmd`; the old machine never ran on a
   real GPU — confirm the views, Split, 2D, stamps and text render correctly.
3. **Scenario sweep on `main`**: run every scenario at 1280×800 on the built-in sample; all should report `"ok": true`.
   Run `wrap_surface` with `test-assets/shapes/torus_knot.obj` and with WaterBottle / Avocado GLBs and look at the shots.
4. **Re-baseline performance** with `WTE_FRAME_LOG=1` on ABeautifulGame (`workflow_path`, `brush_types_path`,
   `text_warp`) — the old numbers do not apply.
5. **Round 8 branch**: build `round8-wip`, run its tests (expect 4 failures: `wa4_r8`, `wa20`, `wa22`, `wa24`) and the
   sweep; confirm the skeptics' findings (torus_knot coverage loss, Avocado doubles, WaterBottle no longer exact,
   mirrored-UV / instancing explanations). Then plan round 9 with the user.
6. Check that the GNU toolchain setup works there (`TOOLCHAIN.md`); `tools\setup-toolchain.ps1` and `cargo-gnu.cmd`
   were tested on the old laptop against its installed 1.99.1 only (shim rebuilt byte-identical, a test exe linked),
   not on a fresh machine.
7. Ask the user about the GPU brush (section 6) and the FBX `IMPORTER_VERSION` point before scheduling work.
