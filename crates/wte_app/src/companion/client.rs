//! `TestClient`: a minimal blocking protocol client (COMPANION_PLAN 2.1) for the loopback tests and the `companion`
//! self-test scenario. It does what the tablet app does on the wire: magic, HELLO, AUTH (verifying the server MAC),
//! then it keeps the picture up to date from FRAME tiles (decoded with `wte_core::decode_image`), acknowledges frames
//! and sends input with its own sequence numbers. Only ever connects to 127.0.0.1 in tests and scenarios.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use wte_core::PixelBuf;

use super::auth::{self, ClientKey};
use super::crypto::{self, ct_eq};
use super::proto::{self, Frame, FrameReader, HelloAck, Msg, auth_mode};

pub struct TestClient {
    stream: TcpStream,
    reader: FrameReader,
    seq: u32,
    hello: Option<proto::Hello>,
    /// The picture as the tablet would show it (RGBA, picture pixels).
    pub picture: Option<PixelBuf>,
    pub state: Option<proto::State>,
    /// Every FRAME received, tiles dropped after they were applied.
    pub frames: Vec<Frame>,
    /// Acknowledge every frame as soon as it is applied (the tablet's behaviour).
    pub auto_ack: bool,
    /// AUTH_WAIT messages seen during the handshake.
    pub auth_waits: u32,
    /// Messages received that `pump` did not consume (PONG, CURSOR, ERROR, ...).
    pub inbox: Vec<Msg>,
}

/// Error text of an ERROR message (the tests match on "code N").
fn error_text(e: &proto::ErrorMsg) -> String {
    format!("ERROR code {}: {}", e.code, e.text)
}

impl TestClient {
    /// Connects and exchanges the magic.
    pub fn connect(addr: SocketAddr) -> io::Result<TestClient> {
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
        stream.set_nodelay(true)?;
        stream.write_all(&proto::MAGIC)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut magic = [0u8; 4];
        stream.read_exact(&mut magic)?;
        if magic != proto::MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "no WTEC magic"));
        }
        Ok(TestClient {
            stream,
            reader: FrameReader::new(proto::MAX_S2C),
            seq: 0,
            hello: None,
            picture: None,
            state: None,
            frames: Vec::new(),
            auto_ack: true,
            auth_waits: 0,
            inbox: Vec::new(),
        })
    }

    pub fn send(&mut self, msg: &Msg) -> io::Result<()> {
        self.stream.write_all(&msg.encode())
    }

    /// The next message within `timeout` (`Ok(None)` on timeout).
    pub fn recv(&mut self, timeout: Duration) -> io::Result<Option<Msg>> {
        let deadline = Instant::now() + timeout;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if let Some(m) = self.reader.next_msg().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))? {
                return Ok(Some(m));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            self.stream.set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed by the server")),
                Ok(n) => self.reader.push(&buf[..n]),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }

    /// Sends HELLO; the HELLO_ACK, or the server's error.
    pub fn hello(&mut self, hello: proto::Hello) -> Result<HelloAck, String> {
        self.hello = Some(hello.clone());
        let payload = Msg::Hello(hello).payload();
        self.hello_raw(&payload)
    }

    /// Sends a HELLO with this exact payload (a longer HELLO of a later version); `hello` must describe its fields.
    pub fn hello_raw(&mut self, payload: &[u8]) -> Result<HelloAck, String> {
        if self.hello.is_none() {
            self.hello = Some(match Msg::decode(proto::ty::HELLO, 0, payload) {
                Ok(Msg::Hello(h)) => h,
                _ => return Err("not a HELLO payload".into()),
            });
        }
        let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(&proto::ty::HELLO.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(payload);
        self.stream.write_all(&bytes).map_err(|e| e.to_string())?;
        match self.recv(Duration::from_secs(5)).map_err(|e| e.to_string())? {
            Some(Msg::HelloAck(a)) => Ok(a),
            Some(Msg::Error(e)) => Err(error_text(&e)),
            other => Err(format!("unexpected answer to HELLO: {other:?}")),
        }
    }

    /// AUTH with `key` (the mode follows from the key); verifies the server MAC; returns the session token.
    pub fn auth(&mut self, ack: &HelloAck, key: ClientKey) -> Result<[u8; 32], String> {
        let hello = self.hello.clone().ok_or("no HELLO sent")?;
        if ack.pbkdf2_iters > auth::MAX_ITERS {
            return Err(format!("the server asks for {} PBKDF2 iterations", ack.pbkdf2_iters));
        }
        let mode = match key {
            ClientKey::Token(_) => auth_mode::TOKEN,
            ClientKey::Code(_) => auth_mode::CODE,
            ClientKey::Confirm => auth_mode::CONFIRM,
        };
        let (msg, expect) = auth::client_auth(ack, &hello, mode, &key);
        self.send(&Msg::Auth(msg)).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(70);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv(left).map_err(|e| e.to_string())? {
                Some(Msg::AuthWait) => self.auth_waits += 1,
                Some(Msg::AuthOk(ok)) => {
                    return match (&key, expect) {
                        (ClientKey::Confirm, _) => Ok(ok.token),
                        (_, Some(server_mac)) if ct_eq(&ok.server_mac, &server_mac) => Ok(match key {
                            ClientKey::Token(t) => t,
                            ClientKey::Code(code) => {
                                let salt = auth::pair_salt(&ack.desktop_id, &hello.tablet_id, &ack.server_nonce, &hello.client_nonce);
                                auth::token_from_pair_key(&auth::pair_key(&code, &salt, ack.pbkdf2_iters))
                            }
                            ClientKey::Confirm => unreachable!(),
                        }),
                        _ => Err("the server MAC does not match".into()),
                    };
                }
                Some(Msg::Error(e)) => return Err(error_text(&e)),
                Some(other) => return Err(format!("unexpected answer to AUTH: {other:?}")),
                None => return Err("no answer to AUTH".into()),
            }
        }
    }

    /// HELLO + AUTH.
    pub fn handshake(&mut self, hello: proto::Hello, key: ClientKey) -> Result<[u8; 32], String> {
        let ack = self.hello(hello)?;
        self.auth(&ack, key)
    }

    /// The next input sequence number.
    pub fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    /// The last input sequence number sent.
    pub fn last_seq(&self) -> u32 {
        self.seq
    }

    pub fn pen(&mut self, samples: Vec<proto::PenSample>) -> io::Result<u32> {
        let seq = self.next_seq();
        self.send(&Msg::Pen(proto::Pen { seq, samples })).map(|_| seq)
    }

    pub fn touch(&mut self, points: Vec<proto::TouchPoint>) -> io::Result<u32> {
        let seq = self.next_seq();
        self.send(&Msg::Touch(proto::Touch { seq, points })).map(|_| seq)
    }

    pub fn key(&mut self, key: u16, pressed: bool, mods: u8) -> io::Result<u32> {
        let seq = self.next_seq();
        self.send(&Msg::Key(proto::Key { seq, key, pressed, repeat: false, mods })).map(|_| seq)
    }

    pub fn text(&mut self, text: &str) -> io::Result<u32> {
        let seq = self.next_seq();
        self.send(&Msg::Text { seq, text: text.to_owned() }).map(|_| seq)
    }

    pub fn mods(&mut self, mods: u8) -> io::Result<u32> {
        let seq = self.next_seq();
        self.send(&Msg::Mods { seq, mods }).map(|_| seq)
    }

    pub fn action(&mut self, action: u16) -> io::Result<u32> {
        let seq = self.next_seq();
        self.send(&Msg::Action { seq, action }).map(|_| seq)
    }

    pub fn ack(&mut self, frame_seq: u32) -> io::Result<()> {
        self.send(&Msg::FrameAck(proto::FrameAck { frame_seq, decode_ms: 1 }))
    }

    /// Applies a FRAME to the picture (KEY replaces it).
    fn apply(&mut self, f: &Frame) -> Result<(), String> {
        let key = f.flags & proto::frame_flag::KEY != 0;
        let (w, h) = (u32::from(f.pic_w), u32::from(f.pic_h));
        if key || self.picture.as_ref().is_none_or(|p| (p.width, p.height) != (w, h)) {
            self.picture = Some(PixelBuf::new(w, h));
        }
        let pic = self.picture.as_mut().expect("set above");
        for t in &f.tiles {
            let img = wte_core::decode_image(&t.data, Some("tile.jpg"), 4096).map_err(|e| e.to_string())?;
            let tile = img.pixels;
            for y in 0..tile.height.min(h.saturating_sub(u32::from(t.y))) {
                for x in 0..tile.width.min(w.saturating_sub(u32::from(t.x))) {
                    let s = ((y * tile.width + x) * 4) as usize;
                    let d = (((u32::from(t.y) + y) * w + u32::from(t.x) + x) * 4) as usize;
                    pic.data[d..d + 4].copy_from_slice(&tile.data[s..s + 4]);
                }
            }
        }
        Ok(())
    }

    /// Reads for up to `timeout`, applying STATE and FRAME (acking when `auto_ack`); returns the first FRAME, or
    /// `None` when none came.
    pub fn wait_frame(&mut self, timeout: Duration) -> Option<Frame> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv(left) {
                Ok(Some(Msg::Frame(f))) => {
                    if let Err(e) = self.apply(&f) {
                        log::warn!("test client: bad tile: {e}");
                    }
                    if self.auto_ack {
                        let _ = self.ack(f.frame_seq);
                    }
                    let mut kept = f.clone();
                    kept.tiles.iter_mut().for_each(|t| t.data = Vec::new());
                    self.frames.push(kept);
                    return Some(f);
                }
                Ok(Some(Msg::State(s))) => self.state = Some(s),
                Ok(Some(other)) => self.inbox.push(other),
                Ok(None) | Err(_) => return None,
            }
        }
    }

    /// Reads (applying frames) until a received message matches `pred` or `timeout` passes.
    pub fn wait_for(&mut self, timeout: Duration, mut pred: impl FnMut(&Msg) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.recv(left) {
                Ok(Some(m)) => {
                    let hit = pred(&m);
                    match m {
                        Msg::Frame(f) => {
                            let _ = self.apply(&f);
                            if self.auto_ack {
                                let _ = self.ack(f.frame_seq);
                            }
                            let mut kept = f;
                            kept.tiles.iter_mut().for_each(|t| t.data = Vec::new());
                            self.frames.push(kept);
                        }
                        Msg::State(s) => self.state = Some(s),
                        other => self.inbox.push(other),
                    }
                    if hit {
                        return true;
                    }
                }
                Ok(None) | Err(_) => return false,
            }
        }
    }

    /// Reads and applies everything that arrives within `timeout`.
    pub fn drain(&mut self, timeout: Duration) {
        self.wait_for(timeout, |_| false);
    }

    /// The highest `last_input_seq` of the frames received.
    pub fn confirmed_seq(&self) -> u32 {
        self.frames.iter().map(|f| f.last_input_seq).max().unwrap_or(0)
    }

    /// Sends BYE and closes.
    pub fn bye(mut self) {
        let _ = self.send(&Msg::ByeC { reason: 0 });
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    /// Closes without BYE (a pulled cable).
    pub fn drop_connection(self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// A HELLO as the test client sends it.
pub fn test_hello(link: u8, tablet_id: [u8; 16], name: &str) -> proto::Hello {
    proto::Hello {
        proto_min: 1,
        proto_max: 1,
        caps: proto::caps::CLIENT_PNG | proto::caps::CLIENT_TRAIL,
        link,
        tablet_id,
        client_nonce: crypto::random_array(),
        screen_w: 2960,
        screen_h: 1848,
        density_dpi: 340,
        app_version: "test".into(),
        device_name: name.to_owned(),
    }
}
