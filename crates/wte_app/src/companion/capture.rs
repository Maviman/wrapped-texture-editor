//! The capture scheduler (COMPANION_PLAN 2.3), UI thread: decides when to ask egui-wgpu for a screenshot of the window
//! (`ctx.request_screenshot`, decision C1) and a poller thread that drives the wgpu readback (`device.poll`; nothing in
//! eframe polls). The capture is always of the frame after the request (ARCHITECTURE 8.13 pitfall 5).
//!
//! Rule, every pass while a session streams: a pass that is not the captured one may have changed the picture
//! (`dirty`); a dirty picture is captured when no capture is pending, the writer wants a frame and the frame-rate cap
//! allows it. When the writer's ack window is full the UI says so (`Hub` is woken when the window opens again), so the
//! stream always ends on the latest picture and stops when the app is idle.
//!
//! The cap drops to `IDLE_CAPTURE_INTERVAL` while the captures keep coming out unchanged (about a second of them, as
//! counted by the writer) and no input came in the last `INPUT_RECENT`: an app that repaints without changing (a
//! hovering pen, a latched modifier, the self-test runner) costs two window readbacks a second, not one per frame.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::{egui, wgpu};

use super::server::{Captured, Hub};

/// A pending capture that never arrives is given up after this long (the next pass may ask again).
pub const CAPTURE_GIVE_UP: Duration = Duration::from_secs(1);
/// The poller polls this often while a capture is outstanding, for at most `POLL_FOR`.
const POLL_EVERY: Duration = Duration::from_millis(1);
const POLL_FOR: Duration = Duration::from_millis(250);
/// The capture interval while nothing changes (see the module comment).
pub const IDLE_CAPTURE_INTERVAL: Duration = Duration::from_millis(500);
/// Input (local or remote) this recent keeps the full rate.
pub const INPUT_RECENT: Duration = Duration::from_secs(1);

/// The time between two captures: `1 / fps`, or `IDLE_CAPTURE_INTERVAL` once `empty_captures` (unchanged captures in a
/// row) cover about a second at that rate and no input is recent. Pure (unit-tested).
pub fn capture_interval(fps: u32, empty_captures: u32, input_recent: bool) -> Duration {
    let fps = fps.max(1);
    let normal = Duration::from_secs_f64(1.0 / f64::from(fps));
    if !input_recent && empty_captures >= fps.max(3) { IDLE_CAPTURE_INTERVAL.max(normal) } else { normal }
}

/// Shared between the UI thread, the screenshot callback and the poller.
struct Shared {
    pending: AtomicBool,
    /// The input sequence injected in the captured pass.
    seq: AtomicU32,
    /// The session the capture was asked for (`Captured::session`).
    session: AtomicU64,
    /// A pass after the capture request changed (or may have changed) the picture.
    dirty: AtomicBool,
    stop: AtomicBool,
    /// Measurements for the notes: captures delivered, total and max milliseconds request -> pixels.
    stats: Mutex<(u64, f64, f64)>,
}

/// The poller thread: polls the device while a capture is pending.
struct Poller {
    thread: JoinHandle<()>,
}

/// The UI-thread scheduler.
pub struct Capture {
    shared: Arc<Shared>,
    pass: u64,
    capture_pass: Option<u64>,
    requested_at: Option<Instant>,
    next_allowed: Instant,
    poller: Option<Poller>,
    device: Option<wgpu::Device>,
}

impl Default for Capture {
    fn default() -> Capture {
        Capture::new()
    }
}

impl Capture {
    pub fn new() -> Capture {
        Capture {
            shared: Arc::new(Shared {
                pending: AtomicBool::new(false),
                seq: AtomicU32::new(0),
                session: AtomicU64::new(0),
                dirty: AtomicBool::new(true),
                stop: AtomicBool::new(false),
                stats: Mutex::new((0, 0.0, 0.0)),
            }),
            pass: 0,
            capture_pass: None,
            requested_at: None,
            next_allowed: Instant::now(),
            poller: None,
            device: None,
        }
    }

    /// Captures delivered, mean and max milliseconds from request to pixels.
    pub fn stats(&self) -> (u64, f64, f64) {
        let s = *self.shared.stats.lock().unwrap_or_else(|e| e.into_inner());
        (s.0, if s.0 > 0 { s.1 / s.0 as f64 } else { 0.0 }, s.2)
    }

    /// A new session (or a restore after minimize): the next pass is captured, with no input seq of an older session.
    pub fn reset(&mut self) {
        self.shared.dirty.store(true, Ordering::SeqCst);
        self.shared.seq.store(0, Ordering::SeqCst);
    }

    /// One pass (`Companion::logic`, while a session streams and the window has a picture). `injected_seq` = the last
    /// input sequence injected in this pass's `raw_input_hook`; `session` = the session it belongs to; `input_recent` =
    /// input came within `INPUT_RECENT` (keeps the full rate).
    pub fn pass(&mut self, ctx: &egui::Context, hub: &Arc<Hub>, device: &wgpu::Device, injected_seq: u32, session: u64, input_recent: bool) {
        self.pass += 1;
        let now = Instant::now();
        if self.capture_pass == Some(self.pass) {
            // This pass is the one being captured: its input sequence goes with the picture.
            self.shared.seq.store(injected_seq, Ordering::SeqCst);
        } else {
            self.shared.dirty.store(true, Ordering::SeqCst);
        }
        let pending = self.shared.pending.load(Ordering::SeqCst);
        if pending && self.requested_at.is_some_and(|t| now.saturating_duration_since(t) > CAPTURE_GIVE_UP) {
            log::warn!("companion: a window capture did not arrive within {} ms; asking again", CAPTURE_GIVE_UP.as_millis());
            self.shared.pending.store(false, Ordering::SeqCst);
        }
        if !self.shared.dirty.load(Ordering::SeqCst) || self.shared.pending.load(Ordering::SeqCst) || !hub.want_frame() {
            return;
        }
        if now < self.next_allowed {
            ctx.request_repaint_after(self.next_allowed - now);
            return;
        }
        self.ensure_poller(device);
        self.shared.pending.store(true, Ordering::SeqCst);
        self.shared.dirty.store(false, Ordering::SeqCst);
        self.capture_pass = Some(self.pass + 1);
        self.requested_at = Some(now);
        self.next_allowed = now + capture_interval(hub.fps(), hub.empty_captures(), input_recent);
        self.shared.session.store(session, Ordering::SeqCst);
        let shared = Arc::clone(&self.shared);
        let hub = Arc::clone(hub);
        let ctx2 = ctx.clone();
        let ppp = ctx.pixels_per_point();
        ctx.request_screenshot(move |image| {
            let ms = now.elapsed().as_secs_f64() * 1000.0;
            {
                let mut s = shared.stats.lock().unwrap_or_else(|e| e.into_inner());
                s.0 += 1;
                s.1 += ms;
                s.2 = s.2.max(ms);
            }
            hub.push_frame(Captured {
                session: shared.session.load(Ordering::SeqCst),
                image,
                last_input_seq: shared.seq.load(Ordering::SeqCst),
                pixels_per_point: ppp,
                capture_ms: ms.round() as u32,
            });
            shared.pending.store(false, Ordering::SeqCst);
            if shared.dirty.load(Ordering::SeqCst) {
                ctx2.request_repaint();
            }
        });
        if let Some(p) = &self.poller {
            p.thread.thread().unpark();
        }
    }

    fn ensure_poller(&mut self, device: &wgpu::Device) {
        if self.poller.is_some() {
            return;
        }
        self.device = Some(device.clone());
        let device = device.clone();
        let shared = Arc::clone(&self.shared);
        let thread = super::server::spawn_named("wte-companion-poll", move || {
            while !shared.stop.load(Ordering::SeqCst) {
                if !shared.pending.load(Ordering::SeqCst) {
                    std::thread::park_timeout(Duration::from_millis(100));
                    continue;
                }
                let start = Instant::now();
                while shared.pending.load(Ordering::SeqCst) && start.elapsed() < POLL_FOR && !shared.stop.load(Ordering::SeqCst) {
                    let _ = device.poll(wgpu::PollType::Poll);
                    std::thread::sleep(POLL_EVERY);
                }
                if shared.pending.load(Ordering::SeqCst) {
                    // Not served yet (the request is answered in the next painted frame): wait to be unparked.
                    std::thread::park_timeout(Duration::from_millis(20));
                }
            }
        });
        self.poller = Some(Poller { thread });
    }

    /// Stops the poller (the session or the server ended).
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(p) = self.poller.take() {
            p.thread.thread().unpark();
            let _ = p.thread.join();
        }
        self.shared.stop.store(false, Ordering::SeqCst);
        self.shared.pending.store(false, Ordering::SeqCst);
        self.capture_pass = None;
        self.requested_at = None;
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capture_rate_drops_while_nothing_changes() {
        let at = |fps| Duration::from_secs_f64(1.0 / f64::from(fps));
        assert_eq!(capture_interval(30, 0, false), at(30));
        assert_eq!(capture_interval(30, 29, false), at(30), "less than a second of unchanged captures");
        assert_eq!(capture_interval(30, 30, false), IDLE_CAPTURE_INTERVAL);
        assert_eq!(capture_interval(30, 500, true), at(30), "recent input keeps the full rate");
        assert_eq!(capture_interval(1, 2, false), at(1), "at least three unchanged captures");
        assert_eq!(capture_interval(1, 3, false), at(1), "the idle interval never makes a slow cap faster");
        assert_eq!(capture_interval(0, 0, false), at(1), "fps 0 is 1");
    }
}
