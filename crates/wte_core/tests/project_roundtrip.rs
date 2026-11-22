// Owner: core-integration (C7). Project round-trip tests of ARCHITECTURE.md 9.1 ("integration", project_roundtrip.rs):
// raster, stamp and text layers through save -> load (stamp parameters, rotated UV stamps stored verbatim, pixels per
// layer), a byte-identical manifest on re-save, and the degraded loads of 7.1 that need stamps and text: a corrupt
// stamp cache (re-baked), a deleted image blob (layers survive as raster), a missing font blob (bundled fallback), and
// a project saved right after a `Commit::Preview` edit.
//
// Round 4 (package T5, FEATURE_TEXT 4.3 and 8.1 "Project"): format version 2 for the text effects (`TextFx`): the two
// frozen fixtures, round trips of every effect, `min_reader_version` from `required_reader`, unknown and broken effect
// entries from a file, and the save check of every text float.
//
// Only the public API is used. Temporary files go through `testutil::temp_dir` (under target\ on E:).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use wte_core::glam::{Vec2, Vec3};
use wte_core::import::{import_from_bundle, normalize_ref};
use wte_core::testutil::temp_dir;
use wte_core::text::FontCatalog;
use wte_core::*;
use wte_core::{project, samples};

// ------------------------------------------------------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------------------------------------------------------

fn fnv(bytes: &[u8]) -> u64 {
    fnv1a64(0xcbf2_9ce4_8422_2325, bytes)
}

#[derive(Clone, PartialEq, Debug)]
struct LayerState {
    id: LayerId,
    name: String,
    visible: bool,
    opacity_bits: u32,
    blend: BlendMode,
    locked: bool,
    alpha_lock: bool,
    kind: &'static str,
    stamp: Option<StampId>,
    rect: IRect,
    pixels: u64,
}

#[derive(Clone, PartialEq, Debug)]
struct SetState {
    name: String,
    size: [u32; 2],
    active: Option<LayerId>,
    layers: Vec<LayerState>,
    composite: u64,
}

/// Everything a project file must reproduce, pixels as hashes.
#[derive(Clone, PartialEq, Debug)]
struct DocState {
    name: String,
    sets: Vec<SetState>,
    stamps: Vec<StampObject>,
}

fn layer_state(l: &Layer, w: u32, h: u32) -> LayerState {
    let dense = l.to_dense(w, h);
    LayerState {
        id: l.id,
        name: l.name.clone(),
        visible: l.visible,
        opacity_bits: l.opacity.to_bits(),
        blend: l.blend,
        locked: l.locked,
        alpha_lock: l.alpha_lock,
        kind: l.kind_name(),
        stamp: l.stamp_id(),
        rect: if l.is_raster() { IRect::EMPTY } else { l.content_rect() },
        pixels: fnv(&dense.data),
    }
}

fn doc_state(doc: &mut Document) -> DocState {
    doc.recomposite_all();
    let sets = doc
        .set_ids()
        .into_iter()
        .map(|id| {
            let set = doc.set(id).unwrap();
            assert!(*set.composite() == doc.flatten(id, false).unwrap(), "set {id:?}: stale composite");
            SetState {
                name: set.name.clone(),
                size: [set.width, set.height],
                active: set.active,
                layers: set.layers.iter().map(|l| layer_state(l, set.width, set.height)).collect(),
                composite: testutil::hash_pixels(set.composite()),
            }
        })
        .collect();
    DocState { name: doc.name.clone(), sets, stamps: doc.stamps.clone() }
}

fn png_bytes(image: &PixelBuf) -> Arc<[u8]> {
    Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
}

fn dense(layer: &Layer, width: u32, height: u32) -> PixelBuf {
    layer.to_dense(width, height)
}

/// A re-importable document: OBJ + MTL (+ checker PNGs of `side` px for the textured materials) imported from memory.
/// Sets without a texture start blank at `side` px.
fn importable_doc(raw: RawMesh, stem: &str, side: u32) -> Document {
    let textures = raw.materials.iter().filter_map(|m| m.texture).max().map_or(0, |m| m as usize + 1);
    let names: Vec<String> = (0..textures).map(|i| format!("{stem}_tex{i}.png")).collect();
    let (obj, mtl) = samples::to_obj(&raw, &format!("{stem}.mtl"), &names);
    let file = |name: &str, bytes: Vec<u8>| SourceFile {
        key: normalize_ref(name),
        original_name: name.to_owned(),
        bytes: Arc::from(bytes),
    };
    let mut sidecars = vec![file(&format!("{stem}.mtl"), mtl.into_bytes())];
    for (i, name) in names.iter().enumerate() {
        let image = samples::checker_texture(side, side, 8 + 4 * i as u32);
        sidecars.push(file(name, encode_png(side, side, &image.data, PngLevel::Fast).unwrap()));
    }
    let bundle = SourceBundle {
        format: ModelFormat::Obj,
        main: file(&format!("{stem}.obj"), obj.into_bytes()),
        sidecars,
        extras: Vec::new(),
        original_path: None,
    };
    let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
    let choices = model
        .mesh
        .sets
        .iter()
        .map(|s| match s.texture {
            Some(_) => TextureChoice::Detected,
            None => TextureChoice::Blank { width: side, height: side, fill: [255, 255, 255, 255] },
        })
        .collect();
    Document::from_import(model, choices, &NoProgress).unwrap()
}

fn text_spec(font: FontRef, content: &str) -> TextSpec {
    TextSpec {
        content: content.to_owned(),
        font,
        size_px: TEXT_PLACEMENT_PX,
        color: [200, 30, 30, 255],
        align: TextAlign::Left,
        line_spacing: 1.1,
        letter_spacing: 0.05,
        outline: Some(Outline { width_em: 0.05, color: [0, 0, 0, 255] }),
        fx: TextFx::default(),
    }
}

fn ray_hit(doc: &Document, origin: Vec3, target: Vec3) -> Hit {
    doc.raycast(&Ray::new(origin, target - origin)).expect("the ray hits the model")
}

fn layer_of(doc: &Document, set: TextureSetId, name: &str) -> LayerId {
    doc.set(set).unwrap().layers.iter().find(|l| l.name == name).unwrap().id
}

fn save_options() -> SaveOptions {
    SaveOptions {
        png: PngLevel::Fast,
        write_flattened: true,
        keep_backup: false,
        now_unix: 1_650_000_000,
        app_version: "integration".to_owned(),
    }
}

fn read_zip_entry(path: &Path, name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(std::fs::read(path).unwrap())).unwrap();
    let mut entry = zip.by_name(name).unwrap();
    let mut out = Vec::new();
    entry.read_to_end(&mut out).unwrap();
    out
}

/// Copies a project archive entry by entry; `edit` may replace an entry's bytes or drop it (`None`).
fn rewrite_project(src: &Path, dst: &Path, mut edit: impl FnMut(&str, Vec<u8>) -> Option<Vec<u8>>) {
    let mut zip = zip::ZipArchive::new(Cursor::new(std::fs::read(src).unwrap())).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::DEFAULT.compression_method(zip::CompressionMethod::Stored);
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        drop(entry);
        if let Some(data) = edit(&name, data) {
            out.start_file(name.as_str(), stored).unwrap();
            out.write_all(&data).unwrap();
        }
    }
    std::fs::write(dst, out.finish().unwrap().into_inner()).unwrap();
}

/// The parsed layer entries of a saved project, with their set.
fn layer_entries(path: &Path) -> Vec<(TextureSetId, LayerEntry)> {
    let manifest = project::read_manifest(path).unwrap();
    manifest
        .sets
        .iter()
        .flat_map(|s| s.layers.iter().map(move |v| (s.id, serde_json::from_value::<LayerEntry>(v.clone()).unwrap())))
        .collect()
}

/// The content every test here starts from: a two-material model with paint (incl. RGB under alpha 0), a hidden and
/// a Multiply layer, image and text stamps in 3D and in UV space (rotated and flipped). Returns the stamp ids
/// (image 3D, image UV, text 3D, text UV with the Hack font).
fn rich_document() -> (Document, [StampId; 4]) {
    let mut doc = importable_doc(samples::two_material_model(), "robot", 256);
    let (body, head) = (TextureSetId(0), TextureSetId(1));

    // Paint across both materials, then erase part of it: erasing lowers alpha only, so RGB stays under alpha 0.
    let hits: Vec<Hit> = (0..80)
        .filter_map(|i| doc.raycast(&Ray::new(Vec3::new(-0.1, 0.2 + 0.01 * i as f32, 3.0), Vec3::NEG_Z)))
        .collect();
    doc.stroke_begin(&BrushParams { radius: 0.08, color: [10, 200, 40, 255], ..BrushParams::default() }).unwrap();
    for hit in &hits {
        doc.stroke_to_3d(hit, 1.0);
    }
    assert!(doc.stroke_end());
    doc.stroke_begin(&BrushParams { radius: 0.05, mode: BrushMode::Erase, hardness: 1.0, ..BrushParams::default() })
        .unwrap();
    for hit in &hits[..20] {
        doc.stroke_to_3d(hit, 1.0);
    }
    assert!(doc.stroke_end());
    let paint = doc.layer(layer_of(&doc, body, "Paint 1")).unwrap();
    assert!(
        paint.raster().unwrap().data.as_chunks::<4>().0.iter().any(|p| p[3] == 0 && p[..3] != [0, 0, 0]),
        "the erase must leave RGB under alpha 0"
    );
    // A 2D stroke on the head set, on a new layer with alpha lock off, then a hidden and a Multiply layer.
    let extra = doc.add_raster_layer(head, Some("Detail")).unwrap();
    doc.stroke_begin(&BrushParams { radius: 6.0, color: [250, 250, 0, 200], ..BrushParams::default() }).unwrap();
    for i in 0..20 {
        doc.stroke_to_2d(head, Vec2::new(250.0 + 2.0 * i as f32, 30.0 + i as f32), 1.0);
    }
    assert!(doc.stroke_end());
    doc.set_layer_alpha_lock(extra, true).unwrap();
    let hidden = doc.add_raster_layer(body, Some("Hidden")).unwrap();
    doc.fill(body, hidden, Vec2::new(0.5, 0.25), [0, 0, 255, 255], FillScope::Island).unwrap();
    doc.set_layer_visible(hidden, false).unwrap();
    let multiply = doc.add_raster_layer(body, Some("Shade ü")).unwrap();
    doc.fill(body, multiply, Vec2::ZERO, [128, 128, 160, 255], FillScope::Layer).unwrap();
    doc.set_layer_blend(multiply, BlendMode::Multiply).unwrap();
    doc.set_layer_opacity(multiply, 0.5, Commit::Final).unwrap();

    // Image stamp in 3D on the cube front, image stamp in UV space of the head (rotated, flipped).
    let blob = doc.add_image_blob("logo.png", png_bytes(&samples::sample_stamp_image(96))).unwrap();
    let hit = ray_hit(&doc, Vec3::new(0.2, -0.2, 3.0), Vec3::new(0.2, -0.2, 0.0));
    let p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::new(0.2, 0.15), 0.25).rotated(0.25);
    let image3d = doc.add_stamp("Logo", StampSource::Image { blob }, Placement::Projected(p), body).unwrap();
    doc.set_stamp_tint(image3d, [255, 220, 200, 230], Commit::Final).unwrap();
    let uv = UvPlacement {
        set: head,
        center_px: Vec2::new(77.25, 140.5),
        size_px: Vec2::new(90.0, 60.0),
        rotation: 0.712_345_6,
        flip_x: true,
        flip_y: false,
        clip_to_islands: true,
    };
    let image_uv = doc.add_stamp("Sticker", StampSource::Image { blob }, Placement::Uv(uv), head).unwrap();

    // Text in 3D on the head with the default font; text in UV space of the body with the bundled Hack font.
    let font = doc.default_font();
    let spec = text_spec(font, "Hello\nWorld");
    let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
    let hit = ray_hit(&doc, Vec3::new(0.0, 0.85, 3.0), Vec3::new(0.0, 0.85, 0.0));
    let p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::new(0.2, 0.2 * h as f32 / w as f32), 0.3);
    let text3d = doc.add_stamp("Greeting", StampSource::Text(spec), Placement::Projected(p), head).unwrap();

    let catalog = FontCatalog::bundled();
    let hack = catalog.find("Hack", "Regular").expect("the bundled Hack font");
    let (bytes, face) = catalog.face_bytes(hack.id).unwrap();
    let hack_ref = doc.register_font("Hack", "Regular", face, bytes);
    let spec = text_spec(hack_ref, "code 42");
    let uv = UvPlacement {
        set: body,
        center_px: Vec2::new(128.0, 40.0),
        size_px: Vec2::new(120.0, 30.0),
        rotation: -1.234_567_9,
        flip_x: false,
        flip_y: true,
        clip_to_islands: false,
    };
    let text_uv = doc.add_stamp("Code", StampSource::Text(spec), Placement::Uv(uv), body).unwrap();
    doc.rename_stamp(text_uv, "Code label").unwrap();
    doc.set_layer_opacity(doc.stamp_layers(text3d)[0].1, 0.75, Commit::Final).unwrap();

    for id in [image3d, image_uv, text3d, text_uv] {
        assert!(doc.stamp_layers(id).iter().any(|(_, l)| !doc.layer(*l).unwrap().content_rect().is_empty()), "{id:?} baked");
    }
    (doc, [image3d, image_uv, text3d, text_uv])
}

// ------------------------------------------------------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------------------------------------------------------

#[test]
fn raster_stamp_and_text_layers_round_trip() {
    let (mut doc, [_, image_uv, _, text_uv]) = rich_document();
    let expected = doc_state(&mut doc);
    let dir = temp_dir("roundtrip_rich");
    let path = dir.join("rich.wtep");
    project::save(&doc.snapshot(), &path, &save_options(), &NoProgress).unwrap();

    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(!report.force_save_as);
    let mut loaded = report.document;
    assert_eq!(doc_state(&mut loaded), expected);
    assert!(!loaded.is_modified() && !loaded.can_undo() && !loaded.can_redo());

    // Pixels per layer, byte for byte (including RGB under alpha 0 of the erased paint).
    for set in doc.set_ids() {
        let (a, b) = (doc.set(set).unwrap(), loaded.set(set).unwrap());
        for (la, lb) in a.layers.iter().zip(&b.layers) {
            assert!(la.to_dense(a.width, a.height) == lb.to_dense(b.width, b.height), "layer {}", la.name);
        }
        assert!(a.composite() == b.composite());
    }
    // Stamp parameters are equal, and the UV rotations are stored verbatim (bit for bit).
    assert_eq!(loaded.stamps, doc.stamps);
    for id in [image_uv, text_uv] {
        let (Placement::Uv(a), Placement::Uv(b)) = (doc.stamp(id).unwrap().placement, loaded.stamp(id).unwrap().placement)
        else {
            panic!("UV placement expected");
        };
        assert_eq!(a.rotation.to_bits(), b.rotation.to_bits());
        assert_eq!(a.center_px.x.to_bits(), b.center_px.x.to_bits());
    }
    // The referenced blobs come back with their names and bytes.
    for blob in doc.blobs.iter() {
        let other = loaded.blobs.get(blob.id).unwrap_or_else(|| panic!("blob {:?} lost", blob.id));
        assert_eq!((other.kind, other.name.as_str(), &other.bytes[..]), (blob.kind, blob.name.as_str(), &blob.bytes[..]));
    }
    // The loaded document keeps working: undoable edits on the loaded stamps.
    loaded.set_stamp_tint(image_uv, [10, 20, 30, 255], Commit::Final).unwrap();
    assert!(loaded.undo().is_some());
    assert_eq!(doc_state(&mut loaded), expected);

    // Re-saving the loaded project gives a byte-identical manifest.json.
    let again = dir.join("again.wtep");
    project::save(&loaded.snapshot(), &again, &save_options(), &NoProgress).unwrap();
    let (m1, m2) = (read_zip_entry(&path, "manifest.json"), read_zip_entry(&again, "manifest.json"));
    assert!(m1 == m2, "manifest.json differs on re-save:\n{}\n---\n{}", String::from_utf8_lossy(&m1), String::from_utf8_lossy(&m2));
    // Every stamp and text layer was written with its cache rect.
    for (set, entry) in layer_entries(&path) {
        let layer = doc.layer(entry.id).unwrap();
        assert_eq!(doc.find_layer(entry.id).unwrap().0, set);
        match (&entry.kind, entry.pixels) {
            (LayerKindEntry::Raster, Some(p)) => assert!(layer.is_raster() && p.rect[2] > 0),
            (LayerKindEntry::Stamp { stamp } | LayerKindEntry::Text { stamp }, Some(p)) => {
                let r = layer.content_rect();
                assert_eq!(Some(*stamp), layer.stamp_id());
                assert_eq!(p.rect, [r.x0, r.y0, r.width() as i32, r.height() as i32]);
            }
            (_, None) => assert!(layer.views().iter().all(|v| v.data.iter().all(|&b| b == 0))),
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn corrupt_stamp_caches_are_rebuilt() {
    let (mut doc, _) = rich_document();
    let expected = doc_state(&mut doc);
    let dir = temp_dir("roundtrip_corrupt_cache");
    let path = dir.join("good.wtep");
    project::save(&doc.snapshot(), &path, &save_options(), &NoProgress).unwrap();

    // Truncate the PNG of every stamp and text cache.
    let mut corrupted = Vec::new();
    let mut entries = std::collections::BTreeMap::new();
    for (_, entry) in layer_entries(&path) {
        if let (LayerKindEntry::Stamp { .. } | LayerKindEntry::Text { .. }, Some(p)) = (&entry.kind, &entry.pixels) {
            entries.insert(p.file.entry.clone(), entry.id);
            corrupted.push(entry.id);
        }
    }
    assert!(corrupted.len() >= 4, "{corrupted:?}");
    let bad = dir.join("bad.wtep");
    rewrite_project(&path, &bad, |name, mut data| {
        if entries.contains_key(name) {
            data.truncate(data.len() / 2);
        }
        Some(data)
    });

    let report = project::load(&bad, &NoProgress).unwrap();
    let mut warned: Vec<LayerId> = report
        .warnings
        .iter()
        .map(|w| match w {
            LoadWarning::BakeCacheRebuilt { layer } => *layer,
            other => panic!("unexpected warning {other:?}"),
        })
        .collect();
    warned.sort();
    corrupted.sort();
    assert_eq!(warned, corrupted);
    let mut loaded = report.document;
    // The re-bake happened during the load: nothing is left to composite or to upload.
    assert!(!loaded.has_pending_composite());
    for set in loaded.set_ids() {
        assert!(loaded.take_gpu_dirty(set).is_empty());
    }
    // The re-bake reproduces the lost caches exactly, and the document is clean after loading.
    assert_eq!(doc_state(&mut loaded), expected);
    assert!(!loaded.is_modified() && !loaded.can_undo());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn deleted_image_blob_keeps_the_layers_as_raster() {
    let (mut doc, [image3d, image_uv, text3d, text_uv]) = rich_document();
    let expected = doc_state(&mut doc);
    let dir = temp_dir("roundtrip_blob_missing");
    let path = dir.join("good.wtep");
    project::save(&doc.snapshot(), &path, &save_options(), &NoProgress).unwrap();
    let manifest = project::read_manifest(&path).unwrap();
    let image_blob = manifest.blobs.iter().find(|b| b.kind == BlobKind::Image).unwrap().clone();
    let bad = dir.join("bad.wtep");
    rewrite_project(&path, &bad, |name, data| (name != image_blob.file.entry).then_some(data));

    let report = project::load(&bad, &NoProgress).unwrap();
    assert_eq!(report.warnings, vec![LoadWarning::BlobMissing { blob: image_blob.id, used_by: vec![image3d, image_uv] }]);
    let mut loaded = report.document;
    assert!(loaded.stamp(image3d).is_none() && loaded.stamp(image_uv).is_none());
    assert!(loaded.stamp(text3d).is_some() && loaded.stamp(text_uv).is_some());
    let state = doc_state(&mut loaded);
    // Same pixels everywhere; the image stamps' layers are plain raster layers now.
    assert_eq!(state.stamps, expected.stamps.iter().filter(|s| s.id != image3d && s.id != image_uv).cloned().collect::<Vec<_>>());
    for (a, b) in expected.sets.iter().zip(&state.sets) {
        assert_eq!(a.composite, b.composite);
        assert_eq!(a.active, b.active);
        assert_eq!(a.layers.len(), b.layers.len());
    }
    for set in doc.set_ids() {
        let (w, h) = (doc.set(set).unwrap().width, doc.set(set).unwrap().height);
        for (la, lb) in doc.set(set).unwrap().layers.iter().zip(&loaded.set(set).unwrap().layers) {
            assert_eq!((la.id, &la.name, la.visible, la.blend), (lb.id, &lb.name, lb.visible, lb.blend));
            if matches!(la.stamp_id(), Some(s) if s == image3d || s == image_uv) {
                assert!(lb.is_raster() && lb.stamp_id().is_none(), "{} must survive as raster", la.name);
                assert!(dense(la, w, h) == dense(lb, w, h), "{}: pixels", la.name);
            } else {
                assert_eq!(layer_state(la, w, h), layer_state(lb, w, h));
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_font_blob_falls_back_to_the_bundled_font() {
    let (mut doc, [_, _, text3d, text_uv]) = rich_document();
    let expected = doc_state(&mut doc);
    let dir = temp_dir("roundtrip_font_missing");
    let path = dir.join("good.wtep");
    project::save(&doc.snapshot(), &path, &save_options(), &NoProgress).unwrap();

    // Delete the Hack font (used by the UV text only); the default font of the other text stays.
    let StampSource::Text(hack_spec) = doc.stamp(text_uv).unwrap().source.clone() else { panic!() };
    let manifest = project::read_manifest(&path).unwrap();
    let hack_blob = manifest.blobs.iter().find(|b| b.id == hack_spec.font.blob).unwrap().clone();
    assert_eq!(hack_blob.kind, BlobKind::Font);
    let bad = dir.join("bad.wtep");
    rewrite_project(&path, &bad, |name, data| (name != hack_blob.file.entry).then_some(data));

    let report = project::load(&bad, &NoProgress).unwrap();
    assert_eq!(report.warnings, vec![LoadWarning::FontFallback { stamp: text_uv, family: "Hack".to_owned() }]);
    let mut loaded = report.document;
    let state = doc_state(&mut loaded);

    // The other text stamp is untouched.
    assert_eq!(loaded.stamp(text3d), doc.stamp(text3d));
    // The fallen-back stamp keeps everything but its font, which is now the bundled default.
    let StampSource::Text(spec) = loaded.stamp(text_uv).unwrap().source.clone() else { panic!() };
    assert_eq!((spec.font.family.as_str(), spec.font.style.as_str()), (DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE));
    assert_eq!(&loaded.blobs.get(spec.font.blob).unwrap().bytes[..], text::default_font_bytes());
    assert_eq!(TextSpec { font: hack_spec.font.clone(), ..spec.clone() }, hack_spec);
    assert_eq!(loaded.stamp(text_uv).unwrap().placement, doc.stamp(text_uv).unwrap().placement);
    // It was re-baked with the fallback font: a non-empty cache that differs from the Hack one.
    let (_, old_layer) = doc.stamp_layers(text_uv)[0];
    let (_, new_layer) = loaded.stamp_layers(text_uv)[0];
    assert_eq!(old_layer, new_layer);
    let (old, new) = (doc.layer(old_layer).unwrap(), loaded.layer(new_layer).unwrap());
    assert!(!new.content_rect().is_empty());
    let home = doc.find_layer(old_layer).unwrap().0;
    let (w, h) = (doc.set(home).unwrap().width, doc.set(home).unwrap().height);
    assert!(old.to_dense(w, h) != new.to_dense(w, h), "the cache must be the fallback font's re-bake");
    // A re-bake through the editing path gives the same cache (the loaded cache is a correct Full bake).
    loaded.set_stamp_tint(text_uv, [255, 255, 255, 254], Commit::Final).unwrap();
    loaded.set_stamp_tint(text_uv, [255, 255, 255, 255], Commit::Final).unwrap();
    assert_eq!(doc_state(&mut loaded).sets, state.sets);
    // Everything else is unchanged.
    for (a, b) in expected.sets.iter().zip(&state.sets) {
        for (la, lb) in a.layers.iter().zip(&b.layers) {
            if la.stamp != Some(text_uv) {
                assert_eq!(la, lb);
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The model of the frozen version-1 text fixture: a unit quad without materials (the OBJ of the frozen fixture in
/// `project::load`'s tests, so its `uv_hash` is known).
const FROZEN_V1_TEXT_OBJ: &str = "\
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

/// A version-1 manifest with one text stamp and one font blob, exactly as version 1 writes it (round 4, TEXT 4.3
/// item 5). Never edit this string: files of this shape must load forever. Written and run green on the round-3
/// code before the text entry gained `fx`. The loader takes entry sizes from the archive, so the recorded sizes and
/// CRCs are not checked.
const FROZEN_V1_TEXT_MANIFEST: &str = r#"{
  "format": "wtep",
  "format_version": 1,
  "min_reader_version": 1,
  "app_version": "0.1.0",
  "name": "frozen text",
  "saved_unix": 1700000000,
  "next_id": 6,
  "source": {
    "format": "obj",
    "main": {
      "key": "quad.obj",
      "original_name": "quad.obj",
      "file": { "entry": "source/main.obj", "size": 141, "crc32": 0 }
    },
    "sidecars": [],
    "extras": [],
    "original_path": null
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
      "width": 64,
      "height": 64,
      "active_layer": 5,
      "flattened": null,
      "layers": [
        {
          "id": 1,
          "name": "Base",
          "visible": true,
          "locked": false,
          "alpha_lock": false,
          "opacity": 1.0,
          "blend": "normal",
          "kind": "raster",
          "pixels": null
        },
        {
          "id": 5,
          "name": "WTE 2026",
          "visible": true,
          "locked": false,
          "alpha_lock": false,
          "opacity": 1.0,
          "blend": "normal",
          "kind": "text",
          "stamp": 4,
          "pixels": null
        }
      ]
    }
  ],
  "stamps": [
    {
      "id": 4,
      "name": "WTE 2026",
      "home_set": 0,
      "tint": [255, 255, 255, 255],
      "source": {
        "kind": "text",
        "content": "WTE 2026",
        "font": { "blob": 2, "face_index": 0, "family": "Ubuntu", "style": "Light" },
        "size_px": 32.0,
        "color": [200, 30, 30, 255],
        "align": "center",
        "line_spacing": 1.0,
        "letter_spacing": 0.0,
        "outline": { "width_em": 0.0625, "color": [0, 0, 0, 255] }
      },
      "placement": {
        "space": "uv",
        "set": 0,
        "center_px": [32.0, 32.0],
        "size_px": [48.0, 12.0],
        "rotation": 0.0,
        "flip_x": false,
        "flip_y": false,
        "clip_to_islands": false
      }
    }
  ],
  "blobs": [
    {
      "id": 2,
      "kind": "font",
      "original_name": "Ubuntu-Light.ttf",
      "file": { "entry": "blobs/b00002.ttf", "size": 0, "crc32": 0 }
    }
  ],
  "ui_state": {}
}"#;

#[test]
fn frozen_v1_text_fixture_loads() {
    let stored = zip::write::SimpleFileOptions::DEFAULT.compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::DEFAULT.compression_method(zip::CompressionMethod::Deflated);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, options: zip::write::SimpleFileOptions, data: &[u8]| {
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    };
    add("mimetype", stored, b"application/x-wtep");
    add("source/main.obj", deflated, FROZEN_V1_TEXT_OBJ.as_bytes());
    add("blobs/b00002.ttf", deflated, text::default_font_bytes());
    add("manifest.json", deflated, FROZEN_V1_TEXT_MANIFEST.as_bytes());
    let dir = temp_dir("roundtrip_frozen_v1_text");
    let path = dir.join("frozen_text.wtep");
    std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();

    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(!report.force_save_as);
    let mut doc = report.document;
    assert_eq!(doc.name, "frozen text");
    assert_eq!(doc.stamps.len(), 1);
    let stamp = doc.stamp(StampId(4)).unwrap().clone();
    assert_eq!((stamp.name.as_str(), stamp.home_set, stamp.tint), ("WTE 2026", TextureSetId(0), [255; 4]));
    let StampSource::Text(spec) = &stamp.source else { panic!("a text stamp expected") };
    assert_eq!(spec.content, "WTE 2026");
    assert_eq!(spec.font, FontRef { blob: BlobId(2), face_index: 0, family: "Ubuntu".to_owned(), style: "Light".to_owned() });
    assert_eq!(spec.size_px.to_bits(), 32.0f32.to_bits());
    assert_eq!(spec.color, [200, 30, 30, 255]);
    assert_eq!(spec.align, TextAlign::Center);
    assert_eq!((spec.line_spacing, spec.letter_spacing), (1.0, 0.0));
    assert_eq!(spec.outline, Some(Outline { width_em: 0.0625, color: [0, 0, 0, 255] }));
    // Format version 2 (added after the test ran green on the version-1 code): a version-1 text has no effects.
    assert!(spec.fx.is_default());
    assert_eq!(
        stamp.placement,
        Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(32.0, 32.0),
            size_px: Vec2::new(48.0, 12.0),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        })
    );
    let blob = doc.blobs.get(BlobId(2)).expect("the font blob");
    assert_eq!((blob.kind, blob.name.as_str()), (BlobKind::Font, "Ubuntu-Light.ttf"));
    assert_eq!(&blob.bytes[..], text::default_font_bytes());
    let layer = doc.set(TextureSetId(0)).unwrap().layers.iter().find(|l| l.id == LayerId(5)).unwrap();
    assert_eq!((layer.kind_name(), layer.stamp_id()), ("text", Some(StampId(4))));
    // The text is usable with the stored font: an edit re-bakes it into a non-empty cache.
    doc.set_stamp_tint(StampId(4), [255, 255, 255, 254], Commit::Final).unwrap();
    assert!(!doc.layer(LayerId(5)).unwrap().content_rect().is_empty(), "the re-baked text has ink");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn project_saved_after_a_preview_edit_holds_full_quality_caches() {
    fn setup() -> (Document, StampId, Projector) {
        let mut doc = importable_doc(samples::occluded_quads(), "quads", 256);
        let blob = doc.add_image_blob("logo.png", png_bytes(&samples::sample_stamp_image(64))).unwrap();
        let p = Projector::from_surface(Vec3::new(0.8, 0.8, 0.0), Vec3::Z, Vec3::Y, Vec2::splat(0.4), 1.0);
        let id = doc.add_stamp("Logo", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
        (doc, id, p)
    }
    let moved = |p: &Projector| Placement::Projected(p.moved_to(Vec3::new(0.9, 0.85, 0.0), Vec3::Z).rotated(0.2));

    let (mut reference, id, p) = setup();
    reference.set_stamp_placement(id, moved(&p), Commit::Final).unwrap();
    let full = doc_state(&mut reference);

    let (mut doc, id, p) = setup();
    doc.set_stamp_placement(id, moved(&p), Commit::Preview).unwrap();
    assert!(doc.has_pending_edit());
    assert_ne!(doc_state(&mut doc), full, "a Preview bake (no occlusion) differs from Full on occluded_quads");
    // What the app does before saving: end the pending edit.
    doc.flush_pending_edit();
    assert!(!doc.has_pending_edit());
    assert_eq!(doc_state(&mut doc), full);

    let dir = temp_dir("roundtrip_preview");
    let path = dir.join("preview.wtep");
    project::save(&doc.snapshot(), &path, &save_options(), &NoProgress).unwrap();
    let report = project::load(&path, &NoProgress).unwrap();
    // The flat fixture carries no normals; generated normals were reported at import and are not repeated on load.
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let mut loaded = report.document;
    assert_eq!(doc_state(&mut loaded), full, "the saved caches are the Full-quality ones");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ------------------------------------------------------------------------------------------------------------------
// Round 4: text effects in the project file (format version 2, FEATURE_TEXT 4.3, tests of 8.1 "Project")
// ------------------------------------------------------------------------------------------------------------------

/// A unit quad with a blank 128 px texture: the carrier of the text tests below.
fn plate_doc() -> Document {
    importable_doc(samples::flat_quad(), "plate", 128)
}

/// A 32 px text with the round-3 settings of `text_spec` and the given effects.
fn fx_text(font: FontRef, content: &str, fx: TextFx) -> TextSpec {
    TextSpec { size_px: 32.0, fx, ..text_spec(font, content) }
}

/// Places `spec` as a UV text of set 0 around `center`.
fn uv_text(doc: &mut Document, name: &str, spec: TextSpec, center: [f32; 2]) -> StampId {
    let uv = UvPlacement {
        set: TextureSetId(0),
        center_px: Vec2::from(center),
        size_px: Vec2::new(96.0, 24.0),
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        clip_to_islands: false,
    };
    doc.add_stamp(name, StampSource::Text(spec), Placement::Uv(uv), TextureSetId(0)).unwrap()
}

fn text_of(doc: &Document, id: StampId) -> TextSpec {
    match &doc.stamp(id).unwrap().source {
        StampSource::Text(spec) => spec.clone(),
        StampSource::Image { .. } => panic!("stamp {id:?} is not a text"),
    }
}

/// Every effect at once: the example "with everything set" of FEATURE_TEXT 4.3.
fn full_fx() -> TextFx {
    TextFx {
        gaps: vec![Gap { after: 2, em: 0.3 }, Gap { after: 5, em: -0.05 }],
        bold_em: 0.02,
        slant_deg: 12.0,
        warp: Some(Warp { style: WarpStyle::Arc, bend: 0.5, horizontal: 0.0, vertical: 0.0, vertical_axis: false }),
        path: Some(TextPath {
            shape: PathShape::Curve { points: vec![[-2.0, 0.3], [0.0, -0.2], [2.0, 0.3]] },
            start: 0.0,
            flip: false,
        }),
    }
}

fn save_to(doc: &Document, path: &Path) {
    project::save(&doc.snapshot(), path, &save_options(), &NoProgress).unwrap();
}

fn manifest_value(path: &Path) -> Value {
    serde_json::from_slice(&read_zip_entry(path, "manifest.json")).unwrap()
}

/// The `source` object of stamp `id` in a manifest.
fn source_of(manifest: &Value, id: StampId) -> &Value {
    let stamps = manifest["stamps"].as_array().expect("a stamp list");
    let stamp = stamps.iter().find(|s| s["id"] == json!(id.0)).unwrap_or_else(|| panic!("stamp {id:?} not in the manifest"));
    &stamp["source"]
}

fn source_of_mut(manifest: &mut Value, id: StampId) -> &mut Value {
    let stamps = manifest["stamps"].as_array_mut().expect("a stamp list");
    let stamp = stamps.iter_mut().find(|s| s["id"] == json!(id.0)).unwrap_or_else(|| panic!("stamp {id:?} not in the manifest"));
    &mut stamp["source"]
}

/// Copies the project `src` to `dst` with its manifest edited.
fn with_manifest(src: &Path, dst: &Path, edit: impl FnOnce(&mut Value)) {
    let mut edit = Some(edit);
    rewrite_project(src, dst, |name, data| {
        if name != "manifest.json" {
            return Some(data);
        }
        let mut value: Value = serde_json::from_slice(&data).unwrap();
        (edit.take().expect("one manifest"))(&mut value);
        Some(serde_json::to_vec_pretty(&value).unwrap())
    });
}

/// Like `with_manifest`, and the stored cache picture of text stamp `id` is replaced by a solid magenta image of the
/// same size (a valid PNG, so the load does not re-bake it as unreadable). The loaded layer shows text again only
/// when the load re-baked the stamp; whatever the painters draw, the magenta block proves the re-bake.
fn with_manifest_and_solid_cache(src: &Path, dst: &Path, id: StampId, edit: impl FnOnce(&mut Value)) {
    let (_, entry) = layer_entries(src)
        .into_iter()
        .find(|(_, e)| matches!(e.kind, LayerKindEntry::Text { stamp } if stamp == id))
        .unwrap_or_else(|| panic!("no layer of stamp {id:?}"));
    let pixels = entry.pixels.expect("a stored cache picture");
    let [_, _, w, h] = pixels.rect;
    let solid = PixelBuf { width: w as u32, height: h as u32, data: [255, 0, 255, 255].repeat((w * h) as usize) };
    let solid_png = png_bytes(&solid).to_vec();
    let mut edit = Some(edit);
    rewrite_project(src, dst, |name, data| {
        if name == pixels.file.entry {
            return Some(solid_png.clone());
        }
        if name != "manifest.json" {
            return Some(data);
        }
        let mut value: Value = serde_json::from_slice(&data).unwrap();
        (edit.take().expect("one manifest"))(&mut value);
        Some(serde_json::to_vec_pretty(&value).unwrap())
    });
}

/// True when the stamp's layer still holds a magenta pixel of `with_manifest_and_solid_cache`.
fn has_magenta(doc: &Document, id: StampId) -> bool {
    stamp_pixels(doc, id).data.as_chunks::<4>().0.contains(&[255, 0, 255, 255])
}

/// The keys of a JSON object, sorted.
fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value.as_object().expect("an object").keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

/// The pixels of the (only) layer of a stamp.
fn stamp_pixels(doc: &Document, id: StampId) -> PixelBuf {
    let (set, layer) = doc.stamp_layers(id)[0];
    let set = doc.set(set).unwrap();
    doc.layer(layer).unwrap().to_dense(set.width, set.height)
}

/// Builds a project archive around a literal manifest: the quad model of the frozen fixtures and the bundled Ubuntu
/// Light as blob 2.
fn frozen_archive(dir: &Path, name: &str, manifest: &str) -> PathBuf {
    let stored = zip::write::SimpleFileOptions::DEFAULT.compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::DEFAULT.compression_method(zip::CompressionMethod::Deflated);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |entry: &str, options: zip::write::SimpleFileOptions, data: &[u8]| {
        zip.start_file(entry, options).unwrap();
        zip.write_all(data).unwrap();
    };
    add("mimetype", stored, b"application/x-wtep");
    add("source/main.obj", deflated, FROZEN_V1_TEXT_OBJ.as_bytes());
    add("blobs/b00002.ttf", deflated, text::default_font_bytes());
    add("manifest.json", deflated, manifest.as_bytes());
    let path = dir.join(name);
    std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();
    path
}

/// A version-2 manifest with one text stamp that uses every effect (the `fx` object of FEATURE_TEXT 4.3), around the
/// same quad and font as the version-1 fixture. Never edit this string: files of this shape must load forever.
const FROZEN_V2_FX_MANIFEST: &str = r#"{
  "format": "wtep",
  "format_version": 2,
  "min_reader_version": 2,
  "app_version": "0.4.0",
  "name": "frozen fx",
  "saved_unix": 1763400000,
  "next_id": 6,
  "source": {
    "format": "obj",
    "main": {
      "key": "quad.obj",
      "original_name": "quad.obj",
      "file": { "entry": "source/main.obj", "size": 141, "crc32": 0 }
    },
    "sidecars": [],
    "extras": [],
    "original_path": null
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
      "width": 64,
      "height": 64,
      "active_layer": 5,
      "flattened": null,
      "layers": [
        {
          "id": 1,
          "name": "Base",
          "visible": true,
          "locked": false,
          "alpha_lock": false,
          "opacity": 1.0,
          "blend": "normal",
          "kind": "raster",
          "pixels": null
        },
        {
          "id": 5,
          "name": "WTE 2026",
          "visible": true,
          "locked": false,
          "alpha_lock": false,
          "opacity": 1.0,
          "blend": "normal",
          "kind": "text",
          "stamp": 4,
          "pixels": null
        }
      ]
    }
  ],
  "stamps": [
    {
      "id": 4,
      "name": "WTE 2026",
      "home_set": 0,
      "tint": [255, 255, 255, 255],
      "source": {
        "kind": "text",
        "content": "WTE 2026",
        "font": { "blob": 2, "face_index": 0, "family": "Ubuntu", "style": "Light" },
        "size_px": 32.0,
        "color": [0, 0, 0, 255],
        "align": "center",
        "line_spacing": 1.0,
        "letter_spacing": 0.0,
        "outline": null,
        "fx": {
          "gaps": [ { "after": 2, "em": 0.3 }, { "after": 5, "em": -0.05 } ],
          "bold_em": 0.02,
          "slant_deg": 12.0,
          "warp": { "style": "arc", "bend": 0.5, "horizontal": 0.0, "vertical": 0.0, "vertical_axis": false },
          "path": { "kind": "curve", "points": [[-2.0, 0.3], [0.0, -0.2], [2.0, 0.3]], "start": 0.0, "flip": false }
        }
      },
      "placement": {
        "space": "uv",
        "set": 0,
        "center_px": [32.0, 32.0],
        "size_px": [48.0, 12.0],
        "rotation": 0.0,
        "flip_x": false,
        "flip_y": false,
        "clip_to_islands": false
      }
    }
  ],
  "blobs": [
    {
      "id": 2,
      "kind": "font",
      "original_name": "Ubuntu-Light.ttf",
      "file": { "entry": "blobs/b00002.ttf", "size": 0, "crc32": 0 }
    }
  ],
  "ui_state": {}
}"#;

#[test]
fn frozen_v2_fx_fixture_loads() {
    let dir = temp_dir("roundtrip_frozen_v2_fx");
    let path = frozen_archive(&dir, "frozen_fx.wtep", FROZEN_V2_FX_MANIFEST);
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(!report.force_save_as);
    let mut doc = report.document;
    assert_eq!(doc.name, "frozen fx");
    let spec = text_of(&doc, StampId(4));
    assert_eq!(spec.content, "WTE 2026");
    assert_eq!(spec.font, FontRef { blob: BlobId(2), face_index: 0, family: "Ubuntu".to_owned(), style: "Light".to_owned() });
    assert_eq!((spec.size_px, spec.color, spec.align, spec.outline), (32.0, [0, 0, 0, 255], TextAlign::Center, None));
    assert_eq!(spec.fx, full_fx());
    assert_eq!(spec.fx.gaps[1].em.to_bits(), (-0.05f32).to_bits());

    // Saved again: the file needs a version-2 reader and carries the effects exactly as they were read (round 6: the
    // writer's own version is 3).
    let again = dir.join("again.wtep");
    save_to(&doc, &again);
    let saved = manifest_value(&again);
    assert_eq!((&saved["format_version"], &saved["min_reader_version"]), (&json!(FORMAT_VERSION), &json!(2)));
    let fixture: Value = serde_json::from_str(FROZEN_V2_FX_MANIFEST).unwrap();
    assert_eq!(source_of(&saved, StampId(4)), source_of(&fixture, StampId(4)));

    // The text is usable: an edit re-bakes it into a non-empty cache.
    doc.set_stamp_tint(StampId(4), [255, 255, 255, 254], Commit::Final).unwrap();
    assert!(!doc.layer(LayerId(5)).unwrap().content_rect().is_empty(), "the re-baked text has ink");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn gaps_round_trip() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let gaps = vec![Gap { after: 0, em: 0.3 }, Gap { after: 3, em: -0.125 }, Gap { after: 6, em: 1.7 }];
    let fx = TextFx { gaps: gaps.clone(), ..TextFx::default() };
    let id = uv_text(&mut doc, "Gaps", fx_text(font, "WTE 2026", fx), [64.0, 64.0]);
    assert_eq!(text_of(&doc, id).fx.gaps, gaps);
    let expected = doc_state(&mut doc);

    let dir = temp_dir("roundtrip_gaps");
    let path = dir.join("gaps.wtep");
    save_to(&doc, &path);
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty() && !report.force_save_as, "{:?}", report.warnings);
    let mut loaded = report.document;
    let back = text_of(&loaded, id).fx.gaps;
    assert_eq!(back.len(), 3);
    for (a, b) in gaps.iter().zip(&back) {
        assert_eq!((a.after, a.em.to_bits()), (b.after, b.em.to_bits()), "bit for bit");
    }
    assert_eq!(doc_state(&mut loaded), expected);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn fx_round_trips() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let mut ids = Vec::new();
    // Each of the 15 warp styles, with thickening, slant and both directions.
    for (i, style) in WarpStyle::ALL.into_iter().enumerate() {
        let k = i as f32;
        let fx = TextFx {
            bold_em: 0.02 + 0.001 * k,
            slant_deg: -20.0 + 3.0 * k,
            warp: Some(Warp { style, bend: 0.5 - 0.07 * k, horizontal: 0.25, vertical: -0.125, vertical_axis: i % 2 == 1 }),
            ..TextFx::default()
        };
        ids.push(uv_text(&mut doc, style.label(), fx_text(font.clone(), "Ag", fx), [64.0, 40.0]));
    }
    // Each line kind.
    let shapes = [
        PathShape::Arc { bend: -0.4 },
        PathShape::Circle { diameter: 6.0, center: [0.5, -2.0] },
        PathShape::Curve { points: vec![[-3.0, 0.5], [-1.0, -0.25], [1.0, 0.75], [3.0, 0.0]] },
    ];
    for (i, shape) in shapes.into_iter().enumerate() {
        let fx = TextFx {
            slant_deg: 8.0,
            path: Some(TextPath { shape, start: 1.5 * i as f32, flip: i == 1 }),
            ..TextFx::default()
        };
        ids.push(uv_text(&mut doc, &format!("Line {i}"), fx_text(font.clone(), "Ag", fx), [64.0, 90.0]));
    }
    assert_eq!(ids.len(), 18);
    let expected = doc_state(&mut doc);

    let dir = temp_dir("roundtrip_fx");
    let path = dir.join("fx.wtep");
    save_to(&doc, &path);
    assert_eq!(manifest_value(&path)["min_reader_version"], json!(2));
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty() && !report.force_save_as, "{:?}", report.warnings);
    let mut loaded = report.document;
    for &id in &ids {
        assert_eq!(text_of(&loaded, id), text_of(&doc, id), "{id:?}");
    }
    assert_eq!(doc_state(&mut loaded), expected);
    // Re-saving the loaded project gives a byte-identical manifest.
    let again = dir.join("again.wtep");
    save_to(&loaded, &again);
    assert!(read_zip_entry(&path, "manifest.json") == read_zip_entry(&again, "manifest.json"), "manifest differs on re-save");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn plain_text_entry_has_the_v1_keys() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let id = uv_text(&mut doc, "Plain", fx_text(font, "Plain", TextFx::default()), [64.0, 64.0]);
    let dir = temp_dir("roundtrip_plain_keys");
    let path = dir.join("plain.wtep");
    save_to(&doc, &path);
    let manifest = manifest_value(&path);
    assert_eq!(
        keys(source_of(&manifest, id)),
        ["align", "color", "content", "font", "kind", "letter_spacing", "line_spacing", "outline", "size_px"]
    );
    // Readable by the round-3 build (its format version is 1).
    assert_eq!((&manifest["format_version"], &manifest["min_reader_version"]), (&json!(FORMAT_VERSION), &json!(1)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn required_reader_follows_fx() {
    assert_eq!(FORMAT_VERSION, 3);
    // The migration steps (`STEPS.len() == FORMAT_VERSION - 1` is asserted in `project::migrate`) change nothing
    // but the version of a version-1 manifest.
    let v1: Value = serde_json::from_str(FROZEN_V1_TEXT_MANIFEST).unwrap();
    let mut expected = v1.clone();
    expected["format_version"] = json!(FORMAT_VERSION);
    assert_eq!(project::migrate::upgrade(v1, 1).unwrap(), expected);

    let mut doc = plate_doc();
    let font = doc.default_font();
    let plain = fx_text(font, "WTE 2026", TextFx::default());
    let id = uv_text(&mut doc, "Text", plain.clone(), [64.0, 64.0]);
    let blob = doc.add_image_blob("logo.png", png_bytes(&samples::sample_stamp_image(32))).unwrap();
    let uv = UvPlacement {
        set: TextureSetId(0),
        center_px: Vec2::new(30.0, 100.0),
        size_px: Vec2::new(40.0, 40.0),
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        clip_to_islands: false,
    };
    doc.add_stamp("Logo", StampSource::Image { blob }, Placement::Uv(uv), TextureSetId(0)).unwrap();
    assert_eq!(project::required_reader(&doc.snapshot()), 1, "plain text and an image");

    let arc = TextPath { shape: PathShape::Arc { bend: 0.0 }, start: 0.0, flip: false };
    let cases = [
        ("one gap value", TextFx { gaps: vec![Gap { after: 0, em: 0.1 }], ..TextFx::default() }, 2),
        ("a warp", TextFx { warp: Some(Warp::new(WarpStyle::Flag)), ..TextFx::default() }, 2),
        ("a slant", TextFx { slant_deg: -12.0, ..TextFx::default() }, 2),
        ("thickening", TextFx { bold_em: 0.02, ..TextFx::default() }, 2),
        ("an arc of bend 0", TextFx { path: Some(arc), ..TextFx::default() }, 2),
        ("no effect again", TextFx::default(), 1),
    ];
    let dir = temp_dir("roundtrip_required_reader");
    for (n, (what, fx, needed)) in cases.into_iter().enumerate() {
        doc.set_stamp_source(id, StampSource::Text(TextSpec { fx, ..plain.clone() }), Commit::Final).unwrap();
        assert_eq!(project::required_reader(&doc.snapshot()), needed, "{what}");
        let path = dir.join(format!("case{n}.wtep"));
        save_to(&doc, &path);
        let manifest = manifest_value(&path);
        assert_eq!((&manifest["format_version"], &manifest["min_reader_version"]), (&json!(FORMAT_VERSION), &json!(needed)), "{what}");
        // What a round-3 build (format version 1) does with it: it refuses exactly the files that need version 2.
        assert_eq!(manifest["min_reader_version"].as_u64().unwrap() > 1, needed == 2, "{what}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_end_of_line_gap_needs_reader_2() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    // A value after "B", the last character of the first line: it has no effect now, but it is real data.
    let fx = TextFx { gaps: vec![Gap { after: 1, em: 0.4 }], ..TextFx::default() };
    let id = uv_text(&mut doc, "Lines", fx_text(font, "AB\nCD", fx.clone()), [64.0, 64.0]);
    assert_eq!(text_of(&doc, id).fx, fx, "the end-of-line value is kept");
    assert_eq!(project::required_reader(&doc.snapshot()), 2);
    let dir = temp_dir("roundtrip_end_of_line_gap");
    let path = dir.join("lines.wtep");
    save_to(&doc, &path);
    assert_eq!(manifest_value(&path)["min_reader_version"], json!(2));
    let loaded = project::load(&path, &NoProgress).unwrap().document;
    assert_eq!(text_of(&loaded, id).fx, fx);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unknown_warp_style_loads_without_warp() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let fx = TextFx { gaps: vec![Gap { after: 1, em: 0.5 }], warp: Some(Warp::new(WarpStyle::Flag)), ..TextFx::default() };
    let id = uv_text(&mut doc, "Ribbon", fx_text(font.clone(), "WTE 2026", fx.clone()), [64.0, 64.0]);
    // The same text without the warp, placed the same way in a second document: what the loaded stamp must show.
    let without = TextFx { warp: None, ..fx };
    let mut reference = plate_doc();
    let reference_font = reference.default_font();
    assert_eq!(reference_font, font);
    let reference_id = uv_text(&mut reference, "Ribbon", fx_text(reference_font, "WTE 2026", without.clone()), [64.0, 64.0]);
    assert_eq!(reference_id, id);

    let dir = temp_dir("roundtrip_unknown_warp");
    let good = dir.join("good.wtep");
    save_to(&doc, &good);
    let ribbon = dir.join("ribbon.wtep");
    // The stored picture is replaced by a magenta block: only a re-bake during the load can give the reference
    // pixels, whatever the painters draw.
    with_manifest_and_solid_cache(&good, &ribbon, id, |m| source_of_mut(m, id)["fx"]["warp"]["style"] = json!("ribbon"));

    let report = project::load(&ribbon, &NoProgress).unwrap();
    let unknown = LoadWarning::TextFeatureUnknown { stamp: id, what: "warp style \"ribbon\"".to_owned() };
    assert_eq!(report.warnings, vec![unknown.clone()]);
    assert!(report.force_save_as);
    let loaded = report.document;
    let spec = text_of(&loaded, id);
    assert!(spec.fx.warp.is_none());
    assert_eq!(spec, TextSpec { fx: without, ..text_of(&doc, id) }, "everything else is kept");
    // The stamp was re-baked without the warp during the load, and the document is clean.
    assert!(!has_magenta(&loaded, id), "the stored picture was replaced by a re-bake");
    assert!(stamp_pixels(&loaded, id) == stamp_pixels(&reference, id), "the picture shows the text without the warp");
    assert!(!loaded.is_modified() && !loaded.can_undo());
    // The message names the feature.
    assert!(unknown.to_string().contains("warp style \"ribbon\""), "{unknown}");

    // A newer format version with the same entry: both warnings, Save As forced.
    let newer = dir.join("newer.wtep");
    with_manifest(&good, &newer, |m| {
        m["format_version"] = json!(FORMAT_VERSION + 1);
        source_of_mut(m, id)["fx"]["warp"]["style"] = json!("ribbon");
    });
    let report = project::load(&newer, &NoProgress).unwrap();
    assert_eq!(report.warnings, vec![LoadWarning::NewerFormat { found: FORMAT_VERSION + 1 }, unknown]);
    assert!(report.force_save_as);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn broken_fx_from_a_file_is_repaired() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let a = uv_text(&mut doc, "Repaired", fx_text(font.clone(), "WTE 2026", TextFx::default()), [64.0, 40.0]);
    let b = uv_text(&mut doc, "Nameless", fx_text(font, "Broken", TextFx::default()), [64.0, 90.0]);
    let dir = temp_dir("roundtrip_broken_fx");
    let good = dir.join("good.wtep");
    save_to(&doc, &good);

    let points: Vec<[f32; 2]> = (0..40).map(|i| if i == 1 { [1e6, -1e6] } else { [0.1 * i as f32, 0.0] }).collect();
    let bad = dir.join("bad.wtep");
    // The stored picture of `a` is replaced by a magenta block: its effects are repaired, so the load must re-bake it.
    with_manifest_and_solid_cache(&good, &bad, a, |m| {
        // Values out of range: a gap beyond the text, a gap too wide, a bend of 7, 40 curve points, one at 1e6 em.
        source_of_mut(m, a)["fx"] = json!({
            "gaps": [ { "after": 2, "em": 0.3 }, { "after": 99, "em": 0.5 }, { "after": 4, "em": 9.0 } ],
            "warp": { "style": "arc", "bend": 7.0 },
            "path": { "kind": "curve", "points": points, "start": 0.0 }
        });
        // Missing names: a warp without a style, a path without a kind.
        source_of_mut(m, b)["fx"] = json!({ "warp": { "bend": 0.5 }, "path": { "start": 1.0 } });
    });

    let report = project::load(&bad, &NoProgress).unwrap();
    assert_eq!(
        report.warnings,
        vec![
            LoadWarning::TextFeatureUnknown { stamp: b, what: "warp style \"\"".to_owned() },
            LoadWarning::TextFeatureUnknown { stamp: b, what: "path kind \"\"".to_owned() },
        ]
    );
    assert!(report.force_save_as);
    let loaded = report.document;
    let repaired = text_of(&loaded, a).fx;
    assert_eq!(repaired.gaps, vec![Gap { after: 2, em: 0.3 }, Gap { after: 4, em: GAP_MAX_EM }]);
    assert_eq!(repaired.warp, Some(Warp { style: WarpStyle::Arc, bend: 1.0, horizontal: 0.0, vertical: 0.0, vertical_axis: false }));
    let Some(TextPath { shape: PathShape::Curve { points }, .. }) = &repaired.path else { panic!("a curve: {repaired:?}") };
    assert_eq!(points.len(), MAX_PATH_POINTS);
    assert_eq!(points[1], [PATH_COORD_MAX_EM, -PATH_COORD_MAX_EM]);
    assert_eq!(points[2], [0.2, 0.0]);
    assert!(text_of(&loaded, b).fx.is_default(), "both unknown features are left out");
    // A repaired text is re-baked during the load (no warning: the values were only out of range), so its picture is
    // the one of the repaired spec: equal to a fresh text with that spec at the same place.
    assert!(!has_magenta(&loaded, a), "the stamp with repaired effects was re-baked");
    let mut reference = plate_doc();
    let placement = loaded.stamp(a).unwrap().placement;
    let reference_a =
        reference.add_stamp("Repaired", StampSource::Text(text_of(&loaded, a)), placement, TextureSetId(0)).unwrap();
    assert_eq!(reference_a, a);
    assert_eq!(text_of(&reference, a), text_of(&loaded, a), "the repaired spec is already sanitized");
    assert!(stamp_pixels(&loaded, a) == stamp_pixels(&reference, a), "the picture follows the repaired spec");
    assert!(!loaded.is_modified() && !loaded.can_undo());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn valid_fx_written_differently_keeps_the_stored_picture() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let id = uv_text(&mut doc, "Unusual", fx_text(font, "WTE 2026", TextFx::default()), [64.0, 64.0]);
    let dir = temp_dir("roundtrip_unusual_fx");
    let good = dir.join("good.wtep");
    save_to(&doc, &good);
    let unusual = dir.join("unusual.wtep");
    // Valid values, written the way a person or another program might: gaps out of order, two entries for one
    // character (the later wins), a gap of -0.0, and an arc path that also carries circle and curve keys. Nothing is
    // repaired, so the stored picture (here a magenta block) stays and no re-bake runs.
    with_manifest_and_solid_cache(&good, &unusual, id, |m| {
        source_of_mut(m, id)["fx"] = json!({
            "gaps": [
                { "after": 4, "em": 0.2 },
                { "after": 2, "em": 0.1 },
                { "after": 1, "em": -0.0 },
                { "after": 2, "em": 0.3 }
            ],
            "warp": { "style": "arc", "bend": 0.5 },
            "path": { "kind": "arc", "bend": 0.3, "diameter": 6.0, "center": [1.0, 2.0], "points": [[0.0, 0.0], [1.0, 1.0]], "start": 0.5 }
        });
    });
    let report = project::load(&unusual, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(!report.force_save_as);
    let loaded = report.document;
    let fx = text_of(&loaded, id).fx;
    assert_eq!(fx.gaps, vec![Gap { after: 2, em: 0.3 }, Gap { after: 4, em: 0.2 }]);
    assert_eq!(fx.path, Some(TextPath { shape: PathShape::Arc { bend: 0.3 }, start: 0.5, flip: false }));
    assert!(has_magenta(&loaded, id), "the stored picture is kept: the effects needed no repair");

    // One value out of range in the same unusual file still re-bakes.
    let repaired = dir.join("repaired.wtep");
    with_manifest_and_solid_cache(&good, &repaired, id, |m| {
        source_of_mut(m, id)["fx"] = json!({
            "gaps": [ { "after": 4, "em": 0.2 }, { "after": 2, "em": 0.3 } ],
            "path": { "kind": "arc", "bend": 7.0, "diameter": 6.0, "start": 0.5 }
        });
    });
    let loaded = project::load(&repaired, &NoProgress).unwrap().document;
    assert!(!has_magenta(&loaded, id), "a repaired value re-bakes");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_path_entry_writes_only_its_own_keys() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let shapes = [
        (PathShape::Arc { bend: 0.4 }, vec!["bend", "flip", "kind", "start"]),
        (PathShape::Arc { bend: 0.0 }, vec!["flip", "kind", "start"]),
        (PathShape::Circle { diameter: 6.0, center: [0.5, -2.0] }, vec!["center", "diameter", "flip", "kind", "start"]),
        (PathShape::Curve { points: vec![[-2.0, 0.3], [0.0, -0.2], [2.0, 0.3]] }, vec!["flip", "kind", "points", "start"]),
    ];
    let mut ids = Vec::new();
    for (i, (shape, _)) in shapes.iter().enumerate() {
        let fx = TextFx { path: Some(TextPath { shape: shape.clone(), start: 0.5, flip: false }), ..TextFx::default() };
        ids.push(uv_text(&mut doc, &format!("Path {i}"), fx_text(font.clone(), "Ag", fx), [64.0, 30.0 + 20.0 * i as f32]));
    }
    let dir = temp_dir("roundtrip_path_keys");
    let path = dir.join("paths.wtep");
    save_to(&doc, &path);
    let manifest = manifest_value(&path);
    for (id, (_, expected)) in ids.iter().zip(&shapes) {
        assert_eq!(keys(&source_of(&manifest, *id)["fx"]["path"]), *expected, "{id:?}");
    }
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    for (id, (shape, _)) in ids.iter().zip(&shapes) {
        assert_eq!(text_of(&report.document, *id).fx.path.map(|p| p.shape), Some(shape.clone()), "an arc of bend 0 stays an arc");
        assert_eq!(text_of(&report.document, *id), text_of(&doc, *id));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_wrong_type_in_fx_is_a_manifest_error() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let fx = TextFx { gaps: vec![Gap { after: 1, em: 0.2 }], ..TextFx::default() };
    let id = uv_text(&mut doc, "Typed", fx_text(font, "WTE 2026", fx), [64.0, 64.0]);
    let dir = temp_dir("roundtrip_wrong_type");
    let good = dir.join("good.wtep");
    save_to(&doc, &good);
    // The documented limit of FEATURE_TEXT 4.3 item 4: a wrong JSON type fails the manifest like in version 1.
    let edits: [(&str, &str, Value); 3] = [
        ("a text where a number belongs", "bold_em", json!("thick")),
        ("a negative gap index", "gaps", json!([{ "after": -1, "em": 0.2 }])),
        ("an object in place of a list", "gaps", json!({ "after": 1, "em": 0.2 })),
    ];
    for (n, (what, key, value)) in edits.into_iter().enumerate() {
        let bad = dir.join(format!("bad{n}.wtep"));
        with_manifest(&good, &bad, |m| source_of_mut(m, id)["fx"][key] = value);
        match project::load(&bad, &NoProgress) {
            Err(ProjectError::ManifestInvalid(_)) => {}
            Err(other) => panic!("{what}: {other}"),
            Ok(_) => panic!("{what}: the project loaded"),
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_nan_in_any_float_refuses_save() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    // An outline (from `text_spec`), 2 gaps, a warp and a curve of 3 points: 18 floats.
    let id = uv_text(&mut doc, "Full", fx_text(font, "WTE 2026", full_fx()), [64.0, 64.0]);
    let mut names = Vec::new();
    text_of(&doc, id).floats(&mut |name, _| names.push(name));
    assert_eq!(names.len(), 18, "{names:?}");
    let index = doc.stamps.iter().position(|s| s.id == id).unwrap();
    let dir = temp_dir("roundtrip_nan");
    let path = dir.join("nan.wtep");
    for (case, name) in names.iter().enumerate() {
        // Straight into the snapshot, past `sanitize_fx`.
        let mut snapshot = doc.snapshot();
        let StampSource::Text(spec) = &mut snapshot.stamps[index].source else { panic!("a text stamp expected") };
        let mut n = 0;
        spec.floats_mut(&mut |_, v| {
            if n == case {
                *v = f32::NAN;
            }
            n += 1;
        });
        match project::save(&snapshot, &path, &save_options(), &NoProgress) {
            Err(ProjectError::ManifestInvalid(message)) => {
                assert!(message.starts_with(name) && message.contains("stamp \"Full\""), "case {case} ({name}): {message}");
            }
            Err(other) => panic!("case {case} ({name}): {other}"),
            Ok(_) => panic!("case {case} ({name}): saved with a NaN"),
        }
        assert!(!path.exists(), "case {case}: nothing is written");
    }
    // Without a NaN the same document saves.
    save_to(&doc, &path);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn nan_in_arc_and_circle_floats_refuses_save() {
    // The Arc and Circle branches of the float visitor, which the Curve path of `full_fx` never reaches.
    let mut doc = plate_doc();
    let font = doc.default_font();
    let cases = [
        ("Arc", PathShape::Arc { bend: 0.4 }, vec!["the arc bend"]),
        (
            "Circle",
            PathShape::Circle { diameter: 6.0, center: [0.5, -2.0] },
            vec!["the circle diameter", "the circle centre", "the circle centre"],
        ),
    ];
    let dir = temp_dir("roundtrip_nan_arc_circle");
    let path = dir.join("nan.wtep");
    for (i, (stamp, shape, expected)) in cases.iter().enumerate() {
        let fx = TextFx { path: Some(TextPath { shape: shape.clone(), start: 0.5, flip: false }), ..TextFx::default() };
        let id = uv_text(&mut doc, stamp, fx_text(font.clone(), "Ag", fx), [64.0, 30.0 + 40.0 * i as f32]);
        let mut names = Vec::new();
        text_of(&doc, id).floats(&mut |name, _| names.push(name));
        // The shape's own floats come right after "the path start", the last of the visitor's list.
        let start = names.iter().position(|n| *n == "the path start").expect("the path start is visited") + 1;
        assert_eq!(names[start..], expected[..], "{stamp}: {names:?}");
        let index = doc.stamps.iter().position(|s| s.id == id).unwrap();
        for (case, &name) in names.iter().enumerate().skip(start) {
            let mut snapshot = doc.snapshot();
            let StampSource::Text(spec) = &mut snapshot.stamps[index].source else { panic!("a text stamp expected") };
            let mut n = 0;
            spec.floats_mut(&mut |_, v| {
                if n == case {
                    *v = f32::NAN;
                }
                n += 1;
            });
            match project::save(&snapshot, &path, &save_options(), &NoProgress) {
                Err(ProjectError::ManifestInvalid(message)) => {
                    assert!(
                        message.starts_with(name) && message.contains(&format!("stamp \"{stamp}\"")),
                        "{stamp} case {case} ({name}): {message}"
                    );
                }
                Err(other) => panic!("{stamp} case {case} ({name}): {other}"),
                Ok(_) => panic!("{stamp} case {case} ({name}): saved with a NaN"),
            }
            assert!(!path.exists(), "{stamp} case {case}: nothing is written");
        }
    }
    // Without a NaN the same document saves.
    save_to(&doc, &path);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Exports the flat PNG and the per-layer PNGs of every set into `dir`: file name -> bytes.
fn export_pngs(doc: &Document, dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::create_dir_all(dir).unwrap();
    let opts = ExportOptions {
        dir: dir.to_path_buf(),
        base_name: "plate".to_owned(),
        sets: None,
        flat: Some(FlatFormat::Png(PngLevel::Balanced)),
        per_layer_png: true,
        ora: false,
        psd: false,
        include_hidden: true,
        dilate_px: 0,
    };
    let report = export::run(&doc.snapshot(), &opts, &NoProgress).unwrap();
    report
        .files
        .iter()
        .map(|f| (f.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(f).unwrap()))
        .collect()
}

#[test]
fn styled_text_survives_save_reopen_and_export() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let fx = TextFx {
        gaps: vec![Gap { after: 2, em: 0.4 }],
        bold_em: 0.03,
        slant_deg: 12.0,
        warp: Some(Warp { style: WarpStyle::Arc, bend: 0.5, horizontal: 0.0, vertical: 0.0, vertical_axis: false }),
        path: None,
    };
    let uv = uv_text(&mut doc, "Styled", fx_text(font.clone(), "WTE 2026", fx.clone()), [64.0, 64.0]);
    let arc = TextPath { shape: PathShape::Arc { bend: 0.3 }, start: 0.0, flip: false };
    // The gap moves to the first character: "Ag" has no character 2 (on base4 `add_stamp` does not sanitize yet).
    let spec3d = fx_text(font, "Ag", TextFx { gaps: vec![Gap { after: 0, em: 0.4 }], warp: None, path: Some(arc), ..fx });
    let p = Projector::from_surface(Vec3::new(0.5, 0.3, 0.0), Vec3::Z, Vec3::Y, Vec2::new(0.3, 0.1), 0.2);
    let d3 = doc.add_stamp("Styled 3D", StampSource::Text(spec3d), Placement::Projected(p), TextureSetId(0)).unwrap();
    for id in [uv, d3] {
        assert!(!doc.layer(doc.stamp_layers(id)[0].1).unwrap().content_rect().is_empty(), "{id:?} baked");
    }
    let dir = temp_dir("roundtrip_styled_export");
    let before = export_pngs(&doc, &dir.join("before"));
    assert_eq!(before.keys().filter(|name| name.contains("_text_")).count(), 2, "{:?}", before.keys());

    let path = dir.join("styled.wtep");
    save_to(&doc, &path);
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    for id in [uv, d3] {
        assert_eq!(text_of(&report.document, id), text_of(&doc, id), "{id:?}: the effects survive the file");
    }
    let after = export_pngs(&report.document, &dir.join("after"));
    assert_eq!(before.keys().collect::<Vec<_>>(), after.keys().collect::<Vec<_>>());
    for (name, bytes) in &before {
        assert!(*bytes == after[name], "{name} differs after reopening");
    }

    // A file the app wrote is trusted: its stored pictures are used as they are, nothing is re-baked (the effects
    // read back equal to what was written, so the repair check of the load finds nothing to repair).
    let marked = dir.join("marked.wtep");
    with_manifest_and_solid_cache(&path, &marked, uv, |_| {});
    let report = project::load(&marked, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(has_magenta(&report.document, uv), "a clean styled text is not re-baked on load");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The pixel side of the file tests above, which needs the real painters (T1's warp, T2's styled painter; the
/// `#[ignore]` of the stub time was removed at M1 after it passed on `base4_core`).
#[test]
fn the_effects_change_the_saved_pixels() {
    let mut doc = plate_doc();
    let font = doc.default_font();
    let styled = TextFx {
        gaps: vec![Gap { after: 2, em: 0.4 }],
        bold_em: 0.03,
        slant_deg: 12.0,
        warp: Some(Warp { style: WarpStyle::Arc, bend: 0.5, horizontal: 0.0, vertical: 0.0, vertical_axis: false }),
        path: None,
    };
    let id = uv_text(&mut doc, "Styled", fx_text(font.clone(), "WTE 2026", styled), [64.0, 64.0]);
    let flag = TextFx { gaps: vec![Gap { after: 1, em: 0.5 }], warp: Some(Warp::new(WarpStyle::Flag)), ..TextFx::default() };
    let flag_id = uv_text(&mut doc, "Ribbon", fx_text(font.clone(), "WTE 2026", flag.clone()), [64.0, 64.0]);

    let mut plain = plate_doc();
    let plain_id = uv_text(&mut plain, "Styled", fx_text(font.clone(), "WTE 2026", TextFx::default()), [64.0, 64.0]);
    let unwarped_id = uv_text(&mut plain, "Ribbon", fx_text(font, "WTE 2026", TextFx { warp: None, ..flag }), [64.0, 64.0]);
    assert_eq!((plain_id, unwarped_id), (id, flag_id));
    // The reference of `an_unknown_warp_style_loads_without_warp` really differs from the warped text.
    assert!(stamp_pixels(&doc, flag_id) != stamp_pixels(&plain, flag_id), "the Flag warp changes the picture");
    // The styled picture differs from the plain one, and it is what the file stores and gives back.
    assert!(stamp_pixels(&doc, id) != stamp_pixels(&plain, id), "the effects change the picture");
    let dir = temp_dir("roundtrip_effect_pixels");
    let path = dir.join("styled.wtep");
    save_to(&doc, &path);
    let loaded = project::load(&path, &NoProgress).unwrap().document;
    assert!(stamp_pixels(&loaded, id) == stamp_pixels(&doc, id), "the styled picture is saved");
    assert!(stamp_pixels(&loaded, id) != stamp_pixels(&plain, id), "not the plain one");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ------------------------------------------------------------------------------------------------------------------
// Round 6 (package A, WA13): wrapped stamps and the Wrap options in the project file (format 3)
// ------------------------------------------------------------------------------------------------------------------

/// The `placement` object of stamp `id` in a manifest.
fn placement_of_entry(manifest: &Value, id: StampId) -> &Value {
    let stamps = manifest["stamps"].as_array().expect("a stamp list");
    &stamps.iter().find(|s| s["id"] == json!(id.0)).expect("the stamp")["placement"]
}

#[test]
fn wrapped_stamps_and_wrap_options_round_trip() {
    let mut doc = importable_doc(samples::cylinder(32, 1.0, 2.0), "cyl", 256);
    let blob = doc.add_image_blob("cells.png", png_bytes(&samples::cells_picture(64, 32))).unwrap();
    let image = StampSource::Image { blob };
    let p = Projector::from_surface(Vec3::new(1.0, 0.0, 0.0), Vec3::X, Vec3::Y, Vec2::new(0.7, 0.3), 0.7);
    let plain = doc.add_stamp("Plain", image.clone(), Placement::Projected(p), TextureSetId(0)).unwrap();
    assert_eq!(project::required_reader(&doc.snapshot()), 1, "a projected stamp without options");
    let q = Projector::from_surface(Vec3::new(-1.0, 0.2, 0.0), Vec3::NEG_X, Vec3::Y, Vec2::new(0.9, 0.3), 0.9);
    let w = Projector { mode: SurfaceMode::Wrap, wrap: WrapOptions { jump_gaps: true, overlap: true }, ..q };
    let wrapped = doc.add_stamp("Wrapped", image, Placement::Projected(w), TextureSetId(0)).unwrap();
    assert_eq!(project::required_reader(&doc.snapshot()), 3, "a wrapped stamp");
    let dir = temp_dir("roundtrip_wrapped");
    let path = dir.join("wrapped.wtep");
    save_to(&doc, &path);
    let manifest = manifest_value(&path);
    assert_eq!((&manifest["format_version"], &manifest["min_reader_version"]), (&json!(FORMAT_VERSION), &json!(3)));
    let entry = placement_of_entry(&manifest, wrapped);
    assert_eq!((&entry["mode"], &entry["wrap_gaps"], &entry["wrap_overlap"]), (&json!("wrap"), &json!(true), &json!(true)));
    let entry = placement_of_entry(&manifest, plain);
    for key in ["mode", "wrap_gaps", "wrap_overlap"] {
        assert!(entry.get(key).is_none(), "{key} written for a plain projected stamp");
    }
    // Reopened: mode, options and every float bit-exact, the stored caches used as they are (no re-bake).
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty() && !report.force_save_as, "{:?}", report.warnings);
    let mut loaded = report.document;
    assert_eq!(doc_state(&mut loaded), doc_state(&mut doc));
    assert_eq!(loaded.stamp(wrapped).unwrap().placement, Placement::Projected(w));

    // A projected stamp that only keeps the options for switching back needs reader 3 too; without options 1.
    let back = Projector { mode: SurfaceMode::Project, ..w };
    doc.set_stamp_placement(wrapped, Placement::Projected(back), Commit::Final).unwrap();
    assert_eq!(project::required_reader(&doc.snapshot()), 3, "a projected stamp keeping Wrap options");
    let only_gaps = Projector { wrap: WrapOptions { jump_gaps: true, overlap: false }, ..back };
    doc.set_stamp_placement(wrapped, Placement::Projected(only_gaps), Commit::Final).unwrap();
    assert_eq!(project::required_reader(&doc.snapshot()), 3);
    let none = Projector { wrap: WrapOptions::default(), ..back };
    doc.set_stamp_placement(wrapped, Placement::Projected(none), Commit::Final).unwrap();
    assert_eq!(project::required_reader(&doc.snapshot()), 1, "nothing of round 6 left");
    let plain_path = dir.join("plain.wtep");
    save_to(&doc, &plain_path);
    assert_eq!(manifest_value(&plain_path)["min_reader_version"], json!(1));

    // An unknown mode (a later round): loaded as projected, named, re-baked, Save As forced.
    let melt = dir.join("melt.wtep");
    with_manifest(&path, &melt, |m| {
        let stamps = m["stamps"].as_array_mut().unwrap();
        let stamp = stamps.iter_mut().find(|s| s["id"] == json!(wrapped.0)).unwrap();
        stamp["placement"]["mode"] = json!("melt");
    });
    let report = project::load(&melt, &NoProgress).unwrap();
    let unknown = LoadWarning::StampFeatureUnknown { stamp: wrapped, what: "surface mode \"melt\"".to_owned() };
    assert_eq!(report.warnings, vec![unknown.clone()]);
    assert!(report.force_save_as);
    assert!(unknown.to_string().contains("surface mode \"melt\""), "{unknown}");
    let Placement::Projected(melted) = report.document.stamp(wrapped).unwrap().placement else { panic!("not projected") };
    assert_eq!(melted.mode, SurfaceMode::Project);
    assert_eq!(melted.wrap, WrapOptions { jump_gaps: true, overlap: true }, "the options are kept");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// check:A (round 6): a reopened wrapped stamp (Overlap, 1.5 x round) shows the saved caches; its parameterization is
/// built on demand (`ensure_wrap_param`) with the stats of the original; later edits (tint, a move) bake the same bytes
/// as in the original document.
#[test]
fn reopened_wrapped_stamps_rebuild_their_parameterization_exactly() {
    let mut doc = importable_doc(samples::cylinder(32, 1.0, 2.0), "cyl", 256);
    let blob = doc.add_image_blob("cells.png", png_bytes(&samples::cells_picture(64, 32))).unwrap();
    let q = Projector::from_surface(Vec3::new(-1.0, 0.2, 0.0), Vec3::NEG_X, Vec3::Y, Vec2::new(4.5, 0.3), 0.9);
    let w = Projector { mode: SurfaceMode::Wrap, wrap: WrapOptions { jump_gaps: false, overlap: true }, ..q };
    let id = doc.add_stamp("Wrapped", StampSource::Image { blob }, Placement::Projected(w), TextureSetId(0)).unwrap();
    let stats = doc.stamp_wrap_stats(id).expect("the bake cached its parameterization");
    assert_eq!(stats.turns, 3, "{stats:?}");
    let dir = temp_dir("roundtrip_wrapped_reopen");
    let path = dir.join("reopen.wtep");
    save_to(&doc, &path);
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let mut loaded = report.document;
    assert_eq!(doc_state(&mut loaded), doc_state(&mut doc));
    assert!(loaded.stamp_wrap_stats(id).is_none(), "a loaded document has no parameterization yet");
    assert!(loaded.stamp_wrap_outline(id, 8).is_empty());
    let ensured = loaded.ensure_wrap_param(id).expect("ensure_wrap_param");
    let same = |a: WrapStats, b: WrapStats| WrapStats { micros: 0, ..a } == WrapStats { micros: 0, ..b };
    assert!(same(ensured, stats), "{ensured:?} vs {stats:?}");
    assert!(same(loaded.stamp_wrap_stats(id).unwrap(), stats));
    assert!(!loaded.stamp_wrap_outline(id, 8).is_empty());
    for d in [&mut doc, &mut loaded] {
        d.set_stamp_tint(id, [255, 180, 90, 255], Commit::Final).unwrap();
    }
    assert_eq!(doc_state(&mut loaded), doc_state(&mut doc), "a tint after reopening bakes differently");
    let moved = Projector { origin: w.origin + Vec3::Y * 0.1, ..w };
    for d in [&mut doc, &mut loaded] {
        d.set_stamp_placement(id, Placement::Projected(moved), Commit::Final).unwrap();
    }
    assert_eq!(doc_state(&mut loaded), doc_state(&mut doc), "a move after reopening bakes differently");
    std::fs::remove_dir_all(&dir).unwrap();
}
