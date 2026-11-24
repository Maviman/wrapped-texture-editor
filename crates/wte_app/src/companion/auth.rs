//! The server side of the companion handshake (protocol 4.3), pure: version negotiation, the offered auth modes, the
//! key and MAC derivation, the check of an AUTH message, the pairing window (code, lifetime, attempt limit) and the
//! paired-tablet records kept in the preferences (`PairedTablet`). The session threads (`server.rs`) hold the state
//! behind a mutex; nothing here touches a socket or the clock (`now` is passed in).

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::crypto::{self, HmacKey, ct_eq, hex, hmac_sha256, pbkdf2_hmac_sha256};
use super::proto::{self, Auth, AuthOk, ErrorMsg, Hello, HelloAck, auth_mode, err};

/// PBKDF2 iterations the server asks for (4.3).
pub const PBKDF2_ITERS: u32 = 100_000;
/// The range a client accepts (4.3).
#[allow(dead_code, reason = "the client-side lower limit of the spec (4.3), enforced by the tablet")]
pub const MIN_ITERS: u32 = 50_000;
pub const MAX_ITERS: u32 = 2_000_000;
/// The oldest protocol major version this desktop still speaks (4.2 rule 3: every released tablet version).
pub const PROTO_MIN_SUPPORTED: u16 = 1;
/// A code is replaced after this long, even while the dialog stays open (4.3).
pub const CODE_LIFETIME: Duration = Duration::from_secs(600);
/// Failed code attempts before the code is replaced (4.3).
pub const MAX_CODE_FAILURES: u32 = 5;
/// How long the server answers TOO_MANY_ATTEMPTS after the fifth failure (4.3).
pub const LOCKOUT: Duration = Duration::from_secs(30);
/// How long a confirm request waits for Allow / Deny (4.3 AUTH_WAIT).
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

/// A tablet that paired with this desktop (prefs `companion.paired`; the token is stored as hex in plain text in the
/// per-user preferences file, ARCHITECTURE 7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedTablet {
    /// `tablet_id` as 32 hex digits.
    pub id: String,
    pub name: String,
    /// The 32-byte token as 64 hex digits.
    pub token: String,
    /// Date of pairing, `YYYY-MM-DD` (UTC).
    pub added: String,
}

impl PairedTablet {
    pub fn new(tablet_id: &[u8; 16], name: &str, token: &[u8; 32], added: String) -> PairedTablet {
        PairedTablet { id: hex(tablet_id), name: clean_name(name), token: hex(token), added }
    }

    pub fn tablet_id(&self) -> Option<[u8; 16]> {
        crypto::unhex_array(&self.id)
    }

    pub fn token_bytes(&self) -> Option<[u8; 32]> {
        crypto::unhex_array(&self.token)
    }
}

/// A device name as shown in the dialog: control characters removed, at most 64 characters, "Tablet" when empty.
pub fn clean_name(name: &str) -> String {
    let s: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    let s = s.trim();
    if s.is_empty() { "Tablet".to_owned() } else { s.to_owned() }
}

/// Today's date in UTC as `YYYY-MM-DD` (civil-from-days, Howard Hinnant's algorithm).
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    date_from_days((secs / 86_400) as i64)
}

/// `YYYY-MM-DD` of a day number since 1970-01-01.
pub fn date_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The highest major version both sides speak, or `None` (ERROR VERSION).
pub fn negotiate(proto_min: u16, proto_max: u16) -> Option<u16> {
    let hi = proto_max.min(proto::PROTO_VERSION);
    let lo = proto_min.max(PROTO_MIN_SUPPORTED);
    (lo <= hi).then_some(hi)
}

/// `T = desktop_id ‖ tablet_id ‖ server_nonce ‖ client_nonce ‖ u16le(proto) ‖ u8(link)` (4.3).
pub fn transcript(desktop_id: &[u8; 16], tablet_id: &[u8; 16], server_nonce: &[u8; 16], client_nonce: &[u8; 16], proto: u16, link: u8) -> Vec<u8> {
    let mut t = Vec::with_capacity(67);
    t.extend_from_slice(desktop_id);
    t.extend_from_slice(tablet_id);
    t.extend_from_slice(server_nonce);
    t.extend_from_slice(client_nonce);
    t.extend_from_slice(&proto.to_le_bytes());
    t.push(link);
    t
}

/// The PBKDF2 salt of code mode: `"WTEC1-pair" ‖ desktop_id ‖ tablet_id ‖ server_nonce ‖ client_nonce`.
pub fn pair_salt(desktop_id: &[u8; 16], tablet_id: &[u8; 16], server_nonce: &[u8; 16], client_nonce: &[u8; 16]) -> Vec<u8> {
    let mut s = b"WTEC1-pair".to_vec();
    for part in [desktop_id, tablet_id, server_nonce, client_nonce] {
        s.extend_from_slice(part);
    }
    s
}

/// `pair_key = PBKDF2-HMAC-SHA256(code, salt, iters, 32)`; `code` is the normalized code.
pub fn pair_key(code: &str, salt: &[u8], iters: u32) -> [u8; 32] {
    let mut k = [0u8; 32];
    pbkdf2_hmac_sha256(code.as_bytes(), salt, iters, &mut k);
    k
}

/// The token both sides derive in code mode: `HMAC(pair_key, "WTEC1-token")`.
pub fn token_from_pair_key(pair_key: &[u8; 32]) -> [u8; 32] {
    hmac_sha256(pair_key, &[b"WTEC1-token"])
}

/// `client_mac = HMAC(K, "WTEC1-client" ‖ T)`.
pub fn client_mac(k: &[u8; 32], t: &[u8]) -> [u8; 32] {
    HmacKey::new(k).mac(&[b"WTEC1-client", t])
}

/// `server_mac = HMAC(K, "WTEC1-server" ‖ T)`.
pub fn server_mac(k: &[u8; 32], t: &[u8]) -> [u8; 32] {
    HmacKey::new(k).mac(&[b"WTEC1-server", t])
}

/// The pairing code on display and its limits (4.3). The dialog opens and closes it; the session threads ask it.
#[derive(Clone, Debug, Default)]
pub struct PairingWindow {
    /// The Tablet companion dialog is open (unknown tablets may pair).
    open: bool,
    code: Option<String>,
    issued: Option<Instant>,
    failures: u32,
    locked_until: Option<Instant>,
    /// A fixed code (`WTE_COMPANION_TEST_CODE`, self-test runs only): never replaced, the window counts as open.
    fixed: Option<String>,
}

impl PairingWindow {
    /// A window with a fixed code that is always open (the `companion_host` scenario).
    pub fn fixed(code: String) -> PairingWindow {
        PairingWindow { open: true, code: Some(code.clone()), issued: None, failures: 0, locked_until: None, fixed: Some(code) }
    }

    /// The dialog opened: a fresh code (unless one is still valid).
    pub fn open(&mut self, now: Instant) {
        self.open = true;
        if self.fixed.is_none() && (self.code.is_none() || self.expired(now)) {
            self.issue(now);
        }
    }

    /// The dialog closed: no code, no confirm requests.
    pub fn close(&mut self) {
        if self.fixed.is_some() {
            return;
        }
        self.open = false;
        self.code = None;
        self.issued = None;
        self.failures = 0;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn expired(&self, now: Instant) -> bool {
        self.issued.is_some_and(|t| now.saturating_duration_since(t) >= CODE_LIFETIME)
    }

    fn issue(&mut self, now: Instant) {
        self.code = Some(crypto::new_code());
        self.issued = Some(now);
        self.failures = 0;
    }

    /// The valid code at `now` (replaced when its lifetime is over); `None` while closed.
    pub fn code(&mut self, now: Instant) -> Option<String> {
        if !self.open {
            return None;
        }
        if self.fixed.is_none() && (self.code.is_none() || self.expired(now)) {
            self.issue(now);
        }
        self.code.clone()
    }

    /// The code without refreshing it (the dialog's display).
    #[cfg_attr(not(test), allow(dead_code, reason = "used by the tests (and the tablet side of the protocol)"))]
    pub fn shown_code(&self) -> Option<&str> {
        self.code.as_deref().filter(|_| self.open)
    }

    /// True while a fifth failure's lockout lasts.
    pub fn locked(&self, now: Instant) -> bool {
        self.locked_until.is_some_and(|t| now < t)
    }

    /// A failed code attempt: after the fifth, a new code and the lockout.
    pub fn fail(&mut self, now: Instant) {
        self.failures += 1;
        if self.failures >= MAX_CODE_FAILURES {
            self.locked_until = Some(now + LOCKOUT);
            if self.fixed.is_none() {
                self.issue(now);
            }
            self.failures = 0;
        }
    }
}

/// What the server knows when it answers a HELLO or checks an AUTH.
pub struct Policy<'a> {
    pub desktop_id: [u8; 16],
    pub paired: &'a [PairedTablet],
    /// The valid pairing code, if one is on display.
    pub code: Option<String>,
    /// The dialog is open (confirm mode possible).
    pub pairing_open: bool,
    /// TOO_MANY_ATTEMPTS lockout.
    pub locked: bool,
    pub iters: u32,
    /// Auto-allow (self-test scenarios only): confirm requests are allowed at once.
    pub auto_allow: bool,
}

impl Policy<'_> {
    /// The stored token of a paired tablet.
    pub fn token_of(&self, tablet_id: &[u8; 16]) -> Option<[u8; 32]> {
        let id = hex(tablet_id);
        self.paired.iter().find(|p| p.id == id).and_then(|p| p.token_bytes())
    }
}

/// Where the connection came from (for confirm mode).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// The peer address is 127.0.0.1 / ::1 (USB through `adb reverse`).
    pub loopback: bool,
    /// The connection came in on the Bluetooth RFCOMM listener.
    pub rfcomm: bool,
}

/// The auth modes the server offers this HELLO (4.3).
pub fn offered_modes(hello: &Hello, peer: Peer, policy: &Policy<'_>) -> u8 {
    let mut modes = 0;
    if policy.token_of(&hello.tablet_id).is_some() {
        modes |= auth_mode::bit(auth_mode::TOKEN);
    }
    if policy.code.is_some() && !policy.locked {
        modes |= auth_mode::bit(auth_mode::CODE);
    }
    let confirm_link = (hello.link == proto::link::USB && peer.loopback) || peer.rfcomm;
    if (policy.pairing_open || policy.auto_allow) && confirm_link {
        modes |= auth_mode::bit(auth_mode::CONFIRM);
    }
    modes
}

fn error(code: u16, text: &str) -> ErrorMsg {
    ErrorMsg { code, text: text.to_owned() }
}

/// One connection's handshake after a valid HELLO.
#[derive(Clone, Debug)]
pub struct Handshake {
    pub hello: Hello,
    pub proto: u16,
    pub server_nonce: [u8; 16],
    pub modes: u8,
    pub desktop_id: [u8; 16],
}

/// The result of an AUTH.
#[derive(Clone, Debug, PartialEq)]
pub enum AuthOutcome {
    /// Authenticated: the AUTH_OK to send and the token the tablet now has (store it when `new_pairing`).
    Ok { ack: AuthOk, token: [u8; 32], new_pairing: bool },
    /// Confirm mode: ask the desktop user (AUTH_WAIT meanwhile), then `Handshake::confirmed`.
    NeedConfirm,
    /// Refused: send the error and close; `code_failure` counts against the pairing code.
    Fail { error: ErrorMsg, code_failure: bool },
}

impl Handshake {
    /// Answers a HELLO: the HELLO_ACK, or the ERROR to send before closing.
    pub fn start(
        hello: Hello,
        peer: Peer,
        policy: &Policy<'_>,
        server_nonce: [u8; 16],
        app_version: &str,
        desktop_name: &str,
    ) -> Result<(Handshake, HelloAck), ErrorMsg> {
        let Some(proto) = negotiate(hello.proto_min, hello.proto_max) else {
            return Err(error(
                err::VERSION,
                &format!(
                    "the desktop speaks protocol {}..{}, the tablet {}..{}",
                    PROTO_MIN_SUPPORTED,
                    proto::PROTO_VERSION,
                    hello.proto_min,
                    hello.proto_max
                ),
            ));
        };
        let modes = offered_modes(&hello, peer, policy);
        if modes == 0 {
            return Err(if policy.locked && policy.code.is_some() {
                error(err::TOO_MANY_ATTEMPTS, "too many wrong codes; wait 30 seconds")
            } else {
                error(err::PAIRING_CLOSED, "unknown tablet: open Help > Tablet companion on the desktop")
            });
        }
        let ack = HelloAck {
            proto,
            caps: proto::caps::SERVER_CURSOR | proto::caps::SERVER_FIT,
            desktop_id: policy.desktop_id,
            server_nonce,
            auth_modes: modes,
            pbkdf2_iters: policy.iters,
            app_version: app_version.to_owned(),
            desktop_name: desktop_name.to_owned(),
        };
        Ok((Handshake { hello, proto, server_nonce, modes, desktop_id: policy.desktop_id }, ack))
    }

    fn transcript(&self) -> Vec<u8> {
        transcript(&self.desktop_id, &self.hello.tablet_id, &self.server_nonce, &self.hello.client_nonce, self.proto, self.hello.link)
    }

    /// Checks an AUTH (constant-time MAC compare).
    pub fn check(&self, auth: &Auth, policy: &Policy<'_>) -> AuthOutcome {
        if auth.mode == 0 || auth.mode > 8 || self.modes & auth_mode::bit(auth.mode) == 0 {
            return AuthOutcome::Fail { error: error(err::AUTH_FAILED, "this way of pairing is not offered"), code_failure: false };
        }
        let t = self.transcript();
        match auth.mode {
            auth_mode::TOKEN => match policy.token_of(&self.hello.tablet_id) {
                Some(token) if ct_eq(&client_mac(&token, &t), &auth.client_mac) => AuthOutcome::Ok {
                    ack: AuthOk { mode: auth_mode::TOKEN, server_mac: server_mac(&token, &t), token: [0; 32] },
                    token,
                    new_pairing: false,
                },
                _ => AuthOutcome::Fail { error: error(err::AUTH_FAILED, "the pairing key does not match: pair again"), code_failure: false },
            },
            auth_mode::CODE => {
                if policy.locked {
                    return AuthOutcome::Fail { error: error(err::TOO_MANY_ATTEMPTS, "too many wrong codes; wait 30 seconds"), code_failure: false };
                }
                let Some(code) = policy.code.as_deref() else {
                    return AuthOutcome::Fail { error: error(err::PAIRING_CLOSED, "no pairing code is shown on the desktop"), code_failure: false };
                };
                let salt = pair_salt(&self.desktop_id, &self.hello.tablet_id, &self.server_nonce, &self.hello.client_nonce);
                let k = pair_key(code, &salt, policy.iters);
                if ct_eq(&client_mac(&k, &t), &auth.client_mac) {
                    AuthOutcome::Ok {
                        ack: AuthOk { mode: auth_mode::CODE, server_mac: server_mac(&k, &t), token: [0; 32] },
                        token: token_from_pair_key(&k),
                        new_pairing: true,
                    }
                } else {
                    AuthOutcome::Fail { error: error(err::AUTH_FAILED, "wrong code"), code_failure: true }
                }
            }
            auth_mode::CONFIRM => AuthOutcome::NeedConfirm,
            _ => AuthOutcome::Fail { error: error(err::AUTH_FAILED, "unknown pairing mode"), code_failure: false },
        }
    }

    /// The AUTH_OK of an allowed confirm request, with the new random `token`.
    pub fn confirmed(&self, token: [u8; 32]) -> AuthOk {
        AuthOk { mode: auth_mode::CONFIRM, server_mac: [0; 32], token }
    }
}

/// The client side of the MACs (the test client; the Android app does the same with platform crypto).
pub fn client_auth(ack: &HelloAck, hello: &Hello, mode: u8, key: &ClientKey) -> (Auth, Option<[u8; 32]>) {
    let t = transcript(&ack.desktop_id, &hello.tablet_id, &ack.server_nonce, &hello.client_nonce, ack.proto, hello.link);
    match (mode, key) {
        (auth_mode::TOKEN, ClientKey::Token(token)) => (Auth { mode, client_mac: client_mac(token, &t) }, Some(server_mac(token, &t))),
        (auth_mode::CODE, ClientKey::Code(code)) => {
            let salt = pair_salt(&ack.desktop_id, &hello.tablet_id, &ack.server_nonce, &hello.client_nonce);
            let k = pair_key(code, &salt, ack.pbkdf2_iters);
            (Auth { mode, client_mac: client_mac(&k, &t) }, Some(server_mac(&k, &t)))
        }
        _ => (Auth { mode: auth_mode::CONFIRM, client_mac: [0; 32] }, None),
    }
}

/// What a client authenticates with.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code, reason = "used by the tests (and the tablet side of the protocol)"))]
pub enum ClientKey {
    Token([u8; 32]),
    /// The normalized code.
    Code(String),
    Confirm,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::companion::crypto::unhex_array;
    use crate::companion::proto::link;

    pub(crate) const DESKTOP: [u8; 16] = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f];

    pub(crate) fn hello(tablet: u8, link: u8) -> Hello {
        Hello {
            proto_min: 1,
            proto_max: 1,
            caps: 0,
            link,
            tablet_id: [tablet; 16],
            client_nonce: [0xc1; 16],
            screen_w: 2960,
            screen_h: 1848,
            density_dpi: 340,
            app_version: "1.0.0".into(),
            device_name: "Tab".into(),
        }
    }

    fn policy<'a>(paired: &'a [PairedTablet], code: Option<&str>, open: bool) -> Policy<'a> {
        Policy { desktop_id: DESKTOP, paired, code: code.map(str::to_owned), pairing_open: open, locked: false, iters: 1000, auto_allow: false }
    }

    const LOOP: Peer = Peer { loopback: true, rfcomm: false };
    const LAN: Peer = Peer { loopback: false, rfcomm: false };

    fn run(h: &Hello, peer: Peer, p: &Policy<'_>, key: ClientKey, mode: u8) -> (Result<(Handshake, HelloAck), ErrorMsg>, Option<AuthOutcome>) {
        match Handshake::start(h.clone(), peer, p, [0x5e; 16], "0.2.0", "PC") {
            Err(e) => (Err(e), None),
            Ok((hs, ack)) => {
                let (auth, expect_server) = client_auth(&ack, h, mode, &key);
                let out = hs.check(&auth, p);
                if let (AuthOutcome::Ok { ack: ok, .. }, Some(s)) = (&out, expect_server) {
                    assert!(ct_eq(&ok.server_mac, &s), "the client verifies the server MAC");
                }
                (Ok((hs, ack)), Some(out))
            }
        }
    }

    #[test]
    fn version_negotiation() {
        assert_eq!(negotiate(1, 1), Some(1));
        assert_eq!(negotiate(1, 5), Some(1), "a newer tablet that also speaks 1");
        assert_eq!(negotiate(0, 1), Some(1));
        assert_eq!(negotiate(2, 3), None, "only newer versions");
        assert_eq!(negotiate(0, 0), None, "only older than supported");
        assert_eq!(negotiate(3, 2), None, "empty range");
        let h = Hello { proto_min: 2, proto_max: 3, ..hello(1, link::USB) };
        let e = Handshake::start(h, LOOP, &policy(&[], None, true), [0; 16], "", "").unwrap_err();
        assert_eq!(e.code, err::VERSION);
    }

    #[test]
    fn token_mode_succeeds_and_fails() {
        let token = [0x77; 32];
        let paired = [PairedTablet::new(&[1; 16], "Tab", &token, "2026-11-24".into())];
        let p = policy(&paired, None, false);
        let h = hello(1, link::WIFI);
        let (ack, out) = run(&h, LAN, &p, ClientKey::Token(token), auth_mode::TOKEN);
        assert_eq!(ack.unwrap().1.auth_modes, auth_mode::bit(auth_mode::TOKEN), "closed dialog, known tablet: token only");
        match out.unwrap() {
            AuthOutcome::Ok { token: t, new_pairing, ack } => {
                assert_eq!(t, token);
                assert!(!new_pairing);
                assert_eq!(ack.token, [0; 32], "the token is never sent in token mode");
            }
            other => panic!("{other:?}"),
        }
        let (_, out) = run(&h, LAN, &p, ClientKey::Token([0x78; 32]), auth_mode::TOKEN);
        assert!(matches!(out.unwrap(), AuthOutcome::Fail { error: ErrorMsg { code: err::AUTH_FAILED, .. }, code_failure: false }));
        // A token record for another tablet does not help.
        let (r, _) = run(&hello(2, link::WIFI), LAN, &p, ClientKey::Token(token), auth_mode::TOKEN);
        assert_eq!(r.unwrap_err().code, err::PAIRING_CLOSED);
    }

    #[test]
    fn code_mode_derives_the_same_token_on_both_sides() {
        let p = policy(&[], Some("K7QF3M9T"), true);
        let h = hello(3, link::WIFI);
        let (ack, out) = run(&h, LAN, &p, ClientKey::Code("K7QF3M9T".into()), auth_mode::CODE);
        let ack = ack.unwrap().1;
        assert_eq!(ack.auth_modes, auth_mode::bit(auth_mode::CODE), "Wi-Fi: no confirm");
        let AuthOutcome::Ok { token, new_pairing, .. } = out.unwrap() else { panic!() };
        assert!(new_pairing);
        let salt = pair_salt(&DESKTOP, &h.tablet_id, &ack.server_nonce, &h.client_nonce);
        assert_eq!(token, token_from_pair_key(&pair_key("K7QF3M9T", &salt, 1000)), "the client derives the same token");
        let (_, out) = run(&h, LAN, &p, ClientKey::Code("K7QF3M9X".into()), auth_mode::CODE);
        assert!(matches!(out.unwrap(), AuthOutcome::Fail { code_failure: true, .. }));
    }

    #[test]
    fn wrong_codes_count_and_the_fifth_replaces_the_code() {
        let t0 = Instant::now();
        let mut w = PairingWindow::default();
        assert_eq!(w.code(t0), None, "closed");
        w.open(t0);
        let first = w.code(t0).unwrap();
        assert_eq!(w.code(t0 + Duration::from_secs(5)).unwrap(), first, "stable while valid");
        for _ in 0..4 {
            w.fail(t0);
            assert!(!w.locked(t0));
        }
        assert_eq!(w.code(t0).unwrap(), first);
        w.fail(t0);
        assert!(w.locked(t0) && w.locked(t0 + Duration::from_secs(29)) && !w.locked(t0 + LOCKOUT));
        assert_ne!(w.code(t0).unwrap(), first, "replaced after the fifth failure");
        // Locked: no code mode is offered and a code AUTH gets TOO_MANY_ATTEMPTS.
        let code = w.code(t0).unwrap();
        let p = Policy { locked: true, ..policy(&[], Some(&code), true) };
        let e = Handshake::start(hello(4, link::WIFI), LAN, &p, [0; 16], "", "").unwrap_err();
        assert_eq!(e.code, err::TOO_MANY_ATTEMPTS);
        // Lifetime: replaced after 10 minutes, even while open; closing forgets it.
        let c = w.code(t0 + LOCKOUT).unwrap();
        assert_ne!(w.code(t0 + CODE_LIFETIME + Duration::from_secs(1)).unwrap(), c);
        w.close();
        assert_eq!(w.code(t0), None);
        assert_eq!(w.shown_code(), None);
        // A fixed test code is never replaced and keeps the window open.
        let mut f = PairingWindow::fixed("K7QF3M9T".into());
        for _ in 0..5 {
            f.fail(t0);
        }
        f.close();
        assert_eq!(f.code(t0 + CODE_LIFETIME * 2).as_deref(), Some("K7QF3M9T"));
    }

    #[test]
    fn confirm_only_with_the_dialog_open_and_usb_loopback_or_rfcomm() {
        let open = policy(&[], None, true);
        let closed = policy(&[], None, false);
        let confirm = auth_mode::bit(auth_mode::CONFIRM);
        assert_eq!(offered_modes(&hello(5, link::USB), LOOP, &open), confirm);
        assert_eq!(offered_modes(&hello(5, link::USB), LAN, &open), 0, "usb claimed over the LAN");
        assert_eq!(offered_modes(&hello(5, link::WIFI), LOOP, &open), 0, "Wi-Fi link: code only");
        assert_eq!(offered_modes(&hello(5, link::BLUETOOTH), Peer { loopback: false, rfcomm: true }, &open), confirm);
        assert_eq!(offered_modes(&hello(5, link::USB), LOOP, &closed), 0);
        let e = Handshake::start(hello(5, link::USB), LOOP, &closed, [0; 16], "", "").unwrap_err();
        assert_eq!(e.code, err::PAIRING_CLOSED);
        let auto = Policy { auto_allow: true, ..policy(&[], None, false) };
        assert_eq!(offered_modes(&hello(5, link::USB), LOOP, &auto), confirm, "auto-allow (self-test) opens confirm");
        let (hs, _) = Handshake::start(hello(5, link::USB), LOOP, &open, [0; 16], "", "").unwrap();
        assert_eq!(hs.check(&Auth { mode: auth_mode::CONFIRM, client_mac: [0; 32] }, &open), AuthOutcome::NeedConfirm);
        let ok = hs.confirmed([9; 32]);
        assert_eq!((ok.mode, ok.token, ok.server_mac), (auth_mode::CONFIRM, [9; 32], [0; 32]));
        // A mode that was not offered fails.
        let out = hs.check(&Auth { mode: auth_mode::CODE, client_mac: [0; 32] }, &open);
        assert!(matches!(out, AuthOutcome::Fail { code_failure: false, .. }));
        let out = hs.check(&Auth { mode: 0, client_mac: [0; 32] }, &open);
        assert!(matches!(out, AuthOutcome::Fail { .. }));
    }

    #[test]
    fn names_and_dates() {
        assert_eq!(clean_name("  Galaxy\u{7}Tab  "), "GalaxyTab");
        assert_eq!(clean_name(""), "Tablet");
        assert_eq!(clean_name(&"x".repeat(100)).len(), 64);
        assert_eq!(date_from_days(0), "1970-01-01");
        assert_eq!(date_from_days(20_781), "2026-11-24");
        assert_eq!(date_from_days(11_016), "2000-02-29");
        assert_eq!(today().len(), 10);
        let p = PairedTablet::new(&[0xab; 16], "Tab", &[0xcd; 32], "2026-11-24".into());
        assert_eq!(p.tablet_id(), Some([0xab; 16]));
        assert_eq!(p.token_bytes(), Some([0xcd; 32]));
    }

    /// `docs/protocol/fixtures/v1/handshake.json`: written with `WTE_WRITE_FIXTURES=1`, else checked; the values are
    /// cross-checked with Python by `tools/companion/check_handshake.py`.
    #[test]
    fn handshake_fixture() {
        let path = crate::companion::proto::tests::fixture_dir().join("handshake.json");
        let desktop_id: [u8; 16] = unhex_array("101112131415161718191a1b1c1d1e1f").unwrap();
        let tablet_id: [u8; 16] = unhex_array("a0a1a2a3a4a5a6a7a8a9aaabacadaeaf").unwrap();
        let server_nonce: [u8; 16] = unhex_array("505152535455565758595a5b5c5d5e5f").unwrap();
        let client_nonce: [u8; 16] = unhex_array("c0c1c2c3c4c5c6c7c8c9cacbcccdcecf").unwrap();
        let (proto, link, code, iters) = (1u16, link::WIFI, "K7QF3M9T", PBKDF2_ITERS);
        let t = transcript(&desktop_id, &tablet_id, &server_nonce, &client_nonce, proto, link);
        let salt = pair_salt(&desktop_id, &tablet_id, &server_nonce, &client_nonce);
        let k = pair_key(code, &salt, iters);
        let token = token_from_pair_key(&k);
        let value = serde_json::json!({
            "desktop_id": hex(&desktop_id),
            "tablet_id": hex(&tablet_id),
            "server_nonce": hex(&server_nonce),
            "client_nonce": hex(&client_nonce),
            "proto": proto,
            "link": link,
            "code": code,
            "code_typed": "k7qf-3m9t",
            "pbkdf2_iters": iters,
            "transcript": hex(&t),
            "salt": hex(&salt),
            "pair_key": hex(&k),
            "token": hex(&token),
            "code_mode": { "client_mac": hex(&client_mac(&k, &t)), "server_mac": hex(&server_mac(&k, &t)) },
            "token_mode": { "client_mac": hex(&client_mac(&token, &t)), "server_mac": hex(&server_mac(&token, &t)) },
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
        if std::env::var("WTE_WRITE_FIXTURES").is_ok_and(|v| v == "1") {
            std::fs::write(&path, &text).unwrap();
        }
        let stored = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(stored, text, "handshake.json differs from the Rust derivation");
        assert_eq!(crypto::normalize_code("k7qf-3m9t").as_deref(), Some(code));
    }
}
