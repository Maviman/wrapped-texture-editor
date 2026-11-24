// Branch `companion` (COMPANION_PLAN 8.3, ARCHITECTURE 8.11 / 8.14): the scenarios `companion` and `companion_host`.
//
// `companion`: in-process and loopback only, no Android. The scenario starts the companion server on 127.0.0.1:0 with
// auto-allow and drives a `TestClient` on a thread of its own (the "tablet"): it pairs in confirm mode over "USB",
// receives the KEY frame, sends a 2D pen ramp (pressure 0.1 -> 1.0), an eraser-end stroke, Undo through ACTION, a barrel
// drag and a two-finger pinch in 3D, KEY / TEXT into the Text tool's content field, and finally pulls the cable in the
// middle of a stroke. Shots: `companion_dialog` (Help > Tablet companion while connected); the final screenshot shows
// the ramp and the cut-off stroke.
//
// `companion_host`: the desktop side of the Android end-to-end run (tablet-companion/tools/e2e.sh). It listens on
// 127.0.0.1:`WTE_COMPANION_PORT` (default 47811) with `WTE_COMPANION_TEST_CODE` / `WTE_COMPANION_AUTO_ALLOW`, writes the
// surface coordinates of the 2D view to `<out_stem>_coords.json`, waits (up to 300 s) until
// `WTE_COMPANION_EXPECT_SESSIONS` sessions have each drawn a pen stroke with varying pressure, asserts the paint and
// exits with the screenshot.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};

use super::pen::{RAMP_U, RAMP_V, SIZE, alpha_at, assert_ramp, paint_count, remember_paint, reset_probes, same_paint};
use super::*;
use crate::companion::client::{TestClient, test_hello};
use crate::companion::proto::{self, PenSample, TouchPoint, pen_flag, pen_kind, state_flag, touch_phase};
use crate::companion::{CompanionAction, TEST_PORT, TestServer, auth::ClientKey};

/// The test tablet's name (the chip and the paired list show it).
const TABLET: &str = "Galaxy Tab S8 Ultra (test)";
/// Input messages are spaced like a 120 Hz digitizer batched per display frame.
const MESSAGE_GAP: Duration = Duration::from_millis(16);

// ---- the test tablet (a `TestClient` on its own thread) ----

/// What the scenario asks the tablet to do. Positions are egui points (the driver converts to surface pixels).
enum Cmd {
    /// Hover at the first point, touch down, move through the rest; `lift` = up and out of range at the end.
    Pen { points: Vec<(egui::Pos2, f32)>, flags: u8, lift: bool },
    /// TOUCH messages, each a list of (finger id, phase, position).
    Touch(Vec<Vec<(u8, u8, egui::Pos2)>>),
    Action(u16),
    Key(u16, bool),
    Text(&'static str),
    /// Close the connection without BYE (a pulled cable).
    Drop,
}

/// What the tablet saw.
#[derive(Clone, Debug, Default)]
struct Report {
    connected: bool,
    /// The picture size of the first KEY frame.
    key: Option<[u16; 2]>,
    frames: usize,
    sent_seq: u32,
    confirmed: u32,
    flags: u32,
    ppp: f32,
    /// Commands done.
    done: u32,
    error: Option<String>,
}

struct Driver {
    tx: Sender<Cmd>,
    issued: u32,
    report: Arc<Mutex<Report>>,
}

static DRIVER: Mutex<Option<Driver>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn report() -> Result<Report, String> {
    let d = lock(&DRIVER);
    let d = d.as_ref().ok_or("the test tablet is not running")?;
    let r = lock(&d.report).clone();
    match &r.error {
        Some(e) => Err(format!("test tablet: {e}")),
        None => Ok(r),
    }
}

fn send(cmd: Cmd) -> Result<(), String> {
    let mut d = lock(&DRIVER);
    let d = d.as_mut().ok_or("the test tablet is not running")?;
    d.issued += 1;
    d.tx.send(cmd).map_err(|_| "the test tablet thread ended".to_owned())
}

/// Every command is done and the last input is confirmed by a frame (`last_input_seq`).
fn caught_up(_: &AppState, _: &Scratch) -> Result<bool, String> {
    let issued = lock(&DRIVER).as_ref().map_or(0, |d| d.issued);
    let r = report()?;
    Ok(r.done == issued && r.sent_seq > 0 && r.confirmed >= r.sent_seq)
}

/// Connects the test tablet to the scenario's server (confirm mode on a loopback "USB" link).
fn start_tablet(state: &mut AppState, _: &mut Scratch) -> Result<(), String> {
    let addr = *state.companion.tcp.first().ok_or("the companion server is not listening")?;
    if !addr.ip().is_loopback() {
        return Err(format!("the server listens on {addr}, not on 127.0.0.1"));
    }
    let (tx, rx) = channel();
    let report = Arc::new(Mutex::new(Report::default()));
    let r = Arc::clone(&report);
    std::thread::Builder::new()
        .name("wte-test-tablet".into())
        .spawn(move || {
            if let Err(e) = run_tablet(addr, rx, &r) {
                lock(&r).error = Some(e);
            }
        })
        .map_err(|e| e.to_string())?;
    *lock(&DRIVER) = Some(Driver { tx, issued: 0, report });
    Ok(())
}

fn run_tablet(addr: std::net::SocketAddr, rx: Receiver<Cmd>, report: &Mutex<Report>) -> Result<(), String> {
    let mut client = TestClient::connect(addr).map_err(|e| e.to_string())?;
    client.handshake(test_hello(proto::link::USB, [0x5a; 16], TABLET), ClientKey::Confirm)?;
    lock(report).connected = true;
    loop {
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(Cmd::Drop) => {
                client.drop_connection();
                lock(report).done += 1;
                return Ok(());
            }
            Ok(cmd) => {
                run_cmd(&mut client, cmd, report).map_err(|e| e.to_string())?;
                lock(report).done += 1;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        pump(&mut client, Duration::from_millis(5), report);
    }
}

/// Reads frames and STATE for `t` (acking them) and updates the report.
fn pump(client: &mut TestClient, t: Duration, report: &Mutex<Report>) {
    client.wait_for(t, |_| false);
    let mut r = lock(report);
    r.frames = client.frames.len();
    if r.key.is_none() {
        r.key = client.frames.iter().find(|f| f.flags & proto::frame_flag::KEY != 0).map(|f| [f.pic_w, f.pic_h]);
    }
    r.sent_seq = client.last_seq();
    r.confirmed = client.confirmed_seq();
    if let Some(s) = &client.state {
        r.flags = s.flags;
        r.ppp = s.pixels_per_point;
    }
}

fn run_cmd(client: &mut TestClient, cmd: Cmd, report: &Mutex<Report>) -> std::io::Result<()> {
    let ppp = client.state.as_ref().map_or(1.0, |s| s.pixels_per_point).max(0.1);
    let sample = |kind: u8, p: egui::Pos2, pressure: f32, flags: u8| PenSample {
        kind,
        flags,
        pressure: (pressure.clamp(0.0, 1.0) * 65535.0).round() as u16,
        x: p.x * ppp,
        y: p.y * ppp,
        tilt: 255,
        reserved: 0,
        dt_us: 8000,
    };
    match cmd {
        Cmd::Pen { points, flags, lift } => {
            let Some(&(first, _)) = points.first() else { return Ok(()) };
            let mut samples = vec![sample(pen_kind::HOVER, first, 0.0, flags)];
            for (k, (p, pressure)) in points.iter().enumerate() {
                samples.push(sample(if k == 0 { pen_kind::DOWN } else { pen_kind::MOVE }, *p, *pressure, flags));
            }
            if lift {
                let last = points.last().map_or(first, |p| p.0);
                samples.push(sample(pen_kind::UP, last, 0.0, flags));
                samples.push(sample(pen_kind::LEAVE, last, 0.0, flags));
            }
            // Two samples per message, one message per display frame (every sample is sent, in order).
            for chunk in samples.chunks(2) {
                client.pen(chunk.to_vec())?;
                pump(client, MESSAGE_GAP, report);
            }
        }
        Cmd::Touch(messages) => {
            // Fingers right after the pen are a resting palm (5.3): wait out the palm hold first.
            pump(client, crate::companion::input::PALM_HOLD + Duration::from_millis(100), report);
            for m in messages {
                let points = m.iter().map(|&(id, phase, p)| TouchPoint { id, phase, x: p.x * ppp, y: p.y * ppp }).collect();
                client.touch(points)?;
                pump(client, MESSAGE_GAP, report);
            }
        }
        Cmd::Action(a) => {
            client.action(a)?;
        }
        Cmd::Key(key, pressed) => {
            client.key(key, pressed, 0)?;
        }
        Cmd::Text(t) => {
            client.text(t)?;
        }
        Cmd::Drop => {}
    }
    pump(client, MESSAGE_GAP, report);
    Ok(())
}

// ---- helpers ----

/// Contact points along screen positions with the pressure ramping from `p.0` to `p.1`.
fn ramp(screen: &[egui::Pos2], p: (f32, f32)) -> Vec<(egui::Pos2, f32)> {
    let points = densify(screen);
    let n = points.len().max(2) - 1;
    points.iter().enumerate().map(|(k, q)| (*q, p.0 + (p.1 - p.0) * (k as f32 / n as f32))).collect()
}

/// A tablet pen stroke in the 2D view along the row `v`.
fn uv_row(state: &AppState, v: f32, p: (f32, f32), flags: u8, lift: bool) -> Result<(), String> {
    let xf = view_xf(state, ViewId::Main2D)?;
    let screen = [uv_screen(&xf, [RAMP_U[0], v]), uv_screen(&xf, [RAMP_U[1], v])];
    send(Cmd::Pen { points: ramp(&screen, p), flags, lift })
}

fn camera(state: &AppState) -> Result<crate::state::OrbitCamera, String> {
    Ok(state.session.as_ref().ok_or("no session")?.camera)
}

fn expect_chip(state: &AppState, want: &str) -> Result<(), String> {
    match state.companion.chip.as_deref() {
        Some(c) if c == want => Ok(()),
        other => Err(format!("the status chip says {other:?}, not {want:?}")),
    }
}

/// The 2D setup of the `pen` scenario: Sphere, 2D view, blue, size 40, hard, pressure drives the size from 0.
fn document_setup() -> Vec<Step> {
    vec![
        Step::Do(AppAction::OpenSample(SampleKind::Sphere)),
        Step::WaitIdle,
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::Do(AppAction::SetPrimaryColor(BLUE)),
        Step::Setup("brush: size 40, hardness 1, flow 1, pressure Size on (min 0), Flow off", |s, _| {
            s.tools.brush = BrushSettings {
                size_px: SIZE,
                hardness: 1.0,
                flow: 1.0,
                pressure_size: true,
                pressure_size_min: 0.0,
                pressure_flow: false,
                ..BrushSettings::default()
            };
            Ok(())
        }),
        Step::WaitIdle,
    ]
}

// ---- `companion` ----

/// The `companion` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn companion() -> Vec<Step> {
    let mut steps = document_setup();
    steps.extend([
        Step::Do(AppAction::Companion(CompanionAction::StartTest(TestServer {
            port: 0,
            code: None,
            auto_allow: true,
            adb_serial: None,
        }))),
        Step::WaitFrames(2),
        Step::Assert("listening on 127.0.0.1 only; the chip says waiting", |s, _| {
            let tcp = &s.companion.tcp;
            if tcp.is_empty() || !tcp.iter().all(|a| a.ip().is_loopback()) {
                return Err(format!("bound {tcp:?}"));
            }
            expect_chip(s, "Tablet: waiting")
        }),
        // (1) Pair in confirm mode, get the KEY frame.
        Step::Setup("connect the test tablet (USB, confirm mode)", start_tablet),
        Step::Until("the tablet has the KEY frame", |_, _| Ok(report()?.key.is_some())),
        Step::WaitFrames(2),
        Step::Assert("the session runs, the tablet is paired, the KEY frame has the window's size", |s, _| {
            expect_chip(s, &format!("Tablet: {TABLET} (USB)"))?;
            if !s.prefs.companion.paired.iter().any(|p| p.name == TABLET) {
                return Err(format!("paired: {:?}", s.prefs.companion.paired));
            }
            let key = report()?.key.ok_or("no KEY frame")?;
            let surface = s.companion.surface;
            if [u32::from(key[0]), u32::from(key[1])] != surface {
                return Err(format!("the KEY frame is {key:?}, the window {surface:?}"));
            }
            Ok(())
        }),
        Step::Do(AppAction::ShowDialog(Dialog::Companion)),
        Step::WaitIdle,
        Step::Shot("companion_dialog"),
        Step::Do(AppAction::CloseDialog),
        Step::WaitIdle,
        // (2) The 2D ramp from the tablet: thin at 20 %, full width at the end.
        Step::Setup("reset the pen probes", reset_probes),
        Step::Setup("tablet: 2D pen ramp, pressure 0.1 -> 1.0", |s, _| uv_row(s, RAMP_V, (0.1, 1.0), 0, true)),
        Step::Until("the tablet's input is confirmed", caught_up),
        Step::WaitIdle,
        Step::Assert("the ramp is thin at 20 % and 40 texels at the end", assert_ramp),
        Step::Assert("the pen was in contact for some frames", |_, sc| {
            let frames = sc.values.get("pen_contact_frames").copied().unwrap_or(0);
            if frames > 0 { Ok(()) } else { Err("no contact frames".to_owned()) }
        }),
        // (3) The eraser end across the ramp: the Eraser while touching, the Brush again after.
        Step::Setup("reset the pen probes", reset_probes),
        Step::Setup("tablet: an eraser-end stroke across the ramp", |s, _| {
            let xf = view_xf(s, ViewId::Main2D)?;
            let screen = [uv_screen(&xf, [0.5, RAMP_V - 0.12]), uv_screen(&xf, [0.5, RAMP_V + 0.12])];
            let points = densify(&screen).into_iter().map(|p| (p, 1.0)).collect();
            send(Cmd::Pen { points, flags: pen_flag::ERASER, lift: true })
        }),
        Step::Until("the tablet's input is confirmed", caught_up),
        Step::WaitIdle,
        Step::Assert("the centre is erased; the Eraser in every contact frame", |s, sc| {
            let a = alpha_at(s, [0.5, RAMP_V])?;
            let (contact, eraser) =
                (sc.values.get("pen_contact_frames").copied().unwrap_or(0), sc.values.get("pen_eraser_frames").copied().unwrap_or(0));
            if a != 0 || contact == 0 || eraser != contact {
                return Err(format!("alpha {a}; the Eraser in {eraser} of {contact} contact frames"));
            }
            if s.doc().and_then(|d| d.undo_label()) != Some("Erase") {
                return Err(format!("the last entry is {:?}", s.doc().and_then(|d| d.undo_label())));
            }
            Ok(())
        }),
        Step::WaitFrames(2),
        Step::Assert("the pen left: the Brush is back", |s, _| {
            if s.tool == ToolKind::Brush { Ok(()) } else { Err(format!("tool {:?}", s.tool)) }
        }),
        // (4) Undo from the tablet's toolbar (ACTION 1): the ramp is whole again.
        Step::Setup("tablet: Undo", |_, _| send(Cmd::Action(1))),
        Step::Until("the tablet's input is confirmed", caught_up),
        Step::WaitIdle,
        Step::Assert("Undo removed the erase stroke", |s, sc| {
            if alpha_at(s, [0.5, RAMP_V])? == 0 {
                return Err("the centre is still erased".to_owned());
            }
            assert_ramp(s, sc)
        }),
        // (5) 3D: the barrel button orbits and paints nothing.
        Step::Do(AppAction::SetViewMode(ViewMode::View3D)),
        Step::WaitIdle,
        Step::Setup("remember the camera and the paint", |s, sc| {
            let cam = camera(s)?;
            sc.values.insert("yaw", u64::from(cam.yaw.to_bits()));
            sc.values.insert("pitch", u64::from(cam.pitch.to_bits()));
            sc.values.insert("distance", u64::from(cam.distance.to_bits()));
            remember_paint(s, sc, "before_barrel")
        }),
        Step::Setup("tablet: a barrel drag over the model", |s, _| {
            let (_, c, r) = model_disc(s, ViewId::Main3D)?;
            let screen = [disc_point(c, r, [-0.4, 0.0]), disc_point(c, r, [0.4, 0.2])];
            send(Cmd::Pen { points: ramp(&screen, (0.8, 0.8)), flags: pen_flag::BARREL, lift: true })
        }),
        Step::Until("the tablet's input is confirmed", caught_up),
        Step::WaitIdle,
        Step::Assert("the camera turned and nothing was painted", |s, sc| {
            let cam = camera(s)?;
            let turned = u64::from(cam.yaw.to_bits()) != remembered(sc, "yaw")? || u64::from(cam.pitch.to_bits()) != remembered(sc, "pitch")?;
            if !turned {
                return Err("the camera did not move".to_owned());
            }
            same_paint(s, sc, "before_barrel")
        }),
        // (6) A two-finger pinch over the model zooms in.
        Step::Setup("tablet: a two-finger pinch out over the model", |s, sc| {
            let (_, c, _) = model_disc(s, ViewId::Main3D)?;
            sc.values.insert("distance", u64::from(camera(s)?.distance.to_bits()));
            let at = |dx: f32| c + egui::vec2(dx, 0.0);
            let mut msgs = vec![vec![(0, touch_phase::DOWN, at(-30.0))], vec![(1, touch_phase::DOWN, at(30.0))]];
            for k in 1..=6 {
                let d = 30.0 + 5.0 * k as f32;
                msgs.push(vec![(0, touch_phase::MOVE, at(-d)), (1, touch_phase::MOVE, at(d))]);
            }
            msgs.push(vec![(0, touch_phase::UP, at(-60.0)), (1, touch_phase::UP, at(60.0))]);
            send(Cmd::Touch(msgs))
        }),
        Step::Until("the tablet's input is confirmed", caught_up),
        Step::WaitIdle,
        Step::Assert("the pinch zoomed in", |s, sc| {
            let (now, then) = (camera(s)?.distance, f32::from_bits(remembered(sc, "distance")? as u32));
            if now < then * 0.95 { Ok(()) } else { Err(format!("camera distance {then} -> {now}")) }
        }),
        Step::Do(AppAction::FrameModel),
        Step::WaitIdle,
        // (7) KEY and TEXT into the Text tool's content field.
        Step::Do(AppAction::SetTool(ToolKind::Text)),
        Step::Setup("empty the content field and focus it", |s, _| {
            s.tools.text.content.clear();
            s.focus_text_content = true;
            Ok(())
        }),
        Step::WaitFrames(3),
        Step::Until("the tablet sees TEXT_WANTED", |_, _| Ok(report()?.flags & state_flag::TEXT_WANTED != 0)),
        Step::Setup("tablet: H (KEY + TEXT), \"i\", Backspace", |_, _| {
            send(Cmd::Key(8, true))?;
            send(Cmd::Text("h"))?;
            send(Cmd::Key(8, false))?;
            send(Cmd::Text("i"))?;
            send(Cmd::Key(42, true))?;
            send(Cmd::Key(42, false))
        }),
        Step::Until("the tablet's input is confirmed", caught_up),
        Step::WaitFrames(2),
        Step::Assert("the content field says \"h\"", |s, _| {
            if s.tools.text.content == "h" { Ok(()) } else { Err(format!("content {:?}", s.tools.text.content)) }
        }),
        Step::Do(AppAction::SetTool(ToolKind::Brush)),
        Step::WaitIdle,
        // (8) The cable is pulled in the middle of a stroke: the stroke ends cleanly, nothing stays held.
        Step::Do(AppAction::SetViewMode(ViewMode::View2D)),
        Step::WaitIdle,
        Step::Setup("remember the undo depth", |s, sc| {
            sc.values.insert("undo_len", doc_of(s)?.history.undo_len() as u64);
            Ok(())
        }),
        Step::Setup("tablet: half a stroke, then the cable is pulled", |s, _| {
            uv_row(s, 0.3, (0.8, 0.8), 0, false)?;
            send(Cmd::Drop)
        }),
        Step::Until("the session ended", |s, _| Ok(s.companion.session.is_none() && s.companion.sessions_ended >= 1)),
        Step::WaitIdle,
        Step::Assert("after the disconnect: no button down, no contact, the stroke committed, the chip waits", |s, sc| {
            if s.companion.pointer_down || s.pen.frame.in_contact {
                return Err(format!("pointer down {}, pen {:?}", s.companion.pointer_down, s.pen.frame));
            }
            if doc_of(s)?.history.undo_len() as u64 != remembered(sc, "undo_len")? + 1 {
                return Err("the cut-off stroke is not one undo step".to_owned());
            }
            if alpha_at(s, [0.3, 0.3])? == 0 {
                return Err("the cut-off stroke painted nothing".to_owned());
            }
            expect_chip(s, "Tablet: waiting")
        }),
        Step::Assert("the tablet got frames; every input it sent before the cut was confirmed", |_, _| {
            let r = report()?;
            if r.frames < 2 || r.key.is_none() {
                return Err(format!("{r:?}"));
            }
            log::info!("companion: the test tablet got {} frames; last confirmed input {}", r.frames, r.confirmed);
            Ok(())
        }),
    ]);
    steps
}

// ---- `companion_host` ----

/// The `companion_host` steps (the dispatcher adds the final screenshot and Exit).
pub(super) fn companion_host() -> Vec<Step> {
    let mut steps = document_setup();
    steps.extend([
        Step::Setup("read WTE_COMPANION_EXPECT_SESSIONS (default 1)", |_, sc| {
            let n = std::env::var("WTE_COMPANION_EXPECT_SESSIONS").ok().and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(1);
            sc.values.insert("expect_sessions", n);
            Ok(())
        }),
        Step::Do(AppAction::Companion(CompanionAction::StartTest(TestServer::from_env(TEST_PORT, false)))),
        Step::WaitFrames(2),
        Step::Assert("listening on 127.0.0.1", |s, _| {
            let tcp = &s.companion.tcp;
            if tcp.is_empty() || !tcp.iter().all(|a| a.ip().is_loopback()) {
                return Err(format!("bound {tcp:?} {:?}", s.companion.errors));
            }
            Ok(())
        }),
        Step::WaitIdle,
        Step::Setup("write the surface coordinates for the tablet test", write_coords),
        Step::Until("the expected sessions drew a pen stroke with varying pressure", |s, sc| {
            let want = remembered(sc, "expect_sessions")?;
            Ok(s.companion.stroke_sessions.len() as u64 >= want)
        }),
        Step::WaitIdle,
        Step::Assert("the tablet's strokes painted", |s, sc| {
            let want = remembered(sc, "expect_sessions")?;
            let painted = paint_count(s)?;
            let steps = doc_of(s)?.history.undo_len() as u64;
            log::info!(
                "companion_host: {} sessions started, {} drew a stroke; {painted} texels painted, {steps} undo steps",
                s.companion.sessions_started,
                s.companion.stroke_sessions.len()
            );
            if want > 0 && (painted == 0 || steps < want) {
                return Err(format!("{painted} texels painted, {steps} undo steps for {want} sessions"));
            }
            Ok(())
        }),
    ]);
    steps
}

/// Logs and writes `<out_stem>_coords.json`: the port, the surface size and the 2D view's centre and stroke row in
/// surface pixels (what the tablet test draws along).
fn write_coords(state: &mut AppState, sc: &mut Scratch) -> Result<(), String> {
    let xf = view_xf(state, ViewId::Main2D)?;
    let ppp = if state.companion.ppp > 0.0 { state.companion.ppp } else { 1.0 };
    let px = |p: egui::Pos2| [(p.x * ppp).round(), (p.y * ppp).round()];
    let centre = px(xf.rect.center());
    let a = px(uv_screen(&xf, [RAMP_U[0], RAMP_V]));
    let b = px(uv_screen(&xf, [RAMP_U[1], RAMP_V]));
    let port = state.companion.tcp.first().map_or(0, |t| t.port());
    let [w, h] = state.companion.surface;
    let json = format!(
        "{{\n  \"port\": {port},\n  \"surface\": [{w}, {h}],\n  \"pixels_per_point\": {ppp},\n  \"view_centre\": [{}, {}],\n  \
         \"stroke_row\": [[{}, {}], [{}, {}]],\n  \"brush_size_texels\": {SIZE}\n}}\n",
        centre[0], centre[1], a[0], a[1], b[0], b[1]
    );
    log::info!(
        "companion_host: port {port}, surface {w}x{h}, 2D view centre {centre:?}, stroke row {a:?} -> {b:?} (surface pixels)"
    );
    let mut path = sc.out_stem.clone().into_os_string();
    path.push("_coords.json");
    std::fs::write(&path, json).map_err(|e| format!("cannot write {}: {e}", Path::new(&path).display()))
}
