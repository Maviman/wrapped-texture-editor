// Portions adapted from PhotoCraft (MIT licence, see licenses/photocraft-MIT.txt).
//! The GrabCut box cut-out (STICKER_PLAN 4.5 "GrabCut box", 7 "S1"; ARCHITECTURE 5.27, 6.12): the user drags a box
//! around the object of a photo; what lies outside the box is background, the box probably holds the object, and the
//! picture is cut along the edge between the two. Classic image maths ("no AI", plan 0.4 rule 6): colour models and
//! a minimum graph cut, no learned model, no network.
//!
//! Implemented from C. Rother, V. Kolmogorov and A. Blake, "GrabCut: Interactive Foreground Extraction using Iterated
//! Graph Cuts", SIGGRAPH 2004, and Y. Boykov and V. Kolmogorov, "An Experimental Comparison of Min-Cut/Max-Flow
//! Algorithms for Energy Minimization in Vision", IEEE PAMI 26(9), 2004:
//!
//! 1. A working copy of the box plus a margin (15 % of the box's longer side, at least 8 pixels; clipped to the
//!    picture), box-filtered by a whole step so that its longer side is at most [`WORK_SIDE`] pixels.
//! 2. Trimap: working pixels whose block centre lies outside the box, and blocks that are mostly transparent, are
//!    fixed background; the others are "probably object". When nothing opaque lies outside the box (a box over the
//!    whole picture), a ring of the working copy (1/50 of its shorter side, at least 1) is background instead.
//! 3. [`ITERATIONS`] times: a [`COMPONENTS`]-component full-covariance Gaussian mixture for each side (the first
//!    time from a deterministic k-means++ start, then from each sample's most likely component), the data costs
//!    `-ln p(colour)`, and one minimum cut of the 8-neighbour grid with the contrast-sensitive smoothness
//!    `GAMMA * exp(-beta |z_p - z_q|^2) / dist(p, q)` (`beta = 1 / (2 <|z_p - z_q|^2>)`), Boykov-Kolmogorov max-flow.
//!    Stops early when no label changed.
//! 4. With a step above 1: the result scaled up and cut once more at full size in a band of [`BAND`] pixels around
//!    its edge (the pixels outside the band keep their label and act as fixed neighbours).
//! 5. The soft edge of Remove Background (4.5 step 4): pixels within 2 pixels of the background lose its tint by
//!    colour to alpha against the colour of the closest background pixel (`cutout::soft_edge`).
//!
//! The result is a [`BackgroundCut`], applied to the pixels or to the layer mask exactly like Remove Background's.
//!
//! Determinism (ARCHITECTURE 4.7): fixed-order `f64` sums, a seeded generator, the max-flow on one thread; rayon only
//! over independent pixels or rows whose sums are added in row order. Identical results for any thread count.

use rayon::prelude::*;

use crate::math::IRect;
use crate::pixel::PixelBuf;
use crate::sticker::cutout::{BackgroundCut, soft_edge};
use crate::text::edt::distance_to_ink;

/// GrabCut's smoothness weight.
pub const GAMMA: f32 = 50.0;
/// Components of each colour model.
pub const COMPONENTS: usize = 5;
/// Most GrabCut iterations on the working copy.
pub const ITERATIONS: usize = 5;
/// Longest side of the working copy (pixels).
pub const WORK_SIDE: u32 = 512;
/// Half-width of the full-size re-cut band around the working result's edge (pixels).
pub const BAND: f32 = 6.0;
/// The box must hold at least this many picture pixels on each side.
pub const MIN_BOX_SIDE: u32 = 4;
/// Samples a colour model is fitted to at most (every k-th one of a side).
const MAX_SAMPLES: usize = 40_000;
/// Ridge added to the covariance diagonals (colours in 0..=1: about a fifth of an 8-bit level of deviation), so flat
/// colour areas stay well-conditioned.
const RIDGE: f64 = 2e-5;
/// Lloyd iterations of the k-means start.
const KMEANS_ITERS: usize = 10;
/// Margin around the box that the working copy includes (share of the box's longer side; at least `MIN_MARGIN`).
const MARGIN_SHARE: f32 = 0.15;
const MIN_MARGIN: i32 = 8;
/// "No node" in the index planes.
const NONE: u32 = u32::MAX;

/// Why the box cut did nothing. `message()` is the text the UI shows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BoxCutRefusal {
    /// An empty picture.
    Empty,
    /// The box holds fewer than [`MIN_BOX_SIDE`] picture pixels on a side.
    BoxTooSmall,
    /// Nothing opaque to learn the background from (the picture is transparent around and inside the box).
    NoBackground,
    /// The cut kept nothing.
    NothingFound,
    /// The progress callback asked to stop.
    Cancelled,
}

impl BoxCutRefusal {
    /// The note shown to the user.
    pub fn message(self) -> &'static str {
        match self {
            BoxCutRefusal::Empty => "This layer has no pixels",
            BoxCutRefusal::BoxTooSmall => "Drag a larger box around the object (at least 4 x 4 pixels of the picture)",
            BoxCutRefusal::NoBackground => "There is no background around the box to tell the object from",
            BoxCutRefusal::NothingFound => "Nothing to keep was found in the box: drag the box closer around the object",
            BoxCutRefusal::Cancelled => "Cut Out Box was cancelled",
        }
    }
}

/// The GrabCut box cut-out of `src` with the box `rect` (picture pixels, clipped to the picture). `progress` is called
/// with the share done (percent, 0..=100) a few times on the calling thread and returns `false` to stop (the result
/// is then [`BoxCutRefusal::Cancelled`]). Nothing is changed: the returned cut is applied by the caller.
pub fn cut_out_box(src: &PixelBuf, rect: IRect, progress: &dyn Fn(u32) -> bool) -> Result<BackgroundCut, BoxCutRefusal> {
    let (w, h) = (src.width as usize, src.height as usize);
    if w == 0 || h == 0 || src.data.len() != w * h * 4 {
        return Err(BoxCutRefusal::Empty);
    }
    let pic = IRect::from_size(src.width, src.height);
    let r = rect.intersect(&pic);
    if r.width() < MIN_BOX_SIDE || r.height() < MIN_BOX_SIDE {
        return Err(BoxCutRefusal::BoxTooSmall);
    }
    let check = |p: u32| if progress(p) { Ok(()) } else { Err(BoxCutRefusal::Cancelled) };
    check(0)?;
    let margin = ((r.width().max(r.height()) as f32 * MARGIN_SHARE).ceil() as i32).max(MIN_MARGIN);
    let win = IRect::new(r.x0 - margin, r.y0 - margin, r.x1 + margin, r.y1 + margin).intersect(&pic);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let step = ww.max(wh).div_ceil(WORK_SIDE as usize).max(1);

    // 1-2. The working copy and its trimap.
    let low = Low::sample(src, win, step);
    let (lw, lh) = (low.w, low.h);
    let mut fg = vec![false; lw * lh];
    for ly in 0..lh {
        for lx in 0..lw {
            let i = ly * lw + lx;
            if low.solid[i] && low.centre_in(win, lx, ly, &r) {
                fg[i] = true;
            }
        }
    }
    let mut free = fg.clone();
    if !(0..lw * lh).any(|i| low.solid[i] && !free[i]) {
        let ring = (lw.min(lh) / 50).max(1);
        for ly in 0..lh {
            for lx in 0..lw {
                if lx < ring || ly < ring || lx + ring >= lw || ly + ring >= lh {
                    let i = ly * lw + lx;
                    free[i] = false;
                    fg[i] = false;
                }
            }
        }
    }
    if !(0..lw * lh).any(|i| low.solid[i] && !fg[i]) {
        return Err(BoxCutRefusal::NoBackground);
    }
    if !fg.iter().any(|&f| f) {
        return Err(BoxCutRefusal::NothingFound);
    }
    let nodes: Vec<usize> = (0..lw * lh).filter(|&i| free[i]).collect();
    let mut index = vec![NONE; lw * lh];
    for (k, &i) in nodes.iter().enumerate() {
        index[i] = k as u32;
    }
    let low_color = |i: usize| low.solid[i].then(|| low.px[i]);
    let beta = contrast_beta(lw, lh, &low_color);
    check(5)?;

    // 3. GrabCut iterations on the working copy.
    let mut models: Option<(Gmm, Gmm)> = None;
    for it in 0..ITERATIONS {
        let (mut fs, mut bs) = (Vec::new(), Vec::new());
        for ((&solid, &f), &z) in low.solid.iter().zip(&fg).zip(&low.px) {
            if solid {
                if f { fs.push(z) } else { bs.push(z) }
            }
        }
        if fs.is_empty() {
            return Err(BoxCutRefusal::NothingFound);
        }
        let (fs, bs) = (subsample(&fs, MAX_SAMPLES), subsample(&bs, MAX_SAMPLES));
        let (fm, bm) = match models.take() {
            None => (Gmm::fit(&fs, COMPONENTS).ok_or(BoxCutRefusal::NothingFound)?, Gmm::fit(&bs, COMPONENTS).ok_or(BoxCutRefusal::NoBackground)?),
            Some((f, b)) => (f.refit(&fs).unwrap_or(f), b.refit(&bs).unwrap_or(b)),
        };
        let costs: Vec<(f32, f32)> = nodes.par_iter().with_min_len(4096).map(|&i| (fm.neg_log(low.px[i]), bm.neg_log(low.px[i]))).collect();
        let cut = grid_cut(lw, lh, &nodes, &index, &fg, &low_color, &costs, beta);
        let mut changed = 0usize;
        for (&i, &c) in nodes.iter().zip(&cut) {
            changed += (fg[i] != c) as usize;
            fg[i] = c;
        }
        models = Some((fm, bm));
        check(10 + (60 * (it as u32 + 1)) / ITERATIONS as u32)?;
        if changed == 0 {
            break;
        }
    }
    let Some((fm, bm)) = models else { return Err(BoxCutRefusal::NothingFound) };
    if !fg.iter().any(|&f| f) {
        return Err(BoxCutRefusal::NothingFound);
    }

    // 4. Full size: the working result scaled up, re-cut in a band around its edge.
    let keep_win: Vec<bool> = if step == 1 {
        fg
    } else {
        let full_color = |i: usize| -> Option<[f32; 3]> {
            let (x, y) = (win.x0 as usize + i % ww, win.y0 as usize + i / ww);
            let p = &src.data[(y * w + x) * 4..(y * w + x) * 4 + 4];
            (p[3] >= 128).then(|| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])
        };
        // Pixels the re-cut may change: inside the box and opaque.
        let open = |i: usize| -> bool {
            let (x, y) = (win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
            r.contains(x, y) && full_color(i).is_some()
        };
        let mut keep: Vec<bool> = (0..ww * wh)
            .into_par_iter()
            .with_min_len(1 << 14)
            .map(|i| {
                let (x, y) = (i % ww, i / ww);
                fg[(y / step).min(lh - 1) * lw + (x / step).min(lw - 1)] && open(i)
            })
            .collect();
        let plane = |want: bool| -> Vec<u8> { keep.par_iter().with_min_len(1 << 14).map(|&k| if k == want { 255 } else { 0 }).collect() };
        let to_kept = distance_to_ink(&plane(true), ww as u32, wh as u32);
        let to_cleared = distance_to_ink(&plane(false), ww as u32, wh as u32);
        check(75)?;
        let band: Vec<usize> = (0..ww * wh)
            .into_par_iter()
            .with_min_len(1 << 14)
            .filter(|&i| open(i) && if keep[i] { to_cleared[i] <= BAND } else { to_kept[i] <= BAND })
            .collect();
        drop((to_kept, to_cleared));
        if !band.is_empty() {
            let mut index = vec![NONE; ww * wh];
            for (k, &i) in band.iter().enumerate() {
                index[i] = k as u32;
            }
            let beta = contrast_beta(ww, wh, &full_color);
            let costs: Vec<(f32, f32)> = band
                .par_iter()
                .with_min_len(4096)
                .map(|&i| {
                    let z = full_color(i).unwrap_or([0.0; 3]);
                    (fm.neg_log(z), bm.neg_log(z))
                })
                .collect();
            let cut = grid_cut(ww, wh, &band, &index, &keep, &full_color, &costs, beta);
            for (&i, &c) in band.iter().zip(&cut) {
                keep[i] = c;
            }
        }
        keep
    };
    check(90)?;

    // 5. The region (background) of the whole picture and its soft edge.
    let mut region = vec![255u8; w * h];
    for y in 0..wh {
        let row = (win.y0 as usize + y) * w + win.x0 as usize;
        for (x, &k) in keep_win[y * ww..(y + 1) * ww].iter().enumerate() {
            if k {
                region[row + x] = 0;
            }
        }
    }
    let region_pixels = region.iter().filter(|&&v| v != 0).count() as u64;
    if region_pixels == (w * h) as u64 {
        return Err(BoxCutRefusal::NothingFound);
    }
    let (keep, recolor) = soft_edge(src, &region, |rc| [rc[0], rc[1], rc[2]]);
    check(100)?;
    Ok(BackgroundCut { keep, colors: Vec::new(), region_pixels, recolor })
}

/// Every k-th element so that at most `max` remain.
fn subsample<T: Copy>(v: &[T], max: usize) -> Vec<T> {
    if v.len() <= max {
        return v.to_vec();
    }
    let k = v.len().div_ceil(max);
    v.iter().step_by(k).copied().collect()
}

#[inline]
fn d2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    x * x + y * y + z * z
}

// ---------------------------------------------------------------------------------------------------------------
// The working copy
// ---------------------------------------------------------------------------------------------------------------

/// The box-filtered working copy of a window: colours in 0..=1 (alpha-weighted means of a block), and whether a block
/// is mostly opaque (mean alpha >= 128).
struct Low {
    w: usize,
    h: usize,
    step: usize,
    px: Vec<[f32; 3]>,
    solid: Vec<bool>,
}

impl Low {
    fn sample(src: &PixelBuf, win: IRect, step: usize) -> Low {
        let (ww, wh) = (win.width() as usize, win.height() as usize);
        let (lw, lh) = (ww.div_ceil(step), wh.div_ceil(step));
        let sw = src.width as usize;
        let rows: Vec<(Vec<[f32; 3]>, Vec<bool>)> = (0..lh)
            .into_par_iter()
            .map(|ly| {
                let mut px = Vec::with_capacity(lw);
                let mut solid = Vec::with_capacity(lw);
                let (y0, y1) = (ly * step, ((ly + 1) * step).min(wh));
                for lx in 0..lw {
                    let (x0, x1) = (lx * step, ((lx + 1) * step).min(ww));
                    let (mut s, mut a, mut n) = ([0u64; 3], 0u64, 0u64);
                    for y in y0..y1 {
                        let row = (win.y0 as usize + y) * sw + win.x0 as usize;
                        for x in x0..x1 {
                            let p = &src.data[(row + x) * 4..(row + x) * 4 + 4];
                            let al = p[3] as u64;
                            for c in 0..3 {
                                s[c] += p[c] as u64 * al;
                            }
                            a += al;
                            n += 1;
                        }
                    }
                    px.push(if a == 0 { [0.0; 3] } else { s.map(|v| (v as f64 / a as f64 / 255.0) as f32) });
                    solid.push(a * 2 >= n * 255);
                }
                (px, solid)
            })
            .collect();
        let (mut px, mut solid) = (Vec::with_capacity(lw * lh), Vec::with_capacity(lw * lh));
        for (p, s) in rows {
            px.extend(p);
            solid.extend(s);
        }
        Low { w: lw, h: lh, step, px, solid }
    }

    /// The centre of block `(lx, ly)` (picture pixels) lies inside `r`.
    fn centre_in(&self, win: IRect, lx: usize, ly: usize, r: &IRect) -> bool {
        let x0 = win.x0 as f32 + (lx * self.step) as f32;
        let x1 = (win.x0 as f32 + ((lx + 1) * self.step) as f32).min(win.x1 as f32);
        let y0 = win.y0 as f32 + (ly * self.step) as f32;
        let y1 = (win.y0 as f32 + ((ly + 1) * self.step) as f32).min(win.y1 as f32);
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        cx >= r.x0 as f32 && cx < r.x1 as f32 && cy >= r.y0 as f32 && cy < r.y1 as f32
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The grid cut
// ---------------------------------------------------------------------------------------------------------------

/// The four forward neighbours of an 8-connected grid and their lengths.
const NB: [(i32, i32, f32); 4] = [(1, 0, 1.0), (0, 1, 1.0), (1, 1, std::f32::consts::SQRT_2), (-1, 1, std::f32::consts::SQRT_2)];

/// GrabCut's contrast parameter `beta = 1 / (2 <|z_m - z_n|^2>)` over the 8-neighbour pairs of opaque pixels
/// (`color` is `None` for a transparent one). Rows in parallel, their sums added in row order.
fn contrast_beta(w: usize, h: usize, color: &(dyn Fn(usize) -> Option<[f32; 3]> + Sync)) -> f32 {
    let rows: Vec<(f64, u64)> = (0..h)
        .into_par_iter()
        .map(|y| {
            let (mut sum, mut n) = (0.0f64, 0u64);
            for x in 0..w {
                let Some(a) = color(y * w + x) else { continue };
                for (dx, dy, _) in NB {
                    let (nx, ny) = (x as i32 + dx, y + dy as usize);
                    if nx < 0 || nx as usize >= w || ny >= h {
                        continue;
                    }
                    if let Some(b) = color(ny * w + nx as usize) {
                        sum += d2(a, b) as f64;
                        n += 1;
                    }
                }
            }
            (sum, n)
        })
        .collect();
    let (mut sum, mut n) = (0.0f64, 0u64);
    for (s, k) in rows {
        sum += s;
        n += k;
    }
    if n == 0 || sum <= 1e-12 { 0.0 } else { (n as f64 / (2.0 * sum)) as f32 }
}

/// One minimum cut of a `w x h` grid (Boykov and Jolly 2001): minimises the data costs of the free pixels `nodes`
/// (`costs[k]` = the cost of labelling node `k` object, background) plus `GAMMA exp(-beta |z_p - z_q|^2) / dist(p, q)`
/// for every 8-neighbour pair labelled differently. A pixel that is not a node (`index` = `NONE`) keeps its `label`
/// and its links fold into its free neighbours' terminal capacities; a pair with a transparent pixel (`color` =
/// `None`) has no link. Returns the new label of every node (`true` = object).
#[allow(clippy::too_many_arguments)]
fn grid_cut(
    w: usize,
    h: usize,
    nodes: &[usize],
    index: &[u32],
    label: &[bool],
    color: &(dyn Fn(usize) -> Option<[f32; 3]> + Sync),
    costs: &[(f32, f32)],
    beta: f32,
) -> Vec<bool> {
    let mut g = Graph::with_capacity(nodes.len(), nodes.len() * 4);
    for (k, &(cost_fg, cost_bg)) in costs.iter().enumerate() {
        g.add_tweights(k, cost_bg, cost_fg);
    }
    let weight = |a: [f32; 3], b: [f32; 3], len: f32| GAMMA * (-beta * d2(a, b)).exp() / len;
    let fold = |g: &mut Graph, k: usize, object: bool, wt: f32| {
        if object {
            // Labelling the node background cuts the link to an object neighbour.
            g.add_tweights(k, wt, 0.0);
        } else {
            g.add_tweights(k, 0.0, wt);
        }
    };
    for (k, &i) in nodes.iter().enumerate() {
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let Some(a) = color(i) else { continue };
        for (dx, dy, len) in NB {
            for (sx, sy, forward) in [(dx, dy, true), (-dx, -dy, false)] {
                let (nx, ny) = (x + sx, y + sy);
                if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                let Some(b) = color(j) else { continue };
                match index[j] {
                    NONE => fold(&mut g, k, label[j], weight(a, b, len)),
                    other if forward => {
                        let wt = weight(a, b, len);
                        g.add_edge(k, other as usize, wt, wt);
                    }
                    _ => {}
                }
            }
        }
    }
    g.maxflow();
    (0..nodes.len()).map(|k| g.in_source(k)).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Gaussian mixture colour models
// ---------------------------------------------------------------------------------------------------------------

/// One Gaussian component: its weight, mean, the inverse of its (ridged) covariance and
/// `ln(weight) - ln det(cov) / 2 - 3/2 ln(2 pi)`.
#[derive(Clone, Debug)]
struct Component {
    mean: [f64; 3],
    inv: [[f64; 3]; 3],
    log_norm: f64,
}

impl Component {
    fn new(weight: f64, mean: [f64; 3], cov: [[f64; 3]; 3]) -> Option<Component> {
        let mut c = cov;
        for (i, row) in c.iter_mut().enumerate() {
            row[i] += RIDGE;
        }
        let det = c[0][0] * (c[1][1] * c[2][2] - c[1][2] * c[2][1]) - c[0][1] * (c[1][0] * c[2][2] - c[1][2] * c[2][0])
            + c[0][2] * (c[1][0] * c[2][1] - c[1][1] * c[2][0]);
        // NaN fails both tests (a degenerate component is dropped).
        if det.is_nan() || det <= 1e-30 || weight.is_nan() || weight <= 0.0 {
            return None;
        }
        let inv = [
            [(c[1][1] * c[2][2] - c[1][2] * c[2][1]) / det, (c[0][2] * c[2][1] - c[0][1] * c[2][2]) / det, (c[0][1] * c[1][2] - c[0][2] * c[1][1]) / det],
            [(c[1][2] * c[2][0] - c[1][0] * c[2][2]) / det, (c[0][0] * c[2][2] - c[0][2] * c[2][0]) / det, (c[0][2] * c[1][0] - c[0][0] * c[1][2]) / det],
            [(c[1][0] * c[2][1] - c[1][1] * c[2][0]) / det, (c[0][1] * c[2][0] - c[0][0] * c[2][1]) / det, (c[0][0] * c[1][1] - c[0][1] * c[1][0]) / det],
        ];
        let log_norm = weight.ln() - 0.5 * det.ln() - 1.5 * (2.0 * std::f64::consts::PI).ln();
        Some(Component { mean, inv, log_norm })
    }

    /// `ln(weight * N(z; mean, cov))`.
    #[inline]
    fn log_weighted(&self, z: [f32; 3]) -> f64 {
        let d = [z[0] as f64 - self.mean[0], z[1] as f64 - self.mean[1], z[2] as f64 - self.mean[2]];
        let m = &self.inv;
        let q = d[0] * (m[0][0] * d[0] + m[0][1] * d[1] + m[0][2] * d[2])
            + d[1] * (m[1][0] * d[0] + m[1][1] * d[1] + m[1][2] * d[2])
            + d[2] * (m[2][0] * d[0] + m[2][1] * d[1] + m[2][2] * d[2]);
        self.log_norm - 0.5 * q
    }
}

/// Sums of one component's samples.
#[derive(Default, Clone, Copy)]
struct Acc {
    n: f64,
    s: [f64; 3],
    ss: [[f64; 3]; 3],
}

impl Acc {
    fn add(&mut self, z: [f32; 3]) {
        let z = z.map(|v| v as f64);
        self.n += 1.0;
        for i in 0..3 {
            self.s[i] += z[i];
            for j in 0..3 {
                self.ss[i][j] += z[i] * z[j];
            }
        }
    }

    fn component(&self, total: f64) -> Option<Component> {
        if self.n <= 0.0 {
            return None;
        }
        let m = self.s.map(|v| v / self.n);
        let mut c = [[0.0f64; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                c[i][j] = self.ss[i][j] / self.n - m[i] * m[j];
            }
        }
        Component::new(self.n / total, m, c)
    }
}

/// A Gaussian mixture over RGB (0..=1) with full 3 x 3 covariances.
#[derive(Clone, Debug)]
struct Gmm {
    comps: Vec<Component>,
}

impl Gmm {
    /// `k` components (fewer without enough distinct samples) from a k-means++ start with a fixed seed; `None`
    /// without samples.
    fn fit(samples: &[[f32; 3]], k: usize) -> Option<Gmm> {
        if samples.is_empty() {
            return None;
        }
        let centers = kmeans(samples, k.clamp(1, samples.len()), KMEANS_ITERS);
        let labels: Vec<usize> = samples.iter().map(|z| nearest(&centers, *z)).collect();
        Gmm::from_labels(samples, &labels, centers.len())
    }

    /// Component statistics from hard assignments (empty components are dropped).
    fn from_labels(samples: &[[f32; 3]], labels: &[usize], k: usize) -> Option<Gmm> {
        let mut acc = vec![Acc::default(); k];
        for (z, &l) in samples.iter().zip(labels) {
            acc[l].add(*z);
        }
        let total = samples.len() as f64;
        let comps: Vec<Component> = acc.iter().filter_map(|a| a.component(total)).collect();
        (!comps.is_empty()).then_some(Gmm { comps })
    }

    /// GrabCut's update: every sample to its most likely component, then the statistics again.
    fn refit(&self, samples: &[[f32; 3]]) -> Option<Gmm> {
        if samples.is_empty() {
            return None;
        }
        let labels: Vec<usize> = samples.par_iter().with_min_len(4096).map(|z| self.component_of(*z)).collect();
        Gmm::from_labels(samples, &labels, self.comps.len())
    }

    fn component_of(&self, z: [f32; 3]) -> usize {
        let mut best = 0;
        let mut bv = f64::MIN;
        for (i, c) in self.comps.iter().enumerate() {
            let v = c.log_weighted(z);
            if v > bv {
                bv = v;
                best = i;
            }
        }
        best
    }

    /// `-ln p(z)`, clamped to a finite range.
    fn neg_log(&self, z: [f32; 3]) -> f32 {
        let mut vals = [0.0f64; COMPONENTS];
        let n = self.comps.len().min(COMPONENTS);
        let mut mx = f64::MIN;
        for (v, c) in vals.iter_mut().zip(&self.comps) {
            *v = c.log_weighted(z);
            mx = mx.max(*v);
        }
        let s: f64 = vals[..n].iter().map(|v| (v - mx).exp()).sum();
        let lp = mx + s.ln();
        if lp.is_finite() { (-lp).clamp(-50.0, 200.0) as f32 } else { 200.0 }
    }
}

fn nearest(centers: &[[f32; 3]], z: [f32; 3]) -> usize {
    let mut best = 0;
    let mut bd = f32::MAX;
    for (i, c) in centers.iter().enumerate() {
        let d = d2(*c, z);
        if d < bd {
            bd = d;
            best = i;
        }
    }
    best
}

/// A small xorshift generator with a fixed seed (the k-means++ start).
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// k-means with a k-means++ start (seeded by the number of samples: the same samples give the same centres); empty
/// clusters are dropped.
fn kmeans(samples: &[[f32; 3]], k: usize, iters: usize) -> Vec<[f32; 3]> {
    let mut rng = Rng((0x5EED_u64 + samples.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut centers = vec![samples[(rng.next_u64() % samples.len() as u64) as usize]];
    let mut dist: Vec<f32> = samples.iter().map(|z| d2(*z, centers[0])).collect();
    while centers.len() < k {
        let total: f64 = dist.iter().map(|d| *d as f64).sum();
        if total <= 1e-12 {
            break;
        }
        let mut t = rng.unit() * total;
        let mut pick = samples.len() - 1;
        for (i, d) in dist.iter().enumerate() {
            t -= *d as f64;
            if t <= 0.0 {
                pick = i;
                break;
            }
        }
        let c = samples[pick];
        centers.push(c);
        for (d, z) in dist.iter_mut().zip(samples) {
            *d = d.min(d2(*z, c));
        }
    }
    for _ in 0..iters {
        let mut sums = vec![([0.0f64; 3], 0usize); centers.len()];
        for z in samples {
            let i = nearest(&centers, *z);
            for (acc, v) in sums[i].0.iter_mut().zip(z) {
                *acc += *v as f64;
            }
            sums[i].1 += 1;
        }
        let new: Vec<[f32; 3]> = sums.iter().filter(|s| s.1 > 0).map(|(s, n)| s.map(|v| (v / *n as f64) as f32)).collect();
        let done = new == centers;
        centers = new;
        if done {
            break;
        }
    }
    centers
}

// ---------------------------------------------------------------------------------------------------------------
// Boykov-Kolmogorov max-flow
// ---------------------------------------------------------------------------------------------------------------

const NO_ARC: u32 = u32::MAX;
const TERMINAL: u32 = u32::MAX - 1;
const ORPHAN: u32 = u32::MAX - 2;
const INF_DIST: u32 = u32::MAX;
/// Residual capacities at or below this count as saturated (float round-off would otherwise leave long chains of
/// negligible augmentations).
const EPS: f32 = 1e-5;

/// A directed graph with source / sink terminal capacities, solved by the Boykov-Kolmogorov algorithm: two search
/// trees (source and sink) grow toward each other, each contact gives an augmenting path, saturated tree arcs make
/// orphans that are adopted again or freed; the trees are reused between augmentations. The source side is the
/// object.
struct Graph {
    first: Vec<u32>,
    /// Residual terminal capacity: `> 0` from the source, `< 0` to the sink.
    tr_cap: Vec<f32>,
    parent: Vec<u32>,
    is_sink: Vec<bool>,
    ts: Vec<u32>,
    dist: Vec<u32>,
    active: Vec<bool>,
    // Arcs in pairs: arc `a` and its reverse `a ^ 1`.
    head: Vec<u32>,
    next: Vec<u32>,
    r_cap: Vec<f32>,
}

impl Graph {
    fn with_capacity(nodes: usize, edges: usize) -> Graph {
        Graph {
            first: vec![NO_ARC; nodes],
            tr_cap: vec![0.0; nodes],
            parent: vec![NO_ARC; nodes],
            is_sink: vec![false; nodes],
            ts: vec![0; nodes],
            dist: vec![0; nodes],
            active: vec![false; nodes],
            head: Vec::with_capacity(edges * 2),
            next: Vec::with_capacity(edges * 2),
            r_cap: Vec::with_capacity(edges * 2),
        }
    }

    /// Adds capacities from the source and to the sink (accumulating); only their difference is kept (the common
    /// part is flow that passes anyway).
    fn add_tweights(&mut self, i: usize, cap_source: f32, cap_sink: f32) {
        let delta = self.tr_cap[i];
        let (mut s, mut t) = (cap_source, cap_sink);
        if delta > 0.0 {
            s += delta;
        } else {
            t -= delta;
        }
        self.tr_cap[i] = s - t;
    }

    /// An edge `i -> j` with capacity `cap` and `j -> i` with `rev_cap`.
    fn add_edge(&mut self, i: usize, j: usize, cap: f32, rev_cap: f32) {
        let a = self.head.len() as u32;
        self.head.push(j as u32);
        self.next.push(self.first[i]);
        self.r_cap.push(cap);
        self.first[i] = a;
        self.head.push(i as u32);
        self.next.push(self.first[j]);
        self.r_cap.push(rev_cap);
        self.first[j] = a + 1;
    }

    /// After `maxflow`: node `i` is on the source side of the minimum cut (nodes reached from neither terminal are
    /// on the sink side).
    fn in_source(&self, i: usize) -> bool {
        self.parent[i] != NO_ARC && !self.is_sink[i]
    }

    fn maxflow(&mut self) {
        use std::collections::VecDeque;
        let n = self.first.len();
        let mut queue: VecDeque<u32> = VecDeque::new();
        let mut orphans: VecDeque<u32> = VecDeque::new();
        for i in 0..n {
            self.active[i] = false;
            self.ts[i] = 0;
            if self.tr_cap[i].abs() > EPS {
                self.is_sink[i] = self.tr_cap[i] < 0.0;
                self.parent[i] = TERMINAL;
                self.dist[i] = 1;
                self.active[i] = true;
                queue.push_back(i as u32);
            } else {
                self.parent[i] = NO_ARC;
            }
        }
        let mut time = 0u32;
        while let Some(i) = queue.pop_front() {
            let i = i as usize;
            self.active[i] = false;
            if self.parent[i] == NO_ARC {
                continue;
            }
            // Growth.
            let mut contact = NO_ARC;
            let sink = self.is_sink[i];
            let mut a = self.first[i];
            while a != NO_ARC {
                let au = a as usize;
                let j = self.head[au] as usize;
                let cap = if sink { self.r_cap[au ^ 1] } else { self.r_cap[au] };
                if cap > EPS {
                    if self.parent[j] == NO_ARC {
                        self.is_sink[j] = sink;
                        self.parent[j] = a ^ 1;
                        self.ts[j] = self.ts[i];
                        self.dist[j] = self.dist[i] + 1;
                        if !self.active[j] {
                            self.active[j] = true;
                            queue.push_back(j as u32);
                        }
                    } else if self.is_sink[j] != sink {
                        contact = if sink { a ^ 1 } else { a };
                        break;
                    } else if self.ts[j] <= self.ts[i] && self.dist[j] > self.dist[i] {
                        // Shorter paths to the terminal.
                        self.parent[j] = a ^ 1;
                        self.ts[j] = self.ts[i];
                        self.dist[j] = self.dist[i] + 1;
                    }
                }
                a = self.next[au];
            }
            if contact == NO_ARC {
                continue;
            }
            // This node is scanned again after the augmentation.
            if !self.active[i] {
                self.active[i] = true;
                queue.push_front(i as u32);
            }
            time += 1;
            self.augment(contact as usize, &mut orphans);
            while let Some(o) = orphans.pop_front() {
                self.adopt(o as usize, time, &mut queue, &mut orphans);
            }
        }
    }

    fn augment(&mut self, m: usize, orphans: &mut std::collections::VecDeque<u32>) {
        let u = self.head[m ^ 1] as usize;
        let v = self.head[m] as usize;
        // The bottleneck.
        let mut b = self.r_cap[m];
        let mut x = u;
        loop {
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            b = b.min(self.r_cap[a as usize ^ 1]);
            x = self.head[a as usize] as usize;
        }
        b = b.min(self.tr_cap[x]);
        let mut x = v;
        loop {
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            b = b.min(self.r_cap[a as usize]);
            x = self.head[a as usize] as usize;
        }
        b = b.min(-self.tr_cap[x]);
        // The push.
        self.r_cap[m ^ 1] += b;
        self.r_cap[m] -= b;
        let mut x = u;
        loop {
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            let au = a as usize;
            self.r_cap[au] += b;
            self.r_cap[au ^ 1] -= b;
            let up = self.head[au] as usize;
            if self.r_cap[au ^ 1] <= EPS {
                self.parent[x] = ORPHAN;
                orphans.push_back(x as u32);
            }
            x = up;
        }
        self.tr_cap[x] -= b;
        if self.tr_cap[x] <= EPS {
            self.parent[x] = ORPHAN;
            orphans.push_back(x as u32);
        }
        let mut x = v;
        loop {
            let a = self.parent[x];
            if a == TERMINAL {
                break;
            }
            let au = a as usize;
            self.r_cap[au ^ 1] += b;
            self.r_cap[au] -= b;
            let up = self.head[au] as usize;
            if self.r_cap[au] <= EPS {
                self.parent[x] = ORPHAN;
                orphans.push_back(x as u32);
            }
            x = up;
        }
        self.tr_cap[x] += b;
        if self.tr_cap[x] >= -EPS {
            self.parent[x] = ORPHAN;
            orphans.push_back(x as u32);
        }
    }

    fn adopt(&mut self, i: usize, time: u32, queue: &mut std::collections::VecDeque<u32>, orphans: &mut std::collections::VecDeque<u32>) {
        let sink = self.is_sink[i];
        let mut best = NO_ARC;
        let mut d_min = INF_DIST;
        let mut a = self.first[i];
        while a != NO_ARC {
            let au = a as usize;
            let cap = if sink { self.r_cap[au] } else { self.r_cap[au ^ 1] };
            let j = self.head[au] as usize;
            if cap > EPS && self.is_sink[j] == sink && self.parent[j] != NO_ARC {
                // Does j's path lead to a terminal?
                let mut d = 0u32;
                let mut k = j;
                loop {
                    if self.ts[k] == time {
                        d = d.saturating_add(self.dist[k]);
                        break;
                    }
                    let pa = self.parent[k];
                    d += 1;
                    if pa == TERMINAL {
                        self.ts[k] = time;
                        self.dist[k] = 1;
                        break;
                    }
                    if pa == ORPHAN {
                        d = INF_DIST;
                        break;
                    }
                    k = self.head[pa as usize] as usize;
                }
                if d < INF_DIST {
                    if d < d_min {
                        best = a;
                        d_min = d;
                    }
                    // Cache the distances along the path.
                    let mut k = j;
                    let mut dd = d;
                    while self.ts[k] != time {
                        self.ts[k] = time;
                        self.dist[k] = dd;
                        dd -= 1;
                        k = self.head[self.parent[k] as usize] as usize;
                    }
                }
            }
            a = self.next[au];
        }
        if best != NO_ARC {
            self.parent[i] = best;
            self.ts[i] = time;
            self.dist[i] = d_min + 1;
            return;
        }
        // No parent: the node becomes free.
        self.parent[i] = NO_ARC;
        let mut a = self.first[i];
        while a != NO_ARC {
            let au = a as usize;
            let j = self.head[au] as usize;
            let pj = self.parent[j];
            if self.is_sink[j] == sink && pj != NO_ARC {
                let cap = if sink { self.r_cap[au] } else { self.r_cap[au ^ 1] };
                if cap > EPS && !self.active[j] {
                    self.active[j] = true;
                    queue.push_back(j as u32);
                }
                if pj != TERMINAL && pj != ORPHAN && self.head[pj as usize] as usize == i {
                    self.parent[j] = ORPHAN;
                    orphans.push_back(j as u32);
                }
            }
            a = self.next[au];
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
