# Fix pass (2026-11-24): desktop side

Two checkers reviewed branch `companion` @ ab2535f (desktop / security, and Android end-to-end). This file lists the
desktop findings and what was done; the tablet half is in the tablet repo's `docs/notes/fix.md`
(https://github.com/Maviman/tablet-companion). ARCHITECTURE 8.14 describes the new behaviour.

## Findings

| Finding (severity) | Verdict | Fix | Test |
|---|---|---|---|
| A tablet that keeps the connection but stops reading disables the keep-alive: the writer blocks in `write_all` holding the link mutex, the reader blocks on it to send PONG, the idle check never runs; the session never ends and other tablets get BUSY (major) | confirmed (code: no write timeout, PONG through the shared mutex) | `Link::set_write_timeout` (TCP; RFCOMM `SO_SNDTIMEO`) = idle time (RFCOMM doubled) on the link and the writer's clone. The reader never writes: PING -> `Feedback::pong`, the writer sends PONG; the reader's last ERROR uses `try_lock`. `run_writer` returns its send error, then shuts the link down so the reader ends; the session's end reason says "stopped reading" | `server::tests::a_tablet_that_stops_reading_is_dropped` (noise frames, acks and pings sent, nothing read: ends, another tablet connects); `a_ping_flood_without_reading_still_times_out` (20,000 PINGs unread, then quiet: ends within the idle time) |
| Minimizing during a remote stroke drops the releases: local mouse locked out after restore, the next tablet stroke continues the old one, a released Ctrl latch still applies (major) | confirmed (the checker's scratch tests reproduce it) | While minimized, remote input is dropped but its seq counts as processed; the first `inject` after the minimize runs `InputStage::end_session` once (pen Leave, gesture release, keys and mods cleared, holder None) | `companion::tests::a_minimize_in_mid_stroke_releases_the_remote`, `a_minimize_clears_a_latched_modifier` (the checker's scenarios, asserting the fixed behaviour) |
| A capture waiting in the mailbox reaches the next session with the old session's `last_input_seq` (minor) | confirmed | `Captured::session` (the session the UI knew when it asked); the writer drops captures of another session; the mailbox is emptied at session start and end; `Capture::reset` zeroes the stored seq | `server::tests::a_capture_of_another_session_is_dropped` |
| PBKDF2 under `hub.auth`, which the UI thread locks every frame (56 ms freeze per wrong code) (minor) | confirmed | The check runs on copies without the lock. A code attempt reserves its place first (`PairingWindow::begin_attempt` / `end_attempt`): failures + attempts running never exceed 5 | `server::tests::a_code_attempt_does_not_hold_the_auth_lock` (300,000 iterations, the lock is never held over 30 ms); `auth::tests::concurrent_code_attempts_cannot_exceed_the_limit` |
| One thread per connection, unbounded; `.expect` on spawn (minor) | confirmed | `MAX_HANDSHAKES` = 8 connections in their handshake (`Hub::admit`, `Admission` dropped after AUTH_OK); more are closed in the accept loop without a thread (TCP and RFCOMM share `spawn_session`); a failed spawn drops the connection. Magic + HELLO within `hello_timeout` (4 s; whole handshake still 10 s) | `server::tests::idle_connections_are_limited_and_time_out` |
| `Server::stop` joins the adb watcher on the UI thread; each adb call may take 5 s (minor) | confirmed by reading (adb not run) | `adb::run_until(.., stop)`: a running call gives up within one 20 ms poll when the server's stop flag is set (adb left running, never killed); the watcher also checks the flag between reverse calls. `run` (unused) removed | `adb::tests::a_running_call_gives_up_when_the_server_stops` (`ping -n 4 127.0.0.1` as the slow command: gives up in < 1.5 s) |
| The window is captured on every repaint even when nothing changed; a hovering pen or a latched modifier forces a repaint every 100 ms (minor) | confirmed | The writer counts unchanged captures in a row (`Hub::empty_captures`); after about a second of them and no input for 1 s the capture interval is 500 ms (`capture::capture_interval`, pure). The 100 ms repaint runs only while the remote holds something down (`InputStage::needs_ticks`), not for hover or a latch | `capture::tests::the_capture_rate_drops_while_nothing_changes`; the `empty_captures` counter in `a_capture_of_another_session_is_dropped`. Measured: `companion_host` with the emulator connected, 91 captures in 31 s (2 frames), against 412 in 14 s before |
| Wrong test count in `d5-d11.md` (minor) | confirmed (1,401, not 1,441) | corrected | - |

Protocol: no wire change. The spec header now names the tablet repo as the Android app's home (doc only, both copies
in the same step; `protocol-changes.md`).

## Verified (this pass)

- `cargo test --workspace --no-fail-fast`: 1,411 passed / 1 failed (WA20, known) / 48 ignored (+10 new tests over
  1,401). `cargo clippy --workspace --all-targets`: 0 warnings. `cargo build --release -p wte_app`: OK, exe SHA-256
  starts `30d1def19a0a7702`. `Cargo.lock` and every `Cargo.toml` unchanged (no new crate).
- Tablet repo: JVM tests 35/35, lint 0 errors, instrumented 18 OK on the headless emulator.
- End to end (`tablet-companion/tools/e2e.sh --exe <copy of this exe>`): USB and Wi-Fi passed, MAD 2.48 / 2.50.
- By hand on the emulator against `companion_host` on 127.0.0.1:47800 (adb reverse): Home -> "session 1 ended: the
  tablet said bye"; launcher -> one new session (token mode), no reconnect loop; the overlay shows the ping.
- Every desktop run used my own copy of the exe (`target/companion/fix_e2e/bin/`), off-screen, 127.0.0.1 only; that
  copy and the emulator were stopped at the end.
