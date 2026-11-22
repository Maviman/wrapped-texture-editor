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
pub mod doc_layers;
pub mod doc_stamps;
pub mod document;
pub mod error;
pub mod export;
pub mod fill;
pub mod history;
pub mod ids;
pub mod image_io;
pub mod import;
pub mod layer;
pub mod math;
pub mod mesh;
pub mod mesh_build;
pub mod pixel;
pub mod pixel_ops;
pub mod progress;
pub mod project;
pub mod samples;
pub mod stamp;
pub mod stamp_bake;
pub mod stamp_job;
pub mod stroke;
pub mod texel_map;
pub mod text;
pub mod tip_bake;
pub mod tip_field;
pub mod wrap_bake;
pub mod uv_share;
pub mod wrap_param;
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
pub use stroke::*;
pub use texel_map::*;
pub use tip_bake::*;
pub use tip_field::*;
pub use wrap_bake::*;
pub use wrap_param::*;

pub use export::{ExportLayer, ExportOptions, ExportReport, ExportSet};
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
