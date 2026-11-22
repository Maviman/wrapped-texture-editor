// Owner: T1 text geometry (round 4). Glyph outlines (FEATURE_TEXT 5.3, 5.13): `ttf_parser::Face::outline_glyph`
// flattened into closed polylines in glyph space, and the character-to-glyph mapping the painter uses.

/// Flattening tolerance of curves, in raster pixels.
pub const FLATTEN_TOLERANCE_PX: f32 = 0.05;

/// Most lines one curve piece is cut into (a guard against absurd control points in a broken font).
const MAX_PIECES: f32 = 256.0;

/// The outline of one glyph at one size: closed polylines in glyph space (origin on the baseline at the pen position,
/// x to the right, y down, raster pixels). The last point of a contour is not repeated.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphOutline {
    pub contours: Vec<Vec<[f32; 2]>>,
}

impl GlyphOutline {
    /// True when the glyph has no ink (a space, a bitmap-only or colour glyph).
    pub fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.len() < 3)
    }

    /// Bounding box `[min, max]` of all points; `None` without points.
    pub fn bounds(&self) -> Option<[[f32; 2]; 2]> {
        let mut points = self.contours.iter().flatten();
        let first = *points.next()?;
        Some(points.fold([first, first], |[lo, hi], p| {
            [[lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]]
        }))
    }
}

/// Parses face `face_index` of a font file (only the table directory is read). `None` when the bytes are no font.
pub fn parse_face(bytes: &[u8], face_index: u32) -> Option<ttf_parser::Face<'_>> {
    ttf_parser::Face::parse(bytes, face_index).ok()
}

/// Collects the outline of one glyph: scales font units to raster pixels, turns y down and flattens curves.
struct Flattener {
    scale: f32,
    contours: Vec<Vec<[f32; 2]>>,
    current: Vec<[f32; 2]>,
}

impl Flattener {
    fn pt(&self, x: f32, y: f32) -> [f32; 2] {
        [x * self.scale, -y * self.scale]
    }

    fn last(&self) -> [f32; 2] {
        self.current.last().copied().unwrap_or([0.0, 0.0])
    }

    fn push(&mut self, p: [f32; 2]) {
        if self.current.last() != Some(&p) {
            self.current.push(p);
        }
    }

    fn end_contour(&mut self) {
        let mut contour = std::mem::take(&mut self.current);
        while contour.len() > 1 && contour.first() == contour.last() {
            contour.pop();
        }
        if contour.len() >= 3 {
            self.contours.push(contour);
        }
    }
}

/// Number of lines for a curve piece whose largest second difference is `m` (FEATURE_TEXT 5.3).
fn pieces(m: f32, factor: f32) -> usize {
    let n = (factor * m / FLATTEN_TOLERANCE_PX).sqrt().ceil();
    if n.is_finite() { n.clamp(1.0, MAX_PIECES) as usize } else { 1 }
}

fn len2(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

impl ttf_parser::OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.end_contour();
        let p = self.pt(x, y);
        self.current.push(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.push(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last();
        let p1 = self.pt(x1, y1);
        let p2 = self.pt(x, y);
        let m = len2([p0[0] - 2.0 * p1[0] + p2[0], p0[1] - 2.0 * p1[1] + p2[1]]);
        let n = pieces(m, 0.25);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let p = if i == n {
                p2
            } else {
                [
                    u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0],
                    u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1],
                ]
            };
            self.push(p);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last();
        let p1 = self.pt(x1, y1);
        let p2 = self.pt(x2, y2);
        let p3 = self.pt(x, y);
        let m = len2([p0[0] - 2.0 * p1[0] + p2[0], p0[1] - 2.0 * p1[1] + p2[1]])
            .max(len2([p1[0] - 2.0 * p2[0] + p3[0], p1[1] - 2.0 * p2[1] + p3[1]]));
        let n = pieces(m, 0.75);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let p = if i == n {
                p3
            } else {
                [
                    a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                    a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
                ]
            };
            self.push(p);
        }
    }

    fn close(&mut self) {
        self.end_contour();
    }
}

/// The outline of glyph `glyph` at `px` raster pixels per em, flattened with `FLATTEN_TOLERANCE_PX`. Empty for a
/// glyph without an outline, an index beyond the font, or a size that is not positive and finite.
pub fn glyph_outline(face: &ttf_parser::Face<'_>, glyph: u16, px: f32) -> GlyphOutline {
    if !(px.is_finite() && px > 0.0) {
        return GlyphOutline::default();
    }
    let scale = px / f32::from(face.units_per_em().max(1));
    let mut builder = Flattener { scale, contours: Vec::new(), current: Vec::new() };
    if face.outline_glyph(ttf_parser::GlyphId(glyph), &mut builder).is_none() {
        return GlyphOutline::default();
    }
    builder.end_contour();
    GlyphOutline { contours: builder.contours }
}

/// The glyph index of `ch` the way `fontdue` maps it: every cmap subtable in file order, the last mapping that is not
/// glyph 0 wins (FEATURE_TEXT 5.13). 0 when no subtable maps the character.
pub fn char_to_glyph(face: &ttf_parser::Face<'_>, ch: char) -> u16 {
    let Some(cmap) = face.tables().cmap else { return 0 };
    let mut found = 0;
    for subtable in cmap.subtables {
        if let Some(g) = subtable.glyph_index(ch as u32)
            && g.0 != 0
        {
            found = g.0;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> [(&'static str, &'static [u8]); 2] {
        [("Ubuntu Light", epaint_default_fonts::UBUNTU_LIGHT), ("Hack", epaint_default_fonts::HACK_REGULAR)]
    }

    fn fontdue_font(bytes: &[u8], px: f32) -> fontdue::Font {
        let settings = fontdue::FontSettings { collection_index: 0, scale: px, load_substitutions: false };
        fontdue::Font::from_bytes(bytes, settings).unwrap()
    }

    /// TEXT 8.1 `outline_bounds_match_fontdue_metrics`.
    #[test]
    fn outline_bounds_match_fontdue_metrics() {
        for (name, bytes) in fonts() {
            let face = parse_face(bytes, 0).unwrap();
            let font = fontdue_font(bytes, 128.0);
            for ch in ['A', 'g', 'j', 'W'] {
                let glyph = font.lookup_glyph_index(ch);
                assert_ne!(glyph, 0);
                let outline = glyph_outline(&face, glyph, 128.0);
                assert!(!outline.is_empty());
                let [lo, hi] = outline.bounds().unwrap();
                let b = font.metrics_indexed(glyph, 128.0).bounds;
                // fontdue: y up, ymin = bottom. Ours: y down.
                let close = |a: f32, b: f32| (a - b).abs() <= 1.0;
                assert!(close(lo[0], b.xmin) && close(hi[0], b.xmin + b.width), "{name} {ch}: x {lo:?} {hi:?} {b:?}");
                assert!(close(-hi[1], b.ymin) && close(-lo[1], b.ymin + b.height), "{name} {ch}: y {lo:?} {hi:?} {b:?}");
            }
            // A space has no ink; bad sizes give nothing.
            assert!(glyph_outline(&face, font.lookup_glyph_index(' '), 128.0).is_empty());
            assert!(glyph_outline(&face, font.lookup_glyph_index('A'), f32::NAN).is_empty());
            assert!(glyph_outline(&face, u16::MAX, 128.0).is_empty());
        }
    }

    /// TEXT 8.1 `coverage_matches_fontdue`: the same glyphs at 64 px, at least 99 % of the pixels within 8/255.
    ///
    /// Two references (contract request T1-1):
    /// - fontdue parsed at 1024 px per em, the quality check of 8.1 (limit 99 %): fontdue flattens curves while
    ///   parsing, with an error threshold of 3 px at the parse scale; at 1024 its flattening error at 64 px is below
    ///   0.2 px, so the comparison measures our outline and scan;
    /// - fontdue parsed at 64, the scale bucket the classic painter really uses for 64 px
    ///   (`font_catalog::scale_bucket`): its coarser curves miss 99 %; the measured share is guarded by a lower
    ///   limit so a regression of either painter shows.
    #[test]
    fn coverage_matches_fontdue() {
        for (parse_scale, limit) in [(1024.0f32, 0.99f64), (64.0, COVERAGE_LIMIT_AT_BUCKET_64)] {
            for (name, bytes) in fonts() {
                let share = coverage_share(bytes, parse_scale);
                eprintln!("coverage_matches_fontdue: {name}, reference parsed at {parse_scale}: {share:.4} within 8/255");
                assert!(share >= limit, "{name} at parse scale {parse_scale}: {share} < {limit}");
            }
        }
    }

    /// Lower limit of the comparison with fontdue parsed at the classic painter's bucket of 64 px (T1-1). Measured
    /// 2026-11-18: Ubuntu Light 0.9799, Hack 0.9960.
    const COVERAGE_LIMIT_AT_BUCKET_64: f64 = 0.975;

    /// Share of the pixels of ten glyphs at 64 px where our outline + scan is within 8/255 of fontdue's bitmap, with
    /// fontdue's font parsed at `parse_scale`.
    fn coverage_share(bytes: &[u8], parse_scale: f32) -> f64 {
        let face = parse_face(bytes, 0).unwrap();
        let font = fontdue_font(bytes, parse_scale);
        let (mut total, mut good) = (0usize, 0usize);
        for ch in ['A', 'g', 'j', 'W', 'e', '@', '8', 'Q', '%', 'ß'] {
            let glyph = font.lookup_glyph_index(ch);
            if glyph == 0 {
                continue;
            }
            let (m, bitmap) = font.rasterize_indexed(glyph, 64.0);
            let outline = glyph_outline(&face, glyph, 64.0);
            // Pixel (0, 0) of fontdue's bitmap starts at x = xmin and, y up, at ymin + height.
            let origin = [m.xmin as f32, -((m.ymin + m.height as i32) as f32)];
            let ours = crate::text::scan::fill_contours(&outline.contours, origin, m.width as u32, m.height as u32);
            for (a, b) in ours.data.iter().zip(&bitmap) {
                total += 1;
                if a.abs_diff(*b) <= 8 {
                    good += 1;
                }
            }
        }
        assert!(total > 10_000, "{total} pixels compared");
        good as f64 / total as f64
    }

    /// TEXT 8.1 `the_sample_maps_like_the_painter`: 300 mapped characters of both bundled fonts.
    #[test]
    fn the_sample_maps_like_the_painter() {
        for (name, bytes) in fonts() {
            let face = parse_face(bytes, 0).unwrap();
            let font = fontdue_font(bytes, 32.0);
            let mut chars: Vec<char> = font.chars().keys().copied().collect();
            chars.sort_unstable();
            let step = (chars.len() / 300).max(1);
            let picked: Vec<char> = chars.iter().copied().step_by(step).take(300).collect();
            assert!(picked.len() >= 300.min(chars.len()), "{name}");
            for ch in picked {
                assert_eq!(char_to_glyph(&face, ch), font.lookup_glyph_index(ch), "{name} {ch:?}");
            }
            for ch in ['\u{E000}', '\u{10FFFD}'] {
                assert_eq!(char_to_glyph(&face, ch), font.lookup_glyph_index(ch), "{name} {ch:?} unmapped");
            }
        }
        // A hand-made font with only a (1, 0) and a (3, 0) subtable: `Face::glyph_index` sees nothing, the painter's
        // map sees both, and the later subtable wins.
        let bytes = symbol_font();
        let face = parse_face(&bytes, 0).expect("the hand-made font parses");
        assert_eq!(face.glyph_index('A'), None);
        assert_eq!(char_to_glyph(&face, 'A'), 3);
        assert_eq!(char_to_glyph(&face, 'B'), 2);
        assert_eq!(char_to_glyph(&face, 'C'), 1);
        assert_eq!(char_to_glyph(&face, 'D'), 0);
        // fontdue builds the same table.
        if let Ok(font) =
            fontdue::Font::from_bytes(&bytes[..], fontdue::FontSettings { collection_index: 0, scale: 32.0, load_substitutions: false })
        {
            for ch in ['A', 'B', 'C', 'D'] {
                assert_eq!(char_to_glyph(&face, ch), font.lookup_glyph_index(ch), "{ch}");
            }
        }
    }

    /// A minimal font: `head`, `hhea`, `maxp` (4 glyphs) and a `cmap` with a Macintosh Roman (1, 0) and a Windows
    /// Symbol (3, 0) subtable, both format 0. (1, 0): 'A' -> 1, 'C' -> 1. (3, 0): 'A' -> 3, 'B' -> 2.
    fn symbol_font() -> Vec<u8> {
        fn be16(v: &mut Vec<u8>, x: u16) {
            v.extend_from_slice(&x.to_be_bytes());
        }
        fn be32(v: &mut Vec<u8>, x: u32) {
            v.extend_from_slice(&x.to_be_bytes());
        }
        let mut head = Vec::new();
        be32(&mut head, 0x0001_0000);
        be32(&mut head, 0x0001_0000);
        be32(&mut head, 0);
        be32(&mut head, 0x5F0F_3CF5);
        be16(&mut head, 0);
        be16(&mut head, 1000); // units per em
        head.extend_from_slice(&[0; 16]); // created, modified
        head.extend_from_slice(&[0; 8]); // bounds
        be16(&mut head, 0);
        be16(&mut head, 8);
        be16(&mut head, 2);
        be16(&mut head, 0); // index to location format
        be16(&mut head, 0);
        let mut hhea = Vec::new();
        be32(&mut hhea, 0x0001_0000);
        be16(&mut hhea, 800);
        be16(&mut hhea, (-200i16) as u16);
        be16(&mut hhea, 0);
        hhea.extend_from_slice(&[0; 24]);
        be16(&mut hhea, 0); // number of metrics
        let mut maxp = Vec::new();
        be32(&mut maxp, 0x0000_5000);
        be16(&mut maxp, 4);
        let format0 = |map: &[(u8, u8)]| {
            let mut v = Vec::new();
            be16(&mut v, 0);
            be16(&mut v, 262);
            be16(&mut v, 0);
            let mut ids = [0u8; 256];
            for &(c, g) in map {
                ids[c as usize] = g;
            }
            v.extend_from_slice(&ids);
            v
        };
        let mac = format0(&[(b'A', 1), (b'C', 1)]);
        let sym = format0(&[(b'A', 3), (b'B', 2)]);
        let mut cmap = Vec::new();
        be16(&mut cmap, 0);
        be16(&mut cmap, 2);
        be16(&mut cmap, 1);
        be16(&mut cmap, 0);
        be32(&mut cmap, 4 + 16);
        be16(&mut cmap, 3);
        be16(&mut cmap, 0);
        be32(&mut cmap, 4 + 16 + mac.len() as u32);
        cmap.extend_from_slice(&mac);
        cmap.extend_from_slice(&sym);

        let tables: [(&[u8; 4], &Vec<u8>); 4] = [(b"cmap", &cmap), (b"head", &head), (b"hhea", &hhea), (b"maxp", &maxp)];
        let mut out = Vec::new();
        be32(&mut out, 0x0001_0000);
        be16(&mut out, tables.len() as u16);
        be16(&mut out, 64);
        be16(&mut out, 2);
        be16(&mut out, 0);
        let mut offset = 12 + 16 * tables.len() as u32;
        let mut body = Vec::new();
        for (tag, data) in tables {
            out.extend_from_slice(tag);
            be32(&mut out, 0);
            be32(&mut out, offset);
            be32(&mut out, data.len() as u32);
            body.extend_from_slice(data);
            while body.len() % 4 != 0 {
                body.push(0);
            }
            offset = 12 + 16 * tables.len() as u32 + body.len() as u32;
        }
        out.extend_from_slice(&body);
        out
    }

    /// Largest distance from `curve` (sampled at 1,000 parameters) to the polyline `line`.
    fn worst_distance(line: &[[f32; 2]], curve: impl Fn(f32) -> [f32; 2]) -> f32 {
        let seg = |p: [f32; 2], s: [f32; 2], e: [f32; 2]| {
            let d = [e[0] - s[0], e[1] - s[1]];
            let l2 = d[0] * d[0] + d[1] * d[1];
            let t = if l2 > 0.0 { (((p[0] - s[0]) * d[0] + (p[1] - s[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
            len2([p[0] - s[0] - t * d[0], p[1] - s[1] - t * d[1]])
        };
        (0..=1000)
            .map(|i| {
                let p = curve(i as f32 / 1000.0);
                line.windows(2).map(|w| seg(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
            })
            .fold(0.0f32, f32::max)
    }

    /// The flattened polyline of a quadratic and of a cubic piece stays within the tolerance of the true curve
    /// (review of T1: the distance itself, not only the counts).
    #[test]
    fn flattened_curves_stay_within_the_tolerance() {
        use ttf_parser::OutlineBuilder;
        let slack = 1e-3;
        for (p0, p1, p2) in [([0.0f32, 0.0f32], [150.0f32, 400.0f32], [300.0f32, 0.0f32]), ([10.0, 5.0], [11.0, 60.0], [400.0, 7.0])] {
            let mut f = Flattener { scale: 1.0, contours: Vec::new(), current: Vec::new() };
            f.move_to(p0[0], p0[1]);
            f.quad_to(p1[0], p1[1], p2[0], p2[1]);
            let line = f.current.clone();
            assert!(line.len() > 10, "{}", line.len());
            let worst = worst_distance(&line, |t| {
                let u = 1.0 - t;
                [u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0], -(u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1])]
            });
            assert!(worst <= FLATTEN_TOLERANCE_PX + slack, "quadratic: {worst}");
        }
        for (p0, p1, p2, p3) in [
            ([0.0f32, 0.0f32], [0.0f32, 300.0f32], [400.0f32, 300.0f32], [400.0f32, 0.0f32]),
            ([0.0, 0.0], [500.0, 200.0], [-100.0, 200.0], [400.0, 0.0]),
        ] {
            let mut f = Flattener { scale: 1.0, contours: Vec::new(), current: Vec::new() };
            f.move_to(p0[0], p0[1]);
            f.curve_to(p1[0], p1[1], p2[0], p2[1], p3[0], p3[1]);
            let line = f.current.clone();
            assert!(line.len() > 10, "{}", line.len());
            let worst = worst_distance(&line, |t| {
                let u = 1.0 - t;
                let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                [
                    a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                    -(a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]),
                ]
            });
            assert!(worst <= FLATTEN_TOLERANCE_PX + slack, "cubic: {worst}");
        }
    }

    #[test]
    fn flattening_follows_the_tolerance() {
        // A quadratic piece with a large bend is cut into enough lines, a straight one into one.
        assert_eq!(pieces(0.0, 0.25), 1);
        assert_eq!(pieces(100.0, 0.25), 23);
        assert_eq!(pieces(f32::INFINITY, 0.75), 1);
        // The flattened "O" of Ubuntu: closed polylines without a repeated end point, finely cut (the distance to the
        // true curve is checked in `flattened_curves_stay_within_the_tolerance`).
        let face = parse_face(epaint_default_fonts::UBUNTU_LIGHT, 0).unwrap();
        let glyph = char_to_glyph(&face, 'O');
        let outline = glyph_outline(&face, glyph, 512.0);
        assert_eq!(outline.contours.len(), 2);
        for c in &outline.contours {
            assert_ne!(c.first(), c.last());
            assert!(c.len() > 40, "{}", c.len());
        }
    }
}
