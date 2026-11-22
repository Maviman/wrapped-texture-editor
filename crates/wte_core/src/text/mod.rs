//! Text stamps (ARCHITECTURE.md 5.13, 6.8): the shared types live here; font lookup, layout, rasterization and the
//! distance transform live in the sub-modules.
//!
//! Round 4 (FEATURE_TEXT 4.1): every new setting of a text sits in `TextSpec::fx` (`TextFx`): gap spacing between
//! single characters, thickening, slant, warp and a line the text runs along. `sanitize_fx` repairs them; it never
//! rejects and never panics. The painters (`raster::render_text`, `raster::text_frame`) and the geometry modules
//! (`outline`, `scan`, `warp`, `path`, `gaps`, `styled`, `frame`) are filled by the text packages of round 4.

use serde::{Deserialize, Serialize};

use crate::ids::BlobId;
use crate::pixel::PixelBuf;

pub mod edt;
pub mod font_catalog;
pub mod frame;
pub mod gaps;
pub mod layout;
pub mod outline;
pub mod path;
pub mod raster;
pub mod scan;
pub mod styled;
pub mod warp;

pub use edt::distance_to_ink;
pub use font_catalog::{
    DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE, FaceTraits, FontCache, FontCatalog, FontFaceInfo, default_font_bytes,
    face_traits, style_name,
};
pub use frame::{EmMap, Keep, place_anchor_at_centre, refit_placement};
pub use gaps::GapSlot;
pub use layout::{layout_text, measure_text};
pub use raster::{FaceFonts, TextFonts, rasterize_text, render_text, text_frame};
pub use warp::WarpMap;

/// The file extension (without the dot) of a font file, from its container signature: `ttf` (TrueType, also the
/// fallback), `otf` (CFF OpenType), `ttc` (collection), `woff`, `woff2`.
pub fn font_file_extension(bytes: &[u8]) -> &'static str {
    match bytes.get(..4) {
        Some(b"OTTO") => "otf",
        Some(b"ttcf") => "ttc",
        Some(b"wOFF") => "woff",
        Some(b"wOF2") => "woff2",
        _ => "ttf",
    }
}

/// Horizontal alignment of the lines of a text block.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Text outline.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Outline {
    /// Fraction of the em, `0..=0.25`; width in raster px = `width_em * size_px`.
    pub width_em: f32,
    pub color: [u8; 4],
}

/// A font face whose file bytes live in the document's `BlobStore`. `style` follows the naming rule of
/// `font_catalog::style_name`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontRef {
    pub blob: BlobId,
    pub face_index: u32,
    pub family: String,
    pub style: String,
}

/// Everything that defines the rasterized image of a text stamp.
#[derive(Clone, PartialEq, Debug)]
pub struct TextSpec {
    /// May contain `'\n'`.
    pub content: String,
    pub font: FontRef,
    /// Raster resolution (em size in px), clamped to `8..=512`. Chosen by the core for stamps (section 6.5); callers
    /// creating a spec use `TEXT_PLACEMENT_PX`.
    pub size_px: f32,
    pub color: [u8; 4],
    pub align: TextAlign,
    /// Multiplier, default 1.0.
    pub line_spacing: f32,
    /// Extra advance per glyph as a fraction of the em, default 0 (px = `letter_spacing * size_px`).
    pub letter_spacing: f32,
    pub outline: Option<Outline>,
    /// The round-4 settings (FEATURE_TEXT 4.1). `TextFx::default()` = a text exactly as in round 3.
    pub fx: TextFx,
}

/// `size_px` of a freshly built `TextSpec` (before the core picks the real resolution).
pub const TEXT_PLACEMENT_PX: f32 = 128.0;

/// One glyph bitmap placed on the text canvas.
#[derive(Copy, Clone, Debug)]
pub struct PlacedGlyph {
    pub ch: char,
    pub font_index: u8,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Result of `layout_text`: glyph positions and the canvas size.
#[derive(Clone, Debug)]
pub struct TextLayout {
    pub glyphs: Vec<PlacedGlyph>,
    pub width: u32,
    pub height: u32,
    pub line_count: u32,
    pub pad: u32,
}

// ---------------------------------------------------------------------------------------------------------------------
// Round 4: the text effects (FEATURE_TEXT 4.1)
// ---------------------------------------------------------------------------------------------------------------------

/// Smallest own spacing of one gap, in em (the lower limit `letter_spacing` has in the layout).
pub const GAP_MIN_EM: f32 = -0.5;
/// Largest own spacing of one gap, in em (the upper limit `letter_spacing` has in the layout).
pub const GAP_MAX_EM: f32 = 4.0;
/// Most points of a `PathShape::Curve`.
pub const MAX_PATH_POINTS: usize = 16;
/// Limit of every coordinate of a curve point and of a circle centre, in em. A curve across the whole range is 122
/// em wide and high with its letters: 3,904 px at 32 px per em, 15.2 megapixels, inside both picture limits.
pub const PATH_COORD_MAX_EM: f32 = 60.0;
/// Area limit of a styled picture (FEATURE_TEXT 5.10): 16.8 megapixels.
pub const MAX_STYLED_PIXELS: u64 = 1 << 24;
/// What the B button sets when the family has no bold face.
pub const FAUX_BOLD_EM: f32 = 0.02;
/// What the I button sets when the family has no italic face.
pub const FAUX_SLANT_DEG: f32 = 12.0;
/// Largest thickening, in em.
pub const MAX_BOLD_EM: f32 = 0.08;
/// Largest slant, in degrees (either way).
pub const MAX_SLANT_DEG: f32 = 45.0;
/// Limit of `TextPath::start`, in em (either way).
pub const PATH_START_MAX_EM: f32 = 100.0;
/// Limits of `PathShape::Circle::diameter`, in em.
pub const CIRCLE_DIAMETER_MIN_EM: f32 = 0.5;
pub const CIRCLE_DIAMETER_MAX_EM: f32 = 100.0;
/// The diameter a `Circle` gets when its stored value is not a finite number.
const CIRCLE_DIAMETER_DEFAULT_EM: f32 = 1.0;

/// The round-4 settings of a text. `TextFx::default()` is a text that uses no new feature and is drawn exactly as in
/// round 3 (the classic painter).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct TextFx {
    /// Own spacing of single gaps; sorted by `after`, one entry per character at most, no zeros (`sanitize_fx`).
    pub gaps: Vec<Gap>,
    /// `0..=MAX_BOLD_EM`: every edge grows by this share of the em; 0 = off.
    pub bold_em: f32,
    /// `-MAX_SLANT_DEG..=MAX_SLANT_DEG`, positive leans to the right; 0 = off.
    pub slant_deg: f32,
    pub warp: Option<Warp>,
    pub path: Option<TextPath>,
}

impl TextFx {
    /// True when the text uses no new feature (`== TextFx::default()`).
    pub fn is_default(&self) -> bool {
        *self == TextFx::default()
    }

    /// Thickening, slant, warp or a path: the styled painter draws the text. Gap values alone stay on the classic
    /// painter (FEATURE_TEXT 5.1).
    pub fn needs_outlines(&self) -> bool {
        self.bold_em != 0.0 || self.slant_deg != 0.0 || self.warp.is_some() || self.path.is_some()
    }
}

/// The extra space after the stored character number `after` (counted in `char`s of `content`), as a fraction of the
/// em like `letter_spacing`. It takes effect when another glyph follows on the same text line.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Gap {
    pub after: u32,
    /// `GAP_MIN_EM..=GAP_MAX_EM`.
    pub em: f32,
}

/// A warp of the letter shapes (FEATURE_TEXT 5.6).
#[derive(Copy, Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Warp {
    pub style: WarpStyle,
    /// `-1..=1`.
    pub bend: f32,
    /// Horizontal distortion, `-1..=1`.
    pub horizontal: f32,
    /// Vertical distortion, `-1..=1`.
    pub vertical: f32,
    /// Direction "Vertical": the style is applied along the other axis.
    pub vertical_axis: bool,
}

impl Warp {
    /// The style with every amount 0 (draws as the plain shape).
    pub fn new(style: WarpStyle) -> Warp {
        Warp { style, bend: 0.0, horizontal: 0.0, vertical: 0.0, vertical_axis: false }
    }
}

/// The 15 warp styles (the names of Photoshop's Warp Text; the formulas are our own, FEATURE_TEXT 5.6).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarpStyle {
    Arc,
    ArcLower,
    ArcUpper,
    Arch,
    Bulge,
    ShellLower,
    ShellUpper,
    Flag,
    Wave,
    Fish,
    Rise,
    Fisheye,
    Inflate,
    Squeeze,
    Twist,
}

impl WarpStyle {
    /// Every style in the order of the style list.
    pub const ALL: [WarpStyle; 15] = [
        WarpStyle::Arc,
        WarpStyle::ArcLower,
        WarpStyle::ArcUpper,
        WarpStyle::Arch,
        WarpStyle::Bulge,
        WarpStyle::ShellLower,
        WarpStyle::ShellUpper,
        WarpStyle::Flag,
        WarpStyle::Wave,
        WarpStyle::Fish,
        WarpStyle::Rise,
        WarpStyle::Fisheye,
        WarpStyle::Inflate,
        WarpStyle::Squeeze,
        WarpStyle::Twist,
    ];

    /// The name stored in a project file (`"arc_lower"`). Never renamed.
    pub fn key(self) -> &'static str {
        match self {
            WarpStyle::Arc => "arc",
            WarpStyle::ArcLower => "arc_lower",
            WarpStyle::ArcUpper => "arc_upper",
            WarpStyle::Arch => "arch",
            WarpStyle::Bulge => "bulge",
            WarpStyle::ShellLower => "shell_lower",
            WarpStyle::ShellUpper => "shell_upper",
            WarpStyle::Flag => "flag",
            WarpStyle::Wave => "wave",
            WarpStyle::Fish => "fish",
            WarpStyle::Rise => "rise",
            WarpStyle::Fisheye => "fisheye",
            WarpStyle::Inflate => "inflate",
            WarpStyle::Squeeze => "squeeze",
            WarpStyle::Twist => "twist",
        }
    }

    /// The style stored under `key`; `None` for an unknown or empty name.
    pub fn from_key(key: &str) -> Option<WarpStyle> {
        WarpStyle::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The name shown in the panel (`"Arc Lower"`).
    pub fn label(self) -> &'static str {
        match self {
            WarpStyle::Arc => "Arc",
            WarpStyle::ArcLower => "Arc Lower",
            WarpStyle::ArcUpper => "Arc Upper",
            WarpStyle::Arch => "Arch",
            WarpStyle::Bulge => "Bulge",
            WarpStyle::ShellLower => "Shell Lower",
            WarpStyle::ShellUpper => "Shell Upper",
            WarpStyle::Flag => "Flag",
            WarpStyle::Wave => "Wave",
            WarpStyle::Fish => "Fish",
            WarpStyle::Rise => "Rise",
            WarpStyle::Fisheye => "Fisheye",
            WarpStyle::Inflate => "Inflate",
            WarpStyle::Squeeze => "Squeeze",
            WarpStyle::Twist => "Twist",
        }
    }
}

/// A line the text runs along (FEATURE_TEXT 5.5).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct TextPath {
    pub shape: PathShape,
    /// Slides the text along the line, in em, `-PATH_START_MAX_EM..=PATH_START_MAX_EM` (open lines clamp it further
    /// to one line length in the layout).
    pub start: f32,
    /// "Other side": the text stands on the other side of the line and reads the other way.
    pub flip: bool,
}

/// The kinds of line.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum PathShape {
    /// `bend` `-1..=1`: the share of a full circle the widest text line covers.
    Arc { bend: f32 },
    /// `diameter` in em, `CIRCLE_DIAMETER_MIN_EM..=CIRCLE_DIAMETER_MAX_EM`; `center` relative to the anchor, in em,
    /// each coordinate `-PATH_COORD_MAX_EM..=PATH_COORD_MAX_EM`.
    Circle { diameter: f32, center: [f32; 2] },
    /// 2 to `MAX_PATH_POINTS` points in em relative to the anchor, y down, each coordinate
    /// `-PATH_COORD_MAX_EM..=PATH_COORD_MAX_EM`.
    Curve { points: Vec<[f32; 2]> },
}

impl PathShape {
    /// The name stored in a project file: `"arc"`, `"circle"`, `"curve"`.
    pub fn key(&self) -> &'static str {
        match self {
            PathShape::Arc { .. } => "arc",
            PathShape::Circle { .. } => "circle",
            PathShape::Curve { .. } => "curve",
        }
    }
}

/// Where the text sits in its full-resolution picture. All lengths in raster pixels of that picture.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct TextFrame {
    /// Picture size.
    pub size: [u32; 2],
    /// The anchor (origin of text space) in the picture.
    pub anchor_px: [f32; 2],
    /// Raster pixels per em.
    pub em_px: f32,
    /// Width and height of the straight line box.
    pub box_px: [f32; 2],
    /// y of the first baseline relative to the anchor (negative = above it).
    pub baseline0_px: f32,
    /// The point that a change of spacing keeps in place (FEATURE_TEXT 5.11): on the line box at the alignment side
    /// (left edge, centre or right edge), at the height of the anchor; equal to `anchor_px` for a text with a warp or
    /// a path.
    pub align_px: [f32; 2],
    /// Circle only, else 0: the largest share of its own circle that a text line covers; above 1 the text runs over
    /// its own start.
    pub circle_fill: f32,
}

/// How many pixels a picture of a styled text may have (FEATURE_TEXT 5.8). The classic painter ignores it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RasterQuality {
    Full,
    /// The same rectangle with at most `max_pixels` pixels.
    Preview { max_pixels: u32 },
}

/// A rendered text: the picture, the full-resolution frame (also for a preview) and whether the picture is the full
/// one.
#[derive(Clone, Debug)]
pub struct TextRaster {
    pub image: PixelBuf,
    pub frame: TextFrame,
    pub full: bool,
}

/// `v` when it is finite, else `fallback`; then clamped to `lo..=hi`.
fn repair(v: &mut f32, fallback: f32, lo: f32, hi: f32) {
    *v = if v.is_finite() { v.clamp(lo, hi) } else { fallback };
}

/// The one field list behind `TextSpec::floats` and `TextSpec::floats_mut`, so the two visitors cannot disagree.
/// `$f` receives the name and a reference (`&f32`, or `&mut f32` when `mut` is given).
macro_rules! visit_spec_floats {
    ($spec:ident, $f:ident $(, $m:tt)?) => {{
        $f("the text size", & $($m)? $spec.size_px);
        $f("the line spacing", & $($m)? $spec.line_spacing);
        $f("the letter spacing", & $($m)? $spec.letter_spacing);
        if let Some(outline) = & $($m)? $spec.outline {
            $f("the outline width", & $($m)? outline.width_em);
        }
        for gap in & $($m)? $spec.fx.gaps {
            $f("a gap", & $($m)? gap.em);
        }
        $f("the thickening", & $($m)? $spec.fx.bold_em);
        $f("the slant", & $($m)? $spec.fx.slant_deg);
        if let Some(warp) = & $($m)? $spec.fx.warp {
            $f("the warp bend", & $($m)? warp.bend);
            $f("the horizontal distortion", & $($m)? warp.horizontal);
            $f("the vertical distortion", & $($m)? warp.vertical);
        }
        if let Some(path) = & $($m)? $spec.fx.path {
            $f("the path start", & $($m)? path.start);
            match & $($m)? path.shape {
                PathShape::Arc { bend } => $f("the arc bend", bend),
                PathShape::Circle { diameter, center } => {
                    $f("the circle diameter", diameter);
                    for c in center {
                        $f("the circle centre", c);
                    }
                }
                PathShape::Curve { points } => {
                    for point in points {
                        for c in point {
                            $f("a curve point", c);
                        }
                    }
                }
            }
        }
    }};
}

impl TextSpec {
    /// A spec with today's defaults: `TEXT_PLACEMENT_PX`, black, left-aligned, line spacing 1, no letter spacing, no
    /// outline, no effects.
    pub fn new(content: &str, font: FontRef) -> TextSpec {
        TextSpec {
            content: content.to_owned(),
            font,
            size_px: TEXT_PLACEMENT_PX,
            color: [0, 0, 0, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: TextFx::default(),
        }
    }

    /// Repairs `fx` (FEATURE_TEXT 4.1): never rejects, never panics. Non-finite numbers become the default and every
    /// value is clamped to its range; a curve keeps at most `MAX_PATH_POINTS` points and is dropped with fewer than
    /// 2; gap entries that are not finite, lie beyond the text, are keyed to a line feed, to a skipped control
    /// character, to a zero-width joiner or to the left character of a joined pair are removed, values are clamped,
    /// of two entries for one character the later wins, zeros are removed and the list is sorted. The eight fields
    /// of round 3 are not touched (the layout sanitizes them on use).
    pub fn sanitize_fx(&mut self) {
        let fx = &mut self.fx;
        repair(&mut fx.bold_em, 0.0, 0.0, MAX_BOLD_EM);
        repair(&mut fx.slant_deg, 0.0, -MAX_SLANT_DEG, MAX_SLANT_DEG);
        if let Some(warp) = &mut fx.warp {
            repair(&mut warp.bend, 0.0, -1.0, 1.0);
            repair(&mut warp.horizontal, 0.0, -1.0, 1.0);
            repair(&mut warp.vertical, 0.0, -1.0, 1.0);
        }
        if let Some(path) = &mut fx.path {
            repair(&mut path.start, 0.0, -PATH_START_MAX_EM, PATH_START_MAX_EM);
            let coord = |c: &mut f32| repair(c, 0.0, -PATH_COORD_MAX_EM, PATH_COORD_MAX_EM);
            match &mut path.shape {
                PathShape::Arc { bend } => repair(bend, 0.0, -1.0, 1.0),
                PathShape::Circle { diameter, center } => {
                    repair(diameter, CIRCLE_DIAMETER_DEFAULT_EM, CIRCLE_DIAMETER_MIN_EM, CIRCLE_DIAMETER_MAX_EM);
                    center.iter_mut().for_each(coord);
                }
                PathShape::Curve { points } => {
                    points.truncate(MAX_PATH_POINTS);
                    points.iter_mut().flatten().for_each(coord);
                }
            }
            if matches!(&path.shape, PathShape::Curve { points } if points.len() < 2) {
                fx.path = None;
            }
        }

        // Gaps: which stored characters may carry a value.
        let chars: Vec<char> = self.content.chars().collect();
        let allowed = |after: u32| -> bool {
            let Some(&ch) = chars.get(after as usize) else { return false };
            if ch == '\n' || (ch.is_control() && ch != '\t') || ch == '\u{200D}' {
                return false;
            }
            !chars.get(after as usize + 1).is_some_and(|&next| gaps::is_joined(next))
        };
        let mut kept: Vec<Gap> = Vec::with_capacity(fx.gaps.len());
        for gap in fx.gaps.drain(..) {
            if !gap.em.is_finite() || !allowed(gap.after) {
                continue;
            }
            let gap = Gap { after: gap.after, em: gap.em.clamp(GAP_MIN_EM, GAP_MAX_EM) };
            // The later of two entries for one character wins.
            match kept.iter_mut().find(|g| g.after == gap.after) {
                Some(existing) => *existing = gap,
                None => kept.push(gap),
            }
        }
        kept.retain(|g| g.em != 0.0);
        kept.sort_by_key(|g| g.after);
        fx.gaps = kept;
    }

    /// Every float of the spec, read only, in a fixed order with a name for messages (the save check, FEATURE_TEXT
    /// 4.3 item 6): the text size, line spacing, letter spacing, the outline width, every gap value, thickening,
    /// slant, the three warp numbers and every path number.
    pub fn floats(&self, f: &mut dyn FnMut(&'static str, f32)) {
        let spec = self;
        let mut visit = |name: &'static str, v: &f32| f(name, *v);
        visit_spec_floats!(spec, visit);
    }

    /// The same floats as `floats`, in the same order, as places that can be repaired.
    pub fn floats_mut(&mut self, f: &mut dyn FnMut(&'static str, &mut f32)) {
        let spec = self;
        let mut visit = |name: &'static str, v: &mut f32| f(name, v);
        visit_spec_floats!(spec, visit, mut);
    }

    /// True when `self` and `other` differ, and differ only in `letter_spacing` and / or `fx.gaps` (a change of
    /// spacing alone, FEATURE_TEXT 5.11).
    pub fn differs_only_in_spacing(&self, other: &TextSpec) -> bool {
        if self == other {
            return false;
        }
        let mut a = self.clone();
        a.letter_spacing = other.letter_spacing;
        a.fx.gaps.clone_from(&other.fx.gaps);
        a == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> FontRef {
        FontRef { blob: BlobId(0), face_index: 0, family: "Ubuntu".to_owned(), style: "Light".to_owned() }
    }

    /// A spec with an outline, 2 gaps, a warp and a curve of 3 points.
    fn full_spec() -> TextSpec {
        let mut spec = TextSpec::new("WTE 2026", font());
        spec.outline = Some(Outline { width_em: 0.05, color: [0, 0, 0, 255] });
        spec.fx = TextFx {
            gaps: vec![Gap { after: 2, em: 0.3 }, Gap { after: 5, em: -0.05 }],
            bold_em: 0.02,
            slant_deg: 12.0,
            warp: Some(Warp { style: WarpStyle::Arc, bend: 0.5, horizontal: 0.0, vertical: 0.1, vertical_axis: false }),
            path: Some(TextPath {
                shape: PathShape::Curve { points: vec![[-2.0, 0.3], [0.0, -0.2], [2.0, 0.3]] },
                start: 0.0,
                flip: false,
            }),
        };
        spec
    }

    #[test]
    fn warp_style_keys_round_trip() {
        let mut keys = std::collections::HashSet::new();
        for style in WarpStyle::ALL {
            assert_eq!(WarpStyle::from_key(style.key()), Some(style));
            assert!(keys.insert(style.key()));
            assert!(!style.label().is_empty());
            // The serde name is the key, so a project file and the panel agree.
            assert_eq!(serde_json::to_value(style).unwrap(), serde_json::Value::from(style.key()));
        }
        assert_eq!(WarpStyle::ArcLower.label(), "Arc Lower");
        assert_eq!(WarpStyle::from_key("ribbon"), None);
        assert_eq!(WarpStyle::from_key(""), None);
    }

    #[test]
    fn default_fx_uses_no_feature() {
        let spec = TextSpec::new("A", font());
        assert!(spec.fx.is_default() && !spec.fx.needs_outlines());
        let mut gaps_only = spec.fx.clone();
        gaps_only.gaps.push(Gap { after: 0, em: 0.2 });
        assert!(!gaps_only.is_default() && !gaps_only.needs_outlines(), "gaps stay on the classic painter");
        for fx in [
            TextFx { bold_em: 0.01, ..TextFx::default() },
            TextFx { slant_deg: -3.0, ..TextFx::default() },
            TextFx { warp: Some(Warp::new(WarpStyle::Flag)), ..TextFx::default() },
            TextFx { path: Some(TextPath { shape: PathShape::Arc { bend: 0.0 }, start: 0.0, flip: false }), ..TextFx::default() },
        ] {
            assert!(!fx.is_default() && fx.needs_outlines(), "{fx:?}");
        }
    }

    #[test]
    fn float_list_is_complete() {
        let mut spec = full_spec();
        let mut read: Vec<&'static str> = Vec::new();
        spec.floats(&mut |name, _| read.push(name));
        let mut written: Vec<&'static str> = Vec::new();
        spec.floats_mut(&mut |name, v| {
            written.push(name);
            *v += 0.0;
        });
        assert_eq!(read.len(), 18, "{read:?}");
        assert_eq!(read, written);
        // The values come in the same order through both visitors.
        let mut values = Vec::new();
        spec.floats(&mut |_, v| values.push(v.to_bits()));
        let mut index = 0;
        spec.floats_mut(&mut |_, v| {
            assert_eq!(v.to_bits(), values[index]);
            index += 1;
        });
        // A circle has its diameter and two centre coordinates; an arc its bend.
        spec.fx.path = Some(TextPath { shape: PathShape::Circle { diameter: 4.0, center: [0.0, 2.0] }, start: 1.0, flip: true });
        let mut n = 0;
        spec.floats(&mut |_, _| n += 1);
        assert_eq!(n, 15, "a circle has 3 path numbers where the curve had 6");
    }

    #[test]
    fn sanitize_repairs_the_effects() {
        let mut spec = TextSpec::new("WTE 2026", font());
        spec.fx = TextFx {
            gaps: Vec::new(),
            bold_em: f32::NAN,
            slant_deg: 90.0,
            warp: Some(Warp { style: WarpStyle::Twist, bend: f32::INFINITY, horizontal: -7.0, vertical: f32::NAN, vertical_axis: true }),
            path: Some(TextPath {
                shape: PathShape::Curve { points: vec![[1e6, -1e6]; 40] },
                start: 1e9,
                flip: false,
            }),
        };
        spec.sanitize_fx();
        assert_eq!((spec.fx.bold_em, spec.fx.slant_deg), (0.0, MAX_SLANT_DEG));
        assert_eq!(
            spec.fx.warp,
            Some(Warp { style: WarpStyle::Twist, bend: 0.0, horizontal: -1.0, vertical: 0.0, vertical_axis: true })
        );
        let path = spec.fx.path.clone().unwrap();
        assert_eq!(path.start, PATH_START_MAX_EM);
        let PathShape::Curve { points } = path.shape else { panic!() };
        assert_eq!(points.len(), MAX_PATH_POINTS);
        assert!(points.iter().all(|p| *p == [PATH_COORD_MAX_EM, -PATH_COORD_MAX_EM]));
        let mut all_finite = true;
        spec.floats(&mut |_, v| all_finite &= v.is_finite());
        assert!(all_finite);

        // A curve with one point is dropped; a circle is clamped; an arc at bend 0 stays.
        spec.fx.path = Some(TextPath { shape: PathShape::Curve { points: vec![[0.0, 0.0]] }, start: 0.0, flip: false });
        spec.sanitize_fx();
        assert_eq!(spec.fx.path, None);
        spec.fx.path =
            Some(TextPath { shape: PathShape::Circle { diameter: f32::NAN, center: [0.0, 500.0] }, start: 0.0, flip: false });
        spec.sanitize_fx();
        assert_eq!(spec.fx.path.as_ref().unwrap().shape, PathShape::Circle { diameter: 1.0, center: [0.0, PATH_COORD_MAX_EM] });
        spec.fx.path = Some(TextPath { shape: PathShape::Arc { bend: 0.0 }, start: 0.0, flip: false });
        spec.fx.warp = Some(Warp::new(WarpStyle::Arc));
        let before = spec.fx.clone();
        spec.sanitize_fx();
        assert_eq!(spec.fx, before, "valid plain shapes stay");
        // The eight fields of round 3 are not touched.
        spec.size_px = f32::NAN;
        spec.sanitize_fx();
        assert!(spec.size_px.is_nan());
    }

    #[test]
    fn non_finite_and_out_of_range_gaps_are_sanitized() {
        let gap = |after: u32, em: f32| Gap { after, em };
        // Characters: A B \n C \r D e U+0301 x U+200D y
        let mut spec = TextSpec::new("AB\nC\rDe\u{301}x\u{200D}y", font());
        spec.fx.gaps = vec![
            gap(1, f32::NAN),     // non-finite
            gap(0, f32::INFINITY), // non-finite
            gap(5, 9.0),          // D: clamped to 4
            gap(3, -3.0),         // C: clamped to -0.5
            gap(2, 0.5),          // line feed
            gap(4, 0.5),          // carriage return (skipped)
            gap(6, 0.5),          // "e" of a joined pair
            gap(8, 0.5),          // x: its right neighbour is a joiner
            gap(9, 0.5),          // the joiner
            gap(11, 0.5),         // beyond the text
            gap(10, 0.25),        // last character: kept
            gap(10, 0.75),        // the later wins
            gap(1, 0.0),          // zero
        ];
        spec.sanitize_fx();
        assert_eq!(spec.fx.gaps, vec![gap(3, GAP_MIN_EM), gap(5, GAP_MAX_EM), gap(10, 0.75)]);
        // A tab may carry a value.
        let mut spec = TextSpec::new("a\tb", font());
        spec.fx.gaps = vec![gap(1, 0.1)];
        spec.sanitize_fx();
        assert_eq!(spec.fx.gaps, vec![gap(1, 0.1)]);
        // Indices count characters, not bytes.
        let mut spec = TextSpec::new("\u{e4}\u{20ac}x", font());
        spec.fx.gaps = vec![gap(2, 0.1), gap(1, 0.2), gap(3, 0.3)];
        spec.sanitize_fx();
        assert_eq!(spec.fx.gaps, vec![gap(1, 0.2), gap(2, 0.1)]);
    }

    #[test]
    fn differs_only_in_spacing_is_strict() {
        let base = full_spec();
        assert!(!base.differs_only_in_spacing(&base));
        let mut a = base.clone();
        a.fx.gaps[0].em = 0.5;
        assert!(base.differs_only_in_spacing(&a));
        let mut b = base.clone();
        b.letter_spacing = 0.1;
        b.fx.gaps.clear();
        assert!(base.differs_only_in_spacing(&b));
        for change in [
            (|s: &mut TextSpec| s.content.push('!')) as fn(&mut TextSpec),
            |s| s.align = TextAlign::Right,
            |s| s.font.style = "Bold".to_owned(),
            |s| s.fx.bold_em = 0.0,
            |s| s.size_px = 64.0,
        ] {
            let mut c = base.clone();
            c.letter_spacing = 0.2;
            change(&mut c);
            assert!(!base.differs_only_in_spacing(&c));
        }
    }
}