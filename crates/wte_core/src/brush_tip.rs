// Written by the scaffold S1 of round 4, complete; no later owner (a defect is fixed as a listed cross-package edit).
// The tip of a 3D brush stroke (FEATURE_BRUSH 4.1, 5.3): the volume (ball or a field), which surfaces it reaches,
// how it is turned, offset and scaled, and the frame of one dab.
//
// Round-4 user answer (plan section 7 item 1): there is no "Stay on connected surface" setting, so `SurfaceReach`
// has no `Connected` value.

use std::sync::Arc;

use glam::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::tip_field::TipField;

/// Which surfaces inside the tip receive paint.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SurfaceReach {
    /// Same side as the stroke's reference normal: today's `same_side_only == true` (Circular).
    SameSide,
    /// Everything inside the tip, hidden or not: today's `same_side_only == false` (Sphere).
    All,
}

/// How a tip is turned as it moves. Ignored by a ball.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipAlign {
    /// "Follow surface": the tip's +Y follows the viewer-facing normal of each dab (`tip_frame`).
    #[default]
    Surface,
    /// "Fixed in space": the tip keeps its orientation in world space.
    World,
}

/// The volume of the brush.
#[derive(Clone, Debug)]
pub enum TipVolume {
    Ball,
    Field(Arc<TipField>),
}

/// Everything about a stroke's tip that is not in `BrushParams`.
#[derive(Clone, Debug)]
pub struct BrushTip {
    pub volume: TipVolume,
    pub reach: SurfaceReach,
    pub align: TipAlign,
    /// The user's rotation in the tip's own axes. Ignored by a ball.
    pub rotation: Quat,
    /// Shift of the tip centre along the viewer-facing normal, in radii, `-1..=1`.
    pub offset: f32,
    /// World length of one tip unit, in brush radii. 1.0 = the shape fits exactly inside the ring. `0.25..=4`.
    pub scale: f32,
    /// false = Solid (the volume), true = Through (the outline along tip Y, inside the ring's ball). Ignored by a
    /// ball; always treated as true for a flat outline.
    pub through: bool,
    /// World direction that counts as "up on screen" for this stroke (the camera's up at the press); +Y if unknown.
    pub up_hint: Vec3,
}

impl BrushTip {
    /// The tip `stroke_begin` uses: a ball, `SameSide` (`same_side_only`) or `All`, no offset, scale 1.
    pub fn legacy(same_side_only: bool) -> BrushTip {
        BrushTip {
            volume: TipVolume::Ball,
            reach: if same_side_only { SurfaceReach::SameSide } else { SurfaceReach::All },
            align: TipAlign::Surface,
            rotation: Quat::IDENTITY,
            offset: 0.0,
            scale: 1.0,
            through: false,
            up_hint: Vec3::Y,
        }
    }

    /// True for the ball (no field).
    pub fn is_ball(&self) -> bool {
        matches!(self.volume, TipVolume::Ball)
    }

    /// The field of a shape tip; `None` for the ball.
    pub fn field(&self) -> Option<&Arc<TipField>> {
        match &self.volume {
            TipVolume::Ball => None,
            TipVolume::Field(field) => Some(field),
        }
    }

    /// The tip with every value forced into its range (FEATURE_BRUSH 4.1 "Rules", test C22): `offset` to `-1..=1`
    /// (NaN becomes 0), `scale` to `0.25..=4` (NaN becomes 1), a rotation that is not finite or has no length becomes
    /// the identity and is normalized, an `up_hint` that is not finite or has no length becomes +Y (and is
    /// normalized), and `through` is true for a flat outline.
    pub fn sanitized(&self) -> BrushTip {
        let mut tip = self.clone();
        tip.offset = if tip.offset.is_nan() { 0.0 } else { tip.offset.clamp(-1.0, 1.0) };
        tip.scale = if tip.scale.is_nan() { 1.0 } else { tip.scale.clamp(0.25, 4.0) };
        tip.rotation = if tip.rotation.is_finite() && tip.rotation.length() > 1e-6 {
            tip.rotation.normalize()
        } else {
            Quat::IDENTITY
        };
        tip.up_hint = if tip.up_hint.is_finite() && tip.up_hint.length() > 1e-6 { tip.up_hint.normalize() } else { Vec3::Y };
        if tip.field().is_some_and(|f| f.is_flat()) {
            tip.through = true;
        }
        tip
    }
}

/// World directions of the tip's X, Y, Z axes (columns) for a dab whose viewer-facing normal is `facing`, before the
/// user's rotation, and the tangent to carry to the next dab (FEATURE_BRUSH 5.3). `prev_up` = the tangent the
/// previous dab of the segment returned.
///
/// `out` = `facing` normalized (+Y without a direction); `up_t` = the seed (`prev_up`, else `up_hint`) minus its part
/// along `out`, falling back to `up_hint` and then to any vector at right angles when that is shorter than 0.01;
/// `right = up_t x out`; the columns are `(right, out, -up_t)`: tip X to the right on screen, tip Y out of the
/// surface, tip Z down on screen. Always a right-handed orthonormal frame.
pub fn tip_frame(facing: Vec3, up_hint: Vec3, prev_up: Option<Vec3>) -> (Mat3, Vec3) {
    let out = if facing.is_finite() { facing.try_normalize().unwrap_or(Vec3::Y) } else { Vec3::Y };
    let tangent = |seed: Vec3| -> Option<Vec3> {
        let seed = if seed.is_finite() { seed.try_normalize()? } else { return None };
        let t = seed - out * seed.dot(out);
        (t.length() >= 0.01).then(|| t.normalize())
    };
    let up_t = prev_up
        .and_then(tangent)
        .or_else(|| tangent(up_hint))
        .unwrap_or_else(|| out.any_orthonormal_vector());
    let right = up_t.cross(out);
    (Mat3::from_cols(right, out, -up_t), up_t)
}

/// Where the last dab of the open stroke put the tip (for the cursor).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TipPose {
    /// The tip centre `c'` in world space.
    pub center: Vec3,
    /// World-from-tip rotation of the dab (`R = A * U`).
    pub world_from_tip: Mat3,
    /// World length of one tip unit (`k = radius * scale`).
    pub unit: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_frame(m: Mat3, normal: Vec3) {
        let (x, y, z) = (m.x_axis, m.y_axis, m.z_axis);
        for a in [x, y, z] {
            assert!((a.length() - 1.0).abs() < 1e-4, "{m:?}");
        }
        assert!(x.dot(y).abs() < 1e-4 && y.dot(z).abs() < 1e-4 && x.dot(z).abs() < 1e-4, "{m:?}");
        assert!((x.cross(y) - z).length() < 1e-4, "right-handed: {m:?}");
        assert!((y - normal.normalize()).length() < 1e-4, "Y column is the normal: {m:?}");
    }

    #[test]
    fn frames_are_right_handed_with_the_normal_as_y() {
        // A deterministic pseudo-random sequence.
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        for _ in 0..1000 {
            let n = Vec3::new(next(), next(), next());
            let hint = Vec3::new(next(), next(), next());
            if n.length() < 1e-3 {
                continue;
            }
            let (m, up) = tip_frame(n, hint, None);
            assert_frame(m, n);
            assert!((m.z_axis + up).length() < 1e-5, "the carried tangent is -Z");
            // Carrying the tangent to a slightly different normal keeps the frame close.
            let (m2, _) = tip_frame(n + Vec3::new(0.01, 0.0, 0.0), hint, Some(up));
            assert!(m2.x_axis.dot(m.x_axis) > 0.9);
        }
        // Normal +Z with hint +Y: columns (+X, +Z, -Y).
        let (m, up) = tip_frame(Vec3::Z, Vec3::Y, None);
        assert!((m.x_axis - Vec3::X).length() < 1e-6 && (m.y_axis - Vec3::Z).length() < 1e-6);
        assert!((m.z_axis - Vec3::NEG_Y).length() < 1e-6 && (up - Vec3::Y).length() < 1e-6);
        // A hint parallel to the normal, a zero hint and no normal at all still give orthonormal frames.
        assert_frame(tip_frame(Vec3::Y, Vec3::Y, None).0, Vec3::Y);
        assert_frame(tip_frame(Vec3::X, Vec3::ZERO, Some(Vec3::X)).0, Vec3::X);
        assert_frame(tip_frame(Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 0.0), None).0, Vec3::Y);
        assert_frame(tip_frame(Vec3::new(f32::INFINITY, 0.0, 0.0), Vec3::Y, None).0, Vec3::Y);
    }

    #[test]
    fn legacy_and_clamping() {
        let tip = BrushTip::legacy(true);
        assert!(tip.is_ball() && tip.reach == SurfaceReach::SameSide && tip.offset == 0.0 && tip.scale == 1.0);
        assert_eq!(BrushTip::legacy(false).reach, SurfaceReach::All);

        let wild = BrushTip {
            volume: TipVolume::Ball,
            reach: SurfaceReach::All,
            align: TipAlign::World,
            rotation: Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0),
            offset: 5.0,
            scale: f32::NAN,
            through: false,
            up_hint: Vec3::ZERO,
        };
        let s = wild.sanitized();
        assert_eq!((s.rotation, s.offset, s.scale, s.up_hint), (Quat::IDENTITY, 1.0, 1.0, Vec3::Y));
        let s = BrushTip { offset: f32::NAN, scale: 0.01, rotation: Quat::from_xyzw(0.0, 0.0, 0.0, 2.0), ..wild }.sanitized();
        assert_eq!((s.offset, s.scale, s.rotation), (0.0, 0.25, Quat::IDENTITY));
        // A flat outline is always used Through.
        let flat = Arc::new(TipField::outline_from_fn([33, 33], glam::Vec2::splat(-1.0), 1.0 / 16.0, |p| p.length() - 0.5));
        let s = BrushTip { volume: TipVolume::Field(flat), through: false, ..BrushTip::legacy(false) }.sanitized();
        assert!(s.through);
        assert_eq!(serde_json::to_value(TipAlign::World).unwrap(), "world");
    }
}
