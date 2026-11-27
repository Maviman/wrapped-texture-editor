//! Serde schema of `manifest.json` inside a `.wtep` project (ARCHITECTURE.md 7.2) and the conversions between the
//! manifest entries and the document's stamp types.
//!
//! New optional fields must be `#[serde(default)]`; enum variant names are never renamed. Plain arrays are used instead
//! of glam's serde support so the JSON stays stable. Every float is stored exactly as the core holds it (same unit, same
//! `f32` bits), so a round trip is bit-exact.

use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::blob::BlobKind;
use crate::ids::{BlobId, LayerId, StampId, StickerId, TextureSetId};
use crate::import::ModelFormat;
use crate::layer::BlendMode;
use crate::project::sticker_entries::{LinkEntry, PaintLinkEntry, StickerEntry, StyleStackEntry};
use crate::stamp::{Placement, Projector, StampObject, StampSource, SurfaceMode, UvPlacement, WrapOptions};
use crate::text::{FontRef, Gap, Outline, PathShape, TextAlign, TextFx, TextPath, TextSpec, Warp, WarpStyle};

/// Root object of `manifest.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Always `"wtep"`.
    pub format: String,
    /// `FORMAT_VERSION` of the writer.
    pub format_version: u32,
    /// Oldest reader that can open this file without misrendering.
    pub min_reader_version: u32,
    pub app_version: String,
    pub name: String,
    pub saved_unix: u64,
    pub next_id: u64,
    pub source: SourceEntry,
    pub import: ImportEntry,
    pub sets: Vec<SetEntry>,
    #[serde(default)]
    pub stamps: Vec<StampEntry>,
    #[serde(default)]
    pub blobs: Vec<BlobEntry>,
    #[serde(default)]
    pub ui_state: serde_json::Value,
    /// Format 4 (STICKER_PLAN 2.11): the stickers a stamp shows and the kept tray stickers. Not written when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stickers: Vec<StickerEntry>,
    /// Format 4: PhotoCraft links (STICKER_PLAN 5.3). Not written when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkEntry>,
    /// Stickers P12: editor links of raster paint layers (STICKER_PLAN 5.9). Not written when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_links: Vec<PaintLinkEntry>,
}

/// Reference to one ZIP entry with its uncompressed size and CRC-32.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileRef {
    pub entry: String,
    pub size: u64,
    pub crc32: u32,
}

/// The original model files (`SourceBundle`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SourceEntry {
    pub format: ModelFormat,
    pub main: SourceFileEntry,
    pub sidecars: Vec<SourceFileEntry>,
    /// Manually chosen texture files (`source/x<nnn>.<ext>`).
    #[serde(default)]
    pub extras: Vec<SourceFileEntry>,
    pub original_path: Option<String>,
}

/// One file of the source bundle.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SourceFileEntry {
    pub key: String,
    pub original_name: String,
    pub file: FileRef,
}

/// Import options and a fingerprint of the imported mesh.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ImportEntry {
    pub generate_fallback_uvs: bool,
    pub max_texture_dim: u32,
    pub importer_version: u32,
    /// 16 lower-case hex digits.
    pub uv_hash: String,
    pub vertex_count: u32,
    pub triangle_count: u32,
}

/// One texture set.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SetEntry {
    pub id: TextureSetId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub active_layer: Option<LayerId>,
    pub flattened: Option<FileRef>,
    /// `LayerEntry` values, bottom -> top (kept as raw JSON so one unreadable layer does not fail the whole file).
    pub layers: Vec<serde_json::Value>,
}

/// One layer of a set.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LayerEntry {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub alpha_lock: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    #[serde(flatten)]
    pub kind: LayerKindEntry,
    pub pixels: Option<PixelsEntry>,
}

/// Kind of a layer (`"kind": "raster" | "stamp" | "text"`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerKindEntry {
    Raster,
    Stamp { stamp: StampId },
    Text { stamp: StampId },
}

/// Stored pixels of a layer: a PNG entry and the canvas rectangle it covers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PixelsEntry {
    pub file: FileRef,
    /// x, y, w, h.
    pub rect: [i32; 4],
}

/// One stamp object.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StampEntry {
    pub id: StampId,
    pub name: String,
    pub home_set: TextureSetId,
    pub tint: [u8; 4],
    pub source: StampSourceEntry,
    pub placement: PlacementEntry,
    /// Format 4: the stamp's layer styles (STICKER_PLAN 2.5); not written for the default (empty) stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub styles: Option<StyleStackEntry>,
}

/// Source of a stamp (`"kind": "image" | "text" | "sticker"`; `sticker` since format 4).
#[allow(clippy::large_enum_variant)] // the text entry grew with `fx` (round 4); entries are few
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StampSourceEntry {
    Image { blob: BlobId },
    Text(TextEntry),
    Sticker { sticker: StickerId },
}

/// A `TextSpec`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TextEntry {
    pub content: String,
    pub font: FontRefEntry,
    /// Raster resolution.
    pub size_px: f32,
    pub color: [u8; 4],
    pub align: TextAlign,
    pub line_spacing: f32,
    /// Em fraction.
    pub letter_spacing: f32,
    pub outline: Option<OutlineEntry>,
    /// The round-4 settings (format version 2, FEATURE_TEXT 4.3). Not written for a text that uses none of them, so
    /// such a text has exactly the keys of version 1.
    #[serde(default, skip_serializing_if = "TextFxEntry::is_default")]
    pub fx: TextFxEntry,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn is_zero2(v: &[f32; 2]) -> bool {
    *v == [0.0, 0.0]
}

/// A `TextFx`. Every key is optional; values are repaired by `TextSpec::sanitize_fx` after reading.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TextFxEntry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<GapEntry>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bold_em: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub slant_deg: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp: Option<WarpEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathEntry>,
}

impl TextFxEntry {
    /// True when the entry holds nothing (the text uses no round-4 feature).
    pub fn is_default(&self) -> bool {
        *self == TextFxEntry::default()
    }
}

/// A `Gap`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct GapEntry {
    pub after: u32,
    pub em: f32,
}

/// A `Warp`. The style is a string (`WarpStyle::key`), so an unknown or missing name cannot fail the file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct WarpEntry {
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub bend: f32,
    #[serde(default)]
    pub horizontal: f32,
    #[serde(default)]
    pub vertical: f32,
    #[serde(default)]
    pub vertical_axis: bool,
}

/// A `TextPath`. `kind` is `"arc"`, `"circle"` or `"curve"` (missing = `""` = unknown); the fields of the other
/// kinds are zero or empty and therefore not written.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PathEntry {
    #[serde(default)]
    pub kind: String,
    /// Arc.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bend: f32,
    /// Circle.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub diameter: f32,
    /// Circle.
    #[serde(default, skip_serializing_if = "is_zero2")]
    pub center: [f32; 2],
    /// Curve.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f32; 2]>,
    #[serde(default)]
    pub start: f32,
    #[serde(default)]
    pub flip: bool,
}

impl From<&TextFx> for TextFxEntry {
    fn from(fx: &TextFx) -> TextFxEntry {
        TextFxEntry {
            gaps: fx.gaps.iter().map(|g| GapEntry { after: g.after, em: g.em }).collect(),
            bold_em: fx.bold_em,
            slant_deg: fx.slant_deg,
            warp: fx.warp.map(|w| WarpEntry {
                style: w.style.key().to_owned(),
                bend: w.bend,
                horizontal: w.horizontal,
                vertical: w.vertical,
                vertical_axis: w.vertical_axis,
            }),
            path: fx.path.as_ref().map(|p| {
                let mut entry = PathEntry {
                    kind: p.shape.key().to_owned(),
                    start: p.start,
                    flip: p.flip,
                    ..PathEntry::default()
                };
                match &p.shape {
                    PathShape::Arc { bend } => entry.bend = *bend,
                    PathShape::Circle { diameter, center } => {
                        entry.diameter = *diameter;
                        entry.center = *center;
                    }
                    PathShape::Curve { points } => entry.points.clone_from(points),
                }
                entry
            }),
        }
    }
}

impl TextFxEntry {
    /// The effects this entry describes, not yet sanitized; an unknown warp style or path kind gives `None` for
    /// that feature (`TextEntry::unknown_features` names it).
    fn to_fx(&self) -> TextFx {
        TextFx {
            gaps: self.gaps.iter().map(|g| Gap { after: g.after, em: g.em }).collect(),
            bold_em: self.bold_em,
            slant_deg: self.slant_deg,
            warp: self.warp.as_ref().and_then(|w| {
                Some(Warp {
                    style: WarpStyle::from_key(&w.style)?,
                    bend: w.bend,
                    horizontal: w.horizontal,
                    vertical: w.vertical,
                    vertical_axis: w.vertical_axis,
                })
            }),
            path: self.path.as_ref().and_then(|p| {
                let shape = match p.kind.as_str() {
                    "arc" => PathShape::Arc { bend: p.bend },
                    "circle" => PathShape::Circle { diameter: p.diameter, center: p.center },
                    "curve" => PathShape::Curve { points: p.points.clone() },
                    _ => return None,
                };
                Some(TextPath { shape, start: p.start, flip: p.flip })
            }),
        }
    }
}

/// A `FontRef`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FontRefEntry {
    pub blob: BlobId,
    pub face_index: u32,
    pub family: String,
    pub style: String,
}

/// An `Outline`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    pub width_em: f32,
    pub color: [u8; 4],
}

/// A `Placement` (`"space": "projected" | "uv"`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "space", rename_all = "snake_case")]
pub enum PlacementEntry {
    Projected {
        origin: [f32; 3],
        /// Quaternion x, y, z, w.
        rotation: [f32; 4],
        half_size: [f32; 2],
        depth_front: f32,
        depth_back: f32,
        angle_start_deg: f32,
        angle_end_deg: f32,
        backface_cull: bool,
        occlusion: bool,
        flip_x: bool,
        flip_y: bool,
        /// Round 6 (format 3): `""` (absent) = `SurfaceMode::Project`, `"wrap"` = `SurfaceMode::Wrap`. Any other
        /// value (a later round) loads as Project with `LoadWarning::StampFeatureUnknown`.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        mode: String,
        /// Round 6 (format 3): `WrapOptions::jump_gaps`, written only when `true`.
        #[serde(default, skip_serializing_if = "is_false")]
        wrap_gaps: bool,
        /// Round 6 (format 3): `WrapOptions::overlap`, written only when `true`.
        #[serde(default, skip_serializing_if = "is_false")]
        wrap_overlap: bool,
    },
    Uv {
        set: TextureSetId,
        center_px: [f32; 2],
        size_px: [f32; 2],
        /// Radians, `UvPlacement.rotation` verbatim.
        rotation: f32,
        flip_x: bool,
        flip_y: bool,
        clip_to_islands: bool,
    },
}

/// One stored blob (stamp image or font file).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BlobEntry {
    pub id: BlobId,
    pub kind: BlobKind,
    pub original_name: String,
    pub file: FileRef,
}

impl From<&StampObject> for StampEntry {
    fn from(s: &StampObject) -> StampEntry {
        StampEntry {
            id: s.id,
            name: s.name.clone(),
            home_set: s.home_set,
            tint: s.tint,
            source: match &s.source {
                StampSource::Image { blob } => StampSourceEntry::Image { blob: *blob },
                StampSource::Text(spec) => StampSourceEntry::Text(TextEntry::from(spec)),
                StampSource::Sticker { sticker } => StampSourceEntry::Sticker { sticker: *sticker },
            },
            placement: PlacementEntry::from(&s.placement),
            styles: StyleStackEntry::of(&s.styles),
        }
    }
}

impl StampEntry {
    /// The stamp object this entry describes (styles sanitized).
    pub fn to_stamp(&self) -> StampObject {
        StampObject {
            id: self.id,
            name: self.name.clone(),
            source: match &self.source {
                StampSourceEntry::Image { blob } => StampSource::Image { blob: *blob },
                StampSourceEntry::Text(text) => StampSource::Text(text.to_spec()),
                StampSourceEntry::Sticker { sticker } => StampSource::Sticker { sticker: *sticker },
            },
            placement: self.placement.to_placement(),
            tint: self.tint,
            home_set: self.home_set,
            styles: self.styles.as_ref().map(StyleStackEntry::to_styles).unwrap_or_default(),
        }
    }
}

impl From<&TextSpec> for TextEntry {
    fn from(t: &TextSpec) -> TextEntry {
        TextEntry {
            content: t.content.clone(),
            font: FontRefEntry {
                blob: t.font.blob,
                face_index: t.font.face_index,
                family: t.font.family.clone(),
                style: t.font.style.clone(),
            },
            size_px: t.size_px,
            color: t.color,
            align: t.align,
            line_spacing: t.line_spacing,
            letter_spacing: t.letter_spacing,
            outline: t.outline.map(|o| OutlineEntry { width_em: o.width_em, color: o.color }),
            fx: TextFxEntry::from(&t.fx),
        }
    }
}

impl TextEntry {
    /// The text specification this entry describes. An unknown or missing warp style or path kind leaves that
    /// feature out (`unknown_features` names it); the effects are repaired with `TextSpec::sanitize_fx`.
    pub fn to_spec(&self) -> TextSpec {
        let mut spec = TextSpec {
            content: self.content.clone(),
            font: FontRef {
                blob: self.font.blob,
                face_index: self.font.face_index,
                family: self.font.family.clone(),
                style: self.font.style.clone(),
            },
            size_px: self.size_px,
            color: self.color,
            align: self.align,
            line_spacing: self.line_spacing,
            letter_spacing: self.letter_spacing,
            outline: self.outline.as_ref().map(|o| Outline { width_em: o.width_em, color: o.color }),
            fx: self.fx.to_fx(),
        };
        spec.sanitize_fx();
        spec
    }

    /// The round-4 features of this entry that this build does not know, for `LoadWarning::TextFeatureUnknown`:
    /// for example `["warp style \"ribbon\""]`. A missing name is unknown too (`warp style ""`).
    pub fn unknown_features(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(warp) = &self.fx.warp
            && WarpStyle::from_key(&warp.style).is_none()
        {
            out.push(format!("warp style \"{}\"", warp.style));
        }
        if let Some(path) = &self.fx.path
            && !matches!(path.kind.as_str(), "arc" | "circle" | "curve")
        {
            out.push(format!("path kind \"{}\"", path.kind));
        }
        out
    }
}

impl From<&Placement> for PlacementEntry {
    fn from(p: &Placement) -> PlacementEntry {
        match p {
            Placement::Projected(p) => PlacementEntry::Projected {
                origin: p.origin.to_array(),
                rotation: p.rotation.to_array(),
                half_size: p.half_size.to_array(),
                depth_front: p.depth_front,
                depth_back: p.depth_back,
                angle_start_deg: p.angle_start_deg,
                angle_end_deg: p.angle_end_deg,
                backface_cull: p.backface_cull,
                occlusion: p.occlusion,
                flip_x: p.flip_x,
                flip_y: p.flip_y,
                mode: mode_key(p.mode).to_owned(),
                wrap_gaps: p.wrap.jump_gaps,
                wrap_overlap: p.wrap.overlap,
            },
            Placement::Uv(p) => PlacementEntry::Uv {
                set: p.set,
                center_px: p.center_px.to_array(),
                size_px: p.size_px.to_array(),
                rotation: p.rotation,
                flip_x: p.flip_x,
                flip_y: p.flip_y,
                clip_to_islands: p.clip_to_islands,
            },
        }
    }
}

impl PlacementEntry {
    /// The placement this entry describes (the quaternion is taken verbatim, without re-normalization).
    pub fn to_placement(&self) -> Placement {
        match *self {
            PlacementEntry::Projected {
                origin,
                rotation,
                half_size,
                depth_front,
                depth_back,
                angle_start_deg,
                angle_end_deg,
                backface_cull,
                occlusion,
                flip_x,
                flip_y,
                ref mode,
                wrap_gaps,
                wrap_overlap,
            } => Placement::Projected(Projector {
                origin: Vec3::from_array(origin),
                rotation: Quat::from_array(rotation),
                half_size: Vec2::from_array(half_size),
                depth_front,
                depth_back,
                angle_start_deg,
                angle_end_deg,
                backface_cull,
                occlusion,
                flip_x,
                flip_y,
                mode: mode_of_key(mode).unwrap_or_default(),
                wrap: WrapOptions { jump_gaps: wrap_gaps, overlap: wrap_overlap },
            }),
            PlacementEntry::Uv { set, center_px, size_px, rotation, flip_x, flip_y, clip_to_islands } => {
                Placement::Uv(UvPlacement {
                    set,
                    center_px: Vec2::from_array(center_px),
                    size_px: Vec2::from_array(size_px),
                    rotation,
                    flip_x,
                    flip_y,
                    clip_to_islands,
                })
            }
        }
    }
}

impl PlacementEntry {
    /// Round 6: the surface mode features of this entry that this build does not know, for
    /// `LoadWarning::StampFeatureUnknown`: for example `["surface mode \"melt\""]`.
    pub fn unknown_features(&self) -> Vec<String> {
        match self {
            PlacementEntry::Projected { mode, .. } if mode_of_key(mode).is_none() => {
                vec![format!("surface mode \"{mode}\"")]
            }
            _ => Vec::new(),
        }
    }
}

/// The manifest key of a surface mode: `""` for `Project` (not written), `"wrap"` for `Wrap`.
fn mode_key(mode: SurfaceMode) -> &'static str {
    match mode {
        SurfaceMode::Project => "",
        SurfaceMode::Wrap => "wrap",
    }
}

/// The surface mode of a manifest key; `None` for a key this build does not know.
fn mode_of_key(key: &str) -> Option<SurfaceMode> {
    match key {
        "" => Some(SurfaceMode::Project),
        "wrap" => Some(SurfaceMode::Wrap),
        _ => None,
    }
}

/// `skip_serializing_if` helper: a `false` flag is not written.
fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projected_stamp() -> StampObject {
        StampObject {
            id: StampId(7),
            name: "Logo".into(),
            source: StampSource::Image { blob: BlobId(3) },
            placement: Placement::Projected(Projector {
                origin: Vec3::new(0.1, -2.5, 3.333_333_3),
                rotation: Quat::from_xyzw(0.182_574_19, 0.365_148_37, 0.547_722_6, 0.730_296_7),
                half_size: Vec2::new(0.3, 0.7),
                depth_front: 0.123_456_79,
                depth_back: 1e-7,
                angle_start_deg: 60.0,
                angle_end_deg: 85.0,
                backface_cull: true,
                occlusion: false,
                flip_x: true,
                flip_y: false,
                mode: SurfaceMode::Project,
                wrap: WrapOptions::default(),
            }),
            tint: [255, 128, 0, 200],
            home_set: TextureSetId(1),
            styles: Default::default(),
        }
    }

    fn text_stamp() -> StampObject {
        StampObject {
            id: StampId(u64::MAX - 1),
            name: "Label \"quoted\"\n".into(),
            source: StampSource::Text(TextSpec {
                content: "Hello\nW\u{f6}rld".into(),
                font: FontRef { blob: BlobId(0), face_index: 2, family: "Ubuntu".into(), style: "Light".into() },
                size_px: 128.0,
                color: [1, 2, 3, 4],
                align: TextAlign::Center,
                line_spacing: 1.1,
                letter_spacing: 0.05,
                outline: Some(Outline { width_em: 0.062_5, color: [0, 0, 0, 255] }),
                fx: TextFx::default(),
            }),
            placement: Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: Vec2::new(511.5, 100.25),
                size_px: Vec2::new(300.0, 80.0),
                rotation: std::f32::consts::FRAC_PI_3,
                flip_x: false,
                flip_y: true,
                clip_to_islands: true,
            }),
            tint: [255; 4],
            home_set: TextureSetId(0),
            styles: Default::default(),
        }
    }

    #[test]
    fn stamp_round_trips_through_json_bit_exactly() {
        for stamp in [projected_stamp(), text_stamp()] {
            let entry = StampEntry::from(&stamp);
            assert_eq!(entry.to_stamp(), stamp);
            let json = serde_json::to_string_pretty(&entry).unwrap();
            let back: StampEntry = serde_json::from_str(&json).unwrap();
            assert_eq!(back, entry);
            assert_eq!(back.to_stamp(), stamp);
            // Also through a `serde_json::Value`, as the loader does.
            let value = serde_json::to_value(&entry).unwrap();
            let back: StampEntry = serde_json::from_value(value).unwrap();
            assert_eq!(back.to_stamp(), stamp);
        }
    }

    #[test]
    fn rotation_is_stored_verbatim() {
        let entry = StampEntry::from(&text_stamp());
        match entry.placement {
            PlacementEntry::Uv { rotation, .. } => assert_eq!(rotation.to_bits(), std::f32::consts::FRAC_PI_3.to_bits()),
            _ => panic!("expected a UV placement"),
        }
        match StampEntry::from(&projected_stamp()).placement {
            PlacementEntry::Projected { rotation, angle_start_deg, .. } => {
                assert_eq!(rotation, [0.182_574_19, 0.365_148_37, 0.547_722_6, 0.730_296_7]);
                assert_eq!(angle_start_deg, 60.0);
            }
            _ => panic!("expected a projected placement"),
        }
    }

    #[test]
    fn tagged_json_shapes() {
        let entry = StampEntry::from(&projected_stamp());
        let v = serde_json::to_value(&entry).unwrap();
        assert_eq!(v["source"]["kind"], "image");
        assert_eq!(v["source"]["blob"], 3);
        assert_eq!(v["placement"]["space"], "projected");
        assert_eq!(v["home_set"], 1);

        let v = serde_json::to_value(StampEntry::from(&text_stamp())).unwrap();
        assert_eq!(v["source"]["kind"], "text");
        assert_eq!(v["source"]["align"], "center");
        assert_eq!(v["placement"]["space"], "uv");
    }

    #[test]
    fn layer_entry_json() {
        let raster = LayerEntry {
            id: LayerId(4),
            name: "Paint 1".into(),
            visible: true,
            locked: false,
            alpha_lock: true,
            opacity: 0.75,
            blend: BlendMode::Multiply,
            kind: LayerKindEntry::Raster,
            pixels: None,
        };
        let v = serde_json::to_value(&raster).unwrap();
        assert_eq!(v["kind"], "raster");
        assert_eq!(v["blend"], "multiply");
        assert!(v["pixels"].is_null());
        assert_eq!(serde_json::from_value::<LayerEntry>(v).unwrap(), raster);

        let text = LayerEntry {
            kind: LayerKindEntry::Text { stamp: StampId(9) },
            pixels: Some(PixelsEntry {
                file: FileRef { entry: "sets/s00/l5.png".into(), size: 1234, crc32: 0xdead_beef },
                rect: [10, 20, 30, 40],
            }),
            ..raster.clone()
        };
        let v = serde_json::to_value(&text).unwrap();
        assert_eq!(v["kind"], "text");
        assert_eq!(v["stamp"], 9);
        assert_eq!(v["pixels"]["rect"][3], 40);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<LayerEntry>(&json).unwrap(), text);

        // An unknown kind is a parse error (the loader turns it into LoadWarning::UnknownLayer).
        let mut bad = serde_json::to_value(&raster).unwrap();
        bad["kind"] = "adjustment".into();
        assert!(serde_json::from_value::<LayerEntry>(bad).is_err());
    }

    #[test]
    fn text_fx_round_trips_and_plain_text_keeps_the_v1_keys() {
        // A plain text: exactly the keys of version 1 (the eight fields and "kind").
        let plain = StampEntry::from(&text_stamp());
        let v = serde_json::to_value(&plain).unwrap();
        let keys: Vec<&String> = v["source"].as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 9, "{keys:?}");
        assert!(v["source"].get("fx").is_none());

        // Every feature set.
        let mut stamp = text_stamp();
        let StampSource::Text(spec) = &mut stamp.source else { panic!() };
        spec.fx = TextFx {
            gaps: vec![Gap { after: 2, em: 0.3 }, Gap { after: 7, em: -0.05 }],
            bold_em: 0.02,
            slant_deg: 12.0,
            warp: Some(Warp { style: WarpStyle::ArcLower, bend: 0.5, horizontal: -0.25, vertical: 0.0, vertical_axis: true }),
            path: Some(TextPath { shape: PathShape::Curve { points: vec![[-2.0, 0.3], [0.0, -0.2], [2.0, 0.3]] }, start: 0.5, flip: true }),
        };
        let entry = StampEntry::from(&stamp);
        let v = serde_json::to_value(&entry).unwrap();
        assert_eq!(v["source"]["fx"]["warp"]["style"], "arc_lower");
        assert_eq!(v["source"]["fx"]["path"]["kind"], "curve");
        // A path entry writes only the keys of its kind.
        let path_keys: Vec<&String> = v["source"]["fx"]["path"].as_object().unwrap().keys().collect();
        assert!(!path_keys.iter().any(|k| ["bend", "diameter", "center"].contains(&k.as_str())), "{path_keys:?}");
        let back: StampEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back.to_stamp(), stamp);
        for shape in [PathShape::Arc { bend: -0.25 }, PathShape::Circle { diameter: 4.0, center: [0.0, 2.5] }] {
            let StampSource::Text(spec) = &mut stamp.source else { panic!() };
            spec.fx.path = Some(TextPath { shape, start: -1.0, flip: false });
            let json = serde_json::to_string(&StampEntry::from(&stamp)).unwrap();
            assert!(!json.contains("\"points\""));
            assert_eq!(serde_json::from_str::<StampEntry>(&json).unwrap().to_stamp(), stamp);
        }
        assert!(entry.source != plain.source);
    }

    #[test]
    fn unknown_fx_names_load_without_the_feature() {
        let mut v = serde_json::to_value(StampEntry::from(&text_stamp())).unwrap();
        v["source"]["fx"] = serde_json::json!({
            "gaps": [ { "after": 99, "em": 0.5 }, { "after": 1, "em": 9.0 } ],
            "warp": { "style": "ribbon", "bend": 0.5 },
            "path": { "points": [[0.0, 0.0], [1.0, 0.0]] }
        });
        let entry: StampEntry = serde_json::from_value(v).unwrap();
        let StampSourceEntry::Text(text) = &entry.source else { panic!() };
        assert_eq!(text.unknown_features(), vec!["warp style \"ribbon\"".to_owned(), "path kind \"\"".to_owned()]);
        let StampSource::Text(spec) = entry.to_stamp().source else { panic!() };
        assert_eq!(spec.fx.warp, None);
        assert_eq!(spec.fx.path, None);
        // The spec is sanitized: the gap beyond the text is gone, the other one clamped.
        assert_eq!(spec.fx.gaps, vec![Gap { after: 1, em: crate::text::GAP_MAX_EM }]);
        // A wrong JSON type inside fx fails the entry (the documented limit, FEATURE_TEXT 4.3 item 4).
        let mut bad = serde_json::to_value(StampEntry::from(&text_stamp())).unwrap();
        bad["source"]["fx"] = serde_json::json!({ "bold_em": "thick" });
        assert!(serde_json::from_value::<StampEntry>(bad).is_err());
    }

    #[test]
    fn optional_manifest_fields_default() {
        let json = serde_json::json!({
            "format": "wtep",
            "format_version": 1,
            "min_reader_version": 1,
            "app_version": "0.1.0",
            "name": "demo",
            "saved_unix": 0,
            "next_id": 5,
            "source": {
                "format": "obj",
                "main": { "key": "a.obj", "original_name": "A.obj",
                          "file": { "entry": "source/main.obj", "size": 1, "crc32": 2 } },
                "sidecars": [],
                "original_path": null
            },
            "import": { "generate_fallback_uvs": false, "max_texture_dim": 8192, "importer_version": 1,
                        "uv_hash": "0123456789abcdef", "vertex_count": 3, "triangle_count": 1 },
            "sets": [ { "id": 0, "name": "Body", "width": 16, "height": 16, "active_layer": null,
                        "flattened": null, "layers": [] } ]
        });
        let m: Manifest = serde_json::from_value(json).unwrap();
        assert!(m.stamps.is_empty() && m.blobs.is_empty() && m.ui_state.is_null());
        assert!(m.source.extras.is_empty());
        assert_eq!(m.source.format, ModelFormat::Obj);
        assert_eq!(m.sets[0].id, TextureSetId(0));
        let again: Manifest = serde_json::from_str(&serde_json::to_string_pretty(&m).unwrap()).unwrap();
        assert_eq!(again, m);
    }

    /// WA13 (round 6): a wrapped stamp keeps its mode and both options bit-exactly; the options are written only when
    /// set; a placement without `mode` is Project; an unknown mode loads as Project and is named.
    #[test]
    fn wrapped_placements_round_trip_and_write_new_keys_only_when_set() {
        let base = projected_stamp();
        let Placement::Projected(p) = base.placement else { unreachable!() };
        let json_of = |stamp: &StampObject| serde_json::to_value(StampEntry::from(stamp)).unwrap();
        // Project, no options: no new key.
        let v = json_of(&base);
        for key in ["mode", "wrap_gaps", "wrap_overlap"] {
            assert!(v["placement"].get(key).is_none(), "{key} written for a plain projected stamp");
        }
        for (mode, jump_gaps, overlap) in [
            (SurfaceMode::Wrap, false, false),
            (SurfaceMode::Wrap, true, false),
            (SurfaceMode::Wrap, false, true),
            (SurfaceMode::Wrap, true, true),
            (SurfaceMode::Project, true, true),
            (SurfaceMode::Project, false, true),
        ] {
            let stamp = StampObject {
                placement: Placement::Projected(Projector { mode, wrap: WrapOptions { jump_gaps, overlap }, ..p }),
                ..base.clone()
            };
            let v = json_of(&stamp);
            let placement = &v["placement"];
            assert_eq!(placement.get("mode").is_some(), mode == SurfaceMode::Wrap);
            if mode == SurfaceMode::Wrap {
                assert_eq!(placement["mode"], "wrap");
            }
            assert_eq!(placement.get("wrap_gaps").is_some(), jump_gaps);
            assert_eq!(placement.get("wrap_overlap").is_some(), overlap);
            let back: StampEntry = serde_json::from_str(&serde_json::to_string_pretty(&v).unwrap()).unwrap();
            assert_eq!(back.to_stamp(), stamp);
            assert!(back.placement.unknown_features().is_empty());
        }
        // An unknown mode: Project, named.
        let mut v = json_of(&base);
        v["placement"]["mode"] = serde_json::json!("melt");
        v["placement"]["wrap_overlap"] = serde_json::json!(true);
        let entry: StampEntry = serde_json::from_value(v).unwrap();
        let Placement::Projected(loaded) = entry.to_stamp().placement else { panic!("expected a projected placement") };
        assert_eq!(loaded.mode, SurfaceMode::Project);
        assert!(loaded.wrap.overlap && !loaded.wrap.jump_gaps);
        assert_eq!(entry.placement.unknown_features(), vec!["surface mode \"melt\"".to_owned()]);
    }
}
