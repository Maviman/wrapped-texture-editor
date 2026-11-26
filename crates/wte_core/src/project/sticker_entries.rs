//! Manifest entries of project format 4 (STICKER_PLAN 2.11, ARCHITECTURE 7.2): stickers, stamp styles, adjustments,
//! shapes and PhotoCraft links, and their conversions to and from the core types.
//!
//! The rules of `manifest.rs` hold here too: new optional fields are `#[serde(default)]`, enum variant names are never
//! renamed, floats are stored exactly as the core holds them (a round trip is bit-exact), and unknown extra fields are
//! ignored. Values read from a file are repaired by the model's `sanitize` after conversion, never trusted.

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::ids::{BlobId, StickerId, StickerLayerId};
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::project::manifest::{FileRef, TextEntry};
use crate::sticker::LayerTransform;
use crate::sticker::adjust::Adjustment;
use crate::sticker::model::ArtboardShape;
use crate::sticker::shape::{Paint, ShapeKind, ShapeSpec};
use crate::sticker::style::{
    Bevel, ColorOverlay, Contour, Glow, Gradient, GradientOverlay, Shadow, Stroke, StrokePos, StyleStack,
};

fn is_false(b: &bool) -> bool {
    !*b
}

// ----- stickers -----

/// One sticker (`StickerDoc`) of `Manifest::stickers`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StickerEntry {
    pub id: StickerId,
    pub name: String,
    /// The artboard in layer space: x, y, w, h.
    pub artboard: [i32; 4],
    #[serde(default)]
    pub shape: ArtboardShapeEntry,
    /// Saved even when no stamp uses it (a tray sticker).
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep: bool,
    pub next_layer: u32,
    /// Bottom to top.
    pub layers: Vec<StickerLayerEntry>,
}

/// `ArtboardShape` (`"kind": "rect" | "rounded_rect" | "ellipse"`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtboardShapeEntry {
    #[default]
    Rect,
    RoundedRect {
        radius: f32,
    },
    Ellipse,
}

/// One layer of a sticker (`StickerLayer`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StickerLayerEntry {
    pub id: StickerLayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    /// translate x, translate y, scale x, scale y, rotation (radians).
    pub transform: [f32; 5],
    pub content: StickerContentEntry,
    #[serde(default, skip_serializing_if = "is_false")]
    pub clip: bool,
    /// The mask PNG (`stickers/k<id>/m<layer>.png`, 8-bit grey, the size of the picture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<FileRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<AdjustmentEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub styles: Option<StyleStackEntry>,
}

/// What a sticker layer shows (`"kind": "picture" | "text" | "shape"`).
#[allow(clippy::large_enum_variant)] // entries are few
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StickerContentEntry {
    /// Exactly one of the two: an unchanged imported image (a stored blob), or edited pixels
    /// (`stickers/k<id>/p<layer>.png`, straight RGBA).
    Picture {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blob: Option<BlobId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file: Option<FileRef>,
    },
    Text(TextEntry),
    Shape(ShapeEntry),
}

/// A `ShapeSpec`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShapeEntry {
    /// The geometry (`ShapeSpec::kind`; named `form` because `kind` is the content tag).
    pub form: ShapeFormEntry,
    pub size: [f32; 2],
    pub fill: PaintEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<ShapeOutlineEntry>,
}

/// A `ShapeKind` (`"type": ...`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShapeFormEntry {
    Rect,
    RoundedRect { radius: f32 },
    Ellipse,
    Polygon { sides: u32 },
    Star { points: u32, inner: f32 },
    Line { width: f32 },
}

/// A `Paint` (`"type": "none" | "solid" | "linear"`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaintEntry {
    None,
    Solid { color: [u8; 4] },
    Linear { gradient: Vec<StopEntry>, angle_deg: f32 },
}

/// The outline of a shape: colour and width (centred on the edge).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ShapeOutlineEntry {
    pub color: [u8; 4],
    pub width: f32,
}

/// One stop of a `Gradient`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct StopEntry {
    /// 0..=1.
    pub at: f32,
    pub color: [u8; 4],
}

fn stops_of(g: &Gradient) -> Vec<StopEntry> {
    g.stops.iter().map(|&(at, color)| StopEntry { at, color }).collect()
}

fn gradient_of(stops: &[StopEntry]) -> Gradient {
    Gradient { stops: stops.iter().map(|s| (s.at, s.color)).collect() }
}

// ----- adjustments -----

/// An `Adjustment` (`"kind": ...`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdjustmentEntry {
    BrightnessContrast { brightness: f32, contrast: f32 },
    HueSaturation { hue_deg: f32, saturation: f32, lightness: f32, colorize: bool },
    Levels { in_black: u8, in_white: u8, gamma: f32, out_black: u8, out_white: u8 },
    Invert,
    BlackWhite,
    Posterize { levels: u8 },
    Threshold { level: u8 },
}

impl From<&Adjustment> for AdjustmentEntry {
    fn from(a: &Adjustment) -> AdjustmentEntry {
        match *a {
            Adjustment::BrightnessContrast { brightness, contrast } => AdjustmentEntry::BrightnessContrast { brightness, contrast },
            Adjustment::HueSaturation { hue_deg, saturation, lightness, colorize } => {
                AdjustmentEntry::HueSaturation { hue_deg, saturation, lightness, colorize }
            }
            Adjustment::Levels { in_black, in_white, gamma, out_black, out_white } => {
                AdjustmentEntry::Levels { in_black, in_white, gamma, out_black, out_white }
            }
            Adjustment::Invert => AdjustmentEntry::Invert,
            Adjustment::BlackWhite => AdjustmentEntry::BlackWhite,
            Adjustment::Posterize { levels } => AdjustmentEntry::Posterize { levels },
            Adjustment::Threshold { level } => AdjustmentEntry::Threshold { level },
        }
    }
}

impl AdjustmentEntry {
    /// The adjustment, not yet sanitized.
    pub fn to_adjustment(&self) -> Adjustment {
        match *self {
            AdjustmentEntry::BrightnessContrast { brightness, contrast } => Adjustment::BrightnessContrast { brightness, contrast },
            AdjustmentEntry::HueSaturation { hue_deg, saturation, lightness, colorize } => {
                Adjustment::HueSaturation { hue_deg, saturation, lightness, colorize }
            }
            AdjustmentEntry::Levels { in_black, in_white, gamma, out_black, out_white } => {
                Adjustment::Levels { in_black, in_white, gamma, out_black, out_white }
            }
            AdjustmentEntry::Invert => Adjustment::Invert,
            AdjustmentEntry::BlackWhite => Adjustment::BlackWhite,
            AdjustmentEntry::Posterize { levels } => Adjustment::Posterize { levels },
            AdjustmentEntry::Threshold { level } => Adjustment::Threshold { level },
        }
    }
}

// ----- styles -----

/// A `StyleStack`: every member optional (absent = `None`); an entry with `"on": false` is kept.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct StyleStackEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contour: Option<ContourEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<StrokeEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drop_shadow: Option<ShadowEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<ShadowEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_glow: Option<GlowEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_glow: Option<GlowEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_overlay: Option<ColorOverlayEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient_overlay: Option<GradientOverlayEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<BevelEntry>,
}

/// A `Contour`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ContourEntry {
    pub on: bool,
    pub width: f32,
    pub smooth: f32,
    pub fill_holes: bool,
    pub color: [u8; 4],
    pub opacity: f32,
}

/// `StrokePos` (`"outside" | "inside" | "center"`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StrokePosEntry {
    Outside,
    Inside,
    Center,
}

/// A `Stroke`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct StrokeEntry {
    pub on: bool,
    pub width: f32,
    pub position: StrokePosEntry,
    pub color: [u8; 4],
    pub opacity: f32,
}

/// A `Shadow` (drop or inner).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ShadowEntry {
    pub on: bool,
    pub color: [u8; 4],
    pub opacity: f32,
    pub angle_deg: f32,
    pub distance: f32,
    pub spread: f32,
    pub size: f32,
}

/// A `Glow` (outer or inner).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct GlowEntry {
    pub on: bool,
    pub color: [u8; 4],
    pub opacity: f32,
    pub spread: f32,
    pub size: f32,
}

/// A `ColorOverlay`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ColorOverlayEntry {
    pub on: bool,
    pub color: [u8; 4],
    pub opacity: f32,
}

/// A `GradientOverlay`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GradientOverlayEntry {
    pub on: bool,
    pub gradient: Vec<StopEntry>,
    pub angle_deg: f32,
    pub opacity: f32,
}

/// A `Bevel`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BevelEntry {
    pub on: bool,
    pub size: f32,
    pub depth: f32,
    pub angle_deg: f32,
    pub altitude_deg: f32,
    pub highlight: [u8; 4],
    pub shadow: [u8; 4],
    pub opacity: f32,
}

fn shadow_entry(s: &Shadow) -> ShadowEntry {
    ShadowEntry {
        on: s.on,
        color: s.color,
        opacity: s.opacity,
        angle_deg: s.angle_deg,
        distance: s.distance,
        spread: s.spread,
        size: s.size,
    }
}

fn shadow_of(e: &ShadowEntry) -> Shadow {
    Shadow {
        on: e.on,
        color: e.color,
        opacity: e.opacity,
        angle_deg: e.angle_deg,
        distance: e.distance,
        spread: e.spread,
        size: e.size,
    }
}

fn glow_entry(g: &Glow) -> GlowEntry {
    GlowEntry { on: g.on, color: g.color, opacity: g.opacity, spread: g.spread, size: g.size }
}

fn glow_of(e: &GlowEntry) -> Glow {
    Glow { on: e.on, color: e.color, opacity: e.opacity, spread: e.spread, size: e.size }
}

impl From<&StyleStack> for StyleStackEntry {
    fn from(s: &StyleStack) -> StyleStackEntry {
        StyleStackEntry {
            contour: s.contour.as_ref().map(|c| ContourEntry {
                on: c.on,
                width: c.width,
                smooth: c.smooth,
                fill_holes: c.fill_holes,
                color: c.color,
                opacity: c.opacity,
            }),
            stroke: s.stroke.as_ref().map(|k| StrokeEntry {
                on: k.on,
                width: k.width,
                position: match k.position {
                    StrokePos::Outside => StrokePosEntry::Outside,
                    StrokePos::Inside => StrokePosEntry::Inside,
                    StrokePos::Center => StrokePosEntry::Center,
                },
                color: k.color,
                opacity: k.opacity,
            }),
            drop_shadow: s.drop_shadow.as_ref().map(shadow_entry),
            inner_shadow: s.inner_shadow.as_ref().map(shadow_entry),
            outer_glow: s.outer_glow.as_ref().map(glow_entry),
            inner_glow: s.inner_glow.as_ref().map(glow_entry),
            color_overlay: s.color_overlay.as_ref().map(|o| ColorOverlayEntry { on: o.on, color: o.color, opacity: o.opacity }),
            gradient_overlay: s.gradient_overlay.as_ref().map(|o| GradientOverlayEntry {
                on: o.on,
                gradient: stops_of(&o.gradient),
                angle_deg: o.angle_deg,
                opacity: o.opacity,
            }),
            bevel: s.bevel.as_ref().map(|b| BevelEntry {
                on: b.on,
                size: b.size,
                depth: b.depth,
                angle_deg: b.angle_deg,
                altitude_deg: b.altitude_deg,
                highlight: b.highlight,
                shadow: b.shadow,
                opacity: b.opacity,
            }),
        }
    }
}

impl StyleStackEntry {
    /// The entry of a stack, `None` for the default stack (nothing is written).
    pub fn of(styles: &StyleStack) -> Option<StyleStackEntry> {
        (!styles.is_default()).then(|| StyleStackEntry::from(styles))
    }

    /// The style stack this entry describes, sanitized (2.3 limits).
    pub fn to_styles(&self) -> StyleStack {
        let mut s = StyleStack {
            contour: self.contour.map(|c| Contour {
                on: c.on,
                width: c.width,
                smooth: c.smooth,
                fill_holes: c.fill_holes,
                color: c.color,
                opacity: c.opacity,
            }),
            stroke: self.stroke.map(|k| Stroke {
                on: k.on,
                width: k.width,
                position: match k.position {
                    StrokePosEntry::Outside => StrokePos::Outside,
                    StrokePosEntry::Inside => StrokePos::Inside,
                    StrokePosEntry::Center => StrokePos::Center,
                },
                color: k.color,
                opacity: k.opacity,
            }),
            drop_shadow: self.drop_shadow.as_ref().map(shadow_of),
            inner_shadow: self.inner_shadow.as_ref().map(shadow_of),
            outer_glow: self.outer_glow.as_ref().map(glow_of),
            inner_glow: self.inner_glow.as_ref().map(glow_of),
            color_overlay: self.color_overlay.map(|o| ColorOverlay { on: o.on, color: o.color, opacity: o.opacity }),
            gradient_overlay: self.gradient_overlay.as_ref().map(|o| GradientOverlay {
                on: o.on,
                gradient: gradient_of(&o.gradient),
                angle_deg: o.angle_deg,
                opacity: o.opacity,
            }),
            bevel: self.bevel.map(|b| Bevel {
                on: b.on,
                size: b.size,
                depth: b.depth,
                angle_deg: b.angle_deg,
                altitude_deg: b.altitude_deg,
                highlight: b.highlight,
                shadow: b.shadow,
                opacity: b.opacity,
            }),
        };
        s.sanitize();
        s
    }
}

/// Every float of a style stack that is written to a manifest, with a name for error messages (the
/// `stamp_floats` pattern of `save.rs`).
pub(crate) fn style_floats(s: &StyleStack, out: &mut Vec<(&'static str, f32)>) {
    if let Some(c) = &s.contour {
        out.extend([("the contour width", c.width), ("the contour smoothing", c.smooth), ("the contour opacity", c.opacity)]);
    }
    if let Some(k) = &s.stroke {
        out.extend([("the stroke width", k.width), ("the stroke opacity", k.opacity)]);
    }
    for (name, sh) in [("the drop shadow", &s.drop_shadow), ("the inner shadow", &s.inner_shadow)] {
        if let Some(sh) = sh {
            out.extend([
                (name, sh.opacity),
                (name, sh.angle_deg),
                (name, sh.distance),
                (name, sh.spread),
                (name, sh.size),
            ]);
        }
    }
    for (name, g) in [("the outer glow", &s.outer_glow), ("the inner glow", &s.inner_glow)] {
        if let Some(g) = g {
            out.extend([(name, g.opacity), (name, g.spread), (name, g.size)]);
        }
    }
    if let Some(o) = &s.color_overlay {
        out.push(("the colour overlay opacity", o.opacity));
    }
    if let Some(o) = &s.gradient_overlay {
        out.extend(o.gradient.stops.iter().map(|st| ("a gradient stop", st.0)));
        out.extend([("the gradient overlay angle", o.angle_deg), ("the gradient overlay opacity", o.opacity)]);
    }
    if let Some(b) = &s.bevel {
        let name = "the bevel";
        out.extend([(name, b.size), (name, b.depth), (name, b.angle_deg), (name, b.altitude_deg), (name, b.opacity)]);
    }
}

// ----- shapes, transforms, artboards -----

impl From<&ShapeSpec> for ShapeEntry {
    fn from(s: &ShapeSpec) -> ShapeEntry {
        ShapeEntry {
            form: match s.kind {
                ShapeKind::Rect => ShapeFormEntry::Rect,
                ShapeKind::RoundedRect { radius } => ShapeFormEntry::RoundedRect { radius },
                ShapeKind::Ellipse => ShapeFormEntry::Ellipse,
                ShapeKind::Polygon { sides } => ShapeFormEntry::Polygon { sides },
                ShapeKind::Star { points, inner } => ShapeFormEntry::Star { points, inner },
                ShapeKind::Line { width } => ShapeFormEntry::Line { width },
            },
            size: s.size.to_array(),
            fill: match &s.fill {
                Paint::None => PaintEntry::None,
                Paint::Solid(color) => PaintEntry::Solid { color: *color },
                Paint::Linear { gradient, angle_deg } => PaintEntry::Linear { gradient: stops_of(gradient), angle_deg: *angle_deg },
            },
            outline: s.outline.map(|(color, width)| ShapeOutlineEntry { color, width }),
        }
    }
}

impl ShapeEntry {
    /// The shape this entry describes, not yet sanitized.
    pub fn to_shape(&self) -> ShapeSpec {
        ShapeSpec {
            kind: match self.form {
                ShapeFormEntry::Rect => ShapeKind::Rect,
                ShapeFormEntry::RoundedRect { radius } => ShapeKind::RoundedRect { radius },
                ShapeFormEntry::Ellipse => ShapeKind::Ellipse,
                ShapeFormEntry::Polygon { sides } => ShapeKind::Polygon { sides },
                ShapeFormEntry::Star { points, inner } => ShapeKind::Star { points, inner },
                ShapeFormEntry::Line { width } => ShapeKind::Line { width },
            },
            size: Vec2::from_array(self.size),
            fill: match &self.fill {
                PaintEntry::None => Paint::None,
                PaintEntry::Solid { color } => Paint::Solid(*color),
                PaintEntry::Linear { gradient, angle_deg } => Paint::Linear { gradient: gradient_of(gradient), angle_deg: *angle_deg },
            },
            outline: self.outline.map(|o| (o.color, o.width)),
        }
    }
}

/// Every float of a shape, named.
pub(crate) fn shape_floats(s: &ShapeSpec, out: &mut Vec<(&'static str, f32)>) {
    out.extend(s.size.to_array().map(|v| ("the shape size", v)));
    match s.kind {
        ShapeKind::RoundedRect { radius } => out.push(("the corner radius", radius)),
        ShapeKind::Star { inner, .. } => out.push(("the star's inner radius", inner)),
        ShapeKind::Line { width } => out.push(("the line width", width)),
        ShapeKind::Rect | ShapeKind::Ellipse | ShapeKind::Polygon { .. } => {}
    }
    if let Paint::Linear { gradient, angle_deg } = &s.fill {
        out.extend(gradient.stops.iter().map(|st| ("a gradient stop", st.0)));
        out.push(("the fill angle", *angle_deg));
    }
    if let Some((_, w)) = s.outline {
        out.push(("the outline width", w));
    }
}

/// Every float of an adjustment, named.
pub(crate) fn adjustment_floats(a: &Adjustment, out: &mut Vec<(&'static str, f32)>) {
    match *a {
        Adjustment::BrightnessContrast { brightness, contrast } => {
            out.extend([("the brightness", brightness), ("the contrast", contrast)]);
        }
        Adjustment::HueSaturation { hue_deg, saturation, lightness, .. } => {
            out.extend([("the hue", hue_deg), ("the saturation", saturation), ("the lightness", lightness)]);
        }
        Adjustment::Levels { gamma, .. } => out.push(("the levels gamma", gamma)),
        Adjustment::Invert | Adjustment::BlackWhite | Adjustment::Posterize { .. } | Adjustment::Threshold { .. } => {}
    }
}

/// `[tx, ty, sx, sy, rotation]` of a layer transform.
pub fn transform_entry(t: &LayerTransform) -> [f32; 5] {
    [t.translate.x, t.translate.y, t.scale.x, t.scale.y, t.rotation]
}

/// The layer transform of `[tx, ty, sx, sy, rotation]`, not yet sanitized.
pub fn transform_of(v: [f32; 5]) -> LayerTransform {
    LayerTransform { translate: Vec2::new(v[0], v[1]), scale: Vec2::new(v[2], v[3]), rotation: v[4] }
}

impl From<ArtboardShape> for ArtboardShapeEntry {
    fn from(s: ArtboardShape) -> ArtboardShapeEntry {
        match s {
            ArtboardShape::Rect => ArtboardShapeEntry::Rect,
            ArtboardShape::RoundedRect { radius } => ArtboardShapeEntry::RoundedRect { radius },
            ArtboardShape::Ellipse => ArtboardShapeEntry::Ellipse,
        }
    }
}

impl ArtboardShapeEntry {
    /// The shape this entry describes, not yet sanitized.
    pub fn to_shape(self) -> ArtboardShape {
        match self {
            ArtboardShapeEntry::Rect => ArtboardShape::Rect,
            ArtboardShapeEntry::RoundedRect { radius } => ArtboardShape::RoundedRect { radius },
            ArtboardShapeEntry::Ellipse => ArtboardShape::Ellipse,
        }
    }
}

/// `[x, y, w, h]` of an artboard.
pub fn artboard_entry(r: &IRect) -> [i32; 4] {
    [r.x0, r.y0, r.width() as i32, r.height() as i32]
}

/// The artboard of `[x, y, w, h]`; `None` unless both sides are 1..=`MAX_ARTBOARD` and the corners fit an `i32`.
pub fn artboard_of(v: [i32; 4]) -> Option<IRect> {
    let [x, y, w, h] = v;
    let max = crate::sticker::MAX_ARTBOARD as i32;
    if !(1..=max).contains(&w) || !(1..=max).contains(&h) {
        return None;
    }
    let (x1, y1) = (x.checked_add(w)?, y.checked_add(h)?);
    Some(IRect::new(x, y, x1, y1))
}

// ----- editor links (STICKER_PLAN 5.3; written by P5c) -----

fn default_editor() -> String {
    crate::link::EDITOR_PHOTOCRAFT.to_owned()
}

/// One link of `Manifest::links` (STICKER_PLAN 5.3 `StickerLink` plus the archive entry of the linked file). P4 wrote
/// none; `editor` (13.1) is optional with the default `"photocraft"`, and `editor_native` is also read under its
/// revision-2 name `pc_native`, so a format-4 file of any earlier build loads.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LinkEntry {
    pub sticker: StickerId,
    pub file: LinkFileEntry,
    /// The editor's key: `"photocraft"`, `"photoshop"` or `"other"`.
    #[serde(default = "default_editor")]
    pub editor: String,
    pub mode: LinkModeEntry,
    /// PSD pixels per layer-space pixel.
    pub scale: f32,
    pub synced: FileStampEntry,
    pub synced_sticker: u64,
    #[serde(default)]
    pub layer_map: Vec<LinkLayerEntry>,
    pub next_lyid: u32,
    #[serde(default, alias = "pc_native", skip_serializing_if = "is_false")]
    pub editor_native: bool,
    /// `links/k<id:05>.psd` inside the project, when stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
}

/// Where the linked file lives (`"kind": "in_folder" | "external"`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkFileEntry {
    /// A file name in the project's links folder.
    InFolder { name: String },
    External { path: String },
}

/// How the linked file comes back (`"layers" | "merged"`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkModeEntry {
    Layers,
    Merged,
}

/// Size, modification time and CRC-32 of a linked file's content.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStampEntry {
    pub size: u64,
    /// Nanoseconds since the Unix epoch, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_unix_ns: Option<u64>,
    pub crc32: u32,
}

/// One PSD layer id <-> sticker layer pair of a link.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LinkLayerEntry {
    pub lyid: u32,
    /// `0` for the placeholder layer of a sticker without layers.
    pub layer: StickerLayerId,
    /// The layer name written or read (P5c; empty in files without it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub origin: [i32; 2],
    pub size: [u32; 2],
    pub pixels_crc: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_crc: Option<u32>,
}

impl LinkEntry {
    /// The entry of `link` (sticker `sticker`); `archive` = the stored copy of the file, if any.
    pub fn of(sticker: StickerId, link: &crate::link::StickerLink, archive: Option<String>) -> LinkEntry {
        use crate::link::{LinkFile, LinkMode};
        LinkEntry {
            sticker,
            file: match &link.file {
                LinkFile::InFolder(name) => LinkFileEntry::InFolder { name: name.clone() },
                LinkFile::External(path) => LinkFileEntry::External { path: path.to_string_lossy().into_owned() },
            },
            editor: link.editor.clone(),
            mode: match link.mode {
                LinkMode::Layers => LinkModeEntry::Layers,
                LinkMode::Merged => LinkModeEntry::Merged,
            },
            scale: link.scale,
            synced: FileStampEntry { size: link.synced.size, modified_unix_ns: link.synced.modified_ns, crc32: link.synced.crc32 },
            synced_sticker: link.synced_sticker,
            layer_map: link
                .layer_map
                .iter()
                .map(|e| LinkLayerEntry {
                    lyid: e.lyid,
                    layer: e.layer,
                    name: e.name.clone(),
                    origin: e.origin,
                    size: e.size,
                    pixels_crc: e.pixels_crc,
                    mask_crc: e.mask_crc,
                })
                .collect(),
            next_lyid: link.next_lyid,
            editor_native: link.editor_native,
            archive,
        }
    }

    /// The link this entry describes; `None` (with the reason) when it cannot be one: an `InFolder` name that is not
    /// a plain file name, a scale that is not a finite number >= 1, or two map entries with one `lyid`.
    pub fn to_link(&self) -> Result<crate::link::StickerLink, &'static str> {
        use crate::link::{FileStamp, LinkFile, LinkLayer, LinkMode, StickerLink};
        let file = match &self.file {
            LinkFileEntry::InFolder { name } => {
                let plain = !name.is_empty()
                    && std::path::Path::new(name).file_name().is_some_and(|n| n == std::ffi::OsStr::new(name))
                    && !name.contains(['/', '\\', ':']);
                if !plain {
                    return Err("the linked file name is not a plain file name");
                }
                LinkFile::InFolder(name.clone())
            }
            LinkFileEntry::External { path } => LinkFile::External(path.into()),
        };
        if !(self.scale.is_finite() && self.scale >= 1.0) {
            return Err("the link scale is not a number of at least 1");
        }
        let mut ids: Vec<u32> = self.layer_map.iter().map(|e| e.lyid).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|p| p[0] == p[1]) {
            return Err("two layers of the link have the same id");
        }
        let top = ids.last().copied().unwrap_or(0);
        Ok(StickerLink {
            file,
            editor: self.editor.clone(),
            mode: match self.mode {
                LinkModeEntry::Layers => LinkMode::Layers,
                LinkModeEntry::Merged => LinkMode::Merged,
            },
            scale: self.scale,
            synced: FileStamp { size: self.synced.size, modified_ns: self.synced.modified_unix_ns, crc32: self.synced.crc32 },
            synced_sticker: self.synced_sticker,
            layer_map: self
                .layer_map
                .iter()
                .map(|e| LinkLayer {
                    lyid: e.lyid,
                    layer: e.layer,
                    name: e.name.clone(),
                    origin: e.origin,
                    size: e.size,
                    pixels_crc: e.pixels_crc,
                    mask_crc: e.mask_crc,
                })
                .collect(),
            next_lyid: self.next_lyid.max(top.saturating_add(1)).max(1),
            editor_native: self.editor_native,
            problem: None,
        })
    }
}
