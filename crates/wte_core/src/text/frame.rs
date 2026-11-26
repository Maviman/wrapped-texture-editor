// Owner: T2 text painters (round 4). The anchor rule (FEATURE_TEXT 5.11, pure part): where a text's picture sits on
// its placement when the picture changes, and the mapping between em of text space and the host space of a
// placement.
//
// Signatures by the scaffold S1, bodies by T2. `refit_placement` applies the whole rule of 5.11: the host size of one
// em is kept and the point named by `keep` (`Keep::Anchor` or `Keep::AlignEdge`) stays where it was on the model; for
// a refit between two frames of the same em whose kept points are both picture centres it is round 3's text rescale
// bit for bit (the centre is not touched). `place_anchor_at_centre` moves a placement so that the picture's anchor
// lies where its centre was. `EmMap` maps between em of text space and the host space of a placement.

use glam::{Vec2, Vec3};

use crate::stamp::Placement;
use crate::text::TextFrame;

/// Which point of the picture keeps its place on the model when the frame changes.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Keep {
    /// The anchor (`TextFrame::anchor_px`): every change except a change of spacing alone.
    Anchor,
    /// The alignment side of the line box (`TextFrame::align_px`): a change of spacing alone, without warp or path.
    AlignEdge,
}

/// The rule of FEATURE_TEXT 5.11: the placement of a picture with frame `old` is changed for a picture with frame
/// `new` so that the host size of one em (per axis) and the host position of the kept point stay.
///
/// With `k = host size / picture size` per axis: `k1 = k0 * S0 / S1`, `(W1, H1) = (w1, h1) * k1` and
/// `c1 = c0 + ex * ((a0.x - w0/2) k0.x - (a1.x - w1/2) k1.x) + ey * ((a0.y - h0/2) k0.y - (a1.y - h1/2) k1.y)`, where
/// `a` is the kept point (`anchor_px` for `Keep::Anchor`, `align_px` for `Keep::AlignEdge`). With equal pixels per
/// em the size is round 3's rescale `w1 / w0`, `h1 / h0` bit for bit; when both kept points lie exactly in the
/// picture centres the centre is not touched. A degenerate frame (zero size, non-finite or zero em) changes nothing.
pub fn refit_placement(placement: &mut Placement, old: &TextFrame, new: &TextFrame, keep: Keep) {
    let [w0, h0] = old.size.map(|v| v as f32);
    let [w1, h1] = new.size.map(|v| v as f32);
    // With the same pixels per em this must be exactly round 3's `w1 / w0` (doc_stamps.rs `set_stamp_source`): a
    // multiply and divide by the same em size is not bit-exact.
    let same_em = old.em_px == new.em_px;
    let em_ratio = old.em_px / new.em_px;
    let f = if same_em {
        Vec2::new(w1 / w0, h1 / h0)
    } else {
        Vec2::new(w1 / w0 * em_ratio, h1 / h0 * em_ratio)
    };
    if !(f.is_finite() && f.x > 0.0 && f.y > 0.0) {
        return;
    }
    let (a0, a1) = match keep {
        Keep::Anchor => (old.anchor_px, new.anchor_px),
        Keep::AlignEdge => (old.align_px, new.align_px),
    };
    let d0 = Vec2::from_array(a0) - Vec2::new(w0, h0) / 2.0;
    let d1 = Vec2::from_array(a1) - Vec2::new(w1, h1) / 2.0;
    let frame = HostFrame::of(placement);
    let k0 = frame.size / Vec2::new(w0, h0);
    let k1 = if same_em { k0 } else { k0 * em_ratio };
    let moved = d0 != Vec2::ZERO || d1 != Vec2::ZERO;
    let shift = d0 * k0 - d1 * k1;
    let shift_ok = shift.is_finite();
    match placement {
        Placement::Projected(p) => {
            p.half_size *= f;
            if moved && shift_ok {
                p.origin += frame.ex * shift.x + frame.ey * shift.y;
            }
        }
        Placement::Uv(p) => {
            p.size_px *= f;
            if moved && shift_ok {
                p.center_px += (frame.ex * shift.x + frame.ey * shift.y).truncate();
            }
        }
    }
}

/// Moves the rectangle so that the anchor lies where the centre of the rectangle was (the Text tool places the
/// anchor at the click). Nothing changes when the anchor is exactly the picture centre (a classic picture).
pub fn place_anchor_at_centre(placement: &mut Placement, frame: &TextFrame) {
    let size = Vec2::new(frame.size[0] as f32, frame.size[1] as f32);
    let d = Vec2::from_array(frame.anchor_px) - size / 2.0;
    if d == Vec2::ZERO {
        return;
    }
    let host = HostFrame::of(placement);
    let k = host.size / size;
    let shift = host.ex * (d.x * k.x) + host.ey * (d.y * k.y);
    if !shift.is_finite() {
        return;
    }
    match placement {
        Placement::Projected(p) => p.origin -= shift,
        Placement::Uv(p) => p.center_px -= shift.truncate(),
    }
}

/// The frame of a placement in its host space (FEATURE_TEXT 5.11): unit directions of the picture's x and y axes and
/// the full size of the rectangle.
pub(crate) struct HostFrame {
    pub(crate) ex: Vec3,
    pub(crate) ey: Vec3,
    pub(crate) size: Vec2,
}

impl HostFrame {
    pub(crate) fn of(placement: &Placement) -> HostFrame {
        match placement {
            Placement::Projected(p) => {
                let (right, up, _) = p.axes();
                let ex = if p.flip_x { -right } else { right };
                let ey = if p.flip_y { up } else { -up };
                HostFrame { ex, ey, size: 2.0 * p.half_size }
            }
            Placement::Uv(p) => {
                let (s, c) = p.rotation.sin_cos();
                let ex = Vec3::new(c, s, 0.0);
                let ey = Vec3::new(-s, c, 0.0);
                let ex = if p.flip_x { -ex } else { ex };
                let ey = if p.flip_y { -ey } else { ey };
                HostFrame { ex, ey, size: p.size_px }
            }
        }
    }
}

/// Em of text space <-> host space (world units in 3D, texels in 2D) for one placement and frame.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EmMap {
    /// Host position of the picture centre.
    center: Vec3,
    /// Host vectors of one picture pixel along x and along y.
    ex: Vec3,
    ey: Vec3,
    /// Normal of the projector plane (3D); +Z in 2D.
    normal: Vec3,
    /// Picture size, anchor and pixels per em.
    size: Vec2,
    anchor: Vec2,
    em_px: f32,
}

impl EmMap {
    /// The map of a picture with frame `frame` on `placement` (FEATURE_TEXT 5.11 "Frame of a placement").
    pub fn new(placement: &Placement, frame: &TextFrame) -> EmMap {
        let size = Vec2::new(frame.size[0] as f32, frame.size[1] as f32).max(Vec2::ONE);
        let (center, ex, ey, normal, full) = match placement {
            Placement::Projected(p) => {
                let (right, up, forward) = p.axes();
                let ex = if p.flip_x { -right } else { right };
                let ey = if p.flip_y { up } else { -up };
                (p.origin, ex, ey, forward, 2.0 * p.half_size)
            }
            Placement::Uv(p) => {
                let (s, c) = p.rotation.sin_cos();
                let ex = Vec3::new(c, s, 0.0);
                let ey = Vec3::new(-s, c, 0.0);
                let ex = if p.flip_x { -ex } else { ex };
                let ey = if p.flip_y { -ey } else { ey };
                (p.center_px.extend(0.0), ex, ey, Vec3::Z, p.size_px)
            }
        };
        let k = full / size;
        EmMap {
            center,
            ex: ex * k.x,
            ey: ey * k.y,
            normal,
            size,
            anchor: Vec2::from_array(frame.anchor_px),
            em_px: frame.em_px,
        }
    }

    /// Picture pixel of an em point of text space.
    fn pixel(&self, em: [f32; 2]) -> Vec2 {
        self.anchor + Vec2::from_array(em) * self.em_px
    }

    /// Host point of a picture pixel.
    fn host(&self, q: Vec2) -> Vec3 {
        let d = q - self.size / 2.0;
        self.center + self.ex * d.x + self.ey * d.y
    }

    /// Em point of a host point (projected onto the picture plane).
    fn em_of(&self, p: Vec3) -> [f32; 2] {
        let d = p - self.center;
        // Solve d = ex * a + ey * b in the plane (ex and ey are orthogonal).
        let a = d.dot(self.ex) / self.ex.length_squared().max(f32::MIN_POSITIVE);
        let b = d.dot(self.ey) / self.ey.length_squared().max(f32::MIN_POSITIVE);
        let q = Vec2::new(a, b) + self.size / 2.0;
        ((q - self.anchor) / self.em_px).to_array()
    }

    /// World point of an em point (3D placement).
    pub fn to_world(&self, em: [f32; 2]) -> Vec3 {
        self.host(self.pixel(em))
    }

    /// Texel of an em point (UV placement).
    pub fn to_texel(&self, em: [f32; 2]) -> Vec2 {
        self.host(self.pixel(em)).truncate()
    }

    /// A point and the normal of the projector plane.
    pub fn plane(&self) -> (Vec3, Vec3) {
        (self.center, self.normal)
    }

    /// Em point of a world point (projected along the plane normal).
    pub fn from_world(&self, p: Vec3) -> [f32; 2] {
        self.em_of(p)
    }

    /// Em point of a texel.
    pub fn from_texel(&self, px: Vec2) -> [f32; 2] {
        self.em_of(px.extend(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TextureSetId;
    use crate::stamp::{Projector, UvPlacement};

    fn frame(size: [u32; 2], anchor: [f32; 2], em_px: f32) -> TextFrame {
        TextFrame {
            size,
            anchor_px: anchor,
            em_px,
            box_px: [size[0] as f32 - 8.0, size[1] as f32 - 8.0],
            baseline0_px: 0.0,
            align_px: anchor,
            circle_fill: 0.0,
        }
    }

    #[test]
    fn em_map_round_trips() {
        let f = frame([300, 120], [140.0, 70.0], 64.0);
        let projected = Placement::Projected(
            Projector::from_surface(Vec3::new(0.2, 0.3, 0.1), Vec3::new(0.3, 0.2, 1.0).normalize(), Vec3::Y, Vec2::new(0.4, 0.16), 0.2)
                .rotated(0.5),
        );
        let uv = Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(100.0, 80.0),
            size_px: Vec2::new(150.0, 60.0),
            rotation: std::f32::consts::FRAC_PI_6,
            flip_x: true,
            flip_y: false,
            clip_to_islands: false,
        });
        for placement in [projected, uv] {
            let map = EmMap::new(&placement, &f);
            for em in [[0.0, 0.0], [1.5, -0.25], [-2.0, 0.7]] {
                let back = match placement {
                    Placement::Projected(_) => map.from_world(map.to_world(em)),
                    Placement::Uv(_) => map.from_texel(map.to_texel(em)),
                };
                assert!((back[0] - em[0]).abs() < 1e-3 && (back[1] - em[1]).abs() < 1e-3, "{em:?} -> {back:?}");
            }
        }
        // The anchor of a centred frame is the placement centre.
        let centred = frame([300, 120], [150.0, 60.0], 64.0);
        let map = EmMap::new(&uv, &centred);
        assert!((map.to_texel([0.0, 0.0]) - Vec2::new(100.0, 80.0)).length() < 1e-4);
    }

    #[test]
    fn refit_keeps_the_em_size() {
        let mut placement = Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(100.0, 80.0),
            size_px: Vec2::new(150.0, 60.0),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        let old = frame([300, 120], [150.0, 60.0], 64.0);
        let new = frame([600, 120], [300.0, 60.0], 64.0);
        refit_placement(&mut placement, &old, &new, Keep::Anchor);
        let Placement::Uv(p) = placement else { panic!() };
        assert_eq!(p.size_px, Vec2::new(300.0, 60.0));
        assert_eq!(p.center_px, Vec2::new(100.0, 80.0));
    }

    /// With the same em size the refit is bit for bit round 3's text rescale (`w1 / w0`, `h1 / h0`), for awkward
    /// sizes and em sizes where `w1 / w0 * em / em` would round differently.
    #[test]
    fn refit_with_the_same_em_is_the_round_3_rescale() {
        let start = Projector::from_surface(Vec3::ZERO, Vec3::Z, Vec3::Y, Vec2::new(0.37, 0.113), 0.2);
        for em in [13.7f32, 64.0, 97.3, 211.9] {
            for (w0, w1) in [(3u32, 35u32), (4, 13), (5, 7), (301, 517), (999, 1001)] {
                for (h0, h1) in [(4u32, 21u32), (77, 91)] {
                    let old = frame([w0, h0], [w0 as f32 / 2.0, h0 as f32 / 2.0], em);
                    let new = frame([w1, h1], [w1 as f32 / 2.0, h1 as f32 / 2.0], em);
                    let mut placement = Placement::Projected(start);
                    refit_placement(&mut placement, &old, &new, Keep::Anchor);
                    let Placement::Projected(p) = placement else { panic!() };
                    let round3 = start.half_size * Vec2::new(w1 as f32 / w0 as f32, h1 as f32 / h0 as f32);
                    assert_eq!(p.half_size.to_array().map(f32::to_bits), round3.to_array().map(f32::to_bits), "{em} {w0}->{w1}");
                    assert_eq!(p.origin, start.origin);
                }
            }
        }
    }

    // ---- Round 4: the anchor rule (FEATURE_TEXT 5.11, 8.1 "Frame") ----

    use crate::text::TextAlign;
    use crate::text::layout::testkit::{spec, ubuntu};
    use crate::text::raster::classic_frame;

    /// Host point of picture pixel `q` of a picture with frame `f` on `placement` (5.11 "Frame of a placement").
    fn host(placement: &Placement, f: &TextFrame, q: [f32; 2]) -> Vec3 {
        let h = HostFrame::of(placement);
        let size = Vec2::new(f.size[0] as f32, f.size[1] as f32);
        let k = h.size / size;
        let c = match placement {
            Placement::Projected(p) => p.origin,
            Placement::Uv(p) => p.center_px.extend(0.0),
        };
        let d = Vec2::from_array(q) - size / 2.0;
        c + h.ex * d.x * k.x + h.ey * d.y * k.y
    }

    fn centre(placement: &Placement) -> Vec3 {
        match placement {
            Placement::Projected(p) => p.origin,
            Placement::Uv(p) => p.center_px.extend(0.0),
        }
    }

    fn diagonal(placement: &Placement) -> f32 {
        HostFrame::of(placement).size.length()
    }

    /// Host size of one em per axis.
    fn em_size(placement: &Placement, f: &TextFrame) -> Vec2 {
        HostFrame::of(placement).size / Vec2::new(f.size[0] as f32, f.size[1] as f32) * f.em_px
    }

    /// Projected and UV placements, rotation 30 degrees, every combination of flips, and one stretched to twice its
    /// width.
    fn placements(aspect: f32) -> Vec<Placement> {
        let mut out = Vec::new();
        let r = std::f32::consts::FRAC_PI_6;
        for (flip_x, flip_y) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut p = Projector::from_surface(Vec3::new(0.3, -0.2, 0.4), Vec3::new(0.2, 0.3, 1.0).normalize(), Vec3::Y, Vec2::new(0.5 * aspect, 0.5), 0.2)
                .rotated(r);
            p.flip_x = flip_x;
            p.flip_y = flip_y;
            out.push(Placement::Projected(p));
            out.push(Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: Vec2::new(300.0, 200.0),
                size_px: Vec2::new(100.0 * aspect, 100.0),
                rotation: r,
                flip_x,
                flip_y,
                clip_to_islands: false,
            }));
        }
        let mut stretched = out[1];
        if let Placement::Uv(p) = &mut stretched {
            p.size_px.x *= 2.0;
        }
        out.push(stretched);
        out
    }

    #[test]
    fn refit_keeps_the_alignment_edge() {
        let font = ubuntu(128.0);
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            let mut a = spec("WTE 2026");
            a.align = align;
            let mut b = a.clone();
            b.fx.gaps = vec![crate::text::Gap { after: 2, em: 0.5 }];
            let (f0, f1) = (classic_frame(&[&font], &a).unwrap(), classic_frame(&[&font], &b).unwrap());
            assert_eq!(f1.size[0], f0.size[0] + 64, "the second frame is 64 px wider");
            for start in placements(f0.size[0] as f32 / f0.size[1] as f32) {
                let mut p = start;
                refit_placement(&mut p, &f0, &f1, Keep::AlignEdge);
                let before = host(&start, &f0, f0.align_px);
                let after = host(&p, &f1, f1.align_px);
                let tol = 1e-4 * diagonal(&start);
                assert!((after - before).length() < tol, "{align:?} {start:?}: {before} -> {after}");
                let ex = HostFrame::of(&start).ex;
                let k = HostFrame::of(&start).size.x / f0.size[0] as f32;
                let moved = centre(&p) - centre(&start);
                let want = match align {
                    TextAlign::Left => 32.0 * k,
                    TextAlign::Center => 0.0,
                    TextAlign::Right => -32.0 * k,
                };
                assert!((moved - ex * want).length() < tol, "{align:?}: the centre moved by {moved}, expected {}", ex * want);
                if align == TextAlign::Center {
                    assert_eq!(centre(&p).to_array().map(f32::to_bits), centre(&start).to_array().map(f32::to_bits));
                }
                // One em keeps its host size.
                let (e0, e1) = (em_size(&start, &f0), em_size(&p, &f1));
                assert!(((e1 - e0) / e0).abs().max_element() < 1e-5, "{e0} {e1}");
            }
        }
    }

    /// A deterministic generator for the frame pairs.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f64 / (1u64 << 31) as f64) as f32
        }
    }

    #[test]
    fn refit_keeps_anchor_and_em() {
        let mut rng = Lcg(42);
        let mut worst = (0.0f32, 0.0f32);
        for _ in 0..100 {
            let make = |rng: &mut Lcg, em: f32| -> TextFrame {
                let size = [(50.0 + rng.next() * 900.0) as u32, (30.0 + rng.next() * 400.0) as u32];
                let anchor = [rng.next() * size[0] as f32, rng.next() * size[1] as f32];
                frame(size, anchor, em)
            };
            let (e0, e1) = (16.0 + rng.next() * 496.0, 16.0 + rng.next() * 496.0);
            let (f0, f1) = (make(&mut rng, e0), make(&mut rng, e1));
            for start in placements(1.0 + rng.next() * 3.0) {
                let mut p = start;
                refit_placement(&mut p, &f0, &f1, Keep::Anchor);
                let moved = (host(&p, &f1, f1.anchor_px) - host(&start, &f0, f0.anchor_px)).length() / diagonal(&start);
                let em = ((em_size(&p, &f1) - em_size(&start, &f0)) / em_size(&start, &f0)).abs().max_element();
                worst = (worst.0.max(moved), worst.1.max(em));
                assert!(moved < 1e-4, "{moved}");
                assert!(em < 1e-6, "{em}");
            }
        }
        eprintln!("worst anchor drift {} of the diagonal, worst em change {}", worst.0, worst.1);
    }

    #[test]
    fn centred_frames_leave_the_centre_bits() {
        for start in placements(2.5) {
            for (s0, s1) in [(128.0, 128.0), (128.0, 256.0), (97.3, 13.7)] {
                let f0 = frame([301, 77], [150.5, 38.5], s0);
                let f1 = frame([517, 91], [258.5, 45.5], s1);
                for keep in [Keep::Anchor, Keep::AlignEdge] {
                    let mut p = start;
                    refit_placement(&mut p, &f0, &f1, keep);
                    assert_eq!(centre(&p).to_array().map(f32::to_bits), centre(&start).to_array().map(f32::to_bits));
                }
            }
        }
    }

    #[test]
    fn repeated_refits_do_not_drift() {
        let mut rng = Lcg(5);
        // A text of 6.3 x 1.4 em with its anchor off centre, at 20 resolutions between 128 and 512 px per em.
        let at = |em: f32| -> TextFrame {
            let size = [(6.3 * em).round() as u32 + 7, (1.4 * em).round() as u32 + 7];
            frame(size, [2.1 * em + 3.5, 0.9 * em + 3.5], em)
        };
        for start in placements(4.0) {
            let mut p = start;
            let first = at(256.0);
            let mut f = first;
            for _ in 0..20 {
                let next = at(128.0 + (rng.next() * 384.0).round());
                refit_placement(&mut p, &f, &next, Keep::Anchor);
                f = next;
            }
            let drift = (host(&p, &f, f.anchor_px) - host(&start, &first, first.anchor_px)).length();
            match start {
                Placement::Uv(_) => assert!(drift < 0.01, "{drift} texel"),
                Placement::Projected(_) => assert!(drift < 1e-5, "{drift} of a unit model"),
            }
        }
    }

    #[test]
    fn the_anchor_goes_where_the_centre_was() {
        let f = frame([300, 120], [80.0, 90.0], 64.0);
        for start in placements(2.5) {
            let mut p = start;
            place_anchor_at_centre(&mut p, &f);
            let anchor = EmMap::new(&p, &f);
            let got = match p {
                Placement::Projected(_) => anchor.to_world([0.0, 0.0]),
                Placement::Uv(_) => anchor.to_texel([0.0, 0.0]).extend(0.0),
            };
            assert!((got - centre(&start)).length() < 1e-4 * diagonal(&start), "{got} vs {}", centre(&start));
        }
        // A centred anchor changes nothing.
        let centred = frame([300, 120], [150.0, 60.0], 64.0);
        for start in placements(2.5) {
            let mut p = start;
            place_anchor_at_centre(&mut p, &centred);
            assert_eq!(p, start);
        }
    }
}
