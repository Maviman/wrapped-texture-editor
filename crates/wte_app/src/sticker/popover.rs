// Owner: stickers P7b (STICKER_PLAN 3.3 "Commands"; ARCHITECTURE 8.15). The small window of a Sticker command that
// takes numbers: Color to Alpha... (the key colour: White / Black / Primary, the colour button, or a click on the canvas
// picks it), Defringe... (width), Blur... (radius), Sharpen... (amount, radius), Color Range... (colour, tolerance, all
// layers, mode) and Grow... / Shrink... / Feather... (pixels). It floats over the canvas's top-left corner, is not
// modal (the canvas still pans and zooms; the 3D half still navigates), and shows a live preview while it is open:
// the canvas draws the layer as Apply would leave it (`filters::compute` + `Document::sticker_picture_with`, the
// document unchanged), or the selection's outline as it would become. Apply runs the command (one entry, or a new
// selection); Cancel, Esc, another active layer or leaving the view close it with no change.
//
// The preview is computed when what it shows changes (the numbers, the layer's pixels, the selection, "Paint on
// mask") and at most every `PREVIEW_EVERY` while a slider is dragged. P7b check: a picture preview (the command's maths
// and the styled render of the sticker) runs on a job thread, one at a time; the canvas keeps the last preview until
// the next one lands (on the UI thread a 2048-px styled sticker took ~255 ms per update). The selection previews
// stay on the UI thread (Color Range is the wand's work, Grow / Shrink a distance transform: within 8.4).
//
// Stickers P8: the whole-sticker commands with numbers have a popover too (Trim... padding, Add Padding... pixels,
// Canvas Size... width / height / anchor, Resize... percent or pixels with Keep aspect, Rotate... degrees); they have
// no preview (the artboard outline does not change until Apply).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Rect, vec2};
use glam::Vec2;
use wte_core::{PixelBuf, SelectMode, Selection, StickerLayerId};

use crate::state::{AppState, PanelHit};
use crate::sticker::commands::Modify;
use crate::sticker::filters::{self, Change, CutOut};
use crate::sticker::layout::Layout;
use crate::sticker::{Shown, StickerAction, edit_of, edit_ref, shown};

/// The shortest time between two previews while the pointer is held (a slider drag).
pub(crate) const PREVIEW_EVERY: Duration = Duration::from_millis(100);

/// What the popover sets.
#[derive(Clone, Debug, PartialEq)]
pub enum PopKind {
    ColorToAlpha { key: [u8; 3] },
    Defringe { width: u32 },
    Blur { radius: f32 },
    /// Amount 0..=3 (shown as 0..=300 %).
    Sharpen { amount: f32, radius: f32 },
    ColorRange { color: [u8; 4], tolerance: u8, all_layers: bool, mode: SelectMode },
    Modify { how: Modify, px: f32 },
    /// Stickers P8 (0 sizes are filled with the artboard's when the popover opens).
    Trim { padding: u32 },
    Padding { px: u32 },
    /// The anchor: 0, 1, 2 per axis (left / centre / right, top / middle / bottom).
    CanvasSize { w: u32, h: u32, anchor: [u8; 2] },
    Resize { w: u32, h: u32, keep: bool },
    Rotate { deg: f32 },
}

impl PopKind {
    /// The window's title.
    pub(crate) fn title(&self) -> &'static str {
        match self {
            PopKind::ColorToAlpha { .. } => "Color to Alpha",
            PopKind::Defringe { .. } => "Defringe",
            PopKind::Blur { .. } => "Blur",
            PopKind::Sharpen { .. } => "Sharpen",
            PopKind::ColorRange { .. } => "Color Range",
            PopKind::Modify { how: Modify::Grow, .. } => "Grow Selection",
            PopKind::Modify { how: Modify::Shrink, .. } => "Shrink Selection",
            PopKind::Modify { how: Modify::Feather, .. } => "Feather Selection",
            PopKind::Trim { .. } => "Trim",
            PopKind::Padding { .. } => "Add Padding",
            PopKind::CanvasSize { .. } => "Canvas Size",
            PopKind::Resize { .. } => "Resize",
            PopKind::Rotate { .. } => "Rotate",
        }
    }

    /// The command Apply runs.
    pub(crate) fn action(&self) -> StickerAction {
        match *self {
            PopKind::ColorToAlpha { key } => StickerAction::ColorToAlpha(key),
            PopKind::Defringe { width } => StickerAction::Defringe(width),
            PopKind::Blur { radius } => StickerAction::Blur(radius),
            PopKind::Sharpen { amount, radius } => StickerAction::Sharpen(amount, radius),
            PopKind::ColorRange { color, tolerance, all_layers, mode } => StickerAction::ColorRange { color, tolerance, all_layers, mode },
            PopKind::Modify { how: Modify::Grow, px } => StickerAction::GrowSelection(px),
            PopKind::Modify { how: Modify::Shrink, px } => StickerAction::ShrinkSelection(px),
            PopKind::Modify { how: Modify::Feather, px } => StickerAction::FeatherSelection(px),
            PopKind::Trim { padding } => StickerAction::Layout(Layout::Trim(padding)),
            PopKind::Padding { px } => StickerAction::Layout(Layout::Padding(px)),
            PopKind::CanvasSize { w, h, anchor } => {
                StickerAction::Layout(Layout::CanvasSize([w, h], [anchor[0].min(2) as f32 * 0.5, anchor[1].min(2) as f32 * 0.5]))
            }
            PopKind::Resize { w, h, .. } => StickerAction::Layout(Layout::Resize([w, h])),
            PopKind::Rotate { deg } => StickerAction::Layout(Layout::Rotate(deg)),
        }
    }

    /// The pixel command it previews (`None`: a selection command).
    fn cut(&self) -> Option<CutOut> {
        match *self {
            PopKind::ColorToAlpha { key } => Some(CutOut::ColorToAlpha(key)),
            PopKind::Defringe { width } => Some(CutOut::Defringe(width)),
            PopKind::Blur { radius } => Some(CutOut::Blur(radius)),
            PopKind::Sharpen { amount, radius } => Some(CutOut::Sharpen(amount, radius)),
            _ => None,
        }
    }

    /// A click on the canvas picks its colour.
    pub(crate) fn picks(&self) -> bool {
        matches!(self, PopKind::ColorToAlpha { .. } | PopKind::ColorRange { .. })
    }
}

/// What the preview was made from (computed again when this changes).
#[derive(Clone, Debug, PartialEq)]
struct PreviewKey {
    kind: PopKind,
    layer: Option<StickerLayerId>,
    /// The shown sticker's `Arc` (its address) or the image stamp's blob id.
    source: usize,
    selection: usize,
    paint_on_mask: bool,
}

/// The open popover (`StickerUi::popover`).
#[derive(Clone, Debug)]
pub(crate) struct Popover {
    pub(crate) kind: PopKind,
    /// The active layer it was opened on.
    pub(crate) layer: Option<StickerLayerId>,
    previewed: Option<PreviewKey>,
    last: Option<Instant>,
    /// The canvas texture shows the preview picture.
    pub(crate) shows_picture: bool,
    /// The selection as Apply would make it (Color Range, Grow / Shrink / Feather).
    pub(crate) selection: Option<Arc<Selection>>,
    /// Previews computed (tests).
    pub(crate) previews: u64,
}

/// Opens the popover of `kind` on the active layer (a picture layer for the pixel commands, a selection for Grow /
/// Shrink / Feather; else the status line says why).
pub(crate) fn open(state: &mut AppState, kind: PopKind) {
    let Some(edit) = edit_of(state).cloned() else { return };
    if kind.cut().is_some() && crate::sticker::tools::paint_target(state, crate::state::StickerTool::Brush, false).is_none() {
        return;
    }
    if matches!(kind, PopKind::Modify { .. }) && edit.selection.is_none() {
        state.status_hint = crate::sticker::commands::NO_SELECTION_HINT.to_owned();
        return;
    }
    let Some((_, size)) = crate::sticker::tools::artboard_of(state) else { return };
    // Stickers P8: the size fields start at the artboard's size.
    let kind = match kind {
        PopKind::CanvasSize { w: 0, h: 0, anchor } => PopKind::CanvasSize { w: size[0], h: size[1], anchor },
        PopKind::Resize { w: 0, h: 0, keep } => PopKind::Resize { w: size[0], h: size[1], keep },
        other => other,
    };
    state.sticker_ui.popover =
        Some(Popover { kind, layer: edit.active_layer, previewed: None, last: None, shows_picture: false, selection: None, previews: 0 });
}

/// Apply: closes the popover and runs its command.
pub(crate) fn apply(state: &mut AppState) {
    let Some(p) = state.sticker_ui.popover.take() else { return };
    crate::sticker::run_in(state, p.kind.action());
}

/// A click on the canvas while a picking popover is open: its colour becomes the key (layer-space point `p`). True when
/// the click was taken.
pub(crate) fn pick(state: &mut AppState, p: Vec2) -> bool {
    if !state.sticker_ui.popover.as_ref().is_some_and(|p| p.kind.picks()) {
        return false;
    }
    let Some(on) = edit_of(state).and_then(edit_ref) else { return true };
    let color = state.doc_mut().and_then(|d| d.sticker_color_at(on, p));
    let Some(pop) = state.sticker_ui.popover.as_mut() else { return true };
    match (color, &mut pop.kind) {
        (Some(c), PopKind::ColorToAlpha { key }) if c[3] > 0 => *key = [c[0], c[1], c[2]],
        (Some(c), PopKind::ColorRange { color, .. }) if c[3] > 0 => *color = c,
        _ => state.status_hint = crate::sticker::tools::eyedropper::NOTHING_HINT.to_owned(),
    }
    true
}

/// The key of what the preview would show now.
fn key_now(state: &mut AppState, pop: &Popover) -> Option<PreviewKey> {
    let edit = edit_of(state)?.clone();
    let selection = edit.selection.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize);
    let session = state.session.as_mut()?;
    let source = match shown(&mut session.doc, &edit)? {
        Shown::Sticker { doc, .. } => Arc::as_ptr(&doc) as usize,
        Shown::Image { blob, .. } => blob.0 as usize,
        Shown::Text { .. } => return None,
    };
    Some(PreviewKey { kind: pop.kind.clone(), layer: pop.layer, source, selection, paint_on_mask: state.sticker_ui.paint_on_mask })
}

/// P7b check: the job drawing a picture preview off the UI thread (`StickerUi::popover_job`): the key it was started
/// for and the channel of its picture (`None`: the command found nothing to do).
pub(crate) struct PreviewJob {
    key: PreviewKey,
    rx: mpsc::Receiver<Option<PixelBuf>>,
}

/// Counts a preview thread while it runs; at its end (a panic included) the count drops and a frame is asked for.
struct AliveGuard {
    alive: Arc<AtomicU64>,
    ctx: egui::Context,
}

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::AcqRel);
        self.ctx.request_repaint();
    }
}

/// True while the preview of the open popover is not shown yet (the self-test's idle check waits for it).
pub(crate) fn pending(state: &AppState) -> bool {
    state.sticker_ui.popover_job.is_some() || state.sticker_ui.popover.as_ref().is_some_and(|p| p.previewed.is_none())
}

/// Once per frame before the canvas is drawn: takes a finished preview job's picture into the canvas texture (when it
/// is still what the popover shows), and starts the next job when the preview is out of date (one job at a time, not
/// more often than every `PREVIEW_EVERY` while a button is held). P7b check: a picture preview (the command's maths
/// and the styled render, ~255 ms for a 2048-px styled sticker) runs on a job thread; the UI thread only gathers its
/// inputs. The selection previews (Color Range, Grow / Shrink / Feather: within the wand's budget of 8.4) stay here.
pub(crate) fn frame(ctx: &egui::Context, state: &mut AppState) {
    let Some(pop) = state.sticker_ui.popover.clone() else {
        state.sticker_ui.popover_job = None;
        return;
    };
    let _p = crate::app::frame_profile::scope("sticker.popover");
    let Some(key) = key_now(state, &pop) else { return };
    if let Some(job) = state.sticker_ui.popover_job.as_ref() {
        match job.rx.try_recv() {
            Ok(picture) => {
                let job = state.sticker_ui.popover_job.take();
                if job.is_some_and(|j| j.key == key) {
                    show_picture(ctx, state, picture);
                }
            }
            // Still drawing: its end asks for a frame.
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                state.sticker_ui.popover_job = None;
                show_picture(ctx, state, None);
            }
        }
    }
    let Some(pop) = state.sticker_ui.popover.clone() else { return };
    if pop.previewed.as_ref() == Some(&key) {
        return;
    }
    let held = ctx.input(|i| i.pointer.any_down());
    if held && pop.last.is_some_and(|t| t.elapsed() < PREVIEW_EVERY) {
        ctx.request_repaint_after(PREVIEW_EVERY);
        return;
    }
    let selection = match pop.kind.cut() {
        Some(cut) => {
            // A dropped job (of an earlier key or a closed popover) still running finishes first.
            if state.sticker_ui.popover_alive.load(Ordering::Acquire) > 0 {
                return;
            }
            start_job(ctx, state, cut, key.clone());
            None
        }
        None => {
            let selection = preview_selection(state, &pop.kind);
            if pop.shows_picture {
                show_picture(ctx, state, None);
            }
            selection
        }
    };
    if let Some(p) = state.sticker_ui.popover.as_mut() {
        p.previewed = Some(key);
        p.last = Some(Instant::now());
        p.selection = selection;
        p.previews += 1;
    }
}

/// Starts the job of a picture preview for `key`: the inputs are gathered here (`filters::prepare`,
/// `Document::sticker_picture_with_job`: `Arc`s), the maths and the render run on the thread.
fn start_job(ctx: &egui::Context, state: &mut AppState, cut: CutOut, key: PreviewKey) {
    let Some(prepared) = filters::prepare(state, cut) else {
        show_picture(ctx, state, None);
        return;
    };
    let (on, layer) = (prepared.t.on, prepared.t.layer);
    let Some(draw) = state.doc_mut().and_then(|d| d.sticker_picture_with_job(on, layer).ok()) else {
        show_picture(ctx, state, None);
        return;
    };
    let (tx, rx) = mpsc::channel();
    let alive = Arc::clone(&state.sticker_ui.popover_alive);
    alive.fetch_add(1, Ordering::AcqRel);
    let guard = AliveGuard { alive, ctx: ctx.clone() };
    #[cfg(test)]
    let hold = Arc::clone(&state.sticker_ui.jobs.hold);
    let spawned = std::thread::Builder::new().name("Sticker popover preview".to_owned()).spawn(move || {
        let _guard = guard;
        let picture = match prepared.math() {
            Ok(Change::Data(data)) => draw.run(data).ok(),
            _ => None,
        };
        #[cfg(test)]
        while hold.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        // The guard's drop (after the send) asks for the frame that takes the picture.
        let _ = tx.send(picture);
    });
    match spawned {
        Ok(_) => state.sticker_ui.popover_job = Some(PreviewJob { key, rx }),
        Err(e) => {
            // The closure, and the guard in it, went with the error: the count is back.
            log::warn!("the popover preview cannot start: {e}");
            show_picture(ctx, state, None);
        }
    }
}

/// Puts a preview picture into the canvas texture (`None`: the canvas draws the sticker again).
fn show_picture(ctx: &egui::Context, state: &mut AppState, picture: Option<PixelBuf>) {
    let shown_before = state.sticker_ui.popover.as_ref().is_some_and(|p| p.shows_picture);
    if let Some(p) = &picture {
        let size = [p.width, p.height];
        crate::sticker::live::upload(ctx, state, &wte_core::IRect::from_size(size[0], size[1]), &p.data, size, true);
        if let Some(c) = state.sticker_ui.canvas.as_mut() {
            c.key = crate::sticker::canvas::CanvasKey::Stale;
            c.exact = false;
        }
    } else if shown_before && let Some(c) = state.sticker_ui.canvas.as_mut() {
        // The preview cannot be drawn any more: the canvas draws the sticker again.
        c.key = crate::sticker::canvas::CanvasKey::Stale;
    }
    if let Some(p) = state.sticker_ui.popover.as_mut() {
        p.shows_picture = picture.is_some();
    }
}

/// The selection Color Range or Grow / Shrink / Feather would make.
fn preview_selection(state: &mut AppState, kind: &PopKind) -> Option<Arc<Selection>> {
    let current = edit_of(state)?.selection.clone();
    match *kind {
        PopKind::ColorRange { color, tolerance, all_layers, mode } => {
            let mask = crate::sticker::tools::wand::color_range_mask(state, color, tolerance, all_layers)?;
            let size = [mask.width, mask.height];
            let result = match (mode, current.filter(|c| c.size() == size)) {
                (SelectMode::Replace, _) | (SelectMode::Add, None) => Selection::from_mask(mask),
                (SelectMode::Subtract | SelectMode::Intersect, None) => Selection::none(size[0], size[1]),
                (mode, Some(cur)) => {
                    let mut s = (*cur).clone();
                    s.combine(&mask, mode);
                    s
                }
            };
            Some(Arc::new(result))
        }
        PopKind::Modify { how, px } => {
            let mut s = (*current?).clone();
            crate::sticker::commands::modified(&mut s, how, px);
            Some(Arc::new(s))
        }
        _ => None,
    }
}

/// Draws the open popover at the canvas's top-left corner; its buttons push `StickerAction`s.
pub(crate) fn show(ui: &egui::Ui, state: &mut AppState, canvas: Rect) {
    let Some(mut kind) = state.sticker_ui.popover.as_ref().map(|p| p.kind.clone()) else { return };
    let primary = state.colors.primary;
    let mut actions = Vec::new();
    let mut hits = Vec::new();
    let window = egui::Window::new(kind.title())
        .id(egui::Id::unique("sticker_popover"))
        .fixed_pos(canvas.min + vec2(10.0, 10.0))
        .collapsible(false)
        .resizable(false)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(260.0);
            match &mut kind {
                PopKind::ColorToAlpha { key } => {
                    key_row(ui, key, primary, &mut hits);
                    ui.weak("Or click the canvas to pick the colour.");
                }
                PopKind::Defringe { width } => {
                    ui.add(egui::Slider::new(width, 1..=wte_core::sticker::cutout::MAX_DEFRINGE).text("Width").suffix(" px"));
                }
                PopKind::Blur { radius } => {
                    ui.add(egui::Slider::new(radius, 0.5..=wte_core::sticker::filter::MAX_BLUR_RADIUS).logarithmic(true).text("Radius").suffix(" px"));
                }
                PopKind::Sharpen { amount, radius } => {
                    let mut percent = *amount * 100.0;
                    if ui.add(egui::Slider::new(&mut percent, 0.0..=300.0).text("Amount").suffix(" %").max_decimals(0)).changed() {
                        *amount = percent / 100.0;
                    }
                    ui.add(egui::Slider::new(radius, 0.5..=wte_core::sticker::filter::MAX_SHARPEN_RADIUS).text("Radius").suffix(" px"));
                }
                PopKind::ColorRange { color, tolerance, all_layers, mode } => {
                    let mut rgb = [color[0], color[1], color[2]];
                    key_row(ui, &mut rgb, primary, &mut hits);
                    *color = [rgb[0], rgb[1], rgb[2], 255];
                    ui.weak("Or click the canvas to pick the colour.");
                    ui.add(egui::Slider::new(tolerance, 0..=255).text("Tolerance"));
                    ui.checkbox(all_layers, "All layers").on_hover_text("Look at every visible layer (else the active layer)");
                    mode_row(ui, mode, &mut hits);
                }
                PopKind::Modify { px, .. } => {
                    ui.add(egui::Slider::new(px, 0.0..=wte_core::sticker::select::MAX_MODIFY).logarithmic(true).text("Pixels").max_decimals(1));
                }
                // Stickers P8.
                PopKind::Trim { padding } => {
                    ui.add(egui::DragValue::new(padding).range(0..=wte_core::sticker::layout::MAX_PADDING).suffix(" px").prefix("Padding "));
                    ui.weak("The artboard becomes the bounds of what is visible.");
                }
                PopKind::Padding { px } => {
                    ui.add(egui::DragValue::new(px).range(0..=wte_core::sticker::layout::MAX_PADDING).suffix(" px").prefix("Every side "));
                }
                PopKind::CanvasSize { w, h, anchor } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(w).range(1..=wte_core::MAX_ARTBOARD).suffix(" px").prefix("W "));
                        ui.add(egui::DragValue::new(h).range(1..=wte_core::MAX_ARTBOARD).suffix(" px").prefix("H "));
                    });
                    ui.label("Anchor (the part that stays put)");
                    egui::Grid::new("sticker_canvas_anchor").spacing([2.0, 2.0]).show(ui, |ui| {
                        for y in 0..3u8 {
                            for x in 0..3u8 {
                                let on = *anchor == [x, y];
                                if ui.add(egui::Button::selectable(on, if on { "o" } else { " " }).min_size(vec2(24.0, 24.0))).clicked() {
                                    *anchor = [x, y];
                                }
                            }
                            ui.end_row();
                        }
                    });
                }
                PopKind::Resize { w, h, keep } => {
                    let (w0, h0) = (*w, *h);
                    let base = crate::sticker::tools::artboard_of(state).map_or([w0.max(1), h0.max(1)], |(_, s)| s);
                    let mut pct = *w as f32 / base[0].max(1) as f32 * 100.0;
                    let rp = ui.add(egui::DragValue::new(&mut pct).range(1.0..=10_000.0).speed(1.0).max_decimals(1).suffix(" %"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(w).range(1..=wte_core::MAX_ARTBOARD).suffix(" px").prefix("W "));
                        ui.add(egui::DragValue::new(h).range(1..=wte_core::MAX_ARTBOARD).suffix(" px").prefix("H "));
                    });
                    ui.checkbox(keep, "Keep aspect");
                    if rp.changed() {
                        *w = ((base[0] as f32 * pct / 100.0).round() as u32).clamp(1, wte_core::MAX_ARTBOARD);
                        *h = ((base[1] as f32 * pct / 100.0).round() as u32).clamp(1, wte_core::MAX_ARTBOARD);
                    } else if *keep && *w != w0 {
                        *h = ((*w as f32 * base[1] as f32 / base[0].max(1) as f32).round() as u32).clamp(1, wte_core::MAX_ARTBOARD);
                    } else if *keep && *h != h0 {
                        *w = ((*h as f32 * base[0] as f32 / base[1].max(1) as f32).round() as u32).clamp(1, wte_core::MAX_ARTBOARD);
                    }
                    ui.weak("No picture is resampled; the sticker keeps its size on the model.");
                }
                PopKind::Rotate { deg } => {
                    ui.add(egui::DragValue::new(deg).range(-360.0..=360.0).speed(0.5).max_decimals(1).suffix(" deg").prefix("Clockwise "));
                }
            }
            ui.horizontal(|ui| {
                let apply = ui.button("Apply");
                hits.push((PanelHit::StickerPopoverApply, apply.rect));
                if apply.clicked() {
                    actions.push(StickerAction::ApplyPopover);
                }
                let cancel = ui.button("Cancel").on_hover_text("Close without a change (Esc)");
                hits.push((PanelHit::StickerPopoverCancel, cancel.rect));
                if cancel.clicked() {
                    actions.push(StickerAction::ClosePopover);
                }
            });
        });
    if let Some(w) = window {
        hits.push((PanelHit::StickerPopover, w.response.rect));
    }
    if let Some(p) = state.sticker_ui.popover.as_mut() {
        p.kind = kind;
    }
    state.panel_rects.extend(hits);
    state.actions.extend(actions.into_iter().map(crate::action::AppAction::Sticker));
}

/// The key colour: its button, and White / Black / Primary.
fn key_row(ui: &mut egui::Ui, key: &mut [u8; 3], primary: [u8; 4], hits: &mut Vec<(PanelHit, Rect)>) {
    ui.horizontal(|ui| {
        ui.label("Color");
        ui.color_edit_button_srgb(key);
        let choices = [("White", [255, 255, 255]), ("Black", [0, 0, 0]), ("Primary", [primary[0], primary[1], primary[2]])];
        for (i, (label, c)) in choices.into_iter().enumerate() {
            let r = ui.small_button(label);
            hits.push((PanelHit::StickerPopoverKey(i as u8), r.rect));
            if r.clicked() {
                *key = c;
            }
        }
    });
}

/// The mode a new selection combines with (New, Add, Subtract, Intersect).
pub(crate) fn mode_row(ui: &mut egui::Ui, mode: &mut SelectMode, hits: &mut Vec<(PanelHit, Rect)>) {
    ui.horizontal_wrapped(|ui| {
        for (i, (m, label, hover)) in MODES.into_iter().enumerate() {
            let r = ui.selectable_label(*mode == m, label).on_hover_text(hover);
            hits.push((PanelHit::StickerSelectMode(i as u8), r.rect));
            if r.clicked() {
                *mode = m;
            }
        }
    });
}

/// The selection modes with their labels (the Tool tab's row and the Color Range popover).
pub(crate) const MODES: [(SelectMode, &str, &str); 4] = [
    (SelectMode::Replace, "New", "A new selection replaces the old one"),
    (SelectMode::Add, "Add", "Add to the selection (or hold Shift)"),
    (SelectMode::Subtract, "Subtract", "Take away from the selection (or hold Alt)"),
    (SelectMode::Intersect, "Intersect", "Keep only what both select (or hold Shift and Alt)"),
];

/// Tests: the preview picture of the open popover, as the canvas texture holds it.
#[cfg(test)]
pub(crate) fn previewed_data(state: &mut AppState) -> Option<wte_core::PixelData> {
    let kind = state.sticker_ui.popover.as_ref()?.kind.cut()?;
    match filters::compute(state, kind)? {
        (_, Change::Data(d)) => Some(d),
        _ => None,
    }
}
