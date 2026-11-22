//! Small geometric and numeric helpers (ARCHITECTURE.md 5.3): `IRect`, `Aabb`, `Ray`, `DirtyRegion`, exact `div255`,
//! `smoothstep`, sRGB transfer functions and the FNV-1a hash.

use glam::Vec3;

/// Integer pixel rectangle, half-open: `x0 <= x < x1`, `y0 <= y < y1`.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    /// The canonical empty rectangle (all zero).
    pub const EMPTY: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

    /// Rectangle from its two corners (half-open).
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> IRect {
        IRect { x0, y0, x1, y1 }
    }

    /// Rectangle from its top-left corner and size.
    pub fn from_xywh(x: i32, y: i32, w: u32, h: u32) -> IRect {
        let w = w.min(i32::MAX as u32) as i32;
        let h = h.min(i32::MAX as u32) as i32;
        IRect { x0: x, y0: y, x1: x.saturating_add(w), y1: y.saturating_add(h) }
    }

    /// The rectangle `0,0 .. w,h`.
    pub fn from_size(w: u32, h: u32) -> IRect {
        IRect::from_xywh(0, 0, w, h)
    }

    /// Width in pixels; 0 when empty.
    pub fn width(&self) -> u32 {
        if self.is_empty() { 0 } else { (self.x1 as i64 - self.x0 as i64).min(u32::MAX as i64) as u32 }
    }

    /// Height in pixels; 0 when empty.
    pub fn height(&self) -> u32 {
        if self.is_empty() { 0 } else { (self.y1 as i64 - self.y0 as i64).min(u32::MAX as i64) as u32 }
    }

    /// True when the rectangle contains no pixel.
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    /// Number of pixels.
    pub fn area(&self) -> u64 {
        self.width() as u64 * self.height() as u64
    }

    /// Common part of both rectangles; `EMPTY` when disjoint.
    pub fn intersect(&self, o: &IRect) -> IRect {
        let r = IRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        };
        if r.is_empty() || self.is_empty() || o.is_empty() { IRect::EMPTY } else { r }
    }

    /// Bounding box of both rectangles; empty operands are ignored.
    pub fn union(&self, o: &IRect) -> IRect {
        match (self.is_empty(), o.is_empty()) {
            (true, true) => IRect::EMPTY,
            (true, false) => *o,
            (false, true) => *self,
            (false, false) => IRect {
                x0: self.x0.min(o.x0),
                y0: self.y0.min(o.y0),
                x1: self.x1.max(o.x1),
                y1: self.y1.max(o.y1),
            },
        }
    }

    /// True when pixel `(x, y)` lies inside.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// Grows (or, for negative `px`, shrinks) every side by `px`. An empty rectangle stays `EMPTY`.
    pub fn expand(&self, px: i32) -> IRect {
        if self.is_empty() {
            return IRect::EMPTY;
        }
        let r = IRect {
            x0: self.x0.saturating_sub(px),
            y0: self.y0.saturating_sub(px),
            x1: self.x1.saturating_add(px),
            y1: self.y1.saturating_add(px),
        };
        if r.is_empty() { IRect::EMPTY } else { r }
    }

    /// True when the two (non-empty) rectangles overlap or share an edge of non-zero length (corner contact does not count).
    pub fn touches(&self, o: &IRect) -> bool {
        if self.is_empty() || o.is_empty() {
            return false;
        }
        let ox = self.x1.min(o.x1) as i64 - self.x0.max(o.x0) as i64;
        let oy = self.y1.min(o.y1) as i64 - self.y0.max(o.y0) as i64;
        ox >= 0 && oy >= 0 && (ox > 0 || oy > 0)
    }
}

/// Axis-aligned bounding box in world space.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// The empty box: `min = +INF`, `max = -INF`.
    pub const EMPTY: Aabb = Aabb { min: Vec3::INFINITY, max: Vec3::NEG_INFINITY };

    /// Bounding box of a point set (`EMPTY` for no points).
    pub fn from_points(points: &[Vec3]) -> Aabb {
        let mut b = Aabb::EMPTY;
        for &p in points {
            b.extend(p);
        }
        b
    }

    /// Grows the box to contain `p`.
    pub fn extend(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    /// Bounding box of both boxes.
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }

    /// Centre of the box (`Vec3::ZERO` when empty).
    pub fn center(&self) -> Vec3 {
        if self.is_empty() { Vec3::ZERO } else { (self.min + self.max) * 0.5 }
    }

    /// Length of `max - min`; 0 when empty.
    pub fn diagonal(&self) -> f32 {
        if self.is_empty() { 0.0 } else { (self.max - self.min).length() }
    }

    /// True when the box contains no point.
    pub fn is_empty(&self) -> bool {
        !(self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z)
    }
}

/// Ray with a unit-length direction.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    /// Builds a ray; `dir` is normalized (a zero or non-finite direction becomes `Vec3::ZERO`).
    pub fn new(origin: Vec3, dir: Vec3) -> Ray {
        Ray { origin, dir: dir.normalize_or_zero() }
    }

    /// The point `origin + dir * t`.
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}

/// The changed part of an image, as pairwise disjoint rectangles.
///
/// Up to `MAX_RECTS` rectangles are kept exactly (touching rects merge). Beyond that the region switches to a grid of
/// `TILE` x `TILE` tiles: every added rect marks the tiles it covers, and the rectangles are the runs of marked tiles
/// per tile row (rows with identical runs merged), clipped to the bounds of what was added. So many small disjoint
/// rects spread over a large area (a 3D dab that reaches many UV islands of a packed atlas) stay close to their own
/// area instead of collapsing to their common bounding box.
#[derive(Clone, Debug, Default)]
pub struct DirtyRegion {
    rects: Vec<IRect>,
    /// Tile mode: the marked tiles (tile x, tile y), ordered by row then column.
    tiles: std::collections::BTreeSet<(i32, i32)>,
    /// Tile mode: bounding box of everything added.
    tile_bounds: IRect,
}

impl DirtyRegion {
    /// Most rectangles kept exactly.
    pub const MAX_RECTS: usize = 32;
    /// Side of a tile of the tile mode, in pixels.
    pub const TILE: i32 = 64;

    /// Adds a rectangle (empty ones are ignored).
    pub fn add(&mut self, r: IRect) {
        if r.is_empty() {
            return;
        }
        if !self.tiles.is_empty() {
            self.mark_tiles(r);
            return;
        }
        let mut r = r;
        // Merging can make the grown rect touch further rects: repeat until stable.
        while let Some(i) = self.rects.iter().position(|e| e.touches(&r)) {
            let e = self.rects.swap_remove(i);
            r = r.union(&e);
        }
        self.rects.push(r);
        if self.rects.len() > Self::MAX_RECTS {
            for r in std::mem::take(&mut self.rects) {
                self.mark_tiles(r);
            }
        }
    }

    fn mark_tiles(&mut self, r: IRect) {
        let t = Self::TILE;
        for ty in r.y0.div_euclid(t)..=(r.y1 - 1).div_euclid(t) {
            for tx in r.x0.div_euclid(t)..=(r.x1 - 1).div_euclid(t) {
                self.tiles.insert((ty, tx));
            }
        }
        self.tile_bounds = self.tile_bounds.union(&r);
    }

    /// The rectangles of the tile mode (see the type).
    fn tile_rects(&self) -> Vec<IRect> {
        let t = Self::TILE;
        // Runs of consecutive tiles per tile row: (row, first column, end column).
        let mut runs: Vec<(i32, i32, i32)> = Vec::new();
        for &(ty, tx) in &self.tiles {
            match runs.last_mut() {
                Some(last) if last.0 == ty && last.2 == tx => last.2 = tx + 1,
                _ => runs.push((ty, tx, tx + 1)),
            }
        }
        // Merge a run into the rect of the previous tile row with exactly the same columns.
        let mut out: Vec<(i32, i32, i32, i32)> = Vec::new(); // (x0 tile, x1 tile, first row, end row)
        let mut open: Vec<usize> = Vec::new();
        let mut row = i32::MIN;
        let mut next_open = Vec::new();
        for (ty, a, b) in runs {
            if ty != row {
                open = if ty == row + 1 { std::mem::take(&mut next_open) } else { Vec::new() };
                next_open.clear();
                row = ty;
            }
            match open.iter().position(|&i| out[i].0 == a && out[i].1 == b) {
                Some(k) => {
                    let i = open.swap_remove(k);
                    out[i].3 = ty + 1;
                    next_open.push(i);
                }
                None => {
                    out.push((a, b, ty, ty + 1));
                    next_open.push(out.len() - 1);
                }
            }
        }
        out.into_iter()
            .map(|(a, b, r0, r1)| IRect::new(a * t, r0 * t, b * t, r1 * t).intersect(&self.tile_bounds))
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// True when nothing is dirty.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty() && self.tiles.is_empty()
    }

    /// Returns the rectangles and clears the region.
    pub fn take(&mut self) -> Vec<IRect> {
        let out = self.rects();
        *self = DirtyRegion::default();
        out
    }

    /// The current rectangles: pairwise disjoint (and pairwise non-touching while fewer than `MAX_RECTS` were kept).
    pub fn rects(&self) -> Vec<IRect> {
        if self.tiles.is_empty() { self.rects.clone() } else { self.tile_rects() }
    }

    /// Bounding box of the region; `IRect::EMPTY` when nothing is dirty.
    pub fn bounds(&self) -> IRect {
        if self.tiles.is_empty() { self.rects.iter().fold(IRect::EMPTY, |acc, r| acc.union(r)) } else { self.tile_bounds }
    }
}

/// Exact `round(x / 255)` for `x <= 65535`.
#[inline]
pub fn div255(x: u32) -> u32 {
    let t = x + 128;
    (t + (t >> 8)) >> 8
}

/// `round(a * b / 255)`.
#[inline]
pub fn mul_u8(a: u8, b: u8) -> u8 {
    div255(a as u32 * b as u32) as u8
}

/// Hermite smoothstep: 0 for `x <= e0`, 1 for `x >= e1`. With `e0 == e1` it is a step at that edge.
#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e0 == e1 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    if t.is_nan() {
        return 0.0;
    }
    t * t * (3.0 - 2.0 * t)
}

/// sRGB-encoded byte to linear light in 0..=1.
pub fn srgb_to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

/// Linear light (clamped to 0..=1; NaN counts as 0) to an sRGB-encoded byte.
pub fn linear_to_srgb(c: f32) -> u8 {
    let c = if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) };
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// FNV-1a over `bytes`, starting from `seed` (pass the previous result to chain; the conventional first seed is the
/// FNV offset basis `0xcbf2_9ce4_8422_2325`). Used for `uv_hash` and test hashes.
pub fn fnv1a64(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div255_is_exact_for_all_u16() {
        for x in 0u32..=65535 {
            let reference = (x as f64 / 255.0).round() as u32;
            assert_eq!(div255(x), reference, "x = {x}");
        }
    }

    #[test]
    fn mul_u8_identities() {
        for a in 0u32..=255 {
            assert_eq!(mul_u8(a as u8, 255), a as u8);
            assert_eq!(mul_u8(a as u8, 0), 0);
            assert_eq!(mul_u8(255, a as u8), a as u8);
        }
        assert_eq!(mul_u8(128, 128), 64);
    }

    #[test]
    fn irect_basics() {
        let r = IRect::from_xywh(2, 3, 10, 5);
        assert_eq!(r, IRect::new(2, 3, 12, 8));
        assert_eq!((r.width(), r.height(), r.area()), (10, 5, 50));
        assert!(!r.is_empty());
        assert!(r.contains(2, 3) && r.contains(11, 7));
        assert!(!r.contains(12, 7) && !r.contains(11, 8) && !r.contains(1, 3));
        assert_eq!(IRect::from_size(4, 6), IRect::new(0, 0, 4, 6));
        assert_eq!(IRect::default(), IRect::EMPTY);

        let inverted = IRect::new(5, 5, 1, 9);
        assert!(inverted.is_empty());
        assert_eq!((inverted.width(), inverted.height(), inverted.area()), (0, 0, 0));
        assert!(IRect::new(0, 0, 0, 10).is_empty());
    }

    #[test]
    fn irect_intersect() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(5, 6, 20, 8);
        assert_eq!(a.intersect(&b), IRect::new(5, 6, 10, 8));
        assert_eq!(b.intersect(&a), IRect::new(5, 6, 10, 8));
        // Disjoint and edge-adjacent rects give exactly EMPTY.
        assert_eq!(a.intersect(&IRect::new(10, 0, 20, 10)), IRect::EMPTY);
        assert_eq!(a.intersect(&IRect::new(30, 30, 40, 40)), IRect::EMPTY);
        assert_eq!(a.intersect(&IRect::EMPTY), IRect::EMPTY);
        assert_eq!(a.intersect(&IRect::new(7, 7, 3, 3)), IRect::EMPTY);
        assert_eq!(a.intersect(&a), a);
    }

    #[test]
    fn irect_union_ignores_empty() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(20, -5, 30, 4);
        assert_eq!(a.union(&b), IRect::new(0, -5, 30, 10));
        assert_eq!(a.union(&IRect::EMPTY), a);
        assert_eq!(IRect::EMPTY.union(&b), b);
        assert_eq!(IRect::new(100, 100, 100, 200).union(&a), a);
        assert_eq!(IRect::new(9, 9, 3, 3).union(&IRect::new(50, 50, 50, 50)), IRect::EMPTY);
    }

    #[test]
    fn irect_expand_and_touches() {
        let a = IRect::new(10, 10, 20, 20);
        assert_eq!(a.expand(2), IRect::new(8, 8, 22, 22));
        assert_eq!(a.expand(-4), IRect::new(14, 14, 16, 16));
        assert_eq!(a.expand(-5), IRect::EMPTY);
        assert_eq!(IRect::EMPTY.expand(3), IRect::EMPTY);

        assert!(a.touches(&IRect::new(15, 15, 30, 30)), "overlap");
        assert!(a.touches(&IRect::new(20, 12, 30, 18)), "shared vertical edge");
        assert!(a.touches(&IRect::new(0, 0, 15, 10)), "shared horizontal edge");
        assert!(!a.touches(&IRect::new(20, 20, 30, 30)), "corner contact only");
        assert!(!a.touches(&IRect::new(21, 10, 30, 20)), "one pixel gap");
        assert!(!a.touches(&IRect::EMPTY));
        assert!(!IRect::EMPTY.touches(&a));
    }

    #[test]
    fn dirty_region_merges_touching_rects() {
        let mut d = DirtyRegion::default();
        assert!(d.is_empty());
        assert_eq!(d.bounds(), IRect::EMPTY);
        d.add(IRect::EMPTY);
        d.add(IRect::new(5, 5, 2, 2));
        assert!(d.is_empty());

        d.add(IRect::new(0, 0, 10, 10));
        d.add(IRect::new(100, 100, 110, 110));
        assert_eq!(d.rects().len(), 2);
        // Touches the first one (shared edge): merged into its bounding box.
        d.add(IRect::new(10, 0, 20, 10));
        assert_eq!(d.rects().len(), 2);
        assert!(d.rects().contains(&IRect::new(0, 0, 20, 10)));
        // Bridges both remaining rects: everything collapses into one.
        d.add(IRect::new(15, 5, 105, 105));
        assert_eq!(d.rects(), &[IRect::new(0, 0, 110, 110)]);
        assert_eq!(d.bounds(), IRect::new(0, 0, 110, 110));

        let taken = d.take();
        assert_eq!(taken, vec![IRect::new(0, 0, 110, 110)]);
        assert!(d.is_empty());
    }

    #[test]
    fn dirty_region_overflow_switches_to_tiles() {
        let mut d = DirtyRegion::default();
        let n = DirtyRegion::MAX_RECTS as i32;
        // A diagonal of small rects far apart: their bounding box is huge.
        for i in 0..n {
            d.add(IRect::from_xywh(i * 100, i * 100, 10, 10));
        }
        let rects = d.rects();
        assert_eq!(rects.len(), DirtyRegion::MAX_RECTS);
        for (i, r) in rects.iter().enumerate() {
            for o in &rects[i + 1..] {
                assert!(!r.touches(o));
            }
        }
        // One more: the region keeps one tile per rect instead of collapsing to the bounding box.
        d.add(IRect::from_xywh(5000, 5, 10, 10));
        let rects = d.rects();
        assert_eq!(rects.len(), DirtyRegion::MAX_RECTS + 1);
        let area: u64 = rects.iter().map(IRect::area).sum();
        // At most 2x2 tiles per added rect (a 10 px rect may straddle a tile corner).
        assert!(area <= 4 * (n as u64 + 1) * 64 * 64, "{area}");
        assert_eq!(d.bounds(), IRect::new(0, 0, 5010, (n - 1) * 100 + 10));
        // Rects inside the added bounds; a row of neighbouring tiles is one rect, equal rows merge.
        let mut d = DirtyRegion::default();
        for i in 0..=n {
            d.add(IRect::from_xywh(i * 1000, 0, 1, 1));
        }
        d.add(IRect::new(0, 0, 200, 150));
        assert!(d.rects().contains(&IRect::new(0, 0, 256, 150)), "{:?}", d.rects());
        assert_eq!(d.take().len(), n as usize + 1);
        assert!(d.is_empty() && d.rects().is_empty());
        // Negative coordinates are tiled too.
        for i in 0..=n {
            d.add(IRect::from_xywh(-500 + i * 100, -70, 3, 3));
        }
        assert!(d.rects().iter().all(|r| r.y0 >= -70 && r.y1 <= -67), "{:?}", d.rects());
    }

    /// Finding SP-3 (review round 2): a 3D stroke on a many-island atlas adds hundreds of small disjoint rects per
    /// frame. The region must stay close to the painted area instead of collapsing to the bounding box of the texture.
    #[test]
    fn many_scattered_dab_rects_stay_small() {
        let mut state = 0x9e37_79b9u32;
        let mut next = move |m: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % m) as i32
        };
        let mut d = DirtyRegion::default();
        let mut painted = 0u64;
        // 240 islands scattered over a 4096 atlas, a 20x20 texel piece of the dab in each.
        for _ in 0..240 {
            let r = IRect::from_xywh(next(4076), next(4076), 20, 20);
            painted += r.area();
            d.add(r);
        }
        let rects = d.rects();
        let area: u64 = rects.iter().map(IRect::area).sum();
        for (i, a) in rects.iter().enumerate() {
            assert!(rects[i + 1..].iter().all(|b| a.intersect(b).is_empty()), "overlapping rects");
        }
        // The old rule (collapse to the bounds after 8 rects) gave ~16.7 Mpx here; the tiles keep it near the
        // painted area (each 20 px piece rounds up to at most 2 x 2 tiles of 64 px).
        assert!(area <= 240 * 4 * 64 * 64, "dirty area {area} for {painted} painted texels");
    }

    #[test]
    fn dirty_region_always_covers_what_was_added() {
        // Deterministic pseudo-random rects: every added pixel must be covered by the region afterwards.
        let mut state = 0x1234_5678u32;
        let mut next = move |m: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % m) as i32
        };
        let mut d = DirtyRegion::default();
        let mut added = Vec::new();
        for _ in 0..200 {
            let r = IRect::from_xywh(next(500), next(500), 1 + next(40) as u32, 1 + next(40) as u32);
            d.add(r);
            added.push(r);
            assert!(d.rects().len() <= DirtyRegion::MAX_RECTS);
            for a in &added {
                let covered: u64 = d.rects().iter().map(|e| e.intersect(a).area()).sum();
                assert!(covered >= a.area());
            }
        }
    }

    #[test]
    fn aabb_basics() {
        assert!(Aabb::EMPTY.is_empty());
        assert_eq!(Aabb::EMPTY.diagonal(), 0.0);
        assert_eq!(Aabb::EMPTY.center(), Vec3::ZERO);
        assert!(Aabb::from_points(&[]).is_empty());

        let b = Aabb::from_points(&[Vec3::new(1.0, -2.0, 3.0), Vec3::new(-1.0, 2.0, 0.0)]);
        assert_eq!(b.min, Vec3::new(-1.0, -2.0, 0.0));
        assert_eq!(b.max, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(b.center(), Vec3::new(0.0, 0.0, 1.5));
        assert!((b.diagonal() - (4.0f32 + 16.0 + 9.0).sqrt()).abs() < 1e-6);
        assert!(!b.is_empty());

        assert_eq!(b.union(&Aabb::EMPTY), b);
        assert_eq!(Aabb::EMPTY.union(&b), b);
        let mut single = Aabb::EMPTY;
        single.extend(Vec3::ONE);
        assert!(!single.is_empty());
        assert_eq!(single.diagonal(), 0.0);
    }

    #[test]
    fn ray_normalizes() {
        let r = Ray::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -10.0));
        assert_eq!(r.dir, Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(r.at(2.0), Vec3::new(1.0, 2.0, 1.0));
        assert_eq!(Ray::new(Vec3::ZERO, Vec3::ZERO).dir, Vec3::ZERO);
    }

    #[test]
    fn smoothstep_edges() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.5), 0.5);
        assert!(smoothstep(0.0, 1.0, 0.25) < 0.25);
        assert_eq!(smoothstep(2.0, 4.0, 3.0), 0.5);
        assert_eq!(smoothstep(1.0, 1.0, 0.5), 0.0);
        assert_eq!(smoothstep(1.0, 1.0, 1.0), 1.0);
        assert_eq!(smoothstep(0.0, 1.0, f32::NAN), 0.0);
    }

    #[test]
    fn srgb_round_trip() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(128) - 0.2158).abs() < 1e-3);
        for c in 0u32..=255 {
            assert_eq!(linear_to_srgb(srgb_to_linear(c as u8)), c as u8);
        }
        assert_eq!(linear_to_srgb(-1.0), 0);
        assert_eq!(linear_to_srgb(2.0), 255);
        assert_eq!(linear_to_srgb(f32::NAN), 0);
    }

    #[test]
    fn fnv1a64_reference_values() {
        const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        assert_eq!(fnv1a64(BASIS, b""), BASIS);
        assert_eq!(fnv1a64(BASIS, b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(BASIS, b"foobar"), 0x8594_4171_f739_67e8);
        // Chaining equals hashing the concatenation.
        assert_eq!(fnv1a64(fnv1a64(BASIS, b"foo"), b"bar"), fnv1a64(BASIS, b"foobar"));
    }
}
