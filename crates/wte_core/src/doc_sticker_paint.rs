// Owner: stickers P7a (STICKER_PLAN 3.3, 3.5, review A6; ARCHITECTURE 5.20, 6.10). What the editor's painting needs
// from the document beside the strokes of `doc_sticker_edits.rs`:
// - the canvas during a stroke: `sticker_canvas_region` (a dirty rectangle drawn again with the stroke's layer
//   unstyled, the other layers from the layer cache);
// - render jobs off the UI thread (`StickerRenderJob`): the canvas's restyle at the end of a stroke on a styled layer
//   (`sticker_canvas_job`, scale 1) and the model's live Preview during a stroke (`sticker_preview_job`, the users'
//   Preview scale), applied with `apply_sticker_preview`;
// - read-outs for the tools: the composite colour under the Eyedropper (`sticker_color_at`), a layer's pixels and mask
//   (`sticker_layer_pixels`, `sticker_layer_mask`) for Fill, Gradient and the mask commands;
// - `rasterize_sticker_layer`: a text or shape layer becomes a picture layer (the paint tools offer it, never silently).
//
// Choices the plan leaves open, documented at the items below:
// - A job renders a snapshot of the sticker (an `Arc` clone). The next dab then copies the painted buffer once
//   (copy-on-write); a Preview job starts at most every 100 ms (the app's throttle), so a stroke copies a buffer at most
//   ten times a second.
// - `apply_sticker_preview` bakes the users at Preview quality from the job's picture: the picture is put into the
//   render cache under the sticker's current value for that one bake and dropped right after it, so no later lookup can
//   take a picture of an earlier state (2.7, review A8).

use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::Vec2;

use crate::doc_stickers::{RenderEntry, StickerRef};
use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::ids::{StickerId, StickerLayerId};
use crate::math::IRect;
use crate::pixel::{AlphaBuf, CroppedBuf, PixelBuf};
use crate::pixel_ops::PremulMips;
use crate::stamp::{BakeQuality, StampSource};
use crate::sticker::model::{PicturePixels, StickerContent, StickerDoc};
use crate::sticker::pipeline::sticker_scale;
use crate::sticker::render::{StickerRenderInput, layer_content, render, render_region};
use crate::sticker::{LayerTransform, MAX_CREATED_SIDE};
use crate::text::RasterQuality;

/// History label of `rasterize_sticker_layer`.
pub const RASTERIZE_LABEL: &str = "Rasterize Sticker Layer";

/// What a `StickerRenderJob` is for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StickerRenderPurpose {
    /// The editor canvas at the end of a stroke on a styled layer: scale 1, full quality, with every layer's styles.
    Canvas,
    /// The model during a stroke: the users' Preview render scale (`apply_sticker_preview`).
    Preview,
}

/// A render of a sticker off the UI thread (stickers P7a). `Send + 'static`.
pub struct StickerRenderJob {
    sticker: StickerId,
    purpose: StickerRenderPurpose,
    input: StickerRenderInput,
    scale: f32,
    quality: RasterQuality,
}

const _: fn() = || {
    fn send_static<T: Send + 'static>() {}
    send_static::<StickerRenderJob>();
    send_static::<StickerRenderOutput>();
};

/// The result of a `StickerRenderJob`.
pub struct StickerRenderOutput {
    pub sticker: StickerId,
    pub purpose: StickerRenderPurpose,
    /// The sticker value the picture was drawn from (the snapshot the job took).
    pub doc: Arc<StickerDoc>,
    pub scale: f32,
    /// Straight RGBA8, `render_size(doc, scale)`.
    pub picture: Arc<PixelBuf>,
    /// The picture's mips (Preview jobs only: the bake samples them; empty for a canvas job).
    pub mips: Option<Arc<PremulMips>>,
    /// Worker time.
    pub elapsed: Duration,
}

impl StickerRenderJob {
    /// The sticker the job draws.
    pub fn sticker(&self) -> StickerId {
        self.sticker
    }

    /// The sticker value the job draws (its snapshot).
    pub fn doc(&self) -> &Arc<StickerDoc> {
        &self.input.doc
    }

    /// Draws the picture (on any thread; deterministic: `sticker::render`).
    pub fn run(self) -> StickerRenderOutput {
        let start = Instant::now();
        let picture = render(&self.input, self.scale, self.quality);
        let mips = (self.purpose == StickerRenderPurpose::Preview).then(|| Arc::new(PremulMips::build(&picture)));
        StickerRenderOutput {
            sticker: self.sticker,
            purpose: self.purpose,
            doc: self.input.doc,
            scale: self.scale,
            picture: Arc::new(picture),
            mips,
            elapsed: start.elapsed(),
        }
    }
}

impl Document {
    // ----- the canvas during a stroke (3.5, review A6) -----

    /// The pixels of `region` (artboard pixels at scale 1, clipped to the artboard) of sticker `id`'s picture with
    /// layer `live` drawn without its styles (`render::render_region`; the other layers come from the document's
    /// layer cache). Read only, never a history command. `UnknownSticker` for an unknown id.
    pub fn sticker_canvas_region(&mut self, id: StickerId, region: IRect, live: Option<StickerLayerId>) -> Result<CroppedBuf, EditError> {
        let doc = Arc::clone(self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?);
        let input = self.sticker_render_input(&doc)?;
        Ok(render_region(&input, region, live, Some(&mut self.sticker_state.cache.layers)))
    }

    /// The canvas's restyle job (3.5 "at the stroke's end"): sticker `id` at scale 1, full quality, every layer with its
    /// styles (the same picture as `sticker_picture(id, None, 1.0)`). `UnknownSticker` for an unknown id.
    pub fn sticker_canvas_job(&mut self, id: StickerId) -> Result<StickerRenderJob, EditError> {
        let doc = Arc::clone(self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?);
        let input = self.sticker_render_input(&doc)?;
        Ok(StickerRenderJob { sticker: id, purpose: StickerRenderPurpose::Canvas, input, scale: 1.0, quality: RasterQuality::Full })
    }

    /// The model's live Preview during a stroke (3.5): sticker `id` with its layer styles at the largest Preview render
    /// scale of its users (2.6). `Ok(None)` when no stamp shows the sticker (nothing to bake).
    pub fn sticker_preview_job(&mut self, id: StickerId) -> Result<Option<StickerRenderJob>, EditError> {
        let doc = Arc::clone(self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?);
        let users = self.sticker_users(id);
        let scale = users
            .iter()
            .filter_map(|u| self.stamp(*u))
            .map(|s| sticker_scale(&doc, self.texels_of(s).y, BakeQuality::Preview))
            .fold(0.0f32, f32::max);
        if users.is_empty() || !(scale.is_finite() && scale > 0.0) {
            return Ok(None);
        }
        let input = self.sticker_render_input(&doc)?;
        let quality = crate::doc_stamps::raster_quality(BakeQuality::Preview);
        Ok(Some(StickerRenderJob { sticker: id, purpose: StickerRenderPurpose::Preview, input, scale, quality }))
    }

    /// Bakes the users of the Preview job's sticker at Preview quality from the job's picture (3.5: the model follows
    /// the stroke). Only while a stroke on that sticker is open (else `false` and nothing happens: the stroke's end has
    /// its Final bake). The picture serves this one bake (put into the render cache under the sticker's current value
    /// and dropped after it). No history.
    pub fn apply_sticker_preview(&mut self, out: StickerRenderOutput) -> bool {
        let open = self.sticker_stroke_target().is_some_and(|(s, _)| s == out.sticker);
        let (Some(mips), true, StickerRenderPurpose::Preview) = (out.mips, open, out.purpose) else { return false };
        let Some(current) = self.sticker_state.map.get(&out.sticker).cloned() else { return false };
        self.forget_sticker_pictures(out.sticker);
        self.sticker_state.cache.insert_render(RenderEntry {
            doc: current,
            scale_bits: out.scale.to_bits(),
            quality: BakeQuality::Preview,
            picture: out.picture,
            mips,
        });
        self.rebake_sticker_users(out.sticker, BakeQuality::Preview);
        self.forget_sticker_pictures(out.sticker);
        true
    }

    // ----- read-outs of the paint tools -----

    /// The colour of the finished sticker (all visible layers, their styles, scale 1) at the layer-space point `p`:
    /// the editor's Eyedropper (3.3). An image stamp that is not a sticker yet: its picture. `None` outside the
    /// artboard, for an unknown sticker or stamp, a text stamp, or a point that is not finite. Read only.
    pub fn sticker_color_at(&mut self, on: StickerRef, p: Vec2) -> Option<[u8; 4]> {
        if !p.is_finite() {
            return None;
        }
        let (picture, origin) = match on {
            StickerRef::OfStamp(stamp, _) => match self.stamp(stamp)?.source.clone() {
                StampSource::Image { blob } => (self.blob_picture(blob).ok()?, Vec2::ZERO),
                StampSource::Sticker { sticker } => self.sticker_picture_at_one(sticker)?,
                StampSource::Text(_) => return None,
            },
            StickerRef::Id(id) => self.sticker_picture_at_one(id)?,
        };
        let q = (p - origin).floor();
        if q.x < 0.0 || q.y < 0.0 || q.x >= picture.width as f32 || q.y >= picture.height as f32 {
            return None;
        }
        Some(picture.get(q.x as u32, q.y as u32))
    }

    /// The render of sticker `id` at scale 1 (cached) and the artboard's top-left corner.
    fn sticker_picture_at_one(&mut self, id: StickerId) -> Option<(Arc<PixelBuf>, Vec2)> {
        let doc = Arc::clone(self.sticker_state.map.get(&id)?);
        let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
        Some((self.sticker_render(&doc, 1.0, BakeQuality::Full).ok()?.picture, origin))
    }

    /// The pixels (straight RGBA8, shared) of picture layer `layer` of the sticker `on` names; an image stamp that is
    /// not a sticker yet: its picture as the virtual layer 1. Errors: `UnknownStamp` / `UnknownSticker` /
    /// `UnknownStickerLayer`, `InvalidParam` (a text stamp, a text or shape layer), the image errors of a blob.
    pub fn sticker_layer_pixels(&mut self, on: StickerRef, layer: StickerLayerId) -> Result<Arc<PixelBuf>, EditError> {
        let doc = match self.sticker_or_image(on)? {
            Ok(doc) => doc,
            Err(blob) => {
                return if layer == StickerLayerId(1) { self.blob_picture(blob) } else { Err(EditError::UnknownStickerLayer) };
            }
        };
        let l = doc.layer(layer).ok_or(EditError::UnknownStickerLayer)?;
        match &l.content {
            StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => Ok(Arc::clone(pixels)),
            StickerContent::Picture(PicturePixels::Blob(b)) => self.blob_picture(*b),
            _ => Err(EditError::InvalidParam("only a picture layer has pixels to paint")),
        }
    }

    /// The mask of picture layer `layer` (shared), `None` without one (an image stamp's virtual layer never has one).
    /// Errors as `sticker_layer_pixels`.
    pub fn sticker_layer_mask(&mut self, on: StickerRef, layer: StickerLayerId) -> Result<Option<Arc<AlphaBuf>>, EditError> {
        let doc = match self.sticker_or_image(on)? {
            Ok(doc) => doc,
            Err(_) if layer == StickerLayerId(1) => return Ok(None),
            Err(_) => return Err(EditError::UnknownStickerLayer),
        };
        let l = doc.layer(layer).ok_or(EditError::UnknownStickerLayer)?;
        Ok(l.mask.as_ref().map(|m| Arc::clone(&m.pixels)))
    }

    /// The sticker `on` names (`Ok(Ok(doc))`), or the blob of an image stamp that is not a sticker yet (`Ok(Err(blob))`).
    fn sticker_or_image(&self, on: StickerRef) -> Result<Result<Arc<StickerDoc>, crate::ids::BlobId>, EditError> {
        let id = match on {
            StickerRef::Id(id) => id,
            StickerRef::OfStamp(stamp, _) => match &self.stamp(stamp).ok_or(EditError::UnknownStamp)?.source {
                StampSource::Sticker { sticker } => *sticker,
                StampSource::Image { blob } => return Ok(Err(*blob)),
                StampSource::Text(_) => return Err(EditError::InvalidParam("a text stamp has no layers")),
            },
        };
        Ok(Ok(Arc::clone(self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?)))
    }

    // ----- Rasterize (3.3 layer-kind rule) -----

    /// "Rasterize": text or shape layer `layer` becomes a picture layer with the same look: its content (adjustments
    /// included; they are cleared from the layer) drawn at scale 1 and placed by a whole-pixel move; name, visibility,
    /// opacity, blend, clip and styles are kept. The pixels kept reach up to 1024 pixels beyond the artboard (at most
    /// `MAX_CREATED_SIDE` a side, else the artboard). One entry "Rasterize Sticker Layer"; an image stamp is promoted
    /// inside it (it has only a picture layer, so that never happens in practice). Errors: as `set_sticker`,
    /// `UnknownStickerLayer`, `InvalidParam` (already a picture layer, or nothing drawn).
    pub fn rasterize_sticker_layer(&mut self, on: StickerRef, layer: StickerLayerId) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        let doc = match self.sticker_or_image(on)? {
            Ok(doc) => doc,
            Err(_) => return Err(EditError::InvalidParam("the layer is a picture already")),
        };
        let l = doc.layer(layer).ok_or(EditError::UnknownStickerLayer)?.clone();
        if matches!(l.content, StickerContent::Picture(_)) {
            return Err(EditError::InvalidParam("the layer is a picture already"));
        }
        if l.locked {
            return Err(EditError::LayerLocked);
        }
        let input = self.sticker_render_input(&doc)?;
        let [w, h] = doc.size();
        let reach = 1024i32;
        let mut clip = IRect::new(-reach, -reach, w as i32 + reach, h as i32 + reach);
        if clip.width() > MAX_CREATED_SIDE || clip.height() > MAX_CREATED_SIDE {
            clip = IRect::from_size(w, h);
        }
        let drawn = layer_content(&input, &l, 1.0, RasterQuality::Full, clip)
            .filter(|c| !c.is_empty())
            .ok_or(EditError::InvalidParam("the layer draws nothing to rasterize"))?;
        let r = drawn.rect;
        let pixels = PixelBuf { width: r.width(), height: r.height(), data: drawn.data };
        let at = Vec2::new((doc.artboard.x0 + r.x0) as f32, (doc.artboard.y0 + r.y0) as f32);
        self.set_sticker(
            on,
            move |d| {
                if let Some(l) = d.layer_mut(layer) {
                    l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(pixels), version: 0 });
                    l.transform = LayerTransform::moved(at.x, at.y);
                    l.adjustments.clear();
                    l.mask = None;
                }
            },
            RASTERIZE_LABEL,
            Commit::Final,
        )
    }
}
