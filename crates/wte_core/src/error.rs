//! Every error and warning enum of the crate (ARCHITECTURE.md 4.5, 5.2).

use std::fmt;

use crate::ids::{BlobId, LayerId, StampId, StickerId, StickerLayerId, TextureSetId};

/// Failure of an `AssetResolver` to deliver a referenced file.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("cannot read {0}: {1}")]
    Io(String, #[source] std::io::Error),
}

/// Image decode / encode failure.
#[derive(Debug, thiserror::Error)]
pub enum ImageIoError {
    #[error("cannot decode image: {0}")]
    Decode(String),
    #[error("cannot encode image: {0}")]
    Encode(String),
    #[error("unsupported image format: {0}")]
    UnsupportedFormat(String),
    #[error("image is too large: {width}x{height}")]
    TooLarge { width: u32, height: u32 },
    #[error("image has zero size")]
    Empty,
}

/// Fatal model import failure.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("unrecognized model format")]
    UnknownFormat,
    #[error("OBJ parse error: {0}")]
    Obj(String),
    #[error("glTF error: {0}")]
    InvalidGltf(String),
    /// A damaged or unsupported FBX file (`import/fbx.rs`); the text says what is wrong.
    #[error("invalid FBX file: {0}")]
    InvalidFbx(String),
    #[error("glTF requires unsupported extension(s): {0:?}")]
    UnsupportedGltfExtension(Vec<String>),
    /// Only a missing glTF `.bin` is fatal.
    #[error("referenced file not found: {0}")]
    MissingSidecar(String),
    #[error("buffer {index} too short: expected {expected}, got {actual}")]
    BufferLength { index: usize, expected: usize, actual: usize },
    /// `reasons`: why the loader skipped geometry (the messages of its `SkippedPrimitive` warnings), so the user
    /// learns which accessor or primitive is broken.
    #[error("model contains no triangles{}", empty_reasons(reasons))]
    Empty { reasons: Vec<String> },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The model file is larger than a project can store (`import::MAX_SOURCE_FILE`).
    #[error("{name} is {size} bytes; a project can store files of at most {limit} bytes")]
    FileTooLarge { name: String, size: u64, limit: u64 },
    #[error("cancelled")]
    Cancelled,
}

/// The tail of `ImportError::Empty`'s message: up to three reasons, then a count of the rest.
fn empty_reasons(reasons: &[String]) -> String {
    const SHOWN: usize = 3;
    if reasons.is_empty() {
        return String::new();
    }
    let mut text = format!(" ({}", reasons.iter().take(SHOWN).cloned().collect::<Vec<_>>().join("; "));
    if reasons.len() > SHOWN {
        text.push_str(&format!("; and {} more", reasons.len() - SHOWN));
    }
    text.push(')');
    text
}

/// Recoverable import problem, returned next to the imported model.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportWarning {
    MissingMtl(String),
    MissingTexture { material: String, reference: String },
    TextureDecodeFailed { reference: String, reason: String },
    TextureResized { reference: String, from: [u32; 2], to: [u32; 2] },
    NoUvs { material: String },
    DegenerateUvs { material: String, percent: u8 },
    /// UVs outside 0..1 were folded onto one tile (no UDIM).
    UvWrapped { triangles: u32 },
    NormalsGenerated,
    DroppedDegenerate { triangles: u32 },
    SkippedPrimitive { reason: String },
    SkinIgnored,
    FallbackUvsGenerated,
    /// OBJ lines that were ignored (`import/obj.rs`): `v` / `vt` / `vn` lines with a missing, malformed or non-finite
    /// number, and `f` lines with a position index that points nowhere. `first_line` is the 1-based line number of the
    /// first one; faces using a bad `v` (and the bad `f` lines) were dropped, faces using a bad `vt` / `vn` were kept
    /// without UVs or with generated normals.
    BadObjNumbers { first_line: usize, lines: u32, dropped_faces: u32, degraded_faces: u32 },
}

impl fmt::Display for ImportWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportWarning::MissingMtl(name) => {
                write!(f, "The material library \"{name}\" was not found; default materials are used.")
            }
            ImportWarning::MissingTexture { material, reference } => {
                write!(f, "The texture \"{reference}\" of material \"{material}\" was not found.")
            }
            ImportWarning::TextureDecodeFailed { reference, reason } => {
                write!(f, "The texture \"{reference}\" could not be decoded: {reason}.")
            }
            ImportWarning::TextureResized { reference, from, to } => write!(
                f,
                "The texture \"{reference}\" was downscaled from {}x{} to {}x{}.",
                from[0], from[1], to[0], to[1]
            ),
            ImportWarning::NoUvs { material } => {
                write!(f, "Material \"{material}\" has no UV coordinates and cannot be painted.")
            }
            ImportWarning::DegenerateUvs { material, percent } => write!(
                f,
                "{percent}% of the triangles of material \"{material}\" have degenerate UVs and cannot be painted."
            ),
            ImportWarning::UvWrapped { triangles } => write!(
                f,
                "{triangles} triangle(s) use UVs outside 0..1; they were folded onto a single tile (UDIM is not supported)."
            ),
            ImportWarning::NormalsGenerated => {
                write!(f, "The model has no normals; smooth normals were generated.")
            }
            ImportWarning::DroppedDegenerate { triangles } => {
                write!(f, "{triangles} degenerate triangle(s) were dropped.")
            }
            ImportWarning::SkippedPrimitive { reason } => {
                write!(f, "A primitive was skipped: {reason}.")
            }
            ImportWarning::SkinIgnored => {
                write!(f, "Skinning and morph targets are ignored; the model is imported in its rest pose.")
            }
            ImportWarning::FallbackUvsGenerated => {
                write!(f, "The model has no UV coordinates; a simple automatic UV layout was generated.")
            }
            ImportWarning::BadObjNumbers { first_line, lines, dropped_faces, degraded_faces } => {
                write!(
                    f,
                    "{lines} line(s) of the OBJ file have missing or invalid numbers or refer to vertices that do not exist (the first is line {first_line}); they were ignored"
                )?;
                if *dropped_faces > 0 {
                    write!(f, "; {dropped_faces} face(s) were left out")?;
                }
                if *degraded_faces > 0 {
                    write!(f, "; {degraded_faces} face(s) lost their UV coordinates or normals")?;
                }
                write!(f, ".")
            }
        }
    }
}

impl ImportWarning {
    /// A fact about the model file itself (generated normals or UVs, folded tiles, dropped triangles, skipped
    /// primitives, ignored skinning, unpaintable materials, unusable OBJ number lines). The import dialog shows it once; re-importing the same
    /// stored file on project load gives it again unchanged, so `project::load` does not report it a second time.
    /// The texture and material-library warnings are not of this kind: on load they can mean that a file stored in
    /// the project is missing or unreadable.
    pub fn is_model_fact(&self) -> bool {
        match self {
            ImportWarning::NoUvs { .. }
            | ImportWarning::DegenerateUvs { .. }
            | ImportWarning::UvWrapped { .. }
            | ImportWarning::NormalsGenerated
            | ImportWarning::DroppedDegenerate { .. }
            | ImportWarning::SkippedPrimitive { .. }
            | ImportWarning::SkinIgnored
            | ImportWarning::FallbackUvsGenerated
            | ImportWarning::BadObjNumbers { .. } => true,
            ImportWarning::MissingMtl(_)
            | ImportWarning::MissingTexture { .. }
            | ImportWarning::TextureDecodeFailed { .. }
            | ImportWarning::TextureResized { .. } => false,
        }
    }
}

/// Document construction failure.
#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("the model has no UV-mapped geometry")]
    NoTextureSets,
    #[error("invalid document setup: {0}")]
    BadSetup(String),
    #[error(transparent)]
    Image(#[from] ImageIoError),
    #[error("cancelled")]
    Cancelled,
}

/// Font / text rasterization failure.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("font error: {0}")]
    Font(String),
    #[error("font data is missing from the project")]
    MissingFont,
    #[error("text image would be too large: {width}x{height}")]
    TooLarge { width: u32, height: u32 },
}

/// Rejected document command.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("unknown texture set")]
    UnknownSet,
    #[error("unknown layer")]
    UnknownLayer,
    #[error("unknown stamp")]
    UnknownStamp,
    #[error("unknown image or font")]
    UnknownBlob,
    #[error("layer is locked")]
    LayerLocked,
    #[error("layer is hidden")]
    LayerHidden,
    #[error("layer is not a paint layer")]
    NotRaster,
    #[error("no paintable layer")]
    NoPaintableLayer,
    #[error("a stroke is in progress")]
    StrokeInProgress,
    #[error("cannot merge: {0}")]
    CannotMerge(&'static str),
    #[error("invalid parameter: {0}")]
    InvalidParam(&'static str),
    /// Stickers (STICKER_PLAN 2.8): no sticker with that id in the document.
    #[error("unknown sticker")]
    UnknownSticker,
    /// No layer with that id in the sticker.
    #[error("unknown sticker layer")]
    UnknownStickerLayer,
    /// A sticker limit (`sticker::MAX_*`) would be exceeded; the text names it for the user.
    #[error("{0}")]
    StickerLimit(&'static str),
    #[error(transparent)]
    Text(#[from] TextError),
    #[error(transparent)]
    Image(#[from] ImageIoError),
}

/// Fatal project save / load failure.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a Wrapped Texture Editor project: {0}")]
    NotAZip(String),
    #[error("project manifest is missing")]
    ManifestMissing,
    #[error("project manifest is invalid: {0}")]
    ManifestInvalid(String),
    #[error("project was saved by a newer version (format {found}, supported {supported})")]
    TooNew { found: u32, supported: u32 },
    #[error("model data is unrecoverable: {0}")]
    ModelUnrecoverable(String),
    #[error("entry {entry} exceeds the size limit")]
    EntryTooLarge { entry: String },
    #[error(transparent)]
    Image(#[from] ImageIoError),
    /// From `zip::result::ZipError` (non_exhaustive): map with `to_string()`.
    #[error("zip error: {0}")]
    Zip(String),
    #[error("save verification failed: {0}")]
    VerifyFailed(String),
    #[error("cancelled")]
    Cancelled,
}

/// Recoverable project load problem, returned next to the loaded document.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadWarning {
    NewerFormat { found: u32 },
    MeshUvChanged,
    /// The project was saved by an older importer (`saved` < `IMPORTER_VERSION`) with an automatic UV layout, and the
    /// layout generated now differs (`uv_hash`). Nothing is changed on disk.
    OlderGeneratedUvLayout { saved: u32 },
    LayerPixelsLost { layer: LayerId, name: String, reason: String },
    BakeCacheRebuilt { layer: LayerId },
    BlobMissing { blob: BlobId, used_by: Vec<StampId> },
    FontFallback { stamp: StampId, family: String },
    UnknownLayer { set: TextureSetId, index: usize },
    /// A text stamp uses a round-4 feature this build does not know (an unknown warp style or path kind, named in
    /// `what`); the stamp loads without that feature and plain Save is refused (`LoadReport::force_save_as`).
    TextFeatureUnknown { stamp: StampId, what: String },
    /// Round 6: a 3D stamp uses a surface mode this build does not know (named in `what`); it loads as a projected
    /// stamp and plain Save is refused (`LoadReport::force_save_as`).
    StampFeatureUnknown { stamp: StampId, what: String },
    /// Format 4 (STICKER_PLAN 2.11): a sticker is missing from the file or could not be read (`reason`); the stamps
    /// showing it keep their pictures as paint layers (the rule of a missing image blob).
    StickerLost { sticker: StickerId, used_by: Vec<StampId>, reason: String },
    /// Format 4: the picture of a sticker layer (or, with `mask`, its mask) could not be read; the layer is now empty
    /// (a lost mask hides the whole layer).
    StickerPixelsLost { sticker: StickerId, layer: StickerLayerId, name: String, mask: bool, reason: String },
    /// Format 4: the font of a sticker's text layer is missing; the built-in font is used instead.
    StickerFontFallback { sticker: StickerId, layer: StickerLayerId, family: String },
    Import(ImportWarning),
}

impl fmt::Display for LoadWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadWarning::NewerFormat { found } => write!(
                f,
                "The project was saved by a newer version (format {found}); use Save As to keep the original file intact."
            ),
            LoadWarning::MeshUvChanged => write!(
                f,
                "The model's UV layout differs from the one the project was saved with; painted content may be misplaced."
            ),
            LoadWarning::OlderGeneratedUvLayout { saved } => write!(
                f,
                "This project was saved with an older automatic texture layout (importer version {saved}). This version \
                 lays the texture out differently, so painted content may sit in the wrong place on the model. \
                 Nothing was overwritten: the project file stays as it is unless you save it."
            ),
            LoadWarning::LayerPixelsLost { name, reason, .. } => {
                write!(f, "The pixels of layer \"{name}\" could not be read ({reason}); the layer is now empty.")
            }
            LoadWarning::BakeCacheRebuilt { layer } => {
                write!(f, "The stored image of stamp layer {} was unreadable and has been rebuilt.", layer.0)
            }
            LoadWarning::BlobMissing { blob, used_by } => write!(
                f,
                "Stored file {} is missing or unreadable; {} stamp(s) using it can no longer be edited.",
                blob.0,
                used_by.len()
            ),
            LoadWarning::FontFallback { stamp, family } => write!(
                f,
                "The font \"{family}\" of text stamp {} is missing; the built-in font is used instead.",
                stamp.0
            ),
            LoadWarning::UnknownLayer { set, index } => write!(
                f,
                "Layer {index} of texture set {} has an unknown format and was skipped.",
                set.0
            ),
            LoadWarning::TextFeatureUnknown { stamp, what } => write!(
                f,
                "Text stamp {} uses a {what} that this version does not know; it is shown without it. Use Save As to keep the original file intact.",
                stamp.0
            ),
            LoadWarning::StampFeatureUnknown { stamp, what } => write!(
                f,
                "Stamp {} uses a {what} that this version does not know; it is shown projected instead. Use Save As to keep the original file intact.",
                stamp.0
            ),
            LoadWarning::StickerLost { sticker, used_by, reason } => write!(
                f,
                "Sticker {} could not be loaded ({reason}); {} stamp(s) showing it keep their pictures as paint layers.",
                sticker.0,
                used_by.len()
            ),
            LoadWarning::StickerPixelsLost { name, mask: false, reason, .. } => {
                write!(f, "The picture of sticker layer \"{name}\" could not be read ({reason}); the layer is now empty.")
            }
            LoadWarning::StickerPixelsLost { name, mask: true, reason, .. } => write!(
                f,
                "The mask of sticker layer \"{name}\" could not be read ({reason}); the layer is hidden by an empty mask."
            ),
            LoadWarning::StickerFontFallback { family, .. } => write!(
                f,
                "The font \"{family}\" of a sticker's text layer is missing; the built-in font is used instead."
            ),
            LoadWarning::Import(w) => write!(f, "{w}"),
        }
    }
}

/// Fatal export failure.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Image(#[from] ImageIoError),
    #[error("zip error: {0}")]
    Zip(String),
    #[error("PSD would exceed 4 GiB ({bytes} bytes)")]
    PsdTooLarge { bytes: u64 },
    #[error("PSD supports at most 30000 px per side ({width}x{height})")]
    PsdDimension { width: u32, height: u32 },
    #[error("nothing selected for export")]
    NothingToExport,
    #[error("cancelled")]
    Cancelled,
}
