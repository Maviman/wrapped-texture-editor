//! Sticker files `.wtes` (STICKER_PLAN 2.12, E7; ARCHITECTURE 7.8): one sticker for reuse in another project.
//!
//! A ZIP with, in this order: `mimetype` (`application/x-wtes`, stored, first), `blobs/b<id:05>.<ext>` (the images of
//! unedited picture layers and the fonts of text layers), `p<layer:03>.png` / `m<layer:03>.png` (owned pictures,
//! straight RGBA; masks, 8-bit grey), `preview.png` (the finished picture, at most 512 px a side) and `sticker.json`
//! (`WtesManifest`: `{ format: 1, sticker: StickerEntry, styles, blobs: [BlobEntry] }`). Entry time stamps are fixed,
//! so the same sticker always gives the same bytes. Reading assigns new ids: the blobs are added to the target
//! document's blob store (de-duplicated) and the sticker gets a fresh id from `add_sticker`.

use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Read, Seek, Write};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::blob::{BlobKind, BlobStore};
use crate::document::Document;
use crate::error::{EditError, LoadWarning, ProjectError};
use crate::ids::{BlobId, StickerId, StickerLayerId};
use crate::image_io::{PngLevel, decode_image, encode_png, encode_png_gray};
use crate::pixel::PixelBuf;
use crate::pixel_ops::thumbnail;
use crate::project::manifest::{BlobEntry, FileRef};
use crate::project::sticker_entries::{StickerContentEntry, StickerEntry, StyleStackEntry, style_floats};
use crate::sticker::model::{PicturePixels, StickerContent, StickerDoc};
use crate::sticker::style::StyleStack;

/// Content of the `mimetype` entry.
pub const WTES_MIMETYPE: &str = "application/x-wtes";
/// File extension of a sticker file (without the dot).
pub const WTES_EXTENSION: &str = "wtes";
/// The `format` this build writes and the newest it reads.
pub const WTES_FORMAT: u32 = 1;
/// Longest side of `preview.png`.
pub const WTES_PREVIEW_SIDE: u32 = 512;
/// Name of the manifest entry, the last entry of the archive.
const MANIFEST: &str = "sticker.json";
/// Size limit of `sticker.json`.
const MANIFEST_LIMIT: u64 = 16 << 20;

/// `sticker.json`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WtesManifest {
    pub format: u32,
    /// The sticker (its `id` is 0 in a written file and has no meaning).
    pub sticker: StickerEntry,
    /// Stamp styles saved with the sticker (the style of the stamp it was exported from), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub styles: Option<StyleStackEntry>,
    #[serde(default)]
    pub blobs: Vec<BlobEntry>,
}

/// A read sticker file. Blob ids inside `sticker` refer to `blobs` (the ids of the file) until
/// `Document::import_wtes` maps them into a document.
pub struct WtesFile {
    pub sticker: StickerDoc,
    pub styles: Option<StyleStack>,
    pub blobs: BlobStore,
    /// `preview.png`, when present and readable.
    pub preview: Option<PixelBuf>,
    /// What could not be read (a lost picture or mask, a missing font), as for a project.
    pub warnings: Vec<LoadWarning>,
}

/// Options of every entry: the given method, a fixed time stamp (1980-01-01).
fn options(stored: bool) -> SimpleFileOptions {
    let base = if stored { crate::project::save::zip_stored() } else { crate::project::save::zip_deflated() };
    base.last_modified_time(zip::DateTime::default())
}

fn zip_error(e: zip::result::ZipError) -> ProjectError {
    match e {
        zip::result::ZipError::Io(io) => ProjectError::Io(io),
        other => ProjectError::Zip(other.to_string()),
    }
}

/// Writes `sticker` (with `styles`, the blobs it needs from `blobs`, and `preview` scaled to at most 512 px a side) as
/// the bytes of a `.wtes` file. Deterministic: the same input gives the same bytes. Errors: `ManifestInvalid` for a
/// non-finite number (named) or a blob the sticker needs that `blobs` does not hold, `Image` for an encoder failure.
pub fn write_wtes(
    sticker: &StickerDoc,
    styles: Option<&StyleStack>,
    blobs: &BlobStore,
    preview: Option<&PixelBuf>,
) -> Result<Vec<u8>, ProjectError> {
    use crate::project::save::{check_finite, entry_extension, sticker_entry, sticker_floats};
    for (layer, field, value) in sticker_floats(sticker) {
        check_finite(value, || match layer {
            Some(layer) => format!("{field} of layer \"{layer}\" of sticker \"{}\"", sticker.name),
            None => format!("{field} of sticker \"{}\"", sticker.name),
        })?;
    }
    if let Some(styles) = styles {
        let mut floats = Vec::new();
        style_floats(styles, &mut floats);
        for (field, value) in floats {
            check_finite(value, || format!("{field} of the styles of sticker \"{}\"", sticker.name))?;
        }
    }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let add = |zip: &mut ZipWriter<Cursor<Vec<u8>>>, name: String, stored: bool, bytes: &[u8]| -> Result<FileRef, ProjectError> {
        zip.start_file(name.as_str(), options(stored)).map_err(zip_error)?;
        zip.write_all(bytes)?;
        Ok(FileRef { entry: name, size: bytes.len() as u64, crc32: crc32fast::hash(bytes) })
    };
    add(&mut zip, "mimetype".to_owned(), true, WTES_MIMETYPE.as_bytes())?;

    let mut blob_entries = Vec::new();
    for id in sticker.blobs() {
        let blob = blobs.get(id).ok_or_else(|| ProjectError::ManifestInvalid(format!("stored file {} of the sticker is missing", id.0)))?;
        let name = format!("blobs/b{:05}.{}", id.0, entry_extension(&blob.name));
        let file = add(&mut zip, name, blob.kind == BlobKind::Image, &blob.bytes)?;
        blob_entries.push(BlobEntry { id, kind: blob.kind, original_name: blob.name.clone(), file });
    }

    let id = StickerId(0);
    let mut files: HashMap<(StickerId, StickerLayerId, bool), FileRef> = HashMap::new();
    for layer in &sticker.layers {
        if let StickerContent::Picture(PicturePixels::Owned { pixels, .. }) = &layer.content {
            let png = encode_png(pixels.width, pixels.height, &pixels.data, PngLevel::Fast)?;
            files.insert((id, layer.id, false), add(&mut zip, format!("p{:03}.png", layer.id.0), true, &png)?);
        }
        if let Some(mask) = &layer.mask {
            let png = encode_png_gray(mask.pixels.width, mask.pixels.height, &mask.pixels.data, PngLevel::Fast)?;
            files.insert((id, layer.id, true), add(&mut zip, format!("m{:03}.png", layer.id.0), true, &png)?);
        }
    }
    if let Some(picture) = preview.filter(|p| p.width > 0 && p.height > 0) {
        let side = picture.width.max(picture.height).min(WTES_PREVIEW_SIDE);
        let small = thumbnail(picture.view(), picture.width, picture.height, side);
        let png = encode_png(small.width, small.height, &small.data, PngLevel::Fast)?;
        add(&mut zip, "preview.png".to_owned(), true, &png)?;
    }
    let manifest = WtesManifest {
        format: WTES_FORMAT,
        sticker: sticker_entry(id, sticker, &files),
        styles: styles.and_then(StyleStackEntry::of),
        blobs: blob_entries,
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| ProjectError::ManifestInvalid(e.to_string()))?;
    add(&mut zip, MANIFEST.to_owned(), false, &json)?;
    Ok(zip.finish().map_err(zip_error)?.into_inner())
}

/// Reads one entry completely (at most `limit` bytes).
fn read_small<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str, limit: u64) -> Option<Vec<u8>> {
    let mut entry = zip.by_name(name).ok()?;
    if entry.size() > limit {
        return None;
    }
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry.by_ref().take(limit).read_to_end(&mut out).ok()?;
    Some(out)
}

/// Stickers P8 (3.8 "Library"): the preview picture of a `.wtes` file (its `preview.png`, at most 512 px) and the
/// sticker's name, without reading the layers: what the library list shows. `None` when the bytes are not a sticker
/// file; the picture is `None` when the file has no readable preview.
pub fn read_wtes_preview(bytes: &[u8]) -> Option<(String, Option<PixelBuf>)> {
    read_wtes_preview_from(Cursor::new(bytes))
}

/// `read_wtes_preview` from a reader (check P8: the library opens each file and reads only the zip directory, the
/// manifest and `preview.png`, never the whole file with its layers).
pub fn read_wtes_preview_from<R: Read + Seek>(reader: R) -> Option<(String, Option<PixelBuf>)> {
    let mut zip = ZipArchive::new(reader).ok()?;
    if read_small(&mut zip, "mimetype", 64).as_deref() != Some(WTES_MIMETYPE.as_bytes()) {
        return None;
    }
    let json = read_small(&mut zip, MANIFEST, MANIFEST_LIMIT)?;
    let value: serde_json::Value = serde_json::from_slice(&json).ok()?;
    let name = value.get("sticker").and_then(|s| s.get("name")).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
    let preview = read_small(&mut zip, "preview.png", 8 << 20)
        .and_then(|png| decode_image(&png, Some("preview.png"), WTES_PREVIEW_SIDE).ok())
        .map(|image| image.pixels);
    Some((name, preview))
}

/// Reads the bytes of a `.wtes` file. Errors: `NotAZip`; `ManifestInvalid` when it is not a sticker file (no
/// `mimetype` of `WTES_MIMETYPE`), when `sticker.json` is missing or does not parse, or when the sticker is invalid as a
/// whole (`StickerLost`'s reason); `TooNew` for a format above `WTES_FORMAT`. A picture, mask or font that cannot be read
/// degrades as in a project (`WtesFile::warnings`).
pub fn read_wtes(bytes: &[u8]) -> Result<WtesFile, ProjectError> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| ProjectError::NotAZip(e.to_string()))?;
    if read_small(&mut zip, "mimetype", 64).as_deref() != Some(WTES_MIMETYPE.as_bytes()) {
        return Err(ProjectError::ManifestInvalid("this is not a sticker file".to_owned()));
    }
    let json = read_small(&mut zip, MANIFEST, MANIFEST_LIMIT).ok_or(ProjectError::ManifestMissing)?;
    let value: serde_json::Value = serde_json::from_slice(&json).map_err(|e| ProjectError::ManifestInvalid(e.to_string()))?;
    let format = value.get("format").and_then(serde_json::Value::as_u64).unwrap_or(0);
    if format == 0 {
        return Err(ProjectError::ManifestInvalid("\"format\" is missing or not a version number".to_owned()));
    }
    if format > WTES_FORMAT as u64 {
        return Err(ProjectError::TooNew { found: u32::try_from(format).unwrap_or(u32::MAX), supported: WTES_FORMAT });
    }
    let manifest: WtesManifest = serde_json::from_value(value).map_err(|e| ProjectError::ManifestInvalid(e.to_string()))?;
    let mut blobs = crate::project::load::read_blobs(&mut zip, &manifest.blobs);
    let mentioned = manifest.blobs.iter().map(|b| b.id.0).chain(manifest.sticker.layers.iter().filter_map(|l| match &l.content {
        StickerContentEntry::Picture { blob, .. } => blob.map(|b| b.0),
        StickerContentEntry::Text(text) => Some(text.font.blob.0),
        StickerContentEntry::Shape(_) => None,
    }));
    let floor = mentioned.max().map_or(0, |m| m.saturating_add(1));
    let mut warnings = Vec::new();
    let mut loaded = crate::project::load::load_stickers(&mut zip, std::slice::from_ref(&manifest.sticker), floor, &mut blobs, &mut warnings);
    if let Some((_, reason)) = loaded.lost.pop_first() {
        return Err(ProjectError::ManifestInvalid(format!("the sticker cannot be read: {reason}")));
    }
    let sticker = loaded.stickers.pop_first().map(|(_, doc)| doc).ok_or_else(|| ProjectError::ManifestInvalid("the file holds no sticker".to_owned()))?;
    let preview = read_small(&mut zip, "preview.png", 8 << 20)
        .and_then(|png| decode_image(&png, Some("preview.png"), WTES_PREVIEW_SIDE).ok())
        .map(|image| image.pixels);
    Ok(WtesFile { sticker, styles: manifest.styles.as_ref().map(StyleStackEntry::to_styles), blobs, preview, warnings })
}

impl Document {
    /// The stickers a project save writes (STICKER_PLAN 2.11): those a stamp shows and the kept tray stickers,
    /// ascending id.
    pub fn saved_sticker_ids(&self) -> Vec<StickerId> {
        let shown: BTreeSet<StickerId> = self
            .stamps
            .iter()
            .filter_map(|s| match s.source {
                crate::stamp::StampSource::Sticker { sticker } => Some(sticker),
                _ => None,
            })
            .collect();
        self.stickers().filter(|(id, doc)| doc.keep || shown.contains(id)).map(|(id, _)| id).collect()
    }

    /// The finished picture (scale 1, no stamp styles) of every saved sticker with its name, for "Extract Original
    /// Files" (`project::extract_bundle`, `stickers\<name>.png`). A sticker that cannot be rendered is left out (logged).
    pub fn sticker_extract_pictures(&mut self) -> Vec<(String, PixelBuf)> {
        let mut out = Vec::new();
        for id in self.saved_sticker_ids() {
            let name = self.sticker(id).map(|d| d.name.clone()).unwrap_or_default();
            let name = if name.trim().is_empty() { format!("Sticker {}", id.0) } else { name };
            match self.sticker_picture(id, None, 1.0) {
                Ok(picture) => out.push((name, picture)),
                Err(e) => log::warn!("sticker {} could not be rendered for extraction: {e}", id.0),
            }
        }
        out
    }

    /// The bytes of a `.wtes` file of sticker `id`, with `styles` (the stamp styles to keep with it, if any) and its
    /// finished picture (with those styles) as the preview. Never a history command.
    pub fn sticker_wtes(&mut self, id: StickerId, styles: Option<&StyleStack>) -> Result<Vec<u8>, ProjectError> {
        let sticker = Arc::clone(self.sticker(id).ok_or_else(|| ProjectError::ManifestInvalid(EditError::UnknownSticker.to_string()))?);
        let preview = self.sticker_picture(id, styles, 1.0).map_err(|e| ProjectError::ManifestInvalid(e.to_string()))?;
        write_wtes(&sticker, styles, &self.blobs, Some(&preview))
    }

    /// Adds the sticker of a read `.wtes` file to the document as a kept tray sticker (`keep = true`; E7): its blobs
    /// are added to the blob store (identical bytes are shared), the sticker's blob ids mapped to them, and the sticker
    /// put in with `add_sticker` (a new id, fresh picture versions, one history entry "Add Sticker"). Returns the new
    /// id; the file's styles stay with the caller (`WtesFile::styles`).
    pub fn import_wtes(&mut self, file: &WtesFile) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        let mut map: HashMap<BlobId, BlobId> = HashMap::new();
        for id in file.sticker.blobs() {
            let Some(blob) = file.blobs.get(id) else { return Err(EditError::UnknownBlob) };
            map.insert(id, self.blobs.insert(blob.kind, &blob.name, Arc::clone(&blob.bytes)));
        }
        let mut sticker = file.sticker.clone();
        for layer in &mut sticker.layers {
            match &mut layer.content {
                StickerContent::Picture(PicturePixels::Blob(b)) => *b = map[b],
                StickerContent::Text(spec) => spec.font.blob = map[&spec.font.blob],
                _ => {}
            }
        }
        sticker.keep = true;
        self.add_sticker(sticker)
    }
}
