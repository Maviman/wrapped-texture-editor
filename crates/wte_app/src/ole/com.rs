//! Stickers P10a (STICKER_PLAN 6.1): the Windows declarations of the OLE code, hand-written like `pen/win.rs` (no new
//! crate): `raw-dylib` imports from `ole32`, `shell32`, `user32` and `kernel32`, the `#[repr(C)]` structs and vtables
//! of `IUnknown`, `IDataObject`, `IDropTarget`, `IDropSource` (stickers P11), `IEnumFORMATETC` and `IStream` (read only)
//! as in the Windows SDK headers (`objidl.h`, `oleidl.h`), and the interface ids. The layout tests assert the SDK's sizes
//! and offsets for x64.
//!
//! Who frees what (the Win32 contracts the callers follow):
//! - `IDataObject::GetData` fills a `STGMEDIUM` the caller owns; the caller ends it with `ReleaseStgMedium`, which
//!   `GlobalFree`s an `HGLOBAL` (or calls `pUnkForRelease->Release()` instead when that is set) and `Release`s an
//!   `IStream`.
//! - An `HGLOBAL` is read between `GlobalLock` and `GlobalUnlock`; `GlobalSize` may be larger than the data.
//! - A COM object starts with one reference; `AddRef` / `Release` count it; `QueryInterface` adds one on success.

#![allow(non_snake_case, reason = "Win32 names")]
#![allow(dead_code, reason = "the declarations follow the SDK headers whole; the clipboard (P10b) and drag-out (P11) use the rest")]

use std::ffi::c_void;

pub type Hresult = i32;
pub type Hwnd = isize;
pub type Hglobal = isize;

pub const S_OK: Hresult = 0;
pub const S_FALSE: Hresult = 1;
pub const E_NOTIMPL: Hresult = 0x8000_4001_u32 as i32;
pub const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
pub const E_POINTER: Hresult = 0x8000_4003_u32 as i32;
pub const E_UNEXPECTED: Hresult = 0x8000_FFFF_u32 as i32;
pub const E_OUTOFMEMORY: Hresult = 0x8007_000E_u32 as i32;
pub const DV_E_FORMATETC: Hresult = 0x8004_0064_u32 as i32;
pub const DV_E_LINDEX: Hresult = 0x8004_0068_u32 as i32;
pub const DV_E_TYMED: Hresult = 0x8004_0069_u32 as i32;
pub const DV_E_DVASPECT: Hresult = 0x8004_006B_u32 as i32;
pub const OLE_E_ADVISENOTSUPPORTED: Hresult = 0x8004_0003_u32 as i32;

pub const DVASPECT_CONTENT: u32 = 1;
pub const TYMED_HGLOBAL: u32 = 1;
pub const TYMED_ISTREAM: u32 = 4;
pub const DATADIR_GET: u32 = 1;
pub const DROPEFFECT_NONE: u32 = 0;
pub const DROPEFFECT_COPY: u32 = 1;
pub const GMEM_MOVEABLE: u32 = 0x0002;
/// Stickers P11: `IDropSource::QueryContinueDrag` / `GiveFeedback` answers and `DoDragDrop` results (`oleidl.h`).
pub const DRAGDROP_S_DROP: Hresult = 0x0004_0100;
pub const DRAGDROP_S_CANCEL: Hresult = 0x0004_0101;
pub const DRAGDROP_S_USEDEFAULTCURSORS: Hresult = 0x0004_0102;
/// `grfKeyState` bits of `QueryContinueDrag` (`winuser.h` `MK_*`).
pub const MK_LBUTTON: u32 = 0x0001;

/// `GUID`.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

const fn com_iid(d1: u32) -> Guid {
    Guid { d1, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] }
}

pub const IID_IUNKNOWN: Guid = com_iid(0x0000_0000);
pub const IID_ISTREAM: Guid = com_iid(0x0000_000C);
pub const IID_IENUMFORMATETC: Guid = com_iid(0x0000_0103);
pub const IID_IDATAOBJECT: Guid = com_iid(0x0000_010E);
pub const IID_IDROPTARGET: Guid = com_iid(0x0000_0122);
pub const IID_IDROPSOURCE: Guid = com_iid(0x0000_0121);

/// `FORMATETC`.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct FormatEtc {
    pub cf_format: u16,
    pub ptd: *mut c_void,
    pub dw_aspect: u32,
    pub lindex: i32,
    pub tymed: u32,
}

impl FormatEtc {
    /// Content aspect, every index (`-1`), the given media.
    pub fn new(cf_format: u16, tymed: u32) -> FormatEtc {
        FormatEtc { cf_format, ptd: std::ptr::null_mut(), dw_aspect: DVASPECT_CONTENT, lindex: -1, tymed }
    }
}

/// `STGMEDIUM`: `data` is the union (`hGlobal` or `pstm`).
#[repr(C)]
#[derive(Debug)]
pub struct StgMedium {
    pub tymed: u32,
    pub data: *mut c_void,
    pub unk_for_release: *mut c_void,
}

impl Default for StgMedium {
    fn default() -> StgMedium {
        StgMedium { tymed: 0, data: std::ptr::null_mut(), unk_for_release: std::ptr::null_mut() }
    }
}

/// `POINTL` (passed by value to the `IDropTarget` methods).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PointL {
    pub x: i32,
    pub y: i32,
}

pub type QueryInterfaceFn = unsafe extern "system" fn(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult;
pub type RefFn = unsafe extern "system" fn(this: *mut c_void) -> u32;

/// `IUnknownVtbl`.
#[repr(C)]
pub struct IUnknownVtbl {
    pub query_interface: QueryInterfaceFn,
    pub add_ref: RefFn,
    pub release: RefFn,
}

/// `IDataObjectVtbl`.
#[repr(C)]
pub struct IDataObjectVtbl {
    pub base: IUnknownVtbl,
    pub get_data: unsafe extern "system" fn(this: *mut c_void, format: *const FormatEtc, medium: *mut StgMedium) -> Hresult,
    pub get_data_here: unsafe extern "system" fn(this: *mut c_void, format: *const FormatEtc, medium: *mut StgMedium) -> Hresult,
    pub query_get_data: unsafe extern "system" fn(this: *mut c_void, format: *const FormatEtc) -> Hresult,
    pub get_canonical_format_etc: unsafe extern "system" fn(this: *mut c_void, inp: *const FormatEtc, out: *mut FormatEtc) -> Hresult,
    pub set_data:
        unsafe extern "system" fn(this: *mut c_void, format: *const FormatEtc, medium: *const StgMedium, release: i32) -> Hresult,
    pub enum_format_etc: unsafe extern "system" fn(this: *mut c_void, direction: u32, out: *mut *mut c_void) -> Hresult,
    pub d_advise: unsafe extern "system" fn(
        this: *mut c_void,
        format: *const FormatEtc,
        advf: u32,
        sink: *mut c_void,
        connection: *mut u32,
    ) -> Hresult,
    pub d_unadvise: unsafe extern "system" fn(this: *mut c_void, connection: u32) -> Hresult,
    pub enum_d_advise: unsafe extern "system" fn(this: *mut c_void, out: *mut *mut c_void) -> Hresult,
}

/// `IDropTargetVtbl`.
#[repr(C)]
pub struct IDropTargetVtbl {
    pub base: IUnknownVtbl,
    pub drag_enter:
        unsafe extern "system" fn(this: *mut c_void, data: *mut c_void, keys: u32, pt: PointL, effect: *mut u32) -> Hresult,
    pub drag_over: unsafe extern "system" fn(this: *mut c_void, keys: u32, pt: PointL, effect: *mut u32) -> Hresult,
    pub drag_leave: unsafe extern "system" fn(this: *mut c_void) -> Hresult,
    pub drop: unsafe extern "system" fn(this: *mut c_void, data: *mut c_void, keys: u32, pt: PointL, effect: *mut u32) -> Hresult,
}

/// `IDropSourceVtbl` (stickers P11).
#[repr(C)]
pub struct IDropSourceVtbl {
    pub base: IUnknownVtbl,
    pub query_continue_drag: unsafe extern "system" fn(this: *mut c_void, escape_pressed: i32, keys: u32) -> Hresult,
    pub give_feedback: unsafe extern "system" fn(this: *mut c_void, effect: u32) -> Hresult,
}

/// `IEnumFORMATETCVtbl`.
#[repr(C)]
pub struct IEnumFormatEtcVtbl {
    pub base: IUnknownVtbl,
    pub next: unsafe extern "system" fn(this: *mut c_void, count: u32, out: *mut FormatEtc, fetched: *mut u32) -> Hresult,
    pub skip: unsafe extern "system" fn(this: *mut c_void, count: u32) -> Hresult,
    pub reset: unsafe extern "system" fn(this: *mut c_void) -> Hresult,
    pub clone: unsafe extern "system" fn(this: *mut c_void, out: *mut *mut c_void) -> Hresult,
}

/// `IStreamVtbl` (`ISequentialStream` + `IStream`); WTE calls only `Read`, the other slots keep the layout.
#[repr(C)]
pub struct IStreamVtbl {
    pub base: IUnknownVtbl,
    pub read: unsafe extern "system" fn(this: *mut c_void, buf: *mut c_void, len: u32, read: *mut u32) -> Hresult,
    pub write: unsafe extern "system" fn(this: *mut c_void, buf: *const c_void, len: u32, written: *mut u32) -> Hresult,
    pub seek: unsafe extern "system" fn(this: *mut c_void, mv: i64, origin: u32, pos: *mut u64) -> Hresult,
    pub set_size: unsafe extern "system" fn(this: *mut c_void, size: u64) -> Hresult,
    pub copy_to: unsafe extern "system" fn(this: *mut c_void, to: *mut c_void, len: u64, read: *mut u64, written: *mut u64) -> Hresult,
    pub commit: unsafe extern "system" fn(this: *mut c_void, flags: u32) -> Hresult,
    pub revert: unsafe extern "system" fn(this: *mut c_void) -> Hresult,
    pub lock_region: unsafe extern "system" fn(this: *mut c_void, at: u64, len: u64, kind: u32) -> Hresult,
    pub unlock_region: unsafe extern "system" fn(this: *mut c_void, at: u64, len: u64, kind: u32) -> Hresult,
    pub stat: unsafe extern "system" fn(this: *mut c_void, stat: *mut c_void, flags: u32) -> Hresult,
    pub clone: unsafe extern "system" fn(this: *mut c_void, out: *mut *mut c_void) -> Hresult,
}

/// Any COM object: a pointer to its vtable first.
#[repr(C)]
pub struct ComObject<V> {
    pub vtbl: *const V,
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn RegisterDragDrop(hwnd: Hwnd, target: *mut c_void) -> Hresult;
    pub fn RevokeDragDrop(hwnd: Hwnd) -> Hresult;
    pub fn ReleaseStgMedium(medium: *mut StgMedium);
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn SHCreateStdEnumFmtEtc(count: u32, formats: *const FormatEtc, out: *mut *mut c_void) -> Hresult;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn RegisterClipboardFormatW(name: *const u16) -> u32;
    pub fn ScreenToClient(hwnd: Hwnd, point: *mut PointL) -> i32;
    /// Stickers P11: the drag message (`WTE.OleDrag.v1`) and its post to the app window.
    pub fn RegisterWindowMessageW(name: *const u16) -> u32;
    pub fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GlobalAlloc(flags: u32, bytes: usize) -> Hglobal;
    pub fn GlobalLock(mem: Hglobal) -> *mut c_void;
    pub fn GlobalUnlock(mem: Hglobal) -> i32;
    pub fn GlobalSize(mem: Hglobal) -> usize;
    pub fn GlobalFree(mem: Hglobal) -> Hglobal;
}

/// Calls `IUnknown::Release` of a COM object.
///
/// # Safety
/// `obj` is a live COM object that holds a reference the caller owns.
pub unsafe fn release(obj: *mut c_void) -> u32 {
    // SAFETY: every COM object starts with a vtable whose first three slots are IUnknown's.
    unsafe { ((*(*(obj as *mut ComObject<IUnknownVtbl>)).vtbl).release)(obj) }
}

/// Calls `IUnknown::AddRef` of a COM object.
///
/// # Safety
/// `obj` is a live COM object.
pub unsafe fn add_ref(obj: *mut c_void) -> u32 {
    // SAFETY: as `release`.
    unsafe { ((*(*(obj as *mut ComObject<IUnknownVtbl>)).vtbl).add_ref)(obj) }
}

/// Copies `bytes` into a new movable `HGLOBAL` (0 when out of memory).
pub fn global_from(bytes: &[u8]) -> Hglobal {
    // SAFETY: plain kernel32 calls; the block is at least `bytes.len()` long (1 for an empty block) and locked while
    // written.
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if mem == 0 {
            return 0;
        }
        let p = GlobalLock(mem);
        if p.is_null() {
            GlobalFree(mem);
            return 0;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
        GlobalUnlock(mem);
        mem
    }
}

/// The NUL-terminated UTF-16 of a format name, registered (the same id for the same name in every process).
pub fn register_format(name: &str) -> u16 {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a NUL-terminated name that lives across the call.
    let id = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    u16::try_from(id).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::mem::{offset_of, size_of};

    use super::*;

    /// The x64 layouts of the Windows SDK (`objidl.h`, `oleidl.h`, `windef.h`).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn struct_and_vtable_layouts() {
        assert_eq!(size_of::<Guid>(), 16);
        assert_eq!(size_of::<FormatEtc>(), 32);
        assert_eq!(offset_of!(FormatEtc, ptd), 8);
        assert_eq!(offset_of!(FormatEtc, dw_aspect), 16);
        assert_eq!(offset_of!(FormatEtc, lindex), 20);
        assert_eq!(offset_of!(FormatEtc, tymed), 24);
        assert_eq!(size_of::<StgMedium>(), 24);
        assert_eq!(offset_of!(StgMedium, data), 8);
        assert_eq!(offset_of!(StgMedium, unk_for_release), 16);
        assert_eq!(size_of::<PointL>(), 8);
        // Slots: IUnknown 3; IDataObject 3 + 9; IDropTarget 3 + 4; IEnumFORMATETC 3 + 4; IStream 3 + 2 + 9.
        assert_eq!(size_of::<IUnknownVtbl>(), 3 * 8);
        assert_eq!(size_of::<IDataObjectVtbl>(), 12 * 8);
        assert_eq!(offset_of!(IDataObjectVtbl, get_data), 3 * 8);
        assert_eq!(offset_of!(IDataObjectVtbl, query_get_data), 5 * 8);
        assert_eq!(offset_of!(IDataObjectVtbl, enum_format_etc), 8 * 8);
        assert_eq!(size_of::<IDropTargetVtbl>(), 7 * 8);
        assert_eq!(offset_of!(IDropTargetVtbl, drop), 6 * 8);
        assert_eq!(size_of::<IEnumFormatEtcVtbl>(), 7 * 8);
        // IDropSource 3 + 2 (stickers P11).
        assert_eq!(size_of::<IDropSourceVtbl>(), 5 * 8);
        assert_eq!(offset_of!(IDropSourceVtbl, query_continue_drag), 3 * 8);
        assert_eq!(offset_of!(IDropSourceVtbl, give_feedback), 4 * 8);
        assert_eq!(size_of::<IStreamVtbl>(), 14 * 8);
        assert_eq!(offset_of!(IStreamVtbl, read), 3 * 8);
        assert_eq!(offset_of!(IStreamVtbl, clone), 13 * 8);
    }

    #[test]
    fn interface_ids() {
        // {0000010E-0000-0000-C000-000000000046} and {00000122-0000-0000-C000-000000000046}.
        assert_eq!(IID_IDATAOBJECT.d1, 0x10E);
        assert_eq!(IID_IDROPTARGET.d1, 0x122);
        // {00000121-0000-0000-C000-000000000046}.
        assert_eq!(IID_IDROPSOURCE.d1, 0x121);
        assert_eq!(IID_IUNKNOWN.d4, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    }

    #[test]
    fn registered_formats_are_stable() {
        let a = register_format(crate::ole::formats::NAME_STICKER);
        assert!(a >= 0xC000, "registered formats are 0xC000..=0xFFFF: {a:#x}");
        assert_eq!(register_format(crate::ole::formats::NAME_STICKER), a);
        assert_ne!(register_format(crate::ole::formats::NAME_PNG), a);
    }
}
