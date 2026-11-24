# Protocol changes (wire protocol v1)

The spec `docs/protocol/COMPANION_PROTOCOL.md` and the fixtures `docs/protocol/fixtures/v1/` exist twice: here and,
byte for byte, in the tablet repo (https://github.com/Maviman/tablet-companion, `protocol/`). Any change is made in the
same step in both repos: spec, fixtures (`WTE_WRITE_FIXTURES=1 cargo test -p wte_app proto` rewrites them here; copy
them over), the Rust side, the Kotlin side; recorded here and in the tablet repo's `protocol/CHANGES.md`. Within v1
only compatible changes are allowed (spec 4.2: new message types, fields appended at the end, new caps bits); anything
else is v2 and the desktop keeps speaking v1 for released tablets.

| Date | Change | Compatible | Both repos |
|---|---|---|---|
| 2026-11-24 | v1 as written in the plan (D0/D1). | — | spec copied at the tablet repo's first commit |

Clarifications that are not changes (2026-11-24, desktop D5-D11):

- The server sets HELLO_ACK `caps` = `SERVER_CURSOR | SERVER_FIT` (it sends CURSOR and honours FIT_REQUEST). The
  `hello_ack.hex` fixture is a sample message and keeps its value.
- The server sends an empty FRAME (0 tiles) only when `last_input_seq` advanced; nothing while the picture and the
  input stay the same.
- STATE `trail_rgba` is read by the tablet as an Android colour int `0xAARRGGBB` (the desktop sends `0xFF2040D0`, an
  opaque blue). Recorded the same way in the tablet repo's `protocol/CHANGES.md` (2026-11-24, Android builder).
