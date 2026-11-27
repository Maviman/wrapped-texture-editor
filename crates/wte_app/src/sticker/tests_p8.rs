//! Tests of stickers P8 (STICKER_PLAN 7 "P8", 3.2, 3.3, 3.8): the Move tool's transform handles and numbers, the
//! Crop, Text and Shape tools, the whole-sticker commands, Merge Down, Clip to Layer Below, adjustments, text layers
//! through the Text block's target, the library and Export as PSD. Every user action is one undo step; numeric fields
//! take 0, negative, huge and non-numeric values without breaking anything.

use std::path::PathBuf;

use eframe::egui;
use glam::Vec2;
use wte_core::{Adjustment, ArtboardShape, Commit, LayerTransform, StampSource, StickerContent, StickerLayerId};

use super::layout::{Layout, LayerTransformCmd};
use super::tests::{CANVAS, canvas_state, dispatch_queued, image_stamp_state};
use super::*;
use crate::state::{StickerTool, ViewMode};
use crate::sticker::canvas::Hit;

fn undo_len(state: &AppState) -> usize {
    state.doc().unwrap().history.undo_len()
}

fn label(state: &AppState) -> Option<String> {
    state.doc().unwrap().undo_label().map(str::to_owned)
}

/// The sticker the open editor shows (promoted or not).
fn shown_doc(state: &mut AppState) -> StickerDoc {
    layered(state, None).map(|(d, _)| d).or_else(|| {
        let e = edit_of(state)?.clone();
        shown(&mut state.session.as_mut()?.doc, &e)?.layers().cloned()
    }).expect("a layered sticker")
}

fn press(pos: egui::Pos2, pressed: bool, mods: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods }
}

/// One egui pass of the canvas with `events` and `mods` held, then the queued commands.
fn pass(ctx: &egui::Context, state: &mut AppState, geom: crate::sticker::canvas::Geom, hit: Hit, events: Vec<egui::Event>, mods: egui::Modifiers) {
    let mut all = vec![egui::Event::ModifiersChanged(mods)];
    all.extend(events);
    let input = egui::RawInput {
        events: all,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        ..Default::default()
    };
    crate::theme::run_test_pass(ctx, input, |ui| crate::sticker::canvas::handle_input(ui, state, CANVAS, geom, hit, true));
    dispatch_queued(state);
}

/// A primary drag on the canvas from `a` to `b` (layer space) in `steps` moves, with `mods` held.
fn drag(state: &mut AppState, a: Vec2, b: Vec2, steps: usize, mods: egui::Modifiers) {
    let ctx = egui::Context::default();
    let d = shown_doc(state);
    let geom = crate::sticker::canvas::Geom { origin: Vec2::new(d.artboard.x0 as f32, d.artboard.y0 as f32), size: d.size() };
    let cam = edit_of(state).unwrap().camera;
    let hit = match state.doc().unwrap().stamp(stamp_of(state)).unwrap().source {
        StampSource::Sticker { sticker } => Hit::Sticker(sticker),
        _ => Hit::Image,
    };
    let at = |p: Vec2| geom.to_screen(&cam, CANVAS, p);
    pass(&ctx, state, geom, hit, vec![egui::Event::PointerMoved(at(a)), press(at(a), true, mods)], mods);
    for k in 1..=steps {
        let p = a + (b - a) * (k as f32 / steps as f32);
        pass(&ctx, state, geom, hit, vec![egui::Event::PointerMoved(at(p))], mods);
    }
    pass(&ctx, state, geom, hit, vec![press(at(b), false, mods)], mods);
}

fn stamp_of(state: &AppState) -> wte_core::StampId {
    match edit_of(state).unwrap().target {
        crate::state::StickerTarget::Stamp(s) => s,
        crate::state::StickerTarget::Tray(_) => panic!("a tray sticker"),
    }
}

fn set_tool(state: &mut AppState, tool: StickerTool) {
    edit_mut(state).unwrap().tool = tool;
}

/// Transform handles: a corner drag scales keeping the aspect (one entry, the image stamp promoted inside it), Shift
/// frees it; the rotate handle turns in 15-degree steps with Shift; the opposite corner stays put.
#[test]
fn k8_transform_handles_scale_and_turn_in_one_entry_each() {
    let (mut state, stamp, _) = canvas_state();
    let undo = undo_len(&state);
    // The top-left corner (0, 0) dragged to (16, 20): keep aspect -> the projection onto the diagonal (0.71875).
    drag(&mut state, Vec2::new(0.0, 0.0), Vec2::new(16.0, 20.0), 3, egui::Modifiers::NONE);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (undo + 1, Some(tools::transform::TRANSFORM_LABEL)));
    assert!(matches!(state.doc().unwrap().stamp(stamp).unwrap().source, StampSource::Sticker { .. }), "promoted inside the entry");
    let t = shown_doc(&mut state).layers[0].transform;
    assert!((t.scale.x - t.scale.y).abs() < 1e-4 && (t.scale.x - 0.71875).abs() < 0.01, "{t:?}");
    let b = wte_core::LayerBox::centred(Vec2::splat(64.0));
    assert!((b.to_layer(&t, Vec2::splat(64.0)) - Vec2::splat(64.0)).abs().max_element() < 0.01, "the opposite corner stays");
    // Shift frees the aspect.
    let c = b.to_layer(&t, Vec2::ZERO);
    drag(&mut state, c, c - Vec2::new(10.0, 0.0), 2, egui::Modifiers::SHIFT);
    let t2 = shown_doc(&mut state).layers[0].transform;
    assert!((t2.scale.y - t.scale.y).abs() < 1e-4 && t2.scale.x > t.scale.x, "{t2:?}");
    assert_eq!(undo_len(&state), undo + 2);
    // The rotate handle: above the top edge's middle.
    let top = b.to_layer(&t2, Vec2::new(32.0, 0.0));
    let cam = edit_of(&state).unwrap().camera;
    let up = 22.0 / cam.zoom;
    let handle = top - Vec2::new(0.0, up);
    let centre = b.centre(&t2);
    let r = (handle - centre).length();
    let to = centre + Vec2::new(r * 0.30_f32.sin(), -r * 0.30_f32.cos());
    drag(&mut state, handle, to, 3, egui::Modifiers::SHIFT);
    let t3 = shown_doc(&mut state).layers[0].transform;
    assert!((t3.rotation - 15f32.to_radians()).abs() < 1e-4, "17 degrees stepped to 15: {}", t3.rotation.to_degrees());
    assert!((b.centre(&t3) - centre).abs().max_element() < 0.01, "turned about the box centre");
    assert_eq!(undo_len(&state), undo + 3);
}

/// The handle maths alone: a drag past the fixed corner flips; zero and NaN never reach the transform.
#[test]
fn k8_handle_maths_flips_and_never_makes_a_zero_scale() {
    use tools::transform::{Handle, dragged, reset, size_of, with_size};
    let b = wte_core::LayerBox::centred(Vec2::new(40.0, 20.0));
    let t = LayerTransform::IDENTITY;
    let flipped = dragged(&b, &t, Handle::Scale(1, 0), Vec2::new(40.0, 10.0), Vec2::new(-20.0, 10.0), false, true);
    assert!(flipped.scale.x < 0.0 && (flipped.scale.x + 0.5).abs() < 1e-4, "{flipped:?}");
    let zero = dragged(&b, &t, Handle::Scale(1, 1), Vec2::new(40.0, 20.0), Vec2::ZERO, false, true);
    assert!(zero.scale.x.abs() >= 1e-3 && zero.scale.y.abs() >= 1e-3, "{zero:?}");
    let nan = dragged(&b, &t, Handle::Scale(1, 1), Vec2::new(40.0, 20.0), Vec2::NAN, false, true);
    assert!(nan.scale.is_finite() && nan.translate.is_finite());
    assert_eq!(dragged(&b, &t, Handle::Rotate, Vec2::new(20.0, -5.0), Vec2::new(20.0, 10.0), false, true), t, "on the centre: no turn");
    // The W / H fields: 0, negative, NaN and huge.
    for bad in [0.0, -5.0, f32::NAN, f32::INFINITY] {
        assert_eq!(with_size(&b, &t, Vec2::new(bad, 20.0)).scale.x, 1.0, "{bad}");
    }
    let huge = with_size(&b, &t, Vec2::new(1.0e9, 20.0));
    assert_eq!(huge.scale.x, 1.0e3);
    let s = with_size(&b, &t, Vec2::new(80.0, 10.0));
    assert_eq!(size_of(&b, &s), Vec2::new(80.0, 10.0));
    assert!((b.centre(&s) - b.centre(&t)).abs().max_element() < 1e-4);
    let back = reset(&b, &wte_core::LayerTransform { rotation: 1.0, scale: Vec2::new(-2.0, 3.0), translate: Vec2::new(5.0, 5.0) });
    assert_eq!((back.scale, back.rotation), (Vec2::ONE, 0.0));
}

/// Flip H / V and Reset of a layer, SetLayerTransform with values that are not numbers, a locked layer.
#[test]
fn k8_layer_flip_reset_and_bad_numbers() {
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::LayerTransform(LayerTransformCmd::FlipH));
    let t = shown_doc(&mut state).layers[0].transform;
    assert_eq!((t.scale, undo_len(&state)), (Vec2::new(-1.0, 1.0), undo + 1));
    run_in(&mut state, StickerAction::LayerTransform(LayerTransformCmd::FlipV));
    run_in(&mut state, StickerAction::LayerTransform(LayerTransformCmd::Reset));
    assert_eq!(shown_doc(&mut state).layers[0].transform, LayerTransform::IDENTITY);
    assert_eq!(undo_len(&state), undo + 3);
    let id = shown_doc(&mut state).layers[0].id;
    let bad = LayerTransform { translate: Vec2::new(f32::NAN, 2.0), scale: Vec2::new(0.0, f32::INFINITY), rotation: f32::NAN };
    run_in(&mut state, StickerAction::SetLayerTransform(id, bad, Commit::Final));
    let t = shown_doc(&mut state).layers[0].transform;
    assert!(t.translate.is_finite() && t.scale.is_finite() && t.rotation.is_finite(), "{t:?}");
    run_in(&mut state, StickerAction::SetLayerLocked(id, true));
    let n = undo_len(&state);
    run_in(&mut state, StickerAction::LayerTransform(LayerTransformCmd::FlipH));
    run_in(&mut state, StickerAction::SetLayerTransform(id, LayerTransform::moved(3.0, 3.0), Commit::Final));
    assert_eq!(undo_len(&state), n, "a locked layer stays");
}

/// The Crop tool: a drag on the right edge, then Enter: one entry; the artboard shape from the options; Esc drops the
/// box; Delete cropped pixels cuts the picture.
#[test]
fn k8_crop_tool_drags_the_box_and_enter_applies_it() {
    let (mut state, stamp, _) = canvas_state();
    set_tool(&mut state, StickerTool::Crop);
    let undo = undo_len(&state);
    drag(&mut state, Vec2::new(64.0, 32.0), Vec2::new(40.0, 32.0), 3, egui::Modifiers::NONE);
    assert_eq!(state.sticker_ui.crop, Some([0.0, 0.0, 40.0, 64.0]));
    assert_eq!(undo_len(&state), undo, "the box is app state");
    state.sticker_ui.crop_opts.shape = tools::crop::CropShape::Ellipse;
    run_in(&mut state, StickerAction::EnterKey);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (undo + 1, Some(wte_core::CROP_LABEL)));
    let d = shown_doc(&mut state);
    assert_eq!((d.size(), d.shape), ([40, 64], ArtboardShape::Ellipse));
    assert!(state.sticker_ui.crop.is_none());
    assert!(matches!(state.doc().unwrap().stamp(stamp).unwrap().source, StampSource::Sticker { .. }));
    // A new box outside, then Cancel.
    drag(&mut state, Vec2::new(-10.0, 10.0), Vec2::new(-2.0, 30.0), 2, egui::Modifiers::NONE);
    assert_eq!(state.sticker_ui.crop, Some([-10.0, 10.0, -2.0, 30.0]));
    run_in(&mut state, StickerAction::CancelCrop);
    assert!(state.sticker_ui.crop.is_none());
    // Delete cropped pixels: the picture keeps only what is inside.
    state.sticker_ui.crop = Some([0.0, 0.0, 20.0, 20.0]);
    state.sticker_ui.crop_opts = tools::crop::CropOptions { delete_pixels: true, ..Default::default() };
    run_in(&mut state, StickerAction::ApplyCrop);
    let d = shown_doc(&mut state);
    let p = d.layers[0].owned_picture().expect("cut pixels");
    assert_eq!(([p.width, p.height], d.size(), d.shape), ([20, 20], [20, 20], ArtboardShape::Rect));
    // Bad boxes: nothing.
    let n = undo_len(&state);
    state.sticker_ui.crop = Some([f32::NAN, 0.0, 5.0, 5.0]);
    run_in(&mut state, StickerAction::ApplyCrop);
    assert_eq!(undo_len(&state), n);
}

/// The Text tool: a click adds a text layer at the point (one entry, active, the content field asked for the focus);
/// a click on it selects it (no entry); the Text block's target is the layer; typing is one entry; the name follows.
#[test]
fn k8_text_tool_adds_and_selects_text_layers() {
    let (mut state, _, _) = canvas_state();
    set_tool(&mut state, StickerTool::Text);
    state.tools.text.content = "Hi".to_owned();
    let undo = undo_len(&state);
    drag(&mut state, Vec2::new(20.0, 30.0), Vec2::new(20.0, 30.0), 0, egui::Modifiers::NONE);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (undo + 1, Some(tools::text::ADD_TEXT_LABEL)));
    let d = shown_doc(&mut state);
    let layer = d.layers.last().unwrap().clone();
    let StickerContent::Text(spec) = &layer.content else { panic!("a text layer") };
    assert_eq!((spec.content.as_str(), spec.color, layer.transform.translate), ("Hi", state.colors.primary, Vec2::new(20.0, 30.0)));
    assert_eq!(spec.size_px, tools::text::default_em([64, 64]));
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(layer.id));
    assert!(state.sticker_ui.focus_text);
    assert_eq!(crate::panels::text_block::target(&state), Some(crate::panels::text_block::TextTarget::Layer(layer.id)));
    // Typing: a Preview then a Final are one entry; the layer's name follows its text.
    let mut new = (**spec).clone();
    new.content = "Hello".to_owned();
    run_in(&mut state, StickerAction::SetLayerText(layer.id, Box::new(new.clone()), Commit::Preview));
    new.content = "Hello!".to_owned();
    run_in(&mut state, StickerAction::SetLayerText(layer.id, Box::new(new), Commit::Final));
    assert_eq!(undo_len(&state), undo + 2);
    let d = shown_doc(&mut state);
    assert_eq!(d.layers.last().unwrap().name, "Hello!");
    // A click on one of its letters selects it.
    run_in(&mut state, StickerAction::SelectLayer(d.layers[0].id));
    let on = edit_ref(edit_of(&state).unwrap()).unwrap();
    let alone = state.doc_mut().unwrap().sticker_layer_on_artboard(on, layer.id).unwrap();
    let i = alone.data.as_chunks::<4>().0.iter().position(|p| p[3] == 255).expect("an opaque pixel of the text");
    let inside = Vec2::new((i as u32 % alone.width) as f32 + 0.5, (i as u32 / alone.width) as f32 + 0.5)
        + Vec2::new(d.artboard.x0 as f32, d.artboard.y0 as f32);
    drag(&mut state, inside, inside, 0, egui::Modifiers::NONE);
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(layer.id));
    assert_eq!(undo_len(&state), undo + 2, "selecting is no entry");
    // The draft target with no text layer active and the Text tool.
    run_in(&mut state, StickerAction::SelectLayer(d.layers[0].id));
    assert_eq!(crate::panels::text_block::target(&state), Some(crate::panels::text_block::TextTarget::Draft));
    set_tool(&mut state, StickerTool::Move);
    assert_eq!(crate::panels::text_block::target(&state), None);
}

/// The Shape tool: a drag adds a shape layer filling the box (one entry); its options then edit it (one entry per
/// change); the footer's Shape adds one in the middle.
#[test]
fn k8_shape_tool_adds_and_edits_shape_layers() {
    let (mut state, _, _) = canvas_state();
    set_tool(&mut state, StickerTool::Shape);
    state.sticker_ui.shape.kind = tools::shape::ShapeChoice::Star;
    let undo = undo_len(&state);
    drag(&mut state, Vec2::new(10.0, 10.0), Vec2::new(50.0, 40.0), 3, egui::Modifiers::NONE);
    assert_eq!((undo_len(&state), label(&state).as_deref()), (undo + 1, Some(tools::shape::ADD_SHAPE_LABEL)));
    let d = shown_doc(&mut state);
    let l = d.layers.last().unwrap();
    let StickerContent::Shape(s) = &l.content else { panic!("a shape layer") };
    assert_eq!((s.size, l.transform.translate), (Vec2::new(40.0, 30.0), Vec2::new(10.0, 10.0)));
    assert!(matches!(s.kind, wte_core::ShapeKind::Star { .. }));
    let mut edited = s.clone();
    edited.kind = wte_core::ShapeKind::Ellipse;
    run_in(&mut state, StickerAction::SetLayerShape(l.id, Box::new(edited), Commit::Final));
    assert_eq!((undo_len(&state), label(&state).as_deref()), (undo + 2, Some(tools::shape::EDIT_SHAPE_LABEL)));
    run_in(&mut state, StickerAction::AddShapeLayer(None));
    let d = shown_doc(&mut state);
    let StickerContent::Shape(s) = &d.layers.last().unwrap().content else { panic!() };
    assert_eq!(s.size, Vec2::new(32.0, 32.0));
    // SetLayerShape on a picture layer changes nothing.
    let n = undo_len(&state);
    run_in(&mut state, StickerAction::SetLayerShape(d.layers[0].id, Box::new(s.clone()), Commit::Final));
    assert!(matches!(shown_doc(&mut state).layers[0].content, StickerContent::Picture(_)));
    assert_eq!(undo_len(&state), n);
}

/// The whole-sticker commands through their actions: one entry each; Merge Down and Clip; the bottom layer refuses.
#[test]
fn k8_sticker_and_layer_commands_are_one_entry_each() {
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    let start = undo_len(&state);
    let steps: Vec<(StickerAction, &str)> = vec![
        (StickerAction::Layout(Layout::Padding(8)), wte_core::PADDING_LABEL),
        (StickerAction::Layout(Layout::Trim(2)), wte_core::TRIM_LABEL),
        (StickerAction::Layout(Layout::Rotate(90.0)), wte_core::ROTATE_LABEL),
        (StickerAction::Layout(Layout::Resize([136, 136])), wte_core::RESIZE_LABEL),
        (StickerAction::Layout(Layout::Flip(false)), wte_core::FLIP_V_LABEL),
        (StickerAction::Layout(Layout::CanvasSize([100, 120], [0.5, 0.5])), wte_core::CANVAS_SIZE_LABEL),
        (StickerAction::AddTextLayer(None), tools::text::ADD_TEXT_LABEL),
        (StickerAction::ToggleClip(None), layout::CLIP_LABEL),
        (StickerAction::MergeDown(None), wte_core::MERGE_DOWN_LABEL),
        (StickerAction::AddShapeLayer(None), tools::shape::ADD_SHAPE_LABEL),
        (StickerAction::Layout(Layout::Flatten), wte_core::FLATTEN_LABEL),
    ];
    for (k, (action, want)) in steps.into_iter().enumerate() {
        run_in(&mut state, action.clone());
        assert_eq!((undo_len(&state), label(&state).as_deref()), (start + k + 1, Some(want)), "{action:?}");
    }
    let d = shown_doc(&mut state);
    assert_eq!((d.layers.len(), d.size()), (1, [100, 120]));
    // The bottom layer: no Merge Down, no Clip.
    let n = undo_len(&state);
    run_in(&mut state, StickerAction::MergeDown(None));
    run_in(&mut state, StickerAction::ToggleClip(None));
    assert_eq!(undo_len(&state), n);
    // Numbers out of range: a resize below 1 % is refused (warning, no entry); a canvas of 0 is 1 px.
    run_in(&mut state, StickerAction::Layout(Layout::Resize([0, 0])));
    assert_eq!(undo_len(&state), n);
    run_in(&mut state, StickerAction::Layout(Layout::CanvasSize([0, u32::MAX], [f32::NAN, -3.0])));
    assert_eq!(shown_doc(&mut state).size(), [1, wte_core::MAX_ARTBOARD]);
    run_in(&mut state, StickerAction::Layout(Layout::Rotate(f32::NAN)));
    assert_eq!(undo_len(&state), n + 1);
    // Undo everything: the image stamp again.
    while undo_len(&state) > start {
        state.doc_mut().unwrap().undo();
    }
    assert!(matches!(state.doc().unwrap().stamp(stamp_of(&state)).unwrap().source, StampSource::Image { .. }));
}

/// The popovers of the commands with numbers open with the artboard's size and Apply runs the command.
#[test]
fn k8_resize_and_canvas_size_popovers_start_at_the_artboard() {
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    run_in(&mut state, StickerAction::OpenPopover(popover::PopKind::Resize { w: 0, h: 0, keep: true }));
    assert_eq!(state.sticker_ui.popover.as_ref().map(|p| p.kind.clone()), Some(popover::PopKind::Resize { w: 64, h: 64, keep: true }));
    if let Some(p) = state.sticker_ui.popover.as_mut() {
        p.kind = popover::PopKind::Resize { w: 128, h: 128, keep: true };
    }
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::ApplyPopover);
    assert_eq!((undo_len(&state), shown_doc(&mut state).size()), (undo + 1, [128, 128]));
    run_in(&mut state, StickerAction::OpenPopover(popover::PopKind::CanvasSize { w: 0, h: 0, anchor: [1, 1] }));
    assert_eq!(state.sticker_ui.popover.as_ref().map(|p| p.kind.clone()), Some(popover::PopKind::CanvasSize { w: 128, h: 128, anchor: [1, 1] }));
    assert!(matches!(
        popover::PopKind::CanvasSize { w: 10, h: 20, anchor: [9, 0] }.action(),
        StickerAction::Layout(Layout::CanvasSize([10, 20], [1.0, 0.0]))
    ));
}

/// Adjustments: added, dragged (Preview calls fold), removed: one entry each; values out of range are sanitized.
#[test]
fn k8_adjustments_are_one_entry_per_change() {
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    let id = shown_doc(&mut state).layers[0].id;
    let undo = undo_len(&state);
    let bc = |b: f32| Adjustment::BrightnessContrast { brightness: b, contrast: 0.0 };
    run_in(&mut state, StickerAction::SetLayerAdjustments(id, vec![bc(0.0)], Commit::Final));
    run_in(&mut state, StickerAction::SetLayerAdjustments(id, vec![bc(20.0)], Commit::Preview));
    run_in(&mut state, StickerAction::SetLayerAdjustments(id, vec![bc(500.0)], Commit::Final));
    assert_eq!((undo_len(&state), label(&state).as_deref()), (undo + 2, Some(adjust_ui::ADJUST_LABEL)));
    assert_eq!(shown_doc(&mut state).layers[0].adjustments, vec![bc(100.0)], "clamped");
    let nan = Adjustment::Levels { in_black: 0, in_white: 255, gamma: f32::NAN, out_black: 0, out_white: 255 };
    run_in(&mut state, StickerAction::SetLayerAdjustments(id, vec![nan; 40], Commit::Final));
    let list = shown_doc(&mut state).layers[0].adjustments.clone();
    assert_eq!(list.len(), adjust_ui::MAX_ADJUSTMENTS);
    assert!(matches!(list[0], Adjustment::Levels { gamma, .. } if gamma == 1.0));
    run_in(&mut state, StickerAction::SetLayerAdjustments(id, Vec::new(), Commit::Final));
    assert!(shown_doc(&mut state).layers[0].adjustments.is_empty());
    assert_eq!(undo_len(&state), undo + 4);
}

/// A content command of P8 on a sticker two stamps show asks the shared-picture question first.
#[test]
fn k8_layout_commands_ask_the_shared_question_first() {
    let (mut state, _, blob) = image_stamp_state(ViewMode::View3D);
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.3, 0.0, 0.4), glam::Vec3::Z, glam::Vec3::Y, Vec2::splat(0.2), 0.6);
    state.doc_mut().unwrap().add_stamp("Logo 2", StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    enter_in(&mut state, None);
    let undo = undo_len(&state);
    for action in [StickerAction::Layout(Layout::Rotate(90.0)), StickerAction::AddTextLayer(None), StickerAction::ToggleClip(None), StickerAction::ApplyCrop] {
        run_in(&mut state, action.clone());
        assert!(matches!(state.dialog, crate::state::Dialog::StickerShared { .. }), "{action:?}");
        assert_eq!(undo_len(&state), undo);
        run_in(&mut state, StickerAction::AnswerShared(None));
    }
}

fn unit_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target").join("sticker").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The library: Save to Library writes a `.wtes` into the folder of the prefs; the listing job lists it; a click adds
/// it to the tray (one entry) as the Stamp tool's source. Export as PSD writes a PSD that reads back layer by layer.
#[test]
fn k8_library_save_list_add_and_export_as_psd() {
    let dir = unit_dir("unit_library");
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    run_in(&mut state, StickerAction::AddTextLayer(None));
    state.prefs.sticker_library_dir = Some(dir.clone());
    run_in(&mut state, StickerAction::SaveToLibrary(None));
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.path()).collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(crate::dnd::extension_of(&files[0]), "wtes");
    // The listing job.
    let ctx = egui::Context::default();
    for _ in 0..2000 {
        library::frame(&ctx, &mut state);
        if !state.sticker_ui.library.busy() && !state.sticker_ui.library.items.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(state.sticker_ui.library.items.len(), 1);
    assert_eq!(state.sticker_ui.library.items[0].name, "Logo");
    // Add from the library.
    let before = state.doc().unwrap().stickers().count();
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::LibraryAdd(files[0].clone()));
    assert_eq!((state.doc().unwrap().stickers().count(), undo_len(&state)), (before + 1, undo + 1));
    let id = state.sticker_ui.tray_selected.unwrap();
    assert_eq!(state.tools.stamp.source, crate::state::StampToolSource::Sticker(id));
    assert!(state.doc().unwrap().sticker(id).unwrap().keep);
    assert_eq!(state.doc().unwrap().sticker(id).unwrap().layers.len(), 2);
    // A file that is not a sticker: an error toast, nothing added.
    std::fs::write(dir.join("broken.wtes"), b"no zip").unwrap();
    run_in(&mut state, StickerAction::LibraryAdd(dir.join("broken.wtes")));
    assert_eq!(undo_len(&state), undo + 1);
    // Export as PSD.
    let psd = dir.join("out.psd");
    run_in(&mut state, StickerAction::ExportPsd(Some(psd.clone())));
    let bytes = std::fs::read(&psd).unwrap();
    let read = wte_core::psd::read(&bytes, &wte_core::psd::PsdLimits::default()).unwrap();
    assert_eq!(read.layers.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The command list: the P8 groups follow what the editor has (Merge Down needs a layer below; Rasterize a text or
/// shape layer); every index fits its panel id; the menu shows the Sticker and Layer groups first.
#[test]
fn k8_the_command_list_has_the_sticker_and_layer_groups() {
    use crate::sticker::filters::{COMMANDS, Group, available, available_more, enabled_more};
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    let find = |label: &str| COMMANDS.iter().find(|d| d.label == label).unwrap();
    let (have, more) = (available(&mut state), available_more(&mut state));
    assert!(enabled_more(find("Trim..."), have, more) && enabled_more(find("Flatten"), have, more));
    assert!(!enabled_more(find("Merge Down"), have, more) && !enabled_more(find("Rasterize"), have, more));
    run_in(&mut state, StickerAction::AddTextLayer(None));
    let (have, more) = (available(&mut state), available_more(&mut state));
    assert!(enabled_more(find("Merge Down"), have, more) && enabled_more(find("Rasterize"), have, more));
    assert!(COMMANDS.len() <= u8::MAX as usize);
    assert_eq!(Group::MENU[..2], [Group::Sticker, Group::Layer]);
    assert!(Group::ALL.iter().all(|g| COMMANDS.iter().any(|d| d.group == *g)));
    // Every Layer and Sticker command runs without a panic on an image stamp (the dialogs are dropped headless).
    for def in COMMANDS.iter().filter(|d| matches!(d.group, Group::Sticker | Group::Layer)) {
        let action = (def.make)(&state);
        if matches!(action, StickerAction::ExportPsd(None) | StickerAction::AddPictureFile(None) | StickerAction::SaveToLibrary(_)) {
            continue;
        }
        run_in(&mut state, action);
        state.sticker_ui.popover = None;
        state.dialog = crate::state::Dialog::None;
    }
    let _ = StickerLayerId(1);
}

