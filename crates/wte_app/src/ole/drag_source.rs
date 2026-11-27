//! Stickers P11 (STICKER_PLAN 6.4, 6.1; ARCHITECTURE 8.17): the OLE side of drag out. The app never calls `DoDragDrop`
//! inside a frame (critic C4): when a prepared drag should become an OS drag, `dragout.rs` stores it here as a
//! [`DragJob`] and posts the registered message `WTE.OleDrag.v1` to the app window ([`request`]). WTE's window-procedure
//! subclass (`subclass.rs`) handles that message outside winit's event handler ([`handle_message`]): it takes the job,
//! sets [`active`] (a second message while a drag runs drops its job), hands WTE's own `IDataObject` and an
//! `IDropSource` to `DoDragDrop`, clears the flag and queues a [`super::DragEnded`] (`ole::take_drag_ended`), from
//! which `raw_input_hook` injects the pointer-up egui never got (OLE ate the button release).
//!
//! `DoDragDrop` is behind the [`DoDrag`] seam: the real one (`window_seam`) is compiled only into a non-test Windows
//! build; every test and every self-test run uses the [`Recorder`], which reads the data object as a drop target would
//! (`EnumFormatEtc`, `GetData` of each format), asks the drop source the three `QueryContinueDrag` questions and
//! answers with a canned effect. In a self-test run the "post" is [`post_for_test`] and the runner's frame delivers it
//! ([`deliver_test_post`]) where the subclass would.
//!
//! `IDropSource`: `QueryContinueDrag` cancels on Esc, drops when the primary button is up (`grfKeyState`), else
//! continues; `GiveFeedback` uses the default cursors. Every body is inside `catch_unwind`; `QueryInterface` answers
//! `IUnknown` and `IDropSource`. The offered effect is `DROPEFFECT_COPY` only.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use super::DragEnded;
use super::com::*;
use super::data_object::{DataObject, Entry};

/// The registered message that starts a posted drag (6.1: not a fixed `WM_APP + n`).
pub const MESSAGE_NAME: &str = "WTE.OleDrag.v1";

/// A prepared drag: the data object's entries, in the order they are offered, and the app's tag for it.
pub struct DragJob {
    pub tag: u64,
    pub entries: Vec<Entry>,
}

/// Why a request did not reach the window.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A drag is running (`DRAG_ACTIVE`): the request is dropped.
    Busy,
    /// `PostMessageW` failed (or there is no window).
    NotPosted,
}

static SLOT: Mutex<Option<DragJob>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static MESSAGE: AtomicU32 = AtomicU32::new(0);
static TEST_POSTED: AtomicBool = AtomicBool::new(false);

fn slot() -> std::sync::MutexGuard<'static, Option<DragJob>> {
    SLOT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The id of `WTE.OleDrag.v1` (registered at the first call; the same id in every process for the same name).
pub fn message_id() -> u32 {
    let id = MESSAGE.load(Ordering::Acquire);
    if id != 0 {
        return id;
    }
    let wide: Vec<u16> = MESSAGE_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a NUL-terminated name that lives across the call.
    let id = unsafe { RegisterWindowMessageW(wide.as_ptr()) };
    MESSAGE.store(id, Ordering::Release);
    id
}

/// The id if it was registered already (the subclass looks at every message; it never registers), else 0.
pub fn registered_id() -> u32 {
    MESSAGE.load(Ordering::Acquire)
}

/// True while `DoDragDrop` runs (`DRAG_ACTIVE`; a request then is dropped).
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

thread_local! {
    /// Set while this thread runs a drag (`handle_message`). `DoDragDrop`'s modal loop calls WTE's own drop target on
    /// this same thread (OLE's apartment), so the target asks this rather than the process-wide flag (and the tests,
    /// which run on many threads at once, cannot see each other's drags).
    static OWN_DRAG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True while this thread runs one of WTE's drags: WTE's drop target then takes nothing (R3).
pub fn own_drag_on_this_thread() -> bool {
    OWN_DRAG.with(std::cell::Cell::get)
}

/// Stores `job` (replacing one that was not started yet) and posts the drag message with `post`. A request while a drag
/// runs is dropped (`Busy`); a failed post takes the job back (`NotPosted`).
pub fn request(job: DragJob, post: &mut dyn FnMut() -> bool) -> Result<(), Refused> {
    if active() {
        return Err(Refused::Busy);
    }
    *slot() = Some(job);
    if post() {
        Ok(())
    } else {
        slot().take();
        Err(Refused::NotPosted)
    }
}

/// The posted message of a non-test Windows build: `PostMessageW(window, WTE.OleDrag.v1)`.
#[cfg_attr(test, allow(dead_code, reason = "the real post of an interactive run"))]
pub fn post_to_window(hwnd: Hwnd) -> bool {
    let msg = message_id();
    // SAFETY: plain user32 call with our own window and message.
    hwnd != 0 && msg != 0 && unsafe { PostMessageW(hwnd, msg, 0, 0) } != 0
}

/// The "post" of a self-test run and of the tests: the frame delivers it (`deliver_test_post`).
pub fn post_for_test() -> bool {
    TEST_POSTED.store(true, Ordering::Release);
    true
}

/// Delivers a test post (a self-test run's frame, where the subclass would get the real message).
pub fn deliver_test_post(seam: &dyn DoDrag) -> Option<DragEnded> {
    if TEST_POSTED.swap(false, Ordering::AcqRel) { handle_message(seam) } else { None }
}

/// The seam around `DoDragDrop(data, source, DROPEFFECT_COPY, &effect)`: `(result, effect)`.
pub trait DoDrag {
    /// # Safety contract
    /// `data` and `source` are live COM objects for the duration of the call.
    fn do_drag(&self, data: *mut c_void, source: *mut c_void) -> (Hresult, u32);
}

/// The drag message (the subclass, or a test post): takes the job, runs the drag through `seam` with the flag set, and
/// queues and returns what it ended with. `None` without a job, or when a drag is running (the job is dropped).
pub fn handle_message(seam: &dyn DoDrag) -> Option<DragEnded> {
    let job = slot().take()?;
    if ACTIVE.swap(true, Ordering::AcqRel) {
        log::warn!("drag out: a drag is running; the second one is dropped");
        return None;
    }
    let tag = job.tag;
    RUNNING_TAG.store(tag, Ordering::Release);
    let data = DataObject::create(job.entries, None);
    let source = DropSource::create();
    OWN_DRAG.with(|c| c.set(true));
    let (hr, effect) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| seam.do_drag(data, source))).unwrap_or((E_UNEXPECTED, DROPEFFECT_NONE));
    OWN_DRAG.with(|c| c.set(false));
    // SAFETY: the references `create` gave us; OLE has released its own by now.
    unsafe {
        release(data);
        release(source);
    }
    ACTIVE.store(false, Ordering::Release);
    let effect = if hr == DRAGDROP_S_DROP { effect } else { DROPEFFECT_NONE };
    log::info!("drag out: DoDragDrop ended ({hr:#010x}, effect {effect})");
    let ended = DragEnded { tag, effect };
    super::push_drag_ended(ended);
    Some(ended)
}

/// The seam the window's subclass uses: `DoDragDrop` in a non-test build, the recorder in a test build.
#[cfg(not(test))]
pub fn window_seam() -> &'static dyn DoDrag {
    &real::RealDoDrag
}

/// The seam the window's subclass uses: `DoDragDrop` in a non-test build, the recorder in a test build.
#[cfg(test)]
pub fn window_seam() -> &'static dyn DoDrag {
    static RECORDER: Recorder = Recorder { effect: DROPEFFECT_COPY };
    &RECORDER
}

#[cfg(not(test))]
mod real {
    use std::ffi::c_void;

    use super::{DROPEFFECT_COPY, DoDrag, Hresult};

    #[link(name = "ole32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn DoDragDrop(data: *mut c_void, source: *mut c_void, ok_effects: u32, effect: *mut u32) -> Hresult;
    }

    /// The real `DoDragDrop` (a modal loop until the drop or the cancel; the window's thread, OLE initialised by winit).
    pub struct RealDoDrag;

    impl DoDrag for RealDoDrag {
        fn do_drag(&self, data: *mut c_void, source: *mut c_void) -> (Hresult, u32) {
            let mut effect = 0u32;
            // SAFETY: live objects of ours; a valid out pointer.
            let hr = unsafe { DoDragDrop(data, source, DROPEFFECT_COPY, &mut effect) };
            (hr, effect)
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// IDropSource
// ---------------------------------------------------------------------------------------------------------------------

/// WTE's `IDropSource` (`#[repr(C)]`: the vtable pointer first).
#[repr(C)]
pub struct DropSource {
    vtbl: *const IDropSourceVtbl,
    refs: AtomicU32,
}

static DROP_SOURCE_VTBL: IDropSourceVtbl = IDropSourceVtbl {
    base: IUnknownVtbl { query_interface: source_qi, add_ref: source_add_ref, release: source_release },
    query_continue_drag: source_query_continue,
    give_feedback: source_give_feedback,
};

impl DropSource {
    /// A new object with one reference (the caller's), as a COM pointer.
    pub fn create() -> *mut c_void {
        Box::into_raw(Box::new(DropSource { vtbl: &DROP_SOURCE_VTBL, refs: AtomicU32::new(1) })).cast()
    }
}

/// `QueryContinueDrag`'s answer: Esc cancels, the primary button up drops, else the drag goes on.
pub fn continue_answer(escape_pressed: bool, keys: u32) -> Hresult {
    if escape_pressed {
        DRAGDROP_S_CANCEL
    } else if keys & MK_LBUTTON == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn source_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if out.is_null() || iid.is_null() {
            return E_POINTER;
        }
        // SAFETY: `this` is a `DropSource`; the caller's pointers are valid.
        unsafe {
            let iid = *iid;
            if iid == IID_IUNKNOWN || iid == IID_IDROPSOURCE {
                source_add_ref(this);
                *out = this;
                S_OK
            } else {
                *out = std::ptr::null_mut();
                E_NOINTERFACE
            }
        }
    }))
    .unwrap_or(E_UNEXPECTED)
}

unsafe extern "system" fn source_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `DropSource`.
    unsafe { (*(this as *const DropSource)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn source_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `DropSource`; the last reference frees the box made by `create`.
    unsafe {
        let left = (*(this as *const DropSource)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(Box::from_raw(this as *mut DropSource));
        }
        left
    }
}

unsafe extern "system" fn source_query_continue(_this: *mut c_void, escape_pressed: i32, keys: u32) -> Hresult {
    std::panic::catch_unwind(|| continue_answer(escape_pressed != 0, keys)).unwrap_or(DRAGDROP_S_CANCEL)
}

unsafe extern "system" fn source_give_feedback(_this: *mut c_void, _effect: u32) -> Hresult {
    DRAGDROP_S_USEDEFAULTCURSORS
}

// ---------------------------------------------------------------------------------------------------------------------
// The recorder (tests and self-test runs)
// ---------------------------------------------------------------------------------------------------------------------

/// One drag the [`Recorder`] saw: every offered format (by name, in the enumerator's order) with its bytes, and whether
/// the drop source answered the three `QueryContinueDrag` questions as 6.4 says.
#[derive(Clone, Debug)]
pub struct RecordedDrag {
    pub tag: u64,
    pub formats: Vec<(String, Vec<u8>)>,
    pub source_ok: bool,
    pub effect: u32,
}

impl RecordedDrag {
    /// The bytes of the format `name`.
    pub fn bytes(&self, name: &str) -> Option<&[u8]> {
        self.formats.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice())
    }

    /// The format names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.formats.iter().map(|(n, _)| n.as_str()).collect()
    }
}

static RECORDED: Mutex<Vec<RecordedDrag>> = Mutex::new(Vec::new());

/// The drags recorded so far (taken).
#[cfg(test)]
pub fn take_recorded() -> Vec<RecordedDrag> {
    std::mem::take(&mut *RECORDED.lock().unwrap_or_else(PoisonError::into_inner))
}

/// A copy of the drags recorded so far.
pub fn recorded() -> Vec<RecordedDrag> {
    RECORDED.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// `DoDragDrop`'s stand-in: reads the data object like a drop target, checks the drop source, answers `effect`
/// (`DROPEFFECT_NONE` = the user pressed Esc: `DRAGDROP_S_CANCEL`). Never shows anything, never touches the OS
/// drag loop.
pub struct Recorder {
    pub effect: u32,
}

/// The name of an offered format, as `ole::clipboard::ClipFormat::name` spells it.
fn format_name(id: u16) -> String {
    let f = super::drop_target::Formats::get();
    match id {
        super::formats::CF_HDROP => "CF_HDROP".to_owned(),
        super::formats::CF_DIBV5 => "CF_DIBV5".to_owned(),
        super::formats::CF_DIB => "CF_DIB".to_owned(),
        _ if id == f.png => super::formats::NAME_PNG.to_owned(),
        _ if id == f.sticker => super::formats::NAME_STICKER.to_owned(),
        _ => format!("format {id:#06x}"),
    }
}

impl DoDrag for Recorder {
    fn do_drag(&self, data: *mut c_void, source: *mut c_void) -> (Hresult, u32) {
        // SAFETY: live objects for the duration of the call (`DoDrag`'s contract); every medium `GetData` fills is
        // released.
        let (formats, source_ok) = unsafe {
            let dvt = &*(*(data as *mut ComObject<IDataObjectVtbl>)).vtbl;
            let mut enumerator: *mut c_void = std::ptr::null_mut();
            let mut formats = Vec::new();
            if (dvt.enum_format_etc)(data, DATADIR_GET, &mut enumerator) == S_OK && !enumerator.is_null() {
                let evt = &*(*(enumerator as *mut ComObject<IEnumFormatEtcVtbl>)).vtbl;
                loop {
                    let mut f = FormatEtc::new(0, 0);
                    let mut fetched = 0u32;
                    if (evt.next)(enumerator, 1, &mut f, &mut fetched) != S_OK || fetched == 0 {
                        break;
                    }
                    let ask = FormatEtc::new(f.cf_format, TYMED_HGLOBAL);
                    let mut medium = StgMedium::default();
                    let mut bytes = Vec::new();
                    if (dvt.get_data)(data, &ask, &mut medium) >= 0 {
                        if medium.tymed == TYMED_HGLOBAL {
                            let mem = medium.data as Hglobal;
                            let p = GlobalLock(mem);
                            if !p.is_null() {
                                bytes = std::slice::from_raw_parts(p.cast::<u8>(), GlobalSize(mem)).to_vec();
                                GlobalUnlock(mem);
                            }
                        }
                        ReleaseStgMedium(&mut medium);
                    }
                    formats.push((format_name(f.cf_format), bytes));
                }
                release(enumerator);
            }
            let svt = &*(*(source as *mut ComObject<IDropSourceVtbl>)).vtbl;
            let source_ok = (svt.query_continue_drag)(source, 0, MK_LBUTTON) == S_OK
                && (svt.query_continue_drag)(source, 1, MK_LBUTTON) == DRAGDROP_S_CANCEL
                && (svt.query_continue_drag)(source, 0, 0) == DRAGDROP_S_DROP
                && (svt.give_feedback)(source, DROPEFFECT_COPY) == DRAGDROP_S_USEDEFAULTCURSORS;
            (formats, source_ok)
        };
        let rec = RecordedDrag { tag: slot_tag_hint(), formats, source_ok, effect: self.effect };
        log::info!("drag out (the recorder, no OS drag): drag {} offers {}; effect {}", rec.tag, rec.names().join(", "), rec.effect);
        RECORDED.lock().unwrap_or_else(PoisonError::into_inner).push(rec);
        if self.effect == DROPEFFECT_NONE { (DRAGDROP_S_CANCEL, DROPEFFECT_NONE) } else { (DRAGDROP_S_DROP, self.effect) }
    }
}

/// The tag of the drag being run (`handle_message` keeps it for the recorder).
fn slot_tag_hint() -> u64 {
    RUNNING_TAG.load(Ordering::Acquire)
}

static RUNNING_TAG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Tests: the global slot, flag and recorder are shared; tests that use them run one at a time.
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ole::formats;

    fn job(tag: u64) -> DragJob {
        let f = crate::ole::drop_target::Formats::get();
        DragJob {
            tag,
            entries: vec![
                Entry::global(formats::CF_HDROP, formats::build_dropfiles(&[std::path::PathBuf::from(r"C:\x\Badge.png")])),
                Entry::global(f.png, b"\x89PNG-bytes".to_vec()),
            ],
        }
    }

    /// The request posts once; the message runs the drag through the seam with the flag set, the data object offers
    /// the entries in order with their bytes, the drop source answers Esc / button up / held as 6.4 says, and the
    /// end is queued with the effect.
    #[test]
    fn a_request_posts_and_the_message_runs_one_drag() {
        let _one = test_lock();
        take_recorded();
        crate::ole::take_drag_ended();
        let mut posts = 0;
        assert_eq!(request(job(71), &mut || {
            posts += 1;
            true
        }), Ok(()));
        assert_eq!(posts, 1);
        let ended = handle_message(&Recorder { effect: DROPEFFECT_COPY }).expect("the drag ran");
        assert_eq!(ended, DragEnded { tag: 71, effect: DROPEFFECT_COPY });
        assert!(!active(), "the flag is cleared after the drag");
        let rec = take_recorded();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].tag, 71);
        assert_eq!(rec[0].names(), ["CF_HDROP", formats::NAME_PNG]);
        assert_eq!(formats::parse_dropfiles(rec[0].bytes("CF_HDROP").unwrap(), 8), [std::path::PathBuf::from(r"C:\x\Badge.png")]);
        assert_eq!(rec[0].bytes(formats::NAME_PNG), Some(&b"\x89PNG-bytes"[..]));
        assert!(rec[0].source_ok);
        assert_eq!(crate::ole::take_drag_ended().into_iter().filter(|e| e.tag == 71).count(), 1);
        // A second message without a new request does nothing.
        assert_eq!(handle_message(&Recorder { effect: DROPEFFECT_COPY }), None);
        // Esc: the effect is none.
        request(job(72), &mut || true).unwrap();
        assert_eq!(handle_message(&Recorder { effect: DROPEFFECT_NONE }), Some(DragEnded { tag: 72, effect: DROPEFFECT_NONE }));
        take_recorded();
        crate::ole::take_drag_ended();
    }

    /// A seam that requests a second drag while the first one runs (as a frame inside the modal loop could): the second
    /// request is dropped (`Busy`) and the message it would post finds no job.
    struct Reentrant;

    impl DoDrag for Reentrant {
        fn do_drag(&self, _data: *mut c_void, _source: *mut c_void) -> (Hresult, u32) {
            assert!(active());
            assert_eq!(request(job(82), &mut || panic!("a busy request never posts")), Err(Refused::Busy));
            assert_eq!(handle_message(&Recorder { effect: DROPEFFECT_COPY }), None, "no second drag inside the first");
            (DRAGDROP_S_DROP, DROPEFFECT_COPY)
        }
    }

    #[test]
    fn a_second_request_while_a_drag_runs_is_dropped() {
        let _one = test_lock();
        request(job(81), &mut || true).unwrap();
        assert_eq!(handle_message(&Reentrant), Some(DragEnded { tag: 81, effect: DROPEFFECT_COPY }));
        assert!(!active());
        assert!(slot().is_none(), "nothing is left to start");
        // A failed post takes the job back.
        assert_eq!(request(job(83), &mut || false), Err(Refused::NotPosted));
        assert!(slot().is_none());
        // A panic inside the seam is caught: the flag is cleared and the end says "none".
        struct Panics;
        impl DoDrag for Panics {
            fn do_drag(&self, _d: *mut c_void, _s: *mut c_void) -> (Hresult, u32) {
                panic!("test panic inside DoDragDrop's seam")
            }
        }
        request(job(84), &mut || true).unwrap();
        assert_eq!(handle_message(&Panics), Some(DragEnded { tag: 84, effect: DROPEFFECT_NONE }));
        assert!(!active());
        take_recorded();
        crate::ole::take_drag_ended();
    }

    #[test]
    fn the_drop_source_answers() {
        assert_eq!(continue_answer(true, MK_LBUTTON), DRAGDROP_S_CANCEL);
        assert_eq!(continue_answer(false, MK_LBUTTON), S_OK);
        assert_eq!(continue_answer(false, 0), DRAGDROP_S_DROP);
        assert_eq!(continue_answer(false, 0x0002), DRAGDROP_S_DROP, "only the primary button holds the drag");
        let s = DropSource::create();
        // SAFETY: our own object; QueryInterface adds a reference that is given back.
        unsafe {
            let vt = &*(*(s as *mut ComObject<IDropSourceVtbl>)).vtbl;
            let mut out = std::ptr::null_mut();
            assert_eq!((vt.base.query_interface)(s, &IID_IDROPSOURCE, &mut out), S_OK);
            assert_eq!(out, s);
            assert_eq!(release(out), 1);
            assert_eq!((vt.base.query_interface)(s, &IID_IDATAOBJECT, &mut out), E_NOINTERFACE);
            assert!(out.is_null());
            assert_eq!(release(s), 0);
        }
    }

    #[test]
    fn the_message_is_registered_once() {
        let a = message_id();
        assert!(a >= 0xC000, "registered messages are 0xC000..=0xFFFF: {a:#x}");
        assert_eq!(message_id(), a);
        assert_eq!(registered_id(), a);
    }
}
