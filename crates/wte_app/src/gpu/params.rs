//! Plain data crossing the view / gpu boundary (ARCHITECTURE.md 8.4): what a view asks the renderer to draw.
//!
//! Shared contract file [S]: written completely by the wave-0 scaffold and not edited by later packages.

use wte_core::TextureSetId;

use crate::state::{Backdrop, ShadingMode};

/// The brush cursor drawn on the surface of the model.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushRing {
    /// World space.
    pub center: glam::Vec3,
    /// World units.
    pub radius: f32,
    /// `0..=1`; draws the thin inner ring at `radius * hardness`.
    pub hardness: f32,
    /// `Some(n)` = draw the ring only on faces of that side (facing `n`: the Circular type skips faces it will not
    /// paint, FEATURE_BRUSH 5.11, 6.5); `None` = on every face (the ring of round 3). Round 4; used by B6.
    pub side_normal: Option<glam::Vec3>,
}

/// Parameters of one 3D render (`Renderer::render_3d`).
#[derive(Clone, Debug, PartialEq)]
pub struct View3dParams {
    pub view_proj: glam::Mat4,
    pub eye: glam::Vec3,
    pub shading: ShadingMode,
    pub lit: bool,
    pub alpha_checker: bool,
    /// `Some(rgb)`: transparent texels are matted onto this colour instead of the checker (export preview of a flat
    /// format without alpha, i.e. JPEG). Takes precedence over `alpha_checker`.
    pub matte: Option<[u8; 3]>,
    pub wireframe: bool,
    pub seams: bool,
    pub nearest: bool,
    pub backdrop: Backdrop,
    pub brush: Option<BrushRing>,
}

/// Parameters of one UV-view render (`Renderer::render_uv`).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewUvParams {
    pub set: TextureSetId,
    pub center_uv: glam::Vec2,
    /// Physical pixels per texel.
    pub px_per_texel: f32,
    pub uv_wire: bool,
    pub alpha_checker: bool,
}
