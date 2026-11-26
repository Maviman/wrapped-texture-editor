// Owner: core-persist (C6). Writing a `.wtep` project archive (ARCHITECTURE.md 5.23, container and procedure: 7.1).

use std::collections::BTreeSet;
use std::io::{BufReader, BufWriter, Seek, Write};
use std::path::Path;

use rayon::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::composite::flatten_layers;
use crate::document::{DocumentSnapshot, SetSnapshot};
use crate::error::{ImageIoError, ProjectError};
use crate::ids::BlobId;
use crate::image_io::{PngLevel, encode_png};
use crate::import::{IMPORTER_VERSION, MAX_SOURCE_FILE, SourceFile};
use crate::layer::{Layer, LayerKind};
use crate::pixel_ops::{crop, nonzero_bounds, thumbnail};
use crate::progress::Progress;
use crate::project::atomic::write_atomic;
use crate::project::manifest::{
    BlobEntry, FileRef, ImportEntry, LayerEntry, LayerKindEntry, Manifest, PixelsEntry, SetEntry, SourceEntry,
    SourceFileEntry, StampEntry,
};
use crate::project::migrate::parse_manifest;
use crate::project::{FORMAT_VERSION, PROJECT_MIMETYPE, PngReuse, SaveOptions, SaveStats, composite_key};
use crate::stamp::{Placement, StampObject, StampSource, SurfaceMode, WrapOptions};

/// Name of the manifest entry, the last entry of the archive.
pub(crate) const MANIFEST_ENTRY: &str = "manifest.json";
/// Longest side of the project thumbnail and of the ORA thumbnail.
pub(crate) const THUMBNAIL_SIDE: u32 = 256;

/// Options of a STORED entry (section 7.1). The compression level must stay `None`.
pub(crate) fn zip_stored() -> SimpleFileOptions {
    SimpleFileOptions::DEFAULT.compression_method(CompressionMethod::Stored)
}

/// Options of a "deflate 6" entry (section 7.1).
pub(crate) fn zip_deflated() -> SimpleFileOptions {
    SimpleFileOptions::DEFAULT.compression_method(CompressionMethod::Deflated).compression_level(Some(6))
}

/// How many `width x height` RGBA images may be encoded or decoded at the same time: `max(1, 1 GiB / (w * h * 4))`.
pub(crate) fn parallel_budget(width: u32, height: u32) -> usize {
    let bytes = (width as u64 * height as u64 * 4).max(1);
    ((1u64 << 30) / bytes).clamp(1, 1024) as usize
}

/// Extension of an archive entry for a file with this original name: its lower-case ASCII-alphanumeric extension
/// (at most 8 characters), else `bin`. No other part of a user string reaches a ZIP path.
pub(crate) fn entry_extension(original_name: &str) -> String {
    let file = original_name.rsplit(['/', '\\']).next().unwrap_or(original_name);
    match file.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            ext.to_ascii_lowercase()
        }
        _ => "bin".to_owned(),
    }
}

/// Already compressed formats are stored instead of deflated.
fn source_options(ext: &str) -> SimpleFileOptions {
    if matches!(ext, "png" | "jpg" | "jpeg" | "webp") { zip_stored() } else { zip_deflated() }
}

/// The longest side of a thumbnail of a `width x height` image: at most `THUMBNAIL_SIDE`, never an enlargement.
pub(crate) fn thumbnail_side(width: u32, height: u32) -> u32 {
    width.max(height).clamp(1, THUMBNAIL_SIDE)
}

fn zip_error(e: zip::result::ZipError) -> ProjectError {
    match e {
        zip::result::ZipError::Io(io) => ProjectError::Io(io),
        other => ProjectError::Zip(other.to_string()),
    }
}

fn check_finite(value: f32, what: impl Fn() -> String) -> Result<(), ProjectError> {
    if value.is_finite() { Ok(()) } else { Err(ProjectError::ManifestInvalid(format!("{} is not a finite number", what()))) }
}

/// Step 1 of the save procedure: every float that goes into the manifest is finite (serde_json would write `null`).
fn validate(snapshot: &DocumentSnapshot) -> Result<(), ProjectError> {
    // Stickers (STICKER_PLAN P3a): sticker stamps and stamp styles are stored by project format 4 (package P4). Until
    // then a project that uses them is refused rather than written without them.
    if let Some(stamp) = snapshot.stamps.iter().find(|s| matches!(s.source, StampSource::Sticker { .. }) || !s.styles.is_default()) {
        return Err(ProjectError::ManifestInvalid(format!(
            "stamp \"{}\" uses a sticker or stamp styles, which this build cannot save yet",
            stamp.name
        )));
    }
    for stamp in &snapshot.stamps {
        for (field, value) in stamp_floats(stamp) {
            check_finite(value, || format!("{field} of stamp \"{}\"", stamp.name))?;
        }
    }
    for layer in snapshot.sets.iter().flat_map(|s| &s.layers) {
        check_finite(layer.opacity, || format!("the opacity of layer \"{}\"", layer.name))?;
    }
    // `load` refuses larger source and blob entries: never write a project that cannot be opened again.
    check_entry_sizes(snapshot, MAX_SOURCE_FILE)
}

/// `EntryTooLarge` (named after the original file) when a source file or a stored blob is larger than `limit`.
fn check_entry_sizes(snapshot: &DocumentSnapshot, limit: u64) -> Result<(), ProjectError> {
    let source = &snapshot.source;
    let files = std::iter::once(&source.main).chain(&source.sidecars).chain(&source.extras);
    let named = files.map(|f| (f.original_name.as_str(), f.bytes.len()));
    let blobs = referenced_blobs(&snapshot.stamps).into_iter().filter_map(|id| snapshot.blobs.get(id));
    for (name, len) in named.chain(blobs.map(|b| (b.name.as_str(), b.bytes.len()))) {
        if len as u64 > limit {
            return Err(ProjectError::EntryTooLarge { entry: name.to_owned() });
        }
    }
    Ok(())
}

/// The lowest format version whose reader shows everything in this file correctly (FEATURE_TEXT 4.3 item 2): 3 when
/// any projected stamp is wrapped or has a Wrap option set (round 6; also a projected stamp that only keeps options for
/// switching back: an older build would drop them silently on its next save), else 2 when any text stamp uses a
/// round-4 feature (a gap value counts, also one on the last character of a line), else 1. `save` writes it as
/// `min_reader_version`, so a project without a newer feature stays readable by the older rounds.
pub fn required_reader(snapshot: &DocumentSnapshot) -> u32 {
    let uses_wrap = snapshot.stamps.iter().any(|s| {
        matches!(&s.placement, Placement::Projected(p) if p.mode != SurfaceMode::Project || p.wrap != WrapOptions::default())
    });
    if uses_wrap {
        return 3;
    }
    let uses_fx = snapshot.stamps.iter().any(|s| matches!(&s.source, StampSource::Text(spec) if !spec.fx.is_default()));
    if uses_fx { 2 } else { 1 }
}

/// Every float of a stamp that is written to the manifest, with a name for error messages.
fn stamp_floats(stamp: &StampObject) -> Vec<(&'static str, f32)> {
    let mut out = Vec::with_capacity(16);
    match &stamp.placement {
        Placement::Projected(p) => {
            out.extend(p.origin.to_array().map(|v| ("the origin", v)));
            out.extend(p.rotation.to_array().map(|v| ("the rotation", v)));
            out.extend(p.half_size.to_array().map(|v| ("the size", v)));
            out.push(("the front depth", p.depth_front));
            out.push(("the back depth", p.depth_back));
            out.push(("the start angle", p.angle_start_deg));
            out.push(("the end angle", p.angle_end_deg));
        }
        Placement::Uv(p) => {
            out.extend(p.center_px.to_array().map(|v| ("the centre", v)));
            out.extend(p.size_px.to_array().map(|v| ("the size", v)));
            out.push(("the rotation", p.rotation));
        }
    }
    if let StampSource::Text(spec) = &stamp.source {
        // The one float list of a text (FEATURE_TEXT 4.3 item 6): the round-3 fields and every round-4 effect value,
        // read without cloning the spec.
        spec.floats(&mut |name, value| out.push((name, value)));
    }
    out
}

/// The archive being written: adds entries, counts them and honours cancellation between entries.
struct Archive<'a, W: Write + Seek> {
    zip: ZipWriter<W>,
    entries: u32,
    progress: &'a dyn Progress,
}

impl<W: Write + Seek> Archive<'_, W> {
    fn add(&mut self, entry: String, options: SimpleFileOptions, bytes: &[u8]) -> Result<FileRef, ProjectError> {
        if self.progress.cancelled() {
            return Err(ProjectError::Cancelled);
        }
        self.zip.start_file(entry.as_str(), options).map_err(zip_error)?;
        self.zip.write_all(bytes)?;
        self.entries += 1;
        Ok(FileRef { entry, size: bytes.len() as u64, crc32: crc32fast::hash(bytes) })
    }

    fn add_source(&mut self, entry_stem: &str, ext: &str, file: &SourceFile) -> Result<SourceFileEntry, ProjectError> {
        let file_ref = self.add(format!("{entry_stem}.{ext}"), source_options(ext), &file.bytes)?;
        Ok(SourceFileEntry { key: file.key.clone(), original_name: file.original_name.clone(), file: file_ref })
    }
}

/// An encoded layer image: the manifest rectangle `[x, y, w, h]` and the PNG bytes.
type EncodedLayer = ([i32; 4], Vec<u8>);

/// The previous project file opened for copying unchanged images (`PngReuse`), while it is still the file the record
/// describes. Read failures (a damaged entry) only mean that the image is encoded again.
struct Reuse<'r> {
    record: &'r PngReuse,
    zip: ZipArchive<BufReader<std::fs::File>>,
}

impl<'r> Reuse<'r> {
    fn open(record: Option<&'r PngReuse>) -> Option<Reuse<'r>> {
        let record = record?;
        if !record.file_unchanged() {
            return None;
        }
        let file = std::fs::File::open(&record.path).ok()?;
        let zip = ZipArchive::new(BufReader::new(file)).ok()?;
        Some(Reuse { record, zip })
    }

    /// The bytes of an entry (CRC-checked by the reader).
    fn read(&mut self, entry: &str) -> Option<Vec<u8>> {
        let mut file = self.zip.by_name(entry).ok()?;
        let mut out = Vec::with_capacity(file.size().min(1 << 30) as usize);
        std::io::Read::read_to_end(&mut file, &mut out).ok()?;
        Some(out)
    }

    /// The stored image of a layer whose pixels did not change since it was written.
    fn layer(&mut self, layer: &Layer) -> Option<EncodedLayer> {
        let (version, entry, rect) = self.record.layers.get(&layer.id)?;
        if *version != layer.version() {
            return None;
        }
        let (entry, rect) = (entry.clone(), *rect);
        Some((rect, self.read(&entry)?))
    }

    /// The stored entry recorded as `(key, entry)` when its key is `key`.
    fn keyed(&mut self, stored: Option<&(u64, String)>, key: u64) -> Option<Vec<u8>> {
        let (k, entry) = stored?;
        if *k != key {
            return None;
        }
        let entry = entry.clone();
        self.read(&entry)
    }
}

/// The PNG of one layer and the canvas rectangle it covers; `None` when the layer stores no pixels.
///
/// Raster layers are cropped to `nonzero_bounds` (RGB under alpha 0 is content); stamp and text layers are written
/// with their cache rectangle.
fn encode_layer(layer: &Layer, level: PngLevel) -> Result<Option<EncodedLayer>, ImageIoError> {
    let (rect, png) = match &layer.kind {
        LayerKind::Raster(buf) => {
            let Some(rect) = nonzero_bounds(buf.view()) else {
                return Ok(None);
            };
            if rect == buf.rect() {
                (rect, encode_png(buf.width, buf.height, &buf.data, level)?)
            } else {
                let cropped = crop(buf, rect);
                (rect, encode_png(rect.width(), rect.height(), &cropped.data, level)?)
            }
        }
        LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => {
            if cache.is_empty() {
                return Ok(None);
            }
            // One rectangle per layer in the file: the parts are joined over their bounds (the loader splits them
            // again, `SparseBuf::from_cropped`).
            let joined = cache.to_cropped();
            (joined.rect, encode_png(joined.rect.width(), joined.rect.height(), &joined.data, level)?)
        }
    };
    Ok(Some(([rect.x0, rect.y0, rect.width() as i32, rect.height() as i32], png)))
}

fn layer_entry(layer: &Layer, pixels: Option<PixelsEntry>) -> LayerEntry {
    LayerEntry {
        id: layer.id,
        name: layer.name.clone(),
        visible: layer.visible,
        locked: layer.locked,
        alpha_lock: layer.alpha_lock,
        opacity: layer.opacity,
        blend: layer.blend,
        kind: match &layer.kind {
            LayerKind::Raster(_) => LayerKindEntry::Raster,
            LayerKind::Stamp { stamp, .. } => LayerKindEntry::Stamp { stamp: *stamp },
            LayerKind::Text { stamp, .. } => LayerKindEntry::Text { stamp: *stamp },
        },
        pixels,
    }
}

/// Ids of the blobs a stamp needs: the image of an image stamp, the font of a text stamp. (Sticker stamps: format 4,
/// package P4.)
fn referenced_blobs(stamps: &[StampObject]) -> BTreeSet<BlobId> {
    stamps
        .iter()
        .filter_map(|s| match &s.source {
            StampSource::Image { blob } => Some(*blob),
            StampSource::Text(spec) => Some(spec.font.blob),
            StampSource::Sticker { .. } => None,
        })
        .collect()
}

/// Writes every entry of the archive in the order of the table in section 7.1 and returns the entry count.
fn write_archive<W: Write + Seek>(
    out: W,
    snapshot: &DocumentSnapshot,
    opts: &SaveOptions,
    progress: &dyn Progress,
) -> Result<(W, u32, PngReuse), ProjectError> {
    let invalid = |e: serde_json::Error| ProjectError::ManifestInvalid(e.to_string());
    let mut archive = Archive { zip: ZipWriter::new(out), entries: 0, progress };
    // Unchanged images are copied from the file this document was loaded from or last saved to (finding SP-6).
    let mut reuse = Reuse::open(snapshot.png_reuse.as_deref());
    let mut written = PngReuse::default();
    progress.report(0.0, "Saving project");
    archive.add("mimetype".to_owned(), zip_stored(), PROJECT_MIMETYPE.as_bytes())?;

    // Original model files.
    let bundle = &snapshot.source;
    let main = archive.add_source("source/main", bundle.format.extension(), &bundle.main)?;
    let mut sidecars = Vec::with_capacity(bundle.sidecars.len());
    for (n, file) in bundle.sidecars.iter().enumerate() {
        sidecars.push(archive.add_source(&format!("source/f{n:03}"), &entry_extension(&file.original_name), file)?);
    }
    let mut extras = Vec::with_capacity(bundle.extras.len());
    for (n, file) in bundle.extras.iter().enumerate() {
        extras.push(archive.add_source(&format!("source/x{n:03}"), &entry_extension(&file.original_name), file)?);
    }

    // Stamp images and fonts that a stamp still uses.
    let mut blobs = Vec::new();
    for id in referenced_blobs(&snapshot.stamps) {
        let Some(blob) = snapshot.blobs.get(id) else {
            log::warn!("blob {} is referenced by a stamp but is not in the document; it is not saved", id.0);
            continue;
        };
        let options = match blob.kind {
            crate::blob::BlobKind::Image => zip_stored(),
            crate::blob::BlobKind::Font => zip_deflated(),
        };
        let entry = format!("blobs/b{:05}.{}", id.0, entry_extension(&blob.name));
        let file = archive.add(entry, options, &blob.bytes)?;
        blobs.push(BlobEntry { id, kind: blob.kind, original_name: blob.name.clone(), file });
    }

    // Layers and composites, set by set.
    let total_layers = snapshot.sets.iter().map(|s| s.layers.len() + 1).sum::<usize>().max(1) as f32;
    let mut done = 0usize;
    let mut sets = Vec::with_capacity(snapshot.sets.len());
    let mut thumb: Option<Vec<u8>> = None;
    for (index, set) in snapshot.sets.iter().enumerate() {
        let dir = format!("sets/s{:02}", set.id.0);
        let mut layers = Vec::with_capacity(set.layers.len());
        for chunk in set.layers.chunks(parallel_budget(set.width, set.height)) {
            if progress.cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let copied: Vec<Option<EncodedLayer>> = chunk.iter().map(|l| reuse.as_mut().and_then(|r| r.layer(l))).collect();
            let encoded: Vec<_> = chunk
                .par_iter()
                .zip(copied.into_par_iter())
                .map(|(layer, copied)| match copied {
                    Some(stored) => Ok(Some(stored)),
                    None => encode_layer(layer, opts.png),
                })
                .collect();
            for (layer, encoded) in chunk.iter().zip(encoded) {
                let pixels = match encoded? {
                    Some((rect, png)) => {
                        let entry = format!("{dir}/l{}.png", layer.id.0);
                        let file = archive.add(entry.clone(), zip_stored(), &png)?;
                        written.layers.insert(layer.id, (layer.version(), entry, rect));
                        Some(PixelsEntry { file, rect })
                    }
                    None => None,
                };
                layers.push(serde_json::to_value(layer_entry(layer, pixels)).map_err(invalid)?);
                done += 1;
                progress.report(0.05 + 0.9 * done as f32 / total_layers, "Saving layers");
            }
        }
        let flattened = write_composite(&mut archive, set, &dir, opts, index == 0, &mut thumb, &mut reuse, &mut written)?;
        done += 1;
        progress.report(0.05 + 0.9 * done as f32 / total_layers, "Saving layers");
        sets.push(SetEntry {
            id: set.id,
            name: set.name.clone(),
            width: set.width,
            height: set.height,
            active_layer: set.active,
            flattened,
            layers,
        });
    }
    // The old file is not needed any more: closed before the new one replaces it.
    drop(reuse);
    if let Some(png) = thumb {
        archive.add("thumbnail.png".to_owned(), zip_stored(), &png)?;
    }

    let mesh = &snapshot.mesh;
    let manifest = Manifest {
        format: "wtep".to_owned(),
        format_version: FORMAT_VERSION,
        min_reader_version: required_reader(snapshot),
        app_version: opts.app_version.clone(),
        name: snapshot.name.clone(),
        saved_unix: opts.now_unix,
        next_id: snapshot.next_id,
        source: SourceEntry {
            format: bundle.format,
            main,
            sidecars,
            extras,
            original_path: bundle.original_path.clone(),
        },
        import: ImportEntry {
            generate_fallback_uvs: snapshot.import_options.generate_fallback_uvs,
            max_texture_dim: snapshot.import_options.max_texture_dim,
            importer_version: IMPORTER_VERSION,
            uv_hash: format!("{:016x}", mesh.uv_hash),
            vertex_count: mesh.positions.len() as u32,
            triangle_count: mesh.tri_count(),
        },
        sets,
        stamps: snapshot.stamps.iter().map(StampEntry::from).collect(),
        blobs,
        ui_state: snapshot.ui_state.clone(),
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(invalid)?;
    archive.add(MANIFEST_ENTRY.to_owned(), zip_deflated(), &json)?;

    let entries = archive.entries;
    let out = archive.zip.finish().map_err(zip_error)?;
    progress.report(0.97, "Saving project");
    Ok((out, entries, written))
}

/// Flattens one set, writes `flattened.png` when requested and keeps the (encoded) thumbnail of the first set. When
/// the composite inputs did not change since the file `reuse` describes was written, both are copied from it.
#[expect(clippy::too_many_arguments, reason = "one step of write_archive with its shared state")]
fn write_composite<W: Write + Seek>(
    archive: &mut Archive<'_, W>,
    set: &SetSnapshot,
    dir: &str,
    opts: &SaveOptions,
    wants_thumbnail: bool,
    thumb: &mut Option<Vec<u8>>,
    reuse: &mut Option<Reuse<'_>>,
    written: &mut PngReuse,
) -> Result<Option<FileRef>, ProjectError> {
    if !opts.write_flattened && !wants_thumbnail {
        return Ok(None);
    }
    if archive.progress.cancelled() {
        return Err(ProjectError::Cancelled);
    }
    let key = composite_key(set);
    let entry = format!("{dir}/flattened.png");
    let thumb_entry = "thumbnail.png".to_owned();
    if let Some(r) = reuse.as_mut() {
        let flat = if opts.write_flattened { r.keyed(r.record.flattened.get(&set.id).cloned().as_ref(), key) } else { None };
        let small = if wants_thumbnail { r.keyed(r.record.thumbnail.clone().as_ref(), key) } else { None };
        if flat.is_some() == opts.write_flattened && small.is_some() == wants_thumbnail {
            if let Some(png) = small {
                *thumb = Some(png);
                written.thumbnail = Some((key, thumb_entry));
            }
            let Some(png) = flat else { return Ok(None) };
            let file = archive.add(entry.clone(), zip_stored(), &png)?;
            written.flattened.insert(set.id, (key, entry));
            return Ok(Some(file));
        }
    }
    let flat = flatten_layers(&set.layers, set.width, set.height, false);
    if wants_thumbnail && set.width > 0 && set.height > 0 {
        let small = thumbnail(flat.view(), set.width, set.height, thumbnail_side(set.width, set.height));
        *thumb = Some(encode_png(small.width, small.height, &small.data, PngLevel::Fast)?);
        written.thumbnail = Some((key, thumb_entry));
    }
    if !opts.write_flattened {
        return Ok(None);
    }
    let png = encode_png(flat.width, flat.height, &flat.data, opts.png)?;
    drop(flat);
    let file = archive.add(entry.clone(), zip_stored(), &png)?;
    written.flattened.insert(set.id, (key, entry));
    Ok(Some(file))
}

/// Every entry a manifest refers to. Layer entries that do not parse are skipped (the writer never produces them).
pub(crate) fn file_refs(manifest: &Manifest) -> Vec<FileRef> {
    let source = &manifest.source;
    let mut refs = vec![source.main.file.clone()];
    refs.extend(source.sidecars.iter().chain(&source.extras).map(|f| f.file.clone()));
    refs.extend(manifest.blobs.iter().map(|b| b.file.clone()));
    for set in &manifest.sets {
        for value in &set.layers {
            if let Ok(layer) = serde_json::from_value::<LayerEntry>(value.clone())
                && let Some(pixels) = layer.pixels
            {
                refs.push(pixels.file);
            }
        }
        refs.extend(set.flattened.clone());
    }
    refs
}

/// Step 4 of the save procedure: the finished temp file is a readable archive whose manifest parses and whose every
/// referenced entry exists with the recorded size.
fn verify(tmp: &Path) -> Result<(), ProjectError> {
    let failed = |message: String| ProjectError::VerifyFailed(message);
    let file = std::fs::File::open(tmp)?;
    let mut zip = ZipArchive::new(BufReader::new(file)).map_err(|e| failed(e.to_string()))?;
    let mut json = Vec::new();
    {
        use std::io::Read;
        let mut entry = zip.by_name(MANIFEST_ENTRY).map_err(|e| failed(format!("{MANIFEST_ENTRY}: {e}")))?;
        entry.read_to_end(&mut json).map_err(|e| failed(format!("{MANIFEST_ENTRY}: {e}")))?;
    }
    let manifest = parse_manifest(&json).map_err(|e| failed(e.to_string()))?.manifest;
    for file_ref in file_refs(&manifest) {
        let entry = zip.by_name(&file_ref.entry).map_err(|e| failed(format!("{}: {e}", file_ref.entry)))?;
        if entry.size() != file_ref.size {
            return Err(failed(format!(
                "{} has {} bytes, the manifest records {}",
                file_ref.entry,
                entry.size(),
                file_ref.size
            )));
        }
    }
    Ok(())
}

/// Writes a complete .wtep atomically (temp file in the same directory, fsync, verify, rename).
///
/// The snapshot must not contain Preview-quality stamp caches (the caller flushes a pending edit before taking it).
/// Errors: `ManifestInvalid` for a non-finite stamp parameter or layer opacity, `Cancelled` when `progress` asks for
/// it between two entries, `VerifyFailed` when the written archive cannot be read back; in every error case the file
/// at `path` is untouched and no temp file remains.
pub fn save(snapshot: &DocumentSnapshot, path: &Path, opts: &SaveOptions, progress: &dyn Progress) -> Result<SaveStats, ProjectError> {
    validate(snapshot)?;
    let mut stats = SaveStats::default();
    let mut written = None;
    write_atomic(
        path,
        opts.keep_backup,
        |file: &mut std::fs::File| -> Result<(), ProjectError> {
            let buffered = BufWriter::with_capacity(1 << 20, &mut *file);
            let (buffered, entries, reuse) = write_archive(buffered, snapshot, opts, progress)?;
            buffered.into_inner().map_err(|e| ProjectError::Io(e.into_error()))?;
            stats = SaveStats { bytes_written: file.metadata()?.len(), entries, reuse: None };
            written = Some(reuse);
            Ok(())
        },
        verify,
    )?;
    // What the new file holds, for the next save of this document.
    stats.reuse = written.zip(PngReuse::for_file(path)).map(|(w, file)| PngReuse { path: file.path, len: file.len, modified: file.modified, ..w });
    progress.report(1.0, "Project saved");
    Ok(stats)
}

/// Document builders shared by the persistence and export tests: real (re-importable) imports with small textures,
/// and direct pixel writes through the crate-internal accessors (no strokes, no stamps).
#[cfg(test)]
pub(crate) mod testkit {
    use std::sync::Arc;

    use crate::doc_build::TextureChoice;
    use crate::document::Document;
    use crate::ids::LayerId;
    use crate::image_io::{PngLevel, encode_png};
    use crate::import::{ImportOptions, ModelFormat, SourceBundle, SourceFile, import_from_bundle, normalize_ref};
    use crate::layer::LayerKind;
    use crate::math::IRect;
    use crate::mesh::RawMesh;
    use crate::pixel::PixelBuf;
    use crate::progress::NoProgress;
    use crate::samples;
    use crate::testutil::XorShift;

    /// Random RGBA: a quarter of the texels has alpha 0 (with random RGB), a quarter alpha 255, the rest random alpha.
    pub(crate) fn noise_texture(width: u32, height: u32, seed: u64) -> PixelBuf {
        let mut rng = XorShift::new(seed);
        let mut buf = PixelBuf::new(width, height);
        for px in buf.data.as_chunks_mut::<4>().0 {
            let r = rng.next_u32();
            let a = match r % 4 {
                0 => 0,
                1 => 255,
                _ => (r >> 8) as u8,
            };
            let rgb = rng.next_u32().to_le_bytes();
            *px = [rgb[0], rgb[1], rgb[2], a];
        }
        buf
    }

    /// A source file whose key is `normalize_ref(name)`.
    pub(crate) fn source_file(name: &str, bytes: Vec<u8>) -> SourceFile {
        SourceFile { key: normalize_ref(name), original_name: name.to_owned(), bytes: Arc::from(bytes) }
    }

    /// `<stem>.obj` + `<stem>.mtl` + one PNG per texture (`textures[i]` is raw texture index i).
    pub(crate) fn obj_files(stem: &str, raw: &RawMesh, textures: &[(&str, &PixelBuf)]) -> Vec<(String, Vec<u8>)> {
        let mtl_name = format!("{stem}.mtl");
        let names: Vec<String> = textures.iter().map(|(name, _)| (*name).to_owned()).collect();
        let (obj, mtl) = samples::to_obj(raw, &mtl_name, &names);
        let mut files = vec![(format!("{stem}.obj"), obj.into_bytes()), (mtl_name, mtl.into_bytes())];
        for (name, image) in textures {
            files.push(((*name).to_owned(), encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap()));
        }
        files
    }

    /// The files of `obj_files` as an in-memory bundle (the first file is the model).
    pub(crate) fn obj_bundle(stem: &str, raw: &RawMesh, textures: &[(&str, &PixelBuf)]) -> SourceBundle {
        let mut files = obj_files(stem, raw, textures).into_iter().map(|(name, bytes)| source_file(&name, bytes));
        let main = files.next().unwrap();
        SourceBundle { format: ModelFormat::Obj, main, sidecars: files.collect(), extras: Vec::new(), original_path: None }
    }

    /// Imports the bundle (textures decoded) and builds the document with every texture `Detected`.
    pub(crate) fn doc_from_bundle(bundle: &SourceBundle) -> Document {
        let model = import_from_bundle(bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
        let choices = vec![TextureChoice::Detected; model.mesh.sets.len()];
        Document::from_import(model, choices, &NoProgress).unwrap()
    }

    /// `seam_cube(1.0)` with `texture` as its only texture: one set, locked "Base" = `texture`, empty active "Paint 1".
    pub(crate) fn cube_doc(texture: &PixelBuf) -> Document {
        doc_from_bundle(&obj_bundle("cube", &samples::seam_cube(1.0), &[("cube_tex.png", texture)]))
    }

    /// `two_material_model()` with two small noise textures: sets "body" (48 x 32) and "head" (32 x 32).
    pub(crate) fn two_set_doc() -> Document {
        let body = noise_texture(48, 32, 21);
        let head = noise_texture(32, 32, 22);
        doc_from_bundle(&obj_bundle("robot", &samples::two_material_model(), &[("body.png", &body), ("head.png", &head)]))
    }

    /// Edits the pixels of a raster layer directly (ignoring locks) and reports the whole layer as changed.
    pub(crate) fn edit_pixels(doc: &mut Document, layer: LayerId, edit: impl FnOnce(&mut PixelBuf)) {
        let (set, index) = doc.find_layer(layer).expect("unknown layer");
        doc.sets[set.index()].allocate_raster(index);
        let rect = {
            let layer = doc.layer_mut(layer).expect("unknown layer");
            let LayerKind::Raster(buf) = &mut layer.kind else { panic!("not a raster layer") };
            let buf = Arc::make_mut(buf);
            edit(buf);
            IRect::from_size(buf.width, buf.height)
        };
        doc.mark_layer_dirty(layer, rect);
    }

    /// Fills `rect` of a raster layer with `color`.
    pub(crate) fn paint_block(doc: &mut Document, layer: LayerId, rect: IRect, color: [u8; 4]) {
        edit_pixels(doc, layer, |buf| {
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    buf.set(x as u32, y as u32, color);
                }
            }
        });
    }

    /// Structure, properties, pixels and composites of two documents are equal.
    pub(crate) fn assert_same_document(a: &Document, b: &Document) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.next_id, b.next_id);
        assert_eq!(a.ui_state, b.ui_state);
        assert_eq!(a.mesh.uv_hash, b.mesh.uv_hash);
        assert_eq!(a.sets.len(), b.sets.len());
        for (sa, sb) in a.sets.iter().zip(&b.sets) {
            assert_eq!((sa.id, &sa.name, sa.width, sa.height, sa.active), (sb.id, &sb.name, sb.width, sb.height, sb.active));
            assert_eq!(sa.layers.len(), sb.layers.len(), "set {}", sa.name);
            for (la, lb) in sa.layers.iter().zip(&sb.layers) {
                assert_eq!(
                    (la.id, &la.name, la.visible, la.locked, la.alpha_lock, la.opacity.to_bits(), la.blend, la.kind_name()),
                    (lb.id, &lb.name, lb.visible, lb.locked, lb.alpha_lock, lb.opacity.to_bits(), lb.blend, lb.kind_name())
                );
                let (va, vb) = (la.to_dense(sa.width, sa.height), lb.to_dense(sb.width, sb.height));
                assert_eq!(la.content_rects().is_empty(), lb.content_rects().is_empty(), "layer {}", la.name);
                assert!(va.data == vb.data, "pixels of layer {} differ", la.name);
            }
            assert!(sa.composite() == sb.composite(), "composite of set {} differs", sa.name);
        }
        assert_eq!(a.stamps, b.stamps);
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use zip::CompressionMethod;

    use super::testkit::{assert_same_document, cube_doc, edit_pixels, noise_texture, obj_files, paint_block, two_set_doc};
    use super::*;
    use crate::doc_build::{SampleKind, TextureChoice};
    use crate::document::{Commit, Document};
    use crate::ids::{LayerId, TextureSetId};
    use crate::image_io::decode_image;
    use crate::import::{ImportOptions, import_from_path};
    use crate::layer::BlendMode;
    use crate::math::IRect;
    use crate::pixel::PixelBuf;
    use crate::progress::NoProgress;
    use crate::project::load::load;
    use crate::testutil::temp_dir;

    fn options(now_unix: u64) -> SaveOptions {
        SaveOptions { png: PngLevel::Fast, write_flattened: true, keep_backup: false, now_unix, app_version: "0.1.0-test".to_owned() }
    }

    fn dir_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    fn archive(path: &Path) -> ZipArchive<Cursor<Vec<u8>>> {
        ZipArchive::new(Cursor::new(std::fs::read(path).unwrap())).unwrap()
    }

    fn entry_bytes(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap_or_else(|e| panic!("{name}: {e}")).read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn manifest_of(path: &Path) -> Manifest {
        serde_json::from_slice(&entry_bytes(&mut archive(path), MANIFEST_ENTRY)).unwrap()
    }

    fn layer_entries(manifest: &Manifest, set: usize) -> Vec<LayerEntry> {
        manifest.sets[set].layers.iter().map(|v| serde_json::from_value(v.clone()).unwrap()).collect()
    }

    /// Something in every kind of raster layer: painted blocks, RGB under alpha 0, an empty hidden layer, changed
    /// properties, a non-default active layer and UI state.
    fn decorated(mut doc: Document) -> Document {
        for set in doc.set_ids() {
            let paint = doc.sets[set.index()].active.unwrap();
            let (w, h) = (doc.sets[set.index()].width as i32, doc.sets[set.index()].height as i32);
            paint_block(&mut doc, paint, IRect::new(1, 2, w / 2, h / 2), [200, 10, 40, 255]);
            paint_block(&mut doc, paint, IRect::new(w / 2, h / 2, w - 1, h - 3), [10, 20, 30, 0]);
            let extra = doc.add_raster_layer(set, Some("Detail \u{e9}\u{6728}")).unwrap();
            paint_block(&mut doc, extra, IRect::new(3, 3, 9, 7), [1, 2, 3, 128]);
            doc.set_layer_opacity(extra, 0.3, Commit::Final).unwrap();
            doc.set_layer_blend(extra, BlendMode::Overlay).unwrap();
            doc.set_layer_alpha_lock(extra, true).unwrap();
            let hidden = doc.add_raster_layer(set, Some("Empty and hidden")).unwrap();
            doc.set_layer_visible(hidden, false).unwrap();
            doc.set_active_layer(set, Some(extra)).unwrap();
        }
        doc.ui_state = serde_json::json!({ "camera": { "yaw": 0.25 }, "view_mode": "split" });
        doc.recomposite_all();
        doc
    }

    /// Finding SP-6 (review round 2): a save copies the images of layers whose pixels did not change, and the
    /// composite of a set whose layers did not change, from the file of the last save (or of the load) instead of
    /// encoding them again. Saved with another PNG level, a copied entry keeps the old file's bytes.
    #[test]
    fn a_resave_copies_unchanged_images() {
        let dir = temp_dir("save_reuse");
        let mut doc = decorated(two_set_doc());
        let first = dir.join("first.wtep");
        let stats = save(&doc.snapshot(), &first, &options(1), &NoProgress).unwrap();
        let reuse = stats.reuse.clone().expect("the save records its images");
        assert_eq!(reuse.path, first);
        assert_eq!(reuse.flattened.len(), 2);
        assert!(reuse.thumbnail.is_some());
        let with_pixels = doc.sets.iter().flat_map(|s| &s.layers).filter(|l| !l.content_rects().is_empty()).count();
        assert_eq!(reuse.layers.len(), with_pixels);
        doc.set_png_reuse(stats.reuse);

        // Change one layer of the head set only, then save with another PNG level.
        let head_paint = doc.sets[1].layers[1].id;
        paint_block(&mut doc, head_paint, IRect::new(4, 4, 12, 12), [0, 255, 0, 255]);
        doc.recomposite_all();
        let second = dir.join("second.wtep");
        let high = SaveOptions { png: PngLevel::High, ..options(2) };
        // Precondition: the two levels encode differently, so equal bytes below mean "copied".
        let base = doc.sets[0].layers[0].raster().unwrap().clone();
        let fast = encode_png(base.width, base.height, &base.data, PngLevel::Fast).unwrap();
        assert_ne!(fast, encode_png(base.width, base.height, &base.data, PngLevel::High).unwrap());
        let stats2 = save(&doc.snapshot(), &second, &high, &NoProgress).unwrap();
        let (mut a, mut b) = (archive(&first), archive(&second));
        let manifest = manifest_of(&second);
        for set in 0..2 {
            for entry in layer_entries(&manifest, set) {
                let Some(pixels) = entry.pixels else { continue };
                let same = entry_bytes(&mut a, &pixels.file.entry) == entry_bytes(&mut b, &pixels.file.entry);
                assert_eq!(same, entry.id != head_paint, "layer {} copied: {same}", entry.name);
            }
        }
        assert_eq!(entry_bytes(&mut a, "sets/s00/flattened.png"), entry_bytes(&mut b, "sets/s00/flattened.png"), "unchanged set");
        assert_ne!(entry_bytes(&mut a, "sets/s01/flattened.png"), entry_bytes(&mut b, "sets/s01/flattened.png"), "changed set");
        assert_eq!(entry_bytes(&mut a, "thumbnail.png"), entry_bytes(&mut b, "thumbnail.png"), "the first set did not change");
        assert_same_document(&doc, &load(&second, &NoProgress).unwrap().document);

        // A loaded project knows its file too: re-saving it at once copies everything.
        let loaded = load(&second, &NoProgress).unwrap().document;
        let third = dir.join("third.wtep");
        save(&loaded.snapshot(), &third, &options(3), &NoProgress).unwrap();
        let mut c = archive(&third);
        for set in 0..2 {
            for entry in layer_entries(&manifest, set) {
                let Some(pixels) = entry.pixels else { continue };
                assert!(entry_bytes(&mut b, &pixels.file.entry) == entry_bytes(&mut c, &pixels.file.entry), "{}", entry.name);
            }
        }
        assert_eq!(entry_bytes(&mut b, "sets/s01/flattened.png"), entry_bytes(&mut c, "sets/s01/flattened.png"));

        // A file changed on disk since is never copied from: everything is encoded again (Fast differs from High).
        doc.set_png_reuse(stats2.reuse);
        std::fs::write(&second, b"replaced").unwrap();
        let fourth = dir.join("fourth.wtep");
        save(&doc.snapshot(), &fourth, &options(4), &NoProgress).unwrap();
        assert_same_document(&doc, &load(&fourth, &NoProgress).unwrap().document);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_material_sample_round_trips() {
        let doc = decorated(Document::sample(SampleKind::TwoMaterial));
        let dir = temp_dir("project-sample");
        let path = dir.join("two.wtep");
        let stats = save(&doc.snapshot(), &path, &options(1_700_000_000), &NoProgress).unwrap();
        assert_eq!(stats.bytes_written, std::fs::metadata(&path).unwrap().len());
        assert_eq!(stats.entries as usize, archive(&path).len());

        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(!report.force_save_as);
        let loaded = report.document;
        assert_same_document(&doc, &loaded);
        for set in &loaded.sets {
            assert!(set.layers[0].locked, "Base of {} is no longer locked", set.name);
            assert_eq!(set.layers[0].name, "Base");
        }
        assert!(!loaded.is_modified());
        assert!(!loaded.can_undo());

        // Thumbnail: from the composite of set 0, at most 256 px. Flattened: the full-size composite.
        let mut zip = archive(&path);
        let thumb = decode_image(&entry_bytes(&mut zip, "thumbnail.png"), None, 16384).unwrap().pixels;
        assert_eq!((thumb.width, thumb.height), (256, 256));
        let flat = decode_image(&entry_bytes(&mut zip, "sets/s01/flattened.png"), None, 16384).unwrap().pixels;
        assert!(&flat == doc.composite(TextureSetId(1)).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn layer_pixels_are_cropped_to_nonzero_bounds() {
        let mut doc = cube_doc(&noise_texture(32, 32, 3));
        let set = TextureSetId(0);
        let paint = doc.sets[0].active.unwrap();
        // RGB under alpha 0 everywhere: content that must round-trip byte for byte.
        edit_pixels(&mut doc, paint, |buf| {
            for px in buf.data.as_chunks_mut::<4>().0 {
                *px = [10, 20, 30, 0];
            }
        });
        let block = doc.add_raster_layer(set, Some("Block")).unwrap();
        paint_block(&mut doc, block, IRect::new(5, 6, 9, 8), [0, 0, 7, 0]);
        let empty = doc.add_raster_layer(set, Some("Empty")).unwrap();
        doc.recomposite_all();

        let dir = temp_dir("project-crop");
        let path = dir.join("crop.wtep");
        save(&doc.snapshot(), &path, &options(5), &NoProgress).unwrap();
        let manifest = manifest_of(&path);
        let entries = layer_entries(&manifest, 0);
        let find = |id: LayerId| entries.iter().find(|e| e.id == id).unwrap();
        assert_eq!(find(paint).pixels.as_ref().unwrap().rect, [0, 0, 32, 32]);
        let block_pixels = find(block).pixels.as_ref().unwrap();
        assert_eq!(block_pixels.rect, [5, 6, 4, 2]);
        assert_eq!(block_pixels.file.entry, format!("sets/s00/l{}.png", block.0));
        let png = entry_bytes(&mut archive(&path), &block_pixels.file.entry);
        assert_eq!((block_pixels.file.size, block_pixels.file.crc32), (png.len() as u64, crc32fast::hash(&png)));
        assert_eq!(decode_image(&png, None, 64).unwrap().pixels, PixelBuf::filled(4, 2, [0, 0, 7, 0]));
        assert!(find(empty).pixels.is_none(), "an all-zero layer is stored as pixels: null");
        assert!(!archive(&path).file_names().any(|n| n.unwrap() == format!("sets/s00/l{}.png", empty.0)));

        let loaded = load(&path, &NoProgress).unwrap().document;
        assert_same_document(&doc, &loaded);
        let LayerKind::Raster(buf) = &loaded.layer(paint).unwrap().kind else { panic!("not a raster layer") };
        assert!(buf.data.as_chunks::<4>().0.iter().all(|p| *p == [10, 20, 30, 0]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_is_deterministic_and_a_resave_keeps_the_manifest() {
        let doc = decorated(two_set_doc());
        let dir = temp_dir("project-determinism");
        let (a, b, c) = (dir.join("a.wtep"), dir.join("b.wtep"), dir.join("c.wtep"));
        save(&doc.snapshot(), &a, &options(42), &NoProgress).unwrap();
        save(&doc.snapshot(), &b, &options(42), &NoProgress).unwrap();
        assert!(std::fs::read(&a).unwrap() == std::fs::read(&b).unwrap(), "two saves differ");

        let loaded = load(&a, &NoProgress).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        save(&loaded.document.snapshot(), &c, &options(42), &NoProgress).unwrap();
        let (ma, mc) = (entry_bytes(&mut archive(&a), MANIFEST_ENTRY), entry_bytes(&mut archive(&c), MANIFEST_ENTRY));
        assert_eq!(String::from_utf8(ma).unwrap(), String::from_utf8(mc).unwrap());
        assert!(std::fs::read(&a).unwrap() == std::fs::read(&c).unwrap(), "the re-saved file differs");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn entries_follow_the_table_order_and_mimetype_is_stored_first() {
        let doc = cube_doc(&noise_texture(16, 16, 4));
        let dir = temp_dir("project-entries");
        let path = dir.join("cube.wtep");
        save(&doc.snapshot(), &path, &options(0), &NoProgress).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // Local file header of the first entry: method 0 (STORED), name length 8, no extra field, then the name and
        // directly the uncompressed content.
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(&bytes[8..10], [0, 0]);
        assert_eq!(&bytes[26..30], [8, 0, 0, 0]);
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + PROJECT_MIMETYPE.len()], PROJECT_MIMETYPE.as_bytes());

        let mut zip = archive(&path);
        let names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned()).collect();
        let base = doc.sets[0].layers[0].id.0;
        assert_eq!(
            names,
            [
                "mimetype".to_owned(),
                "source/main.obj".to_owned(),
                "source/f000.mtl".to_owned(),
                "source/f001.png".to_owned(),
                format!("sets/s00/l{base}.png"),
                "sets/s00/flattened.png".to_owned(),
                "thumbnail.png".to_owned(),
                "manifest.json".to_owned(),
            ]
        );
        let method = |zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str| zip.by_name(name).unwrap().compression();
        assert_eq!(method(&mut zip, "mimetype"), CompressionMethod::Stored);
        assert_eq!(method(&mut zip, "source/main.obj"), CompressionMethod::Deflated);
        assert_eq!(method(&mut zip, "source/f000.mtl"), CompressionMethod::Deflated);
        assert_eq!(method(&mut zip, "source/f001.png"), CompressionMethod::Stored);
        assert_eq!(method(&mut zip, &format!("sets/s00/l{base}.png")), CompressionMethod::Stored);
        assert_eq!(method(&mut zip, "sets/s00/flattened.png"), CompressionMethod::Stored);
        assert_eq!(method(&mut zip, "manifest.json"), CompressionMethod::Deflated);

        let manifest = manifest_of(&path);
        // Round 4: a file without new text features stays readable by a version-1 reader (round 6: the writer's own
        // version is 3).
        assert_eq!((manifest.format.as_str(), manifest.format_version, manifest.min_reader_version), ("wtep", FORMAT_VERSION, 1));
        assert_eq!((manifest.saved_unix, manifest.app_version.as_str()), (0, "0.1.0-test"));
        assert_eq!(manifest.import.uv_hash, format!("{:016x}", doc.mesh.uv_hash));
        assert_eq!(manifest.import.importer_version, IMPORTER_VERSION);
        assert_eq!(manifest.source.main.original_name, "cube.obj");
        assert_eq!(manifest.source.sidecars[1].key, "cube_tex.png");
        assert_eq!(manifest.sets[0].active_layer, doc.sets[0].active);
        assert!(manifest.blobs.is_empty() && manifest.stamps.is_empty());
        // The source entries hold the original bytes.
        assert_eq!(entry_bytes(&mut zip, "source/main.obj"), &doc.source.main.bytes[..]);
        assert_eq!(entry_bytes(&mut zip, "source/f001.png"), &doc.source.sidecars[1].bytes[..]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_flattened_images_the_thumbnail_is_still_written() {
        let doc = two_set_doc();
        let dir = temp_dir("project-noflat");
        let path = dir.join("p.wtep");
        let opts = SaveOptions { write_flattened: false, ..options(1) };
        save(&doc.snapshot(), &path, &opts, &NoProgress).unwrap();
        let manifest = manifest_of(&path);
        assert!(manifest.sets.iter().all(|s| s.flattened.is_none()));
        let mut zip = archive(&path);
        assert!(!zip.file_names().any(|n| n.unwrap().ends_with("flattened.png")));
        let thumb = decode_image(&entry_bytes(&mut zip, "thumbnail.png"), None, 16384).unwrap().pixels;
        // Set 0 is 48 x 32: a thumbnail never enlarges.
        assert_eq!((thumb.width, thumb.height), (48, 32));
        assert_same_document(&doc, &load(&path, &NoProgress).unwrap().document);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_project_is_self_contained() {
        let src = temp_dir("project-selfcontained-src");
        let texture = noise_texture(32, 32, 9);
        for (name, bytes) in obj_files("Crate Box", &crate::samples::seam_cube(2.0), &[("Crate Tex.png", &texture)]) {
            std::fs::write(src.join(name), bytes).unwrap();
        }
        let model = import_from_path(&src.join("Crate Box.obj"), &ImportOptions::default(), &NoProgress).unwrap();
        assert_eq!(model.bundle.sidecars.len(), 2);
        let doc = Document::from_import(model, vec![TextureChoice::Detected], &NoProgress).unwrap();
        let dir = temp_dir("project-selfcontained");
        let path = dir.join("crate.wtep");
        save(&doc.snapshot(), &path, &options(3), &NoProgress).unwrap();
        std::fs::remove_dir_all(&src).unwrap();

        let report = load(&path, &NoProgress).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.document.mesh.uv_hash, doc.mesh.uv_hash);
        assert_same_document(&doc, &report.document);
        assert_eq!(report.document.source.main.original_name, "Crate Box.obj");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Cancels from the `after`-th call of `cancelled` on.
    struct CancelAt {
        after: usize,
        calls: AtomicUsize,
    }

    impl Progress for CancelAt {
        fn report(&self, _fraction: f32, _stage: &str) {}

        fn cancelled(&self) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst) >= self.after
        }
    }

    #[test]
    fn failures_leave_the_target_untouched_and_no_temp_file() {
        let mut doc = two_set_doc();
        let dir = temp_dir("project-failures");
        let path = dir.join("p.wtep");
        std::fs::write(&path, b"previous version").unwrap();

        // Cancellation at every point where it is checked.
        let mut point = 0;
        loop {
            let progress = CancelAt { after: point, calls: AtomicUsize::new(0) };
            match save(&doc.snapshot(), &path, &options(1), &progress) {
                Err(ProjectError::Cancelled) => {}
                Ok(_) => break,
                Err(e) => panic!("unexpected error {e}"),
            }
            assert_eq!(std::fs::read(&path).unwrap(), b"previous version");
            assert_eq!(dir_names(&dir), ["p.wtep"], "cancel point {point}");
            point += 1;
        }
        assert!(point > 8, "cancellation is checked between entries ({point} points)");
        std::fs::write(&path, b"previous version").unwrap();

        // A non-finite value never reaches the manifest.
        let paint = doc.sets[0].active.unwrap();
        doc.layer_mut(paint).unwrap().opacity = f32::NAN;
        assert!(matches!(save(&doc.snapshot(), &path, &options(1), &NoProgress), Err(ProjectError::ManifestInvalid(_))));
        assert_eq!(std::fs::read(&path).unwrap(), b"previous version");
        assert_eq!(dir_names(&dir), ["p.wtep"]);

        // A target in a missing directory is an I/O error and creates nothing.
        doc.layer_mut(paint).unwrap().opacity = 1.0;
        let missing = dir.join("missing").join("p.wtep");
        assert!(matches!(save(&doc.snapshot(), &missing, &options(1), &NoProgress), Err(ProjectError::Io(_))));
        assert_eq!(dir_names(&dir), ["p.wtep"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_rotation() {
        let doc = cube_doc(&noise_texture(8, 8, 1));
        let dir = temp_dir("project-bak");
        let path = dir.join("p.wtep");
        std::fs::write(&path, b"v0").unwrap();
        let opts = SaveOptions { keep_backup: true, ..options(1) };
        save(&doc.snapshot(), &path, &opts, &NoProgress).unwrap();
        assert_eq!(dir_names(&dir), ["p.wtep", "p.wtep.bak"]);
        assert_eq!(std::fs::read(dir.join("p.wtep.bak")).unwrap(), b"v0");
        let first = std::fs::read(&path).unwrap();
        save(&doc.snapshot(), &path, &SaveOptions { now_unix: 2, ..opts }, &NoProgress).unwrap();
        assert!(std::fs::read(dir.join("p.wtep.bak")).unwrap() == first);
        assert!(load(&dir.join("p.wtep.bak"), &NoProgress).is_ok());
        assert_eq!(dir_names(&dir), ["p.wtep", "p.wtep.bak"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn entries_load_would_refuse_are_never_saved() {
        let doc = cube_doc(&noise_texture(16, 16, 3));
        let snapshot = doc.snapshot();
        let largest = std::iter::once(&snapshot.source.main).chain(&snapshot.source.sidecars).map(|f| f.bytes.len()).max();
        let largest = largest.unwrap() as u64;
        assert!(check_entry_sizes(&snapshot, largest).is_ok());
        match check_entry_sizes(&snapshot, largest - 1) {
            Err(ProjectError::EntryTooLarge { entry }) => assert!(!entry.is_empty()),
            other => panic!("expected EntryTooLarge, got {other:?}"),
        }
        // The real limit is the one `load` applies.
        assert_eq!(crate::project::load::FILE_LIMIT_FOR_TESTS, MAX_SOURCE_FILE);
    }

    #[test]
    fn helpers() {
        assert_eq!(entry_extension("Brick Wall.PNG"), "png");
        assert_eq!(entry_extension("dir.v2/file"), "bin");
        assert_eq!(entry_extension("a\\b.c\\tex.tga"), "tga");
        assert_eq!(entry_extension("noext"), "bin");
        assert_eq!(entry_extension("x.toolongext"), "bin");
        assert_eq!(entry_extension("x.p-g"), "bin");
        assert_eq!(entry_extension("x."), "bin");
        assert_eq!(thumbnail_side(8192, 4096), 256);
        assert_eq!(thumbnail_side(100, 30), 100);
        assert_eq!(parallel_budget(8192, 8192), 4);
        assert_eq!(parallel_budget(16384, 16384), 1);
        assert_eq!(parallel_budget(1, 1), 1024);
    }
}
