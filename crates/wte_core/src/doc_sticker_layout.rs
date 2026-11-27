// Owner: stickers P8 (STICKER_PLAN 3.3 "Sticker" and "Layer" commands, 2.9, 2.10; ARCHITECTURE 5.20, 6.11). The
// layout commands of the Sticker view, each one history entry through `set_sticker_with` (an image stamp is promoted
// inside it, 2.10; the placement refit of 2.9 when the artboard changes):
// - Trim, Add Padding, Canvas Size, Crop (with its artboard shape and "Delete cropped pixels"): the artboard changes,
//   the layers keep their coordinates, every user's placement is refitted, so the artwork stays on its texel;
// - Rotate (90 / 180 / any angle), Flip: every layer turns or mirrors about the artboard, the refit follows the new
//   artboard (a quarter turn keeps whole pixels, `sticker::layout::rotate`);
// - Resize: the artboard and everything on it scaled, the placements **kept** (`Refit::Scaled`: the sticker keeps its
//   size on the model; the users' stamp-level style lengths scale with it), no picture resampled;
// - Flatten and Merge Down: rendered pixels (scale 1, artboard pixels) replace the layers;
// - the read-outs the editor needs: `sticker_layer_box` (where a layer's content box lands, for the transform
//   handles), `sticker_layered` (the sticker or an image stamp's virtual one-layer sticker) and `sticker_psd_bytes`
//   (Export as PSD: the link writer of 5.2 without a link).

use std::sync::Arc;

use glam::Vec2;

use crate::document::{Commit, Document};
use crate::doc_stickers::{Refit, StickerRef};
use crate::error::EditError;
use crate::ids::{StickerId, StickerLayerId};
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::pixel_ops::content_bounds;
use crate::sticker::layout::{self, LayerBox};
use crate::sticker::model::{ArtboardShape, LayerMask, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::sticker::style::StyleStack;
use crate::sticker::{LayerTransform, MAX_ARTBOARD};

/// History labels of the layout commands.
pub const TRIM_LABEL: &str = "Trim Sticker";
pub const PADDING_LABEL: &str = "Add Padding";
pub const CANVAS_SIZE_LABEL: &str = "Canvas Size";
pub const CROP_LABEL: &str = "Crop Sticker";
pub const RESIZE_LABEL: &str = "Resize Sticker";
pub const ROTATE_LABEL: &str = "Rotate Sticker";
pub const FLIP_H_LABEL: &str = "Flip Sticker Horizontal";
pub const FLIP_V_LABEL: &str = "Flip Sticker Vertical";
pub const FLATTEN_LABEL: &str = "Flatten Sticker";
pub const MERGE_DOWN_LABEL: &str = "Merge Down Sticker Layer";

impl Document {
    /// The layered document the editor shows for `on`: the sticker, or the virtual one-layer sticker of an image stamp
    /// that is not a sticker yet (layer id 1, the id its promotion gives it). Errors: `UnknownStamp`,
    /// `UnknownSticker`, `InvalidParam` (a text stamp), the image errors. Read only.
    pub fn sticker_layered(&mut self, on: StickerRef) -> Result<Arc<StickerDoc>, EditError> {
        match self.sticker_or_image(on)? {
            Ok(doc) => Ok(doc),
            Err(blob) => {
                let name = match on {
                    StickerRef::OfStamp(stamp, _) => self.stamp(stamp).map(|s| s.name.clone()).unwrap_or_default(),
                    StickerRef::Id(_) => String::new(),
                };
                let size = self.image_size(blob)?;
                Ok(Arc::new(StickerDoc::from_picture(&name, PicturePixels::Blob(blob), size)))
            }
        }
    }

    /// Where the content box of layer `layer` of `doc` lands (`sticker::layout::LayerBox`): a picture its pixels, a
    /// shape its size, a text its frame at `size_px` (the text engine draws at most 512 px per em; a larger em is the
    /// 512-px frame scaled up, as `render` draws it). `None` for a picture whose image cannot be read or a text that
    /// cannot be laid out.
    pub fn layer_box_of(&mut self, layer: &StickerLayer) -> Option<LayerBox> {
        match &layer.content {
            StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => {
                Some(LayerBox::centred(Vec2::new(pixels.width as f32, pixels.height as f32)))
            }
            StickerContent::Picture(PicturePixels::Blob(b)) => {
                let [w, h] = self.image_size(*b).ok()?;
                Some(LayerBox::centred(Vec2::new(w as f32, h as f32)))
            }
            StickerContent::Shape(shape) => Some(LayerBox::centred(shape.size)),
            StickerContent::Text(spec) => {
                let want = if spec.size_px.is_finite() && spec.size_px > 0.0 { spec.size_px } else { 32.0 };
                let mut drawn = (**spec).clone();
                drawn.size_px = want.clamp(8.0, 512.0);
                let frame = self.frame_of_spec(&drawn).ok()?;
                let em = if frame.em_px.is_finite() && frame.em_px > 0.0 { frame.em_px } else { drawn.size_px };
                let k = want / em;
                Some(LayerBox {
                    size: Vec2::new(frame.size[0] as f32, frame.size[1] as f32) * k,
                    pivot: Vec2::from_array(frame.anchor_px) * k,
                    text: true,
                })
            }
        }
    }

    /// `layer_box_of` for layer `layer` of the sticker `on` names (`sticker_layered`). Read only.
    pub fn sticker_layer_box(&mut self, on: StickerRef, layer: StickerLayerId) -> Result<LayerBox, EditError> {
        let doc = self.sticker_layered(on)?;
        let l = doc.layer(layer).ok_or(EditError::UnknownStickerLayer)?;
        self.layer_box_of(l).ok_or(EditError::InvalidParam("the layer has no content to place"))
    }

    /// The content box of every layer of `doc`, in order (`None` where `layer_box_of` has none).
    fn boxes_of(&mut self, doc: &StickerDoc) -> Vec<(StickerLayerId, Option<LayerBox>)> {
        doc.layers.iter().map(|l| (l.id, self.layer_box_of(l))).collect()
    }

    /// "Trim" (3.3): the artboard becomes the bounds of the visible content (the finished picture's pixels with alpha
    /// above 0, inside the artboard), grown by `padding` pixels on every side. One entry; the placements are refitted.
    /// `InvalidParam` when nothing is visible. Nothing happens (no entry) when the artboard is already that.
    pub fn trim_sticker(&mut self, on: StickerRef, padding: u32) -> Result<StickerId, EditError> {
        let doc = self.sticker_layered(on)?;
        let picture = self.render_unstored((*doc).clone())?;
        let bounds = content_bounds(picture.view()).ok_or(EditError::InvalidParam("the sticker shows nothing to trim to"))?;
        let p = padding.min(layout::MAX_PADDING) as i32;
        let (x, y) = (doc.artboard.x0, doc.artboard.y0);
        let rect = layout::limited(IRect::new(x + bounds.x0 - p, y + bounds.y0 - p, x + bounds.x1 + p, y + bounds.y1 + p));
        self.set_sticker_with(on, move |d| d.artboard = rect, TRIM_LABEL, Commit::Final, Refit::Artboard)
    }

    /// "Add Padding" (3.3): `px` more pixels on every side (`sticker::layout::pad`). One entry.
    pub fn pad_sticker(&mut self, on: StickerRef, px: u32) -> Result<StickerId, EditError> {
        self.set_sticker_with(on, move |d| layout::pad(d, px), PADDING_LABEL, Commit::Final, Refit::Artboard)
    }

    /// "Canvas Size" (3.3): the artboard becomes `size` pixels placed by `anchor` (`sticker::layout::canvas_size`).
    /// One entry.
    pub fn canvas_size_sticker(&mut self, on: StickerRef, size: [u32; 2], anchor: [f32; 2]) -> Result<StickerId, EditError> {
        self.set_sticker_with(on, move |d| layout::canvas_size(d, size, anchor), CANVAS_SIZE_LABEL, Commit::Final, Refit::Artboard)
    }

    /// The Crop tool's Apply (3.3, E2): the artboard becomes `rect` (cut to the limits) with the artboard shape
    /// `shape`; with `delete_pixels` every picture layer that is only moved (scale 1, no rotation, whole-pixel
    /// position) keeps only its pixels inside the new artboard (its mask with them; a layer left with nothing keeps one
    /// transparent pixel). One entry; the placements are refitted.
    pub fn crop_sticker(&mut self, on: StickerRef, rect: IRect, shape: ArtboardShape, delete_pixels: bool) -> Result<StickerId, EditError> {
        if rect.is_empty() {
            return Err(EditError::InvalidParam("the crop is empty"));
        }
        let rect = layout::limited(rect);
        let mut cut: Vec<(StickerLayerId, PixelBuf, Option<AlphaBuf>, Vec2)> = Vec::new();
        if delete_pixels {
            let doc = self.sticker_layered(on)?;
            for l in &doc.layers {
                let StickerContent::Picture(picture) = &l.content else { continue };
                let t = l.transform;
                if t.scale != Vec2::ONE || t.rotation != 0.0 || t.translate != t.translate.round() {
                    continue;
                }
                let pixels = match picture {
                    PicturePixels::Owned { pixels, .. } => Arc::clone(pixels),
                    PicturePixels::Blob(b) => self.blob_picture(*b)?,
                };
                let at = IRect::from_xywh(t.translate.x as i32, t.translate.y as i32, pixels.width, pixels.height);
                let keep = at.intersect(&rect);
                if keep == at {
                    continue;
                }
                let local = if keep.is_empty() {
                    IRect::from_xywh(0, 0, 1, 1)
                } else {
                    IRect::new(keep.x0 - at.x0, keep.y0 - at.y0, keep.x1 - at.x0, keep.y1 - at.y0)
                };
                let mut out = PixelBuf::new(local.width(), local.height());
                if !keep.is_empty() {
                    for y in 0..local.height() {
                        for x in 0..local.width() {
                            out.set(x, y, pixels.get(local.x0 as u32 + x, local.y0 as u32 + y));
                        }
                    }
                }
                let mask = l.mask.as_ref().filter(|m| (m.pixels.width, m.pixels.height) == (pixels.width, pixels.height)).map(|m| {
                    let mut a = AlphaBuf::filled(local.width(), local.height(), 255);
                    if !keep.is_empty() {
                        for y in 0..local.height() {
                            for x in 0..local.width() {
                                a.set(x, y, m.pixels.get(local.x0 as u32 + x, local.y0 as u32 + y));
                            }
                        }
                    }
                    a
                });
                let origin = if keep.is_empty() { Vec2::new(rect.x0 as f32, rect.y0 as f32) } else { Vec2::new(keep.x0 as f32, keep.y0 as f32) };
                cut.push((l.id, out, mask, origin));
            }
        }
        self.set_sticker_with(
            on,
            move |d| {
                d.artboard = rect;
                d.shape = shape;
                for (id, pixels, mask, origin) in cut {
                    if let Some(l) = d.layer_mut(id) {
                        l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(pixels), version: 0 });
                        l.mask = mask.map(|m| LayerMask { pixels: Arc::new(m), version: 0 });
                        l.transform.translate = origin;
                    }
                }
            },
            CROP_LABEL,
            Commit::Final,
            Refit::Artboard,
        )
    }

    /// "Resize..." (3.3): the artboard becomes `size` pixels and every layer is scaled with it
    /// (`sticker::layout::resize`); no picture is resampled. The users keep their placements (the sticker keeps its
    /// size on the model) and their stamp-level style lengths scale by the mean factor. One entry.
    pub fn resize_sticker(&mut self, on: StickerRef, size: [u32; 2]) -> Result<StickerId, EditError> {
        let doc = self.sticker_layered(on)?;
        let [w, h] = doc.size();
        let size = size.map(|v| v.clamp(1, MAX_ARTBOARD));
        let f = Vec2::new(size[0] as f32 / w.max(1) as f32, size[1] as f32 / h.max(1) as f32);
        if !(f.x >= layout::MIN_RESIZE && f.y >= layout::MIN_RESIZE && f.x <= layout::MAX_RESIZE && f.y <= layout::MAX_RESIZE) {
            return Err(EditError::InvalidParam("a resize goes from 1 % to 10,000 %"));
        }
        let boxes = self.boxes_of(&doc);
        let mean = (f.x + f.y) * 0.5;
        self.set_sticker_with(
            on,
            move |d| layout::resize(d, size, &|l: &StickerLayer| boxes.iter().find(|b| b.0 == l.id).and_then(|b| b.1)),
            RESIZE_LABEL,
            Commit::Final,
            Refit::Scaled(mean),
        )
    }

    /// "Rotate 90 CW / CCW / 180" and "Rotate..." (3.3): every layer turned by `deg` degrees clockwise about the
    /// artboard (`sticker::layout::rotate`); the artboard becomes the turned artboard's bounding box. One entry; the
    /// placements are refitted, so the sticker turns on the model about its centre.
    pub fn rotate_sticker(&mut self, on: StickerRef, deg: f32) -> Result<StickerId, EditError> {
        if !deg.is_finite() {
            return Err(EditError::InvalidParam("the angle is not a number"));
        }
        let doc = self.sticker_layered(on)?;
        let boxes = self.boxes_of(&doc);
        self.set_sticker_with(
            on,
            move |d| layout::rotate(d, deg, &|l: &StickerLayer| boxes.iter().find(|b| b.0 == l.id).and_then(|b| b.1)),
            ROTATE_LABEL,
            Commit::Final,
            Refit::Artboard,
        )
    }

    /// "Flip Horizontal / Vertical" of the sticker (3.3): every layer mirrored at the artboard's centre line. One entry.
    pub fn flip_sticker(&mut self, on: StickerRef, horizontal: bool) -> Result<StickerId, EditError> {
        let doc = self.sticker_layered(on)?;
        let boxes = self.boxes_of(&doc);
        let label = if horizontal { FLIP_H_LABEL } else { FLIP_V_LABEL };
        self.set_sticker_with(
            on,
            move |d| layout::flip(d, horizontal, &|l: &StickerLayer| boxes.iter().find(|b| b.0 == l.id).and_then(|b| b.1)),
            label,
            Commit::Final,
            Refit::Artboard,
        )
    }

    /// "Flatten" (3.3): all layers become one picture layer of the finished picture (scale 1, layer styles drawn in,
    /// hidden layers left out) at the artboard's corner. The artboard shape stays (the picture is drawn without it, so
    /// the sticker looks exactly as before). One entry. `InvalidParam` for a sticker with one plain layer already.
    pub fn flatten_sticker(&mut self, on: StickerRef) -> Result<StickerId, EditError> {
        let doc = self.sticker_layered(on)?;
        let plain = StickerDoc { shape: ArtboardShape::Rect, ..(*doc).clone() };
        if doc.layers.len() <= 1 && doc.trivial_picture().is_some() {
            return Err(EditError::InvalidParam("the sticker has one layer already"));
        }
        let picture = self.render_unstored(plain)?;
        let name = "Flattened".to_owned();
        let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
        self.set_sticker_with(
            on,
            move |d| {
                let id = d.alloc_layer();
                let mut layer = StickerLayer::new(id, &name, StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(picture), version: 0 }));
                layer.transform = LayerTransform::moved(origin.x, origin.y);
                d.layers = vec![layer];
            },
            FLATTEN_LABEL,
            Commit::Final,
            Refit::Artboard,
        )
    }

    /// "Merge Down" of sticker layer `layer` (3.3): it and the layer below become one picture layer of the two drawn
    /// together (each with its own opacity, blend, mask, adjustments and styles; the upper one clipped to the lower
    /// one when it is clipped and the lower one is not; when both are clipped they share a base further down, which
    /// clips the merged layer) over the artboard, at scale 1. The result takes the lower layer's name, place in the
    /// stack and clipping flag; Normal, full opacity, no mask or style. Content outside the artboard is not kept. One
    /// entry. Errors: `UnknownStickerLayer`, `InvalidParam` (the bottom layer, a locked or a hidden layer).
    pub fn merge_down_sticker_layer(&mut self, on: StickerRef, layer: StickerLayerId) -> Result<StickerId, EditError> {
        let doc = self.sticker_layered(on)?;
        let i = doc.layers.iter().position(|l| l.id == layer).ok_or(EditError::UnknownStickerLayer)?;
        if i == 0 {
            return Err(EditError::InvalidParam("the bottom layer has no layer below it"));
        }
        let (lower, upper) = (&doc.layers[i - 1], &doc.layers[i]);
        if lower.locked || upper.locked {
            return Err(EditError::InvalidParam("a locked layer cannot be merged"));
        }
        // Check P8: a hidden layer is not drawn, so merging it would throw its pixels away unasked (Photoshop refuses
        // a Merge Down onto a hidden layer too).
        if !lower.visible || !upper.visible {
            return Err(EditError::InvalidParam("a hidden layer cannot be merged: show it first"));
        }
        let lower_base = StickerLayer { clip: false, ..lower.clone() };
        // Check P8: when both layers are clipped (to the same base further down), the upper one was clipped to that
        // base, not to the lower layer: drawn as a pair it is not clipped; the merged layer keeps the lower one's clip,
        // so the base cuts both as before.
        let upper_drawn = StickerLayer { clip: upper.clip && !lower.clip, ..upper.clone() };
        let pair = StickerDoc { shape: ArtboardShape::Rect, layers: vec![lower_base, upper_drawn], ..(*doc).clone() };
        let picture = self.render_unstored(pair)?;
        let (lower_id, name, clip, visible) = (lower.id, lower.name.clone(), lower.clip, true);
        let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
        self.set_sticker_with(
            on,
            move |d| {
                let Some(at) = d.layers.iter().position(|l| l.id == lower_id) else { return };
                let mut merged = StickerLayer::new(lower_id, &name, StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(picture), version: 0 }));
                merged.transform = LayerTransform::moved(origin.x, origin.y);
                merged.clip = clip;
                merged.visible = visible;
                merged.blend = BlendMode::Normal;
                merged.styles = StyleStack::default();
                d.layers[at] = merged;
                d.layers.retain(|l| l.id != layer);
            },
            MERGE_DOWN_LABEL,
            Commit::Final,
            Refit::Artboard,
        )
    }

    /// "Export as PSD..." (3.8): the bytes of a PSD of the sticker `on` names, written as a link file would be (5.2,
    /// layer by layer at the link scale) without making a link. Read only.
    pub fn sticker_psd_bytes(&mut self, on: StickerRef) -> Result<Vec<u8>, EditError> {
        let doc = self.sticker_layered(on)?;
        let users: Vec<_> = match on {
            StickerRef::Id(id) => self.sticker_users(id),
            StickerRef::OfStamp(stamp, _) => vec![stamp],
        };
        let final_scale = users
            .iter()
            .filter_map(|u| self.stamp(*u).cloned())
            .map(|u| crate::sticker::pipeline::sticker_scale(&doc, self.texels_of(&u).y, crate::stamp::BakeQuality::Full))
            .fold(1.0f32, f32::max);
        let s = crate::link::link_scale(&doc, final_scale);
        let input = self.link_render_input(&doc)?;
        let made = crate::link::export(&input, s, None).map_err(|e| EditError::InvalidParam(psd_reason(&e)))?;
        crate::psd::write_sticker_to_vec(&made.psd).map_err(|e| EditError::InvalidParam(psd_reason(&e)))
    }
}

/// A fixed sentence for a PSD error (`EditError::InvalidParam` takes a static text).
fn psd_reason(e: &crate::psd::PsdError) -> &'static str {
    log::warn!("Export as PSD: {e}");
    "the sticker cannot be written as a PSD"
}
