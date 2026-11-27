//! Rendering a sticker (STICKER_PLAN 4.1; ARCHITECTURE 6.10): a pure function of `StickerRenderInput` (everything
//! resolved, `Send`), so a stamp bake job can render on its worker.
//!
//! For each visible layer, bottom to top: (a) the content pixels (picture -> adjustments -> mask; text drawn at
//! `size_px * s`; shape rasterized at `size * s`), (b) placed into output pixels with the layer transform (4.7),
//! (c) the layer's styles (4.3). The rasters are composited with the seven blend kernels and the layer opacity into an
//! output cleared to transparent; a clipped layer's alpha is multiplied by its base layer's step-(b) raster. Last the
//! artboard shape's coverage multiplies the alpha. The trivial fast path (one plain picture filling the artboard, at
//! scale 1) returns the picture's bytes unchanged.
//!
//! Determinism (ARCHITECTURE 4.7): fixed-order arithmetic; rayon only inside the resampler and `apply_styles`, which
//! are thread-count independent; layers are composited in order on the calling thread.

use std::sync::Arc;

use glam::Vec2;

use crate::blend::blend_row;
use crate::composite::opacity_u8;
use crate::ids::{BlobId, StickerLayerId};
use crate::math::{IRect, mul_u8};
use crate::pixel::{CroppedBuf, PixelBuf};
use crate::sticker::adjust::{Adjustment, apply_adjustments};
use crate::sticker::fx::apply_styles;
use crate::sticker::model::{ArtboardShape, PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::sticker::resample::transform_picture;
use crate::sticker::shape::{ShapeSpec, rasterize, sd_ellipse, sd_round_box};
use crate::sticker::style::StyleStack;
use crate::sticker::{LayerTransform, counters};
use crate::stamp_job::{OwnFace, text_fonts_of};
use crate::error::TextError;
use crate::text::{RasterQuality, TextSpec, render_text};

/// Most layer rasters the per-layer cache keeps (2.7).
const LAYER_CACHE_ENTRIES: usize = 64;
/// Most bytes the per-layer cache keeps (2.7).
const LAYER_CACHE_BYTES: usize = 256 << 20;
/// Smallest raster size of a text layer (pixels per em; a larger em is drawn smaller and scaled up).
const MIN_TEXT_PX: f32 = 8.0;
/// Largest raster size of a text layer (the text engine's `raster_px` limit).
const MAX_TEXT_PX: f32 = 512.0;

/// The fonts of one text layer: its own face (when its blob exists) and the bundled fallback.
pub type LayerFonts = (StickerLayerId, Option<OwnFace>, Arc<fontdue::Font>);

/// Everything `render` needs (2.7): the sticker value, the decoded pictures of its blob layers (straight RGBA8) and the
/// fonts of its text layers. `Send + 'static`.
#[derive(Clone)]
pub struct StickerRenderInput {
    pub doc: Arc<StickerDoc>,
    pub pictures: Vec<(BlobId, Arc<PixelBuf>)>,
    pub fonts: Vec<LayerFonts>,
}

const _: fn() = || {
    fn send_static<T: Send + Sync + 'static>() {}
    send_static::<StickerRenderInput>();
};

impl std::fmt::Debug for StickerRenderInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StickerRenderInput")
            .field("doc", &self.doc.name)
            .field("pictures", &self.pictures.iter().map(|p| p.0).collect::<Vec<_>>())
            .field("fonts", &self.fonts.len())
            .finish()
    }
}

/// Output size of a render at scale `s`: `ceil(artboard * s)` per axis, at least 1.
pub fn render_size(doc: &StickerDoc, s: f32) -> [u32; 2] {
    let s = if s.is_finite() && s > 0.0 { s } else { 1.0 };
    let [w, h] = doc.size();
    [((w as f32 * s).ceil() as u32).max(1), ((h as f32 * s).ceil() as u32).max(1)]
}

/// What identifies the content of one layer raster without reading pixels (2.7): blob pictures by `BlobId`, owned
/// pictures and masks by their version, texts and shapes by value.
#[derive(Clone, Debug, PartialEq)]
enum ContentKey {
    Blob(BlobId),
    Owned(u64),
    Text(Box<TextSpec>),
    Shape(ShapeSpec),
}

/// The full input of one layer raster (steps a-c of 4.1), compared by value; pixels enter only by version / blob id,
/// so building or comparing a key never reads pixel bytes (review A8).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerKey {
    content: ContentKey,
    mask: Option<u64>,
    transform: LayerTransform,
    adjustments: Vec<Adjustment>,
    styles: StyleStack,
    artboard: IRect,
    scale_bits: u32,
    quality: RasterQuality,
}

impl LayerKey {
    /// The key of `layer` in a sticker with artboard `artboard`, rendered at `s` and `quality`.
    pub fn of(layer: &StickerLayer, artboard: IRect, s: f32, quality: RasterQuality) -> LayerKey {
        let content = match &layer.content {
            StickerContent::Picture(PicturePixels::Blob(b)) => ContentKey::Blob(*b),
            StickerContent::Picture(PicturePixels::Owned { version, .. }) => ContentKey::Owned(*version),
            StickerContent::Text(spec) => ContentKey::Text(spec.clone()),
            StickerContent::Shape(shape) => ContentKey::Shape(shape.clone()),
        };
        LayerKey {
            content,
            mask: layer.mask.as_ref().map(|m| m.version),
            transform: layer.transform,
            adjustments: layer.adjustments.clone(),
            styles: layer.styles.clone(),
            artboard,
            scale_bits: s.to_bits(),
            quality,
        }
    }
}

/// One layer's rasters in output pixels: after step (b) (`content`: what a clipped layer above is cut by) and after
/// step (c) (`styled`; the same `Arc` without styles). `None` = nothing drawn.
#[derive(Clone, Debug, Default)]
pub struct LayerRaster {
    pub content: Option<Arc<CroppedBuf>>,
    pub styled: Option<Arc<CroppedBuf>>,
}

impl LayerRaster {
    fn bytes(&self) -> usize {
        let c = self.content.as_ref().map_or(0, |b| b.byte_len());
        let s = match (&self.content, &self.styled) {
            (Some(a), Some(b)) if Arc::ptr_eq(a, b) => 0,
            (_, Some(b)) => b.byte_len(),
            _ => 0,
        };
        c + s
    }
}

/// The per-layer raster cache of a document (2.7): least recently used last, at most 64 entries and 256 MiB.
#[derive(Default)]
pub struct LayerCache {
    entries: Vec<(LayerKey, LayerRaster)>,
    /// Lookups that found their raster (tests).
    pub hits: u64,
}

impl LayerCache {
    fn get(&mut self, key: &LayerKey) -> Option<LayerRaster> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(i);
        let raster = entry.1.clone();
        self.entries.push(entry);
        self.hits += 1;
        Some(raster)
    }

    fn insert(&mut self, key: LayerKey, raster: LayerRaster) {
        self.entries.retain(|(k, _)| *k != key);
        self.entries.push((key, raster));
        while self.entries.len() > LAYER_CACHE_ENTRIES
            || (self.entries.len() > 1 && self.entries.iter().map(|e| e.1.bytes()).sum::<usize>() > LAYER_CACHE_BYTES)
        {
            self.entries.remove(0);
        }
    }

    /// Number of cached rasters.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The picture of `sticker` at render scale `s` (output `render_size(doc, s)`, straight RGBA8), 4.1. Pure.
pub fn render(input: &StickerRenderInput, s: f32, quality: RasterQuality) -> PixelBuf {
    render_with(input, s, quality, None)
}

/// `render` that takes and fills a per-layer raster cache.
pub fn render_with(input: &StickerRenderInput, s: f32, quality: RasterQuality, mut cache: Option<&mut LayerCache>) -> PixelBuf {
    counters::rendered();
    let s = if s.is_finite() && s > 0.0 { s } else { 1.0 };
    let doc = &*input.doc;
    let [ow, oh] = render_size(doc, s);
    // The trivial fast path (identity contract of 4.1).
    if s == 1.0
        && let Some(picture) = doc.trivial_picture()
        && let Some(pixels) = picture_of(input, picture)
        && [pixels.width, pixels.height] == doc.size()
    {
        return PixelBuf::clone(pixels);
    }
    let mut out = PixelBuf::new(ow, oh);
    let out_rect = out.rect();
    let mut rasters: Vec<LayerRaster> = Vec::with_capacity(doc.layers.len());
    for (i, layer) in doc.layers.iter().enumerate() {
        // Step 1: the layer raster (also of hidden layers' bases only when needed: a hidden base hides its clipped
        // layers, so a hidden layer needs no raster at all).
        if !layer.visible {
            rasters.push(LayerRaster::default());
            continue;
        }
        let key = cache.as_ref().map(|_| LayerKey::of(layer, doc.artboard, s, quality));
        let raster = match (key.as_ref(), cache.as_deref_mut()) {
            (Some(k), Some(c)) => match c.get(k) {
                Some(r) => r,
                None => {
                    let r = layer_raster(input, layer, s, quality, out_rect);
                    c.insert(k.clone(), r.clone());
                    r
                }
            },
            _ => layer_raster(input, layer, s, quality, out_rect),
        };
        rasters.push(raster);
        // Step 2: composite.
        let Some(styled) = rasters[i].styled.clone() else { continue };
        let drawn: Arc<CroppedBuf> = if layer.clip && i > 0 {
            let base = (0..i).rev().find(|&j| !doc.layers[j].clip).unwrap_or(0);
            if !doc.layers[base].visible {
                continue;
            }
            let Some(cut) = rasters[base].content.as_ref() else { continue };
            Arc::new(clipped(&styled, cut))
        } else {
            styled
        };
        composite(&mut out, &drawn, opacity_u8(layer.opacity), layer.blend);
    }
    // Step 3: the artboard shape.
    apply_shape(&mut out, doc, s);
    out
}

/// Stickers P7a (STICKER_PLAN 3.5, review A6): the pixels of `region` (output pixels at render scale 1, clipped to
/// the artboard) of the sticker's picture, with layer `live` drawn **without its styles** (its content, adjustments,
/// mask and transform only: all local, so a dirty rectangle can be drawn again on its own). Every other layer comes
/// from its full raster (`cache`, the same entries `render_with` uses at scale 1 and `RasterQuality::Full`); the live
/// layer is never cached (its pixels change with every dab). With `live == None`, or a live layer without an enabled
/// style, every pixel that is seen equals the same pixel of `render(input, 1.0, Full)` (the region path of the editor
/// canvas during a stroke; a fully transparent pixel may carry another colour: a layer clipped to the live layer is
/// left out where the live layer draws nothing in the region, while the whole render composites it there at alpha 0).
/// Clipping and the artboard shape are per pixel, so they are drawn in the region too.
pub fn render_region(input: &StickerRenderInput, region: IRect, live: Option<StickerLayerId>, mut cache: Option<&mut LayerCache>) -> CroppedBuf {
    counters::region_rendered();
    let doc = &*input.doc;
    let out_rect = IRect::from_size(doc.size()[0], doc.size()[1]);
    let region = region.intersect(&out_rect);
    if region.is_empty() {
        return CroppedBuf::empty();
    }
    // The trivial fast path of `render_with` returns the picture's bytes: the same here.
    if let Some(picture) = doc.trivial_picture()
        && let Some(pixels) = picture_of(input, picture)
        && [pixels.width, pixels.height] == doc.size()
    {
        let mut out = CroppedBuf::new(region);
        let w = region.width() as usize * 4;
        for y in region.y0..region.y1 {
            let s = (y as usize * pixels.width as usize + region.x0 as usize) * 4;
            let d = (y - region.y0) as usize * w;
            out.data[d..d + w].copy_from_slice(&pixels.data[s..s + w]);
        }
        return out;
    }
    let mut out = CroppedBuf::new(region);
    let mut rasters: Vec<LayerRaster> = Vec::with_capacity(doc.layers.len());
    for (i, layer) in doc.layers.iter().enumerate() {
        if !layer.visible {
            rasters.push(LayerRaster::default());
            continue;
        }
        let raster = if Some(layer.id) == live {
            let content = live_content(input, layer, region).filter(|c| !c.is_empty()).map(Arc::new);
            LayerRaster { styled: content.clone(), content }
        } else {
            match cache.as_deref_mut() {
                Some(c) => {
                    let key = LayerKey::of(layer, doc.artboard, 1.0, RasterQuality::Full);
                    match c.get(&key) {
                        Some(r) => r,
                        None => {
                            let r = layer_raster(input, layer, 1.0, RasterQuality::Full, out_rect);
                            c.insert(key, r.clone());
                            r
                        }
                    }
                }
                None => layer_raster(input, layer, 1.0, RasterQuality::Full, out_rect),
            }
        };
        rasters.push(raster);
        let Some(styled) = rasters[i].styled.clone() else { continue };
        let cut = if layer.clip && i > 0 {
            let base = (0..i).rev().find(|&j| !doc.layers[j].clip).unwrap_or(0);
            if !doc.layers[base].visible {
                continue;
            }
            match rasters[base].content.clone() {
                Some(c) => Some(c),
                None => continue,
            }
        } else {
            None
        };
        composite_region(&mut out, &styled, cut.as_deref(), opacity_u8(layer.opacity), layer.blend);
    }
    apply_shape_region(&mut out, doc, out_rect);
    out
}

/// The live layer's content (step a and b with its mask) in `region`. A picture moved by whole pixels (scale 1, no
/// rotation) is cut to the region before its adjustments and mask are applied, so a dab costs the region and not the
/// picture (budget 8.4: 2 ms on a 2048-pixel layer); every other layer kind and transform takes the general path.
fn live_content(input: &StickerRenderInput, layer: &StickerLayer, region: IRect) -> Option<CroppedBuf> {
    let doc = &*input.doc;
    let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
    let t = layer.transform;
    let shift = t.translate - origin;
    let whole = t.scale == Vec2::ONE && t.rotation == 0.0 && shift.fract() == Vec2::ZERO && shift.abs().max_element() < 1.0e6;
    let StickerContent::Picture(picture) = &layer.content else {
        return content_raster(input, layer, 1.0, RasterQuality::Full, region, true);
    };
    if !whole {
        return content_raster(input, layer, 1.0, RasterQuality::Full, region, true);
    }
    let pixels = picture_of(input, picture)?;
    let (dx, dy) = (shift.x as i64, shift.y as i64);
    // The region in content pixels, clipped to the picture.
    let src = IRect::new(
        (region.x0 as i64 - dx).clamp(0, pixels.width as i64) as i32,
        (region.y0 as i64 - dy).clamp(0, pixels.height as i64) as i32,
        (region.x1 as i64 - dx).clamp(0, pixels.width as i64) as i32,
        (region.y1 as i64 - dy).clamp(0, pixels.height as i64) as i32,
    );
    if src.is_empty() {
        return None;
    }
    let (w, h) = (src.width(), src.height());
    let mut part = PixelBuf::new(w, h);
    let row = w as usize * 4;
    for y in 0..h as usize {
        let s = ((src.y0 as usize + y) * pixels.width as usize + src.x0 as usize) * 4;
        part.data[y * row..(y + 1) * row].copy_from_slice(&pixels.data[s..s + row]);
    }
    if !layer.adjustments.is_empty() {
        apply_adjustments(&mut part, &layer.adjustments);
    }
    if let Some(mask) = &layer.mask
        && (mask.pixels.width, mask.pixels.height) == (pixels.width, pixels.height)
    {
        for y in 0..h as usize {
            let m = (src.y0 as usize + y) * mask.pixels.width as usize + src.x0 as usize;
            for (p, &a) in part.data[y * row..(y + 1) * row].as_chunks_mut::<4>().0.iter_mut().zip(&mask.pixels.data[m..m + w as usize]) {
                p[3] = mul_u8(p[3], a);
            }
        }
    }
    let rect = IRect::new((src.x0 as i64 + dx) as i32, (src.y0 as i64 + dy) as i32, (src.x1 as i64 + dx) as i32, (src.y1 as i64 + dy) as i32);
    Some(CroppedBuf { rect, data: part.data })
}

/// `src` (cut by the content alpha of `cut` when given: a clipping mask) composited over `out` within `out.rect`.
fn composite_region(out: &mut CroppedBuf, src: &CroppedBuf, cut: Option<&CroppedBuf>, opacity: u8, mode: crate::layer::BlendMode) {
    let r = src.rect.intersect(&out.rect);
    if r.is_empty() || opacity == 0 {
        return;
    }
    let ow = out.rect.width() as usize * 4;
    let sw = src.rect.width() as usize * 4;
    let n = r.width() as usize * 4;
    let mut row = vec![0u8; n];
    for y in r.y0..r.y1 {
        let d = (y - out.rect.y0) as usize * ow + (r.x0 - out.rect.x0) as usize * 4;
        let s = (y - src.rect.y0) as usize * sw + (r.x0 - src.rect.x0) as usize * 4;
        let line = match cut {
            Some(base) => {
                row.copy_from_slice(&src.data[s..s + n]);
                for (k, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    p[3] = mul_u8(p[3], base.get(r.x0 + k as i32, y)[3]);
                }
                &row[..]
            }
            None => &src.data[s..s + n],
        };
        blend_row(&mut out.data[d..d + n], line, opacity, mode);
    }
}

/// Step 3 of 4.1 inside `out.rect` at scale 1 (`full` = the whole output): the same coverage per pixel as
/// `apply_shape`.
fn apply_shape_region(out: &mut CroppedBuf, doc: &StickerDoc, full: IRect) {
    let (w, h) = (full.width() as f32, full.height() as f32);
    let c = Vec2::new(w, h) * 0.5;
    let sd: Box<dyn Fn(Vec2) -> f32> = match doc.shape {
        ArtboardShape::Rect => return,
        ArtboardShape::RoundedRect { radius } => {
            let r = radius.clamp(0.0, w.min(h) * 0.5);
            Box::new(move |p| sd_round_box(p, c, c, r))
        }
        ArtboardShape::Ellipse => Box::new(move |p| sd_ellipse(p, c, c)),
    };
    let rect = out.rect;
    let width = rect.width() as usize;
    for (i, p) in out.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let q = Vec2::new((rect.x0 + (i % width) as i32) as f32 + 0.5, (rect.y0 + (i / width) as i32) as f32 + 0.5);
        let cov = (0.5 - sd(q)).clamp(0.0, 1.0);
        if cov < 1.0 {
            p[3] = (p[3] as f32 * cov + 0.5) as u8;
        }
    }
}

/// The decoded pixels of a picture layer.
fn picture_of<'a>(input: &'a StickerRenderInput, picture: &'a PicturePixels) -> Option<&'a Arc<PixelBuf>> {
    match picture {
        PicturePixels::Blob(b) => input.pictures.iter().find(|(id, _)| id == b).map(|(_, p)| p),
        PicturePixels::Owned { pixels, .. } => Some(pixels),
    }
}

/// Steps (a) and (b) of 4.1 for one layer **without its mask and without its styles**, in output pixels (artboard
/// origin at 0, scale `s`), clipped to `clip`: the pixels a linked PSD gets for the layer (STICKER_PLAN 5.4 "WTE ->
/// PSD"; the mask is written separately, `mask_raster`). `None` when the layer draws nothing.
pub fn layer_content(input: &StickerRenderInput, layer: &StickerLayer, s: f32, quality: RasterQuality, clip: IRect) -> Option<CroppedBuf> {
    content_raster(input, layer, s, quality, clip, false).filter(|c| !c.is_empty())
}

/// The alpha (0..=255) that layer `layer` draws at the layer-space point `p` at render scale 1, its mask applied and
/// before its styles, its opacity and clipping: the hit test of the editor's Move tool (STICKER_PLAN 3.3, stickers
/// P6a). 0 outside its content and for a point that is not finite.
pub fn layer_alpha_at(input: &StickerRenderInput, layer: &StickerLayer, p: Vec2) -> u8 {
    if !p.is_finite() {
        return 0;
    }
    let doc = &*input.doc;
    // Output pixels at scale 1 have the artboard's top-left corner at 0.
    let x = (p.x - doc.artboard.x0 as f32).floor() as i32;
    let y = (p.y - doc.artboard.y0 as f32).floor() as i32;
    let clip = IRect::new(x, y, x.saturating_add(1), y.saturating_add(1));
    let Some(c) = content_raster(input, layer, 1.0, RasterQuality::Full, clip, true).filter(|c| !c.is_empty()) else {
        return 0;
    };
    if !c.rect.contains(x, y) {
        return 0;
    }
    let i = ((y - c.rect.y0) as usize * c.rect.width() as usize + (x - c.rect.x0) as usize) * 4 + 3;
    c.data.get(i).copied().unwrap_or(0)
}

/// The mask of a picture layer placed like its picture (`layer_content`): an output-pixel buffer whose red channel is
/// the mask value wherever the picture lands (alpha > 0 there); `None` without a mask (or when the mask does not have
/// the picture's size).
pub fn mask_raster(input: &StickerRenderInput, layer: &StickerLayer, s: f32, clip: IRect) -> Option<CroppedBuf> {
    let mask = layer.mask.as_ref()?;
    let StickerContent::Picture(picture) = &layer.content else { return None };
    let pixels = picture_of(input, picture)?;
    if (mask.pixels.width, mask.pixels.height) != (pixels.width, pixels.height) {
        return None;
    }
    let mut grey = PixelBuf::new(mask.pixels.width, mask.pixels.height);
    for (p, &v) in grey.data.as_chunks_mut::<4>().0.iter_mut().zip(&mask.pixels.data) {
        *p = [v, v, v, 255];
    }
    let doc = &*input.doc;
    let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
    let t = layer.transform;
    let moved = LayerTransform { translate: t.translate - origin, ..t };
    Some(transform_picture(&grey, &moved, s, Some(clip))).filter(|c| !c.is_empty())
}

/// Steps (a) to (c) of 4.1 for one layer, in output pixels (artboard origin at 0, scale `s`).
fn layer_raster(input: &StickerRenderInput, layer: &StickerLayer, s: f32, quality: RasterQuality, out_rect: IRect) -> LayerRaster {
    // Styles can reach in from outside the output: keep that much of the content.
    let m = layer.styles.margin(s) as i32;
    let clip = IRect::new(out_rect.x0 - m, out_rect.y0 - m, out_rect.x1 + m, out_rect.y1 + m);
    let content = content_raster(input, layer, s, quality, clip, true);
    let Some(content) = content.filter(|c| !c.is_empty()) else { return LayerRaster::default() };
    let content = Arc::new(content);
    if !layer.styles.any_on() {
        return LayerRaster { content: Some(Arc::clone(&content)), styled: Some(content) };
    }
    let r = content.rect;
    let base = PixelBuf { width: r.width(), height: r.height(), data: content.data.clone() };
    let styled = apply_styles(&base, &layer.styles, s);
    let m = ((styled.width - base.width) / 2) as i32;
    let rect = IRect::new(r.x0 - m, r.y0 - m, r.x0 - m + styled.width as i32, r.y0 - m + styled.height as i32);
    LayerRaster { content: Some(content), styled: Some(Arc::new(CroppedBuf { rect, data: styled.data })) }
}

/// Steps (a) and (b) of 4.1 for one layer (its mask applied when `with_mask`), clipped to `clip`.
fn content_raster(
    input: &StickerRenderInput,
    layer: &StickerLayer,
    s: f32,
    quality: RasterQuality,
    clip: IRect,
    with_mask: bool,
) -> Option<CroppedBuf> {
    let doc = &*input.doc;
    let origin = Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
    let t = layer.transform;
    match &layer.content {
        StickerContent::Picture(picture) => picture_of(input, picture).map(|pixels| {
            let mut src: std::borrow::Cow<'_, PixelBuf> = std::borrow::Cow::Borrowed(&**pixels);
            if !layer.adjustments.is_empty() {
                apply_adjustments(src.to_mut(), &layer.adjustments);
            }
            if with_mask
                && let Some(mask) = &layer.mask
                && (mask.pixels.width, mask.pixels.height) == (src.width, src.height)
            {
                let buf = src.to_mut();
                for (p, &a) in buf.data.as_chunks_mut::<4>().0.iter_mut().zip(&mask.pixels.data) {
                    p[3] = mul_u8(p[3], a);
                }
            }
            let moved = LayerTransform { translate: t.translate - origin, ..t };
            transform_picture(&src, &moved, s, Some(clip))
        }),
        StickerContent::Text(spec) => text_content(input, layer.id, spec, &t, origin, s, quality, &layer.adjustments, clip),
        StickerContent::Shape(shape) => {
            let raster = rasterize(shape, s);
            (!raster.is_empty()).then(|| {
                let r = raster.rect;
                let mut pic = PixelBuf { width: r.width(), height: r.height(), data: raster.data };
                if !layer.adjustments.is_empty() {
                    apply_adjustments(&mut pic, &layer.adjustments);
                }
                // Content box [0, size * s) of the raster; `r` may start before it (a centred outline).
                let pivot_in = shape.size * s * 0.5 - Vec2::new(r.x0 as f32, r.y0 as f32);
                let pivot_out = (t.translate + shape.size * 0.5 - origin) * s;
                place(&pic, pivot_in, pivot_out, t.scale, t.rotation, clip)
            })
        }
    }
}

/// A text layer's content (4.1 a, b): drawn at `size_px * s` (within the text engine's 8..=512 px per em; a larger em
/// is drawn at 512 and scaled up, a picture over the engine's limits is drawn smaller and scaled up), its anchor
/// placed at `(translate - origin) * s`, scale and rotation about the anchor.
#[allow(clippy::too_many_arguments)]
fn text_content(
    input: &StickerRenderInput,
    id: StickerLayerId,
    spec: &TextSpec,
    t: &LayerTransform,
    origin: Vec2,
    s: f32,
    quality: RasterQuality,
    adjustments: &[Adjustment],
    clip: IRect,
) -> Option<CroppedBuf> {
    let (_, own, fallback) = input.fonts.iter().find(|f| f.0 == id)?;
    let fonts = text_fonts_of(own, fallback);
    let target = if spec.size_px.is_finite() && spec.size_px > 0.0 { spec.size_px * s } else { 32.0 * s };
    let mut px = target.clamp(MIN_TEXT_PX, MAX_TEXT_PX);
    let raster = loop {
        let mut drawn = spec.clone();
        drawn.size_px = px;
        match render_text(&fonts, &drawn, quality) {
            Ok(r) => break r,
            Err(TextError::TooLarge { .. }) if px > MIN_TEXT_PX => px = (px / 2.0).max(MIN_TEXT_PX),
            Err(e) => {
                log::warn!("sticker text layer {}: cannot be drawn: {e}", id.0);
                return None;
            }
        }
    };
    let mut pic = raster.image;
    if pic.width == 0 || pic.height == 0 {
        return None;
    }
    if !adjustments.is_empty() {
        apply_adjustments(&mut pic, adjustments);
    }
    // The picture may have fewer pixels per em than asked (engine limit, preview): scale it up to the target. Check
    // P3a: a Preview picture of a large styled text is drawn smaller than its frame (FEATURE_TEXT 5.8, `full ==
    // false`), whose em and anchor are those of the full picture: both follow the picture's own size.
    let f = if raster.frame.size[0] > 0 && pic.width != raster.frame.size[0] {
        Vec2::new(pic.width as f32 / raster.frame.size[0] as f32, pic.height as f32 / raster.frame.size[1].max(1) as f32)
    } else {
        Vec2::ONE
    };
    let em = if raster.frame.em_px > 0.0 && raster.frame.em_px.is_finite() { raster.frame.em_px * f.x } else { px };
    let extra = target / em;
    let pivot_in = Vec2::from_array(raster.frame.anchor_px) * f;
    let pivot_out = (t.translate - origin) * s;
    Some(place(&pic, pivot_in, pivot_out, t.scale * extra, t.rotation, clip))
}

/// `pic` placed so that its point `pivot_in` (picture pixels) lands on `pivot_out` (output pixels), scaled by `scale`
/// and turned by `rotation` about that point (4.7 resampler at scale 1).
fn place(pic: &PixelBuf, pivot_in: Vec2, pivot_out: Vec2, scale: Vec2, rotation: f32, clip: IRect) -> CroppedBuf {
    let c = Vec2::new(pic.width as f32, pic.height as f32) * 0.5;
    let (sin, cos) = rotation.sin_cos();
    let q = (c - pivot_in) * scale;
    let rq = Vec2::new(q.x * cos - q.y * sin, q.x * sin + q.y * cos);
    let t = LayerTransform { translate: pivot_out - c + rq, scale, rotation };
    transform_picture(pic, &t, 1.0, Some(clip))
}

/// `raster` with its alpha multiplied by the alpha of `base` at the same output pixel (a clipping mask).
fn clipped(raster: &CroppedBuf, base: &CroppedBuf) -> CroppedBuf {
    let mut out = raster.clone();
    let r = out.rect;
    let w = r.width() as usize;
    for (i, p) in out.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (r.x0 + (i % w) as i32, r.y0 + (i / w) as i32);
        p[3] = mul_u8(p[3], base.get(x, y)[3]);
    }
    out
}

/// Composites `src` over `out` (both output pixels) with `opacity` and `mode`, clipped to `out`.
fn composite(out: &mut PixelBuf, src: &CroppedBuf, opacity: u8, mode: crate::layer::BlendMode) {
    let r = src.rect.intersect(&out.rect());
    if r.is_empty() || opacity == 0 {
        return;
    }
    let stride = out.width as usize * 4;
    let sw = src.rect.width() as usize * 4;
    let n = r.width() as usize * 4;
    for y in r.y0..r.y1 {
        let d = y as usize * stride + r.x0 as usize * 4;
        let s = (y - src.rect.y0) as usize * sw + (r.x0 - src.rect.x0) as usize * 4;
        blend_row(&mut out.data[d..d + n], &src.data[s..s + n], opacity, mode);
    }
}

/// Step 3 of 4.1: multiplies the alpha by the coverage of the artboard shape at scale `s`.
fn apply_shape(out: &mut PixelBuf, doc: &StickerDoc, s: f32) {
    let (w, h) = (out.width as f32, out.height as f32);
    let c = Vec2::new(w, h) * 0.5;
    let sd: Box<dyn Fn(Vec2) -> f32> = match doc.shape {
        ArtboardShape::Rect => return,
        ArtboardShape::RoundedRect { radius } => {
            let r = (radius * s).clamp(0.0, w.min(h) * 0.5);
            Box::new(move |p| sd_round_box(p, c, c, r))
        }
        ArtboardShape::Ellipse => Box::new(move |p| sd_ellipse(p, c, c)),
    };
    let width = out.width as usize;
    for (i, p) in out.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let q = Vec2::new((i % width) as f32 + 0.5, (i / width) as f32 + 0.5);
        let cov = (0.5 - sd(q)).clamp(0.0, 1.0);
        if cov < 1.0 {
            p[3] = (p[3] as f32 * cov + 0.5) as u8;
        }
    }
}
