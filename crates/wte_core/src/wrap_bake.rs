// Owner: round 6, package A (R6-1). The wrap bake (ARCHITECTURE.md 6.5a, plan 4.5): paints the picture of a wrapped
// stamp through its parameterization (`wrap_param.rs`). The scratch, band and crop structure is that of the projected
// bake (`stamp_bake::raster_jobs`), with the parameter coordinates of each triangle in place of the projection.
//
// Choices the plan leaves open, documented at the items below:
// - A fragment's stamp-local coordinates are the barycentric blend of its triangle's parameter corners (gutter
//   fragments extrapolate them, as the projected bake extrapolates positions), so UV seams are padded with the
//   continuation of the picture.
// - The mip level follows the parameterization's stretch: `src_px_per_world * texel_world * sqrt(param_area /
//   world_area)` (6.5a).

use std::sync::Arc;

use glam::Vec2;
use rayon::prelude::*;

use crate::blend::blend_row;
use crate::ids::{TextureSetId, TriId};
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::mesh::MeshData;
use crate::pixel::SparseBuf;
use crate::pixel_ops::PremulMips;
use crate::stamp::{BakeQuality, Projector};
use crate::stamp_bake::{BAND_ROWS, MIN_SIZE, SAMPLE_ALPHA_MAX, SCRATCH_BUDGET, crop_scratch, scratch_groups, shade, tint_factors};
use crate::texel_map::{Fragment, TexelMap, raster_tri};
use crate::wrap_param::{OwnerGrid, WrapLayer, WrapParam};

/// Overlap: the along range of a turn's layer, `[lo, hi)` on coordinate `axis`, and its period (0 for an open turn).
#[derive(Copy, Clone)]
struct Clip {
    axis: usize,
    lo: f32,
    hi: f32,
    period: f32,
}

impl Clip {
    fn of(layer: &WrapLayer) -> Option<Clip> {
        layer.clip.map(|[lo, hi]| Clip { axis: layer.axis, lo, hi, period: layer.period.unwrap_or(0.0) })
    }
}

/// The jobs of one layer: its turn range, its owner grid, the jobs per set.
type LayerJobs<'a> = (Option<Clip>, Option<&'a OwnerGrid>, Vec<Vec<WrapJob>>);

/// One triangle of a wrap bake.
struct WrapJob {
    tri: TriId,
    /// Unwrapped pixel-space box of the part of the triangle inside the picture (plus the gutter).
    clip: [f32; 4],
    lod: f32,
    corners: [Vec2; 3],
    /// Round 8: the corners' depth `g` and the triangle's longest 3D edge (the owner test).
    depth: [f32; 3],
    edge: f32,
}

/// Everything a fragment of a wrap bake needs.
struct WrapCtx<'a> {
    src: &'a PremulMips,
    quality: BakeQuality,
    limit_x: f32,
    limit_y: f32,
    inv_width: f32,
    inv_height: f32,
    flip_x: bool,
    flip_y: bool,
    tint: [f32; 4],
}

impl WrapCtx<'_> {
    /// The picture pixel of one fragment when its alpha beats `stored` (max-alpha rule), else `None`.
    #[inline]
    fn fragment(&self, job: &WrapJob, clip: Option<Clip>, owner: Option<&OwnerGrid>, f: &Fragment, stored: u8) -> Option<[u8; 4]> {
        let b = f.bary;
        let c = &job.corners;
        let mut l = c[0] * b[0] + c[1] * b[1] + c[2] * b[2];
        // Overlap: the turn's along range; a closed turn moves a fragment outside it by its period.
        if let Some(clip) = clip {
            let a = l[clip.axis];
            let shift = [0.0, clip.period, -clip.period].into_iter().find(|s| {
                let v = a + s;
                v >= clip.lo && v < clip.hi
            })?;
            l[clip.axis] += shift;
        }
        // Written so that NaN is rejected.
        if !(l.x.abs() <= self.limit_x && l.y.abs() <= self.limit_y) {
            return None;
        }
        // Round 8 (2.2-4): the owner test. The picture point goes to the surface place nearest the spine along the
        // surface; a fragment deeper than the cell's owner by more than the tolerance is another place: dropped.
        if let Some(grid) = owner
            && let Some(k) = grid.index(l.x, l.y)
        {
            let d = &job.depth;
            if !grid.keeps(k, d[0] * b[0] + d[1] * b[1] + d[2] * b[2], job.edge) {
                return None;
            }
        }
        // A fragment that cannot beat the stored alpha needs no sample (`shade` rounds monotonically).
        if stored > 0 && (((SAMPLE_ALPHA_MAX * self.tint[3]).min(1.0) * 255.0 + 0.5) as u8) <= stored {
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
        let pixel = shade(sample, self.tint, 1.0)?;
        (pixel[3] > stored).then_some(pixel)
    }
}

/// Unwrapped pixel-space box of the part of the parameter triangle `c` inside `|x| <= lx, |y| <= ly` (and the turn's
/// along range), grown by `grow` texels; `None` when it misses.
fn footprint(c: &[Vec2; 3], uv_px: &[Vec2; 3], lx: f32, ly: f32, clip: Option<Clip>, grow: f32) -> Option<[f32; 4]> {
    match clip {
        Some(k) if k.period != 0.0 => {
            // A closed turn: the union of the footprints of the triangle and its copies one period either way.
            let mut out: Option<[f32; 4]> = None;
            for shift in [0.0, k.period, -k.period] {
                let mut moved = *c;
                for p in &mut moved {
                    p[k.axis] += shift;
                }
                if let Some(b) = footprint_once(&moved, uv_px, lx, ly, clip, grow) {
                    out = Some(match out {
                        None => b,
                        Some(o) => [o[0].min(b[0]), o[1].min(b[1]), o[2].max(b[2]), o[3].max(b[3])],
                    });
                }
            }
            out
        }
        _ => footprint_once(c, uv_px, lx, ly, clip, grow),
    }
}

/// `footprint` of one copy of the triangle.
fn footprint_once(c: &[Vec2; 3], uv_px: &[Vec2; 3], lx: f32, ly: f32, clip: Option<Clip>, grow: f32) -> Option<[f32; 4]> {
    // (position, barycentric weights) of a convex polygon, clipped plane by plane.
    let mut poly: Vec<(Vec2, [f32; 3])> = vec![(c[0], [1.0, 0.0, 0.0]), (c[1], [0.0, 1.0, 0.0]), (c[2], [0.0, 0.0, 1.0])];
    let mut planes = vec![(0usize, 1.0f32, lx), (0, -1.0, lx), (1, 1.0, ly), (1, -1.0, ly)];
    if let Some(k) = clip {
        planes.push((k.axis, 1.0, k.hi));
        planes.push((k.axis, -1.0, -k.lo));
    }
    for (axis, sign, limit) in planes {
        let inside = |p: &(Vec2, [f32; 3])| limit - sign * p.0[axis];
        let mut next = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (da, db) = (inside(&a), inside(&b));
            if da >= 0.0 {
                next.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                let w = [0, 1, 2].map(|k| a.1[k] + (b.1[k] - a.1[k]) * t);
                next.push((a.0 + (b.0 - a.0) * t, w));
            }
        }
        poly = next;
        if poly.is_empty() {
            return None;
        }
    }
    let (mut lo, mut hi) = (Vec2::INFINITY, Vec2::NEG_INFINITY);
    for (_, w) in &poly {
        let p = uv_px[0] * w[0] + uv_px[1] * w[1] + uv_px[2] * w[2];
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return None;
    }
    Some([lo.x - grow, lo.y - grow, hi.x + grow, hi.y + grow])
}

/// The jobs of one layer per set, ascending triangle id.
fn layer_jobs(mesh: &MeshData, maps: &[Arc<TexelMap>], layer: &WrapLayer, ctx: &WrapCtx<'_>, src_px_per_world: f32) -> Vec<Vec<WrapJob>> {
    let mut jobs: Vec<Vec<WrapJob>> = maps.iter().map(|_| Vec::new()).collect();
    let clip = Clip::of(layer);
    for (i, (&tri, corners)) in layer.tris.iter().zip(&layer.corners).enumerate() {
        let Some(set) = mesh.tri_set(tri) else { continue };
        let Some(map) = maps.get(set.index()) else { continue };
        if map.width == 0 || map.height == 0 {
            continue;
        }
        let c = corners.map(Vec2::from);
        if !c.iter().all(|p| p.is_finite()) {
            continue;
        }
        let (lo, hi) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
        let periodic = clip.is_some_and(|k| k.period != 0.0);
        if !periodic && (lo.x > ctx.limit_x || hi.x < -ctx.limit_x || lo.y > ctx.limit_y || hi.y < -ctx.limit_y) {
            continue;
        }
        let pos = mesh.tri_positions(tri);
        let size = Vec2::new(map.width as f32, map.height as f32);
        let uv_px = mesh.tri_uvs(tri).map(|uv| uv * size);
        let Some(clip_box) = footprint(&c, &uv_px, ctx.limit_x, ctx.limit_y, clip, map.gutter as f32 + 2.0) else {
            continue;
        };
        let area_world = 0.5 * (pos[1] - pos[0]).cross(pos[2] - pos[0]).length();
        let area_px = 0.5 * (uv_px[1] - uv_px[0]).perp_dot(uv_px[2] - uv_px[0]).abs();
        let area_param = 0.5 * (c[1] - c[0]).perp_dot(c[2] - c[0]).abs();
        let texel_world = (area_world / area_px).sqrt();
        let texel_world = if texel_world.is_finite() && texel_world > 0.0 { texel_world } else { map.stats.texel_world.max(0.0) };
        let stretch = (area_param / area_world).sqrt();
        let stretch = if stretch.is_finite() && stretch > 0.0 { stretch } else { 1.0 };
        jobs[set.index()].push(WrapJob {
            tri,
            clip: clip_box,
            lod: (src_px_per_world * texel_world * stretch).max(1.0).log2(),
            corners: c,
            depth: layer.depth.get(i).copied().unwrap_or([0.0; 3]),
            edge: layer.edge.get(i).copied().unwrap_or(0.0),
        });
    }
    jobs
}

/// The scratch box of one set, `[x0, x1) x [y0, y1)` texels.
#[derive(Clone, Copy)]
struct Area {
    set: usize,
    x0: i64,
    x1: i64,
    y0: i64,
    y1: i64,
}

impl Area {
    fn bytes(&self) -> usize {
        ((self.x1 - self.x0) * (self.y1 - self.y0) * 4) as usize
    }
    fn width(&self) -> usize {
        (self.x1 - self.x0) as usize
    }
}

/// Texel columns / rows a clip interval selects; a wrapping interval forces the whole axis.
fn span(lo: f32, hi: f32, size: i64) -> (i64, i64) {
    let (a, b) = (lo.floor() as i64, (hi.floor() as i64).saturating_add(1));
    if a >= 0 && b <= size { (a, b) } else { (0, size) }
}

/// Rasters the jobs of one set (of one layer) into the scratch `pixels` of `area`, in 64-row bands; each texel
/// sees its fragments in job order (ascending triangle), so the result does not depend on the thread count.
#[allow(clippy::too_many_arguments)]
fn raster_set(mesh: &MeshData, map: &TexelMap, area: &Area, jobs: &[WrapJob], ctx: &WrapCtx<'_>, clip: Option<Clip>, owner: Option<&OwnerGrid>, pixels: &mut [u8]) {
    let (h, bw) = (map.height as i64, area.width());
    let bh = (area.y1 - area.y0) as usize;
    let band_count = bh.div_ceil(BAND_ROWS);
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); band_count];
    for (j, job) in jobs.iter().enumerate() {
        let (a, b) = span(job.clip[1], job.clip[3], h);
        if b <= area.y0 || a >= area.y1 {
            continue;
        }
        let first = (a.max(area.y0) - area.y0) as usize / BAND_ROWS;
        let last = ((b.min(area.y1) - 1 - area.y0).max(0) as usize / BAND_ROWS).min(band_count - 1);
        for bin in &mut bins[first..=last] {
            bin.push(j as u32);
        }
    }
    pixels.par_chunks_mut(BAND_ROWS * bw * 4).enumerate().for_each(|(band, pixels)| {
        let (x0, x1) = (area.x0, area.x1);
        let row0 = area.y0 as u32 + (band * BAND_ROWS) as u32;
        let rows = row0..row0 + (pixels.len() / (bw * 4)) as u32;
        for &j in &bins[band] {
            let job = &jobs[j as usize];
            raster_tri(mesh, map, job.tri, Some(job.clip), rows.clone(), &mut |f: Fragment| {
                let x = f.x as i64;
                if x < x0 || x >= x1 || f.y < row0 {
                    return;
                }
                let i = ((f.y - row0) as usize * bw + (x - x0) as usize) * 4;
                let Some(stored) = pixels.get(i + 3).copied() else { return };
                // Max-alpha rule: idempotent for texels visited more than once; full alpha cannot change any more.
                if stored == u8::MAX {
                    return;
                }
                if let Some(pixel) = ctx.fragment(job, clip, owner, &f, stored) {
                    pixels[i..i + 4].copy_from_slice(&pixel);
                }
            });
        }
    });
}

/// Paints `src` through the parameterization `param` of the wrapped stamp `p` and returns one cropped buffer per
/// touched set (ascending set id), like `bake_projected` (6.5a). `param` must be `wrap_param` of `p` (or serve it).
///
/// A projector with a non-finite field or a zero size, an empty source or a tint with alpha 0 bakes nothing.
/// With several turns (Overlap) the layers are rastered bottom first, each with the max-alpha rule inside the layer,
/// and composited over the set's scratch with the Normal blend at 100 %; with one turn the single-layer path runs.
/// Deterministic: the result does not depend on the thread count.
pub fn bake_wrapped(
    mesh: &MeshData,
    maps: &[Arc<TexelMap>],
    param: &WrapParam,
    src: &PremulMips,
    tint: [u8; 4],
    p: &Projector,
    quality: BakeQuality,
) -> Vec<(TextureSetId, SparseBuf)> {
    let [src_w, src_h] = src.size();
    let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
    if src_w == 0 || src_h == 0 || tint[3] == 0 || maps.is_empty() || !p.is_finite() || !(hw > MIN_SIZE && hh > MIN_SIZE) {
        return Vec::new();
    }
    let (bx, by) = (2.0 * hw / src_w as f32, 2.0 * hh / src_h as f32);
    let src_px_per_world = (src_w as f32 / (2.0 * hw)).max(src_h as f32 / (2.0 * hh));
    let ctx = WrapCtx {
        src,
        quality,
        limit_x: hw + bx,
        limit_y: hh + by,
        inv_width: 0.5 / hw,
        inv_height: 0.5 / hh,
        flip_x: p.flip_x,
        flip_y: p.flip_y,
        tint: tint_factors(tint),
    };
    // Jobs per layer (bottom first) and set.
    let layers: Vec<LayerJobs<'_>> = param
        .layers
        .iter()
        .map(|layer| (Clip::of(layer), layer.owner.as_ref(), layer_jobs(mesh, maps, layer, &ctx, src_px_per_world)))
        .collect();
    // The scratch box per set: the union of the footprints of every layer.
    let mut areas: Vec<Area> = Vec::new();
    for (s, map) in maps.iter().enumerate() {
        let (w, h) = (map.width as i64, map.height as i64);
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0, h, 0);
        for job in layers.iter().flat_map(|l| &l.2[s]) {
            let (a, b) = span(job.clip[0], job.clip[2], w);
            x0 = x0.min(a);
            x1 = x1.max(b);
            let (a, b) = span(job.clip[1], job.clip[3], h);
            y0 = y0.min(a);
            y1 = y1.max(b);
        }
        if x0 < x1 && y0 < y1 {
            areas.push(Area { set: s, x0, x1, y0, y1 });
        }
    }
    let multi = layers.len() > 1;
    let mut results = Vec::new();
    // One extra scratch per set of a group with several layers: the budget counts both.
    let factor = if multi { 2 } else { 1 };
    for group in scratch_groups(&areas.iter().map(|a| a.bytes() * factor).collect::<Vec<_>>(), SCRATCH_BUDGET) {
        for area in &areas[group] {
            let map = &*maps[area.set];
            let mut scratch = vec![0u8; area.bytes()];
            if !multi {
                let (clip, owner, jobs) = &layers[0];
                raster_set(mesh, map, area, &jobs[area.set], &ctx, *clip, *owner, &mut scratch);
            } else {
                let mut layer_scratch = vec![0u8; area.bytes()];
                let row = area.width() * 4;
                for (clip, owner, jobs) in &layers {
                    if jobs[area.set].is_empty() {
                        continue;
                    }
                    raster_set(mesh, map, area, &jobs[area.set], &ctx, *clip, *owner, &mut layer_scratch);
                    scratch.par_chunks_mut(row).zip(layer_scratch.par_chunks_mut(row)).for_each(|(dst, src)| {
                        blend_row(dst, src, 255, BlendMode::Normal);
                        src.fill(0);
                    });
                }
            }
            let rect = IRect::new(area.x0 as i32, area.y0 as i32, area.x1 as i32, area.y1 as i32);
            let cropped = crop_scratch(rect, scratch);
            if !cropped.is_empty() {
                results.push((TextureSetId(area.set as u32), cropped));
            }
        }
    }
    results
}
