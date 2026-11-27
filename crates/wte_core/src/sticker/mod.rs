//! Stickers (STICKER_PLAN, ARCHITECTURE 1.12, 5.27, 6.11): small layered picture documents placed on the model by
//! stamps, and the picture maths they need. Headless, deterministic, CPU only; all pixels straight sRGB RGBA8
//! (ARCHITECTURE 4.2).
//!
//! Package P1: the layer styles ([`style`], drawn by [`fx::apply_styles`]), adjustments ([`adjust`]), vector shapes
//! and polygon coverage ([`shape`]), the affine resampler ([`resample`]) and the limits below. Package P2:
//! selections and colour-tolerance regions ([`select`]), the cut-out helpers ([`cutout`]) and blur, sharpen and the
//! gradient tool ([`filter`]). Package P3a: the sticker document ([`model`]), its rendering ([`render`]), the
//! pipeline from a stamp's source to the picture its bake places ([`pipeline`]), `refit_placement` (2.9) and the
//! work counters ([`counters`]); the document commands are `doc_stickers.rs`. Package P3b: the pixel edits, the
//! sticker strokes, promotion inside an edit, text conversion and Revert (`doc_sticker_edits.rs`). Package P4: the
//! `.wtes` sticker file ([`file`]; the project entries are `project::sticker_entries`).
//!
//! Determinism (ARCHITECTURE 4.7): integer or fixed-order `f32` arithmetic; rayon only over independent rows,
//! columns or bands; results identical for any thread count (tested with local pools of 1 and 8 threads).

pub mod adjust;
pub mod cutout;
pub mod file;
pub mod filter;
pub mod fx;
pub mod layout;
pub mod model;
pub mod pipeline;
pub mod render;
pub mod resample;
pub mod select;
pub mod shape;
pub mod style;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod fx_tests;

use glam::Vec2;

use crate::math::IRect;
use crate::stamp::Placement;

pub use model::{ArtboardShape, LayerMask, PicturePixels, StickerContent, StickerDoc, StickerLayer};
pub use pipeline::SourceFrame;
pub use render::{StickerRenderInput, render, render_size};

/// Largest artboard side (pixels).
pub const MAX_ARTBOARD: u32 = 4096;
/// Most layers in one sticker.
pub const MAX_STICKER_LAYERS: usize = 32;
/// Largest side of an imported picture layer (the decode limit of image stamps).
pub const MAX_PICTURE_SIDE: u32 = 8192;
/// Largest side of pixels created by an in-editor operation.
pub const MAX_CREATED_SIDE: u32 = 4096;
/// Most owned picture bytes in one sticker.
pub const MAX_OWNED_BYTES: u64 = 512 << 20;
/// Most stops of a gradient.
pub const MAX_GRADIENT_STOPS: usize = 8;
/// Longest style length (pixels at reference scale, STICKER_PLAN 2.3).
pub const MAX_STYLE_LENGTH: f32 = 256.0;

/// Placement of a layer's content in layer space (STICKER_PLAN 2.2): content pixels -> layer space. Scale and
/// rotation are about the content centre; flips are negative scales. Identity: content pixel `(x, y)` sits at
/// layer-space `(x, y)`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LayerTransform {
    pub translate: Vec2,
    pub scale: Vec2,
    /// Radians, clockwise as seen (y down), about the content centre.
    pub rotation: f32,
}

impl Default for LayerTransform {
    fn default() -> LayerTransform {
        LayerTransform::IDENTITY
    }
}

impl LayerTransform {
    /// No move, no scale, no rotation.
    pub const IDENTITY: LayerTransform = LayerTransform { translate: Vec2::ZERO, scale: Vec2::ONE, rotation: 0.0 };

    /// A pure move.
    pub fn moved(x: f32, y: f32) -> LayerTransform {
        LayerTransform { translate: Vec2::new(x, y), ..LayerTransform::IDENTITY }
    }

    /// True for the identity transform (bit for bit).
    pub fn is_identity(&self) -> bool {
        *self == LayerTransform::IDENTITY
    }

    /// Replaces non-finite values (translate 0, scale 1, rotation 0), keeps each scale's magnitude in
    /// `1e-3..=1e3` (its sign kept: a flip stays a flip) and the translation within +-1e6 pixels.
    pub fn sanitize(&mut self) {
        for v in [&mut self.translate.x, &mut self.translate.y] {
            *v = if v.is_finite() { v.clamp(-1.0e6, 1.0e6) } else { 0.0 };
        }
        for v in [&mut self.scale.x, &mut self.scale.y] {
            *v = if v.is_finite() && *v != 0.0 { v.signum() * v.abs().clamp(1.0e-3, 1.0e3) } else { 1.0 };
        }
        if !self.rotation.is_finite() {
            self.rotation = 0.0;
        }
    }

    /// Layer-space position of the content point `p` (content pixels, continuous) of a picture of `size`.
    pub fn apply(&self, size: [u32; 2], p: Vec2) -> Vec2 {
        let c = Vec2::new(size[0] as f32, size[1] as f32) * 0.5;
        let (sin, cos) = self.rotation.sin_cos();
        let q = (p - c) * self.scale;
        self.translate + c + Vec2::new(q.x * cos - q.y * sin, q.x * sin + q.y * cos)
    }

    /// The content point (content pixels, continuous) at layer-space point `q` of a picture of `size`: the inverse of
    /// `apply` (scales are never 0 after `sanitize`).
    pub fn invert(&self, size: [u32; 2], q: Vec2) -> Vec2 {
        let c = Vec2::new(size[0] as f32, size[1] as f32) * 0.5;
        let (sin, cos) = self.rotation.sin_cos();
        let d = q - self.translate - c;
        c + Vec2::new(d.x * cos + d.y * sin, -d.x * sin + d.y * cos) / self.scale
    }

    /// The mean of the two scales' magnitudes (content pixels per layer-space pixel is its inverse); 1 when not usable.
    pub fn mean_scale(&self) -> f32 {
        let m = (self.scale.x.abs() + self.scale.y.abs()) * 0.5;
        if m.is_finite() && m > 0.0 { m } else { 1.0 }
    }
}

/// The placement refit of 2.9 when a sticker's artboard changes from `old` to `new` (crop, trim, padding, canvas
/// size): the on-model size of one layer-space pixel stays (`k = host size / old artboard size` per axis), and so does
/// the on-model position of every layer-space point: the centre moves by `(new centre - old centre) * k` along the
/// placement's own axes (flips and rotation honoured), the size becomes `new size * k`. A degenerate `old` changes
/// nothing; an unchanged artboard changes nothing (bit for bit).
pub fn refit_placement(placement: &mut Placement, old: &IRect, new: &IRect) {
    if old == new {
        return;
    }
    let (ow, oh) = (old.width() as f32, old.height() as f32);
    let (nw, nh) = (new.width() as f32, new.height() as f32);
    if !(ow > 0.0 && oh > 0.0 && nw > 0.0 && nh > 0.0) {
        return;
    }
    let host = crate::text::frame::HostFrame::of(placement);
    let k = host.size / Vec2::new(ow, oh);
    let centre = |r: &IRect| Vec2::new((r.x0 as f32 + r.x1 as f32) * 0.5, (r.y0 as f32 + r.y1 as f32) * 0.5);
    let d = (centre(new) - centre(old)) * k;
    let shift = host.ex * d.x + host.ey * d.y;
    if !(k.is_finite() && shift.is_finite()) {
        return;
    }
    let f = Vec2::new(nw / ow, nh / oh);
    match placement {
        Placement::Projected(p) => {
            p.half_size *= f;
            p.origin += shift;
        }
        Placement::Uv(p) => {
            p.size_px *= f;
            p.center_px += shift.truncate();
        }
    }
}

/// Thread-local work counters of the sticker code (tests and the frame budget): renders, `apply_styles` calls and the
/// largest picture styled, per thread (one add per call, so they exist in every build, like the text raster counter).
pub mod counters {
    use std::cell::Cell;

    thread_local! {
        static RENDERS: Cell<u64> = const { Cell::new(0) };
        static STYLED: Cell<u64> = const { Cell::new(0) };
        static MAX_SIDE: Cell<u32> = const { Cell::new(0) };
        static MAX_PIXELS: Cell<u64> = const { Cell::new(0) };
        static BUFFER_COPIES: Cell<u64> = const { Cell::new(0) };
        static REGIONS: Cell<u64> = const { Cell::new(0) };
    }

    /// Stickers P7a: records one `render::render_region` call (a dirty rectangle of the editor canvas).
    pub(crate) fn region_rendered() {
        REGIONS.with(|c| c.set(c.get() + 1));
    }

    /// `render::render_region` calls on this thread so far (stickers P7a; `renders` counts only whole renders).
    pub fn region_renders() -> u64 {
        REGIONS.with(Cell::get)
    }

    /// Records one copy of a sticker picture or mask buffer that a snapshot, a cache or a job still shared
    /// (`Arc::make_mut` on a shared buffer, P3b).
    pub(crate) fn buffer_copied() {
        BUFFER_COPIES.with(|c| c.set(c.get() + 1));
    }

    /// Sticker picture and mask buffers copied because they were shared (copy-on-write), on this thread so far
    /// (STICKER_PLAN 2.10 "The first dab after a snapshot": one per stroke at most, in `sticker_stroke_begin`).
    pub fn buffer_copies() -> u64 {
        BUFFER_COPIES.with(Cell::get)
    }

    pub(crate) fn rendered() {
        RENDERS.with(|c| c.set(c.get() + 1));
    }

    pub(crate) fn styled(width: u32, height: u32) {
        STYLED.with(|c| c.set(c.get() + 1));
        MAX_SIDE.with(|c| c.set(c.get().max(width.max(height))));
    }

    /// Records the pixel count (margin included) of a picture styled at the Preview working size.
    pub(crate) fn styled_pixels(pixels: u64) {
        MAX_PIXELS.with(|c| c.set(c.get().max(pixels)));
    }

    /// Sticker renders (`sticker::render`) on this thread so far.
    pub fn renders() -> u64 {
        RENDERS.with(Cell::get)
    }

    /// `apply_styles` calls that drew something, on this thread so far.
    pub fn styled_calls() -> u64 {
        STYLED.with(Cell::get)
    }

    /// The largest input side `apply_styles` drew on this thread so far.
    pub fn max_styled_side() -> u32 {
        MAX_SIDE.with(Cell::get)
    }

    /// The largest output (margin included, pixels) the stamp pipeline styled on this thread so far.
    pub fn max_styled_pixels() -> u64 {
        MAX_PIXELS.with(Cell::get)
    }

    /// Sets this thread's largest-side and largest-output records back to 0.
    pub fn reset_max() {
        MAX_SIDE.with(|c| c.set(0));
        MAX_PIXELS.with(|c| c.set(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k1_layer_transform_basics() {
        let t = LayerTransform::IDENTITY;
        assert!(t.is_identity() && LayerTransform::default().is_identity());
        assert_eq!(t.apply([10, 6], Vec2::new(3.0, 4.0)), Vec2::new(3.0, 4.0));
        assert_eq!(LayerTransform::moved(2.0, -1.0).apply([10, 6], Vec2::new(3.0, 4.0)), Vec2::new(5.0, 3.0));
        // Clockwise as seen: a quarter turn takes the right edge's midpoint to the bottom.
        let q = LayerTransform { rotation: std::f32::consts::FRAC_PI_2, ..LayerTransform::IDENTITY };
        let p = q.apply([10, 10], Vec2::new(10.0, 5.0));
        assert!((p - Vec2::new(5.0, 10.0)).length() < 1e-5, "{p}");
        // A flip about the centre.
        let f = LayerTransform { scale: Vec2::new(-1.0, 1.0), ..LayerTransform::IDENTITY };
        assert_eq!(f.apply([10, 6], Vec2::new(0.0, 0.0)), Vec2::new(10.0, 0.0));
        let mut bad = LayerTransform {
            translate: Vec2::new(f32::NAN, 5.0e7),
            scale: Vec2::new(0.0, -1.0e9),
            rotation: f32::INFINITY,
        };
        bad.sanitize();
        assert_eq!(bad, LayerTransform { translate: Vec2::new(0.0, 1.0e6), scale: Vec2::new(1.0, -1.0e3), rotation: 0.0 });
    }
}
