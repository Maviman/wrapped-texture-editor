// Owner: app-panels (A4). The status bar of ARCHITECTURE.md 8.7: tool hint, hover read-out (set, UV, texel, RGBA),
// texture size, zoom, undo label, job spinner with stage and progress; branch `pen-input`: the pen read-out ("Pen 47 %",
// "Pen", "Pen (eraser)") while a pen is in range, the item dropped first.
//
// The items never overlap (R3-10, finding F9): `plan` decides from their measured widths which ones fit, dropping the
// least important first, and the hint takes the rest of the width, elided at its end.

use eframe::egui;
use egui::{Align, Color32, FontId, Layout, Pos2, Rect, RichText, TextStyle, vec2};

use crate::panels::to_color32;
use crate::state::{AppState, BusyInfo, HoverInfo, ViewMode};

/// Width of the job progress bar.
const PROGRESS_WIDTH: f32 = 120.0;
/// Side of the colour chip in front of the hover read-out.
const SWATCH_SIDE: f32 = 12.0;
/// Width the hint keeps before lower-priority items are dropped (it is elided to fit, never dropped).
const HINT_MIN_WIDTH: f32 = 160.0;
/// Width of a separator line including its own spacing (egui's `Separator` default).
const SEPARATOR_SPACING: f32 = 6.0;

/// The hover read-out, e.g. "Set 1 | UV 0.512, 0.250 | Texel 524, 256 | RGBA 255 0 0 255"; "" when nothing is hovered
/// (the fullest of `hover_variants`).
#[cfg(test)]
pub(crate) fn hover_text(hover: &HoverInfo, set_name: Option<&str>) -> String {
    hover_variants(hover, set_name).into_iter().next().unwrap_or_default()
}

/// The hover read-out from the fullest to the shortest form: with the set's name, with the set number only, without
/// the set, texel and RGBA only, RGBA only. Repeated and empty forms are left out.
pub(crate) fn hover_variants(hover: &HoverInfo, set_name: Option<&str>) -> Vec<String> {
    let set_full = hover.set.map(|set| match set_name {
        Some(name) => format!("Set {} ({name})", set.0 + 1),
        None => format!("Set {}", set.0 + 1),
    });
    let set_short = hover.set.map(|set| format!("Set {}", set.0 + 1));
    let uv = hover.uv.map(|[u, v]| format!("UV {u:.3}, {v:.3}"));
    let texel = hover.texel.map(|[x, y]| format!("Texel {x}, {y}"));
    let rgba = hover.rgba.map(|[r, g, b, a]| format!("RGBA {r} {g} {b} {a}"));
    let forms = [
        vec![&set_full, &uv, &texel, &rgba],
        vec![&set_short, &uv, &texel, &rgba],
        vec![&uv, &texel, &rgba],
        vec![&texel, &rgba],
        vec![&rgba],
    ];
    let mut out: Vec<String> = Vec::new();
    for form in forms {
        let text = form.into_iter().flatten().map(String::as_str).collect::<Vec<_>>().join("  |  ");
        if !text.is_empty() && out.last() != Some(&text) {
            out.push(text);
        }
    }
    out
}

/// Measured widths of the status bar items (points); `None` = the item has nothing to show.
#[derive(Clone, Debug, Default)]
pub(crate) struct StatusWidths {
    /// Width of the whole bar.
    pub avail: f32,
    /// Cost of one separator between two items.
    pub sep: f32,
    /// Spinner, stage text and progress bar of a running job.
    pub busy: Option<f32>,
    pub hint: Option<f32>,
    /// The forms of `hover_variants`, fullest first (colour chip included).
    pub hover: Vec<f32>,
    pub size: Option<f32>,
    pub zoom: Option<f32>,
    pub undo: Option<f32>,
    /// Branch `pen-input`: the pen read-out (`PenFrame::readout`), only while a pen is in range.
    pub pen: Option<f32>,
}

/// Which items the status bar draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StatusPlan {
    /// Width given to the hint (it is elided at its end when it is narrower than the text).
    pub hint: Option<f32>,
    /// Index into `hover_variants`.
    pub hover: Option<usize>,
    pub size: bool,
    pub zoom: bool,
    pub undo: bool,
    pub pen: bool,
}

/// Fits the items into `w.avail`. The job progress is always shown and the hint keeps `HINT_MIN_WIDTH`; then, while
/// they fit, in this order: texture size, the shortest hover read-out, zoom, fuller hover read-outs, the undo label, the
/// pen read-out (branch `pen-input`: the lowest priority, so the bar of a mouse user never changes).
/// The hint gets what is left, up to its full width. Every item costs its width plus one separator.
pub(crate) fn plan(w: &StatusWidths) -> StatusPlan {
    let item = |x: f32| x + w.sep;
    let hint_reserved = w.hint.map_or(0.0, |h| item(h.min(HINT_MIN_WIDTH)));
    let mut used = w.busy.map_or(0.0, item) + hint_reserved;
    let mut p = StatusPlan::default();
    let take = |used: &mut f32, width: Option<f32>| -> bool {
        match width {
            Some(x) if *used + item(x) <= w.avail => {
                *used += item(x);
                true
            }
            _ => false,
        }
    };
    p.size = take(&mut used, w.size);
    let mut hover_cost = 0.0;
    if let Some(&shortest) = w.hover.last()
        && take(&mut used, Some(shortest))
    {
        p.hover = Some(w.hover.len() - 1);
        hover_cost = item(shortest);
    }
    p.zoom = take(&mut used, w.zoom);
    if let Some(mut index) = p.hover {
        while index > 0 {
            let cost = item(w.hover[index - 1]);
            if used - hover_cost + cost > w.avail {
                break;
            }
            used += cost - hover_cost;
            hover_cost = cost;
            index -= 1;
        }
        p.hover = Some(index);
    }
    p.undo = take(&mut used, w.undo);
    p.pen = (p.undo || w.undo.is_none()) && take(&mut used, w.pen);
    p.hint = w.hint.map(|h| h.min(w.avail - (used - hint_reserved) - w.sep).max(0.0));
    p
}

/// "Title: stage" of a running job.
fn stage_text(busy: &BusyInfo) -> String {
    if busy.stage.is_empty() { busy.title.clone() } else { format!("{}: {}", busy.title, busy.stage) }
}

/// Draws the status bar: hint, hover read-out, texture size, zoom, undo label, job progress. The drawn item rects go to
/// `state.status_rects`.
pub(super) fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let mut rects = Vec::new();
    ui.horizontal(|ui| {
        ui.set_min_height(20.0);
        let full = ui.available_rect_before_wrap();
        let gap = ui.spacing().item_spacing.x;
        let body = TextStyle::Body.resolve(ui.style());
        let mono = TextStyle::Monospace.resolve(ui.style());
        let measure = |text: &str, font: &FontId| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x;

        let session = state.session.as_ref();
        let doc = session.map(|s| &s.doc);
        let busy = state.busy.as_ref().map(|b| (stage_text(b), b.fraction));
        let undo = doc.and_then(|d| d.undo_label()).map(|l| format!("Undo: {l}"));
        let zoom = session
            .filter(|_| matches!(state.view_mode, ViewMode::View2D | ViewMode::Split))
            .map(|s| format!("Zoom {:.0} %", s.uv_view.zoom * 100.0));
        let size = session.and_then(|s| {
            let doc = &s.doc;
            doc.set(s.active_set).map(|set| {
                if doc.sets.len() > 1 {
                    format!("{}: {} x {}", set.name, set.width, set.height)
                } else {
                    format!("{} x {}", set.width, set.height)
                }
            })
        });
        let hover = if state.hover.view.is_some() {
            let set_name = state.hover.set.and_then(|s| doc.and_then(|d| d.set(s))).map(|s| s.name.as_str());
            hover_variants(&state.hover, set_name)
        } else {
            Vec::new()
        };
        let chip = if state.hover.rgba.is_some() { SWATCH_SIDE + gap } else { 0.0 };
        let pen = state.pen.frame.readout();
        let spinner = ui.spacing().interact_size.y;
        let widths = StatusWidths {
            avail: full.width(),
            sep: SEPARATOR_SPACING + 2.0 * gap,
            busy: busy.as_ref().map(|(stage, _)| PROGRESS_WIDTH + gap + measure(stage, &body) + gap + spinner),
            hint: (!state.status_hint.is_empty()).then(|| measure(&state.status_hint, &body)),
            hover: hover.iter().map(|t| measure(t, &mono) + chip).collect(),
            size: size.as_deref().map(|t| measure(t, &body)),
            zoom: zoom.as_deref().map(|t| measure(t, &body)),
            undo: undo.as_deref().map(|t| measure(t, &body)),
            pen: pen.as_deref().map(|t| measure(t, &body)),
        };
        let plan = plan(&widths);

        // Right group, drawn first so the left group knows where it has to stop.
        let mut right = ui.new_child(egui::UiBuilder::new().max_rect(full).layout(Layout::right_to_left(Align::Center)));
        let mut first = true;
        let mut separate = |ui: &mut egui::Ui| {
            if !std::mem::take(&mut first) {
                ui.separator();
            }
        };
        if let Some((stage, fraction)) = &busy {
            separate(&mut right);
            rects.push(right.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).desired_width(PROGRESS_WIDTH).show_percentage()).rect);
            rects.push(right.label(stage.as_str()).rect);
            rects.push(right.add(egui::Spinner::new()).rect);
        }
        if plan.undo && let Some(text) = &undo {
            separate(&mut right);
            rects.push(right.label(RichText::new(text).weak()).rect);
        }
        if plan.zoom && let Some(text) = &zoom {
            separate(&mut right);
            rects.push(right.label(text.as_str()).rect);
        }
        if plan.size && let Some(text) = &size {
            separate(&mut right);
            rects.push(right.label(text.as_str()).on_hover_text("Texture size of the active texture set").rect);
        }
        if plan.pen && let Some(text) = &pen {
            separate(&mut right);
            rects.push(right.label(text.as_str()).on_hover_text("Pen pressure (Help > Tablet setup)").rect);
        }
        let right_used = right.min_rect();
        let right_left = if right_used.is_positive() { right_used.left() - gap - SEPARATOR_SPACING } else { full.right() };

        // Left group, clipped at the right group: whatever the measurements say, nothing can overlap.
        let left_rect = Rect::from_min_max(full.min, Pos2::new(right_left.max(full.left()), full.bottom()));
        let mut left = ui.new_child(egui::UiBuilder::new().max_rect(left_rect).layout(Layout::left_to_right(Align::Center)));
        left.shrink_clip_rect(left_rect);
        let mut shown_left = false;
        if let Some(width) = plan.hint
            && width >= 1.0
        {
            let width = width.min(left_rect.width());
            let resp = left
                .allocate_ui_with_layout(vec2(width, full.height()), Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&state.status_hint).strong()).truncate())
                })
                .inner;
            rects.push(resp.rect);
            shown_left = true;
        }
        if let Some(index) = plan.hover
            && let Some(text) = hover.get(index)
        {
            if shown_left {
                left.separator();
            }
            if let Some(rgba) = state.hover.rgba {
                let (rect, _) = left.allocate_exact_size(vec2(SWATCH_SIDE, SWATCH_SIDE), egui::Sense::hover());
                crate::panels::paint_checker(left.painter(), rect);
                left.painter().rect_filled(rect, 0.0, to_color32(rgba));
                rects.push(rect);
            }
            rects.push(left.add(egui::Label::new(RichText::new(text).monospace()).truncate()).rect);
        }
        ui.advance_cursor_after_rect(full);
    });
    state.status_rects = rects;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::tests::{run_panels, sample_state};
    use crate::panels::PanelState;
    use wte_core::{SampleKind, TextureSetId};

    fn full_hover() -> HoverInfo {
        HoverInfo {
            view: Some(crate::state::ViewId::Main3D),
            set: Some(TextureSetId(0)),
            uv: Some([0.5, 0.25]),
            texel: Some([512, 256]),
            rgba: Some([255, 0, 0, 255]),
            world: Some([0.0, 0.0, 0.0]),
        }
    }

    #[test]
    fn hover_text_lists_what_is_known() {
        assert_eq!(hover_text(&HoverInfo::default(), None), "");
        let hover = full_hover();
        assert_eq!(hover_text(&hover, Some("Body")), "Set 1 (Body)  |  UV 0.500, 0.250  |  Texel 512, 256  |  RGBA 255 0 0 255");
        let partial = HoverInfo { set: Some(TextureSetId(1)), uv: Some([0.0, 1.0]), ..hover.clone() };
        let partial = HoverInfo { texel: None, rgba: None, ..partial };
        assert_eq!(hover_text(&partial, None), "Set 2  |  UV 0.000, 1.000");
    }

    #[test]
    fn hover_variants_get_shorter_and_never_repeat() {
        let v = hover_variants(&full_hover(), Some("Body"));
        assert_eq!(
            v,
            [
                "Set 1 (Body)  |  UV 0.500, 0.250  |  Texel 512, 256  |  RGBA 255 0 0 255",
                "Set 1  |  UV 0.500, 0.250  |  Texel 512, 256  |  RGBA 255 0 0 255",
                "UV 0.500, 0.250  |  Texel 512, 256  |  RGBA 255 0 0 255",
                "Texel 512, 256  |  RGBA 255 0 0 255",
                "RGBA 255 0 0 255",
            ]
        );
        assert!(v.windows(2).all(|w| w[0].len() > w[1].len()));
        // Without a set name the first two forms are the same: listed once.
        assert_eq!(hover_variants(&full_hover(), None).len(), 4);
        assert!(hover_variants(&HoverInfo::default(), None).is_empty());
    }

    fn widths(avail: f32) -> StatusWidths {
        StatusWidths {
            avail,
            sep: 14.0,
            busy: None,
            hint: Some(220.0),
            hover: vec![420.0, 360.0, 300.0, 230.0, 120.0],
            size: Some(70.0),
            zoom: Some(80.0),
            undo: Some(130.0),
            pen: Some(60.0),
        }
    }

    /// Total width the plan draws (each item plus one separator).
    fn plan_width(w: &StatusWidths, p: &StatusPlan) -> f32 {
        let item = |x: f32| x + w.sep;
        w.busy.map_or(0.0, item)
            + p.hint.map_or(0.0, item)
            + p.hover.map_or(0.0, |i| item(w.hover[i]))
            + if p.size { item(w.size.unwrap()) } else { 0.0 }
            + if p.zoom { item(w.zoom.unwrap()) } else { 0.0 }
            + if p.undo { item(w.undo.unwrap()) } else { 0.0 }
            + if p.pen { item(w.pen.unwrap()) } else { 0.0 }
    }

    #[test]
    fn a_wide_bar_shows_everything_and_a_narrow_one_drops_the_least_important_first() {
        let w = widths(1400.0);
        let p = plan(&w);
        assert_eq!(p, StatusPlan { hint: Some(220.0), hover: Some(0), size: true, zoom: true, undo: true, pen: true });
        // Without a pen in range the plan is the one of a mouse user.
        let p = plan(&StatusWidths { pen: None, ..w });
        assert_eq!(p, StatusPlan { hint: Some(220.0), hover: Some(0), size: true, zoom: true, undo: true, pen: false });

        // 900 pt: the pen read-out and the undo label go before the hover read-out loses anything.
        let w = widths(900.0);
        let p = plan(&w);
        assert!(!p.undo && !p.pen && p.size && p.zoom, "{p:?}");
        // The pen read-out is dropped first: where the undo label still fits, the pen may not.
        let w = widths(960.0);
        let p = plan(&w);
        assert!(p.undo && !p.pen, "{p:?}");
        assert_eq!(p.hover, Some(0));

        // 640 pt (the 640x480 window minus the toolbar): a shorter read-out, the hint elided rather than overlapping.
        let w = widths(596.0);
        let p = plan(&w);
        assert!(p.size && !p.undo, "{p:?}");
        assert!(p.hover.is_some_and(|i| i > 0), "{p:?}");

        // Very narrow: only the hint (elided) is left.
        let w = widths(200.0);
        let p = plan(&w);
        assert_eq!(p, StatusPlan { hint: Some(186.0), hover: None, size: false, zoom: false, undo: false, pen: false });

        for avail in [120.0, 200.0, 400.0, 596.0, 640.0, 800.0, 1000.0, 1236.0, 1600.0] {
            let w = widths(avail);
            let p = plan(&w);
            assert!(plan_width(&w, &p) <= avail + 0.01, "{avail}: {p:?} needs {}", plan_width(&w, &p));
            // A running job is always shown, and still nothing overflows the rest.
            let w = StatusWidths { busy: Some(100.0), ..widths(avail) };
            let p = plan(&w);
            assert!(plan_width(&w, &p) <= avail.max(100.0 + 2.0 * w.sep) + 0.01, "{avail} busy: {p:?}");
        }
    }

    /// The real bar, drawn headless: no two items overlap at 1280, 900 and 640 points, and every item stays on screen.
    #[test]
    fn status_items_never_overlap() {
        for (w, h) in [(1280.0, 800.0), (900.0, 600.0), (640.0, 480.0)] {
            let ctx = egui::Context::default();
            let mut state = sample_state(SampleKind::Sphere);
            state.status_hint = "Type the text in the tool options first".to_owned();
            state.hover = full_hover();
            state.view_mode = ViewMode::Split;
            // Branch `pen-input`: a pen in contact adds its read-out.
            state.pen.frame = crate::pen::PenFrame { in_range: true, in_contact: true, pressure: 0.47, ..Default::default() };
            state.doc_mut().unwrap().sets[0].name = "sphere_checker".to_owned();
            let doc = state.doc_mut().unwrap();
            let paint = doc.sets[0].active.unwrap();
            doc.set_layer_opacity(paint, 0.5, wte_core::Commit::Final).unwrap();
            let mut ps = PanelState::new();
            let input = || egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))),
                ..Default::default()
            };
            for _ in 0..3 {
                run_panels(&ctx, &mut state, &mut ps, input());
            }
            let rects = state.status_rects.clone();
            assert!(rects.len() >= 3, "{w}: {rects:?}");
            for (i, a) in rects.iter().enumerate() {
                assert!(a.left() >= -0.5 && a.right() <= w + 0.5, "{w}: item {i} {a:?} is off screen");
                for b in &rects[i + 1..] {
                    let overlap = a.intersect(*b);
                    assert!(!overlap.is_positive() || overlap.width() < 0.5, "{w}: {a:?} overlaps {b:?}");
                }
            }
            if w >= 1280.0 {
                // Hint, chip, read-out, size, zoom and undo label fit; the pen read-out is dropped first.
                assert_eq!(rects.len(), 6, "{rects:?}");
                // Branch `pen-input`: without the hover read-out the pen read-out fits too, apart from the others.
                let hover = std::mem::take(&mut state.hover);
                run_panels(&ctx, &mut state, &mut ps, input());
                let rects = state.status_rects.clone();
                assert_eq!(rects.len(), 5, "hint, size, zoom, undo, pen: {rects:?}");
                for (i, a) in rects.iter().enumerate() {
                    for b in &rects[i + 1..] {
                        let overlap = a.intersect(*b);
                        assert!(!overlap.is_positive() || overlap.width() < 0.5, "{w} pen: {a:?} overlaps {b:?}");
                    }
                }
                state.hover = hover;
            }
            // With a job running the bar still fits.
            state.busy = Some(BusyInfo { title: "Export".into(), stage: "writing sphere.png".into(), fraction: 0.4, modal: false });
            run_panels(&ctx, &mut state, &mut ps, input());
            let rects = state.status_rects.clone();
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    let overlap = a.intersect(*b);
                    assert!(!overlap.is_positive() || overlap.width() < 0.5, "{w} busy: {a:?} overlaps {b:?}");
                }
            }
        }
    }
}
