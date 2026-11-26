// Owner: core-persist (C6). Reading a `.wtep` project archive (ARCHITECTURE.md 5.23, procedure: 7.1, versions: 7.3)
// and "Extract Original Files" (R7).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use zip::ZipArchive;
use zip::result::ZipError;

use crate::blob::{BlobKind, BlobStore};
use crate::doc_build::{DocumentParts, MAX_TEXTURE_SIDE, SetParts};
use crate::document::Document;
use crate::error::{DocError, ImportError, ImportWarning, LoadWarning, ProjectError};
use crate::export::naming::{NameDeduper, is_plain_dir_name, sanitize_extension, sanitize_file_stem, split_extension};
use crate::ids::{BlobId, LayerId, StampId, StickerId, StickerLayerId};
use crate::image_io::decode_image;
use crate::import::{IMPORTER_VERSION, ImportOptions, SourceBundle, SourceFile, import_from_bundle, normalize_ref};
use crate::layer::{Layer, LayerKind};
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf, SparseBuf};
use crate::progress::Progress;
use crate::project::{LinkAtLoad, LinkFileState, LoadReport, PngReuse, composite_key};
use crate::project::manifest::{
    BlobEntry, GapEntry, ImportEntry, LayerEntry, LayerKindEntry, Manifest, SetEntry, SourceEntry, SourceFileEntry,
    StampSourceEntry, TextFxEntry,
};
use crate::project::migrate::{ParsedManifest, parse_manifest};
use crate::project::save::{MANIFEST_ENTRY, parallel_budget};
use crate::project::sticker_entries::{LinkEntry, StickerContentEntry, StickerEntry, artboard_of, transform_of};
use crate::stamp::{BakeQuality, StampObject, StampSource};
use crate::sticker::{LayerMask, MAX_PICTURE_SIDE, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::text::{DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE, FontRef, default_font_bytes};

/// Size limit of `manifest.json`.
const MANIFEST_LIMIT: u64 = 64 << 20;
/// Size limit of a source file or blob: the same limit the importer and `save` apply, so every project that could be
/// written can be opened again.
const FILE_LIMIT: u64 = crate::import::MAX_SOURCE_FILE;
#[cfg(test)]
pub(crate) const FILE_LIMIT_FOR_TESTS: u64 = FILE_LIMIT;
/// A layer PNG may be this much larger than its raw pixels.
const PNG_SLACK: u64 = 1 << 20;
/// Largest image side a layer PNG can legitimately have (`decode_image` never downscales below this).
const DECODE_MAX_DIM: u32 = 16384;
/// Original name given to the bundled font when it replaces a missing font blob.
const DEFAULT_FONT_FILE: &str = "Ubuntu-Light.ttf";

type Archive = ZipArchive<BufReader<std::fs::File>>;

/// Why an entry could not be read.
enum EntryError {
    Missing,
    TooLarge,
    Failed(String),
}

impl EntryError {
    fn reason(&self) -> String {
        match self {
            EntryError::Missing => "the entry is missing".to_owned(),
            EntryError::TooLarge => "the entry exceeds the size limit".to_owned(),
            EntryError::Failed(reason) => reason.clone(),
        }
    }
}

/// Step 1 of the load procedure: failure to open the archive is `NotAZip`.
fn open_archive(path: &Path) -> Result<Archive, ProjectError> {
    let file = std::fs::File::open(path)?;
    ZipArchive::new(BufReader::new(file)).map_err(|e| ProjectError::NotAZip(e.to_string()))
}

/// Reads one entry completely. The declared size is checked against `limit` before anything is read, and the read
/// itself is capped as well (a lying header cannot make it allocate more). The ZIP CRC is verified at the end of the
/// entry, so corrupt data surfaces as `Failed`.
fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str, limit: u64) -> Result<Vec<u8>, EntryError> {
    let mut entry = match zip.by_name(name) {
        Ok(entry) => entry,
        Err(ZipError::FileNotFound) => return Err(EntryError::Missing),
        Err(e) => return Err(EntryError::Failed(e.to_string())),
    };
    let size = entry.size();
    if size > limit {
        return Err(EntryError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(size.min(64 << 20) as usize);
    entry.by_ref().take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| EntryError::Failed(e.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(EntryError::TooLarge);
    }
    Ok(bytes)
}

fn too_large(entry: &str) -> ProjectError {
    ProjectError::EntryTooLarge { entry: entry.to_owned() }
}

/// Step 2: `manifest.json` after the version handling of section 7.3.
fn read_parsed_manifest(zip: &mut Archive) -> Result<ParsedManifest, ProjectError> {
    let bytes = read_entry(zip, MANIFEST_ENTRY, MANIFEST_LIMIT).map_err(|e| match e {
        EntryError::Missing => ProjectError::ManifestMissing,
        EntryError::TooLarge => too_large(MANIFEST_ENTRY),
        EntryError::Failed(reason) => ProjectError::ManifestInvalid(reason),
    })?;
    parse_manifest(&bytes)
}

/// Rebuilds the `SourceBundle` from the `source/*` entries. The model file must be readable (beyond the size limit:
/// `EntryTooLarge`); a sidecar or extra that is not, for whatever reason, is dropped with a logged warning (whether the
/// model survives without it is decided by the re-import).
fn read_bundle(zip: &mut Archive, source: &SourceEntry) -> Result<SourceBundle, ProjectError> {
    fn read(zip: &mut Archive, file: &SourceFileEntry) -> Result<SourceFile, EntryError> {
        let bytes = read_entry(zip, &file.file.entry, FILE_LIMIT)?;
        Ok(SourceFile { key: file.key.clone(), original_name: file.original_name.clone(), bytes: Arc::from(bytes) })
    }
    fn optional(zip: &mut Archive, files: &[SourceFileEntry]) -> Vec<SourceFile> {
        let mut out = Vec::with_capacity(files.len());
        for entry in files {
            match read(zip, entry) {
                Ok(file) => out.push(file),
                Err(e) => log::warn!(
                    "project file {} ({}) is unreadable and was dropped: {}",
                    entry.file.entry,
                    entry.key,
                    e.reason()
                ),
            }
        }
        out
    }
    let main = read(zip, &source.main).map_err(|e| match e {
        EntryError::TooLarge => too_large(&source.main.file.entry),
        e => ProjectError::ModelUnrecoverable(format!("cannot read {}: {}", source.main.file.entry, e.reason())),
    })?;
    let sidecars = optional(zip, &source.sidecars);
    let extras = optional(zip, &source.extras);
    Ok(SourceBundle { format: source.format, main, sidecars, extras, original_path: source.original_path.clone() })
}

/// Step 4: the blobs of the manifest. Unreadable ones (also ones beyond the size limit) are left out: the stamps using
/// them degrade in step 5.
pub(crate) fn read_blobs<R: Read + Seek>(zip: &mut ZipArchive<R>, entries: &[BlobEntry]) -> BlobStore {
    let mut blobs = BlobStore::default();
    for entry in entries {
        match read_entry(zip, &entry.file.entry, FILE_LIMIT) {
            Ok(bytes) => blobs.insert_with_id(entry.id, entry.kind, &entry.original_name, Arc::from(bytes)),
            Err(e) => log::warn!("blob {} ({}) is unreadable: {}", entry.id.0, entry.file.entry, e.reason()),
        }
    }
    blobs
}

/// Maps the 0..=1 progress of a sub-operation into `lo..=hi` of the load.
struct Scaled<'a> {
    inner: &'a dyn Progress,
    lo: f32,
    hi: f32,
}

impl Progress for Scaled<'_> {
    fn report(&self, fraction: f32, stage: &str) {
        let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
        self.inner.report(self.lo + (self.hi - self.lo) * f, stage);
    }

    fn cancelled(&self) -> bool {
        self.inner.cancelled()
    }
}

fn check_cancelled(progress: &dyn Progress) -> Result<(), ProjectError> {
    if progress.cancelled() { Err(ProjectError::Cancelled) } else { Ok(()) }
}

/// The manifest's sets in id order, validated: ids are exactly `0..n` and every side is `1..=MAX_TEXTURE_SIDE`.
fn ordered_sets(manifest: &Manifest) -> Result<Vec<&SetEntry>, ProjectError> {
    let mut sets: Vec<&SetEntry> = manifest.sets.iter().collect();
    sets.sort_by_key(|s| s.id);
    for (index, set) in sets.iter().enumerate() {
        if set.id.index() != index {
            return Err(ProjectError::ManifestInvalid("texture set ids are not 0..n".to_owned()));
        }
        if set.width == 0 || set.height == 0 || set.width > MAX_TEXTURE_SIDE || set.height > MAX_TEXTURE_SIDE {
            return Err(ProjectError::ManifestInvalid(format!(
                "texture set \"{}\" is {}x{}; a side must be 1..={MAX_TEXTURE_SIDE}",
                set.name, set.width, set.height
            )));
        }
    }
    Ok(sets)
}

/// One above every blob id the manifest mentions (blob entries, stamps, sticker layers): the lowest id the bundled
/// font may take when it replaces a missing font.
fn blob_floor(manifest: &Manifest) -> u32 {
    let stamps = manifest.stamps.iter().filter_map(|s| match &s.source {
        crate::project::manifest::StampSourceEntry::Image { blob } => Some(blob.0),
        crate::project::manifest::StampSourceEntry::Text(text) => Some(text.font.blob.0),
        crate::project::manifest::StampSourceEntry::Sticker { .. } => None,
    });
    let stickers = manifest.stickers.iter().flat_map(|k| &k.layers).filter_map(|l| match &l.content {
        StickerContentEntry::Picture { blob, .. } => blob.map(|b| b.0),
        StickerContentEntry::Text(text) => Some(text.font.blob.0),
        StickerContentEntry::Shape(_) => None,
    });
    let mentioned = manifest.blobs.iter().map(|b| b.id.0).chain(stamps).chain(stickers);
    mentioned.max().map_or(0, |m| m.saturating_add(1))
}

/// The bundled default font as a `FontRef`, stored in `blobs` under an id that nothing else uses: at least `floor`
/// (`blob_floor`), so a dangling reference of the file can never start to point at this font.
pub(crate) fn fallback_font(blobs: &mut BlobStore, floor: u32) -> FontRef {
    let bytes = default_font_bytes();
    let existing = blobs.iter().find(|b| b.kind == BlobKind::Font && b.bytes[..] == *bytes).map(|b| b.id);
    let blob = existing.unwrap_or_else(|| {
        let id = BlobId(blobs.iter().map(|b| b.id.0.saturating_add(1)).max().unwrap_or(0).max(floor));
        blobs.insert_with_id(id, BlobKind::Font, DEFAULT_FONT_FILE, Arc::from(bytes));
        id
    });
    FontRef { blob, face_index: 0, family: DEFAULT_FONT_FAMILY.to_owned(), style: DEFAULT_FONT_STYLE.to_owned() }
}

/// Result of step 5.
struct LoadedStamps {
    stamps: Vec<StampObject>,
    /// Stamps of the manifest that could not be kept; their layers are loaded as plain raster layers.
    dropped: BTreeSet<StampId>,
    /// Stamps whose caches must be rebuilt after the document exists.
    rebake: BTreeSet<StampId>,
}

/// `entry` written the way the app writes the same effects, without repairing any value: gaps sorted by character,
/// the later of two entries for one character kept, zero gaps (also `-0.0`) left out, and a path entry without the
/// keys of the other kinds. A file that differs from what the app writes only in these ways describes the same
/// picture, so its stored picture is kept; any repaired value still makes the result differ from the sanitized spec.
fn canonical_fx(entry: &TextFxEntry) -> TextFxEntry {
    let mut fx = entry.clone();
    let mut gaps: Vec<GapEntry> = Vec::with_capacity(fx.gaps.len());
    for gap in fx.gaps.drain(..) {
        match gaps.iter_mut().find(|g| g.after == gap.after) {
            Some(existing) => *existing = gap,
            None => gaps.push(gap),
        }
    }
    gaps.retain(|g| g.em != 0.0);
    gaps.sort_by_key(|g| g.after);
    fx.gaps = gaps;
    if let Some(path) = &mut fx.path {
        let arc = path.kind == "arc";
        let circle = path.kind == "circle";
        let curve = path.kind == "curve";
        if !arc {
            path.bend = 0.0;
        }
        if !circle {
            path.diameter = 0.0;
            path.center = [0.0, 0.0];
        }
        if !curve {
            path.points.clear();
        }
    }
    fx
}

/// Step 5: builds the stamp objects, degrading those whose image or font blob or whose sticker is missing.
fn load_stamps(
    manifest: &Manifest,
    set_count: usize,
    blobs: &mut BlobStore,
    stickers: &mut LoadedStickers,
    warnings: &mut Vec<LoadWarning>,
) -> LoadedStamps {
    let mut out = LoadedStamps { stamps: Vec::new(), dropped: BTreeSet::new(), rebake: BTreeSet::new() };
    let mut seen = BTreeSet::new();
    let mut missing: BTreeMap<BlobId, Vec<StampId>> = BTreeMap::new();
    let mut fallbacks = Vec::new();
    let mut features = Vec::new();
    for entry in &manifest.stamps {
        if !seen.insert(entry.id) {
            log::warn!("stamp id {} occurs twice in the project; the second one is ignored", entry.id.0);
            continue;
        }
        let mut stamp = entry.to_stamp();
        if stamp.home_set.index() >= set_count {
            log::warn!("stamp {} belongs to a texture set that does not exist; it is no longer editable", stamp.id.0);
            out.dropped.insert(stamp.id);
            continue;
        }
        // A round-4 feature this build does not know (an unknown or missing warp style or path kind, FEATURE_TEXT 4.3
        // item 4): the stamp loads without it (`to_spec` left it out) and is re-baked, so the picture shows what the
        // spec says; `load` forces Save As.
        // Round 6: a surface mode this build does not know (a later round): the stamp loads as projected, is re-baked
        // so the picture shows what it says, and `load` forces Save As.
        let unknown_mode = entry.placement.unknown_features();
        if !unknown_mode.is_empty() {
            out.rebake.insert(stamp.id);
            features.extend(unknown_mode.into_iter().map(|what| LoadWarning::StampFeatureUnknown { stamp: stamp.id, what }));
        }
        if let StampSourceEntry::Text(text) = &entry.source {
            let unknown = text.unknown_features();
            if !unknown.is_empty() {
                out.rebake.insert(stamp.id);
                features.extend(unknown.into_iter().map(|what| LoadWarning::TextFeatureUnknown { stamp: stamp.id, what }));
            }
            // Effects that `sanitize_fx` repaired (a value out of range, a gap beyond the text, too many curve
            // points, a NaN): the stored picture was drawn from the file's values, so it is re-baked from the repaired
            // spec, silently (nothing is lost that the app could have written). The app writes sanitized effects, which
            // read back equal, so its own files keep their stored pictures; so do valid effects written in another
            // order or with extra keys (`canonical_fx`).
            if let StampSource::Text(spec) = &stamp.source
                && TextFxEntry::from(&spec.fx) != canonical_fx(&text.fx)
            {
                out.rebake.insert(stamp.id);
            }
        }
        match &mut stamp.source {
            StampSource::Image { blob } => {
                if blobs.get(*blob).is_none() {
                    missing.entry(*blob).or_default().push(stamp.id);
                    out.dropped.insert(stamp.id);
                    continue;
                }
            }
            StampSource::Text(spec) => {
                if blobs.get(spec.font.blob).is_none() {
                    fallbacks.push(LoadWarning::FontFallback { stamp: stamp.id, family: spec.font.family.clone() });
                    spec.font = fallback_font(blobs, blob_floor(manifest));
                    out.rebake.insert(stamp.id);
                }
            }
            // Format 4: a stamp whose sticker is missing or unreadable keeps its layers as paint layers.
            StampSource::Sticker { sticker } => {
                if !stickers.stickers.contains_key(sticker) {
                    stickers.lost.entry(*sticker).or_insert_with(|| "it is not in the file".to_owned());
                    stickers.users.entry(*sticker).or_default().push(stamp.id);
                    out.dropped.insert(stamp.id);
                    continue;
                }
                if stickers.degraded.contains(sticker) {
                    out.rebake.insert(stamp.id);
                }
            }
        }
        out.stamps.push(stamp);
    }
    warnings.extend(missing.into_iter().map(|(blob, mut used_by)| {
        used_by.sort();
        LoadWarning::BlobMissing { blob, used_by }
    }));
    warnings.extend(fallbacks);
    warnings.extend(features);
    warnings.extend(std::mem::take(&mut stickers.lost).into_iter().map(|(sticker, reason)| {
        let mut used_by = stickers.users.remove(&sticker).unwrap_or_default();
        used_by.sort();
        LoadWarning::StickerLost { sticker, used_by, reason }
    }));
    out.stamps.sort_by_key(|s| s.id);
    out
}

/// Result of step 4b (format 4).
#[derive(Default)]
pub(crate) struct LoadedStickers {
    pub(crate) stickers: BTreeMap<StickerId, StickerDoc>,
    /// Owned pictures and masks read as stored: `(sticker, layer, mask, archive entry)`, for `PngReuse::stickers`.
    stored: Vec<(StickerId, StickerLayerId, bool, String)>,
    /// Stickers that could not be loaded, with the reason (warned in step 5 together with their users).
    pub(crate) lost: BTreeMap<StickerId, String>,
    /// Stamps showing a lost sticker.
    users: BTreeMap<StickerId, Vec<StampId>>,
    /// Stickers that loaded with a lost picture, mask or font: the stamps showing them are re-baked.
    degraded: BTreeSet<StickerId>,
}

/// One stored sticker PNG to decode: (sticker index, layer index, is mask, archive entry, bytes or the read error).
type StickerPng = (usize, usize, bool, String, Result<Vec<u8>, String>);

/// A transparent 1 x 1 picture: what a sticker picture layer holds when its pixels are lost (it draws nothing).
fn empty_picture() -> PicturePixels {
    PicturePixels::Owned { pixels: Arc::new(PixelBuf::new(1, 1)), version: 0 }
}

/// Decodes a stored sticker picture (straight RGBA, at most `MAX_PICTURE_SIDE` a side, never resized) or mask (8-bit
/// grey; any colour PNG is read through its first channel).
fn decode_sticker_png(bytes: &[u8]) -> Result<PixelBuf, String> {
    let image = decode_image(bytes, Some("sticker.png"), MAX_PICTURE_SIDE).map_err(|e| e.to_string())?;
    if image.resized {
        let [w, h] = image.original_size;
        return Err(format!("the stored image is {w}x{h}, larger than {MAX_PICTURE_SIDE} pixels on a side"));
    }
    Ok(image.pixels)
}

/// Step 4b (format 4): the stickers of the manifest. Owned pictures and masks are read from their PNG entries; a picture
/// that cannot be read leaves its layer empty, a mask that cannot be read (or has another size than its picture) hides
/// its layer, a text layer whose font is missing gets the bundled font, each with a warning. A sticker whose entry is
/// invalid as a whole (artboard, layer ids, limits) is lost: the stamps showing it degrade in step 5.
pub(crate) fn load_stickers<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    entries: &[StickerEntry],
    font_floor: u32,
    blobs: &mut BlobStore,
    warnings: &mut Vec<LoadWarning>,
) -> LoadedStickers {
    let mut out = LoadedStickers::default();
    let first_warning = warnings.len();
    let png_limit = MAX_PICTURE_SIDE as u64 * MAX_PICTURE_SIDE as u64 * 4 + PNG_SLACK;
    let mut docs: Vec<(StickerId, StickerDoc)> = Vec::new();
    let mut pngs: Vec<StickerPng> = Vec::new();
    for entry in entries {
        if out.lost.contains_key(&entry.id) || docs.iter().any(|(id, _)| *id == entry.id) {
            log::warn!("sticker id {} occurs twice in the project; the second one is ignored", entry.id.0);
            continue;
        }
        let (queued, warned) = (pngs.len(), warnings.len());
        match sticker_doc(entry, blobs, font_floor, docs.len(), &mut pngs, warnings) {
            Ok(doc) => docs.push((entry.id, doc)),
            Err(reason) => {
                // The PNGs this entry queued before it failed carry the index the next good sticker will get, and its
                // layer warnings are covered by `StickerLost`: both are taken back (check-P4).
                pngs.truncate(queued);
                warnings.truncate(warned);
                out.lost.insert(entry.id, reason);
            }
        }
    }
    // The stored PNGs: read sequentially (one archive reader), decoded in parallel.
    for png in &mut pngs {
        png.4 = read_entry(zip, &png.3, png_limit).map_err(|e| e.reason());
    }
    let decoded: Vec<Result<PixelBuf, String>> =
        pngs.par_iter().map(|(_, _, _, _, bytes)| bytes.as_ref().map_err(Clone::clone).and_then(|b| decode_sticker_png(b))).collect();
    // Pictures first, so that every mask can be checked against its picture's size.
    let order = |png: &StickerPng| png.2;
    let mut indices: Vec<usize> = (0..pngs.len()).collect();
    indices.sort_by_key(|&i| order(&pngs[i]));
    let mut decoded: Vec<Option<Result<PixelBuf, String>>> = decoded.into_iter().map(Some).collect();
    // Decoded sizes of the image blobs that carry a mask, each decoded once however many layers show it (check-P4).
    let mut blob_sizes: BTreeMap<BlobId, [u32; 2]> = BTreeMap::new();
    for i in indices {
        let (k, l, mask, entry, _) = &pngs[i];
        let result = decoded[i].take().expect("decoded once");
        let (id, doc) = &mut docs[*k];
        let layer = &mut doc.layers[*l];
        let lost = |reason: String| LoadWarning::StickerPixelsLost {
            sticker: *id,
            layer: layer.id,
            name: layer.name.clone(),
            mask: *mask,
            reason,
        };
        if !*mask {
            match result {
                Ok(pixels) => {
                    layer.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(pixels), version: 0 });
                    out.stored.push((*id, layer.id, false, entry.clone()));
                }
                Err(reason) => warnings.push(lost(reason)),
            }
            continue;
        }
        let size = match &layer.content {
            StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => [pixels.width, pixels.height],
            StickerContent::Picture(PicturePixels::Blob(b)) => {
                *blob_sizes.entry(*b).or_insert_with(|| blob_picture_size(blobs, *b).unwrap_or([1, 1]))
            }
            _ => [1, 1],
        };
        let mask_of = |pixels: &PixelBuf| AlphaBuf { width: pixels.width, height: pixels.height, data: pixels.data.iter().step_by(4).copied().collect() };
        let result = result.and_then(|pixels| {
            if [pixels.width, pixels.height] == size {
                Ok(mask_of(&pixels))
            } else {
                Err(format!("the mask is {}x{}, its picture {}x{}", pixels.width, pixels.height, size[0], size[1]))
            }
        });
        match result {
            Ok(alpha) => {
                layer.mask = Some(LayerMask { pixels: Arc::new(alpha), version: 0 });
                out.stored.push((*id, layer.id, true, entry.clone()));
            }
            Err(reason) => {
                warnings.push(lost(reason));
                layer.mask = Some(LayerMask { pixels: Arc::new(AlphaBuf::new(size[0], size[1])), version: 0 });
            }
        }
    }
    for (id, mut doc) in docs {
        doc.sanitize();
        match doc.check_limits() {
            Ok(()) => {
                out.stickers.insert(id, doc);
            }
            Err(e) => {
                out.stored.retain(|s| s.0 != id);
                out.lost.insert(id, e.to_string());
            }
        }
    }
    // A sticker that lost a picture, a mask or a font no longer looks like the stamps' stored pictures.
    out.degraded = warnings[first_warning..]
        .iter()
        .filter_map(|w| match w {
            LoadWarning::StickerPixelsLost { sticker, .. } | LoadWarning::StickerFontFallback { sticker, .. } => Some(*sticker),
            _ => None,
        })
        .collect();
    out
}

/// The decoded size of an image blob (only needed for a mask on an unedited picture).
fn blob_picture_size(blobs: &BlobStore, blob: BlobId) -> Option<[u32; 2]> {
    let blob = blobs.get(blob)?;
    let image = decode_image(&blob.bytes, Some(&blob.name), MAX_PICTURE_SIDE).ok()?;
    Some([image.pixels.width, image.pixels.height])
}

/// The sticker an entry describes, its owned pictures and masks still to be read (queued in `pngs`; until then such a
/// layer holds `empty_picture`). `Err(reason)` when the entry is invalid as
/// a whole.
fn sticker_doc(
    entry: &StickerEntry,
    blobs: &mut BlobStore,
    font_floor: u32,
    index: usize,
    pngs: &mut Vec<StickerPng>,
    warnings: &mut Vec<LoadWarning>,
) -> Result<StickerDoc, String> {
    let artboard = artboard_of(entry.artboard).ok_or_else(|| "its artboard is invalid".to_owned())?;
    let mut doc = StickerDoc::new(&entry.name, 1, 1);
    doc.artboard = artboard;
    doc.shape = entry.shape.to_shape();
    doc.keep = entry.keep;
    let mut ids = BTreeSet::new();
    for (l, layer) in entry.layers.iter().enumerate() {
        if !ids.insert(layer.id) {
            return Err(format!("two of its layers have the id {}", layer.id.0));
        }
        let lost = |mask: bool, reason: &str| LoadWarning::StickerPixelsLost {
            sticker: entry.id,
            layer: layer.id,
            name: layer.name.clone(),
            mask,
            reason: reason.to_owned(),
        };
        let content = match &layer.content {
            StickerContentEntry::Picture { blob: Some(blob), file: None } => {
                if blobs.get(*blob).is_some_and(|b| b.kind == BlobKind::Image) {
                    StickerContent::Picture(PicturePixels::Blob(*blob))
                } else {
                    warnings.push(lost(false, "its image file is missing"));
                    StickerContent::Picture(empty_picture())
                }
            }
            StickerContentEntry::Picture { blob: None, file: Some(file) } => {
                pngs.push((index, l, false, file.entry.clone(), Ok(Vec::new())));
                StickerContent::Picture(empty_picture())
            }
            StickerContentEntry::Picture { .. } => {
                warnings.push(lost(false, "the entry names no picture or two"));
                StickerContent::Picture(empty_picture())
            }
            StickerContentEntry::Text(text) => {
                let mut spec = text.to_spec();
                if !blobs.get(spec.font.blob).is_some_and(|b| b.kind == BlobKind::Font) {
                    warnings.push(LoadWarning::StickerFontFallback { sticker: entry.id, layer: layer.id, family: spec.font.family.clone() });
                    spec.font = fallback_font(blobs, font_floor);
                }
                StickerContent::Text(Box::new(spec))
            }
            StickerContentEntry::Shape(shape) => StickerContent::Shape(shape.to_shape()),
        };
        let mut out = StickerLayer::new(layer.id, &layer.name, content);
        out.visible = layer.visible;
        out.locked = layer.locked;
        out.opacity = layer.opacity;
        out.blend = layer.blend;
        out.transform = transform_of(layer.transform);
        out.clip = layer.clip;
        out.adjustments = layer.adjustments.iter().map(|a| a.to_adjustment()).collect();
        out.styles = layer.styles.as_ref().map(|s| s.to_styles()).unwrap_or_default();
        if let Some(mask) = &layer.mask {
            if matches!(out.content, StickerContent::Picture(_)) {
                pngs.push((index, l, true, mask.entry.clone(), Ok(Vec::new())));
            } else {
                log::warn!("sticker {} layer {}: a mask on a text or shape layer is ignored", entry.id.0, layer.id.0);
            }
        }
        doc.layers.push(out);
    }
    let next = ids.last().map_or(1, |id| id.0.saturating_add(1));
    doc.next_layer = entry.next_layer.max(next);
    Ok(doc)
}

/// What a layer entry becomes.
#[derive(Copy, Clone)]
enum Target {
    /// A paint layer (also the fate of the layers of a dropped stamp).
    Raster,
    /// A linked stamp (`text == false`) or text layer.
    Cache { stamp: StampId, text: bool },
}

/// A layer entry that passed the structural checks, with the bytes of its PNG.
struct PendingLayer {
    entry: LayerEntry,
    target: Target,
    /// `None`: the layer stores no pixels. `Some(Err)`: the entry could not be read.
    png: Option<Result<Vec<u8>, String>>,
}

/// Decoded pixels of one layer.
enum Decoded {
    Raster(PixelBuf),
    Cache(SparseBuf),
    Failed(String),
}

/// Decodes a layer PNG and checks it against the declared rectangle (`[x, y, w, h]`, inside the set).
fn decode_pixels(bytes: &[u8], rect: [i32; 4], width: u32, height: u32) -> Result<CroppedBuf, String> {
    let [x, y, w, h] = rect;
    let inside = x >= 0 && y >= 0 && w > 0 && h > 0 && x as i64 + w as i64 <= width as i64 && y as i64 + h as i64 <= height as i64;
    if !inside {
        return Err(format!("the stored rectangle {x},{y} {w}x{h} does not lie inside the {width}x{height} texture"));
    }
    let image = decode_image(bytes, Some("layer.png"), DECODE_MAX_DIM).map_err(|e| e.to_string())?;
    if image.resized || image.pixels.width != w as u32 || image.pixels.height != h as u32 {
        return Err(format!(
            "the stored image is {}x{}, its rectangle is {w}x{h}",
            image.original_size[0], image.original_size[1]
        ));
    }
    Ok(CroppedBuf { rect: IRect::from_xywh(x, y, w as u32, h as u32), data: image.pixels.data })
}

fn decode_layer(pending: &PendingLayer, width: u32, height: u32) -> Decoded {
    let cropped = match (&pending.png, &pending.entry.pixels) {
        (Some(Ok(bytes)), Some(pixels)) => decode_pixels(bytes, pixels.rect, width, height),
        (Some(Err(reason)), _) => Err(reason.clone()),
        _ => Ok(CroppedBuf::empty()),
    };
    match (cropped, pending.target) {
        (Err(reason), _) => Decoded::Failed(reason),
        (Ok(cropped), Target::Cache { .. }) => Decoded::Cache(SparseBuf::from_cropped(cropped)),
        // A paint layer without stored pixels stays unallocated (it is fully transparent).
        (Ok(cropped), Target::Raster) if cropped.is_empty() => Decoded::Raster(PixelBuf::new(0, 0)),
        (Ok(cropped), Target::Raster) => {
            if cropped.rect == IRect::from_size(width, height) {
                Decoded::Raster(PixelBuf { width, height, data: cropped.data })
            } else {
                Decoded::Raster(cropped.to_dense(width, height))
            }
        }
    }
}

/// Step 6 for one set: parses the layer entries bottom to top, reads and decodes their pixels.
#[allow(clippy::too_many_arguments)]
fn load_set_layers(
    zip: &mut Archive,
    set: &SetEntry,
    stamps: &LoadedStamps,
    layer_ids: &mut BTreeSet<LayerId>,
    linked: &mut BTreeSet<(StampId, usize)>,
    rebake: &mut BTreeSet<StampId>,
    warnings: &mut Vec<LoadWarning>,
    stored: &mut Vec<(LayerId, String, [i32; 4])>,
    progress: &dyn Progress,
) -> Result<Vec<Layer>, ProjectError> {
    let (width, height) = (set.width, set.height);
    let set_index = set.id.index();
    let png_limit = width as u64 * height as u64 * 4 + PNG_SLACK;

    // Structure first, sequentially (the archive has one reader).
    let mut pending = Vec::with_capacity(set.layers.len());
    for (index, value) in set.layers.iter().enumerate() {
        let unknown = LoadWarning::UnknownLayer { set: set.id, index };
        let Ok(entry) = serde_json::from_value::<LayerEntry>(value.clone()) else {
            warnings.push(unknown);
            continue;
        };
        let target = match entry.kind {
            LayerKindEntry::Raster => Target::Raster,
            LayerKindEntry::Stamp { stamp } | LayerKindEntry::Text { stamp } => {
                if stamps.dropped.contains(&stamp) {
                    Target::Raster
                } else if let Some(object) = stamps.stamps.iter().find(|s| s.id == stamp) {
                    if linked.contains(&(stamp, set_index)) {
                        // A stamp has at most one linked layer per set.
                        warnings.push(unknown);
                        continue;
                    }
                    Target::Cache { stamp, text: matches!(object.source, StampSource::Text(_)) }
                } else {
                    warnings.push(unknown);
                    continue;
                }
            }
        };
        if !layer_ids.insert(entry.id) {
            warnings.push(unknown);
            continue;
        }
        if let Target::Cache { stamp, .. } = target {
            linked.insert((stamp, set_index));
        }
        let png = match &entry.pixels {
            None => None,
            // An entry beyond the limit is as unreadable as a corrupt one: only this layer's pixels are lost.
            Some(pixels) => match read_entry(zip, &pixels.file.entry, png_limit) {
                Ok(bytes) => Some(Ok(bytes)),
                Err(e) => Some(Err(e.reason())),
            },
        };
        pending.push(PendingLayer { entry, target, png });
    }

    // Decoding in parallel, a bounded number of full-size buffers at a time.
    let mut layers = Vec::with_capacity(pending.len());
    for chunk in pending.chunks_mut(parallel_budget(width, height)) {
        check_cancelled(progress)?;
        let decoded: Vec<Decoded> = chunk.par_iter().map(|p| decode_layer(p, width, height)).collect();
        for (pending, decoded) in chunk.iter_mut().zip(decoded) {
            pending.png = None;
            let entry = &pending.entry;
            if let Some(pixels) = &entry.pixels
                && !matches!(decoded, Decoded::Failed(_))
            {
                stored.push((entry.id, pixels.file.entry.clone(), pixels.rect));
            }
            let kind = match (decoded, pending.target) {
                (Decoded::Raster(buf), _) => LayerKind::Raster(Arc::new(buf)),
                (Decoded::Cache(cache), Target::Cache { stamp, text }) => stamp_kind(stamp, text, cache),
                (Decoded::Cache(cache), Target::Raster) => LayerKind::Raster(Arc::new(cache.to_dense(width, height))),
                (Decoded::Failed(reason), Target::Raster) => {
                    warnings.push(LoadWarning::LayerPixelsLost { layer: entry.id, name: entry.name.clone(), reason });
                    LayerKind::empty_raster()
                }
                (Decoded::Failed(reason), Target::Cache { stamp, text }) => {
                    log::warn!("the cache of layer {} is unreadable ({reason}); it will be rebuilt", entry.id.0);
                    warnings.push(LoadWarning::BakeCacheRebuilt { layer: entry.id });
                    rebake.insert(stamp);
                    stamp_kind(stamp, text, SparseBuf::empty())
                }
            };
            let mut layer = Layer::new(entry.id, &entry.name, kind);
            layer.visible = entry.visible;
            layer.locked = entry.locked;
            layer.alpha_lock = entry.alpha_lock;
            layer.opacity = if entry.opacity.is_finite() { entry.opacity.clamp(0.0, 1.0) } else { 1.0 };
            layer.blend = entry.blend;
            layers.push(layer);
        }
    }
    Ok(layers)
}

fn stamp_kind(stamp: StampId, text: bool, cache: SparseBuf) -> LayerKind {
    let cache = Arc::new(cache);
    if text { LayerKind::Text { stamp, cache } } else { LayerKind::Stamp { stamp, cache } }
}

/// The warning for a re-imported model whose `uv_hash` differs from the saved one: `OlderGeneratedUvLayout` when the
/// project was saved by an older importer and the layout is an automatic one now as then (the generated layout is
/// what changes between importer versions, section 6.1), `MeshUvChanged` for any other cause.
fn uv_change_warning(saved: &ImportEntry, warnings: &[ImportWarning]) -> LoadWarning {
    let generated = warnings.contains(&ImportWarning::FallbackUvsGenerated);
    if saved.importer_version < IMPORTER_VERSION && saved.generate_fallback_uvs && generated {
        LoadWarning::OlderGeneratedUvLayout { saved: saved.importer_version }
    } else {
        LoadWarning::MeshUvChanged
    }
}

/// Opens a .wtep: re-imports the model from the stored source files, loads layers, stamps and blobs (section 7.1).
///
/// Fatal: not a ZIP (`NotAZip`), missing or invalid manifest, a file that needs a newer reader (`TooNew`), a model
/// that can no longer be imported or whose texture set count changed (`ModelUnrecoverable`), a manifest or model file
/// beyond its size limit (`EntryTooLarge`). Everything else degrades and is reported in `LoadReport::warnings`. The returned document
/// is unmodified (`is_modified() == false`) with an empty history.
pub fn load(path: &Path, progress: &dyn Progress) -> Result<LoadReport, ProjectError> {
    progress.report(0.0, "Opening project");
    let mut zip = open_archive(path)?;
    let ParsedManifest { manifest, found_version, newer } = read_parsed_manifest(&mut zip)?;
    let mut warnings = Vec::new();
    if newer {
        warnings.push(LoadWarning::NewerFormat { found: found_version });
    }
    let set_entries = ordered_sets(&manifest)?;
    check_cancelled(progress)?;

    // Step 3: the model, re-imported from the stored source files without decoding its textures.
    progress.report(0.02, "Reading model files");
    let bundle = read_bundle(&mut zip, &manifest.source)?;
    let import_options = ImportOptions {
        generate_fallback_uvs: manifest.import.generate_fallback_uvs,
        max_texture_dim: manifest.import.max_texture_dim,
    };
    let model = import_from_bundle(&bundle, &import_options, false, &Scaled { inner: progress, lo: 0.05, hi: 0.3 })
        .map_err(|e| match e {
            ImportError::Cancelled => ProjectError::Cancelled,
            other => ProjectError::ModelUnrecoverable(other.to_string()),
        })?;
    // Material libraries the project lists but whose bytes are not in the file: only those mean a damaged project.
    let lost_libraries: BTreeSet<String> = manifest
        .source
        .sidecars
        .iter()
        .map(|f| f.key.clone())
        .filter(|key| !bundle.sidecars.iter().any(|s| &s.key == key))
        .collect();
    drop(bundle);
    if model.mesh.sets.len() != set_entries.len() {
        return Err(ProjectError::ModelUnrecoverable("texture set count changed".to_owned()));
    }
    if format!("{:016x}", model.mesh.uv_hash) != manifest.import.uv_hash.to_ascii_lowercase() {
        warnings.push(uv_change_warning(&manifest.import, &model.warnings));
    }
    // Facts about the stored model file (generated normals or UVs, ...) were shown when it was imported and come back
    // unchanged on every re-import: only the ones that can mean a damaged project are reported again. A material
    // library that was already missing at import was never stored, so its `MissingMtl` is such a fact too; one the
    // project lists but no longer holds is reported.
    warnings.extend(
        model
            .warnings
            .iter()
            .filter(|w| match w {
                ImportWarning::MissingMtl(reference) => lost_libraries.contains(&normalize_ref(reference.trim())),
                other => !other.is_model_fact(),
            })
            .cloned()
            .map(LoadWarning::Import),
    );
    check_cancelled(progress)?;

    // Steps 4 and 5: blobs and stamps.
    progress.report(0.3, "Reading stamps");
    let mut blobs = read_blobs(&mut zip, &manifest.blobs);
    let mut stickers = load_stickers(&mut zip, &manifest.stickers, blob_floor(&manifest), &mut blobs, &mut warnings);
    let mut stamps = load_stamps(&manifest, set_entries.len(), &mut blobs, &mut stickers, &mut warnings);
    let mut rebake = std::mem::take(&mut stamps.rebake);

    // Step 6: layers.
    let mut layer_ids = BTreeSet::new();
    let mut linked = BTreeSet::new();
    let mut sets = Vec::with_capacity(set_entries.len());
    let mut stored = Vec::new();
    for (index, set) in set_entries.iter().enumerate() {
        check_cancelled(progress)?;
        progress.report(0.3 + 0.35 * index as f32 / set_entries.len() as f32, "Reading layers");
        let layers = load_set_layers(
            &mut zip,
            set,
            &stamps,
            &mut layer_ids,
            &mut linked,
            &mut rebake,
            &mut warnings,
            &mut stored,
            progress,
        )?;
        sets.push(SetParts { name: set.name.clone(), width: set.width, height: set.height, layers, active: None });
    }
    // P5c: which link copies the archive holds (read later by the app with `read_link_archive`, never here).
    let link_archives: BTreeSet<String> =
        manifest.links.iter().filter_map(|l| l.archive.clone()).filter(|a| zip.by_name(a).is_ok()).collect();
    drop(zip);

    // Step 7: every stamp needs a linked layer in its home set before the document is assembled.
    let max_id = layer_ids
        .iter()
        .map(|l| l.0)
        .chain(stamps.stamps.iter().map(|s| s.id.0))
        .chain(stickers.stickers.keys().map(|k| k.0))
        .max()
        .unwrap_or(0);
    let mut next_id = manifest.next_id.max(max_id.saturating_add(1));
    for stamp in &stamps.stamps {
        let home = stamp.home_set.index();
        if linked.insert((stamp.id, home)) {
            let text = matches!(stamp.source, StampSource::Text(_));
            sets[home].layers.push(Layer::new(LayerId(next_id), &stamp.name, stamp_kind(stamp.id, text, SparseBuf::empty())));
            next_id += 1;
            rebake.insert(stamp.id);
        }
    }
    for (parts, entry) in sets.iter_mut().zip(&set_entries) {
        let exists = |id: &LayerId| parts.layers.iter().any(|l| l.id == *id);
        parts.active = entry.active_layer.filter(exists).or_else(|| parts.layers.last().map(|l| l.id));
    }

    let parts = DocumentParts {
        name: manifest.name.clone(),
        model,
        sets,
        stamps: stamps.stamps,
        blobs,
        ui_state: manifest.ui_state.clone(),
        next_id,
        stickers: std::mem::take(&mut stickers.stickers),
    };
    let mut document =
        Document::from_parts(parts, &Scaled { inner: progress, lo: 0.65, hi: 0.95 }).map_err(|e| match e {
            DocError::Cancelled => ProjectError::Cancelled,
            other => ProjectError::ManifestInvalid(other.to_string()),
        })?;

    // The images of this file that the next save may copy (`PngReuse`): every layer image that was read as stored,
    // with the version the layer has now (a re-baked cache gets a newer one below and is encoded again). The
    // composites only when every layer came back exactly as saved.
    if let Some(mut reuse) = PngReuse::for_file(path) {
        for (id, entry, rect) in stored {
            if let Some(version) = document.layer_version(id) {
                reuse.layers.insert(id, (version, entry, rect));
            }
        }
        // Sticker pictures and masks read as stored, by the picture version `from_parts` gave them.
        for (sticker, layer, mask, entry) in stickers.stored {
            let Some(layer) = document.sticker(sticker).and_then(|d| d.layer(layer)) else { continue };
            let version = if mask {
                layer.mask.as_ref().map(|m| m.version)
            } else {
                match &layer.content {
                    StickerContent::Picture(PicturePixels::Owned { version, .. }) => Some(*version),
                    _ => None,
                }
            };
            if let Some(version) = version {
                reuse.stickers.insert(version, entry);
            }
        }
        if rebake.is_empty() && warnings.is_empty() {
            let snapshot = document.snapshot();
            for (set, entry) in snapshot.sets.iter().zip(&set_entries) {
                if let Some(flat) = &entry.flattened {
                    reuse.flattened.insert(set.id, (composite_key(set), flat.entry.clone()));
                }
            }
            if let Some(first) = snapshot.sets.first() {
                reuse.thumbnail = Some((composite_key(first), "thumbnail.png".to_owned()));
            }
        }
        document.set_png_reuse(Some(reuse));
    }

    if !rebake.is_empty() {
        progress.report(0.95, "Rebuilding stamps");
        for &id in &rebake {
            if let Err(e) = document.rebake_stamp(id, BakeQuality::Full) {
                log::warn!("stamp {} could not be rebuilt: {e}", id.0);
            }
        }
        // The rebuilt caches belong to the freshly loaded state: composite them and start with nothing to upload.
        document.recomposite_all();
        for set in document.set_ids() {
            document.take_gpu_dirty(set);
        }
    }
    document.history.clear();
    document.mark_saved(document.history_state());
    // Step 8 (P5c): the editor links, with what is on disk for each. Nothing is written.
    let (links, link_report) = load_links(path, &manifest.links, &link_archives, &document);
    document.set_loaded_links(links);
    document.mark_links_saved(document.links_revision());
    progress.report(1.0, "Project opened");
    // A newer format, or a text or stamp feature this build cannot keep: a plain Save would overwrite the newer file
    // with less.
    let unknown_text = warnings
        .iter()
        .any(|w| matches!(w, LoadWarning::TextFeatureUnknown { .. } | LoadWarning::StampFeatureUnknown { .. }));
    Ok(LoadReport { document, warnings, force_save_as: newer || unknown_text, links: link_report })
}

/// Step 8 of the load procedure (P5c, STICKER_PLAN 5.3 "At load"): the links of `entries` whose sticker was loaded,
/// and per link what is at its path: the synced content (`Present`; a copy with another time counts), nothing
/// (`Missing`), or other content (`Changed`). An entry that cannot be a link (a damaged or hand-edited file) or whose
/// sticker is gone is left out with a log line. Only reads files.
fn load_links(
    project: &Path,
    entries: &[LinkEntry],
    archives: &BTreeSet<String>,
    document: &Document,
) -> (BTreeMap<StickerId, crate::link::StickerLink>, Vec<LinkAtLoad>) {
    let mut links = BTreeMap::new();
    let mut report = Vec::new();
    for entry in entries {
        if document.sticker(entry.sticker).is_none() || links.contains_key(&entry.sticker) {
            log::warn!("the link of sticker {} is left out: no such sticker, or a second link", entry.sticker.0);
            continue;
        }
        let link = match entry.to_link() {
            Ok(link) => link,
            Err(why) => {
                log::warn!("the link of sticker {} is left out: {why}", entry.sticker.0);
                continue;
            }
        };
        let path = link.file.path(Some(project));
        let state = match path.as_deref().map(std::fs::metadata) {
            Some(Ok(meta)) if meta.is_file() => {
                let modified = meta.modified().ok().and_then(crate::link::unix_ns);
                let same_stamp = meta.len() == link.synced.size && modified.is_some() && modified == link.synced.modified_ns;
                let same_content = same_stamp
                    || path
                        .as_deref()
                        .and_then(|p| std::fs::read(p).ok())
                        .is_some_and(|b| b.len() as u64 == link.synced.size && crc32fast::hash(&b) == link.synced.crc32);
                if same_content { LinkFileState::Present } else { LinkFileState::Changed }
            }
            _ => LinkFileState::Missing,
        };
        let archive = entry.archive.clone().filter(|a| archives.contains(a));
        report.push(LinkAtLoad { sticker: entry.sticker, state, path, archive });
        links.insert(entry.sticker, link);
    }
    (links, report)
}

/// The stored copy of a linked file (`LinkAtLoad::archive`) from the project at `project` (P5c: the app restores a
/// missing link file with it). Reads only.
pub fn read_link_archive(project: &Path, entry: &str) -> Result<Vec<u8>, ProjectError> {
    let mut zip = open_archive(project)?;
    read_entry(&mut zip, entry, FILE_LIMIT).map_err(|e| ProjectError::ManifestInvalid(format!("{entry}: {}", e.reason())))
}

/// Reads only `manifest.json` of a .wtep (after version handling and migration).
pub fn read_manifest(path: &Path) -> Result<Manifest, ProjectError> {
    let mut zip = open_archive(path)?;
    Ok(read_parsed_manifest(&mut zip)?.manifest)
}

/// The folder (relative, `/`-separated, "" = the destination itself) a source file with this key is extracted to:
/// the key's own folders when the key is a plain relative path, else none.
fn key_dir(key: &str) -> String {
    let mut parts: Vec<&str> = key.split('/').collect();
    parts.pop();
    if parts.iter().all(|p| is_plain_dir_name(p)) { parts.join("/") } else { String::new() }
}

/// A safe file name for an extracted file. The original name is kept unchanged when Windows accepts it (the model's
/// files refer to each other by these names: `mtllib`, `map_Kd`, glTF `uri`); otherwise the sanitized stem of the
/// original name plus its (sanitized) extension.
fn extracted_name(original_name: &str, fallback: &str) -> String {
    let file_part = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or(s).to_owned();
    let mut name = file_part(original_name);
    if name.trim().is_empty() {
        name = file_part(fallback);
    }
    if is_plain_dir_name(&name) && name.encode_utf16().count() <= MAX_NAME_UNITS {
        return name;
    }
    let (stem, ext) = split_extension(&name);
    let (stem, ext) = (sanitize_file_stem(stem), sanitize_extension(ext));
    if ext.is_empty() { stem } else { format!("{stem}.{ext}") }
}

/// Longest file name (in UTF-16 units) Windows file systems accept.
const MAX_NAME_UNITS: usize = 255;

/// The extracted name of a blob: fonts stored without a font extension (projects of earlier versions named font
/// blobs "Family-Style") get the one their container format calls for, so the extracted file can be installed.
fn blob_file_name(blob: &crate::blob::Blob) -> String {
    let name = extracted_name(&blob.name, &format!("blob{}", blob.id.0));
    if blob.kind != BlobKind::Font {
        return name;
    }
    let (_, ext) = split_extension(&name);
    let known = ["ttf", "otf", "ttc", "otc", "woff", "woff2"].iter().any(|e| ext.eq_ignore_ascii_case(e));
    if known { name } else { format!("{name}.{}", crate::text::font_file_extension(&blob.bytes)) }
}

/// R7 "Extract Original Files", working on in-memory data (so it also works for a document that was never saved): writes
/// the model file, every sidecar and every extra at their relative paths, every blob under `stamps\` (images) or
/// `fonts\` (fonts) under its original name (sanitized only when Windows would refuse it), and every finished sticker
/// picture of `stickers` (`(sticker name, picture)`, `Document::sticker_extract_pictures`) as `stickers\<name>.png`
/// (STICKER_PLAN 2.11), into `dest_dir` (created if missing). Returns the written paths (7.1).
///
/// The files refer to each other by name, so they are never renamed one by one: with `overwrite == false`, when any
/// of them already exists in `dest_dir`, the whole set goes into a fresh sub-folder `<model stem>_original` (`_2`,
/// `_3`, ... when that exists too) instead, and nothing existing is replaced. With `overwrite == true` existing files
/// are replaced. Two files of one call that would get the same name are told apart by `_2`, `_3`, ... suffixes.
///
/// Order of the returned paths: model file, sidecars, extras, blobs by id, sticker pictures in the given order.
pub fn extract_bundle(
    bundle: &SourceBundle,
    blobs: &BlobStore,
    stickers: &[(String, PixelBuf)],
    dest_dir: &Path,
    overwrite: bool,
) -> Result<Vec<PathBuf>, ProjectError> {
    let sticker_pngs = stickers
        .iter()
        .map(|(name, picture)| {
            let png = crate::image_io::encode_png(picture.width, picture.height, &picture.data, crate::image_io::PngLevel::Balanced)?;
            Ok((format!("{}.png", sanitize_file_stem(name)), png))
        })
        .collect::<Result<Vec<(String, Vec<u8>)>, crate::error::ImageIoError>>()?;
    // The plan: (sub-folder, unique name, bytes).
    let main_name = extracted_name(&bundle.main.original_name, &bundle.main.key);
    let mut plan: Vec<(String, String, &[u8])> = vec![(String::new(), main_name.clone(), &bundle.main.bytes[..])];
    for file in bundle.sidecars.iter().chain(&bundle.extras) {
        plan.push((key_dir(&file.key), extracted_name(&file.original_name, &file.key), &file.bytes[..]));
    }
    for blob in blobs.iter() {
        let sub_dir = match blob.kind {
            BlobKind::Image => "stamps",
            BlobKind::Font => "fonts",
        };
        plan.push((sub_dir.to_owned(), blob_file_name(blob), &blob.bytes[..]));
    }
    for (name, png) in &sticker_pngs {
        plan.push(("stickers".to_owned(), name.clone(), &png[..]));
    }
    let mut names = NameDeduper::default();
    for (sub_dir, name, _) in &mut plan {
        *name = names.unique(sub_dir, name, |_| false);
    }
    let path_of = |root: &Path, sub_dir: &str, name: &str| -> PathBuf {
        sub_dir.split('/').filter(|p| !p.is_empty()).fold(root.to_path_buf(), |dir, part| dir.join(part)).join(name)
    };

    let mut root = dest_dir.to_path_buf();
    if !overwrite && plan.iter().any(|(sub_dir, name, _)| path_of(&root, sub_dir, name).exists()) {
        let base = format!("{}_original", sanitize_file_stem(split_extension(&main_name).0));
        let mut n = 1;
        loop {
            let candidate = if n == 1 { dest_dir.join(&base) } else { dest_dir.join(format!("{base}_{n}")) };
            if !candidate.exists() {
                root = candidate;
                break;
            }
            n += 1;
        }
        log::info!("files of the model already exist in {}; extracting into {}", dest_dir.display(), root.display());
    }

    let mut written = Vec::with_capacity(plan.len());
    for (sub_dir, name, bytes) in plan {
        let path = path_of(&root, &sub_dir, &name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, bytes)?;
        written.push(path);
    }
    Ok(written)
}

/// Thin wrapper: reads `source/*`, the extras and the blobs of a saved .wtep (no import, no layers, so no sticker
/// pictures: those need the document) and calls extract_bundle.
///
/// Entries are taken from the manifest. The model file must be readable (`ModelUnrecoverable` otherwise); other
/// unreadable files are skipped with a logged warning.
pub fn extract_sources(path: &Path, dest_dir: &Path, overwrite: bool) -> Result<Vec<PathBuf>, ProjectError> {
    let mut zip = open_archive(path)?;
    let manifest = read_parsed_manifest(&mut zip)?.manifest;
    let bundle = read_bundle(&mut zip, &manifest.source)?;
    let blobs = read_blobs(&mut zip, &manifest.blobs);
    drop(zip);
    extract_bundle(&bundle, &blobs, &[], dest_dir, overwrite)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use serde_json::Value;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;
    use crate::doc_build::TextureChoice;
    use crate::image_io::{PngLevel, encode_png};
    use crate::import::ImportOptions;
    use crate::progress::NoProgress;
    use crate::project::save::testkit::{
        assert_same_document, cube_doc, noise_texture, obj_bundle, paint_block, source_file, two_set_doc,
    };
    use crate::project::save::{save, zip_deflated, zip_stored};
    use crate::project::{FORMAT_VERSION, SaveOptions};
    use crate::samples;
    use crate::testutil::temp_dir;

    fn options() -> SaveOptions {
        SaveOptions { png: PngLevel::Fast, write_flattened: true, keep_backup: false, now_unix: 7, app_version: "test".to_owned() }
    }

    fn save_doc(doc: &Document, path: &Path) {
        save(&doc.snapshot(), path, &options(), &NoProgress).unwrap();
    }

    /// Every entry of an archive, in order, with its compression method.
    fn entries(bytes: &[u8]) -> Vec<(String, CompressionMethod, Vec<u8>)> {
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut entry = zip.by_index(i).unwrap();
                let name = entry.name().unwrap().into_owned();
                let method = entry.compression();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                (name, method, data)
            })
            .collect()
    }

    /// Rewrites the archive at `path`; `edit` returns the new content of an entry or `None` to drop it.
    fn rewrite(path: &Path, mut edit: impl FnMut(&str, Vec<u8>) -> Option<Vec<u8>>) {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, method, data) in entries(&std::fs::read(path).unwrap()) {
            if let Some(data) = edit(&name, data) {
                let options = if method == CompressionMethod::Stored { zip_stored() } else { zip_deflated() };
                zip.start_file(name, options).unwrap();
                zip.write_all(&data).unwrap();
            }
        }
        std::fs::write(path, zip.finish().unwrap().into_inner()).unwrap();
    }

    /// Edits `manifest.json` as JSON.
    fn edit_manifest(path: &Path, edit: impl FnOnce(&mut Value)) {
        let mut edit = Some(edit);
        rewrite(path, |name, data| {
            if name != MANIFEST_ENTRY {
                return Some(data);
            }
            let mut value: Value = serde_json::from_slice(&data).unwrap();
            (edit.take().unwrap())(&mut value);
            Some(serde_json::to_vec_pretty(&value).unwrap())
        });
    }

    fn load_err(path: &Path) -> ProjectError {
        match load(path, &NoProgress) {
            Ok(_) => panic!("the project loaded"),
            Err(e) => e,
        }
    }

    /// A saved cube project with a painted "Paint 1" and a second painted layer above it.
    fn painted_cube(dir: &Path) -> (PathBuf, Document, LayerId, LayerId) {
        let mut doc = cube_doc(&noise_texture(32, 32, 8));
        let paint = doc.sets[0].active.unwrap();
        paint_block(&mut doc, paint, IRect::new(2, 2, 20, 12), [250, 0, 0, 255]);
        let top = doc.add_raster_layer(crate::ids::TextureSetId(0), Some("Top")).unwrap();
        paint_block(&mut doc, top, IRect::new(10, 10, 30, 30), [0, 0, 250, 200]);
        doc.recomposite_all();
        let path = dir.join("cube.wtep");
        save_doc(&doc, &path);
        (path, doc, paint, top)
    }

    #[test]
    fn not_a_zip() {
        let dir = temp_dir("load-notzip");
        let (path, _, _, _) = painted_cube(&dir);
        let bytes = std::fs::read(&path).unwrap();
        let bad = dir.join("bad.wtep");
        for data in [b"garbage, not a zip archive at all".to_vec(), Vec::new(), bytes[..bytes.len() / 2].to_vec()] {
            std::fs::write(&bad, &data).unwrap();
            assert!(matches!(load_err(&bad), ProjectError::NotAZip(_)), "{} bytes", data.len());
            assert!(matches!(read_manifest(&bad), Err(ProjectError::NotAZip(_))));
        }
        assert!(matches!(load_err(&dir.join("does not exist.wtep")), ProjectError::Io(_)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn manifest_missing_invalid_and_too_new() {
        let dir = temp_dir("load-manifest");
        let (path, _, _, _) = painted_cube(&dir);
        let original = std::fs::read(&path).unwrap();
        assert_eq!(read_manifest(&path).unwrap().name, "cube");

        rewrite(&path, |name, data| (name != MANIFEST_ENTRY).then_some(data));
        assert!(matches!(load_err(&path), ProjectError::ManifestMissing));
        assert!(matches!(extract_sources(&path, &dir.join("x"), false), Err(ProjectError::ManifestMissing)));

        std::fs::write(&path, &original).unwrap();
        rewrite(&path, |name, data| Some(if name == MANIFEST_ENTRY { b"{ not json".to_vec() } else { data }));
        assert!(matches!(load_err(&path), ProjectError::ManifestInvalid(_)));

        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| m["format"] = "zip".into());
        assert!(matches!(load_err(&path), ProjectError::ManifestInvalid(_)));

        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| {
            m["format_version"] = 999.into();
            m["min_reader_version"] = 999.into();
        });
        assert!(matches!(load_err(&path), ProjectError::TooNew { found: 999, supported: FORMAT_VERSION }));
        assert!(matches!(read_manifest(&path), Err(ProjectError::TooNew { .. })));

        // Written by a newer version that this one can still read: opened, flagged, Save As forced.
        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| {
            m["format_version"] = (FORMAT_VERSION + 1).into();
            m["min_reader_version"] = FORMAT_VERSION.into();
            m["field_from_the_future"] = serde_json::json!([1, 2, 3]);
        });
        let report = load(&path, &NoProgress).unwrap();
        assert_eq!(report.warnings, [LoadWarning::NewerFormat { found: FORMAT_VERSION + 1 }]);
        assert!(report.force_save_as);

        // Texture set ids that are not 0..n.
        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| m["sets"][0]["id"] = 3.into());
        assert!(matches!(load_err(&path), ProjectError::ManifestInvalid(_)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn flipped_byte_in_a_layer_png_loses_only_that_layer() {
        let dir = temp_dir("load-flipped");
        let (path, doc, paint, top) = painted_cube(&dir);
        let entry = format!("sets/s00/l{}.png", paint.0);
        let mut bytes = std::fs::read(&path).unwrap();
        let png = entries(&bytes).into_iter().find(|(name, _, _)| *name == entry).unwrap().2;
        // Layer PNGs are STORED: the entry's bytes appear verbatim in the archive.
        let at = bytes.windows(png.len()).position(|w| w == png.as_slice()).unwrap();
        bytes[at + png.len() / 2] ^= 0x5a;
        std::fs::write(&path, &bytes).unwrap();

        let report = load(&path, &NoProgress).unwrap();
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(
            matches!(&report.warnings[0], LoadWarning::LayerPixelsLost { layer, name, .. } if *layer == paint && name == "Paint 1"),
            "{:?}",
            report.warnings
        );
        let loaded = report.document;
        let lost = loaded.layer(paint).unwrap();
        assert!(lost.raster().is_some_and(|b| b.data.iter().all(|&v| v == 0)), "the lost layer is transparent");
        assert_eq!(lost.name, "Paint 1");
        assert!(crate::testutil::layer_dense(&loaded, top).data == crate::testutil::layer_dense(&doc, top).data);
        let base = doc.sets[0].layers[0].id;
        assert!(crate::testutil::layer_dense(&loaded, base).data == crate::testutil::layer_dense(&doc, base).data);
        // The composite is rebuilt from what was loaded.
        let expected = crate::composite::flatten_layers(&loaded.sets[0].layers, 32, 32, false);
        assert!(loaded.sets[0].composite() == &expected);
        assert!(!loaded.is_modified());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_layer_rectangles_and_oversized_entries() {
        let dir = temp_dir("load-rects");
        let (path, doc, paint, top) = painted_cube(&dir);
        let original = std::fs::read(&path).unwrap();
        let index_of = |id: LayerId| doc.sets[0].layer_index(id).unwrap();

        // The declared rectangle disagrees with the PNG size, or lies outside the set.
        for rect in [[2, 2, 5, 5], [30, 30, 18, 10], [-1, 0, 18, 10]] {
            std::fs::write(&path, &original).unwrap();
            let i = index_of(paint);
            edit_manifest(&path, |m| m["sets"][0]["layers"][i]["pixels"]["rect"] = serde_json::json!(rect));
            let report = load(&path, &NoProgress).unwrap();
            assert!(
                matches!(report.warnings.as_slice(), [LoadWarning::LayerPixelsLost { layer, .. }] if *layer == paint),
                "{rect:?}: {:?}",
                report.warnings
            );
        }

        // A layer entry bigger than its set can hold is refused before it is read: only that layer is lost.
        std::fs::write(&path, &original).unwrap();
        let entry = format!("sets/s00/l{}.png", top.0);
        rewrite(&path, |name, data| Some(if name == entry { vec![0; 32 * 32 * 4 + (1 << 20) + 1] } else { data }));
        let report = load(&path, &NoProgress).unwrap();
        assert!(
            matches!(report.warnings.as_slice(), [LoadWarning::LayerPixelsLost { layer, reason, .. }]
                if *layer == top && reason.contains("size limit")),
            "{:?}",
            report.warnings
        );
        assert!(crate::testutil::layer_dense(&report.document, paint).data == crate::testutil::layer_dense(&doc, paint).data);

        // A missing layer entry only loses that layer.
        std::fs::write(&path, &original).unwrap();
        rewrite(&path, |name, data| (name != entry).then_some(data));
        let report = load(&path, &NoProgress).unwrap();
        assert!(matches!(report.warnings.as_slice(), [LoadWarning::LayerPixelsLost { layer, .. }] if *layer == top));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Sets the sizes the central directory declares for `entry` (what the reader checks against its limits).
    fn patch_declared_size(bytes: &mut [u8], entry: &str, size: u32) {
        let name = entry.as_bytes();
        let mut patched = 0;
        for at in 46..bytes.len().saturating_sub(name.len()) {
            if &bytes[at..at + name.len()] == name && bytes[at - 46..at - 42] == [0x50, 0x4b, 0x01, 0x02] {
                bytes[at - 46 + 20..at - 46 + 24].copy_from_slice(&size.to_le_bytes());
                bytes[at - 46 + 24..at - 46 + 28].copy_from_slice(&size.to_le_bytes());
                patched += 1;
            }
        }
        assert_eq!(patched, 1, "central directory header of {entry}");
    }

    #[test]
    fn one_oversized_optional_entry_does_not_make_the_project_unopenable() {
        let dir = temp_dir("load-optional-too-large");
        let (path, doc, paint, top) = painted_cube(&dir);
        let original = std::fs::read(&path).unwrap();
        let manifest = read_manifest(&path).unwrap();
        let texture = manifest.source.sidecars.iter().find(|f| f.key.ends_with(".png")).unwrap().file.entry.clone();

        // A texture sidecar (never decoded on load) declared beyond the 2 GiB limit: dropped with a log line.
        let mut bytes = original.clone();
        patch_declared_size(&mut bytes, &texture, 0xC000_0000);
        std::fs::write(&path, &bytes).unwrap();
        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_same_document(&doc, &report.document);
        // Extraction still writes everything else.
        let written = extract_sources(&path, &dir.join("out"), false).unwrap();
        assert_eq!(written.len(), manifest.source.sidecars.len());

        // A layer PNG declared beyond its limit: that layer's pixels are lost, everything else is there.
        let mut bytes = original.clone();
        patch_declared_size(&mut bytes, &format!("sets/s00/l{}.png", paint.0), 0xC000_0000);
        std::fs::write(&path, &bytes).unwrap();
        let report = load(&path, &NoProgress).unwrap();
        assert!(matches!(report.warnings.as_slice(), [LoadWarning::LayerPixelsLost { layer, .. }] if *layer == paint));
        assert!(crate::testutil::layer_dense(&report.document, top).data == crate::testutil::layer_dense(&doc, top).data);

        // The model file itself is still fatal.
        let mut bytes = original.clone();
        patch_declared_size(&mut bytes, &manifest.source.main.file.entry, 0xC000_0000);
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(load_err(&path), ProjectError::EntryTooLarge { entry } if entry == manifest.source.main.file.entry));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversized_blobs_degrade_like_missing_ones() {
        let dir = temp_dir("load-blob-too-large");
        let mut doc = cube_doc(&noise_texture(32, 32, 8));
        let png = encode_png(8, 8, &[200u8; 8 * 8 * 4], PngLevel::Fast).unwrap();
        let blob = doc.add_image_blob("logo.png", Arc::from(png)).unwrap();
        let placement = crate::stamp::Placement::Uv(crate::stamp::UvPlacement {
            set: crate::ids::TextureSetId(0),
            center_px: glam::Vec2::new(16.0, 16.0),
            size_px: glam::Vec2::new(8.0, 8.0),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        let stamp = doc.add_stamp("Logo", StampSource::Image { blob }, placement, crate::ids::TextureSetId(0)).unwrap();
        let path = dir.join("stamp.wtep");
        save_doc(&doc, &path);
        let entry = read_manifest(&path).unwrap().blobs[0].file.entry.clone();
        let mut bytes = std::fs::read(&path).unwrap();
        patch_declared_size(&mut bytes, &entry, 0xC000_0000);
        std::fs::write(&path, &bytes).unwrap();
        let report = load(&path, &NoProgress).unwrap();
        assert!(
            matches!(report.warnings.as_slice(), [LoadWarning::BlobMissing { used_by, .. }] if used_by == &[stamp]),
            "{:?}",
            report.warnings
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_or_changed_model_is_unrecoverable() {
        let dir = temp_dir("load-model");
        let (path, _, _, _) = painted_cube(&dir);
        let original = std::fs::read(&path).unwrap();
        let unrecoverable = |path: &Path| matches!(load_err(path), ProjectError::ModelUnrecoverable(_));

        // Readable entry, but no model in it.
        rewrite(&path, |name, data| Some(if name == "source/main.obj" { b"\x00\x01 this is no model".to_vec() } else { data }));
        assert!(unrecoverable(&path));

        // Damaged compressed data: the ZIP CRC (or the inflater) catches it.
        std::fs::write(&path, &original).unwrap();
        let mut bytes = original.clone();
        let mut zip = ZipArchive::new(Cursor::new(&original[..])).unwrap();
        let (start, len) = {
            let entry = zip.by_name("source/main.obj").unwrap();
            (entry.data_start().unwrap() as usize, entry.compressed_size() as usize)
        };
        for b in &mut bytes[start + len / 3..start + len / 3 + 8] {
            *b ^= 0xff;
        }
        std::fs::write(&path, &bytes).unwrap();
        assert!(unrecoverable(&path));

        // Missing model entry.
        std::fs::write(&path, &original).unwrap();
        rewrite(&path, |name, data| (name != "source/main.obj").then_some(data));
        assert!(unrecoverable(&path));
        std::fs::remove_dir_all(&dir).unwrap();

        // The model now yields another number of texture sets.
        let dir = temp_dir("load-setcount");
        let path = dir.join("two.wtep");
        save_doc(&two_set_doc(), &path);
        edit_manifest(&path, |m| {
            m["sets"].as_array_mut().unwrap().pop();
        });
        match load_err(&path) {
            ProjectError::ModelUnrecoverable(message) => assert_eq!(message, "texture set count changed"),
            other => panic!("expected ModelUnrecoverable, got {other}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn changed_uv_layout_is_a_warning() {
        let dir = temp_dir("load-uvhash");
        let (path, doc, _, _) = painted_cube(&dir);
        edit_manifest(&path, |m| m["import"]["uv_hash"] = "0123456789abcdef".into());
        let report = load(&path, &NoProgress).unwrap();
        assert_eq!(report.warnings, [LoadWarning::MeshUvChanged]);
        assert_same_document(&doc, &report.document);

        // An older importer version does not change that when the model has UVs of its own.
        edit_manifest(&path, |m| {
            m["import"]["importer_version"] = 1.into();
            m["import"]["generate_fallback_uvs"] = true.into();
        });
        assert_eq!(load(&path, &NoProgress).unwrap().warnings, [LoadWarning::MeshUvChanged]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A saved project of `no_uv_tetra` (no UVs, no normals) with the automatic UV layout and a painted block.
    fn painted_no_uv_tetra(dir: &Path) -> (PathBuf, Document) {
        let bundle = obj_bundle("rock", &samples::no_uv_tetra(), &[]);
        let options = ImportOptions { generate_fallback_uvs: true, ..ImportOptions::default() };
        let model = import_from_bundle(&bundle, &options, true, &NoProgress).unwrap();
        assert_eq!(model.warnings, [ImportWarning::NormalsGenerated, ImportWarning::FallbackUvsGenerated]);
        let mut doc = Document::from_import(model, vec![TextureChoice::Detected], &NoProgress).unwrap();
        let paint = doc.sets[0].active.unwrap();
        paint_block(&mut doc, paint, IRect::new(4, 4, 40, 30), [0, 200, 0, 255]);
        doc.recomposite_all();
        let path = dir.join("rock.wtep");
        save_doc(&doc, &path);
        (path, doc)
    }

    #[test]
    fn reopening_a_generated_layout_project_repeats_no_import_facts() {
        let dir = temp_dir("load-no-nag");
        let (path, doc) = painted_no_uv_tetra(&dir);
        // Generated normals and generated UVs were shown at import: a reopen reports nothing.
        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_same_document(&doc, &report.document);
        assert!(!report.document.is_modified());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_stored_material_library_is_still_reported_on_load() {
        let dir = temp_dir("load-missing-mtl");
        let (path, _, _, _) = painted_cube(&dir);
        let mtl = read_manifest(&path).unwrap().source.sidecars.iter().find(|f| f.key.ends_with(".mtl")).cloned();
        let mtl = mtl.expect("the cube project stores its material library");
        rewrite(&path, |name, data| (name != mtl.file.entry).then_some(data));
        let report = load(&path, &NoProgress).unwrap();
        assert!(
            report.warnings.iter().any(|w| matches!(w, LoadWarning::Import(ImportWarning::MissingMtl(_)))),
            "{:?}",
            report.warnings
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_material_library_missing_at_import_is_not_repeated_on_load() {
        let dir = temp_dir("load-mtl-gone-at-import");
        let mut bundle = obj_bundle("q", &samples::flat_quad(), &[]);
        let obj = format!("mtllib nowhere.mtl\n{}", String::from_utf8(bundle.main.bytes.to_vec()).unwrap());
        bundle.main.bytes = std::sync::Arc::from(obj.into_bytes());
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        assert!(model.warnings.contains(&ImportWarning::MissingMtl("nowhere.mtl".into())), "{:?}", model.warnings);
        let doc = Document::from_import(model, vec![TextureChoice::Detected], &NoProgress).unwrap();
        let path = dir.join("q.wtep");
        save_doc(&doc, &path);
        // The library was never stored, so its absence says nothing about the project: shown at import only.
        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn older_generated_layout_is_named_in_the_warning() {
        let dir = temp_dir("load-older-layout");
        let (path, doc) = painted_no_uv_tetra(&dir);
        let original = std::fs::read(&path).unwrap();

        // Saved by importer version 1 with a different hash: the plain-words warning, nothing else; the document
        // loads as stored (nothing is rewritten).
        edit_manifest(&path, |m| {
            m["import"]["importer_version"] = 1.into();
            m["import"]["uv_hash"] = "0123456789abcdef".into();
        });
        let report = load(&path, &NoProgress).unwrap();
        assert_eq!(report.warnings, [LoadWarning::OlderGeneratedUvLayout { saved: 1 }]);
        let text = report.warnings[0].to_string();
        assert!(text.contains("older automatic texture layout"), "{text}");
        assert!(text.contains("wrong place"), "{text}");
        assert!(text.contains("Nothing was overwritten"), "{text}");
        assert!(!text.contains("  ") && !text.contains('\n'), "one line, single spaces: {text:?}");
        assert_same_document(&doc, &report.document);
        assert!(!report.document.is_modified());

        // The same version-1 manifest with the right hash: no warning at all.
        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| m["import"]["importer_version"] = 1.into());
        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        // The current version with a different hash: the existing text.
        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| m["import"]["uv_hash"] = "0123456789abcdef".into());
        assert_eq!(load(&path, &NoProgress).unwrap().warnings, [LoadWarning::MeshUvChanged]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn model_facts_are_the_informational_import_warnings() {
        use ImportWarning as W;
        let s = || String::from("x");
        for w in [
            W::NoUvs { material: s() },
            W::DegenerateUvs { material: s(), percent: 3 },
            W::UvWrapped { triangles: 2 },
            W::NormalsGenerated,
            W::DroppedDegenerate { triangles: 1 },
            W::SkippedPrimitive { reason: s() },
            W::SkinIgnored,
            W::FallbackUvsGenerated,
            W::BadObjNumbers { first_line: 7, lines: 2, dropped_faces: 3, degraded_faces: 0 },
        ] {
            assert!(w.is_model_fact(), "{w:?}");
        }
        for w in [
            W::MissingMtl(s()),
            W::MissingTexture { material: s(), reference: s() },
            W::TextureDecodeFailed { reference: s(), reason: s() },
            W::TextureResized { reference: s(), from: [2, 2], to: [1, 1] },
        ] {
            assert!(!w.is_model_fact(), "{w:?}");
        }
    }

    #[test]
    fn unknown_layers_are_skipped() {
        let dir = temp_dir("load-unknown");
        let (path, doc, paint, top) = painted_cube(&dir);
        let original = std::fs::read(&path).unwrap();
        let i = doc.sets[0].layer_index(top).unwrap();
        let unknown = LoadWarning::UnknownLayer { set: crate::ids::TextureSetId(0), index: i };

        // A kind this version does not know (and the set's active layer pointed at it).
        edit_manifest(&path, |m| {
            m["sets"][0]["layers"][i]["kind"] = "adjustment".into();
            m["sets"][0]["active_layer"] = top.0.into();
        });
        let report = load(&path, &NoProgress).unwrap();
        assert_eq!(report.warnings, std::slice::from_ref(&unknown));
        let loaded = report.document;
        assert!(loaded.layer(top).is_none());
        assert_eq!(loaded.sets[0].layers.len(), doc.sets[0].layers.len() - 1);
        assert_eq!(loaded.sets[0].active, Some(paint), "the topmost remaining layer becomes active");
        assert!(crate::testutil::layer_dense(&loaded, paint).data == crate::testutil::layer_dense(&doc, paint).data);

        // A stamp layer naming a stamp the file does not contain.
        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| {
            m["sets"][0]["layers"][i]["kind"] = "stamp".into();
            m["sets"][0]["layers"][i]["stamp"] = 999.into();
        });
        assert_eq!(load(&path, &NoProgress).unwrap().warnings, std::slice::from_ref(&unknown));

        // Not a layer object at all, and a duplicated layer id.
        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| m["sets"][0]["layers"][i] = serde_json::json!("layer"));
        assert_eq!(load(&path, &NoProgress).unwrap().warnings, std::slice::from_ref(&unknown));
        std::fs::write(&path, &original).unwrap();
        edit_manifest(&path, |m| m["sets"][0]["layers"][i]["id"] = paint.0.into());
        let report = load(&path, &NoProgress).unwrap();
        assert_eq!(report.warnings, [unknown]);
        assert_eq!(report.document.sets[0].layers.len(), doc.sets[0].layers.len() - 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancellation() {
        struct Cancel;
        impl Progress for Cancel {
            fn report(&self, _: f32, _: &str) {}
            fn cancelled(&self) -> bool {
                true
            }
        }
        let dir = temp_dir("load-cancel");
        let (path, _, _, _) = painted_cube(&dir);
        assert!(matches!(load(&path, &Cancel), Err(ProjectError::Cancelled)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The original model file of the frozen version-1 fixture: a unit quad without materials.
    const FROZEN_V1_OBJ: &str = "\
# frozen version-1 fixture
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vt 0 1
vt 1 1
vt 1 0
vt 0 0
vn 0 0 1
f 1/1/1 2/2/1 3/3/1
f 1/1/1 3/3/1 4/4/1
";

    /// A version-1 manifest exactly as version 1 writes it. Never edit this string: files of this shape must load
    /// forever (section 7.3). The loader takes entry sizes from the archive, so the recorded sizes are not checked.
    /// The `uv_hash` is that of `FROZEN_V1_OBJ`; a load without `MeshUvChanged` proves the UV fingerprint is stable.
    const FROZEN_V1_MANIFEST: &str = r#"{
  "format": "wtep",
  "format_version": 1,
  "min_reader_version": 1,
  "app_version": "0.1.0",
  "name": "frozen quad",
  "saved_unix": 1700000000,
  "next_id": 7,
  "source": {
    "format": "obj",
    "main": {
      "key": "quad.obj",
      "original_name": "quad.obj",
      "file": { "entry": "source/main.obj", "size": 141, "crc32": 0 }
    },
    "sidecars": [],
    "extras": [],
    "original_path": "C:\\models\\quad.obj"
  },
  "import": {
    "generate_fallback_uvs": false,
    "max_texture_dim": 8192,
    "importer_version": 1,
    "uv_hash": "01d49df0c0cbd647",
    "vertex_count": 4,
    "triangle_count": 2
  },
  "sets": [
    {
      "id": 0,
      "name": "quad",
      "width": 4,
      "height": 4,
      "active_layer": 6,
      "flattened": null,
      "layers": [
        {
          "id": 1,
          "name": "Base",
          "visible": true,
          "locked": true,
          "alpha_lock": false,
          "opacity": 1.0,
          "blend": "normal",
          "kind": "raster",
          "pixels": { "file": { "entry": "sets/s00/l1.png", "size": 0, "crc32": 0 }, "rect": [0, 0, 4, 4] }
        },
        {
          "id": 6,
          "name": "Paint 1",
          "visible": false,
          "locked": false,
          "alpha_lock": true,
          "opacity": 0.5,
          "blend": "multiply",
          "kind": "raster",
          "pixels": { "file": { "entry": "sets/s00/l6.png", "size": 0, "crc32": 0 }, "rect": [1, 2, 2, 1] }
        },
        {
          "id": 3,
          "name": "Empty",
          "visible": true,
          "locked": false,
          "alpha_lock": false,
          "opacity": 0.25,
          "blend": "screen",
          "kind": "raster",
          "pixels": null
        }
      ]
    }
  ],
  "stamps": [],
  "blobs": [],
  "ui_state": { "view_mode": "view3d", "active_set": 0 }
}"#;

    #[test]
    fn frozen_v1_manifest_fixture_loads() {
        let manifest = FROZEN_V1_MANIFEST;

        let base: Vec<u8> = (0..16u8).flat_map(|i| [i * 16, 255 - i, i, if i % 3 == 0 { 0 } else { 255 }]).collect();
        let paint = [1u8, 2, 3, 4, 5, 6, 7, 0];
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let mut add = |name: &str, options: zip::write::SimpleFileOptions, data: &[u8]| {
            zip.start_file(name, options).unwrap();
            zip.write_all(data).unwrap();
        };
        add("mimetype", zip_stored(), b"application/x-wtep");
        add("source/main.obj", zip_deflated(), FROZEN_V1_OBJ.as_bytes());
        add("sets/s00/l1.png", zip_stored(), &encode_png(4, 4, &base, PngLevel::Fast).unwrap());
        add("sets/s00/l6.png", zip_stored(), &encode_png(2, 1, &paint, PngLevel::Fast).unwrap());
        add("manifest.json", zip_deflated(), manifest.as_bytes());
        let dir = temp_dir("load-frozen");
        let path = dir.join("frozen.wtep");
        std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();

        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(!report.force_save_as);
        let doc = report.document;
        assert_eq!(doc.name, "frozen quad");
        assert_eq!(doc.next_id, 7);
        assert_eq!(doc.ui_state, serde_json::json!({ "view_mode": "view3d", "active_set": 0 }));
        assert_eq!(doc.sets.len(), 1);
        let set = &doc.sets[0];
        assert_eq!((set.name.as_str(), set.width, set.height, set.active), ("quad", 4, 4, Some(LayerId(6))));
        let ids: Vec<u64> = set.layers.iter().map(|l| l.id.0).collect();
        assert_eq!(ids, [1, 6, 3]);
        let [b, p, e] = [&set.layers[0], &set.layers[1], &set.layers[2]];
        assert_eq!((b.name.as_str(), b.locked, b.visible, b.opacity, b.blend), ("Base", true, true, 1.0, crate::layer::BlendMode::Normal));
        assert_eq!(
            (p.name.as_str(), p.visible, p.alpha_lock, p.opacity, p.blend),
            ("Paint 1", false, true, 0.5, crate::layer::BlendMode::Multiply)
        );
        assert_eq!((e.name.as_str(), e.opacity, e.blend), ("Empty", 0.25, crate::layer::BlendMode::Screen));
        assert_eq!(b.raster().unwrap().data, &base[..]);
        let mut expected_paint = PixelBuf::new(4, 4);
        expected_paint.set(1, 2, [1, 2, 3, 4]);
        expected_paint.set(2, 2, [5, 6, 7, 0]);
        assert_eq!(p.to_dense(4, 4).data, &expected_paint.data[..]);
        assert!(e.raster().is_some_and(|b| b.data.is_empty()), "an empty paint layer loads unallocated");
        assert_eq!(doc.source.original_path.as_deref(), Some("C:\\models\\quad.obj"));
        assert!(!doc.is_modified());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn manually_chosen_texture_comes_back_on_extract() {
        let dir = temp_dir("load-extras");
        // The user picks a texture file from disk; it is deleted after the project is saved.
        let chosen_path = dir.join("My Paint.png");
        let chosen = noise_texture(24, 24, 31);
        let chosen_bytes = encode_png(24, 24, &chosen.data, PngLevel::High).unwrap();
        std::fs::write(&chosen_path, &chosen_bytes).unwrap();

        let bundle = obj_bundle("cube", &samples::seam_cube(1.0), &[("cube_tex.png", &noise_texture(16, 16, 2))]);
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        let original = source_file("My Paint.png", std::fs::read(&chosen_path).unwrap());
        let choice = TextureChoice::Image { pixels: chosen.clone(), original: Some(original) };
        let doc = Document::from_import(model, vec![choice], &NoProgress).unwrap();
        assert_eq!(doc.source.extras[0].key, "textures/set00/my paint.png");
        let path = dir.join("chosen.wtep");
        save_doc(&doc, &path);
        std::fs::remove_file(&chosen_path).unwrap();

        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let loaded = report.document;
        assert_same_document(&doc, &loaded);
        assert_eq!(loaded.sets[0].layers[0].raster().unwrap().data, &chosen.data[..]);

        let out = dir.join("extracted");
        let written = extract_bundle(&loaded.source, &loaded.blobs, &[], &out, false).unwrap();
        let relative: Vec<String> = written
            .iter()
            .map(|p| p.strip_prefix(&out).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(relative, ["cube.obj", "cube.mtl", "cube_tex.png", "textures/set00/My Paint.png"]);
        assert_eq!(std::fs::read(out.join("textures").join("set00").join("My Paint.png")).unwrap(), chosen_bytes);
        for (file, written) in [&bundle.main, &bundle.sidecars[0], &bundle.sidecars[1]].into_iter().zip(&written) {
            assert_eq!(std::fs::read(written).unwrap(), &file.bytes[..]);
        }

        // The same straight from the project file, without importing anything.
        let out2 = dir.join("from file");
        let from_file = extract_sources(&path, &out2, false).unwrap();
        assert_eq!(from_file.len(), 4);
        for (a, b) in written.iter().zip(&from_file) {
            assert_eq!(a.strip_prefix(&out).unwrap(), b.strip_prefix(&out2).unwrap());
            assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extract_never_replaces_without_overwrite() {
        let dir = temp_dir("load-extract");
        let bundle = obj_bundle("model", &samples::flat_quad(), &[]);
        let mut blobs = BlobStore::default();
        blobs.insert(BlobKind::Image, "Logo: final?.png", Arc::from(&b"png bytes"[..]));
        blobs.insert(BlobKind::Font, "fonts/Brand.ttf", Arc::from(&b"font bytes"[..]));
        blobs.insert(BlobKind::Image, "", Arc::from(&b"nameless"[..]));
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("model.obj"), b"user's own file").unwrap();

        let rel = |paths: &[PathBuf]| -> Vec<String> {
            paths.iter().map(|p| p.strip_prefix(&out).unwrap().to_string_lossy().replace('\\', "/")).collect()
        };
        // A file of the set exists: the whole set goes into a fresh folder, under its own names.
        let first = extract_bundle(&bundle, &blobs, &[], &out, false).unwrap();
        let expect = |folder: &str| -> Vec<String> {
            ["model.obj", "model.mtl", "stamps/Logo_ final_.png", "fonts/Brand.ttf", "stamps/blob2"]
                .iter()
                .map(|name| format!("{folder}/{name}"))
                .collect()
        };
        assert_eq!(rel(&first), expect("model_original"));
        assert_eq!(std::fs::read(out.join("model.obj")).unwrap(), b"user's own file");
        assert_eq!(std::fs::read(&first[0]).unwrap(), &bundle.main.bytes[..]);
        assert_eq!(std::fs::read(out.join("model_original").join("fonts").join("Brand.ttf")).unwrap(), b"font bytes");

        let second = extract_bundle(&bundle, &blobs, &[], &out, false).unwrap();
        assert_eq!(rel(&second), expect("model_original_2"));

        // With overwrite the plain names are used and replaced.
        let third = extract_bundle(&bundle, &blobs, &[], &out, true).unwrap();
        assert_eq!(rel(&third), ["model.obj", "model.mtl", "stamps/Logo_ final_.png", "fonts/Brand.ttf", "stamps/blob2"]);
        assert_eq!(std::fs::read(out.join("model.obj")).unwrap(), &bundle.main.bytes[..]);

        // Into an empty folder: the plain names.
        let empty = dir.join("empty");
        let fourth = extract_bundle(&bundle, &blobs, &[], &empty, false).unwrap();
        assert_eq!(fourth[0], empty.join("model.obj"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracted_files_keep_their_cross_references() {
        let dir = temp_dir("load-extract-links");
        // A texture name longer than 64 characters, as Substance and Unreal pipelines write them.
        let texture = "Character_Body_LOD0_Export_From_Substance_Painter_v2_Material_BaseColor.png";
        let mut raw = samples::flat_quad();
        raw.materials[0].texture = Some(0);
        let bundle = obj_bundle("crate", &raw, &[(texture, &noise_texture(8, 8, 4))]);
        assert!(bundle.sidecars[0].bytes.windows(texture.len()).any(|w| w == texture.as_bytes()), "map_Kd names it");
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        assert_eq!(model.warnings, [crate::error::ImportWarning::NormalsGenerated]);

        // The destination already holds a different texture of the same name.
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(texture), b"someone else's texture").unwrap();
        let written = extract_bundle(&bundle, &BlobStore::default(), &[], &out, false).unwrap();
        let folder = out.join("crate_original");
        assert_eq!(written, [folder.join("crate.obj"), folder.join("crate.mtl"), folder.join(texture)]);
        assert_eq!(std::fs::read(out.join(texture)).unwrap(), b"someone else's texture");

        // Re-importing the extracted model finds exactly the recovered files.
        let again = crate::import::import_from_path(&written[0], &ImportOptions::default(), &NoProgress).unwrap();
        assert_eq!(again.warnings, [crate::error::ImportWarning::NormalsGenerated]);
        assert_eq!(again.textures[0].image, model.textures[0].image);
        assert_eq!(again.mesh.uv_hash, model.mesh.uv_hash);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracted_fonts_get_a_font_extension() {
        let dir = temp_dir("load-extract-fonts");
        let bundle = obj_bundle("model", &samples::flat_quad(), &[]);
        let mut blobs = BlobStore::default();
        // Named like font blobs of earlier versions: "Family-Style", no extension.
        blobs.insert(BlobKind::Font, "Ubuntu-Light", Arc::from(&b"\0\x01\0\0 truetype"[..]));
        blobs.insert(BlobKind::Font, "Source Sans 3-Bold", Arc::from(&b"OTTO cff"[..]));
        blobs.insert(BlobKind::Font, "Noto.CJK-Regular", Arc::from(&b"ttcf collection"[..]));
        blobs.insert(BlobKind::Font, "Brand.woff2", Arc::from(&b"wOF2 web"[..]));
        let written = extract_bundle(&bundle, &blobs, &[], &dir, false).unwrap();
        let names: Vec<String> = written[2..].iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Ubuntu-Light.ttf", "Source Sans 3-Bold.otf", "Noto.CJK-Regular.ttc", "Brand.woff2"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extract_keeps_plain_relative_folders_only() {
        assert_eq!(key_dir("textures/set00/my tex.png"), "textures/set00");
        assert_eq!(key_dir("maps/wood.png"), "maps");
        assert_eq!(key_dir("wood.png"), "");
        assert_eq!(key_dir("../outside/wood.png"), "");
        assert_eq!(key_dir("c:/abs/wood.png"), "");
        assert_eq!(key_dir("/abs/wood.png"), "");
        assert_eq!(key_dir("a/con/wood.png"), "");
        assert_eq!(extracted_name("Brick Wall.PNG", "x"), "Brick Wall.PNG");
        assert_eq!(extracted_name("dir\\sub/na:me.tga", "x"), "na_me.tga");
        assert_eq!(extracted_name("", "textures/set00/fallback.png"), "fallback.png");
        assert_eq!(extracted_name("aux.png", "x"), "_aux.png");
        assert_eq!(extracted_name("noext", "x"), "noext");
        // Names Windows accepts are kept as they are, however long (the model's files refer to them).
        let long = format!("{}.png", "Long_Texture_Name_".repeat(10));
        assert_eq!(extracted_name(&long, "x"), long);
        let too_long = format!("{}.png", "x".repeat(300));
        assert_eq!(extracted_name(&too_long, "x"), format!("{}.png", "x".repeat(64)));
        assert_eq!(extracted_name("trailing dot.", "x"), "trailing dot");
    }
}
