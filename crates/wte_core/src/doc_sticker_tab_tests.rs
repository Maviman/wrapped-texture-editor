// Tests of the core additions of package P6b (STICKER_PLAN 7 "P6b", names `k6b_*`): `add_stamp_styled` (the Stamp
// tool's styles for new stamps, 3.6), `set_styles_of` (one slider of the Sticker tab on every selected stamp, 3.7) and
// `stamp_style_reference_size` (the presets of the Sticker tab, 2.3).

use std::sync::Arc;

use glam::{Vec2, Vec3};

use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::ids::TextureSetId;
use crate::image_io::{PngLevel, encode_png};
use crate::layer::LayerKind;
use crate::math::fnv1a64;
use crate::pixel::PixelBuf;
use crate::samples;
use crate::stamp::{Placement, Projector, StampSource, UvPlacement};
use crate::sticker::style::{Shadow, StylePreset, StyleStack};
use crate::testutil::doc_from_raw;
use crate::text::{TEXT_PLACEMENT_PX, TextAlign, TextFx, TextSpec};

fn png(image: &PixelBuf) -> Arc<[u8]> {
    Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
}

fn front(cx: f32, cy: f32, z: f32, half: Vec2, depth: f32) -> Projector {
    Projector::from_surface(Vec3::new(cx, cy, z), Vec3::Z, Vec3::Y, half, depth)
}

fn uv(center: [f32; 2], size: [f32; 2]) -> UvPlacement {
    UvPlacement {
        set: TextureSetId(0),
        center_px: Vec2::from_array(center),
        size_px: Vec2::from_array(size),
        rotation: 0.0,
        flip_x: false,
        flip_y: false,
        clip_to_islands: false,
    }
}

fn shadow(distance: f32, size: f32) -> StyleStack {
    StyleStack {
        drop_shadow: Some(Shadow { on: true, color: [0, 0, 0, 255], opacity: 0.8, angle_deg: 120.0, distance, spread: 0.3, size }),
        ..StyleStack::default()
    }
}

/// One hash over every linked cache and every composite.
fn cache_hash(doc: &mut Document) -> u64 {
    doc.recomposite_all();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for set in &doc.sets {
        for layer in &set.layers {
            if let LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } = &layer.kind {
                h = fnv1a64(h, layer.name.as_bytes());
                h = fnv1a64(h, &cache.to_dense(set.width, set.height).data);
            }
        }
        h = fnv1a64(h, &set.composite.data);
    }
    h
}

#[test]
fn k6b_1_add_stamp_styled_with_default_styles_is_add_stamp() {
    let make = |styled: bool| {
        let mut doc = doc_from_raw(samples::flat_quad(), 128);
        let blob = doc.add_image_blob("s.png", png(&samples::sample_stamp_image(48))).unwrap();
        let p = Placement::Uv(uv([64.0, 64.0], [40.0, 40.0]));
        let source = StampSource::Image { blob };
        let id = if styled {
            doc.add_stamp_styled("s", source, p, TextureSetId(0), StyleStack::default()).unwrap()
        } else {
            doc.add_stamp("s", source, p, TextureSetId(0)).unwrap()
        };
        let object = doc.stamp(id).unwrap().clone();
        let label = doc.undo_label().map(str::to_owned);
        (cache_hash(&mut doc), object, label, doc.history.undo_len())
    };
    let (a, b) = (make(false), make(true));
    assert_eq!(a.0, b.0, "the same pixels");
    assert_eq!(a.1, b.1, "the same stamp object");
    assert_eq!((a.2, a.3), (b.2, b.3), "the same history");
    assert!(matches!(b.1.source, StampSource::Image { .. }) && b.1.styles.is_default());
}

#[test]
fn k6b_2_add_stamp_styled_draws_the_styles_in_one_entry_and_fits_the_depth_to_them() {
    let mut doc = doc_from_raw(samples::uv_sphere(32, 16, 1.0), 256);
    let blob = doc.add_image_blob("d.png", png(&samples::sample_stamp_image(64))).unwrap();
    let p = doc.fit_stamp_depth(&front(0.0, 0.0, 1.0, Vec2::splat(0.35), 0.1));
    let plain = doc.add_stamp("plain", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
    let undo = doc.history.undo_len();
    let styles = shadow(16.0, 16.0);
    let id = doc.add_stamp_styled("styled", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0), styles.clone()).unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1, "one entry");
    assert_eq!(doc.undo_label(), Some("Add Stamp"));
    let object = doc.stamp(id).unwrap().clone();
    assert_eq!(object.styles, styles.sanitized());
    let Placement::Projected(stored) = object.placement else { panic!() };
    assert_eq!((stored.origin, stored.half_size), (p.origin, p.half_size), "the stored placement is the artwork box");
    assert!(stored.depth_back > p.depth_back, "the automatic depth covers the shadow: {} -> {}", p.depth_back, stored.depth_back);
    let Some(Placement::Projected(bake)) = doc.bake_placement(id) else { panic!() };
    assert!(doc.stamp_depth_is_auto(&bake), "the stored depth is the automatic depth of the bake placement");
    assert!(doc.stamp_ink_area(id) > doc.stamp_ink_area(plain) * 1.2, "the shadow is baked");
    // One undo removes the stamp.
    doc.undo().unwrap();
    assert!(doc.stamp(id).is_none() && doc.stamp(plain).is_some());
}

#[test]
fn k6b_3_one_style_edit_on_several_stamps_is_one_entry() {
    let mut doc = doc_from_raw(samples::flat_quad(), 128);
    let blob = doc.add_image_blob("s.png", png(&samples::sample_stamp_image(32))).unwrap();
    let ids: Vec<_> = (0..3)
        .map(|i| {
            let p = Placement::Uv(uv([24.0 + 40.0 * i as f32, 64.0], [24.0, 24.0]));
            doc.add_stamp(&format!("s{i}"), StampSource::Image { blob }, p, TextureSetId(0)).unwrap()
        })
        .collect();
    let undo = doc.history.undo_len();
    let before = cache_hash(&mut doc);
    for size in [2.0, 4.0, 6.0] {
        doc.set_styles_of(&ids, &shadow(2.0, size), Commit::Preview).unwrap();
    }
    doc.set_styles_of(&ids, &shadow(2.0, 6.0), Commit::Final).unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1, "one entry for the drag");
    assert_eq!(doc.undo_label(), Some(crate::doc_stickers::STYLE_LABEL));
    for id in &ids {
        assert_eq!(doc.stamp(*id).unwrap().styles, shadow(2.0, 6.0).sanitized(), "every stamp got the styles");
        assert!(doc.stamp(*id).unwrap().source == StampSource::Image { blob }, "a style edit never promotes");
    }
    assert_ne!(cache_hash(&mut doc), before);
    doc.undo().unwrap();
    assert!(ids.iter().all(|id| doc.stamp(*id).unwrap().styles.is_default()), "one undo takes it off all of them");
    assert_eq!(cache_hash(&mut doc), before);
    // A duplicate id counts once; nothing to change is no entry.
    doc.set_styles_of(&[ids[0], ids[0]], &StyleStack::default(), Commit::Final).unwrap();
    assert_eq!(doc.history.undo_len(), undo);
    // A locked stamp refuses the whole edit before anything changes.
    let layer = doc.stamp_layers(ids[2])[0].1;
    doc.set_layer_locked(layer, true).unwrap();
    let undo = doc.history.undo_len();
    assert!(matches!(doc.set_styles_of(&ids, &shadow(1.0, 1.0), Commit::Final), Err(EditError::LayerLocked)));
    assert!(ids.iter().all(|id| doc.stamp(*id).unwrap().styles.is_default()));
    assert_eq!(doc.history.undo_len(), undo);
    assert!(matches!(doc.set_styles_of(&[crate::ids::StampId(9999)], &shadow(1.0, 1.0), Commit::Final), Err(EditError::UnknownStamp)));
}

#[test]
fn k6b_4_the_reference_size_of_each_source_kind() {
    let mut doc = doc_from_raw(samples::flat_quad(), 256);
    let blob = doc.add_image_blob("s.png", png(&samples::sample_stamp_image(96))).unwrap();
    let image = doc.add_stamp("img", StampSource::Image { blob }, Placement::Uv(uv([64.0, 64.0], [40.0, 40.0])), TextureSetId(0)).unwrap();
    assert_eq!(doc.stamp_style_reference_size(image).unwrap(), [96, 96], "image pixels");
    // A text: the picture at 128 px per em, whatever size it is drawn at.
    let font = doc.default_font();
    let spec = TextSpec {
        content: "Hi".to_owned(),
        font,
        size_px: TEXT_PLACEMENT_PX,
        color: [255; 4],
        align: TextAlign::Center,
        line_spacing: 1.0,
        letter_spacing: 0.0,
        outline: None,
        fx: TextFx::default(),
    };
    let small = doc.add_stamp("t1", StampSource::Text(spec.clone()), Placement::Uv(uv([64.0, 128.0], [40.0, 16.0])), TextureSetId(0)).unwrap();
    let large = doc.add_stamp("t2", StampSource::Text(spec), Placement::Uv(uv([128.0, 200.0], [200.0, 80.0])), TextureSetId(0)).unwrap();
    let (a, b) = (doc.stamp_style_reference_size(small).unwrap(), doc.stamp_style_reference_size(large).unwrap());
    assert!(a[1] > 100 && a[1] < 260, "about one line at 128 px per em: {a:?}");
    for i in 0..2 {
        assert!((a[i] as f32 - b[i] as f32).abs() <= 0.15 * a[i] as f32 + 2.0, "about the same at every size (hinting and padding differ a little): {a:?} {b:?}");
    }
    // A sticker: its artboard.
    let sticker = doc.promote_stamp(image, crate::doc_stickers::ShareScope::ThisStamp).unwrap();
    assert_eq!(doc.stamp_style_reference_size(image).unwrap(), doc.sticker(sticker).unwrap().size());
    // The presets scale with it: twice the picture, twice the contour.
    let k1 = StylePreset::ClassicSticker.styles([96, 96]).contour.unwrap().width;
    let k2 = StylePreset::ClassicSticker.styles([192, 192]).contour.unwrap().width;
    assert!((k2 - 2.0 * k1).abs() < 1e-4);
    assert!(matches!(doc.stamp_style_reference_size(crate::ids::StampId(9999)), Err(EditError::UnknownStamp)));
}
