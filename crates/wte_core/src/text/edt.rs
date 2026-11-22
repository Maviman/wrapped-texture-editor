// Owner: core-stamps-text (C4). Exact Euclidean distance transform for text outlines (ARCHITECTURE.md 5.13, 6.8).
//
// Felzenszwalb-Huttenlocher in two 1-D passes. The column pass is done with two running sweeps (nearest ink above and
// below), the row pass builds the lower envelope of parabolas and is independent per row, so it runs in parallel.

use rayon::prelude::*;

/// Coverage from which a pixel counts as ink.
const INK: u8 = 128;
/// "No ink in this column" marker of the column pass.
const NONE: i32 = i32::MAX;
/// Images with at least this many pixels run the row pass in parallel.
const PAR_PIXELS: usize = 1 << 16;

/// Exact Euclidean distance (in pixels) from every pixel to the nearest pixel with coverage >= 128, with the
/// sub-pixel edge correction of section 6.8. Returns w*h distances (0 inside).
///
/// For a pixel outside the ink the result is `sqrt(d2) + 0.5 - coverage[site] / 255`, where `site` is the nearest ink
/// pixel (centre to centre) and `d2` the squared distance to it: a fully covered site has its edge half a pixel
/// beyond its centre, a half covered one right at its centre. Among equally near sites the one the transform finds
/// is used. Without any ink every distance is `f32::INFINITY`; so is the whole result when `coverage` is shorter than
/// `width * height`.
pub fn distance_to_ink(coverage: &[u8], width: u32, height: u32) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    let n = w * h;
    if n == 0 {
        return Vec::new();
    }
    if coverage.len() < n {
        return vec![f32::INFINITY; n];
    }

    // Column pass: `dy[i]` = signed row offset from pixel i to the nearest ink pixel of its column (negative = above),
    // NONE when the column has no ink. Ties go to the pixel above.
    let mut dy = vec![NONE; n];
    for y in 0..h {
        let (done, row) = dy.split_at_mut(y * w);
        let above = if y > 0 { Some(&done[(y - 1) * w..]) } else { None };
        for x in 0..w {
            if coverage[y * w + x] >= INK {
                row[x] = 0;
            } else if let Some(above) = above
                && above[x] != NONE
            {
                row[x] = above[x] - 1;
            }
        }
    }
    let mut below = vec![NONE; w];
    for y in (0..h).rev() {
        let row = &mut dy[y * w..(y + 1) * w];
        for x in 0..w {
            below[x] = if coverage[y * w + x] >= INK {
                0
            } else if below[x] == NONE {
                NONE
            } else {
                below[x] + 1
            };
            if below[x] != NONE && (row[x] == NONE || below[x] < -row[x]) {
                row[x] = below[x];
            }
        }
    }

    // Row pass: lower envelope of the parabolas `(x - q)^2 + dy[q]^2` over the columns q that have ink.
    let mut out = vec![0.0f32; n];
    let dy = &dy;
    let do_row = |(hull, starts): &mut (Vec<usize>, Vec<f64>), (y, row): (usize, &mut [f32])| {
        let d = &dy[y * w..(y + 1) * w];
        hull.clear();
        starts.clear();
        let height_at = |q: usize| -> f64 {
            let v = d[q] as f64;
            v * v + (q * q) as f64
        };
        for (q, &dq) in d.iter().enumerate() {
            if dq == NONE {
                continue;
            }
            let fq = height_at(q);
            loop {
                let Some(&p) = hull.last() else {
                    hull.push(q);
                    starts.push(f64::NEG_INFINITY);
                    break;
                };
                // Abscissa from which parabola q lies below parabola p.
                let s = (fq - height_at(p)) / (2.0 * (q - p) as f64);
                if s <= starts[starts.len() - 1] {
                    hull.pop();
                    starts.pop();
                } else {
                    hull.push(q);
                    starts.push(s);
                    break;
                }
            }
        }
        if hull.is_empty() {
            row.fill(f32::INFINITY);
            return;
        }
        let mut k = 0;
        for (x, value) in row.iter_mut().enumerate() {
            while k + 1 < hull.len() && starts[k + 1] < x as f64 {
                k += 1;
            }
            let q = hull[k];
            let dx = x as f64 - q as f64;
            let dyq = d[q] as f64;
            let d2 = dx * dx + dyq * dyq;
            *value = if d2 <= 0.0 {
                0.0
            } else {
                let site = (y as i64 + d[q] as i64) as usize * w + q;
                (d2.sqrt() as f32 + 0.5 - coverage[site] as f32 / 255.0).max(0.0)
            };
        }
    };
    if n >= PAR_PIXELS {
        out.par_chunks_mut(w).enumerate().for_each_init(|| (Vec::new(), Vec::new()), do_row);
    } else {
        let mut scratch = (Vec::new(), Vec::new());
        out.chunks_mut(w).enumerate().for_each(|item| do_row(&mut scratch, item));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::XorShift;

    /// For every pixel: the smallest and the largest corrected distance over all ink pixels that are nearest to it
    /// (centre to centre); `None` without ink.
    fn brute_force(cov: &[u8], w: usize, h: usize) -> Vec<Option<(f32, f32)>> {
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut best: Option<i64> = None;
                for sy in 0..h {
                    for sx in 0..w {
                        if cov[sy * w + sx] >= INK {
                            let d2 = (sx as i64 - x as i64).pow(2) + (sy as i64 - y as i64).pow(2);
                            best = Some(best.map_or(d2, |b| b.min(d2)));
                        }
                    }
                }
                let Some(best) = best else {
                    out.push(None);
                    continue;
                };
                if best == 0 {
                    out.push(Some((0.0, 0.0)));
                    continue;
                }
                let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                for sy in 0..h {
                    for sx in 0..w {
                        let d2 = (sx as i64 - x as i64).pow(2) + (sy as i64 - y as i64).pow(2);
                        if cov[sy * w + sx] >= INK && d2 == best {
                            let d = ((best as f64).sqrt() as f32 + 0.5 - cov[sy * w + sx] as f32 / 255.0).max(0.0);
                            lo = lo.min(d);
                            hi = hi.max(d);
                        }
                    }
                }
                out.push(Some((lo, hi)));
            }
        }
        out
    }

    fn check(cov: &[u8], w: usize, h: usize) {
        let got = distance_to_ink(cov, w as u32, h as u32);
        let want = brute_force(cov, w, h);
        assert_eq!(got.len(), w * h);
        for (i, (g, wnt)) in got.iter().zip(&want).enumerate() {
            match wnt {
                None => assert!(g.is_infinite(), "pixel {i}: {g}"),
                Some((lo, hi)) => {
                    assert!(*g >= lo - 1e-4 && *g <= hi + 1e-4, "pixel {i} ({}, {}): {g} not in [{lo}, {hi}]", i % w, i / w)
                }
            }
        }
    }

    #[test]
    fn matches_brute_force_on_a_16x16_mask() {
        let mut rng = XorShift::new(7);
        // Binary masks of different densities: the correction is the same for every site, so the result is unique.
        for density in [0.02f32, 0.1, 0.4, 0.9] {
            let cov: Vec<u8> = (0..256).map(|_| if rng.next_f32() < density { 255 } else { 0 }).collect();
            check(&cov, 16, 16);
            let got = distance_to_ink(&cov, 16, 16);
            for (i, &c) in cov.iter().enumerate() {
                assert_eq!(got[i] == 0.0, c == 255, "exactly the ink pixels are at distance 0");
            }
        }
        // Anti-aliased coverage: the sub-pixel correction of the nearest site.
        for seed in 0..4 {
            let mut rng = XorShift::new(100 + seed);
            let cov: Vec<u8> =
                (0..256).map(|_| if rng.next_f32() < 0.15 { (rng.next_u32() % 256) as u8 } else { 0 }).collect();
            check(&cov, 16, 16);
        }
    }

    #[test]
    fn non_square_and_degenerate_shapes() {
        let mut rng = XorShift::new(3);
        for (w, h) in [(1usize, 1usize), (1, 9), (9, 1), (5, 13), (23, 4)] {
            let cov: Vec<u8> = (0..w * h).map(|_| if rng.next_f32() < 0.2 { 200 } else { 30 }).collect();
            check(&cov, w, h);
        }
        assert!(distance_to_ink(&[], 0, 7).is_empty());
        assert!(distance_to_ink(&[], 7, 0).is_empty());
        // No ink at all, and a coverage slice that is too short.
        assert!(distance_to_ink(&[0, 100, 127, 5], 2, 2).iter().all(|d| d.is_infinite()));
        assert!(distance_to_ink(&[255], 2, 2).iter().all(|d| d.is_infinite()));
    }

    #[test]
    fn single_site_gives_the_analytic_distance() {
        let (w, h) = (9usize, 7usize);
        let mut cov = vec![0u8; w * h];
        cov[3 * w + 4] = 255;
        let d = distance_to_ink(&cov, w as u32, h as u32);
        for y in 0..h {
            for x in 0..w {
                let r = ((x as f32 - 4.0).powi(2) + (y as f32 - 3.0).powi(2)).sqrt();
                let want = if r == 0.0 { 0.0 } else { r - 0.5 };
                assert!((d[y * w + x] - want).abs() < 1e-5, "({x}, {y}): {} vs {want}", d[y * w + x]);
            }
        }
        // A half covered site has its edge at its centre.
        cov[3 * w + 4] = 128;
        let d = distance_to_ink(&cov, w as u32, h as u32);
        assert!((d[3 * w + 6] - (2.0 + 0.5 - 128.0 / 255.0)).abs() < 1e-5);
        assert_eq!(d[3 * w + 4], 0.0);
    }

    #[test]
    fn parallel_path_equals_the_serial_one() {
        // 300 x 300 is above the parallel threshold; compare a window against brute force on a sparse mask.
        let (w, h) = (300usize, 300usize);
        let mut cov = vec![0u8; w * h];
        let mut rng = XorShift::new(11);
        let mut sites = Vec::new();
        for _ in 0..40 {
            let (x, y) = ((rng.next_u32() % 300) as usize, (rng.next_u32() % 300) as usize);
            cov[y * w + x] = 255;
            sites.push((x as f32, y as f32));
        }
        let d = distance_to_ink(&cov, w as u32, h as u32);
        for y in (0..h).step_by(7) {
            for x in (0..w).step_by(5) {
                let r = sites.iter().map(|&(sx, sy)| ((sx - x as f32).powi(2) + (sy - y as f32).powi(2)).sqrt()).fold(f32::INFINITY, f32::min);
                let want = if r == 0.0 { 0.0 } else { r - 0.5 };
                assert!((d[y * w + x] - want).abs() < 1e-3, "({x}, {y}): {} vs {want}", d[y * w + x]);
            }
        }
    }
}
