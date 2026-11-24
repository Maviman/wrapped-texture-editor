//! The pure pen event router (branch `pen-input`, PEN_INPUT_PLAN 4.3, ARCHITECTURE 8.3).
//!
//! egui-winit turns every Windows Ink pen sample into an `Event::Touch` plus emulated mouse events (`PointerMoved`,
//! `PointerButton { Primary }`, `PointerGone`). The router pairs each pen `Touch` with the record the window procedure
//! saw for the same history entry (`PenRecord`: exact pressure and the pen's flags), drops the `Touch` (decision D1: the
//! pen is a mouse with pressure, egui never enters its touch-screen mode), rewrites the button for the barrel button,
//! synthesizes the emulated events when egui-winit's emulation is stuck on an older touch, and notes the pressure of
//! every pointer event it produced (`PenFrame::pointer`). Finger touches and every non-touch event pass unchanged; a
//! frame without pen touches and without records leaves the event list as it was, bit for bit.

use std::collections::VecDeque;

use eframe::egui;
use egui::{Event, Modifiers, PointerButton, Pos2, TouchPhase};

use super::{FEED_CAP, PenFrame, PenKind, PenRecord};

/// The router's state across frames.
#[derive(Clone, Debug, Default)]
pub struct PenRouter {
    /// Every touch is a pen (interactive runs without the window-procedure subclass, `pen::install` failed or not on
    /// Windows): pens then paint with winit's pressure; fingers act as a mouse too.
    pub touch_is_pen: bool,
    /// Touch ids known to be pens (from the records); forgotten when the pen leaves.
    known: Vec<u64>,
    /// The pen in contact and the button its press became.
    contact: Option<(u64, PointerButton)>,
    /// The last record's state (in range, in contact, flags) and the last contact pressure.
    in_range: bool,
    in_contact: bool,
    inverted: bool,
    barrel: bool,
    third: bool,
    pressure: f32,
    /// The last pen position (the release of a lost contact goes there).
    last_pos: Option<Pos2>,
    /// The modifiers of the last `ModifiersChanged` / `PointerButton` seen.
    modifiers: Modifiers,
    /// The last pointer event the router passed or made came from the pen.
    pointer_is_pen: bool,
    /// A pen has been seen in this run.
    seen: bool,
}

impl PenRouter {
    /// Routes one frame's events (see the module comment); `feed` holds the records of this frame (oldest first) and is
    /// empty afterwards.
    pub fn process(&mut self, events: &mut Vec<Event>, feed: &mut VecDeque<PenRecord>) -> PenFrame {
        cap_feed(feed);
        let mut frame = PenFrame::default();
        if feed.is_empty() && !events.iter().any(|e| matches!(e, Event::Touch { .. })) {
            // The mouse-only frame: nothing is rewritten and no pressure is noted (every pointer event reads 1.0).
            for e in events.iter() {
                self.note_passthrough(e, &mut frame);
            }
            self.fill(&mut frame);
            return frame;
        }
        let input = std::mem::take(events);
        let mut out = Out { events: Vec::with_capacity(input.len() + 4), frame: &mut frame };
        let mut i = 0;
        while i < input.len() {
            let event = &input[i];
            i += 1;
            let Event::Touch { id, phase, pos, force, .. } = *event else {
                self.note_passthrough(event, out.frame);
                out.pass(event.clone());
                continue;
            };
            let tid = id.0;
            if !self.is_pen(tid, feed) {
                // A finger: the touch and the emulated events egui-winit put after it stay as they are.
                out.events.push(event.clone());
                continue;
            }
            let record = self.take_record(tid, phase, force, feed, &mut out);
            self.apply(&record);
            let contact_p = if record.pressure > 0.0 { record.pressure } else { force.unwrap_or(1.0) };
            match phase {
                TouchPhase::Start => {
                    let button = if record.barrel {
                        PointerButton::Secondary
                    } else if record.third {
                        PointerButton::Middle
                    } else {
                        PointerButton::Primary
                    };
                    let (mut bpos, mut mods) = (pos, self.modifiers);
                    if matches!(input.get(i), Some(Event::PointerMoved(p)) if *p == pos) {
                        i += 1;
                        if let Some(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers }) =
                            input.get(i)
                        {
                            (bpos, mods) = (*p, *modifiers);
                            i += 1;
                        }
                    }
                    self.pressure = contact_p;
                    out.pen(Event::PointerMoved(pos), Some(contact_p));
                    out.pen(Event::PointerButton { pos: bpos, button, pressed: true, modifiers: mods }, Some(contact_p));
                    self.contact = Some((tid, button));
                    self.last_pos = Some(pos);
                    self.pointer_is_pen = true;
                }
                TouchPhase::Move => {
                    if matches!(input.get(i), Some(Event::PointerMoved(p)) if *p == pos) {
                        i += 1;
                    }
                    let touching = self.contact.is_some_and(|(c, _)| c == tid);
                    if touching {
                        self.pressure = contact_p;
                    }
                    out.pen(Event::PointerMoved(pos), touching.then_some(contact_p));
                    self.last_pos = Some(pos);
                    self.pointer_is_pen = true;
                }
                TouchPhase::End | TouchPhase::Cancel => {
                    let held = self.contact.filter(|(c, _)| *c == tid);
                    let (mut bpos, mut mods) = (self.last_pos.unwrap_or(pos), self.modifiers);
                    let mut released = false;
                    if let Some(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers }) =
                        input.get(i)
                    {
                        (bpos, mods) = (*p, *modifiers);
                        released = true;
                        i += 1;
                    }
                    if matches!(input.get(i), Some(Event::PointerGone)) {
                        i += 1;
                    }
                    if held.is_some() || released {
                        let button = held.map_or(PointerButton::Primary, |(_, b)| b);
                        out.pen(Event::PointerButton { pos: bpos, button, pressed: false, modifiers: mods }, Some(self.pressure));
                    }
                    out.pen(Event::PointerGone, None);
                    if held.is_some() {
                        self.contact = None;
                    }
                    // The pointer is gone already: a later Leave has no hover to clear.
                    self.pointer_is_pen = false;
                    if self.touch_is_pen {
                        // Without the subclass no Leave ever comes: the lifted pen counts as out of range.
                        self.in_range = false;
                        self.in_contact = false;
                    }
                }
            }
        }
        // Records without a touch of this frame: Leave records, and entries winit skipped (state only).
        while let Some(record) = feed.pop_front() {
            self.apply_record(record, &mut out);
        }
        *events = out.events;
        self.fill(&mut frame);
        frame
    }

    /// A pen touch: a record of that id waits in the feed, or the id is a known pen.
    fn is_pen(&self, id: u64, feed: &VecDeque<PenRecord>) -> bool {
        self.touch_is_pen || self.known.contains(&id) || feed.iter().any(|r| u64::from(r.pointer_id) == id)
    }

    /// The record of a pen touch: the first record of that id whose kind matches the phase (records of the same id
    /// before it are applied on the way, Leave included); without one, a fallback from winit's data (pressure `force`
    /// in contact, flags as last known). Records of other ids keep their place.
    fn take_record(&mut self, id: u64, phase: TouchPhase, force: Option<f32>, feed: &mut VecDeque<PenRecord>, out: &mut Out) -> PenRecord {
        let matches = |kind: PenKind| match phase {
            TouchPhase::Start => kind == PenKind::Down,
            TouchPhase::Move => kind == PenKind::Update,
            TouchPhase::End => kind == PenKind::Up,
            TouchPhase::Cancel => matches!(kind, PenKind::Up | PenKind::Leave),
        };
        if let Some(k) = feed.iter().position(|r| u64::from(r.pointer_id) == id && matches(r.kind)) {
            let mut j = 0;
            let mut k = k;
            while j < k {
                if u64::from(feed[j].pointer_id) == id {
                    let skipped = feed.remove(j).expect("index in range");
                    self.apply_record(skipped, out);
                    k -= 1;
                } else {
                    j += 1;
                }
            }
            return feed.remove(k).expect("index in range");
        }
        let in_contact = match phase {
            TouchPhase::Start => true,
            TouchPhase::Move => self.contact.is_some_and(|(c, _)| c == id),
            TouchPhase::End | TouchPhase::Cancel => false,
        };
        PenRecord {
            pointer_id: id as u32,
            kind: match phase {
                TouchPhase::Start => PenKind::Down,
                TouchPhase::Move => PenKind::Update,
                TouchPhase::End | TouchPhase::Cancel => PenKind::Up,
            },
            in_contact,
            pressure: if in_contact { force.map_or(1.0, |f| f.clamp(0.0, 1.0)) } else { 0.0 },
            barrel: self.barrel,
            third: self.third,
            inverted: self.inverted,
        }
    }

    /// A record without its own touch event: its state, and for a Leave the end of the pen's contact or hover.
    fn apply_record(&mut self, record: PenRecord, out: &mut Out) {
        self.apply(&record);
        if record.kind != PenKind::Leave {
            return;
        }
        let id = u64::from(record.pointer_id);
        if let Some((_, button)) = self.contact.filter(|(c, _)| *c == id) {
            // Capture lost in contact: release the button where the pen was, so no drag stays open.
            let pos = self.last_pos.unwrap_or(Pos2::ZERO);
            let mods = self.modifiers;
            out.pen(Event::PointerButton { pos, button, pressed: false, modifiers: mods }, Some(self.pressure));
            out.pen(Event::PointerGone, None);
            self.contact = None;
        } else if self.pointer_is_pen {
            // The hovering pen left: hover highlights go away.
            out.pen(Event::PointerGone, None);
        }
    }

    /// The state a record says.
    fn apply(&mut self, r: &PenRecord) {
        let id = u64::from(r.pointer_id);
        self.seen = true;
        if r.kind == PenKind::Leave {
            self.known.retain(|k| *k != id);
            self.in_range = false;
            self.in_contact = false;
            return;
        }
        if !self.known.contains(&id) {
            self.known.push(id);
        }
        self.in_range = true;
        self.in_contact = r.in_contact;
        self.inverted = r.inverted;
        self.barrel = r.barrel;
        self.third = r.third;
        if r.in_contact && r.pressure > 0.0 {
            self.pressure = r.pressure;
        }
    }

    /// Bookkeeping of an event that passes unchanged.
    fn note_passthrough(&mut self, e: &Event, frame: &mut PenFrame) {
        match e {
            Event::ModifiersChanged(m) => self.modifiers = *m,
            Event::PointerButton { pressed, modifiers, .. } => {
                self.modifiers = *modifiers;
                self.pointer_is_pen = false;
                if *pressed {
                    frame.mouse_press = true;
                }
            }
            Event::PointerMoved(_) | Event::PointerGone | Event::MouseMoved(_) => self.pointer_is_pen = false,
            _ => {}
        }
    }

    /// The frame's pen state from the router's.
    fn fill(&self, frame: &mut PenFrame) {
        frame.in_range = self.in_range;
        frame.in_contact = self.contact.is_some();
        frame.inverted = self.in_range && self.inverted;
        frame.barrel = self.in_range && self.barrel;
        frame.pressure = if frame.in_contact { self.pressure } else { 1.0 };
        frame.seen = self.seen;
    }
}

/// The event list being built and the frame's notes.
struct Out<'a> {
    events: Vec<Event>,
    frame: &'a mut PenFrame,
}

impl Out<'_> {
    /// An event that passes unchanged: a pointer event gets the entry "no pressure" once the frame has pen entries.
    fn pass(&mut self, e: Event) {
        if let Event::PointerMoved(pos) | Event::PointerButton { pos, .. } = e {
            self.frame.pointer.push((pos, None));
        }
        self.events.push(e);
    }

    /// An event the pen made, with its pressure (`None` for hover).
    fn pen(&mut self, e: Event, pressure: Option<f32>) {
        if let Event::PointerMoved(pos) | Event::PointerButton { pos, .. } = e {
            self.frame.pointer.push((pos, pressure));
        }
        self.events.push(e);
    }
}

/// Keeps the newest `FEED_CAP` records (a feed that is not drained while frames do not run cannot grow).
pub fn cap_feed(feed: &mut VecDeque<PenRecord>) {
    while feed.len() > FEED_CAP {
        feed.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{TouchDeviceId, TouchId};

    const PEN: u32 = 77;

    fn touch(id: u32, phase: TouchPhase, x: f32, force: Option<f32>) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(u64::from(id)), phase, pos: egui::pos2(x, 10.0), force }
    }

    fn rec(kind: PenKind, in_contact: bool, pressure: f32) -> PenRecord {
        PenRecord { pointer_id: PEN, kind, in_contact, pressure, barrel: false, third: false, inverted: false }
    }

    fn moved(x: f32) -> Event {
        Event::PointerMoved(egui::pos2(x, 10.0))
    }

    fn button(x: f32, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos: egui::pos2(x, 10.0), button, pressed, modifiers: Modifiers::NONE }
    }

    /// What egui-winit pushes for a pen sample while it translates that pen.
    fn emulated(phase: TouchPhase, x: f32, force: Option<f32>, last_x: f32) -> Vec<Event> {
        let mut v = vec![touch(PEN, phase, x, force)];
        match phase {
            TouchPhase::Start => v.extend([moved(x), button(x, PointerButton::Primary, true)]),
            TouchPhase::Move => v.push(moved(x)),
            TouchPhase::End => v.extend([button(last_x, PointerButton::Primary, false), Event::PointerGone]),
            TouchPhase::Cancel => v.push(Event::PointerGone),
        }
        v
    }

    fn feed(records: &[PenRecord]) -> VecDeque<PenRecord> {
        records.iter().copied().collect()
    }

    fn no_touch(events: &[Event]) -> bool {
        !events.iter().any(|e| matches!(e, Event::Touch { .. }))
    }

    #[test]
    fn a_tap_is_a_click_without_touch_events() {
        let mut r = PenRouter::default();
        let mut events = emulated(TouchPhase::Start, 5.0, Some(0.5), 5.0);
        events.extend(emulated(TouchPhase::End, 5.0, None, 5.0));
        let mut f = feed(&[rec(PenKind::Down, true, 0.4), rec(PenKind::Up, false, 0.0)]);
        let frame = r.process(&mut events, &mut f);
        assert_eq!(
            events,
            vec![moved(5.0), button(5.0, PointerButton::Primary, true), button(5.0, PointerButton::Primary, false), Event::PointerGone]
        );
        assert!(f.is_empty());
        assert_eq!(frame.pointer, vec![(egui::pos2(5.0, 10.0), Some(0.4)); 3], "the record's pressure, not winit's force");
        assert!(frame.in_range && !frame.in_contact && frame.seen);
    }

    #[test]
    fn a_move_burst_keeps_every_pressure_in_order() {
        let mut r = PenRouter::default();
        let mut events = emulated(TouchPhase::Start, 0.0, Some(1.0), 0.0);
        let mut records = vec![rec(PenKind::Down, true, 0.1)];
        for k in 1..=6 {
            // winit gives every coalesced sample the latest pressure; the records hold the real ones.
            events.extend(emulated(TouchPhase::Move, k as f32, Some(0.9), 0.0));
            records.push(rec(PenKind::Update, true, 0.1 * (k + 1) as f32));
        }
        let frame = r.process(&mut events, &mut feed(&records));
        assert!(no_touch(&events));
        let pressures: Vec<f32> = frame.pointer.iter().map(|(_, p)| p.unwrap()).collect();
        let want: Vec<f32> = [0.1, 0.1].into_iter().chain((1..=6).map(|k| 0.1 * (k + 1) as f32)).collect();
        assert_eq!(pressures, want);
        assert!(frame.in_contact && (frame.pressure - 0.7).abs() < 1e-6);
    }

    #[test]
    fn barrel_and_third_button_presses() {
        for (flag, want) in [("barrel", PointerButton::Secondary), ("third", PointerButton::Middle)] {
            let mut r = PenRouter::default();
            let mut events = emulated(TouchPhase::Start, 2.0, Some(0.5), 2.0);
            events.extend(emulated(TouchPhase::End, 2.0, None, 2.0));
            let mut down = rec(PenKind::Down, true, 0.5);
            let mut up = rec(PenKind::Up, false, 0.0);
            if flag == "barrel" {
                (down.barrel, up.barrel) = (true, true);
            } else {
                (down.third, up.third) = (true, true);
            }
            r.process(&mut events, &mut feed(&[down, up]));
            assert_eq!(events[1], button(2.0, want, true), "{flag}");
            assert_eq!(events[2], button(2.0, want, false), "{flag}: the release matches the press");
        }
    }

    #[test]
    fn the_eraser_end_is_reported_and_paints_with_the_primary_button() {
        let mut r = PenRouter::default();
        let inv = |kind, contact, p| PenRecord { inverted: true, ..rec(kind, contact, p) };
        let mut events = emulated(TouchPhase::Move, 1.0, None, 0.0);
        let frame = r.process(&mut events, &mut feed(&[inv(PenKind::Update, false, 0.0)]));
        assert!(frame.inverted && frame.in_range && !frame.in_contact);
        assert_eq!(events, vec![moved(1.0)], "hover moves stay, untagged");
        assert_eq!(frame.pointer, vec![(egui::pos2(1.0, 10.0), None)]);
        let mut events = emulated(TouchPhase::Start, 1.0, Some(0.3), 1.0);
        let frame = r.process(&mut events, &mut feed(&[inv(PenKind::Down, true, 0.3)]));
        assert!(frame.inverted && frame.in_contact);
        assert_eq!(events[1], button(1.0, PointerButton::Primary, true));
    }

    #[test]
    fn finger_touches_pass_unchanged() {
        let mut r = PenRouter::default();
        let finger = |phase, x| touch(5, phase, x, None);
        let mut events = vec![finger(TouchPhase::Start, 1.0), moved(1.0), button(1.0, PointerButton::Primary, true), finger(TouchPhase::Move, 2.0), moved(2.0)];
        let want = events.clone();
        let frame = r.process(&mut events, &mut VecDeque::new());
        assert_eq!(events, want, "a finger keeps its touch and its emulation");
        assert!(!frame.seen && !frame.in_range);
        assert!(frame.pointer.iter().all(|(_, p)| p.is_none()));
    }

    #[test]
    fn without_the_subclass_every_touch_is_a_pen_with_winits_pressure() {
        let mut r = PenRouter { touch_is_pen: true, ..PenRouter::default() };
        let mut events = emulated(TouchPhase::Start, 1.0, Some(0.3), 1.0);
        events.extend(emulated(TouchPhase::Move, 2.0, Some(0.6), 1.0));
        let frame = r.process(&mut events, &mut VecDeque::new());
        assert!(no_touch(&events) && frame.in_contact);
        let p: Vec<Option<f32>> = frame.pointer.iter().map(|(_, p)| *p).collect();
        assert_eq!(p, vec![Some(0.3), Some(0.3), Some(0.6)]);
    }

    #[test]
    fn a_record_count_mismatch_resyncs_without_panic() {
        let mut r = PenRouter::default();
        // Fewer records than touches: the moves without a record use winit's force.
        let mut events = emulated(TouchPhase::Start, 0.0, Some(0.6), 0.0);
        events.extend(emulated(TouchPhase::Move, 1.0, Some(0.6), 0.0));
        events.extend(emulated(TouchPhase::Move, 2.0, Some(0.65), 0.0));
        let frame = r.process(&mut events, &mut feed(&[rec(PenKind::Down, true, 0.2)]));
        let p: Vec<Option<f32>> = frame.pointer.iter().map(|(_, p)| *p).collect();
        assert_eq!(p, vec![Some(0.2), Some(0.2), Some(0.6), Some(0.65)]);
        // More records than touches (winit skipped an entry): the extra ones are applied at the end of the frame, the
        // stroke ends at its Up.
        let mut events = emulated(TouchPhase::Move, 3.0, Some(0.7), 2.0);
        events.extend(emulated(TouchPhase::End, 3.0, None, 3.0));
        let records = [rec(PenKind::Update, true, 0.3), rec(PenKind::Update, true, 0.4), rec(PenKind::Up, false, 0.0)];
        let mut f = feed(&records);
        let frame = r.process(&mut events, &mut f);
        assert!(f.is_empty() && no_touch(&events) && !frame.in_contact);
        assert_eq!(frame.pointer[0].1, Some(0.3));
        assert_eq!(events.last(), Some(&Event::PointerGone));
        // A pen touch of a known id with no record at all.
        let mut events = emulated(TouchPhase::Start, 4.0, None, 4.0);
        let frame = r.process(&mut events, &mut VecDeque::new());
        assert!(no_touch(&events) && frame.in_contact);
        assert_eq!(frame.pointer[0].1, Some(1.0), "no force: full pressure");
    }

    #[test]
    fn stuck_emulation_gets_synthesized_events() {
        let mut r = PenRouter::default();
        // egui-winit still translates an older touch: it pushes the touch events only.
        let mut events = vec![
            touch(PEN, TouchPhase::Start, 1.0, Some(0.5)),
            touch(PEN, TouchPhase::Move, 2.0, Some(0.5)),
            touch(PEN, TouchPhase::End, 2.0, None),
        ];
        let records = [rec(PenKind::Down, true, 0.5), rec(PenKind::Update, true, 0.6), rec(PenKind::Up, false, 0.0)];
        r.process(&mut events, &mut feed(&records));
        assert_eq!(
            events,
            vec![
                moved(1.0),
                button(1.0, PointerButton::Primary, true),
                moved(2.0),
                button(2.0, PointerButton::Primary, false),
                Event::PointerGone
            ]
        );
    }

    #[test]
    fn leave_ends_a_contact_and_a_hover() {
        let mut r = PenRouter::default();
        let mut events = emulated(TouchPhase::Start, 3.0, Some(0.5), 3.0);
        r.process(&mut events, &mut feed(&[rec(PenKind::Down, true, 0.5)]));
        // Capture lost during contact: no touch event, only the Leave record.
        let mut events = Vec::new();
        let frame = r.process(&mut events, &mut feed(&[rec(PenKind::Leave, false, 0.0)]));
        assert_eq!(events, vec![button(3.0, PointerButton::Primary, false), Event::PointerGone]);
        assert!(!frame.in_range && !frame.in_contact);
        // A hovering pen leaves: the pointer is gone.
        let mut events = emulated(TouchPhase::Move, 4.0, None, 3.0);
        r.process(&mut events, &mut feed(&[rec(PenKind::Update, false, 0.0)]));
        let mut events = Vec::new();
        r.process(&mut events, &mut feed(&[rec(PenKind::Leave, false, 0.0)]));
        assert_eq!(events, vec![Event::PointerGone]);
        // After a mouse move the pen's Leave does not hide the mouse.
        let mut events = emulated(TouchPhase::Move, 4.0, None, 3.0);
        r.process(&mut events, &mut feed(&[rec(PenKind::Update, false, 0.0)]));
        let mut events = vec![moved(50.0)];
        r.process(&mut events, &mut VecDeque::new());
        let mut events = Vec::new();
        r.process(&mut events, &mut feed(&[rec(PenKind::Leave, false, 0.0)]));
        assert!(events.is_empty());
    }

    #[test]
    fn a_mouse_only_frame_is_unchanged() {
        let mut r = PenRouter::default();
        let mut events = vec![
            Event::ModifiersChanged(Modifiers::SHIFT),
            moved(1.0),
            button(1.0, PointerButton::Secondary, true),
            moved(2.5),
            Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
            button(2.5, PointerButton::Secondary, false),
            Event::PointerGone,
        ];
        let want = events.clone();
        let frame = r.process(&mut events, &mut VecDeque::new());
        assert_eq!(events, want);
        assert!(frame.pointer.is_empty(), "no pressure notes: every event reads 1.0");
        assert!(frame.mouse_press && !frame.in_range && !frame.seen);
        assert_eq!(frame.pressure, 1.0);
    }

    #[test]
    fn the_feed_is_capped() {
        let mut f: VecDeque<PenRecord> = (0..FEED_CAP + 100).map(|_| rec(PenKind::Update, false, 0.0)).collect();
        cap_feed(&mut f);
        assert_eq!(f.len(), FEED_CAP);
    }
}
