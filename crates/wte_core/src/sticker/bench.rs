//! Timings of STICKER_PLAN 8.4 for package P1 (ignored `perf_k1_*` tests, run serially in a release test build:
//! `cargo test -p wte_core --release --lib perf_k1 -- --ignored --test-threads=1 --nocapture`). Median of 5 runs after
//! one warm-up; peak working memory with the allocation counter of the release test build
//! (`text::styled::alloc_count`). A budget missed by less than 50 % is reported (and decided at the gate); more fails.

use std::time::Instant;

use crate::pixel::PixelBuf;
use crate::sticker::fx::apply_styles;
use crate::sticker::render::StickerRenderInput;
use crate::sticker::style::StylePreset;
use crate::text::edt::distance_to_ink;
use crate::testutil::XorShift;

fn median_ms(mut f: impl FnMut()) -> f64 {
    f();
    let mut t: Vec<f64> = (0..5)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[2]
}

/// A photo-like test picture: a disc filling most of the square, noisy colours inside.
fn photo(side: u32) -> PixelBuf {
    let mut rng = XorShift::new(side as u64);
    let mut b = PixelBuf::new(side, side);
    let (c, r) = (side as f32 * 0.5, side as f32 * 0.45);
    for y in 0..side {
        for x in 0..side {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            let a = ((r + 0.5 - d).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            if a > 0 {
                let n = (rng.next_u32() % 64) as u8;
                b.set(x, y, [(x % 256) as u8, (y % 256) as u8, 120 + n, a]);
            }
        }
    }
    b
}

/// Checks a time against its budget: prints the line, fails beyond 1.5 x.
fn report(what: &str, ms: f64, budget: f64) {
    let verdict = if ms <= budget {
        "ok"
    } else if ms <= budget * 1.5 {
        "MISSED by < 50 % (gate decides)"
    } else {
        "FAILED"
    };
    eprintln!("{what}: {ms:.1} ms (budget {budget} ms) {verdict}");
    assert!(ms <= budget * 1.5, "{what}: {ms:.1} ms is more than 1.5 x the budget of {budget} ms");
}

#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k1 -- --ignored --test-threads=1)"]
fn perf_k1_apply_styles_classic_sticker() {
    // The plan's row: ClassicSticker as defined (contour 12, smooth 0.5, shadow 40 %) on bases of 1024 to 4096.
    let classic = StylePreset::ClassicSticker.styles([512, 512]);
    for (side, budget) in [(1024u32, 60.0), (2048, 250.0), (4096, 1100.0)] {
        let base = photo(side);
        let ms = median_ms(|| {
            std::hint::black_box(apply_styles(&base, &classic, 1.0));
        });
        report(&format!("apply_styles ClassicSticker {side} x {side} (margin {})", classic.margin(1.0)), ms, budget);
    }
    // Reported only: the preset with its lengths scaled to the picture, as the Sticker tab applies it (contour 24 at
    // 1024, 48 at 2048).
    for side in [1024u32, 2048] {
        let base = photo(side);
        let st = StylePreset::ClassicSticker.styles([side, side]);
        let ms = median_ms(|| {
            std::hint::black_box(apply_styles(&base, &st, 1.0));
        });
        eprintln!("apply_styles ClassicSticker scaled to {side} x {side} (margin {}): {ms:.1} ms (no budget)", st.margin(1.0));
    }
    // Preview working size: margin included at most 524,288 pixels.
    let out_side = |side: u32| side + 2 * classic.margin(1.0);
    let mut preview = 724u32;
    while (out_side(preview) as u64).pow(2) > 524_288 {
        preview -= 4;
    }
    let base = photo(preview);
    let out = out_side(preview);
    let ms = median_ms(|| {
        std::hint::black_box(apply_styles(&base, &classic, 1.0));
    });
    report(&format!("apply_styles ClassicSticker at the Preview size ({preview} px base, {out} x {out} out)"), ms, 33.0);
}

#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k1 -- --ignored --test-threads=1)"]
fn perf_k1_apply_styles_memory_at_4096() {
    #[cfg(not(debug_assertions))]
    {
        use crate::sticker::style::StyleStack;
        let base = photo(4096);
        let classic = StylePreset::ClassicSticker.styles([512, 512]);
        let scaled = StylePreset::ClassicSticker.styles([4096, 4096]);
        let mut all = StylePreset::Badge.styles([4096, 4096]);
        all.outer_glow = StylePreset::Glow.styles([4096, 4096]).outer_glow;
        all.inner_glow = all.outer_glow.clone();
        all.inner_shadow = all.drop_shadow.clone();
        let mb = |st: &StyleStack| {
            crate::text::styled::alloc_count::peak_during(|| {
                std::hint::black_box(apply_styles(&base, st, 1.0));
            }) as f64
                / (1024.0 * 1024.0)
        };
        let out = |st: &StyleStack| {
            let s = 4096 + 2 * st.margin(1.0) as u64;
            (s * s * 4) as f64 / (1024.0 * 1024.0)
        };
        let (c, sc, a) = (mb(&classic), mb(&scaled), mb(&all));
        let verdict = if c <= 256.0 { "ok" } else { "MISSED" };
        eprintln!("peak memory of apply_styles at 4096 x 4096 (budget 256 MB, the output included):");
        eprintln!("  ClassicSticker {c:.0} MB (output {:.0} MB) {verdict}", out(&classic));
        eprintln!("  ClassicSticker scaled to the picture {sc:.0} MB (output {:.0} MB; reported only)", out(&scaled));
        eprintln!("  every style on {a:.0} MB (output {:.0} MB; reported only)", out(&all));
        assert!(c <= 256.0 * 1.5, "ClassicSticker peak {c:.0} MB");
    }
    #[cfg(debug_assertions)]
    eprintln!("peak memory: not measured in a debug build (the counter is in release test builds only)");
}

#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k1 -- --ignored --test-threads=1)"]
fn perf_k1_edt_2048() {
    let base = photo(2048);
    let alpha: Vec<u8> = base.data.iter().skip(3).step_by(4).copied().collect();
    let ms = median_ms(|| {
        std::hint::black_box(distance_to_ink(&alpha, 2048, 2048));
    });
    report("distance_to_ink 2048 x 2048", ms, 80.0);
}

// ---------------------------------------------------------------------------------------------------------------
// Package P2 (`perf_k2_*`): `cargo test -p wte_core --release --lib perf_k2 -- --ignored --test-threads=1 --nocapture`
// ---------------------------------------------------------------------------------------------------------------

/// An object on a white background with an anti-aliased edge (the usual "remove the background" picture): a disc and
/// a bar in a dark red, a little noise in the background.
fn object_on_white(side: u32) -> PixelBuf {
    use crate::sticker::select::Selection;
    use glam::Vec2;
    let s = side as f32;
    let disc = Selection::ellipse(side, side, Vec2::splat(s * 0.15), Vec2::splat(s * 0.7)).mask;
    let bar = Selection::rect(side, side, Vec2::new(s * 0.5, s * 0.55), Vec2::new(s * 0.9, s * 0.85)).mask;
    let mut rng = XorShift::new(side as u64 + 1);
    let mut b = PixelBuf::new(side, side);
    for (i, p) in b.data.chunks_mut(4).enumerate() {
        let k = disc.data[i].max(bar.data[i]) as u32;
        let n = rng.next_u32() % 6;
        let bg = 250 + n;
        let mix = |o: u32| ((bg * (255 - k) + o * k + 127) / 255) as u8;
        p.copy_from_slice(&[mix(170), mix(30), mix(40), 255]);
    }
    b
}

#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k2 -- --ignored --test-threads=1)"]
fn perf_k2_wand_magic_eraser_remove_background_2048() {
    use crate::sticker::cutout::{RemoveBgOptions, remove_background};
    use crate::sticker::select::{RegionOptions, erase, region};
    let img = object_on_white(2048);
    let opts = RegionOptions { tolerance: 32, contiguous: true, anti_alias: true };
    let ms = median_ms(|| {
        std::hint::black_box(region(&img, (0, 0), opts));
    });
    report("wand (contiguous, anti-alias) 2048 x 2048", ms, 60.0);
    let ms = median_ms(|| {
        let mut p = img.clone();
        let r = region(&p, (0, 0), opts);
        erase(&mut p, &r);
        std::hint::black_box(p);
    });
    report("magic eraser 2048 x 2048 (picture copy included)", ms, 60.0);
    let ms = median_ms(|| {
        let mut p = img.clone();
        let cut = remove_background(&p, RemoveBgOptions::default()).unwrap();
        cut.apply(&mut p, None);
        std::hint::black_box(p);
    });
    report("remove background 2048 x 2048 (found and applied)", ms, 150.0);
    let opts = RemoveBgOptions { follow_gradients: true, enclosed: true, ..RemoveBgOptions::default() };
    let ms = median_ms(|| {
        let mut p = img.clone();
        let cut = remove_background(&p, opts).unwrap();
        cut.apply(&mut p, None);
        std::hint::black_box(p);
    });
    eprintln!("remove background 2048 x 2048 with Follow gradients and Also enclosed areas: {ms:.1} ms (reported only)");
}

#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k2 -- --ignored --test-threads=1)"]
fn perf_k2_gaussian_blur_radius_16_2048() {
    use crate::sticker::filter::{blur, sharpen};
    let img = photo(2048);
    let ms = median_ms(|| {
        std::hint::black_box(blur(&img, 16.0, None));
    });
    report("Gaussian blur radius 16, 2048 x 2048", ms, 150.0);
    let ms = median_ms(|| {
        std::hint::black_box(blur(&img, 64.0, None));
    });
    eprintln!("Gaussian blur radius 64, 2048 x 2048: {ms:.1} ms (reported only)");
    let ms = median_ms(|| {
        std::hint::black_box(sharpen(&img, 1.0, 4.0, None));
    });
    eprintln!("sharpen amount 100 % radius 4, 2048 x 2048: {ms:.1} ms (reported only)");
}

#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k2 -- --ignored --test-threads=1)"]
fn perf_k2_flood_fill_worst_case_4096() {
    use crate::sticker::select::{RegionOptions, region};
    // One colour everywhere: the flood reaches every pixel (the stack holds one entry per run).
    let img = PixelBuf::filled(4096, 4096, [10, 200, 30, 255]);
    for aa in [false, true] {
        let opts = RegionOptions { tolerance: 0, contiguous: true, anti_alias: aa };
        let mut r = None;
        let ms = median_ms(|| {
            r = Some(region(&img, (2048, 2048), opts));
        });
        assert!(r.is_some_and(|r| r.data.iter().all(|&v| v == 255)));
        report(&format!("flood fill 4096 x 4096 one colour (anti-alias {aa})"), ms, 400.0);
    }
    // A worst case for the span stack: one-pixel vertical stripes joined at the bottom (a comb).
    let mut comb = PixelBuf::filled(4096, 4096, [0, 0, 0, 255]);
    for y in 0..4095 {
        for x in (1..4096).step_by(2) {
            comb.set(x, y, [255, 255, 255, 255]);
        }
    }
    let opts = RegionOptions { tolerance: 0, contiguous: true, anti_alias: false };
    let ms = median_ms(|| {
        std::hint::black_box(region(&comb, (0, 0), opts));
    });
    report("flood fill 4096 x 4096 comb (2,048 one-pixel teeth)", ms, 400.0);
}

/// 8.4 "sticker::render, 3 layers (picture, text, shape) with one layer style, 1024 x 1024: cold / one layer changed"
/// (120 / 50 ms) and "Preview-quality render" (33 ms; the Preview scale of a 1024 x 1024 sticker is 1, so the Preview
/// row is the cached case with the changed layer drawn at Preview quality). Stickers P3a.
#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k3a -- --ignored --test-threads=1)"]
fn perf_k3a_render_three_layers_1024() {
    use std::sync::Arc;

    use glam::Vec2;

    use crate::ids::StickerLayerId;
    use crate::sticker::model::{PicturePixels, StickerContent, StickerDoc, StickerLayer};
    use crate::sticker::render::{LayerCache, render_with};
    use crate::sticker::shape::{Paint, ShapeKind, ShapeSpec};
    use crate::text::RasterQuality;

    let mut doc = crate::testutil::doc_from_raw(crate::samples::flat_quad(), 32);
    let mut sticker = StickerDoc::new("perf", 1024, 1024);
    let picture = PicturePixels::Owned { pixels: Arc::new(photo(1024)), version: doc.next_pixel_version() };
    let mut layer = StickerLayer::new(StickerLayerId(1), "photo", StickerContent::Picture(picture));
    layer.styles = StylePreset::ClassicSticker.styles([1024, 1024]);
    sticker.layers.push(layer);
    let mut spec = crate::text::TextSpec::new("Sticker", doc.default_font());
    spec.size_px = 160.0;
    let mut text = StickerLayer::new(StickerLayerId(2), "text", StickerContent::Text(Box::new(spec)));
    text.transform.translate = Vec2::new(512.0, 800.0);
    sticker.layers.push(text);
    let shape = ShapeSpec { kind: ShapeKind::Star { points: 5, inner: 0.5 }, size: Vec2::splat(300.0), fill: Paint::Solid([250, 200, 20, 255]), outline: Some(([0, 0, 0, 255], 8.0)) };
    let mut star = StickerLayer::new(StickerLayerId(3), "star", StickerContent::Shape(shape));
    star.transform.translate = Vec2::new(650.0, 80.0);
    sticker.layers.push(star);
    sticker.next_layer = 4;
    let doc_arc = Arc::new(sticker.clone());
    let input = doc.sticker_render_input(&doc_arc).unwrap();
    let cold = median_ms(|| {
        let _ = render_with(&input, 1.0, RasterQuality::Full, None);
    });
    report("sticker render, 3 layers + ClassicSticker on the photo, 1024, cold", cold, 120.0);
    // One layer changed: the star moves, the photo and the text come from the layer cache.
    let mut cache = LayerCache::default();
    let _ = render_with(&input, 1.0, RasterQuality::Full, Some(&mut cache));
    let mut step = 0.0f32;
    let mut changed = |quality: RasterQuality| {
        step += 1.0;
        let mut moved = sticker.clone();
        moved.layers[2].transform.translate.x += step;
        let input = StickerRenderInput { doc: Arc::new(moved), ..input.clone() };
        let _ = render_with(&input, 1.0, quality, Some(&mut cache));
    };
    let one = median_ms(|| changed(RasterQuality::Full));
    report("sticker render, one layer changed (layer cache), 1024", one, 50.0);
    let preview = median_ms(|| changed(RasterQuality::Preview { max_pixels: crate::doc_stamps::TEXT_EDIT_PREVIEW_PIXELS }));
    report("sticker render at Preview quality, one layer changed, 1024", preview, 33.0);
}

/// STICKER_PLAN 8.4 "GrabCut (S1) 2048x2048": 3 s (on a job with a progress note). The star scene at 2048 x 2048,
/// the box around the star (working copy 512 px, step 4, then the full-size re-cut and the soft edge).
#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_s1 -- --ignored --test-threads=1)"]
fn perf_s1_grabcut_box_2048() {
    use crate::sticker::grabcut::cut_out_box;
    use crate::sticker::grabcut::tests::{Scene, scene};
    let (img, _, b) = scene(Scene::StarOnCarpet, 2048, 2048);
    let ms = median_ms(|| {
        std::hint::black_box(cut_out_box(&img, b, &|_| true).unwrap());
    });
    report("GrabCut box 2048 x 2048 (found, not applied)", ms, 3000.0);
    // The box over the whole picture (the largest window, the ring background).
    let whole = crate::math::IRect::from_size(2048, 2048);
    let ms = median_ms(|| {
        std::hint::black_box(cut_out_box(&img, whole, &|_| true).unwrap());
    });
    report("GrabCut box over the whole 2048 x 2048 picture", ms, 3000.0);
}

/// S1 check: the worst case of the full-size re-cut - a box over fine red / green noise on a green picture
/// (2048 x 2048). The red pixels are the "object", so the result is salt and pepper and the re-cut band covers the
/// whole window; the cut must still stay within the 8.4 budget.
#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_check_s1 -- --ignored --test-threads=1)"]
fn perf_check_s1_grabcut_fragmented_worst_case_2048() {
    use crate::sticker::grabcut::cut_out_box;
    let mut img = crate::pixel::PixelBuf::new(2048, 2048);
    let mut rng = crate::testutil::XorShift::new(5);
    let b = crate::math::IRect::new(200, 200, 1848, 1848);
    // Inside the box: red and green noise in cells of the working copy's step (4 px), so the working result is the
    // same salt and pepper; outside: green only (the red cells are the "object").
    let cells: Vec<bool> = (0..512 * 512).map(|_| rng.next_f32() < 0.5).collect();
    for y in 0..2048 {
        for x in 0..2048 {
            let red = b.contains(x as i32, y as i32) && cells[(y as usize / 4) * 512 + x as usize / 4];
            img.set(x, y, if red { [200, 60, 40, 255] } else { [40, 160, 60, 255] });
        }
    }
    let ms = median_ms(|| {
        let _ = std::hint::black_box(cut_out_box(&img, b, &|_| true));
    });
    let kept = cut_out_box(&img, b, &|_| true).map(|c| c.keep.data.iter().filter(|&&k| k >= 128).count());
    eprintln!("kept pixels: {kept:?}");
    report("GrabCut box over 2048 x 2048 two-colour noise (fragmented)", ms, 3000.0);
}
