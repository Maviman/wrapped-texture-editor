//! The layout maths of a sticker (STICKER_PLAN 3.3 "Sticker" commands and the Move / Transform tool; stickers P8):
//! where a layer's content box lands in layer space (`LayerBox`), and the whole-sticker edits that change only numbers:
//! Resize, Rotate, Flip, Canvas Size, Add Padding. Pure functions on a `StickerDoc`; no picture is resampled (Resize
//! scales transforms, text sizes, shape sizes and style lengths). The document commands that use them (one history
//! entry each, the placement refit of 2.9) are in `doc_sticker_layout.rs`.
//!
//! The content box of a layer: a picture `[0, w] x [0, h]` (content pixels), a shape `[0, size]`, a text its frame
//! (the picture the text engine draws at `size_px`, with the anchor inside it). A content point `q` lands at
//! `pivot_pos + R(rotation) * (scale * (q - pivot))` (`LayerBox::to_layer`), where `pivot` is the box centre for
//! pictures and shapes (`pivot_pos = translate + pivot`) and the anchor for a text (`pivot_pos = translate`): exactly
//! the placement `render` draws (`LayerTransform::apply`, `text_content`, the shape branch of `content_raster`).

use glam::Vec2;

use crate::math::IRect;
use crate::sticker::model::{ArtboardShape, StickerContent, StickerDoc, StickerLayer};
use crate::sticker::shape::ShapeKind;
use crate::sticker::{LayerTransform, MAX_ARTBOARD};

/// Smallest and largest factor of one Resize (per axis).
pub const MIN_RESIZE: f32 = 0.01;
pub const MAX_RESIZE: f32 = 100.0;
/// Most pixels Add Padding adds on each side.
pub const MAX_PADDING: u32 = 2048;

/// Where a layer's content box lands (module docs).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LayerBox {
    /// Size of the content box (content pixels).
    pub size: Vec2,
    /// The point of the box the transform turns and scales about.
    pub pivot: Vec2,
    /// A text layer: the pivot (its anchor) sits at `translate`; otherwise at `translate + pivot`.
    pub text: bool,
}

/// `v` turned by `angle` radians (clockwise as seen, y down; the `LayerTransform` convention).
fn rot(v: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

impl LayerBox {
    /// The box of a picture or a shape of `size` (pivot at its centre).
    pub fn centred(size: Vec2) -> LayerBox {
        LayerBox { size, pivot: size * 0.5, text: false }
    }

    /// Layer-space position of the pivot under `t`.
    pub fn pivot_pos(&self, t: &LayerTransform) -> Vec2 {
        if self.text { t.translate } else { t.translate + self.pivot }
    }

    /// The translation that puts the pivot at `pos`.
    pub fn translate_for(&self, pos: Vec2) -> Vec2 {
        if self.text { pos } else { pos - self.pivot }
    }

    /// Layer-space position of the content point `q` under `t`.
    pub fn to_layer(&self, t: &LayerTransform, q: Vec2) -> Vec2 {
        self.pivot_pos(t) + rot((q - self.pivot) * t.scale, t.rotation)
    }

    /// The content point at the layer-space point `p` under `t` (scales are never 0 after `sanitize`).
    pub fn to_content(&self, t: &LayerTransform, p: Vec2) -> Vec2 {
        self.pivot + rot(p - self.pivot_pos(t), -t.rotation) / t.scale
    }

    /// The four corners in layer space (top-left, top-right, bottom-right, bottom-left of the content box).
    pub fn corners(&self, t: &LayerTransform) -> [Vec2; 4] {
        let s = self.size;
        [Vec2::ZERO, Vec2::new(s.x, 0.0), s, Vec2::new(0.0, s.y)].map(|q| self.to_layer(t, q))
    }

    /// The layer-space bounds of the corners (`[min, max]`).
    pub fn bounds(&self, t: &LayerTransform) -> [Vec2; 2] {
        let c = self.corners(t);
        let min = c.iter().fold(Vec2::splat(f32::INFINITY), |a, &b| a.min(b));
        let max = c.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, &b| a.max(b));
        [min, max]
    }

    /// Layer-space position of the box centre under `t`.
    pub fn centre(&self, t: &LayerTransform) -> Vec2 {
        self.to_layer(t, self.size * 0.5)
    }

    /// `t` with its rotation set to `angle`, the box centre kept where it is.
    pub fn with_rotation(&self, t: &LayerTransform, angle: f32) -> LayerTransform {
        let c = self.centre(t);
        let mut out = LayerTransform { rotation: angle, ..*t };
        let pivot_pos = c - rot((self.size * 0.5 - self.pivot) * t.scale, angle);
        out.translate = self.translate_for(pivot_pos);
        out
    }

    /// `t` with its scale set to `scale`, the content point `fixed` kept at its layer-space position.
    pub fn with_scale(&self, t: &LayerTransform, scale: Vec2, fixed: Vec2) -> LayerTransform {
        let at = self.to_layer(t, fixed);
        let mut out = LayerTransform { scale, ..*t };
        let pivot_pos = at - rot((fixed - self.pivot) * scale, t.rotation);
        out.translate = self.translate_for(pivot_pos);
        out
    }

    /// `t` turned by `angle` about the layer-space point `about` (the whole layer moves with it).
    pub fn rotated_about(&self, t: &LayerTransform, about: Vec2, angle: f32) -> LayerTransform {
        let pivot_pos = about + rot(self.pivot_pos(t) - about, angle);
        LayerTransform { translate: self.translate_for(pivot_pos), scale: t.scale, rotation: t.rotation + angle }
    }

    /// `t` mirrored at the vertical line `x = axis` (`horizontal`) or the horizontal line `y = axis`.
    pub fn mirrored(&self, t: &LayerTransform, horizontal: bool, axis: f32) -> LayerTransform {
        let p = self.pivot_pos(t);
        let (pivot_pos, scale) = if horizontal {
            (Vec2::new(2.0 * axis - p.x, p.y), Vec2::new(-t.scale.x, t.scale.y))
        } else {
            (Vec2::new(p.x, 2.0 * axis - p.y), Vec2::new(t.scale.x, -t.scale.y))
        };
        LayerTransform { translate: self.translate_for(pivot_pos), scale, rotation: -t.rotation }
    }

    /// `t` mirrored in place (about its own box centre): Flip Horizontal / Vertical of one layer.
    pub fn flipped(&self, t: &LayerTransform, horizontal: bool) -> LayerTransform {
        // Mirror about the box centre in the layer's own frame: negate the scale, keep the centre.
        let c = self.centre(t);
        let scale = if horizontal { Vec2::new(-t.scale.x, t.scale.y) } else { Vec2::new(t.scale.x, -t.scale.y) };
        let out = LayerTransform { scale, ..*t };
        let pivot_pos = c - rot((self.size * 0.5 - self.pivot) * scale, t.rotation);
        LayerTransform { translate: self.translate_for(pivot_pos), ..out }
    }
}

/// The artboard's centre (layer space).
pub fn artboard_centre(r: &IRect) -> Vec2 {
    Vec2::new((r.x0 as f32 + r.x1 as f32) * 0.5, (r.y0 as f32 + r.y1 as f32) * 0.5)
}

/// Resize (3.3): the artboard becomes `new` pixels (its top-left corner kept), every layer is scaled with it about
/// that corner: picture and text transforms, text sizes (`size_px` by the mean factor, the rest of the axis ratio in
/// the scale), shape sizes, outline widths and radii, layer style lengths, a rounded artboard's radius. Masks follow
/// their pictures; no pixel is resampled. `boxes` gives each layer's content box (`None`: the layer is left as it is).
/// `new` is clamped to 1..=4096 a side.
pub fn resize(doc: &mut StickerDoc, new: [u32; 2], boxes: &dyn Fn(&StickerLayer) -> Option<LayerBox>) {
    let [w, h] = doc.size();
    if w == 0 || h == 0 {
        return;
    }
    let new = new.map(|v| v.clamp(1, MAX_ARTBOARD));
    let f = Vec2::new(new[0] as f32 / w as f32, new[1] as f32 / h as f32);
    let mean = (f.x + f.y) * 0.5;
    let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
    for layer in &mut doc.layers {
        let Some(b) = boxes(layer) else { continue };
        let t = layer.transform;
        let pos = origin + (b.pivot_pos(&t) - origin) * f;
        match &mut layer.content {
            StickerContent::Picture(_) => {
                let t2 = LayerTransform { scale: t.scale * f, ..t };
                layer.transform = LayerTransform { translate: b.translate_for(pos), ..t2 };
            }
            StickerContent::Text(spec) => {
                spec.size_px *= mean;
                layer.transform = LayerTransform { translate: pos, scale: t.scale * (f / mean), rotation: t.rotation };
            }
            StickerContent::Shape(shape) => {
                shape.size *= f;
                match &mut shape.kind {
                    ShapeKind::RoundedRect { radius } => *radius *= mean,
                    ShapeKind::Line { width } => *width *= f.y,
                    _ => {}
                }
                if let Some((_, width)) = &mut shape.outline {
                    *width *= mean;
                }
                // The pivot (the box centre) of the scaled shape.
                let nb = LayerBox::centred(shape.size);
                layer.transform = LayerTransform { translate: nb.translate_for(pos), ..t };
            }
        }
        layer.styles = layer.styles.scaled(mean);
    }
    if let ArtboardShape::RoundedRect { radius } = &mut doc.shape {
        *radius *= mean;
    }
    doc.artboard = IRect::from_xywh(doc.artboard.x0, doc.artboard.y0, new[0], new[1]);
}

/// Rotate (3.3): every layer turned by `deg` degrees (clockwise as seen) about a pivot near the artboard centre, and
/// the artboard becomes the rotated artboard's bounding box. For a quarter or half turn the pivot is chosen so the new
/// artboard has whole-pixel corners and a pixel-aligned layer stays pixel-aligned (when the two sides differ in
/// parity the pivot is half a pixel off the centre, so that CW then CCW, and four quarter turns, are the identity). Any other angle turns about the exact centre and the bounding box is rounded
/// outwards.
pub fn rotate(doc: &mut StickerDoc, deg: f32, boxes: &dyn Fn(&StickerLayer) -> Option<LayerBox>) {
    if !deg.is_finite() {
        return;
    }
    let deg = deg.rem_euclid(360.0);
    if deg == 0.0 {
        return;
    }
    let r = doc.artboard;
    let c = artboard_centre(&r);
    // Sides of different parity: the pivot half a pixel off the centre, chosen per case so that CW and CCW undo each
    // other exactly (the same pivot) and four quarter turns come back to the start.
    let odd_w = r.width() % 2 == 1;
    let parity_differs = odd_w != (r.height() % 2 == 1);
    let off = |a: Vec2, b: Vec2| if !parity_differs { Vec2::ZERO } else if odd_w { a } else { b };
    let (pivot, angle, quarter) = match deg {
        90.0 => (c + off(Vec2::new(0.0, -0.5), Vec2::new(0.0, 0.5)), std::f32::consts::FRAC_PI_2, true),
        270.0 => (c + off(Vec2::new(-0.5, 0.0), Vec2::new(0.5, 0.0)), -std::f32::consts::FRAC_PI_2, true),
        180.0 => (c, std::f32::consts::PI, true),
        _ => (c, deg.to_radians(), false),
    };
    // Exact quarter turns: no rounding noise in the corners (sin / cos of pi/2 are not exactly 0 / 1 in f32).
    let turn = |v: Vec2| -> Vec2 {
        match deg {
            90.0 => Vec2::new(-v.y, v.x),
            270.0 => Vec2::new(v.y, -v.x),
            180.0 => -v,
            _ => rot(v, angle),
        }
    };
    for layer in &mut doc.layers {
        let Some(b) = boxes(layer) else { continue };
        let t = layer.transform;
        let pivot_pos = pivot + turn(b.pivot_pos(&t) - pivot);
        let mut rotation = t.rotation + angle;
        // Keep the angle small (a turn and back must come back to exactly 0).
        if quarter {
            let back = rotation.rem_euclid(std::f32::consts::TAU);
            rotation = if back > std::f32::consts::PI { back - std::f32::consts::TAU } else { back };
            if rotation.abs() < 1.0e-6 {
                rotation = 0.0;
            }
        }
        layer.transform = LayerTransform { translate: b.translate_for(pivot_pos), scale: t.scale, rotation };
    }
    let corners = [Vec2::new(r.x0 as f32, r.y0 as f32), Vec2::new(r.x1 as f32, r.y0 as f32), Vec2::new(r.x1 as f32, r.y1 as f32), Vec2::new(r.x0 as f32, r.y1 as f32)]
        .map(|p| pivot + turn(p - pivot));
    let min = corners.iter().fold(Vec2::splat(f32::INFINITY), |a, &b| a.min(b));
    let max = corners.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, &b| a.max(b));
    let (x0, y0, x1, y1) = if quarter {
        (min.x.round() as i32, min.y.round() as i32, max.x.round() as i32, max.y.round() as i32)
    } else {
        (min.x.floor() as i32, min.y.floor() as i32, max.x.ceil() as i32, max.y.ceil() as i32)
    };
    // The bounding box of a turned artboard can exceed the limit: it is cut to 4096 about its centre.
    let clamp_side = |a: i32, b: i32| -> (i32, i32) {
        let side = (b - a).max(1) as u32;
        if side <= MAX_ARTBOARD {
            return (a, a + side as i32);
        }
        let cut = ((side - MAX_ARTBOARD) / 2) as i32;
        (a + cut, a + cut + MAX_ARTBOARD as i32)
    };
    let (x0, x1) = clamp_side(x0, x1);
    let (y0, y1) = clamp_side(y0, y1);
    doc.artboard = IRect::new(x0, y0, x1, y1);
}

/// Flip Horizontal / Vertical of the whole sticker (3.3): every layer mirrored at the artboard's centre line; the
/// artboard is unchanged.
pub fn flip(doc: &mut StickerDoc, horizontal: bool, boxes: &dyn Fn(&StickerLayer) -> Option<LayerBox>) {
    let c = artboard_centre(&doc.artboard);
    let axis = if horizontal { c.x } else { c.y };
    for layer in &mut doc.layers {
        let Some(b) = boxes(layer) else { continue };
        layer.transform = b.mirrored(&layer.transform, horizontal, axis);
    }
}

/// Canvas Size (3.3): the artboard becomes `new` pixels, placed by `anchor` (0 = left / top, 0.5 = centre, 1 = right /
/// bottom of the old one); the layers keep their coordinates. `new` is clamped to 1..=4096 a side.
pub fn canvas_size(doc: &mut StickerDoc, new: [u32; 2], anchor: [f32; 2]) {
    let new = new.map(|v| v.clamp(1, MAX_ARTBOARD));
    let r = doc.artboard;
    let place = |a: i32, old: u32, new: u32, k: f32| -> i32 {
        let k = if k.is_finite() { k.clamp(0.0, 1.0) } else { 0.5 };
        a + ((old as f32 - new as f32) * k).round() as i32
    };
    let x0 = place(r.x0, r.width(), new[0], anchor[0]);
    let y0 = place(r.y0, r.height(), new[1], anchor[1]);
    doc.artboard = IRect::from_xywh(x0, y0, new[0], new[1]);
}

/// Add Padding (3.3): `px` more pixels on every side (at most 2048, the artboard limit kept).
pub fn pad(doc: &mut StickerDoc, px: u32) {
    let px = px.min(MAX_PADDING);
    let r = doc.artboard;
    let room = |side: u32| ((MAX_ARTBOARD.saturating_sub(side)) / 2).min(px) as i32;
    let (dx, dy) = (room(r.width()), room(r.height()));
    doc.artboard = IRect::new(r.x0 - dx, r.y0 - dy, r.x1 + dx, r.y1 + dy);
}

/// The artboard `rect` cut to the limit (each side at most 4096, kept about its own centre; at least 1).
pub fn limited(rect: IRect) -> IRect {
    // Check P8: in i64 (a rectangle from the API may span more than i32 can hold: `b - a` overflowed); a side that
    // would end past i32::MAX starts earlier.
    let side = |a: i32, b: i32| -> (i32, i32) {
        let (a, b) = (i64::from(a), i64::from(b));
        let len = (b - a).max(1);
        let (start, len) = if len <= i64::from(MAX_ARTBOARD) { (a, len) } else { (a + (len - i64::from(MAX_ARTBOARD)) / 2, i64::from(MAX_ARTBOARD)) };
        let start = start.min(i64::from(i32::MAX) - len);
        (start as i32, (start + len) as i32)
    };
    let (x0, x1) = side(rect.x0, rect.x1);
    let (y0, y1) = side(rect.y0, rect.y1);
    IRect::new(x0, y0, x1, y1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{BlobId, StickerLayerId};
    use crate::sticker::model::PicturePixels;
    use crate::sticker::shape::{Paint, ShapeSpec};

    fn picture_doc(w: u32, h: u32) -> StickerDoc {
        let mut d = StickerDoc::new("s", w, h);
        let id = d.alloc_layer();
        d.layers.push(StickerLayer::new(id, "p", StickerContent::Picture(PicturePixels::Blob(BlobId(1)))));
        d
    }

    fn boxes_of(size: [u32; 2]) -> impl Fn(&StickerLayer) -> Option<LayerBox> {
        move |l: &StickerLayer| match &l.content {
            StickerContent::Shape(s) => Some(LayerBox::centred(s.size)),
            _ => Some(LayerBox::centred(Vec2::new(size[0] as f32, size[1] as f32))),
        }
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).abs().max_element() < 1.0e-3
    }

    #[test]
    fn k8_a_box_maps_like_the_transform() {
        let b = LayerBox::centred(Vec2::new(40.0, 20.0));
        let t = LayerTransform { translate: Vec2::new(10.0, 5.0), scale: Vec2::new(2.0, -1.0), rotation: 0.3 };
        for q in [Vec2::ZERO, Vec2::new(40.0, 0.0), Vec2::new(13.0, 7.0)] {
            assert!(near(b.to_layer(&t, q), t.apply([40, 20], q)));
            assert!(near(b.to_content(&t, b.to_layer(&t, q)), q));
        }
        let turned = b.with_rotation(&t, 1.0);
        assert!(near(b.centre(&turned), b.centre(&t)), "the centre stays");
        let scaled = b.with_scale(&t, Vec2::new(3.0, 3.0), Vec2::ZERO);
        assert!(near(b.to_layer(&scaled, Vec2::ZERO), b.to_layer(&t, Vec2::ZERO)), "the fixed corner stays");
        let flipped = b.flipped(&t, true);
        assert!(near(b.centre(&flipped), b.centre(&t)));
        assert_eq!(b.flipped(&flipped, true), t);
        let text = LayerBox { size: Vec2::new(30.0, 10.0), pivot: Vec2::new(2.0, 8.0), text: true };
        let tt = LayerTransform::moved(50.0, 60.0);
        assert!(near(text.to_layer(&tt, Vec2::new(2.0, 8.0)), Vec2::new(50.0, 60.0)), "a text's anchor sits at translate");
    }

    #[test]
    fn k8_quarter_turns_keep_whole_pixels_and_undo_each_other() {
        for (w, h) in [(100, 60), (101, 60), (100, 61), (7, 7), (1, 2)] {
            let start = picture_doc(w, h);
            let boxes = boxes_of([w, h]);
            let mut d = start.clone();
            rotate(&mut d, 90.0, &boxes);
            assert_eq!(d.size(), [h, w]);
            let t = d.layers[0].transform;
            let corners = LayerBox::centred(Vec2::new(w as f32, h as f32)).bounds(&t);
            assert!(near(corners[0], Vec2::new(d.artboard.x0 as f32, d.artboard.y0 as f32)), "{w}x{h}: the picture fills the artboard ({corners:?} vs {:?})", d.artboard);
            rotate(&mut d, -90.0, &boxes);
            assert_eq!(d.artboard, start.artboard, "{w}x{h}");
            assert!(near(d.layers[0].transform.translate, Vec2::ZERO) && d.layers[0].transform.rotation == 0.0, "{w}x{h}: {:?}", d.layers[0].transform);
            let mut e = start.clone();
            rotate(&mut e, 180.0, &boxes);
            rotate(&mut e, 180.0, &boxes);
            assert_eq!(e.artboard, start.artboard);
            assert_eq!(e.layers[0].transform.rotation, 0.0);
            let mut f = start.clone();
            for _ in 0..4 {
                rotate(&mut f, 90.0, &boxes);
            }
            assert_eq!(f.artboard, start.artboard, "{w}x{h}: four turns");
            assert!(near(f.layers[0].transform.translate, Vec2::ZERO) && f.layers[0].transform.rotation == 0.0, "{w}x{h}: {:?}", f.layers[0].transform);
        }
    }

    #[test]
    fn k8_any_angle_bounds_the_turned_artboard() {
        let mut d = picture_doc(100, 100);
        rotate(&mut d, 45.0, &boxes_of([100, 100]));
        let side = 100.0 * std::f32::consts::SQRT_2;
        assert!((d.size()[0] as f32 - side).abs() <= 2.0, "{:?}", d.size());
        assert!(near(artboard_centre(&d.artboard), Vec2::splat(50.0)));
        let mut big = picture_doc(4000, 4000);
        rotate(&mut big, 45.0, &boxes_of([4000, 4000]));
        assert_eq!(big.size(), [4096, 4096], "the limit holds");
        let mut nan = picture_doc(10, 10);
        rotate(&mut nan, f32::NAN, &boxes_of([10, 10]));
        assert_eq!(nan, picture_doc(10, 10));
    }

    #[test]
    fn k8_resize_scales_everything_about_the_corner() {
        let mut d = picture_doc(100, 50);
        d.layers[0].transform = LayerTransform::moved(10.0, 5.0);
        d.layers[0].styles.stroke = Some(crate::sticker::style::Stroke {
            on: true,
            width: 4.0,
            position: crate::sticker::style::StrokePos::Outside,
            color: [0, 0, 0, 255],
            opacity: 1.0,
        });
        let sid = d.alloc_layer();
        let shape = ShapeSpec { kind: ShapeKind::RoundedRect { radius: 4.0 }, size: Vec2::new(20.0, 10.0), fill: Paint::Solid([1, 2, 3, 255]), outline: Some(([0; 4], 2.0)) };
        let mut sl = StickerLayer::new(sid, "s", StickerContent::Shape(shape));
        sl.transform = LayerTransform::moved(30.0, 20.0);
        d.layers.push(sl);
        d.shape = ArtboardShape::RoundedRect { radius: 6.0 };
        let before = LayerBox::centred(Vec2::new(100.0, 50.0)).to_layer(&d.layers[0].transform, Vec2::new(30.0, 30.0));
        resize(&mut d, [200, 100], &boxes_of([100, 50]));
        assert_eq!(d.size(), [200, 100]);
        let after = LayerBox::centred(Vec2::new(100.0, 50.0)).to_layer(&d.layers[0].transform, Vec2::new(30.0, 30.0));
        assert!(near(after, before * 2.0), "a content point moves with the artboard: {before:?} -> {after:?}");
        assert_eq!(d.layers[0].styles.stroke.as_ref().map(|s| s.width), Some(8.0));
        let StickerContent::Shape(s) = &d.layers[1].content else { panic!() };
        assert_eq!((s.size, s.kind.clone(), s.outline.map(|o| o.1)), (Vec2::new(40.0, 20.0), ShapeKind::RoundedRect { radius: 8.0 }, Some(4.0)));
        assert!(near(d.layers[1].transform.translate, Vec2::new(60.0, 40.0)));
        assert_eq!(d.shape, ArtboardShape::RoundedRect { radius: 12.0 });
        resize(&mut d, [100, 50], &boxes_of([100, 50]));
        assert!(near(d.layers[0].transform.translate, Vec2::new(10.0, 5.0)) && d.layers[0].transform.scale == Vec2::ONE);
        let mut zero = picture_doc(10, 10);
        resize(&mut zero, [0, 99_999], &boxes_of([10, 10]));
        assert_eq!(zero.size(), [1, 4096], "the sizes are clamped");
    }

    #[test]
    fn k8_flip_canvas_size_and_padding() {
        let mut d = picture_doc(100, 50);
        d.layers[0].transform = LayerTransform::moved(10.0, 0.0);
        flip(&mut d, true, &boxes_of([100, 50]));
        let t = d.layers[0].transform;
        assert!(near(t.translate, Vec2::new(-10.0, 0.0)) && t.scale == Vec2::new(-1.0, 1.0), "{t:?}");
        flip(&mut d, true, &boxes_of([100, 50]));
        assert_eq!(d.layers[0].transform, LayerTransform::moved(10.0, 0.0));
        canvas_size(&mut d, [120, 70], [0.5, 1.0]);
        assert_eq!(d.artboard, IRect::new(-10, -20, 110, 50));
        canvas_size(&mut d, [0, 0], [f32::NAN, 0.0]);
        assert_eq!(d.size(), [1, 1]);
        let mut p = picture_doc(10, 10);
        pad(&mut p, 5);
        assert_eq!(p.artboard, IRect::new(-5, -5, 15, 15));
        let mut q = picture_doc(4090, 10);
        pad(&mut q, 100);
        assert_eq!(q.size(), [4096, 210], "the limit holds on the long side");
        assert_eq!(limited(IRect::new(0, 0, 5000, 3)), IRect::new(452, 0, 4548, 3));
        let _ = StickerLayerId(0);
    }
}
