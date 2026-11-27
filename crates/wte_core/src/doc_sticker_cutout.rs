// Owner: stickers P7b (STICKER_PLAN 3.3, 3.4, 4.4; ARCHITECTURE 5.20, 6.12). The read-outs the editor's selection and
// cut-out tools need from the document beside those of `doc_sticker_paint.rs`:
// - `sticker_layer_on_artboard`: one layer drawn alone over the artboard (the Magic Wand and Color Range with "Sample:
//   active layer", "Selection from layer alpha");
// - `sticker_unstyled_composite`: every visible layer without its styles (the wand's "Sample: all layers", 4.4);
// - `sticker_picture_with`: the finished picture with one layer's pixels or mask replaced, the document unchanged (the
//   live preview of the Color to Alpha, Blur and Sharpen popovers: what Apply would give, before it is applied).
// All three are read only, never a history command, and render at scale 1 (artboard pixels) without the layer cache
// (a preview's buffer is drawn once and must not push the document's cached layers out).

use std::sync::Arc;

use crate::doc_sticker_edits::PixelData;
use crate::doc_stickers::StickerRef;
use crate::document::Document;
use crate::error::EditError;
use crate::ids::StickerLayerId;
use crate::layer::BlendMode;
use crate::pixel::PixelBuf;
use crate::sticker::style::StyleStack;
use crate::sticker::model::{LayerMask, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::sticker::render::render;
use crate::text::RasterQuality;

impl Document {
    /// Layer `layer` of the sticker `on` names drawn alone over the whole artboard at scale 1: shown, full opacity,
    /// Normal, not clipped, without its styles; its content, transform, mask and adjustments as they are. The pixel
    /// `(0, 0)` is the artboard's top-left corner. An image stamp that is not a sticker yet: its picture (the virtual
    /// layer 1). Errors: `UnknownStamp` / `UnknownSticker` / `UnknownStickerLayer`, `InvalidParam` (a text stamp), the
    /// image and text errors of the layer's content. Read only.
    pub fn sticker_layer_on_artboard(&mut self, on: StickerRef, layer: StickerLayerId) -> Result<PixelBuf, EditError> {
        let doc = match self.sticker_or_image(on)? {
            Ok(doc) => doc,
            Err(blob) => {
                return if layer == StickerLayerId(1) { Ok((*self.blob_picture(blob)?).clone()) } else { Err(EditError::UnknownStickerLayer) };
            }
        };
        let one = StickerLayer {
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            clip: false,
            styles: StyleStack::default(),
            ..doc.layer(layer).ok_or(EditError::UnknownStickerLayer)?.clone()
        };
        let alone = StickerDoc { layers: vec![one], ..(*doc).clone() };
        self.render_unstored(alone)
    }

    /// The composite of every visible layer of the sticker `on` names, each without its styles (STICKER_PLAN 4.4
    /// "Sample all layers"), at scale 1 over the artboard. An image stamp that is not a sticker yet: its picture.
    /// Errors as `sticker_layer_on_artboard`. Read only.
    pub fn sticker_unstyled_composite(&mut self, on: StickerRef) -> Result<PixelBuf, EditError> {
        let doc = match self.sticker_or_image(on)? {
            Ok(doc) => doc,
            Err(blob) => return Ok((*self.blob_picture(blob)?).clone()),
        };
        let mut plain = (*doc).clone();
        for l in &mut plain.layers {
            l.styles = StyleStack::default();
        }
        self.render_unstored(plain)
    }

    /// The finished picture (scale 1, layer styles included, no stamp styles) of the sticker `on` names with the
    /// pixels (`PixelData::Rgba`) or the mask (`PixelData::Alpha`) of picture layer `layer` replaced by `new`; the
    /// document is not changed. An image stamp that is not a sticker yet: `new` itself (its one layer has no mask).
    /// Errors: as `sticker_layer_on_artboard`; `InvalidParam` for a text or shape layer, a mask on a layer without one
    /// or of another size, and empty data. Read only. (`sticker_picture_with_job` + `StickerPictureWith::run`.)
    pub fn sticker_picture_with(&mut self, on: StickerRef, layer: StickerLayerId, new: PixelData) -> Result<PixelBuf, EditError> {
        if !new.is_valid() {
            return Err(EditError::InvalidParam("the new pixels are empty or do not match their size"));
        }
        self.sticker_picture_with_job(on, layer)?.run(new)
    }

    /// P7b check: `sticker_picture_with` in two halves, so a live preview draws off the UI thread: this half (UI
    /// thread, cheap: the sticker's render input, `Arc`s) checks the sticker and the layer; `StickerPictureWith::run`
    /// (any thread) puts the new pixels in and draws. Errors: as `sticker_picture_with` except those of `new`. Read
    /// only.
    pub fn sticker_picture_with_job(&mut self, on: StickerRef, layer: StickerLayerId) -> Result<StickerPictureWith, EditError> {
        let doc = match self.sticker_or_image(on)? {
            Ok(doc) => doc,
            Err(_) if layer != StickerLayerId(1) => return Err(EditError::UnknownStickerLayer),
            Err(_) => return Ok(StickerPictureWith { draw: None }),
        };
        let l = doc.layer(layer).ok_or(EditError::UnknownStickerLayer)?;
        if !matches!(l.content, StickerContent::Picture(_)) {
            return Err(EditError::InvalidParam("only a picture layer has pixels"));
        }
        let version = self.next_pixel_version();
        let input = self.sticker_render_input(&doc)?;
        Ok(StickerPictureWith { draw: Some((input, layer, version)) })
    }

    /// Renders `doc` at scale 1, full quality, outside the caches.
    pub(crate) fn render_unstored(&mut self, doc: StickerDoc) -> Result<PixelBuf, EditError> {
        let input = self.sticker_render_input(&Arc::new(doc))?;
        Ok(render(&input, 1.0, RasterQuality::Full))
    }
}

/// P7b check: the second half of `Document::sticker_picture_with` (`Document::sticker_picture_with_job`): the sticker's
/// render input, the layer and a pixel version taken on the UI thread; `run` works on any thread.
pub struct StickerPictureWith {
    /// `None`: an image stamp that is not a sticker yet (the picture is the new pixels themselves).
    draw: Option<(crate::sticker::render::StickerRenderInput, StickerLayerId, u64)>,
}

const _: fn() = || {
    fn send_static<T: Send + 'static>() {}
    send_static::<StickerPictureWith>();
};

impl StickerPictureWith {
    /// The finished picture with the layer's pixels or mask replaced by `new` (deterministic: `sticker::render`).
    /// Errors: `InvalidParam` for empty data, a mask on a layer without one or of another size, a mask of an image stamp.
    pub fn run(self, new: PixelData) -> Result<PixelBuf, EditError> {
        if !new.is_valid() {
            return Err(EditError::InvalidParam("the new pixels are empty or do not match their size"));
        }
        let Some((mut input, layer, version)) = self.draw else {
            return match new {
                PixelData::Rgba(p) => Ok(p),
                PixelData::Alpha(_) => Err(EditError::InvalidParam("the picture has no mask")),
            };
        };
        let mut changed = (*input.doc).clone();
        let l = changed.layer_mut(layer).ok_or(EditError::UnknownStickerLayer)?;
        match new {
            PixelData::Rgba(p) => l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(p), version }),
            PixelData::Alpha(a) => match &mut l.mask {
                Some(m) if [m.pixels.width, m.pixels.height] == [a.width, a.height] => *m = LayerMask { pixels: Arc::new(a), version },
                _ => return Err(EditError::InvalidParam("the layer has no mask of that size")),
            },
        }
        input.doc = Arc::new(changed);
        Ok(render(&input, 1.0, RasterQuality::Full))
    }
}
