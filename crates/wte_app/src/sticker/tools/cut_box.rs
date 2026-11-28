// Owner: stickers S1 (STICKER_PLAN 7 "S1", 4.5 "GrabCut box"; ARCHITECTURE 8.15). The Cut Out Box tool: drag a box
// around the object of a photo on the active picture layer; GrabCut (`wte_core::cut_out_box`) keeps the object and
// clears the rest, with Remove Background's soft edge. Option "Into mask (keep pixels)": the result goes to the layer
// mask (a mask is added when needed). Limited by the selection like every cut-out command; one entry "Cut Out Box"
// (an image stamp is promoted inside it).
//
// The cut takes up to about a second at 2048 px (plan 8.4: 3 s), so it runs on a job thread with a progress note in
// the canvas header and the Tool tab (Cancel, or Esc). Its inputs are taken on the UI thread as `Arc`s (no copy); the
// thread computes the cut and the changed buffer; the frame that sees the result puts it in as one entry when the
// layer still holds the pixels (and mask) the cut was made from, else drops it and says why. At most one cut thread
// runs: a new box while one runs cancels it and starts when its thread has ended (the cut checks for the cancel
// between its steps). A result that lands during a drag or a stroke waits for its end. Leaving the view cancels.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use glam::Vec2;
use wte_core::{AlphaBuf, BoxCutRefusal, IRect, PixelBuf, PixelTarget};

use crate::state::{AppState, StickerTool};
use crate::sticker::filters::{self, Change};
use crate::sticker::{edit_of, edit_ref};

use super::PaintTarget;

/// The history label.
pub(crate) const CUT_BOX_LABEL: &str = "Cut Out Box";
/// The largest picture side the tool works on (its full-size planes are a few bytes a pixel, several of them).
pub(crate) const MAX_CUT_SIDE: u32 = 4096;
/// The status line when the layer changed while the cut ran.
pub(crate) const CHANGED_NOTE: &str = "Cut Out Box: the layer changed while it was cutting; drag the box again";
/// Below this box side (layer-space pixels) a drag is a click and cuts nothing.
pub(crate) const MIN_DRAG: f32 = 2.0;

/// The tool's options (`StickerUi::cut_box`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct CutBoxOptions {
    /// Hide the background on the layer mask; the pixels are kept.
    pub into_mask: bool,
}

/// A running cut (`StickerUi::cut_job`): what it was started on and its channel.
pub(crate) struct CutJob {
    t: PaintTarget,
    /// The buffers the cut was made from (compared when the result lands).
    pixels: Arc<PixelBuf>,
    mask: Option<Arc<AlphaBuf>>,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<Change, BoxCutRefusal>>,
    /// The result, kept while a drag or a stroke runs.
    ready: Option<Result<Change, BoxCutRefusal>>,
}

/// The cut threads (`StickerUi::cut_threads`): how many run (a cancelled one included) and the box that waits for
/// the last one to end.
#[derive(Default)]
pub(crate) struct CutThreads {
    pub(crate) alive: Arc<AtomicU64>,
    queued: Option<[f32; 4]>,
    /// The most cut threads that ran at the same time (tests: one).
    pub(crate) most_alive: u64,
    pub(crate) started: u64,
    /// Tests: while set, a cut thread waits before it sends its result.
    #[cfg(test)]
    pub(crate) hold: Arc<AtomicBool>,
}

/// Counts a cut thread while it runs; at its end (a panic included) the count drops and a frame is asked for.
struct AliveGuard {
    alive: Arc<AtomicU64>,
    ctx: eframe::egui::Context,
}

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::AcqRel);
        self.ctx.request_repaint();
    }
}

/// True while a cut runs or waits to start or to land (the self-test's idle check waits for it).
pub(crate) fn pending(state: &AppState) -> bool {
    state.sticker_ui.cut_job.is_some() || state.sticker_ui.cut_threads.queued.is_some()
}

/// The share done of the running cut (percent), `None` without one.
pub(crate) fn progress(state: &AppState) -> Option<u32> {
    if let Some(job) = state.sticker_ui.cut_job.as_ref() {
        return Some(job.progress.load(Ordering::Acquire).min(100));
    }
    state.sticker_ui.cut_threads.queued.map(|_| 0)
}

/// The header's and the Tool tab's note while a cut runs.
pub(crate) fn note(pc: u32) -> String {
    format!("Cutting out... {pc} %")
}

/// The box `start`..`to` (layer space) as `[x0, y0, x1, y1]`, `None` for a click.
pub(crate) fn drag_box(start: Vec2, to: Vec2) -> Option<[f32; 4]> {
    let (a, b) = (start.min(to), start.max(to));
    let d = b - a;
    (a.is_finite() && b.is_finite() && d.x >= MIN_DRAG && d.y >= MIN_DRAG).then_some([a.x, a.y, b.x, b.y])
}

/// The box (layer space) in the content pixels of the target: the bounds of its four corners mapped through the
/// layer's transform, rounded outward (a turned layer gets the bounding box of the turned box).
pub(crate) fn content_rect(t: &PaintTarget, b: [f32; 4]) -> IRect {
    let corners = [Vec2::new(b[0], b[1]), Vec2::new(b[2], b[1]), Vec2::new(b[2], b[3]), Vec2::new(b[0], b[3])].map(|c| t.to_content(c));
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for c in corners {
        lo = lo.min(c);
        hi = hi.max(c);
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return IRect::EMPTY;
    }
    let clamp = |v: f32| v.clamp(-(1 << 24) as f32, (1 << 24) as f32) as i32;
    IRect::new(clamp(lo.x.floor()), clamp(lo.y.floor()), clamp(hi.x.ceil()), clamp(hi.y.ceil()))
}

/// `StickerAction::CutOutBox`: the cut of the active picture layer inside the layer-space box `b` is asked for (a
/// running cut is cancelled). It starts after the frame's dispatch (`after_dispatch`), once no cut thread runs any
/// more.
pub(crate) fn start(state: &mut AppState, b: [f32; 4]) {
    if let Some(job) = state.sticker_ui.cut_job.take() {
        job.cancel.store(true, Ordering::Release);
    }
    state.sticker_ui.cut_threads.queued = Some(b);
}

/// After the frame's actions (and in every canvas frame): starts the box that was asked for when no cut thread runs.
pub(crate) fn after_dispatch(ctx: &eframe::egui::Context, state: &mut AppState) {
    if let Some(b) = state.sticker_ui.cut_threads.queued
        && state.sticker_ui.cut_job.is_none()
        && state.sticker_ui.cut_threads.alive.load(Ordering::Acquire) == 0
    {
        state.sticker_ui.cut_threads.queued = None;
        if edit_of(state).is_some() {
            launch(state, b, ctx);
        }
    }
}

/// Starts the thread of a cut (the inputs are gathered here as `Arc`s).
fn launch(state: &mut AppState, b: [f32; 4], ctx: &eframe::egui::Context) {
    let Some(mut t) = super::paint_target(state, StickerTool::CutOutBox, false) else { return };
    // The cut is about the picture: on "Paint on mask" it still looks at the pixels; "Into mask" decides the target.
    t.target = PixelTarget::Pixels;
    if t.size.iter().any(|&s| s > MAX_CUT_SIDE) {
        state.status_hint = format!("{CUT_BOX_LABEL} works on pictures up to {MAX_CUT_SIDE} pixels a side");
        return;
    }
    let rect = content_rect(&t, b);
    let inside = rect.intersect(&IRect::from_size(t.size[0], t.size[1]));
    if inside.width() < wte_core::sticker::grabcut::MIN_BOX_SIDE || inside.height() < wte_core::sticker::grabcut::MIN_BOX_SIDE {
        say(state, BoxCutRefusal::BoxTooSmall.message());
        return;
    }
    let selection = super::selection_for(state, &t);
    let into_mask = state.sticker_ui.cut_box.into_mask;
    let Some(doc) = state.doc_mut() else { return };
    let pixels = match doc.sticker_layer_pixels(t.on, t.layer) {
        Ok(p) => p,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return;
        }
    };
    let mask = doc.sticker_layer_mask(t.on, t.layer).ok().flatten();
    let progress = Arc::new(AtomicU32::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let threads = &mut state.sticker_ui.cut_threads;
    let alive = Arc::clone(&threads.alive);
    let now = alive.fetch_add(1, Ordering::AcqRel) + 1;
    threads.most_alive = threads.most_alive.max(now);
    threads.started += 1;
    let guard = AliveGuard { alive, ctx: ctx.clone() };
    #[cfg(test)]
    let hold = Arc::clone(&threads.hold);
    let (p2, c2, pix2, mask2, size) = (Arc::clone(&progress), Arc::clone(&cancel), Arc::clone(&pixels), mask.clone(), t.size);
    let spawned = std::thread::Builder::new().name("Sticker cut out box".to_owned()).spawn(move || {
        let guard = guard;
        let tick = |pc: u32| {
            p2.store(pc, Ordering::Release);
            guard.ctx.request_repaint();
            !c2.load(Ordering::Acquire)
        };
        let result = wte_core::cut_out_box(&pix2, rect, &tick)
            .map(|found| filters::apply_cut(&found, &pix2, mask2.as_deref(), size, selection.as_deref(), into_mask));
        #[cfg(test)]
        while hold.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // The guard's drop (after the send) asks for the frame that takes the result.
        let _ = tx.send(result);
    });
    match spawned {
        Ok(_) => {
            state.status_hint = note(0);
            state.sticker_ui.cut_job = Some(CutJob { t, pixels, mask, progress, cancel, rx, ready: None });
        }
        Err(e) => {
            log::warn!("the cut-out cannot start: {e}");
            state.status_hint = format!("{CUT_BOX_LABEL} could not start");
        }
    }
}

/// Esc / Cancel: stops the running cut (and a box waiting to start). True when there was one.
pub(crate) fn cancel(state: &mut AppState) -> bool {
    let queued = state.sticker_ui.cut_threads.queued.take().is_some();
    let job = state.sticker_ui.cut_job.take();
    if let Some(job) = &job {
        job.cancel.store(true, Ordering::Release);
    }
    let any = queued || job.is_some();
    if any {
        state.status_hint = BoxCutRefusal::Cancelled.message().to_owned();
    }
    any
}

/// Leaving the view: the cut stops without a word.
pub(crate) fn clear(state: &mut AppState) {
    state.sticker_ui.cut_threads.queued = None;
    if let Some(job) = state.sticker_ui.cut_job.take() {
        job.cancel.store(true, Ordering::Release);
    }
}

fn say(state: &mut AppState, text: &str) {
    state.status_hint = text.to_owned();
    state.toasts.info(text);
}

/// Once per frame (with the canvas): starts a box that waited for an earlier thread, keeps the progress note, and
/// puts a finished cut in as one entry (or drops it and says why).
pub(crate) fn frame(ctx: &eframe::egui::Context, state: &mut AppState) {
    if state.sticker_ui.cut_job.is_none() {
        after_dispatch(ctx, state);
        return;
    }
    // The editor was left or shows another sticker: the cut is not wanted.
    let target_now = edit_of(state).and_then(edit_ref);
    if state.sticker_ui.cut_job.as_ref().is_some_and(|j| Some(j.t.on) != target_now) {
        clear(state);
        return;
    }
    let Some(job) = state.sticker_ui.cut_job.as_mut() else { return };
    if job.ready.is_none() {
        match job.rx.try_recv() {
            Ok(r) => job.ready = Some(r),
            Err(mpsc::TryRecvError::Empty) => {
                let pc = job.progress.load(Ordering::Acquire).min(100);
                state.status_hint = note(pc);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                state.sticker_ui.cut_job = None;
                say(state, &format!("{CUT_BOX_LABEL} stopped with an error"));
                return;
            }
        }
    }
    // A drag or a stroke ends first (the result is one entry of its own).
    if state.sticker_ui.drag.is_some() || state.sticker_ui.live.is_some() {
        return;
    }
    let Some(job) = state.sticker_ui.cut_job.take() else { return };
    let Some(result) = job.ready else { return };
    let change = match result {
        Ok(c) => c,
        Err(BoxCutRefusal::Cancelled) => return,
        Err(refusal) => {
            say(state, refusal.message());
            return;
        }
    };
    // The layer must still hold what the cut was made from.
    let Some(doc) = state.doc_mut() else { return };
    let same_pixels = doc.sticker_layer_pixels(job.t.on, job.t.layer).is_ok_and(|p| Arc::ptr_eq(&p, &job.pixels) || *p == *job.pixels);
    let same_mask = doc.sticker_layer_mask(job.t.on, job.t.layer).is_ok_and(|m| match (&m, &job.mask) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || **a == **b,
        _ => false,
    });
    if !(same_pixels && same_mask) {
        say(state, CHANGED_NOTE);
        return;
    }
    let mut t = job.t;
    match change {
        Change::Data(data) => {
            t.target = data.target();
            super::put(state, &t, data, CUT_BOX_LABEL);
        }
        Change::NewMask(mask) => {
            super::put_new_mask(state, &t, mask, CUT_BOX_LABEL);
        }
    }
}
