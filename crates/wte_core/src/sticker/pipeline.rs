//! From a stamp's source to the picture its bake places (STICKER_PLAN 2.6; ARCHITECTURE 6.5): the frame of a styled
//! picture (`content + 2 * margin`), the bake placement that puts the content on the stored placement, the working
//! size at which styles are drawn, and the render scale of a sticker stamp. Pure functions shared by the document's
//! UI-thread path and the bake job.

use glam::Vec2;

use crate::math::mul_u8;
use crate::pixel::PixelBuf;
use crate::pixel_ops::{PremulMips, unpremultiply};
use crate::stamp::{BakeQuality, Placement};
use crate::sticker::fx::apply_styles;
use crate::sticker::model::StickerDoc;
use crate::sticker::style::StyleStack;
use crate::sticker::{MAX_ARTBOARD, counters};

/// Most pixels (margin included) of a picture styled at the Preview working size (2.6, on the UI thread).
pub const PREVIEW_STYLE_PIXELS: u64 = 524_288;
/// Largest input side of `apply_styles` in the stamp pipeline (2.6 (a)).
pub const MAX_STYLE_SIDE: u32 = 4096;
/// Largest output side (margin included) of a styled stamp picture: a bound on `apply_styles`' memory for large
/// style lengths at a large reference scale (check note P1, minor 4).
pub const MAX_STYLED_SIDE: u32 = 6144;
/// Largest render of a sticker stamp: pixels a side and in total (2.6 "Sticker render scale").
pub const MAX_RENDER_SIDE: u32 = 4096;
pub const MAX_RENDER_PIXELS: u64 = 16_777_216;
/// Most pixels of a sticker render for a `BakeQuality::Preview` bake.
pub const PREVIEW_RENDER_PIXELS: u64 = 1_048_576;
/// Text stamps: style lengths are pixels at this many pixels per em (2.3 reference scale).
pub const STYLE_REFERENCE_EM: f32 = 128.0;

/// What a stamp's picture looks like before pixels exist (2.6): picture size = `content + 2 * margin` on both axes.
/// `content` is in the picture's own pixels (the working size of a styled picture).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SourceFrame {
    pub content: [u32; 2],
    pub margin: u32,
}

impl SourceFrame {
    /// A frame without margin.
    pub fn plain(content: [u32; 2]) -> SourceFrame {
        SourceFrame { content, margin: 0 }
    }

    /// The picture size `content + 2 * margin`.
    pub fn size(&self) -> [u32; 2] {
        self.content.map(|c| c + 2 * self.margin)
    }
}

/// The bake placement of a picture with `frame` (2.6 "Two placements"): the stored placement with its size multiplied
/// per axis by `(content + 2 * margin) / content`; centre, rotation, flips, depth and mode untouched. With margin 0
/// (or an empty content) the stored placement itself, without arithmetic (bit for bit).
pub fn bake_placement_of(stored: &Placement, frame: &SourceFrame) -> Placement {
    let [cw, ch] = frame.content;
    if frame.margin == 0 || cw == 0 || ch == 0 {
        return *stored;
    }
    let m = 2.0 * frame.margin as f32;
    let f = Vec2::new((cw as f32 + m) / cw as f32, (ch as f32 + m) / ch as f32);
    let mut p = *stored;
    match &mut p {
        Placement::Projected(pr) => pr.half_size *= f,
        Placement::Uv(u) => u.size_px *= f,
    }
    p
}

/// Texels a placement covers per axis (the density measure of `text_raster_px`): projected `2 |half_size| /
/// texel_world` of its home set, UV `|size_px|`.
pub fn placement_texels(p: &Placement, texel_world: f32) -> Vec2 {
    match p {
        Placement::Projected(pr) => 2.0 * pr.half_size.abs() / texel_world,
        Placement::Uv(u) => u.size_px.abs(),
    }
}

/// Size of mip level `level` of a picture of `size` (`PremulMips`: `max(1, side / 2)` per step).
pub fn level_size(size: [u32; 2], level: u32) -> [u32; 2] {
    size.map(|v| v.checked_shr(level).unwrap_or(0).max(1).min(v.max(1)))
}

/// How a stamp's styles are drawn (2.6 steps 2-4).
#[derive(Clone, Debug, PartialEq)]
pub struct Styling {
    /// Sanitized, with at least one entry on.
    pub styles: StyleStack,
    /// Multiplied into the base picture before the styles (the bake then gets `[255; 4]`).
    pub tint: [u8; 4],
    /// Picture pixels per style-length unit (image 1, text `size_px / 128`, sticker its render scale).
    pub ref_scale: f32,
    /// Texels the stored placement covers per axis (`placement_texels`).
    pub texels: Vec2,
    pub quality: BakeQuality,
}

/// The working level `L` of 2.6 for a base picture of `size`: the smallest `L` (scale `1 / 2^L`) such that the working
/// picture has no side over `MAX_STYLE_SIDE` and its styled output none over `MAX_STYLED_SIDE`, and for a `Full` bake
/// is not more than twice as dense as the texels it lands on (the denser axis is allowed down to 2 pixels per texel;
/// the other stays at least that dense), for a `Preview` has at most `PREVIEW_STYLE_PIXELS` pixels with its margin.
/// Returns `(L, style scale = ref_scale / 2^L, margin)`.
pub fn work_level(size: [u32; 2], styling: &Styling) -> (u32, f32, u32) {
    let max_level = 32 - size[0].max(size[1]).max(1).leading_zeros();
    let mut chosen = None;
    for l in 0..=max_level {
        let [w, h] = level_size(size, l);
        let scale = styling.ref_scale / (1u64 << l) as f32;
        let m = styling.styles.margin(scale);
        let (ow, oh) = (w as u64 + 2 * m as u64, h as u64 + 2 * m as u64);
        chosen = Some((l, scale, m));
        if w.max(h) > MAX_STYLE_SIDE || ow.max(oh) > MAX_STYLED_SIDE as u64 {
            continue;
        }
        let fits = match styling.quality {
            BakeQuality::Preview => ow * oh <= PREVIEW_STYLE_PIXELS,
            BakeQuality::Full => {
                let t = styling.texels;
                let density = (w as f32 / t.x).min(h as f32 / t.y);
                !(density.is_finite() && density > 2.0)
            }
        };
        if fits {
            break;
        }
    }
    chosen.unwrap_or((0, styling.ref_scale, styling.styles.margin(styling.ref_scale)))
}

/// Draws `styling` around the base picture `mips` (premultiplied, the base at level 0) at its working level (2.6 step
/// 4): level `L` un-premultiplied, the tint multiplied in, `apply_styles` at the style scale. Returns the styled mips,
/// their frame and the style scale.
pub fn styled_mips(mips: &PremulMips, styling: &Styling) -> (PremulMips, SourceFrame, f32) {
    let (l, scale, _) = work_level(mips.size(), styling);
    let level = mips.levels.get(l as usize).or(mips.levels.last());
    let mut work = level.cloned().unwrap_or_else(|| PixelBuf::new(0, 0));
    for p in work.data.as_chunks_mut::<4>().0.iter_mut() {
        let q = unpremultiply([p[0], p[1], p[2], p[3]]);
        p.copy_from_slice(&q);
    }
    let (styled, frame) = style_work(work, styling, scale);
    (PremulMips::build(&styled), frame, scale)
}

/// The finished styled picture of a straight base (copy, export): at its working level (`work_level`; level 0 is the
/// base itself, no premultiplied round trip), tinted and styled, straight RGBA8.
pub fn styled_straight(base: &PixelBuf, styling: &Styling) -> PixelBuf {
    let (l, scale, _) = work_level([base.width, base.height], styling);
    let work = if l == 0 {
        base.clone()
    } else {
        let mips = PremulMips::build(base);
        let mut level = mips.levels.into_iter().nth(l as usize).unwrap_or_else(|| PixelBuf::new(0, 0));
        for p in level.data.as_chunks_mut::<4>().0.iter_mut() {
            let q = unpremultiply([p[0], p[1], p[2], p[3]]);
            p.copy_from_slice(&q);
        }
        level
    };
    style_work(work, styling, scale).0
}

/// Steps 2 and 4 of 2.6 on a straight working picture: the tint multiplied in, `apply_styles` at `scale`.
fn style_work(mut work: PixelBuf, styling: &Styling, scale: f32) -> (PixelBuf, SourceFrame) {
    let tint = styling.tint;
    if tint != [255; 4] {
        for p in work.data.as_chunks_mut::<4>().0.iter_mut() {
            for (c, t) in p.iter_mut().zip(tint) {
                *c = mul_u8(*c, t);
            }
        }
    }
    let styled = apply_styles(&work, &styling.styles, scale);
    let margin = (styled.width.saturating_sub(work.width)) / 2;
    if styling.quality == BakeQuality::Preview {
        counters::styled_pixels(styled.width as u64 * styled.height as u64);
    }
    (styled, SourceFrame { content: [work.width, work.height], margin })
}

/// The render scale of a sticker stamp (2.6): 1 when every visible layer is a picture drawn at scale 1 or larger;
/// otherwise the smallest of 1, 2, 4 that gives at least one sticker pixel per texel along the artboard's height
/// (`texels_h` = texels the placement covers vertically), capped so that the render stays within 4096 pixels a side
/// and 16.8 megapixels. `Preview` then halves it (down to 1/4) until the render has at most 1,048,576 pixels.
pub fn sticker_scale(doc: &StickerDoc, texels_h: f32, quality: BakeQuality) -> f32 {
    let [w, h] = doc.size().map(|v| v.clamp(1, MAX_ARTBOARD) as f32);
    let fits = |s: f32| {
        let (rw, rh) = ((w * s).ceil(), (h * s).ceil());
        rw <= MAX_RENDER_SIDE as f32 && rh <= MAX_RENDER_SIDE as f32 && rw as f64 * rh as f64 <= MAX_RENDER_PIXELS as f64
    };
    let mut s: f32 = if doc.scale_one_is_enough() {
        1.0
    } else {
        let per_texel = if texels_h.is_finite() && texels_h > 0.0 { texels_h / h } else { 1.0 };
        [1.0f32, 2.0, 4.0].into_iter().find(|&c| c >= per_texel).unwrap_or(4.0)
    };
    while s > 1.0 && !fits(s) {
        s /= 2.0;
    }
    if quality == BakeQuality::Preview {
        while s > 0.25 && ((w * s).ceil() as f64) * ((h * s).ceil() as f64) > PREVIEW_RENDER_PIXELS as f64 {
            s /= 2.0;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stamp::{Projector, UvPlacement};
    use crate::sticker::style::StylePreset;
    use glam::Vec3;

    #[test]
    fn k3a_bake_placement_grows_by_the_margin_and_is_exact_without_one() {
        let p = Placement::Projected(Projector::from_surface(Vec3::ZERO, Vec3::Z, Vec3::Y, Vec2::new(0.3, 0.1), 0.2));
        assert_eq!(bake_placement_of(&p, &SourceFrame::plain([100, 50])), p);
        let Placement::Projected(b) = bake_placement_of(&p, &SourceFrame { content: [100, 50], margin: 10 }) else { panic!() };
        assert!((b.half_size - Vec2::new(0.36, 0.14)).abs().max_element() < 1e-6, "{}", b.half_size);
        let uv = Placement::Uv(UvPlacement {
            set: crate::ids::TextureSetId(0),
            center_px: Vec2::new(5.0, 5.0),
            size_px: Vec2::new(-20.0, 10.0),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        let Placement::Uv(u) = bake_placement_of(&uv, &SourceFrame { content: [20, 10], margin: 5 }) else { panic!() };
        assert_eq!((u.size_px, u.center_px), (Vec2::new(-30.0, 20.0), Vec2::new(5.0, 5.0)));
    }

    #[test]
    fn k3a_work_level_bounds_the_picture() {
        let styles = StylePreset::ClassicSticker.styles([512, 512]);
        let st = |quality, texels: f32| Styling { styles: styles.clone(), tint: [255; 4], ref_scale: 1.0, texels: Vec2::splat(texels), quality };
        // An 8192 photo is styled at 4096 or less; a Preview at most 524,288 pixels with its margin.
        let (l, s, m) = work_level([8192, 8192], &st(BakeQuality::Full, 1.0e6));
        assert_eq!((l, s), (1, 0.5));
        assert!(m > 2);
        let (l, _, m) = work_level([8192, 8192], &st(BakeQuality::Preview, 1.0e6));
        let [w, h] = level_size([8192, 8192], l);
        assert!((w as u64 + 2 * m as u64) * (h as u64 + 2 * m as u64) <= PREVIEW_STYLE_PIXELS);
        // A Full bake on 256 texels: not more than twice as dense.
        let (l, _, _) = work_level([2048, 2048], &st(BakeQuality::Full, 256.0));
        assert_eq!(level_size([2048, 2048], l), [512, 512]);
        assert_eq!(level_size([5, 3], 1), [2, 1]);
        assert_eq!(level_size([5, 3], 9), [1, 1]);
    }

    #[test]
    fn k3a_sticker_scale_rules() {
        use crate::ids::{BlobId, StickerLayerId};
        use crate::sticker::model::{PicturePixels, StickerContent, StickerLayer};
        let picture = StickerDoc::from_picture("p", PicturePixels::Blob(BlobId(0)), [100, 100]);
        assert_eq!(sticker_scale(&picture, 1000.0, BakeQuality::Full), 1.0);
        let mut shrunk = picture.clone();
        shrunk.layers[0].transform.scale = Vec2::splat(0.5);
        assert_eq!(sticker_scale(&shrunk, 150.0, BakeQuality::Full), 2.0);
        assert_eq!(sticker_scale(&shrunk, 1000.0, BakeQuality::Full), 4.0);
        assert_eq!(sticker_scale(&shrunk, 50.0, BakeQuality::Full), 1.0);
        let mut big = StickerDoc::new("b", 3000, 3000);
        big.layers.push(StickerLayer::new(StickerLayerId(1), "s", StickerContent::Picture(PicturePixels::Blob(BlobId(0)))));
        big.layers[0].transform.scale = Vec2::splat(0.5);
        big.next_layer = 2;
        assert_eq!(sticker_scale(&big, 1.0e5, BakeQuality::Full), 1.0, "capped at 4096 a side");
        assert_eq!(sticker_scale(&big, 1.0e5, BakeQuality::Preview), 0.25);
    }
}
