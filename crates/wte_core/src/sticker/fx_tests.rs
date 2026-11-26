//! Tests of `sticker::fx` (STICKER_PLAN 7 "P1"): the styles, the distance helpers and the tent blur.

use crate::pixel::PixelBuf;
use crate::pixel_ops::{PremulMips, premultiply, unpremultiply};
use crate::sticker::fx::*;
use crate::sticker::style::*;
use crate::testutil::XorShift;

/// A disc of radius `r` centred at `(cx, cy)` (continuous pixel coordinates), anti-aliased by the centre distance,
/// in colour `rgb`.
fn disc(w: u32, h: u32, cx: f32, cy: f32, r: f32, rgb: [u8; 3]) -> PixelBuf {
    let mut b = PixelBuf::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let a = ((r + 0.5 - d).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            if a > 0 {
                b.set(x, y, [rgb[0], rgb[1], rgb[2], a]);
            }
        }
    }
    b
}

fn contour(width: f32, smooth: f32, fill_holes: bool) -> Contour {
    Contour { on: true, width, smooth, fill_holes, color: [255, 255, 255, 255], opacity: 1.0 }
}

fn stroke(width: f32, position: StrokePos, color: [u8; 4]) -> Stroke {
    Stroke { on: true, width, position, color, opacity: 1.0 }
}

fn shadow(angle_deg: f32, distance: f32, spread: f32, size: f32) -> Shadow {
    Shadow { on: true, color: [0, 0, 0, 255], opacity: 1.0, angle_deg, distance, spread, size }
}

fn glow(spread: f32, size: f32, color: [u8; 4]) -> Glow {
    Glow { on: true, color, opacity: 1.0, spread, size }
}

fn plane_sum(p: &Plane) -> f64 {
    p.v.iter().map(|&v| v as f64 / 255.0).sum()
}

#[test]
fn k1_contour_of_a_disc_is_a_disc_ten_pixels_wider() {
    let base = disc(100, 100, 50.0, 50.0, 20.0, [200, 0, 0]);
    let a = alpha_plane(&base, 0, false);
    let m = contour_plane(&a, &contour(10.0, 0.0, false));
    let (mut worst, mut partial) = (0.0f32, 0);
    for y in 0..100 {
        for x in 0..100 {
            let r = ((x as f32 + 0.5 - 50.0).powi(2) + (y as f32 + 0.5 - 50.0).powi(2)).sqrt();
            let v = m.v[y * 100 + x];
            match v {
                255 => assert!(r <= 30.6, "full pixel at radius {r}"),
                0 => assert!(r >= 29.4, "empty pixel at radius {r}"),
                _ => {
                    // The edge crosses this pixel where the coverage says.
                    let edge = r + (v as f32 / 255.0 - 0.5);
                    worst = worst.max((edge - 30.0).abs());
                    partial += 1;
                }
            }
        }
    }
    assert!(partial > 100, "an anti-aliased edge: {partial} partial pixels");
    assert!(worst <= 0.6, "edge radius off by {worst}");
    let area = plane_sum(&m);
    let radius = (area / std::f64::consts::PI).sqrt();
    assert!((radius - 30.0).abs() < 0.6, "area radius {radius}");
}

#[test]
fn k1_contour_fills_the_hole_of_a_ring() {
    let mut ring = disc(80, 80, 40.0, 40.0, 25.0, [0, 0, 255]);
    let hole = disc(80, 80, 40.0, 40.0, 12.0, [0, 0, 0]);
    for (p, h) in ring.data.as_chunks_mut::<4>().0.iter_mut().zip(hole.data.as_chunks::<4>().0.iter()) {
        p[3] = ((p[3] as u32 * (255 - h[3] as u32) + 127) / 255) as u8;
    }
    let a = alpha_plane(&ring, 4, false);
    let filled = contour_plane(&a, &contour(3.0, 0.0, true));
    let open = contour_plane(&a, &contour(3.0, 0.0, false));
    let centre = 44 * filled.w + 44;
    assert_eq!(filled.v[centre], 255);
    assert_eq!(open.v[centre], 0);
    // Filling changes only the hole.
    for (i, (&f, &o)) in filled.v.iter().zip(&open.v).enumerate() {
        let (x, y) = ((i % filled.w) as f32 - 44.0 + 0.5, (i / filled.w) as f32 - 44.0 + 0.5);
        if (x * x + y * y).sqrt() > 12.0 {
            assert_eq!(f, o, "pixel {i} outside the hole changed");
        }
    }
}

#[test]
fn k1_smoothing_closes_a_notch_and_never_lowers_alpha() {
    // A 60 x 60 square with a notch 23 px wide (= 3 + 2 * 10) and 30 deep cut from the top: a contour of width 10
    // leaves a 3-px notch in the die-cut, which smoothing closes.
    let mut base = PixelBuf::new(100, 100);
    for y in 20..80u32 {
        for x in 20..80u32 {
            if !((39..62).contains(&x) && y < 50) {
                base.set(x, y, [10, 120, 30, 255]);
            }
        }
    }
    let a = alpha_plane(&base, 0, false);
    let sharp = contour_plane(&a, &contour(10.0, 0.0, false));
    let smooth = contour_plane(&a, &contour(10.0, 1.0, false));
    let at = |p: &Plane, x: usize, y: usize| p.v[y * p.w + x];
    for y in [25, 30, 35] {
        assert_eq!(at(&sharp, 50, y), 0, "the sharp contour has the notch at y {y}");
        assert!(at(&smooth, 50, y) >= 128, "smoothing closed the notch at y {y}: {}", at(&smooth, 50, y));
    }
    for (m, av) in smooth.v.iter().zip(&a.v) {
        assert!(m >= av, "the contour never cuts into the artwork");
    }
    // And through apply_styles: the output alpha is never below the base alpha at the base's place.
    let st = StyleStack { contour: Some(contour(10.0, 1.0, false)), ..StyleStack::default() };
    let out = apply_styles(&base, &st, 1.0);
    let m = st.margin(1.0);
    for y in 0..100u32 {
        for x in 0..100u32 {
            assert!(out.get(x + m, y + m)[3] >= base.get(x, y)[3]);
        }
    }
}

#[test]
fn k1_stroke_areas_on_a_square() {
    // An opaque 40 x 40 picture, padded by 10.
    let base = PixelBuf::filled(40, 40, [0, 0, 255, 255]);
    let a = alpha_plane(&base, 10, false);
    let inv = alpha_plane(&base, 10, true);
    let d_in = inv.dist();
    let pi = std::f64::consts::PI;
    let check = |got: f64, want: f64, what: &str| {
        assert!((got - want).abs() / want < 0.01, "{what}: {got} vs {want}");
    };
    // Outside, width 6: four sides plus round corners. Inside: a square ring. Center: half of each.
    check(plane_sum(&outside_stroke_plane(&a, 6.0)), 160.0 * 6.0 + pi * 36.0, "outside");
    check(plane_sum(&inside_stroke_plane(&inv, &d_in, 6.0)), 1600.0 - 28.0 * 28.0, "inside");
    check(plane_sum(&outside_stroke_plane(&a, 3.0)), 160.0 * 3.0 + pi * 9.0, "center, outside part");
    check(plane_sum(&inside_stroke_plane(&inv, &d_in, 3.0)), 1600.0 - 34.0 * 34.0, "center, inside part");
}

#[test]
fn k1_an_outside_stroke_runs_around_the_contour() {
    let base = disc(60, 60, 30.0, 30.0, 20.0, [255, 0, 0]);
    let st = StyleStack {
        contour: Some(contour(6.0, 0.0, false)),
        stroke: Some(stroke(3.0, StrokePos::Outside, [0, 0, 0, 255])),
        ..StyleStack::default()
    };
    let m = st.margin(1.0);
    assert_eq!(m, 6 + 3 + 2);
    let out = apply_styles(&base, &st, 1.0);
    assert_eq!([out.width, out.height], [60 + 2 * m, 60 + 2 * m]);
    let c = 30.0 + m as f32;
    let px = |r: f32| out.get((c + r) as u32, c as u32);
    assert_eq!(px(10.0), [255, 0, 0, 255], "artwork");
    assert_eq!(px(23.0), [255, 255, 255, 255], "contour");
    assert_eq!(px(27.5), [0, 0, 0, 255], "stroke outside the contour");
    assert_eq!(px(31.0)[3], 0, "nothing beyond the stroke");
}

#[test]
fn k1_drop_shadow_offset_follows_the_light() {
    let base = PixelBuf::filled(10, 10, [255, 255, 255, 255]);
    let a = alpha_plane(&base, 20, false);
    let d = a.dist();
    let centroid = |p: &Plane| {
        let (mut sx, mut sy, mut s) = (0.0f64, 0.0f64, 0.0f64);
        for (i, &v) in p.v.iter().enumerate() {
            let v = v as f64;
            sx += v * (i % p.w) as f64;
            sy += v * (i / p.w) as f64;
            s += v;
        }
        (sx / s, sy / s)
    };
    let (ax, ay) = centroid(&a);
    for (angle, want) in [(0.0, (-8.0, 0.0)), (90.0, (0.0, 8.0)), (120.0, (4.0, 7.0)), (270.0, (0.0, -8.0))] {
        let sp = drop_shadow_plane(&a, Some(&d), &shadow(angle, 8.0, 0.0, 0.0));
        let (sx, sy) = centroid(&sp);
        assert!((sx - ax - want.0).abs() < 1e-9 && (sy - ay - want.1).abs() < 1e-9, "{angle}: ({}, {})", sx - ax, sy - ay);
        // Blurred, the centroid stays where the offset put it.
        let soft = drop_shadow_plane(&a, Some(&d), &shadow(angle, 8.0, 0.0, 5.0));
        let (bx, by) = centroid(&soft);
        assert!((bx - sx).abs() < 0.05 && (by - sy).abs() < 0.05, "{angle}: blurred ({bx}, {by}) vs ({sx}, {sy})");
    }
}

#[test]
fn k1_spread_one_is_hard_spread_zero_is_a_tent() {
    // Left half opaque: a straight vertical edge between columns 49 and 50.
    let mut base = PixelBuf::new(100, 40);
    for y in 0..40 {
        for x in 0..50 {
            base.set(x, y, [0, 0, 0, 255]);
        }
    }
    let pad = 14usize;
    let a = alpha_plane(&base, pad, false);
    let d = a.dist();
    let row = |p: &Plane| -> Vec<u8> { (0..p.w).map(|x| p.v[(20 + pad) * p.w + x]).collect() };
    let hard = row(&glow_plane(&a, Some(&d), &glow(1.0, 10.0, [255; 4])));
    for x in 0..100 {
        let want = if x < 60 { 255 } else { 0 };
        assert_eq!(hard[x + pad], want, "spread 1 at column {x}");
    }
    let soft = row(&glow_plane(&a, Some(&d), &glow(0.0, 10.0, [255; 4])));
    for x in 30..80 {
        assert!(soft[x + pad] >= soft[x + pad + 1], "monotone at column {x}");
    }
    for k in 0..12 {
        let (inside, outside) = (soft[49 - k + pad] as i32, soft[50 + k + pad] as i32);
        assert!((inside + outside - 255).abs() <= 2, "symmetric at {k}: {inside} + {outside}");
    }
    assert!(soft[50 + pad] > 100 && soft[50 + pad] < 155, "half way at the edge: {}", soft[50 + pad]);
    assert_eq!(soft[40 + pad], 255);
    assert_eq!(soft[59 + pad], 0);
}

/// A random style stack with lengths up to `max_len`.
fn random_stack(rng: &mut XorShift, max_len: f32) -> StyleStack {
    let mut len = || rng.next_f32() * max_len;
    let (l1, l2, l3, l4, l5, l6, l7) = (len(), len(), len(), len(), len(), len(), len());
    let mut pick = || rng.next_f32();
    let on = |v: f32| v < 0.55;
    let (p1, p2, p3, p4, p5, p6, p7, p8, p9) = (pick(), pick(), pick(), pick(), pick(), pick(), pick(), pick(), pick());
    let (q1, q2, q3, q4) = (pick(), pick(), pick(), pick());
    let position = [StrokePos::Outside, StrokePos::Inside, StrokePos::Center][(q1 * 2.999) as usize];
    StyleStack {
        contour: on(p1).then_some(Contour { on: true, width: l1, smooth: q2, fill_holes: q3 < 0.5, color: [255; 4], opacity: 1.0 }),
        stroke: on(p2).then_some(Stroke { on: q4 < 0.9, width: l2, position, color: [0, 0, 0, 255], opacity: 0.8 }),
        drop_shadow: on(p3).then(|| shadow(q1 * 360.0, l3, q2, l4)),
        inner_shadow: on(p4).then(|| shadow(q3 * 360.0, l5, q4, l6)),
        outer_glow: on(p5).then(|| glow(q3, l7, [255, 200, 0, 255])),
        inner_glow: on(p6).then(|| glow(q2, l1, [0, 255, 0, 200])),
        color_overlay: on(p7).then_some(ColorOverlay { on: true, color: [9, 9, 200, 255], opacity: q4 }),
        gradient_overlay: on(p8).then(|| GradientOverlay {
            on: true,
            gradient: Gradient::two([255, 0, 0, 255], [0, 0, 255, 128]),
            angle_deg: q1 * 360.0,
            opacity: q3,
        }),
        bevel: on(p9).then_some(Bevel {
            on: true,
            size: l2,
            depth: q2 * 4.0,
            angle_deg: 120.0,
            altitude_deg: 30.0,
            highlight: [255; 4],
            shadow: [0, 0, 0, 255],
            opacity: 0.7,
        }),
    }
}

#[test]
fn k1_the_margin_is_enough_for_200_random_stacks() {
    let mut rng = XorShift::new(2025);
    let mut drawn_outside = 0;
    for case in 0..200 {
        let st = random_stack(&mut rng, 16.0);
        let (w, h) = (16 + rng.next_u32() % 24, 16 + rng.next_u32() % 24);
        let base = match case % 4 {
            0 => PixelBuf::filled(w, h, [120, 60, 30, 255]),
            1 => disc(w, h, w as f32 * 0.5, h as f32 * 0.5, w.min(h) as f32 * 0.4, [200, 10, 10]),
            2 => disc(w, h, 2.0, 3.0, 9.0, [10, 200, 10]),
            _ => {
                let mut b = PixelBuf::new(w, h);
                for _ in 0..12 {
                    let (x, y) = (rng.next_u32() % w, rng.next_u32() % h);
                    b.set(x, y, [1, 2, 3, (rng.next_u32() % 256) as u8]);
                }
                b
            }
        };
        let scale = [0.5f32, 1.0, 1.5][case % 3];
        let m = st.margin(scale);
        let out = apply_styles(&base, &st, scale);
        assert_eq!([out.width, out.height], [w + 2 * m, h + 2 * m], "case {case}");
        if m == 0 {
            continue;
        }
        drawn_outside += 1;
        let (ow, oh) = (out.width, out.height);
        for y in 0..oh {
            for x in 0..ow {
                if x == 0 || y == 0 || x == ow - 1 || y == oh - 1 {
                    assert_eq!(out.get(x, y), [0; 4], "case {case}: border pixel ({x}, {y}) of {ow}x{oh}, margin {m}, {st:?}");
                }
            }
        }
    }
    assert!(drawn_outside > 100, "most stacks draw outside: {drawn_outside}");
}

/// Turns every present entry off (the numbers stay).
fn switch_all_off(s: &mut StyleStack) {
    if let Some(c) = &mut s.contour {
        c.on = false;
    }
    if let Some(c) = &mut s.stroke {
        c.on = false;
    }
    for c in [&mut s.drop_shadow, &mut s.inner_shadow].into_iter().flatten() {
        c.on = false;
    }
    for c in [&mut s.outer_glow, &mut s.inner_glow].into_iter().flatten() {
        c.on = false;
    }
    if let Some(c) = &mut s.color_overlay {
        c.on = false;
    }
    if let Some(c) = &mut s.gradient_overlay {
        c.on = false;
    }
    if let Some(c) = &mut s.bevel {
        c.on = false;
    }
}

#[test]
fn k1_no_style_on_returns_the_base_unchanged() {
    let mut rng = XorShift::new(4);
    let base = PixelBuf::from_raw(23, 17, (0..23 * 17 * 4).map(|_| (rng.next_u32() % 256) as u8).collect()).unwrap();
    assert_eq!(apply_styles(&base, &StyleStack::default(), 1.0), base);
    // Every kind of entry present, all switched off.
    let mut off = StyleStack {
        contour: Some(contour(5.0, 0.5, true)),
        stroke: Some(stroke(2.0, StrokePos::Outside, [0; 4])),
        drop_shadow: Some(shadow(30.0, 4.0, 0.0, 4.0)),
        inner_shadow: Some(shadow(30.0, 4.0, 0.0, 4.0)),
        outer_glow: Some(glow(0.0, 5.0, [9; 4])),
        inner_glow: Some(glow(0.0, 5.0, [9; 4])),
        color_overlay: Some(ColorOverlay { on: true, color: [1; 4], opacity: 1.0 }),
        gradient_overlay: Some(GradientOverlay {
            on: true,
            gradient: Gradient::two([0; 4], [255; 4]),
            angle_deg: 0.0,
            opacity: 1.0,
        }),
        bevel: StylePreset::Badge.styles([64, 64]).bevel,
    };
    switch_all_off(&mut off);
    assert_eq!(off.margin(1.0), 0);
    assert_eq!(apply_styles(&base, &off, 1.0), base);
    // An empty or fully transparent base gives a transparent picture of the output size.
    let st = StylePreset::ClassicSticker.styles([64, 64]);
    let m = st.margin(1.0);
    assert_eq!(apply_styles(&PixelBuf::new(10, 6), &st, 1.0), PixelBuf::new(10 + 2 * m, 6 + 2 * m));
    assert_eq!(apply_styles(&PixelBuf::new(0, 0), &st, 1.0), PixelBuf::new(2 * m, 2 * m));
    // Degenerate inputs do not panic: a 1 x 1 picture, a huge or invalid scale.
    let one = PixelBuf::filled(1, 1, [1, 2, 3, 255]);
    let all = random_stack(&mut XorShift::new(99), 8.0);
    for s in [1.0f32, 0.0, -1.0, f32::NAN, f32::INFINITY, 1.0e-6] {
        let out = apply_styles(&one, &StylePreset::Badge.styles([1, 1]), s);
        assert!(out.width >= 1 && out.data.len() == (out.width * out.height * 4) as usize);
        let _ = apply_styles(&one, &all, s);
    }
}

#[test]
fn k1_inside_styles_work_on_a_fully_opaque_picture() {
    let blue = [0, 0, 255, 255];
    let base = PixelBuf::filled(64, 64, blue);
    let ring = |x: u32, y: u32| x.min(y).min(63 - x).min(63 - y);
    // An Inside stroke of width 4 colours exactly the 4-px border ring.
    let st = StyleStack { stroke: Some(stroke(4.0, StrokePos::Inside, [255, 0, 0, 255])), ..StyleStack::default() };
    let out = apply_styles(&base, &st, 1.0);
    assert_eq!([out.width, out.height], [64, 64]);
    for y in 0..64 {
        for x in 0..64 {
            match ring(x, y) {
                0..=2 => assert_eq!(out.get(x, y), [255, 0, 0, 255], "({x}, {y})"),
                3 | 4 => {}
                _ => assert_eq!(out.get(x, y), blue, "({x}, {y})"),
            }
        }
    }
    assert_eq!(out.get(3, 30), [255, 0, 0, 255], "the fourth pixel of the ring");
    assert_eq!(out.get(5, 30), blue);
    // Inner glow and bevel change every border pixel; an inner shadow (light from 120 degrees) every pixel of the
    // lit sides' borders. The size stays 64 x 64.
    let glow_st = StyleStack { inner_glow: Some(glow(0.0, 6.0, [255, 255, 0, 255])), ..StyleStack::default() };
    let bevel_st = StyleStack {
        bevel: Some(Bevel {
            on: true,
            size: 6.0,
            depth: 1.0,
            angle_deg: 120.0,
            altitude_deg: 30.0,
            highlight: [255, 255, 255, 255],
            shadow: [0, 0, 0, 255],
            opacity: 1.0,
        }),
        ..StyleStack::default()
    };
    let shadow_st = StyleStack { inner_shadow: Some(shadow(120.0, 2.0, 0.0, 6.0)), ..StyleStack::default() };
    for (name, st, all_sides) in [("inner glow", glow_st, true), ("bevel", bevel_st, true), ("inner shadow", shadow_st, false)] {
        let out = apply_styles(&base, &st, 1.0);
        assert_eq!([out.width, out.height], [64, 64], "{name}");
        for y in 0..64 {
            for x in 0..64 {
                let lit = x == 0 || y == 0;
                if ring(x, y) == 0 && (all_sides || lit) {
                    assert_ne!(out.get(x, y), blue, "{name}: border pixel ({x}, {y}) unchanged");
                }
            }
        }
        assert_eq!(out.get(32, 32), blue, "{name}: the middle stays");
    }
}

#[test]
fn k1_overlays_keep_alpha_and_gradient_endpoints_are_exact() {
    let base = disc(40, 40, 20.0, 20.0, 15.0, [255, 0, 0]);
    let st = StyleStack {
        color_overlay: Some(ColorOverlay { on: true, color: [0, 255, 0, 255], opacity: 1.0 }),
        ..StyleStack::default()
    };
    assert_eq!(st.margin(1.0), 0);
    let out = apply_styles(&base, &st, 1.0);
    for (o, b) in out.data.as_chunks::<4>().0.iter().zip(base.data.as_chunks::<4>().0.iter()) {
        assert_eq!(o[3], b[3], "alpha kept");
        if b[3] > 0 {
            assert_eq!(&o[..3], &[0, 255, 0]);
        }
    }
    let half = StyleStack {
        color_overlay: Some(ColorOverlay { on: true, color: [0, 255, 0, 255], opacity: 0.5 }),
        ..StyleStack::default()
    };
    let out = apply_styles(&PixelBuf::filled(4, 4, [255, 0, 0, 255]), &half, 1.0);
    assert_eq!(out.get(1, 1), [128, 128, 0, 255]);
    // Gradient overlay across an opaque 16 x 8 box, angle 0: the first column is the first stop, the last the last.
    let g = StyleStack {
        gradient_overlay: Some(GradientOverlay {
            on: true,
            gradient: Gradient::two([10, 20, 30, 255], [250, 240, 230, 255]),
            angle_deg: 0.0,
            opacity: 1.0,
        }),
        ..StyleStack::default()
    };
    let out = apply_styles(&PixelBuf::filled(16, 8, [0, 0, 0, 255]), &g, 1.0);
    for y in 0..8 {
        assert_eq!(out.get(0, y), [10, 20, 30, 255]);
        assert_eq!(out.get(15, y), [250, 240, 230, 255]);
    }
    // Angle 90: the first stop at the bottom.
    let mut g90 = g.clone();
    g90.gradient_overlay.as_mut().unwrap().angle_deg = 90.0;
    let out = apply_styles(&PixelBuf::filled(16, 8, [0, 0, 0, 255]), &g90, 1.0);
    assert_eq!(out.get(5, 7), [10, 20, 30, 255]);
    assert_eq!(out.get(5, 0), [250, 240, 230, 255]);
}

#[test]
fn k1_a_bevel_leaves_a_flat_interior_alone() {
    let mut rng = XorShift::new(12);
    let mut base = PixelBuf::new(64, 64);
    for p in base.data.as_chunks_mut::<4>().0.iter_mut() {
        p.copy_from_slice(&[(rng.next_u32() % 256) as u8, (rng.next_u32() % 256) as u8, 77, 255]);
    }
    let st = StyleStack {
        bevel: Some(Bevel {
            on: true,
            size: 4.0,
            depth: 3.0,
            angle_deg: 45.0,
            altitude_deg: 40.0,
            highlight: [255; 4],
            shadow: [0, 0, 0, 255],
            opacity: 1.0,
        }),
        ..StyleStack::default()
    };
    let out = apply_styles(&base, &st, 1.0);
    let mut changed = 0;
    for y in 0..64u32 {
        for x in 0..64u32 {
            let k = x.min(y).min(63 - x).min(63 - y);
            if k >= 8 {
                assert_eq!(out.get(x, y), base.get(x, y), "({x}, {y})");
            } else if out.get(x, y) != base.get(x, y) {
                changed += 1;
            }
        }
    }
    assert!(changed > 200, "the bevel draws near the edge: {changed}");
}

#[test]
fn k1_tent_blur_keeps_the_sum_and_is_symmetric() {
    let (w, h) = (200u32, 160u32);
    let mut rng = XorShift::new(21);
    let mut v8 = vec![0u8; (w * h) as usize];
    for y in 50..110 {
        for x in 60..140 {
            v8[(y * w + x) as usize] = (rng.next_u32() % 256) as u8;
        }
    }
    let mut v16: Vec<u16> = v8.iter().map(|&v| v as u16 * 257).collect();
    let (s8, s16) = (v8.iter().map(|&v| v as f64).sum::<f64>(), v16.iter().map(|&v| v as f64).sum::<f64>());
    tent(&mut v8, w, h, 9);
    tent(&mut v16, w, h, 10);
    let (t8, t16) = (v8.iter().map(|&v| v as f64).sum::<f64>(), v16.iter().map(|&v| v as f64).sum::<f64>());
    assert!((t8 - s8).abs() / s8 < 0.002, "u8 sum {s8} -> {t8}");
    assert!((t16 - s16).abs() / s16 < 0.0002, "u16 sum {s16} -> {t16}");
    // An impulse spreads symmetrically, even for an even width.
    for b in [4u32, 7] {
        let mut imp = vec![0u16; 41 * 41];
        imp[20 * 41 + 20] = 60_000;
        tent(&mut imp, 41, 41, b);
        for y in 0..41 {
            for x in 0..41 {
                assert_eq!(imp[y * 41 + x], imp[y * 41 + (40 - x)], "b {b}: mirror x at ({x}, {y})");
                assert_eq!(imp[y * 41 + x], imp[(40 - y) * 41 + x], "b {b}: mirror y at ({x}, {y})");
            }
        }
        let reach = (0..41).filter(|&x| imp[20 * 41 + x] > 0).count();
        assert_eq!(reach as u32, 2 * b - 1, "b {b}: a tent is 2b - 1 wide");
    }
    // Width 1 changes nothing; a constant stays constant inside.
    let mut c = vec![200u8; 50 * 50];
    let copy = c.clone();
    tent(&mut c, 50, 50, 1);
    assert_eq!(c, copy);
    tent(&mut c, 50, 50, 5);
    assert_eq!(c[25 * 50 + 25], 200);
    assert!(c[0] < 200, "outside counts as 0");
}

#[test]
fn k1_apply_styles_is_thread_count_independent() {
    let base = disc(300, 220, 150.0, 110.0, 90.0, [180, 40, 90]);
    let mut st = random_stack(&mut XorShift::new(5), 12.0);
    st.contour = Some(contour(9.0, 0.6, true));
    st.stroke = Some(stroke(3.0, StrokePos::Center, [0, 0, 0, 255]));
    st.drop_shadow = Some(shadow(120.0, 6.0, 0.2, 9.0));
    st.inner_glow = Some(glow(0.1, 5.0, [255, 255, 255, 200]));
    st.bevel = StylePreset::Badge.styles([300, 220]).bevel;
    let pools = [1usize, 8].map(|n| rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap());
    let a = pools[0].install(|| apply_styles(&base, &st, 1.3));
    let b = pools[1].install(|| apply_styles(&base, &st, 1.3));
    assert_eq!(a, b);
    assert!(a.data.len() / 4 > 1 << 16, "large enough for the parallel paths");
}

#[test]
fn k1_half_working_scale_matches_the_reduced_full_result() {
    // A disc with a colour ramp; styled at full size, and its first mip level styled at scale 1/2.
    let mut base = disc(256, 256, 128.0, 128.0, 90.0, [0, 0, 0]);
    for y in 0..256u32 {
        for x in 0..256u32 {
            let mut p = base.get(x, y);
            if p[3] > 0 {
                p[0] = x as u8;
                p[1] = y as u8;
                p[2] = 255 - x as u8 / 2;
                base.set(x, y, p);
            }
        }
    }
    let st = StyleStack {
        contour: Some(contour(12.0, 0.5, true)),
        stroke: Some(stroke(4.0, StrokePos::Outside, [20, 20, 20, 255])),
        drop_shadow: Some(Shadow { opacity: 0.6, ..shadow(120.0, 6.0, 0.0, 10.0) }),
        outer_glow: Some(Glow { opacity: 0.5, ..glow(0.0, 8.0, [255, 230, 0, 255]) }),
        ..StyleStack::default()
    };
    let full = apply_styles(&base, &st, 1.0);
    let mips = PremulMips::build(&base);
    let lvl = &mips.levels[1];
    let mut half_base = PixelBuf::new(lvl.width, lvl.height);
    for (o, p) in half_base.data.as_chunks_mut::<4>().0.iter_mut().zip(lvl.data.as_chunks::<4>().0.iter()) {
        o.copy_from_slice(&unpremultiply([p[0], p[1], p[2], p[3]]));
    }
    let half = apply_styles(&half_base, &st, 0.5);
    let (m1, mh) = (st.margin(1.0) as i64, st.margin(0.5) as i64);
    let fetch = |x: i64, y: i64| -> [f64; 4] {
        if x < 0 || y < 0 || x >= full.width as i64 || y >= full.height as i64 {
            return [0.0; 4];
        }
        premultiply(full.get(x as u32, y as u32)).map(|v| v as f64)
    };
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for y in 0..half.height as i64 {
        for x in 0..half.width as i64 {
            let (fx, fy) = (m1 + 2 * (x - mh), m1 + 2 * (y - mh));
            let mut acc = [0.0f64; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = fetch(fx + dx, fy + dy);
                for c in 0..4 {
                    acc[c] += p[c] * 0.25;
                }
            }
            let h = premultiply(half.get(x as u32, y as u32));
            for c in 0..4 {
                sum += (acc[c] - h[c] as f64).abs();
            }
            n += 4;
        }
    }
    let mean = sum / n as f64;
    assert!(mean <= 3.0, "mean difference {mean:.3} / 255");
}

#[test]
fn k1_dist_helpers() {
    // dist_in of an opaque square inside a transparent frame: half a pixel at the edge, growing inwards.
    let mut a = vec![0u8; 12 * 12];
    for y in 1..11 {
        for x in 1..11 {
            a[y * 12 + x] = 255;
        }
    }
    let din = dist_in(&a, 12, 12);
    assert_eq!(din[12 + 5], 0.5);
    assert_eq!(din[2 * 12 + 5], 1.5);
    assert_eq!(din[0], 0.0);
    let dout = dist_out(&a, 12, 12);
    assert_eq!(dout[5], 0.5);
    assert_eq!(dout[5 * 12 + 5], 0.0);
    assert_eq!(tent_width(0.2), 1);
    assert_eq!(tent_width(2.5), 3);
    assert_eq!(tent_width(f32::NAN), 1);
}
