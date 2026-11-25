# Android builder (A0-A7): what changed on the desktop side

The Android app lives in https://github.com/Maviman/tablet-companion (local clone `G:\software_projects\tablet-companion`);
its notes are there (`docs/notes/android.md`, `docs/notes/STATE.md`). This file lists only what the Android builder
changed in this repo.

## `WTE_COMPANION_LINGER_MS` (companion_host)

Problem found in the first end-to-end run: `companion_host` took its screenshot and exited within about 100 ms of the
tablet's stroke. The tablet test could not count on a FRAME confirming its last PEN message, and its `screencap` came
after the connection was gone ("Reconnecting..." over a possibly older picture), so the picture comparison was a race.

Fix: after the asserts the scenario keeps serving for `WTE_COMPANION_LINGER_MS` milliseconds (default 0 = old
behaviour, capped at 60 s), then takes its screenshot and exits. `tools/e2e.sh` in the tablet repo sets 8000. Pure
parser `linger_ms` with a unit test (`linger_tests`). Docs: ARCHITECTURE 8.11 table, HANDOFF variable table,
`desktop.md`.

## Results with the real exe (headless emulator, 2026-11-24)

USB (adb reverse, confirm mode) and Wi-Fi (10.0.2.2, wrong code refused, code pairing, token reconnect) both pass;
the desktop's own screenshots and the emulator's screencaps agree (mean absolute difference about 2.5 of 255; the
stroke rows match). Outputs: `target/companion/e2e/`. Numbers: tablet repo `docs/notes/android.md`.

No protocol change. One clarification (trail colour byte order) in `protocol-changes.md`.
