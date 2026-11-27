// Owner: stickers P7a (STICKER_PLAN 3.5, review A6, 8.4; ARCHITECTURE 8.15). The sticker canvas during and right after
// a brush or eraser stroke, and the render jobs that keep the canvas and the model up to date:
//
// - **Dirty rectangles.** Every dab reports the content pixels it changed (`note_dab`); `frame` draws only that
//   rectangle again (`Document::sticker_canvas_region`: the other layers from the layer cache, the stroke's layer
//   unstyled) and uploads it with `TextureHandle::set_partial`. A layer without an enabled style is then exactly the
//   finished picture (no full recomposite during or after the stroke).
// - **Stroke mode.** A layer with an enabled style (contour, stroke, shadow, glow: they need the whole alpha) is shown
//   unstyled while the stroke runs: one full recomposite when the stroke starts, dirty rectangles after it, and the
//   header says "styles update when you lift". At the stroke's end the canvas's restyle runs on a job
//   (`Document::sticker_canvas_job`); the unstyled picture stays until the restyled one arrives (the stale styled
//   picture is never shown in between).
// - **The model.** During a stroke at most every 100 ms (`PREVIEW_EVERY`) a job renders the sticker with its styles at
//   Preview scale (`sticker_preview_job`); the result is baked as a Preview (`apply_sticker_preview`). A job still
//   running is not started twice. The stroke's end bakes the Final (the core: deferred to the bake jobs).
//
// Jobs are plain named threads with a channel (like `stamp_bakes::warm_up`): they render a snapshot of the sticker and
// change nothing; a result that no longer fits (another stroke, an undo, a closed view) is dropped.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use wte_core::{IRect, StickerId, StickerLayerId, StickerRenderJob, StickerRenderOutput};

use crate::state::AppState;
use crate::sticker::CanvasTex;
use crate::sticker::canvas::{CanvasKey, TEXTURE_OPTIONS, sticker_key};

/// The model's live Preview during a stroke starts at most this often (STICKER_PLAN 3.5).
pub(crate) const PREVIEW_EVERY: Duration = Duration::from_millis(100);
/// The header note of stroke mode.
pub(crate) const STROKE_MODE_NOTE: &str = "styles update when you lift";

/// The live canvas of one stroke (`StickerUi::live`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LiveStroke {
    pub(crate) sticker: StickerId,
    pub(crate) layer: StickerLayerId,
    /// The layer has an enabled style: the canvas shows it unstyled until the restyle lands ("stroke mode").
    pub(crate) styled: bool,
    /// Artboard pixels changed since the last upload.
    pub(crate) dirty: IRect,
    /// One full recomposite is due (entering stroke mode, or a texture that did not show the sticker).
    pub(crate) full_due: bool,
    /// The stroke ended (kept); a styled layer waits for its restyle job.
    pub(crate) ended: bool,
    /// A dab changed pixels since the last Preview job started.
    pub(crate) painted: bool,
}

impl LiveStroke {
    /// True while the header shows the stroke-mode note.
    pub(crate) fn stroke_mode(&self) -> bool {
        self.styled
    }
}

/// The render jobs of the editor (`StickerUi::jobs`).
#[derive(Default)]
pub(crate) struct RenderJobs {
    preview: Option<mpsc::Receiver<StickerRenderOutput>>,
    /// The canvas's restyle job and the canvas key of the sticker value it draws.
    canvas: Option<(mpsc::Receiver<StickerRenderOutput>, CanvasKey)>,
    /// When the last Preview job started.
    last_preview: Option<Instant>,
}

impl RenderJobs {
    /// True while a job runs.
    pub(crate) fn running(&self) -> bool {
        self.preview.is_some() || self.canvas.is_some()
    }
}

/// What the live canvas did (tests, the frame log).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveCounters {
    /// Full recomposites in stroke mode (entering it, or a texture that had to be drawn whole).
    pub full_composites: u64,
    /// Dirty rectangles drawn and uploaded.
    pub regions: u64,
    pub previews_started: u64,
    pub previews_applied: u64,
    /// The most Preview jobs that ran at the same time (STICKER_PLAN 3.5: one).
    pub most_previews_running: u64,
    pub restyles_started: u64,
    pub restyles_applied: u64,
    /// Restyle results that no longer fit the sticker (dropped; the canvas draws it again from the document).
    pub restyles_dropped: u64,
    /// Frames in which the header showed the stroke-mode note (the scenario's proof that it was on during a stroke).
    pub note_frames: u64,
}

/// True while the live canvas or one of its jobs has work left (the self-test's idle check waits for it).
pub(crate) fn pending(state: &AppState) -> bool {
    state.sticker_ui.live.is_some() || state.sticker_ui.jobs.running()
}

/// A stroke was opened on `layer` of `sticker` (the layer has an enabled style: `styled`). `exact`: the canvas
/// texture shows that sticker's finished picture as it was before the stroke (else it is drawn whole once).
pub(crate) fn start(state: &mut AppState, sticker: StickerId, layer: StickerLayerId, styled: bool, exact: bool) {
    let ui = &mut state.sticker_ui;
    // A restyle of an earlier stroke would land on top of this one: dropped.
    if ui.jobs.canvas.take().is_some() {
        ui.counters.restyles_dropped += 1;
    }
    let full_due = styled || !exact || ui.canvas.is_none();
    if let Some(c) = ui.canvas.as_mut() {
        c.exact = false;
    }
    ui.live = Some(LiveStroke { sticker, layer, styled, dirty: IRect::EMPTY, full_due, ended: false, painted: false });
}

/// A dab of the open stroke changed `content` (content pixels of the stroke's layer): the matching artboard pixels are
/// drawn again in the next `frame`.
pub(crate) fn note_dab(state: &mut AppState, content: IRect) {
    if content.is_empty() {
        return;
    }
    let Some(live) = state.sticker_ui.live.clone() else { return };
    let Some(doc) = state.doc().and_then(|d| d.sticker(live.sticker)).cloned() else { return };
    let Some(layer) = doc.layer(live.layer) else { return };
    let size = match &layer.content {
        wte_core::StickerContent::Picture(wte_core::PicturePixels::Owned { pixels, .. }) => [pixels.width, pixels.height],
        _ => return,
    };
    let t = layer.transform;
    let origin = glam::Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
    let corners = [(content.x0, content.y0), (content.x1, content.y0), (content.x0, content.y1), (content.x1, content.y1)]
        .map(|(x, y)| t.apply(size, glam::Vec2::new(x as f32, y as f32)) - origin);
    let (mut lo, mut hi) = (corners[0], corners[0]);
    for c in &corners[1..] {
        lo = lo.min(*c);
        hi = hi.max(*c);
    }
    // A whole-pixel move maps the rectangle exactly; the resampler of any other transform reaches one pixel further.
    let whole = t.scale == glam::Vec2::ONE && t.rotation == 0.0 && (t.translate - origin).fract() == glam::Vec2::ZERO;
    let pad = if whole { 0.0 } else { 2.0 };
    let clamp = |v: f32| v.clamp(-1.0e7, 1.0e7) as i32;
    let r = IRect::new(clamp((lo.x - pad).floor()), clamp((lo.y - pad).floor()), clamp((hi.x + pad).ceil()), clamp((hi.y + pad).ceil()));
    let r = r.intersect(&IRect::from_size(doc.size()[0], doc.size()[1]));
    if let Some(live) = state.sticker_ui.live.as_mut() {
        live.dirty = live.dirty.union(&r);
        live.painted = true;
    }
}

/// The stroke ended and was kept.
pub(crate) fn ended(state: &mut AppState) {
    if let Some(live) = state.sticker_ui.live.as_mut() {
        live.ended = true;
    }
}

/// The stroke was taken back (Esc, a command, Undo): the live canvas ends, the canvas draws the document again.
pub(crate) fn cancelled(state: &mut AppState) {
    let ui = &mut state.sticker_ui;
    ui.live = None;
    ui.jobs.preview = None;
    if ui.jobs.canvas.take().is_some() {
        ui.counters.restyles_dropped += 1;
    }
    if let Some(c) = ui.canvas.as_mut() {
        c.exact = false;
        c.key = CanvasKey::Stale;
    }
}

/// Drops the live canvas and its jobs (the view closes, a new document).
pub(crate) fn clear(state: &mut AppState) {
    let ui = &mut state.sticker_ui;
    ui.live = None;
    ui.jobs = RenderJobs::default();
}

/// Once per frame after the canvas's input: uploads the dirty rectangle (or the full recomposite of stroke mode),
/// starts and applies the model's Preview jobs while the stroke is open, and after its end either takes the canvas
/// out of the live mode (a layer without styles) or runs the restyle job and shows its picture when it lands.
pub(crate) fn frame(ctx: &egui::Context, state: &mut AppState) {
    let _p = crate::app::frame_profile::scope("sticker.live");
    let Some(live) = state.sticker_ui.live.clone() else {
        // A job of a canvas that left the live mode (cancelled) has nothing to do any more.
        state.sticker_ui.jobs.preview = None;
        return;
    };
    let Some(doc) = state.doc().and_then(|d| d.sticker(live.sticker)).cloned() else {
        clear(state);
        return;
    };
    let full = IRect::from_size(doc.size()[0], doc.size()[1]);
    let sized = state.sticker_ui.canvas.as_ref().is_some_and(|c| c.size == doc.size());
    let region = if live.full_due || !sized { full } else { live.dirty };
    if !region.is_empty() {
        let _p = crate::app::frame_profile::scope("sticker.region");
        let drawn = state.doc_mut().map(|d| d.sticker_canvas_region(live.sticker, region, Some(live.layer)));
        match drawn {
            Some(Ok(buf)) if !buf.is_empty() => {
                let whole = buf.rect == full;
                upload(ctx, state, &buf.rect, &buf.data, doc.size(), whole);
                if whole {
                    state.sticker_ui.counters.full_composites += 1;
                } else {
                    state.sticker_ui.counters.regions += 1;
                }
            }
            Some(Err(e)) => log::warn!("the sticker canvas cannot draw the stroke: {e}"),
            _ => {}
        }
        if let Some(l) = state.sticker_ui.live.as_mut() {
            l.dirty = IRect::EMPTY;
            l.full_due = false;
        }
    }
    preview(ctx, state, &live);
    if !live.ended {
        return;
    }
    if !live.styled {
        // The texture is the finished picture now: it takes the sticker's key without a redraw.
        let key = sticker_key(live.sticker, &doc);
        if let Some(c) = state.sticker_ui.canvas.as_mut() {
            c.key = key;
            c.exact = true;
        }
        state.sticker_ui.live = None;
        return;
    }
    restyle(ctx, state, &live, &doc);
}

/// The model's Preview while the stroke is open: applies a finished job, starts the next one at most every
/// `PREVIEW_EVERY` (none while one runs).
fn preview(ctx: &egui::Context, state: &mut AppState, live: &LiveStroke) {
    if let Some(rx) = state.sticker_ui.jobs.preview.as_ref() {
        match rx.try_recv() {
            Ok(out) => {
                state.sticker_ui.jobs.preview = None;
                let applied = state.doc_mut().is_some_and(|d| d.apply_sticker_preview(out));
                if applied {
                    state.sticker_ui.counters.previews_applied += 1;
                }
            }
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => state.sticker_ui.jobs.preview = None,
        }
    }
    let open = state.doc().and_then(|d| d.sticker_stroke_target()) == Some((live.sticker, live.layer));
    if live.ended || !open || !state.sticker_ui.live.as_ref().is_some_and(|l| l.painted) {
        return;
    }
    if let Some(t) = state.sticker_ui.jobs.last_preview {
        let since = t.elapsed();
        if since < PREVIEW_EVERY {
            ctx.request_repaint_after(PREVIEW_EVERY - since);
            return;
        }
    }
    let job = match state.doc_mut().map(|d| d.sticker_preview_job(live.sticker)) {
        Some(Ok(Some(job))) => job,
        Some(Err(e)) => {
            log::warn!("the sticker preview cannot start: {e}");
            return;
        }
        _ => return,
    };
    if let Some(rx) = spawn(ctx, "Sticker preview", job) {
        let ui = &mut state.sticker_ui;
        ui.jobs.preview = Some(rx);
        ui.jobs.last_preview = Some(Instant::now());
        ui.counters.previews_started += 1;
        ui.counters.most_previews_running = ui.counters.most_previews_running.max(1);
        if let Some(l) = ui.live.as_mut() {
            l.painted = false;
        }
    }
}

/// After the end of a stroke on a styled layer: starts the canvas's restyle job, and when it lands shows its picture
/// (when the sticker is still the value it drew) and leaves the live mode.
fn restyle(ctx: &egui::Context, state: &mut AppState, live: &LiveStroke, doc: &wte_core::StickerDoc) {
    if state.sticker_ui.jobs.canvas.is_none() {
        let job = match state.doc_mut().map(|d| d.sticker_canvas_job(live.sticker)) {
            Some(Ok(job)) => job,
            _ => {
                cancelled(state);
                return;
            }
        };
        let key = sticker_key(live.sticker, job.doc());
        match spawn(ctx, "Sticker restyle", job) {
            Some(rx) => {
                state.sticker_ui.jobs.canvas = Some((rx, key));
                state.sticker_ui.counters.restyles_started += 1;
            }
            None => cancelled(state),
        }
        return;
    }
    let Some((rx, key)) = state.sticker_ui.jobs.canvas.as_ref() else { return };
    let out = match rx.try_recv() {
        Ok(out) => out,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            cancelled(state);
            return;
        }
    };
    let key = key.clone();
    state.sticker_ui.jobs.canvas = None;
    if key != sticker_key(live.sticker, doc) {
        state.sticker_ui.counters.restyles_dropped += 1;
        cancelled(state);
        return;
    }
    let size = [out.picture.width, out.picture.height];
    upload(ctx, state, &IRect::from_size(size[0], size[1]), &out.picture.data, size, true);
    if let Some(c) = state.sticker_ui.canvas.as_mut() {
        c.key = key;
        c.exact = true;
    }
    state.sticker_ui.counters.restyles_applied += 1;
    state.sticker_ui.live = None;
}

/// Runs `job` on a named thread; the result comes back through the channel and asks for a frame.
fn spawn(ctx: &egui::Context, name: &str, job: StickerRenderJob) -> Option<mpsc::Receiver<StickerRenderOutput>> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name(name.to_owned()).spawn(move || {
        let out = job.run();
        if tx.send(out).is_ok() {
            ctx.request_repaint();
        }
    });
    match spawned {
        Ok(_) => Some(rx),
        Err(e) => {
            log::warn!("{name}: the job cannot start ({e})");
            None
        }
    }
}

/// Puts `data` (straight RGBA8 of `rect`, artboard pixels) into the canvas texture of an artboard of `size`: the
/// whole texture (`whole`, a new texture when there is none or its size differs) or the rectangle (`set_partial`).
pub(crate) fn upload(ctx: &egui::Context, state: &mut AppState, rect: &IRect, data: &[u8], size: [u32; 2], whole: bool) {
    let image = egui::ColorImage::from_rgba_unmultiplied([rect.width() as usize, rect.height() as usize], data);
    let ui = &mut state.sticker_ui;
    #[cfg(test)]
    {
        let mirror = ui.mirror.get_or_insert_with(|| wte_core::PixelBuf::new(size[0], size[1]));
        if [mirror.width, mirror.height] != size {
            *mirror = wte_core::PixelBuf::new(size[0], size[1]);
        }
        let w = rect.width() as usize * 4;
        for y in rect.y0..rect.y1 {
            let d = (y as usize * size[0] as usize + rect.x0 as usize) * 4;
            let s = (y - rect.y0) as usize * w;
            mirror.data[d..d + w].copy_from_slice(&data[s..s + w]);
        }
    }
    match ui.canvas.as_mut() {
        Some(c) if c.size == size && !whole => c.tex.set_partial([rect.x0 as usize, rect.y0 as usize], image, TEXTURE_OPTIONS),
        Some(c) if c.size == size || whole => {
            c.tex.set(image, TEXTURE_OPTIONS);
            c.size = size;
        }
        _ => {
            let tex = ctx.load_texture("sticker-canvas", image, TEXTURE_OPTIONS);
            ui.canvas = Some(CanvasTex { key: CanvasKey::Stale, tex, size, exact: false });
        }
    }
}
