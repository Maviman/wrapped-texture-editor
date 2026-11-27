//! Stickers P10a (STICKER_PLAN 6.1, 6.2; ARCHITECTURE 8.17): WTE's own OLE drag and drop on Windows, hand-written
//! `raw-dylib` FFI (no new crate). This part is the drop target: `install` swaps winit's `IDropTarget` (which reads
//! `CF_HDROP` only and loses the drop point) for WTE's (`drop_target.rs`), whose callbacks queue `OleEvent`s here; the
//! app drains them in `dnd::handle`. The clipboard (P10b) and drag out (P11) add to this module.
//!
//! Rules: the callbacks run on the UI thread inside winit's message dispatch, outside eframe's frame: they only push
//! small events (and `Drop` the copied data) to the queue and ask for a repaint. `install` runs in interactive runs; a
//! self-test run installs it only with `WTE_OLE=1` (to show it does no harm) and drops what it queues
//! (`discard`); the runner's `Step::Drop` feeds the same queue through the same drop target object afterwards.
//! Everywhere else (non-Windows, a failed registration, a self-test without `WTE_OLE`) egui's `hovered_files` /
//! `dropped_files` still work and `dnd.rs` reads both sources.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use eframe::egui;

pub mod formats;

#[cfg(windows)]
pub mod com;
#[cfg(windows)]
pub mod data_object;
#[cfg(windows)]
pub mod drop_target;

/// What a drag carries, for the overlay and the drop marker (`Hover`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragKinds {
    /// Lower-case extensions of the files (or of the virtual file) it carries.
    pub extensions: Vec<String>,
    /// Picture data without a file (`PNG`, `CF_DIBV5`, `CF_DIB`).
    pub picture: bool,
    /// A sticker of another WTE (`WTE.Sticker.v1`).
    pub sticker: bool,
}

/// What kind of bytes a `DropItem::Data` holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DataKind {
    /// An encoded picture (the registered `PNG` format).
    Png,
    /// A packed DIB (`CF_DIBV5` / `CF_DIB`).
    Dib,
    /// The `.wtes` bytes of a `WTE.Sticker.v1`.
    Sticker,
    /// A virtual file (`FileContents`): its name's extension says what it is.
    File,
}

/// One dropped thing: a file path, or bytes copied out of the data object.
#[derive(Clone, Debug, PartialEq)]
pub enum DropItem {
    File(PathBuf),
    Data { name: String, kind: DataKind, bytes: Vec<u8> },
}

/// What the drop target queues. Positions are client pixels (physical); `dnd.rs` divides by `pixels_per_point`.
#[derive(Clone, Debug, PartialEq)]
pub enum OleEvent {
    /// `DragEnter` / `DragOver` of a drag that carries something WTE takes.
    Hover { pos: [f32; 2], kinds: DragKinds },
    /// `DragLeave` (the drag left the window or was cancelled).
    Leave,
    /// `Drop`: the copied items and how many were not taken because of a cap (6.1).
    Drop { pos: [f32; 2], items: Vec<DropItem>, skipped: usize },
}

/// At most this many events wait in the queue (the oldest go first; a stream of `Hover`s is all that can pile up).
const QUEUE_CAP: usize = 256;

static QUEUE: Mutex<VecDeque<OleEvent>> = Mutex::new(VecDeque::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);

fn queue() -> std::sync::MutexGuard<'static, VecDeque<OleEvent>> {
    QUEUE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queues an event and asks for a frame (the callbacks and the self-test runner).
pub fn push(event: OleEvent) {
    {
        let mut q = queue();
        q.push_back(event);
        while q.len() > QUEUE_CAP {
            q.pop_front();
        }
    }
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// Takes the queued events.
pub fn drain() -> Vec<OleEvent> {
    queue().drain(..).collect()
}

/// True while events wait (the self-test's idle check).
pub fn pending() -> bool {
    !queue().is_empty()
}

/// Drops the queued events (a self-test run with `WTE_OLE=1`: a real drag must not reach a scenario).
pub fn discard() {
    queue().clear();
}

/// True when WTE's drop target is registered on the window.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Registers WTE's drop target on the app window in place of winit's (`RevokeDragDrop`, then `RegisterDragDrop`;
/// winit's `with_drag_and_drop(true)` has called `OleInitialize` on this thread). Called from `WteApp::new` in
/// interactive runs (a self-test run: only with `WTE_OLE=1`). A failure is logged and returned; when the revoke
/// succeeded but the registration failed, winit's target cannot be put back (its object is not reachable through
/// winit), so file drops do not work in that run (`dnd` toasts once). Returns true when installed.
pub fn install(cc: &eframe::CreationContext<'_>) -> bool {
    let _ = CTX.set(cc.egui_ctx.clone());
    #[cfg(windows)]
    {
        use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
        let hwnd = match cc.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            other => {
                log::warn!("ole: no Win32 window handle ({:?}); drops use egui's file list", other.map(|_| ()));
                return false;
            }
        };
        match register(hwnd) {
            Ok(line) => {
                log::info!("{line}");
                INSTALLED.store(true, Ordering::Release);
                true
            }
            Err(e) => {
                log::warn!("ole: {e}");
                REGISTRATION_LOST.store(e.contains("winit's"), Ordering::Release);
                false
            }
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Set when winit's target was revoked but WTE's could not be registered (file drops are off; `dnd` says so once).
static REGISTRATION_LOST: AtomicBool = AtomicBool::new(false);

/// True once when the drop registration was lost (the toast of `dnd`).
pub fn take_registration_lost() -> bool {
    REGISTRATION_LOST.swap(false, Ordering::AcqRel)
}

#[cfg(windows)]
fn register(hwnd: com::Hwnd) -> Result<String, String> {
    use drop_target::{CAPS, DropTarget, Formats};
    // The target lives for the whole process: its own reference is never given back (Windows may hold it).
    let target = DropTarget::create(hwnd, Formats::get(), CAPS, Box::new(push));
    // SAFETY: our own window on its (STA, OLE-initialised) thread; `target` is a live IDropTarget.
    unsafe {
        let revoked = com::RevokeDragDrop(hwnd);
        let hr = com::RegisterDragDrop(hwnd, target);
        if hr != com::S_OK {
            let lost = if revoked == com::S_OK { "; winit's drop target was revoked and cannot be put back" } else { "" };
            return Err(format!("RegisterDragDrop failed ({hr:#010x}){lost}"));
        }
        Ok(format!(
            "ole: WTE's drop target is registered (winit's revoked: {})",
            if revoked == com::S_OK { "yes" } else { "it was not registered" }
        ))
    }
}

/// The self-test runner's `Step::Drop` / `Step::DragHover` / `Step::DragLeave` (8.2): drives a drop target object (not
/// registered anywhere, its points taken as client pixels) with WTE's own data object made from `items`, so the
/// events reach the queue exactly as a real drop's would. Without Windows the events are queued directly.
pub struct TestDrag {
    #[cfg(windows)]
    target: *mut std::ffi::c_void,
    #[cfg(windows)]
    data: *mut std::ffi::c_void,
    #[cfg(not(windows))]
    items: Vec<DropItem>,
    #[cfg(not(windows))]
    kinds: DragKinds,
}

impl TestDrag {
    /// Starts a drag of `items` at `pos` (client pixels): `DragEnter`.
    pub fn enter(items: &[DropItem], pos: [f32; 2]) -> TestDrag {
        #[cfg(windows)]
        {
            use drop_target::{CAPS, DropTarget, Formats};
            let target = DropTarget::create(0, Formats::get(), CAPS, Box::new(push));
            let data = data_object::DataObject::create(entries_of(items), None);
            let drag = TestDrag { target, data };
            // SAFETY: live objects of ours; a valid effect pointer.
            unsafe {
                let mut effect = com::DROPEFFECT_COPY;
                (drag.vtbl().drag_enter)(target, data, 0, point(pos), &mut effect);
            }
            drag
        }
        #[cfg(not(windows))]
        {
            let kinds = kinds_of(items);
            push(OleEvent::Hover { pos, kinds: kinds.clone() });
            TestDrag { items: items.to_vec(), kinds }
        }
    }

    #[cfg(windows)]
    fn vtbl(&self) -> &com::IDropTargetVtbl {
        // SAFETY: `target` is a live `DropTarget` until `Drop for TestDrag`.
        unsafe { &*(*(self.target as *mut com::ComObject<com::IDropTargetVtbl>)).vtbl }
    }

    /// `DragOver` at `pos`.
    pub fn over(&self, pos: [f32; 2]) {
        #[cfg(windows)]
        // SAFETY: as `enter`.
        unsafe {
            let mut effect = com::DROPEFFECT_COPY;
            (self.vtbl().drag_over)(self.target, 0, point(pos), &mut effect);
        }
        #[cfg(not(windows))]
        {
            push(OleEvent::Hover { pos, kinds: self.kinds.clone() });
        }
    }

    /// `Drop` at `pos`.
    pub fn drop_at(self, pos: [f32; 2]) {
        #[cfg(windows)]
        // SAFETY: as `enter`.
        unsafe {
            let mut effect = com::DROPEFFECT_COPY;
            (self.vtbl().drop)(self.target, self.data, 0, point(pos), &mut effect);
        }
        #[cfg(not(windows))]
        {
            push(OleEvent::Drop { pos, items: self.items.clone(), skipped: 0 });
        }
    }

    /// `DragLeave`.
    pub fn leave(self) {
        #[cfg(windows)]
        // SAFETY: as `enter`.
        unsafe {
            (self.vtbl().drag_leave)(self.target);
        }
        #[cfg(not(windows))]
        {
            push(OleEvent::Leave);
        }
    }
}

#[cfg(windows)]
impl Drop for TestDrag {
    fn drop(&mut self) {
        // SAFETY: the references `enter` made.
        unsafe {
            com::release(self.data);
            com::release(self.target);
        }
    }
}

#[cfg(windows)]
fn point(pos: [f32; 2]) -> com::PointL {
    com::PointL { x: pos[0].round() as i32, y: pos[1].round() as i32 }
}

/// The data object entries that offer `items` as another program would: files as `CF_HDROP`, a PNG as `PNG`, a DIB
/// as `CF_DIBV5`, a sticker as `WTE.Sticker.v1` of another process, a virtual file as `FileGroupDescriptorW` +
/// `FileContents` (an `IStream`).
#[cfg(windows)]
fn entries_of(items: &[DropItem]) -> Vec<data_object::Entry> {
    use data_object::Entry;
    let f = drop_target::Formats::get();
    let files: Vec<PathBuf> = items.iter().filter_map(|i| if let DropItem::File(p) = i { Some(p.clone()) } else { None }).collect();
    let mut entries = Vec::new();
    if !files.is_empty() {
        entries.push(Entry::global(formats::CF_HDROP, formats::build_dropfiles(&files)));
    }
    for item in items {
        let DropItem::Data { name, kind, bytes } = item else { continue };
        match kind {
            DataKind::Png => entries.push(Entry::global(f.png, bytes.clone())),
            DataKind::Dib => entries.push(Entry::global(formats::CF_DIBV5, bytes.clone())),
            DataKind::Sticker => {
                entries.push(Entry::global(f.sticker, formats::wrap_sticker(std::process::id().wrapping_add(1), bytes)));
            }
            DataKind::File => {
                let d = formats::FileDescriptor { name: name.clone(), size: Some(bytes.len() as u64) };
                entries.push(Entry::global(f.file_group, formats::build_file_group(&[d])));
                entries.push(Entry::stream(f.file_contents, 0, bytes.clone()));
            }
        }
    }
    entries
}

/// What `items` carry (the non-Windows `TestDrag` and the tests).
#[cfg_attr(all(windows, not(test)), allow(dead_code, reason = "the non-Windows test drag"))]
pub fn kinds_of(items: &[DropItem]) -> DragKinds {
    let mut kinds = DragKinds::default();
    for item in items {
        match item {
            DropItem::File(p) => kinds.extensions.push(crate::dnd::extension_of(p)),
            DropItem::Data { kind: DataKind::Png | DataKind::Dib, .. } => kinds.picture = true,
            DropItem::Data { kind: DataKind::Sticker, .. } => kinds.sticker = true,
            DropItem::Data { kind: DataKind::File, name, .. } => kinds.extensions.push(crate::dnd::extension_of(std::path::Path::new(name))),
        }
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_is_capped_and_drained_in_order() {
        // Other tests may use the queue at the same time: only this test's own events are looked at.
        let mine = |e: &OleEvent| matches!(e, OleEvent::Hover { pos: [x, _], .. } if *x == -77.0);
        for i in 0..QUEUE_CAP + 10 {
            push(OleEvent::Hover { pos: [-77.0, i as f32], kinds: DragKinds::default() });
        }
        let got: Vec<OleEvent> = drain().into_iter().filter(mine).collect();
        assert!(got.len() <= QUEUE_CAP);
        let ys: Vec<f32> = got.iter().map(|e| if let OleEvent::Hover { pos, .. } = e { pos[1] } else { 0.0 }).collect();
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "oldest first");
        assert_eq!(ys.last().copied(), Some((QUEUE_CAP + 9) as f32), "the newest is kept");
    }

    /// The runner's drag (through the real drop target and data object on Windows) queues what a real drop would.
    #[test]
    fn a_test_drag_queues_hover_and_drop() {
        let items = vec![
            DropItem::Data { name: "Dropped picture.png".to_owned(), kind: DataKind::Png, bytes: b"\x89PNG-test-drag".to_vec() },
        ];
        let drag = TestDrag::enter(&items, [-55.0, 1.0]);
        drag.over([-55.0, 2.0]);
        drag.drop_at([-55.0, 3.0]);
        let mine: Vec<OleEvent> = drain()
            .into_iter()
            .filter(|e| matches!(e, OleEvent::Hover { pos: [x, _], .. } | OleEvent::Drop { pos: [x, _], .. } if *x == -55.0))
            .collect();
        assert_eq!(mine.len(), 3, "{mine:?}");
        assert!(matches!(&mine[0], OleEvent::Hover { kinds, .. } if kinds.picture));
        assert_eq!(mine[2], OleEvent::Drop { pos: [-55.0, 3.0], items, skipped: 0 });
        assert_eq!(kinds_of(&[DropItem::File(PathBuf::from("a.WTEP"))]).extensions, ["wtep"]);
    }
}
