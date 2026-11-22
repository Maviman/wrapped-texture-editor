// Owner: core-document (C2). Layer compositing (ARCHITECTURE.md 5.16, algorithm 6.6): `composite_rect`,
// `flatten_layers` and the document's composite cache (`recomposite_all`, `take_gpu_dirty`).

use rayon::prelude::*;

use crate::blend::blend_row;
use crate::document::Document;
use crate::ids::TextureSetId;
use crate::layer::{BlendMode, Layer, LayerKind, TextureSet};
use crate::math::IRect;
use crate::pixel::{PixelBuf, PixelView};
use crate::pixel_ops::nonzero_bounds_par;

/// Rects smaller than this are composited on the calling thread (a brush dab is not worth a trip through the pool).
const PARALLEL_MIN_PIXELS: u64 = 32 * 1024;

/// Layer opacity quantized to u8 once per layer: `round(opacity * 255)`; NaN counts as 0.
pub(crate) fn opacity_u8(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// One pixel view of a layer that takes part in a composite (a paint layer gives one, a stamp/text cache one per
/// part; the parts of one layer are disjoint, so folding them one after the other equals folding the layer once).
struct Contributor<'a> {
    view: PixelView<'a>,
    /// The part of `view.rect` that can hold non-zero pixels (a paint layer's content bound).
    clip: IRect,
    opacity: u8,
    blend: BlendMode,
}

/// The contributors of `layers` (bottom -> top) that reach `area`.
fn contributors(layers: &[Layer], include_hidden: bool, area: IRect) -> Vec<Contributor<'_>> {
    let mut out = Vec::new();
    for l in layers.iter().filter(|l| include_hidden || (l.visible && l.opacity > 0.0)) {
        let bound = l.content_rect().intersect(&area);
        if bound.is_empty() {
            continue;
        }
        for view in l.views() {
            let clip = view.rect.intersect(&bound);
            if !clip.is_empty() {
                out.push(Contributor { view, clip, opacity: opacity_u8(l.opacity), blend: l.blend });
            }
        }
    }
    out
}

/// Recomputes the span `x0..x1` of one full-width row: clear, then fold the layers bottom to top.
fn composite_row(row: &mut [u8], y: i32, x0: i32, x1: i32, layers: &[Contributor<'_>]) {
    row[x0 as usize * 4..x1 as usize * 4].fill(0);
    for layer in layers {
        // A layer contributes nothing outside its own content.
        let clip = layer.clip;
        if y < clip.y0 || y >= clip.y1 {
            continue;
        }
        let (xa, xb) = (x0.max(clip.x0), x1.min(clip.x1));
        if xa >= xb {
            continue;
        }
        let Some(src) = layer.view.row(y) else {
            continue;
        };
        let rect = layer.view.rect;
        let src = &src[(xa - rect.x0) as usize * 4..(xb - rect.x0) as usize * 4];
        blend_row(&mut row[xa as usize * 4..xb as usize * 4], src, layer.opacity, layer.blend);
    }
}

fn composite_rect_impl(
    layers: &[Layer],
    width: u32,
    height: u32,
    rect: IRect,
    include_hidden: bool,
    out: &mut PixelBuf,
    parallel: bool,
) {
    let r = rect.intersect(&IRect::from_size(width.min(out.width), height.min(out.height)));
    if r.is_empty() {
        return;
    }
    let layers = contributors(layers, include_hidden, r);
    let stride = out.width as usize * 4;
    let rows = &mut out.data[r.y0 as usize * stride..r.y1 as usize * stride];
    if parallel {
        rows.par_chunks_mut(stride)
            .enumerate()
            .for_each(|(i, row)| composite_row(row, r.y0 + i as i32, r.x0, r.x1, &layers));
    } else {
        rows.chunks_mut(stride).enumerate().for_each(|(i, row)| composite_row(row, r.y0 + i as i32, r.x0, r.x1, &layers));
    }
}

/// Recomputes `out[rect]` from `layers` (bottom -> top). Rows are processed with rayon.
///
/// `out` is the `width x height` composite; `rect` is clipped to it. Visible layers with `opacity > 0` take part,
/// or every layer when `include_hidden` is set. Pixels outside `rect` are not touched.
pub fn composite_rect(layers: &[Layer], width: u32, height: u32, rect: IRect, include_hidden: bool, out: &mut PixelBuf) {
    let parallel = rect.area() >= PARALLEL_MIN_PIXELS;
    composite_rect_impl(layers, width, height, rect, include_hidden, out, parallel);
}

/// Flattens `layers` (bottom -> top) into a new `width x height` image.
pub fn flatten_layers(layers: &[Layer], width: u32, height: u32, include_hidden: bool) -> PixelBuf {
    let mut out = PixelBuf::new(width, height);
    composite_rect(layers, width, height, IRect::from_size(width, height), include_hidden, &mut out);
    out
}

impl Document {
    /// Composites every pending dirty rectangle (pending -> composite) and extends `gpu_dirty`.
    pub fn recomposite_all(&mut self) {
        self.refresh_content_bounds();
        for set in &mut self.sets {
            if set.pending.is_empty() {
                continue;
            }
            let TextureSet { layers, width, height, composite, pending, gpu_dirty, .. } = set;
            for r in pending.take() {
                composite_rect(layers, *width, *height, r, false, composite);
                gpu_dirty.add(r);
            }
        }
    }

    /// Computes the content bound (`Layer::content`) of every paint layer whose bound is not known for its current
    /// version (a new document, a loaded project, an undo that brought back an older buffer). Later pixel changes keep
    /// the bound up to date through `mark_layer_dirty`, so this scans each buffer once.
    pub(crate) fn refresh_content_bounds(&mut self) {
        for layer in self.sets.iter_mut().flat_map(|s| &mut s.layers) {
            if layer.known_content().is_some() {
                continue;
            }
            if let LayerKind::Raster(buf) = &layer.kind {
                let bound = nonzero_bounds_par(buf).unwrap_or(IRect::EMPTY);
                layer.content = Some((layer.version, bound));
            }
        }
    }

    /// True when layer changes are waiting to be composited.
    pub fn has_pending_composite(&self) -> bool {
        self.sets.iter().any(|s| !s.pending.is_empty())
    }

    /// Rects of `composite` changed since the last call.
    pub fn take_gpu_dirty(&mut self, set: TextureSetId) -> Vec<IRect> {
        self.set_mut(set).map(|s| s.gpu_dirty.take()).unwrap_or_default()
    }

    /// The composite of a set; byte-identical to `flatten(set, false)` once nothing is pending.
    pub fn composite(&self, set: TextureSetId) -> Option<&PixelBuf> {
        self.set(set).map(|s| &s.composite)
    }

    /// Flattens the layers of a set into a new image.
    pub fn flatten(&self, set: TextureSetId, include_hidden: bool) -> Option<PixelBuf> {
        self.set(set).map(|s| flatten_layers(&s.layers, s.width, s.height, include_hidden))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::blend::blend_pixel;
    use crate::ids::{LayerId, StampId};
    use crate::layer::LayerKind;
    use crate::pixel::{CroppedBuf, SparseBuf};
    use crate::samples;
    use crate::testutil::{XorShift, doc_from_raw};

    fn random_buf(rng: &mut XorShift, w: u32, h: u32) -> PixelBuf {
        let mut buf = PixelBuf::new(w, h);
        for px in buf.data.as_chunks_mut::<4>().0 {
            // A third fully transparent (often with hidden colour), a third opaque, the rest anything.
            let a = match rng.next_u32() % 3 {
                0 => 0,
                1 => 255,
                _ => (rng.next_u32() & 0xff) as u8,
            };
            let hidden = rng.next_u32().is_multiple_of(2);
            let c = |rng: &mut XorShift| if a == 0 && !hidden { 0 } else { (rng.next_u32() & 0xff) as u8 };
            *px = [c(rng), c(rng), c(rng), a];
        }
        buf
    }

    fn raster(id: u64, buf: PixelBuf) -> Layer {
        Layer::new(LayerId(id), &format!("L{id}"), LayerKind::Raster(Arc::new(buf)))
    }

    fn cropped(id: u64, rect: IRect, rng: &mut XorShift) -> Layer {
        let mut c = CroppedBuf::new(rect);
        for b in &mut c.data {
            *b = (rng.next_u32() & 0xff) as u8;
        }
        Layer::new(LayerId(id), "decal", LayerKind::Stamp { stamp: StampId(900 + id), cache: Arc::new(SparseBuf::from_cropped(c)) })
    }

    /// A stack with every blend mode, mixed opacities, a hidden layer, a zero-opacity layer and two cropped layers.
    fn random_stack(rng: &mut XorShift, w: u32, h: u32) -> Vec<Layer> {
        let mut layers = Vec::new();
        for (i, mode) in BlendMode::ALL.into_iter().enumerate() {
            let mut l = raster(i as u64 + 1, random_buf(rng, w, h));
            l.blend = mode;
            l.opacity = [1.0, 0.75, 0.3, 1.0, 0.5, 0.9, 0.1][i];
            layers.push(l);
        }
        layers[2].visible = false;
        layers[4].opacity = 0.0;
        let mut a = cropped(20, IRect::new(3, 5, w as i32 - 9, h as i32 / 2), rng);
        a.opacity = 0.8;
        a.blend = BlendMode::Multiply;
        layers.insert(3, a);
        // Reaches outside the canvas on every side.
        layers.push(cropped(21, IRect::new(-7, -3, w as i32 + 5, 11), rng));
        layers.push(cropped(22, IRect::new(w as i32 - 4, h as i32 - 6, w as i32 + 20, h as i32 + 20), rng));
        // An empty cache contributes nothing.
        layers.push(Layer::new(LayerId(23), "empty", LayerKind::Text { stamp: StampId(7), cache: Arc::new(SparseBuf::empty()) }));
        layers
    }

    /// Pixel-by-pixel reference built on `blend_pixel`.
    fn reference(layers: &[Layer], w: u32, h: u32, include_hidden: bool) -> PixelBuf {
        let mut out = PixelBuf::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0u8; 4];
                for l in layers {
                    if !(include_hidden || (l.visible && l.opacity > 0.0)) {
                        continue;
                    }
                    if l.views().iter().all(|v| !v.rect.contains(x as i32, y as i32)) {
                        continue;
                    }
                    acc = blend_pixel(acc, l.get(x as i32, y as i32), opacity_u8(l.opacity), l.blend);
                }
                out.set(x, y, acc);
            }
        }
        out
    }

    #[test]
    fn opacity_quantization() {
        assert_eq!(opacity_u8(1.0), 255);
        assert_eq!(opacity_u8(0.0), 0);
        assert_eq!(opacity_u8(0.5), 128);
        assert_eq!(opacity_u8(7.0), 255);
        assert_eq!(opacity_u8(-1.0), 0);
        assert_eq!(opacity_u8(f32::NAN), 0);
    }

    #[test]
    fn flatten_matches_the_per_pixel_reference() {
        let mut rng = XorShift::new(11);
        let (w, h) = (61, 47);
        let layers = random_stack(&mut rng, w, h);
        assert_eq!(flatten_layers(&layers, w, h, false), reference(&layers, w, h, false));
        assert_eq!(flatten_layers(&layers, w, h, true), reference(&layers, w, h, true));
    }

    #[test]
    fn hidden_and_zero_opacity_layers_are_skipped() {
        let mut rng = XorShift::new(12);
        let (w, h) = (40, 30);
        let layers = random_stack(&mut rng, w, h);
        let visible_only: Vec<Layer> = layers.iter().filter(|l| l.visible && l.opacity > 0.0).cloned().collect();
        assert!(visible_only.len() < layers.len());
        assert_eq!(flatten_layers(&layers, w, h, false), flatten_layers(&visible_only, w, h, false));
        // With include_hidden they do take part.
        assert_ne!(flatten_layers(&layers, w, h, true), flatten_layers(&layers, w, h, false));
        // No layers at all: transparent.
        assert_eq!(flatten_layers(&[], w, h, false), PixelBuf::new(w, h));
    }

    #[test]
    fn cropped_layers_are_clipped_to_their_rect_and_the_canvas() {
        let (w, h) = (32u32, 24u32);
        let base = raster(1, PixelBuf::filled(w, h, [10, 20, 30, 255]));
        let rect = IRect::new(-5, 20, 6, 40);
        let mut c = CroppedBuf::new(rect);
        for px in c.data.as_chunks_mut::<4>().0 {
            *px = [200, 100, 50, 255];
        }
        let decal = Layer::new(LayerId(2), "decal", LayerKind::Stamp { stamp: StampId(1), cache: Arc::new(SparseBuf::from_cropped(c)) });
        let out = flatten_layers(&[base, decal], w, h, false);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let want = if rect.contains(x, y) { [200, 100, 50, 255] } else { [10, 20, 30, 255] };
                assert_eq!(out.get(x as u32, y as u32), want, "({x}, {y})");
            }
        }
    }

    #[test]
    fn dirty_rect_recomposite_equals_full_recomposite() {
        let mut rng = XorShift::new(13);
        let (w, h) = (96u32, 80u32);
        let mut layers = random_stack(&mut rng, w, h);
        let mut composite = flatten_layers(&layers, w, h, false);
        for step in 0..60 {
            // Random edit of one raster layer inside a random rect (sometimes reaching outside the canvas).
            let x0 = (rng.next_u32() % w) as i32 - 4;
            let y0 = (rng.next_u32() % h) as i32 - 4;
            let r = IRect::new(x0, y0, x0 + 1 + (rng.next_u32() % 30) as i32, y0 + 1 + (rng.next_u32() % 30) as i32);
            let clipped = r.intersect(&IRect::from_size(w, h));
            let target = (rng.next_u32() % 7) as usize;
            let index = layers.iter().position(|l| l.id == LayerId(target as u64 + 1)).unwrap();
            let LayerKind::Raster(buf) = &mut layers[index].kind else { unreachable!() };
            let buf = Arc::make_mut(buf);
            for y in clipped.y0..clipped.y1 {
                for x in clipped.x0..clipped.x1 {
                    let v = rng.next_u32().to_le_bytes();
                    buf.set(x as u32, y as u32, v);
                }
            }
            composite_rect(&layers, w, h, r, false, &mut composite);
            assert_eq!(composite, flatten_layers(&layers, w, h, false), "step {step}");
        }
    }

    #[test]
    fn pixels_outside_the_rect_are_untouched() {
        let mut rng = XorShift::new(14);
        let (w, h) = (50u32, 40u32);
        let layers = random_stack(&mut rng, w, h);
        let mut out = PixelBuf::filled(w, h, [1, 2, 3, 4]);
        let r = IRect::new(10, 5, 31, 22);
        composite_rect(&layers, w, h, r, false, &mut out);
        let full = flatten_layers(&layers, w, h, false);
        for y in 0..h {
            for x in 0..w {
                let want = if r.contains(x as i32, y as i32) { full.get(x, y) } else { [1, 2, 3, 4] };
                assert_eq!(out.get(x, y), want);
            }
        }
        // Empty and fully outside rects do nothing.
        let before = out.clone();
        composite_rect(&layers, w, h, IRect::EMPTY, false, &mut out);
        composite_rect(&layers, w, h, IRect::new(60, 0, 90, 10), false, &mut out);
        assert_eq!(out, before);
    }

    #[test]
    fn parallel_equals_serial() {
        let mut rng = XorShift::new(15);
        let (w, h) = (300u32, 260u32);
        let layers = random_stack(&mut rng, w, h);
        let full = IRect::from_size(w, h);
        assert!(full.area() >= PARALLEL_MIN_PIXELS, "the public entry point must take the parallel path here");
        let mut serial = PixelBuf::new(w, h);
        composite_rect_impl(&layers, w, h, full, false, &mut serial, false);
        let mut parallel = PixelBuf::new(w, h);
        composite_rect_impl(&layers, w, h, full, false, &mut parallel, true);
        assert_eq!(serial, parallel);
        assert_eq!(flatten_layers(&layers, w, h, false), serial);
    }

    #[test]
    fn flattening_one_layer_reproduces_it_byte_for_byte() {
        let mut rng = XorShift::new(16);
        let (w, h) = (64u32, 48u32);
        let buf = random_buf(&mut rng, w, h);
        assert!(buf.data.as_chunks::<4>().0.iter().any(|p| p[3] == 0 && p[..3] != [0, 0, 0]), "needs hidden colour");
        for mode in BlendMode::ALL {
            let mut layer = raster(1, buf.clone());
            layer.blend = mode;
            assert_eq!(flatten_layers(&[layer], w, h, false), buf, "{mode:?}");
        }
    }

    #[test]
    fn hidden_colour_survives_under_an_empty_layer() {
        let (w, h) = (16u32, 16u32);
        let base = PixelBuf::filled(w, h, [10, 20, 30, 0]);
        for mode in BlendMode::ALL {
            let mut top = raster(2, PixelBuf::new(w, h));
            top.blend = mode;
            let out = flatten_layers(&[raster(1, base.clone()), top], w, h, false);
            assert_eq!(out, base, "{mode:?}");
        }
        // Base + empty Paint 1 with real content: export(import(x)) == x.
        let mut rng = XorShift::new(17);
        let texture = random_buf(&mut rng, w, h);
        let out = flatten_layers(&[raster(1, texture.clone()), raster(2, PixelBuf::new(w, h))], w, h, false);
        assert_eq!(out, texture);
    }

    #[test]
    fn transparent_pixels_never_change_a_visible_accumulator() {
        let mut rng = XorShift::new(18);
        let (w, h) = (32u32, 32u32);
        let base = PixelBuf::filled(w, h, [90, 80, 70, 200]);
        let mut top_buf = random_buf(&mut rng, w, h);
        for px in top_buf.data.as_chunks_mut::<4>().0 {
            px[3] = 0;
        }
        for mode in BlendMode::ALL {
            let mut top = raster(2, top_buf.clone());
            top.blend = mode;
            top.opacity = 0.6;
            assert_eq!(flatten_layers(&[raster(1, base.clone()), top], w, h, false), base, "{mode:?}");
        }
    }

    #[test]
    fn document_composite_equals_flatten_and_reports_gpu_dirty() {
        let mut doc = doc_from_raw(samples::two_material_model(), 64);
        assert_eq!(doc.sets.len(), 2);
        assert!(!doc.has_pending_composite());
        for set in doc.set_ids() {
            assert_eq!(doc.composite(set), doc.flatten(set, false).as_ref());
            assert!(doc.take_gpu_dirty(set).is_empty(), "a fresh document has nothing to upload incrementally");
        }
        assert!(doc.composite(TextureSetId(9)).is_none());
        assert!(doc.flatten(TextureSetId(9), true).is_none());
        assert!(doc.take_gpu_dirty(TextureSetId(9)).is_empty());

        // Edit one layer of set 1 directly and report it the official way.
        let layer = doc.sets[1].layers[0].id;
        let rect = IRect::new(5, 6, 20, 30);
        {
            let LayerKind::Raster(buf) = &mut doc.layer_mut(layer).unwrap().kind else { unreachable!() };
            let buf = Arc::make_mut(buf);
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    buf.set(x as u32, y as u32, [200, 10, 10, 128]);
                }
            }
        }
        let stale = doc.composite(TextureSetId(1)).unwrap().clone();
        doc.mark_layer_dirty(layer, rect);
        assert!(doc.has_pending_composite());
        assert_eq!(doc.composite(TextureSetId(1)), Some(&stale), "the composite changes only in recomposite_all");
        doc.recomposite_all();
        assert!(!doc.has_pending_composite());
        assert_eq!(doc.composite(TextureSetId(1)), doc.flatten(TextureSetId(1), false).as_ref());
        assert_ne!(doc.composite(TextureSetId(1)), Some(&stale));
        assert!(doc.take_gpu_dirty(TextureSetId(0)).is_empty());
        assert_eq!(doc.take_gpu_dirty(TextureSetId(1)), vec![rect]);
        assert!(doc.take_gpu_dirty(TextureSetId(1)).is_empty(), "take clears");

        // Hidden layers appear only in flatten(.., true).
        doc.sets[1].layers[0].visible = false;
        assert_eq!(doc.flatten(TextureSetId(1), false).unwrap(), PixelBuf::new(64, 64));
        assert_ne!(doc.flatten(TextureSetId(1), true).unwrap(), PixelBuf::new(64, 64));
    }
}
