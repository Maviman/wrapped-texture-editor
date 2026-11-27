// Owner: stickers P11 (STICKER_PLAN 6.4, 6.5, 13.5, the P11 rows of 7 and 13.9; ARCHITECTURE 8.17 "Drag out"). Drag out
// of WTE: a row of the Layers list (texture layers, stamp layers, the Sticker view's sticker layers), the Sticker tab's
// thumbnails (the selected stamp, the tray) and the PSD handle become an OS drag when the pointer leaves the window with
// the primary button held. Inside the window nothing changes: the Layers rows still reorder.
//
// The state machine (one `Pending` at a time, `DragOutUi`):
// 1. `begin` (the `AppAction::DragOut` of a source whose egui drag just started): a worker renders / encodes the PNG
//    (`PngLevel::Fast`) into the drag-out folder and builds the data object's entries - `CF_HDROP` with that one file
//    first, then `PNG`, `CF_DIBV5` and, for stickers, `WTE.Sticker.v1` (review A23: prepared before the pointer leaves).
//    The PSD handle needs no job: its link is made at once (`link::flows::prepare_handle`: save first, the shared
//    picture, a new link's PSD) and its entry is `CF_HDROP` with the linked PSD.
// 2. `pump` (every frame, after dispatch): the worker's result is taken (and the file recorded in the manifest); the
//    button released before the OS drag started cancels it and removes the file nobody took; ready + button held +
//    pointer outside the window (or gone) -> `DragPort::post` (`ole::drag_source::request` + the posted message). Not
//    ready yet: it is posted in the frame the job finishes, if the button is still held and the pointer still outside.
// 3. The subclass runs `DoDragDrop` outside the frame; its end comes back as `ole::DragEnded`. `input_hook` (from
//    `raw_input_hook`, before egui sees the frame) injects the primary button's release and `PointerGone` that OLE ate,
//    clears egui's drag payload and stops the row drag (and tells the rows not to take that release as a reorder).
//    `finish`: a PSD handle drag that was taken marks the link opened (5.5) and shows the Photoshop tab-bar toast once
//    per session (13.5); a PNG nobody took (Esc, no target) is removed.
//
// Never `DoDragDrop` inside a frame (critic C4); in a self-test run and in tests the port is the test port (the drag
// is recorded by `ole::drag_source::Recorder`, never the OS) and the runner's frame delivers the post.

pub(crate) mod files;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use eframe::egui;
use wte_core::sticker::select;
use wte_core::{AlphaBuf, IRect, LayerId, LayerKind, PixelBuf, StampId, StickerContent, StickerId, StickerLayerId};

use crate::clip::{Picked, Take};
use crate::link::flows::HandleReady;
use crate::ole::clipboard::{self, ClipFormat};
use crate::state::AppState;

/// Where a drag out starts.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DragSource {
    /// A row of the Layers list: a paint layer's full-canvas picture (the pixels "Export this layer" writes); a stamp's
    /// or text's layer hands out the stamp's finished picture.
    Layer(LayerId),
    /// A row of the Sticker view's Layers list: that layer of the edited sticker.
    StickerLayer(StickerLayerId),
    /// The Sticker tab's thumbnail of the selected stamp.
    Stamp(StampId),
    /// A thumbnail of the Sticker tab's tray.
    Tray(StickerId),
    /// The PSD handle beside the selected stamp's thumbnail: the stamp's linked PSD.
    PsdHandle(StampId),
}

/// The toast after a PSD-handle drag that was taken, once per session (13.5).
pub(crate) const PHOTOSHOP_TAB_TOAST: &str =
    "In Photoshop the sticker stays linked only when it opened as its own tab. Drop it on the tab bar, not on a picture.";

/// The PSD handle's caption (13.9).
pub(crate) const HANDLE_CAPTION: &str = "drag to your editor: stays linked";

/// The status line of a click on the PSD handle (it is dragged, not clicked).
pub(crate) const HANDLE_CLICK_HINT: &str = "Drag the PSD handle onto your editor (in Photoshop: onto the tab bar) to open the linked file there";

/// What the OS drag hands over: the entries in the order they are offered (`CF_HDROP` first) and the file.
#[derive(Clone, Debug)]
pub struct Offer {
    pub entries: Vec<(ClipFormat, Vec<u8>)>,
    pub file: PathBuf,
    /// The PNG WTE wrote for this drag (removed again when nobody takes it); `None` for the linked PSD.
    written: Option<files::ManifestEntry>,
    /// The PSD handle's link.
    psd: Option<HandleReady>,
}

enum Phase {
    /// The worker renders and encodes.
    Preparing { rx: mpsc::Receiver<Result<Offer, String>>, cancel: Arc<AtomicBool> },
    /// Ready, waiting for the pointer to leave the window.
    Ready(Offer),
    /// Handed to the OS drag (posted), waiting for its end.
    Posted(Offer),
}

struct Pending {
    tag: u64,
    source: DragSource,
    phase: Phase,
}

/// How a prepared drag reaches `DoDragDrop`.
pub trait DragPort {
    /// Hands the entries to the window's drag message (moved: a large layer's PNG and DIB are not kept twice while
    /// the drag runs). `Err` = a sentence for the status line.
    fn post(&mut self, tag: u64, entries: Vec<(ClipFormat, Vec<u8>)>) -> Result<(), String>;
}

/// The port of every self-test run and every test: through `ole::drag_source`'s slot, guard and data object, but the
/// "post" is delivered by the frame (`deliver_test_post`) to the recorder, never to the OS.
pub struct TestPort;

impl DragPort for TestPort {
    fn post(&mut self, tag: u64, entries: Vec<(ClipFormat, Vec<u8>)>) -> Result<(), String> {
        #[cfg(windows)]
        {
            let job = crate::ole::drag_source::DragJob { tag, entries: data_entries(entries) };
            crate::ole::drag_source::request(job, &mut crate::ole::drag_source::post_for_test).map_err(refused_text)
        }
        #[cfg(not(windows))]
        {
            let _ = (tag, entries);
            Err("Dragging out of WTE works on Windows only".to_owned())
        }
    }
}

/// The port of an interactive Windows run with WTE's subclass installed: `PostMessageW(window, WTE.OleDrag.v1)`.
#[cfg(all(windows, not(test)))]
pub struct WindowPort(pub isize);

#[cfg(all(windows, not(test)))]
impl DragPort for WindowPort {
    fn post(&mut self, tag: u64, entries: Vec<(ClipFormat, Vec<u8>)>) -> Result<(), String> {
        let hwnd = self.0;
        let job = crate::ole::drag_source::DragJob { tag, entries: data_entries(entries) };
        crate::ole::drag_source::request(job, &mut || crate::ole::drag_source::post_to_window(hwnd)).map_err(refused_text)
    }
}

#[cfg(windows)]
fn refused_text(r: crate::ole::drag_source::Refused) -> String {
    match r {
        crate::ole::drag_source::Refused::Busy => "A drag is still running".to_owned(),
        crate::ole::drag_source::Refused::NotPosted => "The drag could not be started".to_owned(),
    }
}

/// The data object's entries (format ids) for `entries`.
#[cfg(windows)]
fn data_entries(entries: Vec<(ClipFormat, Vec<u8>)>) -> Vec<crate::ole::data_object::Entry> {
    use crate::ole::formats;
    let f = crate::ole::drop_target::Formats::get();
    entries
        .into_iter()
        .map(|(format, bytes)| {
            let id = match format {
                ClipFormat::Sticker => f.sticker,
                ClipFormat::Png => f.png,
                ClipFormat::DibV5 => formats::CF_DIBV5,
                ClipFormat::Dib => formats::CF_DIB,
                ClipFormat::HDrop => formats::CF_HDROP,
                ClipFormat::UnicodeText => formats::CF_UNICODETEXT,
            };
            crate::ole::data_object::Entry::global(id, bytes)
        })
        .collect()
}

/// The drag-out state (`AppState::drag_out`).
pub struct DragOutUi {
    /// The drag-out folder (`None`: drag out is off - a test that did not set one).
    pub(crate) folder: Option<PathBuf>,
    /// How a drag reaches the OS (`None`: not available in this run, e.g. the subclass could not be installed).
    pub(crate) port: Option<Box<dyn DragPort>>,
    pending: Option<Pending>,
    next_tag: u64,
    /// The Photoshop tab-bar toast was shown in this session (13.5).
    pub(crate) psd_toast_shown: bool,
    /// The injected release of a finished drag is in this frame: the Layers rows do not take it as a reorder.
    pub(crate) released_this_frame: bool,
    /// Where the pointer was when the drag was posted (the injected release's position).
    last_pos: Option<egui::Pos2>,
    /// Counters for tests and the self-test: jobs started, OS drags posted, drags ended, files removed unused.
    pub(crate) jobs: u64,
    pub(crate) posted: u64,
    pub(crate) ended: u64,
    pub(crate) removed: u64,
    /// The last frame's egui drag state (the self-test's "the next move drags nothing").
    pub(crate) egui_dragging: bool,
    /// The effect the self-test's recorder answers (`DROPEFFECT_COPY`; 0 = the user pressed Esc).
    pub(crate) test_effect: u32,
}

impl DragOutUi {
    /// The state of a run: self-test runs and tests get the test port; an interactive run gets its port and folder
    /// in `WteApp::new` (`install`).
    pub fn new(selftest: bool) -> DragOutUi {
        DragOutUi {
            folder: None,
            port: selftest.then(|| Box::new(TestPort) as Box<dyn DragPort>),
            pending: None,
            next_tag: 1,
            psd_toast_shown: false,
            released_this_frame: false,
            last_pos: None,
            jobs: 0,
            posted: 0,
            ended: 0,
            removed: 0,
            egui_dragging: false,
            test_effect: 1,
        }
    }

    /// True while a drag out is being prepared or runs (the self-test's idle check).
    pub(crate) fn busy(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| matches!(p.phase, Phase::Preparing { .. } | Phase::Posted(_)))
    }

    /// The source of the drag in progress, and whether it is ready / posted (tests, scenario asserts).
    pub(crate) fn current(&self) -> Option<(DragSource, &'static str)> {
        self.pending.as_ref().map(|p| {
            let phase = match p.phase {
                Phase::Preparing { .. } => "preparing",
                Phase::Ready(_) => "ready",
                Phase::Posted(_) => "posted",
            };
            (p.source, phase)
        })
    }

    /// The file of the drag in progress, once prepared.
    #[cfg(test)]
    pub(crate) fn current_file(&self) -> Option<&std::path::Path> {
        match &self.pending.as_ref()?.phase {
            Phase::Ready(o) | Phase::Posted(o) => Some(&o.file),
            Phase::Preparing { .. } => None,
        }
    }
}

/// The drag-out folder of an interactive run: `%LOCALAPPDATA%\WrappedTextureEditor\dragout`.
pub fn default_folder() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).map(|v| PathBuf::from(v).join("WrappedTextureEditor").join("dragout"))
}

/// Start-up (`WteApp::new`): the folder (`folder`), the start-up cleanup of 6.4 (B7), and the window port of an
/// interactive run with WTE's subclass installed (`window`; a self-test run keeps the test port).
pub fn install(state: &mut AppState, folder: Option<PathBuf>, window: Option<isize>) {
    if let Some(dir) = &folder {
        let c = files::cleanup(dir, std::time::SystemTime::now());
        log::info!(
            "drag out: folder {} ({} old file(s) of WTE's removed, {} kept, {} changed by others and left alone)",
            dir.display(),
            c.removed.len(),
            c.kept_fresh.len(),
            c.forgotten.len()
        );
    }
    state.drag_out.folder = folder;
    if !state.selftest {
        #[cfg(all(windows, not(test)))]
        {
            state.drag_out.port = window.map(|h| Box::new(WindowPort(h)) as Box<dyn DragPort>);
        }
        #[cfg(not(all(windows, not(test))))]
        let _ = window;
        log::info!("drag out: {}", if state.drag_out.port.is_some() { "on (WTE's OLE drag source)" } else { "off (WTE's window subclass is not installed)" });
    }
}

/// `AppAction::DragOut(source)`: a drag on `source` was recognised (its egui drag started). Starts its job (or, for the
/// PSD handle, makes the link); a drag still pending is cancelled first.
pub(crate) fn begin(ctx: &egui::Context, state: &mut AppState, source: DragSource) {
    if state.drag_out.port.is_none() {
        return;
    }
    cancel(state);
    let Some(folder) = state.drag_out.folder.clone() else { return };
    let tag = state.drag_out.next_tag;
    state.drag_out.next_tag += 1;
    if let DragSource::PsdHandle(stamp) = source {
        let Some(ready) = crate::link::flows::prepare_handle(state, crate::link::EditRequest::for_handle(stamp)) else { return };
        let entries = vec![(ClipFormat::HDrop, crate::ole::formats::build_dropfiles(std::slice::from_ref(&ready.path)))];
        let offer = Offer { entries, file: ready.path.clone(), written: None, psd: Some(ready) };
        state.drag_out.pending = Some(Pending { tag, source, phase: Phase::Ready(offer) });
        return;
    }
    match pick(state, source) {
        Ok(Some(picked)) => start_job(ctx, state, tag, source, folder, picked),
        Ok(None) => {}
        Err(hint) => state.status_hint = hint,
    }
}

/// Starts the worker of a drag (`begin`; the tests hand in their own `Picked`).
pub(crate) fn start_job(ctx: &egui::Context, state: &mut AppState, tag: u64, source: DragSource, folder: PathBuf, picked: Picked) {
    let (tx, rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancel_flag);
    let ctx = ctx.clone();
    let name = files::file_name(&picked.name, "png");
    let take = picked.take;
    std::thread::spawn(move || {
        let result = prepare(&folder, &name, take, &stop);
        let written = result.as_ref().ok().and_then(|o| o.written.clone().map(|e| (o.file.clone(), e)));
        if stop.load(Ordering::Acquire) || tx.send(result).is_err() {
            // Cancelled meanwhile: nobody will take the file.
            if let Some((path, entry)) = written {
                files::remove_unused(&folder, &path, Some(&entry));
            }
        }
        ctx.request_repaint();
    });
    state.drag_out.jobs += 1;
    log::info!("drag out: {source:?} recognised; its picture is prepared");
    state.drag_out.pending = Some(Pending { tag, source, phase: Phase::Preparing { rx, cancel: cancel_flag } });
}

/// The worker: the picture, its PNG (Fast) as a new file in `folder`, and the entries.
fn prepare(folder: &std::path::Path, name: &str, take: Take, stop: &AtomicBool) -> Result<Offer, String> {
    let cancelled = || stop.load(Ordering::Acquire);
    if cancelled() {
        return Err(CANCELLED.to_owned());
    }
    let (pixels, wtes) = take()?;
    if cancelled() {
        return Err(CANCELLED.to_owned());
    }
    let png = wte_core::encode_png(pixels.width, pixels.height, &pixels.data, wte_core::PngLevel::Fast).map_err(|e| e.to_string())?;
    if cancelled() {
        return Err(CANCELLED.to_owned());
    }
    let (path, entry) = files::write_new(folder, name, &png)?;
    let mut entries = vec![(ClipFormat::HDrop, crate::ole::formats::build_dropfiles(std::slice::from_ref(&path)))];
    entries.extend(clipboard::picture_entries(png, &pixels, wtes.as_deref()));
    Ok(Offer { entries, file: path, written: Some(entry), psd: None })
}

/// What a cancelled job ends with (never shown).
const CANCELLED: &str = "the drag was cancelled";

/// The status line of a paint layer without pixels.
pub(crate) const EMPTY_LAYER_HINT: &str = "This layer has no pixels yet: nothing to drag out";

/// What a source hands out (the picture is taken here; rendering a stamp's styles happens here as for Copy, the PNG
/// on the worker). `Ok(None)`: the source is gone.
fn pick(state: &mut AppState, source: DragSource) -> Result<Option<Picked>, String> {
    match source {
        DragSource::Stamp(stamp) => crate::clip::stamp_take(state, stamp),
        DragSource::Layer(id) => {
            let Some(doc) = state.doc() else { return Ok(None) };
            let Some((set, _)) = doc.find_layer(id) else { return Ok(None) };
            let Some(layer) = doc.layer(id) else { return Ok(None) };
            if let Some(stamp) = layer.stamp_id() {
                return crate::clip::stamp_take(state, stamp);
            }
            let [w, h] = doc.set(set).map_or([0, 0], |s| [s.width, s.height]);
            match &layer.kind {
                LayerKind::Raster(buf) if buf.width == w && buf.height == h && w > 0 && h > 0 => {
                    let buf = Arc::clone(buf);
                    Ok(Some(Picked {
                        what: format!("the layer \"{}\"", layer.name),
                        name: layer.name.clone(),
                        take: Box::new(move || Ok((buf, None))),
                    }))
                }
                _ => Err(EMPTY_LAYER_HINT.to_owned()),
            }
        }
        DragSource::StickerLayer(id) => sticker_layer(state, id),
        DragSource::Tray(id) => {
            let Some(doc) = state.doc_mut() else { return Ok(None) };
            let Some(sticker) = doc.sticker(id).map(|d| (**d).clone()) else { return Ok(None) };
            let picture = doc.sticker_picture(id, None, 1.0).map_err(|e| e.to_string())?;
            let blobs = doc.blobs.clone();
            Ok(Some(Picked {
                what: format!("the sticker \"{}\"", sticker.name),
                name: sticker.name.clone(),
                take: Box::new(move || {
                    let wtes = wte_core::write_wtes(&sticker, None, &blobs, Some(&picture))
                        .map_err(|e| log::warn!("drag out: the sticker could not be written as WTE.Sticker.v1 ({e})"))
                        .ok();
                    Ok((Arc::new(picture), wtes))
                }),
            }))
        }
        DragSource::PsdHandle(_) => Ok(None),
    }
}

/// A layer of the edited sticker: a picture layer's pixels with its mask applied (as Copy in the Sticker view takes
/// them, without layer styles); a text or shape layer drawn alone and cut to what it covers.
fn sticker_layer(state: &mut AppState, id: StickerLayerId) -> Result<Option<Picked>, String> {
    let Some(edit) = crate::sticker::edit_of(state).cloned() else { return Ok(None) };
    let Some(on) = crate::sticker::edit_ref(&edit) else { return Ok(None) };
    let Some(session) = state.session.as_mut() else { return Ok(None) };
    let Some(shown) = crate::sticker::shown(&mut session.doc, &edit) else { return Ok(None) };
    let Some(doc) = shown.layers() else { return Err(crate::sticker::tools::TEXT_STAMP_HINT.to_owned()) };
    let Some(layer) = doc.layer(id).cloned() else { return Ok(None) };
    let (artboard, what, name) = (doc.artboard, format!("the layer \"{}\"", layer.name), layer.name.clone());
    let take: Take = if matches!(layer.content, StickerContent::Picture(_)) {
        let pixels = session.doc.sticker_layer_pixels(on, id).map_err(|e| e.to_string())?;
        let mask = layer.mask.as_ref().map(|m| Arc::clone(&m.pixels));
        let transform = layer.transform;
        Box::new(move || crate::clip::layer_pixels(&pixels, mask.as_deref(), None, artboard, &transform).map(|p| (Arc::new(p), None)))
    } else {
        let drawn = session.doc.sticker_layer_on_artboard(on, id).map_err(|e| e.to_string())?;
        Box::new(move || {
            let bounds = select::mask_bounds(&AlphaBuf::from_alpha(&drawn));
            if bounds.is_empty() {
                return Err("The layer shows nothing to drag out".to_owned());
            }
            Ok((Arc::new(crop(&drawn, bounds)), None))
        })
    };
    Ok(Some(Picked { what, name, take }))
}

fn crop(p: &PixelBuf, r: IRect) -> PixelBuf {
    let (w, h) = (r.width(), r.height());
    let mut out = PixelBuf::new(w, h);
    for y in 0..h as usize {
        let s = ((r.y0 as usize + y) * p.width as usize + r.x0 as usize) * 4;
        out.data[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(&p.data[s..s + w as usize * 4]);
    }
    out
}

/// The pointer as the drag-out state machine needs it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct PointerNow {
    /// The primary button is held.
    pub down: bool,
    /// The pointer is outside the window's rect (or gone: `PointerGone` while the button is held).
    pub outside: bool,
    pub pos: Option<egui::Pos2>,
}

/// The pointer of this frame.
pub(crate) fn pointer_now(ctx: &egui::Context) -> PointerNow {
    ctx.input(|i| {
        let pos = i.pointer.latest_pos();
        let rect = i.viewport_rect();
        PointerNow { down: i.pointer.primary_down(), outside: pos.is_none_or(|p| !rect.contains(p)), pos }
    })
}

/// Every frame (`App::ui`, after dispatch): the job's result, the release, the post (see the module comment).
pub(crate) fn pump(ctx: &egui::Context, state: &mut AppState) {
    state.drag_out.egui_dragging = ctx.dragged_id().is_some() || egui::DragAndDrop::has_any_payload(ctx);
    step(state, pointer_now(ctx));
}

/// `pump` on a given pointer (the tests).
pub(crate) fn step(state: &mut AppState, now: PointerNow) {
    poll_job(state);
    let Some(p) = state.drag_out.pending.as_ref() else { return };
    match &p.phase {
        Phase::Posted(_) => {}
        _ if !now.down => {
            log::info!("drag out: the button was released in the window; nothing was dragged out");
            cancel(state);
        }
        Phase::Ready(_) if now.outside => post(state, now.pos),
        _ => {}
    }
}

/// Takes the worker's result when it is there.
fn poll_job(state: &mut AppState) {
    let Some(p) = state.drag_out.pending.as_mut() else { return };
    let Phase::Preparing { rx, .. } = &p.phase else { return };
    match rx.try_recv() {
        Ok(Ok(offer)) => {
            if let (Some(folder), Some(entry)) = (&state.drag_out.folder, &offer.written) {
                files::record(folder, entry.clone());
            }
            p.phase = Phase::Ready(offer);
        }
        Ok(Err(e)) => {
            state.drag_out.pending = None;
            if e != CANCELLED {
                state.toasts.warn(format!("Could not drag this out: {e}"));
            }
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => state.drag_out.pending = None,
    }
}

/// Hands the ready drag to the port.
fn post(state: &mut AppState, pos: Option<egui::Pos2>) {
    let Some(mut p) = state.drag_out.pending.take() else { return };
    let Phase::Ready(mut offer) = p.phase else {
        state.drag_out.pending = Some(p);
        return;
    };
    let Some(port) = state.drag_out.port.as_mut() else { return };
    let names: Vec<&str> = offer.entries.iter().map(|(f, _)| f.name()).collect();
    match port.post(p.tag, std::mem::take(&mut offer.entries)) {
        Ok(()) => {
            log::info!("drag out: {} ({:?}) handed to the OS drag as {}", offer.file.display(), p.source, names.join(", "));
            state.drag_out.posted += 1;
            state.drag_out.last_pos = pos;
            p.phase = Phase::Posted(offer);
            state.drag_out.pending = Some(p);
        }
        Err(e) => {
            state.status_hint = e;
            remove_unused(state, &offer);
        }
    }
}

/// Drops the pending drag (a new one, or the release in the window): its job is told to stop and a PNG nobody took is
/// removed. A posted drag cannot be taken back (the OS runs it); it is only forgotten.
pub(crate) fn cancel(state: &mut AppState) {
    let Some(p) = state.drag_out.pending.take() else { return };
    match p.phase {
        Phase::Preparing { rx, cancel } => {
            cancel.store(true, Ordering::Release);
            // A result that arrived meanwhile: its file is removed here (the worker removes it when the send fails).
            if let Ok(Ok(offer)) = rx.try_recv() {
                remove_unused(state, &offer);
            }
        }
        Phase::Ready(offer) => remove_unused(state, &offer),
        Phase::Posted(_) => {}
    }
}

/// Removes the PNG of `offer` that nobody took (never the linked PSD).
fn remove_unused(state: &mut AppState, offer: &Offer) {
    if let (Some(folder), Some(entry)) = (&state.drag_out.folder, &offer.written) {
        files::remove_unused(folder, &offer.file, Some(entry));
        state.drag_out.removed += 1;
    }
}

/// `raw_input_hook` (after the self-test's input): a finished OS drag injects the primary button's release and
/// `PointerGone` (OLE ate the release; egui would think the button is still held), clears egui's drag payload, stops
/// the row's drag and tells the rows of this frame not to take the release as a reorder; then `finish`.
pub(crate) fn input_hook(ctx: &egui::Context, raw: &mut egui::RawInput, state: &mut AppState) {
    state.drag_out.released_this_frame = false;
    let ended = crate::ole::take_drag_ended();
    if ended.is_empty() {
        return;
    }
    let pos = state.drag_out.last_pos.or_else(|| ctx.input(|i| i.pointer.latest_pos())).unwrap_or(egui::Pos2::ZERO);
    let modifiers = ctx.input(|i| i.modifiers);
    raw.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers });
    raw.events.push(egui::Event::PointerGone);
    egui::DragAndDrop::clear_payload(ctx);
    ctx.stop_dragging();
    state.drag_out.released_this_frame = true;
    for e in ended {
        state.drag_out.ended += 1;
        let ours = state.drag_out.pending.as_ref().is_some_and(|p| p.tag == e.tag && matches!(p.phase, Phase::Posted(_)));
        if !ours {
            log::info!("drag out: the end of drag {} arrived after it was dropped", e.tag);
            continue;
        }
        if let Some(Pending { phase: Phase::Posted(offer), .. }) = state.drag_out.pending.take() {
            finish(state, offer, e.effect, e.dropped);
        }
    }
}

/// The OS drag ended with `effect` (0 = cancelled or not taken); `dropped`: it ended with a drop.
fn finish(state: &mut AppState, offer: Offer, effect: u32, dropped: bool) {
    let file = offer.file.file_name().map_or_else(|| offer.file.display().to_string(), |n| n.to_string_lossy().into_owned());
    if effect == 0 {
        if dropped {
            // P11 check: a program may answer "no effect" and still read the file after the drop; a PNG of WTE's stays
            // listed in the manifest and goes with the start-up cleanup a day later.
            log::info!("drag out: {file} was dropped but not taken; it stays for the day-old cleanup");
        } else {
            log::info!("drag out: {file} was not taken (cancelled)");
            remove_unused(state, &offer);
        }
        return;
    }
    let Some(ready) = &offer.psd else {
        state.status_hint = format!("Dragged out {file}");
        return;
    };
    // The linked file was handed to a program: the link counts as opened from now on (5.5, `opened_at`).
    let s = state.link_ui.session(ready.sticker);
    s.stale_in_editor = wte_core::link::next_stale(s.stale_in_editor, wte_core::link::StaleEvent::EditAgain { child_exited: false });
    s.opened = true;
    s.opened_editor = Some(ready.editor.clone());
    state.status_hint = format!("Dragged out {file}: Ctrl+S in {} updates the sticker", ready.editor_name);
    // 13.5: in Photoshop only a drop on the tab bar (or the empty workspace) keeps the link; once per session.
    let photoshop = ready.editor == wte_core::link::EDITOR_PHOTOSHOP
        || state.link_ui.known.iter().any(|e| matches!(e, crate::link::editor::ExternalEditor::Photoshop));
    if photoshop && !state.drag_out.psd_toast_shown {
        state.drag_out.psd_toast_shown = true;
        state.toasts.info(PHOTOSHOP_TAB_TOAST);
    }
}

/// A self-test run's frame (`raw_input_hook`, after the real OLE events were dropped): the test port's post reaches
/// the recorder here, where the subclass would run `DoDragDrop`.
pub(crate) fn deliver_test_post(state: &AppState) {
    #[cfg(windows)]
    {
        crate::ole::drag_source::deliver_test_post(&crate::ole::drag_source::Recorder { effect: state.drag_out.test_effect });
    }
    #[cfg(not(windows))]
    let _ = state;
}
