//! Tablet companion (branch `companion`, `docs/plans/COMPANION_PLAN.md`, wire spec `docs/protocol/COMPANION_PROTOCOL.md`,
//! ARCHITECTURE 1.11 and 8.14): an in-process server, off by default, that mirrors the app window to the Android
//! companion app and feeds its pen, finger and keyboard input into the app in-process.
//!
//! - `crypto.rs`: SHA-256, HMAC, PBKDF2, the pairing code, OS randomness.
//! - `proto.rs`: the wire protocol v1 (messages, framing, key table).
//! - `auth.rs`: the server handshake (versions, auth modes, MACs, the pairing window, paired tablets).
//! - `encode.rs`: RGB, downscale, cell diff, rectangles, JPEG tiles, refine, link profiles, `RateController`.

#![allow(dead_code, reason = "built up step by step (COMPANION_PLAN 10.1); every item is used once D10 is done")]

pub mod crypto;
pub mod proto;
pub mod auth;
pub mod encode;

/// The session port (TCP), COMPANION_PLAN C12.
pub const DEFAULT_PORT: u16 = 47800;
/// The discovery port (UDP, Wi-Fi only).
pub const DISCOVERY_PORT: u16 = 47801;
/// The port of the `companion_host` test scenario (never the real one).
pub const TEST_PORT: u16 = 47811;
