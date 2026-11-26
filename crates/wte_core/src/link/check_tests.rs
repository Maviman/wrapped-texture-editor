// Checker tests of package P5c (check-P5c.md): "Update now" after an undo of "Update Linked Sticker" and "Take the
// editor's version" of a conflict must give the file's content, never a "linked" badge over a sticker the file
// does not show; plus the checker's own renumbering pattern and mapping rows derived by hand from fixture bytes.

use glam::Vec2;

use super::map::LayerOutcome;
use super::tests::*;
use super::*;
use crate::doc_stickers::StickerRef;
use crate::document::Commit;
use crate::sticker::{PicturePixels, StickerContent};

#[test]
fn k5_link_check_update_now_after_an_undo_gives_the_files_content_again() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let before = (**doc.sticker(sid).unwrap()).clone();
    // In the editor: the photo repainted, the shape moved, the text untouched.
    let mut psd = read(&bytes);
    layer(&mut psd, "photo").pixels.as_mut().unwrap().data[..4].copy_from_slice(&[1, 2, 3, 255]);
    let l = layer(&mut psd, "shape");
    let p = l.pixels.as_ref().unwrap();
    let r = p.rect;
    l.pixels = Some(crate::pixel::CroppedBuf { rect: crate::math::IRect::new(r.x0 + 3, r.y0 - 2, r.x1 + 3, r.y1 - 2), data: p.data.clone() });
    l.rect = l.pixels.as_ref().unwrap().rect;
    let stamp = other(11);
    doc.apply_link_update(sid, &psd, stamp, "Photoshop", "Logo_k1.psd").unwrap();
    let updated = (**doc.sticker(sid).unwrap()).clone();
    assert_ne!(updated, before);
    assert!(matches!(by_name(&updated, "photo").content, StickerContent::Picture(PicturePixels::Owned { .. })));
    // Undo: the WTE layers are back, "changed in WTE".
    doc.undo();
    assert_eq!(**doc.sticker(sid).unwrap(), before);
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::ChangedInWte);
    // "Update now" applies the file again: the sticker is the updated one, not the WTE one under a "linked" badge.
    doc.apply_link_update(sid, &psd, stamp, "Photoshop", "Logo_k1.psd").unwrap();
    let again = (**doc.sticker(sid).unwrap()).clone();
    assert_eq!(names(&again), names(&updated));
    for (a, b) in again.layers.iter().zip(&updated.layers) {
        assert_eq!((a.id, &a.transform, a.mask.is_some()), (b.id, &b.transform, b.mask.is_some()), "{}", a.name);
        match (&a.content, &b.content) {
            (StickerContent::Picture(PicturePixels::Owned { pixels: pa, .. }), StickerContent::Picture(PicturePixels::Owned { pixels: pb, .. })) => {
                assert_eq!(pa, pb, "{}", a.name)
            }
            (ca, cb) => assert_eq!(ca, cb, "{}", a.name),
        }
    }
    assert!(matches!(by_name(&again, "text").content, StickerContent::Text(_)), "the untouched text stays text");
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::Linked);
}

#[test]
fn k5_link_check_take_the_editors_version_of_a_conflict_drops_wtes_layer_changes() {
    let (mut doc, sid, _) = linked_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let before = (**doc.sticker(sid).unwrap()).clone();
    // In WTE since the sync: the shape moved and the text's words changed (layers the editor did not touch).
    doc.set_sticker(
        StickerRef::Id(sid),
        |d| {
            d.layers[2].transform.translate += Vec2::new(5.0, 0.0);
            if let StickerContent::Text(t) = &mut d.layers[1].content {
                t.content = "ABC".into();
            }
        },
        "Edit",
        Commit::Final,
    )
    .unwrap();
    // In the editor: only the photo repainted.
    let mut psd = read(&bytes);
    layer(&mut psd, "photo").pixels.as_mut().unwrap().data[..4].copy_from_slice(&[1, 2, 3, 255]);
    let stamp = other(12);
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::Conflict);
    // "Take <editor>'s version": the file is applied; the sticker shows the file, then "linked".
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert!(made.outcomes.contains(&LayerOutcome::PixelsChanged(by_name(&before, "photo").id)));
    doc.apply_link_update(sid, &psd, stamp, "Photoshop", "Logo_k1.psd").unwrap();
    let now = (**doc.sticker(sid).unwrap()).clone();
    assert_eq!(doc.link_state(sid, Some(&stamp)).unwrap(), LinkState::Linked);
    assert_eq!(by_name(&now, "shape").transform, by_name(&before, "shape").transform, "the shape is where the file has it");
    match &by_name(&now, "text").content {
        StickerContent::Text(t) => assert_eq!(t.content, "WTE", "the text says what the file shows"),
        StickerContent::Picture(_) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn k5_link_check_the_sync_info_survives_save_and_load() {
    let (dir, project, mut doc, sid, bytes) = saved_link_project("k5_link_check_wte");
    let opts = crate::project::SaveOptions {
        png: crate::image_io::PngLevel::Fast,
        write_flattened: false,
        keep_backup: false,
        now_unix: 7,
        app_version: "test".to_owned(),
    };
    let stats = crate::project::save(&doc.snapshot(), &project, &opts, &crate::progress::NoProgress).unwrap();
    doc.set_png_reuse(stats.reuse);
    let mut loaded = crate::project::load(&project, &crate::progress::NoProgress).unwrap().document;
    let link = loaded.link(sid).unwrap().clone();
    assert!(link.layer_map.iter().all(|e| e.wte.is_some()), "saved in the manifest");
    let sticker = (**loaded.sticker(sid).unwrap()).clone();
    for e in &link.layer_map {
        let l = sticker.layer(e.layer).unwrap();
        assert_eq!(loaded.link_layer_hash(l), e.wte.unwrap().hash, "{}: the layer hash is the same after a load", l.name);
    }
    // The file applied after the load: every layer untouched, the text still text.
    let psd = read(&bytes);
    loaded.apply_link_update(sid, &psd, other(21), "Photoshop", "Logo_k1.psd").unwrap();
    assert_eq!(**loaded.sticker(sid).unwrap(), sticker);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k5_link_check_a_refused_file_does_not_mark_the_project_modified() {
    let (mut doc, sid, _) = linked_doc();
    make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    doc.mark_links_saved(doc.links_revision());
    doc.mark_saved(doc.history_state());
    assert!(!doc.is_modified());
    let err = crate::psd::PsdError::Unsupported(crate::psd::PsdRefusal::Depth32);
    let state = doc.link_file_refused(sid, &err, other(30), "Logo_k1.psd").unwrap();
    assert!(matches!(state, LinkState::Refused(_)), "{state:?}");
    assert!(!doc.is_modified(), "a problem is not saved: no unsaved-changes prompt for it");
    // The badge still shows it.
    assert!(matches!(doc.link_state(sid, Some(&other(30))).unwrap(), LinkState::Refused(_)));
}

#[test]
fn k5_link_check_renumbered_after_a_delete_or_a_reorder_in_the_editor() {
    // The checker's own patterns (13.9 "P5c" checker): map ids 1, 2, 4, 5 (photo, text, masked, added).
    let (doc, sid, bytes) = renumber_doc();
    let before = (**doc.sticker(sid).unwrap()).clone();
    // 1. "photo" deleted in the editor, then every layer renumbered 1..3 by position.
    let mut psd = read(&bytes);
    psd.layers.retain(|l| l.name != "photo");
    for (i, l) in psd.layers.iter_mut().enumerate() {
        l.lyid = Some(i as u32 + 1);
    }
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.ids_lost, Some("the ids were renumbered"));
    assert_eq!(names(&made.sticker), ["text", "masked", "added"]);
    assert_eq!(made.outcomes.iter().filter(|o| matches!(o, LayerOutcome::Untouched(_))).count(), 3, "{:?}", made.outcomes);
    assert!(made.outcomes.contains(&LayerOutcome::Deleted(by_name(&before, "photo").id)));
    assert!(matches!(by_name(&made.sticker, "text").content, StickerContent::Text(_)), "a text stays text");
    // 2. "text" and "masked" swapped in the editor, then renumbered 4..1 from the bottom up.
    let mut psd = read(&bytes);
    psd.layers.swap(1, 2);
    let n = psd.layers.len() as u32;
    for (i, l) in psd.layers.iter_mut().enumerate() {
        l.lyid = Some(n - i as u32);
    }
    let made = import_now(&doc, sid, &psd, PS).unwrap();
    assert_eq!(made.ids_lost, Some("the ids were renumbered"));
    assert!(made.outcomes.iter().all(|o| matches!(o, LayerOutcome::Untouched(_))), "{:?}", made.outcomes);
    assert_eq!(names(&made.sticker), ["photo", "masked", "text", "added"]);
    for l in &made.sticker.layers {
        assert_eq!(l, by_name(&before, &l.name), "{} is the same layer", l.name);
    }
}

// ----- three rows of the mapping table re-derived by hand from the bytes of f15_link_pc.psd -----

fn f15_pc() -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/psd/f15_link_pc.psd")).unwrap()
}

#[test]
fn k5_link_check_rows_derived_by_hand_from_f15_bytes() {
    // Values from a hand parse of f15_link_pc.psd (a small Python walker independent of WTE's reader, check-P5c.md):
    // per layer the RLE channels decoded, RGB zeroed under alpha 0, trimmed to alpha > 0, then the CRC-32 of the
    // RGBA bytes. WTE's map must hold exactly these (row 1 "untouched" compares with them).
    /// lyid, name, normalised origin, size, CRC-32.
    type Hand = (u32, &'static str, [i32; 2], [u32; 2], u32);
    const HAND: [Hand; 4] = [
        (1, "photo", [3, 5], [28, 28], 0x6466_8350),
        (2, "text", [25, 7], [37, 14], 0x6506_8780),
        (3, "shape", [38, 28], [18, 14], 0xeffe_4c02),
        (4, "masked", [4, 32], [20, 12], 0x5b45_600e),
    ];
    let (mut doc, sid, _) = linked_doc();
    make_link(&mut doc, sid, EDITOR_PHOTOCRAFT);
    let before = (**doc.sticker(sid).unwrap()).clone();
    let map = doc.link(sid).unwrap().layer_map.clone();
    for (e, (lyid, name, origin, size, crc)) in map.iter().zip(HAND) {
        assert_eq!((e.lyid, e.name.as_str(), e.origin, e.size, e.pixels_crc), (lyid, name, origin, size, crc));
    }
    let bytes = f15_pc();
    let at = |o: usize, want: &[u8]| assert_eq!(&bytes[o..o + want.len()], want, "offset {o}");
    // Offsets from the hand parse: the shape's record (its rect) at 550; "8BIM" + blend, then opacity, clipping and
    // flags after the channel list of photo (264), text (430) and masked (764); masked's mask flags at 801.
    at(264, b"8BIMnorm");
    at(430, b"8BIMnorm");
    at(550, &[0, 0, 0, 28, 0, 0, 0, 38, 0, 0, 0, 42, 0, 0, 0, 56]);
    at(764, b"8BIMnorm");
    at(780, &[0, 0, 0, 20]);
    let apply = |patch: &dyn Fn(&mut Vec<u8>)| {
        let mut b = bytes.clone();
        patch(&mut b);
        import_now(&doc, sid, &read(&b), PC).unwrap()
    };
    let id = |n: &str| by_name(&before, n).id;

    // Row 2 "same bytes, another origin": the shape's rect moved by (+3, -2) in the record -> kept as a shape, moved.
    let made = apply(&|b: &mut Vec<u8>| {
        for (k, v) in [26i32, 41, 40, 59].into_iter().enumerate() {
            b[550 + 4 * k..554 + 4 * k].copy_from_slice(&v.to_be_bytes());
        }
    });
    assert_eq!(made.outcomes[2], LayerOutcome::Moved(id("shape")));
    let shape = by_name(&made.sticker, "shape");
    assert!(matches!(shape.content, StickerContent::Shape(_)));
    assert_eq!(shape.transform.translate, Vec2::new(41.0, 26.0), "38 + 3, 28 - 2");
    assert!(made.outcomes.iter().enumerate().all(|(i, o)| i == 2 || matches!(o, LayerOutcome::Untouched(_))));

    // Row 1 "untouched; name, visibility, opacity, blend and clip from the PSD": photo hidden, Multiply, opacity
    // 128; text clipped.
    let made = apply(&|b: &mut Vec<u8>| {
        b[268..272].copy_from_slice(b"mul ");
        b[272] = 128;
        b[274] |= 0x02;
        b[439] = 1;
    });
    assert!(made.outcomes.iter().all(|o| matches!(o, LayerOutcome::Untouched(_))), "{:?}", made.outcomes);
    let photo = by_name(&made.sticker, "photo");
    assert_eq!((photo.visible, photo.blend), (false, crate::layer::BlendMode::Multiply));
    assert!((photo.opacity - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(photo.content, by_name(&before, "photo").content);
    let text = by_name(&made.sticker, "text");
    assert!(text.clip && matches!(text.content, StickerContent::Text(_)));

    // Row "a mask marked disabled": masked's mask flags bit 1 -> the whole-pixel picture at scale 1 keeps its pixels
    // and has no mask; the map remembers the disabled mask's CRC so the next read is untouched.
    let disabled = |b: &mut Vec<u8>| b[801] |= 0x02;
    let made = apply(&disabled);
    assert_eq!(made.outcomes[3], LayerOutcome::MaskChanged { layer: id("masked"), kept: true });
    let masked = by_name(&made.sticker, "masked");
    assert!(masked.mask.is_none());
    assert_eq!(masked.content, by_name(&before, "masked").content);
    let mut b = bytes.clone();
    disabled(&mut b);
    let psd = read(&b);
    doc.apply_link_update(sid, &psd, FileStamp::of_bytes(&b, None), "PhotoCraft", "f15.psd").unwrap();
    let again = import_now(&doc, sid, &psd, PC).unwrap();
    assert!(again.outcomes.iter().all(|o| matches!(o, LayerOutcome::Untouched(_))), "{:?}", again.outcomes);
}

#[cfg(windows)]
#[test]
fn k5_link_check_a_file_held_without_sharing_gives_could_not_be_replaced() {
    // A real lock in a temp folder (no other program): the PSD opened with share mode 0, as an editor might hold it.
    use std::os::windows::fs::OpenOptionsExt;
    struct NoPause;
    impl LinkFileIo for NoPause {
        fn pause(&mut self, _ms: u64) {}
    }
    let dir = crate::testutil::temp_dir("k5_link_check_lock");
    let target = dir.join("Logo_k1.psd");
    std::fs::write(&target, b"old").unwrap();
    let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&target).unwrap();
    let err = write_link_file(&target, b"new", &mut NoPause).unwrap_err();
    assert_eq!(err, LinkWriteError::Locked { file: "Logo_k1.psd".into() });
    drop(held);
    assert_eq!(std::fs::read(&target).unwrap(), b"old", "nothing changed");
    let left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, ["Logo_k1.psd"], "no temp file, no backup");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k5_link_check_a_changed_layer_at_scale_two_is_sent_back_with_the_same_pixels() {
    // A layer that came back as a picture at transform scale 1/2 is written again pixel for pixel (no resampling), so
    // a Send after an update does not soften the editor's work.
    let (mut doc, sid) = scaled_doc();
    let bytes = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let mut psd = read(&bytes);
    layer(&mut psd, "text").pixels.as_mut().unwrap().data[3] ^= 0x40;
    doc.apply_link_update(sid, &psd, other(40), "Photoshop", "Logo_k1.psd").unwrap();
    let sent = make_link(&mut doc, sid, EDITOR_PHOTOSHOP);
    let again = read(&sent);
    let want = crate::psd::write::normalise(layer(&mut psd, "text").pixels.as_ref().unwrap());
    let got = layer(&mut again.clone(), "text").pixels.clone().unwrap();
    assert_eq!((got.rect, got.data), (want.rect, want.data));
}
