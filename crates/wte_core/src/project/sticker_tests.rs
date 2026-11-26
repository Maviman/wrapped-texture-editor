// Tests of package P4 (STICKER_PLAN 7 "P4", names `k4_*`): project format 4 (stickers, stamp styles, the PNG entries of
// owned pictures and masks, save reuse by picture version, load degradation) and the `.wtes` sticker file. The frozen
// version-3 project of the P0 exe is tested in `tests/project_roundtrip.rs` (`k4_a_v3_project_of_the_p0_exe_loads_as_before`).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec2;
use serde_json::{Value, json};
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::blob::BlobKind;
use crate::doc_sticker_edits::{PixelData, PixelTarget, StickerBrush};
use crate::doc_stickers::{ShareScope, StickerRef};
use crate::document::{Commit, Document, DocumentSnapshot};
use crate::error::{LoadWarning, ProjectError};
use crate::ids::{StampId, StickerId, StickerLayerId, TextureSetId};
use crate::image_io::{PngLevel, decode_image, encode_png};
use crate::layer::{BlendMode, LayerKind};
use crate::math::IRect;
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::progress::NoProgress;
use crate::project::save::testkit::{assert_same_document, cube_doc, noise_texture};
use crate::project::save::{MANIFEST_ENTRY, save, zip_deflated, zip_stored};
use crate::project::{LinkEntry, Manifest, SaveOptions, StickerContentEntry, counters, load, read_manifest, required_reader};
use crate::samples;
use crate::stamp::{BakeQuality, Placement, StampSource, UvPlacement};
use crate::sticker::adjust::Adjustment;
use crate::sticker::file::{WTES_MIMETYPE, read_wtes, write_wtes};
use crate::sticker::shape::{Paint, ShapeKind, ShapeSpec};
use crate::sticker::style::{Contour, Gradient, Shadow, Stroke, StrokePos, StylePreset, StyleStack};
use crate::sticker::{ArtboardShape, LayerMask, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::testutil::{XorShift, temp_dir};
use crate::text::{TextAlign, TextFx, TextSpec};

fn options() -> SaveOptions {
    SaveOptions { png: PngLevel::Fast, write_flattened: true, keep_backup: false, now_unix: 7, app_version: "test".to_owned() }
}

/// Saves `doc` to `path` and hands the record of the written file back to the document (as the app does).
fn save_doc(doc: &mut Document, path: &Path) {
    let stats = save(&doc.snapshot(), path, &options(), &NoProgress).unwrap();
    doc.set_png_reuse(stats.reuse);
}

fn noise(w: u32, h: u32, seed: u64, opaque: bool) -> PixelBuf {
    let mut rng = XorShift::new(seed);
    let mut p = PixelBuf::from_raw(w, h, (0..w * h * 4).map(|_| rng.next_u32() as u8).collect()).unwrap();
    if opaque {
        p.data.as_chunks_mut::<4>().0.iter_mut().for_each(|px| px[3] = 255);
    }
    p
}

fn png(p: &PixelBuf) -> Arc<[u8]> {
    Arc::from(encode_png(p.width, p.height, &p.data, PngLevel::Fast).unwrap())
}

fn ramp_mask(w: u32, h: u32) -> AlphaBuf {
    AlphaBuf::from_raw(w, h, (0..w * h).map(|i| ((i % w) * 255 / w.max(1)) as u8).collect()).unwrap()
}

fn uv(center: [f32; 2], size: [f32; 2]) -> Placement {
    Placement::Uv(UvPlacement {
        set: TextureSetId(0),
        center_px: Vec2::from_array(center),
        size_px: Vec2::from_array(size),
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        clip_to_islands: false,
    })
}

fn owned(doc: &mut Document, pixels: PixelBuf) -> PicturePixels {
    PicturePixels::Owned { pixels: Arc::new(pixels), version: doc.next_pixel_version() }
}

fn mask(doc: &mut Document, pixels: AlphaBuf) -> Option<LayerMask> {
    Some(LayerMask { pixels: Arc::new(pixels), version: doc.next_pixel_version() })
}

fn text_spec(doc: &mut Document, content: &str, fx: TextFx) -> TextSpec {
    TextSpec {
        content: content.to_owned(),
        font: doc.default_font(),
        size_px: 20.0,
        color: [250, 240, 20, 255],
        align: TextAlign::Center,
        line_spacing: 1.1,
        letter_spacing: 0.05,
        outline: None,
        fx,
    }
}

/// The cube document of the project tests (one 64 x 64 set) with nothing on it.
fn base_doc() -> Document {
    let mut doc = cube_doc(&noise_texture(64, 64, 4));
    doc.history.clear();
    doc
}

/// A sticker with every layer kind: an unedited image with a mask, an edited picture (clipped, every adjustment, a
/// stroke style, Multiply, rotated and flipped), a text layer with styles, a hidden locked star with a gradient fill and
/// an outline, a rounded rectangle; a rounded artboard that does not start at the origin.
fn every_kind_sticker(doc: &mut Document) -> StickerDoc {
    let blob = doc.add_image_blob("logo.png", png(&samples::sample_stamp_image(48))).unwrap();
    let mut s = StickerDoc::new("Every kind", 96, 64);
    s.artboard = IRect::new(-2, -3, 94, 61);
    s.shape = ArtboardShape::RoundedRect { radius: 6.5 };
    let id = s.alloc_layer();
    let mut image = StickerLayer::new(id, "Logo", StickerContent::Picture(PicturePixels::Blob(blob)));
    image.mask = mask(doc, ramp_mask(48, 48));
    image.transform.translate = Vec2::new(4.0, 4.0);
    s.layers.push(image);
    let id = s.alloc_layer();
    let mut photo = StickerLayer::new(id, "Photo", StickerContent::Picture(owned(doc, noise(40, 30, 9, true))));
    photo.clip = true;
    photo.opacity = 0.8;
    photo.blend = BlendMode::Multiply;
    photo.transform.translate = Vec2::new(30.0, 20.0);
    photo.transform.rotation = 0.3;
    photo.transform.scale = Vec2::new(1.2, -0.9);
    photo.adjustments = vec![
        Adjustment::BrightnessContrast { brightness: 12.5, contrast: -20.0 },
        Adjustment::HueSaturation { hue_deg: 33.0, saturation: -0.25, lightness: 0.125, colorize: true },
        Adjustment::Levels { in_black: 10, in_white: 240, gamma: 1.3, out_black: 5, out_white: 250 },
        Adjustment::Invert,
        Adjustment::BlackWhite,
        Adjustment::Posterize { levels: 6 },
        Adjustment::Threshold { level: 140 },
    ];
    photo.styles = StyleStack {
        stroke: Some(Stroke { on: true, width: 2.5, position: StrokePos::Inside, color: [0, 0, 255, 255], opacity: 0.75 }),
        ..StyleStack::default()
    };
    s.layers.push(photo);
    let id = s.alloc_layer();
    let spec = text_spec(doc, "Hi\nthere", TextFx::default());
    let mut text = StickerLayer::new(id, "Text", StickerContent::Text(Box::new(spec)));
    text.transform.translate = Vec2::new(48.0, 40.0);
    text.blend = BlendMode::Screen;
    text.styles = StylePreset::Badge.styles([96, 64]);
    s.layers.push(text);
    let id = s.alloc_layer();
    let star = ShapeSpec {
        kind: ShapeKind::Star { points: 7, inner: 0.45 },
        size: Vec2::new(30.0, 28.0),
        fill: Paint::Linear { gradient: Gradient::two([255, 0, 0, 255], [0, 0, 255, 128]), angle_deg: 30.0 },
        outline: Some(([10, 20, 30, 255], 1.5)),
    };
    let mut star = StickerLayer::new(id, "Star", StickerContent::Shape(star));
    star.visible = false;
    star.locked = true;
    star.transform.translate = Vec2::new(60.0, 10.0);
    s.layers.push(star);
    let id = s.alloc_layer();
    let rect = ShapeSpec { kind: ShapeKind::RoundedRect { radius: 3.0 }, size: Vec2::new(20.0, 10.0), fill: Paint::Solid([0, 200, 0, 255]), outline: None };
    let mut rect = StickerLayer::new(id, "Box", StickerContent::Shape(rect));
    rect.blend = BlendMode::Darken;
    s.layers.push(rect);
    s
}

/// Stamp styles with many members (an off entry included).
fn every_style() -> StyleStack {
    let mut s = StylePreset::Badge.styles([64, 64]);
    s.contour = Some(Contour { on: true, width: 6.0, smooth: 0.5, fill_holes: true, color: [255, 255, 255, 255], opacity: 1.0 });
    s.inner_shadow = Some(Shadow { on: false, color: [0, 0, 0, 255], opacity: 0.5, angle_deg: 120.0, distance: 3.0, spread: 0.2, size: 4.0 });
    s.sanitize();
    s
}

struct Fixture {
    doc: Document,
    /// The sticker with every layer kind, shown by `a` (styled) and `b`.
    shared: StickerId,
    /// A kept tray sticker no stamp shows (an owned picture with a mask).
    tray: StickerId,
    /// A sticker nobody shows that is not kept (a leftover).
    unused: StickerId,
    a: StampId,
    b: StampId,
    /// An image stamp with styles.
    image: StampId,
}

fn fixture() -> Fixture {
    let mut doc = base_doc();
    let sticker = every_kind_sticker(&mut doc);
    let shared = doc.add_sticker(sticker).unwrap();
    let a = doc.add_stamp("A", StampSource::Sticker { sticker: shared }, uv([20.0, 20.0], [30.0, 20.0]), TextureSetId(0)).unwrap();
    let b = doc.add_stamp("B", StampSource::Sticker { sticker: shared }, uv([44.0, 44.0], [24.0, 16.0]), TextureSetId(0)).unwrap();
    doc.set_stamp_styles(a, every_style(), Commit::Final).unwrap();
    let mut tray = StickerDoc::from_picture("Tray", owned(&mut doc, noise(20, 20, 5, false)), [20, 20]);
    tray.layers[0].mask = mask(&mut doc, ramp_mask(20, 20));
    tray.keep = true;
    let tray = doc.add_sticker(tray).unwrap();
    let leftover = StickerDoc::from_picture("Leftover", owned(&mut doc, noise(8, 8, 6, true)), [8, 8]);
    let unused = doc.add_sticker(leftover).unwrap();
    let blob = doc.add_image_blob("i.png", png(&noise(16, 16, 7, true))).unwrap();
    let image = doc.add_stamp("I", StampSource::Image { blob }, uv([50.0, 14.0], [16.0, 16.0]), TextureSetId(0)).unwrap();
    doc.set_stamp_styles(image, StylePreset::ShadowOnly.styles([16, 16]), Commit::Final).unwrap();
    doc.settle_final_bakes();
    doc.recomposite_all();
    Fixture { doc, shared, tray, unused, a, b, image }
}

/// Every entry of an archive, in order: name, compression, bytes.
fn entries(path: &Path) -> Vec<(String, CompressionMethod, Vec<u8>)> {
    let mut zip = ZipArchive::new(Cursor::new(std::fs::read(path).unwrap())).unwrap();
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

fn entry(path: &Path, name: &str) -> Vec<u8> {
    entries(path).into_iter().find(|e| e.0 == name).unwrap_or_else(|| panic!("no entry {name}")).2
}

/// Rewrites the archive at `path`; `edit` returns the new content of an entry or `None` to drop it.
fn rewrite(path: &Path, mut edit: impl FnMut(&str, Vec<u8>) -> Option<Vec<u8>>) {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, method, data) in entries(path) {
        if let Some(data) = edit(&name, data) {
            zip.start_file(name, if method == CompressionMethod::Stored { zip_stored() } else { zip_deflated() }).unwrap();
            zip.write_all(&data).unwrap();
        }
    }
    std::fs::write(path, zip.finish().unwrap().into_inner()).unwrap();
}

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

/// The sticker PNG entries (`stickers/...`) of an archive by name.
fn sticker_entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
    entries(path).into_iter().filter(|e| e.0.starts_with("stickers/")).map(|e| (e.0, e.2)).collect()
}

/// The dense linked cache of stamp `id` in set 0.
fn cache(doc: &Document, id: StampId) -> PixelBuf {
    let set = &doc.sets[0];
    set.layers.iter().find(|l| l.stamp_id() == Some(id)).map(|l| l.to_dense(set.width, set.height)).unwrap()
}

fn a_copy(from: &Path, name: &str) -> PathBuf {
    let to = from.with_file_name(name);
    std::fs::copy(from, &to).unwrap();
    to
}

// ----- required_reader -----

#[test]
fn k4_required_reader_is_4_for_each_new_feature_alone_and_unchanged_otherwise() {
    let mut doc = base_doc();
    let blob = doc.add_image_blob("i.png", png(&noise(16, 16, 1, true))).unwrap();
    let image = doc.add_stamp("I", StampSource::Image { blob }, uv([20.0, 20.0], [16.0, 16.0]), TextureSetId(0)).unwrap();
    assert_eq!(required_reader(&doc.snapshot()), 1, "an image stamp");
    // An unused sticker that is not kept is a leftover: not saved, so no feature.
    let leftover = StickerDoc::from_picture("x", owned(&mut doc, noise(8, 8, 2, true)), [8, 8]);
    let leftover = doc.add_sticker(leftover).unwrap();
    assert_eq!(required_reader(&doc.snapshot()), 1, "a leftover sticker");
    type Apply = Box<dyn Fn(&mut Document)>;
    let table: Vec<(&str, Apply)> = vec![
        ("stamp styles", Box::new(move |d| d.set_stamp_styles(image, StylePreset::Outline.styles([16, 16]), Commit::Final).unwrap())),
        (
            "a style entry that is off",
            Box::new(move |d| {
                let stroke = Stroke { on: false, width: 3.0, position: StrokePos::Outside, color: [0, 0, 0, 255], opacity: 1.0 };
                d.set_stamp_styles(image, StyleStack { stroke: Some(stroke), ..StyleStack::default() }, Commit::Final).unwrap();
            }),
        ),
        (
            "a sticker stamp",
            Box::new(move |d| {
                d.promote_stamp(image, ShareScope::ThisStamp).unwrap();
            }),
        ),
        (
            "a kept tray sticker",
            Box::new(move |d| {
                d.set_sticker(StickerRef::Id(leftover), |s| s.keep = true, "Keep", Commit::Final).unwrap();
            }),
        ),
    ];
    for (what, apply) in &table {
        apply(&mut doc);
        assert_eq!(required_reader(&doc.snapshot()), 4, "{what} alone");
        doc.undo().unwrap();
        assert_eq!(required_reader(&doc.snapshot()), 1, "{what} undone");
    }
    // The older features keep their numbers.
    let spec = text_spec(&mut doc, "x", TextFx { slant_deg: 10.0, ..TextFx::default() });
    doc.add_stamp("T", StampSource::Text(spec), uv([40.0, 40.0], [20.0, 10.0]), TextureSetId(0)).unwrap();
    assert_eq!(required_reader(&doc.snapshot()), 2, "a round-4 text effect");
    doc.set_stamp_styles(image, StylePreset::Glow.styles([16, 16]), Commit::Final).unwrap();
    assert_eq!(required_reader(&doc.snapshot()), 4);
}

// ----- the full round trip -----

#[test]
fn k4_a_sticker_with_every_layer_kind_shared_by_two_stamps_round_trips() {
    let Fixture { mut doc, shared, tray, unused, a, b, image } = fixture();
    let dir = temp_dir("k4_round_trip");
    let path = dir.join("p.wtep");
    let before = counters::sticker_encodes();
    save_doc(&mut doc, &path);
    // Owned pictures and masks: the shared sticker's edited photo and the mask on its image; the tray's picture and
    // mask. The leftover is not written.
    assert_eq!(counters::sticker_encodes() - before, 4);
    let names: Vec<String> = sticker_entries(&path).into_keys().collect();
    let (k, t) = (format!("stickers/k{:05}", shared.0), format!("stickers/k{:05}", tray.0));
    assert_eq!(names, [format!("{k}/m001.png"), format!("{k}/p002.png"), format!("{t}/m001.png"), format!("{t}/p001.png")]);
    let manifest = read_manifest(&path).unwrap();
    assert_eq!((manifest.format_version, manifest.min_reader_version), (4, 4));
    assert_eq!(manifest.stickers.iter().map(|s| s.id).collect::<Vec<_>>(), [shared, tray], "the leftover is dropped at save");
    assert!(manifest.links.is_empty());
    // The mask entry is 8-bit grey, the size of its picture.
    let m = decode_image(&entry(&path, &format!("{k}/m001.png")), None, 4096).unwrap().pixels;
    assert_eq!((m.width, m.height), (48, 48));
    // A stamp without styles has no `styles` key; a sticker stamp names its sticker.
    let raw: Value = serde_json::from_slice(&entry(&path, MANIFEST_ENTRY)).unwrap();
    let stamp_b = raw["stamps"].as_array().unwrap().iter().find(|s| s["name"] == "B").unwrap();
    assert!(stamp_b.get("styles").is_none());
    assert_eq!(stamp_b["source"], json!({ "kind": "sticker", "sticker": shared.0 }));

    let report = load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(!report.force_save_as);
    let mut loaded = report.document;
    assert_same_document(&doc, &loaded);
    assert_eq!(loaded.sticker(shared).map(|s| &**s), doc.sticker(shared).map(|s| &**s), "the shared sticker comes back equal");
    assert_eq!(loaded.sticker(tray).map(|s| &**s), doc.sticker(tray).map(|s| &**s), "the tray sticker too");
    assert!(loaded.sticker(unused).is_none());
    assert_eq!(loaded.sticker_users(shared), [a, b]);
    assert!(loaded.stamp(a).unwrap().styles == every_style());
    assert!(!loaded.stamp(image).unwrap().styles.is_default());
    // A new id never collides with a loaded sticker.
    let fresh = loaded.add_sticker(StickerDoc::new("n", 4, 4)).unwrap();
    assert!(fresh.0 > tray.0 && fresh.0 > shared.0);
    loaded.undo().unwrap();
    // Baking again from the loaded definitions gives the same pictures.
    for id in [a, b, image] {
        loaded.rebake_stamp(id, BakeQuality::Full).unwrap();
        doc.rebake_stamp(id, BakeQuality::Full).unwrap();
        assert_eq!(cache(&loaded, id), cache(&doc, id), "stamp {id:?}");
    }
    // A save of the reopened project copies every sticker PNG (no encode, same bytes).
    let again = dir.join("again.wtep");
    let before = counters::sticker_encodes();
    save_doc(&mut loaded, &again);
    assert_eq!(counters::sticker_encodes() - before, 0, "nothing is encoded again");
    assert_eq!(sticker_entries(&again), sticker_entries(&path));
    assert_eq!(read_manifest(&again).unwrap().stickers, manifest.stickers);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- load degradation -----

#[test]
fn k4_lost_pictures_masks_fonts_and_stickers_degrade_with_warnings() {
    let Fixture { mut doc, shared, tray, a, b, image, .. } = fixture();
    let dir = temp_dir("k4_lost");
    let path = dir.join("p.wtep");
    save_doc(&mut doc, &path);
    let k = format!("stickers/k{:05}", shared.0);

    // A missing picture entry: that layer is empty, one warning; the stamps re-bake from what is left.
    let lost_picture = a_copy(&path, "lost_picture.wtep");
    rewrite(&lost_picture, |name, data| (name != format!("{k}/p002.png")).then_some(data));
    let report = load(&lost_picture, &NoProgress).unwrap();
    assert!(
        matches!(report.warnings.as_slice(), [LoadWarning::StickerPixelsLost { sticker, layer: StickerLayerId(2), mask: false, .. }] if *sticker == shared),
        "{:?}",
        report.warnings
    );
    assert!(report.warnings[0].to_string().contains("\"Photo\""));
    let loaded = report.document;
    let photo = &loaded.sticker(shared).unwrap().layers[1];
    match &photo.content {
        StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => assert!(pixels.data.iter().all(|&v| v == 0)),
        other => panic!("{other:?}"),
    }
    assert_eq!(photo.adjustments, doc.sticker(shared).unwrap().layers[1].adjustments, "the rest of the layer stays");
    assert_ne!(cache(&loaded, a), cache(&doc, a), "the users show the sticker as it now is");

    // A broken mask: the layer is hidden by an empty mask.
    let lost_mask = a_copy(&path, "lost_mask.wtep");
    rewrite(&lost_mask, |name, data| Some(if name == format!("{k}/m001.png") { b"not a png".to_vec() } else { data }));
    let report = load(&lost_mask, &NoProgress).unwrap();
    assert!(matches!(report.warnings.as_slice(), [LoadWarning::StickerPixelsLost { mask: true, .. }]), "{:?}", report.warnings);
    let m = report.document.sticker(shared).unwrap().layers[0].mask.clone().unwrap();
    assert_eq!((m.pixels.width, m.pixels.height), (48, 48));
    assert!(m.pixels.data.iter().all(|&v| v == 0));

    // A missing sticker: its stamps keep their pictures as paint layers; the other stamp and the tray stay.
    let lost_sticker = a_copy(&path, "lost_sticker.wtep");
    edit_manifest(&lost_sticker, |m| m["stickers"].as_array_mut().unwrap().retain(|s| s["id"] != json!(shared.0)));
    let report = load(&lost_sticker, &NoProgress).unwrap();
    assert!(
        matches!(report.warnings.as_slice(), [LoadWarning::StickerLost { sticker, used_by, .. }] if *sticker == shared && *used_by == [a, b]),
        "{:?}",
        report.warnings
    );
    let loaded = report.document;
    assert!(loaded.stamp(a).is_none() && loaded.stamp(b).is_none());
    assert!(loaded.stamp(image).is_some() && loaded.sticker(tray).is_some());
    let set = &loaded.sets[0];
    for (id, name) in [(a, "A"), (b, "B")] {
        let layer = set.layers.iter().find(|l| l.name == name).unwrap();
        assert!(matches!(layer.kind, LayerKind::Raster(_)), "{name} is a paint layer now");
        assert_eq!(layer.to_dense(set.width, set.height), cache(&doc, id), "with the picture it showed");
    }

    // An invalid sticker entry (an artboard of zero width) is lost as a whole.
    let bad = a_copy(&path, "bad_artboard.wtep");
    edit_manifest(&bad, |m| {
        for s in m["stickers"].as_array_mut().unwrap() {
            if s["id"] == json!(tray.0) {
                s["artboard"] = json!([0, 0, 0, 20]);
            }
        }
    });
    let report = load(&bad, &NoProgress).unwrap();
    assert!(matches!(report.warnings.as_slice(), [LoadWarning::StickerLost { used_by, .. }] if used_by.is_empty()), "{:?}", report.warnings);
    assert!(report.document.sticker(tray).is_none());

    // A missing font of a text layer: a font from the file or the bundled one, one warning, the users re-baked.
    let lost_font = a_copy(&path, "lost_font.wtep");
    edit_manifest(&lost_font, |m| {
        for s in m["stickers"].as_array_mut().unwrap() {
            for l in s["layers"].as_array_mut().unwrap() {
                if l["content"]["kind"] == "text" {
                    l["content"]["font"]["blob"] = json!(999);
                }
            }
        }
    });
    let report = load(&lost_font, &NoProgress).unwrap();
    assert!(matches!(report.warnings.as_slice(), [LoadWarning::StickerFontFallback { .. }]), "{:?}", report.warnings);
    let StickerContent::Text(spec) = &report.document.sticker(shared).unwrap().layers[2].content else { panic!() };
    assert!(report.document.blobs.get(spec.font.blob).is_some_and(|b| b.kind == BlobKind::Font));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k4_unknown_extra_fields_in_the_new_entries_still_load() {
    let Fixture { mut doc, shared, a, .. } = fixture();
    let dir = temp_dir("k4_extra_fields");
    let path = dir.join("p.wtep");
    save_doc(&mut doc, &path);
    let link = json!({
        "sticker": shared.0,
        "file": { "kind": "in_folder", "name": "Every kind_k1.psd", "later": 1 },
        "mode": "layers",
        "scale": 2.0,
        "synced": { "size": 10, "crc32": 7, "later": true },
        "synced_sticker": 99,
        "layer_map": [{ "lyid": 3, "layer": 1, "origin": [0, 0], "size": [4, 4], "pixels_crc": 5, "later": [] }],
        "next_lyid": 4,
        "archive": "links/k00001.psd",
        "from_a_later_build": "x"
    });
    // A link entry is plain data in P4 (its meaning is package P5's): it parses, with unknown keys ignored.
    let parsed: LinkEntry = serde_json::from_value(link.clone()).unwrap();
    assert_eq!((parsed.next_lyid, parsed.layer_map.len(), parsed.pc_native), (4, 1, false));
    edit_manifest(&path, |m| {
        m["links"] = json!([link]);
        let s = &mut m["stickers"][0];
        s["later"] = json!({ "a": 1 });
        for l in s["layers"].as_array_mut().unwrap() {
            l["later"] = json!(true);
            l["content"]["later"] = json!(2);
            if let Some(styles) = l.get_mut("styles").and_then(Value::as_object_mut) {
                styles.insert("satin".to_owned(), json!({ "on": true }));
            }
        }
        for st in m["stamps"].as_array_mut().unwrap() {
            if let Some(styles) = st.get_mut("styles").and_then(Value::as_object_mut) {
                styles.insert("pattern_overlay".to_owned(), json!({ "on": true }));
            }
        }
    });
    let report = load(&path, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let loaded = report.document;
    assert_eq!(loaded.sticker(shared).map(|s| &**s), doc.sticker(shared).map(|s| &**s));
    assert!(loaded.stamp(a).unwrap().styles == every_style());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k4_non_finite_numbers_are_refused_at_save_with_a_named_message() {
    let Fixture { doc, shared, .. } = fixture();
    let dir = temp_dir("k4_non_finite");
    let path = dir.join("p.wtep");
    let message = |snapshot: &DocumentSnapshot| match save(snapshot, &path, &options(), &NoProgress) {
        Err(ProjectError::ManifestInvalid(m)) => m,
        other => panic!("{other:?}"),
    };
    fn styles_of_a(s: &mut DocumentSnapshot) -> &mut StyleStack {
        &mut s.stamps.iter_mut().find(|s| s.name == "A").unwrap().styles
    }
    let mut snapshot = doc.snapshot();
    styles_of_a(&mut snapshot).drop_shadow.as_mut().unwrap().distance = f32::NAN;
    assert_eq!(message(&snapshot), "the drop shadow of stamp \"A\" is not a finite number");
    let mut snapshot = doc.snapshot();
    styles_of_a(&mut snapshot).stroke.as_mut().unwrap().width = f32::INFINITY;
    assert_eq!(message(&snapshot), "the stroke width of stamp \"A\" is not a finite number");
    type Edit = Box<dyn Fn(&mut StickerDoc)>;
    let edits: Vec<(&str, Edit)> = vec![
        ("the opacity of layer \"Photo\"", Box::new(|s| s.layers[1].opacity = f32::NAN)),
        (
            "the brightness of layer \"Photo\"",
            Box::new(|s| s.layers[1].adjustments[0] = Adjustment::BrightnessContrast { brightness: f32::NEG_INFINITY, contrast: 0.0 }),
        ),
        (
            "the shape size of layer \"Star\"",
            Box::new(|s| {
                if let StickerContent::Shape(shape) = &mut s.layers[3].content {
                    shape.size.x = f32::NAN;
                }
            }),
        ),
        ("the transform of layer \"Logo\"", Box::new(|s| s.layers[0].transform.rotation = f32::NAN)),
        ("the corner radius", Box::new(|s| s.shape = ArtboardShape::RoundedRect { radius: f32::NAN })),
    ];
    for (what, edit) in &edits {
        let mut snapshot = doc.snapshot();
        edit(Arc::make_mut(snapshot.stickers.get_mut(&shared).unwrap()));
        assert_eq!(message(&snapshot), format!("{what} of sticker \"Every kind\" is not a finite number"));
    }
    assert!(!path.exists(), "nothing is written");
    // The same check guards a sticker file.
    let mut sticker = (**doc.sticker(shared).unwrap()).clone();
    sticker.layers[2].opacity = f32::NAN;
    assert!(matches!(write_wtes(&sticker, None, &doc.blobs, None), Err(ProjectError::ManifestInvalid(m)) if m.contains("\"Text\"")));
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- save reuse by picture version (review A8) -----

/// A document with a UV sticker stamp whose sticker has one owned 48 x 32 picture with a mask, and a kept tray sticker
/// (a 16 x 16 picture, no mask).
fn reuse_doc() -> (Document, StickerId, StampId, StickerId) {
    let mut doc = base_doc();
    let mut sticker = StickerDoc::from_picture("s", owned(&mut doc, noise(48, 32, 11, true)), [48, 32]);
    sticker.layers[0].mask = mask(&mut doc, AlphaBuf::filled(48, 32, 255));
    let sid = doc.add_sticker(sticker).unwrap();
    let stamp = doc.add_stamp("s", StampSource::Sticker { sticker: sid }, uv([32.0, 32.0], [48.0, 32.0]), TextureSetId(0)).unwrap();
    let mut tray = StickerDoc::from_picture("t", owned(&mut doc, noise(16, 16, 12, false)), [16, 16]);
    tray.keep = true;
    let tray = doc.add_sticker(tray).unwrap();
    doc.history.clear();
    (doc, sid, stamp, tray)
}

/// Every stored sticker PNG of `path` decodes to exactly the buffer the document holds for it now.
fn assert_entries_match(doc: &Document, path: &Path, step: &str) {
    let manifest: Manifest = read_manifest(path).unwrap();
    let stored = sticker_entries(path);
    let decoded = |name: &str| decode_image(&stored[name], None, 8192).unwrap().pixels;
    assert_eq!(manifest.stickers.iter().map(|s| s.id).collect::<Vec<_>>(), doc.saved_sticker_ids(), "{step}: saved stickers");
    for s in &manifest.stickers {
        let sticker = doc.sticker(s.id).unwrap();
        for l in &s.layers {
            let layer = sticker.layer(l.id).unwrap();
            if let StickerContentEntry::Picture { file: Some(file), .. } = &l.content {
                let StickerContent::Picture(PicturePixels::Owned { pixels, .. }) = &layer.content else { panic!("{step}") };
                assert!(decoded(&file.entry) == **pixels, "{step}: picture {} of sticker {} differs", l.id.0, s.id.0);
            }
            match (&l.mask, &layer.mask) {
                (Some(file), Some(m)) => {
                    let grey: Vec<u8> = decoded(&file.entry).data.iter().step_by(4).copied().collect();
                    assert!(grey == m.pixels.data, "{step}: mask {} of sticker {} differs", l.id.0, s.id.0);
                }
                (None, None) => {}
                other => panic!("{step}: the mask is saved for one side only: {other:?}"),
            }
        }
    }
}

#[test]
fn k4_save_reuse_copies_unchanged_pictures_and_encodes_new_versions() {
    let (mut doc, sid, _stamp, _tray) = reuse_doc();
    let dir = temp_dir("k4_reuse");
    let (one, two, three) = (dir.join("1.wtep"), dir.join("2.wtep"), dir.join("3.wtep"));
    let before = counters::sticker_encodes();
    save_doc(&mut doc, &one);
    assert_eq!(counters::sticker_encodes() - before, 3, "picture, mask, tray picture");
    // Unchanged: every sticker PNG is copied.
    let before = counters::sticker_encodes();
    save_doc(&mut doc, &two);
    assert_eq!(counters::sticker_encodes() - before, 0);
    assert_eq!(sticker_entries(&two), sticker_entries(&one));
    // A stroke and its undo: the same bytes under a new version, so the picture is encoded again (the mask is not).
    let version = |d: &Document| match &d.sticker(sid).unwrap().layers[0].content {
        StickerContent::Picture(PicturePixels::Owned { version, .. }) => *version,
        other => panic!("{other:?}"),
    };
    let (lid, v0) = (doc.sticker(sid).unwrap().layers[0].id, version(&doc));
    let brush = StickerBrush { radius: 4.0, color: [255, 0, 0, 255], ..StickerBrush::default() };
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, brush).unwrap();
    doc.sticker_stroke_dab(Vec2::new(10.0, 10.0), 1.0).unwrap();
    doc.sticker_stroke_end().unwrap();
    doc.undo().unwrap();
    assert_ne!(version(&doc), v0);
    let before = counters::sticker_encodes();
    save_doc(&mut doc, &three);
    assert_eq!(counters::sticker_encodes() - before, 1);
    assert_eq!(sticker_entries(&three), sticker_entries(&one), "the same bytes, encoded again");
    assert_entries_match(&doc, &three, "after the undo");
    // A record whose file changed since is not used.
    std::fs::write(&three, b"replaced").unwrap();
    let before = counters::sticker_encodes();
    save_doc(&mut doc, &one);
    assert_eq!(counters::sticker_encodes() - before, 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The sticker stamp `id` shows, else `fallback`.
fn shown(doc: &Document, id: StampId, fallback: StickerId) -> StickerId {
    match doc.stamp(id).map(|s| &s.source) {
        Some(StampSource::Sticker { sticker }) => *sticker,
        _ => fallback,
    }
}

#[test]
fn k4_two_different_pictures_never_share_a_reuse_record() {
    // 200 random edit / save steps (strokes on pictures and masks, replaced pictures of the same and of another size,
    // replaced masks, undo, redo, detached copies that share a buffer, reloads); after each save every sticker PNG of
    // the file is compared with the document.
    let (mut doc, sid, stamp, tray) = reuse_doc();
    let other = doc.add_stamp("o", StampSource::Sticker { sticker: sid }, uv([16.0, 48.0], [24.0, 16.0]), TextureSetId(0)).unwrap();
    doc.history.clear();
    let dir = temp_dir("k4_property");
    let paths: [PathBuf; 2] = [dir.join("a.wtep"), dir.join("b.wtep")];
    let mut rng = XorShift::new(0x5eed_0004);
    let (mut copied, mut encoded, mut reloads) = (0u64, 0u64, 0u32);
    for step in 0..200 {
        for _ in 0..1 + rng.next_u32() % 3 {
            let target = match rng.next_u32() % 3 {
                0 => tray,
                1 => shown(&doc, other, sid),
                _ => shown(&doc, stamp, sid),
            };
            let Some(sticker) = doc.sticker(target) else { continue };
            let lid = sticker.layers[0].id;
            let has_mask = sticker.layers[0].mask.is_some();
            let size = match &sticker.layers[0].content {
                StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => [pixels.width, pixels.height],
                _ => [16, 16],
            };
            match rng.next_u32() % 8 {
                0 | 1 => {
                    let on_mask = has_mask && rng.next_u32().is_multiple_of(2);
                    let brush = StickerBrush {
                        radius: 2.0 + (rng.next_u32() % 6) as f32,
                        color: [rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8, 255],
                        ..StickerBrush::default()
                    };
                    let t = if on_mask { PixelTarget::Mask } else { PixelTarget::Pixels };
                    if doc.sticker_stroke_begin(StickerRef::Id(target), lid, t, brush).is_ok() {
                        let at = Vec2::new((rng.next_u32() % size[0]) as f32, (rng.next_u32() % size[1]) as f32);
                        doc.sticker_stroke_dab(at, 1.0).unwrap();
                        doc.sticker_stroke_end().unwrap();
                    }
                }
                2 => {
                    doc.undo();
                }
                3 => {
                    doc.redo();
                }
                4 => {
                    let new = PixelData::Rgba(noise(size[0], size[1], rng.next_u32() as u64, false));
                    let _ = doc.replace_sticker_pixels(StickerRef::Id(target), lid, PixelTarget::Pixels, new, "Replace");
                }
                5 if !has_mask => {
                    let (w, h) = (8 + rng.next_u32() % 40, 8 + rng.next_u32() % 40);
                    let new = PixelData::Rgba(noise(w, h, rng.next_u32() as u64, true));
                    let _ = doc.replace_sticker_pixels(StickerRef::Id(target), lid, PixelTarget::Pixels, new, "Resize");
                }
                6 => {
                    let _ = doc.detach_sticker(other);
                }
                _ if has_mask => {
                    let mut m = AlphaBuf::new(size[0], size[1]);
                    m.data.iter_mut().for_each(|v| *v = rng.next_u32() as u8);
                    let _ = doc.replace_sticker_pixels(StickerRef::Id(target), lid, PixelTarget::Mask, PixelData::Alpha(m), "Mask");
                }
                _ => {}
            }
        }
        let path = &paths[step % 2];
        let before = counters::sticker_encodes();
        save_doc(&mut doc, path);
        let n = counters::sticker_encodes() - before;
        encoded += n;
        copied += (sticker_entries(path).len() as u64).saturating_sub(n);
        assert_entries_match(&doc, path, &format!("step {step}"));
        if rng.next_u32().is_multiple_of(25) {
            // Reopen: fresh versions and the record of the loaded file.
            let report = load(path, &NoProgress).unwrap();
            assert!(report.warnings.is_empty(), "{:?}", report.warnings);
            doc = report.document;
            reloads += 1;
            assert_entries_match(&doc, path, &format!("step {step}, reopened"));
        }
    }
    assert!(copied > 100 && encoded > 100 && reloads > 0, "{copied} copied, {encoded} encoded, {reloads} reloads");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- .wtes -----

#[test]
fn k4_wtes_round_trip_with_deterministic_bytes() {
    let Fixture { mut doc, shared, .. } = fixture();
    let styles = every_style();
    let bytes = doc.sticker_wtes(shared, Some(&styles)).unwrap();
    assert_eq!(doc.sticker_wtes(shared, Some(&styles)).unwrap(), bytes, "the same sticker gives the same bytes");
    // The container: mimetype first and stored, sticker.json last.
    let mut zip = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    let names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned()).collect();
    assert_eq!(names.first().map(String::as_str), Some("mimetype"));
    assert_eq!(names.last().map(String::as_str), Some("sticker.json"));
    assert_eq!(zip.by_index(0).unwrap().compression(), CompressionMethod::Stored);
    for name in ["m001.png", "p002.png", "preview.png"] {
        assert!(names.iter().any(|n| n == name), "{name} in {names:?}");
    }
    assert_eq!(names.iter().filter(|n| n.starts_with("blobs/")).count(), 2, "the image and the font");

    let file = read_wtes(&bytes).unwrap();
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    assert_eq!(&file.sticker, &**doc.sticker(shared).unwrap(), "blob ids are still the file's (equal to the source's here)");
    assert!(file.styles.as_ref() == Some(&styles));
    let preview = file.preview.as_ref().unwrap();
    assert!(preview.width <= 512 && preview.height <= 512 && preview.width > 0);

    // Into another document: a new id, kept, the same finished picture; identical blobs are shared.
    let mut other = base_doc();
    let id = other.import_wtes(&file).unwrap();
    assert_eq!(other.undo_label(), Some("Add Sticker"));
    assert!(other.sticker(id).unwrap().keep);
    assert_eq!(other.sticker_picture(id, None, 1.0).unwrap(), doc.sticker_picture(shared, None, 1.0).unwrap());
    assert_eq!(other.saved_sticker_ids(), [id], "a kept sticker is saved");
    let blobs_before = other.blobs.iter().count();
    other.import_wtes(&file).unwrap();
    assert_eq!(other.blobs.iter().count(), blobs_before);

    // Refusals: not a zip, not a sticker file, a newer format.
    assert!(matches!(read_wtes(b"not a zip"), Err(ProjectError::NotAZip(_))));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("mimetype", zip_stored()).unwrap();
    zip.write_all(b"application/x-wtep").unwrap();
    assert!(matches!(read_wtes(&zip.finish().unwrap().into_inner()), Err(ProjectError::ManifestInvalid(_))));
    let mut newer: Value = {
        let mut zip = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        let mut json = Vec::new();
        zip.by_name("sticker.json").unwrap().read_to_end(&mut json).unwrap();
        serde_json::from_slice(&json).unwrap()
    };
    newer["format"] = json!(2);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("mimetype", zip_stored()).unwrap();
    zip.write_all(WTES_MIMETYPE.as_bytes()).unwrap();
    zip.start_file("sticker.json", zip_deflated()).unwrap();
    zip.write_all(&serde_json::to_vec(&newer).unwrap()).unwrap();
    assert!(matches!(read_wtes(&zip.finish().unwrap().into_inner()), Err(ProjectError::TooNew { found: 2, supported: 1 })));
}

#[test]
fn k4_a_wtes_with_a_lost_picture_degrades_like_a_project() {
    let Fixture { mut doc, tray, .. } = fixture();
    let bytes = doc.sticker_wtes(tray, None).unwrap();
    let mut zip = ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
    let mut out = ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        let name = e.name().unwrap().into_owned();
        if name == "p001.png" {
            continue;
        }
        let method = e.compression();
        let mut data = Vec::new();
        e.read_to_end(&mut data).unwrap();
        out.start_file(name, if method == CompressionMethod::Stored { zip_stored() } else { zip_deflated() }).unwrap();
        out.write_all(&data).unwrap();
    }
    let file = read_wtes(&out.finish().unwrap().into_inner()).unwrap();
    assert!(
        matches!(file.warnings.as_slice(), [LoadWarning::StickerPixelsLost { mask: false, .. }, LoadWarning::StickerPixelsLost { mask: true, .. }]),
        "{:?}",
        file.warnings
    );
    assert!(file.styles.is_none());
}

// ----- extract -----

#[test]
fn k4_extract_writes_the_finished_picture_of_every_saved_sticker() {
    let Fixture { mut doc, shared, tray, .. } = fixture();
    let pictures = doc.sticker_extract_pictures();
    assert_eq!(pictures.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["Every kind", "Tray"]);
    assert_eq!(pictures[0].1, doc.sticker_picture(shared, None, 1.0).unwrap());
    assert_eq!(pictures[1].1, doc.sticker_picture(tray, None, 1.0).unwrap());
    let dir = temp_dir("k4_extract");
    let written = crate::project::extract_bundle(&doc.source, &doc.blobs, &pictures, &dir, false).unwrap();
    let rel: Vec<String> = written.iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
    assert!(rel.ends_with(&["stickers/Every kind.png".to_owned(), "stickers/Tray.png".to_owned()]), "{rel:?}");
    let back = decode_image(&std::fs::read(dir.join("stickers").join("Tray.png")).unwrap(), None, 4096).unwrap().pixels;
    assert_eq!(back, pictures[1].1);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- check-P4 (the P4 checker's tests) -----

#[test]
fn k4_check_a_sticker_lost_after_queuing_its_pictures_leaves_the_next_sticker_alone() {
    // The shared sticker's last layer gets the id of its first one: the entry is invalid as a whole, but only at its
    // last layer, after its mask and its owned picture were queued for reading. Those queued PNGs must not land in the
    // next sticker of the file (the tray, which has one layer: before the fix the load panicked on a layer index).
    let Fixture { mut doc, shared, tray, a, b, .. } = fixture();
    let dir = temp_dir("k4_check_dup_layer");
    let path = dir.join("p.wtep");
    save_doc(&mut doc, &path);
    edit_manifest(&path, |m| {
        for s in m["stickers"].as_array_mut().unwrap() {
            if s["id"] == json!(shared.0) {
                let layers = s["layers"].as_array_mut().unwrap();
                let first = layers[0]["id"].clone();
                let last = layers.len() - 1;
                layers[last]["id"] = first;
            }
        }
    });
    let report = load(&path, &NoProgress).unwrap();
    assert!(
        matches!(report.warnings.as_slice(), [LoadWarning::StickerLost { sticker, used_by, .. }] if *sticker == shared && *used_by == [a, b]),
        "{:?}",
        report.warnings
    );
    let loaded = report.document;
    assert!(loaded.sticker(shared).is_none());
    let (before, after) = (doc.sticker(tray).unwrap(), loaded.sticker(tray).unwrap());
    assert_eq!(after.layers.len(), 1);
    let pixels = |s: &StickerDoc| match &s.layers[0].content {
        StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => Arc::clone(pixels),
        other => panic!("{other:?}"),
    };
    assert!(*pixels(after) == *pixels(before), "the tray picture is its own");
    assert!(after.layers[0].mask.as_ref().unwrap().pixels.data == before.layers[0].mask.as_ref().unwrap().pixels.data, "and so is its mask");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The version check of the P0 build (`project::migrate::parse_manifest` at b2fb8f5, format 3), copied verbatim apart
/// from the constant: `Ok(newer)` when the build opens the file, `Err` otherwise.
fn p0_version_check(bytes: &[u8]) -> Result<bool, ProjectError> {
    const FORMAT_VERSION: u32 = 3;
    fn version_field(value: &Value, name: &str) -> Result<u32, ProjectError> {
        value
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| ProjectError::ManifestInvalid(format!("\"{name}\" is missing or not a version number")))
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| ProjectError::ManifestInvalid(e.to_string()))?;
    if value.get("format").and_then(Value::as_str) != Some("wtep") {
        return Err(ProjectError::ManifestInvalid("\"format\" is not \"wtep\"".to_owned()));
    }
    let found_version = version_field(&value, "format_version")?;
    let min_reader_version = version_field(&value, "min_reader_version")?;
    if min_reader_version > FORMAT_VERSION {
        return Err(ProjectError::TooNew { found: found_version.max(min_reader_version), supported: FORMAT_VERSION });
    }
    Ok(found_version > FORMAT_VERSION)
}

#[test]
fn k4_check_the_p0_version_check_refuses_a_sticker_project_and_opens_a_plain_one() {
    let dir = temp_dir("k4_check_p0");
    // With a sticker stamp: "saved by a newer version".
    let (mut doc, ..) = reuse_doc();
    let with = dir.join("with.wtep");
    save_doc(&mut doc, &with);
    let result = p0_version_check(&entry(&with, MANIFEST_ENTRY));
    assert!(matches!(result, Err(ProjectError::TooNew { found: 4, supported: 3 })), "{result:?}");
    assert!(result.unwrap_err().to_string().contains("saved by a newer version"));
    // Only a kept tray sticker: refused as well (an old build would drop it on its next save).
    let (mut doc, _, stamp, _) = reuse_doc();
    doc.remove_stamp(stamp).unwrap();
    let tray_only = dir.join("tray.wtep");
    save_doc(&mut doc, &tray_only);
    assert!(matches!(p0_version_check(&entry(&tray_only, MANIFEST_ENTRY)), Err(ProjectError::TooNew { .. })));
    // Without a new feature: the P0 build opens it (as a newer format it can read) and the manifest has no new key.
    let mut plain = base_doc();
    let plain_path = dir.join("plain.wtep");
    save_doc(&mut plain, &plain_path);
    let manifest = entry(&plain_path, MANIFEST_ENTRY);
    assert!(matches!(p0_version_check(&manifest), Ok(true)));
    let raw: Value = serde_json::from_slice(&manifest).unwrap();
    assert!(raw.get("stickers").is_none() && raw.get("links").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}
