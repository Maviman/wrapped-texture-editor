//! Stickers (STICKER_PLAN, ARCHITECTURE 1.12, 5.27, 6.11): small layered picture documents placed on the model by
//! stamps, and the picture maths they need. Headless, deterministic, CPU only; all pixels straight sRGB RGBA8
//! (ARCHITECTURE 4.2).
//!
//! Package P1 (this state): the layer styles ([`style`], drawn by [`fx::apply_styles`]), adjustments ([`adjust`]),
//! vector shapes and polygon coverage ([`shape`]), the affine resampler ([`resample`]) and the limits below. The
//! sticker document itself, its rendering and its place in `Document` follow in P3a.
//!
//! Determinism (ARCHITECTURE 4.7): integer or fixed-order `f32` arithmetic; rayon only over independent rows,
//! columns or bands; results identical for any thread count (tested with local pools of 1 and 8 threads).

pub mod adjust;
pub mod fx;
pub mod resample;
pub mod shape;
pub mod style;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod fx_tests;

use glam::Vec2;

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
