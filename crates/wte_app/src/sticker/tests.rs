//! Tests of the Sticker view (stickers P6a; STICKER_PLAN 7 "P6a"): entering and leaving, the commands, Revert, the
//! canvas's pointer routing (the Move tool, pans, zoom), and the helpers other modules' tests use.

use eframe::egui;
use glam::Vec2;

use super::*;
use crate::action::AppAction;
use crate::state::{AppState, StickerTool, ViewMode};
use crate::sticker::canvas::{Geom, Hit, handle_input};

/// The sphere sample with an image stamp of the 64-pixel sample logo (selected), in `mode`.
pub(crate) fn image_stamp_state(mode: ViewMode) -> (AppState, StampId, BlobId) {
    let mut state = crate::tools::test_support::state_with(wte_core::SampleKind::Sphere);
    state.view_mode = mode;
    state.last_edit_mode = mode;
    let doc = state.doc_mut().unwrap();
    let img = wte_core::samples::sample_stamp_image(64);
    let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
    let blob = doc.add_image_blob("logo.png", std::sync::Arc::from(png)).unwrap();
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.0, 0.5), glam::Vec3::Z, glam::Vec3::Y, Vec2::splat(0.3), 0.6);
    let stamp = doc.add_stamp("Logo", StampSource::Image { blob }, wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    state.session.as_mut().unwrap().selected_stamp = Some(stamp);
    (state, stamp, blob)
}

/// A second blob: a 16 x 8 opaque green picture.
fn green_blob(state: &mut AppState) -> BlobId {
    let mut img = PixelBuf::new(16, 8);
    img.data.as_chunks_mut::<4>().0.iter_mut().for_each(|p| *p = [0, 200, 0, 255]);
    let png = wte_core::encode_png(16, 8, &img.data, wte_core::PngLevel::Fast).unwrap();
    state.doc_mut().unwrap().add_image_blob("green.png", std::sync::Arc::from(png)).unwrap()
}

fn doc(state: &AppState) -> &Document {
    state.doc().unwrap()
}

fn undo_len(state: &AppState) -> usize {
    doc(state).history.undo_len()
}

/// The sticker of the stamp (panics for an image or text stamp).
fn sticker_of(state: &AppState, stamp: StampId) -> Arc<StickerDoc> {
    match doc(state).stamp(stamp).unwrap().source {
        StampSource::Sticker { sticker } => Arc::clone(doc(state).sticker(sticker).unwrap()),
        ref other => panic!("not a sticker stamp: {other:?}"),
    }
}

/// The sticker id of a sticker stamp.
fn sticker_id(state: &AppState, stamp: StampId) -> StickerId {
    match doc(state).stamp(stamp).unwrap().source {
        StampSource::Sticker { sticker } => sticker,
        ref other => panic!("not a sticker stamp: {other:?}"),
    }
}

/// Runs every queued sticker action of the state (what dispatch does at the end of a frame, for the state-only
/// commands).
pub(crate) fn dispatch_queued(state: &mut AppState) {
    for action in std::mem::take(&mut state.actions) {
        match action {
            AppAction::Sticker(a) => run_in(state, a),
            AppAction::SetStickerTool(t) => edit_mut(state).unwrap().tool = t,
            other => panic!("unexpected action {other:?}"),
        }
    }
}

#[test]
fn enter_and_leave_keep_the_previous_mode_and_change_nothing() {
    for mode in [ViewMode::Split, ViewMode::View3D, ViewMode::View2D] {
        let (mut state, stamp, _) = image_stamp_state(mode);
        state.tool = crate::state::ToolKind::Fill;
        let undo = undo_len(&state);
        let revision = doc(&state).revision();
        enter_in(&mut state, None);
        assert_eq!(state.view_mode, ViewMode::Sticker);
        let edit = edit_of(&state).unwrap();
        assert_eq!((edit.target, edit.return_mode, edit.active_layer), (StickerTarget::Stamp(stamp), mode, Some(StickerLayerId(1))));
        assert_eq!(state.remembered_view_mode(), mode, "the Sticker view is never what is remembered");
        assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }), "no promotion by opening");
        assert_eq!((undo_len(&state), doc(&state).revision()), (undo, revision));
        leave_in(&mut state, None);
        assert_eq!(state.view_mode, mode);
        assert!(state.session.as_ref().unwrap().sticker_edit.is_none());
        assert_eq!(state.tool, crate::state::ToolKind::Fill, "the paint tool comes back");
    }
    // Without a stamp: the status line says what to do.
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    state.session.as_mut().unwrap().selected_stamp = None;
    enter_in(&mut state, None);
    assert_eq!((state.view_mode, state.status_hint.as_str()), (ViewMode::View3D, NO_STAMP_HINT));
    // Opened from Preview it goes back to Preview; what is remembered is the edit mode before Preview.
    let (mut state, _, _) = image_stamp_state(ViewMode::Split);
    state.view_mode = ViewMode::Preview;
    enter_in(&mut state, None);
    assert_eq!(state.remembered_view_mode(), ViewMode::Split);
    leave_in(&mut state, None);
    assert_eq!(state.view_mode, ViewMode::Preview);
}

#[test]
fn the_sticker_split_preference_reads_leniently() {
    let prefs = crate::prefs::from_json(r#"{"sticker_split": 0.3}"#).unwrap();
    assert_eq!(prefs.sticker_split, 0.3);
    for text in [r#"{"sticker_split": "wide"}"#, r#"{}"#, r#"{"sticker_split": null}"#] {
        assert_eq!(crate::prefs::from_json(text).unwrap().sticker_split, crate::state::STICKER_SPLIT_DEFAULT, "{text}");
    }
    assert_eq!(crate::prefs::from_json(r#"{"sticker_split": 7}"#).unwrap().sticker_split, 0.85);
    let json = serde_json::to_string(&crate::state::Prefs::default()).unwrap();
    assert!(!json.contains("view_mode") && !json.contains("\"sticker\""), "no view mode in the prefs: {json}");
}

/// A layer added to an image stamp promotes it inside the same entry: one undo brings the image stamp back.
#[test]
fn a_layer_add_on_an_image_stamp_promotes_and_one_undo_undoes_both() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    let green = green_blob(&mut state);
    enter_in(&mut state, Some(stamp));
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::AddPictureLayer(Some(green)));
    let sticker = sticker_of(&state, stamp);
    assert_eq!(sticker.layers.len(), 2);
    let added = &sticker.layers[1];
    assert_eq!((added.id, added.name.as_str()), (StickerLayerId(2), "green"));
    assert_eq!(added.transform.translate, Vec2::new(24.0, 28.0), "centred on the 64 x 64 artboard");
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(StickerLayerId(2)));
    assert_eq!(undo_len(&state), undo + 1, "promotion and add are one entry");
    assert_eq!(doc(&state).undo_label(), Some(ADD_LABEL));
    state.doc_mut().unwrap().undo();
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }));
    validate_in(&mut state);
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(StickerLayerId(1)), "the active layer follows the undo");
    assert_eq!(state.view_mode, ViewMode::Sticker, "the view stays open on the image stamp");
}

#[test]
fn layer_commands_are_one_entry_each_and_keep_their_rules() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    let green = green_blob(&mut state);
    enter_in(&mut state, Some(stamp));
    // The last layer cannot go; a new empty layer has the artboard's size.
    run_in(&mut state, StickerAction::DeleteLayer(None));
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }), "nothing happened");
    assert!(state.status_hint.contains("at least one layer"));
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    let s = sticker_of(&state, stamp);
    let new = s.layers[1].owned_picture().expect("an owned picture");
    assert_eq!(([new.width, new.height], s.layers[1].name.as_str()), ([64, 64], "Layer 2"));
    run_in(&mut state, StickerAction::AddPictureLayer(Some(green)));
    let mut undo = undo_len(&state);
    fn step(state: &mut AppState, undo: &mut usize, action: StickerAction, label: &str) {
        run_in(state, action);
        *undo += 1;
        assert_eq!(undo_len(state), *undo, "{label}: one entry");
        assert_eq!(doc(state).undo_label(), Some(label));
    }
    let ids: Vec<StickerLayerId> = sticker_of(&state, stamp).layers.iter().map(|l| l.id).collect();
    assert_eq!(ids, [StickerLayerId(1), StickerLayerId(2), StickerLayerId(3)]);
    step(&mut state, &mut undo, StickerAction::MoveLayer(None, -1), REORDER_LABEL);
    assert_eq!(sticker_of(&state, stamp).layers[1].id, StickerLayerId(3));
    step(&mut state, &mut undo, StickerAction::MoveLayerTo(StickerLayerId(3), 0), REORDER_LABEL);
    assert_eq!(sticker_of(&state, stamp).layers[0].id, StickerLayerId(3));
    step(&mut state, &mut undo, StickerAction::SetLayerVisible(StickerLayerId(1), false), "Hide Sticker Layer");
    step(&mut state, &mut undo, StickerAction::SetLayerBlend(StickerLayerId(1), BlendMode::Multiply), BLEND_LABEL);
    step(&mut state, &mut undo, StickerAction::RenameLayer(StickerLayerId(1), "  Base  ".into()), RENAME_LABEL);
    assert_eq!(sticker_of(&state, stamp).layers[1].name, "Base");
    step(&mut state, &mut undo, StickerAction::DuplicateLayer(Some(StickerLayerId(1))), DUPLICATE_LABEL);
    let s = sticker_of(&state, stamp);
    assert_eq!((s.layers[2].id, s.layers[2].name.as_str()), (StickerLayerId(4), "Base copy"));
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(StickerLayerId(4)));
    step(&mut state, &mut undo, StickerAction::Nudge(10, -1), NUDGE_LABEL);
    assert_eq!(sticker_of(&state, stamp).layers[2].transform.translate, Vec2::new(10.0, -1.0));
    // A locked layer is neither moved nor deleted.
    step(&mut state, &mut undo, StickerAction::SetLayerLocked(StickerLayerId(4), true), "Lock Sticker Layer");
    run_in(&mut state, StickerAction::Nudge(1, 0));
    run_in(&mut state, StickerAction::SetLayerPosition(StickerLayerId(4), [5.0, 5.0], Commit::Final));
    run_in(&mut state, StickerAction::DeleteLayer(None));
    assert_eq!(undo_len(&state), undo, "nothing on a locked layer");
    assert!(state.status_hint.contains("locked"));
    step(&mut state, &mut undo, StickerAction::SetLayerLocked(StickerLayerId(4), false), "Unlock Sticker Layer");
    step(&mut state, &mut undo, StickerAction::DeleteLayer(None), DELETE_LABEL);
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(StickerLayerId(1)), "the layer below becomes active");
    // A Preview opacity drag folds into one entry; Merge Down never touches a texture layer.
    let layers_before = doc(&state).sets[0].layers.len();
    run_in(&mut state, StickerAction::SetLayerOpacity(StickerLayerId(1), 0.8, Commit::Preview));
    run_in(&mut state, StickerAction::SetLayerOpacity(StickerLayerId(1), 0.6, Commit::Preview));
    run_in(&mut state, StickerAction::SetLayerOpacity(StickerLayerId(1), 0.5, Commit::Final));
    assert_eq!((undo_len(&state), sticker_of(&state, stamp).layers[1].opacity), (undo + 1, 0.5));
    run_in(&mut state, StickerAction::MergeDown(None));
    assert_eq!((undo_len(&state), doc(&state).sets[0].layers.len()), (undo + 1, layers_before));
}

/// Delete in the Sticker view (with the stamp selected) leaves the stamp, the sticker and the texture layers.
#[test]
fn delete_in_the_sticker_view_never_deletes_the_stamp() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    let (undo, layers) = (undo_len(&state), doc(&state).sets[0].layers.len());
    assert_eq!(state.session.as_ref().unwrap().selected_stamp, Some(stamp));
    run_in(&mut state, StickerAction::DeleteKey);
    assert!(doc(&state).stamp(stamp).is_some());
    assert_eq!((undo_len(&state), doc(&state).sets[0].layers.len()), (undo, layers));
    assert_eq!(state.status_hint, DELETE_HINT);
}

/// Revert (review B12): one entry that brings the stamp back as it was when the editor opened; undoable; disabled
/// while nothing differs.
#[test]
fn revert_puts_the_entry_state_back_in_one_undoable_entry() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    let green = green_blob(&mut state);
    enter_in(&mut state, Some(stamp));
    let differs = |state: &AppState| differs_from_entry(doc(state), edit_of(state).unwrap());
    assert!(!differs(&state));
    run_in(&mut state, StickerAction::AddPictureLayer(Some(green)));
    run_in(&mut state, StickerAction::Nudge(3, 3));
    assert!(differs(&state));
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::Revert);
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 1, Some(REVERT_LABEL)));
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }));
    assert!(!differs(&state));
    validate_in(&mut state);
    assert_eq!(state.view_mode, ViewMode::Sticker, "the editor stays open");
    state.doc_mut().unwrap().undo();
    assert_eq!(sticker_of(&state, stamp).layers.len(), 2, "the undo of Revert brings the edits back");
    assert!(differs(&state));
}

/// The editor closes when its stamp is gone and when another mode is set without it.
#[test]
fn the_view_closes_when_its_stamp_is_gone() {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::Split);
    enter_in(&mut state, None);
    state.doc_mut().unwrap().remove_stamp(stamp).unwrap();
    validate_in(&mut state);
    assert_eq!(state.view_mode, ViewMode::Split);
    assert!(state.session.as_ref().unwrap().sticker_edit.is_none());
    // A view mode set behind its back closes the editor at the next validate.
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, None);
    state.view_mode = ViewMode::View2D;
    validate_in(&mut state);
    assert!(state.session.as_ref().unwrap().sticker_edit.is_none());
    assert_eq!(state.view_mode, ViewMode::View2D);
}

/// A text stamp opens read-only; "Convert to layered sticker" makes it a sticker with one text layer.
#[test]
fn a_text_stamp_opens_read_only_and_converts() {
    let (mut state, _, _) = image_stamp_state(ViewMode::View3D);
    let doc_mut = state.doc_mut().unwrap();
    let spec = wte_core::TextSpec::new("Hi", doc_mut.default_font());
    let p = wte_core::Projector::from_surface(glam::Vec3::new(0.0, 0.2, 0.5), glam::Vec3::Z, glam::Vec3::Y, Vec2::new(0.3, 0.1), 0.6);
    let text = doc_mut.add_stamp("Hi", StampSource::Text(spec), wte_core::Placement::Projected(p), wte_core::TextureSetId(0)).unwrap();
    enter_in(&mut state, Some(text));
    assert_eq!(edit_of(&state).unwrap().active_layer, None);
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    run_in(&mut state, StickerAction::Nudge(1, 0));
    assert_eq!(undo_len(&state), undo, "a text stamp is not edited here");
    run_in(&mut state, StickerAction::ConvertText);
    let s = sticker_of(&state, text);
    assert!(matches!(s.layers[0].content, StickerContent::Text(_)));
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(s.layers[0].id));
    assert_eq!(undo_len(&state), undo + 1);
}

// ---- the canvas's pointer routing ----

pub(crate) const CANVAS: egui::Rect = egui::Rect { min: egui::pos2(100.0, 100.0), max: egui::pos2(500.0, 500.0) };

/// One egui pass in which the canvas reads `events`, then the queued commands run.
pub(crate) fn canvas_pass(ctx: &egui::Context, state: &mut AppState, geom: Geom, hit: Hit, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        events,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        ..Default::default()
    };
    crate::theme::run_test_pass(ctx, input, |ui| handle_input(ui, state, CANVAS, geom, hit, true));
    dispatch_queued(state);
}

fn button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }
}

/// The image stamp opened, its camera fitted to the 400 x 400 canvas.
pub(crate) fn canvas_state() -> (AppState, StampId, Geom) {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, Some(stamp));
    let geom = Geom { origin: Vec2::ZERO, size: [64, 64] };
    edit_mut(&mut state).unwrap().camera.fit(geom.size, CANVAS);
    (state, stamp, geom)
}

/// A Move drag on the canvas: Preview positions while dragging (whole pixels), one entry at the release; the image
/// stamp is promoted inside it. A click without motion changes nothing.
#[test]
fn a_move_drag_on_the_canvas_is_one_entry() {
    let (mut state, stamp, geom) = canvas_state();
    let ctx = egui::Context::default();
    let cam = edit_of(&state).unwrap().camera;
    let start = geom.to_screen(&cam, CANVAS, Vec2::new(32.0, 32.0));
    let undo = undo_len(&state);
    // A click: nothing.
    canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![egui::Event::PointerMoved(start), button(start, egui::PointerButton::Primary, true)]);
    canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![button(start, egui::PointerButton::Primary, false)]);
    assert_eq!(undo_len(&state), undo);
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }));
    // A drag by 10 x 4 pixels in three moves.
    canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![egui::Event::PointerMoved(start), button(start, egui::PointerButton::Primary, true)]);
    for k in 1..=3 {
        let p = geom.to_screen(&cam, CANVAS, Vec2::new(32.0, 32.0) + Vec2::new(10.0, 4.0) * (k as f32 / 3.0));
        canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![egui::Event::PointerMoved(p)]);
        assert!(doc(&state).has_pending_edit(), "a Preview while dragging");
    }
    let end = geom.to_screen(&cam, CANVAS, Vec2::new(42.0, 36.0));
    canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![button(end, egui::PointerButton::Primary, false)]);
    assert!(!doc(&state).has_pending_edit() && state.sticker_ui.drag.is_none());
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 1, Some(MOVE_LABEL)));
    assert_eq!(sticker_of(&state, stamp).layers[0].transform.translate, Vec2::new(10.0, 4.0));
    state.doc_mut().unwrap().undo();
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }), "one undo: move and promotion");
}

/// Secondary and Middle drags pan with any tool, so does Hand; the wheel zooms about the pointer. None of it touches
/// the document.
#[test]
fn pans_and_zoom_touch_only_the_camera() {
    let (mut state, _, geom) = canvas_state();
    let ctx = egui::Context::default();
    let undo = undo_len(&state);
    let at = egui::pos2(300.0, 300.0);
    for (tool, b) in [
        (StickerTool::Move, egui::PointerButton::Secondary),
        (StickerTool::Move, egui::PointerButton::Middle),
        (StickerTool::Hand, egui::PointerButton::Primary),
    ] {
        edit_mut(&mut state).unwrap().tool = tool;
        let before = edit_of(&state).unwrap().camera;
        canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![egui::Event::PointerMoved(at), button(at, b, true)]);
        canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![egui::Event::PointerMoved(at + egui::vec2(30.0, 10.0))]);
        canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![button(at + egui::vec2(30.0, 10.0), b, false)]);
        let after = edit_of(&state).unwrap().camera;
        assert!(after.center_uv != before.center_uv && after.zoom == before.zoom, "{tool:?} {b:?}: {before:?} -> {after:?}");
    }
    let before = edit_of(&state).unwrap().camera;
    canvas_pass(&ctx, &mut state, geom, Hit::Image, vec![egui::Event::PointerMoved(at), wheel(2.0)]);
    assert!(edit_of(&state).unwrap().camera.zoom > before.zoom);
    assert_eq!(undo_len(&state), undo);
    assert!(!doc(&state).has_pending_edit());
}

/// A wheel event of `notches` lines.
pub(crate) fn wheel(notches: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, notches),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A locked layer is not dragged; a tool that is not built says so.
#[test]
fn a_locked_layer_and_an_unbuilt_tool_do_nothing() {
    let (mut state, stamp, geom) = canvas_state();
    run_in(&mut state, StickerAction::SetLayerLocked(StickerLayerId(1), true));
    let ctx = egui::Context::default();
    let cam = edit_of(&state).unwrap().camera;
    let at = geom.to_screen(&cam, CANVAS, Vec2::new(20.0, 20.0));
    let hit = Hit::Sticker(sticker_id(&state, stamp));
    canvas_pass(&ctx, &mut state, geom, hit, vec![egui::Event::PointerMoved(at), button(at, egui::PointerButton::Primary, true)]);
    assert!(state.sticker_ui.drag.is_none() && state.status_hint.contains("locked"), "{}", state.status_hint);
    canvas_pass(&ctx, &mut state, geom, hit, vec![button(at, egui::PointerButton::Primary, false)]);
    edit_mut(&mut state).unwrap().tool = StickerTool::Brush;
    canvas_pass(&ctx, &mut state, geom, hit, vec![button(at, egui::PointerButton::Primary, true)]);
    assert!(state.status_hint.contains("Brush tool is not available yet"), "{}", state.status_hint);
    assert!(state.sticker_ui.drag.is_none());
}

/// Auto-select: a click picks the top layer under the pointer; off, the active layer stays.
#[test]
fn auto_select_picks_the_top_layer_under_the_pointer() {
    let (mut state, stamp, geom) = canvas_state();
    let green = green_blob(&mut state);
    run_in(&mut state, StickerAction::AddPictureLayer(Some(green)));
    run_in(&mut state, StickerAction::SelectLayer(StickerLayerId(1)));
    let hit = Hit::Sticker(sticker_id(&state, stamp));
    let ctx = egui::Context::default();
    let cam = edit_of(&state).unwrap().camera;
    // The green picture covers 24..40 x 28..36.
    let on_green = geom.to_screen(&cam, CANVAS, Vec2::new(30.0, 30.0));
    canvas_pass(&ctx, &mut state, geom, hit, vec![egui::Event::PointerMoved(on_green), button(on_green, egui::PointerButton::Primary, true)]);
    canvas_pass(&ctx, &mut state, geom, hit, vec![button(on_green, egui::PointerButton::Primary, false)]);
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(StickerLayerId(2)));
    state.sticker_ui.auto_select = false;
    run_in(&mut state, StickerAction::SelectLayer(StickerLayerId(1)));
    canvas_pass(&ctx, &mut state, geom, hit, vec![button(on_green, egui::PointerButton::Primary, true)]);
    canvas_pass(&ctx, &mut state, geom, hit, vec![button(on_green, egui::PointerButton::Primary, false)]);
    assert_eq!(edit_of(&state).unwrap().active_layer, Some(StickerLayerId(1)), "off: the active layer stays");
}

#[test]
fn guarded_commands_are_the_document_changes() {
    assert!(StickerAction::AddPictureLayer(None).changes_document());
    assert!(StickerAction::SetLayerPosition(StickerLayerId(1), [0.0, 0.0], Commit::Final).changes_document());
    assert!(!StickerAction::SetLayerPosition(StickerLayerId(1), [0.0, 0.0], Commit::Preview).changes_document());
    assert!(!StickerAction::SelectLayer(StickerLayerId(1)).changes_document());
    assert!(!StickerAction::Fit.changes_document());
    assert!(!StickerAction::Escape.changes_document());
}
