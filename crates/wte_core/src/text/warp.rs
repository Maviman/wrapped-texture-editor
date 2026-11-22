// Owner: T1 text geometry (round 4). Warp of the letter shapes (FEATURE_TEXT 5.6): 15 styles, the vertical
// direction, the two distortions and the outline refinement.
//
// The formulas are our own; they give the shapes the style names describe (FEATURE_TEXT 5.6, checked against the
// reference script `target\round4_design\work\text\warp_check.py`). Normalised frame coordinates: `u = (x - cx) / a`,
// `v = (y - cy) / h`, `v = -1` is the top edge.

use std::f32::consts::PI;

use crate::text::{Warp, WarpStyle};

/// Below this half size (px) a warp does nothing.
const MIN_HALF_PX: f32 = 0.5;
/// Points per side of the frame border used for the distortion box.
const BORDER_SAMPLES: usize = 65;
/// Outline refinement (FEATURE_TEXT 5.6): longest straight piece before mapping, deviation and mapped length that
/// make a piece split, and the most halvings.
const REFINE_MAX_PIECE_PX: f32 = 8.0;
const REFINE_TOLERANCE_PX: f32 = 0.1;
const REFINE_MAX_MAPPED_PX: f32 = 16.0;
const REFINE_MAX_DEPTH: u32 = 6;

/// A warp prepared for one warp frame (the bounding box of all glyph cells, text-space pixels).
///
/// Build it with [`WarpMap::new`]; it is immutable (`new` caches the box of the distortion from `warp` and the
/// frame), so the fields are private and read through [`WarpMap::warp`], [`WarpMap::frame_min`] and
/// [`WarpMap::frame_max`] (contract request T1-2, applied at M1).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WarpMap {
    warp: Warp,
    frame_min: [f32; 2],
    frame_max: [f32; 2],
    /// The bounding box of the warped frame border, `[min_x, min_y, max_x, max_y]` (the box of the distortion);
    /// `None` when neither distortion is set.
    distortion_box: Option<[f32; 4]>,
}

/// `c(t) = 1 - t^2`.
fn c(t: f32) -> f32 {
    1.0 - t * t
}

/// The style on normalised coordinates of a frame with half sizes `a` (along u) and `h` (along v).
fn style(style: WarpStyle, b: f32, u: f32, v: f32, a: f32, h: f32) -> (f32, f32) {
    match style {
        WarpStyle::Arc => {
            if b.abs() < 1e-4 {
                return (u, v);
            }
            let rn = 2.0 * a / (b.abs() * PI);
            let n = h.min(0.75 * rn);
            let t = a * u / rn;
            if b > 0.0 {
                let r = rn - n + (1.0 - v) * h;
                (r * t.sin() / a, (h - n + rn - r * t.cos()) / h)
            } else {
                let r = rn - n + (v + 1.0) * h;
                (r * t.sin() / a, (-h + n - rn + r * t.cos()) / h)
            }
        }
        WarpStyle::ArcLower => (u, v + b * c(u) * (1.0 + v) / 2.0),
        WarpStyle::ArcUpper => (u, v - b * c(u) * (1.0 - v) / 2.0),
        WarpStyle::Arch => (u, v - b * c(u)),
        WarpStyle::Bulge => (u, v * (1.0 + 0.5 * b * c(u))),
        WarpStyle::ShellLower => (u, v + b * u * u * (1.0 + v) / 2.0),
        WarpStyle::ShellUpper => (u, v - b * u * u * (1.0 - v) / 2.0),
        WarpStyle::Flag => (u, v - 0.5 * b * (PI * u).sin()),
        WarpStyle::Wave => (u + 0.1 * b * (PI * v).sin(), v - 0.5 * b * (PI * u).sin()),
        WarpStyle::Fish => (u, v * (1.0 + 0.5 * b * (PI * u).sin())),
        WarpStyle::Rise => (u, v - 0.25 * b * (3.0 * u - u * u * u)),
        WarpStyle::Fisheye => {
            let q = u * u + v * v;
            if q < 1.0 {
                let g = 1.0 + 0.4 * b * (1.0 - q);
                (u * g, v * g)
            } else {
                (u, v)
            }
        }
        WarpStyle::Inflate => (u * (1.0 + 0.25 * b * c(v)), v * (1.0 + 0.5 * b * c(u))),
        WarpStyle::Squeeze => (u * (1.0 - 0.5 * b * c(v)), v * (1.0 + 0.5 * b * c(u))),
        WarpStyle::Twist => {
            let r = (u * u + v * v).sqrt().min(1.0);
            let al = b * (PI / 2.0) * (1.0 - r) * (1.0 - r);
            let (s, co) = al.sin_cos();
            (u * co - v * s, u * s + v * co)
        }
    }
}

impl WarpMap {
    /// The map of `warp` on the frame `frame_min..frame_max`.
    pub fn new(warp: &Warp, frame_min: [f32; 2], frame_max: [f32; 2]) -> WarpMap {
        let mut map = WarpMap { warp: *warp, frame_min, frame_max, distortion_box: None };
        if map.is_inactive() || (warp.horizontal == 0.0 && warp.vertical == 0.0) {
            return map;
        }
        // The box of the warped frame border, each side sampled at 65 points.
        let mut lo = [f32::INFINITY; 2];
        let mut hi = [f32::NEG_INFINITY; 2];
        let mut add = |p: [f32; 2]| {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        };
        for i in 0..BORDER_SAMPLES {
            let t = i as f32 / (BORDER_SAMPLES - 1) as f32;
            let x = frame_min[0] + t * (frame_max[0] - frame_min[0]);
            let y = frame_min[1] + t * (frame_max[1] - frame_min[1]);
            add(map.styled([x, frame_min[1]]));
            add(map.styled([x, frame_max[1]]));
            add(map.styled([frame_min[0], y]));
            add(map.styled([frame_max[0], y]));
        }
        if lo[0].is_finite() && hi[0] - lo[0] > 0.0 && hi[1] - lo[1] > 0.0 {
            map.distortion_box = Some([lo[0], lo[1], hi[0], hi[1]]);
        }
        map
    }

    /// The warp this map was built for.
    pub fn warp(&self) -> &Warp {
        &self.warp
    }

    /// Lower corner of the warp frame (text-space pixels).
    pub fn frame_min(&self) -> [f32; 2] {
        self.frame_min
    }

    /// Upper corner of the warp frame (text-space pixels).
    pub fn frame_max(&self) -> [f32; 2] {
        self.frame_max
    }

    /// Centre and half sizes of the frame.
    fn frame(&self) -> ([f32; 2], f32, f32) {
        let centre = [0.5 * (self.frame_min[0] + self.frame_max[0]), 0.5 * (self.frame_min[1] + self.frame_max[1])];
        (centre, 0.5 * (self.frame_max[0] - self.frame_min[0]), 0.5 * (self.frame_max[1] - self.frame_min[1]))
    }

    /// True when the map is the identity: a frame thinner than 1 px, a non-finite number, or no bend and no
    /// distortion.
    fn is_inactive(&self) -> bool {
        let (_, a, h) = self.frame();
        let w = &self.warp;
        !(a >= MIN_HALF_PX && h >= MIN_HALF_PX && a.is_finite() && h.is_finite())
            || !(w.bend.is_finite() && w.horizontal.is_finite() && w.vertical.is_finite())
            || (w.bend == 0.0 && w.horizontal == 0.0 && w.vertical == 0.0)
    }

    /// The style alone (with the direction), no distortion.
    fn styled(&self, p: [f32; 2]) -> [f32; 2] {
        let w = &self.warp;
        let (centre, a, h) = self.frame();
        if w.bend == 0.0 || (w.style == WarpStyle::Arc && w.bend.abs() < 1e-4) {
            return p;
        }
        let x = p[0] - centre[0];
        let y = p[1] - centre[1];
        if w.vertical_axis {
            // Turn by -90 degrees, apply the style with the half sizes exchanged, turn back.
            let (pp, q) = style(w.style, w.bend, y / h, -x / a, h, a);
            [centre[0] - q * a, centre[1] + pp * h]
        } else {
            let (u, v) = style(w.style, w.bend, x / a, y / h, a, h);
            [centre[0] + a * u, centre[1] + h * v]
        }
    }

    /// The warped position of a text-space point.
    pub fn map(&self, p: [f32; 2]) -> [f32; 2] {
        if self.is_inactive() {
            return p;
        }
        let s = self.styled(p);
        let Some([x0, y0, x1, y1]) = self.distortion_box else { return s };
        let (bx, by) = (0.5 * (x0 + x1), 0.5 * (y0 + y1));
        let (ba, bh) = (0.5 * (x1 - x0), 0.5 * (y1 - y0));
        let us = (s[0] - bx) / ba;
        let vs = (s[1] - by) / bh;
        [
            bx + (s[0] - bx) * (1.0 + 0.4 * self.warp.vertical * vs),
            by + (s[1] - by) * (1.0 + 0.4 * self.warp.horizontal * us),
        ]
    }

    /// A flattened polyline cut into pieces short enough that the mapped polyline follows the true curve within
    /// about 0.1 px (FEATURE_TEXT 5.6, "Outline refinement"), then mapped. `closed`: the last point joins the first
    /// (the result is closed the same way and does not repeat its first point).
    pub fn map_polyline(&self, points: &[[f32; 2]], closed: bool) -> Vec<[f32; 2]> {
        if self.is_inactive() || points.len() < 2 {
            return points.iter().map(|&p| self.map(p)).collect();
        }
        let mut out = Vec::with_capacity(points.len() * 2);
        let edges = if closed { points.len() } else { points.len() - 1 };
        for i in 0..edges {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            let wa = self.map(a);
            out.push(wa);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let n = if len.is_finite() { (len / REFINE_MAX_PIECE_PX).ceil().clamp(1.0, 1e6) as usize } else { 1 };
            let mut prev = (a, wa);
            for k in 1..=n {
                let t = k as f32 / n as f32;
                let q = if k == n { b } else { [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])] };
                let wq = self.map(q);
                self.refine(prev, (q, wq), 0, &mut out);
                if k < n {
                    out.push(wq);
                }
                prev = (q, wq);
            }
        }
        if !closed {
            out.push(self.map(points[points.len() - 1]));
        }
        out
    }

    /// Pushes the mapped points strictly between `a` and `b`, halving while the mapped middle is off the mapped chord
    /// by more than 0.1 px or the mapped piece is longer than 16 px, at most 6 times.
    fn refine(&self, a: ([f32; 2], [f32; 2]), b: ([f32; 2], [f32; 2]), depth: u32, out: &mut Vec<[f32; 2]>) {
        if depth >= REFINE_MAX_DEPTH {
            return;
        }
        let m = [0.5 * (a.0[0] + b.0[0]), 0.5 * (a.0[1] + b.0[1])];
        let wm = self.map(m);
        let chord_mid = [0.5 * (a.1[0] + b.1[0]), 0.5 * (a.1[1] + b.1[1])];
        let off = ((wm[0] - chord_mid[0]).powi(2) + (wm[1] - chord_mid[1]).powi(2)).sqrt();
        let mapped = ((b.1[0] - a.1[0]).powi(2) + (b.1[1] - a.1[1]).powi(2)).sqrt();
        if off > REFINE_TOLERANCE_PX || mapped > REFINE_MAX_MAPPED_PX {
            self.refine(a, (m, wm), depth + 1, out);
            out.push(wm);
            self.refine((m, wm), b, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use rayon::prelude::*;

    use super::*;

    fn warp(style: WarpStyle, bend: f32) -> Warp {
        Warp { style, bend, horizontal: 0.0, vertical: 0.0, vertical_axis: false }
    }

    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    /// A frame centred at (10, -5) with the given width and height.
    fn frame(w: f32, h: f32) -> ([f32; 2], [f32; 2]) {
        ([10.0 - w / 2.0, -5.0 - h / 2.0], [10.0 + w / 2.0, -5.0 + h / 2.0])
    }

    /// TEXT 8.1 `every_style_is_the_identity_at_zero`.
    #[test]
    fn every_style_is_the_identity_at_zero() {
        let (lo, hi) = frame(400.0, 100.0);
        for style in WarpStyle::ALL {
            for vertical_axis in [false, true] {
                let mut w = warp(style, 0.0);
                w.vertical_axis = vertical_axis;
                let map = WarpMap::new(&w, lo, hi);
                for i in 0..9 {
                    for j in 0..9 {
                        let p = [lo[0] + (hi[0] - lo[0]) * i as f32 / 8.0, lo[1] + (hi[1] - lo[1]) * j as f32 / 8.0];
                        assert!(dist(map.map(p), p) < 1e-4, "{style:?} {vertical_axis}");
                    }
                }
            }
        }
        // A frame thinner than 1 px: nothing happens.
        let map = WarpMap::new(&warp(WarpStyle::Flag, 1.0), [0.0, 0.0], [100.0, 0.5]);
        assert_eq!(map.map([30.0, 0.2]), [30.0, 0.2]);
        let map = WarpMap::new(&warp(WarpStyle::Flag, f32::NAN), [0.0, 0.0], [100.0, 50.0]);
        assert_eq!(map.map([30.0, 0.2]), [30.0, 0.2]);
    }

    /// Smallest ratio of mapped to original signed area over the triangles of a 64 x 64 grid of the frame.
    fn smallest_area_ratio(map: &WarpMap) -> f32 {
        const N: usize = 64;
        let (lo, hi) = (map.frame_min, map.frame_max);
        let grid: Vec<[f32; 2]> = (0..=N)
            .flat_map(|j| {
                (0..=N).map(move |i| {
                    [lo[0] + (hi[0] - lo[0]) * i as f32 / N as f32, lo[1] + (hi[1] - lo[1]) * j as f32 / N as f32]
                })
            })
            .collect();
        let mapped: Vec<[f32; 2]> = grid.iter().map(|&p| map.map(p)).collect();
        let area = |p: &[[f32; 2]], a: usize, b: usize, c: usize| {
            0.5 * ((p[b][0] - p[a][0]) * (p[c][1] - p[a][1]) - (p[c][0] - p[a][0]) * (p[b][1] - p[a][1]))
        };
        let mut worst = f32::INFINITY;
        for j in 0..N {
            for i in 0..N {
                let k = j * (N + 1) + i;
                for (a, b, c) in [(k, k + 1, k + N + 2), (k, k + N + 2, k + N + 1)] {
                    worst = worst.min(area(&mapped, a, b, c) / area(&grid, a, b, c));
                }
            }
        }
        worst
    }

    /// TEXT 8.1 `no_style_folds_over`.
    #[test]
    fn no_style_folds_over() {
        let mut cases = Vec::new();
        for style in WarpStyle::ALL {
            for vertical_axis in [false, true] {
                for step in [-4, -3, -2, -1, 1, 2, 3, 4] {
                    for aspect in [0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0] {
                        for horizontal in [-1.0f32, 0.0, 1.0] {
                            for vertical in [-1.0f32, 0.0, 1.0] {
                                cases.push((style, vertical_axis, step as f32 * 0.25, aspect, horizontal, vertical));
                            }
                        }
                    }
                }
            }
        }
        let worst = cases
            .par_iter()
            .map(|&(style, vertical_axis, bend, aspect, horizontal, vertical)| {
                let w = Warp { style, bend, horizontal, vertical, vertical_axis };
                let (lo, hi) = frame(100.0 * aspect, 100.0);
                let ratio = smallest_area_ratio(&WarpMap::new(&w, lo, hi));
                (ratio, format!("{style:?} vertical {vertical_axis} bend {bend} aspect {aspect} distortions {horizontal} {vertical}"))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap();
        assert!(worst.0 > 0.05, "folds: {} at {}", worst.0, worst.1);
    }

    /// TEXT 8.1 `arc_at_full_bend_is_a_half_circle`.
    #[test]
    fn arc_at_full_bend_is_a_half_circle() {
        let (lo, hi) = frame(800.0, 100.0);
        let map = WarpMap::new(&warp(WarpStyle::Arc, 1.0), lo, hi);
        for x in [lo[0], hi[0]] {
            let top = map.map([x, lo[1]]);
            let bottom = map.map([x, hi[1]]);
            assert!((top[1] - bottom[1]).abs() < 0.01, "{top:?} {bottom:?}");
            assert!((top[0] - bottom[0]).abs() > 50.0);
        }
        // The middle of the top edge is the highest point: a half circle opening downwards.
        let mid = map.map([10.0, lo[1]]);
        let left = map.map([lo[0], lo[1]]);
        assert!(mid[1] < left[1] - 100.0);
    }

    /// Length of the mapped horizontal line at `y` across the frame.
    fn mapped_length(map: &WarpMap, y: f32) -> f32 {
        let n = 2000;
        let mut len = 0.0;
        let mut prev = map.map([map.frame_min[0], y]);
        for i in 1..=n {
            let x = map.frame_min[0] + (map.frame_max[0] - map.frame_min[0]) * i as f32 / n as f32;
            let p = map.map([x, y]);
            len += dist(p, prev);
            prev = p;
        }
        len
    }

    /// TEXT 8.1 `arc_keeps_the_middle_line`.
    #[test]
    fn arc_keeps_the_middle_line() {
        let (lo, hi) = frame(400.0, 100.0);
        let map = WarpMap::new(&warp(WarpStyle::Arc, 0.5), lo, hi);
        let len = mapped_length(&map, -5.0);
        assert!((len / 400.0 - 1.0).abs() < 0.005, "{len}");
    }

    /// TEXT 8.1 `arc_inner_edge_keeps_a_quarter`.
    #[test]
    fn arc_inner_edge_keeps_a_quarter() {
        let (lo, hi) = frame(100.0, 200.0);
        for bend in [1.0f32, -1.0] {
            let map = WarpMap::new(&warp(WarpStyle::Arc, bend), lo, hi);
            // The inner edge is the bottom for a positive bend, the top for a negative one.
            let inner = if bend > 0.0 { hi[1] } else { lo[1] };
            let len = mapped_length(&map, inner);
            assert!(len >= 0.249 * 100.0, "{bend}: {len}");
        }
    }

    /// The frame border of `frame(300, 80)`: top, bottom, left, right, 51 points each.
    fn border_300_80() -> ([f32; 2], [f32; 2], [Vec<[f32; 2]>; 4]) {
        let (lo, hi) = frame(300.0, 80.0);
        let border = |f: &dyn Fn(f32) -> [f32; 2]| (0..=50).map(|i| f(i as f32 / 50.0)).collect::<Vec<_>>();
        let top = border(&|t| [lo[0] + t * (hi[0] - lo[0]), lo[1]]);
        let bottom = border(&|t| [lo[0] + t * (hi[0] - lo[0]), hi[1]]);
        let left = border(&|t| [lo[0], lo[1] + t * (hi[1] - lo[1])]);
        let right = border(&|t| [hi[0], lo[1] + t * (hi[1] - lo[1])]);
        (lo, hi, [top, bottom, left, right])
    }

    /// TEXT 8.1 `arc_lower_keeps_the_top_edge`.
    #[test]
    fn arc_lower_keeps_the_top_edge() {
        let (lo, hi, [top, bottom, _, _]) = border_300_80();
        for bend in [-1.0f32, -0.5, 0.5, 1.0] {
            let lower = WarpMap::new(&warp(WarpStyle::ArcLower, bend), lo, hi);
            for &p in &top {
                assert!(dist(lower.map(p), p) < 1e-4);
            }
            // ... and the other edge moves.
            assert!(dist(lower.map(bottom[25]), bottom[25]) > 1.0);
        }
    }

    /// TEXT 8.1 `arc_upper_keeps_the_bottom_edge`.
    #[test]
    fn arc_upper_keeps_the_bottom_edge() {
        let (lo, hi, [top, bottom, _, _]) = border_300_80();
        for bend in [-1.0f32, -0.5, 0.5, 1.0] {
            let upper = WarpMap::new(&warp(WarpStyle::ArcUpper, bend), lo, hi);
            for &p in &bottom {
                assert!(dist(upper.map(p), p) < 1e-4);
            }
            assert!(dist(upper.map(top[25]), top[25]) > 1.0);
        }
    }

    /// TEXT 8.1 `fisheye_and_twist_keep_the_frame_border`.
    #[test]
    fn fisheye_and_twist_keep_the_frame_border() {
        let (lo, hi, [top, bottom, left, right]) = border_300_80();
        for bend in [-1.0f32, -0.5, 0.5, 1.0] {
            for style in [WarpStyle::Fisheye, WarpStyle::Twist] {
                let map = WarpMap::new(&warp(style, bend), lo, hi);
                for &p in top.iter().chain(&bottom).chain(&left).chain(&right) {
                    assert!(dist(map.map(p), p) < 1e-4, "{style:?} {bend} {p:?}");
                }
                assert!(dist(map.map([10.0, -5.0 + 20.0]), [10.0, 15.0]) > 0.5, "{style:?} moves the inside");
            }
        }
    }

    /// Wave, Fish, Inflate and Squeeze against the reference script (`warp_check.py`, `style_point`, frame half sizes
    /// 100 x 50; review of T1): (style, point, bend, vertical axis, expected), relative to the frame centre.
    #[test]
    fn wave_fish_inflate_squeeze_match_the_reference() {
        /// (style, point, bend, vertical axis, expected).
        type Case = (WarpStyle, [f32; 2], f32, bool, [f32; 2]);
        let cases: [Case; 12] = [
            (WarpStyle::Wave, [50.0, -25.0], 1.0, false, [40.0, -50.0]),
            (WarpStyle::Wave, [-30.0, 40.0], -0.6, false, [-33.52671, 27.86475]),
            (WarpStyle::Wave, [50.0, -25.0], 0.8, true, [10.0, -29.0]),
            (WarpStyle::Fish, [50.0, -25.0], 1.0, false, [50.0, -37.5]),
            (WarpStyle::Fish, [-30.0, 40.0], -0.6, false, [-30.0, 49.7082]),
            (WarpStyle::Fish, [50.0, -25.0], 0.8, true, [30.0, -25.0]),
            (WarpStyle::Inflate, [50.0, -25.0], 1.0, false, [59.375, -34.375]),
            (WarpStyle::Inflate, [-30.0, 40.0], -0.6, false, [-28.38, 29.08]),
            (WarpStyle::Inflate, [50.0, -25.0], 0.8, true, [65.0, -28.75]),
            (WarpStyle::Squeeze, [50.0, -25.0], 1.0, false, [31.25, -34.375]),
            (WarpStyle::Squeeze, [-30.0, 40.0], -0.6, false, [-33.24, 29.08]),
            (WarpStyle::Squeeze, [50.0, -25.0], 0.8, true, [65.0, -17.5]),
        ];
        let centre = [7.0f32, -3.0f32];
        for (style, p, bend, vertical_axis, expected) in cases {
            let mut w = warp(style, bend);
            w.vertical_axis = vertical_axis;
            let map = WarpMap::new(&w, [centre[0] - 100.0, centre[1] - 50.0], [centre[0] + 100.0, centre[1] + 50.0]);
            let got = map.map([centre[0] + p[0], centre[1] + p[1]]);
            let want = [centre[0] + expected[0], centre[1] + expected[1]];
            assert!(dist(got, want) < 1e-3, "{style:?} {p:?} {bend} {vertical_axis}: {got:?} vs {want:?}");
        }
    }

    /// TEXT 8.1 `distortion_scales_the_sides`.
    #[test]
    fn distortion_scales_the_sides() {
        let (lo, hi) = frame(300.0, 80.0);
        let w = Warp { style: WarpStyle::Arc, bend: 0.0, horizontal: 1.0, vertical: 0.0, vertical_axis: false };
        let map = WarpMap::new(&w, lo, hi);
        let height = |x: f32| map.map([x, hi[1]])[1] - map.map([x, lo[1]])[1];
        assert!((height(hi[0]) / 80.0 - 1.4).abs() < 0.001, "{}", height(hi[0]));
        assert!((height(lo[0]) / 80.0 - 0.6).abs() < 0.001, "{}", height(lo[0]));
        // Vertical distortion: the bottom edge is 1.4 times, the top edge 0.6 times as wide.
        let w = Warp { horizontal: 0.0, vertical: 1.0, ..w };
        let map = WarpMap::new(&w, lo, hi);
        let width = |y: f32| map.map([hi[0], y])[0] - map.map([lo[0], y])[0];
        assert!((width(hi[1]) / 300.0 - 1.4).abs() < 0.001);
        assert!((width(lo[1]) / 300.0 - 0.6).abs() < 0.001);
    }

    /// TEXT 8.1 `vertical_axis_is_the_turned_style`: the vertical direction on a frame is the horizontal style on the
    /// turned frame, turned back.
    #[test]
    fn vertical_axis_is_the_turned_style() {
        let (a, h) = (150.0f32, 40.0f32);
        for style in WarpStyle::ALL {
            for bend in [-0.7f32, 0.4, 1.0] {
                let mut w = warp(style, bend);
                w.vertical_axis = true;
                let vertical = WarpMap::new(&w, [-a, -h], [a, h]);
                w.vertical_axis = false;
                let turned = WarpMap::new(&w, [-h, -a], [h, a]);
                // R(x, y) = (y, -x) turns the frame of half sizes (a, h) into one of (h, a).
                for i in 0..=10 {
                    for j in 0..=10 {
                        let p = [-a + 2.0 * a * i as f32 / 10.0, -h + 2.0 * h * j as f32 / 10.0];
                        let q = turned.map([p[1], -p[0]]);
                        let back = [-q[1], q[0]];
                        assert!(dist(vertical.map(p), back) < 1e-3, "{style:?} {bend} {p:?}");
                    }
                }
                if bend == 1.0 && style == WarpStyle::Flag {
                    // A vertical flag moves points sideways, not up and down.
                    let m = vertical.map([0.0, -h / 2.0]);
                    assert!((m[0]).abs() > 1.0 && (m[1] + h / 2.0).abs() < 1e-3, "{m:?}");
                }
            }
        }
    }

    /// TEXT 8.1 `refinement_follows_the_curve`.
    #[test]
    fn refinement_follows_the_curve() {
        let (lo, hi) = frame(400.0, 100.0);
        let map = WarpMap::new(&warp(WarpStyle::Arc, 1.0), lo, hi);
        // A straight edge of 200 px on the top edge of the frame.
        let (a, b) = ([lo[0] + 100.0, lo[1]], [lo[0] + 300.0, lo[1]]);
        let line = map.map_polyline(&[a, b], false);
        assert_eq!(line.first(), Some(&map.map(a)));
        assert_eq!(line.last(), Some(&map.map(b)));
        let longest = line.windows(2).map(|w| dist(w[0], w[1])).fold(0.0f32, f32::max);
        assert!(longest <= 16.0, "{longest}");
        let seg_dist = |p: [f32; 2]| {
            line.windows(2)
                .map(|w| {
                    let (s, e) = (w[0], w[1]);
                    let d = [e[0] - s[0], e[1] - s[1]];
                    let l2 = d[0] * d[0] + d[1] * d[1];
                    let t = if l2 > 0.0 { (((p[0] - s[0]) * d[0] + (p[1] - s[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
                    dist(p, [s[0] + t * d[0], s[1] + t * d[1]])
                })
                .fold(f32::INFINITY, f32::min)
        };
        let worst = (0..1000)
            .map(|i| {
                let t = i as f32 / 999.0;
                seg_dist(map.map([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]))
            })
            .fold(0.0f32, f32::max);
        assert!(worst <= 0.15, "{worst}");
        // A closed square: closed output without the first point repeated; no warp: the points themselves.
        let square = [[0.0, -50.0], [40.0, -50.0], [40.0, 0.0], [0.0, 0.0]];
        let closed = map.map_polyline(&square, true);
        assert!(closed.len() > 4 && closed.first() != closed.last());
        let plain = WarpMap::new(&warp(WarpStyle::Arc, 0.0), lo, hi);
        assert_eq!(plain.map_polyline(&square, true), square.to_vec());
    }

    #[test]
    fn styles_bend_the_way_their_names_say() {
        let (lo, hi) = frame(400.0, 100.0);
        let up = |style: WarpStyle, p: [f32; 2]| p[1] - WarpMap::new(&warp(style, 1.0), lo, hi).map(p)[1];
        let mid_top = [10.0, lo[1]];
        let mid_bottom = [10.0, hi[1]];
        assert!(up(WarpStyle::Arch, mid_top) > 10.0 && up(WarpStyle::Arch, mid_bottom) > 10.0);
        assert!(up(WarpStyle::Bulge, mid_top) > 10.0 && up(WarpStyle::Bulge, mid_bottom) < -10.0);
        assert!(up(WarpStyle::Rise, [hi[0], -5.0]) > 10.0 && up(WarpStyle::Rise, [lo[0], -5.0]) < -10.0);
        assert!(up(WarpStyle::ShellUpper, [hi[0], lo[1]]) > 10.0);
        assert!(up(WarpStyle::ShellLower, [hi[0], hi[1]]) < -10.0);
    }
}
