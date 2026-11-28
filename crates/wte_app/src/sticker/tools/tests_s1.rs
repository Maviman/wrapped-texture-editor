//! Tests of the Cut Out Box tool (stickers S1; STICKER_PLAN 7 "S1", 4.5 "GrabCut box"): the canvas drag starts the
//! cut on a job and the result lands as one entry "Cut Out Box" (an image stamp promoted inside it); "Into mask";
//! the selection limits it; a click cuts nothing; Cancel stops a running cut; a new box while one runs waits for its
//! thread (at most one); a layer changed meanwhile drops the result; leaving the view stops it; the refusals; the box
//! of a scaled layer in its content pixels.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use eframe::egui;
use glam::Vec2;
use wte_core::{Document, IRect, LayerTransform, PixelBuf, StampId, StampSource, StickerId};

use crate::action::AppAction;
use crate::state::{AppState, StickerTool, ViewMode};
use crate::sticker::canvas::{Geom, Hit, handle_input, refresh};
use crate::sticker::tests::CANVAS;
use crate::sticker::tools::cut_box::{CHANGED_NOTE, CUT_BOX_LABEL, content_rect, drag_box, pending};
use crate::sticker::{StickerAction, edit_mut, edit_of, enter_in, run_in};

const SIDE: u32 = 96;
const GEOM: Geom = Geom { origin: Vec2::ZERO, size: [SIDE, SIDE] };
const ORANGE: [u8; 3] = [235, 125, 30];

fn doc(state: &AppState) -> &Document {
    state.doc().unwrap()
}

fn undo_len(state: &AppState) -> usize {
    doc(state).history.undo_len()
}

fn label(state: &AppState) -> Option<String> {
    doc(state).undo_label().map(str::to_owned)
}

/// A small photo: an orange ball (radius 28, soft edge) on noisy grass.
fn photo() -> PixelBuf {
    let mut p = PixelBuf::new(SIDE, SIDE);
    let mut seed = 0x1234_5678u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 1000) as f32 / 1000.0
    };
    for y in 0..SIDE {
        for x in 0..SIDE {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::splat(48.0));
            let k = (28.5 - d).clamp(0.0, 1.0);
            let blotch = (((x / 12) * 7 + (y / 12) * 3) % 5) as f32 / 5.0;
            let g = [40.0 + 30.0 * blotch + 30.0 * rnd(), 100.0 + 50.0 * blotch + 30.0 * rnd(), 30.0 + 20.0 * rnd()];
            let rgb: [u8; 3] = std::array::from_fn(|c| (g[c] * (1.0 - k) + ORANGE[c] as f32 * k).round().clamp(0.0, 255.0) as u8);
            p.set(x, y, [rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    p
}

/// The sphere sample with an image stamp of `photo()` (selected), open in the Sticker view on the Cut Out Box.
fn opened(ctx: &egui::Context) -> (AppState, StampId) {
    let (mut state, _, _) = crate::sticker::tests::image_stamp_state(ViewMode::View3D);
    let img = photo();
    let png = wte_core::encode_png(SIDE, SIDE, &img.data, wte_core::PngLevel::Fast).unwrap();
    let doc = state.doc_mut().unwrap();
    let blob = doc.add_image_blob("ball.png", Arc::from(png)).unwrap();
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.0, 0.5), glam::Vec3::Z, glam::Vec3::Y, Vec2::splat(0.3), 0.6);
    let stamp = doc.add_stamp("Ball", StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    doc.history.clear();
    state.session.as_mut().unwrap().selected_stamp = Some(stamp);
    enter_in(&mut state, Some(stamp));
    edit_mut(&mut state).unwrap().camera.fit(GEOM.size, CANVAS);
    edit_mut(&mut state).unwrap().tool = StickerTool::CutOutBox;
    refresh(ctx, &mut state);
    (state, stamp)
}

fn run_queued(state: &mut AppState) {
    for action in std::mem::take(&mut state.actions) {
        match action {
            AppAction::Sticker(a) => run_in(state, a),
            other => panic!("unexpected action {other:?}"),
        }
    }
}

fn pass(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) {
    state.pen.frame = crate::pen::PenFrame::default();
    let input = egui::RawInput {
        events,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        ..Default::default()
    };
    crate::theme::run_test_pass(ctx, input, |ui| handle_input(ui, state, CANVAS, GEOM, Hit::Image, true));
    run_queued(state);
    refresh(ctx, state);
}

fn screen(state: &AppState, p: Vec2) -> egui::Pos2 {
    GEOM.to_screen(&edit_of(state).unwrap().camera, CANVAS, p)
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// A press at `a`, a move to the middle, a release at `b` (layer space).
fn drag(ctx: &egui::Context, state: &mut AppState, a: Vec2, b: Vec2) {
    let (pa, pm, pb) = (screen(state, a), screen(state, (a + b) * 0.5), screen(state, b));
    pass(ctx, state, vec![egui::Event::PointerMoved(pa), button(pa, true)]);
    pass(ctx, state, vec![egui::Event::PointerMoved(pm)]);
    pass(ctx, state, vec![egui::Event::PointerMoved(pb), button(pb, false)]);
}

/// Frames until no cut runs or waits (bounded).
fn settle(ctx: &egui::Context, state: &mut AppState) {
    let start = Instant::now();
    loop {
        refresh(ctx, state);
        if !pending(state) && state.sticker_ui.cut_threads.alive.load(Ordering::Acquire) == 0 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "the cut never ended");
        std::thread::sleep(Duration::from_millis(2));
    }
    refresh(ctx, state);
}

fn sticker_id(state: &AppState, stamp: StampId) -> StickerId {
    match doc(state).stamp(stamp).unwrap().source {
        StampSource::Sticker { sticker } => sticker,
        ref other => panic!("not a sticker stamp: {other:?}"),
    }
}

/// Layer 0 of the stamp's sticker: its pixels and mask.
fn layer0(state: &mut AppState, stamp: StampId) -> (PixelBuf, Option<wte_core::AlphaBuf>) {
    let id = sticker_id(state, stamp);
    let d = Arc::clone(doc(state).sticker(id).unwrap());
    let l = &d.layers[0];
    let p = match l.owned_picture() {
        Some(p) => (**p).clone(),
        // Promoted with only a mask: the picture is still the image blob.
        None => (*state.doc_mut().unwrap().sticker_layer_pixels(wte_core::StickerRef::Id(id), l.id).unwrap()).clone(),
    };
    (p, l.mask.as_ref().map(|m| (*m.pixels).clone()))
}

fn alpha(p: &PixelBuf, x: u32, y: u32) -> u8 {
    p.get(x, y)[3]
}

fn is_image(state: &AppState, stamp: StampId) -> bool {
    matches!(doc(state).stamp(stamp).unwrap().source, StampSource::Image { .. })
}

#[test]
fn a_box_drag_cuts_the_object_out_as_one_entry_and_promotes_the_image_stamp() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    drag(&ctx, &mut state, Vec2::new(10.0, 10.0), Vec2::new(86.0, 86.0));
    assert!(state.sticker_ui.drag.is_none());
    settle(&ctx, &mut state);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(CUT_BOX_LABEL)));
    assert!(!is_image(&state, stamp), "promoted inside the entry");
    let (p, mask) = layer0(&mut state, stamp);
    assert!(mask.is_none());
    assert_eq!(alpha(&p, 2, 2), 0, "outside the box");
    assert_eq!(alpha(&p, 20, 20), 0, "grass inside the box");
    assert_eq!(alpha(&p, 48, 48), 255, "the ball");
    assert_eq!(alpha(&p, 48, 25), 255, "the ball near its top");
    let kept = p.data.as_chunks::<4>().0.iter().filter(|c| c[3] >= 128).count() as f64;
    let ball = std::f64::consts::PI * 28.5 * 28.5;
    assert!((kept - ball).abs() / ball < 0.1, "kept {kept}, the ball {ball:.0}");
    assert_eq!(state.sticker_ui.cut_threads.started, 1);
    assert!(state.status_hint.is_empty(), "{}", state.status_hint);
    // Undo: the image stamp again.
    assert!(state.doc_mut().unwrap().undo().is_some());
    assert!(is_image(&state, stamp));
}

#[test]
fn into_mask_keeps_the_pixels_and_adds_a_mask() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    state.sticker_ui.cut_box.into_mask = true;
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    settle(&ctx, &mut state);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(CUT_BOX_LABEL)));
    let (p, mask) = layer0(&mut state, stamp);
    assert_eq!(p, photo(), "the pixels are kept");
    let mask = mask.expect("a mask was added");
    assert_eq!((mask.get(2, 2), mask.get(48, 48)), (0, 255));
    // Undone, then a layer that has a mask (all shown): the cut changes that mask (the other path).
    assert!(state.doc_mut().unwrap().undo().is_some());
    run_in(&mut state, StickerAction::AddMask);
    let n = undo_len(&state);
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    settle(&ctx, &mut state);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (n + 1, Some(CUT_BOX_LABEL)));
    let (p, again) = layer0(&mut state, stamp);
    assert_eq!(p, photo());
    assert_eq!(again.unwrap(), mask, "the same cut into the mask it had");
}

#[test]
fn the_selection_limits_the_cut() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    run_in(&mut state, StickerAction::SelectRect([0.0, 0.0, 48.0, SIDE as f32]));
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    settle(&ctx, &mut state);
    assert_eq!(label(&state).as_deref(), Some(CUT_BOX_LABEL));
    let (p, _) = layer0(&mut state, stamp);
    assert_eq!(alpha(&p, 5, 5), 0, "selected: cleared");
    assert_eq!(alpha(&p, 90, 5), 255, "not selected: kept");
    assert_eq!(alpha(&p, 60, 48), 255, "the ball");
}

#[test]
fn a_click_cuts_nothing() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    drag(&ctx, &mut state, Vec2::new(40.0, 40.0), Vec2::new(40.5, 41.0));
    settle(&ctx, &mut state);
    assert_eq!(undo_len(&state), 0);
    assert_eq!(state.sticker_ui.cut_threads.started, 0);
    assert!(is_image(&state, stamp));
    assert_eq!(drag_box(Vec2::new(5.0, 5.0), Vec2::new(1.0, 30.0)), Some([1.0, 5.0, 5.0, 30.0]));
    assert_eq!(drag_box(Vec2::new(5.0, 5.0), Vec2::new(6.0, 30.0)), None);
    assert_eq!(drag_box(Vec2::new(f32::NAN, 5.0), Vec2::new(60.0, 30.0)), None);
}

#[test]
fn cancel_stops_a_running_cut_and_nothing_lands() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let hold = Arc::clone(&state.sticker_ui.cut_threads.hold);
    hold.store(true, Ordering::Release);
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    assert!(pending(&state) && crate::sticker::live::pending(&state), "the idle check waits for it");
    refresh(&ctx, &mut state);
    assert!(state.sticker_ui.cut_job.is_some(), "the job runs");
    assert!(crate::sticker::tools::cut_box::progress(&state).is_some());
    run_in(&mut state, StickerAction::CancelCutOutBox);
    assert!(!pending(&state));
    assert_eq!(state.status_hint, wte_core::BoxCutRefusal::Cancelled.message());
    hold.store(false, Ordering::Release);
    settle(&ctx, &mut state);
    assert_eq!(undo_len(&state), 0);
    assert!(is_image(&state, stamp));
}

#[test]
fn a_new_box_while_one_runs_waits_for_its_thread_and_only_the_last_lands() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let hold = Arc::clone(&state.sticker_ui.cut_threads.hold);
    hold.store(true, Ordering::Release);
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.cut_threads.alive.load(Ordering::Acquire), 1);
    // The second box: the first is cancelled; the second waits for its thread.
    run_in(&mut state, StickerAction::CutOutBox([14.0, 14.0, 82.0, 82.0]));
    refresh(&ctx, &mut state);
    refresh(&ctx, &mut state);
    assert!(state.sticker_ui.cut_job.is_none() && pending(&state), "queued");
    assert_eq!(state.sticker_ui.cut_threads.started, 1);
    hold.store(false, Ordering::Release);
    settle(&ctx, &mut state);
    assert_eq!(state.sticker_ui.cut_threads.started, 2);
    assert_eq!(state.sticker_ui.cut_threads.most_alive, 1, "never two cut threads");
    assert!(state.status_hint.is_empty(), "{}", state.status_hint);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(CUT_BOX_LABEL)));
    let (p, _) = layer0(&mut state, stamp);
    assert_eq!((alpha(&p, 12, 12), alpha(&p, 48, 48)), (0, 255));
}

#[test]
fn a_layer_changed_while_cutting_drops_the_result() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let hold = Arc::clone(&state.sticker_ui.cut_threads.hold);
    hold.store(true, Ordering::Release);
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    refresh(&ctx, &mut state);
    assert!(state.sticker_ui.cut_job.is_some());
    // Meanwhile the user fills the grass (one entry, the stamp promoted).
    state.colors.primary = [250, 250, 10, 255];
    run_in(&mut state, StickerAction::Fill([2.0, 2.0]));
    assert_eq!(label(&state).as_deref(), Some("Fill"));
    hold.store(false, Ordering::Release);
    settle(&ctx, &mut state);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some("Fill")), "no cut entry");
    assert_eq!(state.status_hint, CHANGED_NOTE);
    assert!(!is_image(&state, stamp));
}

#[test]
fn leaving_the_view_stops_the_cut() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    let hold = Arc::clone(&state.sticker_ui.cut_threads.hold);
    hold.store(true, Ordering::Release);
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    refresh(&ctx, &mut state);
    assert!(state.sticker_ui.cut_job.is_some());
    run_in(&mut state, StickerAction::Leave);
    assert!(edit_of(&state).is_none() && !pending(&state));
    hold.store(false, Ordering::Release);
    let start = Instant::now();
    while state.sticker_ui.cut_threads.alive.load(Ordering::Acquire) > 0 {
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(undo_len(&state), 0);
    assert!(is_image(&state, stamp));
}

#[test]
fn refusals_say_why_and_change_nothing() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    // A box over 2 x 2 picture pixels (bigger than a click on the canvas, smaller than the cut needs).
    run_in(&mut state, StickerAction::CutOutBox([40.0, 40.0, 42.5, 42.5]));
    settle(&ctx, &mut state);
    assert_eq!(state.status_hint, wte_core::BoxCutRefusal::BoxTooSmall.message());
    assert_eq!(state.sticker_ui.cut_threads.started, 0);
    // A box outside the picture.
    run_in(&mut state, StickerAction::CutOutBox([200.0, 200.0, 300.0, 300.0]));
    settle(&ctx, &mut state);
    assert_eq!(state.status_hint, wte_core::BoxCutRefusal::BoxTooSmall.message());
    assert_eq!(undo_len(&state), 0);
    assert!(is_image(&state, stamp));
    // A text layer offers Rasterize (the paint tools' rule) and starts nothing.
    run_in(&mut state, StickerAction::AddTextLayer(None));
    let n = undo_len(&state);
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    settle(&ctx, &mut state);
    assert_eq!(undo_len(&state), n);
    assert_eq!(state.status_hint, crate::sticker::tools::RASTERIZE_HINT);
    assert_eq!(state.sticker_ui.cut_threads.started, 0);
}

#[test]
fn the_box_of_a_moved_and_scaled_layer_is_taken_in_its_content_pixels() {
    let ctx = egui::Context::default();
    let (mut state, _) = opened(&ctx);
    let mut t = crate::sticker::tools::paint_target(&mut state, StickerTool::CutOutBox, false).unwrap();
    assert_eq!(content_rect(&t, [10.2, 10.0, 86.0, 85.5]), IRect::new(10, 10, 86, 86));
    t.transform = LayerTransform { translate: Vec2::new(100.0, 50.0), scale: Vec2::splat(2.0), rotation: 0.0 };
    // Scaled about the content centre (48, 48) and moved: layer point p <- content c = (p - 100 - 48) / 2 + 48 ... as
    // the transform says; the box's corners map through `to_content`.
    let r = content_rect(&t, [100.0, 50.0, 196.0, 146.0]);
    let a = t.to_content(Vec2::new(100.0, 50.0));
    let b = t.to_content(Vec2::new(196.0, 146.0));
    assert_eq!(r, IRect::new(a.x.floor() as i32, a.y.floor() as i32, b.x.ceil() as i32, b.y.ceil() as i32));
    assert_eq!(r.width(), 48);
    assert!(matches!(state.doc_mut().unwrap().sticker_layer_pixels(t.on, t.layer).map(|p| p.width), Ok(SIDE)));
}

#[test]
fn a_result_waits_for_a_running_drag_to_end() {
    let ctx = egui::Context::default();
    let (mut state, _) = opened(&ctx);
    let hold = Arc::clone(&state.sticker_ui.cut_threads.hold);
    hold.store(true, Ordering::Release);
    run_in(&mut state, StickerAction::CutOutBox([10.0, 10.0, 86.0, 86.0]));
    refresh(&ctx, &mut state);
    // A pan starts while the cut runs.
    state.sticker_ui.drag = Some(crate::sticker::CanvasDrag::Pan { last: egui::Pos2::ZERO, button: egui::PointerButton::Secondary });
    hold.store(false, Ordering::Release);
    let start = Instant::now();
    while state.sticker_ui.cut_threads.alive.load(Ordering::Acquire) > 0 {
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(2));
    }
    for _ in 0..3 {
        refresh(&ctx, &mut state);
    }
    assert_eq!(undo_len(&state), 0, "not during the drag");
    assert!(pending(&state));
    state.sticker_ui.drag = None;
    refresh(&ctx, &mut state);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (1, Some(CUT_BOX_LABEL)));
    assert!(!pending(&state));
}
