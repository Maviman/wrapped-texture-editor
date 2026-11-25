# Tablet Companion wire protocol (COMPANION_PROTOCOL.md)

Source of truth for the desktop app (Wrapped Texture Editor, `crates/wte_app/src/companion/`) and the Android app
"Tablet Companion" (https://github.com/Maviman/tablet-companion, the Gradle project at that repo's root). This file and
the golden fixtures `fixtures/v1/` exist byte for byte in both repos (editor `docs/protocol/`, tablet repo `protocol/`).
Copied from the editor's `docs/plans/COMPANION_PLAN.md` section 4; later changes are made in both copies in the same
step and listed in the editor's `docs/notes/companion/protocol-changes.md` and the tablet repo's `protocol/CHANGES.md`.

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

