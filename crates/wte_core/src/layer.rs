//! Layers and texture sets (ARCHITECTURE.md 5.11).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ids::{LayerId, StampId, TextureSetId};
use crate::math::{DirtyRegion, IRect};
use crate::pixel::{CroppedBuf, PixelBuf, PixelView, SparseBuf};
use crate::texel_map::TexelMap;

/// Layer blend mode (kernels in `blend.rs`, section 6.6).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Add,
    Darken,
    Lighten,
}

/// Pixel storage of a layer.
#[derive(Clone, Debug)]
pub enum LayerKind {
    /// Paint layer; the buffer has exactly the set's size, or is `0 x 0` while the layer has never been painted
    /// (an unallocated, fully transparent layer: new paint layers cost no memory until the first stroke or fill).
    Raster(Arc<PixelBuf>),
    /// Baked image decal linked to a `StampObject`.
    Stamp { stamp: StampId, cache: Arc<SparseBuf> },
    /// Baked text decal linked to a `StampObject`.
    Text { stamp: StampId, cache: Arc<SparseBuf> },
}

impl LayerKind {
    /// An unallocated (transparent) paint layer.
    pub fn empty_raster() -> LayerKind {
        LayerKind::Raster(Arc::new(PixelBuf::new(0, 0)))
    }
}

/// One layer of a texture set.
#[derive(Clone, Debug)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub locked: bool,
    pub alpha_lock: bool,
    pub kind: LayerKind,
    /// Pixel-content version, see `Document::mark_layer_dirty` (5.14).
    pub(crate) version: u64,
    /// Paint layers: `(version, rect)` = at that `version` every pixel outside `rect` is `[0, 0, 0, 0]` (a conservative
    /// content bound, grown by `mark_layer_dirty`, recomputed by `Document::refresh_content_bounds`). Valid only while
    /// the version matches; compositing and dirty marking then skip the empty rest of the layer.
    pub(crate) content: Option<(u64, IRect)>,
}

impl Layer {
    /// Visible, opacity 1, Normal, unlocked, no alpha lock, version 0.
    pub fn new(id: LayerId, name: &str, kind: LayerKind) -> Layer {
        Layer {
            id,
            name: name.to_owned(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            locked: false,
            alpha_lock: false,
            kind,
            version: 0,
            content: None,
        }
    }

    /// Changes whenever this layer's pixels change (never reused within a document). Thumbnail caches key on (id, version).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The dense buffer of a paint layer (`0 x 0` while unallocated); `None` for stamp/text layers.
    pub fn raster(&self) -> Option<&PixelBuf> {
        match &self.kind {
            LayerKind::Raster(buf) => Some(buf),
            _ => None,
        }
    }

    /// The pixel views of the layer: the dense buffer of a paint layer (none while unallocated), the parts of a
    /// stamp/text cache. Pixels outside every view are transparent.
    pub fn views(&self) -> Vec<PixelView<'_>> {
        match &self.kind {
            LayerKind::Raster(buf) if buf.data.is_empty() => Vec::new(),
            LayerKind::Raster(buf) => vec![buf.view()],
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.parts().iter().map(CroppedBuf::view).collect(),
        }
    }

    /// Pixel at canvas position `(x, y)`; `[0; 4]` outside the stored pixels.
    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        match &self.kind {
            LayerKind::Raster(buf) => buf.view().get(x, y),
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.get(x, y),
        }
    }

    /// The layer as a dense `width x height` image.
    pub fn to_dense(&self, width: u32, height: u32) -> PixelBuf {
        match &self.kind {
            LayerKind::Raster(buf) if buf.width == width && buf.height == height => PixelBuf::clone(buf),
            LayerKind::Raster(buf) => CroppedBuf { rect: buf.rect(), data: buf.data.clone() }.to_dense(width, height),
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.to_dense(width, height),
        }
    }

    /// The rectangles outside of which the layer is transparent: the content bound of a paint layer (its whole
    /// buffer while the bound is unknown, nothing while unallocated), the part rectangles of a stamp/text cache.
    pub fn content_rects(&self) -> Vec<IRect> {
        match &self.kind {
            LayerKind::Raster(_) => {
                let r = self.content_rect();
                if r.is_empty() { Vec::new() } else { vec![r] }
            }
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.rects().collect(),
        }
    }

    /// Bounding box of `content_rects`.
    pub fn content_rect(&self) -> IRect {
        match &self.kind {
            LayerKind::Raster(buf) => match self.content {
                Some((version, rect)) if version == self.version => rect.intersect(&buf.rect()),
                _ => buf.rect(),
            },
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.bounds(),
        }
    }

    /// The known content bound of a paint layer, `None` when it is not known for the current version.
    pub(crate) fn known_content(&self) -> Option<IRect> {
        self.content.filter(|(v, _)| *v == self.version).map(|(_, r)| r)
    }

    /// The linked stamp of a stamp/text layer.
    pub fn stamp_id(&self) -> Option<StampId> {
        match &self.kind {
            LayerKind::Raster(_) => None,
            LayerKind::Stamp { stamp, .. } | LayerKind::Text { stamp, .. } => Some(*stamp),
        }
    }

    /// True for a paint layer.
    pub fn is_raster(&self) -> bool {
        matches!(self.kind, LayerKind::Raster(_))
    }

    /// `"paint"`, `"stamp"` or `"text"`.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            LayerKind::Raster(_) => "paint",
            LayerKind::Stamp { .. } => "stamp",
            LayerKind::Text { .. } => "text",
        }
    }

    /// Size of the layer's pixel storage in bytes.
    pub fn byte_len(&self) -> usize {
        match &self.kind {
            LayerKind::Raster(buf) => buf.byte_len(),
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.byte_len(),
        }
    }

    /// True when both layers share the very same pixel storage (`Arc::ptr_eq`).
    pub fn same_pixels(&self, other: &Layer) -> bool {
        match (&self.kind, &other.kind) {
            (LayerKind::Raster(a), LayerKind::Raster(b)) => Arc::ptr_eq(a, b),
            (LayerKind::Stamp { cache: a, .. }, LayerKind::Stamp { cache: b, .. })
            | (LayerKind::Text { cache: a, .. }, LayerKind::Text { cache: b, .. }) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// One paintable texture of the document: a layer stack plus its composite.
pub struct TextureSet {
    pub id: TextureSetId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Bottom -> top.
    pub layers: Vec<Layer>,
    pub active: Option<LayerId>,
    /// Built eagerly at document creation.
    pub texel_map: Arc<TexelMap>,
    /// What the 3D view, the preview and the flattened export show.
    pub(crate) composite: PixelBuf,
    /// Layer changes not yet composited.
    pub(crate) pending: DirtyRegion,
    /// Composited, not yet handed to the app.
    pub(crate) gpu_dirty: DirtyRegion,
    /// Per-set brush scratch, reused between strokes.
    pub(crate) scratch: Option<Box<crate::brush::StrokeScratch>>,
}

impl TextureSet {
    /// The composited pixels of the set (up to date once nothing is pending, see `Document::recomposite_all`).
    pub fn composite(&self) -> &PixelBuf {
        &self.composite
    }

    /// The rectangle `0,0 .. width,height`.
    pub fn full_rect(&self) -> IRect {
        IRect::from_size(self.width, self.height)
    }

    /// The layer with this id.
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Position of the layer in `layers` (0 = bottom).
    pub fn layer_index(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// The active layer, if any.
    pub fn active_layer(&self) -> Option<&Layer> {
        self.layer(self.active?)
    }

    /// Clips `r` to the set and adds it to `pending`.
    pub(crate) fn mark_dirty(&mut self, r: IRect) {
        let clipped = r.intersect(&self.full_rect());
        self.pending.add(clipped);
    }

    /// Marks the area the layer at `index` can change in the composite (its `content_rects`): for a property change,
    /// an added, removed or reordered layer.
    pub(crate) fn mark_layer_content_dirty(&mut self, index: usize) {
        let Some(layer) = self.layers.get(index) else { return };
        for r in layer.content_rects() {
            self.mark_dirty(r);
        }
    }

    /// Gives the unallocated paint layer at `index` its dense, transparent buffer (before the first stroke or fill
    /// writes to it). The pixels do not change: no version, no dirty rect. No-op for other layers.
    pub(crate) fn allocate_raster(&mut self, index: usize) {
        let (w, h) = (self.width, self.height);
        if let Some(LayerKind::Raster(buf)) = self.layers.get_mut(index).map(|l| &mut l.kind)
            && buf.data.is_empty()
            && w > 0
            && h > 0
        {
            *buf = Arc::new(PixelBuf::new(w, h));
        }
    }

    /// Marks every rectangle of `rects` (clipped to the set).
    pub(crate) fn mark_rects_dirty(&mut self, rects: &[IRect]) {
        for r in rects {
            self.mark_dirty(*r);
        }
    }
}
