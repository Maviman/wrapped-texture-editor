//! Tests of `dnd.rs`: egui's file list (as before P10a), and (stickers P10a) the OLE drops at a point: the overlay from
//! both sources, the spot under the point, and the table of STICKER_PLAN 6.2 row by row through `prepare` + `apply`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::state::{Prefs, ViewMode};
use crate::tools::test_support::{at, state_with, xf_2d, xf_3d};
use wte_core::{Placement, SampleKind, StickerContent, StickerLayer};

#[derive(Debug)]
struct FakeDrop(PathBuf);
impl egui::DroppedFile for FakeDrop {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("not read".into())
    }
}

#[test]
fn dropped_files_become_open_path_actions() {
    let ctx = egui::Context::default();
    let mut state = AppState::new(Prefs::default(), true);
    let files: Vec<egui::DroppedFileHandle> = vec![
        Arc::new(FakeDrop(PathBuf::from(r"E:\m\a.OBJ"))),
        Arc::new(FakeDrop(PathBuf::from(r"E:\m\b.wtep"))),
        Arc::new(FakeDrop(PathBuf::from(r"E:\m\logo.png"))),
    ];
    let input = egui::RawInput { dropped_files: files, ..Default::default() };
    crate::theme::run_test_pass(&ctx, input, |ui| handle_events(ui.ctx(), &mut state, Vec::new()));
    let mut toasts = Vec::new();
    let paths: Vec<_> = state
        .actions
        .iter()
        .filter_map(|a| match a {
            AppAction::OpenPath(p) => Some(p.clone()),
            AppAction::Toast(ToastLevel::Info, text) => {
                toasts.push(text.clone());
                None
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    // The project wins over the model; the image is not a stamp when a document comes with the drop.
    assert_eq!(paths, [PathBuf::from(r"E:\m\b.wtep")]);
    assert_eq!(toasts.len(), 1, "one toast names the files not opened");
    assert!(toasts[0].contains("a.OBJ") && toasts[0].contains("logo.png"), "{toasts:?}");
}

#[test]
fn a_model_dropped_with_its_material_and_texture_opens_the_model() {
    let p = |s: &str| PathBuf::from(s);
    // Explorer lists the grabbed file first: here the .mtl.
    let plan = plan_drop(&[p(r"E:\m\model.mtl"), p(r"E:\m\model.obj"), p(r"E:\m\texture.png")]);
    assert_eq!(plan.document, Some(p(r"E:\m\model.obj")));
    assert!(plan.stamps.is_empty(), "the model's texture does not become a stamp");
    assert_eq!(plan.ignored, [p(r"E:\m\model.mtl"), p(r"E:\m\texture.png")]);
    // Only images: stamp sources, in order; a folder or unknown file is ignored.
    let plan = plan_drop(&[p(r"E:\img\folder"), p(r"E:\img\a.png"), p(r"E:\img\b.JPG"), p(r"E:\img\notes.txt")]);
    assert_eq!(plan.document, None);
    assert_eq!(plan.stamps, [p(r"E:\img\a.png"), p(r"E:\img\b.JPG")]);
    assert_eq!(plan.ignored, [p(r"E:\img\folder"), p(r"E:\img\notes.txt")]);
    // Two models: the first one; a glTF's .bin is ignored.
    let plan = plan_drop(&[p("x.bin"), p("x.gltf"), p("y.glb")]);
    assert_eq!(plan.document, Some(p("x.gltf")));
    assert_eq!(plan.ignored, [p("x.bin"), p("y.glb")]);
    assert_eq!(plan_drop(&[]), DropPlan { reason: "not a model, project, image, PSD or sticker file", ..DropPlan::default() });
    // Stickers P9a (5.8): a PSD file without a model or project is opened as a sticker, in order with the images;
    // P10a: a sticker file too.
    let plan = plan_drop(&[p(r"E:\img.png"), p(r"E:\img\Logo.PSD"), p(r"E:\imgig.psb"), p(r"E:\s\Badge.wtes")]);
    assert_eq!(plan.stamps, [p(r"E:\img.png"), p(r"E:\img\Logo.PSD"), p(r"E:\imgig.psb"), p(r"E:\s\Badge.wtes")]);
    let plan = plan_drop(&[p(r"E:\m\model.obj"), p(r"E:\img\Logo.psd")]);
    assert_eq!(plan.ignored, [p(r"E:\img\Logo.psd")]);
    assert!(is_psd_path(Path::new("x.PSD")) && is_psd_path(Path::new("x.psb")) && !is_psd_path(Path::new("x.png")));
    assert!(is_wtes_path(Path::new("x.WTES")) && !is_wtes_path(Path::new("x.wtep")));
}

#[test]
fn extensions() {
    assert!(is_image_path(Path::new("x.JPG")));
    assert!(!is_image_path(Path::new("x.obj")));
    assert_eq!(extension_of(Path::new("dir.v2/model")), "");
    assert_eq!(extension_of(Path::new("a.GLB")), "glb");
    // FBX models open like the other formats (drop, file dialog filter, OpenPath).
    assert!(MODEL_EXTENSIONS.contains(&extension_of(Path::new(r"E:\m\Spider.FBX")).as_str()));
    let plan = plan_drop(&[PathBuf::from(r"E:\m\tex.png"), PathBuf::from(r"E:\m\Spider.FBX")]);
    assert_eq!(plan.document, Some(PathBuf::from(r"E:\m\Spider.FBX")));
}

// ---------------------------------------------------------------------------------------------------------------------
// Stickers P10a
// ---------------------------------------------------------------------------------------------------------------------

fn png_bytes(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut p = wte_core::PixelBuf::new(w, h);
    for px in p.data.as_chunks_mut::<4>().0 {
        px.copy_from_slice(&rgba);
    }
    wte_core::encode_png(w, h, &p.data, wte_core::PngLevel::Fast).unwrap()
}

fn png_item(name: &str) -> DropItem {
    DropItem::Data { name: name.to_owned(), kind: DataKind::Png, bytes: png_bytes(40, 20, [200, 30, 30, 255]) }
}

/// The bytes of a `.wtes` with one 16 x 16 green picture layer named "Badge".
fn wtes_bytes() -> Vec<u8> {
    let mut p = wte_core::PixelBuf::new(16, 16);
    for px in p.data.as_chunks_mut::<4>().0 {
        px.copy_from_slice(&[20, 180, 40, 255]);
    }
    let pixels = wte_core::PicturePixels::Owned { pixels: Arc::new(p), version: 0 };
    let doc = StickerDoc::from_picture("Badge", pixels, [16, 16]);
    wte_core::write_wtes(&doc, None, &wte_core::BlobStore::default(), None).unwrap()
}

/// `prepare` + `apply` of `items` at `spot`.
fn drop_at(state: &mut AppState, spot: Spot, items: Vec<DropItem>) {
    let prepared = prepare(spot, items);
    assert!(prepared.failed.is_empty(), "{:?}", prepared.failed);
    apply(state, &prepared);
}

fn undo_len(state: &AppState) -> usize {
    state.doc().unwrap().history.undo_len()
}

/// The overlay is the same whether the hover comes from egui's file list or from the drop target's events; a
/// project or model is "Drop to open", pictures, stickers and PSDs are placed; a modal job blocks.
#[test]
fn the_overlay_is_the_same_from_both_sources() {
    let cases: [(&[&str], Overlay); 5] = [
        (&[r"C:\p\a.wtep"], Overlay::Open),
        (&[r"C:\p\model.glb", r"C:\p\t.png"], Overlay::Open),
        (&[r"C:\p\a.png", r"C:\p\b.psd"], Overlay::Place),
        (&[r"C:\p\badge.WTES"], Overlay::Place),
        (&[r"C:\p\notes.txt"], Overlay::Open),
    ];
    for (paths, want) in cases {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        // egui's list.
        let ctx = egui::Context::default();
        let mut state = AppState::new(Prefs::default(), true);
        let hovered = paths.iter().map(|p| egui::HoveredFile { path: Some(p.clone()), ..Default::default() }).collect();
        let input = egui::RawInput { hovered_files: hovered, ..Default::default() };
        crate::theme::run_test_pass(&ctx, input, |ui| handle_events(ui.ctx(), &mut state, Vec::new()));
        let from_egui = state.drop_ui.hover.clone().expect("hover from egui");
        assert_eq!(from_egui.pos, None);
        // The drop target's events.
        let mut state2 = AppState::new(Prefs::default(), true);
        let items: Vec<DropItem> = paths.iter().cloned().map(DropItem::File).collect();
        let events = vec![OleEvent::Hover { pos: [100.0, 50.0], kinds: crate::ole::kinds_of(&items) }];
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| handle_events(ui.ctx(), &mut state2, events.clone()));
        let from_ole = state2.drop_ui.hover.clone().expect("hover from the drop target");
        assert_eq!(from_ole.kinds, from_egui.kinds);
        assert!(from_ole.pos.is_some());
        assert_eq!(overlay_of(&from_egui.kinds, false), want, "{paths:?}");
        assert_eq!(overlay_of(&from_ole.kinds, false), want);
        assert_eq!(overlay_of(&from_ole.kinds, true), Overlay::Blocked);
        // Leave clears the hover; so does an empty egui list.
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| handle_events(ui.ctx(), &mut state2, vec![OleEvent::Leave]));
        assert!(state2.drop_ui.hover.is_none());
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| handle_events(ui.ctx(), &mut state, Vec::new()));
        assert!(state.drop_ui.hover.is_none());
    }
    let data = DragKinds { picture: true, ..DragKinds::default() };
    assert_eq!(overlay_of(&data, false), Overlay::Place);
}

/// The spot under a point: a stamp's row, the Sticker tab's thumbnail, the views, a dialog in front.
#[test]
fn the_spot_under_the_point() {
    let mut state = state_with(SampleKind::Sphere);
    let xf3 = xf_3d(&mut state);
    let xf2 = xf_2d(&mut state);
    let centre = xf3.rect.center();
    assert_eq!(spot_at(&state, centre), Spot::View(ViewId::Main3D, centre));
    assert_eq!(spot_at(&state, xf2.rect.center()), Spot::View(ViewId::Main2D, xf2.rect.center()));
    assert_eq!(spot_at(&state, egui::pos2(-10.0, -10.0)), Spot::Elsewhere);
    // A stamp's row.
    let prepared = prepare(Spot::View(ViewId::Main3D, centre), vec![png_item("logo.png")]);
    apply(&mut state, &prepared);
    let stamp = state.session.as_ref().unwrap().selected_stamp.expect("a stamp");
    let layer = state.doc().unwrap().stamp_layers(stamp)[0].1;
    let row = egui::Rect::from_min_size(egui::pos2(1500.0, 300.0), egui::vec2(280.0, 30.0));
    state.panel_rects.insert(PanelHit::LayerRow(layer), row);
    assert_eq!(spot_at(&state, row.center()), Spot::Stamp(stamp));
    // A row scrolled out of the list's visible part is not a target.
    state.panel_rects.insert(PanelHit::LayersList, egui::Rect::from_min_size(egui::pos2(1500.0, 400.0), egui::vec2(280.0, 200.0)));
    assert_eq!(spot_at(&state, row.center()), Spot::Elsewhere);
    // The Sticker tab's thumbnail: the selected stamp.
    let thumb = egui::Rect::from_min_size(egui::pos2(1500.0, 100.0), egui::vec2(64.0, 64.0));
    state.panel_rects.insert(PanelHit::StickerTabThumb, thumb);
    assert_eq!(spot_at(&state, thumb.center()), Spot::Stamp(stamp));
    // The Preview view and an open dialog are no targets.
    state.view_mode = ViewMode::Preview;
    assert_eq!(spot_at(&state, centre), Spot::Elsewhere);
    state.view_mode = ViewMode::Split;
    state.dialog = Dialog::Message { title: "t".into(), lines: Vec::new() };
    assert_eq!(spot_at(&state, centre), Spot::Elsewhere);
}

/// 6.2 row "3D view, the ray hits the model": a picture becomes a stamp centred at the hit (the Stamp tool's click,
/// its size, one undo step), selected, with the Transform tool; a `.wtes` the same as a sticker stamp; a miss is
/// "anywhere else": the picture is the Stamp tool's picture and nothing is placed.
#[test]
fn a_drop_on_the_model_places_a_stamp_at_the_point() {
    let mut state = state_with(SampleKind::Sphere);
    let xf = xf_3d(&mut state);
    let pos = at(&xf, 0.55, 0.45);
    let hit = match surface_of(&state, ViewId::Main3D, pos) {
        Some((_, Surface::Model(hit))) => hit,
        other => panic!("no hit: {:?}", other.map(|o| o.1)),
    };
    drop_at(&mut state, Spot::View(ViewId::Main3D, pos), vec![png_item("Dropped picture.png")]);
    let session = state.session.as_ref().unwrap();
    let id = session.selected_stamp.expect("the new stamp is selected");
    let stamp = session.doc.stamp(id).unwrap();
    let Placement::Projected(pr) = stamp.placement else { panic!("projected") };
    assert!((pr.origin - hit.position).length() < 1e-4, "centred at the drop point: {:?} vs {:?}", pr.origin, hit.position);
    let diag = session.doc.mesh.bounds.diagonal();
    assert!((pr.half_size.max_element() - state.tools.stamp.size_frac * diag / 2.0).abs() < 1e-4, "the Stamp tool's size");
    assert_eq!(stamp.name, "Dropped picture");
    assert_eq!(state.tool, ToolKind::Transform);
    assert_eq!(undo_len(&state), 1, "one undo step");
    // A sticker file: a tray sticker shown by a new stamp, one undo step for both.
    let sticker = DropItem::Data { name: "Sticker.wtes".into(), kind: DataKind::Sticker, bytes: wtes_bytes() };
    drop_at(&mut state, Spot::View(ViewId::Main3D, pos), vec![sticker]);
    let id2 = state.session.as_ref().unwrap().selected_stamp.unwrap();
    assert_ne!(id2, id);
    assert!(matches!(state.doc().unwrap().stamp(id2).unwrap().source, StampSource::Sticker { .. }));
    assert_eq!(undo_len(&state), 2, "add sticker + add stamp are one entry");
    assert_eq!(state.doc().unwrap().undo_label(), Some("Drop"));
    state.doc_mut().unwrap().undo();
    assert!(state.doc().unwrap().stamp(id2).is_none());
    assert_eq!(state.doc().unwrap().stickers().count(), 0, "the tray sticker went with the stamp");
    // A miss (the corner of the view): nothing placed, the picture is the Stamp tool's picture.
    let stamps = state.doc().unwrap().stamps.len();
    drop_at(&mut state, Spot::View(ViewId::Main3D, at(&xf, 0.01, 0.01)), vec![png_item("miss.png")]);
    assert_eq!(state.doc().unwrap().stamps.len(), stamps);
    assert!(matches!(state.tools.stamp.source, StampToolSource::Blob(_)));
    assert_eq!(state.tools.stamp.source_name, "miss.png");
    assert!(state.actions.iter().any(|a| matches!(a, AppAction::SetTool(ToolKind::Stamp))));
}

/// 6.2 row "2D view": a UV stamp centred at the texel under the point.
#[test]
fn a_drop_on_the_2d_view_places_a_uv_stamp_at_the_texel() {
    let mut state = state_with(SampleKind::Cube);
    let xf = xf_2d(&mut state);
    let pos = at(&xf, 0.3, 0.6);
    let px = match surface_of(&state, ViewId::Main2D, pos) {
        Some((_, Surface::Uv { px, .. })) => px,
        _ => panic!("not on the texture"),
    };
    drop_at(&mut state, Spot::View(ViewId::Main2D, pos), vec![png_item("a.png")]);
    let id = state.session.as_ref().unwrap().selected_stamp.unwrap();
    let Placement::Uv(u) = state.doc().unwrap().stamp(id).unwrap().placement else { panic!("uv") };
    assert!((u.center_px - px).length() <= 2.0, "{:?} vs {px:?}", u.center_px);
}

/// 6.2 row "a stamp's row": an image stamp's picture is replaced; a sticker stamp gets the picture as a new top layer
/// (a sticker other stamps show is detached first: one undo step); a `.wtes` is shown by the stamp; a text stamp says
/// why nothing happens.
#[test]
fn a_drop_on_a_stamp_row() {
    let mut state = state_with(SampleKind::Sphere);
    let xf = xf_3d(&mut state);
    let pos = at(&xf, 0.5, 0.5);
    drop_at(&mut state, Spot::View(ViewId::Main3D, pos), vec![png_item("first.png")]);
    let stamp = state.session.as_ref().unwrap().selected_stamp.unwrap();
    let blob_of = |state: &AppState| match state.doc().unwrap().stamp(stamp).unwrap().source {
        StampSource::Image { blob } => blob,
        _ => panic!("image"),
    };
    let before = blob_of(&state);
    let n = undo_len(&state);
    let other = DropItem::Data { name: "second.png".into(), kind: DataKind::Png, bytes: png_bytes(30, 30, [0, 0, 255, 255]) };
    drop_at(&mut state, Spot::Stamp(stamp), vec![other.clone()]);
    assert_ne!(blob_of(&state), before, "the picture is replaced");
    assert_eq!(undo_len(&state), n + 1);
    // A sticker file: the stamp becomes a sticker stamp showing it.
    let sticker = DropItem::Data { name: "Sticker.wtes".into(), kind: DataKind::Sticker, bytes: wtes_bytes() };
    drop_at(&mut state, Spot::Stamp(stamp), vec![sticker]);
    let StampSource::Sticker { sticker: sid } = state.doc().unwrap().stamp(stamp).unwrap().source else { panic!("sticker stamp") };
    assert_eq!(state.doc().unwrap().sticker(sid).unwrap().name, "Badge");
    assert_eq!(undo_len(&state), n + 2, "promote + import + show: one entry");
    // A picture on the sticker stamp: one more layer on top.
    drop_at(&mut state, Spot::Stamp(stamp), vec![other.clone()]);
    let StampSource::Sticker { sticker: sid } = state.doc().unwrap().stamp(stamp).unwrap().source else { panic!() };
    let layers = &state.doc().unwrap().sticker(sid).unwrap().layers;
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[1].name, "second");
    assert_eq!(undo_len(&state), n + 3);
    // Shared with a second stamp: only the stamp dropped on changes.
    let placement = state.doc().unwrap().stamp(stamp).unwrap().placement;
    let second = state
        .doc_mut()
        .unwrap()
        .add_stamp("Twin", StampSource::Sticker { sticker: sid }, placement, wte_core::TextureSetId(0))
        .unwrap();
    let n = undo_len(&state);
    drop_at(&mut state, Spot::Stamp(stamp), vec![other]);
    let StampSource::Sticker { sticker: mine } = state.doc().unwrap().stamp(stamp).unwrap().source else { panic!() };
    assert_ne!(mine, sid, "detached");
    assert_eq!(state.doc().unwrap().sticker(mine).unwrap().layers.len(), 3);
    assert_eq!(state.doc().unwrap().sticker(sid).unwrap().layers.len(), 2, "the twin's sticker is unchanged");
    assert!(matches!(state.doc().unwrap().stamp(second).unwrap().source, StampSource::Sticker { sticker } if sticker == sid));
    assert_eq!(undo_len(&state), n + 1, "detach + add layer: one entry");
}

/// 6.2 row "the sticker canvas": a picture becomes a layer centred at the point, a `.wtes` adds its layers (and leaves
/// no tray sticker behind); one undo step each.
#[test]
fn a_drop_on_the_sticker_canvas_adds_layers_at_the_point() {
    let mut state = state_with(SampleKind::Sphere);
    let xf = xf_3d(&mut state);
    drop_at(&mut state, Spot::View(ViewId::Main3D, at(&xf, 0.5, 0.5)), vec![png_item("base.png")]);
    let stamp = state.session.as_ref().unwrap().selected_stamp.unwrap();
    crate::sticker::enter_in(&mut state, Some(stamp));
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 60.0), egui::vec2(600.0, 600.0));
    state.sticker_canvas_rect = Some(rect);
    if let Some(e) = crate::sticker::edit_mut(&mut state) {
        e.camera.fit([40, 20], rect);
    }
    let pos = rect.min + egui::vec2(200.0, 250.0);
    let want = crate::sticker::canvas::layer_point(&mut state, pos).expect("a canvas point");
    let n = undo_len(&state);
    let picture = DropItem::Data { name: "dot.png".into(), kind: DataKind::Png, bytes: png_bytes(10, 6, [255, 255, 0, 255]) };
    drop_at(&mut state, Spot::Canvas(pos), vec![picture]);
    let StampSource::Sticker { sticker } = state.doc().unwrap().stamp(stamp).unwrap().source else { panic!("promoted") };
    let doc = Arc::clone(state.doc().unwrap().sticker(sticker).unwrap());
    let top: &StickerLayer = doc.layers.last().unwrap();
    assert_eq!(top.name, "dot");
    assert!(matches!(top.content, StickerContent::Picture(_)));
    let centre = top.transform.translate + glam::Vec2::new(5.0, 3.0);
    assert!((centre - want).length() <= 1.0, "centred at the point: {centre:?} vs {want:?}");
    assert_eq!(undo_len(&state), n + 1);
    assert_eq!(crate::sticker::edit_of(&state).unwrap().active_layer, Some(top.id));
    // A sticker file: its layer is added; the tray keeps nothing of it.
    let sticker_item = DropItem::Data { name: "Sticker.wtes".into(), kind: DataKind::Sticker, bytes: wtes_bytes() };
    drop_at(&mut state, Spot::Canvas(pos), vec![sticker_item]);
    let doc = Arc::clone(state.doc().unwrap().sticker(sticker).unwrap());
    assert_eq!(doc.layers.len(), 3);
    assert_eq!(doc.layers[2].name, "Badge");
    assert_eq!(state.doc().unwrap().stickers().count(), 1, "no tray sticker left behind");
    assert_eq!(undo_len(&state), n + 2);
}

/// Several pictures in one drop: the first follows the table, the others go to the tray ("N more added to the
/// tray"), all in one undo step; a sticker file anywhere else goes to the tray as the Stamp tool's source.
#[test]
fn several_pictures_and_drops_elsewhere() {
    let mut state = state_with(SampleKind::Sphere);
    let xf = xf_3d(&mut state);
    let items = vec![png_item("one.png"), png_item("two.png"), png_item("three.png")];
    drop_at(&mut state, Spot::View(ViewId::Main3D, at(&xf, 0.5, 0.5)), items);
    assert_eq!(state.doc().unwrap().stamps.len(), 1);
    assert_eq!(state.doc().unwrap().stickers().count(), 2, "two tray stickers");
    assert_eq!(undo_len(&state), 1);
    assert!(state.toasts.visible_texts().contains(&"2 more added to the tray"), "{:?}", state.toasts.visible_texts());
    // A sticker file elsewhere.
    let sticker = DropItem::Data { name: "Sticker.wtes".into(), kind: DataKind::Sticker, bytes: wtes_bytes() };
    drop_at(&mut state, Spot::Elsewhere, vec![sticker]);
    let StampToolSource::Sticker(id) = state.tools.stamp.source else { panic!("the Stamp tool's source") };
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().name, "Badge");
    assert_eq!(state.sticker_ui.tray_selected, Some(id));
    // A picture file elsewhere goes the way of Import Stamp Image, as before.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join("sticker").join("unit_dnd");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("from disk.png");
    std::fs::write(&file, png_bytes(8, 8, [1, 2, 3, 255])).unwrap();
    state.actions.clear();
    drop_at(&mut state, Spot::Elsewhere, vec![DropItem::File(file.clone())]);
    assert!(matches!(&state.actions[..], [AppAction::ImportStampImage(Some(p))] if *p == file), "{:?}", state.actions);
    // A picture that cannot be decoded: a toast, nothing else.
    let n = undo_len(&state);
    let bad = DropItem::Data { name: "bad.png".into(), kind: DataKind::Png, bytes: b"not a png".to_vec() };
    drop_at(&mut state, Spot::View(ViewId::Main3D, at(&xf, 0.5, 0.5)), vec![bad]);
    assert_eq!(undo_len(&state), n);
    assert!(state.toasts.visible_texts().iter().any(|t| t.starts_with("Cannot use bad.png")));
}

/// A DIB dropped as data is converted on the worker (with the all-zero-alpha rule); a malformed one is a sentence.
#[test]
fn picture_data_is_converted_on_the_worker() {
    let mut p = wte_core::PixelBuf::new(3, 2);
    p.data.fill(0);
    for px in p.data.as_chunks_mut::<4>().0 {
        px[0] = 9;
    }
    let dib = crate::ole::formats::pixels_to_dibv5(&p);
    let prepared = prepare(Spot::Elsewhere, vec![DropItem::Data { name: "Dropped picture.png".into(), kind: DataKind::Dib, bytes: dib }]);
    let Ready::Picture { bytes, .. } = &prepared.ready[0] else { panic!("a picture") };
    let decoded = wte_core::decode_image(bytes, Some("x.png"), 64).unwrap();
    assert_eq!(&decoded.pixels.data[..4], &[9, 0, 0, 255], "all-zero alpha is opaque");
    let bad = prepare(Spot::Elsewhere, vec![DropItem::Data { name: "x".into(), kind: DataKind::Dib, bytes: vec![1, 2, 3] }]);
    assert!(bad.ready.is_empty());
    assert!(bad.failed[0].starts_with("Cannot use the dropped picture"));
}

/// A drop from the drop target: a project among the files opens as before; without a project open pictures are
/// refused with the usual toast; picture data is prepared on a worker and handed to dispatch.
#[test]
fn drops_from_the_drop_target_start_a_job_or_open_a_document() {
    let ctx = egui::Context::default();
    let mut state = AppState::new(Prefs::default(), true);
    let wtep = DropItem::File(PathBuf::from(r"C:\p\project.wtep"));
    let events = vec![OleEvent::Drop { pos: [10.0, 10.0], items: vec![wtep, png_item("a.png")], skipped: 0 }];
    crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| handle_events(ui.ctx(), &mut state, events.clone()));
    assert!(matches!(&state.actions[..], [AppAction::OpenPath(p)] if p.ends_with("project.wtep")), "{:?}", state.actions);
    state.actions.clear();
    let events = vec![OleEvent::Drop { pos: [10.0, 10.0], items: vec![png_item("a.png")], skipped: 2 }];
    crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| handle_events(ui.ctx(), &mut state, events.clone()));
    let texts = state.toasts.visible_texts();
    assert!(texts.contains(&"Open a model first"), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("2 dropped items were too large")), "{texts:?}");
    assert!(!state.drop_ui.busy());
    // With a model open the drop goes to a worker, then to dispatch as `DropReady`.
    let mut state = state_with(SampleKind::Sphere);
    let events = vec![OleEvent::Drop { pos: [10.0, 10.0], items: vec![png_item("a.png")], skipped: 0 }];
    crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| handle_events(ui.ctx(), &mut state, events.clone()));
    assert!(state.drop_ui.busy());
    for _ in 0..500 {
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| handle_events(ui.ctx(), &mut state, Vec::new()));
        if !state.drop_ui.busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(matches!(&state.actions[..], [AppAction::DropReady(p)] if p.spot == Spot::Elsewhere && p.ready.len() == 1), "{:?}", state.actions);
}

/// The marker's sentences say what a drop would do.
#[test]
fn the_hover_sentences() {
    let mut state = state_with(SampleKind::Sphere);
    let xf = xf_3d(&mut state);
    let picture = DragKinds { picture: true, ..DragKinds::default() };
    let pos = at(&xf, 0.5, 0.5);
    assert_eq!(hover_text(&state, Spot::View(ViewId::Main3D, pos), &picture), "Drop to place the picture here");
    let several = DragKinds { extensions: vec!["png".into(), "jpg".into(), "png".into()], ..DragKinds::default() };
    assert_eq!(hover_text(&state, Spot::View(ViewId::Main3D, pos), &several), "Drop to place the picture here (2 more go to the tray)");
    let sticker = DragKinds { sticker: true, ..DragKinds::default() };
    assert_eq!(hover_text(&state, Spot::Elsewhere, &sticker), "Drop to add the sticker to the tray");
    assert_eq!(hover_text(&state, Spot::View(ViewId::Main3D, at(&xf, 0.01, 0.01)), &picture), "Drop to use the picture with the Stamp tool");
    assert_eq!(hover_text(&AppState::new(Prefs::default(), true), Spot::Elsewhere, &picture), "Open a model first");
}
