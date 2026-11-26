//! Timings of STICKER_PLAN 8.4 for package P1 (ignored `perf_k1_*` tests, run serially in a release test build:
//! `cargo test -p wte_core --release --lib perf_k1 -- --ignored --test-threads=1 --nocapture`). Median of 5 runs after
//! one warm-up; peak working memory with the allocation counter of the release test build
//! (`text::styled::alloc_count`). A budget missed by less than 50 % is reported (and decided at the gate); more fails.

use std::time::Instant;

use crate::pixel::PixelBuf;
use crate::sticker::fx::apply_styles;
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
