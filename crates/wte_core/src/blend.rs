// Owner: core-document (C2). Per-pixel layer blending (ARCHITECTURE.md 5.16, algorithm 6.6) and the blend-mode names
// of the export formats.
//
// All math is `u32` fixed point on the sRGB-encoded bytes (gamma-space blending, section 4.2). The source alpha is
// carried as `sa * opacity` in units of 1/65025, so a low layer opacity loses no precision; the result is within 1 LSB
// of the real-valued W3C formula for every mode.

use crate::layer::BlendMode;
use crate::math::div255;

/// `255 * 255`: the unit of the un-rounded source alpha `sa * opacity`.
const ONE: u32 = 255 * 255;

/// The separable blend function `B(Cb, Cs)` of a mode on bytes (0..=255).
#[inline(always)]
fn blend_fn(mode: BlendMode, cb: u32, cs: u32) -> u32 {
    match mode {
        BlendMode::Normal => cs,
        BlendMode::Multiply => div255(cb * cs),
        BlendMode::Screen => cb + cs - div255(cb * cs),
        // Cb <= 0.5 <=> Cb <= 127 on bytes; both products stay below 65536, the range in which div255 is exact.
        BlendMode::Overlay => {
            if cb <= 127 {
                div255(2 * cb * cs)
            } else {
                255 - div255(2 * (255 - cb) * (255 - cs))
            }
        }
        BlendMode::Add => (cb + cs).min(255),
        BlendMode::Darken => cb.min(cs),
        BlendMode::Lighten => cb.max(cs),
    }
}

/// The one compositing kernel. `normal` enables the copy fast path; `b` is the mode's blend function.
#[inline(always)]
fn kernel(d: [u8; 4], s: [u8; 4], opacity: u32, normal: bool, b: impl Fn(u32, u32) -> u32) -> [u8; 4] {
    // Source alpha times layer opacity in units of 1/65025.
    let a = s[3] as u32 * opacity;
    let ab = d[3] as u32;

    // Alpha-0 rule (6.6): RGB stored under alpha 0 is data and survives compositing.
    if ab == 0 {
        let as8 = div255(a) as u8;
        return if as8 > 0 || d == [0, 0, 0, 0] { [s[0], s[1], s[2], as8] } else { d };
    }
    if a == 0 {
        return d;
    }
    if normal && a == ONE {
        return s;
    }

    let mut out = [0u8; 4];
    if ab == 255 {
        // Opaque backdrop: Co = as * B + (1 - as) * Cb, no division by the result alpha.
        let ia = ONE - a;
        for c in 0..3 {
            let (cb, cs) = (d[c] as u32, s[c] as u32);
            out[c] = ((a * b(cb, cs) + ia * cb + ONE / 2) / ONE) as u8;
        }
        out[3] = 255;
    } else {
        // Weights in units of 1/255^3; they sum to `den` = ao * 255^3, so the quotient is the exact rounded Co.
        let w_src = (255 - ab) * a;
        let w_mix = ab * a;
        let w_dst = (ONE - a) * ab;
        let den = w_src + w_mix + w_dst;
        for c in 0..3 {
            let (cb, cs) = (d[c] as u32, s[c] as u32);
            // At most 255 * den <= 255^4, which (plus den / 2) still fits in u32.
            out[c] = ((w_src * cs + w_mix * b(cb, cs) + w_dst * cb + den / 2) / den) as u8;
        }
        out[3] = ((den + ONE / 2) / ONE) as u8;
    }
    out
}

#[inline(always)]
fn row_kernel(dst: &mut [u8], src: &[u8], opacity: u32, normal: bool, b: impl Fn(u32, u32) -> u32 + Copy) {
    let (d, _) = dst.as_chunks_mut::<4>();
    let (s, _) = src.as_chunks::<4>();
    for (d, s) in d.iter_mut().zip(s) {
        *d = kernel(*d, *s, opacity, normal, b);
    }
}

/// Composite one straight-alpha source pixel over a destination pixel (section 6.6). `opacity` = layer opacity 0..=255.
pub fn blend_pixel(dst: [u8; 4], src: [u8; 4], opacity: u8, mode: BlendMode) -> [u8; 4] {
    kernel(dst, src, opacity as u32, mode == BlendMode::Normal, |cb, cs| blend_fn(mode, cb, cs))
}

/// Same for a row: dst.len() == src.len(), multiple of 4.
///
/// Pixel for pixel identical to `blend_pixel`. If the lengths differ, only the common whole pixels are processed.
pub fn blend_row(dst: &mut [u8], src: &[u8], opacity: u8, mode: BlendMode) {
    debug_assert_eq!(dst.len(), src.len(), "blend_row: row lengths differ");
    debug_assert_eq!(dst.len() % 4, 0, "blend_row: row length is not a multiple of 4");
    let opacity = opacity as u32;
    // One monomorphised loop per mode, so the mode is not dispatched per pixel.
    match mode {
        BlendMode::Normal => row_kernel(dst, src, opacity, true, |_, cs| cs),
        BlendMode::Multiply => row_kernel(dst, src, opacity, false, |cb, cs| blend_fn(BlendMode::Multiply, cb, cs)),
        BlendMode::Screen => row_kernel(dst, src, opacity, false, |cb, cs| blend_fn(BlendMode::Screen, cb, cs)),
        BlendMode::Overlay => row_kernel(dst, src, opacity, false, |cb, cs| blend_fn(BlendMode::Overlay, cb, cs)),
        BlendMode::Add => row_kernel(dst, src, opacity, false, |cb, cs| blend_fn(BlendMode::Add, cb, cs)),
        BlendMode::Darken => row_kernel(dst, src, opacity, false, |cb, cs| blend_fn(BlendMode::Darken, cb, cs)),
        BlendMode::Lighten => row_kernel(dst, src, opacity, false, |cb, cs| blend_fn(BlendMode::Lighten, cb, cs)),
    }
}

impl BlendMode {
    /// Every blend mode, in menu order.
    pub const ALL: [BlendMode; 7] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Add,
        BlendMode::Darken,
        BlendMode::Lighten,
    ];

    /// UI label: "Normal", "Multiply", ...
    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Add => "Add",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
        }
    }

    /// PSD blend-mode key: norm, "mul ", scrn, over, lddg, dark, lite.
    pub fn psd_key(self) -> [u8; 4] {
        match self {
            BlendMode::Normal => *b"norm",
            BlendMode::Multiply => *b"mul ",
            BlendMode::Screen => *b"scrn",
            BlendMode::Overlay => *b"over",
            BlendMode::Add => *b"lddg",
            BlendMode::Darken => *b"dark",
            BlendMode::Lighten => *b"lite",
        }
    }

    /// OpenRaster composite-op: svg:src-over, svg:multiply, svg:screen, svg:overlay, svg:plus, svg:darken, svg:lighten.
    pub fn ora_op(self) -> &'static str {
        match self {
            BlendMode::Normal => "svg:src-over",
            BlendMode::Multiply => "svg:multiply",
            BlendMode::Screen => "svg:screen",
            BlendMode::Overlay => "svg:overlay",
            BlendMode::Add => "svg:plus",
            BlendMode::Darken => "svg:darken",
            BlendMode::Lighten => "svg:lighten",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::XorShift;

    /// Real-valued W3C formula of section 6.6 with the alpha-0 rule; returns RGBA scaled to 0..=255 (not rounded).
    fn reference(dst: [u8; 4], src: [u8; 4], opacity: u8, mode: BlendMode) -> [f64; 4] {
        let f = |v: u8| v as f64 / 255.0;
        let a_s = f(src[3]) * f(opacity);
        let a_b = f(dst[3]);
        let as8 = (a_s * 255.0).round();
        let keep = [dst[0] as f64, dst[1] as f64, dst[2] as f64, dst[3] as f64];
        if dst[3] == 0 {
            return if as8 > 0.0 || dst == [0; 4] { [src[0] as f64, src[1] as f64, src[2] as f64, as8] } else { keep };
        }
        if a_s == 0.0 {
            return keep;
        }
        let a_o = a_s + a_b * (1.0 - a_s);
        let mut out = [0.0; 4];
        for c in 0..3 {
            let (cb, cs) = (f(dst[c]), f(src[c]));
            let b = match mode {
                BlendMode::Normal => cs,
                BlendMode::Multiply => cb * cs,
                BlendMode::Screen => cb + cs - cb * cs,
                BlendMode::Overlay => {
                    if cb <= 0.5 {
                        2.0 * cb * cs
                    } else {
                        1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
                    }
                }
                BlendMode::Add => (cb + cs).min(1.0),
                BlendMode::Darken => cb.min(cs),
                BlendMode::Lighten => cb.max(cs),
            };
            let co = ((1.0 - a_b) * a_s * cs + a_b * a_s * b + (1.0 - a_s) * a_b * cb) / a_o;
            out[c] = co * 255.0;
        }
        out[3] = a_o * 255.0;
        out
    }

    fn assert_close(dst: [u8; 4], src: [u8; 4], opacity: u8, mode: BlendMode) {
        let got = blend_pixel(dst, src, opacity, mode);
        let want = reference(dst, src, opacity, mode);
        for c in 0..4 {
            assert!(
                (got[c] as f64 - want[c]).abs() <= 1.0 + 1e-9,
                "{mode:?} dst {dst:?} src {src:?} opacity {opacity}: got {got:?}, reference {want:?}"
            );
        }
    }

    /// Random bytes biased towards the extremes, where the special cases live.
    fn edgy(rng: &mut XorShift) -> u8 {
        match rng.next_u32() % 8 {
            0 => 0,
            1 => 255,
            2 => 1,
            3 => 254,
            4 => 127 + (rng.next_u32() % 2) as u8,
            _ => (rng.next_u32() & 0xff) as u8,
        }
    }

    #[test]
    fn every_mode_is_within_one_lsb_of_the_f64_reference() {
        let mut rng = XorShift::new(0xb1e4d);
        for mode in BlendMode::ALL {
            for _ in 0..60_000 {
                let dst = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), edgy(&mut rng)];
                let src = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), edgy(&mut rng)];
                assert_close(dst, src, edgy(&mut rng), mode);
            }
        }
    }

    #[test]
    fn exhaustive_alpha_pairs_on_fixed_colours() {
        // Every (source alpha, backdrop alpha) pair at three opacities: covers the rounding of the result alpha.
        for mode in BlendMode::ALL {
            for opacity in [255u8, 128, 3] {
                for sa in 0..=255u8 {
                    for ab in 0..=255u8 {
                        assert_close([200, 30, 128, ab], [10, 250, 127, sa], opacity, mode);
                    }
                }
            }
        }
    }

    #[test]
    fn opaque_backdrop_stays_opaque_and_alpha_never_drops() {
        let mut rng = XorShift::new(77);
        for mode in BlendMode::ALL {
            for _ in 0..20_000 {
                let dst = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), edgy(&mut rng)];
                let src = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), edgy(&mut rng)];
                let out = blend_pixel(dst, src, edgy(&mut rng), mode);
                assert!(out[3] >= dst[3], "{mode:?} {dst:?} {src:?} -> {out:?}");
                if dst[3] == 255 {
                    assert_eq!(out[3], 255);
                }
            }
        }
    }

    #[test]
    fn normal_at_full_alpha_copies_the_source() {
        let mut rng = XorShift::new(5);
        for _ in 0..5_000 {
            let dst = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), edgy(&mut rng)];
            let src = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), 255];
            assert_eq!(blend_pixel(dst, src, 255, BlendMode::Normal), src);
        }
    }

    #[test]
    fn alpha_zero_rule() {
        for mode in BlendMode::ALL {
            // as > 0, ab == 0: the source colour with the source alpha, whatever the mode and the hidden backdrop colour.
            assert_eq!(blend_pixel([9, 8, 7, 0], [10, 20, 30, 200], 255, mode), [10, 20, 30, 200]);
            assert_eq!(blend_pixel([0, 0, 0, 0], [10, 20, 30, 200], 128, mode), [10, 20, 30, div255(200 * 128) as u8]);
            // as == 0, ab > 0: the accumulator is unchanged.
            assert_eq!(blend_pixel([1, 2, 3, 4], [250, 240, 230, 0], 255, mode), [1, 2, 3, 4]);
            assert_eq!(blend_pixel([1, 2, 3, 255], [250, 240, 230, 255], 0, mode), [1, 2, 3, 255]);
            // as == 0, ab == 0: an all-zero accumulator takes the hidden colour ...
            assert_eq!(blend_pixel([0, 0, 0, 0], [10, 20, 30, 0], 255, mode), [10, 20, 30, 0]);
            assert_eq!(blend_pixel([0, 0, 0, 0], [10, 20, 30, 255], 0, mode), [10, 20, 30, 0]);
            // ... but an accumulator that already carries a hidden colour keeps it (the bottom-most layer wins),
            assert_eq!(blend_pixel([10, 20, 30, 0], [0, 0, 0, 0], 255, mode), [10, 20, 30, 0]);
            assert_eq!(blend_pixel([10, 20, 30, 0], [99, 98, 97, 0], 255, mode), [10, 20, 30, 0]);
            // also when the source alpha only rounds to zero.
            assert_eq!(blend_pixel([10, 20, 30, 0], [99, 98, 97, 1], 100, mode), [10, 20, 30, 0]);
        }
    }

    #[test]
    fn a_source_with_alpha_zero_never_changes_a_visible_accumulator() {
        let mut rng = XorShift::new(31);
        for mode in BlendMode::ALL {
            for _ in 0..10_000 {
                let dst = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), 1 + (rng.next_u32() % 255) as u8];
                let src = [edgy(&mut rng), edgy(&mut rng), edgy(&mut rng), 0];
                assert_eq!(blend_pixel(dst, src, edgy(&mut rng), mode), dst);
            }
        }
    }

    #[test]
    fn row_equals_pixel() {
        let mut rng = XorShift::new(1234);
        for mode in BlendMode::ALL {
            for opacity in [255u8, 200, 1, 0] {
                let n = 257;
                let dst: Vec<u8> = (0..n * 4).map(|_| edgy(&mut rng)).collect();
                let src: Vec<u8> = (0..n * 4).map(|_| edgy(&mut rng)).collect();
                let mut row = dst.clone();
                blend_row(&mut row, &src, opacity, mode);
                for i in 0..n {
                    let d = [dst[i * 4], dst[i * 4 + 1], dst[i * 4 + 2], dst[i * 4 + 3]];
                    let s = [src[i * 4], src[i * 4 + 1], src[i * 4 + 2], src[i * 4 + 3]];
                    assert_eq!(row[i * 4..i * 4 + 4], blend_pixel(d, s, opacity, mode), "{mode:?} pixel {i}");
                }
            }
        }
        // An empty row is fine.
        blend_row(&mut [], &[], 255, BlendMode::Normal);
    }

    #[test]
    fn known_values() {
        // 50% white over opaque black, Normal.
        assert_eq!(blend_pixel([0, 0, 0, 255], [255, 255, 255, 128], 255, BlendMode::Normal), [128, 128, 128, 255]);
        // Multiply, Screen, Add, Darken, Lighten on opaque pixels.
        let (d, s) = ([100, 200, 50, 255], [200, 100, 250, 255]);
        assert_eq!(blend_pixel(d, s, 255, BlendMode::Multiply), [78, 78, 49, 255]);
        assert_eq!(blend_pixel(d, s, 255, BlendMode::Screen), [222, 222, 251, 255]);
        assert_eq!(blend_pixel(d, s, 255, BlendMode::Add), [255, 255, 255, 255]);
        assert_eq!(blend_pixel(d, s, 255, BlendMode::Darken), [100, 100, 50, 255]);
        assert_eq!(blend_pixel(d, s, 255, BlendMode::Lighten), [200, 200, 250, 255]);
        // Overlay: dark backdrop multiplies, light backdrop screens.
        assert_eq!(blend_pixel([64, 191, 0, 255], [128, 128, 77, 255], 255, BlendMode::Overlay), [64, 191, 0, 255]);
        // Two half-transparent pixels: ao = 0.5 + 0.5 * 0.5.
        let out = blend_pixel([255, 0, 0, 128], [0, 0, 255, 128], 255, BlendMode::Normal);
        assert_eq!(out[3], 192);
        assert!((out[0] as i32 - 85).abs() <= 1 && (out[2] as i32 - 170).abs() <= 1, "{out:?}");
    }

    #[test]
    fn names_are_distinct_and_as_specified() {
        let keys: Vec<[u8; 4]> = BlendMode::ALL.iter().map(|m| m.psd_key()).collect();
        assert_eq!(keys, [*b"norm", *b"mul ", *b"scrn", *b"over", *b"lddg", *b"dark", *b"lite"]);
        let ops: Vec<&str> = BlendMode::ALL.iter().map(|m| m.ora_op()).collect();
        assert_eq!(
            ops,
            ["svg:src-over", "svg:multiply", "svg:screen", "svg:overlay", "svg:plus", "svg:darken", "svg:lighten"]
        );
        let labels: Vec<&str> = BlendMode::ALL.iter().map(|m| m.label()).collect();
        assert_eq!(labels, ["Normal", "Multiply", "Screen", "Overlay", "Add", "Darken", "Lighten"]);
    }
}
