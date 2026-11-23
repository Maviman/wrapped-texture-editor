// Owner: core-stamps-text (C4). Projector / UvPlacement geometry and the two decal bakes (ARCHITECTURE.md 5.12,
// algorithm 6.5).
//
// Choices the contract leaves open, all documented at the items below:
// - `Projector::corners` / `UvPlacement::corners_px` describe the placement rectangle; the flips mirror the image
//   inside it and do not reorder the corners.
// - The mip level is chosen from the more strongly minified image axis, so a non-uniformly squashed decal is blurred
//   rather than aliased.
// - The filter border of the source (5.5: transparent half a level texel outside the image) is baked too: both bakes
//   look one source texel, or one target texel when that is larger, beyond the image rectangle.
// - A projected bake scans only the part of each triangle that lies inside the projection box (the triangle is
//   clipped against the box in projector space), not the whole triangle.
// - Occlusion rays of gutter texels start at the nearest point inside the triangle, so a seam's padding has the
//   visibility of the texels it pads.

use std::sync::Arc;

use glam::{Mat3, Quat, Vec2, Vec3};
use rayon::prelude::*;

use crate::bvh::{Bvh, Cull};
use crate::ids::{TextureSetId, TriId};
use crate::math::{IRect, Ray, smoothstep};
use crate::mesh::MeshData;
use crate::pixel::SparseBuf;
use crate::pixel_ops::PremulMips;
use crate::stamp::{BakeQuality, Projector, SurfaceMode, UvPlacement, WrapOptions};
use crate::texel_map::{Fragment, TexelMap, raster_tri};

/// Default of `Projector::angle_start_deg`. Full strength up to 70 degrees: the decal is stretched by 1 / cos(angle)
/// along the slope, i.e. at most 2.9x there, which still reads as the image; so a stamp wraps at full strength over
/// most of a moderately curved surface (on a sphere, over the inner 94 % of the projected disc radius).
pub const DEFAULT_ANGLE_START_DEG: f32 = 70.0;
/// Default of `Projector::angle_end_deg`. The weight falls smoothly to zero at 88 degrees (stretch 29x), so the decal
/// runs out softly right at the silhouette instead of stopping with a visible edge at 85 degrees; surfaces facing
/// away are still removed by back-face culling.
pub const DEFAULT_ANGLE_END_DEG: f32 = 88.0;
/// Part of each depth limit over which the weight fades to zero (smoothstep), so the end of the projection box never
/// leaves a hard edge.
pub const DEPTH_FADE_FRACTION: f32 = 0.2;
/// Depth auto-fit (`fit_projector_depth`): rays per side of the sample grid over the footprint.
const FIT_GRID: usize = 17;
/// Depth auto-fit: hits per ray that are considered (the surface of the placement point may lie behind others).
const FIT_HITS_PER_RAY: usize = 4;
/// Depth auto-fit: the sampled range along the projection axis, in units of the larger half size of the decal.
const FIT_REACH: f32 = 2.0;
/// Depth auto-fit: the steepest slope (depth per sideways distance) the surface test uses, 8 = 83 degrees; steeper
/// parts are almost tangent to the projection and receive little anyway.
const FIT_MAX_SLOPE: f32 = 8.0;
/// Depth auto-fit: two neighbouring samples belong to one surface when their depth step is at most
/// `grid step * (FIT_SLOPE_TOLERANCE * steeper slope + FIT_STEP_TOLERANCE)`: a smooth surface steps by about its
/// slope, a step to a separate surface behind (an object edge) is much larger than the slopes on either side.
const FIT_SLOPE_TOLERANCE: f32 = 1.5;
const FIT_STEP_TOLERANCE: f32 = 0.35;
/// Depth auto-fit: margin and minimum of each fitted depth, in units of the larger half size (covers detail between
/// the grid samples and keeps a flat placement from getting a zero-depth box).
const FIT_MARGIN: f32 = 0.1;
/// Rows per parallel band (section 6.3).
pub(crate) const BAND_ROWS: usize = 64;

/// Test-build counters of `bake_projected` for the timing probes (no-ops outside tests): 0 candidates ns, 1 raster +
/// fragments ns, 2 crop ns, 3 scratch texels, 4 fragments visited, 5 fragments shaded, 6 occlusion rays, 7 jobs, 8 occluded rays, 9 occlusions proven without the BVH, 10 clear rays proven without it, 11 occluder grid build ns (inside slot 0).
pub(crate) mod probe {
    #[cfg(test)]
    pub(crate) static SLOTS: [std::sync::atomic::AtomicU64; 12] = [const { std::sync::atomic::AtomicU64::new(0) }; 12];

    /// Per-fragment counts (slots 4, 5, 6, 8, 9, 10) only while this is set: shared atomics in the hot loop would
    /// serialize the threads and distort the timings.
    /// Timing probes: bake without the occluder grid.
    #[cfg(test)]
    pub(crate) static NO_GRID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[cfg(test)]
    thread_local! {
        /// Tests, on the thread that calls `bake_projected`: `Some(fine)` builds the occluder grid whatever the size
        /// of the bake, with (`true`) or without (`false`) the cover and floor grids. `None`: the release rule.
        pub(crate) static FORCE_FINE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
        /// Tests: `Some(bytes)` replaces `SCRATCH_BUDGET` for bakes on this thread.
        pub(crate) static SCRATCH_BUDGET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
        /// Tests: the largest scratch bytes held at once by a bake on this thread (`raster_jobs`), kept as a maximum.
        pub(crate) static PEAK_SCRATCH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Sets `FORCE_FINE` on this thread until dropped.
    #[cfg(test)]
    pub(crate) struct ForceFine(Option<bool>);
    #[cfg(test)]
    impl ForceFine {
        pub(crate) fn new(fine: bool) -> ForceFine {
            ForceFine(FORCE_FINE.with(|f| f.replace(Some(fine))))
        }
    }
    #[cfg(test)]
    impl Drop for ForceFine {
        fn drop(&mut self) {
            FORCE_FINE.with(|f| f.set(self.0));
        }
    }
    #[cfg(test)]
    pub(crate) static COUNT_FRAGMENTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    #[cfg(test)]
    #[inline]
    pub(crate) fn count(slot: usize, n: u64) {
        if matches!(slot, 4 | 5 | 6 | 8 | 9 | 10) && !COUNT_FRAGMENTS.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        SLOTS[slot].fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(not(test))]
    #[inline(always)]
    pub(crate) fn count(_slot: usize, _n: u64) {}

    #[cfg(test)]
    pub(crate) fn now() -> Option<std::time::Instant> {
        Some(std::time::Instant::now())
    }
    #[cfg(not(test))]
    #[inline(always)]
    pub(crate) fn now() -> Option<std::time::Instant> {
        None
    }

    #[inline]
    pub(crate) fn since(slot: usize, t: Option<std::time::Instant>) {
        if let Some(t) = t {
            count(slot, t.elapsed().as_nanos() as u64);
        }
    }

    /// Takes and resets every slot.
    #[cfg(test)]
    pub(crate) fn take() -> [u64; 12] {
        std::array::from_fn(|i| SLOTS[i].swap(0, std::sync::atomic::Ordering::Relaxed))
    }
}
/// Upper bound of the alpha of a `PremulMips` sample (1 plus room for the rounding of the filter weights).
pub(crate) const SAMPLE_ALPHA_MAX: f32 = 1.0 + 1e-5;
/// Sizes below this (world units or texels) bake nothing.
pub(crate) const MIN_SIZE: f32 = 1e-9;

/// `v` normalized, or `fallback` when it has no direction.
fn unit_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let n = v.normalize_or_zero();
    if n == Vec3::ZERO || !n.is_finite() { fallback } else { n }
}

impl Projector {
    /// Frame from a surface point: Z = normal, Y = up_hint projected onto the tangent plane (any orthonormal vector
    /// if parallel), X = Y x Z. depth_front = depth_back = depth. Other fields take their defaults.
    ///
    /// Defaults: angles 70 / 88 degrees (`DEFAULT_ANGLE_START_DEG` / `DEFAULT_ANGLE_END_DEG`), back-face culling and occlusion on, no flips. A normal without direction
    /// counts as +Z.
    pub fn from_surface(position: Vec3, normal: Vec3, up_hint: Vec3, half_size: Vec2, depth: f32) -> Projector {
        let z = unit_or(normal, Vec3::Z);
        let hint = unit_or(up_hint, Vec3::ZERO);
        let tangent = hint - z * hint.dot(z);
        let y = if tangent.length_squared() > 1e-8 { unit_or(tangent, Vec3::ZERO) } else { Vec3::ZERO };
        let y = if y == Vec3::ZERO { unit_or(z.any_orthonormal_vector(), Vec3::Y) } else { y };
        let x = unit_or(y.cross(z), Vec3::X);
        Projector {
            origin: position,
            rotation: Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize(),
            half_size,
            depth_front: depth,
            depth_back: depth,
            angle_start_deg: DEFAULT_ANGLE_START_DEG,
            angle_end_deg: DEFAULT_ANGLE_END_DEG,
            backface_cull: true,
            occlusion: true,
            flip_x: false,
            flip_y: false,
            mode: SurfaceMode::Project,
            wrap: WrapOptions::default(),
        }
    }

    /// `rotation` as a unit quaternion (identity when it is unusable).
    fn unit_rotation(&self) -> Quat {
        let len2 = self.rotation.length_squared();
        if len2.is_finite() && len2 > 1e-12 { self.rotation * len2.sqrt().recip() } else { Quat::IDENTITY }
    }

    /// World right, up, out.
    pub fn axes(&self) -> (Vec3, Vec3, Vec3) {
        let q = self.unit_rotation();
        (q * Vec3::X, q * Vec3::Y, q * Vec3::Z)
    }

    /// TL, TR, BR, BL of the image, on the plane z = 0.
    ///
    /// These are the corners of the placement rectangle in the projector frame (TL = `origin - right * hw + up * hh`);
    /// `flip_x` / `flip_y` mirror the image inside this rectangle and do not change the corner order.
    pub fn corners(&self) -> [Vec3; 4] {
        let (right, up, _) = self.axes();
        let (r, u) = (right * self.half_size.x, up * self.half_size.y);
        [self.origin - r + u, self.origin + r + u, self.origin + r - u, self.origin - r - u]
    }

    /// Same decal moved to another surface point: the frame is rotated by the shortest arc from the old to the new normal.
    ///
    /// A normal without direction moves the decal without turning it.
    pub fn moved_to(&self, position: Vec3, normal: Vec3) -> Projector {
        let mut moved = *self;
        moved.origin = position;
        let n = unit_or(normal, Vec3::ZERO);
        if n != Vec3::ZERO {
            let rotation = self.unit_rotation();
            let out = unit_or(rotation * Vec3::Z, Vec3::Z);
            moved.rotation = (Quat::from_rotation_arc(out, n) * rotation).normalize();
        }
        moved
    }

    /// Rotation about local +Z. Positive = clockwise as seen from +Z (by the viewer), the same sense as UvPlacement.rotation:
    /// `rotation = rotation * Quat::from_rotation_z(-radians)`. A +90 degree turn moves the image's top edge to the right.
    pub fn rotated(&self, radians: f32) -> Projector {
        let mut turned = *self;
        turned.rotation = (self.rotation * Quat::from_rotation_z(-radians)).normalize();
        turned
    }

    /// Radius of a sphere around `origin` that contains the whole projection box.
    pub fn bounding_radius(&self) -> f32 {
        let depth = self.depth_front.abs().max(self.depth_back.abs());
        Vec3::new(self.half_size.x, self.half_size.y, depth).length()
    }

    /// True when every float of the projector is finite.
    pub fn is_finite(&self) -> bool {
        self.origin.is_finite()
            && self.rotation.is_finite()
            && self.half_size.is_finite()
            && self.depth_front.is_finite()
            && self.depth_back.is_finite()
            && self.angle_start_deg.is_finite()
            && self.angle_end_deg.is_finite()
    }
}

impl UvPlacement {
    /// The image's right and down directions in texel space (v-down): a positive rotation turns clockwise on screen.
    fn frame(&self) -> (Vec2, Vec2) {
        let (sin, cos) = self.rotation.sin_cos();
        (Vec2::new(cos, sin), Vec2::new(-sin, cos))
    }

    /// TL, TR, BR, BL of the image in texels of the target set.
    ///
    /// Corners of the placement rectangle (`center_px -/+ right * size.x / 2 -/+ down * size.y / 2`, rotated); the flips
    /// mirror the image inside it and do not change the corner order.
    pub fn corners_px(&self) -> [Vec2; 4] {
        let (right, down) = self.frame();
        let (r, d) = (right * (self.size_px.x * 0.5), down * (self.size_px.y * 0.5));
        let c = self.center_px;
        [c - r - d, c + r - d, c + r + d, c - r + d]
    }

    /// True when every float of the placement is finite.
    pub fn is_finite(&self) -> bool {
        self.center_px.is_finite() && self.size_px.is_finite() && self.rotation.is_finite()
    }
}

/// Premultiplied sample (0..=1) -> straight RGBA8: un-premultiply, multiply by `tint` (0..=1 per channel) and the
/// alpha additionally by `weight`. `None` when the resulting alpha rounds to zero.
#[inline]
pub(crate) fn shade(sample: [f32; 4], tint: [f32; 4], weight: f32) -> Option<[u8; 4]> {
    let alpha = sample[3];
    if alpha.is_nan() || alpha <= 0.0 {
        return None;
    }
    let out_alpha = ((alpha * tint[3] * weight).min(1.0) * 255.0 + 0.5) as u8;
    if out_alpha == 0 {
        return None;
    }
    let inv = alpha.recip();
    let channel = |c: f32, t: f32| ((c * inv).min(1.0) * t * 255.0 + 0.5) as u8;
    Some([channel(sample[0], tint[0]), channel(sample[1], tint[1]), channel(sample[2], tint[2]), out_alpha])
}

pub(crate) fn tint_factors(tint: [u8; 4]) -> [f32; 4] {
    tint.map(|c| c as f32 * (1.0 / 255.0))
}

/// Turns a dense scratch buffer covering `rect` of the canvas into the cache: the touched 64 x 64 tiles grouped
/// into parts, each cropped to its pixels (a texel the bake did not write stays all zero; written texels have
/// alpha > 0, so non-zero == alpha > 0 here).
pub(crate) fn crop_scratch(rect: IRect, data: Vec<u8>) -> SparseBuf {
    SparseBuf::from_dense(rect, data)
}

/// One triangle of a projected bake.
struct TriJob {
    tri: TriId,
    /// Unwrapped pixel-space box of the part of the triangle inside the projection box (plus the gutter).
    clip: [f32; 4],
    lod: f32,
    /// Half extents of the box including the filter border, for this triangle's texel size.
    limit_x: f32,
    limit_y: f32,
    /// Unit geometric normal.
    geo: Vec3,
    /// Start offset of occlusion rays: the geometric normal turned toward the projector, times the bias.
    bias: Vec3,
}

/// Up to 12 vertices of a convex polygon: (position in projector space, barycentric weights in the triangle).
#[derive(Copy, Clone)]
struct Poly {
    len: usize,
    v: [(Vec3, Vec3); 12],
}

impl Poly {
    /// The part of the polygon where `sign * position[axis] <= limit`.
    fn clipped(&self, axis: usize, sign: f32, limit: f32) -> Poly {
        let mut out = Poly { len: 0, v: self.v };
        let inside_by = |p: &(Vec3, Vec3)| limit - sign * p.0[axis];
        for i in 0..self.len {
            let (a, b) = (self.v[i], self.v[(i + 1) % self.len]);
            let (da, db) = (inside_by(&a), inside_by(&b));
            if da >= 0.0 && out.len < out.v.len() {
                out.v[out.len] = a;
                out.len += 1;
            }
            if (da >= 0.0) != (db >= 0.0) && out.len < out.v.len() {
                let t = da / (da - db);
                out.v[out.len] = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                out.len += 1;
            }
        }
        out
    }
}

/// Unwrapped pixel-space bounding box of the part of a triangle that lies inside the projection box
/// `|x| <= limit_x, |y| <= limit_y, -back <= z <= front`, grown by `grow` texels. `None` when the triangle misses the box.
fn footprint(local: &[Vec3; 3], uv_px: &[Vec2; 3], limit_x: f32, limit_y: f32, front: f32, back: f32, grow: f32) -> Option<[f32; 4]> {
    let mut poly = Poly { len: 3, v: [(Vec3::ZERO, Vec3::ZERO); 12] };
    poly.v[0] = (local[0], Vec3::X);
    poly.v[1] = (local[1], Vec3::Y);
    poly.v[2] = (local[2], Vec3::Z);
    for (axis, sign, limit) in
        [(0, 1.0, limit_x), (0, -1.0, limit_x), (1, 1.0, limit_y), (1, -1.0, limit_y), (2, 1.0, front), (2, -1.0, back)]
    {
        poly = poly.clipped(axis, sign, limit);
        if poly.len == 0 {
            return None;
        }
    }
    let (mut lo, mut hi) = (Vec2::INFINITY, Vec2::NEG_INFINITY);
    for (_, bary) in &poly.v[..poly.len] {
        let p = uv_px[0] * bary.x + uv_px[1] * bary.y + uv_px[2] * bary.z;
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return None;
    }
    Some([lo.x - grow, lo.y - grow, hi.x + grow, hi.y + grow])
}

/// Everything a fragment of a projected bake needs.
struct ProjectCtx<'a> {
    mesh: &'a MeshData,
    bvh: &'a Bvh,
    src: &'a PremulMips,
    quality: BakeQuality,
    origin: Vec3,
    right: Vec3,
    up: Vec3,
    out: Vec3,
    inv_width: f32,
    inv_height: f32,
    front: f32,
    back: f32,
    cos_start: f32,
    cos_end: f32,
    backface_cull: bool,
    occlusion: bool,
    flip_x: bool,
    flip_y: bool,
    tint: [f32; 4],
    /// The occluder grid of a `Full` bake with occlusion (`Occluders`); `None` leaves every ray to the BVH.
    occluders: Option<Occluders>,
}

impl ProjectCtx<'_> {
    fn local(&self, world: Vec3) -> Vec3 {
        let d = world - self.origin;
        Vec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.out))
    }

    /// Soft fade (smoothstep) over the last `DEPTH_FADE_FRACTION` of each depth limit.
    fn depth_fade(&self, z: f32) -> f32 {
        let front = if self.front > 0.0 { smoothstep(0.0, DEPTH_FADE_FRACTION * self.front, self.front - z) } else { 1.0 };
        let back = if self.back > 0.0 { smoothstep(0.0, DEPTH_FADE_FRACTION * self.back, z + self.back) } else { 1.0 };
        front * back
    }

    /// The decal pixel for one fragment of `job` (section 6.5 step 3) when its alpha exceeds `stored` (the alpha the
    /// texel already holds), `None` when the fragment receives nothing or could not win the max-alpha rule. The
    /// occlusion ray, the expensive part, is only cast for a pixel that would be written: the result is the same as
    /// shading every fragment and comparing afterwards.
    fn fragment(
        &self,
        job: &TriJob,
        pos: &[Vec3; 3],
        normals: &[Vec3; 3],
        f: &Fragment,
        stored: u8,
        hint: Option<&mut Option<TriId>>,
    ) -> Option<[u8; 4]> {
        let b = f.bary;
        let world = pos[0] * b[0] + pos[1] * b[1] + pos[2] * b[2];
        let l = self.local(world);
        // Written so that NaN is rejected.
        if !(l.x.abs() <= job.limit_x && l.y.abs() <= job.limit_y && l.z <= self.front && l.z >= -self.back) {
            return None;
        }
        // Hidden under the cover surface of its cell (`Occluders::below_floor`): the occlusion test at the end would
        // reject the fragment, so it is rejected now. The ray of a texel that is not a gutter texel starts at `world`
        // and its `t_max` is `front - l.z`, as computed below.
        if !f.gutter
            && self.front - l.z > 0.0
            && self.occlusion
            && self.quality == BakeQuality::Full
            && self.occluders.as_ref().is_some_and(|g| g.below_floor(l))
        {
            #[cfg(test)]
            probe::count(9, 1);
            return None;
        }

        // Gutter barycentrics are extrapolated: the blend can degenerate.
        let n = (normals[0] * b[0] + normals[1] * b[1] + normals[2] * b[2]).normalize_or_zero();
        let n = if n == Vec3::ZERO || !n.is_finite() { job.geo } else { n };
        let mut facing = n.dot(self.out);
        if !self.backface_cull {
            facing = facing.abs();
        }
        let weight = smoothstep(self.cos_end, self.cos_start, facing) * self.depth_fade(l.z);
        if weight.is_nan() || weight <= 0.0 {
            return None;
        }
        // The alpha `shade` can reach for any sample (a filtered premultiplied alpha is at most 1 up to rounding): a
        // fragment that cannot beat the stored alpha needs no sample. `shade` rounds monotonically, so this is exact.
        if stored > 0 && (((SAMPLE_ALPHA_MAX * self.tint[3] * weight).min(1.0) * 255.0 + 0.5) as u8) <= stored {
            return None;
        }
        // The occlusion ray of the fragment (section 6.5 step 3), tested at the end. A fragment under a surface that
        // covers its cell of the occluder grid is rejected here already, in constant time and before the sample:
        // `Occluders::covered` only says yes where the BVH finds a hit too, so the result is the same.
        let occlusion_ray = (self.occlusion && self.quality == BakeQuality::Full).then(|| {
            let start = if f.gutter {
                // Nearest point inside the triangle: clamp the extrapolated weights.
                let c = [b[0].max(0.0), b[1].max(0.0), b[2].max(0.0)];
                let sum = c[0] + c[1] + c[2];
                if sum > 0.0 { (pos[0] * c[0] + pos[1] * c[1] + pos[2] * c[2]) / sum } else { world }
            } else {
                world
            };
            (Ray { origin: start + job.bias, dir: self.out }, self.front - (start - self.origin).dot(self.out))
        });
        if let Some((ray, t_max)) = &occlusion_ray
            && *t_max > 0.0
            && self.occluders.as_ref().is_some_and(|g| g.covered(ray, *t_max))
        {
            #[cfg(test)]
            probe::count(9, 1);
            return None;
        }

        let mut u = 0.5 + l.x * self.inv_width;
        let mut v = 0.5 - l.y * self.inv_height;
        if self.flip_x {
            u = 1.0 - u;
        }
        if self.flip_y {
            v = 1.0 - v;
        }
        let sample = match self.quality {
            BakeQuality::Full => self.src.sample(u, v, job.lod),
            BakeQuality::Preview => self.src.sample_nearest_level(u, v, job.lod),
        };
        let pixel = shade(sample, self.tint, weight)?;
        #[cfg(test)]
        probe::count(5, 1);
        // Max-alpha rule (step 3): a pixel that does not beat the stored alpha is never written.
        if pixel[3] <= stored {
            return None;
        }
        // Occlusion last: even with the shortcuts of `occluded` a ray costs more than a sample, and transparent or
        // losing fragments need none.
        if let Some((ray, t_max)) = occlusion_ray {
            #[cfg(test)]
            probe::count(6, 1);
            if t_max > 0.0 && self.occluded(&ray, t_max, hint) {
                #[cfg(test)]
                probe::count(8, 1);
                return None;
            }
        }
        Some(pixel)
    }

    /// `bvh.occluded(ray, 0, t_max)`, the occlusion test of section 6.5. With a `hint` (the occluder met last in the
    /// same band) the answer is first looked for without the BVH: that triangle, then the triangles of the ray's cell
    /// in `occluders`. Those shortcuts only answer what `classify` proves the BVH query would answer too, so the
    /// result never changes; anything unproven goes to the BVH. Without a hint: the BVH alone.
    fn occluded(&self, ray: &Ray, t_max: f32, hint: Option<&mut Option<TriId>>) -> bool {
        let Some(hint) = hint else { return self.bvh.occluded(self.mesh, ray, 0.0, t_max) };
        if let Some(grid) = &self.occluders {
            match grid.occluder(ray, t_max, *hint) {
                Proof::Hit(tri) => {
                    *hint = Some(tri);
                    #[cfg(test)]
                    probe::count(9, 1);
                    return true;
                }
                Proof::Clear => {
                    #[cfg(test)]
                    probe::count(10, 1);
                    return false;
                }
                Proof::Unknown => {}
            }
        } else if let Some(tri) = *hint
            && classify(self.mesh.tri_positions(tri), ray, t_max) == Crossing::Hit
        {
            #[cfg(test)]
            probe::count(9, 1);
            return true;
        }
        match self.bvh.occluder(self.mesh, ray, 0.0, t_max) {
            Some(tri) => {
                *hint = Some(tri);
                true
            }
            None => false,
        }
    }
}

/// `classify`: largest condition estimate decided.
const MAX_COND: f64 = 1e4;
/// `classify`: error factor, about 17 f32 epsilons.
const ERR: f64 = 2e-6;
/// `BARY_EPS` of `bvh.rs`, the barycentric tolerance of its triangle test.
const TRI_EPS: f64 = 1e-6;

/// The verdict of `classify` from the barycentric weights `w` and the distance `t` of the crossing, with `cond` and
/// `span` as defined there; `slack` widens the margins of `w` and `t` by the f64 rounding of their computation.
#[inline]
fn verdict(w: [f64; 3], t: f64, t_max: f64, cond: f64, span: f64, slack: [f64; 2]) -> Crossing {
    if !(cond.is_finite() && cond <= MAX_COND && t.is_finite() && w.iter().all(|x| x.is_finite())) {
        return Crossing::Unknown;
    }
    let err_t = ERR * cond * (span + t.abs()) + slack[1] + 1e-12;
    if t < -err_t || t > t_max + err_t {
        return Crossing::Miss;
    }
    let largest = w[0].abs().max(w[1].abs()).max(w[2].abs());
    let err_w = 2.0 * ERR * cond * (1.0 + largest) + slack[0] + 1e-9;
    if w[0] < -TRI_EPS - err_w || w[1] < -TRI_EPS - err_w || w[2] < -TRI_EPS - err_w {
        return Crossing::Miss;
    }
    if w[0] > err_w && w[1] > err_w && w[2] > err_w && t > err_t && t < t_max - err_t {
        return Crossing::Hit;
    }
    Crossing::Unknown
}

/// What `classify` can prove about a ray and one triangle.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Crossing {
    /// `Bvh::occluded(ray, 0, t_max)` certainly finds a hit on this triangle.
    Hit,
    /// The BVH's triangle test certainly reports no hit on this triangle within `[0, t_max]`.
    Miss,
    /// Too close to call, or too ill-conditioned.
    Unknown,
}

/// Decides, in f64, what the f32 triangle test of `Bvh::occluded` (`ray_triangle`: Moller-Trumbore with a
/// barycentric tolerance of 1e-6, a hit accepted when `0 <= t <= t_max`) reports for this triangle, wherever the
/// rounding of that test cannot change the answer.
///
/// `ray_triangle` works on differences of the f32 inputs (exact up to one rounding each), so its error in the
/// barycentric weights is a few f32 epsilons times the condition `K = span * longest edge / |det|` (`span` =
/// distance origin - first vertex plus both edge lengths, `|det| = |n . dir|`), and in `t` that times `span`. The
/// margins below are about 17 epsilons times `K` (times `1 + |weight|`, `span + |t|`), which bounds those errors with
/// room to spare; above `K = 1e4` nothing is decided. A `Hit` lies well inside the triangle and well inside the
/// range, so the BVH (padded boxes, conservative slab tests, any-hit) finds a hit too. A `Miss` lies well outside
/// the triangle or the range.
fn classify([a, b, c]: [Vec3; 3], ray: &Ray, t_max: f32) -> Crossing {
    let (a, b, c) = (a.as_dvec3(), b.as_dvec3(), c.as_dvec3());
    let (o, d) = (ray.origin.as_dvec3(), ray.dir.as_dvec3());
    let (e1, e2) = (b - a, c - a);
    let n = e1.cross(e2);
    let nd = n.dot(d);
    let span = (o - a).length() + e1.length() + e2.length();
    let cond = span * e1.length().max(e2.length()) / nd.abs();
    if (d.length() - 1.0).abs() >= 1e-3 {
        return Crossing::Unknown;
    }
    let t = n.dot(a - o) / nd;
    let p = o + d * t;
    let nn = n.length_squared();
    let w = [(b - p).cross(c - p).dot(n) / nn, (c - p).cross(a - p).dot(n) / nn, (a - p).cross(b - p).dot(n) / nn];
    let size = a.abs().max(b.abs()).max(c.abs()).max(o.abs()).max_element();
    verdict(w, t, t_max as f64, cond, span, f64_slack(size, nn.sqrt(), nd.abs()))
}

/// Margins for the f64 rounding of `classify`'s weights and distance (cancellation of coordinates of size `size`
/// against a triangle of doubled area `n_len` seen at `|n . d| = nd`); negligible unless the triangle is tiny far
/// from the origin.
#[inline]
fn f64_slack(size: f64, n_len: f64, nd: f64) -> [f64; 2] {
    let s = 4.0 * size;
    [1e-13 * s * s / n_len, 1e-13 * s * n_len / nd + 1e-13 * s]
}

/// `classify` for one triangle and every ray along one direction `d`, prepared: the crossing distance and the
/// barycentric weights are affine functions of the ray origin (the values `classify` computes, up to f64 rounding,
/// which the margins dwarf). The distance from the origin to the first vertex enters `span` through an upper bound,
/// the 1-norm, which only widens the margins.
struct Prepped {
    tri: TriId,
    a: glam::DVec3,
    /// |e1| + |e2|.
    edges: f64,
    /// The longer of e1, e2 over |n . d|: `cond = span * cond_per_span`.
    cond_per_span: f64,
    /// t = t0 + tv . o.
    t0: f64,
    tv: glam::DVec3,
    /// w_i = g_i . o + h_i.
    g: [glam::DVec3; 3],
    h: [f64; 3],
    /// Largest coordinate of the vertices, |n|, |n . d| (for `f64_slack`).
    size: f64,
    n_len: f64,
    nd: f64,
}

impl Prepped {
    fn new(tri: TriId, [a, b, c]: [Vec3; 3], d: glam::DVec3) -> Prepped {
        let (a, b, c) = (a.as_dvec3(), b.as_dvec3(), c.as_dvec3());
        let (e1, e2) = (b - a, c - a);
        let n = e1.cross(e2);
        let (nd, nn) = (n.dot(d), n.length_squared());
        let (t0, tv) = (n.dot(a) / nd, -n / nd);
        // The weight of the vertex opposite the edge (q, r): ((q - p) x (r - p)) . n / nn
        // = ((q x r) . n + p . (n x (r - q))) / nn, with p = o + d * (t0 + tv . o).
        let weight = |q: glam::DVec3, r: glam::DVec3| {
            let m = n.cross(r - q) / nn;
            let k = q.cross(r).dot(n) / nn;
            let md = m.dot(d);
            (m + tv * md, k + md * t0)
        };
        let (w0, w1, w2) = (weight(b, c), weight(c, a), weight(a, b));
        Prepped {
            tri,
            a,
            edges: e1.length() + e2.length(),
            cond_per_span: e1.length().max(e2.length()) / nd.abs(),
            t0,
            tv,
            g: [w0.0, w1.0, w2.0],
            h: [w0.1, w1.1, w2.1],
            size: a.abs().max(b.abs()).max(c.abs()).max_element(),
            n_len: nn.sqrt(),
            nd: nd.abs(),
        }
    }

    #[inline]
    fn classify(&self, o: glam::DVec3, t_max: f64) -> Crossing {
        let r = o - self.a;
        let span = r.x.abs() + r.y.abs() + r.z.abs() + self.edges;
        let t = self.t0 + self.tv.dot(o);
        let w = [self.g[0].dot(o) + self.h[0], self.g[1].dot(o) + self.h[1], self.g[2].dot(o) + self.h[2]];
        let slack = f64_slack(self.size.max(o.abs().max_element()), self.n_len, self.nd);
        verdict(w, t, t_max, span * self.cond_per_span, span, slack)
    }
}

/// What `Occluders::occluder` proved.
enum Proof {
    /// `Bvh::occluded` finds a hit; this triangle is one.
    Hit(TriId),
    /// `Bvh::occluded` finds nothing.
    Clear,
    Unknown,
}

/// Cells per side of the occluder grid.
const OCC_GRID: usize = 64;
/// A cell with more triangles than this is left to the BVH.
const OCC_CELL_MAX: usize = 48;
/// Size limit of the grid (cell entries); a bake that would need more uses the BVH alone.
const OCC_MAX_ENTRIES: usize = 1 << 22;

/// Every triangle an occlusion ray of one projected bake can hit, binned by the projector-space (x, y) of the part
/// of the projection box it overlaps (fix round 3, R3-4). An occlusion ray runs along `out` from a point of the box
/// to its front, so its (x, y) is fixed and only triangles whose padded (x, y) box contains it and whose top lies
/// above its start can be hit; `classify` decides those one by one.
struct Occluders {
    origin: glam::DVec3,
    right: glam::DVec3,
    up: glam::DVec3,
    out: glam::DVec3,
    /// Lower corner and inverse cell size of the grid in projector (x, y).
    lo: [f64; 2],
    inv_cell: [f64; 2],
    /// Rays starting below this projector z are left to the BVH (triangles below it are not binned).
    bottom: f64,
    /// Cell `k` holds `entries[offsets[k]..offsets[k + 1]]` unless `full[k]` (left to the BVH).
    offsets: Vec<u32>,
    full: Vec<bool>,
    /// The binned triangles, ascending id.
    tris: Vec<Prepped>,
    /// (index into `tris`, highest projector z of its padded extent), per cell sorted by that height, highest first.
    entries: Vec<(u32, f64)>,
    /// Per cell of the finer `COVER_GRID` grid over the same (x, y) area: the index into `tris` of the highest
    /// triangle that seems to cover the whole cell below the front (`u32::MAX`: none). Only a candidate: `covered`
    /// still proves each ray with `Prepped::classify`.
    cover: Vec<u32>,
    /// Per cell of the cover grid, the depth below which a fragment is proven hidden (`floor_grid`; `-inf`: none),
    /// and the f32 lower corner and inverse cell size of that grid, for `below_floor`.
    floor: Vec<f32>,
    floor_lo: [f32; 2],
    floor_inv: [f32; 2],
}

/// Cells per side of the cover grid (`Occluders::cover`).
const COVER_GRID: usize = 512;
/// Bakes with fewer fragments than this (estimated from the jobs' clip boxes) build no cover and floor grids.
const FINE_MIN_FRAGMENTS: f64 = 1e6;
/// Bakes with fewer fragments than this build no occluder grid at all: the BVH answers their few rays faster than the
/// grid is built (`Occluders::build` queries, prepares and sorts every triangle in the box depth).
const COARSE_MIN_FRAGMENTS: f64 = 2e5;
/// Size limit of the cover search (triangle-cell pairs); above it the cover grid stays empty.
const COVER_MAX_PAIRS: usize = 1 << 24;
/// Scratch bytes `raster_jobs` holds at once: sets are rastered in groups whose summed scratch stays under this (a
/// group always takes at least one set, so one set larger than this is rastered alone, as before).
pub(crate) const SCRATCH_BUDGET: usize = 256 << 20;

impl Occluders {
    /// The grid over `|x| <= half.x`, `|y| <= half.y` (the projection box plus the filter border) for rays that start
    /// at `z >= -back` and end at `z <= front`. `None` when it would get too large.
    fn build(mesh: &MeshData, bvh: &Bvh, ctx: &ProjectCtx<'_>, half: Vec2, fine: bool) -> Option<Occluders> {
        let diag = mesh.diag() as f64;
        // Covers the offset of the ray origins (1e-4 diag), the triangle test's tolerance and every rounding.
        let pad = 1e-3 * diag + 1e-9;
        let (hx, hy) = (half.x as f64 + pad, half.y as f64 + pad);
        let (front, back) = (ctx.front as f64 + pad, ctx.back as f64 + pad);
        if !(hx.is_finite() && hy.is_finite() && front.is_finite() && back.is_finite()) {
            return None;
        }
        let radius = (hx * hx + hy * hy + front.max(back).powi(2)).sqrt() * 1.01 + pad;
        let mut candidates = Vec::new();
        bvh.query_sphere(mesh, ctx.origin, radius as f32, &mut candidates);
        let (origin, right, up, out) = (ctx.origin.as_dvec3(), ctx.right.as_dvec3(), ctx.up.as_dvec3(), ctx.out.as_dvec3());
        let local = |p: Vec3| {
            let q = p.as_dvec3() - origin;
            glam::DVec3::new(q.dot(right), q.dot(up), q.dot(out))
        };
        let n = OCC_GRID;
        let (lo, cell) = ([-hx, -hy], [2.0 * hx / n as f64, 2.0 * hy / n as f64]);
        let cell_of = |v: f64, axis: usize| ((v - lo[axis]) / cell[axis]).floor().clamp(0.0, (n - 1) as f64) as usize;
        let mut boxes: Vec<(u32, f64, [usize; 4])> = Vec::new();
        let mut tris: Vec<Prepped> = Vec::new();
        // The padded projector (x, y) box of each binned triangle, for `cover_grid`.
        let mut flat: Vec<[f64; 4]> = Vec::new();
        for tri in candidates {
            let [p0, p1, p2] = mesh.tri_positions(tri).map(local);
            let edge = (p1 - p0).length().max((p2 - p0).length()).max((p2 - p1).length());
            let grow = pad + 4.0 * 1e-6 * edge;
            let (min, max) = (p0.min(p1).min(p2) - grow, p0.max(p1).max(p2) + grow);
            if !(min.is_finite() && max.is_finite()) {
                return None;
            }
            if max.x < -hx || min.x > hx || max.y < -hy || min.y > hy || max.z < -back || min.z > front {
                continue;
            }
            boxes.push((tris.len() as u32, max.z, [cell_of(min.x, 0), cell_of(max.x, 0), cell_of(min.y, 1), cell_of(max.y, 1)]));
            tris.push(Prepped::new(tri, mesh.tri_positions(tri), out));
            flat.push([min.x, max.x, min.y, max.y]);
        }
        let cover = if fine { cover_grid(&tris, &flat, [origin, right, up], lo, [2.0 * hx, 2.0 * hy], ctx.front as f64) } else { Vec::new() };
        // The start offset of the rays (1e-4 diag) plus the f32 rounding of a fragment's projector coordinates.
        let m = 1.01e-4 * diag + 2e-6 * (2.0 * origin.abs().element_sum() + hx + hy + front + back);
        let floor =
            floor_grid(&tris, &cover, [origin, right, up, out], lo, [2.0 * hx, 2.0 * hy], ctx.front as f64, ctx.back as f64, m);
        let floor_cell = [2.0 * hx / COVER_GRID as f64, 2.0 * hy / COVER_GRID as f64];
        let mut counts = vec![0usize; n * n];
        for (_, _, [x0, x1, y0, y1]) in &boxes {
            for y in *y0..=*y1 {
                for x in *x0..=*x1 {
                    counts[y * n + x] += 1;
                }
            }
        }
        let full: Vec<bool> = counts.iter().map(|&c| c > OCC_CELL_MAX).collect();
        let mut offsets = vec![0u32; n * n + 1];
        for k in 0..n * n {
            let kept = if full[k] { 0 } else { counts[k] };
            offsets[k + 1] = offsets[k] + kept as u32;
        }
        let total = offsets[n * n] as usize;
        if total > OCC_MAX_ENTRIES {
            return None;
        }
        let mut entries: Vec<(u32, f64)> = vec![(0, 0.0); total];
        let mut fill: Vec<u32> = offsets[..n * n].to_vec();
        for (tri, top, [x0, x1, y0, y1]) in boxes {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let k = y * n + x;
                    if !full[k] {
                        entries[fill[k] as usize] = (tri, top);
                        fill[k] += 1;
                    }
                }
            }
        }
        // Highest first: a ray stops looking at the first triangle that lies entirely below its start.
        for k in 0..n * n {
            entries[offsets[k] as usize..offsets[k + 1] as usize].sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        }
        Some(Occluders {
            origin,
            right,
            up,
            out,
            lo,
            inv_cell: [1.0 / cell[0], 1.0 / cell[1]],
            bottom: -back,
            offsets,
            full,
            tris,
            entries,
            cover,
            floor,
            floor_lo: [lo[0] as f32, lo[1] as f32],
            floor_inv: [(1.0 / floor_cell[0]) as f32, (1.0 / floor_cell[1]) as f32],
        })
    }

    /// True when the fragment at projector coordinates `l` (`ProjectCtx::local` of its surface point, a fragment that
    /// is not a gutter texel, so its ray starts there) lies below the floor of its cell (`floor_grid`): its occlusion
    /// ray certainly hits. A few f32 operations; `false` proves nothing.
    #[inline]
    fn below_floor(&self, l: Vec3) -> bool {
        let n = COVER_GRID as f32;
        let cx = (l.x - self.floor_lo[0]) * self.floor_inv[0];
        let cy = (l.y - self.floor_lo[1]) * self.floor_inv[1];
        if !(cx >= 0.0 && cy >= 0.0 && cx < n && cy < n) {
            return false;
        }
        let k = (cy as usize).min(COVER_GRID - 1) * COVER_GRID + (cx as usize).min(COVER_GRID - 1);
        self.floor.get(k).is_some_and(|&f| l.z < f)
    }

    /// True when the cover triangle of the ray's cell is proven (`Prepped::classify`) to give
    /// `Bvh::occluded(ray, 0, t_max)` a hit. Constant time: the fragment loop asks this before it samples the source,
    /// so a texel hidden under a surface costs neither a sample nor a ray. `false` proves nothing.
    #[inline]
    fn covered(&self, ray: &Ray, t_max: f32) -> bool {
        if self.cover.is_empty() || ray.dir.as_dvec3() != self.out {
            return false;
        }
        let o = ray.origin.as_dvec3();
        let q = o - self.origin;
        let (x, y, z) = (q.dot(self.right), q.dot(self.up), q.dot(self.out));
        let n = OCC_GRID as f64;
        let (cx, cy) = ((x - self.lo[0]) * self.inv_cell[0], (y - self.lo[1]) * self.inv_cell[1]);
        if !(cx >= 0.0 && cy >= 0.0 && cx < n && cy < n && z >= self.bottom) {
            return false;
        }
        let f = (COVER_GRID / OCC_GRID) as f64;
        let k = ((cy * f) as usize).min(COVER_GRID - 1) * COVER_GRID + ((cx * f) as usize).min(COVER_GRID - 1);
        let i = self.cover[k];
        i != u32::MAX && self.tris[i as usize].classify(o, t_max as f64) == Crossing::Hit
    }

    /// What `Bvh::occluded(ray, 0, t_max)` answers, where `hint` (when binned) or the triangles of the ray's cell
    /// prove it.
    fn occluder(&self, ray: &Ray, t_max: f32, hint: Option<TriId>) -> Proof {
        let o = ray.origin.as_dvec3();
        let q = o - self.origin;
        let (x, y, z) = (q.dot(self.right), q.dot(self.up), q.dot(self.out));
        let n = OCC_GRID as f64;
        let (cx, cy) = ((x - self.lo[0]) * self.inv_cell[0], (y - self.lo[1]) * self.inv_cell[1]);
        // Outside the grid or below the binned depth range (a gutter texel's ray starts at the nearest point inside
        // its triangle, which may lie outside the box), or a ray not along `out`: the BVH decides.
        if !(cx >= 0.0 && cy >= 0.0 && cx < n && cy < n && z >= self.bottom) || ray.dir.as_dvec3() != self.out {
            return Proof::Unknown;
        }
        let t_max = t_max as f64;
        if let Some(tri) = hint
            && let Ok(i) = self.tris.binary_search_by_key(&tri, |p| p.tri)
            && self.tris[i].classify(o, t_max) == Crossing::Hit
        {
            return Proof::Hit(tri);
        }
        let k = cy as usize * OCC_GRID + cx as usize;
        if self.full[k] {
            return Proof::Unknown;
        }
        let mut unknown = false;
        for &(i, top) in &self.entries[self.offsets[k] as usize..self.offsets[k + 1] as usize] {
            // The ray rises from z: a triangle entirely below its start cannot be hit, nor can the ones after it.
            if top < z {
                break;
            }
            let prepped = &self.tris[i as usize];
            match prepped.classify(o, t_max) {
                Crossing::Hit => return Proof::Hit(prepped.tri),
                Crossing::Miss => {}
                Crossing::Unknown => unknown = true,
            }
        }
        if unknown { Proof::Unknown } else { Proof::Clear }
    }
}

/// The cover grid of `Occluders`: per cell of a `COVER_GRID` x `COVER_GRID` grid over the projector (x, y) rectangle
/// `lo .. lo + size`, the index of the binned triangle that covers the whole cell (all three barycentric weights
/// positive at its four corners) with its plane below `front` there, the highest such plane (`u32::MAX`: none). A
/// ray from any point of the cell below that plane (the hidden surfaces under it) then crosses it, which
/// `Occluders::covered` proves ray by ray.
/// Empty when the search would get too large.
fn cover_grid(tris: &[Prepped], flat: &[[f64; 4]], [origin, right, up]: [glam::DVec3; 3], lo: [f64; 2], size: [f64; 2], front: f64) -> Vec<u32> {
    let n = COVER_GRID;
    let (cw, ch) = (size[0] / n as f64, size[1] / n as f64);
    if !(cw > 0.0 && ch > 0.0 && front.is_finite()) {
        return Vec::new();
    }
    // Cells lying entirely inside the (x, y) box of each triangle: only those can be covered by it.
    let range = |min: f64, max: f64, lo: f64, c: f64| {
        let first = ((min - lo) / c).ceil().max(0.0);
        let end = ((max - lo) / c).floor().min(n as f64);
        if end > first { (first as usize, end as usize) } else { (0, 0) }
    };
    let ranges: Vec<[usize; 4]> = flat
        .iter()
        .map(|&[x0, x1, y0, y1]| {
            let (a, b) = range(x0, x1, lo[0], cw);
            let (c, d) = range(y0, y1, lo[1], ch);
            [a, b, c, d]
        })
        .collect();
    let pairs: usize = ranges.iter().map(|r| (r[1] - r[0]) * (r[3] - r[2])).sum();
    if pairs > COVER_MAX_PAIRS {
        return Vec::new();
    }
    let corner = |i: usize, j: usize| origin + right * (lo[0] + i as f64 * cw) + up * (lo[1] + j as f64 * ch);
    // (lowest height of the plane over the cell, triangle index) per cell; the highest plane is kept, and of equal
    // planes the lowest index, so the parallel chunks merge to the serial answer.
    let better = |a: (f64, u32), b: (f64, u32)| if b.0 > a.0 || (b.0 == a.0 && b.1 < a.1) { b } else { a };
    let chunk = tris.len().div_ceil(rayon::current_num_threads().max(1)).max(256);
    let partial: Vec<Vec<(f64, u32)>> = tris
        .par_chunks(chunk)
        .zip(ranges.par_chunks(chunk))
        .enumerate()
        .filter(|(_, (_, r))| r.iter().any(|r| r[1] > r[0] && r[3] > r[2]))
        .map(|(c, (tris, ranges))| {
            let mut best: Vec<(f64, u32)> = vec![(f64::NEG_INFINITY, u32::MAX); n * n];
            cover_chunk(tris, ranges, c * chunk, &corner, front, &mut best);
            best
        })
        .collect();
    let mut best: Vec<(f64, u32)> = vec![(f64::NEG_INFINITY, u32::MAX); n * n];
    for part in partial {
        for (b, p) in best.iter_mut().zip(part) {
            *b = better(*b, p);
        }
    }
    best.into_iter().map(|(_, i)| i).collect()
}

/// One chunk of `cover_grid`: triangles `tris` (index `first` onwards) and their cell ranges, into `best`.
fn cover_chunk(
    tris: &[Prepped],
    ranges: &[[usize; 4]],
    first: usize,
    corner: &(impl Fn(usize, usize) -> glam::DVec3 + Sync),
    front: f64,
    best: &mut [(f64, u32)],
) {
    let n = COVER_GRID;
    for (index, (t, r)) in tris.iter().zip(ranges).enumerate() {
        let index = first + index;
        for j in r[2]..r[3] {
            for i in r[0]..r[1] {
                let mut low = f64::INFINITY;
                let mut high = f64::NEG_INFINITY;
                let mut inside = true;
                for p in [corner(i, j), corner(i + 1, j), corner(i, j + 1), corner(i + 1, j + 1)] {
                    let w = [t.g[0].dot(p) + t.h[0], t.g[1].dot(p) + t.h[1], t.g[2].dot(p) + t.h[2]];
                    if !(w[0] > 1e-9 && w[1] > 1e-9 && w[2] > 1e-9) {
                        inside = false;
                        break;
                    }
                    // The crossing distance from the plane z = 0 of the projector: the plane's height there.
                    let z = t.t0 + t.tv.dot(p);
                    low = low.min(z);
                    high = high.max(z);
                }
                let k = j * n + i;
                if inside && high < front && low.is_finite() && low > best[k].0 {
                    best[k] = (low, index as u32);
                }
            }
        }
    }
}

/// The floor grid of `Occluders` (same cells as `cover_grid`): per cell, a projector depth `F` (f32, `-inf`: none)
/// such that every occlusion ray whose origin lies within `m` of the prism `cell x [-back, F]` (projector space) is
/// proven to hit the cell's cover triangle: `verdict` returns `Hit` for all of them. The weights and the crossing
/// distance are affine in the ray origin and the error terms of `verdict` grow with `span`, `|t|`, `|w|` and the
/// coordinate size, all convex in it, so the conditions are checked at the eight corners of the prism with the
/// error terms at their largest there. `m` covers the start offset of the rays (`TriJob::bias`) and every f32
/// rounding between a fragment's projector coordinates and its ray (see `ProjectCtx::fragment`).
#[allow(clippy::too_many_arguments)]
fn floor_grid(
    tris: &[Prepped],
    cover: &[u32],
    [origin, right, up, out]: [glam::DVec3; 4],
    lo: [f64; 2],
    size: [f64; 2],
    front: f64,
    back: f64,
    m: f64,
) -> Vec<f32> {
    let n = COVER_GRID;
    if cover.len() != n * n || !(front.is_finite() && back.is_finite() && m.is_finite()) {
        return Vec::new();
    }
    let (cw, ch) = (size[0] / n as f64, size[1] / n as f64);
    let at = |x: f64, y: f64, z: f64| origin + right * x + up * y + out * z;
    let z_low = -back - m;
    cover
        .par_iter()
        .enumerate()
        .map(|(k, &c)| {
            let Some(t) = tris.get(c as usize) else { return f32::NEG_INFINITY };
            let (i, j) = (k % n, k / n);
            let xs = [lo[0] + i as f64 * cw - m, lo[0] + (i + 1) as f64 * cw + m];
            let ys = [lo[1] + j as f64 * ch - m, lo[1] + (j + 1) as f64 * ch + m];
            // Height of the cover plane over the cell (the crossing distance from z = 0).
            let heights = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(a, b)| t.t0 + t.tv.dot(at(xs[a], ys[b], 0.0)));
            let (top_min, top_max) = heights.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &z| (l.min(z), h.max(z)));
            // Worst-case error terms over the prism from `z_low` up to the plane.
            let prism = |z_hi: f64| {
                let mut corners = [glam::DVec3::ZERO; 8];
                for (q, corner) in corners.iter_mut().enumerate() {
                    *corner = at(xs[q & 1], ys[(q >> 1) & 1], if q & 4 == 0 { z_low } else { z_hi });
                }
                corners
            };
            let wide = prism(top_max);
            let span = wide.iter().map(|o| (*o - t.a).abs().element_sum()).fold(0.0, f64::max) + t.edges;
            let size = wide.iter().map(|o| o.abs().max_element()).fold(t.size, f64::max);
            let cond = span * t.cond_per_span;
            if !(cond.is_finite() && cond <= MAX_COND && top_min.is_finite() && top_max.is_finite()) {
                return f32::NEG_INFINITY;
            }
            let slack = f64_slack(size, t.n_len, t.nd);
            let t_abs = top_max - z_low + 2.0 * m;
            let err_t = ERR * cond * (span + t_abs) + slack[1] + 1e-12;
            let floor = top_min - err_t - 2.0 * m;
            if floor.partial_cmp(&z_low) != Some(std::cmp::Ordering::Greater) {
                return f32::NEG_INFINITY;
            }
            let corners = prism(floor);
            let weights = corners.map(|o| [0, 1, 2].map(|q| t.g[q].dot(o) + t.h[q]));
            let largest = weights.iter().flatten().fold(0.0f64, |a, w| a.max(w.abs()));
            let err_w = 2.0 * ERR * cond * (1.0 + largest) + slack[0] + 1e-9 + 1e-12 * (1.0 + largest);
            let proven = corners.iter().zip(&weights).enumerate().all(|(q, (o, w))| {
                let z = if q & 4 == 0 { z_low } else { floor };
                let cross = t.t0 + t.tv.dot(*o);
                w.iter().all(|&x| x > err_w) && cross > err_t && cross < front - z - 2.0 * m - err_t
            });
            if !proven {
                return f32::NEG_INFINITY;
            }
            // Rounded down to f32.
            let f = floor as f32;
            if f as f64 > floor { f.next_down() } else { f }
        })
        .collect()
}

/// `p` with `depth_front` / `depth_back` fitted to the surface under its footprint (section 6.5, "Depth auto-fit").
///
/// A `FIT_GRID` x `FIT_GRID` grid of rays along `-out` covers the placement rectangle; each ray records up to
/// `FIT_HITS_PER_RAY` hits within `FIT_REACH` half sizes of the placement plane (front faces only while `backface_cull`
/// is on). Starting from the hit nearest to the placement point at the centre, the surface is grown over the grid
/// (4-neighbours): a neighbour's hit closest in depth belongs to it when the depth step is at most
/// `grid step * (FIT_SLOPE_TOLERANCE * s + FIT_STEP_TOLERANCE)`, `s` = the steeper of the two hits' slopes
/// (`tan` of the angle between the triangle normal and the projection axis, capped at `FIT_MAX_SLOPE`). A smooth
/// surface passes, a jump at an object edge to a surface behind does not. Every cell adds its first (visible) hit only
/// when it lies within one half size of the placement plane (the placement default of earlier versions): off the
/// surface, so a decal over many separate small parts still reaches the nearby ones; on it, so a nearer part that
/// hides the grown surface (a sleeve over a bottle's body) lies inside the box, not in its fade (accept_fixes item F). The fitted depths cover all
/// that so that it lies before the fade (`extent / (1 - DEPTH_FADE_FRACTION)`), plus `FIT_MARGIN` half sizes, never
/// less than the margin and never more than the sampled reach plus the margin. Only the placement point's own
/// surface extends the box beyond one half size, and only front faces count, so the decal cannot leak through a thin
/// model to its far side; back-face culling and occlusion still apply in the bake. Deterministic: the result depends
/// only on the mesh and on `origin`, `rotation`, `half_size` and `backface_cull` of `p`, not on its current depths. A projector that is not finite or has no size,
/// or a centre without any surface, is returned unchanged.
pub fn fit_projector_depth(mesh: &MeshData, bvh: &Bvh, p: &Projector) -> Projector {
    let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
    let s = hw.max(hh);
    if !p.is_finite() || !(hw > MIN_SIZE && hh > MIN_SIZE) {
        return *p;
    }
    let (right, up, out) = p.axes();
    let reach = FIT_REACH * s;
    let cull = if p.backface_cull { Cull::Back } else { Cull::None };
    let eps = 1e-4 * s;
    let n = FIT_GRID;
    let step = |i: usize, half: f32| -half + 2.0 * half * i as f32 / (n - 1) as f32;
    // (depth = local z, slope) of the hits per grid cell, nearest to the viewer first.
    let cells: Vec<Vec<(f32, f32)>> = (0..n * n)
        .map(|k| {
            let (i, j) = (k % n, k / n);
            let start = p.origin + right * step(i, hw) + up * step(j, hh) + out * reach;
            let ray = Ray { origin: start, dir: -out };
            let mut hits = Vec::new();
            let mut t_min = 0.0;
            while hits.len() < FIT_HITS_PER_RAY {
                let Some(hit) = bvh.raycast(mesh, &ray, t_min, 2.0 * reach, cull) else { break };
                if !hit.t.is_finite() {
                    break;
                }
                let c = hit.geo_normal.dot(out).abs();
                let slope = if c > 0.0 { ((1.0 - c * c).max(0.0).sqrt() / c).min(FIT_MAX_SLOPE) } else { FIT_MAX_SLOPE };
                hits.push((reach - hit.t, if slope.is_nan() { FIT_MAX_SLOPE } else { slope }));
                t_min = hit.t + eps;
            }
            hits
        })
        .collect();
    let center = (n / 2) * n + n / 2;
    let Some(seed) = cells[center].iter().copied().min_by(|a, b| a.0.abs().total_cmp(&b.0.abs())) else { return *p };
    let (step_x, step_y) = (2.0 * hw / (n - 1) as f32, 2.0 * hh / (n - 1) as f32);
    let mut surface: Vec<Option<(f32, f32)>> = vec![None; n * n];
    surface[center] = Some(seed);
    let mut queue = std::collections::VecDeque::from([center]);
    while let Some(k) = queue.pop_front() {
        let Some((z, slope)) = surface[k] else { continue };
        let (i, j) = (k % n, k / n);
        let neighbours = [
            (i > 0).then(|| (k - 1, step_x)),
            (i + 1 < n).then(|| (k + 1, step_x)),
            (j > 0).then(|| (k - n, step_y)),
            (j + 1 < n).then(|| (k + n, step_y)),
        ];
        for (m, grid_step) in neighbours.into_iter().flatten() {
            if surface[m].is_some() {
                continue;
            }
            let best = cells[m].iter().copied().min_by(|a, b| (a.0 - z).abs().total_cmp(&(b.0 - z).abs()));
            if let Some(best) = best
                && (best.0 - z).abs() <= grid_step * (FIT_SLOPE_TOLERANCE * slope.max(best.1) + FIT_STEP_TOLERANCE)
            {
                surface[m] = Some(best);
                queue.push_back(m);
            }
        }
    }
    // The first (visible) hit of every cell still counts when it lies within the placement default depth (the larger
    // half size): off the surface, so a decal over a model of many separate parts reaches them as before; on it, when a
    // nearer part hides the tracked surface (accept_fixes item F: WaterBottle's grip sleeve stands ~4.5 mm in front of
    // the body that continues under it; the grown surface follows the hidden body, and the visible sleeve fell into the
    // front fade).
    let near_parts = cells.iter().filter_map(|hits| hits.first());
    let (lo, hi) = surface
        .iter()
        .flatten()
        .chain(near_parts.filter(|(z, _)| z.abs() <= s))
        .fold((0.0f32, 0.0f32), |(lo, hi), &(z, _)| (lo.min(z), hi.max(z)));
    let margin = FIT_MARGIN * s;
    let fit = |extent: f32| (extent / (1.0 - DEPTH_FADE_FRACTION) + margin).min(reach + margin);
    let mut fitted = *p;
    fitted.depth_front = fit(hi.max(0.0));
    fitted.depth_back = fit((-lo).max(0.0));
    fitted
}

/// True when the depths of `p` are the auto-fitted ones (`fit_projector_depth`, within a relative 1e-3), or the
/// placement default of earlier versions (`depth_front == depth_back ==` the larger half size). Such a stamp is
/// re-fitted when it is moved or scaled; one whose depth the user typed in keeps it.
pub fn projector_depth_is_auto(mesh: &MeshData, bvh: &Bvh, p: &Projector) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1e-12);
    let legacy = p.half_size.abs().max_element();
    if near(p.depth_front, legacy) && near(p.depth_back, legacy) {
        return true;
    }
    let fitted = fit_projector_depth(mesh, bvh, p);
    near(p.depth_front, fitted.depth_front) && near(p.depth_back, fitted.depth_back)
}

/// Projects `src` through `p` onto the surface and returns one cropped buffer per touched set (ascending set id).
/// `maps[i]` is the texel map of set i. Sets with no coverage are omitted.
///
/// Algorithm: section 6.5. A projector with a non-finite field, a zero size, an empty source or a tint with alpha 0
/// bakes nothing. `angle_start_deg` / `angle_end_deg` are read as `0 <= start <= end <= 90`. Deterministic: the
/// result does not depend on the thread count.
pub fn bake_projected(
    mesh: &MeshData,
    bvh: &Bvh,
    maps: &[Arc<TexelMap>],
    src: &PremulMips,
    tint: [u8; 4],
    p: &Projector,
    quality: BakeQuality,
) -> Vec<(TextureSetId, SparseBuf)> {
    let t_candidates = probe::now();
    let Some((ctx, jobs)) = projected_jobs(mesh, bvh, maps, src, tint, p, quality) else { return Vec::new() };
    probe::since(0, t_candidates);
    raster_jobs(mesh, maps, &ctx, &jobs)
}

/// Steps 1 of `bake_projected`: the fragment context and the jobs (candidate triangles) per set, in ascending
/// triangle order. `None` when the bake is empty from the start (see `bake_projected`).
fn projected_jobs<'a>(
    mesh: &'a MeshData,
    bvh: &'a Bvh,
    maps: &[Arc<TexelMap>],
    src: &'a PremulMips,
    tint: [u8; 4],
    p: &Projector,
    quality: BakeQuality,
) -> Option<(ProjectCtx<'a>, Vec<Vec<TriJob>>)> {
    let [src_w, src_h] = src.size();
    let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
    if src_w == 0 || src_h == 0 || tint[3] == 0 || maps.is_empty() || !p.is_finite() || !(hw > MIN_SIZE && hh > MIN_SIZE) {
        return None;
    }
    let (right, up, out) = p.axes();
    let front = p.depth_front.max(0.0);
    let back = p.depth_back.max(0.0);
    let angle_start = p.angle_start_deg.clamp(0.0, 90.0);
    let angle_end = p.angle_end_deg.clamp(angle_start, 90.0);
    // One source texel in world units, and source texels per world unit along the more strongly minified axis.
    let (border_x, border_y) = (2.0 * hw / src_w as f32, 2.0 * hh / src_h as f32);
    let src_px_per_world = (src_w as f32 / (2.0 * hw)).max(src_h as f32 / (2.0 * hh));
    let diag = mesh.diag();
    let mut ctx = ProjectCtx {
        mesh,
        bvh,
        src,
        quality,
        origin: p.origin,
        right,
        up,
        out,
        inv_width: 0.5 / hw,
        inv_height: 0.5 / hh,
        front,
        back,
        cos_start: angle_start.to_radians().cos(),
        cos_end: angle_end.to_radians().cos(),
        backface_cull: p.backface_cull,
        occlusion: p.occlusion,
        flip_x: p.flip_x,
        flip_y: p.flip_y,
        tint: tint_factors(tint),
        occluders: None,
    };

    // 1. Candidates: triangles near the box, UV-mapped, facing the projector, and really touching the box.
    let mean_texel = maps.iter().map(|m| m.stats.texel_world).filter(|t| t.is_finite()).fold(0.0f32, f32::max);
    let radius = Vec3::new(hw + border_x + 2.0 * mean_texel, hh + border_y + 2.0 * mean_texel, front.max(back)).length();
    let mut candidates: Vec<TriId> = Vec::new();
    bvh.query_sphere(mesh, p.origin, radius, &mut candidates);
    let mut jobs: Vec<Vec<TriJob>> = maps.iter().map(|_| Vec::new()).collect();
    for tri in candidates {
        let Some(set) = mesh.tri_set(tri) else { continue };
        let Some(map) = maps.get(set.index()) else { continue };
        if map.width == 0 || map.height == 0 {
            continue;
        }
        let geo = mesh.geo_normal(tri);
        let toward = geo.dot(out);
        if p.backface_cull && toward <= 0.0 {
            continue;
        }
        let pos = mesh.tri_positions(tri);
        let size = Vec2::new(map.width as f32, map.height as f32);
        let uv_px = mesh.tri_uvs(tri).map(|uv| uv * size);
        let area_world = 0.5 * (pos[1] - pos[0]).cross(pos[2] - pos[0]).length();
        let area_px = 0.5 * (uv_px[1] - uv_px[0]).perp_dot(uv_px[2] - uv_px[0]).abs();
        let texel_world = (area_world / area_px).sqrt();
        let texel_world =
            if texel_world.is_finite() && texel_world > 0.0 { texel_world } else { map.stats.texel_world.max(0.0) };
        let (limit_x, limit_y) = (hw + border_x.max(texel_world), hh + border_y.max(texel_world));
        let local = pos.map(|q| ctx.local(q));
        let Some(clip) = footprint(&local, &uv_px, limit_x, limit_y, front, back, map.gutter as f32 + 2.0) else {
            continue;
        };
        jobs[set.index()].push(TriJob {
            tri,
            clip,
            lod: (src_px_per_world * texel_world).max(1.0).log2(),
            limit_x,
            limit_y,
            geo,
            bias: (if toward < 0.0 { -geo } else { geo }) * (1e-4 * diag),
        });
    }

    #[cfg(test)]
    let grid_on = !probe::NO_GRID.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let grid_on = true;
    if grid_on && ctx.occlusion && ctx.quality == BakeQuality::Full && jobs.iter().any(|j| !j.is_empty()) {
        let half = jobs.iter().flatten().fold(Vec2::ZERO, |m, j| m.max(Vec2::new(j.limit_x, j.limit_y)));
        // The cover and floor grids (about 10 ms to build) only pay off for a bake of many fragments.
        let fragments: f64 = jobs.iter().flatten().map(|j| ((j.clip[2] - j.clip[0]) * (j.clip[3] - j.clip[1])).max(0.0) as f64 * 0.5).sum();
        // `Some(fine)`: build the grid, with the cover and floor grids when `fine`. A small bake builds none.
        let grid = (fragments >= COARSE_MIN_FRAGMENTS).then_some(fragments >= FINE_MIN_FRAGMENTS);
        #[cfg(test)]
        let grid = probe::FORCE_FINE.with(|f| f.get()).or(grid);
        if let Some(fine) = grid {
            let t_build = probe::now();
            ctx.occluders = Occluders::build(mesh, bvh, &ctx, half, fine);
            probe::since(11, t_build);
        }
    }
    Some((ctx, jobs))
}

/// Steps 2 to 4 of `bake_projected`: per set, a dense scratch over the union of the footprints, filled in 64-row bands,
/// then cropped. The sets are taken in groups whose scratches together stay within `SCRATCH_BUDGET` (one parallel
/// pass per group).
fn raster_jobs(mesh: &MeshData, maps: &[Arc<TexelMap>], ctx: &ProjectCtx<'_>, jobs: &[Vec<TriJob>]) -> Vec<(TextureSetId, SparseBuf)> {
    /// The scratch box of one set, `[x0, x1) x [y0, y1)` texels, and the jobs of each band.
    struct Area {
        set: usize,
        x0: i64,
        x1: i64,
        y0: i64,
        y1: i64,
        bins: Vec<Vec<u32>>,
    }
    let t_raster = probe::now();
    let mut areas: Vec<Area> = Vec::new();
    for (s, jobs) in jobs.iter().enumerate() {
        if jobs.is_empty() {
            continue;
        }
        probe::count(7, jobs.len() as u64);
        let map = &*maps[s];
        let (w, h) = (map.width as i64, map.height as i64);
        // Texel columns / rows a clip interval selects; a wrapping interval forces the whole axis.
        let span = |lo: f32, hi: f32, size: i64| -> (i64, i64) {
            let (a, b) = (lo.floor() as i64, (hi.floor() as i64).saturating_add(1));
            if a >= 0 && b <= size { (a, b) } else { (0, size) }
        };
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0, h, 0);
        for job in jobs {
            let (a, b) = span(job.clip[0], job.clip[2], w);
            x0 = x0.min(a);
            x1 = x1.max(b);
            let (a, b) = span(job.clip[1], job.clip[3], h);
            y0 = y0.min(a);
            y1 = y1.max(b);
        }
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let bh = (y1 - y0) as usize;
        let band_count = bh.div_ceil(BAND_ROWS);
        let mut bins: Vec<Vec<u32>> = vec![Vec::new(); band_count];
        for (j, job) in jobs.iter().enumerate() {
            let (a, b) = span(job.clip[1], job.clip[3], h);
            let first = (a.max(y0) - y0) as usize / BAND_ROWS;
            let last = ((b.min(y1) - 1 - y0).max(0) as usize / BAND_ROWS).min(band_count - 1);
            for bin in &mut bins[first..=last] {
                bin.push(j as u32);
            }
        }
        areas.push(Area { set: s, x0, x1, y0, y1, bins });
    }
    let bytes_of = |a: &Area| ((a.x1 - a.x0) * (a.y1 - a.y0) * 4) as usize;
    probe::count(3, areas.iter().map(|a| bytes_of(a) as u64 / 4).sum());
    #[cfg(test)]
    let budget = probe::SCRATCH_BUDGET.with(|b| b.get()).unwrap_or(SCRATCH_BUDGET);
    #[cfg(not(test))]
    let budget = SCRATCH_BUDGET;

    // The bands of every set of a group in one parallel pass. Each texel still sees its fragments in job order,
    // whatever the band layout, the grouping or the thread count, so the result does not depend on any of them.
    let mut results = Vec::new();
    let mut crop_ns = 0u64;
    for group in scratch_groups(&areas.iter().map(bytes_of).collect::<Vec<_>>(), budget) {
        let areas = &areas[group];
        let mut scratches: Vec<Vec<u8>> = areas.iter().map(|a| vec![0u8; bytes_of(a)]).collect();
        #[cfg(test)]
        probe::PEAK_SCRATCH.with(|p| p.set(p.get().max(scratches.iter().map(Vec::len).sum())));
        let mut bands: Vec<(&Area, usize, &mut [u8])> = Vec::new();
        for (area, data) in areas.iter().zip(&mut scratches) {
            let bw = (area.x1 - area.x0) as usize;
            bands.extend(data.chunks_mut(BAND_ROWS * bw * 4).enumerate().map(|(band, pixels)| (area, band, pixels)));
        }
        bands.into_par_iter().for_each(|(area, band, pixels)| {
            let (map, jobs) = (&*maps[area.set], &jobs[area.set]);
            let (x0, x1, bw) = (area.x0, area.x1, (area.x1 - area.x0) as usize);
            let row0 = area.y0 as u32 + (band * BAND_ROWS) as u32;
            let rows = row0..row0 + (pixels.len() / (bw * 4)) as u32;
            // The last occluder met in this band (`ProjectCtx::occluded`).
            let mut hint: Option<TriId> = None;
            for &j in &area.bins[band] {
                let job = &jobs[j as usize];
                let pos = mesh.tri_positions(job.tri);
                let normals = mesh.tri_normals(job.tri);
                raster_tri(mesh, map, job.tri, Some(job.clip), rows.clone(), &mut |f: Fragment| {
                    let x = f.x as i64;
                    if x < x0 || x >= x1 || f.y < row0 {
                        return;
                    }
                    let i = ((f.y - row0) as usize * bw + (x - x0) as usize) * 4;
                    let Some(stored) = pixels.get(i + 3).copied() else { return };
                    // Max-alpha rule: idempotent for texels visited more than once. A texel at full alpha cannot change
                    // any more (on tiled UVs a large decal reaches each texel through many images).
                    if stored == u8::MAX {
                        return;
                    }
                    #[cfg(test)]
                    probe::count(4, 1);
                    if let Some(pixel) = ctx.fragment(job, &pos, &normals, &f, stored, Some(&mut hint)) {
                        pixels[i..i + 4].copy_from_slice(&pixel);
                    }
                });
            }
        });

        // 4. Crop to the content.
        let t_crop = std::time::Instant::now();
        for (area, data) in areas.iter().zip(scratches) {
            let rect = IRect::new(area.x0 as i32, area.y0 as i32, area.x1 as i32, area.y1 as i32);
            let cropped = crop_scratch(rect, data);
            if !cropped.is_empty() {
                results.push((TextureSetId(area.set as u32), cropped));
            }
        }
        crop_ns += t_crop.elapsed().as_nanos() as u64;
    }
    if let Some(t) = t_raster {
        probe::count(1, (t.elapsed().as_nanos() as u64).saturating_sub(crop_ns));
    }
    probe::count(2, crop_ns);
    results
}

/// Consecutive groups of the scratch sizes `bytes` whose sums stay within `budget`; a size above the budget forms a
/// group of its own.
pub(crate) fn scratch_groups(bytes: &[usize], budget: usize) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let (mut start, mut sum) = (0, 0usize);
    for (i, &b) in bytes.iter().enumerate() {
        if i > start && sum.saturating_add(b) > budget {
            groups.push(start..i);
            (start, sum) = (i, 0);
        }
        sum = sum.saturating_add(b);
    }
    if start < bytes.len() {
        groups.push(start..bytes.len());
    }
    groups
}

/// Places `src` directly in the texture of `p.set`. `map` is that set's texel map (used for clip_to_islands and the size).
///
/// Algorithm: section 6.5. No wrapping: the part of the decal outside the texture is dropped. A non-finite placement,
/// a zero size, an empty source or a tint with alpha 0 bakes nothing.
pub fn bake_uv(src: &PremulMips, tint: [u8; 4], p: &UvPlacement, map: &TexelMap) -> SparseBuf {
    let [src_w, src_h] = src.size();
    let size = p.size_px;
    if src_w == 0
        || src_h == 0
        || map.width == 0
        || map.height == 0
        || tint[3] == 0
        || !p.is_finite()
        || !(size.x.abs() > MIN_SIZE && size.y.abs() > MIN_SIZE)
    {
        return SparseBuf::empty();
    }
    let (right, down) = p.frame();
    // Source texels per target texel along the more strongly minified axis.
    let scale = (src_w as f32 / size.x.abs()).max(src_h as f32 / size.y.abs()).max(1.0);
    let lod = scale.log2();
    // The filter border: one texel of the sampled level, in target texels.
    let border = (scale * size.x.abs() / src_w as f32).max(scale * size.y.abs() / src_h as f32);
    let margin = border.ceil().min(1e6) + 1.0;
    let corners = p.corners_px();
    let (mut lo, mut hi) = (Vec2::INFINITY, Vec2::NEG_INFINITY);
    for c in corners {
        lo = lo.min(c);
        hi = hi.max(c);
    }
    let clamp = |v: f32, size: u32| (v as i64).clamp(0, size as i64);
    let (x0, x1) = (clamp((lo.x - margin).floor(), map.width), clamp((hi.x + margin).ceil(), map.width));
    let (y0, y1) = (clamp((lo.y - margin).floor(), map.height), clamp((hi.y + margin).ceil(), map.height));
    if x0 >= x1 || y0 >= y1 {
        return SparseBuf::empty();
    }

    let bw = (x1 - x0) as usize;
    let mut scratch = vec![0u8; bw * (y1 - y0) as usize * 4];
    let tint = tint_factors(tint);
    let (inv_w, inv_h) = (size.x.recip(), size.y.recip());
    scratch.par_chunks_mut(bw * 4).enumerate().for_each(|(row, pixels)| {
        let y = y0 as u32 + row as u32;
        for (i, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = x0 as u32 + i as u32;
            if p.clip_to_islands && map.owner(x, y).is_none() {
                continue;
            }
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - p.center_px;
            let mut u = 0.5 + d.dot(right) * inv_w;
            let mut v = 0.5 + d.dot(down) * inv_h;
            if p.flip_x {
                u = 1.0 - u;
            }
            if p.flip_y {
                v = 1.0 - v;
            }
            if let Some(shaded) = shade(src.sample(u, v, lod), tint, 1.0) {
                *pixel = shaded;
            }
        }
    });
    crop_scratch(IRect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32), scratch)
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use crate::pixel::PixelBuf;

    /// Texel maps of every set of `mesh` at `size` x `size` with the default gutter.
    pub(crate) fn maps_of(mesh: &MeshData, size: u32) -> Vec<Arc<TexelMap>> {
        let gutter = TexelMap::default_gutter(size, size);
        (0..mesh.sets.len())
            .map(|s| Arc::new(TexelMap::build(mesh, TextureSetId(s as u32), size, size, gutter)))
            .collect()
    }

    /// The bake as section 6.5 states it, written plainly: every job of every set rasterized over the whole texture
    /// in one pass, every fragment shaded and occlusion-tested, then compared with the stored alpha.
    pub(crate) fn reference_bake(
        mesh: &MeshData,
        bvh: &Bvh,
        maps: &[Arc<TexelMap>],
        src: &PremulMips,
        tint: [u8; 4],
        p: &Projector,
        quality: BakeQuality,
    ) -> Vec<PixelBuf> {
        let mut out: Vec<PixelBuf> = maps.iter().map(|m| PixelBuf::new(m.width, m.height)).collect();
        let Some((mut ctx, jobs)) = projected_jobs(mesh, bvh, maps, src, tint, p, quality) else { return out };
        // Every ray to the BVH.
        ctx.occluders = None;
        for (s, jobs) in jobs.iter().enumerate() {
            let map = &*maps[s];
            let image = &mut out[s];
            for job in jobs {
                let pos = mesh.tri_positions(job.tri);
                let normals = mesh.tri_normals(job.tri);
                raster_tri(mesh, map, job.tri, Some(job.clip), 0..map.height, &mut |f: Fragment| {
                    let stored = image.get(f.x, f.y)[3];
                    // Stored alpha 0 for the fragment: every visible fragment is shaded and ray-tested.
                    if let Some(pixel) = ctx.fragment(job, &pos, &normals, &f, 0, None)
                        && pixel[3] > stored
                    {
                        image.set(f.x, f.y, pixel);
                    }
                });
            }
        }
        out
    }

    /// The bake result of one set as a dense image (transparent when the set received nothing).
    pub(crate) fn dense(result: &[(TextureSetId, SparseBuf)], set: u32, size: u32) -> PixelBuf {
        match result.iter().find(|(id, _)| id.0 == set) {
            Some((_, buf)) => buf.to_dense(size, size),
            None => PixelBuf::new(size, size),
        }
    }

    /// Number of pixels with alpha > 0 inside the UV rectangle `[u0, u1, v0, v1]`.
    pub(crate) fn coverage_in(image: &PixelBuf, uv: [f32; 4]) -> u32 {
        let (w, h) = (image.width as f32, image.height as f32);
        let mut n = 0;
        for y in (uv[2] * h).floor() as u32..((uv[3] * h).ceil() as u32).min(image.height) {
            for x in (uv[0] * w).floor() as u32..((uv[1] * w).ceil() as u32).min(image.width) {
                n += (image.get(x, y)[3] > 0) as u32;
            }
        }
        n
    }

    /// Centroid (in 0..1 image coordinates) of the pixels for which `pick` is true; `None` when there are none.
    pub(crate) fn centroid(image: &PixelBuf, pick: impl Fn([u8; 4]) -> bool) -> Option<Vec2> {
        let (mut sum, mut n) = (Vec2::ZERO, 0u32);
        for y in 0..image.height {
            for x in 0..image.width {
                if pick(image.get(x, y)) {
                    sum += Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    n += 1;
                }
            }
        }
        (n > 0).then(|| sum / n as f32 / Vec2::new(image.width as f32, image.height as f32))
    }

    /// The yellow dot of `samples::sample_stamp_image` (lower right of the centre: (0.7, 0.62)).
    pub(crate) fn is_dot(p: [u8; 4]) -> bool {
        p[3] > 200 && p[0] > 200 && p[1] > 150 && p[2] < 110
    }

    /// The red arrow of `samples::sample_stamp_image` (pointing up; its tip is at (0.5, 0.2)).
    pub(crate) fn is_arrow(p: [u8; 4]) -> bool {
        p[3] > 200 && p[0] > 180 && p[1] < 110 && p[2] < 110
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::testkit::*;
    use super::*;
    use crate::pixel::PixelBuf;
    use crate::pixel_ops::{content_bounds, resize};
    use crate::samples;
    use crate::testutil::{mesh_of, psnr};

    const WHITE: [u8; 4] = [255; 4];

    fn solid(rgba: [u8; 4]) -> PremulMips {
        PremulMips::build(&PixelBuf::filled(16, 16, rgba))
    }

    fn stamp(size: u32) -> PremulMips {
        PremulMips::build(&samples::sample_stamp_image(size))
    }

    /// Projector facing -Z onto the plane z = `z`, centred at (cx, cy), image up = +Y.
    fn front_projector(cx: f32, cy: f32, z: f32, half: f32, depth: f32) -> Projector {
        Projector::from_surface(Vec3::new(cx, cy, z), Vec3::Z, Vec3::Y, Vec2::splat(half), depth)
    }

    fn uv_placement(center: [f32; 2], size: [f32; 2]) -> UvPlacement {
        UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::from(center),
            size_px: Vec2::from(size),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn from_surface_builds_a_right_handed_frame() {
        let p = Projector::from_surface(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, 2.0), Vec3::Y, Vec2::new(0.5, 0.25), 0.3);
        let (right, up, out) = p.axes();
        assert!(close(right, Vec3::X) && close(up, Vec3::Y) && close(out, Vec3::Z), "{right} {up} {out}");
        assert_eq!((p.depth_front, p.depth_back), (0.3, 0.3));
        assert_eq!((p.angle_start_deg, p.angle_end_deg), (70.0, 88.0));
        assert!(p.backface_cull && p.occlusion && !p.flip_x && !p.flip_y);
        assert_eq!(p.origin, Vec3::new(1.0, 2.0, 3.0));

        // A tilted up hint is projected onto the tangent plane.
        let n = Vec3::new(1.0, 1.0, 0.0).normalize();
        let p = Projector::from_surface(Vec3::ZERO, n, Vec3::Y, Vec2::ONE, 1.0);
        let (right, up, out) = p.axes();
        assert!(close(out, n));
        assert!(up.dot(out).abs() < 1e-5 && up.dot(Vec3::Y) > 0.5);
        assert!(close(right.cross(up), out), "right-handed");
        assert!(close(right, up.cross(out)));

        // Up hint parallel to the normal, zero or NaN: any orthonormal frame.
        for hint in [Vec3::Z, Vec3::NEG_Z * 3.0, Vec3::ZERO, Vec3::NAN] {
            let p = Projector::from_surface(Vec3::ZERO, Vec3::Z, hint, Vec2::ONE, 1.0);
            let (right, up, out) = p.axes();
            assert!(p.is_finite());
            assert!(close(out, Vec3::Z) && close(right.cross(up), out), "{hint}");
            assert!((right.length() - 1.0).abs() < 1e-5 && (up.length() - 1.0).abs() < 1e-5);
        }
        // A normal without direction counts as +Z.
        let p = Projector::from_surface(Vec3::ZERO, Vec3::ZERO, Vec3::Y, Vec2::ONE, 1.0);
        assert!(close(p.axes().2, Vec3::Z));
    }

    #[test]
    fn corners_radius_and_finiteness() {
        let p = Projector::from_surface(Vec3::new(1.0, 1.0, 0.0), Vec3::Z, Vec3::Y, Vec2::new(0.5, 0.25), 2.0);
        let [tl, tr, br, bl] = p.corners();
        assert!(close(tl, Vec3::new(0.5, 1.25, 0.0)) && close(tr, Vec3::new(1.5, 1.25, 0.0)));
        assert!(close(br, Vec3::new(1.5, 0.75, 0.0)) && close(bl, Vec3::new(0.5, 0.75, 0.0)));
        assert!((p.bounding_radius() - (0.25f32 + 0.0625 + 4.0).sqrt()).abs() < 1e-6);
        let mut deep = p;
        deep.depth_back = 5.0;
        assert!((deep.bounding_radius() - (0.25f32 + 0.0625 + 25.0).sqrt()).abs() < 1e-5);
        // Every corner and every box corner lies inside the bounding sphere.
        for c in p.corners() {
            assert!((c - p.origin).length() <= p.bounding_radius());
        }

        assert!(p.is_finite());
        for break_it in [
            (|p: &mut Projector| p.origin.x = f32::NAN) as fn(&mut Projector),
            |p| p.rotation = Quat::from_xyzw(f32::INFINITY, 0.0, 0.0, 1.0),
            |p| p.half_size.y = f32::NAN,
            |p| p.depth_front = f32::INFINITY,
            |p| p.depth_back = f32::NAN,
            |p| p.angle_start_deg = f32::NAN,
            |p| p.angle_end_deg = f32::NEG_INFINITY,
        ] {
            let mut broken = p;
            break_it(&mut broken);
            assert!(!broken.is_finite());
        }

        let mut uv = uv_placement([100.0, 50.0], [40.0, 20.0]);
        assert_eq!(
            uv.corners_px(),
            [Vec2::new(80.0, 40.0), Vec2::new(120.0, 40.0), Vec2::new(120.0, 60.0), Vec2::new(80.0, 60.0)]
        );
        assert!(uv.is_finite());
        // +90 degrees clockwise on the v-down texture: the top-left corner moves to the top right.
        uv.rotation = FRAC_PI_2;
        let c = uv.corners_px();
        assert!((c[0] - Vec2::new(110.0, 30.0)).length() < 1e-3, "{:?}", c);
        assert!((c[1] - Vec2::new(110.0, 70.0)).length() < 1e-3);
        uv.rotation = f32::NAN;
        assert!(!uv.is_finite());
        uv.rotation = 0.0;
        uv.size_px.x = f32::INFINITY;
        assert!(!uv.is_finite());
    }

    #[test]
    fn rotated_and_moved_to() {
        let p = front_projector(0.0, 0.0, 0.0, 1.0, 1.0);
        // +90 degrees: the image's top edge (local +Y) points to the viewer's right (world +X, seen from +Z).
        let turned = p.rotated(FRAC_PI_2);
        let (right, up, out) = turned.axes();
        assert!(close(up, Vec3::X) && close(right, Vec3::NEG_Y) && close(out, Vec3::Z), "{right} {up} {out}");
        let [tl, tr, ..] = turned.corners();
        assert!(close(tl, Vec3::new(1.0, 1.0, 0.0)) && close(tr, Vec3::new(1.0, -1.0, 0.0)));
        // Rotations add up and a full turn is the identity.
        let (r2, u2, _) = p.rotated(0.3).rotated(0.4).axes();
        let (r1, u1, _) = p.rotated(0.7).axes();
        assert!(close(r1, r2) && close(u1, u2));
        assert!(close(p.rotated(std::f32::consts::TAU).axes().0, Vec3::X));

        // Moving onto a face with another normal turns the frame by the shortest arc: here about the Y axis.
        let moved = p.moved_to(Vec3::new(5.0, 0.0, 0.0), Vec3::X * 2.0);
        let (right, up, out) = moved.axes();
        assert_eq!(moved.origin, Vec3::new(5.0, 0.0, 0.0));
        assert!(close(out, Vec3::X) && close(up, Vec3::Y) && close(right, Vec3::NEG_Z), "{right} {up} {out}");
        assert_eq!((moved.half_size, moved.depth_front, moved.backface_cull), (p.half_size, p.depth_front, true));
        // Same normal: only the position changes. Opposite normal: still an orthonormal frame facing the new way.
        let same = p.moved_to(Vec3::ONE, Vec3::Z);
        assert!(close(same.axes().0, Vec3::X) && close(same.axes().1, Vec3::Y));
        let flipped = p.moved_to(Vec3::ZERO, Vec3::NEG_Z);
        let (right, up, out) = flipped.axes();
        assert!(close(out, Vec3::NEG_Z) && close(right.cross(up), out));
        // A degenerate normal keeps the orientation.
        let kept = p.rotated(0.5).moved_to(Vec3::ONE, Vec3::ZERO);
        assert!(close(kept.axes().0, p.rotated(0.5).axes().0));
        assert_eq!(kept.origin, Vec3::ONE);
    }

    #[test]
    fn projection_matches_direct_resampling() {
        let mesh = mesh_of(samples::flat_quad());
        let bvh = Bvh::build(&mesh);
        // The projector covers the quad exactly, so the baked texture is the stamp image resampled to the map size.
        let p = front_projector(0.5, 0.5, 0.0, 0.5, 0.5);
        // 1:1, a power-of-two minification and a magnification match within 40 dB. A fractional minification (4:3,
        // lod 0.415) blends two mip levels, which approximates the reference's area filter less closely.
        for (src_size, map_size, min_db) in [(256u32, 256u32, 40.0), (512, 128, 40.0), (64, 256, 40.0), (128, 96, 36.0)] {
            let image = samples::sample_stamp_image(src_size);
            let maps = maps_of(&mesh, map_size);
            for quality in [BakeQuality::Full, BakeQuality::Preview] {
                let result = bake_projected(&mesh, &bvh, &maps, &PremulMips::build(&image), WHITE, &p, quality);
                assert_eq!(result.len(), 1);
                // Compared premultiplied, i.e. as seen: the straight RGB of nearly transparent texels depends on
                // rounding (a premultiplied u8 at alpha 3 carries the colour to within +-40 levels).
                let visible = |image: &PixelBuf| -> PixelBuf {
                    let mut out = image.clone();
                    for px in out.data.as_chunks_mut::<4>().0 {
                        *px = crate::pixel_ops::premultiply(*px);
                    }
                    out
                };
                let baked = visible(&dense(&result, 0, map_size));
                let reference = visible(&resize(&image, map_size, map_size));
                let quality_db = psnr(&baked, &reference);
                assert!(quality_db > min_db, "{src_size} -> {map_size} ({quality:?}): {quality_db:.1} dB");
            }
        }
    }

    #[test]
    fn partial_projection_lands_where_the_box_is() {
        let mesh = mesh_of(samples::flat_quad());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        // A quarter-size decal centred at world (0.25, 0.75) = texture (0.25, 0.25).
        let p = front_projector(0.25, 0.75, 0.0, 0.125, 0.5);
        let result = bake_projected(&mesh, &bvh, &maps, &solid([10, 200, 30, 255]), WHITE, &p, BakeQuality::Full);
        let (_, buf) = &result[0];
        // 64 x 64 texels, plus the half-transparent filter border.
        assert!(buf.bounds().x0 >= 30 && buf.bounds().x0 <= 32 && buf.bounds().x1 >= 96 && buf.bounds().x1 <= 98, "{:?}", buf.bounds());
        assert!(buf.bounds().y0 >= 30 && buf.bounds().y0 <= 32 && buf.bounds().y1 >= 96 && buf.bounds().y1 <= 98, "{:?}", buf.bounds());
        assert_eq!(buf.get(64, 64), [10, 200, 30, 255]);
        assert_eq!(buf.get(40, 90), [10, 200, 30, 255]);
        assert_eq!(buf.get(20, 64), [0; 4]);
        // Cropped rect is tight.
        assert_eq!(content_bounds(buf.to_cropped().view()), Some(buf.bounds()));

        // Tint multiplies all four channels.
        let tinted = bake_projected(&mesh, &bvh, &maps, &solid([255, 200, 100, 255]), [255, 128, 0, 128], &p, BakeQuality::Full);
        assert_eq!(tinted[0].1.get(64, 64), [255, 100, 0, 128]);
        // Deterministic.
        let again = bake_projected(&mesh, &bvh, &maps, &solid([10, 200, 30, 255]), WHITE, &p, BakeQuality::Full);
        assert_eq!(again, result);
    }

    #[test]
    fn flips_land_where_expected() {
        let mesh = mesh_of(samples::flat_quad());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let src = stamp(256);
        let dot_of = |flip_x: bool, flip_y: bool| -> Vec2 {
            let mut p = front_projector(0.5, 0.5, 0.0, 0.5, 0.5);
            p.flip_x = flip_x;
            p.flip_y = flip_y;
            let baked = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, BakeQuality::Full), 0, 256);
            centroid(&baked, is_dot).expect("the dot is visible")
        };
        let near = |a: Vec2, b: [f32; 2]| (a - Vec2::from(b)).length() < 0.02;
        assert!(near(dot_of(false, false), [0.7, 0.62]), "{}", dot_of(false, false));
        assert!(near(dot_of(true, false), [0.3, 0.62]), "{}", dot_of(true, false));
        assert!(near(dot_of(false, true), [0.7, 0.38]), "{}", dot_of(false, true));
        assert!(near(dot_of(true, true), [0.3, 0.38]), "{}", dot_of(true, true));

        // The same through bake_uv.
        let map = &maps[0];
        let dot_uv = |flip_x: bool, flip_y: bool| -> Vec2 {
            let mut p = uv_placement([128.0, 128.0], [256.0, 256.0]);
            p.flip_x = flip_x;
            p.flip_y = flip_y;
            centroid(&bake_uv(&src, WHITE, &p, map).to_dense(256, 256), is_dot).unwrap()
        };
        assert!(near(dot_uv(false, false), [0.7, 0.62]));
        assert!(near(dot_uv(true, false), [0.3, 0.62]));
        assert!(near(dot_uv(false, true), [0.7, 0.38]));
    }

    #[test]
    fn a_quarter_turn_moves_the_top_edge_to_the_right_in_both_bakes() {
        let mesh = mesh_of(samples::flat_quad());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let src = stamp(256);
        let near = |a: Vec2, b: [f32; 2]| (a - Vec2::from(b)).length() < 0.025;

        // Unrotated: the arrow is above the centre and points up.
        let upright = front_projector(0.5, 0.5, 0.0, 0.5, 0.5);
        let baked = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &upright, BakeQuality::Full), 0, 256);
        let arrow = centroid(&baked, is_arrow).unwrap();
        assert!(arrow.y < 0.48 && (arrow.x - 0.5).abs() < 0.02, "{arrow}");

        // Projected, +90 degrees: the arrow (image top) is now right of the centre, the dot at the lower left.
        let turned = upright.rotated(FRAC_PI_2);
        let baked = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &turned, BakeQuality::Full), 0, 256);
        let projected_arrow = centroid(&baked, is_arrow).unwrap();
        assert!(projected_arrow.x > 0.52 && (projected_arrow.y - 0.5).abs() < 0.02, "{projected_arrow}");
        let projected_dot = centroid(&baked, is_dot).unwrap();
        assert!(near(projected_dot, [0.38, 0.7]), "{projected_dot}");

        // UV placement, +90 degrees: identical sense.
        let mut placement = uv_placement([128.0, 128.0], [256.0, 256.0]);
        placement.rotation = FRAC_PI_2;
        let baked = bake_uv(&src, WHITE, &placement, &maps[0]).to_dense(256, 256);
        let uv_arrow = centroid(&baked, is_arrow).unwrap();
        let uv_dot = centroid(&baked, is_dot).unwrap();
        assert!(uv_arrow.x > 0.52 && (uv_arrow.y - 0.5).abs() < 0.02, "{uv_arrow}");
        assert!(near(uv_dot, [0.38, 0.7]), "{uv_dot}");
        assert!((uv_arrow - projected_arrow).length() < 0.01 && (uv_dot - projected_dot).length() < 0.01);
    }

    #[test]
    fn projection_across_a_cube_edge_covers_two_islands_and_no_back_face() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let normal = Vec3::new(1.0, 0.0, 1.0).normalize();
        let p = Projector::from_surface(Vec3::new(0.5, 0.0, 0.5), normal, Vec3::Y, Vec2::splat(0.3), 0.5);
        let result = bake_projected(&mesh, &bvh, &maps, &solid([255, 0, 0, 255]), WHITE, &p, BakeQuality::Full);
        assert_eq!(result.len(), 1);
        let baked = dense(&result, 0, 256);
        // Faces in atlas order: +X, -X, +Y, -Y, +Z, -Z. Island rects grown by the gutter (2 texels at 256 px).
        let island = |face: u32| -> u32 {
            let [u0, u1, v0, v1] = samples::seam_cube_island(face);
            let g = 3.0 / 256.0;
            coverage_in(&baked, [u0 - g, u1 + g, v0 - g, v1 + g])
        };
        let counts: Vec<u32> = (0..6).map(island).collect();
        assert!(counts[0] > 1000 && counts[4] > 1000, "{counts:?}");
        assert_eq!([counts[1], counts[2], counts[3], counts[5]], [0; 4], "{counts:?}");
        // Both halves are equal by symmetry (up to rasterization at the island borders).
        assert!((counts[0] as i64 - counts[4] as i64).abs() < 80, "{counts:?}");
        // The decal continues into the gutter of the covered islands: texels just outside the island edge that
        // borders the cube edge are filled.
        let [u0, _, v0, v1] = samples::seam_cube_island(4);
        let (x_edge, y_mid) = ((u0 * 256.0).floor() as u32, ((v0 + v1) * 128.0) as u32);
        let gutter_hits = (1..=2).filter(|d| baked.get(x_edge - d, y_mid)[3] > 0).count()
            + (1..=2).filter(|d| baked.get(((samples::seam_cube_island(4)[1]) * 256.0).ceil() as u32 + d - 1, y_mid)[3] > 0).count();
        assert!(gutter_hits >= 2, "seam padding is baked ({gutter_hits})");

        // Without culling the result is the same here: the back faces are out of reach and occluded.
        let mut uncull = p;
        uncull.backface_cull = false;
        let unculled = dense(&bake_projected(&mesh, &bvh, &maps, &solid([255, 0, 0, 255]), WHITE, &uncull, BakeQuality::Full), 0, 256);
        assert_eq!(unculled, baked);
    }

    #[test]
    fn project_through_covers_the_near_and_the_far_side() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let src = solid([0, 0, 255, 255]);
        let mut p = front_projector(0.0, 0.0, 0.5, 0.3, 0.5);
        p.depth_back = 2.0;
        let islands = |p: &Projector, quality: BakeQuality| -> Vec<u32> {
            let baked = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, p, quality), 0, 256);
            (0..6).map(|face| coverage_in(&baked, samples::seam_cube_island(face))).collect()
        };
        // Defaults: only the near (+Z) face.
        let near_only = islands(&p, BakeQuality::Full);
        assert!(near_only[4] > 1500, "{near_only:?}");
        assert_eq!(near_only.iter().sum::<u32>(), near_only[4], "{near_only:?}");

        // Project-through: near and far face, nothing on the four side faces.
        let mut through = p;
        through.backface_cull = false;
        through.occlusion = false;
        let both = islands(&through, BakeQuality::Full);
        assert!(both[4] > 1500 && both[5] > 1500, "{both:?}");
        assert_eq!([both[0], both[1], both[2], both[3]], [0; 4], "{both:?}");
        assert!((both[4] as i64 - both[5] as i64).abs() < 40);

        // Culling off but occlusion on: the far face is hidden behind the near one...
        let mut hidden = through;
        hidden.occlusion = true;
        let occluded = islands(&hidden, BakeQuality::Full);
        assert!(occluded[4] > 1500);
        assert_eq!(occluded[5], 0, "{occluded:?}");
        // ...except in Preview quality, which casts no rays.
        assert!(islands(&hidden, BakeQuality::Preview)[5] > 1500);
        // A shallow box does not reach the far side.
        let mut shallow = through;
        shallow.depth_back = 0.5;
        assert_eq!(islands(&shallow, BakeQuality::Full)[5], 0);
    }

    #[test]
    fn reversed_winding_needs_culling_off() {
        let mut raw = samples::flat_quad();
        samples::reverse_winding(&mut raw);
        let mesh = mesh_of(raw);
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 128);
        let src = solid([255, 255, 0, 255]);
        let p = front_projector(0.5, 0.5, 0.0, 0.25, 0.5);
        for quality in [BakeQuality::Full, BakeQuality::Preview] {
            assert!(bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, quality).is_empty());
            let mut uncull = p;
            uncull.backface_cull = false;
            let result = bake_projected(&mesh, &bvh, &maps, &src, WHITE, &uncull, quality);
            assert_eq!(result.len(), 1, "{quality:?}");
            // 64 x 64 texels at full weight. The 16 px source is magnified 4x: its bilinear edge (transparent
            // outside the image) fades over half a source texel = 2 texels on each side.
            let joined = result[0].1.to_cropped();
            let pixels = joined.data.as_chunks::<4>().0;
            let opaque = pixels.iter().filter(|p| p[3] == 255).count();
            let covered = pixels.iter().filter(|p| p[3] > 0).count();
            assert!(opaque >= 58 * 58 && covered >= 64 * 64, "{opaque} {covered}");
        }
    }

    #[test]
    fn occlusion_keeps_the_decal_off_hidden_surfaces() {
        let mesh = mesh_of(samples::occluded_quads());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let src = solid([0, 255, 0, 255]);
        const FRONT: [f32; 4] = [0.04, 0.46, 0.04, 0.46];
        const REAR: [f32; 4] = [0.54, 0.96, 0.54, 0.96];
        // The box reaches the rear quad (z = -0.5), which lies completely behind the front quad here.
        let p = front_projector(0.5, 0.5, 0.0, 0.4, 1.0);
        let counts = |p: &Projector, quality: BakeQuality| -> (u32, u32) {
            let baked = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, p, quality), 0, 256);
            (coverage_in(&baked, FRONT), coverage_in(&baked, REAR))
        };
        let (front, rear) = counts(&p, BakeQuality::Full);
        assert!(front > 5000, "{front}");
        assert_eq!(rear, 0, "the rear quad is hidden");
        let (front_preview, rear_preview) = counts(&p, BakeQuality::Preview);
        assert!(rear_preview > 1000, "Preview casts no occlusion rays ({rear_preview})");
        assert!((front_preview as i64 - front as i64).abs() < 50);
        let mut open = p;
        open.occlusion = false;
        let (_, rear_open) = counts(&open, BakeQuality::Full);
        assert!(rear_open > 1000, "{rear_open}");

        // A decal that sticks out over the front quad's border reaches the rear quad only there.
        let wide = front_projector(1.0, 0.5, 0.0, 0.3, 1.0);
        let baked = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &wide, BakeQuality::Full), 0, 256);
        // Rear quad: world x in -0.5..1.5 maps to u in 0.55..0.95, so x = 1 is u = 0.85.
        let hidden_part = coverage_in(&baked, [0.72, 0.84, 0.6, 0.9]);
        let visible_part = coverage_in(&baked, [0.86, 0.9, 0.7, 0.8]);
        assert_eq!(hidden_part, 0);
        assert!(visible_part > 50, "{visible_part}");
    }

    #[test]
    fn angle_falloff_is_monotonic_on_the_sphere() {
        let mesh = mesh_of(samples::uv_sphere(48, 24, 1.0));
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let p = front_projector(0.0, 0.0, 1.0, 1.2, 1.5);
        let result = bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &p, BakeQuality::Full);
        let baked = dense(&result, 0, 256);
        let map = &maps[0];
        // Mean alpha per degree of angle between the surface normal and the projector axis.
        let mut sum = [0.0f64; 181];
        let mut count = [0u32; 181];
        for y in 0..256 {
            for x in 0..256 {
                let Some((_, false)) = map.owner(x, y) else { continue };
                let point = map.locate(&mesh, x, y).unwrap();
                let angle = point.normal.dot(Vec3::Z).clamp(-1.0, 1.0).acos().to_degrees();
                let alpha = baked.get(x, y)[3];
                if angle < 68.0 {
                    assert_eq!(alpha, 255, "full weight at {angle:.1} degrees ({x}, {y})");
                }
                if angle > 89.0 {
                    assert_eq!(alpha, 0, "no weight at {angle:.1} degrees ({x}, {y})");
                }
                sum[angle as usize] += alpha as f64;
                count[angle as usize] += 1;
            }
        }
        let mean: Vec<f64> = (66..91).filter(|&a| count[a] > 0).map(|a| sum[a] / count[a] as f64).collect();
        assert!(mean.len() > 20);
        for pair in mean.windows(2) {
            assert!(pair[1] <= pair[0] + 1.0, "falloff is not monotonic: {mean:?}");
        }
        assert!(mean[0] > 254.0 && *mean.last().unwrap() < 1.0);

        // The limits can be set: a hard cut at 30 degrees.
        let mut narrow = p;
        narrow.angle_start_deg = 30.0;
        narrow.angle_end_deg = 30.0;
        let baked = dense(&bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &narrow, BakeQuality::Preview), 0, 256);
        for y in (0..256).step_by(3) {
            for x in (0..256).step_by(3) {
                let Some((_, false)) = map.owner(x, y) else { continue };
                let angle = map.locate(&mesh, x, y).unwrap().normal.dot(Vec3::Z).clamp(-1.0, 1.0).acos().to_degrees();
                let alpha = baked.get(x, y)[3];
                if angle < 28.0 {
                    assert_eq!(alpha, 255);
                } else if angle > 32.0 {
                    assert_eq!(alpha, 0);
                }
            }
        }
    }

    #[test]
    fn depth_limits_fade_softly() {
        // The rear quad of occluded_quads is 0.5 behind the projector plane.
        let mesh = mesh_of(samples::occluded_quads());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let rear_alpha = |depth_back: f32| -> u8 {
            let mut p = front_projector(0.5, 0.5, 0.0, 0.3, 1.0);
            p.occlusion = false;
            p.depth_back = depth_back;
            let baked = dense(&bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &p, BakeQuality::Full), 0, 256);
            // Rear quad centre: u = v = 0.75.
            baked.get(192, 192)[3]
        };
        assert_eq!(rear_alpha(1.0), 255);
        assert_eq!(rear_alpha(0.49), 0, "outside the box");
        // z = -0.5 with depth_back 0.55: 0.05 inside, the fade spans the last 0.11 (smoothstep(0.45) = 0.43).
        let faded = rear_alpha(0.55);
        assert!(faded > 80 && faded < 140, "{faded}");
        assert!(rear_alpha(0.52) < faded && faded < rear_alpha(0.59));
        // The whole fade is smooth: no step between neighbouring depths.
        let alphas: Vec<i32> = (0..=20).map(|i| rear_alpha(0.5 + 0.0075 * i as f32) as i32).collect();
        for pair in alphas.windows(2) {
            assert!(pair[1] >= pair[0] && pair[1] - pair[0] <= 40, "{alphas:?}");
        }
    }

    /// The angle (degrees) between the surface normal of a texel and `out`, and the texel's point in projector space.
    fn texel_frame(mesh: &MeshData, map: &TexelMap, p: &Projector, x: u32, y: u32) -> Option<(f32, Vec3)> {
        let Some((_, false)) = map.owner(x, y) else { return None };
        let point = map.locate(mesh, x, y)?;
        let (right, up, out) = p.axes();
        let d = point.position - p.origin;
        Some((point.normal.dot(out).clamp(-1.0, 1.0).acos().to_degrees(), Vec3::new(d.dot(right), d.dot(up), d.dot(out))))
    }

    /// Curved fixtures: a unit sphere with a large decal, and the narrow tip of an ellipsoid (an avocado end: the
    /// surface falls away deeper than the decal is wide). With the depth of the old placement default (= the larger
    /// half size) part of the front-facing surface under the decal is cut off with an edge; the auto-fitted depth
    /// covers all of it: every front-facing texel in the footprint below 68 degrees gets full alpha, and nothing up
    /// to 80 degrees is cut off.
    #[test]
    fn auto_fitted_depth_covers_curved_surfaces_up_to_the_silhouette() {
        let ellipsoid = {
            let mut raw = samples::uv_sphere(96, 48, 1.0);
            let scale = Vec3::new(0.4, 0.4, 1.0);
            for t in &mut raw.triangles {
                t.positions = t.positions.map(|q| q * scale);
                t.normals = t.normals.map(|ns| ns.map(|n| (n / scale).normalize()));
            }
            raw
        };
        let cases = [("sphere", samples::uv_sphere(96, 48, 1.0), 1.0, 0.8), ("ellipsoid tip", ellipsoid, 1.0, 0.35)];
        for (name, raw, tip_z, half) in cases {
            let mesh = mesh_of(raw);
            let bvh = Bvh::build(&mesh);
            let maps = maps_of(&mesh, 512);
            let map = &maps[0];
            let legacy = Projector::from_surface(Vec3::new(0.0, 0.0, tip_z), Vec3::Z, Vec3::Y, Vec2::splat(half), half);
            assert!(projector_depth_is_auto(&mesh, &bvh, &legacy), "the old default counts as automatic");
            let fitted = fit_projector_depth(&mesh, &bvh, &legacy);
            assert!(fitted.depth_back > legacy.depth_back, "{name}: {fitted:?}");
            assert!(fitted.depth_back <= FIT_REACH * half * 1.1, "{name}: capped, {fitted:?}");
            assert!(projector_depth_is_auto(&mesh, &bvh, &fitted));
            let mut edited = fitted;
            edited.depth_back *= 1.5;
            assert!(!projector_depth_is_auto(&mesh, &bvh, &edited), "a depth typed in by hand is kept");

            let bake = |p: &Projector| dense(&bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, p, BakeQuality::Full), 0, 512);
            let (old, new) = (bake(&legacy), bake(&fitted));
            let (mut old_cut, mut checked) = (0, 0);
            for y in 0..512 {
                for x in 0..512 {
                    let Some((angle, l)) = texel_frame(&mesh, map, &fitted, x, y) else { continue };
                    // Inside the footprint, away from the image's filtered border (half a source texel of 16).
                    let inside = l.x.abs() < half * 0.9 && l.y.abs() < half * 0.9;
                    if !inside || angle >= 80.0 {
                        continue;
                    }
                    checked += 1;
                    let a = new.get(x, y)[3];
                    if angle < 68.0 {
                        assert_eq!(a, 255, "{name}: full weight at {angle:.1} degrees, depth {:.3} ({x}, {y})", l.z);
                    }
                    assert!(a > 0, "{name}: cut off at {angle:.1} degrees, depth {:.3} ({x}, {y})", l.z);
                    if old.get(x, y)[3] == 0 {
                        old_cut += 1;
                    }
                }
            }
            assert!(checked > 1000, "{name}: {checked}");
            assert!(old_cut > 20, "{name}: the old depth already covered the surface ({old_cut} texels cut)");
        }
    }

    /// accept_fixes item F: a decal placed on a body just above a sleeve that stands 0.05 in front of it (less than
    /// the half size): the surface grown from the centre follows the body hidden under the sleeve, but the visible
    /// sleeve must lie inside the box (before the fix it was beyond the front depth and cut off entirely).
    #[test]
    fn auto_fit_covers_a_visible_part_in_front_of_the_grown_surface() {
        let mesh = mesh_of(samples::sleeve_quads());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let p = front_projector(0.5, 0.6, 0.0, 0.3, 0.01);
        let fitted = fit_projector_depth(&mesh, &bvh, &p);
        assert!(fitted.depth_front >= 0.05 / (1.0 - DEPTH_FADE_FRACTION), "the sleeve lies before the fade: {fitted:?}");
        assert!(fitted.depth_front <= 0.05 / (1.0 - DEPTH_FADE_FRACTION) + FIT_MARGIN * 0.3 + 1e-4, "{fitted:?}");
        assert!((fitted.depth_back - FIT_MARGIN * 0.3).abs() < 1e-4, "nothing behind the body: {fitted:?}");
        let baked = dense(&bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &fitted, BakeQuality::Full), 0, 256);
        let opaque = |uv: [f32; 4]| {
            let mut n = 0;
            for y in (uv[2] * 256.0) as u32..(uv[3] * 256.0) as u32 {
                for x in (uv[0] * 256.0) as u32..(uv[1] * 256.0) as u32 {
                    n += (baked.get(x, y)[3] == 255) as u32;
                }
            }
            n
        };
        let (sleeve, body) = (opaque([0.55, 0.95, 0.55, 0.95]), opaque([0.05, 0.45, 0.05, 0.45]));
        // The footprint, x 0.2..0.8 and y 0.3..0.9, lies 0.6 x 0.2 on the sleeve (y up to 0.5) and 0.6 x 0.4 on the
        // visible body: about 61 x 41 texels on each quad's UV square (102 x 102 texels).
        assert!(sleeve > 600, "the visible sleeve is painted at full weight: {sleeve} texels");
        assert!(body > 1200, "the body above it too: {body} texels");
    }

    #[test]
    fn auto_fit_keeps_to_the_surface_of_the_placement_point() {
        // A decal at the right edge of the front quad that hangs over it: the rear quad shows beside it, 0.5 behind
        // (more than the half size 0.3), and is a different surface (a step at the edge, both flat): it stays out of
        // the box.
        let mesh = mesh_of(samples::occluded_quads());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let p = front_projector(0.85, 0.5, 0.0, 0.3, 0.01);
        let fitted = fit_projector_depth(&mesh, &bvh, &p);
        assert!((fitted.depth_back - FIT_MARGIN * 0.3).abs() < 1e-4, "{fitted:?}");
        assert!((fitted.depth_front - FIT_MARGIN * 0.3).abs() < 1e-4, "{fitted:?}");
        let baked = dense(&bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &fitted, BakeQuality::Full), 0, 256);
        assert!(coverage_in(&baked, [0.06, 0.44, 0.06, 0.44]) > 1500);
        assert_eq!(coverage_in(&baked, [0.55, 0.95, 0.55, 0.95]), 0, "nothing on the rear quad");
        // With a decal twice the size the rear quad lies within one half size: like a part next to it, it is reached
        // (the placement default of earlier versions reached it too).
        let p = front_projector(0.85, 0.5, 0.0, 0.6, 0.01);
        let fitted = fit_projector_depth(&mesh, &bvh, &p);
        assert!(fitted.depth_back > 0.5 && fitted.depth_back <= 0.5 / (1.0 - DEPTH_FADE_FRACTION) + FIT_MARGIN * 0.6 + 1e-4, "{fitted:?}");

        // Placed on the rear quad next to the front one, whose edge overlaps the footprint 0.5 in front of it: the
        // fit follows the rear surface (the occlusion of the bake keeps the part behind the front quad free).
        let p = front_projector(-0.1, 0.5, -0.5, 0.2, 0.01);
        let fitted = fit_projector_depth(&mesh, &bvh, &p);
        assert!(fitted.depth_front < 0.1 && fitted.depth_back < 0.1, "{fitted:?}");

        // Nothing under the centre, a degenerate size or a non-finite projector: unchanged.
        let miss = front_projector(5.0, 5.0, 0.0, 0.2, 0.33);
        assert_eq!(fit_projector_depth(&mesh, &bvh, &miss), miss);
        let flat = front_projector(0.5, 0.5, 0.0, 0.0, 0.33);
        assert_eq!(fit_projector_depth(&mesh, &bvh, &flat), flat);
        let mut nan = front_projector(0.5, 0.5, 0.0, 0.2, 0.33);
        nan.origin.x = f32::NAN;
        assert!(fit_projector_depth(&mesh, &bvh, &nan).depth_back == 0.33);
    }

    #[test]
    fn bake_uv_places_scales_and_crops() {
        let mesh = mesh_of(samples::flat_quad());
        let map = TexelMap::build(&mesh, TextureSetId(0), 256, 256, 2);
        let image = samples::sample_stamp_image(128);
        let src = PremulMips::build(&image);

        // Identity scale at an integer position: the image itself.
        let placed = bake_uv(&src, WHITE, &uv_placement([100.0, 90.0], [128.0, 128.0]), &map);
        let mut reference = PixelBuf::new(256, 256);
        for y in 0..128 {
            for x in 0..128 {
                let p = image.get(x, y);
                if p[3] > 0 {
                    reference.set(36 + x, 26 + y, p);
                }
            }
        }
        let quality_db = psnr(&placed.to_dense(256, 256), &reference);
        assert!(quality_db > 45.0, "{quality_db}");
        // Cropped rect is tight.
        assert_eq!(content_bounds(placed.to_cropped().view()), Some(placed.bounds()));
        assert!(placed.bounds().x0 >= 36 && placed.bounds().x1 <= 164 && placed.bounds().y0 >= 26 && placed.bounds().y1 <= 154);

        // Half size: compare with a direct resize.
        let half = bake_uv(&src, WHITE, &uv_placement([64.0, 64.0], [64.0, 64.0]), &map);
        let mut reference = PixelBuf::new(256, 256);
        let small = resize(&image, 64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let p = small.get(x, y);
                if p[3] > 0 {
                    reference.set(32 + x, 32 + y, p);
                }
            }
        }
        let quality_db = psnr(&half.to_dense(256, 256), &reference);
        assert!(quality_db > 40.0, "{quality_db}");

        // No wrapping: a decal over the border is cut, one outside bakes nothing. (A source of the placement's
        // resolution: its filter border, half a source texel, is then half a target texel.)
        let solid_src = PremulMips::build(&PixelBuf::filled(100, 40, [200, 100, 50, 255]));
        let cut = bake_uv(&solid_src, WHITE, &uv_placement([0.0, 128.0], [100.0, 40.0]), &map);
        assert_eq!(cut.bounds().x0, 0);
        assert!(cut.bounds().x1 >= 50 && cut.bounds().x1 <= 52, "{:?}", cut.bounds());
        assert_eq!(cut.get(10, 128), [200, 100, 50, 255]);
        assert!(bake_uv(&solid_src, WHITE, &uv_placement([-500.0, 128.0], [100.0, 40.0]), &map).is_empty());
        assert!(bake_uv(&solid_src, WHITE, &uv_placement([128.0, 9000.0], [100.0, 40.0]), &map).is_empty());

        // Tint, and inputs that bake nothing.
        let tinted = bake_uv(&solid_src, [255, 255, 0, 128], &uv_placement([128.0, 128.0], [40.0, 40.0]), &map);
        assert_eq!(tinted.get(128, 128), [200, 100, 0, 128]);
        assert!(bake_uv(&solid_src, [255, 255, 255, 0], &uv_placement([128.0, 128.0], [40.0, 40.0]), &map).is_empty());
        assert!(bake_uv(&solid_src, WHITE, &uv_placement([128.0, 128.0], [0.0, 40.0]), &map).is_empty());
        assert!(bake_uv(&solid_src, WHITE, &uv_placement([f32::NAN, 128.0], [40.0, 40.0]), &map).is_empty());
        let mut spinning = uv_placement([128.0, 128.0], [40.0, 40.0]);
        spinning.rotation = f32::INFINITY;
        assert!(bake_uv(&solid_src, WHITE, &spinning, &map).is_empty());
        assert!(bake_uv(&PremulMips::build(&PixelBuf::new(0, 0)), WHITE, &uv_placement([128.0, 128.0], [40.0, 40.0]), &map).is_empty());
    }

    #[test]
    fn bake_uv_rotation_covers_the_rotated_rectangle() {
        let mesh = mesh_of(samples::flat_quad());
        let map = TexelMap::build(&mesh, TextureSetId(0), 256, 256, 2);
        // A source of the placement's resolution, so the filter border is half a texel wide.
        let src = PremulMips::build(&PixelBuf::filled(120, 40, [255, 0, 255, 255]));
        let mut p = uv_placement([128.0, 128.0], [120.0, 40.0]);
        // Unrotated: wide. Rotated by 90 degrees: tall.
        let wide = bake_uv(&src, WHITE, &p, &map);
        assert!(wide.bounds().width() >= 120 && wide.bounds().width() <= 122 && wide.bounds().height() >= 40 && wide.bounds().height() <= 42);
        p.rotation = FRAC_PI_2;
        let tall = bake_uv(&src, WHITE, &p, &map);
        assert!(tall.bounds().width() >= 40 && tall.bounds().width() <= 42 && tall.bounds().height() >= 120 && tall.bounds().height() <= 122);
        // 45 degrees: inside the rotated rectangle opaque, outside nothing.
        p.rotation = std::f32::consts::FRAC_PI_4;
        let diagonal = bake_uv(&src, WHITE, &p, &map);
        let (right, down) = p.frame();
        for y in (60..196).step_by(2) {
            for x in (60..196).step_by(2) {
                let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - p.center_px;
                let (lx, ly) = (d.dot(right).abs(), d.dot(down).abs());
                let alpha = diagonal.get(x, y)[3];
                if lx < 58.0 && ly < 18.0 {
                    assert_eq!(alpha, 255, "({x}, {y})");
                } else if lx > 62.0 || ly > 22.0 {
                    assert_eq!(alpha, 0, "({x}, {y})");
                }
            }
        }
        // Clockwise on the v-down texture: the image's right edge points down-right.
        assert!(right.x > 0.7 && right.y > 0.7);
    }

    #[test]
    fn bake_uv_clip_to_islands() {
        let mesh = mesh_of(samples::occluded_quads());
        let map = TexelMap::build(&mesh, TextureSetId(0), 256, 256, 2);
        let src = solid([9, 9, 9, 255]);
        // Covers the gap between the two islands (0.05..0.45 and 0.55..0.95).
        let mut p = uv_placement([128.0, 128.0], [120.0, 120.0]);
        let free = bake_uv(&src, WHITE, &p, &map);
        p.clip_to_islands = true;
        let clipped = bake_uv(&src, WHITE, &p, &map);
        let (mut on_owned, mut on_free, mut clipped_free) = (0, 0, 0);
        for y in 0..256u32 {
            for x in 0..256u32 {
                let owned = map.owner(x, y).is_some();
                let (a, b) = (free.get(x as i32, y as i32)[3] > 0, clipped.get(x as i32, y as i32)[3] > 0);
                on_free += (a && !owned) as u32;
                clipped_free += (b && !owned) as u32;
                on_owned += (b && owned) as u32;
                if owned {
                    assert_eq!(a, b, "owned texels are unaffected ({x}, {y})");
                }
            }
        }
        assert!(on_free > 1000 && on_owned > 1000, "{on_free} {on_owned}");
        assert_eq!(clipped_free, 0);
    }

    #[test]
    fn preview_and_full_agree_without_occlusion() {
        let mesh = mesh_of(samples::seam_cube(1.0));
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 256);
        let mut p = front_projector(0.1, -0.05, 0.5, 0.35, 0.5).rotated(0.4);
        p.occlusion = false;
        for src_size in [64u32, 256, 512] {
            let src = stamp(src_size);
            let full = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, BakeQuality::Full), 0, 256);
            let preview = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, BakeQuality::Preview), 0, 256);
            let quality_db = psnr(&full, &preview);
            assert!(quality_db > 30.0, "{src_size}: {quality_db:.1} dB");
            // Same footprint.
            let (a, b) = (content_bounds(full.view()).unwrap(), content_bounds(preview.view()).unwrap());
            assert!((a.x0 - b.x0).abs() <= 1 && (a.x1 - b.x1).abs() <= 1 && (a.y0 - b.y0).abs() <= 1 && (a.y1 - b.y1).abs() <= 1);
        }
        // With occlusion on nothing is hidden on a convex cube face either.
        p.occlusion = true;
        let src = stamp(256);
        let with_rays = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, BakeQuality::Full), 0, 256);
        p.occlusion = false;
        let without = dense(&bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, BakeQuality::Full), 0, 256);
        assert_eq!(with_rays, without);
    }

    #[test]
    fn two_sets_wrapping_uvs_and_degenerate_input() {
        // Two materials: one buffer per touched set, ascending.
        let mesh = mesh_of(samples::two_material_model());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 128);
        let p = front_projector(0.0, 0.5, 0.5, 0.4, 0.6);
        let result = bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &p, BakeQuality::Full);
        assert_eq!(result.iter().map(|(s, _)| s.0).collect::<Vec<_>>(), [0, 1]);
        for (_, buf) in &result {
            assert_eq!(content_bounds(buf.to_cropped().view()), Some(buf.bounds()), "tight");
        }
        // Fewer maps than sets: the missing set is skipped.
        let partial = bake_projected(&mesh, &bvh, &maps[..1], &solid(WHITE), WHITE, &p, BakeQuality::Full);
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0], result[0]);

        // Degenerate projectors bake nothing.
        let mut nan = p;
        nan.origin.x = f32::NAN;
        let mut flat = p;
        flat.half_size.x = 0.0;
        for bad in [nan, flat] {
            assert!(bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &bad, BakeQuality::Full).is_empty());
        }
        assert!(bake_projected(&mesh, &bvh, &maps, &solid(WHITE), [255, 255, 255, 0], &p, BakeQuality::Full).is_empty());
        assert!(bake_projected(&mesh, &bvh, &[], &solid(WHITE), WHITE, &p, BakeQuality::Full).is_empty());
        // Far away from the model.
        let far = front_projector(50.0, 0.0, 0.0, 0.4, 0.6);
        assert!(bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &far, BakeQuality::Full).is_empty());

        // wrapped_quad: UVs in [0.5, 1.5]^2 are written at wrapped coordinates, in all four corners of the texture.
        let mesh = mesh_of(samples::wrapped_quad());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 128);
        let p = front_projector(0.5, 0.5, 0.0, 0.4, 0.5);
        let result = bake_projected(&mesh, &bvh, &maps, &solid(WHITE), WHITE, &p, BakeQuality::Full);
        let baked = dense(&result, 0, 128);
        // World (0.5, 0.5) -> uv (1.0, 1.0) -> wrapped texel (0, 0); the decal spans 0.4 around it.
        for (x, y) in [(2u32, 2u32), (125, 2), (2, 125), (125, 125), (40, 40), (90, 90), (40, 90)] {
            assert_eq!(baked.get(x, y)[3], 255, "({x}, {y})");
        }
        assert_eq!(baked.get(64, 64)[3], 0, "world (0, 0) is outside the decal");
    }

    /// Finding SP-2 (review round 2): a decal over neighbouring quads whose UV islands lie far apart in the atlas is
    /// stored as small parts, not as one dense rectangle over all of them; the pixels are the same.
    #[test]
    fn a_decal_on_a_fragmented_atlas_is_stored_sparsely() {
        let mesh = mesh_of(samples::scattered_islands());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 1024);
        // Around the centre of the square: the four inner quads, whose islands are scattered.
        let p = front_projector(0.5, 0.5, 0.0, 0.2, 0.5);
        let result = bake_projected(&mesh, &bvh, &maps, &solid([200, 30, 30, 255]), WHITE, &p, BakeQuality::Full);
        assert_eq!(result.len(), 1);
        let cache = &result[0].1;
        let bounds = cache.bounds();
        assert!(cache.parts().len() >= 4, "{:?}", cache.rects().collect::<Vec<_>>());
        // The bounding box spans most of the atlas; the parts hold a small fraction of it.
        assert!(bounds.area() > 1024 * 1024 / 4, "{bounds:?}");
        assert!(cache.byte_len() * 10 < bounds.area() as usize * 4, "{} bytes for {bounds:?}", cache.byte_len());
        // Every part is disjoint, and joining them gives the pixels a dense crop would have held.
        let rects: Vec<IRect> = cache.rects().collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(rects[i + 1..].iter().all(|b| a.intersect(b).is_empty()));
        }
        let dense_image = cache.to_dense(1024, 1024);
        assert_eq!(crate::pixel_ops::crop(&dense_image, bounds), cache.to_cropped());
        assert_eq!(content_bounds(dense_image.view()), Some(bounds));
        // The decal is on all four inner islands.
        assert_eq!(cache.parts().len(), 4, "one part per island: {:?}", cache.rects().collect::<Vec<_>>());
        for slot in [(6u32, 0u32), (2, 3), (0, 7), (5, 1)] {
            let (x, y) = (slot.0 * 128, slot.1 * 128);
            let covered = (y..y + 128).any(|yy| (x..x + 128).any(|xx| dense_image.get(xx, yy)[3] == 255));
            assert!(covered, "island at slot {slot:?}");
        }
    }

    /// A sphere whose UVs repeat 3 x 3 times and lie outside 0..1 (u in -1.3..1.7, v in -0.7..2.3), plus a disc in
    /// front of part of it (it occludes the sphere for a projector looking down -Z) whose triangles each span about four
    /// tiles, like the n-gons of ngons_tiling.obj.
    fn tiled_mesh() -> crate::mesh::RawMesh {
        let mut raw = samples::uv_sphere(32, 16, 1.0);
        for t in &mut raw.triangles {
            if let Some(uv) = &mut t.uvs {
                *uv = uv.map(|c| c * 3.0 - Vec2::new(1.3, 0.7));
            }
        }
        let (c, r, n) = (Vec3::new(0.35, 0.25, 1.15), 0.35f32, 9);
        let uv_of = |p: Vec3| Vec2::new(p.x - c.x, p.y - c.y) * (2.0 / r) + Vec2::new(2.0, 2.0);
        for k in 0..n {
            let a0 = std::f32::consts::TAU * k as f32 / n as f32;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / n as f32;
            let p1 = c + Vec3::new(a0.cos(), a0.sin(), 0.0) * r;
            let p2 = c + Vec3::new(a1.cos(), a1.sin(), 0.0) * r;
            raw.triangles.push(crate::mesh::RawTriangle {
                positions: [c, p1, p2],
                normals: None,
                uvs: Some([uv_of(c), uv_of(p1), uv_of(p2)]),
                material: 0,
            });
        }
        raw
    }

    #[test]
    fn tiled_uv_bakes_equal_the_plain_reference() {
        let mesh = mesh_of(tiled_mesh());
        let bvh = Bvh::build(&mesh);
        let maps = maps_of(&mesh, 128);
        let src = stamp(64);
        let mut checked = 0;
        // The release rule (this small bake builds no grid), the coarse grid alone, and the cover and floor grids.
        for fine in [None, Some(false), Some(true)] {
            let _force = fine.map(probe::ForceFine::new);
            for (cx, cy, half) in [(0.2f32, 0.1f32, 0.6f32), (-0.3, 0.4, 0.9), (0.4, 0.3, 0.25), (0.0, 0.0, 1.2)] {
                for (cull, occlusion) in [(true, true), (false, true), (true, false)] {
                    for tint in [[255u8; 4], [200, 90, 30, 128], [10, 255, 255, 3]] {
                        for quality in [BakeQuality::Full, BakeQuality::Preview] {
                            let mut p = front_projector(cx, cy, 1.2, half, 1.5);
                            p.backface_cull = cull;
                            p.occlusion = occlusion;
                            let p = p.rotated(0.3);
                            let result = bake_projected(&mesh, &bvh, &maps, &src, tint, &p, quality);
                            let reference = reference_bake(&mesh, &bvh, &maps, &src, tint, &p, quality);
                            let baked = dense(&result, 0, 128);
                            assert!(baked.data == reference[0].data, "{fine:?} {cx} {cy} {half} cull {cull} occlusion {occlusion} {tint:?} {quality:?}");
                            if baked.data.as_chunks::<4>().0.iter().any(|px| px[3] > 0) {
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(checked >= 180, "only {checked} bakes covered anything");
    }

    /// A small Full bake builds no occluder grid (the BVH is faster there), a large one does; the cover grid built in
    /// parallel chunks equals the one built in a single chunk.
    #[test]
    fn the_occluder_grid_is_built_only_for_large_bakes_and_deterministically() {
        let mesh = mesh_of(tiled_mesh());
        let bvh = Bvh::build(&mesh);
        let src = stamp(32);
        let small = front_projector(0.2, 0.1, 1.2, 0.05, 1.5);
        let (ctx, _) = projected_jobs(&mesh, &bvh, &maps_of(&mesh, 64), &src, WHITE, &small, BakeQuality::Full).unwrap();
        assert!(ctx.occluders.is_none(), "a small bake builds no grid");
        let maps = maps_of(&mesh, 1024);
        let large = front_projector(0.0, 0.0, 1.2, 1.2, 1.5).rotated(0.3);
        let (ctx, _) = projected_jobs(&mesh, &bvh, &maps, &src, WHITE, &large, BakeQuality::Full).unwrap();
        let grid = ctx.occluders.expect("a large bake builds the grid");
        assert!(!grid.cover.is_empty() && grid.cover.iter().any(|&c| c != u32::MAX), "with the cover grid");
        assert!(grid.tris.len() > 512, "several chunks: {}", grid.tris.len());
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let (ctx1, _) = one.install(|| projected_jobs(&mesh, &bvh, &maps, &src, WHITE, &large, BakeQuality::Full)).unwrap();
        let grid1 = ctx1.occluders.unwrap();
        assert!(grid1.cover == grid.cover && grid1.floor == grid.floor);
    }

    /// The sets of a bake are rastered in groups whose scratches stay within the budget (a wrapping footprint takes
    /// the whole texture of its set), with the same result as all at once and as the plain reference.
    #[test]
    fn sets_are_rastered_within_the_scratch_budget() {
        assert_eq!(scratch_groups(&[], 10), Vec::<std::ops::Range<usize>>::new());
        assert_eq!(scratch_groups(&[4, 4, 4], 8), [0..2, 2..3]);
        assert_eq!(scratch_groups(&[20, 4, 4], 8), [0..1, 1..3]);
        assert_eq!(scratch_groups(&[4, 20, 4], 8), [0..1, 1..2, 2..3]);
        assert_eq!(scratch_groups(&[4, 4, 4], usize::MAX), std::iter::once(0..3).collect::<Vec<_>>());

        let mut raw = tiled_mesh();
        raw.materials = (0..3)
            .map(|i| crate::mesh::RawMaterial { name: format!("M{i}"), base_color: [1.0; 4], double_sided: false, texture: None })
            .collect();
        for (i, t) in raw.triangles.iter_mut().enumerate() {
            t.material = (i % 3) as _;
        }
        let mesh = mesh_of(raw);
        assert_eq!(mesh.sets.len(), 3);
        let bvh = Bvh::build(&mesh);
        let size = 64u32;
        let maps = maps_of(&mesh, size);
        let src = stamp(32);
        let texture = (size * size * 4) as usize;
        for quality in [BakeQuality::Full, BakeQuality::Preview] {
            let p = front_projector(0.1, 0.1, 1.2, 0.9, 1.5).rotated(0.3);
            probe::PEAK_SCRATCH.with(|c| c.set(0));
            let all = bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, quality);
            let all_peak = probe::PEAK_SCRATCH.with(|c| c.get());
            assert_eq!(all.len(), 3, "{quality:?}");
            assert!(all_peak > texture, "{quality:?}: the sets were rastered at once ({all_peak} bytes)");

            probe::SCRATCH_BUDGET.with(|b| b.set(Some(texture)));
            probe::PEAK_SCRATCH.with(|c| c.set(0));
            let grouped = bake_projected(&mesh, &bvh, &maps, &src, WHITE, &p, quality);
            let peak = probe::PEAK_SCRATCH.with(|c| c.get());
            probe::SCRATCH_BUDGET.with(|b| b.set(None));
            assert!(peak <= texture, "{quality:?}: {peak} bytes held at once, budget {texture}");
            assert!(grouped == all, "{quality:?}");
            let reference = reference_bake(&mesh, &bvh, &maps, &src, WHITE, &p, quality);
            for s in 0..3u32 {
                assert!(dense(&grouped, s, size).data == reference[s as usize].data, "{quality:?} set {s}");
            }
        }
    }

    /// The occlusion shortcuts (`ProjectCtx::occluded` with a hint: the last occluder, the occluder grid and
    /// `classify`) answer exactly what the BVH answers, ray by ray, on tiled, curved, stacked and seam meshes, for
    /// rays from points of the projection box (including points just outside it, as gutter texels give).
    #[test]
    fn occlusion_shortcuts_agree_with_the_bvh() {
        let _force = probe::ForceFine::new(true);
        let mut rng = crate::testutil::XorShift::new(7);
        let mut decided = 0usize;
        let mut covered = 0usize;
        let mut floored = 0usize;
        let mut total = 0usize;
        for raw in [tiled_mesh(), samples::uv_sphere(24, 12, 1.0), samples::occluded_quads(), samples::seam_cube(1.5), samples::two_material_model()] {
            let mesh = mesh_of(raw);
            let bvh = Bvh::build(&mesh);
            let maps = maps_of(&mesh, 64);
            let src = solid(WHITE);
            for (cx, cy, half, rot) in [(0.2f32, 0.1f32, 0.6f32, 0.3f32), (-0.3, 0.4, 0.9, 1.1), (0.5, 0.5, 0.3, 0.0), (0.0, 0.0, 1.4, 2.0)] {
                let p = front_projector(cx, cy, 1.2, half, 2.0).rotated(rot);
                let Some((ctx, _)) = projected_jobs(&mesh, &bvh, &maps, &src, WHITE, &p, BakeQuality::Full) else { continue };
                assert!(ctx.occluders.is_some());
                let mut hint = None;
                for _ in 0..4000 {
                    // A point on a random triangle (sometimes slightly outside it), pushed off along its normal.
                    let tri = (rng.next_u32() % mesh.tri_count()) as TriId;
                    let [a, b, c] = mesh.tri_positions(tri);
                    let (mut u, mut v) = (rng.next_f32(), rng.next_f32());
                    if u + v > 1.0 {
                        (u, v) = (1.0 - u, 1.0 - v);
                    }
                    let (u, v) = if rng.next_u32().is_multiple_of(8) { (u * 1.2 - 0.1, v * 1.2 - 0.1) } else { (u, v) };
                    let point = a + (b - a) * u + (c - a) * v;
                    let geo = mesh.geo_normal(tri);
                    let bias = if geo.dot(ctx.out) < 0.0 { -geo } else { geo } * (1e-4 * mesh.diag());
                    let start = point + bias;
                    let t_max = ctx.front - (point - ctx.origin).dot(ctx.out);
                    if t_max.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                        continue;
                    }
                    let ray = Ray { origin: start, dir: ctx.out };
                    let expected = bvh.occluded(&mesh, &ray, 0.0, t_max);
                    assert_eq!(ctx.occluded(&ray, t_max, Some(&mut hint)), expected, "tri {tri} at {point}");
                    if ctx.occluders.as_ref().is_some_and(|g| g.covered(&ray, t_max)) {
                        assert!(expected, "proven covered, tri {tri} at {point}");
                        covered += 1;
                    }
                    // The floor test as `fragment` makes it, for a fragment whose ray starts at `point`.
                    if ctx.occluders.as_ref().is_some_and(|g| g.below_floor(ctx.local(point))) {
                        assert!(expected, "below the floor, tri {tri} at {point}");
                        floored += 1;
                    }
                    match ctx.occluders.as_ref().map(|g| g.occluder(&ray, t_max, None)) {
                        Some(Proof::Hit(_)) => {
                            assert!(expected, "proven hit, tri {tri} at {point}");
                            decided += 1;
                        }
                        Some(Proof::Clear) => {
                            assert!(!expected, "proven clear, tri {tri} at {point}");
                            decided += 1;
                        }
                        _ => {}
                    }
                    total += 1;
                }
            }
        }
        assert!(total > 30_000, "{total} rays");
        // The grid decides many of these rays by itself (many start outside the box, where it does not try).
        assert!(decided * 3 > total, "{decided} of {total} rays decided without the BVH");
        assert!(covered * 20 > total, "{covered} of {total} rays proven by the cover grid");
        assert!(floored * 20 > total, "{floored} of {total} rays proven by the floor grid");
    }
}
