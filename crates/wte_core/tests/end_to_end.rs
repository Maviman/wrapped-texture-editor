// Owner: core-integration (C7). Whole-pipeline tests of ARCHITECTURE.md 9.1 ("integration", end_to_end.rs): the built-in
// samples, a 3D stroke across the material boundary, image and text stamps placed in 3D and in UV space, gpu-dirty
// coverage, save -> load, export of every format (re-read and checked), undo-all / redo-all, randomized command
// sequences, history growth after property toggles, and the stroke guard of the stamp commands.
//
// Only the public API is used. Temporary files go through `testutil::temp_dir` (under target\ on E:).

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::sync::Arc;

use wte_core::glam::{Vec2, Vec3};
use wte_core::import::{import_from_bundle, normalize_ref};
use wte_core::testutil::{XorShift, doc_from_raw, temp_dir};
use wte_core::text::{EmMap, place_anchor_at_centre};
use wte_core::*;
use wte_core::{export, project, samples};

// ------------------------------------------------------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------------------------------------------------------

fn fnv(bytes: &[u8]) -> u64 {
    fnv1a64(0xcbf2_9ce4_8422_2325, bytes)
}

/// Everything about a layer that undo / redo / save / load must reproduce (pixels as a hash).
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
    layers: Vec<LayerState>,
    composite: u64,
}

/// The observable document content. The active layer is deliberately left out: `set_active_layer` produces no
/// history and undoing a pixel entry does not restore it (5.17, 6.7), so it is not part of what undo must reproduce.
#[derive(Clone, PartialEq, Debug)]
struct DocState {
    sets: Vec<SetState>,
    stamps: Vec<StampObject>,
}

/// Captures the document after bringing the composites up to date, and checks that the composite equals a fresh
/// flatten (no change was left unmarked).
fn doc_state(doc: &mut Document) -> DocState {
    doc.recomposite_all();
    let sets = doc
        .set_ids()
        .into_iter()
        .map(|id| {
            let set = doc.set(id).unwrap();
            let flat = doc.flatten(id, false).unwrap();
            assert!(*set.composite() == flat, "set {id:?}: the composite is stale (a change was not marked dirty)");
            SetState {
                layers: set
                    .layers
                    .iter()
                    .map(|l| {
                        let dense = l.to_dense(set.width, set.height);
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
                    })
                    .collect(),
                composite: testutil::hash_pixels(set.composite()),
            }
        })
        .collect();
    DocState { sets, stamps: doc.stamps.clone() }
}

fn composites(doc: &Document) -> Vec<PixelBuf> {
    doc.set_ids().into_iter().map(|s| doc.composite(s).unwrap().clone()).collect()
}

/// Recomposites, then checks for every set that the rects of `take_gpu_dirty` cover every composite pixel that
/// differs from `before`. Returns the new composites.
fn check_gpu_dirty(doc: &mut Document, before: &[PixelBuf], step: &str) -> Vec<PixelBuf> {
    doc.recomposite_all();
    assert!(!doc.has_pending_composite(), "{step}: composite still pending after recomposite_all");
    let mut after = Vec::new();
    for (i, set) in doc.set_ids().into_iter().enumerate() {
        let rects = doc.take_gpu_dirty(set);
        let now = doc.composite(set).unwrap().clone();
        assert!(now == doc.flatten(set, false).unwrap(), "{step}: composite of set {i} differs from flatten");
        let old = &before[i];
        assert_eq!((old.width, old.height), (now.width, now.height));
        for y in 0..now.height {
            if old.row(y) == now.row(y) {
                continue;
            }
            for x in 0..now.width {
                if old.get(x, y) != now.get(x, y) {
                    assert!(
                        rects.iter().any(|r| r.contains(x as i32, y as i32)),
                        "{step}: set {i} pixel ({x}, {y}) changed but is outside every gpu-dirty rect {rects:?}"
                    );
                }
            }
        }
        after.push(now);
    }
    after
}

/// Recomposites and empties the gpu-dirty lists.
fn drain_gpu_dirty(doc: &mut Document) -> Vec<PixelBuf> {
    doc.recomposite_all();
    for set in doc.set_ids() {
        doc.take_gpu_dirty(set);
    }
    composites(doc)
}

fn png_bytes(image: &PixelBuf) -> Arc<[u8]> {
    Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
}

fn dense(layer: &Layer, width: u32, height: u32) -> PixelBuf {
    layer.to_dense(width, height)
}

/// Equal alpha everywhere and equal RGB wherever alpha > 0 (the layered formats crop to `content_bounds`, so RGB
/// stored under alpha 0 outside the content box is not carried).
fn assert_visible_equal(a: &PixelBuf, b: &PixelBuf, what: &str) {
    assert_eq!((a.width, a.height), (b.width, b.height), "{what}: size");
    for (i, (p, q)) in a.data.as_chunks::<4>().0.iter().zip(b.data.as_chunks::<4>().0).enumerate() {
        let same = p[3] == q[3] && (p[3] == 0 || p == q);
        assert!(same, "{what}: pixel {} differs: {p:?} vs {q:?}", i);
    }
}

fn decode(bytes: &[u8], name: &str) -> PixelBuf {
    decode_image(bytes, Some(name), 16384).unwrap_or_else(|e| panic!("{name}: {e}")).pixels
}

/// A re-importable two-material document (OBJ + MTL + checker PNGs of `side` px), like `Document::sample` but small.
fn small_doc(side: u32) -> Document {
    let raw = samples::two_material_model();
    let names = vec!["body.png".to_owned(), "head.png".to_owned()];
    let (obj, mtl) = samples::to_obj(&raw, "model.mtl", &names);
    let file = |name: &str, bytes: Vec<u8>| SourceFile {
        key: normalize_ref(name),
        original_name: name.to_owned(),
        bytes: Arc::from(bytes),
    };
    let mut sidecars = vec![file("model.mtl", mtl.into_bytes())];
    for (i, name) in names.iter().enumerate() {
        let image = samples::checker_texture(side, side, 8 + 4 * i as u32);
        sidecars.push(file(name, encode_png(side, side, &image.data, PngLevel::Fast).unwrap()));
    }
    let bundle = SourceBundle {
        format: ModelFormat::Obj,
        main: file("model.obj", obj.into_bytes()),
        sidecars,
        extras: Vec::new(),
        original_path: None,
    };
    let model = import_from_bundle(&bundle, &ImportOptions::default(), true, &NoProgress).unwrap();
    let choices = vec![TextureChoice::Detected; model.mesh.sets.len()];
    Document::from_import(model, choices, &NoProgress).unwrap()
}

fn ray_hit(doc: &Document, origin: Vec3, target: Vec3) -> Option<Hit> {
    doc.raycast(&Ray::new(origin, target - origin))
}

/// Hits of rays shot from the front (+Z) at x along y0..y1.
fn front_hits(doc: &Document, x: f32, y0: f32, y1: f32, n: usize) -> Vec<Hit> {
    (0..n)
        .filter_map(|i| {
            let y = y0 + (y1 - y0) * i as f32 / (n - 1) as f32;
            doc.raycast(&Ray::new(Vec3::new(x, y, 3.0), Vec3::NEG_Z))
        })
        .collect()
}

fn text_spec(doc: &mut Document, content: &str) -> TextSpec {
    TextSpec {
        content: content.to_owned(),
        font: doc.default_font(),
        size_px: TEXT_PLACEMENT_PX,
        color: [30, 40, 170, 255],
        align: TextAlign::Center,
        line_spacing: 1.0,
        letter_spacing: 0.02,
        outline: Some(Outline { width_em: 0.06, color: [255, 255, 255, 255] }),
        fx: TextFx::default(),
    }
}

fn layer_of(doc: &Document, set: TextureSetId, name: &str) -> LayerId {
    doc.set(set).unwrap().layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no layer {name}")).id
}

fn is_blank(layer: &Layer) -> bool {
    layer.views().iter().all(|v| v.data.iter().all(|&b| b == 0))
}

fn stamp_layer(doc: &Document, stamp: StampId, set: TextureSetId) -> &Layer {
    let (_, id) = *doc.stamp_layers(stamp).iter().find(|(s, _)| *s == set).expect("linked layer in that set");
    doc.layer(id).unwrap()
}

fn read_zip_entry(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut entry = zip.by_name(name).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut out = Vec::new();
    entry.read_to_end(&mut out).unwrap();
    out
}

/// Value of attribute `key` in one `<layer .../>` line of an ORA stack.xml, unescaped.
fn xml_attr(line: &str, key: &str) -> String {
    let pat = format!(" {key}=\"");
    let start = line.find(&pat).unwrap_or_else(|| panic!("no {key} in {line}")) + pat.len();
    let end = start + line[start..].find('"').unwrap();
    line[start..end]
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// ------------------------------------------------------------------------------------------------------------------
// Samples
// ------------------------------------------------------------------------------------------------------------------

#[test]
fn samples_of_every_kind() {
    for (kind, sets) in [(SampleKind::Sphere, 1), (SampleKind::Cube, 1), (SampleKind::TwoMaterial, 2)] {
        let mut doc = Document::sample(kind);
        assert_eq!(doc.sets.len(), sets, "{kind:?}");
        assert_eq!(doc.mesh.sets.len(), sets, "{kind:?}");
        assert!(doc.import_warnings.is_empty(), "{kind:?}: {:?}", doc.import_warnings);
        assert!(!doc.is_modified() && !doc.can_undo() && !doc.can_redo(), "{kind:?}");
        for set in &doc.sets {
            assert_eq!(set.layers.len(), 2, "{kind:?}");
            let (base, paint) = (&set.layers[0], &set.layers[1]);
            assert_eq!((base.name.as_str(), paint.name.as_str()), ("Base", "Paint 1"));
            assert!(base.locked && base.is_raster() && !is_blank(base), "{kind:?}: Base");
            assert!(!paint.locked && paint.is_raster() && is_blank(paint), "{kind:?}: Paint 1");
            assert_eq!(set.active, Some(paint.id), "{kind:?}");
            assert_eq!((set.width, set.height), (1024, 1024));
            // The untouched composite is the imported texture itself (export(import(x)) == x).
            assert!(*set.composite() == dense(base, 1024, 1024), "{kind:?}: composite == Base texture");
        }
        doc.recomposite_all();
        for set in doc.set_ids() {
            assert_eq!(doc.paint_target(set), PaintTarget::Ready(doc.set(set).unwrap().layers[1].id));
        }
    }
}

// ------------------------------------------------------------------------------------------------------------------
// The whole pipeline on the two-material sample
// ------------------------------------------------------------------------------------------------------------------

#[test]
fn two_material_pipeline() {
    let mut doc = Document::sample(SampleKind::TwoMaterial);
    let (body, head) = (TextureSetId(0), TextureSetId(1));
    let initial_state = doc_state(&mut doc);
    let initial = drain_gpu_dirty(&mut doc);
    let mut current = initial.clone();
    let mut entries = 0usize;

    // --- 3D stroke from the cube's front face up across the boundary onto the head ---
    let hits = front_hits(&doc, 0.0, 0.25, 1.0, 120);
    assert!(hits.iter().any(|h| h.texture_set == Some(body)) && hits.iter().any(|h| h.texture_set == Some(head)));
    let brush = BrushParams { radius: 0.06, color: [220, 30, 30, 255], ..BrushParams::default() };
    doc.stroke_begin(&brush).unwrap();
    for hit in &hits {
        doc.stroke_to_3d(hit, 1.0);
    }
    assert!(doc.stroke_skipped_sets().is_empty());
    assert!(doc.stroke_end());
    entries += 1;
    for set in [body, head] {
        let paint = doc.layer(layer_of(&doc, set, "Paint 1")).unwrap();
        assert!(!is_blank(paint), "the stroke did not reach set {set:?}");
    }
    current = check_gpu_dirty(&mut doc, &current, "3D stroke");
    assert!(current[0] != initial[0] && current[1] != initial[1], "both composites must change");

    // --- image stamp, projected in 3D onto the cube's front ---
    let blob = doc.add_image_blob("logo.png", png_bytes(&samples::sample_stamp_image(128))).unwrap();
    let hit = ray_hit(&doc, Vec3::new(0.2, -0.15, 3.0), Vec3::new(0.2, -0.15, 0.0)).unwrap();
    assert_eq!(hit.texture_set, Some(body));
    let projector = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::splat(0.2), 0.25);
    let logo = doc.add_stamp("Logo", StampSource::Image { blob }, Placement::Projected(projector), body).unwrap();
    entries += 1;
    assert!(!stamp_layer(&doc, logo, body).content_rect().is_empty());
    assert_eq!(stamp_layer(&doc, logo, body).kind_name(), "stamp");
    current = check_gpu_dirty(&mut doc, &current, "image stamp (3D)");

    // A drag: three Preview moves and a Final give exactly one entry.
    for k in 1..=3 {
        let moved = projector.moved_to(projector.origin + Vec3::new(0.02 * k as f32, 0.0, 0.0), Vec3::Z);
        doc.set_stamp_placement(logo, Placement::Projected(moved), Commit::Preview).unwrap();
        assert!(doc.has_pending_edit());
    }
    let moved = projector.moved_to(projector.origin + Vec3::new(0.08, 0.0, 0.0), Vec3::Z).rotated(0.3);
    doc.set_stamp_placement(logo, Placement::Projected(moved), Commit::Final).unwrap();
    entries += 1;
    assert!(!doc.has_pending_edit());
    assert_eq!(doc.history.undo_len(), entries);
    current = check_gpu_dirty(&mut doc, &current, "stamp drag");

    // --- text stamp, projected in 3D onto the head ---
    let spec = text_spec(&mut doc, "Hi\nthere");
    let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
    let hit = ray_hit(&doc, Vec3::new(0.0, 0.85, 3.0), Vec3::new(0.0, 0.85, 0.0)).unwrap();
    assert_eq!(hit.texture_set, Some(head));
    let half = Vec2::new(0.15, 0.15 * h as f32 / w as f32);
    let projector = Projector::from_surface(hit.position, hit.normal, Vec3::Y, half, 0.3);
    let label = doc.add_stamp("Label", StampSource::Text(spec), Placement::Projected(projector), head).unwrap();
    entries += 1;
    assert_eq!(stamp_layer(&doc, label, head).kind_name(), "text");
    assert!(!stamp_layer(&doc, label, head).content_rect().is_empty());
    current = check_gpu_dirty(&mut doc, &current, "text stamp (3D)");

    // --- image stamp in UV space of the head set, rotated and flipped ---
    let uv = UvPlacement {
        set: head,
        center_px: Vec2::new(300.0, 600.0),
        size_px: Vec2::new(200.0, 200.0),
        rotation: 0.6,
        flip_x: true,
        flip_y: false,
        clip_to_islands: false,
    };
    let decal = doc.add_stamp("Decal", StampSource::Image { blob }, Placement::Uv(uv), head).unwrap();
    entries += 1;
    assert!(!stamp_layer(&doc, decal, head).content_rect().is_empty());
    current = check_gpu_dirty(&mut doc, &current, "image stamp (UV)");

    // --- text stamp in UV space of the body set, clipped to the islands ---
    let spec = text_spec(&mut doc, "WTE");
    let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
    let uv = UvPlacement {
        set: body,
        center_px: Vec2::new(512.0, 200.0),
        size_px: Vec2::new(300.0, 300.0 * h as f32 / w as f32),
        rotation: -0.3,
        flip_x: false,
        flip_y: true,
        clip_to_islands: true,
    };
    let caption = doc.add_stamp("Caption", StampSource::Text(spec), Placement::Uv(uv), body).unwrap();
    entries += 1;
    assert!(!stamp_layer(&doc, caption, body).content_rect().is_empty());
    current = check_gpu_dirty(&mut doc, &current, "text stamp (UV)");

    // --- a few layer properties, so the exports carry them ---
    let label_layer = stamp_layer(&doc, label, head).id;
    doc.set_layer_opacity(label_layer, 0.8, Commit::Final).unwrap();
    doc.set_layer_blend(stamp_layer(&doc, decal, head).id, BlendMode::Multiply).unwrap();
    entries += 2;
    doc.set_stamp_tint(logo, [255, 200, 200, 255], Commit::Final).unwrap();
    entries += 1;
    assert_eq!(doc.history.undo_len(), entries);
    current = check_gpu_dirty(&mut doc, &current, "properties");
    let final_state = doc_state(&mut doc);

    // --- save -> load ---
    let dir = temp_dir("e2e_pipeline");
    let path = dir.join("pipeline.wtep");
    let save = SaveOptions {
        png: PngLevel::Fast,
        write_flattened: true,
        keep_backup: false,
        now_unix: 1_700_000_000,
        app_version: "integration".to_owned(),
    };
    project::save(&doc.snapshot(), &path, &save, &NoProgress).unwrap();
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let mut loaded = report.document;
    assert_eq!(doc_state(&mut loaded), final_state);
    assert!(composites(&loaded) == current, "composites after load");
    for set in doc.set_ids() {
        assert_eq!(loaded.set(set).unwrap().active, doc.set(set).unwrap().active);
    }
    assert!(!loaded.is_modified() && !loaded.can_undo());

    // --- export of every format ---
    let out = dir.join("export");
    let snapshot = doc.snapshot();
    let opts = ExportOptions {
        dir: out.clone(),
        base_name: "robot".to_owned(),
        sets: None,
        flat: Some(FlatFormat::Png(PngLevel::Balanced)),
        per_layer_png: true,
        ora: true,
        psd: true,
        include_hidden: true,
        dilate_px: 0,
        layers: None,
        package: wte_core::ExportPackage::Files,
    };
    let planned = export::plan(&snapshot, &opts);
    let written = export::run(&snapshot, &opts, &NoProgress).unwrap();
    assert_eq!(written.files, planned);
    for set in &snapshot.sets {
        let (w, h) = (set.width, set.height);
        let composite = doc.composite(set.id).unwrap();
        let flat = std::fs::read(out.join(export::flat_name("robot", &set.name, true, "png"))).unwrap();
        assert!(decode(&flat, "flat.png") == *composite, "flattened PNG of {}", set.name);

        // Per-layer PNGs: one full-canvas file per layer, stamps and texts as separate files (R4).
        let mut kinds = Vec::new();
        for (i, layer) in set.layers.iter().enumerate() {
            let name = export::layer_name("robot", &set.name, true, i, layer.kind_name(), &layer.name);
            let file = std::fs::read(out.join(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(decode(&file, &name) == dense(layer, w, h), "{name}");
            kinds.push(layer.kind_name());
        }
        assert!(kinds.contains(&"stamp") && kinds.contains(&"text"), "set {}: {kinds:?}", set.name);

        // OpenRaster.
        let ora = std::fs::read(out.join(export::layered_name("robot", &set.name, true, "ora"))).unwrap();
        assert_eq!(read_zip_entry(&ora, "mimetype"), b"image/openraster");
        let stack = String::from_utf8(read_zip_entry(&ora, "stack.xml")).unwrap();
        let lines: Vec<&str> = stack.lines().filter(|l| l.trim_start().starts_with("<layer ")).collect();
        assert_eq!(lines.len(), set.layers.len());
        for (line, layer) in lines.iter().zip(set.layers.iter().rev()) {
            assert_eq!(xml_attr(line, "name"), layer.name);
            assert_eq!(xml_attr(line, "visibility"), if layer.visible { "visible" } else { "hidden" });
            assert_eq!(xml_attr(line, "composite-op"), layer.blend.ora_op());
            assert_eq!(xml_attr(line, "opacity"), format!("{:.3}", layer.opacity));
            let src = xml_attr(line, "src");
            let png = decode(&read_zip_entry(&ora, &src), &src);
            let (x, y) = (xml_attr(line, "x").parse::<i32>().unwrap(), xml_attr(line, "y").parse::<i32>().unwrap());
            let placed = CroppedBuf { rect: IRect::from_xywh(x, y, png.width, png.height), data: png.data }.to_dense(w, h);
            assert_visible_equal(&placed, &dense(layer, w, h), &format!("ORA layer {}", layer.name));
        }
        assert!(decode(&read_zip_entry(&ora, "mergedimage.png"), "merged.png") == *composite, "ORA merged image");

        // PSD.
        let psd = std::fs::read(out.join(export::layered_name("robot", &set.name, true, "psd"))).unwrap();
        let parsed = export::read_psd(&psd).unwrap();
        assert_eq!((parsed.width, parsed.height), (w, h));
        assert_eq!(parsed.layers.len(), set.layers.len());
        for (p, layer) in parsed.layers.iter().zip(&set.layers) {
            assert_eq!(p.unicode_name, layer.name);
            assert_eq!(p.hidden, !layer.visible);
            assert_eq!(p.opacity, (layer.opacity * 255.0).round() as u8);
            assert_eq!(p.blend_key, layer.blend.psd_key());
            assert_visible_equal(&p.pixels.to_dense(w, h), &dense(layer, w, h), &format!("PSD layer {}", layer.name));
        }
        assert!(parsed.merged == *composite, "PSD merged image (the composite is opaque)");
    }

    // The same export as one zip package: its entries are the files just checked, in plan order, byte for byte.
    let zip_opts = ExportOptions { dir: dir.join("package"), package: wte_core::ExportPackage::Zip, ..opts.clone() };
    let packed = export::run(&snapshot, &zip_opts, &NoProgress).unwrap();
    assert_eq!(packed.files, [dir.join("package").join("robot.zip")]);
    let zip_entries = export::read_package(&std::fs::read(&packed.files[0]).unwrap()).unwrap();
    assert_eq!(packed.packaged, zip_entries.len());
    assert_eq!(zip_entries.len(), planned.len());
    for ((name, bytes), path) in zip_entries.iter().zip(&planned) {
        assert_eq!(name.as_str(), path.file_name().unwrap().to_str().unwrap());
        assert!(*bytes == std::fs::read(path).unwrap(), "zip entry {name} differs from the loose file");
    }

    // A layer selection: the stamp and text layers of the body set.
    let body_set = snapshot.sets.iter().find(|s| s.id == body).unwrap();
    let picked: Vec<usize> = body_set.layers.iter().enumerate().filter(|(_, l)| !l.is_raster()).map(|(i, _)| i).collect();
    assert!(picked.len() >= 2, "the body set holds a stamp and a text layer");
    let ids: Vec<_> = picked.iter().map(|&i| body_set.layers[i].id).collect();
    let sel_dir = dir.join("selection");
    let sel_opts = ExportOptions {
        dir: sel_dir.clone(),
        flat: None,
        ora: false,
        psd: false,
        layers: Some(ids.clone()),
        ..opts.clone()
    };
    let sel = export::run(&snapshot, &sel_opts, &NoProgress).unwrap();
    assert_eq!(sel.files.len(), ids.len());
    let mut listing: Vec<String> =
        std::fs::read_dir(&sel_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    listing.sort();
    let mut wanted: Vec<String> = picked
        .iter()
        .map(|&i| {
            let layer = &body_set.layers[i];
            export::layer_name("robot", &body_set.name, true, i, layer.kind_name(), &layer.name)
        })
        .collect();
    wanted.sort();
    assert_eq!(listing, wanted, "exactly the selected layers, named with their set (the model has two sets)");
    for &i in &picked {
        let layer = &body_set.layers[i];
        let name = export::layer_name("robot", &body_set.name, true, i, layer.kind_name(), &layer.name);
        assert!(planned.iter().any(|p| p.file_name().unwrap().to_str() == Some(name.as_str())), "{name}: the full export's name");
        let file = std::fs::read(sel_dir.join(&name)).unwrap();
        assert!(decode(&file, &name) == dense(layer, body_set.width, body_set.height), "{name}");
    }

    // Every flat format.
    let formats = [
        FlatFormat::Png(PngLevel::Fast),
        FlatFormat::Jpeg { quality: 95, matte: [255, 255, 255] },
        FlatFormat::Tga,
        FlatFormat::Bmp,
        FlatFormat::Tiff,
        FlatFormat::WebpLossless,
    ];
    for (k, format) in formats.into_iter().enumerate() {
        let base = format!("flat{k}");
        let opts = ExportOptions { base_name: base.clone(), flat: Some(format), per_layer_png: false, ora: false, psd: false, ..opts.clone() };
        let written = export::run(&snapshot, &opts, &NoProgress).unwrap();
        assert_eq!(written.files.len(), 2);
        for set in &snapshot.sets {
            let name = export::flat_name(&base, &set.name, true, format.extension());
            let image = decode(&std::fs::read(out.join(&name)).unwrap(), &name);
            let composite = doc.composite(set.id).unwrap();
            if format.keeps_alpha() {
                assert!(image == *composite, "{name}: lossless export must be exact");
            } else {
                assert_eq!((image.width, image.height), (composite.width, composite.height));
                let p = testutil::psnr(&image, composite);
                assert!(p > 30.0, "{name}: JPEG PSNR {p}");
            }
        }
    }

    // --- undo everything, then redo everything ---
    let mut undone = 0;
    while doc.undo().is_some() {
        undone += 1;
    }
    assert_eq!(undone, entries);
    let after_undo = check_gpu_dirty(&mut doc, &current, "undo all");
    assert!(after_undo == initial, "composite after undo-all equals the initial one");
    assert_eq!(doc_state(&mut doc), initial_state);
    while doc.redo().is_some() {}
    let after_redo = check_gpu_dirty(&mut doc, &after_undo, "redo all");
    assert!(after_redo == current, "composite after redo-all");
    assert_eq!(doc_state(&mut doc), final_state);

    std::fs::remove_dir_all(&dir).unwrap();
}

// ------------------------------------------------------------------------------------------------------------------
// Delegation and stamp-specific history behaviour across packages
// ------------------------------------------------------------------------------------------------------------------

#[test]
fn layer_commands_on_linked_layers_act_on_the_whole_stamp() {
    let mut doc = small_doc(256);
    let (body, head) = (TextureSetId(0), TextureSetId(1));
    let blob = doc.add_image_blob("logo.png", png_bytes(&samples::sample_stamp_image(64))).unwrap();
    // A projected stamp straddling the boundary on the front: covers both sets.
    let hit = ray_hit(&doc, Vec3::new(0.0, 0.5, 3.0), Vec3::new(0.0, 0.5, 0.0)).unwrap();
    let p = Projector::from_surface(hit.position, Vec3::Z, Vec3::Y, Vec2::splat(0.3), 0.6);
    let id = doc.add_stamp("Wide", StampSource::Image { blob }, Placement::Projected(p), hit.texture_set.unwrap()).unwrap();
    let linked = doc.stamp_layers(id);
    assert_eq!(linked.iter().map(|(s, _)| *s).collect::<Vec<_>>(), vec![body, head], "linked layers in both sets");
    let start = doc_state(&mut doc);
    let n = doc.history.undo_len();

    // rename_layer on one linked layer renames the stamp and every linked layer: one entry.
    doc.rename_layer(linked[1].1, "Badge").unwrap();
    assert_eq!(doc.history.undo_len(), n + 1);
    assert_eq!(doc.stamp(id).unwrap().name, "Badge");
    assert!(doc.stamp_layers(id).iter().all(|(_, l)| doc.layer(*l).unwrap().name == "Badge"));

    // duplicate_layer duplicates the stamp: one entry, the copy's layer in the same set is returned.
    let copy_layer = doc.duplicate_layer(linked[0].1).unwrap();
    assert_eq!(doc.history.undo_len(), n + 2);
    let copy = doc.stamp_of_layer(copy_layer).unwrap();
    assert_ne!(copy, id);
    assert_eq!(doc.find_layer(copy_layer).unwrap().0, body);
    assert_eq!(doc.stamp_layers(copy).len(), 2);

    // remove_layer on a linked layer removes the whole stamp: one entry.
    doc.remove_layer(doc.stamp_layers(copy)[1].1).unwrap();
    assert_eq!(doc.history.undo_len(), n + 3);
    assert!(doc.stamp(copy).is_none() && doc.stamp_layers(copy).is_empty());

    // rasterize_layer on one set keeps the stamp in the other.
    let before_raster = doc_state(&mut doc);
    let raster = doc.stamp_layers(id)[0].1;
    doc.rasterize_layer(raster).unwrap();
    assert!(doc.layer(raster).unwrap().is_raster());
    assert_eq!(doc.stamp_layers(id).len(), 1);
    assert_eq!(doc.stamp(id).unwrap().home_set, head);
    doc.undo().unwrap();
    assert_eq!(doc_state(&mut doc), before_raster);

    for _ in 0..3 {
        doc.undo().unwrap();
    }
    assert_eq!(doc_state(&mut doc), start);
}

#[test]
fn stamp_commands_return_stroke_in_progress_during_a_stroke() {
    let mut doc = small_doc(256);
    let (body, head) = (TextureSetId(0), TextureSetId(1));
    let blob = doc.add_image_blob("logo.png", png_bytes(&samples::sample_stamp_image(64))).unwrap();
    let hit = ray_hit(&doc, Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO).unwrap();
    let p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::splat(0.2), 0.3);
    let image = doc.add_stamp("Logo", StampSource::Image { blob }, Placement::Projected(p), body).unwrap();
    let spec = text_spec(&mut doc, "abc");
    let uv = UvPlacement {
        set: head,
        center_px: Vec2::new(128.0, 128.0),
        size_px: Vec2::new(100.0, 40.0),
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        clip_to_islands: false,
    };
    let text = doc.add_stamp("Text", StampSource::Text(spec.clone()), Placement::Uv(uv), head).unwrap();
    // The stamps became active; paint on Paint 1 again.
    doc.set_active_layer(body, Some(layer_of(&doc, body, "Paint 1"))).unwrap();
    let before = doc_state(&mut doc);
    let (undo, redo, revision) = (doc.history.undo_len(), doc.history.redo_len(), doc.revision());

    doc.stroke_begin(&BrushParams { radius: 0.05, ..BrushParams::default() }).unwrap();
    doc.stroke_to_3d(&hit, 1.0);
    let in_progress = |r: Result<(), EditError>, what: &str| {
        assert!(matches!(r, Err(EditError::StrokeInProgress)), "{what}: {r:?}");
    };
    let image_layer = stamp_layer(&doc, image, body).id;
    let text_layer = stamp_layer(&doc, text, head).id;
    in_progress(
        doc.add_stamp("New", StampSource::Image { blob }, Placement::Projected(p), body).map(|_| ()),
        "add_stamp",
    );
    for commit in [Commit::Preview, Commit::Final] {
        in_progress(doc.set_stamp_placement(image, Placement::Projected(p.rotated(0.5)), commit), "set_stamp_placement");
        let mut changed = spec.clone();
        changed.content = "xyz".to_owned();
        in_progress(doc.set_stamp_source(text, StampSource::Text(changed), commit), "set_stamp_source");
        in_progress(doc.set_stamp_tint(image, [1, 2, 3, 4], commit), "set_stamp_tint");
    }
    in_progress(doc.rename_stamp(image, "Other"), "rename_stamp");
    in_progress(doc.remove_stamp(text), "remove_stamp");
    in_progress(doc.duplicate_stamp(image).map(|_| ()), "duplicate_stamp");
    in_progress(doc.remove_layer(image_layer), "remove_layer (stamp)");
    in_progress(doc.duplicate_layer(text_layer).map(|_| ()), "duplicate_layer (text)");
    in_progress(doc.rename_layer(text_layer, "x"), "rename_layer (text)");
    in_progress(doc.rasterize_layer(image_layer), "rasterize_layer");
    in_progress(doc.merge_down(text_layer).map(|_| ()), "merge_down");
    assert!(doc.undo().is_none() && doc.redo().is_none());
    assert!(!doc.has_pending_edit());
    assert!(doc.stroke_active());
    doc.stroke_cancel();

    assert_eq!(doc_state(&mut doc), before);
    assert_eq!((doc.history.undo_len(), doc.history.redo_len(), doc.revision()), (undo, redo, revision));
    // After the stroke the same commands work again.
    doc.rename_stamp(image, "Other").unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1);
}

#[test]
fn preview_stamp_edit_flushed_by_stroke_begin_is_full_quality() {
    fn setup() -> (Document, StampId, Projector) {
        let mut doc = doc_from_raw(samples::occluded_quads(), 256);
        let blob = doc.add_image_blob("logo.png", png_bytes(&samples::sample_stamp_image(64))).unwrap();
        let p = Projector::from_surface(Vec3::new(0.8, 0.8, 0.0), Vec3::Z, Vec3::Y, Vec2::splat(0.4), 1.0);
        let id = doc.add_stamp("Logo", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
        doc.set_active_layer(TextureSetId(0), Some(LayerId(1))).unwrap();
        (doc, id, p)
    }
    let moved = |p: &Projector| p.moved_to(Vec3::new(0.9, 0.9, 0.0), Vec3::Z);

    // Reference: the same edit committed with Final.
    let (mut reference, id, p) = setup();
    reference.set_stamp_placement(id, Placement::Projected(moved(&p)), Commit::Final).unwrap();
    let expected = doc_state(&mut reference);

    let (mut doc, id, p) = setup();
    doc.set_stamp_placement(id, Placement::Projected(moved(&p)), Commit::Preview).unwrap();
    assert!(doc.has_pending_edit());
    let preview = doc_state(&mut doc);
    assert_ne!(preview, expected, "Preview (no occlusion) must differ from Full on occluded_quads");

    doc.stroke_begin(&BrushParams { radius: 0.05, ..BrushParams::default() }).unwrap();
    assert!(!doc.has_pending_edit(), "stroke_begin flushes the pending edit");
    doc.stroke_cancel();
    assert_eq!(doc_state(&mut doc), expected, "the flushed caches are the Full-quality ones");
    assert_eq!(doc.history.undo_len(), reference.history.undo_len());
    assert_eq!(doc.undo_label(), reference.undo_label());
}

/// Round 4, R4-16 (FEATURE_TEXT 5.8, 5.11): a styled text placed as the Text tool places it (anchor at the click),
/// typed into with deferred Preview bakes, committed, saved and reopened. The anchor stays on the clicked point
/// through every step; the typing is one history entry; after the load the frame comes from geometry and equals the
/// one before; undo restores text, placement and pixels.
#[test]
fn a_styled_text_keeps_its_anchor_through_typing_save_and_load() {
    let mut doc = small_doc(128);
    let diagonal = doc.mesh.bounds.diagonal();
    let hit = front_hits(&doc, 0.0, 0.3, 0.7, 5)[2];
    let mut spec = text_spec(&mut doc, "WTE");
    spec.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Arc) });
    spec.fx.bold_em = 0.02;
    let source = StampSource::Text(spec.clone());
    let frame = doc.stamp_source_frame(&source).unwrap();
    assert_ne!(frame.anchor_px, [frame.size[0] as f32 / 2.0, frame.size[1] as f32 / 2.0], "an Arc's anchor is off-centre");
    // One em = 8 % of the model's diagonal; the rectangle centred on the click, then moved so the anchor is there.
    let em_world = 0.08 * diagonal;
    let half = Vec2::new(frame.size[0] as f32, frame.size[1] as f32) * em_world / frame.em_px / 2.0;
    let p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, half, 0.0);
    let mut placement = Placement::Projected(doc.fit_stamp_depth(&p));
    place_anchor_at_centre(&mut placement, &frame);
    let id = doc.add_stamp("WTE", source, placement, TextureSetId(0)).unwrap();
    let anchor = |doc: &mut Document| {
        let frame = doc.stamp_text_frame(id).unwrap();
        EmMap::new(&doc.stamp(id).unwrap().placement, &frame).to_world([0.0, 0.0])
    };
    let near = |a: Vec3, b: Vec3| a.distance(b) < 1e-4 * diagonal;
    assert!(near(anchor(&mut doc), hit.position), "the anchor is on the clicked point");
    let entries = doc.history.undo_len();
    let before = doc_state(&mut doc);

    // Typing with deferred Preview bakes (as the app does).
    doc.set_defer_preview_bakes(true);
    let mut typed = match &doc.stamp(id).unwrap().source {
        StampSource::Text(s) => s.clone(),
        StampSource::Image { .. } => unreachable!(),
    };
    for c in [" ", "2", "0", "2", "6"] {
        typed.content.push_str(c);
        doc.set_stamp_source(id, StampSource::Text(typed.clone()), Commit::Preview).unwrap();
        assert!(doc.has_stale_previews());
        assert!(near(anchor(&mut doc), hit.position), "after typing {c:?}");
        doc.bake_stale_previews();
    }
    doc.flush_pending_edit();
    assert_eq!(doc.history.undo_len(), entries + 1);
    assert_eq!(doc.undo_label(), Some("Edit Stamp"));
    assert!(near(anchor(&mut doc), hit.position), "after the final picture");
    let frame = doc.stamp_text_frame(id).unwrap();
    let typed_state = doc_state(&mut doc);

    // Save and reopen: the frame needs only geometry and is the same.
    let dir = temp_dir("e2e_styled_text");
    let path = dir.join("styled.wtep");
    let save = SaveOptions {
        png: PngLevel::Fast,
        write_flattened: false,
        keep_backup: false,
        now_unix: 7,
        app_version: "integration".to_owned(),
    };
    project::save(&doc.snapshot(), &path, &save, &NoProgress).unwrap();
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let mut loaded = report.document;
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(loaded.cached_text_frame(id), None);
    assert_eq!(loaded.stamp_text_frame(id).unwrap(), frame);
    assert_eq!(doc_state(&mut loaded), typed_state);

    // Undo restores text, placement and pixels.
    doc.undo();
    assert_eq!(doc_state(&mut doc), before);
    assert!(near(anchor(&mut doc), hit.position));
}

#[test]
fn property_toggles_do_not_pin_pixels_and_a_stroke_grows_history_by_its_patch_only() {
    let mut doc = Document::sample(SampleKind::Cube);
    let set = TextureSetId(0);
    let base = layer_of(&doc, set, "Base");
    let paint = layer_of(&doc, set, "Paint 1");
    let buffer = |doc: &Document, id: LayerId| match &doc.layer(id).unwrap().kind {
        LayerKind::Raster(buf) => Arc::clone(buf),
        _ => unreachable!(),
    };
    // A 1024^2 layer is 4 MiB: a single pinned copy would exceed this budget and evict the toggles.
    doc.history.budget_bytes = 2 << 20;

    doc.set_layer_visible(base, false).unwrap();
    doc.set_layer_visible(paint, false).unwrap();
    doc.set_layer_visible(paint, true).unwrap();
    doc.set_layer_blend(paint, BlendMode::Screen).unwrap();
    doc.set_layer_opacity(paint, 0.7, Commit::Preview).unwrap();
    doc.set_layer_opacity(paint, 0.9, Commit::Final).unwrap();
    assert_eq!(doc.history.undo_len(), 5);
    for id in [base, paint] {
        let buf = buffer(&doc, id);
        assert_eq!(Arc::strong_count(&buf), 2, "only the layer (and this probe) may hold the pixels of {id:?}");
    }
    // "Paint 1" is unallocated until its first stroke (no memory for an untouched paint layer).
    assert!(buffer(&doc, paint).data.is_empty());
    let hits = front_hits(&doc, 0.1, -0.3, 0.3, 30);
    let brush = BrushParams { radius: 0.05, color: [0, 200, 0, 255], ..BrushParams::default() };
    doc.stroke_begin(&brush).unwrap();
    for hit in &hits {
        doc.stroke_to_3d(hit, 1.0);
    }
    assert!(doc.stroke_end());
    let allocated = buffer(&doc, paint);
    assert_eq!(allocated.data.len(), 1024 * 1024 * 4);
    assert_eq!(Arc::strong_count(&allocated), 2, "the history holds no copy of the layer, only its tile patch");
    let before_ptr = Arc::as_ptr(&allocated);
    drop(allocated);
    doc.stroke_begin(&brush).unwrap();
    for hit in hits.iter().rev() {
        doc.stroke_to_3d(hit, 0.5);
    }
    assert!(doc.stroke_end());
    assert_eq!(Arc::as_ptr(&buffer(&doc, paint)), before_ptr, "a later stroke must not deep-copy the layer");
    assert_eq!(doc.history.undo_len(), 7, "nothing evicted: the history holds the toggles and the tile patches only");

    // Undo / redo of everything still reproduces the content.
    let painted = doc_state(&mut doc);
    while doc.undo().is_some() {}
    assert!(is_blank(doc.layer(paint).unwrap()));
    while doc.redo().is_some() {}
    assert_eq!(doc_state(&mut doc), painted);
}

// ------------------------------------------------------------------------------------------------------------------
// Randomized command / stroke / stamp / undo / redo sequences
// ------------------------------------------------------------------------------------------------------------------

fn rf(rng: &mut XorShift, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rng.next_f32()
}

fn pick<T: Copy>(rng: &mut XorShift, items: &[T]) -> Option<T> {
    if items.is_empty() { None } else { Some(items[rng.next_u32() as usize % items.len()]) }
}

fn random_unit(rng: &mut XorShift) -> Vec3 {
    loop {
        let v = Vec3::new(rf(rng, -1.0, 1.0), rf(rng, -1.0, 1.0), rf(rng, -1.0, 1.0));
        let l = v.length_squared();
        if (0.05..=1.0).contains(&l) {
            return v.normalize();
        }
    }
}

fn random_hit(doc: &Document, rng: &mut XorShift) -> Option<Hit> {
    for _ in 0..16 {
        let target = Vec3::new(rf(rng, -0.4, 0.4), rf(rng, -0.3, 1.0), rf(rng, -0.4, 0.4));
        let origin = target + random_unit(rng) * 3.0;
        if let Some(hit) = ray_hit(doc, origin, target) {
            return Some(hit);
        }
    }
    None
}

fn random_brush(rng: &mut XorShift, radius: f32) -> BrushParams {
    BrushParams {
        radius,
        hardness: rf(rng, 0.0, 1.0),
        opacity: rf(rng, 0.3, 1.0),
        flow: rf(rng, 0.2, 1.0),
        spacing: rf(rng, 0.05, 0.5),
        color: [rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8, 128 + (rng.next_u32() % 128) as u8],
        mode: if rng.next_u32().is_multiple_of(5) { BrushMode::Erase } else { BrushMode::Paint },
        same_side_only: !rng.next_u32().is_multiple_of(4),
        dynamics: wte_core::PressureDynamics::default(),
    }
}

fn all_layers(doc: &Document) -> Vec<LayerId> {
    doc.sets.iter().flat_map(|s| s.layers.iter().map(|l| l.id)).collect()
}

fn random_uv_placement(doc: &Document, rng: &mut XorShift) -> UvPlacement {
    let set = pick(rng, &doc.set_ids()).unwrap();
    let side = doc.set(set).unwrap().width as f32;
    UvPlacement {
        set,
        center_px: Vec2::new(rf(rng, 0.0, side), rf(rng, 0.0, side)),
        size_px: Vec2::new(rf(rng, 10.0, side * 0.6), rf(rng, 10.0, side * 0.6)),
        rotation: rf(rng, -3.2, 3.2),
        flip_x: rng.next_u32().is_multiple_of(2),
        flip_y: rng.next_u32().is_multiple_of(2),
        clip_to_islands: rng.next_u32().is_multiple_of(2),
    }
}

fn random_projector(doc: &Document, rng: &mut XorShift) -> Option<Projector> {
    let hit = random_hit(doc, rng)?;
    let mut p = Projector::from_surface(
        hit.position,
        hit.normal,
        Vec3::Y,
        Vec2::new(rf(rng, 0.05, 0.3), rf(rng, 0.05, 0.3)),
        rf(rng, 0.1, 0.5),
    );
    p.backface_cull = !rng.next_u32().is_multiple_of(4);
    p.occlusion = !rng.next_u32().is_multiple_of(4);
    p.flip_x = rng.next_u32().is_multiple_of(3);
    Some(p.rotated(rf(rng, -1.0, 1.0)))
}

const WORDS: [&str; 5] = ["A", "Hello", "two\nlines", "", "Wrapped Texture"];

#[derive(Copy, Clone, PartialEq, Debug)]
enum OpKind {
    Command,
    Undo,
    Redo,
    Cancel,
}

/// One random operation. `pending` restricts the choice to operations that end a pending edit (or cancel it).
fn random_op(doc: &mut Document, rng: &mut XorShift, blobs: &[BlobId], pending: bool) -> (OpKind, String) {
    if pending {
        return match rng.next_u32() % 5 {
            0 => {
                doc.flush_pending_edit();
                (OpKind::Command, "flush".into())
            }
            1 => {
                assert!(doc.cancel_pending_edit());
                (OpKind::Cancel, "cancel pending".into())
            }
            2 => {
                doc.undo();
                (OpKind::Undo, "undo (flushes)".into())
            }
            _ => {
                random_stroke(doc, rng);
                (OpKind::Command, "stroke (flushes)".into())
            }
        };
    }
    let layers = all_layers(doc);
    let stamps: Vec<StampId> = doc.stamps.iter().map(|s| s.id).collect();
    let sets = doc.set_ids();
    let choice = rng.next_u32() % 30;
    let what = match choice {
        0..=3 => {
            random_stroke(doc, rng);
            "stroke"
        }
        4 => {
            let set = pick(rng, &sets).unwrap();
            let side = doc.set(set).unwrap().width as f32;
            let radius = rf(rng, 1.0, 30.0);
            if doc.stroke_begin(&random_brush(rng, radius)).is_ok() {
                for _ in 0..6 {
                    doc.stroke_to_2d(set, Vec2::new(rf(rng, -20.0, side + 20.0), rf(rng, -20.0, side + 20.0)), rf(rng, 0.2, 1.0));
                }
                if rng.next_u32().is_multiple_of(4) { doc.stroke_cancel() } else { doc.stroke_end(); }
            }
            "2D stroke"
        }
        5 => {
            let set = pick(rng, &sets).unwrap();
            if let Some(layer) = pick(rng, &doc.set(set).unwrap().layers.iter().map(|l| l.id).collect::<Vec<_>>()) {
                let scope = if rng.next_u32().is_multiple_of(2) { FillScope::Layer } else { FillScope::Island };
                let color = [rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8];
                let _ = doc.fill(set, layer, Vec2::new(rng.next_f32(), rng.next_f32()), color, scope);
            }
            "fill"
        }
        6 => {
            let _ = doc.add_raster_layer(pick(rng, &sets).unwrap(), None);
            "add layer"
        }
        7 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.remove_layer(l);
            }
            "remove layer"
        }
        8 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.duplicate_layer(l);
            }
            "duplicate layer"
        }
        9 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.move_layer(l, (rng.next_u32() % 6) as usize);
            }
            "move layer"
        }
        10 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.merge_down(l);
            }
            "merge down"
        }
        11 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.rasterize_layer(l);
            }
            "rasterize"
        }
        12 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.set_layer_visible(l, !rng.next_u32().is_multiple_of(3));
            }
            "visible"
        }
        13 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.set_layer_locked(l, rng.next_u32().is_multiple_of(3));
            }
            "lock"
        }
        14 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.set_layer_alpha_lock(l, rng.next_u32().is_multiple_of(2));
            }
            "alpha lock"
        }
        15 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.set_layer_blend(l, BlendMode::ALL[rng.next_u32() as usize % 7]);
            }
            "blend"
        }
        16 => {
            if let Some(l) = pick(rng, &layers) {
                for _ in 0..rng.next_u32() % 3 {
                    let _ = doc.set_layer_opacity(l, rf(rng, 0.0, 1.0), Commit::Preview);
                }
                if rng.next_u32().is_multiple_of(2) {
                    let _ = doc.set_layer_opacity(l, rf(rng, 0.0, 1.0), Commit::Final);
                }
            }
            "opacity"
        }
        17 => {
            if let Some(l) = pick(rng, &layers) {
                let _ = doc.rename_layer(l, &format!("L{}", rng.next_u32() % 50));
            }
            "rename layer"
        }
        18 => {
            let set = pick(rng, &sets).unwrap();
            let ids: Vec<LayerId> = doc.set(set).unwrap().layers.iter().map(|l| l.id).collect();
            let _ = doc.set_active_layer(set, pick(rng, &ids));
            "set active"
        }
        19 | 20 => {
            let source = if choice == 19 {
                StampSource::Image { blob: pick(rng, blobs).unwrap() }
            } else {
                let mut spec = text_spec(doc, pick(rng, &WORDS).unwrap());
                spec.align = [TextAlign::Left, TextAlign::Center, TextAlign::Right][rng.next_u32() as usize % 3];
                StampSource::Text(spec)
            };
            if rng.next_u32().is_multiple_of(2) {
                if let Some(p) = random_projector(doc, rng) {
                    let home = pick(rng, &sets).unwrap();
                    let _ = doc.add_stamp("S", source, Placement::Projected(p), home);
                }
            } else {
                let uv = random_uv_placement(doc, rng);
                let _ = doc.add_stamp("U", source, Placement::Uv(uv), uv.set);
            }
            "add stamp"
        }
        21 | 22 => {
            if let Some(id) = pick(rng, &stamps) {
                let placement = doc.stamp(id).unwrap().placement;
                for k in 0..1 + rng.next_u32() % 3 {
                    let next = match placement {
                        Placement::Projected(p) => match random_hit(doc, rng) {
                            Some(hit) => Placement::Projected(p.moved_to(hit.position, hit.normal).rotated(0.1 * k as f32)),
                            None => Placement::Projected(p.rotated(0.2)),
                        },
                        Placement::Uv(_) => Placement::Uv(random_uv_placement(doc, rng)),
                    };
                    let _ = doc.set_stamp_placement(id, next, Commit::Preview);
                }
                match rng.next_u32() % 3 {
                    0 => {
                        let current = doc.stamp(id).unwrap().placement;
                        let _ = doc.set_stamp_placement(id, current, Commit::Final);
                    }
                    1 => {
                        doc.flush_pending_edit();
                    }
                    _ => {} // left pending: the next operation ends it
                }
            }
            "stamp placement"
        }
        23 => {
            if let Some(id) = pick(rng, &stamps) {
                let tint = [rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8, 255];
                let commit = if rng.next_u32().is_multiple_of(2) { Commit::Final } else { Commit::Preview };
                let _ = doc.set_stamp_tint(id, tint, commit);
            }
            "stamp tint"
        }
        24 => {
            if let Some(id) = pick(rng, &stamps) {
                let source = match doc.stamp(id).unwrap().source.clone() {
                    StampSource::Image { .. } => StampSource::Image { blob: pick(rng, blobs).unwrap() },
                    StampSource::Text(mut spec) => {
                        spec.content = pick(rng, &WORDS).unwrap().to_owned();
                        spec.letter_spacing = rf(rng, 0.0, 0.2);
                        StampSource::Text(spec)
                    }
                };
                let _ = doc.set_stamp_source(id, source, Commit::Final);
            }
            "stamp source"
        }
        25 => {
            if let Some(id) = pick(rng, &stamps) {
                let _ = doc.rename_stamp(id, &format!("S{}", rng.next_u32() % 20));
            }
            "rename stamp"
        }
        26 => {
            if let Some(id) = pick(rng, &stamps) {
                let _ = doc.remove_stamp(id);
            }
            "remove stamp"
        }
        27 => {
            if let Some(id) = pick(rng, &stamps) {
                let _ = doc.duplicate_stamp(id);
            }
            "duplicate stamp"
        }
        28 => {
            doc.undo();
            return (OpKind::Undo, "undo".into());
        }
        _ => {
            doc.redo();
            return (OpKind::Redo, "redo".into());
        }
    };
    (OpKind::Command, what.into())
}

fn random_stroke(doc: &mut Document, rng: &mut XorShift) {
    let dir = random_unit(rng);
    let target = Vec3::new(rf(rng, -0.4, 0.4), rf(rng, -0.3, 1.0), rf(rng, -0.4, 0.4));
    let step = random_unit(rng) * 0.03;
    let hits: Vec<Hit> = (0..8)
        .filter_map(|i| {
            let t = target + step * i as f32;
            ray_hit(doc, t + dir * 3.0, t)
        })
        .collect();
    let radius = rf(rng, 0.02, 0.15);
    let brush = random_brush(rng, radius);
    if doc.stroke_begin(&brush).is_ok() {
        for (i, hit) in hits.iter().enumerate() {
            if i == 4 && rng.next_u32().is_multiple_of(3) {
                doc.stroke_break();
            }
            doc.stroke_to_3d(hit, rf(rng, 0.3, 1.0));
        }
        if rng.next_u32().is_multiple_of(6) {
            doc.stroke_cancel();
        } else {
            doc.stroke_end();
        }
    }
}

/// Runs one random sequence and checks that every reachable history position reproduces its recorded state.
fn run_sequence(seed: u64, ops: usize) -> BTreeSet<String> {
    let mut doc = small_doc(128);
    doc.history.budget_bytes = usize::MAX;
    doc.history.max_entries = usize::MAX;
    let blobs = vec![
        doc.add_image_blob("ring.png", png_bytes(&samples::sample_stamp_image(48))).unwrap(),
        doc.add_image_blob("checker.png", png_bytes(&samples::checker_texture(32, 24, 4))).unwrap(),
    ];
    let mut rng = XorShift::new(seed);
    // timeline[i] = the state at undo depth i (None where an intermediate state was never observed).
    let mut timeline: Vec<Option<DocState>> = vec![Some(doc_state(&mut doc))];
    let mut current = drain_gpu_dirty(&mut doc);
    let mut log: Vec<String> = Vec::new();
    let mut pending = false;
    let mut labels = BTreeSet::new();

    let compare = |timeline: &mut Vec<Option<DocState>>, at: usize, state: DocState, log: &[String]| match &timeline[at] {
        Some(expected) => {
            assert!(*expected == state, "seed {seed}: state at depth {at} differs after {:?}", &log[log.len().saturating_sub(6)..]);
        }
        None => timeline[at] = Some(state),
    };

    for _ in 0..ops {
        let n0 = doc.history.undo_len();
        let was_pending = pending;
        let (kind, what) = random_op(&mut doc, &mut rng, &blobs, was_pending);
        log.push(what);
        assert!(!doc.stroke_active());
        current = check_gpu_dirty(&mut doc, &current, log.last().unwrap());
        pending = doc.has_pending_edit();
        if pending {
            continue;
        }
        let state = doc_state(&mut doc);
        let n1 = doc.history.undo_len();
        let r1 = doc.history.redo_len();
        if n1 > n0 && kind == OpKind::Command {
            labels.insert(doc.undo_label().unwrap_or_default().to_owned());
        }
        let mut base = n0;
        if was_pending && kind != OpKind::Cancel {
            // The pending edit was flushed first: one entry whose resulting state was never observed.
            timeline.truncate(n0 + 1);
            timeline.push(None);
            base = n0 + 1;
        }
        match kind {
            OpKind::Command => {
                if n1 == base {
                    compare(&mut timeline, n1, state, &log);
                } else {
                    assert_eq!(n1, base + 1, "seed {seed}: one command pushed several entries: {log:?}");
                    timeline.truncate(base + 1);
                    timeline.push(Some(state));
                }
            }
            OpKind::Undo | OpKind::Redo | OpKind::Cancel => compare(&mut timeline, n1, state, &log),
        }
        assert_eq!(timeline.len(), n1 + r1 + 1, "seed {seed}: history bookkeeping after {log:?}");
    }
    if pending {
        doc.flush_pending_edit();
        let n = doc.history.undo_len();
        timeline.truncate(n);
        current = check_gpu_dirty(&mut doc, &current, "final flush");
        timeline.push(Some(doc_state(&mut doc)));
    }

    // Redo what the sequence left undone, undo everything, then redo everything: every recorded state comes back
    // byte for byte.
    while doc.redo().is_some() {
        let n = doc.history.undo_len();
        current = check_gpu_dirty(&mut doc, &current, "redo");
        let state = doc_state(&mut doc);
        if let Some(expected) = &timeline[n] {
            assert!(*expected == state, "seed {seed}: redo, depth {n} differs");
        }
    }
    let top = doc.history.undo_len();
    assert_eq!(top + 1, timeline.len());

    // The random document survives save -> load and every export format.
    persist_and_export(&mut doc, seed);

    while doc.undo().is_some() {
        let n = doc.history.undo_len();
        current = check_gpu_dirty(&mut doc, &current, "undo-all");
        let state = doc_state(&mut doc);
        if let Some(expected) = &timeline[n] {
            assert!(*expected == state, "seed {seed}: undo-all, depth {n} differs");
        }
    }
    assert_eq!(doc.history.undo_len(), 0);
    assert!(timeline[0].as_ref().unwrap() == &doc_state(&mut doc), "seed {seed}: undo-all does not give the initial document");
    while doc.redo().is_some() {
        let n = doc.history.undo_len();
        current = check_gpu_dirty(&mut doc, &current, "redo-all");
        let state = doc_state(&mut doc);
        if let Some(expected) = &timeline[n] {
            assert!(*expected == state, "seed {seed}: redo-all, depth {n} differs");
        }
    }
    assert_eq!(doc.history.undo_len(), top);
    labels
}

/// Saves and reloads `doc` (same content, same active layers, no warnings) and exports it in every layered format.
fn persist_and_export(doc: &mut Document, seed: u64) {
    let dir = temp_dir(&format!("e2e_random_{seed}"));
    let path = dir.join("random.wtep");
    let save = SaveOptions {
        png: PngLevel::Fast,
        write_flattened: true,
        keep_backup: false,
        now_unix: seed,
        app_version: "integration".to_owned(),
    };
    project::save(&doc.snapshot(), &path, &save, &NoProgress).unwrap();
    let report = project::load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "seed {seed}: {:?}", report.warnings);
    let mut loaded = report.document;
    assert_eq!(doc_state(&mut loaded), doc_state(doc), "seed {seed}: save -> load");
    for set in doc.set_ids() {
        assert_eq!(loaded.set(set).unwrap().active, doc.set(set).unwrap().active, "seed {seed}: active layer");
    }

    let snapshot = doc.snapshot();
    let opts = ExportOptions {
        dir: dir.join("export"),
        base_name: "random".to_owned(),
        sets: None,
        flat: Some(FlatFormat::Png(PngLevel::Fast)),
        per_layer_png: true,
        ora: true,
        psd: true,
        include_hidden: seed.is_multiple_of(2),
        dilate_px: 0,
        layers: None,
        package: wte_core::ExportPackage::Files,
    };
    let report = export::run(&snapshot, &opts, &NoProgress).unwrap();
    assert_eq!(report.files, export::plan(&snapshot, &opts));
    for set in &snapshot.sets {
        let flat = std::fs::read(opts.dir.join(export::flat_name("random", &set.name, true, "png"))).unwrap();
        assert!(decode(&flat, "flat.png") == *doc.composite(set.id).unwrap(), "seed {seed}: flattened export");
        let exported: Vec<&Layer> = set.layers.iter().filter(|l| opts.include_hidden || l.visible).collect();
        let psd = std::fs::read(opts.dir.join(export::layered_name("random", &set.name, true, "psd"))).unwrap();
        let parsed = export::read_psd(&psd).unwrap_or_else(|e| panic!("seed {seed}: PSD does not parse: {e}"));
        assert_eq!(parsed.layers.len(), exported.len());
        for (p, layer) in parsed.layers.iter().zip(&exported) {
            assert_eq!(p.unicode_name, layer.name);
            let (w, h) = (set.width, set.height);
            assert_visible_equal(&p.pixels.to_dense(w, h), &dense(layer, w, h), &format!("seed {seed}: PSD layer"));
        }
        let ora = std::fs::read(opts.dir.join(export::layered_name("random", &set.name, true, "ora"))).unwrap();
        let stack = String::from_utf8(read_zip_entry(&ora, "stack.xml")).unwrap();
        assert_eq!(stack.lines().filter(|l| l.trim_start().starts_with("<layer ")).count(), exported.len());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 6 seeds x 90 operations by default. A longer stress run: set `WTE_STRESS_SEEDS` (number of seeds) and
/// `WTE_STRESS` (operations per seed), e.g. 60 and 200 (about 3 minutes).
#[test]
fn random_sequences_restore_byte_identical_documents() {
    let env = |name: &str, default: u64| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
    let (seeds, ops) = (env("WTE_STRESS_SEEDS", 6), env("WTE_STRESS", 90) as usize);
    let mut labels = BTreeSet::new();
    for seed in 1..=seeds {
        labels.extend(run_sequence(seed, ops));
    }
    // The sequences must really exercise the commands, not only fail on invalid arguments.
    for expected in ["Brush Stroke", "Fill", "Add Stamp", "Add Text", "Edit Stamp", "Delete Stamp", "Duplicate Stamp"] {
        assert!(labels.contains(expected), "no successful {expected:?} in the random sequences: {labels:?}");
    }
    assert!(labels.len() >= 15, "too few distinct commands succeeded: {labels:?}");
}

// ------------------------------------------------------------------------------------------------------------------
// Error cases across packages
// ------------------------------------------------------------------------------------------------------------------

#[test]
fn error_cases_change_nothing() {
    let mut doc = small_doc(128);
    let blob = doc.add_image_blob("ring.png", png_bytes(&samples::sample_stamp_image(32))).unwrap();
    let hit = ray_hit(&doc, Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO).unwrap();
    let p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::splat(0.2), 0.3);
    let id = doc.add_stamp("Logo", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
    let before = doc_state(&mut doc);
    let (n, revision) = (doc.history.undo_len(), doc.revision());

    assert!(matches!(doc.add_image_blob("bad.png", Arc::from(&b"not an image"[..])), Err(EditError::Image(_))));
    let src = StampSource::Image { blob };
    assert!(matches!(
        doc.add_stamp("x", src.clone(), Placement::Projected(p), TextureSetId(9)),
        Err(EditError::UnknownSet)
    ));
    assert!(matches!(
        doc.add_stamp("x", StampSource::Image { blob: BlobId(999) }, Placement::Projected(p), TextureSetId(0)),
        Err(EditError::UnknownBlob)
    ));
    let mut nan = p;
    nan.half_size.x = f32::NAN;
    assert!(matches!(doc.add_stamp("x", src.clone(), Placement::Projected(nan), TextureSetId(0)), Err(EditError::InvalidParam(_))));
    assert!(matches!(doc.set_stamp_placement(id, Placement::Projected(nan), Commit::Final), Err(EditError::InvalidParam(_))));
    assert!(matches!(doc.set_stamp_placement(StampId(12345), Placement::Projected(p), Commit::Final), Err(EditError::UnknownStamp)));
    let spec = text_spec(&mut doc, "t");
    assert!(matches!(doc.set_stamp_source(id, StampSource::Text(spec), Commit::Final), Err(EditError::InvalidParam(_))));
    assert!(matches!(doc.remove_stamp(StampId(12345)), Err(EditError::UnknownStamp)));
    let base = layer_of(&doc, TextureSetId(0), "Base");
    assert!(matches!(doc.remove_layer(base), Err(EditError::LayerLocked)));
    assert!(matches!(doc.remove_layer(LayerId(99_999)), Err(EditError::UnknownLayer)));
    assert!(matches!(
        doc.fill(TextureSetId(0), base, Vec2::splat(0.5), [1, 2, 3, 255], FillScope::Layer),
        Err(EditError::LayerLocked)
    ));
    let paint = layer_of(&doc, TextureSetId(0), "Paint 1");
    assert!(matches!(
        doc.fill(TextureSetId(0), paint, Vec2::new(0.995, 0.995), [1, 2, 3, 255], FillScope::Island),
        Err(EditError::InvalidParam(_))
    ));
    let stamp_layer_id = stamp_layer(&doc, id, TextureSetId(0)).id;
    assert!(matches!(
        doc.fill(TextureSetId(0), stamp_layer_id, Vec2::splat(0.5), [1, 2, 3, 255], FillScope::Layer),
        Err(EditError::NotRaster)
    ));
    assert_eq!(doc_state(&mut doc), before);
    assert_eq!((doc.history.undo_len(), doc.revision()), (n, revision));

    // No paintable layer anywhere: the stroke cannot start.
    for set in doc.set_ids() {
        let paint = layer_of(&doc, set, "Paint 1");
        doc.set_layer_locked(paint, true).unwrap();
        doc.set_active_layer(set, Some(paint)).unwrap();
    }
    assert!(matches!(doc.stroke_begin(&BrushParams::default()), Err(EditError::NoPaintableLayer)));
    assert!(!doc.stroke_active());

    // Export with nothing selected.
    let dir = temp_dir("e2e_errors");
    let opts = ExportOptions {
        dir: dir.join("out"),
        base_name: "x".to_owned(),
        sets: None,
        flat: None,
        per_layer_png: false,
        ora: false,
        psd: false,
        include_hidden: false,
        dilate_px: 0,
        layers: None,
        package: wte_core::ExportPackage::Files,
    };
    assert!(export::plan(&doc.snapshot(), &opts).is_empty());
    assert!(matches!(export::run(&doc.snapshot(), &opts, &NoProgress), Err(ExportError::NothingToExport)));
    assert!(!dir.join("out").exists());
    // Loading something that is not a project.
    let junk = dir.join("junk.wtep");
    std::fs::write(&junk, b"junk").unwrap();
    assert!(matches!(project::load(&junk, &NoProgress), Err(ProjectError::NotAZip(_))));
    assert!(matches!(project::load(&dir.join("missing.wtep"), &NoProgress), Err(ProjectError::Io(_))));
    std::fs::remove_dir_all(&dir).unwrap();
}
