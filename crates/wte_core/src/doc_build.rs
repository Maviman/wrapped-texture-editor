// Owner: core-document (C2). Document construction (ARCHITECTURE.md 5.15): from a fresh import, from already-built
// parts (project load, tests) and the built-in samples.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use rayon::prelude::*;

use crate::blob::BlobStore;
use crate::bvh::Bvh;
use crate::composite::flatten_layers;
use crate::document::Document;
use crate::error::DocError;
use crate::history::History;
use crate::ids::{LayerId, TextureSetId};
use crate::image_io::{PngLevel, encode_png};
use crate::import::{ImportOptions, ImportedModel, ModelFormat, SourceBundle, SourceFile, import_from_bundle, normalize_ref};
use crate::layer::{Layer, LayerKind, TextureSet};
use crate::math::DirtyRegion;
use crate::pixel::PixelBuf;
use crate::pixel_ops::resize;
use crate::progress::{NoProgress, Progress};
use crate::samples;
use crate::stamp::{StampObject, StampSource};
use crate::texel_map::TexelMap;
use crate::text::FontCache;

/// Largest texture side the app can upload (D10).
pub const MAX_TEXTURE_SIDE: u32 = 8192;
/// Side of a blank texture created when a material has none.
pub const DEFAULT_BLANK_SIDE: u32 = 1024;

/// The built-in sample models.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SampleKind {
    Sphere,
    Cube,
    TwoMaterial,
    /// Objects-tabs plan 2.4: `samples::object_trio` (objects "Floor", "Cube", "Ball").
    Objects,
}

/// Where the base texture of a set comes from.
#[derive(Clone, Debug)]
pub enum TextureChoice {
    /// The material's texture; falls back to Blank(1024, white) if missing.
    Detected,
    /// User-chosen image, already decoded (<= MAX_TEXTURE_SIDE). `original` = the chosen file's bytes (key = file name);
    /// from_import stores it in `SourceBundle::extras` so the project stays self-contained (R7). None only in tests.
    Image { pixels: PixelBuf, original: Option<SourceFile> },
    Blank { width: u32, height: u32, fill: [u8; 4] },
}

/// Already-built layers of one set, input of `Document::from_parts`.
pub struct SetParts {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub active: Option<LayerId>,
}

/// Everything `Document::from_parts` assembles a document from. `sets.len() == model.mesh.sets.len()`.
pub struct DocumentParts {
    pub name: String,
    pub model: ImportedModel,
    pub sets: Vec<SetParts>,
    pub stamps: Vec<StampObject>,
    pub blobs: BlobStore,
    pub ui_state: serde_json::Value,
    pub next_id: u64,
}

/// Maps the 0..=1 progress of a sub-operation into `lo..=hi` of the caller's operation.
struct ScaledProgress<'a> {
    inner: &'a dyn Progress,
    lo: f32,
    hi: f32,
}

impl Progress for ScaledProgress<'_> {
    fn report(&self, fraction: f32, stage: &str) {
        let f = fraction.clamp(0.0, 1.0);
        // Exact at the end, whatever the rounding of the interpolation.
        let mapped = if f >= 1.0 { self.hi } else { self.lo + (self.hi - self.lo) * f };
        self.inner.report(mapped, stage);
    }

    fn cancelled(&self) -> bool {
        self.inner.cancelled()
    }
}

fn bad(message: String) -> DocError {
    DocError::BadSetup(message)
}

/// An image no larger than `MAX_TEXTURE_SIDE` on either side (aspect-preserving downscale otherwise).
fn fit_texture(pixels: PixelBuf) -> PixelBuf {
    let longest = pixels.width.max(pixels.height);
    if longest <= MAX_TEXTURE_SIDE {
        return pixels;
    }
    let scale = MAX_TEXTURE_SIDE as f64 / longest as f64;
    let side = |v: u32| ((v as f64 * scale).round() as u32).clamp(1, MAX_TEXTURE_SIDE);
    resize(&pixels, side(pixels.width), side(pixels.height))
}

/// Checks everything `from_parts` relies on and the structure invariants of section 6.5, so that bad input is an
/// error instead of a debug assertion. `parts.stamps` must already be sorted by id.
fn validate_parts(parts: &DocumentParts) -> Result<(), DocError> {
    let set_count = parts.model.mesh.sets.len();
    if set_count == 0 {
        return Err(DocError::NoTextureSets);
    }
    if parts.sets.len() != set_count {
        return Err(bad(format!("{} texture sets given, the model has {set_count}", parts.sets.len())));
    }
    if let Some(pair) = parts.stamps.windows(2).find(|w| w[0].id == w[1].id) {
        return Err(bad(format!("duplicate stamp id {}", pair[0].id.0)));
    }
    let stamp_is_text: HashMap<_, bool> =
        parts.stamps.iter().map(|s| (s.id, matches!(s.source, StampSource::Text(_)))).collect();

    let mut layer_ids = BTreeSet::new();
    // (stamp, set index) pairs of the linked layers.
    let mut links = BTreeSet::new();
    for (index, set) in parts.sets.iter().enumerate() {
        let name = &set.name;
        if set.width == 0 || set.height == 0 || set.width > MAX_TEXTURE_SIDE || set.height > MAX_TEXTURE_SIDE {
            return Err(bad(format!(
                "texture set \"{name}\" is {}x{}; a side must be 1..={MAX_TEXTURE_SIDE}",
                set.width, set.height
            )));
        }
        for layer in &set.layers {
            if !layer_ids.insert(layer.id) {
                return Err(bad(format!("duplicate layer id {}", layer.id.0)));
            }
            let (stamp, wants_text) = match &layer.kind {
                LayerKind::Raster(buf) => {
                    let bytes = set.width as usize * set.height as usize * 4;
                    let unallocated = buf.width == 0 && buf.height == 0 && buf.data.is_empty();
                    if !unallocated && (buf.width != set.width || buf.height != set.height || buf.data.len() != bytes) {
                        return Err(bad(format!(
                            "layer \"{}\" is {}x{}, texture set \"{name}\" is {}x{}",
                            layer.name, buf.width, buf.height, set.width, set.height
                        )));
                    }
                    continue;
                }
                LayerKind::Stamp { stamp, cache } | LayerKind::Text { stamp, cache } => {
                    if cache.parts().iter().any(|p| p.data.len() as u64 != p.rect.area() * 4) {
                        return Err(bad(format!("layer \"{}\" has an inconsistent pixel cache", layer.name)));
                    }
                    (*stamp, matches!(layer.kind, LayerKind::Text { .. }))
                }
            };
            if stamp_is_text.get(&stamp) != Some(&wants_text) {
                return Err(bad(format!("layer \"{}\" is not linked to a matching stamp", layer.name)));
            }
            if !links.insert((stamp, index)) {
                return Err(bad(format!("stamp {} has two layers in texture set \"{name}\"", stamp.0)));
            }
        }
        if let Some(active) = set.active
            && !set.layers.iter().any(|l| l.id == active)
        {
            return Err(bad(format!("the active layer of texture set \"{name}\" does not exist")));
        }
    }
    for stamp in &parts.stamps {
        if !links.contains(&(stamp.id, stamp.home_set.index())) {
            return Err(bad(format!("stamp \"{}\" has no layer in its home texture set", stamp.name)));
        }
    }
    Ok(())
}

impl Document {
    /// One TextureChoice per set (choices.len() == model.mesh.sets.len()). Each set gets two raster layers: "Base" (the
    /// imported / chosen / blank texture, `locked: true`) and above it an empty transparent "Paint 1", which is the active
    /// layer. So the first stroke or erase never damages the original texture (unlock Base to edit it directly).
    /// An `Image { original: Some(f), .. }` choice for set n appends `f` to `bundle.extras` with
    /// `key = format!("textures/set{n:02}/{}", normalize_ref(&f.original_name))`. Err(NoTextureSets) if the mesh has no set.
    ///
    /// The document is named after the model. Layer ids start at 1 (Base and Paint 1 of set 0 are 1 and 2). An image
    /// larger than `MAX_TEXTURE_SIDE` is scaled down; an empty image or blank size is `Err(BadSetup)`.
    pub fn from_import(model: ImportedModel, choices: Vec<TextureChoice>, progress: &dyn Progress) -> Result<Document, DocError> {
        let mut model = model;
        let set_count = model.mesh.sets.len();
        if set_count == 0 {
            return Err(DocError::NoTextureSets);
        }
        if choices.len() != set_count {
            return Err(bad(format!("{} texture choices given, the model has {set_count} texture sets", choices.len())));
        }
        progress.report(0.0, "Preparing textures");

        let mut next_id = 1u64;
        let mut sets = Vec::with_capacity(set_count);
        for (n, choice) in choices.into_iter().enumerate() {
            if progress.cancelled() {
                return Err(DocError::Cancelled);
            }
            let info = &model.mesh.sets[n];
            let base = match choice {
                TextureChoice::Detected => {
                    // Sets are keyed by texture (6.1 step 5), so no other set needs this image: take it.
                    let detected = info
                        .texture
                        .and_then(|t| model.textures.get_mut(t as usize))
                        .and_then(|t| t.image.take())
                        .filter(|img| img.width > 0 && img.height > 0 && img.data.len() == img.width as usize * img.height as usize * 4);
                    match detected {
                        Some(image) => fit_texture(image),
                        None => PixelBuf::filled(DEFAULT_BLANK_SIDE, DEFAULT_BLANK_SIDE, [255; 4]),
                    }
                }
                TextureChoice::Image { pixels, original } => {
                    if pixels.width == 0 || pixels.height == 0 || pixels.data.len() != pixels.width as usize * pixels.height as usize * 4 {
                        return Err(bad(format!("the image chosen for texture set \"{}\" is empty or malformed", info.name)));
                    }
                    if let Some(file) = original {
                        let key = format!("textures/set{n:02}/{}", normalize_ref(&file.original_name));
                        model.bundle.extras.push(SourceFile { key, original_name: file.original_name, bytes: file.bytes });
                    }
                    fit_texture(pixels)
                }
                TextureChoice::Blank { width, height, fill } => {
                    if width == 0 || height == 0 || width > MAX_TEXTURE_SIDE || height > MAX_TEXTURE_SIDE {
                        return Err(bad(format!(
                            "blank texture for set \"{}\" is {width}x{height}; a side must be 1..={MAX_TEXTURE_SIDE}",
                            info.name
                        )));
                    }
                    PixelBuf::filled(width, height, fill)
                }
            };
            let (width, height) = (base.width, base.height);
            let (base_id, paint_id) = (LayerId(next_id), LayerId(next_id + 1));
            next_id += 2;
            let mut base = Layer::new(base_id, "Base", LayerKind::Raster(Arc::new(base)));
            base.locked = true;
            // Unallocated until the first stroke (an 8192 set saves 256 MB per untouched paint layer).
            let paint = Layer::new(paint_id, "Paint 1", LayerKind::empty_raster());
            sets.push(SetParts { name: info.name.clone(), width, height, layers: vec![base, paint], active: Some(paint_id) });
            progress.report(0.2 * (n + 1) as f32 / set_count as f32, "Preparing textures");
        }

        let parts = DocumentParts {
            name: model.name.clone(),
            model,
            sets,
            stamps: Vec::new(),
            blobs: BlobStore::default(),
            ui_state: serde_json::Value::Null,
            next_id,
        };
        Document::from_parts(parts, &ScaledProgress { inner: progress, lo: 0.2, hi: 1.0 })
    }

    /// Assembles a document from already-built layers (project load, tests): builds the BVH and texel maps (rayon over
    /// sets), validates sizes, assigns fresh layer versions, composites everything, clears history. Does not bake stamps
    /// and does not add or lock any layer.
    ///
    /// Input that breaks the structure invariants of section 6.5 (duplicate ids, a dangling active layer, a stamp
    /// without a layer in its home set, ...) is `Err(BadSetup)`. Stamps are sorted by id; `next_id` is raised above
    /// every id in use. The new document is unmodified (`revision == 0`, saved).
    pub fn from_parts(parts: DocumentParts, progress: &dyn Progress) -> Result<Document, DocError> {
        let mut parts = parts;
        parts.stamps.sort_by_key(|s| s.id);
        validate_parts(&parts)?;
        if progress.cancelled() {
            return Err(DocError::Cancelled);
        }
        let DocumentParts { name, model, sets: set_parts, stamps, blobs, ui_state, next_id } = parts;
        let ImportedModel { mesh, warnings, bundle, options, .. } = model;

        progress.report(0.0, "Analysing the model");
        let sizes: Vec<(u32, u32)> = set_parts.iter().map(|s| (s.width, s.height)).collect();
        let (bvh, texel_maps) = rayon::join(
            || Bvh::build(&mesh),
            || {
                sizes
                    .par_iter()
                    .enumerate()
                    .map(|(i, &(w, h))| {
                        let gutter = TexelMap::default_gutter(w, h);
                        let cancelled = || progress.cancelled();
                        TexelMap::build_cancellable(&mesh, TextureSetId(i as u32), w, h, gutter, &cancelled)
                    })
                    .collect::<Option<Vec<TexelMap>>>()
            },
        );
        let Some(texel_maps) = texel_maps else {
            return Err(DocError::Cancelled);
        };
        if progress.cancelled() {
            return Err(DocError::Cancelled);
        }

        let set_count = set_parts.len();
        let mut version_counter = 0u64;
        let mut max_id = stamps.iter().map(|s| s.id.0).max().unwrap_or(0);
        let mut sets = Vec::with_capacity(set_count);
        for (i, (part, texel_map)) in set_parts.into_iter().zip(texel_maps).enumerate() {
            progress.report(0.6 + 0.4 * i as f32 / set_count as f32, "Compositing layers");
            if progress.cancelled() {
                return Err(DocError::Cancelled);
            }
            let SetParts { name, width, height, mut layers, active } = part;
            for layer in &mut layers {
                version_counter += 1;
                layer.version = version_counter;
                max_id = max_id.max(layer.id.0);
                // Content bound of a paint layer (one scan; compositing skips the empty rest from now on).
                if let LayerKind::Raster(buf) = &layer.kind {
                    let bound = crate::pixel_ops::nonzero_bounds_par(buf).unwrap_or(crate::math::IRect::EMPTY);
                    layer.content = Some((version_counter, bound));
                }
            }
            let composite = flatten_layers(&layers, width, height, false);
            sets.push(TextureSet {
                id: TextureSetId(i as u32),
                name,
                width,
                height,
                layers,
                active,
                texel_map: Arc::new(texel_map),
                composite,
                pending: DirtyRegion::default(),
                gpu_dirty: DirtyRegion::default(),
                scratch: None,
            });
        }

        let object_count = mesh.objects.len();
        let document = Document {
            name,
            mesh: Arc::new(mesh),
            bvh: Arc::new(bvh),
            sets,
            stamps,
            blobs,
            source: Arc::new(bundle),
            import_options: options,
            import_warnings: warnings,
            history: History::default(),
            ui_state,
            next_id: next_id.max(max_id.saturating_add(1)),
            revision: 0,
            saved_revision: 0,
            stroke: None,
            pending_edit: None,
            preview_baked: BTreeSet::new(),
            bakes: Box::default(),
            defer_preview_bakes: false,
            defer_final_bakes: false,
            png_reuse: None,
            version_counter,
            fonts: FontCache::default(),
            image_mips: HashMap::new(),
            text_mips: HashMap::new(),
            text_caches: Box::default(),
            hidden_objects: vec![false; object_count],
            visibility_generation: 0,
            sticker_state: Box::default(),
        };
        document.debug_check_structure();
        progress.report(1.0, "Compositing layers");
        Ok(document)
    }

    /// Built-in sample: samples::* -> to_obj -> in-memory SourceBundle (OBJ + MTL + checker PNGs encoded with encode_png)
    /// -> import_from_bundle(.., decode_textures = true, ..) -> from_import(all Detected). Needs core-import at run time.
    ///
    /// Sphere: `uv_sphere(64, 32, 1.0)` named "sphere"; Cube: `seam_cube(1.5)` named "cube"; TwoMaterial:
    /// `two_material_model()` named "two_materials" with the sets "body_checker" and "head_checker"; Objects:
    /// `object_trio()` named "objects", written with `to_obj_objects` (so the OBJ importer reads its three objects
    /// back), texture "objects_checker.png". Every texture is a 1024 x 1024 checker.
    pub fn sample(kind: SampleKind) -> Document {
        const SIDE: u32 = 1024;
        let mut objects = None;
        let (stem, raw, textures) = match kind {
            SampleKind::Sphere => ("sphere", samples::uv_sphere(64, 32, 1.0), vec![("sphere_checker.png", 16)]),
            SampleKind::Cube => ("cube", samples::seam_cube(1.5), vec![("cube_checker.png", 12)]),
            SampleKind::TwoMaterial => {
                ("two_materials", samples::two_material_model(), vec![("body_checker.png", 12), ("head_checker.png", 16)])
            }
            SampleKind::Objects => {
                let (raw, trio) = samples::object_trio();
                objects = Some(trio);
                ("objects", raw, vec![("objects_checker.png", 16)])
            }
        };
        let file = |name: &str, bytes: Vec<u8>| SourceFile {
            key: normalize_ref(name),
            original_name: name.to_owned(),
            bytes: Arc::from(bytes),
        };
        let mtl_name = format!("{stem}.mtl");
        let texture_names: Vec<String> = textures.iter().map(|(name, _)| (*name).to_owned()).collect();
        let (obj, mtl) = match &objects {
            Some(objects) => samples::to_obj_objects(&raw, objects, &mtl_name, &texture_names),
            None => samples::to_obj(&raw, &mtl_name, &texture_names),
        };

        let mut sidecars = vec![file(&mtl_name, mtl.into_bytes())];
        for &(name, cells) in &textures {
            let image = samples::checker_texture(SIDE, SIDE, cells);
            let png = encode_png(image.width, image.height, &image.data, PngLevel::Fast)
                .expect("the sample checker texture is a valid RGBA8 image");
            sidecars.push(file(name, png));
        }
        let bundle = SourceBundle {
            format: ModelFormat::Obj,
            main: file(&format!("{stem}.obj"), obj.into_bytes()),
            sidecars,
            extras: Vec::new(),
            original_path: None,
        };
        let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress)
            .expect("the built-in sample is a valid OBJ model");
        let choices = vec![TextureChoice::Detected; model.mesh.sets.len()];
        Document::from_import(model, choices, &NoProgress).expect("the built-in sample has UV-mapped geometry")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use super::*;
    use crate::history::testkit::{doc_state, doc_with_stamp, raster_arc};
    use crate::math::IRect;
    use crate::mesh::RawMesh;
    use crate::pixel::{CroppedBuf, SparseBuf};
    use crate::testutil::imported_from_raw;

    fn assert_send<T: Send>() {}

    /// Records the reported fractions; cancels after `cancel_after` reports (0 = at once, u32::MAX = never).
    struct Recorder {
        reports: std::sync::Mutex<Vec<f32>>,
        seen: AtomicU32,
        cancel_after: u32,
        asked: AtomicBool,
    }

    impl Recorder {
        fn new(cancel_after: u32) -> Recorder {
            Recorder { reports: Default::default(), seen: AtomicU32::new(0), cancel_after, asked: AtomicBool::new(false) }
        }
    }

    impl Progress for Recorder {
        fn report(&self, fraction: f32, _stage: &str) {
            self.reports.lock().unwrap().push(fraction);
            self.seen.fetch_add(1, Ordering::Relaxed);
        }

        fn cancelled(&self) -> bool {
            self.asked.store(true, Ordering::Relaxed);
            self.seen.load(Ordering::Relaxed) >= self.cancel_after
        }
    }

    fn gradient(w: u32, h: u32) -> PixelBuf {
        let mut buf = PixelBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // Includes fully transparent texels with colour.
                buf.set(x, y, [x as u8, y as u8, (x ^ y) as u8, if (x + y) % 5 == 0 { 0 } else { 255 }]);
            }
        }
        buf
    }

    fn check_base_and_paint(doc: &Document, set: usize, base: &PixelBuf) {
        let s = &doc.sets[set];
        assert_eq!((s.width, s.height), (base.width, base.height));
        assert_eq!(s.id, TextureSetId(set as u32));
        assert_eq!(s.layers.len(), 2, "two layers per set");
        let (b, p) = (&s.layers[0], &s.layers[1]);
        assert_eq!((b.name.as_str(), p.name.as_str()), ("Base", "Paint 1"));
        assert!(b.locked && !p.locked, "Base is locked, Paint 1 is not");
        assert!(b.is_raster() && p.is_raster());
        assert!(b.visible && p.visible && b.opacity == 1.0 && p.opacity == 1.0);
        assert_eq!(**raster_arc(doc, b.id), *base, "Base holds the chosen pixels");
        let paint = raster_arc(doc, p.id);
        assert_eq!((paint.width, paint.height), (0, 0), "Paint 1 is empty and unallocated");
        assert!(paint.data.is_empty());
        assert_eq!(s.active, Some(p.id), "Paint 1 is active");
        assert_eq!(doc.paint_target(s.id), crate::doc_layers::PaintTarget::Ready(p.id));
        // export(import(x)) == x: the untouched composite is the texture, hidden colour included.
        assert_eq!(s.composite(), base);
        assert_eq!((s.texel_map.width, s.texel_map.height), (base.width, base.height));
        assert_eq!(s.texel_map.gutter, TexelMap::default_gutter(base.width, base.height));
    }

    #[test]
    fn document_is_send() {
        assert_send::<Document>();
        assert_send::<DocumentParts>();
        assert_send::<TextureChoice>();
    }

    #[test]
    fn from_import_detected_image_and_blank() {
        // Two sets; the model carries a texture for set 0 only.
        let detected = gradient(48, 40);
        let model = imported_from_raw(samples::two_material_model(), vec![Some(detected.clone()), None]);
        assert_eq!(model.mesh.sets.len(), 2);
        let names: Vec<String> = model.mesh.sets.iter().map(|s| s.name.clone()).collect();
        let progress = Recorder::new(u32::MAX);
        let doc = Document::from_import(model, vec![TextureChoice::Detected, TextureChoice::Detected], &progress).unwrap();
        check_base_and_paint(&doc, 0, &detected);
        // A missing texture falls back to a blank white 1024 x 1024 one.
        check_base_and_paint(&doc, 1, &PixelBuf::filled(DEFAULT_BLANK_SIDE, DEFAULT_BLANK_SIDE, [255; 4]));
        assert_eq!(doc.name, "dummy");
        assert_eq!(doc.sets.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), names);
        let ids: Vec<u64> = doc.sets.iter().flat_map(|s| &s.layers).map(|l| l.id.0).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        assert_eq!(doc.next_id, 5);
        assert!(doc.stamps.is_empty() && doc.blobs.is_empty() && doc.source.extras.is_empty());
        assert!(!doc.is_modified() && !doc.can_undo() && !doc.has_pending_composite() && !doc.has_pending_edit());
        assert_eq!(doc.import_options, ImportOptions { generate_fallback_uvs: false, max_texture_dim: 8192 });
        assert_eq!(&*doc.source.main.bytes, b"dummy");
        assert!(doc.raycast(&crate::math::Ray::new(glam::Vec3::new(0.0, 0.0, 5.0), glam::Vec3::NEG_Z)).is_some(), "the BVH is built");
        // Progress is monotone and ends at 1.
        let reports = progress.reports.lock().unwrap().clone();
        assert!(reports.windows(2).all(|w| w[0] <= w[1]), "{reports:?}");
        assert_eq!((reports.first(), reports.last()), (Some(&0.0), Some(&1.0)));

        // Image and Blank choices.
        let chosen = gradient(64, 32);
        let model = imported_from_raw(samples::two_material_model(), vec![Some(detected.clone()), None]);
        let choices = vec![
            TextureChoice::Image { pixels: chosen.clone(), original: None },
            TextureChoice::Blank { width: 16, height: 128, fill: [10, 20, 30, 0] },
        ];
        let doc = Document::from_import(model, choices, &NoProgress).unwrap();
        check_base_and_paint(&doc, 0, &chosen);
        check_base_and_paint(&doc, 1, &PixelBuf::filled(16, 128, [10, 20, 30, 0]));
        assert!(doc.source.extras.is_empty());
    }

    #[test]
    fn from_import_stores_the_chosen_file_in_the_bundle() {
        let model = imported_from_raw(samples::two_material_model(), Vec::new());
        let file = |name: &str, byte: u8| SourceFile { key: name.to_owned(), original_name: name.to_owned(), bytes: Arc::from(vec![byte; 9]) };
        let choices = vec![
            TextureChoice::Image { pixels: gradient(8, 8), original: Some(file("My Skin.PNG", 1)) },
            TextureChoice::Image { pixels: gradient(8, 8), original: Some(file("head.tga", 2)) },
        ];
        let doc = Document::from_import(model, choices, &NoProgress).unwrap();
        let extras = &doc.source.extras;
        assert_eq!(extras.len(), 2);
        assert_eq!(extras[0].key, "textures/set00/my skin.png");
        assert_eq!(extras[0].original_name, "My Skin.PNG");
        assert_eq!(&*extras[0].bytes, &[1u8; 9]);
        assert_eq!(extras[1].key, "textures/set01/head.tga");
        assert_eq!(&*extras[1].bytes, &[2u8; 9]);
        assert!(doc.source.sidecars.is_empty());
    }

    #[test]
    fn from_import_rejects_bad_setups() {
        let model = || imported_from_raw(samples::flat_quad(), Vec::new());
        // The flat quad references no texture: Detected gives the blank fallback.
        let doc = Document::from_import(model(), vec![TextureChoice::Detected], &NoProgress).unwrap();
        check_base_and_paint(&doc, 0, &PixelBuf::filled(1024, 1024, [255; 4]));

        let err = |choices: Vec<TextureChoice>| Document::from_import(model(), choices, &NoProgress).err().expect("must fail");
        assert!(matches!(err(vec![]), DocError::BadSetup(_)));
        assert!(matches!(err(vec![TextureChoice::Detected, TextureChoice::Detected]), DocError::BadSetup(_)));
        assert!(matches!(err(vec![TextureChoice::Blank { width: 0, height: 16, fill: [0; 4] }]), DocError::BadSetup(_)));
        assert!(matches!(err(vec![TextureChoice::Blank { width: 16, height: 8193, fill: [0; 4] }]), DocError::BadSetup(_)));
        assert!(matches!(err(vec![TextureChoice::Image { pixels: PixelBuf::new(0, 0), original: None }]), DocError::BadSetup(_)));
        let malformed = PixelBuf { width: 4, height: 4, data: vec![0; 5] };
        assert!(matches!(err(vec![TextureChoice::Image { pixels: malformed, original: None }]), DocError::BadSetup(_)));

        // No UV-mapped geometry: no texture set, no document.
        let no_uv = imported_from_raw(samples::no_uv_tetra(), Vec::new());
        assert!(no_uv.mesh.sets.is_empty());
        assert!(matches!(Document::from_import(no_uv, vec![], &NoProgress), Err(DocError::NoTextureSets)));
        let empty = imported_from_raw(RawMesh::default(), Vec::new());
        assert!(matches!(Document::from_import(empty, vec![], &NoProgress), Err(DocError::NoTextureSets)));

        // Cancelled before and during the build.
        for after in [0, 1, 2, 3] {
            let progress = Recorder::new(after);
            let result = Document::from_import(model(), vec![TextureChoice::Blank { width: 64, height: 64, fill: [0; 4] }], &progress);
            assert!(matches!(result, Err(DocError::Cancelled)), "cancel after {after} reports");
            assert!(progress.asked.load(Ordering::Relaxed));
        }
    }

    #[test]
    fn oversized_textures_are_scaled_down() {
        // 8200 x 4: only the long side is over the limit.
        let big = PixelBuf::filled(8200, 4, [9, 8, 7, 255]);
        let model = imported_from_raw(samples::flat_quad(), Vec::new());
        let doc = Document::from_import(model, vec![TextureChoice::Image { pixels: big, original: None }], &NoProgress).unwrap();
        assert_eq!((doc.sets[0].width, doc.sets[0].height), (MAX_TEXTURE_SIDE, 4));
        assert!(raster_arc(&doc, LayerId(1)).data.as_chunks::<4>().0.iter().all(|p| *p == [9, 8, 7, 255]));
        assert_eq!(fit_texture(PixelBuf::new(16384, 1)).width, 8192);
        assert_eq!(fit_texture(PixelBuf::new(16384, 1)).height, 1);
        let same = PixelBuf::new(8192, 2);
        assert_eq!(fit_texture(same.clone()), same);
    }

    fn one_set_parts(layers: Vec<Layer>, active: Option<LayerId>, size: u32) -> DocumentParts {
        DocumentParts {
            name: "parts".to_owned(),
            model: imported_from_raw(samples::flat_quad(), Vec::new()),
            sets: vec![SetParts { name: "Set".to_owned(), width: size, height: size, layers, active }],
            stamps: Vec::new(),
            blobs: BlobStore::default(),
            ui_state: serde_json::json!({ "zoom": 2 }),
            next_id: 1,
        }
    }

    fn raster(id: u64, w: u32, h: u32) -> Layer {
        Layer::new(LayerId(id), &format!("L{id}"), LayerKind::Raster(Arc::new(PixelBuf::filled(w, h, [id as u8, 0, 0, 255]))))
    }

    #[test]
    fn from_parts_adds_and_locks_nothing() {
        let mut hidden = raster(7, 32, 32);
        hidden.visible = false;
        hidden.opacity = 0.25;
        hidden.alpha_lock = true;
        let doc = Document::from_parts(one_set_parts(vec![raster(4, 32, 32), hidden], None, 32), &NoProgress).unwrap();
        let set = &doc.sets[0];
        assert_eq!(set.name, "Set");
        assert_eq!(set.layers.iter().map(|l| (l.id.0, l.name.clone())).collect::<Vec<_>>(), [(4, "L4".to_owned()), (7, "L7".to_owned())]);
        assert!(set.layers.iter().all(|l| !l.locked), "nothing is locked");
        assert_eq!(set.active, None, "no layer is made active");
        assert!(!set.layers[1].visible && set.layers[1].opacity == 0.25 && set.layers[1].alpha_lock);
        assert_eq!(doc.name, "parts");
        assert_eq!(doc.ui_state, serde_json::json!({ "zoom": 2 }));
        // Fresh, distinct, non-zero versions.
        let versions: Vec<u64> = set.layers.iter().map(|l| l.version()).collect();
        assert!(versions[0] != versions[1] && versions.iter().all(|&v| v != 0), "{versions:?}");
        // next_id is raised above every id in use.
        assert_eq!(doc.next_id, 8);
        // Composited (the hidden layer does not show), nothing pending, clean history, unmodified.
        assert_eq!(*set.composite(), PixelBuf::filled(32, 32, [4, 0, 0, 255]));
        assert!(!doc.has_pending_composite() && !doc.can_undo() && !doc.can_redo() && !doc.is_modified());
        assert_eq!((doc.history.undo_len(), doc.revision()), (0, 0));

        // An empty layer list is a valid (transparent) set.
        let doc = Document::from_parts(one_set_parts(Vec::new(), None, 16), &NoProgress).unwrap();
        assert!(doc.sets[0].layers.is_empty());
        assert_eq!(*doc.sets[0].composite(), PixelBuf::new(16, 16));
        assert_eq!(doc.next_id, 1);
    }

    #[test]
    fn from_parts_validates_its_input() {
        let bad = |parts: DocumentParts| match Document::from_parts(parts, &NoProgress) {
            Err(DocError::BadSetup(message)) => message,
            Err(other) => panic!("expected BadSetup, got {other}"),
            Ok(_) => panic!("expected BadSetup, got a document"),
        };
        // Size mismatch between a raster layer and its set.
        let message = bad(one_set_parts(vec![raster(1, 32, 16)], None, 32));
        assert!(message.contains("32x16") && message.contains("32x32"), "{message}");
        bad(one_set_parts(vec![raster(1, 64, 64)], None, 32));
        let mut broken = raster(1, 32, 32);
        broken.kind = LayerKind::Raster(Arc::new(PixelBuf { width: 32, height: 32, data: vec![0; 7] }));
        bad(one_set_parts(vec![broken], None, 32));
        // Set sizes.
        bad(one_set_parts(Vec::new(), None, 0));
        bad(one_set_parts(Vec::new(), None, MAX_TEXTURE_SIDE + 1));
        // Set count.
        let mut parts = one_set_parts(Vec::new(), None, 16);
        parts.sets.clear();
        bad(parts);
        let mut parts = one_set_parts(Vec::new(), None, 16);
        parts.sets.push(SetParts { name: "extra".to_owned(), width: 16, height: 16, layers: Vec::new(), active: None });
        bad(parts);
        // Structure invariants.
        bad(one_set_parts(vec![raster(1, 16, 16), raster(1, 16, 16)], None, 16));
        bad(one_set_parts(vec![raster(1, 16, 16)], Some(LayerId(2)), 16));
        let stamp_layer = |id: u64, stamp: u64| {
            Layer::new(LayerId(id), "decal", LayerKind::Stamp { stamp: crate::ids::StampId(stamp), cache: Arc::new(SparseBuf::empty()) })
        };
        bad(one_set_parts(vec![stamp_layer(1, 50)], None, 16));
        let mut inconsistent = stamp_layer(1, 50);
        inconsistent.kind = LayerKind::Stamp {
            stamp: crate::ids::StampId(50),
            cache: Arc::new(SparseBuf::from_parts(vec![CroppedBuf { rect: IRect::new(0, 0, 4, 4), data: vec![0; 8] }])),
        };
        bad(one_set_parts(vec![inconsistent], None, 16));

        // No texture set at all.
        let mut parts = one_set_parts(Vec::new(), None, 16);
        parts.model = imported_from_raw(samples::no_uv_tetra(), Vec::new());
        parts.sets.clear();
        assert!(matches!(Document::from_parts(parts, &NoProgress), Err(DocError::NoTextureSets)));
        // Cancelled.
        let parts = one_set_parts(vec![raster(1, 16, 16)], None, 16);
        assert!(matches!(Document::from_parts(parts, &Recorder::new(0)), Err(DocError::Cancelled)));
    }

    #[test]
    fn from_parts_checks_stamp_links() {
        // The fixture itself is valid and keeps its stamp unbaked (caches as given).
        let mut doc = doc_with_stamp();
        assert_eq!(doc.stamps.len(), 1);
        assert_eq!(doc.next_id, 200);
        assert_eq!(doc.layer(LayerId(101)).unwrap().content_rect(), IRect::new(10, 12, 50, 40));
        doc_state(&mut doc);

        let stamp = |id: u64, home: u32| -> StampObject {
            let mut s = doc.stamps[0].clone();
            s.id = crate::ids::StampId(id);
            s.home_set = TextureSetId(home);
            s
        };
        let layer = |id: u64, stamp: u64| {
            Layer::new(LayerId(id), "decal", LayerKind::Stamp { stamp: crate::ids::StampId(stamp), cache: Arc::new(SparseBuf::empty()) })
        };
        let build = |layers: Vec<Layer>, stamps: Vec<StampObject>| {
            let mut parts = one_set_parts(layers, None, 16);
            parts.stamps = stamps;
            Document::from_parts(parts, &NoProgress)
        };
        // Valid: unsorted stamps are sorted; next_id covers stamp ids too.
        let doc2 = build(vec![layer(1, 60), layer(2, 50)], vec![stamp(60, 0), stamp(50, 0)]).unwrap();
        assert_eq!(doc2.stamps.iter().map(|s| s.id.0).collect::<Vec<_>>(), [50, 60]);
        assert_eq!(doc2.next_id, 61);
        // A stamp without a layer in its home set, a home set that does not exist, two layers in one set,
        // duplicate stamp ids, and a text layer linked to an image stamp.
        assert!(matches!(build(vec![], vec![stamp(50, 0)]), Err(DocError::BadSetup(_))));
        assert!(matches!(build(vec![layer(1, 50)], vec![stamp(50, 3)]), Err(DocError::BadSetup(_))));
        assert!(matches!(build(vec![layer(1, 50), layer(2, 50)], vec![stamp(50, 0)]), Err(DocError::BadSetup(_))));
        assert!(matches!(build(vec![layer(1, 50)], vec![stamp(50, 0), stamp(50, 0)]), Err(DocError::BadSetup(_))));
        let mut text = layer(1, 50);
        text.kind = LayerKind::Text { stamp: crate::ids::StampId(50), cache: Arc::new(SparseBuf::empty()) };
        assert!(matches!(build(vec![text], vec![stamp(50, 0)]), Err(DocError::BadSetup(_))));
    }

    /// Objects-tabs WP1 (plan 2.4): the "Three Objects" sample goes through the OBJ importer and keeps its objects.
    #[test]
    fn the_objects_sample_has_floor_cube_and_ball() {
        let doc = Document::sample(SampleKind::Objects);
        let names: Vec<&str> = doc.mesh.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, crate::samples::OBJECT_TRIO_NAMES);
        assert_eq!(doc.mesh.objects.iter().map(|o| o.tri_count).sum::<u32>(), doc.mesh.tri_count());
        assert_eq!(doc.sets.len(), 1);
        assert_eq!(doc.sets[0].name, "objects_checker");
        // The cube stands in the middle of the floor; the ball beside it.
        let (floor, cube, ball) = (doc.mesh.objects[0].bounds, doc.mesh.objects[1].bounds, doc.mesh.objects[2].bounds);
        assert!(floor.min.x < cube.min.x && cube.max.x < ball.min.x && ball.max.x < floor.max.x);
        assert_eq!((cube.min.y, cube.max.y), (0.0, 1.0));
        // The other samples are one object named after the model.
        let sphere = Document::sample(SampleKind::Sphere);
        assert_eq!(sphere.mesh.objects.len(), 1);
        assert_eq!(sphere.mesh.objects[0].name, "sphere");
    }
}
