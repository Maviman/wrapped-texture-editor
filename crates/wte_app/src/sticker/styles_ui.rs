// Owner: stickers P6b (STICKER_PLAN 2.3, 3.2, 3.7, 3.9; ARCHITECTURE 8.7, 8.15). The `StyleStack` editor: one block
// per style (die-cut contour, stroke, drop shadow, inner shadow, outer glow, inner glow, colour overlay, gradient
// overlay, bevel), each with an on/off box in its header and its numbers in the body; the preset row and Copy / Paste /
// Clear. The same editor edits a stamp's styles (the Sticker tab: image, text and sticker stamps, E3) and a sticker
// layer's styles (the Tool tab in the Sticker view). Every change goes out through `make` as an `AppAction` with the
// `Commit` of `panels::continuous_edit` (Preview while dragged, Final at the release: one entry per drag).
//
// A block that was never used is `None` in the stack; ticking its box creates it with default numbers scaled to the
// picture (`k` = reference side / 512, as the presets); unticking keeps the numbers (`on: false`, 2.3).

use eframe::egui;
use egui::color_picker::{Alpha, color_edit_button_srgba};
use egui::{Rect, Response};
use wte_core::{
    Bevel, ColorOverlay, Commit, Contour, Glow, Gradient, GradientOverlay, Shadow, Stroke, StrokePos, StylePreset, StyleStack,
};

use crate::action::AppAction;
use crate::panels::{PanelState, continuous_edit, from_color32, percent_slider, to_color32};
use crate::state::PanelHit;

/// The style blocks in the order of the editor (the index is `PanelHit::StyleBlockToggle`).
pub(crate) const BLOCKS: [&str; 9] = [
    "Die-cut contour",
    "Stroke",
    "Drop shadow",
    "Inner shadow",
    "Outer glow",
    "Inner glow",
    "Color overlay",
    "Gradient overlay",
    "Bevel",
];

/// The one line under the Stroke block of a text stamp (STICKER_PLAN 2.3).
pub(crate) const TEXT_OUTLINE_NOTE: &str = "Text also has its own Outline (Text tab). This stroke goes around it.";

/// The longest style length the sliders offer (2.3: every length 0..=256).
const MAX_LENGTH: f32 = 256.0;

/// What the editor needs besides the stack.
pub(crate) struct EditorCx<'a> {
    pub ps: &'a mut PanelState,
    pub out: &'a mut Vec<AppAction>,
    pub hits: &'a mut Vec<(PanelHit, Rect)>,
    /// The action for a new stack.
    pub make: &'a dyn Fn(StyleStack, Commit) -> AppAction,
    /// Scale of the default numbers of a block ticked for the first time (reference side / 512).
    pub k: f32,
    /// The Stroke block shows `TEXT_OUTLINE_NOTE`.
    pub text_note: bool,
    /// The edited stack differs between the selected stamps (the "mixed values" hint).
    pub mixed: bool,
}

impl EditorCx<'_> {
    /// Sends `new` for the response of one control (Preview while it is dragged or typed in, Final at the end).
    fn emit(&mut self, ui: &egui::Ui, resp: &Response, new: StyleStack) {
        let make = self.make;
        continuous_edit(self.ps, self.out, ui, resp, |c| make(new.clone(), c));
    }
}

/// A slider of a length (pixels at reference scale) that never rewrites an idle value (`SliderClamping::Edits`, see
/// `panels::percent_slider`).
fn length_slider(value: &mut f32, max: f32) -> egui::Slider<'_> {
    egui::Slider::from_get_set(0.0..=f64::from(max), move |v| {
        if let Some(v) = v {
            *value = v as f32;
        }
        f64::from(*value)
    })
    .suffix(" px")
    .max_decimals(1)
    .clamping(egui::SliderClamping::Edits)
}

/// A slider of an angle in degrees.
fn angle_slider(value: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Slider<'_> {
    egui::Slider::from_get_set(f64::from(*range.start())..=f64::from(*range.end()), move |v| {
        if let Some(v) = v {
            *value = v as f32;
        }
        f64::from(*value)
    })
    .suffix(" °")
    .max_decimals(0)
    .clamping(egui::SliderClamping::Edits)
}

/// A plain number slider (bevel depth).
fn number_slider(value: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Slider<'_> {
    egui::Slider::from_get_set(f64::from(*range.start())..=f64::from(*range.end()), move |v| {
        if let Some(v) = v {
            *value = v as f32;
        }
        f64::from(*value)
    })
    .max_decimals(2)
    .clamping(egui::SliderClamping::Edits)
}

/// One labelled row of a block body.
fn row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 18.0), egui::Sense::hover());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, egui::TextStyle::Body.resolve(ui.style()), ui.visuals().text_color());
        add(ui)
    })
    .inner
}

/// A colour button.
fn color_row(ui: &mut egui::Ui, label: &str, color: [u8; 4]) -> (Response, [u8; 4]) {
    row(ui, label, |ui| {
        let mut c = to_color32(color);
        let resp = color_edit_button_srgba(ui, &mut c, Alpha::OnlyBlend);
        (resp, from_color32(c))
    })
}

/// The default numbers of block `i` for a picture of reference side `512 * k` (on).
pub(crate) fn default_block(i: usize, k: f32) -> StyleStack {
    let shadow = |opacity: f32, distance: f32, size: f32| Shadow {
        on: true,
        color: [0, 0, 0, 255],
        opacity,
        angle_deg: 120.0,
        distance: distance * k,
        spread: 0.0,
        size: size * k,
    };
    let glow = |color: [u8; 4], size: f32| Glow { on: true, color, opacity: 0.75, spread: 0.0, size: size * k };
    let mut s = StyleStack::default();
    match i {
        0 => s.contour = Some(Contour { on: true, width: 12.0 * k, smooth: 0.5, fill_holes: true, color: [255; 4], opacity: 1.0 }),
        1 => s.stroke = Some(Stroke { on: true, width: 6.0 * k, position: StrokePos::Outside, color: [0, 0, 0, 255], opacity: 1.0 }),
        2 => s.drop_shadow = Some(shadow(0.4, 6.0, 10.0)),
        3 => s.inner_shadow = Some(shadow(0.5, 4.0, 8.0)),
        4 => s.outer_glow = Some(glow([255, 240, 160, 255], 24.0)),
        5 => s.inner_glow = Some(glow([255, 255, 255, 255], 12.0)),
        6 => s.color_overlay = Some(ColorOverlay { on: true, color: [255, 80, 80, 255], opacity: 1.0 }),
        7 => {
            s.gradient_overlay =
                Some(GradientOverlay { on: true, gradient: Gradient::two([255, 255, 255, 255], [0, 0, 0, 255]), angle_deg: 90.0, opacity: 0.5 })
        }
        _ => {
            s.bevel = Some(Bevel {
                on: true,
                size: 6.0 * k,
                depth: 1.0,
                angle_deg: 120.0,
                altitude_deg: 30.0,
                highlight: [255, 255, 255, 255],
                shadow: [0, 0, 0, 255],
                opacity: 0.75,
            })
        }
    }
    s
}

/// Whether block `i` of `s` exists and is on.
pub(crate) fn block_on(s: &StyleStack, i: usize) -> Option<bool> {
    match i {
        0 => s.contour.as_ref().map(|v| v.on),
        1 => s.stroke.as_ref().map(|v| v.on),
        2 => s.drop_shadow.as_ref().map(|v| v.on),
        3 => s.inner_shadow.as_ref().map(|v| v.on),
        4 => s.outer_glow.as_ref().map(|v| v.on),
        5 => s.inner_glow.as_ref().map(|v| v.on),
        6 => s.color_overlay.as_ref().map(|v| v.on),
        7 => s.gradient_overlay.as_ref().map(|v| v.on),
        _ => s.bevel.as_ref().map(|v| v.on),
    }
}

/// `s` with block `i` switched on or off: a block never used is created with its default numbers (`k`); an existing
/// block keeps its numbers.
pub(crate) fn toggled(s: &StyleStack, i: usize, on: bool, k: f32) -> StyleStack {
    let mut s = s.clone();
    if block_on(&s, i).is_none() {
        if !on {
            return s;
        }
        let d = default_block(i, k);
        match i {
            0 => s.contour = d.contour,
            1 => s.stroke = d.stroke,
            2 => s.drop_shadow = d.drop_shadow,
            3 => s.inner_shadow = d.inner_shadow,
            4 => s.outer_glow = d.outer_glow,
            5 => s.inner_glow = d.inner_glow,
            6 => s.color_overlay = d.color_overlay,
            7 => s.gradient_overlay = d.gradient_overlay,
            _ => s.bevel = d.bevel,
        }
        return s;
    }
    match i {
        0 => s.contour.iter_mut().for_each(|v| v.on = on),
        1 => s.stroke.iter_mut().for_each(|v| v.on = on),
        2 => s.drop_shadow.iter_mut().for_each(|v| v.on = on),
        3 => s.inner_shadow.iter_mut().for_each(|v| v.on = on),
        4 => s.outer_glow.iter_mut().for_each(|v| v.on = on),
        5 => s.inner_glow.iter_mut().for_each(|v| v.on = on),
        6 => s.color_overlay.iter_mut().for_each(|v| v.on = on),
        7 => s.gradient_overlay.iter_mut().for_each(|v| v.on = on),
        _ => s.bevel.iter_mut().for_each(|v| v.on = on),
    }
    s
}

/// The preset row: one button per `StylePreset` (index = `PanelHit::StylePresetButton`). Returns the clicked preset.
pub(crate) fn preset_row(ui: &mut egui::Ui, hits: &mut Vec<(PanelHit, Rect)>, enabled: bool) -> Option<StylePreset> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (i, preset) in StylePreset::ALL.into_iter().enumerate() {
            let tip = match preset {
                StylePreset::ClassicSticker => "White die-cut border and a soft shadow",
                StylePreset::Outline => "A black stroke around the artwork",
                StylePreset::Glow => "A soft light glow",
                StylePreset::Badge => "Border, dark stroke, bevel and shadow",
                StylePreset::ShadowOnly => "Only a drop shadow",
                StylePreset::Clear => "No styles",
            };
            let resp = ui.add_enabled(enabled, egui::Button::new(preset.name())).on_hover_text(tip);
            hits.push((PanelHit::StylePresetButton(i as u8), resp.rect));
            if resp.clicked() {
                picked = Some(preset);
            }
        }
    });
    picked
}

/// The style editor for `current`. Returns its rect.
pub(crate) fn editor(ui: &mut egui::Ui, salt: &str, current: &StyleStack, enabled: bool, cx: &mut EditorCx<'_>) -> Rect {
    let out = ui.add_enabled_ui(enabled, |ui| {
        ui.vertical(|ui| {
            if cx.mixed {
                ui.colored_label(ui.visuals().warn_fg_color, "The selected stamps have different styles: a change gives all of them these.");
            }
            for i in 0..BLOCKS.len() {
                block(ui, salt, i, current, cx);
            }
        })
        .response
        .rect
    });
    let rect = out.inner;
    cx.hits.push((PanelHit::StyleEditor, rect));
    rect
}

/// One block: header with the on/off box, body with the numbers (shown when open and the block exists).
fn block(ui: &mut egui::Ui, salt: &str, i: usize, current: &StyleStack, cx: &mut EditorCx<'_>) {
    let id = ui.make_persistent_id(("style_block", salt, i));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let was = block_on(current, i);
    let k = cx.k;
    let header = state.show_header(ui, |ui| {
        let mut on = was.unwrap_or(false);
        let resp = ui.checkbox(&mut on, BLOCKS[i]).on_hover_text("Switch this style on or off (its numbers are kept)");
        (resp, on)
    });
    let (_, head, _) = header.body(|ui| {
        if was.is_none() {
            ui.weak("Tick the box to add this style.");
            return;
        }
        body(ui, i, current, cx);
    });
    let (resp, on) = head.inner;
    cx.hits.push((PanelHit::StyleBlockToggle(i as u8), resp.rect));
    if resp.changed() {
        cx.out.push((cx.make)(toggled(current, i, on, k), Commit::Final));
    }
}

/// The numbers of block `i` (it exists in `current`).
fn body(ui: &mut egui::Ui, i: usize, current: &StyleStack, cx: &mut EditorCx<'_>) {
    match i {
        0 => {
            let Some(v) = current.contour.clone() else { return };
            let with = |f: &dyn Fn(&mut Contour)| {
                let mut s = current.clone();
                let mut v = v.clone();
                f(&mut v);
                s.contour = Some(v);
                s
            };
            let mut w = v.width;
            let r = row(ui, "Width", |ui| ui.add(length_slider(&mut w, MAX_LENGTH)));
            cx.emit(ui, &r, with(&|c| c.width = w));
            let mut sm = v.smooth;
            let r = row(ui, "Smooth", |ui| ui.add(percent_slider(&mut sm, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.smooth = sm));
            let mut fill = v.fill_holes;
            let r = row(ui, "", |ui| ui.checkbox(&mut fill, "Fill holes"));
            cx.emit(ui, &r, with(&|c| c.fill_holes = fill));
            let (r, color) = color_row(ui, "Color", v.color);
            cx.emit(ui, &r, with(&|c| c.color = color));
            let mut o = v.opacity;
            let r = row(ui, "Opacity", |ui| ui.add(percent_slider(&mut o, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.opacity = o));
        }
        1 => {
            let Some(v) = current.stroke.clone() else { return };
            let with = |f: &dyn Fn(&mut Stroke)| {
                let mut s = current.clone();
                let mut v = v.clone();
                f(&mut v);
                s.stroke = Some(v);
                s
            };
            let mut w = v.width;
            let r = row(ui, "Width", |ui| ui.add(length_slider(&mut w, MAX_LENGTH)));
            cx.emit(ui, &r, with(&|c| c.width = w));
            // Outside / Center / Inside as three buttons (no popup: the tablet rule of 3.10), one entry per click.
            let mut pos = v.position;
            row(ui, "Position", |ui| {
                for p in [StrokePos::Outside, StrokePos::Center, StrokePos::Inside] {
                    ui.selectable_value(&mut pos, p, pos_name(p));
                }
            });
            if pos != v.position {
                cx.out.push((cx.make)(with(&|c| c.position = pos), Commit::Final));
            }
            let (r, color) = color_row(ui, "Color", v.color);
            cx.emit(ui, &r, with(&|c| c.color = color));
            let mut o = v.opacity;
            let r = row(ui, "Opacity", |ui| ui.add(percent_slider(&mut o, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.opacity = o));
            if cx.text_note {
                ui.weak(TEXT_OUTLINE_NOTE);
            }
        }
        2 | 3 => {
            let Some(v) = (if i == 2 { current.drop_shadow.clone() } else { current.inner_shadow.clone() }) else { return };
            let with = |f: &dyn Fn(&mut Shadow)| {
                let mut s = current.clone();
                let mut v = v.clone();
                f(&mut v);
                if i == 2 {
                    s.drop_shadow = Some(v);
                } else {
                    s.inner_shadow = Some(v);
                }
                s
            };
            let (r, color) = color_row(ui, "Color", v.color);
            cx.emit(ui, &r, with(&|c| c.color = color));
            let mut o = v.opacity;
            let r = row(ui, "Opacity", |ui| ui.add(percent_slider(&mut o, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.opacity = o));
            let mut a = v.angle_deg;
            let r = row(ui, "Angle", |ui| ui.add(angle_slider(&mut a, -180.0..=180.0)));
            cx.emit(ui, &r, with(&|c| c.angle_deg = a));
            let mut d = v.distance;
            let r = row(ui, "Distance", |ui| ui.add(length_slider(&mut d, MAX_LENGTH)));
            cx.emit(ui, &r, with(&|c| c.distance = d));
            let mut sp = v.spread;
            let r = row(ui, "Spread", |ui| ui.add(percent_slider(&mut sp, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.spread = sp));
            let mut sz = v.size;
            let r = row(ui, "Size", |ui| ui.add(length_slider(&mut sz, MAX_LENGTH)));
            cx.emit(ui, &r, with(&|c| c.size = sz));
        }
        4 | 5 => {
            let Some(v) = (if i == 4 { current.outer_glow.clone() } else { current.inner_glow.clone() }) else { return };
            let with = |f: &dyn Fn(&mut Glow)| {
                let mut s = current.clone();
                let mut v = v.clone();
                f(&mut v);
                if i == 4 {
                    s.outer_glow = Some(v);
                } else {
                    s.inner_glow = Some(v);
                }
                s
            };
            let (r, color) = color_row(ui, "Color", v.color);
            cx.emit(ui, &r, with(&|c| c.color = color));
            let mut o = v.opacity;
            let r = row(ui, "Opacity", |ui| ui.add(percent_slider(&mut o, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.opacity = o));
            let mut sp = v.spread;
            let r = row(ui, "Spread", |ui| ui.add(percent_slider(&mut sp, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.spread = sp));
            let mut sz = v.size;
            let r = row(ui, "Size", |ui| ui.add(length_slider(&mut sz, MAX_LENGTH)));
            cx.emit(ui, &r, with(&|c| c.size = sz));
        }
        6 => {
            let Some(v) = current.color_overlay.clone() else { return };
            let with = |f: &dyn Fn(&mut ColorOverlay)| {
                let mut s = current.clone();
                let mut v = v.clone();
                f(&mut v);
                s.color_overlay = Some(v);
                s
            };
            let (r, color) = color_row(ui, "Color", v.color);
            cx.emit(ui, &r, with(&|c| c.color = color));
            let mut o = v.opacity;
            let r = row(ui, "Opacity", |ui| ui.add(percent_slider(&mut o, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.opacity = o));
        }
        7 => {
            let Some(v) = current.gradient_overlay.clone() else { return };
            let with = |f: &dyn Fn(&mut GradientOverlay)| {
                let mut s = current.clone();
                let mut v = v.clone();
                f(&mut v);
                s.gradient_overlay = Some(v);
                s
            };
            let first = v.gradient.stops.first().map_or([255; 4], |s| s.1);
            let last = v.gradient.stops.last().map_or([0, 0, 0, 255], |s| s.1);
            let (r, color) = color_row(ui, "From", first);
            cx.emit(ui, &r, with(&|c| set_end(&mut c.gradient, false, color)));
            let (r, color) = color_row(ui, "To", last);
            cx.emit(ui, &r, with(&|c| set_end(&mut c.gradient, true, color)));
            let mut a = v.angle_deg;
            let r = row(ui, "Angle", |ui| ui.add(angle_slider(&mut a, -180.0..=180.0)));
            cx.emit(ui, &r, with(&|c| c.angle_deg = a));
            let mut o = v.opacity;
            let r = row(ui, "Opacity", |ui| ui.add(percent_slider(&mut o, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.opacity = o));
        }
        _ => {
            let Some(v) = current.bevel.clone() else { return };
            let with = |f: &dyn Fn(&mut Bevel)| {
                let mut s = current.clone();
                let mut v = v.clone();
                f(&mut v);
                s.bevel = Some(v);
                s
            };
            let mut sz = v.size;
            let r = row(ui, "Size", |ui| ui.add(length_slider(&mut sz, MAX_LENGTH)));
            cx.emit(ui, &r, with(&|c| c.size = sz));
            let mut d = v.depth;
            let r = row(ui, "Depth", |ui| ui.add(number_slider(&mut d, 0.0..=10.0)));
            cx.emit(ui, &r, with(&|c| c.depth = d));
            let mut a = v.angle_deg;
            let r = row(ui, "Angle", |ui| ui.add(angle_slider(&mut a, -180.0..=180.0)));
            cx.emit(ui, &r, with(&|c| c.angle_deg = a));
            let mut alt = v.altitude_deg;
            let r = row(ui, "Altitude", |ui| ui.add(angle_slider(&mut alt, 0.0..=90.0)));
            cx.emit(ui, &r, with(&|c| c.altitude_deg = alt));
            let (r, color) = color_row(ui, "Highlight", v.highlight);
            cx.emit(ui, &r, with(&|c| c.highlight = color));
            let (r, color) = color_row(ui, "Shadow", v.shadow);
            cx.emit(ui, &r, with(&|c| c.shadow = color));
            let mut o = v.opacity;
            let r = row(ui, "Opacity", |ui| ui.add(percent_slider(&mut o, 0.0, 100.0)));
            cx.emit(ui, &r, with(&|c| c.opacity = o));
        }
    }
}

/// The first (`last` false) or last stop's colour of a gradient.
fn set_end(g: &mut Gradient, last: bool, color: [u8; 4]) {
    if g.stops.len() < 2 {
        g.sanitize();
    }
    let i = if last { g.stops.len() - 1 } else { 0 };
    g.stops[i].1 = color;
}

fn pos_name(p: StrokePos) -> &'static str {
    match p {
        StrokePos::Outside => "Outside",
        StrokePos::Center => "Center",
        StrokePos::Inside => "Inside",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_ticked_for_the_first_time_gets_scaled_defaults_and_unticking_keeps_the_numbers() {
        let none = StyleStack::default();
        for i in 0..BLOCKS.len() {
            assert_eq!(block_on(&none, i), None);
            assert_eq!(toggled(&none, i, false, 1.0), none, "unticking a block that was never used changes nothing");
            let on = toggled(&none, i, true, 2.0);
            assert_eq!(block_on(&on, i), Some(true), "block {i}");
            assert_eq!(on, on.sanitized(), "block {i}: the defaults are within the limits");
            let off = toggled(&on, i, false, 2.0);
            assert_eq!(block_on(&off, i), Some(false));
            assert_eq!(toggled(&off, i, true, 9.0), on, "block {i}: ticking again keeps the numbers");
            // Only that block exists.
            assert_eq!((0..BLOCKS.len()).filter(|j| block_on(&on, *j).is_some()).count(), 1);
        }
        let small = toggled(&none, 0, true, 0.5).contour.unwrap().width;
        let large = toggled(&none, 0, true, 2.0).contour.unwrap().width;
        assert!((large - 4.0 * small).abs() < 1e-4, "the defaults scale with the picture");
    }

    #[test]
    fn the_gradient_ends_are_its_first_and_last_stops() {
        let mut g = Gradient::two([1, 2, 3, 255], [4, 5, 6, 255]);
        set_end(&mut g, true, [9, 9, 9, 255]);
        set_end(&mut g, false, [7, 7, 7, 255]);
        assert_eq!(g.stops.iter().map(|s| s.1).collect::<Vec<_>>(), vec![[7, 7, 7, 255], [9, 9, 9, 255]]);
        let mut empty = Gradient { stops: Vec::new() };
        set_end(&mut empty, true, [9, 9, 9, 255]);
        assert_eq!(empty.stops.len(), 2);
    }
}
