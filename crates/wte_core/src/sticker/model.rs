//! The sticker document (STICKER_PLAN 2.2; ARCHITECTURE 5.27): a small layered picture with an artboard, a stack of
//! picture, text and shape layers, each with opacity, blend mode, transform, an optional mask, adjustments and
//! styles. Pure data: rendering is `sticker::render`, the document commands are `doc_stickers.rs`.
//!
//! Picture versions (2.2, review A8): every owned picture and every mask carries a `version` from
//! `Document::next_pixel_version` (monotonic for the life of the process, never reused, not saved). Equal versions
//! always mean equal bytes; caches key pixels by version (blob pictures by `BlobId`), never by address or content.
//! `PartialEq` compares the version first and the bytes only when the versions differ.

use std::sync::Arc;

use crate::error::EditError;
use crate::ids::{BlobId, StickerLayerId};
use crate::layer::BlendMode;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::sticker::adjust::Adjustment;
use crate::sticker::shape::ShapeSpec;
use crate::sticker::style::StyleStack;
use crate::sticker::{LayerTransform, MAX_ARTBOARD, MAX_OWNED_BYTES, MAX_PICTURE_SIDE, MAX_STICKER_LAYERS};
use crate::text::TextSpec;

/// The shape the artboard cuts out of the layer composite (E2, 4.1 step 3).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum ArtboardShape {
    #[default]
    Rect,
    /// Corner radius in artboard pixels (clamped to half the shorter side when drawn).
    RoundedRect { radius: f32 },
    Ellipse,
}

/// An 8-bit mask of a picture layer, the size of the picture; 255 = shown.
#[derive(Clone, Debug)]
pub struct LayerMask {
    pub pixels: Arc<AlphaBuf>,
    /// The identity of the bytes (module docs).
    pub version: u64,
}

impl PartialEq for LayerMask {
    fn eq(&self, other: &LayerMask) -> bool {
        self.version == other.version || Arc::ptr_eq(&self.pixels, &other.pixels) || *self.pixels == *other.pixels
    }
}

/// The pixels of a picture layer (straight RGBA8).
#[derive(Clone, Debug)]
pub enum PicturePixels {
    /// An imported image that was never changed: the blob's decoded pixels. Saved as the blob.
    Blob(BlobId),
    /// Edited pixels (saved as a PNG inside the project, P4).
    Owned { pixels: Arc<PixelBuf>, version: u64 },
}

impl PartialEq for PicturePixels {
    fn eq(&self, other: &PicturePixels) -> bool {
        match (self, other) {
            (PicturePixels::Blob(a), PicturePixels::Blob(b)) => a == b,
            (PicturePixels::Owned { pixels: a, version: va }, PicturePixels::Owned { pixels: b, version: vb }) => {
                va == vb || Arc::ptr_eq(a, b) || **a == **b
            }
            _ => false,
        }
    }
}

/// What a sticker layer shows.
#[derive(Clone, Debug, PartialEq)]
pub enum StickerContent {
    Picture(PicturePixels),
    /// `size_px` = one em in layer-space pixels at render scale 1; the frame's anchor sits at `transform.translate`.
    Text(Box<TextSpec>),
    Shape(ShapeSpec),
}

/// One layer of a sticker (2.2).
#[derive(Clone, Debug, PartialEq)]
pub struct StickerLayer {
    pub id: StickerLayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    /// 0..=1.
    pub opacity: f32,
    pub blend: BlendMode,
    /// Content pixels -> layer space. For a text layer the anchor sits at `translate` and scale and rotation turn about
    /// it; for pictures and shapes they turn about the content centre.
    pub transform: LayerTransform,
    pub content: StickerContent,
    /// Clipping mask: this layer shows only where its base shows (the nearest layer below with `clip == false`; the
    /// bottom layer's flag is ignored).
    pub clip: bool,
    /// Picture layers only: the size of the picture.
    pub mask: Option<LayerMask>,
    /// Applied in order to the content pixels, non-destructive.
    pub adjustments: Vec<Adjustment>,
    pub styles: StyleStack,
}

impl StickerLayer {
    /// A visible, unlocked layer at opacity 1, Normal, identity transform, no clip, mask, adjustment or style.
    pub fn new(id: StickerLayerId, name: &str, content: StickerContent) -> StickerLayer {
        StickerLayer {
            id,
            name: name.to_owned(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            transform: LayerTransform::IDENTITY,
            content,
            clip: false,
            mask: None,
            adjustments: Vec::new(),
            styles: StyleStack::default(),
        }
    }

    /// The owned picture of a picture layer.
    pub fn owned_picture(&self) -> Option<&Arc<PixelBuf>> {
        match &self.content {
            StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => Some(pixels),
            _ => None,
        }
    }

    /// Clamps every number into its range (opacity 0..=1, NaN -> 1; transform, styles, adjustments, shape, text).
    pub fn sanitize(&mut self) {
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(0.0, 1.0) } else { 1.0 };
        self.transform.sanitize();
        self.styles.sanitize();
        for a in &mut self.adjustments {
            a.sanitize();
        }
        match &mut self.content {
            StickerContent::Text(spec) => spec.sanitize_fx(),
            StickerContent::Shape(shape) => shape.sanitize(),
            StickerContent::Picture(_) => {}
        }
    }
}

/// A sticker: a small layered picture document (2.2). `Clone` shares the pixel buffers (`Arc`).
#[derive(Clone, Debug, PartialEq)]
pub struct StickerDoc {
    pub name: String,
    /// The artboard in layer space (pixels), each side 1..=`MAX_ARTBOARD`. Layers keep their coordinates when it changes.
    pub artboard: IRect,
    /// What the artboard cuts out of the layer composite.
    pub shape: ArtboardShape,
    /// Bottom to top; at most `MAX_STICKER_LAYERS`.
    pub layers: Vec<StickerLayer>,
    /// The next free layer id (every layer id is below it; ids are never reused).
    pub next_layer: u32,
    /// Shown in the tray and saved even when no stamp uses it (E4, E7).
    pub keep: bool,
}

impl StickerDoc {
    /// An empty sticker with an artboard of `width x height` at the origin.
    pub fn new(name: &str, width: u32, height: u32) -> StickerDoc {
        StickerDoc {
            name: name.to_owned(),
            artboard: IRect::from_size(width, height),
            shape: ArtboardShape::Rect,
            layers: Vec::new(),
            next_layer: 1,
            keep: false,
        }
    }

    /// The sticker of a promoted image stamp (2.8): one `Picture(Blob)` layer, the artboard = the image at the origin,
    /// identity transform. It renders the image's pixels unchanged (the trivial fast path of 4.1).
    pub fn from_picture(name: &str, picture: PicturePixels, size: [u32; 2]) -> StickerDoc {
        let mut doc = StickerDoc::new(name, size[0], size[1]);
        let id = doc.alloc_layer();
        doc.layers.push(StickerLayer::new(id, name, StickerContent::Picture(picture)));
        doc
    }

    /// A fresh layer id (`next_layer += 1`).
    pub fn alloc_layer(&mut self) -> StickerLayerId {
        let id = StickerLayerId(self.next_layer);
        self.next_layer += 1;
        id
    }

    /// The layer with this id.
    pub fn layer(&self, id: StickerLayerId) -> Option<&StickerLayer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Mutable access to the layer with this id.
    pub fn layer_mut(&mut self, id: StickerLayerId) -> Option<&mut StickerLayer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// Artboard size `[w, h]` (0 for a degenerate rectangle).
    pub fn size(&self) -> [u32; 2] {
        [self.artboard.width(), self.artboard.height()]
    }

    /// Bytes of every distinct owned picture and mask buffer.
    pub fn owned_bytes(&self) -> u64 {
        let mut seen: Vec<*const u8> = Vec::new();
        let mut total = 0u64;
        for l in &self.layers {
            let bufs = l.owned_picture().map(|p| (Arc::as_ptr(p) as *const u8, p.byte_len()));
            let mask = l.mask.as_ref().map(|m| (Arc::as_ptr(&m.pixels) as *const u8, m.pixels.byte_len()));
            for (ptr, len) in bufs.into_iter().chain(mask) {
                if !seen.contains(&ptr) {
                    seen.push(ptr);
                    total += len as u64;
                }
            }
        }
        total
    }

    /// The blobs this sticker needs: the images of `Picture(Blob)` layers and the fonts of text layers, ascending.
    pub fn blobs(&self) -> Vec<BlobId> {
        let mut out: Vec<BlobId> = self
            .layers
            .iter()
            .filter_map(|l| match &l.content {
                StickerContent::Picture(PicturePixels::Blob(b)) => Some(*b),
                StickerContent::Text(spec) => Some(spec.font.blob),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Clamps every number into its range (every layer's `sanitize`; a rounded artboard's radius finite and >= 0).
    pub fn sanitize(&mut self) {
        if let ArtboardShape::RoundedRect { radius } = &mut self.shape {
            *radius = if radius.is_finite() { radius.max(0.0) } else { 0.0 };
        }
        for l in &mut self.layers {
            l.sanitize();
        }
    }

    /// The limits of 2.7 that need no decoded pixels, each as `StickerLimit` / `InvalidParam`: artboard sides
    /// 1..=4096, at most 32 layers, unique layer ids below `next_layer`, owned pictures 1..=8192 px a side and at most
    /// 512 MiB together, masks only on picture layers and of the size of an owned picture. (A mask on a `Blob`
    /// picture is checked against the decoded size by the document.)
    pub fn check_limits(&self) -> Result<(), EditError> {
        let [w, h] = self.size();
        if w == 0 || h == 0 || self.artboard.is_empty() {
            return Err(EditError::StickerLimit("the artboard must be at least 1 x 1 pixels"));
        }
        if w > MAX_ARTBOARD || h > MAX_ARTBOARD {
            return Err(EditError::StickerLimit("a sticker's artboard can be at most 4096 pixels on a side"));
        }
        if self.layers.len() > MAX_STICKER_LAYERS {
            return Err(EditError::StickerLimit("a sticker can have at most 32 layers"));
        }
        let mut ids: Vec<u32> = self.layers.iter().map(|l| l.id.0).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|p| p[0] == p[1]) {
            return Err(EditError::InvalidParam("two sticker layers have the same id"));
        }
        if ids.last().is_some_and(|&last| last >= self.next_layer) {
            return Err(EditError::InvalidParam("a sticker layer id is not below next_layer"));
        }
        for l in &self.layers {
            if let Some(p) = l.owned_picture()
                && (p.width == 0
                    || p.height == 0
                    || p.width > MAX_PICTURE_SIDE
                    || p.height > MAX_PICTURE_SIDE
                    || p.data.len() != p.width as usize * p.height as usize * 4)
            {
                return Err(EditError::StickerLimit("a picture layer can be at most 8192 pixels on a side"));
            }
            if let Some(mask) = &l.mask {
                match &l.content {
                    StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => {
                        if (mask.pixels.width, mask.pixels.height) != (pixels.width, pixels.height)
                            || mask.pixels.data.len() != mask.pixels.width as usize * mask.pixels.height as usize
                        {
                            return Err(EditError::InvalidParam("a layer mask must have the size of its picture"));
                        }
                    }
                    StickerContent::Picture(PicturePixels::Blob(_)) => {}
                    _ => return Err(EditError::InvalidParam("only picture layers can have a mask")),
                }
            }
        }
        if self.owned_bytes() > MAX_OWNED_BYTES {
            return Err(EditError::StickerLimit("a sticker can hold at most 512 MB of edited pictures"));
        }
        Ok(())
    }

    /// The picture of the trivial fast path (4.1): exactly one visible layer, a picture with identity transform,
    /// opacity 1, Normal, no clip, no mask, no adjustments, default styles, artboard at the origin and shape `Rect`.
    /// The caller checks that the picture's size equals the artboard's.
    pub fn trivial_picture(&self) -> Option<&PicturePixels> {
        let mut visible = self.layers.iter().filter(|l| l.visible);
        let l = visible.next()?;
        if visible.next().is_some()
            || self.shape != ArtboardShape::Rect
            || self.artboard.x0 != 0
            || self.artboard.y0 != 0
            || !l.transform.is_identity()
            || l.opacity != 1.0
            || l.blend != BlendMode::Normal
            || l.clip
            || l.mask.is_some()
            || !l.adjustments.is_empty()
            || !l.styles.is_default()
        {
            return None;
        }
        match &l.content {
            StickerContent::Picture(p) => Some(p),
            _ => None,
        }
    }

    /// True when a render at scale 1 shows every layer at full resolution (2.6 "Sticker render scale"): every visible
    /// layer is a picture drawn at scale 1 or larger on both axes.
    pub fn scale_one_is_enough(&self) -> bool {
        self.layers.iter().filter(|l| l.visible).all(|l| {
            matches!(l.content, StickerContent::Picture(_)) && l.transform.scale.x.abs() >= 1.0 && l.transform.scale.y.abs() >= 1.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k3a_versions_decide_equality_before_bytes() {
        let a = Arc::new(PixelBuf::filled(2, 2, [1, 2, 3, 4]));
        let b = Arc::new(PixelBuf::filled(2, 2, [1, 2, 3, 4]));
        let c = Arc::new(PixelBuf::filled(2, 2, [9, 2, 3, 4]));
        let own = |p: &Arc<PixelBuf>, v| PicturePixels::Owned { pixels: Arc::clone(p), version: v };
        assert_eq!(own(&a, 1), own(&c, 1), "equal versions mean equal bytes (not read)");
        assert_eq!(own(&a, 1), own(&b, 2), "a loaded copy compares equal by its bytes");
        assert_ne!(own(&a, 1), own(&c, 2));
        assert_ne!(own(&a, 1), PicturePixels::Blob(BlobId(0)));
    }

    #[test]
    fn k3a_limits_are_checked() {
        let mut doc = StickerDoc::new("s", 10, 10);
        assert!(doc.check_limits().is_ok());
        doc.artboard = IRect::from_size(4097, 10);
        assert!(matches!(doc.check_limits(), Err(EditError::StickerLimit(_))));
        doc.artboard = IRect::from_size(10, 10);
        for _ in 0..33 {
            let id = doc.alloc_layer();
            doc.layers.push(StickerLayer::new(id, "l", StickerContent::Picture(PicturePixels::Blob(BlobId(0)))));
        }
        assert!(matches!(doc.check_limits(), Err(EditError::StickerLimit(_))));
        doc.layers.truncate(2);
        doc.layers[1].id = doc.layers[0].id;
        assert!(matches!(doc.check_limits(), Err(EditError::InvalidParam(_))));
    }
}
