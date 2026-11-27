// Owner: stickers P7a (STICKER_PLAN 3.3 rows Brush and Eraser, 3.5, 3.10). The sticker Brush and Eraser: a press opens
// a stroke on the active picture layer (or its mask with "Paint on mask"), every pointer event of the drag is one
// sample with the pen's pressure (`Document::sticker_stroke_dab`; the core interpolates dabs between samples), the
// release ends it (one entry "Sticker Brush" / "Sticker Eraser"; an image stamp is promoted inside it). Size,
// hardness, opacity, flow, spacing and the pen-pressure options are the existing `BrushSettings` (the Brush's for the
// Brush, the Eraser's for the Eraser), in layer-space pixels. The Brush paints the primary colour; on a mask it paints
// the colour's grey level (white shows, black hides) and the Eraser hides. The live canvas follows (`live.rs`).
//
// The Eraser shares everything with the Brush but the mode, so it lives here (the plan's `eraser.rs` is this file).

use glam::Vec2;
use wte_core::{BrushMode, StickerBrush};

use crate::state::{AppState, BrushSettings, StickerTool};
use crate::sticker::{CanvasDrag, canvas, live, shown};

/// The core brush of the settings `b` (diameter -> radius), in `mode`, with the primary colour.
pub(crate) fn brush_of(b: &BrushSettings, mode: BrushMode, color: [u8; 4]) -> StickerBrush {
    StickerBrush {
        radius: b.size_px * 0.5,
        hardness: b.hardness,
        opacity: b.opacity,
        flow: b.flow,
        spacing: b.spacing,
        color,
        mode,
        dynamics: b.dynamics(),
        selection: None,
    }
}

/// The settings a sticker tool paints with: the Eraser's for the Eraser, else the Brush's.
pub(crate) fn settings_of(state: &AppState, tool: StickerTool) -> &BrushSettings {
    if tool == StickerTool::Eraser { &state.tools.eraser } else { &state.tools.brush }
}

/// A press of the Brush or Eraser at layer-space point `p` with pen `pressure`: opens the stroke and paints its first
/// dab. False (and the status line says why) when nothing can be painted.
pub(crate) fn press(state: &mut AppState, tool: StickerTool, p: Vec2, pressure: f32, button: eframe::egui::PointerButton) -> bool {
    let Some(target) = super::paint_target(state, tool, true) else { return false };
    let mode = if tool == StickerTool::Eraser { BrushMode::Erase } else { BrushMode::Paint };
    let mut brush = brush_of(settings_of(state, tool), mode, state.colors.primary);
    brush.selection = super::selection_for(state, &target);
    // Is the texture the finished picture of what is shown before the stroke? (else the live canvas draws it whole)
    let exact = {
        let Some(edit) = crate::sticker::edit_of(state).cloned() else { return false };
        let Some(session) = state.session.as_mut() else { return false };
        let Some(shown) = shown(&mut session.doc, &edit) else { return false };
        let key = canvas::key_of_shown(&shown, &session.doc);
        state.sticker_ui.canvas.as_ref().is_some_and(|c| c.exact && Some(&c.key) == key.as_ref())
    };
    let Some(doc) = state.doc_mut() else { return false };
    let id = match doc.sticker_stroke_begin(target.on, target.layer, target.target, brush) {
        Ok(id) => id,
        Err(e) => {
            crate::dispatch::edited::<()>(state, Err(e));
            return false;
        }
    };
    // The live canvas shows the stroke's layer unstyled only when its styles are on the picture being painted (on a
    // mask the styles follow the mask's alpha just the same).
    live::start(state, id, target.layer, target.styled, exact);
    state.sticker_ui.drag = Some(CanvasDrag::Stroke { button, painted: false });
    state.status_hint.clear();
    sample(state, p, pressure);
    true
}

/// One sample of the open stroke (a pointer event of the drag).
pub(crate) fn sample(state: &mut AppState, p: Vec2, pressure: f32) {
    let Some(doc) = state.doc_mut() else { return };
    if !doc.sticker_stroke_active() {
        return;
    }
    match doc.sticker_stroke_dab(p, pressure) {
        Ok(r) if !r.is_empty() => {
            live::note_dab(state, r);
            if let Some(CanvasDrag::Stroke { painted, .. }) = state.sticker_ui.drag.as_mut() {
                *painted = true;
            }
        }
        Ok(_) => {}
        Err(e) => log::debug!("sticker stroke sample dropped: {e}"),
    }
}

/// Ends the open stroke and keeps it (the release, Save, a tool switch): one entry; the users get their Final bake.
pub(crate) fn end(state: &mut AppState) {
    if !matches!(state.sticker_ui.drag, Some(CanvasDrag::Stroke { .. }) | None) {
        return;
    }
    state.sticker_ui.drag = None;
    let Some(doc) = state.doc_mut() else { return };
    if !doc.sticker_stroke_active() {
        live::cancelled(state);
        return;
    }
    let r = doc.sticker_stroke_end();
    if crate::dispatch::edited(state, r).is_some() {
        live::ended(state);
    } else {
        live::cancelled(state);
    }
}

/// Takes the open stroke back (Esc, Undo, another command): no entry, the pixels as before.
pub(crate) fn cancel(state: &mut AppState) {
    if let Some(doc) = state.doc_mut() {
        doc.sticker_stroke_cancel();
    }
    live::cancelled(state);
}
