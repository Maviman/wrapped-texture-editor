// Owner: T7 text tools (round 4).
//
// The round points of a text on a Curve (FEATURE_TEXT 5.5, 6.5): drawn in the view over the selected text and dragged
// with the Transform tool (a drag is `Preview`s and one `Final`; Esc restores the curve). `transform.rs` routes the
// pointer here before its corners and after its knob.
//
// Positions come from `EmMap` (FEATURE_TEXT 5.11): the curve points are em of text space relative to the anchor; for
// a projected text they lie on the projector plane (where the gizmo corners lie too) and the pointer is brought back
// by cutting the camera ray with that plane; for a UV text through the texel position of the pointer.

use eframe::egui;
use glam::Vec3;
use wte_core::text::EmMap;
use wte_core::{
    Document, PATH_COORD_MAX_EM, PathShape, Placement, StampId, StampSource, SurfaceMode, TextFrame, TextPath, TextSpec,
};

use crate::state::ViewId;
use crate::tools::ViewXform;

/// Radius of a drawn curve point, in points.
pub(crate) const POINT_RADIUS_PT: f32 = 5.0;
/// A press within this distance of a curve point (points) grabs it.
pub(crate) const HIT_PT: f32 = 8.0;
/// Steps per piece of the drawn curve (33 points per piece).
const CURVE_STEPS: usize = 32;
/// Two neighbouring points closer than this (em) count as this far apart for the knot spacing.
const MIN_KNOT_EM: f32 = 1e-5;

/// Screen positions of the curve points and of the drawn curve.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PathPoints {
    pub points: Vec<egui::Pos2>,
    pub curve: Vec<egui::Pos2>,
}

/// A curve point drag of the Transform tool.
#[derive(Clone, Debug)]
pub(crate) struct PathDrag {
    pub id: StampId,
    pub view: ViewId,
    pub point: u8,
    /// Offset from the pointer to the point on screen at the press.
    pub grab: egui::Vec2,
    /// A `Preview` was accepted: release commits it.
    pub moved: bool,
    /// The reason of a refused `Preview` was shown (once per drag).
    pub refused_said: bool,
}

/// The points of a Curve, if `path` is one.
pub(crate) fn curve_points(path: &TextPath) -> Option<&[[f32; 2]]> {
    match &path.shape {
        PathShape::Curve { points } => Some(points),
        PathShape::Arc { .. } | PathShape::Circle { .. } => None,
    }
}

/// The selected text stamp `id` with a Curve: its placement and spec.
pub(crate) fn curve_stamp(doc: &Document, id: StampId) -> Option<(Placement, TextSpec)> {
    let stamp = doc.stamp(id)?;
    let StampSource::Text(spec) = &stamp.source else { return None };
    curve_points(spec.fx.path.as_ref()?)?;
    Some((stamp.placement, spec.clone()))
}

/// `spec` with curve point `index` at `em` (clamped to the coordinate range of FEATURE_TEXT 4.1, so a drag far out
/// stops at 60 em from the anchor); `None` when the spec has no such point.
pub(crate) fn with_point(spec: &TextSpec, index: u8, em: [f32; 2]) -> Option<TextSpec> {
    let mut out = spec.clone();
    let path = out.fx.path.as_mut()?;
    let PathShape::Curve { points } = &mut path.shape else { return None };
    let slot = points.get_mut(index as usize)?;
    *slot = clamp_em(em);
    Some(out)
}

/// Each coordinate within `-PATH_COORD_MAX_EM..=PATH_COORD_MAX_EM`; a non-finite one becomes 0.
pub(crate) fn clamp_em(em: [f32; 2]) -> [f32; 2] {
    em.map(|v| if v.is_finite() { v.clamp(-PATH_COORD_MAX_EM, PATH_COORD_MAX_EM) } else { 0.0 })
}

/// True when `placement` is edited in this view (a projected text in a 3D view, a UV text in a 2D view).
fn view_fits(placement: &Placement, xf: &ViewXform) -> bool {
    matches!((placement, xf.is_3d), (Placement::Projected(_), true) | (Placement::Uv(_), false))
}

/// Screen position of an em point of text space.
fn em_to_screen(map: &EmMap, placement: &Placement, xf: &ViewXform, em: [f32; 2]) -> Option<egui::Pos2> {
    let pos = match placement {
        Placement::Projected(_) => xf.world_to_screen(map.to_world(em))?,
        Placement::Uv(_) => xf.px_to_screen(map.to_texel(em)),
    };
    pos.is_finite().then_some(pos)
}

/// The curve points and the drawn curve of a text on a Curve in this view; `None` for another kind of line, a view
/// that does not edit this placement, or a point behind the camera.
pub(crate) fn path_points(placement: &Placement, frame: &TextFrame, path: &TextPath, xf: &ViewXform) -> Option<PathPoints> {
    let pts = curve_points(path)?;
    if !view_fits(placement, xf) || pts.is_empty() {
        return None;
    }
    let map = EmMap::new(placement, frame);
    let points = pts.iter().map(|&em| em_to_screen(&map, placement, xf, em)).collect::<Option<Vec<_>>>()?;
    let curve = sample_curve(pts).into_iter().filter_map(|em| em_to_screen(&map, placement, xf, em)).collect();
    Some(PathPoints { points, curve })
}

/// Round 6 (R6-2): the projected placement when it is wrapped.
fn wrapped_projector(placement: &Placement) -> Option<wte_core::Projector> {
    match placement {
        Placement::Projected(p) if p.mode == SurfaceMode::Wrap => Some(*p),
        _ => None,
    }
}

/// Round 6 (R6-2): the world position of an em point of a wrapped text `id`: the point of the flat frame
/// (`EmMap::to_world`) taken to the surface through the stamp's parameterization (`Document::stamp_wrap_surface`
/// of its stamp-local coordinates); the flat point where the picture has no surface or no parameterization is cached.
pub(crate) fn wrapped_world(doc: &Document, id: StampId, p: &wte_core::Projector, map: &EmMap, em: [f32; 2]) -> Vec3 {
    let flat = map.to_world(em);
    let (right, up, _) = p.axes();
    let d = flat - p.origin;
    let local = glam::Vec2::new(d.dot(right), d.dot(up));
    match doc.stamp_wrap_surface(id, local) {
        Some(sp) if sp.position.is_finite() => sp.position,
        _ => flat,
    }
}

/// `path_points` for the selected text `id` of `doc`: in Wrap mode the curve points and the drawn curve lie on the
/// surface (`wrapped_world`); otherwise as `path_points`.
pub(crate) fn path_points_on(
    doc: &Document,
    id: StampId,
    placement: &Placement,
    frame: &TextFrame,
    path: &TextPath,
    xf: &ViewXform,
) -> Option<PathPoints> {
    let Some(p) = wrapped_projector(placement) else { return path_points(placement, frame, path, xf) };
    let pts = curve_points(path)?;
    if !view_fits(placement, xf) || pts.is_empty() {
        return None;
    }
    let map = EmMap::new(placement, frame);
    let screen = |em: [f32; 2]| xf.world_to_screen(wrapped_world(doc, id, &p, &map, em)).filter(|s| s.is_finite());
    let points = pts.iter().map(|&em| screen(em)).collect::<Option<Vec<_>>>()?;
    let curve = sample_curve(pts).into_iter().filter_map(screen).collect();
    Some(PathPoints { points, curve })
}

/// `screen_to_em` for the text `id` of `doc`: in Wrap mode the pointer's ray hits the model and the hit is taken to
/// the stamp's flat frame through its parameterization (`Document::stamp_wrap_local`); where the ray misses the model
/// or the picture has no parameterization there, the projector plane as for a projected text.
pub(crate) fn screen_to_em_on(
    doc: &Document,
    id: StampId,
    placement: &Placement,
    frame: &TextFrame,
    xf: &ViewXform,
    pos: egui::Pos2,
) -> Option<[f32; 2]> {
    if let Some(p) = wrapped_projector(placement)
        && view_fits(placement, xf)
        && pos.is_finite()
        && let Some(hit) = doc.raycast(&xf.camera.ray(pos, xf.rect, xf.scene_radius))
        && let Some(local) = doc.stamp_wrap_local(id, hit.tri, hit.bary)
    {
        let (right, up, _) = p.axes();
        let em = EmMap::new(placement, frame).from_world(p.origin + right * local.x + up * local.y);
        if em.iter().all(|v| v.is_finite()) {
            return Some(em);
        }
    }
    screen_to_em(placement, frame, xf, pos)
}

/// The curve point under `pos` (within `HIT_PT`; the nearest when several are).
pub(crate) fn hit(points: &PathPoints, pos: egui::Pos2) -> Option<u8> {
    points
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.distance(pos)))
        .filter(|(_, d)| *d <= HIT_PT)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .and_then(|(i, _)| u8::try_from(i).ok())
}

/// The em point of text space under the screen position `pos`: in 3D where the camera ray cuts the projector plane
/// (`None` when it runs along the plane or the plane is behind the camera), in 2D through the texel under `pos`.
pub(crate) fn screen_to_em(placement: &Placement, frame: &TextFrame, xf: &ViewXform, pos: egui::Pos2) -> Option<[f32; 2]> {
    if !view_fits(placement, xf) || !pos.is_finite() {
        return None;
    }
    let map = EmMap::new(placement, frame);
    let em = match placement {
        Placement::Projected(_) => {
            let ray = xf.camera.ray(pos, xf.rect, xf.scene_radius);
            let (point, normal) = map.plane();
            let along = ray.dir.dot(normal);
            if along.abs() < 1e-6 {
                return None;
            }
            let t = (point - ray.origin).dot(normal) / along;
            if !(t.is_finite() && t >= 0.0) {
                return None;
            }
            map.from_world(ray.origin + ray.dir * t)
        }
        Placement::Uv(_) => map.from_texel(xf.screen_to_px(pos)),
    };
    em.iter().all(|v| v.is_finite()).then_some(em)
}

/// The centripetal Catmull-Rom spline through `points` (the line the text follows, FEATURE_TEXT 5.5), sampled with
/// `CURVE_STEPS + 1` points per piece, in em. Two points give a straight piece.
pub(crate) fn sample_curve(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = points.len();
    if n < 2 {
        return points.to_vec();
    }
    let p = |i: isize| -> Vec3 {
        let at = |k: usize| Vec3::new(points[k][0], points[k][1], 0.0);
        if i < 0 {
            at(0) * 2.0 - at(1)
        } else if i as usize >= n {
            at(n - 1) * 2.0 - at(n - 2)
        } else {
            at(i as usize)
        }
    };
    let mut out = Vec::with_capacity((n - 1) * (CURVE_STEPS + 1));
    for i in 0..n as isize - 1 {
        let (p0, p1, p2, p3) = (p(i - 1), p(i), p(i + 1), p(i + 2));
        let (c1, c2) = if n == 2 {
            let d = p2 - p1;
            (p1 + d / 3.0, p2 - d / 3.0)
        } else {
            let knot = |a: Vec3, b: Vec3| (b - a).length().max(MIN_KNOT_EM).sqrt();
            let (t1, t2) = (knot(p0, p1), knot(p0, p1) + knot(p1, p2));
            let t3 = t2 + knot(p2, p3);
            let m1 = ((p1 - p0) / t1 - (p2 - p0) / t2 + (p2 - p1) / (t2 - t1)) * (t2 - t1);
            let m2 = ((p2 - p1) / (t2 - t1) - (p3 - p1) / (t3 - t1) + (p3 - p2) / (t3 - t2)) * (t2 - t1);
            (p1 + m1 / 3.0, p2 - m2 / 3.0)
        };
        for j in 0..=CURVE_STEPS {
            let t = j as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            let q = p1 * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + p2 * (t * t * t);
            out.push([q.x, q.y]);
        }
    }
    out
}

/// Draws the curve as a thin line and one round point per curve point (`hot` filled), in a colour different from the
/// gizmo's.
pub(crate) fn draw(painter: &egui::Painter, points: &PathPoints, hot: Option<u8>) {
    let colour = egui::Color32::from_rgb(90, 200, 255);
    if points.curve.len() >= 2 {
        painter.add(egui::Shape::line(points.curve.clone(), egui::Stroke::new(1.25, colour)));
    }
    for (i, p) in points.points.iter().enumerate() {
        let fill = if hot == u8::try_from(i).ok() { colour } else { egui::Color32::from_black_alpha(160) };
        painter.circle(*p, POINT_RADIUS_PT, fill, egui::Stroke::new(1.5, colour));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::*;
    use glam::Vec2;
    use wte_core::{SampleKind, TextureSetId, UvPlacement};

    fn frame() -> TextFrame {
        TextFrame {
            size: [400, 200],
            anchor_px: [180.0, 120.0],
            em_px: 64.0,
            box_px: [300.0, 80.0],
            baseline0_px: 0.0,
            align_px: [180.0, 120.0],
            circle_fill: 0.0,
        }
    }

    fn curve(points: &[[f32; 2]]) -> TextPath {
        TextPath { shape: PathShape::Curve { points: points.to_vec() }, start: 0.0, flip: false }
    }

    #[test]
    fn points_map_to_the_screen_and_back() {
        let mut state = state_with(SampleKind::Sphere);
        let path = curve(&[[-2.0, 0.3], [0.0, -0.5], [1.5, 0.2], [2.5, 1.0]]);
        let pts = curve_points(&path).unwrap().to_vec();
        // 2D: a turned UV placement.
        let xf = xf_2d(&mut state);
        let u = UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(500.0, 420.0),
            size_px: Vec2::new(300.0, 150.0),
            rotation: 0.4,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        };
        let placement = Placement::Uv(u);
        let screen = path_points(&placement, &frame(), &path, &xf).expect("2D points");
        assert_eq!(screen.points.len(), 4);
        assert_eq!(screen.curve.len(), 3 * 33);
        for (em, s) in pts.iter().zip(&screen.points) {
            let back = screen_to_em(&placement, &frame(), &xf, *s).unwrap();
            assert!((back[0] - em[0]).abs() < 1e-3 && (back[1] - em[1]).abs() < 1e-3, "{em:?} -> {s:?} -> {back:?}");
        }
        // The curve passes through the points.
        assert!(screen.curve.first().unwrap().distance(screen.points[0]) < 1e-3);
        assert!(screen.curve.last().unwrap().distance(screen.points[3]) < 1e-3);
        // Not in the 3D view.
        let xf3 = xf_3d(&mut state);
        assert!(path_points(&placement, &frame(), &path, &xf3).is_none());

        // 3D: a projector facing the camera on the front of the sphere.
        let doc = state.doc().unwrap();
        let wte_core_hit = match xf3.surface_at(doc, TextureSetId(0), xf3.rect.center()) {
            crate::tools::Surface::Model(hit) => hit,
            _ => panic!("the centre misses"),
        };
        let d = doc.mesh.bounds.diagonal();
        let (_, up, _) = xf3.camera.basis();
        let pr = wte_core::Projector::from_surface(wte_core_hit.position, wte_core_hit.normal, up, Vec2::new(0.2, 0.1) * d, 0.1 * d)
            .rotated(0.3);
        let placement = Placement::Projected(pr);
        let screen = path_points(&placement, &frame(), &path, &xf3).expect("3D points");
        for (em, s) in pts.iter().zip(&screen.points) {
            let back = screen_to_em(&placement, &frame(), &xf3, *s).unwrap();
            assert!((back[0] - em[0]).abs() < 1e-3 && (back[1] - em[1]).abs() < 1e-3, "{em:?} -> {s:?} -> {back:?}");
        }
        assert!(path_points(&placement, &frame(), &path, &xf).is_none(), "not in the 2D view");
    }

    /// Round 6 (R6-2, ROUND6_PLAN 5.3): the curve points of a wrapped text lie on the surface (distance to the mesh
    /// below 1e-4 of the diagonal), where the flat frame's points do not; a handle's screen position maps back to its
    /// em point through the parameterization.
    #[test]
    fn the_handles_of_a_wrapped_curved_text_lie_on_the_surface() {
        use wte_core::{Commit, Projector, StampSource, SurfaceMode};
        let mut state = state_with(SampleKind::Sphere);
        let xf3 = xf_3d(&mut state);
        let doc = state.doc_mut().unwrap();
        let crate::tools::Surface::Model(hit) = xf3.surface_at(doc, TextureSetId(0), xf3.rect.center()) else { panic!("the centre misses") };
        let (center, d) = (doc.mesh.bounds.center(), doc.mesh.bounds.diagonal());
        let (_, up, _) = xf3.camera.basis();
        let flat = Projector::from_surface(hit.position, hit.normal, up, Vec2::new(0.25, 0.06) * d, 0.1 * d);
        let mut spec = TextSpec::new("WRAP AROUND", doc.default_font());
        let placement = Placement::Projected(Projector { mode: SurfaceMode::Wrap, ..flat });
        let id = doc.add_stamp("t", StampSource::Text(spec.clone()), placement, TextureSetId(0)).unwrap();
        let frame = doc.stamp_text_frame(id).unwrap();
        let mut path = wte_core::text::path::new_curve(&frame);
        if let PathShape::Curve { points } = &mut path.shape {
            points[1][1] += 1.0;
        }
        spec.fx.path = Some(path);
        doc.set_stamp_source(id, StampSource::Text(spec), Commit::Final).unwrap();
        let (placement, spec) = curve_stamp(doc, id).expect("a text on a Curve");
        let Placement::Projected(p) = placement else { panic!() };
        assert!(doc.stamp_wrap_stats(id).is_some(), "the Final bake cached the parameterization");
        let frame = doc.stamp_text_frame(id).unwrap();
        let doc: &Document = doc;
        let path = spec.fx.path.clone().unwrap();
        let map = EmMap::new(&placement, &frame);
        let to_mesh = |w: Vec3| {
            let ray = wte_core::Ray::new(center, (w - center).normalize());
            let hit = doc.raycast(&ray).expect("a ray from the centre hits the sphere");
            (hit.position - w).length()
        };
        let mut flat_off = 0.0f32;
        for &em in curve_points(&path).unwrap() {
            let w = wrapped_world(doc, id, &p, &map, em);
            assert!(to_mesh(w) < 1e-4 * d, "{em:?}: {} from the mesh", to_mesh(w));
            flat_off = flat_off.max(to_mesh(map.to_world(em)));
        }
        assert!(flat_off > 1e-2 * d, "the flat frame's ends stand off the sphere ({flat_off})");
        // On screen, and back to em through the surface.
        let screen = path_points_on(doc, id, &placement, &frame, &path, &xf3).expect("3D points");
        assert_eq!(screen.points.len(), 3);
        assert!(screen.curve.len() >= 2 * 33);
        for (em, s) in curve_points(&path).unwrap().iter().zip(&screen.points) {
            let back = screen_to_em_on(doc, id, &placement, &frame, &xf3, *s).unwrap();
            assert!((back[0] - em[0]).abs() < 0.05 && (back[1] - em[1]).abs() < 0.05, "{em:?} -> {s:?} -> {back:?}");
        }
        // A projected text keeps the plane mapping of `path_points`.
        let projected = Placement::Projected(flat);
        assert_eq!(path_points_on(doc, id, &projected, &frame, &path, &xf3), path_points(&projected, &frame, &path, &xf3));
    }

    #[test]
    fn hit_takes_the_nearest_point_within_8_pt() {
        let p = PathPoints { points: vec![egui::pos2(0.0, 0.0), egui::pos2(10.0, 0.0), egui::pos2(100.0, 0.0)], curve: Vec::new() };
        assert_eq!(hit(&p, egui::pos2(6.0, 0.0)), Some(1));
        assert_eq!(hit(&p, egui::pos2(4.0, 1.0)), Some(0));
        assert_eq!(hit(&p, egui::pos2(100.0, 7.9)), Some(2));
        assert_eq!(hit(&p, egui::pos2(100.0, 8.1)), None);
        assert_eq!(hit(&p, egui::pos2(55.0, 0.0)), None);
    }

    #[test]
    fn with_point_moves_one_point_within_the_range() {
        let mut spec = TextSpec::new("Hi", wte_core::FontRef {
            blob: wte_core::BlobId(0),
            family: "F".into(),
            style: "R".into(),
            face_index: 0,
        });
        assert!(with_point(&spec, 0, [1.0, 1.0]).is_none(), "no path");
        spec.fx.path = Some(curve(&[[-1.0, 0.0], [0.0, 0.0], [1.0, 0.0]]));
        let moved = with_point(&spec, 1, [0.5, -2.0]).unwrap();
        assert_eq!(curve_points(moved.fx.path.as_ref().unwrap()).unwrap(), &[[-1.0, 0.0], [0.5, -2.0], [1.0, 0.0]]);
        let far = with_point(&spec, 2, [500.0, f32::NAN]).unwrap();
        assert_eq!(curve_points(far.fx.path.as_ref().unwrap()).unwrap()[2], [PATH_COORD_MAX_EM, 0.0]);
        assert!(with_point(&spec, 3, [0.0, 0.0]).is_none());
    }

    #[test]
    fn the_sampled_curve_is_the_painters_line() {
        // Three points: the curve runs through all three, and two points give a straight piece.
        let pts = [[-3.0, 0.0], [0.0, -1.0], [3.0, 0.0]];
        let s = sample_curve(&pts);
        assert_eq!(s.len(), 66);
        assert!(s.iter().any(|p| (p[0] - 0.0).abs() < 1e-4 && (p[1] + 1.0).abs() < 1e-4));
        let line = sample_curve(&[[0.0, 0.0], [3.0, 3.0]]);
        assert!(line.iter().all(|p| (p[0] - p[1]).abs() < 1e-5));
    }
}
