// Owner: stickers P3b (STICKER_PLAN 2.7, 2.8, 2.10; ARCHITECTURE 5.18, 5.20, 5.27, 6.7). Pixel edits on stickers:
// brush and eraser strokes on picture layers and masks (`sticker_stroke_*`), whole-buffer tool results
// (`replace_sticker_pixels`), the undo of both (`Edit::StickerPixels`, generic tile patches), promotion of an image
// stamp inside the edit that needs it (2.10, review A3), `convert_text_stamp` (review A24), the editor's Revert
// (`sticker_checkpoint`, `restore_sticker_state`) and the re-bake of a sticker's users (`rebake_sticker_users`).
//
// Choices the plan leaves open, documented at the items below:
// - A stroke's pixels follow the texture stroke's model (6.4): a 16-bit coverage per pixel that dabs raise (flow
//   builds), re-applied from the pre-stroke pixels (opacity caps). Dabs do not wrap around the picture's borders.
// - Every dab that changes pixels gives the buffer a new version (not only `begin`), so a Preview render during a
//   stroke can never take a layer raster of an earlier state from the cache (2.2: equal versions mean equal bytes).
// - Writes go through `Arc::make_mut`. Cached renders and styled pictures made from the sticker hold its `Arc`; they
//   are dropped before every write (they are stale after it), so only a history snapshot (or a running job) can make
//   a write copy a buffer: the one copy per stroke happens in `sticker_stroke_begin` (review A25).

use std::sync::Arc;

use glam::Vec2;
use rayon::prelude::*;

use crate::brush::{BrushMode, PressureDynamics, apply_brush, falloff, mask_increment, unit};
use crate::doc_stickers::{BaseIdentity, ShareScope, StickerRef};
use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::history::{Edit, MaskBackup, MaskPatch, PendingEdit, StructureSnapshot, TileBackup, TilePatch};
use crate::ids::{BlobId, StampId, StickerId, StickerLayerId};
use crate::layer::LayerKind;
use crate::math::IRect;
use crate::pixel::{AlphaBuf, PixelBuf};
use crate::stamp::{BakeQuality, StampObject, StampSource};
use crate::sticker::pipeline::STYLE_REFERENCE_EM;
use crate::sticker::{
    LayerTransform, MAX_CREATED_SIDE, MAX_OWNED_BYTES, PicturePixels, StickerContent, StickerDoc, StickerLayer, counters,
};

/// History label of a brush stroke on a sticker picture or mask.
pub const STICKER_BRUSH_LABEL: &str = "Sticker Brush";
/// History label of an eraser stroke on a sticker picture or mask.
pub const STICKER_ERASER_LABEL: &str = "Sticker Eraser";
/// History label of `convert_text_stamp` (and of `promote_stamp`).
pub const CONVERT_LABEL: &str = "Convert to Sticker";
/// The default label of the editor's Revert (`restore_sticker_state`).
pub const REVERT_LABEL: &str = "Revert Sticker";

/// Rects of at least this many pixels are re-applied on the rayon pool.
const PARALLEL_PIXELS: u64 = 1 << 15;
/// A segment that would need more dabs than this paints none (guards absurd jumps).
const MAX_DABS_PER_SAMPLE: f32 = 100_000.0;

/// Which buffer of a picture layer a pixel edit changes (2.8).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PixelTarget {
    /// The picture (straight RGBA8).
    Pixels,
    /// The layer mask (8 bits, 255 = shown).
    Mask,
}

/// The new pixels of `replace_sticker_pixels` (the plan's `TileData`, renamed: `history::TileData` is the tile
/// payload of a patch). The kind must match the target.
#[derive(Clone, Debug, PartialEq)]
pub enum PixelData {
    Rgba(PixelBuf),
    Alpha(AlphaBuf),
}

impl PixelData {
    /// The target this data fits.
    pub fn target(&self) -> PixelTarget {
        match self {
            PixelData::Rgba(_) => PixelTarget::Pixels,
            PixelData::Alpha(_) => PixelTarget::Mask,
        }
    }

    /// `[width, height]`.
    pub fn size(&self) -> [u32; 2] {
        match self {
            PixelData::Rgba(p) => [p.width, p.height],
            PixelData::Alpha(a) => [a.width, a.height],
        }
    }

    /// At least 1 x 1 and as many bytes as the size says.
    fn is_valid(&self) -> bool {
        let [w, h] = self.size();
        let bytes = match self {
            PixelData::Rgba(p) => p.data.len() == w as usize * h as usize * 4,
            PixelData::Alpha(a) => a.data.len() == w as usize * h as usize,
        };
        w > 0 && h > 0 && bytes
    }
}

/// The tile patch of an `Edit::StickerPixels`: 4 bytes per pixel for a picture, 1 for a mask (2.10, review A4).
pub enum StickerPatch {
    Pixels(TilePatch),
    Mask(MaskPatch),
}

impl StickerPatch {
    /// The buffer the patch is made for.
    pub fn target(&self) -> PixelTarget {
        match self {
            StickerPatch::Pixels(_) => PixelTarget::Pixels,
            StickerPatch::Mask(_) => PixelTarget::Mask,
        }
    }

    /// Memory held by the patch, in bytes (its history cost).
    pub fn byte_len(&self) -> usize {
        match self {
            StickerPatch::Pixels(p) => p.byte_len(),
            StickerPatch::Mask(p) => p.byte_len(),
        }
    }

    /// True when the patch holds no tile.
    pub fn is_empty(&self) -> bool {
        match self {
            StickerPatch::Pixels(p) => p.is_empty(),
            StickerPatch::Mask(p) => p.is_empty(),
        }
    }
}

/// The brush of a sticker stroke (2.8; the 2D brush of 6.4 with lengths in layer-space pixels).
#[derive(Clone, Debug, PartialEq)]
pub struct StickerBrush {
    /// Layer-space pixels (converted to content pixels with the layer's mean scale).
    pub radius: f32,
    /// 0..=1.
    pub hardness: f32,
    /// Stroke ceiling 0..=1.
    pub opacity: f32,
    /// Per-dab build-up 0..=1.
    pub flow: f32,
    /// Fraction of the diameter between dabs, 0.02..=1.
    pub spacing: f32,
    /// Straight sRGB; alpha scales the strength. On a mask the colour's grey level is the value painted (Rec. 601).
    pub color: [u8; 4],
    /// Paint, or Erase (a picture loses alpha; a mask is painted towards 0, hidden).
    pub mode: BrushMode,
    pub dynamics: PressureDynamics,
    /// The selection (2.8 "honouring the selection mask"): the size of the target buffer, 255 = fully selected. A dab
    /// changes a pixel by at most its selection value.
    pub selection: Option<Arc<AlphaBuf>>,
}

impl Default for StickerBrush {
    /// Radius 16, hardness 0.8, opacity 1, flow 1, spacing 0.12, black, Paint, default dynamics, no selection.
    fn default() -> StickerBrush {
        StickerBrush {
            radius: 16.0,
            hardness: 0.8,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.12,
            color: [0, 0, 0, 255],
            mode: BrushMode::Paint,
            dynamics: PressureDynamics::default(),
            selection: None,
        }
    }
}

impl StickerBrush {
    /// Every number in its range (NaN and infinities included), as the texture stroke's `sanitize`.
    pub fn sanitized(&self) -> StickerBrush {
        StickerBrush {
            radius: if self.radius.is_finite() { self.radius.clamp(0.0, 4096.0) } else { 0.0 },
            hardness: if self.hardness.is_nan() { 1.0 } else { self.hardness.clamp(0.0, 1.0) },
            opacity: unit(self.opacity),
            flow: unit(self.flow),
            spacing: if self.spacing.is_nan() { 0.12 } else { self.spacing.clamp(0.02, 1.0) },
            dynamics: self.dynamics.sanitized(),
            ..self.clone()
        }
    }
}

/// What the editor's Revert puts back (2.8): the named stamps and stickers as they were (`Arc` clones; no pixel is
/// copied).
#[derive(Clone, Debug)]
pub struct StickerCheckpoint {
    pub(crate) stamps: Vec<StampObject>,
    pub(crate) stickers: Vec<(StickerId, Arc<StickerDoc>)>,
}

impl StickerCheckpoint {
    /// The stamps of the checkpoint, ascending id.
    pub fn stamps(&self) -> &[StampObject] {
        &self.stamps
    }

    /// The sticker `id` as the checkpoint holds it.
    pub fn sticker(&self, id: StickerId) -> Option<&Arc<StickerDoc>> {
        self.stickers.iter().find(|(s, _)| *s == id).map(|(_, d)| d)
    }
}

/// The pre-stroke tiles of the stroke's buffer.
enum StrokeBackup {
    Pixels(TileBackup),
    Mask(MaskBackup),
}

/// The open sticker stroke (`Document::sticker_state.stroke`).
pub(crate) struct StickerStroke {
    sticker: StickerId,
    layer: StickerLayerId,
    /// Sanitized.
    brush: StickerBrush,
    /// The structure before `begin` changed it (a promotion, a blob picture made owned); `None` when it did not.
    before: Option<Box<StructureSnapshot>>,
    backup: StrokeBackup,
    /// Accumulated stroke coverage per buffer pixel, 0..=65535.
    coverage: Vec<u16>,
    size: [u32; 2],
    /// The layer's transform at `begin` (layer space -> content pixels).
    transform: LayerTransform,
    /// The previous sample (content pixels, pressure).
    last: Option<(Vec2, f32)>,
    /// Distance travelled since the last dab, below one step.
    residual: f32,
    /// A Preview re-bake of the users ran during the stroke (`rebake_sticker_users`).
    previewed: bool,
    /// Reused row buffer of the re-apply.
    row: Vec<u8>,
}

/// The buffer a pixel edit would change, read before anything changes.
enum Current {
    Owned(Arc<PixelBuf>),
    Blob(BlobId),
    Mask(Arc<AlphaBuf>),
}

/// `Arc::make_mut` that counts a copy of a shared buffer (`counters::buffer_copies`).
fn make_mut_counted<T: Clone>(arc: &mut Arc<T>) -> &mut T {
    if Arc::get_mut(arc).is_none() {
        counters::buffer_copied();
    }
    Arc::make_mut(arc)
}

/// Rec. 601 grey level of a colour.
fn grey(c: [u8; 4]) -> u8 {
    ((299 * c[0] as u32 + 587 * c[1] as u32 + 114 * c[2] as u32 + 500) / 1000) as u8
}

/// One disc dab on a `w x h` coverage plane without wrapping (the 2D kernel of 6.4: `falloff` with a one-pixel
/// anti-aliasing ramp, `mask_increment`). Returns the rect of changed pixels.
fn dab(coverage: &mut [u16], [w, h]: [u32; 2], centre: Vec2, radius: f32, hardness: f32, strength: f32) -> IRect {
    const AA: f32 = 1.0;
    let strength = unit(strength);
    if strength <= 0.0 || !(centre.is_finite() && radius.is_finite() && radius >= 0.0) {
        return IRect::EMPTY;
    }
    let hardness = if radius < AA { 1.0 } else { hardness };
    let support = radius + 0.5 * AA;
    let x0 = ((centre.x - support).floor().max(0.0) as i64).min(w as i64) as u32;
    let x1 = (((centre.x + support).floor() + 1.0).max(0.0) as i64).min(w as i64) as u32;
    let y0 = ((centre.y - support).floor().max(0.0) as i64).min(h as i64) as u32;
    let y1 = (((centre.y + support).floor() + 1.0).max(0.0) as i64).min(h as i64) as u32;
    let mut changed = IRect::EMPTY;
    for y in y0..y1 {
        let dy = y as f32 + 0.5 - centre.y;
        let row = &mut coverage[y as usize * w as usize..(y as usize + 1) * w as usize];
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - centre.x;
            let cov = falloff((dx * dx + dy * dy).sqrt(), radius, hardness, AA);
            if cov <= 0.0 {
                continue;
            }
            let m = &mut row[x as usize];
            let inc = mask_increment(u16::MAX - *m, strength * cov);
            if inc > 0 {
                *m += inc;
                changed = changed.union(&IRect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1));
            }
        }
    }
    changed
}

/// Strength of a pixel: coverage times the ceiling times the selection.
#[inline]
fn strength(m: u16, ceiling: f32, sel: u8) -> f32 {
    (m as f32 / 65535.0) * ceiling * (sel as f32 / 255.0)
}

/// Recomputes the pixels of `r` of a sticker picture from their pre-stroke values and the coverage.
fn reapply_rgba(buf: &mut PixelBuf, backup: &TileBackup, cov: &[u16], brush: &StickerBrush, r: IRect, row: &mut Vec<u8>) {
    let ceiling = brush.opacity * (brush.color[3] as f32 / 255.0);
    let (color, mode, w) = (brush.color, brush.mode, buf.width as usize);
    let sel = brush.selection.as_deref();
    let shade = |y: u32, original: &[u8], out: &mut [u8]| {
        let base = y as usize * w + r.x0 as usize;
        for (i, (o, s)) in out.as_chunks_mut::<4>().0.iter_mut().zip(original.as_chunks::<4>().0).enumerate() {
            let m = cov[base + i];
            let a = sel.map_or(255, |sel| sel.data[base + i]);
            *o = if m > 0 { apply_brush(*s, color, strength(m, ceiling, a), mode, false) } else { *s };
        }
    };
    let (x0, x1) = (r.x0 as u32, r.x1 as u32);
    if r.area() < PARALLEL_PIXELS {
        for y in r.y0 as u32..r.y1 as u32 {
            backup.original_row(buf, y, x0, x1, row);
            shade(y, row, &mut buf.row_mut(y)[x0 as usize * 4..x1 as usize * 4]);
        }
        return;
    }
    let stride = r.width() as usize * 4;
    let mut block = vec![0u8; stride * r.height() as usize];
    let source: &PixelBuf = buf;
    block.par_chunks_mut(stride).enumerate().for_each_init(Vec::new, |original, (j, out)| {
        let y = r.y0 as u32 + j as u32;
        backup.original_row(source, y, x0, x1, original);
        shade(y, original, out);
    });
    for (j, out) in block.chunks(stride).enumerate() {
        buf.row_mut(r.y0 as u32 + j as u32)[x0 as usize * 4..x1 as usize * 4].copy_from_slice(out);
    }
}

/// Recomputes the values of `r` of a mask: Paint moves a value towards the colour's grey level, Erase towards 0.
fn reapply_mask(buf: &mut AlphaBuf, backup: &MaskBackup, cov: &[u16], brush: &StickerBrush, r: IRect, row: &mut Vec<u8>) {
    let ceiling = brush.opacity * (brush.color[3] as f32 / 255.0);
    let target = match brush.mode {
        BrushMode::Paint => grey(brush.color) as f32,
        BrushMode::Erase => 0.0,
    };
    let w = buf.width as usize;
    let sel = brush.selection.as_deref();
    let (x0, x1) = (r.x0 as u32, r.x1 as u32);
    for y in r.y0 as u32..r.y1 as u32 {
        backup.original_span(buf, y, x0, x1, row);
        let base = y as usize * w + x0 as usize;
        for (i, &v) in row.iter().enumerate() {
            let m = cov[base + i];
            let a = sel.map_or(255, |sel| sel.data[base + i]);
            buf.data[base + i] = if m > 0 {
                let s = unit(strength(m, ceiling, a));
                (v as f32 + (target - v as f32) * s).round().clamp(0.0, 255.0) as u8
            } else {
                v
            };
        }
    }
}

impl Document {
    // ----- strokes (2.8) -----

    /// Starts a brush or eraser stroke on picture layer `layer` (`PixelTarget::Pixels`) or on its mask
    /// (`PixelTarget::Mask`) of the sticker `on` names. For an image stamp that is not a sticker yet the stroke
    /// promotes it first (2.10: one undo step for both). A `Picture(Blob)` becomes `Owned` here. This is the one
    /// place a buffer that a history snapshot shares is copied (review A25). Errors before anything changes:
    /// `StrokeInProgress`, `UnknownStamp` / `UnknownSticker` / `UnknownStickerLayer`, `LayerLocked` (the layer or a
    /// user's linked layer), `InvalidParam` (not a picture layer, no mask, a text stamp, a selection of another size),
    /// `StickerLimit` (the owned pictures would exceed 512 MiB), the image errors of a blob picture.
    pub fn sticker_stroke_begin(
        &mut self,
        on: StickerRef,
        layer: StickerLayerId,
        target: PixelTarget,
        params: StickerBrush,
    ) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let brush = params.sanitized();
        let promote = self.promotion_of(on)?;
        // The "before" state is taken only when `begin` changes the structure: a snapshot shares the sticker, and a
        // needless one would make the write below copy the picture (review A25).
        let mut before = promote.map(|_| self.snapshot_structure(&[]));
        let id = match promote {
            Some((stamp, scope)) => self.promote_no_history(stamp, scope)?,
            None => self.resolve_sticker(on)?,
        };
        let current = match self.pixel_target(id, layer, target) {
            Ok(c) => c,
            Err(e) => {
                if let Some(before) = before {
                    self.restore_structure(before);
                }
                return Err(e);
            }
        };
        // A blob picture becomes owned: its decoded pixels (the cache's `Arc`, copied below exactly once).
        let blob_pixels = match &current {
            Current::Blob(b) => match self.blob_picture(*b).and_then(|p| {
                let doc = self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?;
                if doc.owned_bytes() + p.byte_len() as u64 > MAX_OWNED_BYTES {
                    return Err(EditError::StickerLimit("a sticker can hold at most 512 MB of edited pictures"));
                }
                Ok(p)
            }) {
                Ok(p) => Some(p),
                Err(e) => {
                    if let Some(before) = before {
                        self.restore_structure(before);
                    }
                    return Err(e);
                }
            },
            _ => None,
        };
        let size = match &current {
            Current::Owned(p) => [p.width, p.height],
            Current::Blob(_) => blob_pixels.as_ref().map_or([0, 0], |p| [p.width, p.height]),
            Current::Mask(m) => [m.width, m.height],
        };
        if let Some(sel) = &brush.selection
            && [sel.width, sel.height] != size
        {
            if let Some(before) = before {
                self.restore_structure(before);
            }
            return Err(EditError::InvalidParam("the selection must have the size of the painted buffer"));
        }
        // `current` holds an `Arc` of the buffer: dropped before the write, which must not copy it for that.
        drop(current);
        if before.is_none() && blob_pixels.is_some() {
            before = Some(self.snapshot_structure(&[]));
        }
        let version = self.next_pixel_version();
        self.forget_sticker_pictures(id);
        let transform;
        {
            let Some(arc) = self.sticker_state.map.get_mut(&id) else { return Err(EditError::UnknownSticker) };
            let doc = Arc::make_mut(arc);
            let Some(l) = doc.layer_mut(layer) else { return Err(EditError::UnknownStickerLayer) };
            transform = l.transform;
            if let Some(pixels) = blob_pixels {
                l.content = StickerContent::Picture(PicturePixels::Owned { pixels, version });
            }
            match target {
                PixelTarget::Pixels => {
                    if let StickerContent::Picture(PicturePixels::Owned { pixels, version: v }) = &mut l.content {
                        make_mut_counted(pixels);
                        *v = version;
                    }
                }
                PixelTarget::Mask => {
                    if let Some(m) = &mut l.mask {
                        make_mut_counted(&mut m.pixels);
                        m.version = version;
                    }
                }
            }
        }
        let backup = match target {
            PixelTarget::Pixels => StrokeBackup::Pixels(TileBackup::new(size[0], size[1])),
            PixelTarget::Mask => StrokeBackup::Mask(MaskBackup::new(size[0], size[1])),
        };
        self.sticker_state.stroke = Some(Box::new(StickerStroke {
            sticker: id,
            layer,
            brush,
            before: before.map(Box::new),
            backup,
            coverage: vec![0; size[0] as usize * size[1] as usize],
            size,
            transform,
            last: None,
            residual: 0.0,
            previewed: false,
            row: Vec::new(),
        }));
        Ok(id)
    }

    /// One sample of the open sticker stroke at layer-space point `at` with pen `pressure` (0..=1; a mouse sends 1):
    /// a dab at the first sample, then dabs every `spacing * diameter` along the segment from the previous sample
    /// (pressure interpolated). Returns the changed rectangle in buffer (content) pixels, `EMPTY` when nothing
    /// changed. `Err(InvalidParam)` without an open sticker stroke.
    pub fn sticker_stroke_dab(&mut self, at: Vec2, pressure: f32) -> Result<IRect, EditError> {
        let Some(mut s) = self.sticker_state.stroke.take() else {
            return Err(EditError::InvalidParam("no sticker stroke is open"));
        };
        let r = self.stroke_sample(&mut s, at, pressure);
        self.sticker_state.stroke = Some(s);
        Ok(r)
    }

    /// Ends the open sticker stroke: one history entry ("Sticker Brush" / "Sticker Eraser"; with a promotion or a blob
    /// picture made owned at `begin`, `Edit::Multi([Structure(before), StickerPixels])`), then the users are re-baked
    /// at Final quality (deferred when that mode is on). A stroke that changed no pixel leaves no entry and takes back
    /// what `begin` changed. `Err(InvalidParam)` without an open sticker stroke.
    pub fn sticker_stroke_end(&mut self) -> Result<(), EditError> {
        let Some(s) = self.sticker_state.stroke.take() else {
            return Err(EditError::InvalidParam("no sticker stroke is open"));
        };
        let s = *s;
        let label = match s.brush.mode {
            BrushMode::Paint => STICKER_BRUSH_LABEL,
            BrushMode::Erase => STICKER_ERASER_LABEL,
        };
        let patch = {
            let layer = self.sticker_state.map.get(&s.sticker).and_then(|d| d.layer(s.layer));
            match (s.backup, layer) {
                (StrokeBackup::Pixels(b), Some(l)) => match &l.content {
                    StickerContent::Picture(PicturePixels::Owned { pixels, .. }) => StickerPatch::Pixels(b.into_patch(&**pixels)),
                    _ => StickerPatch::Pixels(TilePatch::empty()),
                },
                (StrokeBackup::Mask(b), Some(l)) => match &l.mask {
                    Some(m) => StickerPatch::Mask(b.into_patch(&*m.pixels)),
                    None => StickerPatch::Mask(MaskPatch::empty()),
                },
                (StrokeBackup::Pixels(_), None) => StickerPatch::Pixels(TilePatch::empty()),
                (StrokeBackup::Mask(_), None) => StickerPatch::Mask(MaskPatch::empty()),
            }
        };
        self.forget_sticker_pictures(s.sticker);
        if patch.is_empty() {
            match s.before {
                Some(before) => {
                    self.restore_structure(*before);
                }
                None if s.previewed => self.rebake_users_final(s.sticker),
                None => {}
            }
            return Ok(());
        }
        let pixels = Edit::StickerPixels { sticker: s.sticker, layer: s.layer, patch };
        let edit = match s.before {
            Some(before) => Edit::Multi(vec![Edit::Structure(before), pixels]),
            None => pixels,
        };
        self.push_edit(label, edit);
        self.debug_check_structure();
        self.rebake_users_final(s.sticker);
        Ok(())
    }

    /// Takes back the open sticker stroke without a history entry (pixels, and what `begin` changed); no-op without one.
    pub fn sticker_stroke_cancel(&mut self) {
        let Some(s) = self.sticker_state.stroke.take() else { return };
        let s = *s;
        self.forget_sticker_pictures(s.sticker);
        if let Some(before) = s.before {
            self.restore_structure(*before);
            return;
        }
        let version = self.next_pixel_version();
        if let Some(l) = self.sticker_state.map.get_mut(&s.sticker).and_then(|arc| Arc::make_mut(arc).layer_mut(s.layer)) {
            match (&s.backup, &mut l.content, &mut l.mask) {
                (StrokeBackup::Pixels(b), StickerContent::Picture(PicturePixels::Owned { pixels, version: v }), _) => {
                    b.restore(make_mut_counted(pixels));
                    *v = version;
                }
                (StrokeBackup::Mask(b), _, Some(m)) => {
                    b.restore(make_mut_counted(&mut m.pixels));
                    m.version = version;
                }
                _ => {}
            }
        }
        if s.previewed {
            self.rebake_users_final(s.sticker);
        }
    }

    /// Ends the current segment of the open sticker stroke: the next sample paints a dab where it is, without a line
    /// from the previous one (as `stroke_break`). No-op without an open sticker stroke.
    pub fn sticker_stroke_break(&mut self) {
        if let Some(s) = self.sticker_state.stroke.as_mut() {
            s.last = None;
            s.residual = 0.0;
        }
    }

    /// True while a sticker stroke is open.
    pub fn sticker_stroke_active(&self) -> bool {
        self.sticker_state.stroke.is_some()
    }

    /// The sticker and layer of the open sticker stroke.
    pub fn sticker_stroke_target(&self) -> Option<(StickerId, StickerLayerId)> {
        self.sticker_state.stroke.as_ref().map(|s| (s.sticker, s.layer))
    }

    /// The dabs of one sample (`sticker_stroke_dab`), painted into the buffer.
    fn stroke_sample(&mut self, s: &mut StickerStroke, at: Vec2, pressure: f32) -> IRect {
        if !at.is_finite() {
            return IRect::EMPTY;
        }
        let pressure = if pressure.is_finite() { pressure.clamp(0.0, 1.0) } else { 1.0 };
        let p = s.transform.invert(s.size, at);
        let (spacing, hardness, flow, dynamics) = (s.brush.spacing, s.brush.hardness, s.brush.flow, s.brush.dynamics);
        let radius = s.brush.radius / s.transform.mean_scale();
        let mut centres: Vec<(Vec2, f32)> = Vec::new();
        match s.last {
            None => centres.push((p, pressure)),
            Some((q, qp)) => {
                let step = (spacing * 2.0 * radius).max(0.5);
                let len = q.distance(p);
                let travelled = s.residual + len;
                let count = (travelled / step).floor();
                if len > 0.0 && count.is_finite() && count <= MAX_DABS_PER_SAMPLE {
                    for k in 1..=count as u32 {
                        let t = (k as f32 * step - s.residual) / len;
                        centres.push((q + (p - q) * t, qp + (pressure - qp) * t));
                    }
                    let rest = travelled - count * step;
                    s.residual = if (0.0..step).contains(&rest) { rest } else { 0.0 };
                } else if len > 0.0 {
                    s.residual = 0.0;
                }
            }
        }
        s.last = Some((p, pressure));
        let mut rects = Vec::new();
        for (c, pr) in centres {
            let r = dab(
                &mut s.coverage,
                s.size,
                c,
                radius * dynamics.size_factor(pr),
                hardness,
                flow * dynamics.flow_factor(pr),
            );
            if !r.is_empty() {
                rects.push(r);
            }
        }
        if rects.is_empty() {
            return IRect::EMPTY;
        }
        let version = self.next_pixel_version();
        self.forget_sticker_pictures(s.sticker);
        let Some(l) = self.sticker_state.map.get_mut(&s.sticker).and_then(|arc| Arc::make_mut(arc).layer_mut(s.layer)) else {
            return IRect::EMPTY;
        };
        let mut dirty = IRect::EMPTY;
        match (&mut s.backup, &mut l.content, &mut l.mask) {
            (StrokeBackup::Pixels(backup), StickerContent::Picture(PicturePixels::Owned { pixels, version: v }), _) => {
                let buf = make_mut_counted(pixels);
                for r in &rects {
                    backup.touch_rect(buf, *r);
                    reapply_rgba(buf, backup, &s.coverage, &s.brush, *r, &mut s.row);
                    dirty = dirty.union(r);
                }
                *v = version;
            }
            (StrokeBackup::Mask(backup), _, Some(m)) => {
                let buf = make_mut_counted(&mut m.pixels);
                for r in &rects {
                    backup.touch_rect(buf, *r);
                    reapply_mask(buf, backup, &s.coverage, &s.brush, *r, &mut s.row);
                    dirty = dirty.union(r);
                }
                m.version = version;
            }
            _ => {}
        }
        dirty
    }

    // ----- whole-buffer results (2.8) -----

    /// The result of a tool on a whole picture or mask (magic eraser, colour to alpha, defringe, fill, gradient, blur,
    /// sharpen, remove background, delete selection). An owned picture or a mask of the same size becomes an
    /// `Edit::StickerPixels` tile patch of the changed tiles; a picture of another size, or a blob picture (which
    /// becomes owned), a `Structure` entry. Nothing changes and no entry is made when the pixels are the same. An image
    /// stamp is promoted inside the edit (one entry). The users are re-baked at Final quality. Errors before anything
    /// changes: as `sticker_stroke_begin`, `InvalidParam` (the data does not match the target, an empty picture, a
    /// mask of another size, a picture of another size under a mask), `StickerLimit` (a new size over 4096 pixels a
    /// side, owned pictures over 512 MiB).
    pub fn replace_sticker_pixels(
        &mut self,
        on: StickerRef,
        layer: StickerLayerId,
        target: PixelTarget,
        new: PixelData,
        label: &str,
    ) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        if new.target() != target {
            return Err(EditError::InvalidParam("a picture takes RGBA pixels and a mask one byte per pixel"));
        }
        if !new.is_valid() {
            return Err(EditError::InvalidParam("the new pixels are empty or do not match their size"));
        }
        let promote = self.promotion_of(on)?;
        let before = promote.map(|_| self.snapshot_structure(&[]));
        let id = match promote {
            Some((stamp, scope)) => self.promote_no_history(stamp, scope)?,
            None => self.resolve_sticker(on)?,
        };
        match self.replace_inner(id, layer, target, new, label, before) {
            Ok(()) => Ok(id),
            Err((e, Some(before))) => {
                self.restore_structure(*before);
                Err(e)
            }
            Err((e, None)) => Err(e),
        }
    }

    /// The body of `replace_sticker_pixels` after the sticker is known. `before` is the state before a promotion
    /// (`None`: no promotion); on an error it comes back to the caller, which takes the promotion back.
    fn replace_inner(
        &mut self,
        id: StickerId,
        layer: StickerLayerId,
        target: PixelTarget,
        new: PixelData,
        label: &str,
        before: Option<StructureSnapshot>,
    ) -> Result<(), (EditError, Option<Box<StructureSnapshot>>)> {
        let before = before.map(Box::new);
        let current = match self.pixel_target(id, layer, target) {
            Ok(c) => c,
            Err(e) => return Err((e, before)),
        };
        let size = new.size();
        match (current, new) {
            (Current::Owned(old), PixelData::Rgba(new)) if [old.width, old.height] == size => {
                let patch = TilePatch::diff(&*old, &new);
                self.put_pixels(id, layer, label, StickerPatch::Pixels(patch), PixelData::Rgba(new));
                Ok(())
            }
            (Current::Mask(old), PixelData::Alpha(new)) => {
                if [old.width, old.height] != size {
                    return Err((EditError::InvalidParam("a layer mask must have the size of its picture"), before));
                }
                let patch = MaskPatch::diff(&*old, &new);
                self.put_pixels(id, layer, label, StickerPatch::Mask(patch), PixelData::Alpha(new));
                Ok(())
            }
            (current, PixelData::Rgba(new)) => {
                let old_size = match &current {
                    Current::Owned(p) => [p.width, p.height],
                    Current::Blob(b) => match self.image_size(*b) {
                        Ok(s) => s,
                        Err(e) => return Err((e, before)),
                    },
                    Current::Mask(_) => return Err((EditError::InvalidParam("a mask takes one byte per pixel"), before)),
                };
                if let Current::Blob(b) = current
                    && old_size == size
                    && self.blob_picture(b).is_ok_and(|p| *p == new)
                {
                    // The same pixels: only a promotion is recorded.
                    if let Some(before) = before {
                        self.push_structure_edit(label, *before);
                    }
                    return Ok(());
                }
                if old_size != size && (size[0] > MAX_CREATED_SIDE || size[1] > MAX_CREATED_SIDE) {
                    return Err((
                        EditError::StickerLimit("pictures made in the sticker editor can be at most 4096 pixels on a side"),
                        before,
                    ));
                }
                let Some(old) = self.sticker_state.map.get(&id).cloned() else {
                    return Err((EditError::UnknownSticker, before));
                };
                let mut doc = (*old).clone();
                let version = self.next_pixel_version();
                if let Some(l) = doc.layer_mut(layer) {
                    if l.mask.as_ref().is_some_and(|m| [m.pixels.width, m.pixels.height] != size) {
                        return Err((EditError::InvalidParam("a layer mask must have the size of its picture"), before));
                    }
                    l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(new), version });
                }
                if let Err(e) = self.check_sticker(&doc) {
                    return Err((e, before));
                }
                let before = before.unwrap_or_else(|| Box::new(self.snapshot_structure(&[])));
                self.forget_sticker_pictures(id);
                self.sticker_state.map.insert(id, Arc::new(doc));
                self.push_structure_edit(label, *before);
                self.rebake_users_final(id);
                Ok(())
            }
            (_, PixelData::Alpha(_)) => Err((EditError::InvalidParam("a picture takes RGBA pixels"), before)),
        }
    }

    /// Puts `new` (same size as the current buffer) into the layer with a new version and pushes `patch` (the tiles
    /// of the old buffer where they differ); nothing when the patch is empty.
    fn put_pixels(&mut self, id: StickerId, layer: StickerLayerId, label: &str, patch: StickerPatch, new: PixelData) {
        if patch.is_empty() {
            return;
        }
        let version = self.next_pixel_version();
        self.forget_sticker_pictures(id);
        if let Some(l) = self.sticker_state.map.get_mut(&id).and_then(|arc| Arc::make_mut(arc).layer_mut(layer)) {
            match new {
                PixelData::Rgba(p) => l.content = StickerContent::Picture(PicturePixels::Owned { pixels: Arc::new(p), version }),
                PixelData::Alpha(a) => {
                    if let Some(m) = &mut l.mask {
                        m.pixels = Arc::new(a);
                        m.version = version;
                    }
                }
            }
        }
        self.push_edit(label, Edit::StickerPixels { sticker: id, layer, patch });
        self.rebake_users_final(id);
    }

    // ----- conversion and Revert (2.8) -----

    /// "Convert to Sticker" of a text stamp (2.8, review A24): a sticker with one Text layer whose em is the stamp's
    /// current raster size (`text_raster_px`, 32..=512), so the artboard has exactly the pixels of today's text picture
    /// (the frame's size; the anchor at the layer's `translate`) and the stored placement is unchanged. The stamp's
    /// styles are kept, their lengths multiplied by `em / 128` (2.3). The linked layers become `LayerKind::Stamp`.
    /// One entry ("Convert to Sticker"); the stamp is re-baked (the picture is the same within rounding). Errors:
    /// `UnknownStamp`, `InvalidParam` (not a text stamp, an empty text), `LayerLocked`, `StrokeInProgress`, `Text`.
    pub fn convert_text_stamp(&mut self, stamp: StampId) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let object = self.stamp(stamp).ok_or(EditError::UnknownStamp)?.clone();
        let StampSource::Text(spec) = &object.source else {
            return Err(EditError::InvalidParam("only a text stamp can be converted this way"));
        };
        if self.stamp_locked(stamp) {
            return Err(EditError::LayerLocked);
        }
        self.stamp_text_frame(stamp)?;
        let mut layer_spec = spec.clone();
        layer_spec.size_px = self.text_raster_px(stamp);
        let frame = match self.frame_of_spec(&layer_spec) {
            Ok(f) => f,
            Err(_) => {
                layer_spec = spec.clone();
                self.frame_of_spec(&layer_spec)?
            }
        };
        // The too-large fallback draws at a smaller em than asked: the layer takes the em that is drawn.
        if frame.em_px.is_finite() && frame.em_px > 0.0 && frame.em_px < layer_spec.size_px {
            layer_spec.size_px = frame.em_px;
        }
        let [w, h] = frame.size;
        if w == 0 || h == 0 {
            return Err(EditError::InvalidParam("the text has no pixels"));
        }
        let em = layer_spec.size_px;
        let mut doc = StickerDoc::new(&object.name, w, h);
        let lid = doc.alloc_layer();
        let mut layer = StickerLayer::new(lid, &object.name, StickerContent::Text(Box::new(layer_spec)));
        layer.transform = LayerTransform::moved(frame.anchor_px[0], frame.anchor_px[1]);
        doc.layers.push(layer);
        doc.sanitize();
        self.check_sticker(&doc)?;
        self.begin_pending(CONVERT_LABEL);
        let id = StickerId(self.alloc_id());
        self.sticker_state.map.insert(id, Arc::new(doc));
        let new_object = StampObject {
            source: StampSource::Sticker { sticker: id },
            styles: object.styles.scaled(em / STYLE_REFERENCE_EM).sanitized(),
            ..object.clone()
        };
        let (old_frame, new_frame) = (self.styled_frame(&object), self.styled_frame(&new_object));
        let placement = self.refit_auto_depth_framed(&object.placement, &old_frame, object.placement, &new_frame);
        if let Some(s) = self.stamps.iter_mut().find(|s| s.id == stamp) {
            *s = StampObject { placement, ..new_object };
        }
        self.set_linked_kind(stamp, false);
        self.text_mips.remove(&stamp);
        self.mark_stamp_edited(stamp, Commit::Final);
        self.flush_pending_edit();
        Ok(id)
    }

    /// The state the editor's Revert goes back to (2.8): the named stamps and stickers, plus the sticker of every named
    /// sticker stamp (`Arc` clones). Unknown ids are left out.
    pub fn sticker_checkpoint(&self, stamps: &[StampId], stickers: &[StickerId]) -> StickerCheckpoint {
        let stamps: Vec<StampObject> = self.stamps.iter().filter(|s| stamps.contains(&s.id)).cloned().collect();
        let mut ids: Vec<StickerId> = stickers.to_vec();
        ids.extend(stamps.iter().filter_map(|s| match s.source {
            StampSource::Sticker { sticker } => Some(sticker),
            _ => None,
        }));
        ids.sort_unstable();
        ids.dedup();
        let stickers = ids.into_iter().filter_map(|id| self.sticker_state.map.get(&id).map(|d| (id, Arc::clone(d)))).collect();
        StickerCheckpoint { stamps, stickers }
    }

    /// "Revert" in the editor (3.1): one `Structure` entry `label` that puts the stickers and stamps of `to` back
    /// (stamps deleted since are not brought back; a stamp's linked layers follow a change between text and picture).
    /// The affected stamps are re-baked at Final quality. Nothing happens when everything is as in `to`. Errors before
    /// anything changes: `StrokeInProgress`, `LayerLocked` (an affected stamp is locked).
    pub fn restore_sticker_state(&mut self, to: &StickerCheckpoint, label: &str) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let stamps: Vec<StampObject> =
            to.stamps.iter().filter(|s| self.stamp(s.id).is_some_and(|cur| *cur != **s)).cloned().collect();
        let stickers: Vec<(StickerId, Arc<StickerDoc>)> = to
            .stickers
            .iter()
            .filter(|(id, d)| self.sticker_state.map.get(id).is_none_or(|cur| !Arc::ptr_eq(cur, d) && **cur != **d))
            .cloned()
            .collect();
        if stamps.is_empty() && stickers.is_empty() {
            return Ok(());
        }
        let mut affected: Vec<StampId> = stamps.iter().map(|s| s.id).collect();
        for (id, _) in &stickers {
            affected.extend(self.sticker_users(*id));
            affected.extend(to.stamps.iter().filter(|s| s.source == StampSource::Sticker { sticker: *id }).map(|s| s.id));
        }
        affected.sort_unstable();
        affected.dedup();
        affected.retain(|id| self.stamp(*id).is_some());
        if affected.iter().any(|id| self.stamp_locked(*id)) {
            return Err(EditError::LayerLocked);
        }
        self.begin_pending(label);
        for (id, doc) in stickers {
            self.forget_sticker_pictures(id);
            self.sticker_state.map.insert(id, doc);
        }
        for object in stamps {
            let was_text = self.stamp(object.id).is_some_and(|s| matches!(s.source, StampSource::Text(_)));
            let is_text = matches!(object.source, StampSource::Text(_));
            if was_text != is_text {
                self.set_linked_kind(object.id, is_text);
            }
            if let Some(s) = self.stamps.iter_mut().find(|s| s.id == object.id) {
                *s = object;
            }
        }
        let stamps = &self.stamps;
        self.text_mips.retain(|id, (cached, _)| {
            stamps.iter().any(|s| s.id == *id && matches!(&s.source, StampSource::Text(spec) if *spec == *cached))
        });
        for id in affected {
            self.mark_stamp_edited(id, Commit::Final);
        }
        self.flush_pending_edit();
        Ok(())
    }

    // ----- re-bakes -----

    /// Re-bakes the users of sticker `id` (2.8). While a stroke on this sticker is open: the live preview, on this
    /// thread at `quality` (normally `Preview`), no history; the stroke's end bakes at Final, its cancel puts the
    /// earlier pictures back. Otherwise a Final bake, deferred (the users become unbaked) when deferred Final bakes are
    /// on (a `Preview` outside a stroke is never left on the model).
    pub fn rebake_sticker_users(&mut self, id: StickerId, quality: BakeQuality) {
        let stroking = self.sticker_state.stroke.as_ref().is_some_and(|s| s.sticker == id);
        if stroking {
            for u in self.sticker_users(id) {
                if let Err(e) = self.bake_and_apply(u, quality) {
                    log::warn!("stamp {}: the preview bake failed: {e}", u.0);
                }
            }
            if let Some(s) = self.sticker_state.stroke.as_mut() {
                s.previewed = true;
            }
        } else {
            self.rebake_users_final(id);
        }
    }

    /// Final re-bake of the users of `id` after a pixel change (deferred when that mode is on).
    pub(crate) fn rebake_users_final(&mut self, id: StickerId) {
        for u in self.sticker_users(id) {
            self.sticker_state.cache.styled.remove(&u);
            if self.defer_final_bakes {
                self.bakes.unbaked.insert(u);
            } else {
                if let Err(e) = self.bake_and_apply(u, BakeQuality::Full) {
                    log::warn!("stamp {}: the final bake failed, the earlier picture stays: {e}", u.0);
                }
                self.remove_empty_linked(u);
                self.bakes.unbaked.remove(&u);
            }
        }
    }

    /// After an undo or redo: re-bakes the users of every sticker whose pixels `Edit::StickerPixels` swapped and that
    /// still exists (a structure restore in the same entry may have taken it away).
    pub(crate) fn settle_sticker_pixel_edits(&mut self) {
        let swapped = std::mem::take(&mut self.sticker_state.swapped);
        for id in swapped {
            if self.sticker_state.map.contains_key(&id) {
                self.forget_sticker_pictures(id);
                self.rebake_users_final(id);
            }
        }
    }

    /// Applies (swaps) the patch of an `Edit::StickerPixels` (history.rs `apply_edit`): copy-on-write of the sticker and
    /// the buffer, a new version; the users are re-baked by `settle_sticker_pixel_edits`.
    pub(crate) fn swap_sticker_patch(&mut self, sticker: StickerId, layer: StickerLayerId, patch: &mut StickerPatch) {
        let version = self.next_pixel_version();
        self.forget_sticker_pictures(sticker);
        let Some(l) = self.sticker_state.map.get_mut(&sticker).and_then(|arc| Arc::make_mut(arc).layer_mut(layer)) else {
            debug_assert!(false, "history: sticker pixels for {sticker:?} / {layer:?}, which do not exist");
            return;
        };
        match (patch, &mut l.content, &mut l.mask) {
            (StickerPatch::Pixels(p), StickerContent::Picture(PicturePixels::Owned { pixels, version: v }), _) => {
                p.swap_with(make_mut_counted(pixels));
                *v = version;
            }
            (StickerPatch::Mask(p), _, Some(m)) => {
                p.swap_with(make_mut_counted(&mut m.pixels));
                m.version = version;
            }
            _ => debug_assert!(false, "history: sticker pixels for a buffer that is not there"),
        }
        self.sticker_state.swapped.insert(sticker);
    }

    // ----- internals -----

    /// `Some((stamp, scope))` when `on` names an image stamp that the edit promotes first (2.10); `None` for a sticker
    /// id or a sticker stamp. `Err(UnknownStamp)` for an unknown stamp.
    pub(crate) fn promotion_of(&self, on: StickerRef) -> Result<Option<(StampId, ShareScope)>, EditError> {
        match on {
            StickerRef::Id(_) => Ok(None),
            StickerRef::OfStamp(stamp, scope) => match self.stamp(stamp).ok_or(EditError::UnknownStamp)?.source {
                StampSource::Image { .. } => Ok(Some((stamp, scope))),
                _ => Ok(None),
            },
        }
    }

    /// Starts the pending entry of a promotion inside a structural edit (2.10): `before` (taken before the promotion)
    /// becomes the pending entry's "before" unless an entry with the same label is pending already.
    pub(crate) fn begin_pending_with(&mut self, label: &str, before: StructureSnapshot) {
        if self.pending_edit.as_ref().is_some_and(|p| p.label == label) {
            return;
        }
        self.flush_pending_edit();
        self.pending_edit = Some(Box::new(PendingEdit { label: label.to_owned(), before }));
    }

    /// Checks a pixel edit of `target` on layer `layer` of sticker `id` and returns the buffer it changes. Read only.
    fn pixel_target(&mut self, id: StickerId, layer: StickerLayerId, target: PixelTarget) -> Result<Current, EditError> {
        let doc = Arc::clone(self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?);
        if self.sticker_users(id).iter().any(|u| self.stamp_locked(*u)) {
            return Err(EditError::LayerLocked);
        }
        let l = doc.layer(layer).ok_or(EditError::UnknownStickerLayer)?;
        if l.locked {
            return Err(EditError::LayerLocked);
        }
        let StickerContent::Picture(picture) = &l.content else {
            return Err(EditError::InvalidParam("only a picture layer can be painted"));
        };
        match target {
            PixelTarget::Mask => match &l.mask {
                Some(m) => Ok(Current::Mask(Arc::clone(&m.pixels))),
                None => Err(EditError::InvalidParam("the layer has no mask")),
            },
            PixelTarget::Pixels => match picture {
                PicturePixels::Owned { pixels, .. } => Ok(Current::Owned(Arc::clone(pixels))),
                PicturePixels::Blob(b) => {
                    self.image_size(*b)?;
                    Ok(Current::Blob(*b))
                }
            },
        }
    }

    /// Drops the cached renders and styled stamp pictures made from the current value of sticker `id` (they hold its
    /// `Arc`): called before every write to it, after which they would be stale.
    pub(crate) fn forget_sticker_pictures(&mut self, id: StickerId) {
        let state = &mut *self.sticker_state;
        let Some(doc) = state.map.get(&id) else { return };
        state.cache.renders.retain(|e| !Arc::ptr_eq(&e.doc, doc));
        state.cache.styled.retain(|_, e| !matches!(&e.base, BaseIdentity::Sticker(d, _) if Arc::ptr_eq(d, doc)));
    }

    /// Gives the linked layers of `stamp` the kind of a text stamp (`text`) or of a picture stamp, keeping their caches.
    fn set_linked_kind(&mut self, stamp: StampId, text: bool) {
        for set in &mut self.sets {
            for l in &mut set.layers {
                let kind = match &l.kind {
                    LayerKind::Text { stamp: s, cache } if *s == stamp && !text => LayerKind::Stamp { stamp, cache: Arc::clone(cache) },
                    LayerKind::Stamp { stamp: s, cache } if *s == stamp && text => LayerKind::Text { stamp, cache: Arc::clone(cache) },
                    _ => continue,
                };
                l.kind = kind;
            }
        }
    }
}
