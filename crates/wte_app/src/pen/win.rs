//! Windows glue of the pen input (branch `pen-input`, PEN_INPUT_PLAN 5 and 6, ARCHITECTURE 8.3 / 8.13).
//!
//! A window-procedure subclass on the app's HWND (`SetWindowLongPtrW(GWLP_WNDPROC)`) that, for every pen pointer message,
//! reads the pen's whole history with `GetPointerPenInfoHistory` (per-sample pressure and the barrel / eraser flags,
//! which winit drops), decodes it oldest first and queues the records, then forwards the message unchanged to the
//! previous procedure (winit's), whose result it returns. It also answers `WM_TABLET_QUERYSYSTEMGESTURESTATUS` (no
//! press-and-hold, tap and barrel feedback, no flicks), and at install turns the pen's Windows Ink visual feedback off
//! (`SetWindowFeedbackSetting`) and sets the `MicrosoftTabletPenServiceProperty` window property. user32 only, through
//! `raw-dylib` imports (as `gpu_check.rs`): no new crate. Nothing here may panic across the FFI boundary: the message
//! work runs inside `catch_unwind` and the forwarding call happens whatever it did.
//!
//! winit never passes pointer messages to `DefWindowProc` (no duplicate mouse messages for the pen) and keeps its own
//! state in `GWLP_USERDATA` and its class procedure, which the subclass leaves alone. `EnableMouseInPointer` is not used.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, PoisonError};

use super::decode::{RawPen, decode};
use super::{FEED_CAP, PenKind, PenRecord};

type Hwnd = isize;

const GWLP_WNDPROC: i32 = -4;
const WM_NCDESTROY: u32 = 0x0082;
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERDOWN: u32 = 0x0246;
const WM_POINTERUP: u32 = 0x0247;
const WM_POINTERLEAVE: u32 = 0x024A;
const WM_POINTERCAPTURECHANGED: u32 = 0x024C;
const WM_TABLET_QUERYSYSTEMGESTURESTATUS: u32 = 0x02CC;
const PT_PEN: u32 = 3;
/// `FEEDBACK_PEN_BARRELVISUALIZATION`, `_TAP`, `_DOUBLETAP`, `_PRESSANDHOLD`, `_RIGHTTAP` (touch feedback is left alone).
const PEN_FEEDBACK_KINDS: [i32; 5] = [2, 3, 4, 5, 6];
/// `TABLET_DISABLE_PRESSANDHOLD | TABLET_DISABLE_PENTAPFEEDBACK | TABLET_DISABLE_PENBARRELFEEDBACK | TABLET_DISABLE_FLICKS`.
pub const GESTURE_FLAGS: u32 = 0x1 | 0x8 | 0x10 | 0x1_0000;
/// At most this many history entries are read per message.
const MAX_HISTORY: u32 = 1024;

/// `POINT`.
#[repr(C)]
#[derive(Copy, Clone, Default)]
struct Point {
    x: i32,
    y: i32,
}

/// `POINTER_INFO`.
#[repr(C)]
#[derive(Copy, Clone, Default)]
struct PointerInfo {
    pointer_type: u32,
    pointer_id: u32,
    frame_id: u32,
    pointer_flags: u32,
    source_device: isize,
    hwnd_target: isize,
    pt_pixel_location: Point,
    pt_himetric_location: Point,
    pt_pixel_location_raw: Point,
    pt_himetric_location_raw: Point,
    time: u32,
    history_count: u32,
    input_data: i32,
    key_states: u32,
    performance_count: u64,
    button_change_type: i32,
}

/// `POINTER_PEN_INFO`.
#[repr(C)]
#[derive(Copy, Clone, Default)]
struct PointerPenInfo {
    pointer_info: PointerInfo,
    pen_flags: u32,
    pen_mask: u32,
    pressure: u32,
    rotation: u32,
    tilt_x: i32,
    tilt_y: i32,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<PointerInfo>() == 96 && size_of::<PointerPenInfo>() == 120);

impl PointerPenInfo {
    fn raw(&self) -> RawPen {
        RawPen {
            pointer_flags: self.pointer_info.pointer_flags,
            pen_flags: self.pen_flags,
            pen_mask: self.pen_mask,
            pressure: self.pressure,
        }
    }
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    fn CallWindowProcW(prev: isize, hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn GetPointerType(pointer_id: u32, pointer_type: *mut u32) -> i32;
    fn GetPointerPenInfo(pointer_id: u32, info: *mut PointerPenInfo) -> i32;
    fn GetPointerPenInfoHistory(pointer_id: u32, entries: *mut u32, info: *mut PointerPenInfo) -> i32;
    fn SetWindowFeedbackSetting(hwnd: Hwnd, feedback: i32, flags: u32, size: u32, config: *const std::ffi::c_void) -> i32;
    fn SetPropW(hwnd: Hwnd, name: *const u16, data: isize) -> i32;
    fn RemovePropW(hwnd: Hwnd, name: *const u16) -> isize;
}

/// The previous window procedure (winit's) and the subclassed window; 0 = not installed.
static PREV: AtomicIsize = AtomicIsize::new(0);
static HWND: AtomicIsize = AtomicIsize::new(0);

/// What the window procedure shares with `raw_input_hook` (both run on the UI thread, so the lock is never contended;
/// the Mutex keeps it sound if that ever changes).
#[derive(Debug, Default)]
pub(crate) struct Shared {
    queue: VecDeque<PenRecord>,
    /// Pen ids seen in range (their Leave is queued).
    pens: Vec<u32>,
    /// Pen ids whose last record was in contact (a lost capture then ends the contact).
    contact: Vec<u32>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { queue: VecDeque::new(), pens: Vec::new(), contact: Vec::new() });

fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Queues the records of one message's history, given newest first (as `GetPointerPenInfoHistory` returns it):
    /// decoded oldest first; entries without DOWN / UP / UPDATE are skipped (winit skips them too). Keeps at most
    /// `FEED_CAP` records.
    pub(crate) fn push_history(&mut self, pointer_id: u32, newest_first: &[RawPen]) {
        for raw in newest_first.iter().rev() {
            let Some(record) = decode(pointer_id, raw) else { continue };
            self.queue.push_back(record);
            if !self.pens.contains(&pointer_id) {
                self.pens.push(pointer_id);
            }
            self.contact.retain(|&id| id != pointer_id);
            if record.in_contact {
                self.contact.push(pointer_id);
            }
        }
        while self.queue.len() > FEED_CAP {
            self.queue.pop_front();
        }
    }

    /// `WM_POINTERLEAVE` (`capture_lost` false) or `WM_POINTERCAPTURECHANGED` (true) of `pointer_id`: a Leave record for a
    /// known pen. A lost capture counts only while that pen is in contact (after a normal Up the pen still hovers).
    pub(crate) fn leave(&mut self, pointer_id: u32, capture_lost: bool) {
        let known = if capture_lost { self.contact.contains(&pointer_id) } else { self.pens.contains(&pointer_id) };
        if !known {
            return;
        }
        self.pens.retain(|&id| id != pointer_id);
        self.contact.retain(|&id| id != pointer_id);
        let leave = PenRecord {
            pointer_id,
            kind: PenKind::Leave,
            in_contact: false,
            pressure: 0.0,
            barrel: false,
            third: false,
            inverted: false,
        };
        self.queue.push_back(leave);
        while self.queue.len() > FEED_CAP {
            self.queue.pop_front();
        }
    }

    /// Moves the queued records into `feed`.
    pub(crate) fn drain_into(&mut self, feed: &mut VecDeque<PenRecord>) {
        feed.extend(self.queue.drain(..));
    }
}

/// Moves the records the window procedure queued since the last call into `feed`.
pub fn drain_into(feed: &mut VecDeque<PenRecord>) {
    shared().drain_into(feed);
}

/// Drops the queued records (a self-test run with `WTE_PEN_SUBCLASS=1`: real pens must not reach a scenario).
pub fn discard() {
    shared().queue.clear();
}

/// The window property name, NUL-terminated UTF-16.
fn property_name() -> Vec<u16> {
    "MicrosoftTabletPenServiceProperty".encode_utf16().chain(std::iter::once(0)).collect()
}

/// Installs the subclass on `hwnd` (once per process) and turns the pen's Ink feedback and system gestures off. Must
/// run on the window's thread. Returns a line for the log.
pub fn install(hwnd: Hwnd) -> Result<String, String> {
    if hwnd == 0 {
        return Err("no window handle".to_owned());
    }
    if HWND.load(Ordering::Acquire) != 0 {
        return Ok("already installed".to_owned());
    }
    // SAFETY: plain user32 calls on the app's own window, on its thread; `FALSE` lives across the calls; the property
    // name is NUL-terminated.
    let feedback_off = unsafe {
        let off: i32 = 0;
        PEN_FEEDBACK_KINDS
            .iter()
            .filter(|&&kind| SetWindowFeedbackSetting(hwnd, kind, 0, 4, (&off as *const i32).cast()) != 0)
            .count()
    };
    let name = property_name();
    // SAFETY: as above.
    let property = unsafe { SetPropW(hwnd, name.as_ptr(), GESTURE_FLAGS as isize) != 0 };
    // The previous procedure is stored before the swap, so a message dispatched right after it always finds it.
    // SAFETY: reading and replacing the window procedure of our own window on its thread; `pen_wndproc` has the
    // `WNDPROC` signature and lives for the whole process.
    let prev = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
    if prev == 0 {
        return Err("GetWindowLongPtrW(GWLP_WNDPROC) failed".to_owned());
    }
    PREV.store(prev, Ordering::Release);
    let wndproc: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize = pen_wndproc;
    // SAFETY: see above.
    let replaced = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc as usize as isize) };
    if replaced == 0 {
        PREV.store(0, Ordering::Release);
        return Err("SetWindowLongPtrW(GWLP_WNDPROC) failed".to_owned());
    }
    if replaced != prev {
        PREV.store(replaced, Ordering::Release);
    }
    HWND.store(hwnd, Ordering::Release);
    Ok(format!(
        "pen input: window procedure subclassed; Ink pen feedback off for {feedback_off} of {} kinds; pen service property {}",
        PEN_FEEDBACK_KINDS.len(),
        if property { "set" } else { "NOT set" }
    ))
}

/// The subclass procedure (see the module comment).
unsafe extern "system" fn pen_wndproc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    let prev = PREV.load(Ordering::Acquire);
    if msg == WM_TABLET_QUERYSYSTEMGESTURESTATUS {
        return GESTURE_FLAGS as isize;
    }
    if msg == WM_NCDESTROY {
        // The old procedure goes back first; this message and every later one go to it.
        // SAFETY: our own window on its thread; `prev` is the procedure that was there before.
        unsafe {
            if prev != 0 {
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, prev);
            }
            let name = property_name();
            RemovePropW(hwnd, name.as_ptr());
        }
        HWND.store(0, Ordering::Release);
    } else if matches!(
        msg,
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP | WM_POINTERLEAVE | WM_POINTERCAPTURECHANGED
    ) {
        // A panic must never cross the FFI boundary; the message is forwarded whatever happened here.
        let _ = std::panic::catch_unwind(|| on_pointer_message(msg, wparam));
    }
    if prev == 0 {
        // SAFETY: the default procedure for a window whose previous one is unknown (cannot happen after `install`).
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: `prev` is the window's previous procedure, called with the unchanged message.
    unsafe { CallWindowProcW(prev, hwnd, msg, wparam, lparam) }
}

/// Records of one pointer message of a pen.
fn on_pointer_message(msg: u32, wparam: usize) {
    let pointer_id = (wparam & 0xFFFF) as u32;
    match msg {
        WM_POINTERLEAVE => shared().leave(pointer_id, false),
        WM_POINTERCAPTURECHANGED => shared().leave(pointer_id, true),
        _ => {
            if !is_pen(pointer_id) {
                return;
            }
            if let Some(history) = read_history(pointer_id) {
                shared().push_history(pointer_id, &history);
            }
        }
    }
}

fn is_pen(pointer_id: u32) -> bool {
    let mut kind = 0u32;
    // SAFETY: `kind` is a valid out pointer.
    unsafe { GetPointerType(pointer_id, &mut kind) != 0 && kind == PT_PEN }
}

/// The pen's history of the current message, newest first; the current state alone when the history cannot be read.
fn read_history(pointer_id: u32) -> Option<Vec<RawPen>> {
    let mut current = PointerPenInfo::default();
    // SAFETY: `current` is a valid, writable POINTER_PEN_INFO.
    if unsafe { GetPointerPenInfo(pointer_id, &mut current) } == 0 {
        return None;
    }
    let count = current.pointer_info.history_count.clamp(1, MAX_HISTORY);
    if count == 1 {
        return Some(vec![current.raw()]);
    }
    let mut buf = vec![PointerPenInfo::default(); count as usize];
    let mut n = count;
    // SAFETY: `buf` holds `n` writable entries; the call writes at most `n` and returns the number in `n`.
    if unsafe { GetPointerPenInfoHistory(pointer_id, &mut n, buf.as_mut_ptr()) } == 0 {
        return Some(vec![current.raw()]);
    }
    buf.truncate(n.min(count) as usize);
    Some(buf.iter().map(PointerPenInfo::raw).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pen::decode::{PEN_MASK_PRESSURE, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE};

    fn contact(pressure: u32) -> RawPen {
        RawPen {
            pointer_flags: POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT | POINTER_FLAG_UPDATE,
            pen_flags: 0,
            pen_mask: PEN_MASK_PRESSURE,
            pressure,
        }
    }

    #[test]
    fn struct_layouts() {
        assert_eq!(std::mem::align_of::<PointerPenInfo>(), 8);
        assert_eq!(std::mem::offset_of!(PointerPenInfo, pen_flags), 96);
        assert_eq!(std::mem::offset_of!(PointerPenInfo, tilt_y), 116);
        assert_eq!(std::mem::offset_of!(PointerInfo, history_count), 68);
        assert_eq!(std::mem::offset_of!(PointerInfo, performance_count), 80);
    }

    #[test]
    fn the_history_is_queued_oldest_first() {
        let mut s = Shared::default();
        // Newest first, as GetPointerPenInfoHistory returns it; one entry without a phase is skipped.
        let skipped = RawPen { pointer_flags: POINTER_FLAG_INRANGE, ..RawPen::default() };
        s.push_history(9, &[contact(768), skipped, contact(512), contact(256)]);
        let p: Vec<f32> = s.queue.iter().map(|r| r.pressure).collect();
        assert_eq!(p, vec![0.25, 0.5, 0.75]);
        assert_eq!(s.contact, vec![9]);
        let up = RawPen { pointer_flags: POINTER_FLAG_INRANGE | POINTER_FLAG_UP, ..RawPen::default() };
        s.push_history(9, &[up]);
        assert!(s.contact.is_empty() && s.pens == vec![9]);
        let mut feed = VecDeque::new();
        s.drain_into(&mut feed);
        assert_eq!(feed.len(), 4);
        assert!(s.queue.is_empty());
    }

    #[test]
    fn leave_and_lost_capture() {
        let mut s = Shared::default();
        s.leave(5, false);
        assert!(s.queue.is_empty(), "an unknown pointer (a finger, the mouse) queues nothing");
        s.push_history(5, &[contact(100)]);
        s.push_history(5, &[RawPen { pointer_flags: POINTER_FLAG_INRANGE | POINTER_FLAG_UP, ..RawPen::default() }]);
        s.leave(5, true);
        assert_eq!(s.queue.len(), 2, "a capture change after the Up is no Leave: the pen still hovers");
        s.leave(5, false);
        assert_eq!(s.queue.back().map(|r| r.kind), Some(PenKind::Leave));
        s.push_history(5, &[contact(100)]);
        s.leave(5, true);
        assert_eq!(s.queue.back().map(|r| r.kind), Some(PenKind::Leave), "capture lost in contact");
        assert!(s.pens.is_empty() && s.contact.is_empty());
    }

    #[test]
    fn the_queue_is_capped() {
        let mut s = Shared::default();
        let burst = vec![contact(10); FEED_CAP + 50];
        s.push_history(1, &burst);
        assert_eq!(s.queue.len(), FEED_CAP);
    }
}
