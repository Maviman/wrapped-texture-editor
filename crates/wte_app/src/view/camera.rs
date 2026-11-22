//! Orbit camera of the 3D views (ARCHITECTURE.md 8.5): eye position, view-projection, picking rays, projection of
//! world points to the screen and the orbit / pan / zoom / frame / preset navigation.

use eframe::egui;
use glam::{Mat4, Vec3, Vec4};

use crate::state::{CameraPreset, OrbitCamera};

/// Radians of yaw / pitch per point of pointer motion.
const ORBIT_RAD_PER_POINT: f32 = 0.008;
/// Pitch limit (+-89 degrees).
const MAX_PITCH: f32 = 89.0 * std::f32::consts::PI / 180.0;
/// Zoom limits as multiples of the model radius.
const MIN_DISTANCE_FACTOR: f32 = 0.02;
const MAX_DISTANCE_FACTOR: f32 = 50.0;
/// Margin of `frame` around the bounding sphere.
const FRAME_MARGIN: f32 = 1.15;
/// Lower limit for radii and distances derived from an empty or degenerate model.
const MIN_RADIUS: f32 = 1e-6;

/// Half-diagonal of the bounds, never zero (an empty box counts as a unit-sized model).
fn bounds_radius(bounds: &wte_core::Aabb) -> f32 {
    let r = bounds.diagonal() * 0.5;
    if r.is_finite() && r > MIN_RADIUS { r } else { 1.0 }
}

impl OrbitCamera {
    /// `target + distance * (cos p * sin y, sin p, cos p * cos y)`.
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + self.distance * Vec3::new(cp * sy, sp, cp * cy)
    }

    /// Right, up, forward (world).
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let forward = -Vec3::new(cp * sy, sp, cp * cy);
        // The yaw alone defines "right", so the frame stays well defined up to (and including) +-90 degrees of pitch.
        let right = Vec3::new(cy, 0.0, -sy);
        let up = right.cross(forward).normalize_or(Vec3::Y);
        (right, up, forward)
    }

    /// Near and far plane for a scene radius.
    fn near_far(&self, scene_radius: f32) -> (f32, f32) {
        let r = scene_radius.max(MIN_RADIUS);
        let near = (self.distance - 1.5 * r).max(1e-3 * r);
        let far = (self.distance + 1.5 * r).max(near * 1.001 + MIN_RADIUS);
        (near, far)
    }

    /// `near = max(distance - 1.5R, 1e-3 R)`, `far = distance + 1.5R` with R = `scene_radius` (see `scene_radius`).
    /// `ortho`: `directx::orthographic` with half-height `distance * tan(fov_y / 2)` and the same near/far.
    pub fn view_proj(&self, aspect: f32, scene_radius: f32) -> Mat4 {
        let aspect = if aspect.is_finite() && aspect > 1e-4 { aspect } else { 1.0 };
        let (_, up, _) = self.basis();
        let view = glam::camera::rh::view::look_at_mat4(self.eye(), self.target, up);
        let (near, far) = self.near_far(scene_radius);
        let fov = self.fov_y.clamp(1e-3, 3.0);
        let proj = if self.ortho {
            let half_h = (self.distance * (fov * 0.5).tan()).max(MIN_RADIUS);
            let half_w = half_h * aspect;
            glam::camera::rh::proj::directx::orthographic(-half_w, half_w, -half_h, half_h, near, far)
        } else {
            glam::camera::rh::proj::directx::perspective(fov, aspect, near, far)
        };
        proj * view
    }

    /// The ray through a screen position: unprojects near/far through `inverse(view_proj)`.
    pub fn ray(&self, pos: egui::Pos2, rect: egui::Rect, scene_radius: f32) -> wte_core::Ray {
        let inv = self.view_proj(rect.aspect_ratio(), scene_radius).inverse();
        let w = rect.width().max(1e-3);
        let h = rect.height().max(1e-3);
        let x = (pos.x - rect.min.x) / w * 2.0 - 1.0;
        let y = 1.0 - (pos.y - rect.min.y) / h * 2.0;
        let near = inv.project_point3(Vec3::new(x, y, 0.0));
        let far = inv.project_point3(Vec3::new(x, y, 1.0));
        if near.is_finite() && far.is_finite() && near != far {
            wte_core::Ray::new(near, far - near)
        } else {
            let (_, _, forward) = self.basis();
            wte_core::Ray::new(self.eye(), forward)
        }
    }

    /// Screen position of a world point; `None` behind the camera.
    pub fn project(&self, p: Vec3, rect: egui::Rect, scene_radius: f32) -> Option<egui::Pos2> {
        let (_, _, forward) = self.basis();
        if !self.ortho && (p - self.eye()).dot(forward) <= 0.0 {
            return None;
        }
        let clip = self.view_proj(rect.aspect_ratio(), scene_radius) * Vec4::new(p.x, p.y, p.z, 1.0);
        if clip.w.is_nan() || clip.w <= 1e-12 || !clip.is_finite() {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(egui::pos2(
            rect.min.x + (ndc.x + 1.0) * 0.5 * rect.width(),
            rect.min.y + (1.0 - ndc.y) * 0.5 * rect.height(),
        ))
    }

    /// Orbits by a pointer delta: 0.008 rad per point (the model follows the pointer).
    pub fn orbit(&mut self, delta_points: egui::Vec2) {
        if !delta_points.is_finite() {
            return;
        }
        self.yaw = (self.yaw - delta_points.x * ORBIT_RAD_PER_POINT).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + delta_points.y * ORBIT_RAD_PER_POINT).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Moves `target` in the view plane; afterwards clamps it so that
    /// `|target - bounds.center()| <= 2 * bounds.diagonal()`.
    pub fn pan(&mut self, delta_points: egui::Vec2, rect_height: f32, bounds: &wte_core::Aabb) {
        if !delta_points.is_finite() {
            return;
        }
        let (right, up, _) = self.basis();
        let world_per_point = 2.0 * self.distance * (self.fov_y * 0.5).tan() / rect_height.max(1.0);
        self.target += (-right * delta_points.x + up * delta_points.y) * world_per_point;
        let center = bounds.center();
        let limit = 2.0 * bounds.diagonal().max(MIN_RADIUS);
        let offset = self.target - center;
        if offset.length() > limit {
            self.target = center + offset.normalize_or_zero() * limit;
        }
    }

    /// One wheel notch = 15%: `distance *= exp(-0.15 * notches)`, clamped to `[0.02, 50] * model_radius`.
    pub fn zoom(&mut self, notches: f32, model_radius: f32) {
        if !notches.is_finite() {
            return;
        }
        let r = if model_radius.is_finite() && model_radius > MIN_RADIUS { model_radius } else { 1.0 };
        self.distance = (self.distance * (-0.15 * notches).exp()).clamp(MIN_DISTANCE_FACTOR * r, MAX_DISTANCE_FACTOR * r);
    }

    /// Frames the bounds: `target = centre`, `distance = R / sin(fov_y / 2) * 1.15`.
    pub fn frame(&mut self, bounds: wte_core::Aabb) {
        let r = bounds_radius(&bounds);
        self.target = bounds.center();
        self.distance = r / (self.fov_y.clamp(1e-3, 3.0) * 0.5).sin() * FRAME_MARGIN;
    }

    /// Looks along an axis (target and distance are kept): Front looks from +Z, Right from +X, Top from +Y.
    pub fn preset(&mut self, p: CameraPreset) {
        use std::f32::consts::{FRAC_PI_2, PI};
        let (yaw, pitch) = match p {
            CameraPreset::Front => (0.0, 0.0),
            CameraPreset::Back => (PI, 0.0),
            CameraPreset::Right => (FRAC_PI_2, 0.0),
            CameraPreset::Left => (3.0 * FRAC_PI_2, 0.0),
            CameraPreset::Top => (0.0, MAX_PITCH),
            CameraPreset::Bottom => (0.0, -MAX_PITCH),
        };
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// The R every caller passes to `view_proj` / `ray` / `project` (and stores in `ViewXform.scene_radius`):
    /// `bounds.diagonal() * 0.5 + (self.target - bounds.center()).length()` — the model stays inside near/far after
    /// panning.
    pub fn scene_radius(&self, bounds: &wte_core::Aabb) -> f32 {
        let r = bounds.diagonal() * 0.5 + (self.target - bounds.center()).length();
        if r.is_finite() && r > MIN_RADIUS { r } else { 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wte_core::Aabb;

    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 600.0))
    }

    fn unit_box() -> Aabb {
        Aabb { min: Vec3::new(-1.0, -0.5, -2.0), max: Vec3::new(1.0, 1.5, 0.5) }
    }

    fn corners(b: &Aabb) -> Vec<Vec3> {
        let mut out = Vec::new();
        for i in 0..8 {
            out.push(Vec3::new(
                if i & 1 == 0 { b.min.x } else { b.max.x },
                if i & 2 == 0 { b.min.y } else { b.max.y },
                if i & 4 == 0 { b.min.z } else { b.max.z },
            ));
        }
        out
    }

    /// View-space depth (distance along the view direction) of a point.
    fn depth(cam: &OrbitCamera, p: Vec3) -> f32 {
        (p - cam.eye()).dot(cam.basis().2)
    }

    #[test]
    fn eye_and_basis_follow_the_definition() {
        let cam = OrbitCamera { target: Vec3::new(1.0, 2.0, 3.0), yaw: 0.0, pitch: 0.0, distance: 5.0, ..OrbitCamera::default() };
        assert!((cam.eye() - Vec3::new(1.0, 2.0, 8.0)).length() < 1e-5);
        let (right, up, forward) = cam.basis();
        assert!((right - Vec3::X).length() < 1e-5);
        assert!((up - Vec3::Y).length() < 1e-5);
        assert!((forward - Vec3::NEG_Z).length() < 1e-5);
        let cam = OrbitCamera { yaw: 0.7, pitch: 0.4, ..cam };
        let (right, up, forward) = cam.basis();
        assert!(right.dot(up).abs() < 1e-5 && right.dot(forward).abs() < 1e-5 && up.dot(forward).abs() < 1e-5);
        assert!(((cam.target - cam.eye()).normalize() - forward).length() < 1e-5);
    }

    #[test]
    fn ray_and_project_round_trip() {
        for ortho in [false, true] {
            let mut cam = OrbitCamera { yaw: 0.9, pitch: -0.3, ortho, ..OrbitCamera::default() };
            cam.frame(unit_box());
            let r = cam.scene_radius(&unit_box());
            for &(x, y) in &[(0.5f32, 0.5f32), (0.1, 0.2), (0.9, 0.8), (0.33, 0.71)] {
                let pos = rect().min + egui::vec2(x * rect().width(), y * rect().height());
                let ray = cam.ray(pos, rect(), r);
                assert!((ray.dir.length() - 1.0).abs() < 1e-4);
                for t in [0.5f32, 2.0, 4.0] {
                    let back = cam.project(ray.at(t), rect(), r).expect("in front of the camera");
                    assert!((back - pos).length() < 0.05, "ortho {ortho}: {back:?} vs {pos:?} at t {t}");
                }
            }
            // A point behind a perspective camera does not project.
            if !ortho {
                let behind = cam.eye() - cam.basis().2 * 3.0;
                assert!(cam.project(behind, rect(), r).is_none());
            }
        }
    }

    #[test]
    fn frame_puts_the_bounds_in_view() {
        let b = unit_box();
        for (yaw, pitch) in [(0.0f32, 0.0f32), (0.6, 0.35), (2.5, -1.2), (4.0, 1.5)] {
            let mut cam = OrbitCamera { yaw, pitch, ..OrbitCamera::default() };
            cam.frame(b);
            assert_eq!(cam.target, b.center());
            let r = cam.scene_radius(&b);
            // A square view, so the vertical field of view limits both axes.
            let square = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 500.0));
            for c in corners(&b) {
                let p = cam.project(c, square, r).expect("corner in front of the camera");
                assert!(square.contains(p), "corner {c:?} at {p:?} outside the view (yaw {yaw}, pitch {pitch})");
                let (near, far) = cam.near_far(r);
                let d = depth(&cam, c);
                assert!(d > near && d < far);
            }
        }
    }

    #[test]
    fn model_stays_between_near_and_far_after_pan_and_orbit() {
        let b = unit_box();
        let mut cam = OrbitCamera::default();
        cam.frame(b);
        // Zoomed out far enough that the eye stays outside the panned bounding sphere (closer than that, parts of
        // the model are legitimately behind the eye).
        cam.zoom(-8.0, b.diagonal() * 0.5);
        // Pan far beyond the clamp limit.
        for _ in 0..50 {
            cam.pan(egui::vec2(-4000.0, 2500.0), 600.0, &b);
        }
        assert!((cam.target - b.center()).length() <= 2.0 * b.diagonal() + 1e-4);
        assert!((cam.target - b.center()).length() > 1.9 * b.diagonal());
        for delta in [egui::vec2(300.0, 0.0), egui::vec2(-120.0, 400.0), egui::vec2(77.0, -900.0)] {
            cam.orbit(delta);
            let r = cam.scene_radius(&b);
            let (near, far) = cam.near_far(r);
            let vp = cam.view_proj(1.5, r);
            for c in corners(&b) {
                let d = depth(&cam, c);
                assert!(d > near && d < far, "corner {c:?} depth {d} outside {near}..{far}");
                let clip = vp * c.extend(1.0);
                let z = clip.z / clip.w;
                assert!((0.0..=1.0).contains(&z));
            }
        }
    }

    #[test]
    fn orbit_clamps_pitch_and_zoom_clamps_distance() {
        let mut cam = OrbitCamera::default();
        cam.orbit(egui::vec2(0.0, 1e5));
        assert!((cam.pitch - MAX_PITCH).abs() < 1e-6);
        cam.orbit(egui::vec2(0.0, -1e6));
        assert!((cam.pitch + MAX_PITCH).abs() < 1e-6);
        let before = cam.distance;
        cam.zoom(1.0, 10.0);
        assert!((cam.distance - before * (-0.15f32).exp()).abs() < 1e-5);
        cam.zoom(1000.0, 10.0);
        assert!((cam.distance - 0.2).abs() < 1e-5);
        cam.zoom(-1000.0, 10.0);
        assert!((cam.distance - 500.0).abs() < 1e-3);
    }

    #[test]
    fn presets_look_along_the_axes() {
        let mut cam = OrbitCamera::default();
        cam.preset(CameraPreset::Front);
        assert!((cam.basis().2 - Vec3::NEG_Z).length() < 1e-5);
        cam.preset(CameraPreset::Right);
        assert!((cam.basis().2 - Vec3::NEG_X).length() < 1e-5);
        cam.preset(CameraPreset::Left);
        assert!((cam.basis().2 - Vec3::X).length() < 1e-5);
        cam.preset(CameraPreset::Back);
        assert!((cam.basis().2 - Vec3::Z).length() < 1e-5);
        cam.preset(CameraPreset::Top);
        assert!(cam.basis().2.y < -0.99);
        cam.preset(CameraPreset::Bottom);
        assert!(cam.basis().2.y > 0.99);
        // The matrix stays finite at the pitch limit.
        assert!(cam.view_proj(1.0, 2.0).is_finite());
    }
}
