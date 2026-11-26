// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Blur, sharpen and the gradient tool (STICKER_PLAN 4.9; ARCHITECTURE 5.27, 6.12).
//!
//! - **Blur** (Gaussian of `sigma = radius`): three box blurs per axis with the widths of the "three boxes for a
//!   Gaussian" rule, on premultiplied 16-bit values (a transparent pixel lends no colour), edges replicate the border
//!   pixel, integer running sums with rounding `(sum + b/2) / b`, un-premultiplied with round half up.
//! - **Sharpen** (unsharp mask): `src + amount * (src - blur(src))` per colour channel of visible pixels.
//! - **Gradient**: `t` = projection of the pixel centre on the press -> release line, colour from the stops,
//!   source-over with the opacity.
//!
//! Each takes an optional selection of the picture's size: the result is mixed with the original by `selection / 255`
//! (a selection of another size changes nothing). Rows, and the rows of the transposed picture, run in parallel;
//! every value is an exact integer or a fixed-order `f32` expression of one pixel, so any thread count gives the same
//! bytes.

use glam::Vec2;
use rayon::prelude::*;

use crate::pixel::{AlphaBuf, PixelBuf};
use crate::sticker::fx::over;
use crate::sticker::select::{mix_selected, mix_selected_mask, mix_u8, sel_at};
use crate::sticker::style::Gradient;

/// Largest blur radius (pixels, Blur... dialog: 0.5..=64).
pub const MAX_BLUR_RADIUS: f32 = 64.0;
/// Largest sharpen amount (3 = 300 %).
pub const MAX_SHARPEN_AMOUNT: f32 = 3.0;
/// Largest sharpen radius (pixels).
pub const MAX_SHARPEN_RADIUS: f32 = 16.0;
/// Pictures with at least this many pixels run row-parallel.
const PAR_PIXELS: usize = 1 << 16;

/// Box widths (odd, ascending) of three box blurs that approximate a Gaussian of `sigma` (the standard rule:
/// `w = sqrt(12 sigma^2 / 3 + 1)`, the odd widths below and above it, as many of the smaller as gives the closest
/// variance). Widths of 1 change nothing; `sigma` is clamped to `0..=MAX_BLUR_RADIUS`.
pub fn gaussian_boxes(sigma: f32) -> [u32; 3] {
    let s = if sigma.is_finite() { sigma.clamp(0.0, MAX_BLUR_RADIUS) as f64 } else { 0.0 };
    let n = 3.0f64;
    let ideal = (12.0 * s * s / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m_ideal = (12.0 * s * s - n * (wl * wl) as f64 - 4.0 * n * wl as f64 - 3.0 * n) / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round().clamp(0.0, 3.0) as usize;
    std::array::from_fn(|i| if i < m { wl as u32 } else { wu as u32 })
}

/// One box pass of odd width `b` over a line of `N`-channel values with replicated ends, `src` -> `dst`.
fn box_pass<const N: usize>(src: &[[u16; N]], dst: &mut [[u16; N]], b: u32) {
    let n = src.len();
    if n == 0 {
        return;
    }
    let r = (b / 2) as isize;
    let at = |j: isize| &src[j.clamp(0, n as isize - 1) as usize];
    let mut sum = [0u32; N];
    for j in -r..=r {
        for (s, &v) in sum.iter_mut().zip(at(j)) {
            *s += v as u32;
        }
    }
    let half = b / 2;
    for (x, d) in dst.iter_mut().enumerate() {
        for c in 0..N {
            d[c] = ((sum[c] + half) / b) as u16;
        }
        let (add, sub) = (at(x as isize + r + 1), at(x as isize - r));
        for c in 0..N {
            sum[c] = sum[c] + add[c] as u32 - sub[c] as u32;
        }
    }
}

/// The three box passes of `boxes` along every row of a `w x h` grid, in place.
fn blur_rows<const N: usize>(buf: &mut [[u16; N]], w: usize, boxes: [u32; 3]) {
    if w == 0 || boxes.iter().all(|&b| b <= 1) {
        return;
    }
    let do_row = |tmp: &mut Vec<[u16; N]>, row: &mut [[u16; N]]| {
        tmp.clear();
        tmp.resize(row.len(), [0; N]);
        let mut in_row = true;
        for &b in boxes.iter().filter(|&&b| b > 1) {
            if in_row {
                box_pass(row, tmp, b);
            } else {
                box_pass(tmp, row, b);
            }
            in_row = !in_row;
        }
        if !in_row {
            row.copy_from_slice(tmp);
        }
    };
    if buf.len() >= PAR_PIXELS {
        buf.par_chunks_mut(w).for_each_init(Vec::new, do_row);
    } else {
        let mut tmp = Vec::new();
        buf.chunks_mut(w).for_each(|row| do_row(&mut tmp, row));
    }
}

/// Transpose of a `w x h` grid (rows become columns), in blocks of 64 for the cache.
fn transpose<const N: usize>(src: &[[u16; N]], w: usize, h: usize) -> Vec<[u16; N]> {
    let mut out = vec![[0u16; N]; w * h];
    if w == 0 || h == 0 {
        return out;
    }
    const B: usize = 64;
    // Output row x (length h) = input column x. Chunks of B output rows.
    let do_chunk = |(ci, chunk): (usize, &mut [[u16; N]])| {
        let x0 = ci * B;
        let rows = chunk.len() / h;
        for y0 in (0..h).step_by(B) {
            let y1 = (y0 + B).min(h);
            for xr in 0..rows {
                let x = x0 + xr;
                let o = &mut chunk[xr * h..(xr + 1) * h];
                for y in y0..y1 {
                    o[y] = src[y * w + x];
                }
            }
        }
    };
    if w * h >= PAR_PIXELS {
        out.par_chunks_mut(B * h).enumerate().for_each(do_chunk);
    } else {
        out.chunks_mut(B * h).enumerate().for_each(do_chunk);
    }
    out
}

/// Separable three-box blur of a grid: rows, transpose, rows, transpose back.
fn blur_grid<const N: usize>(buf: Vec<[u16; N]>, w: usize, h: usize, boxes: [u32; 3]) -> Vec<[u16; N]> {
    let mut buf = buf;
    blur_rows(&mut buf, w, boxes);
    let mut t = transpose(&buf, w, h);
    drop(buf);
    blur_rows(&mut t, h, boxes);
    transpose(&t, h, w)
}

/// Gaussian blur (4.9) of a straight RGBA8 picture; radius 0.5..=64 px (`sigma = radius`; smaller does nothing).
/// With `sel` (the picture's size) the blurred picture is mixed with the original by `sel / 255`.
pub fn blur(src: &PixelBuf, radius: f32, sel: Option<&AlphaBuf>) -> PixelBuf {
    let (w, h) = (src.width as usize, src.height as usize);
    let boxes = gaussian_boxes(radius);
    if w == 0 || h == 0 || src.data.len() != w * h * 4 || boxes.iter().all(|&b| b <= 1) {
        return src.clone();
    }
    // Premultiplied, in units of 1 / (255 * 255): colour c * a, alpha a * 255.
    let pre: Vec<[u16; 4]> = src
        .data
        .par_chunks(4)
        .with_min_len(1 << 14)
        .map(|p| {
            let a = p[3] as u16;
            [p[0] as u16 * a, p[1] as u16 * a, p[2] as u16 * a, a * 255]
        })
        .collect();
    let out = blur_grid(pre, w, h, boxes);
    let mut res = PixelBuf::new(src.width, src.height);
    res.data.par_chunks_mut(4).with_min_len(1 << 14).enumerate().for_each(|(i, d)| {
        let q = out[i];
        let a = q[3] as u32;
        let alpha = ((a + 127) / 255) as u8;
        if alpha == 0 {
            // Nothing visible left: the source colour at alpha 0 (invisible data kept, no division by 0).
            d.copy_from_slice(&[src.data[i * 4], src.data[i * 4 + 1], src.data[i * 4 + 2], 0]);
            return;
        }
        for c in 0..3 {
            d[c] = ((q[c] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
        d[3] = alpha;
    });
    if let Some(sel) = sel {
        mix_selected(src, &mut res, sel);
    }
    res
}

/// Gaussian blur of a mask (one channel, the same rule; edges replicate).
pub fn blur_mask(src: &AlphaBuf, radius: f32, sel: Option<&AlphaBuf>) -> AlphaBuf {
    let (w, h) = (src.width as usize, src.height as usize);
    let boxes = gaussian_boxes(radius);
    if w == 0 || h == 0 || src.data.len() != w * h || boxes.iter().all(|&b| b <= 1) {
        return src.clone();
    }
    let pre: Vec<[u16; 1]> = src.data.iter().map(|&v| [v as u16 * 255]).collect();
    let out = blur_grid(pre, w, h, boxes);
    let data = out.iter().map(|q| ((q[0] as u32 + 127) / 255) as u8).collect();
    let mut res = AlphaBuf { width: src.width, height: src.height, data };
    if let Some(sel) = sel {
        mix_selected_mask(src, &mut res, sel);
    }
    res
}

/// Sharpen (unsharp mask, 4.9): `clamp(src + amount * (src - blur(src, radius)))` per colour channel of every pixel
/// with alpha > 0, round half up; alpha unchanged. Amount 0..=3 (300 %), radius 0.5..=16. Amount 0 returns the
/// picture unchanged.
pub fn sharpen(src: &PixelBuf, amount: f32, radius: f32, sel: Option<&AlphaBuf>) -> PixelBuf {
    let amount = if amount.is_finite() { amount.clamp(0.0, MAX_SHARPEN_AMOUNT) } else { 0.0 };
    let radius = if radius.is_finite() { radius.clamp(0.0, MAX_SHARPEN_RADIUS) } else { 0.0 };
    if amount == 0.0 || src.data.len() != src.width as usize * src.height as usize * 4 {
        return src.clone();
    }
    let blurred = blur(src, radius, None);
    let len = src.data.len() / 4;
    let mut res = src.clone();
    res.data.par_chunks_mut(4).with_min_len(1 << 14).enumerate().for_each(|(i, d)| {
        let k = sel_at(sel, len, i);
        if d[3] == 0 || k == 0 {
            return;
        }
        let b = &blurred.data[i * 4..i * 4 + 3];
        let before = [d[0], d[1], d[2]];
        for c in 0..3 {
            let s = before[c] as f32;
            let v = (s + amount * (s - b[c] as f32) + 0.5).floor().clamp(0.0, 255.0) as u8;
            d[c] = mix_u8(before[c], v, k);
        }
    });
    res
}

/// `t` of the pixel centre `p` on the line `from -> to` (0 at `from`, 1 at `to`, clamped). A line of length 0 gives
/// 1 everywhere (the whole picture is past the release point).
#[inline]
fn line_t(p: Vec2, from: Vec2, dir: Vec2, len2: f32) -> f32 {
    if len2 <= 0.0 {
        return 1.0;
    }
    ((p - from).dot(dir) / len2).clamp(0.0, 1.0)
}

/// Gradient tool (4.9) on a picture: source-over of `gradient` along `from -> to` (layer pixels, continuous; pixel
/// centres at `+0.5`), times `opacity` (0..=1) and the selection.
pub fn draw_gradient(pixels: &mut PixelBuf, from: Vec2, to: Vec2, gradient: &Gradient, opacity: f32, sel: Option<&AlphaBuf>) {
    let w = pixels.width as usize;
    if w == 0 || !from.is_finite() || !to.is_finite() {
        return;
    }
    let mut g = gradient.clone();
    g.sanitize();
    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 0.0 };
    let (dir, len2) = (to - from, (to - from).length_squared());
    let len = pixels.data.len() / 4;
    let row = |(y, row): (usize, &mut [u8])| {
        for (x, d) in row.chunks_mut(4).enumerate() {
            let i = y * w + x;
            let k = sel_at(sel, len, i);
            if k == 0 {
                continue;
            }
            let c = g.sample(line_t(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), from, dir, len2));
            let sa = opacity * (c[3] as f32 / 255.0) * (k as f32 / 255.0);
            over(d, [c[0] as f32, c[1] as f32, c[2] as f32], sa);
        }
    };
    if len >= PAR_PIXELS {
        pixels.data.par_chunks_mut(w * 4).enumerate().for_each(row);
    } else {
        pixels.data.chunks_mut(w * 4).enumerate().for_each(row);
    }
}

/// Gradient tool on a layer mask: the luma `round(0.299 r + 0.587 g + 0.114 b)` of the gradient's colour is mixed in
/// by `opacity * colour alpha * selection`.
pub fn draw_gradient_mask(mask: &mut AlphaBuf, from: Vec2, to: Vec2, gradient: &Gradient, opacity: f32, sel: Option<&AlphaBuf>) {
    let w = mask.width as usize;
    if w == 0 || !from.is_finite() || !to.is_finite() {
        return;
    }
    let mut g = gradient.clone();
    g.sanitize();
    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 0.0 };
    let (dir, len2) = (to - from, (to - from).length_squared());
    let len = mask.data.len();
    let row = |(y, row): (usize, &mut [u8])| {
        for (x, m) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let c = g.sample(line_t(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), from, dir, len2));
            let luma = (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32 + 0.5) as u8;
            let k = opacity * (c[3] as f32 / 255.0) * (sel_at(sel, len, i) as f32 / 255.0);
            *m = mix_u8(*m, luma, (k * 255.0 + 0.5) as u8);
        }
    };
    if len >= PAR_PIXELS {
        mask.data.par_chunks_mut(w).enumerate().for_each(row);
    } else {
        mask.data.chunks_mut(w).enumerate().for_each(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sticker::select::Selection;
    use crate::testutil::XorShift;

    fn premul_sum(b: &PixelBuf) -> [f64; 4] {
        let mut s = [0.0f64; 4];
        for p in b.data.chunks(4) {
            let a = p[3] as f64;
            for c in 0..3 {
                s[c] += p[c] as f64 * a / 255.0;
            }
            s[3] += a;
        }
        s
    }

    #[test]
    fn k2_three_box_widths() {
        assert_eq!(gaussian_boxes(0.0), [1, 1, 1]);
        assert_eq!(gaussian_boxes(0.4), [1, 1, 1]);
        assert_eq!(gaussian_boxes(f32::NAN), [1, 1, 1]);
        for s in [1.0f32, 2.0, 3.3, 8.0, 16.0, 40.0, 64.0] {
            let b = gaussian_boxes(s);
            assert!(b.iter().all(|&v| v % 2 == 1), "{b:?}");
            assert!(b[0] <= b[1] && b[1] <= b[2] && b[2] - b[0] <= 2);
            // The variance of the three boxes is close to sigma^2.
            let var: f64 = b.iter().map(|&w| (w as f64 * w as f64 - 1.0) / 12.0).sum();
            let s2 = (s * s) as f64;
            assert!((var - s2).abs() <= 0.15 * s2 + 0.7, "sigma {s}: {b:?} var {var}");
        }
        assert_eq!(gaussian_boxes(1000.0), gaussian_boxes(MAX_BLUR_RADIUS));
    }

    #[test]
    fn k2_blur_of_a_constant_picture_is_the_same_picture() {
        for c in [[200u8, 100, 50, 255], [10, 220, 90, 77], [0, 0, 0, 1]] {
            let img = PixelBuf::filled(37, 23, c);
            assert_eq!(blur(&img, 5.0, None), img, "{c:?}");
            let m = AlphaBuf::filled(37, 23, c[3]);
            assert_eq!(blur_mask(&m, 5.0, None), m);
        }
        // Radius below a box of 3 changes nothing.
        let mut rng = XorShift::new(2);
        let mut img = PixelBuf::new(9, 9);
        for v in img.data.iter_mut() {
            *v = rng.next_u32() as u8;
        }
        assert_eq!(blur(&img, 0.3, None), img);
    }

    #[test]
    fn k2_blur_keeps_the_premultiplied_sum_and_bleeds_no_colour() {
        // A random patch in the middle of a large transparent field: the premultiplied sum is kept (rounding only).
        let mut rng = XorShift::new(7);
        let mut img = PixelBuf::new(160, 140);
        for y in 50..90 {
            for x in 60..100 {
                let c = rng.next_u32().to_le_bytes();
                img.set(x, y, c);
            }
        }
        let out = blur(&img, 6.0, None);
        let (a, b) = (premul_sum(&img), premul_sum(&out));
        for c in 0..4 {
            assert!((a[c] - b[c]).abs() / a[c] < 0.005, "channel {c}: {} -> {}", a[c], b[c]);
        }
        // Opaque red next to transparent (black, and green) pixels: no visible pixel gets darker or greener.
        let mut img = PixelBuf::new(60, 20);
        for y in 0..20 {
            for x in 0..60 {
                img.set(x, y, if x < 30 { [255, 0, 0, 255] } else if x < 45 { [0, 0, 0, 0] } else { [0, 255, 0, 0] });
            }
        }
        let out = blur(&img, 8.0, None);
        let mut soft = 0;
        for p in out.data.chunks(4) {
            if p[3] > 0 {
                assert_eq!(&p[..3], &[255, 0, 0], "{p:?}");
                soft += (p[3] < 255) as u32;
            }
        }
        assert!(soft > 100);
        // Edges replicate: a picture that is red in its left half stays red at the left border.
        assert_eq!(out.get(0, 10), [255, 0, 0, 255]);
    }

    #[test]
    fn k2_blur_inside_a_selection_and_on_a_mask() {
        let mut img = PixelBuf::new(40, 40);
        for y in 0..40 {
            for x in 0..40 {
                img.set(x, y, if (x / 4 + y / 4) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
            }
        }
        let sel = Selection::rect(40, 40, glam::Vec2::new(10.0, 10.0), glam::Vec2::new(30.0, 30.0));
        let out = blur(&img, 3.0, Some(&sel.mask));
        for y in 0..40 {
            for x in 0..40 {
                let inside = (10..30).contains(&x) && (10..30).contains(&y);
                if !inside {
                    assert_eq!(out.get(x, y), img.get(x, y));
                }
            }
        }
        assert_ne!(out.get(20, 20), img.get(20, 20));
        // A hard mask edge becomes a ramp, its sum kept in the middle of the picture.
        let mut m = AlphaBuf::new(80, 80);
        for y in 20..60 {
            for x in 20..60 {
                m.set(x, y, 255);
            }
        }
        let b = blur_mask(&m, 4.0, None);
        let (s0, s1): (u64, u64) = (m.data.iter().map(|&v| v as u64).sum(), b.data.iter().map(|&v| v as u64).sum());
        assert!(s0.abs_diff(s1) * 200 < s0);
        assert!(b.get(20, 40) > 64 && b.get(20, 40) < 192);
        assert_eq!(b.get(40, 40), 255);
    }

    #[test]
    fn k2_sharpen_rules() {
        let mut rng = XorShift::new(4);
        let mut img = PixelBuf::new(30, 30);
        for p in img.data.chunks_mut(4) {
            let c = rng.next_u32().to_le_bytes();
            p.copy_from_slice(&[c[0], c[1], c[2], if c[3] < 40 { 0 } else { c[3] }]);
        }
        assert_eq!(sharpen(&img, 0.0, 2.0, None), img);
        let s = sharpen(&img, 1.5, 2.0, None);
        assert_ne!(s, img);
        for (a, b) in s.data.chunks(4).zip(img.data.chunks(4)) {
            assert_eq!(a[3], b[3]);
            if b[3] == 0 {
                assert_eq!(a, b);
            }
        }
        // An edge gets steeper: the dark side darker, the light side lighter.
        let mut e = PixelBuf::new(20, 4);
        for y in 0..4 {
            for x in 0..20 {
                let v = if x < 10 { 80 } else { 170 };
                e.set(x, y, [v, v, v, 255]);
            }
        }
        let s = sharpen(&e, 1.0, 2.0, None);
        assert!(s.get(9, 2)[0] < 80 && s.get(10, 2)[0] > 170);
        assert_eq!(s.get(0, 2)[0], 80);
        // A selection limits it.
        let sel = AlphaBuf::new(20, 4);
        assert_eq!(sharpen(&e, 1.0, 2.0, Some(&sel)), e);
    }

    #[test]
    fn k2_gradient_endpoints_are_exact_and_a_selection_limits_it() {
        let g = Gradient::two([255, 0, 0, 255], [0, 0, 255, 255]);
        let mut img = PixelBuf::filled(21, 3, [9, 9, 9, 255]);
        draw_gradient(&mut img, Vec2::new(0.5, 1.5), Vec2::new(20.5, 1.5), &g, 1.0, None);
        assert_eq!(img.get(0, 1), [255, 0, 0, 255]);
        assert_eq!(img.get(20, 1), [0, 0, 255, 255]);
        assert_eq!(img.get(10, 0), [128, 0, 128, 255]);
        // Beyond the ends the end colours.
        let mut img = PixelBuf::new(30, 1);
        draw_gradient(&mut img, Vec2::new(10.5, 0.5), Vec2::new(19.5, 0.5), &g, 1.0, None);
        assert_eq!(img.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(img.get(29, 0), [0, 0, 255, 255]);
        // A selection limits it; half opacity mixes.
        let mut img = PixelBuf::filled(10, 10, [255, 255, 255, 255]);
        let sel = Selection::rect(10, 10, Vec2::new(0.0, 0.0), Vec2::new(5.0, 10.0));
        let black = Gradient::two([0, 0, 0, 255], [0, 0, 0, 255]);
        draw_gradient(&mut img, Vec2::ZERO, Vec2::new(10.0, 0.0), &black, 0.5, Some(&sel.mask));
        assert_eq!(img.get(2, 5), [128, 128, 128, 255]);
        assert_eq!(img.get(7, 5), [255, 255, 255, 255]);
        // On a mask: the luma, the selection.
        let mut m = AlphaBuf::new(11, 1);
        let bw = Gradient::two([0, 0, 0, 255], [255, 255, 255, 255]);
        draw_gradient_mask(&mut m, Vec2::new(0.5, 0.5), Vec2::new(10.5, 0.5), &bw, 1.0, None);
        assert_eq!(m.get(0, 0), 0);
        assert_eq!(m.get(10, 0), 255);
        assert_eq!(m.get(5, 0), 128);
        let mut m2 = AlphaBuf::new(11, 1);
        draw_gradient_mask(&mut m2, Vec2::new(0.5, 0.5), Vec2::new(10.5, 0.5), &bw, 1.0, Some(&AlphaBuf::new(11, 1)));
        assert_eq!(m2, AlphaBuf::new(11, 1));
        // A zero-length line fills with the last stop.
        let mut img = PixelBuf::new(3, 3);
        draw_gradient(&mut img, Vec2::ONE, Vec2::ONE, &g, 1.0, None);
        assert!(img.data.chunks(4).all(|p| p == [0, 0, 255, 255]));
    }

    #[test]
    fn k2_filters_are_thread_count_independent() {
        let mut rng = XorShift::new(17);
        let mut img = PixelBuf::new(333, 271);
        for p in img.data.chunks_mut(4) {
            let c = rng.next_u32().to_le_bytes();
            p.copy_from_slice(&[c[0], c[1], c[2], if c[3] < 60 { 0 } else { c[3] }]);
        }
        let sel = Selection::ellipse(333, 271, Vec2::new(30.0, 20.0), Vec2::new(300.0, 250.0)).mask;
        let run = || {
            let a = blur(&img, 9.5, Some(&sel));
            let b = sharpen(&img, 2.0, 3.0, None);
            let c = blur_mask(&sel, 12.0, None);
            let mut d = img.clone();
            let g = Gradient { stops: vec![(0.0, [255, 0, 0, 255]), (0.4, [0, 255, 0, 128]), (1.0, [0, 0, 255, 255])] };
            draw_gradient(&mut d, Vec2::new(3.0, 7.0), Vec2::new(250.0, 200.0), &g, 0.8, Some(&sel));
            let mut e = sel.clone();
            draw_gradient_mask(&mut e, Vec2::new(3.0, 7.0), Vec2::new(250.0, 200.0), &g, 0.8, None);
            (a, b, c, d, e)
        };
        let pools = [1usize, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
        let x = pools[0].install(run);
        let y = pools[1].install(run);
        assert!(x == y, "results differ between 1 and 8 threads");
    }
}
