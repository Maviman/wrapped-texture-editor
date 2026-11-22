// Owner: core-paint (C3). The fill tool (ARCHITECTURE.md 5.19, algorithm 6.4 "Fill"): whole-layer fill and UV-island
// flood fill.
//
// The island flood walks over covered (non-gutter) texels only, because the gutters of neighbouring islands touch in
// real atlases; the island's own gutter is added afterwards through the owner triangles the flood collected.

use std::sync::Arc;

use glam::Vec2;
use rayon::prelude::*;

use crate::document::Document;
use crate::error::EditError;
use crate::history::{Edit, TileBackup};
use crate::ids::{LayerId, TextureSetId, TriId};
use crate::layer::LayerKind;
use crate::math::IRect;
use crate::mesh::MeshData;
use crate::pixel::PixelBuf;
use crate::texel_map::{TexelMap, raster_tri};

/// What the fill tool covers.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FillScope {
    /// The whole layer.
    Layer,
    /// The UV island under `seed_uv`.
    Island,
}

const NO_ISLAND: EditError = EditError::InvalidParam("no UV island here");

/// A fixed-size set of small integers.
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    fn new(len: usize) -> BitSet {
        BitSet { words: vec![0; len.div_ceil(64)] }
    }

    #[inline]
    fn get(&self, i: usize) -> bool {
        self.words.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 != 0)
    }

    #[inline]
    fn insert(&mut self, i: usize) {
        if let Some(w) = self.words.get_mut(i / 64) {
            *w |= 1 << (i % 64);
        }
    }
}

/// The texels of one UV island (gutter included) and their bounding box in image coordinates.
struct Region {
    width: u32,
    texels: BitSet,
    bounds: IRect,
}

impl Region {
    fn new(width: u32, height: u32) -> Region {
        Region { width, texels: BitSet::new(width as usize * height as usize), bounds: IRect::EMPTY }
    }

    #[inline]
    fn contains(&self, x: u32, y: u32) -> bool {
        self.texels.get(y as usize * self.width as usize + x as usize)
    }

    /// Adds the texels `x0..=x1` of row `y`.
    fn insert_span(&mut self, x0: u32, x1: u32, y: u32) {
        let row = y as usize * self.width as usize;
        for x in x0..=x1 {
            self.texels.insert(row + x as usize);
        }
        self.bounds = self.bounds.union(&IRect::new(x0 as i32, y as i32, x1 as i32 + 1, y as i32 + 1));
    }
}

/// The wrapped coordinates of `lo..hi` grown by `grow` on both sides: every coordinate once when that covers the axis.
fn grown_wrapped(lo: i32, hi: i32, grow: u32, size: u32) -> impl Iterator<Item = u32> {
    let (start, count) = (lo as i64 - grow as i64, (hi - lo) as i64 + 2 * grow as i64);
    let (start, count) = if count >= size as i64 { (0, size as i64) } else { (start, count) };
    (0..count).map(move |k| (start + k).rem_euclid(size as i64) as u32)
}

/// The island under `seed_uv` plus its own gutter (section 6.4, "Fill", steps 1-3).
fn island_region(mesh: &MeshData, map: &TexelMap, seed_uv: Vec2) -> Result<Region, EditError> {
    let (w, h) = (map.width, map.height);
    if w == 0 || h == 0 || !seed_uv.is_finite() {
        return Err(NO_ISLAND);
    }
    // Step 1: the seed texel, wrapped in integer pixel space (pitfall 1).
    let wrap = |t: f32, size: u32| ((t as f64 * size as f64).floor() as i64).rem_euclid(size as i64) as u32;
    let seed = (wrap(seed_uv.x, w), wrap(seed_uv.y, h));
    let (seed_tri, seed_is_gutter) = map.owner(seed.0, seed.1).ok_or(NO_ISLAND)?;
    // A gutter seed starts from any covered texel of its owner triangle instead.
    let start = if seed_is_gutter {
        let mut found = None;
        raster_tri(mesh, map, seed_tri, None, 0..h, &mut |f| {
            if found.is_none() && !f.gutter && map.owner(f.x, f.y) == Some((seed_tri, false)) {
                found = Some((f.x, f.y));
            }
        });
        found
    } else {
        Some(seed)
    };

    let mut region = Region::new(w, h);
    let mut tris = BitSet::new(mesh.tri_count() as usize);
    // The owner of a covered texel; `None` for gutter and free texels, which the flood never enters.
    let covered = |x: u32, y: u32| -> Option<TriId> {
        match map.owner(x, y) {
            Some((tri, false)) => Some(tri),
            _ => None,
        }
    };

    match start {
        // Step 2: scanline flood over covered texels, 4-connected, wrapping at the image borders.
        Some(start) => {
            let mut stack = vec![start];
            while let Some((x, y)) = stack.pop() {
                if region.contains(x, y) || covered(x, y).is_none() {
                    continue;
                }
                let open = |region: &Region, x: u32| !region.contains(x, y) && covered(x, y).is_some();
                let (mut x0, mut x1) = (x, x);
                while x0 > 0 && open(&region, x0 - 1) {
                    x0 -= 1;
                }
                while x1 + 1 < w && open(&region, x1 + 1) {
                    x1 += 1;
                }
                for xi in x0..=x1 {
                    if let Some(tri) = covered(xi, y) {
                        tris.insert(tri as usize);
                    }
                }
                region.insert_span(x0, x1, y);
                // Across the left / right border.
                if x0 == 0 {
                    stack.push((w - 1, y));
                }
                if x1 == w - 1 {
                    stack.push((0, y));
                }
                // One seed per run of open texels in the rows above and below.
                for ny in [(y + h - 1) % h, (y + 1) % h] {
                    let mut xi = x0;
                    while xi <= x1 {
                        if !region.contains(xi, ny) && covered(xi, ny).is_some() {
                            stack.push((xi, ny));
                            while xi <= x1 && !region.contains(xi, ny) && covered(xi, ny).is_some() {
                                xi += 1;
                            }
                        } else {
                            xi += 1;
                        }
                    }
                }
            }
        }
        // The owner triangle covers no texel centre of its own: the island is just the texels it owns.
        None => {
            raster_tri(mesh, map, seed_tri, None, 0..h, &mut |f| {
                if map.owner(f.x, f.y).is_some_and(|(tri, _)| tri == seed_tri) {
                    region.insert_span(f.x, f.x, f.y);
                }
            });
            region.insert_span(seed.0, seed.0, seed.1);
            return Ok(region);
        }
    }

    // Step 3: the gutter texels owned by the island's triangles lie within `gutter` texels of the covered ones.
    if map.gutter > 0 && !region.bounds.is_empty() {
        let b = region.bounds;
        let columns: Vec<u32> = grown_wrapped(b.x0, b.x1, map.gutter as u32, w).collect();
        for y in grown_wrapped(b.y0, b.y1, map.gutter as u32, h) {
            for &x in &columns {
                if let Some((tri, true)) = map.owner(x, y)
                    && tris.get(tri as usize)
                {
                    region.insert_span(x, x, y);
                }
            }
        }
    }
    Ok(region)
}

/// The fill's pixel rule: `rgba = color`, or with alpha lock `rgb = color.rgb`, alpha unchanged.
#[inline]
fn fill_span(pixels: &mut [u8], color: [u8; 4], alpha_lock: bool) {
    for px in pixels.as_chunks_mut::<4>().0 {
        *px = if alpha_lock { [color[0], color[1], color[2], px[3]] } else { color };
    }
}

/// Fills the region's texels of `buf`, backing up the tiles it writes to; returns the changed bounds.
fn fill_region(buf: &mut PixelBuf, backup: &mut TileBackup, region: &Region, color: [u8; 4], alpha_lock: bool) -> IRect {
    let b = region.bounds.intersect(&buf.rect());
    for y in b.y0 as u32..b.y1 as u32 {
        let mut x = b.x0 as u32;
        while x < b.x1 as u32 {
            if !region.contains(x, y) {
                x += 1;
                continue;
            }
            let run_start = x;
            while x < b.x1 as u32 && region.contains(x, y) {
                x += 1;
            }
            backup.touch_rect(buf, IRect::new(run_start as i32, y as i32, x as i32, y as i32 + 1));
            fill_span(&mut buf.row_mut(y)[run_start as usize * 4..x as usize * 4], color, alpha_lock);
        }
    }
    b
}

impl Document {
    /// Fills `layer` (unlocked, visible raster; else LayerLocked / LayerHidden / NotRaster) with `color`. Island scope
    /// needs an owned texel at seed_uv (else Err(InvalidParam("no UV island here"))) and fills exactly that island plus
    /// its own gutter (section 6.4). Respects alpha_lock. One Edit::Pixels entry.
    ///
    /// `seed_uv` is wrapped into the image and ignored for `FillScope::Layer`. `layer` must belong to `set`
    /// (else `UnknownLayer`; an unknown set is `UnknownSet`). The entry is labelled "Fill"; a fill that changes no
    /// pixel (the area already has that colour) pushes no entry.
    pub fn fill(&mut self, set: TextureSetId, layer: LayerId, seed_uv: Vec2, color: [u8; 4], scope: FillScope) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let target = self.set(set).ok_or(EditError::UnknownSet)?;
        let index = target.layer_index(layer).ok_or(EditError::UnknownLayer)?;
        let info = &target.layers[index];
        if !info.is_raster() {
            return Err(EditError::NotRaster);
        }
        if info.locked {
            return Err(EditError::LayerLocked);
        }
        if !info.visible {
            return Err(EditError::LayerHidden);
        }
        let alpha_lock = info.alpha_lock;
        let region = match scope {
            FillScope::Layer => None,
            FillScope::Island => Some(island_region(&self.mesh, &target.texel_map, seed_uv)?),
        };

        self.sets[set.index()].allocate_raster(index);
        let LayerKind::Raster(pixels) = &mut self.sets[set.index()].layers[index].kind else {
            return Err(EditError::NotRaster);
        };
        let buf = Arc::make_mut(pixels);
        let mut backup = TileBackup::new(buf.width, buf.height);
        let dirty = match &region {
            None => {
                let all = buf.rect();
                backup.touch_rect(buf, all);
                let stride = (buf.width as usize * 4).max(4);
                buf.data.par_chunks_mut(stride).for_each(|row| fill_span(row, color, alpha_lock));
                all
            }
            Some(region) => fill_region(buf, &mut backup, region, color, alpha_lock),
        };
        let patch = backup.into_patch(buf);
        if patch.is_empty() {
            return Ok(());
        }
        self.mark_layer_dirty(layer, dirty);
        self.push_edit("Fill", Edit::Pixels { set, layer, patch });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::testkit::{doc_state, doc_with_stamp, paint_rect};
    use crate::samples;
    use crate::testutil::doc_from_raw;

    const S0: TextureSetId = TextureSetId(0);
    const RED: [u8; 4] = [255, 0, 0, 255];
    const WHITE: [u8; 4] = [255; 4];

    fn pixels(doc: &Document, layer: LayerId) -> Vec<[u8; 4]> {
        { let set = doc.find_layer(layer).unwrap().0; let s = doc.set(set).unwrap(); doc.layer(layer).unwrap().to_dense(s.width, s.height).data.as_chunks::<4>().0.to_vec() }
    }

    /// Cube face of a triangle in `seam_cube` atlas order: +X, -X, +Y, -Y, +Z, -Z.
    fn face_of(mesh: &MeshData, tri: TriId) -> usize {
        let n = mesh.geo_normal(tri);
        let a = n.abs();
        if a.x >= a.y && a.x >= a.z {
            if n.x > 0.0 { 0 } else { 1 }
        } else if a.y >= a.z {
            if n.y > 0.0 { 2 } else { 3 }
        } else if n.z > 0.0 {
            4
        } else {
            5
        }
    }

    #[test]
    fn layer_fill_and_undo() {
        let mut doc = doc_from_raw(samples::flat_quad(), 32);
        let before = doc_state(&mut doc);
        doc.fill(S0, LayerId(1), Vec2::new(-7.0, f32::NAN), RED, FillScope::Layer).unwrap();
        assert!(pixels(&doc, LayerId(1)).iter().all(|&p| p == RED));
        assert_eq!(doc.history.undo_len(), 1);
        assert_eq!(doc.undo_label(), Some("Fill"));
        assert!(matches!(doc.history.undo.last().unwrap().edit, Edit::Pixels { set: S0, layer: LayerId(1), .. }));
        let after = doc_state(&mut doc);
        assert!(doc.is_modified());
        // The same fill again changes nothing and pushes nothing.
        doc.fill(S0, LayerId(1), Vec2::ZERO, RED, FillScope::Layer).unwrap();
        assert_eq!(doc.history.undo_len(), 1);
        doc.undo().unwrap();
        assert_eq!(doc_state(&mut doc), before);
        doc.redo().unwrap();
        assert_eq!(doc_state(&mut doc), after);
    }

    #[test]
    fn fill_respects_alpha_lock() {
        let mut doc = doc_from_raw(samples::flat_quad(), 32);
        paint_rect(&mut doc, LayerId(1), IRect::new(4, 4, 12, 12), [1, 2, 3, 50]);
        paint_rect(&mut doc, LayerId(1), IRect::new(20, 20, 22, 22), [1, 2, 3, 0]);
        doc.set_layer_alpha_lock(LayerId(1), true).unwrap();
        let before = pixels(&doc, LayerId(1));
        doc.fill(S0, LayerId(1), Vec2::ZERO, [9, 8, 7, 200], FillScope::Layer).unwrap();
        for (a, b) in pixels(&doc, LayerId(1)).iter().zip(&before) {
            assert_eq!(*a, [9, 8, 7, b[3]]);
        }
        doc_state(&mut doc);
    }

    #[test]
    fn fill_rejects_unpaintable_targets() {
        let mut doc = doc_with_stamp();
        let n = doc.history.undo_len();
        let fill = |doc: &mut Document, set: u32, layer: u64| {
            doc.fill(TextureSetId(set), LayerId(layer), Vec2::splat(0.5), RED, FillScope::Layer)
        };
        assert!(matches!(fill(&mut doc, 0, 101), Err(EditError::NotRaster)));
        assert!(matches!(fill(&mut doc, 0, 2), Err(EditError::UnknownLayer)), "layer of another set");
        assert!(matches!(fill(&mut doc, 0, 999), Err(EditError::UnknownLayer)));
        assert!(matches!(fill(&mut doc, 7, 1), Err(EditError::UnknownSet)));
        doc.set_layer_locked(LayerId(1), true).unwrap();
        assert!(matches!(fill(&mut doc, 0, 1), Err(EditError::LayerLocked)));
        doc.set_layer_visible(LayerId(3), false).unwrap();
        assert!(matches!(fill(&mut doc, 0, 3), Err(EditError::LayerHidden)));
        assert_eq!(doc.history.undo_len(), n + 2, "only the two property changes are history");
    }

    /// Island fill on seam_cube covers exactly the texels (covered and gutter) owned by the seed island.
    #[test]
    fn island_fill_on_seam_cube_stays_in_one_island_and_covers_its_gutter() {
        let size = 192;
        let mut doc = doc_from_raw(samples::seam_cube(1.0), size);
        let [u0, u1, v0, v1] = samples::seam_cube_island(0);
        let seed = Vec2::new((u0 + u1) * 0.5, (v0 + v1) * 0.5);
        doc.fill(S0, LayerId(1), seed, RED, FillScope::Island).unwrap();
        let map = Arc::clone(&doc.sets[0].texel_map);
        let px = pixels(&doc, LayerId(1));
        let (mut covered, mut gutter) = (0, 0);
        for y in 0..size {
            for x in 0..size {
                let mine = map.owner(x, y).filter(|&(t, _)| face_of(&doc.mesh, t) == 0);
                let p = px[(y * size + x) as usize];
                match mine {
                    Some((_, g)) => {
                        assert_eq!(p, RED, "texel ({x}, {y}) of the island not filled");
                        if g { gutter += 1 } else { covered += 1 }
                    }
                    None => assert_eq!(p, WHITE, "texel ({x}, {y}) outside the island filled"),
                }
            }
        }
        assert!(covered > 1000 && gutter > 100, "{covered} covered, {gutter} gutter");
        doc_state(&mut doc);
    }

    /// The gutters of tight_islands touch; the flood must not walk through them, whether it starts on a covered
    /// texel or on a gutter texel of island A.
    #[test]
    fn island_fill_on_tight_islands_leaves_the_neighbour_untouched() {
        let size = 256;
        let mut doc = doc_from_raw(samples::tight_islands(), size);
        let map = Arc::clone(&doc.sets[0].texel_map);
        // Triangles 0, 1 = island A, 2, 3 = island B.
        let a_gutter_seed = (0..size * size)
            .map(|i| (i % size, i / size))
            .filter(|&(x, y)| matches!(map.owner(x, y), Some((t, true)) if t < 2))
            .max_by_key(|&(x, _)| x)
            .expect("island A has a gutter");
        // The chosen gutter texel lies right next to island B's gutter.
        let (gx, gy) = a_gutter_seed;
        assert!(matches!(map.owner(gx + 1, gy), Some((t, true)) if t >= 2), "precondition: the gutters touch");
        let seeds = [
            Vec2::new(0.373, 0.5),
            Vec2::new((gx as f32 + 0.5) / size as f32, (gy as f32 + 0.5) / size as f32),
            // Wrapped seed.
            Vec2::new(-0.627, 2.5),
        ];
        for seed in seeds {
            doc.fill(S0, LayerId(1), seed, RED, FillScope::Island).unwrap();
            let px = pixels(&doc, LayerId(1));
            for y in 0..size {
                for x in 0..size {
                    let p = px[(y * size + x) as usize];
                    match map.owner(x, y) {
                        Some((t, _)) if t < 2 => assert_eq!(p, RED, "island A texel ({x}, {y}) for seed {seed}"),
                        Some((_, false)) => assert_eq!(p, WHITE, "island B texel ({x}, {y}) filled from seed {seed}"),
                        Some((_, true)) => assert_eq!(p, WHITE, "island B gutter ({x}, {y}) filled from seed {seed}"),
                        None => assert_eq!(p, WHITE, "free texel ({x}, {y}) filled from seed {seed}"),
                    }
                }
            }
            doc.undo().unwrap();
            assert!(pixels(&doc, LayerId(1)).iter().all(|&p| p == WHITE));
        }
    }

    #[test]
    fn island_fill_needs_an_island() {
        let mut doc = doc_from_raw(samples::tight_islands(), 256);
        let n = doc.history.undo_len();
        for seed in [Vec2::new(0.05, 0.05), Vec2::new(f32::NAN, 0.5), Vec2::new(0.5, f32::INFINITY)] {
            let r = doc.fill(S0, LayerId(1), seed, RED, FillScope::Island);
            assert!(matches!(r, Err(EditError::InvalidParam("no UV island here"))), "{seed}: {r:?}");
        }
        assert_eq!(doc.history.undo_len(), n);
        assert!(pixels(&doc, LayerId(1)).iter().all(|&p| p == WHITE));
    }

    /// An island that crosses the image border is flooded across it.
    #[test]
    fn island_fill_wraps_at_the_borders() {
        let mut doc = doc_from_raw(samples::wrapped_quad(), 64);
        doc.fill(S0, LayerId(1), Vec2::new(0.02, 0.98), RED, FillScope::Island).unwrap();
        let px = pixels(&doc, LayerId(1));
        // Quad A covers the whole wrapped tile.
        for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63), (32, 32)] {
            assert_eq!(px[y * 64 + x], RED, "({x}, {y})");
        }
        doc_state(&mut doc);
    }

    /// Finding SP-5 (review round 2): a new paint layer is unallocated; the first fill gives it its buffer, undo keeps
    /// the composite exact.
    #[test]
    fn fill_allocates_an_unallocated_layer() {
        let mut doc = doc_from_raw(samples::flat_quad(), 32);
        let layer = doc.add_raster_layer(S0, None).unwrap();
        let LayerKind::Raster(buf) = &doc.layer(layer).unwrap().kind else { panic!("not raster") };
        assert!(buf.data.is_empty(), "unallocated before the first edit");
        doc.fill(S0, layer, Vec2::splat(0.5), RED, FillScope::Layer).unwrap();
        let LayerKind::Raster(buf) = &doc.layer(layer).unwrap().kind else { panic!("not raster") };
        assert_eq!((buf.width, buf.height), (32, 32));
        assert!(pixels(&doc, layer).iter().all(|&p| p == RED));
        doc_state(&mut doc);
        doc.undo().unwrap();
        assert!(pixels(&doc, layer).iter().all(|&p| p == [0; 4]));
        doc_state(&mut doc);
    }

    #[test]
    fn grown_wrapped_lists_every_coordinate_once() {
        assert_eq!(grown_wrapped(2, 4, 3, 10).collect::<Vec<_>>(), vec![9, 0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(grown_wrapped(0, 8, 2, 10).collect::<Vec<_>>(), (0..10).collect::<Vec<_>>());
    }
}
