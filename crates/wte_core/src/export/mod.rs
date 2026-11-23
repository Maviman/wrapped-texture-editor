// Owner: core-persist (C6). Export of ARCHITECTURE.md 5.24 (procedure: 7.4, naming: 7.5): the plan of output files
// and the run that writes them (flattened textures, per-layer PNGs, OpenRaster, PSD).

use std::io::{BufWriter, Write};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::composite::flatten_layers;
use crate::document::{DocumentSnapshot, SetSnapshot};
use crate::error::{ExportError, ImageIoError};
use crate::ids::{LayerId, StampId, TextureSetId};
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

pub use naming::{flat_name, layer_name, layered_name, package_name, sanitize_file_stem, selection_package_name};
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
    /// Layer scope. `None` = every layer that `include_hidden` lets through (as before). `Some(ids)` = exactly these
    /// layers, hidden ones included; `include_hidden` is ignored; ids that do not exist are ignored; a set that holds
    /// none of them is left out completely. The flattened texture (and the merged image of ORA / PSD) is then the
    /// selected layers that are visible, flattened bottom to top; ORA / PSD hold the selected layers only.
    pub layers: Option<Vec<LayerId>>,
    /// Loose files, or one `<base>.zip` holding them.
    pub package: ExportPackage,
}

/// How the output of an export is written.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ExportPackage {
    /// Loose files in `dir` (as before).
    #[default]
    Files,
    /// One `<base>.zip` in `dir` holding those files under the same names (flat, no folders).
    Zip,
}

/// What `export::run` wrote.
#[derive(Clone, Debug, Default)]
pub struct ExportReport {
    /// `Files`: the written files in plan order. `Zip`: the one zip.
    pub files: Vec<PathBuf>,
    /// `Files`: their total size. `Zip`: the size of the zip.
    pub bytes_written: u64,
    /// `Zip`: the number of files inside. `Files`: 0.
    pub packaged: usize,
}

/// One file of an export as the dialog lists it (`plan_contents`): its name and what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFile {
    /// File name (also the entry name inside a zip package).
    pub name: String,
    pub set: TextureSetId,
    pub kind: PlannedKind,
}

/// What a planned file holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlannedKind {
    /// The flattened texture of the set.
    Flat,
    /// A per-layer PNG: the layer, its position in the set's full stack (0 = bottom) and whether it is a paint layer
    /// (false = a stamp or text layer).
    Layer { layer: LayerId, index: usize, raster: bool },
    Ora,
    Psd,
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

/// A design (stamp or text) that spans several texture sets has one layer per set, all linked to the same `StampId`
/// (`LayerKind::Stamp { stamp, .. }` / `LayerKind::Text { stamp, .. }`, `Document::stamp_layers`). The selection
/// `ids` completed by those linked layers: the ids as given, then the linked layers of their stamps in the other sets
/// (set order), without duplicates. Ids that do not exist stay (they are ignored later). The export dialog shows the
/// count of the added layers.
pub fn linked_selection(snapshot: &DocumentSnapshot, ids: &[LayerId]) -> Vec<LayerId> {
    let stamps: Vec<StampId> = snapshot
        .sets
        .iter()
        .flat_map(|set| &set.layers)
        .filter(|layer| ids.contains(&layer.id))
        .filter_map(Layer::stamp_id)
        .collect();
    let mut out: Vec<LayerId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    if stamps.is_empty() {
        return out;
    }
    for layer in snapshot.sets.iter().flat_map(|set| &set.layers) {
        if layer.stamp_id().is_some_and(|s| stamps.contains(&s)) && !out.contains(&layer.id) {
            out.push(layer.id);
        }
    }
    out
}

/// `opts` with its layer selection completed by the linked layers of its designs (`linked_selection`), so "Export
/// this layer" on a stamp across two materials writes that stamp's layer of both sets. Borrowed when nothing changes.
fn resolved<'a>(snapshot: &DocumentSnapshot, opts: &'a ExportOptions) -> Cow<'a, ExportOptions> {
    match &opts.layers {
        Some(ids) => {
            let linked = linked_selection(snapshot, ids);
            if linked.as_slice() == ids.as_slice() {
                Cow::Borrowed(opts)
            } else {
                Cow::Owned(ExportOptions { layers: Some(linked), ..opts.clone() })
            }
        }
        None => Cow::Borrowed(opts),
    }
}

/// Indices of the sets to export: all, or the requested ones that exist, ascending and without duplicates. With a
/// layer selection (`opts.layers`), the sets that hold none of the selected layers are left out.
///
/// The second value says whether file names carry the set name. Without a layer selection: the sets to export are
/// more than one (unchanged rule). Under a layer selection: the model has more than one set, whatever `opts.sets` and
/// the selection narrow it to. So a layer selected on a multi-set model keeps its set name (`<base>_<set>_L02_...`),
/// the same name the full export gives that layer, and two sets' layers with the same index and name never share a
/// file name (check_exp_1 D4).
fn selected_sets(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> (Vec<usize>, bool) {
    let mut indices: Vec<usize> = match &opts.sets {
        None => (0..snapshot.sets.len()).collect(),
        Some(ids) => {
            let mut indices: Vec<usize> = ids.iter().map(|id| id.index()).filter(|&i| i < snapshot.sets.len()).collect();
            indices.sort_unstable();
            indices.dedup();
            indices
        }
    };
    let multi = if opts.layers.is_some() { snapshot.sets.len() > 1 } else { indices.len() > 1 };
    if opts.layers.is_some() {
        indices.retain(|&i| exported_layers(&snapshot.sets[i], opts).next().is_some());
    }
    (indices, multi)
}

/// The layers that go into per-layer and layered output, with their index in the stack: those `include_hidden` lets
/// through, or exactly the selected ones (`opts.layers`).
fn exported_layers<'a>(set: &'a SetSnapshot, opts: &'a ExportOptions) -> impl Iterator<Item = (usize, &'a Layer)> + 'a {
    set.layers.iter().enumerate().filter(move |(_, layer)| match &opts.layers {
        None => opts.include_hidden || layer.visible,
        Some(ids) => ids.contains(&layer.id),
    })
}

/// What a planned file holds, as the key of the name map (`reference_names`).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
enum NameKey {
    Flat,
    Layer(usize),
    Ora,
    Psd,
}

/// The file names of one export, the same for every narrower export of the document (fix_export item 4): the names
/// are handed out (`NameDeduper`, `_2`, `_3`, ...) over the *reference plan*, not over the files of this export. The
/// reference plan is every file the full export could write: per set (all sets when names carry the set name, else
/// the selected ones) the flattened texture (`opts.flat`'s extension, PNG when off), every layer of the stack (hidden
/// ones too), ORA and PSD, in plan order. So a selection of the second of two sets whose names collide (`Body` /
/// `body`, `Mat:1` / `Mat_1`, two long names that share their first 64 characters) gets the `_2` name the full export
/// gives that file, and an export with fewer formats or without hidden layers keeps every other name.
fn reference_names(
    snapshot: &DocumentSnapshot,
    opts: &ExportOptions,
    sets: &[usize],
    multi: bool,
) -> HashMap<(usize, NameKey), String> {
    let base = opts.base_name.as_str();
    let reference: Vec<usize> = if multi { (0..snapshot.sets.len()).collect() } else { sets.to_vec() };
    let flat_ext = opts.flat.map_or("png", |format| format.extension());
    let mut names = naming::NameDeduper::default();
    let mut map = HashMap::new();
    for index in reference {
        let set = &snapshot.sets[index];
        let mut give = |key: NameKey, name: String| {
            map.insert((index, key), names.unique("", &name, |_| false));
        };
        give(NameKey::Flat, flat_name(base, &set.name, multi, flat_ext));
        for (i, layer) in set.layers.iter().enumerate() {
            give(NameKey::Layer(i), layer_name(base, &set.name, multi, i, layer.kind_name(), &layer.name));
        }
        give(NameKey::Ora, layered_name(base, &set.name, multi, "ora"));
        give(NameKey::Psd, layered_name(base, &set.name, multi, "psd"));
    }
    map
}

/// The ordered list of output files: per set the flattened texture, the per-layer PNGs bottom to top, ORA, PSD.
/// Names come from `reference_names` (colliding names get `_2`, `_3`, ..., section 7.5).
fn build_plan(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PlanItem> {
    let (sets, multi) = selected_sets(snapshot, opts);
    let names = reference_names(snapshot, opts, &sets, multi);
    let mut items = Vec::new();
    let mut push = |key: NameKey, set: usize, output: Output| {
        let name = names.get(&(set, key)).cloned().unwrap_or_default();
        items.push(PlanItem { path: opts.dir.join(name), set, output });
    };
    for index in sets {
        let set = &snapshot.sets[index];
        if let Some(format) = opts.flat {
            push(NameKey::Flat, index, Output::Flat(format));
        }
        if opts.per_layer_png {
            for (i, _) in exported_layers(set, opts) {
                push(NameKey::Layer(i), index, Output::Layer(i));
            }
        }
        if opts.ora {
            push(NameKey::Ora, index, Output::Ora);
        }
        if opts.psd {
            push(NameKey::Psd, index, Output::Psd);
        }
    }
    items
}

/// The name of the zip of a zip export (`ExportPackage::Zip`): `<base>.zip` for an export of the layers (Visible /
/// All), `<base>_selection.zip` for a layer selection ("Export this layer", "Export selected layers"), so a selection
/// never replaces the full package in the same folder. Section 7.5.
pub fn package_name_for(opts: &ExportOptions) -> String {
    if opts.layers.is_some() { selection_package_name(&opts.base_name) } else { package_name(&opts.base_name) }
}

/// The exact list of files `run` would write for these options (same naming and de-duplication, same order), without
/// touching the disk. The export dialog shows it and checks it for existing files before starting.
///
/// Per selected set (ascending id; ids that do not exist are ignored): the flattened texture, the per-layer PNGs
/// bottom to top (index = position in the full layer stack), the ORA file, the PSD file.
///
/// A layer selection is completed by the linked layers of its designs in the other sets (`linked_selection`).
///
/// `Files`: these are the files written. `Zip`: the one `<base>.zip` or `<base>_selection.zip` (`package_name_for`),
/// or nothing when there is nothing to pack; `plan_contents` lists what goes inside.
pub fn plan(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PathBuf> {
    let opts = resolved(snapshot, opts);
    let items = build_plan(snapshot, &opts);
    match opts.package {
        ExportPackage::Files => items.into_iter().map(|item| item.path).collect(),
        ExportPackage::Zip if items.is_empty() => Vec::new(),
        ExportPackage::Zip => vec![opts.dir.join(package_name_for(&opts))],
    }
}

/// The files of the export in plan order, whatever the package: for `Files` the names of `plan`, for `Zip` the
/// entries of the zip (same names). Touches no disk.
pub fn plan_contents(snapshot: &DocumentSnapshot, opts: &ExportOptions) -> Vec<PlannedFile> {
    build_plan(snapshot, &resolved(snapshot, opts))
        .into_iter()
        .map(|item| {
            let set = &snapshot.sets[item.set];
            let kind = match item.output {
                Output::Flat(_) => PlannedKind::Flat,
                Output::Layer(index) => {
                    let layer = &set.layers[index];
                    PlannedKind::Layer { layer: layer.id, index, raster: layer.is_raster() }
                }
                Output::Ora => PlannedKind::Ora,
                Output::Psd => PlannedKind::Psd,
            };
            let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            PlannedFile { name, set: set.id, kind }
        })
        .collect()
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
        let mut flat = match &opts.layers {
            None => flatten_layers(&set.layers, width, height, false),
            // A layer selection: the selected layers only (flatten_layers skips the hidden ones).
            Some(_) => {
                let selected: Vec<Layer> = exported_layers(set, opts).map(|(_, layer)| layer.clone()).collect();
                flatten_layers(&selected, width, height, false)
            }
        };
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
            layers: exported_layers(set, opts)
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
                    full_canvas_layers: exported_layers(set, opts)
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
        layers = layers.max(exported_layers(set, opts).count() as u64);
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
/// ("Finishing", up to 1); cancellation is honoured between files (`Cancelled`; the files written so far stay). When
/// a set fails, the others stop before their next file, and the first failure in plan order that is not a
/// cancellation is returned.
///
/// `ExportPackage::Zip` writes the same files into one `<base>.zip` instead (`run_zip`): atomic as a whole, nothing
/// is left behind on failure or cancellation.
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
    let opts = resolved(snapshot, opts);
    match opts.package {
        ExportPackage::Files => run_files(snapshot, &opts, progress, max_lanes, true),
        ExportPackage::Zip => run_zip(snapshot, &opts, progress, max_lanes),
    }
}

/// The loose-file export (`ExportPackage::Files`, whatever `opts.package` says). `flush` = run `finish` at the end
/// (the staging run of a zip export skips it: its files are thrown away after packing).
fn run_files(
    snapshot: &DocumentSnapshot,
    opts: &ExportOptions,
    progress: &dyn Progress,
    max_lanes: usize,
    flush: bool,
) -> Result<ExportReport, ExportError> {
    let items = build_plan(snapshot, opts);
    if items.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    let run = Run { total: items.len(), progress, failed: AtomicBool::new(false), written: Mutex::new(0) };
    run.check_cancelled()?;
    progress.report(0.0, "Exporting");
    std::fs::create_dir_all(&opts.dir)?;
    preflight(&opts.dir, items.iter().map(|item| item.path.as_path()))?;

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
    let finished = if flush { finish(&sets, &run) } else { Ok(()) };
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
        packaged: 0,
    };
    match failure {
        Some(e) => Err(e),
        None if cancelled => Err(ExportError::Cancelled),
        None => Ok(report),
    }
}

/// Checks, before anything is written (fix_export item 9), that the output folder takes files (a probe file is
/// created and removed again) and that no planned file is a folder. A read-only or blocked folder then fails before a
/// single file of the export replaces an old one, so no mix of new and old files is left. A failure while writing
/// (a full disk, a file held open by another program) still leaves the files written before it, as documented.
fn preflight<'a>(dir: &Path, planned: impl Iterator<Item = &'a Path>) -> Result<(), ExportError> {
    for path in planned {
        if path.is_dir() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return Err(ExportError::Io(std::io::Error::other(format!("\"{name}\" in the export folder is a folder"))));
        }
    }
    let probe = temp_path_for(&dir.join(".wte-export-probe"));
    std::fs::File::create(&probe)?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Share of the progress bar for writing the files into the stage folder of a zip export; packing takes it up to
/// `PACK_END`, the final flush and rename the rest.
const STAGE_SHARE: f32 = 0.6;
const PACK_END: f32 = 0.95;

/// Forwards the staging run's progress to the caller's, scaled into `0..STAGE_SHARE`.
struct StageProgress<'a>(&'a dyn Progress);

impl Progress for StageProgress<'_> {
    fn report(&self, fraction: f32, stage: &str) {
        self.0.report(fraction / WRITE_SHARE * STAGE_SHARE, stage);
    }

    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }
}

/// Removes the stage folder of a zip export (and whatever is in it) when dropped: on success, failure, cancellation
/// and panic alike.
struct StageDir(PathBuf);

impl Drop for StageDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Removes the output folder when dropped unless `keep` was called; only an empty folder is removed (`remove_dir`),
/// and only one that this export created.
struct CreatedDir(Option<PathBuf>);

impl CreatedDir {
    fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for CreatedDir {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// Zip entry options of a packaged file: already compressed files (PNG, JPEG, WebP, ORA) are stored, the rest
/// (PSD, TGA, BMP, TIFF) deflated at level 6. The time stamp is fixed (1980-01-01 00:00:00) so that two exports of
/// the same document are byte-identical; `large` switches the entry to ZIP64 sizes.
fn package_options(name: &str, large: bool) -> zip::write::SimpleFileOptions {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    let options = if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "ora") {
        crate::project::save::zip_stored()
    } else {
        crate::project::save::zip_deflated()
    };
    options.last_modified_time(zip::DateTime::default()).large_file(large)
}

/// A staged file at least this large gets a ZIP64 entry (its sizes may not fit the 32-bit fields).
const LARGE_ENTRY: u64 = 0xFFFF_0000;

fn zip_io(e: zip::result::ZipError) -> ExportError {
    match e {
        zip::result::ZipError::Io(io) => ExportError::Io(io),
        other => ExportError::Zip(other.to_string()),
    }
}

/// The zip export (section 7.4, "Package"): the unchanged loose-file exporter writes into a stage folder next to the
/// zip (`<zip name>.tmp-<pid>.d`, sets in parallel as usual, no flush), then the staged files are packed in plan
/// order into `<zip name>.tmp-<pid>` (each staged file deleted once packed), the archive is flushed and renamed to
/// the zip (`package_name_for`: `<base>.zip`, or `<base>_selection.zip` for a layer selection). Until that rename
/// nothing named like the zip changes; a failure or a cancellation leaves no zip, no temp file and no stage folder
/// (and removes the output folder again when this export created it and it is empty). Entry names are the loose file
/// names; the entries are byte-identical to the loose files. What a crashed export of another process left behind is
/// removed by `remove_stale_temps`, which the app runs before every export.
fn run_zip(
    snapshot: &DocumentSnapshot,
    opts: &ExportOptions,
    progress: &dyn Progress,
    max_lanes: usize,
) -> Result<ExportReport, ExportError> {
    let contents = plan_contents(snapshot, opts);
    if contents.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    if progress.cancelled() {
        return Err(ExportError::Cancelled);
    }
    progress.report(0.0, "Exporting");
    let created = CreatedDir((!opts.dir.exists()).then(|| opts.dir.clone()));
    std::fs::create_dir_all(&opts.dir)?;
    let zip_path = opts.dir.join(package_name_for(opts));
    preflight(&opts.dir, std::iter::once(zip_path.as_path()))?;
    let tmp_zip = temp_path_for(&zip_path);
    let mut stage_name = tmp_zip.file_name().unwrap_or_default().to_os_string();
    stage_name.push(".d");
    let stage = opts.dir.join(stage_name);
    if stage.is_dir() {
        // Left behind by a process with the same id that did not finish.
        let _ = std::fs::remove_dir_all(&stage);
    }
    std::fs::create_dir(&stage)?;
    let stage_guard = StageDir(stage.clone());

    let staged_opts = ExportOptions { dir: stage.clone(), package: ExportPackage::Files, ..opts.clone() };
    let staged = run_files(snapshot, &staged_opts, &StageProgress(progress), max_lanes, false)?;
    debug_assert_eq!(staged.files.len(), contents.len());

    let guard = TempFile(Some(tmp_zip.clone()));
    let file = std::fs::File::create(&tmp_zip)?;
    let mut zip = zip::ZipWriter::new(BufWriter::with_capacity(1 << 20, file));
    for (k, (path, planned)) in staged.files.iter().zip(&contents).enumerate() {
        if progress.cancelled() {
            return Err(ExportError::Cancelled);
        }
        let mut source = std::fs::File::open(path)?;
        let large = source.metadata()?.len() >= LARGE_ENTRY;
        zip.start_file(planned.name.as_str(), package_options(&planned.name, large)).map_err(zip_io)?;
        std::io::copy(&mut source, &mut zip)?;
        drop(source);
        // Keeps the extra disk use near one file instead of the whole export.
        let _ = std::fs::remove_file(path);
        let fraction = STAGE_SHARE + (PACK_END - STAGE_SHARE) * (k + 1) as f32 / contents.len() as f32;
        progress.report(fraction, "Packing");
    }
    let file = zip.finish().map_err(zip_io)?.into_inner().map_err(|e| ExportError::Io(e.into_error()))?;
    if progress.cancelled() {
        return Err(ExportError::Cancelled);
    }
    progress.report(PACK_END, "Finishing");
    file.sync_all()?;
    let bytes_written = file.metadata()?.len();
    drop(file);
    std::fs::rename(&tmp_zip, &zip_path)?;
    guard.keep();
    drop(stage_guard);
    created.keep();
    progress.report(1.0, "Finishing");
    Ok(ExportReport { files: vec![zip_path], bytes_written, packaged: contents.len() })
}

/// The entries of a zip package (or any zip) in archive order: (name, bytes). Test and self-test helper.
#[doc(hidden)]
pub fn read_package(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        entries.push((name, data));
    }
    Ok(entries)
}

/// Extensions of the files an export writes (flattened formats, per-layer PNGs, ORA, PSD, the zip): only temp files
/// of these are ever removed by `remove_stale_temps`.
const EXPORT_EXTENSIONS: [&str; 11] = ["png", "jpg", "jpeg", "tga", "bmp", "tif", "tiff", "webp", "ora", "psd", "zip"];

/// The process id of an export's temp name, when `name` is one: `<file>.tmp-<pid>` (a temp file of `Run::write` or
/// of the zip) or `<zip>.tmp-<pid>.d` (the stage folder of a zip export), `<file>` having an export extension (a
/// stage folder: `.zip`). The second value says whether it is a stage folder name.
fn temp_pid(name: &str) -> Option<(u32, bool)> {
    let (stem, folder) = match name.strip_suffix(".d") {
        Some(stem) => (stem, true),
        None => (name, false),
    };
    let at = stem.rfind(".tmp-")?;
    let (file, pid) = (&stem[..at], &stem[at + ".tmp-".len()..]);
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let pid: u32 = pid.parse().ok()?;
    let (_, ext) = naming::split_extension(file);
    let ext = ext.to_ascii_lowercase();
    let known = if folder { ext == "zip" } else { EXPORT_EXTENSIONS.contains(&ext.as_str()) };
    known.then_some((pid, folder))
}

/// Removes what an export that did not finish left in `dir` (fix_export item 7): temp files `<file>.tmp-<pid>` and
/// stage folders `<zip>.tmp-<pid>.d` (`temp_pid`) whose process is not running (`is_running(pid)` false) and is not
/// this one. A crashed (killed) zip export leaves its stage folder and temp zip; a crashed loose-file export one temp
/// file per set in flight. Returns what was removed (nothing when `dir` cannot be read). The app runs this in the
/// export job before `run`; `is_running` asks the system (on Windows `OpenProcess` + `GetExitCodeProcess`).
pub fn remove_stale_temps(dir: &Path, is_running: &dyn Fn(u32) -> bool) -> Vec<PathBuf> {
    let own = std::process::id();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some((pid, folder)) = name.to_str().and_then(temp_pid) else { continue };
        if pid == own || is_running(pid) {
            continue;
        }
        let path = entry.path();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let gone = match (folder, is_dir) {
            (true, true) => std::fs::remove_dir_all(&path).is_ok(),
            (false, false) => std::fs::remove_file(&path).is_ok(),
            _ => false,
        };
        if gone {
            log::info!("export: removed {} left by process {pid}, which is not running", path.display());
            removed.push(path);
        }
    }
    removed.sort();
    removed
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
            layers: None,
            package: ExportPackage::Files,
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

    // ---- Layer selection and the zip package (EXPORT_SELECT_PLAN 6.1) ----

    /// The ids of the four layers of `layered_doc`: base, paint, hidden, top.
    fn layer_ids(doc: &Document) -> [LayerId; 4] {
        let ids: Vec<LayerId> = doc.sets[0].layers.iter().map(|l| l.id).collect();
        [ids[0], ids[1], ids[2], ids[3]]
    }

    #[test]
    fn default_options_plan_and_bytes_are_unchanged() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-default");
        let opts = every_format(&root.join("default"));
        assert_eq!((opts.layers.clone(), opts.package), (None, ExportPackage::Files));
        assert_eq!(
            file_names(&plan(&snapshot, &opts)),
            ["model.png", "model_L00_paint_Base.png", "model_L01_paint_Paint 1.png", "model_L03_paint_Gl\u{fc}hen.png", "model.ora", "model.psd"]
        );
        let contents = plan_contents(&snapshot, &opts);
        assert_eq!(contents.iter().map(|f| f.name.clone()).collect::<Vec<_>>(), file_names(&plan(&snapshot, &opts)));
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(report.packaged, 0);
        // Selecting every layer gives exactly the files of "All layers" (a hidden layer adds nothing to the flattened
        // texture either way), byte for byte.
        let all = ExportOptions { include_hidden: true, ..every_format(&root.join("all")) };
        let selected = ExportOptions { layers: Some(layer_ids(&doc).to_vec()), ..every_format(&root.join("selected")) };
        run(&snapshot, &all, &NoProgress).unwrap();
        run(&snapshot, &selected, &NoProgress).unwrap();
        let (a, b) = (dir_bytes(&root.join("all")), dir_bytes(&root.join("selected")));
        assert_eq!(a.len(), 7);
        assert_eq!(a, b);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn plan_with_a_layer_selection() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let [_, paint, _, top] = layer_ids(&doc);
        let opts = ExportOptions { per_layer_png: true, layers: Some(vec![top, LayerId(9999), paint]), ..options(Path::new("out")) };
        // Indices are positions in the full stack, so the names are those of a full export.
        assert_eq!(file_names(&plan(&snapshot, &opts)), ["model_L01_paint_Paint 1.png", "model_L03_paint_Gl\u{fc}hen.png"]);
        let contents = plan_contents(&snapshot, &opts);
        assert_eq!(
            contents.iter().map(|f| (f.set, f.kind)).collect::<Vec<_>>(),
            [
                (TextureSetId(0), PlannedKind::Layer { layer: paint, index: 1, raster: true }),
                (TextureSetId(0), PlannedKind::Layer { layer: top, index: 3, raster: true }),
            ]
        );
        // The order of the id list does not matter.
        let swapped = ExportOptions { layers: Some(vec![paint, top]), ..opts.clone() };
        assert_eq!(plan_contents(&snapshot, &swapped), contents);
        // The other kinds come along per set, in the usual order.
        let all_kinds = ExportOptions { flat: Some(FlatFormat::Tga), ora: true, psd: true, ..opts.clone() };
        let kinds: Vec<PlannedKind> = plan_contents(&snapshot, &all_kinds).into_iter().map(|f| f.kind).collect();
        assert_eq!(kinds.len(), 5);
        assert_eq!((kinds[0], kinds[3], kinds[4]), (PlannedKind::Flat, PlannedKind::Ora, PlannedKind::Psd));
        // A zip package plans one file (a selection's own name, fix_export item 2) and holds the same contents.
        let zipped = ExportOptions { package: ExportPackage::Zip, ..opts.clone() };
        assert_eq!(plan(&snapshot, &zipped), [Path::new("out").join("model_selection.zip")]);
        assert_eq!(plan_contents(&snapshot, &zipped), contents);
    }

    #[test]
    fn a_selected_hidden_layer_is_exported() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let [_, _, hidden, _] = layer_ids(&doc);
        let dir = temp_dir("export-sel-hidden");
        let opts = ExportOptions { per_layer_png: true, ora: true, psd: true, include_hidden: false, layers: Some(vec![hidden]), ..options(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(file_names(&report.files), ["model_L02_paint_Hidden_ notes.png", "model.ora", "model.psd"]);
        assert_eq!(dir_listing(&dir), ["model.ora", "model.psd", "model_L02_paint_Hidden_ notes.png"]);
        let png = decode(&report.files[0]);
        let LayerKind::Raster(buf) = &snapshot.sets[0].layers[2].kind else { panic!("raster") };
        assert_eq!(&png, buf.as_ref());
        assert_eq!(png.get(20, 20), [10, 200, 10, 128]);
        // ORA: the one layer, flagged hidden; the merged image is empty (no visible layer selected).
        let mut zip = zip::ZipArchive::new(Cursor::new(std::fs::read(&report.files[1]).unwrap())).unwrap();
        let mut xml = String::new();
        zip.by_name("stack.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert_eq!(xml.matches("<layer ").count(), 1, "{xml}");
        assert!(xml.contains("name=\"Hidden: notes\"") && xml.contains("visibility=\"hidden\""), "{xml}");
        let mut merged = Vec::new();
        zip.by_name("mergedimage.png").unwrap().read_to_end(&mut merged).unwrap();
        assert_eq!(decode_image(&merged, None, 16384).unwrap().pixels, PixelBuf::new(64, 64));
        // PSD: the one layer, hidden.
        let psd = read_psd(&std::fs::read(&report.files[2]).unwrap()).unwrap();
        assert_eq!(psd.layers.len(), 1);
        assert!(psd.layers[0].hidden);
        assert_eq!(psd.layers[0].unicode_name, "Hidden: notes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn flattened_of_a_selection_is_the_selected_visible_layers() {
        let (mut doc, _) = layered_doc();
        let [_, paint, hidden, top] = layer_ids(&doc);
        // Give the multiply layer something to multiply.
        paint_block(&mut doc, top, IRect::new(0, 0, 32, 64), [128, 255, 64, 255]);
        doc.recomposite_all();
        let snapshot = doc.snapshot();
        let set = &snapshot.sets[0];
        let dir = temp_dir("export-sel-flat");
        let opts = ExportOptions { layers: Some(vec![paint, hidden, top]), ..every_format(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(
            file_names(&report.files),
            ["model.png", "model_L01_paint_Paint 1.png", "model_L02_paint_Hidden_ notes.png", "model_L03_paint_Gl\u{fc}hen.png", "model.ora", "model.psd"]
        );
        let expected = flatten_layers(&[set.layers[1].clone(), set.layers[3].clone()], 64, 64, false);
        let flat = decode(&report.files[0]);
        assert_eq!(flat, expected);
        assert_ne!(&flat, doc.composite(TextureSetId(0)).unwrap(), "the base is not part of the selection");
        // The selected hidden layer does not change the flattened picture.
        let without_hidden = ExportOptions { layers: Some(vec![paint, top]), flat: Some(FlatFormat::Png(PngLevel::Balanced)), ..options(&dir.join("b")) };
        assert_eq!(decode(&run(&snapshot, &without_hidden, &NoProgress).unwrap().files[0]), expected);
        // Transparent where only the base would show.
        assert_eq!(flat.get(50, 50)[3], 0);
        // ORA and PSD: exactly the selected layers, and their merged image is the same picture.
        let mut zip = zip::ZipArchive::new(Cursor::new(std::fs::read(&report.files[4]).unwrap())).unwrap();
        let mut xml = String::new();
        zip.by_name("stack.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert_eq!(xml.matches("<layer ").count(), 3, "{xml}");
        assert!(!xml.contains("name=\"Base\""), "{xml}");
        let mut merged = Vec::new();
        zip.by_name("mergedimage.png").unwrap().read_to_end(&mut merged).unwrap();
        assert_eq!(decode_image(&merged, None, 16384).unwrap().pixels, expected);
        let psd = read_psd(&std::fs::read(&report.files[5]).unwrap()).unwrap();
        let names: Vec<&str> = psd.layers.iter().map(|l| l.unicode_name.as_str()).collect();
        assert_eq!(names, ["Paint 1", "Hidden: notes", "Gl\u{fc}hen"]);
        assert_eq!(psd.merged.data.len(), expected.data.len());
        for (got, want) in psd.merged.data.as_chunks::<4>().0.iter().zip(expected.data.as_chunks::<4>().0) {
            assert_eq!(got[3], want[3]);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_or_stale_selection_is_nothing_to_export() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-sel-empty");
        let dir = root.join("out");
        for package in [ExportPackage::Files, ExportPackage::Zip] {
            for layers in [Vec::new(), vec![LayerId(9999)]] {
                let opts = ExportOptions { layers: Some(layers.clone()), package, ..every_format(&dir) };
                assert!(plan(&snapshot, &opts).is_empty(), "{package:?} {layers:?}");
                assert!(plan_contents(&snapshot, &opts).is_empty());
                assert!(matches!(run(&snapshot, &opts, &NoProgress), Err(ExportError::NothingToExport)));
                assert!(!dir.exists());
            }
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_selection_leaves_out_sets_without_selected_layers() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let glaze = snapshot.sets[0].layers[2].id;
        let opts = ExportOptions { layers: Some(vec![glaze]), ..every_format(Path::new("out")) };
        let (body, head) = (&snapshot.sets[0].name, &snapshot.sets[1].name);
        // Only the first set is left; the model has two sets, so its files keep the `_<set>` part (the names the full
        // export gives them).
        assert_eq!(
            file_names(&plan(&snapshot, &opts)),
            [
                format!("model_{body}.png"),
                format!("model_{body}_L02_paint_Glaze.png"),
                format!("model_{body}.ora"),
                format!("model_{body}.psd")
            ]
        );
        assert!(plan_contents(&snapshot, &opts).iter().all(|f| f.set == TextureSetId(0)));
        // One layer of each set: both sets, with the set names.
        let head_paint = snapshot.sets[1].layers[1].id;
        let both = ExportOptions { layers: Some(vec![glaze, head_paint]), per_layer_png: true, ..options(Path::new("out")) };
        assert_eq!(
            file_names(&plan(&snapshot, &both)),
            [format!("model_{body}_L02_paint_Glaze.png"), format!("model_{head}_L01_paint_Paint 1.png")]
        );
    }

    /// check_exp_1 D4: two sets' layers with the same index and name, each exported alone (with or without the set
    /// filter the dialog opened from a layer row sets), get different file names (each the name the full export gives
    /// that layer), so the second export does not replace the first. A set filter of one set without a layer
    /// selection still drops the set name, as before.
    #[test]
    fn a_selection_on_a_multi_set_model_keeps_the_set_name() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let (body_paint, head_paint) = (snapshot.sets[0].layers[1].id, snapshot.sets[1].layers[1].id);
        assert_eq!(snapshot.sets[0].layers[1].name, snapshot.sets[1].layers[1].name, "the same layer name in both sets");
        let one = |id| ExportOptions { layers: Some(vec![id]), per_layer_png: true, ..options(Path::new("out")) };
        let (a, b) = (file_names(&plan(&snapshot, &one(body_paint))), file_names(&plan(&snapshot, &one(head_paint))));
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_ne!(a, b, "two sets' layers never share a file name");
        let full = ExportOptions { per_layer_png: true, include_hidden: true, ..options(Path::new("out")) };
        let full = file_names(&plan(&snapshot, &full));
        assert!(full.contains(&a[0]) && full.contains(&b[0]), "{a:?} {b:?} are the full export's names: {full:?}");
        // The Export dialog opened from a layer row also sets the set filter to that layer's set: same names.
        let filtered = ExportOptions { sets: Some(vec![TextureSetId(1)]), ..one(head_paint) };
        assert_eq!(file_names(&plan(&snapshot, &filtered)), b);
        // The zip of a selection: entries named like the loose files of the full export too.
        let zip = ExportOptions { package: ExportPackage::Zip, ..one(head_paint) };
        assert_eq!(plan_contents(&snapshot, &zip).into_iter().map(|f| f.name).collect::<Vec<_>>(), b);
        // A set filter of one set (no layer selection): no set name, unchanged.
        let only_head = ExportOptions { sets: Some(vec![TextureSetId(1)]), per_layer_png: true, ..options(Path::new("out")) };
        let names = file_names(&plan(&snapshot, &only_head));
        let head = format!("_{}", snapshot.sets[1].name);
        assert!(!names.is_empty() && names.iter().all(|n| !n.contains(&head)), "set filter: no set name: {names:?}");
    }

    /// The entries of a zip file.
    fn zip_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        read_package(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn zip_entries_are_the_loose_files() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-zip-entries");
        let (loose, packed) = (root.join("loose"), root.join("packed"));
        let tga = ExportOptions { flat: Some(FlatFormat::Tga), ..every_format(&loose) };
        let loose_report = run(&snapshot, &tga, &NoProgress).unwrap();
        let zip_opts = ExportOptions { dir: packed.clone(), package: ExportPackage::Zip, ..tga.clone() };
        let report = run(&snapshot, &zip_opts, &NoProgress).unwrap();
        let zip_path = packed.join("model.zip");
        assert_eq!(report.files, std::slice::from_ref(&zip_path));
        assert_eq!(plan(&snapshot, &zip_opts), std::slice::from_ref(&zip_path));
        assert_eq!(dir_listing(&packed), ["model.zip"], "no stage folder, no temp file");
        assert_eq!(report.bytes_written, std::fs::metadata(&zip_path).unwrap().len());
        let entries = zip_entries(&zip_path);
        assert_eq!(report.packaged, entries.len());
        assert_eq!(entries.len(), 11);
        let names: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, file_names(&loose_report.files), "plan order");
        assert_eq!(names, plan_contents(&snapshot, &zip_opts).into_iter().map(|f| f.name).collect::<Vec<_>>());
        for (name, bytes) in &entries {
            assert!(*bytes == std::fs::read(loose.join(name)).unwrap(), "{name} differs from the loose file");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn zip_export_is_deterministic() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-zip-determinism");
        let zip_in = |name: &str, lanes: usize| {
            let opts = ExportOptions { package: ExportPackage::Zip, ..every_format(&root.join(name)) };
            run_in_lanes(&snapshot, &opts, &NoProgress, lanes).unwrap();
            std::fs::read(root.join(name).join("model.zip")).unwrap()
        };
        let (a, b, one_lane) = (zip_in("a", usize::MAX), zip_in("b", usize::MAX), zip_in("c", 1));
        assert!(a == b, "two exports differ");
        assert!(a == one_lane, "one lane differs from several");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn zip_methods_and_timestamps() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let dir = temp_dir("export-zip-methods");
        for (format, method) in [(FlatFormat::Png(PngLevel::Balanced), zip::CompressionMethod::Stored), (FlatFormat::Tga, zip::CompressionMethod::Deflated)] {
            let opts = ExportOptions { flat: Some(format), package: ExportPackage::Zip, ..every_format(&dir) };
            run(&snapshot, &opts, &NoProgress).unwrap();
            let mut zip = zip::ZipArchive::new(Cursor::new(std::fs::read(dir.join("model.zip")).unwrap())).unwrap();
            for i in 0..zip.len() {
                let entry = zip.by_index(i).unwrap();
                let name = entry.name().unwrap().into_owned();
                let want = match name.rsplit_once('.').unwrap().1 {
                    "png" | "ora" => zip::CompressionMethod::Stored,
                    "psd" => zip::CompressionMethod::Deflated,
                    _ => method,
                };
                assert_eq!(entry.compression(), want, "{name}");
                let t = entry.last_modified().unwrap();
                assert_eq!((t.year(), t.month(), t.day(), t.hour(), t.minute(), t.second()), (1980, 1, 1, 0, 0, 0), "{name}");
                assert!(!entry.is_dir() && !name.contains('/'));
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zip_of_a_selection() {
        let (doc, _) = layered_doc();
        let snapshot = doc.snapshot();
        let [_, paint, hidden, _] = layer_ids(&doc);
        let root = temp_dir("export-zip-selection");
        let loose = ExportOptions { layers: Some(vec![hidden, paint]), ..every_format(&root.join("loose")) };
        run(&snapshot, &loose, &NoProgress).unwrap();
        let packed = ExportOptions { dir: root.join("packed"), package: ExportPackage::Zip, ..loose.clone() };
        let report = run(&snapshot, &packed, &NoProgress).unwrap();
        let entries = zip_entries(&report.files[0]);
        assert_eq!(entries, dir_bytes_in_order(&root.join("loose"), &plan_contents(&snapshot, &loose)));
        assert_eq!(
            entries.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["model.png", "model_L01_paint_Paint 1.png", "model_L02_paint_Hidden_ notes.png", "model.ora", "model.psd"]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The files of a folder in the order of a plan.
    fn dir_bytes_in_order(dir: &Path, contents: &[PlannedFile]) -> Vec<(String, Vec<u8>)> {
        contents.iter().map(|f| (f.name.clone(), std::fs::read(dir.join(&f.name)).unwrap())).collect()
    }

    /// Cancels at the `after`-th report of `stage` (counting reports with a fraction above 0).
    struct CancelAtStage {
        stage: &'static str,
        after: usize,
        seen: AtomicUsize,
    }

    impl Progress for CancelAtStage {
        fn report(&self, fraction: f32, stage: &str) {
            if fraction > 0.0 && stage == self.stage {
                self.seen.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn cancelled(&self) -> bool {
            self.seen.load(Ordering::SeqCst) >= self.after
        }
    }

    #[test]
    fn a_cancelled_or_failed_zip_export_leaves_nothing() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-zip-cancel");
        let dir = root.join("out");
        let opts = ExportOptions { package: ExportPackage::Zip, ..every_format(&dir) };
        // A previous package that must survive every failed attempt unchanged.
        std::fs::create_dir_all(&dir).unwrap();
        let old = b"the previous package".to_vec();
        std::fs::write(dir.join("model.zip"), &old).unwrap();
        let tmp_zip = temp_path_for(&dir.join("model.zip"));
        let stage = dir.join(format!("{}.d", tmp_zip.file_name().unwrap().to_string_lossy()));
        let only_the_old_zip = |what: &str| {
            assert_eq!(dir_listing(&dir), ["model.zip"], "{what}");
            assert!(std::fs::read(dir.join("model.zip")).unwrap() == old, "{what}: the old package changed");
        };
        for lanes in [1, usize::MAX] {
            // Cancelled while staging, after 3 files.
            let progress = CancelAtStage { stage: "Exporting", after: 3, seen: AtomicUsize::new(0) };
            assert!(matches!(run_in_lanes(&snapshot, &opts, &progress, lanes), Err(ExportError::Cancelled)));
            only_the_old_zip("cancel while staging");
            // Cancelled while packing, after 2 entries.
            let progress = CancelAtStage { stage: "Packing", after: 2, seen: AtomicUsize::new(0) };
            assert!(matches!(run_in_lanes(&snapshot, &opts, &progress, lanes), Err(ExportError::Cancelled)));
            only_the_old_zip("cancel while packing");
        }
        // The stage folder cannot be created (a file is in the way): nothing is written.
        std::fs::write(&stage, b"in the way").unwrap();
        assert!(matches!(run(&snapshot, &opts, &NoProgress), Err(ExportError::Io(_))));
        std::fs::remove_file(&stage).unwrap();
        only_the_old_zip("stage blocked");
        // The temp zip cannot be created (a folder is in the way): the staged files are removed again.
        std::fs::create_dir_all(tmp_zip.join("in the way")).unwrap();
        assert!(matches!(run(&snapshot, &opts, &NoProgress), Err(ExportError::Io(_))));
        assert!(!stage.exists(), "the stage folder is removed");
        std::fs::remove_dir_all(&tmp_zip).unwrap();
        only_the_old_zip("temp zip blocked");
        // A panic while staging also removes the stage folder.
        let progress = PanicAfter { after: 3, files: AtomicUsize::new(0) };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&snapshot, &opts, &progress)));
        assert!(result.is_err());
        only_the_old_zip("panic");
        // A cancelled export into a folder that did not exist removes that folder again.
        let fresh = root.join("fresh");
        let progress = CancelAtStage { stage: "Exporting", after: 1, seen: AtomicUsize::new(0) };
        assert!(matches!(run(&snapshot, &ExportOptions { dir: fresh.clone(), ..opts.clone() }, &progress), Err(ExportError::Cancelled)));
        assert!(!fresh.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn zip_replaces_an_existing_package_atomically() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-zip-replace");
        let dir = root.join("out");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.zip"), b"old").unwrap();
        let opts = ExportOptions { package: ExportPackage::Zip, ..every_format(&dir) };
        let report = run(&snapshot, &opts, &NoProgress).unwrap();
        let fresh = ExportOptions { dir: root.join("fresh"), ..opts.clone() };
        run(&snapshot, &fresh, &NoProgress).unwrap();
        assert!(std::fs::read(&report.files[0]).unwrap() == std::fs::read(root.join("fresh").join("model.zip")).unwrap());
        assert_eq!(dir_listing(&dir), ["model.zip"]);
        std::fs::remove_dir_all(&root).unwrap();
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

    // ---- fix_export (stable gate critic): names, linked layers, the selection zip, stale temps, non-ASCII ----

    /// Item 4: a selection uses the names of the full export, `_2` included, for every kind of collision the
    /// deduper resolves: letter case (`Body` / `body`), sanitizing (`Mat:1` / `Mat_1`) and long names that share
    /// their first 64 characters.
    #[test]
    fn a_selection_has_the_full_exports_names_when_set_names_collide() {
        let long_a = format!("{}_alpha", "x".repeat(64));
        let long_b = format!("{}_beta", "x".repeat(64));
        for (a, b) in [("Body", "body"), ("Mat:1", "Mat_1"), (long_a.as_str(), long_b.as_str())] {
            let mut doc = painted_two_set_doc();
            doc.sets[0].name = a.to_owned();
            doc.sets[1].name = b.to_owned();
            let snapshot = doc.snapshot();
            for format in [Some(FlatFormat::Png(PngLevel::Balanced)), Some(FlatFormat::Tga), None] {
                let full = ExportOptions { flat: format, per_layer_png: true, ..every_format(Path::new("out")) };
                let full_names = file_names(&plan(&snapshot, &full));
                let unique: std::collections::BTreeSet<String> = full_names.iter().map(|n| n.to_lowercase()).collect();
                assert_eq!(unique.len(), full_names.len(), "{a} / {b}: {full_names:?}");
                assert!(full_names.iter().any(|n| n.contains("_2")), "{a} / {b}: the names collide: {full_names:?}");
                for set in 0..2 {
                    for layer in snapshot.sets[set].layers.iter().map(|l| l.id) {
                        let one = ExportOptions { layers: Some(vec![layer]), ..full.clone() };
                        for name in file_names(&plan(&snapshot, &one)) {
                            assert!(full_names.contains(&name), "{a} / {b}: {name} is not a name of the full export {full_names:?}");
                        }
                        // The per-layer file of that layer is exactly the full export's file of that layer.
                        let contents = plan_contents(&snapshot, &one);
                        let full_contents = plan_contents(&snapshot, &full);
                        for file in contents.iter().filter(|f| matches!(f.kind, PlannedKind::Layer { .. })) {
                            let same = full_contents.iter().find(|f| f.kind == file.kind && f.set == file.set).unwrap();
                            assert_eq!(file.name, same.name, "{a} / {b}");
                        }
                    }
                }
                // Fewer formats or no hidden layers never rename the other files.
                let fewer = ExportOptions { ora: false, psd: false, include_hidden: false, ..full.clone() };
                for name in file_names(&plan(&snapshot, &fewer)) {
                    assert!(full_names.contains(&name), "{a} / {b}: {name}");
                }
            }
        }
    }

    /// Item 4 on disk: the second set's layer exported alone is byte-identical to the full export's file of the same
    /// name (the `_2` one), so two exports into one folder pair the right files.
    #[test]
    fn a_selection_of_a_colliding_set_writes_the_full_exports_file() {
        let mut doc = painted_two_set_doc();
        doc.sets[0].name = "Body".to_owned();
        doc.sets[1].name = "body".to_owned();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-collide-selection");
        let full = ExportOptions { per_layer_png: true, ..options(&root.join("full")) };
        run(&snapshot, &full, &NoProgress).unwrap();
        let second = snapshot.sets[1].layers[1].id;
        let one = ExportOptions { dir: root.join("one"), layers: Some(vec![second]), ..full.clone() };
        let report = run(&snapshot, &one, &NoProgress).unwrap();
        let names = file_names(&report.files);
        assert_eq!(names, ["model_body_L01_paint_Paint 1_2.png"]);
        assert!(std::fs::read(&report.files[0]).unwrap() == std::fs::read(root.join("full").join(&names[0])).unwrap());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A snapshot of the two-set document with one stamp whose design spans both sets: one stamp layer per set, linked
    /// by the same `StampId` (as `Document::stamp_layers` reports them), with different pixels.
    fn snapshot_with_a_two_set_stamp() -> (DocumentSnapshot, [LayerId; 2]) {
        let doc = painted_two_set_doc();
        let mut snapshot = doc.snapshot();
        let stamp = StampId(7_000);
        let mut ids = [LayerId(0); 2];
        for (set, color) in [(0usize, [250u8, 10, 10, 255]), (1, [10u8, 250, 10, 255])] {
            let rect = IRect::new(4, 4, 12, 10);
            let data: Vec<u8> = std::iter::repeat_n(color, rect.area() as usize).flatten().collect();
            let cache = std::sync::Arc::new(crate::pixel::SparseBuf::from_parts(vec![CroppedBuf { rect, data }]));
            let id = LayerId(7_001 + set as u64);
            snapshot.sets[set].layers.push(Layer::new(id, "Logo", LayerKind::Stamp { stamp, cache }));
            ids[set] = id;
        }
        (snapshot, ids)
    }

    /// Item 3: "Export this layer" on one layer of a design that spans two sets writes that design's layer of every
    /// set it touches (the flattened texture of both sets too), and the selection zip holds both.
    #[test]
    fn a_design_across_two_sets_exports_its_layer_of_every_set() {
        let (snapshot, [in_body, in_head]) = snapshot_with_a_two_set_stamp();
        assert_eq!(linked_selection(&snapshot, &[in_head]), [in_head, in_body]);
        assert_eq!(linked_selection(&snapshot, &[in_body, in_head, in_body]), [in_body, in_head]);
        let paint = snapshot.sets[0].layers[1].id;
        assert_eq!(linked_selection(&snapshot, &[paint]), [paint], "a paint layer has no linked layers");
        let root = temp_dir("export-linked");
        for clicked in [in_body, in_head] {
            let one = ExportOptions { layers: Some(vec![clicked]), per_layer_png: true, ..options(&root.join("one")) };
            let contents = plan_contents(&snapshot, &one);
            let layers: Vec<LayerId> = contents
                .iter()
                .filter_map(|f| match f.kind {
                    PlannedKind::Layer { layer, .. } => Some(layer),
                    _ => None,
                })
                .collect();
            assert_eq!(layers, [in_body, in_head], "both sets' layers of the design, in set order");
            let full = ExportOptions { per_layer_png: true, ..options(&root.join("full")) };
            let full_names: Vec<String> = plan_contents(&snapshot, &full).into_iter().map(|f| f.name).collect();
            assert!(contents.iter().all(|f| full_names.contains(&f.name)));
            let report = run(&snapshot, &one, &NoProgress).unwrap();
            assert_eq!(report.files.len(), 2);
            for (path, set) in report.files.iter().zip([0usize, 1]) {
                let image = decode(path);
                let want = encode_full_canvas(snapshot.sets[set].layers.last().unwrap(), image.width, image.height).unwrap();
                assert!(std::fs::read(path).unwrap() == want, "{}", path.display());
            }
            std::fs::remove_dir_all(root.join("one")).unwrap();
        }
        // The flattened texture under that selection: both sets get one.
        let flat = ExportOptions { layers: Some(vec![in_head]), flat: Some(FlatFormat::Png(PngLevel::Fast)), ..options(&root) };
        assert_eq!(plan(&snapshot, &flat).len(), 2);
        // Zipped: one `_selection` zip with both files.
        let zip = ExportOptions { layers: Some(vec![in_head]), per_layer_png: true, package: ExportPackage::Zip, ..options(&root) };
        let report = run(&snapshot, &zip, &NoProgress).unwrap();
        assert_eq!(file_names(&report.files), ["model_selection.zip"]);
        assert_eq!(zip_entries(&report.files[0]).len(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Item 2: a zip of a selection is `<base>_selection.zip`; the full package `<base>.zip` next to it stays as it
    /// was, and a Visible / All zip is still `<base>.zip`.
    #[test]
    fn a_selection_zip_never_replaces_the_full_package() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-selection-zip-name");
        let full = ExportOptions { package: ExportPackage::Zip, ..every_format(&root) };
        assert_eq!(package_name_for(&full), "model.zip");
        let full_report = run(&snapshot, &full, &NoProgress).unwrap();
        let before = std::fs::read(&full_report.files[0]).unwrap();
        let paint = snapshot.sets[1].layers[1].id;
        let one = ExportOptions { layers: Some(vec![paint]), ..full.clone() };
        assert_eq!(package_name_for(&one), "model_selection.zip");
        assert_eq!(plan(&snapshot, &one), [root.join("model_selection.zip")]);
        run(&snapshot, &one, &NoProgress).unwrap();
        assert_eq!(dir_listing(&root), ["model.zip", "model_selection.zip"]);
        assert!(std::fs::read(root.join("model.zip")).unwrap() == before, "the full package is unchanged");
        assert_eq!(selection_package_name("a:b"), "a_b_selection.zip");
        let all = ExportOptions { include_hidden: true, ..full.clone() };
        assert_eq!(package_name_for(&all), "model.zip");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Item 7: temp files and stage folders of an export whose process is gone are removed; those of a running
    /// process, of this one, and anything that is not an export temp stay.
    #[test]
    fn stale_temps_of_processes_that_are_not_running_are_removed() {
        let root = temp_dir("export-stale-temps");
        let own = std::process::id();
        let dead = own.wrapping_add(1_000_003);
        let alive = own.wrapping_add(2_000_003);
        let stage = root.join(format!("model.zip.tmp-{dead}.d"));
        std::fs::create_dir_all(stage.join("inner")).unwrap();
        std::fs::write(stage.join("inner").join("model.png"), b"x").unwrap();
        for name in [
            format!("model.zip.tmp-{dead}"),
            format!("model_L01_paint_Paint 1.png.tmp-{dead}"),
            format!("model.psd.tmp-{dead}"),
            format!("model.zip.tmp-{alive}"),
            format!("model.png.tmp-{own}"),
            format!("notes.txt.tmp-{dead}"),
            format!("model.png.tmp-{dead}x"),
            "model.zip".to_owned(),
            "model.png".to_owned(),
        ] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        // A stage-like folder of a running process, and a file named like a stage folder.
        std::fs::create_dir(root.join(format!("model.zip.tmp-{alive}.d"))).unwrap();
        std::fs::write(root.join(format!("other.zip.tmp-{dead}.d")), b"x").unwrap();
        let removed = remove_stale_temps(&root, &|pid| pid == alive);
        assert_eq!(
            file_names(&removed),
            [format!("model.psd.tmp-{dead}"), format!("model.zip.tmp-{dead}"), format!("model.zip.tmp-{dead}.d"), format!("model_L01_paint_Paint 1.png.tmp-{dead}")]
        );
        let mut left = dir_listing(&root);
        left.sort();
        let mut want = vec![
            format!("model.png.tmp-{dead}x"),
            format!("model.png.tmp-{own}"),
            format!("model.zip.tmp-{alive}"),
            format!("model.zip.tmp-{alive}.d"),
            format!("notes.txt.tmp-{dead}"),
            format!("other.zip.tmp-{dead}.d"),
            "model.png".to_owned(),
            "model.zip".to_owned(),
        ];
        want.sort();
        assert_eq!(left, want);
        assert!(remove_stale_temps(&root.join("missing"), &|_| false).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Item 9 (L1): a folder that cannot take the export fails before anything is written: a planned file that is a
    /// folder, a folder path that is a file. Files and Zip alike; the old files stay as they were.
    #[test]
    fn a_blocked_export_folder_fails_before_writing_anything() {
        let doc = painted_two_set_doc();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-blocked");
        let dir = root.join("out");
        let opts = every_format(&dir);
        let planned = plan(&snapshot, &opts);
        std::fs::create_dir_all(&dir).unwrap();
        // An old export's file, and a folder in the way of the last planned file.
        std::fs::write(&planned[0], b"old").unwrap();
        std::fs::create_dir(planned.last().unwrap()).unwrap();
        let result = run(&snapshot, &opts, &NoProgress);
        assert!(matches!(&result, Err(ExportError::Io(e)) if e.to_string().contains("is a folder")), "{:?}", result.map(|r| r.files));
        assert_eq!(std::fs::read(&planned[0]).unwrap(), b"old", "the old file was not replaced");
        let mut want = vec![file_names(&planned[..1])[0].clone(), file_names(&planned[planned.len() - 1..])[0].clone()];
        want.sort();
        assert_eq!(dir_listing(&dir), want, "nothing else was written, no temp file");
        // A zip whose name is taken by a folder: no stage folder, no temp zip.
        let zip = ExportOptions { package: ExportPackage::Zip, ..opts.clone() };
        std::fs::create_dir(dir.join("model.zip")).unwrap();
        assert!(run(&snapshot, &zip, &NoProgress).is_err());
        assert!(dir_listing(&dir).iter().all(|n| !n.contains(".tmp-")));
        // The folder path is a file.
        let file = root.join("a file");
        std::fs::write(&file, b"x").unwrap();
        for package in [ExportPackage::Files, ExportPackage::Zip] {
            let blocked = ExportOptions { dir: file.join("out"), package, ..opts.clone() };
            assert!(matches!(run(&snapshot, &blocked, &NoProgress), Err(ExportError::Io(_))));
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Item 9 (L2): non-ASCII model, set and layer names go into a zip under their names (UTF-8) and read back
    /// byte-identical to the loose files.
    #[test]
    fn non_ascii_names_round_trip_through_a_zip() {
        let mut doc = painted_two_set_doc();
        doc.sets[0].name = "\u{8eab}\u{4f53}".to_owned();
        doc.sets[1].name = "T\u{ea}te".to_owned();
        let layer = doc.sets[0].layers[1].id;
        doc.rename_layer(layer, "Gl\u{fc}hen \u{2728} \u{30ed}\u{30b4}").unwrap();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-non-ascii");
        let base = "\u{30e2}\u{30c7}\u{30eb}".to_owned();
        let loose = ExportOptions { base_name: base.clone(), ..every_format(&root.join("loose")) };
        let loose_report = run(&snapshot, &loose, &NoProgress).unwrap();
        let zip = ExportOptions { dir: root.join("zip"), package: ExportPackage::Zip, ..loose.clone() };
        let report = run(&snapshot, &zip, &NoProgress).unwrap();
        assert_eq!(file_names(&report.files), [format!("{base}.zip")]);
        let entries = zip_entries(&report.files[0]);
        let names: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, file_names(&loose_report.files));
        assert!(names.iter().any(|n| n.contains("\u{8eab}\u{4f53}") && n.contains("\u{2728}")), "{names:?}");
        for (name, bytes) in &entries {
            assert!(*bytes == std::fs::read(root.join("loose").join(name)).unwrap(), "{name}");
        }
        // The UTF-8 flag is set on the non-ASCII entries (bit 11), so other tools read the same names.
        let mut archive = zip::ZipArchive::new(Cursor::new(std::fs::read(&report.files[0]).unwrap())).unwrap();
        let raw = archive.by_index(1).unwrap().name_raw().to_vec();
        assert_eq!(String::from_utf8(raw).unwrap(), names[1]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Item 9 (M4): a 4096 export of two sets with four paint layers each and every format, as one zip. Records the
    /// time (stderr); peak memory is read by `tools/stable/peak_mem.py` around this test. Needs about 2 GB.
    #[test]
    #[ignore = "large: run with --ignored export_at_4096 (and tools/stable/peak_mem.py for the peak memory)"]
    fn export_at_4096_with_several_sets_and_layers() {
        let body = noise_texture(4096, 4096, 31);
        let head = noise_texture(4096, 4096, 32);
        let mut doc = crate::project::save::testkit::doc_from_bundle(&crate::project::save::testkit::obj_bundle(
            "big",
            &crate::samples::two_material_model(),
            &[("body.png", &body), ("head.png", &head)],
        ));
        for set in 0..2u32 {
            for k in 0..3 {
                let id = doc.add_raster_layer(TextureSetId(set), None).unwrap();
                let o = 200 * k;
                paint_block(&mut doc, id, IRect::new(o, o, 3000 + o, 2500 + o), [40 + 60 * k as u8, 90, 200 - 50 * k as u8, 180]);
            }
        }
        doc.recomposite_all();
        let snapshot = doc.snapshot();
        let root = temp_dir("export-4096");
        for package in [ExportPackage::Files, ExportPackage::Zip] {
            let opts = ExportOptions { base_name: "big".to_owned(), package, ..every_format(&root.join(format!("{package:?}"))) };
            let t = std::time::Instant::now();
            let report = run(&snapshot, &opts, &NoProgress).unwrap();
            eprintln!(
                "export_at_4096 {package:?}: {} file(s), {} packaged, {} bytes in {:.2} s",
                report.files.len(),
                report.packaged,
                report.bytes_written,
                t.elapsed().as_secs_f64()
            );
            assert_eq!(report.files.len().max(report.packaged), 2 * (1 + 4 + 2));
        }
        let entries = zip_entries(&root.join("Zip").join("big.zip"));
        for (name, bytes) in &entries {
            assert!(*bytes == std::fs::read(root.join("Files").join(name)).unwrap(), "{name}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
