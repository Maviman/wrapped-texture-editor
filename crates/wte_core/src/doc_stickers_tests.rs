// Tests of package P3a (STICKER_PLAN 7 "P3a", names `k3a_*`): the sticker document in `Document`, the stamp styles
// pipeline, the two placements, job freshness and the caches.

use std::sync::Arc;

use glam::{Vec2, Vec3};

use crate::layer::BlendMode;
use crate::composite::flatten_layers;
use crate::doc_stickers::{ShareScope, StickerRef};
use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::ids::{BlobId, LayerId, StampId, StickerId, StickerLayerId, TextureSetId};
use crate::image_io::{PngLevel, encode_png};
use crate::layer::{Layer, LayerKind};
use crate::math::{IRect, fnv1a64};
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::samples;
use crate::stamp::{BakeQuality, Placement, Projector, StampSource, SurfaceMode, UvPlacement, WrapOptions};
use crate::stamp_bake::bake_uv;
use crate::stamp_job::FinalBake;
use crate::sticker::pipeline::{SourceFrame, Styling, bake_placement_of, styled_mips, work_level};
use crate::sticker::render::{LayerKey, StickerRenderInput, render};
use crate::sticker::shape::{Paint, ShapeKind, ShapeSpec};
use crate::sticker::style::{Contour, Shadow, Stroke, StrokePos, StylePreset, StyleStack};
use crate::sticker::{ArtboardShape, LayerMask, PicturePixels, StickerContent, StickerDoc, StickerLayer, counters};
use crate::testutil::{XorShift, doc_from_raw};
use crate::text::{RasterQuality, TEXT_PLACEMENT_PX, TextAlign, TextFx, TextSpec};

/// The cache hash of `identity_doc` recorded with the P0 code (main b2fb8f5 + P0-P2, before any P3a change).
const P0_IDENTITY_HASH: u64 = 0x63d2_6d88_cd3f_46ee;

fn png(image: &PixelBuf) -> Arc<[u8]> {
    Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
}

/// Projector facing -Z onto the plane z = `z`, centred at (cx, cy), image up = +Y.
fn front(cx: f32, cy: f32, z: f32, half: Vec2, depth: f32) -> Projector {
    Projector::from_surface(Vec3::new(cx, cy, z), Vec3::Z, Vec3::Y, half, depth)
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

fn uv(set: u32, center: [f32; 2], size: [f32; 2], rotation: f32, flip_x: bool) -> UvPlacement {
    UvPlacement {
        set: TextureSetId(set),
        center_px: Vec2::from_array(center),
        size_px: Vec2::from_array(size),
        rotation,
        flip_x,
        flip_y: false,
        clip_to_islands: false,
    }
}

/// An anti-aliased disc of `rgba` with radius `r` in a transparent `size x size` picture.
fn disc(size: u32, r: f32, rgba: [u8; 4]) -> PixelBuf {
    let mut img = PixelBuf::new(size, size);
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let d = Vec2::new(x as f32 + 0.5 - c, y as f32 + 0.5 - c).length();
            let a = (r + 0.5 - d).clamp(0.0, 1.0);
            if a > 0.0 {
                img.set(x, y, [rgba[0], rgba[1], rgba[2], (rgba[3] as f32 * a + 0.5) as u8]);
            }
        }
    }
    img
}

fn noise(w: u32, h: u32, seed: u64) -> PixelBuf {
    let mut rng = XorShift::new(seed);
    let data = (0..w * h * 4).map(|_| rng.next_u32() as u8).collect();
    PixelBuf::from_raw(w, h, data).unwrap()
}

/// An owned picture with a fresh version of `doc`.
fn owned(doc: &mut Document, pixels: PixelBuf) -> PicturePixels {
    PicturePixels::Owned { pixels: Arc::new(pixels), version: doc.next_pixel_version() }
}

/// The identity document of k3a-1: image and text stamps, projected, wrapped and UV, on the two-material model (two
/// sets), every stamp with default styles, baked synchronously.
fn identity_doc_sized(size: u32) -> (Document, BlobId, Vec<StampId>) {
    let mut doc = doc_from_raw(samples::two_material_model(), size);
    let k = size as f32 / 128.0;
    let blob = doc.add_image_blob("stamp.png", png(&samples::sample_stamp_image(64))).unwrap();
    let image = StampSource::Image { blob };
    let mut ids = Vec::new();
    let p = front(0.0, -0.1, 0.5, Vec2::new(0.3, 0.3), 0.6);
    ids.push(doc.add_stamp("img-p", image.clone(), Placement::Projected(p), TextureSetId(0)).unwrap());
    let w = Projector { mode: SurfaceMode::Wrap, wrap: WrapOptions::default(), ..front(0.2, 0.0, 0.5, Vec2::new(0.5, 0.15), 0.6) };
    ids.push(doc.add_stamp("img-w", image.clone(), Placement::Projected(w), TextureSetId(0)).unwrap());
    let u = UvPlacement { flip_x: true, ..uv(1, [64.0 * k, 60.0 * k], [40.0 * k, 30.0 * k], 0.3, true) };
    ids.push(doc.add_stamp("img-uv", image, Placement::Uv(u), TextureSetId(1)).unwrap());
    let spec = text(&mut doc, "Hi P3");
    let source = StampSource::Text(spec);
    let tp = front(-0.1, 0.25, 0.5, Vec2::new(0.3, 0.1), 0.6);
    ids.push(doc.add_stamp("txt-p", source.clone(), Placement::Projected(tp), TextureSetId(0)).unwrap());
    let tw = Projector { mode: SurfaceMode::Wrap, ..front(-0.2, -0.3, 0.5, Vec2::new(0.4, 0.1), 0.6) };
    ids.push(doc.add_stamp("txt-w", source.clone(), Placement::Projected(tw), TextureSetId(0)).unwrap());
    let tuv = uv(1, [40.0 * k, 90.0 * k], [50.0 * k, 16.0 * k], 0.0, false);
    ids.push(doc.add_stamp("txt-uv", source, Placement::Uv(tuv), TextureSetId(1)).unwrap());
    (doc, blob, ids)
}

fn identity_doc() -> (Document, BlobId, Vec<StampId>) {
    identity_doc_sized(128)
}

/// One hash over every linked cache (as a dense picture of its set) and every composite.
fn cache_hash(doc: &mut Document) -> u64 {
    doc.recomposite_all();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for set in &doc.sets {
        for layer in &set.layers {
            if let LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } = &layer.kind {
                let dense = cache.to_dense(set.width, set.height);
                h = fnv1a64(h, layer.name.as_bytes());
                h = fnv1a64(h, &dense.data);
            }
        }
        h = fnv1a64(h, &set.composite.data);
    }
    h
}

/// The linked cache of stamp `id` in `set` as a dense picture of the set (transparent when there is none).
fn cache(doc: &Document, id: StampId, set: u32) -> PixelBuf {
    let s = doc.set(TextureSetId(set)).unwrap();
    s.layers
        .iter()
        .find(|l| l.stamp_id() == Some(id))
        .map_or_else(|| PixelBuf::new(s.width, s.height), |l| l.to_dense(s.width, s.height))
}

/// Bounds of the pixels with alpha above `min`.
fn ink(buf: &PixelBuf, min: u8) -> IRect {
    let mut r = IRect::EMPTY;
    for y in 0..buf.height {
        for x in 0..buf.width {
            if buf.get(x, y)[3] > min {
                let p = IRect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1);
                r = if r.is_empty() { p } else { r.union(&p) };
            }
        }
    }
    r
}

/// Bakes every waiting stamp the way the app does; returns the number of applied bakes.
fn pump(doc: &mut Document) -> usize {
    let mut applied = 0;
    while let Some(job) = doc.next_final_bake() {
        match doc.apply_final_bake(job.run()) {
            FinalBake::Applied { .. } => applied += 1,
            other => panic!("a pumped bake was not applied: {other:?}"),
        }
    }
    applied
}

fn contour(width: f32) -> StyleStack {
    StyleStack {
        contour: Some(Contour { on: true, width, smooth: 0.0, fill_holes: true, color: [255, 255, 255, 255], opacity: 1.0 }),
        ..StyleStack::default()
    }
}

fn shadow(distance: f32, size: f32) -> StyleStack {
    StyleStack {
        drop_shadow: Some(Shadow {
            on: true,
            color: [0, 0, 0, 255],
            opacity: 0.8,
            angle_deg: 120.0,
            distance,
            spread: 0.3,
            size,
        }),
        ..StyleStack::default()
    }
}

/// The stored placement's geometry without its depth.
fn geometry(p: &Placement) -> String {
    match p {
        Placement::Projected(p) => format!("{:?} {:?} {:?} {} {} {:?}", p.origin, p.rotation, p.half_size, p.flip_x, p.flip_y, p.mode),
        Placement::Uv(u) => format!("{u:?}"),
    }
}

// ----- 1: identity -----

#[test]
fn k3a_1_identity_promotion_and_bake_placement() {
    let (mut doc, _, ids) = identity_doc();
    assert_eq!(cache_hash(&mut doc), P0_IDENTITY_HASH, "default styles bake exactly as the P0 code");
    for &id in &ids {
        let stored = doc.stamp(id).unwrap().placement;
        let bake = doc.bake_placement(id).unwrap();
        assert_eq!(format!("{bake:?}"), format!("{stored:?}"), "margin 0: the stored placement bit for bit");
    }
    // The Preview of an image stamp, to compare with the promoted stamp's below.
    let Placement::Projected(p0) = doc.stamp(ids[0]).unwrap().placement else { panic!() };
    let moved = Placement::Projected(Projector { origin: p0.origin + Vec3::new(0.03, -0.02, 0.0), ..p0 });
    doc.set_stamp_placement(ids[0], moved, Commit::Preview).unwrap();
    let preview = cache(&doc, ids[0], 0);
    assert!(doc.cancel_pending_edit());
    // Promotion changes no cache byte and starts no bake.
    doc.set_defer_final_bakes(true);
    let (renders, undo) = (counters::renders(), doc.history.undo_len());
    for &id in &ids[..3] {
        doc.promote_stamp(id, ShareScope::ThisStamp).unwrap();
        assert!(matches!(doc.stamp(id).unwrap().source, StampSource::Sticker { .. }));
    }
    doc.debug_check_structure();
    assert!(!doc.has_unbaked_stamps(), "no bake is started");
    assert_eq!(counters::renders(), renders);
    assert_eq!(doc.history.undo_len(), undo + 3);
    assert_eq!(cache_hash(&mut doc), P0_IDENTITY_HASH);
    doc.set_stamp_placement(ids[0], moved, Commit::Preview).unwrap();
    assert_eq!(cache(&doc, ids[0], 0), preview, "a promoted stamp's Preview is the image stamp's");
    assert!(doc.cancel_pending_edit());
    // A bake of the promoted stamps (job path, deferred and synchronous) gives the same bytes: the trivial fast path.
    doc.set_defer_final_bakes(false);
    for &id in &ids[..3] {
        doc.rebake_stamp(id, BakeQuality::Full).unwrap();
    }
    assert_eq!(cache_hash(&mut doc), P0_IDENTITY_HASH, "a promoted stamp bakes bit for bit as the image stamp");
    assert_eq!(counters::renders(), renders, "nothing was rendered for a promoted image");
    // Undo brings the image stamps back.
    for _ in 0..3 {
        doc.undo();
    }
    assert!(ids[..3].iter().all(|id| matches!(doc.stamp(*id).unwrap().source, StampSource::Image { .. })));
    assert_eq!(cache_hash(&mut doc), P0_IDENTITY_HASH);
}

// ----- 2: a contour on a promoted stamp -----

#[test]
fn k3a_2_contour_grows_the_layer_and_keeps_the_artwork() {
    let (mut doc, _, ids) = identity_doc_sized(512);
    let id = ids[0];
    let before = cache_hash(&mut doc);
    let old = cache(&doc, id, 0);
    let stored = doc.stamp(id).unwrap().placement;
    doc.promote_stamp(id, ShareScope::ThisStamp).unwrap();
    doc.set_stamp_styles(id, contour(6.0), Commit::Final).unwrap();
    doc.debug_check_structure();
    assert_eq!(geometry(&doc.stamp(id).unwrap().placement), geometry(&stored), "the stored placement keeps its box");
    let new = cache(&doc, id, 0);
    let (b0, b1) = (ink(&old, 0), ink(&new, 0));
    assert!(b1.intersect(&b0) == b0 && b1.area() > b0.area(), "the linked layer grows: {b0:?} -> {b1:?}");
    // Where the artwork is opaque all around, the contour behind it changes nothing.
    let mut checked = 0;
    for y in 1..old.height - 1 {
        for x in 1..old.width - 1 {
            let opaque = (0..9).all(|k| old.get(x + k % 3 - 1, y + k / 3 - 1)[3] == 255);
            if opaque {
                checked += 1;
                let (a, b) = (old.get(x, y), new.get(x, y));
                assert!(a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 1), "texel ({x}, {y}): {a:?} -> {b:?}");
            }
        }
    }
    assert!(checked > 100, "only {checked} interior texels");
    doc.undo();
    doc.undo();
    assert_eq!(cache_hash(&mut doc), before, "undo restores the caches byte for byte");
    assert!(matches!(doc.stamp(id).unwrap().source, StampSource::Image { .. }));
}

// ----- 3: two placements -----

#[test]
fn k3a_3_two_placements_and_the_auto_depth() {
    let (mut doc, _, ids) = identity_doc_sized(256);
    let id = ids[1];
    assert!(doc.ensure_wrap_param(id).is_some());
    let outline0 = doc.stamp_wrap_outline(id, 6);
    let area0 = doc.stamp_ink_area(id);
    let stored = doc.stamp(id).unwrap().placement;
    doc.set_defer_final_bakes(true);
    doc.set_stamp_styles(id, shadow(6.0, 6.0), Commit::Final).unwrap();
    assert_eq!(geometry(&doc.stamp(id).unwrap().placement), geometry(&stored));
    let (Placement::Projected(s), Some(Placement::Projected(b))) = (stored, doc.bake_placement(id)) else { panic!() };
    assert!(b.half_size.x > s.half_size.x && b.half_size.y > s.half_size.y, "the bake placement is larger");
    assert!(doc.stamp_wrap_stats(id).is_none(), "the cached parameterization does not serve the bake placement");
    assert!(pump(&mut doc) >= 1);
    assert!(doc.stamp_wrap_stats(id).is_some(), "the re-parameterization serves");
    let outline1 = doc.stamp_wrap_outline(id, 6);
    assert_eq!(outline0.len(), outline1.len());
    for (a, b) in outline0.iter().flatten().zip(outline1.iter().flatten()) {
        assert!((a.position - b.position).length() < 2e-3, "the gizmo outline keeps the artwork: {} vs {}", a.position, b.position);
    }
    assert!(doc.stamp_ink_area(id) > area0 * 1.05, "the baked footprint is larger");

    // Auto depth on a curved sample mesh: refitted on the bake footprint; a hand-set depth is left alone.
    let mut doc = doc_from_raw(samples::uv_sphere(32, 16, 1.0), 256);
    let blob = doc.add_image_blob("d.png", png(&disc(64, 30.0, [200, 40, 40, 255]))).unwrap();
    let p = doc.fit_stamp_depth(&front(0.0, 0.0, 1.0, Vec2::splat(0.35), 0.1));
    let auto = doc.add_stamp("auto", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
    let hand = Projector { depth_front: 0.05, depth_back: 0.05, ..p };
    assert!(!doc.stamp_depth_is_auto(&hand));
    let manual = doc.add_stamp("hand", StampSource::Image { blob }, Placement::Projected(hand), TextureSetId(0)).unwrap();
    let ink0 = doc.stamp_ink_area(auto);
    let big = shadow(16.0, 16.0);
    doc.set_stamp_styles(auto, big.clone(), Commit::Final).unwrap();
    doc.set_stamp_styles(manual, big, Commit::Final).unwrap();
    let Placement::Projected(a) = doc.stamp(auto).unwrap().placement else { panic!() };
    let Some(Placement::Projected(ab)) = doc.bake_placement(auto) else { panic!() };
    assert!(a.depth_back > p.depth_back, "the depth follows the larger footprint: {} -> {}", p.depth_back, a.depth_back);
    assert!(doc.stamp_depth_is_auto(&ab), "the stored depth is the automatic depth of the bake placement");
    assert!(doc.stamp_ink_area(auto) > ink0 * 1.2, "the shadow is baked on the curved surface");
    let Placement::Projected(m) = doc.stamp(manual).unwrap().placement else { panic!() };
    assert_eq!((m.depth_front, m.depth_back), (0.05, 0.05), "a hand-set depth is kept");
}

// ----- 4: text stamp stroke -----

#[test]
fn k3a_4_a_text_stroke_is_measured_in_em() {
    let mut doc = doc_from_raw(samples::flat_quad(), 1024);
    for em in [64.0f32, 128.0, 256.0] {
        let spec = text(&mut doc, "I");
        let frame = doc.stamp_source_frame(&StampSource::Text(spec.clone())).unwrap();
        let k = em / frame.em_px;
        let p = uv(0, [512.0, 512.0], [frame.size[0] as f32 * k, frame.size[1] as f32 * k], 0.0, false);
        let id = doc.add_stamp("I", StampSource::Text(spec), Placement::Uv(p), TextureSetId(0)).unwrap();
        let StampSource::Text(drawn) = &doc.stamp(id).unwrap().source else { panic!() };
        assert_eq!(drawn.size_px, em, "the raster size follows the em on the model");
        let b0 = ink(&cache(&doc, id, 0), 127);
        let stroke = StyleStack {
            stroke: Some(Stroke { on: true, width: 8.0, position: StrokePos::Outside, color: [0, 0, 0, 255], opacity: 1.0 }),
            ..StyleStack::default()
        };
        doc.set_stamp_styles(id, stroke, Commit::Final).unwrap();
        let b1 = ink(&cache(&doc, id, 0), 127);
        let grow = (b1.width() as f32 - b0.width() as f32) / 2.0;
        let expected = 8.0 * em / 128.0;
        assert!((grow - expected).abs() <= 1.0, "em {em}: the stroke is {grow} texels, expected {expected}");
        doc.remove_stamp(id).unwrap();
    }
}

// ----- 5: working size -----

#[test]
fn k3a_5_styles_never_draw_more_pixels_than_the_model_shows() {
    let mut doc = doc_from_raw(samples::flat_quad(), 512);
    let blob = doc.add_image_blob("big.png", png(&disc(8192, 3000.0, [30, 120, 220, 255]))).unwrap();
    let id = doc.add_stamp("big", StampSource::Image { blob }, Placement::Uv(uv(0, [256.0, 256.0], [400.0, 400.0], 0.0, false)), TextureSetId(0)).unwrap();
    doc.set_defer_final_bakes(true);
    let calls = counters::styled_calls();
    doc.set_stamp_styles(id, StylePreset::ClassicSticker.styles([8192, 8192]), Commit::Final).unwrap();
    assert_eq!(counters::styled_calls(), calls, "a Final styles nothing on the calling thread");
    let job = doc.next_final_bake().expect("the styled stamp waits for its job");
    let (out, side, worker_calls) = std::thread::spawn(move || {
        counters::reset_max();
        let out = job.run();
        (out, counters::max_styled_side(), counters::styled_calls())
    })
    .join()
    .unwrap();
    assert!(worker_calls >= 1 && side > 0 && side <= 4096, "styled on the worker at {side} px");
    assert!(matches!(doc.apply_final_bake(out), FinalBake::Applied { .. }));
    // A Preview is styled on this thread at the Preview working size.
    counters::reset_max();
    doc.set_stamp_styles(id, StylePreset::Badge.styles([8192, 8192]), Commit::Preview).unwrap();
    let px = counters::max_styled_pixels();
    assert!(px > 0 && px <= 524_288, "the Preview styled {px} pixels");
    assert!(counters::max_styled_side() <= 4096);
    assert!(doc.cancel_pending_edit());

    // The styled width on the model is the same within 1 texel for working scales 1, 1/2 and 1/4.
    let mut doc = doc_from_raw(samples::flat_quad(), 512);
    let base = disc(1024, 300.0, [255, 0, 0, 255]);
    let mips = crate::pixel_ops::PremulMips::build(&base);
    let p = uv(0, [256.0, 256.0], [256.0, 256.0], 0.0, false);
    let map = Arc::clone(&doc.set(TextureSetId(0)).unwrap().texel_map);
    let mut widths = Vec::new();
    for level in 0..3u32 {
        let texels = Vec2::splat(1024.0 / (1u32 << level) as f32 / 2.0);
        let styling = Styling { styles: contour(40.0), tint: [255; 4], ref_scale: 1.0, texels, quality: BakeQuality::Full };
        assert_eq!(work_level([1024, 1024], &styling).0, level);
        let (styled, frame, _) = styled_mips(&mips, &styling);
        let Placement::Uv(bp) = bake_placement_of(&Placement::Uv(p), &frame) else { panic!() };
        let baked = bake_uv(&styled, [255; 4], &bp, &map).to_dense(512, 512);
        widths.push(ink(&baked, 127).width() as f32);
    }
    let expected = 2.0 * 340.0 / 1024.0 * 256.0;
    for w in &widths {
        assert!((w - widths[0]).abs() <= 1.0 && (w - expected).abs() <= 2.0, "widths {widths:?}, expected {expected}");
    }
    let _ = &mut doc;
}

// ----- 6: a crop keeps layer points on the model -----

/// The centroid (texels) of the strongly red texels of stamp `id`'s cache in `set`.
fn red_centroid(doc: &Document, id: StampId, set: u32) -> Vec2 {
    let c = cache(doc, id, set);
    let (mut sum, mut n) = (Vec2::ZERO, 0.0);
    for y in 0..c.height {
        for x in 0..c.width {
            let p = c.get(x, y);
            if p[3] > 200 && p[0] > 200 && p[1] < 80 {
                sum += Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                n += 1.0;
            }
        }
    }
    assert!(n > 0.0, "no red texel");
    sum / n
}

/// A 64 x 64 sticker picture: a soft grey ring and a red 4 x 4 block at (40..44, 20..24).
fn marked_picture() -> PixelBuf {
    let mut img = disc(64, 28.0, [90, 90, 90, 160]);
    for y in 20..24 {
        for x in 40..44 {
            img.set(x, y, [255, 0, 0, 255]);
        }
    }
    img
}

#[test]
fn k3a_6_a_crop_keeps_every_layer_point_on_the_model() {
    let cases: Vec<(&str, Placement)> = vec![
        ("uv", Placement::Uv(uv(0, [256.0, 256.0], [256.0, 256.0], 0.0, false))),
        ("uv turned and flipped", Placement::Uv(uv(0, [250.0, 270.0], [220.0, 240.0], 0.5, true))),
        ("projected", Placement::Projected(front(0.5, 0.5, 0.0, Vec2::splat(0.3), 0.2))),
        ("projected turned and flipped", {
            let p = Projector::from_surface(Vec3::new(0.5, 0.5, 0.0), Vec3::Z, Vec3::new(0.5, 0.86, 0.0), Vec2::new(0.3, 0.25), 0.2);
            Placement::Projected(Projector { flip_x: true, ..p })
        }),
    ];
    for (name, placement) in cases {
        let mut doc = doc_from_raw(samples::flat_quad(), 512);
        let picture = owned(&mut doc, marked_picture());
        let sid = doc.add_sticker(StickerDoc::from_picture("marked", picture, [64, 64])).unwrap();
        let id = doc.add_stamp("m", StampSource::Sticker { sticker: sid }, placement, TextureSetId(0)).unwrap();
        let p0 = red_centroid(&doc, id, 0);
        doc.set_sticker(StickerRef::Id(sid), |d| d.artboard = IRect::new(16, 8, 60, 50), "Crop", Commit::Final).unwrap();
        doc.debug_check_structure();
        let p1 = red_centroid(&doc, id, 0);
        assert!((p1 - p0).length() <= 1.0, "{name}: the marked point moved from {p0} to {p1}");
        let undo = doc.undo_label().map(str::to_owned);
        assert_eq!(undo.as_deref(), Some("Crop"));
    }
}

// ----- 7: a shared sticker -----

#[test]
fn k3a_7_a_shared_sticker_bakes_all_users_and_detach_parts_them() {
    let mut doc = doc_from_raw(samples::two_material_model(), 128);
    let blob = doc.add_image_blob("s.png", png(&samples::sample_stamp_image(64))).unwrap();
    let a = doc.add_stamp("a", StampSource::Image { blob }, Placement::Projected(front(-0.2, 0.0, 0.5, Vec2::splat(0.2), 0.6)), TextureSetId(0)).unwrap();
    let b = doc.add_stamp("b", StampSource::Image { blob }, Placement::Projected(front(0.2, 0.0, 0.5, Vec2::splat(0.2), 0.6)), TextureSetId(0)).unwrap();
    let sid = doc.promote_stamp(a, ShareScope::AllUsers).unwrap();
    assert_eq!(doc.sticker_users(sid), vec![a, b]);
    assert!(doc.blob_users(blob).is_empty());
    let (a0, b0) = (cache(&doc, a, 0), cache(&doc, b, 0));
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.5, "Opacity", Commit::Final).unwrap();
    let (a1, b1) = (cache(&doc, a, 0), cache(&doc, b, 0));
    assert!(a1 != a0 && b1 != b0, "one edit re-bakes both users");
    let sid2 = doc.detach_sticker(b).unwrap();
    assert_ne!(sid2, sid);
    assert_eq!(doc.stamp(b).unwrap().source, StampSource::Sticker { sticker: sid2 });
    assert_eq!(cache(&doc, b, 0), b1, "detaching draws nothing new");
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.25, "Opacity", Commit::Final).unwrap();
    assert_ne!(cache(&doc, a, 0), a1);
    assert_eq!(cache(&doc, b, 0), b1, "the detached stamp keeps its own sticker");
    // A locked user protects the sticker; a sticker in use cannot be deleted.
    let layer = doc.stamp_layers(a)[0].1;
    doc.set_layer_locked(layer, true).unwrap();
    assert!(matches!(
        doc.set_sticker(StickerRef::Id(sid), |d| d.name = "x".into(), "Rename", Commit::Final),
        Err(EditError::LayerLocked)
    ));
    assert!(matches!(doc.remove_sticker(sid2), Err(EditError::InvalidParam(_))));
}

// ----- 8: history -----

#[test]
fn k3a_8_history_shares_pictures_and_counts_them_once() {
    let mut doc = doc_from_raw(samples::flat_quad(), 64);
    let picture = owned(&mut doc, noise(256, 256, 1));
    let sid = doc.add_sticker(StickerDoc::from_picture("tray", picture, [256, 256])).unwrap();
    let pixels = |doc: &Document| Arc::clone(doc.sticker(sid).unwrap().layers[0].owned_picture().unwrap());
    let first = pixels(&doc);
    let base = doc.history_bytes();
    // A rename copies no pixel: the snapshot shares the buffer.
    doc.set_sticker(StickerRef::Id(sid), |d| d.name = "renamed".into(), "Rename Sticker", Commit::Final).unwrap();
    assert!(Arc::ptr_eq(&first, &pixels(&doc)));
    assert_eq!(doc.history_bytes(), base, "a shared buffer costs nothing");
    // A replaced picture is counted once, also when two snapshots hold it.
    let new = Arc::new(noise(256, 256, 2));
    doc.set_sticker(
        StickerRef::Id(sid),
        |d| d.layers[0].content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::clone(&new), version: 0 }),
        "Replace",
        Commit::Final,
    )
    .unwrap();
    let PicturePixels::Owned { version, .. } = (match &doc.sticker(sid).unwrap().layers[0].content {
        StickerContent::Picture(p) => p.clone(),
        _ => panic!(),
    }) else {
        panic!()
    };
    assert!(version > 0, "a new buffer gets a fresh version");
    assert_eq!(doc.history_bytes(), base + first.byte_len());
    doc.set_sticker(StickerRef::Id(sid), |d| d.name = "again".into(), "Rename Sticker", Commit::Final).unwrap();
    assert_eq!(doc.history_bytes(), base + first.byte_len(), "counted once");
    // Undo brings the first buffer back (the same Arc).
    doc.undo();
    doc.undo();
    assert!(Arc::ptr_eq(&first, &pixels(&doc)));
    doc.redo();
    doc.redo();
    // Preview edits fold into one entry; a cancel restores.
    let undo = doc.history.undo_len();
    for o in [0.9, 0.8, 0.7] {
        doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = o, "Layer Opacity", Commit::Preview).unwrap();
    }
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.6, "Layer Opacity", Commit::Final).unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1);
    let before = Arc::clone(doc.sticker(sid).unwrap());
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.1, "Layer Opacity", Commit::Preview).unwrap();
    assert!(doc.cancel_pending_edit());
    assert!(Arc::ptr_eq(&before, doc.sticker(sid).unwrap()));
    // Eviction under a small budget keeps the document consistent.
    doc.history.budget_bytes = 300_000;
    for seed in 3..9 {
        let next = Arc::new(noise(256, 256, seed));
        doc.set_sticker(
            StickerRef::Id(sid),
            |d| d.layers[0].content = StickerContent::Picture(PicturePixels::Owned { pixels: next, version: 0 }),
            "Replace",
            Commit::Final,
        )
        .unwrap();
        doc.debug_check_structure();
    }
    assert!(doc.history_bytes() <= 300_000 + 262_144, "{}", doc.history_bytes());
    while doc.undo().is_some() {
        doc.debug_check_structure();
    }
}

// ----- 9: job freshness -----

/// A projected sticker stamp whose sticker renders (opacity 0.8, not the trivial path).
fn sticker_doc(wrapped: bool) -> (Document, StickerId, StampId) {
    let mut doc = doc_from_raw(samples::two_material_model(), 128);
    let picture = owned(&mut doc, samples::sample_stamp_image(64));
    let mut sticker = StickerDoc::from_picture("s", picture, [64, 64]);
    sticker.layers[0].opacity = 0.8;
    let sid = doc.add_sticker(sticker).unwrap();
    let p = front(0.1, 0.0, 0.5, Vec2::new(0.3, 0.2), 0.6);
    let p = if wrapped { Projector { mode: SurfaceMode::Wrap, ..p } } else { p };
    let id = doc.add_stamp("s", StampSource::Sticker { sticker: sid }, Placement::Projected(p), TextureSetId(0)).unwrap();
    (doc, sid, id)
}

#[test]
fn k3a_9_a_job_of_an_older_sticker_is_stale() {
    let (mut doc, sid, id) = sticker_doc(false);
    doc.set_defer_final_bakes(true);
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.6, "Layer Opacity", Commit::Final).unwrap();
    assert!(doc.stamp_is_unbaked(id));
    let job = doc.next_final_bake().unwrap();
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.4, "Layer Opacity", Commit::Final).unwrap();
    assert!(matches!(doc.apply_final_bake(job.run()), FinalBake::Stale), "the stamp object is the same, the sticker is not");
    assert!(doc.stamp_is_unbaked(id));
    assert_eq!(pump(&mut doc), 1);
    let applied = cache(&doc, id, 0);
    doc.set_defer_final_bakes(false);
    doc.rebake_stamp(id, BakeQuality::Full).unwrap();
    assert_eq!(cache(&doc, id, 0), applied, "the job's result is the synchronous bake of the new sticker");

    // Undo while a bake is running never applies a stale result.
    doc.set_defer_final_bakes(true);
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.3, "Layer Opacity", Commit::Final).unwrap();
    let job = doc.next_final_bake().unwrap();
    doc.undo();
    assert!(matches!(doc.apply_final_bake(job.run()), FinalBake::Stale));
    assert_eq!(cache(&doc, id, 0), applied);

    // A job applied without an edit in between fills the caches: the next Preview draws nothing.
    doc.set_stamp_styles(id, contour(4.0), Commit::Final).unwrap();
    let job = doc.next_final_bake().unwrap();
    assert!(matches!(doc.apply_final_bake(job.run()), FinalBake::Applied { .. }));
    let (renders, styled) = (counters::renders(), counters::styled_calls());
    let Placement::Projected(p) = doc.stamp(id).unwrap().placement else { panic!() };
    let moved = Projector { origin: p.origin + Vec3::new(0.01, 0.0, 0.0), ..p };
    doc.set_stamp_placement(id, Placement::Projected(moved), Commit::Preview).unwrap();
    assert_eq!((counters::renders(), counters::styled_calls()), (renders, styled), "the Preview used the job's pictures");
    assert!(doc.cancel_pending_edit());

    // Wrapped Preview jobs: a changed sticker makes the applied picture the last good one (due again).
    let (mut doc, sid, id) = sticker_doc(true);
    doc.set_defer_wrap_previews(true);
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.5, "Layer Opacity", Commit::Preview).unwrap();
    let job = doc.next_wrap_preview_job().unwrap();
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.3, "Layer Opacity", Commit::Preview).unwrap();
    assert_eq!(doc.apply_wrap_preview(job.run()), crate::stamp_job::WrapPreview::Applied { due_again: true });
    let job = doc.next_wrap_preview_job().unwrap();
    assert_eq!(doc.apply_wrap_preview(job.run()), crate::stamp_job::WrapPreview::Applied { due_again: false });
    assert!(doc.cancel_pending_edit());
    let _ = id;
}

// ----- 10: caches -----

#[test]
fn k3a_10_caches_never_hand_out_another_stickers_picture() {
    let mut doc = doc_from_raw(samples::flat_quad(), 32);
    for i in 0..1000u32 {
        let colour = [(i % 251) as u8, (i / 251) as u8, 77, 255];
        let picture = owned(&mut doc, PixelBuf::filled(6, 5, colour));
        let mut sticker = StickerDoc::from_picture("c", picture, [6, 5]);
        sticker.layers[0].opacity = if i % 2 == 0 { 1.0 } else { 0.5 };
        let sid = doc.add_sticker(sticker).unwrap();
        let shown = doc.sticker_picture(sid, None, 1.0).unwrap();
        let want = if i % 2 == 0 { colour } else { crate::blend::blend_pixel([0; 4], colour, 128, BlendMode::Normal) };
        assert!(shown.data.as_chunks::<4>().0.iter().all(|p| *p == want), "cycle {i}: {:?} instead of {want:?}", &shown.data[..4]);
        // A second lookup is a cache hit with the same bytes.
        assert_eq!(doc.sticker_picture(sid, None, 1.0).unwrap(), shown);
        doc.remove_sticker(sid).unwrap();
        if i % 10 == 9 {
            doc.history.clear();
        }
    }
    // The per-layer key reads versions, never pixels.
    let a = Arc::new(PixelBuf::filled(4, 4, [1, 2, 3, 4]));
    let b = Arc::new(PixelBuf::filled(4, 4, [9, 9, 9, 9]));
    let layer = |p: &Arc<PixelBuf>, v| {
        StickerLayer::new(StickerLayerId(1), "l", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::clone(p), version: v }))
    };
    let key = |l: &StickerLayer| LayerKey::of(l, IRect::from_size(4, 4), 1.0, RasterQuality::Full);
    assert_eq!(key(&layer(&a, 7)), key(&layer(&b, 7)), "equal versions: equal keys whatever the bytes");
    assert_ne!(key(&layer(&a, 7)), key(&layer(&a, 8)), "a new version: a new key");
}

// ----- 11: render -----

fn input(doc: StickerDoc) -> StickerRenderInput {
    StickerRenderInput { doc: Arc::new(doc), pictures: Vec::new(), fonts: Vec::new() }
}

fn picture_layer(id: u32, pixels: PixelBuf, version: u64) -> StickerLayer {
    StickerLayer::new(StickerLayerId(id), "p", StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(pixels), version }))
}

#[test]
fn k3a_11_render_matches_the_layer_compositor_and_draws_clips_masks_and_shapes() {
    // Layer order, opacity and every blend mode against `flatten_layers` on the same pixels.
    let (lower, upper) = (noise(16, 12, 5), noise(16, 12, 6));
    for mode in BlendMode::ALL {
        let mut s = StickerDoc::new("b", 16, 12);
        s.layers.push(picture_layer(1, lower.clone(), 1));
        let mut top = picture_layer(2, upper.clone(), 2);
        top.opacity = 0.6;
        top.blend = mode;
        s.layers.push(top);
        s.next_layer = 3;
        let mut l0 = Layer::new(LayerId(1), "a", LayerKind::Raster(Arc::new(lower.clone())));
        let mut l1 = Layer::new(LayerId(2), "b", LayerKind::Raster(Arc::new(upper.clone())));
        l0.opacity = 1.0;
        l1.opacity = 0.6;
        l1.blend = mode;
        let want = flatten_layers(&[l0, l1], 16, 12, false);
        assert_eq!(render(&input(s), 1.0, RasterQuality::Full), want, "{mode:?}");
    }
    // The trivial fast path returns the picture's bytes.
    let pic = noise(9, 7, 11);
    let s = StickerDoc { layers: vec![picture_layer(1, pic.clone(), 3)], next_layer: 2, ..StickerDoc::new("t", 9, 7) };
    assert_eq!(render(&input(s.clone()), 1.0, RasterQuality::Full), pic);
    // Deterministic for any thread count.
    let mut turned = s.clone();
    turned.layers[0].transform.rotation = 0.4;
    turned.layers[0].transform.scale = Vec2::new(0.7, 1.3);
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| render(&input(turned.clone()), 2.0, RasterQuality::Full));
    let eight = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap().install(|| render(&input(turned.clone()), 2.0, RasterQuality::Full));
    assert_eq!(one, eight);
    // A mask multiplies the alpha.
    let mut masked = s.clone();
    let mask = AlphaBuf::from_raw(9, 7, (0..63).map(|i| (i * 4) as u8).collect()).unwrap();
    masked.layers[0].mask = Some(LayerMask { pixels: Arc::new(mask.clone()), version: 9 });
    let out = render(&input(masked), 1.0, RasterQuality::Full);
    for (i, (o, p)) in out.data.as_chunks::<4>().0.iter().zip(pic.data.as_chunks::<4>().0.iter()).enumerate() {
        assert_eq!(o[3], crate::math::mul_u8(p[3], mask.data[i]));
    }
    // The artboard shape crops.
    let solid = StickerDoc {
        layers: vec![picture_layer(1, PixelBuf::filled(40, 30, [10, 200, 30, 255]), 4)],
        next_layer: 2,
        shape: ArtboardShape::Ellipse,
        ..StickerDoc::new("e", 40, 30)
    };
    let out = render(&input(solid), 1.0, RasterQuality::Full);
    assert_eq!(out.get(0, 0)[3], 0);
    assert_eq!(out.get(20, 15)[3], 255);
    // Scale 2 doubles a shape's size exactly.
    let rect = ShapeSpec { kind: ShapeKind::Rect, size: Vec2::new(10.0, 6.0), fill: Paint::Solid([0, 0, 255, 255]), outline: None };
    let mut shape = StickerLayer::new(StickerLayerId(1), "r", StickerContent::Shape(rect));
    shape.transform.translate = Vec2::new(3.0, 4.0);
    let s = StickerDoc { layers: vec![shape], next_layer: 2, ..StickerDoc::new("r", 20, 20) };
    assert_eq!(ink(&render(&input(s.clone()), 1.0, RasterQuality::Full), 0), IRect::new(3, 4, 13, 10));
    assert_eq!(ink(&render(&input(s), 2.0, RasterQuality::Full), 0), IRect::new(6, 8, 26, 20));
}

#[test]
fn k3a_11_a_clipped_pattern_shows_only_inside_its_text() {
    let mut doc = doc_from_raw(samples::flat_quad(), 32);
    let mut spec = text(&mut doc, "OK");
    spec.size_px = 48.0;
    spec.color = [20, 20, 20, 255];
    let mut letters = StickerLayer::new(StickerLayerId(1), "text", StickerContent::Text(Box::new(spec)));
    letters.transform.translate = Vec2::new(60.0, 40.0);
    let alone = StickerDoc { layers: vec![letters.clone()], next_layer: 2, ..StickerDoc::new("t", 120, 80) };
    let mut pattern = picture_layer(2, noise(120, 80, 3), 0);
    pattern.clip = true;
    let mut clipped = StickerDoc { layers: vec![letters, pattern], next_layer: 3, ..StickerDoc::new("c", 120, 80) };
    for l in &mut clipped.layers {
        if let StickerContent::Picture(PicturePixels::Owned { pixels, .. }) = &mut l.content {
            Arc::make_mut(pixels).data.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
        }
    }
    let t = doc.add_sticker(alone).unwrap();
    let c = doc.add_sticker(clipped).unwrap();
    let base = doc.sticker_picture(t, None, 1.0).unwrap();
    let out = doc.sticker_picture(c, None, 1.0).unwrap();
    assert!(base.data.as_chunks::<4>().0.iter().any(|p| p[3] == 255), "the text draws");
    let mut changed = 0;
    for (o, b) in out.data.as_chunks::<4>().0.iter().zip(base.data.as_chunks::<4>().0.iter()) {
        assert!(b[3] > 0 || o[3] == 0, "the pattern shows only where the text is");
        changed += usize::from(o != b);
    }
    assert!(changed > 100, "the pattern covers the letters ({changed} pixels)");
    // A hidden base hides its clipped layer.
    doc.set_sticker(StickerRef::Id(c), |d| d.layers[0].visible = false, "Hide", Commit::Final).unwrap();
    let out = doc.sticker_picture(c, None, 1.0).unwrap();
    assert!(out.data.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    // A shrunk picture asks for a render scale above 1.
    let mut shrunk = StickerDoc::from_picture("s", PicturePixels::Blob(BlobId(0)), [100, 100]);
    shrunk.layers[0].transform.scale = Vec2::splat(0.5);
    assert!(crate::sticker::pipeline::sticker_scale(&shrunk, 300.0, BakeQuality::Full) > 1.0);
}

// ----- the rest of 2.8 -----

#[test]
fn k3a_finished_pictures_paste_style_and_save() {
    let (mut doc, blob, ids) = identity_doc();
    let img = crate::image_io::decode_image(&doc.blobs.get(blob).unwrap().bytes, None, 8192).unwrap().pixels;
    assert_eq!(doc.stamp_picture(ids[0], true).unwrap(), img, "without styles: the image itself");
    // Paste Style: one entry, every stamp styled; the finished picture grows by the margin.
    let undo = doc.history.undo_len();
    let styles = contour(5.0);
    doc.set_stamps_styles(&[ids[0], ids[3]], &styles).unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1);
    assert_eq!(doc.undo_label(), Some("Paste Style"));
    assert!(doc.stamp(ids[0]).unwrap().styles == styles && doc.stamp(ids[3]).unwrap().styles == styles);
    let m = styles.margin(1.0);
    let styled = doc.stamp_picture(ids[0], true).unwrap();
    assert_eq!([styled.width, styled.height], [64 + 2 * m, 64 + 2 * m]);
    assert_eq!(doc.stamp_picture(ids[0], false).unwrap(), img);
    let frame = doc.stamp_source_frame2(ids[0]).unwrap();
    assert_eq!(frame, SourceFrame { content: [64, 64], margin: m });
    // A sticker of a tray: its picture with styles.
    let picture = owned(&mut doc, disc(32, 12.0, [0, 0, 0, 255]));
    let sid = doc.add_sticker(StickerDoc::from_picture("tray", picture, [32, 32])).unwrap();
    let plain = doc.sticker_picture(sid, None, 1.0).unwrap();
    let decorated = doc.sticker_picture(sid, Some(&styles), 2.0).unwrap();
    assert_eq!(plain.width, 32);
    assert_eq!(decorated.width, 64 + 2 * styles.margin(2.0));
    // Projects with stickers and styles are written as format 4 since package P4 (P3a refused them here; the round
    // trip itself is tested in `project::sticker_tests`).
    let dir = crate::testutil::temp_dir("k3a_save");
    let opts = crate::project::SaveOptions {
        png: PngLevel::Fast,
        write_flattened: false,
        keep_backup: false,
        now_unix: 0,
        app_version: "test".into(),
    };
    let result = crate::project::save(&doc.snapshot(), &dir.join("s.wtep"), &opts, &crate::progress::NoProgress);
    assert!(result.is_ok(), "{result:?}");
    let manifest = crate::project::read_manifest(&dir.join("s.wtep")).unwrap();
    assert_eq!(manifest.min_reader_version, 4, "stamp styles need format 4");
    std::fs::remove_dir_all(&dir).unwrap();
    // Unknown ids.
    assert!(matches!(doc.remove_sticker(StickerId(999_999)), Err(EditError::UnknownSticker)));
    assert!(matches!(doc.promote_stamp(ids[3], ShareScope::ThisStamp), Err(EditError::InvalidParam(_))));
}

// ----- check P3a -----

/// Mean absolute difference of two equal-size pictures, all channels.
fn mean_diff(a: &PixelBuf, b: &PixelBuf) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let sum: u64 = a.data.iter().zip(&b.data).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.data.len() as f64
}

#[test]
fn check_p3a_a_styled_text_preview_keeps_its_stroke_width() {
    // A large slanted text: its Preview picture is drawn smaller than its frame (FEATURE_TEXT 5.8).
    let mut doc = doc_from_raw(samples::flat_quad(), 2048);
    let mut spec = text(&mut doc, "WWWWW");
    spec.fx.slant_deg = 10.0;
    let frame = doc.stamp_source_frame(&StampSource::Text(spec.clone())).unwrap();
    let k = 512.0 / frame.em_px;
    let p = uv(0, [1024.0, 1024.0], [frame.size[0] as f32 * k, frame.size[1] as f32 * k], 0.0, false);
    let id = doc.add_stamp("W", StampSource::Text(spec), Placement::Uv(p), TextureSetId(0)).unwrap();
    let StampSource::Text(drawn) = doc.stamp(id).unwrap().source.clone() else { panic!() };
    assert_eq!(drawn.size_px, 512.0);
    let full_frame = doc.stamp_text_frame(id).unwrap();
    assert!(full_frame.size[0] as u64 * full_frame.size[1] as u64 > 1 << 20, "the text is large enough: {:?}", full_frame.size);
    let stroke = StyleStack {
        stroke: Some(Stroke { on: true, width: 8.0, position: StrokePos::Outside, color: [0, 0, 0, 255], opacity: 1.0 }),
        ..StyleStack::default()
    };
    doc.set_stamp_styles(id, stroke, Commit::Final).unwrap();
    let final_ink = ink(&cache(&doc, id, 0), 127);
    // A colour change while typing: a Preview spec change draws the Preview picture.
    let recoloured = TextSpec { color: [20, 200, 240, 255], ..drawn };
    doc.set_stamp_source(id, StampSource::Text(recoloured), Commit::Preview).unwrap();
    let preview_size = doc.text_mips[&id].1.size();
    assert_ne!(preview_size, full_frame.size, "the Preview picture is the reduced one");
    let preview_ink = ink(&cache(&doc, id, 0), 127);
    for (a, b) in [(preview_ink.x0, final_ink.x0), (preview_ink.y0, final_ink.y0), (preview_ink.x1, final_ink.x1), (preview_ink.y1, final_ink.y1)] {
        assert!((a - b).abs() <= 3, "the Preview's stroke has the Final's width: {preview_ink:?} vs {final_ink:?}");
    }
    assert!(doc.cancel_pending_edit());
}

#[test]
fn check_p3a_a_large_styled_text_layer_renders_in_place_in_previews() {
    let mut doc = doc_from_raw(samples::flat_quad(), 32);
    let mut spec = text(&mut doc, "WWWWWW");
    spec.size_px = 480.0;
    spec.fx.slant_deg = 10.0;
    spec.color = [200, 30, 30, 255];
    let frame = doc.stamp_source_frame(&StampSource::Text(spec.clone())).unwrap();
    assert!(frame.size[0] as u64 * frame.size[1] as u64 > 1 << 20, "the text is large enough: {:?}", frame.size);
    let mut layer = StickerLayer::new(StickerLayerId(1), "text", StickerContent::Text(Box::new(spec)));
    layer.transform.translate = Vec2::new(512.0, 512.0);
    let sid = doc.add_sticker(StickerDoc { layers: vec![layer], next_layer: 2, ..StickerDoc::new("t", 1024, 1024) }).unwrap();
    let sticker = Arc::clone(doc.sticker(sid).unwrap());
    let full = doc.sticker_render(&sticker, 1.0, BakeQuality::Full).unwrap().picture;
    let preview = doc.sticker_render(&sticker, 1.0, BakeQuality::Preview).unwrap().picture;
    let (f, p) = (ink(&full, 127), ink(&preview, 127));
    assert!(!f.is_empty());
    for (a, b) in [(p.x0, f.x0), (p.y0, f.y0), (p.x1, f.x1), (p.y1, f.y1)] {
        assert!((a - b).abs() <= 3, "the Preview text sits where the Full one does: {p:?} vs {f:?}");
    }
    let d = mean_diff(&full, &preview);
    assert!(d < 12.0, "mean difference {d}");
}

#[test]
fn check_p3a_a_promoted_photo_shares_its_pixels_in_finished_pictures() {
    let (mut doc, blob, ids) = identity_doc();
    let sid = doc.promote_stamp(ids[0], ShareScope::ThisStamp).unwrap();
    let sticker = Arc::clone(doc.sticker(sid).unwrap());
    let renders = counters::renders();
    let entry = doc.sticker_render(&sticker, 1.0, BakeQuality::Full).unwrap();
    assert_eq!(counters::renders(), renders, "nothing is drawn for the trivial fast path");
    assert!(Arc::ptr_eq(&entry.picture, &doc.blob_picture(blob).unwrap()), "the decoded photo is shared, not copied");
    assert!(Arc::ptr_eq(&entry.mips, &doc.image_source(blob).unwrap()), "the image mips are shared, not built again");
    // The same bytes as the renderer's own fast path.
    let input = doc.sticker_render_input(&sticker).unwrap();
    assert_eq!(*entry.picture, render(&input, 1.0, RasterQuality::Full));
    let img = crate::image_io::decode_image(&doc.blobs.get(blob).unwrap().bytes, None, 8192).unwrap().pixels;
    assert_eq!(doc.stamp_picture(ids[0], true).unwrap(), img, "the finished picture of a promoted stamp is the image");
}

/// The cache hash of `identity_doc_sized(512)` on the code before P3a (9b0d9be), measured by the P3a checker with
/// the same document and hash built on that commit.
const P0_IDENTITY_HASH_512: u64 = 0x1d96_8db0_2d9a_6712;

#[test]
fn check_p3a_identity_at_512_with_styles_switched_off_and_promoted() {
    let (mut doc, _, ids) = identity_doc_sized(512);
    assert_eq!(cache_hash(&mut doc), P0_IDENTITY_HASH_512, "default styles bake exactly as the pre-P3a code");
    // Entries kept but switched off draw nothing: the bit-identical path.
    let mut off = StylePreset::ClassicSticker.styles([64, 64]);
    if let Some(c) = off.contour.as_mut() {
        c.on = false;
    }
    if let Some(s) = off.drop_shadow.as_mut() {
        s.on = false;
    }
    off.stroke = Some(Stroke { on: false, width: 6.0, position: StrokePos::Outside, color: [0, 0, 0, 255], opacity: 1.0 });
    assert!(!off.is_default() && !off.any_on());
    for &id in &ids {
        doc.set_stamp_styles(id, off.clone(), Commit::Final).unwrap();
        assert_eq!(format!("{:?}", doc.bake_placement(id).unwrap()), format!("{:?}", doc.stamp(id).unwrap().placement));
    }
    assert_eq!(cache_hash(&mut doc), P0_IDENTITY_HASH_512, "styles that are all off bake exactly as none");
    for &id in &ids[..3] {
        doc.promote_stamp(id, ShareScope::ThisStamp).unwrap();
        doc.rebake_stamp(id, BakeQuality::Full).unwrap();
    }
    assert_eq!(cache_hash(&mut doc), P0_IDENTITY_HASH_512, "promoted stamps with styles off bake exactly as before");
}

/// Stickers P6a: the Move tool's hit test finds the top visible layer that draws a pixel at the point (its mask
/// applied), and the Layers tab's thumbnail draws one layer alone, scaled to the side asked for.
#[test]
fn k6a_layer_at_and_layer_thumbnail() {
    let mut doc = doc_from_raw(samples::two_material_model(), 64);
    let mut sticker = StickerDoc::new("hit", 40, 20);
    let bottom = sticker.alloc_layer();
    let red = owned(&mut doc, disc(20, 10.0, [255, 0, 0, 255]));
    sticker.layers.push(StickerLayer::new(bottom, "red", StickerContent::Picture(red)));
    let top = sticker.alloc_layer();
    let blue = owned(&mut doc, disc(20, 10.0, [0, 0, 255, 255]));
    let mut layer = StickerLayer::new(top, "blue", StickerContent::Picture(blue));
    layer.transform.translate = Vec2::new(20.0, 0.0);
    sticker.layers.push(layer);
    let id = doc.add_sticker(sticker).unwrap();
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(10.5, 10.5)), Some(bottom));
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(30.5, 10.5)), Some(top));
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(0.5, 0.5)), None, "a corner outside both discs");
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(-5.0, 10.0)), None);
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(f32::NAN, 10.0)), None);
    // Hidden layers are skipped; a mask that hides a pixel hides it from the click too.
    doc.set_sticker(StickerRef::Id(id), |d| d.layers[1].visible = false, "Hide", Commit::Final).unwrap();
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(30.5, 10.5)), None);
    let mut mask = AlphaBuf::new(20, 20);
    mask.data.fill(255);
    mask.data[10 * 20 + 10] = 0;
    let version = doc.next_pixel_version();
    doc.set_sticker(StickerRef::Id(id), |d| d.layers[0].mask = Some(LayerMask { pixels: Arc::new(mask), version }), "Mask", Commit::Final)
        .unwrap();
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(10.5, 10.5)), None, "masked pixel");
    assert_eq!(doc.sticker_layer_at(id, Vec2::new(9.5, 10.5)), Some(bottom));
    // The thumbnail: the whole 40 x 20 artboard at side 20 (scale 1/2), the hidden blue layer drawn on its own.
    let thumb = doc.sticker_layer_thumbnail(id, top, 20).unwrap();
    assert_eq!((thumb.width, thumb.height), (20, 10));
    let px = |b: &PixelBuf, x: u32, y: u32| b.data[((y * b.width + x) * 4) as usize..][..4].to_vec();
    assert_eq!(px(&thumb, 15, 5)[2], 255, "the blue disc in the right half: {:?}", px(&thumb, 15, 5));
    assert_eq!(px(&thumb, 5, 5)[3], 0, "nothing of the red layer");
    assert!(doc.sticker_layer_thumbnail(id, StickerLayerId(99), 20).is_none());
    assert!(doc.sticker_layer_thumbnail(StickerId(9_999), top, 20).is_none());
    assert_eq!(doc.history.undo_len(), 3, "read-outs make no entry");
}
