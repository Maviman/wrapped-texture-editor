// Owner: stickers P8 (STICKER_PLAN 3.3 "Move / Transform"; ARCHITECTURE 8.15). The Move tool's transform handles: the
// active layer's content box (`Document::sticker_layer_box`) drawn on the canvas with eight scale handles (corners keep
// the aspect, Shift frees it; edges scale one axis) and a rotate handle above the top edge (Shift = 15 degree steps).
// A handle drag sends `StickerAction::SetLayerTransform` with `Commit::Preview` while it moves and one `Final` at the
// release: one entry "Transform Sticker Layer". The Tool tab's numbers (W, H, rotation) and Flip / Reset use the same
// box maths (`wte_core::LayerBox`).

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke};
use glam::Vec2;
use wte_core::{LayerBox, LayerTransform, StickerContent, StickerLayerId};

use crate::state::AppState;
use crate::sticker::canvas::Geom;
use crate::sticker::{edit_of, edit_ref};

/// History label of a handle drag, the numeric fields and Flip / Reset of one layer.
pub(crate) const TRANSFORM_LABEL: &str = "Transform Sticker Layer";
/// Distance (points) within which a press takes a handle.
pub(crate) const HANDLE_GRAB: f32 = 9.0;
/// Half side of a drawn handle (points); the rotate handle sits this far above the top edge.
const HANDLE_HALF: f32 = 4.0;
const ROTATE_OFFSET: f32 = 22.0;
/// The Shift step of the rotate handle.
const ROTATE_STEP_DEG: f32 = 15.0;
/// Smallest scale magnitude a handle drag sets (`LayerTransform::sanitize` would turn 0 into 1).
const MIN_SCALE: f32 = 1.0e-3;

/// A handle of the transform box: a scale handle at `(hx, hy)` (-1, 0, 1 per axis: a corner has both non-zero, an
/// edge one), or the rotate handle.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Handle {
    Scale(i8, i8),
    Rotate,
}

/// The eight scale handles.
const SCALE_HANDLES: [(i8, i8); 8] = [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)];

/// The content point of a scale handle.
fn handle_point(b: &LayerBox, hx: i8, hy: i8) -> Vec2 {
    b.size * 0.5 * Vec2::new(1.0 + hx as f32, 1.0 + hy as f32)
}

/// The cached box of the active layer (`StickerUi::layer_box`): the layer's id and content it was made from.
#[derive(Clone, Debug)]
pub(crate) struct BoxCache {
    layer: StickerLayerId,
    content: StickerContent,
    pub(crate) b: LayerBox,
}

/// The content box and transform of the active layer of the open editor (`None` for a text stamp, no active layer,
/// or a layer without content). The box is computed once per content (a text is laid out once).
pub(crate) fn active_box(state: &mut AppState) -> Option<(StickerLayerId, LayerBox, LayerTransform, bool)> {
    let edit = edit_of(state)?.clone();
    let on = edit_ref(&edit)?;
    let id = edit.active_layer?;
    let (doc, _) = crate::sticker::layered(state, Some(id))?;
    let layer = doc.layer(id)?.clone();
    if let Some(c) = &state.sticker_ui.layer_box
        && c.layer == id
        && c.content == layer.content
    {
        return Some((id, c.b, layer.transform, layer.locked));
    }
    let b = state.doc_mut()?.sticker_layer_box(on, id).ok()?;
    state.sticker_ui.layer_box = Some(BoxCache { layer: id, content: layer.content.clone(), b });
    Some((id, b, layer.transform, layer.locked))
}

/// Screen positions of the handles of box `b` under `t`.
fn handle_positions(b: &LayerBox, t: &LayerTransform, geom: Geom, cam: &crate::state::Canvas2D, rect: Rect) -> Vec<(Handle, Pos2)> {
    let mut out: Vec<(Handle, Pos2)> =
        SCALE_HANDLES.iter().map(|&(hx, hy)| (Handle::Scale(hx, hy), geom.to_screen(cam, rect, b.to_layer(t, handle_point(b, hx, hy))))).collect();
    let top = geom.to_screen(cam, rect, b.to_layer(t, handle_point(b, 0, -1)));
    let centre = geom.to_screen(cam, rect, b.centre(t));
    let up = (top - centre).normalized();
    let up = if up.x.is_finite() && up.y.is_finite() && up.length() > 0.5 { up } else { egui::vec2(0.0, -1.0) };
    out.push((Handle::Rotate, top + up * ROTATE_OFFSET));
    out
}

/// The handle under the screen point `pos` (the nearest within `HANDLE_GRAB`).
pub(crate) fn handle_at(state: &mut AppState, geom: Geom, rect: Rect, pos: Pos2) -> Option<(StickerLayerId, Handle, LayerBox, LayerTransform)> {
    let cam = edit_of(state)?.camera;
    let (id, b, t, locked) = active_box(state)?;
    if locked {
        return None;
    }
    let (handle, d) = handle_positions(&b, &t, geom, &cam, rect)
        .into_iter()
        .map(|(h, p)| (h, p.distance(pos)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    (d <= HANDLE_GRAB).then_some((id, handle, b, t))
}

/// The transform a drag of `handle` from the layer-space point `start` to `p` gives (`free`: Shift held, a corner
/// scales each axis on its own / the rotation is not stepped... for the rotate handle Shift steps by 15 degrees).
pub(crate) fn dragged(b: &LayerBox, t0: &LayerTransform, handle: Handle, start: Vec2, p: Vec2, shift: bool, keep_aspect: bool) -> LayerTransform {
    match handle {
        Handle::Rotate => {
            let c = b.centre(t0);
            let (a0, a1) = ((start - c).to_angle(), (p - c).to_angle());
            if !(a0.is_finite() && a1.is_finite()) || (p - c).length() < 1.0e-3 {
                return *t0;
            }
            let mut angle = t0.rotation + (a1 - a0);
            if shift {
                let step = ROTATE_STEP_DEG.to_radians();
                angle = (angle / step).round() * step;
            }
            b.with_rotation(t0, angle)
        }
        Handle::Scale(hx, hy) => {
            let fixed = handle_point(b, -hx, -hy);
            let at = b.to_layer(t0, fixed);
            let (sin, cos) = (-t0.rotation).sin_cos();
            let d = p - at;
            let v = Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
            let diag = (handle_point(b, hx, hy) - fixed) * t0.scale;
            let mut s = t0.scale;
            let corner = hx != 0 && hy != 0;
            if corner && keep_aspect != shift {
                let len2 = diag.length_squared();
                if len2 > 0.0 {
                    let k = v.dot(diag) / len2;
                    s = t0.scale * k;
                }
            } else {
                if hx != 0 && diag.x != 0.0 {
                    s.x = t0.scale.x * v.x / diag.x;
                }
                if hy != 0 && diag.y != 0.0 {
                    s.y = t0.scale.y * v.y / diag.y;
                }
            }
            let floor = |v: f32, was: f32| {
                if !v.is_finite() {
                    was
                } else if v.abs() < MIN_SCALE {
                    MIN_SCALE * if v < 0.0 { -1.0 } else { 1.0 }
                } else {
                    v.clamp(-1.0e3, 1.0e3)
                }
            };
            let s = Vec2::new(floor(s.x, t0.scale.x), floor(s.y, t0.scale.y));
            b.with_scale(t0, s, fixed)
        }
    }
}

/// Draws the box, its handles and the rotate handle of the active layer (Move tool).
pub(crate) fn draw(painter: &egui::Painter, state: &mut AppState, geom: Geom, rect: Rect) {
    let Some(cam) = edit_of(state).map(|e| e.camera) else { return };
    let Some((_, b, t, locked)) = active_box(state) else { return };
    let corners = b.corners(&t).map(|c| geom.to_screen(&cam, rect, c));
    crate::view::draw_quad(painter, corners, crate::theme::ACCENT, 1.0);
    if locked {
        return;
    }
    let positions = handle_positions(&b, &t, geom, &cam, rect);
    let top = geom.to_screen(&cam, rect, b.to_layer(&t, handle_point(&b, 0, -1)));
    for (h, p) in positions {
        match h {
            Handle::Scale(..) => {
                let r = Rect::from_center_size(p, egui::vec2(HANDLE_HALF * 2.0, HANDLE_HALF * 2.0));
                painter.rect_filled(r, 0.0, Color32::WHITE);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, crate::theme::ACCENT), egui::StrokeKind::Outside);
            }
            Handle::Rotate => {
                painter.line_segment([top, p], Stroke::new(1.0, crate::theme::ACCENT));
                painter.circle_filled(p, HANDLE_HALF + 1.0, Color32::WHITE);
                painter.circle_stroke(p, HANDLE_HALF + 1.0, Stroke::new(1.0, crate::theme::ACCENT));
            }
        }
    }
}

/// The Tool tab's numbers of the active layer: width and height in layer-space pixels (`|scale| * box`), rotation
/// in degrees (clockwise), and the transforms the fields set (the box centre kept).
pub(crate) fn size_of(b: &LayerBox, t: &LayerTransform) -> Vec2 {
    b.size * t.scale.abs()
}

/// `t` with the width / height `size` (layer-space pixels; signs kept), the box centre kept.
pub(crate) fn with_size(b: &LayerBox, t: &LayerTransform, size: Vec2) -> LayerTransform {
    let s = |want: f32, len: f32, was: f32| {
        if !(want.is_finite() && len > 0.0) || want <= 0.0 {
            return was;
        }
        let v = (want / len).clamp(MIN_SCALE, 1.0e3);
        if was < 0.0 { -v } else { v }
    };
    let scale = Vec2::new(s(size.x, b.size.x, t.scale.x), s(size.y, b.size.y, t.scale.y));
    b.with_scale(t, scale, b.size * 0.5)
}

/// "Reset transform": scale 1, no rotation, no flip; the box centre kept.
pub(crate) fn reset(b: &LayerBox, t: &LayerTransform) -> LayerTransform {
    let turned = b.with_rotation(t, 0.0);
    b.with_scale(&turned, Vec2::ONE, b.size * 0.5)
}
