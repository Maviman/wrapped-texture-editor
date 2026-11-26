// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Non-destructive adjustments of a sticker layer's pixels (STICKER_PLAN 2.4, 4.6; ARCHITECTURE 6.11). Per pixel on
//! straight RGB, alpha untouched; pixels with alpha 0 keep their RGB. Every adjustment except Hue / Saturation and
//! Black & White is one 256-entry table per channel. Settings that change nothing return the bytes unchanged.

use rayon::prelude::*;

use crate::pixel::PixelBuf;

/// Pictures with at least this many pixels are adjusted in parallel.
const PAR_PIXELS: usize = 1 << 16;

/// One adjustment (2.4).
#[derive(Clone, Debug, PartialEq)]
pub enum Adjustment {
    /// Brightness and contrast, each -100..=100.
    BrightnessContrast { brightness: f32, contrast: f32 },
    /// Hue shift -180..=180 degrees, saturation and lightness -1..=1; Colorize sets the hue instead of shifting it.
    HueSaturation { hue_deg: f32, saturation: f32, lightness: f32, colorize: bool },
    /// Input black / white points, midtone gamma 0.1..=9.99, output black / white points.
    Levels { in_black: u8, in_white: u8, gamma: f32, out_black: u8, out_white: u8 },
    Invert,
    BlackWhite,
    /// 2..=32 levels per channel.
    Posterize { levels: u8 },
    /// Luma at or above `level` -> white, else black.
    Threshold { level: u8 },
}

/// Rounds a 0..=1 value to a byte, half up.
#[inline]
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8
}

/// Brightness curve (4.6): a line of slope `1.375^(b/50)` through the origin, rolled off to (1, 1) by a `v^P`
/// white-anchor term; `b = 0` is the identity.
fn brightness_curve(v: f32, b: f32) -> f32 {
    if b == 0.0 {
        return v;
    }
    let s = 1.375f32.powf(b / 50.0);
    let p = if b >= 0.0 { (4.5 - 0.013 * b).max(2.0) } else { 5.0 - 0.072 * b };
    let v = v.clamp(0.0, 1.0);
    (s * v + (1.0 - s) * v.powf(p)).clamp(0.0, 1.0)
}

/// Contrast curve (4.6): a symmetric cubic-Hermite S-curve about 0.5 with centre slope `1 + c/128` and end slope
/// `1 - c/128` (clamped at 0); pins 0, 0.5 and 1; `c = 0` is the identity.
fn contrast_curve(v: f32, c: f32) -> f32 {
    if c == 0.0 {
        return v;
    }
    let k = c / 128.0;
    let (end, mid) = ((1.0 - k).max(0.0), 1.0 + k);
    let half = |t: f32| {
        end * 0.5 * (t * t * t - 2.0 * t * t + t) + (-2.0 * t * t * t + 3.0 * t * t) * 0.5 + mid * 0.5 * (t * t * t - t * t)
    };
    let v = v.clamp(0.0, 1.0);
    if v <= 0.5 { half(v / 0.5) } else { 1.0 - half((1.0 - v) / 0.5) }
}

/// Straight RGB (0..=1) -> HSL (hue 0..1).
fn rgb_to_hsl(c: [f32; 3]) -> (f32, f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d <= 0.0 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == c[0] {
        (c[1] - c[2]) / d + if c[1] < c[2] { 6.0 } else { 0.0 }
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    (h / 6.0, s, l)
}

/// HSL (hue 0..1) -> straight RGB (0..=1).
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0)]
}

/// Luma `round(0.299 r + 0.587 g + 0.114 b)` (Black & White, Threshold).
#[inline]
fn luma(r: u8, g: u8, b: u8) -> u8 {
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 + 0.5).floor().clamp(0.0, 255.0) as u8
}

impl Adjustment {
    /// Clamps every number into its range (2.4); non-finite values become the neutral setting.
    pub fn sanitize(&mut self) {
        let fin = |v: &mut f32, lo: f32, hi: f32, neutral: f32| *v = if v.is_finite() { v.clamp(lo, hi) } else { neutral };
        match self {
            Adjustment::BrightnessContrast { brightness, contrast } => {
                fin(brightness, -100.0, 100.0, 0.0);
                fin(contrast, -100.0, 100.0, 0.0);
            }
            Adjustment::HueSaturation { hue_deg, saturation, lightness, .. } => {
                fin(hue_deg, -180.0, 180.0, 0.0);
                fin(saturation, -1.0, 1.0, 0.0);
                fin(lightness, -1.0, 1.0, 0.0);
            }
            Adjustment::Levels { gamma, .. } => fin(gamma, 0.1, 9.99, 1.0),
            Adjustment::Posterize { levels } => *levels = (*levels).clamp(2, 32),
            Adjustment::Invert | Adjustment::BlackWhite | Adjustment::Threshold { .. } => {}
        }
    }

    /// True when the settings leave every pixel unchanged.
    pub fn is_identity(&self) -> bool {
        match *self {
            Adjustment::BrightnessContrast { brightness, contrast } => brightness == 0.0 && contrast == 0.0,
            Adjustment::HueSaturation { hue_deg, saturation, lightness, colorize } => {
                !colorize && hue_deg == 0.0 && saturation == 0.0 && lightness == 0.0
            }
            Adjustment::Levels { in_black, in_white, gamma, out_black, out_white } => {
                in_black == 0 && in_white == 255 && gamma == 1.0 && out_black == 0 && out_white == 255
            }
            Adjustment::Invert | Adjustment::BlackWhite | Adjustment::Threshold { .. } => false,
            Adjustment::Posterize { .. } => false,
        }
    }

    /// The 256-entry table of a per-channel adjustment; `None` for Hue / Saturation and Black & White.
    pub fn lut(&self) -> Option<[u8; 256]> {
        let mut a = self.clone();
        a.sanitize();
        let table = |f: &dyn Fn(u8) -> u8| -> [u8; 256] { std::array::from_fn(|x| f(x as u8)) };
        match a {
            Adjustment::BrightnessContrast { brightness, contrast } => Some(table(&|x| {
                byte(contrast_curve(brightness_curve(x as f32 / 255.0, brightness), contrast))
            })),
            Adjustment::Levels { in_black, in_white, gamma, out_black, out_white } => Some(table(&|x| {
                let range = (in_white as f32 - in_black as f32).max(1.0);
                let v = ((x as f32 - in_black as f32) / range).clamp(0.0, 1.0).powf(1.0 / gamma);
                let o = out_black as f32 + v * (out_white as f32 - out_black as f32);
                (o + 0.5).floor().clamp(0.0, 255.0) as u8
            })),
            Adjustment::Invert => Some(table(&|x| 255 - x)),
            Adjustment::Posterize { levels } => {
                let n = levels as u32;
                Some(table(&|x| {
                    let bin = (x as u32 * n / 256).min(n - 1);
                    ((bin * 255 * 2 + (n - 1)) / (2 * (n - 1))) as u8
                }))
            }
            Adjustment::Threshold { .. } | Adjustment::HueSaturation { .. } | Adjustment::BlackWhite => None,
        }
    }

    /// The adjustment of one straight RGB triple.
    fn pixel(&self, lut: Option<&[u8; 256]>, rgb: [u8; 3]) -> [u8; 3] {
        if let Some(t) = lut {
            return [t[rgb[0] as usize], t[rgb[1] as usize], t[rgb[2] as usize]];
        }
        match *self {
            Adjustment::BlackWhite => {
                let y = luma(rgb[0], rgb[1], rgb[2]);
                [y, y, y]
            }
            Adjustment::Threshold { level } => {
                let v = if luma(rgb[0], rgb[1], rgb[2]) >= level { 255 } else { 0 };
                [v, v, v]
            }
            Adjustment::HueSaturation { hue_deg, saturation, lightness, colorize } => {
                let c = [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0];
                let (mut h, mut s, l) = rgb_to_hsl(c);
                if colorize {
                    h = hue_deg.rem_euclid(360.0) / 360.0;
                    s = 0.25 + 0.75 * saturation.max(0.0);
                } else {
                    h = (h + hue_deg / 360.0).rem_euclid(1.0);
                    s = if saturation <= 0.0 { s * (1.0 + saturation) } else { s + (1.0 - s) * saturation };
                }
                let mut out = hsl_to_rgb(h, s.clamp(0.0, 1.0), l);
                if lightness > 0.0 {
                    out = out.map(|v| v + (1.0 - v) * lightness);
                } else if lightness < 0.0 {
                    out = out.map(|v| v * (1.0 + lightness));
                }
                out.map(byte)
            }
            _ => rgb,
        }
    }

    /// Applies the adjustment to `buf` in place (alpha untouched, pixels with alpha 0 left as they are).
    pub fn apply(&self, buf: &mut PixelBuf) {
        let mut a = self.clone();
        a.sanitize();
        if a.is_identity() {
            return;
        }
        let lut = a.lut();
        let work = |chunk: &mut [u8]| {
            for px in chunk.as_chunks_mut::<4>().0.iter_mut() {
                if px[3] == 0 {
                    continue;
                }
                let o = a.pixel(lut.as_ref(), [px[0], px[1], px[2]]);
                px[..3].copy_from_slice(&o);
            }
        };
        if buf.data.len() / 4 >= PAR_PIXELS {
            buf.data.par_chunks_mut(1 << 16).for_each(work);
        } else {
            work(&mut buf.data);
        }
    }
}

/// Applies `list` in order to `buf` (4.1 step 1a).
pub fn apply_adjustments(buf: &mut PixelBuf, list: &[Adjustment]) {
    for a in list {
        a.apply(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::XorShift;

    fn one(rgb: [u8; 3], a: &Adjustment) -> [u8; 3] {
        let mut b = PixelBuf::filled(1, 1, [rgb[0], rgb[1], rgb[2], 200]);
        a.apply(&mut b);
        assert_eq!(b.data[3], 200, "alpha untouched");
        [b.data[0], b.data[1], b.data[2]]
    }

    fn random_picture(seed: u64) -> PixelBuf {
        let mut rng = XorShift::new(seed);
        let data = (0..300 * 300 * 4).map(|_| (rng.next_u32() % 256) as u8).collect();
        PixelBuf::from_raw(300, 300, data).unwrap()
    }

    #[test]
    fn k1_identity_settings_are_byte_identical() {
        let pic = random_picture(1);
        for a in [
            Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0 },
            Adjustment::HueSaturation { hue_deg: 0.0, saturation: 0.0, lightness: 0.0, colorize: false },
            Adjustment::Levels { in_black: 0, in_white: 255, gamma: 1.0, out_black: 0, out_white: 255 },
        ] {
            let mut b = pic.clone();
            a.apply(&mut b);
            assert_eq!(b, pic, "{a:?}");
            // The tables themselves are the identity too.
            if let Some(t) = a.lut() {
                assert!(t.iter().enumerate().all(|(i, &v)| v as usize == i), "{a:?}");
            }
        }
        // Invert twice is the identity; alpha-0 pixels keep their RGB.
        let mut b = pic.clone();
        apply_adjustments(&mut b, &[Adjustment::Invert, Adjustment::Invert]);
        assert_eq!(b, pic);
        let mut z = PixelBuf::from_raw(1, 1, vec![10, 20, 30, 0]).unwrap();
        Adjustment::Invert.apply(&mut z);
        assert_eq!(z.data, vec![10, 20, 30, 0]);
    }

    #[test]
    fn k1_adjustments_against_hand_computed_values() {
        assert_eq!(one([0, 100, 255], &Adjustment::Invert), [255, 155, 0]);
        // Luma 0.299 * 200 + 0.587 * 100 + 0.114 * 50 = 124.2 -> 124.
        assert_eq!(one([200, 100, 50], &Adjustment::BlackWhite), [124, 124, 124]);
        assert_eq!(one([200, 100, 50], &Adjustment::Threshold { level: 124 }), [255, 255, 255]);
        assert_eq!(one([200, 100, 50], &Adjustment::Threshold { level: 125 }), [0, 0, 0]);
        // Posterize 2: bins 0..127 -> 0, 128..255 -> 255; 4: x * 4 / 256 -> 0, 85, 170, 255.
        assert_eq!(one([127, 128, 255], &Adjustment::Posterize { levels: 2 }), [0, 255, 255]);
        assert_eq!(one([63, 64, 200], &Adjustment::Posterize { levels: 4 }), [0, 85, 255]);
        assert_eq!(one([128, 191, 192], &Adjustment::Posterize { levels: 4 }), [170, 170, 255]);
        // Levels: input 50..200, gamma 1: (100 - 50) / 150 * 255 = 85; output 20..220: 20 + v * 200.
        let lv = Adjustment::Levels { in_black: 50, in_white: 200, gamma: 1.0, out_black: 0, out_white: 255 };
        assert_eq!(one([100, 40, 210], &lv), [85, 0, 255]);
        let lv = Adjustment::Levels { in_black: 0, in_white: 255, gamma: 2.0, out_black: 20, out_white: 220 };
        // 64 / 255 = 0.25098, sqrt = 0.50098, 20 + 100.196 = 120.196 -> 120.
        assert_eq!(one([64, 0, 255], &lv), [120, 20, 220]);
        // Hue / saturation: a hue turn of 120 degrees, saturation -1 to grey (L = 0.5 -> 127.5 -> 128), lightness
        // half way to white, Colorize (hue -120 = 240, saturation 0.25 from grey L 0.5: 95.6 / 95.6 / 159.4).
        let hs = |h: f32, s: f32, l: f32, c: bool| Adjustment::HueSaturation { hue_deg: h, saturation: s, lightness: l, colorize: c };
        assert_eq!(one([255, 0, 0], &hs(120.0, 0.0, 0.0, false)), [0, 255, 0]);
        assert_eq!(one([255, 0, 0], &hs(-120.0, 0.0, 0.0, false)), [0, 0, 255]);
        assert_eq!(one([255, 0, 0], &hs(0.0, -1.0, 0.0, false)), [128, 128, 128]);
        assert_eq!(one([0, 0, 0], &hs(0.0, 0.0, 0.5, false)), [128, 128, 128]);
        assert_eq!(one([200, 100, 50], &hs(0.0, 0.0, -1.0, false)), [0, 0, 0]);
        assert_eq!(one([128, 128, 128], &hs(-120.0, 0.0, 0.0, true)), [96, 96, 160]);
        // Saturation +1 of a half-saturated colour is full saturation: hsl(0, 0.5, 0.5) = (191, 64, 64) -> red at L 0.5.
        assert_eq!(one([191, 64, 64], &hs(0.0, 1.0, 0.0, false)), [255, 0, 0]);
        // Brightness / contrast pin black and white and move the middle the right way.
        let bc = |b: f32, c: f32| Adjustment::BrightnessContrast { brightness: b, contrast: c };
        for a in [bc(60.0, 0.0), bc(-60.0, 0.0), bc(0.0, 80.0), bc(0.0, -80.0), bc(40.0, 40.0)] {
            assert_eq!(one([0, 255, 0], &a)[..2], [0, 255], "{a:?}");
        }
        assert!(one([100, 0, 0], &bc(60.0, 0.0))[0] > 100);
        assert!(one([100, 0, 0], &bc(-60.0, 0.0))[0] < 100);
        // Brightness 50: slope 1.375 at black, so 10 -> about 13.7 (minus the tiny v^P term) -> 14.
        assert_eq!(one([10, 0, 0], &bc(50.0, 0.0))[0], 14);
        // Contrast pins the middle and steepens it: 64 darker, 192 lighter, 128 -> 128.5 -> 129 at most.
        let c = one([64, 192, 128], &bc(0.0, 80.0));
        assert!(c[0] < 64 && c[1] > 192 && (128..=129).contains(&c[2]), "{c:?}");
        let c = one([64, 192, 128], &bc(0.0, -80.0));
        assert!(c[0] > 64 && c[1] < 192, "{c:?}");
        // Tables are monotone for brightness / contrast / levels.
        for a in [bc(80.0, 50.0), bc(-90.0, -90.0), Adjustment::Levels { in_black: 30, in_white: 220, gamma: 0.4, out_black: 10, out_white: 240 }] {
            let t = a.lut().unwrap();
            assert!(t.windows(2).all(|w| w[0] <= w[1]), "{a:?}");
        }
    }

    #[test]
    fn k1_adjustments_sanitize_and_parallel_path() {
        let mut a = Adjustment::BrightnessContrast { brightness: f32::NAN, contrast: 500.0 };
        a.sanitize();
        assert_eq!(a, Adjustment::BrightnessContrast { brightness: 0.0, contrast: 100.0 });
        let mut p = Adjustment::Posterize { levels: 0 };
        p.sanitize();
        assert_eq!(p, Adjustment::Posterize { levels: 2 });
        let mut l = Adjustment::Levels { in_black: 200, in_white: 100, gamma: 0.0, out_black: 0, out_white: 255 };
        l.sanitize();
        assert!(l.lut().is_some(), "a reversed input range does not panic");
        // The parallel path equals the serial one pixel by pixel.
        let pic = random_picture(9);
        let hs = Adjustment::HueSaturation { hue_deg: 33.0, saturation: 0.4, lightness: -0.2, colorize: false };
        let mut big = pic.clone();
        hs.apply(&mut big);
        for y in (0..300).step_by(37) {
            for x in (0..300).step_by(29) {
                let p = pic.get(x, y);
                let mut small = PixelBuf::from_raw(1, 1, p.to_vec()).unwrap();
                hs.apply(&mut small);
                assert_eq!(big.get(x, y), small.get(0, 0));
            }
        }
    }
}
