// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Vector shapes and coverage (STICKER_PLAN 4.8; ARCHITECTURE 6.11). Coverage of a pixel = `clamp(0.5 - sd, 0, 1)`
//! with `sd` the signed distance (pixels, negative inside) from the pixel centre to the shape's edge: rectangle and
//! rounded rectangle exact; ellipse by the first-order estimate `(f - 1) / |grad f|`; polygons and stars as the
//! distance to the edge list with the non-zero rule; a line as a capsule. An outline is
//! `clamp(width / 2 + 0.5 - |sd|, 0, 1)`, centred on the edge.

use glam::Vec2;
use rayon::prelude::*;

use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf};
use crate::sticker::style::Gradient;

/// Rows with at least this many pixels in total are rasterized in parallel.
const PAR_PIXELS: usize = 1 << 16;
/// Largest raster side of a shape (pixels).
const MAX_SHAPE_SIDE: f32 = 8192.0;

/// The geometry of a shape layer; it fills its content box `size`.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeKind {
    Rect,
    RoundedRect { radius: f32 },
    Ellipse,
    /// Regular polygon inscribed in the box's ellipse, first corner at the top; 3..=12 sides.
    Polygon { sides: u32 },
    /// Star inscribed in the box's ellipse, first point at the top; 3..=24 points, inner radius 0.1..=0.95 of the outer.
    Star { points: u32, inner: f32 },
    /// A horizontal capsule through the middle of the box, `width` thick, round caps inside the box.
    Line { width: f32 },
}

/// How a shape is filled.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    None,
    Solid([u8; 4]),
    /// A linear gradient across the content box (4.3a); `angle_deg` counter-clockwise from +x, y down.
    Linear { gradient: Gradient, angle_deg: f32 },
}

/// A shape layer's content (STICKER_PLAN 2.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeSpec {
    pub kind: ShapeKind,
    /// Content size in pixels at render scale 1.
    pub size: Vec2,
    pub fill: Paint,
    /// Colour and width (centred on the edge).
    pub outline: Option<([u8; 4], f32)>,
}

impl ShapeSpec {
    /// Clamps the numbers into their ranges: size 1..=8192 per axis, sides 3..=12, points 3..=24, inner 0.1..=0.95,
    /// radius and widths 0..=half the box (non-finite values become the smallest legal value), gradients sanitized.
    pub fn sanitize(&mut self) {
        let fin = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { lo };
        self.size = Vec2::new(fin(self.size.x, 1.0, MAX_SHAPE_SIDE), fin(self.size.y, 1.0, MAX_SHAPE_SIDE));
        let half = self.size.x.min(self.size.y) * 0.5;
        match &mut self.kind {
            ShapeKind::RoundedRect { radius } => *radius = fin(*radius, 0.0, half),
            ShapeKind::Polygon { sides } => *sides = (*sides).clamp(3, 12),
            ShapeKind::Star { points, inner } => {
                *points = (*points).clamp(3, 24);
                *inner = fin(*inner, 0.1, 0.95);
            }
            ShapeKind::Line { width } => *width = fin(*width, 0.0, self.size.y),
            ShapeKind::Rect | ShapeKind::Ellipse => {}
        }
        if let Paint::Linear { gradient, angle_deg } = &mut self.fill {
            gradient.sanitize();
            *angle_deg = if angle_deg.is_finite() { angle_deg.clamp(-3600.0, 3600.0) } else { 0.0 };
        }
        if let Some((_, w)) = &mut self.outline {
            *w = fin(*w, 0.0, 256.0);
        }
    }
}

/// Exact signed distance to the box `[0, size]` (centre `c`, half extents `b`), rounded by `r`.
pub(crate) fn sd_round_box(p: Vec2, c: Vec2, b: Vec2, r: f32) -> f32 {
    let q = (p - c).abs() - b + Vec2::splat(r);
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - r
}

/// First-order signed distance to the ellipse with centre `c` and semi-axes `ab`.
pub(crate) fn sd_ellipse(p: Vec2, c: Vec2, ab: Vec2) -> f32 {
    let d = p - c;
    let f = ((d.x / ab.x).powi(2) + (d.y / ab.y).powi(2)).sqrt();
    if f <= 1.0e-6 {
        return -ab.x.min(ab.y);
    }
    let g = Vec2::new(d.x / (ab.x * ab.x * f), d.y / (ab.y * ab.y * f));
    (f - 1.0) / g.length().max(1.0e-12)
}

/// Distance from `p` to the segment `a`-`b`.
fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

/// The corner list of a polygon or star kind in a box of `size` (scaled), or `None` for the analytic kinds.
pub fn shape_points(kind: &ShapeKind, size: Vec2) -> Option<Vec<Vec2>> {
    let c = size * 0.5;
    let at = |angle: f32, k: f32| c + Vec2::new(angle.cos() * c.x * k, angle.sin() * c.y * k);
    let start = -std::f32::consts::FRAC_PI_2;
    let tau = std::f32::consts::TAU;
    match *kind {
        ShapeKind::Polygon { sides } => {
            let n = sides.clamp(3, 12);
            Some((0..n).map(|k| at(start + tau * k as f32 / n as f32, 1.0)).collect())
        }
        ShapeKind::Star { points, inner } => {
            let n = points.clamp(3, 24);
            let inner = if inner.is_finite() { inner.clamp(0.1, 0.95) } else { 0.5 };
            Some((0..2 * n).map(|k| at(start + tau * k as f32 / (2 * n) as f32, if k % 2 == 0 { 1.0 } else { inner })).collect())
        }
        _ => None,
    }
}

/// Signed distances of one row of pixel centres to a polygon (non-zero rule), for the pixels `x0..x0 + n` of row
/// `yc` (pixel-centre y). Distances below `reach` (at least 1.5) are exact; farther pixels report `reach` or more
/// (`+-1e6`). The fill needs 1.5; a centred outline of width `w` needs `w / 2 + 1` (checker P1: with a fixed band of
/// 1.5 px a thick polygon or star outline was cut to about 2 px).
fn polygon_row(points: &[Vec2], yc: f32, x0: i32, n: usize, reach: f32, out: &mut [f32]) {
    let reach = if reach.is_finite() { reach.max(1.5) } else { 1.5 };
    out.fill(f32::INFINITY);
    let m = points.len();
    // Crossings of the scan line (half-open in y), with direction.
    let mut cross: Vec<(f32, i32)> = Vec::new();
    for i in 0..m {
        let (a, b) = (points[i], points[(i + 1) % m]);
        if (a.y <= yc) != (b.y <= yc) {
            let t = (yc - a.y) / (b.y - a.y);
            cross.push((a.x + (b.x - a.x) * t, if b.y > a.y { 1 } else { -1 }));
        }
        // Exact distances for pixels near this edge: the part of the edge within `reach` of the row.
        let (ylo, yhi) = (yc - reach, yc + reach);
        let (lo, hi) = if a.y == b.y {
            if a.y < ylo || a.y > yhi {
                continue;
            }
            (a.x.min(b.x), a.x.max(b.x))
        } else {
            let (t0, t1) = ((ylo - a.y) / (b.y - a.y), (yhi - a.y) / (b.y - a.y));
            let (t0, t1) = (t0.min(t1).max(0.0), t0.max(t1).min(1.0));
            if t0 > t1 {
                continue;
            }
            let (xa, xb) = (a.x + (b.x - a.x) * t0, a.x + (b.x - a.x) * t1);
            (xa.min(xb), xa.max(xb))
        };
        let first = ((lo - reach).floor() as i64 - x0 as i64).max(0) as usize;
        let last = ((hi + reach).ceil() as i64 - x0 as i64).min(n as i64 - 1);
        if last < first as i64 {
            continue;
        }
        for (k, o) in out.iter_mut().enumerate().take(last as usize + 1).skip(first) {
            let p = Vec2::new((x0 + k as i32) as f32 + 0.5, yc);
            *o = o.min(seg_dist(p, a, b));
        }
    }
    cross.sort_by(|p, q| p.0.total_cmp(&q.0));
    let mut ci = 0;
    let mut winding = 0;
    for (k, o) in out.iter_mut().enumerate() {
        let xc = (x0 + k as i32) as f32 + 0.5;
        while ci < cross.len() && cross[ci].0 < xc {
            winding += cross[ci].1;
            ci += 1;
        }
        let d = o.min(1.0e6);
        *o = if winding != 0 { -d.max(0.0) } else { d.max(0.0) };
        if !d.is_finite() || d >= 1.0e6 {
            *o = if winding != 0 { -1.0e6 } else { 1.0e6 };
        }
    }
}

/// Coverage of a polygon (non-zero rule, edge distance; lasso selections and polygon shapes) on a `width x height`
/// grid, pixel centres at `+0.5`.
pub fn polygon_coverage(points: &[Vec2], width: u32, height: u32) -> AlphaBuf {
    let mut out = AlphaBuf::new(width, height);
    let w = width as usize;
    if points.len() < 3 || w == 0 || height == 0 || points.iter().any(|p| !p.is_finite()) {
        return out;
    }
    let do_row = |row_buf: &mut Vec<f32>, (y, row): (usize, &mut [u8])| {
        row_buf.resize(w, 0.0);
        polygon_row(points, y as f32 + 0.5, 0, w, 1.5, row_buf);
        for (o, &sd) in row.iter_mut().zip(row_buf.iter()) {
            *o = cov(0.5 - sd);
        }
    };
    if out.data.len() >= PAR_PIXELS {
        out.data.par_chunks_mut(w).enumerate().for_each_init(Vec::new, do_row);
    } else {
        let mut b = Vec::new();
        out.data.chunks_mut(w).enumerate().for_each(|r| do_row(&mut b, r));
    }
    out
}

/// `clamp(v, 0, 1)` as a byte, rounded half up.
#[inline]
fn cov(v: f32) -> u8 {
    if v >= 1.0 {
        255
    } else if v > 0.0 {
        (v * 255.0 + 0.5) as u8
    } else {
        0
    }
}

/// Straight-alpha source-over of `c` with coverage `k` (0..=1).
#[inline]
fn over(d: &mut [u8], c: [u8; 4], k: f32) {
    let sa = k * c[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = d[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    let a8 = (oa * 255.0 + 0.5) as u8;
    if a8 == 0 {
        return;
    }
    for i in 0..3 {
        d[i] = ((c[i] as f32 * sa + d[i] as f32 * da * (1.0 - sa)) / oa + 0.5).clamp(0.0, 255.0) as u8;
    }
    d[3] = a8;
}

/// Draws the shape at render scale `scale` (4.8): a cropped buffer in content pixels at that scale (the content box
/// is `[0, size * scale)`); the rect grows by half the outline width (plus one pixel) on every side.
pub fn rasterize(spec: &ShapeSpec, scale: f32) -> CroppedBuf {
    let mut spec = spec.clone();
    spec.sanitize();
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let size = (spec.size * scale).min(Vec2::splat(MAX_SHAPE_SIDE));
    let outline = spec.outline.map(|(c, w)| (c, w * scale)).filter(|(c, w)| *w > 0.0 && c[3] > 0);
    let pad = outline.map_or(1.0, |(_, w)| w * 0.5 + 1.0);
    // Exact signed distances are needed out to the outline's half width (plus the anti-aliasing pixel).
    let reach = outline.map_or(1.5, |(_, w)| w * 0.5 + 1.0);
    let rect = IRect::new((-pad).floor() as i32, (-pad).floor() as i32, (size.x + pad).ceil() as i32, (size.y + pad).ceil() as i32);
    let mut out = CroppedBuf::new(rect);
    let rw = rect.width() as usize;
    if rw == 0 {
        return out;
    }
    let c = size * 0.5;
    let points = shape_points(&spec.kind, size);
    let kind = spec.kind.clone();
    let line_w = if let ShapeKind::Line { width } = kind { width * scale } else { 0.0 };
    let radius = if let ShapeKind::RoundedRect { radius } = kind { radius * scale } else { 0.0 };
    // Fill colour per pixel: solid, or the gradient across the content box's pixel centres (4.3a).
    let fill = spec.fill.clone();
    let (dir, half) = match &fill {
        Paint::Linear { angle_deg, .. } => {
            let (s, co) = angle_deg.to_radians().sin_cos();
            ([co, -s], ((size.x - 1.0).max(0.0) * co.abs() + (size.y - 1.0).max(0.0) * s.abs()) * 0.5)
        }
        _ => ([1.0, 0.0], 0.0),
    };
    let do_row = |sd_row: &mut Vec<f32>, (ry, row): (usize, &mut [u8])| {
        let y = rect.y0 + ry as i32;
        let yc = y as f32 + 0.5;
        sd_row.resize(rw, 0.0);
        match &points {
            Some(pts) => polygon_row(pts, yc, rect.x0, rw, reach, sd_row),
            None => {
                for (k, o) in sd_row.iter_mut().enumerate() {
                    let p = Vec2::new((rect.x0 + k as i32) as f32 + 0.5, yc);
                    *o = match kind {
                        ShapeKind::Rect => sd_round_box(p, c, c, 0.0),
                        ShapeKind::RoundedRect { .. } => sd_round_box(p, c, c, radius.min(c.x).min(c.y)),
                        ShapeKind::Ellipse => sd_ellipse(p, c, c),
                        ShapeKind::Line { .. } => {
                            let r = line_w * 0.5;
                            let (a, b) = (Vec2::new(r.min(c.x), c.y), Vec2::new((size.x - r).max(c.x), c.y));
                            seg_dist(p, a, b) - r
                        }
                        _ => f32::INFINITY,
                    };
                }
            }
        }
        for (k, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let sd = sd_row[k];
            let p = Vec2::new((rect.x0 + k as i32) as f32 + 0.5, yc);
            let fc = cov(0.5 - sd);
            if fc > 0 {
                let color = match &fill {
                    Paint::None => None,
                    Paint::Solid(c) => Some(*c),
                    Paint::Linear { gradient, .. } => {
                        let (cx, cy) = (p.x - 0.5 - (size.x - 1.0) * 0.5, p.y - 0.5 - (size.y - 1.0) * 0.5);
                        let t = if half > 0.0 { 0.5 + (cx * dir[0] + cy * dir[1]) / (2.0 * half) } else { 0.5 };
                        Some(gradient.sample(t))
                    }
                };
                if let Some(col) = color {
                    over(px, col, fc as f32 / 255.0);
                }
            }
            if let Some((oc, ow)) = outline {
                let k = cov(ow * 0.5 + 0.5 - sd.abs());
                if k > 0 {
                    over(px, oc, k as f32 / 255.0);
                }
            }
        }
    };
    if out.data.len() / 4 >= PAR_PIXELS {
        out.data.par_chunks_mut(rw * 4).enumerate().for_each_init(Vec::new, do_row);
    } else {
        let mut b = Vec::new();
        out.data.chunks_mut(rw * 4).enumerate().for_each(|r| do_row(&mut b, r));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(kind: ShapeKind, w: f32, h: f32) -> ShapeSpec {
        ShapeSpec { kind, size: Vec2::new(w, h), fill: Paint::Solid([255, 0, 0, 255]), outline: None }
    }

    /// Sum of alpha / 255 over the raster.
    fn area(buf: &CroppedBuf) -> f64 {
        buf.data.as_chunks::<4>().0.iter().map(|p| p[3] as f64 / 255.0).sum()
    }

    fn shoelace(p: &[Vec2]) -> f64 {
        let n = p.len();
        (0..n).map(|i| (p[i].x as f64) * (p[(i + 1) % n].y as f64) - (p[(i + 1) % n].x as f64) * (p[i].y as f64)).sum::<f64>().abs()
            * 0.5
    }

    #[test]
    fn k1_shape_areas() {
        // Ellipse 80 x 50: pi * 40 * 25.
        let e = rasterize(&solid(ShapeKind::Ellipse, 80.0, 50.0), 1.0);
        let want = std::f64::consts::PI * 40.0 * 25.0;
        assert!((area(&e) - want).abs() / want < 0.005, "ellipse {} vs {want}", area(&e));
        // The same at render scale 2.5 (an ellipse 200 x 125).
        let e = rasterize(&solid(ShapeKind::Ellipse, 80.0, 50.0), 2.5);
        let want = std::f64::consts::PI * 100.0 * 62.5;
        assert!((area(&e) - want).abs() / want < 0.005, "ellipse x2.5 {} vs {want}", area(&e));
        // Star of 5 points, inner 0.45, in a 90 x 90 box: the shoelace area of its corners.
        let kind = ShapeKind::Star { points: 5, inner: 0.45 };
        let s = rasterize(&solid(kind.clone(), 90.0, 90.0), 1.0);
        let want = shoelace(&shape_points(&kind, Vec2::new(90.0, 90.0)).unwrap());
        assert!((area(&s) - want).abs() / want < 0.005, "star {} vs {want}", area(&s));
        // Hexagon, a rectangle with fractional size, a rounded rectangle, a line capsule.
        let kind = ShapeKind::Polygon { sides: 6 };
        let h = rasterize(&solid(kind.clone(), 70.0, 60.0), 1.0);
        let want = shoelace(&shape_points(&kind, Vec2::new(70.0, 60.0)).unwrap());
        assert!((area(&h) - want).abs() / want < 0.005, "hexagon {} vs {want}", area(&h));
        let r = rasterize(&solid(ShapeKind::Rect, 30.5, 20.0), 1.0);
        assert!((area(&r) - 610.0).abs() < 0.5, "rect {}", area(&r));
        let rr = rasterize(&solid(ShapeKind::RoundedRect { radius: 10.0 }, 60.0, 40.0), 1.0);
        let want = 60.0 * 40.0 - (4.0 - std::f64::consts::PI) * 100.0;
        assert!((area(&rr) - want).abs() / want < 0.005, "rounded {} vs {want}", area(&rr));
        let l = rasterize(&solid(ShapeKind::Line { width: 8.0 }, 100.0, 20.0), 1.0);
        let want = 92.0 * 8.0 + std::f64::consts::PI * 16.0;
        assert!((area(&l) - want).abs() / want < 0.005, "line {} vs {want}", area(&l));
    }

    #[test]
    fn k1_shape_outline_fill_and_gradient() {
        // A centred outline of width 4 around a 40 x 40 rectangle with no fill: ring area (44^2 - 36^2) minus the
        // rounded outer corners (4 * (4 - pi) for radius 2).
        let spec = ShapeSpec {
            kind: ShapeKind::Rect,
            size: Vec2::new(40.0, 40.0),
            fill: Paint::None,
            outline: Some(([0, 0, 0, 255], 4.0)),
        };
        let o = rasterize(&spec, 1.0);
        let want = 44.0 * 44.0 - 36.0 * 36.0 - (4.0 - std::f64::consts::PI) * 4.0;
        assert!((area(&o) - want).abs() / want < 0.01, "outline {} vs {want}", area(&o));
        assert!(o.rect.x0 <= -2 && o.rect.x1 >= 42);
        // Gradient fill: the first and last columns of the box are the stops exactly.
        let g = Gradient::two([0, 0, 255, 255], [255, 255, 0, 255]);
        let spec = ShapeSpec {
            kind: ShapeKind::Rect,
            size: Vec2::new(16.0, 8.0),
            fill: Paint::Linear { gradient: g, angle_deg: 0.0 },
            outline: None,
        };
        let r = rasterize(&spec, 1.0);
        assert_eq!(r.get(0, 3), [0, 0, 255, 255]);
        assert_eq!(r.get(15, 3), [255, 255, 0, 255]);
        assert_eq!(r.get(-1, 3)[3], 0);
        // Deterministic and thread-count independent.
        let big = solid(ShapeKind::Star { points: 7, inner: 0.4 }, 600.0, 500.0);
        let pools = [1usize, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
        let a = pools[0].install(|| rasterize(&big, 1.0));
        let b = pools[1].install(|| rasterize(&big, 1.0));
        assert_eq!(a, b);
    }

    #[test]
    fn k1_thick_polygon_and_star_outlines_follow_the_edge_distance() {
        // Checker P1: an outline wider than the 1.5-px exact band used for the fill must still be
        // `clamp(width / 2 + 0.5 - |sd|)` against the true distance to the edge list (brute force here).
        for (kind, ow) in [
            (ShapeKind::Polygon { sides: 6 }, 10.0f32),
            (ShapeKind::Star { points: 5, inner: 0.45 }, 8.0),
            (ShapeKind::Polygon { sides: 3 }, 24.0),
        ] {
            let size = Vec2::new(90.0, 80.0);
            let spec = ShapeSpec { kind: kind.clone(), size, fill: Paint::None, outline: Some(([0, 0, 0, 255], ow)) };
            let r = rasterize(&spec, 1.0);
            let pts = shape_points(&kind, size).unwrap();
            let mut worst = 0i32;
            for y in r.rect.y0..r.rect.y1 {
                for x in r.rect.x0..r.rect.x1 {
                    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let d = (0..pts.len()).map(|i| seg_dist(p, pts[i], pts[(i + 1) % pts.len()])).fold(f32::INFINITY, f32::min);
                    let want = cov(ow * 0.5 + 0.5 - d) as i32;
                    worst = worst.max((r.get(x, y)[3] as i32 - want).abs());
                }
            }
            assert!(worst <= 1, "{kind:?} outline {ow}: off by {worst} levels");
        }
    }

    #[test]
    fn k1_polygon_coverage_rules() {
        // A square drawn twice in the same direction (winding 2) is filled (non-zero); a self-overlapping bow tie
        // has both lobes; a degenerate polygon gives nothing.
        let sq = [Vec2::new(2.0, 2.0), Vec2::new(12.0, 2.0), Vec2::new(12.0, 12.0), Vec2::new(2.0, 12.0)];
        let twice: Vec<Vec2> = sq.iter().chain(sq.iter()).copied().collect();
        let a = polygon_coverage(&sq, 16, 16);
        let b = polygon_coverage(&twice, 16, 16);
        assert_eq!(a, b);
        assert_eq!(a.get(5, 5), 255);
        assert_eq!(a.get(0, 0), 0);
        let sum: u32 = a.data.iter().map(|&v| v as u32).sum();
        assert!((sum as f64 / 255.0 - 100.0).abs() < 0.5);
        let bow = [Vec2::new(0.0, 0.0), Vec2::new(20.0, 10.0), Vec2::new(20.0, 0.0), Vec2::new(0.0, 10.0)];
        let c = polygon_coverage(&bow, 20, 10);
        assert_eq!(c.get(2, 5), 255);
        assert_eq!(c.get(17, 5), 255);
        assert_eq!(c.get(10, 1), 0);
        assert!(polygon_coverage(&sq[..2], 16, 16).data.iter().all(|&v| v == 0));
        assert!(polygon_coverage(&[Vec2::NAN; 3], 4, 4).data.iter().all(|&v| v == 0));
    }
}
