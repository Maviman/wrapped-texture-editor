//! Stickers P10a (STICKER_PLAN 6.1): WTE's own `IDataObject`, a fixed list of `(format, bytes)` entries. Drag-out
//! (P11) hands it to `DoDragDrop`; the tests and the self-test runner's `Step::Drop` hand it to WTE's drop target as the
//! "other program". Also a read-only `IStream` over bytes (`MemStream`, optionally endless for the cap test) and, for
//! the tests, a `pUnkForRelease` owner that poisons and frees an `HGLOBAL` when the medium is released (so a drop
//! target that read the memory after `ReleaseStgMedium` would see 0xDD bytes) and counts the releases.
//!
//! Every method body runs inside `catch_unwind` (a panic must not cross FFI); `QueryInterface` answers `IUnknown` and
//! the object's own interface, `E_NOINTERFACE` otherwise. The objects are boxes with an atomic reference count,
//! freed when it reaches zero.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::com::*;

/// One entry: `format` with its bytes, offered on the media in `tymed` (`TYMED_HGLOBAL`, `TYMED_ISTREAM` or both;
/// `GetData` prefers `HGLOBAL`). `lindex` -1 answers every index, else only that one (`FileContents` of a group).
#[derive(Clone, Debug)]
pub struct Entry {
    pub format: u16,
    pub lindex: i32,
    pub tymed: u32,
    pub bytes: Arc<Vec<u8>>,
    /// Tests: an `IStream` that never ends (the cap test).
    pub endless: bool,
}

impl Entry {
    /// An `HGLOBAL` entry for every index.
    pub fn global(format: u16, bytes: Vec<u8>) -> Entry {
        Entry { format, lindex: -1, tymed: TYMED_HGLOBAL, bytes: Arc::new(bytes), endless: false }
    }

    /// An `IStream` entry for one index.
    pub fn stream(format: u16, lindex: i32, bytes: Vec<u8>) -> Entry {
        Entry { format, lindex, tymed: TYMED_ISTREAM, bytes: Arc::new(bytes), endless: false }
    }
}

/// Counters of a `DataObject` the tests read: media handed out by `GetData` and media released (through
/// `ReleaseStgMedium` -> `pUnkForRelease` / `IStream::Release`).
#[derive(Debug, Default)]
pub struct Counters {
    pub handed: AtomicUsize,
    pub released: AtomicUsize,
}

/// WTE's `IDataObject` (`#[repr(C)]`: the vtable pointer first).
#[repr(C)]
pub struct DataObject {
    vtbl: *const IDataObjectVtbl,
    refs: AtomicU32,
    entries: Vec<Entry>,
    /// Tests: `HGLOBAL`s go out with a `pUnkForRelease` owner that poisons and frees them, and both media count.
    tracked: Option<Arc<Counters>>,
}

static DATA_OBJECT_VTBL: IDataObjectVtbl = IDataObjectVtbl {
    base: IUnknownVtbl { query_interface: data_qi, add_ref: data_add_ref, release: data_release },
    get_data: data_get_data,
    get_data_here: data_get_data_here,
    query_get_data: data_query_get_data,
    get_canonical_format_etc: data_get_canonical,
    set_data: data_set_data,
    enum_format_etc: data_enum_format_etc,
    d_advise: data_d_advise,
    d_unadvise: data_d_unadvise,
    enum_d_advise: data_enum_d_advise,
};

impl DataObject {
    /// A new object with one reference (the caller's), as a COM pointer.
    pub fn create(entries: Vec<Entry>, tracked: Option<Arc<Counters>>) -> *mut c_void {
        let obj = Box::new(DataObject { vtbl: &DATA_OBJECT_VTBL, refs: AtomicU32::new(1), entries, tracked });
        Box::into_raw(obj).cast()
    }

    fn find(&self, f: &FormatEtc) -> Result<&Entry, Hresult> {
        if f.dw_aspect != DVASPECT_CONTENT {
            return Err(DV_E_DVASPECT);
        }
        let mut format_seen = false;
        for e in &self.entries {
            if e.format != f.cf_format {
                continue;
            }
            format_seen = true;
            // -1 on either side means every index (a `QueryGetData` of `FileContents` often asks with -1).
            if e.lindex != -1 && f.lindex != -1 && e.lindex != f.lindex {
                continue;
            }
            if e.tymed & f.tymed == 0 {
                return Err(DV_E_TYMED);
            }
            return Ok(e);
        }
        Err(if format_seen { DV_E_LINDEX } else { DV_E_FORMATETC })
    }
}

/// Runs a method body; a panic becomes `E_UNEXPECTED`.
fn guarded(f: impl FnOnce() -> Hresult) -> Hresult {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(E_UNEXPECTED)
}

/// Shared `QueryInterface`: `IUnknown` and `own` give `this` with one more reference.
///
/// # Safety
/// `this` is a live object with an `AtomicU32` count right after its vtable pointer; `iid` and `out` are valid.
unsafe fn query(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void, own: &Guid) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers are valid (checked above for null).
    unsafe {
        let iid = *iid;
        if iid == IID_IUNKNOWN || iid == *own {
            add_ref(this);
            *out = this;
            S_OK
        } else {
            *out = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn data_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: `this` is a `DataObject` (this vtable is only installed on those).
    guarded(|| unsafe { query(this, iid, out, &IID_IDATAOBJECT) })
}

unsafe extern "system" fn data_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `DataObject`.
    unsafe { (*(this as *const DataObject)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn data_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `DataObject`; the last reference frees the box made by `create`.
    unsafe {
        let left = (*(this as *const DataObject)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            let mut obj = Box::from_raw(this as *mut DataObject);
            // Poison what the object held (a reader that kept a pointer into it would see 0xDD).
            for e in &mut obj.entries {
                if let Some(bytes) = Arc::get_mut(&mut e.bytes) {
                    bytes.fill(0xDD);
                }
            }
        }
        left
    }
}

unsafe extern "system" fn data_get_data(this: *mut c_void, format: *const FormatEtc, medium: *mut StgMedium) -> Hresult {
    guarded(|| {
        if format.is_null() || medium.is_null() {
            return E_POINTER;
        }
        // SAFETY: `this` is a live `DataObject`; `format` and `medium` are valid (checked for null).
        unsafe {
            let obj = &*(this as *const DataObject);
            let entry = match obj.find(&*format) {
                Ok(e) => e,
                Err(hr) => return hr,
            };
            let wanted = (*format).tymed & entry.tymed;
            let out = if wanted & TYMED_HGLOBAL != 0 {
                let mem = global_from(&entry.bytes);
                if mem == 0 {
                    return E_OUTOFMEMORY;
                }
                let owner = match &obj.tracked {
                    Some(c) => Owner::create(mem, Arc::clone(c)),
                    None => std::ptr::null_mut(),
                };
                StgMedium { tymed: TYMED_HGLOBAL, data: mem as *mut c_void, unk_for_release: owner }
            } else {
                let stream = MemStream::create(Arc::clone(&entry.bytes), entry.endless, obj.tracked.clone());
                StgMedium { tymed: TYMED_ISTREAM, data: stream, unk_for_release: std::ptr::null_mut() }
            };
            *medium = out;
            if let Some(c) = &obj.tracked {
                c.handed.fetch_add(1, Ordering::SeqCst);
            }
            S_OK
        }
    })
}

unsafe extern "system" fn data_get_data_here(_this: *mut c_void, _f: *const FormatEtc, _m: *mut StgMedium) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn data_query_get_data(this: *mut c_void, format: *const FormatEtc) -> Hresult {
    guarded(|| {
        if format.is_null() {
            return E_POINTER;
        }
        // SAFETY: `this` is a live `DataObject`; `format` is valid.
        unsafe { (*(this as *const DataObject)).find(&*format).map_or_else(|hr| hr, |_| S_OK) }
    })
}

unsafe extern "system" fn data_get_canonical(_this: *mut c_void, _i: *const FormatEtc, out: *mut FormatEtc) -> Hresult {
    if !out.is_null() {
        // SAFETY: a valid out pointer; "no canonical format" leaves `ptd` null.
        unsafe { (*out).ptd = std::ptr::null_mut() };
    }
    E_NOTIMPL
}

unsafe extern "system" fn data_set_data(_this: *mut c_void, _f: *const FormatEtc, _m: *const StgMedium, _r: i32) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn data_enum_format_etc(this: *mut c_void, direction: u32, out: *mut *mut c_void) -> Hresult {
    guarded(|| {
        if out.is_null() {
            return E_POINTER;
        }
        if direction != DATADIR_GET {
            // SAFETY: a valid out pointer.
            unsafe { *out = std::ptr::null_mut() };
            return E_NOTIMPL;
        }
        // SAFETY: `this` is a live `DataObject`; shell32 copies the array into its own enumerator.
        unsafe {
            let obj = &*(this as *const DataObject);
            let list: Vec<FormatEtc> = obj
                .entries
                .iter()
                .map(|e| FormatEtc { lindex: e.lindex, ..FormatEtc::new(e.format, e.tymed) })
                .collect();
            SHCreateStdEnumFmtEtc(list.len() as u32, list.as_ptr(), out)
        }
    })
}

unsafe extern "system" fn data_d_advise(_t: *mut c_void, _f: *const FormatEtc, _a: u32, _s: *mut c_void, _c: *mut u32) -> Hresult {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_d_unadvise(_this: *mut c_void, _connection: u32) -> Hresult {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_enum_d_advise(_this: *mut c_void, _out: *mut *mut c_void) -> Hresult {
    OLE_E_ADVISENOTSUPPORTED
}

// ---------------------------------------------------------------------------------------------------------------------
// A read-only IStream over bytes
// ---------------------------------------------------------------------------------------------------------------------

/// A read-only `IStream` over shared bytes (`endless`: every `Read` fills the whole buffer, forever).
#[repr(C)]
pub struct MemStream {
    vtbl: *const IStreamVtbl,
    refs: AtomicU32,
    bytes: Arc<Vec<u8>>,
    pos: AtomicUsize,
    endless: bool,
    tracked: Option<Arc<Counters>>,
}

static MEM_STREAM_VTBL: IStreamVtbl = IStreamVtbl {
    base: IUnknownVtbl { query_interface: stream_qi, add_ref: stream_add_ref, release: stream_release },
    read: stream_read,
    write: stream_write,
    seek: stream_seek,
    set_size: stream_set_size,
    copy_to: stream_copy_to,
    commit: stream_commit,
    revert: stream_revert,
    lock_region: stream_lock_region,
    unlock_region: stream_lock_region,
    stat: stream_stat,
    clone: stream_clone,
};

impl MemStream {
    /// A new stream with one reference, as a COM pointer.
    pub fn create(bytes: Arc<Vec<u8>>, endless: bool, tracked: Option<Arc<Counters>>) -> *mut c_void {
        let s = Box::new(MemStream {
            vtbl: &MEM_STREAM_VTBL,
            refs: AtomicU32::new(1),
            bytes,
            pos: AtomicUsize::new(0),
            endless,
            tracked,
        });
        Box::into_raw(s).cast()
    }
}

unsafe extern "system" fn stream_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: `this` is a `MemStream`.
    guarded(|| unsafe { query(this, iid, out, &IID_ISTREAM) })
}

unsafe extern "system" fn stream_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `MemStream`.
    unsafe { (*(this as *const MemStream)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn stream_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `MemStream`; the last reference frees the box.
    unsafe {
        let left = (*(this as *const MemStream)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            let s = Box::from_raw(this as *mut MemStream);
            if let Some(c) = &s.tracked {
                c.released.fetch_add(1, Ordering::SeqCst);
            }
        }
        left
    }
}

unsafe extern "system" fn stream_read(this: *mut c_void, buf: *mut c_void, len: u32, read: *mut u32) -> Hresult {
    guarded(|| {
        if buf.is_null() {
            return E_POINTER;
        }
        // SAFETY: `this` is a live `MemStream`; `buf` holds `len` writable bytes (the IStream contract).
        unsafe {
            let s = &*(this as *const MemStream);
            let n = if s.endless {
                std::ptr::write_bytes(buf.cast::<u8>(), 0x5A, len as usize);
                len as usize
            } else {
                let pos = s.pos.load(Ordering::Acquire).min(s.bytes.len());
                let n = (s.bytes.len() - pos).min(len as usize);
                std::ptr::copy_nonoverlapping(s.bytes[pos..].as_ptr(), buf.cast::<u8>(), n);
                s.pos.store(pos + n, Ordering::Release);
                n
            };
            if !read.is_null() {
                *read = n as u32;
            }
            if n < len as usize { S_FALSE } else { S_OK }
        }
    })
}

unsafe extern "system" fn stream_write(_t: *mut c_void, _b: *const c_void, _l: u32, _w: *mut u32) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_seek(_t: *mut c_void, _m: i64, _o: u32, _p: *mut u64) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_set_size(_t: *mut c_void, _s: u64) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_copy_to(_t: *mut c_void, _to: *mut c_void, _l: u64, _r: *mut u64, _w: *mut u64) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_commit(_t: *mut c_void, _f: u32) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_revert(_t: *mut c_void) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_lock_region(_t: *mut c_void, _a: u64, _l: u64, _k: u32) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_stat(_t: *mut c_void, _s: *mut c_void, _f: u32) -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn stream_clone(_t: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if !out.is_null() {
        // SAFETY: a valid out pointer.
        unsafe { *out = std::ptr::null_mut() };
    }
    E_NOTIMPL
}

// ---------------------------------------------------------------------------------------------------------------------
// The tests' pUnkForRelease: poisons and frees an HGLOBAL
// ---------------------------------------------------------------------------------------------------------------------

/// The `pUnkForRelease` of a tracked `HGLOBAL`: its last `Release` fills the block with 0xDD, frees it and counts.
#[repr(C)]
struct Owner {
    vtbl: *const IUnknownVtbl,
    refs: AtomicU32,
    mem: Hglobal,
    counters: Arc<Counters>,
}

static OWNER_VTBL: IUnknownVtbl = IUnknownVtbl { query_interface: owner_qi, add_ref: owner_add_ref, release: owner_release };

impl Owner {
    fn create(mem: Hglobal, counters: Arc<Counters>) -> *mut c_void {
        Box::into_raw(Box::new(Owner { vtbl: &OWNER_VTBL, refs: AtomicU32::new(1), mem, counters })).cast()
    }
}

unsafe extern "system" fn owner_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: `this` is an `Owner`.
    guarded(|| unsafe { query(this, iid, out, &IID_IUNKNOWN) })
}

unsafe extern "system" fn owner_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `Owner`.
    unsafe { (*(this as *const Owner)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn owner_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live `Owner`; the last reference poisons and frees the block it owns, then the box.
    unsafe {
        let left = (*(this as *const Owner)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            let o = Box::from_raw(this as *mut Owner);
            let p = GlobalLock(o.mem);
            if !p.is_null() {
                std::ptr::write_bytes(p.cast::<u8>(), 0xDD, GlobalSize(o.mem));
                GlobalUnlock(o.mem);
            }
            GlobalFree(o.mem);
            o.counters.released.fetch_add(1, Ordering::SeqCst);
        }
        left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vtbl(obj: *mut c_void) -> &'static IDataObjectVtbl {
        // SAFETY: a `DataObject` made by `create`.
        unsafe { &*(*(obj as *mut ComObject<IDataObjectVtbl>)).vtbl }
    }

    #[test]
    fn get_data_query_get_data_and_enum_format_etc() {
        let counters = Arc::new(Counters::default());
        let obj = DataObject::create(
            vec![Entry::global(15, b"hdrop".to_vec()), Entry::stream(0xC123, 0, b"stream bytes".to_vec())],
            Some(Arc::clone(&counters)),
        );
        let v = vtbl(obj);
        // SAFETY: valid object and arguments throughout.
        unsafe {
            assert_eq!((v.query_get_data)(obj, &FormatEtc::new(15, TYMED_HGLOBAL)), S_OK);
            assert_eq!((v.query_get_data)(obj, &FormatEtc::new(15, TYMED_ISTREAM)), DV_E_TYMED);
            assert_eq!((v.query_get_data)(obj, &FormatEtc::new(8, TYMED_HGLOBAL)), DV_E_FORMATETC);
            let wrong_index = FormatEtc { lindex: 3, ..FormatEtc::new(0xC123, TYMED_ISTREAM) };
            assert_eq!((v.query_get_data)(obj, &wrong_index), DV_E_LINDEX);
            let mut m = StgMedium::default();
            assert_eq!((v.get_data)(obj, &FormatEtc::new(15, TYMED_HGLOBAL | TYMED_ISTREAM), &mut m), S_OK);
            assert_eq!(m.tymed, TYMED_HGLOBAL);
            let p = GlobalLock(m.data as Hglobal);
            assert_eq!(std::slice::from_raw_parts(p.cast::<u8>(), 5), b"hdrop");
            GlobalUnlock(m.data as Hglobal);
            ReleaseStgMedium(&mut m);
            let mut m = StgMedium::default();
            let f = FormatEtc { lindex: 0, ..FormatEtc::new(0xC123, TYMED_ISTREAM) };
            assert_eq!((v.get_data)(obj, &f, &mut m), S_OK);
            assert_eq!(m.tymed, TYMED_ISTREAM);
            let stream = &*(*(m.data as *mut ComObject<IStreamVtbl>)).vtbl;
            let mut buf = [0u8; 64];
            let mut n = 0u32;
            assert_eq!((stream.read)(m.data, buf.as_mut_ptr().cast(), 64, &mut n), S_FALSE);
            assert_eq!(&buf[..n as usize], b"stream bytes");
            ReleaseStgMedium(&mut m);
            assert_eq!(counters.handed.load(Ordering::SeqCst), 2);
            assert_eq!(counters.released.load(Ordering::SeqCst), 2, "both media released");
            // The enumerator lists both formats.
            let mut e: *mut c_void = std::ptr::null_mut();
            assert_eq!((v.enum_format_etc)(obj, DATADIR_GET, &mut e), S_OK);
            let ev = &*(*(e as *mut ComObject<IEnumFormatEtcVtbl>)).vtbl;
            let mut got = [FormatEtc::new(0, 0); 4];
            let mut fetched = 0u32;
            (ev.next)(e, 4, got.as_mut_ptr(), &mut fetched);
            assert_eq!(fetched, 2);
            assert_eq!([got[0].cf_format, got[1].cf_format], [15, 0xC123]);
            release(e);
            // QueryInterface: IUnknown and IDataObject only.
            let mut out: *mut c_void = std::ptr::null_mut();
            assert_eq!((v.base.query_interface)(obj, &IID_IUNKNOWN, &mut out), S_OK);
            assert_eq!(out, obj);
            assert_eq!(release(obj), 1);
            assert_eq!((v.base.query_interface)(obj, &IID_IDROPTARGET, &mut out), E_NOINTERFACE);
            assert!(out.is_null());
            assert_eq!(release(obj), 0, "the last reference frees the object");
        }
    }
}
