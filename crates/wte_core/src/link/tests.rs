// Tests of package P5c (STICKER_PLAN 7 "P5c", 13.9 "P5c"; names `k5_link_*`): the link data, the mapping table of 5.4
// row by row, the ids rules of 13.2, scale, the state table of 5.5, the atomic write with `.bak.psd` and the failed
// rename, the save rule and `LoadReport::links`, the forcing features and the refusal sentences per editor. The
// fixture tests (PhotoCraft and Photoshop files, the new f15) are `fixture_tests.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec2;

use super::map::{ImportContext, ImportProblem, LayerOutcome, import, import_fitted, mask_crc};
use super::*;
use crate::doc_links::LinkApplied;
use crate::doc_stickers::StickerRef;
use crate::document::{Commit, Document};
use crate::ids::{StampId, StickerId, StickerLayerId, TextureSetId};
use crate::image_io::PngLevel;
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf};
use crate::progress::NoProgress;
use crate::project::save::testkit::{cube_doc, noise_texture};
use crate::project::{LinkFileState, SaveOptions, SaveWarning, load, read_link_archive};
use crate::psd::testkit as kit;
use crate::psd::{PsdBlend, PsdDoc, PsdError, PsdFeatures, PsdLimits, PsdMask, PsdRefusal};
use crate::stamp::{Placement, StampSource, UvPlacement};
use crate::sticker::adjust::Adjustment;
use crate::sticker::shape::{Paint, ShapeKind, ShapeSpec};
use crate::sticker::style::{Contour, StyleStack};
use crate::sticker::{LayerMask, LayerTransform, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::testutil::temp_dir;
use crate::text::{TextAlign, TextFx, TextSpec};

pub(super) const PS: (&str, &str) = (EDITOR_PHOTOSHOP, "Photoshop");
pub(super) const PC: (&str, &str) = (EDITOR_PHOTOCRAFT, "PhotoCraft");

/// The cube document of the project tests (one 64 x 64 set) with an empty history.
pub(super) fn base_doc() -> Document {
    let mut doc = cube_doc(&noise_texture(64, 64, 4));
    doc.history.clear();
    doc
}

/// A soft-edged disc of `rgb`: alpha 255 to radius `r0`, 0 from `r1`, colour noise kept under alpha 0 inside the box.
pub(super) fn soft_disc(w: u32, h: u32, r0: f32, r1: f32, rgb: [u8; 3]) -> PixelBuf {
    let mut p = PixelBuf::new(w, h);
    let c = Vec2::new(w as f32, h as f32) * 0.5;
    for y in 0..h {
        for x in 0..w {
            let d = (Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c).length();
            let a = ((r1 - d) / (r1 - r0)).clamp(0.0, 1.0);
            p.set(x, y, [rgb[0], rgb[1].wrapping_add(x as u8), rgb[2], (a * 255.0 + 0.5) as u8]);
        }
    }
    p
}

pub(super) fn owned(doc: &mut Document, pixels: PixelBuf) -> PicturePixels {
    PicturePixels::Owned { pixels: Arc::new(pixels), version: doc.next_pixel_version() }
}

pub(super) fn text_spec(doc: &mut Document, content: &str, size: f32) -> TextSpec {
    TextSpec {
        content: content.to_owned(),
        font: doc.default_font(),
        size_px: size,
        color: [20, 40, 200, 255],
        align: TextAlign::Center,
        line_spacing: 1.0,
        letter_spacing: 0.0,
        outline: None,
        fx: TextFx::default(),
    }
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

/// The test sticker (64 x 48): `photo` (a soft disc, whole-pixel move, a contour style), `text` ("WTE"), `shape` (a
/// red ellipse), `masked` (a picture with a ramp mask). Shown by one UV stamp (link scale 1).
pub(super) fn sticker_doc(doc: &mut Document) -> StickerDoc {
    let mut s = StickerDoc::new("Logo", 64, 48);
    let id = s.alloc_layer();
    let mut photo = StickerLayer::new(id, "photo", StickerContent::Picture(owned(doc, soft_disc(30, 30, 9.0, 14.0, [200, 60, 30]))));
    photo.transform = LayerTransform::moved(2.0, 4.0);
    photo.styles = StyleStack {
        contour: Some(Contour { on: true, width: 3.0, smooth: 0.0, fill_holes: true, color: [255; 4], opacity: 1.0 }),
        ..StyleStack::default()
    };
    s.layers.push(photo);
    let id = s.alloc_layer();
    let mut text = StickerLayer::new(id, "text", StickerContent::Text(Box::new(text_spec(doc, "WTE", 18.0))));
    text.transform = LayerTransform::moved(44.0, 14.0);
    s.layers.push(text);
    let id = s.alloc_layer();
    let shape = ShapeSpec { kind: ShapeKind::Ellipse, size: Vec2::new(18.0, 14.0), fill: Paint::Solid([230, 20, 20, 255]), outline: None };
    let mut shape = StickerLayer::new(id, "shape", StickerContent::Shape(shape));
    shape.transform = LayerTransform::moved(38.0, 28.0);
    s.layers.push(shape);
    let id = s.alloc_layer();
    let mut masked = StickerLayer::new(id, "masked", StickerContent::Picture(owned(doc, PixelBuf::filled(20, 12, [30, 160, 90, 255]))));
    masked.transform = LayerTransform::moved(4.0, 32.0);
    let ramp = AlphaBuf::from_raw(20, 12, (0..240u32).map(|i| ((i % 20) * 12) as u8).collect()).unwrap();
    masked.mask = Some(LayerMask { pixels: Arc::new(ramp), version: doc.next_pixel_version() });
    s.layers.push(masked);
    s
}

/// A document with the test sticker on a stamp.
pub(super) fn linked_doc() -> (Document, StickerId, StampId) {
    let mut doc = base_doc();
    let s = sticker_doc(&mut doc);
    let sid = doc.add_sticker(s).unwrap();
    let stamp = doc.add_stamp("Logo", StampSource::Sticker { sticker: sid }, uv([32.0, 32.0], [30.0, 20.0]), TextureSetId(0)).unwrap();
    (doc, sid, stamp)
}

/// Makes (or sends) the link of `sid` for `editor` without a disk: the prepared bytes are "written" as they are.
pub(super) fn make_link(doc: &mut Document, sid: StickerId, editor: &str) -> Vec<u8> {
    let file = doc.link(sid).map_or_else(|| LinkFile::InFolder(link_file_name("Logo", sid)), |l| l.file.clone());
    let prepared = doc.prepare_link_file(sid, file, editor).unwrap();
    let bytes = prepared.bytes.clone();
    doc.finish_link_write(prepared, FileStamp::of_bytes(&bytes, None)).unwrap();
    bytes
}

pub(super) fn read(bytes: &[u8]) -> PsdDoc {
    crate::psd::read(bytes, &PsdLimits::default()).unwrap()
}

/// A stamp for a file content "the editor saved" (any CRC other than the synced one).
pub(super) fn other(n: u32) -> FileStamp {
    FileStamp { size: 1000 + n as u64, modified_ns: Some(n as u64), crc32: 0xC0FF_EE00 ^ n }
}

pub(super) fn cx<'a>(editor: (&'a str, &'a str), blobs: &'a [(crate::ids::BlobId, [u32; 2])]) -> ImportContext<'a> {
    ImportContext { editor_key: editor.0, editor_name: editor.1, file_name: "Logo_k1.psd", blob_sizes: blobs, layer_hashes: &[] }
}

/// `import` of `psd` against the current sticker and link of `sid`.
pub(super) fn import_now(doc: &Document, sid: StickerId, psd: &PsdDoc, editor: (&str, &str)) -> Result<super::LinkImport, ImportProblem> {
    let sticker = (**doc.sticker(sid).unwrap()).clone();
    let link = doc.link(sid).unwrap().clone();
    import(psd, &sticker, &link, &cx(editor, &[]))
}

pub(super) fn layer<'a>(psd: &'a mut PsdDoc, name: &str) -> &'a mut crate::psd::PsdLayer {
    psd.layers.iter_mut().find(|l| l.name == name).unwrap_or_else(|| panic!("no layer {name}"))
}

pub(super) fn by_name<'a>(s: &'a StickerDoc, name: &str) -> &'a StickerLayer {
    s.layers.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no layer {name}"))
}

/// `buf` moved by `(dx, dy)`.
fn moved(buf: &CroppedBuf, dx: i32, dy: i32) -> CroppedBuf {
    let r = buf.rect;
    CroppedBuf { rect: IRect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy), data: buf.data.clone() }
}

pub(super) fn names(s: &StickerDoc) -> Vec<&str> {
    s.layers.iter().map(|l| l.name.as_str()).collect()
}

pub(super) fn all_untouched(outcomes: &[LayerOutcome]) -> bool {
    outcomes.iter().all(|o| matches!(o, LayerOutcome::Untouched(_)))
}

// ----- export, ids kept, the comparison rule -----

#[test]
fn k5_link_export_and_untouched_import_give_the_same_sticker() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let link = doc.link(sid).unwrap().clone();
    assert_eq!(link.scale, 1.0);
    assert_eq!(link.layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), [1, 2, 3, 4], "lyid 1.. bottom to top");
    assert_eq!(link.next_lyid, 5);
    let psd = read(&bytes);
    assert_eq!(psd.size, [64, 48]);
    assert_eq!(kit::mode(&psd, true), kit::Mode::Layers);
    // The masked layer carries its mask; the photo's styles are not in the file (its pixels are the bare disc).
    assert!(psd.layers[3].mask.is_some() && psd.layers[0].mask.is_none());
    assert!(psd.layers[0].rect.width() <= 30, "no contour in the file: {:?}", psd.layers[0].rect);
    // The file read back is the sticker: every layer untouched, the sticker value equal (text and shape stay).
    let before = (**doc.sticker(sid).unwrap()).clone();
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
    assert_eq!(made.sticker, before);
    // `import` is pure: the sync info of each entry (`wte`) is the document's to fill (check-P5c).
    let without_wte = |m: &[LinkLayer]| m.iter().map(|e| LinkLayer { wte: None, ..e.clone() }).collect::<Vec<_>>();
    assert!(made.layer_map.iter().all(|e| e.wte.is_none()) && link.layer_map.iter().all(|e| e.wte.is_some()));
    assert_eq!(made.layer_map, without_wte(&link.layer_map));
    let undo = doc.history.undo_len();
    let applied = doc.apply_link_update(sid, &psd, FileStamp::of_bytes(&bytes, None), "Photoshop", "Logo_k1.psd").unwrap();
    assert_eq!(applied, LinkApplied::Updated { mode: LinkMode::Layers, toast: None, profile_note: None, log: Vec::new() });
    assert_eq!(doc.history.undo_len(), undo, "nothing changed: no undo step");
    assert_eq!(**doc.sticker(sid).unwrap(), before);
    assert_eq!(doc.link_state(sid, Some(&FileStamp::of_bytes(&bytes, None))).unwrap(), LinkState::Linked);

    // Writing the same sticker again gives the same bytes and ids.
    let again = doc.prepare_link_file(sid, link.file.clone(), EDITOR_PHOTOSHOP).unwrap();
    assert_eq!(again.bytes, bytes);
    // A layer added in WTE keeps the old ids and takes next_lyid; a deleted one's id is never reused.
    let text = by_name(&before, "text").id;
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers.retain(|l| l.id != text), "Delete Layer", Commit::Final).unwrap();
    doc.set_sticker(
        StickerRef::Id(sid),
        |d| {
            let id = d.alloc_layer();
            let pixels = PicturePixels::Owned { pixels: Arc::new(PixelBuf::filled(4, 4, [1, 2, 3, 255])), version: 0 };
            d.layers.push(StickerLayer::new(id, "added", StickerContent::Picture(pixels)));
        },
        "Add Layer",
        Commit::Final,
    )
    .unwrap();
    make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let link = doc.link(sid).unwrap();
    assert_eq!(
        link.layer_map.iter().map(|e| (e.lyid, e.name.as_str())).collect::<Vec<_>>(),
        [(1, "photo"), (3, "shape"), (4, "masked"), (5, "added")]
    );
    assert_eq!(link.next_lyid, 6);
}

#[test]
fn k5_link_an_empty_sticker_is_written_with_one_empty_layer_that_never_comes_back() {
    let mut doc = base_doc();
    let sid = doc.add_sticker(StickerDoc::new("Empty", 20, 10)).unwrap();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let psd = read(&bytes);
    assert_eq!(psd.layers.len(), 1, "check-P5b finding 1: never a flat file");
    assert_eq!((psd.layers[0].name.as_str(), psd.layers[0].rect), ("Layer 1", IRect::EMPTY));
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.outcomes, [LayerOutcome::Placeholder]);
    assert!(made.sticker.layers.is_empty());
    // Painted in the editor: a new picture layer.
    let mut painted = psd.clone();
    painted.layers[0].pixels = Some(CroppedBuf { rect: IRect::new(2, 2, 4, 3), data: vec![9, 9, 9, 255, 9, 9, 9, 255] });
    painted.layers[0].rect = IRect::new(2, 2, 4, 3);
    let made = import_now(&doc, sid, &painted, PS).unwrap();
    assert!(matches!(made.outcomes[..], [LayerOutcome::New(_)]), "{:?}", made.outcomes);
    assert_eq!(made.sticker.layers.len(), 1);
}

// ----- the mapping table of 5.4, row by row -----

#[test]
fn k5_link_rows_moved_mask_pixels_new_deleted_clip_props() {
    let (mut doc, sid, _) = linked_doc();
    // An adjustment on the photo (part of the pixels WTE sends; removed when the pixels change).
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[0].adjustments.push(Adjustment::Invert), "Adjust", Commit::Final).unwrap();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let base = read(&bytes);
    let before = (**doc.sticker(sid).unwrap()).clone();
    let id = |n: &str| by_name(&before, n).id;

    // Moved: same bytes at another origin -> kept (a shape stays a shape) and moved.
    let mut psd = base.clone();
    let l = layer(&mut psd, "shape");
    l.pixels = l.pixels.as_ref().map(|p| moved(p, 3, -2));
    l.rect = l.pixels.as_ref().unwrap().rect;
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.outcomes[2], LayerOutcome::Moved(id("shape")));
    let shape = by_name(&made.sticker, "shape");
    assert!(matches!(shape.content, StickerContent::Shape(_)));
    assert_eq!(shape.transform.translate, by_name(&before, "shape").transform.translate + Vec2::new(3.0, -2.0));

    // A mask added to a plain picture (whole-pixel move, scale 1): kept, its mask replaced (mapped into picture space).
    let mut psd = base.clone();
    let r = layer(&mut psd, "photo").rect;
    let values: Vec<u8> = (0..r.area()).map(|i| (i % 7 * 30) as u8).collect();
    layer(&mut psd, "photo").mask =
        Some(PsdMask { rect: r, default: 0, pixels: AlphaBuf::from_raw(r.width(), r.height(), values).unwrap(), disabled: false, rendered: false });
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.outcomes[0], LayerOutcome::MaskChanged { layer: id("photo"), kept: true });
    let photo = by_name(&made.sticker, "photo");
    assert_eq!(photo.content, by_name(&before, "photo").content, "the picture itself is the same");
    assert_eq!(photo.adjustments, [Adjustment::Invert], "kept as it is: adjustments stay");
    let m = photo.mask.as_ref().unwrap();
    assert_eq!((m.pixels.width, m.pixels.height), (30, 30));
    let pm = layer(&mut psd, "photo").mask.clone().unwrap();
    for (px, py) in [(0u32, 0u32), (15, 15), (29, 3), (7, 22)] {
        assert_eq!(m.pixels.get(px, py), pm.value(px as i32 + 2, py as i32 + 4), "picture pixel ({px}, {py})");
    }

    // A mask added to a text layer: the text becomes a picture with the PSD pixels and the mask.
    let mut psd = base.clone();
    let r = layer(&mut psd, "text").rect;
    layer(&mut psd, "text").mask =
        Some(PsdMask { rect: r, default: 255, pixels: AlphaBuf::filled(r.width(), r.height(), 100), disabled: false, rendered: false });
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.outcomes[1], LayerOutcome::MaskChanged { layer: id("text"), kept: false });
    let text = by_name(&made.sticker, "text");
    assert!(matches!(text.content, StickerContent::Picture(_)));
    assert!(text.mask.as_ref().is_some_and(|m| m.pixels.data.iter().all(|&v| v == 100)));
    assert!(matches!(by_name(&made.sticker, "shape").content, StickerContent::Shape(_)), "only this layer");

    // Pixels changed: a picture with the PSD pixels; adjustments removed; styles kept; id kept.
    let mut psd = base.clone();
    let l = layer(&mut psd, "photo");
    let p = l.pixels.as_mut().unwrap();
    p.data[..4].copy_from_slice(&[1, 2, 3, 255]);
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.outcomes[0], LayerOutcome::PixelsChanged(id("photo")));
    let photo = by_name(&made.sticker, "photo");
    assert!(photo.adjustments.is_empty());
    assert_eq!(photo.styles, by_name(&before, "photo").styles, "the WTE layer styles are kept");
    let StickerContent::Picture(PicturePixels::Owned { pixels, .. }) = &photo.content else { panic!() };
    let r = psd.layers[0].rect;
    assert_eq!((pixels.width, pixels.height), (r.width(), r.height()));
    assert_eq!(photo.transform, LayerTransform::moved(r.x0 as f32, r.y0 as f32));
    assert_eq!(pixels.get(0, 0), [1, 2, 3, 255]);

    // A new layer (unknown id), a deleted one, clip, name, visibility, opacity and blend from the file.
    let mut psd = base.clone();
    let mut extra = psd.layers[2].clone();
    extra.name = "pc new".into();
    extra.lyid = Some(40);
    psd.layers.push(extra);
    psd.layers.retain(|l| l.name != "masked");
    let l = layer(&mut psd, "shape");
    l.clipped = true;
    l.visible = false;
    l.opacity = 128;
    l.blend = PsdBlend::Known(BlendMode::Multiply);
    l.name = "renamed".into();
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(
        matches!(
            made.outcomes[..],
            [
                LayerOutcome::Untouched(_),
                LayerOutcome::Untouched(_),
                LayerOutcome::Untouched(_),
                LayerOutcome::New(_),
                LayerOutcome::Deleted(_)
            ]
        ),
        "{:?}",
        made.outcomes
    );
    assert_eq!(names(&made.sticker), ["photo", "text", "renamed", "pc new"]);
    let shape = &made.sticker.layers[2];
    assert_eq!((shape.id, shape.clip, shape.visible, shape.blend), (id("shape"), true, false, BlendMode::Multiply));
    assert!((shape.opacity - 128.0 / 255.0).abs() < 1e-6);
    let new = &made.sticker.layers[3];
    assert!(new.id.0 >= before.next_layer, "a fresh layer id");
    assert_eq!(made.layer_map.last().map(|e| e.lyid), Some(40));
    assert_eq!(made.next_lyid, 41);
}

#[test]
fn k5_link_a_disabled_mask_is_no_mask_and_not_a_change_every_time() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let mut psd = read(&bytes);
    layer(&mut psd, "masked").mask.as_mut().unwrap().disabled = true;
    let LinkApplied::Updated { .. } = doc.apply_link_update(sid, &psd, other(1), "Photoshop", "Logo_k1.psd").unwrap() else { panic!() };
    let s = (**doc.sticker(sid).unwrap()).clone();
    assert!(by_name(&s, "masked").mask.is_none(), "a disabled mask is imported as no mask");
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
    assert_eq!(made.sticker, s);
}

#[test]
fn k5_link_each_forcing_feature_comes_back_merged_with_its_sentence_per_editor() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let base = read(&bytes);
    let cases = [
        (PsdFeatures::EFFECTS, "layer effects"),
        (PsdFeatures::GROUP, "groups"),
        (PsdFeatures::ADJUSTMENT, "adjustment layers"),
        (PsdFeatures::VECTOR_MASK, "vector masks"),
        (PsdFeatures::MASK_EXTRA, "mask density or feather"),
        (PsdFeatures::FILL_OPACITY, "fill opacity below 100 %"),
        (PsdFeatures::BLEND_OTHER, "other blend modes"),
        (PsdFeatures::BLEND_IF, "blend-if ranges"),
        (PsdFeatures::NO_PIXELS, "shape or fill layers without pixels"),
        (PsdFeatures::SMART, "smart objects"),
    ];
    for (feature, words) in cases {
        let mut psd = base.clone();
        layer(&mut psd, "shape").features |= feature;
        for editor in [PS, PC] {
            let made = import_now(&doc, sid, &psd, editor).unwrap();
            if feature == PsdFeatures::SMART && editor == PC {
                // SMART is not forcing on a PhotoCraft link (13.2): the file comes back layer by layer.
                assert_eq!(made.mode, LinkMode::Layers);
                assert!(made.toast.is_none());
                continue;
            }
            assert_eq!(made.mode, LinkMode::Merged, "{words} / {}", editor.1);
            assert_eq!(names(&made.sticker), [format!("{} picture", editor.1).as_str()]);
            let want = format!(
                "This sticker now uses {words}, which only {0} can show. WTE shows the finished picture; keep editing its layers in {0}.",
                editor.1
            );
            assert_eq!(made.toast.as_deref(), Some(want.as_str()));
            assert_eq!(made.sticker.layers[0].transform, LayerTransform::IDENTITY, "scale 1, at the artboard origin");
            let StickerContent::Picture(PicturePixels::Owned { pixels, .. }) = &made.sticker.layers[0].content else { panic!() };
            assert_eq!(**pixels, *base.merged.as_ref().unwrap());
            // Without a real merged picture: needs a merged picture.
            let mut no_merged = psd.clone();
            no_merged.merged = None;
            no_merged.has_real_merged = false;
            assert_eq!(import_now(&doc, sid, &no_merged, editor), Err(ImportProblem::NeedsMerged));
        }
    }
    // Two features: both named.
    let mut psd = base.clone();
    layer(&mut psd, "shape").features |= PsdFeatures::EFFECTS | PsdFeatures::GROUP;
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(made.toast.unwrap().starts_with("This sticker now uses layer effects and groups, which"));
    // A feature on a hidden layer forces nothing (except a group); TEXT is never forcing.
    let mut psd = base.clone();
    let l = layer(&mut psd, "shape");
    l.visible = false;
    l.features |= PsdFeatures::EFFECTS;
    layer(&mut psd, "text").features |= PsdFeatures::TEXT;
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.mode, LinkMode::Layers);
    assert!(made.editor_native);
}

#[test]
fn k5_link_merged_replaces_the_layers_and_undo_restores_them() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let before = (**doc.sticker(sid).unwrap()).clone();
    let mut psd = read(&bytes);
    layer(&mut psd, "photo").features |= PsdFeatures::EFFECTS;
    let stamp = other(7);
    let applied = doc.apply_link_update(sid, &psd, stamp, "Photoshop", "Logo_k1.psd").unwrap();
    let LinkApplied::Updated { mode: LinkMode::Merged, toast: Some(toast), .. } = applied else { panic!("{applied:?}") };
    assert!(toast.starts_with("This sticker now uses layer effects, which only Photoshop can show."), "{toast}");
    assert_eq!(names(doc.sticker(sid).unwrap()), ["Photoshop picture"]);
    assert_eq!(doc.undo_label(), Some(UPDATE_LABEL));
    let link = doc.link(sid).unwrap();
    assert_eq!((link.mode, link.synced), (LinkMode::Merged, stamp));
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::Linked);
    // Undo brings the WTE layers back; the link stays as it is (outside the history): "changed in WTE", and the file
    // (unchanged on disk) is not applied again by itself.
    doc.undo();
    assert_eq!(**doc.sticker(sid).unwrap(), before);
    assert_eq!(doc.link(sid).unwrap().synced, stamp);
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::ChangedInWte);
    assert_eq!(sync_action(false, true, false), SyncAction::Nothing);
    // "Update now" applies it again.
    doc.apply_link_update(sid, &psd, stamp, "Photoshop", "Logo_k1.psd").unwrap();
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::Linked);
}

// ----- the ids rules of 13.2 -----

#[test]
fn k5_link_ids_rewritten_to_unknown_values_or_missing_are_matched_by_content() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let before = (**doc.sticker(sid).unwrap()).clone();
    for (case, ids) in [("unknown ids", [Some(101), Some(102), Some(103), Some(104)]), ("no ids", [None; 4])] {
        let mut psd = read(&bytes);
        for (l, id) in psd.layers.iter_mut().zip(ids) {
            l.lyid = id;
        }
        let made = import_now(&doc, sid, &psd, PS).unwrap();
        assert_eq!(made.ids_lost, Some("no layer carries a known id"), "{case}");
        assert!(all_untouched(&made.outcomes), "{case}: {:?}", made.outcomes);
        assert_eq!(made.sticker, before, "{case}: text stays text");
        assert_eq!(made.log, ["layer ids of Logo_k1.psd were not kept (no layer carries a known id)".to_owned()]);
        // The map is rebuilt from the file's ids, or from next_lyid upward.
        let want: Vec<u32> = if ids[0].is_some() { vec![101, 102, 103, 104] } else { vec![5, 6, 7, 8] };
        assert_eq!(made.layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), want, "{case}");
        assert!(made.next_lyid > *want.last().unwrap());
    }
}

/// The test sticker linked, then a layer deleted and one added in WTE and sent again: map ids 1, 2, 4, 5.
pub(super) fn renumber_doc() -> (Document, StickerId, Vec<u8>) {
    let (mut doc, sid, _) = linked_doc();
    make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let shape = by_name(doc.sticker(sid).unwrap(), "shape").id;
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers.retain(|l| l.id != shape), "Delete Layer", Commit::Final).unwrap();
    doc.set_sticker(
        StickerRef::Id(sid),
        |d| {
            let id = d.alloc_layer();
            let pixels = PicturePixels::Owned { pixels: Arc::new(PixelBuf::filled(6, 5, [7, 8, 9, 255])), version: 0 };
            let mut l = StickerLayer::new(id, "added", StickerContent::Picture(pixels));
            l.transform = LayerTransform::moved(50.0, 2.0);
            l.styles = StyleStack {
                contour: Some(Contour { on: true, width: 2.0, smooth: 0.0, fill_holes: true, color: [0, 0, 0, 255], opacity: 1.0 }),
                ..StyleStack::default()
            };
            d.layers.push(l);
        },
        "Add Layer",
        Commit::Final,
    )
    .unwrap();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    assert_eq!(doc.link(sid).unwrap().layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), [1, 2, 4, 5]);
    (doc, sid, bytes)
}

#[test]
fn k5_link_ids_renumbered_by_position_after_a_delete_and_an_add() {
    let (doc, sid, bytes) = renumber_doc();
    let before = (**doc.sticker(sid).unwrap()).clone();
    let mut psd = read(&bytes);
    for (i, l) in psd.layers.iter_mut().enumerate() {
        l.lyid = Some(i as u32 + 1);
    }
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.ids_lost, Some("the ids were renumbered"));
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
    assert_eq!(made.sticker, before, "a text stays text, nothing deleted or added");
    assert_eq!(made.layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), [1, 2, 3, 4]);

    // The same with one layer repainted: pass 2 keeps it as the same layer (styles kept).
    for repainted in ["masked", "added"] {
        let mut psd = psd.clone();
        layer(&mut psd, repainted).pixels.as_mut().unwrap().data[0..4].copy_from_slice(&[200, 1, 1, 255]);
        let made = import_now(&doc, sid, &psd, PS).unwrap();
        assert_eq!(made.ids_lost, Some("the ids were renumbered"), "{repainted}");
        let id = by_name(&before, repainted).id;
        assert!(made.outcomes.contains(&LayerOutcome::PixelsChanged(id)), "{repainted}: {:?}", made.outcomes);
        assert_eq!(made.outcomes.iter().filter(|o| matches!(o, LayerOutcome::Untouched(_))).count(), 3, "{repainted}");
        assert_eq!(by_name(&made.sticker, repainted).styles, by_name(&before, repainted).styles);
        assert_eq!(made.sticker.layers.len(), 4);
    }
}

#[test]
fn k5_link_one_layer_renumbered_and_duplicate_ids() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let before = (**doc.sticker(sid).unwrap()).clone();
    // Rule 3: one layer under a new id, the others keep theirs.
    let mut psd = read(&bytes);
    layer(&mut psd, "shape").lyid = Some(77);
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.ids_lost, None);
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
    assert_eq!(made.sticker, before);
    assert_eq!(made.layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), [1, 2, 77, 4], "the entry takes the new id");
    // Duplicate ids: the first from the bottom keeps the id, the later one is "unknown" (a new layer).
    let mut psd = read(&bytes);
    let mut copy = psd.layers[0].clone();
    copy.name = "photo copy".into();
    copy.pixels = copy.pixels.as_ref().map(|p| moved(p, 5, 0));
    psd.layers.push(copy);
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(matches!(made.outcomes[0], LayerOutcome::Untouched(_)));
    assert!(matches!(made.outcomes[4], LayerOutcome::New(_)), "{:?}", made.outcomes);
    assert_eq!(made.layer_map.iter().map(|e| e.lyid).collect::<Vec<_>>(), [1, 2, 3, 4, 5], "the copy gets a fresh id");
    // The next read of the same file: the copy is recognised by rule 3 (its fresh id is not in the file).
    doc.apply_link_update(sid, &psd, other(3), "Photoshop", "Logo_k1.psd").unwrap();
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
}

#[test]
fn k5_link_lyid_reuse_is_deleted_plus_new() {
    // PhotoCraft gives a new layer the lowest free id (fixture f12): the WTE layer "text" (id 2) deleted, a new layer
    // takes id 2 with other pixels and another name.
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOCRAFT);
    let before = (**doc.sticker(sid).unwrap()).clone();
    let mut psd = read(&bytes);
    let text = layer(&mut psd, "text");
    text.name = "pc new".into();
    text.pixels = Some(CroppedBuf { rect: IRect::new(1, 1, 3, 2), data: vec![0, 200, 200, 255, 0, 200, 200, 255] });
    let made = import_now(&doc, sid, &psd, PC).unwrap();
    assert_eq!(made.ids_lost, None);
    let text_id = by_name(&before, "text").id;
    assert!(made.outcomes.contains(&LayerOutcome::Deleted(text_id)), "{:?}", made.outcomes);
    assert!(matches!(made.outcomes[1], LayerOutcome::New(id) if id != text_id));
    assert!(by_name(&made.sticker, "pc new").styles.is_default(), "the deleted layer's styles do not move over");
}

#[test]
fn k5_link_a_photoshop_like_resave_keeps_every_layer() {
    // 13.4: a WTE file re-written the way Photoshop re-saves it: extra resources and layer blocks, rects one pixel
    // larger with white RGB under alpha 0, the 40-byte ranges, names padded to 2, pad bytes, ids kept.
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let before = (**doc.sticker(sid).unwrap()).clone();
    let src = read(&bytes);
    let mut file = kit::File::rgb(64, 48).merged_rgba(&src.merged.as_ref().unwrap().data);
    file.resources.push((1005, vec![0, 72, 0, 0, 0, 2, 0, 2, 0, 72, 0, 0, 0, 2, 0, 2]));
    file.resources.push((1044, 4u32.to_be_bytes().to_vec()));
    file.resources.push((1077, vec![0; 6]));
    file = file.version_info(true);
    for l in &src.layers {
        let p = l.pixels.as_ref().unwrap();
        let r = p.rect;
        let big = IRect::new(r.x0 - 1, r.y0 - 1, r.x1 + 1, r.y1 + 1).intersect(&IRect::from_size(64, 48));
        let mut rgba = Vec::new();
        for y in big.y0..big.y1 {
            for x in big.x0..big.x1 {
                let px = p.get(x, y);
                rgba.extend_from_slice(if px[3] == 0 { &[255, 255, 255, 0] } else { &px });
            }
        }
        let mut out = kit::Layer::rgba(&l.name, [big.y0, big.x0, big.y1, big.x1], &rgba)
            .block(b"luni", kit::unicode(&l.name))
            .lyid(l.lyid.unwrap())
            .block(b"lnsr", b"layr".to_vec())
            .block(b"clbl", vec![1, 0, 0, 0])
            .block(b"shmd", vec![0, 0, 0, 0]);
        out.name_pad = Some(1 - l.name.len() % 2);
        if let Some(m) = &l.mask {
            out = out.mask([m.rect.y0, m.rect.x0, m.rect.y1, m.rect.x1], &m.pixels.data, m.default, 0);
        }
        file.layers.push(out);
    }
    let ps = read(&file.build());
    assert_eq!(ps.layers.len(), 4);
    assert_ne!(ps.layers[0].rect, src.layers[0].rect, "the rects differ");
    let made = import_now(&doc, sid, &ps, PS).unwrap();
    assert_eq!(made.ids_lost, None);
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
    assert_eq!(made.sticker, before);
}

// ----- scale (review B4) -----

/// The test sticker with a picture shown at half size: link scale 2.
pub(super) fn scaled_doc() -> (Document, StickerId) {
    let (mut doc, sid, _) = linked_doc();
    doc.set_sticker(
        StickerRef::Id(sid),
        |d| {
            let id = d.alloc_layer();
            let pixels = PicturePixels::Owned { pixels: Arc::new(soft_disc(16, 16, 4.0, 7.0, [9, 90, 190])), version: 0 };
            let mut l = StickerLayer::new(id, "half", StickerContent::Picture(pixels));
            l.transform = LayerTransform { translate: Vec2::new(20.0, 20.0), scale: Vec2::splat(0.5), rotation: 0.0 };
            d.layers.push(l);
        },
        "Add Layer",
        Commit::Final,
    )
    .unwrap();
    (doc, sid)
}

#[test]
fn k5_link_scale_two_text_untouched_and_a_changed_layer_keeps_its_size() {
    let (mut doc, sid) = scaled_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    assert_eq!(doc.link(sid).unwrap().scale, 2.0);
    let before = (**doc.sticker(sid).unwrap()).clone();
    let psd = read(&bytes);
    assert_eq!(psd.size, [128, 96], "canvas = artboard * 2");
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(all_untouched(&made.outcomes), "{:?}", made.outcomes);
    assert_eq!(made.sticker, before, "the text exported at scale 2 comes back as the same text");

    // A changed text: a picture at scale 1/2, placed so that its on-model size and place are the PSD's / 2.
    let mut psd2 = psd.clone();
    layer(&mut psd2, "text").pixels.as_mut().unwrap().data[3] ^= 0x40;
    let made = import_now(&doc, sid, &psd2, PS).unwrap();
    let text = by_name(&made.sticker, "text");
    let StickerContent::Picture(PicturePixels::Owned { pixels, .. }) = &text.content else { panic!() };
    assert_eq!(text.transform.scale, Vec2::splat(0.5));
    let r = crate::psd::write::normalise(psd2.layers[1].pixels.as_ref().unwrap()).rect;
    let size = [pixels.width, pixels.height];
    let tl = text.transform.apply(size, Vec2::ZERO);
    let br = text.transform.apply(size, Vec2::new(size[0] as f32, size[1] as f32));
    assert!((tl - Vec2::new(r.x0 as f32, r.y0 as f32) / 2.0).length() < 1e-4, "{tl}");
    assert!((br - Vec2::new(r.x1 as f32, r.y1 as f32) / 2.0).length() < 1e-4, "{br}");

    // A canvas change maps to the artboard through the scale (top-left kept): 148 / 2 and 105 / 2 rounded up.
    let mut psd3 = psd.clone();
    psd3.size = [148, 105];
    let made = import_now(&doc, sid, &psd3, PS).unwrap();
    assert_eq!(made.sticker.artboard, IRect::new(0, 0, 74, 53));
}

#[test]
fn k5_link_a_canvas_over_the_artboard_limit_applies_nothing_then_fits() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let before = (**doc.sticker(sid).unwrap()).clone();
    let mut psd = read(&bytes);
    psd.size = [5000, 40];
    psd.merged = Some(PixelBuf::filled(5000, 40, [10, 20, 30, 255]));
    let stamp = other(5);
    let applied = doc.apply_link_update(sid, &psd, stamp, "Photoshop", "Logo_k1.psd").unwrap();
    assert_eq!(applied, LinkApplied::Problem(LinkState::CanvasTooLarge { width: 5000, height: 40 }));
    assert_eq!(**doc.sticker(sid).unwrap(), before, "nothing applied");
    let badge = LinkState::CanvasTooLarge { width: 5000, height: 40 }.badge("Photoshop", "Logo", "Logo_k1.psd");
    assert_eq!(badge, "Logo is now 5000 x 40 in Photoshop; a sticker can be at most 4096 px.");
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::CanvasTooLarge { width: 5000, height: 40 });
    // "Fit it into the sticker": one merged picture, scale 5000 / 4096, artboard 4096 x 33, no pixel thrown away.
    let LinkApplied::Updated { mode, .. } = doc.apply_link_fit(sid, &psd, stamp, "Photoshop", "Logo_k1.psd").unwrap() else { panic!() };
    assert_eq!(mode, LinkMode::Merged);
    let s = doc.sticker(sid).unwrap();
    assert_eq!(s.artboard, IRect::new(0, 0, 4096, 33));
    assert_eq!(doc.link(sid).unwrap().scale, 5000.0 / 4096.0);
    let StickerContent::Picture(PicturePixels::Owned { pixels, .. }) = &s.layers[0].content else { panic!() };
    assert_eq!((pixels.width, pixels.height), (5000, 40));
    // Over the reader's limits: the "too large" message only.
    let mut big = psd.clone();
    big.too_large = true;
    big.size = [9000, 9000];
    let applied = doc.apply_link_update(sid, &big, other(6), "Photoshop", "Logo_k1.psd").unwrap();
    assert_eq!(applied, LinkApplied::Problem(LinkState::TooLarge { width: 9000, height: 9000 }));
    assert_eq!(
        LinkState::TooLarge { width: 9000, height: 9000 }.badge("Photoshop", "Logo", "Logo_k1.psd"),
        "Logo_k1.psd is too large for WTE (9000 x 9000; at most 8192 px a side)."
    );
}

#[test]
fn k5_link_scale_rules() {
    let mut doc = base_doc();
    let mut s = sticker_doc(&mut doc);
    assert_eq!(link_scale(&s, 1.0), 1);
    assert_eq!(link_scale(&s, 1.5), 2, "the Final render scale");
    assert_eq!(link_scale(&s, 9.0), 4, "at most 4");
    s.layers[0].transform.scale = Vec2::new(0.3, 0.5);
    assert_eq!(link_scale(&s, 1.0), 4, "ceil(1 / 0.3) = 4");
    s.layers[0].transform.scale = Vec2::new(-0.5, 0.5);
    assert_eq!(link_scale(&s, 1.0), 2, "a flip counts by its size");
    s.artboard = IRect::from_size(1500, 100);
    assert_eq!(link_scale(&s, 4.0), 2, "lowered until the canvas fits 4096");
    s.artboard = IRect::from_size(4096, 100);
    assert_eq!(link_scale(&s, 4.0), 1);
}

// ----- the state table of 5.5 -----

#[test]
fn k5_link_state_table_and_the_stale_rule() {
    let (mut doc, sid, stamp) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let synced = FileStamp::of_bytes(&bytes, None);
    let copied = FileStamp { modified_ns: Some(12345), ..synced };
    let edited = other(1);
    assert_eq!(doc.link_state(sid, Some(&synced)).unwrap(), LinkState::Linked);
    assert_eq!(doc.link_state(sid, Some(&copied)).unwrap(), LinkState::Linked, "the same content with another time");
    assert_eq!(doc.link_state(sid, None).unwrap(), LinkState::Missing);
    assert_eq!(doc.link_state(sid, Some(&edited)).unwrap(), LinkState::ChangedInEditor);
    assert_eq!(sync_action(true, false, false), SyncAction::Apply);
    // Styles, the stamp and the sticker name are not in the file: not "changed in WTE".
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[1].styles = d.layers[0].styles.clone(), "Style", Commit::Final).unwrap();
    doc.set_stamp_styles(stamp, StyleStack::default(), Commit::Final).unwrap();
    doc.set_sticker(StickerRef::Id(sid), |d| d.name = "Renamed".into(), "Rename", Commit::Final).unwrap();
    assert_eq!(doc.link_state(sid, Some(&synced)).unwrap(), LinkState::Linked);
    // A change in WTE.
    doc.set_sticker(StickerRef::Id(sid), |d| d.layers[2].opacity = 0.5, "Layer Opacity", Commit::Final).unwrap();
    assert_eq!(doc.link_state(sid, Some(&synced)).unwrap(), LinkState::ChangedInWte);
    assert_eq!(sync_action(false, true, false), SyncAction::Nothing, "nothing is written or applied by itself");
    let link = doc.link(sid).unwrap().clone();
    assert_eq!(send_first_question(&link, true, "Photoshop").as_deref(), Some("Send WTE's changes to the file first?"));
    assert_eq!(send_first_question(&link, false, "Photoshop"), None);
    let native = StickerLink { editor_native: true, ..link.clone() };
    assert_eq!(
        send_first_question(&native, true, "Photoshop").as_deref(),
        Some(
            "Send WTE's changes to the file first? This replaces Photoshop's own layers (text, effects) in the file with plain picture layers."
        )
    );
    // Both: a conflict, never automatic.
    assert_eq!(doc.link_state(sid, Some(&edited)).unwrap(), LinkState::Conflict);
    assert_eq!(sync_action(true, true, false), SyncAction::Conflict { saved_over: false });
    // Undo back to the synced sticker: in sync again.
    doc.undo();
    assert_eq!(doc.link_state(sid, Some(&synced)).unwrap(), LinkState::Linked);

    // The stale rule (5.5, 13.1): a rewrite of a file opened this session turns the next change into a question.
    let stale = next_stale(false, StaleEvent::Rewrote { opened: true });
    assert!(stale);
    assert!(!next_stale(false, StaleEvent::Rewrote { opened: false }));
    assert_eq!(sync_action(true, false, stale), SyncAction::Conflict { saved_over: true });
    assert!(next_stale(stale, StaleEvent::EditAgain { child_exited: false }), "the old window may still be open");
    assert!(!next_stale(stale, StaleEvent::EditAgain { child_exited: true }));
    assert!(!next_stale(stale, StaleEvent::Answered));
    assert_eq!(texts::saved_over("Logo", "Photoshop"), "Photoshop saved over the changes WTE sent to Logo.");
    assert_eq!(texts::conflict("Logo", "PhotoCraft"), "Both WTE and PhotoCraft changed Logo.");
    assert_eq!(rewrite_warning(OpenedState::NotOpened, "Photoshop"), None);
    assert_eq!(
        rewrite_warning(OpenedState::ChildRunning, "PhotoCraft").as_deref(),
        Some(
            "PhotoCraft has this file open. Its next save will replace what you send now. Close the file in PhotoCraft first, or reopen it there after sending."
        )
    );
    assert!(rewrite_warning(OpenedState::Unknown, "Photoshop").unwrap().starts_with("Photoshop may still have this file open."));
}

#[test]
fn k5_link_badges_and_refusal_sentences() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    for (state, want) in [
        (LinkState::Linked, "linked"),
        (LinkState::ChangedInWte, "changed in WTE"),
        (LinkState::ChangedInEditor, "changed in Photoshop"),
        (LinkState::Conflict, "conflict"),
        (LinkState::Missing, "file missing"),
        (LinkState::NeedsMerged, "needs a merged picture"),
    ] {
        assert_eq!(state.badge("Photoshop", "Logo", "Logo_k1.psd"), want);
    }
    assert_eq!(LinkState::ChangedInEditor.badge("gimp-2.10", "Logo", "x.psd"), "changed in gimp-2.10");
    // Every refusal: its sentence on the badge while the file has that content; the sticker and the sync untouched.
    let synced = doc.link(sid).unwrap().synced;
    let before = (**doc.sticker(sid).unwrap()).clone();
    let rgb = "In Photoshop choose Image > Mode > RGB Color and save again.";
    let bits = "In Photoshop choose Image > Mode > 8 Bits/Channel and save again.";
    let refusals = [
        (PsdRefusal::Bitmap, format!("Logo_k1.psd is in Bitmap mode. {rgb}")),
        (PsdRefusal::Indexed, format!("Logo_k1.psd is in Indexed Color mode. {rgb}")),
        (PsdRefusal::Cmyk, format!("Logo_k1.psd is in CMYK Color mode. {rgb}")),
        (PsdRefusal::Multichannel, format!("Logo_k1.psd is in Multichannel mode. {rgb}")),
        (PsdRefusal::Duotone, format!("Logo_k1.psd is in Duotone mode. {rgb}")),
        (PsdRefusal::Lab, format!("Logo_k1.psd is in Lab Color mode. {rgb}")),
        (PsdRefusal::Depth32, format!("Logo_k1.psd has 32 bits per channel. {bits}")),
        (PsdRefusal::OtherMode(5), format!("Logo_k1.psd uses a colour mode WTE cannot read (5). {rgb}")),
        (PsdRefusal::OtherDepth(4), format!("Logo_k1.psd has 4 bits per channel. {bits}")),
        (PsdRefusal::Compression(9), "Logo_k1.psd uses a compression method WTE cannot read (9). Save it again with default settings.".to_owned()),
    ];
    for (i, (refusal, sentence)) in refusals.into_iter().enumerate() {
        let stamp = other(100 + i as u32);
        let state = doc.link_file_refused(sid, &PsdError::Unsupported(refusal), stamp, "Logo_k1.psd").unwrap();
        assert_eq!(state, LinkState::Refused(sentence.clone()));
        assert_eq!(state.badge("Photoshop", "Logo", "Logo_k1.psd"), sentence);
        assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), state);
        // The next content of the file is judged again.
        assert_eq!(doc.link_state(sid, Some(&other(999))).unwrap(), LinkState::ChangedInEditor);
    }
    assert_eq!(**doc.sticker(sid).unwrap(), before);
    assert_eq!(doc.link(sid).unwrap().synced, synced);
    // A half-written file is no problem (the watcher waits); a damaged one is.
    let stamp = other(50);
    assert_eq!(doc.link_file_refused(sid, &PsdError::Truncated, stamp, "Logo_k1.psd").unwrap(), LinkState::ChangedInEditor);
    let state = doc.link_file_refused(sid, &PsdError::Corrupt("bad layer"), stamp, "Logo_k1.psd").unwrap();
    assert_eq!(state.badge("Photoshop", "Logo", "Logo_k1.psd"), "Logo_k1.psd could not be read (the PSD file is damaged (bad layer)).");
    // The needs-merged sentences, per editor; the convert button only for PhotoCraft.
    let mut psd = read(&bytes);
    layer(&mut psd, "photo").features |= PsdFeatures::EFFECTS;
    psd.merged = None;
    psd.has_real_merged = false;
    let applied = doc.apply_link_update(sid, &psd, other(60), "Photoshop", "Logo_k1.psd").unwrap();
    assert_eq!(applied, LinkApplied::Problem(LinkState::NeedsMerged));
    assert_eq!(
        texts::needs_merged(EDITOR_PHOTOSHOP, "Photoshop"),
        "Photoshop saved this file without a merged picture. In Photoshop choose Edit > Preferences > File Handling, set \"Maximize PSD and PSB File Compatibility\" to Always, and save again."
    );
    assert_eq!(
        texts::needs_merged(EDITOR_PHOTOCRAFT, "PhotoCraft"),
        "PhotoCraft saved this file without a merged picture. In PhotoCraft turn on Preferences > File Handling > Maximize PSD and PSB File Compatibility and save again."
    );
    assert!(texts::needs_merged(EDITOR_OTHER, "gimp-2.10").starts_with("gimp-2.10 saved this file without a finished picture."));
    assert!(texts::offers_cli_convert(EDITOR_PHOTOCRAFT) && !texts::offers_cli_convert(EDITOR_PHOTOSHOP));
    assert_eq!(texts::updated("Photoshop"), "Updated from Photoshop");
    assert_eq!(texts::merged_layer_name("PhotoCraft"), "PhotoCraft picture");
    assert_eq!(texts::send_to("Photoshop"), "Send to Photoshop");
    assert_eq!(texts::edit_in("PhotoCraft"), "Edit in PhotoCraft");
}

// ----- links folder, names, the modified flag -----

#[test]
fn k5_link_folder_file_names_and_the_modified_flag() {
    assert_eq!(links_folder(Path::new("C:/a b/Car.wtep")), PathBuf::from("C:/a b/Car_links"));
    assert_eq!(link_file_name("Logo", StickerId(12)), "Logo_k12.psd");
    assert_eq!(link_file_name("a/b:c*?", StickerId(3)), "a_b_c___k3.psd");
    assert_eq!(link_file_name("Слой ü", StickerId(4)), "Слой ü_k4.psd");
    assert_eq!(link_file_name("  ", StickerId(5)), "sticker_k5.psd");
    assert_eq!(link_file_name(&"x".repeat(90), StickerId(6)).len(), 40 + "_k6.psd".len());
    assert_eq!(archive_entry(StickerId(7)), "links/k00007.psd");
    assert_eq!(LinkFile::InFolder("a.psd".into()).path(None), None);
    assert_eq!(LinkFile::InFolder("a.psd".into()).path(Some(Path::new("D:/p/X.wtep"))), Some(PathBuf::from("D:/p/X_links/a.psd")));
    assert_eq!(LinkFile::External("E:/art/b.psd".into()).display_name(), "b.psd");
    assert_eq!(known_editor_name(EDITOR_PHOTOSHOP), Some("Photoshop"));
    assert!(smart_forces(EDITOR_PHOTOSHOP) && smart_forces(EDITOR_OTHER) && !smart_forces(EDITOR_PHOTOCRAFT));

    let (mut doc, sid, _) = linked_doc();
    doc.mark_saved(doc.history_state());
    doc.mark_links_saved(doc.links_revision());
    assert!(!doc.is_modified());
    make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    assert!(doc.is_modified(), "a new link sets the modified flag (it is not in the history)");
    let snap = doc.snapshot();
    assert_eq!(snap.links.len(), 1);
    doc.mark_links_saved(snap.links_revision);
    assert!(!doc.is_modified());
    doc.set_link_file_and_editor(sid, None, Some(EDITOR_PHOTOCRAFT)).unwrap();
    assert!(doc.is_modified());
    assert_eq!(doc.link(sid).unwrap().editor, EDITOR_PHOTOCRAFT);
    doc.mark_links_saved(doc.links_revision());
    doc.set_link_file_and_editor(sid, None, Some(EDITOR_PHOTOCRAFT)).unwrap();
    assert!(!doc.is_modified(), "no change, no flag");
    doc.unlink(sid);
    assert!(doc.is_modified() && doc.link(sid).is_none());
}

// ----- the atomic write, .bak.psd, the failed rename (13.1 A8) -----

#[derive(Default)]
struct FakeIo {
    refuse: u32,
    tries: u32,
    pauses: Vec<u64>,
    registered: Vec<u32>,
    taken_back: Vec<u32>,
}

impl LinkFileIo for FakeIo {
    fn rename(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.tries += 1;
        if self.tries <= self.refuse {
            return Err(std::io::Error::from_raw_os_error(32));
        }
        std::fs::rename(from, to)
    }
    fn pause(&mut self, ms: u64) {
        self.pauses.push(ms);
    }
    fn before_rename(&mut self, crc32: u32) {
        self.registered.push(crc32);
    }
    fn rename_failed(&mut self, crc32: u32) {
        self.taken_back.push(crc32);
    }
}

#[test]
fn k5_link_write_keeps_one_backup_and_a_locked_file_changes_nothing() {
    let dir = temp_dir("k5_link_write");
    let target = dir.join("Logo_k1.psd");
    let bak = dir.join("Logo_k1.bak.psd");
    assert_eq!(file::backup_path(&target), bak);
    let mut io = FakeIo::default();
    let first = write_link_file(&target, b"first", &mut io).unwrap();
    assert_eq!((first.size, first.crc32), (5, crc32fast::hash(b"first")));
    assert!(first.modified_ns.is_some());
    assert!(!bak.exists(), "no backup of nothing");
    assert_eq!(io.registered, [crc32fast::hash(b"first")], "registered before the rename");
    write_link_file(&target, b"second", &mut io).unwrap();
    assert_eq!(std::fs::read(&bak).unwrap(), b"first");
    write_link_file(&target, b"third", &mut io).unwrap();
    assert_eq!((std::fs::read(&target).unwrap(), std::fs::read(&bak).unwrap()), (b"third".to_vec(), b"second".to_vec()), "one generation");
    // Two refusals: retried 200 ms apart, then it works.
    let mut io = FakeIo { refuse: 2, ..FakeIo::default() };
    write_link_file(&target, b"fourth", &mut io).unwrap();
    assert_eq!((io.tries, io.pauses.as_slice()), (3, &[200u64, 200][..]));
    assert_eq!(std::fs::read(&bak).unwrap(), b"third");
    // Always refused: 1 + 3 tries, then nothing changed: file, backup, registration; no temp file left.
    let mut io = FakeIo { refuse: u32::MAX, ..FakeIo::default() };
    let err = write_link_file(&target, b"fifth", &mut io).unwrap_err();
    assert_eq!(err, LinkWriteError::Locked { file: "Logo_k1.psd".into() });
    assert_eq!(io.tries, 4);
    assert_eq!(io.pauses, [200, 200, 200]);
    assert_eq!(io.taken_back, io.registered, "the own-write registration is taken back");
    assert_eq!(std::fs::read(&target).unwrap(), b"fourth");
    assert_eq!(std::fs::read(&bak).unwrap(), b"third");
    let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, ["Logo_k1.bak.psd", "Logo_k1.psd"]);
    assert_eq!(
        err.sentence("Photoshop"),
        "Logo_k1.psd could not be replaced because another program has it open. Close it in Photoshop (or save it there), then choose Send to Photoshop again."
    );
    // A missing folder is an I/O error, nothing written.
    let err = write_link_file(&dir.join("nope").join("x.psd"), b"x", &mut RealFileIo).unwrap_err();
    assert!(matches!(err, LinkWriteError::Io { .. }));
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- the project: the save rule, `LoadReport::links`, the manifest -----

fn options() -> SaveOptions {
    SaveOptions { png: PngLevel::Fast, write_flattened: true, keep_backup: false, now_unix: 7, app_version: "test".to_owned() }
}

fn save_doc(doc: &mut Document, path: &Path) -> Vec<SaveWarning> {
    let stats = crate::project::save(&doc.snapshot(), path, &options(), &NoProgress).unwrap();
    doc.set_png_reuse(stats.reuse);
    stats.warnings
}

fn archive_bytes(project: &Path, entry: &str) -> Option<Vec<u8>> {
    read_link_archive(project, entry).ok()
}

/// A saved project with a link whose file is written in its links folder.
pub(super) fn saved_link_project(name: &str) -> (PathBuf, PathBuf, Document, StickerId, Vec<u8>) {
    let dir = temp_dir(name);
    let project = dir.join("Car.wtep");
    let (mut doc, sid, _) = linked_doc();
    save_doc(&mut doc, &project);
    let folder = links_folder(&project);
    std::fs::create_dir_all(&folder).unwrap();
    let file = LinkFile::InFolder(link_file_name("Logo", sid));
    let prepared = doc.prepare_link_file(sid, file.clone(), EDITOR_PHOTOSHOP).unwrap();
    let bytes = prepared.bytes.clone();
    let written = write_link_file(&file.path(Some(&project)).unwrap(), &bytes, &mut RealFileIo).unwrap();
    doc.finish_link_write(prepared, written).unwrap();
    (dir, project, doc, sid, bytes)
}

#[test]
fn k5_link_save_never_writes_the_file_and_archives_it_by_the_rule() {
    let (dir, project, mut doc, sid, bytes) = saved_link_project("k5_link_save");
    let file = doc.link(sid).unwrap().file.path(Some(&project)).unwrap();
    let time = std::fs::metadata(&file).unwrap().modified().unwrap();
    assert!(save_doc(&mut doc, &project).is_empty());
    assert!(save_doc(&mut doc, &project).is_empty());
    assert_eq!(std::fs::read(&file).unwrap(), bytes, "a project save never writes a link file");
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), time);
    let entry = archive_entry(sid);
    assert_eq!(archive_bytes(&project, &entry), Some(bytes.clone()), "CRC equal: stored");
    let manifest = crate::project::read_manifest(&project).unwrap();
    assert_eq!(manifest.links.len(), 1);
    assert_eq!(manifest.links[0].archive.as_deref(), Some(entry.as_str()));
    assert_eq!(manifest.links[0].editor, "photoshop");
    assert_eq!(manifest.min_reader_version, 4);

    // The editor saved another complete file: stored as it is, no warning.
    let src = read(&bytes);
    let mut out = kit::File::rgb(64, 48).merged_rgba(&src.merged.clone().unwrap().data);
    out.layers.push(kit::Layer::rgba("only", [0, 0, 2, 2], &[5; 16]).lyid(9));
    let other_bytes = out.build();
    std::fs::write(&file, &other_bytes).unwrap();
    assert!(save_doc(&mut doc, &project).is_empty());
    assert_eq!(archive_bytes(&project, &entry), Some(other_bytes.clone()), "different and readable: stored");
    // The editor is writing it right now (half a file): the earlier copy stays, with one warning.
    std::fs::write(&file, &other_bytes[..other_bytes.len() / 2]).unwrap();
    let warnings = save_doc(&mut doc, &project);
    assert_eq!(warnings, [SaveWarning::LinkBeingWritten { sticker: sid, name: "Logo".into() }]);
    assert_eq!(warnings[0].to_string(), "Logo: the linked file is being written; the project keeps the earlier copy.");
    assert_eq!(archive_bytes(&project, &entry), Some(other_bytes), "the old archive entry is kept");
    // The pure rule.
    use crate::project::save::{ArchiveChoice, archive_choice};
    assert_eq!(archive_choice(Some(&bytes), crc32fast::hash(&bytes)), ArchiveChoice::Store);
    assert_eq!(archive_choice(Some(&bytes[..100]), crc32fast::hash(&bytes)), ArchiveChoice::KeepEarlier { warn: true });
    assert_eq!(archive_choice(None, 1), ArchiveChoice::KeepEarlier { warn: false });
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k5_link_a_first_save_with_a_half_written_file_stores_nothing_and_warns() {
    let dir = temp_dir("k5_link_first_save");
    let project = dir.join("New.wtep");
    let (mut doc, sid, _) = linked_doc();
    let prepared = doc.prepare_link_file(sid, LinkFile::InFolder("Logo_k1.psd".into()), EDITOR_PHOTOCRAFT).unwrap();
    std::fs::create_dir_all(links_folder(&project)).unwrap();
    std::fs::write(links_folder(&project).join("Logo_k1.psd"), &prepared.bytes[..50]).unwrap();
    let stamp = FileStamp::of_bytes(&prepared.bytes, None);
    doc.finish_link_write(prepared, stamp).unwrap();
    let warnings = save_doc(&mut doc, &project);
    assert_eq!(warnings.len(), 1);
    let manifest = crate::project::read_manifest(&project).unwrap();
    assert_eq!(manifest.links[0].archive, None, "none on a first save");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k5_link_load_reports_present_changed_missing_and_writes_nothing() {
    let (dir, project, mut doc, sid, bytes) = saved_link_project("k5_link_load");
    let ext = temp_dir("k5_link_load_ext").join("gone.psd");
    // A second sticker with an external link to a file that does not exist.
    let s2 = {
        let mut s = StickerDoc::new("Ext", 8, 8);
        let id = s.alloc_layer();
        s.layers.push(StickerLayer::new(id, "p", StickerContent::Picture(owned(&mut doc, PixelBuf::filled(8, 8, [1, 1, 1, 255])))));
        s.keep = true;
        doc.add_sticker(s).unwrap()
    };
    let prepared = doc.prepare_link_file(s2, LinkFile::External(ext.clone()), "other").unwrap();
    let stamp = FileStamp::of_bytes(&prepared.bytes, None);
    doc.finish_link_write(prepared, stamp).unwrap();
    save_doc(&mut doc, &project);
    let listing = |d: &Path| {
        let mut v: Vec<_> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name()).collect();
        v.sort();
        v
    };
    let folder = links_folder(&project);
    let before = (listing(&dir), listing(&folder));

    let report = load(&project, &NoProgress).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(report.links.len(), 2);
    assert_eq!(report.links[0].sticker, sid);
    assert_eq!(report.links[0].state, LinkFileState::Present);
    assert_eq!(report.links[0].archive.as_deref(), Some(archive_entry(sid).as_str()));
    assert_eq!((report.links[1].sticker, report.links[1].state, report.links[1].archive.as_deref()), (s2, LinkFileState::Missing, None));
    let mut loaded = report.document;
    assert!(!loaded.is_modified());
    let mut want = doc.link(sid).unwrap().clone();
    want.problem = None;
    assert_eq!(loaded.link(sid), Some(&want), "the manifest round trip");
    assert_eq!(loaded.link(s2).unwrap().file, LinkFile::External(ext));
    let file = links_folder(&project).join(link_file_name("Logo", sid));
    let synced = FileStamp::read(&file).unwrap().0;
    assert_eq!(loaded.link_state(sid, Some(&synced)).unwrap(), LinkState::Linked, "the sticker hash survives save and load");

    // Changed while closed.
    let mut changed = bytes.clone();
    changed.extend_from_slice(&[0; 4]);
    std::fs::write(&file, &changed).unwrap();
    assert_eq!(load(&project, &NoProgress).unwrap().links[0].state, LinkFileState::Changed);
    // A copy of the same content with another time is present.
    std::fs::write(&file, &bytes).unwrap();
    assert_eq!(load(&project, &NoProgress).unwrap().links[0].state, LinkFileState::Present);
    // Deleted: missing, restorable from the archive by the app; load itself writes nothing.
    std::fs::remove_file(&file).unwrap();
    let report = load(&project, &NoProgress).unwrap();
    assert_eq!(report.links[0].state, LinkFileState::Missing);
    assert!(!file.exists(), "load writes nothing");
    assert_eq!(read_link_archive(&project, report.links[0].archive.as_deref().unwrap()).unwrap(), bytes);
    let mut after = (listing(&dir), listing(&folder));
    after.1.push(file.file_name().unwrap().to_owned());
    after.1.sort();
    assert_eq!(after, before, "no file was written by load");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k5_link_entries_that_cannot_be_links_are_left_out() {
    use crate::project::{FileStampEntry, LinkEntry, LinkFileEntry, LinkLayerEntry, LinkModeEntry};
    let entry = |name: &str, scale: f32, ids: &[u32]| LinkEntry {
        sticker: StickerId(1),
        file: LinkFileEntry::InFolder { name: name.into() },
        editor: "photoshop".into(),
        mode: LinkModeEntry::Layers,
        scale,
        synced: FileStampEntry { size: 1, modified_unix_ns: None, crc32: 2 },
        synced_sticker: 3,
        layer_map: ids
            .iter()
            .map(|&lyid| LinkLayerEntry {
                lyid,
                layer: StickerLayerId(lyid),
                name: String::new(),
                origin: [0, 0],
                size: [0, 0],
                pixels_crc: 0,
                mask_crc: None,
                wte: None,
            })
            .collect(),
        next_lyid: 1,
        editor_native: false,
        archive: None,
    };
    let link = entry("a.psd", 2.0, &[3, 5]).to_link().unwrap();
    assert_eq!((link.scale, link.next_lyid), (2.0, 6), "next_lyid above every id");
    assert!(entry("../a.psd", 1.0, &[]).to_link().is_err());
    assert!(entry("c:x.psd", 1.0, &[]).to_link().is_err());
    assert!(entry("", 1.0, &[]).to_link().is_err());
    assert!(entry("a.psd", 0.5, &[]).to_link().is_err());
    assert!(entry("a.psd", f32::NAN, &[]).to_link().is_err());
    assert!(entry("a.psd", 1.0, &[2, 2]).to_link().is_err());
    // The revision-2 name of the native flag still reads.
    let mut old = serde_json::to_value(entry("a.psd", 1.0, &[])).unwrap();
    old.as_object_mut().unwrap().remove("editor");
    old["pc_native"] = serde_json::json!(true);
    let parsed: LinkEntry = serde_json::from_value(old).unwrap();
    assert!(parsed.editor_native);
    assert_eq!(parsed.editor, "photocraft");
    // The mask CRC: values over the layer's rectangle and the disabled flag, not the mask's own rectangle.
    let mask = PsdMask { rect: IRect::new(0, 0, 2, 1), default: 0, pixels: AlphaBuf::filled(2, 1, 9), disabled: false, rendered: false };
    let off = PsdMask { disabled: true, ..mask.clone() };
    let r = IRect::new(-1, 0, 3, 2);
    assert_ne!(mask_crc(Some(&mask), r), mask_crc(Some(&off), r), "the disabled flag counts");
    let values = (0..42).map(|i| if i % 7 >= 5 && i / 7 == 5 { 9 } else { 0 }).collect();
    let wider = PsdMask { rect: IRect::new(-5, -5, 2, 1), pixels: AlphaBuf::from_raw(7, 6, values).unwrap(), ..mask.clone() };
    assert_eq!(mask_crc(Some(&mask), r), mask_crc(Some(&wider), r), "only the values over the layer count");
    assert_eq!(mask_crc(None, r), None);
}

#[test]
fn k5_link_fitted_import_of_a_small_canvas_and_no_merged() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let psd = read(&bytes);
    let sticker = (**doc.sticker(sid).unwrap()).clone();
    let link = doc.link(sid).unwrap().clone();
    let made = import_fitted(&psd, &sticker, &link, &cx(PS, &[])).unwrap();
    assert_eq!((made.scale, made.sticker.artboard), (1.0, IRect::new(0, 0, 64, 48)));
    let mut none = psd.clone();
    none.merged = None;
    assert_eq!(import_fitted(&none, &sticker, &link, &cx(PS, &[])), Err(ImportProblem::NeedsMerged));
}
