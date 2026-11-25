# Tablet companion gate report (gate + completeness critic)

Editor branch `companion` (worktree `G:\software_projects\texture_wrapper_ap_cmp`), plan `docs/plans/COMPANION_PLAN.md`;
tablet app https://github.com/Maviman/tablet-companion (`main`). The user's request: an Android tablet app that mirrors
the desktop app live and lets them draw into it, over USB, Wi-Fi or Bluetooth, with no tablet update needed when the
desktop app changes. Gate run 2026-11-24 on the gate machine (RTX 5070, Vulkan; headless emulator `wte_tablet`, API 35,
pixel_tablet profile, WHPX; no real tablet).

Gated:

- Editor `companion` @ `7c5bfc6` (desktop D0-D11, `WTE_COMPANION_LINGER_MS`, fix pass). Release exe
  `target\release\wrapped_texture_editor.exe`, SHA-256
  `30D1DEF19A0A7702D913290E529E4C56F13891EC34151CACB2F9438C7E87EE48` (copy `target\companion\gate\wte_gate.exe`).
- Baseline: `e9c6567` (pen gate), `target\companion\baseline\wte_base.exe`, SHA-256
  `B599A21E70CFDB1269225BAFDD37D0904004A3D4120DCCD36AD5C17F9DADBE91`.
- Tablet app `main` @ `ef581a0`, fresh clone into a scratch folder; debug APK SHA-256
  `409D81F2629F05AC02D99B0DDB6B0938C81BCDDA5B287C6DA83FD12967233CA7` (local build; CI builds use their own debug key).
- Raw outputs: `target\companion\gate\` (test / clippy / build logs, `sweep_new\`, `sweep_base\`, `compare.txt`,
  `e2e\`).

## Verdict

**PASS.** Every claim that can be checked without the real Galaxy Tab S8 Ultra is met: the tablet app built from a fresh
clone works end to end on the headless emulator against the real desktop exe over both the USB path (adb reverse) and
the Wi-Fi path (code, wrong code, token reconnect), a pressure stroke drawn on the emulator is painted on the desktop
thin to thick, and the tablet shows the desktop's picture. With every link off the editor is unchanged against the
baseline (pixels and exports), apart from the named UI additions. The gate found no blocker or major to fix; it added
three hand tests (T15-T17) for gaps the earlier passes did not name. What is left is hardware and decisions: sections 5
and 6.

| # | Item | Status |
|---|---|---|
| R1 | `wrap_tests::wa20_a_bumpy_ball_receives_the_picture_once_whole_and_smooth` fails | known red, the same at e9c6567 / `main` |
| R2 | `import_save` on BoxTextured.glb: `"ok": false` | pre-existing, the same with the baseline exe (the scenario needs a model with sidecar files; on `coincident_flipped_spheres.obj` it passes) |
| O1 | Nothing has run on the real tablet: S Pen feel, palm, side button / Air command, USB debugging + adb on this PC, real Wi-Fi + the firewall prompt, latency feel | hardware only: section 6 |
| O2 | Bluetooth (desktop RFCOMM listener and tablet BluetoothLink) has never run, by rule (plan C14) | hardware only: T10 / desktop step 9 |
| O3 | No tablet release exists and the tablet repo is private, while the editor repo (public) links to its releases page | user decision D1 |

## 1. Editor: workspace tests, clippy, release build

`cargo-gnu.cmd test --workspace --no-fail-fast` (low priority, stdin closed): **1,411 passed / 1 failed (WA20, R1) / 48
ignored** (baseline e9c6567: 1,344 / 1 / 48; +67 companion tests). `clippy --workspace --all-targets`: **0 warnings**.
`build --release -p wte_app`: OK, hash above (equal to the fixer's `30d1def1...`). `Cargo.toml` / `Cargo.lock` unchanged
since e9c6567: no new crates.

## 2. Editor: scenarios and companion-off identity

Sweep `tools/objtabs/sweep.sh` with the job list `target\companion\gate\jobs.txt` = `tools/pen/jobs_gate.txt` (every
`NAMES` scenario that needs no model, the `_path` ones on the Khronos samples, the small-window sizes) plus `companion`
at 1280x800 and 900x600: **68 jobs, 67 `"ok": true`**, the one red is R2. `companion` and `companion_900` pass (4 s
each). `companion_host` is not in the sweep: it waits for a real tablet and is covered by the end-to-end runs
(section 3). Every run opened off-screen in the background; nothing took the focus.

The same 68 jobs with the baseline exe (`sweep_base\`), compared with `tools/objtabs/compare.py`:

- **303 files identical**, among them every export output (PNG / zip / OBJ / MTL of `export*`, `roundtrip`, `workflow`,
  `objects_flow`, `wrap_surface`, ...): byte-identical.
- 14 `.wtep` saves differ only in `manifest.json` `saved_unix` (the save time): checked by normalizing that field, 0 other
  differences.
- 8 PNGs differ, all named:
  - `companion\shot.png`, `companion_900\shot.png` (+ 2 `shot_companion_dialog.png` only in new): the baseline does not
    know the scenario.
  - `pen\shot_tablet_help.png`, `pen_900\...`, `pen_640\...`: the new "Tablet companion" paragraph in Help > Tablet
    setup.
  - `export\shot.png` (959 px) and `brush_shape\shot_dialog.png` (10,672 px): the output / asset path shown in the
    dialog contains the sweep folder (`sweep_base` vs `sweep_new`, one character shorter, so the dialog is narrower).
    Looked at side by side: only the path text and the dialog width.
  - `final_bake_async\shot.png` (9,879 px, the toast area): the fade of the "Exported 4 files" toasts is at a different
    step; same text. Timing, not companion.

## 3. Tablet app: fresh clone, build, tests, end to end

- Fresh `git clone https://github.com/Maviman/tablet-companion` (3 commits, 85 files, all text LF; `.kotlin/`, `.gradle/`,
  `build/`, keystores, `local.properties` ignored). `gradlew.bat clean testDebugUnitTest assembleDebug
  assembleDebugAndroidTest assembleRelease lintDebug`: **BUILD SUCCESSFUL** (1 min 52 s).
- JVM tests **35/35** (AuthTest 5, InputTest 12, ProtoTest 8, SessionTest 10). Lint **0 errors**, 2 warnings
  (`DataExtractionRules`, `ObsoleteSdkInt`, both known in `docs/notes/android.md`).
- Instrumented tests on the headless emulator (`-no-window`, only `-s emulator-5554` addressed): **OK (18 tests)**:
  CanvasTest 9, LifecycleTest 3, PalmTest 3; E2eTest skips itself without `-e e2e true`.
- `tools/e2e.sh --exe <editor>\target\release\wrapped_texture_editor.exe --editor <editor worktree>` from the clone:
  **END-TO-END PASSED**.
  - USB (confirm mode through `adb reverse tcp:47811`, auto-allowed in the headless run): stroke of 41 samples, confirmed
    63 ms after the last sample; desktop `"ok": true` (16,182 texels painted, 1 undo step).
  - Wi-Fi (`10.0.2.2:47811`): the wrong code is refused (tablet message, desktop log "wrong code (2)"); the right code
    pairs (mode 2); the second connection uses the stored token (mode 1, no code); desktop `"ok": true`.
  - Pictures, looked at: the desktop screenshot `e2e\e2e_wifi.png` shows the two blue strokes thin to thick in the 2D
    view; the emulator screencap `e2e\e2e_wifi_tablet.png` shows the same window letterboxed next to the toolbar (Undo,
    Redo, Ctrl, Shift, Alt, Eraser, Keys, Fit, Hide, Exit), with the strokes and the chip "Tablet: Google
    sdk_gphone64_x86_64 (Wi-Fi)". `compare_shots.py`: mean absolute difference **2.48 (USB) / 2.50 (Wi-Fi)** of 255
    (limit 12); stroke rows contrast 51-55, tablet vs desktop 5-9.
- Protocol: `tablet-companion/protocol/` equals the editor's `docs/protocol/` byte for byte (`diff -r`: only
  `CHANGES.md` extra); the pushed `origin/companion` spec equals the clone's.
- CI: the "Android APK" workflow is green on `e0c3af0` and `ef581a0`.

Measurements on the emulator (software rendering; the tablet should do better): 11-17 fps while drawing, 0.9-1.2
Mbit/s; input to the picture that contains it: mean 171 / 121 / 88 ms (USB / Wi-Fi code / Wi-Fi token), max 483 ms;
desktop capture request to pixels mean 24.6 ms (max 134 ms); ping 3-35 ms.

## 4. Fixes

None needed: no blocker or major was confirmed. Gate changes (docs only):

- `tablet-companion/docs/HOW_TO_TEST.md`: hand tests T15 (Windows display scaling 125 / 150 %: the end-to-end runs used
  100 % only; the coordinate maths is unit-tested at 1.0 / 1.25 / 2.0), T16 (a large window on a slow link: a FRAME is one
  write and the write limit is the idle time, 5 s / 10 s on Bluetooth; a KEY frame that takes longer ends the session),
  T17 (a fullscreen game in front of the desktop app).
- `dist/HOW_TO_TEST.txt`: desktop step 12 (display scaling).
- `tablet-companion/docs/notes/STATE.md`: gate results and the open decisions.

Checked and not a problem: adb started by the watcher when no adb server runs does not hold the output pipes (tested
with a separate adb server on port 5099: the call returns in 2.1 s and the reader ends); the `WTE_COMPANION_*` variables
are honoured only in `--screenshot` runs; all links default to off; the self-test bind plan is 127.0.0.1 for every
preference (unit test).

## 5. Completeness: what the real Galaxy Tab S8 Ultra still needs

- **Install.** No release and a private repo: the APK comes from the newest green Actions run (artifact
  `tablet-companion-apk`, signed-in browser on the PC) or, simplest here, from a local build:
  `adb install -r app\build\outputs\apk\debug\app-debug.apk` once USB debugging is on.
- **USB.** Developer options + USB debugging on the tablet; Google platform-tools on the PC (not bundled: plan C11; the
  desktop finds adb in `ANDROID_HOME`, `PATH`, `%LOCALAPPDATA%\Android\Sdk`, next to the exe, or by Browse); the RSA
  "Allow USB debugging" prompt; the desktop's Allow question on the first connection. Samsung's USB driver may be needed
  on some PCs (adb shows no device without it).
- **Wi-Fi.** Turning Wi-Fi on makes Windows show its firewall prompt once (that prompt takes the focus: by the user's own
  action only, plan C13). Cancel creates a block rule: README troubleshooting covers it. A network marked Public blocks
  discovery.
- **Bluetooth.** Never run on either side.
- **Pen.** Pressure curve and feel, hover rate, side button vs Samsung Air command, palm with a real palm (Samsung may
  cancel palms itself; FLAG_CANCELED is handled), tilt (sent, unused).
- **Latency feel** and fps on the real device (emulator numbers above are pessimistic).
- **Long sessions** (hours), large desktop windows (a 4K picture is a 33 MB bitmap on the tablet).

## 6. Hand tests for the user

Tablet side: `tablet-companion/docs/HOW_TO_TEST.md` section 4, T1-T17. Desktop side: `dist/HOW_TO_TEST.txt` "Tablet
companion", steps 1-12. In order, the ones that decide whether it works:

| # | Do | Expected |
|---|---|---|
| H1 | Install the APK (README "Install", or `adb install` of a local build). | "Tablet Companion" opens its connect screen. |
| H2 | USB: README "Connect with the USB cable" (T2, desktop 1-2). | Desktop line "Ready (SM X900...)"; after Allow the tablet shows the editor within about 2 s; no console window flashes; a fullscreen game keeps the focus. |
| H3 | Draw light and hard with the S Pen in 2D and 3D; tick "Show speed" (T3, desktop 3). | Width follows pressure; no gaps; the thin trail is replaced by the real stroke; note fps / ping / pen->picture. |
| H4 | Side button drag; palm resting while drawing (T5, T6, desktop 4). | Orbit / pan, nothing painted; no stray strokes; if Air command pops up, tick "side button erases" or turn Air command off. |
| H5 | Pull the cable mid-stroke; plug in again (T11, desktop 7). | The stroke ends cleanly; "Reconnecting..." then it continues by itself. |
| H6 | Minimize the desktop app mid-stroke, lift the pen, restore (T12). | The desktop mouse works at once; Ctrl from the tablet toolbar does not stick. |
| H7 | Home, screen off / on, reopen from the launcher (T13). | Disconnects and comes back without pairing; never "Connection lost" over and over. |
| H8 | Wi-Fi: allow the firewall prompt (private), type the code; reopen the app (T9, desktop 8). | Pairs once; the next connect needs no code. |
| H9 | Display scaling 125 / 150 % (T15, desktop 12). | Pen tip and desktop cursor agree everywhere. |
| H10 | Bluetooth (T10, desktop 9), also with a maximized window (T16). | Connects; coarse picture, responsive pen; no drop right after connecting. |

## 7. Decisions for the user

- **D1 Tablet APK distribution.** Tag `v1.0.0` in tablet-companion (CI then publishes `TabletCompanion-1.0.0.apk` as a
  release)? Make the tablet repo, or only its releases, public? The editor repo is public and its Help / README / installer
  release notes point to `https://github.com/Maviman/tablet-companion/releases`, a 404 for anyone else while the repo is
  private.
- **D2 Signing.** Create a release keystore and add the four secrets (`ANDROID_KEYSTORE_BASE64`,
  `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`; `docs/notes/android.md` "Signing"). Without
  it every CI build has its own debug key: an update needs an uninstall and a new pairing. Keep the keystore safe:
  losing it means the same again.
- **D3 adb.** Platform-tools stay a separate download (plan C11, current), or bundle adb next to the exe in the
  installer? Google's SDK licence restricts redistribution, so bundling needs a licence check first.
- **D4 Editor version.** Land `companion` on `main` (through a PR like v0.2.0) and release it as v0.3.0, after the hand
  tests or before? The companion is off by default, and with it off the app is unchanged (section 2).
- **D5 Behaviour** (from the fix pass): palm rejection blocks fingers while the pen is near (300 ms); the tablet
  disconnects in the background instead of a grace period. Keep, or add settings?
- **D6 Firewall.** Leave the one-time Windows prompt (current), or have the installer add a private-network firewall rule
  for the exe (no prompt; needs admin rights, the installer is per-user today)?

## 8. Commits

- Editor `companion`: this report + `dist/HOW_TO_TEST.txt` step 12 (docs only).
- tablet-companion `main`: `docs/HOW_TO_TEST.md` T15-T17, `docs/notes/STATE.md` gate section (docs only).

The emulator started by the gate is stopped; no desktop app copy started by the gate is left running.
