//! Stamp data model (ARCHITECTURE.md 5.12): types only. The `Projector` / `UvPlacement` methods and the bake
//! functions live in `stamp_bake.rs`.

use glam::{Quat, Vec2, Vec3};

use crate::ids::{BlobId, StampId, TextureSetId};
use crate::text::TextSpec;

/// What a stamp shows: an image blob or a text specification.
#[derive(Clone, Debug, PartialEq)]
pub enum StampSource {
    Image { blob: BlobId },
    Text(TextSpec),
}

/// Oriented projection box. Local frame: +X right, +Y up, +Z out of the surface toward the viewer.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Projector {
    /// Decal centre, on the surface.
    pub origin: Vec3,
    /// Local -> world.
    pub rotation: Quat,
    /// World units (x = half width, y = half height).
    pub half_size: Vec2,
    /// Box extent along +Z (toward the viewer).
    pub depth_front: f32,
    /// Box extent along -Z (into the surface).
    pub depth_back: f32,
    /// Full weight while the surface faces the projector within this angle (default 70).
    pub angle_start_deg: f32,
    /// Zero weight beyond this angle (default 88).
    pub angle_end_deg: f32,
    /// Default true.
    pub backface_cull: bool,
    /// Default true: surfaces hidden behind others receive nothing.
    pub occlusion: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Round 6: how the picture reaches the surface (`Project` = the projection box above, `Wrap` = laid on the surface
    /// like a sticker, `wrap.rs`). In `Wrap` the depth, angle, back-face and occlusion fields are kept and ignored.
    pub mode: SurfaceMode,
    /// Round 6: the options of `Wrap` mode; kept (and ignored) in `Project` mode, so a toggle back keeps them.
    pub wrap: WrapOptions,
}

/// How a projected (3D) stamp reaches the surface (round 6, ARCHITECTURE.md 5.12).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SurfaceMode {
    /// A slide projector from the front (`bake_projected`): today's behaviour.
    #[default]
    Project,
    /// Laid on the surface like a sticker: the picture's centre line runs straight along the surface (`wrap_param`,
    /// `bake_wrapped`).
    Wrap,
}

/// The two options of `SurfaceMode::Wrap` (round 6). Both `false` = stop at the edge of a part, ends meet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct WrapOptions {
    /// Continue onto separate parts closer than 1 % of the model's diagonal (`GAP_JUMP`).
    pub jump_gaps: bool,
    /// A picture longer than the way round keeps winding like tape, the later part on top (instead of the two ends
    /// meeting at the back).
    pub overlap: bool,
}

/// Placement of a stamp directly in the texture of one set.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvPlacement {
    pub set: TextureSetId,
    /// Texels of the target set.
    pub center_px: Vec2,
    /// Texels of the target set.
    pub size_px: Vec2,
    /// Radians, positive = clockwise on screen (v-down); stored verbatim in the manifest.
    pub rotation: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Default false.
    pub clip_to_islands: bool,
}

/// Where a stamp sits: projected onto the surface or placed in UV space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Placement {
    Projected(Projector),
    Uv(UvPlacement),
}

/// A re-editable decal (image or text). Each texture set it covers holds one linked layer.
#[derive(Clone, Debug, PartialEq)]
pub struct StampObject {
    pub id: StampId,
    pub name: String,
    pub source: StampSource,
    pub placement: Placement,
    /// Multiplies the source (straight RGBA); default `[255; 4]`.
    pub tint: [u8; 4],
    /// The set that always keeps a layer for this stamp, even when its bake is empty.
    pub home_set: TextureSetId,
}

/// Bake quality: `Preview` skips occlusion rays and samples a single mip level.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BakeQuality {
    Preview,
    Full,
}

/// Characters of the text used as the automatic name of a text stamp.
const TEXT_NAME_CHARS: usize = 32;

/// The automatic name of a text stamp (5.12): the first non-blank line of `content`, trimmed and shortened to 32
/// characters plus "..."; "Text" when there is none. The Text tool names a new stamp with it, and `set_stamp_source`
/// keeps a stamp that still carries the automatic name of its old text named after its new text (fix round 3, R3-9).
pub fn text_stamp_name(content: &str) -> String {
    let line = content.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Text");
    let mut name: String = line.chars().take(TEXT_NAME_CHARS).collect();
    if line.chars().count() > TEXT_NAME_CHARS {
        name.push_str("...");
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_stamp_names_are_the_first_line() {
        assert_eq!(text_stamp_name("  \nHello\nWorld"), "Hello");
        assert_eq!(text_stamp_name("WTE 2026"), "WTE 2026");
        assert_eq!(text_stamp_name(""), "Text");
        assert_eq!(text_stamp_name(" \n\t\n"), "Text");
        let long = text_stamp_name(&"x".repeat(100));
        assert_eq!(long, format!("{}...", "x".repeat(32)));
        assert_eq!(text_stamp_name(&"ä".repeat(32)), "ä".repeat(32));
    }
}
