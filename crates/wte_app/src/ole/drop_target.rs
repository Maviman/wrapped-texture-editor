//! Stickers P10a (STICKER_PLAN 6.1, 6.2): WTE's `IDropTarget`, registered on the app window in place of winit's (which
//! reads `CF_HDROP` only and drops the point). It accepts, best first: WTE's `WTE.Sticker.v1` (ignored altogether when
//! the process id inside is this process: a drag that started in this WTE, R3), `CF_HDROP`, the registered `PNG`,
//! `CF_DIBV5`, `CF_DIB`, and a virtual file (`FileGroupDescriptorW` + `FileContents`: the first entry whose name is a
//! picture, a `.psd` / `.psb` or a `.wtes`).
//!
//! What each callback does (review A22): `DragEnter` looks at the formats (and reads the small file-name lists) and
//! queues `Hover`; `DragOver` queues `Hover`; `DragLeave` queues `Leave`. **`Drop` copies the data before it returns**
//! (the data object and every `HGLOBAL` / `IStream` it hands out are gone afterwards): `GetData`, copy into a `Vec`,
//! `ReleaseStgMedium` on every path, then `Drop { pos, items, skipped }` is queued. Caps: 64 MiB per item, 256 MiB per
//! drop, 64 items; an `IStream` is read in 1-MiB chunks up to the cap; an item over a cap is not taken and counted in
//! `skipped`. No decoding happens here (`dnd.rs` does it on a job). Every body runs in `catch_unwind`; the object is a
//! leaked box with an atomic count while registered.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use super::com::*;
use super::formats::{self, CF_DIB, CF_DIBV5, CF_HDROP};
use super::{DataKind, DragKinds, DropItem, OleEvent};

/// The size limits of one drop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Caps {
    pub item: usize,
    pub total: usize,
    pub items: usize,
}

/// The limits of 6.1.
pub const CAPS: Caps = Caps { item: 64 << 20, total: 256 << 20, items: 64 };
/// `IStream` reads go in chunks of this size.
const CHUNK: usize = 1 << 20;

/// The registered format ids (the same in every process for the same name).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Formats {
    pub png: u16,
    pub file_group: u16,
    pub file_contents: u16,
    pub sticker: u16,
}

impl Formats {
    /// The ids, registered once per process.
    pub fn get() -> Formats {
        static F: std::sync::OnceLock<Formats> = std::sync::OnceLock::new();
        *F.get_or_init(|| Formats {
            png: register_format(formats::NAME_PNG),
            file_group: register_format(formats::NAME_FILE_GROUP),
            file_contents: register_format(formats::NAME_FILE_CONTENTS),
            sticker: register_format(formats::NAME_STICKER),
        })
    }
}

/// Where the target's events go: `ole::push` for the window's target, a test's list otherwise.
pub type Sink = Box<dyn Fn(OleEvent) + Send + Sync>;

/// WTE's `IDropTarget` (`#[repr(C)]`: the vtable pointer first).
#[repr(C)]
pub struct DropTarget {
    vtbl: *const IDropTargetVtbl,
    refs: AtomicU32,
    /// The window (`ScreenToClient`); 0 = the points are client pixels already (the self-test runner, tests).
    hwnd: Hwnd,
    formats: Formats,
    caps: Caps,
    sink: Sink,
    /// What the drag in progress carries (`None`: nothing WTE takes, or no drag).
    current: Mutex<Option<DragKinds>>,
    /// Tests: the next callback panics (the `catch_unwind` test).
    #[cfg(test)]
    panic_next: std::sync::atomic::AtomicBool,
}

static DROP_TARGET_VTBL: IDropTargetVtbl = IDropTargetVtbl {
    base: IUnknownVtbl { query_interface: target_qi, add_ref: target_add_ref, release: target_release },
    drag_enter: target_drag_enter,
    drag_over: target_drag_over,
    drag_leave: target_drag_leave,
    drop: target_drop,
};

impl DropTarget {
    /// A new target with one reference (the caller's), as a COM pointer.
    pub fn create(hwnd: Hwnd, formats: Formats, caps: Caps, sink: Sink) -> *mut c_void {
        let t = Box::new(DropTarget {
            vtbl: &DROP_TARGET_VTBL,
            refs: AtomicU32::new(1),
            hwnd,
            formats,
            caps,
            sink,
            current: Mutex::new(None),
            #[cfg(test)]
            panic_next: std::sync::atomic::AtomicBool::new(false),
        });
        Box::into_raw(t).cast()
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<DragKinds>> {
        self.current.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The point in client pixels (physical).
    fn client(&self, pt: PointL) -> [f32; 2] {
        let mut p = pt;
        if self.hwnd != 0 {
            // SAFETY: our own window, a valid point.
            unsafe { ScreenToClient(self.hwnd, &mut p) };
        }
        [p.x as f32, p.y as f32]
    }
}

/// Runs a callback body; a panic becomes `E_UNEXPECTED` and the effect `DROPEFFECT_NONE`.
fn guarded(effect: *mut u32, f: impl FnOnce() -> Hresult) -> Hresult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(hr) => hr,
        Err(_) => {
            log::error!("ole: a drop target callback panicked; the drag is refused");
            if !effect.is_null() {
                // SAFETY: OLE's valid out pointer.
                unsafe { *effect = DROPEFFECT_NONE };
            }
            E_UNEXPECTED
        }
    }
}

impl DropTarget {
    /// Tests: panics when the test asked for it (`panic_next`).
    fn test_panic_hook(&self) {
        #[cfg(test)]
        if self.panic_next.swap(false, Ordering::SeqCst) {
            panic!("test hook: a panic inside a drop target callback");
        }
    }
}

/// The effect to answer: copy when the drag carries something WTE takes and the source allows a copy.
///
/// # Safety
/// `effect` is null or OLE's valid in/out pointer.
unsafe fn answer(effect: *mut u32, accept: bool) {
    if effect.is_null() {
        return;
    }
    // SAFETY: see above.
    unsafe {
        let allowed = *effect;
        *effect = if accept && allowed & DROPEFFECT_COPY != 0 { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
    }
}

unsafe extern "system" fn target_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    guarded(std::ptr::null_mut(), || {
        if out.is_null() || iid.is_null() {
            return E_POINTER;
        }
        // SAFETY: `this` is a `DropTarget`; the pointers are valid.
        unsafe {
            let iid = *iid;
            if iid == IID_IUNKNOWN || iid == IID_IDROPTARGET {
                target_add_ref(this);
                *out = this;
                S_OK
            } else {
                *out = std::ptr::null_mut();
                E_NOINTERFACE
            }
        }
    })
}

unsafe extern "system" fn target_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `DropTarget`.
    unsafe { (*(this as *const DropTarget)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn target_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `DropTarget`; the last reference frees the box (never for the window's target, whose
    // own reference is never given back).
    unsafe {
        let left = (*(this as *const DropTarget)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(Box::from_raw(this as *mut DropTarget));
        }
        left
    }
}

unsafe extern "system" fn target_drag_enter(this: *mut c_void, data: *mut c_void, _keys: u32, pt: PointL, effect: *mut u32) -> Hresult {
    guarded(effect, || {
        // SAFETY: `this` is a live `DropTarget`; `data` is OLE's data object for the duration of the call.
        unsafe {
            let t = &*(this as *const DropTarget);
            t.test_panic_hook();
            let kinds = if data.is_null() { None } else { inspect(data, &t.formats, &t.caps) };
            *t.current() = kinds.clone();
            answer(effect, kinds.is_some());
            if let Some(kinds) = kinds {
                (t.sink)(OleEvent::Hover { pos: t.client(pt), kinds });
            }
        }
        S_OK
    })
}

unsafe extern "system" fn target_drag_over(this: *mut c_void, _keys: u32, pt: PointL, effect: *mut u32) -> Hresult {
    guarded(effect, || {
        // SAFETY: `this` is a live `DropTarget`.
        unsafe {
            let t = &*(this as *const DropTarget);
            t.test_panic_hook();
            let kinds = t.current().clone();
            answer(effect, kinds.is_some());
            if let Some(kinds) = kinds {
                (t.sink)(OleEvent::Hover { pos: t.client(pt), kinds });
            }
        }
        S_OK
    })
}

unsafe extern "system" fn target_drag_leave(this: *mut c_void) -> Hresult {
    guarded(std::ptr::null_mut(), || {
        // SAFETY: `this` is a live `DropTarget`.
        unsafe {
            let t = &*(this as *const DropTarget);
            t.test_panic_hook();
            if t.current().take().is_some() {
                (t.sink)(OleEvent::Leave);
            }
        }
        S_OK
    })
}

unsafe extern "system" fn target_drop(this: *mut c_void, data: *mut c_void, _keys: u32, pt: PointL, effect: *mut u32) -> Hresult {
    guarded(effect, || {
        // SAFETY: `this` is a live `DropTarget`; `data` lives for this call only: everything is copied before return.
        unsafe {
            let t = &*(this as *const DropTarget);
            t.test_panic_hook();
            let accepted = t.current().take().is_some();
            if !accepted || data.is_null() {
                answer(effect, false);
                return S_OK;
            }
            let (items, skipped) = read_payload(data, &t.formats, &t.caps);
            answer(effect, !items.is_empty());
            (t.sink)(OleEvent::Drop { pos: t.client(pt), items, skipped });
        }
        S_OK
    })
}

/// The bytes of a `WTE.Sticker.v1` block that say who offers it (magic, version, process id).
const STICKER_HEADER: usize = 12;

/// What `get_bytes` found.
#[derive(Debug, PartialEq)]
enum Read {
    Bytes(Vec<u8>),
    /// Larger than the cap: not taken.
    TooLarge,
    /// Not offered, or `GetData` failed.
    Missing,
}

fn vtbl(data: *mut c_void) -> *const IDataObjectVtbl {
    // SAFETY: OLE's data object starts with its vtable pointer.
    unsafe { (*(data as *mut ComObject<IDataObjectVtbl>)).vtbl }
}

/// True when `data` offers `format` on an `HGLOBAL` or an `IStream`.
///
/// # Safety
/// `data` is a live `IDataObject`.
unsafe fn has(data: *mut c_void, format: u16) -> bool {
    let f = FormatEtc::new(format, TYMED_HGLOBAL | TYMED_ISTREAM);
    // SAFETY: see above.
    format != 0 && unsafe { ((*vtbl(data)).query_get_data)(data, &f) } == S_OK
}

/// The bytes of `format` (index `lindex`), copied out of the medium, which is then released (`ReleaseStgMedium` runs
/// on every path where `GetData` succeeded).
///
/// # Safety
/// `data` is a live `IDataObject`.
unsafe fn get_bytes(data: *mut c_void, format: u16, lindex: i32, cap: usize) -> Read {
    // SAFETY: as the caller's.
    unsafe { get_medium(data, format, lindex, cap, false) }
}

/// At most the first `n` bytes of `format` (stickers P11: the header of a sticker block), the medium released.
///
/// # Safety
/// `data` is a live `IDataObject`.
unsafe fn get_prefix(data: *mut c_void, format: u16, n: usize) -> Read {
    // SAFETY: as the caller's.
    unsafe { get_medium(data, format, -1, n, true) }
}

/// `get_bytes` (`prefix` false: all of it up to `cap`) and `get_prefix` (`prefix` true: the first `cap` bytes).
///
/// # Safety
/// `data` is a live `IDataObject`.
unsafe fn get_medium(data: *mut c_void, format: u16, lindex: i32, cap: usize, prefix: bool) -> Read {
    let f = FormatEtc { lindex, ..FormatEtc::new(format, TYMED_HGLOBAL | TYMED_ISTREAM) };
    let mut medium = StgMedium::default();
    // SAFETY: see above; `medium` is ours to release after a success.
    unsafe {
        // Any success code means the medium was filled and is ours to release (check P10a: a source answering with a
        // success code other than S_OK would have leaked it); a failure leaves it untouched.
        if ((*vtbl(data)).get_data)(data, &f, &mut medium) < 0 {
            return Read::Missing;
        }
        let read = match medium.tymed {
            TYMED_HGLOBAL if prefix => read_global_prefix(medium.data as Hglobal, cap),
            TYMED_ISTREAM if prefix && !medium.data.is_null() => read_stream_prefix(medium.data, cap),
            TYMED_HGLOBAL => read_global(medium.data as Hglobal, cap),
            TYMED_ISTREAM if !medium.data.is_null() => read_stream(medium.data, cap),
            _ => Read::Missing,
        };
        ReleaseStgMedium(&mut medium);
        read
    }
}

/// Copies an `HGLOBAL` (its whole `GlobalSize`).
///
/// # Safety
/// `mem` is a valid global memory block.
unsafe fn read_global(mem: Hglobal, cap: usize) -> Read {
    // SAFETY: see above; the block is read between Lock and Unlock and is `size` bytes long.
    unsafe {
        let size = GlobalSize(mem);
        if size > cap {
            return Read::TooLarge;
        }
        let p = GlobalLock(mem);
        if p.is_null() {
            return Read::Missing;
        }
        let bytes = std::slice::from_raw_parts(p.cast::<u8>(), size).to_vec();
        GlobalUnlock(mem);
        Read::Bytes(bytes)
    }
}

/// Copies the first `n` bytes of an `HGLOBAL` (all of a smaller block): the header of a sticker block.
///
/// # Safety
/// `mem` is a valid global memory block.
unsafe fn read_global_prefix(mem: Hglobal, n: usize) -> Read {
    // SAFETY: see above; at most `GlobalSize` bytes are read between Lock and Unlock.
    unsafe {
        let size = GlobalSize(mem).min(n);
        let p = GlobalLock(mem);
        if p.is_null() {
            return Read::Missing;
        }
        let bytes = std::slice::from_raw_parts(p.cast::<u8>(), size).to_vec();
        GlobalUnlock(mem);
        Read::Bytes(bytes)
    }
}

/// Reads at most the first `n` bytes of an `IStream`.
///
/// # Safety
/// `stream` is a live `IStream`.
unsafe fn read_stream_prefix(stream: *mut c_void, n: usize) -> Read {
    let mut out = vec![0u8; n];
    let mut got = 0usize;
    while got < n {
        let mut read = 0u32;
        // SAFETY: see above; `out[got..]` holds `n - got` writable bytes.
        let hr = unsafe {
            ((*(*(stream as *mut ComObject<IStreamVtbl>)).vtbl).read)(stream, out[got..].as_mut_ptr().cast(), (n - got) as u32, &mut read)
        };
        let read = (read as usize).min(n - got);
        if hr < 0 || read == 0 {
            break;
        }
        got += read;
    }
    out.truncate(got);
    Read::Bytes(out)
}

/// Reads an `IStream` to its end in 1-MiB chunks; more than `cap` bytes is `TooLarge` (an endless stream stops there).
///
/// # Safety
/// `stream` is a live `IStream`.
unsafe fn read_stream(stream: *mut c_void, cap: usize) -> Read {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let mut n = 0u32;
        // SAFETY: see above; `chunk` holds CHUNK writable bytes.
        let hr = unsafe {
            ((*(*(stream as *mut ComObject<IStreamVtbl>)).vtbl).read)(stream, chunk.as_mut_ptr().cast(), CHUNK as u32, &mut n)
        };
        let n = (n as usize).min(CHUNK);
        if hr < 0 || n == 0 {
            break;
        }
        if out.len() + n > cap {
            return Read::TooLarge;
        }
        out.extend_from_slice(&chunk[..n]);
    }
    Read::Bytes(out)
}

/// A file name's extension, lower case.
fn extension(name: &str) -> String {
    crate::dnd::extension_of(std::path::Path::new(name))
}

/// True for a virtual file WTE takes (a picture, a PSD or a sticker file).
fn takes_name(name: &str) -> bool {
    let e = extension(name);
    crate::dnd::IMAGE_EXTENSIONS.contains(&e.as_str()) || matches!(e.as_str(), "psd" | "psb" | wte_core::WTES_EXTENSION)
}

/// What a drag carries (`DragEnter`); `None` = nothing WTE takes, or a drag that started in this WTE.
///
/// # Safety
/// `data` is a live `IDataObject`.
unsafe fn inspect(data: *mut c_void, f: &Formats, caps: &Caps) -> Option<DragKinds> {
    // Stickers P11 (R3): a drag WTE itself runs (`DoDragDrop` of a drag out) is never taken back by WTE, whatever it
    // carries (a texture layer's PNG has no WTE.Sticker.v1 marker).
    if super::drag_source::own_drag_on_this_thread() {
        return None;
    }
    // SAFETY: see above, for every call below.
    unsafe {
        // Stickers P11 (check P10a): only the 12-byte header of a sticker block is copied here (another WTE's drag out
        // offers the whole `.wtes`; `Drop` copies it once).
        if has(data, f.sticker)
            && let Read::Bytes(b) = get_prefix(data, f.sticker, STICKER_HEADER)
            && let Some((pid, _)) = formats::unwrap_sticker(&b)
        {
            if pid == std::process::id() {
                return None;
            }
            return Some(DragKinds { sticker: true, ..DragKinds::default() });
        }
        if has(data, CF_HDROP)
            && let Read::Bytes(b) = get_bytes(data, CF_HDROP, -1, caps.item)
        {
            let names = formats::parse_dropfiles(&b, caps.items);
            if !names.is_empty() {
                let extensions = names.iter().map(|p| crate::dnd::extension_of(p)).collect();
                return Some(DragKinds { extensions, ..DragKinds::default() });
            }
        }
        if has(data, f.png) || has(data, CF_DIBV5) || has(data, CF_DIB) {
            return Some(DragKinds { picture: true, ..DragKinds::default() });
        }
        // `FileContents` is not asked for here: many sources answer its `QueryGetData` only with an index.
        if has(data, f.file_group)
            && let Read::Bytes(b) = get_bytes(data, f.file_group, -1, caps.item)
            && let Some(d) = formats::parse_file_group(&b, caps.items).into_iter().find(|d| takes_name(&d.name))
        {
            return Some(DragKinds { extensions: vec![extension(&d.name)], ..DragKinds::default() });
        }
        None
    }
}

/// The items of a drop: the best format present (falling back to the next when it cannot be read), copied. Returns
/// the items and how many were not taken because of a cap.
///
/// # Safety
/// `data` is a live `IDataObject`.
unsafe fn read_payload(data: *mut c_void, f: &Formats, caps: &Caps) -> (Vec<DropItem>, usize) {
    let cap = caps.item.min(caps.total);
    // SAFETY: see above, for every call below.
    unsafe {
        if has(data, f.sticker) {
            match get_bytes(data, f.sticker, -1, cap) {
                Read::Bytes(b) => {
                    if let Some((pid, wtes)) = formats::unwrap_sticker(&b)
                        && pid != std::process::id()
                    {
                        let item = DropItem::Data { name: "Sticker.wtes".to_owned(), kind: DataKind::Sticker, bytes: wtes.to_vec() };
                        return (vec![item], 0);
                    }
                }
                Read::TooLarge => return (Vec::new(), 1),
                Read::Missing => {}
            }
        }
        if has(data, CF_HDROP) {
            match get_bytes(data, CF_HDROP, -1, cap) {
                Read::Bytes(b) => {
                    let all = formats::parse_dropfiles(&b, usize::MAX);
                    if !all.is_empty() {
                        let skipped = all.len().saturating_sub(caps.items);
                        let items = all.into_iter().take(caps.items).map(DropItem::File).collect();
                        return (items, skipped);
                    }
                }
                Read::TooLarge => return (Vec::new(), 1),
                Read::Missing => {}
            }
        }
        for (format, kind, name) in [
            (f.png, DataKind::Png, "Dropped picture.png"),
            (CF_DIBV5, DataKind::Dib, "Dropped picture.png"),
            (CF_DIB, DataKind::Dib, "Dropped picture.png"),
        ] {
            if !has(data, format) {
                continue;
            }
            match get_bytes(data, format, -1, cap) {
                Read::Bytes(bytes) => return (vec![DropItem::Data { name: name.to_owned(), kind, bytes }], 0),
                Read::TooLarge => return (Vec::new(), 1),
                Read::Missing => {}
            }
        }
        if has(data, f.file_group)
            && let Read::Bytes(b) = get_bytes(data, f.file_group, -1, cap)
            && let Some((index, d)) = formats::parse_file_group(&b, caps.items).into_iter().enumerate().find(|(_, d)| takes_name(&d.name))
        {
            if d.size.is_some_and(|s| s > cap as u64) {
                return (Vec::new(), 1);
            }
            match get_bytes(data, f.file_contents, index as i32, cap) {
                Read::Bytes(mut bytes) => {
                    // Check P10a: an `HGLOBAL` may be larger than the file (`GlobalSize` can round up); the descriptor's
                    // size, when given, is the file's.
                    if let Some(size) = d.size.and_then(|s| usize::try_from(s).ok()) {
                        bytes.truncate(size);
                    }
                    return (vec![DropItem::Data { name: d.name, kind: DataKind::File, bytes }], 0);
                }
                Read::TooLarge => return (Vec::new(), 1),
                Read::Missing => {}
            }
        }
        (Vec::new(), 0)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use super::super::data_object::{Counters, DataObject, Entry};
    use super::*;

    type Events = Arc<Mutex<Vec<OleEvent>>>;

    fn target(caps: Caps) -> (*mut c_void, Events) {
        let events: Events = Arc::default();
        let sink_events = Arc::clone(&events);
        let sink: Sink = Box::new(move |e| sink_events.lock().unwrap().push(e));
        (DropTarget::create(0, Formats::get(), caps, sink), events)
    }

    fn tv(t: *mut c_void) -> &'static IDropTargetVtbl {
        // SAFETY: a `DropTarget` made by `create`.
        unsafe { &*(*(t as *mut ComObject<IDropTargetVtbl>)).vtbl }
    }

    fn refs(obj: *mut c_void) -> u32 {
        // SAFETY: a live object of ours; AddRef then Release leaves the count unchanged and returns it + 1.
        unsafe {
            add_ref(obj);
            release(obj)
        }
    }

    /// DragEnter, DragOver, Drop at (30, 40) with `entries`; returns the effect of the drop and the events.
    fn drag_and_drop(entries: Vec<Entry>, caps: Caps) -> (u32, Vec<OleEvent>, Arc<Counters>) {
        let counters = Arc::new(Counters::default());
        let data = DataObject::create(entries, Some(Arc::clone(&counters)));
        let (t, events) = target(caps);
        let v = tv(t);
        let pt = PointL { x: 30, y: 40 };
        // SAFETY: live objects throughout; the effect pointers are valid.
        let effect = unsafe {
            let mut e = DROPEFFECT_COPY;
            assert_eq!((v.drag_enter)(t, data, 0, pt, &mut e), S_OK);
            let mut e = DROPEFFECT_COPY;
            (v.drag_over)(t, 0, PointL { x: 31, y: 41 }, &mut e);
            let mut e = DROPEFFECT_COPY;
            (v.drop)(t, data, 0, pt, &mut e);
            // The source lets go of its object: it is freed and its memory poisoned.
            assert_eq!(release(data), 0);
            assert_eq!(release(t), 0);
            e
        };
        let events = std::mem::take(&mut *events.lock().unwrap());
        (effect, events, counters)
    }

    fn dropped(events: &[OleEvent]) -> (&[DropItem], usize, [f32; 2]) {
        match events.last() {
            Some(OleEvent::Drop { items, skipped, pos }) => (items, *skipped, *pos),
            other => panic!("no drop event: {other:?}"),
        }
    }

    #[test]
    fn each_format_is_copied_before_drop_returns_and_every_medium_is_released() {
        let f = Formats::get();
        let png = b"\x89PNG\r\n\x1a\nfake png bytes".to_vec();
        let files = vec![PathBuf::from(r"C:\Pictures\a b.png"), PathBuf::from(r"C:\Bilder\Größe.jpg")];
        let dib = formats::pixels_to_dibv5(&wte_core::PixelBuf::new(2, 2));
        let group = formats::build_file_group(&[
            formats::FileDescriptor { name: "readme.txt".to_owned(), size: None },
            formats::FileDescriptor { name: "photo.JPG".to_owned(), size: Some(9) },
        ]);
        let other_wte = formats::wrap_sticker(std::process::id().wrapping_add(1), b"PK wtes");
        let cases: Vec<(Vec<Entry>, DropItem)> = vec![
            (vec![Entry::global(CF_HDROP, formats::build_dropfiles(&files))], DropItem::File(files[0].clone())),
            (vec![Entry::global(f.png, png.clone()), Entry::global(CF_DIBV5, dib.clone())], DropItem::Data {
                name: "Dropped picture.png".to_owned(),
                kind: DataKind::Png,
                bytes: png.clone(),
            }),
            (vec![Entry::global(CF_DIBV5, dib.clone())], DropItem::Data { name: "Dropped picture.png".to_owned(), kind: DataKind::Dib, bytes: dib.clone() }),
            (vec![Entry::global(CF_DIB, dib.clone())], DropItem::Data { name: "Dropped picture.png".to_owned(), kind: DataKind::Dib, bytes: dib.clone() }),
            (
                vec![Entry::global(f.file_group, group), Entry::stream(f.file_contents, 1, b"jpeg data".to_vec())],
                DropItem::Data { name: "photo.JPG".to_owned(), kind: DataKind::File, bytes: b"jpeg data".to_vec() },
            ),
            (
                vec![Entry::global(f.sticker, other_wte), Entry::global(CF_HDROP, formats::build_dropfiles(&files))],
                DropItem::Data { name: "Sticker.wtes".to_owned(), kind: DataKind::Sticker, bytes: b"PK wtes".to_vec() },
            ),
        ];
        for (case, (entries, first)) in cases.into_iter().enumerate() {
            let (effect, events, counters) = drag_and_drop(entries, CAPS);
            assert_eq!(effect, DROPEFFECT_COPY, "case {case}: {events:?}");
            assert!(matches!(events[0], OleEvent::Hover { pos: [30.0, 40.0], .. }), "{:?}", events[0]);
            assert!(matches!(events[1], OleEvent::Hover { pos: [31.0, 41.0], .. }));
            let (items, skipped, pos) = dropped(&events);
            assert_eq!((items.first(), skipped, pos), (Some(&first), 0, [30.0, 40.0]));
            // The payload is right although the object poisoned everything it held when it was released.
            let handed = counters.handed.load(Ordering::SeqCst);
            assert!(handed >= 1);
            assert_eq!(counters.released.load(Ordering::SeqCst), handed, "ReleaseStgMedium on every path");
        }
    }

    /// Check P10a: a virtual file handed out as an `HGLOBAL` larger than the file keeps only the descriptor's size.
    #[test]
    fn check_p10a_a_virtual_file_in_a_larger_hglobal_is_cut_to_its_size() {
        let f = Formats::get();
        let group = formats::build_file_group(&[formats::FileDescriptor { name: "pic.png".to_owned(), size: Some(5) }]);
        let contents = Entry { lindex: 0, ..Entry::global(f.file_contents, b"12345\0\0\0".to_vec()) };
        let (_, events, counters) = drag_and_drop(vec![Entry::global(f.file_group, group), contents], CAPS);
        let (items, skipped, _) = dropped(&events);
        assert_eq!(items, [DropItem::Data { name: "pic.png".to_owned(), kind: DataKind::File, bytes: b"12345".to_vec() }]);
        assert_eq!(skipped, 0);
        assert_eq!(counters.released.load(Ordering::SeqCst), counters.handed.load(Ordering::SeqCst));
    }

    #[test]
    fn hover_kinds_and_the_wte_origin_marker() {
        let f = Formats::get();
        let files = vec![PathBuf::from(r"C:\m\x.WTEP"), PathBuf::from(r"C:\m\y.png")];
        let (_, events, _) = drag_and_drop(vec![Entry::global(CF_HDROP, formats::build_dropfiles(&files))], CAPS);
        match &events[0] {
            OleEvent::Hover { kinds, .. } => assert_eq!(kinds.extensions, ["wtep", "png"]),
            other => panic!("{other:?}"),
        }
        // A drag that started in this WTE: nothing is queued, the effect is none, even with CF_HDROP beside it.
        let own = formats::wrap_sticker(std::process::id(), b"PK");
        let (effect, events, counters) =
            drag_and_drop(vec![Entry::global(f.sticker, own), Entry::global(CF_HDROP, formats::build_dropfiles(&files))], CAPS);
        assert_eq!(effect, DROPEFFECT_NONE);
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(counters.released.load(Ordering::SeqCst), counters.handed.load(Ordering::SeqCst));
        // Nothing WTE takes (plain text only): refused.
        let (effect, events, _) = drag_and_drop(vec![Entry::global(formats::CF_UNICODETEXT, vec![65, 0, 0, 0])], CAPS);
        assert_eq!((effect, events.len()), (DROPEFFECT_NONE, 0));
        // A virtual file group without a picture in it: refused.
        let group = formats::build_file_group(&[formats::FileDescriptor { name: "a.txt".to_owned(), size: None }]);
        let (effect, _, _) = drag_and_drop(vec![Entry::global(f.file_group, group), Entry::stream(f.file_contents, 0, vec![1])], CAPS);
        assert_eq!(effect, DROPEFFECT_NONE);
    }

    #[test]
    fn caps_skip_large_items_and_stop_endless_streams() {
        let f = Formats::get();
        // A 65-MiB item over the 64-MiB cap is not taken.
        let (_, events, counters) = drag_and_drop(vec![Entry::global(f.png, vec![7u8; 65 << 20])], CAPS);
        let (items, skipped, _) = dropped(&events);
        assert!(items.is_empty());
        assert_eq!(skipped, 1);
        assert_eq!(counters.released.load(Ordering::SeqCst), counters.handed.load(Ordering::SeqCst));
        // An endless IStream stops at the cap.
        let group = formats::build_file_group(&[formats::FileDescriptor { name: "big.png".to_owned(), size: None }]);
        let mut endless = Entry::stream(f.file_contents, 0, Vec::new());
        endless.endless = true;
        let (_, events, counters) = drag_and_drop(vec![Entry::global(f.file_group, group), endless], CAPS);
        let (items, skipped, _) = dropped(&events);
        assert!(items.is_empty());
        assert_eq!(skipped, 1);
        assert_eq!(counters.released.load(Ordering::SeqCst), counters.handed.load(Ordering::SeqCst));
        // More than 64 files: 64 taken, the rest counted.
        let files: Vec<PathBuf> = (0..70).map(|i| PathBuf::from(format!(r"C:\p\{i}.png"))).collect();
        let (_, events, _) = drag_and_drop(vec![Entry::global(CF_HDROP, formats::build_dropfiles(&files))], CAPS);
        let (items, skipped, _) = dropped(&events);
        assert_eq!((items.len(), skipped), (64, 6));
    }

    #[test]
    fn leave_query_interface_reference_counts_and_a_caught_panic() {
        let (t, events) = target(CAPS);
        let v = tv(t);
        let data = DataObject::create(vec![Entry::global(Formats::get().png, vec![1, 2, 3])], None);
        assert_eq!(refs(t), 1);
        assert_eq!(refs(data), 1);
        // SAFETY: live objects; valid pointers.
        unsafe {
            let mut out: *mut c_void = std::ptr::null_mut();
            assert_eq!((v.base.query_interface)(t, &IID_IDROPTARGET, &mut out), S_OK);
            assert_eq!(out, t);
            assert_eq!(refs(t), 2);
            release(t);
            assert_eq!((v.base.query_interface)(t, &IID_IUNKNOWN, &mut out), S_OK);
            release(t);
            assert_eq!((v.base.query_interface)(t, &IID_IDATAOBJECT, &mut out), E_NOINTERFACE);
            assert!(out.is_null());
            let mut e = DROPEFFECT_COPY;
            (v.drag_enter)(t, data, 0, PointL::default(), &mut e);
            assert_eq!(e, DROPEFFECT_COPY);
            assert_eq!((v.drag_leave)(t), S_OK);
            // A source that allows only a move gets no effect.
            let mut e = 2;
            (v.drag_enter)(t, data, 0, PointL::default(), &mut e);
            assert_eq!(e, DROPEFFECT_NONE);
            (v.drag_leave)(t);
            // A panic inside a callback is caught: E_UNEXPECTED, effect none, nothing crosses FFI.
            (*(t as *const DropTarget)).panic_next.store(true, Ordering::SeqCst);
            let mut e = DROPEFFECT_COPY;
            assert_eq!((v.drag_enter)(t, data, 0, PointL::default(), &mut e), E_UNEXPECTED);
            assert_eq!(e, DROPEFFECT_NONE);
            // A drop without an accepted enter queues nothing.
            let mut e = DROPEFFECT_COPY;
            (v.drop)(t, data, 0, PointL::default(), &mut e);
            assert_eq!(e, DROPEFFECT_NONE);
            // Reference counts are back at their start.
            assert_eq!(refs(t), 1);
            assert_eq!(refs(data), 1);
            release(data);
            release(t);
        }
        let events = events.lock().unwrap();
        assert!(matches!(events[..], [OleEvent::Hover { .. }, OleEvent::Leave, OleEvent::Hover { .. }, OleEvent::Leave]), "{events:?}");
    }
}
