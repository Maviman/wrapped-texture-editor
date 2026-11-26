// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! Selections and colour-tolerance regions (STICKER_PLAN 3.4, 4.4; ARCHITECTURE 5.27, 6.12).
//!
//! A [`Selection`] is a one-byte coverage mask of the artboard's size (255 = fully selected) plus the bounding box of
//! its non-zero pixels. Regions (magic wand, colour range, the Fill and Magic Eraser tools) are computed on a straight
//! RGBA8 picture with the colour difference of 4.4; the caller decides which picture (the active layer's content in
//! artboard space, or the sticker's composite for "Sample all layers").
//!
//! Every buffer handed to one call must have the same size; a selection or coverage of another size counts as
//! **empty** (nothing is selected, so nothing changes). Everything is deterministic: the flood fill's result is a
//! set (the order of the stack does not matter), and the parallel passes work on independent rows.

use glam::Vec2;
use rayon::prelude::*;

use crate::math::{IRect, mul_u8};
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::pixel_ops::{premultiply, unpremultiply};
use crate::sticker::fx::{cov, dist_in, dist_out, over, tent, tent_width};
use crate::sticker::shape::{polygon_coverage, sd_ellipse, sd_round_box};

/// Buffers with at least this many pixels are processed row-parallel.
const PAR_PIXELS: usize = 1 << 16;
/// The wand's default tolerance (Tool tab, STICKER_PLAN 3.3).
pub const DEFAULT_TOLERANCE: u8 = 32;
/// Longest grow / shrink / feather length (pixels).
pub const MAX_MODIFY: f32 = 512.0;

/// How a new selection combines with the existing one (Shift adds, Alt subtracts).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SelectMode {
    #[default]
    Replace,
    /// `max(old, new)`.
    Add,
    /// `old * (255 - new) / 255`.
    Subtract,
    /// `min(old, new)`.
    Intersect,
}

/// A selection over the artboard (STICKER_PLAN 3.4): `mask` has the artboard's size; `bounds` is the bounding box of
/// its non-zero pixels (`IRect::EMPTY` when nothing is selected). Build it with [`Selection::from_mask`] (or the
/// constructors below) so `bounds` stays right; every method keeps it right.
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub mask: AlphaBuf,
    pub bounds: IRect,
}

impl Selection {
    /// The selection of a coverage mask; `bounds` computed.
    pub fn from_mask(mask: AlphaBuf) -> Selection {
        let bounds = mask_bounds(&mask);
        Selection { mask, bounds }
    }

    /// Nothing selected.
    pub fn none(width: u32, height: u32) -> Selection {
        Selection { mask: AlphaBuf::new(width, height), bounds: IRect::EMPTY }
    }

    /// Everything selected (Select All).
    pub fn all(width: u32, height: u32) -> Selection {
        Selection::from_mask(AlphaBuf::filled(width, height, 255))
    }

    /// Rectangle marquee from two corners (continuous pixel coordinates, any order): coverage `clamp(0.5 - sd)` of
    /// 4.8, so whole-pixel corners give a hard edge.
    pub fn rect(width: u32, height: u32, a: Vec2, b: Vec2) -> Selection {
        let (lo, hi) = (a.min(b), a.max(b));
        let (c, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
        Selection::from_mask(coverage_of(width, height, |p| sd_round_box(p, c, half, 0.0)))
    }

    /// Ellipse marquee inscribed in the box of two corners (4.8's ellipse distance).
    pub fn ellipse(width: u32, height: u32, a: Vec2, b: Vec2) -> Selection {
        let (lo, hi) = (a.min(b), a.max(b));
        let (c, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
        if half.x <= 0.0 || half.y <= 0.0 {
            return Selection::none(width, height);
        }
        Selection::from_mask(coverage_of(width, height, |p| sd_ellipse(p, c, half)))
    }

    /// Lasso: the polygon through `points` (non-zero rule, edge distance; 4.8).
    pub fn polygon(width: u32, height: u32, points: &[Vec2]) -> Selection {
        Selection::from_mask(polygon_coverage(points, width, height))
    }

    /// "Selection from layer alpha": the alpha channel of `picture` (already in artboard space).
    pub fn from_alpha(picture: &PixelBuf) -> Selection {
        Selection::from_mask(AlphaBuf::from_alpha(picture))
    }

    /// Width and height of the mask.
    pub fn size(&self) -> [u32; 2] {
        [self.mask.width, self.mask.height]
    }

    /// True when no pixel is selected.
    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    /// Coverage at `(x, y)`; 0 outside the mask.
    pub fn at(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x as u32 >= self.mask.width || y as u32 >= self.mask.height {
            return 0;
        }
        self.mask.data[y as usize * self.mask.width as usize + x as usize]
    }

    /// Combines `new` (same size; another size counts as empty) into this selection.
    pub fn combine(&mut self, new: &AlphaBuf, mode: SelectMode) {
        let same = same_size(&self.mask, new);
        match mode {
            SelectMode::Replace => {
                self.mask = if same { new.clone() } else { AlphaBuf::new(self.mask.width, self.mask.height) };
            }
            _ if !same => {
                if mode == SelectMode::Intersect {
                    self.mask.data.fill(0);
                }
            }
            SelectMode::Add => zip_in_place(&mut self.mask.data, &new.data, |o, n| o.max(n)),
            SelectMode::Subtract => zip_in_place(&mut self.mask.data, &new.data, |o, n| mul_u8(o, 255 - n)),
            SelectMode::Intersect => zip_in_place(&mut self.mask.data, &new.data, |o, n| o.min(n)),
        }
        self.bounds = mask_bounds(&self.mask);
    }

    /// Invert Selection: `255 - m`.
    pub fn invert(&mut self) {
        par_map(&mut self.mask.data, |v| 255 - v);
        self.bounds = mask_bounds(&self.mask);
    }

    /// Grow by `n` pixels: `max(m, clamp(n + 0.5 - dist_out(m)))` (4.4; the same threshold as the die-cut contour).
    pub fn grow(&mut self, n: f32) {
        let n = clean_len(n);
        if n <= 0.0 || self.is_empty() {
            return;
        }
        let (w, h) = (self.mask.width, self.mask.height);
        let d = dist_out(&self.mask.data, w, h);
        zip_index(&mut self.mask.data, |i, v| v.max(cov(n + 0.5 - d[i])));
        self.bounds = mask_bounds(&self.mask);
    }

    /// Shrink by `n` pixels: `min(m, 255 - clamp(n + 0.5 - dist_in(m)))`. The artboard's border is not an edge: a
    /// selection that touches it does not shrink away from it (Select All stays all).
    pub fn shrink(&mut self, n: f32) {
        let n = clean_len(n);
        if n <= 0.0 || self.is_empty() {
            return;
        }
        let (w, h) = (self.mask.width, self.mask.height);
        let d = dist_in(&self.mask.data, w, h);
        zip_index(&mut self.mask.data, |i, v| v.min(255 - cov(n + 0.5 - d[i])));
        self.bounds = mask_bounds(&self.mask);
    }

    /// Feather by `n` pixels: a tent blur of box width `round(n)` (4.2). The mask is extended by its own border
    /// values first, so a selection that touches the artboard's border is not faded there (Select All stays all).
    pub fn feather(&mut self, n: f32) {
        let b = tent_width(clean_len(n));
        if b <= 1 || self.is_empty() {
            return;
        }
        feather_mask(&mut self.mask, b);
        self.bounds = mask_bounds(&self.mask);
    }

    /// Multiplies a coverage (same size) by this selection: what a tool may change is limited by it (3.4).
    pub fn limit(&self, coverage: &mut AlphaBuf) {
        if !same_size(&self.mask, coverage) {
            coverage.data.fill(0);
            return;
        }
        zip_in_place(&mut coverage.data, &self.mask.data, mul_u8);
    }
}

/// `n` as a modify length: non-finite -> 0, clamped to `0..=MAX_MODIFY`.
fn clean_len(n: f32) -> f32 {
    if n.is_finite() { n.clamp(0.0, MAX_MODIFY) } else { 0.0 }
}

fn same_size(a: &AlphaBuf, b: &AlphaBuf) -> bool {
    a.width == b.width && a.height == b.height && a.data.len() == b.data.len()
}

/// Bounding box of the non-zero pixels of a mask; `IRect::EMPTY` when there is none.
pub fn mask_bounds(mask: &AlphaBuf) -> IRect {
    let w = mask.width as usize;
    if w == 0 || mask.height == 0 || mask.data.len() < w * mask.height as usize {
        return IRect::EMPTY;
    }
    let rows: Vec<Option<(usize, usize)>> = mask
        .data
        .par_chunks(w)
        .with_min_len(64)
        .map(|row| {
            let first = row.iter().position(|&v| v != 0)?;
            let last = row.iter().rposition(|&v| v != 0)?;
            Some((first, last))
        })
        .collect();
    let mut r: Option<IRect> = None;
    for (y, span) in rows.iter().enumerate() {
        if let Some((a, b)) = span {
            let (y, a, b) = (y as i32, *a as i32, *b as i32);
            r = Some(match r {
                None => IRect::new(a, y, b + 1, y + 1),
                Some(o) => IRect::new(o.x0.min(a), o.y0, o.x1.max(b + 1), y + 1),
            });
        }
    }
    r.unwrap_or(IRect::EMPTY)
}

/// Coverage `clamp(0.5 - sd(p))` at every pixel centre.
fn coverage_of(width: u32, height: u32, sd: impl Fn(Vec2) -> f32 + Sync) -> AlphaBuf {
    let mut out = AlphaBuf::new(width, height);
    let w = width as usize;
    if w == 0 {
        return out;
    }
    let row = |(y, row): (usize, &mut [u8])| {
        for (x, o) in row.iter_mut().enumerate() {
            *o = cov(0.5 - sd(Vec2::new(x as f32 + 0.5, y as f32 + 0.5)));
        }
    };
    if out.data.len() >= PAR_PIXELS {
        out.data.par_chunks_mut(w).enumerate().for_each(row);
    } else {
        out.data.chunks_mut(w).enumerate().for_each(row);
    }
    out
}

fn par_map(v: &mut [u8], f: impl Fn(u8) -> u8 + Sync) {
    v.par_iter_mut().with_min_len(1 << 14).for_each(|x| *x = f(*x));
}

fn zip_in_place(v: &mut [u8], o: &[u8], f: impl Fn(u8, u8) -> u8 + Sync) {
    v.par_iter_mut().with_min_len(1 << 14).zip(o).for_each(|(x, &y)| *x = f(*x, y));
}

fn zip_index(v: &mut [u8], f: impl Fn(usize, u8) -> u8 + Sync) {
    v.par_iter_mut().with_min_len(1 << 14).enumerate().for_each(|(i, x)| *x = f(i, *x));
}

/// Tent blur of a mask with its border values extended by the blur's reach on every side.
fn feather_mask(mask: &mut AlphaBuf, b: u32) {
    let (w, h) = (mask.width as usize, mask.height as usize);
    let e = b as usize;
    let (ew, eh) = (w + 2 * e, h + 2 * e);
    let mut big = vec![0u8; ew * eh];
    for (yy, row) in big.chunks_mut(ew).enumerate() {
        let y = yy.saturating_sub(e).min(h - 1);
        let src = &mask.data[y * w..(y + 1) * w];
        row[..e].fill(src[0]);
        row[e..e + w].copy_from_slice(src);
        row[e + w..].fill(src[w - 1]);
    }
    tent(&mut big, ew as u32, eh as u32, b);
    for (y, row) in mask.data.chunks_mut(w).enumerate() {
        row.copy_from_slice(&big[(y + e) * ew + e..(y + e) * ew + e + w]);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Colour-tolerance regions (4.4)
// ---------------------------------------------------------------------------------------------------------------

/// Options of the wand, the Fill tool and the Magic Eraser.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RegionOptions {
    /// 0..=255: a pixel matches when its difference to the seed colour is at most this.
    pub tolerance: u8,
    /// Only the 4-connected region around the seed (else every matching pixel).
    pub contiguous: bool,
    /// Soft edge: the pixels touching the region get a partial value.
    pub anti_alias: bool,
}

impl Default for RegionOptions {
    fn default() -> RegionOptions {
        RegionOptions { tolerance: DEFAULT_TOLERANCE, contiguous: true, anti_alias: true }
    }
}

/// Colour difference of 4.4: `max(|dr|, |dg|, |db|, |da|)` on straight RGBA8, where the colour of a fully
/// transparent pixel is ignored (two transparent pixels differ by 0; a transparent and a visible one by `|da|`).
#[inline]
pub fn color_diff(p: [u8; 4], q: [u8; 4]) -> u8 {
    if p[3] == 0 || q[3] == 0 {
        return p[3].abs_diff(q[3]);
    }
    p[0].abs_diff(q[0]).max(p[1].abs_diff(q[1])).max(p[2].abs_diff(q[2])).max(p[3].abs_diff(q[3]))
}

/// The partial value of a pixel next to a region (4.4): `round(255 * clamp(1 - (diff - tol) / max(tol, 16)))`.
#[inline]
pub fn fringe_value(diff: u8, tolerance: u8) -> u8 {
    if diff <= tolerance {
        return 255;
    }
    let k = 1.0 - (diff - tolerance) as f32 / (tolerance.max(16)) as f32;
    cov(k)
}

#[inline]
fn px(src: &PixelBuf, i: usize) -> [u8; 4] {
    let d = &src.data[i * 4..i * 4 + 4];
    [d[0], d[1], d[2], d[3]]
}

/// Magic wand / Fill / Magic Eraser region from the pixel `seed` (4.4): 255 inside, the anti-aliased fringe when
/// asked, 0 elsewhere. A seed outside the picture gives an empty region.
pub fn region(src: &PixelBuf, seed: (u32, u32), opts: RegionOptions) -> AlphaBuf {
    let (w, h) = (src.width as usize, src.height as usize);
    if seed.0 as usize >= w || seed.1 as usize >= h || src.data.len() != w * h * 4 {
        return AlphaBuf::new(src.width, src.height);
    }
    let target = px(src, seed.1 as usize * w + seed.0 as usize);
    let marks = if opts.contiguous {
        flood(src, (seed.0 as usize, seed.1 as usize), |p| color_diff(p, target) <= opts.tolerance)
    } else {
        matching(src, |p| color_diff(p, target) <= opts.tolerance)
    };
    finish_region(src, marks, opts, |p| color_diff(p, target))
}

/// Color Range (3.3): every pixel within `tolerance` of `color` (the wand with Contiguous off, from a picked colour
/// instead of a pixel).
pub fn color_range(src: &PixelBuf, color: [u8; 4], tolerance: u8, anti_alias: bool) -> AlphaBuf {
    let (w, h) = (src.width as usize, src.height as usize);
    if src.data.len() != w * h * 4 {
        return AlphaBuf::new(src.width, src.height);
    }
    let marks = matching(src, |p| color_diff(p, color) <= tolerance);
    let opts = RegionOptions { tolerance, contiguous: false, anti_alias };
    finish_region(src, marks, opts, |p| color_diff(p, color))
}

/// The marks (255 / 0) with the anti-aliased fringe added when asked.
fn finish_region(src: &PixelBuf, marks: Vec<u8>, opts: RegionOptions, diff: impl Fn([u8; 4]) -> u8 + Sync) -> AlphaBuf {
    let (w, h) = (src.width as usize, src.height as usize);
    if !opts.anti_alias || w == 0 || h == 0 {
        return AlphaBuf { width: src.width, height: src.height, data: marks };
    }
    let mut out = vec![0u8; w * h];
    let marks = &marks;
    let row = |(y, row): (usize, &mut [u8])| {
        for (x, o) in row.iter_mut().enumerate() {
            let i = y * w + x;
            if marks[i] != 0 {
                *o = 255;
                continue;
            }
            let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let (y0, y1) = (y.saturating_sub(1), (y + 1).min(h - 1));
            let touches = (y0..=y1).any(|yy| marks[yy * w + x0..=yy * w + x1].iter().any(|&m| m != 0));
            if touches {
                *o = fringe_value(diff(px(src, i)), opts.tolerance);
            }
        }
    };
    if w * h >= PAR_PIXELS {
        out.par_chunks_mut(w).enumerate().for_each(row);
    } else {
        out.chunks_mut(w).enumerate().for_each(row);
    }
    AlphaBuf { width: src.width, height: src.height, data: out }
}

/// Every pixel for which `hit` holds (255), in parallel rows.
fn matching(src: &PixelBuf, hit: impl Fn([u8; 4]) -> bool + Sync) -> Vec<u8> {
    let w = src.width as usize;
    let mut marks = vec![0u8; src.data.len() / 4];
    if w == 0 {
        return marks;
    }
    let row = |(y, row): (usize, &mut [u8])| {
        for (x, m) in row.iter_mut().enumerate() {
            if hit(px(src, y * w + x)) {
                *m = 255;
            }
        }
    };
    if marks.len() >= PAR_PIXELS {
        marks.par_chunks_mut(w).enumerate().for_each(row);
    } else {
        marks.chunks_mut(w).enumerate().for_each(row);
    }
    marks
}

/// Scanline flood fill (4-connected) from `seed` through pixels for which `hit` holds: 255 = reached. The test is
/// evaluated once per pixel, in parallel rows, into a byte plane (0 = no match, 1 = match not yet reached, 255 =
/// reached); the flood itself then compares bytes only. Work and memory are linear in the picture (each pixel is
/// reached once; the stack holds at most one entry per run).
fn flood(src: &PixelBuf, seed: (usize, usize), hit: impl Fn([u8; 4]) -> bool + Sync) -> Vec<u8> {
    let w = src.width as usize;
    let mut marks = matching(src, hit);
    for m in marks.iter_mut() {
        *m >>= 7; // 255 -> 1 (match, not reached)
    }
    flood_plane(&mut marks, w, &[seed.1 * w + seed.0]);
    marks
}

/// Scanline flood fill (4-connected) on a plane of `w`-wide rows where 1 = may be reached and 0 = may not, from
/// every seed index whose value is 1. Afterwards 255 = reached and 0 = everything else. Each pixel is reached once;
/// the stack holds at most one entry per run.
pub(crate) fn flood_plane(marks: &mut [u8], w: usize, seeds: &[usize]) {
    if w == 0 {
        return;
    }
    let h = marks.len() / w;
    let mut stack: Vec<(usize, usize)> = seeds.iter().filter(|&&i| i < w * h).map(|&i| (i % w, i / w)).collect();
    while let Some((x, y)) = stack.pop() {
        let row = y * w;
        if marks[row + x] != 1 {
            continue;
        }
        let (mut l, mut r) = (x, x);
        while l > 0 && marks[row + l - 1] == 1 {
            l -= 1;
        }
        while r + 1 < w && marks[row + r + 1] == 1 {
            r += 1;
        }
        marks[row + l..=row + r].fill(255);
        for ny in [y.wrapping_sub(1), y + 1] {
            if ny >= h {
                continue;
            }
            let line = &marks[ny * w + l..=ny * w + r];
            let mut in_run = false;
            for (k, &m) in line.iter().enumerate() {
                let ok = m == 1;
                if ok && !in_run {
                    stack.push((l + k, ny));
                }
                in_run = ok;
            }
        }
    }
    par_map(marks, |m| if m == 255 { 255 } else { 0 });
}

// ---------------------------------------------------------------------------------------------------------------
// Applying a coverage (Magic Eraser, Fill, Delete, the selection limit of every destructive command)
// ---------------------------------------------------------------------------------------------------------------

/// Coverage value `i` of an optional selection of the picture's size; `None` = everything (255), another size =
/// nothing (0).
#[inline]
pub(crate) fn sel_at(sel: Option<&AlphaBuf>, len: usize, i: usize) -> u8 {
    match sel {
        None => 255,
        Some(s) if s.data.len() == len => s.data[i],
        Some(_) => 0,
    }
}

/// Magic Eraser / Delete (4.4): `alpha' = alpha * (255 - c) / 255` (rounded), RGB kept. `coverage` must have the
/// picture's size (else nothing changes).
pub fn erase(pixels: &mut PixelBuf, coverage: &AlphaBuf) {
    if coverage.data.len() * 4 != pixels.data.len() || coverage.width != pixels.width {
        return;
    }
    pixels.data.par_chunks_mut(4).with_min_len(1 << 14).zip(&coverage.data).for_each(|(p, &c)| {
        p[3] = mul_u8(p[3], 255 - c);
    });
}

/// Magic Eraser with "Into mask (keep pixels)": the same on a layer mask.
pub fn erase_mask(mask: &mut AlphaBuf, coverage: &AlphaBuf) {
    if !same_size(mask, coverage) {
        return;
    }
    zip_in_place(&mut mask.data, &coverage.data, |m, c| mul_u8(m, 255 - c));
}

/// Fill (4.4): source-over of `color` with strength `coverage / 255` (straight alpha). `coverage` must have the
/// picture's size (else nothing changes).
pub fn fill(pixels: &mut PixelBuf, coverage: &AlphaBuf, color: [u8; 4]) {
    if coverage.data.len() * 4 != pixels.data.len() || coverage.width != pixels.width {
        return;
    }
    let rgb = [color[0] as f32, color[1] as f32, color[2] as f32];
    let k = color[3] as f32 / (255.0 * 255.0);
    pixels.data.par_chunks_mut(4).with_min_len(1 << 14).zip(&coverage.data).for_each(|(p, &c)| {
        if c != 0 {
            over(p, rgb, c as f32 * k);
        }
    });
}

/// Fill on a layer mask: `m + (v - m) * c / 255` toward the value `v` (white shows, black hides).
pub fn fill_mask(mask: &mut AlphaBuf, coverage: &AlphaBuf, value: u8) {
    if !same_size(mask, coverage) {
        return;
    }
    zip_in_place(&mut mask.data, &coverage.data, |m, c| mix_u8(m, value, c));
}

/// `a + (b - a) * k / 255`, rounded half up; `k = 0` gives `a`, `k = 255` gives `b`.
#[inline]
pub(crate) fn mix_u8(a: u8, b: u8, k: u8) -> u8 {
    let (a, b, k) = (a as i32, b as i32, k as i32);
    let v = a * 255 + (b - a) * k;
    ((v + 127) / 255).clamp(0, 255) as u8
}

/// "Layer via Copy": the picture with its alpha multiplied by the selection (same size; another size gives a
/// transparent picture).
pub fn extract(src: &PixelBuf, sel: &AlphaBuf) -> PixelBuf {
    let mut out = src.clone();
    let len = src.data.len() / 4;
    out.data.par_chunks_mut(4).with_min_len(1 << 14).enumerate().for_each(|(i, p)| {
        p[3] = mul_u8(p[3], sel_at(Some(sel), len, i));
    });
    out
}

/// One pixel of `before` mixed toward `after` by `k / 255`, premultiplied (so a transparent side lends no colour);
/// `k = 0` is `before` and `k = 255` is `after`, byte for byte.
#[inline]
pub fn mix_px(before: [u8; 4], after: [u8; 4], k: u8) -> [u8; 4] {
    match k {
        0 => before,
        255 => after,
        _ => {
            let (b, a) = (premultiply(before), premultiply(after));
            let m: [u8; 4] = std::array::from_fn(|c| mix_u8(b[c], a[c], k));
            if m[3] == 0 {
                return [before[0], before[1], before[2], 0];
            }
            unpremultiply(m)
        }
    }
}

/// Limits a finished edit to a selection: every pixel of `after` becomes `mix_px(before, after, sel)`. All three
/// have the same size (a selection of another size keeps `before` everywhere).
pub fn mix_selected(before: &PixelBuf, after: &mut PixelBuf, sel: &AlphaBuf) {
    if before.data.len() != after.data.len() {
        return;
    }
    let len = before.data.len() / 4;
    after.data.par_chunks_mut(4).with_min_len(1 << 14).enumerate().for_each(|(i, p)| {
        let k = sel_at(Some(sel), len, i);
        if k != 255 {
            let b = &before.data[i * 4..i * 4 + 4];
            let m = mix_px([b[0], b[1], b[2], b[3]], [p[0], p[1], p[2], p[3]], k);
            p.copy_from_slice(&m);
        }
    });
}

/// The same for a mask: `before + (after - before) * sel / 255`.
pub fn mix_selected_mask(before: &AlphaBuf, after: &mut AlphaBuf, sel: &AlphaBuf) {
    if !same_size(before, after) {
        return;
    }
    let len = before.data.len();
    after.data.par_iter_mut().with_min_len(1 << 14).enumerate().for_each(|(i, a)| {
        *a = mix_u8(before.data[i], *a, sel_at(Some(sel), len, i));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::XorShift;

    fn two_colour(w: u32, h: u32, split: u32) -> PixelBuf {
        let mut b = PixelBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                b.set(x, y, if x < split { [200, 30, 30, 255] } else { [30, 30, 200, 255] });
            }
        }
        b
    }

    fn disc_mask(w: u32, h: u32, c: Vec2, r: f32) -> AlphaBuf {
        Selection::ellipse(w, h, c - Vec2::splat(r), c + Vec2::splat(r)).mask
    }

    fn sum(m: &AlphaBuf) -> f64 {
        m.data.iter().map(|&v| v as f64).sum()
    }

    /// Radius of the 128 edge along the row through the centre (sub-pixel, by linear interpolation).
    fn edge_radius(m: &AlphaBuf, cx: u32, cy: u32) -> f32 {
        let row = |x: u32| m.get(x, cy) as f32;
        let mut x = cx;
        while x + 1 < m.width && row(x + 1) >= 128.0 {
            x += 1;
        }
        let (a, b) = (row(x), row(x + 1));
        let t = (a - 127.5) / (a - b).max(1e-3);
        x as f32 + 0.5 + t - (cx as f32 + 0.5)
    }

    #[test]
    fn k2_wand_selects_one_side_of_a_two_colour_image_at_tolerance_0() {
        let img = two_colour(40, 30, 17);
        let opts = RegionOptions { tolerance: 0, contiguous: true, anti_alias: false };
        let r = region(&img, (3, 4), opts);
        for y in 0..30 {
            for x in 0..40 {
                assert_eq!(r.get(x, y), if x < 17 { 255 } else { 0 }, "({x}, {y})");
            }
        }
        let s = Selection::from_mask(r);
        assert_eq!(s.bounds, IRect::new(0, 0, 17, 30));
        // With anti-aliasing, the column next to the region gets the fringe value of its difference (170 > 0).
        let aa = region(&img, (3, 4), RegionOptions { anti_alias: true, ..opts });
        assert_eq!(aa.get(17, 5), fringe_value(170, 0));
        assert_eq!(aa.get(17, 5), 0);
        assert_eq!(aa.get(18, 5), 0);
        // A seed outside gives nothing.
        assert!(region(&img, (40, 0), opts).data.iter().all(|&v| v == 0));
    }

    #[test]
    fn k2_tolerance_edges_and_contiguity() {
        // Row of greys 100, 110, ..., the seed at 100: a difference equal to the tolerance is in.
        let mut img = PixelBuf::new(8, 1);
        for x in 0..8 {
            let v = 100 + 10 * x as u8;
            img.set(x, 0, [v, v, v, 255]);
        }
        let r = region(&img, (0, 0), RegionOptions { tolerance: 30, contiguous: true, anti_alias: false });
        assert_eq!(r.data, vec![255, 255, 255, 255, 0, 0, 0, 0]);
        let r = region(&img, (0, 0), RegionOptions { tolerance: 29, contiguous: true, anti_alias: false });
        assert_eq!(r.data, vec![255, 255, 255, 0, 0, 0, 0, 0]);
        // Contiguous vs not: the same colour on both sides of a wall.
        let mut img = PixelBuf::filled(9, 3, [10, 10, 10, 255]);
        for y in 0..3 {
            img.set(4, y, [250, 250, 250, 255]);
        }
        let o = RegionOptions { tolerance: 5, contiguous: true, anti_alias: false };
        let c = region(&img, (0, 1), o);
        assert_eq!(c.data.iter().filter(|&&v| v == 255).count(), 12);
        let n = region(&img, (0, 1), RegionOptions { contiguous: false, ..o });
        assert_eq!(n.data.iter().filter(|&&v| v == 255).count(), 24);
        assert_eq!(n, color_range(&img, [10, 10, 10, 255], 5, false));
        // Diagonal neighbours are not 4-connected.
        let mut img = PixelBuf::filled(3, 3, [0, 0, 0, 255]);
        img.set(0, 0, [255, 255, 255, 255]);
        img.set(1, 1, [255, 255, 255, 255]);
        let r = region(&img, (0, 0), RegionOptions { tolerance: 0, contiguous: true, anti_alias: false });
        assert_eq!(r.data.iter().filter(|&&v| v == 255).count(), 1);
    }

    #[test]
    fn k2_anti_alias_values_on_a_known_ramp() {
        // A horizontal ramp 0, 8, 16, ...: seed at 0, tolerance 16. Pixels 0..=2 are in (diff 0, 8, 16); pixel 3
        // (diff 24) touches the region: 255 * (1 - 8 / 16) = 127.5 -> 128; pixel 4 (diff 32) does not touch it.
        let mut img = PixelBuf::new(10, 3);
        for y in 0..3 {
            for x in 0..10 {
                let v = 8 * x as u8;
                img.set(x, y, [v, v, v, 255]);
            }
        }
        let r = region(&img, (0, 1), RegionOptions { tolerance: 16, contiguous: true, anti_alias: true });
        for y in 0..3 {
            assert_eq!(&r.data[y * 10..y * 10 + 6], &[255, 255, 255, 128, 0, 0]);
        }
        // Tolerance 40 (above 16): the divisor is the tolerance. Pixel 6 (diff 48): 255 * (1 - 8 / 40) = 204.
        let r = region(&img, (0, 1), RegionOptions { tolerance: 40, contiguous: true, anti_alias: true });
        assert_eq!(&r.data[10..18], &[255, 255, 255, 255, 255, 255, 204, 0]);
        // Tolerance 0: everything that differs gets 0 (the divisor is 16, a difference of 8 gives 128).
        assert_eq!(fringe_value(8, 0), 128);
        assert_eq!(fringe_value(16, 0), 0);
        assert_eq!(fringe_value(5, 5), 255);
    }

    #[test]
    fn k2_transparent_pixels_match_each_other() {
        assert_eq!(color_diff([255, 0, 0, 0], [0, 255, 7, 0]), 0);
        assert_eq!(color_diff([255, 0, 0, 0], [0, 0, 0, 90]), 90);
        assert_eq!(color_diff([10, 20, 30, 255], [13, 18, 30, 250]), 5);
        // Transparent pixels of any colour form one region.
        let mut img = PixelBuf::new(6, 4);
        let mut rng = XorShift::new(3);
        for y in 0..4 {
            for x in 0..6 {
                let c = rng.next_u32().to_le_bytes();
                img.set(x, y, if x == 5 { [9, 9, 9, 255] } else { [c[0], c[1], c[2], 0] });
            }
        }
        let r = region(&img, (0, 0), RegionOptions { tolerance: 0, contiguous: true, anti_alias: false });
        for y in 0..4 {
            for x in 0..6 {
                assert_eq!(r.get(x, y), if x < 5 { 255 } else { 0 });
            }
        }
    }

    #[test]
    fn k2_magic_eraser_on_white_background_and_fill_honours_a_selection() {
        // A red disc on white with an anti-aliased edge: the magic eraser from a corner clears the background and
        // leaves the object's opaque pixels at 255.
        let (w, h) = (64u32, 64u32);
        let disc = disc_mask(w, h, Vec2::new(32.0, 32.0), 20.0);
        let mut img = PixelBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let a = disc.get(x, y);
                let mix = |o: u8, b: u8| mix_u8(b, o, a);
                img.set(x, y, [mix(220, 255), mix(20, 255), mix(20, 255), 255]);
            }
        }
        let opts = RegionOptions { tolerance: 32, contiguous: true, anti_alias: true };
        let r = region(&img, (0, 0), opts);
        erase(&mut img, &r);
        for y in 0..h {
            for x in 0..w {
                let (a, d) = (img.get(x, y)[3], disc.get(x, y));
                if d == 255 {
                    assert_eq!(a, 255, "object pixel ({x}, {y})");
                }
                if d == 0 {
                    assert_eq!(a, 0, "background pixel ({x}, {y})");
                }
            }
        }
        // The same into a mask: the pixels keep their alpha.
        let mut mask = AlphaBuf::filled(w, h, 255);
        erase_mask(&mut mask, &r);
        assert_eq!(mask.get(0, 0), 0);
        assert_eq!(mask.get(32, 32), 255);

        // Fill honours a selection: fill the whole white picture (one region) inside a rectangle only.
        let mut img = PixelBuf::filled(20, 10, [255, 255, 255, 255]);
        let mut cov = region(&img, (0, 0), RegionOptions::default());
        let sel = Selection::rect(20, 10, Vec2::new(5.0, 2.0), Vec2::new(12.0, 7.0));
        sel.limit(&mut cov);
        fill(&mut img, &cov, [0, 128, 0, 255]);
        for y in 0..10 {
            for x in 0..20 {
                let inside = (5..12).contains(&x) && (2..7).contains(&y);
                assert_eq!(img.get(x, y), if inside { [0, 128, 0, 255] } else { [255, 255, 255, 255] }, "({x}, {y})");
            }
        }
        // Half coverage of an opaque colour over white gives the half mix.
        let mut img = PixelBuf::filled(1, 1, [255, 255, 255, 255]);
        fill(&mut img, &AlphaBuf::filled(1, 1, 128), [0, 0, 0, 255]);
        assert_eq!(img.get(0, 0), [127, 127, 127, 255]);
        // Filling a transparent pixel gives the colour at the coverage.
        let mut img = PixelBuf::new(1, 1);
        fill(&mut img, &AlphaBuf::filled(1, 1, 128), [10, 20, 30, 255]);
        assert_eq!(img.get(0, 0), [10, 20, 30, 128]);
        // A coverage of the wrong size changes nothing.
        let before = img.clone();
        fill(&mut img, &AlphaBuf::filled(2, 1, 255), [1, 2, 3, 255]);
        assert_eq!(img, before);
        let mut m = AlphaBuf::filled(2, 1, 100);
        fill_mask(&mut m, &AlphaBuf::from_raw(2, 1, vec![255, 0]).unwrap(), 0);
        assert_eq!(m.data, vec![0, 100]);
    }

    #[test]
    fn k2_marquee_shapes_and_combine_modes() {
        let r = Selection::rect(16, 16, Vec2::new(10.0, 12.0), Vec2::new(2.0, 3.0));
        assert_eq!(r.bounds, IRect::new(2, 3, 10, 12));
        assert_eq!(r.mask.data.iter().filter(|&&v| v == 255).count(), 8 * 9);
        assert_eq!(r.mask.data.iter().filter(|&&v| v != 0 && v != 255).count(), 0);
        // Half-pixel edges give half coverage.
        let h = Selection::rect(8, 8, Vec2::new(1.5, 0.0), Vec2::new(4.0, 8.0));
        assert_eq!(h.mask.get(1, 3), 128);
        assert_eq!(h.mask.get(2, 3), 255);
        // An ellipse's area within 1 %.
        let e = Selection::ellipse(200, 120, Vec2::new(20.0, 10.0), Vec2::new(180.0, 110.0));
        let area = sum(&e.mask) / 255.0;
        let exact = std::f64::consts::PI * 80.0 * 50.0;
        assert!((area - exact).abs() / exact < 0.01, "{area} vs {exact}");
        assert!(Selection::ellipse(10, 10, Vec2::new(3.0, 3.0), Vec2::new(3.0, 8.0)).is_empty());
        // Lasso triangle.
        let t = Selection::polygon(20, 20, &[Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0), Vec2::new(0.0, 20.0)]);
        assert!((sum(&t.mask) / 255.0 - 200.0).abs() < 2.0);
        // Combine.
        let a = Selection::rect(10, 1, Vec2::new(0.0, 0.0), Vec2::new(6.0, 1.0));
        let b = Selection::rect(10, 1, Vec2::new(4.0, 0.0), Vec2::new(10.0, 1.0));
        let mut s = a.clone();
        s.combine(&b.mask, SelectMode::Add);
        assert_eq!(s.mask.data, vec![255; 10]);
        let mut s = a.clone();
        s.combine(&b.mask, SelectMode::Subtract);
        assert_eq!(s.mask.data, vec![255, 255, 255, 255, 0, 0, 0, 0, 0, 0]);
        assert_eq!(s.bounds, IRect::new(0, 0, 4, 1));
        let mut s = a.clone();
        s.combine(&b.mask, SelectMode::Intersect);
        assert_eq!(s.mask.data, vec![0, 0, 0, 0, 255, 255, 0, 0, 0, 0]);
        let mut s = a.clone();
        s.combine(&b.mask, SelectMode::Replace);
        assert_eq!(s, b);
        // A mask of another size counts as empty.
        let mut s = a.clone();
        s.combine(&AlphaBuf::filled(3, 3, 255), SelectMode::Add);
        assert_eq!(s, a);
        s.combine(&AlphaBuf::filled(3, 3, 255), SelectMode::Intersect);
        assert!(s.is_empty());
        // Invert, all, none, at.
        let mut s = a.clone();
        s.invert();
        assert_eq!(s.bounds, IRect::new(6, 0, 10, 1));
        assert_eq!(s.at(7, 0), 255);
        assert_eq!(s.at(-1, 0), 0);
        assert_eq!(Selection::all(3, 2).bounds, IRect::new(0, 0, 3, 2));
        assert!(Selection::none(3, 2).is_empty());
        let px = PixelBuf::from_raw(2, 1, vec![0, 0, 0, 0, 1, 1, 1, 9]).unwrap();
        assert_eq!(Selection::from_alpha(&px).bounds, IRect::new(1, 0, 2, 1));
    }

    #[test]
    fn k2_grow_then_shrink_of_a_disc_returns_it_and_feather_keeps_the_sum() {
        let c = Vec2::new(64.0, 64.0);
        let disc = Selection::from_mask(disc_mask(128, 128, c, 30.0));
        let r0 = edge_radius(&disc.mask, 64, 64);
        let mut g = disc.clone();
        g.grow(8.0);
        let rg = edge_radius(&g.mask, 64, 64);
        assert!((rg - (r0 + 8.0)).abs() <= 0.6, "grown edge {rg} vs {}", r0 + 8.0);
        let mut gs = g.clone();
        gs.shrink(8.0);
        let diff = gs.mask.data.iter().zip(&disc.mask.data).filter(|(a, b)| a.abs_diff(**b) > 128).count();
        assert_eq!(diff, 0, "grow then shrink moved the edge by more than a pixel");
        let rs = edge_radius(&gs.mask, 64, 64);
        assert!((rs - r0).abs() <= 1.0, "{rs} vs {r0}");
        // Shrink by more than the radius empties it; growth stops at the artboard.
        let mut s = disc.clone();
        s.shrink(40.0);
        assert!(s.is_empty());
        let mut s = disc.clone();
        s.grow(500.0);
        assert_eq!(s.bounds, IRect::new(0, 0, 128, 128));
        // Select All stays all under shrink and feather.
        let mut all = Selection::all(40, 30);
        all.shrink(5.0);
        assert_eq!(all, Selection::all(40, 30));
        all.feather(9.0);
        assert_eq!(all, Selection::all(40, 30));
        // Feather keeps the sum within 1 % and makes the edge soft.
        let mut f = disc.clone();
        f.feather(10.0);
        let (s0, s1) = (sum(&disc.mask), sum(&f.mask));
        assert!((s1 - s0).abs() / s0 < 0.01, "{s0} -> {s1}");
        assert!(f.mask.data.iter().filter(|&&v| v > 0 && v < 255).count() > 4 * disc.mask.data.iter().filter(|&&v| v > 0 && v < 255).count());
        assert!(f.bounds.width() > disc.bounds.width());
        // Non-finite and zero lengths change nothing.
        let mut z = disc.clone();
        z.grow(f32::NAN);
        z.shrink(0.0);
        z.feather(f32::INFINITY);
        assert_eq!(z, disc);
    }

    #[test]
    fn k2_mixing_by_a_selection() {
        assert_eq!(mix_u8(0, 255, 128), 128);
        assert_eq!(mix_u8(10, 200, 0), 10);
        assert_eq!(mix_u8(10, 200, 255), 200);
        assert_eq!(mix_px([1, 2, 3, 4], [5, 6, 7, 8], 0), [1, 2, 3, 4]);
        assert_eq!(mix_px([1, 2, 3, 4], [5, 6, 7, 8], 255), [5, 6, 7, 8]);
        // Premultiplied: half way between opaque red and transparent green keeps red at half alpha.
        assert_eq!(mix_px([255, 0, 0, 255], [0, 255, 0, 0], 128), [255, 0, 0, 127]);
        let before = PixelBuf::filled(3, 1, [10, 10, 10, 255]);
        let mut after = PixelBuf::filled(3, 1, [250, 250, 250, 255]);
        mix_selected(&before, &mut after, &AlphaBuf::from_raw(3, 1, vec![0, 128, 255]).unwrap());
        assert_eq!(after.data, vec![10, 10, 10, 255, 130, 130, 130, 255, 250, 250, 250, 255]);
        let mut m = AlphaBuf::filled(2, 1, 200);
        mix_selected_mask(&AlphaBuf::filled(2, 1, 0), &mut m, &AlphaBuf::from_raw(2, 1, vec![0, 255]).unwrap());
        assert_eq!(m.data, vec![0, 200]);
        let src = PixelBuf::filled(2, 1, [9, 9, 9, 200]);
        let e = extract(&src, &AlphaBuf::from_raw(2, 1, vec![255, 0]).unwrap());
        assert_eq!(e.data, vec![9, 9, 9, 200, 9, 9, 9, 0]);
    }

    #[test]
    fn k2_regions_and_modifiers_are_thread_count_independent() {
        let mut rng = XorShift::new(11);
        let (w, h) = (300u32, 260u32);
        let mut img = PixelBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let n = (rng.next_u32() % 24) as u8;
                let v = if ((x / 37) + (y / 29)) % 2 == 0 { 40 + n } else { 200 + n };
                img.set(x, y, [v, v / 2, 255 - v, if x % 50 == 0 { 0 } else { 255 }]);
            }
        }
        let run = || {
            let a = region(&img, (5, 5), RegionOptions { tolerance: 30, contiguous: true, anti_alias: true });
            let b = region(&img, (5, 5), RegionOptions { tolerance: 30, contiguous: false, anti_alias: true });
            let mut s = Selection::from_mask(b.clone());
            s.grow(3.5);
            s.shrink(2.0);
            s.feather(6.0);
            let mut p = img.clone();
            fill(&mut p, &a, [1, 2, 3, 200]);
            erase(&mut p, &s.mask);
            (a, b, s, p)
        };
        let pools = [1usize, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
        let x = pools[0].install(run);
        let y = pools[1].install(run);
        assert!(x == y, "results differ between 1 and 8 threads");
    }
}
