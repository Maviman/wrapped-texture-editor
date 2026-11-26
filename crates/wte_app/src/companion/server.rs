//! The companion server (COMPANION_PLAN 2.2, protocol 4): listeners, one session at a time, the handshake, the reader
//! and writer threads, and the `Hub` that the UI thread and the session threads share. No thread here touches
//! `AppState`: input goes to the UI as `HubEvent`s, pictures come from the UI as `Captured` frames.
//!
//! What is opened is decided by the pure `bind_plan` (decision C13): a self-test run binds 127.0.0.1 only, never UDP,
//! Bluetooth or adb (except the adb serial named by `WTE_COMPANION_ADB_SERIAL`), whatever the preferences say.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;

use super::auth::{self, AuthOutcome, Handshake, PairedTablet, PairingWindow, Peer, Policy};
use super::crypto;
use super::encode::{Encoder, RateController, apply_config, profile};
use super::proto::{self, ErrorMsg, FrameReader, Msg, err};
use crate::state::CompanionPrefs;

/// What the companion opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bind {
    /// A TCP session listener (127.0.0.1 for USB; 0.0.0.0 for Wi-Fi, which also takes loopback connections).
    Tcp(SocketAddr),
    /// The UDP discovery responder (Wi-Fi only).
    Udp(SocketAddr),
    /// The Bluetooth RFCOMM listener (Windows; never started by agents, C14).
    Rfcomm,
    /// The adb watcher (USB): every device, or only this serial (`WTE_COMPANION_ADB_SERIAL` in a self-test run).
    Adb { only_serial: Option<String> },
}

/// What to open for these preferences (decision C13, unit-tested): a self-test run opens only `127.0.0.1:port` (plus the
/// adb watcher for the one serial named by `adb_serial`); otherwise USB = loopback TCP + adb, Wi-Fi = TCP on every
/// address + UDP discovery, Bluetooth = RFCOMM. Nothing is opened while every link is off.
pub fn bind_plan(selftest: bool, prefs: &CompanionPrefs, port: u16, adb_serial: Option<&str>) -> Vec<Bind> {
    let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    if selftest {
        let mut plan = vec![Bind::Tcp(loopback)];
        if let Some(serial) = adb_serial.filter(|s| !s.trim().is_empty()) {
            plan.push(Bind::Adb { only_serial: Some(serial.trim().to_owned()) });
        }
        return plan;
    }
    let mut plan = Vec::new();
    if prefs.wifi {
        plan.push(Bind::Tcp(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)));
        plan.push(Bind::Udp(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), super::DISCOVERY_PORT)));
    } else if prefs.usb {
        plan.push(Bind::Tcp(loopback));
    }
    if prefs.usb {
        plan.push(Bind::Adb { only_serial: None });
    }
    if prefs.bluetooth {
        plan.push(Bind::Rfcomm);
    }
    plan
}

/// A byte stream to a tablet: TCP or RFCOMM.
pub trait Link: Read + Write + Send {
    fn try_clone_link(&self) -> io::Result<Box<dyn Link>>;
    /// Closes both directions (wakes a blocked reader).
    fn shutdown(&self);
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    /// A write that makes no progress for this long fails (a tablet that stopped reading ends its session).
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    /// The peer is on this machine (USB through `adb reverse`).
    fn peer_loopback(&self) -> bool;
    /// The connection came in on the RFCOMM listener.
    fn is_rfcomm(&self) -> bool {
        false
    }
}

impl Link for TcpStream {
    fn try_clone_link(&self) -> io::Result<Box<dyn Link>> {
        Ok(Box::new(self.try_clone()?))
    }
    fn shutdown(&self) {
        let _ = TcpStream::shutdown(self, std::net::Shutdown::Both);
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }
    fn peer_loopback(&self) -> bool {
        self.peer_addr().is_ok_and(|a| a.ip().is_loopback())
    }
}

/// A picture captured on the UI thread (`capture.rs`), waiting in the mailbox for the writer.
#[derive(Clone)]
pub struct Captured {
    /// The session the UI thought was connected when it asked for the picture; the writer of another session drops
    /// it (its `last_input_seq` belongs to that session).
    pub session: u64,
    pub image: Arc<egui::ColorImage>,
    /// The last input message the UI had injected when the captured frame ran.
    pub last_input_seq: u32,
    pub pixels_per_point: f32,
    /// Milliseconds from the capture request to the pixels.
    pub capture_ms: u32,
}

/// Messages from the session threads to the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum HubEvent {
    /// A session is authenticated and streaming.
    Started { session: u64, tablet_id: [u8; 16], name: String, link: u8, caps: u64, screen: [u16; 2] },
    Config { session: u64, config: proto::Config },
    /// PEN, TOUCH, KEY, TEXT, MODS, ACTION or FIT_REQUEST, in arrival order.
    Input { session: u64, msg: Msg },
    Ended { session: u64, reason: String },
    /// A tablet paired (code or confirm mode): store it in the preferences.
    Paired(PairedTablet),
    /// An unknown tablet asks in confirm mode: show Allow / Deny (`Hub::answer_confirm`).
    ConfirmRequest { id: u64, name: String, link: u8 },
    /// A handshake failed (for the log, the dialog and the tests).
    Refused { name: String, code: u16, text: String },
}

/// Server settings that do not change while it runs.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub desktop_name: String,
    pub app_version: String,
    pub iters: u32,
    /// No byte for this long closes a session (4.6); RFCOMM doubles it.
    pub idle_timeout: Duration,
    /// Confirm requests are allowed at once (self-test scenarios only).
    pub auto_allow: bool,
    pub confirm_timeout: Duration,
    /// The magic and HELLO must arrive within this long (the whole handshake within `HANDSHAKE_TIMEOUT`).
    pub hello_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig {
            desktop_name: desktop_name(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            iters: auth::PBKDF2_ITERS,
            idle_timeout: Duration::from_secs(5),
            auto_allow: false,
            confirm_timeout: auth::CONFIRM_TIMEOUT,
            hello_timeout: Duration::from_secs(4),
        }
    }
}

/// This PC's name (`COMPUTERNAME`), shown on the tablet.
pub fn desktop_name() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Desktop".to_owned())
}

/// The handshake state shared by the sessions and the dialog.
pub struct AuthState {
    pub desktop_id: [u8; 16],
    pub window: PairingWindow,
    pub paired: Vec<PairedTablet>,
    /// The confirm request waiting for Allow / Deny: (id, name, answer).
    pub confirm: Option<(u64, String, Option<bool>)>,
}

/// The connected session as the UI sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionInfo {
    pub id: u64,
    pub tablet_id: [u8; 16],
    pub name: String,
    pub link: u8,
    pub caps: u64,
}

struct Current {
    info: SessionInfo,
    link: Box<dyn Link>,
}

/// The mailbox and the desktop state the writer sends.
struct Slot {
    captured: Option<Captured>,
    state: proto::State,
    state_rev: u64,
    cursor: Option<proto::Cursor>,
    cursor_rev: u64,
}

/// Per-session values the reader hands the writer.
#[derive(Default)]
struct Feedback {
    acks: Vec<(u32, Instant)>,
    config: Option<proto::Config>,
    /// The newest PING's `t`: the writer answers it (the reader never writes, so a tablet that stops reading cannot
    /// block the reader's idle check, protocol 4.6).
    pong: Option<u32>,
}

/// State shared by the UI thread and the server threads.
pub struct Hub {
    pub cfg: ServerConfig,
    pub auth: Mutex<AuthState>,
    current: Mutex<Option<Current>>,
    slot: Mutex<Slot>,
    cond: Condvar,
    feedback: Mutex<Feedback>,
    /// The writer wants a new picture (fewer than `window` frames unacknowledged).
    want_frame: AtomicBool,
    /// The writer's current frame-rate cap.
    fps: AtomicU32,
    /// The next frame is a KEY frame (the window came back from minimized).
    force_key: AtomicBool,
    stop: AtomicBool,
    next_id: AtomicU64,
    events: Mutex<Sender<HubEvent>>,
    wake: Box<dyn Fn() + Send + Sync>,
    /// Frames sent and bytes sent (for the log and the tests).
    pub frames_sent: AtomicU64,
    pub bytes_sent: AtomicU64,
    /// Connections between accept and the end of their handshake (at most `MAX_HANDSHAKES`).
    handshakes: AtomicUsize,
    /// Captures in a row that changed nothing (the capture scheduler slows down, `capture::interval`).
    empty_captures: AtomicU32,
}

/// Connections allowed in their handshake at once; more are closed at once (no thread is started for them).
pub const MAX_HANDSHAKES: usize = 8;

/// A connection's place among the `MAX_HANDSHAKES`; given back when dropped.
pub struct Admission(Arc<Hub>);

impl Drop for Admission {
    fn drop(&mut self) {
        self.0.handshakes.fetch_sub(1, Ordering::SeqCst);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hub {
    /// A hub and the receiver of its events; `wake` is called after every event (`ctx.request_repaint`).
    pub fn new(cfg: ServerConfig, desktop_id: [u8; 16], window: PairingWindow, paired: Vec<PairedTablet>, wake: Box<dyn Fn() + Send + Sync>) -> (Arc<Hub>, Receiver<HubEvent>) {
        let (tx, rx) = channel();
        let hub = Hub {
            cfg,
            auth: Mutex::new(AuthState { desktop_id, window, paired, confirm: None }),
            current: Mutex::new(None),
            slot: Mutex::new(Slot {
                captured: None,
                state: proto::State { surface_w: 0, surface_h: 0, pixels_per_point: 1.0, flags: 0, trail_rgba: 0xff20_40d0, title: String::new() },
                state_rev: 0,
                cursor: None,
                cursor_rev: 0,
            }),
            cond: Condvar::new(),
            feedback: Mutex::new(Feedback::default()),
            want_frame: AtomicBool::new(false),
            fps: AtomicU32::new(30),
            force_key: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            events: Mutex::new(tx),
            wake,
            frames_sent: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            handshakes: AtomicUsize::new(0),
            empty_captures: AtomicU32::new(0),
        };
        (Arc::new(hub), rx)
    }

    fn emit(&self, e: HubEvent) {
        let _ = lock(&self.events).send(e);
        (self.wake)();
    }

    /// The connected session, if any.
    pub fn session(&self) -> Option<SessionInfo> {
        lock(&self.current).as_ref().map(|c| c.info.clone())
    }

    /// The writer wants a picture.
    pub fn want_frame(&self) -> bool {
        self.want_frame.load(Ordering::SeqCst)
    }

    pub fn fps(&self) -> u32 {
        self.fps.load(Ordering::SeqCst).max(1)
    }

    /// A place for a new connection's handshake, or `None` when `MAX_HANDSHAKES` are running (close it).
    pub fn admit(self: &Arc<Self>) -> Option<Admission> {
        if self.handshakes.fetch_add(1, Ordering::SeqCst) >= MAX_HANDSHAKES {
            self.handshakes.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Admission(Arc::clone(self)))
    }

    /// Connections in their handshake.
    #[cfg(test)]
    pub fn handshakes(&self) -> usize {
        self.handshakes.load(Ordering::SeqCst)
    }

    /// Captures in a row that changed nothing on the tablet (0 after a frame was sent or a session started).
    pub fn empty_captures(&self) -> u32 {
        self.empty_captures.load(Ordering::SeqCst)
    }

    /// The next frame is sent as a KEY frame (all cells).
    pub fn request_key(&self) {
        self.force_key.store(true, Ordering::SeqCst);
        self.cond.notify_all();
    }

    /// A captured picture for the writer (replaces one that was not taken yet).
    pub fn push_frame(&self, c: Captured) {
        lock(&self.slot).captured = Some(c);
        self.cond.notify_all();
    }

    /// The desktop state for STATE (sent when it changed).
    pub fn set_state(&self, state: proto::State) {
        let mut s = lock(&self.slot);
        if s.state != state {
            s.state = state;
            s.state_rev += 1;
            drop(s);
            self.cond.notify_all();
        }
    }

    /// The desktop's own pointer for CURSOR.
    pub fn set_cursor(&self, cursor: Option<proto::Cursor>) {
        let mut s = lock(&self.slot);
        if s.cursor != cursor {
            s.cursor = cursor;
            s.cursor_rev += 1;
            drop(s);
            self.cond.notify_all();
        }
    }

    /// Answers the confirm request `id` (the dialog's Allow / Deny).
    pub fn answer_confirm(&self, id: u64, allow: bool) {
        if let Some((cid, _, answer)) = lock(&self.auth).confirm.as_mut()
            && *cid == id
        {
            *answer = Some(allow);
        }
    }

    /// The confirm request waiting for an answer.
    pub fn pending_confirm(&self) -> Option<(u64, String)> {
        lock(&self.auth).confirm.as_ref().filter(|c| c.2.is_none()).map(|c| (c.0, c.1.clone()))
    }

    /// Closes the session (Disconnect, a link turned off, the server stopping).
    pub fn disconnect(&self) {
        if let Some(c) = lock(&self.current).take() {
            c.link.shutdown();
            self.emit(HubEvent::Ended { session: c.info.id, reason: "disconnected on the desktop".into() });
        }
        self.cond.notify_all();
    }

    fn is_current(&self, id: u64) -> bool {
        lock(&self.current).as_ref().is_some_and(|c| c.info.id == id)
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// The running listeners. Dropping it does not stop them; call `stop`.
pub struct Server {
    pub hub: Arc<Hub>,
    threads: Vec<JoinHandle<()>>,
    /// The bound TCP addresses (an ephemeral port resolved).
    pub tcp_addrs: Vec<SocketAddr>,
    pub udp_addr: Option<SocketAddr>,
    /// Binds that failed (shown in the dialog).
    pub errors: Vec<String>,
}

impl Server {
    /// Opens `binds` (TCP and UDP here; RFCOMM and adb are started by their own modules through `spawn_link_listener`
    /// and `adb::watch`). A bind that fails is reported in `errors`, the others still run.
    pub fn start(hub: Arc<Hub>, binds: &[Bind]) -> Server {
        let mut s = Server { hub, threads: Vec::new(), tcp_addrs: Vec::new(), udp_addr: None, errors: Vec::new() };
        for b in binds {
            match b {
                Bind::Tcp(addr) => match TcpListener::bind(addr).and_then(|l| l.set_nonblocking(true).map(|_| l)) {
                    Ok(listener) => {
                        let local = listener.local_addr().unwrap_or(*addr);
                        log::info!("companion: listening on TCP {local}");
                        s.tcp_addrs.push(local);
                        let hub = Arc::clone(&s.hub);
                        s.threads.push(spawn_named("wte-companion-tcp", move || tcp_accept_loop(listener, hub)));
                    }
                    Err(e) => s.errors.push(format!("cannot listen on TCP {addr}: {e}")),
                },
                Bind::Udp(addr) => match super::discovery::Responder::bind(*addr) {
                    Ok(r) => {
                        s.udp_addr = r.local_addr();
                        let hub = Arc::clone(&s.hub);
                        let tcp_port = s.tcp_addrs.first().map_or(super::DEFAULT_PORT, |a| a.port());
                        s.threads.push(spawn_named("wte-companion-udp", move || r.run(&hub, tcp_port)));
                    }
                    Err(e) => s.errors.push(format!("cannot listen on UDP {addr}: {e}")),
                },
                Bind::Rfcomm | Bind::Adb { .. } => {}
            }
        }
        s
    }

    /// Adds a thread that belongs to this server (the RFCOMM accept loop, the adb watcher).
    pub fn adopt(&mut self, t: JoinHandle<()>) {
        self.threads.push(t);
    }

    /// Stops every listener and the session; joins the listener threads.
    pub fn stop(self) {
        self.hub.stop.store(true, Ordering::SeqCst);
        self.hub.disconnect();
        self.hub.cond.notify_all();
        for t in self.threads {
            let _ = t.join();
        }
    }
}

/// A named thread.
pub fn spawn_named(name: &str, f: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new().name(name.to_owned()).spawn(f).expect("spawn a companion thread")
}

/// Starts the session thread of an accepted connection (TCP or RFCOMM): closes it at once when `MAX_HANDSHAKES` are
/// running or no thread can be started (the accept loop never panics).
pub fn spawn_session(link: Box<dyn Link>, hub: &Arc<Hub>) {
    let Some(admission) = hub.admit() else {
        log::warn!("companion: {MAX_HANDSHAKES} connections are in their handshake; closed a new one");
        link.shutdown();
        return;
    };
    let hub = Arc::clone(hub);
    if let Err(e) = std::thread::Builder::new().name("wte-companion-session".to_owned()).spawn(move || run_session(link, hub, admission)) {
        log::warn!("companion: cannot start a session thread ({e}); connection closed");
    }
}

/// The accept loop: non-blocking accept with a 50 ms poll and the stop flag.
fn tcp_accept_loop(listener: TcpListener, hub: Arc<Hub>) {
    while !hub.is_stopped() {
        match listener.accept() {
            Ok((stream, peer)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_nodelay(true);
                log::info!("companion: connection from {peer}");
                spawn_session(Box::new(stream), &hub);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                log::warn!("companion: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

/// Reads one message with a deadline (the handshake).
fn read_msg(link: &mut dyn Link, reader: &mut FrameReader, deadline: Instant, hub: &Hub) -> io::Result<Msg> {
    let mut buf = [0u8; 4096];
    loop {
        match reader.next_msg() {
            Ok(Some(m)) => return Ok(m),
            Ok(None) => {}
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        }
        if Instant::now() >= deadline || hub.is_stopped() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "handshake timeout"));
        }
        match link.read(&mut buf) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed")),
            Ok(n) => reader.push(&buf[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
    }
}

fn send(link: &Mutex<Box<dyn Link>>, msg: &Msg) -> io::Result<usize> {
    let bytes = msg.encode();
    let mut l = lock(link);
    l.write_all(&bytes)?;
    l.flush()?;
    Ok(bytes.len())
}

/// `send` unless the writer holds the link (the reader's last ERROR: it must not wait behind a stalled write).
fn try_send(link: &Mutex<Box<dyn Link>>, msg: &Msg) {
    if let Ok(mut l) = link.try_lock() {
        let _ = l.write_all(&msg.encode());
        let _ = l.flush();
    }
}

/// How long the handshake may take (the confirm wait has its own limit).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// The reader's socket timeout (how often it looks at the idle time and the stop flag).
const READ_POLL: Duration = Duration::from_millis(100);

/// One connection: magic, HELLO, AUTH, then the reader loop; the writer runs on its own thread. `admission` is held
/// until the handshake ends.
pub fn run_session(mut link: Box<dyn Link>, hub: Arc<Hub>, admission: Admission) {
    let _ = link.set_read_timeout(Some(READ_POLL));
    // A tablet that stops reading makes writes fail after the idle time (4.6), not block forever.
    let write_limit = if link.is_rfcomm() { hub.cfg.idle_timeout * 2 } else { hub.cfg.idle_timeout };
    let _ = link.set_write_timeout(Some(write_limit));
    let start = Instant::now();
    let deadline = start + HANDSHAKE_TIMEOUT;
    let hello_deadline = start + hub.cfg.hello_timeout.min(HANDSHAKE_TIMEOUT);
    // Magic.
    let mut magic = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        if Instant::now() >= hello_deadline || hub.is_stopped() {
            return;
        }
        match link.read(&mut magic[got..]) {
            Ok(0) => return,
            Ok(n) => got += n,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
    if magic != proto::MAGIC {
        log::warn!("companion: a peer sent no WTEC magic; closed");
        link.shutdown();
        return;
    }
    if link.write_all(&proto::MAGIC).is_err() {
        return;
    }
    let peer = Peer { loopback: link.peer_loopback(), rfcomm: link.is_rfcomm() };
    let mut reader = FrameReader::new(proto::MAX_C2S);
    let refuse = |link: &mut Box<dyn Link>, name: &str, e: ErrorMsg| {
        log::info!("companion: refused {name}: {} ({})", e.text, e.code);
        let _ = link.write_all(&Msg::Error(e.clone()).encode());
        let _ = link.flush();
        hub.emit(HubEvent::Refused { name: name.to_owned(), code: e.code, text: e.text });
        // Let the peer read the error before the close.
        std::thread::sleep(Duration::from_millis(50));
        link.shutdown();
    };
    let hello = match read_msg(link.as_mut(), &mut reader, hello_deadline, &hub) {
        Ok(Msg::Hello(h)) => h,
        Ok(_) => return refuse(&mut link, "?", ErrorMsg { code: err::BAD_MESSAGE, text: "HELLO expected".into() }),
        Err(_) => return link.shutdown(),
    };
    let name = auth::clean_name(&hello.device_name);
    // One tablet at a time (C6): another tablet gets BUSY; the same one replaces its old session after AUTH.
    if let Some(cur) = hub.session()
        && cur.tablet_id != hello.tablet_id
    {
        return refuse(&mut link, &name, ErrorMsg { code: err::BUSY, text: format!("{} is connected", cur.name) });
    }
    let server_nonce: [u8; 16] = crypto::random_array();
    let started = {
        let mut a = lock(&hub.auth);
        let now = Instant::now();
        let code = a.window.code(now);
        let locked = a.window.locked(now);
        let open = a.window.is_open();
        let policy = Policy { desktop_id: a.desktop_id, paired: &a.paired, code, pairing_open: open, locked, iters: hub.cfg.iters, auto_allow: hub.cfg.auto_allow };
        Handshake::start(hello.clone(), peer, &policy, server_nonce, &hub.cfg.app_version, &hub.cfg.desktop_name)
    };
    let (hs, ack) = match started {
        Ok(v) => v,
        Err(e) => return refuse(&mut link, &name, e),
    };
    if link.write_all(&Msg::HelloAck(ack).encode()).is_err() {
        return;
    }
    let auth_msg = match read_msg(link.as_mut(), &mut reader, deadline, &hub) {
        Ok(Msg::Auth(a)) => a,
        Ok(_) => return refuse(&mut link, &name, ErrorMsg { code: err::BAD_MESSAGE, text: "AUTH expected".into() }),
        Err(_) => return link.shutdown(),
    };
    // The check runs without the lock (a code attempt is a PBKDF2 that the UI thread must not wait for); a code
    // attempt reserves its place among the allowed failures first, so concurrent attempts cannot exceed them.
    let code_attempt = auth_msg.mode == proto::auth_mode::CODE;
    let (desktop_id, paired, code, open, locked, reserved) = {
        let mut a = lock(&hub.auth);
        let now = Instant::now();
        let code = a.window.code(now);
        let mut locked = a.window.locked(now);
        let mut reserved = false;
        if code_attempt && !locked && code.is_some() {
            reserved = a.window.begin_attempt();
            locked = !reserved;
        }
        (a.desktop_id, a.paired.clone(), code, a.window.is_open(), locked, reserved)
    };
    let policy = Policy { desktop_id, paired: &paired, code, pairing_open: open, locked, iters: hub.cfg.iters, auto_allow: hub.cfg.auto_allow };
    let outcome = hs.check(&auth_msg, &policy);
    if reserved {
        let failed = matches!(outcome, AuthOutcome::Fail { code_failure: true, .. });
        lock(&hub.auth).window.end_attempt(Instant::now(), failed);
    }
    let (ok, token, new_pairing) = match outcome {
        AuthOutcome::Ok { ack, token, new_pairing } => (ack, token, new_pairing),
        AuthOutcome::Fail { error, .. } => return refuse(&mut link, &name, error),
        AuthOutcome::NeedConfirm => match confirm(&mut link, &hub, &name, hello.link) {
            Ok(true) => {
                let token: [u8; 32] = crypto::random_array();
                (hs.confirmed(token), token, true)
            }
            Ok(false) => return refuse(&mut link, &name, ErrorMsg { code: err::DENIED, text: "the desktop user said no".into() }),
            Err(_) => return link.shutdown(),
        },
    };
    if new_pairing {
        let record = PairedTablet::new(&hello.tablet_id, &name, &token, auth::today());
        {
            let mut a = lock(&hub.auth);
            a.paired.retain(|p| p.id != record.id);
            a.paired.push(record.clone());
        }
        hub.emit(HubEvent::Paired(record));
    }
    if link.write_all(&Msg::AuthOk(ok).encode()).is_err() {
        return;
    }
    drop(admission);
    // Streaming: this session replaces any other (the same tablet reconnecting).
    let id = hub.next_id.fetch_add(1, Ordering::SeqCst);
    let info = SessionInfo { id, tablet_id: hello.tablet_id, name: name.clone(), link: hello.link, caps: hello.caps };
    let Ok(kill) = link.try_clone_link() else { return };
    let Ok(write_half) = link.try_clone_link() else { return };
    let _ = write_half.set_write_timeout(Some(write_limit));
    {
        let mut cur = lock(&hub.current);
        if let Some(old) = cur.take() {
            old.link.shutdown();
            hub.emit(HubEvent::Ended { session: old.info.id, reason: "replaced by a new connection".into() });
        }
        *cur = Some(Current { info, link: kill });
    }
    *lock(&hub.feedback) = Feedback::default();
    // A picture captured for the previous session never reaches this one (its input seq belongs to that session).
    lock(&hub.slot).captured = None;
    hub.empty_captures.store(0, Ordering::SeqCst);
    log::info!("companion: session {id} with {name} (link {}) started", hello.link);
    hub.emit(HubEvent::Started {
        session: id,
        tablet_id: hello.tablet_id,
        name: name.clone(),
        link: hello.link,
        caps: hello.caps,
        screen: [hello.screen_w, hello.screen_h],
    });
    let writer_link = Arc::new(Mutex::new(write_half));
    let w_hub = Arc::clone(&hub);
    let w_link = Arc::clone(&writer_link);
    let link_kind = hello.link;
    let writer_failed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let w_failed = Arc::clone(&writer_failed);
    let writer = spawn_named("wte-companion-writer", move || {
        if let Err(e) = run_writer(&w_hub, id, link_kind, &w_link) {
            *lock(&w_failed) = Some(format!("sending to the tablet failed ({e}): it stopped reading or the link broke"));
        }
        // The session ends with the writer: this wakes the reader.
        lock(&w_link).shutdown();
    });
    let reason = run_reader(link.as_mut(), &mut reader, &hub, id, &writer_link, link_kind);
    // End of the session, for any reason.
    let was_current = {
        let mut cur = lock(&hub.current);
        let mine = cur.as_ref().is_some_and(|c| c.info.id == id);
        if mine {
            *cur = None;
        }
        mine
    };
    link.shutdown();
    hub.cond.notify_all();
    let _ = writer.join();
    let reason = lock(&writer_failed).take().unwrap_or(reason);
    if was_current {
        hub.want_frame.store(false, Ordering::SeqCst);
        lock(&hub.slot).captured = None;
        log::info!("companion: session {id} ended: {reason}");
        hub.emit(HubEvent::Ended { session: id, reason });
    }
}

/// Confirm mode: asks the desktop user, sending AUTH_WAIT every second; `Ok(true)` = Allow.
fn confirm(link: &mut Box<dyn Link>, hub: &Hub, name: &str, link_kind: u8) -> io::Result<bool> {
    if hub.cfg.auto_allow {
        return Ok(true);
    }
    let id = hub.next_id.fetch_add(1, Ordering::SeqCst);
    lock(&hub.auth).confirm = Some((id, name.to_owned(), None));
    hub.emit(HubEvent::ConfirmRequest { id, name: name.to_owned(), link: link_kind });
    let start = Instant::now();
    let mut last_wait = start;
    let result = loop {
        std::thread::sleep(Duration::from_millis(50));
        let (answer, open) = {
            let a = lock(&hub.auth);
            (a.confirm.as_ref().filter(|c| c.0 == id).and_then(|c| c.2), a.window.is_open())
        };
        if let Some(allow) = answer {
            break Ok(allow);
        }
        if !open || hub.is_stopped() || start.elapsed() >= hub.cfg.confirm_timeout {
            break Ok(false);
        }
        if last_wait.elapsed() >= Duration::from_secs(1) {
            last_wait = Instant::now();
            if let Err(e) = link.write_all(&Msg::AuthWait.encode()) {
                break Err(e);
            }
        }
    };
    let mut a = lock(&hub.auth);
    if a.confirm.as_ref().is_some_and(|c| c.0 == id) {
        a.confirm = None;
    }
    result
}

/// The reader loop of a streaming session; returns why it ended.
fn run_reader(link: &mut dyn Link, reader: &mut FrameReader, hub: &Hub, id: u64, out: &Mutex<Box<dyn Link>>, link_kind: u8) -> String {
    let idle = if link_kind == proto::link::BLUETOOTH { hub.cfg.idle_timeout * 2 } else { hub.cfg.idle_timeout };
    let mut last_rx = Instant::now();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        loop {
            let msg = match reader.next_msg() {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(e) => {
                    try_send(out, &Msg::Error(ErrorMsg { code: err::BAD_MESSAGE, text: e.to_string() }));
                    return format!("bad message: {e}");
                }
            };
            match msg {
                Msg::Ping { t } => {
                    lock(&hub.feedback).pong = Some(t);
                    hub.cond.notify_all();
                }
                Msg::ByeC { reason } => return format!("the tablet said bye ({reason})"),
                Msg::FrameAck(a) => {
                    lock(&hub.feedback).acks.push((a.frame_seq, Instant::now()));
                    hub.cond.notify_all();
                }
                Msg::Config(c) => {
                    lock(&hub.feedback).config = Some(c.clone());
                    hub.cond.notify_all();
                    hub.emit(HubEvent::Config { session: id, config: c });
                }
                m @ (Msg::Pen(_) | Msg::Touch(_) | Msg::Key(_) | Msg::Text { .. } | Msg::Mods { .. } | Msg::Action { .. } | Msg::FitRequest(_)) => {
                    hub.emit(HubEvent::Input { session: id, msg: m });
                }
                // Unknown types and server-to-client types are skipped (4.2).
                _ => {}
            }
        }
        if hub.is_stopped() {
            return "the server stopped".into();
        }
        if !hub.is_current(id) {
            return "closed on the desktop".into();
        }
        if last_rx.elapsed() > idle {
            return format!("nothing received for {} s", idle.as_secs());
        }
        match link.read(&mut buf) {
            Ok(0) => return "the tablet closed the connection".into(),
            Ok(n) => {
                last_rx = Instant::now();
                reader.push(&buf[..n]);
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return format!("read failed: {e}"),
        }
    }
}

/// The writer: PONG, STATE on change, FRAME when a picture waits and the ack window is open, REFINE when idle, CURSOR
/// throttled to 30 per second. Returns the send error that ended it (a write blocked longer than the write timeout:
/// the tablet stopped reading), `Ok` when the session ended otherwise.
fn run_writer(hub: &Hub, id: u64, link_kind: u8, out: &Mutex<Box<dyn Link>>) -> io::Result<()> {
    let result = writer_loop(hub, id, link_kind, out);
    hub.want_frame.store(false, Ordering::SeqCst);
    result
}

fn writer_loop(hub: &Hub, id: u64, link_kind: u8, out: &Mutex<Box<dyn Link>>) -> io::Result<()> {
    let mut enc = Encoder::new();
    let base = profile(link_kind);
    let mut rc = RateController::new(base);
    let mut unacked: VecDeque<(u32, Instant)> = VecDeque::new();
    let mut sent_state: Option<proto::State> = None;
    let mut sent_cursor_rev = 0u64;
    let mut last_cursor = Instant::now() - Duration::from_secs(1);
    let mut surface: Option<[u16; 2]> = None;
    let mut ppp = 1.0f32;
    let mut was_open = true;
    loop {
        if hub.is_stopped() || !hub.is_current(id) {
            return Ok(());
        }
        // Acks, the CONFIG and the newest PING from the reader.
        let fb = std::mem::take(&mut *lock(&hub.feedback));
        if let Some(t) = fb.pong {
            send(out, &Msg::Pong { t })?;
        }
        if let Some(c) = fb.config {
            rc.set_ceiling(apply_config(base, c.quality_pref, c.max_fps));
        }
        if hub.force_key.swap(false, Ordering::SeqCst) {
            enc.need_key = true;
        }
        for (seq, at) in fb.acks {
            while let Some(&(s, sent)) = unacked.front() {
                if s > seq {
                    break;
                }
                unacked.pop_front();
                if s == seq && rc.on_ack(at.saturating_duration_since(sent), at) {
                    enc.need_key = true;
                }
            }
        }
        hub.fps.store(rc.fps(), Ordering::SeqCst);
        let window = rc.ceiling().window.max(1) as usize;
        let open = unacked.len() < window;
        hub.want_frame.store(open, Ordering::SeqCst);
        if open && !was_open {
            // The ack window opened again: the UI may hold a picture it could not hand over (capture.rs).
            (hub.wake)();
        }
        was_open = open;
        let params = rc.params();
        // What waits.
        let (captured, state, cursor) = {
            let mut s = lock(&hub.slot);
            // A picture asked for while another session was connected is dropped (its input seq is not ours).
            let captured = if open { s.captured.take().filter(|c| c.session == id) } else { None };
            let cursor = (s.cursor_rev != sent_cursor_rev && last_cursor.elapsed() >= Duration::from_millis(33)).then(|| {
                sent_cursor_rev = s.cursor_rev;
                s.cursor.clone()
            });
            (captured, s.state.clone(), cursor)
        };
        let now = Instant::now();
        let mut sent_any = false;
        if let Some(c) = &captured {
            surface = Some([c.image.size[0] as u16, c.image.size[1] as u16]);
            ppp = c.pixels_per_point;
        }
        // STATE first when it (or the surface it describes) changed.
        let mut st = state;
        if let Some([w, h]) = surface {
            (st.surface_w, st.surface_h, st.pixels_per_point) = (w, h, ppp);
        }
        if sent_state.as_ref() != Some(&st) && (st.surface_w > 0 || surface.is_some()) {
            send(out, &Msg::State(st.clone()))?;
            sent_state = Some(st);
            sent_any = true;
        }
        if let Some(c) = captured {
            let t0 = Instant::now();
            let frame = enc.frame(&c.image, params, c.last_input_seq, c.capture_ms, now);
            if frame.is_some() {
                hub.empty_captures.store(0, Ordering::SeqCst);
            } else {
                hub.empty_captures.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(frame) = frame {
                let tiles = frame.tiles.len();
                let seq = frame.frame_seq;
                let key = frame.flags & proto::frame_flag::KEY != 0;
                let encode_ms = t0.elapsed().as_secs_f32() * 1000.0;
                let n = send(out, &Msg::Frame(frame))?;
                unacked.push_back((seq, Instant::now()));
                hub.frames_sent.fetch_add(1, Ordering::SeqCst);
                hub.bytes_sent.fetch_add(n as u64, Ordering::SeqCst);
                log::debug!("companion: frame {seq} ({}{tiles} tiles, {n} bytes, encode {encode_ms:.1} ms)", if key { "key, " } else { "" });
                sent_any = true;
            }
        } else if open && let Some(frame) = enc.refine(params, now) {
            let seq = frame.frame_seq;
            let n = send(out, &Msg::Frame(frame))?;
            unacked.push_back((seq, Instant::now()));
            hub.frames_sent.fetch_add(1, Ordering::SeqCst);
            hub.bytes_sent.fetch_add(n as u64, Ordering::SeqCst);
            sent_any = true;
        }
        if let Some(cursor) = cursor {
            last_cursor = Instant::now();
            let msg = cursor.unwrap_or(proto::Cursor { x: 0.0, y: 0.0, visible: false, icon: 0 });
            send(out, &Msg::Cursor(msg))?;
        }
        if sent_any {
            continue;
        }
        // Wait for a picture, an ack, a state change or the refine time.
        let mut wait = Duration::from_millis(50);
        if let Some(at) = enc.refine_at(params) {
            wait = wait.min(at.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)));
        }
        let s = lock(&hub.slot);
        let _ = hub.cond.wait_timeout(s, wait);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::companion::auth::ClientKey;
    use crate::companion::client::TestClient;
    use crate::companion::proto::{auth_mode, link};

    #[test]
    fn bind_plan_self_test_is_loopback_only_for_every_pref() {
        for bits in 0..8u8 {
            let prefs = CompanionPrefs { usb: bits & 1 != 0, wifi: bits & 2 != 0, bluetooth: bits & 4 != 0, ..CompanionPrefs::default() };
            let plan = bind_plan(true, &prefs, 0, None);
            assert_eq!(plan, vec![Bind::Tcp("127.0.0.1:0".parse().unwrap())], "{bits}");
            let plan = bind_plan(true, &prefs, 47811, Some("emulator-5554"));
            assert_eq!(
                plan,
                vec![Bind::Tcp("127.0.0.1:47811".parse().unwrap()), Bind::Adb { only_serial: Some("emulator-5554".into()) }]
            );
            assert_eq!(bind_plan(true, &prefs, 1, Some("  ")).len(), 1, "an empty serial is no serial");
        }
    }

    #[test]
    fn bind_plan_interactive() {
        let off = CompanionPrefs::default();
        assert!(bind_plan(false, &off, 47800, None).is_empty(), "all off: nothing opened");
        let usb = CompanionPrefs { usb: true, ..off.clone() };
        assert_eq!(bind_plan(false, &usb, 47800, None), vec![Bind::Tcp("127.0.0.1:47800".parse().unwrap()), Bind::Adb { only_serial: None }]);
        let wifi = CompanionPrefs { wifi: true, ..off.clone() };
        assert_eq!(
            bind_plan(false, &wifi, 47800, None),
            vec![Bind::Tcp("0.0.0.0:47800".parse().unwrap()), Bind::Udp("0.0.0.0:47801".parse().unwrap())]
        );
        let all = CompanionPrefs { usb: true, wifi: true, bluetooth: true, ..off.clone() };
        let plan = bind_plan(false, &all, 47800, Some("ignored"));
        assert_eq!(plan.len(), 4);
        assert!(plan.contains(&Bind::Rfcomm) && plan.contains(&Bind::Adb { only_serial: None }));
        // Wi-Fi off: no LAN bind, no UDP, whatever else is on.
        for p in [usb, CompanionPrefs { bluetooth: true, ..off.clone() }, CompanionPrefs { usb: true, bluetooth: true, ..off }] {
            let plan = bind_plan(false, &p, 47800, None);
            assert!(!plan.iter().any(|b| matches!(b, Bind::Udp(_))));
            assert!(!plan.iter().any(|b| matches!(b, Bind::Tcp(a) if !a.ip().is_loopback())));
        }
    }

    /// A hub on 127.0.0.1 with an ephemeral port, its event receiver and the server.
    pub(crate) struct Fixture {
        pub hub: Arc<Hub>,
        pub events: Receiver<HubEvent>,
        pub server: Option<Server>,
        pub addr: SocketAddr,
    }

    impl Fixture {
        pub(crate) fn new(window: PairingWindow, paired: Vec<PairedTablet>, auto_allow: bool) -> Fixture {
            Fixture::with_cfg(window, paired, auto_allow, |_| {})
        }

        /// `new` with changes to the test server config.
        pub(crate) fn with_cfg(window: PairingWindow, paired: Vec<PairedTablet>, auto_allow: bool, change: impl FnOnce(&mut ServerConfig)) -> Fixture {
            let mut cfg = ServerConfig { iters: 1000, auto_allow, idle_timeout: Duration::from_millis(1500), ..ServerConfig::default() };
            change(&mut cfg);
            let (hub, events) = Hub::new(cfg, [0x42; 16], window, paired, Box::new(|| {}));
            let server = Server::start(Arc::clone(&hub), &[Bind::Tcp("127.0.0.1:0".parse().unwrap())]);
            assert!(server.errors.is_empty(), "{:?}", server.errors);
            let addr = server.tcp_addrs[0];
            assert!(addr.ip().is_loopback());
            Fixture { hub, events, server: Some(server), addr }
        }

        /// The next event of a kind within 5 s.
        pub(crate) fn event(&self, pred: impl Fn(&HubEvent) -> bool) -> HubEvent {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.events.recv_timeout(left) {
                    Ok(e) if pred(&e) => return e,
                    Ok(_) => {}
                    Err(_) => panic!("no such event within 5 s"),
                }
            }
        }

        pub(crate) fn image(w: usize, h: usize, rgb: [u8; 3]) -> Arc<egui::ColorImage> {
            Arc::new(egui::ColorImage::new([w, h], vec![egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]); w * h]))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(s) = self.server.take() {
                s.stop();
            }
        }
    }

    fn usb_hello(tablet: u8) -> proto::Hello {
        crate::companion::auth::tests::hello(tablet, link::USB)
    }

    /// The id of the next session that starts.
    fn started_id(f: &Fixture) -> u64 {
        match f.event(|e| matches!(e, HubEvent::Started { .. })) {
            HubEvent::Started { session, .. } => session,
            _ => unreachable!(),
        }
    }

    /// A picture of noise (does not compress: a few of them fill the socket buffers).
    fn noise(w: usize, h: usize, seed: u32) -> Arc<egui::ColorImage> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        let px = (0..w * h)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let [r, g, b, _] = x.to_le_bytes();
                egui::Color32::from_rgb(r, g, b)
            })
            .collect();
        Arc::new(egui::ColorImage::new([w, h], px))
    }

    /// A tablet that keeps the connection, keeps sending (acks, pings) but never reads: the writer's blocked send times
    /// out and the session ends; the reader never waits behind it (checker finding: the session never ended).
    #[test]
    fn a_tablet_that_stops_reading_is_dropped() {
        let f = Fixture::new(PairingWindow::default(), Vec::new(), true);
        let mut c = TestClient::connect(f.addr).unwrap();
        c.handshake(usb_hello(1), ClientKey::Confirm).unwrap();
        let session = started_id(&f);
        let t0 = Instant::now();
        let mut k = 0u32;
        let reason = loop {
            assert!(t0.elapsed() < Duration::from_secs(20), "the stalled session did not end");
            if let Ok(HubEvent::Ended { session: s, reason }) = f.events.try_recv()
                && s == session
            {
                break reason;
            }
            k += 1;
            f.hub.push_frame(Captured { session, image: noise(512, 512, k), last_input_seq: 0, pixels_per_point: 1.0, capture_ms: 1 });
            // Keep the ack window open and the reader busy without reading anything.
            let _ = c.ack(k + 1000);
            let _ = c.send(&Msg::Ping { t: k });
            std::thread::sleep(Duration::from_millis(30));
        };
        assert!(reason.contains("stopped reading"), "{reason}");
        assert!(f.hub.session().is_none());
        // Another tablet can connect now.
        let mut b = TestClient::connect(f.addr).unwrap();
        b.handshake(usb_hello(2), ClientKey::Confirm).unwrap();
    }

    /// Flooding PINGs without reading the PONGs: the reader keeps up (one PONG per wake-up) and the idle check still
    /// ends the session once the tablet goes quiet.
    #[test]
    fn a_ping_flood_without_reading_still_times_out() {
        let f = Fixture::new(PairingWindow::default(), Vec::new(), true);
        let mut c = TestClient::connect(f.addr).unwrap();
        c.handshake(usb_hello(1), ClientKey::Confirm).unwrap();
        let session = started_id(&f);
        for t in 0..20_000u32 {
            c.send(&Msg::Ping { t }).unwrap();
        }
        let quiet = Instant::now();
        let ev = f.event(|e| matches!(e, HubEvent::Ended { session: s, .. } if *s == session));
        assert!(quiet.elapsed() < Duration::from_secs(4), "{ev:?} after {:?}", quiet.elapsed());
    }

    /// A picture captured for an earlier session never reaches the next one (checker finding: it carried the old
    /// session's input seq into the reconnect).
    #[test]
    fn a_capture_of_another_session_is_dropped() {
        let f = Fixture::new(PairingWindow::default(), Vec::new(), true);
        let mut c = TestClient::connect(f.addr).unwrap();
        c.auto_ack = false;
        let token = c.handshake(usb_hello(1), ClientKey::Confirm).unwrap();
        let s1 = started_id(&f);
        for k in 0..2u8 {
            f.hub.push_frame(Captured { session: s1, image: Fixture::image(64, 64, [k * 90, 0, 0]), last_input_seq: 0, pixels_per_point: 1.0, capture_ms: 1 });
            std::thread::sleep(Duration::from_millis(150));
        }
        // The ack window is full: this one waits in the mailbox when the tablet goes.
        f.hub.push_frame(Captured { session: s1, image: Fixture::image(64, 64, [0, 200, 0]), last_input_seq: 500, pixels_per_point: 1.0, capture_ms: 1 });
        std::thread::sleep(Duration::from_millis(100));
        c.drop_connection();
        f.event(|e| matches!(e, HubEvent::Ended { .. }));
        let mut c = TestClient::connect(f.addr).unwrap();
        c.handshake(usb_hello(1), ClientKey::Token(token)).unwrap();
        let s2 = started_id(&f);
        assert!(c.wait_frame(Duration::from_millis(700)).is_none(), "no frame of the old session");
        // A late capture asked for while the old session was current is dropped too.
        f.hub.push_frame(Captured { session: s1, image: Fixture::image(64, 64, [0, 0, 200]), last_input_seq: 600, pixels_per_point: 1.0, capture_ms: 1 });
        assert!(c.wait_frame(Duration::from_millis(500)).is_none(), "no late frame of the old session");
        f.hub.push_frame(Captured { session: s2, image: Fixture::image(64, 64, [9, 9, 9]), last_input_seq: 2, pixels_per_point: 1.0, capture_ms: 1 });
        let key = c.wait_frame(Duration::from_secs(3)).expect("this session's frame");
        assert_eq!(key.flags & proto::frame_flag::KEY, proto::frame_flag::KEY);
        assert_eq!(key.last_input_seq, 2);
        // An unchanged capture counts as empty (the capture scheduler slows down on a run of them).
        assert_eq!(f.hub.empty_captures(), 0);
        f.hub.push_frame(Captured { session: s2, image: Fixture::image(64, 64, [9, 9, 9]), last_input_seq: 2, pixels_per_point: 1.0, capture_ms: 1 });
        let t0 = Instant::now();
        while f.hub.empty_captures() == 0 {
            assert!(t0.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// At most `MAX_HANDSHAKES` connections wait in their handshake; more are closed at once, and a connection that
    /// sends no HELLO is closed after `hello_timeout` (checker finding: one thread per idle connection, unbounded).
    #[test]
    fn idle_connections_are_limited_and_time_out() {
        let f = Fixture::with_cfg(PairingWindow::default(), Vec::new(), true, |c| c.hello_timeout = Duration::from_millis(1500));
        let mut idle = Vec::new();
        for _ in 0..MAX_HANDSHAKES + 4 {
            let s = TcpStream::connect(f.addr).unwrap();
            s.set_read_timeout(Some(Duration::from_millis(800))).unwrap();
            idle.push(s);
        }
        std::thread::sleep(Duration::from_millis(300));
        assert!(f.hub.handshakes() <= MAX_HANDSHAKES, "{}", f.hub.handshakes());
        let mut closed = 0;
        for s in &mut idle {
            let mut b = [0u8; 4];
            match s.read(&mut b) {
                Ok(0) => closed += 1,
                Err(e) if matches!(e.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted) => closed += 1,
                _ => {}
            }
        }
        assert!(closed >= 4, "the connections over the limit were closed at once ({closed})");
        // The rest are closed by the HELLO deadline; then a real tablet gets in.
        let t0 = Instant::now();
        while f.hub.handshakes() > 0 {
            assert!(t0.elapsed() < Duration::from_secs(5), "{} still in their handshake", f.hub.handshakes());
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(idle);
        let mut c = TestClient::connect(f.addr).unwrap();
        c.handshake(usb_hello(1), ClientKey::Confirm).unwrap();
    }

    /// A code attempt's PBKDF2 runs without `hub.auth` held (checker finding: the UI thread waited for it every frame).
    /// 50,000 iterations: still a PBKDF2 of over a second in a debug build (far above the 30 ms limit), but well inside
    /// the 10 s handshake deadline the client's own PBKDF2 must meet. 300,000 took about 5-7 s per side in a debug
    /// build and failed with os error 10053 under load (STICKER_PLAN P0 check).
    #[test]
    fn a_code_attempt_does_not_hold_the_auth_lock() {
        let code = "K7QF3M9T".to_owned();
        let f = Fixture::with_cfg(PairingWindow::fixed(code), Vec::new(), false, |c| c.iters = 50_000);
        let addr = f.addr;
        let t = std::thread::spawn(move || {
            let mut c = TestClient::connect(addr).unwrap();
            c.handshake(crate::companion::auth::tests::hello(9, link::WIFI), ClientKey::Code("K7QF3M9X".into())).unwrap_err()
        });
        let mut worst = Duration::ZERO;
        while !t.is_finished() {
            let t0 = Instant::now();
            drop(f.hub.auth.lock().unwrap());
            worst = worst.max(t0.elapsed());
            std::thread::sleep(Duration::from_millis(2));
        }
        let e = t.join().unwrap();
        assert!(e.contains("code 2"), "{e}");
        assert!(worst < Duration::from_millis(30), "the auth lock was held for {worst:?}");
        f.event(|e| matches!(e, HubEvent::Refused { .. }));
    }

    #[test]
    fn confirm_mode_then_token_mode_and_frames() {
        let f = Fixture::new(PairingWindow::default(), Vec::new(), true);
        let mut c = TestClient::connect(f.addr).unwrap();
        let token = c.handshake(usb_hello(1), ClientKey::Confirm).unwrap();
        assert!(matches!(f.event(|e| matches!(e, HubEvent::Paired(_))), HubEvent::Paired(p) if p.token_bytes() == Some(token)));
        let started = f.event(|e| matches!(e, HubEvent::Started { .. }));
        let HubEvent::Started { session, name, link: l, .. } = started else { unreachable!() };
        assert_eq!((name.as_str(), l), ("Tab", link::USB));
        c.send(&Msg::Config(proto::Config { view_w: 1000, view_h: 600, max_fps: 0, quality_pref: 0, flags: 0 })).unwrap();
        // The writer wants a frame; a KEY frame comes, then only the changed rectangle.
        let t0 = Instant::now();
        while !f.hub.want_frame() {
            assert!(t0.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        f.hub.set_state(proto::State { surface_w: 200, surface_h: 130, pixels_per_point: 1.0, flags: 0, trail_rgba: 0, title: "t".into() });
        f.hub.push_frame(Captured { session, image: Fixture::image(200, 130, [10, 20, 30]), last_input_seq: 0, pixels_per_point: 1.0, capture_ms: 1 });
        let key = c.wait_frame(Duration::from_secs(5)).unwrap();
        assert_eq!(key.flags & proto::frame_flag::KEY, proto::frame_flag::KEY);
        assert_eq!((key.pic_w, key.pic_h), (200, 130));
        assert_eq!(c.state.as_ref().map(|s| (s.surface_w, s.surface_h)), Some((200, 130)), "STATE came before the frame");
        let pic = c.picture.as_ref().unwrap();
        let px = &pic.data[((60 * pic.width + 100) * 4) as usize..][..3];
        assert!(px.iter().zip([10u8, 20, 30]).all(|(a, b)| a.abs_diff(b) <= 6), "{px:?}");
        let mut img = (*Fixture::image(200, 130, [10, 20, 30])).clone();
        img.pixels[70 * 200 + 130] = egui::Color32::from_rgb(255, 0, 0);
        f.hub.push_frame(Captured { session, image: Arc::new(img), last_input_seq: 3, pixels_per_point: 1.0, capture_ms: 1 });
        let diff = c.wait_frame(Duration::from_secs(5)).unwrap();
        assert_eq!(diff.flags, 0);
        assert_eq!(diff.tiles.iter().map(|t| (t.x, t.y, t.w, t.h)).collect::<Vec<_>>(), vec![(128, 64, 64, 64)]);
        assert_eq!(diff.last_input_seq, 3);
        drop(c);
        f.event(|e| matches!(e, HubEvent::Ended { .. }));
        // Token mode on the next connection, without the dialog (the record is in the hub now).
        let mut c = TestClient::connect(f.addr).unwrap();
        let ack = c.hello(usb_hello(1)).unwrap();
        assert_ne!(ack.auth_modes & auth_mode::bit(auth_mode::TOKEN), 0);
        c.auth(&ack, ClientKey::Token(token)).unwrap();
    }

    #[test]
    fn wrong_token_wrong_code_closed_pairing_version_busy() {
        let code = "K7QF3M9T".to_owned();
        let paired = vec![PairedTablet::new(&[7; 16], "Seven", &[7; 32], "2026-11-24".into())];
        let f = Fixture::new(PairingWindow::fixed(code.clone()), paired, false);
        // Wrong token.
        let mut c = TestClient::connect(f.addr).unwrap();
        let e = c.handshake(crate::companion::auth::tests::hello(7, link::WIFI), ClientKey::Token([8; 32])).unwrap_err();
        assert!(e.contains("AUTH_FAILED") || e.contains("code 2"), "{e}");
        // Wrong code; right code pairs.
        let mut c = TestClient::connect(f.addr).unwrap();
        let e = c.handshake(crate::companion::auth::tests::hello(9, link::WIFI), ClientKey::Code("K7QF3M9X".into())).unwrap_err();
        assert!(e.contains("code 2"), "{e}");
        let mut c = TestClient::connect(f.addr).unwrap();
        c.handshake(crate::companion::auth::tests::hello(9, link::WIFI), ClientKey::Code(code.clone())).unwrap();
        // Another tablet while one is connected: BUSY.
        f.event(|e| matches!(e, HubEvent::Started { .. }));
        let mut b = TestClient::connect(f.addr).unwrap();
        let e = b.handshake(crate::companion::auth::tests::hello(10, link::WIFI), ClientKey::Code(code.clone())).unwrap_err();
        assert!(e.contains("code 5"), "{e}");
        drop(c);
        f.event(|e| matches!(e, HubEvent::Ended { .. }));
        // A version this desktop does not speak.
        let mut v = TestClient::connect(f.addr).unwrap();
        let h = proto::Hello { proto_min: 2, proto_max: 3, ..crate::companion::auth::tests::hello(11, link::WIFI) };
        let e = v.hello(h).unwrap_err();
        assert!(e.contains("code 1"), "{e}");
        // A client with unknown caps bits and a longer HELLO is fine.
        let mut x = TestClient::connect(f.addr).unwrap();
        let mut payload = Msg::Hello(proto::Hello { caps: u64::MAX, ..crate::companion::auth::tests::hello(12, link::WIFI) }).payload();
        payload.extend_from_slice(b"fields of a later version");
        let ack = x.hello_raw(&payload).unwrap();
        x.auth(&ack, ClientKey::Code(code)).unwrap();
    }

    #[test]
    fn pairing_closed_without_the_dialog_and_bad_magic() {
        let f = Fixture::new(PairingWindow::default(), Vec::new(), false);
        let mut c = TestClient::connect(f.addr).unwrap();
        let e = c.hello(usb_hello(1)).unwrap_err();
        assert!(e.contains("code 3"), "{e}");
        let ev = f.event(|e| matches!(e, HubEvent::Refused { .. }));
        assert!(matches!(ev, HubEvent::Refused { code: err::PAIRING_CLOSED, .. }));
        // Garbage instead of the magic: closed without an answer.
        let mut s = TcpStream::connect(f.addr).unwrap();
        s.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut b = [0u8; 8];
        assert!(matches!(s.read(&mut b), Ok(0) | Err(_)));
    }

    #[test]
    fn confirm_waits_for_the_desktop_user() {
        let mut window = PairingWindow::default();
        window.open(Instant::now());
        let f = Fixture::new(window, Vec::new(), false);
        let addr = f.addr;
        let t = std::thread::spawn(move || {
            let mut c = TestClient::connect(addr).unwrap();
            c.handshake(usb_hello(1), ClientKey::Confirm).map(|_| c.auth_waits)
        });
        let ev = f.event(|e| matches!(e, HubEvent::ConfirmRequest { .. }));
        let HubEvent::ConfirmRequest { id, name, .. } = ev else { unreachable!() };
        assert_eq!(name, "Tab");
        assert_eq!(f.hub.pending_confirm(), Some((id, "Tab".to_owned())));
        std::thread::sleep(Duration::from_millis(1300));
        f.hub.answer_confirm(id, true);
        let waits = t.join().unwrap().unwrap();
        assert!(waits >= 1, "AUTH_WAIT while waiting");
        // Deny.
        f.hub.disconnect();
        let t = std::thread::spawn(move || TestClient::connect(addr).unwrap().handshake(usb_hello(2), ClientKey::Confirm));
        let HubEvent::ConfirmRequest { id, .. } = f.event(|e| matches!(e, HubEvent::ConfirmRequest { .. })) else { unreachable!() };
        f.hub.answer_confirm(id, false);
        let e = t.join().unwrap().unwrap_err();
        assert!(e.contains("code 4"), "{e}");
    }

    #[test]
    fn the_ack_window_is_respected() {
        let f = Fixture::new(PairingWindow::default(), Vec::new(), true);
        let mut c = TestClient::connect(f.addr).unwrap();
        c.auto_ack = false;
        c.handshake(usb_hello(1), ClientKey::Confirm).unwrap();
        let session = started_id(&f);
        // USB window = 2: without acks, the third frame waits.
        for k in 0..4u8 {
            f.hub.push_frame(Captured { session, image: Fixture::image(64, 64, [k * 50, 0, 0]), last_input_seq: 0, pixels_per_point: 1.0, capture_ms: 1 });
            std::thread::sleep(Duration::from_millis(150));
        }
        let mut frames = 0;
        while c.wait_frame(Duration::from_millis(400)).is_some() {
            frames += 1;
        }
        assert_eq!(frames, 2);
        assert!(!f.hub.want_frame());
        // One ack: the newest waiting picture follows.
        c.ack(1).unwrap();
        let next = c.wait_frame(Duration::from_secs(3)).expect("a frame after the ack");
        assert_eq!(next.frame_seq, 3);
    }

    #[test]
    fn input_arrives_in_order_keepalive_timeout_and_replace() {
        let f = Fixture::new(PairingWindow::default(), Vec::new(), true);
        let mut c = TestClient::connect(f.addr).unwrap();
        let token = c.handshake(usb_hello(1), ClientKey::Confirm).unwrap();
        let first = match f.event(|e| matches!(e, HubEvent::Started { .. })) {
            HubEvent::Started { session, .. } => session,
            _ => unreachable!(),
        };
        let pen = proto::PenSample { kind: proto::pen_kind::DOWN, flags: 0, pressure: 30000, x: 5.0, y: 6.0, tilt: 255, reserved: 0, dt_us: 0 };
        let sent = [
            c.pen(vec![pen]),
            c.touch(vec![proto::TouchPoint { id: 0, phase: 0, x: 1.0, y: 2.0 }]),
            c.key(26, true, proto::mods::CTRL),
            c.text("a"),
            c.mods(proto::mods::SHIFT),
            c.action(proto::action::UNDO),
        ];
        assert_eq!(sent.map(|r| r.unwrap()), [1, 2, 3, 4, 5, 6]);
        let mut seqs = Vec::new();
        while seqs.len() < 6 {
            if let HubEvent::Input { session, msg } = f.event(|e| matches!(e, HubEvent::Input { .. })) {
                assert_eq!(session, first);
                seqs.push(msg.input_seq().unwrap());
            }
        }
        assert_eq!(seqs, [1, 2, 3, 4, 5, 6]);
        // PING -> PONG.
        c.send(&Msg::Ping { t: 77 }).unwrap();
        assert!(c.wait_for(Duration::from_secs(3), |m| matches!(m, Msg::Pong { t: 77 })));
        // The same tablet connects again: the old session ends, the new one starts.
        let mut again = TestClient::connect(f.addr).unwrap();
        again.handshake(usb_hello(1), ClientKey::Token(token)).unwrap();
        assert!(matches!(f.event(|e| matches!(e, HubEvent::Ended { .. })), HubEvent::Ended { session, .. } if session == first));
        f.event(|e| matches!(e, HubEvent::Started { .. }));
        // Keep-alive: silence for longer than the (test) idle timeout ends the session.
        let t0 = Instant::now();
        let ev = f.event(|e| matches!(e, HubEvent::Ended { .. }));
        assert!(matches!(ev, HubEvent::Ended { ref reason, .. } if reason.contains("nothing received")), "{ev:?}");
        assert!(t0.elapsed() < Duration::from_secs(4));
        drop(c);
    }
}
