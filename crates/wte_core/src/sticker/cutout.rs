// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Cut-out helpers (STICKER_PLAN 4.5; ARCHITECTURE 5.27, 6.12): colour to alpha, removing a white or black matte,
//! defringe, and removing a plain background. Classic image maths only ("no AI", plan 0.4 rule 6): no model, no
//! network, deterministic for any thread count.
//!
//! Every command takes an optional selection of the picture's size; with one, the result is mixed with the original
//! by `selection / 255` per pixel (premultiplied, `select::mix_px`); a selection of another size changes nothing.

use rayon::prelude::*;

use crate::math::mul_u8;
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::sticker::select::{color_diff, flood_plane, mix_px, mix_u8, sel_at};
use crate::text::edt::{distance_to_ink, nearest_ink};

/// Longest defringe width (pixels, Sticker menu: 1..=10).
pub const MAX_DEFRINGE: u32 = 10;
/// Default tolerance of Remove Background.
pub const DEFAULT_BG_TOLERANCE: u8 = 40;
/// Width of the soft-edge band of Remove Background (pixels outside the region).
pub const BG_SOFT_BAND: f32 = 2.0;
/// Share of the border a background colour must hold (percent).
const BG_MIN_SHARE: usize = 15;
/// Most background colours Remove Background looks for.
const BG_MAX_COLORS: usize = 3;

/// Runs `f(index, pixel)` over every pixel, in parallel for large pictures, mixing the result with the original by
/// the selection (`None` = all).
fn per_pixel(pixels: &mut PixelBuf, sel: Option<&AlphaBuf>, f: impl Fn(usize, [u8; 4]) -> [u8; 4] + Sync) {
    let len = pixels.data.len() / 4;
    pixels.data.par_chunks_mut(4).with_min_len(1 << 14).enumerate().for_each(|(i, p)| {
        let k = sel_at(sel, len, i);
        if k == 0 {
            return;
        }
        let before = [p[0], p[1], p[2], p[3]];
        let after = f(i, before);
        p.copy_from_slice(&mix_px(before, after, k));
    });
}

/// `v` (0..=1) as a byte, rounded half up.
#[inline]
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Colour to alpha of one straight pixel against `key` (4.5): the most transparent `a` for which the pixel is a mix
/// of some in-range colour and `key`; thresholds `t0 < t1` remap `a` linearly (`0`, `1` = as computed). Returns the
/// pixel with its RGB unmixed and its alpha multiplied by `a'`, and `a'` as a byte. A pixel equal to the key becomes
/// fully transparent (RGB = the key); a fully transparent pixel is returned as it is.
pub fn unmix_pixel(p: [u8; 4], key: [u8; 3], t0: f32, t1: f32) -> ([u8; 4], u8) {
    if p[3] == 0 {
        return (p, 255);
    }
    let mut a = 0.0f32;
    for i in 0..3 {
        let (pi, ci) = (p[i] as f32 / 255.0, key[i] as f32 / 255.0);
        let ai = if pi > ci {
            if ci < 1.0 { (pi - ci) / (1.0 - ci) } else { 1.0 }
        } else if pi < ci {
            if ci > 0.0 { (ci - pi) / ci } else { 1.0 }
        } else {
            0.0
        };
        a = a.max(ai);
    }
    let a = remap(a.clamp(0.0, 1.0), t0, t1);
    if a >= 1.0 {
        return (p, 255);
    }
    if a <= 0.0 {
        return ([key[0], key[1], key[2], 0], 0);
    }
    let rgb: [u8; 3] = std::array::from_fn(|i| {
        let (pi, ci) = (p[i] as f32 / 255.0, key[i] as f32 / 255.0);
        byte((pi - ci) / a + ci)
    });
    let ab = byte(a);
    ([rgb[0], rgb[1], rgb[2], byte(a * p[3] as f32 / 255.0)], ab)
}

/// The thresholds of colour to alpha: `a <= t0` -> 0, `a >= t1` -> 1, linear between (sanitised: finite, `0 <= t0 <
/// t1 <= 1`; a bad pair means `0, 1`).
fn remap(a: f32, t0: f32, t1: f32) -> f32 {
    let (t0, t1) = if t0.is_finite() && t1.is_finite() && 0.0 <= t0 && t0 < t1 && t1 <= 1.0 { (t0, t1) } else { (0.0, 1.0) };
    if a <= t0 {
        0.0
    } else if a >= t1 {
        1.0
    } else {
        ((a - t0) / (t1 - t0)).clamp(0.0, 1.0)
    }
}

/// Colour to Alpha (4.5) on the whole picture (or the selection).
pub fn color_to_alpha(pixels: &mut PixelBuf, key: [u8; 3], t0: f32, t1: f32, sel: Option<&AlphaBuf>) {
    per_pixel(pixels, sel, |_, p| unmix_pixel(p, key, t0, t1).0);
}

/// Which matte a picture was composited on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Matte {
    White,
    Black,
}

/// Remove White Matte / Remove Black Matte (4.5): `C = clamp((P - (1 - a)) / a)` or `C = clamp(P / a)` with `a` the
/// pixel's alpha; alpha unchanged; pixels with alpha 0 (and opaque ones, where nothing changes) are left.
pub fn remove_matte(pixels: &mut PixelBuf, matte: Matte, sel: Option<&AlphaBuf>) {
    per_pixel(pixels, sel, |_, p| {
        if p[3] == 0 || p[3] == 255 {
            return p;
        }
        let a = p[3] as f32 / 255.0;
        let c = |v: u8| {
            let v = v as f32 / 255.0;
            byte(match matte {
                Matte::White => (v - (1.0 - a)) / a,
                Matte::Black => v / a,
            })
        };
        [c(p[0]), c(p[1]), c(p[2]), p[3]]
    });
}

/// Defringe (4.5, as corrected by the P2 check): the edge pixels (alpha > 0 and within `width` of a fully transparent
/// pixel, `d0 <= width` with `d0 = distance_to_ink(alpha == 0)`) take the RGB of their nearest interior pixel (alpha >
/// 0 and `d0 > width`), when that pixel is at most `width + 2` px away (centre to centre); alpha unchanged. The
/// picture's own border is not an edge (an opaque photo has no fringe). Without any transparent or interior pixel
/// nothing changes. Returns the number of pixels whose RGB changed.
///
/// The plan's literal rule (edge = `dist_in <= width` on the 128 threshold, interior = alpha >= 250) made every pixel
/// with alpha < 128 an edge pixel at any depth, so a soft shadow or glow anywhere in the picture took the colour of the
/// nearest opaque object (check P2). Measuring the depth from the fully transparent pixels (as PhotoCraft's defringe
/// does) and keeping the source near the pixel leaves shadows, glows and isolated faint strokes alone.
pub fn defringe(pixels: &mut PixelBuf, width: u32, sel: Option<&AlphaBuf>) -> u64 {
    let (w, h) = (pixels.width, pixels.height);
    let n = (w as usize) * (h as usize);
    if n == 0 || pixels.data.len() != n * 4 || width == 0 {
        return 0;
    }
    let width = width.min(MAX_DEFRINGE) as f32;
    let alpha: Vec<u8> = pixels.data.iter().skip(3).step_by(4).copied().collect();
    let outside: Vec<u8> = alpha.iter().map(|&a| if a == 0 { 255 } else { 0 }).collect();
    if !outside.contains(&255) {
        return 0;
    }
    let d = distance_to_ink(&outside, w, h);
    drop(outside);
    let interior: Vec<u8> = alpha.iter().zip(&d).map(|(&a, &d0)| if a > 0 && d0 > width { 255 } else { 0 }).collect();
    if !interior.contains(&255) {
        return 0;
    }
    let near = nearest_ink(&interior, w, h);
    drop(interior);
    let reach2 = (width + 2.0) * (width + 2.0);
    let src = pixels.clone();
    let before = pixels.clone();
    let wu = w as usize;
    per_pixel(pixels, sel, |i, p| {
        if alpha[i] == 0 || d[i] > width {
            return p;
        }
        let s = near[i] as usize;
        if s >= n {
            return p;
        }
        let (dx, dy) = ((s % wu) as f32 - (i % wu) as f32, (s / wu) as f32 - (i / wu) as f32);
        if dx * dx + dy * dy > reach2 {
            return p;
        }
        let c = &src.data[s * 4..s * 4 + 3];
        [c[0], c[1], c[2], p[3]]
    });
    pixels.data.chunks(4).zip(before.data.chunks(4)).filter(|(a, b)| a[..3] != b[..3]).count() as u64
}

// ---------------------------------------------------------------------------------------------------------------
// Remove background (plain backgrounds)
// ---------------------------------------------------------------------------------------------------------------

/// Options of Remove Background (Tool tab).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RemoveBgOptions {
    /// A pixel is background when it differs from a background colour by at most this (4.4's difference).
    pub tolerance: u8,
    /// Also accept a pixel that differs from an accepted neighbour by at most `tolerance / 4` (gradients).
    pub follow_gradients: bool,
    /// Afterwards add every pixel within `tolerance / 2` of a background colour (holes of letters).
    pub enclosed: bool,
}

impl Default for RemoveBgOptions {
    fn default() -> RemoveBgOptions {
        RemoveBgOptions { tolerance: DEFAULT_BG_TOLERANCE, follow_gradients: false, enclosed: false }
    }
}

/// Why Remove Background did nothing. `message()` is the text the UI shows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RemoveBgRefusal {
    /// An empty picture.
    Empty,
    /// At least half of the 1-px border is transparent already.
    AlreadyTransparent,
    /// No colour holds 15 % of the border.
    NoPlainBackground,
}

impl RemoveBgRefusal {
    /// The note shown to the user.
    pub fn message(self) -> &'static str {
        match self {
            RemoveBgRefusal::Empty => "This layer has no pixels",
            RemoveBgRefusal::AlreadyTransparent => "This picture already has a transparent background",
            RemoveBgRefusal::NoPlainBackground => {
                "No plain background found along the edges. Try the Magic Eraser or Colour to Alpha"
            }
        }
    }
}

/// The result of Remove Background before it is applied: what to keep of each pixel and the unmixed colours of the
/// soft edge. Apply it to the pixels ([`BackgroundCut::apply`]) or to the layer mask ("Into mask",
/// [`BackgroundCut::apply_to_mask`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundCut {
    /// 0 = background (cleared), 255 = kept, between = the soft edge.
    pub keep: AlphaBuf,
    /// The background colours found (1..=3), most frequent first.
    pub colors: Vec<[u8; 3]>,
    /// Pixels in the background region (keep = 0).
    pub region_pixels: u64,
    /// Soft-edge pixels: index and unmixed RGB.
    pub recolor: Vec<(u32, [u8; 3])>,
}

impl BackgroundCut {
    /// Applies the cut to the pixels: alpha times `keep`, the soft edge recoloured; mixed by `sel` when given.
    pub fn apply(&self, pixels: &mut PixelBuf, sel: Option<&AlphaBuf>) {
        let n = pixels.data.len() / 4;
        if self.keep.data.len() != n {
            return;
        }
        let mut target = pixels.clone();
        target.data.par_chunks_mut(4).with_min_len(1 << 14).zip(&self.keep.data).for_each(|(p, &k)| {
            p[3] = mul_u8(p[3], k);
        });
        for &(i, rgb) in &self.recolor {
            let i = i as usize;
            if i < n {
                target.data[i * 4..i * 4 + 3].copy_from_slice(&rgb);
            }
        }
        per_pixel(pixels, sel, |i, _| {
            let t = &target.data[i * 4..i * 4 + 4];
            [t[0], t[1], t[2], t[3]]
        });
    }

    /// "Into mask (keep pixels)": the layer mask (the picture's size) times `keep`, mixed by `sel` when given; the
    /// pixels are not touched.
    pub fn apply_to_mask(&self, mask: &mut AlphaBuf, sel: Option<&AlphaBuf>) {
        if mask.data.len() != self.keep.data.len() || mask.width != self.keep.width {
            return;
        }
        let len = mask.data.len();
        mask.data.par_iter_mut().with_min_len(1 << 14).enumerate().for_each(|(i, m)| {
            *m = mix_u8(*m, mul_u8(*m, self.keep.data[i]), sel_at(sel, len, i));
        });
    }
}

#[inline]
fn px(src: &PixelBuf, i: usize) -> [u8; 4] {
    let d = &src.data[i * 4..i * 4 + 4];
    [d[0], d[1], d[2], d[3]]
}

/// Everything not reached (255) becomes 0.
fn par_zero_unreached(region: &mut [u8]) {
    region.par_iter_mut().with_min_len(1 << 14).for_each(|r| {
        if *r != 255 {
            *r = 0;
        }
    });
}

/// Where a pixel lies for the soft edge of Remove Background.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Zone {
    Region,
    Band,
    Inside,
}

/// How far (pixels) the soft edge looks for the nearest region / inside pixel.
const SEARCH_RADIUS: i32 = 4;

/// The offsets of a disc of radius `r`, nearest first (ties: by row, then column): a fixed search order.
fn window_offsets(r: i32) -> Vec<(i32, i32)> {
    let mut v: Vec<(i32, i32)> = (-r..=r).flat_map(|dy| (-r..=r).map(move |dx| (dx, dy))).filter(|&(dx, dy)| dx * dx + dy * dy <= r * r).collect();
    v.sort_by_key(|&(dx, dy)| (dx * dx + dy * dy, dy, dx));
    v
}

#[inline]
fn opaque(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

/// Indices of the 1-px frame of a `w x h` picture, each once (top row, bottom row, then the side columns).
fn frame(w: usize, h: usize) -> Vec<usize> {
    let mut f: Vec<usize> = (0..w).collect();
    if h > 1 {
        f.extend((0..w).map(|x| (h - 1) * w + x));
    }
    for y in 1..h.saturating_sub(1) {
        f.push(y * w);
        if w > 1 {
            f.push(y * w + w - 1);
        }
    }
    f
}

/// The background colours of 4.5 step 2: colours quantised to 4 bits per channel; the most frequent cell (ties: the
/// lower cell index) seeds a cluster of every not yet taken opaque border pixel within `tolerance` of the cell's
/// mean; the cluster's mean is a background colour when it holds at least 15 % of the border. Up to three colours.
fn background_colors(src: &PixelBuf, border: &[usize], tolerance: u8) -> Vec<[u8; 3]> {
    let total = border.len();
    let mut free: Vec<[u8; 4]> = border.iter().map(|&i| px(src, i)).filter(|p| p[3] >= 128).collect();
    let mut out = Vec::new();
    while out.len() < BG_MAX_COLORS && !free.is_empty() {
        let mut counts = vec![0u32; 4096];
        let cell = |p: &[u8; 4]| ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
        for p in &free {
            counts[cell(p)] += 1;
        }
        let best = (0..4096).max_by(|&a, &b| counts[a].cmp(&counts[b]).then(b.cmp(&a))).unwrap_or(0);
        let seed = mean(free.iter().filter(|p| cell(p) == best));
        let (taken, rest): (Vec<[u8; 4]>, Vec<[u8; 4]>) =
            free.into_iter().partition(|p| color_diff(*p, opaque(seed)) <= tolerance);
        if taken.len() * 100 < BG_MIN_SHARE * total {
            break;
        }
        out.push(mean(taken.iter()));
        free = rest;
    }
    out
}

/// Mean RGB (rounded) of some pixels; black for none.
fn mean<'a>(pixels: impl Iterator<Item = &'a [u8; 4]>) -> [u8; 3] {
    let (mut s, mut n) = ([0u64; 3], 0u64);
    for p in pixels {
        for c in 0..3 {
            s[c] += p[c] as u64;
        }
        n += 1;
    }
    if n == 0 {
        return [0; 3];
    }
    std::array::from_fn(|c| ((s[c] + n / 2) / n) as u8)
}

/// Remove Background (4.5) for plain backgrounds: finds the background colours along the border, floods the
/// background from the border, softens a 2-px band outside it with colour to alpha, and returns the cut (nothing is
/// changed yet). Deterministic; linear in the picture.
pub fn remove_background(src: &PixelBuf, opts: RemoveBgOptions) -> Result<BackgroundCut, RemoveBgRefusal> {
    let (w, h) = (src.width as usize, src.height as usize);
    let n = w * h;
    if n == 0 || src.data.len() != n * 4 {
        return Err(RemoveBgRefusal::Empty);
    }
    let border = frame(w, h);
    let transparent = border.iter().filter(|&&i| src.data[i * 4 + 3] < 128).count();
    if transparent * 2 >= border.len() {
        return Err(RemoveBgRefusal::AlreadyTransparent);
    }
    let tol = opts.tolerance;
    let colors = background_colors(src, &border, tol);
    if colors.is_empty() {
        return Err(RemoveBgRefusal::NoPlainBackground);
    }
    let is_bg = |p: [u8; 4], t: u8| colors.iter().any(|&c| color_diff(p, opaque(c)) <= t);

    // 3. The region: a flood (4-connected) from every matching border pixel. The colour test runs once per pixel,
    // in parallel, into a plane. A pixel is accepted when it matches a background colour, or (Follow gradients)
    // differs from an accepted neighbour by at most tol / 4. Only accepted pixels are marked, so a pixel refused from
    // one side can still be reached from another: the result is the smallest closed set, whatever the order.
    let mut region: Vec<u8> = (0..n).into_par_iter().with_min_len(1 << 14).map(|i| is_bg(px(src, i), tol) as u8).collect();
    if !opts.follow_gradients {
        flood_plane(&mut region, w, &border);
    } else {
        let step = tol / 4;
        let mut stack: Vec<usize> = Vec::new();
        for &i in &border {
            if region[i] == 1 {
                region[i] = 255;
                stack.push(i);
            }
        }
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let here = px(src, i);
            let mut visit = |j: usize| {
                let r = region[j];
                if r == 255 {
                    return;
                }
                let q = px(src, j);
                if r == 1 || (q[3] >= 128 && color_diff(q, here) <= step) {
                    region[j] = 255;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        par_zero_unreached(&mut region);
    }
    if opts.enclosed {
        let half = tol / 2;
        region.par_iter_mut().with_min_len(1 << 14).enumerate().for_each(|(i, r)| {
            if *r == 0 && is_bg(px(src, i), half) {
                *r = 255;
            }
        });
    }
    let region_pixels = region.iter().filter(|&&r| r != 0).count() as u64;

    // 4. Soft edge: pixels within 2 px outside the region lose the background tint. Key = the background colour
    // nearest to the closest region pixel (with Follow gradients: that pixel's own colour). The pixel's colour-to-
    // alpha opacity is divided by the opacity of the closest "inside" pixel (outside the region and the band) against
    // the same key, so the object's own opaque pixels along the edge stay opaque and keep their colour, and an
    // anti-aliased pixel `k O + (1 - k) C` gets back `O` at alpha `k`. Without an inside pixel nearby, plain colour
    // to alpha.
    let d = distance_to_ink(&region, w as u32, h as u32);
    let class = |i: usize| -> Zone {
        if region[i] != 0 {
            Zone::Region
        } else if d[i] <= BG_SOFT_BAND {
            Zone::Band
        } else {
            Zone::Inside
        }
    };
    let offsets = window_offsets(SEARCH_RADIUS);
    let nearest = |i: usize, zone: Zone| -> Option<usize> {
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        offsets.iter().find_map(|&(dx, dy)| {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                return None;
            }
            let j = ny as usize * w + nx as usize;
            (class(j) == zone).then_some(j)
        })
    };
    let mut keep: Vec<u8> = region.iter().map(|&r| 255 - r).collect();
    let band: Vec<(usize, [u8; 3], u8)> = (0..n)
        .into_par_iter()
        .with_min_len(1 << 14)
        .filter_map(|i| {
            if class(i) != Zone::Band {
                return None;
            }
            let p = px(src, i);
            if p[3] == 0 {
                return None;
            }
            let rc = nearest(i, Zone::Region).map(|s| px(src, s)).unwrap_or(p);
            let key = if opts.follow_gradients {
                [rc[0], rc[1], rc[2]]
            } else {
                *colors.iter().min_by_key(|&&c| color_diff(rc, opaque(c))).unwrap_or(&colors[0])
            };
            let opacity = |q: [u8; 4]| unmix_pixel([q[0], q[1], q[2], 255], key, 0.0, 1.0).1 as f32 / 255.0;
            let a_obj = nearest(i, Zone::Inside).map(|s| opacity(px(src, s))).filter(|&a| a >= 0.05).unwrap_or(1.0);
            let k = (opacity(p) / a_obj).min(1.0);
            if k >= 1.0 {
                return None;
            }
            let a = byte(k);
            let rgb: [u8; 3] = std::array::from_fn(|c| {
                let (pc, kc) = (p[c] as f32 / 255.0, key[c] as f32 / 255.0);
                if k > 0.0 { byte((pc - kc) / k + kc) } else { key[c] }
            });
            (a < 255).then_some((i, rgb, a))
        })
        .collect();
    let mut recolor = Vec::with_capacity(band.len());
    for (i, rgb, a) in band {
        keep[i] = a;
        if a > 0 {
            recolor.push((i as u32, rgb));
        }
    }
    let keep = AlphaBuf::from_raw(src.width, src.height, keep).unwrap_or_else(|| AlphaBuf::new(src.width, src.height));
    Ok(BackgroundCut { keep, colors, region_pixels, recolor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sticker::select::Selection;
    use crate::testutil::XorShift;
    use glam::Vec2;

    /// Object coverage of a generated picture: a disc plus a bar (a shape with a concave part).
    fn object(w: u32, h: u32) -> AlphaBuf {
        let (wf, hf) = (w as f32, h as f32);
        let mut m = Selection::ellipse(w, h, Vec2::new(wf * 0.2, hf * 0.2), Vec2::new(wf * 0.65, hf * 0.65)).mask;
        let bar = Selection::rect(w, h, Vec2::new(wf * 0.5, hf * 0.55), Vec2::new(wf * 0.85, hf * 0.8)).mask;
        for (a, b) in m.data.iter_mut().zip(&bar.data) {
            *a = (*a).max(*b);
        }
        m
    }

    /// `object` over a background, anti-aliased (the object's colour mixed with the background by its coverage).
    fn composite(obj: &AlphaBuf, color: [u8; 3], mut bg: impl FnMut(u32, u32) -> [u8; 3]) -> PixelBuf {
        let mut b = PixelBuf::new(obj.width, obj.height);
        for y in 0..obj.height {
            for x in 0..obj.width {
                let k = obj.get(x, y);
                let g = bg(x, y);
                b.set(x, y, [mix_u8(g[0], color[0], k), mix_u8(g[1], color[1], k), mix_u8(g[2], color[2], k), 255]);
            }
        }
        b
    }

    fn iou(result: &PixelBuf, obj: &AlphaBuf) -> f64 {
        let (mut inter, mut uni) = (0u64, 0u64);
        for (p, &o) in result.data.chunks(4).zip(&obj.data) {
            let (a, b) = (p[3] >= 128, o >= 128);
            inter += (a && b) as u64;
            uni += (a || b) as u64;
        }
        inter as f64 / uni as f64
    }

    #[test]
    fn k2_color_to_alpha_rules() {
        let white = [255, 255, 255];
        // Pure key -> alpha 0.
        assert_eq!(unmix_pixel([255, 255, 255, 255], white, 0.0, 1.0).0[3], 0);
        let mut img = PixelBuf::filled(2, 1, [255, 255, 255, 255]);
        // A 50 % mix of red over white -> red at alpha 128 +- 1.
        img.set(1, 0, [255, 128, 128, 255]);
        color_to_alpha(&mut img, white, 0.0, 1.0, None);
        assert_eq!(img.get(0, 0)[3], 0);
        let p = img.get(1, 0);
        assert!(p[3].abs_diff(128) <= 1 && p[0] == 255 && p[1] <= 1 && p[2] <= 1, "{p:?}");
        // Opaque black stays opaque (and unchanged).
        let mut img = PixelBuf::filled(1, 1, [0, 0, 0, 255]);
        color_to_alpha(&mut img, white, 0.0, 1.0, None);
        assert_eq!(img.get(0, 0), [0, 0, 0, 255]);
        // Composited over the key again, the result reproduces the pixel (within rounding).
        let mut rng = XorShift::new(5);
        for _ in 0..2000 {
            let c = rng.next_u32().to_le_bytes();
            let key = [c[3], c[1] / 2, 200];
            let p = [c[0], c[1], c[2], 255];
            let (q, _) = unmix_pixel(p, key, 0.0, 1.0);
            let a = q[3] as f32 / 255.0;
            for i in 0..3 {
                let back = q[i] as f32 * a + key[i] as f32 * (1.0 - a);
                assert!((back - p[i] as f32).abs() <= 1.5, "{p:?} key {key:?} -> {q:?}");
            }
        }
        // Thresholds: everything at or below t0 vanishes, at or above t1 stays.
        assert_eq!(unmix_pixel([255, 200, 200, 255], white, 0.3, 1.0).0[3], 0);
        assert_eq!(unmix_pixel([255, 100, 100, 255], white, 0.0, 0.5).0, [255, 100, 100, 255]);
        // A transparent pixel is left alone; a selection limits the change.
        assert_eq!(unmix_pixel([1, 2, 3, 0], white, 0.0, 1.0).0, [1, 2, 3, 0]);
        let mut img = PixelBuf::filled(2, 1, [255, 255, 255, 255]);
        color_to_alpha(&mut img, white, 0.0, 1.0, Some(&AlphaBuf::from_raw(2, 1, vec![255, 0]).unwrap()));
        assert_eq!(img.data, vec![255, 255, 255, 0, 255, 255, 255, 255]);
    }

    #[test]
    fn k2_remove_matte_inverts_a_matte() {
        // A colour Q at alpha a matted on white / black is stored as P with alpha a; removing the matte gives Q back
        // (within the rounding of the stored P: 0.5 / a levels), and matting the result again gives P within 1 level.
        let mut rng = XorShift::new(9);
        for matte in [Matte::White, Matte::Black] {
            for _ in 0..3000 {
                let c = rng.next_u32().to_le_bytes();
                let a = 32 + (c[3] as u32 * 223 / 255) as u8;
                let q = [c[0], c[1], c[2]];
                let af = a as f32 / 255.0;
                let m = if matte == Matte::White { 255.0 } else { 0.0 };
                let p: [u8; 3] = std::array::from_fn(|i| (q[i] as f32 * af + m * (1.0 - af) + 0.5) as u8);
                let mut img = PixelBuf::filled(1, 1, [p[0], p[1], p[2], a]);
                remove_matte(&mut img, matte, None);
                let r = img.get(0, 0);
                assert_eq!(r[3], a);
                let allowed = (0.5 / af).ceil() + 1.0;
                for i in 0..3 {
                    assert!((r[i] as f32 - q[i] as f32).abs() <= allowed, "{matte:?} q {q:?} a {a} -> {r:?}");
                    let again = r[i] as f32 * af + m * (1.0 - af);
                    assert!((again - p[i] as f32).abs() <= 1.0, "{matte:?} re-matte {again} vs {}", p[i]);
                }
            }
        }
        // Alpha 0 and 255 are left.
        let mut img = PixelBuf::from_raw(2, 1, vec![9, 9, 9, 0, 9, 9, 9, 255]).unwrap();
        remove_matte(&mut img, Matte::White, None);
        assert_eq!(img.data, vec![9, 9, 9, 0, 9, 9, 9, 255]);
    }

    #[test]
    fn k2_defringe_replaces_a_white_halo() {
        // A red disc of radius 20 with a white halo 3 px wide (alpha falling off) on transparency.
        let (w, h) = (64u32, 64u32);
        let mut img = PixelBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 + 0.5 - 32.0).powi(2) + (y as f32 + 0.5 - 32.0).powi(2)).sqrt();
                if d < 20.0 {
                    img.set(x, y, [230, 10, 10, 255]);
                } else if d < 23.0 {
                    let a = (255.0 * (23.0 - d) / 3.0) as u8;
                    img.set(x, y, [250, 245, 240, a.max(1)]);
                }
            }
        }
        let alpha_before: Vec<u8> = img.data.iter().skip(3).step_by(4).copied().collect();
        let changed = defringe(&mut img, 4, None);
        assert!(changed > 100, "{changed}");
        for (i, p) in img.data.chunks(4).enumerate() {
            assert_eq!(p[3], alpha_before[i], "alpha changed at {i}");
            if p[3] > 0 {
                assert!(p[1] <= 40, "pixel {i} keeps g = {} (alpha {})", p[1], p[3]);
            }
        }
        // An opaque picture has no fringe; no interior means no change; width 0 is nothing.
        let mut opaque = PixelBuf::filled(10, 10, [255, 255, 255, 255]);
        assert_eq!(defringe(&mut opaque, 3, None), 0);
        let mut thin = PixelBuf::filled(10, 10, [255, 255, 255, 100]);
        assert_eq!(defringe(&mut thin, 3, None), 0);
        assert_eq!(defringe(&mut img.clone(), 0, None), 0);
    }

    #[test]
    fn k2_remove_background_on_three_synthetic_pictures() {
        let (w, h) = (240u32, 200u32);
        let obj = object(w, h);
        // 1. Flat white.
        let img = composite(&obj, [180, 30, 40], |_, _| [255, 255, 255]);
        let cut = remove_background(&img, RemoveBgOptions::default()).unwrap();
        assert_eq!(cut.colors, vec![[255, 255, 255]]);
        let mut out = img.clone();
        cut.apply(&mut out, None);
        let s = iou(&out, &obj);
        eprintln!("remove background: flat white IoU {s:.4}, {} soft-edge pixels", cut.recolor.len());
        assert!(s >= 0.98, "flat white IoU {s}");
        assert_eq!(out.get(0, 0)[3], 0);
        assert_eq!(out.get(w / 2 - 20, h / 2 - 20)[3], 255);
        // The soft edge lost its white tint: no visible pixel is much lighter than the object in green / blue.
        for p in out.data.chunks(4).filter(|p| p[3] >= 16) {
            assert!(p[1] <= 60 && p[2] <= 70, "halo pixel {p:?}");
        }
        // The object's own pixels along the edge stay opaque, and the anti-aliased ones get the object's colour
        // back (plain colour to alpha would make the 2-px rim about 12 % transparent and shift its colour).
        for (p, &o) in out.data.chunks(4).zip(&obj.data) {
            if o == 255 {
                assert!(p[3] >= 250, "object pixel {p:?}");
            }
            if p[3] >= 64 {
                let d = p[0].abs_diff(180).max(p[1].abs_diff(30)).max(p[2].abs_diff(40));
                assert!(d <= 12, "edge pixel {p:?} (coverage {o})");
            }
        }
        // 2. Two-colour border: sky blue above, green below.
        let img = composite(&obj, [200, 40, 40], |_, y| if y < h / 2 { [120, 170, 230] } else { [60, 160, 70] });
        let cut = remove_background(&img, RemoveBgOptions::default()).unwrap();
        assert_eq!(cut.colors.len(), 2, "{:?}", cut.colors);
        let mut out = img.clone();
        cut.apply(&mut out, None);
        let s = iou(&out, &obj);
        eprintln!("remove background: two-colour IoU {s:.4}, colours {:?}", cut.colors);
        assert!(s >= 0.98, "two-colour IoU {s}");
        // 3. A vertical gradient with Follow gradients.
        let grad = |_: u32, y: u32| {
            let t = y as f32 / (h - 1) as f32;
            [(255.0 - 225.0 * t) as u8, (250.0 - 190.0 * t) as u8, (240.0 - 120.0 * t) as u8]
        };
        let img = composite(&obj, [20, 90, 20], grad);
        let opts = RemoveBgOptions { follow_gradients: true, ..RemoveBgOptions::default() };
        let cut = remove_background(&img, opts).unwrap();
        let mut out = img.clone();
        cut.apply(&mut out, None);
        let s = iou(&out, &obj);
        eprintln!("remove background: gradient IoU {s:.4}, colours {:?}", cut.colors);
        assert!(s >= 0.98, "gradient IoU {s}");
        // Without Follow gradients the middle of the gradient stays.
        let plain = remove_background(&img, RemoveBgOptions::default()).unwrap();
        assert!(plain.region_pixels < cut.region_pixels);

        // Into mask: the pixels are kept, the mask gets the cut.
        let img = composite(&obj, [180, 30, 40], |_, _| [255, 255, 255]);
        let cut = remove_background(&img, RemoveBgOptions::default()).unwrap();
        let mut mask = AlphaBuf::filled(w, h, 255);
        cut.apply_to_mask(&mut mask, None);
        assert_eq!(mask, cut.keep);
        // A selection limits it.
        let sel = Selection::rect(w, h, Vec2::ZERO, Vec2::new(10.0, 10.0)).mask;
        let mut out = img.clone();
        cut.apply(&mut out, Some(&sel));
        assert_eq!(out.get(5, 5)[3], 0);
        assert_eq!(out.get(5, 50)[3], 255);
    }

    #[test]
    fn k2_remove_background_refusals_and_enclosed_areas() {
        // Already transparent.
        let mut img = PixelBuf::new(30, 20);
        img.set(15, 10, [1, 2, 3, 255]);
        assert_eq!(remove_background(&img, RemoveBgOptions::default()), Err(RemoveBgRefusal::AlreadyTransparent));
        assert_eq!(RemoveBgRefusal::AlreadyTransparent.message(), "This picture already has a transparent background");
        assert_eq!(remove_background(&PixelBuf::new(0, 0), RemoveBgOptions::default()), Err(RemoveBgRefusal::Empty));
        // Noise along the border: no colour holds 15 %.
        let mut rng = XorShift::new(1);
        let mut img = PixelBuf::new(40, 40);
        for p in img.data.chunks_mut(4) {
            let c = rng.next_u32().to_le_bytes();
            p.copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
        assert_eq!(remove_background(&img, RemoveBgOptions::default()), Err(RemoveBgRefusal::NoPlainBackground));
        // A ring (an "O") on white: the hole is only removed with "Also enclosed areas".
        let (w, h) = (80u32, 80u32);
        let outer = Selection::ellipse(w, h, Vec2::new(10.0, 10.0), Vec2::new(70.0, 70.0)).mask;
        let inner = Selection::ellipse(w, h, Vec2::new(25.0, 25.0), Vec2::new(55.0, 55.0)).mask;
        let ring = AlphaBuf::from_raw(w, h, outer.data.iter().zip(&inner.data).map(|(&o, &i)| mul_u8(o, 255 - i)).collect()).unwrap();
        let img = composite(&ring, [10, 10, 120], |_, _| [255, 255, 255]);
        let cut = remove_background(&img, RemoveBgOptions::default()).unwrap();
        assert_eq!(cut.keep.get(40, 40), 255);
        let cut = remove_background(&img, RemoveBgOptions { enclosed: true, ..RemoveBgOptions::default() }).unwrap();
        assert_eq!(cut.keep.get(40, 40), 0);
        let mut out = img.clone();
        cut.apply(&mut out, None);
        assert!(iou(&out, &ring) >= 0.97);
    }

    #[test]
    fn k2_remove_background_on_a_noisy_grey_background() {
        // Grey 128 +- 12 straddles the 4-bit cell boundary at 128 in every channel, so no single cell holds 15 % of
        // the border (the plan's literal rule would refuse); the cell only seeds a cluster within the tolerance.
        let (w, h) = (200u32, 160u32);
        let obj = object(w, h);
        let mut rng = XorShift::new(31);
        let img = composite(&obj, [30, 40, 160], |_, _| {
            let n = |r: &mut XorShift| (116 + r.next_u32() % 25) as u8;
            [n(&mut rng), n(&mut rng), n(&mut rng)]
        });
        let border = frame(w as usize, h as usize);
        let mut cells = std::collections::HashMap::new();
        for &i in &border {
            let p = px(&img, i);
            *cells.entry((p[0] >> 4, p[1] >> 4, p[2] >> 4)).or_insert(0usize) += 1;
        }
        assert!(cells.values().all(|&c| c * 100 < 15 * border.len()), "a cell holds 15 %: the test proves nothing");
        let cut = remove_background(&img, RemoveBgOptions::default()).unwrap();
        assert_eq!(cut.colors.len(), 1);
        assert!(cut.colors[0].iter().all(|&c| c.abs_diff(128) <= 3), "{:?}", cut.colors);
        let mut out = img.clone();
        cut.apply(&mut out, None);
        let s = iou(&out, &obj);
        assert!(s >= 0.98, "noisy grey IoU {s}");
    }

    #[test]
    fn k2c_defringe_leaves_shadows_glows_and_faint_strokes_alone() {
        // Check P2: a red square, a soft black shadow apart from it, a soft glow touching it, a faint isolated stroke.
        let (w, h) = (140u32, 70u32);
        let mut img = PixelBuf::new(w, h);
        for y in 10..50 {
            for x in 10..40 {
                img.set(x, y, [220, 10, 10, 255]);
            }
            // Glow right of the square, touching it (alpha 90, cyan).
            for x in 40..52 {
                img.set(x, y, [0, 200, 200, 90]);
            }
            // Shadow apart (alpha 100, black).
            for x in 70..110 {
                img.set(x, y, [0, 0, 0, 100]);
            }
        }
        // Faint stroke, 2 px wide, alone.
        for y in 10..60 {
            for x in 125..127 {
                img.set(x, y, [0, 0, 255, 60]);
            }
        }
        let before = img.clone();
        defringe(&mut img, 2, None);
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (img.get(x, y), before.get(x, y));
                if x >= 52 {
                    assert_eq!(a, b, "({x}, {y}): shadow / stroke recoloured");
                }
                if (44..52).contains(&x) && (14..46).contains(&y) {
                    // Glow pixels away from the square (more than width + 2 from any red pixel) stay cyan.
                    assert_eq!(a, b, "({x}, {y}): glow recoloured");
                }
                assert_eq!(a[3], b[3]);
            }
        }
        // The square's own rim next to transparency still takes the inside colour (it already has it: no change),
        // and a white rim on it is replaced.
        let mut img = PixelBuf::new(40, 40);
        for y in 5..35 {
            for x in 5..35 {
                let rim = !(7..33).contains(&x) || !(7..33).contains(&y);
                img.set(x, y, if rim { [255, 255, 255, 160] } else { [220, 10, 10, 255] });
            }
        }
        assert!(defringe(&mut img, 2, None) > 0);
        assert!(img.data.chunks(4).filter(|p| p[3] > 0).all(|p| p[..3] == [220, 10, 10]));
    }

    #[test]
    fn k2c_remove_background_on_a_noisy_white_background() {
        // Check P2 (plan 7 "P2", checker): an object on white with per-channel noise 236..=255, for a light, a dark
        // and a grey object; the background goes, the object stays whole.
        let (w, h) = (256u32, 200u32);
        let obj = object(w, h);
        let mut rng = XorShift::new(99);
        for (color, tol) in [([250u8, 225, 90], 40u8), ([40, 60, 160], 40), ([200, 200, 200], 30)] {
            let img = composite(&obj, color, |_, _| {
                let n = |r: &mut XorShift| 255 - (r.next_u32() % 20) as u8;
                [n(&mut rng), n(&mut rng), n(&mut rng)]
            });
            let cut = remove_background(&img, RemoveBgOptions { tolerance: tol, ..RemoveBgOptions::default() }).unwrap();
            assert_eq!(cut.colors.len(), 1, "{:?}", cut.colors);
            let mut out = img.clone();
            cut.apply(&mut out, None);
            let s = iou(&out, &obj);
            assert!(s >= 0.98, "{color:?}: IoU {s}");
            for (p, &o) in out.data.chunks(4).zip(&obj.data) {
                if o == 0 {
                    assert_eq!(p[3], 0, "{color:?}: background speck {p:?}");
                }
                if o == 255 {
                    assert!(p[3] >= 250, "{color:?}: object pixel {p:?}");
                }
            }
        }
    }

    #[test]
    fn k2_cutout_is_thread_count_independent() {
        let (w, h) = (300u32, 280u32);
        let obj = object(w, h);
        let mut rng = XorShift::new(21);
        let img = composite(&obj, [150, 20, 200], |_, _| {
            let n = (rng.next_u32() % 9) as u8;
            [246 + n, 246 + n, 246 + n]
        });
        let run = || {
            let cut = remove_background(&img, RemoveBgOptions { follow_gradients: true, enclosed: true, tolerance: 30 }).unwrap();
            let mut a = img.clone();
            cut.apply(&mut a, None);
            let mut b = img.clone();
            color_to_alpha(&mut b, [250, 250, 250], 0.1, 0.9, None);
            let mut c = a.clone();
            defringe(&mut c, 2, None);
            let mut d = b.clone();
            remove_matte(&mut d, Matte::White, Some(&obj));
            (cut, a, b, c, d)
        };
        let pools = [1usize, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
        let x = pools[0].install(run);
        let y = pools[1].install(run);
        assert!(x == y, "results differ between 1 and 8 threads");
    }
}
