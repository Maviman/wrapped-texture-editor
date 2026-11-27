// Owner: stickers P8 (STICKER_PLAN 3.3 "Shape", 4.8; ARCHITECTURE 8.15). The sticker Shape tool: a drag on the canvas
// adds a shape layer filling the dragged box (a click: a 64-pixel shape centred there), with the Tool tab's options:
// kind (rectangle, rounded rectangle, ellipse, polygon, star, line), fill (none, the primary colour, or a gradient
// from the primary to the secondary colour), outline (the secondary colour and a width), corner radius, sides /
// points. One entry "Add Shape Layer". With a shape layer active the same options edit it ("Edit Sticker Shape").

use eframe::egui;
use glam::Vec2;
use wte_core::{Gradient, Paint, ShapeKind, ShapeSpec, StickerContent, StickerLayer, StickerLayerId};

use crate::state::AppState;
use crate::sticker::{edit_of, layered, set_active};

/// History labels.
pub(crate) const ADD_SHAPE_LABEL: &str = "Add Shape Layer";
pub(crate) const EDIT_SHAPE_LABEL: &str = "Edit Sticker Shape";
/// The size of a shape made by a click (layer-space pixels).
const CLICK_SIZE: f32 = 64.0;
/// A drag shorter than this (layer-space pixels, both axes) is a click.
pub(crate) const MIN_DRAG: f32 = 2.0;

/// The kinds of the Shape tool.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ShapeChoice {
    #[default]
    Rectangle,
    Rounded,
    Ellipse,
    Polygon,
    Star,
    Line,
}

impl ShapeChoice {
    pub(crate) const ALL: [ShapeChoice; 6] =
        [ShapeChoice::Rectangle, ShapeChoice::Rounded, ShapeChoice::Ellipse, ShapeChoice::Polygon, ShapeChoice::Star, ShapeChoice::Line];

    pub(crate) fn label(self) -> &'static str {
        match self {
            ShapeChoice::Rectangle => "Rectangle",
            ShapeChoice::Rounded => "Rounded",
            ShapeChoice::Ellipse => "Ellipse",
            ShapeChoice::Polygon => "Polygon",
            ShapeChoice::Star => "Star",
            ShapeChoice::Line => "Line",
        }
    }

    fn of(kind: &ShapeKind) -> ShapeChoice {
        match kind {
            ShapeKind::Rect => ShapeChoice::Rectangle,
            ShapeKind::RoundedRect { .. } => ShapeChoice::Rounded,
            ShapeKind::Ellipse => ShapeChoice::Ellipse,
            ShapeKind::Polygon { .. } => ShapeChoice::Polygon,
            ShapeKind::Star { .. } => ShapeChoice::Star,
            ShapeKind::Line { .. } => ShapeChoice::Line,
        }
    }
}

/// How a new shape is filled.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum FillChoice {
    None,
    #[default]
    Solid,
    Gradient,
}

/// The Shape tool's options (`StickerUi::shape`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ShapeOptions {
    pub kind: ShapeChoice,
    pub radius: f32,
    pub sides: u32,
    pub points: u32,
    pub inner: f32,
    pub line_width: f32,
    pub fill: FillChoice,
    pub outline: bool,
    pub outline_width: f32,
}

impl Default for ShapeOptions {
    fn default() -> ShapeOptions {
        ShapeOptions {
            kind: ShapeChoice::Rectangle,
            radius: 12.0,
            sides: 6,
            points: 5,
            inner: 0.5,
            line_width: 8.0,
            fill: FillChoice::Solid,
            outline: false,
            outline_width: 4.0,
        }
    }
}

impl ShapeOptions {
    /// The shape of these options in a box of `size` with the colours `primary` (fill) and `secondary` (outline,
    /// gradient end).
    pub(crate) fn spec(&self, size: Vec2, primary: [u8; 4], secondary: [u8; 4]) -> ShapeSpec {
        let kind = match self.kind {
            ShapeChoice::Rectangle => ShapeKind::Rect,
            ShapeChoice::Rounded => ShapeKind::RoundedRect { radius: self.radius },
            ShapeChoice::Ellipse => ShapeKind::Ellipse,
            ShapeChoice::Polygon => ShapeKind::Polygon { sides: self.sides },
            ShapeChoice::Star => ShapeKind::Star { points: self.points, inner: self.inner },
            ShapeChoice::Line => ShapeKind::Line { width: self.line_width },
        };
        let fill = match self.fill {
            FillChoice::None => Paint::None,
            FillChoice::Solid => Paint::Solid(primary),
            FillChoice::Gradient => Paint::Linear { gradient: Gradient::two(primary, secondary), angle_deg: 90.0 },
        };
        let mut spec = ShapeSpec { kind, size, fill, outline: self.outline.then_some((secondary, self.outline_width)) };
        spec.sanitize();
        spec
    }

    /// The options a shape layer has (its kind and numbers; the colours stay those of the layer).
    pub(crate) fn of(spec: &ShapeSpec, base: &ShapeOptions) -> ShapeOptions {
        let mut o = ShapeOptions { kind: ShapeChoice::of(&spec.kind), ..*base };
        match spec.kind {
            ShapeKind::RoundedRect { radius } => o.radius = radius,
            ShapeKind::Polygon { sides } => o.sides = sides,
            ShapeKind::Star { points, inner } => {
                o.points = points;
                o.inner = inner;
            }
            ShapeKind::Line { width } => o.line_width = width,
            _ => {}
        }
        o.fill = match spec.fill {
            Paint::None => FillChoice::None,
            Paint::Solid(_) => FillChoice::Solid,
            Paint::Linear { .. } => FillChoice::Gradient,
        };
        o.outline = spec.outline.is_some();
        if let Some((_, w)) = spec.outline {
            o.outline_width = w;
        }
        o
    }

    /// `spec` with these options' kind and numbers, its own colours kept where the fill kind stays.
    pub(crate) fn apply_to(&self, spec: &ShapeSpec, primary: [u8; 4], secondary: [u8; 4]) -> ShapeSpec {
        let mut new = self.spec(spec.size, primary, secondary);
        match (&spec.fill, &mut new.fill) {
            (Paint::Solid(c), Paint::Solid(n)) => *n = *c,
            (Paint::Linear { gradient, angle_deg }, Paint::Linear { gradient: g, angle_deg: a }) => {
                *g = gradient.clone();
                *a = *angle_deg;
            }
            _ => {}
        }
        if let (Some((c, _)), Some((n, _))) = (spec.outline, new.outline.as_mut()) {
            *n = c;
        }
        new
    }
}

/// The box `[x0, y0, x1, y1]` a drag from `a` to `b` makes (a click: a 64-pixel box centred at `a`).
pub(crate) fn drag_box(a: Vec2, b: Vec2) -> [f32; 4] {
    let d = (b - a).abs();
    if d.x < MIN_DRAG && d.y < MIN_DRAG {
        let h = CLICK_SIZE * 0.5;
        return [a.x - h, a.y - h, a.x + h, a.y + h].map(f32::round);
    }
    let (lo, hi) = (a.min(b).round(), a.max(b).round());
    [lo.x, lo.y, hi.x.max(lo.x + 1.0), hi.y.max(lo.y + 1.0)]
}

/// "Add shape": a shape layer filling the box `b` (layer space; `None` = half the artboard, centred) above the active
/// layer, which becomes the active layer. One entry.
pub(crate) fn add(state: &mut AppState, b: Option<[f32; 4]>) {
    let Some((doc, _)) = layered(state, None).or_else(|| {
        let e = edit_of(state)?.clone();
        let session = state.session.as_mut()?;
        let d = crate::sticker::shown(&mut session.doc, &e)?.layers()?.clone();
        let id = d.layers.first()?.id;
        Some((d, id))
    }) else {
        state.status_hint = "Convert the text to a layered sticker first".to_owned();
        return;
    };
    let a = doc.artboard;
    let b = b.filter(|b| b.iter().all(|v| v.is_finite())).unwrap_or_else(|| {
        let (w, h) = (a.width() as f32, a.height() as f32);
        [a.x0 as f32 + w * 0.25, a.y0 as f32 + h * 0.25, a.x0 as f32 + w * 0.75, a.y0 as f32 + h * 0.75].map(f32::round)
    });
    let size = Vec2::new((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
    let spec = state.sticker_ui.shape.spec(size, state.colors.primary, state.colors.secondary);
    let name = state.sticker_ui.shape.kind.label().to_owned();
    let new_id = StickerLayerId(doc.next_layer);
    let above = edit_of(state).and_then(|e| e.active_layer).and_then(|id| doc.layers.iter().position(|l| l.id == id));
    let done = crate::sticker::apply_edit(state, ADD_SHAPE_LABEL, wte_core::Commit::Final, move |d| {
        let id = d.alloc_layer();
        let mut layer = StickerLayer::new(id, &name, StickerContent::Shape(spec));
        layer.transform.translate = Vec2::new(b[0], b[1]);
        let i = above.map_or(d.layers.len(), |i| (i + 1).min(d.layers.len()));
        d.layers.insert(i, layer);
    });
    if done {
        set_active(state, new_id);
    }
}

/// The Shape tool's options in the Tool tab; with a shape layer active they edit it (one entry per drag or click,
/// through `text_block::value_edit`).
pub(crate) fn options(
    ui: &mut egui::Ui,
    state: &mut AppState,
    ps: &mut crate::panels::PanelState,
    active: Option<(StickerLayerId, ShapeSpec, bool)>,
    out: &mut Vec<crate::action::AppAction>,
) {
    let before = match &active {
        Some((_, spec, _)) => ShapeOptions::of(spec, &state.sticker_ui.shape),
        None => state.sticker_ui.shape,
    };
    let mut o = before;
    let mut resp = ui
        .horizontal_wrapped(|ui| {
            let mut r = ui.label("Kind");
            for k in ShapeChoice::ALL {
                r = r.union(ui.selectable_value(&mut o.kind, k, k.label()));
            }
            r
        })
        .inner;
    let mut add = |r: egui::Response| resp = resp.union(r);
    match o.kind {
        ShapeChoice::Rounded => add(ui.horizontal(|ui| {
            ui.label("Radius");
            ui.add(egui::DragValue::new(&mut o.radius).range(0.0..=4096.0).speed(0.5).suffix(" px"))
        }).inner),
        ShapeChoice::Polygon => add(ui.horizontal(|ui| {
            ui.label("Sides");
            ui.add(egui::Slider::new(&mut o.sides, 3..=12))
        }).inner),
        ShapeChoice::Star => {
            add(ui.horizontal(|ui| {
                ui.label("Points");
                ui.add(egui::Slider::new(&mut o.points, 3..=24))
            }).inner);
            add(ui.horizontal(|ui| {
                ui.label("Inner");
                ui.add(egui::Slider::new(&mut o.inner, 0.1..=0.95).max_decimals(2))
            }).inner);
        }
        ShapeChoice::Line => add(ui.horizontal(|ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut o.line_width).range(0.0..=4096.0).speed(0.5).suffix(" px"))
        }).inner),
        _ => {}
    }
    add(ui.horizontal(|ui| {
        ui.label("Fill");
        let a = ui.selectable_value(&mut o.fill, FillChoice::None, "None");
        let b = ui.selectable_value(&mut o.fill, FillChoice::Solid, "Color").on_hover_text("The primary colour");
        let c = ui.selectable_value(&mut o.fill, FillChoice::Gradient, "Gradient").on_hover_text("Primary to secondary colour");
        a.union(b).union(c)
    }).inner);
    add(ui.horizontal(|ui| {
        let a = ui.checkbox(&mut o.outline, "Outline").on_hover_text("The secondary colour");
        let b = ui.add_enabled(o.outline, egui::DragValue::new(&mut o.outline_width).range(0.0..=256.0).speed(0.2).suffix(" px"));
        a.union(b)
    }).inner);
    match active {
        Some((id, spec, locked)) => {
            if !locked {
                let (p, s) = (state.colors.primary, state.colors.secondary);
                let new = o.apply_to(&spec, p, s);
                crate::panels::text_block::value_edit(ps, out, ui, &resp, o != before, |commit| {
                    crate::action::AppAction::Sticker(crate::sticker::StickerAction::SetLayerShape(id, Box::new(new.clone()), commit))
                });
            }
        }
        None => state.sticker_ui.shape = o,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_options_make_and_read_back_a_spec() {
        let o = ShapeOptions { kind: ShapeChoice::Star, points: 7, inner: 0.4, fill: FillChoice::Gradient, outline: true, outline_width: 3.0, ..ShapeOptions::default() };
        let spec = o.spec(Vec2::new(40.0, 30.0), [255, 0, 0, 255], [0, 0, 255, 255]);
        assert_eq!(spec.kind, ShapeKind::Star { points: 7, inner: 0.4 });
        assert!(matches!(spec.fill, Paint::Linear { .. }));
        assert_eq!(spec.outline, Some(([0, 0, 255, 255], 3.0)));
        let back = ShapeOptions::of(&spec, &ShapeOptions::default());
        assert_eq!((back.kind, back.points, back.fill, back.outline), (ShapeChoice::Star, 7, FillChoice::Gradient, true));
        let solid = ShapeOptions::default().spec(Vec2::new(10.0, 10.0), [1, 2, 3, 255], [0; 4]);
        let recoloured = ShapeOptions { kind: ShapeChoice::Ellipse, ..ShapeOptions::default() }.apply_to(&solid, [9, 9, 9, 255], [0; 4]);
        assert_eq!((recoloured.kind, recoloured.fill), (ShapeKind::Ellipse, Paint::Solid([1, 2, 3, 255])), "the layer keeps its colour");
        assert_eq!(drag_box(Vec2::new(10.0, 10.0), Vec2::new(11.0, 10.5)), [-22.0, -22.0, 42.0, 42.0]);
        assert_eq!(drag_box(Vec2::new(30.0, 40.0), Vec2::new(10.2, 20.0)), [10.0, 20.0, 30.0, 40.0]);
        let bad = ShapeOptions { sides: 99, radius: f32::NAN, kind: ShapeChoice::Polygon, ..ShapeOptions::default() }.spec(Vec2::new(5.0, 5.0), [0; 4], [0; 4]);
        assert_eq!(bad.kind, ShapeKind::Polygon { sides: 12 }, "sanitized");
    }
}
