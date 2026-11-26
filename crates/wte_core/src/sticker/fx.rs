// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Distances, the tent blur and the layer styles (STICKER_PLAN 4.2, 4.3; ARCHITECTURE 6.11).
//!
//! `apply_styles(base, styles, scale)` draws a `StyleStack` around a straight RGBA8 picture. All fields are computed on
//! the base padded by `p = max(margin, 1)` transparent pixels, so the picture's own edge is an edge even when it is
//! fully opaque (review A7), and cropped to the margin at the end. Working memory is kept small (8.4: 256 MB at 4096 x
//! 4096): one-byte planes, at most one `f32` distance field alive at a time, each plane dropped as soon as it is drawn.
//!
//! Deliberate differences from PhotoCraft / Photoshop: strokes use the Euclidean distance (round corners) instead of a
//! 5 x 5 chamfer metric; shadows and outer glows are cast by the whole outline shape (artwork + contour + outside
//! stroke); no per-style blend modes (everything source-over); no contour curves, noise or knockout.

use rayon::prelude::*;

use crate::math::mul_u8;
use crate::pixel::PixelBuf;
use crate::sticker::style::{Contour, Glow, Shadow, StrokePos, StyleStack, light_offset};
use crate::text::edt::distance_to_ink;

/// Planes with at least this many pixels are processed in parallel.
const PAR_PIXELS: usize = 1 << 16;
/// Widest tent box (pixels); far beyond any sanitized length at a sane scale, keeps the running sums in `u32`.
const MAX_BOX: u32 = 16_384;

/// A value a tent blur can run over: `u8` coverage or `u16` heights.
pub trait TentCell: Copy + Default + Send + Sync {
    /// The value as an integer.
    fn get(self) -> u32;
    /// An integer (already in range) as the value.
    fn put(v: u32) -> Self;
}

impl TentCell for u8 {
    #[inline]
    fn get(self) -> u32 {
        self as u32
    }
    #[inline]
    fn put(v: u32) -> u8 {
        v.min(255) as u8
    }
}

impl TentCell for u16 {
    #[inline]
    fn get(self) -> u32 {
        self as u32
    }
    #[inline]
    fn put(v: u32) -> u16 {
        v.min(65_535) as u16
    }
}

/// One box pass along a line: `dst[i] = round(sum(src[i - lo ..= i + hi]) / (lo + hi + 1))`, outside the line 0.
fn box_line<T: TentCell>(src: &[T], dst: &mut [T], lo: usize, hi: usize) {
    let n = src.len();
    if n == 0 {
        return;
    }
    let b = (lo + hi + 1) as u32;
    let mut sum: u32 = src[..=hi.min(n - 1)].iter().map(|v| v.get()).sum();
    for i in 0..n {
        dst[i] = T::put((sum + b / 2) / b);
        if i + hi + 1 < n {
            sum += src[i + hi + 1].get();
        }
        if i >= lo {
            sum -= src[i - lo].get();
        }
    }
}

/// One vertical box pass `src -> dst` (both `w * h`), in bands of rows; exact integer sums, so the result does not
/// depend on the band split or the thread count.
fn box_cols<T: TentCell>(src: &[T], dst: &mut [T], w: usize, h: usize, lo: usize, hi: usize) {
    let b = (lo + hi + 1) as u32;
    let band = 64.max(lo + hi + 1);
    let do_band = |(bi, chunk): (usize, &mut [T])| {
        let y0 = bi * band;
        let rows = chunk.len() / w;
        let mut sum = vec![0u32; w];
        for y in y0.saturating_sub(lo)..=(y0 + hi).min(h - 1) {
            for (s, v) in sum.iter_mut().zip(&src[y * w..(y + 1) * w]) {
                *s += v.get();
            }
        }
        for r in 0..rows {
            let y = y0 + r;
            for (d, s) in chunk[r * w..(r + 1) * w].iter_mut().zip(&sum) {
                *d = T::put((s + b / 2) / b);
            }
            if y + hi + 1 < h {
                for (s, v) in sum.iter_mut().zip(&src[(y + hi + 1) * w..(y + hi + 2) * w]) {
                    *s += v.get();
                }
            }
            if y >= lo {
                for (s, v) in sum.iter_mut().zip(&src[(y - lo) * w..(y - lo + 1) * w]) {
                    *s -= v.get();
                }
            }
        }
    };
    if w * h >= PAR_PIXELS {
        dst.par_chunks_mut(band * w).enumerate().for_each(do_band);
    } else {
        dst.chunks_mut(band * w).enumerate().for_each(do_band);
    }
}

/// Tent blur (4.2): two box blurs of width `b` per axis (the second box mirrored, so an even width stays
/// symmetric), outside = 0, integer arithmetic with rounding `(sum + b/2) / b`. `b <= 1` changes nothing; `b` is
/// capped at 16,384. Rows, then columns; rows and column bands run in parallel for large buffers.
pub fn tent<T: TentCell>(buf: &mut [T], width: u32, height: u32, b: u32) {
    let (w, h) = (width as usize, height as usize);
    if b <= 1 || w == 0 || h == 0 || buf.len() < w * h {
        return;
    }
    let b = b.min(MAX_BOX) as usize;
    let lo = (b - 1) / 2;
    let hi = b - 1 - lo;
    let buf = &mut buf[..w * h];
    let do_row = |scratch: &mut Vec<T>, row: &mut [T]| {
        scratch.clear();
        scratch.resize(row.len(), T::default());
        box_line(row, scratch, lo, hi);
        box_line(scratch, row, hi, lo);
    };
    if w * h >= PAR_PIXELS {
        buf.par_chunks_mut(w).for_each_init(Vec::new, do_row);
    } else {
        let mut scratch = Vec::new();
        buf.chunks_mut(w).for_each(|row| do_row(&mut scratch, row));
    }
    let mut tmp = vec![T::default(); w * h];
    box_cols(buf, &mut tmp, w, h, lo, hi);
    box_cols(&tmp, buf, w, h, hi, lo);
}

/// Tent box width of a blur length: `max(1, round(width))` (4.2).
pub fn tent_width(width: f32) -> u32 {
    if width.is_finite() { width.round().clamp(1.0, MAX_BOX as f32) as u32 } else { 1 }
}

/// Distance from every pixel to the shape (coverage >= 128) with the sub-pixel edge term: `distance_to_ink`.
pub fn dist_out(alpha: &[u8], width: u32, height: u32) -> Vec<f32> {
    distance_to_ink(alpha, width, height)
}

/// Distance from every pixel to the outside of the shape: `distance_to_ink(255 - alpha)`.
pub fn dist_in(alpha: &[u8], width: u32, height: u32) -> Vec<f32> {
    let inv: Vec<u8> = alpha.iter().map(|&a| 255 - a).collect();
    distance_to_ink(&inv, width, height)
}

/// `clamp(v, 0, 1)` as a byte, rounded half up; NaN and `-inf` give 0.
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

/// One coverage plane of the padded size.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Plane {
    pub w: usize,
    pub h: usize,
    pub v: Vec<u8>,
}

impl Plane {
    /// The plane with each value replaced by `f(index, value)`, in parallel for large planes.
    fn map(&self, f: impl Fn(usize, u8) -> u8 + Sync) -> Plane {
        let mut v = vec![0u8; self.v.len()];
        let w = self.w.max(1);
        let do_row = |(y, row): (usize, &mut [u8])| {
            for (x, o) in row.iter_mut().enumerate() {
                let i = y * w + x;
                *o = f(i, self.v[i]);
            }
        };
        if self.v.len() >= PAR_PIXELS {
            v.par_chunks_mut(w).enumerate().for_each(do_row);
        } else {
            v.chunks_mut(w).enumerate().for_each(do_row);
        }
        Plane { w: self.w, h: self.h, v }
    }

    /// Distance field of this plane as a shape (`distance_to_ink`).
    pub(crate) fn dist(&self) -> Vec<f32> {
        distance_to_ink(&self.v, self.w as u32, self.h as u32)
    }

    /// The plane moved by `(dx, dy)`; pixels that come in from outside get `fill`.
    fn shifted(&self, dx: i32, dy: i32, fill: u8) -> Plane {
        if dx == 0 && dy == 0 {
            return self.clone();
        }
        let (w, h) = (self.w as i64, self.h as i64);
        let mut v = vec![fill; self.v.len()];
        for y in 0..h {
            let sy = y - dy as i64;
            if sy < 0 || sy >= h {
                continue;
            }
            let x0 = (dx as i64).clamp(0, w);
            let x1 = (w + dx as i64).clamp(0, w);
            if x0 >= x1 {
                continue;
            }
            let (d0, s0) = ((y * w + x0) as usize, (sy * w + x0 - dx as i64) as usize);
            let n = (x1 - x0) as usize;
            v[d0..d0 + n].copy_from_slice(&self.v[s0..s0 + n]);
        }
        Plane { w: self.w, h: self.h, v }
    }

    /// Tent blur in place with the box width of `length`.
    fn blur(&mut self, length: f32) {
        tent(&mut self.v, self.w as u32, self.h as u32, tent_width(length));
    }
}

/// The base's alpha padded by `p` transparent pixels on every side (`inverted`: 255 - alpha, the pad 255).
pub(crate) fn alpha_plane(base: &PixelBuf, p: usize, inverted: bool) -> Plane {
    let (w, h) = (base.width as usize, base.height as usize);
    let (pw, ph) = (w + 2 * p, h + 2 * p);
    let outside = if inverted { 255 } else { 0 };
    let mut v = vec![outside; pw * ph];
    for y in 0..h {
        let row = &base.data[y * w * 4..(y + 1) * w * 4];
        let dst = &mut v[(y + p) * pw + p..(y + p) * pw + p + w];
        for (d, px) in dst.iter_mut().zip(row.as_chunks::<4>().0.iter()) {
            *d = if inverted { 255 - px[3] } else { px[3] };
        }
    }
    Plane { w: pw, h: ph, v }
}

/// Contour (die-cut) shape `M` of 4.3 step 1 on the padded alpha `a`, lengths already scaled.
pub(crate) fn contour_plane(a: &Plane, c: &Contour) -> Plane {
    let d = a.dist();
    let mut m = a.map(|i, av| av.max(cov(c.width + 0.5 - d[i])));
    drop(d);
    if c.fill_holes {
        fill_holes(&mut m);
    }
    let r = (c.smooth * c.width).round();
    if r >= 1.0 {
        // Smooth in 16 bits, then sharpen the edge back to about one pixel.
        let r = r.min(MAX_BOX as f32);
        let mut b: Vec<u16> = m.v.par_iter().with_min_len(1 << 14).map(|&v| v as u16 * 257).collect();
        tent(&mut b, m.w as u32, m.h as u32, r as u32);
        let k = r + 1.0;
        m.v.par_iter_mut().with_min_len(1 << 14).zip(&b).for_each(|(o, &bv)| {
            *o = cov((bv as f32 / 65_535.0 - 0.5) * k + 0.5);
        });
    }
    // The contour never cuts into the artwork.
    m.v.par_iter_mut().with_min_len(1 << 14).zip(&a.v).for_each(|(o, &av)| *o = (*o).max(av));
    m
}

/// Fill holes: every pixel the border cannot reach through pixels below one half (4-connected) becomes 255, except
/// the outer edge pixels that touch the reached outside.
fn fill_holes(m: &mut Plane) {
    let (w, h) = (m.w, m.h);
    if w == 0 || h == 0 {
        return;
    }
    // Span (scanline) flood from every border pixel below one half, 4-connected.
    let mut reached = vec![false; w * h];
    let open = |i: usize, reached: &[bool]| !reached[i] && m.v[i] < 128;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h - 1));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w - 1, y));
    }
    while let Some((x, y)) = stack.pop() {
        let row = y * w;
        if !open(row + x, &reached) {
            continue;
        }
        let (mut l, mut r) = (x, x);
        while l > 0 && open(row + l - 1, &reached) {
            l -= 1;
        }
        while r + 1 < w && open(row + r + 1, &reached) {
            r += 1;
        }
        reached[row + l..=row + r].fill(true);
        for ny in [y.wrapping_sub(1), y + 1] {
            if ny >= h {
                continue;
            }
            let nrow = ny * w;
            let mut k = l;
            while k <= r {
                if open(nrow + k, &reached) {
                    stack.push((k, ny));
                    while k <= r && open(nrow + k, &reached) {
                        k += 1;
                    }
                } else {
                    k += 1;
                }
            }
        }
    }
    // Not reached and not touching the reached outside: a hole or the shape's inside. The anti-aliased outer edge
    // (pixels that touch the outside) keeps its values.
    let reached = &reached;
    let fill_row = |(y, row): (usize, &mut [u8])| {
        for (x, v) in row.iter_mut().enumerate() {
            let i = y * w + x;
            if reached[i] {
                continue;
            }
            let touches = (x > 0 && reached[i - 1])
                || (x + 1 < w && reached[i + 1])
                || (y > 0 && reached[i - w])
                || (y + 1 < h && reached[i + w]);
            if !touches {
                *v = 255;
            }
        }
    };
    if w * h >= PAR_PIXELS {
        m.v.par_chunks_mut(w).enumerate().for_each(fill_row);
    } else {
        m.v.chunks_mut(w).enumerate().for_each(fill_row);
    }
}

/// Outside stroke coverage `So = clamp(wo + 0.5 - dist_out(X)) * (1 - X)` (4.3 step 2).
pub(crate) fn outside_stroke_plane(x: &Plane, wo: f32) -> Plane {
    let d = x.dist();
    x.map(|i, xv| mul_u8(cov(wo + 0.5 - d[i]), 255 - xv))
}

/// Inside stroke coverage `Si = clamp(wi + 0.5 - dist_in(A)) * A` (4.3 step 2) from the inverted alpha and its
/// distance field.
pub(crate) fn inside_stroke_plane(inv: &Plane, d_in: &[f32], wi: f32) -> Plane {
    inv.map(|i, iv| mul_u8(cov(wi + 0.5 - d_in[i]), 255 - iv))
}

/// The hard part of a soft style (4.2 spread split): `solid = round(size * spread)` pixels.
fn solid_of(size: f32, spread: f32) -> f32 {
    (size * spread).round()
}

/// The dilated source of a shadow or glow (4.2 spread split): `max(src, clamp(solid + 0.5 - d))`, moved by
/// `(dx, dy)` (pixels coming in get `fill`); `d` is needed only when `solid >= 1`. Returns the plane and the blur
/// length `size - solid`.
fn spread_plane(src: &Plane, d: Option<&[f32]>, size: f32, spread: f32, shift: (i32, i32), fill: u8) -> (Plane, f32) {
    let solid = solid_of(size, spread);
    let plane = match d {
        Some(d) if solid >= 1.0 => src.map(|i, v| v.max(cov(solid + 0.5 - d[i]))).shifted(shift.0, shift.1, fill),
        _ => src.shifted(shift.0, shift.1, fill),
    };
    (plane, size - solid)
}

/// Drop shadow coverage from the outline shape `s` and its distance field (4.3 step 4).
pub(crate) fn drop_shadow_plane(s: &Plane, d: Option<&[f32]>, sh: &Shadow) -> Plane {
    let (mut plane, blur) = spread_plane(s, d, sh.size, sh.spread, light_offset(sh.angle_deg, sh.distance), 0);
    plane.blur(blur);
    plane
}

/// Outer glow coverage (4.3 step 5); on `1 - A` (pad 255) the inner glow before the multiplication by `A`.
pub(crate) fn glow_plane(s: &Plane, d: Option<&[f32]>, g: &Glow) -> Plane {
    let (mut plane, blur) = spread_plane(s, d, g.size, g.spread, (0, 0), 0);
    plane.blur(blur);
    plane
}

/// Inner shadow coverage before the multiplication by `A` (4.3 step 6): the drop shadow of `1 - A`; pixels that
/// come in from outside count as outside (255).
pub(crate) fn inner_shadow_plane(inv: &Plane, d_in: Option<&[f32]>, sh: &Shadow) -> Plane {
    let (mut plane, blur) = spread_plane(inv, d_in, sh.size, sh.spread, light_offset(sh.angle_deg, sh.distance), 255);
    plane.blur(blur);
    plane
}

/// Straight-alpha source-over of colour `rgb` (0..=255) with coverage `sa` (0..=1) onto pixel `d`. A result whose
/// alpha rounds to 0 leaves the pixel as it was.
#[inline]
fn over(d: &mut [u8], rgb: [f32; 3], sa: f32) {
    if sa.is_nan() || sa <= 0.0 {
        return;
    }
    let sa = sa.min(1.0);
    if sa >= 1.0 || d[3] == 0 {
        // Nothing shows through (or nothing is below): the colour itself at the source alpha.
        let a8 = (sa * 255.0 + 0.5) as u8;
        if a8 == 0 {
            return;
        }
        for c in 0..3 {
            d[c] = (rgb[c] + 0.5).clamp(0.0, 255.0) as u8;
        }
        d[3] = a8;
        return;
    }
    let da = d[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    let a8 = (oa * 255.0 + 0.5) as u8;
    let k = da * (1.0 - sa);
    for c in 0..3 {
        d[c] = ((rgb[c] * sa + d[c] as f32 * k) / oa + 0.5).clamp(0.0, 255.0) as u8;
    }
    d[3] = a8;
}

/// RGB of a straight colour as floats.
#[inline]
fn rgb_of(c: [u8; 4]) -> [f32; 3] {
    [c[0] as f32, c[1] as f32, c[2] as f32]
}

/// The working picture of `apply_styles`: the padded output and where the base sits in it.
struct Canvas<'a> {
    base: &'a PixelBuf,
    out: PixelBuf,
    /// Pad on every side.
    p: usize,
}

impl Canvas<'_> {
    /// Base alpha at padded `(x, y)`; 0 outside the base.
    #[inline]
    fn alpha(&self, x: usize, y: usize) -> u8 {
        let (w, h, p) = (self.base.width as usize, self.base.height as usize, self.p);
        if x < p || y < p || x >= p + w || y >= p + h {
            return 0;
        }
        self.base.data[((y - p) * w + (x - p)) * 4 + 3]
    }

    /// Runs `f(x, y, pixel)` over every output pixel, rows in parallel for large pictures.
    fn each(&mut self, f: impl Fn(usize, usize, &mut [u8]) + Sync) {
        let w = self.out.width as usize;
        if w == 0 {
            return;
        }
        let rows = |(y, row): (usize, &mut [u8])| {
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                f(x, y, px);
            }
        };
        if self.out.data.len() / 4 >= PAR_PIXELS {
            self.out.data.par_chunks_mut(w * 4).enumerate().for_each(rows);
        } else {
            self.out.data.chunks_mut(w * 4).enumerate().for_each(rows);
        }
    }

    /// Source-over of `color` through `plane` (times `opacity`, times the base alpha when `inside`).
    fn paint(&mut self, plane: &Plane, color: [u8; 4], opacity: f32, inside: bool) {
        let rgb = rgb_of(color);
        let k = opacity * color[3] as f32 / (255.0 * 255.0);
        let w = plane.w;
        let base = self.base;
        let (bw, bh, p) = (base.width as usize, base.height as usize, self.p);
        let a_at = move |x: usize, y: usize| -> u8 {
            if x < p || y < p || x >= p + bw || y >= p + bh {
                0
            } else {
                base.data[((y - p) * bw + (x - p)) * 4 + 3]
            }
        };
        self.each(|x, y, px| {
            let c = plane.v[y * w + x];
            if c == 0 {
                return;
            }
            let mut sa = c as f32 * k;
            if inside {
                sa *= a_at(x, y) as f32 / 255.0;
            }
            over(px, rgb, sa);
        });
    }
}

/// Draws `styles` around the straight RGBA8 picture `base` (4.3). Lengths are multiplied by `scale` (the reference
/// scale times the working scale, 2.6). Output: `(w + 2m) x (h + 2m)` with `m = styles.margin(scale)`, the base at
/// `(m, m)`. With no style on, the base is returned unchanged (same bytes, margin 0). An empty or fully transparent
/// base gives a transparent picture of the output size.
pub fn apply_styles(base: &PixelBuf, styles: &StyleStack, scale: f32) -> PixelBuf {
    let st = styles.sanitized();
    if !st.any_on() {
        return base.clone();
    }
    let scale = if scale.is_finite() { scale.max(0.0) } else { 0.0 };
    let m = st.margin(scale) as usize;
    let s = st.scaled(scale);
    let (w, h) = (base.width as usize, base.height as usize);
    let (ow, oh) = ((w + 2 * m) as u32, (h + 2 * m) as u32);
    if w == 0 || h == 0 || base.data.len() != w * h * 4 || base.data.iter().skip(3).step_by(4).all(|&a| a == 0) {
        return PixelBuf::new(ow, oh);
    }
    let p = m.max(1);
    let (pw, ph) = (w + 2 * p, h + 2 * p);

    let contour = s.contour.as_ref().filter(|c| c.on);
    let (wo, wi, stroke) = match s.stroke.as_ref().filter(|st| st.on && st.width > 0.0) {
        Some(st) => match st.position {
            StrokePos::Outside => (st.width, 0.0, Some(st)),
            StrokePos::Inside => (0.0, st.width, Some(st)),
            StrokePos::Center => (st.width / 2.0, st.width / 2.0, Some(st)),
        },
        None => (0.0, 0.0, None),
    };
    let shadow = s.drop_shadow.as_ref().filter(|d| d.on);
    let glow = s.outer_glow.as_ref().filter(|g| g.on);
    let ishadow = s.inner_shadow.as_ref().filter(|d| d.on);
    let iglow = s.inner_glow.as_ref().filter(|g| g.on);
    let bevel = s.bevel.as_ref().filter(|b| b.on);

    // 1-3: contour, outside stroke, the outline shape and what it casts.
    let a = alpha_plane(base, p, false);
    let mplane = contour.map(|c| contour_plane(&a, c));
    let so = (wo > 0.0).then(|| outside_stroke_plane(mplane.as_ref().unwrap_or(&a), wo));
    let (shp, glp) = if shadow.is_some() || glow.is_some() {
        let joined;
        let outline: &Plane = match (&mplane, &so) {
            (Some(mp), Some(sp)) => {
                joined = mp.map(|i, v| v.max(sp.v[i]));
                &joined
            }
            (None, Some(sp)) => {
                joined = a.map(|i, v| v.max(sp.v[i]));
                &joined
            }
            (Some(mp), None) => mp,
            (None, None) => &a,
        };
        let need_d = shadow.is_some_and(|sh| solid_of(sh.size, sh.spread) >= 1.0)
            || glow.is_some_and(|g| solid_of(g.size, g.spread) >= 1.0);
        let d = need_d.then(|| outline.dist());
        let shp = shadow.map(|sh| drop_shadow_plane(outline, d.as_deref(), sh));
        let glp = glow.map(|g| glow_plane(outline, d.as_deref(), g));
        (shp, glp)
    } else {
        (None, None)
    };
    drop(a);

    // 6 and the inside stroke: everything measured from the outside of the artwork.
    let (si, isp, igp) = if wi > 0.0 || ishadow.is_some() || iglow.is_some() {
        let inv = alpha_plane(base, p, true);
        let need_d = wi > 0.0
            || ishadow.is_some_and(|sh| solid_of(sh.size, sh.spread) >= 1.0)
            || iglow.is_some_and(|g| solid_of(g.size, g.spread) >= 1.0);
        let d = need_d.then(|| inv.dist());
        let si = d.as_deref().filter(|_| wi > 0.0).map(|d| inside_stroke_plane(&inv, d, wi));
        let isp = ishadow.map(|sh| inner_shadow_plane(&inv, d.as_deref(), sh));
        let igp = iglow.map(|g| glow_plane(&inv, d.as_deref(), g));
        (si, isp, igp)
    } else {
        (None, None, None)
    };

    // 8: compose bottom to top.
    let mut cv = Canvas { base, out: PixelBuf::new(pw as u32, ph as u32), p };
    if let (Some(pl), Some(sh)) = (&shp, shadow) {
        cv.paint(pl, sh.color, sh.opacity, false);
    }
    drop(shp);
    if let (Some(pl), Some(g)) = (&glp, glow) {
        cv.paint(pl, g.color, g.opacity, false);
    }
    drop(glp);
    if let (Some(pl), Some(c)) = (&mplane, contour) {
        cv.paint(pl, c.color, c.opacity, false);
    }
    drop(mplane);
    if let (Some(pl), Some(st)) = (&so, stroke) {
        cv.paint(pl, st.color, st.opacity, false);
    }
    drop(so);
    paint_artwork(&mut cv, &s);
    if let (Some(pl), Some(g)) = (&igp, iglow) {
        cv.paint(pl, g.color, g.opacity, true);
    }
    drop(igp);
    if let (Some(pl), Some(sh)) = (&isp, ishadow) {
        cv.paint(pl, sh.color, sh.opacity, true);
    }
    drop(isp);
    if let Some(b) = bevel {
        paint_bevel(&mut cv, b);
    }
    if let (Some(pl), Some(st)) = (&si, stroke) {
        cv.paint(pl, st.color, st.opacity, false);
    }
    drop(si);

    if p == m {
        return cv.out;
    }
    // Inside styles only: crop the 1-px working pad away.
    let off = p - m;
    let mut out = PixelBuf::new(ow, oh);
    let (ow, oh) = (ow as usize, oh as usize);
    for y in 0..oh {
        let src = &cv.out.data[((y + off) * pw + off) * 4..((y + off) * pw + off + ow) * 4];
        out.data[y * ow * 4..(y + 1) * ow * 4].copy_from_slice(src);
    }
    out
}

/// The artwork with the gradient overlay, then the colour overlay mixed into its RGB (alpha kept; 4.3 step 8, 4.3a).
fn paint_artwork(cv: &mut Canvas<'_>, s: &StyleStack) {
    let base = cv.base;
    let (w, h, p) = (base.width as usize, base.height as usize, cv.p);
    let grad = s.gradient_overlay.as_ref().filter(|g| g.on);
    let color = s.color_overlay.as_ref().filter(|c| c.on);
    // Gradient direction and the half extent of the box's pixel centres along it.
    let (dir, half) = match grad {
        Some(g) => {
            let a = g.angle_deg.to_radians();
            let (sin, cos) = a.sin_cos();
            let dir = [cos, -sin];
            let half = ((w as f32 - 1.0) * cos.abs() + (h as f32 - 1.0) * sin.abs()) * 0.5;
            (dir, half)
        }
        None => ([1.0, 0.0], 0.0),
    };
    cv.each(|x, y, px| {
        if x < p || y < p || x >= p + w || y >= p + h {
            return;
        }
        let (bx, by) = (x - p, y - p);
        let src = &base.data[(by * w + bx) * 4..(by * w + bx) * 4 + 4];
        if src[3] == 0 {
            return;
        }
        let mut rgb = [src[0] as f32, src[1] as f32, src[2] as f32];
        if let Some(g) = grad {
            let (cx, cy) = (bx as f32 - (w as f32 - 1.0) * 0.5, by as f32 - (h as f32 - 1.0) * 0.5);
            let t = if half > 0.0 { 0.5 + (cx * dir[0] + cy * dir[1]) / (2.0 * half) } else { 0.5 };
            let c = g.gradient.sample(t);
            let k = g.opacity * c[3] as f32 / 255.0;
            for (v, &cc) in rgb.iter_mut().zip(&c) {
                *v += (cc as f32 - *v) * k;
            }
        }
        if let Some(o) = color {
            let k = o.opacity * o.color[3] as f32 / 255.0;
            for (v, &c) in rgb.iter_mut().zip(&o.color) {
                *v += (c as f32 - *v) * k;
            }
        }
        if px[3] == 0 {
            // Nothing below: the artwork's own bytes (no float round trip of the alpha).
            for i in 0..3 {
                px[i] = (rgb[i] + 0.5).clamp(0.0, 255.0) as u8;
            }
            px[3] = src[3];
        } else {
            over(px, rgb, src[3] as f32 / 255.0);
        }
    });
}

/// Smooth inner bevel (4.3 step 7): height = tent of the alpha, normal from central differences, highlight and
/// shadow from the light. Heights are scaled by `depth * size` as in the reference, so depth 1 is a slope of about
/// one at the edge whatever the size.
fn paint_bevel(cv: &mut Canvas<'_>, b: &crate::sticker::style::Bevel) {
    let (pw, ph) = (cv.out.width as usize, cv.out.height as usize);
    let mut hgt: Vec<u16> = Vec::with_capacity(pw * ph);
    for y in 0..ph {
        for x in 0..pw {
            hgt.push(cv.alpha(x, y) as u16 * 257);
        }
    }
    tent(&mut hgt, pw as u32, ph as u32, tent_width(b.size));
    let k = b.depth * b.size.max(1.0) / 65_535.0 * 0.5;
    let (sa, ca) = b.angle_deg.to_radians().sin_cos();
    let (se, ce) = b.altitude_deg.to_radians().sin_cos();
    let light = [ca * ce, -sa * ce, se];
    let (hi_rgb, sh_rgb) = (rgb_of(b.highlight), rgb_of(b.shadow));
    let hi_k = b.opacity * b.highlight[3] as f32 / 255.0;
    let sh_k = b.opacity * b.shadow[3] as f32 / 255.0;
    let base = cv.base;
    let (bw, bh, p) = (base.width as usize, base.height as usize, cv.p);
    let hgt = &hgt;
    cv.each(|x, y, px| {
        if x < p || y < p || x >= p + bw || y >= p + bh {
            return;
        }
        let a = base.data[((y - p) * bw + (x - p)) * 4 + 3];
        if a == 0 {
            return;
        }
        let at = |xx: usize, yy: usize| hgt[yy * pw + xx] as f32;
        let gx = (at((x + 1).min(pw - 1), y) - at(x.saturating_sub(1), y)) * k;
        let gy = (at(x, (y + 1).min(ph - 1)) - at(x, y.saturating_sub(1))) * k;
        if gx == 0.0 && gy == 0.0 {
            return;
        }
        let len = (gx * gx + gy * gy + 1.0).sqrt();
        let shade = (-gx * light[0] - gy * light[1] + light[2]) / len - se;
        let af = a as f32 / 255.0;
        if shade > 0.0 {
            let v = (shade / (1.0 - se).max(1e-3)).min(1.0);
            over(px, hi_rgb, v * hi_k * af);
        } else if shade < 0.0 {
            let v = (-shade / se.max(1e-3)).min(1.0);
            over(px, sh_rgb, v * sh_k * af);
        }
    });
}

