// Tests of package P12 (STICKER_PLAN 7 "P12", 5.9, 13.9 "P12"; names `k12_*`): `replace_layer_pixels` (one
// `Edit::Pixels` entry with only the changed tiles, undo / redo), the PSD of a layer reads back to the same pixels, the
// source of an update (one layer, several layers, forcing, no merged picture), a size mismatch is refused and changes
// nothing, the state table of a layer link, the project (a project without layer links writes no new key and needs no
// newer reader; a layer link survives a save and a load with its archived file), and Paste as New Paint Layer at equal
// and at different sizes.

use std::path::{Path, PathBuf};

use super::fit_centred;
use crate::doc_links::LinkApplied;
use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::history::Edit;
use crate::ids::{LayerId, TextureSetId};
use crate::image_io::PngLevel;
use crate::link::paint::{layer_psd, normalise_in_place, normalised_crc};
use crate::link::{
    EDITOR_PHOTOCRAFT, EDITOR_PHOTOSHOP, FileStamp, LinkFile, LinkMode, LinkState, PAINT_UPDATE_LABEL, PaintProblem, RealFileIo,
    links_folder, paint_archive_entry, paint_link_file_name, paint_source, texts, write_link_file,
};
use crate::pixel::{CroppedBuf, PixelBuf};
use crate::progress::NoProgress;
use crate::project::save::testkit::{cube_doc, noise_texture};
use crate::project::{LinkFileState, SaveOptions, load, read_link_archive};
use crate::psd::testkit as kit;
use crate::psd::{PsdDoc, PsdLimits};
use crate::testutil::temp_dir;

const S0: TextureSetId = TextureSetId(0);

/// The cube document with a 200 x 130 texture (4 x 3 tiles of 64) and an empty history.
fn doc() -> Document {
    let mut doc = cube_doc(&noise_texture(200, 130, 4));
    doc.history.clear();
    doc
}

/// A new paint layer of set 0.
fn paint_layer(doc: &mut Document) -> LayerId {
    let id = doc.add_raster_layer(S0, Some("Dirt")).unwrap();
    doc.history.clear();
    id
}

fn dense(doc: &Document, id: LayerId) -> PixelBuf {
    doc.layer(id).unwrap().to_dense(200, 130)
}

/// A picture of the texture's size: transparent with a rectangle of `rgba` and RGB noise under alpha 0.
fn picture(rect: [u32; 4], rgba: [u8; 4]) -> PixelBuf {
    let mut p = PixelBuf::new(200, 130);
    for y in 0..130 {
        for x in 0..200 {
            let inside = x >= rect[0] && x < rect[2] && y >= rect[1] && y < rect[3];
            p.set(x, y, if inside { rgba } else { [x as u8, y as u8, 7, 0] });
        }
    }
    p
}

/// `p` cropped or padded to `w x h` (top-left kept).
fn resized(p: &PixelBuf, w: u32, h: u32) -> PixelBuf {
    let mut out = PixelBuf::new(w, h);
    for y in 0..h.min(p.height) {
        for x in 0..w.min(p.width) {
            out.set(x, y, p.get(x, y));
        }
    }
    out
}

fn normalised(p: &PixelBuf) -> PixelBuf {
    let mut p = p.clone();
    normalise_in_place(&mut p.data);
    p
}

fn read(bytes: &[u8]) -> PsdDoc {
    crate::psd::read(bytes, &PsdLimits::default()).unwrap()
}

fn options() -> SaveOptions {
    SaveOptions { png: PngLevel::Fast, write_flattened: true, keep_backup: false, now_unix: 7, app_version: "test".to_owned() }
}

fn save(doc: &mut Document, path: &Path) {
    let stats = crate::project::save(&doc.snapshot(), path, &options(), &NoProgress).unwrap();
    doc.set_png_reuse(stats.reuse);
    doc.mark_saved(doc.history_state());
    doc.mark_links_saved(doc.links_revision());
}

/// A linked layer whose file is written into the links folder of a saved project: (folder, project, document, layer,
/// file).
fn linked(name: &str, editor: &str) -> (PathBuf, PathBuf, Document, LayerId, PathBuf) {
    let dir = temp_dir(name);
    let project = dir.join("Car.wtep");
    let mut doc = doc();
    let id = paint_layer(&mut doc);
    doc.replace_layer_pixels(id, picture([10, 10, 60, 40], [200, 30, 30, 255]), "Paint").unwrap();
    doc.history.clear();
    save(&mut doc, &project);
    std::fs::create_dir_all(links_folder(&project)).unwrap();
    let file = LinkFile::InFolder(paint_link_file_name("Dirt", id));
    let path = file.path(Some(&project)).unwrap();
    let prepared = doc.prepare_layer_link_file(id, file, editor).unwrap();
    let written = write_link_file(&path, &prepared.bytes, &mut RealFileIo).unwrap();
    doc.finish_layer_link_write(prepared, written).unwrap();
    (dir, project, doc, id, path)
}

/// The file an editor saves: one layer `rgba` over the whole canvas, with a real merged picture.
fn one_layer_file(rgba: &PixelBuf) -> Vec<u8> {
    let mut f = kit::File::rgb(rgba.width, rgba.height).merged_rgba(&rgba.data).version_info(true);
    f.layers.push(kit::Layer::rgba("Dirt", [0, 0, rgba.height as i32, rgba.width as i32], &rgba.data).lyid(1));
    f.build()
}

fn stamp_of(bytes: &[u8]) -> FileStamp {
    FileStamp::of_bytes(bytes, None)
}

#[test]
fn k12_replace_layer_pixels_is_one_pixel_entry_with_the_changed_tiles_only() {
    let mut doc = doc();
    let id = paint_layer(&mut doc);
    let before = dense(&doc, id);
    // An unallocated layer and an all-transparent picture: nothing changes, nothing is allocated.
    assert!(!doc.replace_layer_pixels(id, PixelBuf::new(200, 130), PAINT_UPDATE_LABEL).unwrap());
    assert!(doc.layer(id).unwrap().raster().unwrap().data.is_empty());
    assert_eq!(doc.history.undo_len(), 0);
    // A rectangle inside the tile (1, 0); the RGB under alpha 0 is zero, so nothing else changes.
    let new = normalised(&picture([70, 5, 120, 50], [10, 200, 30, 255]));
    let v0 = doc.layer(id).unwrap().version();
    assert!(doc.replace_layer_pixels(id, new.clone(), PAINT_UPDATE_LABEL).unwrap());
    assert_eq!(doc.history.undo_len(), 1);
    assert_eq!(doc.undo_label(), Some(PAINT_UPDATE_LABEL));
    let Edit::Pixels { patch, layer, .. } = &doc.history.undo.last().unwrap().edit else { panic!("not a pixel entry") };
    assert_eq!(*layer, id);
    let tiles: Vec<(u16, u16)> = patch.tiles.iter().map(|t| (t.tx, t.ty)).collect();
    assert_eq!(tiles, [(1, 0)], "only the tile that changed");
    assert_ne!(doc.layer(id).unwrap().version(), v0, "a new version: caches and views update");
    assert_eq!(dense(&doc, id), new);
    // RGB noise under alpha 0 is a change of the stored pixels too (every tile).
    let noisy = picture([70, 5, 120, 50], [10, 200, 30, 255]);
    assert!(doc.replace_layer_pixels(id, noisy.clone(), PAINT_UPDATE_LABEL).unwrap());
    doc.undo().unwrap();
    // The same picture again: no entry.
    assert!(!doc.replace_layer_pixels(id, new.clone(), PAINT_UPDATE_LABEL).unwrap());
    assert_eq!(doc.history.undo_len(), 1);
    doc.undo().unwrap();
    assert_eq!(dense(&doc, id), before);
    doc.redo().unwrap();
    assert_eq!(dense(&doc, id), new);
    // Refusals change nothing.
    assert!(matches!(doc.replace_layer_pixels(id, PixelBuf::new(10, 10), "x"), Err(EditError::InvalidParam(_))));
    doc.set_layer_locked(id, true).unwrap();
    assert!(matches!(doc.replace_layer_pixels(id, PixelBuf::new(200, 130), "x"), Err(EditError::LayerLocked)));
    assert_eq!(dense(&doc, id), new);
}

#[test]
fn k12_the_psd_of_a_layer_reads_back_to_the_same_pixels() {
    let mut doc = doc();
    let id = paint_layer(&mut doc);
    // Never painted: one empty layer at the texture's size; it comes back as a transparent picture.
    let empty = doc.prepare_layer_link_file(id, LinkFile::InFolder("a.psd".into()), EDITOR_PHOTOCRAFT).unwrap();
    let read_empty = read(&empty.bytes);
    assert_eq!(read_empty.size, [200, 130]);
    assert_eq!(read_empty.layers.len(), 1);
    assert_eq!(paint_source(&read_empty, [200, 130], EDITOR_PHOTOCRAFT).unwrap().pixels, PixelBuf::new(200, 130));
    assert_eq!(empty.link.synced_pixels, normalised_crc(&PixelBuf::new(0, 0), 200, 130));
    // Painted with soft alpha and noise under alpha 0; the layer's own opacity is not sent.
    let mut p = picture([20, 30, 150, 100], [40, 80, 160, 255]);
    for x in 20..150 {
        p.set(x, 30, [40, 80, 160, (x % 256) as u8]);
    }
    doc.replace_layer_pixels(id, p.clone(), "Paint").unwrap();
    doc.set_layer_opacity(id, 0.5, Commit::Final).unwrap();
    let prepared = doc.prepare_layer_link_file(id, LinkFile::InFolder("a.psd".into()), EDITOR_PHOTOSHOP).unwrap();
    let psd = read(&prepared.bytes);
    assert_eq!(psd.layers.len(), 1);
    let l = &psd.layers[0];
    assert_eq!((l.name.as_str(), l.lyid, l.opacity, l.visible), ("Dirt", Some(1), 255, true), "opacity and blend stay in WTE");
    assert!(psd.has_real_merged);
    let back = paint_source(&psd, [200, 130], EDITOR_PHOTOSHOP).unwrap();
    assert!(!back.merged && !back.editor_layers);
    assert_eq!(back.pixels, normalised(&p), "the straight, normalised pixels");
    assert_eq!(prepared.link.synced_pixels, normalised_crc(&p, 200, 130));
    assert_eq!(prepared.link.editor, EDITOR_PHOTOSHOP);
    assert_eq!(prepared.crc32, crc32fast::hash(&prepared.bytes));
    // The merged picture holds the same pixels (the writer composites the one layer).
    let merged = psd.merged.as_ref().unwrap();
    for (a, b) in merged.data.as_chunks::<4>().0.iter().zip(normalised(&p).data.as_chunks::<4>().0) {
        assert_eq!(a[3], b[3]);
        if a[3] == 255 {
            assert_eq!(a, b);
        }
    }
    // A stamp layer cannot be linked.
    if let Some(s) = doc.set(S0).unwrap().layers.iter().find(|l| !l.is_raster()).map(|l| l.id) {
        assert!(matches!(doc.prepare_layer_link_file(s, LinkFile::InFolder("b.psd".into()), "photocraft"), Err(EditError::NotRaster)));
    }
    assert_eq!(layer_psd("x", &PixelBuf::new(0, 0), 3, 2).layers[0].pixels, CroppedBuf::empty());
}

#[test]
fn k12_one_layer_and_several_layers_choose_the_right_source() {
    let red = picture([10, 10, 60, 40], [220, 20, 20, 255]);
    let blue = picture([100, 50, 180, 120], [20, 20, 220, 255]);
    // One layer, no merged picture at all (latest PhotoCraft with MaxCompat off): the layer.
    let mut f = kit::File::rgb(200, 130).version_info(false);
    f.layers.push(kit::Layer::rgba("Dirt", [0, 0, 130, 200], &red.data).lyid(1));
    let got = paint_source(&read(&f.build()), [200, 130], EDITOR_PHOTOCRAFT).unwrap();
    assert!(!got.merged);
    assert_eq!(got.pixels, normalised(&red));
    // The one layer at 50 % opacity and with a mask that hides its left part: both applied to alpha.
    let mask: Vec<u8> = (0..200 * 130).map(|i| if i % 200 < 35 { 0 } else { 255 }).collect();
    let mut f = kit::File::rgb(200, 130).version_info(false);
    let mut layer = kit::Layer::rgba("Dirt", [0, 0, 130, 200], &red.data).lyid(1).mask([0, 0, 130, 200], &mask, 255, 0);
    layer.opacity = 128;
    f.layers.push(layer);
    let got = paint_source(&read(&f.build()), [200, 130], EDITOR_PHOTOCRAFT).unwrap();
    assert_eq!(got.pixels.get(20, 20), [0, 0, 0, 0], "masked out");
    assert_eq!(got.pixels.get(50, 20), [220, 20, 20, 128], "half opacity");
    // A hidden second layer does not count; a visible one does: the merged picture.
    let mut both = red.clone();
    for y in 0..130 {
        for x in 0..200 {
            if blue.get(x, y)[3] == 255 {
                both.set(x, y, blue.get(x, y));
            }
        }
    }
    let mut f = kit::File::rgb(200, 130).merged_rgba(&both.data).version_info(true);
    f.layers.push(kit::Layer::rgba("Dirt", [0, 0, 130, 200], &red.data).lyid(1));
    let mut hidden = kit::Layer::rgba("Blue", [0, 0, 130, 200], &blue.data).lyid(2);
    hidden.flags |= 0x02;
    f.layers.push(hidden);
    let got = paint_source(&read(&f.build()), [200, 130], EDITOR_PHOTOCRAFT).unwrap();
    assert!(!got.merged && got.editor_layers, "the hidden layer is the editor's");
    assert_eq!(got.pixels, normalised(&red));
    f.layers[1].flags &= !0x02;
    let got = paint_source(&read(&f.build()), [200, 130], EDITOR_PHOTOCRAFT).unwrap();
    assert!(got.merged && got.editor_layers);
    assert_eq!(got.pixels, normalised(&both));
    // Two visible layers without merged data: "needs a merged picture".
    let mut f = kit::File::rgb(200, 130).version_info(false);
    f.layers.push(kit::Layer::rgba("Dirt", [0, 0, 130, 200], &red.data).lyid(1));
    f.layers.push(kit::Layer::rgba("Blue", [0, 0, 130, 200], &blue.data).lyid(2));
    assert_eq!(paint_source(&read(&f.build()), [200, 130], EDITOR_PHOTOSHOP), Err(PaintProblem::NeedsMerged));
    // One layer with an effect (forcing): the merged picture.
    let mut f = kit::File::rgb(200, 130).merged_rgba(&both.data).version_info(true);
    f.layers.push(kit::Layer::rgba("Dirt", [0, 0, 130, 200], &red.data).lyid(1).block(b"lrFX", vec![0; 4]));
    let got = paint_source(&read(&f.build()), [200, 130], EDITOR_PHOTOSHOP).unwrap();
    assert!(got.merged && got.editor_layers);
    assert_eq!(got.pixels, normalised(&both));
    // A flat file (no layers): its merged picture.
    let flat = kit::File::rgb(200, 130).merged_rgba(&blue.data).version_info(true).build();
    let got = paint_source(&read(&flat), [200, 130], EDITOR_PHOTOCRAFT).unwrap();
    assert!(got.merged);
    assert_eq!(got.pixels, normalised(&blue));
}

#[test]
fn k12_a_size_mismatch_is_refused_with_the_message_and_changes_nothing() {
    let (dir, _project, mut doc, id, path) = linked("k12_size", EDITOR_PHOTOSHOP);
    let before = dense(&doc, id);
    let undo = doc.history.undo_len();
    let revision = doc.links_revision();
    let bytes = one_layer_file(&resized(&picture([0, 0, 20, 20], [1, 2, 3, 255]), 256, 256));
    std::fs::write(&path, &bytes).unwrap();
    let stamp = stamp_of(&bytes);
    let r = doc.apply_layer_link_update(id, &read(&bytes), stamp, "Photoshop", "Dirt_l.psd").unwrap();
    let want = LinkState::SizeMismatch { size: [256, 256], texture: [200, 130] };
    assert_eq!(r, LinkApplied::Problem(want.clone()));
    assert_eq!(
        want.badge("Photoshop", "Dirt", "Dirt_l.psd"),
        "Dirt is 256 x 256 in Photoshop; the texture is 200 x 130. Undo the size change in Photoshop."
    );
    assert_eq!(
        texts::size_mismatch("Dirt", [256, 256], [200, 130], "PhotoCraft"),
        "Dirt is 256 x 256 in PhotoCraft; the texture is 200 x 130. Undo the size change in PhotoCraft."
    );
    assert_eq!(dense(&doc, id), before);
    assert_eq!(doc.history.undo_len(), undo);
    assert_eq!(doc.links_revision(), revision, "a problem alone does not ask to save");
    assert_eq!(doc.layer_link_state(id, Some(&stamp)).unwrap(), want, "the badge keeps it while the file has this content");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k12_the_state_table_and_an_update_of_a_layer_link() {
    let (dir, _project, mut doc, id, path) = linked("k12_state", EDITOR_PHOTOCRAFT);
    let synced = doc.layer_link(id).unwrap().synced;
    assert_eq!(doc.layer_link_state(id, Some(&synced)).unwrap(), LinkState::Linked);
    assert_eq!(doc.layer_link_state(id, None).unwrap(), LinkState::Missing);
    // The editor saves a changed picture: "changed in <editor>", then applied as one undo step.
    let new = picture([30, 20, 90, 70], [10, 220, 40, 255]);
    let bytes = one_layer_file(&new);
    std::fs::write(&path, &bytes).unwrap();
    let stamp = stamp_of(&bytes);
    assert_eq!(doc.layer_link_state(id, Some(&stamp)).unwrap(), LinkState::ChangedInEditor);
    assert_eq!(LinkState::ChangedInEditor.badge("PhotoCraft", "Dirt", "f"), "changed in PhotoCraft");
    let undo = doc.history.undo_len();
    let r = doc.apply_layer_link_update(id, &read(&bytes), stamp, "PhotoCraft", "Dirt_l.psd").unwrap();
    assert!(matches!(r, LinkApplied::Updated { mode: LinkMode::Layers, .. }));
    assert_eq!(doc.history.undo_len(), undo + 1);
    assert_eq!(doc.undo_label(), Some(PAINT_UPDATE_LABEL));
    assert_eq!(dense(&doc, id), normalised(&new));
    assert_eq!(doc.layer_link_state(id, Some(&stamp)).unwrap(), LinkState::Linked);
    assert!(doc.is_modified(), "the link changed: the project asks to be saved");
    // Undo: the file did not change, the layer did -> "changed in WTE" (R6: nothing comes back by itself).
    doc.undo().unwrap();
    assert_eq!(doc.layer_link_state(id, Some(&stamp)).unwrap(), LinkState::ChangedInWte);
    assert!(doc.layer_link_changed_in_wte(id));
    // Both changed: conflict.
    let other = one_layer_file(&picture([0, 0, 10, 10], [1, 1, 1, 255]));
    assert_eq!(doc.layer_link_state(id, Some(&stamp_of(&other))).unwrap(), LinkState::Conflict);
    // Redo: in sync again.
    doc.redo().unwrap();
    assert_eq!(doc.layer_link_state(id, Some(&stamp)).unwrap(), LinkState::Linked);
    // Needs a merged picture.
    let mut f = kit::File::rgb(200, 130).version_info(false);
    f.layers.push(kit::Layer::rgba("A", [0, 0, 130, 200], &new.data).lyid(1));
    f.layers.push(kit::Layer::rgba("B", [0, 0, 130, 200], &new.data).lyid(2));
    let nm = f.build();
    let r = doc.apply_layer_link_update(id, &read(&nm), stamp_of(&nm), "PhotoCraft", "x").unwrap();
    assert_eq!(r, LinkApplied::Problem(LinkState::NeedsMerged));
    // A refused file (CMYK): its sentence on the badge.
    let mut cmyk = kit::File::rgb(200, 130);
    cmyk.mode = 4;
    cmyk.channels = 4;
    cmyk.merged.push(vec![0; 200 * 130]);
    let cmyk = cmyk.build();
    let err = crate::psd::read(&cmyk, &PsdLimits::default()).unwrap_err();
    let st = doc.layer_link_file_refused(id, &err, stamp_of(&cmyk), "Dirt_l1.psd").unwrap();
    let LinkState::Refused(sentence) = st else { panic!("not refused: {st:?}") };
    assert!(sentence.starts_with("Dirt_l1.psd is in CMYK"), "{sentence}");
    // Unlink: the pixels stay.
    let pixels = dense(&doc, id);
    doc.unlink_layer(id);
    assert!(doc.layer_link(id).is_none());
    assert_eq!(dense(&doc, id), pixels);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k12_a_project_without_layer_links_writes_no_new_key_and_a_layer_link_survives_a_save_and_a_load() {
    // Without layer links: no "layer_links" key and the reader version of a plain project.
    let dir = temp_dir("k12_project_plain");
    let plain = dir.join("Plain.wtep");
    let mut d = doc();
    let id = paint_layer(&mut d);
    d.replace_layer_pixels(id, picture([10, 10, 60, 40], [200, 30, 30, 255]), "Paint").unwrap();
    save(&mut d, &plain);
    let json = String::from_utf8(read_link_archive(&plain, "manifest.json").unwrap()).unwrap();
    assert!(!json.contains("layer_links"), "a project without layer links has no new key");
    assert_eq!(crate::project::read_manifest(&plain).unwrap().min_reader_version, 1);
    std::fs::remove_dir_all(&dir).unwrap();

    // With a layer link: the entry, the archived file, reader version 4, and the link back after a load.
    let (dir, project, mut doc, id, path) = linked("k12_project", EDITOR_PHOTOSHOP);
    let bytes = std::fs::read(&path).unwrap();
    let time = std::fs::metadata(&path).unwrap().modified().unwrap();
    save(&mut doc, &project);
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "a project save never writes a link file");
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), time);
    let manifest = crate::project::read_manifest(&project).unwrap();
    assert_eq!(manifest.min_reader_version, 4);
    assert!(manifest.links.is_empty());
    assert_eq!(manifest.layer_links.len(), 1);
    let entry = &manifest.layer_links[0];
    assert_eq!((entry.layer, entry.editor.as_str()), (id, EDITOR_PHOTOSHOP));
    assert_eq!(entry.archive.as_deref(), Some(paint_archive_entry(id).as_str()));
    assert_eq!(read_link_archive(&project, &paint_archive_entry(id)).unwrap(), bytes);
    let report = load(&project, &NoProgress).unwrap();
    assert_eq!(report.layer_links.len(), 1);
    assert_eq!(report.layer_links[0].state, LinkFileState::Present);
    assert_eq!(report.layer_links[0].path.as_deref(), Some(path.as_path()));
    let mut loaded = report.document;
    assert_eq!(loaded.layer_link(id), doc.layer_link(id));
    assert!(!loaded.is_modified());
    let synced = loaded.layer_link(id).unwrap().synced;
    assert_eq!(loaded.layer_link_state(id, Some(&synced)).unwrap(), LinkState::Linked, "the CRC holds across a load");
    // Changed while closed, and missing (restorable from the archive).
    std::fs::write(&path, one_layer_file(&picture([0, 0, 5, 5], [9, 9, 9, 255]))).unwrap();
    assert_eq!(load(&project, &NoProgress).unwrap().layer_links[0].state, LinkFileState::Changed);
    std::fs::remove_file(&path).unwrap();
    let report = load(&project, &NoProgress).unwrap();
    assert_eq!(report.layer_links[0].state, LinkFileState::Missing);
    assert!(report.layer_links[0].archive.is_some());
    // The link of a deleted layer is not saved.
    doc.remove_layer(id).unwrap();
    save(&mut doc, &project);
    assert!(crate::project::read_manifest(&project).unwrap().layer_links.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn k12_paste_as_new_paint_layer_at_equal_and_at_other_sizes() {
    let mut doc = doc();
    let same = picture([5, 5, 50, 50], [10, 20, 30, 255]);
    let id = doc.add_paint_layer_with_pixels(S0, "Pasted", fit_centred(&same, 200, 130), "Paste as New Paint Layer").unwrap();
    assert_eq!(doc.history.undo_len(), 1, "one undo step");
    assert_eq!(doc.undo_label(), Some("Paste as New Paint Layer"));
    assert_eq!(dense(&doc, id), normalised(&same), "the same size: as it is");
    assert_eq!(doc.set(S0).unwrap().active, Some(id));
    assert_eq!(doc.layer(id).unwrap().name, "Pasted");
    doc.undo().unwrap();
    assert!(doc.layer(id).is_none());
    doc.redo().unwrap();
    assert_eq!(dense(&doc, id), normalised(&same));
    // A 100 x 100 opaque picture into 200 x 130: scaled to 130 x 130, centred (35 px on either side).
    let square = PixelBuf::filled(100, 100, [0, 0, 255, 255]);
    let fitted = fit_centred(&square, 200, 130);
    assert_eq!(fitted.get(34, 65)[3], 0);
    assert_eq!(fitted.get(35, 0), [0, 0, 255, 255]);
    assert_eq!(fitted.get(164, 129), [0, 0, 255, 255]);
    assert_eq!(fitted.get(165, 65)[3], 0);
    // A wide 400 x 50 picture: 200 x 25, centred vertically (rows 52 to 76).
    let wide = fit_centred(&PixelBuf::filled(400, 50, [255, 0, 0, 255]), 200, 130);
    assert_eq!(wide.get(0, 51)[3], 0);
    assert_eq!(wide.get(0, 52), [255, 0, 0, 255]);
    assert_eq!(wide.get(199, 76), [255, 0, 0, 255]);
    assert_eq!(wide.get(0, 77)[3], 0);
    let id2 = doc.add_paint_layer_with_pixels(S0, "Pasted 2", fitted.clone(), "Paste as New Paint Layer").unwrap();
    assert_eq!(dense(&doc, id2), fitted);
    assert!(matches!(doc.add_paint_layer_with_pixels(S0, "x", square, "x"), Err(EditError::InvalidParam(_))));
}

/// Timing (8.4 has no own budget for a linked paint layer; the sticker update has 1.5 s from the settled file to the
/// model): at 4096 x 4096 the one-layer PSD made (`prepare_layer_link_file`: CRC + writer), read (on the watcher's
/// thread), and applied on the UI thread (`apply_layer_link_update`: `paint_source` + `replace_layer_pixels` + the new
/// CRC); and Paste as New Paint Layer's `fit_centred` of a 3000 x 2000 picture (on its worker).
#[test]
#[ignore = "perf: run with --release --ignored --nocapture"]
fn perf_k12_layer_link_4096() {
    let side = 4096;
    let mut doc = cube_doc(&noise_texture(side, side, 9));
    doc.history.clear();
    let id = doc.add_raster_layer(S0, Some("Dirt")).unwrap();
    let mut p = PixelBuf::new(side, side);
    for (i, px) in p.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        *px = [(i % 251) as u8, (i % 241) as u8, 90, if i % 7 == 0 { 0 } else { 255 }];
    }
    doc.replace_layer_pixels(id, p.clone(), "Paint").unwrap();
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let mut best = [f64::MAX; 4];
    for round in 0..3u8 {
        let t = std::time::Instant::now();
        let prepared = doc.prepare_layer_link_file(id, LinkFile::InFolder("a.psd".into()), EDITOR_PHOTOCRAFT).unwrap();
        best[0] = best[0].min(ms(t));
        doc.finish_layer_link_write(prepared.clone(), FileStamp::of_bytes(&prepared.bytes, None)).unwrap();
        // The editor's save: a band repainted.
        let mut q = p.clone();
        for y in 1000..1200 + u32::from(round) {
            for x in 0..side {
                q.set(x, y, [10, 200, 30, 255]);
            }
        }
        let bytes = crate::psd::write_sticker_to_vec(&layer_psd("Dirt", &q, side, side)).unwrap();
        let t = std::time::Instant::now();
        let psd = read(&bytes);
        best[1] = best[1].min(ms(t));
        let t = std::time::Instant::now();
        let r = doc.apply_layer_link_update(id, &psd, stamp_of(&bytes), "PhotoCraft", "a.psd").unwrap();
        best[2] = best[2].min(ms(t));
        assert!(matches!(r, LinkApplied::Updated { .. }));
        doc.undo().unwrap();
    }
    let big = PixelBuf::filled(3000, 2000, [1, 2, 3, 255]);
    let t = std::time::Instant::now();
    let fitted = fit_centred(&big, side, side);
    best[3] = ms(t);
    assert_eq!(fitted.width, side);
    println!(
        "perf_k12 (4096 x 4096): prepare the PSD {:.1} ms; read it {:.1} ms; apply an update (UI thread) {:.1} ms; fit_centred 3000 x 2000 {:.1} ms",
        best[0], best[1], best[2], best[3]
    );
}

#[test]
fn k12_the_badge_does_not_hash_the_texture_during_a_stroke() {
    let (dir, _project, mut doc, id, _path) = linked("k12_stroke", EDITOR_PHOTOCRAFT);
    let synced = doc.layer_link(id).unwrap().synced;
    assert_eq!(doc.layer_link_state(id, Some(&synced)).unwrap(), LinkState::Linked);
    // A 2D stroke on the layer: every dab is a new version; the state is "changed in WTE" without a CRC of the texture.
    doc.set_active_layer(S0, Some(id)).unwrap();
    let brush = crate::brush::BrushParams { radius: 6.0, color: [255, 0, 255, 255], ..crate::brush::BrushParams::default() };
    doc.stroke_begin(&brush).unwrap();
    doc.stroke_to_2d(S0, glam::Vec2::new(100.0, 60.0), 1.0);
    doc.stroke_to_2d(S0, glam::Vec2::new(120.0, 70.0), 1.0);
    assert!(doc.stroke_active());
    let before = doc.sticker_state.layer_crcs.get(&id).copied();
    assert_eq!(doc.layer_link_state(id, Some(&synced)).unwrap(), LinkState::ChangedInWte);
    assert_eq!(doc.sticker_state.layer_crcs.get(&id).copied(), before, "no CRC computed during the stroke");
    assert!(doc.stroke_end());
    assert_eq!(doc.layer_link_state(id, Some(&synced)).unwrap(), LinkState::ChangedInWte, "after the stroke: computed once");
    assert_eq!(doc.sticker_state.layer_crcs.get(&id).map(|(v, _)| *v), Some(doc.layer(id).unwrap().version()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// check-P12: an editor's save of the unchanged picture over a layer that keeps colours under alpha 0 (an eraser
/// leaves them; the link file holds normalised pixels) is no change: no undo step "Update Linked Layer", the same
/// layer version, the link in sync with the new file.
#[test]
fn check_p12_an_unchanged_picture_over_rgb_under_alpha_0_is_no_undo_step() {
    let (dir, _project, mut doc, id, _path) = linked("check_p12_same", EDITOR_PHOTOCRAFT);
    let stored = dense(&doc, id);
    assert!(stored.data.as_chunks::<4>().0.iter().any(|p| p[3] == 0 && p[..3] != [0, 0, 0]), "the fixture keeps RGB under alpha 0");
    let version = doc.layer(id).unwrap().version();
    let undo = doc.history.undo_len();
    // The editor re-saves the same picture (its own bytes: another file content).
    let bytes = one_layer_file(&normalised(&stored));
    let stamp = stamp_of(&bytes);
    assert_ne!(stamp.crc32, doc.layer_link(id).unwrap().synced.crc32);
    let r = doc.apply_layer_link_update(id, &read(&bytes), stamp, "PhotoCraft", "Dirt_l.psd").unwrap();
    assert!(matches!(r, LinkApplied::Updated { .. }));
    assert_eq!(doc.history.undo_len(), undo, "no undo step for an invisible difference");
    assert_eq!(doc.layer(id).unwrap().version(), version, "the layer is untouched");
    assert_eq!(dense(&doc, id), stored);
    assert_eq!(doc.layer_link_state(id, Some(&stamp)).unwrap(), LinkState::Linked);
    // A one-pixel change is still applied.
    let mut changed = normalised(&stored);
    changed.set(0, 0, [1, 2, 3, 255]);
    let bytes = one_layer_file(&changed);
    doc.apply_layer_link_update(id, &read(&bytes), stamp_of(&bytes), "PhotoCraft", "Dirt_l.psd").unwrap();
    assert_eq!(doc.history.undo_len(), undo + 1);
    assert_eq!(dense(&doc, id), changed);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// check-P12 (5.9): WTE's largest texture (8192 a side, `MAX_TEXTURE_SIDE`) links, and its file reads back within the
/// reader's limits and applies (the "too large to link" branch guards textures WTE cannot make).
#[test]
fn check_p12_the_largest_texture_side_links_and_reads_back() {
    assert_eq!(crate::doc_build::MAX_TEXTURE_SIDE, crate::link::MAX_PAINT_LINK_SIDE);
    let mut doc = cube_doc(&noise_texture(8193, 2, 3));
    assert_eq!(doc.set(S0).map(|s| [s.width, s.height]), Some([8192, 2]), "textures are capped at 8192");
    let id = doc.add_raster_layer(S0, Some("Wide")).unwrap();
    let prepared = doc.prepare_layer_link_file(id, LinkFile::InFolder("w.psd".into()), EDITOR_PHOTOCRAFT).unwrap();
    let psd = read(&prepared.bytes);
    assert!(!psd.too_large);
    let back = paint_source(&psd, [8192, 2], EDITOR_PHOTOCRAFT).unwrap();
    assert_eq!(back.pixels, PixelBuf::new(8192, 2));
}
