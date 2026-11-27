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
// mask") and at most every `PREVIEW_EVERY` while a slider is dragged; it runs on the UI thread (the budgets of 8.4:
// blur radius 16 at 2048 x 2048 is about 50 ms).

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Rect, vec2};
use glam::Vec2;
use wte_core::{SelectMode, Selection, StickerLayerId};

use crate::state::{AppState, PanelHit};
use crate::sticker::commands::Modify;
use crate::sticker::filters::{self, Change, CutOut};
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
        }
    }

    /// The pixel command it previews (`None`: a selection command).
    fn cut(&self) -> Option<CutOut> {
        match *self {
            PopKind::ColorToAlpha { key } => Some(CutOut::ColorToAlpha(key)),
            PopKind::Defringe { width } => Some(CutOut::Defringe(width)),
            PopKind::Blur { radius } => Some(CutOut::Blur(radius)),
            PopKind::Sharpen { amount, radius } => Some(CutOut::Sharpen(amount, radius)),
            PopKind::ColorRange { .. } | PopKind::Modify { .. } => None,
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
    if crate::sticker::tools::artboard_of(state).is_none() {
        return;
    }
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

/// Once per frame before the canvas is drawn: computes the preview when it is out of date (not more often than every
/// `PREVIEW_EVERY` while a button is held) and puts a preview picture into the canvas texture.
pub(crate) fn frame(ctx: &egui::Context, state: &mut AppState) {
    let Some(pop) = state.sticker_ui.popover.clone() else { return };
    let _p = crate::app::frame_profile::scope("sticker.popover");
    let Some(key) = key_now(state, &pop) else { return };
    if pop.previewed.as_ref() == Some(&key) {
        return;
    }
    let held = ctx.input(|i| i.pointer.any_down());
    if held && pop.last.is_some_and(|t| t.elapsed() < PREVIEW_EVERY) {
        ctx.request_repaint_after(PREVIEW_EVERY);
        return;
    }
    let mut picture = None;
    let mut selection = None;
    match pop.kind.cut() {
        Some(cut) => {
            if let Some((t, Change::Data(data))) = filters::compute(state, cut) {
                picture = state.doc_mut().and_then(|d| d.sticker_picture_with(t.on, t.layer, data).ok());
            }
        }
        None => selection = preview_selection(state, &pop.kind),
    }
    if let Some(p) = &picture {
        let size = [p.width, p.height];
        crate::sticker::live::upload(ctx, state, &wte_core::IRect::from_size(size[0], size[1]), &p.data, size, true);
        if let Some(c) = state.sticker_ui.canvas.as_mut() {
            c.key = crate::sticker::canvas::CanvasKey::Stale;
            c.exact = false;
        }
    } else if pop.shows_picture
        && let Some(c) = state.sticker_ui.canvas.as_mut()
    {
        // The preview cannot be drawn any more: the canvas draws the sticker again.
        c.key = crate::sticker::canvas::CanvasKey::Stale;
    }
    if let Some(p) = state.sticker_ui.popover.as_mut() {
        p.previewed = Some(key);
        p.last = Some(Instant::now());
        p.shows_picture = picture.is_some();
        p.selection = selection;
        p.previews += 1;
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
