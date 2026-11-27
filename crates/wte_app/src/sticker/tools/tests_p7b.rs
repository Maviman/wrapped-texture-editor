//! Tests of the selection and cut-out tools (stickers P7b; STICKER_PLAN 7 "P7b", 3.3, 3.4): the Marquee, Lasso and
//! Magic Wand through the canvas's pointer routing (modes by key and by the Tool tab's row; no entry, no promotion),
//! the Magic Eraser, Delete with and without a selection, Move Selection and Layer via Copy / Cut, the selection
//! commands, the Cut-out and Filter commands (one entry each, an image stamp promoted inside), the popovers with their
//! live preview, the selection outline, the command list's rules and the shared-picture question.

use std::sync::Arc;

use eframe::egui;
use glam::Vec2;
use wte_core::{Document, IRect, PixelBuf, SelectMode, Selection, StampId, StampSource, StickerContent, StickerDoc, StickerId, StickerLayerId};

use crate::action::AppAction;
use crate::state::{AppState, Dialog, StickerTool, ViewMode};
use crate::sticker::canvas::{Geom, Hit, handle_input, refresh};
use crate::sticker::commands::{CROP_TO_SELECTION_LABEL, DELETE_SELECTED_LABEL};
use crate::sticker::filters::{COMMANDS, available, enabled};
use crate::sticker::outline::{OutlineCache, outline_of};
use crate::sticker::popover::PopKind;
use crate::sticker::tests::CANVAS;
use crate::sticker::tools::magic_eraser::MAGIC_ERASER_LABEL;
use crate::sticker::tools::move_selection::{LAYER_VIA_COPY_LABEL, LAYER_VIA_CUT_LABEL, MOVE_SELECTION_LABEL};
use crate::sticker::{StickerAction, edit_mut, edit_of, enter_in, run_in};

// ---- helpers ----

const GEOM: Geom = Geom { origin: Vec2::ZERO, size: [64, 64] };
const RED: [u8; 4] = [200, 30, 40, 255];
const NONE: egui::Modifiers = egui::Modifiers::NONE;

fn doc(state: &AppState) -> &Document {
    state.doc().unwrap()
}

fn undo_len(state: &AppState) -> usize {
    doc(state).history.undo_len()
}

/// The test picture: a red disc of radius 20 at (32, 32) with a one-pixel soft edge on white, opaque.
fn disc() -> PixelBuf {
    let mut p = PixelBuf::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::splat(32.0));
            let k = (20.5 - d).clamp(0.0, 1.0);
            let c = |a: u8| (255.0 + (a as f32 - 255.0) * k).round() as u8;
            p.set(x, y, [c(RED[0]), c(RED[1]), c(RED[2]), 255]);
        }
    }
    p
}

/// The sphere sample with an image stamp of `disc()` (selected), open in the Sticker view, camera fitted.
fn opened(ctx: &egui::Context) -> (AppState, StampId) {
    let (mut state, _, _) = crate::sticker::tests::image_stamp_state(ViewMode::View3D);
    let img = disc();
    let png = wte_core::encode_png(64, 64, &img.data, wte_core::PngLevel::Fast).unwrap();
    let doc = state.doc_mut().unwrap();
    let blob = doc.add_image_blob("disc.png", Arc::from(png)).unwrap();
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.0, 0.5), glam::Vec3::Z, glam::Vec3::Y, Vec2::splat(0.3), 0.6);
    let stamp = doc.add_stamp("Disc", StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    doc.history.clear();
    state.session.as_mut().unwrap().selected_stamp = Some(stamp);
    enter_in(&mut state, Some(stamp));
    edit_mut(&mut state).unwrap().camera.fit(GEOM.size, CANVAS);
    refresh(ctx, &mut state);
    (state, stamp)
}

fn run_queued(state: &mut AppState) {
    for action in std::mem::take(&mut state.actions) {
        match action {
            AppAction::Sticker(a) => run_in(state, a),
            AppAction::SetPrimaryColor(c) => state.colors.primary = c,
            other => panic!("unexpected action {other:?}"),
        }
    }
}

fn pass(ctx: &egui::Context, state: &mut AppState, hit: Hit, events: Vec<egui::Event>) {
    state.pen.frame = crate::pen::PenFrame::default();
    let input = egui::RawInput {
        events,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        ..Default::default()
    };
    crate::theme::run_test_pass(ctx, input, |ui| handle_input(ui, state, CANVAS, GEOM, hit, true));
    run_queued(state);
    refresh(ctx, state);
}

fn button(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers }
}

fn screen(state: &AppState, p: Vec2) -> egui::Pos2 {
    GEOM.to_screen(&edit_of(state).unwrap().camera, CANVAS, p)
}

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

/// A press with `mods`, moves through `path` (layer space), a release; one pass per point.
fn drag(ctx: &egui::Context, state: &mut AppState, hit: Hit, path: &[Vec2], mods: egui::Modifiers) {
    let first = screen(state, path[0]);
    pass(ctx, state, hit, vec![egui::Event::PointerMoved(first), button(first, true, mods)]);
    for p in &path[1..] {
        let at = screen(state, *p);
        pass(ctx, state, hit, vec![egui::Event::PointerMoved(at)]);
    }
    let last = screen(state, *path.last().unwrap());
    pass(ctx, state, hit, vec![button(last, false, mods)]);
}

/// A click (press and release without motion).
fn click(ctx: &egui::Context, state: &mut AppState, hit: Hit, p: Vec2, mods: egui::Modifiers) {
    let at = screen(state, p);
    pass(ctx, state, hit, vec![egui::Event::PointerMoved(at), button(at, true, mods)]);
    pass(ctx, state, hit, vec![button(at, false, mods)]);
}

fn tool(state: &mut AppState, t: StickerTool) {
    edit_mut(state).unwrap().tool = t;
}

fn selection(state: &AppState) -> Option<Arc<Selection>> {
    edit_of(state).unwrap().selection.clone()
}

fn sel_at(state: &AppState, x: i32, y: i32) -> u8 {
    selection(state).map_or(0, |s| s.at(x, y))
}

fn is_image(state: &AppState, stamp: StampId) -> bool {
    matches!(doc(state).stamp(stamp).unwrap().source, StampSource::Image { .. })
}

fn sticker_id(state: &AppState, stamp: StampId) -> StickerId {
    match doc(state).stamp(stamp).unwrap().source {
        StampSource::Sticker { sticker } => sticker,
        ref other => panic!("not a sticker stamp: {other:?}"),
    }
}

fn sticker_of(state: &AppState, stamp: StampId) -> Arc<StickerDoc> {
    Arc::clone(doc(state).sticker(sticker_id(state, stamp)).unwrap())
}

/// The owned pixels of layer `index` (bottom = 0) of the stamp's sticker.
fn layer_pixels(state: &AppState, stamp: StampId, index: usize) -> Arc<PixelBuf> {
    Arc::clone(sticker_of(state, stamp).layers[index].owned_picture().expect("edited pixels"))
}

fn select_rect(state: &mut AppState, a: [f32; 2], b: [f32; 2]) {
    run_in(state, StickerAction::SelectRect([a[0], a[1], b[0], b[1]]));
}

fn label(state: &AppState) -> Option<String> {
    doc(state).undo_label().map(str::to_owned)
}

// ---- the selection tools ----

/// The Marquee: a drag selects a box; Shift adds, Alt subtracts, the Tool tab's row too; a click drops the selection;
/// the ellipse and the fixed aspect; no history entry and no promotion of the image stamp.
#[test]
fn k7b_the_marquee_selects_boxes_by_mode() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    tool(&mut state, StickerTool::Marquee);
    drag(&ctx, &mut state, Hit::Image, &[v(4.0, 4.0), v(12.0, 8.0), v(20.0, 12.0)], NONE);
    let s = selection(&state).expect("a selection");
    assert_eq!(s.bounds, IRect::new(4, 4, 20, 12));
    assert_eq!((sel_at(&state, 10, 8), sel_at(&state, 30, 30)), (255, 0));
    drag(&ctx, &mut state, Hit::Image, &[v(40.0, 40.0), v(50.0, 50.0)], egui::Modifiers::SHIFT);
    assert_eq!((sel_at(&state, 10, 8), sel_at(&state, 45, 45)), (255, 255), "Shift adds");
    drag(&ctx, &mut state, Hit::Image, &[v(4.0, 4.0), v(10.0, 12.0)], egui::Modifiers::ALT);
    assert_eq!((sel_at(&state, 6, 8), sel_at(&state, 15, 8)), (0, 255), "Alt subtracts");
    state.sticker_ui.select_mode = SelectMode::Intersect;
    drag(&ctx, &mut state, Hit::Image, &[v(0.0, 0.0), v(18.0, 64.0)], NONE);
    assert_eq!((sel_at(&state, 15, 8), sel_at(&state, 19, 8), sel_at(&state, 45, 45)), (255, 0, 0), "the row's Intersect");
    state.sticker_ui.select_mode = SelectMode::Replace;
    click(&ctx, &mut state, Hit::Image, v(30.0, 30.0), NONE);
    assert!(selection(&state).is_none(), "a click drops the selection");
    state.sticker_ui.marquee.ellipse = true;
    drag(&ctx, &mut state, Hit::Image, &[v(0.0, 0.0), v(64.0, 64.0)], NONE);
    assert_eq!((sel_at(&state, 32, 32), sel_at(&state, 1, 1)), (255, 0), "the ellipse leaves the corners");
    state.sticker_ui.marquee = crate::sticker::tools::marquee::MarqueeOptions { fixed_aspect: true, aspect: [2.0, 1.0], ..Default::default() };
    drag(&ctx, &mut state, Hit::Image, &[v(0.0, 0.0), v(10.0, 30.0)], NONE);
    assert_eq!(selection(&state).unwrap().bounds, IRect::new(0, 0, 60, 30), "2 : 1 from the longer side");
    assert_eq!(undo_len(&state), 0);
    assert!(is_image(&state, stamp), "a selection never promotes");
}

/// The Lasso: a freehand drag closes at the release; clicks put polygon corners that Enter closes; Cancel drops an
/// open polygon; two points make nothing.
#[test]
fn k7b_the_lasso_draws_freehand_and_polygons() {
    let ctx = egui::Context::default();
    let (mut state, _) = opened(&ctx);
    tool(&mut state, StickerTool::Lasso);
    drag(&ctx, &mut state, Hit::Image, &[v(10.0, 10.0), v(30.0, 10.0), v(30.0, 30.0), v(10.0, 30.0)], NONE);
    assert!(state.sticker_ui.lasso.is_none(), "closed at the release");
    assert_eq!((sel_at(&state, 20, 20), sel_at(&state, 5, 5), sel_at(&state, 35, 20)), (255, 0, 0));
    for p in [v(40.0, 40.0), v(60.0, 40.0), v(60.0, 60.0)] {
        click(&ctx, &mut state, Hit::Image, p, egui::Modifiers::SHIFT);
    }
    assert_eq!(state.sticker_ui.lasso.as_ref().map(|l| (l.points.len(), l.mode)), Some((3, SelectMode::Add)));
    assert_eq!(sel_at(&state, 55, 45), 0, "not yet closed");
    run_in(&mut state, StickerAction::EnterKey);
    assert!(state.sticker_ui.lasso.is_none());
    assert_eq!((sel_at(&state, 20, 20), sel_at(&state, 55, 45), sel_at(&state, 42, 58)), (255, 255, 0), "added, a triangle");
    // A click on the first point closes too.
    for p in [v(2.0, 40.0), v(8.0, 40.0), v(8.0, 50.0)] {
        click(&ctx, &mut state, Hit::Image, p, NONE);
    }
    click(&ctx, &mut state, Hit::Image, v(2.2, 40.1), NONE);
    assert!(state.sticker_ui.lasso.is_none());
    assert_eq!((sel_at(&state, 6, 44), sel_at(&state, 20, 20)), (255, 0), "a new selection");
    for p in [v(40.0, 2.0), v(50.0, 2.0)] {
        click(&ctx, &mut state, Hit::Image, p, NONE);
    }
    run_in(&mut state, StickerAction::CancelLasso);
    assert!(state.sticker_ui.lasso.is_none());
    assert_eq!(sel_at(&state, 6, 44), 255, "Cancel keeps the selection");
    for p in [v(40.0, 2.0), v(50.0, 2.0)] {
        click(&ctx, &mut state, Hit::Image, p, NONE);
    }
    run_in(&mut state, StickerAction::CloseLasso);
    assert!(state.status_hint.contains("three points") && sel_at(&state, 6, 44) == 255, "{}", state.status_hint);
    assert_eq!(undo_len(&state), 0);
}

/// The Magic Wand: the clicked colour region of the active layer, or with "Sample all layers" of the composite;
/// Contiguous off takes every match; Shift adds.
#[test]
fn k7b_the_wand_selects_the_region_of_the_sampled_picture() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    tool(&mut state, StickerTool::Wand);
    click(&ctx, &mut state, Hit::Image, v(2.0, 2.0), NONE);
    assert_eq!((sel_at(&state, 2, 2), sel_at(&state, 62, 62), sel_at(&state, 32, 32)), (255, 255, 0), "the white around the disc");
    assert!(is_image(&state, stamp));
    // A transparent layer on top: the active layer alone is transparent everywhere; all layers see the disc.
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    let id = sticker_id(&state, stamp);
    click(&ctx, &mut state, Hit::Sticker(id), v(32.0, 32.0), NONE);
    assert_eq!(sel_at(&state, 2, 2), 255, "the empty layer is one transparent region");
    state.sticker_ui.wand.all_layers = true;
    click(&ctx, &mut state, Hit::Sticker(id), v(32.0, 32.0), NONE);
    assert_eq!((sel_at(&state, 32, 32), sel_at(&state, 2, 2)), (255, 0), "the disc of the composite");
    click(&ctx, &mut state, Hit::Sticker(id), v(2.0, 2.0), egui::Modifiers::SHIFT);
    assert_eq!((sel_at(&state, 32, 32), sel_at(&state, 2, 2)), (255, 255));
    run_in(&mut state, StickerAction::WandAt([-5.0, 2.0], SelectMode::Replace));
    assert_eq!(state.status_hint, crate::sticker::tools::wand::OUTSIDE_HINT);
}

/// Color Range: a popover whose colour a canvas click picks; its live outline; Apply makes the selection.
#[test]
fn k7b_color_range_picks_previews_and_applies() {
    let ctx = egui::Context::default();
    let (mut state, _) = opened(&ctx);
    run_in(&mut state, StickerAction::OpenPopover(PopKind::ColorRange { color: [0, 0, 255, 255], tolerance: 32, all_layers: false, mode: SelectMode::Replace }));
    click(&ctx, &mut state, Hit::Image, v(32.0, 32.0), NONE);
    assert!(matches!(state.sticker_ui.popover.as_ref().unwrap().kind, PopKind::ColorRange { color, .. } if color == RED), "the click picked red");
    crate::sticker::popover::frame(&ctx, &mut state);
    let preview = state.sticker_ui.popover.as_ref().unwrap().selection.clone().expect("a live outline");
    assert_eq!((preview.at(32, 32), preview.at(2, 2)), (255, 0));
    assert!(selection(&state).is_none(), "nothing applied yet");
    run_in(&mut state, StickerAction::ApplyPopover);
    assert!(state.sticker_ui.popover.is_none());
    assert_eq!(*selection(&state).unwrap(), *preview);
}

// ---- pixels ----

/// The Magic Eraser: one entry that promotes the image stamp; limited by the selection; "Into mask" adds a mask and
/// keeps the pixels; undo brings the image stamp back.
#[test]
fn k7b_the_magic_eraser_erases_the_region_once() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    tool(&mut state, StickerTool::MagicEraser);
    select_rect(&mut state, [0.0, 0.0], [32.0, 64.0]);
    click(&ctx, &mut state, Hit::Image, v(2.0, 2.0), NONE);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(MAGIC_ERASER_LABEL)));
    let p = layer_pixels(&state, stamp, 0);
    assert_eq!((p.get(2, 2)[3], p.get(32, 32)[3], p.get(62, 2)[3]), (0, 255, 255), "erased inside the selection only");
    state.doc_mut().unwrap().undo();
    assert!(is_image(&state, stamp));
    run_in(&mut state, StickerAction::SelectNone);
    state.sticker_ui.magic_eraser.into_mask = true;
    let id = {
        click(&ctx, &mut state, Hit::Image, v(2.0, 2.0), NONE);
        sticker_id(&state, stamp)
    };
    let s = doc(&state).sticker(id).unwrap();
    let mask = s.layers[0].mask.as_ref().expect("a mask was added");
    assert_eq!((mask.pixels.get(2, 2), mask.pixels.get(32, 32)), (0, 255));
    assert_eq!(undo_len(&state), 1);
    assert!(matches!(s.layers[0].content, StickerContent::Picture(wte_core::PicturePixels::Blob(_))), "the pixels are kept");
}

/// Delete / Backspace in the Sticker view (the dispatcher maps Delete to `DeleteKey`): with a selection the selected
/// pixels go (one entry, the stamp stays selected and placed); without one the status line says what to do.
#[test]
fn k7b_delete_clears_the_selected_pixels_and_never_the_stamp() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let stamps = doc(&state).stamps.len();
    run_in(&mut state, StickerAction::DeleteKey);
    assert_eq!((state.status_hint.as_str(), undo_len(&state)), (crate::sticker::DELETE_HINT, 0));
    select_rect(&mut state, [0.0, 0.0], [16.0, 16.0]);
    run_in(&mut state, StickerAction::DeleteKey);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(DELETE_SELECTED_LABEL)));
    let p = layer_pixels(&state, stamp, 0);
    assert_eq!((p.get(5, 5)[3], p.get(20, 20)[3]), (0, 255));
    assert_eq!(doc(&state).stamps.len(), stamps);
    assert_eq!(state.session.as_ref().unwrap().selected_stamp, Some(stamp));
    assert_eq!(state.view_mode, ViewMode::Sticker);
    // Again on the same pixels: nothing changes, no entry.
    run_in(&mut state, StickerAction::DeleteKey);
    assert_eq!(undo_len(&state), 1);
    // With "Paint on mask" it hides on the mask.
    run_in(&mut state, StickerAction::SelectNone);
    run_in(&mut state, StickerAction::AddMask);
    select_rect(&mut state, [40.0, 40.0], [48.0, 48.0]);
    run_in(&mut state, StickerAction::DeleteKey);
    let s = sticker_of(&state, stamp);
    assert_eq!(s.layers[0].mask.as_ref().unwrap().pixels.get(44, 44), 0);
    assert_eq!(layer_pixels(&state, stamp, 0).get(44, 44)[3], 255, "the pixels are kept");
}

/// Move Selection: a Move drag inside the selection cuts the pixels into a new layer and moves it: one entry, the
/// selection follows; one undo brings the image stamp back; Esc in the middle leaves nothing; a press outside the
/// selection moves the layer as before.
#[test]
fn k7b_move_selection_cuts_and_moves_in_one_entry() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    tool(&mut state, StickerTool::Move);
    select_rect(&mut state, [24.0, 24.0], [40.0, 40.0]);
    // Esc (cancel_drag) in the middle: nothing stays.
    let at = screen(&state, v(32.0, 32.0));
    pass(&ctx, &mut state, Hit::Image, vec![egui::Event::PointerMoved(at), button(at, true, NONE)]);
    let to = screen(&state, v(37.0, 32.0));
    pass(&ctx, &mut state, Hit::Image, vec![egui::Event::PointerMoved(to)]);
    assert!(crate::sticker::drag_moved(&state) && doc(&state).has_pending_edit());
    crate::sticker::cancel_drag(&mut state);
    assert!(is_image(&state, stamp) && !doc(&state).has_pending_edit() && undo_len(&state) == 0);
    // The whole drag.
    drag(&ctx, &mut state, Hit::Image, &[v(32.0, 32.0), v(37.0, 32.0), v(42.0, 32.0)], NONE);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(MOVE_SELECTION_LABEL)));
    let s = sticker_of(&state, stamp);
    assert_eq!(s.layers.len(), 2);
    let new = &s.layers[1];
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(new.id));
    let piece = new.owned_picture().unwrap();
    assert_eq!(([piece.width, piece.height], new.transform.translate), ([16, 16], v(34.0, 24.0)), "cropped, moved by 10");
    assert_eq!(piece.get(0, 0), disc().get(24, 24));
    assert_eq!(layer_pixels(&state, stamp, 0).get(30, 30)[3], 0, "cut out of the source");
    assert_eq!((sel_at(&state, 45, 30), sel_at(&state, 26, 30)), (255, 0), "the selection moved with the pixels");
    state.doc_mut().unwrap().undo();
    assert!(is_image(&state, stamp), "one undo takes the cut back too");
    // A press outside the selection is the ordinary Move.
    crate::sticker::validate_in(&mut state);
    let at = screen(&state, v(4.0, 4.0));
    pass(&ctx, &mut state, Hit::Image, vec![egui::Event::PointerMoved(at), button(at, true, NONE)]);
    assert!(matches!(state.sticker_ui.drag, Some(crate::sticker::CanvasDrag::Move { .. })));
    pass(&ctx, &mut state, Hit::Image, vec![button(at, false, NONE)]);
}

/// Layer via Copy and Layer via Cut: one entry each, the new layer active above the source, the selection dropped.
#[test]
fn k7b_layer_via_copy_and_cut() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    run_in(&mut state, StickerAction::LayerViaCopy);
    assert_eq!((undo_len(&state), state.status_hint.as_str()), (0, "Select an area first"));
    select_rect(&mut state, [24.0, 24.0], [40.0, 40.0]);
    run_in(&mut state, StickerAction::LayerViaCopy);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(LAYER_VIA_COPY_LABEL)));
    let s = sticker_of(&state, stamp);
    assert_eq!((s.layers.len(), s.layers[1].name.as_str()), (2, "Disc copy"));
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(s.layers[1].id));
    assert!(selection(&state).is_none());
    assert!(matches!(s.layers[0].content, StickerContent::Picture(wte_core::PicturePixels::Blob(_))), "Copy keeps the source");
    run_in(&mut state, StickerAction::SelectLayer(s.layers[0].id));
    select_rect(&mut state, [24.0, 24.0], [40.0, 40.0]);
    run_in(&mut state, StickerAction::LayerViaCut);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (2, Some(LAYER_VIA_CUT_LABEL)));
    let s = sticker_of(&state, stamp);
    assert_eq!(s.layers.len(), 3);
    assert_eq!(layer_pixels(&state, stamp, 0).get(30, 30)[3], 0);
    // An empty part of the layer: nothing to copy.
    select_rect(&mut state, [24.0, 24.0], [40.0, 40.0]);
    run_in(&mut state, StickerAction::SelectLayer(s.layers[0].id));
    run_in(&mut state, StickerAction::LayerViaCopy);
    assert_eq!((undo_len(&state), state.status_hint.as_str()), (2, crate::sticker::tools::move_selection::NOTHING_SELECTED_HINT));
}

/// The selection commands: All, Invert, None, Grow / Shrink / Feather, From Layer Alpha, Crop to Selection (one entry,
/// the artboard is the selection's bounds, the selection goes).
#[test]
fn k7b_the_selection_commands() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    run_in(&mut state, StickerAction::SelectAll);
    assert_eq!(selection(&state).unwrap().bounds, IRect::new(0, 0, 64, 64));
    run_in(&mut state, StickerAction::InvertSelection);
    assert!(selection(&state).is_none(), "everything inverted is nothing");
    run_in(&mut state, StickerAction::InvertSelection);
    assert_eq!(selection(&state).unwrap().bounds, IRect::new(0, 0, 64, 64), "nothing inverted is everything");
    run_in(&mut state, StickerAction::SelectNone);
    run_in(&mut state, StickerAction::GrowSelection(2.0));
    assert_eq!(state.status_hint, crate::sticker::commands::NO_SELECTION_HINT);
    select_rect(&mut state, [20.0, 20.0], [30.0, 30.0]);
    run_in(&mut state, StickerAction::GrowSelection(2.0));
    assert_eq!(selection(&state).unwrap().bounds, IRect::new(18, 18, 32, 32));
    run_in(&mut state, StickerAction::ShrinkSelection(4.0));
    assert_eq!(selection(&state).unwrap().bounds, IRect::new(22, 22, 28, 28));
    run_in(&mut state, StickerAction::FeatherSelection(4.0));
    let s = selection(&state).unwrap();
    assert!(s.bounds.width() > 6 && s.at(25, 25) > 0 && s.at(25, 25) < 255 || s.at(22, 22) < 255, "a soft edge");
    // From Layer Alpha after Magic Eraser of the white: the disc.
    state.sticker_ui.magic_eraser.region.anti_alias = false;
    run_in(&mut state, StickerAction::SelectNone);
    run_in(&mut state, StickerAction::MagicErase([2.0, 2.0]));
    run_in(&mut state, StickerAction::SelectionFromAlpha);
    assert_eq!((sel_at(&state, 32, 32), sel_at(&state, 2, 2)), (255, 0));
    let undo = undo_len(&state);
    select_rect(&mut state, [8.0, 8.0], [40.0, 24.0]);
    run_in(&mut state, StickerAction::CropToSelection);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (undo + 1, Some(CROP_TO_SELECTION_LABEL)));
    assert_eq!(sticker_of(&state, stamp).artboard, IRect::new(8, 8, 40, 24));
    assert!(selection(&state).is_none());
    let _ = &ctx;
}

/// The Cut-out and Filter commands: one entry each with its label, an image stamp promoted inside; Remove Background
/// keeps the disc and clears the white with no white halo, then says the background is already transparent.
#[test]
fn k7b_cut_out_and_filter_commands_are_one_entry_each() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    run_in(&mut state, StickerAction::RemoveBackground);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some("Remove Background")));
    let p = layer_pixels(&state, stamp, 0);
    assert_eq!((p.get(0, 0)[3], p.get(32, 32)[3]), (0, 255));
    for px in p.data.as_chunks::<4>().0 {
        if px[3] > 0 && px[3] < 255 {
            assert!(px[1] < 90, "a soft edge pixel keeps a white tint: {px:?}");
        }
    }
    run_in(&mut state, StickerAction::RemoveBackground);
    assert_eq!((undo_len(&state), state.status_hint.as_str()), (1, "This picture already has a transparent background"));
    state.doc_mut().unwrap().undo();
    assert!(is_image(&state, stamp));
    run_in(&mut state, StickerAction::ColorToAlpha([255, 255, 255]));
    assert_eq!(label(&state).as_deref(), Some("Color to Alpha"));
    let p = layer_pixels(&state, stamp, 0);
    assert_eq!(p.get(0, 0)[3], 0);
    assert!((200..=230).contains(&p.get(32, 32)[3]), "red over white keeps {}", p.get(32, 32)[3]);
    for (action, name) in [
        (StickerAction::RemoveMatte(wte_core::Matte::White), "Remove White Matte"),
        (StickerAction::Defringe(2), "Defringe"),
        (StickerAction::Sharpen(1.0, 2.0), "Sharpen"),
    ] {
        let before = undo_len(&state);
        run_in(&mut state, action);
        assert!(undo_len(&state) == before + 1 && label(&state).as_deref() == Some(name) || state.status_hint.contains("nothing changed"), "{name}: {}", state.status_hint);
    }
    // Blur inside a selection: the rest is unchanged.
    let before = layer_pixels(&state, stamp, 0);
    select_rect(&mut state, [0.0, 0.0], [32.0, 64.0]);
    run_in(&mut state, StickerAction::Blur(3.0));
    assert_eq!(label(&state).as_deref(), Some("Blur"));
    let after = layer_pixels(&state, stamp, 0);
    for y in 0..64 {
        for x in 40..64 {
            assert_eq!(after.get(x, y), before.get(x, y), "({x}, {y}) is outside the selection");
        }
    }
    assert_ne!(after.get(12, 32), before.get(12, 32), "the edge inside the selection is blurred");
    let _ = &ctx;
}

/// The popovers: Blur's live preview is the picture Apply gives (the document unchanged until then); Cancel leaves
/// nothing and the canvas shows the sticker again; Color to Alpha takes its key from a canvas click.
#[test]
fn k7b_a_popover_previews_exactly_what_apply_gives() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let start = state.sticker_ui.mirror.clone().unwrap();
    run_in(&mut state, StickerAction::OpenPopover(PopKind::Blur { radius: 3.0 }));
    settle_popover(&ctx, &mut state);
    assert!(state.sticker_ui.popover.as_ref().unwrap().shows_picture);
    let preview = state.sticker_ui.mirror.clone().unwrap();
    assert_ne!(preview, start);
    assert_eq!(undo_len(&state), 0);
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.mirror.as_ref(), Some(&preview), "the preview stays while the popover is open");
    crate::sticker::popover::frame(&ctx, &mut state);
    assert_eq!(state.sticker_ui.popover.as_ref().unwrap().previews, 1, "computed once while nothing changes");
    run_in(&mut state, StickerAction::ClosePopover);
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.mirror.as_ref(), Some(&start), "Cancel shows the sticker again");
    assert!(is_image(&state, stamp) && undo_len(&state) == 0);
    run_in(&mut state, StickerAction::OpenPopover(PopKind::Blur { radius: 3.0 }));
    settle_popover(&ctx, &mut state);
    let want = crate::sticker::popover::previewed_data(&mut state);
    run_in(&mut state, StickerAction::ApplyPopover);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some("Blur")));
    let id = sticker_id(&state, stamp);
    let finished = state.doc_mut().unwrap().sticker_picture(id, None, 1.0).unwrap();
    assert_eq!(finished, preview, "the preview was the result");
    assert!(matches!(want, Some(wte_core::PixelData::Rgba(p)) if p == finished));
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.mirror.as_ref(), Some(&finished));
    // Color to Alpha: the canvas click is the key.
    run_in(&mut state, StickerAction::OpenPopover(PopKind::ColorToAlpha { key: [255, 255, 255] }));
    click(&ctx, &mut state, Hit::Sticker(id), v(32.0, 32.0), NONE);
    let PopKind::ColorToAlpha { key } = state.sticker_ui.popover.as_ref().unwrap().kind else { panic!() };
    assert_eq!(key, [RED[0], RED[1], RED[2]]);
    assert_eq!(undo_len(&state), 1, "picking changes nothing");
}

/// P7b check: waits (bounded) until the open popover's preview is shown, a frame every few milliseconds.
fn settle_popover(ctx: &egui::Context, state: &mut AppState) {
    let start = std::time::Instant::now();
    refresh(ctx, state);
    while crate::sticker::popover::pending(state) {
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "the popover preview never finished");
        std::thread::sleep(std::time::Duration::from_millis(2));
        refresh(ctx, state);
    }
}

/// Waits (bounded) until no popover preview thread runs any more.
fn wait_popover_threads(state: &AppState) {
    let start = std::time::Instant::now();
    while state.sticker_ui.popover_alive.load(std::sync::atomic::Ordering::Acquire) > 0 {
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "a popover preview thread never ended");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// P7b check: the picture preview of a popover is drawn on a job, not in the frame (a 2048-px styled sticker cost
/// ~255 ms of UI thread per update): the frame that starts it returns at once with the canvas unchanged; a result for
/// numbers that changed meanwhile is dropped and the newer numbers are drawn next (one job at a time); a job of a
/// closed popover never reaches the canvas; what lands is exactly what Apply gives.
#[test]
fn k7b_check_the_popover_preview_runs_on_a_job() {
    use std::sync::atomic::Ordering;
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let start = state.sticker_ui.mirror.clone().unwrap();
    let hold = Arc::clone(&state.sticker_ui.jobs.hold);
    hold.store(true, Ordering::Release);
    run_in(&mut state, StickerAction::OpenPopover(PopKind::Blur { radius: 3.0 }));
    refresh(&ctx, &mut state);
    let pop = state.sticker_ui.popover.as_ref().unwrap();
    assert!(state.sticker_ui.popover_job.is_some() && !pop.shows_picture && pop.previews == 1, "the job runs, nothing shown yet");
    assert!(crate::sticker::popover::pending(&state) && crate::sticker::live::pending(&state), "the idle check waits for it");
    assert_eq!(state.sticker_ui.mirror.as_ref(), Some(&start), "the frame did not draw the preview itself");
    // New numbers while it runs: no second job beside it.
    state.sticker_ui.popover.as_mut().unwrap().kind = PopKind::Blur { radius: 6.0 };
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.popover.as_ref().unwrap().previews, 1);
    assert_eq!(state.sticker_ui.popover_alive.load(Ordering::Acquire), 1);
    hold.store(false, Ordering::Release);
    settle_popover(&ctx, &mut state);
    assert_eq!(state.sticker_ui.popover.as_ref().unwrap().previews, 2, "the stale result was dropped, the new numbers drawn");
    let shown = state.sticker_ui.mirror.clone().unwrap();
    let (t, change) = crate::sticker::filters::compute(&mut state, crate::sticker::filters::CutOut::Blur(6.0)).unwrap();
    let crate::sticker::filters::Change::Data(data) = change else { panic!("pixels expected") };
    let want = state.doc_mut().unwrap().sticker_picture_with(t.on, t.layer, data).unwrap();
    assert_eq!(shown, want, "the preview of radius 6");
    assert_eq!(undo_len(&state), 0);
    // A job of a closed popover never lands.
    hold.store(true, Ordering::Release);
    state.sticker_ui.popover.as_mut().unwrap().kind = PopKind::Blur { radius: 9.0 };
    refresh(&ctx, &mut state);
    assert!(state.sticker_ui.popover_job.is_some());
    run_in(&mut state, StickerAction::ClosePopover);
    refresh(&ctx, &mut state);
    assert!(state.sticker_ui.popover_job.is_none() && !crate::sticker::popover::pending(&state));
    hold.store(false, Ordering::Release);
    wait_popover_threads(&state);
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.mirror.as_ref(), Some(&start), "the canvas shows the sticker");
    assert!(is_image(&state, stamp) && undo_len(&state) == 0);
    // Apply runs the command itself (on the UI thread, one entry), whatever the job was doing.
    run_in(&mut state, StickerAction::OpenPopover(PopKind::Blur { radius: 6.0 }));
    run_in(&mut state, StickerAction::ApplyPopover);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some("Blur")));
    let id = sticker_id(&state, stamp);
    assert_eq!(state.doc_mut().unwrap().sticker_picture(id, None, 1.0).unwrap(), want);
}

/// P7b check: an open polygon lasso is dropped when another tool is chosen (it stayed drawn under the Brush and Enter
/// closed it into a selection there); choosing the Lasso again keeps it.
#[test]
fn k7b_check_another_tool_drops_an_open_lasso() {
    let ctx = egui::Context::default();
    let (mut state, _) = opened(&ctx);
    tool(&mut state, StickerTool::Lasso);
    for p in [v(10.0, 10.0), v(30.0, 10.0), v(30.0, 30.0)] {
        click(&ctx, &mut state, Hit::Image, p, NONE);
    }
    crate::sticker::tool_changing(&mut state, StickerTool::Lasso);
    assert_eq!(state.sticker_ui.lasso.as_ref().map(|l| l.points.len()), Some(3));
    crate::sticker::tool_changing(&mut state, StickerTool::Brush);
    tool(&mut state, StickerTool::Brush);
    assert!(state.sticker_ui.lasso.is_none());
    run_in(&mut state, StickerAction::EnterKey);
    assert!(selection(&state).is_none(), "Enter under the Brush makes no selection");
}

/// P7b check (plan 7 "P7b" Checker): Delete and Backspace through the real key table in the Sticker view, with and
/// without a selection, with the stamp selected: the selected pixels go, the stamp and the texture layers stay.
#[test]
fn k7b_check_delete_and_backspace_keys_with_a_stamp_selected() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let layers_before = doc(&state).set(wte_core::TextureSetId(0)).map(|s| s.layers.len());
    for (k, key) in [egui::Key::Delete, egui::Key::Backspace].into_iter().enumerate() {
        let actions = key_actions(&ctx, &mut state, key);
        assert_eq!(actions.len(), 1, "{key:?}: {actions:?}");
        run_dispatched(&mut state, actions);
        assert_eq!(undo_len(&state), k, "{key:?} without a selection makes no entry");
        assert_eq!(state.status_hint, crate::sticker::DELETE_HINT);
        let y = 4.0 + 10.0 * k as f32;
        select_rect(&mut state, [0.0, y], [8.0, y + 6.0]);
        let actions = key_actions(&ctx, &mut state, key);
        run_dispatched(&mut state, actions);
        assert_eq!((undo_len(&state), label(&state).as_deref()), (k + 1, Some(DELETE_SELECTED_LABEL)), "{key:?}");
        assert_eq!(layer_pixels(&state, stamp, 0).get(4, y as u32 + 2)[3], 0, "{key:?}: the selected pixels went");
        assert!(doc(&state).stamp(stamp).is_some(), "{key:?}: the stamp stays");
        assert_eq!(state.session.as_ref().unwrap().selected_stamp, Some(stamp));
        run_in(&mut state, StickerAction::SelectNone);
    }
    assert_eq!(doc(&state).set(wte_core::TextureSetId(0)).map(|s| s.layers.len()), layers_before, "no texture layer went");
}

/// The actions the shortcut table makes of one key press in the Sticker view (no text field focused).
fn key_actions(ctx: &egui::Context, state: &mut AppState, key: egui::Key) -> Vec<AppAction> {
    let mut out = Vec::new();
    let input = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(NONE), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: NONE }],
        ..Default::default()
    };
    crate::theme::run_test_pass(ctx, input, |ui| out = crate::shortcuts::handle(ui.ctx(), state));
    out
}

/// Runs what `dispatch` makes of them in the Sticker view (`DeleteSelection` is the Sticker view's Delete key).
fn run_dispatched(state: &mut AppState, actions: Vec<AppAction>) {
    assert_eq!(state.view_mode, ViewMode::Sticker);
    for a in actions {
        match crate::dispatch::in_sticker_view(a) {
            AppAction::Sticker(s) => run_in(state, s),
            other => panic!("unexpected action {other:?}"),
        }
    }
}

/// A popover closes when another layer becomes active; a pixel popover needs a picture layer, Grow a selection.
#[test]
fn k7b_popovers_belong_to_their_layer() {
    let ctx = egui::Context::default();
    let (mut state, _) = opened(&ctx);
    run_in(&mut state, StickerAction::OpenPopover(PopKind::Modify { how: crate::sticker::commands::Modify::Grow, px: 3.0 }));
    assert!(state.sticker_ui.popover.is_none() && state.status_hint == crate::sticker::commands::NO_SELECTION_HINT);
    run_in(&mut state, StickerAction::OpenPopover(PopKind::Sharpen { amount: 1.0, radius: 2.0 }));
    assert!(state.sticker_ui.popover.is_some());
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    crate::sticker::validate_in(&mut state);
    assert!(state.sticker_ui.popover.is_none(), "another active layer closes it");
    let _ = &ctx;
}

// ---- outline, command list, sharing ----

/// The outline of a box is four runs, computed once per selection; a noisy selection is drawn as its bounds.
#[test]
fn k7b_the_outline_is_merged_runs_computed_once() {
    let s = Selection::rect(10, 10, v(2.0, 2.0), v(6.0, 5.0));
    let o = outline_of(&s);
    assert_eq!(o.h, vec![[2, 2, 6], [5, 2, 6]]);
    assert_eq!(o.v, vec![[2, 2, 5], [6, 2, 5]]);
    assert!(!o.boxed);
    let mut cache = OutlineCache::default();
    let a = Arc::new(s.clone());
    cache.get(&a);
    cache.get(&a);
    assert_eq!(cache.computed, 1);
    cache.get(&Arc::new(s));
    assert_eq!(cache.computed, 2, "another selection (even an equal one) is computed again");
    let mut noisy = wte_core::AlphaBuf::new(600, 600);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for v in noisy.data.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *v = if seed & 1 == 0 { 255 } else { 0 };
    }
    let o = outline_of(&Selection::from_mask(noisy));
    assert!(o.boxed && o.h.is_empty() && o.bounds == IRect::new(0, 0, 600, 600));
    let faint = Selection::from_mask(wte_core::AlphaBuf::filled(8, 8, 60));
    assert!(outline_of(&faint).boxed, "no edge at 128: the bounds");
}

/// The command list: what each needs (a selection, a picture layer), the same for the menu and the Tool tab.
#[test]
fn k7b_the_command_list_follows_what_the_editor_has() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let find = |label: &str| COMMANDS.iter().find(|d| d.label == label).unwrap();
    let have = available(&mut state);
    assert_eq!(have, (true, false, true));
    assert!(enabled(find("All"), have) && enabled(find("Blur..."), have));
    assert!(!enabled(find("Delete"), have) && !enabled(find("Crop to Selection"), have));
    select_rect(&mut state, [0.0, 0.0], [8.0, 8.0]);
    let have = available(&mut state);
    assert!(enabled(find("Delete"), have) && enabled(find("Layer via Cut"), have));
    // A text layer: no pixel command.
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    let id = sticker_id(&state, stamp);
    let spec = wte_core::TextSpec::new("Hi", state.doc_mut().unwrap().default_font());
    state
        .doc_mut()
        .unwrap()
        .set_sticker(
            wte_core::StickerRef::Id(id),
            move |s| {
                let lid = s.alloc_layer();
                s.layers.push(wte_core::StickerLayer::new(lid, "text", StickerContent::Text(Box::new(spec))));
            },
            "Add Text",
            wte_core::Commit::Final,
        )
        .unwrap();
    let text = sticker_of(&state, stamp).layers.last().unwrap().id;
    run_in(&mut state, StickerAction::SelectLayer(text));
    let have = available(&mut state);
    assert!(!enabled(find("Blur..."), have) && !enabled(find("Delete"), have) && enabled(find("Invert"), have));
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::Blur(2.0));
    assert_eq!((undo_len(&state), state.status_hint.as_str()), (undo, crate::sticker::tools::RASTERIZE_HINT));
    // Every command's index fits its panel id.
    assert!(COMMANDS.len() <= u8::MAX as usize);
}

/// A content edit of a picture two stamps show asks the shared-picture question first (no entry); a selection does not.
#[test]
fn k7b_cut_out_on_a_shared_picture_asks_first() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let doc = state.doc_mut().unwrap();
    let StampSource::Image { blob } = doc.stamp(stamp).unwrap().source.clone() else { panic!() };
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.3, 0.4), glam::Vec3::Z, glam::Vec3::Y, Vec2::splat(0.2), 0.6);
    doc.add_stamp("Disc 2", StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    doc.history.clear();
    select_rect(&mut state, [0.0, 0.0], [8.0, 8.0]);
    assert!(matches!(state.dialog, Dialog::None), "a selection is no content edit");
    for action in [StickerAction::DeleteKey, StickerAction::MagicErase([2.0, 2.0]), StickerAction::RemoveBackground, StickerAction::LayerViaCut] {
        state.dialog = Dialog::None;
        run_in(&mut state, action.clone());
        assert!(matches!(state.dialog, Dialog::StickerShared { users: 2 }), "{action:?}");
        assert_eq!(undo_len(&state), 0, "{action:?}");
    }
    run_in(&mut state, StickerAction::AnswerShared(Some(wte_core::ShareScope::ThisStamp)));
    assert_eq!(label(&state).as_deref(), Some(LAYER_VIA_CUT_LABEL), "the waiting edit ran after the answer");
    let _ = (&ctx, StickerLayerId(1));
}
