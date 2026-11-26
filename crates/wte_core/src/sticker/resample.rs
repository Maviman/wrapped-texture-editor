// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Affine resampling of a layer's content into output pixels (STICKER_PLAN 4.7; ARCHITECTURE 6.11).
//!
//! Inverse mapping per output pixel centre, premultiplied bilinear sampling, transparent outside the picture. When
//! the scale on both axes is below 0.5 the source is first reduced with the exact box-filter chain (`PremulMips`: the
//! level whose scale is closest to but not below the larger of the two axis scales). Moves by whole pixels, flips and
//! quarter turns that land on the pixel grid copy pixels (bytes unchanged, RGB under alpha 0 kept). Output is
//! un-premultiplied with round half up.

use glam::{DVec2, Vec2};
use rayon::prelude::*;

use crate::math::{IRect, mul_u8};
use crate::pixel::{CroppedBuf, PixelBuf};
use crate::pixel_ops::PremulMips;
use crate::sticker::LayerTransform;

/// Outputs with at least this many pixels are resampled in parallel.
const PAR_PIXELS: usize = 1 << 16;
/// Largest output (pixels) `transform_picture` produces; beyond it the result is clipped to its top-left part.
pub const MAX_RESAMPLE_PIXELS: u64 = 1 << 26;

/// Output-space position of the content point `p` under transform `t` at render scale `s` (f64, exact enough to put
/// grid-aligned pixel centres on half-integers).
fn forward(t: &LayerTransform, size: [u32; 2], s: f64, p: DVec2) -> DVec2 {
    let c = DVec2::new(size[0] as f64, size[1] as f64) * 0.5;
    let (sin, cos) = (t.rotation as f64).sin_cos();
    let q = (p - c) * DVec2::new(t.scale.x as f64, t.scale.y as f64);
    (DVec2::new(t.translate.x as f64, t.translate.y as f64) + c + DVec2::new(q.x * cos - q.y * sin, q.x * sin + q.y * cos)) * s
}

/// Bounding rectangle (output pixels) of the transformed content box.
pub fn transformed_bounds(size: [u32; 2], t: &LayerTransform, s: f32) -> IRect {
    let mut t = *t;
    t.sanitize();
    let s = if s.is_finite() && s > 0.0 { s as f64 } else { 1.0 };
    let (w, h) = (size[0] as f64, size[1] as f64);
    if size[0] == 0 || size[1] == 0 {
        return IRect::EMPTY;
    }
    let corners = [DVec2::ZERO, DVec2::new(w, 0.0), DVec2::new(0.0, h), DVec2::new(w, h)].map(|p| forward(&t, size, s, p));
    let (mut lo, mut hi) = (corners[0], corners[0]);
    for c in &corners[1..] {
        lo = lo.min(*c);
        hi = hi.max(*c);
    }
    // Snap values within 1e-6 of an integer so a grid-aligned box does not grow by a pixel.
    let snap = |v: f64| if (v - v.round()).abs() < 1e-6 { v.round() } else { v };
    let clampi = |v: f64| v.clamp(-1.0e9, 1.0e9) as i32;
    IRect::new(clampi(snap(lo.x).floor()), clampi(snap(lo.y).floor()), clampi(snap(hi.x).ceil()), clampi(snap(hi.y).ceil()))
}

/// The quarter turn `k` (0..4) when `rotation` is a multiple of 90 degrees within 1e-6 rad.
fn quarter_turn(rotation: f32) -> Option<i32> {
    let q = rotation as f64 / std::f64::consts::FRAC_PI_2;
    let k = q.round();
    ((q - k).abs() * std::f64::consts::FRAC_PI_2 <= 1e-6).then_some((k as i64).rem_euclid(4) as i32)
}

/// Pixel-exact copy for moves, flips and quarter turns that map pixel centres onto pixel centres; `None` otherwise.
fn copy_exact(src: &PixelBuf, t: &LayerTransform, s: f64, rect: IRect) -> Option<CroppedBuf> {
    quarter_turn(t.rotation)?;
    let (ex, ey) = (t.scale.x as f64 * s, t.scale.y as f64 * s);
    if (ex.abs() - 1.0).abs() > 1e-9 || (ey.abs() - 1.0).abs() > 1e-9 {
        return None;
    }
    let size = [src.width, src.height];
    // Every source pixel centre must land within 1e-4 of an output pixel centre.
    let probe = forward(t, size, s, DVec2::new(0.5, 0.5)) - DVec2::splat(0.5);
    if (probe - probe.round()).abs().max_element() > 1e-4 {
        return None;
    }
    let mut out = CroppedBuf::new(rect);
    let rw = rect.width() as i64;
    for y in 0..src.height {
        for x in 0..src.width {
            let o = forward(t, size, s, DVec2::new(x as f64 + 0.5, y as f64 + 0.5)) - DVec2::splat(0.5);
            let (ox, oy) = (o.x.round() as i64 - rect.x0 as i64, o.y.round() as i64 - rect.y0 as i64);
            if ox < 0 || oy < 0 || ox >= rw || oy >= rect.height() as i64 {
                continue;
            }
            let d = ((oy * rw + ox) * 4) as usize;
            out.data[d..d + 4].copy_from_slice(&src.get(x, y));
        }
    }
    Some(out)
}

/// Bilinear sample of a premultiplied level at continuous pixel coordinates `(x, y)` (centres at +0.5); outside
/// the level transparent. Premultiplied 0..=255.
#[inline]
fn bilinear(level: &PixelBuf, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (level.width as i32, level.height as i32);
    let (fx0, fy0) = (x - 0.5, y - 0.5);
    if !(fx0 > -1.0 && fy0 > -1.0 && fx0 < w as f32 && fy0 < h as f32) {
        return [0.0; 4];
    }
    let (xf, yf) = (fx0.floor(), fy0.floor());
    let (fx, fy) = (fx0 - xf, fy0 - yf);
    let (x0, y0) = (xf as i32, yf as i32);
    let fetch = |xi: i32, yi: i32| -> [f32; 4] {
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            return [0.0; 4];
        }
        let i = (yi as usize * w as usize + xi as usize) * 4;
        let p = &level.data[i..i + 4];
        [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
    };
    let (p00, p10, p01, p11) = (fetch(x0, y0), fetch(x0 + 1, y0), fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1));
    let (w00, w10, w01, w11) = ((1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy);
    std::array::from_fn(|c| p00[c] * w00 + p10[c] * w10 + p01[c] * w01 + p11[c] * w11)
}

/// Premultiplied copy of a straight picture.
fn premultiplied(src: &PixelBuf) -> PixelBuf {
    let mut p = src.clone();
    for px in p.data.as_chunks_mut::<4>().0.iter_mut() {
        let a = px[3];
        if a != 255 {
            px[0] = mul_u8(px[0], a);
            px[1] = mul_u8(px[1], a);
            px[2] = mul_u8(px[2], a);
        }
    }
    p
}

/// The content picture `src` transformed by `t` at render scale `s` into output pixels (4.7), clipped to `clip` when
/// given. The result covers `transformed_bounds(..) ∩ clip`; an empty source or an empty intersection gives an empty
/// buffer.
pub fn transform_picture(src: &PixelBuf, t: &LayerTransform, s: f32, clip: Option<IRect>) -> CroppedBuf {
    let mut t = *t;
    t.sanitize();
    let s = if s.is_finite() && s > 0.0 { s } else { 1.0 };
    let size = [src.width, src.height];
    if src.width == 0 || src.height == 0 || src.data.len() != src.width as usize * src.height as usize * 4 {
        return CroppedBuf::empty();
    }
    let mut rect = transformed_bounds(size, &t, s);
    if let Some(c) = clip {
        rect = rect.intersect(&c);
    }
    if rect.is_empty() {
        return CroppedBuf::empty();
    }
    if rect.area() > MAX_RESAMPLE_PIXELS {
        let rows = (MAX_RESAMPLE_PIXELS / rect.width() as u64).max(1) as i32;
        rect = IRect::new(rect.x0, rect.y0, rect.x1.min(rect.x0 + (MAX_RESAMPLE_PIXELS as i32)), rect.y0 + rows.min(rect.height() as i32));
    }
    if let Some(out) = copy_exact(src, &t, s as f64, rect) {
        return out;
    }

    // Level choice: the larger axis scale decides; reduce only when both axes are below 0.5.
    let need = (t.scale.x.abs() * s).max(t.scale.y.abs() * s);
    let mut level_k = 0u32;
    while need * (1u32 << (level_k + 1)) as f32 <= 1.0 && level_k < 30 {
        level_k += 1;
    }
    let level = if level_k == 0 {
        premultiplied(src)
    } else {
        let mips = PremulMips::build(src);
        let k = (level_k as usize).min(mips.levels.len() - 1);
        mips.levels.into_iter().nth(k).unwrap_or_else(|| premultiplied(src))
    };
    let (kx, ky) = (level.width as f32 / src.width as f32, level.height as f32 / src.height as f32);

    // Inverse map: output point -> content point.
    let c = Vec2::new(src.width as f32, src.height as f32) * 0.5;
    let (sin, cos) = t.rotation.sin_cos();
    let inv_scale = Vec2::new(1.0 / t.scale.x, 1.0 / t.scale.y);
    let to_content = |o: Vec2| -> Vec2 {
        let l = o / s - t.translate - c;
        // Inverse rotation.
        let r = Vec2::new(l.x * cos + l.y * sin, -l.x * sin + l.y * cos);
        c + r * inv_scale
    };
    let mut out = CroppedBuf::new(rect);
    let rw = rect.width() as usize;
    let do_row = |(ry, row): (usize, &mut [u8])| {
        let oy = (rect.y0 + ry as i32) as f32 + 0.5;
        for (k, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let ox = (rect.x0 + k as i32) as f32 + 0.5;
            let p = to_content(Vec2::new(ox, oy));
            let v = bilinear(&level, p.x * kx, p.y * ky);
            let a = v[3];
            let a8 = (a + 0.5).floor().clamp(0.0, 255.0) as u8;
            if a8 == 0 {
                continue;
            }
            for i in 0..3 {
                px[i] = (v[i] * 255.0 / a + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
            px[3] = a8;
        }
    };
    if out.data.len() / 4 >= PAR_PIXELS {
        out.data.par_chunks_mut(rw * 4).enumerate().for_each(do_row);
    } else {
        out.data.chunks_mut(rw * 4).enumerate().for_each(do_row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::XorShift;
    use std::f32::consts::FRAC_PI_2;

    fn random_picture(w: u32, h: u32, seed: u64) -> PixelBuf {
        let mut rng = XorShift::new(seed);
        PixelBuf::from_raw(w, h, (0..w * h * 4).map(|_| (rng.next_u32() % 256) as u8).collect()).unwrap()
    }

    #[test]
    fn k1_identity_and_whole_pixel_moves_copy_bytes() {
        let pic = random_picture(13, 7, 3);
        let id = transform_picture(&pic, &LayerTransform::IDENTITY, 1.0, None);
        assert_eq!(id.rect, IRect::new(0, 0, 13, 7));
        assert_eq!(id.data, pic.data, "identity copies bytes, RGB under alpha 0 included");
        let moved = transform_picture(&pic, &LayerTransform::moved(5.0, -3.0), 1.0, None);
        assert_eq!(moved.rect, IRect::new(5, -3, 18, 4));
        assert_eq!(moved.data, pic.data);
        // A flip copies mirrored pixels.
        let flip = LayerTransform { scale: Vec2::new(-1.0, 1.0), ..LayerTransform::IDENTITY };
        let f = transform_picture(&pic, &flip, 1.0, None);
        assert_eq!(f.rect, IRect::new(0, 0, 13, 7));
        for y in 0..7 {
            for x in 0..13 {
                assert_eq!(f.get(x, y), pic.get(12 - x as u32, y as u32));
            }
        }
        // Render scale 2 with layer scale 0.5 is a copy too; a clip cuts it.
        let half = LayerTransform { scale: Vec2::splat(0.5), translate: Vec2::new(-3.25, -1.75), ..LayerTransform::IDENTITY };
        let c = transform_picture(&pic, &half, 2.0, None);
        assert_eq!(c.data, pic.data, "{:?}", c.rect);
        let clipped = transform_picture(&pic, &LayerTransform::IDENTITY, 1.0, Some(IRect::new(2, 2, 5, 4)));
        assert_eq!(clipped.rect, IRect::new(2, 2, 5, 4));
        assert_eq!(clipped.get(3, 3), pic.get(3, 3));
        assert!(transform_picture(&pic, &LayerTransform::IDENTITY, 1.0, Some(IRect::new(50, 50, 60, 60))).is_empty());
        assert!(transform_picture(&PixelBuf::new(0, 4), &LayerTransform::IDENTITY, 1.0, None).is_empty());
    }

    #[test]
    fn k1_four_quarter_turns_are_the_identity() {
        // Even-by-odd sizes keep the grid only for some turns; a square of even side keeps it for all.
        let pic = random_picture(10, 10, 8);
        let turn = LayerTransform { rotation: FRAC_PI_2, ..LayerTransform::IDENTITY };
        let mut cur = pic.clone();
        for _ in 0..4 {
            let r = transform_picture(&cur, &turn, 1.0, None);
            assert_eq!(r.rect, IRect::new(0, 0, 10, 10));
            cur = r.to_dense(10, 10);
        }
        assert_eq!(cur, pic);
        // One quarter turn is clockwise: the top-left pixel goes to the top-right.
        let r = transform_picture(&pic, &turn, 1.0, None);
        assert_eq!(r.get(9, 0), pic.get(0, 0));
        // A rectangle turned about its centre: 6 x 4 -> 4 x 6 lands on the grid (centre (3, 2)).
        let rect = random_picture(6, 4, 2);
        let r = transform_picture(&rect, &turn, 1.0, None);
        assert_eq!(r.rect, IRect::new(1, -1, 5, 5));
        assert_eq!(r.get(4, -1), rect.get(0, 0));
    }

    #[test]
    fn k1_bilinear_and_mip_paths() {
        // A solid opaque picture scaled by 1.5 stays the same colour inside.
        let solid = PixelBuf::filled(20, 20, [10, 200, 30, 255]);
        let big = LayerTransform { scale: Vec2::splat(1.5), ..LayerTransform::IDENTITY };
        let r = transform_picture(&solid, &big, 1.0, None);
        assert_eq!(r.rect, IRect::new(-5, -5, 25, 25));
        assert_eq!(r.get(10, 10), [10, 200, 30, 255]);
        let corner = r.get(-5, -5)[3];
        assert!(corner > 0 && corner < 255, "a corner pixel is partly covered: {corner}");
        // Shrunk to a quarter: the box chain, still the same colour inside.
        let small = LayerTransform { scale: Vec2::splat(0.25), ..LayerTransform::IDENTITY };
        let r = transform_picture(&solid, &small, 1.0, None);
        assert_eq!(r.get(10, 10), [10, 200, 30, 255]);
        // A half-transparent colour keeps its straight RGB through premultiplied sampling.
        let ghost = PixelBuf::filled(16, 16, [200, 100, 50, 128]);
        let rot = LayerTransform { rotation: 0.3, ..LayerTransform::IDENTITY };
        let r = transform_picture(&ghost, &rot, 1.0, None);
        let want = crate::pixel_ops::unpremultiply(crate::pixel_ops::premultiply([200, 100, 50, 128]));
        assert_eq!(r.get(8, 8), want);
        // A transparent neighbour does not darken an edge (premultiplied).
        let mut half = PixelBuf::new(8, 8);
        for y in 0..8 {
            for x in 0..4 {
                half.set(x, y, [250, 250, 250, 255]);
            }
        }
        let shift = LayerTransform::moved(0.5, 0.0);
        let r = transform_picture(&half, &shift, 1.0, None);
        let edge = r.get(4, 3);
        assert_eq!(edge[3], 128);
        assert_eq!(&edge[..3], &[250, 250, 250]);
    }

    #[test]
    fn k1_resampler_is_thread_count_independent() {
        let pic = random_picture(400, 300, 5);
        let t = LayerTransform { translate: Vec2::new(3.3, -7.1), scale: Vec2::new(0.8, -1.3), rotation: 0.7 };
        let small = LayerTransform { scale: Vec2::new(0.2, 0.3), rotation: 0.1, ..LayerTransform::IDENTITY };
        let pools = [1usize, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
        for tr in [t, small] {
            let a = pools[0].install(|| transform_picture(&pic, &tr, 1.25, None));
            let b = pools[1].install(|| transform_picture(&pic, &tr, 1.25, None));
            assert!(!a.is_empty());
            assert_eq!(a, b);
        }
    }
}
