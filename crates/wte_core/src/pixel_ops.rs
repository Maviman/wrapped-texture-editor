// Owner: core-geometry (C1). Pixel operations of ARCHITECTURE.md 5.5: premultiplication, content bounds, cropping,
// the premultiplied mip chain used by every resampling bake, resize, thumbnails and export padding.

use rayon::prelude::*;

use crate::math::{IRect, mul_u8};
use crate::pixel::{CroppedBuf, PixelBuf, PixelView};
use crate::texel_map::TexelMap;

/// Images with at least this many pixels are processed row-parallel.
const PAR_PIXELS: usize = 1 << 16;

/// Straight alpha -> premultiplied alpha.
#[inline]
pub fn premultiply(p: [u8; 4]) -> [u8; 4] {
    [mul_u8(p[0], p[3]), mul_u8(p[1], p[3]), mul_u8(p[2], p[3]), p[3]]
}

/// Premultiplied alpha -> straight alpha: `(c*255 + a/2) / a`; `a == 0` -> `[0; 4]`.
#[inline]
pub fn unpremultiply(p: [u8; 4]) -> [u8; 4] {
    let a = p[3] as u32;
    if a == 0 {
        return [0; 4];
    }
    let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
    [un(p[0]), un(p[1]), un(p[2]), p[3]]
}

/// Bounding box (canvas coordinates) of the pixels of `view` for which `hit` is true.
fn bounds_where(view: PixelView<'_>, hit: impl Fn(&[u8; 4]) -> bool) -> Option<IRect> {
    let w = view.rect.width() as usize;
    let stride = w * 4;
    if stride == 0 {
        return None;
    }
    let h = (view.rect.height() as usize).min(view.data.len() / stride);
    let row = |y: usize| view.data[y * stride..(y + 1) * stride].as_chunks::<4>().0;
    let any = |y: usize| row(y).iter().any(&hit);

    let top = (0..h).find(|&y| any(y))?;
    let bottom = (top..h).rev().find(|&y| any(y)).unwrap_or(top);
    // Only the parts left of the current minimum and right of the current maximum can still widen the box.
    let mut x0 = w;
    let mut x1 = 0usize; // exclusive
    for y in top..=bottom {
        let r = row(y);
        if let Some(first) = r[..x0].iter().position(&hit) {
            x0 = first;
        }
        if let Some(last) = r[x1..].iter().rposition(&hit) {
            x1 += last + 1;
        }
    }
    Some(IRect::new(
        view.rect.x0 + x0 as i32,
        view.rect.y0 + top as i32,
        view.rect.x0 + x1 as i32,
        view.rect.y0 + bottom as i32 + 1,
    ))
}

/// Bbox of alpha > 0; None if fully transparent.
pub fn content_bounds(view: PixelView<'_>) -> Option<IRect> {
    bounds_where(view, |p| p[3] != 0)
}

/// Bbox of pixels != [0, 0, 0, 0] (keeps RGB stored under alpha 0); None if every byte is zero. Used when saving raster layers.
pub fn nonzero_bounds(view: PixelView<'_>) -> Option<IRect> {
    bounds_where(view, |p| *p != [0, 0, 0, 0])
}

/// `nonzero_bounds` of a whole dense image, scanned in parallel bands of rows (large paint layers).
pub fn nonzero_bounds_par(buf: &PixelBuf) -> Option<IRect> {
    const BAND: usize = 64;
    let stride = buf.width as usize * 4;
    if stride == 0 || buf.height == 0 || buf.data.len() < stride * buf.height as usize {
        return None;
    }
    buf.data[..stride * buf.height as usize]
        .par_chunks(stride * BAND)
        .enumerate()
        .filter_map(|(band, data)| {
            let y0 = (band * BAND) as i32;
            let rect = IRect::new(0, y0, buf.width as i32, y0 + (data.len() / stride) as i32);
            nonzero_bounds(PixelView { rect, data })
        })
        .reduce_with(|a, b| a.union(&b))
}

/// Copies `rect` (clipped to the image) out of `buf`.
pub fn crop(buf: &PixelBuf, rect: IRect) -> CroppedBuf {
    let r = rect.intersect(&buf.rect());
    if r.is_empty() {
        return CroppedBuf::empty();
    }
    let n = r.width() as usize * 4;
    let start = r.x0 as usize * 4;
    let mut data = Vec::with_capacity(n * r.height() as usize);
    for y in r.y0..r.y1 {
        data.extend_from_slice(&buf.row(y as u32)[start..start + n]);
    }
    CroppedBuf { rect: r, data }
}

/// `crop` to `content_bounds` (an empty buffer if fully transparent).
pub fn crop_to_content(buf: &PixelBuf) -> CroppedBuf {
    match content_bounds(buf.view()) {
        Some(rect) => crop(buf, rect),
        None => CroppedBuf::empty(),
    }
}

/// Premultiplied RGBA8 box-filtered mip chain of a source image. levels[0] = full size.
#[derive(Clone, Debug)]
pub struct PremulMips {
    pub levels: Vec<PixelBuf>,
}

/// Source taps of one destination pixel of the exact box filter `src -> dst` along one axis:
/// (first source index, tap count, integer weights that sum to `src`).
fn box_taps(src: usize, dst: usize) -> Vec<(usize, usize, [u32; 4])> {
    (0..dst)
        .map(|i| {
            // In units of 1/dst source pixels: destination pixel i spans [i*src, (i+1)*src), source pixel s spans
            // [s*dst, (s+1)*dst).
            let (lo, hi) = (i * src, (i + 1) * src);
            let first = lo / dst;
            let end = hi.div_ceil(dst).min(src);
            let count = (end - first).min(4);
            let mut weights = [0u32; 4];
            for (k, w) in weights.iter_mut().enumerate().take(count) {
                let s = first + k;
                *w = (hi.min((s + 1) * dst) - lo.max(s * dst)) as u32;
            }
            (first, count, weights)
        })
        .collect()
}

/// Next mip level (`max(1, w/2) x max(1, h/2)`) of a premultiplied image: exact area average.
fn downsample_half(src: &PixelBuf) -> PixelBuf {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (dw, dh) = ((sw / 2).max(1), (sh / 2).max(1));
    let mut out = PixelBuf::new(dw as u32, dh as u32);
    let stride = sw * 4;
    let even = sw == dw * 2 && sh == dh * 2;
    let (tx, ty) = if even { (Vec::new(), Vec::new()) } else { (box_taps(sw, dw), box_taps(sh, dh)) };
    let total = (sw * sh) as u64;

    let do_row = |(y, drow): (usize, &mut [u8])| {
        if even {
            let r0 = &src.data[2 * y * stride..(2 * y + 1) * stride];
            let r1 = &src.data[(2 * y + 1) * stride..(2 * y + 2) * stride];
            for (x, d) in drow.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let i = x * 8;
                for c in 0..4 {
                    let s = r0[i + c] as u32 + r0[i + 4 + c] as u32 + r1[i + c] as u32 + r1[i + 4 + c] as u32;
                    d[c] = ((s + 2) >> 2) as u8;
                }
            }
        } else {
            let (y0, ny, wy) = ty[y];
            for (x, d) in drow.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x0, nx, wx) = tx[x];
                let mut acc = [0u64; 4];
                for (j, &wyj) in wy.iter().enumerate().take(ny) {
                    let row = &src.data[(y0 + j) * stride..(y0 + j + 1) * stride];
                    for (i, &wxi) in wx.iter().enumerate().take(nx) {
                        let w = wyj as u64 * wxi as u64;
                        let p = &row[(x0 + i) * 4..(x0 + i) * 4 + 4];
                        for c in 0..4 {
                            acc[c] += w * p[c] as u64;
                        }
                    }
                }
                for c in 0..4 {
                    d[c] = ((acc[c] + total / 2) / total) as u8;
                }
            }
        }
    };
    if dw * dh >= PAR_PIXELS {
        out.data.par_chunks_mut(dw * 4).enumerate().for_each(do_row);
    } else {
        out.data.chunks_mut(dw * 4).enumerate().for_each(do_row);
    }
    out
}

/// Bilinear sample of one premultiplied level at normalized `(u, v)`; texels outside the image are transparent.
#[inline]
fn bilinear(level: &PixelBuf, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (level.width as i32, level.height as i32);
    let x = u * level.width as f32 - 0.5;
    let y = v * level.height as f32 - 0.5;
    // Written so that NaN and infinities fall out here.
    if !(x > -1.0 && y > -1.0 && x < w as f32 && y < h as f32) {
        return [0.0; 4];
    }
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (x0, y0) = (xf as i32, yf as i32);
    let stride = w as usize * 4;
    let fetch = |xi: i32, yi: i32| -> [f32; 4] {
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            return [0.0; 4];
        }
        let i = yi as usize * stride + xi as usize * 4;
        match level.data.get(i..i + 4) {
            Some(p) => [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32],
            None => [0.0; 4],
        }
    };
    let (p00, p10, p01, p11) = (fetch(x0, y0), fetch(x0 + 1, y0), fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1));
    let (w00, w10, w01, w11) = ((1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy);
    let mut out = [0.0f32; 4];
    for c in 0..4 {
        out[c] = (p00[c] * w00 + p10[c] * w10 + p01[c] * w01 + p11[c] * w11) * (1.0 / 255.0);
    }
    out
}

impl PremulMips {
    /// Builds the premultiplied mip chain of a straight-alpha image.
    pub fn build(src: &PixelBuf) -> PremulMips {
        let pixels = src.width as usize * src.height as usize;
        if pixels == 0 || src.data.len() != pixels * 4 {
            return PremulMips { levels: vec![PixelBuf::new(0, 0)] };
        }
        let mut data = src.data.clone();
        let premul = |chunk: &mut [u8]| {
            for p in chunk.as_chunks_mut::<4>().0 {
                let a = p[3];
                if a != 255 {
                    p[0] = mul_u8(p[0], a);
                    p[1] = mul_u8(p[1], a);
                    p[2] = mul_u8(p[2], a);
                }
            }
        };
        if pixels >= PAR_PIXELS {
            data.par_chunks_mut(1 << 16).for_each(premul);
        } else {
            premul(&mut data);
        }
        let mut levels = vec![PixelBuf { width: src.width, height: src.height, data }];
        loop {
            let last = &levels[levels.len() - 1];
            if last.width <= 1 && last.height <= 1 {
                break;
            }
            let next = downsample_half(last);
            levels.push(next);
        }
        PremulMips { levels }
    }

    /// Size of level 0.
    pub fn size(&self) -> [u32; 2] {
        match self.levels.first() {
            Some(l) => [l.width, l.height],
            None => [0, 0],
        }
    }

    /// Bilinear within a level, linear between levels, transparent outside 0..1. Returns premultiplied RGBA in 0..=1.
    ///
    /// "Transparent outside" is the border colour of the bilinear filter: a sample on the image edge is half
    /// transparent and the result reaches zero half a texel (of the sampled level) outside the image.
    pub fn sample(&self, u: f32, v: f32, lod: f32) -> [f32; 4] {
        let n = self.levels.len();
        if n == 0 {
            return [0.0; 4];
        }
        let lod = if lod.is_nan() { 0.0 } else { lod.clamp(0.0, (n - 1) as f32) };
        let l0 = lod.floor() as usize;
        let t = lod - l0 as f32;
        let a = bilinear(&self.levels[l0], u, v);
        if t <= 0.0 || l0 + 1 >= n {
            return a;
        }
        let b = bilinear(&self.levels[l0 + 1], u, v);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
    }

    /// Bilinear on the single nearest level (Preview quality).
    pub fn sample_nearest_level(&self, u: f32, v: f32, lod: f32) -> [f32; 4] {
        let n = self.levels.len();
        if n == 0 {
            return [0.0; 4];
        }
        let lod = if lod.is_nan() { 0.0 } else { lod.clamp(0.0, (n - 1) as f32) };
        let level = ((lod + 0.5).floor() as usize).min(n - 1);
        bilinear(&self.levels[level], u, v)
    }
}

/// Filter taps of one axis of `resize`: destination pixel `i` reads `weights[offsets[i]..offsets[i + 1]]`, starting
/// at source index `first[i]`. Weights of one pixel sum to 1.
struct AxisTaps {
    first: Vec<u32>,
    offsets: Vec<u32>,
    weights: Vec<f32>,
}

impl AxisTaps {
    fn new(src: u32, dst: u32) -> AxisTaps {
        let mut taps = AxisTaps {
            first: Vec::with_capacity(dst as usize),
            offsets: Vec::with_capacity(dst as usize + 1),
            weights: Vec::new(),
        };
        taps.offsets.push(0);
        let scale = src as f64 / dst as f64;
        for i in 0..dst {
            if dst < src {
                // Area filter: destination pixel i covers the source interval [i*scale, (i+1)*scale).
                let (lo, hi) = (i as f64 * scale, (i as f64 + 1.0) * scale);
                let s0 = (lo.floor() as u32).min(src - 1);
                let s1 = (hi.ceil() as u32).clamp(s0 + 1, src);
                let start = taps.weights.len();
                let mut sum = 0.0f64;
                for s in s0..s1 {
                    let overlap = (hi.min(s as f64 + 1.0) - lo.max(s as f64)).max(0.0);
                    sum += overlap;
                    taps.weights.push(overlap as f32);
                }
                if sum > 0.0 {
                    for w in &mut taps.weights[start..] {
                        *w = (*w as f64 / sum) as f32;
                    }
                } else {
                    taps.weights[start] = 1.0;
                }
                taps.first.push(s0);
            } else if dst == src {
                taps.first.push(i);
                taps.weights.push(1.0);
            } else {
                // Bilinear, clamped at the edges.
                let c = (i as f64 + 0.5) * scale - 0.5;
                let i0 = c.floor();
                let f = (c - i0) as f32;
                let a = (i0.max(0.0) as u32).min(src - 1);
                let b = ((i0 + 1.0).max(0.0) as u32).min(src - 1);
                taps.first.push(a);
                if a == b {
                    taps.weights.push(1.0);
                } else {
                    taps.weights.push(1.0 - f);
                    taps.weights.push(f);
                }
            }
            taps.offsets.push(taps.weights.len() as u32);
        }
        taps
    }

    #[inline]
    fn of(&self, i: usize) -> (usize, &[f32]) {
        (self.first[i] as usize, &self.weights[self.offsets[i] as usize..self.offsets[i + 1] as usize])
    }
}

/// Resizes with a premultiplied area/bilinear filter (area average when shrinking an axis, bilinear when enlarging it).
/// Where the filtered alpha is 0 the plain average of the RGB values is kept, so RGB stored under alpha 0 survives.
pub fn resize(src: &PixelBuf, width: u32, height: u32) -> PixelBuf {
    let src_pixels = src.width as usize * src.height as usize;
    if width == 0 || height == 0 || src_pixels == 0 || src.data.len() != src_pixels * 4 {
        return PixelBuf::new(width, height);
    }
    if width == src.width && height == src.height {
        return src.clone();
    }
    let tx = AxisTaps::new(src.width, width);
    let ty = AxisTaps::new(src.height, height);
    let mut out = PixelBuf::new(width, height);
    let w = width as usize;

    // Per pixel: premultiplied r, g, b, alpha, straight r, g, b.
    let do_row = |acc: &mut Vec<f32>, (y, drow): (usize, &mut [u8])| {
        acc.clear();
        acc.resize(w * 7, 0.0);
        let (sy0, wys) = ty.of(y);
        for (j, &wy) in wys.iter().enumerate() {
            let srow = src.row((sy0 + j) as u32);
            for (x, a) in acc.as_chunks_mut::<7>().0.iter_mut().enumerate() {
                let (sx0, wxs) = tx.of(x);
                for (i, &wx) in wxs.iter().enumerate() {
                    let p = &srow[(sx0 + i) * 4..(sx0 + i) * 4 + 4];
                    let wgt = wx * wy;
                    let alpha = p[3] as f32;
                    let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
                    a[0] += wgt * r * alpha;
                    a[1] += wgt * g * alpha;
                    a[2] += wgt * b * alpha;
                    a[3] += wgt * alpha;
                    a[4] += wgt * r;
                    a[5] += wgt * g;
                    a[6] += wgt * b;
                }
            }
        }
        let to_u8 = |v: f32| (v + 0.5).clamp(0.0, 255.0) as u8;
        for (d, a) in drow.as_chunks_mut::<4>().0.iter_mut().zip(acc.as_chunks::<7>().0) {
            let alpha = to_u8(a[3]);
            *d = if alpha == 0 || a[3] <= 0.0 {
                [to_u8(a[4]), to_u8(a[5]), to_u8(a[6]), 0]
            } else {
                [to_u8(a[0] / a[3]), to_u8(a[1] / a[3]), to_u8(a[2] / a[3]), alpha]
            };
        }
    };
    if w * height as usize >= PAR_PIXELS {
        out.data.par_chunks_mut(w * 4).enumerate().for_each_init(Vec::new, do_row);
    } else {
        let mut acc = Vec::new();
        out.data.chunks_mut(w * 4).enumerate().for_each(|item| do_row(&mut acc, item));
    }
    out
}

/// `thumbnail` over an arbitrary pixel source (`get(x, y)` in canvas coordinates).
pub(crate) fn thumbnail_with(canvas_w: u32, canvas_h: u32, side: u32, mut get: impl FnMut(i32, i32) -> [u8; 4]) -> PixelBuf {
    if side == 0 || canvas_w == 0 || canvas_h == 0 {
        return PixelBuf::new(0, 0);
    }
    let scaled = |short: u32, long: u32| ((side as u64 * short as u64 + long as u64 / 2) / long as u64).max(1) as u32;
    let (tw, th) =
        if canvas_w >= canvas_h { (side, scaled(canvas_h, canvas_w)) } else { (scaled(canvas_w, canvas_h), side) };
    // Two regular sample positions (1/4 and 3/4 of the footprint) per axis; they coincide when enlarging.
    let positions = |i: u32, canvas: u32, thumb: u32| -> ([i32; 2], usize) {
        let at = |q: u64| ((4 * i as u64 + q) * canvas as u64 / (4 * thumb as u64)).min(canvas as u64 - 1) as i32;
        let (a, b) = (at(1), at(3));
        ([a, b], if a == b { 1 } else { 2 })
    };
    let xs: Vec<([i32; 2], usize)> = (0..tw).map(|x| positions(x, canvas_w, tw)).collect();
    let mut out = PixelBuf::new(tw, th);
    for ty in 0..th {
        let (sy, ny) = positions(ty, canvas_h, th);
        for (tx, &(sx, nx)) in xs.iter().enumerate() {
            let (mut sum_a, mut sum_c) = (0u32, [0u32; 3]);
            for &y in &sy[..ny] {
                for &x in &sx[..nx] {
                    let p = get(x, y);
                    let a = p[3] as u32;
                    sum_a += a;
                    sum_c[0] += p[0] as u32 * a;
                    sum_c[1] += p[1] as u32 * a;
                    sum_c[2] += p[2] as u32 * a;
                }
            }
            if sum_a == 0 {
                continue;
            }
            let n = (nx * ny) as u32;
            let c = |v: u32| ((v + sum_a / 2) / sum_a).min(255) as u8;
            out.set(tx as u32, ty, [c(sum_c[0]), c(sum_c[1]), c(sum_c[2]), ((sum_a + n / 2) / n) as u8]);
        }
    }
    out
}

/// Aspect-preserving thumbnail (longest side = `side`) of a canvas of size canvas_w x canvas_h showing `view`.
/// Stride-sampled: every output pixel averages at most 2x2 source samples (premultiplied) taken at regular positions inside its
/// footprint, so the cost is O(side^2) whatever the canvas size (never a full box filter over the canvas).
pub fn thumbnail(view: PixelView<'_>, canvas_w: u32, canvas_h: u32, side: u32) -> PixelBuf {
    thumbnail_with(canvas_w, canvas_h, side, |x, y| view.get(x, y))
}

/// Export padding: fill texels that have no owner in `map` and alpha 0 with the nearest covered colour, up to `max_px` steps.
///
/// Multi-source breadth-first growth from every texel the map assigns to a triangle (covered or gutter), one ring per
/// step through the 8-neighbourhood, wrapping at the image borders like the gutter itself. A filled texel copies the
/// texel it was reached from; within a ring straight steps are taken before diagonal ones. Texels that are unowned but
/// not transparent are neither changed nor used as a source. Does nothing when the buffer and the map differ in size.
pub fn dilate_islands(buf: &mut PixelBuf, map: &TexelMap, max_px: u32) {
    let (w, h) = (buf.width as usize, buf.height as usize);
    let n = w * h;
    if max_px == 0 || n == 0 || map.width != buf.width || map.height != buf.height || buf.data.len() != n * 4 {
        return;
    }
    let owners = map.owner_raw();
    if owners.len() != n {
        return;
    }
    // Axial neighbours first.
    const NEIGHBOURS: [(isize, isize); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)];
    let neighbour = |idx: usize, (dx, dy): (isize, isize)| -> usize {
        let x = (idx % w) as isize + dx;
        let y = (idx / w) as isize + dy;
        y.rem_euclid(h as isize) as usize * w + x.rem_euclid(w as isize) as usize
    };

    // A texel may be filled when it has no owner, is transparent and has not been filled yet.
    let mut open: Vec<bool> =
        (0..n).map(|i| owners[i] == TexelMap::NO_OWNER && buf.data[i * 4 + 3] == 0).collect();
    // First ring: every open texel looks for an owned neighbour.
    let mut frontier: Vec<usize> = Vec::new();
    for dst in (0..n).filter(|&i| open[i]) {
        let source = NEIGHBOURS.iter().map(|&d| neighbour(dst, d)).find(|&src| owners[src] != TexelMap::NO_OWNER);
        if let Some(src) = source {
            buf.data.copy_within(src * 4..src * 4 + 4, dst * 4);
            frontier.push(dst);
        }
    }
    for &dst in &frontier {
        open[dst] = false;
    }
    // Further rings grow from the previous one.
    for _ in 1..max_px {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for directions in [&NEIGHBOURS[..4], &NEIGHBOURS[4..]] {
            for &src in &frontier {
                for &d in directions {
                    let dst = neighbour(src, d);
                    if open[dst] {
                        open[dst] = false;
                        buf.data.copy_within(src * 4..src * 4 + 4, dst * 4);
                        next.push(dst);
                    }
                }
            }
        }
        frontier = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TextureSetId;
    use crate::samples;
    use crate::testutil::{XorShift, mesh_of};

    #[test]
    fn premultiply_round_trip() {
        assert_eq!(premultiply([200, 100, 50, 255]), [200, 100, 50, 255]);
        assert_eq!(premultiply([200, 100, 50, 0]), [0, 0, 0, 0]);
        assert_eq!(premultiply([255, 255, 255, 128]), [128, 128, 128, 128]);
        assert_eq!(unpremultiply([9, 9, 9, 0]), [0; 4]);
        assert_eq!(unpremultiply([128, 64, 0, 128]), [255, 128, 0, 128]);
        // Opaque pixels are untouched in both directions.
        for c in 0..=255u8 {
            assert_eq!(unpremultiply(premultiply([c, c, c, 255])), [c, c, c, 255]);
        }
        // Every valid premultiplied value survives un-premultiply + premultiply exactly.
        for a in 1..=255u8 {
            for q in 0..=a {
                assert_eq!(premultiply(unpremultiply([q, q, q, a])), [q, q, q, a], "q = {q}, a = {a}");
            }
        }
        // Straight -> premultiplied -> straight is exact up to the quantization of the alpha step.
        for a in 1..=255u32 {
            for c in 0..=255u32 {
                let back = unpremultiply(premultiply([c as u8, 0, 0, a as u8]))[0] as f64;
                assert!((back - c as f64).abs() <= 127.5 / a as f64 + 0.5 + 1e-9, "c = {c}, a = {a}, back = {back}");
            }
        }
        // Out-of-gamut premultiplied input (c > a) clamps instead of wrapping.
        assert_eq!(unpremultiply([255, 0, 0, 1]), [255, 0, 0, 1]);
    }

    fn red_green_checker(size: u32) -> PixelBuf {
        let mut img = PixelBuf::new(size, size);
        for y in 0..size {
            for x in 0..size {
                img.set(x, y, if (x + y) % 2 == 0 { [255, 0, 0, 255] } else { [0, 255, 0, 0] });
            }
        }
        img
    }

    #[test]
    fn mip_of_red_and_transparent_green_stays_red() {
        let mips = PremulMips::build(&red_green_checker(8));
        assert_eq!(mips.size(), [8, 8]);
        assert_eq!(mips.levels.len(), 4);
        assert_eq!(mips.levels.iter().map(|l| l.width).collect::<Vec<_>>(), vec![8, 4, 2, 1]);
        for level in &mips.levels[1..] {
            for p in level.data.as_chunks::<4>().0.iter() {
                assert_eq!(p[1], 0, "green leaked into a mip");
                assert_eq!(p[2], 0);
                assert_eq!(unpremultiply([p[0], p[1], p[2], p[3]]), [255, 0, 0, 128]);
            }
        }
        // Sampling between levels is still pure red.
        let s = mips.sample(0.5, 0.5, 1.5);
        assert!(s[1] == 0.0 && s[2] == 0.0);
        assert!((s[0] - s[3]).abs() < 1e-6 && (s[3] - 128.0 / 255.0).abs() < 1e-6, "{s:?}");
    }

    #[test]
    fn mips_of_odd_and_non_square_sizes() {
        // A solid colour must survive every level whatever the size.
        for (w, h) in [(1u32, 1u32), (2, 1), (5, 3), (7, 7), (9, 2), (1, 6), (13, 31)] {
            let mips = PremulMips::build(&PixelBuf::filled(w, h, [90, 180, 30, 200]));
            let expect = premultiply([90, 180, 30, 200]);
            let last = mips.levels.last().unwrap();
            assert_eq!((last.width, last.height), (1, 1), "{w}x{h}");
            for (i, level) in mips.levels.iter().enumerate() {
                assert_eq!(level.width, (w >> i).max(1));
                assert_eq!(level.height, (h >> i).max(1));
                assert!(level.data.as_chunks::<4>().0.iter().all(|p| *p == expect), "{w}x{h} level {i}");
            }
        }
        // 3x1 -> 1x1 is the exact average of the three texels.
        let img = PixelBuf::from_raw(3, 1, vec![30, 0, 0, 255, 60, 0, 0, 255, 90, 0, 0, 255]).unwrap();
        assert_eq!(PremulMips::build(&img).levels[1].data, vec![60, 0, 0, 255]);
        // A large image takes the parallel paths and gives the same result as its rows processed serially.
        let mut rng = XorShift::new(17);
        let mut big = PixelBuf::new(512, 300);
        for b in big.data.iter_mut() {
            *b = rng.next_u32() as u8;
        }
        let mips = PremulMips::build(&big);
        assert_eq!(mips.levels.len(), 10);
        let strip = PixelBuf::from_raw(512, 2, big.data[..512 * 2 * 4].to_vec()).unwrap();
        let strip_mips = PremulMips::build(&strip);
        assert_eq!(mips.levels[0].row(0), strip_mips.levels[0].row(0));
        assert_eq!(mips.levels[1].row(0), strip_mips.levels[1].row(0));
        // Premultiplied levels never have a colour above their alpha.
        assert!(mips.levels.iter().all(|l| l.data.as_chunks::<4>().0.iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])));
        // Degenerate inputs do not panic.
        let empty = PremulMips::build(&PixelBuf::new(0, 4));
        assert_eq!(empty.size(), [0, 0]);
        assert_eq!(empty.sample(0.5, 0.5, 0.0), [0.0; 4]);
        assert_eq!(PremulMips { levels: Vec::new() }.sample_nearest_level(0.5, 0.5, 3.0), [0.0; 4]);
        assert_eq!(PremulMips { levels: Vec::new() }.size(), [0, 0]);
    }

    #[test]
    fn bilinear_border_is_transparent() {
        let mips = PremulMips::build(&PixelBuf::filled(4, 4, [255, 255, 255, 255]));
        let close = |a: [f32; 4], v: f32| a.iter().all(|c| (c - v).abs() < 1e-5);
        assert!(close(mips.sample(0.5, 0.5, 0.0), 1.0));
        // Texel centres are exact.
        assert!(close(mips.sample(0.125, 0.875, 0.0), 1.0));
        // On the image edge the sample is half inside, half transparent border.
        assert!(close(mips.sample(0.0, 0.5, 0.0), 0.5));
        assert!(close(mips.sample(0.5, 1.0, 0.0), 0.5));
        assert!(close(mips.sample(0.0, 0.0, 0.0), 0.25));
        // Half a texel outside and beyond: fully transparent.
        assert!(close(mips.sample(-0.125, 0.5, 0.0), 0.0));
        assert!(close(mips.sample(1.125, 0.5, 0.0), 0.0));
        assert!(close(mips.sample(0.5, -3.0, 0.0), 0.0));
        assert!(close(mips.sample(f32::NAN, 0.5, 0.0), 0.0));
        assert!(close(mips.sample(0.5, f32::INFINITY, 0.0), 0.0));
        // Level selection: out-of-range and NaN lods clamp.
        assert!(close(mips.sample(0.5, 0.5, 99.0), 1.0));
        assert!(close(mips.sample(0.5, 0.5, -4.0), 1.0));
        assert!(close(mips.sample(0.5, 0.5, f32::NAN), 1.0));
        assert!(close(mips.sample_nearest_level(0.5, 0.5, 1.4), 1.0));
    }

    #[test]
    fn sampling_interpolates_within_and_between_levels() {
        // 2x1: black opaque | white opaque.
        let img = PixelBuf::from_raw(2, 1, vec![0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
        let mips = PremulMips::build(&img);
        assert_eq!(mips.levels.len(), 2);
        assert_eq!(mips.levels[1].data, vec![128, 128, 128, 255]);
        let mid = mips.sample(0.5, 0.5, 0.0);
        assert!((mid[0] - 0.5).abs() < 1e-5 && (mid[3] - 1.0).abs() < 1e-5);
        let left = mips.sample(0.25, 0.5, 0.0);
        assert!(left[0].abs() < 1e-6);
        // Trilinear in the image centre: half way between level 0 (0.5) and the grey 1x1 level (128 / 255).
        let tri = mips.sample(0.5, 0.5, 0.5);
        assert!((tri[0] - 0.5 * (0.5 + 128.0 / 255.0)).abs() < 1e-5 && (tri[3] - 1.0).abs() < 1e-5, "{tri:?}");
        // Nearest level: 0.4 -> level 0, 0.6 -> level 1.
        assert!(mips.sample_nearest_level(0.25, 0.5, 0.4)[0].abs() < 1e-6);
        assert!((mips.sample_nearest_level(0.5, 0.5, 0.6)[0] - 128.0 / 255.0).abs() < 1e-5);
        // Coarser levels have wider texels, so their transparent border reaches further into the image: at
        // u = 0.25 the 1x1 level is already a quarter border.
        let coarse = mips.sample(0.25, 0.5, 1.0);
        assert!((coarse[3] - 0.75).abs() < 1e-5 && (coarse[0] - 0.75 * 128.0 / 255.0).abs() < 1e-5, "{coarse:?}");
        let between = mips.sample(0.25, 0.5, 0.5);
        assert!((between[3] - 0.875).abs() < 1e-5 && (between[0] - 0.375 * 128.0 / 255.0).abs() < 1e-5);
    }

    #[test]
    fn content_bounds_is_tight() {
        let mut img = PixelBuf::new(16, 12);
        assert_eq!(content_bounds(img.view()), None);
        assert!(crop_to_content(&img).is_empty());
        img.set(3, 2, [1, 2, 3, 4]);
        assert_eq!(content_bounds(img.view()), Some(IRect::new(3, 2, 4, 3)));
        img.set(10, 9, [0, 0, 0, 255]);
        img.set(5, 11, [0, 0, 0, 1]);
        img.set(15, 4, [9, 9, 9, 9]);
        img.set(0, 7, [9, 9, 9, 9]);
        assert_eq!(content_bounds(img.view()), Some(IRect::new(0, 2, 16, 12)));

        // A view with an offset reports canvas coordinates.
        let mut c = CroppedBuf::new(IRect::new(100, 50, 108, 56));
        c.data[(2 * 8 + 5) * 4 + 3] = 7;
        c.data[(4 * 8 + 1) * 4 + 3] = 7;
        assert_eq!(content_bounds(c.view()), Some(IRect::new(101, 52, 106, 55)));
        assert_eq!(content_bounds(CroppedBuf::empty().view()), None);

        // Random content against a brute-force scan.
        let mut rng = XorShift::new(7);
        for _ in 0..50 {
            let mut img = PixelBuf::new(23, 17);
            let mut expect: Option<IRect> = None;
            for _ in 0..rng.next_u32() % 6 {
                let (x, y) = (rng.next_u32() % 23, rng.next_u32() % 17);
                img.set(x, y, [0, 0, 0, 1 + (rng.next_u32() % 255) as u8]);
                let r = IRect::from_xywh(x as i32, y as i32, 1, 1);
                expect = Some(expect.map_or(r, |e| e.union(&r)));
            }
            assert_eq!(content_bounds(img.view()), expect);
            let cropped = crop_to_content(&img);
            assert_eq!(cropped.rect, expect.unwrap_or(IRect::EMPTY));
            assert_eq!(cropped.to_dense(23, 17), img);
        }
    }

    #[test]
    fn nonzero_bounds_keeps_rgb_under_alpha_zero() {
        let mut img = PixelBuf::new(8, 8);
        assert_eq!(nonzero_bounds(img.view()), None);
        img.set(2, 5, [10, 20, 30, 0]);
        assert_eq!(content_bounds(img.view()), None);
        assert_eq!(nonzero_bounds(img.view()), Some(IRect::new(2, 5, 3, 6)));
        img.set(6, 1, [0, 0, 1, 0]);
        assert_eq!(nonzero_bounds(img.view()), Some(IRect::new(2, 1, 7, 6)));
        // Every single non-zero byte counts.
        for byte in 0..4 {
            let mut one = PixelBuf::new(3, 3);
            one.data[(3 + 1) * 4 + byte] = 1;
            assert_eq!(nonzero_bounds(one.view()), Some(IRect::new(1, 1, 2, 2)));
        }
        let full = PixelBuf::filled(5, 4, [10, 20, 30, 0]);
        assert_eq!(nonzero_bounds(full.view()), Some(IRect::new(0, 0, 5, 4)));
    }

    #[test]
    fn crop_clips_to_the_image() {
        let mut img = PixelBuf::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.set(x, y, [x as u8, y as u8, 0, 255]);
            }
        }
        let c = crop(&img, IRect::new(2, 1, 9, 3));
        assert_eq!(c.rect, IRect::new(2, 1, 4, 3));
        assert_eq!(c.data.len(), 2 * 2 * 4);
        assert_eq!(c.get(3, 2), [3, 2, 0, 255]);
        assert_eq!(c.get(1, 2), [0; 4]);
        assert!(crop(&img, IRect::new(4, 0, 8, 4)).is_empty());
        assert!(crop(&img, IRect::new(-5, -5, 0, 0)).is_empty());
        assert!(crop(&img, IRect::EMPTY).is_empty());
        assert_eq!(crop(&img, IRect::new(-10, -10, 10, 10)).to_dense(4, 4), img);
    }

    #[test]
    fn resize_preserves_a_solid_colour() {
        for color in [[200u8, 100, 50, 255], [13, 250, 77, 128], [255, 255, 255, 3], [10, 20, 30, 0]] {
            let src = PixelBuf::filled(37, 21, color);
            for (w, h) in [(37u32, 21u32), (8, 8), (1, 1), (64, 5), (100, 63), (36, 22), (3, 300)] {
                let out = resize(&src, w, h);
                assert_eq!((out.width, out.height), (w, h));
                assert!(out.data.as_chunks::<4>().0.iter().all(|p| *p == color), "{color:?} -> {w}x{h}");
            }
        }
        assert_eq!(resize(&PixelBuf::new(0, 0), 4, 4), PixelBuf::new(4, 4));
        assert_eq!(resize(&PixelBuf::filled(4, 4, [1, 2, 3, 4]), 0, 7).byte_len(), 0);
    }

    #[test]
    fn resize_filters_premultiplied() {
        // Halving a red / transparent-green checker gives pure red at half alpha.
        let out = resize(&red_green_checker(8), 4, 4);
        assert!(out.data.as_chunks::<4>().0.iter().all(|p| *p == [255, 0, 0, 128]), "{:?}", &out.data[..8]);
        // Exact area average when shrinking by an integer factor.
        let src = PixelBuf::from_raw(4, 1, vec![0, 0, 0, 255, 100, 0, 0, 255, 50, 0, 0, 255, 150, 0, 0, 255]).unwrap();
        assert_eq!(resize(&src, 2, 1).data, vec![50, 0, 0, 255, 100, 0, 0, 255]);
        // Enlarging interpolates and keeps the end values.
        let src = PixelBuf::from_raw(2, 1, vec![0, 0, 0, 255, 200, 0, 0, 255]).unwrap();
        let up = resize(&src, 4, 1);
        assert_eq!(up.get(0, 0), [0, 0, 0, 255]);
        assert_eq!(up.get(3, 0), [200, 0, 0, 255]);
        assert!(up.get(1, 0)[0] > 0 && up.get(1, 0)[0] < up.get(2, 0)[0] && up.get(2, 0)[0] < 200);
        // Large enough to take the parallel path: every output row must equal the same row computed serially
        // (a one-row-high strip of the source resized on its own takes the serial path).
        let mut rng = XorShift::new(3);
        let mut big = PixelBuf::new(600, 300);
        for b in big.data.iter_mut() {
            *b = rng.next_u32() as u8;
        }
        let wide = resize(&big, 280, 300);
        assert_eq!(wide, resize(&big, 280, 300), "deterministic");
        for y in [0u32, 1, 150, 299] {
            let strip = crop(&big, IRect::new(0, y as i32, 600, y as i32 + 1));
            let strip = PixelBuf::from_raw(600, 1, strip.data).unwrap();
            assert_eq!(resize(&strip, 280, 1).data, wide.row(y), "row {y}");
        }
    }

    #[test]
    fn thumbnail_cost_is_independent_of_the_canvas() {
        for side in [1u32, 16, 64] {
            let mut reads = 0u64;
            let out = thumbnail_with(8192, 8192, side, |x, y| {
                assert!((0..8192).contains(&x) && (0..8192).contains(&y));
                reads += 1;
                [40, 80, 120, 255]
            });
            assert_eq!((out.width, out.height), (side, side));
            assert!(reads <= 4 * side as u64 * side as u64, "{reads} reads for side {side}");
            assert!(out.data.as_chunks::<4>().0.iter().all(|p| *p == [40, 80, 120, 255]));
        }
        // The public entry point on a view that covers only part of a huge canvas: outside is transparent.
        let part = CroppedBuf { rect: IRect::new(0, 0, 128, 128), data: [9, 9, 9, 255].repeat(128 * 128) };
        let out = thumbnail(part.view(), 8192, 8192, 64);
        assert_eq!(out.get(0, 0), [9, 9, 9, 255]);
        assert_eq!(out.get(1, 0), [0; 4]);
        assert_eq!(out.get(63, 63), [0; 4]);
    }

    #[test]
    fn thumbnail_preserves_solid_colours_and_aspect() {
        for color in [[200u8, 100, 50, 255], [13, 250, 77, 128], [255, 0, 255, 1]] {
            let src = PixelBuf::filled(300, 100, color);
            let out = thumbnail(src.view(), 300, 100, 48);
            assert_eq!((out.width, out.height), (48, 16));
            assert!(out.data.as_chunks::<4>().0.iter().all(|p| *p == color), "{color:?}");
        }
        let tall = PixelBuf::filled(10, 1000, [1, 2, 3, 255]);
        let out = thumbnail(tall.view(), 10, 1000, 32);
        assert_eq!((out.width, out.height), (1, 32));
        // Enlarging a tiny canvas repeats its texels.
        let tiny = PixelBuf::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let out = thumbnail(tiny.view(), 2, 1, 8);
        assert_eq!((out.width, out.height), (8, 4));
        assert_eq!(out.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(out.get(7, 3), [0, 0, 255, 255]);
        // Transparent texels do not darken the average.
        let mut half = PixelBuf::new(4, 4);
        for y in 0..4 {
            half.set(0, y, [200, 100, 0, 255]);
            half.set(2, y, [200, 100, 0, 255]);
        }
        let out = thumbnail(half.view(), 4, 4, 2);
        assert!(out.data.as_chunks::<4>().0.iter().all(|p| *p == [200, 100, 0, 128]), "{:?}", out.data);
        assert_eq!(thumbnail(half.view(), 4, 4, 0).byte_len(), 0);
        assert_eq!(thumbnail(half.view(), 0, 4, 8).byte_len(), 0);
    }

    /// True when a texel within `steps` (8-neighbourhood, wrapping) of `(x, y)` has an owner.
    fn near_owner(map: &TexelMap, x: u32, y: u32, steps: i32) -> bool {
        let (w, h) = (map.width as i32, map.height as i32);
        (-steps..=steps).any(|dy| {
            (-steps..=steps)
                .any(|dx| map.owner((x as i32 + dx).rem_euclid(w) as u32, (y as i32 + dy).rem_euclid(h) as u32).is_some())
        })
    }

    #[test]
    fn dilate_fills_only_unowned_transparent_texels() {
        // tight_islands: two islands in the middle of the texture, free space all around.
        let mesh = mesh_of(samples::tight_islands());
        let map = TexelMap::build(&mesh, TextureSetId(0), 64, 64, 2);
        let mut buf = PixelBuf::new(64, 64);
        // Paint every owned texel (covered red, gutter green) and one unowned opaque texel.
        for y in 0..64 {
            for x in 0..64 {
                match map.owner(x, y) {
                    Some((_, false)) => buf.set(x, y, [200, 0, 0, 255]),
                    Some((_, true)) => buf.set(x, y, [0, 200, 0, 255]),
                    None => {}
                }
            }
        }
        assert_eq!(map.owner(2, 2), None);
        buf.set(2, 2, [1, 2, 3, 255]);
        let before = buf.clone();

        let mut same = buf.clone();
        dilate_islands(&mut same, &map, 0);
        assert_eq!(same, before, "max_px == 0 changes nothing");

        dilate_islands(&mut buf, &map, 3);
        let (mut filled, mut expected) = (0, 0);
        for y in 0..64 {
            for x in 0..64 {
                let (was, now) = (before.get(x, y), buf.get(x, y));
                if map.owner(x, y).is_some() || was[3] != 0 {
                    assert_eq!(now, was, "owned or non-transparent texel ({x}, {y}) changed");
                    continue;
                }
                // Exactly the unowned transparent texels within 3 steps of an island are filled, with the
                // colour of the gutter they continue.
                let near = near_owner(&map, x, y, 3);
                expected += near as usize;
                if now != was {
                    filled += 1;
                    assert_eq!(now, [0, 200, 0, 255]);
                    assert!(near, "({x}, {y}) is further than 3 steps from an island");
                }
            }
        }
        assert!(filled > 0);
        assert_eq!(filled, expected);

        // A second call starts from the map's owners again: nothing new is reachable.
        let mut again = buf.clone();
        dilate_islands(&mut again, &map, 3);
        assert_eq!(again, buf);

        // Unlimited distance fills every transparent texel; a size mismatch is ignored.
        let mut all = before.clone();
        dilate_islands(&mut all, &map, u32::MAX);
        assert!(all.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        assert_eq!(all.get(2, 2), [1, 2, 3, 255]);
        let mut other = PixelBuf::new(32, 32);
        dilate_islands(&mut other, &map, 4);
        assert_eq!(other, PixelBuf::new(32, 32));
    }

    #[test]
    fn dilate_continues_the_nearest_colour() {
        // Without a gutter the first ring copies the covered texel straight beside it.
        let mesh = mesh_of(samples::tight_islands());
        let map = TexelMap::build(&mesh, TextureSetId(0), 32, 32, 0);
        // Islands: columns 8..=15 and 16..=23, rows 8..=23. Give every covered texel its own colour.
        let mut buf = PixelBuf::new(32, 32);
        for y in 0..32u32 {
            for x in 0..32u32 {
                if map.owner(x, y).is_some() {
                    buf.set(x, y, [x as u8 * 8, y as u8 * 8, 77, 255]);
                }
            }
        }
        assert!(map.owner(8, 8).is_some() && map.owner(7, 8).is_none() && map.owner(23, 23).is_some());
        dilate_islands(&mut buf, &map, 2);
        // Straight neighbours.
        assert_eq!(buf.get(7, 12), buf.get(8, 12));
        assert_eq!(buf.get(6, 12), buf.get(8, 12));
        assert_eq!(buf.get(24, 20), buf.get(23, 20));
        assert_eq!(buf.get(12, 7), buf.get(12, 8));
        assert_eq!(buf.get(12, 25), buf.get(12, 23));
        // Corners continue the corner texel diagonally.
        assert_eq!(buf.get(7, 7), buf.get(8, 8));
        assert_eq!(buf.get(6, 6), buf.get(8, 8));
        assert_eq!(buf.get(5, 12), [0; 4], "three steps away");

        // An owned texel with alpha 0 spreads its (invisible) colour; the result stays transparent.
        let mut clear = PixelBuf::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                if map.owner(x, y).is_some() {
                    clear.set(x, y, [10, 20, 30, 0]);
                }
            }
        }
        dilate_islands(&mut clear, &map, 1);
        let ring = (0..32u32)
            .flat_map(|y| (0..32u32).map(move |x| (x, y)))
            .filter(|&(x, y)| map.owner(x, y).is_none() && clear.get(x, y) == [10, 20, 30, 0])
            .count();
        assert_eq!(ring, 18 * 18 - 16 * 16);
        assert!(clear.data.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    }
}
