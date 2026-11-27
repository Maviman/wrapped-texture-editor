// Owner: stickers P8 (STICKER_PLAN 3.3 "Crop", E2; ARCHITECTURE 8.15). The Crop tool: a box on the canvas that starts
// as the artboard; a drag on one of its eight handles moves that edge or corner, a drag inside it moves it, a drag
// outside it draws a new box (the aspect presets 1:1, 4:3, 16:9 hold for a new box and the corners). The Tool tab
// sets the artboard shape (Rectangle / Rounded with its radius / Ellipse, E2) and "Delete cropped pixels" (off by
// default). Enter or Apply crop applies it (`Document::crop_sticker`: one entry "Crop Sticker", the placements
// refitted so the artwork stays on the model); Esc or Cancel drops the box. The box is app state (not in the history).

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke};
use glam::Vec2;
use wte_core::{ArtboardShape, IRect};

use crate::state::AppState;
use crate::sticker::canvas::Geom;
use crate::sticker::{StickerAction, edit_of, edit_ref};

/// Grab distance of a crop handle (points).
const GRAB: f32 = 9.0;
/// The smallest box side a drag makes (layer-space pixels).
const MIN_SIDE: f32 = 1.0;

/// The artboard shape the Crop tool gives (E2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CropShape {
    #[default]
    Rectangle,
    Rounded,
    Ellipse,
}

impl CropShape {
    pub(crate) const ALL: [CropShape; 3] = [CropShape::Rectangle, CropShape::Rounded, CropShape::Ellipse];

    pub(crate) fn label(self) -> &'static str {
        match self {
            CropShape::Rectangle => "Rectangle",
            CropShape::Rounded => "Rounded",
            CropShape::Ellipse => "Ellipse",
        }
    }
}

/// The aspect presets of the Crop tool.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CropAspect {
    #[default]
    Free,
    Square,
    FourThree,
    SixteenNine,
}

impl CropAspect {
    pub(crate) const ALL: [CropAspect; 4] = [CropAspect::Free, CropAspect::Square, CropAspect::FourThree, CropAspect::SixteenNine];

    pub(crate) fn label(self) -> &'static str {
        match self {
            CropAspect::Free => "Free",
            CropAspect::Square => "1:1",
            CropAspect::FourThree => "4:3",
            CropAspect::SixteenNine => "16:9",
        }
    }

    /// Width / height, `None` for Free.
    fn ratio(self) -> Option<f32> {
        match self {
            CropAspect::Free => None,
            CropAspect::Square => Some(1.0),
            CropAspect::FourThree => Some(4.0 / 3.0),
            CropAspect::SixteenNine => Some(16.0 / 9.0),
        }
    }
}

/// The Crop tool's options (`StickerUi::crop_opts`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CropOptions {
    pub shape: CropShape,
    /// Corner radius of the Rounded shape (layer-space pixels).
    pub radius: f32,
    pub aspect: CropAspect,
    pub delete_pixels: bool,
}

impl Default for CropOptions {
    fn default() -> CropOptions {
        CropOptions { shape: CropShape::Rectangle, radius: 16.0, aspect: CropAspect::Free, delete_pixels: false }
    }
}

impl CropOptions {
    /// The artboard shape these options give.
    pub(crate) fn artboard_shape(&self) -> ArtboardShape {
        match self.shape {
            CropShape::Rectangle => ArtboardShape::Rect,
            CropShape::Rounded => ArtboardShape::RoundedRect { radius: if self.radius.is_finite() { self.radius.max(0.0) } else { 0.0 } },
            CropShape::Ellipse => ArtboardShape::Ellipse,
        }
    }
}

/// What a crop drag changes.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum CropGrip {
    /// An edge or corner (-1, 0, 1 per axis).
    Edge(i8, i8),
    Move,
    New,
}

/// The open editor's artboard as a box `[x0, y0, x1, y1]` (layer space).
fn artboard_box(state: &mut AppState) -> Option<[f32; 4]> {
    let (origin, size) = crate::sticker::tools::artboard_of(state)?;
    Some([origin.x, origin.y, origin.x + size[0] as f32, origin.y + size[1] as f32])
}

/// The crop box shown: the one being made, else the artboard.
pub(crate) fn current_box(state: &mut AppState) -> Option<[f32; 4]> {
    match state.sticker_ui.crop {
        Some(b) => Some(b),
        None => artboard_box(state),
    }
}

/// The handle positions of box `b` (layer space) with their grips.
fn grips(b: [f32; 4]) -> [(CropGrip, Vec2); 8] {
    let (x0, y0, x1, y1) = (b[0], b[1], b[2], b[3]);
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    [
        (CropGrip::Edge(-1, -1), Vec2::new(x0, y0)),
        (CropGrip::Edge(0, -1), Vec2::new(cx, y0)),
        (CropGrip::Edge(1, -1), Vec2::new(x1, y0)),
        (CropGrip::Edge(1, 0), Vec2::new(x1, cy)),
        (CropGrip::Edge(1, 1), Vec2::new(x1, y1)),
        (CropGrip::Edge(0, 1), Vec2::new(cx, y1)),
        (CropGrip::Edge(-1, 1), Vec2::new(x0, y1)),
        (CropGrip::Edge(-1, 0), Vec2::new(x0, cy)),
    ]
}

/// What a press at the screen point `pos` grabs.
pub(crate) fn grip_at(state: &mut AppState, geom: Geom, rect: Rect, pos: Pos2, p: Vec2) -> Option<(CropGrip, [f32; 4])> {
    let cam = edit_of(state)?.camera;
    let b = current_box(state)?;
    let near = grips(b)
        .into_iter()
        .map(|(g, q)| (g, geom.to_screen(&cam, rect, q).distance(pos)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d <= GRAB)
        .map(|(g, _)| g);
    let grip = near.unwrap_or(if p.x > b[0] && p.x < b[2] && p.y > b[1] && p.y < b[3] { CropGrip::Move } else { CropGrip::New });
    Some((grip, b))
}

/// The box a drag of `grip` from `start` to `p` makes from the box `from` (whole pixels, normalised, at least 1 px).
pub(crate) fn dragged(grip: CropGrip, from: [f32; 4], start: Vec2, p: Vec2, aspect: CropAspect) -> [f32; 4] {
    let p = p.round();
    let mut b = match grip {
        CropGrip::New => {
            let s = start.round();
            let mut end = p;
            if let Some(r) = aspect.ratio() {
                let d = end - s;
                let w = d.x.abs().max(d.y.abs() * r);
                end = s + Vec2::new(w.copysign(if d.x == 0.0 { 1.0 } else { d.x }), (w / r).copysign(if d.y == 0.0 { 1.0 } else { d.y }));
                end = end.round();
            }
            [s.x.min(end.x), s.y.min(end.y), s.x.max(end.x), s.y.max(end.y)]
        }
        CropGrip::Move => {
            let d = (p - start.round()).round();
            [from[0] + d.x, from[1] + d.y, from[2] + d.x, from[3] + d.y]
        }
        CropGrip::Edge(hx, hy) => {
            let mut b = from;
            if hx < 0 {
                b[0] = p.x;
            } else if hx > 0 {
                b[2] = p.x;
            }
            if hy < 0 {
                b[1] = p.y;
            } else if hy > 0 {
                b[3] = p.y;
            }
            if let (Some(r), true) = (aspect.ratio(), hx != 0 && hy != 0) {
                let w = (b[2] - b[0]).abs();
                let h = (w / r).round();
                if hy < 0 {
                    b[1] = b[3] - h;
                } else {
                    b[3] = b[1] + h;
                }
            }
            [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])]
        }
    };
    if b[2] - b[0] < MIN_SIDE {
        b[2] = b[0] + MIN_SIDE;
    }
    if b[3] - b[1] < MIN_SIDE {
        b[3] = b[1] + MIN_SIDE;
    }
    b
}

/// The box as an artboard rectangle (`None` when it is empty or not finite).
pub(crate) fn box_rect(b: [f32; 4]) -> Option<IRect> {
    if !b.iter().all(|v| v.is_finite() && v.abs() < 1.0e6) {
        return None;
    }
    let r = IRect::new(b[0].round() as i32, b[1].round() as i32, b[2].round() as i32, b[3].round() as i32);
    (!r.is_empty()).then_some(r)
}

/// Enter / "Apply crop": the artboard becomes the box with the chosen shape (one entry). A box equal to the artboard
/// with the same shape changes nothing.
pub(crate) fn apply(state: &mut AppState) {
    let Some(b) = current_box(state) else { return };
    let Some(rect) = box_rect(b) else {
        state.status_hint = "The crop box is empty".to_owned();
        return;
    };
    let Some(on) = edit_of(state).and_then(edit_ref) else { return };
    let opts = state.sticker_ui.crop_opts;
    let r = state.doc_mut().map(|d| d.crop_sticker(on, rect, opts.artboard_shape(), opts.delete_pixels));
    if let Some(r) = r
        && crate::dispatch::edited(state, r).is_some()
    {
        state.sticker_ui.crop = None;
        state.status_hint.clear();
    }
}

/// Draws the crop box: the area outside it darkened, its outline (and the shape it will cut), its handles.
pub(crate) fn draw(painter: &egui::Painter, state: &mut AppState, geom: Geom, rect: Rect) {
    let Some(cam) = edit_of(state).map(|e| e.camera) else { return };
    let Some(b) = current_box(state) else { return };
    let r = Rect::from_two_pos(geom.to_screen(&cam, rect, Vec2::new(b[0], b[1])), geom.to_screen(&cam, rect, Vec2::new(b[2], b[3])));
    let shade = Color32::from_black_alpha(110);
    for part in [
        Rect::from_min_max(rect.min, egui::pos2(rect.max.x, r.min.y)),
        Rect::from_min_max(egui::pos2(rect.min.x, r.max.y), rect.max),
        Rect::from_min_max(egui::pos2(rect.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)),
        Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(rect.max.x, r.max.y)),
    ] {
        let part = part.intersect(rect);
        if part.is_positive() {
            painter.rect_filled(part, 0.0, shade);
        }
    }
    painter.rect_stroke(r, 0.0, Stroke::new(3.0, Color32::from_black_alpha(140)), egui::StrokeKind::Middle);
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
    match state.sticker_ui.crop_opts.artboard_shape() {
        ArtboardShape::Ellipse => {
            painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() * 0.5, Stroke::new(1.0, crate::theme::ACCENT)));
        }
        ArtboardShape::RoundedRect { radius } => {
            let k = r.width() / (b[2] - b[0]).max(1.0);
            let rr = (radius * k).min(r.width().min(r.height()) * 0.5).max(0.0);
            painter.rect_stroke(r, rr, Stroke::new(1.0, crate::theme::ACCENT), egui::StrokeKind::Middle);
        }
        ArtboardShape::Rect => {}
    }
    for (_, q) in grips(b) {
        let p = geom.to_screen(&cam, rect, q);
        let h = Rect::from_center_size(p, egui::vec2(8.0, 8.0));
        painter.rect_filled(h, 0.0, Color32::WHITE);
        painter.rect_stroke(h, 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Outside);
    }
}

/// The Crop tool's options in the Tool tab: shape, radius, aspect, Delete cropped pixels, Apply / Cancel.
pub(crate) fn options(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<crate::action::AppAction>, hits: &mut Vec<(crate::state::PanelHit, Rect)>) {
    use crate::state::PanelHit;
    let o = &mut state.sticker_ui.crop_opts;
    ui.horizontal_wrapped(|ui| {
        ui.label("Shape");
        for (i, s) in CropShape::ALL.into_iter().enumerate() {
            let r = ui.selectable_value(&mut o.shape, s, s.label());
            hits.push((PanelHit::StickerCropShape(i as u8), r.rect));
        }
    });
    if o.shape == CropShape::Rounded {
        ui.horizontal(|ui| {
            ui.label("Radius");
            ui.add(egui::DragValue::new(&mut o.radius).range(0.0..=2048.0).speed(0.5).max_decimals(1).suffix(" px"));
        });
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("Aspect");
        for a in CropAspect::ALL {
            ui.selectable_value(&mut o.aspect, a, a.label());
        }
    });
    let r = ui.checkbox(&mut o.delete_pixels, "Delete cropped pixels").on_hover_text("Off: the pixels outside stay in the layers (a later crop can show them again)");
    hits.push((PanelHit::StickerCropDelete, r.rect));
    let b = state.sticker_ui.crop.or_else(|| current_box(state));
    if let Some(b) = b {
        ui.weak(format!("{:.0} x {:.0} px at {:.0}, {:.0}", b[2] - b[0], b[3] - b[1], b[0], b[1]));
    }
    ui.horizontal(|ui| {
        let apply = ui.button("Apply crop").on_hover_text("Make the box the artboard (Enter)");
        hits.push((PanelHit::StickerCropApply, apply.rect));
        if apply.clicked() {
            actions.push(crate::action::AppAction::Sticker(StickerAction::ApplyCrop));
        }
        let cancel = ui.add_enabled(state.sticker_ui.crop.is_some(), egui::Button::new("Cancel")).on_hover_text("Back to the artboard (Esc)");
        hits.push((PanelHit::StickerCropCancel, cancel.rect));
        if cancel.clicked() {
            actions.push(crate::action::AppAction::Sticker(StickerAction::CancelCrop));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_drags_move_edges_keep_the_aspect_and_never_empty_the_box() {
        let b = [0.0, 0.0, 100.0, 50.0];
        assert_eq!(dragged(CropGrip::Edge(1, 0), b, Vec2::ZERO, Vec2::new(80.4, 9.0), CropAspect::Free), [0.0, 0.0, 80.0, 50.0]);
        assert_eq!(dragged(CropGrip::Edge(-1, -1), b, Vec2::ZERO, Vec2::new(120.0, 70.0), CropAspect::Free), [100.0, 50.0, 120.0, 70.0], "a corner dragged past the other side flips over");
        assert_eq!(dragged(CropGrip::Move, b, Vec2::new(10.0, 10.0), Vec2::new(15.0, 5.0), CropAspect::Free), [5.0, -5.0, 105.0, 45.0]);
        assert_eq!(dragged(CropGrip::New, b, Vec2::new(10.0, 10.0), Vec2::new(50.0, 20.0), CropAspect::Square), [10.0, 10.0, 50.0, 50.0]);
        assert_eq!(dragged(CropGrip::New, b, Vec2::new(10.0, 10.0), Vec2::new(10.0, 10.0), CropAspect::Free), [10.0, 10.0, 11.0, 11.0], "at least 1 px");
        assert_eq!(dragged(CropGrip::Edge(1, 1), b, Vec2::ZERO, Vec2::new(160.0, 10.0), CropAspect::SixteenNine), [0.0, 0.0, 160.0, 90.0]);
        assert_eq!(box_rect([0.4, 0.6, 10.2, 9.5]), Some(IRect::new(0, 1, 10, 10)));
        assert_eq!(box_rect([f32::NAN, 0.0, 1.0, 1.0]), None);
        assert_eq!(box_rect([3.0, 3.0, 3.2, 9.0]), None);
        let o = CropOptions { shape: CropShape::Rounded, radius: f32::NAN, ..CropOptions::default() };
        assert_eq!(o.artboard_shape(), ArtboardShape::RoundedRect { radius: 0.0 });
    }
}
