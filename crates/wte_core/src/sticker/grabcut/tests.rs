//! Tests of the GrabCut box cut-out (STICKER_PLAN 7 "S1"): three synthetic photo-like pictures with textured
//! backgrounds reach IoU >= 0.9, determinism for 1 and 8 threads, the refusals, cancelling, the progress, the box over
//! the whole picture, transparent pixels, and the max-flow against a brute-force minimum on small grids.

use super::*;
use crate::testutil::XorShift;

/// Smooth value noise in 0..=1: random values on a grid of `cell` pixels, bilinearly interpolated (smoothstep).
fn value_noise(w: u32, h: u32, cell: f32, seed: u64) -> Vec<f32> {
    let (gw, gh) = ((w as f32 / cell).ceil() as usize + 2, (h as f32 / cell).ceil() as usize + 2);
    let mut rng = XorShift::new(seed);
    let grid: Vec<f32> = (0..gw * gh).map(|_| rng.next_f32()).collect();
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / cell, y as f32 / cell);
            let (ix, iy) = (fx as usize, fy as usize);
            let s = |t: f32| t * t * (3.0 - 2.0 * t);
            let (tx, ty) = (s(fx - ix as f32), s(fy - iy as f32));
            let g = |gx: usize, gy: usize| grid[gy * gw + gx];
            let top = g(ix, iy) * (1.0 - tx) + g(ix + 1, iy) * tx;
            let bottom = g(ix, iy + 1) * (1.0 - tx) + g(ix + 1, iy + 1) * tx;
            out.push(top * (1.0 - ty) + bottom * ty);
        }
    }
    out
}

fn clamp_byte(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// The three test scenes.
#[derive(Copy, Clone, Debug)]
pub(crate) enum Scene {
    /// An orange ball with shading on grass (green value noise with brown patches and grain), 400 x 300.
    BallOnGrass,
    /// A white mug with a red band and a handle on a wooden table under a blue-grey wall, 640 x 480.
    MugOnWood,
    /// A yellow star with a dark rim and a blue centre on a purple / teal carpet, any size (1200 x 900 in the test:
    /// the working copy is scaled, so the full-size re-cut runs).
    StarOnCarpet,
}

/// A scene at `w x h`: the picture (opaque), the true object mask (coverage of 4 x 4 samples >= 1/2), the box the
/// user would drag (the object's bounds plus a margin).
pub(crate) fn scene(kind: Scene, w: u32, h: u32) -> (PixelBuf, Vec<bool>, IRect) {
    let (wf, hf) = (w as f32, h as f32);
    let mut rng = XorShift::new(w as u64 * 31 + h as u64 + kind as u64 * 7);
    let n1 = value_noise(w, h, wf / 12.0, 11 + kind as u64);
    let n2 = value_noise(w, h, wf / 40.0, 23 + kind as u64);
    // Inside test of the object (and an inner part with another colour).
    let (cx, cy) = (wf * 0.5, hf * 0.52);
    let size = wf.min(hf);
    let inside = |x: f32, y: f32| -> u8 {
        match kind {
            Scene::BallOnGrass => ((x - cx).powi(2) + (y - cy).powi(2) <= (size * 0.3).powi(2)) as u8,
            Scene::MugOnWood => {
                let (bx0, bx1, by0, by1) = (cx - size * 0.2, cx + size * 0.14, cy - size * 0.25, cy + size * 0.25);
                let body = x >= bx0 && x <= bx1 && y >= by0 && y <= by1;
                let (hx, hy) = (bx1, cy);
                let d = ((x - hx).powi(2) + (y - hy).powi(2)).sqrt();
                let handle = x > bx1 && d >= size * 0.08 && d <= size * 0.14;
                if body {
                    if (y - cy).abs() <= size * 0.04 { 2 } else { 1 }
                } else {
                    handle as u8
                }
            }
            Scene::StarOnCarpet => {
                let (dx, dy) = (x - cx, y - cy);
                let r = (dx * dx + dy * dy).sqrt();
                let a = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
                // A five-pointed star with round-ish arms: radius between 0.17 and 0.36 of the size.
                let k = (a * 5.0 / 2.0).cos().abs();
                let edge = size * (0.17 + 0.19 * k.powf(3.0));
                if r > edge {
                    0
                } else if r <= size * 0.08 {
                    2
                } else if r >= edge - size * 0.012 {
                    3
                } else {
                    1
                }
            }
        }
    };
    let mut p = PixelBuf::new(w, h);
    let mut truth = vec![false; (w * h) as usize];
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            // Coverage and the part from 4 x 4 samples.
            let mut cov = 0u32;
            let mut part = 0u8;
            for sy in 0..4 {
                for sx in 0..4 {
                    let v = inside(x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0);
                    if v > 0 {
                        cov += 1;
                        part = part.max(v);
                    }
                }
            }
            let k = cov as f32 / 16.0;
            truth[i] = cov >= 8;
            if cov > 0 {
                x0 = x0.min(x as i32);
                y0 = y0.min(y as i32);
                x1 = x1.max(x as i32 + 1);
                y1 = y1.max(y as i32 + 1);
            }
            let (a, b) = (n1[i], n2[i]);
            let grain = (rng.next_f32() - 0.5) * 2.0;
            let bg: [f32; 3] = match kind {
                Scene::BallOnGrass => {
                    let brown = (a > 0.68) as u8 as f32;
                    let g = [50.0 + 40.0 * b, 110.0 + 60.0 * a, 35.0 + 25.0 * b];
                    let br = [120.0 + 30.0 * b, 90.0 + 25.0 * b, 50.0 + 20.0 * a];
                    std::array::from_fn(|c| g[c] * (1.0 - brown) + br[c] * brown + grain * 18.0)
                }
                Scene::MugOnWood => {
                    if (y as f32) < hf * 0.62 {
                        [95.0 + 30.0 * a + grain * 6.0, 110.0 + 30.0 * a + grain * 6.0, 140.0 + 25.0 * a + grain * 6.0]
                    } else {
                        let ring = ((y as f32 * 0.35 + 18.0 * a + 6.0 * b).sin() * 0.5 + 0.5) * 0.6 + 0.4 * b;
                        [120.0 + 70.0 * ring + grain * 10.0, 75.0 + 45.0 * ring + grain * 8.0, 40.0 + 25.0 * ring + grain * 6.0]
                    }
                }
                Scene::StarOnCarpet => {
                    let t = (a * 2.0 + b).fract();
                    let purple = [110.0, 50.0, 140.0];
                    let teal = [30.0, 130.0, 130.0];
                    std::array::from_fn(|c| purple[c] * t + teal[c] * (1.0 - t) + grain * 22.0)
                }
            };
            let shade = 1.0 - 0.25 * ((x as f32 - cx) / size + (y as f32 - cy) / size);
            let obj: [f32; 3] = match (kind, part) {
                (Scene::BallOnGrass, _) => [235.0 * shade, 125.0 * shade, 30.0 * shade],
                (Scene::MugOnWood, 2) => [205.0 * shade, 30.0, 35.0],
                (Scene::MugOnWood, _) => [235.0 * shade, 232.0 * shade, 225.0 * shade],
                (Scene::StarOnCarpet, 2) => [40.0, 80.0, 220.0 * shade],
                (Scene::StarOnCarpet, 3) => [120.0, 90.0, 10.0],
                (Scene::StarOnCarpet, _) => [250.0 * shade, 215.0 * shade, 40.0],
            };
            let fine = (rng.next_f32() - 0.5) * 10.0;
            let rgb: [u8; 3] = std::array::from_fn(|c| clamp_byte(bg[c] * (1.0 - k) + (obj[c] + fine) * k));
            p.set(x, y, [rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    let m = (((x1 - x0).max(y1 - y0)) as f32 * 0.06).ceil() as i32 + 3;
    (p, truth, IRect::new(x0 - m, y0 - m, x1 + m, y1 + m))
}

/// Intersection over union of `keep >= 128` and the truth.
fn iou(cut: &BackgroundCut, truth: &[bool]) -> f64 {
    let (mut inter, mut union) = (0u64, 0u64);
    for (&k, &t) in cut.keep.data.iter().zip(truth) {
        let kept = k >= 128;
        inter += (kept && t) as u64;
        union += (kept || t) as u64;
    }
    inter as f64 / union.max(1) as f64
}

fn run(kind: Scene, w: u32, h: u32) -> (BackgroundCut, f64) {
    let (p, truth, b) = scene(kind, w, h);
    let cut = cut_out_box(&p, b, &|_| true).expect("the cut");
    let v = iou(&cut, &truth);
    (cut, v)
}

#[test]
fn k_s1_ball_on_grass_reaches_iou_0_9() {
    let (_, v) = run(Scene::BallOnGrass, 400, 300);
    eprintln!("ball on grass: IoU {v:.4}");
    assert!(v >= 0.9, "IoU {v:.4}");
}

#[test]
fn k_s1_mug_on_wood_reaches_iou_0_9() {
    let (_, v) = run(Scene::MugOnWood, 640, 480);
    eprintln!("mug on wood: IoU {v:.4}");
    assert!(v >= 0.9, "IoU {v:.4}");
}

#[test]
fn k_s1_star_on_carpet_reaches_iou_0_9_through_the_full_size_re_cut() {
    let (p, _, b) = scene(Scene::StarOnCarpet, 1200, 900);
    let r = b.intersect(&IRect::from_size(p.width, p.height));
    let margin = ((r.width().max(r.height()) as f32 * MARGIN_SHARE).ceil() as i32).max(MIN_MARGIN);
    let win = IRect::new(r.x0 - margin, r.y0 - margin, r.x1 + margin, r.y1 + margin).intersect(&IRect::from_size(p.width, p.height));
    assert!(win.width().max(win.height()) > WORK_SIDE, "the working copy must be scaled for this test");
    let (_, v) = run(Scene::StarOnCarpet, 1200, 900);
    eprintln!("star on carpet: IoU {v:.4}");
    assert!(v >= 0.9, "IoU {v:.4}");
}

#[test]
fn k_s1_everything_outside_the_box_is_cleared_and_the_soft_edge_is_inside_it() {
    let (p, _, b) = scene(Scene::BallOnGrass, 400, 300);
    let cut = cut_out_box(&p, b, &|_| true).unwrap();
    for y in 0..p.height as i32 {
        for x in 0..p.width as i32 {
            let k = cut.keep.data[(y as u32 * p.width + x as u32) as usize];
            if !b.contains(x, y) {
                assert_eq!(k, 0, "({x}, {y}) outside the box");
            }
        }
    }
    assert!(cut.colors.is_empty());
    assert!(!cut.recolor.is_empty(), "the edge is soft");
    let zeros = cut.keep.data.iter().filter(|&&k| k == 0).count() as u64;
    assert!(cut.region_pixels > 0 && cut.region_pixels <= zeros);
}

#[test]
fn k_s1_the_same_for_1_and_8_threads_and_for_two_runs() {
    let (p, _, b) = scene(Scene::StarOnCarpet, 900, 700);
    let pools = [1usize, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
    let a = pools[0].install(|| cut_out_box(&p, b, &|_| true).unwrap());
    let c = pools[1].install(|| cut_out_box(&p, b, &|_| true).unwrap());
    let again = pools[1].install(|| cut_out_box(&p, b, &|_| true).unwrap());
    assert_eq!(a, c);
    assert_eq!(c, again);
}

#[test]
fn k_s1_refusals() {
    let (p, _, b) = scene(Scene::BallOnGrass, 120, 90);
    assert_eq!(cut_out_box(&PixelBuf::new(0, 0), b, &|_| true).unwrap_err(), BoxCutRefusal::Empty);
    assert_eq!(cut_out_box(&p, IRect::new(10, 10, 13, 40), &|_| true).unwrap_err(), BoxCutRefusal::BoxTooSmall);
    assert_eq!(cut_out_box(&p, IRect::new(200, 200, 300, 300), &|_| true).unwrap_err(), BoxCutRefusal::BoxTooSmall);
    // A transparent picture: no background to learn from.
    let clear = PixelBuf::new(120, 90);
    assert_eq!(cut_out_box(&clear, b, &|_| true).unwrap_err(), BoxCutRefusal::NoBackground);
    // Cancelled at once and after some steps.
    assert_eq!(cut_out_box(&p, b, &|_| false).unwrap_err(), BoxCutRefusal::Cancelled);
    assert_eq!(cut_out_box(&p, b, &|pc| pc < 30).unwrap_err(), BoxCutRefusal::Cancelled);
    for r in [BoxCutRefusal::Empty, BoxCutRefusal::BoxTooSmall, BoxCutRefusal::NoBackground, BoxCutRefusal::NothingFound, BoxCutRefusal::Cancelled] {
        assert!(!r.message().is_empty());
    }
}

#[test]
fn k_s1_a_box_over_plain_background_keeps_nothing_or_little() {
    // A box over grass only: the colour models of both sides are grass; whatever is kept must be small.
    let (p, truth, _) = scene(Scene::BallOnGrass, 400, 300);
    match cut_out_box(&p, IRect::new(2, 2, 60, 50), &|_| true) {
        Err(BoxCutRefusal::NothingFound) => {}
        Ok(cut) => {
            let kept = cut.keep.data.iter().filter(|&&k| k >= 128).count();
            assert!(kept <= 58 * 48, "kept {kept}");
            assert!(cut.keep.data.iter().zip(&truth).all(|(&k, &t)| k == 0 || !t), "nothing of the ball outside the box");
        }
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn k_s1_progress_rises_to_100() {
    let (p, _, b) = scene(Scene::StarOnCarpet, 1200, 900);
    let seen = std::cell::RefCell::new(Vec::new());
    cut_out_box(&p, b, &|pc| {
        seen.borrow_mut().push(pc);
        true
    })
    .unwrap();
    let seen = seen.into_inner();
    assert_eq!(seen.first(), Some(&0));
    assert_eq!(seen.last(), Some(&100));
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert!(seen.len() >= 5, "{seen:?}");
}

#[test]
fn k_s1_a_box_over_the_whole_picture_learns_the_background_from_a_ring() {
    let (p, truth, _) = scene(Scene::BallOnGrass, 400, 300);
    let cut = cut_out_box(&p, IRect::new(-20, -20, 500, 400), &|_| true).unwrap();
    let v = iou(&cut, &truth);
    eprintln!("whole-picture box: IoU {v:.4}");
    assert!(v >= 0.8, "IoU {v:.4}");
}

#[test]
fn k_s1_transparent_pixels_stay_background() {
    let (mut p, truth, b) = scene(Scene::BallOnGrass, 400, 300);
    // A transparent hole in the ball and a transparent strip of grass inside the box.
    for y in 140..160 {
        for x in 190..210 {
            p.set(x, y, [0, 0, 0, 0]);
        }
    }
    for y in b.y0.max(0) as u32..b.y1.min(300) as u32 {
        for x in b.x0.max(0) as u32..b.x0.max(0) as u32 + 4 {
            p.set(x, y, [0, 0, 0, 0]);
        }
    }
    let cut = cut_out_box(&p, b, &|_| true).unwrap();
    for y in 140..160u32 {
        for x in 190..210u32 {
            assert_eq!(cut.keep.data[(y * 400 + x) as usize], 0, "({x}, {y})");
        }
    }
    assert!(iou(&cut, &truth) >= 0.88);
}

#[test]
fn k_s1_an_opaque_picture_with_a_soft_edge_keeps_its_alpha_after_apply() {
    let (mut p, truth, b) = scene(Scene::MugOnWood, 640, 480);
    let cut = cut_out_box(&p, b, &|_| true).unwrap();
    let before = p.clone();
    cut.apply(&mut p, None);
    // Kept object pixels far from the edge are unchanged; cleared pixels are transparent.
    let mut unchanged = 0;
    for (i, &t) in truth.iter().enumerate() {
        if cut.keep.data[i] == 255 {
            assert_eq!(&p.data[i * 4..i * 4 + 4], &before.data[i * 4..i * 4 + 4]);
            unchanged += t as usize;
        }
        if cut.keep.data[i] == 0 {
            assert_eq!(p.data[i * 4 + 3], 0);
        }
    }
    assert!(unchanged > 10_000);
    // Into a mask: the mask is `keep`, the pixels untouched.
    let mut mask = crate::pixel::AlphaBuf::filled(640, 480, 255);
    cut.apply_to_mask(&mut mask, None);
    assert_eq!(mask, cut.keep);
}

// ---------------------------------------------------------------------------------------------------------------
// The graph cut against brute force
// ---------------------------------------------------------------------------------------------------------------

/// The energy of a labelling of a small grid (all pixels free, opaque).
fn energy(w: usize, h: usize, px: &[[f32; 3]], costs: &[(f32, f32)], beta: f32, labels: &[bool]) -> f64 {
    let mut e = 0.0f64;
    for (i, &(cf, cb)) in costs.iter().enumerate() {
        e += if labels[i] { cf } else { cb } as f64;
    }
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            for (dx, dy, len) in NB {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || nx as usize >= w || ny as usize >= h {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if labels[i] != labels[j] {
                    e += (GAMMA * (-beta * d2(px[i], px[j])).exp() / len) as f64;
                }
            }
        }
    }
    e
}

#[test]
fn k_s1_the_grid_cut_finds_the_brute_force_minimum() {
    let mut rng = XorShift::new(77);
    for case in 0..40 {
        let (w, h) = if case % 2 == 0 { (4usize, 3usize) } else { (3, 4) };
        let n = w * h;
        let px: Vec<[f32; 3]> = (0..n).map(|_| [rng.next_f32(), rng.next_f32(), rng.next_f32()]).collect();
        let costs: Vec<(f32, f32)> = (0..n).map(|_| ((rng.next_f32() - 0.3) * 120.0, (rng.next_f32() - 0.3) * 120.0)).collect();
        let beta = rng.next_f32() * 8.0;
        let nodes: Vec<usize> = (0..n).collect();
        let index: Vec<u32> = (0..n as u32).collect();
        let labels = vec![false; n];
        let color = |i: usize| Some(px[i]);
        let cut = grid_cut(w, h, &nodes, &index, &labels, &color, &costs, beta);
        let got = energy(w, h, &px, &costs, beta, &cut);
        let best = (0u32..1 << n)
            .map(|m| {
                let l: Vec<bool> = (0..n).map(|k| m >> k & 1 == 1).collect();
                energy(w, h, &px, &costs, beta, &l)
            })
            .fold(f64::INFINITY, f64::min);
        assert!((got - best).abs() <= 1e-2 * best.abs().max(1.0), "case {case}: cut {got}, minimum {best}");
    }
}

#[test]
fn k_s1_fixed_neighbours_fold_into_the_terminals() {
    // A 3 x 1 grid: the middle pixel free, its left neighbour fixed object, its right neighbour fixed background, the
    // same colour everywhere (a link of GAMMA each side). With equal data costs the tie goes to the background (a node
    // reached from neither tree is on the sink side); a small preference decides.
    let px = [[0.5f32; 3]; 3];
    let color = |i: usize| Some(px[i]);
    let index = vec![NONE, 0, NONE];
    let labels = vec![true, false, false];
    let cut = grid_cut(3, 1, &[1], &index, &labels, &color, &[(1.0, 2.0)], 1.0);
    assert_eq!(cut, vec![true], "cheaper as object");
    let cut = grid_cut(3, 1, &[1], &index, &labels, &color, &[(2.0, 1.0)], 1.0);
    assert_eq!(cut, vec![false], "cheaper as background");
}

#[test]
fn k_s1_colour_models_prefer_their_own_colours() {
    let mut rng = XorShift::new(5);
    let red: Vec<[f32; 3]> = (0..2000).map(|_| [0.8 + rng.next_f32() * 0.1, 0.1 + rng.next_f32() * 0.1, 0.1 + rng.next_f32() * 0.05]).collect();
    let green: Vec<[f32; 3]> = (0..2000).map(|_| [0.1 + rng.next_f32() * 0.1, 0.7 + rng.next_f32() * 0.2, 0.2 + rng.next_f32() * 0.1]).collect();
    let fr = Gmm::fit(&red, COMPONENTS).unwrap();
    let fg = Gmm::fit(&green, COMPONENTS).unwrap();
    assert!(fr.comps.len() <= COMPONENTS && !fr.comps.is_empty());
    assert!(fr.neg_log([0.85, 0.15, 0.12]) < fg.neg_log([0.85, 0.15, 0.12]));
    assert!(fg.neg_log([0.15, 0.8, 0.25]) < fr.neg_log([0.15, 0.8, 0.25]));
    // A refit keeps the number of components and is deterministic.
    let again = fr.refit(&red).unwrap();
    assert_eq!(again.comps.len(), fr.comps.len());
    assert_eq!(format!("{:?}", fr.refit(&red).unwrap()), format!("{again:?}"));
    // One colour: a single component, finite costs.
    let flat = vec![[0.5f32, 0.5, 0.5]; 100];
    let f = Gmm::fit(&flat, COMPONENTS).unwrap();
    assert_eq!(f.comps.len(), 1);
    assert!(f.neg_log([0.5, 0.5, 0.5]).is_finite() && f.neg_log([0.0, 1.0, 0.0]).is_finite());
}

// ---------------------------------------------------------------------------------------------------------------
// S1 check: the Boykov-Kolmogorov max-flow against an independent max-flow on larger graphs
// ---------------------------------------------------------------------------------------------------------------

/// Dinic's max-flow in `f64` on a graph given as terminal capacities (`> 0` from the source, `< 0` to the sink, as
/// `Graph::tr_cap`) and arcs `(i, j, cap, rev_cap)`.
fn dinic_value(n: usize, tr: &[f32], arcs: &[(usize, usize, f32, f32)]) -> f64 {
    let (s, t) = (n, n + 1);
    let mut head: Vec<usize> = Vec::new();
    let mut cap: Vec<f64> = Vec::new();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n + 2];
    let add = |head: &mut Vec<usize>, cap: &mut Vec<f64>, adj: &mut Vec<Vec<usize>>, a: usize, b: usize, c: f64, rc: f64| {
        adj[a].push(head.len());
        head.push(b);
        cap.push(c);
        adj[b].push(head.len());
        head.push(a);
        cap.push(rc);
    };
    for (i, &v) in tr.iter().enumerate() {
        if v > 0.0 {
            add(&mut head, &mut cap, &mut adj, s, i, v as f64, 0.0);
        } else if v < 0.0 {
            add(&mut head, &mut cap, &mut adj, i, t, -v as f64, 0.0);
        }
    }
    for &(i, j, c, rc) in arcs {
        add(&mut head, &mut cap, &mut adj, i, j, c as f64, rc as f64);
    }
    let mut flow = 0.0f64;
    loop {
        let mut level = vec![usize::MAX; n + 2];
        level[s] = 0;
        let mut q = std::collections::VecDeque::from([s]);
        while let Some(u) = q.pop_front() {
            for &e in &adj[u] {
                if cap[e] > 1e-12 && level[head[e]] == usize::MAX {
                    level[head[e]] = level[u] + 1;
                    q.push_back(head[e]);
                }
            }
        }
        if level[t] == usize::MAX {
            return flow;
        }
        let mut it = vec![0usize; n + 2];
        #[allow(clippy::too_many_arguments)]
        fn dfs(u: usize, t: usize, f: f64, head: &[usize], cap: &mut [f64], adj: &[Vec<usize>], level: &[usize], it: &mut [usize]) -> f64 {
            if u == t {
                return f;
            }
            while it[u] < adj[u].len() {
                let e = adj[u][it[u]];
                let v = head[e];
                if cap[e] > 1e-12 && level[v] == level[u] + 1 {
                    let d = dfs(v, t, f.min(cap[e]), head, cap, adj, level, it);
                    if d > 0.0 {
                        cap[e] -= d;
                        cap[e ^ 1] += d;
                        return d;
                    }
                }
                it[u] += 1;
            }
            0.0
        }
        loop {
            let f = dfs(s, t, f64::INFINITY, &head, &mut cap, &adj, &level, &mut it);
            if f <= 0.0 {
                break;
            }
            flow += f;
        }
    }
}

#[test]
fn check_s1_the_max_flow_equals_an_independent_max_flow_on_larger_graphs() {
    // Random 8-neighbour grids of up to 30 x 30 with asymmetric arc capacities (some near zero), terminal capacities of
    // both signs (some zero), and a few long-range arcs: the cut the BK solver labels must cost the max-flow value
    // computed by Dinic's algorithm (min-cut = max-flow), i.e. the labelling is a true minimum cut.
    let mut rng = XorShift::new(2026);
    for case in 0..60 {
        let (w, h) = (5 + (rng.next_u32() % 26) as usize, 5 + (rng.next_u32() % 26) as usize);
        let n = w * h;
        let mut tr = vec![0.0f32; n];
        let mut g = Graph::with_capacity(n, n * 5);
        for (i, v) in tr.iter_mut().enumerate() {
            let r = rng.next_f32();
            let (cs, ct) = if r < 0.2 { (0.0, 0.0) } else { ((rng.next_f32() * 200.0).max(0.0), (rng.next_f32() * 200.0).max(0.0)) };
            g.add_tweights(i, cs, ct);
            *v = g.tr_cap[i];
        }
        let mut arcs = Vec::new();
        for y in 0..h {
            for x in 0..w {
                for (dx, dy, _) in NB {
                    let (nx, ny) = (x as i32 + dx, y + dy as usize);
                    if nx < 0 || nx as usize >= w || ny >= h {
                        continue;
                    }
                    let (i, j) = (y * w + x, ny * w + nx as usize);
                    let scale = if rng.next_f32() < 0.1 { 1e-4 } else { 60.0 };
                    let (c, rc) = (rng.next_f32() * scale, rng.next_f32() * scale);
                    g.add_edge(i, j, c, rc);
                    arcs.push((i, j, c, rc));
                }
            }
        }
        for _ in 0..n / 10 {
            let (i, j) = ((rng.next_u32() as usize) % n, (rng.next_u32() as usize) % n);
            if i != j {
                let (c, rc) = (rng.next_f32() * 30.0, rng.next_f32() * 30.0);
                g.add_edge(i, j, c, rc);
                arcs.push((i, j, c, rc));
            }
        }
        g.maxflow();
        let side: Vec<bool> = (0..n).map(|i| g.in_source(i)).collect();
        let mut cut = 0.0f64;
        for (i, &v) in tr.iter().enumerate() {
            if side[i] && v < 0.0 {
                cut += -v as f64;
            }
            if !side[i] && v > 0.0 {
                cut += v as f64;
            }
        }
        for &(i, j, c, rc) in &arcs {
            if side[i] && !side[j] {
                cut += c as f64;
            }
            if side[j] && !side[i] {
                cut += rc as f64;
            }
        }
        let best = dinic_value(n, &tr, &arcs);
        assert!((cut - best).abs() <= 1e-3 * best.max(1.0), "case {case} ({w} x {h}): BK cut {cut}, max-flow {best}");
    }
}
