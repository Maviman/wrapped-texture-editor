# Hand-over: desktop side for the Android builder (tablet-companion repo)

Editor worktree `G:\software_projects\texture_wrapper_ap_cmp`, branch `companion` (desktop part D0-D11 done). The
Android app lives in https://github.com/Maviman/tablet-companion (clone `G:\software_projects\tablet-companion`).

## What the desktop gives you

- Wire protocol v1: `docs/protocol/COMPANION_PROTOCOL.md` and `docs/protocol/fixtures/v1/` (22 `.hex` messages +
  `handshake.json`). Copy both byte for byte into `tablet-companion/protocol/`. They are unchanged since D1 (see
  `protocol-changes.md`). `tools/companion/check_handshake.py` shows how the crypto values are computed in Python.
- A headless desktop server: the self-test scenario `companion_host` (below).
- A reference client in Python: `tools/companion/host_probe.py` (magic, HELLO, AUTH in code / token / confirm mode
  with the MAC checks, CONFIG, frames + FRAME_ACK, PEN with all samples, PING, BYE). It passed against
  `companion_host`: code mode, then a token reconnect without the code.
- A reference client in Rust: `crates/wte_app/src/companion/client.rs` (`TestClient`).

## Run the desktop server headless

From the editor worktree root, Git Bash. Builds of this tree open off-screen in the background (no focus, no taskbar
button); the server binds 127.0.0.1 only.

```sh
cd /g/software_projects/texture_wrapper_ap_cmp
exe=target/release/wrapped_texture_editor.exe        # build: cmd //c "start /b /low /wait G:\Claude_Resources\rust\cargo-gnu.cmd build --release -p wte_app" < /dev/null
test -f "$exe" || echo "build the exe first"
out=target/companion/e2e; mkdir -p "$out"
# USB path (confirm mode: the tablet connects to its own 127.0.0.1:47811 through adb reverse, link = usb)
WTE_COMPANION_PORT=47811 WTE_COMPANION_AUTO_ALLOW=1 WTE_COMPANION_EXPECT_SESSIONS=1 \
  WTE_COMPANION_ADB_SERIAL=emulator-5554 ANDROID_HOME='G:\Claude_Resources\android\sdk' \
  "$exe" --screenshot "$out/e2e_usb.png" --scenario companion_host --size 1280x800 --log "$out/e2e_usb.log" < /dev/null &
desktop=$!
# ... run the tablet test ...
wait $desktop; echo "desktop exit $?"; grep '"ok"' "$out/e2e_usb.json"
```

- `WTE_COMPANION_ADB_SERIAL=emulator-5554` (+ `ANDROID_HOME` so adb.exe is found) makes the desktop run
  `adb -s emulator-5554 reverse tcp:47811 tcp:47811` itself (log line "adb reverse tcp:47811 set for emulator-5554",
  within about 2 s). Leave it out and run `adb -s emulator-5554 reverse tcp:47811 tcp:47811` in the script instead if
  you prefer; both work. It never touches another serial.
- Wi-Fi path (code mode; inside the emulator the desktop is `10.0.2.2:47811`, link = wifi):
  `WTE_COMPANION_PORT=47811 WTE_COMPANION_TEST_CODE=K7QF3M9T WTE_COMPANION_EXPECT_SESSIONS=2` (no AUTO_ALLOW). The
  code is accepted in any case and with or without the dash (normalized). Session 1 pairs with the code, session 2
  must reconnect in token mode (no code).
- Variables (honoured only in `--screenshot` runs):

| Variable | Default | Meaning |
|---|---|---|
| `WTE_COMPANION_PORT` | 47811 | TCP port on 127.0.0.1 |
| `WTE_COMPANION_TEST_CODE` | none | fixed pairing code (8 Crockford characters); the pairing window stays open; 5 wrong codes lock code mode for 30 s |
| `WTE_COMPANION_AUTO_ALLOW` | off | `1` = confirm mode is offered (USB link from loopback) and allowed at once |
| `WTE_COMPANION_EXPECT_SESSIONS` | 1 | sessions that must each draw a stroke with varying pressure before the run ends |
| `WTE_COMPANION_ADB_SERIAL` | none | the one adb serial the desktop's adb watcher may address |
| `WTE_COMPANION_LINGER_MS` | 0 | after the asserts, keep serving this long (at most 60 s) before the screenshot and exit; `tools/e2e.sh` uses 8000 so the tablet confirms its last input and takes its screencap of the settled picture (added by the Android builder) |
| `WTE_SELFTEST_RUN_TIMEOUT` / `WTE_SELFTEST_STEP_TIMEOUT` | 330 / 300 s | override the timeouts (whole seconds) |

Without AUTO_ALLOW and without a code a new tablet gets ERROR 3 PAIRING_CLOSED (the dialog is not open in a headless
run). A wrong code gets ERROR 2 AUTH_FAILED (log: `companion: refused <name>: wrong code (2)`).

## Coordinates

The scenario writes `<out>/e2e_usb_coords.json` (named after `--screenshot` without `.png`, plus `_coords.json`) as soon
as the layout is stable (about 1 s after start), and logs the same:

```json
{ "port": 47811, "surface": [1280, 800], "pixels_per_point": 1,
  "view_centre": [512, 413], "stroke_row": [[279, 413], [745, 413]], "brush_size_texels": 40 }
```

All values are surface pixels (the coordinates the protocol uses). Draw the stroke along `stroke_row` (left to right)
with pressure rising, e.g. 0.1 -> 1.0. The tablet maps its touch position to surface pixels with the letterbox of
STATE `surface_w/h` (the desktop window at 1280x800 has the aspect 1.6; the emulator's view is letterboxed).

## How a run ends and how to read the result

- The run ends when `WTE_COMPANION_EXPECT_SESSIONS` different sessions have each sent a pen stroke (DOWN, MOVEs, then
  UP / CANCEL / LEAVE) with at least 3 samples whose pressure spans at least 20 % of the range. Then it waits until the
  paint is done, asserts that something was painted and that there are at least that many undo steps, takes
  `<out>/e2e_usb.png` and exits (exit code 0). It does not wait for the tablet to disconnect: take the tablet's
  `screencap` after your stroke is confirmed (a FRAME with `last_input_seq` >= your last PEN `seq`) and before or while
  the desktop exits.
- Result: `<out>/e2e_usb.json` has `"ok": true` (exit code 0); `"ok": false` with `"errors"` otherwise (exit code 2 =
  failed assert, 3 = timeout: no qualifying stroke within 300 s). The log has one line per session
  (`companion: session N drew a stroke with varying pressure`) and per disconnect (captures, frames, bytes).
- The desktop screenshot is 1280x800 with the 2D view of the Sphere sample and your blue stroke (40 texels at full
  pressure, thinner at low pressure).

Desktop-only rehearsal of the whole thing (no Android), as done for this hand-over:

```sh
WTE_COMPANION_PORT=47811 WTE_COMPANION_TEST_CODE=K7QF3M9T WTE_COMPANION_EXPECT_SESSIONS=2 \
  "$exe" --screenshot "$out/host.png" --scenario companion_host --size 1280x800 --log "$out/host.log" < /dev/null &
python -E -P tools/companion/host_probe.py --coords "$out/host_coords.json" --code k7qf-3m9t --link wifi --sessions 2
wait; grep '"ok"' "$out/host.json"
```

## What the desktop does that the tablet sees

- After AUTH_OK: STATE (surface 1280x800, ppp 1.0 in a self-test, flags; `TRAIL` while the Brush or Eraser is active,
  `TEXT_WANTED` while a text field has the focus), then a KEY FRAME of the whole window (USB / Wi-Fi: den 1, JPEG
  tiles of at most 512x512 on the 64-px grid), then FRAMEs with changed rectangles, REFINE frames after 400 ms idle,
  CURSOR when the desktop mouse moves. At most 2 frames unacknowledged on USB / Wi-Fi (1 on Bluetooth): send
  FRAME_ACK for every FRAME or the stream stops.
- HELLO_ACK `caps` = `SERVER_CURSOR | SERVER_FIT` (bits 0 and 1).
- Idle timeout: 5 s without any byte from the tablet (10 s on Bluetooth): send PING every second.
- A pinch or finger gesture within 300 ms after the pen was in range is ignored (palm rejection on the desktop).
- Coordinates in PEN / TOUCH are surface pixels as floats; the desktop divides by `pixels_per_point`.

## Protocol changes

None. See `protocol-changes.md` (also for the rule: change the spec, the fixtures, the Rust side and the Kotlin side
in one step in both repos).
