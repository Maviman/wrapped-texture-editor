// Tests of package P8's core layout commands (STICKER_PLAN 7 "P8", 3.3, 2.9, 2.10; names `k8_*`): Trim, Add Padding,
// Canvas Size, Crop, Resize, Rotate, Flip, Flatten, Merge Down, the layer boxes and Export as PSD. A layer-space point
// must stay on its texel through every artboard change (2.9) except a turn, which turns about the centre; Resize keeps
// the placement; every command is one undo step; an image stamp is promoted inside the command.

use std::sync::Arc;

use glam::Vec2;

use crate::doc_stickers::{ShareScope, StickerRef};
use crate::document::Document;
use crate::error::EditError;
use crate::ids::{StampId, StickerId, StickerLayerId, TextureSetId};
use crate::image_io::{PngLevel, encode_png};
use crate::math::IRect;
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::samples;
use crate::stamp::{Placement, StampSource, UvPlacement};
use crate::sticker::shape::{Paint, ShapeKind, ShapeSpec};
use crate::sticker::style::StylePreset;
use crate::sticker::{ArtboardShape, LayerMask, LayerTransform, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::testutil::doc_from_raw;

fn uv(size: Vec2) -> Placement {
    Placement::Uv(UvPlacement {
        set: TextureSetId(0),
        center_px: Vec2::new(128.0, 128.0),
        size_px: size,
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        clip_to_islands: false,
    })
}

fn flat(w: u32, h: u32, c: [u8; 4]) -> PixelBuf {
    PixelBuf::filled(w, h, c)
}

/// A 64 x 48 sticker at (4, -2): a blue base filling it, a red 16 x 16 square at (24, 8) with a mask hiding its left
/// half and a ClassicSticker style at half opacity Multiply, and a green star shape; one UV stamp at 1 texel per
/// layer-space pixel.
fn doc() -> (Document, StickerId, StampId, StickerLayerId, StickerLayerId) {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let mut s = StickerDoc::new("lay", 64, 48);
    s.artboard = IRect::new(4, -2, 68, 46);
    let base = s.alloc_layer();
    let mut l = StickerLayer::new(base, "base", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(flat(64, 48, [20, 40, 200, 255])), version: doc.next_pixel_version() }));
    l.transform = LayerTransform::moved(4.0, -2.0);
    s.layers.push(l);
    let red = s.alloc_layer();
    let mut l = StickerLayer::new(red, "red", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(flat(16, 16, [230, 20, 20, 255])), version: doc.next_pixel_version() }));
    l.transform = LayerTransform::moved(24.0, 8.0);
    l.styles = StylePreset::ClassicSticker.styles([16, 16]);
    l.opacity = 0.5;
    l.blend = crate::layer::BlendMode::Multiply;
    let mut mask = AlphaBuf::filled(16, 16, 255);
    for y in 0..16 {
        for x in 0..8 {
            mask.data[y * 16 + x] = 0;
        }
    }
    l.mask = Some(LayerMask { pixels: Arc::new(mask), version: doc.next_pixel_version() });
    s.layers.push(l);
    let star = s.alloc_layer();
    let shape = ShapeSpec { kind: ShapeKind::Star { points: 5, inner: 0.5 }, size: Vec2::new(12.0, 12.0), fill: Paint::Solid([20, 200, 20, 255]), outline: None };
    let mut l = StickerLayer::new(star, "star", StickerContent::Shape(shape));
    l.transform = LayerTransform::moved(46.0, 20.0);
    s.layers.push(l);
    let sid = doc.add_sticker(s).unwrap();
    let stamp = doc.add_stamp("lay", StampSource::Sticker { sticker: sid }, uv(Vec2::new(64.0, 48.0)), TextureSetId(0)).unwrap();
    doc.history.clear();
    (doc, sid, stamp, base, red)
}

fn on(stamp: StampId) -> StickerRef {
    StickerRef::OfStamp(stamp, ShareScope::ThisStamp)
}

/// The texel (UV pixels) where the layer-space point `p` of the stamp's sticker lands (UV placement without rotation
/// or flips: the artboard centre at `center_px`, `size_px / artboard size` texels per layer-space pixel).
fn texel_of(doc: &Document, stamp: StampId, p: Vec2) -> Vec2 {
    let s = doc.stamp(stamp).unwrap();
    let Placement::Uv(u) = s.placement else { panic!("not a UV placement") };
    let StampSource::Sticker { sticker } = s.source else { panic!("not a sticker") };
    let d = doc.sticker(sticker).unwrap();
    let a = d.artboard;
    let c = Vec2::new((a.x0 + a.x1) as f32 * 0.5, (a.y0 + a.y1) as f32 * 0.5);
    let k = u.size_px / Vec2::new(a.width() as f32, a.height() as f32);
    u.center_px + (p - c) * k
}

fn near(a: Vec2, b: Vec2, tol: f32) -> bool {
    (a - b).abs().max_element() <= tol
}

fn one_entry(doc: &Document, before: usize, label: &str) {
    assert_eq!(doc.history.undo_len(), before + 1, "one entry");
    assert_eq!(doc.undo_label(), Some(label));
}

#[test]
fn k8_trim_padding_canvas_size_and_crop_keep_the_artwork_on_its_texel() {
    let (mut doc, sid, stamp, base, _) = doc();
    // Make the base smaller than the artboard, so Trim has something to cut.
    doc.set_sticker(StickerRef::Id(sid), |d| {
        let l = d.layer_mut(base).unwrap();
        l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(flat(30, 20, [20, 40, 200, 255])), version: 0 });
        l.transform = LayerTransform::moved(10.0, 4.0);
    }, "setup", crate::document::Commit::Final)
    .unwrap();
    let mark = Vec2::new(12.5, 6.5);
    let start = texel_of(&doc, stamp, mark);
    let n = doc.history.undo_len();
    doc.trim_sticker(on(stamp), 0).unwrap();
    one_entry(&doc, n, crate::TRIM_LABEL);
    let a = doc.sticker(sid).unwrap().artboard;
    assert_eq!((a.x0, a.y0), (10, 4), "the trimmed artboard starts at the base: {a:?}");
    assert!(near(texel_of(&doc, stamp, mark), start, 1e-3), "trim keeps the texel");
    doc.pad_sticker(on(stamp), 5).unwrap();
    assert_eq!(doc.sticker(sid).unwrap().artboard.x0, a.x0 - 5);
    assert!(near(texel_of(&doc, stamp, mark), start, 1e-3), "padding keeps the texel");
    doc.canvas_size_sticker(on(stamp), [100, 80], [0.0, 0.0]).unwrap();
    assert_eq!(doc.sticker(sid).unwrap().size(), [100, 80]);
    assert!(near(texel_of(&doc, stamp, mark), start, 1e-3), "canvas size keeps the texel");
    doc.crop_sticker(on(stamp), IRect::new(8, 2, 48, 34), ArtboardShape::Ellipse, false).unwrap();
    let d = doc.sticker(sid).unwrap();
    assert_eq!((d.artboard, d.shape), (IRect::new(8, 2, 48, 34), ArtboardShape::Ellipse));
    assert!(near(texel_of(&doc, stamp, mark), start, 1e-3), "crop keeps the texel");
    // Every step one undo step: four undos give the start.
    for _ in 0..4 {
        doc.undo();
    }
    assert!(near(texel_of(&doc, stamp, mark), start, 1e-3));
    assert_eq!(doc.history.undo_len(), n);
}

#[test]
fn k8_trim_of_nothing_is_refused_and_a_crop_can_delete_the_cropped_pixels() {
    let (mut doc, sid, stamp, base, red) = doc();
    let n = doc.history.undo_len();
    let empty = StickerDoc::new("e", 10, 10);
    let eid = doc.add_sticker(empty).unwrap();
    assert!(matches!(doc.trim_sticker(StickerRef::Id(eid), 0), Err(EditError::InvalidParam(_))));
    doc.undo();
    assert_eq!(doc.history.undo_len(), n);
    doc.crop_sticker(on(stamp), IRect::new(14, 0, 34, 20), ArtboardShape::Rect, true).unwrap();
    let d = doc.sticker(sid).unwrap();
    let b = d.layer(base).unwrap();
    let p = b.owned_picture().unwrap();
    assert_eq!(([p.width, p.height], b.transform.translate), ([20, 20], Vec2::new(14.0, 0.0)), "the base kept only its pixels inside");
    let r = d.layer(red).unwrap();
    let rp = r.owned_picture().unwrap();
    assert_eq!(([rp.width, rp.height], r.transform.translate), ([10, 12], Vec2::new(24.0, 8.0)));
    let m = r.mask.as_ref().unwrap();
    assert_eq!((m.pixels.width, m.pixels.height, m.pixels.get(0, 0), m.pixels.get(9, 0)), (10, 12, 0, 255), "the mask is cut with its picture");
    assert!(matches!(doc.crop_sticker(on(stamp), IRect::new(5, 5, 5, 9), ArtboardShape::Rect, false), Err(EditError::InvalidParam(_))));
}

#[test]
fn k8_resize_keeps_the_placement_and_scales_the_stamp_styles() {
    let (mut doc, sid, stamp, _, red) = doc();
    doc.set_stamp_styles(stamp, StylePreset::ClassicSticker.styles([64, 48]), crate::document::Commit::Final).unwrap();
    let placement = doc.stamp(stamp).unwrap().placement;
    let width = doc.stamp(stamp).unwrap().styles.contour.as_ref().unwrap().width;
    let before = doc.sticker_picture(sid, None, 1.0).unwrap();
    let n = doc.history.undo_len();
    doc.resize_sticker(on(stamp), [128, 96]).unwrap();
    one_entry(&doc, n, crate::RESIZE_LABEL);
    let s = doc.stamp(stamp).unwrap();
    assert_eq!(s.placement, placement, "the sticker keeps its size on the model");
    assert!((s.styles.contour.as_ref().unwrap().width - width * 2.0).abs() < 1e-3, "the stamp's style lengths scale");
    let d = Arc::clone(doc.sticker(sid).unwrap());
    assert_eq!((d.size(), d.layer(red).unwrap().transform.scale), ([128, 96], Vec2::splat(2.0)));
    // The picture at render scale 1 equals the old one at scale 2 (within resampling).
    let after = doc.sticker_picture(sid, None, 1.0).unwrap();
    assert_eq!([after.width, after.height], [128, 96]);
    let (a, b) = (after.get(70, 40), before.get(35, 20));
    assert!(a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3), "{a:?} vs {b:?}");
    assert!(matches!(doc.resize_sticker(on(stamp), [1, 1]), Err(EditError::InvalidParam(_))), "below 1 %");
    doc.undo();
    assert_eq!(doc.stamp(stamp).unwrap().styles.contour.as_ref().unwrap().width, width);
    assert_eq!(doc.sticker(sid).unwrap().size(), [64, 48]);
}

#[test]
fn k8_rotate_turns_about_the_centre_and_flip_mirrors() {
    let (mut doc, sid, stamp, _, _) = doc();
    let centre = Vec2::new(36.0, 22.0);
    let at_centre = texel_of(&doc, stamp, centre);
    let before = doc.sticker_picture(sid, None, 1.0).unwrap();
    let n = doc.history.undo_len();
    doc.rotate_sticker(on(stamp), 90.0).unwrap();
    one_entry(&doc, n, crate::ROTATE_LABEL);
    let d = Arc::clone(doc.sticker(sid).unwrap());
    assert_eq!(d.size(), [48, 64]);
    let Placement::Uv(u) = doc.stamp(stamp).unwrap().placement else { panic!() };
    assert!(near(u.size_px, Vec2::new(48.0, 64.0), 1e-3), "one layer-space pixel keeps its size: {:?}", u.size_px);
    assert!(near(texel_of(&doc, stamp, centre), at_centre, 1e-3), "the centre stays");
    let turned = doc.sticker_picture(sid, None, 1.0).unwrap();
    // A clockwise quarter turn: the old (x, y) is at (h - 1 - y, x).
    for (x, y) in [(2u32, 3u32), (30, 20), (60, 40)] {
        let (o, t) = (before.get(x, y), turned.get(47 - y, x));
        assert!(o.iter().zip(t).all(|(a, b)| a.abs_diff(b) <= 2), "({x}, {y}): {o:?} vs {t:?}");
    }
    doc.rotate_sticker(on(stamp), -90.0).unwrap();
    assert_eq!(doc.sticker(sid).unwrap().artboard, IRect::new(4, -2, 68, 46));
    assert_eq!(doc.sticker_picture(sid, None, 1.0).unwrap(), before, "CW then CCW is the start");
    doc.flip_sticker(on(stamp), true).unwrap();
    assert_eq!(doc.undo_label(), Some(crate::FLIP_H_LABEL));
    let flipped = doc.sticker_picture(sid, None, 1.0).unwrap();
    assert_eq!(flipped.get(63 - 30, 20), before.get(30, 20));
    doc.flip_sticker(on(stamp), true).unwrap();
    assert_eq!(doc.sticker_picture(sid, None, 1.0).unwrap(), before);
    assert!(matches!(doc.rotate_sticker(on(stamp), f32::NAN), Err(EditError::InvalidParam(_))));
}

#[test]
fn k8_flatten_and_merge_down_keep_the_look() {
    let (mut doc, sid, stamp, base, red) = doc();
    let before = doc.sticker_picture(sid, None, 1.0).unwrap();
    let n = doc.history.undo_len();
    doc.merge_down_sticker_layer(on(stamp), red).unwrap();
    one_entry(&doc, n, crate::MERGE_DOWN_LABEL);
    let d = Arc::clone(doc.sticker(sid).unwrap());
    assert_eq!(d.layers.len(), 2);
    assert_eq!((d.layers[0].id, d.layers[0].name.as_str()), (base, "base"), "the merged layer is the lower one");
    assert_eq!(doc.sticker_picture(sid, None, 1.0).unwrap(), before, "merging changes no pixel");
    assert!(matches!(doc.merge_down_sticker_layer(on(stamp), base), Err(EditError::InvalidParam(_))), "the bottom layer");
    doc.set_sticker(StickerRef::Id(sid), |d| d.shape = ArtboardShape::Ellipse, "shape", crate::document::Commit::Final).unwrap();
    let ellipse = doc.sticker_picture(sid, None, 1.0).unwrap();
    doc.flatten_sticker(on(stamp)).unwrap();
    assert_eq!(doc.undo_label(), Some(crate::FLATTEN_LABEL));
    let d = Arc::clone(doc.sticker(sid).unwrap());
    assert_eq!((d.layers.len(), d.shape), (1, ArtboardShape::Ellipse));
    assert_eq!(doc.sticker_picture(sid, None, 1.0).unwrap(), ellipse, "flattening changes no pixel");
    for _ in 0..3 {
        doc.undo();
    }
    assert_eq!(doc.sticker(sid).unwrap().layers.len(), 3);
    assert_eq!(doc.history.undo_len(), n);
}

#[test]
fn k8_an_image_stamp_is_promoted_inside_a_layout_command() {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let png = encode_png(20, 10, &flat(20, 10, [200, 100, 0, 255]).data, PngLevel::Fast).unwrap();
    let blob = doc.add_image_blob("p.png", Arc::from(png)).unwrap();
    let stamp = doc.add_stamp("p", StampSource::Image { blob }, uv(Vec2::new(40.0, 20.0)), TextureSetId(0)).unwrap();
    doc.history.clear();
    let virtual_doc = doc.sticker_layered(on(stamp)).unwrap();
    assert_eq!((virtual_doc.size(), virtual_doc.layers[0].id), ([20, 10], StickerLayerId(1)));
    let b = doc.sticker_layer_box(on(stamp), StickerLayerId(1)).unwrap();
    assert_eq!((b.size, b.pivot, b.text), (Vec2::new(20.0, 10.0), Vec2::new(10.0, 5.0), false));
    doc.rotate_sticker(on(stamp), 90.0).unwrap();
    assert!(matches!(doc.stamp(stamp).unwrap().source, StampSource::Sticker { .. }));
    assert_eq!(doc.history.undo_len(), 1, "promotion and turn are one entry");
    doc.undo();
    assert_eq!(doc.stamp(stamp).unwrap().source, StampSource::Image { blob });
    assert!(matches!(doc.flatten_sticker(on(stamp)), Err(EditError::InvalidParam(_))), "one plain layer already");
    assert_eq!(doc.stamp(stamp).unwrap().source, StampSource::Image { blob });
}

#[test]
fn k8_layer_boxes_and_export_as_psd() {
    let (mut doc, sid, stamp, base, _) = doc();
    let r = StickerRef::Id(sid);
    let d = Arc::clone(doc.sticker(sid).unwrap());
    let star = d.layers[2].id;
    assert_eq!(doc.sticker_layer_box(r, base).unwrap(), crate::LayerBox::centred(Vec2::new(64.0, 48.0)));
    assert_eq!(doc.sticker_layer_box(r, star).unwrap(), crate::LayerBox::centred(Vec2::new(12.0, 12.0)));
    // A text layer: its frame at size_px (also above the engine's 512 px).
    let font = doc.default_font();
    let mut spec = crate::text::TextSpec::new("Hi", font);
    spec.size_px = 40.0;
    let small = doc.layer_box_of(&StickerLayer::new(StickerLayerId(9), "t", StickerContent::Text(Box::new(spec.clone())))).unwrap();
    assert!(small.text && small.size.x > 20.0 && small.size.y > 20.0, "{small:?}");
    spec.size_px = 512.0;
    let at_512 = doc.layer_box_of(&StickerLayer::new(StickerLayerId(9), "t", StickerContent::Text(Box::new(spec.clone())))).unwrap();
    spec.size_px = 1024.0;
    let big = doc.layer_box_of(&StickerLayer::new(StickerLayerId(9), "t", StickerContent::Text(Box::new(spec)))).unwrap();
    assert!((big.size - at_512.size * 2.0).abs().max_element() < 1e-3, "{big:?} vs {at_512:?}: the 512-px frame scaled up");
    let bytes = doc.sticker_psd_bytes(on(stamp)).unwrap();
    let walk = crate::psd::strict::walk(&bytes).unwrap();
    let _ = walk;
    let read = crate::psd::read(&bytes, &crate::psd::PsdLimits::default()).unwrap();
    assert_eq!(read.layers.len(), 3);
    assert!(doc.link(sid).is_none(), "no link is made");
}

#[test]
fn k8_a_library_file_shows_its_name_and_preview_without_its_layers() {
    let (mut doc, sid, _, _, _) = doc();
    let bytes = doc.sticker_wtes(sid, None).unwrap();
    let (name, preview) = crate::sticker::file::read_wtes_preview(&bytes).unwrap();
    let preview = preview.unwrap();
    assert_eq!((name.as_str(), [preview.width, preview.height]), ("lay", [64, 48]));
    assert!(crate::sticker::file::read_wtes_preview(b"not a zip").is_none());
    let file = crate::sticker::file::read_wtes(&bytes).unwrap();
    let id = doc.import_wtes(&file).unwrap();
    assert_eq!(doc.sticker_picture(id, None, 1.0).unwrap(), doc.sticker_picture(sid, None, 1.0).unwrap());
}

/// Timing (8.4 has no own budget for the layout commands; they run once per click, like Crop to Selection): Trim,
/// Rotate 90, Resize, Merge Down and Flatten of a 2048 x 2048 sticker of a white picture and a styled red square.
#[test]
#[ignore = "perf: run with --release --ignored --nocapture"]
fn perf_k8_layout_commands() {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let mut s = StickerDoc::new("big", 2048, 2048);
    let bottom = s.alloc_layer();
    let v = doc.next_pixel_version();
    s.layers.push(StickerLayer::new(bottom, "bottom", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(flat(2000, 2000, [255, 255, 255, 255])), version: v })));
    let top = s.alloc_layer();
    let v = doc.next_pixel_version();
    let mut l = StickerLayer::new(top, "top", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(flat(800, 800, [200, 30, 30, 255])), version: v }));
    l.transform = LayerTransform::moved(600.0, 600.0);
    l.styles = StylePreset::ClassicSticker.styles([2048, 2048]);
    s.layers.push(l);
    let sid = doc.add_sticker(s).unwrap();
    let on = StickerRef::Id(sid);
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let mut best = [f64::MAX; 5];
    for _ in 0..3 {
        let n = doc.history.undo_len();
        let t = std::time::Instant::now();
        doc.trim_sticker(on, 0).unwrap();
        best[0] = best[0].min(ms(t));
        let t = std::time::Instant::now();
        doc.rotate_sticker(on, 90.0).unwrap();
        best[1] = best[1].min(ms(t));
        let t = std::time::Instant::now();
        doc.resize_sticker(on, [1000, 1000]).unwrap();
        best[2] = best[2].min(ms(t));
        doc.undo();
        let t = std::time::Instant::now();
        doc.merge_down_sticker_layer(on, top).unwrap();
        best[3] = best[3].min(ms(t));
        doc.undo();
        let t = std::time::Instant::now();
        doc.flatten_sticker(on).unwrap();
        best[4] = best[4].min(ms(t));
        while doc.history.undo_len() > n {
            doc.undo();
        }
    }
    println!(
        "perf_k8: trim {:.1} ms; rotate 90 {:.1} ms; resize {:.1} ms; merge down {:.1} ms; flatten {:.1} ms",
        best[0], best[1], best[2], best[3], best[4]
    );
}
