//! WinTab, opt-in (branch `pen-input` part G, PEN_INPUT_PLAN 8, decision D8, ARCHITECTURE 8.3 / 8.13).
//!
//! Used only when `Prefs::tablet_input == WinTab`, in interactive runs, for tablets whose driver has its Windows Ink
//! option off. `wintab32.dll` is loaded at run time (`LoadLibraryW` + `GetProcAddress`): it exists only with a tablet
//! driver, and a load-time import would stop the exe from starting without one. A context is opened on the app window
//! with `CXO_SYSTEM` kept (the driver still moves the cursor and makes the mouse clicks) and `CXO_MESSAGES` added; the
//! window-procedure subclass (`win.rs`) hands `WT_PACKET` / `WT_PROXIMITY` / `WM_ACTIVATE` to `on_message`, which keeps
//! the last packet. WinTab supplies **only** the pressure and the eraser flag: positions and buttons always come from the
//! one egui pointer stream, so no input can arrive twice (`apply`: the frame's pointer events get WinTab pressures while
//! the tip is down; an Ink pen's own pressure is replaced; without packets nothing changes).

use eframe::egui;

use super::PenFrame;

/// The pen state of the last WinTab packets.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct WintabPen {
    pub in_range: bool,
    pub tip_down: bool,
    /// 0..=1.
    pub pressure: f32,
    pub inverted: bool,
}

/// One packet of the context (`PK_CURSOR | PK_BUTTONS | PK_NORMAL_PRESSURE`, absolute mode: 12 bytes in this order).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Packet {
    pub cursor: u32,
    pub buttons: u32,
    pub pressure: u32,
}

const _: () = assert!(size_of::<Packet>() == 12);

/// The pen of the newest packet: tip down = button 0 held; pressure scaled by the device's maximum (`<= 0` -> 1023);
/// the eraser = cursor index 2 modulo 3 (the WinTab convention: puck, pen tip, eraser per cursor type). Without packets
/// the previous state with the given range.
pub fn wintab_state(packets: &[Packet], pressure_max: i32, in_range: bool, previous: WintabPen) -> WintabPen {
    let Some(p) = packets.last() else { return WintabPen { in_range, ..previous } };
    let max = if pressure_max <= 0 { 1023.0 } else { pressure_max as f32 };
    WintabPen {
        in_range,
        tip_down: p.buttons & 1 != 0,
        pressure: (p.pressure as f32 / max).clamp(0.0, 1.0),
        inverted: p.cursor % 3 == 2,
    }
}

/// Linearly spaced pressures for the `n` pointer events of a frame, from just after `prev` to `now` (the last one is
/// `now`).
pub fn spread(prev: f32, now: f32, n: usize) -> Vec<f32> {
    (1..=n).map(|k| if k == n { now } else { prev + (now - prev) * (k as f32 / n as f32) }).collect()
}

/// The pressure ramp between frames (`apply`).
#[derive(Clone, Debug, Default)]
pub struct WintabRamp {
    /// The pressure of the previous frame while the tip was down.
    prev: Option<f32>,
}

/// Lays the WinTab pen over a routed frame (decision D8). While the tip is down every `PointerMoved` / `PointerButton`
/// of the frame gets a pressure of `spread(previous frame's, now, n)` in order (a press gets the first value); an Ink
/// pen's notes are replaced. `in_range` / `in_contact` / `pressure` / `inverted` come from WinTab while it is in range or
/// down. A frame without a WinTab pen is left as it is.
pub fn apply(frame: &mut PenFrame, events: &[egui::Event], tab: &WintabPen, ramp: &mut WintabRamp) {
    if !tab.tip_down {
        ramp.prev = None;
    }
    if !tab.in_range && !tab.tip_down {
        return;
    }
    frame.seen = true;
    frame.in_range = true;
    frame.inverted = tab.inverted;
    if !tab.tip_down {
        return;
    }
    if frame.pointer.is_empty() {
        frame.pointer = events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerMoved(pos) | egui::Event::PointerButton { pos, .. } => Some((*pos, None)),
                _ => None,
            })
            .collect();
    }
    let ramp_from = ramp.prev.unwrap_or(tab.pressure);
    let values = spread(ramp_from, tab.pressure, frame.pointer.len());
    let first = values.first().copied().unwrap_or(tab.pressure);
    let mut k = 0;
    for e in events {
        match e {
            egui::Event::PointerMoved(_) => {
                if let Some(note) = frame.pointer.get_mut(k) {
                    note.1 = Some(values[k]);
                }
                k += 1;
            }
            egui::Event::PointerButton { pressed, .. } => {
                if let Some(note) = frame.pointer.get_mut(k) {
                    note.1 = Some(if *pressed { first } else { values[k] });
                }
                k += 1;
            }
            _ => {}
        }
    }
    frame.in_contact = true;
    frame.pressure = tab.pressure;
    ramp.prev = Some(tab.pressure);
}

#[cfg(windows)]
pub use ffi::{Status, WM_ACTIVATE, WT_PACKET, WT_PROXIMITY, close, on_message, open, poll, status};

#[cfg(windows)]
mod ffi {
    //! The WinTab FFI: run-time loaded `wintab32.dll`, the context, the packets.

    use std::sync::{Mutex, PoisonError};

    use super::{Packet, WintabPen, wintab_state};

    type Hwnd = isize;
    type Hctx = isize;
    type WtInfo = unsafe extern "system" fn(u32, u32, *mut std::ffi::c_void) -> u32;
    type WtOpen = unsafe extern "system" fn(Hwnd, *mut LogContext, i32) -> Hctx;
    type WtClose = unsafe extern "system" fn(Hctx) -> i32;
    type WtPacketsGet = unsafe extern "system" fn(Hctx, i32, *mut std::ffi::c_void) -> i32;
    type WtEnable = unsafe extern "system" fn(Hctx, i32) -> i32;
    type WtOverlap = unsafe extern "system" fn(Hctx, i32) -> i32;

    const WTI_DEFSYSCTX: u32 = 4;
    const WTI_DEVICES: u32 = 100;
    const DVC_NPRESSURE: u32 = 15;
    const CXO_MESSAGES: u32 = 0x4;
    const PK_CURSOR: u32 = 0x20;
    const PK_BUTTONS: u32 = 0x40;
    const PK_NORMAL_PRESSURE: u32 = 0x400;
    pub const WT_DEFBASE: u32 = 0x7FF0;
    pub const WT_PACKET: u32 = WT_DEFBASE;
    pub const WT_PROXIMITY: u32 = WT_DEFBASE + 5;
    pub const WM_ACTIVATE: u32 = 0x0006;
    /// Packets read per `WTPacketsGet` call.
    const BATCH: usize = 64;

    /// `LOGCONTEXTW` (212 bytes); the geometry fields are kept as the driver's defaults.
    #[repr(C)]
    #[derive(Copy, Clone)]
    pub(super) struct LogContext {
        name: [u16; 40],
        options: u32,
        status: u32,
        locks: u32,
        msg_base: u32,
        device: u32,
        pkt_rate: u32,
        pkt_data: u32,
        pkt_mode: u32,
        move_mask: u32,
        btn_dn_mask: u32,
        btn_up_mask: u32,
        /// `lcInOrgX` .. `lcSysSensY`: 22 LONG / FIX32 / BOOL fields.
        rest: [i32; 22],
    }

    /// `AXIS`.
    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    struct Axis {
        min: i32,
        max: i32,
        units: u32,
        resolution: u32,
    }

    const _: () = assert!(size_of::<LogContext>() == 212 && size_of::<Axis>() == 16);

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut std::ffi::c_void;
        fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> *mut std::ffi::c_void;
    }

    #[derive(Copy, Clone)]
    struct Fns {
        info: WtInfo,
        open: WtOpen,
        close: WtClose,
        packets_get: WtPacketsGet,
        enable: WtEnable,
        overlap: WtOverlap,
    }

    /// What `status` reports.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum Status {
        Closed,
        Open,
        /// `wintab32.dll` or one of its functions is missing (no WinTab tablet driver).
        NoDriver,
        /// The driver did not give a context.
        OpenFailed,
    }

    struct Shared {
        fns: Option<Fns>,
        ctx: Hctx,
        pressure_max: i32,
        pen: WintabPen,
        status: Status,
    }

    static SHARED: Mutex<Shared> = Mutex::new(Shared {
        fns: None,
        ctx: 0,
        pressure_max: 0,
        pen: WintabPen { in_range: false, tip_down: false, pressure: 0.0, inverted: false },
        status: Status::Closed,
    });

    fn shared() -> std::sync::MutexGuard<'static, Shared> {
        SHARED.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Loads `wintab32.dll` and its six functions (once; the DLL stays loaded for the process).
    fn load() -> Option<Fns> {
        let name: Vec<u16> = "wintab32.dll".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated name; LoadLibraryW returns 0 when the DLL is absent.
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return None;
        }
        let proc = |name: &[u8]| {
            // SAFETY: `module` is loaded and `name` is NUL-terminated ASCII.
            let p = unsafe { GetProcAddress(module, name.as_ptr()) };
            (!p.is_null()).then_some(p.cast_const())
        };
        // SAFETY: each pointer is the named WinTab export, whose C signature (stdcall = "system" on x86-64 and x86) is
        // the type it is transmuted to.
        unsafe {
            Some(Fns {
                info: std::mem::transmute::<*const std::ffi::c_void, WtInfo>(proc(b"WTInfoW\0")?),
                open: std::mem::transmute::<*const std::ffi::c_void, WtOpen>(proc(b"WTOpenW\0")?),
                close: std::mem::transmute::<*const std::ffi::c_void, WtClose>(proc(b"WTClose\0")?),
                packets_get: std::mem::transmute::<*const std::ffi::c_void, WtPacketsGet>(proc(b"WTPacketsGet\0")?),
                enable: std::mem::transmute::<*const std::ffi::c_void, WtEnable>(proc(b"WTEnable\0")?),
                overlap: std::mem::transmute::<*const std::ffi::c_void, WtOverlap>(proc(b"WTOverlap\0")?),
            })
        }
    }

    /// Opens a context on `hwnd` (no-op when one is open). Must run on the window's thread.
    pub fn open(hwnd: Hwnd) -> Status {
        let mut s = shared();
        if s.ctx != 0 {
            return Status::Open;
        }
        let fns = match s.fns.or_else(load) {
            Some(f) => f,
            None => {
                s.status = Status::NoDriver;
                return s.status;
            }
        };
        s.fns = Some(fns);
        // SAFETY: WTInfoW with a null buffer returns the size it would write; the buffers below are at least that large
        // (checked) and writable.
        unsafe {
            let need = (fns.info)(WTI_DEFSYSCTX, 0, std::ptr::null_mut());
            if need == 0 || need as usize > size_of::<LogContext>() {
                s.status = Status::OpenFailed;
                return s.status;
            }
            let mut lc: LogContext = std::mem::zeroed();
            if (fns.info)(WTI_DEFSYSCTX, 0, (&mut lc as *mut LogContext).cast()) == 0 {
                s.status = Status::OpenFailed;
                return s.status;
            }
            lc.options |= CXO_MESSAGES;
            lc.pkt_data = PK_CURSOR | PK_BUTTONS | PK_NORMAL_PRESSURE;
            lc.move_mask = lc.pkt_data;
            lc.pkt_mode = 0;
            lc.msg_base = WT_DEFBASE;
            let mut axis = Axis::default();
            let axis_size = (fns.info)(WTI_DEVICES, DVC_NPRESSURE, std::ptr::null_mut());
            s.pressure_max = if axis_size as usize <= size_of::<Axis>()
                && (fns.info)(WTI_DEVICES, DVC_NPRESSURE, (&mut axis as *mut Axis).cast()) != 0
            {
                axis.max
            } else {
                0
            };
            let ctx = (fns.open)(hwnd, &mut lc, 1);
            if ctx == 0 {
                s.status = Status::OpenFailed;
                return s.status;
            }
            s.ctx = ctx;
        }
        s.pen = WintabPen::default();
        s.status = Status::Open;
        s.status
    }

    /// Closes the context (no-op without one).
    pub fn close() {
        let mut s = shared();
        if s.ctx != 0
            && let Some(fns) = s.fns
        {
            // SAFETY: `ctx` was opened by `WTOpenW` and is closed once.
            unsafe { (fns.close)(s.ctx) };
        }
        s.ctx = 0;
        s.pen = WintabPen::default();
        s.status = Status::Closed;
    }

    pub fn status() -> Status {
        shared().status
    }

    /// The pen of the newest packets (default while no context is open).
    pub fn poll() -> WintabPen {
        let s = shared();
        if s.ctx == 0 { WintabPen::default() } else { s.pen }
    }

    /// The subclass's hook (`win.rs`): `WT_PACKET` drains the queued packets into the pen state, `WT_PROXIMITY` sets the
    /// range, `WM_ACTIVATE` enables the context (and puts it on top) while the window is active. Other messages and a
    /// closed context: nothing.
    pub fn on_message(msg: u32, wparam: usize, lparam: isize) {
        if !matches!(msg, WT_PACKET | WT_PROXIMITY | WM_ACTIVATE) {
            return;
        }
        let mut s = shared();
        let (ctx, Some(fns)) = (s.ctx, s.fns) else { return };
        if ctx == 0 {
            return;
        }
        match msg {
            WT_PACKET if lparam == ctx => {
                let mut buf = [Packet::default(); BATCH];
                let mut newest: Vec<Packet> = Vec::new();
                loop {
                    // SAFETY: `buf` holds BATCH packets of the context's 12-byte layout; the call writes at most BATCH.
                    let n = unsafe { (fns.packets_get)(ctx, BATCH as i32, buf.as_mut_ptr().cast()) };
                    if n <= 0 {
                        break;
                    }
                    newest.clear();
                    newest.extend_from_slice(&buf[..(n as usize).min(BATCH)]);
                    if (n as usize) < BATCH {
                        break;
                    }
                }
                let in_range = s.pen.in_range || !newest.is_empty();
                s.pen = wintab_state(&newest, s.pressure_max, in_range, s.pen);
            }
            WT_PROXIMITY if wparam as isize == ctx => {
                let entering = lparam & 0xFFFF != 0;
                s.pen = WintabPen { in_range: entering, tip_down: s.pen.tip_down && entering, ..s.pen };
            }
            WM_ACTIVATE => {
                let active = wparam & 0xFFFF != 0;
                // SAFETY: an open context of this process.
                unsafe {
                    (fns.enable)(ctx, i32::from(active));
                    if active {
                        (fns.overlap)(ctx, 1);
                    }
                }
            }
            _ => {}
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn layouts() {
            assert_eq!(std::mem::offset_of!(LogContext, options), 80);
            assert_eq!(std::mem::offset_of!(LogContext, msg_base), 92);
            assert_eq!(std::mem::offset_of!(LogContext, pkt_data), 104);
            assert_eq!(std::mem::offset_of!(LogContext, move_mask), 112);
            assert_eq!(std::mem::offset_of!(LogContext, rest), 124);
        }

        #[test]
        fn closed_context_reports_nothing() {
            assert_eq!(poll(), WintabPen::default());
            on_message(WT_PACKET, 0, 0);
            assert_eq!(poll(), WintabPen::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(cursor: u32, buttons: u32, pressure: u32) -> Packet {
        Packet { cursor, buttons, pressure }
    }

    #[test]
    fn the_newest_packet_decides() {
        let none = WintabPen::default();
        let s = wintab_state(&[pk(1, 0, 0), pk(1, 1, 512)], 1023, true, none);
        assert!(s.tip_down && s.in_range && !s.inverted);
        assert!((s.pressure - 512.0 / 1023.0).abs() < 1e-6);
        let eraser = wintab_state(&[pk(2, 1, 1023)], 1023, true, none);
        assert!(eraser.inverted && eraser.tip_down && eraser.pressure == 1.0);
        assert!(wintab_state(&[pk(5, 0, 0)], 1023, true, none).inverted, "cursor 5 is an eraser too");
        assert!(!wintab_state(&[pk(4, 0, 0)], 1023, true, none).inverted);
        // Zero or negative maximum: 1023; clamped at 1.
        assert!((wintab_state(&[pk(1, 1, 1023)], 0, true, none).pressure - 1.0).abs() < 1e-6);
        assert_eq!(wintab_state(&[pk(1, 1, 9000)], 2047, true, none).pressure, 1.0);
        // No packets: the previous state, new range.
        let kept = wintab_state(&[], 1023, false, eraser);
        assert_eq!(kept, WintabPen { in_range: false, ..eraser });
    }

    #[test]
    fn spread_ends_at_now() {
        let v = spread(0.2, 0.6, 4);
        assert!(v.iter().zip([0.3f32, 0.4, 0.5, 0.6]).all(|(a, b)| (a - b).abs() < 1e-6), "{v:?}");
        assert_eq!(v[3], 0.6, "the last value is now itself");
        assert_eq!(spread(0.5, 0.5, 3), vec![0.5; 3]);
        assert_eq!(spread(0.0, 1.0, 1), vec![1.0]);
        assert!(spread(0.3, 0.9, 0).is_empty());
        let v = spread(0.1, 0.9, 7);
        assert!(v.windows(2).all(|w| w[0] < w[1]) && v[6] == 0.9);
    }

    fn moved(x: f32) -> egui::Event {
        egui::Event::PointerMoved(egui::pos2(x, 0.0))
    }

    fn press(x: f32, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::pos2(x, 0.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn mouse_events_get_wintab_pressures_while_the_tip_is_down() {
        let events = vec![moved(1.0), press(1.0, true), moved(2.0), moved(3.0)];
        let mut frame = PenFrame::default();
        let mut ramp = WintabRamp::default();
        let tab = WintabPen { in_range: true, tip_down: true, pressure: 0.4, inverted: false };
        apply(&mut frame, &events, &tab, &mut ramp);
        let p: Vec<Option<f32>> = frame.pointer.iter().map(|(_, p)| *p).collect();
        assert_eq!(p, vec![Some(0.4); 4], "a new contact starts at its own pressure");
        assert!(frame.in_contact && frame.in_range && frame.pressure == 0.4);
        // The next frame ramps from 0.4 to 0.8 over its events.
        let mut frame = PenFrame::default();
        let tab = WintabPen { pressure: 0.8, ..tab };
        apply(&mut frame, &[moved(4.0), moved(5.0)], &tab, &mut ramp);
        let p: Vec<f32> = frame.pointer.iter().map(|(_, p)| p.unwrap()).collect();
        assert!((p[0] - 0.6).abs() < 1e-6 && p[1] == 0.8, "{p:?}");
        // A press takes the first value of the ramp.
        let mut frame = PenFrame::default();
        let tab = WintabPen { pressure: 0.2, ..tab };
        apply(&mut frame, &[moved(6.0), press(6.0, true)], &tab, &mut ramp);
        assert_eq!(frame.pointer[1].1, frame.pointer[0].1);
    }

    #[test]
    fn an_ink_pen_with_wintab_gives_one_stroke_with_wintab_pressure() {
        // The router already turned the Ink pen into one stream of pointer events with Ink pressures; WinTab replaces
        // the pressures and adds no event.
        let events = vec![moved(1.0), press(1.0, true), moved(2.0)];
        let mut frame = PenFrame {
            pointer: vec![(egui::pos2(1.0, 0.0), Some(0.9)), (egui::pos2(1.0, 0.0), Some(0.9)), (egui::pos2(2.0, 0.0), Some(0.9))],
            in_range: true,
            in_contact: true,
            pressure: 0.9,
            ..PenFrame::default()
        };
        let tab = WintabPen { in_range: true, tip_down: true, pressure: 0.3, inverted: false };
        apply(&mut frame, &events, &tab, &mut WintabRamp::default());
        assert_eq!(frame.pointer.len(), 3, "no event added");
        assert!(frame.pointer.iter().all(|(_, p)| *p == Some(0.3)));
        assert_eq!(frame.pressure, 0.3);
    }

    #[test]
    fn without_a_wintab_pen_nothing_changes() {
        let events = vec![moved(1.0), press(1.0, true)];
        let mut frame = PenFrame::default();
        apply(&mut frame, &events, &WintabPen::default(), &mut WintabRamp::default());
        assert_eq!(frame, PenFrame::default(), "mouse events stay untagged");
        // Hovering (in range, tip up): range and eraser flag only.
        let hover = WintabPen { in_range: true, tip_down: false, pressure: 0.0, inverted: true };
        apply(&mut frame, &events, &hover, &mut WintabRamp::default());
        assert!(frame.pointer.is_empty() && frame.in_range && frame.inverted && !frame.in_contact);
    }
}
