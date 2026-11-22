// Owner: core-stamps-text (C4). Text rasterization (ARCHITECTURE.md 5.13, algorithm 6.8): glyph coverage, outline from
// the distance transform, fill over outline.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::error::TextError;
use crate::math::div255;
use crate::pixel::PixelBuf;
use crate::text::edt::distance_to_ink;
use crate::text::{Outline, RasterQuality, TextAlign, TextFrame, TextRaster, TextSpec};
use crate::text::layout::{layout_text, outline_px, raster_px};

/// Images with at least this many pixels are composed row-parallel.
const PAR_PIXELS: usize = 1 << 16;

/// Fill `(color.rgb, color.a * cov)` over outline `(outline.rgb, outline.a * outline_a)`, straight alpha.
#[inline]
fn fill_over_outline(color: [u8; 4], cov: u8, outline: [u8; 4], outline_a: f32) -> [u8; 4] {
    let fa = color[3] as f32 * cov as f32 * (1.0 / 65025.0);
    let oa = outline[3] as f32 * (1.0 / 255.0) * outline_a;
    let under = oa * (1.0 - fa);
    let out_a = fa + under;
    let a8 = (out_a * 255.0 + 0.5) as u8;
    if a8 == 0 || out_a <= 0.0 {
        return [0; 4];
    }
    let mix = |f: u8, o: u8| ((f as f32 * fa + o as f32 * under) / out_a + 0.5).clamp(0.0, 255.0) as u8;
    [mix(color[0], outline[0]), mix(color[1], outline[1]), mix(color[2], outline[2]), a8]
}

/// Rasterizes `spec` (fill over outline) into a straight-alpha image. `fonts` as in `layout_text`.
///
/// The image has the size `measure_text` reports. Fully transparent pixels are `[0, 0, 0, 0]`. An outline of zero
/// width or zero alpha draws nothing.
pub fn rasterize_text(fonts: &[&fontdue::Font], spec: &TextSpec) -> Result<PixelBuf, TextError> {
    let layout = layout_text(fonts, spec)?;
    let px = raster_px(spec);
    let (w, h) = (layout.width as usize, layout.height as usize);

    // 1. Coverage canvas: the maximum over all placed glyphs.
    let mut cov = vec![0u8; w * h];
    let mut bitmaps: HashMap<(u8, char), (usize, usize, Vec<u8>)> = HashMap::new();
    for glyph in &layout.glyphs {
        if glyph.width == 0 || glyph.height == 0 {
            continue;
        }
        let Some(font) = fonts.get(glyph.font_index as usize) else { continue };
        let (gw, gh, bitmap) = bitmaps.entry((glyph.font_index, glyph.ch)).or_insert_with(|| {
            let (metrics, bitmap) = font.rasterize(glyph.ch, px);
            (metrics.width, metrics.height, bitmap)
        });
        let (gw, gh) = (*gw, *gh);
        if bitmap.len() < gw * gh {
            continue;
        }
        // Clip the bitmap to the canvas.
        let (gx, gy) = (glyph.x as i64, glyph.y as i64);
        let x0 = gx.max(0);
        let x1 = (gx + gw as i64).min(w as i64);
        let y0 = gy.max(0);
        let y1 = (gy + gh as i64).min(h as i64);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let n = (x1 - x0) as usize;
        for y in y0..y1 {
            let src = (y - gy) as usize * gw + (x0 - gx) as usize;
            let dst = y as usize * w + x0 as usize;
            for (d, s) in cov[dst..dst + n].iter_mut().zip(&bitmap[src..src + n]) {
                *d = (*d).max(*s);
            }
        }
    }

    // 2. and 3.: outline and fill over outline.
    Ok(compose(&cov, layout.width, layout.height, spec.color, spec.outline, outline_px(spec, px)))
}

/// Steps 2 and 3 of both painters (FEATURE_TEXT 5.10): the outline from the distance to the ink of the merged
/// coverage `cov` (`width x height`, one byte per pixel) with a one-pixel ramp at `ow` raster pixels, then the fill
/// `(color.rgb, color.a * coverage)` over it. Rows in parallel from `PAR_PIXELS` pixels up.
pub(crate) fn compose(cov: &[u8], width: u32, height: u32, color: [u8; 4], outline: Option<Outline>, ow: f32) -> PixelBuf {
    let (w, h) = (width as usize, height as usize);
    // 2. Outline: distance to the ink, turned into coverage with a one-pixel ramp at `ow`.
    let outline = outline.filter(|o| ow > 0.0 && o.color[3] > 0);
    let distance = outline.map(|_| distance_to_ink(cov, width, height));

    // 3. Fill over outline.
    let mut out = PixelBuf::new(width, height);
    let compose_row = |(y, row): (usize, &mut [u8])| {
        let cov = &cov[y * w..(y + 1) * w];
        match (&outline, &distance) {
            (Some(outline), Some(distance)) => {
                let distance = &distance[y * w..(y + 1) * w];
                for ((p, &c), &d) in row.as_chunks_mut::<4>().0.iter_mut().zip(cov).zip(distance) {
                    let outline_a = (ow + 0.5 - d).clamp(0.0, 1.0);
                    if c == 0 && outline_a <= 0.0 {
                        continue;
                    }
                    *p = fill_over_outline(color, c, outline.color, outline_a);
                }
            }
            _ => {
                for (p, &c) in row.as_chunks_mut::<4>().0.iter_mut().zip(cov) {
                    let a = div255(color[3] as u32 * c as u32) as u8;
                    if a > 0 {
                        *p = [color[0], color[1], color[2], a];
                    }
                }
            }
        }
    };
    if w * h >= PAR_PIXELS {
        out.data.par_chunks_mut(w * 4).enumerate().for_each(compose_row);
    } else {
        out.data.chunks_mut(w * 4).enumerate().for_each(compose_row);
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: the dispatcher between the classic and the styled painter (FEATURE_TEXT 5.1). A spec whose
// `fx.needs_outlines()` goes to the styled painter (`styled.rs`); every other spec to the classic painter above.
// ---------------------------------------------------------------------------------------------------------------------

/// One face as both painters need it: the `fontdue` font (positions, advances, kerning) and the file bytes it was
/// parsed from (`ttf_parser` outlines for the styled painter).
#[derive(Copy, Clone)]
pub struct FaceFonts<'a> {
    pub metrics: &'a fontdue::Font,
    pub bytes: &'a [u8],
    pub face_index: u32,
}

/// The fonts of one text: its own face (`None` when its blob is missing) and the bundled fallback.
#[derive(Copy, Clone)]
pub struct TextFonts<'a> {
    pub font: Option<FaceFonts<'a>>,
    pub fallback: FaceFonts<'a>,
}

impl<'a> TextFonts<'a> {
    /// The `fontdue` fonts in layout order: the text's own face first (when present), then the fallback.
    pub fn metrics(&self) -> Vec<&'a fontdue::Font> {
        match self.font {
            Some(font) if !std::ptr::eq(font.metrics, self.fallback.metrics) => vec![font.metrics, self.fallback.metrics],
            Some(font) => vec![font.metrics],
            None => vec![self.fallback.metrics],
        }
    }

    /// The faces in the order of `metrics()` (the styled painter reads the outlines of `faces()[i]` for a glyph that
    /// the layout took from `metrics()[i]`).
    pub fn faces(&self) -> Vec<FaceFonts<'a>> {
        match self.font {
            Some(font) if !std::ptr::eq(font.metrics, self.fallback.metrics) => vec![font, self.fallback],
            Some(font) => vec![font],
            None => vec![self.fallback],
        }
    }
}

/// The frame of the classic painter (FEATURE_TEXT 5.1): the canvas, the anchor in its centre, the line box inside
/// the padding, the first baseline and the alignment point. Geometry only.
pub(crate) fn classic_frame(fonts: &[&fontdue::Font], spec: &TextSpec) -> Result<TextFrame, TextError> {
    let layout = layout_text(fonts, spec)?;
    let px = raster_px(spec);
    let (w, h) = (layout.width as f32, layout.height as f32);
    let pad = layout.pad as f32;
    let (box_w, box_h) = (w - 2.0 * pad, h - 2.0 * pad);
    let ascent = match fonts.first().and_then(|f| f.horizontal_line_metrics(px)) {
        Some(m) if m.ascent.is_finite() && m.descent.is_finite() && m.new_line_size.is_finite() => m.ascent,
        _ => 0.8 * px,
    };
    let anchor = [w / 2.0, h / 2.0];
    let align_x = match spec.align {
        TextAlign::Left => anchor[0] - box_w / 2.0,
        TextAlign::Center => anchor[0],
        TextAlign::Right => anchor[0] + box_w / 2.0,
    };
    Ok(TextFrame {
        size: [layout.width, layout.height],
        anchor_px: anchor,
        em_px: px,
        box_px: [box_w, box_h],
        baseline0_px: ascent.round() - box_h / 2.0,
        align_px: [align_x, anchor[1]],
        circle_fill: 0.0,
    })
}

/// Where the text sits in its full-resolution picture, without making pixels: exactly the frame
/// `render_text(.., Full)` reports.
pub fn text_frame(fonts: &TextFonts<'_>, spec: &TextSpec) -> Result<TextFrame, TextError> {
    if spec.fx.needs_outlines() {
        crate::text::styled::styled_frame(fonts, spec)
    } else {
        classic_frame(&fonts.metrics(), spec)
    }
}

/// Renders a text. Without thickening, slant, warp or path: the classic painter (`rasterize_text`, always in full;
/// `quality` is ignored). Otherwise the styled painter (`styled.rs`).
pub fn render_text(fonts: &TextFonts<'_>, spec: &TextSpec, quality: RasterQuality) -> Result<TextRaster, TextError> {
    if spec.fx.needs_outlines() {
        return crate::text::styled::render_styled(fonts, spec, quality);
    }
    let metrics = fonts.metrics();
    let frame = classic_frame(&metrics, spec)?;
    let image = rasterize_text(&metrics, spec)?;
    Ok(TextRaster { image, frame, full: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::IRect;
    use crate::pixel_ops::content_bounds;
    use crate::text::layout::measure_text;
    use crate::text::layout::testkit::*;
    use crate::text::{Outline, TextAlign};

    fn ink(buf: &PixelBuf) -> IRect {
        content_bounds(buf.view()).expect("the text has ink")
    }

    #[test]
    fn measure_equals_the_raster_size() {
        let font = ubuntu(128.0);
        let hack = hack();
        let mut outlined = spec("Outlined\ntext");
        outlined.outline = Some(Outline { width_em: 0.08, color: [10, 20, 30, 255] });
        outlined.align = TextAlign::Center;
        let mut small = spec("small j");
        small.size_px = 19.0;
        let mut spaced = spec("s p a c e d");
        spaced.letter_spacing = 0.2;
        spaced.line_spacing = 1.7;
        for s in [spec("Hello, World"), spec(""), spec("a\n\nb"), outlined, small, spaced] {
            let image = rasterize_text(&[&font, &hack], &s).unwrap();
            assert_eq!(measure_text(&[&font, &hack], &s).unwrap(), [image.width, image.height], "{:?}", s.content);
            assert_eq!(image.data.len(), image.width as usize * image.height as usize * 4);
        }
    }

    #[test]
    fn fill_colour_and_coverage() {
        let font = ubuntu(128.0);
        let mut s = spec("H");
        s.color = [200, 100, 50, 255];
        let image = rasterize_text(&[&font], &s).unwrap();
        let mut opaque = 0;
        for p in image.data.as_chunks::<4>().0 {
            if p[3] == 0 {
                assert_eq!(*p, [0; 4]);
            } else {
                assert_eq!(&p[..3], &[200, 100, 50]);
                opaque += (p[3] == 255) as u32;
            }
        }
        assert!(opaque > 500, "the stems of a 128 px H are fully covered ({opaque})");
        // The ink lies inside the padding.
        let r = ink(&image);
        assert!(r.x0 >= 1 && r.y0 >= 1 && r.x1 < image.width as i32 && r.y1 < image.height as i32, "{r:?}");

        // The colour's alpha scales the coverage.
        s.color = [200, 100, 50, 128];
        let half = rasterize_text(&[&font], &s).unwrap();
        let max_a = half.data.as_chunks::<4>().0.iter().map(|p| p[3]).max().unwrap();
        assert_eq!(max_a, 128);
        s.color = [200, 100, 50, 0];
        let none = rasterize_text(&[&font], &s).unwrap();
        assert!(none.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn empty_text_is_fully_transparent_and_one_line_high() {
        let font = ubuntu(128.0);
        let empty = rasterize_text(&[&font], &spec("")).unwrap();
        assert!(empty.data.iter().all(|&b| b == 0));
        let line = rasterize_text(&[&font], &spec("x")).unwrap();
        assert_eq!(empty.height, line.height);
        assert!(empty.width >= 5);
        // Also with an outline: no ink, no outline.
        let mut s = spec("");
        s.outline = Some(Outline { width_em: 0.1, color: [255, 0, 0, 255] });
        assert!(rasterize_text(&[&font], &s).unwrap().data.iter().all(|&b| b == 0));
    }

    #[test]
    fn right_alignment_shifts_the_ink() {
        let font = ubuntu(128.0);
        let lower_half_ink = |align: TextAlign| -> IRect {
            let mut s = spec("WWWW\ni");
            s.align = align;
            let image = rasterize_text(&[&font], &s).unwrap();
            // Ink of the second line only.
            let mut lower = image.clone();
            let split = image.height / 2;
            for y in 0..split {
                lower.row_mut(y).fill(0);
            }
            ink(&lower)
        };
        let (left, center, right) =
            (lower_half_ink(TextAlign::Left), lower_half_ink(TextAlign::Center), lower_half_ink(TextAlign::Right));
        assert!(left.x0 < center.x0 && center.x0 < right.x0, "{left:?} {center:?} {right:?}");
        assert_eq!(left.width(), right.width());
        let full = ink(&rasterize_text(&[&font], &spec("WWWW\ni")).unwrap());
        assert!(left.x0 - full.x0 < 20, "left-aligned ink starts at the left edge");
        assert!(full.x1 - right.x1 < 20, "right-aligned ink ends at the right edge");
    }

    #[test]
    fn letter_spacing_widens_the_ink() {
        let font = ubuntu(128.0);
        let plain = ink(&rasterize_text(&[&font], &spec("abc")).unwrap());
        let mut s = spec("abc");
        s.letter_spacing = 0.5;
        let spaced = ink(&rasterize_text(&[&font], &s).unwrap());
        // Two gaps of 64 px.
        assert!((spaced.width() as i32 - plain.width() as i32 - 128).abs() <= 2, "{plain:?} {spaced:?}");
        assert_eq!(spaced.height(), plain.height());
    }

    #[test]
    fn outline_grows_the_ink_box_by_twice_its_width() {
        let font = ubuntu(128.0);
        let plain_image = rasterize_text(&[&font], &spec("Text")).unwrap();
        let plain = ink(&plain_image);
        let mut s = spec("Text");
        s.color = [255, 255, 255, 255];
        s.outline = Some(Outline { width_em: 0.1, color: [255, 0, 0, 255] });
        let image = rasterize_text(&[&font], &s).unwrap();
        let outlined = ink(&image);
        let want = 2.0 * 0.1 * 128.0;
        let (dw, dh) = (
            outlined.width() as f32 - plain.width() as f32,
            outlined.height() as f32 - plain.height() as f32,
        );
        assert!((dw - want).abs() <= 3.0 && (dh - want).abs() <= 3.0, "grew by {dw} x {dh}, expected about {want}");
        // The canvas grew by the padding of the outline; the ink box stays centred in the same place.
        assert_eq!(image.width, plain_image.width + 26);
        assert!((outlined.x0 + outlined.x1 - 26 - (plain.x0 + plain.x1)).abs() <= 2);

        // Inside a stem the fill covers the outline; just outside the glyph the outline colour shows, opaque.
        let (mut white, mut red) = (0, 0);
        for p in image.data.as_chunks::<4>().0 {
            white += (*p == [255, 255, 255, 255]) as u32;
            red += (*p == [255, 0, 0, 255]) as u32;
        }
        assert!(white > 1000 && red > 1000, "{white} white, {red} red");
        // Every pixel that had ink is opaque now (the outline lies under anti-aliased glyph edges).
        for y in 0..plain_image.height {
            for x in 0..plain_image.width {
                if plain_image.get(x, y)[3] >= 128 {
                    assert_eq!(image.get(x + 13, y + 13)[3], 255, "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn outline_width_scales_with_the_raster_size() {
        // The same em fraction at 64 px and at 128 px: the outlined ink box is twice as large (em-relative look).
        let measure = |px: f32| -> (IRect, IRect) {
            let font = ubuntu(px);
            let mut s = spec("o");
            s.size_px = px;
            let plain = ink(&rasterize_text(&[&font], &s).unwrap());
            s.outline = Some(Outline { width_em: 0.125, color: [0, 0, 0, 255] });
            (plain, ink(&rasterize_text(&[&font], &s).unwrap()))
        };
        let (p64, o64) = measure(64.0);
        let (p128, o128) = measure(128.0);
        let grow64 = (o64.width() - p64.width()) as f32;
        let grow128 = (o128.width() - p128.width()) as f32;
        assert!((grow64 - 16.0).abs() <= 2.0, "{grow64}");
        assert!((grow128 - 32.0).abs() <= 2.0, "{grow128}");
    }

    #[test]
    fn translucent_outline_and_degenerate_outlines() {
        let font = ubuntu(128.0);
        let mut s = spec("I");
        s.color = [0, 0, 255, 255];
        s.outline = Some(Outline { width_em: 0.1, color: [255, 0, 0, 128] });
        let image = rasterize_text(&[&font], &s).unwrap();
        let mut seen_outline = false;
        for p in image.data.as_chunks::<4>().0 {
            if *p == [255, 0, 0, 128] {
                seen_outline = true;
            }
            // Fill over a half transparent outline: alpha never drops below the outline's where there is ink.
            if p[2] > 0 && p[0] == 0 {
                assert_eq!(p[3], 255);
            }
        }
        assert!(seen_outline);

        // Zero width or zero alpha: identical to no outline at all (the canvas keeps the plain size too).
        let plain = rasterize_text(&[&font], &spec("I")).unwrap();
        let mut zero_width = spec("I");
        zero_width.outline = Some(Outline { width_em: 0.0, color: [255, 0, 0, 255] });
        assert_eq!(rasterize_text(&[&font], &zero_width).unwrap(), plain);
        let mut zero_alpha = spec("I");
        zero_alpha.outline = Some(Outline { width_em: 0.1, color: [255, 0, 0, 0] });
        let image = rasterize_text(&[&font], &zero_alpha).unwrap();
        assert_eq!(ink(&image).width(), ink(&plain).width());
    }

    #[test]
    fn missing_glyph_falls_back_to_the_next_font() {
        let (ubuntu, hack) = (ubuntu(128.0), hack());
        let ch = only_in_hack(&ubuntu, &hack);
        let s = spec(&ch.to_string());
        let with_fallback = rasterize_text(&[&ubuntu, &hack], &s).unwrap();
        let hack_only = rasterize_text(&[&hack], &s).unwrap();
        // The glyph is Hack's: the same ink as when Hack is the only font (the line metrics come from fonts[0]).
        let (a, b) = (ink(&with_fallback), ink(&hack_only));
        assert_eq!((a.width(), a.height()), (b.width(), b.height()));
        // Without the fallback Ubuntu draws its missing-glyph box (or nothing): different ink.
        let alone = rasterize_text(&[&ubuntu], &s).unwrap();
        let differs = match content_bounds(alone.view()) {
            None => true,
            Some(r) => (r.width(), r.height()) != (a.width(), a.height()),
        };
        assert!(differs);
    }

    #[test]
    fn large_text_uses_the_parallel_path_deterministically() {
        let font = ubuntu(512.0);
        let mut s = spec("Big");
        s.size_px = 512.0;
        s.outline = Some(Outline { width_em: 0.05, color: [0, 0, 0, 255] });
        let a = rasterize_text(&[&font], &s).unwrap();
        let b = rasterize_text(&[&font], &s).unwrap();
        assert!(a.width as usize * a.height as usize >= PAR_PIXELS);
        assert_eq!(a, b);
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use crate::text::layout::testkit::*;
    use crate::text::{Outline, TextAlign};

    #[test]
    fn the_classic_frame_matches_the_classic_picture() {
        let font = ubuntu(128.0);
        let bytes = crate::text::default_font_bytes();
        let face = FaceFonts { metrics: &font, bytes, face_index: 0 };
        let fonts = TextFonts { font: None, fallback: face };
        for (content, align, outline) in [
            ("WTE 2026", TextAlign::Left, None),
            ("two\ncentred lines", TextAlign::Center, Some(Outline { width_em: 0.08, color: [0, 0, 0, 255] })),
            ("jag", TextAlign::Right, None),
        ] {
            let mut s = spec(content);
            s.align = align;
            s.outline = outline;
            let raster = render_text(&fonts, &s, RasterQuality::Preview { max_pixels: 16 }).unwrap();
            assert!(raster.full, "the classic painter always renders in full");
            assert_eq!(raster.image, rasterize_text(&[&font], &s).unwrap(), "the classic bytes of round 3");
            let frame = text_frame(&fonts, &s).unwrap();
            assert_eq!(frame, raster.frame);
            assert_eq!(frame.size, [raster.image.width, raster.image.height]);
            assert_eq!(frame.anchor_px, [raster.image.width as f32 / 2.0, raster.image.height as f32 / 2.0]);
            assert_eq!(frame.em_px, 128.0);
            assert!(frame.box_px[0] >= 1.0 && frame.box_px[0] < raster.image.width as f32);
            let expected_x = match align {
                TextAlign::Left => frame.anchor_px[0] - frame.box_px[0] / 2.0,
                TextAlign::Center => frame.anchor_px[0],
                TextAlign::Right => frame.anchor_px[0] + frame.box_px[0] / 2.0,
            };
            assert_eq!(frame.align_px, [expected_x, frame.anchor_px[1]]);
            assert!(frame.baseline0_px.abs() < frame.box_px[1] / 2.0, "the first baseline lies inside the line box");
            assert_eq!(frame.circle_fill, 0.0);
        }
    }

    /// The eight round-3 hashes of `target\round4\perf\text_baseline.txt` through the dispatcher (`render_text`, the
    /// path every document bake takes), with the frame of `text_frame`.
    #[test]
    fn plain_text_is_bit_identical() {
        for case in baseline_cases() {
            let fonts = baseline_fonts(&case);
            let ubuntu_bytes = crate::text::default_font_bytes();
            let fallback = FaceFonts { metrics: fonts.last().unwrap(), bytes: ubuntu_bytes, face_index: 0 };
            let own = case.hack_first.then(|| FaceFonts { metrics: &fonts[0], bytes: epaint_default_fonts::HACK_REGULAR, face_index: 0 });
            let text_fonts = TextFonts { font: own, fallback };
            for quality in [RasterQuality::Full, RasterQuality::Preview { max_pixels: 1 << 10 }] {
                let raster = render_text(&text_fonts, &case.spec, quality).unwrap();
                assert!(raster.full);
                assert_eq!([raster.image.width, raster.image.height], case.size, "{}", case.name);
                assert_eq!(crate::math::fnv1a64(0, &raster.image.data), case.hash, "{}: the round-3 picture", case.name);
                assert_eq!(text_frame(&text_fonts, &case.spec).unwrap(), raster.frame, "{}", case.name);
            }
        }
    }
}
/// Gate stage 3 (round 4): the classic painter on the three baseline cases B1-B3 and the font timing B4 of
/// `target\round4\perf\text_baseline.txt`, measured exactly as P0's probe did on the round-3 tree (rasterize_text and
/// PremulMips::build, median of 5 after one warm-up; TEXT 8.4 rows 1 and "B4"). Run with
/// `cargo.cmd test -p wte_core --release --lib gate_classic_painter -- --ignored --nocapture --test-threads=1`.
#[cfg(test)]
mod gate_probe {
    use std::sync::Arc;
    use std::time::Instant;

    use crate::blob::{BlobKind, BlobStore};
    use crate::pixel_ops::PremulMips;
    use crate::text::{FontCache, FontRef, Outline, TextAlign, TextFx, TextSpec, measure_text, rasterize_text};

    fn base(content: &str, px: f32) -> TextSpec {
        TextSpec {
            content: content.to_owned(),
            font: FontRef { blob: crate::ids::BlobId(0), face_index: 0, family: "Ubuntu".to_owned(), style: "Light".to_owned() },
            size_px: px,
            color: [220, 30, 40, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: TextFx::default(),
        }
    }

    fn ubuntu(px: f32) -> Arc<fontdue::Font> {
        FontCache::default().fallback(px)
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    fn time5(name: &str, mut f: impl FnMut() -> f64) {
        f();
        let v: Vec<f64> = (0..5).map(|_| f()).collect();
        let r: Vec<f64> = v.iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
        eprintln!("gate time {name}: median {:.3} ms (5 runs after one warm-up: {r:?})", median(v));
    }

    #[test]
    #[ignore = "timing probe of the gate"]
    fn gate_classic_painter() {
        let raster_mips = |spec: &TextSpec, mips: bool| {
            let u = ubuntu(spec.size_px);
            let t = Instant::now();
            let img = rasterize_text(&[&*u], spec).unwrap();
            if mips {
                std::hint::black_box(PremulMips::build(&img));
            }
            std::hint::black_box(&img);
            t.elapsed().as_secs_f64() * 1e3
        };
        let b1 = base("WTE 2026", 256.0);
        time5("B1 rasterize_text + PremulMips::build, \"WTE 2026\" at 256 px per em", || raster_mips(&b1, true));
        time5("B1 rasterize_text only", || raster_mips(&b1, false));
        let mut b2 = base("ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789", 128.0);
        b2.content.truncate(60);
        b2.outline = Some(Outline { width_em: 0.06, color: [0, 0, 0, 255] });
        time5("B2 rasterize_text + PremulMips::build, 60 letters at 128 px per em, 6 % outline", || raster_mips(&b2, true));
        time5("B2 rasterize_text only", || raster_mips(&b2, false));
        let u512 = ubuntu(512.0);
        let pattern: Vec<char> = "WTE 2026 ".chars().collect();
        let mut content = String::new();
        let mut i = 0;
        loop {
            content.push(pattern[i % pattern.len()]);
            i += 1;
            let [w, _] = measure_text(&[&*u512], &base(&content, 512.0)).unwrap();
            if w as f32 >= 16.0 * 512.0 * 0.98 || i > 200 {
                break;
            }
        }
        let b3 = base(&content, 512.0);
        let img = rasterize_text(&[&*u512], &b3).unwrap();
        eprintln!("gate B3 content {:?} ({} chars), picture {}x{}", content, content.chars().count(), img.width, img.height);
        time5("B3 rasterize_text + PremulMips::build, one line 16 em at 512 px per em", || raster_mips(&b3, true));
        time5("B3 rasterize_text only", || raster_mips(&b3, false));
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(r"C:\Windows\Fonts")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(std::path::Path::new(&local).join(r"Microsoft\Windows\Fonts"));
        }
        for d in &dirs {
            if let Ok(rd) = std::fs::read_dir(d) {
                for e in rd.flatten() {
                    let p = e.path();
                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                    if ["ttf", "otf", "ttc"].contains(&ext.as_str()) {
                        files.push(p);
                    }
                }
            }
        }
        let largest = files.iter().max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).cloned();
        let arial = std::path::PathBuf::from(r"C:\Windows\Fonts\arial.ttf");
        eprintln!("gate B4 font files found: {}", files.len());
        for (name, path) in [("Arial", Some(arial)), ("the largest installed font file", largest)] {
            let Some(path) = path else { continue };
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("gate B4 {name}: cannot read {}", path.display());
                continue;
            };
            eprintln!("gate B4 {name}: {} ({} bytes)", path.display(), bytes.len());
            let bytes: Arc<[u8]> = Arc::from(bytes);
            let mut blobs = BlobStore::default();
            let blob = blobs.insert(BlobKind::Font, "f", Arc::clone(&bytes));
            let font = FontRef { blob, face_index: 0, family: name.to_owned(), style: "Regular".to_owned() };
            time5(&format!("B4 first FontCache::get of {name} at the 128 bucket"), || {
                let mut cache = FontCache::default();
                let t = Instant::now();
                std::hint::black_box(cache.get(&blobs, &font, 128.0).unwrap());
                t.elapsed().as_secs_f64() * 1e3
            });
        }
        time5("font scan at start (FontCatalog::bundled + load_system)", || {
            let t = Instant::now();
            let mut c = crate::text::FontCatalog::bundled();
            c.load_system();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            std::hint::black_box(c.families().len());
            ms
        });
    }
}
