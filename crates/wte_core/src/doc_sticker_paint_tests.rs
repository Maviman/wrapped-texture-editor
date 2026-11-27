// Tests of package P7a's core additions (STICKER_PLAN 7 "P7a", 3.5, review A6; names `k7a_*`): the canvas region
// render with the stroke's layer unstyled, the render jobs (the canvas restyle, the model's live Preview), the
// Eyedropper and layer read-outs, Rasterize and the selection seen from a layer's content pixels.

use std::sync::Arc;

use glam::Vec2;

use crate::doc_sticker_edits::{PixelTarget, StickerBrush};
use crate::doc_sticker_paint::{RASTERIZE_LABEL, StickerRenderPurpose};
use crate::doc_stickers::{ShareScope, StickerRef};
use crate::document::Document;
use crate::error::EditError;
use crate::ids::{StampId, StickerId, StickerLayerId, TextureSetId};
use crate::image_io::{PngLevel, encode_png};
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf};
use crate::samples;
use crate::stamp::{Placement, StampSource, UvPlacement};
use crate::sticker::adjust::Adjustment;
use crate::sticker::render::{LayerCache, render, render_region};
use crate::sticker::select::selection_in_content;
use crate::sticker::shape::{Paint, ShapeKind, ShapeSpec};
use crate::sticker::style::StylePreset;
use crate::sticker::{
    ArtboardShape, LayerMask, LayerTransform, PicturePixels, StickerContent, StickerDoc, StickerLayer, counters,
};
use crate::testutil::{XorShift, doc_from_raw};
use crate::text::{RasterQuality, TEXT_PLACEMENT_PX, TextAlign, TextFx, TextSpec};

fn noise(w: u32, h: u32, seed: u64, alpha: Option<u8>) -> PixelBuf {
    let mut rng = XorShift::new(seed);
    let data = (0..w * h * 4).map(|_| rng.next_u32() as u8).collect();
    let mut p = PixelBuf::from_raw(w, h, data).unwrap();
    if let Some(a) = alpha {
        p.data.as_chunks_mut::<4>().0.iter_mut().for_each(|px| px[3] = a);
    }
    p
}

fn owned(doc: &mut Document, p: PixelBuf) -> PicturePixels {
    PicturePixels::Owned { pixels: Arc::new(p), version: doc.next_pixel_version() }
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

fn text(doc: &mut Document, content: &str) -> TextSpec {
    TextSpec {
        content: content.to_owned(),
        font: doc.default_font(),
        size_px: TEXT_PLACEMENT_PX,
        color: [250, 240, 20, 255],
        align: TextAlign::Center,
        line_spacing: 1.0,
        letter_spacing: 0.0,
        outline: None,
        fx: TextFx::default(),
    }
}

/// A sticker of every kind of layer the region path meets: a styled picture moved by whole pixels (the live layer of
/// the tests, id 2), a picture scaled and turned with a mask and an adjustment, a layer clipped to the live one, a
/// shape with Multiply, a text, hidden layers; ellipse artboard at a non-zero origin.
fn rich_sticker(doc: &mut Document) -> (StickerDoc, StickerLayerId) {
    let mut s = StickerDoc::new("rich", 120, 90);
    s.artboard = IRect::new(10, -6, 130, 84);
    s.shape = ArtboardShape::Ellipse;
    let base = s.alloc_layer();
    let mut l = StickerLayer::new(base, "base", StickerContent::Picture(owned(doc, noise(80, 60, 1, Some(255)))));
    l.transform = LayerTransform { translate: Vec2::new(20.0, 4.0), scale: Vec2::new(1.3, 0.8), rotation: 0.4 };
    let mut mask = AlphaBuf::new(80, 60);
    for (i, v) in mask.data.iter_mut().enumerate() {
        *v = (i % 251) as u8;
    }
    l.mask = Some(LayerMask { pixels: Arc::new(mask), version: doc.next_pixel_version() });
    l.adjustments.push(Adjustment::Invert);
    s.layers.push(l);
    let live = s.alloc_layer();
    let mut l = StickerLayer::new(live, "live", StickerContent::Picture(owned(doc, noise(64, 48, 2, None))));
    l.transform = LayerTransform::moved(30.0, 10.0);
    l.styles = StylePreset::ClassicSticker.styles([64, 48]);
    l.adjustments.push(Adjustment::Posterize { levels: 4 });
    let mut m = AlphaBuf::new(64, 48);
    for (i, v) in m.data.iter_mut().enumerate() {
        *v = 255 - (i % 200) as u8;
    }
    l.mask = Some(LayerMask { pixels: Arc::new(m), version: doc.next_pixel_version() });
    l.opacity = 0.85;
    s.layers.push(l);
    let clipped = s.alloc_layer();
    let mut l = StickerLayer::new(clipped, "clipped", StickerContent::Picture(owned(doc, noise(50, 50, 3, Some(200)))));
    l.transform = LayerTransform::moved(40.0, 20.0);
    l.clip = true;
    l.blend = BlendMode::Screen;
    s.layers.push(l);
    let shape = s.alloc_layer();
    let spec = ShapeSpec { kind: ShapeKind::Star { points: 5, inner: 0.5 }, size: Vec2::new(40.0, 40.0), fill: Paint::Solid([20, 200, 90, 255]), outline: Some(([0, 0, 0, 255], 2.0)) };
    let mut l = StickerLayer::new(shape, "shape", StickerContent::Shape(spec));
    l.transform = LayerTransform::moved(70.0, 30.0);
    l.blend = BlendMode::Multiply;
    l.styles = StylePreset::ClassicSticker.styles([40, 40]);
    s.layers.push(l);
    let t = s.alloc_layer();
    let mut l = StickerLayer::new(t, "text", StickerContent::Text(Box::new(text(doc, "Hi"))));
    l.transform = LayerTransform::moved(60.0, 60.0);
    s.layers.push(l);
    let hidden = s.alloc_layer();
    let mut l = StickerLayer::new(hidden, "hidden", StickerContent::Picture(owned(doc, noise(20, 20, 4, Some(255)))));
    l.visible = false;
    s.layers.push(l);
    (s, live)
}

/// The pixels of `full` (a whole render) inside `r`.
fn crop(full: &PixelBuf, r: IRect) -> Vec<u8> {
    let mut out = Vec::new();
    for y in r.y0..r.y1 {
        let s = (y as usize * full.width as usize + r.x0 as usize) * 4;
        out.extend_from_slice(&full.data[s..s + r.width() as usize * 4]);
    }
    out
}

/// `data` with the colour of every fully transparent pixel set to 0: what is seen (the canvas shows premultiplied
/// pixels). A region can leave out a layer clipped to the live layer where the live layer draws nothing in it; the
/// whole render composites that layer at alpha 0 there, which can leave a colour under alpha 0.
fn seen(mut data: Vec<u8>) -> Vec<u8> {
    for p in data.as_chunks_mut::<4>().0 {
        if p[3] == 0 {
            *p = [0; 4];
        }
    }
    data
}

/// `render_region` equals the whole render in every region, with no live layer and with a live layer without
/// styles; with a styled live layer it equals the render of the sticker whose live layer has no styles.
#[test]
fn k7a_render_region_equals_the_whole_render_in_any_rectangle() {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let (sticker, live) = rich_sticker(&mut doc);
    let arc = Arc::new(sticker.clone());
    let input = doc.sticker_render_input(&arc).unwrap();
    let full = render(&input, 1.0, RasterQuality::Full);
    let mut unstyled = sticker.clone();
    unstyled.layer_mut(live).unwrap().styles = Default::default();
    let unstyled_input = doc.sticker_render_input(&Arc::new(unstyled)).unwrap();
    let plain = render(&unstyled_input, 1.0, RasterQuality::Full);
    assert_ne!(full.data, plain.data, "the live layer's style shows");
    let mut cache = LayerCache::default();
    let mut rng = XorShift::new(9);
    let mut rects = vec![IRect::from_size(120, 90), IRect::new(-20, -20, 500, 500), IRect::new(0, 0, 1, 1), IRect::new(119, 89, 120, 90)];
    for _ in 0..40 {
        let x0 = (rng.next_u32() % 130) as i32 - 5;
        let y0 = (rng.next_u32() % 100) as i32 - 5;
        rects.push(IRect::new(x0, y0, x0 + 1 + (rng.next_u32() % 60) as i32, y0 + 1 + (rng.next_u32() % 50) as i32));
    }
    for r in rects {
        let clipped = r.intersect(&IRect::from_size(120, 90));
        let none = render_region(&input, r, None, Some(&mut cache));
        assert_eq!(none.rect, clipped, "{r:?}");
        assert_eq!(none.data, crop(&full, clipped), "no live layer, {r:?}");
        let uncached = render_region(&input, r, None, None);
        assert_eq!(uncached.data, none.data, "cache or not, {r:?}");
        let lived = render_region(&input, r, Some(live), Some(&mut cache));
        assert_eq!(seen(lived.data), seen(crop(&plain, clipped)), "styled live layer drawn unstyled, {r:?}");
        let unstyled_live = render_region(&unstyled_input, r, Some(live), Some(&mut cache));
        assert_eq!(seen(unstyled_live.data), seen(crop(&plain, clipped)), "an unstyled live layer is the true picture, {r:?}");
    }
    // A live layer that is scaled (the general path) is still exact.
    let mut turned = sticker.clone();
    turned.layer_mut(live).unwrap().transform = LayerTransform { translate: Vec2::new(31.5, 9.25), scale: Vec2::new(-1.2, 0.9), rotation: 0.3 };
    turned.layer_mut(live).unwrap().styles = Default::default();
    let turned_input = doc.sticker_render_input(&Arc::new(turned)).unwrap();
    let turned_full = render(&turned_input, 1.0, RasterQuality::Full);
    for r in [IRect::new(5, 5, 70, 60), IRect::from_size(120, 90)] {
        assert_eq!(seen(render_region(&turned_input, r, Some(live), Some(&mut cache)).data), seen(crop(&turned_full, r)));
    }
}

/// The trivial fast path (one plain picture filling the artboard) gives the picture's bytes, as `render`.
#[test]
fn k7a_render_region_of_a_trivial_sticker_is_the_picture() {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let p = noise(33, 21, 5, None);
    let s = StickerDoc::from_picture("t", owned(&mut doc, p.clone()), [33, 21]);
    let live = s.layers[0].id;
    let input = doc.sticker_render_input(&Arc::new(s)).unwrap();
    let r = IRect::new(3, 2, 30, 19);
    assert_eq!(render_region(&input, r, Some(live), None).data, crop(&p, r));
    assert_eq!(render(&input, 1.0, RasterQuality::Full).data, p.data);
}

/// A document with a UV sticker stamp: the sticker of `rich_sticker`.
fn rich_doc() -> (Document, StickerId, StampId, StickerLayerId) {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let (sticker, live) = rich_sticker(&mut doc);
    let sid = doc.add_sticker(sticker).unwrap();
    let stamp = doc.add_stamp("rich", StampSource::Sticker { sticker: sid }, uv([128.0, 128.0], [120.0, 90.0]), TextureSetId(0)).unwrap();
    doc.history.clear();
    (doc, sid, stamp, live)
}

fn brush(radius: f32) -> StickerBrush {
    StickerBrush { radius, color: [255, 0, 0, 255], hardness: 1.0, ..StickerBrush::default() }
}

/// The document's region read-out during a stroke: the painted pixels with the live layer unstyled; no whole render
/// happens for it (the layer cache serves the other layers after the first call).
#[test]
fn k7a_the_canvas_region_during_a_stroke_shows_the_new_paint_unstyled() {
    let (mut doc, sid, _, live) = rich_doc();
    let artboard = doc.sticker(sid).unwrap().artboard;
    doc.sticker_stroke_begin(StickerRef::Id(sid), live, PixelTarget::Pixels, brush(4.0)).unwrap();
    let dirty = doc.sticker_stroke_dab(Vec2::new(50.0, 30.0), 1.0).unwrap();
    assert!(!dirty.is_empty());
    // Content rect -> artboard pixels (the layer moved by 30, 10; the artboard starts at 10, -6).
    let r = IRect::new(dirty.x0 + 20, dirty.y0 + 16, dirty.x1 + 20, dirty.y1 + 16);
    let renders = counters::renders();
    let regions = counters::region_renders();
    let got = doc.sticker_canvas_region(sid, r, Some(live)).unwrap();
    let again = doc.sticker_canvas_region(sid, r, Some(live)).unwrap();
    assert_eq!(counters::renders(), renders, "no whole render for a dirty rectangle");
    assert_eq!(counters::region_renders(), regions + 2);
    assert_eq!(got.data, again.data);
    let mut unstyled = (**doc.sticker(sid).unwrap()).clone();
    unstyled.layer_mut(live).unwrap().styles = Default::default();
    let input = doc.sticker_render_input(&Arc::new(unstyled)).unwrap();
    let want = render(&input, 1.0, RasterQuality::Full);
    assert_eq!(seen(got.data), seen(crop(&want, got.rect)));
    assert_eq!(artboard, IRect::new(10, -6, 130, 84));
    doc.sticker_stroke_end().unwrap();
    assert!(matches!(doc.sticker_canvas_region(StickerId(9999), r, None), Err(EditError::UnknownSticker)));
}

/// The canvas job equals `sticker_picture(id, None, 1.0)`; it takes a snapshot, so a dab after it starts does not
/// change its picture (the dab copies the buffer once).
#[test]
fn k7a_the_canvas_job_draws_the_finished_sticker_from_its_snapshot() {
    let (mut doc, sid, _, live) = rich_doc();
    doc.sticker_stroke_begin(StickerRef::Id(sid), live, PixelTarget::Pixels, brush(6.0)).unwrap();
    doc.sticker_stroke_dab(Vec2::new(45.0, 25.0), 1.0).unwrap();
    let job = doc.sticker_canvas_job(sid).unwrap();
    assert_eq!(job.sticker(), sid);
    let snapshot = (**job.doc()).clone();
    let copies = counters::buffer_copies();
    doc.sticker_stroke_dab(Vec2::new(60.0, 30.0), 1.0).unwrap();
    assert_eq!(counters::buffer_copies(), copies + 1, "the dab after the snapshot copies the painted buffer once");
    doc.sticker_stroke_dab(Vec2::new(70.0, 30.0), 1.0).unwrap();
    assert_eq!(counters::buffer_copies(), copies + 1, "and only once");
    let out = std::thread::spawn(move || job.run()).join().unwrap();
    assert_eq!((out.purpose, out.scale, out.mips.is_none()), (StickerRenderPurpose::Canvas, 1.0, true));
    assert_eq!(*out.doc, snapshot);
    let input = doc.sticker_render_input(&Arc::new(snapshot)).unwrap();
    assert_eq!(out.picture.data, render(&input, 1.0, RasterQuality::Full).data, "the picture of the snapshot");
    doc.sticker_stroke_end().unwrap();
    let job = doc.sticker_canvas_job(sid).unwrap();
    let out = job.run();
    assert_eq!(*out.picture, doc.sticker_picture(sid, None, 1.0).unwrap(), "after the stroke: the finished picture");
}

/// The model's live Preview: applied only while a stroke on that sticker is open; it changes the stamp's linked
/// layer, makes no entry and leaves no render of an earlier state in the cache; the stroke's end bakes the Final.
#[test]
fn k7a_a_preview_job_bakes_the_users_during_the_stroke_only() {
    let (mut doc, sid, stamp, live) = rich_doc();
    let linked = |doc: &Document| {
        let s = doc.set(TextureSetId(0)).unwrap();
        s.layers.iter().find(|l| l.stamp_id() == Some(stamp)).map(|l| l.to_dense(s.width, s.height).data).unwrap()
    };
    let before = linked(&doc);
    let job = doc.sticker_preview_job(sid).unwrap().expect("the sticker has a user");
    assert!(!doc.apply_sticker_preview(job.run()), "no stroke open: nothing applied");
    doc.sticker_stroke_begin(StickerRef::Id(sid), live, PixelTarget::Pixels, brush(10.0)).unwrap();
    for k in 0..8 {
        doc.sticker_stroke_dab(Vec2::new(35.0 + 4.0 * k as f32, 20.0), 1.0).unwrap();
    }
    let job = doc.sticker_preview_job(sid).unwrap().unwrap();
    let out = job.run();
    assert_eq!(out.purpose, StickerRenderPurpose::Preview);
    assert!(out.mips.is_some());
    let undo = doc.history.undo_len();
    assert!(doc.apply_sticker_preview(out));
    assert_eq!(doc.history.undo_len(), undo, "no entry");
    let previewed = linked(&doc);
    assert_ne!(previewed, before, "the model shows the paint during the stroke");
    let current = Arc::clone(doc.sticker(sid).unwrap());
    assert!(
        !doc.sticker_state.cache.renders.iter().any(|e| Arc::ptr_eq(&e.doc, &current)),
        "the job's picture served one bake and was dropped"
    );
    // Another sticker's job, or one that lands after the stroke, does nothing.
    let other = doc.add_sticker(StickerDoc::new("o", 8, 8));
    assert!(matches!(other, Err(EditError::StrokeInProgress)));
    let late = doc.sticker_preview_job(sid).unwrap().unwrap();
    doc.sticker_stroke_end().unwrap();
    assert!(!doc.apply_sticker_preview(late.run()));
    doc.settle_final_bakes();
    assert_ne!(linked(&doc), before);
    // A sticker without users has nothing to preview.
    let tray = doc.add_sticker(StickerDoc::new("tray", 8, 8)).unwrap();
    assert!(doc.sticker_preview_job(tray).unwrap().is_none());
}

/// The Eyedropper's colour: the finished sticker at a layer-space point; an image stamp's picture; nothing outside.
#[test]
fn k7a_sticker_color_at_reads_the_finished_picture() {
    let (mut doc, sid, stamp, _) = rich_doc();
    let picture = doc.sticker_picture(sid, None, 1.0).unwrap();
    for (x, y) in [(60.0f32, 40.0f32), (12.5, -5.5), (129.9, 83.9), (70.0, 10.0)] {
        let want = picture.get((x - 10.0).floor() as u32, (y + 6.0).floor() as u32);
        assert_eq!(doc.sticker_color_at(StickerRef::Id(sid), Vec2::new(x, y)), Some(want), "{x}, {y}");
        assert_eq!(doc.sticker_color_at(StickerRef::OfStamp(stamp, ShareScope::ThisStamp), Vec2::new(x, y)), Some(want));
    }
    for p in [Vec2::new(9.0, 0.0), Vec2::new(130.0, 0.0), Vec2::new(20.0, 84.0), Vec2::new(f32::NAN, 1.0)] {
        assert_eq!(doc.sticker_color_at(StickerRef::Id(sid), p), None, "{p:?}");
    }
    let img = samples::sample_stamp_image(64);
    let png = encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap();
    let blob = doc.add_image_blob("i.png", Arc::from(png)).unwrap();
    let image = doc.add_stamp("i", StampSource::Image { blob }, uv([60.0, 60.0], [64.0, 64.0]), TextureSetId(0)).unwrap();
    assert_eq!(doc.sticker_color_at(StickerRef::OfStamp(image, ShareScope::ThisStamp), Vec2::new(31.2, 20.7)), Some(img.get(31, 20)));
    assert_eq!(doc.sticker_color_at(StickerRef::OfStamp(image, ShareScope::ThisStamp), Vec2::new(64.0, 3.0)), None);
}

/// Layer read-outs: owned pixels share the layer's buffer; a blob picture its decoded pixels; an image stamp's
/// virtual layer 1; text and shape layers have none.
#[test]
fn k7a_layer_pixels_and_masks_are_shared_read_outs() {
    let (mut doc, sid, stamp, live) = rich_doc();
    let s = Arc::clone(doc.sticker(sid).unwrap());
    let p = doc.sticker_layer_pixels(StickerRef::Id(sid), live).unwrap();
    assert!(Arc::ptr_eq(&p, s.layer(live).unwrap().owned_picture().unwrap()));
    let m = doc.sticker_layer_mask(StickerRef::OfStamp(stamp, ShareScope::ThisStamp), live).unwrap().unwrap();
    assert!(Arc::ptr_eq(&m, &s.layer(live).unwrap().mask.as_ref().unwrap().pixels));
    let shape = s.layers[3].id;
    assert!(matches!(doc.sticker_layer_pixels(StickerRef::Id(sid), shape), Err(EditError::InvalidParam(_))));
    assert!(matches!(doc.sticker_layer_pixels(StickerRef::Id(sid), StickerLayerId(99)), Err(EditError::UnknownStickerLayer)));
    let img = samples::sample_stamp_image(64);
    let png = encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap();
    let blob = doc.add_image_blob("i.png", Arc::from(png)).unwrap();
    let image = doc.add_stamp("i", StampSource::Image { blob }, uv([60.0, 60.0], [64.0, 64.0]), TextureSetId(0)).unwrap();
    let on = StickerRef::OfStamp(image, ShareScope::ThisStamp);
    assert_eq!(*doc.sticker_layer_pixels(on, StickerLayerId(1)).unwrap(), img);
    assert_eq!(doc.sticker_layer_mask(on, StickerLayerId(1)).unwrap(), None);
    assert!(matches!(doc.sticker_layer_pixels(on, StickerLayerId(2)), Err(EditError::UnknownStickerLayer)));
    assert!(matches!(doc.stamp(image).unwrap().source, StampSource::Image { .. }), "read-outs never promote");
}

/// Rasterize: a text and a shape layer become picture layers that look the same (the finished picture is
/// unchanged), one entry each, undo brings the text back; a picture layer and a locked layer are refused.
#[test]
fn k7a_rasterize_keeps_the_picture_and_is_one_entry() {
    let (mut doc, sid, _, live) = rich_doc();
    let before = doc.sticker_picture(sid, None, 1.0).unwrap();
    let s = Arc::clone(doc.sticker(sid).unwrap());
    let (shape, text) = (s.layers[3].id, s.layers[4].id);
    for (n, id) in [(1, text), (2, shape)] {
        doc.rasterize_sticker_layer(StickerRef::Id(sid), id).unwrap();
        assert_eq!((doc.history.undo_len(), doc.undo_label()), (n, Some(RASTERIZE_LABEL)));
        let l = doc.sticker(sid).unwrap().layer(id).unwrap().clone();
        assert!(matches!(l.content, StickerContent::Picture(PicturePixels::Owned { .. })));
        assert!(l.adjustments.is_empty() && l.transform.scale == Vec2::ONE && l.transform.translate.fract() == Vec2::ZERO);
        assert_eq!(doc.sticker_picture(sid, None, 1.0).unwrap(), before, "the same look");
    }
    assert!(matches!(doc.rasterize_sticker_layer(StickerRef::Id(sid), live), Err(EditError::InvalidParam(_))));
    doc.undo();
    doc.undo();
    assert!(matches!(doc.sticker(sid).unwrap().layer(text).unwrap().content, StickerContent::Text(_)));
    let mut locked = (**doc.sticker(sid).unwrap()).clone();
    locked.layer_mut(text).unwrap().locked = true;
    let lid = doc.add_sticker(locked).unwrap();
    assert!(matches!(doc.rasterize_sticker_layer(StickerRef::Id(lid), text), Err(EditError::LayerLocked)));
}

/// `selection_in_content`: a whole-pixel move shifts the mask; a scaled, turned, flipped layer samples the selection
/// at its pixel centres (against a brute force over `LayerTransform::apply`).
#[test]
fn k7a_a_selection_seen_from_a_layer_follows_its_transform() {
    let artboard = IRect::new(-4, 3, 60, 51);
    let mut sel = AlphaBuf::new(64, 48);
    for (i, v) in sel.data.iter_mut().enumerate() {
        *v = ((i * 7) % 256) as u8;
    }
    let at = |x: i64, y: i64| if x < 0 || y < 0 || x >= 64 || y >= 48 { 0 } else { sel.data[y as usize * 64 + x as usize] };
    let moved = LayerTransform::moved(6.0, -2.0);
    let m = selection_in_content(&sel, artboard, &moved, [30, 20]);
    for y in 0..20i64 {
        for x in 0..30i64 {
            assert_eq!(m.data[(y * 30 + x) as usize], at(x + 10, y - 5), "{x}, {y}");
        }
    }
    let turned = LayerTransform { translate: Vec2::new(10.3, 7.7), scale: Vec2::new(-1.7, 0.6), rotation: 0.9 };
    let m = selection_in_content(&sel, artboard, &turned, [25, 33]);
    for y in 0..33u32 {
        for x in 0..25u32 {
            let q = turned.apply([25, 33], Vec2::new(x as f32 + 0.5, y as f32 + 0.5)) - Vec2::new(-4.0, 3.0);
            assert_eq!(m.data[(y * 25 + x) as usize], at(q.x.floor() as i64, q.y.floor() as i64));
        }
    }
    assert!(selection_in_content(&sel, artboard, &moved, [0, 5]).data.is_empty());
}

// ---- performance (8.4; ignored, serial, release) ----

/// One dab of size 64 on a 2048 x 2048 picture layer plus its dirty-rectangle composite: budget 2 ms. Entering stroke
/// mode (one full recomposite of a 2048-pixel sticker with a styled layer): budget 50 ms. The restyle job at the end
/// (full, 2048 x 2048): budget 300 ms; at Preview scale: 60 ms.
#[test]
#[ignore = "perf: run with --release --ignored perf_k7a"]
fn perf_k7a_dab_region_and_restyle() {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let mut s = StickerDoc::new("big", 2048, 2048);
    let bottom = s.alloc_layer();
    let p = owned(&mut doc, noise(2048, 2048, 1, Some(255)));
    s.layers.push(StickerLayer::new(bottom, "bottom", StickerContent::Picture(p)));
    let live = s.alloc_layer();
    let mut pic = PixelBuf::new(2048, 2048);
    for y in 600..1400 {
        for x in 600..1400 {
            pic.set(x, y, [200, 30, 30, 255]);
        }
    }
    let p = owned(&mut doc, pic);
    let mut l = StickerLayer::new(live, "live", StickerContent::Picture(p));
    l.styles = StylePreset::ClassicSticker.styles([2048, 2048]);
    s.layers.push(l);
    let sid = doc.add_sticker(s).unwrap();
    doc.add_stamp("big", StampSource::Sticker { sticker: sid }, uv([128.0, 128.0], [200.0, 200.0]), TextureSetId(0)).unwrap();
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    // The canvas's picture before the stroke (a render-cache hit: the layer cache stays cold, as in the app).
    doc.sticker_picture(sid, None, 1.0).unwrap();
    let b = StickerBrush { radius: 32.0, ..StickerBrush::default() };
    doc.sticker_stroke_begin(StickerRef::Id(sid), live, PixelTarget::Pixels, b).unwrap();
    let t = std::time::Instant::now();
    let full: CroppedBuf = doc.sticker_canvas_region(sid, IRect::from_size(2048, 2048), Some(live)).unwrap();
    let enter = ms(t);
    assert_eq!(full.rect, IRect::from_size(2048, 2048));
    let mut worst = 0.0f64;
    for k in 0..50 {
        let t = std::time::Instant::now();
        let r = doc.sticker_stroke_dab(Vec2::new(700.0 + 12.0 * k as f32, 1000.0), 1.0).unwrap();
        let out = doc.sticker_canvas_region(sid, r, Some(live)).unwrap();
        assert!(!out.is_empty());
        worst = worst.max(ms(t));
    }
    let preview = doc.sticker_preview_job(sid).unwrap().unwrap();
    let t = std::time::Instant::now();
    preview.run();
    let preview_ms = ms(t);
    doc.sticker_stroke_end().unwrap();
    let job = doc.sticker_canvas_job(sid).unwrap();
    let t = std::time::Instant::now();
    job.run();
    let restyle = ms(t);
    println!("perf_k7a: enter stroke mode {enter:.1} ms (50), worst dab + region {worst:.2} ms (2), preview restyle {preview_ms:.1} ms (60), end restyle {restyle:.1} ms (300)");
    assert!(enter < 75.0 && worst < 3.0 && preview_ms < 90.0 && restyle < 450.0, "over 150 % of a budget");
}
