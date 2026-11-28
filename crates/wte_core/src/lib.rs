//! `wte_core` — the data model and every algorithm of Wrapped Texture Editor: model import, UV analysis, BVH
//! raycast, brush engine, stamp/text projection bake, layer compositing, undo, the `.wtep` project file and export.
//!
//! No GPU, no GUI; fully unit-testable headless. The binding contract for this crate is `ARCHITECTURE.md`
//! (sections 4-7) in the workspace root.
//!
//! Conventions in one glance (section 4):
//! - World space is right-handed, Y-up; counter-clockwise triangles are front faces.
//! - UV space has its origin top-left with **v pointing down**; row 0 of every pixel buffer is the top row.
//! - All pixel data is RGBA8, straight alpha, sRGB-encoded; RGB under alpha 0 is preserved.
//! - Errors are `thiserror` enums from [`error`]; bad input never panics.
//!
//! Every type of the public API is re-exported at the crate root; free functions with generic names
//! (`project::save`, `export::run`, `samples::*`, ...) are reached through their modules.

pub use glam;

pub mod blend;
pub mod blob;
pub mod brush;
pub mod brush_tip;
pub mod bvh;
pub mod composite;
pub mod doc_build;
pub mod doc_layer_links;
pub mod doc_layers;
pub mod doc_links;
pub mod doc_stamps;
pub mod doc_sticker_edits;
pub mod doc_sticker_paint;
pub mod doc_sticker_cutout;
pub mod doc_sticker_layout;
pub mod doc_stickers;
pub mod document;
pub mod error;
pub mod export;
pub mod fill;
pub mod history;
pub mod ids;
pub mod image_io;
pub mod import;
pub mod layer;
pub mod link;
pub mod math;
pub mod mesh;
pub mod mesh_build;
pub mod pixel;
pub mod pixel_ops;
pub mod progress;
pub mod project;
pub mod psd;
pub mod samples;
pub mod stamp;
pub mod stamp_bake;
pub mod stamp_job;
pub mod sticker;
pub mod stroke;
pub mod texel_map;
pub mod text;
pub mod tip_bake;
pub mod tip_field;
pub mod wrap_bake;
pub mod uv_share;
pub mod wrap_param;
#[cfg(test)]
mod doc_stickers_tests;
#[cfg(test)]
mod doc_sticker_edits_tests;
#[cfg(test)]
mod doc_sticker_tab_tests;
#[cfg(test)]
mod doc_sticker_paint_tests;
#[cfg(test)]
mod doc_sticker_cutout_tests;
#[cfg(test)]
mod doc_sticker_layout_tests;
#[cfg(test)]
mod wrap_tests;

#[doc(hidden)]
pub mod testutil;

pub use blend::*;
pub use blob::*;
pub use brush::*;
pub use brush_tip::*;
pub use bvh::*;
pub use composite::*;
pub use doc_build::*;
pub use doc_layers::*;
pub use doc_sticker_edits::{
    CONVERT_LABEL, PixelData, PixelTarget, REVERT_LABEL, STICKER_BRUSH_LABEL, STICKER_ERASER_LABEL, StickerBrush,
    StickerCheckpoint, StickerPatch,
};
pub use doc_sticker_paint::{RASTERIZE_LABEL, StickerRenderJob, StickerRenderOutput, StickerRenderPurpose};
pub use doc_sticker_layout::{
    CANVAS_SIZE_LABEL, CROP_LABEL, FLATTEN_LABEL, FLIP_H_LABEL, FLIP_V_LABEL, MERGE_DOWN_LABEL, PADDING_LABEL, RESIZE_LABEL,
    ROTATE_LABEL, TRIM_LABEL,
};
pub use doc_stickers::{STYLE_LABEL, ShareScope, StampPicture, StickerCaches, StickerRef};
pub use document::*;
pub use error::*;
pub use fill::*;
pub use history::*;
pub use ids::*;
pub use image_io::*;
pub use layer::*;
pub use math::*;
pub use mesh::*;
pub use mesh_build::*;
pub use pixel::*;
pub use pixel_ops::*;
pub use progress::*;
pub use stamp::*;
pub use stamp_bake::*;
pub use stamp_job::{FinalBake, StampBakeJob, StampBakeOutput, StampJobKind, WrapPreview};
pub use sticker::adjust::Adjustment;
pub use sticker::layout::LayerBox;
pub use sticker::cutout::{BackgroundCut, Matte, RemoveBgOptions, RemoveBgRefusal};
pub use sticker::grabcut::{BoxCutRefusal, cut_out_box};
pub use sticker::file::{
    WTES_EXTENSION, WTES_FORMAT, WTES_MIMETYPE, WtesFile, WtesManifest, read_wtes, read_wtes_preview, read_wtes_preview_from, write_wtes,
};
pub use sticker::select::{RegionOptions, SelectMode, Selection};
pub use sticker::shape::{Paint, ShapeKind, ShapeSpec};
pub use sticker::style::{
    Bevel, ColorOverlay, Contour, Glow, Gradient, GradientOverlay, Shadow, Stroke, StrokePos, StylePreset, StyleStack,
};
pub use sticker::{
    ArtboardShape, LayerMask, PicturePixels, SourceFrame, StickerContent, StickerDoc, StickerLayer, StickerRenderInput,
    LayerTransform, MAX_ARTBOARD, MAX_CREATED_SIDE, MAX_GRADIENT_STOPS, MAX_OWNED_BYTES, MAX_PICTURE_SIDE,
    MAX_STICKER_LAYERS, MAX_STYLE_LENGTH,
};
pub use stroke::*;
pub use texel_map::*;
pub use tip_bake::*;
pub use tip_field::*;
pub use wrap_bake::*;
pub use wrap_param::*;

pub use export::{ExportLayer, ExportOptions, ExportPackage, ExportReport, ExportSet, PlannedFile, PlannedKind};
pub use import::{
    AssetResolver, DiskResolver, IMPORTER_VERSION, ImportOptions, ImportedModel, ImportedTexture, MemoryResolver,
    ModelFormat, RecordingResolver, SourceBundle, SourceFile,
};
pub use project::{
    BlobEntry, FORMAT_VERSION, FileRef, FontRefEntry, GapEntry, ImportEntry, LayerEntry, LayerKindEntry, LoadReport,
    Manifest, OutlineEntry, PROJECT_EXTENSION, PROJECT_MIMETYPE, PathEntry, PixelsEntry, PlacementEntry, SaveOptions,
    SaveStats, SetEntry, SourceEntry, SourceFileEntry, StampEntry, StampSourceEntry, TextEntry, TextFxEntry, WarpEntry,
};
pub use text::{
    DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE, FAUX_BOLD_EM, FAUX_SLANT_DEG, FaceTraits, FontCache, FontCatalog, FontFaceInfo,
    FontRef, GAP_MAX_EM, GAP_MIN_EM, Gap, MAX_PATH_POINTS, MAX_STYLED_PIXELS, Outline, PATH_COORD_MAX_EM, PathShape,
    PlacedGlyph, RasterQuality, TEXT_PLACEMENT_PX, TextAlign, TextFrame, TextFx, TextLayout, TextPath, TextRaster, TextSpec,
    Warp, WarpStyle,
};
