//! Pen and drawing-tablet input (branch `pen-input`, `docs/plans/PEN_INPUT_PLAN.md`, ARCHITECTURE 1.10, 8.3, 8.13).
//!
//! A pen is "a mouse with pressure" (decision D1). The window procedure subclass (`win.rs`, Windows only) records every
//! pen history entry (`PenRecord`: exact per-sample pressure, barrel button, eraser end); `raw_input_hook` hands the
//! records and the frame's events to the pure router (`route.rs`), which removes the pen's `Touch` events, rewrites
//! the barrel press and notes the pressure of every pointer event (`PenFrame`). The views read that pressure
//! (`PenFrame::cursor`); `apply_eraser_override` turns the eraser end into the Eraser tool.
//!
//! - `decode.rs`: pure decoding of the pointer / pen flags.
//! - `route.rs`: the pure event router.
//! - `win.rs`: the Windows glue (subclass, Ink feedback off, system gestures off).

#[allow(dead_code, reason = "read by the window procedure (win.rs, part D)")]
pub mod decode;
pub mod route;

use std::collections::VecDeque;

use eframe::egui;

use crate::state::ToolKind;

pub use route::PenRouter;

/// At most this many records wait in a feed (oldest dropped first).
pub const FEED_CAP: usize = 8192;

/// What kind of pointer message a record comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PenKind {
    /// `WM_POINTERDOWN` (the tip touched).
    Down,
    /// `WM_POINTERUPDATE` (a move in contact or hovering in range).
    Update,
    /// `WM_POINTERUP` (the tip lifted; still in range).
    Up,
    /// `WM_POINTERLEAVE` / `WM_POINTERCAPTURECHANGED`: left the range or lost the capture (winit sends no touch).
    Leave,
}

/// What the window procedure saw for one history entry of a pen pointer message (or one synthetic test record).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PenRecord {
    pub pointer_id: u32,
    pub kind: PenKind,
    pub in_contact: bool,
    /// 0..=1; 1.0 for a pen without pressure.
    pub pressure: f32,
    /// `PEN_FLAG_BARREL` or `POINTER_FLAG_SECONDBUTTON`.
    pub barrel: bool,
    /// `POINTER_FLAG_THIRDBUTTON`.
    pub third: bool,
    /// `PEN_FLAG_INVERTED` or `PEN_FLAG_ERASER`.
    pub inverted: bool,
}

/// The pen's part of one frame's input; rebuilt by every `raw_input_hook`.
#[derive(Clone, Debug, PartialEq)]
pub struct PenFrame {
    /// One entry per `PointerMoved` / `PointerButton` of the routed event list, in order: its position and, for an
    /// event of a pen in contact, the pressure. Empty in a frame without pen input (every event then reads 1.0).
    pub pointer: Vec<(egui::Pos2, Option<f32>)>,
    pub in_range: bool,
    pub in_contact: bool,
    pub inverted: bool,
    pub barrel: bool,
    /// Of the last contact sample; 1.0 without contact.
    pub pressure: f32,
    /// A pointer button was pressed by something else than the pen (the mouse) in this frame.
    pub mouse_press: bool,
    /// A pen has been seen in this run.
    pub seen: bool,
}

impl Default for PenFrame {
    fn default() -> PenFrame {
        PenFrame {
            pointer: Vec::new(),
            in_range: false,
            in_contact: false,
            inverted: false,
            barrel: false,
            pressure: 1.0,
            mouse_press: false,
            seen: false,
        }
    }
}

impl PenFrame {
    /// A walker over the frame's pointer notes: ask it for every `PointerMoved` / `PointerButton` of the frame's events
    /// in order (egui never removes pointer events, so the order is that of `raw_input_hook`).
    pub fn cursor(&self) -> PenCursor<'_> {
        PenCursor { notes: &self.pointer, next: 0 }
    }

    /// The pressure of a pointer call that has no event of its own (a drag without motion, the hover): the last contact
    /// pressure while the pen touches, else 1.0.
    pub fn held_pressure(&self) -> f32 {
        if self.in_contact { self.pressure } else { 1.0 }
    }

    /// The status-bar readout (ARCHITECTURE 8.7): "Pen 47 %" in contact, "Pen (eraser)" with the eraser end in range,
    /// "Pen" hovering; `None` without a pen in range.
    pub fn readout(&self) -> Option<String> {
        if !self.in_range {
            None
        } else if self.in_contact {
            Some(format!("Pen {} %", (self.pressure * 100.0).round() as i32))
        } else if self.inverted {
            Some("Pen (eraser)".to_owned())
        } else {
            Some("Pen".to_owned())
        }
    }
}

/// See `PenFrame::cursor`.
pub struct PenCursor<'a> {
    notes: &'a [(egui::Pos2, Option<f32>)],
    next: usize,
}

impl PenCursor<'_> {
    /// The pressure of the next pointer event at `pos`: its note when the note's position is the same (bit for bit),
    /// else 1.0 (a mouse).
    pub fn take(&mut self, pos: egui::Pos2) -> f32 {
        let note = self.notes.get(self.next).copied();
        self.next += 1;
        match note {
            Some((at, Some(p))) if at == pos => p,
            _ => 1.0,
        }
    }
}

/// The pen part of `AppState` (`AppState::pen`).
#[derive(Default)]
pub struct PenState {
    /// Drained from the window procedure's queue, plus the self-test's records; emptied by every route.
    pub feed: VecDeque<PenRecord>,
    /// Persists across frames.
    pub router: PenRouter,
    pub frame: PenFrame,
    /// The tool to return to when the eraser end goes away (`apply_eraser_override`).
    pub eraser_restore: Option<ToolKind>,
    /// A manual tool change dropped the override: no new one until the eraser end leaves (or turns around).
    pub eraser_hold_off: bool,
}

impl PenState {
    /// Routes one frame (`raw_input_hook` step 3).
    pub fn route(&mut self, events: &mut Vec<egui::Event>) {
        self.frame = self.router.process(events, &mut self.feed);
    }

    /// The text of the pen line under the Pen pressure options (ARCHITECTURE 8.7).
    pub fn status_line(&self) -> String {
        if self.frame.in_contact {
            format!("Pen: {} %", (self.frame.pressure * 100.0).round() as i32)
        } else if self.frame.in_range || self.frame.seen {
            "Pen detected".to_owned()
        } else {
            "No pen seen yet (Help > Tablet setup)".to_owned()
        }
    }
}

/// Moves the records the window procedure queued since the last frame into `feed` (part D, `win.rs`); nothing while
/// no subclass is installed.
pub fn drain_window_records(feed: &mut VecDeque<PenRecord>) {
    let _ = feed;
}

/// What the eraser override does this frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EraserStep {
    Nothing,
    /// Switch to the Eraser and remember this tool.
    Engage(ToolKind),
    /// Go back to this tool.
    Restore(ToolKind),
    /// The user changed the tool meanwhile: forget the override, keep the tool.
    Drop,
}

/// The eraser-end rule (decision D7, ARCHITECTURE 8.3), pure: while the inverted end is in range the tool is the
/// Eraser; when the pen turns back, leaves the range or the mouse presses, the previous tool returns. Nothing happens
/// while the tools are busy (an open stroke or drag); a manual tool change during the override drops it.
pub fn eraser_step(pen: &PenState, tool: ToolKind, busy: bool) -> EraserStep {
    if busy {
        return EraserStep::Nothing;
    }
    let f = &pen.frame;
    let eraser_end = f.in_range && f.inverted;
    match pen.eraser_restore {
        None => {
            if eraser_end && !pen.eraser_hold_off && tool != ToolKind::Eraser {
                EraserStep::Engage(tool)
            } else {
                EraserStep::Nothing
            }
        }
        Some(previous) => {
            if tool != ToolKind::Eraser {
                EraserStep::Drop
            } else if !eraser_end || f.mouse_press {
                EraserStep::Restore(previous)
            } else {
                EraserStep::Nothing
            }
        }
    }
}

/// Applies `eraser_step` to the pen state and returns the tool to set through the normal `SetTool` dispatch, if any.
pub fn eraser_override(pen: &mut PenState, tool: ToolKind, busy: bool) -> Option<ToolKind> {
    let eraser_end = pen.frame.in_range && pen.frame.inverted;
    if !eraser_end {
        pen.eraser_hold_off = false;
    }
    match eraser_step(pen, tool, busy) {
        EraserStep::Nothing => None,
        EraserStep::Engage(previous) => {
            pen.eraser_restore = Some(previous);
            Some(ToolKind::Eraser)
        }
        EraserStep::Restore(previous) => {
            pen.eraser_restore = None;
            Some(previous)
        }
        EraserStep::Drop => {
            pen.eraser_restore = None;
            pen.eraser_hold_off = eraser_end;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pen(in_range: bool, inverted: bool) -> PenState {
        PenState { frame: PenFrame { in_range, inverted, seen: true, ..PenFrame::default() }, ..PenState::default() }
    }

    #[test]
    fn the_cursor_pairs_notes_with_events() {
        let a = egui::pos2(1.0, 2.0);
        let b = egui::pos2(3.0, 4.0);
        let frame = PenFrame { pointer: vec![(a, None), (a, Some(0.25)), (b, Some(0.5))], ..PenFrame::default() };
        let mut c = frame.cursor();
        assert_eq!(c.take(a), 1.0, "a hover move at the same place reads 1.0, not the next note");
        assert_eq!(c.take(a), 0.25);
        assert_eq!(c.take(a), 1.0, "a position mismatch reads 1.0");
        assert_eq!(c.take(b), 1.0, "past the end: 1.0");
        let empty = PenFrame::default();
        assert_eq!(empty.cursor().take(a), 1.0);
        assert_eq!(empty.held_pressure(), 1.0);
        assert_eq!(PenFrame { in_contact: true, pressure: 0.3, ..PenFrame::default() }.held_pressure(), 0.3);
    }

    #[test]
    fn readout_and_status_line() {
        assert_eq!(PenFrame::default().readout(), None);
        let hover = PenFrame { in_range: true, ..PenFrame::default() };
        assert_eq!(hover.readout().as_deref(), Some("Pen"));
        let eraser = PenFrame { in_range: true, inverted: true, ..PenFrame::default() };
        assert_eq!(eraser.readout().as_deref(), Some("Pen (eraser)"));
        let contact = PenFrame { in_range: true, in_contact: true, pressure: 0.468, ..PenFrame::default() };
        assert_eq!(contact.readout().as_deref(), Some("Pen 47 %"));
        let mut s = PenState::default();
        assert_eq!(s.status_line(), "No pen seen yet (Help > Tablet setup)");
        s.frame = PenFrame { seen: true, ..PenFrame::default() };
        assert_eq!(s.status_line(), "Pen detected");
        s.frame = contact;
        assert_eq!(s.status_line(), "Pen: 47 %");
    }

    #[test]
    fn the_eraser_end_switches_and_restores() {
        let mut p = pen(true, true);
        assert_eq!(eraser_override(&mut p, ToolKind::Brush, true), None, "never while busy");
        assert_eq!(eraser_override(&mut p, ToolKind::Brush, false), Some(ToolKind::Eraser));
        assert_eq!(p.eraser_restore, Some(ToolKind::Brush));
        assert_eq!(eraser_override(&mut p, ToolKind::Eraser, false), None, "held while the end is in range");
        p.frame.inverted = false;
        assert_eq!(eraser_override(&mut p, ToolKind::Eraser, true), None, "busy: wait");
        assert_eq!(eraser_override(&mut p, ToolKind::Eraser, false), Some(ToolKind::Brush));
        assert_eq!(p.eraser_restore, None);
        // Leaving the range restores too; so does a mouse press.
        let mut p = pen(true, true);
        eraser_override(&mut p, ToolKind::Fill, false);
        p.frame.in_range = false;
        assert_eq!(eraser_override(&mut p, ToolKind::Eraser, false), Some(ToolKind::Fill));
        let mut p = pen(true, true);
        eraser_override(&mut p, ToolKind::Navigate, false);
        p.frame.mouse_press = true;
        assert_eq!(eraser_override(&mut p, ToolKind::Eraser, false), Some(ToolKind::Navigate));
        // Already the Eraser: nothing to remember.
        let mut p = pen(true, true);
        assert_eq!(eraser_override(&mut p, ToolKind::Eraser, false), None);
        assert_eq!(p.eraser_restore, None);
    }

    #[test]
    fn a_manual_tool_change_drops_the_override() {
        let mut p = pen(true, true);
        assert_eq!(eraser_override(&mut p, ToolKind::Brush, false), Some(ToolKind::Eraser));
        // The user picks Fill while the eraser end hovers: kept, not fought.
        assert_eq!(eraser_override(&mut p, ToolKind::Fill, false), None);
        assert_eq!(p.eraser_restore, None);
        assert_eq!(eraser_override(&mut p, ToolKind::Fill, false), None, "no new override while the end stays");
        // Turned around and back: the override works again.
        p.frame.inverted = false;
        assert_eq!(eraser_override(&mut p, ToolKind::Fill, false), None);
        p.frame.inverted = true;
        assert_eq!(eraser_override(&mut p, ToolKind::Fill, false), Some(ToolKind::Eraser));
        // A tool change after the pen turned back: the eraser override is simply gone.
        p.frame.inverted = false;
        assert_eq!(eraser_override(&mut p, ToolKind::Text, false), None);
        assert_eq!(p.eraser_restore, None);
    }
}
