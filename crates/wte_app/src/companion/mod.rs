//! Tablet companion (branch `companion`, `docs/plans/COMPANION_PLAN.md`, wire spec `docs/protocol/COMPANION_PROTOCOL.md`,
//! ARCHITECTURE 1.11 and 8.14): an in-process server, off by default, that mirrors the app window to the Android app
//! "Tablet Companion" (https://github.com/Maviman/tablet-companion) and feeds its pen, finger and keyboard input into the
//! app in-process.
//!
//! - `crypto.rs`: SHA-256, HMAC, PBKDF2, the pairing code, OS randomness.
//! - `proto.rs`: the wire protocol v1 (messages, framing, key table).
//! - `auth.rs`: the server handshake (versions, auth modes, MACs, the pairing window, paired tablets).
//! - `encode.rs`: RGB, downscale, cell diff, rectangles, JPEG tiles, refine, link profiles, `RateController`.
//! - `server.rs`: `bind_plan`, listeners, sessions (reader / writer threads), the `Hub` shared with the UI thread.
//! - `capture.rs`: the capture scheduler (UI thread) and the poller thread.
//! - `input.rs`: remote input -> egui events and pen records; the pointer arbiter; gestures.
//! - `discovery.rs`: the UDP responder (Wi-Fi only). `adb.rs`: USB through `adb reverse`. `bt_win.rs`: RFCOMM.
//! - `client.rs`: `TestClient`, the protocol client of the tests and the `companion` scenario.
//!
//! `Companion` (owned by `WteApp`) has three hooks: `inject` in `raw_input_hook` (after the self-test's or the window's
//! pen records, before the pen router), `logic` in `App::logic` (events, STATE, capture, undo / redo, fit) and the UI
//! reads `AppState::companion` (the status). With no server running, `inject` and `logic` return at once.

pub mod adb;
pub mod auth;
#[cfg(windows)]
pub mod bt_win;
pub mod capture;
pub mod client;
pub mod crypto;
pub mod discovery;
pub mod encode;
pub mod input;
pub mod proto;
pub mod server;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::state::{AppState, CompanionPrefs, CompanionStatus, Dialog, ToolKind};
use auth::{PairedTablet, PairingWindow};
use capture::Capture;
use input::{Holder, InputStage, StageCtx};
use proto::{Msg, state_flag};
use server::{Bind, Hub, HubEvent, Server, ServerConfig, SessionInfo};

/// The session port (TCP), COMPANION_PLAN C12.
pub const DEFAULT_PORT: u16 = 47800;
/// The discovery port (UDP, Wi-Fi only).
pub const DISCOVERY_PORT: u16 = 47801;
/// The port of the `companion_host` test scenario (never the real one).
pub const TEST_PORT: u16 = 47811;
/// Where the tablet app is downloaded (dialog, Help, README).
pub const TABLET_APP_RELEASES: &str = "https://github.com/Maviman/tablet-companion/releases";
/// The tablet app's name.
pub const TABLET_APP_NAME: &str = "Tablet Companion";
/// Remote input keeps the window "focused" this long (5.5).
pub const REMOTE_FOCUS_HOLD: Duration = Duration::from_secs(5);
/// The trail colour sent in STATE (a neutral blue, 4.5; the tablet draws it with a dark outline).
pub const TRAIL_RGBA: u32 = 0xff20_40d0;

/// The three links.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Usb,
    Wifi,
    Bluetooth,
}

/// A self-test server (`companion` and `companion_host` scenarios; never in an interactive run).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestServer {
    /// 0 = an ephemeral port (`AppState::companion.tcp` has the bound one).
    pub port: u16,
    /// A fixed pairing code (`WTE_COMPANION_TEST_CODE`): the pairing window is always open.
    pub code: Option<String>,
    /// Confirm requests are allowed at once (`WTE_COMPANION_AUTO_ALLOW=1`).
    pub auto_allow: bool,
    /// The adb serial the watcher may address (`WTE_COMPANION_ADB_SERIAL`).
    pub adb_serial: Option<String>,
}

impl TestServer {
    /// The `companion_host` server from the environment (only read in `--screenshot` runs): `WTE_COMPANION_PORT`
    /// (default `TEST_PORT`), `WTE_COMPANION_TEST_CODE`, `WTE_COMPANION_AUTO_ALLOW`, `WTE_COMPANION_ADB_SERIAL`.
    pub fn from_env(default_port: u16, default_auto_allow: bool) -> TestServer {
        let var = |n: &str| std::env::var(n).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        TestServer {
            port: var("WTE_COMPANION_PORT").and_then(|p| p.parse().ok()).unwrap_or(default_port),
            code: var("WTE_COMPANION_TEST_CODE").and_then(|c| crypto::normalize_code(&c)),
            auto_allow: var("WTE_COMPANION_AUTO_ALLOW").map_or(default_auto_allow, |v| v == "1"),
            adb_serial: var("WTE_COMPANION_ADB_SERIAL"),
        }
    }
}

/// Commands of the dialog, the status chip and the scenarios (`AppAction::Companion`).
#[derive(Clone, Debug, PartialEq)]
pub enum CompanionAction {
    SetLink(LinkKind, bool),
    /// Use this port (restarts the listeners when it changed).
    SetPort(u16),
    Allow(u64),
    Deny(u64),
    /// Forget a paired tablet (its id as hex).
    Forget(String),
    Disconnect,
    /// Fit the window to the tablet (`true`) or restore it.
    Fit(bool),
    /// Browse for adb.exe (`None` = the native file dialog, asked by dispatch).
    BrowseAdb(Option<PathBuf>),
    /// Self-test only: a loopback server.
    StartTest(TestServer),
}

/// The running server and what belongs to it.
struct Running {
    server: Server,
    rx: Receiver<HubEvent>,
    plan: Vec<Bind>,
    adb: Option<adb::SharedLine>,
    bt_channel: Option<u32>,
    /// A self-test server.
    test: bool,
}

/// The connected tablet.
#[derive(Clone, Debug)]
struct Live {
    info: SessionInfo,
    screen: [u16; 2],
    view: Option<[u16; 2]>,
}

/// The desktop half of the companion (owned by `WteApp`).
pub struct Companion {
    selftest: bool,
    run: Option<Running>,
    capture: Capture,
    stage: InputStage,
    session: Option<Live>,
    /// Events read by `inject` that `logic` handles (it has the state's prefs and status).
    queued: Vec<HubEvent>,
    /// The server stopped with a session: the next `inject` releases what the remote held.
    carry_end: bool,
    /// A session ended: the next `logic` restores a fitted window.
    restore_on_end: bool,
    /// The last input sequence injected, and when remote input last came.
    injected_seq: u32,
    last_remote: Option<Instant>,
    /// ACTION ids waiting for idle tools.
    undo_redo: VecDeque<u16>,
    fit_req: Option<proto::FitRequest>,
    /// The inner size (points) before "Fit to tablet".
    fit_old: Option<egui::Vec2>,
    minimized: bool,
    /// What the remote held was released for this minimize (the releases sent while minimized are dropped).
    minimize_released: bool,
    /// When the last input (local or remote) came, for the capture rate.
    input_at: Option<Instant>,
    /// The remote stroke in contact: min and max pressure, samples (for `stroke_sessions`).
    stroke: Option<(u16, u16, u32)>,
    stroke_sessions: Vec<u64>,
    sessions_started: u32,
    sessions_ended: u32,
    /// The Bluetooth adapter check of the dialog (interactive runs only, once).
    bt_adapter: Option<bool>,
    lan: Vec<std::net::IpAddr>,
}

impl Companion {
    pub fn new(selftest: bool) -> Companion {
        Companion {
            selftest,
            run: None,
            capture: Capture::new(),
            stage: InputStage::default(),
            session: None,
            queued: Vec::new(),
            carry_end: false,
            restore_on_end: false,
            injected_seq: 0,
            last_remote: None,
            undo_redo: VecDeque::new(),
            fit_req: None,
            fit_old: None,
            minimized: false,
            minimize_released: false,
            input_at: None,
            stroke: None,
            stroke_sessions: Vec::new(),
            sessions_started: 0,
            sessions_ended: 0,
            bt_adapter: None,
            lan: Vec::new(),
        }
    }

    /// True while a server runs.
    #[cfg(test)]
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    fn session_id(&self) -> Option<u64> {
        self.session.as_ref().map(|s| s.info.id)
    }

    // ---- start / stop ----

    /// Interactive runs: opens what the preferences ask for (`bind_plan`), restarting the server when that changed.
    /// Never in a self-test run (C13).
    pub fn apply_prefs(&mut self, ctx: &egui::Context, prefs: &mut CompanionPrefs) {
        if self.selftest {
            return;
        }
        let plan = server::bind_plan(false, prefs, prefs.port, None);
        if self.run.as_ref().is_some_and(|r| r.plan == plan) {
            return;
        }
        self.stop();
        if plan.is_empty() {
            return;
        }
        if crypto::unhex_array::<16>(&prefs.desktop_id).is_none() {
            prefs.desktop_id = crypto::hex(&crypto::random_array::<16>());
        }
        let desktop_id = crypto::unhex_array::<16>(&prefs.desktop_id).unwrap_or_default();
        self.lan = if prefs.wifi { lan_addresses() } else { Vec::new() };
        let cfg = ServerConfig::default();
        self.start(ctx, plan, cfg, desktop_id, PairingWindow::default(), prefs.paired.clone(), prefs.adb_path.clone(), false);
    }

    /// Self-test runs: a loopback server (`bind_plan(true, ..)`: 127.0.0.1 only, adb only for the named serial).
    pub fn start_test(&mut self, ctx: &egui::Context, t: &TestServer) {
        if !self.selftest {
            log::warn!("companion: a test server is only started in self-test runs");
            return;
        }
        self.stop();
        let plan = server::bind_plan(true, &CompanionPrefs::default(), t.port, t.adb_serial.as_deref());
        let cfg = ServerConfig { auto_allow: t.auto_allow, ..ServerConfig::default() };
        let window = t.code.clone().map_or_else(PairingWindow::default, PairingWindow::fixed);
        self.start(ctx, plan, cfg, crypto::random_array(), window, Vec::new(), None, true);
    }

    #[allow(clippy::too_many_arguments, reason = "one call site per run kind")]
    fn start(
        &mut self,
        ctx: &egui::Context,
        plan: Vec<Bind>,
        cfg: ServerConfig,
        desktop_id: [u8; 16],
        window: PairingWindow,
        paired: Vec<PairedTablet>,
        adb_pref: Option<PathBuf>,
        test: bool,
    ) {
        let wake_ctx = ctx.clone();
        let (hub, rx) = Hub::new(cfg, desktop_id, window, paired, Box::new(move || wake_ctx.request_repaint()));
        let mut server = Server::start(Arc::clone(&hub), &plan);
        let port = server.tcp_addrs.first().map_or(DEFAULT_PORT, |a| a.port());
        let mut adb_line = None;
        let mut bt_channel = None;
        for b in &plan {
            match b {
                Bind::Adb { only_serial } => {
                    let (thread, line) = adb::watch(Arc::clone(&hub), adb_pref.clone(), port, only_serial.clone());
                    server.adopt(thread);
                    adb_line = Some(line);
                }
                Bind::Rfcomm => {
                    #[cfg(windows)]
                    match bt_win::listen_rfcomm(Arc::clone(&hub)) {
                        Ok((thread, channel)) => {
                            server.adopt(thread);
                            bt_channel = Some(channel);
                        }
                        Err(e) => server.errors.push(format!("Bluetooth: {e}")),
                    }
                }
                Bind::Tcp(_) | Bind::Udp(_) => {}
            }
        }
        for e in &server.errors {
            log::warn!("companion: {e}");
        }
        log::info!("companion: started ({plan:?}, TCP {:?})", server.tcp_addrs);
        self.run = Some(Running { server, rx, plan, adb: adb_line, bt_channel, test });
    }

    /// Stops the server (every listener and the session). The next `inject` releases what the remote held.
    pub fn stop(&mut self) {
        if let Some(run) = self.run.take() {
            run.server.stop();
            log::info!("companion: stopped");
        }
        self.capture.stop();
        if self.session.take().is_some() {
            self.sessions_ended += 1;
            self.carry_end = true;
        }
        self.queued.clear();
    }

    // ---- input (raw_input_hook) ----

    /// Feeds the remote input of this frame into `raw` and the pen feed (COMPANION_PLAN 5): after the self-test's or
    /// the window's pen records, before `PenState::route`. Returns at once when no server runs, and leaves the frame
    /// untouched while no tablet is connected.
    pub fn inject(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, state: &mut AppState) {
        let end_pending = std::mem::take(&mut self.carry_end);
        let Some(run) = &self.run else {
            if end_pending {
                self.end_session(raw, state);
            }
            return;
        };
        if !raw.events.is_empty() {
            self.input_at = Some(Instant::now());
        }
        let mut msgs: Vec<Msg> = Vec::new();
        let mut dropped_seq = 0u32;
        let mut ended = end_pending;
        while let Ok(e) = run.rx.try_recv() {
            match e {
                HubEvent::Input { session, msg } => {
                    if Some(session) == self.session.as_ref().map(|s| s.info.id) {
                        if self.minimized {
                            // Dropped (the stage released everything at the minimize), but counted as processed so
                            // the tablet's trail of it goes away.
                            dropped_seq = dropped_seq.max(msg.input_seq().unwrap_or(0));
                        } else {
                            msgs.push(msg);
                        }
                    }
                }
                HubEvent::Started { session, tablet_id, name, link, caps, screen } => {
                    let info = SessionInfo { id: session, tablet_id, name: name.clone(), link, caps };
                    self.session = Some(Live { info, screen, view: None });
                    self.queued.push(HubEvent::Started { session, tablet_id, name, link, caps, screen });
                }
                HubEvent::Ended { session, reason } => {
                    let mine = Some(session) == self.session.as_ref().map(|s| s.info.id);
                    self.queued.push(HubEvent::Ended { session, reason });
                    if mine {
                        self.session = None;
                        ended = true;
                        // Events after the end (a new session's input) wait for the next frame.
                        ctx.request_repaint();
                        break;
                    }
                }
                HubEvent::Config { session, config } => {
                    if let Some(live) = self.session.as_mut().filter(|s| s.info.id == session) {
                        live.view = Some([config.view_w, config.view_h]);
                    }
                }
                other => self.queued.push(other),
            }
        }
        if self.session.is_none() && msgs.is_empty() && !ended {
            return;
        }
        // Minimized (5.6): release what the remote holds once, as at a session end, since the releases the tablet sends
        // meanwhile are dropped; otherwise the local mouse stays locked out and a latched Ctrl sticks after the restore.
        if !self.minimized {
            self.minimize_released = false;
        } else if !self.minimize_released {
            self.minimize_released = true;
            self.stage.end_session(&mut raw.events, &mut state.pen.feed);
            self.stroke = None;
        }
        self.injected_seq = self.injected_seq.max(dropped_seq);
        self.probe_strokes(&msgs);
        let ppp = ctx.pixels_per_point();
        let views = finger_views(state);
        let cx = StageCtx { ppp, views: &views, fingers: state.prefs.companion.fingers, now: Instant::now() };
        let out = self.stage.process(&mut raw.events, &mut state.pen.feed, msgs, &cx);
        if out.last_seq > 0 {
            self.injected_seq = self.injected_seq.max(out.last_seq);
        }
        if out.had_input {
            self.last_remote = Some(Instant::now());
            self.input_at = self.last_remote;
        }
        self.undo_redo.extend(out.actions.iter().copied().filter(|a| matches!(*a, proto::action::UNDO | proto::action::REDO)));
        if out.fit.is_some() {
            self.fit_req = out.fit;
        }
        if ended {
            self.end_session(raw, state);
        }
        // 5.5: the window counts as focused while the remote is active.
        if self.session.is_some() && (self.stage.active() || self.last_remote.is_some_and(|t| t.elapsed() < REMOTE_FOCUS_HOLD)) {
            raw.focused = true;
            raw.events.retain(|e| !matches!(e, egui::Event::WindowFocused(false)));
        }
    }

    /// End of a session, for any reason (5.6): the stage releases the pen, a gesture button and the remote modifiers;
    /// the next `logic` restores a fitted window.
    fn end_session(&mut self, raw: &mut egui::RawInput, state: &mut AppState) {
        self.stage.end_session(&mut raw.events, &mut state.pen.feed);
        self.stroke = None;
        self.injected_seq = 0;
        self.last_remote = None;
        self.undo_redo.clear();
        self.fit_req = None;
        self.restore_on_end = true;
    }

    /// Counts the sessions that drew a stroke with varying pressure (the `companion_host` scenario).
    fn probe_strokes(&mut self, msgs: &[Msg]) {
        let Some(id) = self.session_id() else { return };
        for m in msgs {
            let Msg::Pen(p) = m else { continue };
            for s in &p.samples {
                match s.kind {
                    proto::pen_kind::DOWN => self.stroke = Some((s.pressure, s.pressure, 1)),
                    proto::pen_kind::MOVE => {
                        if let Some((lo, hi, n)) = self.stroke.as_mut() {
                            *lo = (*lo).min(s.pressure);
                            *hi = (*hi).max(s.pressure);
                            *n += 1;
                        }
                    }
                    proto::pen_kind::UP | proto::pen_kind::CANCEL | proto::pen_kind::LEAVE => {
                        if let Some((lo, hi, n)) = self.stroke.take()
                            && n >= 3
                            && hi.saturating_sub(lo) >= 65535 / 5
                            && !self.stroke_sessions.contains(&id)
                        {
                            log::info!("companion: session {id} drew a stroke with varying pressure ({n} samples)");
                            self.stroke_sessions.push(id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- per frame (logic) ----

    /// Events, pairing window, undo / redo, fit, STATE, CURSOR and the capture; writes `state.companion`.
    pub fn logic(&mut self, ctx: &egui::Context, state: &mut AppState, device: &eframe::wgpu::Device, tools_busy: bool) {
        if std::mem::take(&mut self.restore_on_end) {
            self.restore_fit(ctx, &mut state.companion);
        }
        let Some(run) = &self.run else {
            if state.companion.running {
                state.companion = CompanionStatus {
                    sessions_started: self.sessions_started,
                    sessions_ended: self.sessions_ended,
                    stroke_sessions: self.stroke_sessions.clone(),
                    ..CompanionStatus::default()
                };
            }
            return;
        };
        let hub = Arc::clone(&run.server.hub);
        let (test, wifi) = (run.test, run.plan.iter().any(|b| matches!(b, Bind::Udp(_))));
        let now = Instant::now();
        // Events of the session threads.
        for e in std::mem::take(&mut self.queued) {
            match e {
                HubEvent::Started { name, link, .. } => {
                    self.sessions_started += 1;
                    self.capture.reset();
                    log::info!("companion: {name} connected ({})", link_name(link));
                }
                HubEvent::Ended { reason, .. } => {
                    self.sessions_ended += 1;
                    let (n, mean, max) = self.capture.stats();
                    log::info!(
                        "companion: the tablet disconnected: {reason}; {n} captures (request -> pixels mean {mean:.1} ms, max {max:.1} ms), {} frames, {} bytes sent",
                        hub.frames_sent.load(std::sync::atomic::Ordering::SeqCst),
                        hub.bytes_sent.load(std::sync::atomic::Ordering::SeqCst)
                    );
                }
                HubEvent::Paired(record) => {
                    let prefs = &mut state.prefs.companion;
                    prefs.paired.retain(|p| p.id != record.id);
                    prefs.paired.push(record);
                    if !state.selftest {
                        crate::prefs::save(&state.prefs);
                    }
                }
                HubEvent::Refused { name, code, text } => {
                    state.companion.last_refused = Some(format!("{name}: {text} (code {code})"));
                }
                HubEvent::ConfirmRequest { .. } | HubEvent::Config { .. } | HubEvent::Input { .. } => {}
            }
        }
        // The pairing window is open while the dialog is (a fixed self-test code keeps it open).
        let dialog_open = matches!(state.dialog, Dialog::Companion);
        let code = {
            let mut a = hub.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if dialog_open != a.window.is_open() {
                if dialog_open {
                    a.window.open(now);
                } else {
                    a.window.close();
                }
            }
            a.window.code(now).filter(|_| test || wifi)
        };
        // Undo / redo from the toolbar, when the tools are idle.
        while !tools_busy && let Some(a) = self.undo_redo.pop_front() {
            state.push(if a == proto::action::UNDO { crate::action::AppAction::Undo } else { crate::action::AppAction::Redo });
        }
        // Minimized: no picture; back: a KEY frame.
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized != self.minimized {
            self.minimized = minimized;
            if !minimized {
                hub.request_key();
                self.capture.reset();
            }
        }
        // Fit to tablet.
        let (maximized, fullscreen) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
        let fit_ok = state.prefs.companion.allow_resize && !maximized && !fullscreen;
        if let Some(req) = self.fit_req.take() {
            if req.mode == 0 {
                self.restore_fit(ctx, &mut state.companion);
            } else if fit_ok {
                self.fit(ctx, [req.aspect_w, req.aspect_h], &mut state.companion);
            }
        }
        if !state.prefs.companion.allow_resize && self.fit_old.is_some() {
            self.restore_fit(ctx, &mut state.companion);
        }
        // STATE, CURSOR and the capture.
        let ppp = ctx.pixels_per_point();
        if self.session.is_some() {
            let size = (ctx.content_rect().size() * ppp).round();
            let mut flags = 0;
            if minimized {
                flags |= state_flag::NO_PICTURE;
            }
            if ctx.text_edit_focused() {
                flags |= state_flag::TEXT_WANTED;
            }
            if matches!(state.tool, ToolKind::Brush | ToolKind::Eraser) {
                flags |= state_flag::TRAIL;
            }
            if self.stage.holder == Holder::Local {
                flags |= state_flag::POINTER_HELD_LOCALLY;
            }
            if !fit_ok {
                flags |= state_flag::FIT_UNAVAILABLE;
            }
            if self.fit_old.is_some() {
                flags |= state_flag::FITTED;
            }
            hub.set_state(proto::State {
                surface_w: size.x.clamp(0.0, f32::from(u16::MAX)) as u16,
                surface_h: size.y.clamp(0.0, f32::from(u16::MAX)) as u16,
                pixels_per_point: ppp,
                flags,
                trail_rgba: TRAIL_RGBA,
                title: state.last_title.clone(),
            });
            hub.set_cursor(self.stage.local_pointer.map(|p| proto::Cursor { x: p.x * ppp, y: p.y * ppp, visible: true, icon: 0 }));
            if !minimized {
                let _p = crate::app::frame_profile::scope("companion capture");
                let input_recent = self.input_at.is_some_and(|t| t.elapsed() < capture::INPUT_RECENT);
                let session = self.session_id().unwrap_or(0);
                self.capture.pass(ctx, &hub, device, self.injected_seq, session, input_recent);
            }
        }
        // While the remote holds something down, keep frames coming (not for a hovering pen or a latched modifier).
        if self.stage.needs_ticks() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.write_status(state, &hub, code, ctx);
    }

    /// Fills `state.companion` (chip, dialog lines, scenario counters).
    fn write_status(&mut self, state: &mut AppState, hub: &Hub, code: Option<String>, ctx: &egui::Context) {
        let Some(run) = &self.run else { return };
        let confirm = hub.pending_confirm().map(|(id, name)| (id, name, proto::link::USB));
        let session = self.session.as_ref().map(|s| (s.info.name.clone(), s.info.link));
        let chip = Some(match (&session, &confirm) {
            (Some((name, link)), _) => format!("Tablet: {name} ({})", link_name(*link)),
            (None, Some(_)) => "Tablet: pairing".to_owned(),
            (None, None) => "Tablet: waiting".to_owned(),
        });
        let adb_line = match &run.adb {
            Some(line) => line.lock().unwrap_or_else(std::sync::PoisonError::into_inner).text(),
            None => "Off".to_owned(),
        };
        let bt_line = if run.plan.contains(&Bind::Rfcomm) {
            match run.bt_channel {
                Some(_) => format!("Ready: pair the tablet in Windows Settings once, then choose Bluetooth in {TABLET_APP_NAME}"),
                None => "No Bluetooth adapter (or it is turned off)".to_owned(),
            }
        } else if !self.selftest && matches!(state.dialog, Dialog::Companion) {
            // The read-only adapter check, once, in interactive runs only (C14).
            #[cfg(windows)]
            let present = *self.bt_adapter.get_or_insert_with(bt_win::adapter_present);
            #[cfg(not(windows))]
            let present = false;
            if present { "Off".to_owned() } else { "No Bluetooth adapter".to_owned() }
        } else {
            "Off".to_owned()
        };
        let c = &mut state.companion;
        c.running = true;
        c.chip = chip;
        c.session = session;
        c.confirm = confirm;
        c.code = code;
        c.tcp = run.server.tcp_addrs.clone();
        c.adb_line = adb_line;
        c.bt_line = bt_line;
        c.lan = self.lan.clone();
        c.errors = run.server.errors.clone();
        c.fitted = self.fit_old.is_some();
        let size = (ctx.content_rect().size() * ctx.pixels_per_point()).round();
        c.surface = [size.x.max(0.0) as u32, size.y.max(0.0) as u32];
        c.ppp = ctx.pixels_per_point();
        c.sessions_started = self.sessions_started;
        c.sessions_ended = self.sessions_ended;
        c.stroke_sessions = self.stroke_sessions.clone();
        c.last_input_seq = self.injected_seq;
        c.pointer_down = ctx.input(|i| i.pointer.any_down());
        c.frames_sent = hub.frames_sent.load(std::sync::atomic::Ordering::SeqCst);
        c.captures = self.capture.stats().0;
    }

    /// "Fit to tablet": the window takes the tablet's aspect, keeping its height, within the monitor; never a focus
    /// change (`InnerSize` resizes with SWP_NOACTIVATE, plan fact 23).
    fn fit(&mut self, ctx: &egui::Context, aspect: [u16; 2], status: &mut CompanionStatus) {
        let (inner, monitor) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
        let Some(inner) = inner else { return };
        if aspect[0] == 0 || aspect[1] == 0 {
            return;
        }
        let old = *self.fit_old.get_or_insert(inner.size());
        let size = fit_size(old, f32::from(aspect[0]) / f32::from(aspect[1]), monitor);
        log::info!("companion: fit the window to the tablet: {:?} -> {size:?}", inner.size());
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        status.fitted = true;
    }

    /// Restores the size from before "Fit to tablet" (Restore, disconnect, the option turned off).
    fn restore_fit(&mut self, ctx: &egui::Context, status: &mut CompanionStatus) {
        if let Some(old) = self.fit_old.take() {
            log::info!("companion: restore the window size {old:?}");
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(old));
        }
        status.fitted = false;
    }

    // ---- dialog commands ----

    /// Runs a dialog / scenario command.
    pub fn command(&mut self, ctx: &egui::Context, state: &mut AppState, a: CompanionAction) {
        let hub = self.run.as_ref().map(|r| Arc::clone(&r.server.hub));
        match a {
            CompanionAction::SetLink(link, on) => {
                if state.selftest {
                    return;
                }
                let p = &mut state.prefs.companion;
                match link {
                    LinkKind::Usb => p.usb = on,
                    LinkKind::Wifi => p.wifi = on,
                    LinkKind::Bluetooth => p.bluetooth = on,
                }
                self.apply_prefs(ctx, &mut state.prefs.companion);
                crate::prefs::save(&state.prefs);
            }
            CompanionAction::SetPort(port) => {
                if state.selftest || port == 0 {
                    return;
                }
                state.prefs.companion.port = port;
                self.apply_prefs(ctx, &mut state.prefs.companion);
                crate::prefs::save(&state.prefs);
            }
            CompanionAction::Allow(id) => {
                if let Some(hub) = hub {
                    hub.answer_confirm(id, true);
                }
            }
            CompanionAction::Deny(id) => {
                if let Some(hub) = hub {
                    hub.answer_confirm(id, false);
                }
            }
            CompanionAction::Forget(id) => {
                state.prefs.companion.paired.retain(|p| p.id != id);
                if let Some(hub) = &hub {
                    hub.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).paired.retain(|p| p.id != id);
                    if self.session.as_ref().is_some_and(|s| crypto::hex(&s.info.tablet_id) == id) {
                        hub.disconnect();
                    }
                }
                if !state.selftest {
                    crate::prefs::save(&state.prefs);
                }
            }
            CompanionAction::Disconnect => {
                if let Some(hub) = hub {
                    hub.disconnect();
                }
            }
            CompanionAction::Fit(true) => {
                if let Some(live) = &self.session {
                    let [w, h] = live.view.unwrap_or(live.screen);
                    self.fit_req = Some(proto::FitRequest { mode: 1, aspect_w: w, aspect_h: h });
                }
            }
            CompanionAction::Fit(false) => self.fit_req = Some(proto::FitRequest { mode: 0, aspect_w: 0, aspect_h: 0 }),
            CompanionAction::BrowseAdb(path) => {
                let Some(path) = path else { return };
                state.prefs.companion.adb_path = Some(path);
                if state.selftest {
                    return;
                }
                crate::prefs::save(&state.prefs);
                // The watcher looks again with the new path.
                if self.run.is_some() {
                    self.stop();
                    self.apply_prefs(ctx, &mut state.prefs.companion);
                }
            }
            CompanionAction::StartTest(t) => self.start_test(ctx, &t),
        }
    }
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The rects in which the gesture mapper treats one finger as navigation (8.14, `StageCtx::views`): the views of the
/// last frame and (stickers P6a, review A16) the sticker canvas, which is not a `ViewId`; none while a dialog is open.
pub(crate) fn finger_views(state: &AppState) -> Vec<egui::Rect> {
    if !matches!(state.dialog, Dialog::None) {
        return Vec::new();
    }
    state.view_rects.values().copied().chain(state.sticker_canvas_rect).collect()
}

/// "USB", "Wi-Fi", "Bluetooth".
pub fn link_name(link: u8) -> &'static str {
    match link {
        proto::link::USB => "USB",
        proto::link::WIFI => "Wi-Fi",
        proto::link::BLUETOOTH => "Bluetooth",
        _ => "?",
    }
}

/// The fitted inner size (points): the tablet's aspect at the current height, narrowed to the monitor (96 % of its
/// width, 90 % of its height) keeping the aspect. Pure.
pub fn fit_size(current: egui::Vec2, aspect: f32, monitor: Option<egui::Vec2>) -> egui::Vec2 {
    let mut h = current.y.max(100.0);
    let mut w = h * aspect;
    if let Some(m) = monitor.filter(|m| m.x > 0.0 && m.y > 0.0) {
        let (max_w, max_h) = (m.x * 0.96, m.y * 0.9);
        if h > max_h {
            h = max_h;
            w = h * aspect;
        }
        if w > max_w {
            w = max_w;
            h = w / aspect;
        }
    }
    egui::vec2(w.round(), h.round())
}

/// This PC's LAN address for the Wi-Fi line: a UDP socket "connected" to a public address names the outgoing
/// interface; no packet is sent (6.2). Only while Wi-Fi is on, never in a self-test run.
fn lan_addresses() -> Vec<std::net::IpAddr> {
    let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") else { return Vec::new() };
    if sock.connect("192.0.2.1:9").is_err() {
        return Vec::new();
    }
    sock.local_addr().map(|a| vec![a.ip()]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_height_and_the_monitor() {
        assert_eq!(fit_size(egui::vec2(1280.0, 800.0), 1.6, None), egui::vec2(1280.0, 800.0));
        assert_eq!(fit_size(egui::vec2(1280.0, 720.0), 2960.0 / 1848.0, None), egui::vec2(1153.0, 720.0));
        // A wide aspect on a small monitor: narrowed to 96 % of its width.
        assert_eq!(fit_size(egui::vec2(1000.0, 1000.0), 2.0, Some(egui::vec2(1600.0, 1200.0))), egui::vec2(1536.0, 768.0));
        // Taller than the monitor: 90 % of its height.
        assert_eq!(fit_size(egui::vec2(800.0, 1200.0), 1.0, Some(egui::vec2(1920.0, 1080.0))), egui::vec2(972.0, 972.0));
    }

    #[test]
    fn no_server_or_no_session_leaves_the_frame_untouched() {
        let ctx = egui::Context::default();
        let mut c = Companion::new(true);
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        let events = vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0)), egui::Event::WindowFocused(false)];
        let mut raw = egui::RawInput { events: events.clone(), focused: false, ..egui::RawInput::default() };
        c.inject(&ctx, &mut raw, &mut state);
        assert_eq!(raw.events, events);
        assert!(!raw.focused);
        assert!(state.pen.feed.is_empty());
        // A running loopback server without a session leaves the frame alone too.
        c.start_test(&ctx, &TestServer { port: 0, code: None, auto_allow: true, adb_serial: None });
        assert!(c.running());
        assert!(c.run.as_ref().unwrap().server.tcp_addrs.iter().all(|a| a.ip().is_loopback()));
        c.inject(&ctx, &mut raw, &mut state);
        assert_eq!(raw.events, events);
        assert!(!raw.focused);
        c.stop();
        // An interactive companion never starts a test server.
        let mut interactive = Companion::new(false);
        interactive.start_test(&ctx, &TestServer { port: 0, code: None, auto_allow: true, adb_serial: None });
        assert!(!interactive.running());
    }

    #[test]
    fn a_session_end_in_mid_stroke_releases_the_pen() {
        use client::{TestClient, test_hello};
        use proto::{PenSample, pen_kind};
        let ctx = egui::Context::default();
        let mut c = Companion::new(true);
        let mut state = AppState::new(crate::state::Prefs::default(), true);
        c.start_test(&ctx, &TestServer { port: 0, code: None, auto_allow: true, adb_serial: None });
        let addr = c.run.as_ref().unwrap().server.tcp_addrs[0];
        let mut client = TestClient::connect(addr).unwrap();
        client.handshake(test_hello(proto::link::USB, [7; 16], "Tab"), auth::ClientKey::Confirm).unwrap();
        let s = |kind, x: f32| PenSample { kind, flags: 0, pressure: 30000, x, y: 10.0, tilt: 255, reserved: 0, dt_us: 1000 };
        client.pen(vec![s(pen_kind::DOWN, 10.0), s(pen_kind::MOVE, 12.0)]).unwrap();
        // Frames until the session and the stroke arrived.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut router = crate::pen::PenRouter::default();
        while Instant::now() < deadline {
            let mut raw = egui::RawInput::default();
            c.inject(&ctx, &mut raw, &mut state);
            let frame = router.process(&mut raw.events, &mut state.pen.feed);
            if frame.in_contact {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(c.stage.holder, Holder::Remote);
        client.drop_connection();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut released = false;
        while Instant::now() < deadline && !released {
            let mut raw = egui::RawInput::default();
            c.inject(&ctx, &mut raw, &mut state);
            let frame = router.process(&mut raw.events, &mut state.pen.feed);
            released = raw.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. })) && !frame.in_contact;
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(released, "the stroke was released at the disconnect");
        assert_eq!(c.stage.holder, Holder::None);
        assert!(c.session.is_none());
        c.stop();
    }

    /// A connected loopback session for the minimize tests (frames run until the session is known).
    fn minimize_setup() -> (egui::Context, Companion, AppState, client::TestClient, crate::pen::PenRouter) {
        let ctx = egui::Context::default();
        let mut c = Companion::new(true);
        let state = AppState::new(crate::state::Prefs::default(), true);
        c.start_test(&ctx, &TestServer { port: 0, code: None, auto_allow: true, adb_serial: None });
        let addr = c.run.as_ref().unwrap().server.tcp_addrs[0];
        let mut client = client::TestClient::connect(addr).unwrap();
        client.handshake(client::test_hello(proto::link::USB, [7; 16], "Tab"), auth::ClientKey::Confirm).unwrap();
        (ctx, c, state, client, crate::pen::PenRouter::default())
    }

    /// Runs frames for `ms` (pinging so the session stays up); returns every routed event.
    fn frames_for(c: &mut Companion, ctx: &egui::Context, state: &mut AppState, router: &mut crate::pen::PenRouter, client: &mut client::TestClient, ms: u64) -> Vec<egui::Event> {
        let mut all = Vec::new();
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            let _ = client.send(&Msg::Ping { t: 1 });
            let mut raw = egui::RawInput::default();
            c.inject(ctx, &mut raw, state);
            router.process(&mut raw.events, &mut state.pen.feed);
            all.extend(raw.events);
            std::thread::sleep(Duration::from_millis(10));
        }
        all
    }

    fn pen_s(kind: u8, x: f32) -> proto::PenSample {
        proto::PenSample { kind, flags: 0, pressure: 30000, x, y: 10.0, tilt: 255, reserved: 0, dt_us: 1000 }
    }

    /// The window is minimized while the tablet pen is down and the pen lifts meanwhile (checker finding): after the
    /// restore the local mouse works and the tablet's next stroke is a new press, not a continuation.
    #[test]
    fn a_minimize_in_mid_stroke_releases_the_remote() {
        use proto::pen_kind;
        let (ctx, mut c, mut state, mut client, mut router) = minimize_setup();
        client.pen(vec![pen_s(pen_kind::DOWN, 10.0), pen_s(pen_kind::MOVE, 12.0)]).unwrap();
        frames_for(&mut c, &ctx, &mut state, &mut router, &mut client, 600);
        assert_eq!(c.stage.holder, Holder::Remote, "the remote stroke is down");
        // What logic() records when the viewport reports minimized.
        c.minimized = true;
        let ev = frames_for(&mut c, &ctx, &mut state, &mut router, &mut client, 100);
        assert!(ev.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. })), "released at the minimize: {ev:?}");
        let up_seq = client.pen(vec![pen_s(pen_kind::UP, 14.0), pen_s(pen_kind::LEAVE, 14.0)]).unwrap();
        frames_for(&mut c, &ctx, &mut state, &mut router, &mut client, 500);
        assert!(c.injected_seq >= up_seq, "dropped input still counts as processed ({} < {up_seq})", c.injected_seq);
        c.minimized = false;
        let mut raw = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(egui::pos2(50.0, 50.0)),
                egui::Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            ],
            ..egui::RawInput::default()
        };
        c.inject(&ctx, &mut raw, &mut state);
        let f = router.process(&mut raw.events, &mut state.pen.feed);
        assert!(!f.in_contact);
        assert_eq!(raw.events.len(), 2, "the local mouse passes after the restore: {:?}", raw.events);
        assert_eq!(c.stage.holder, Holder::Local);
        let mut raw = egui::RawInput {
            events: vec![egui::Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
            ..egui::RawInput::default()
        };
        c.inject(&ctx, &mut raw, &mut state);
        router.process(&mut raw.events, &mut state.pen.feed);
        // The tablet's next stroke is a stroke of its own.
        client.pen(vec![pen_s(pen_kind::HOVER, 300.0), pen_s(pen_kind::DOWN, 300.0), pen_s(pen_kind::MOVE, 302.0), pen_s(pen_kind::UP, 302.0)]).unwrap();
        let ev = frames_for(&mut c, &ctx, &mut state, &mut router, &mut client, 500);
        let presses: Vec<_> = ev.iter().filter_map(|e| match e {
            egui::Event::PointerButton { pos, pressed: true, .. } => Some(pos.x),
            _ => None,
        }).collect();
        assert_eq!(presses, vec![300.0], "one new press at the new stroke: {ev:?}");
        c.stop();
    }

    /// The tablet's Ctrl latch is released while the window is minimized (checker finding): Ctrl does not stick.
    #[test]
    fn a_minimize_clears_a_latched_modifier() {
        let (ctx, mut c, mut state, mut client, mut router) = minimize_setup();
        client.mods(proto::mods::CTRL).unwrap();
        frames_for(&mut c, &ctx, &mut state, &mut router, &mut client, 600);
        assert!(c.stage.active());
        c.minimized = true;
        client.mods(0).unwrap();
        frames_for(&mut c, &ctx, &mut state, &mut router, &mut client, 400);
        c.minimized = false;
        let mut raw = egui::RawInput {
            events: vec![egui::Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }],
            ..egui::RawInput::default()
        };
        c.inject(&ctx, &mut raw, &mut state);
        assert!(
            !raw.events.iter().any(|e| matches!(e, egui::Event::PointerButton { modifiers, .. } if modifiers.ctrl)),
            "the released latch does not apply to the desktop mouse: {:?}",
            raw.events
        );
        assert!(!c.stage.active());
        c.stop();
    }
}
