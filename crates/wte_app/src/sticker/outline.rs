// Owner: stickers P7b (STICKER_PLAN 3.4; ARCHITECTURE 8.15). The selection's outline on the canvas: the edges between
// selected (mask >= 128) and unselected pixels, merged into horizontal and vertical runs, computed once per selection
// (the cache holds the `Arc` it was made from, compared by pointer) and drawn as a static two-colour dashed line (white
// under black dashes whose phase follows the screen position, so neighbouring runs join up). No animation: marching
// ants would need a repaint every frame and the reactive frame loop stays idle. A selection with more than `MAX_RUNS`
// runs (a noisy Color Range) or no edge at 128 (a faint feather) is drawn as its bounding box, as P7a drew every
// selection.

use std::sync::Arc;

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke};
use glam::Vec2;
use wte_core::{IRect, Selection};

use crate::state::Canvas2D;
use crate::sticker::canvas::Geom;

/// The most runs drawn one by one.
pub(crate) const MAX_RUNS: usize = 50_000;
/// Length of a dash and of a gap (screen points).
const DASH: f32 = 4.0;

/// The runs of one selection's outline (artboard pixel coordinates).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Outline {
    /// Horizontal edges `(y, x0, x1)`: between pixel rows `y - 1` and `y`, from `x0` to `x1`.
    pub(crate) h: Vec<[u32; 3]>,
    /// Vertical edges `(x, y0, y1)`: between pixel columns `x - 1` and `x`.
    pub(crate) v: Vec<[u32; 3]>,
    /// More than `MAX_RUNS` runs (or none at 128): the bounds are drawn instead.
    pub(crate) boxed: bool,
    pub(crate) bounds: IRect,
}

/// The outline of the selection last drawn (`StickerUi::outline`).
#[derive(Clone, Debug, Default)]
pub(crate) struct OutlineCache {
    made_from: Option<Arc<Selection>>,
    outline: Arc<Outline>,
    /// Outlines computed so far (tests: once per selection).
    pub(crate) computed: u64,
}

impl OutlineCache {
    /// The outline of `sel`, computed when `sel` is not the selection of the last call.
    pub(crate) fn get(&mut self, sel: &Arc<Selection>) -> Arc<Outline> {
        if self.made_from.as_ref().is_none_or(|m| !Arc::ptr_eq(m, sel)) {
            self.outline = Arc::new(outline_of(sel));
            self.made_from = Some(Arc::clone(sel));
            self.computed += 1;
        }
        Arc::clone(&self.outline)
    }
}

/// The outline of a selection.
pub(crate) fn outline_of(sel: &Selection) -> Outline {
    let b = sel.bounds;
    let mut out = Outline { bounds: b, ..Outline::default() };
    if b.is_empty() {
        return out;
    }
    let (w, h) = (sel.mask.width as i64, sel.mask.height as i64);
    let inside = |x: i64, y: i64| x >= 0 && y >= 0 && x < w && y < h && sel.mask.data[(y * w + x) as usize] >= 128;
    let (x0, x1, y0, y1) = (b.x0 as i64, b.x1 as i64, b.y0 as i64, b.y1 as i64);
    let mut runs = 0usize;
    'h: for y in y0..=y1 {
        let mut start: Option<i64> = None;
        for x in x0..=x1 {
            let edge = x < x1 && inside(x, y - 1) != inside(x, y);
            match (edge, start) {
                (true, None) => start = Some(x),
                (false, Some(s)) => {
                    out.h.push([y as u32, s as u32, x as u32]);
                    start = None;
                    runs += 1;
                    if runs > MAX_RUNS {
                        break 'h;
                    }
                }
                _ => {}
            }
        }
    }
    'v: for x in x0..=x1 {
        let mut start: Option<i64> = None;
        for y in y0..=y1 {
            let edge = y < y1 && inside(x - 1, y) != inside(x, y);
            match (edge, start) {
                (true, None) => start = Some(y),
                (false, Some(s)) => {
                    out.v.push([x as u32, s as u32, y as u32]);
                    start = None;
                    runs += 1;
                    if runs > MAX_RUNS {
                        break 'v;
                    }
                }
                _ => {}
            }
        }
    }
    if runs > MAX_RUNS || runs == 0 {
        out.h.clear();
        out.v.clear();
        out.boxed = true;
    }
    out
}

/// Draws `outline` on the canvas (`rect`, the canvas camera `cam`, the artboard's frame `geom`).
pub(crate) fn draw(painter: &egui::Painter, outline: &Outline, geom: Geom, cam: &Canvas2D, rect: Rect) {
    let at = |x: u32, y: u32| geom.to_screen(cam, rect, geom.origin + Vec2::new(x as f32, y as f32));
    if outline.boxed {
        let b = outline.bounds;
        if b.is_empty() {
            return;
        }
        let (a, z) = (at(b.x0 as u32, b.y0 as u32), at(b.x1 as u32, b.y1 as u32));
        let r = Rect::from_two_pos(a, z);
        let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        for w in corners.windows(2) {
            dashed(painter, w[0], w[1], rect);
        }
        return;
    }
    for &[y, x0, x1] in &outline.h {
        dashed(painter, at(x0, y), at(x1, y), rect);
    }
    for &[x, y0, y1] in &outline.v {
        dashed(painter, at(x, y0), at(x, y1), rect);
    }
}

/// One axis-aligned run: white, with black dashes on every other `DASH` of the screen coordinate along it.
fn dashed(painter: &egui::Painter, a: Pos2, b: Pos2, clip: Rect) {
    let horizontal = (a.y - b.y).abs() < 0.01;
    let (lo, hi) = if horizontal { (a.x.min(b.x), a.x.max(b.x)) } else { (a.y.min(b.y), a.y.max(b.y)) };
    let (clo, chi) = if horizontal { (clip.min.x, clip.max.x) } else { (clip.min.y, clip.max.y) };
    let across = if horizontal { a.y } else { a.x };
    let (alo, ahi) = if horizontal { (clip.min.y, clip.max.y) } else { (clip.min.x, clip.max.x) };
    let (lo, hi) = (lo.max(clo), hi.min(chi));
    if lo >= hi || across < alo - 1.0 || across > ahi + 1.0 {
        return;
    }
    let point = |t: f32| if horizontal { Pos2::new(t, across) } else { Pos2::new(across, t) };
    painter.line_segment([point(lo), point(hi)], Stroke::new(1.0, Color32::WHITE));
    let mut k = (lo / DASH).floor() as i64;
    while (k as f32) * DASH < hi {
        if k.rem_euclid(2) == 0 {
            let (s, e) = ((k as f32 * DASH).max(lo), ((k + 1) as f32 * DASH).min(hi));
            if e > s {
                painter.line_segment([point(s), point(e)], Stroke::new(1.0, Color32::BLACK));
            }
        }
        k += 1;
    }
}
