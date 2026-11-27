// Tests of package P7b's core read-outs (STICKER_PLAN 7 "P7b", 3.3, 4.4; names `k7b_*`): a layer drawn alone over the
// artboard, the unstyled composite, and the finished picture with one layer's pixels or mask replaced.

use std::sync::Arc;

use glam::Vec2;

use crate::doc_sticker_edits::{PixelData, PixelTarget};
use crate::doc_stickers::{ShareScope, StickerRef};
use crate::document::Document;
use crate::error::EditError;
use crate::ids::{StampId, StickerId, StickerLayerId, TextureSetId};
use crate::image_io::{PngLevel, encode_png};
use crate::math::IRect;
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::samples;
use crate::stamp::{Placement, StampSource, UvPlacement};
use crate::sticker::style::StylePreset;
use crate::sticker::{LayerMask, LayerTransform, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::testutil::doc_from_raw;

fn uv() -> Placement {
    Placement::Uv(UvPlacement {
        set: TextureSetId(0),
        center_px: Vec2::new(128.0, 128.0),
        size_px: Vec2::new(64.0, 48.0),
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        clip_to_islands: false,
    })
}

fn flat(w: u32, h: u32, c: [u8; 4]) -> PixelBuf {
    let mut p = PixelBuf::new(w, h);
    p.data.as_chunks_mut::<4>().0.iter_mut().for_each(|px| *px = c);
    p
}

/// A 64 x 48 sticker (artboard at (4, -2)): a blue base, a red square at (20, 10) with a ClassicSticker style and a
/// half-opacity Multiply blend and a mask that hides its left half, and a hidden green layer.
fn doc() -> (Document, StickerId, StampId, StickerLayerId, StickerLayerId) {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let mut s = StickerDoc::new("cut", 64, 48);
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
    let hidden = s.alloc_layer();
    let mut l = StickerLayer::new(hidden, "hidden", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(flat(64, 48, [0, 255, 0, 255])), version: doc.next_pixel_version() }));
    l.visible = false;
    s.layers.push(l);
    let sid = doc.add_sticker(s).unwrap();
    let stamp = doc.add_stamp("cut", StampSource::Sticker { sticker: sid }, uv(), TextureSetId(0)).unwrap();
    doc.history.clear();
    (doc, sid, stamp, base, red)
}

/// One layer alone: unstyled, full opacity, Normal, its mask and transform kept; the hidden layer is drawn too; the
/// document and its history are unchanged.
#[test]
fn k7b_a_layer_on_the_artboard_is_drawn_alone_and_plain() {
    let (mut doc, sid, stamp, _, red) = doc();
    let before = Arc::clone(doc.sticker(sid).unwrap());
    let p = doc.sticker_layer_on_artboard(StickerRef::OfStamp(stamp, ShareScope::ThisStamp), red).unwrap();
    assert_eq!([p.width, p.height], [64, 48]);
    // The red square covers artboard pixels (20..36, 10..26); its mask hides the left half.
    assert_eq!(p.get(30, 15), [230, 20, 20, 255], "full opacity, Normal, no style");
    assert_eq!(p.get(22, 15)[3], 0, "the mask hides the left half");
    assert_eq!(p.get(10, 5)[3], 0, "nothing else is drawn (no contour, no base)");
    assert_eq!(p.get(37, 15)[3], 0, "no die-cut contour around it");
    let hidden = doc.sticker(sid).unwrap().layers[2].id;
    let h = doc.sticker_layer_on_artboard(StickerRef::Id(sid), hidden).unwrap();
    assert_eq!(h.get(5, 5), [0, 255, 0, 255], "a hidden layer is drawn for sampling");
    assert!(Arc::ptr_eq(&before, doc.sticker(sid).unwrap()));
    assert_eq!(doc.history.undo_len(), 0);
    assert!(matches!(doc.sticker_layer_on_artboard(StickerRef::Id(sid), StickerLayerId(99)), Err(EditError::UnknownStickerLayer)));
}

/// The unstyled composite: every visible layer with its opacity and blend, no style; equals the finished picture of the
/// same sticker with the styles cleared.
#[test]
fn k7b_the_unstyled_composite_leaves_out_the_styles_only() {
    let (mut doc, sid, _, _, red) = doc();
    let plain = doc.sticker_unstyled_composite(StickerRef::Id(sid)).unwrap();
    let styled = doc.sticker_picture(sid, None, 1.0).unwrap();
    assert_ne!(plain, styled, "the style draws a contour");
    doc.set_sticker(StickerRef::Id(sid), |d| d.layer_mut(red).unwrap().styles = Default::default(), "clear", crate::document::Commit::Final).unwrap();
    assert_eq!(plain, doc.sticker_picture(sid, None, 1.0).unwrap());
    assert_eq!(plain.get(5, 5), [20, 40, 200, 255], "the hidden green layer is not in it");
}

/// The picture with replaced pixels equals the picture after the same replacement in the document; a mask can be
/// replaced the same way; the document is unchanged until then.
#[test]
fn k7b_the_picture_with_replaced_pixels_is_what_the_edit_gives() {
    let (mut doc, sid, stamp, base, red) = doc();
    let on = StickerRef::OfStamp(stamp, ShareScope::ThisStamp);
    let new = flat(64, 48, [250, 250, 10, 128]);
    let preview = doc.sticker_picture_with(on, base, PixelData::Rgba(new.clone())).unwrap();
    assert_eq!(doc.history.undo_len(), 0);
    doc.replace_sticker_pixels(on, base, PixelTarget::Pixels, PixelData::Rgba(new), "t").unwrap();
    assert_eq!(preview, doc.sticker_picture(sid, None, 1.0).unwrap());
    let mask = AlphaBuf::filled(16, 16, 255);
    let preview = doc.sticker_picture_with(on, red, PixelData::Alpha(mask.clone())).unwrap();
    doc.replace_sticker_pixels(on, red, PixelTarget::Mask, PixelData::Alpha(mask), "m").unwrap();
    assert_eq!(preview, doc.sticker_picture(sid, None, 1.0).unwrap());
    assert!(matches!(doc.sticker_picture_with(on, base, PixelData::Alpha(AlphaBuf::new(64, 48))), Err(EditError::InvalidParam(_))), "no mask");
    assert!(matches!(doc.sticker_picture_with(on, base, PixelData::Rgba(PixelBuf::new(0, 0))), Err(EditError::InvalidParam(_))));
}

/// An image stamp that is not a sticker yet: its picture for all three, and nothing promotes it.
#[test]
fn k7b_an_image_stamp_answers_with_its_picture_and_stays_an_image() {
    let (mut doc, ..) = doc();
    let img = samples::sample_stamp_image(32);
    let png = encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap();
    let blob = doc.add_image_blob("i.png", Arc::from(png)).unwrap();
    let image = doc.add_stamp("i", StampSource::Image { blob }, uv(), TextureSetId(0)).unwrap();
    let on = StickerRef::OfStamp(image, ShareScope::ThisStamp);
    assert_eq!(doc.sticker_layer_on_artboard(on, StickerLayerId(1)).unwrap(), img);
    assert_eq!(doc.sticker_unstyled_composite(on).unwrap(), img);
    let other = flat(32, 32, [1, 2, 3, 4]);
    assert_eq!(doc.sticker_picture_with(on, StickerLayerId(1), PixelData::Rgba(other.clone())).unwrap(), other);
    assert!(doc.sticker_picture_with(on, StickerLayerId(1), PixelData::Alpha(AlphaBuf::new(32, 32))).is_err());
    assert!(matches!(doc.sticker_layer_on_artboard(on, StickerLayerId(2)), Err(EditError::UnknownStickerLayer)));
    assert!(matches!(doc.stamp(image).unwrap().source, StampSource::Image { .. }));
}

/// The read-outs on a 2048 x 2048 sticker of two picture layers (one styled), release, this machine (STICKER_PLAN 8.4:
/// the wand, Magic Eraser and Remove Background budgets of 60 / 60 / 150 ms include the picture they look at; numbers in
/// `docs/notes/sticker/build-P7b.md`): the active layer alone on the artboard plus the wand's region, the unstyled
/// composite plus the region, and a Blur preview (radius 16) with the finished picture drawn around it.
#[test]
#[ignore = "perf: run with --release --ignored --nocapture"]
fn perf_k7b_wand_sampling_and_preview() {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let mut s = StickerDoc::new("big", 2048, 2048);
    let bottom = s.alloc_layer();
    let v = doc.next_pixel_version();
    s.layers.push(StickerLayer::new(bottom, "bottom", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(flat(2048, 2048, [255, 255, 255, 255])), version: v })));
    let top = s.alloc_layer();
    let mut pic = PixelBuf::new(2048, 2048);
    for y in 600..1400 {
        for x in 600..1400 {
            pic.set(x, y, [200, 30, 30, 255]);
        }
    }
    let v = doc.next_pixel_version();
    let mut l = StickerLayer::new(top, "top", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(pic), version: v }));
    l.styles = StylePreset::ClassicSticker.styles([2048, 2048]);
    s.layers.push(l);
    let sid = doc.add_sticker(s).unwrap();
    let on = StickerRef::Id(sid);
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let opts = crate::sticker::select::RegionOptions::default();
    let mut best = [f64::MAX; 3];
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let p = doc.sticker_layer_on_artboard(on, top).unwrap();
        let r = crate::sticker::select::region(&p, (1000, 1000), opts);
        assert_eq!(r.get(1000, 1000), 255);
        best[0] = best[0].min(ms(t));
        let t = std::time::Instant::now();
        let p = doc.sticker_unstyled_composite(on).unwrap();
        let r = crate::sticker::select::region(&p, (10, 10), opts);
        assert_eq!(r.get(1000, 1000), 0);
        best[1] = best[1].min(ms(t));
        let t = std::time::Instant::now();
        let pixels = doc.sticker_layer_pixels(on, top).unwrap();
        let blurred = crate::sticker::filter::blur(&pixels, 16.0, None);
        let pic = doc.sticker_picture_with(on, top, PixelData::Rgba(blurred)).unwrap();
        assert_eq!(pic.width, 2048);
        best[2] = best[2].min(ms(t));
    }
    println!("perf_k7b: layer on the artboard + wand {:.1} ms; unstyled composite + wand {:.1} ms; blur 16 preview (blur + styled render) {:.1} ms", best[0], best[1], best[2]);
}
