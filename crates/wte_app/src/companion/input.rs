//! Remote input into the app (COMPANION_PLAN 5), pure: the tablet's PEN samples become the same `Event::Touch` +
//! `PenRecord` pairs a local Windows Ink pen gives (5.1, decision C4), so the existing `PenRouter` turns them into
//! pointer events with pressure, eraser and barrel; fingers go through a desktop-side gesture mapper (5.3) and become
//! mouse and wheel events; KEY / TEXT / MODS become key, text and modifier events (5.4). A pointer arbiter (5.2,
//! decision C7) lets whoever pressed first (local or remote) hold the pointer until release.
//!
//! `InputStage::process` runs in `raw_input_hook` after the self-test's or the window's pen records are taken and before
//! `PenState::route`. Without remote messages and without remote modifiers it passes the local events through
//! unchanged; `Companion::inject` does not call it at all without a session.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase};

use super::proto::{self, Msg, pen_flag, pen_kind, touch_phase};
use crate::pen::{PenKind, PenRecord};
use crate::state::FingerMode;

/// The pointer id of the remote pen (its `Touch` events and records).
pub const REMOTE_PEN_ID: u32 = 0x5754_4501;
/// The touch device of the remote pen.
pub const REMOTE_DEVICE: TouchDeviceId = TouchDeviceId(0x5754_4543);
/// Finger travel (points) before a one-finger drag over a view presses (5.3).
pub const DRAG_THRESHOLD: f32 = 6.0;
/// Palm rejection: new finger touches are ignored this long after the pen was in range (5.3).
pub const PALM_HOLD: Duration = Duration::from_millis(300);
/// Pinch: wheel notches per doubling of the finger distance, and the smallest step sent (5.3).
pub const NOTCHES_PER_DOUBLING: f32 = 4.0;
pub const MIN_NOTCH_STEP: f32 = 0.1;
/// The lowest pressure of a remote pen in contact (the router reads 0 as "no pressure known").
pub const MIN_PRESSURE: f32 = 1.0 / 1024.0;

/// Who holds the pointer (5.2).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Holder {
    #[default]
    None,
    Local,
    Remote,
}

/// The frame's context.
pub struct StageCtx<'a> {
    pub ppp: f32,
    /// The editing views (points); empty while a dialog is open (fingers then act as the mouse everywhere).
    pub views: &'a [egui::Rect],
    pub fingers: FingerMode,
    pub now: Instant,
}

/// What a frame's remote input asks of the app besides events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageOut {
    /// ACTION ids, in order (1 undo, 2 redo).
    pub actions: Vec<u16>,
    pub fit: Option<proto::FitRequest>,
    /// The highest input `seq` processed this frame (0 = none).
    pub last_seq: u32,
    /// Any remote input this frame.
    pub had_input: bool,
}

/// The one-finger / two-finger gesture in progress.
#[derive(Clone, Debug, Default, PartialEq)]
enum Gesture {
    #[default]
    Idle,
    One { id: u8, start: Pos2, last: Pos2, over_view: bool, pressed: Option<PointerButton> },
    Two { start: Pos2, last: Pos2, wheel_dist: f32, over_view: bool, middle: bool },
    /// Ignore every finger until all are up (3+ fingers, palm rejection, after a two-finger gesture, held locally).
    Blocked,
}

/// The stage's state across frames.
#[derive(Clone, Debug, Default)]
pub struct InputStage {
    pub holder: Holder,
    /// Local pointer buttons down (bit per button) and a local touch down.
    local_buttons: u8,
    local_touch: bool,
    /// Local releases to drop (their presses were dropped while the remote held the pointer).
    drop_local_releases: u8,
    /// The remote pen: in range, in contact, its last position (points), a contact dropped while held locally.
    pen_in_range: bool,
    pen_contact: bool,
    pen_dropped: bool,
    pen_known: bool,
    pen_last: Option<Pos2>,
    pen_seen: Option<Instant>,
    /// Fingers down (surface pixels converted to points) and the gesture.
    fingers: BTreeMap<u8, Pos2>,
    gesture: Gesture,
    /// Modifiers: local (from passing `ModifiersChanged`), the toolbar latch (MODS), the keys held on the tablet.
    local_mods: Modifiers,
    latched: u8,
    key_mods: u8,
    keys_down: Vec<u16>,
    /// The modifier state egui was last told.
    announced: Modifiers,
    /// The desktop's own pointer (the last local `PointerMoved` that passed; `None` after `PointerGone`), for CURSOR.
    pub local_pointer: Option<Pos2>,
}

fn or(a: Modifiers, b: Modifiers) -> Modifiers {
    Modifiers { alt: a.alt || b.alt, ctrl: a.ctrl || b.ctrl, shift: a.shift || b.shift, mac_cmd: a.mac_cmd || b.mac_cmd, command: a.command || b.command }
}

fn button_bit(b: PointerButton) -> u8 {
    1 << (b as u8).min(7)
}

impl InputStage {
    fn remote_mods(&self) -> Modifiers {
        proto::modifiers_from_bits(self.latched | self.key_mods)
    }

    fn effective(&self) -> Modifiers {
        or(self.local_mods, self.remote_mods())
    }

    /// The remote pen is in range or was within `PALM_HOLD`.
    fn palm(&self, now: Instant) -> bool {
        self.pen_in_range || self.pen_contact || self.pen_seen.is_some_and(|t| now.saturating_duration_since(t) < PALM_HOLD)
    }

    /// True while anything remote is down or in range (the window counts as focused, 5.5).
    pub fn active(&self) -> bool {
        self.pen_in_range || self.pen_contact || !self.fingers.is_empty() || self.holder == Holder::Remote || self.latched != 0 || !self.keys_down.is_empty()
    }

    /// True while the remote holds something down (pen contact, fingers, the pointer, keys): the app keeps frames coming.
    /// A hovering pen or a latched modifier does not (each frame would capture the unchanged window).
    pub fn needs_ticks(&self) -> bool {
        self.pen_contact || !self.fingers.is_empty() || self.holder == Holder::Remote || !self.keys_down.is_empty()
    }

    /// One frame: filters the local `events` (arbiter, modifier merge), then appends the remote messages' events in order;
    /// pen records go to `records` (the pen feed).
    pub fn process(&mut self, events: &mut Vec<Event>, records: &mut VecDeque<PenRecord>, remote: Vec<Msg>, cx: &StageCtx<'_>) -> StageOut {
        let mut out = StageOut::default();
        self.filter_local(events, records);
        let mut ev = Emit { events, records };
        for msg in remote {
            if let Some(seq) = msg.input_seq() {
                out.last_seq = out.last_seq.max(seq);
            }
            out.had_input = true;
            match msg {
                Msg::Pen(p) => {
                    for s in &p.samples {
                        self.pen_sample(s, cx, &mut ev);
                    }
                }
                Msg::Touch(t) => self.touch_msg(&t.points, cx, &mut ev),
                Msg::Key(k) => self.key(&k, &mut ev),
                Msg::Text { text, .. } => {
                    if !text.is_empty() {
                        ev.events.push(Event::Text(text));
                    }
                }
                Msg::Mods { mods, .. } => {
                    self.latched = mods & (proto::mods::CTRL | proto::mods::SHIFT | proto::mods::ALT);
                    self.announce(&mut ev);
                }
                Msg::Action { action, .. } => out.actions.push(action),
                Msg::FitRequest(f) => out.fit = Some(f),
                _ => {}
            }
        }
        out
    }

    /// The local events: modifier merge and the arbiter (5.2).
    fn filter_local(&mut self, events: &mut Vec<Event>, records: &mut VecDeque<PenRecord>) {
        let remote = self.remote_mods();
        if self.holder == Holder::Remote {
            // A local pen must not draw into the remote's stroke: its records go too.
            records.retain(|r| r.pointer_id == REMOTE_PEN_ID);
        }
        events.retain_mut(|e| match e {
            Event::ModifiersChanged(m) => {
                self.local_mods = *m;
                *m = or(*m, remote);
                self.announced = *m;
                true
            }
            Event::PointerButton { button, pressed, modifiers, .. } => {
                let bit = button_bit(*button);
                if *pressed {
                    if self.holder == Holder::Remote {
                        self.drop_local_releases |= bit;
                        return false;
                    }
                    self.local_buttons |= bit;
                    self.holder = Holder::Local;
                } else {
                    if self.drop_local_releases & bit != 0 {
                        self.drop_local_releases &= !bit;
                        return false;
                    }
                    if self.holder == Holder::Remote {
                        return false;
                    }
                    self.local_buttons &= !bit;
                    if self.holder == Holder::Local && self.local_buttons == 0 && !self.local_touch {
                        self.holder = Holder::None;
                    }
                }
                *modifiers = or(*modifiers, remote);
                true
            }
            Event::Touch { phase, .. } => {
                if self.holder == Holder::Remote {
                    return false;
                }
                match phase {
                    TouchPhase::Start => {
                        self.local_touch = true;
                        self.holder = Holder::Local;
                    }
                    TouchPhase::End | TouchPhase::Cancel => {
                        self.local_touch = false;
                        if self.holder == Holder::Local && self.local_buttons == 0 {
                            self.holder = Holder::None;
                        }
                    }
                    TouchPhase::Move => {}
                }
                true
            }
            Event::PointerMoved(p) => {
                let keep = self.holder != Holder::Remote;
                if keep {
                    self.local_pointer = Some(*p);
                }
                keep
            }
            Event::PointerGone => {
                let keep = self.holder != Holder::Remote;
                if keep {
                    self.local_pointer = None;
                }
                keep
            }
            Event::MouseMoved(_) | Event::MouseWheel { .. } => self.holder != Holder::Remote,
            _ => true,
        });
    }

    /// Tells egui the effective modifiers when they differ from what it was last told.
    fn announce(&mut self, ev: &mut Emit<'_>) {
        let m = self.effective();
        if m != self.announced {
            ev.events.push(Event::ModifiersChanged(m));
            self.announced = m;
        }
    }

    fn pen_record(&self, kind: PenKind, in_contact: bool, pressure: f32, flags: u8) -> PenRecord {
        PenRecord {
            pointer_id: REMOTE_PEN_ID,
            kind,
            in_contact,
            pressure,
            barrel: flags & pen_flag::BARREL != 0,
            third: flags & pen_flag::BARREL2 != 0,
            inverted: flags & pen_flag::ERASER != 0,
        }
    }

    fn pen_touch(phase: TouchPhase, pos: Pos2, force: Option<f32>) -> Event {
        Event::Touch { device_id: REMOTE_DEVICE, id: TouchId(u64::from(REMOTE_PEN_ID)), phase, pos, force }
    }

    /// One PEN sample (5.1 table).
    fn pen_sample(&mut self, s: &proto::PenSample, cx: &StageCtx<'_>, ev: &mut Emit<'_>) {
        let ppp = if cx.ppp.is_finite() && cx.ppp > 0.0 { cx.ppp } else { 1.0 };
        let pos = Pos2::new(s.x / ppp, s.y / ppp);
        if !(pos.x.is_finite() && pos.y.is_finite()) {
            return;
        }
        let p = (f32::from(s.pressure) / 65535.0).max(MIN_PRESSURE);
        // The pen in range ends a finger gesture (palm rejection).
        if s.kind != pen_kind::LEAVE {
            self.release_gesture(ev);
            self.gesture = if self.fingers.is_empty() { Gesture::Idle } else { Gesture::Blocked };
        }
        self.pen_seen = Some(cx.now);
        match s.kind {
            pen_kind::HOVER => {
                if self.holder == Holder::Local || self.pen_contact {
                    return;
                }
                self.pen_in_range = true;
                self.pen_known = true;
                self.pen_last = Some(pos);
                ev.events.push(Self::pen_touch(TouchPhase::Move, pos, None));
                ev.records.push_back(self.pen_record(PenKind::Update, false, 0.0, s.flags));
            }
            pen_kind::DOWN => {
                self.pen_in_range = true;
                if self.pen_contact {
                    // A down while in contact: treat as a move.
                    return self.pen_move(pos, p, s.flags, ev);
                }
                if self.holder != Holder::None {
                    self.pen_dropped = true;
                    return;
                }
                self.announce(ev);
                self.holder = Holder::Remote;
                self.pen_contact = true;
                self.pen_known = true;
                self.pen_last = Some(pos);
                ev.events.push(Self::pen_touch(TouchPhase::Start, pos, Some(p)));
                ev.records.push_back(self.pen_record(PenKind::Down, true, p, s.flags));
            }
            pen_kind::MOVE => {
                self.pen_in_range = true;
                if self.pen_contact {
                    self.pen_move(pos, p, s.flags, ev);
                }
            }
            pen_kind::UP => {
                self.pen_in_range = true;
                self.pen_dropped = false;
                if self.pen_contact {
                    self.pen_last = Some(pos);
                    ev.events.push(Self::pen_touch(TouchPhase::End, pos, None));
                    ev.records.push_back(self.pen_record(PenKind::Up, false, 0.0, s.flags));
                    self.pen_contact = false;
                    self.holder = Holder::None;
                }
            }
            pen_kind::LEAVE => {
                self.pen_dropped = false;
                if self.pen_known {
                    ev.records.push_back(self.pen_record(PenKind::Leave, false, 0.0, s.flags));
                }
                if self.pen_contact {
                    self.holder = Holder::None;
                }
                self.pen_contact = false;
                self.pen_in_range = false;
                self.pen_known = false;
            }
            pen_kind::CANCEL => {
                self.pen_dropped = false;
                if self.pen_contact {
                    let at = self.pen_last.unwrap_or(pos);
                    ev.events.push(Self::pen_touch(TouchPhase::End, at, None));
                    ev.records.push_back(self.pen_record(PenKind::Up, false, 0.0, s.flags));
                    self.holder = Holder::None;
                }
                if self.pen_known {
                    ev.records.push_back(self.pen_record(PenKind::Leave, false, 0.0, s.flags));
                }
                self.pen_contact = false;
                self.pen_in_range = false;
                self.pen_known = false;
            }
            _ => {}
        }
    }

    fn pen_move(&mut self, pos: Pos2, p: f32, flags: u8, ev: &mut Emit<'_>) {
        self.pen_last = Some(pos);
        ev.events.push(Self::pen_touch(TouchPhase::Move, pos, Some(p)));
        ev.records.push_back(self.pen_record(PenKind::Update, true, p, flags));
    }

    fn over_view(&self, pos: Pos2, cx: &StageCtx<'_>) -> bool {
        cx.fingers == FingerMode::Navigate && cx.views.iter().any(|r| r.contains(pos))
    }

    fn press(&mut self, at: Pos2, button: PointerButton, ev: &mut Emit<'_>) {
        self.announce(ev);
        ev.events.push(Event::PointerMoved(at));
        ev.events.push(Event::PointerButton { pos: at, button, pressed: true, modifiers: self.effective() });
        self.holder = Holder::Remote;
    }

    fn release(&mut self, at: Pos2, button: PointerButton, ev: &mut Emit<'_>) {
        ev.events.push(Event::PointerButton { pos: at, button, pressed: false, modifiers: self.effective() });
        if !self.pen_contact {
            self.holder = Holder::None;
        }
    }

    /// Releases the button a gesture holds (at its last position).
    fn release_gesture(&mut self, ev: &mut Emit<'_>) {
        match std::mem::take(&mut self.gesture) {
            Gesture::One { last, pressed: Some(b), .. } => self.release(last, b, ev),
            Gesture::Two { last, middle: true, .. } => self.release(last, PointerButton::Middle, ev),
            other => self.gesture = other,
        }
    }

    fn centroid(&self) -> (Pos2, f32) {
        let pts: Vec<Pos2> = self.fingers.values().copied().collect();
        let n = pts.len().max(1) as f32;
        let c = pts.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / n;
        let d = if pts.len() >= 2 { pts[0].distance(pts[1]) } else { 0.0 };
        (c.to_pos2(), d)
    }

    /// One TOUCH message (5.3): moves update the finger positions and the gesture is evaluated once for all of them
    /// (a symmetric pinch keeps its centroid); downs and ups are handled in order, after the moves before them.
    fn touch_msg(&mut self, points: &[proto::TouchPoint], cx: &StageCtx<'_>, ev: &mut Emit<'_>) {
        let ppp = if cx.ppp.is_finite() && cx.ppp > 0.0 { cx.ppp } else { 1.0 };
        let mut moved: Option<Pos2> = None;
        for t in points {
            let pos = Pos2::new(t.x / ppp, t.y / ppp);
            if !(pos.x.is_finite() && pos.y.is_finite()) {
                continue;
            }
            if t.phase == touch_phase::MOVE {
                if let Some(f) = self.fingers.get_mut(&t.id) {
                    *f = pos;
                    if matches!(self.gesture, Gesture::One { id, .. } if id == t.id) || matches!(self.gesture, Gesture::Two { .. }) {
                        moved = Some(pos);
                    }
                }
                continue;
            }
            if let Some(p) = moved.take() {
                self.gesture_move(p, ev);
            }
            self.touch_point(t, pos, cx, ev);
        }
        if let Some(p) = moved {
            self.gesture_move(p, ev);
        }
    }

    /// One TOUCH down / up / cancel (5.3).
    fn touch_point(&mut self, t: &proto::TouchPoint, pos: Pos2, cx: &StageCtx<'_>, ev: &mut Emit<'_>) {
        match t.phase {
            touch_phase::DOWN => {
                self.fingers.insert(t.id, pos);
                let n = self.fingers.len();
                if matches!(self.gesture, Gesture::Blocked) {
                    return;
                }
                if self.palm(cx.now) || self.holder == Holder::Local || n >= 3 {
                    self.release_gesture(ev);
                    self.gesture = Gesture::Blocked;
                    return;
                }
                if n == 1 {
                    let over = self.over_view(pos, cx);
                    let mut pressed = None;
                    if !over {
                        self.press(pos, PointerButton::Primary, ev);
                        pressed = Some(PointerButton::Primary);
                    }
                    self.gesture = Gesture::One { id: t.id, start: pos, last: pos, over_view: over, pressed };
                } else {
                    // One finger -> two: the one-finger button is released first.
                    self.release_gesture(ev);
                    let (c, d) = self.centroid();
                    self.gesture = Gesture::Two { start: c, last: c, wheel_dist: d.max(1.0), over_view: self.over_view(c, cx), middle: false };
                }
            }
            touch_phase::UP | touch_phase::CANCEL => {
                if self.fingers.remove(&t.id).is_none() {
                    return;
                }
                match &self.gesture {
                    Gesture::One { id, .. } if *id == t.id => {
                        if let Gesture::One { pressed: Some(b), .. } = self.gesture {
                            let at = pos;
                            self.gesture = Gesture::Idle;
                            if t.phase == touch_phase::CANCEL {
                                // Released where the gesture was, without a final move.
                                ev.events.push(Event::PointerGone);
                            }
                            self.release(at, b, ev);
                        } else {
                            self.gesture = Gesture::Idle;
                        }
                    }
                    Gesture::Two { .. } => {
                        self.release_gesture(ev);
                        self.gesture = Gesture::Blocked;
                    }
                    _ => {}
                }
                if self.fingers.is_empty() {
                    self.gesture = Gesture::Idle;
                }
            }
            _ => {}
        }
    }

    /// The gesture after the fingers moved (`pos` = the moved finger of a one-finger gesture).
    fn gesture_move(&mut self, pos: Pos2, ev: &mut Emit<'_>) {
        let (c, d) = self.centroid();
        let effective = self.effective();
        // What to press after the match (pressing needs `self` again).
        let mut press: Option<(Pos2, PointerButton)> = None;
        let mut after: Vec<Event> = Vec::new();
        match &mut self.gesture {
            Gesture::One { start, last, over_view, pressed, .. } => {
                *last = pos;
                match pressed {
                    Some(_) => after.push(Event::PointerMoved(pos)),
                    None if *over_view && start.distance(pos) >= DRAG_THRESHOLD => {
                        *pressed = Some(PointerButton::Secondary);
                        press = Some((*start, PointerButton::Secondary));
                        after.push(Event::PointerMoved(pos));
                    }
                    None => {}
                }
            }
            Gesture::Two { start, last, wheel_dist, over_view, middle } => {
                let delta = c - *last;
                *last = c;
                if *over_view {
                    // Pinch: wheel notches at the centroid.
                    if d > 0.0 && *wheel_dist > 0.0 {
                        let notches = (d / *wheel_dist).log2() * NOTCHES_PER_DOUBLING;
                        if notches.abs() >= MIN_NOTCH_STEP {
                            *wheel_dist = d;
                            ev.events.push(Event::PointerMoved(c));
                            ev.events.push(Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Line,
                                delta: egui::vec2(0.0, notches),
                                phase: TouchPhase::Move,
                                source: egui::MouseWheelSource::Wheel,
                                modifiers: effective,
                            });
                        }
                    }
                    // Pan: Middle drag at the centroid once it travelled.
                    if *middle {
                        after.push(Event::PointerMoved(c));
                    } else if start.distance(c) >= DRAG_THRESHOLD {
                        *middle = true;
                        press = Some((*start, PointerButton::Middle));
                        after.push(Event::PointerMoved(c));
                    }
                } else if delta != egui::Vec2::ZERO {
                    ev.events.push(Event::PointerMoved(c));
                    ev.events.push(Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta,
                        phase: TouchPhase::Move,
                        source: egui::MouseWheelSource::Trackpad,
                        modifiers: effective,
                    });
                }
            }
            _ => {}
        }
        if let Some((at, b)) = press {
            self.press(at, b, ev);
        }
        ev.events.extend(after);
    }

    /// KEY: an egui key event with the effective modifiers; Ctrl/Shift/Alt of the key message hold while a remote key
    /// is down.
    fn key(&mut self, k: &proto::Key, ev: &mut Emit<'_>) {
        let Some(key) = proto::key_from_code(k.key) else { return };
        if k.pressed {
            if !self.keys_down.contains(&k.key) {
                self.keys_down.push(k.key);
            }
            self.key_mods = k.mods;
        }
        self.announce(ev);
        ev.events.push(Event::Key { key, physical_key: None, pressed: k.pressed, repeat: k.repeat, modifiers: self.effective() });
        if !k.pressed {
            self.keys_down.retain(|c| *c != k.key);
            if self.keys_down.is_empty() {
                self.key_mods = 0;
                self.announce(ev);
            }
        }
    }

    /// End of a session, for any reason (5.6): the pen leaves (the router releases an open contact), a gesture button
    /// is released, remote modifiers go to 0, a remote hold ends.
    pub fn end_session(&mut self, events: &mut Vec<Event>, records: &mut VecDeque<PenRecord>) {
        let mut ev = Emit { events, records };
        if self.pen_known || self.pen_contact {
            ev.records.push_back(self.pen_record(PenKind::Leave, false, 0.0, 0));
        }
        self.pen_known = false;
        self.pen_contact = false;
        self.pen_in_range = false;
        self.pen_dropped = false;
        self.pen_seen = None;
        self.release_gesture(&mut ev);
        self.gesture = Gesture::Idle;
        self.fingers.clear();
        self.latched = 0;
        self.key_mods = 0;
        for code in std::mem::take(&mut self.keys_down) {
            if let Some(key) = proto::key_from_code(code) {
                ev.events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: self.local_mods });
            }
        }
        self.announce(&mut ev);
        if self.holder == Holder::Remote {
            self.holder = Holder::None;
        }
    }
}

/// Where the stage writes.
struct Emit<'a> {
    events: &'a mut Vec<Event>,
    records: &'a mut VecDeque<PenRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pen::PenRouter;
    use proto::{Pen, PenSample, Touch, TouchPoint};

    fn cx(ppp: f32, views: &[egui::Rect]) -> StageCtx<'_> {
        StageCtx { ppp, views, fingers: FingerMode::Navigate, now: Instant::now() }
    }

    fn sample(kind: u8, x: f32, p: f32, flags: u8) -> PenSample {
        PenSample { kind, flags, pressure: (p * 65535.0).round() as u16, x, y: 20.0, tilt: 255, reserved: 0, dt_us: 1000 }
    }

    fn pen(seq: u32, samples: Vec<PenSample>) -> Msg {
        Msg::Pen(Pen { seq, samples })
    }

    /// Runs remote messages through the stage and the router; returns the routed events and the pressure notes.
    fn route(stage: &mut InputStage, router: &mut PenRouter, local: Vec<Event>, remote: Vec<Msg>, c: &StageCtx<'_>) -> (Vec<Event>, Vec<Option<f32>>, crate::pen::PenFrame) {
        let mut events = local;
        let mut feed = VecDeque::new();
        stage.process(&mut events, &mut feed, remote, c);
        let frame = router.process(&mut events, &mut feed);
        let notes = frame.pointer.iter().map(|(_, p)| *p).collect();
        (events, notes, frame)
    }

    fn moved(x: f32, y: f32) -> Event {
        Event::PointerMoved(egui::pos2(x, y))
    }

    fn button(x: f32, y: f32, b: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos: egui::pos2(x, y), button: b, pressed, modifiers: Modifiers::NONE }
    }

    #[test]
    fn a_remote_stroke_routes_like_a_local_pen() {
        let mut stage = InputStage::default();
        let mut router = PenRouter::default();
        let c = cx(1.0, &[]);
        let samples = vec![
            sample(pen_kind::HOVER, 10.0, 0.0, 0),
            sample(pen_kind::DOWN, 10.0, 0.25, 0),
            sample(pen_kind::MOVE, 12.0, 0.5, 0),
            sample(pen_kind::MOVE, 14.0, 0.75, 0),
            sample(pen_kind::UP, 14.0, 0.0, 0),
            sample(pen_kind::LEAVE, 14.0, 0.0, 0),
        ];
        let (events, notes, frame) = route(&mut stage, &mut router, Vec::new(), vec![pen(1, samples)], &c);
        assert!(!events.iter().any(|e| matches!(e, Event::Touch { .. })), "pen touches are consumed by the router");
        assert_eq!(
            events,
            vec![
                moved(10.0, 20.0),
                moved(10.0, 20.0),
                button(10.0, 20.0, PointerButton::Primary, true),
                moved(12.0, 20.0),
                moved(14.0, 20.0),
                button(14.0, 20.0, PointerButton::Primary, false),
                Event::PointerGone,
            ]
        );
        let p = |v: f32| Some((v * 65535.0).round() / 65535.0);
        assert_eq!(notes, vec![None, p(0.25), p(0.25), p(0.5), p(0.75), p(0.75)]);
        assert!(!frame.in_range && !frame.in_contact && frame.seen);
        assert_eq!(stage.holder, Holder::None);
    }

    #[test]
    fn coordinates_follow_pixels_per_point() {
        for ppp in [1.0f32, 1.25, 2.0] {
            let mut stage = InputStage::default();
            let mut events = Vec::new();
            let mut feed = VecDeque::new();
            stage.process(&mut events, &mut feed, vec![pen(1, vec![sample(pen_kind::DOWN, 100.0, 1.0, 0)])], &cx(ppp, &[]));
            assert!(
                matches!(events[0], Event::Touch { pos, phase: TouchPhase::Start, force: Some(f), .. } if pos == egui::pos2(100.0 / ppp, 20.0 / ppp) && f == 1.0),
                "{ppp}: {:?}",
                events[0]
            );
        }
    }

    #[test]
    fn eraser_barrel_hover_cancel_and_pressure_floor() {
        let mut stage = InputStage::default();
        let mut router = PenRouter::default();
        let c = cx(1.0, &[]);
        // Hover with the eraser end: in range, inverted, no contact.
        let (events, _, frame) = route(&mut stage, &mut router, Vec::new(), vec![pen(1, vec![sample(pen_kind::HOVER, 5.0, 0.0, pen_flag::ERASER)])], &c);
        assert_eq!(events, vec![moved(5.0, 20.0)]);
        assert!(frame.in_range && frame.inverted && !frame.in_contact);
        // Barrel down: Secondary; pressure 0 in contact reads the floor.
        let (events, notes, frame) =
            route(&mut stage, &mut router, Vec::new(), vec![pen(2, vec![sample(pen_kind::DOWN, 5.0, 0.0, pen_flag::BARREL)])], &c);
        assert_eq!(events[1], button(5.0, 20.0, PointerButton::Secondary, true));
        assert_eq!(notes[1], Some(MIN_PRESSURE));
        assert!(frame.barrel && frame.in_contact);
        // Cancel: the release at the last position, then out of range.
        let (events, _, frame) = route(&mut stage, &mut router, Vec::new(), vec![pen(3, vec![sample(pen_kind::CANCEL, 50.0, 0.0, pen_flag::BARREL)])], &c);
        assert_eq!(events, vec![button(5.0, 20.0, PointerButton::Secondary, false), Event::PointerGone]);
        assert!(!frame.in_range && !frame.in_contact);
        assert_eq!(stage.holder, Holder::None);
        // Second barrel button = Middle.
        let (events, _, _) = route(&mut stage, &mut router, Vec::new(), vec![pen(4, vec![sample(pen_kind::DOWN, 5.0, 0.5, pen_flag::BARREL2)])], &c);
        assert_eq!(events[1], button(5.0, 20.0, PointerButton::Middle, true));
    }

    #[test]
    fn the_arbiter_lets_the_first_press_hold() {
        let c = cx(1.0, &[]);
        // Local first: the remote stroke is dropped, hover too; then free again.
        let mut stage = InputStage::default();
        let mut router = PenRouter::default();
        let (events, _, _) = route(&mut stage, &mut router, vec![moved(1.0, 1.0), button(1.0, 1.0, PointerButton::Primary, true)], Vec::new(), &c);
        assert_eq!(events.len(), 2);
        assert_eq!(stage.holder, Holder::Local);
        let remote = vec![pen(1, vec![sample(pen_kind::HOVER, 9.0, 0.0, 0), sample(pen_kind::DOWN, 9.0, 0.5, 0), sample(pen_kind::MOVE, 10.0, 0.5, 0)])];
        let (events, _, _) = route(&mut stage, &mut router, vec![moved(2.0, 1.0)], remote, &c);
        assert_eq!(events, vec![moved(2.0, 1.0)], "the remote stroke is dropped while the mouse holds");
        let (events, _, _) = route(&mut stage, &mut router, vec![button(2.0, 1.0, PointerButton::Primary, false)], vec![pen(2, vec![sample(pen_kind::UP, 10.0, 0.0, 0)])], &c);
        assert_eq!(events, vec![button(2.0, 1.0, PointerButton::Primary, false)]);
        assert_eq!(stage.holder, Holder::None);
        // Remote first: local pointer events are dropped, the dropped press's release too, keys pass.
        let (_, _, _) = route(&mut stage, &mut router, Vec::new(), vec![pen(3, vec![sample(pen_kind::DOWN, 30.0, 0.5, 0)])], &c);
        assert_eq!(stage.holder, Holder::Remote);
        let key = Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let local = vec![moved(3.0, 3.0), button(3.0, 3.0, PointerButton::Primary, true), key.clone(), Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        }];
        let (events, _, _) = route(&mut stage, &mut router, local, vec![pen(4, vec![sample(pen_kind::MOVE, 31.0, 0.5, 0)])], &c);
        assert_eq!(events, vec![key, moved(31.0, 20.0)]);
        let (events, _, _) = route(&mut stage, &mut router, Vec::new(), vec![pen(5, vec![sample(pen_kind::UP, 31.0, 0.0, 0)])], &c);
        assert!(events.contains(&button(31.0, 20.0, PointerButton::Primary, false)));
        assert_eq!(stage.holder, Holder::None);
        let (events, _, _) = route(&mut stage, &mut router, vec![button(3.0, 3.0, PointerButton::Primary, false), moved(4.0, 4.0)], Vec::new(), &c);
        assert_eq!(events, vec![moved(4.0, 4.0)], "the release of the dropped press is dropped too");
        assert_eq!(stage.holder, Holder::None);
        // Hover from both sides while nobody holds: both pass, remote after local.
        let (events, _, _) = route(&mut stage, &mut router, vec![moved(5.0, 5.0)], vec![pen(6, vec![sample(pen_kind::HOVER, 40.0, 0.0, 0)])], &c);
        assert_eq!(events, vec![moved(5.0, 5.0), moved(40.0, 20.0)]);
        // A local pen's records are dropped while the remote holds.
        let mut stage = InputStage { holder: Holder::Remote, ..InputStage::default() };
        let mut feed: VecDeque<PenRecord> = [PenRecord { pointer_id: 77, kind: PenKind::Down, in_contact: true, pressure: 1.0, barrel: false, third: false, inverted: false }].into();
        stage.process(&mut Vec::new(), &mut feed, Vec::new(), &c);
        assert!(feed.is_empty());
    }

    #[test]
    fn modifiers_merge_and_keys() {
        let c = cx(1.0, &[]);
        let mut stage = InputStage::default();
        let mut events = vec![Event::ModifiersChanged(Modifiers::SHIFT)];
        stage.process(&mut events, &mut VecDeque::new(), vec![Msg::Mods { seq: 1, mods: proto::mods::CTRL }], &c);
        let ctrl_shift = Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE };
        assert_eq!(events, vec![Event::ModifiersChanged(Modifiers::SHIFT), Event::ModifiersChanged(ctrl_shift)]);
        // A later local change keeps the latch.
        let mut events = vec![Event::ModifiersChanged(Modifiers::NONE)];
        stage.process(&mut events, &mut VecDeque::new(), Vec::new(), &c);
        assert_eq!(events, vec![Event::ModifiersChanged(Modifiers { ctrl: true, command: true, ..Modifiers::NONE })]);
        // Unlatched: back to the local state.
        let mut events = Vec::new();
        stage.process(&mut events, &mut VecDeque::new(), vec![Msg::Mods { seq: 2, mods: 0 }], &c);
        assert_eq!(events, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        // Ctrl+Z from the tablet's keyboard, then the release; TEXT; an unknown key code is dropped.
        let mut events = Vec::new();
        let msgs = vec![
            Msg::Key(proto::Key { seq: 3, key: 26, pressed: true, repeat: false, mods: proto::mods::CTRL }),
            Msg::Key(proto::Key { seq: 4, key: 26, pressed: false, repeat: false, mods: proto::mods::CTRL }),
            Msg::Text { seq: 5, text: "a".into() },
            Msg::Key(proto::Key { seq: 6, key: 999, pressed: true, repeat: false, mods: 0 }),
        ];
        let out = stage.process(&mut events, &mut VecDeque::new(), msgs, &c);
        let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        assert_eq!(
            events,
            vec![
                Event::ModifiersChanged(ctrl),
                Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: ctrl },
                Event::Key { key: egui::Key::Z, physical_key: None, pressed: false, repeat: false, modifiers: ctrl },
                Event::ModifiersChanged(Modifiers::NONE),
                Event::Text("a".into()),
            ]
        );
        assert_eq!(out.last_seq, 6);
        // ACTION and FIT_REQUEST come back to the app.
        let out = stage.process(
            &mut Vec::new(),
            &mut VecDeque::new(),
            vec![Msg::Action { seq: 7, action: 1 }, Msg::Action { seq: 8, action: 9 }, Msg::FitRequest(proto::FitRequest { mode: 1, aspect_w: 16, aspect_h: 10 })],
            &c,
        );
        assert_eq!(out.actions, vec![1, 9]);
        assert_eq!(out.fit.map(|f| f.mode), Some(1));
    }

    fn touch(seq: u32, pts: &[(u8, u8, f32, f32)]) -> Msg {
        Msg::Touch(Touch { seq, points: pts.iter().map(|&(id, phase, x, y)| TouchPoint { id, phase, x, y }).collect() })
    }

    fn run(stage: &mut InputStage, msgs: Vec<Msg>, c: &StageCtx<'_>) -> Vec<Event> {
        let mut events = Vec::new();
        stage.process(&mut events, &mut VecDeque::new(), msgs, c);
        events
    }

    /// Stickers P6a (STICKER_PLAN 3.10, review A16): with the sticker canvas registered, one finger over it is a
    /// Secondary drag and the canvas pans; two fingers pinch-zoom it; "act like the mouse" gives the Primary button,
    /// which the Move tool takes. The mapper itself is unchanged: it gets the canvas's rect through `finger_views`.
    #[test]
    fn fingers_over_the_sticker_canvas_pan_zoom_or_act_like_the_mouse() {
        use crate::sticker::tests::{CANVAS, canvas_pass, canvas_state};
        let (mut state, _, geom) = canvas_state();
        state.sticker_canvas_rect = Some(CANVAS);
        let views = crate::companion::finger_views(&state);
        assert!(views.contains(&CANVAS));
        let c = cx(1.0, &views);
        let ctx = egui::Context::default();
        let hit = crate::sticker::canvas::Hit::Image;
        let mut stage = InputStage::default();
        let camera = |s: &crate::state::AppState| crate::sticker::edit_of(s).unwrap().camera;
        // One finger dragging over the canvas: a Secondary drag, the canvas pans.
        let before = camera(&state);
        let e = run(&mut stage, vec![touch(1, &[(0, 0, 300.0, 300.0)]), touch(2, &[(0, 1, 320.0, 300.0)])], &c);
        assert!(e.contains(&button(300.0, 300.0, PointerButton::Secondary, true)), "{e:?}");
        canvas_pass(&ctx, &mut state, geom, hit, e);
        let e = run(&mut stage, vec![touch(3, &[(0, 1, 340.0, 310.0)])], &c);
        canvas_pass(&ctx, &mut state, geom, hit, e);
        let e = run(&mut stage, vec![touch(4, &[(0, 2, 340.0, 310.0)])], &c);
        canvas_pass(&ctx, &mut state, geom, hit, e);
        let after = camera(&state);
        assert!(after.center_uv != before.center_uv && after.zoom == before.zoom, "{before:?} -> {after:?}");
        assert!(state.sticker_ui.drag.is_none());
        // Two fingers pinching out: wheel notches at the centroid, the canvas zooms in.
        let before = camera(&state);
        run(&mut stage, vec![touch(5, &[(1, 0, 250.0, 300.0), (2, 0, 350.0, 300.0)])], &c);
        let e = run(&mut stage, vec![touch(6, &[(1, 1, 200.0, 300.0), (2, 1, 400.0, 300.0)])], &c);
        canvas_pass(&ctx, &mut state, geom, hit, e);
        run(&mut stage, vec![touch(7, &[(1, 2, 200.0, 300.0), (2, 2, 400.0, 300.0)])], &c);
        assert!(camera(&state).zoom > before.zoom, "{:?} -> {:?}", before.zoom, camera(&state).zoom);
        // "Act like the mouse": one finger is the Primary button: the Move tool starts a drag of the layer.
        let mouse = StageCtx { fingers: FingerMode::Mouse, ..cx(1.0, &views) };
        let e = run(&mut stage, vec![touch(8, &[(3, 0, 300.0, 300.0)])], &mouse);
        assert!(e.contains(&button(300.0, 300.0, PointerButton::Primary, true)), "{e:?}");
        canvas_pass(&ctx, &mut state, geom, hit, e);
        assert!(matches!(state.sticker_ui.drag, Some(crate::sticker::CanvasDrag::Move { .. })), "{:?}", state.sticker_ui.drag);
        let e = run(&mut stage, vec![touch(9, &[(3, 2, 300.0, 300.0)])], &mouse);
        canvas_pass(&ctx, &mut state, geom, hit, e);
        assert!(state.sticker_ui.drag.is_none());
    }

    #[test]
    fn one_finger_gestures() {
        let view = [egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(500.0, 500.0))];
        let c = cx(1.0, &view);
        let mut stage = InputStage::default();
        // A tap in the view does nothing.
        assert!(run(&mut stage, vec![touch(1, &[(0, 0, 100.0, 100.0)]), touch(2, &[(0, 2, 101.0, 100.0)])], &c).is_empty());
        // A drag in the view: Secondary after 6 points of travel.
        let e = run(&mut stage, vec![touch(3, &[(0, 0, 100.0, 100.0)]), touch(4, &[(0, 1, 103.0, 100.0)])], &c);
        assert!(e.is_empty(), "below the threshold: {e:?}");
        let e = run(&mut stage, vec![touch(5, &[(0, 1, 110.0, 100.0)])], &c);
        assert_eq!(e, vec![moved(100.0, 100.0), button(100.0, 100.0, PointerButton::Secondary, true), moved(110.0, 100.0)]);
        assert_eq!(stage.holder, Holder::Remote);
        let e = run(&mut stage, vec![touch(6, &[(0, 2, 120.0, 100.0)])], &c);
        assert_eq!(e, vec![button(120.0, 100.0, PointerButton::Secondary, false)]);
        assert_eq!(stage.holder, Holder::None);
        // Outside the view: Primary at once (a click on a panel), drag, release.
        let e = run(&mut stage, vec![touch(7, &[(1, 0, 600.0, 50.0)]), touch(8, &[(1, 1, 610.0, 50.0)]), touch(9, &[(1, 2, 610.0, 50.0)])], &c);
        assert_eq!(
            e,
            vec![
                moved(600.0, 50.0),
                button(600.0, 50.0, PointerButton::Primary, true),
                moved(610.0, 50.0),
                button(610.0, 50.0, PointerButton::Primary, false)
            ]
        );
        // "Act like the mouse everywhere": Primary in the view too.
        let mouse = StageCtx { fingers: FingerMode::Mouse, ..cx(1.0, &view) };
        let e = run(&mut stage, vec![touch(10, &[(2, 0, 50.0, 50.0)]), touch(11, &[(2, 2, 50.0, 50.0)])], &mouse);
        assert_eq!(e, vec![moved(50.0, 50.0), button(50.0, 50.0, PointerButton::Primary, true), button(50.0, 50.0, PointerButton::Primary, false)]);
        // Cancel releases without a final move.
        let e = run(&mut stage, vec![touch(12, &[(3, 0, 600.0, 60.0)]), touch(13, &[(3, 3, 650.0, 60.0)])], &c);
        assert_eq!(e[2..], [Event::PointerGone, button(650.0, 60.0, PointerButton::Primary, false)]);
    }

    #[test]
    fn two_finger_pan_pinch_and_the_change_from_one_finger() {
        let view = [egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(500.0, 500.0))];
        let c = cx(1.0, &view);
        let mut stage = InputStage::default();
        // One finger dragging (Secondary), then a second finger: the Secondary is released first.
        run(&mut stage, vec![touch(1, &[(0, 0, 100.0, 100.0)]), touch(2, &[(0, 1, 120.0, 100.0)])], &c);
        let e = run(&mut stage, vec![touch(3, &[(1, 0, 220.0, 100.0)])], &c);
        assert_eq!(e, vec![button(120.0, 100.0, PointerButton::Secondary, false)]);
        // Pinch out to double the distance: +4 notches at the centroid (in steps).
        let e = run(&mut stage, vec![touch(4, &[(0, 1, 70.0, 100.0), (1, 1, 270.0, 100.0)])], &c);
        let notches: f32 = e.iter().filter_map(|ev| if let Event::MouseWheel { delta, unit: egui::MouseWheelUnit::Line, .. } = ev { Some(delta.y) } else { None }).sum();
        assert!((notches - 4.0).abs() < 1e-3, "{notches}: {e:?}");
        assert!(e.contains(&moved(170.0, 100.0)));
        // Two-finger drag: Middle at the centroid once it travelled.
        let e = run(&mut stage, vec![touch(5, &[(0, 1, 70.0, 150.0), (1, 1, 270.0, 150.0)])], &c);
        assert!(e.contains(&button(170.0, 100.0, PointerButton::Middle, true)), "{e:?}");
        assert!(e.contains(&moved(170.0, 150.0)));
        // One finger up: the Middle is released; the remaining finger does nothing until all are up.
        let e = run(&mut stage, vec![touch(6, &[(1, 2, 270.0, 150.0)])], &c);
        assert_eq!(e, vec![button(170.0, 150.0, PointerButton::Middle, false)]);
        assert!(run(&mut stage, vec![touch(7, &[(0, 1, 90.0, 150.0)])], &c).is_empty());
        run(&mut stage, vec![touch(8, &[(0, 2, 90.0, 150.0)])], &c);
        // Over a panel: two-finger drag scrolls (point wheel by the centroid delta).
        let e = run(&mut stage, vec![touch(9, &[(0, 0, 600.0, 100.0), (1, 0, 700.0, 100.0)])], &c);
        assert_eq!(e[..2], [moved(600.0, 100.0), button(600.0, 100.0, PointerButton::Primary, true)]);
        let e = run(&mut stage, vec![touch(10, &[(0, 1, 600.0, 130.0), (1, 1, 700.0, 130.0)])], &c);
        assert!(e.iter().any(|ev| matches!(ev, Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, .. } if *delta == egui::vec2(0.0, 30.0))), "{e:?}");
        // Three fingers: blocked until all are up.
        let e = run(&mut stage, vec![touch(11, &[(2, 0, 650.0, 200.0)]), touch(12, &[(0, 1, 600.0, 200.0)])], &c);
        assert!(e.is_empty(), "{e:?}");
        run(&mut stage, vec![touch(13, &[(0, 2, 0.0, 0.0), (1, 2, 0.0, 0.0), (2, 2, 0.0, 0.0)])], &c);
        assert_eq!(stage.holder, Holder::None);
    }

    #[test]
    fn palm_rejection_while_the_pen_is_near() {
        let view = [egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(500.0, 500.0))];
        let t0 = Instant::now();
        let mut stage = InputStage::default();
        let c = StageCtx { now: t0, ..cx(1.0, &view) };
        // A running one-finger drag is released when the pen comes into range.
        run(&mut stage, vec![touch(1, &[(0, 0, 100.0, 100.0)]), touch(2, &[(0, 1, 120.0, 100.0)])], &c);
        let e = run(&mut stage, vec![pen(3, vec![sample(pen_kind::HOVER, 300.0, 0.0, 0)])], &c);
        assert_eq!(e[0], button(120.0, 100.0, PointerButton::Secondary, false));
        // A new finger while the pen hovers, and within 300 ms after it left: ignored.
        assert!(run(&mut stage, vec![touch(4, &[(1, 0, 50.0, 50.0)]), touch(5, &[(1, 1, 90.0, 50.0)])], &c).is_empty());
        run(&mut stage, vec![touch(6, &[(0, 2, 0.0, 0.0), (1, 2, 0.0, 0.0)])], &c);
        run(&mut stage, vec![pen(7, vec![sample(pen_kind::LEAVE, 300.0, 0.0, 0)])], &c);
        let soon = StageCtx { now: t0 + Duration::from_millis(200), ..cx(1.0, &view) };
        assert!(run(&mut stage, vec![touch(8, &[(2, 0, 50.0, 50.0)]), touch(9, &[(2, 1, 90.0, 50.0)])], &soon).is_empty());
        run(&mut stage, vec![touch(10, &[(2, 2, 90.0, 50.0)])], &soon);
        let later = StageCtx { now: t0 + Duration::from_millis(400), ..cx(1.0, &view) };
        let e = run(&mut stage, vec![touch(11, &[(3, 0, 50.0, 50.0)]), touch(12, &[(3, 1, 90.0, 50.0)])], &later);
        assert!(e.contains(&button(50.0, 50.0, PointerButton::Secondary, true)), "{e:?}");
    }

    #[test]
    fn a_disconnect_in_mid_stroke_leaves_nothing_held() {
        let mut stage = InputStage::default();
        let mut router = PenRouter::default();
        let c = cx(1.0, &[]);
        route(&mut stage, &mut router, Vec::new(), vec![pen(1, vec![sample(pen_kind::DOWN, 10.0, 0.5, 0), sample(pen_kind::MOVE, 20.0, 0.5, 0)]), Msg::Mods { seq: 2, mods: proto::mods::SHIFT }], &c);
        assert_eq!(stage.holder, Holder::Remote);
        let mut events = Vec::new();
        let mut feed = VecDeque::new();
        stage.end_session(&mut events, &mut feed);
        let frame = router.process(&mut events, &mut feed);
        assert_eq!(events, vec![Event::ModifiersChanged(Modifiers::NONE), button(20.0, 20.0, PointerButton::Primary, false), Event::PointerGone]);
        assert!(!frame.in_contact && !frame.in_range);
        assert_eq!(stage.holder, Holder::None);
        assert!(!stage.active());
        // A finger gesture in progress is released too.
        let view = [egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(500.0, 500.0))];
        let c = cx(1.0, &view);
        run(&mut stage, vec![touch(3, &[(0, 0, 100.0, 100.0)]), touch(4, &[(0, 1, 120.0, 100.0)])], &c);
        let mut events = Vec::new();
        stage.end_session(&mut events, &mut VecDeque::new());
        assert_eq!(events, vec![button(120.0, 100.0, PointerButton::Secondary, false)]);
        assert_eq!(stage.holder, Holder::None);
    }

    #[test]
    fn without_remote_input_local_events_are_unchanged() {
        let mut stage = InputStage::default();
        let local = vec![
            Event::ModifiersChanged(Modifiers::SHIFT),
            moved(1.0, 2.0),
            button(1.0, 2.0, PointerButton::Primary, true),
            moved(3.0, 4.0),
            button(3.0, 4.0, PointerButton::Primary, false),
            Event::Text("x".into()),
        ];
        let mut events = local.clone();
        let mut feed = VecDeque::new();
        let out = stage.process(&mut events, &mut feed, Vec::new(), &cx(1.25, &[]));
        assert_eq!(events, local);
        assert!(feed.is_empty() && out == StageOut::default());
    }
}
