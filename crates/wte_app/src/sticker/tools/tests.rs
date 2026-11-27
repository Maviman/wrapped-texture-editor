//! Tests of the sticker paint tools and the live canvas (stickers P7a; STICKER_PLAN 7 "P7a", 3.3, 3.5 review A6):
//! strokes through the canvas's pointer routing (one entry, promotion, pressure, eraser, mask), stroke mode and the
//! dirty-rectangle canvas, Fill, Gradient, Eyedropper, the layer-kind rule with Rasterize, the mask commands, and the
//! guard rules (Esc, Undo, Save, a tool switch, locked / hidden layers, the shared-picture question).

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use glam::Vec2;
use wte_core::sticker::render::{StickerRenderInput, render};
use wte_core::{
    Document, PicturePixels, PixelBuf, RasterQuality, STICKER_BRUSH_LABEL, STICKER_ERASER_LABEL, StampId, StampSource,
    StickerContent, StickerDoc, StickerId, StickerLayerId, StickerRef, StylePreset,
};

use crate::action::AppAction;
use crate::state::{AppState, StickerTool, ViewMode};
use crate::sticker::canvas::{Geom, Hit, handle_input, refresh};
use crate::sticker::tests::{CANVAS, image_stamp_state};
use crate::sticker::{CanvasDrag, StickerAction, edit_mut, edit_of, enter_in, run_in};

// ---- helpers ----

fn doc(state: &AppState) -> &Document {
    state.doc().unwrap()
}

fn undo_len(state: &AppState) -> usize {
    doc(state).history.undo_len()
}

/// The sticker id of a sticker stamp.
fn sticker_id(state: &AppState, stamp: StampId) -> StickerId {
    match doc(state).stamp(stamp).unwrap().source {
        StampSource::Sticker { sticker } => sticker,
        ref other => panic!("not a sticker stamp: {other:?}"),
    }
}

fn sticker_of(state: &AppState, stamp: StampId) -> Arc<StickerDoc> {
    Arc::clone(doc(state).sticker(sticker_id(state, stamp)).unwrap())
}

/// Runs the queued actions the canvas pushes (sticker commands, the Eyedropper's colour).
fn run_queued(state: &mut AppState) {
    for action in std::mem::take(&mut state.actions) {
        match action {
            AppAction::Sticker(a) => run_in(state, a),
            AppAction::SetStickerTool(t) => edit_mut(state).unwrap().tool = t,
            AppAction::SetPrimaryColor(c) => state.colors.primary = c,
            other => panic!("unexpected action {other:?}"),
        }
    }
}

const GEOM: Geom = Geom { origin: Vec2::ZERO, size: [64, 64] };

/// One egui pass: the canvas reads `events` (with pen notes `pen` per pointer event when given), the queued commands
/// run, the texture and the live canvas are brought up to date.
fn pass_with(ctx: &egui::Context, state: &mut AppState, hit: Hit, events: Vec<egui::Event>, pen: Option<f32>) {
    if let Some(p) = pen {
        let notes = events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerMoved(pos) | egui::Event::PointerButton { pos, .. } => Some((*pos, Some(p))),
                _ => None,
            })
            .collect();
        state.pen.frame = crate::pen::PenFrame { pointer: notes, in_range: true, in_contact: true, pressure: p, seen: true, ..Default::default() };
    } else {
        state.pen.frame = crate::pen::PenFrame::default();
    }
    let input = egui::RawInput {
        events,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        ..Default::default()
    };
    crate::theme::run_test_pass(ctx, input, |ui| handle_input(ui, state, CANVAS, GEOM, hit, true));
    run_queued(state);
    crate::sticker::validate_in(state);
    refresh(ctx, state);
}

fn pass(ctx: &egui::Context, state: &mut AppState, hit: Hit, events: Vec<egui::Event>) {
    pass_with(ctx, state, hit, events, None);
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

/// Layer space -> screen through the open editor's camera.
fn screen(state: &AppState, p: Vec2) -> egui::Pos2 {
    GEOM.to_screen(&edit_of(state).unwrap().camera, CANVAS, p)
}

/// The image stamp open in the editor (camera fitted to the test canvas), the canvas texture drawn once.
fn opened(ctx: &egui::Context) -> (AppState, StampId) {
    let (mut state, stamp, _) = image_stamp_state(ViewMode::View3D);
    enter_in(&mut state, Some(stamp));
    edit_mut(&mut state).unwrap().camera.fit(GEOM.size, CANVAS);
    refresh(ctx, &mut state);
    (state, stamp)
}

/// A press, moves through `path` (layer space), a release; one pass per point.
fn drag(ctx: &egui::Context, state: &mut AppState, hit: Hit, path: &[Vec2], pen: Option<f32>) {
    let first = screen(state, path[0]);
    pass_with(ctx, state, hit, vec![egui::Event::PointerMoved(first), button(first, true)], pen);
    for p in &path[1..] {
        let at = screen(state, *p);
        pass_with(ctx, state, hit, vec![egui::Event::PointerMoved(at)], pen);
    }
    let last = screen(state, *path.last().unwrap());
    pass_with(ctx, state, hit, vec![button(last, false)], pen);
}

/// `data` with the colour of every fully transparent pixel set to 0 (what the premultiplied canvas shows).
fn seen(p: &PixelBuf) -> Vec<u8> {
    let mut d = p.data.clone();
    for px in d.as_chunks_mut::<4>().0 {
        if px[3] == 0 {
            *px = [0; 4];
        }
    }
    d
}

/// The finished picture of the sticker (what a fresh full render of the canvas shows).
fn finished(state: &mut AppState, id: StickerId) -> PixelBuf {
    state.doc_mut().unwrap().sticker_picture(id, None, 1.0).unwrap()
}

/// The sticker drawn with layer `unstyled`'s styles off (the stroke-mode picture), from its own pixels.
fn unstyled_render(state: &mut AppState, id: StickerId, unstyled: StickerLayerId) -> PixelBuf {
    let doc = state.doc_mut().unwrap();
    let mut s = (**doc.sticker(id).unwrap()).clone();
    s.layer_mut(unstyled).unwrap().styles = Default::default();
    let mut pictures = Vec::new();
    for l in &s.layers {
        if let StickerContent::Picture(PicturePixels::Blob(b)) = &l.content {
            pictures.push((*b, doc.sticker_layer_pixels(StickerRef::Id(id), l.id).unwrap()));
        }
    }
    render(&StickerRenderInput { doc: Arc::new(s), pictures, fonts: Vec::new() }, 1.0, RasterQuality::Full)
}

/// Waits (bounded) until the live canvas and its jobs are done, a frame every few milliseconds.
fn settle(ctx: &egui::Context, state: &mut AppState) {
    let start = Instant::now();
    while crate::sticker::live::pending(state) {
        assert!(start.elapsed() < Duration::from_secs(20), "the live canvas never finished");
        std::thread::sleep(Duration::from_millis(3));
        refresh(ctx, state);
    }
}

/// The image stamp promoted with a second, empty picture layer on top (active), optionally with ClassicSticker.
fn with_paint_layer(ctx: &egui::Context, styled: bool) -> (AppState, StampId, StickerLayerId) {
    let (mut state, stamp) = opened(ctx);
    run_in(&mut state, StickerAction::AddPictureLayer(None));
    let layer = edit_of(&state).unwrap().active_layer.unwrap();
    if styled {
        let styles = StylePreset::ClassicSticker.styles([64, 64]);
        run_in(&mut state, StickerAction::SetLayerStyles(layer, Box::new(styles), wte_core::Commit::Final));
    }
    refresh(ctx, &mut state);
    (state, stamp, layer)
}

fn tool(state: &mut AppState, t: StickerTool) {
    edit_mut(state).unwrap().tool = t;
}

/// Pixels of the active layer that are not fully transparent.
fn painted(state: &AppState, stamp: StampId, layer: StickerLayerId) -> usize {
    let s = sticker_of(state, stamp);
    let p = s.layer(layer).unwrap().owned_picture().unwrap().clone();
    p.data.as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count()
}

// ---- strokes ----

/// A brush stroke on an image stamp: one entry "Sticker Brush" that promotes it; one undo brings the image stamp back;
/// the canvas shows the paint (a region upload, no whole redraw of the unstyled layer).
#[test]
fn a_brush_stroke_on_an_image_stamp_is_one_entry_and_promotes() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    tool(&mut state, StickerTool::Brush);
    state.tools.brush.size_px = 6.0;
    state.colors.primary = [255, 0, 0, 255];
    let undo = undo_len(&state);
    drag(&ctx, &mut state, Hit::Image, &[Vec2::new(10.0, 10.0), Vec2::new(30.0, 12.0), Vec2::new(50.0, 14.0)], None);
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 1, Some(STICKER_BRUSH_LABEL)));
    let s = sticker_of(&state, stamp);
    let p = s.layers[0].owned_picture().unwrap().clone();
    assert_eq!(p.get(30, 12), [255, 0, 0, 255], "the paint is on the layer");
    assert!(state.sticker_ui.drag.is_none() && !doc(&state).sticker_stroke_active());
    settle(&ctx, &mut state);
    let id = sticker_id(&state, stamp);
    let want = finished(&mut state, id);
    assert_eq!(seen(state.sticker_ui.mirror.as_ref().unwrap()), seen(&want), "the canvas is the finished picture");
    state.doc_mut().unwrap().undo();
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }), "one undo: stroke and promotion");
}

/// Stroke mode (review A6): on a layer with a contour, during the stroke the canvas is "layers below + the active layer
/// unstyled + layers above" (also inside every dab's rectangle) and no restyle is shown; at most one Preview job runs;
/// after the end the canvas equals a fresh full render. On a layer without styles no full recomposite happens.
#[test]
fn stroke_mode_shows_the_layer_unstyled_until_the_restyle_lands() {
    let ctx = egui::Context::default();
    let (mut state, stamp, layer) = with_paint_layer(&ctx, true);
    let id = sticker_id(&state, stamp);
    tool(&mut state, StickerTool::Brush);
    state.tools.brush.size_px = 8.0;
    state.colors.primary = [0, 120, 255, 255];
    let before = state.sticker_ui.counters;
    let first = screen(&state, Vec2::new(12.0, 30.0));
    pass(&ctx, &mut state, Hit::Sticker(id), vec![egui::Event::PointerMoved(first), button(first, true)]);
    assert!(state.sticker_ui.live.as_ref().is_some_and(|l| l.stroke_mode()), "stroke mode");
    assert_eq!(state.sticker_ui.counters.full_composites, before.full_composites + 1, "one full recomposite to enter it");
    for k in 1..=12 {
        let at = screen(&state, Vec2::new(12.0 + 3.0 * k as f32, 30.0 + (k % 3) as f32));
        pass(&ctx, &mut state, Hit::Sticker(id), vec![egui::Event::PointerMoved(at)]);
        let want = unstyled_render(&mut state, id, layer);
        assert_eq!(seen(state.sticker_ui.mirror.as_ref().unwrap()), seen(&want), "dab {k}: the unstyled composite");
        let c = state.sticker_ui.counters;
        assert_eq!((c.restyles_started, c.restyles_applied), (before.restyles_started, before.restyles_applied), "no restyle during the stroke");
        if k % 4 == 0 {
            std::thread::sleep(super::super::live::PREVIEW_EVERY);
        }
    }
    let c = state.sticker_ui.counters;
    assert_eq!(c.full_composites, before.full_composites + 1, "dirty rectangles after entering");
    assert!(c.regions > before.regions);
    assert!(c.most_previews_running <= 1 && c.previews_started >= 1, "{c:?}");
    let last = screen(&state, Vec2::new(48.0, 30.0));
    pass(&ctx, &mut state, Hit::Sticker(id), vec![button(last, false)]);
    assert_eq!(doc(&state).undo_label(), Some(STICKER_BRUSH_LABEL));
    // Until the restyle lands the unstyled picture stays (never the stale styled one).
    if state.sticker_ui.live.is_some() {
        let want = unstyled_render(&mut state, id, layer);
        assert_eq!(seen(state.sticker_ui.mirror.as_ref().unwrap()), seen(&want));
    }
    settle(&ctx, &mut state);
    let c = state.sticker_ui.counters;
    assert_eq!((c.restyles_started, c.restyles_applied), (before.restyles_started + 1, before.restyles_applied + 1), "{c:?}");
    let want = finished(&mut state, id);
    assert_eq!(seen(state.sticker_ui.mirror.as_ref().unwrap()), seen(&want), "the canvas equals a fresh full render");
    let uploads = state.sticker_ui.canvas_uploads;
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.canvas_uploads, uploads, "the restyled texture carries the sticker's key: no redraw");

    // A layer without styles: dirty rectangles only, before and after the end.
    let (mut state, stamp, _) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    tool(&mut state, StickerTool::Brush);
    let (uploads, before) = (state.sticker_ui.canvas_uploads, state.sticker_ui.counters);
    drag(&ctx, &mut state, Hit::Sticker(id), &[Vec2::new(5.0, 5.0), Vec2::new(20.0, 40.0), Vec2::new(60.0, 60.0)], None);
    settle(&ctx, &mut state);
    let c = state.sticker_ui.counters;
    assert_eq!((state.sticker_ui.canvas_uploads, c.full_composites, c.restyles_started), (uploads, before.full_composites, before.restyles_started));
    assert!(c.regions > before.regions);
    let want = finished(&mut state, id);
    assert_eq!(seen(state.sticker_ui.mirror.as_ref().unwrap()), seen(&want), "dirty rectangles give the finished picture");
    refresh(&ctx, &mut state);
    assert_eq!(state.sticker_ui.canvas_uploads, uploads, "and the texture keeps it without a redraw");
}

/// The pen's pressure changes the stroke's width (size dynamics on by default); the Eraser takes alpha away.
#[test]
fn pressure_changes_the_width_and_the_eraser_erases() {
    let ctx = egui::Context::default();
    let mut counts = Vec::new();
    for pressure in [0.15f32, 1.0] {
        let (mut state, stamp, layer) = with_paint_layer(&ctx, false);
        let id = sticker_id(&state, stamp);
        tool(&mut state, StickerTool::Brush);
        state.tools.brush.size_px = 16.0;
        drag(&ctx, &mut state, Hit::Sticker(id), &[Vec2::new(8.0, 32.0), Vec2::new(32.0, 32.0), Vec2::new(56.0, 32.0)], Some(pressure));
        counts.push(painted(&state, stamp, layer));
        if pressure == 1.0 {
            tool(&mut state, StickerTool::Eraser);
            state.tools.eraser.size_px = 64.0;
            state.tools.eraser.hardness = 1.0;
            drag(&ctx, &mut state, Hit::Sticker(id), &[Vec2::new(32.0, 32.0), Vec2::new(33.0, 32.0)], None);
            assert_eq!(doc(&state).undo_label(), Some(STICKER_ERASER_LABEL));
            assert!(painted(&state, stamp, layer) < counts[1] / 4, "the eraser took the paint away");
        }
    }
    assert!(counts[0] > 0 && counts[1] > counts[0] * 2, "a light touch paints a thinner line: {counts:?}");
}

/// Esc cancels a stroke (no entry); an Undo during a stroke undoes only the stroke; Save keeps it as one entry; a
/// tool switch in mid-stroke keeps it.
#[test]
fn the_guard_rules_end_a_stroke() {
    let ctx = egui::Context::default();
    let (mut state, stamp, layer) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    tool(&mut state, StickerTool::Brush);
    let undo = undo_len(&state);
    let press = |state: &mut AppState| {
        let a = screen(state, Vec2::new(10.0, 10.0));
        let b = screen(state, Vec2::new(40.0, 20.0));
        pass(&ctx, state, Hit::Sticker(id), vec![egui::Event::PointerMoved(a), button(a, true)]);
        pass(&ctx, state, Hit::Sticker(id), vec![egui::Event::PointerMoved(b)]);
        assert!(doc(state).sticker_stroke_active() && crate::sticker::drag_moved(state));
    };
    // Cancel (Esc, Undo, another command): no entry, no paint.
    press(&mut state);
    crate::sticker::cancel_drag(&mut state);
    assert!(!doc(&state).sticker_stroke_active() && state.sticker_ui.live.is_none());
    assert_eq!((undo_len(&state), painted(&state, stamp, layer)), (undo, 0));
    refresh(&ctx, &mut state);
    let want = finished(&mut state, id);
    assert_eq!(seen(state.sticker_ui.mirror.as_ref().unwrap()), seen(&want), "the canvas shows the document again");
    // Save: kept, one entry.
    press(&mut state);
    crate::sticker::finish_drag(&mut state);
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 1, Some(STICKER_BRUSH_LABEL)));
    assert!(painted(&state, stamp, layer) > 0);
    // The release after it changes nothing more.
    let b = screen(&state, Vec2::new(40.0, 20.0));
    pass(&ctx, &mut state, Hit::Sticker(id), vec![button(b, false)]);
    assert_eq!(undo_len(&state), undo + 1);
    // A tool switch keeps the stroke (the dispatcher's rule: `finish_drag` for a stroke).
    press(&mut state);
    assert!(matches!(state.sticker_ui.drag, Some(CanvasDrag::Stroke { .. })));
    crate::sticker::finish_drag(&mut state);
    assert_eq!(undo_len(&state), undo + 2);
    settle(&ctx, &mut state);
}

/// Locked, hidden, text: nothing painted, the status line says why; a text layer offers Rasterize (toast and the
/// header's button), which makes it a picture layer that the brush then paints.
#[test]
fn layer_kind_rules_offer_rasterize_and_never_paint_silently() {
    let ctx = egui::Context::default();
    let (mut state, stamp, layer) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    tool(&mut state, StickerTool::Brush);
    let undo = undo_len(&state);
    let at = screen(&state, Vec2::new(30.0, 30.0));
    run_in(&mut state, StickerAction::SetLayerLocked(layer, true));
    pass(&ctx, &mut state, Hit::Sticker(id), vec![egui::Event::PointerMoved(at), button(at, true)]);
    pass(&ctx, &mut state, Hit::Sticker(id), vec![button(at, false)]);
    assert_eq!(state.status_hint, super::LOCKED_HINT);
    run_in(&mut state, StickerAction::SetLayerLocked(layer, false));
    run_in(&mut state, StickerAction::SetLayerVisible(layer, false));
    pass(&ctx, &mut state, Hit::Sticker(id), vec![button(at, true)]);
    pass(&ctx, &mut state, Hit::Sticker(id), vec![button(at, false)]);
    assert_eq!(state.status_hint, super::HIDDEN_HINT);
    run_in(&mut state, StickerAction::SetLayerVisible(layer, true));
    assert_eq!(painted(&state, stamp, layer), 0);
    assert_eq!(undo_len(&state), undo + 4, "only the four lock / show entries");
    // A text layer.
    let text = {
        let d = state.doc_mut().unwrap();
        let spec = wte_core::TextSpec {
            content: "Hi".into(),
            font: d.default_font(),
            size_px: 24.0,
            color: [250, 240, 20, 255],
            align: wte_core::TextAlign::Center,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: wte_core::TextFx::default(),
        };
        let r = d.set_sticker(
            StickerRef::Id(id),
            |s| {
                let lid = s.alloc_layer();
                let mut l = wte_core::StickerLayer::new(lid, "text", StickerContent::Text(Box::new(spec)));
                l.transform.translate = Vec2::new(32.0, 32.0);
                s.layers.push(l);
            },
            "Add Text",
            wte_core::Commit::Final,
        );
        r.unwrap();
        sticker_of(&state, stamp).layers.last().unwrap().id
    };
    run_in(&mut state, StickerAction::SelectLayer(text));
    let undo = undo_len(&state);
    for t in [StickerTool::Brush, StickerTool::Eraser, StickerTool::Fill, StickerTool::Gradient] {
        tool(&mut state, t);
        state.toasts = Default::default();
        state.sticker_ui.rasterize_offer = None;
        let b = screen(&state, Vec2::new(40.0, 36.0));
        pass(&ctx, &mut state, Hit::Sticker(id), vec![egui::Event::PointerMoved(at), button(at, true)]);
        pass(&ctx, &mut state, Hit::Sticker(id), vec![egui::Event::PointerMoved(b)]);
        pass(&ctx, &mut state, Hit::Sticker(id), vec![button(b, false)]);
        assert_eq!(state.status_hint, super::RASTERIZE_HINT, "{t:?}");
        assert_eq!(state.sticker_ui.rasterize_offer, Some(text), "{t:?}");
        assert_eq!(state.toasts.visible_texts(), vec![super::RASTERIZE_HINT], "{t:?}");
        assert_eq!(undo_len(&state), undo, "{t:?} never paints a text layer");
        assert!(matches!(sticker_of(&state, stamp).layer(text).unwrap().content, StickerContent::Text(_)));
    }
    let look = finished(&mut state, id);
    run_in(&mut state, StickerAction::Rasterize);
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 1, Some(wte_core::RASTERIZE_LABEL)));
    assert!(matches!(sticker_of(&state, stamp).layer(text).unwrap().content, StickerContent::Picture(_)));
    assert_eq!(finished(&mut state, id), look, "the same look");
    crate::sticker::validate_in(&mut state);
    assert_eq!(state.sticker_ui.rasterize_offer, None);
    tool(&mut state, StickerTool::Brush);
    drag(&ctx, &mut state, Hit::Sticker(id), &[Vec2::new(30.0, 30.0), Vec2::new(40.0, 30.0)], None);
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 2, Some(STICKER_BRUSH_LABEL)));
}

/// A stroke on a sticker two stamps show asks the shared-picture question first and paints nothing.
#[test]
fn a_stroke_on_a_shared_sticker_asks_first() {
    let ctx = egui::Context::default();
    let (mut state, stamp, _) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    let placement = doc(&state).stamp(stamp).unwrap().placement;
    state.doc_mut().unwrap().add_stamp("Twin", StampSource::Sticker { sticker: id }, placement, wte_core::TextureSetId(0)).unwrap();
    let undo = undo_len(&state);
    tool(&mut state, StickerTool::Brush);
    let at = screen(&state, Vec2::new(30.0, 30.0));
    pass(&ctx, &mut state, Hit::Sticker(id), vec![egui::Event::PointerMoved(at), button(at, true)]);
    assert!(matches!(state.dialog, crate::state::Dialog::StickerShared { users: 2 }));
    assert!(!doc(&state).sticker_stroke_active() && state.sticker_ui.drag.is_none());
    pass(&ctx, &mut state, Hit::Sticker(id), vec![button(at, false)]);
    assert_eq!(undo_len(&state), undo);
}

// ---- Fill, Gradient, Eyedropper ----

/// Fill: the clicked colour region of the active layer (one entry "Fill"); with a selection the selection only.
#[test]
fn fill_takes_the_region_or_the_selection() {
    let ctx = egui::Context::default();
    let (mut state, stamp, layer) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    tool(&mut state, StickerTool::Fill);
    state.colors.primary = [10, 200, 30, 255];
    let undo = undo_len(&state);
    let at = screen(&state, Vec2::new(20.0, 20.0));
    pass(&ctx, &mut state, Hit::Sticker(id), vec![egui::Event::PointerMoved(at), button(at, true)]);
    pass(&ctx, &mut state, Hit::Sticker(id), vec![button(at, false)]);
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 1, Some(super::fill::FILL_LABEL)));
    assert_eq!(painted(&state, stamp, layer), 64 * 64, "the empty layer is one region");
    state.doc_mut().unwrap().undo();
    run_in(&mut state, StickerAction::SelectRect([8.0, 8.0, 24.0, 20.0]));
    run_in(&mut state, StickerAction::Fill([40.0, 40.0]));
    let p = sticker_of(&state, stamp).layer(layer).unwrap().owned_picture().unwrap().clone();
    assert_eq!(painted(&state, stamp, layer), 16 * 12, "only the selection");
    assert_eq!(p.get(10, 10), [10, 200, 30, 255]);
    // A click outside the layer's pixels says so.
    run_in(&mut state, StickerAction::SelectNone);
    run_in(&mut state, StickerAction::Fill([90.0, 10.0]));
    assert_eq!(state.status_hint, super::fill::OUTSIDE_HINT);
}

/// Gradient: a drag draws primary to secondary from the press to the release, limited by the selection; one entry.
#[test]
fn a_gradient_drag_inside_a_selection_is_one_entry() {
    let ctx = egui::Context::default();
    let (mut state, stamp, layer) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    tool(&mut state, StickerTool::Gradient);
    state.colors.primary = [255, 0, 0, 255];
    state.colors.secondary = [0, 0, 255, 255];
    run_in(&mut state, StickerAction::SelectRect([0.0, 16.0, 64.0, 48.0]));
    let undo = undo_len(&state);
    drag(&ctx, &mut state, Hit::Sticker(id), &[Vec2::new(0.0, 32.0), Vec2::new(32.0, 32.0), Vec2::new(64.0, 32.0)], None);
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 1, Some(super::gradient::GRADIENT_LABEL)));
    let p = sticker_of(&state, stamp).layer(layer).unwrap().owned_picture().unwrap().clone();
    let (l, r) = (p.get(0, 30), p.get(63, 30));
    assert!(l[0] > 240 && l[2] < 15 && r[2] > 240 && r[0] < 15, "{l:?} {r:?}");
    assert_eq!(p.get(32, 5)[3], 0, "outside the selection");
    assert_eq!(painted(&state, stamp, layer), 64 * 32);
    // A click without a drag draws nothing; Reverse swaps the ends.
    drag(&ctx, &mut state, Hit::Sticker(id), &[Vec2::new(10.0, 32.0), Vec2::new(10.1, 32.0)], None);
    assert_eq!(undo_len(&state), undo + 1);
    state.sticker_ui.gradient.reverse = true;
    run_in(&mut state, StickerAction::Gradient([0.0, 32.0], [64.0, 32.0]));
    let p = sticker_of(&state, stamp).layer(layer).unwrap().owned_picture().unwrap().clone();
    assert!(p.get(0, 30)[2] > 240, "reversed");
    let g = super::gradient::GradientOptions { mode: super::gradient::GradientMode::PrimaryToTransparent, ..Default::default() };
    assert_eq!(g.gradient([1, 2, 3, 255], [9; 4]).sample(1.0), [1, 2, 3, 0]);
    let mut g = super::gradient::GradientOptions::default();
    for _ in 0..10 {
        g.add_stop();
    }
    assert_eq!(g.stops.len(), wte_core::MAX_GRADIENT_STOPS);
    g.remove_stop(0);
    g.mode = super::gradient::GradientMode::Custom;
    assert_eq!(g.gradient([0; 4], [0; 4]).stops.len(), wte_core::MAX_GRADIENT_STOPS - 1);
}

/// The Eyedropper picks the finished sticker's colour (opaque) and nothing on a transparent pixel.
#[test]
fn the_eyedropper_picks_the_sticker_colour() {
    let ctx = egui::Context::default();
    let (mut state, stamp) = opened(&ctx);
    tool(&mut state, StickerTool::Eyedropper);
    let img = wte_core::samples::sample_stamp_image(64);
    let (x, y) = (0..64u32).flat_map(|y| (0..64u32).map(move |x| (x, y))).find(|&(x, y)| img.get(x, y)[3] == 255).unwrap();
    let at = screen(&state, Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
    pass(&ctx, &mut state, Hit::Image, vec![egui::Event::PointerMoved(at), button(at, true)]);
    pass(&ctx, &mut state, Hit::Image, vec![button(at, false)]);
    let c = img.get(x, y);
    assert_eq!(state.colors.primary, [c[0], c[1], c[2], 255]);
    assert!(matches!(doc(&state).stamp(stamp).unwrap().source, StampSource::Image { .. }), "picking never promotes");
    if let Some((tx, ty)) = (0..64u32).flat_map(|y| (0..64u32).map(move |x| (x, y))).find(|&(x, y)| img.get(x, y)[3] == 0) {
        let before = state.colors.primary;
        assert!(!super::eyedropper::pick(&mut state, Vec2::new(tx as f32 + 0.5, ty as f32 + 0.5)));
        assert_eq!((state.colors.primary, state.status_hint.as_str()), (before, super::eyedropper::NOTHING_HINT));
    }
}

// ---- masks ----

/// Add from selection, Paint on mask (the brush paints white / black into the mask, the picture untouched), Invert,
/// Apply and Delete: one entry each.
#[test]
fn mask_commands_and_paint_on_mask() {
    let ctx = egui::Context::default();
    let (mut state, stamp, layer) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    run_in(&mut state, StickerAction::Fill([5.0, 5.0]));
    let undo = undo_len(&state);
    run_in(&mut state, StickerAction::SelectRect([0.0, 0.0, 32.0, 64.0]));
    run_in(&mut state, StickerAction::AddMask);
    assert_eq!(doc(&state).undo_label(), Some(super::super::commands::ADD_MASK_LABEL));
    assert!(state.sticker_ui.paint_on_mask, "a new mask is the paint target");
    let mask = |state: &AppState| sticker_of(state, stamp).layer(layer).unwrap().mask.as_ref().unwrap().pixels.clone();
    let m = mask(&state);
    assert_eq!((m.get(10, 10), m.get(50, 10)), (255, 0), "from the selection");
    // Paint black into the mask: the picture keeps its pixels.
    run_in(&mut state, StickerAction::SelectNone);
    let picture = sticker_of(&state, stamp).layer(layer).unwrap().owned_picture().unwrap().clone();
    tool(&mut state, StickerTool::Brush);
    state.colors.primary = [0, 0, 0, 255];
    state.tools.brush.size_px = 10.0;
    state.tools.brush.hardness = 1.0;
    drag(&ctx, &mut state, Hit::Sticker(id), &[Vec2::new(10.0, 32.0), Vec2::new(11.0, 32.0)], None);
    assert_eq!(mask(&state).get(10, 32), 0, "the brush hid the mask there");
    assert_eq!(*sticker_of(&state, stamp).layer(layer).unwrap().owned_picture().unwrap().as_ref(), *picture, "picture untouched");
    run_in(&mut state, StickerAction::InvertMask);
    assert_eq!((mask(&state).get(10, 32), mask(&state).get(50, 10)), (255, 255));
    assert_eq!(doc(&state).undo_label(), Some(super::super::commands::INVERT_MASK_LABEL));
    run_in(&mut state, StickerAction::ApplyMask);
    let l = sticker_of(&state, stamp).layer(layer).unwrap().clone();
    assert!(l.mask.is_none() && !state.sticker_ui.paint_on_mask);
    assert_eq!(l.owned_picture().unwrap().get(10, 10)[3], 0, "the mask became the alpha");
    assert_eq!(undo_len(&state), undo + 4);
    run_in(&mut state, StickerAction::AddMask);
    run_in(&mut state, StickerAction::DeleteMask);
    assert!(sticker_of(&state, stamp).layer(layer).unwrap().mask.is_none());
    assert_eq!((undo_len(&state), doc(&state).undo_label()), (undo + 6, Some(super::super::commands::DELETE_MASK_LABEL)));
    run_in(&mut state, StickerAction::SetPaintOnMask(true));
    assert!(!state.sticker_ui.paint_on_mask && state.status_hint == super::super::commands::NO_MASK_HINT);
    for _ in 0..6 {
        state.doc_mut().unwrap().undo();
    }
    assert!(sticker_of(&state, stamp).layer(layer).unwrap().mask.is_none());
}

/// The selection is dropped when the artboard changes size (3.4).
#[test]
fn the_selection_is_dropped_when_the_artboard_changes() {
    let ctx = egui::Context::default();
    let (mut state, stamp, _) = with_paint_layer(&ctx, false);
    let id = sticker_id(&state, stamp);
    run_in(&mut state, StickerAction::SelectRect([0.0, 0.0, 10.0, 10.0]));
    crate::sticker::validate_in(&mut state);
    assert!(edit_of(&state).unwrap().selection.is_some());
    let r = state.doc_mut().unwrap().set_sticker(StickerRef::Id(id), |s| s.artboard = wte_core::IRect::new(0, 0, 80, 64), "Canvas Size", wte_core::Commit::Final);
    r.unwrap();
    crate::sticker::validate_in(&mut state);
    assert!(edit_of(&state).unwrap().selection.is_none());
}
