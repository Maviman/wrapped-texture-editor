//! The stamp transform gizmo (ARCHITECTURE.md 8.5 / 8.6): screen-space shape of a placement, handle hit test and
//! drawing.

use eframe::egui;
use wte_core::{Document, Placement, StampId, SurfaceMode};

use crate::tools::ViewXform;

/// Samples per picture edge of the wrapped outline (round 6, `Document::stamp_wrap_outline`).
pub const WRAP_OUTLINE_SAMPLES: usize = 32;

/// Pick tolerance of the handles (points).
pub const HANDLE_TOLERANCE: f32 = 8.0;
/// Distance of the rotate knob above the top edge (points).
pub const KNOB_OFFSET: f32 = 24.0;
/// Half side of a drawn corner handle (points).
const CORNER_HALF: f32 = 4.0;
/// Radius of the drawn rotate knob (points).
const KNOB_RADIUS: f32 = 5.0;

/// A grabbable part of the stamp gizmo.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum GizmoHandle {
    Body,
    /// 0 = TL, 1 = TR, 2 = BR, 3 = BL.
    Corner(u8),
    Rotate,
}

/// Screen-space shape of the gizmo of one stamp.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct GizmoShape {
    /// TL, TR, BR, BL.
    pub corners: [egui::Pos2; 4],
    pub center: egui::Pos2,
    pub rotate_knob: egui::Pos2,
    /// Projected decals in a 3D view: the projection box on screen, `[front, back]` rectangles (TL, TR, BR, BL of the
    /// placement rectangle moved by `+depth_front` / `-depth_back` along the projection axis). Drawn faintly, so the
    /// depth range is visible while the stamp is selected; not a handle. `None` for UV decals or when a box corner is
    /// behind the camera.
    pub depth_box: Option<[[egui::Pos2; 4]; 2]>,
    /// Round 6: the decal is wrapped on the surface. Its handles stay on the flat frame (hit tests and the self-test
    /// drags are unchanged); the frame is drawn faintly and the wrapped outline (`wrapped_outline`) is drawn by the
    /// caller. No projection box.
    pub wrapped: bool,
}

impl GizmoShape {
    /// Midpoint of the top edge (where the knob's stem starts).
    pub fn top_mid(&self) -> egui::Pos2 {
        self.corners[0].lerp(self.corners[1], 0.5)
    }

    /// Builds the shape from its corners and centre; the knob sits `KNOB_OFFSET` points beyond the top edge, away from
    /// the centre.
    pub fn from_corners(corners: [egui::Pos2; 4], center: egui::Pos2) -> Option<GizmoShape> {
        if !corners.iter().all(|c| c.is_finite()) || !center.is_finite() {
            return None;
        }
        let top = corners[0].lerp(corners[1], 0.5);
        let mut dir = top - center;
        if dir.length() < 1e-3 {
            dir = corners[0] - corners[3];
        }
        let dir = if dir.length() < 1e-3 { egui::vec2(0.0, -1.0) } else { dir.normalized() };
        Some(GizmoShape { corners, center, rotate_knob: top + dir * KNOB_OFFSET, depth_box: None, wrapped: false })
    }

    /// True when `pos` lies inside the (possibly non-convex) quad.
    pub fn contains(&self, pos: egui::Pos2) -> bool {
        // Crossing-number test.
        let mut inside = false;
        for i in 0..4 {
            let (a, b) = (self.corners[i], self.corners[(i + 1) % 4]);
            if (a.y > pos.y) != (b.y > pos.y) {
                let x = a.x + (pos.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if pos.x < x {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// The gizmo of a placement in a view; `None` when the placement kind does not match the view or it is off screen
/// (a corner behind the camera). For a UV placement the caller makes sure the view shows `placement.set`.
pub fn gizmo_shape(placement: &Placement, xf: &ViewXform) -> Option<GizmoShape> {
    match (placement, xf.is_3d) {
        (Placement::Projected(p), true) => {
            let [a, b, c, d] = p.corners().map(|w| xf.world_to_screen(w));
            let center = xf.world_to_screen(p.origin)?;
            let mut shape = GizmoShape::from_corners([a?, b?, c?, d?], center)?;
            if p.mode == SurfaceMode::Wrap {
                shape.wrapped = true;
            } else {
                shape.depth_box = projection_box(p, xf);
            }
            Some(shape)
        }
        (Placement::Uv(u), false) => {
            GizmoShape::from_corners(u.corners_px().map(|c| xf.px_to_screen(c)), xf.px_to_screen(u.center_px))
        }
        _ => None,
    }
}

/// Round 6 (R6-2): the outline of wrapped stamp `id` in a 3D view: the picture's border on the surface
/// (`Document::stamp_wrap_outline`, from the cached parameterization; empty without one), as screen polylines. Points
/// on surfaces facing away from the camera (the back of a bottle the sticker goes round) are dropped, which splits the
/// line there.
pub fn wrapped_outline(doc: &Document, id: StampId, xf: &ViewXform) -> Vec<Vec<egui::Pos2>> {
    if !xf.is_3d {
        return Vec::new();
    }
    let eye = xf.camera.eye();
    let mut lines = Vec::new();
    for run in doc.stamp_wrap_outline(id, WRAP_OUTLINE_SAMPLES) {
        let mut current: Vec<egui::Pos2> = Vec::new();
        for sp in run {
            let facing = sp.normal.dot(eye - sp.position) > 0.0;
            match facing.then(|| xf.world_to_screen(sp.position)).flatten().filter(|p| p.is_finite()) {
                Some(p) => current.push(p),
                None => {
                    if current.len() >= 2 {
                        lines.push(std::mem::take(&mut current));
                    }
                    current.clear();
                }
            }
        }
        if current.len() >= 2 {
            lines.push(current);
        }
    }
    lines
}

/// The projection box of a projected decal on screen: `[front, back]` rectangles; `None` when a corner does not
/// project (behind the camera) or a depth is not finite.
fn projection_box(p: &wte_core::Projector, xf: &ViewXform) -> Option<[[egui::Pos2; 4]; 2]> {
    let (_, _, out) = p.axes();
    let (front, back) = (p.depth_front.max(0.0), p.depth_back.max(0.0));
    if !(front.is_finite() && back.is_finite()) {
        return None;
    }
    let rect = |offset: glam::Vec3| -> Option<[egui::Pos2; 4]> {
        let [a, b, c, d] = p.corners().map(|w| xf.world_to_screen(w + offset));
        Some([a?, b?, c?, d?])
    };
    Some([rect(out * front)?, rect(-out * back)?])
}

/// The handle under `pos` (8-point tolerance on handles).
pub fn hit_test(shape: &GizmoShape, pos: egui::Pos2) -> Option<GizmoHandle> {
    if !pos.is_finite() {
        return None;
    }
    if pos.distance(shape.rotate_knob) <= HANDLE_TOLERANCE {
        return Some(GizmoHandle::Rotate);
    }
    // A tiny gizmo is moved by its body: otherwise its corner handles would cover all of it.
    let shortest = (0..4).map(|i| shape.corners[i].distance(shape.corners[(i + 1) % 4])).fold(f32::INFINITY, f32::min);
    if shortest < 2.0 * HANDLE_TOLERANCE && shape.contains(pos) {
        return Some(GizmoHandle::Body);
    }
    let nearest = (0..4u8)
        .map(|i| (i, shape.corners[i as usize].distance(pos)))
        .filter(|(_, d)| *d <= HANDLE_TOLERANCE)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = nearest {
        return Some(GizmoHandle::Corner(i));
    }
    shape.contains(pos).then_some(GizmoHandle::Body)
}

/// Draws the gizmo; `hot` is highlighted.
pub fn draw(painter: &egui::Painter, shape: &GizmoShape, hot: Option<GizmoHandle>) {
    let accent = crate::theme::ACCENT;
    let white = egui::Color32::WHITE;
    let shadow = egui::Stroke::new(3.5, egui::Color32::from_black_alpha(160));
    let body_color = if hot == Some(GizmoHandle::Body) { accent } else { white };
    // The projection box first, faint: front and back rectangles plus the four depth edges.
    if let Some([front, back]) = shape.depth_box {
        let faint = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(70));
        let shadow = egui::Stroke::new(2.0, egui::Color32::from_black_alpha(45));
        for stroke in [shadow, faint] {
            painter.add(egui::Shape::closed_line(front.to_vec(), stroke));
            painter.add(egui::Shape::closed_line(back.to_vec(), stroke));
            for i in 0..4 {
                painter.line_segment([front[i], back[i]], stroke);
            }
        }
    }
    if hot == Some(GizmoHandle::Body) {
        painter.add(egui::Shape::convex_polygon(shape.corners.to_vec(), accent.gamma_multiply(0.12), egui::Stroke::NONE));
    }
    let outline = shape.corners.to_vec();
    if shape.wrapped {
        // Round 6: the flat frame only carries the handles; the wrapped outline on the surface is the stamp's edge.
        let faint = egui::Stroke::new(1.0, body_color.gamma_multiply(0.45));
        painter.add(egui::Shape::closed_line(outline, faint));
    } else {
        painter.add(egui::Shape::closed_line(outline.clone(), shadow));
        painter.add(egui::Shape::closed_line(outline, egui::Stroke::new(1.5, body_color)));
    }

    // Rotate knob on a stem above the top edge.
    let top = shape.top_mid();
    let knob_color = if hot == Some(GizmoHandle::Rotate) { accent } else { white };
    painter.line_segment([top, shape.rotate_knob], shadow);
    painter.line_segment([top, shape.rotate_knob], egui::Stroke::new(1.5, knob_color));
    painter.circle(shape.rotate_knob, KNOB_RADIUS, knob_color, egui::Stroke::new(1.0, egui::Color32::BLACK));

    // Corner handles.
    for (i, c) in shape.corners.iter().enumerate() {
        let color = if hot == Some(GizmoHandle::Corner(i as u8)) { accent } else { white };
        let r = egui::Rect::from_center_size(*c, egui::Vec2::splat(2.0 * CORNER_HALF));
        painter.rect(r, 1.0, color, egui::Stroke::new(1.0, egui::Color32::BLACK), egui::StrokeKind::Outside);
    }
    // Centre mark.
    painter.circle_filled(shape.center, 2.0, body_color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::*;
    use glam::{Vec2, Vec3};
    use wte_core::{Projector, SampleKind, TextureSetId, UvPlacement};

    fn square() -> GizmoShape {
        let corners =
            [egui::pos2(100.0, 100.0), egui::pos2(200.0, 100.0), egui::pos2(200.0, 200.0), egui::pos2(100.0, 200.0)];
        GizmoShape::from_corners(corners, egui::pos2(150.0, 150.0)).unwrap()
    }

    #[test]
    fn knob_sits_above_the_top_edge() {
        let s = square();
        assert!((s.rotate_knob - egui::pos2(150.0, 100.0 - KNOB_OFFSET)).length() < 1e-4);
    }

    #[test]
    fn hit_test_finds_every_handle() {
        let s = square();
        assert_eq!(hit_test(&s, egui::pos2(150.0, 76.0)), Some(GizmoHandle::Rotate));
        assert_eq!(hit_test(&s, egui::pos2(150.0 + 7.0, 76.0)), Some(GizmoHandle::Rotate));
        assert_eq!(hit_test(&s, egui::pos2(103.0, 104.0)), Some(GizmoHandle::Corner(0)));
        assert_eq!(hit_test(&s, egui::pos2(206.0, 97.0)), Some(GizmoHandle::Corner(1)));
        assert_eq!(hit_test(&s, egui::pos2(199.0, 199.0)), Some(GizmoHandle::Corner(2)));
        assert_eq!(hit_test(&s, egui::pos2(95.0, 205.0)), Some(GizmoHandle::Corner(3)));
        assert_eq!(hit_test(&s, egui::pos2(150.0, 150.0)), Some(GizmoHandle::Body));
        assert_eq!(hit_test(&s, egui::pos2(120.0, 180.0)), Some(GizmoHandle::Body));
        assert_eq!(hit_test(&s, egui::pos2(90.0, 150.0)), None);
        assert_eq!(hit_test(&s, egui::pos2(150.0, 60.0)), None);
        assert_eq!(hit_test(&s, egui::pos2(300.0, 300.0)), None);
    }

    #[test]
    fn rotated_and_tiny_shapes() {
        // A diamond: the knob points away from the centre through the top edge midpoint.
        let corners = [egui::pos2(0.0, -50.0), egui::pos2(50.0, 0.0), egui::pos2(0.0, 50.0), egui::pos2(-50.0, 0.0)];
        let s = GizmoShape::from_corners(corners, egui::Pos2::ZERO).unwrap();
        let dir = (s.rotate_knob - s.top_mid()).normalized();
        assert!((dir - egui::vec2(1.0, -1.0).normalized()).length() < 1e-4);
        assert_eq!(hit_test(&s, egui::pos2(10.0, 10.0)), Some(GizmoHandle::Body));
        assert_eq!(hit_test(&s, egui::pos2(60.0, 0.0)), None);
        // A 6-point gizmo: inside is the body, not a corner.
        let corners = [egui::pos2(0.0, 0.0), egui::pos2(6.0, 0.0), egui::pos2(6.0, 6.0), egui::pos2(0.0, 6.0)];
        let s = GizmoShape::from_corners(corners, egui::pos2(3.0, 3.0)).unwrap();
        assert_eq!(hit_test(&s, egui::pos2(1.0, 1.0)), Some(GizmoHandle::Body));
        assert_eq!(hit_test(&s, egui::pos2(-5.0, -5.0)), Some(GizmoHandle::Corner(0)));
    }

    #[test]
    fn shapes_follow_the_view_kind() {
        let mut state = state_with(SampleKind::Sphere);
        let xf3 = xf_3d(&mut state);
        let xf2 = xf_2d(&mut state);
        let uv = Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(512.0, 256.0),
            size_px: Vec2::new(100.0, 50.0),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        assert!(gizmo_shape(&uv, &xf3).is_none());
        let s = gizmo_shape(&uv, &xf2).unwrap();
        assert!((s.center - xf2.px_to_screen(Vec2::new(512.0, 256.0))).length() < 1e-3);
        assert!(s.corners[1].x > s.corners[0].x && s.corners[3].y > s.corners[0].y);

        let doc = state.doc().unwrap();
        let center = doc.mesh.bounds.center();
        let d = doc.mesh.bounds.diagonal();
        let front = center + Vec3::new(0.0, 0.0, d * 0.29);
        let proj = Placement::Projected(Projector::from_surface(front, Vec3::Z, Vec3::Y, Vec2::splat(d * 0.1), d * 0.1));
        assert!(gizmo_shape(&proj, &xf2).is_none());
        let s = gizmo_shape(&proj, &xf3).unwrap();
        // Seen from the front: TL is left of TR and above BL; the knob is above the top edge.
        assert!(s.corners[0].x < s.corners[1].x && s.corners[0].y < s.corners[3].y);
        assert!(s.rotate_knob.y < s.top_mid().y);
        assert_eq!(hit_test(&s, s.center), Some(GizmoHandle::Body));

        // The projection box of the projected decal: seen from the front, the front rectangle (toward the camera) is
        // drawn larger than the placement rectangle and the back one smaller; a UV decal has none.
        assert!(gizmo_shape(&uv, &xf2).unwrap().depth_box.is_none());
        let [front, back] = s.depth_box.expect("projection box");
        let width = |r: [egui::Pos2; 4]| (r[1] - r[0]).length();
        assert!(width(front) > width(s.corners) && width(back) < width(s.corners), "{front:?} {back:?}");
        // The box follows the depth range.
        let Placement::Projected(mut deep) = proj else { unreachable!() };
        deep.depth_back *= 3.0;
        let [_, deep_back] = gizmo_shape(&Placement::Projected(deep), &xf3).unwrap().depth_box.unwrap();
        assert!(width(deep_back) < width(back));
        // Drawing it does not panic.
        let ctx = egui::Context::default();
        crate::theme::run_test_pass(&ctx, egui::RawInput::default(), |ui| draw(ui.painter(), &s, Some(GizmoHandle::Body)));
    }

    /// Round 6 (R6-2, ROUND6_PLAN 5.3): a wrapped decal keeps the handles of its flat frame but has no projection
    /// box; its outline on the surface comes from the cached parameterization: empty after an undo (the caches come
    /// back, the parameterization does not), on screen again after `ensure_wrap_param`.
    #[test]
    fn a_wrapped_decal_has_no_projection_box_and_an_outline_on_the_surface() {
        use wte_core::{Commit, StampSource, SurfaceMode, samples};
        let mut state = state_with(SampleKind::Sphere);
        let xf3 = xf_3d(&mut state);
        let img = samples::sample_stamp_image(32);
        let png = wte_core::encode_png(img.width, img.height, &img.data, wte_core::PngLevel::Fast).unwrap();
        let doc = state.doc_mut().unwrap();
        let blob = doc.add_image_blob("ring.png", std::sync::Arc::from(png)).unwrap();
        let (center, d) = (doc.mesh.bounds.center(), doc.mesh.bounds.diagonal());
        let front = center + Vec3::new(0.0, 0.0, d * 0.29);
        let projected = Projector::from_surface(front, Vec3::Z, Vec3::Y, Vec2::new(0.12, 0.06) * d, d * 0.1);
        let wrapped = Projector { mode: SurfaceMode::Wrap, ..projected };
        let flat = gizmo_shape(&Placement::Projected(projected), &xf3).unwrap();
        let shape = gizmo_shape(&Placement::Projected(wrapped), &xf3).unwrap();
        assert!(shape.depth_box.is_none() && shape.wrapped && !flat.wrapped);
        assert_eq!((shape.corners, shape.center, shape.rotate_knob), (flat.corners, flat.center, flat.rotate_knob));

        let id = doc.add_stamp("ring", StampSource::Image { blob }, Placement::Projected(wrapped), TextureSetId(0)).unwrap();
        let outline = wrapped_outline(doc, id, &xf3);
        assert!(!outline.is_empty(), "the bake cached the parameterization");
        let points: Vec<egui::Pos2> = outline.iter().flatten().copied().collect();
        assert!(points.len() >= 4 * WRAP_OUTLINE_SAMPLES / 2, "{} points", points.len());
        assert!(points.iter().all(|p| xf3.rect.expand(1.0).contains(*p)), "the outline lies in the view");
        // Around the flat frame's centre on screen, inside its handles' reach.
        let mid = points.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / points.len() as f32;
        assert!((mid.to_pos2() - shape.center).length() < 0.25 * (shape.corners[1] - shape.corners[0]).length());

        let moved = wrapped.moved_to(center + Vec3::new(d * 0.2, 0.0, d * 0.2), Vec3::new(1.0, 0.0, 1.0).normalize());
        doc.set_stamp_placement(id, Placement::Projected(moved), Commit::Final).unwrap();
        doc.undo().expect("an entry to undo");
        assert!(wrapped_outline(doc, id, &xf3).is_empty(), "no parameterization of the restored placement yet");
        assert!(doc.ensure_wrap_param(id).is_some());
        assert_eq!(wrapped_outline(doc, id, &xf3).iter().flatten().count(), points.len());
        // A UV view has no wrapped outline.
        let xf2 = xf_2d(&mut state);
        assert!(wrapped_outline(state.doc().unwrap(), id, &xf2).is_empty());
    }
}
