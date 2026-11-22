// Owner: core-persist (C6). Export of ARCHITECTURE.md 5.24 (procedure: 7.4, naming: 7.5): the plan of output files
// and the run that writes them (flattened textures, per-layer PNGs, OpenRaster, PSD).

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::composite::flatten_layers;
use crate::document::{DocumentSnapshot, SetSnapshot};
use crate::error::{ExportError, ImageIoError};
use crate::ids::TextureSetId;
use crate::image_io::{FlatFormat, PngLevel, encode_flat, encode_png};
use crate::layer::{BlendMode, Layer, LayerKind};
use crate::pixel::{CroppedBuf, PixelBuf, PixelView};
use crate::pixel_ops::dilate_islands;
use crate::progress::Progress;
use crate::project::atomic::temp_path_for;
use crate::project::save::parallel_budget;

pub mod naming;
pub mod ora;
pub mod packbits;
pub mod psd;

pub use naming::{flat_name, layer_name, layered_name, sanitize_file_stem};
pub use ora::write_ora;
pub use psd::{PsdDoc, PsdLayer, read_psd, write_psd};

/// What to export and where.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub dir: PathBuf,
    /// Sanitized by the exporter.
    pub base_name: String,
    /// None = all.
    pub sets: Option<Vec<TextureSetId>>,
    /// Flattened texture per set.
    pub flat: Option<FlatFormat>,
    pub per_layer_png: bool,
    pub ora: bool,
    pub psd: bool,
    /// Hidden layers in per-layer/ORA/PSD output (flattened always = visible only).
    pub include_hidden: bool,
    /// Extra seam padding on the flattened texture; 0 = byte-identical to the composite.
    pub dilate_px: u32,
}

/// What `export::run` wrote.
#[derive(Clone, Debug, Default)]
pub struct ExportReport {
    pub files: Vec<PathBuf>,
    pub bytes_written: u64,
}

/// What one planned file contains.
#[derive(Copy, Clone)]
enum Output {
    Flat(FlatFormat),
    /// Full-canvas PNG of the layer with this index in the set's stack (0 = bottom).
    Layer(usize),
    Ora,
    Psd,
}

/// One file of the plan.
struct PlanItem {
    path: PathBuf,
    /// Index into `DocumentSnapshot::sets`.
    set: usize,
    output: Output,
}

/// Indices of the sets to export: all, or the requested ones that exist, ascending and without duplicates.
fn selected_sets(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<usize> {
    match &opts.sets {
        None => (0..snapshot.sets.len()).collect(),
        Some(ids) => {
            let mut indices: Vec<usize> = ids.iter().map(|id| id.index()).filter(|&i| i < snapshot.sets.len()).collect();
            indices.sort_unstable();
            indices.dedup();
            indices
        }
    }
}

/// The layers that go into per-layer and layered output, with their index in the stack.
fn exported_layers(set: &SetSnapshot, include_hidden: bool) -> impl Iterator<Item = (usize, &Layer)> {
    set.layers.iter().enumerate().filter(move |(_, layer)| include_hidden || layer.visible)
}

/// The ordered list of output files: per set the flattened texture, the per-layer PNGs bottom to top, ORA, PSD.
/// Colliding names get `_2`, `_3`, ... (section 7.5).
fn build_plan(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PlanItem> {
    let sets = selected_sets(snapshot, opts);
    let multi = sets.len() > 1;
    let base = opts.base_name.as_str();
    let mut names = naming::NameDeduper::default();
    let mut items = Vec::new();
    let mut push = |name: String, set: usize, output: Output| {
        let unique = names.unique("", &name, |_| false);
        items.push(PlanItem { path: opts.dir.join(unique), set, output });
    };
    for index in sets {
        let set = &snapshot.sets[index];
        if let Some(format) = opts.flat {
            push(flat_name(base, &set.name, multi, format.extension()), index, Output::Flat(format));
        }
        if opts.per_layer_png {
            for (i, layer) in exported_layers(set, opts.include_hidden) {
                push(layer_name(base, &set.name, multi, i, layer.kind_name(), &layer.name), index, Output::Layer(i));
            }
        }
        if opts.ora {
            push(layered_name(base, &set.name, multi, "ora"), index, Output::Ora);
        }
        if opts.psd {
            push(layered_name(base, &set.name, multi, "psd"), index, Output::Psd);
        }
    }
    items
}

/// The exact list of files `run` would write for these options (same naming and de-duplication, same order), without
/// touching the disk. The export dialog shows it and checks it for existing files before starting.
///
/// Per selected set (ascending id; ids that do not exist are ignored): the flattened texture, the per-layer PNGs
/// bottom to top (index = position in the full layer stack), the ORA file, the PSD file.
pub fn plan(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PathBuf> {
    build_plan(snapshot, opts).into_iter().map(|item| item.path).collect()
}

/// Progress, cancellation and bookkeeping of one export run, shared by the sets that are exported at the same time.
struct Run<'a> {
    total: usize,
    progress: &'a dyn Progress,
    /// Set when a set failed: the other sets stop before their next file.
    failed: AtomicBool,
    /// Files written so far (all sets). Reporting under the lock keeps the progress fractions monotone.
    written: Mutex<usize>,
}

/// The files one set wrote (final paths), in plan order, and their total size.
#[derive(Default)]
struct SetReport {
    files: Vec<PathBuf>,
    bytes: u64,
}

/// Share of the progress bar for writing the files; the rest is the final flush (`finish`).
const WRITE_SHARE: f32 = 0.9;

/// Deletes the temp file it holds when dropped, unless `keep` was called: a failed write or a panic that unwinds
/// through it leaves no temp file behind.
struct TempFile(Option<PathBuf>);

impl TempFile {
    fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if let Some(tmp) = self.0.take() {
            let _ = std::fs::remove_file(tmp);
        }
    }
}

impl Run<'_> {
    fn check_cancelled(&self) -> Result<(), ExportError> {
        if self.progress.cancelled() || self.failed.load(Ordering::Relaxed) { Err(ExportError::Cancelled) } else { Ok(()) }
    }

    /// Writes one file to its temp file (`temp_path_for`), closes it and renames it to its final name at once, without
    /// a flush to the disk (`finish` flushes them all at the end), and records it. A failed write (or a panic) deletes
    /// its temp file, so at most the one file in progress per set is ever a temp file.
    fn write(
        &self,
        out: &mut SetReport,
        path: &Path,
        write: impl FnOnce(&mut std::fs::File) -> Result<(), ExportError>,
    ) -> Result<(), ExportError> {
        self.check_cancelled()?;
        let tmp = temp_path_for(path);
        let guard = TempFile(Some(tmp.clone()));
        let bytes = {
            let mut file = std::fs::File::create(&tmp)?;
            write(&mut file)?;
            file.metadata()?.len()
        };
        std::fs::rename(&tmp, path)?;
        guard.keep();
        out.bytes += bytes;
        out.files.push(path.to_path_buf());
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        *written += 1;
        self.progress.report(*written as f32 / self.total as f32 * WRITE_SHARE, "Exporting");
        Ok(())
    }

    fn write_bytes(&self, out: &mut SetReport, path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
        self.write(out, path, |file| Ok(file.write_all(bytes)?))
    }
}

/// A layer as a full-canvas RGBA PNG, so that the files of one set overlay directly.
fn encode_full_canvas(layer: &Layer, width: u32, height: u32) -> Result<Vec<u8>, ImageIoError> {
    match &layer.kind {
        LayerKind::Raster(buf) if buf.width == width && buf.height == height => {
            encode_png(width, height, &buf.data, PngLevel::Balanced)
        }
        LayerKind::Raster(buf) => {
            // An unallocated (transparent) layer; clip or pad anything else instead of failing.
            let dense = crate::pixel::CroppedBuf { rect: buf.rect(), data: buf.data.clone() }.to_dense(width, height);
            encode_png(width, height, &dense.data, PngLevel::Balanced)
        }
        LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => {
            encode_png(width, height, &cache.to_dense(width, height).data, PngLevel::Balanced)
        }
    }
}

/// Writes the planned files of one set, in plan order.
///
/// The ORA file stores PNGs that this set's other files may already have encoded from the same pixels at the same
/// level: the flattened PNG (`Png(Balanced)`) is its `mergedimage.png`, and a per-layer PNG is the layer's entry when
/// the layer's content covers the whole canvas. They are kept until the ORA is written and stored as they are, so the
/// archive is byte-identical to one encoded on its own (`ora::OraReuse`).
fn export_set(
    set: &SetSnapshot,
    items: &[PlanItem],
    opts: &ExportOptions,
    run: &Run<'_>,
    out: &mut SetReport,
) -> Result<(), ExportError> {
    let (width, height) = (set.width, set.height);
    let needs_flat = items.iter().any(|item| !matches!(item.output, Output::Layer(_)));
    let flat: Option<PixelBuf> = if needs_flat {
        run.check_cancelled()?;
        let mut flat = flatten_layers(&set.layers, width, height, false);
        if opts.dilate_px > 0 {
            dilate_islands(&mut flat, &set.texel_map, opts.dilate_px);
        }
        Some(flat)
    } else {
        None
    };
    // ORA / PSD store one rectangle per layer: a stamp cache of several parts is joined once, only when needed.
    let needs_layered = items.iter().any(|item| matches!(item.output, Output::Ora | Output::Psd));
    let needs_ora = items.iter().any(|item| matches!(item.output, Output::Ora));
    let joined: Vec<Option<CroppedBuf>> = if needs_layered {
        set.layers
            .iter()
            .map(|layer| match &layer.kind {
                LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => Some(cache.to_cropped()),
                LayerKind::Raster(_) => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    let layered = || -> Option<ExportSet<'_>> {
        Some(ExportSet {
            name: &set.name,
            width,
            height,
            flattened: flat.as_ref()?,
            layers: exported_layers(set, opts.include_hidden)
                .map(|(i, layer)| ExportLayer {
                    name: &layer.name,
                    kind: layer.kind_name(),
                    visible: layer.visible,
                    opacity: layer.opacity,
                    blend: layer.blend,
                    pixels: match (&layer.kind, joined.get(i)) {
                        (_, Some(Some(cropped))) => cropped.view(),
                        (LayerKind::Raster(buf), _) => buf.view(),
                        _ => PixelView { rect: crate::math::IRect::EMPTY, data: &[] },
                    },
                })
                .collect(),
        })
    };
    let missing_flat = || ExportError::Io(std::io::Error::other("the flattened image was not prepared"));
    // PNGs kept for the ORA file: the flattened one, and the full-canvas one per layer (by index in the stack).
    let mut flat_png: Option<Vec<u8>> = None;
    let mut layer_pngs: Vec<Option<Vec<u8>>> = vec![None; if needs_ora { set.layers.len() } else { 0 }];

    let mut next = 0;
    while next < items.len() {
        let item = &items[next];
        match item.output {
            Output::Layer(_) => {
                // The run of per-layer files: encoded in parallel, a bounded number of canvases at a time.
                let count = items[next..].iter().take_while(|i| matches!(i.output, Output::Layer(_))).count();
                for chunk in items[next..next + count].chunks(parallel_budget(width, height)) {
                    run.check_cancelled()?;
                    let encoded: Vec<_> = chunk
                        .par_iter()
                        .map(|item| match item.output {
                            Output::Layer(i) => encode_full_canvas(&set.layers[i], width, height),
                            _ => Err(ImageIoError::Encode("not a layer file".to_owned())),
                        })
                        .collect();
                    for (item, png) in chunk.iter().zip(encoded) {
                        let png = png?;
                        run.write_bytes(out, &item.path, &png)?;
                        if let Output::Layer(i) = item.output
                            && let Some(kept) = layer_pngs.get_mut(i)
                        {
                            *kept = Some(png);
                        }
                    }
                }
                next += count;
                continue;
            }
            Output::Flat(format) => {
                let bytes = encode_flat(flat.as_ref().ok_or_else(missing_flat)?, &format)?;
                run.write_bytes(out, &item.path, &bytes)?;
                if needs_ora && format == FlatFormat::Png(PngLevel::Balanced) {
                    flat_png = Some(bytes);
                }
            }
            Output::Ora => {
                let export = layered().ok_or_else(missing_flat)?;
                let reuse = ora::OraReuse {
                    merged: flat_png.as_deref(),
                    full_canvas_layers: exported_layers(set, opts.include_hidden)
                        .map(|(i, _)| layer_pngs.get(i).and_then(|png| png.as_deref()))
                        .collect(),
                };
                run.write(out, &item.path, |file| {
                    ora::write_ora_reusing(BufWriter::with_capacity(1 << 20, file), &export, &reuse)
                })?;
                drop(reuse);
                flat_png = None;
                layer_pngs.clear();
            }
            Output::Psd => {
                let export = layered().ok_or_else(missing_flat)?;
                run.write(out, &item.path, |file| write_psd(BufWriter::with_capacity(1 << 20, file), &export))?;
            }
        }
        next += 1;
    }
    Ok(())
}

/// Flushes every written file to the disk, in plan order, reporting the stage "Finishing" (the last
/// `1 - WRITE_SHARE` of the bar) after each. Flushing them all at the end instead of one by one lets the system write
/// them in large runs: on the USB hard disk of the round-3 measurements, 326 MB took 8-11 s this way and 14-18 s with a
/// flush per file. Cancellation is honoured between files (the files stay, complete, under their final names); a file
/// that cannot be flushed is reported, the others are still flushed, and the first error is returned.
fn finish(sets: &[SetReport], run: &Run<'_>) -> Result<(), ExportError> {
    let files: Vec<&PathBuf> = sets.iter().flat_map(|set| &set.files).collect();
    let mut first = None;
    for (k, path) in files.iter().enumerate() {
        if run.progress.cancelled() {
            return Err(first.unwrap_or(ExportError::Cancelled));
        }
        if let Err(e) = std::fs::OpenOptions::new().write(true).open(path).and_then(|file| file.sync_all()) {
            first.get_or_insert(ExportError::Io(e));
        }
        let fraction = if k + 1 == files.len() { 1.0 } else { WRITE_SHARE + (1.0 - WRITE_SHARE) * (k + 1) as f32 / files.len() as f32 };
        run.progress.report(fraction, "Finishing");
    }
    first.map_or(Ok(()), Err)
}

/// What one set wrote, and how its export ended.
type SetOutcome = (SetReport, Result<(), ExportError>);

/// The plan as runs of items per set: (index into `DocumentSnapshot::sets`, range of plan items).
type SetGroup = (usize, std::ops::Range<usize>);

/// How many sets are exported at the same time: as many as fit into the 1 GiB budget of `parallel_budget` when each
/// set in flight counts its exported layers plus three canvases (flattened image, joined stamp caches, crops), at most
/// one per thread of the pool, at least one. Size and layer count are those of the largest selected set.
fn sets_in_flight(snapshot: &DocumentSnapshot, opts: &ExportOptions, groups: &[SetGroup]) -> usize {
    let (mut canvas, mut layers) = (1u64, 0u64);
    for (set, _) in groups {
        let set = &snapshot.sets[*set];
        canvas = canvas.max(set.width as u64 * set.height as u64 * 4);
        layers = layers.max(exported_layers(set, opts.include_hidden).count() as u64);
    }
    let fit = (1u64 << 30) / (canvas * (layers + 3));
    usize::try_from(fit).unwrap_or(usize::MAX).clamp(1, rayon::current_num_threads().max(1)).min(groups.len().max(1))
}

/// Creates `opts.dir` (create_dir_all), then writes every file of `plan` (each one atomically, replacing an existing
/// file of the same name). Flattened pixels come from flatten_layers(.., false).
///
/// An empty plan is `NothingToExport` (nothing is created). `dilate_px > 0` pads the flattened texture — also the
/// merged image inside ORA / PSD — with `dilate_islands`. Several sets are exported at the same time
/// (`sets_in_flight`); the files of one set are written one after the other in plan order, and `ExportReport::files`
/// is in plan order. Each file goes to its temp file (`<name>.tmp-<pid>`) first and is renamed to its final name as
/// soon as it is closed, so a file under its final name is always complete and at most one temp file per set in
/// flight exists at a time (none after a failed write or a panic); at the end every written file is flushed to the
/// disk (`finish`). Progress is reported per written file ("Exporting", up to `WRITE_SHARE`) and per flushed file
/// ("Finishing", up to 1); cancellation is honoured between files (`Cancelled`; the files written so far stay). When a set fails, the others stop before their next file, and the first failure in plan order
/// that is not a cancellation is returned.
pub fn run(snapshot: &DocumentSnapshot, opts: &ExportOptions, progress: &dyn Progress) -> Result<ExportReport, ExportError> {
    run_in_lanes(snapshot, opts, progress, usize::MAX)
}

/// `run` with at most `max_lanes` sets in flight (tests compare 1 with several).
fn run_in_lanes(
    snapshot: &DocumentSnapshot,
    opts: &ExportOptions,
    progress: &dyn Progress,
    max_lanes: usize,
) -> Result<ExportReport, ExportError> {
    let items = build_plan(snapshot, opts);
    if items.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    let run = Run { total: items.len(), progress, failed: AtomicBool::new(false), written: Mutex::new(0) };
    run.check_cancelled()?;
    progress.report(0.0, "Exporting");
    std::fs::create_dir_all(&opts.dir)?;

    let mut groups: Vec<SetGroup> = Vec::new();
    let mut start = 0;
    while start < items.len() {
        let set = items[start].set;
        let count = items[start..].iter().take_while(|item| item.set == set).count();
        groups.push((set, start..start + count));
        start += count;
    }
    let export_group = |(set, range): &SetGroup| {
        let mut out = SetReport::default();
        let result = export_set(&snapshot.sets[*set], &items[range.clone()], opts, &run, &mut out);
        if result.is_err() {
            run.failed.store(true, Ordering::Relaxed);
        }
        (out, result)
    };
    let lanes = sets_in_flight(snapshot, opts, &groups).min(max_lanes);
    let results: Vec<SetOutcome> = if lanes <= 1 {
        let mut results = Vec::with_capacity(groups.len());
        for group in &groups {
            let result = export_group(group);
            let failed = result.1.is_err();
            results.push(result);
            if failed {
                break;
            }
        }
        results
    } else {
        // `lanes` workers each take the next set until none is left; a set's own encoders share the same pool.
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<SetOutcome>>> = groups.iter().map(|_| Mutex::new(None)).collect();
        rayon::scope(|scope| {
            for _ in 0..lanes {
                scope.spawn(|_| {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(group) = groups.get(index) else { break };
                        let result = export_group(group);
                        *slots[index].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                    }
                });
            }
        });
        slots.into_iter().filter_map(|slot| slot.into_inner().unwrap_or_else(|e| e.into_inner())).collect()
    };

    // Every file written so far is flushed, also after a failure (they stay, as documented).
    let (sets, results): (Vec<SetReport>, Vec<Result<(), ExportError>>) = results.into_iter().unzip();
    let finished = finish(&sets, &run);
    let mut cancelled = false;
    let mut failure = None;
    for result in results.into_iter().chain([finished]) {
        match result {
            Ok(()) => {}
            Err(ExportError::Cancelled) => cancelled = true,
            Err(e) => {
                failure.get_or_insert(e);
            }
        }
    }
    let report = ExportReport {
        files: sets.iter().flat_map(|set| set.files.iter().cloned()).collect(),
        bytes_written: sets.iter().map(|set| set.bytes).sum(),
    };
    match failure {
        Some(e) => Err(e),
        None if cancelled => Err(ExportError::Cancelled),
        None => Ok(report),
    }
}

/// One layer handed to the layered-format writers (ORA, PSD).
pub struct ExportLayer<'a> {
    pub name: &'a str,
    /// "paint" | "stamp" | "text".
    pub kind: &'static str,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub pixels: PixelView<'a>,
}

/// One texture set handed to the layered-format writers (ORA, PSD).
pub struct ExportSet<'a> {
    pub name: &'a str,
    pub width: u32,
    pub height: u32,
    pub flattened: &'a PixelBuf,
    /// Bottom -> top.
    pub layers: Vec<ExportLayer<'a>>,
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use super::*;
    use crate::doc_build::SampleKind;
    use crate::document::Document;
    use crate::image_io::decode_image;
    use crate::math::IRect;
    use crate::progress::NoProgress;
    use crate::project::save::testkit::{cube_doc, noise_texture, paint_block, two_set_doc};
    use crate::testutil::temp_dir;

    fn options(dir: &Path) -> ExportOptions {
        ExportOptions {
            dir: dir.to_path_buf(),
            base_name: "model".to_owned(),
            sets: None,
            flat: None,
            per_layer_png: false,
            ora: false,
            psd: false,
            include_hidden: false,
            dilate_px: 0,
        }
    }

    fn decode(path: &Path) -> PixelBuf {
        decode_image(&std::fs::read(path).unwrap(), path.file_name().and_then(|n| n.to_str()), 16384).unwrap().pixels
    }

    fn file_names(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    fn dir_listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// A cube document with three layers on top of the locked base: a painted one, a hidden one and an empty one.
    fn layered_doc() -> (Document, PixelBuf) {
        let texture = noise_texture(64, 64, 5);
        let mut doc = cube_doc(&texture);
        let set = TextureSetId(0);
        let paint = doc.sets[0].active.unwrap();
        paint_block(&mut doc, paint, IRect::new(8, 8, 40, 30), [200, 40, 40, 255]);
        let hidden = doc.add_raster_layer(set, Some("Hidden: notes")).unwrap();
        paint_block(&mut doc, hidden, IRect::new(20, 20, 60, 60), [10, 200, 10, 128]);
        doc.set_layer_visible(hidden, false).unwrap();
        let top = doc.add_raster_layer(set, Some("Gl\u{fc}hen")).unwrap();
        doc.set_layer_opacity(top, 0.5, crate::document::Commit::Final).unwrap();
        doc.set_layer_blend(top, BlendMode::Multiply).unwrap();
        doc.recomposite_all();
        (doc, texture)
    }

    #[test]
    fn export_of_an_untouched_import_reproduces_the_texture() {
        // Random RGBA with every kind of alpha, including RGB under alpha 0.
        let texture = noise_texture(64, 48, 1);
        assert!(texture.data.as_chunks::<4>().0.iter().any(|p| p[3] == 0 && p[..3] != [0, 0, 0]));
        let doc = cube_doc(&texture);
        let dir = temp_dir("export-identity");
        let opts = ExportOptions { flat: Some(FlatFormat::Png(PngLevel::Fast)), ..options(&dir) };
        let report = run(&doc.snapshot(), &opts, &NoProgress).unwrap();
        assert_eq!(file_names(&report.files), ["model.png"]);
        assert_eq!(decode(&report.files[0]), texture);
        assert_eq!(report.bytes_written, std::fs::metadata(&report.files[0]).unwrap().len());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_of_the_untouched_sample_reproduces_its_checker() {
        let doc = Document::sample(SampleKind::Cube);
        let dir = temp_dir("export-sample");
        let opts = ExportOptions { flat: Some(FlatFormat::Png(PngLevel::Fast)), ..options(&dir) };
        let report = run(&doc.snapshot(), &opts, &NoProgress).unwrap();
        assert_eq!(decode(&report.files[0]), crate::samples::checker_texture(1024, 1024, 12));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn flattened_formats_decode_to_flatten_layers() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let set = &snapshot.sets[0];
        let expected = flatten_layers(&set.layers, set.width, set.height, false);
        assert_eq!(&expected, doc.composite(TextureSetId(0)).unwrap());
        let dir = temp_dir("export-flat");
        for format in [FlatFormat::Png(PngLevel::Balanced), FlatFormat::Tga, FlatFormat::Bmp, FlatFormat::Tiff, FlatFormat::WebpLossless] {
            let opts = ExportOptions { flat: Some(format), ..options(&dir) };
            let report = run(&snapshot, &opts, &NoProgress).unwrap();
            assert_eq!(file_names(&report.files), [format!("model.{}", format.extension())]);
            assert_eq!(decode(&report.files[0]), expected, "{format:?}");
        }
        // JPEG is lossy and has no alpha: only the size is exact.
        let opts = ExportOptions { flat: Some(FlatFormat::Jpeg { quality: 90, matte: [255, 255, 255] }), ..options(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        let jpeg = decode(&report.files[0]);
        assert_eq!((jpeg.width, jpeg.height), (64, 64));
        assert!(jpeg.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn per_layer_pngs_are_full_canvas() {
        let (doc, texture) = layered_doc();
        let snapshot = doc.snapshot();
        let dir = temp_dir("export-layers");
        let opts = ExportOptions { per_layer_png: true, ..options(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        // The hidden layer (index 2) is left out; indices are positions in the full stack.
        assert_eq!(
            file_names(&report.files),
            ["model_L00_paint_Base.png", "model_L01_paint_Paint 1.png", "model_L03_paint_Gl\u{fc}hen.png"]
        );
        assert_eq!(decode(&report.files[0]), texture);
        let paint = decode(&report.files[1]);
        assert_eq!((paint.width, paint.height), (64, 64));
        assert_eq!(paint.get(8, 8), [200, 40, 40, 255]);
        assert_eq!(paint.get(7, 8), [0; 4]);
        assert_eq!(decode(&report.files[2]), PixelBuf::new(64, 64));

        let opts = ExportOptions { per_layer_png: true, include_hidden: true, ..options(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(
            file_names(&report.files),
            [
                "model_L00_paint_Base.png",
                "model_L01_paint_Paint 1.png",
                "model_L02_paint_Hidden_ notes.png",
                "model_L03_paint_Gl\u{fc}hen.png"
            ]
        );
        let hidden = decode(&report.files[2]);
        assert_eq!((hidden.width, hidden.height), (64, 64));
        assert_eq!(hidden.get(20, 20), [10, 200, 10, 128]);
        assert_eq!(hidden.get(19, 20), [0; 4]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plan_lists_exactly_what_run_writes_and_run_creates_the_directory() {
        let mut doc = two_set_doc();
        let paint = doc.sets[1].active.unwrap();
        paint_block(&mut doc, paint, IRect::new(0, 0, 10, 10), [1, 2, 3, 255]);
        doc.recomposite_all();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-plan");
        let dir = root.join("does").join("not exist yet");
        let opts = ExportOptions {
            flat: Some(FlatFormat::Tga),
            per_layer_png: true,
            ora: true,
            psd: true,
            ..options(&dir)
        };
        let planned = plan(&snapshot, &opts);
        assert!(!dir.exists(), "plan must not touch the disk");
        let (body, head) = (&snapshot.sets[0].name, &snapshot.sets[1].name);
        let expected: Vec<String> = [body, head]
            .iter()
            .flat_map(|set| {
                [
                    format!("model_{set}.tga"),
                    format!("model_{set}_L00_paint_Base.png"),
                    format!("model_{set}_L01_paint_Paint 1.png"),
                    format!("model_{set}.ora"),
                    format!("model_{set}.psd"),
                ]
            })
            .collect();
        assert_eq!(file_names(&planned), expected);
        assert!(planned.iter().all(|p| p.parent() == Some(dir.as_path())));

        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(report.files, planned);
        let mut on_disk = dir_listing(&dir);
        let mut wanted = expected.clone();
        on_disk.sort();
        wanted.sort();
        assert_eq!(on_disk, wanted, "no temp files, nothing else");
        let total: u64 = planned.iter().map(|p| std::fs::metadata(p).unwrap().len()).sum();
        assert_eq!(report.bytes_written, total);

        // A single selected set drops the set name from the file names; unknown ids are ignored.
        let single = ExportOptions { sets: Some(vec![TextureSetId(7), TextureSetId(1), TextureSetId(1)]), ..opts.clone() };
        assert_eq!(
            file_names(&plan(&snapshot, &single)),
            ["model.tga", "model_L00_paint_Base.png", "model_L01_paint_Paint 1.png", "model.ora", "model.psd"]
        );
        // Running again replaces the files.
        let again = run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(again.files, planned);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn layered_files_contain_the_layers() {
        let (doc, texture) = layered_doc();
        let snapshot = doc.snapshot();
        let dir = temp_dir("export-layered");
        let opts = ExportOptions { ora: true, psd: true, ..options(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(file_names(&report.files), ["model.ora", "model.psd"]);
        let flat = doc.composite(TextureSetId(0)).unwrap();

        // PSD: the three visible layers, bottom first.
        let psd = read_psd(&std::fs::read(&report.files[1]).unwrap()).unwrap();
        assert_eq!((psd.width, psd.height), (64, 64));
        let names: Vec<&str> = psd.layers.iter().map(|l| l.unicode_name.as_str()).collect();
        assert_eq!(names, ["Base", "Paint 1", "Gl\u{fc}hen"]);
        assert_eq!(psd.layers[1].rect, IRect::new(8, 8, 40, 30));
        assert_eq!(psd.layers[1].pixels.get(8, 8), [200, 40, 40, 255]);
        assert_eq!((psd.layers[2].blend_key, psd.layers[2].opacity), (*b"mul ", 128));
        assert!(psd.layers[2].pixels.is_empty());
        assert_eq!(psd.layers[0].name, "Base");
        // The base layer is the texture wherever it is not fully transparent.
        for (got, want) in psd.layers[0].pixels.to_dense(64, 64).data.as_chunks::<4>().0.iter().zip(texture.data.as_chunks::<4>().0) {
            if want[3] != 0 {
                assert_eq!(got, want);
            }
        }
        assert_eq!(psd.merged.data.as_chunks::<4>().0.len(), flat.data.as_chunks::<4>().0.len());
        for (got, want) in psd.merged.data.as_chunks::<4>().0.iter().zip(flat.data.as_chunks::<4>().0) {
            assert_eq!(got[3], want[3]);
        }

        // ORA: the stack lists them top first and the merged image is the composite.
        let mut zip = zip::ZipArchive::new(Cursor::new(std::fs::read(&report.files[0]).unwrap())).unwrap();
        let mut xml = String::new();
        zip.by_name("stack.xml").unwrap().read_to_string(&mut xml).unwrap();
        let order: Vec<usize> = ["name=\"Gl\u{fc}hen\"", "name=\"Paint 1\"", "name=\"Base\""].iter().map(|n| xml.find(n).unwrap()).collect();
        assert!(order[0] < order[1] && order[1] < order[2], "{xml}");
        assert!(!xml.contains("Hidden"));
        let mut merged = Vec::new();
        zip.by_name("mergedimage.png").unwrap().read_to_end(&mut merged).unwrap();
        assert_eq!(&decode_image(&merged, None, 16384).unwrap().pixels, flat);

        // With include_hidden the hidden layer is part of both files, flagged as hidden.
        let opts = ExportOptions { ora: true, psd: true, include_hidden: true, ..options(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        let psd = read_psd(&std::fs::read(&report.files[1]).unwrap()).unwrap();
        assert_eq!(psd.layers.len(), 4);
        assert_eq!(psd.layers[2].unicode_name, "Hidden: notes");
        assert!(psd.layers[2].hidden && !psd.layers[1].hidden);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn colliding_names_are_numbered() {
        let mut doc = two_set_doc();
        // Two sets whose names sanitize to the same stem, and two layers with the same name.
        doc.sets[0].name = "skin:a".to_owned();
        doc.sets[1].name = "skin_a".to_owned();
        doc.add_raster_layer(TextureSetId(0), Some("Paint 1")).unwrap();
        let snapshot = doc.snapshot();
        let opts = ExportOptions {
            base_name: "con".to_owned(),
            flat: Some(FlatFormat::Png(PngLevel::Fast)),
            per_layer_png: true,
            psd: true,
            ..options(Path::new("unused"))
        };
        let names = file_names(&plan(&snapshot, &opts));
        assert_eq!(
            names,
            [
                "_con_skin_a.png",
                "_con_skin_a_L00_paint_Base.png",
                "_con_skin_a_L01_paint_Paint 1.png",
                "_con_skin_a_L02_paint_Paint 1.png",
                "_con_skin_a.psd",
                "_con_skin_a_2.png",
                "_con_skin_a_L00_paint_Base_2.png",
                "_con_skin_a_L01_paint_Paint 1_2.png",
                "_con_skin_a_2.psd",
            ]
        );
        let unique: std::collections::BTreeSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn nothing_selected_is_an_error_and_creates_nothing() {
        let doc = cube_doc(&noise_texture(16, 16, 2));
        let snapshot = doc.snapshot();
        let root = temp_dir("export-nothing");
        let dir = root.join("out");
        // No output kind.
        assert!(matches!(run(&snapshot, &options(&dir), &NoProgress), Err(ExportError::NothingToExport)));
        // No (existing) set.
        let opts = ExportOptions { psd: true, sets: Some(vec![TextureSetId(9)]), ..options(&dir) };
        assert!(plan(&snapshot, &opts).is_empty());
        assert!(matches!(run(&snapshot, &opts, &NoProgress), Err(ExportError::NothingToExport)));
        let opts = ExportOptions { psd: true, sets: Some(Vec::new()), ..options(&dir) };
        assert!(matches!(run(&snapshot, &opts, &NoProgress), Err(ExportError::NothingToExport)));
        assert!(!dir.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Cancels once `after` written files have been reported (the "Finishing" reports of the flush do not count).
    struct CancelAfter {
        after: usize,
        files: AtomicUsize,
    }

    impl Progress for CancelAfter {
        fn report(&self, fraction: f32, stage: &str) {
            if fraction > 0.0 && stage == "Exporting" {
                self.files.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn cancelled(&self) -> bool {
            self.files.load(Ordering::SeqCst) >= self.after
        }
    }

    #[test]
    fn cancellation_between_files_and_progress_per_file() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let dir = temp_dir("export-cancel");
        let opts = ExportOptions { flat: Some(FlatFormat::Bmp), ora: true, psd: true, ..options(&dir) };
        let progress = CancelAfter { after: 2, files: AtomicUsize::new(0) };
        assert!(matches!(run(&snapshot, &opts, &progress), Err(ExportError::Cancelled)));
        assert_eq!(dir_listing(&dir), ["model.bmp", "model.ora"]);
        // Cancelled before the first file: nothing is written, not even the directory.
        let fresh = dir.join("sub");
        let progress = CancelAfter { after: 0, files: AtomicUsize::new(0) };
        assert!(matches!(run(&snapshot, &ExportOptions { dir: fresh.clone(), ..opts.clone() }, &progress), Err(ExportError::Cancelled)));
        assert!(!fresh.exists());
        // Never cancelled: one report per file.
        let progress = CancelAfter { after: usize::MAX, files: AtomicUsize::new(0) };
        run(&snapshot, &opts, &progress).unwrap();
        assert_eq!(progress.files.load(Ordering::SeqCst), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dilation_pads_only_unowned_transparent_texels() {
        // A transparent texture with a painted layer: the composite is transparent outside the paint.
        // Large enough that the 1/32 atlas margins are wider than the gutter of the texel map.
        const SIDE: u32 = 256;
        let mut doc = cube_doc(&PixelBuf::new(SIDE, SIDE));
        let paint = doc.sets[0].active.unwrap();
        paint_block(&mut doc, paint, IRect::new(0, 0, SIDE as i32, SIDE as i32), [90, 120, 150, 255]);
        // Clear everything the mesh does not own, so there is something to pad.
        let map = doc.sets[0].texel_map.clone();
        let layer_id = paint;
        crate::project::save::testkit::edit_pixels(&mut doc, layer_id, |buf| {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    if map.owner(x, y).is_none() {
                        buf.set(x, y, [0; 4]);
                    }
                }
            }
        });
        doc.recomposite_all();
        let snapshot = doc.snapshot();
        let plain = doc.composite(TextureSetId(0)).unwrap().clone();
        let unowned_clear = (0..SIDE).flat_map(|y| (0..SIDE).map(move |x| (x, y))).filter(|&(x, y)| map.owner(x, y).is_none()).count();
        assert!(unowned_clear > 0, "the cube atlas leaves texels free");

        let dir = temp_dir("export-dilate");
        let png = |dilate_px: u32| {
            let opts = ExportOptions { flat: Some(FlatFormat::Png(PngLevel::Fast)), dilate_px, ..options(&dir) };
            decode(&run(&snapshot, &opts, &NoProgress).unwrap().files[0])
        };
        assert_eq!(png(0), plain, "dilate_px == 0 is the composite itself");
        let padded = png(4);
        let mut filled = 0;
        for y in 0..SIDE {
            for x in 0..SIDE {
                let (before, after) = (plain.get(x, y), padded.get(x, y));
                if map.owner(x, y).is_some() || before[3] != 0 {
                    assert_eq!(before, after, "owned or opaque texel {x},{y} changed");
                } else if after != before {
                    assert_eq!(after, [90, 120, 150, 255]);
                    filled += 1;
                }
            }
        }
        assert!(filled > 0, "padding filled nothing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Every file of a directory with its bytes, by name.
    fn dir_bytes(dir: &Path) -> Vec<(String, Vec<u8>)> {
        dir_listing(dir).into_iter().map(|name| (name.clone(), std::fs::read(dir.join(&name)).unwrap())).collect()
    }

    /// Two sets, each with a painted layer over its base, and an extra half-transparent layer on the first one.
    fn painted_two_set_doc() -> Document {
        let mut doc = two_set_doc();
        for (set, color) in [(0usize, [200, 30, 30, 255]), (1, [20, 30, 200, 255])] {
            let paint = doc.sets[set].active.unwrap();
            paint_block(&mut doc, paint, IRect::new(2, 3, 20, 17), color);
        }
        let extra = doc.add_raster_layer(TextureSetId(0), Some("Glaze")).unwrap();
        paint_block(&mut doc, extra, IRect::new(10, 0, 30, 12), [90, 200, 90, 128]);
        doc.recomposite_all();
        doc
    }

    fn every_format(dir: &Path) -> ExportOptions {
        ExportOptions { flat: Some(FlatFormat::Png(PngLevel::Balanced)), per_layer_png: true, ora: true, psd: true, ..options(dir) }
    }

    /// Records every reported fraction.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<f32>>);

    impl Progress for Recorder {
        fn report(&self, fraction: f32, _stage: &str) {
            self.0.lock().unwrap().push(fraction);
        }

        fn cancelled(&self) -> bool {
            false
        }
    }

    #[test]
    fn sets_in_parallel_write_the_same_bytes_in_plan_order() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-parallel");
        let (serial_dir, parallel_dir, ora_dir) = (root.join("serial"), root.join("parallel"), root.join("ora"));
        let groups: Vec<SetGroup> = vec![(0, 0..6), (1, 6..11)];
        assert!(rayon::current_num_threads() == 1 || sets_in_flight(&snapshot, &every_format(&root), &groups) == 2);

        let serial = run_in_lanes(&snapshot, &every_format(&serial_dir), &NoProgress, 1).unwrap();
        let progress = Recorder::default();
        let parallel = run_in_lanes(&snapshot, &every_format(&parallel_dir), &progress, usize::MAX).unwrap();
        // The report lists the plan, in plan order, both ways.
        assert_eq!(serial.files, plan(&snapshot, &every_format(&serial_dir)));
        assert_eq!(parallel.files, plan(&snapshot, &every_format(&parallel_dir)));
        assert_eq!(file_names(&parallel.files).len(), 11);
        assert_eq!(serial.bytes_written, parallel.bytes_written);
        // Byte-identical files, and no temp file left.
        let (a, b) = (dir_bytes(&serial_dir), dir_bytes(&parallel_dir));
        assert!(a.iter().chain(&b).all(|(name, _)| !name.contains(".tmp-")));
        assert_eq!(a.len(), 11);
        for ((name_a, bytes_a), (name_b, bytes_b)) in a.iter().zip(&b) {
            assert_eq!(name_a, name_b);
            assert!(bytes_a == bytes_b, "{name_a} differs");
        }
        // One report per written file after the initial 0 (up to WRITE_SHARE), then one per flushed file, monotone,
        // ending at 1.
        let fractions = progress.0.into_inner().unwrap();
        assert_eq!(fractions.len(), 23);
        assert_eq!(fractions[0], 0.0);
        assert_eq!(fractions[11], WRITE_SHARE);
        assert!(fractions.windows(2).all(|w| w[0] < w[1]), "{fractions:?}");
        assert_eq!(*fractions.last().unwrap(), 1.0);

        // The ORA files, written alone (nothing to reuse), are the same bytes as those that reused the PNGs.
        let alone = ExportOptions { ora: true, ..options(&ora_dir) };
        let report = run(&snapshot, &alone, &NoProgress).unwrap();
        for path in &report.files {
            let name = path.file_name().unwrap();
            assert!(std::fs::read(path).unwrap() == std::fs::read(parallel_dir.join(name)).unwrap(), "{name:?}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cancelling_a_parallel_export_leaves_only_complete_planned_files() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let dir = temp_dir("export-parallel-cancel");
        let progress = CancelAfter { after: 3, files: AtomicUsize::new(0) };
        assert!(matches!(run_in_lanes(&snapshot, &every_format(&dir), &progress, usize::MAX), Err(ExportError::Cancelled)));
        let planned = file_names(&plan(&snapshot, &every_format(&dir)));
        let written = dir_listing(&dir);
        // Each set checks before every file: at most one more file per other set in flight.
        assert!((3..=4).contains(&written.len()), "{written:?}");
        assert!(written.iter().all(|name| planned.contains(name)), "{written:?}");
        // Complete: every written file decodes (PNG) or opens (ORA, PSD).
        for name in &written {
            let bytes = std::fs::read(dir.join(name)).unwrap();
            if name.ends_with(".png") {
                decode(&dir.join(name));
            } else if name.ends_with(".psd") {
                read_psd(&bytes).unwrap();
            } else {
                zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Panics in `report` once `after` files were written.
    struct PanicAfter {
        after: usize,
        files: AtomicUsize,
    }

    impl Progress for PanicAfter {
        fn report(&self, fraction: f32, _stage: &str) {
            if fraction > 0.0 && self.files.fetch_add(1, Ordering::SeqCst) + 1 >= self.after {
                panic!("progress sink failed");
            }
        }

        fn cancelled(&self) -> bool {
            false
        }
    }

    /// Review finding (round 3): every file used to stay a temp file until the end of the run, so a panic (or a
    /// killed process) left all of them in the export folder. Now each file is renamed as soon as it is written.
    #[test]
    fn a_panic_during_an_export_leaves_no_temp_files() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        for lanes in [1, usize::MAX] {
            let dir = temp_dir("export-panic");
            let progress = PanicAfter { after: 3, files: AtomicUsize::new(0) };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_in_lanes(&snapshot, &every_format(&dir), &progress, lanes)
            }));
            assert!(result.is_err(), "lanes {lanes}: the panic reaches the caller");
            let written = dir_listing(&dir);
            assert!(!written.is_empty(), "lanes {lanes}");
            assert!(written.iter().all(|name| !name.contains(".tmp-")), "lanes {lanes}: {written:?}");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_failing_set_stops_the_export_and_its_error_is_returned() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        for lanes in [1, usize::MAX] {
            let dir = temp_dir("export-parallel-fail");
            let opts = every_format(&dir);
            // A folder where the temp file of the first set's PSD should go: writing it fails.
            let planned = plan(&snapshot, &opts);
            let blocked = planned[5].clone();
            assert!(blocked.to_string_lossy().ends_with(".psd"));
            let blocker = crate::project::atomic::temp_path_for(&blocked);
            std::fs::create_dir_all(blocker.join("in the way")).unwrap();
            let result = run_in_lanes(&snapshot, &opts, &NoProgress, lanes);
            assert!(matches!(result, Err(ExportError::Io(_))), "lanes {lanes}: {:?}", result.map(|r| r.files));
            // The files written before the failure are finished; no temp file is left (only the blocking folder).
            let written = dir_listing(&dir);
            for path in &planned[..5] {
                assert!(written.contains(&path.file_name().unwrap().to_string_lossy().into_owned()), "lanes {lanes}: {written:?}");
            }
            let blocker_name = blocker.file_name().unwrap().to_string_lossy().into_owned();
            assert!(written.iter().all(|n| !n.contains(".tmp-") || *n == blocker_name), "lanes {lanes}: {written:?}");
            assert!(!written.contains(&blocked.file_name().unwrap().to_string_lossy().into_owned()));
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// Timing probe on a real project (not a regression test, fix round 3 item R3-5): `cargo test -p wte_core
    /// --release -- --ignored export_bench --nocapture`. `WTE_EXPORT_BENCH_PROJECT` names a .wtep (default: the
    /// round-3 audit's ABeautifulGame workflow project). Times each stage of every set alone, then the whole `run`.
    /// `WTE_EXPORT_BENCH_STAGES=0` skips the stages, `WTE_EXPORT_BENCH_RUNS` (2), `WTE_EXPORT_BENCH_LANES` (no cap) and
    /// `WTE_EXPORT_BENCH_OUT` (a folder that is kept) shape the runs.
    #[test]
    #[ignore = "timing probe on a local project file"]
    fn export_bench_real_project() {
        let t0 = std::time::Instant::now();
        let path = match std::env::var("WTE_EXPORT_BENCH_PROJECT") {
            Ok(p) => PathBuf::from(p),
            Err(_) => Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../../round3_audit/runs/workflow_path_glb_samples_ABeautifulGame.glb/shot.wtep"),
        };
        if !path.exists() {
            eprintln!("no project at {}", path.display());
            return;
        }
        let doc = crate::project::load(&path, &NoProgress).unwrap().document;
        let snapshot = doc.snapshot();
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
        let mut sums = [0.0f64; 5];
        let stages = std::env::var("WTE_EXPORT_BENCH_STAGES").map_or(true, |v| v != "0");
        for set in snapshot.sets.iter().filter(|_| stages) {
            let t = std::time::Instant::now();
            let flat = flatten_layers(&set.layers, set.width, set.height, false);
            let flatten = ms(t);
            let t = std::time::Instant::now();
            let png = encode_png(set.width, set.height, &flat.data, PngLevel::Balanced).unwrap();
            let flat_png = ms(t);
            let t = std::time::Instant::now();
            for layer in &set.layers {
                encode_full_canvas(layer, set.width, set.height).unwrap();
            }
            let layers = ms(t);
            let joined: Vec<Option<CroppedBuf>> = set
                .layers
                .iter()
                .map(|layer| match &layer.kind {
                    LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => Some(cache.to_cropped()),
                    LayerKind::Raster(_) => None,
                })
                .collect();
            let export = ExportSet {
                name: &set.name,
                width: set.width,
                height: set.height,
                flattened: &flat,
                layers: set
                    .layers
                    .iter()
                    .enumerate()
                    .map(|(i, layer)| ExportLayer {
                        name: &layer.name,
                        kind: layer.kind_name(),
                        visible: layer.visible,
                        opacity: layer.opacity,
                        blend: layer.blend,
                        pixels: match (&layer.kind, &joined[i]) {
                            (_, Some(cropped)) => cropped.view(),
                            (LayerKind::Raster(buf), _) => buf.view(),
                            _ => PixelView { rect: IRect::EMPTY, data: &[] },
                        },
                    })
                    .collect(),
            };
            let t = std::time::Instant::now();
            let mut ora = Cursor::new(Vec::new());
            write_ora(&mut ora, &export).unwrap();
            let ora_ms = ms(t);
            let t = std::time::Instant::now();
            let mut psd = Vec::new();
            write_psd(&mut psd, &export).unwrap();
            let psd_ms = ms(t);
            eprintln!(
                "set {} {}x{} layers {}: flatten {flatten:.0} ms, flat png {flat_png:.0} ms ({} B), layer pngs {layers:.0} ms, ora {ora_ms:.0} ms, psd {psd_ms:.0} ms ({} B)",
                set.name,
                set.width,
                set.height,
                set.layers.len(),
                png.len(),
                psd.len()
            );
            for (sum, v) in sums.iter_mut().zip([flatten, flat_png, layers, ora_ms, psd_ms]) {
                *sum += v;
            }
        }
        eprintln!(
            "sum: flatten {:.0} ms, flat png {:.0} ms, layer pngs {:.0} ms, ora {:.0} ms, psd {:.0} ms",
            sums[0], sums[1], sums[2], sums[3], sums[4]
        );
        let keep = std::env::var("WTE_EXPORT_BENCH_OUT").ok().map(PathBuf::from);
        let dir = keep.clone().unwrap_or_else(|| temp_dir("export-bench"));
        let runs: usize = std::env::var("WTE_EXPORT_BENCH_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let lanes: usize = std::env::var("WTE_EXPORT_BENCH_LANES").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
        eprintln!("export start at {:.0} ms", t0.elapsed().as_secs_f64() * 1e3);
        let opts = ExportOptions {
            flat: Some(FlatFormat::Png(PngLevel::Balanced)),
            per_layer_png: true,
            ora: true,
            psd: true,
            base_name: doc.name.clone(),
            ..options(&dir)
        };
        for _ in 0..runs {
            let t = std::time::Instant::now();
            let report = run_in_lanes(&snapshot, &opts, &NoProgress, lanes).unwrap();
            eprintln!(
                "run (lanes cap {lanes}): {} files, {} B in {:.0} ms, ends at {:.0} ms",
                report.files.len(),
                report.bytes_written,
                ms(t),
                ms(t0)
            );
        }
        if keep.is_none() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
