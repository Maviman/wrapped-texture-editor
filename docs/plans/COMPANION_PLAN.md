# Tablet Companion — Plan (COMPANION_PLAN.md)

Branch `companion` (worktree `G:\software_projects\texture_wrapper_ap_cmp`, cut from `pen-input` @ e9c6567). Notes go to `docs/notes/companion/<step>.md`, the gate report to `docs/reports/COMPANION_GATE_REPORT.md`, the wire spec to `docs/protocol/COMPANION_PROTOCOL.md` (section 4 of this plan, copied verbatim by the desktop builder).

I read the code and crate sources but ran nothing. Facts marked "verify" are assumptions the builder must confirm.

---

## 0. Summary and decisions

The desktop app gets an in-process "Tablet companion" server, off by default. A thin Android app (Kotlin, no third-party libraries) shows the live picture of the app's own window and sends pen, finger and keyboard input back. All input enters the app in-process through `raw_input_hook` and the existing pen router.

| # | Decision | Reason |
|---|---|---|
| C1 | Picture comes from `ctx.request_screenshot` (the egui-wgpu capture path), never an OS screen grab. | Works for covered and off-screen windows; no new API. |
| C2 | Picture codec: changed rectangles as JPEG, encoded through a new `wte_core::encode_jpeg`. The tablet decodes with `BitmapFactory`, so PNG also works; id reserved. | No new crate; `image` (with "jpeg") stays a `wte_core` dependency only. |
| C3 | Pull model: one frame in flight (two on fast links), each acknowledged by the tablet; the diff is taken against the last sent picture. | Adapts to any link speed without queues; nothing is sent when nothing changed. |
| C4 | The remote pen is a synthetic `Event::Touch` plus `PenRecord` with a reserved pointer id, fed to `PenRouter`. | Identical to a local Windows Ink pen (pressure, eraser, barrel, hover, every sample in order); the recipe is the one the `pen` scenario already uses. |
| C5 | Fingers are sent raw; the desktop maps gestures to mouse and wheel events. | Gesture behaviour can change without a tablet update. |
| C6 | One tablet session at a time. | v1 scope. |
| C7 | Pointer arbitration: whoever pressed first (local or remote) holds the pointer until release; the other side's pointer events are dropped meanwhile. | Deterministic; no mixed strokes. |
| C8 | Authentication on every connection: mutual HMAC-SHA256 challenge with a 32-byte token per paired tablet. SHA-256, HMAC and PBKDF2 are implemented locally in Rust; Android uses platform crypto. | No crypto crate is in `Cargo.lock`. |
| C9 | The stream is authenticated at connect but not encrypted and not integrity-protected afterwards. This is stated in the dialog, Help and README. | Honest v1; a capability bit is reserved for encryption. |
| C10 | Pairing by code (8 characters, 40 bits, PBKDF2 100,000 iterations) for Wi-Fi. Pairing by an "Allow" click on the desktop for USB and Bluetooth. Unknown tablets are accepted only while the Tablet companion dialog is open. | A tablet paired once over USB then works over Wi-Fi and Bluetooth without typing a code. |
| C11 | USB uses `adb reverse`, run by the desktop app with an adb.exe it finds or the user points to. Google's platform-tools are not bundled. | The Android SDK licence restricts redistribution of the binary package (verify the wording). |
| C12 | Ports: TCP 47800 (session), UDP 47801 (discovery, Wi-Fi only). Tests use TCP 47811 or an ephemeral port. | A test never collides with a running real app. |
| C13 | In `--screenshot` runs the server binds 127.0.0.1 only, never starts UDP, Bluetooth or adb (except a serial given by `WTE_COMPANION_ADB_SERIAL`), and ignores stored prefs. Enforced in code and unit-tested. | Hard rules: no firewall dialog, never touch the user's tablet. |
| C14 | Bluetooth code (Windows RFCOMM FFI, Android `BluetoothSocket`) is written and compiled, but never executed by agents. | Hard rule: no advertising or pairing on this PC. |
| C15 | Android: AGP 8.7.3, Kotlin 2.0.21, Gradle wrapper 8.10.2, JDK 17, compileSdk/targetSdk 35, minSdk 29, package `io.github.maviman.wtecompanion`, name "WTE Companion". | AGP 8.7 needs Gradle 8.9 or newer; verify the pair on first build. |
| C16 | The APK is debug-signed until the user provides a release keystore. | Never create or commit keystores. |

---

## 1. Verified facts

`REG` = `G:\Claude_Resources\rust\cargo\registry\src\index.crates.io-1949cf8c6b5b557f`.

**Capture**

1. `Context::request_screenshot(callback)` (`REG\egui-0.37.0\src\context.rs:4557`) sends `ViewportCommand::Screenshot`. `send_viewport_cmd_to` also calls `request_repaint_of`.
2. The command lands in `viewport.actions_requested` after the frame that sent it. The next frame's paint serves it (`REG\eframe-0.37.0\src\native\wgpu_integration.rs:685-690, 824-840`). The capture is therefore always of the frame after the request (ARCHITECTURE 8.13 pitfall 5).
3. A pending Screenshot makes a hidden window run the ui and paint (`ActionRequested::wants_paint`, `REG\egui-winit-0.37.0\src\lib.rs`). `RequestPaintWhileHidden` is not needed in addition.
4. With a capture, egui-wgpu renders into its own texture, copies it to a new buffer of w×h×4 bytes, and blits to the surface if there is one (`REG\egui-wgpu-0.37.0\src\capture.rs:122-187`, `winit.rs:600-670`).
5. The surface format must be Rgba8Unorm or Bgra8Unorm (`capture.rs:201-209`). The self-test already relies on this.
6. Readback is `map_async`; the callback converts every pixel into `Vec<Color32>` on the thread that polls the device (`capture.rs:211-247`). Nothing in eframe or egui-wgpu calls `device.poll`. The self-test polls itself (`crates/wte_app/src/selftest/mod.rs:428`).
7. UI-thread cost per captured frame: one extra full-screen blit, one buffer allocation, one texture-to-buffer copy, and the forced extra frame when idle. The conversion (about 2 M pixels at 1080p) runs wherever the poll happens.

**Hidden windows**

8. Minimized or invisible windows repaint at most every 100 ms plus a 10 ms sleep (`REG\eframe-0.37.0\src\native\run.rs` `INVISIBLE_WINDOW_REPAINT_INTERVAL`; `winit_integration.rs:16-30`). While hidden, eframe runs only `App::logic` unless a paint is wanted (`wgpu_integration.rs:712-760`).
9. winit 0.30.13 on Windows emits no `Occluded` event (grep: only `event.rs`). A covered window counts as visible and paints at the normal rate.
10. A minimized window has a zero-sized surface; the capture logs "Cannot capture a screenshot of a zero-sized surface" (`winit.rs:658`). Verify what `surface_state.width/height` hold when minimized.

**Input**

11. `App::raw_input_hook` (`crates/wte_app/src/app.rs:289-305`) is the injection point. It takes self-test or window pen records into `state.pen.feed`, then calls `state.pen.route(&mut raw_input.events)`.
12. `PenRouter::process` (`crates/wte_app/src/pen/route.rs:54`) treats a `Touch` as a pen when a record with the same id waits in the feed or the id is known. It emits `PointerMoved` and `PointerButton` itself, whether or not egui-winit's emulated events follow.
13. Hover is a `Touch{phase: Move}` with an `Update` record that has `in_contact = false`. A `Leave` record alone ends hover or contact (`apply_record`).
14. The router holds a single `contact`, so two pens cannot be down at once.
15. `PenRecord` has `pointer_id, kind, in_contact, pressure, barrel, third, inverted` (`pen/mod.rs:47`). It has no tilt.
16. Views read pressure per pointer event through `PenFrame::cursor` (`view/mod.rs:355-372`).
17. Navigation: a Secondary drag orbits in 3D and pans in 2D; a Middle drag pans; an unmodified wheel zooms at the pointer; a Ctrl/Shift wheel goes to the tool first (`tools/mod.rs:421-426`, `view/mod.rs:436-446`).
18. View rects are in `AppState::view_rects`; also `central_rect`, `side_panel_rect` (`state.rs:1148-1155`).
19. egui 0.37 has no `RawInput::modifiers`; modifier state travels in `Event::ModifiersChanged` (`selftest/mod.rs` comment). The self-test sets `raw.focused = true` so text fields work in a background window.

**Dependencies and structure**

20. `wte_app` depends on wte_core, eframe, glam, bytemuck, serde, serde_json, anyhow, log, env_logger, rfd. `image` (jpeg feature) and `rayon` are `wte_core` dependencies only.
21. `Cargo.lock` has no sha2, hmac or cipher crate. `rand` and `getrandom` are transitive only.
22. `wte_core::encode_png` lives in `crates/wte_core/src/image_io.rs:144`; `encode_jpeg` goes next to it.
23. winit resizes with `SWP_NOACTIVATE` (`REG\winit-0.30.13\src\platform_impl\windows\window_state.rs:475`). An inner-size request does not take focus.
24. Dialogs are modal (`dialogs/mod.rs` `modal(..)`, `state.rs:1035` `Dialog`). Help > "Tablet setup" exists (`dialogs/tablet.rs`).
25. The CI workflow `.github/workflows/installer.yml` has jobs build (windows), smoke (windows) and release (ubuntu, `v*` tags only, `needs: [build, smoke]`).

---

## 2. Architecture (desktop)

### 2.1 Module layout

New directory `crates/wte_app/src/companion/`:

| File | Content | Pure? |
|---|---|---|
| `mod.rs` | `Companion` (owned by `WteApp`), per-frame hooks, start/stop, status for the UI | no |
| `proto.rs` | Message types, encode/decode, `FrameReader` (incremental framing), key table | yes |
| `crypto.rs` | SHA-256, HMAC-SHA256, PBKDF2, constant-time compare, hex, Crockford code; `random_bytes` (FFI) | yes, except random |
| `auth.rs` | Server handshake state machine, `PairStore`, pairing window, attempt limits | yes |
| `encode.rs` | RGBA→RGB, box downscale, 64-px cell diff, rectangle merge, JPEG, `RateController`, link profiles | yes |
| `capture.rs` | Capture scheduler (UI thread), single-slot mailbox, poll helper | no |
| `input.rs` | Remote input → egui events and `PenRecord`s; pointer arbiter; modifier merge; gesture mapper; coordinates | yes |
| `server.rs` | Listeners, session threads, `Link` trait | no |
| `discovery.rs` | UDP responder; packet encode/decode pure | mostly |
| `adb.rs` | Find adb, `devices -l` parser, `reverse`; hidden process spawn | parsers pure |
| `bt_win.rs` | `#[cfg(windows)]` Winsock RFCOMM listener (raw-dylib `ws2_32`) | no; never run by agents |
| `client.rs` | `TestClient`: a minimal protocol client used by tests and the `companion` scenario | no |

Also new: `dialogs/companion.rs` (the dialog) and `selftest/scenarios/companion.rs`.

Edited: `app.rs` (hooks), `state.rs` (`Dialog::Companion`, `CompanionPrefs`, status), `action.rs` and `dispatch.rs` (`AppAction::Companion*`), `panels/menu_bar.rs` (Help > "Tablet companion..."), `panels/status_bar.rs` (chip), `prefs.rs`, `cli.rs`, `crates/wte_core/src/image_io.rs` (`encode_jpeg`).

### 2.2 Threads

```
UI thread (eframe)
  raw_input_hook: companion.inject(events, pen.feed, raw_input)     <- mpsc from the reader thread
  logic:          companion.logic(ctx, state, gpu)                   capture scheduling, requests (fit, undo), state changes
  ui:             dialog, status chip

listener threads   one per enabled transport: TCP 127.0.0.1, TCP LAN (Wi-Fi on), UDP discovery (Wi-Fi on),
                   RFCOMM accept (Bluetooth on), adb watcher (USB on). Non-blocking accept with a 50 ms poll and a stop flag.
session: reader    handshake, then decode C2S messages -> mpsc -> ctx.request_repaint()
session: writer    waits for (captured frame in mailbox) and (ack window open): diff, encode, send.
                   Also sends STATE, CURSOR, PONG. Owns the encoder state.
session: poller    while a capture is outstanding: device.poll(PollType::Poll) every 1 ms (max 250 ms).
                   The map_async callback, and so the pixel conversion, runs here.
encode workers     std::thread::scope, up to min(4, cores - 2), per frame, only when there are 2 or more rectangles
```

- No thread other than the UI thread touches `AppState`.
- Shared between UI and session: `Arc<Shared>` with a mutexed mailbox, atomics (`want_frame`, `last_input_seq`, flags) and two mpsc channels.
- `Link` trait: `Read + Write + Send` plus `try_clone` and `shutdown`. Implemented for `TcpStream` and the RFCOMM socket wrapper. TCP gets `set_nodelay(true)`.
- Poller fallback: if cross-thread `device.poll` causes trouble, poll at the start of `logic()` and call `ctx.request_repaint_after(4 ms)` while a capture is outstanding. Decide by measurement and record the result in `docs/notes/companion/capture.md`.

### 2.3 Frame capture (`capture.rs`)

State on the UI thread: `dirty`, `pending` (a capture requested, not yet delivered), `capture_pass` (the pass number that will be captured), `next_allowed: Instant`.

Rule, evaluated in `Companion::logic` every frame while a session is in the streaming state:

1. If this pass is `capture_pass`, do nothing (it is being captured). Otherwise set `dirty = true`: every other frame may have changed the picture.
2. If `dirty && !pending && shared.want_frame`:
   - If `now >= next_allowed`: call `ctx.request_screenshot(cb)`, set `pending = true`, `dirty = false`, `capture_pass = this + 1`, `next_allowed = now + 1/fps`. Start the poller.
   - Otherwise call `ctx.request_repaint_after(next_allowed - now)`.
3. The callback puts `Captured { image: Arc<ColorImage>, last_input_seq, ppp, t_capture }` into the mailbox, replacing an older one. It clears `pending` through an atomic and calls `ctx.request_repaint()` only if `dirty` is set.

Invariant: every painted frame is either captured or causes a capture of a later frame, so the stream always ends on the latest picture and stops when the app is idle. `last_input_seq` is sampled in `inject()` of the captured pass.

The writer sets `want_frame` when fewer than `window` frames are unacknowledged. An idle app with a connected tablet therefore costs nothing.

Budget, measured by the builder with `frame_profile` and recorded in notes: at most 2 ms of added UI-thread time per captured frame at 1920×1080; a mouse-only brush stroke with a tablet connected stays within 10 % of the frame time without one.

Window states:

| State | Behaviour |
|---|---|
| Visible or covered by another window | Normal (fact 9). |
| Off-screen (self-test window) | Normal (this is how the self-test captures). |
| Resized | The next capture has a new size: the writer sends STATE with the new surface size, then a key FRAME. |
| Minimized | No picture. STATE flag `NO_PICTURE` is set and the tablet shows "The desktop window is minimized". Input is ignored. On restore: key frame. Detect with `ctx.input(|i| i.viewport().minimized)`. |
| Unfocused | Remote input still works. `raw_input.focused` is forced true while the remote is active (section 5.5). |

"Fit to tablet" (optional, from the tablet's Fit button or the dialog):

- `ViewportCommand::InnerSize` with the tablet's aspect ratio, keeping the current height and clamped to the monitor's work area. Coordinate with `cap_window_size` and `fit_window_to_monitor` in `app.rs`.
- Only when the window is neither maximized nor fullscreen; otherwise STATE flag `FIT_UNAVAILABLE`.
- The old size is remembered and restored on "Restore", on disconnect and when the feature is turned off.
- Never send `Focus`, `Minimized(false)` or any other focus-changing command.

### 2.4 Encoding (`encode.rs`, writer thread)

1. Convert `ColorImage` (premultiplied RGBA, alpha ignored) to RGB, box-downscaled by an integer `den` in {1, 2, 3, 4} (picture = surface / den, rounded up).
2. Compare with the last sent picture in 64-px cells. No changed cell: send an empty FRAME (0 tiles) only if `last_input_seq` advanced, otherwise nothing.
3. Merge changed cells into rectangles (row runs, then equal runs vertically), each at most 512×512 and aligned to the 64 grid.
4. JPEG-encode each rectangle with `wte_core::encode_jpeg(w, h, rgb, quality)`, in parallel when there are two or more.
5. Send one FRAME message and copy the sent rectangles into the last-sent picture.
6. Refine: track the quality each cell was last sent with. After 400 ms without a changed cell, re-send cells below the refine quality in one FRAME with the `REFINE` flag.
7. Key frame (first frame, after a size or scale change, after reconnect): all cells, flag `KEY`.

Link profiles (ceiling; the controller moves below it):

| Link | den | Motion quality | Refine quality | Max fps | Ack window |
|---|---|---|---|---|---|
| USB | 1 | 80 | 92 | 30 | 2 |
| Wi-Fi | 1 | 75 | 92 | 30 | 2 |
| Bluetooth | 2 | 50 | 80 | 8 | 1 |

`RateController` (pure):

- Input per ack: bytes sent, send-to-ack time, and the tablet's decode time.
- If send-to-ack exceeds 150 ms three times in a row: step down (quality −15 to a floor of 40, then `den + 1` to a maximum of 4, then halve fps to a floor of 4).
- If it stays under 50 ms for 2 s: step up one level, never above the profile.
- A `den` change forces a key frame.
- CONFIG `quality_pref` from the tablet (low / medium / high) lowers the ceiling.

`wte_core` addition (ARCHITECTURE section 5): `pub fn encode_jpeg(width: u32, height: u32, rgb: &[u8], quality: u8) -> Result<Vec<u8>, ImageIoError>` using `image::codecs::jpeg::JpegEncoder::new_with_quality` with `ExtendedColorType::Rgb8`. Errors follow `encode_png`. Unit tests: round trip through the existing decoder within tolerance; wrong length and zero size fail.

---

## 3. Desktop UI

**Menu.** Help > "Tablet companion..." under "Tablet setup" → `Dialog::Companion`. The existing "Tablet setup" help gets one new paragraph pointing to it.

**Status bar.** A chip, shown only while any link is enabled:

- "Tablet: waiting"
- "Tablet: Galaxy Tab S8 Ultra (USB)"
- "Tablet: pairing"

Clicking it opens the dialog.

**Dialog** (`dialogs/companion.rs`, modal like the others, with a scroll area):

- **Three switches**
  - "USB cable", with the adb status line: "adb found: path", "No adb found" plus [Browse for adb.exe...] and a link text to the platform-tools page, "Tablet connected: unlock it and tap Allow USB debugging", or "Ready: open WTE Companion on the tablet and choose USB".
  - "Wi-Fi", with this PC's name, its LAN addresses and the port.
  - "Bluetooth", with "No Bluetooth adapter" or "Ready: pair the tablet in Windows Settings once, then choose Bluetooth in the tablet app".
- **Pairing**
  - While the dialog is open with Wi-Fi on, a code "K7QF-3M9T" and "Type this code on the tablet".
  - For USB and Bluetooth, when an unknown tablet asks: "<name> wants to connect" with [Allow] and [Deny].
- **Paired tablets**: a list with name, date and [Forget].
- **Options**
  - "Let the tablet resize this window to its shape" (default on).
  - "Fingers": "Navigate in the view, click elsewhere" (default) or "Act like the mouse everywhere".
  - Port (advanced).
- **Fixed text**: "The connection is checked with a pairing key, but the picture and the input are not encrypted. Use Wi-Fi only on a network you trust; USB does not use the network."
- [Disconnect] while connected, and [Close].

**Prefs** (`CompanionPrefs` in `Prefs`, `#[serde(default)]`):

- `usb`, `wifi`, `bluetooth`: bool, all false by default.
- `port: u16 = 47800`.
- `adb_path: Option<PathBuf>`.
- `desktop_id: String` (16 random bytes as hex, created the first time a link is turned on).
- `paired: Vec<PairedTablet { id, name, token, added }>`. The token is stored as hex in the per-user prefs file, in plain text; say so in ARCHITECTURE section 7.
- `allow_resize`, `fingers`.

Enabled links persist across restarts. Turning Wi-Fi on is the user action that causes the one-time Windows Firewall prompt.

**Self-test runs (C13).** `Companion::new(selftest = true)` ignores prefs. A pure `bind_plan(selftest, prefs) -> Vec<Bind>` decides what is opened and is unit-tested: self-test with everything on yields only `Tcp(127.0.0.1)`.

---

## 4. Wire protocol v1 (save as `docs/protocol/COMPANION_PROTOCOL.md`)

Both builders follow this section exactly. Changes to it are made in the spec file first and noted in `docs/notes/companion/protocol-changes.md`.

### 4.1 Basics

- One reliable byte stream (TCP or RFCOMM). The tablet is the client.
- All integers are little-endian. `str` is a `u16` byte length plus UTF-8. `f32` is IEEE 754.
- The client first sends the 4 bytes `57 54 45 43` ("WTEC"). The server answers with the same 4 bytes. Anything else: close.
- After that, messages are `len: u32` (payload bytes, header excluded), `type: u16`, `flags: u16` (0 unless stated), then the payload.
- Limits: client-to-server payload at most 65,536 bytes, server-to-client at most 16 MiB. A larger length closes the connection.
- Coordinates are "surface pixels": physical pixels of the desktop window's content at full resolution, origin top-left, as `f32`. The desktop converts to egui points by dividing by `pixels_per_point`. The picture may be smaller than the surface (`den`); coordinates never are.

### 4.2 Versioning and compatibility rules

1. HELLO carries `proto_min` and `proto_max`, the major versions the client speaks. The server answers with the highest version both support, or ERROR `VERSION`. This document is version 1.
2. Within a version:
   - A receiver skips unknown message types by `len`.
   - A message may grow at the end; receivers ignore trailing bytes they do not know.
   - Unknown enum values and unknown flag bits are ignored.
   - New features are announced by `caps` bits and used only when the peer set the bit.
3. The desktop keeps supporting every major version a released tablet app spoke. The golden fixtures (4.9) of every released version stay in the repo and in both test suites.
4. `caps` bits, client: bit 0 PNG tiles, bit 1 WebP tiles, bit 2 draws a pen trail, bit 3 has IME text. Server: bit 0 sends CURSOR, bit 1 honours FIT_REQUEST. Bit 63 on both sides is reserved for "encrypted session".

### 4.3 Handshake

```
C: magic, HELLO
S: magic, HELLO_ACK            (or ERROR)
C: AUTH
S: AUTH_WAIT*  then AUTH_OK    (or ERROR)
C: CONFIG                      -> streaming
```

| Type | Name | Payload |
|---|---|---|
| 0x0001 | HELLO (C) | `proto_min u16, proto_max u16, caps u64, link u8 (1 usb, 2 wifi, 3 bluetooth), tablet_id [16], client_nonce [16], screen_w u16, screen_h u16, density_dpi u16, app_version str, device_name str` |
| 0x0081 | HELLO_ACK (S) | `proto u16, caps u64, desktop_id [16], server_nonce [16], auth_modes u8 (bit 0 token, bit 1 code, bit 2 confirm), pbkdf2_iters u32, app_version str, desktop_name str` |
| 0x0002 | AUTH (C) | `mode u8 (1 token, 2 code, 3 confirm), client_mac [32]` (zeros for confirm) |
| 0x0082 | AUTH_WAIT (S) | empty; sent every second while the desktop user has not answered a confirm request (at most 60 s) |
| 0x0083 | AUTH_OK (S) | `mode u8, server_mac [32] (zeros for confirm), token [32] (confirm mode only, else zeros)` |
| 0x008F | ERROR (S) | `code u16, text str`; then the server closes |

ERROR codes: 1 VERSION, 2 AUTH_FAILED, 3 PAIRING_CLOSED (unknown tablet and the dialog is not open), 4 DENIED, 5 BUSY (another tablet is connected), 6 BAD_MESSAGE, 7 TOO_MANY_ATTEMPTS.

`auth_modes` offered by the server:

- token: `tablet_id` is in the pair store.
- code: a pairing code is on display.
- confirm: the dialog is open, and either `link` is usb and the peer address is loopback, or the connection came in on RFCOMM.

Keys and MACs (HMAC = HMAC-SHA256; labels are ASCII without terminator; `‖` is concatenation):

```
T          = desktop_id ‖ tablet_id ‖ server_nonce ‖ client_nonce ‖ u16le(proto) ‖ u8(link)
code       = the 8 Crockford base32 characters, upper case, no dash   (alphabet 0123456789ABCDEFGHJKMNPQRSTVWXYZ;
             input is normalized: upper case, spaces and '-' removed, O->0, I->1, L->1)
pair_key   = PBKDF2-HMAC-SHA256(password = code, salt = "WTEC1-pair" ‖ desktop_id ‖ tablet_id ‖ server_nonce ‖ client_nonce,
                                iterations = pbkdf2_iters, 32 bytes)
mode code:    K = pair_key;  token = HMAC(pair_key, "WTEC1-token")      (derived on both sides, never sent)
mode token:   K = token
client_mac = HMAC(K, "WTEC1-client" ‖ T)
server_mac = HMAC(K, "WTEC1-server" ‖ T)
mode confirm: no MACs; after "Allow" the server creates a random token and sends it in AUTH_OK
```

- Both sides compare MACs in constant time. The client verifies `server_mac` before storing a token or sending CONFIG.
- `pbkdf2_iters` is 100,000. The client refuses values below 50,000 or above 2,000,000.
- A pairing code is valid while the dialog is open, for at most 10 minutes. After 5 failed code attempts the code is replaced, and the server answers TOO_MANY_ATTEMPTS for 30 s.
- A second authenticated connection from the same `tablet_id` replaces the old session. One from another tablet gets BUSY.
- Security statement: token and code modes prove on both sides that the peer knows the key, without sending it. The session after the handshake is neither encrypted nor integrity-protected. Confirm mode sends the token in the clear and is offered only on a USB cable (loopback through adb) or an OS-paired Bluetooth link.

### 4.4 Client → server, streaming

Every input message starts with `seq u32`, one counter per connection starting at 1 and incremented per input message.

| Type | Name | Payload |
|---|---|---|
| 0x0010 | CONFIG | `view_w u16, view_h u16 (tablet pixels available for the picture), max_fps u8 (0 = server decides), quality_pref u8 (0 auto, 1 low, 2 medium, 3 high), flags u16` |
| 0x0011 | FRAME_ACK | `frame_seq u32, decode_ms u16` — sent when the frame is decoded and applied |
| 0x0012 | PEN | `seq u32, count u16`, then `count` × sample (16 bytes): `kind u8, flags u8, pressure u16 (0..65535), x f32, y f32, tilt u8 (degrees from perpendicular, 0..90, 255 unknown), reserved u8, dt_us u16 (microseconds since the previous sample, saturating)` |
| 0x0013 | TOUCH | `seq u32, count u8`, then `count` × (10 bytes): `id u8, phase u8 (0 down, 1 move, 2 up, 3 cancel), x f32, y f32` |
| 0x0014 | KEY | `seq u32, key u16 (4.8), pressed u8, repeat u8, mods u8` |
| 0x0015 | TEXT | `seq u32, text str` |
| 0x0016 | MODS | `seq u32, mods u8` — the latched toolbar modifiers; replaces the previous value |
| 0x0017 | ACTION | `seq u32, action u16 (1 undo, 2 redo)`; unknown ids are ignored |
| 0x0018 | FIT_REQUEST | `mode u8 (0 restore, 1 fit), aspect_w u16, aspect_h u16` |
| 0x001E | PING | `t u32` (client milliseconds) |
| 0x001F | BYE | `reason u8` |

- PEN `kind`: 0 hover (in range, not touching), 1 down, 2 move (touching), 3 up (lifted, still in range), 4 leave (out of range), 5 cancel (contact aborted).
- PEN `flags`: bit 0 barrel button (S Pen side button), bit 1 eraser (inverted end, or the toolbar eraser toggle), bit 2 second barrel button.
- `mods` bits: 1 Ctrl, 2 Shift, 4 Alt.
- Every digitizer sample is sent, in order, including the historical samples of a batched `MotionEvent`.

### 4.5 Server → client, streaming

| Type | Name | Payload |
|---|---|---|
| 0x0090 | STATE | `surface_w u16, surface_h u16, pixels_per_point f32, flags u32, trail_rgba u32, title str` — sent after AUTH_OK and on every change |
| 0x0091 | FRAME | `frame_seq u32, flags u8 (bit 0 KEY, bit 1 REFINE), den u8, pic_w u16, pic_h u16, last_input_seq u32, capture_ms u32, tile_count u16`, then per tile: `x u16, y u16, w u16, h u16 (picture pixels), codec u8 (1 JPEG, 2 PNG, 3 WebP), len u32, bytes` |
| 0x0092 | CURSOR | `x f32, y f32, visible u8, icon u8` — the desktop's own mouse pointer, at most 30 per second |
| 0x009E | PONG | `t u32` (echo) |
| 0x009F | BYE | `reason u8` |

STATE flags:

- bit 0 `NO_PICTURE` (minimized)
- bit 1 `TEXT_WANTED` (a text field has the focus)
- bit 2 `TRAIL` (the current tool paints: draw the local pen trail)
- bit 3 `POINTER_HELD_LOCALLY` (the desktop mouse or pen holds the pointer; tablet contact is ignored)
- bit 4 `FIT_UNAVAILABLE`
- bit 5 `FITTED` (the window is currently resized for the tablet)

FRAME rules:

- The tablet keeps one picture of `pic_w × pic_h`. A KEY frame replaces it, and its size if different. Other frames draw their tiles over it. Tiles of one FRAME are applied together.
- The picture is shown scaled to the surface rectangle: uniform scale, centred, bars outside.
- `last_input_seq` is the last input message the desktop had processed when the frame was painted. The tablet drops trail points with `seq <= last_input_seq`.
- The tablet answers every FRAME with FRAME_ACK. The server keeps at most `window` frames unacknowledged.

### 4.6 Keep-alive, disconnect, reconnect

- The client sends PING every 1 s; the server answers PONG.
- Either side closes after 5 s without any received byte (10 s on Bluetooth).
- On any end of a session the desktop releases what the remote held (section 5.6).
- The tablet reconnects to the last endpoint by itself: delays 0.5, 1, 2, 4 s, then every 4 s, until the user leaves the canvas screen. A reconnect is a full handshake in token mode, followed by STATE and a KEY frame.

### 4.7 Discovery (UDP, Wi-Fi only)

- The tablet sends to 255.255.255.255:47801 and to the subnet broadcast address every second while its Wi-Fi screen is open: `"WTEC?" , u8 1`.
- The desktop (only while Wi-Fi is on) answers to the sender's address and port: `"WTEC!", u8 1, tcp_port u16, desktop_id [16], desktop_name str`.
- The desktop never broadcasts. No secret is in either packet.
- Manual entry of address and port is always available.

### 4.8 Key codes

| Code | Key |
|---|---|
| 1–26 | A–Z |
| 30–39 | 0–9 |
| 40–54 | Enter, Escape, Backspace, Tab, Space, Delete, Insert, Home, End, PageUp, PageDown, Left, Right, Up, Down |
| 60–71 | F1–F12 |
| 80–90 | Minus, Equals, BracketLeft, BracketRight, Comma, Period, Slash, Backslash, Semicolon, Quote, Backtick |

- Printable input is sent as KEY (for shortcuts) and, when neither Ctrl nor Alt is held, also as TEXT.
- Unknown codes are ignored by the desktop.

### 4.9 Golden fixtures

`docs/protocol/fixtures/v1/` holds one `.hex` file per message type (a complete framed message as hex text, LF) plus `handshake.json` with fixed ids, nonces, code, iterations and the expected `pair_key`, `token`, `client_mac`, `server_mac`.

- The desktop builder writes them from the Rust encoder and checks the crypto values with Python: `python -E -P` with `hashlib.pbkdf2_hmac`, `hmac`.
- Rust and Kotlin unit tests both decode and re-encode every fixture byte for byte.

---

## 5. Input into the app (`input.rs`)

`Companion::inject` runs in `raw_input_hook` after the self-test or window records are taken and before `state.pen.route(..)`. It is a pure function over (local events, queued remote messages, arbiter state, view rects, ppp) and returns the new event list, the pen records to append and the flags.

### 5.1 Pen

Reserved ids: `REMOTE_PEN_ID: u32 = 0x5754_4501`, `TouchDeviceId(0x5754_4543)`.

| PEN kind | `Event::Touch` (id = REMOTE_PEN_ID, `force = Some(p)`) | `PenRecord` |
|---|---|---|
| hover | `Move` | `Update, in_contact: false, pressure: 0` |
| down | `Start` | `Down, in_contact: true, pressure: p` |
| move | `Move` | `Update, in_contact: true, pressure: p` |
| up | `End` | `Up, in_contact: false` |
| leave | none | `Leave` |
| cancel | `End` at the last position | `Up`, then `Leave` |

- `barrel` and `inverted` come from the flags; `third` from flag bit 2.
- Pressure is `u16 / 65535`, with a floor of 1/1024 while in contact, because the router treats 0 as "no pressure known".
- Position is surface pixels divided by `ppp`.
- Tilt is decoded and dropped; `PenRecord` has no tilt (nice-to-have).
- The eraser end and the toolbar eraser toggle use the existing `eraser_override`. The barrel button gives Secondary: orbit in 3D, pan in 2D, context menus. Nothing new is needed in the router.
- All samples of all queued PEN messages go into one frame, in order.

### 5.2 Pointer arbiter (C7)

State: `holder: None | Local | Remote`, `local_press_dropped: bool`.

- A local press (`PointerButton{pressed: true}` or a local `Touch Start`) while `holder == None` sets `Local`; its release sets `None`.
- A remote pen down or a finger-gesture press while `holder == None` sets `Remote`; its release sets `None`.
- While `Local`: remote pen down, move, up and gesture presses are dropped. Hover is dropped too. STATE flag `POINTER_HELD_LOCALLY` is set so the tablet can show a small "desktop is drawing" mark.
- While `Remote`: local `PointerMoved`, `PointerButton`, `MouseMoved`, `Touch`, `MouseWheel` and local pen records are dropped. A dropped local press sets `local_press_dropped`, and its release is dropped too. Keyboard events pass.
- While `None`: both sides' hover moves pass in arrival order (remote after local within a frame).

### 5.3 Fingers (desktop-side gesture mapper)

Input: raw TOUCH points, `view_rects` of the last frame, the pref `fingers`, and pen proximity. Output: mouse and wheel events, marked remote for the arbiter.

| Gesture | Over an editing view (default mode) | Elsewhere, or mode "act like the mouse" |
|---|---|---|
| One finger down / move / up | After 6 points of travel: Secondary press and drag (orbit in 3D, pan in 2D). A tap does nothing. | Primary press at once, drag, release (buttons, sliders, list rows, menus). |
| Two-finger drag | Middle press and drag at the centroid (pan). | `MouseWheel{unit: Point}` by the centroid delta (scrolls lists). |
| Pinch | `MouseWheel{unit: Line}` at the centroid: `notches = log2(distance ratio) × 4`, sent in steps of at least 0.1 notch (zoom at the pointer). | Nothing. |
| Three or more fingers | Ignored; a running gesture is released. | Same. |

- Going from one finger to two releases the one-finger button first, at its last position.
- Before any gesture press and before a wheel event a `PointerMoved` to that position is sent.
- Palm rejection: while the remote pen is in range (hover or contact) and for 300 ms afterwards, new finger touches are ignored and a running one-finger gesture is released. `cancel` phases release without a click.
- Fingers never become `Event::Touch` in egui, so egui's touch mode is never entered.
- Verify that a Secondary press and release without movement opens no unwanted context menu in a view. If it does, keep the 6-point threshold as the only press trigger.

### 5.4 Keyboard, text, modifiers, actions

- KEY → `Event::Key { key, physical_key: None, pressed, repeat, modifiers }`. The table maps codes to `egui::Key`; unknown codes are dropped.
- TEXT → `Event::Text`.
- Modifiers: effective = local OR remote (toolbar MODS OR the `mods` of the last KEY). The stage tracks the local state from passing `ModifiersChanged` events and rewrites or emits `ModifiersChanged(effective)` whenever the effective value changes. Remote `PointerButton`s carry the effective modifiers; the router takes them from the last `ModifiersChanged`.
- ACTION 1 and 2 → `AppAction::Undo` / `AppAction::Redo` through `dispatch` in `logic()`, not while the tools are busy.
- FIT_REQUEST → section 2.3, if the pref allows it.

### 5.5 Focus

While a session is streaming and remote input arrived in the last 5 s (or a remote-focused text field exists), `raw_input.focused = true` and `Event::WindowFocused(false)` is removed.

Verify in `REG\egui-0.37.0\src` what `focused == false` does to the focused widget. The self-test does the same for the same reason.

### 5.6 End of a session, for any reason

In the next `inject`:

- If the remote pen is known: a `Leave` record (the router releases an open contact).
- Release any gesture button.
- Remote modifiers go to 0; `holder` goes to `None` if it was `Remote`.
- Restore a fitted window.

Unit-tested: a disconnect in mid-stroke leaves no drag open.

---

## 6. Transports

### 6.1 USB (primary)

Design: the desktop listens on 127.0.0.1:47800. `adb reverse tcp:47800 tcp:47800` makes the tablet's 127.0.0.1:47800 reach it. The tablet app connects to 127.0.0.1:47800 with `link = usb`.

adb is found in this order:

1. The prefs path.
2. `ANDROID_HOME` / `ANDROID_SDK_ROOT` + `\platform-tools\adb.exe`.
3. `PATH`.
4. `%LOCALAPPDATA%\Android\Sdk\platform-tools\adb.exe`.
5. `platform-tools\adb.exe` next to the app's exe (for a user who unpacks Google's zip there).

Not bundled (C11). Text in the dialog and Help: download "SDK Platform-Tools for Windows" from developer.android.com, unpack, [Browse for adb.exe...].

Watcher thread, while USB is on:

- Every 2 s (5 s while a session runs) it runs `adb devices -l`.
- For every serial in state `device` it runs `adb -s SERIAL reverse tcp:PORT tcp:PORT` once per appearance.
- `unauthorized` gives the "tap Allow USB debugging" line; `offline` gives "reconnect the cable".

Process spawning:

- `std::process::Command` with `creation_flags(CREATE_NO_WINDOW = 0x0800_0000)`, stdin null, stdout and stderr piped, and a 5 s timeout. A console window never appears.
- The adb server daemon that adb starts is left running; we never kill it.

Self-test: the watcher runs only if `WTE_COMPANION_ADB_SERIAL` is set, and then only for that serial.

User steps (README, Help, HOW_TO_TEST):

1. On the tablet: Settings > About tablet > Software information > tap "Build number" seven times.
2. Settings > Developer options > USB debugging: on.
3. Connect the cable; on the tablet tap "Allow" (tick "Always allow from this computer").
4. In the editor: Help > Tablet companion > USB cable on. The line reads "Ready".
5. On the tablet: open WTE Companion, tap USB. On the desktop: Allow (first time only).

If Windows does not list the tablet: install the Samsung Android USB driver (hand test H2).

Alternative with no adb, documented only: USB tethering makes the cable a network link, used through the Wi-Fi mode with a manual address.

### 6.2 Wi-Fi

- When the switch is on, the server binds `0.0.0.0:PORT` (TCP) and `0.0.0.0:47801` (UDP). When it is off, both sockets are closed.
- The first time, Windows shows its firewall prompt for this exe path. It is caused by the user's click. Help explains: allow on private networks; if it was dismissed, allow the app under Windows Security > Firewall > Allow an app.
- The per-user installer cannot add a firewall rule (no admin rights), so the installer needs nothing extra.
- LAN addresses shown in the dialog come from a UDP socket `connect` to a public address followed by `local_addr()`. No packet is sent. Used only while Wi-Fi is on and never in self-test runs.
- Pairing: code mode (4.3). Afterwards token mode, with no typing.
- The stream is not encrypted (C9).

### 6.3 Bluetooth (RFCOMM)

**Windows** (`bt_win.rs`, raw-dylib `ws2_32`; take struct definitions from `REG\windows-sys-0.61.2` as reference only):

1. `WSAStartup(2.2)`.
2. `socket(AF_BTH = 32, SOCK_STREAM = 1, BTHPROTO_RFCOMM = 3)`.
3. `bind` with `SOCKADDR_BTH { addressFamily: 32, btAddr: 0, serviceClassId: zero GUID, port: BT_PORT_ANY (0xFFFF_FFFF) }`. The struct is packed(1) and 30 bytes long.
4. `listen(1)`, then `getsockname` for the channel.
5. `WSASetServiceW(&WSAQUERYSETW { dwSize, lpszServiceInstanceName: "Wrapped Texture Editor companion", lpServiceClassId: &SERVICE_UUID, dwNameSpace: NS_BTH (16), dwNumberOfCsAddrs: 1, lpcsaBuffer: &CSADDR_INFO { LocalAddr, iSocketType: SOCK_STREAM, iProtocol: BTHPROTO_RFCOMM } }, RNRSERVICE_REGISTER, 0)`.
6. Accept loop: `ioctlsocket(FIONBIO)` plus a 50 ms poll. The accepted socket is wrapped as a `Link` (`recv` / `send`; `shutdown` + `closesocket`).
7. On switch-off: `RNRSERVICE_DELETE`, close.

Service UUID (both sides): `7c1f0d6a-5b0e-4e7a-9c56-57544543a001`.

Adapter check for the dialog: creating the AF_BTH socket succeeds or fails (read-only).

Agents compile this file and unit-test only the struct sizes and layouts, the GUID byte order and the address parsing. They never turn the switch on (C14).

**User steps:** pair the tablet with the PC once in Windows Settings > Bluetooth & devices (the OS dialog). Then turn the switch on in the editor and choose the PC in the tablet app's Bluetooth list. The first time, click Allow on the desktop.

**Android:** `BluetoothAdapter.getBondedDevices()`, `createRfcommSocketToServiceRecord(UUID)`, `cancelDiscovery()`, `connect()` on a worker thread. Permission `BLUETOOTH_CONNECT` (API 31+, asked at the first tap on Bluetooth); `BLUETOOTH` with `maxSdkVersion="30"`. No scan permission: only bonded devices are listed.

Low-bandwidth profile as in 2.4. The pen stays responsive because input goes out immediately and the local trail hides the picture delay.

---

## 7. Android app (`android/`)

### 7.1 Project

```
android/
  settings.gradle.kts, build.gradle.kts, gradle.properties, gradlew, gradlew.bat, gradle/wrapper/*
  app/build.gradle.kts
  app/src/main/AndroidManifest.xml
  app/src/main/kotlin/io/github/maviman/wtecompanion/
     proto/   Messages.kt, Framing.kt, Keys.kt, Auth.kt (platform Mac / SecretKeyFactory "PBKDF2WithHmacSHA256")
     net/     Link.kt (TcpLink, BluetoothLink), Session.kt (reader and writer threads, reconnect), Discovery.kt
     ui/      ConnectActivity.kt, CanvasActivity.kt, MirrorView.kt (SurfaceView), Toolbar.kt, Prefs.kt
     input/   PenInput.kt, TouchInput.kt, KeyInput.kt, Mapping.kt (letterbox maths, pure)
  app/src/main/res/  (strings, vector icon, themes)
  app/src/test/kotlin/...          JVM unit tests (JUnit 4)
  app/src/androidTest/kotlin/...   instrumented tests (androidx.test runner)
```

- Plugins: `com.android.application` 8.7.3, `org.jetbrains.kotlin.android` 2.0.21.
- Dependencies: none in `implementation` beyond the Kotlin stdlib. `testImplementation junit:junit:4.13.2`. `androidTestImplementation androidx.test:runner`, `androidx.test:core`, `androidx.test.ext:junit` (needed for `AndroidJUnitRunner`). Justify each in `docs/notes/companion/android-deps.md`. No Compose, no AppCompat; platform `Activity` and views.
- `proto/` and `input/Mapping.kt` use only `java.*`, `javax.crypto` and Kotlin, so JVM tests cover them.
- `versionName "1.0.0"`, `versionCode 1`. The tablet app has its own version; a desktop release does not require a new APK.
- Git:
  - `.gitignore` for `android/.gradle/`, `android/**/build/`, `android/local.properties`, `*.keystore`, `*.jks`, `android/.idea/`, `*.apk`.
  - `.gitattributes` with `android/gradlew text eol=lf`, `*.bat text eol=crlf`, `*.jar binary`.
  - Generate the wrapper with the local Gradle (`gradle wrapper --gradle-version 8.10.2`).
- Local builds: `JAVA_HOME`, `ANDROID_HOME`, `GRADLE_USER_HOME` as given; run `G:\Claude_Resources\android\gradle-8.10.2\bin\gradle.bat -p android assembleDebug testDebugUnitTest assembleDebugAndroidTest --no-daemon`, low priority, stdin closed. `local.properties` is not committed; `ANDROID_HOME` is enough.

### 7.2 Manifest

- Permissions: `INTERNET`; `BLUETOOTH_CONNECT`; `BLUETOOTH` (maxSdkVersion 30).
- `uses-feature`: bluetooth and stylus, both `required="false"`.
- `ConnectActivity` is the launcher.
- `CanvasActivity`: `screenOrientation="sensorLandscape"`, `configChanges="orientation|screenSize|keyboard|keyboardHidden|navigation|uiMode"`, immersive (`WindowInsetsController.hide(systemBars())`, behaviour `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`), `FLAG_KEEP_SCREEN_ON` while connected, cutout mode `shortEdges`.
- Icon: one adaptive vector (a tablet outline with a pen stroke); no bitmaps.

### 7.3 Connect screen

- Three large buttons.
  - **USB**: connects to 127.0.0.1:port, retrying every second, with the short setup steps beside it.
  - **Wi-Fi**: a list of discovered desktops (name, address), plus "Enter address". An unknown desktop leads to the code field (8 characters; auto-dash; accepts lower case).
  - **Bluetooth**: the list of bonded devices.
- "Last connection" at the top, one tap. It auto-connects on start if the last session ended by an error rather than by "Disconnect".
- Stored in `SharedPreferences`: `tablet_id` (16 random bytes from `SecureRandom`, created once); per desktop id: name, token, last link, last address; settings (pen button mode, quality, show latency).
- Error texts come from the ERROR codes, for example:
  - PAIRING_CLOSED → "On the desktop open Help > Tablet companion, then try again".
  - VERSION → "This app is too old or too new for the desktop app: update".

### 7.4 Canvas (`MirrorView`, a `SurfaceView`)

- **Decode thread**
  - Reads FRAME, decodes each tile with `BitmapFactory.decodeByteArray` (codec-agnostic; `inMutable`, reuse via `inBitmap` when sizes match).
  - Draws the tiles into the back bitmap (`ARGB_8888`, `pic_w × pic_h`) under a lock, and sends FRAME_ACK with the decode time.
  - Marks the view dirty.
- **Render thread**
  - Woken by dirty marks (frame, pen move, toolbar).
  - `holder.lockHardwareCanvas()`: black background, then the back bitmap with the letterbox matrix (bilinear filter), the desktop cursor mark (CURSOR), the pen trail, the pen dot and the toolbar.
- **`Mapping.kt`** (pure): `scale = min(viewW / surfaceW, viewH / surfaceH)`, offsets centred; view → surface and back. Positions outside the picture are clamped for moves in contact; a down outside is ignored.
- **Latency hiding**
  - While STATE has `TRAIL`: pen contact samples are appended to a trail (surface coordinates, tagged with their PEN `seq`), drawn as a 2 dp line in `trail_rgba` with a dark 1 dp outline.
  - Points are removed when a FRAME's `last_input_seq` reaches their `seq`, or after 700 ms.
  - A small ring is always drawn at the pen position (hover and contact) at once.
- **Other states**
  - `NO_PICTURE` shows a centred text over the last picture, dimmed.
  - A lost connection shows "Reconnecting..." over the last picture.
- **Optional overlay** (setting): fps, kbit/s, ping.

### 7.5 Input

**Pen** (`TOOL_TYPE_STYLUS`, `TOOL_TYPE_ERASER`) in `onTouchEvent` and `onGenericMotionEvent` / `onHoverEvent`:

- On `ACTION_DOWN`: `requestUnbufferedDispatch(event)`. On API 31+ also `requestUnbufferedDispatch(InputDevice.SOURCE_STYLUS)` once, for hover.
- For every event: all historical samples (`getHistoricalX/Y/Pressure/AxisValue(AXIS_TILT)/EventTime`), then the current one. One PEN message per `MotionEvent`, written at once; the writer thread takes from a queue and the UI thread never blocks on the socket.
- Mapping:
  - `ACTION_HOVER_ENTER/MOVE` → hover; `ACTION_HOVER_EXIT` → leave, unless a down follows in the same batch.
  - `ACTION_DOWN` → down; `MOVE` → move; `UP` → up; `ACTION_CANCEL` or `FLAG_CANCELED` → cancel.
- Flags: barrel = `buttonState and BUTTON_STYLUS_PRIMARY`; second barrel = `BUTTON_STYLUS_SECONDARY`; eraser = tool type ERASER, or the toolbar eraser toggle, or the setting "S Pen button erases" with the button held.
- Pressure is `getPressure()` clamped to 0..1.

**Fingers** (`TOOL_TYPE_FINGER`):

- Raw TOUCH messages with pointer ids for every pointer and phase (historical positions included for moves).
- Touches that start on the toolbar are handled locally.
- A mouse attached to the tablet is a nice-to-have.

**Toolbar** (overlay strip on the left edge, 56 dp wide, drawn in the canvas):

- Undo, Redo (ACTION).
- Ctrl, Shift, Alt: tap latches for the next pen or finger action, double tap locks; sent as MODS.
- Eraser toggle.
- Keyboard: shows the soft keyboard.
- Fit / Restore (FIT_REQUEST with the view's aspect).
- Hide: collapses to a small handle that brings it back.
- Disconnect.

Finger and pen taps both work on it.

**Keyboard:**

- Hardware keys in `dispatchKeyEvent`: KEY from a `KEYCODE_*` table to protocol codes, with `repeatCount`, plus TEXT from `getUnicodeChar()` when it is printable and neither Ctrl nor Alt is held.
- Soft keyboard: `MirrorView.onCreateInputConnection` returns a `BaseInputConnection` (no composing, `TYPE_TEXT_VARIATION_VISIBLE_PASSWORD | TYPE_TEXT_FLAG_NO_SUGGESTIONS` so text is committed per character). `commitText` → TEXT; `deleteSurroundingText` → Backspace KEY; Enter → KEY.
- When STATE gets `TEXT_WANTED`, the keyboard button is highlighted. Opening the keyboard automatically is a nice-to-have.

---

## 8. Tests

### 8.1 Rust unit tests (in-file, pure, no sockets unless stated)

| Area | Tests |
|---|---|
| `crypto` | SHA-256 of "" and "abc" (FIPS 180), a 1,000,000 × "a" vector; HMAC RFC 4231 cases 1, 2, 3, 6; PBKDF2-HMAC-SHA256 "password"/"salt" with 1, 2 and 4096 iterations. All expected values are generated and cross-checked with Python `hashlib` / `hmac`, not typed from memory. Code generation alphabet; normalization (lower case, dashes, O/I/L); constant-time compare. |
| `proto` | Every message: encode → decode → equal. `FrameReader` fed 1 byte at a time and in arbitrary splits. Unknown type skipped. Longer payload accepted, trailing bytes ignored. Too-long length rejected. Truncated payload = error. All fixtures byte for byte. |
| `auth` | Token mode succeeds and fails. Code mode derives the same token on both sides. Wrong code counts attempts; 5 failures replace the code. Confirm only with the dialog open and loopback+usb or RFCOMM. PAIRING_CLOSED, BUSY, same-tablet replace. Version negotiation (min/max ranges, no overlap). |
| `encode` | RGBA→RGB; box downscale 1/2/3/4 including odd sizes; cell diff (none, one pixel, borders, size change = key); rectangle merge (cover exactly the changed cells, at most 512, on the grid); JPEG rectangles decode back within tolerance; refine picks only low-quality cells; `RateController` steps down and up and respects the ceiling. |
| `input` | Pen mapping table → router: a full stroke through `PenRouter::process` gives the same events and pressures as the equivalent local pen (reuse the router test helpers). Hover; eraser flag; barrel = Secondary; cancel. Coordinates with ppp 1, 1.25, 2. Arbiter (local first, remote first, dropped local press and its release, hover from both). Modifier merge. Key table (every code maps; unknown dropped). Gestures (tap, drag over view / over panel, two-finger pan, pinch notches, 1→2 finger change, palm rejection, cancel). Session end releases everything. |
| `adb` | `devices -l` parser (device, unauthorized, offline, emulator, empty, daemon start lines); search order with a fake environment; the command line built for a serial. |
| `discovery` | Packet encode/decode; garbage ignored. |
| `bind_plan` | Self-test = loopback TCP only, for every pref combination; Wi-Fi off = no LAN bind and no UDP. |
| `bt_win` (windows) | `size_of::<SOCKADDR_BTH>() == 30`, field offsets, `WSAQUERYSETW` size on x64, UUID → GUID bytes. |
| `wte_core` | `encode_jpeg` (2.4). |

### 8.2 Rust loopback tests (real sockets on 127.0.0.1, ephemeral port)

`companion::server` is started with a fake "UI side" (channels, no egui). `TestClient` then covers:

- Handshake in all three modes; wrong token; wrong code.
- Version too new: the client speaks only 2..3.
- A client with unknown capability bits and a longer HELLO.
- Frames: the fake UI pushes two synthetic `ColorImage`s; the client receives a KEY frame, then only the changed rectangle; decodes it with `wte_core`'s decoder; acks.
- The window is respected: without acks at most `window` frames are sent.
- Input: PEN, TOUCH, KEY, TEXT, MODS and ACTION arrive on the UI channel in order with their `seq`.
- Keep-alive timeout, with shortened timeouts through a test config.
- Reconnect replaces the session.
- UDP discovery answered on 127.0.0.1.

### 8.3 Self-test scenarios

Add to `NAMES` and to the ARCHITECTURE 8.11 table.

**`companion`** (in-process, loopback, no Android). Sphere sample, 2D view, brush set as in the `pen` scenario (size 40, pressure → size). The scenario starts the server on 127.0.0.1:0 with auto-allow and starts a `TestClient` thread. The client:

1. Pairs in confirm mode.
2. Waits for the KEY frame.
3. Sends a pen stroke with a pressure ramp 0.1 → 1.0 along the 2D ramp row (coordinates computed by the scenario from `view_rects`).
4. Sends an eraser-flag stroke, a barrel drag, a two-finger pinch, Ctrl+Z through ACTION, and a KEY/TEXT pair into a text field.
5. Disconnects in the middle of a last stroke.

Asserts:

- The ramp is thin at 20 % and full width at the end (reuse `assert_ramp`).
- `pen_contact_frames > 0`.
- The eraser frames switched the tool and it was restored.
- The barrel drag orbited and painted nothing.
- The pinch changed the zoom.
- Undo removed the last stroke.
- After the disconnect the tools are idle and no button is down.
- The client received at least 2 frames, the last with `last_input_seq` equal to its last input; the decoded KEY frame has the window's size.
- The status chip text.

Shots: `companion_dialog` (the dialog with the code and paired list); the final screenshot shows the ramp. Add `Step::Until(name, fn) -> bool` if the runner has no polling step.

**`companion_host`** (for the Android end-to-end run). Same document setup, window 1280×800. It listens on 127.0.0.1:`WTE_COMPANION_PORT` (default 47811) with `WTE_COMPANION_TEST_CODE` as the fixed pairing code and `WTE_COMPANION_AUTO_ALLOW=1`. These variables are honoured only in `--screenshot` runs.

- It waits, with a run timeout of 300 s, until `WTE_COMPANION_EXPECT_SESSIONS` sessions have each delivered a pen stroke with varying pressure.
- It asserts paint from each, takes the screenshot and exits.
- It logs the surface coordinates of the 2D view's centre and of the stroke row, so the Android test and the script use the same numbers.

### 8.4 Android JVM unit tests

- Protocol encode/decode of every message; `FrameReader` with split input; all fixtures byte for byte.
- `handshake.json` values (PBKDF2, HMAC, token).
- Code normalization.
- `Mapping` (letterbox for wider and taller surfaces, round trip, clamping).
- Key table.
- Trail pruning by `last_input_seq` and by age.
- Reconnect back-off sequence.
- PEN batch building from a fake sample list (history order, `dt_us` saturation).

### 8.5 Instrumented tests (emulator only)

- Build with Gradle; install and run with explicit adb commands only. Never `connectedAndroidTest`, because it runs on every attached device:
  - `adb -s emulator-5554 install -r app-debug.apk`, and the same for the test APK.
  - `adb -s emulator-5554 shell am instrument -w -e class ... io.github.maviman.wtecompanion.test/androidx.test.runner.AndroidJUnitRunner`.
- If `adb devices` lists any other serial, it is never addressed.
- Standalone tests, no desktop: a tiny in-test server on the device's localhost speaks the protocol from fixtures. Tests:
  - The canvas shows a KEY frame (read back with `PixelCopy` from the SurfaceView).
  - A tile update changes only its rectangle.
  - Injected stylus `MotionEvent`s (`UiAutomation.injectInputEvent`, tool type STYLUS, pressure ramp, with history) arrive as PEN samples in order with the right coordinates and flags.
  - Hover, the eraser toggle and a button-state barrel.
  - A finger gesture arrives as TOUCH.
  - Toolbar taps send ACTION and MODS.
  - `commitText` on the input connection sends TEXT.

### 8.6 End-to-end (script `tools/companion/e2e.sh`, Git Bash; helper `tools/companion/compare_shots.py`, run with `python -E -P`)

1. Check `target\release\wrapped_texture_editor.exe` exists. Start the emulator headless in the background (the exact command of the task); wait for boot; from here on use only `-s emulator-5554`.
2. Install both APKs; `pm clear` the app.
3. **USB path.** Start the desktop exe: `--screenshot target\companion\e2e_usb.png --scenario companion_host --size 1280x800`, with port 47811, auto-allow and 1 expected session. Run `adb -s emulator-5554 reverse tcp:47811 tcp:47811`. Run `am instrument` for `E2eTest#usb` (arguments port and stroke coordinates). The test connects to 127.0.0.1 with link usb, waits for the first frame, injects a stylus stroke with a pressure ramp, and waits for a frame whose `last_input_seq` covers it. Then `adb -s emulator-5554 exec-out screencap -p > target\companion\e2e_usb_tablet.png`. Wait for the desktop exe to exit.
4. Assert:
   - `e2e_usb.json` has `"ok": true` (the scenario asserted the paint).
   - `compare_shots.py`: the tablet screenshot's picture area, scaled to the desktop screenshot, has a mean absolute difference under a threshold fixed after the first good run and recorded in notes. The stroke's pixels are present in both.
5. **Wi-Fi path.** Remove the reverse rule; `pm clear`. Start the desktop exe with test code `K7QF3M9T`, no auto-allow, 2 expected sessions. Run `E2eTest#wifi`: it connects to 10.0.2.2:47811 with link wifi, pairs with the code, draws stroke 1, disconnects, reconnects (token mode, no code) and draws stroke 2. Same asserts; the tablet screenshot is taken after stroke 2.
6. Negative check: a wrong code shows the error text and the desktop stays unpaired (the instrumented test asserts the text; the desktop log has AUTH_FAILED).
7. Stop the emulator (`adb -s emulator-5554 emu kill`). Outputs are under `target\companion\`.

Bluetooth: pure parts only; the real link is a hand test.

---

## 9. Build, release, documentation

**CI** (`.github/workflows/installer.yml`): a new job `android` on `ubuntu-latest`.

- Steps: checkout; `actions/setup-java@v4` (temurin 17); `cd android && ./gradlew --no-daemon testDebugUnitTest assembleDebug`.
- If the secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` exist: decode the keystore to the runner's temp directory and run `assembleRelease` with a signing config read from environment variables. Otherwise use the debug APK.
- Rename to `WTE-Companion-<versionName>.apk` and upload as artifact `companion-apk`.
- `release` gets `needs: [build, smoke, android]`, downloads `companion-apk` and attaches it next to the installer.
- Rust tests of `companion::*` already run in the workspace test step, if the build job runs the workspace tests; otherwise add them to "Unit tests of the packaging-related code".
- No instrumented tests in CI (v1).

**Signing.** A debug-signed APK from CI is signed with that runner's throw-away debug key. Every release then has a different signature, and installing a newer APK over an older one fails until the old one is uninstalled (pairings are lost, re-pairing takes a minute). Fixing this needs a release keystore from the user (decision U1). Agents never create one. The debug keystore that AGP generates lives in the build machine's user profile, not in the repo.

**Installer.** Unchanged. The APK is a separate release file. Bundling the APK and an "Install on the tablet" button (through adb) is nice-to-have N4.

**Documentation** (the desktop builder writes the desktop parts; the Android builder completes them):

- `README.md`: section "Tablet companion (Android)". What it is; download the APK from the release page on the tablet; allow installing from the browser or My Files; open it. Then the three ways to connect (sections 6.1–6.3) as numbered steps for a Galaxy Tab S8 Ultra. Then the security statement and troubleshooting: firewall prompt dismissed; "No adb found"; tablet not listed (Samsung USB driver); Air command popup on the S Pen button; picture slow on Bluetooth; update the app = uninstall first while debug-signed.
- In-app: the dialog texts (section 3) and a paragraph in Help > Tablet setup.
- `dist/HOW_TO_TEST.txt`: a "Tablet companion" block with the hand tests of section 11.
- `installer/RELEASE_NOTES.md`: the entry.
- `ARCHITECTURE.md`, in the same commits as the code:
  - new 1.11 (requirement, decisions C1–C16);
  - section 2 (new files, `android/`, `docs/protocol/`);
  - section 3 (no new crates; `image` used for JPEG in `wte_core`);
  - section 5 (`encode_jpeg`);
  - section 7 (prefs fields, pointer to the protocol spec);
  - new 8.14 "Tablet companion" (threads, capture rule, inject order, arbiter, self-test guard);
  - 8.3 (`raw_input_hook` and `logic` steps), 8.8 (dialog), 8.11 (scenarios, environment variables), 8.13 (pitfalls: capture is the next frame; never bind LAN in self-test);
  - sections 9 and 10.
- `HANDOFF.md`: state, decisions, open hand tests.

---

## 10. Work order

### 10.1 DESKTOP part (builder 1)

Commit and push after each step. Tests and clippy (0 warnings) before each commit. Baseline: 1,344 passed / 1 failed (WA20) / 48 ignored.

| Step | Content | Done when |
|---|---|---|
| D0 | Save this plan as `docs/plans/COMPANION_PLAN.md`, section 4 as `docs/protocol/COMPANION_PROTOCOL.md`. Verify the "verify" items (fact 10, 5.3 context menu, 5.5 focus, wgpu poll from a second thread) and note results. | Notes written. |
| D1 | `crypto.rs`, `proto.rs`, fixtures (Python cross-check), key table. | 8.1 crypto and proto tests green; fixtures committed. |
| D2 | `auth.rs`, `PairStore`, prefs fields. | Auth tests green. |
| D3 | `wte_core::encode_jpeg`; `encode.rs`. | Encode tests green. |
| D4 | `server.rs`, `client.rs`, `discovery.rs`, `bind_plan`, link profiles. | 8.2 loopback tests green; all sockets on 127.0.0.1. |
| D5 | `capture.rs`, hooks in `app.rs`; measure the budget (2.3). | Frames flow to `TestClient` from a headless run; numbers in notes. |
| D6 | `input.rs` and injection; arbiter, gestures, keys, focus, session end. | Input tests green. |
| D7 | Dialog, menu item, status chip, actions, "Fit to tablet". | Shot `companion_dialog` looks right. |
| D8 | `adb.rs`, watcher, dialog lines. Test against the emulator only (`WTE_COMPANION_ADB_SERIAL=emulator-5554`); stop the emulator afterwards. | The reverse rule appears for the emulator (`adb -s emulator-5554 reverse --list`). |
| D9 | `bt_win.rs` (compile and layout tests only). | Builds; never executed. |
| D10 | Scenarios `companion` and `companion_host`; release build; run `smoke`, `default`, `pen`, `companion`. | All `"ok": true`; no window visible, no focus taken. |
| D11 | ARCHITECTURE, HANDOFF, README (desktop parts), Help texts, HOW_TO_TEST. | Docs match the code. |

Desktop check list (for the checker):

- [ ] No new crate in any `Cargo.toml`; `Cargo.lock` unchanged except the workspace's own entries.
- [ ] Tests: baseline plus the new ones; only WA20 red. Clippy 0.
- [ ] Self-test runs: only 127.0.0.1 is bound (grep for `0.0.0.0`, review `bind_plan`); no UDP, Bluetooth or adb without the serial variable.
- [ ] No `SendInput` or `Inject*`; all remote input goes through `raw_input_hook`.
- [ ] Mouse-only frames with the companion off are bit-identical to before: `inject` returns early when there is no session, and the router's mouse-only path is untouched.
- [ ] The `pen` scenario still passes unchanged.
- [ ] A disconnect in mid-stroke leaves no button down (test and scenario).
- [ ] Capture stops when idle (no repaint loop): a connected, idle run shows no frames after the refine.
- [ ] adb is spawned with `CREATE_NO_WINDOW`; the app kills no process.
- [ ] Prefs with the new fields load when the fields are missing (old prefs file).
- [ ] The fixtures equal the encoder output; crypto vectors are cross-checked with Python.
- [ ] Sources are LF; nothing under `target\` is committed.

### 10.2 ANDROID part (builder 2, after the desktop part)

| Step | Content | Done when |
|---|---|---|
| A0 | Read the spec and the fixtures. Project skeleton, wrapper, `.gitignore`, `.gitattributes`. | `assembleDebug` works with the given environment. |
| A1 | `proto/` and `Auth.kt`; JVM tests with the fixtures. | 8.4 green. |
| A2 | `net/` (TCP link, session, reconnect, discovery, Bluetooth link). | Connects to the desktop `companion_host` over 10.0.2.2 by hand script. |
| A3 | `MirrorView` (decode, render, letterbox, trail, states). | Picture visible in `screencap`. |
| A4 | Input (pen, fingers, toolbar, keyboard, IME). | Standalone instrumented tests green (8.5). |
| A5 | Connect screen, stored pairings, permissions, icon, strings. | — |
| A6 | `tools/companion/e2e.sh` and `compare_shots.py`; run the end-to-end both ways. | 8.6 passes; outputs in `target\companion\`; emulator stopped. |
| A7 | CI job, release attachment, README / HOW_TO_TEST (Android parts), notes on dependencies. | Workflow file valid; PR checks green if a PR is opened. |

If the Android builder finds a protocol defect: change the spec file, the Rust side and the fixtures in one commit, and note it in `protocol-changes.md`.

Android check list:

- [ ] Only the dependencies listed in 7.1; no keystore, `local.properties`, `.gradle` or build output committed.
- [ ] JVM tests green; fixtures byte for byte; handshake values equal to `handshake.json`.
- [ ] Every adb command in scripts and tests has `-s emulator-5554`; no `connectedAndroidTest`.
- [ ] The emulator was started only with `-no-window` and was stopped.
- [ ] Every historical sample is sent in order (test with history).
- [ ] End-to-end USB and Wi-Fi: desktop `"ok": true`, picture comparison under the threshold, wrong code refused, token reconnect without a code.
- [ ] The app works without Bluetooth permission (USB and Wi-Fi) and asks only at the Bluetooth button.
- [ ] minSdk 29 builds; lint has no errors.
- [ ] An unknown message type and a longer STATE from a fake server are ignored (forward compatibility test).

### 10.3 Gate (after both)

- Full Rust test suite, clippy, release build.
- Scenarios `smoke`, `default`, `pen`, `paint3d`, `companion`.
- The end-to-end script.
- Review of the hard rules.
- Report `docs/reports/COMPANION_GATE_REPORT.md` with the hand tests still owed.

---

## 11. Risks and hand tests

| Risk | Mitigation |
|---|---|
| Cross-thread `device.poll` misbehaves on some driver. | Fallback in 2.2; decided by measurement in D0/D5. |
| Capture cost at 2560×1440 or 4K (a 33–66 MB buffer per capture; pixel conversion). | Capture only on demand; fps cap; measure. If too slow, limit capture fps on large windows and recommend "Fit to tablet". A custom smaller capture is not possible without patching egui-wgpu. |
| The `image` JPEG encoder is slow on full-frame changes (3D orbit). | Rectangles in parallel; the rate controller lowers quality and scale. Measured in D3 (target: full 1080p frame within 40 ms on 4 threads). |
| Seams between JPEG rectangles at low quality. | 64-px grid alignment; the refine pass. |
| Local trail differs from the real stroke (size, smoothing). | The trail is a thin neutral line by design and disappears with the next frame. |
| Samsung specifics: the S Pen button opens Air command; palm events; hover rate. | Hand tests H5–H7; a Help note to turn Air command off or use the eraser toggle. |
| adb is missing on a normal user's PC; Samsung USB driver. | Clear dialog text, Browse; decision U2 for a one-click download. |
| Firewall prompt dismissed, or network marked Public. | Troubleshooting text; USB as the recommended link. |
| RFCOMM FFI is untested by agents. | Layout tests; small, isolated file; hand test H10. Failure there does not affect USB or Wi-Fi. |
| The unencrypted stream on Wi-Fi can be watched or altered by someone on the same network. | Stated in the UI; USB recommended; protocol capability bit reserved (N1). |
| A covered window under an exclusive-fullscreen game may not present or paint at full rate. | Not the use case (the user draws on the tablet while the editor is up); hand test H12. |
| Emulator stylus injection differs from a real S Pen. | The hand tests are the acceptance for pen feel. |

Hand tests, only possible with the real tablet and PC. Expected results are given; they go into HOW_TO_TEST.

| # | Test | Expected |
|---|---|---|
| H1 | Sideload the APK from the release page on the tablet. | Installs after allowing the source; the app opens in landscape, full screen. |
| H2 | USB: follow the steps in 6.1. | Dialog line goes "No adb found" → "Ready"; the tablet shows the editor within 2 s after Allow; no console window flashes on the PC; a running fullscreen game keeps the focus. |
| H3 | Draw with the S Pen in the 2D and the 3D view; vary pressure. | Stroke width and flow follow the pressure as with a desktop tablet; no gaps; the trail disappears as the real stroke appears; the delay of the real stroke on USB is not disturbing (note the overlay's ping and fps). |
| H4 | Hover over buttons and the model. | Hover highlights and the brush ring follow; tooltips appear when the pen rests. |
| H5 | Hold the S Pen button and touch: drag in 3D and in 2D. | Orbit in 3D, pan in 2D, nothing painted. If Air command pops up: note it; with the setting "S Pen button erases" the button erases instead. |
| H6 | Rest the palm while drawing. | No stray strokes or view jumps. |
| H7 | Fingers: one-finger drag in the view; two-finger drag; pinch; tap buttons, drag a slider, two-finger scroll in the layer list. | Orbit/pan, pan, zoom at the fingers; UI acts as with a mouse; lists scroll. |
| H8 | Toolbar: Undo, Redo, Ctrl/Shift/Alt latch, Eraser toggle, Keyboard (type into the Text tool), Hide. | Each acts on the desktop as its key or shortcut would; text appears in the Text tool. |
| H9 | Wi-Fi: turn on (answer the firewall prompt once), find the PC in the list, type the code. Close and reopen the tablet app. | Pairs; the second connect needs no code; draws as on USB with a slightly later picture. |
| H10 | Bluetooth: pair in Windows once, switch on, choose the PC on the tablet, Allow. | Connects; the picture is coarse and updates a few times per second, sharpening when idle; the pen trail is immediate; strokes are complete, with no lost samples. |
| H11 | Draw with the desktop mouse while the pen hovers on the tablet, and the reverse. | Whoever pressed first draws; the other is ignored until release; nothing gets stuck. |
| H12 | Cover the editor with another window; minimize it; resize it; press Fit on the tablet. | Covered: still live. Minimized: the tablet says so and returns to live on restore. Resize: the picture follows. Fit: the window takes the tablet's shape without coming to the front; Restore brings the old size back. |
| H13 | Pull the cable or leave Wi-Fi range in mid-stroke; come back. | The stroke ends cleanly on the desktop; the tablet shows "Reconnecting..." and continues by itself. |
| H14 | Install a newer desktop version and keep the old tablet app. | Still connects and works (the protocol version is negotiated). |

---

## 12. Decisions for the user (defaults chosen so work can proceed)

| # | Question | Default taken |
|---|---|---|
| U1 | Provide a release keystore (as GitHub secrets) so APK updates install over each other? | No: debug-signed; updating means uninstall first. |
| U2 | May the app download Google's platform-tools on a button click (user accepts Google's licence), instead of the manual download and Browse? | No: manual, with clear steps. |
| U3 | Fingers in the view: navigate only (default) or also paint? | Navigate only; switchable in the dialog. |
| U4 | Encrypt the Wi-Fi stream in a later version (needs local X25519 and cipher code, or permission for crypto crates)? | Not in v1; stated in the UI. |
| U5 | App name "WTE Companion" and package `io.github.maviman.wtecompanion`. | As written. |
| U6 | Ports 47800 (TCP) and 47801 (UDP). | As written; the TCP port is changeable in the dialog. |

## 13. Nice-to-haves (not v1)

- N1: Encrypted and integrity-protected session (caps bit 63).
- N2: PNG refine tiles for pixel-exact idle pictures (codec id 2 is already decodable by the tablet).
- N3: Tilt into the brush (needs `PenRecord` and brush changes).
- N4: APK bundled with the installer plus an "Install on the tablet" button through adb.
- N5: One-click platform-tools download (U2).
- N6: Local zoom and pan of the mirrored picture on the tablet.
- N7: Soft keyboard opens by itself on `TEXT_WANTED`.
- N8: Trail in the brush colour and size.
- N9: Server-defined toolbar buttons.
- N10: Two-finger tap = undo, three-finger tap = redo.
- N11: A mouse or trackpad attached to the tablet.
- N12: Several tablets at once.
- N13: Instrumented tests in CI.
- N14: Mirroring while minimized.
