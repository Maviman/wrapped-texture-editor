// Owner: core-stamps-text (C4). Text layout (ARCHITECTURE.md 5.13, algorithm 6.8).
//
// One deviation from the canvas formula of 6.8 is documented at `layout_text`: `pad` grows by the amount a glyph
// bitmap sticks out of its line box (a leading "j", italic overhang), so ink is never cut off at the canvas border.

use crate::error::TextError;
use crate::text::{GAP_MAX_EM, GAP_MIN_EM, Gap, MAX_BOLD_EM, PlacedGlyph, TEXT_PLACEMENT_PX, TextAlign, TextLayout, TextSpec};

/// Largest side of a text image.
pub(crate) const MAX_TEXT_SIDE: u64 = 8192;

/// The raster size of a spec: `size_px` clamped to 8..=512 (`TEXT_PLACEMENT_PX` when it is not a finite number).
pub(crate) fn raster_px(spec: &TextSpec) -> f32 {
    if spec.size_px.is_finite() { spec.size_px.clamp(8.0, 512.0) } else { TEXT_PLACEMENT_PX }
}

/// Outline width in raster pixels: `width_em` clamped to 0..=0.25, times the raster size. 0 without an outline.
pub(crate) fn outline_px(spec: &TextSpec, px: f32) -> f32 {
    match spec.outline {
        Some(outline) if outline.width_em.is_finite() => outline.width_em.clamp(0.0, 0.25) * px,
        _ => 0.0,
    }
}

/// Letter spacing in raster pixels: `letter_spacing` clamped to -0.5..=4 em (0 when not finite), times `px`.
pub(crate) fn letter_spacing_px(spec: &TextSpec, px: f32) -> f32 {
    if spec.letter_spacing.is_finite() { spec.letter_spacing.clamp(-0.5, 4.0) * px } else { 0.0 }
}

/// Thickening radius in raster pixels: `fx.bold_em` clamped to 0..=`MAX_BOLD_EM` (0 when not finite), times `px`.
pub(crate) fn bold_px(spec: &TextSpec, px: f32) -> f32 {
    let b = spec.fx.bold_em;
    if b.is_finite() { b.clamp(0.0, MAX_BOLD_EM) * px } else { 0.0 }
}

/// The own spacing after stored character `after`, in raster pixels (FEATURE_TEXT 5.2): the gap value of `fx.gaps`
/// (0 without an entry), clamped to `GAP_MIN_EM..=GAP_MAX_EM` (0 when not finite), times `px`. Works on an
/// unsanitized list too: an unsorted list is searched linearly and of two entries for one character the later wins.
struct GapLookup<'a> {
    gaps: &'a [Gap],
    sorted: bool,
    px: f32,
}

impl<'a> GapLookup<'a> {
    fn new(gaps: &'a [Gap], px: f32) -> Self {
        GapLookup { gaps, sorted: gaps.windows(2).all(|w| w[0].after < w[1].after), px }
    }

    fn px_after(&self, after: u32) -> f32 {
        if self.gaps.is_empty() {
            return 0.0;
        }
        let entry = if self.sorted {
            self.gaps.binary_search_by_key(&after, |g| g.after).ok().map(|i| self.gaps[i])
        } else {
            self.gaps.iter().rev().find(|g| g.after == after).copied()
        };
        match entry {
            Some(g) if g.em.is_finite() => g.em.clamp(GAP_MIN_EM, GAP_MAX_EM) * self.px,
            _ => 0.0,
        }
    }
}

/// One laid-out glyph (FEATURE_TEXT 5.2): shared by the classic and the styled painter.
#[derive(Copy, Clone, Debug)]
pub(crate) struct LaidGlyph {
    pub ch: char,
    /// Index into the font list (`fonts[font]`).
    pub font: u8,
    /// `fontdue` glyph index in that font (`lookup_glyph_index`; 0 = the missing-glyph box).
    pub glyph: u16,
    pub line: u32,
    /// Pen position of the glyph origin in its line, unrounded (after kerning).
    pub pen: f32,
    /// `pen`, rounded (the classic glyph origin).
    pub origin: i32,
    /// Width of the glyph cell: the advance width, plus twice the thickening radius unless the advance is 0 (a mark).
    pub advance: f32,
    /// True for a glyph whose advance width is 0 (a combining mark, rule "Marks" of 5.2).
    pub mark: bool,
    pub xmin: i32,
    pub ymin: i32,
    pub width: u32,
    pub height: u32,
}

/// The lines of a text before the canvas is known (FEATURE_TEXT 5.2 steps 1 to 7).
pub(crate) struct Lines {
    pub glyphs: Vec<LaidGlyph>,
    /// Width of every line: the pen after its last glyph minus the letter spacing and the gap value after it.
    pub widths: Vec<f32>,
    pub ascent: f32,
    pub descent: f32,
    /// Distance of two baselines (`new_line_size * line_spacing`).
    pub line_h: f32,
    /// The widest line.
    pub max_w: f32,
    /// The line box: `max(1, ceil(widest))` by `max(1, ceil(ascent - descent + (lines - 1) * line_h))`.
    pub box_w: i64,
    pub box_h: i64,
}

impl Lines {
    /// Alignment offset of line `line`: `round((widest - width) * {0, 0.5, 1})`.
    pub fn align_offset(&self, line: u32, align: TextAlign) -> i32 {
        let free = self.max_w - self.widths[line as usize];
        let factor = match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.0,
        };
        (free * factor).round() as i32
    }
}

/// The layout rule of FEATURE_TEXT 5.2, shared by both painters. `bold` is the thickening radius in raster pixels
/// (0 for the classic painter). The character loop walks the stored characters of `content` with their index: a
/// line feed ends a line, a tab is four spaces (its gap value follows the fourth), every other control character
/// is skipped. Positions, advances and kerning come from `fontdue` glyph indices. Without gap values and without
/// thickening every number is the number of round 3, bit for bit.
pub(crate) fn lay_out_lines(fonts: &[&fontdue::Font], spec: &TextSpec, bold: f32) -> Result<Lines, TextError> {
    if fonts.is_empty() {
        return Err(TextError::Font("no font to lay the text out with".to_owned()));
    }
    // `LaidGlyph::font` is a u8.
    let fonts = &fonts[..fonts.len().min(256)];
    let px = raster_px(spec);
    let letter_spacing = letter_spacing_px(spec, px);
    let line_spacing = if spec.line_spacing.is_finite() { spec.line_spacing.clamp(0.0, 16.0) } else { 1.0 };

    let (ascent, descent, new_line_size) = match fonts[0].horizontal_line_metrics(px) {
        Some(m) if m.ascent.is_finite() && m.descent.is_finite() && m.new_line_size.is_finite() => {
            (m.ascent, m.descent, m.new_line_size)
        }
        _ => (0.8 * px, -0.2 * px, 1.2 * px),
    };
    let line_h = new_line_size * line_spacing;

    // Empty content is one space (without gap values: it has no stored character).
    let (content, gaps): (&str, &[Gap]) = if spec.content.is_empty() { (" ", &[]) } else { (&spec.content, &spec.fx.gaps) };
    let gaps = GapLookup::new(gaps, px);

    struct LineState {
        pen: f32,
        prev: Option<(u16, usize)>,
        any: bool,
        /// The pen after the last glyph and its letter spacing, before the gap value that follows it.
        end: f32,
    }
    let mut glyphs: Vec<LaidGlyph> = Vec::with_capacity(content.len());
    let mut widths: Vec<f32> = Vec::new();
    let mut line: u32 = 0;
    let mut st = LineState { pen: 0.0, prev: None, any: false, end: 0.0 };

    let mut place = |ch: char, gap: f32, line: u32, st: &mut LineState| {
        let font = fonts.iter().position(|f| f.has_glyph(ch)).unwrap_or(0);
        let glyph = fonts[font].lookup_glyph_index(ch);
        if let Some((prev_glyph, prev_font)) = st.prev
            && prev_font == font
            && let Some(kern) = fonts[font].horizontal_kern_indexed(prev_glyph, glyph, px)
            && kern.is_finite()
        {
            st.pen += kern;
        }
        let m = fonts[font].metrics_indexed(glyph, px);
        let mark = m.advance_width == 0.0;
        let mut advance = m.advance_width;
        if bold != 0.0 && !mark {
            advance += 2.0 * bold;
        }
        glyphs.push(LaidGlyph {
            ch,
            font: font as u8,
            glyph,
            line,
            pen: st.pen,
            origin: st.pen.round() as i32,
            advance,
            mark,
            xmin: m.xmin,
            ymin: m.ymin,
            width: m.width as u32,
            height: m.height as u32,
        });
        st.pen += advance + letter_spacing;
        st.end = st.pen;
        if gap != 0.0 {
            st.pen += gap;
        }
        st.prev = Some((glyph, font));
    };
    let finish = |st: &LineState| -> f32 {
        let width = if st.any {
            // Neither the letter spacing nor a gap value after the last glyph widens the line.
            (st.end - letter_spacing).max(0.0)
        } else {
            0.0
        };
        if width.is_finite() { width } else { 0.0 }
    };
    for (index, ch) in content.chars().enumerate() {
        if ch == '\n' {
            widths.push(finish(&st));
            line += 1;
            // Every line adds `line_h` to the height: stop before an absurd line count costs time.
            if line as u64 > MAX_TEXT_SIDE {
                return Err(TextError::TooLarge { width: 0, height: u32::MAX });
            }
            st = LineState { pen: 0.0, prev: None, any: false, end: 0.0 };
            continue;
        }
        if ch == '\t' {
            for k in 0..4 {
                let gap = if k == 3 { gaps.px_after(index as u32) } else { 0.0 };
                place(' ', gap, line, &mut st);
            }
        } else if !ch.is_control() {
            place(ch, gaps.px_after(index as u32), line, &mut st);
        } else {
            continue;
        }
        st.any = true;
        if st.pen > 4.0 * MAX_TEXT_SIDE as f32 {
            return Err(TextError::TooLarge { width: u32::MAX, height: 0 });
        }
    }
    widths.push(finish(&st));

    let line_count = widths.len() as u32;
    let max_w = widths.iter().copied().fold(0.0f32, f32::max);
    // The line box: what section 6.8 calls the canvas without its padding.
    let box_w = (max_w.ceil() as i64).max(1);
    let box_h = ((ascent - descent + (line_count - 1) as f32 * line_h).ceil() as i64).max(1);
    Ok(Lines { glyphs, widths, ascent, descent, line_h, max_w, box_w, box_h })
}

/// Lays out `spec.content`. `fonts[0]` = chosen face, `fonts[1..]` = fallbacks; each char uses the first font that has the glyph.
///
/// Follows section 6.8. `glyphs` holds one entry per laid-out character in reading order, including characters
/// without ink (spaces: `width == height == 0`); `x`, `y` is the top-left corner of the glyph bitmap on the canvas.
/// A character no font has is drawn with the missing-glyph box of `fonts[0]`. Non-finite or out-of-range parameters are
/// sanitized: `line_spacing` to 0..=16 (1 when not finite), `letter_spacing` to -0.5..=4 em (0 when not finite).
///
/// `pad` is `ceil(outline width) + 2`, plus the number of pixels by which the widest glyph overhang leaves the line
/// box (0 for most text), so glyphs and their outline are never clipped; the canvas is the line box plus `pad` on
/// every side. `Err(TextError::Font)` without any font, `Err(TextError::TooLarge)` beyond 8192 px on a side.
pub fn layout_text(fonts: &[&fontdue::Font], spec: &TextSpec) -> Result<TextLayout, TextError> {
    let lines = lay_out_lines(fonts, spec, 0.0)?;
    let px = raster_px(spec);
    let outline = outline_px(spec, px);
    let (ascent, line_h, box_w, box_h) = (lines.ascent, lines.line_h, lines.box_w, lines.box_h);
    let items = &lines.glyphs;
    let line_count = lines.widths.len() as u32;

    // Glyph bitmap positions relative to the line box, and how far the ink leaves it.
    let align = |line: u32| -> i32 { lines.align_offset(line, spec.align) };
    let mut overhang = 0i64;
    let mut placed: Vec<(i64, i64)> = Vec::with_capacity(items.len());
    for item in items {
        let x = align(item.line) as i64 + item.origin as i64 + item.xmin as i64;
        let baseline = (ascent + item.line as f32 * line_h).round() as i64;
        let y = baseline - item.ymin as i64 - item.height as i64;
        if item.width > 0 && item.height > 0 {
            overhang = overhang
                .max(-x)
                .max(x + item.width as i64 - box_w)
                .max(-y)
                .max(y + item.height as i64 - box_h);
        }
        placed.push((x, y));
    }

    let pad = outline.ceil() as i64 + 2 + overhang.clamp(0, MAX_TEXT_SIDE as i64);
    let width = box_w + 2 * pad;
    let height = box_h + 2 * pad;
    if width as u64 > MAX_TEXT_SIDE || height as u64 > MAX_TEXT_SIDE {
        let clamp = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
        return Err(TextError::TooLarge { width: clamp(width), height: clamp(height) });
    }

    let glyphs = items
        .iter()
        .zip(&placed)
        .map(|(item, &(x, y))| PlacedGlyph {
            ch: item.ch,
            font_index: item.font,
            x: (pad + x) as i32,
            y: (pad + y) as i32,
            width: item.width,
            height: item.height,
        })
        .collect();
    Ok(TextLayout { glyphs, width: width as u32, height: height as u32, line_count, pad: pad as u32 })
}

/// Pixel size `[width, height]` of the image `rasterize_text` would produce.
pub fn measure_text(fonts: &[&fontdue::Font], spec: &TextSpec) -> Result<[u32; 2], TextError> {
    let layout = layout_text(fonts, spec)?;
    Ok([layout.width, layout.height])
}

#[cfg(test)]
pub(crate) mod testkit {
    use std::sync::Arc;

    use crate::ids::BlobId;
    use crate::text::{FontCache, FontRef, TEXT_PLACEMENT_PX, TextAlign, TextSpec};

    /// A left-aligned white spec without outline at `TEXT_PLACEMENT_PX`. The font reference is a dummy: the layout
    /// functions take the fonts as an argument.
    pub(crate) fn spec(content: &str) -> TextSpec {
        TextSpec {
            content: content.to_owned(),
            font: FontRef { blob: BlobId(0), face_index: 0, family: "Ubuntu".to_owned(), style: "Light".to_owned() },
            size_px: TEXT_PLACEMENT_PX,
            color: [255, 255, 255, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: crate::text::TextFx::default(),
        }
    }

    /// The bundled Ubuntu Light, parsed for raster size `px`.
    pub(crate) fn ubuntu(px: f32) -> Arc<fontdue::Font> {
        FontCache::default().fallback(px)
    }

    /// The bundled Hack Regular (a second font with other glyphs and metrics), parsed for 128 px.
    pub(crate) fn hack() -> Arc<fontdue::Font> {
        let settings = fontdue::FontSettings { collection_index: 0, scale: 128.0, load_substitutions: false };
        Arc::new(fontdue::Font::from_bytes(epaint_default_fonts::HACK_REGULAR, settings).unwrap())
    }

    /// The bundled Hack Regular parsed the way `FontCache` parses it for raster size `px` (scale bucket
    /// `px.next_power_of_two()` clamped to 32..=1024).
    pub(crate) fn hack_at(px: f32) -> Arc<fontdue::Font> {
        let scale = (px as u32).next_power_of_two().clamp(32, 1024) as f32;
        let settings = fontdue::FontSettings { collection_index: 0, scale, load_substitutions: false };
        Arc::new(fontdue::Font::from_bytes(epaint_default_fonts::HACK_REGULAR, settings).unwrap())
    }

    /// One case of the round-3 text baseline (`target\round4\perf\text_baseline.txt`, section 1).
    pub(crate) struct BaselineCase {
        pub name: &'static str,
        pub spec: TextSpec,
        /// Case 8: the stamp's own face is Hack, the fallback Ubuntu Light.
        pub hack_first: bool,
        pub size: [u32; 2],
        pub hash: u64,
    }

    /// The eight cases whose FNV-1a hashes P0 recorded on the round-3 code (TEXT 7.5 gate 5).
    pub(crate) fn baseline_cases() -> Vec<BaselineCase> {
        let base = |content: &str| {
            let mut s = spec(content);
            s.color = [220, 30, 40, 255];
            s
        };
        let mut centred = base("WTE 2026\nText");
        centred.align = TextAlign::Center;
        let mut outlined = base("WTE 2026");
        outlined.outline = Some(crate::text::Outline { width_em: 0.08, color: [0, 0, 0, 255] });
        let mut spaced = base("WTE\n2026");
        spaced.letter_spacing = 0.2;
        spaced.line_spacing = 1.7;
        let mut small = base("WTE 2026");
        small.size_px = 32.0;
        let mut large = base("WTE 2026");
        large.size_px = 512.0;
        let case = |name, spec, hack_first, size, hash| BaselineCase { name, spec, hack_first, size, hash };
        vec![
            case("one line", base("WTE 2026"), false, [582, 148], 0xe3a93b1b816d2d1a),
            case("three lines with an empty one", base("WTE\n\n2026"), false, [293, 442], 0xe1fc76eb05f3b8c2),
            case("two centred lines", centred, false, [582, 295], 0x6fbbb67237490b18),
            case("outline 8 %", outlined, false, [604, 170], 0xa7d0e8da1d2c6b79),
            case("letter spacing 20 % with line spacing 1.7", spaced, false, [370, 398], 0xc5e8eb28e41d63e2),
            case("32 px per em", small, false, [149, 40], 0xe7e0c4abe837733f),
            case("512 px per em", large, false, [2313, 578], 0xae4ea27ab966f86b),
            case("a character only the fallback font has", base("A\u{180}B"), true, [233, 153], 0xd8582e5ef8e36348),
        ]
    }

    /// The `fontdue` fonts of a baseline case, in layout order.
    pub(crate) fn baseline_fonts(case: &BaselineCase) -> Vec<Arc<fontdue::Font>> {
        let px = super::raster_px(&case.spec);
        if case.hack_first { vec![hack_at(px), ubuntu(px)] } else { vec![ubuntu(px)] }
    }

    /// The smallest character with ink that Hack has and Ubuntu Light does not.
    pub(crate) fn only_in_hack(ubuntu: &fontdue::Font, hack: &fontdue::Font) -> char {
        let mut chars: Vec<char> = hack.chars().keys().copied().filter(|c| !ubuntu.has_glyph(*c)).collect();
        chars.sort_unstable();
        chars.into_iter().find(|c| hack.metrics(*c, 128.0).width > 0).expect("a character only Hack has")
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::text::Outline;

    fn line_h(font: &fontdue::Font, px: f32) -> f32 {
        font.horizontal_line_metrics(px).unwrap().new_line_size
    }

    #[test]
    fn canvas_follows_the_formula() {
        let font = ubuntu(128.0);
        let s = spec("Hello");
        let layout = layout_text(&[&font], &s).unwrap();
        assert_eq!(layout.line_count, 1);
        assert_eq!(layout.glyphs.len(), 5);
        // "Hello" in Ubuntu Light stays inside its line box: the plain padding of 6.8.
        assert_eq!(layout.pad, 2);
        let m = font.horizontal_line_metrics(128.0).unwrap();
        assert_eq!(layout.height, (m.ascent - m.descent).ceil() as u32 + 4);
        let advance: f32 = "Hello".chars().map(|c| font.metrics(c, 128.0).advance_width).sum();
        let kern: f32 = "Hello"
            .chars()
            .zip("ello".chars())
            .filter_map(|(a, b)| font.horizontal_kern(a, b, 128.0))
            .sum();
        assert_eq!(layout.width, (advance + kern).ceil() as u32 + 4);
        assert_eq!(measure_text(&[&font], &s).unwrap(), [layout.width, layout.height]);
        // Glyphs advance to the right and lie inside the canvas.
        for pair in layout.glyphs.windows(2) {
            assert!(pair[0].x < pair[1].x);
        }
        for g in &layout.glyphs {
            assert!(g.x >= 0 && g.y >= 0);
            assert!(g.x as u32 + g.width <= layout.width && g.y as u32 + g.height <= layout.height);
        }
    }

    #[test]
    fn two_lines_are_taller_by_the_line_height() {
        let font = ubuntu(128.0);
        let one = layout_text(&[&font], &spec("Ag")).unwrap();
        let two = layout_text(&[&font], &spec("Ag\nAg")).unwrap();
        let three = layout_text(&[&font], &spec("Ag\r\nAg\nAg")).unwrap();
        assert_eq!((one.line_count, two.line_count, three.line_count), (1, 2, 3));
        let lh = line_h(&font, 128.0);
        assert!(((two.height - one.height) as f32 - lh).abs() <= 1.0, "{} vs {lh}", two.height - one.height);
        assert!(((three.height - one.height) as f32 - 2.0 * lh).abs() <= 1.0);
        assert_eq!(one.width, two.width);
        // The second line's glyphs sit one line height lower.
        assert!(((two.glyphs[2].y - two.glyphs[0].y) as f32 - lh).abs() <= 1.0);

        // line_spacing scales the distance.
        let mut wide = spec("Ag\nAg");
        wide.line_spacing = 2.0;
        let wide = layout_text(&[&font], &wide).unwrap();
        assert!(((wide.height - one.height) as f32 - 2.0 * lh).abs() <= 1.0);
    }

    #[test]
    fn alignment_shifts_the_short_line() {
        let font = ubuntu(128.0);
        let layouts: Vec<TextLayout> = [TextAlign::Left, TextAlign::Center, TextAlign::Right]
            .into_iter()
            .map(|align| {
                let mut s = spec("WWWWWW\ni");
                s.align = align;
                layout_text(&[&font], &s).unwrap()
            })
            .collect();
        let short = |l: &TextLayout| l.glyphs.last().unwrap().x;
        let (left, center, right) = (short(&layouts[0]), short(&layouts[1]), short(&layouts[2]));
        assert!(left < center && center < right, "{left} {center} {right}");
        assert!(((left + right) / 2 - center).abs() <= 1);
        // The long line does not move and the canvas keeps its size.
        for l in &layouts {
            assert_eq!(l.glyphs[0].x, layouts[0].glyphs[0].x);
            assert_eq!((l.width, l.height), (layouts[0].width, layouts[0].height));
        }
        // Right-aligned: the short line ends where the long one ends.
        let end = |g: &PlacedGlyph| g.x + g.width as i32;
        let long_end = end(&layouts[2].glyphs[5]);
        assert!((end(layouts[2].glyphs.last().unwrap()) - long_end).abs() <= font.metrics('W', 128.0).advance_width as i32 / 4);
    }

    #[test]
    fn letter_spacing_widens_by_an_em_fraction() {
        let font = ubuntu(128.0);
        let plain = layout_text(&[&font], &spec("abcd")).unwrap();
        let mut s = spec("abcd");
        s.letter_spacing = 0.25;
        let spaced = layout_text(&[&font], &s).unwrap();
        // Three gaps of 0.25 em = 32 px each (the spacing after the last glyph is not part of the line).
        assert!(((spaced.width - plain.width) as i32 - 96).abs() <= 1, "{} vs {}", spaced.width, plain.width);
        assert_eq!(spaced.height, plain.height);
        // The same fraction at another raster size gives the same proportions.
        let font64 = ubuntu(64.0);
        s.size_px = 64.0;
        let small = layout_text(&[&font64], &s).unwrap();
        let ratio = (spaced.width - 4) as f32 / (small.width - 4) as f32;
        assert!((ratio - 2.0).abs() < 0.05, "{ratio}");
    }

    #[test]
    fn outline_pads_the_canvas() {
        let font = ubuntu(128.0);
        let plain = layout_text(&[&font], &spec("Hello")).unwrap();
        let mut s = spec("Hello");
        s.outline = Some(Outline { width_em: 0.1, color: [0, 0, 0, 255] });
        let outlined = layout_text(&[&font], &s).unwrap();
        // ow = 12.8 px -> pad grows by ceil(12.8) = 13 on every side.
        assert_eq!(outlined.pad, plain.pad + 13);
        assert_eq!(outlined.width, plain.width + 26);
        assert_eq!(outlined.height, plain.height + 26);
        // width_em is clamped to 0.25.
        s.outline = Some(Outline { width_em: 3.0, color: [0, 0, 0, 255] });
        assert_eq!(layout_text(&[&font], &s).unwrap().pad, plain.pad + 32);
        s.outline = Some(Outline { width_em: f32::NAN, color: [0, 0, 0, 255] });
        assert_eq!(layout_text(&[&font], &s).unwrap().pad, plain.pad);
    }

    #[test]
    fn empty_text_is_one_space() {
        let font = ubuntu(128.0);
        let empty = layout_text(&[&font], &spec("")).unwrap();
        let space = layout_text(&[&font], &spec(" ")).unwrap();
        assert_eq!((empty.width, empty.height, empty.line_count), (space.width, space.height, 1));
        assert_eq!(empty.glyphs.len(), 1);
        assert_eq!(empty.glyphs[0].ch, ' ');
        assert_eq!((empty.glyphs[0].width, empty.glyphs[0].height), (0, 0));
        let one_line = layout_text(&[&font], &spec("x")).unwrap();
        assert_eq!(empty.height, one_line.height);
        assert_eq!(empty.width, font.metrics(' ', 128.0).advance_width.ceil() as u32 + 4);
        // Lines without characters keep their height; a line of control characters is empty.
        let blank_lines = layout_text(&[&font], &spec("\n\u{7}\n")).unwrap();
        assert_eq!(blank_lines.line_count, 3);
        assert_eq!(blank_lines.width, 1 + 4);
        assert!(blank_lines.glyphs.is_empty());
    }

    #[test]
    fn tabs_and_control_characters() {
        let font = ubuntu(128.0);
        let tab = layout_text(&[&font], &spec("a\tb")).unwrap();
        let spaces = layout_text(&[&font], &spec("a    b")).unwrap();
        assert_eq!(tab.width, spaces.width);
        assert_eq!(tab.glyphs.len(), 6);
        let with_control = layout_text(&[&font], &spec("a\u{1}b\rc")).unwrap();
        let without = layout_text(&[&font], &spec("abc")).unwrap();
        assert_eq!(with_control.width, without.width);
        assert_eq!(with_control.glyphs.len(), 3);
    }

    #[test]
    fn missing_glyphs_use_the_first_font_that_has_them() {
        let (ubuntu, hack) = (ubuntu(128.0), hack());
        let ch = only_in_hack(&ubuntu, &hack);
        let text = format!("a{ch}b");
        let layout = layout_text(&[&ubuntu, &hack], &spec(&text)).unwrap();
        let fonts: Vec<u8> = layout.glyphs.iter().map(|g| g.font_index).collect();
        assert_eq!(fonts, [0, 1, 0]);
        assert!(layout.glyphs[1].width > 0);
        // The fallback order is the argument order.
        let layout = layout_text(&[&hack, &ubuntu], &spec(&text)).unwrap();
        assert!(layout.glyphs.iter().all(|g| g.font_index == 0));
        // No font has it: the missing-glyph box of fonts[0].
        let layout = layout_text(&[&ubuntu], &spec(&text)).unwrap();
        assert_eq!(layout.glyphs[1].font_index, 0);
    }

    #[test]
    fn overhanging_glyphs_grow_the_padding_instead_of_being_clipped() {
        let font = ubuntu(512.0);
        // The printable ASCII character that reaches furthest left of its origin (the "j" in most fonts).
        let ch = (33u8..127).map(char::from).min_by_key(|c| font.metrics(*c, 512.0).xmin).unwrap();
        let mut s = spec(&ch.to_string());
        s.size_px = 512.0;
        let layout = layout_text(&[&font], &s).unwrap();
        let m = font.metrics(ch, 512.0);
        assert!(m.xmin < -2, "{ch:?} reaches left of its origin ({})", m.xmin);
        let g = layout.glyphs[0];
        assert!(g.x >= 2 && g.y >= 2, "{g:?}");
        assert!(g.x as u32 + g.width + 2 <= layout.width && g.y as u32 + g.height + 2 <= layout.height);
        assert!(layout.pad > 2);
        // Symmetric: the line box stays centred on the canvas.
        let box_w = layout.width - 2 * layout.pad;
        assert_eq!(box_w, (m.advance_width.ceil() as u32).max(1));
    }

    #[test]
    fn sizes_are_clamped_and_errors_reported() {
        let font = ubuntu(128.0);
        let mut s = spec("Ag");
        s.size_px = 1.0;
        let tiny = layout_text(&[&font], &s).unwrap();
        s.size_px = 8.0;
        let eight = layout_text(&[&font], &s).unwrap();
        assert_eq!((tiny.width, tiny.height), (eight.width, eight.height));
        s.size_px = 100_000.0;
        let huge = layout_text(&[&font], &s).unwrap();
        s.size_px = 512.0;
        let cap = layout_text(&[&font], &s).unwrap();
        assert_eq!((huge.width, huge.height), (cap.width, cap.height));
        s.size_px = f32::NAN;
        let nan = layout_text(&[&font], &s).unwrap();
        let default = layout_text(&[&font], &spec("Ag")).unwrap();
        assert_eq!((nan.width, nan.height), (default.width, default.height));

        assert!(matches!(layout_text(&[], &spec("x")), Err(TextError::Font(_))));
        // 200 W's at 128 px are far wider than 8192 px; 100 lines are far taller.
        let wide = layout_text(&[&font], &spec(&"W".repeat(200)));
        assert!(matches!(wide, Err(TextError::TooLarge { width, .. }) if width > 8192), "{wide:?}");
        let tall = layout_text(&[&font], &spec(&"x\n".repeat(100)));
        assert!(matches!(tall, Err(TextError::TooLarge { height, .. }) if height > 8192), "{tall:?}");
        assert!(matches!(layout_text(&[&font], &spec(&"\n".repeat(20_000))), Err(TextError::TooLarge { .. })));

        // Hostile spacing values do not panic.
        let mut odd = spec("ab\ncd");
        odd.letter_spacing = f32::INFINITY;
        odd.line_spacing = f32::NAN;
        assert!(layout_text(&[&font], &odd).is_ok());
        odd.letter_spacing = -100.0;
        odd.line_spacing = -3.0;
        let squeezed = layout_text(&[&font], &odd).unwrap();
        assert!(squeezed.width >= 1 && squeezed.height >= 1);
    }

    // ---- Round 4: gap spacing (FEATURE_TEXT 5.2, 8.1 "Gap spacing in the layout") ----

    use crate::text::Gap;

    fn with_gaps(content: &str, gaps: &[(u32, f32)]) -> TextSpec {
        let mut s = spec(content);
        s.fx.gaps = gaps.iter().map(|&(after, em)| Gap { after, em }).collect();
        s
    }

    /// Glyph x positions measured from the left edge of the line box.
    fn xs(layout: &TextLayout) -> Vec<i32> {
        layout.glyphs.iter().map(|g| g.x - layout.pad as i32).collect()
    }

    #[test]
    fn one_gap_moves_exactly_the_glyphs_after_it() {
        let font = ubuntu(128.0);
        let plain = layout_text(&[&font], &spec("ABCD\nEFGH")).unwrap();
        // A gap of 0.25 em after "B" (stored character 1) = 32 px at 128 px per em.
        let gapped_spec = with_gaps("ABCD\nEFGH", &[(1, 0.25)]);
        let gapped = layout_text(&[&font], &gapped_spec).unwrap();
        let (a, b) = (xs(&plain), xs(&gapped));
        assert_eq!(&a[..2], &b[..2], "A and B keep their places");
        assert_eq!([b[2] - a[2], b[3] - a[3]], [32, 32], "C and D move by the gap");
        assert_eq!(&a[4..], &b[4..], "the second line does not move");
        let (l0, l1) = (lay_out_lines(&[&font], &spec("ABCD\nEFGH"), 0.0).unwrap(), lay_out_lines(&[&font], &gapped_spec, 0.0).unwrap());
        assert!((l1.widths[0] - l0.widths[0] - 32.0).abs() < 1e-3, "the first line is 32 px longer");
        assert_eq!(l1.widths[1], l0.widths[1]);
        assert_eq!(gapped.width, plain.width + 32);
    }

    #[test]
    fn a_negative_gap_pulls_letters_together() {
        let font = ubuntu(128.0);
        let plain = xs(&layout_text(&[&font], &spec("ABCD\nEFGH")).unwrap());
        let pulled = xs(&layout_text(&[&font], &with_gaps("ABCD\nEFGH", &[(1, -0.25)])).unwrap());
        assert_eq!(&plain[..2], &pulled[..2]);
        assert_eq!([pulled[2] - plain[2], pulled[3] - plain[3]], [-32, -32]);
        assert_eq!(&plain[4..], &pulled[4..]);
    }

    #[test]
    fn a_gap_adds_to_letter_spacing_and_kerning() {
        let font = ubuntu(128.0);
        let mut s = with_gaps("AV", &[(0, 0.2)]);
        s.letter_spacing = 0.1;
        let lines = lay_out_lines(&[&font], &s, 0.0).unwrap();
        let kern = font.horizontal_kern('A', 'V', 128.0).unwrap_or(0.0);
        let want = font.metrics('A', 128.0).advance_width + kern + 0.3 * 128.0;
        let got = (lines.glyphs[1].origin - lines.glyphs[0].origin) as f32;
        assert!((got - want).abs() <= 1.0, "{got} vs {want} (kerning {kern})");
    }

    #[test]
    fn a_gap_after_the_last_character_of_a_line_changes_nothing() {
        let font = ubuntu(128.0);
        let plain = rasterize_text_for_test(&font, &spec("ABCD\nEFGH"));
        let gapped = rasterize_text_for_test(&font, &with_gaps("ABCD\nEFGH", &[(3, 0.5), (8, 2.0)]));
        assert_eq!(plain, gapped, "gaps after D and H: byte-identical picture");
    }

    fn rasterize_text_for_test(font: &fontdue::Font, s: &TextSpec) -> crate::pixel::PixelBuf {
        crate::text::rasterize_text(&[font], s).unwrap()
    }

    #[test]
    fn a_gap_after_a_tab_follows_its_fourth_space() {
        let font = ubuntu(128.0);
        let plain = xs(&layout_text(&[&font], &spec("a\tb")).unwrap());
        let gapped = xs(&layout_text(&[&font], &with_gaps("a\tb", &[(1, 0.5)])).unwrap());
        assert_eq!(plain.len(), 6);
        assert_eq!(&plain[..5], &gapped[..5], "a and the four spaces keep their places");
        assert_eq!(gapped[5] - plain[5], 64, "b follows the gap after the fourth space");
        // A gap before the tab belongs to the "a": the spaces move.
        let before = xs(&layout_text(&[&font], &with_gaps("a\tb", &[(0, 0.5)])).unwrap());
        assert_eq!(before[0], plain[0]);
        assert!(before[1..].iter().zip(&plain[1..]).all(|(g, p)| g - p == 64));
    }

    #[test]
    fn a_gap_ignores_the_fallback_font() {
        let (ubuntu, hack) = (ubuntu(128.0), hack());
        let ch = only_in_hack(&ubuntu, &hack);
        let text = format!("a{ch}b");
        let plain = lay_out_lines(&[&ubuntu, &hack], &spec(&text), 0.0).unwrap();
        assert_eq!(plain.glyphs[1].font, 1, "the middle character comes from the fallback");
        // A gap after the fallback character: the next glyph moves by the value.
        let after = lay_out_lines(&[&ubuntu, &hack], &with_gaps(&text, &[(1, 0.25)]), 0.0).unwrap();
        assert!(((after.glyphs[2].pen - plain.glyphs[2].pen) - 32.0).abs() <= 1.0);
        assert_eq!(after.glyphs[1].pen, plain.glyphs[1].pen);
        // A gap before it: the fallback glyph moves by the value too (a fraction of the em of the text).
        let before = lay_out_lines(&[&ubuntu, &hack], &with_gaps(&text, &[(0, 0.25)]), 0.0).unwrap();
        assert!(((before.glyphs[1].pen - plain.glyphs[1].pen) - 32.0).abs() <= 1.0);
    }

    #[test]
    fn centred_lines_stay_centred() {
        let font = ubuntu(128.0);
        let mut s = with_gaps("ABCD\nEFGH", &[(1, 0.5)]);
        s.align = TextAlign::Center;
        let layout = layout_text(&[&font], &s).unwrap();
        let lines = lay_out_lines(&[&font], &s, 0.0).unwrap();
        let box_w = (layout.width - 2 * layout.pad) as f32;
        for l in 0..2u32 {
            let left = lines.align_offset(l, TextAlign::Center) as f32;
            let centre = left + lines.widths[l as usize] / 2.0;
            assert!((centre - box_w / 2.0).abs() <= 1.0, "line {l}: centre {centre}, box {box_w}");
        }
        // The ink of each line keeps its centre (relative to the picture centre) against the text without the gap:
        // the letters before the gap move left by half the value, the letters after it right by half.
        let mut plain = s.clone();
        plain.fx.gaps.clear();
        let centres = |s: &TextSpec| -> [f32; 2] {
            let image = crate::text::rasterize_text(&[&font], s).unwrap();
            let half = image.height / 2;
            let ink = |rows: std::ops::Range<u32>| {
                let mut part = image.clone();
                for y in 0..image.height {
                    if !rows.contains(&y) {
                        part.row_mut(y).fill(0);
                    }
                }
                let r = crate::pixel_ops::content_bounds(part.view()).unwrap();
                (r.x0 + r.x1) as f32 / 2.0 - image.width as f32 / 2.0
            };
            [ink(0..half), ink(half..image.height)]
        };
        let (a, b) = (centres(&plain), centres(&s));
        assert!((a[0] - b[0]).abs() <= 1.0 && (a[1] - b[1]).abs() <= 1.0, "ink centres {a:?} -> {b:?}");
    }

    #[test]
    fn an_empty_gap_list_is_today() {
        for case in baseline_cases() {
            let fonts = baseline_fonts(&case);
            let refs: Vec<&fontdue::Font> = fonts.iter().map(|f| f.as_ref()).collect();
            let mut s = case.spec.clone();
            s.fx.gaps = Vec::new();
            let image = crate::text::rasterize_text(&refs, &s).unwrap();
            assert_eq!([image.width, image.height], case.size, "{}", case.name);
            assert_eq!(crate::math::fnv1a64(0, &image.data), case.hash, "{}: the round-3 picture", case.name);
            assert_eq!(measure_text(&refs, &s).unwrap(), case.size);
        }
    }

    #[test]
    fn hostile_gap_lists_do_not_panic() {
        let font = ubuntu(128.0);
        // Unsorted, duplicated, non-finite, out of range and beyond the text: the layout repairs on use.
        let s = with_gaps("AB\nCD", &[(4, 9.0), (0, f32::NAN), (1, 0.25), (0, 0.5), (99, 1.0), (2, 1.0)]);
        let lines = lay_out_lines(&[&font], &s, 0.0).unwrap();
        let plain = lay_out_lines(&[&font], &spec("AB\nCD"), 0.0).unwrap();
        // The later of two entries for "A" wins (0.5 em = 64 px); a NaN is no gap; "D" is last on its line.
        assert!(((lines.glyphs[1].pen - plain.glyphs[1].pen) - 64.0).abs() < 1e-3);
        assert_eq!(lines.widths[1], plain.widths[1]);
        let mut empty = with_gaps("", &[(0, 1.0)]);
        assert!(layout_text(&[&font], &empty).is_ok());
        empty.fx.gaps.push(Gap { after: 0, em: f32::INFINITY });
        assert!(layout_text(&[&font], &empty).is_ok());
    }
}
