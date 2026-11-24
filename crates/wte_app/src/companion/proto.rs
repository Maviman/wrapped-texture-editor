//! The companion wire protocol v1 (`docs/protocol/COMPANION_PROTOCOL.md`): message types, encode / decode, the
//! incremental `FrameReader` and the key table (4.8). Pure: no sockets, no egui state.
//!
//! Framing (4.1): after the 4-byte magic, every message is `len u32, type u16, flags u16, payload[len]`, little-endian.
//! Decoding follows the compatibility rules of 4.2: unknown types decode to `Msg::Unknown` (the caller skips them),
//! trailing payload bytes are ignored, a short payload is an error.

use eframe::egui;

/// The 4 bytes each side sends first.
pub const MAGIC: [u8; 4] = *b"WTEC";
/// The major version this build speaks (4.2).
pub const PROTO_VERSION: u16 = 1;
/// Largest client-to-server payload (4.1).
pub const MAX_C2S: u32 = 65_536;
/// Largest server-to-client payload (4.1).
pub const MAX_S2C: u32 = 16 << 20;
/// Header bytes before a payload.
pub const HEADER: usize = 8;

/// Message type numbers (4.3 - 4.5).
pub mod ty {
    pub const HELLO: u16 = 0x0001;
    pub const AUTH: u16 = 0x0002;
    pub const CONFIG: u16 = 0x0010;
    pub const FRAME_ACK: u16 = 0x0011;
    pub const PEN: u16 = 0x0012;
    pub const TOUCH: u16 = 0x0013;
    pub const KEY: u16 = 0x0014;
    pub const TEXT: u16 = 0x0015;
    pub const MODS: u16 = 0x0016;
    pub const ACTION: u16 = 0x0017;
    pub const FIT_REQUEST: u16 = 0x0018;
    pub const PING: u16 = 0x001E;
    pub const BYE_C: u16 = 0x001F;
    pub const HELLO_ACK: u16 = 0x0081;
    pub const AUTH_WAIT: u16 = 0x0082;
    pub const AUTH_OK: u16 = 0x0083;
    pub const ERROR: u16 = 0x008F;
    pub const STATE: u16 = 0x0090;
    pub const FRAME: u16 = 0x0091;
    pub const CURSOR: u16 = 0x0092;
    pub const PONG: u16 = 0x009E;
    pub const BYE_S: u16 = 0x009F;
}

/// HELLO `link` values.
pub mod link {
    pub const USB: u8 = 1;
    pub const WIFI: u8 = 2;
    pub const BLUETOOTH: u8 = 3;
}

/// AUTH `mode` values and HELLO_ACK `auth_modes` bits (bit = 1 << (mode - 1)).
pub mod auth_mode {
    pub const TOKEN: u8 = 1;
    pub const CODE: u8 = 2;
    pub const CONFIRM: u8 = 3;
    /// The `auth_modes` bit of a mode.
    pub const fn bit(mode: u8) -> u8 {
        1 << (mode - 1)
    }
}

/// ERROR codes (4.3).
pub mod err {
    pub const VERSION: u16 = 1;
    pub const AUTH_FAILED: u16 = 2;
    pub const PAIRING_CLOSED: u16 = 3;
    pub const DENIED: u16 = 4;
    pub const BUSY: u16 = 5;
    pub const BAD_MESSAGE: u16 = 6;
    pub const TOO_MANY_ATTEMPTS: u16 = 7;
}

/// `caps` bits (4.2 rule 4).
pub mod caps {
    pub const CLIENT_PNG: u64 = 1 << 0;
    pub const CLIENT_WEBP: u64 = 1 << 1;
    pub const CLIENT_TRAIL: u64 = 1 << 2;
    pub const CLIENT_IME: u64 = 1 << 3;
    pub const SERVER_CURSOR: u64 = 1 << 0;
    pub const SERVER_FIT: u64 = 1 << 1;
    /// Reserved on both sides for an encrypted session (N1).
    pub const ENCRYPTED: u64 = 1 << 63;
}

/// STATE flags (4.5).
pub mod state_flag {
    pub const NO_PICTURE: u32 = 1 << 0;
    pub const TEXT_WANTED: u32 = 1 << 1;
    pub const TRAIL: u32 = 1 << 2;
    pub const POINTER_HELD_LOCALLY: u32 = 1 << 3;
    pub const FIT_UNAVAILABLE: u32 = 1 << 4;
    pub const FITTED: u32 = 1 << 5;
}

/// FRAME flags (4.5).
pub mod frame_flag {
    pub const KEY: u8 = 1 << 0;
    pub const REFINE: u8 = 1 << 1;
}

/// Tile codecs (4.5).
pub mod codec {
    pub const JPEG: u8 = 1;
    pub const PNG: u8 = 2;
    pub const WEBP: u8 = 3;
}

/// PEN sample kinds (4.4).
pub mod pen_kind {
    pub const HOVER: u8 = 0;
    pub const DOWN: u8 = 1;
    pub const MOVE: u8 = 2;
    pub const UP: u8 = 3;
    pub const LEAVE: u8 = 4;
    pub const CANCEL: u8 = 5;
}

/// PEN sample flags (4.4).
pub mod pen_flag {
    pub const BARREL: u8 = 1 << 0;
    pub const ERASER: u8 = 1 << 1;
    pub const BARREL2: u8 = 1 << 2;
}

/// TOUCH phases (4.4).
pub mod touch_phase {
    pub const DOWN: u8 = 0;
    pub const MOVE: u8 = 1;
    pub const UP: u8 = 2;
    pub const CANCEL: u8 = 3;
}

/// `mods` bits (4.4).
pub mod mods {
    pub const CTRL: u8 = 1;
    pub const SHIFT: u8 = 2;
    pub const ALT: u8 = 4;
}

/// ACTION ids (4.4).
pub mod action {
    pub const UNDO: u16 = 1;
    pub const REDO: u16 = 2;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    pub proto_min: u16,
    pub proto_max: u16,
    pub caps: u64,
    pub link: u8,
    pub tablet_id: [u8; 16],
    pub client_nonce: [u8; 16],
    pub screen_w: u16,
    pub screen_h: u16,
    pub density_dpi: u16,
    pub app_version: String,
    pub device_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HelloAck {
    pub proto: u16,
    pub caps: u64,
    pub desktop_id: [u8; 16],
    pub server_nonce: [u8; 16],
    pub auth_modes: u8,
    pub pbkdf2_iters: u32,
    pub app_version: String,
    pub desktop_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Auth {
    pub mode: u8,
    pub client_mac: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthOk {
    pub mode: u8,
    pub server_mac: [u8; 32],
    pub token: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorMsg {
    pub code: u16,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub view_w: u16,
    pub view_h: u16,
    pub max_fps: u8,
    pub quality_pref: u8,
    pub flags: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameAck {
    pub frame_seq: u32,
    pub decode_ms: u16,
}

/// One PEN sample (16 bytes on the wire).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PenSample {
    pub kind: u8,
    pub flags: u8,
    pub pressure: u16,
    pub x: f32,
    pub y: f32,
    pub tilt: u8,
    pub reserved: u8,
    pub dt_us: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub seq: u32,
    pub samples: Vec<PenSample>,
}

/// One TOUCH point (10 bytes on the wire).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TouchPoint {
    pub id: u8,
    pub phase: u8,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Touch {
    pub seq: u32,
    pub points: Vec<TouchPoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub seq: u32,
    pub key: u16,
    pub pressed: bool,
    pub repeat: bool,
    pub mods: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FitRequest {
    pub mode: u8,
    pub aspect_w: u16,
    pub aspect_h: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub surface_w: u16,
    pub surface_h: u16,
    pub pixels_per_point: f32,
    pub flags: u32,
    pub trail_rgba: u32,
    pub title: String,
}

/// One FRAME tile: a rectangle of the picture and its encoded bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub codec: u8,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub frame_seq: u32,
    pub flags: u8,
    pub den: u8,
    pub pic_w: u16,
    pub pic_h: u16,
    pub last_input_seq: u32,
    pub capture_ms: u32,
    pub tiles: Vec<Tile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub x: f32,
    pub y: f32,
    pub visible: bool,
    pub icon: u8,
}

/// Every message of protocol v1, plus the unknown ones (skipped by the receiver).
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Hello(Hello),
    Auth(Auth),
    Config(Config),
    FrameAck(FrameAck),
    Pen(Pen),
    Touch(Touch),
    Key(Key),
    Text { seq: u32, text: String },
    Mods { seq: u32, mods: u8 },
    Action { seq: u32, action: u16 },
    FitRequest(FitRequest),
    Ping { t: u32 },
    ByeC { reason: u8 },
    HelloAck(HelloAck),
    AuthWait,
    AuthOk(AuthOk),
    Error(ErrorMsg),
    State(State),
    Frame(Frame),
    Cursor(Cursor),
    Pong { t: u32 },
    ByeS { reason: u8 },
    Unknown { ty: u16, flags: u16, payload: Vec<u8> },
}

impl Msg {
    /// The message's type number.
    pub fn ty(&self) -> u16 {
        match self {
            Msg::Hello(_) => ty::HELLO,
            Msg::Auth(_) => ty::AUTH,
            Msg::Config(_) => ty::CONFIG,
            Msg::FrameAck(_) => ty::FRAME_ACK,
            Msg::Pen(_) => ty::PEN,
            Msg::Touch(_) => ty::TOUCH,
            Msg::Key(_) => ty::KEY,
            Msg::Text { .. } => ty::TEXT,
            Msg::Mods { .. } => ty::MODS,
            Msg::Action { .. } => ty::ACTION,
            Msg::FitRequest(_) => ty::FIT_REQUEST,
            Msg::Ping { .. } => ty::PING,
            Msg::ByeC { .. } => ty::BYE_C,
            Msg::HelloAck(_) => ty::HELLO_ACK,
            Msg::AuthWait => ty::AUTH_WAIT,
            Msg::AuthOk(_) => ty::AUTH_OK,
            Msg::Error(_) => ty::ERROR,
            Msg::State(_) => ty::STATE,
            Msg::Frame(_) => ty::FRAME,
            Msg::Cursor(_) => ty::CURSOR,
            Msg::Pong { .. } => ty::PONG,
            Msg::ByeS { .. } => ty::BYE_S,
            Msg::Unknown { ty, .. } => *ty,
        }
    }

    /// The input sequence number of an input message (4.4), `None` for the others.
    pub fn input_seq(&self) -> Option<u32> {
        match self {
            Msg::Pen(Pen { seq, .. }) | Msg::Touch(Touch { seq, .. }) | Msg::Key(Key { seq, .. }) => Some(*seq),
            Msg::Text { seq, .. } | Msg::Mods { seq, .. } | Msg::Action { seq, .. } => Some(*seq),
            _ => None,
        }
    }

    /// The payload bytes (no header).
    pub fn payload(&self) -> Vec<u8> {
        let mut w = W(Vec::with_capacity(64));
        match self {
            Msg::Hello(h) => {
                w.u16(h.proto_min);
                w.u16(h.proto_max);
                w.u64(h.caps);
                w.u8(h.link);
                w.bytes(&h.tablet_id);
                w.bytes(&h.client_nonce);
                w.u16(h.screen_w);
                w.u16(h.screen_h);
                w.u16(h.density_dpi);
                w.str(&h.app_version);
                w.str(&h.device_name);
            }
            Msg::Auth(a) => {
                w.u8(a.mode);
                w.bytes(&a.client_mac);
            }
            Msg::Config(c) => {
                w.u16(c.view_w);
                w.u16(c.view_h);
                w.u8(c.max_fps);
                w.u8(c.quality_pref);
                w.u16(c.flags);
            }
            Msg::FrameAck(a) => {
                w.u32(a.frame_seq);
                w.u16(a.decode_ms);
            }
            Msg::Pen(p) => {
                w.u32(p.seq);
                w.u16(p.samples.len().min(u16::MAX as usize) as u16);
                for s in p.samples.iter().take(u16::MAX as usize) {
                    w.u8(s.kind);
                    w.u8(s.flags);
                    w.u16(s.pressure);
                    w.f32(s.x);
                    w.f32(s.y);
                    w.u8(s.tilt);
                    w.u8(s.reserved);
                    w.u16(s.dt_us);
                }
            }
            Msg::Touch(t) => {
                w.u32(t.seq);
                w.u8(t.points.len().min(255) as u8);
                for p in t.points.iter().take(255) {
                    w.u8(p.id);
                    w.u8(p.phase);
                    w.f32(p.x);
                    w.f32(p.y);
                }
            }
            Msg::Key(k) => {
                w.u32(k.seq);
                w.u16(k.key);
                w.u8(u8::from(k.pressed));
                w.u8(u8::from(k.repeat));
                w.u8(k.mods);
            }
            Msg::Text { seq, text } => {
                w.u32(*seq);
                w.str(text);
            }
            Msg::Mods { seq, mods } => {
                w.u32(*seq);
                w.u8(*mods);
            }
            Msg::Action { seq, action } => {
                w.u32(*seq);
                w.u16(*action);
            }
            Msg::FitRequest(f) => {
                w.u8(f.mode);
                w.u16(f.aspect_w);
                w.u16(f.aspect_h);
            }
            Msg::Ping { t } | Msg::Pong { t } => w.u32(*t),
            Msg::ByeC { reason } | Msg::ByeS { reason } => w.u8(*reason),
            Msg::HelloAck(a) => {
                w.u16(a.proto);
                w.u64(a.caps);
                w.bytes(&a.desktop_id);
                w.bytes(&a.server_nonce);
                w.u8(a.auth_modes);
                w.u32(a.pbkdf2_iters);
                w.str(&a.app_version);
                w.str(&a.desktop_name);
            }
            Msg::AuthWait => {}
            Msg::AuthOk(a) => {
                w.u8(a.mode);
                w.bytes(&a.server_mac);
                w.bytes(&a.token);
            }
            Msg::Error(e) => {
                w.u16(e.code);
                w.str(&e.text);
            }
            Msg::State(s) => {
                w.u16(s.surface_w);
                w.u16(s.surface_h);
                w.f32(s.pixels_per_point);
                w.u32(s.flags);
                w.u32(s.trail_rgba);
                w.str(&s.title);
            }
            Msg::Frame(f) => {
                w.u32(f.frame_seq);
                w.u8(f.flags);
                w.u8(f.den);
                w.u16(f.pic_w);
                w.u16(f.pic_h);
                w.u32(f.last_input_seq);
                w.u32(f.capture_ms);
                w.u16(f.tiles.len().min(u16::MAX as usize) as u16);
                for t in f.tiles.iter().take(u16::MAX as usize) {
                    w.u16(t.x);
                    w.u16(t.y);
                    w.u16(t.w);
                    w.u16(t.h);
                    w.u8(t.codec);
                    w.u32(t.data.len() as u32);
                    w.bytes(&t.data);
                }
            }
            Msg::Cursor(c) => {
                w.f32(c.x);
                w.f32(c.y);
                w.u8(u8::from(c.visible));
                w.u8(c.icon);
            }
            Msg::Unknown { payload, .. } => w.bytes(payload),
        }
        w.0
    }

    /// The complete framed message: header plus payload.
    pub fn encode(&self) -> Vec<u8> {
        let payload = self.payload();
        let flags = match self {
            Msg::Unknown { flags, .. } => *flags,
            _ => 0,
        };
        let mut out = Vec::with_capacity(HEADER + payload.len());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ty().to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    /// Decodes one payload of type `ty` (4.2: trailing bytes ignored, unknown types kept as `Unknown`).
    pub fn decode(ty: u16, flags: u16, payload: &[u8]) -> Result<Msg, ProtoError> {
        let mut r = R { b: payload, at: 0, ty };
        let msg = match ty {
            ty::HELLO => Msg::Hello(Hello {
                proto_min: r.u16()?,
                proto_max: r.u16()?,
                caps: r.u64()?,
                link: r.u8()?,
                tablet_id: r.array()?,
                client_nonce: r.array()?,
                screen_w: r.u16()?,
                screen_h: r.u16()?,
                density_dpi: r.u16()?,
                app_version: r.str()?,
                device_name: r.str()?,
            }),
            ty::AUTH => Msg::Auth(Auth { mode: r.u8()?, client_mac: r.array()? }),
            ty::CONFIG => Msg::Config(Config {
                view_w: r.u16()?,
                view_h: r.u16()?,
                max_fps: r.u8()?,
                quality_pref: r.u8()?,
                flags: r.u16()?,
            }),
            ty::FRAME_ACK => Msg::FrameAck(FrameAck { frame_seq: r.u32()?, decode_ms: r.u16()? }),
            ty::PEN => {
                let seq = r.u32()?;
                let count = r.u16()? as usize;
                let mut samples = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    samples.push(PenSample {
                        kind: r.u8()?,
                        flags: r.u8()?,
                        pressure: r.u16()?,
                        x: r.f32()?,
                        y: r.f32()?,
                        tilt: r.u8()?,
                        reserved: r.u8()?,
                        dt_us: r.u16()?,
                    });
                }
                Msg::Pen(Pen { seq, samples })
            }
            ty::TOUCH => {
                let seq = r.u32()?;
                let count = r.u8()? as usize;
                let mut points = Vec::with_capacity(count);
                for _ in 0..count {
                    points.push(TouchPoint { id: r.u8()?, phase: r.u8()?, x: r.f32()?, y: r.f32()? });
                }
                Msg::Touch(Touch { seq, points })
            }
            ty::KEY => Msg::Key(Key { seq: r.u32()?, key: r.u16()?, pressed: r.u8()? != 0, repeat: r.u8()? != 0, mods: r.u8()? }),
            ty::TEXT => Msg::Text { seq: r.u32()?, text: r.str()? },
            ty::MODS => Msg::Mods { seq: r.u32()?, mods: r.u8()? },
            ty::ACTION => Msg::Action { seq: r.u32()?, action: r.u16()? },
            ty::FIT_REQUEST => Msg::FitRequest(FitRequest { mode: r.u8()?, aspect_w: r.u16()?, aspect_h: r.u16()? }),
            ty::PING => Msg::Ping { t: r.u32()? },
            ty::BYE_C => Msg::ByeC { reason: r.u8()? },
            ty::HELLO_ACK => Msg::HelloAck(HelloAck {
                proto: r.u16()?,
                caps: r.u64()?,
                desktop_id: r.array()?,
                server_nonce: r.array()?,
                auth_modes: r.u8()?,
                pbkdf2_iters: r.u32()?,
                app_version: r.str()?,
                desktop_name: r.str()?,
            }),
            ty::AUTH_WAIT => Msg::AuthWait,
            ty::AUTH_OK => Msg::AuthOk(AuthOk { mode: r.u8()?, server_mac: r.array()?, token: r.array()? }),
            ty::ERROR => Msg::Error(ErrorMsg { code: r.u16()?, text: r.str()? }),
            ty::STATE => Msg::State(State {
                surface_w: r.u16()?,
                surface_h: r.u16()?,
                pixels_per_point: r.f32()?,
                flags: r.u32()?,
                trail_rgba: r.u32()?,
                title: r.str()?,
            }),
            ty::FRAME => {
                let (frame_seq, flags, den, pic_w, pic_h) = (r.u32()?, r.u8()?, r.u8()?, r.u16()?, r.u16()?);
                let (last_input_seq, capture_ms, count) = (r.u32()?, r.u32()?, r.u16()? as usize);
                let mut tiles = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    let (x, y, w, h, codec) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u8()?);
                    let len = r.u32()? as usize;
                    tiles.push(Tile { x, y, w, h, codec, data: r.take(len)?.to_vec() });
                }
                Msg::Frame(Frame { frame_seq, flags, den, pic_w, pic_h, last_input_seq, capture_ms, tiles })
            }
            ty::CURSOR => Msg::Cursor(Cursor { x: r.f32()?, y: r.f32()?, visible: r.u8()? != 0, icon: r.u8()? }),
            ty::PONG => Msg::Pong { t: r.u32()? },
            ty::BYE_S => Msg::ByeS { reason: r.u8()? },
            _ => Msg::Unknown { ty, flags, payload: payload.to_vec() },
        };
        Ok(msg)
    }
}

/// A protocol violation; the connection is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    /// The payload of this type ended early.
    Truncated(u16),
    /// A length above the direction's limit.
    TooLong(u32),
    /// A `str` that is not UTF-8.
    BadUtf8(u16),
    /// The first 4 bytes were not "WTEC".
    BadMagic,
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Truncated(t) => write!(f, "message 0x{t:04x} is truncated"),
            ProtoError::TooLong(n) => write!(f, "message length {n} is over the limit"),
            ProtoError::BadUtf8(t) => write!(f, "message 0x{t:04x} has a string that is not UTF-8"),
            ProtoError::BadMagic => write!(f, "the peer did not send the WTEC magic"),
        }
    }
}

/// Little-endian writer.
struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    /// `u16` byte length plus UTF-8; a longer string is cut at a character boundary below 65,536 bytes.
    fn str(&mut self, s: &str) {
        let mut end = s.len().min(u16::MAX as usize);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.u16(end as u16);
        self.bytes(&s.as_bytes()[..end]);
    }
}

/// Little-endian reader over one payload.
struct R<'a> {
    b: &'a [u8],
    at: usize,
    ty: u16,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], ProtoError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(ProtoError::Truncated(self.ty))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtoError> {
        let s = self.take(N)?;
        Ok(s.try_into().expect("length checked"))
    }
    fn u8(&mut self) -> Result<u8, ProtoError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ProtoError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, ProtoError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, ProtoError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, ProtoError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    fn str(&mut self) -> Result<String, ProtoError> {
        let n = self.u16()? as usize;
        let ty = self.ty;
        let s = self.take(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| ProtoError::BadUtf8(ty))
    }
}

/// One framed message as read: type, flags, payload.
#[derive(Clone, Debug, PartialEq)]
pub struct RawMsg {
    pub ty: u16,
    pub flags: u16,
    pub payload: Vec<u8>,
}

impl RawMsg {
    pub fn decode(&self) -> Result<Msg, ProtoError> {
        Msg::decode(self.ty, self.flags, &self.payload)
    }
}

/// Incremental framing: bytes in (any split), whole messages out. The magic is not handled here.
pub struct FrameReader {
    buf: Vec<u8>,
    start: usize,
    max: u32,
}

impl FrameReader {
    /// `max` = the largest payload this direction allows (`MAX_C2S` / `MAX_S2C`).
    pub fn new(max: u32) -> FrameReader {
        FrameReader { buf: Vec::new(), start: 0, max }
    }

    pub fn push(&mut self, data: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > 1 << 16 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(data);
    }

    /// Bytes received but not yet returned as a message.
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }

    /// The next complete message, `Ok(None)` when more bytes are needed, `Err(TooLong)` for a length over the limit.
    pub fn next_raw(&mut self) -> Result<Option<RawMsg>, ProtoError> {
        let b = &self.buf[self.start..];
        if b.len() < HEADER {
            return Ok(None);
        }
        let len = u32::from_le_bytes(b[0..4].try_into().expect("4 bytes"));
        if len > self.max {
            return Err(ProtoError::TooLong(len));
        }
        let total = HEADER + len as usize;
        if b.len() < total {
            return Ok(None);
        }
        let raw = RawMsg {
            ty: u16::from_le_bytes([b[4], b[5]]),
            flags: u16::from_le_bytes([b[6], b[7]]),
            payload: b[HEADER..total].to_vec(),
        };
        self.start += total;
        Ok(Some(raw))
    }

    /// The next complete message, decoded.
    pub fn next_msg(&mut self) -> Result<Option<Msg>, ProtoError> {
        match self.next_raw()? {
            Some(raw) => raw.decode().map(Some),
            None => Ok(None),
        }
    }
}

/// The key table of 4.8: (protocol code, egui key).
pub const KEY_TABLE: &[(u16, egui::Key)] = {
    use egui::Key::*;
    &[
        (1, A),
        (2, B),
        (3, C),
        (4, D),
        (5, E),
        (6, F),
        (7, G),
        (8, H),
        (9, I),
        (10, J),
        (11, K),
        (12, L),
        (13, M),
        (14, N),
        (15, O),
        (16, P),
        (17, Q),
        (18, R),
        (19, S),
        (20, T),
        (21, U),
        (22, V),
        (23, W),
        (24, X),
        (25, Y),
        (26, Z),
        (30, Num0),
        (31, Num1),
        (32, Num2),
        (33, Num3),
        (34, Num4),
        (35, Num5),
        (36, Num6),
        (37, Num7),
        (38, Num8),
        (39, Num9),
        (40, Enter),
        (41, Escape),
        (42, Backspace),
        (43, Tab),
        (44, Space),
        (45, Delete),
        (46, Insert),
        (47, Home),
        (48, End),
        (49, PageUp),
        (50, PageDown),
        (51, ArrowLeft),
        (52, ArrowRight),
        (53, ArrowUp),
        (54, ArrowDown),
        (60, F1),
        (61, F2),
        (62, F3),
        (63, F4),
        (64, F5),
        (65, F6),
        (66, F7),
        (67, F8),
        (68, F9),
        (69, F10),
        (70, F11),
        (71, F12),
        (80, Minus),
        (81, Equals),
        (82, OpenBracket),
        (83, CloseBracket),
        (84, Comma),
        (85, Period),
        (86, Slash),
        (87, Backslash),
        (88, Semicolon),
        (89, Quote),
        (90, Backtick),
    ]
};

/// The egui key of a protocol key code; `None` for an unknown code (ignored, 4.8).
pub fn key_from_code(code: u16) -> Option<egui::Key> {
    KEY_TABLE.iter().find(|(c, _)| *c == code).map(|(_, k)| *k)
}

/// The protocol key code of an egui key (the test client and the tests).
pub fn code_from_key(key: egui::Key) -> Option<u16> {
    KEY_TABLE.iter().find(|(_, k)| *k == key).map(|(c, _)| *c)
}

/// egui modifiers from the `mods` bits.
pub fn modifiers_from_bits(bits: u8) -> egui::Modifiers {
    let (ctrl, shift, alt) = (bits & mods::CTRL != 0, bits & mods::SHIFT != 0, bits & mods::ALT != 0);
    egui::Modifiers { alt, ctrl, shift, mac_cmd: false, command: ctrl }
}

/// The `mods` bits of egui modifiers.
pub fn bits_from_modifiers(m: egui::Modifiers) -> u8 {
    (if m.ctrl || m.command { mods::CTRL } else { 0 }) | (if m.shift { mods::SHIFT } else { 0 }) | (if m.alt { mods::ALT } else { 0 })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One instance of every message type, with values that exercise every field (the golden fixtures are these).
    pub(crate) fn samples() -> Vec<(&'static str, Msg)> {
        let tablet_id: [u8; 16] = std::array::from_fn(|i| 0xa0 + i as u8);
        let desktop_id: [u8; 16] = std::array::from_fn(|i| 0x10 + i as u8);
        let nonce_c: [u8; 16] = std::array::from_fn(|i| 0xc0 + i as u8);
        let nonce_s: [u8; 16] = std::array::from_fn(|i| 0x50 + i as u8);
        let mac: [u8; 32] = std::array::from_fn(|i| i as u8);
        let token: [u8; 32] = std::array::from_fn(|i| 0xff - i as u8);
        vec![
            (
                "hello",
                Msg::Hello(Hello {
                    proto_min: 1,
                    proto_max: 1,
                    caps: caps::CLIENT_PNG | caps::CLIENT_TRAIL | caps::CLIENT_IME,
                    link: link::WIFI,
                    tablet_id,
                    client_nonce: nonce_c,
                    screen_w: 2960,
                    screen_h: 1848,
                    density_dpi: 340,
                    app_version: "1.0.0".into(),
                    device_name: "Galaxy Tab S8 Ultra".into(),
                }),
            ),
            (
                "hello_ack",
                Msg::HelloAck(HelloAck {
                    proto: 1,
                    caps: caps::SERVER_CURSOR | caps::SERVER_FIT,
                    desktop_id,
                    server_nonce: nonce_s,
                    auth_modes: auth_mode::bit(auth_mode::TOKEN) | auth_mode::bit(auth_mode::CODE),
                    pbkdf2_iters: 100_000,
                    app_version: "0.2.0".into(),
                    desktop_name: "STUDIO-PC".into(),
                }),
            ),
            ("auth", Msg::Auth(Auth { mode: auth_mode::CODE, client_mac: mac })),
            ("auth_wait", Msg::AuthWait),
            ("auth_ok", Msg::AuthOk(AuthOk { mode: auth_mode::CONFIRM, server_mac: [0; 32], token })),
            ("error", Msg::Error(ErrorMsg { code: err::PAIRING_CLOSED, text: "Open Help > Tablet companion".into() })),
            ("config", Msg::Config(Config { view_w: 2960, view_h: 1848, max_fps: 0, quality_pref: 2, flags: 0 })),
            ("frame_ack", Msg::FrameAck(FrameAck { frame_seq: 7, decode_ms: 12 })),
            (
                "pen",
                Msg::Pen(Pen {
                    seq: 42,
                    samples: vec![
                        PenSample { kind: pen_kind::HOVER, flags: 0, pressure: 0, x: 100.5, y: 200.25, tilt: 255, reserved: 0, dt_us: 0 },
                        PenSample { kind: pen_kind::DOWN, flags: pen_flag::BARREL, pressure: 6554, x: 101.0, y: 201.0, tilt: 30, reserved: 0, dt_us: 4000 },
                        PenSample { kind: pen_kind::MOVE, flags: pen_flag::ERASER | pen_flag::BARREL2, pressure: 65535, x: 102.75, y: 202.5, tilt: 90, reserved: 0, dt_us: 65535 },
                    ],
                }),
            ),
            (
                "touch",
                Msg::Touch(Touch {
                    seq: 43,
                    points: vec![
                        TouchPoint { id: 0, phase: touch_phase::DOWN, x: 640.0, y: 400.0 },
                        TouchPoint { id: 1, phase: touch_phase::MOVE, x: 700.5, y: 420.25 },
                    ],
                }),
            ),
            ("key", Msg::Key(Key { seq: 44, key: 26, pressed: true, repeat: false, mods: mods::CTRL })),
            ("text", Msg::Text { seq: 45, text: "Hé!".into() }),
            ("mods", Msg::Mods { seq: 46, mods: mods::SHIFT | mods::ALT }),
            ("action", Msg::Action { seq: 47, action: action::UNDO }),
            ("fit_request", Msg::FitRequest(FitRequest { mode: 1, aspect_w: 2960, aspect_h: 1848 })),
            ("ping", Msg::Ping { t: 123_456 }),
            ("bye_c", Msg::ByeC { reason: 1 }),
            (
                "state",
                Msg::State(State {
                    surface_w: 1280,
                    surface_h: 800,
                    pixels_per_point: 1.25,
                    flags: state_flag::TRAIL | state_flag::FITTED,
                    trail_rgba: 0xffd0_4020,
                    title: "sphere - Wrapped Texture Editor".into(),
                }),
            ),
            ("frame", Msg::Frame(fixture_frame())),
            ("cursor", Msg::Cursor(Cursor { x: 320.5, y: 240.0, visible: true, icon: 0 })),
            ("pong", Msg::Pong { t: 123_456 }),
            ("bye_s", Msg::ByeS { reason: 0 }),
        ]
    }

    /// The FRAME fixture: a KEY frame with two real 16x16 JPEG tiles (red and blue), so a client can draw it.
    fn fixture_frame() -> Frame {
        let tile = |rgb: [u8; 3]| -> Vec<u8> {
            let data: Vec<u8> = (0..16 * 16).flat_map(|_| rgb).collect();
            wte_core::encode_jpeg(16, 16, &data, 90).expect("jpeg")
        };
        Frame {
            frame_seq: 1,
            flags: frame_flag::KEY,
            den: 1,
            pic_w: 32,
            pic_h: 16,
            last_input_seq: 0,
            capture_ms: 5,
            tiles: vec![
                Tile { x: 0, y: 0, w: 16, h: 16, codec: codec::JPEG, data: tile([220, 30, 40]) },
                Tile { x: 16, y: 0, w: 16, h: 16, codec: codec::JPEG, data: tile([30, 90, 230]) },
            ],
        }
    }

    /// The fixture directory (`docs/protocol/fixtures/v1`).
    pub(crate) fn fixture_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/protocol/fixtures/v1")
    }

    /// The bytes of a fixture file (hex text).
    pub(crate) fn fixture(name: &str) -> Vec<u8> {
        let path = fixture_dir().join(format!("{name}.hex"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        crate::companion::crypto::unhex(text.trim()).unwrap_or_else(|| panic!("{} is not hex", path.display()))
    }

    #[test]
    fn every_message_round_trips() {
        for (name, msg) in samples() {
            let bytes = msg.encode();
            let mut r = FrameReader::new(MAX_S2C);
            r.push(&bytes);
            let back = r.next_msg().unwrap().unwrap_or_else(|| panic!("{name}: incomplete"));
            assert_eq!(back, msg, "{name}");
            assert_eq!(r.pending(), 0, "{name}");
            assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize, bytes.len() - HEADER);
        }
    }

    /// Writes the fixtures when `WTE_WRITE_FIXTURES=1` (done once by the desktop builder; the FRAME fixture's JPEG
    /// bytes depend on the encoder version, so it is never rewritten implicitly). Without the variable it checks them:
    /// every fixture decodes to the sample message (FRAME: the header and both tiles decode as 16x16 pictures) and
    /// re-encodes byte for byte.
    #[test]
    fn golden_fixtures_decode_and_reencode_byte_for_byte() {
        let dir = fixture_dir();
        if std::env::var("WTE_WRITE_FIXTURES").is_ok_and(|v| v == "1") {
            std::fs::create_dir_all(&dir).unwrap();
            for (name, msg) in samples() {
                let text = format!("{}\n", crate::companion::crypto::hex(&msg.encode()));
                std::fs::write(dir.join(format!("{name}.hex")), text).unwrap();
            }
        }
        for (name, msg) in samples() {
            let bytes = fixture(name);
            let mut r = FrameReader::new(MAX_S2C);
            r.push(&bytes);
            let back = r.next_msg().unwrap().unwrap_or_else(|| panic!("{name}: incomplete fixture"));
            assert_eq!(back.encode(), bytes, "{name}: re-encode differs");
            match (&back, &msg) {
                (Msg::Frame(f), Msg::Frame(m)) => {
                    assert_eq!((f.frame_seq, f.flags, f.den, f.pic_w, f.pic_h), (m.frame_seq, m.flags, m.den, m.pic_w, m.pic_h));
                    assert_eq!(f.tiles.len(), 2);
                    for t in &f.tiles {
                        let img = wte_core::decode_image(&t.data, Some("tile.jpg"), 64).unwrap();
                        assert_eq!(img.original_size, [16, 16]);
                    }
                }
                _ => assert_eq!(&back, &msg, "{name}: decoded value differs"),
            }
        }
        // Every type of the spec has its fixture.
        let names: Vec<&str> = samples().iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), 22);
    }

    #[test]
    fn frame_reader_takes_any_split() {
        let all: Vec<u8> = samples().iter().flat_map(|(_, m)| m.encode()).collect();
        let want: Vec<Msg> = samples().into_iter().map(|(_, m)| m).collect();
        for split in [1usize, 2, 3, 7, 8, 9, 64, 1000] {
            let mut r = FrameReader::new(MAX_S2C);
            let mut got = Vec::new();
            for chunk in all.chunks(split) {
                r.push(chunk);
                while let Some(m) = r.next_msg().unwrap() {
                    got.push(m);
                }
            }
            assert_eq!(got, want, "split {split}");
        }
    }

    #[test]
    fn unknown_types_longer_and_shorter_payloads() {
        // Unknown type: kept as Unknown with its bytes (the receiver skips it).
        let unknown = Msg::Unknown { ty: 0x0777, flags: 3, payload: vec![1, 2, 3] };
        let mut r = FrameReader::new(MAX_C2S);
        r.push(&unknown.encode());
        r.push(&Msg::Ping { t: 9 }.encode());
        assert_eq!(r.next_msg().unwrap(), Some(unknown));
        assert_eq!(r.next_msg().unwrap(), Some(Msg::Ping { t: 9 }));
        // A longer payload (a later minor version): trailing bytes ignored.
        let mut p = Msg::FrameAck(FrameAck { frame_seq: 5, decode_ms: 3 }).payload();
        p.extend_from_slice(&[9, 9, 9, 9]);
        assert_eq!(Msg::decode(ty::FRAME_ACK, 0, &p), Ok(Msg::FrameAck(FrameAck { frame_seq: 5, decode_ms: 3 })));
        let mut hello = samples()[0].1.payload();
        hello.extend_from_slice(b"future fields");
        assert_eq!(Msg::decode(ty::HELLO, 0, &hello).unwrap(), samples()[0].1);
        // A truncated payload is an error for every type that has fields.
        for (name, msg) in samples() {
            let p = msg.payload();
            if p.is_empty() {
                continue;
            }
            assert_eq!(Msg::decode(msg.ty(), 0, &p[..p.len() - 1]), Err(ProtoError::Truncated(msg.ty())), "{name}");
        }
        // A bad string.
        let mut bad = Vec::new();
        bad.extend_from_slice(&5u32.to_le_bytes());
        bad.extend_from_slice(&2u16.to_le_bytes());
        bad.extend_from_slice(&[0xff, 0xfe]);
        assert_eq!(Msg::decode(ty::TEXT, 0, &bad), Err(ProtoError::BadUtf8(ty::TEXT)));
    }

    #[test]
    fn too_long_lengths_are_rejected() {
        let mut r = FrameReader::new(MAX_C2S);
        let mut header = (MAX_C2S + 1).to_le_bytes().to_vec();
        header.extend_from_slice(&[0x12, 0, 0, 0]);
        r.push(&header);
        assert_eq!(r.next_raw(), Err(ProtoError::TooLong(MAX_C2S + 1)));
        // Exactly the limit is fine (waits for the bytes).
        let mut r = FrameReader::new(MAX_C2S);
        let mut header = MAX_C2S.to_le_bytes().to_vec();
        header.extend_from_slice(&[0x77, 0, 0, 0]);
        r.push(&header);
        assert_eq!(r.next_raw(), Ok(None));
        r.push(&vec![0u8; MAX_C2S as usize]);
        assert_eq!(r.next_raw().unwrap().map(|m| m.payload.len()), Some(MAX_C2S as usize));
    }

    #[test]
    fn key_table_maps_every_code_both_ways() {
        let mut codes: Vec<u16> = KEY_TABLE.iter().map(|(c, _)| *c).collect();
        let n = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), n, "codes are unique");
        assert_eq!(n, 26 + 10 + 15 + 12 + 11);
        for &(code, key) in KEY_TABLE {
            assert_eq!(key_from_code(code), Some(key));
            assert_eq!(code_from_key(key), Some(code));
        }
        for unknown in [0u16, 27, 29, 55, 59, 72, 79, 91, 1000, u16::MAX] {
            assert_eq!(key_from_code(unknown), None, "{unknown}");
        }
        assert_eq!(key_from_code(26), Some(egui::Key::Z));
        assert_eq!(key_from_code(40), Some(egui::Key::Enter));
        assert_eq!(key_from_code(90), Some(egui::Key::Backtick));
        assert_eq!(bits_from_modifiers(modifiers_from_bits(7)), 7);
        assert_eq!(modifiers_from_bits(mods::CTRL), egui::Modifiers { command: true, ..egui::Modifiers::CTRL }, "as egui-winit on Windows");
        assert_eq!(modifiers_from_bits(0), egui::Modifiers::NONE);
    }
}
