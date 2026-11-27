//! Stickers P10b (STICKER_PLAN 6.1, 6.3; ARCHITECTURE 8.17): WTE's second window-procedure subclass, installed by
//! `ole::install` above the pen subclass (`pen/win.rs`). egui-winit turns Ctrl+V / Shift+Insert into an
//! `Event::Paste` only when its own in-app string is not empty and never into a key event, so with a picture on the
//! Windows clipboard nothing would reach the app. The subclass sees `WM_KEYDOWN` before winit does: for the paste
//! chords it queues `OleEvent::Chord(Paste)` (`raw_input_hook` handles it in the frame those keys make) and then
//! forwards the message **unchanged** to the previous procedure, whose result it returns. It never eats a message,
//! and every other message goes straight through. (P11 adds the drag message here.)
//!
//! The chords, as egui-winit reads them (`is_paste_command`): V with Ctrl down and Shift and Alt up (Ctrl+Alt is
//! AltGr on many keyboards: never a paste), Insert with Shift down and Ctrl and Alt up (Ctrl+Insert is Copy). Key
//! states come from `GetKeyState`, which is the state of the keyboard when this very message was posted. Nothing here
//! may panic across the FFI boundary: the chord is found inside `catch_unwind` and the message is forwarded whatever
//! happened.

use std::sync::atomic::{AtomicIsize, Ordering};

use super::Chord;
use super::com::Hwnd;

const GWLP_WNDPROC: i32 = -4;
const WM_NCDESTROY: u32 = 0x0082;
pub(crate) const WM_KEYDOWN: u32 = 0x0100;
pub(crate) const VK_SHIFT: i32 = 0x10;
pub(crate) const VK_CONTROL: i32 = 0x11;
pub(crate) const VK_MENU: i32 = 0x12;
pub(crate) const VK_INSERT: i32 = 0x2D;
pub(crate) const VK_V: i32 = 0x56;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    fn CallWindowProcW(prev: isize, hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn GetKeyState(key: i32) -> i16;
}

/// The previous window procedure and the subclassed window; 0 = not installed.
static PREV: AtomicIsize = AtomicIsize::new(0);
static HWND: AtomicIsize = AtomicIsize::new(0);

/// The clipboard chord a message is, given the key states (`down(vk)`): only `WM_KEYDOWN` of the paste chords.
pub(crate) fn chord_of(msg: u32, wparam: usize, down: &dyn Fn(i32) -> bool) -> Option<Chord> {
    if msg != WM_KEYDOWN {
        return None;
    }
    let vk = i32::try_from(wparam & 0xFFFF).ok()?;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    match vk {
        VK_V if ctrl && !shift && !alt => Some(Chord::Paste),
        VK_INSERT if shift && !ctrl && !alt => Some(Chord::Paste),
        _ => None,
    }
}

/// One message through the subclass: its chord (if any) to `sink`, then the message unchanged to `forward`, whose
/// result is returned. A panic while looking at the message is caught; the message is forwarded in every case.
pub(crate) fn route(
    msg: u32,
    wparam: usize,
    lparam: isize,
    down: &dyn Fn(i32) -> bool,
    sink: &dyn Fn(Chord),
    forward: &mut dyn FnMut(u32, usize, isize) -> isize,
) -> isize {
    if msg == WM_KEYDOWN {
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| chord_of(msg, wparam, down)));
        if let Ok(Some(chord)) = found {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(chord)));
        }
    }
    forward(msg, wparam, lparam)
}

fn key_down(vk: i32) -> bool {
    // SAFETY: plain user32 call; the high bit is set while the key is down.
    unsafe { GetKeyState(vk) < 0 }
}

/// Installs the subclass on `hwnd` (once per process; the window's thread). Returns a line for the log.
pub fn install(hwnd: Hwnd) -> Result<String, String> {
    if hwnd == 0 {
        return Err("no window handle".to_owned());
    }
    if HWND.load(Ordering::Acquire) != 0 {
        return Ok("ole: the key subclass is installed already".to_owned());
    }
    // As the pen subclass: the previous procedure is stored before the swap, so a message dispatched right after it
    // always finds it.
    // SAFETY: reading and replacing the window procedure of our own window on its thread; `wndproc` has the `WNDPROC`
    // signature and lives for the whole process.
    let prev = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
    if prev == 0 {
        return Err("GetWindowLongPtrW(GWLP_WNDPROC) failed".to_owned());
    }
    PREV.store(prev, Ordering::Release);
    let proc_: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize = wndproc;
    // SAFETY: see above.
    let replaced = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc_ as usize as isize) };
    if replaced == 0 {
        PREV.store(0, Ordering::Release);
        return Err("SetWindowLongPtrW(GWLP_WNDPROC) failed".to_owned());
    }
    if replaced != prev {
        PREV.store(replaced, Ordering::Release);
    }
    HWND.store(hwnd, Ordering::Release);
    Ok("ole: the key subclass is installed (Ctrl+V and Shift+Insert reach WTE with a picture on the clipboard)".to_owned())
}

/// The subclass procedure (see the module comment).
unsafe extern "system" fn wndproc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    let prev = PREV.load(Ordering::Acquire);
    if msg == WM_NCDESTROY && HWND.load(Ordering::Acquire) == hwnd {
        // The old procedure goes back first; this message and every later one go to it.
        // SAFETY: our own window on its thread; `prev` is the procedure that was there before.
        unsafe {
            if prev != 0 {
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, prev);
            }
        }
        HWND.store(0, Ordering::Release);
    }
    route(msg, wparam, lparam, &key_down, &|chord| super::push(super::OleEvent::Chord(chord)), &mut |m, w, l| {
        if prev == 0 {
            // SAFETY: the default procedure for a window whose previous one is unknown (cannot happen after `install`).
            return unsafe { DefWindowProcW(hwnd, m, w, l) };
        }
        // SAFETY: `prev` is the window's previous procedure, called with the unchanged message.
        unsafe { CallWindowProcW(prev, hwnd, m, w, l) }
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A fake previous procedure: records what it got and answers with a value of its own.
    struct Fake {
        got: RefCell<Vec<(u32, usize, isize)>>,
    }

    fn run(fake: &Fake, msg: u32, wparam: usize, lparam: isize, keys: &[i32]) -> (isize, Vec<Chord>) {
        let chords = RefCell::new(Vec::new());
        let down = |vk: i32| keys.contains(&vk);
        let result = route(msg, wparam, lparam, &down, &|c| chords.borrow_mut().push(c), &mut |m, w, l| {
            fake.got.borrow_mut().push((m, w, l));
            0x5A5A
        });
        (result, chords.into_inner())
    }

    /// The routing on a fake previous procedure (STICKER_PLAN 7 "P10b"): Ctrl+V queues one paste and is forwarded;
    /// Ctrl+Alt (AltGr) queues nothing; Shift+Insert queues; every other message is forwarded unchanged.
    #[test]
    fn paste_chords_are_queued_and_every_message_is_forwarded_unchanged() {
        let fake = Fake { got: RefCell::new(Vec::new()) };
        let lparam = 0x002F_0001;
        assert_eq!(run(&fake, WM_KEYDOWN, VK_V as usize, lparam, &[VK_CONTROL]), (0x5A5A, vec![Chord::Paste]));
        assert_eq!(run(&fake, WM_KEYDOWN, VK_V as usize, lparam, &[VK_CONTROL, VK_MENU]), (0x5A5A, vec![]), "AltGr");
        assert_eq!(run(&fake, WM_KEYDOWN, VK_V as usize, lparam, &[VK_CONTROL, VK_SHIFT]), (0x5A5A, vec![]), "egui-winit: no paste");
        assert_eq!(run(&fake, WM_KEYDOWN, VK_V as usize, lparam, &[]), (0x5A5A, vec![]), "a plain V");
        assert_eq!(run(&fake, WM_KEYDOWN, VK_INSERT as usize, lparam, &[VK_SHIFT]), (0x5A5A, vec![Chord::Paste]));
        assert_eq!(run(&fake, WM_KEYDOWN, VK_INSERT as usize, lparam, &[VK_CONTROL]), (0x5A5A, vec![]), "Ctrl+Insert is Copy");
        assert_eq!(run(&fake, WM_KEYDOWN, VK_INSERT as usize, lparam, &[VK_SHIFT, VK_CONTROL]), (0x5A5A, vec![]));
        // The key-up, a character, a system key, a pointer message, a user message: forwarded, nothing queued.
        let others = [(0x0101, VK_V as usize, 7), (0x0102, 0x16, 9), (0x0104, VK_V as usize, 1 << 29), (0x0246, 3, -5), (0x0400 + 7, usize::MAX, isize::MIN)];
        for (msg, w, l) in others {
            assert_eq!(run(&fake, msg, w, l, &[VK_CONTROL]), (0x5A5A, vec![]), "message {msg:#x}");
        }
        let got = fake.got.borrow();
        assert_eq!(got.len(), 7 + others.len(), "every message reached the previous procedure");
        assert_eq!(got[0], (WM_KEYDOWN, VK_V as usize, lparam));
        for (i, (msg, w, l)) in others.iter().enumerate() {
            assert_eq!(got[7 + i], (*msg, *w, *l), "forwarded unchanged");
        }
    }

    /// A panic while the chord is handled is caught; the message still reaches the previous procedure.
    #[test]
    fn a_panic_in_the_sink_still_forwards() {
        let fake = Fake { got: RefCell::new(Vec::new()) };
        let down = |vk: i32| vk == VK_CONTROL;
        // (The test panic's message is printed; the panic itself is caught.)
        let result = route(WM_KEYDOWN, VK_V as usize, 0, &down, &|_| panic!("test panic in the chord sink"), &mut |m, w, l| {
            fake.got.borrow_mut().push((m, w, l));
            3
        });
        assert_eq!(result, 3);
        assert_eq!(fake.got.borrow().len(), 1);
    }

    static FAKE_CALLS: std::sync::Mutex<Vec<(u32, usize, isize)>> = std::sync::Mutex::new(Vec::new());

    unsafe extern "system" fn fake_prev(_hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
        FAKE_CALLS.lock().unwrap().push((msg, wparam, lparam));
        0x77
    }

    /// The real procedure on a fake previous one (through `CallWindowProcW`, no window): a message that is not a key
    /// reaches it unchanged and its result comes back.
    #[test]
    fn the_window_procedure_forwards_to_the_previous_one() {
        let proc_: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize = fake_prev;
        PREV.store(proc_ as usize as isize, Ordering::Release);
        // SAFETY: the subclass procedure with a fake previous procedure; no window is involved.
        let result = unsafe { wndproc(0, 0x0400 + 11, 42, -42) };
        PREV.store(0, Ordering::Release);
        assert_eq!(result, 0x77);
        assert_eq!(FAKE_CALLS.lock().unwrap().as_slice(), [(0x0400 + 11, 42, -42)]);
    }
}
