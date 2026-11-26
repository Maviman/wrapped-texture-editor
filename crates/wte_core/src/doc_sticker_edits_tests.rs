// Tests of package P3b (STICKER_PLAN 7 "P3b", names `k3b_*`): pixel strokes on sticker pictures and masks, generic
// tile patches, `Edit::StickerPixels`, promotion inside an edit, limits, `convert_text_stamp` and Revert.

use std::sync::Arc;

use glam::Vec2;

use crate::brush::{BrushMode, BrushParams};
use crate::doc_sticker_edits::{PixelData, PixelTarget, REVERT_LABEL, StickerBrush, StickerPatch};
use crate::doc_stickers::{ShareScope, StickerRef};
use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::history::{Edit, MaskPatch, TileData, TilePatch};
use crate::ids::{BlobId, StampId, StickerId, StickerLayerId, TextureSetId};
use crate::image_io::{PngLevel, encode_png};
use crate::layer::LayerKind;
use crate::math::{IRect, fnv1a64};
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::samples;
use crate::stamp::{Placement, StampSource, UvPlacement};
use crate::sticker::style::{Contour, Stroke, StrokePos, StyleStack};
use crate::sticker::{LayerMask, PicturePixels, StickerContent, StickerDoc, StickerLayer, counters};
use crate::testutil::{XorShift, doc_from_raw};
use crate::text::{TEXT_PLACEMENT_PX, TextAlign, TextFx, TextSpec};

/// The bytes of a texture stroke's undo patch, recorded with the P0 code (before any P3b change).
const P0_TEXTURE_PATCH_HASH: u64 = 0xb75b_1fe0_532c_cc75;

fn noise(w: u32, h: u32, seed: u64) -> PixelBuf {
    let mut rng = XorShift::new(seed);
    let data = (0..w * h * 4).map(|_| rng.next_u32() as u8).collect();
    PixelBuf::from_raw(w, h, data).unwrap()
}

/// Opaque noise.
fn opaque(w: u32, h: u32, seed: u64) -> PixelBuf {
    let mut p = noise(w, h, seed);
    p.data.as_chunks_mut::<4>().0.iter_mut().for_each(|px| px[3] = 255);
    p
}

fn png(image: &PixelBuf) -> Arc<[u8]> {
    Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
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

/// The linked cache of stamp `id` in set 0 as a dense picture.
fn cache(doc: &Document, id: StampId) -> PixelBuf {
    let s = doc.set(TextureSetId(0)).unwrap();
    s.layers
        .iter()
        .find(|l| l.stamp_id() == Some(id))
        .map_or_else(|| PixelBuf::new(s.width, s.height), |l| l.to_dense(s.width, s.height))
}

/// A document with a UV sticker stamp whose sticker has one owned 160 x 96 picture layer (opacity 0.9, so it renders).
fn owned_doc() -> (Document, StickerId, StampId, StickerLayerId) {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let picture = PicturePixels::Owned { pixels: Arc::new(opaque(160, 96, 3)), version: doc.next_pixel_version() };
    let mut sticker = StickerDoc::from_picture("s", picture, [160, 96]);
    sticker.layers[0].opacity = 0.9;
    let lid = sticker.layers[0].id;
    let sid = doc.add_sticker(sticker).unwrap();
    let id = doc.add_stamp("s", StampSource::Sticker { sticker: sid }, uv([128.0, 128.0], [160.0, 96.0]), TextureSetId(0)).unwrap();
    doc.history.clear();
    (doc, sid, id, lid)
}

/// A document with two UV image stamps of one 64 x 64 image.
fn image_doc() -> (Document, BlobId, StampId, StampId) {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let blob = doc.add_image_blob("i.png", png(&samples::sample_stamp_image(64))).unwrap();
    let a = doc.add_stamp("a", StampSource::Image { blob }, uv([70.0, 70.0], [64.0, 64.0]), TextureSetId(0)).unwrap();
    let b = doc.add_stamp("b", StampSource::Image { blob }, uv([180.0, 180.0], [64.0, 64.0]), TextureSetId(0)).unwrap();
    doc.history.clear();
    (doc, blob, a, b)
}

fn picture(doc: &Document, sid: StickerId) -> (Arc<PixelBuf>, u64) {
    match &doc.sticker(sid).unwrap().layers[0].content {
        StickerContent::Picture(PicturePixels::Owned { pixels, version }) => (Arc::clone(pixels), *version),
        other => panic!("not an owned picture: {other:?}"),
    }
}

fn mask(doc: &Document, sid: StickerId) -> (Arc<AlphaBuf>, u64) {
    let m = doc.sticker(sid).unwrap().layers[0].mask.as_ref().expect("a mask");
    (Arc::clone(&m.pixels), m.version)
}

fn red(radius: f32) -> StickerBrush {
    StickerBrush { radius, hardness: 1.0, color: [255, 0, 0, 255], ..StickerBrush::default() }
}

fn line(doc: &mut Document, from: Vec2, to: Vec2, samples: u32) {
    for i in 0..=samples {
        doc.sticker_stroke_dab(from + (to - from) * (i as f32 / samples as f32), 1.0).unwrap();
    }
}

fn is_image(doc: &Document, id: StampId) -> bool {
    matches!(doc.stamp(id).unwrap().source, StampSource::Image { .. })
}

fn mean_diff_in(a: &PixelBuf, b: &PixelBuf, r: IRect) -> f64 {
    let mut sum = 0u64;
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let (p, q) = (a.get(x as u32, y as u32), b.get(x as u32, y as u32));
            sum += p.iter().zip(q).map(|(u, v)| u.abs_diff(v) as u64).sum::<u64>();
        }
    }
    sum as f64 / (r.area() as f64 * 4.0)
}

fn ink(buf: &PixelBuf) -> IRect {
    let mut r = IRect::EMPTY;
    for y in 0..buf.height {
        for x in 0..buf.width {
            if buf.get(x, y)[3] > 0 {
                r = r.union(&IRect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1));
            }
        }
    }
    r
}

// ----- 1: strokes -----

#[test]
fn k3b_1_a_stroke_is_one_entry_and_undo_redo_restore_the_bytes_with_new_versions() {
    let (mut doc, sid, stamp, lid) = owned_doc();
    let (p0, v0) = picture(&doc, sid);
    let (bytes0, cache0) = (p0.data.clone(), cache(&doc, stamp));
    drop(p0);
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(5.0)).unwrap();
    assert!(doc.sticker_stroke_active());
    assert_eq!(doc.undo(), None, "no undo while a sticker stroke is open");
    assert!(matches!(doc.stroke_begin(&BrushParams::default()), Err(EditError::StrokeInProgress)), "the strokes exclude each other");
    line(&mut doc, Vec2::new(20.0, 40.0), Vec2::new(120.0, 40.0), 10);
    doc.sticker_stroke_end().unwrap();
    assert_eq!(doc.history.undo_len(), 1, "one entry");
    assert_eq!(doc.undo_label(), Some("Sticker Brush"));
    let (p1, v1) = picture(&doc, sid);
    assert_ne!(v1, v0);
    assert_eq!(p1.get(70, 40), [255, 0, 0, 255], "painted");
    assert_eq!(p1.get(70, 80), opaque(160, 96, 3).get(70, 80), "untouched away from the stroke");
    let (bytes1, cache1) = (p1.data.clone(), cache(&doc, stamp));
    assert_ne!(cache1, cache0, "the user is re-baked");
    drop(p1);
    doc.undo().unwrap();
    let (p2, v2) = picture(&doc, sid);
    assert_eq!(p2.data, bytes0, "undo restores the bytes");
    assert!(v2 != v0 && v2 != v1, "undo gives a new version");
    assert_eq!(cache(&doc, stamp), cache0, "and the user's picture");
    drop(p2);
    doc.redo().unwrap();
    let (p3, v3) = picture(&doc, sid);
    assert_eq!(p3.data, bytes1);
    assert!(v3 > v2, "redo gives a new version too");
    assert_eq!(cache(&doc, stamp), cache1);
    // A pen: lower pressure, lighter paint (flow follows pressure by default).
    let soft = StickerBrush { flow: 1.0, hardness: 1.0, color: [0, 0, 255, 255], radius: 4.0, ..StickerBrush::default() };
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, soft).unwrap();
    doc.sticker_stroke_dab(Vec2::new(30.0, 80.0), 0.3).unwrap();
    doc.sticker_stroke_end().unwrap();
    // The centre of a hard dab of strength 0.3 (the 2D kernel of 6.4 on the pre-stroke pixel).
    let expected = crate::brush::apply_brush(opaque(160, 96, 3).get(30, 80), [0, 0, 255, 255], 19661.0 / 65535.0, BrushMode::Paint, false);
    assert_eq!(picture(&doc, sid).0.get(30, 80), expected, "flow follows the pressure");
}

#[test]
fn k3b_1_cancel_restores_and_a_selection_limits_the_dabs() {
    let (mut doc, sid, stamp, lid) = owned_doc();
    let (p0, _) = picture(&doc, sid);
    let (bytes0, cache0) = (p0.data.clone(), cache(&doc, stamp));
    drop(p0);
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(8.0)).unwrap();
    line(&mut doc, Vec2::new(10.0, 10.0), Vec2::new(150.0, 90.0), 20);
    doc.rebake_sticker_users(sid, crate::stamp::BakeQuality::Preview);
    assert_ne!(cache(&doc, stamp), cache0, "the live preview shows the stroke");
    doc.sticker_stroke_cancel();
    assert!(!doc.sticker_stroke_active());
    assert_eq!(picture(&doc, sid).0.data, bytes0, "cancel restores");
    assert_eq!(cache(&doc, stamp), cache0, "and the user's picture");
    assert_eq!(doc.history.undo_len(), 0);
    // A selection: only the left half (x < 80) may change.
    let mut sel = AlphaBuf::new(160, 96);
    for y in 0..96 {
        for x in 0..80 {
            sel.set(x, y, 255);
        }
    }
    let brush = StickerBrush { selection: Some(Arc::new(sel)), ..red(10.0) };
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, brush).unwrap();
    line(&mut doc, Vec2::new(5.0, 48.0), Vec2::new(155.0, 48.0), 30);
    doc.sticker_stroke_end().unwrap();
    let p = picture(&doc, sid).0;
    let before = PixelBuf::from_raw(160, 96, bytes0).unwrap();
    for y in 0..96 {
        for x in 80..160 {
            assert_eq!(p.get(x, y), before.get(x, y), "({x}, {y}) is outside the selection");
        }
    }
    assert_eq!(p.get(40, 48), [255, 0, 0, 255], "inside it is painted");
    // A selection of another size is refused.
    let bad = StickerBrush { selection: Some(Arc::new(AlphaBuf::new(10, 10))), ..red(3.0) };
    assert!(matches!(
        doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, bad),
        Err(EditError::InvalidParam(_))
    ));
    assert!(!doc.sticker_stroke_active());
}

#[test]
fn k3b_1_a_stroke_on_a_blob_picture_makes_it_owned_and_leaves_the_blob_alone() {
    let (mut doc, blob, a, b) = image_doc();
    let blob_bytes = doc.blobs.get(blob).unwrap().bytes.clone();
    let decoded = doc.blob_picture(blob).unwrap();
    let decoded_bytes = decoded.data.clone();
    let cache_b = cache(&doc, b);
    let sid = doc.promote_stamp(a, ShareScope::ThisStamp).unwrap();
    let lid = doc.sticker(sid).unwrap().layers[0].id;
    assert!(matches!(doc.sticker(sid).unwrap().layers[0].content, StickerContent::Picture(PicturePixels::Blob(_))));
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(4.0)).unwrap();
    line(&mut doc, Vec2::new(10.0, 32.0), Vec2::new(54.0, 32.0), 5);
    doc.sticker_stroke_end().unwrap();
    let (p, _) = picture(&doc, sid);
    assert_eq!(p.get(32, 32), [255, 0, 0, 255]);
    assert_eq!(doc.blobs.get(blob).unwrap().bytes, blob_bytes, "the blob is unchanged");
    assert_eq!(doc.blob_picture(blob).unwrap().data, decoded_bytes, "its decoded pixels too");
    assert!(is_image(&doc, b), "the other user of the blob stays an image stamp");
    assert_eq!(cache(&doc, b), cache_b, "and keeps its picture");
    // One undo takes the stroke and the change to an owned picture back.
    let entries = doc.history.undo_len();
    doc.undo().unwrap();
    assert_eq!(doc.history.undo_len(), entries - 1);
    assert!(matches!(doc.sticker(sid).unwrap().layers[0].content, StickerContent::Picture(PicturePixels::Blob(_))));
}

// ----- 2: masks -----

#[test]
fn k3b_2_a_mask_stroke_is_a_one_byte_patch_and_a_mask_patch_does_not_fit_a_picture() {
    let (mut doc, sid, stamp, lid) = owned_doc();
    let version = doc.next_pixel_version();
    doc.set_sticker(
        StickerRef::Id(sid),
        |d| d.layers[0].mask = Some(LayerMask { pixels: Arc::new(AlphaBuf::filled(160, 96, 255)), version }),
        "Add Mask",
        Commit::Final,
    )
    .unwrap();
    let (m0, _) = mask(&doc, sid);
    let (mask0, cache0) = (m0.data.clone(), cache(&doc, stamp));
    drop(m0);
    let hide = StickerBrush { color: [0, 0, 0, 255], ..red(6.0) };
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Mask, hide).unwrap();
    line(&mut doc, Vec2::new(10.0, 20.0), Vec2::new(150.0, 20.0), 14);
    doc.sticker_stroke_end().unwrap();
    let Edit::StickerPixels { patch: StickerPatch::Mask(patch), sticker, layer } = &doc.history.undo.last().unwrap().edit else {
        panic!("a mask stroke is one StickerPixels entry with a mask patch")
    };
    assert_eq!((*sticker, *layer), (sid, lid));
    assert_eq!(patch.bpp(), 1);
    let tiles = patch.tiles.len();
    assert!(tiles >= 3, "the stroke crosses three tiles");
    for t in &patch.tiles {
        let r = IRect::new(t.tx as i32 * 64, t.ty as i32 * 64, ((t.tx as i32 + 1) * 64).min(160), ((t.ty as i32 + 1) * 64).min(96));
        match &t.data {
            TileData::Raw(d) => assert_eq!(d.len(), r.area() as usize, "one byte per pixel"),
            TileData::Solid(v) => assert_eq!(*v, [255, 0, 0, 0], "a solid mask tile keeps one byte"),
        }
    }
    let (m1, v1) = mask(&doc, sid);
    assert_eq!(m1.get(70, 20), 0, "hidden");
    assert_eq!(m1.get(70, 80), 255);
    let mask1 = m1.data.clone();
    drop(m1);
    assert_ne!(cache(&doc, stamp), cache0);
    doc.undo().unwrap();
    let (m2, v2) = mask(&doc, sid);
    assert_eq!(m2.data, mask0);
    assert_ne!(v2, v1);
    assert_eq!(cache(&doc, stamp), cache0);
    drop(m2);
    doc.redo().unwrap();
    assert_eq!(mask(&doc, sid).0.data, mask1);
    // Kinds: a mask patch is refused on a picture buffer and a picture patch on a mask.
    let mut a = AlphaBuf::filled(70, 70, 9);
    let b = AlphaBuf::filled(70, 70, 200);
    let mut mp = MaskPatch::diff(&a, &b);
    assert_eq!(mp.tiles.len(), 4);
    assert!(!mp.fits::<PixelBuf>() && mp.fits::<AlphaBuf>());
    let mut rgba = PixelBuf::filled(70, 70, [1, 2, 3, 4]);
    assert_eq!(mp.swap_with(&mut rgba), IRect::EMPTY);
    assert_eq!(rgba, PixelBuf::filled(70, 70, [1, 2, 3, 4]), "refused: nothing changed");
    let mut tp = TilePatch::diff(&PixelBuf::filled(70, 70, [0; 4]), &rgba);
    assert_eq!(tp.swap_with(&mut a), IRect::EMPTY);
    assert_eq!(a, AlphaBuf::filled(70, 70, 9));
    // A mask patch on a mask swaps.
    assert_eq!(mp.swap_with(&mut a), IRect::from_size(70, 70));
    assert_eq!(a, AlphaBuf::filled(70, 70, 9), "b differed from a everywhere: the patch held a's tiles");
}

/// k3b-2 (review A4): a texture stroke's patch has the same bytes as on the P0 code.
#[test]
fn k3b_2_texture_stroke_patch_bytes_are_unchanged() {
    let mut doc = doc_from_raw(samples::flat_quad(), 200);
    let layer = doc.add_raster_layer(TextureSetId(0), None).unwrap();
    if let Some(LayerKind::Raster(buf)) = doc.layer_mut(layer).map(|l| &mut l.kind) {
        *buf = Arc::new(noise(200, 200, 7));
    }
    doc.history.clear();
    let b = BrushParams { radius: 9.0, hardness: 0.6, opacity: 0.8, flow: 0.5, color: [10, 200, 30, 255], ..BrushParams::default() };
    doc.stroke_begin(&b).unwrap();
    for i in 0..40 {
        doc.stroke_to_2d(TextureSetId(0), Vec2::new(20.0 + i as f32 * 4.0, 30.0 + i as f32 * 3.5), 0.4 + i as f32 * 0.015);
    }
    assert!(doc.stroke_end());
    let entry = doc.history.undo.last().unwrap();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let Edit::Multi(edits) = &entry.edit else { panic!("a stroke is a Multi entry") };
    let mut tiles = 0;
    for e in edits {
        let Edit::Pixels { patch, .. } = e else { panic!("pixel edits only") };
        for t in &patch.tiles {
            tiles += 1;
            h = fnv1a64(h, &[t.tx.to_le_bytes(), t.ty.to_le_bytes()].concat());
            match &t.data {
                TileData::Solid(c) => h = fnv1a64(h, c),
                TileData::Raw(d) => h = fnv1a64(h, d),
            }
        }
    }
    assert!(tiles > 4);
    assert_eq!(h, P0_TEXTURE_PATCH_HASH, "texture patch hash {h:#018x}");
}

// ----- 3: whole-buffer results -----

#[test]
fn k3b_3_replace_same_size_is_a_patch_of_changed_tiles_another_size_a_structure_entry() {
    let (mut doc, sid, stamp, lid) = owned_doc();
    let (p0, _) = picture(&doc, sid);
    let mut new = (*p0).clone();
    for y in 30..40 {
        for x in 70..80 {
            new.set(x, y, [0, 255, 0, 255]);
        }
    }
    let cache0 = cache(&doc, stamp);
    assert_eq!(doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Pixels, PixelData::Rgba((*p0).clone()), "Fill").unwrap(), sid);
    assert_eq!(doc.history.undo_len(), 0, "the same pixels: no entry");
    doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Pixels, PixelData::Rgba(new.clone()), "Fill").unwrap();
    let Edit::StickerPixels { patch: StickerPatch::Pixels(patch), .. } = &doc.history.undo.last().unwrap().edit else {
        panic!("a tile patch")
    };
    assert_eq!(patch.tiles.iter().map(|t| (t.tx, t.ty)).collect::<Vec<_>>(), vec![(1, 0)], "only the changed tile");
    assert_eq!(doc.undo_label(), Some("Fill"));
    assert_eq!(picture(&doc, sid).0.data, new.data);
    assert_ne!(cache(&doc, stamp), cache0);
    doc.undo().unwrap();
    assert_eq!(picture(&doc, sid).0.data, p0.data);
    assert_eq!(cache(&doc, stamp), cache0);
    doc.redo().unwrap();
    assert_eq!(picture(&doc, sid).0.data, new.data);
    // Another size: a structure entry; undo brings the same buffer back.
    let (before, _) = picture(&doc, sid);
    doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Pixels, PixelData::Rgba(opaque(100, 50, 9)), "Crop").unwrap();
    assert!(matches!(doc.history.undo.last().unwrap().edit, Edit::Structure(_)));
    assert_eq!(picture(&doc, sid).0.width, 100);
    doc.undo().unwrap();
    assert!(Arc::ptr_eq(&before, &picture(&doc, sid).0));
    // Kind mismatches and empty data are refused.
    assert!(matches!(
        doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Mask, PixelData::Rgba(new.clone()), "x"),
        Err(EditError::InvalidParam(_))
    ));
    assert!(matches!(
        doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Mask, PixelData::Alpha(AlphaBuf::new(160, 96)), "x"),
        Err(EditError::InvalidParam(_))
    ), "no mask");
    assert!(matches!(
        doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Pixels, PixelData::Rgba(PixelBuf::new(0, 0)), "x"),
        Err(EditError::InvalidParam(_))
    ));
}

// ----- 4: promotion inside an edit -----

#[test]
fn k3b_4a_a_preview_drag_on_an_image_stamp_is_one_entry_with_its_promotion() {
    let (mut doc, blob, a, _) = image_doc();
    let cache0 = cache(&doc, a);
    let on = StickerRef::OfStamp(a, ShareScope::ThisStamp);
    for o in [0.9, 0.8] {
        doc.set_sticker(on, |d| d.layers[0].opacity = o, "Layer Opacity", Commit::Preview).unwrap();
    }
    assert!(!is_image(&doc, a));
    let sid = doc.set_sticker(on, |d| d.layers[0].opacity = 0.7, "Layer Opacity", Commit::Final).unwrap();
    assert_eq!(doc.history.undo_len(), 1, "one entry");
    assert_eq!(doc.sticker(sid).unwrap().layers[0].opacity, 0.7);
    assert_ne!(cache(&doc, a), cache0);
    doc.undo().unwrap();
    assert_eq!(doc.stamp(a).unwrap().source, StampSource::Image { blob });
    assert!(doc.stickers().all(|(id, _)| doc.sticker_users(id).is_empty()), "no sticker left in use");
    assert_eq!(cache(&doc, a), cache0);
    doc.redo().unwrap();
    assert_eq!(doc.stamp(a).unwrap().source, StampSource::Sticker { sticker: sid });
}

#[test]
fn k3b_4b_cancel_after_the_first_preview_brings_the_image_stamp_back() {
    let (mut doc, blob, a, _) = image_doc();
    let cache0 = cache(&doc, a);
    doc.set_sticker(StickerRef::OfStamp(a, ShareScope::ThisStamp), |d| d.layers[0].opacity = 0.5, "Layer Opacity", Commit::Preview)
        .unwrap();
    assert!(!is_image(&doc, a));
    assert!(doc.cancel_pending_edit());
    assert_eq!(doc.stamp(a).unwrap().source, StampSource::Image { blob });
    assert_eq!(doc.stickers().count(), 0);
    assert_eq!(cache(&doc, a), cache0);
    assert_eq!(doc.history.undo_len(), 0);
    // An edit that is refused (a limit) takes the promotion back too.
    let r = doc.set_sticker(
        StickerRef::OfStamp(a, ShareScope::ThisStamp),
        |d| d.artboard = IRect::from_size(5000, 10),
        "Canvas Size",
        Commit::Final,
    );
    assert!(matches!(r, Err(EditError::StickerLimit(_))));
    assert!(is_image(&doc, a) && doc.stickers().count() == 0 && doc.history.undo_len() == 0);
    // A text stamp is not promoted.
    let spec = text(&mut doc, "T");
    let t = doc.add_stamp("t", StampSource::Text(spec), uv([60.0, 200.0], [60.0, 30.0]), TextureSetId(0)).unwrap();
    assert!(matches!(
        doc.set_sticker(StickerRef::OfStamp(t, ShareScope::ThisStamp), |d| d.name = "x".into(), "Rename", Commit::Final),
        Err(EditError::InvalidParam(_))
    ));
}

#[test]
fn k3b_4c_a_stroke_on_an_image_stamp_is_one_entry_and_4d_this_stamp_only() {
    let (mut doc, blob, a, b) = image_doc();
    let cache_a = cache(&doc, a);
    let undo = doc.history.undo_len();
    let sid = doc
        .sticker_stroke_begin(StickerRef::OfStamp(a, ShareScope::ThisStamp), StickerLayerId(1), PixelTarget::Pixels, red(5.0))
        .unwrap();
    line(&mut doc, Vec2::new(5.0, 30.0), Vec2::new(60.0, 30.0), 6);
    doc.sticker_stroke_end().unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1, "one entry");
    let Edit::Multi(parts) = &doc.history.undo.last().unwrap().edit else { panic!("promotion and pixels in one Multi") };
    assert!(matches!(parts[..], [Edit::Structure(_), Edit::StickerPixels { .. }]));
    let painted = picture(&doc, sid).0.data.clone();
    let cache_painted = cache(&doc, a);
    assert_ne!(cache_painted, cache_a);
    assert!(is_image(&doc, b), "4d: ThisStamp leaves the other stamp of the blob an image stamp");
    doc.undo().unwrap();
    assert_eq!(doc.stamp(a).unwrap().source, StampSource::Image { blob });
    assert!(doc.sticker(sid).is_none());
    assert_eq!(cache(&doc, a), cache_a);
    doc.redo().unwrap();
    assert_eq!(doc.stamp(a).unwrap().source, StampSource::Sticker { sticker: sid });
    assert_eq!(picture(&doc, sid).0.data, painted);
    assert_eq!(cache(&doc, a), cache_painted);
    // A stroke that paints nothing takes the promotion back (no entry).
    let undo = doc.history.undo_len();
    let none = StickerBrush { flow: 0.0, ..red(5.0) };
    doc.sticker_stroke_begin(StickerRef::OfStamp(b, ShareScope::ThisStamp), StickerLayerId(1), PixelTarget::Pixels, none).unwrap();
    assert!(!is_image(&doc, b));
    doc.sticker_stroke_dab(Vec2::new(30.0, 30.0), 1.0).unwrap();
    doc.sticker_stroke_end().unwrap();
    assert!(is_image(&doc, b));
    assert_eq!(doc.history.undo_len(), undo);
    // AllUsers: both stamps of a blob share the promoted sticker.
    let (mut doc, _, a, b) = image_doc();
    let sid = doc
        .replace_sticker_pixels(
            StickerRef::OfStamp(a, ShareScope::AllUsers),
            StickerLayerId(1),
            PixelTarget::Pixels,
            PixelData::Rgba(opaque(64, 64, 4)),
            "Remove Background",
        )
        .unwrap();
    assert_eq!(doc.sticker_users(sid), vec![a, b]);
    assert_eq!(doc.history.undo_len(), 1);
    doc.undo().unwrap();
    assert!(is_image(&doc, a) && is_image(&doc, b));
}

// ----- 5: the one copy per stroke -----

#[test]
fn k3b_5_the_first_dab_after_a_snapshot_copies_the_buffer_once_in_begin() {
    let (mut doc, sid, _, lid) = owned_doc();
    // A structure entry: its snapshot shares the sticker and its picture.
    doc.set_sticker(StickerRef::Id(sid), |d| d.name = "renamed".into(), "Rename Sticker", Commit::Final).unwrap();
    let bytes0 = picture(&doc, sid).0.data.clone();
    let c0 = counters::buffer_copies();
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(6.0)).unwrap();
    assert_eq!(counters::buffer_copies(), c0 + 1, "begin copies the shared buffer once");
    line(&mut doc, Vec2::new(10.0, 10.0), Vec2::new(150.0, 90.0), 40);
    doc.rebake_sticker_users(sid, crate::stamp::BakeQuality::Preview);
    line(&mut doc, Vec2::new(150.0, 10.0), Vec2::new(10.0, 90.0), 40);
    assert_eq!(counters::buffer_copies(), c0 + 1, "no dab copies it again (also after a live preview)");
    doc.sticker_stroke_end().unwrap();
    assert_eq!(counters::buffer_copies(), c0 + 1);
    let Edit::Structure(snapshot) = &doc.history.undo[0].edit else { panic!("the rename's snapshot") };
    let held = snapshot.stickers[&sid].layers[0].owned_picture().unwrap();
    assert_eq!(held.data, bytes0, "the snapshot's Arc still holds the old bytes");
    // The next stroke starts on an unshared buffer: no copy.
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(3.0)).unwrap();
    doc.sticker_stroke_dab(Vec2::new(5.0, 5.0), 1.0).unwrap();
    doc.sticker_stroke_end().unwrap();
    assert_eq!(counters::buffer_copies(), c0 + 1, "nothing shares it now");
}

// ----- 6: limits -----

#[test]
fn k3b_6_limits_are_refused_with_their_messages_and_change_nothing() {
    let (mut doc, sid, _, lid) = owned_doc();
    let before = Arc::clone(doc.sticker(sid).unwrap());
    let undo = doc.history.undo_len();
    let message = |r: Result<StickerId, EditError>| match r {
        Err(EditError::StickerLimit(m)) => m,
        other => panic!("not refused by a limit: {other:?}"),
    };
    let m = message(doc.set_sticker(
        StickerRef::Id(sid),
        |d| {
            while d.layers.len() < 33 {
                let id = d.alloc_layer();
                d.layers.push(StickerLayer::new(id, "l", StickerContent::Picture(PicturePixels::Blob(BlobId(0)))));
            }
        },
        "Add Layer",
        Commit::Final,
    ));
    assert!(m.contains("32 layers"), "{m}");
    let m = message(doc.set_sticker(StickerRef::Id(sid), |d| d.artboard = IRect::from_size(4097, 96), "Canvas Size", Commit::Final));
    assert!(m.contains("4096"), "{m}");
    let big = |doc: &mut Document| PicturePixels::Owned { pixels: Arc::new(PixelBuf::new(8192, 8192)), version: doc.next_pixel_version() };
    let (p1, p2) = (big(&mut doc), big(&mut doc));
    let m = message(doc.set_sticker(
        StickerRef::Id(sid),
        |d| {
            for p in [p1, p2] {
                let id = d.alloc_layer();
                d.layers.push(StickerLayer::new(id, "big", StickerContent::Picture(p)));
            }
        },
        "Add Layer",
        Commit::Final,
    ));
    assert!(m.contains("512 MB"), "{m}");
    let m = message(doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Pixels, PixelData::Rgba(PixelBuf::new(4097, 8)), "Resize"));
    assert!(m.contains("4096"), "{m}");
    assert!(Arc::ptr_eq(&before, doc.sticker(sid).unwrap()), "nothing changed");
    assert_eq!(doc.history.undo_len(), undo);
    assert!(!doc.has_pending_edit());
}

// ----- 7: text conversion -----

#[test]
fn k3b_7_convert_text_stamp_keeps_the_frame_and_the_picture_at_32_128_512() {
    let mut doc = doc_from_raw(samples::flat_quad(), 1024);
    for em in [32.0f32, 128.0, 512.0] {
        let spec = text(&mut doc, "Hi");
        let frame = doc.stamp_source_frame(&StampSource::Text(spec.clone())).unwrap();
        let k = em / frame.em_px;
        let p = uv([512.0, 512.0], [frame.size[0] as f32 * k, frame.size[1] as f32 * k]);
        let id = doc.add_stamp("Hi", StampSource::Text(spec), p, TextureSetId(0)).unwrap();
        let StampSource::Text(drawn) = doc.stamp(id).unwrap().source.clone() else { panic!() };
        assert_eq!(drawn.size_px, em, "the raster size follows the em on the model");
        let stroke = StyleStack {
            stroke: Some(Stroke { on: true, width: 6.0, position: StrokePos::Outside, color: [0, 0, 0, 255], opacity: 1.0 }),
            ..StyleStack::default()
        };
        doc.set_stamp_styles(id, stroke, Commit::Final).unwrap();
        let text_frame = doc.stamp_text_frame(id).unwrap();
        let (placement, before) = (doc.stamp(id).unwrap().placement, cache(&doc, id));
        let undo = doc.history.undo_len();
        let sid = doc.convert_text_stamp(id).unwrap();
        assert_eq!(doc.history.undo_len(), undo + 1);
        assert_eq!(doc.undo_label(), Some("Convert to Sticker"));
        let object = doc.stamp(id).unwrap().clone();
        assert_eq!(object.source, StampSource::Sticker { sticker: sid });
        assert_eq!(object.placement, placement, "em {em}: the placement is unchanged");
        let sticker = doc.sticker(sid).unwrap();
        let [w, h] = sticker.size();
        assert!(w.abs_diff(text_frame.size[0]) <= 1 && h.abs_diff(text_frame.size[1]) <= 1, "em {em}: the frame {w} x {h}");
        let StickerContent::Text(layer_spec) = &sticker.layers[0].content else { panic!("a text layer") };
        assert_eq!(layer_spec.size_px, em);
        let width = object.styles.stroke.as_ref().unwrap().width;
        assert!((width - 6.0 * em / 128.0).abs() < 1e-4, "em {em}: stroke {width}");
        assert!(doc.sets[0].layers.iter().any(|l| matches!(l.kind, LayerKind::Stamp { stamp, .. } if stamp == id)), "a picture layer now");
        let after = cache(&doc, id);
        let r = ink(&before).union(&ink(&after));
        let diff = mean_diff_in(&before, &after, r);
        assert!(diff <= 2.0, "em {em}: mean difference {diff:.3} / 255 over {r:?}");
        // Undo brings the text stamp back exactly.
        doc.undo().unwrap();
        assert!(matches!(doc.stamp(id).unwrap().source, StampSource::Text(_)));
        assert!(doc.sets[0].layers.iter().any(|l| matches!(l.kind, LayerKind::Text { stamp, .. } if stamp == id)));
        assert_eq!(cache(&doc, id), before);
        doc.remove_stamp(id).unwrap();
    }
}

// ----- 8: Revert -----

#[test]
fn k3b_8_revert_after_five_mixed_edits_is_one_entry_and_undo_brings_them_back() {
    let (mut doc, sid, stamp, lid) = owned_doc();
    let start = (Arc::clone(doc.sticker(sid).unwrap()), doc.stamp(stamp).unwrap().clone(), cache(&doc, stamp));
    let cp = doc.sticker_checkpoint(&[stamp], &[]);
    assert!(cp.sticker(sid).is_some(), "the stamp's sticker is part of the checkpoint");
    // Five edits: structure, pixels (stroke), pixels (tool), stamp styles, crop (with a placement refit).
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].opacity = 0.5, "Layer Opacity", Commit::Final).unwrap();
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(7.0)).unwrap();
    line(&mut doc, Vec2::new(10.0, 50.0), Vec2::new(150.0, 50.0), 12);
    doc.sticker_stroke_end().unwrap();
    let mut green = (*picture(&doc, sid).0).clone();
    green.set(3, 3, [0, 255, 0, 255]);
    doc.replace_sticker_pixels(StickerRef::Id(sid), lid, PixelTarget::Pixels, PixelData::Rgba(green), "Fill").unwrap();
    let c = Contour { on: true, width: 6.0, smooth: 0.0, fill_holes: true, color: [255; 4], opacity: 1.0 };
    doc.set_stamp_styles(stamp, StyleStack { contour: Some(c), ..StyleStack::default() }, Commit::Final).unwrap();
    doc.set_sticker(StickerRef::Id(sid), |d| d.artboard = IRect::new(10, 5, 150, 90), "Crop", Commit::Final).unwrap();
    let edited = (Arc::clone(doc.sticker(sid).unwrap()), doc.stamp(stamp).unwrap().clone(), cache(&doc, stamp));
    assert_ne!(edited.1.placement, start.1.placement, "the crop refitted the placement");
    let undo = doc.history.undo_len();
    assert_eq!(undo, 5);
    doc.restore_sticker_state(&cp, REVERT_LABEL).unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1, "one entry");
    assert_eq!(doc.undo_label(), Some("Revert Sticker"));
    assert_eq!(**doc.sticker(sid).unwrap(), *start.0, "the sticker equals the checkpoint");
    assert_eq!(*doc.stamp(stamp).unwrap(), start.1);
    assert_eq!(cache(&doc, stamp), start.2, "and the picture on the model");
    // Nothing to revert: no entry.
    doc.restore_sticker_state(&cp, REVERT_LABEL).unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1);
    doc.undo().unwrap();
    assert_eq!(**doc.sticker(sid).unwrap(), *edited.0, "undo brings the five edits back");
    assert_eq!(*doc.stamp(stamp).unwrap(), edited.1);
    assert_eq!(cache(&doc, stamp), edited.2);
}

// ----- the history around sticker pixels -----

#[test]
fn k3b_history_counts_sticker_patches_and_commands_wait_for_the_stroke() {
    let (mut doc, sid, _, lid) = owned_doc();
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(4.0)).unwrap();
    assert!(matches!(
        doc.set_sticker(StickerRef::Id(sid), |d| d.name = "x".into(), "Rename", Commit::Final),
        Err(EditError::StrokeInProgress)
    ));
    assert!(doc.next_final_bake().is_none(), "no job holds the sticker during a stroke");
    assert!(matches!(
        doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(4.0)),
        Err(EditError::StrokeInProgress)
    ));
    doc.sticker_stroke_dab(Vec2::new(64.0, 32.0), 1.0).unwrap();
    doc.sticker_stroke_end().unwrap();
    assert!(matches!(doc.sticker_stroke_end(), Err(EditError::InvalidParam(_))));
    let Edit::StickerPixels { patch, .. } = &doc.history.undo.last().unwrap().edit else { panic!() };
    let bytes = patch.byte_len();
    assert!(bytes >= 64 * 64 * 4, "at least one raw tile");
    assert!(doc.history_bytes() >= bytes, "the patch is counted");
    // Deferred Final bakes: the stroke's end leaves the user unbaked; undo does not wait for a bake either.
    let users = doc.sticker_users(sid);
    doc.set_defer_final_bakes(true);
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(4.0)).unwrap();
    doc.sticker_stroke_dab(Vec2::new(20.0, 70.0), 1.0).unwrap();
    doc.sticker_stroke_end().unwrap();
    assert!(users.iter().all(|u| doc.stamp_is_unbaked(*u)));
    while let Some(job) = doc.next_final_bake() {
        doc.apply_final_bake(job.run());
    }
    doc.undo().unwrap();
    assert!(users.iter().all(|u| doc.stamp_is_unbaked(*u)), "undo marks the users unbaked instead of baking");
}

// ----- performance (8.4) -----

fn median_ms(mut f: impl FnMut() -> f64) -> f64 {
    f();
    let mut t: Vec<f64> = (0..5).map(|_| f()).collect();
    t.sort_by(f64::total_cmp);
    t[2]
}

fn report(what: &str, ms: f64, budget: f64) {
    let verdict = if ms <= budget {
        "ok"
    } else if ms <= budget * 1.5 {
        "MISSED by < 50 % (gate decides)"
    } else {
        "FAILED"
    };
    eprintln!("{what}: {ms:.2} ms (budget {budget} ms) {verdict}");
    assert!(ms <= budget * 1.5, "{what}: {ms:.2} ms is more than 1.5 x the budget of {budget} ms");
}

/// A sticker document of one owned `side x side` picture, its sticker shared with a history snapshot.
fn perf_doc(side: u32) -> (Document, StickerId, StickerLayerId) {
    let mut doc = doc_from_raw(samples::flat_quad(), 32);
    let picture = PicturePixels::Owned { pixels: Arc::new(opaque(side, side, 5)), version: doc.next_pixel_version() };
    let sid = doc.add_sticker(StickerDoc::from_picture("perf", picture, [side, side])).unwrap();
    (doc, sid, StickerLayerId(1))
}

#[test]
#[ignore = "perf: run serially in release (cargo test -p wte_core --release --lib perf_k3b -- --ignored --test-threads=1)"]
fn perf_k3b_stroke_begin_and_one_dab() {
    // `sticker_stroke_begin` on a 4096 x 4096 picture shared with a snapshot (the one copy): 40 ms.
    let (mut doc, sid, lid) = perf_doc(4096);
    let begin = median_ms(|| {
        doc.set_sticker(StickerRef::Id(sid), |d| d.name.push('x'), "Rename", Commit::Final).unwrap();
        let t = std::time::Instant::now();
        doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, red(32.0)).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        doc.sticker_stroke_cancel();
        ms
    });
    report("sticker_stroke_begin, 4096 x 4096 shared (one copy)", begin, 40.0);
    // One dab of size 64 on a 2048 x 2048 picture (the pixels; the canvas composite is P7a's): 2 ms.
    let (mut doc, sid, lid) = perf_doc(2048);
    let brush = StickerBrush { radius: 32.0, hardness: 0.5, flow: 0.5, ..red(32.0) };
    doc.sticker_stroke_begin(StickerRef::Id(sid), lid, PixelTarget::Pixels, StickerBrush { spacing: 1.0, ..brush }).unwrap();
    let mut x = 100.0f32;
    let dab = median_ms(|| {
        x += 70.0;
        let t = std::time::Instant::now();
        doc.sticker_stroke_dab(Vec2::new(x, 1000.0), 1.0).unwrap();
        doc.sticker_stroke_break();
        t.elapsed().as_secs_f64() * 1000.0
    });
    doc.sticker_stroke_end().unwrap();
    report("one dab, size 64, 2048 x 2048 picture", dab, 2.0);
}
