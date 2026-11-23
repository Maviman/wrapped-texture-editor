// Owner: core-stamps-text (C4). Stamp and text commands of the document (ARCHITECTURE.md 5.20, algorithm 6.5): source
// blobs and their decoded / rasterized mip caches, the linked-layer maintenance shared by every bake, the Preview /
// Final edit pipeline, the text raster resolution and `stamp_at`.
//
// Choices the contract leaves open, all documented at the items below:
// - A command that would change nothing returns `Ok` without a history entry (like the layer commands of 5.17).
// - The source kind of a stamp (image or text) cannot change: `set_stamp_source` with the other kind is `InvalidParam`.
// - A UV-placed stamp lives in the set of its placement: `add_stamp` requires `home_set == placement.set`, and
//   `set_stamp_placement` moving a UV stamp into another set moves its `home_set` along.
// - A stamp is protected by a lock on any of its linked layers (`Document::stamp_locked`): `remove_stamp` and the
//   `set_stamp_*` edits refuse to change it (an unchanged value is still accepted as a no-op).
// - `duplicate_stamp` gives the copies the properties of the originals except the lock, and a copy becomes its set's
//   active layer where the original was active (like `duplicate_layer` for paint layers).
// - A re-bake whose result equals the current cache byte for byte keeps the current `Arc` (no dirty rect, no new
//   layer version).

use std::sync::Arc;

use glam::Vec2;

use crate::blob::BlobKind;
use crate::document::{Commit, Document};
use crate::error::{EditError, TextError};
use crate::ids::{BlobId, LayerId, StampId, TextureSetId};
use crate::image_io::decode_image;
use crate::layer::{Layer, LayerKind, TextureSet};
use crate::math::IRect;
use crate::pixel::{PixelBuf, SparseBuf};
use crate::pixel_ops::{PremulMips, resize, unpremultiply};
use crate::stamp::{BakeQuality, Placement, Projector, StampObject, StampSource, SurfaceMode, text_stamp_name};
use crate::stamp_bake::{bake_projected, bake_uv, fit_projector_depth, projector_depth_is_auto};
use crate::stamp_job::{FinalBake, JobPicture, OwnFace, StampBakeJob, StampBakeOutput, StampJobKind, WrapPreview, text_fonts_of, wrap_bake};
use crate::texel_map::SurfacePoint;
use crate::wrap_param::{WrapMeshCell, WrapMeshWarmup, WrapParam, WrapStats, wrap_param_in};
use crate::text::frame::{Keep, refit_placement};
use crate::text::layout::raster_px;
use crate::text::{
    DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE, FontRef, RasterQuality, TEXT_PLACEMENT_PX, TextFonts, TextFrame, TextRaster,
    TextSpec, default_font_bytes, font_file_extension, render_text, text_frame,
};

/// History label of every `set_stamp_*` edit; all `Commit::Preview` calls of one drag share it.
const EDIT_LABEL: &str = "Edit Stamp";
/// Largest side of a decoded stamp image (6.5).
const MAX_STAMP_IMAGE_SIDE: u32 = 8192;
/// Raster resolution limits of a text stamp (6.5).
const MIN_TEXT_PX: f32 = 32.0;
const MAX_TEXT_PX: f32 = 512.0;
/// Most pixels of the picture of a styled text while it is edited: `set_stamp_source(.., Commit::Preview)` and every
/// `BakeQuality::Preview` bake (FEATURE_TEXT 5.8). A `Final`, a flush and a `Full` bake use the full picture.
pub const TEXT_EDIT_PREVIEW_PIXELS: u32 = 1 << 20;

/// The picture quality a bake of `quality` needs (FEATURE_TEXT 5.8).
fn raster_quality(quality: BakeQuality) -> RasterQuality {
    match quality {
        BakeQuality::Preview => RasterQuality::Preview { max_pixels: TEXT_EDIT_PREVIEW_PIXELS },
        BakeQuality::Full => RasterQuality::Full,
    }
}

/// The picture quality an edit with `commit` prepares: a `Final` bakes at full quality right away.
fn commit_quality(commit: Commit) -> RasterQuality {
    match commit {
        Commit::Preview => raster_quality(BakeQuality::Preview),
        Commit::Final => RasterQuality::Full,
    }
}

/// The picture quality an edit with `commit` prepares on a document: `commit_quality`, except that with deferred Final
/// bakes on (round 5) a `Final` bakes nothing here, so it prepares no more than a Preview would.
fn edit_quality(defer_final_bakes: bool, commit: Commit) -> RasterQuality {
    if commit == Commit::Final && defer_final_bakes { raster_quality(BakeQuality::Preview) } else { commit_quality(commit) }
}

/// True for the error of a text picture beyond the side or area limit (the one error the fallback answers).
fn is_too_large(e: &EditError) -> bool {
    matches!(e, EditError::Text(TextError::TooLarge { .. }))
}

/// The sizes the too-large fallback tries after the size of `spec` (FEATURE_TEXT 5.11 "Where it runs", item 2): `S / 2`,
/// `S / 4`, ... while above 32, then 32 (when `S` itself is above 32).
fn fallback_sizes(spec: &TextSpec) -> Vec<f32> {
    let s = raster_px(spec);
    let mut out = Vec::new();
    let mut c = s / 2.0;
    while c > MIN_TEXT_PX {
        out.push(c);
        c /= 2.0;
    }
    if s > MIN_TEXT_PX {
        out.push(MIN_TEXT_PX);
    }
    out
}

/// Which point of a text keeps its place when its spec changes from `old` to `new` (FEATURE_TEXT 5.11): the
/// alignment side for a change of spacing alone without warp or line, the anchor for everything else.
fn kept_point(old: &TextSpec, new: &TextSpec) -> Keep {
    let straight = |s: &TextSpec| s.fx.warp.is_none() && s.fx.path.is_none();
    if old.differs_only_in_spacing(new) && straight(old) && straight(new) { Keep::AlignEdge } else { Keep::Anchor }
}

/// The number of text pictures this thread has rendered through the document (FEATURE_TEXT 8.1
/// `the_frame_needs_no_pixels_after_undo_and_load`; round 4 T7: the app test `add_stamp_reuses_only_a_full_preview`
/// reads it too, so it exists in every build: one thread-local add per picture).
pub mod raster_counter {
    use std::cell::Cell;

    thread_local! {
        static COUNT: Cell<u64> = const { Cell::new(0) };
    }

    pub(crate) fn bump() {
        COUNT.with(|c| c.set(c.get() + 1));
    }

    /// Text pictures rendered by this thread so far.
    pub fn get() -> u64 {
        COUNT.with(Cell::get)
    }
}

/// True when every float of the placement is finite.
fn placement_is_finite(placement: &Placement) -> bool {
    match placement {
        Placement::Projected(p) => p.is_finite(),
        Placement::Uv(p) => p.is_finite(),
    }
}

/// Refits the placement from an image of `old` pixels to one of `new` pixels. The user's stretch relative to the old
/// image (placement aspect / old image aspect, e.g. from a Shift+corner drag) is kept and applied to the new image's
/// aspect; the larger side of the placement keeps its length. So a replacement with the same aspect leaves the
/// placement unchanged, and one with another aspect is never drawn distorted beyond what the user chose. Without a
/// usable old size the placement simply follows the new image's aspect. Centre, rotation, flips and depth are kept.
fn refit_aspect(placement: &mut Placement, old: Option<[u32; 2]>, new: [u32; 2]) {
    let [iw, ih] = new;
    if iw == 0 || ih == 0 {
        return;
    }
    let new_aspect = iw as f32 / ih as f32;
    let old_aspect = old.filter(|[w, h]| *w > 0 && *h > 0).map(|[w, h]| w as f32 / h as f32);
    let fit = |s: Vec2| -> Vec2 {
        let larger = s.x.abs().max(s.y.abs());
        if !(larger > 0.0 && larger.is_finite()) || old_aspect == Some(new_aspect) {
            return s;
        }
        let current = s.x.abs() / s.y.abs();
        let stretch = match old_aspect {
            Some(a0) if current.is_finite() && current > 0.0 => current / a0,
            _ => 1.0,
        };
        let target = stretch * new_aspect;
        let target = if target.is_finite() && target > 0.0 { target } else { new_aspect };
        let (w, h) = if target >= 1.0 { (larger, larger / target) } else { (larger * target, larger) };
        Vec2::new(w.copysign(s.x), h.copysign(s.y))
    };
    match placement {
        Placement::Projected(p) => p.half_size = fit(p.half_size),
        Placement::Uv(p) => p.size_px = fit(p.size_px),
    }
}

/// Removes the layer at `index` of `set`; when it was the active layer, the layer below (else above, else none) becomes
/// active. Marks the removed layer's rect dirty.
fn remove_layer_at(set: &mut TextureSet, index: usize) {
    set.mark_layer_content_dirty(index);
    let removed = set.layers.remove(index);
    if set.active == Some(removed.id) {
        let neighbour = index.checked_sub(1).or((index < set.layers.len()).then_some(index));
        set.active = neighbour.map(|i| set.layers[i].id);
    }
}

/// The cache of a stamp/text layer.
fn cache_mut(layer: &mut Layer) -> Option<&mut Arc<SparseBuf>> {
    match &mut layer.kind {
        LayerKind::Raster(_) => None,
        LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => Some(cache),
    }
}

impl Document {
    /// `p` with its depth range fitted to this document's surface under the footprint (`fit_projector_depth`, 6.5).
    /// Not a command: the caller applies the result with `add_stamp` / `set_stamp_placement`.
    pub fn fit_stamp_depth(&self, p: &Projector) -> Projector {
        fit_projector_depth(&self.mesh, &self.bvh, p)
    }

    /// True when the depth range of `p` is automatic (`projector_depth_is_auto`, 6.5): moving or scaling the stamp
    /// re-fits it. False once the user typed in a depth of their own.
    pub fn stamp_depth_is_auto(&self, p: &Projector) -> bool {
        projector_depth_is_auto(&self.mesh, &self.bvh, p)
    }

    /// `new` (a changed footprint of the placement `old`) with its depth range re-fitted when `old` is projected with
    /// an automatic depth; otherwise (UV placement, hand-set depth, nothing changed) `new` as it is.
    fn refit_auto_depth(&self, old: &Placement, new: Placement) -> Placement {
        match (old, new) {
            (Placement::Projected(o), Placement::Projected(n)) if n != *o && self.stamp_depth_is_auto(o) => {
                Placement::Projected(self.fit_stamp_depth(&n))
            }
            _ => new,
        }
    }

    /// Stores an encoded image (png/jpeg/tga/bmp/tiff/webp/dds) as a blob after verifying that it decodes.
    ///
    /// The decoded image (at most 8192 px per side) is kept as the stamp source cache. Identical bytes give the
    /// existing blob. `Err(Image)` when the bytes do not decode. Not a history command.
    pub fn add_image_blob(&mut self, name: &str, bytes: Arc<[u8]>) -> Result<BlobId, EditError> {
        let decoded = decode_image(&bytes, Some(name), MAX_STAMP_IMAGE_SIDE)?;
        let blob = self.blobs.insert(BlobKind::Image, name, bytes);
        self.image_mips.entry(blob).or_insert_with(|| Arc::new(PremulMips::build(&decoded.pixels)));
        Ok(blob)
    }

    /// Pixel size of a stored image blob (after the 8192 px limit). `Err(UnknownBlob)` for an unknown or non-image blob.
    pub fn image_size(&mut self, blob: BlobId) -> Result<[u32; 2], EditError> {
        Ok(self.image_source(blob)?.size())
    }

    /// Thumbnail of a stored image blob (longest side = `side`). `Err(UnknownBlob)` for an unknown or non-image blob.
    pub fn blob_thumbnail(&mut self, blob: BlobId, side: u32) -> Result<PixelBuf, EditError> {
        let mips = self.image_source(blob)?;
        // The smallest mip level that still has `side` pixels on its longer side keeps the cost near O(side^2).
        let Some(level) = mips.levels.iter().rev().find(|l| l.width.max(l.height) >= side).or(mips.levels.first()) else {
            return Ok(PixelBuf::new(0, 0));
        };
        // The levels hold premultiplied pixels; `thumbnail` takes straight alpha.
        let mut straight = level.clone();
        for p in straight.data.as_chunks_mut::<4>().0 {
            *p = unpremultiply(*p);
        }
        // Size from the full image, so the aspect ratio does not drift with the rounding of the mip sizes.
        let [w, h] = mips.size();
        if side == 0 || w == 0 || h == 0 {
            return Ok(PixelBuf::new(0, 0));
        }
        let scaled = |short: u32, long: u32| ((side as u64 * short as u64 + long as u64 / 2) / long as u64).max(1) as u32;
        let (tw, th) = if w >= h { (side, scaled(h, w)) } else { (scaled(w, h), side) };
        Ok(resize(&straight, tw, th))
    }

    /// Stores a font file as a blob and returns a FontRef for TextSpec.
    ///
    /// The bytes are not checked here; a file fontdue cannot parse makes the text commands fail with `Err(Text)`.
    pub fn register_font(&mut self, family: &str, style: &str, face_index: u32, bytes: Arc<[u8]>) -> FontRef {
        // The file name the font gets when the project's files are extracted: with the extension of its container.
        let stem = if style.is_empty() { family.to_owned() } else { format!("{family}-{style}") };
        let name = format!("{stem}.{}", font_file_extension(&bytes));
        let blob = self.blobs.insert(BlobKind::Font, &name, bytes);
        let font = FontRef { blob, face_index, family: family.to_owned(), style: style.to_owned() };
        // Remembered for `find_font` (not saved, not part of the history).
        if !self.text_caches.font_refs.contains(&font) {
            self.text_caches.font_refs.push(font.clone());
        }
        font
    }

    /// The bundled Ubuntu Light, registered as a blob on first use (later calls return the same blob).
    pub fn default_font(&mut self) -> FontRef {
        let bytes = default_font_bytes();
        let existing = self.blobs.iter().find(|b| b.kind == BlobKind::Font && b.bytes[..] == *bytes).map(|b| b.id);
        match existing {
            Some(blob) => FontRef {
                blob,
                face_index: 0,
                family: DEFAULT_FONT_FAMILY.to_owned(),
                style: DEFAULT_FONT_STYLE.to_owned(),
            },
            None => self.register_font(DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE, 0, Arc::from(bytes)),
        }
    }

    /// Pixel size of the source image / rasterized text (for aspect-correct placement).
    ///
    /// Text is measured at its own `size_px` (callers building a new spec use `TEXT_PLACEMENT_PX`), after
    /// `sanitize_fx`, from geometry only: the size of its full picture (`text_frame`; for a plain text the classic
    /// painter's `measure_text`). No too-large fallback here (`stamp_source_frame` has it).
    pub fn stamp_source_size(&mut self, source: &StampSource) -> Result<[u32; 2], EditError> {
        match source {
            StampSource::Image { blob } => self.image_size(*blob),
            StampSource::Text(spec) => {
                let mut spec = spec.clone();
                spec.sanitize_fx();
                Ok(self.frame_of_spec(&spec)?.size)
            }
        }
    }

    /// Full bake (for text: after choosing the raster resolution, section 6.5); one linked layer per covered set (always
    /// one in `home_set`), inserted above each set's active layer; the layer in `home_set` becomes active there.
    /// One history entry ("Add Stamp" / "Add Text").
    ///
    /// With deferred Final bakes on (round 5, `set_defer_final_bakes`) nothing is baked here: a text gets its frame
    /// (geometry only, the same too-large fallback; the Text tool's full hover picture is still reused), the stamp
    /// object and an empty linked layer in `home_set` (made active) are inserted, the stamp is marked unbaked and the
    /// entry is pushed. The layers of the other covered sets appear when the bake is applied (`apply_final_bake`).
    ///
    /// The tint starts at `[255; 4]`. Errors (nothing changes): `UnknownSet`, `InvalidParam` for a non-finite placement
    /// or a UV placement in another set than `home_set`, `UnknownBlob` / `Image` for an image source that is missing or
    /// does not decode, `Text` when the text cannot be rasterized.
    pub fn add_stamp(&mut self, name: &str, source: StampSource, placement: Placement, home_set: TextureSetId) -> Result<StampId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        if self.set(home_set).is_none() {
            return Err(EditError::UnknownSet);
        }
        if !placement_is_finite(&placement) {
            return Err(EditError::InvalidParam("the stamp placement is not a finite number"));
        }
        if let Placement::Uv(p) = &placement {
            if self.set(p.set).is_none() {
                return Err(EditError::UnknownSet);
            }
            if p.set != home_set {
                return Err(EditError::InvalidParam("a UV stamp must live in the texture set of its placement"));
            }
        }
        // Prepare the source before anything changes. A text is sanitized (FEATURE_TEXT 4.1) and, when its picture
        // would exceed the side or area limit, drawn at the largest smaller size that fits (5.11, the too-large
        // fallback); the placement is kept as given (the caller sized it, e.g. from `stamp_source_frame`, which
        // applies the same fallback).
        let (source, text) = match source {
            StampSource::Image { blob } => {
                self.image_source(blob)?;
                (StampSource::Image { blob }, None)
            }
            StampSource::Text(mut spec) => {
                spec.sanitize_fx();
                // FEATURE_TEXT 5.14 (round 4 T7): the Text tool's hover picture is reused when it is the full picture
                // of this spec at its own size (not a too-large fallback); otherwise the text is drawn here.
                let reused = self.text_caches.preview.as_ref().and_then(|(cached, _, raster)| {
                    (*cached == spec && raster.full && raster.frame.em_px == raster_px(&spec)).then(|| TextRaster::clone(raster))
                });
                let (fitted, picture) = match reused {
                    Some(raster) => (spec.clone(), NewText::Picture(raster)),
                    None if self.defer_final_bakes => {
                        let (fitted, frame) = self.fit_frame(&spec)?;
                        (fitted, NewText::Frame(frame))
                    }
                    None => {
                        let (fitted, raster) = self.render_fitted(&spec, RasterQuality::Full)?;
                        (fitted, NewText::Picture(raster))
                    }
                };
                (StampSource::Text(fitted.clone()), Some((fitted, picture)))
            }
        };
        let label = if text.is_some() { "Add Text" } else { "Add Stamp" };

        let before = self.snapshot_structure(&[]);
        let id = StampId(self.alloc_id());
        let object = StampObject { id, name: name.to_owned(), source, placement, tint: [255; 4], home_set };
        let at = self.stamps.partition_point(|s| s.id < id);
        self.stamps.insert(at, object);
        let is_text = text.is_some();
        match text {
            Some((spec, NewText::Picture(raster))) => self.store_text_picture(id, spec, raster),
            Some((spec, NewText::Frame(frame))) => self.insert_text_frame(id, spec, frame),
            None => {}
        }

        self.retune_text(id);
        if self.defer_final_bakes {
            self.insert_linked_layer(id, home_set, name, is_text, SparseBuf::empty());
            if let Some(&(_, layer)) = self.stamp_layers(id).iter().find(|(set, _)| *set == home_set) {
                self.sets[home_set.index()].active = Some(layer);
            }
            self.bakes.unbaked.insert(id);
            self.push_structure_edit(label, before);
            return Ok(id);
        }
        if let Err(e) = self.bake_and_apply(id, BakeQuality::Full) {
            // Unreachable after the checks above; undo the partial change all the same.
            self.restore_structure(before);
            self.forget_text_picture(id);
            return Err(e);
        }
        self.remove_empty_linked(id);
        if let Some(&(_, layer)) = self.stamp_layers(id).iter().find(|(set, _)| *set == home_set) {
            self.sets[home_set.index()].active = Some(layer);
        }
        self.push_structure_edit(label, before);
        Ok(id)
    }

    /// Moves / resizes / rotates a stamp; `Commit::Preview` calls are folded into one history entry.
    ///
    /// Shape of 5.20: validate, `begin_pending("Edit Stamp")`, apply, Preview bake, and for `Commit::Final`
    /// `flush_pending_edit()`. A UV placement in another set moves the stamp's `home_set` to that set. Errors:
    /// `UnknownStamp`, `LayerLocked` (a linked layer is locked, `stamp_locked`), `InvalidParam` (non-finite),
    /// `UnknownSet`, `UnknownBlob`, `StrokeInProgress`.
    pub fn set_stamp_placement(&mut self, id: StampId, placement: Placement, commit: Commit) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?;
        if !placement_is_finite(&placement) {
            return Err(EditError::InvalidParam("the stamp placement is not a finite number"));
        }
        if let Placement::Uv(p) = &placement
            && self.set(p.set).is_none()
        {
            return Err(EditError::UnknownSet);
        }
        let changed = stamp.placement != placement;
        let source = stamp.source.clone();
        if !changed && !self.continues_edit() {
            return Ok(());
        }
        // After the no-op check: an unchanged value (e.g. the Final a text field sends when it loses the focus to the
        // lock icon) cannot violate the lock; a real change can.
        if changed && self.stamp_locked(id) {
            return Err(EditError::LayerLocked);
        }
        self.source_mips(id, &source, self.edit_quality(commit))?;

        self.begin_pending(EDIT_LABEL);
        if changed && let Some(stamp) = self.stamps.iter_mut().find(|s| s.id == id) {
            stamp.placement = placement;
            if let Placement::Uv(p) = placement {
                stamp.home_set = p.set;
            }
        }
        self.preview_edit(id, changed, commit);
        Ok(())
    }

    /// Replaces the source. For text the anchor rule of FEATURE_TEXT 5.11 places the new picture: the on-model size of
    /// one em is unchanged (section 6.5) and so is the on-model position of the kept point (the anchor; for a change
    /// of spacing alone without warp or line, the alignment side of the line box). `source.size_px` of an incoming
    /// Text source is ignored (the stamp keeps its current raster resolution until finalized), except when the new
    /// picture would exceed 8192 px on a side or, for a styled text, 16.8 megapixels: then the largest of half, a
    /// quarter, ... down to 32 px per em that fits is taken (the too-large fallback). The incoming spec is sanitized
    /// (`sanitize_fx`). A `Commit::Preview` of a styled text prepares a picture of at most
    /// `TEXT_EDIT_PREVIEW_PIXELS` pixels; the final bake draws it in full.
    /// For another image the larger side of the placement is kept and the other one follows the new image's aspect
    /// ratio, so a replacement is never drawn distorted. A projected placement whose depth range was automatic
    /// (`stamp_depth_is_auto`) gets it re-fitted to the new footprint (6.5); a hand-set depth is kept.
    ///
    /// The source kind cannot change (`Err(InvalidParam)`). Other errors as for `set_stamp_placement`, plus `Image` /
    /// `Text` when the new source cannot be decoded or rasterized (a text even at 32 px per em).
    pub fn set_stamp_source(&mut self, id: StampId, source: StampSource, commit: Commit) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?;
        let current = stamp.source.clone();
        let source = match (&current, source) {
            (StampSource::Image { .. }, StampSource::Image { blob }) => StampSource::Image { blob },
            (StampSource::Text(old), StampSource::Text(mut spec)) => {
                spec.size_px = old.size_px;
                spec.sanitize_fx();
                StampSource::Text(spec)
            }
            _ => return Err(EditError::InvalidParam("a stamp cannot switch between image and text")),
        };
        let changed = source != current;
        if !changed && !self.continues_edit() {
            return Ok(());
        }
        if changed && self.stamp_locked(id) {
            return Err(EditError::LayerLocked);
        }
        // Fix round 3, R3-9: a text stamp that still carries the automatic name of its old text is named after its new
        // text (stamp and linked layers, in the same history entry as the text). A name the user chose is kept.
        let new_name = match (&current, &source) {
            (StampSource::Text(old), StampSource::Text(new)) if changed => {
                let name = &stamp.name;
                let auto = text_stamp_name(&new.content);
                (*name == text_stamp_name(&old.content) && *name != auto).then_some(auto)
            }
            _ => None,
        };
        // Prepare the new picture and the placement before anything changes. The old frame comes from geometry only
        // (the frame cache, or a layout pass): the old text is not rasterized just to learn its size.
        let old_placement = self.stamp(id).map(|s| s.placement).ok_or(EditError::UnknownStamp)?;
        let mut placement = old_placement;
        let mut source = source;
        let mut text = None;
        if changed {
            match (&current, &source) {
                (StampSource::Text(old), StampSource::Text(new)) => {
                    let old_frame = self.stamp_text_frame(id)?;
                    // Round 5: with deferred Final bakes a Final draws no picture here (the bake job draws the full
                    // one): the frame comes from geometry with the same too-large fallback (`fit_frame`).
                    let (fitted, picture) = if commit == Commit::Final && self.defer_final_bakes {
                        let (fitted, frame) = self.fit_frame(new)?;
                        (fitted, NewText::Frame(frame))
                    } else {
                        let (fitted, raster) = self.render_fitted(new, commit_quality(commit))?;
                        (fitted, NewText::Picture(raster))
                    };
                    let new_frame = match &picture {
                        NewText::Picture(raster) => raster.frame,
                        NewText::Frame(frame) => *frame,
                    };
                    refit_placement(&mut placement, &old_frame, &new_frame, kept_point(old, &fitted));
                    source = StampSource::Text(fitted.clone());
                    text = Some((fitted, picture));
                }
                (StampSource::Image { blob: old }, StampSource::Image { blob }) => {
                    let new_size = self.image_source(*blob)?.size();
                    // The old image may be gone (a damaged project): then the placement is refitted to the new one.
                    let old_size = self.image_source(*old).ok().map(|m| m.size());
                    refit_aspect(&mut placement, old_size, new_size);
                }
                _ => {}
            }
            // Fix round 2, item 2: an automatic depth range follows the new footprint (longer text, wider image);
            // a depth typed in by hand is kept.
            placement = self.refit_auto_depth(&old_placement, placement);
        }

        self.begin_pending(EDIT_LABEL);
        if changed && let Some(stamp) = self.stamps.iter_mut().find(|s| s.id == id) {
            stamp.placement = placement;
            stamp.source = source;
        }
        match text {
            Some((spec, NewText::Picture(raster))) => self.store_text_picture(id, spec, raster),
            Some((spec, NewText::Frame(frame))) => self.insert_text_frame(id, spec, frame),
            None => {}
        }
        if let Some(name) = new_name {
            // Inside the pending edit: undo, and a cancel (Esc) of Preview edits, restore the old name with the text.
            self.set_stamp_name(id, &name);
        }
        self.preview_edit(id, changed, commit);
        Ok(())
    }

    /// Changes the tint of a stamp; `Commit::Preview` calls are folded into one history entry. `Err(LayerLocked)` while
    /// a linked layer is locked.
    pub fn set_stamp_tint(&mut self, id: StampId, tint: [u8; 4], commit: Commit) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?;
        let changed = stamp.tint != tint;
        let source = stamp.source.clone();
        if !changed && !self.continues_edit() {
            return Ok(());
        }
        if changed && self.stamp_locked(id) {
            return Err(EditError::LayerLocked);
        }
        self.source_mips(id, &source, self.edit_quality(commit))?;

        self.begin_pending(EDIT_LABEL);
        if changed && let Some(stamp) = self.stamps.iter_mut().find(|s| s.id == id) {
            stamp.tint = tint;
        }
        self.preview_edit(id, changed, commit);
        Ok(())
    }

    /// Renames a stamp; also renames the linked layers. One entry ("Rename Stamp"); none when nothing changes.
    pub fn rename_stamp(&mut self, id: StampId, name: &str) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?;
        let layers_named = self.sets.iter().flat_map(|s| &s.layers).filter(|l| l.stamp_id() == Some(id)).all(|l| l.name == name);
        if stamp.name == name && layers_named {
            return Ok(());
        }
        let before = self.snapshot_structure(&[]);
        self.set_stamp_name(id, name);
        self.push_structure_edit("Rename Stamp", before);
        Ok(())
    }

    /// Names stamp `id` and all its linked layers `name` (no history; the caller records it).
    fn set_stamp_name(&mut self, id: StampId, name: &str) {
        if let Some(stamp) = self.stamps.iter_mut().find(|s| s.id == id) {
            stamp.name = name.to_owned();
        }
        for layer in self.sets.iter_mut().flat_map(|s| &mut s.layers) {
            if layer.stamp_id() == Some(id) {
                layer.name = name.to_owned();
            }
        }
    }

    /// Removes the stamp object and all its linked layers (edits sets/stamps directly). One entry ("Delete Stamp").
    /// `Err(LayerLocked)` while any linked layer is locked (`stamp_locked`).
    ///
    /// In a set whose active layer is removed, the layer below (else above, else none) becomes active.
    pub fn remove_stamp(&mut self, id: StampId) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        if self.stamp(id).is_none() {
            return Err(EditError::UnknownStamp);
        }
        if self.stamp_locked(id) {
            return Err(EditError::LayerLocked);
        }
        let before = self.snapshot_structure(&[]);
        for set in &mut self.sets {
            if let Some(index) = set.layers.iter().position(|l| l.stamp_id() == Some(id)) {
                remove_layer_at(set, index);
            }
        }
        self.stamps.retain(|s| s.id != id);
        self.forget_text_picture(id);
        self.bakes.wrap_params.remove(id);
        self.preview_baked.remove(&id);
        self.bakes.preview_stale.remove(&id);
        self.bakes.unbaked.remove(&id);
        self.push_structure_edit("Delete Stamp", before);
        Ok(())
    }

    /// New stamp "<name> copy" with the same source/placement/tint; its linked layers (Arc-shared caches) are inserted
    /// directly above the originals. One entry ("Duplicate Stamp").
    ///
    /// The copies keep visibility, opacity, blend mode and alpha lock (not the lock); where an original layer was its
    /// set's active layer, the copy becomes active.
    pub fn duplicate_stamp(&mut self, id: StampId) -> Result<StampId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let original = self.stamp(id).ok_or(EditError::UnknownStamp)?.clone();
        let before = self.snapshot_structure(&[]);

        let copy_id = StampId(self.alloc_id());
        let name = format!("{} copy", original.name);
        for s in 0..self.sets.len() {
            let Some(index) = self.sets[s].layers.iter().position(|l| l.stamp_id() == Some(id)) else { continue };
            let layer_id = LayerId(self.alloc_id());
            let version = self.fresh_version();
            let set = &mut self.sets[s];
            let source = &set.layers[index];
            let kind = match &source.kind {
                LayerKind::Stamp { cache, .. } => LayerKind::Stamp { stamp: copy_id, cache: Arc::clone(cache) },
                LayerKind::Text { cache, .. } => LayerKind::Text { stamp: copy_id, cache: Arc::clone(cache) },
                LayerKind::Raster(_) => continue,
            };
            let copy = Layer {
                id: layer_id,
                name: name.clone(),
                visible: source.visible,
                opacity: source.opacity,
                blend: source.blend,
                locked: false,
                alpha_lock: source.alpha_lock,
                kind,
                version,
                content: None,
            };
            let was_active = set.active == Some(source.id);
            set.layers.insert(index + 1, copy);
            if was_active {
                set.active = Some(layer_id);
            }
            set.mark_layer_content_dirty(index + 1);
        }
        let object = StampObject { id: copy_id, name, ..original };
        let at = self.stamps.partition_point(|s| s.id < copy_id);
        self.stamps.insert(at, object);
        if let Some(cached) = self.text_mips.get(&id).cloned() {
            self.text_mips.insert(copy_id, cached);
            // The copy's picture is a preview exactly when the original's is.
            if self.text_caches.preview_ids.contains(&id) {
                self.text_caches.preview_ids.insert(copy_id);
            } else {
                self.text_caches.preview_ids.remove(&copy_id);
            }
        }
        if let Some(frame) = self.text_caches.frames.get(&id).cloned() {
            self.text_caches.frames.insert(copy_id, frame);
        }
        // Round 5: the copy shares the caches, so its Full bake is due exactly when the original's is.
        if self.bakes.unbaked.contains(&id) {
            self.bakes.unbaked.insert(copy_id);
        }
        // Round 6: and the parameterization of a wrapped original (same placement).
        if let Some(param) = self.bakes.wrap_params.peek(id).cloned() {
            let stamps = &self.stamps;
            self.bakes.wrap_params.insert(copy_id, param, |s| stamps.iter().any(|o| o.id == s));
        }
        self.push_structure_edit("Duplicate Stamp", before);
        Ok(copy_id)
    }

    /// Topmost visible stamp/text layer whose cache has alpha > 0 at `uv` (wrapped).
    pub fn stamp_at(&self, set: TextureSetId, uv: Vec2) -> Option<StampId> {
        let set = self.set(set)?;
        if !uv.is_finite() || set.width == 0 || set.height == 0 {
            return None;
        }
        // Wrap in integer texel space (6.9 pitfall 1).
        let wrap = |c: f32, size: u32| ((c * size as f32).floor() as i64).rem_euclid(size as i64) as i32;
        let (x, y) = (wrap(uv.x, set.width), wrap(uv.y, set.height));
        set.layers.iter().rev().find_map(|layer| match &layer.kind {
            LayerKind::Stamp { stamp, cache } | LayerKind::Text { stamp, cache }
                if layer.visible && cache.get(x, y)[3] > 0 =>
            {
                Some(*stamp)
            }
            _ => None,
        })
    }

    /// Re-bakes one stamp without touching history (project load with a lost cache).
    ///
    /// Linked-layer maintenance of 6.5 without the removal of empty layers; the text raster resolution is kept.
    pub(crate) fn rebake_stamp(&mut self, id: StampId, quality: BakeQuality) -> Result<(), EditError> {
        if self.stamp(id).is_none() {
            return Err(EditError::UnknownStamp);
        }
        self.bake_and_apply(id, quality)
    }

    /// Brings a Preview-baked stamp to its final state, without history: (text only) pick `text_raster_px` and, if it
    /// changed, take the new size (frames only, no trial pictures) and apply the anchor rule; bake at `Full` (a preview
    /// picture is drawn again in full first); remove linked layers with an empty cache outside `home_set`;
    /// remove `id` from `preview_baked`. Unknown id: no-op. Called by `flush_pending_edit` (history.rs) and by Commit::Final.
    ///
    /// With deferred Final bakes on (round 5) only the cheap, history-visible part runs (`retune_text`: frames only)
    /// and the stamp is marked unbaked; no bake, no removal of empty layers (both come with `apply_final_bake`).
    pub(crate) fn finalize_stamp_bake(&mut self, id: StampId) {
        if self.defer_final_bakes {
            if self.stamp(id).is_some() {
                self.retune_text(id);
                self.bakes.unbaked.insert(id);
            }
        } else if self.stamp(id).is_some() {
            self.retune_text(id);
            if let Err(e) = self.bake_and_apply(id, BakeQuality::Full) {
                log::warn!("stamp {}: the final bake failed, the preview stays: {e}", id.0);
            }
            self.remove_empty_linked(id);
        }
        self.preview_baked.remove(&id);
        self.bakes.preview_stale.remove(&id);
    }

    /// Raster resolution for a text stamp: `clamp(next_power_of_two(ceil(em height in texels of home_set)), 32, 512)` (6.5).
    ///
    /// Needs the frame of the stamp's current spec (the frame cache, `stamp_text_frame`; FEATURE_TEXT 5.11: the height
    /// of the full picture, also while the cached picture is a preview), which every caller in this file establishes
    /// first; without it the current `size_px` is returned (no change). An image stamp or an unknown id gives
    /// `TEXT_PLACEMENT_PX`.
    pub(crate) fn text_raster_px(&self, id: StampId) -> f32 {
        let Some(stamp) = self.stamp(id) else { return TEXT_PLACEMENT_PX };
        let StampSource::Text(spec) = &stamp.source else { return TEXT_PLACEMENT_PX };
        let s = raster_px(spec);
        let h = match self.cached_text_frame(id) {
            Some(frame) => frame.size[1] as f32,
            None => return spec.size_px,
        };
        let em_texels = match &stamp.placement {
            Placement::Projected(p) => 2.0 * p.half_size.y.abs() * s / h / self.texel_world_size(stamp.home_set),
            Placement::Uv(p) => p.size_px.y.abs() * s / h,
        };
        if !(em_texels.is_finite() && em_texels > 0.0) {
            return MIN_TEXT_PX;
        }
        let bucket = (em_texels.min(MAX_TEXT_PX).ceil() as u32).next_power_of_two() as f32;
        bucket.clamp(MIN_TEXT_PX, MAX_TEXT_PX)
    }

    // ----- round 4: text frames, previews and fonts (FEATURE_TEXT 5.8, 5.11, 5.13, 5.14) -----
    //
    // Signatures by the scaffold S1; bodies by T4. The picture cache `text_mips` keeps its round-3 type; whether a
    // cached picture is a preview is kept in `text_caches.preview_ids` (every insert into `text_mips` in this file
    // goes through `store_text_picture`, which keeps the two in step), and a bake at `BakeQuality::Full` never uses
    // a preview picture (`text_source`).

    /// The frame of text stamp `id` for its current spec, from geometry only (no pixels); fills the frame cache.
    /// `Err(UnknownStamp)` for an unknown id, `Err(InvalidParam)` for an image stamp, `Err(Text)` when the text
    /// cannot be laid out.
    pub fn stamp_text_frame(&mut self, id: StampId) -> Result<TextFrame, EditError> {
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?;
        let StampSource::Text(spec) = &stamp.source else {
            return Err(EditError::InvalidParam("the stamp is not a text"));
        };
        let spec = spec.clone();
        if let Some(frame) = self.cached_text_frame(id) {
            return Ok(frame);
        }
        let frame = self.frame_of_spec(&spec)?;
        self.insert_text_frame(id, spec, frame);
        Ok(frame)
    }

    /// The cached frame of text stamp `id`; `None` when it is not cached for the stamp's current spec (after an undo
    /// or a load, or for an image stamp).
    pub fn cached_text_frame(&self, id: StampId) -> Option<TextFrame> {
        let StampSource::Text(spec) = &self.stamp(id)?.source else { return None };
        match self.text_caches.frames.get(&id) {
            Some((cached, frame)) if cached == spec => Some(*frame),
            _ => None,
        }
    }

    /// The frame of a text that is not placed yet (the Text tool): its spec is sanitized first. When the picture
    /// would exceed the side or area limit at the spec's size, the frame is the one `add_stamp` will draw: at the
    /// largest of half, a quarter, ... down to 32 px per em that fits (the too-large fallback, 5.11), so its `em_px`
    /// can be below `size_px`; a caller sizes the rectangle with `em_px`. `Err(InvalidParam)` for an image source,
    /// `Err(Text)` when the text cannot be laid out even at 32 px per em.
    pub fn stamp_source_frame(&mut self, source: &StampSource) -> Result<TextFrame, EditError> {
        let StampSource::Text(spec) = source else {
            return Err(EditError::InvalidParam("the source is not a text"));
        };
        let mut spec = spec.clone();
        spec.sanitize_fx();
        Ok(self.fit_frame(&spec)?.1)
    }

    /// A picture of `spec` with at most `max_pixels` pixels (the hover picture of the Text tool, FEATURE_TEXT 5.14);
    /// a one-entry cache: the same spec and limit give the same `Arc`. The spec is sanitized first. A text without
    /// outlines is always the full picture. A text too large for its size is drawn as `stamp_source_frame` frames it
    /// (`TextRaster::frame` tells the size).
    pub fn text_preview(&mut self, spec: &TextSpec, max_pixels: u32) -> Result<Arc<TextRaster>, EditError> {
        let mut spec = spec.clone();
        spec.sanitize_fx();
        if let Some((cached, limit, raster)) = self.text_caches.preview.as_ref()
            && *cached == spec
            && *limit == max_pixels
        {
            return Ok(Arc::clone(raster));
        }
        let (_, raster) = self.render_fitted(&spec, RasterQuality::Preview { max_pixels })?;
        let raster = Arc::new(raster);
        self.text_caches.preview = Some((spec, max_pixels, Arc::clone(&raster)));
        Ok(raster)
    }

    /// Whether the cached picture of text stamp `id` is the full picture (not a preview); `None` when no picture is
    /// cached (round 4 T7, read only: the hover-picture tests of FEATURE_TEXT 8.2).
    pub fn text_picture_is_full(&self, id: StampId) -> Option<bool> {
        self.text_mips.contains_key(&id).then(|| !self.text_caches.preview_ids.contains(&id))
    }

    /// A font of this document with that family (compared without regard to case) and style: first among the fonts
    /// of the text stamps, then among the fonts `register_font` handed out in this session. A font whose blob no
    /// longer exists is skipped. Read only.
    pub fn find_font(&self, family: &str, style: &str) -> Option<FontRef> {
        let matches = |f: &FontRef| {
            f.family.to_lowercase() == family.to_lowercase()
                && f.style == style
                && self.blobs.get(f.blob).is_some_and(|b| b.kind == BlobKind::Font)
        };
        let stamped = self.stamps.iter().filter_map(|s| match &s.source {
            StampSource::Text(spec) => Some(&spec.font),
            StampSource::Image { .. } => None,
        });
        stamped.chain(self.text_caches.font_refs.iter()).find(|f| matches(f)).cloned()
    }

    /// Runs `f` with the fonts of `spec`: its own face (when its blob exists) and the bundled fallback, at the
    /// resolution of the spec.
    fn with_text_fonts<R>(&mut self, spec: &TextSpec, f: impl FnOnce(&TextFonts<'_>) -> R) -> Result<R, EditError> {
        let (own, fallback) = self.text_fonts_owned(spec)?;
        Ok(f(&text_fonts_of(&own, &fallback)))
    }

    /// The fonts of `spec` as owned parts (`with_text_fonts`; round 5: what a bake job carries to its worker).
    fn text_fonts_owned(&mut self, spec: &TextSpec) -> Result<(Option<OwnFace>, Arc<fontdue::Font>), EditError> {
        let px = raster_px(spec);
        let fallback = self.fonts.fallback(px);
        let own = match self.fonts.get(&self.blobs, &spec.font, px) {
            Ok(font) => Some(font),
            Err(TextError::MissingFont) => None,
            Err(e) => return Err(e.into()),
        };
        let own = own.and_then(|metrics| {
            self.blobs.get(spec.font.blob).map(|b| (metrics, Arc::clone(&b.bytes), spec.font.face_index))
        });
        Ok((own, fallback))
    }

    /// The frame of `spec` at its own size (geometry only, no pixels).
    fn frame_of_spec(&mut self, spec: &TextSpec) -> Result<TextFrame, EditError> {
        Ok(self.with_text_fonts(spec, |fonts| text_frame(fonts, spec))??)
    }

    /// Renders `spec` at its own size (counted in the test build).
    fn render_spec(&mut self, spec: &TextSpec, quality: RasterQuality) -> Result<TextRaster, EditError> {
        raster_counter::bump();
        Ok(self.with_text_fonts(spec, |fonts| render_text(fonts, spec, quality))??)
    }

    /// `spec` and its frame at the largest size that fits the picture limits (the too-large fallback of 5.11): its
    /// own size, else the first of `fallback_sizes` whose frame fits (one `text_frame` call per try, no pixels). Only
    /// `TextError::TooLarge` makes it try a smaller size; when none fits, the error of the spec's own size is returned.
    fn fit_frame(&mut self, spec: &TextSpec) -> Result<(TextSpec, TextFrame), EditError> {
        match self.frame_of_spec(spec) {
            Ok(frame) => Ok((spec.clone(), frame)),
            Err(e) if is_too_large(&e) => self.fit_smaller(spec, e),
            Err(e) => Err(e),
        }
    }

    /// The fallback part of `fit_frame` after the spec's own size failed with `err`.
    fn fit_smaller(&mut self, spec: &TextSpec, err: EditError) -> Result<(TextSpec, TextFrame), EditError> {
        for size in fallback_sizes(spec) {
            let mut smaller = spec.clone();
            smaller.size_px = size;
            match self.frame_of_spec(&smaller) {
                Ok(frame) => return Ok((smaller, frame)),
                Err(e) if is_too_large(&e) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(err)
    }

    /// The picture of `spec` at `quality`, at the largest size that fits (`fit_frame`): one rendering when the spec's
    /// own size fits (the usual case costs what it costs today), else frames for the smaller sizes and one rendering.
    /// Returns the spec as drawn (its `size_px` is the size used).
    fn render_fitted(&mut self, spec: &TextSpec, quality: RasterQuality) -> Result<(TextSpec, TextRaster), EditError> {
        match self.render_spec(spec, quality) {
            Ok(raster) => Ok((spec.clone(), raster)),
            Err(e) if is_too_large(&e) => {
                let (fitted, _) = self.fit_smaller(spec, e)?;
                let raster = self.render_spec(&fitted, quality)?;
                Ok((fitted, raster))
            }
            Err(e) => Err(e),
        }
    }

    /// Caches the frame of `spec` for stamp `id`; entries of stamps that no longer exist are dropped (5.11).
    fn insert_text_frame(&mut self, id: StampId, spec: TextSpec, frame: TextFrame) {
        let live: std::collections::HashSet<StampId> = self.stamps.iter().map(|s| s.id).collect();
        self.text_caches.frames.retain(|id, _| live.contains(id));
        self.text_caches.frames.insert(id, (spec, frame));
    }

    /// Caches the picture `raster` of `spec` for stamp `id`: its mips in `text_mips`, whether it is a preview in
    /// `preview_ids`, and its (full-resolution) frame in the frame cache. The one way a picture enters `text_mips`.
    fn store_text_picture(&mut self, id: StampId, spec: TextSpec, raster: TextRaster) {
        let mips = Arc::new(PremulMips::build(&raster.image));
        self.store_text_mips(id, spec, raster.frame, raster.full, mips);
    }

    /// `store_text_picture` with the mips already built (round 5: by a bake job).
    fn store_text_mips(&mut self, id: StampId, spec: TextSpec, frame: TextFrame, full: bool, mips: Arc<PremulMips>) {
        if full {
            self.text_caches.preview_ids.remove(&id);
        } else {
            self.text_caches.preview_ids.insert(id);
        }
        self.insert_text_frame(id, spec.clone(), frame);
        self.text_mips.insert(id, (spec, mips));
    }

    /// Drops every cached picture and frame of stamp `id`.
    fn forget_text_picture(&mut self, id: StampId) {
        self.text_mips.remove(&id);
        self.text_caches.preview_ids.remove(&id);
        self.text_caches.frames.remove(&id);
    }

    // ----- internals -----

    /// `edit_quality` for this document's mode.
    fn edit_quality(&self, commit: Commit) -> RasterQuality {
        edit_quality(self.defer_final_bakes, commit)
    }

    /// True while a pending stamp edit is open: a further call continues it, even when it changes nothing.
    fn continues_edit(&self) -> bool {
        self.pending_edit.as_ref().is_some_and(|p| p.label == EDIT_LABEL)
    }

    /// The common tail of the `set_stamp_*` commands, after `begin_pending` and the change itself.
    fn preview_edit(&mut self, id: StampId, changed: bool, commit: Commit) {
        if changed {
            // A Final edit is baked at full quality right away by the flush below (`finalize_stamp_bake`), which
            // replaces every cache a Preview bake would write: it gets no Preview bake first (fix round 3, R3-4: each
            // Final edit cost two bakes). A deferred Preview bake still waiting (`preview_stale`) ends with that flush.
            if commit == Commit::Preview {
                if self.defer_preview_bakes || self.wrap_preview_deferred(id) {
                    // Baked by `bake_stale_previews` (or by the final bake of the flush); round 8: a wrapped stamp's by
                    // its Preview job (`next_wrap_preview_job`).
                    self.bakes.preview_stale.insert(id);
                } else if let Err(e) = self.bake_and_apply(id, BakeQuality::Preview) {
                    log::warn!("stamp {}: the preview bake failed: {e}", id.0);
                }
            }
            // Round 8: a new pending edit gets a new serial.
            if self.preview_baked.is_empty() {
                self.bakes.edit_serial += 1;
            }
            self.preview_baked.insert(id);
        }
        if commit == Commit::Final {
            self.flush_pending_edit();
        }
    }

    /// Defers the bakes of `Commit::Preview` stamp edits (default off). While on, a Preview edit only updates the stamp
    /// object (placement, source, tint; the gizmo follows at once) and marks the stamp stale; the caller bakes the stale
    /// stamps with `bake_stale_previews` when its frame budget allows (the app throttles them so that a slow bake of a
    /// large decal on a large texture cannot block every frame of a drag, a wheel gesture or typing). A `Final`, a
    /// flush or any other command bakes at full quality as before; a cancel restores the old caches.
    pub fn set_defer_preview_bakes(&mut self, on: bool) {
        self.defer_preview_bakes = on;
        if !on {
            self.bake_stale_previews();
        }
    }

    /// True when a deferred Preview bake is waiting (`set_defer_preview_bakes`). Round 8: wrapped stamps waiting for
    /// their Preview job (`set_defer_wrap_previews`) are not counted (`has_stale_wrap_previews`).
    pub fn has_stale_previews(&self) -> bool {
        self.bakes.preview_stale.iter().any(|&id| !self.wrap_preview_deferred(id))
    }

    /// Runs the deferred Preview bakes (ascending stamp id). No history entry; the stamps stay in the pending edit.
    /// Round 8: a wrapped stamp stays waiting for its Preview job while `set_defer_wrap_previews` is on.
    pub fn bake_stale_previews(&mut self) {
        let ids: Vec<StampId> = std::mem::take(&mut self.bakes.preview_stale).into_iter().collect();
        for id in ids {
            if self.stamp(id).is_none() {
                continue;
            }
            if self.wrap_preview_deferred(id) {
                self.bakes.preview_stale.insert(id);
                continue;
            }
            if let Err(e) = self.bake_and_apply(id, BakeQuality::Preview) {
                log::warn!("stamp {}: the preview bake failed: {e}", id.0);
            }
        }
    }

    /// Decoded mips of an image blob (cached per blob).
    fn image_source(&mut self, blob: BlobId) -> Result<Arc<PremulMips>, EditError> {
        if let Some(mips) = self.image_mips.get(&blob) {
            return Ok(Arc::clone(mips));
        }
        let stored = self.blobs.get(blob).filter(|b| b.kind == BlobKind::Image).ok_or(EditError::UnknownBlob)?;
        let decoded = decode_image(&stored.bytes, Some(&stored.name), MAX_STAMP_IMAGE_SIDE)?;
        let mips = Arc::new(PremulMips::build(&decoded.pixels));
        self.image_mips.insert(blob, Arc::clone(&mips));
        Ok(mips)
    }

    /// Mips of the picture of stamp `id` for `spec` at `quality` (cached per stamp in `text_mips`, rebuilt when the
    /// spec changes). A cached picture serves a `Preview` request whatever its quality; a `Full` request never uses a
    /// preview picture (FEATURE_TEXT 5.8): it draws the text again in full and replaces the cache.
    fn text_source(&mut self, id: StampId, spec: &TextSpec, quality: RasterQuality) -> Result<Arc<PremulMips>, EditError> {
        let wants_full = matches!(quality, RasterQuality::Full);
        if let Some((cached, mips)) = self.text_mips.get(&id)
            && cached == spec
            && !(wants_full && self.text_caches.preview_ids.contains(&id))
        {
            return Ok(Arc::clone(mips));
        }
        let raster = self.render_spec(spec, quality)?;
        self.store_text_picture(id, spec.clone(), raster);
        Ok(Arc::clone(&self.text_mips[&id].1))
    }

    /// Source pixels of a stamp; a text at `quality` (`text_source`).
    fn source_mips(&mut self, id: StampId, source: &StampSource, quality: RasterQuality) -> Result<Arc<PremulMips>, EditError> {
        match source {
            StampSource::Image { blob } => self.image_source(*blob),
            StampSource::Text(spec) => self.text_source(id, spec, quality),
        }
    }

    /// Text raster resolution (6.5): when `text_raster_px` differs from `size_px`, the text gets the new size and the
    /// anchor rule of FEATURE_TEXT 5.11 (`refit_placement`, `Keep::Anchor`) keeps the on-model size of one em and the
    /// on-model position of the anchor. If the picture would get too large at the new size (8192 px on a side, or
    /// 16.8 megapixels for a styled text), the next smaller power of two above the current size is tried. The tries
    /// are frames (geometry, no pixels); the picture of the size taken is drawn by the bake that follows. Image
    /// stamps: no-op.
    fn retune_text(&mut self, id: StampId) {
        let Some(StampSource::Text(spec)) = self.stamp(id).map(|s| s.source.clone()) else { return };
        // `text_raster_px` reads the height of the full picture from the frame cache.
        let old_frame = match self.stamp_text_frame(id) {
            Ok(frame) => frame,
            Err(e) => {
                log::warn!("stamp {}: the text cannot be laid out: {e}", id.0);
                return;
            }
        };
        let target = self.text_raster_px(id);
        let s0 = raster_px(&spec);
        let mut candidate = target;
        let (new_spec, new_frame) = loop {
            if candidate == spec.size_px {
                return;
            }
            let mut new_spec = spec.clone();
            new_spec.size_px = candidate;
            match self.frame_of_spec(&new_spec) {
                Ok(frame) => break (new_spec, frame),
                Err(e) => {
                    let smaller = candidate / 2.0;
                    if candidate > s0 && smaller > s0 && smaller >= MIN_TEXT_PX {
                        candidate = smaller;
                    } else {
                        // The current size is the largest that fits (e.g. after the too-large fallback): kept.
                        log::debug!("stamp {}: text stays at {s0} px, {candidate} px does not fit: {e}", id.0);
                        return;
                    }
                }
            }
        };
        let Some(old_placement) = self.stamp(id).map(|s| s.placement) else { return };
        let mut placement = old_placement;
        refit_placement(&mut placement, &old_frame, &new_frame, Keep::Anchor);
        // The raster rounding changes the footprint slightly: an automatic depth range is re-fitted so it stays the
        // automatic one (and keeps following later moves, 6.5).
        let placement = self.refit_auto_depth(&old_placement, placement);
        if let Some(stamp) = self.stamps.iter_mut().find(|s| s.id == id) {
            stamp.placement = placement;
            stamp.source = StampSource::Text(new_spec.clone());
        }
        // The old picture no longer belongs to the spec (`text_source` would not use it); the next bake draws the
        // new one once.
        self.text_mips.remove(&id);
        self.text_caches.preview_ids.remove(&id);
        self.insert_text_frame(id, new_spec, new_frame);
    }

    /// Bakes stamp `id` at `quality` and applies the result (linked-layer maintenance of 6.5).
    ///
    /// Round 5, rule 1 (one code path): a `Full` bake makes the stamp's bake job, runs it on the calling thread and
    /// applies its output without the staleness check (`apply_bake_output`): the same code the worker of a deferred
    /// Final bake runs. The `Preview` bake keeps its direct code.
    fn bake_and_apply(&mut self, id: StampId, quality: BakeQuality) -> Result<(), EditError> {
        if quality == BakeQuality::Full {
            let job = self.make_bake_job(id)?;
            return self.apply_bake_output(job.run());
        }
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?;
        let (source, placement, tint) = (stamp.source.clone(), stamp.placement, stamp.tint);
        let mips = self.source_mips(id, &source, raster_quality(quality))?;
        let baked = match placement {
            Placement::Projected(p) if p.mode == SurfaceMode::Wrap => {
                // Round 6 (4.6): the cached parameterization when it serves, else a fresh one (cached).
                let maps = self.texel_maps();
                let cached = self.bakes.wrap_params.get(id);
                let graph = self.wrap_mesh_cell();
                let (baked, used) = wrap_bake(id, &self.mesh, &self.bvh, &graph, &maps, &mips, tint, &p, cached, quality);
                if let Some(used) = used {
                    self.store_wrap_param(id, used.param, used.fresh);
                }
                baked
            }
            Placement::Projected(p) => {
                let maps = self.texel_maps();
                bake_projected(&self.mesh, &self.bvh, &maps, &mips, tint, &p, quality)
            }
            Placement::Uv(p) => {
                let set = self.set(p.set).ok_or(EditError::UnknownSet)?;
                let buf = bake_uv(&mips, tint, &p, &set.texel_map);
                if buf.is_empty() { Vec::new() } else { vec![(p.set, buf)] }
            }
        };
        self.apply_bake(id, baked);
        Ok(())
    }

    /// Linked-layer maintenance (6.5): a covered set without a linked layer gets one (above its active layer); a
    /// linked layer of an uncovered set gets an empty cache; the home set always has a linked layer. Dirty rect per set
    /// = union of the old and the new cache rect.
    fn apply_bake(&mut self, id: StampId, baked: Vec<(TextureSetId, SparseBuf)>) {
        let Some(stamp) = self.stamp(id) else { return };
        let (home, name, text) = (stamp.home_set, stamp.name.clone(), matches!(stamp.source, StampSource::Text(_)));
        let mut by_set: Vec<Option<SparseBuf>> = (0..self.sets.len()).map(|_| None).collect();
        for (set, buf) in baked {
            if let Some(slot) = by_set.get_mut(set.index())
                && !buf.is_empty()
            {
                *slot = Some(buf);
            }
        }
        for (s, new) in by_set.into_iter().enumerate() {
            let set = TextureSetId(s as u32);
            let linked = self.sets[s].layers.iter().position(|l| l.stamp_id() == Some(id));
            match linked {
                Some(index) => {
                    let layer = &mut self.sets[s].layers[index];
                    let layer_id = layer.id;
                    let Some(cache) = cache_mut(layer) else { continue };
                    let new = new.unwrap_or_else(SparseBuf::empty);
                    if **cache == new {
                        continue;
                    }
                    // The parts of the old and of the new cache: on a packed atlas they are small pieces spread
                    // over the texture, their bounding box is most of it.
                    let dirty: Vec<IRect> = cache.rects().chain(new.rects()).collect();
                    *cache = Arc::new(new);
                    self.mark_layer_rects_dirty(layer_id, &dirty);
                }
                None => {
                    if new.is_some() || set == home {
                        self.insert_linked_layer(id, set, &name, text, new.unwrap_or_else(SparseBuf::empty));
                    }
                }
            }
        }
    }

    /// A new linked layer (opacity 1, Normal, visible) directly above the set's active layer (top if none).
    fn insert_linked_layer(&mut self, stamp: StampId, set: TextureSetId, name: &str, text: bool, cache: SparseBuf) {
        let id = LayerId(self.alloc_id());
        let cache = Arc::new(cache);
        let kind = if text { LayerKind::Text { stamp, cache } } else { LayerKind::Stamp { stamp, cache } };
        let mut layer = Layer::new(id, name, kind);
        layer.version = self.fresh_version();
        let target = &mut self.sets[set.index()];
        let index = target.active.and_then(|a| target.layer_index(a)).map_or(target.layers.len(), |i| i + 1);
        target.layers.insert(index, layer);
        target.mark_layer_content_dirty(index);
    }

    /// Removes the linked layers of `id` whose cache is empty, except the one in its home set.
    fn remove_empty_linked(&mut self, id: StampId) {
        let Some(home) = self.stamp(id).map(|s| s.home_set) else { return };
        for (s, set) in self.sets.iter_mut().enumerate() {
            if s == home.index() {
                continue;
            }
            let empty = set.layers.iter().position(|l| l.stamp_id() == Some(id) && l.content_rect().is_empty());
            if let Some(index) = empty {
                remove_layer_at(set, index);
            }
        }
    }
}

/// The text part of a new stamp in `add_stamp`: its full picture, or (deferred Final bakes) only its frame.
enum NewText {
    Picture(TextRaster),
    Frame(TextFrame),
}

// ----- round 5: deferred Final bakes (ARCHITECTURE.md 5.20 "Deferred Final bakes", rules 1-10) -----

impl Document {
    /// Switches deferred Final bakes on or off (default off; the app turns it on). While on, ending a stamp edit
    /// (`Commit::Final`, a flush, `add_stamp`) does the cheap, history-visible part at once and marks the stamp unbaked
    /// instead of baking it; the caller takes the bakes with `next_final_bake` and hands the outputs back with
    /// `apply_final_bake`. Turning it off bakes every waiting stamp here (`settle_final_bakes`).
    pub fn set_defer_final_bakes(&mut self, on: bool) {
        if !on {
            self.settle_final_bakes();
        }
        self.defer_final_bakes = on;
        if !on {
            // Stamps of an open pending edit are finalized by its flush (mode off: baked there); none stays unbaked.
            self.bakes.unbaked.clear();
        }
    }

    /// True when a stamp waits for its Full bake and is not being edited (an id in `unbaked` that is not in
    /// `preview_baked`).
    pub fn has_unbaked_stamps(&self) -> bool {
        self.bakes.unbaked.iter().any(|id| !self.preview_baked.contains(id))
    }

    /// True when stamp `id` waits for its Full bake (its caches show an earlier state).
    pub fn stamp_is_unbaked(&self, id: StampId) -> bool {
        self.bakes.unbaked.contains(&id)
    }

    /// The job of the lowest unbaked id that is not in `preview_baked`; `None` when there is none or a stroke is open.
    /// A stamp whose inputs cannot be prepared (a missing image, a font that does not parse) is logged, taken out of
    /// `unbaked` and skipped: its caches stay as they are.
    pub fn next_final_bake(&mut self) -> Option<StampBakeJob> {
        if self.stroke.is_some() {
            return None;
        }
        loop {
            let id = self.bakes.unbaked.iter().copied().find(|id| !self.preview_baked.contains(id))?;
            match self.make_bake_job(id) {
                Ok(job) => return Some(job),
                Err(e) => {
                    log::warn!("stamp {}: the final bake cannot start, the earlier picture stays: {e}", id.0);
                    self.bakes.unbaked.remove(&id);
                }
            }
        }
    }

    /// Applies the output of a job made by `next_final_bake` (rule 6). `Busy(out)` while a stroke is open (nothing
    /// changes). The output is used only when the stamp exists and equals the job's object, is still unbaked and not
    /// being edited, and the mesh and every texel map are the job's `Arc`s; otherwise `Stale` and nothing changes.
    /// When used: the drawn text picture is stored, the caches are replaced (linked-layer maintenance of 6.5), empty
    /// linked layers outside the home set are removed, the stamp is no longer unbaked and the revision moves
    /// (`history_state()` and `is_modified()` do not change). An output that carries an error: warning in the log, the
    /// stamp is no longer unbaked and keeps its caches (`Failed`). A locked layer does not stop it (not a user edit).
    pub fn apply_final_bake(&mut self, out: StampBakeOutput) -> FinalBake {
        if self.stroke.is_some() {
            return FinalBake::Busy(Box::new(out));
        }
        let id = out.id;
        let same_maps = out.maps.len() == self.sets.len()
            && out.maps.iter().zip(&self.sets).all(|(m, set)| Arc::ptr_eq(m, &set.texel_map));
        let fresh = self.stamp(id).is_some_and(|s| *s == out.object)
            && self.bakes.unbaked.contains(&id)
            && !self.preview_baked.contains(&id)
            && Arc::ptr_eq(&out.mesh, &self.mesh)
            && same_maps;
        if !fresh {
            return FinalBake::Stale;
        }
        self.bakes.unbaked.remove(&id);
        let result = self.apply_bake_output(out);
        let outcome = match result {
            Ok(()) => {
                self.remove_empty_linked(id);
                self.touch();
                FinalBake::Applied { id, empty: self.stamp_bake_is_empty(id) }
            }
            Err(e) => {
                log::warn!("stamp {}: the final bake failed, the earlier picture stays: {e}", id.0);
                FinalBake::Failed { id }
            }
        };
        self.debug_check_structure();
        outcome
    }

    /// Bakes every unbaked stamp that is not being edited on the calling thread (`next_final_bake` + `run` +
    /// `apply_final_bake` until none is left). Before save, export, merge down and rasterize (the app's settle).
    pub fn settle_final_bakes(&mut self) {
        while let Some(job) = self.next_final_bake() {
            let id = job.stamp();
            match self.apply_final_bake(job.run()) {
                FinalBake::Applied { .. } | FinalBake::Failed { .. } => {}
                other => {
                    // Cannot happen (nothing changes between making and applying the job); never loop on it.
                    log::error!("stamp {}: a synchronous final bake was not applied ({other:?})", id.0);
                    self.bakes.unbaked.remove(&id);
                }
            }
        }
    }

    /// Rule 8: a command that keeps the pixels of stamp `id` (merge down, rasterize) bakes it here first when it is
    /// unbaked. A failed bake keeps the earlier caches (as `apply_final_bake`).
    pub(crate) fn bake_unbaked_now(&mut self, id: StampId) {
        if !self.bakes.unbaked.contains(&id) || self.preview_baked.contains(&id) {
            return;
        }
        let Some(job) = self.next_job_of(id) else { return };
        if let FinalBake::Stale | FinalBake::Busy(_) = self.apply_final_bake(job.run()) {
            log::error!("stamp {}: the bake before a merge or rasterize was not applied", id.0);
            self.bakes.unbaked.remove(&id);
        }
    }

    /// The job of stamp `id` (`make_bake_job`); a stamp whose inputs cannot be prepared leaves `unbaked` (logged).
    fn next_job_of(&mut self, id: StampId) -> Option<StampBakeJob> {
        match self.make_bake_job(id) {
            Ok(job) => Some(job),
            Err(e) => {
                log::warn!("stamp {}: the final bake cannot start, the earlier picture stays: {e}", id.0);
                self.bakes.unbaked.remove(&id);
                None
            }
        }
    }

    /// The Full bake job of stamp `id` (rule 2; UI thread, cheap): an image takes its decoded mips (decoded here on
    /// a cache miss); a text takes its cached picture when that is the full picture of its spec (`text_source`'s
    /// test), else its spec and fonts, and the job draws it. Errors as `bake_and_apply`: `UnknownStamp`,
    /// `UnknownBlob` / `Image`, `Text` (a font that does not parse), `UnknownSet` (a UV placement in a missing set).
    pub(crate) fn make_bake_job(&mut self, id: StampId) -> Result<StampBakeJob, EditError> {
        let object = self.stamp(id).ok_or(EditError::UnknownStamp)?.clone();
        let picture = match &object.source {
            StampSource::Image { blob } => JobPicture::Image(self.image_source(*blob)?),
            StampSource::Text(spec) => match self.text_mips.get(&id) {
                Some((cached, mips)) if cached == spec && !self.text_caches.preview_ids.contains(&id) => {
                    JobPicture::TextCached(Arc::clone(mips))
                }
                _ => {
                    let (own, fallback) = self.text_fonts_owned(spec)?;
                    JobPicture::TextDraw { spec: Box::new(spec.clone()), own, fallback }
                }
            },
        };
        if let Placement::Uv(p) = &object.placement {
            self.set(p.set).ok_or(EditError::UnknownSet)?;
        }
        // Round 6: a wrapped stamp's cached parameterization (`run` checks that it serves).
        let wrap = match object.placement {
            Placement::Projected(p) if p.mode == SurfaceMode::Wrap => self.bakes.wrap_params.get(id),
            _ => None,
        };
        Ok(StampBakeJob {
            id,
            object,
            mesh: Arc::clone(&self.mesh),
            bvh: Arc::clone(&self.bvh),
            maps: self.texel_maps(),
            picture,
            wrap,
            kind: StampJobKind::Full,
            serial: self.bakes.edit_serial,
            graph: self.wrap_mesh_cell(),
        })
    }

    /// Applies a job's output without the staleness check: the drawn text picture (`store_text_mips`), then the caches
    /// (`apply_bake`). An output that carries an error changes nothing and returns it.
    fn apply_bake_output(&mut self, out: StampBakeOutput) -> Result<(), EditError> {
        let StampBakeOutput { id, result, picture, wrap, .. } = out;
        let baked = result?;
        if let Some(drawn) = picture {
            self.store_text_mips(id, drawn.spec, drawn.frame, drawn.full, drawn.mips);
        }
        if let Some(used) = wrap {
            self.store_wrap_param(id, used.param, used.fresh);
        }
        self.apply_bake(id, baked);
        Ok(())
    }

    /// True when no linked layer of stamp `id` holds a texel.
    fn stamp_bake_is_empty(&self, id: StampId) -> bool {
        self.sets.iter().flat_map(|s| &s.layers).filter(|l| l.stamp_id() == Some(id)).all(|l| match &l.kind {
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => cache.is_empty(),
            LayerKind::Raster(_) => true,
        })
    }
}

// ----- round 6: wrapped stamps (ARCHITECTURE.md 5.20, plan 4.1 / 4.4) -----

impl Document {
    /// Caches `param` for stamp `id` (entries of removed stamps are dropped); `fresh` counts a new build.
    pub(crate) fn store_wrap_param(&mut self, id: StampId, param: Arc<WrapParam>, fresh: bool) {
        if fresh {
            self.bakes.wrap_params.fresh += 1;
        }
        let stamps = &self.stamps;
        self.bakes.wrap_params.insert(id, param, |s| stamps.iter().any(|o| o.id == s));
    }

    /// Round 9 (package B, T7): the welded graph cell of the document's mesh, shared with the jobs (made on first use;
    /// a new cell when the mesh is another `Arc`). Nothing is built here.
    pub fn wrap_mesh_cell(&mut self) -> Arc<WrapMeshCell> {
        match &self.bakes.wrap_mesh {
            Some(cell) if cell.is_for(&self.mesh) => Arc::clone(cell),
            _ => {
                let cell = WrapMeshCell::new(&self.mesh);
                self.bakes.wrap_mesh = Some(Arc::clone(&cell));
                cell
            }
        }
    }

    /// Round 9 (package B, T8 / T9): the warm-up of the mesh's derived data when it is not built yet: the shared texels
    /// (any 3D stamp) and, with `graph` (a wrapped stamp), the welded graph. Run it off the UI thread so that
    /// the first wrapped Preview does not pay the build. `None` when there is nothing to build.
    pub fn wrap_mesh_warmup(&mut self, graph: bool) -> Option<WrapMeshWarmup> {
        let cell = self.wrap_mesh_cell();
        let todo = cell.uv_built().is_none() || (graph && !cell.is_built());
        todo.then(|| WrapMeshWarmup { cell, mesh: Arc::clone(&self.mesh), graph })
    }

    /// Round 9 (package B, T9): the share of 3D stamp `id`'s picture that lies on triangles sharing their texels with
    /// other triangles (mirrored or repeated parts: the paint shows there too), sampled on a 16 x 16 grid of picture
    /// points: where a wrapped stamp's parameterization puts them, where a projected stamp's rays hit. `None` while the
    /// shared texels are not found yet (the warm-up job), for a wrapped stamp without a parameterization of its
    /// placement, for a UV stamp and where no sample meets the surface. Cheap: no build on the calling thread.
    pub fn stamp_shared_texels(&self, id: StampId) -> Option<f32> {
        let Placement::Projected(p) = self.stamp(id)?.placement else { return None };
        let cell = self.bakes.wrap_mesh.as_ref().filter(|c| c.is_for(&self.mesh))?;
        let uv = cell.uv_built()?;
        const N: usize = 16;
        let (hw, hh) = (p.half_size.x.abs(), p.half_size.y.abs());
        let local = |i: usize, j: usize| Vec2::new(hw * ((i as f32 + 0.5) / N as f32 * 2.0 - 1.0), hh * ((j as f32 + 0.5) / N as f32 * 2.0 - 1.0));
        let (mut hit, mut shared) = (0usize, 0usize);
        if p.mode == SurfaceMode::Wrap {
            let w = self.serving_wrap_param(id)?;
            for j in 0..N {
                for i in 0..N {
                    if let Some(sp) = w.surface_of(&self.mesh, local(i, j)) {
                        hit += 1;
                        shared += usize::from(uv.shares(sp.tri));
                    }
                }
            }
        } else {
            let (right, up, out) = p.axes();
            let depth = self.mesh.diag();
            for j in 0..N {
                for i in 0..N {
                    let l = local(i, j);
                    let o = p.origin + right * l.x + up * l.y + out * depth;
                    if let Some(h) = self.raycast(&crate::math::Ray::new(o, -out)) {
                        hit += 1;
                        shared += usize::from(uv.shares(h.tri));
                    }
                }
            }
        }
        (hit > 0).then(|| shared as f32 / hit as f32)
    }

    /// The placement of stamp `id` when it is a wrapped 3D stamp.
    fn wrapped_projector(&self, id: StampId) -> Option<Projector> {
        match self.stamp(id)?.placement {
            Placement::Projected(p) if p.mode == SurfaceMode::Wrap => Some(p),
            _ => None,
        }
    }

    /// The cached parameterization of stamp `id` when it serves the stamp's current placement.
    fn serving_wrap_param(&self, id: StampId) -> Option<&Arc<WrapParam>> {
        let p = self.wrapped_projector(id)?;
        self.bakes.wrap_params.peek(id).filter(|w| w.serves(&self.mesh, &p))
    }

    /// The parameterization of wrapped stamp `id`, computed on the calling thread when the cache has none that serves
    /// its placement (after opening a project the caches are loaded, the parameterizations are not). `None` for a
    /// stamp that is not wrapped, an unknown id, a source that cannot be prepared, or a placement without surface.
    /// Not a command: no history, no pixels change.
    pub fn ensure_wrap_param(&mut self, id: StampId) -> Option<WrapStats> {
        let p = self.wrapped_projector(id)?;
        let source = self.stamp(id)?.source.clone();
        let mips = self.source_mips(id, &source, raster_quality(BakeQuality::Preview)).ok()?;
        let maps = self.texel_maps();
        if let Some(w) = self.bakes.wrap_params.get(id)
            && w.serves_inputs(&self.mesh, &maps, mips.size(), &p)
        {
            return Some(w.stats_for_src(&p, mips.size()));
        }
        let graph = self.wrap_mesh_cell();
        let param = Arc::new(wrap_param_in(&self.mesh, &self.bvh, graph.get(&self.mesh), &maps, mips.size(), &p)?);
        let s = param.stats_for_src(&p, mips.size());
        log::info!(
            "wrap: stamp {} param {} verts {} tris in {:.1} ms (fresh), covered {:.1} %, cut {} (ensure)",
            id.0,
            s.verts,
            s.tris,
            s.micros as f64 / 1e3,
            s.covered * 100.0,
            s.cut_tris
        );
        self.store_wrap_param(id, param, true);
        Some(s)
    }

    /// The statistics of wrapped stamp `id` from the cache (`None` when the cache holds no parameterization of its
    /// current placement: see `ensure_wrap_param`); `covered` is measured for the stamp's current size
    /// (`WrapParam::stats_for`).
    pub fn stamp_wrap_stats(&self, id: StampId) -> Option<WrapStats> {
        let p = self.wrapped_projector(id)?;
        self.serving_wrap_param(id).map(|w| w.stats_for(&p))
    }

    /// The border of wrapped stamp `id`'s picture on the surface (`WrapParam::outline`, top turn), from the cache;
    /// empty when the cache has no parameterization of its placement.
    pub fn stamp_wrap_outline(&self, id: StampId, samples_per_edge: usize) -> Vec<Vec<SurfacePoint>> {
        let (Some(w), Some(p)) = (self.serving_wrap_param(id), self.wrapped_projector(id)) else { return Vec::new() };
        w.outline(&self.mesh, p.half_size, samples_per_edge)
    }

    /// Surface -> stamp-local coordinates of wrapped stamp `id` (`WrapParam::local_of`), from the cache.
    pub fn stamp_wrap_local(&self, id: StampId, tri: crate::ids::TriId, bary: [f32; 3]) -> Option<Vec2> {
        self.serving_wrap_param(id)?.local_of(tri, bary)
    }

    /// Stamp-local coordinates -> surface point of wrapped stamp `id` (`WrapParam::surface_of`), from the cache.
    pub fn stamp_wrap_surface(&self, id: StampId, local: Vec2) -> Option<SurfacePoint> {
        self.serving_wrap_param(id)?.surface_of(&self.mesh, local)
    }

    /// The picture's area on the model: the sum over the stamp's linked caches of `alpha / 255` times the world area
    /// of each texel (its owning triangle's world area over its UV area in texels). Gutter texels are not on the
    /// surface and do not count. For tests and the scenario: compare with the picture's opaque area. Any stamp kind;
    /// 0 for an unknown id.
    pub fn stamp_ink_area(&self, id: StampId) -> f32 {
        let mut total = 0.0f64;
        for (set, layer) in self.stamp_layers(id) {
            let (Some(set), Some(layer)) = (self.set(set), self.layer(layer)) else { continue };
            let cache = match &layer.kind {
                crate::layer::LayerKind::Stamp { cache, .. } | crate::layer::LayerKind::Text { cache, .. } => cache,
                crate::layer::LayerKind::Raster(_) => continue,
            };
            let map = &set.texel_map;
            let size = Vec2::new(map.width as f32, map.height as f32);
            let mut per_tri: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
            for part in cache.parts() {
                let r = part.rect;
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        let a = part.get(x, y)[3];
                        if a == 0 {
                            continue;
                        }
                        let Some((tri, false)) = map.owner(x as u32, y as u32) else { continue };
                        let texel_area = *per_tri.entry(tri).or_insert_with(|| {
                            let p = self.mesh.tri_positions(tri);
                            let uv = self.mesh.tri_uvs(tri).map(|q| q * size);
                            let world = 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length() as f64;
                            let px = 0.5 * (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]).abs() as f64;
                            if px > 0.0 && world.is_finite() { world / px } else { 0.0 }
                        });
                        total += a as f64 / 255.0 * texel_area;
                    }
                }
            }
        }
        total as f32
    }

    /// Round 8 (plan 3.3): how once and how whole the picture of wrapped stamp `id` lies, from its baked texels and the
    /// cached parameterization of its placement (`None` without one: see `ensure_wrap_param`; `None` for a stamp that
    /// is not wrapped). Over the inked texels (alpha >= 128, gutter texels excluded) the surface point (`locate`) and
    /// its picture point (`local_of`, top turn) are taken; picture points are binned in cells of
    /// `c = 2 x max(source pixel in world units, texel_world)`. `dup`: share of the inked bins whose surface points
    /// spread over more than `6 c` (the diagonal of their bounding box); `scatter`: share of the 4-neighbour pairs of
    /// inked bins whose surface centroids lie more than `6 c` apart. For tests, the scenario and the gate.
    pub fn stamp_wrap_quality(&self, id: StampId) -> Option<crate::wrap_param::WrapQuality> {
        use glam::DVec3;
        let p = self.wrapped_projector(id)?;
        let w = self.serving_wrap_param(id)?;
        let (hw, hh) = (p.half_size.x.abs() as f64, p.half_size.y.abs() as f64);
        let [sw, sh] = w.src_size();
        let src_px = (2.0 * hw / sw.max(1) as f64).max(2.0 * hh / sh.max(1) as f64);
        let c = 2.0 * src_px.max(w.texel_world() as f64);
        if !(c > 0.0 && c.is_finite()) {
            return None;
        }
        // Per bin: bounding box, sum and count of the surface points.
        let mut bins: std::collections::BTreeMap<(i64, i64), (DVec3, DVec3, DVec3, u32)> = std::collections::BTreeMap::new();
        for (set, layer) in self.stamp_layers(id) {
            let (Some(set), Some(layer)) = (self.set(set), self.layer(layer)) else { continue };
            let cache = match &layer.kind {
                crate::layer::LayerKind::Stamp { cache, .. } | crate::layer::LayerKind::Text { cache, .. } => cache,
                crate::layer::LayerKind::Raster(_) => continue,
            };
            let map = &set.texel_map;
            for part in cache.parts() {
                let r = part.rect;
                for y in r.y0..r.y1 {
                    for x in r.x0..r.x1 {
                        if part.get(x, y)[3] < 128 {
                            continue;
                        }
                        let Some((_, false)) = map.owner(x as u32, y as u32) else { continue };
                        let Some(sp) = map.locate(&self.mesh, x as u32, y as u32) else { continue };
                        let Some(l) = w.local_of(sp.tri, sp.bary) else { continue };
                        let key = ((l.x as f64 / c).floor() as i64, (l.y as f64 / c).floor() as i64);
                        let q = DVec3::new(sp.position.x as f64, sp.position.y as f64, sp.position.z as f64);
                        let e = bins.entry(key).or_insert((q, q, DVec3::ZERO, 0));
                        e.0 = e.0.min(q);
                        e.1 = e.1.max(q);
                        e.2 += q;
                        e.3 += 1;
                    }
                }
            }
        }
        let far = 6.0 * c;
        let dup = bins.values().filter(|b| b.0.distance(b.1) > far).count();
        let (mut pairs, mut apart) = (0usize, 0usize);
        for (&(x, y), b) in &bins {
            let centroid = b.2 / b.3 as f64;
            for key in [(x + 1, y), (x, y + 1)] {
                if let Some(o) = bins.get(&key) {
                    pairs += 1;
                    if centroid.distance(o.2 / o.3 as f64) > far {
                        apart += 1;
                    }
                }
            }
        }
        Some(crate::wrap_param::WrapQuality {
            ink_area: self.stamp_ink_area(id),
            dup: dup as f32 / bins.len().max(1) as f32,
            scatter: apart as f32 / pairs.max(1) as f32,
            bins: bins.len() as u32,
        })
    }

    /// True when stamp `id` is wrapped and its Preview bakes go to jobs (`set_defer_wrap_previews`).
    fn wrap_preview_deferred(&self, id: StampId) -> bool {
        self.bakes.defer_wrap_previews && self.wrapped_projector(id).is_some()
    }

    /// Round 8 (3.4): while on, a Preview edit of a wrapped stamp bakes nothing on the calling thread: the stamp waits in
    /// the deferred set for its Preview job (`next_wrap_preview_job`, run anywhere, `apply_wrap_preview`); projected and
    /// UV stamps are unchanged (their Previews stay on the calling thread). Default off. Turning it off bakes the
    /// waiting stamps here (unless `set_defer_preview_bakes` holds them).
    pub fn set_defer_wrap_previews(&mut self, on: bool) {
        self.bakes.defer_wrap_previews = on;
        if !on && !self.defer_preview_bakes {
            self.bake_stale_previews();
        }
    }

    /// True while wrapped stamps' Preview bakes go to jobs.
    pub fn defers_wrap_previews(&self) -> bool {
        self.bakes.defer_wrap_previews
    }

    /// True when a wrapped stamp waits for its Preview job.
    pub fn has_stale_wrap_previews(&self) -> bool {
        self.bakes.preview_stale.iter().any(|&id| self.wrap_preview_deferred(id))
    }

    /// Round 8 (3.4): the Preview job of the lowest waiting wrapped stamp (taken out of the deferred set), made for the
    /// stamp as it is now and the open pending edit's serial. The picture is the one the synchronous Preview bake uses
    /// (`source_mips` at Preview quality: a text's draft picture is drawn here, as before), so the job's bytes equal the
    /// synchronous Preview's. A stamp whose inputs cannot be prepared is logged and skipped.
    pub fn next_wrap_preview_job(&mut self) -> Option<StampBakeJob> {
        loop {
            let id = self.bakes.preview_stale.iter().copied().find(|&id| self.wrapped_projector(id).is_some())?;
            self.bakes.preview_stale.remove(&id);
            match self.make_wrap_job(id, StampJobKind::Preview) {
                Ok(job) => return Some(job),
                Err(e) => log::warn!("stamp {}: the preview bake cannot start: {e}", id.0),
            }
        }
    }

    /// A Preview or ParamOnly job of wrapped stamp `id` (the Preview picture, the cached parameterization).
    fn make_wrap_job(&mut self, id: StampId, kind: StampJobKind) -> Result<StampBakeJob, EditError> {
        let object = self.stamp(id).ok_or(EditError::UnknownStamp)?.clone();
        let mips = self.source_mips(id, &object.source, raster_quality(BakeQuality::Preview))?;
        let picture = match object.source {
            StampSource::Image { .. } => JobPicture::Image(mips),
            StampSource::Text(_) => JobPicture::TextCached(mips),
        };
        Ok(StampBakeJob {
            id,
            wrap: self.bakes.wrap_params.get(id),
            object,
            mesh: Arc::clone(&self.mesh),
            bvh: Arc::clone(&self.bvh),
            maps: self.texel_maps(),
            picture,
            kind,
            serial: self.bakes.edit_serial,
            graph: self.wrap_mesh_cell(),
        })
    }

    /// True when `out` was made for this document's mesh and texel maps.
    fn same_inputs(&self, out: &StampBakeOutput) -> bool {
        Arc::ptr_eq(&out.mesh, &self.mesh)
            && out.maps.len() == self.sets.len()
            && out.maps.iter().zip(&self.sets).all(|(m, set)| Arc::ptr_eq(m, &set.texel_map))
    }

    /// Round 8 (3.4): applies the output of a wrap Preview job. Applied when the stamp exists, is still in the same
    /// open pending edit (`preview_baked`, the job's serial) and the mesh and maps are the job's — also when the stamp
    /// changed since (the last good picture): then it waits again for its next job (`due_again`). Its parameterization
    /// is cached when it serves the current placement. Esc, undo, redo, a Final (a flush), a removed stamp or another
    /// document's output: `Stale`, nothing changes. A stroke in progress: `Stale` and the stamp waits again.
    pub fn apply_wrap_preview(&mut self, out: StampBakeOutput) -> WrapPreview {
        let id = out.id;
        // check:B: a stamp that is no longer wrapped (turned to Project in the same pending edit) never takes a wrapped
        // picture: its own Preview comes from the calling thread.
        let live = out.kind == StampJobKind::Preview
            && self.wrapped_projector(id).is_some()
            && self.preview_baked.contains(&id)
            && out.serial == self.bakes.edit_serial
            && self.same_inputs(&out);
        if !live {
            // Round 8 (request 1 of package B, applied by check:B): a Preview job that lands after the release (a
            // Final / flush) was often made for the final placement: its parameterization is cached when it serves the
            // stamp now (no pixels change), so the Final job that waited for it reuses it instead of building it again
            // (plan 2.5-3). A parameterization that serves is the one a fresh build would give (WA27), so storing it
            // after Esc / undo is harmless too.
            if out.kind == StampJobKind::Preview
                && self.same_inputs(&out)
                && let Some(used) = out.wrap
                && self.wrapped_projector(id).is_some_and(|p| used.param.serves(&self.mesh, &p))
            {
                self.store_wrap_param(id, used.param, used.fresh);
            }
            return WrapPreview::Stale;
        }
        if self.stroke.is_some() {
            self.bakes.preview_stale.insert(id);
            return WrapPreview::Stale;
        }
        let due_again = self.stamp(id).is_some_and(|s| *s != out.object);
        let StampBakeOutput { result, wrap, .. } = out;
        match result {
            Ok(baked) => self.apply_bake(id, baked),
            Err(e) => log::warn!("stamp {}: the preview bake failed: {e}", id.0),
        }
        if let Some(used) = wrap
            && self.wrapped_projector(id).is_some_and(|p| used.param.serves(&self.mesh, &p))
        {
            self.store_wrap_param(id, used.param, used.fresh);
        }
        if due_again {
            self.bakes.preview_stale.insert(id);
        }
        WrapPreview::Applied { due_again }
    }

    /// Round 8 (3.4): the ParamOnly job of wrapped stamp `id` when the cache holds no parameterization of its current
    /// placement (after an open, an undo): the off-thread form of `ensure_wrap_param`. `None` when there is nothing to do.
    pub fn wrap_param_job(&mut self, id: StampId) -> Option<StampBakeJob> {
        self.wrapped_projector(id)?;
        if self.serving_wrap_param(id).is_some() {
            return None;
        }
        self.make_wrap_job(id, StampJobKind::ParamOnly).ok()
    }

    /// Round 8 (3.4): caches the parameterization of a ParamOnly job's output when it serves stamp's current placement
    /// on this document's mesh and maps (true); no pixel changes.
    pub fn apply_wrap_param(&mut self, out: StampBakeOutput) -> bool {
        if out.kind != StampJobKind::ParamOnly || !self.same_inputs(&out) {
            return false;
        }
        let id = out.id;
        let Some(p) = self.wrapped_projector(id) else { return false };
        match out.wrap {
            Some(used) if used.param.serves(&self.mesh, &p) => {
                self.store_wrap_param(id, used.param, used.fresh);
                true
            }
            _ => false,
        }
    }

    /// Test access: the cached parameterization of stamp `id` and the number of fresh builds so far.
    #[cfg(test)]
    pub(crate) fn wrap_cache_probe(&self, id: StampId) -> (Option<Arc<WrapParam>>, u64) {
        (self.bakes.wrap_params.peek(id).cloned(), self.bakes.wrap_params.fresh)
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::*;
    use crate::history::testkit::doc_state;
    use crate::image_io::{PngLevel, encode_png};
    use crate::math::IRect;
    use crate::samples;
    use crate::stamp::{Projector, UvPlacement};
    use crate::testutil::doc_from_raw;
    use crate::text::{Outline, TextAlign};

    fn png_bytes(image: &PixelBuf) -> Arc<[u8]> {
        Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
    }

    fn solid_png(rgba: [u8; 4]) -> Arc<[u8]> {
        png_bytes(&PixelBuf::filled(16, 16, rgba))
    }

    /// Projector facing -Z onto the plane z = `z`, centred at (cx, cy), image up = +Y.
    fn front(cx: f32, cy: f32, z: f32, half: f32, depth: f32) -> Projector {
        Projector::from_surface(Vec3::new(cx, cy, z), Vec3::Z, Vec3::Y, Vec2::splat(half), depth)
    }

    fn uv(set: u32, center: [f32; 2], size: [f32; 2]) -> UvPlacement {
        UvPlacement {
            set: TextureSetId(set),
            center_px: Vec2::from(center),
            size_px: Vec2::from(size),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        }
    }

    fn cache_of(doc: &Document, layer: LayerId) -> Arc<SparseBuf> {
        match &doc.layer(layer).unwrap().kind {
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => Arc::clone(cache),
            LayerKind::Raster(_) => panic!("not a stamp layer"),
        }
    }

    fn labels(doc: &Document) -> Vec<String> {
        doc.history.undo.iter().map(|e| e.label.clone()).collect()
    }

    fn text_spec(doc: &mut Document, content: &str) -> TextSpec {
        TextSpec {
            content: content.to_owned(),
            font: doc.default_font(),
            size_px: TEXT_PLACEMENT_PX,
            color: [255, 255, 255, 255],
            align: TextAlign::Left,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: crate::text::TextFx::default(),
        }
    }

    fn spec_of(doc: &Document, id: StampId) -> TextSpec {
        match &doc.stamp(id).unwrap().source {
            StampSource::Text(spec) => spec.clone(),
            StampSource::Image { .. } => panic!("not a text stamp"),
        }
    }

    fn projector_of(doc: &Document, id: StampId) -> Projector {
        match doc.stamp(id).unwrap().placement {
            Placement::Projected(p) => p,
            Placement::Uv(_) => panic!("not projected"),
        }
    }

    /// Height of one em on the model in texels (6.5), from the stamp's current raster.
    fn em_texels(doc: &mut Document, id: StampId) -> f32 {
        let spec = spec_of(doc, id);
        let [_, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
        let p = projector_of(doc, id);
        2.0 * p.half_size.y * spec.size_px / h as f32 / doc.texel_world_size(doc.stamp(id).unwrap().home_set)
    }

    /// Bounding box of the pixels with alpha >= 128 of a cache.
    fn ink(cache: &SparseBuf) -> IRect {
        let mut r = IRect::EMPTY;
        let b = cache.bounds();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                if cache.get(x, y)[3] >= 128 {
                    r = r.union(&IRect::new(x, y, x + 1, y + 1));
                }
            }
        }
        r
    }

    #[test]
    fn image_blobs_decode_once_and_report_their_size() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let image = samples::sample_stamp_image(40);
        let bytes = png_bytes(&image);
        let blob = doc.add_image_blob("stamp.png", Arc::clone(&bytes)).unwrap();
        assert_eq!(doc.add_image_blob("again.png", bytes).unwrap(), blob, "identical bytes are one blob");
        assert_eq!(doc.blobs.get(blob).unwrap().name, "stamp.png");
        assert_eq!(doc.image_size(blob).unwrap(), [40, 40]);
        assert!(matches!(doc.add_image_blob("x.png", Arc::from(&b"not an image"[..])), Err(EditError::Image(_))));
        assert_eq!(doc.blobs.len(), 1, "a failed blob is not stored");

        // Thumbnails keep the aspect ratio and the colours (straight alpha).
        let wide = doc.add_image_blob("wide.png", png_bytes(&PixelBuf::filled(80, 20, [200, 40, 10, 255]))).unwrap();
        let thumb = doc.blob_thumbnail(wide, 16).unwrap();
        assert_eq!((thumb.width, thumb.height), (16, 4));
        assert_eq!(thumb.get(8, 2), [200, 40, 10, 255]);
        let half = doc.add_image_blob("half.png", png_bytes(&PixelBuf::filled(8, 8, [90, 180, 30, 128]))).unwrap();
        let p = doc.blob_thumbnail(half, 4).unwrap().get(1, 1);
        assert!(p.iter().zip([90u8, 180, 30, 128]).all(|(a, b)| a.abs_diff(b) <= 1), "{p:?}");
        assert_eq!(doc.blob_thumbnail(half, 0).unwrap().width, 0);
        let tall = doc.add_image_blob("tall.png", png_bytes(&PixelBuf::filled(21, 81, [1, 2, 3, 255]))).unwrap();
        let thumb = doc.blob_thumbnail(tall, 32).unwrap();
        assert_eq!((thumb.width, thumb.height), (8, 32));

        // Unknown blobs and fonts are not images.
        let font = doc.default_font();
        for bad in [BlobId(999), font.blob] {
            assert!(matches!(doc.image_size(bad), Err(EditError::UnknownBlob)));
            assert!(matches!(doc.blob_thumbnail(bad, 8), Err(EditError::UnknownBlob)));
        }
        // None of this is history.
        assert_eq!(doc.history.undo_len(), 0);
    }

    #[test]
    fn fonts_register_once() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let a = doc.default_font();
        let b = doc.default_font();
        assert_eq!(a, b);
        assert_eq!((a.family.as_str(), a.style.as_str(), a.face_index), (DEFAULT_FONT_FAMILY, DEFAULT_FONT_STYLE, 0));
        assert_eq!(doc.blobs.get(a.blob).unwrap().kind, BlobKind::Font);
        let hack = doc.register_font("Hack", "Regular", 0, Arc::from(epaint_default_fonts::HACK_REGULAR));
        assert_ne!(hack.blob, a.blob);
        assert_eq!((hack.family.as_str(), hack.style.as_str()), ("Hack", "Regular"));
        assert_eq!(doc.blobs.len(), 2);

        // The text source size is the size of the rasterized text.
        let spec = text_spec(&mut doc, "Size");
        let size = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
        let image = doc.render_spec(&spec, RasterQuality::Full).unwrap().image;
        assert_eq!(size, [image.width, image.height]);
        // A missing font blob falls back to the bundled font; a broken one is an error.
        let mut missing = spec.clone();
        missing.font.blob = BlobId(77);
        assert_eq!(doc.stamp_source_size(&StampSource::Text(missing)).unwrap(), size);
        let mut broken = spec;
        broken.font = doc.register_font("Broken", "Regular", 0, Arc::from(&b"garbage"[..]));
        assert!(matches!(doc.stamp_source_size(&StampSource::Text(broken)), Err(EditError::Text(_))));
    }

    #[test]
    fn add_stamp_on_two_materials_links_both_sets_in_one_entry() {
        let mut doc = doc_from_raw(samples::two_material_model(), 128);
        let initial = doc_state(&mut doc);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let p = front(0.0, 0.5, 0.5, 0.4, 0.6);
        let id = doc.add_stamp("Decal", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
        assert_eq!(labels(&doc), ["Add Stamp"]);
        let linked = doc.stamp_layers(id);
        assert_eq!(linked.iter().map(|(s, _)| s.0).collect::<Vec<_>>(), [0, 1]);
        for &(set, layer) in &linked {
            let l = doc.layer(layer).unwrap();
            assert_eq!((l.name.as_str(), l.kind_name(), l.visible, l.opacity), ("Decal", "stamp", true, 1.0));
            assert!(!cache_of(&doc, layer).is_empty());
            // Directly above the previously active "Layer 1".
            assert_eq!(doc.find_layer(layer).unwrap(), (set, 1));
        }
        // The home set's layer is active; the other set keeps its active layer.
        assert_eq!(doc.set(TextureSetId(0)).unwrap().active, Some(linked[0].1));
        assert_eq!(doc.set(TextureSetId(1)).unwrap().active, Some(LayerId(2)));
        let stamp = doc.stamp(id).unwrap();
        assert_eq!((stamp.name.as_str(), stamp.tint, stamp.home_set), ("Decal", [255; 4], TextureSetId(0)));
        let added = doc_state(&mut doc);
        assert_ne!(added, initial);
        assert!(doc.preview_baked.is_empty() && !doc.has_pending_edit());

        assert_eq!(doc.undo().as_deref(), Some("Add Stamp"));
        assert_eq!(doc_state(&mut doc), initial);
        assert_eq!(doc.redo().as_deref(), Some("Add Stamp"));
        assert_eq!(doc_state(&mut doc), added);
    }

    #[test]
    fn add_stamp_rejects_bad_input_and_changes_nothing() {
        let mut doc = doc_from_raw(samples::two_material_model(), 64);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let before = doc_state(&mut doc);
        let revision = doc.revision();
        let p = front(0.0, 0.0, 0.5, 0.3, 0.5);
        let image = StampSource::Image { blob };
        let mut nan = p;
        nan.rotation = Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0);
        let cases: Vec<(StampSource, Placement, u32)> = vec![
            (image.clone(), Placement::Projected(p), 7),
            (image.clone(), Placement::Projected(nan), 0),
            (StampSource::Image { blob: BlobId(50) }, Placement::Projected(p), 0),
            (image.clone(), Placement::Uv(uv(1, [10.0, 10.0], [8.0, 8.0])), 0),
            (image.clone(), Placement::Uv(uv(9, [10.0, 10.0], [8.0, 8.0])), 9),
        ];
        for (source, placement, home) in cases {
            assert!(doc.add_stamp("x", source, placement, TextureSetId(home)).is_err());
        }
        assert!(matches!(
            doc.add_stamp("x", image, Placement::Projected(nan), TextureSetId(0)),
            Err(EditError::InvalidParam(_))
        ));
        assert_eq!(doc_state(&mut doc), before);
        assert_eq!(doc.revision(), revision);
        assert!(doc.stamps.is_empty() && doc.history.undo_len() == 0);
    }

    #[test]
    fn a_stamp_that_misses_the_model_keeps_an_empty_home_layer() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let far = front(20.0, 20.0, 0.0, 0.2, 0.2);
        let id = doc.add_stamp("Far", StampSource::Image { blob }, Placement::Projected(far), TextureSetId(0)).unwrap();
        let linked = doc.stamp_layers(id);
        assert_eq!(linked.len(), 1);
        assert!(cache_of(&doc, linked[0].1).is_empty());
        doc_state(&mut doc);
    }

    #[test]
    fn preview_drags_and_final_make_one_entry() {
        let mut doc = doc_from_raw(samples::flat_quad(), 128);
        let blob = doc.add_image_blob("stamp.png", png_bytes(&samples::sample_stamp_image(64))).unwrap();
        let p0 = front(0.3, 0.3, 0.0, 0.2, 0.3);
        let id = doc.add_stamp("S", StampSource::Image { blob }, Placement::Projected(p0), TextureSetId(0)).unwrap();
        let layer = doc.stamp_layers(id)[0].1;
        let added = doc_state(&mut doc);
        let original_cache = cache_of(&doc, layer);

        for i in 1..=5 {
            let p = front(0.3 + 0.05 * i as f32, 0.3, 0.0, 0.2, 0.3);
            doc.set_stamp_placement(id, Placement::Projected(p), Commit::Preview).unwrap();
            assert!(doc.has_pending_edit());
            assert!(doc.preview_baked.contains(&id));
            assert_eq!(doc.history.undo_len(), 1, "previews push nothing");
            assert_eq!(doc.undo_label(), Some(EDIT_LABEL));
        }
        let last = front(0.6, 0.35, 0.0, 0.2, 0.3);
        doc.set_stamp_placement(id, Placement::Projected(last), Commit::Final).unwrap();
        assert_eq!(labels(&doc), ["Add Stamp", EDIT_LABEL]);
        assert!(!doc.has_pending_edit() && doc.preview_baked.is_empty());
        assert_eq!(doc.stamp(id).unwrap().placement, Placement::Projected(last));
        let moved = doc_state(&mut doc);

        // Undo restores the caches without re-baking: the very same Arc.
        assert_eq!(doc.undo().as_deref(), Some(EDIT_LABEL));
        assert_eq!(doc_state(&mut doc), added);
        assert!(Arc::ptr_eq(&cache_of(&doc, layer), &original_cache));
        doc.redo();
        assert_eq!(doc_state(&mut doc), moved);

        // A Final that changes nothing pushes nothing; tint edits work the same way.
        doc.set_stamp_placement(id, Placement::Projected(last), Commit::Final).unwrap();
        assert_eq!(doc.history.undo_len(), 2);
        doc.set_stamp_tint(id, [255, 0, 0, 128], Commit::Preview).unwrap();
        doc.set_stamp_tint(id, [0, 255, 0, 255], Commit::Final).unwrap();
        assert_eq!(doc.history.undo_len(), 3);
        assert_eq!(doc.stamp(id).unwrap().tint, [0, 255, 0, 255]);
        let tinted = cache_of(&doc, layer);
        assert!(tinted.parts().iter().all(|part| part.data.as_chunks::<4>().0.iter().all(|p| p[0] == 0 && p[2] == 0)));
    }

    /// Two identical documents with an occluded_quads stamp whose Full and Preview bakes differ (occlusion).
    fn occluded_pair() -> [(Document, StampId, Placement); 2] {
        std::array::from_fn(|_| {
            let mut doc = doc_from_raw(samples::occluded_quads(), 128);
            let blob = doc.add_image_blob("green.png", solid_png([0, 255, 0, 255])).unwrap();
            let start = front(0.3, 0.3, 0.0, 0.15, 1.0);
            let id = doc.add_stamp("G", StampSource::Image { blob }, Placement::Projected(start), TextureSetId(0)).unwrap();
            // Large enough to reach the rear quad, which only Preview (no occlusion rays) paints behind the front quad.
            (doc, id, Placement::Projected(front(0.5, 0.5, 0.0, 0.45, 1.0)))
        })
    }

    #[test]
    fn preview_then_flush_equals_final() {
        // Direct flush.
        let [(mut a, id, target), (mut b, _, _)] = occluded_pair();
        a.set_stamp_placement(id, target, Commit::Preview).unwrap();
        let preview_cache = cache_of(&a, a.stamp_layers(id)[0].1);
        a.flush_pending_edit();
        b.set_stamp_placement(id, target, Commit::Final).unwrap();
        let final_cache = cache_of(&b, b.stamp_layers(id)[0].1);
        assert_ne!(*preview_cache, *final_cache, "the preview of this edit differs from the final bake");
        assert_eq!(doc_state(&mut a), doc_state(&mut b));
        assert_eq!(labels(&a), labels(&b));
        assert_eq!(a.next_id, b.next_id);
        assert!(a.preview_baked.is_empty() && b.preview_baked.is_empty());
        assert_eq!(*cache_of(&a, a.stamp_layers(id)[0].1), *final_cache);

        // Flushed by undo(): undoing and redoing give the same states as for a Final edit.
        let [(mut a, id, target), (mut b, _, _)] = occluded_pair();
        a.set_stamp_placement(id, target, Commit::Preview).unwrap();
        b.set_stamp_placement(id, target, Commit::Final).unwrap();
        let final_state = doc_state(&mut b);
        assert_eq!(a.undo().as_deref(), Some(EDIT_LABEL));
        assert_eq!(b.undo().as_deref(), Some(EDIT_LABEL));
        assert!(a.preview_baked.is_empty());
        assert_eq!(doc_state(&mut a), doc_state(&mut b));
        a.redo();
        b.redo();
        assert_eq!(doc_state(&mut a), final_state);
        assert_eq!(doc_state(&mut b), final_state);

        // Flushed by another layer command.
        let [(mut a, id, target), (mut b, _, _)] = occluded_pair();
        a.set_stamp_placement(id, target, Commit::Preview).unwrap();
        a.set_layer_visible(LayerId(1), false).unwrap();
        b.set_stamp_placement(id, target, Commit::Final).unwrap();
        b.set_layer_visible(LayerId(1), false).unwrap();
        assert_eq!(doc_state(&mut a), doc_state(&mut b));
        assert_eq!(labels(&a), labels(&b));
        assert_eq!(labels(&a), ["Add Stamp", EDIT_LABEL, "Hide Layer"]);
        assert!(a.preview_baked.is_empty());
    }

    #[test]
    fn cancel_restores_the_full_quality_caches() {
        let [(mut doc, id, target), _] = occluded_pair();
        let before = doc_state(&mut doc);
        let layer = doc.stamp_layers(id)[0].1;
        let cache = cache_of(&doc, layer);
        doc.set_stamp_placement(id, target, Commit::Preview).unwrap();
        doc.set_stamp_tint(id, [10, 20, 30, 255], Commit::Preview).unwrap();
        assert!(doc.cancel_pending_edit());
        assert!(doc.preview_baked.is_empty());
        assert_eq!(doc_state(&mut doc), before);
        assert!(Arc::ptr_eq(&cache_of(&doc, layer), &cache));
        assert_eq!(doc.history.undo_len(), 1);
    }

    #[test]
    fn layers_that_lose_their_coverage_are_removed_on_final_except_at_home() {
        let mut doc = doc_from_raw(samples::two_material_model(), 128);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let both = front(0.0, 0.5, 0.5, 0.4, 0.6);
        let id = doc.add_stamp("D", StampSource::Image { blob }, Placement::Projected(both), TextureSetId(1)).unwrap();
        let head_layer = doc.stamp_layers(id)[1].1;

        // Onto the body only: during the drag the head's layer stays with an empty cache...
        let body_only = Placement::Projected(front(0.0, -0.2, 0.5, 0.2, 0.3));
        doc.set_stamp_placement(id, body_only, Commit::Preview).unwrap();
        let linked = doc.stamp_layers(id);
        assert_eq!(linked.len(), 2);
        assert!(cache_of(&doc, head_layer).is_empty());
        // ...and it is the home set, so it stays after Final too.
        doc.flush_pending_edit();
        assert_eq!(doc.stamp_layers(id).len(), 2);
        doc_state(&mut doc);

        // With the body as home, the empty head layer is removed on Final.
        let mut doc = doc_from_raw(samples::two_material_model(), 128);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let id = doc.add_stamp("D", StampSource::Image { blob }, Placement::Projected(both), TextureSetId(0)).unwrap();
        let head_layer = doc.stamp_layers(id)[1].1;
        doc.set_stamp_placement(id, body_only, Commit::Preview).unwrap();
        assert_eq!(doc.stamp_layers(id).len(), 2, "kept while previewing");
        doc.set_stamp_placement(id, body_only, Commit::Final).unwrap();
        assert_eq!(doc.stamp_layers(id).iter().map(|(s, _)| s.0).collect::<Vec<_>>(), [0]);
        assert!(doc.layer(head_layer).is_none());
        assert_eq!(doc.set(TextureSetId(1)).unwrap().active, Some(LayerId(2)));
        let moved = doc_state(&mut doc);
        // Moving back re-creates a layer in the head set (a new id) above its active layer.
        doc.set_stamp_placement(id, Placement::Projected(both), Commit::Final).unwrap();
        let again = doc.stamp_layers(id);
        assert_eq!(again.len(), 2);
        assert_ne!(again[1].1, head_layer);
        assert_eq!(doc.find_layer(again[1].1).unwrap(), (TextureSetId(1), 1));
        doc.undo();
        assert_eq!(doc_state(&mut doc), moved);
    }

    #[test]
    fn uv_stamps_follow_their_set() {
        let mut doc = doc_from_raw(samples::two_material_model(), 64);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let id = doc
            .add_stamp("U", StampSource::Image { blob }, Placement::Uv(uv(0, [20.0, 20.0], [16.0, 16.0])), TextureSetId(0))
            .unwrap();
        let cache = cache_of(&doc, doc.stamp_layers(id)[0].1);
        assert_eq!(cache.bounds(), IRect::new(12, 12, 28, 28));
        // Into the other set: the home follows, the old layer goes away on Final.
        doc.set_stamp_placement(id, Placement::Uv(uv(1, [30.0, 30.0], [16.0, 16.0])), Commit::Final).unwrap();
        assert_eq!(doc.stamp(id).unwrap().home_set, TextureSetId(1));
        assert_eq!(doc.stamp_layers(id).iter().map(|(s, _)| s.0).collect::<Vec<_>>(), [1]);
        assert_eq!(cache_of(&doc, doc.stamp_layers(id)[0].1).bounds(), IRect::new(22, 22, 38, 38));
        doc_state(&mut doc);
        doc.undo();
        assert_eq!(doc.stamp(id).unwrap().home_set, TextureSetId(0));
        doc_state(&mut doc);

        // Errors leave everything as it was.
        let before = doc_state(&mut doc);
        assert!(matches!(
            doc.set_stamp_placement(id, Placement::Uv(uv(5, [1.0, 1.0], [2.0, 2.0])), Commit::Final),
            Err(EditError::UnknownSet)
        ));
        assert!(matches!(
            doc.set_stamp_placement(id, Placement::Uv(uv(0, [f32::NAN, 1.0], [2.0, 2.0])), Commit::Preview),
            Err(EditError::InvalidParam(_))
        ));
        assert!(matches!(doc.set_stamp_placement(StampId(999), Placement::Uv(uv(0, [1.0, 1.0], [2.0, 2.0])), Commit::Final), Err(EditError::UnknownStamp)));
        assert!(matches!(doc.set_stamp_tint(StampId(999), [0; 4], Commit::Final), Err(EditError::UnknownStamp)));
        assert!(!doc.has_pending_edit());
        assert_eq!(doc_state(&mut doc), before);
    }

    #[test]
    fn replacing_an_image_source() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let red = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let blue = doc.add_image_blob("blue.png", solid_png([0, 0, 255, 255])).unwrap();
        let place = Placement::Uv(uv(0, [32.0, 32.0], [20.0, 20.0]));
        let id = doc.add_stamp("I", StampSource::Image { blob: red }, place, TextureSetId(0)).unwrap();
        let layer = doc.stamp_layers(id)[0].1;
        doc.set_stamp_source(id, StampSource::Image { blob: blue }, Commit::Final).unwrap();
        assert_eq!(cache_of(&doc, layer).get(32, 32), [0, 0, 255, 255]);
        assert_eq!(doc.stamp(id).unwrap().placement, place, "an image of the same aspect keeps the placement");
        assert_eq!(doc.history.undo_len(), 2);
        // Unknown blob, other kind: errors without a change.
        assert!(matches!(doc.set_stamp_source(id, StampSource::Image { blob: BlobId(77) }, Commit::Final), Err(EditError::UnknownBlob)));
        let spec = text_spec(&mut doc, "x");
        assert!(matches!(doc.set_stamp_source(id, StampSource::Text(spec), Commit::Final), Err(EditError::InvalidParam(_))));
        assert_eq!(doc.history.undo_len(), 2);
        doc.undo();
        assert_eq!(cache_of(&doc, layer).get(32, 32), [255, 0, 0, 255]);
    }

    #[test]
    fn a_replacement_image_of_another_aspect_is_not_distorted() {
        let mut doc = doc_from_raw(samples::flat_quad(), 64);
        let square = doc.add_image_blob("logo.png", solid_png([255, 0, 0, 255])).unwrap();
        let banner = doc.add_image_blob("banner.png", png_bytes(&PixelBuf::filled(30, 10, [0, 0, 255, 255]))).unwrap();
        let tall = doc.add_image_blob("tall.png", png_bytes(&PixelBuf::filled(8, 32, [0, 255, 0, 255]))).unwrap();
        // UV placement, rotated and flipped: only the size changes, the larger side is kept.
        let mut place = uv(0, [32.0, 32.0], [24.0, 24.0]);
        place.rotation = 0.5;
        place.flip_x = true;
        let id = doc.add_stamp("I", StampSource::Image { blob: square }, Placement::Uv(place), TextureSetId(0)).unwrap();
        doc.set_stamp_source(id, StampSource::Image { blob: banner }, Commit::Final).unwrap();
        let Placement::Uv(now) = doc.stamp(id).unwrap().placement else { panic!("not UV") };
        assert_eq!(now.size_px, Vec2::new(24.0, 8.0));
        assert_eq!((now.center_px, now.rotation, now.flip_x, now.flip_y), (place.center_px, 0.5, true, false));
        doc.set_stamp_source(id, StampSource::Image { blob: tall }, Commit::Final).unwrap();
        let Placement::Uv(now) = doc.stamp(id).unwrap().placement else { panic!("not UV") };
        assert_eq!(now.size_px, Vec2::new(6.0, 24.0));
        // One history entry per replacement: undo brings back the source and the size together.
        doc.undo();
        assert_eq!(doc.stamp(id).unwrap().source, StampSource::Image { blob: banner });
        assert!(matches!(doc.stamp(id).unwrap().placement, Placement::Uv(p) if p.size_px == Vec2::new(24.0, 8.0)));
        doc.undo();
        assert_eq!(doc.stamp(id).unwrap().placement, Placement::Uv(place));

        // Projected placement: the half size follows the same rule, the depth range stays.
        let p = front(0.5, 0.5, 0.0, 0.2, 0.3);
        let id = doc.add_stamp("P", StampSource::Image { blob: square }, Placement::Projected(p), TextureSetId(0)).unwrap();
        doc.set_stamp_source(id, StampSource::Image { blob: banner }, Commit::Final).unwrap();
        let now = projector_of(&doc, id);
        assert!((now.half_size - Vec2::new(0.2, 0.2 / 3.0)).length() < 1e-6, "{:?}", now.half_size);
        assert_eq!((now.origin, now.rotation, now.depth_front, now.depth_back), (p.origin, p.rotation, p.depth_front, p.depth_back));
    }

    /// Finding refit-aspect-discards-user-stretch: a deliberately stretched stamp keeps its stretch when its image is
    /// replaced (a "v2 of the same logo" keeps the placement exactly), and the stretch carries over to an image of
    /// another aspect, larger side kept.
    #[test]
    fn a_replacement_keeps_the_users_stretch() {
        let mut doc = doc_from_raw(samples::flat_quad(), 256);
        let v1 = doc.add_image_blob("logo_v1.png", png_bytes(&PixelBuf::filled(64, 64, [255, 0, 0, 255]))).unwrap();
        let v2 = doc.add_image_blob("logo_v2.png", png_bytes(&PixelBuf::filled(64, 64, [0, 255, 0, 255]))).unwrap();
        let banner = doc.add_image_blob("banner.png", png_bytes(&PixelBuf::filled(60, 20, [0, 0, 255, 255]))).unwrap();
        let id = doc.add_stamp("L", StampSource::Image { blob: v1 }, Placement::Uv(uv(0, [128.0, 128.0], [200.0, 200.0])), TextureSetId(0)).unwrap();
        // Stretched to 2:1 (what a Shift+corner drag does).
        let stretched = uv(0, [128.0, 128.0], [200.0, 100.0]);
        doc.set_stamp_placement(id, Placement::Uv(stretched), Commit::Final).unwrap();
        doc.set_stamp_source(id, StampSource::Image { blob: v2 }, Commit::Final).unwrap();
        assert_eq!(doc.stamp(id).unwrap().placement, Placement::Uv(stretched), "same aspect: placement unchanged");
        // A 3:1 banner with the 2:1 stretch: 6:1, the larger side (200) kept.
        doc.set_stamp_source(id, StampSource::Image { blob: banner }, Commit::Final).unwrap();
        let Placement::Uv(now) = doc.stamp(id).unwrap().placement else { panic!("not UV") };
        assert!((now.size_px - Vec2::new(200.0, 200.0 / 6.0)).length() < 1e-3, "{:?}", now.size_px);
        // Projected stamps too, with the sign of a flipped half size kept.
        let mut p = front(0.5, 0.5, 0.0, 0.2, 0.3);
        p.half_size = Vec2::new(0.2, -0.1);
        let id = doc.add_stamp("P", StampSource::Image { blob: v1 }, Placement::Projected(p), TextureSetId(0)).unwrap();
        doc.set_stamp_source(id, StampSource::Image { blob: v2 }, Commit::Final).unwrap();
        assert_eq!(projector_of(&doc, id).half_size, Vec2::new(0.2, -0.1));
    }

    /// Finding SP-1: with deferred Preview bakes a drag frame only updates the stamp object; the caches follow when
    /// `bake_stale_previews` runs, a Final bakes at full quality, a cancel restores the old caches.
    #[test]
    fn deferred_preview_bakes_wait_for_bake_stale_previews() {
        let mut doc = doc_from_raw(samples::flat_quad(), 128);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let id = doc.add_stamp("D", StampSource::Image { blob }, Placement::Uv(uv(0, [40.0, 40.0], [30.0, 30.0])), TextureSetId(0)).unwrap();
        let layer = doc.stamp_layers(id)[0].1;
        let mut immediate = doc_from_raw(samples::flat_quad(), 128);
        let blob2 = immediate.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let id2 = immediate
            .add_stamp("D", StampSource::Image { blob: blob2 }, Placement::Uv(uv(0, [40.0, 40.0], [30.0, 30.0])), TextureSetId(0))
            .unwrap();
        doc.set_defer_preview_bakes(true);
        let start = doc_state(&mut doc);
        let original = cache_of(&doc, layer);
        let undo = doc.history.undo_len();

        let moved = Placement::Uv(uv(0, [80.0, 70.0], [30.0, 30.0]));
        doc.set_stamp_placement(id, moved, Commit::Preview).unwrap();
        assert_eq!(doc.stamp(id).unwrap().placement, moved, "the stamp object follows at once");
        assert!(Arc::ptr_eq(&cache_of(&doc, layer), &original), "the bake waits");
        assert!(doc.has_stale_previews());
        doc.bake_stale_previews();
        assert!(!doc.has_stale_previews());
        assert_ne!(cache_of(&doc, layer).bounds(), original.bounds(), "the preview bake ran");
        // Cancel: everything as before.
        let again = Placement::Uv(uv(0, [90.0, 30.0], [30.0, 30.0]));
        doc.set_stamp_placement(id, again, Commit::Preview).unwrap();
        assert!(doc.cancel_pending_edit());
        assert!(!doc.has_stale_previews());
        assert_eq!(doc_state(&mut doc), start);
        // Preview (not baked yet), then Final: one entry, the same full-quality result as without deferral.
        doc.set_stamp_placement(id, again, Commit::Preview).unwrap();
        doc.set_stamp_placement(id, moved, Commit::Preview).unwrap();
        doc.set_stamp_placement(id, moved, Commit::Final).unwrap();
        assert!(!doc.has_stale_previews() && !doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), undo + 1);
        immediate.set_stamp_placement(id2, moved, Commit::Final).unwrap();
        assert_eq!(*cache_of(&doc, layer), *cache_of(&immediate, immediate.stamp_layers(id2)[0].1));
        doc_state(&mut doc);
        doc.set_defer_preview_bakes(false);
        assert!(!doc.has_stale_previews());
    }

    #[test]
    fn layer_commands_on_a_linked_layer_act_on_the_whole_stamp() {
        let mut doc = doc_from_raw(samples::two_material_model(), 128);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let both = front(0.0, 0.5, 0.5, 0.4, 0.6);
        let id = doc.add_stamp("Decal", StampSource::Image { blob }, Placement::Projected(both), TextureSetId(0)).unwrap();
        let linked = doc.stamp_layers(id);
        let start = doc_state(&mut doc);

        // Rename through the head set's layer: stamp and both layers, one entry.
        doc.rename_layer(linked[1].1, "Logo").unwrap();
        assert_eq!(doc.history.undo_len(), 2);
        assert_eq!(doc.stamp(id).unwrap().name, "Logo");
        assert!(linked.iter().all(|(_, l)| doc.layer(*l).unwrap().name == "Logo"));
        doc.rename_layer(linked[0].1, "Logo").unwrap();
        assert_eq!(doc.history.undo_len(), 2, "no change, no entry");

        // Duplicate through the head set's layer.
        let copy_layer = doc.duplicate_layer(linked[1].1).unwrap();
        assert_eq!(doc.history.undo_len(), 3);
        assert_eq!(doc.undo_label(), Some("Duplicate Stamp"));
        let copy = doc.stamp_of_layer(copy_layer).unwrap();
        assert_ne!(copy, id);
        assert_eq!(doc.stamp(copy).unwrap().name, "Logo copy");
        let copies = doc.stamp_layers(copy);
        assert_eq!(copies.len(), 2);
        assert_eq!(copies[1].1, copy_layer);
        for ((set, original), (copy_set, duplicate)) in linked.iter().zip(&copies) {
            assert_eq!(set, copy_set);
            let (_, index) = doc.find_layer(*original).unwrap();
            assert_eq!(doc.find_layer(*duplicate).unwrap().1, index + 1, "directly above the original");
            assert!(Arc::ptr_eq(&cache_of(&doc, *original), &cache_of(&doc, *duplicate)));
        }
        assert_eq!(doc.set(TextureSetId(0)).unwrap().active, Some(copies[0].1), "the active original's copy is active");
        let duplicated = doc_state(&mut doc);

        // Remove through a layer: the whole stamp goes, one entry.
        doc.remove_layer(copies[0].1).unwrap();
        assert!(doc.stamp(copy).is_none() && doc.stamp_layers(copy).is_empty());
        assert_eq!(doc.undo_label(), Some("Delete Stamp"));
        assert_eq!(doc.set(TextureSetId(0)).unwrap().active, Some(linked[0].1), "the layer below becomes active");
        doc.undo();
        assert_eq!(doc_state(&mut doc), duplicated);
        doc.undo();
        doc.undo();
        assert_eq!(doc_state(&mut doc), start);

        doc.remove_stamp(id).unwrap();
        assert!(doc.stamps.is_empty());
        assert_eq!(doc.set(TextureSetId(0)).unwrap().layers.len(), 1);
        assert_eq!(doc.set(TextureSetId(0)).unwrap().active, Some(LayerId(1)));
        assert!(matches!(doc.remove_stamp(id), Err(EditError::UnknownStamp)));
        assert!(matches!(doc.duplicate_stamp(id), Err(EditError::UnknownStamp)));
        assert!(matches!(doc.rename_stamp(id, "x"), Err(EditError::UnknownStamp)));
        doc_state(&mut doc);
    }

    #[test]
    fn a_locked_linked_layer_protects_the_stamp() {
        let mut doc = doc_from_raw(samples::flat_quad(), 256);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let other = doc.add_image_blob("blue.png", solid_png([0, 0, 255, 255])).unwrap();
        let set = TextureSetId(0);
        let start = uv(0, [100.0, 100.0], [60.0, 60.0]);
        let id = doc.add_stamp("A", StampSource::Image { blob }, Placement::Uv(start), set).unwrap();
        let layer = doc.stamp_layers(id)[0].1;
        assert!(!doc.stamp_locked(id));
        doc.set_layer_locked(layer, true).unwrap();
        assert!(doc.stamp_locked(id));
        let undo = doc.history.undo_len();
        let state = doc_state(&mut doc);

        let moved = Placement::Uv(uv(0, [120.0, 90.0], [60.0, 60.0]));
        for commit in [Commit::Preview, Commit::Final] {
            assert!(matches!(doc.set_stamp_placement(id, moved, commit), Err(EditError::LayerLocked)));
            assert!(matches!(doc.set_stamp_tint(id, [1, 2, 3, 255], commit), Err(EditError::LayerLocked)));
            assert!(matches!(doc.set_stamp_source(id, StampSource::Image { blob: other }, commit), Err(EditError::LayerLocked)));
        }
        assert!(matches!(doc.remove_stamp(id), Err(EditError::LayerLocked)));
        assert!(matches!(doc.remove_layer(layer), Err(EditError::LayerLocked)));
        assert!(!doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), undo, "a refused edit adds no entry");
        assert_eq!(doc_state(&mut doc), state);
        // Finding locked-guard-before-noop-check: an unchanged value (the Final a focused field sends when it loses
        // the focus to the lock icon) is a no-op, not a lock violation: Ok, no entry, nothing changes.
        let (source, tint) = (doc.stamp(id).unwrap().source.clone(), doc.stamp(id).unwrap().tint);
        for commit in [Commit::Preview, Commit::Final] {
            doc.set_stamp_placement(id, Placement::Uv(start), commit).unwrap();
            doc.set_stamp_tint(id, tint, commit).unwrap();
            doc.set_stamp_source(id, source.clone(), commit).unwrap();
        }
        assert!(!doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), undo, "an unchanged value adds no entry");
        assert_eq!(doc_state(&mut doc), state);
        // Renaming stays allowed (as for a locked paint layer), duplicating too; the copy is unlocked and editable.
        doc.rename_layer(layer, "Logo").unwrap();
        let copy_layer = doc.duplicate_layer(layer).unwrap();
        let copy = doc.stamp_of_layer(copy_layer).unwrap();
        assert!(!doc.stamp_locked(copy));
        doc.set_stamp_placement(copy, moved, Commit::Final).unwrap();
        // Unlocked again, the original can be edited and removed.
        doc.set_layer_locked(layer, false).unwrap();
        doc.set_stamp_placement(id, moved, Commit::Final).unwrap();
        doc.remove_stamp(id).unwrap();
        assert!(doc.stamp(id).is_none());
    }

    #[test]
    fn stamp_at_picks_the_topmost_visible_stamp() {
        let mut doc = doc_from_raw(samples::flat_quad(), 256);
        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let set = TextureSetId(0);
        let a = doc.add_stamp("A", StampSource::Image { blob }, Placement::Uv(uv(0, [100.0, 100.0], [60.0, 60.0])), set).unwrap();
        let b = doc.add_stamp("B", StampSource::Image { blob }, Placement::Uv(uv(0, [120.0, 120.0], [60.0, 60.0])), set).unwrap();
        let at = |doc: &Document, x: f32, y: f32| doc.stamp_at(set, Vec2::new(x / 256.0, y / 256.0));
        assert_eq!(at(&doc, 110.0, 110.0), Some(b), "B was added above A");
        assert_eq!(at(&doc, 80.0, 80.0), Some(a));
        assert_eq!(at(&doc, 10.0, 200.0), None);
        // Wrapped.
        assert_eq!(at(&doc, 256.0 + 110.0, -256.0 + 110.0), Some(b));
        assert_eq!(doc.stamp_at(set, Vec2::new(f32::NAN, 0.5)), None);
        assert_eq!(doc.stamp_at(TextureSetId(4), Vec2::splat(0.4)), None);
        // Hidden layers are skipped; order counts, not age.
        doc.set_layer_visible(doc.stamp_layers(b)[0].1, false).unwrap();
        assert_eq!(at(&doc, 110.0, 110.0), Some(a));
        doc.set_layer_visible(doc.stamp_layers(b)[0].1, true).unwrap();
        doc.move_layer(doc.stamp_layers(a)[0].1, 5).unwrap();
        assert_eq!(at(&doc, 110.0, 110.0), Some(a));
    }

    #[test]
    fn rebake_restores_a_lost_cache_without_history() {
        let mut doc = doc_from_raw(samples::two_material_model(), 128);
        let blob = doc.add_image_blob("stamp.png", png_bytes(&samples::sample_stamp_image(64))).unwrap();
        let id = doc
            .add_stamp("R", StampSource::Image { blob }, Placement::Projected(front(0.0, 0.5, 0.5, 0.4, 0.6)), TextureSetId(0))
            .unwrap();
        let good = doc_state(&mut doc);
        let entries = doc.history.undo_len();
        let revision = doc.revision();
        for (_, layer) in doc.stamp_layers(id) {
            *cache_mut(doc.layer_mut(layer).unwrap()).unwrap() = Arc::new(SparseBuf::empty());
        }
        doc.rebake_stamp(id, BakeQuality::Full).unwrap();
        assert_eq!(doc_state(&mut doc), good);
        assert_eq!((doc.history.undo_len(), doc.revision()), (entries, revision));
        assert!(matches!(doc.rebake_stamp(StampId(4242), BakeQuality::Full), Err(EditError::UnknownStamp)));
        // finalize of an unknown id is a no-op.
        doc.finalize_stamp_bake(StampId(4242));
        assert_eq!(doc_state(&mut doc), good);
    }

    /// A text stamp on the flat quad (256 px texture, 1/256 world units per texel) whose em is `em` world units high.
    fn text_doc(content: &str, em: f32, outline: Option<Outline>) -> (Document, StampId) {
        let mut doc = doc_from_raw(samples::flat_quad(), 256);
        let mut spec = text_spec(&mut doc, content);
        spec.outline = outline;
        let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
        let hy = em * h as f32 / TEXT_PLACEMENT_PX / 2.0;
        let p = Projector::from_surface(Vec3::new(0.5, 0.5, 0.0), Vec3::Z, Vec3::Y, Vec2::new(hy * w as f32 / h as f32, hy), 0.2);
        let id = doc.add_stamp("T", StampSource::Text(spec), Placement::Projected(p), TextureSetId(0)).unwrap();
        (doc, id)
    }

    #[test]
    fn add_text_picks_the_raster_resolution_and_keeps_the_em_size() {
        // 0.1 world units = 25.6 texels per em -> the 32 px bucket.
        let (mut doc, id) = text_doc("Hi", 0.1, None);
        assert_eq!(doc.undo_label(), Some("Add Text"));
        assert_eq!(spec_of(&doc, id).size_px, 32.0);
        assert_eq!(doc.text_raster_px(id), 32.0);
        let em = em_texels(&mut doc, id);
        assert!((em - 25.6).abs() < 0.256, "{em}");
        let layer = doc.stamp_layers(id)[0].1;
        assert_eq!(doc.layer(layer).unwrap().kind_name(), "text");
        assert!(!cache_of(&doc, layer).is_empty());
        doc_state(&mut doc);
        // A tiny em uses the 32 px floor, a huge one the 512 px cap.
        let (doc, id) = text_doc("a", 0.01, None);
        assert_eq!(spec_of(&doc, id).size_px, 32.0);
        let (doc, id) = text_doc("a", 3.0, None);
        assert_eq!(spec_of(&doc, id).size_px, 512.0);
    }

    #[test]
    fn scaling_text_four_times_rasterizes_it_finer() {
        let outline = Some(Outline { width_em: 0.1, color: [0, 0, 0, 255] });
        let (mut doc, id) = text_doc("Hi", 0.1, outline);
        let layer = doc.stamp_layers(id)[0].1;
        assert_eq!(spec_of(&doc, id).size_px, 32.0);
        let mut p = projector_of(&doc, id);
        p.half_size *= 4.0;

        // Preview keeps the raster resolution (magnified while dragging).
        doc.set_stamp_placement(id, Placement::Projected(p), Commit::Preview).unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 32.0);
        let em_preview = em_texels(&mut doc, id);
        let ink_preview = ink(&cache_of(&doc, layer));
        let outline_texels = |doc: &mut Document| 0.1 * em_texels(doc, id);
        let outline_preview = outline_texels(&mut doc);

        // Final: 102.4 texels per em -> the 128 px bucket, at least one raster pixel per texel.
        doc.set_stamp_placement(id, Placement::Projected(p), Commit::Final).unwrap();
        let spec = spec_of(&doc, id);
        assert_eq!(spec.size_px, 128.0);
        let em_final = em_texels(&mut doc, id);
        assert!((em_final / em_preview - 1.0).abs() < 0.01, "{em_preview} -> {em_final}");
        assert!(spec.size_px / em_final >= 1.0);
        let ink_final = ink(&cache_of(&doc, layer));
        for (a, b) in [(ink_preview.width(), ink_final.width()), (ink_preview.height(), ink_final.height())] {
            // One preview raster pixel is 3.2 texels: allow 1% or 3 texels.
            assert!((a as f32 - b as f32).abs() <= (0.01 * a as f32).max(3.0), "{ink_preview:?} vs {ink_final:?}");
        }
        assert!((outline_texels(&mut doc) / outline_preview - 1.0).abs() < 0.01);
        assert_eq!(doc.history.undo_len(), 2);

        // Way up: capped at 512.
        p.half_size *= 8.0;
        doc.set_stamp_placement(id, Placement::Projected(p), Commit::Final).unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 512.0);
        // Undo returns to the 128 px raster and its placement.
        doc.undo();
        assert_eq!(spec_of(&doc, id).size_px, 128.0);
        doc_state(&mut doc);
    }

    #[test]
    fn editing_the_text_keeps_the_em_size_on_the_model() {
        let (mut doc, id) = text_doc("Hi", 0.1, None);
        let p0 = projector_of(&doc, id);
        let em0 = em_texels(&mut doc, id);
        let mut spec = spec_of(&doc, id);
        spec.content = "Hi there".to_owned();
        spec.size_px = 300.0; // ignored
        doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Preview).unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 32.0);
        let p1 = projector_of(&doc, id);
        assert!((p1.half_size.y - p0.half_size.y).abs() < 1e-6, "one line: same height");
        assert!(p1.half_size.x > 2.0 * p0.half_size.x, "the block grows with the text");
        assert!((em_texels(&mut doc, id) / em0 - 1.0).abs() < 1e-4);
        doc.flush_pending_edit();
        assert_eq!(doc.undo_label(), Some(EDIT_LABEL));
        assert!((em_texels(&mut doc, id) / em0 - 1.0).abs() < 0.01);

        // Two lines: the block gets taller, the em stays.
        spec.content = "Hi\nthere".to_owned();
        doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Final).unwrap();
        assert!(projector_of(&doc, id).half_size.y > 1.5 * p0.half_size.y);
        assert!((em_texels(&mut doc, id) / em0 - 1.0).abs() < 0.01);

        // Emptied text keeps its (fully transparent) layer.
        spec.content = String::new();
        doc.set_stamp_source(id, StampSource::Text(spec), Commit::Final).unwrap();
        let layers = doc.stamp_layers(id);
        assert_eq!(layers.len(), 1);
        assert!(cache_of(&doc, layers[0].1).is_empty());
        assert_eq!(doc.stamp_at(TextureSetId(0), Vec2::splat(0.5)), None);
        doc_state(&mut doc);

        // Undo all the way back to the original text.
        doc.undo();
        doc.undo();
        doc.undo();
        assert_eq!(projector_of(&doc, id), p0);
        assert_eq!(spec_of(&doc, id).content, "Hi");
        doc_state(&mut doc);
    }

    /// Name of the stamp and of each of its linked layers.
    fn names_of(doc: &Document, id: StampId) -> (String, Vec<String>) {
        let layers = doc.stamp_layers(id).into_iter().map(|(_, l)| doc.layer(l).unwrap().name.clone()).collect();
        (doc.stamp(id).unwrap().name.clone(), layers)
    }

    /// Fix round 3, R3-9: a text stamp with the automatic name of its text follows the text, in the same history entry;
    /// a name the user chose is kept.
    #[test]
    fn a_text_stamp_with_its_automatic_name_follows_its_text() {
        let named = |name: &str| (name.to_owned(), vec![name.to_owned()]);
        let (mut doc, id) = text_doc("WTE", 0.1, None);
        doc.rename_stamp(id, &text_stamp_name("WTE")).unwrap();
        assert_eq!(names_of(&doc, id), named("WTE"));
        let entries = doc.history.undo_len();
        let mut spec = spec_of(&doc, id);

        // Final edit: text and name change in one entry; one undo restores both, redo repeats both.
        spec.content = "WTE 2026".to_owned();
        doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Final).unwrap();
        assert_eq!(names_of(&doc, id), named("WTE 2026"));
        assert_eq!((doc.history.undo_len(), doc.undo_label()), (entries + 1, Some(EDIT_LABEL)));
        doc.undo();
        assert_eq!((spec_of(&doc, id).content.as_str(), names_of(&doc, id)), ("WTE", named("WTE")));
        doc.redo();
        assert_eq!((spec_of(&doc, id).content.as_str(), names_of(&doc, id)), ("WTE 2026", named("WTE 2026")));
        doc.undo();

        // Typing (Preview edits): the name follows every step; Esc (cancel) leaves no half-renamed state.
        for text in ["WTE ", "WTE 2", "WTE 20"] {
            spec.content = text.to_owned();
            doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Preview).unwrap();
            assert_eq!(names_of(&doc, id), named(text.trim()));
        }
        assert!(doc.cancel_pending_edit());
        assert_eq!((spec_of(&doc, id).content.as_str(), names_of(&doc, id)), ("WTE", named("WTE")));
        assert_eq!(doc.history.undo_len(), entries);

        // Typing then committing: one entry for the whole typing, one undo restores text and name.
        for text in ["WTE 2", "WTE 20", "Second line\nWTE"] {
            spec.content = text.to_owned();
            doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Preview).unwrap();
        }
        doc.flush_pending_edit();
        assert_eq!(names_of(&doc, id), named("Second line"));
        assert_eq!(doc.history.undo_len(), entries + 1);
        doc.undo();
        assert_eq!((spec_of(&doc, id).content.as_str(), names_of(&doc, id)), ("WTE", named("WTE")));
        doc_state(&mut doc);

        // A renamed stamp keeps its name.
        doc.rename_stamp(id, "Logo").unwrap();
        spec.content = "Other".to_owned();
        doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Final).unwrap();
        assert_eq!(names_of(&doc, id), named("Logo"));
        // So does a stamp named "T" by its caller (not the automatic name of "Hi").
        let (mut doc, id) = text_doc("Hi", 0.1, None);
        let mut spec = spec_of(&doc, id);
        spec.content = "Ho".to_owned();
        doc.set_stamp_source(id, StampSource::Text(spec), Commit::Final).unwrap();
        assert_eq!(names_of(&doc, id), named("T"));
    }

    /// R3-9: a text change that keeps the automatic name (same first line) and a style-only change rename nothing,
    /// and an emptied text is named "Text".
    #[test]
    fn a_text_edit_with_the_same_first_line_keeps_the_name() {
        let named = |name: &str| (name.to_owned(), vec![name.to_owned()]);
        let (mut doc, id) = text_doc("Hello", 0.1, None);
        doc.rename_stamp(id, "Hello").unwrap();
        let mut spec = spec_of(&doc, id);
        spec.content = "Hello\nWorld".to_owned();
        doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Final).unwrap();
        assert_eq!(names_of(&doc, id), named("Hello"));
        spec.color = [10, 20, 30, 255];
        doc.set_stamp_source(id, StampSource::Text(spec.clone()), Commit::Final).unwrap();
        assert_eq!(names_of(&doc, id), named("Hello"));
        spec.content = String::new();
        doc.set_stamp_source(id, StampSource::Text(spec), Commit::Final).unwrap();
        assert_eq!(names_of(&doc, id), named("Text"));
    }

    /// Fix round 2, item 2: a longer text widens a projected text stamp; an automatic depth range is re-fitted to the
    /// wider footprint on the curved surface (else its ends were cut off), a hand-set one is kept.
    #[test]
    fn a_longer_text_refits_an_automatic_depth_and_keeps_a_hand_set_one() {
        let mut doc = doc_from_raw(samples::uv_sphere(64, 32, 1.0), 256);
        let spec = text_spec(&mut doc, "Hi");
        let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
        let hy = 0.15 * h as f32 / TEXT_PLACEMENT_PX;
        let p = Projector::from_surface(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::Y, Vec2::new(hy * w as f32 / h as f32, hy), 0.0);
        let p = doc.fit_stamp_depth(&p);
        let id = doc.add_stamp("T", StampSource::Text(spec.clone()), Placement::Projected(p), TextureSetId(0)).unwrap();
        let longer = |doc: &mut Document, content: &str| {
            let mut s = spec_of(doc, id);
            s.content = content.to_owned();
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Final).unwrap();
            projector_of(doc, id)
        };
        let p1 = longer(&mut doc, "Hi there, everyone");
        assert!(p1.half_size.x > 3.0 * p.half_size.x, "{p:?} -> {p1:?}");
        assert_eq!(p1, doc.fit_stamp_depth(&p1), "the depth follows the wider footprint");
        assert!(p1.depth_back > 2.0 * p.depth_back, "{p:?} -> {p1:?}");

        // A hand-set depth stays as it is.
        let typed = Projector { depth_back: 0.123, depth_front: 0.045, ..p1 };
        doc.set_stamp_placement(id, Placement::Projected(typed), Commit::Final).unwrap();
        assert!(!doc.stamp_depth_is_auto(&typed));
        let p2 = longer(&mut doc, "Hi");
        assert!(p2.half_size.x < p1.half_size.x);
        assert_eq!((p2.depth_front, p2.depth_back), (0.045, 0.123));
        doc_state(&mut doc);
    }

    #[test]
    fn text_raster_px_of_uv_text_and_of_other_stamps() {
        let mut doc = doc_from_raw(samples::flat_quad(), 256);
        let spec = text_spec(&mut doc, "UV");
        let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
        // 40 texels per em.
        let size = Vec2::new(40.0 * w as f32 / TEXT_PLACEMENT_PX, 40.0 * h as f32 / TEXT_PLACEMENT_PX);
        let id = doc
            .add_stamp("U", StampSource::Text(spec), Placement::Uv(uv(0, [128.0, 128.0], size.to_array())), TextureSetId(0))
            .unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 64.0);
        let Placement::Uv(p) = doc.stamp(id).unwrap().placement else { panic!() };
        let [_, h1] = doc.stamp_source_size(&StampSource::Text(spec_of(&doc, id))).unwrap();
        let em = p.size_px.y * 64.0 / h1 as f32;
        assert!((em - 40.0).abs() < 0.4, "{em}");

        let blob = doc.add_image_blob("red.png", solid_png([255, 0, 0, 255])).unwrap();
        let image = doc.add_stamp("I", StampSource::Image { blob }, Placement::Uv(uv(0, [20.0, 20.0], [8.0, 8.0])), TextureSetId(0)).unwrap();
        assert_eq!(doc.text_raster_px(image), TEXT_PLACEMENT_PX);
        assert_eq!(doc.text_raster_px(StampId(999)), TEXT_PLACEMENT_PX);
    }

    /// Timing probe on real awkward models (not a regression test, fix round 3 item R3-4): `cargo test -p wte_core
    /// --release -- --ignored stamp_bench --nocapture`. `WTE_STAMP_BENCH_MODEL` picks the model (default
    /// target/compat/harness/shapes/ngons_tiling.obj). Places an image stamp of 20 % of the model size (the workflow
    /// scenario's) at several points, moves it, re-tints it, and prints where the time goes.
    #[test]
    #[ignore = "timing probe on a local model file"]
    fn stamp_bench_tiled_model() {
        use crate::math::Ray;
        use crate::stamp_bake::probe;
        let path = match std::env::var("WTE_STAMP_BENCH_MODEL") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/compat/harness/shapes/ngons_tiling.obj"),
        };
        if !path.exists() {
            return;
        }
        let model = crate::import::import_from_path(&path, &Default::default(), &crate::progress::NoProgress).unwrap();
        let n = model.mesh.sets.len();
        let mut doc =
            Document::from_import(model, vec![crate::doc_build::TextureChoice::Detected; n], &crate::progress::NoProgress).unwrap();
        doc.recomposite_all();
        for s in &doc.sets {
            eprintln!("set {}x{} texel_world {}", s.width, s.height, s.texel_map.stats.texel_world);
        }
        let blob = doc.add_image_blob("stamp.png", png_bytes(&samples::sample_stamp_image(256))).unwrap();
        let (c, d) = (doc.mesh.bounds.center(), doc.mesh.diag());
        let b = doc.mesh.bounds;
        let mut hits = Vec::new();
        for (fx, fy) in [(0.5f32, 0.5f32), (0.3, 0.6), (0.7, 0.4), (0.45, 0.3), (0.6, 0.7)] {
            let x = b.min.x + fx * (b.max.x - b.min.x);
            let y = b.min.y + fy * (b.max.y - b.min.y);
            if let Some(h) = doc.raycast(&Ray::new(Vec3::new(x, y, c.z + d), -Vec3::Z)) {
                hits.push(h);
            }
        }
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
        let report = |what: &str| {
            let s = probe::take();
            eprintln!(
                "  {what}: candidates {:.1} ms (occluder grid {:.1} ms), raster {:.1} ms, crop {:.1} ms, scratch {:.2} Mtx, frags {:.2} M, shaded {:.2} M, rays {:.2} M (occluded {:.2} M, proven: occluded {:.2} M, clear {:.2} M), jobs {}",
                s[0] as f64 / 1e6,
                s[11] as f64 / 1e6,
                s[1] as f64 / 1e6,
                s[2] as f64 / 1e6,
                s[3] as f64 / 1e6,
                s[4] as f64 / 1e6,
                s[5] as f64 / 1e6,
                s[6] as f64 / 1e6,
                s[8] as f64 / 1e6,
                s[9] as f64 / 1e6,
                s[10] as f64 / 1e6,
                s[7]
            );
        };
        let half = Vec2::splat(0.2 * doc.mesh.bounds.diagonal() / 2.0);
        for hit in hits {
            // As the Stamp tool builds it (8.6).
            let surface_normal = hit.normal;
            let mut p0 = Projector::from_surface(hit.position, surface_normal, Vec3::Y, half, half.max_element());
            p0.backface_cull = hit.front_face;
            let t = std::time::Instant::now();
            let p = doc.fit_stamp_depth(&p0);
            eprintln!("at {:?}: fit {:.1} ms (depth {} / {})", hit.position, ms(t), p.depth_front, p.depth_back);
            let t = std::time::Instant::now();
            assert!(doc.stamp_depth_is_auto(&p));
            eprintln!("  is_auto {:.1} ms", ms(t));
            let maps = doc.texel_maps();
            let mips = doc.image_source(blob).unwrap();
            probe::take();
            let mut full_result: Option<Vec<(TextureSetId, SparseBuf)>> = None;
            for (what, quality, occlusion, counted) in [
                ("Full", BakeQuality::Full, true, false),
                ("Full, no grid", BakeQuality::Full, true, false),
                ("Full again", BakeQuality::Full, true, false),
                ("Full, no grid again", BakeQuality::Full, true, false),
                ("Full, counted (slower)", BakeQuality::Full, true, true),
                ("Full, no occlusion", BakeQuality::Full, false, false),
                ("Preview", BakeQuality::Preview, true, false),
            ] {
                probe::COUNT_FRAGMENTS.store(counted, std::sync::atomic::Ordering::Relaxed);
                probe::NO_GRID.store(what.contains("no grid"), std::sync::atomic::Ordering::Relaxed);
                let q = Projector { occlusion, ..p };
                let t = std::time::Instant::now();
                let r = bake_projected(&doc.mesh, &doc.bvh, &maps, &mips, [255; 4], &q, quality);
                let texels: usize = r.iter().map(|(_, b)| b.parts().iter().map(|p| p.data.len() / 4).sum::<usize>()).sum();
                eprintln!("  bake {what}: {:.1} ms, {} sets, {:.2} M stored texels", ms(t), r.len(), texels as f64 / 1e6);
                report(what);
                // The occluder grid (cover, floor, cell proofs) changes no byte of a Full bake.
                if quality == BakeQuality::Full && occlusion {
                    match &full_result {
                        None => full_result = Some(r),
                        Some(first) => assert!(*first == r, "{what}: differs from the first Full bake"),
                    }
                }
                probe::COUNT_FRAGMENTS.store(false, std::sync::atomic::Ordering::Relaxed);
                probe::NO_GRID.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            // Against the bake of section 6.5 written plainly (every fragment shaded and ray-tested by the BVH).
            let t = std::time::Instant::now();
            let reference = crate::stamp_bake::testkit::reference_bake(&doc.mesh, &doc.bvh, &maps, &mips, [255; 4], &p, BakeQuality::Full);
            let ours = full_result.expect("a Full bake ran");
            for (s, image) in reference.iter().enumerate() {
                let (w, h) = (image.width, image.height);
                let baked = ours.iter().find(|(id, _)| id.0 == s as u32).map_or_else(|| PixelBuf::new(w, h), |(_, b)| b.to_dense(w, h));
                assert!(baked.data == image.data, "set {s}: the bake differs from the plain reference");
            }
            eprintln!("  plain reference bake {:.1} ms: identical", ms(t));
            let t = std::time::Instant::now();
            let id = doc.add_stamp("S", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
            let add = ms(t);
            probe::take();
            let t = std::time::Instant::now();
            doc.recomposite_all();
            eprintln!("  add_stamp {add:.1} ms, recomposite {:.1} ms", ms(t));
            let moved = p.moved_to(p.origin + Vec3::new(0.02 * d, 0.0, 0.0), surface_normal);
            let t = std::time::Instant::now();
            doc.set_stamp_placement(id, Placement::Projected(moved), Commit::Preview).unwrap();
            let preview = ms(t);
            let t = std::time::Instant::now();
            doc.set_stamp_placement(id, Placement::Projected(moved), Commit::Final).unwrap();
            let fin = ms(t);
            probe::take();
            let t = std::time::Instant::now();
            doc.recomposite_all();
            eprintln!("  move: preview {preview:.1} ms, final {fin:.1} ms, recomposite {:.1} ms", ms(t));
            let t = std::time::Instant::now();
            doc.set_stamp_tint(id, [40, 200, 90, 255], Commit::Final).unwrap();
            let tint = ms(t);
            probe::take();
            let t = std::time::Instant::now();
            doc.recomposite_all();
            eprintln!("  tint final {tint:.1} ms, recomposite {:.1} ms", ms(t));

            // A long text stamp as the Text tool places it (em = 6 % of the model size, 8.6).
            for content in ["two\nlines", "A much longer text stamp line for wrapping"] {
                let spec = text_spec(&mut doc, content);
                let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
                let hy = 0.06 * doc.mesh.bounds.diagonal() * h as f32 / TEXT_PLACEMENT_PX / 2.0;
                let mut p = Projector::from_surface(hit.position, surface_normal, Vec3::Y, Vec2::new(hy * w as f32 / h as f32, hy), hy);
                p.backface_cull = hit.front_face;
                let p = doc.fit_stamp_depth(&p);
                probe::take();
                let t = std::time::Instant::now();
                let id = doc.add_stamp("T", StampSource::Text(spec), Placement::Projected(p), TextureSetId(0)).unwrap();
                let add = ms(t);
                report("text add_stamp");
                let raster = doc.text_mips.get(&id).map(|(_, m)| m.size());
                eprintln!("  text {content:?}: add_stamp {add:.1} ms, raster {raster:?}, half {:?}", p.half_size);
                for (counted, quality) in [(false, BakeQuality::Full), (true, BakeQuality::Full), (false, BakeQuality::Preview)] {
                    probe::COUNT_FRAGMENTS.store(counted, std::sync::atomic::Ordering::Relaxed);
                    let t = std::time::Instant::now();
                    let mips = Arc::clone(&doc.text_mips[&id].1);
                    let r = bake_projected(&doc.mesh, &doc.bvh, &doc.texel_maps(), &mips, [255; 4], &projector_of(&doc, id), quality);
                    eprintln!("  text bake alone {:.1} ms ({} sets, counted {counted}, {quality:?})", ms(t), r.len());
                    report("text bake");
                }
                probe::COUNT_FRAGMENTS.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

/// Round 4, T4 (R4-16): the document side of the text anchor rule, the too-large fallback, the frame cache and the
/// preview pictures (FEATURE_TEXT 5.8, 5.11; the "Document" tests of 8.1).
#[cfg(test)]
mod text_document_tests {
    use glam::{Vec2, Vec3};

    use super::*;
    use crate::history::testkit::doc_state;
    use crate::image_io::PngLevel;
    use crate::progress::NoProgress;
    use crate::samples;
    use crate::stamp::UvPlacement;
    use crate::testutil::{doc_from_raw, temp_dir};
    use crate::text::frame::EmMap;
    use crate::text::path::{new_circle, new_curve};
    use crate::text::{Gap, PathShape, TextAlign, TextFx, TextPath, Warp, WarpStyle};

    /// Side of the flat quad's texture in every test here: one world unit is 256 texels.
    const SIDE: u32 = 256;

    fn spec(doc: &mut Document, content: &str, align: TextAlign) -> TextSpec {
        TextSpec {
            content: content.to_owned(),
            font: doc.default_font(),
            size_px: TEXT_PLACEMENT_PX,
            color: [255, 255, 255, 255],
            align,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: TextFx::default(),
        }
    }

    fn spec_of(doc: &Document, id: StampId) -> TextSpec {
        match &doc.stamp(id).unwrap().source {
            StampSource::Text(spec) => spec.clone(),
            StampSource::Image { .. } => panic!("not a text stamp"),
        }
    }

    fn placement_of(doc: &Document, id: StampId) -> Placement {
        doc.stamp(id).unwrap().placement
    }

    fn cache(doc: &Document, id: StampId) -> Arc<SparseBuf> {
        let layer = doc.stamp_layers(id)[0].1;
        match &doc.layer(layer).unwrap().kind {
            LayerKind::Stamp { cache, .. } | LayerKind::Text { cache, .. } => Arc::clone(cache),
            LayerKind::Raster(_) => panic!("not a stamp layer"),
        }
    }

    /// Texel of a world point of the flat quad (x -> u, y -> 1 - v).
    fn texel_of_world(p: Vec3) -> Vec2 {
        Vec2::new(SIDE as f32 * p.x, SIDE as f32 * (1.0 - p.y))
    }

    #[derive(Copy, Clone, Debug, PartialEq)]
    enum Kind {
        Projected,
        Uv,
    }

    /// A text stamp "content" on the flat quad, centred at (128, 128) texels, its picture's x axis turned by
    /// `rotation_deg`, one em `em` texels high.
    fn place(doc: &mut Document, s: TextSpec, kind: Kind, rotation_deg: f32, em: f32, flip_x: bool) -> StampId {
        let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
        let (hw, hh) = (em * w as f32 / s.size_px, em * h as f32 / s.size_px);
        let r = rotation_deg.to_radians();
        let placement = match kind {
            Kind::Projected => {
                // Picture x along (cos r, sin r) in texels = (cos r, -sin r) in world; up = -(picture y).
                let up = Vec3::new(r.sin(), r.cos(), 0.0);
                let half = Vec2::new(hw, hh) / (2.0 * SIDE as f32);
                let mut p = Projector::from_surface(Vec3::new(0.5, 0.5, 0.0), Vec3::Z, up, half, 0.2);
                p.flip_x = flip_x;
                Placement::Projected(p)
            }
            Kind::Uv => Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: Vec2::splat(128.0),
                size_px: Vec2::new(hw, hh),
                rotation: r,
                flip_x,
                flip_y: false,
                clip_to_islands: false,
            }),
        };
        doc.add_stamp("T", StampSource::Text(s), placement, TextureSetId(0)).unwrap()
    }

    /// Host point (texels of the flat quad) of the em point `em` of a stamp (relative to its anchor).
    fn host(doc: &mut Document, id: StampId, em: [f32; 2]) -> Vec2 {
        let frame = doc.stamp_text_frame(id).unwrap();
        let placement = placement_of(doc, id);
        let map = EmMap::new(&placement, &frame);
        match placement {
            Placement::Projected(_) => texel_of_world(map.to_world(em)),
            Placement::Uv(_) => map.to_texel(em),
        }
    }

    /// Unit direction of the picture's x axis in texels.
    fn x_axis(doc: &mut Document, id: StampId) -> Vec2 {
        (host(doc, id, [1.0, 0.0]) - host(doc, id, [0.0, 0.0])).normalize()
    }

    /// Host size of one em in texels, per picture axis.
    fn em_host(doc: &mut Document, id: StampId) -> Vec2 {
        let a = host(doc, id, [0.0, 0.0]);
        Vec2::new(host(doc, id, [1.0, 0.0]).distance(a), host(doc, id, [0.0, 1.0]).distance(a))
    }

    /// Centre of the rectangle in texels.
    fn centre(doc: &Document, id: StampId) -> Vec2 {
        match placement_of(doc, id) {
            Placement::Projected(p) => texel_of_world(p.origin),
            Placement::Uv(p) => p.center_px,
        }
    }

    /// Full width of the rectangle in texels.
    fn rect_width(doc: &Document, id: StampId) -> f32 {
        match placement_of(doc, id) {
            Placement::Projected(p) => 2.0 * p.half_size.x * SIDE as f32,
            Placement::Uv(p) => p.size_px.x,
        }
    }

    /// The baked ink (alpha >= 128) of a stamp split at the widest empty stretch along `axis` (the word space):
    /// (smallest, mean) of the part before it and (mean, largest) of the part after it, as coordinates along `axis`.
    fn ink_parts(cache: &SparseBuf, axis: Vec2) -> ([f32; 2], [f32; 2]) {
        let b = cache.bounds();
        let mut along = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                if cache.get(x, y)[3] >= 128 {
                    along.push(Vec2::new(x as f32 + 0.5, y as f32 + 0.5).dot(axis));
                }
            }
        }
        along.sort_by(f32::total_cmp);
        let split = (1..along.len()).max_by(|&i, &j| (along[i] - along[i - 1]).total_cmp(&(along[j] - along[j - 1]))).unwrap();
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let (left, right) = along.split_at(split);
        ([left[0], mean(left)], [mean(right), right[right.len() - 1]])
    }

    /// TEXT 8.1 `a_spacing_edit_keeps_the_alignment_edge`.
    #[test]
    fn a_spacing_edit_keeps_the_alignment_edge() {
        // em 40 texels -> the 64 px bucket: 0.5 em is 32 raster pixels, 20 texels.
        let em = 40.0;
        for kind in [Kind::Projected, Kind::Uv] {
            for align in [TextAlign::Left, TextAlign::Right, TextAlign::Center] {
                for gap in [true, false] {
                    let what = format!("{kind:?} {align:?} {}", if gap { "gap 0.5 em after E" } else { "letter spacing 10 %" });
                    let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
                    let s = spec(&mut doc, "WTE 2026", align);
                    let id = place(&mut doc, s, kind, 30.0, em, false);
                    assert_eq!(spec_of(&doc, id).size_px, 64.0, "{what}");
                    let axis = x_axis(&mut doc, id);
                    let (em0, c0, w0) = (em_host(&mut doc, id), centre(&doc, id), rect_width(&doc, id));
                    let p0 = placement_of(&doc, id);
                    let (l0, r0) = ink_parts(&cache(&doc, id), axis);

                    let mut edited = spec_of(&doc, id);
                    let widen = if gap {
                        edited.fx.gaps = vec![Gap { after: 2, em: 0.5 }];
                        0.5
                    } else {
                        edited.letter_spacing = 0.1;
                        0.7 // seven spaces between eight characters
                    };
                    doc.set_stamp_source(id, StampSource::Text(edited), Commit::Final).unwrap();
                    assert_eq!(spec_of(&doc, id).size_px, 64.0, "{what}: same raster resolution");
                    let (em1, c1, w1) = (em_host(&mut doc, id), centre(&doc, id), rect_width(&doc, id));
                    let (l1, r1) = ink_parts(&cache(&doc, id), axis);

                    // One em keeps its size on the model within 0.1 %; the rectangle is wider by the added spacing
                    // (within 1 %, or one raster pixel when the spacing is not a whole number of pixels).
                    assert!(((em1 / em0) - 1.0).abs().max_element() < 0.001, "{what}: em {em0} -> {em1}");
                    let added = (w1 - w0) / em0.x;
                    let tolerance = if gap { 0.01 * widen } else { 1.0 / 64.0 };
                    assert!((added - widen).abs() <= tolerance, "{what}: wider by {added} em, not {widen}");
                    let moved = (c1 - c0).dot(axis) / em0.x;
                    let shift = widen * em0.x;
                    match align {
                        TextAlign::Left => {
                            // The left edge of the ink stays; with a gap all of "WTE" stays (letter spacing moves
                            // every letter after the first).
                            assert!((l1[0] - l0[0]).abs() <= 1.0, "{what}: left edge {l0:?} -> {l1:?}");
                            if gap {
                                assert!((l1[1] - l0[1]).abs() <= 1.0, "{what}: WTE {l0:?} -> {l1:?}");
                                let d = [r1[0] - r0[0], r1[1] - r0[1]];
                                assert!(d.iter().all(|d| (d - shift).abs() <= 1.0), "{what}: 2026 moved {d:?}, not {shift}");
                            }
                            assert!((moved - widen / 2.0).abs() < 0.01, "{what}: centre moved {moved} em");
                        }
                        TextAlign::Right => {
                            assert!((r1[1] - r0[1]).abs() <= 1.0, "{what}: right edge {r0:?} -> {r1:?}");
                            if gap {
                                assert!((r1[0] - r0[0]).abs() <= 1.0, "{what}: 2026 {r0:?} -> {r1:?}");
                                let d = [l1[0] - l0[0], l1[1] - l0[1]];
                                assert!(d.iter().all(|d| (d + shift).abs() <= 1.0), "{what}: WTE moved {d:?}, not -{shift}");
                            }
                            assert!((moved + widen / 2.0).abs() < 0.01, "{what}: centre moved {moved} em");
                        }
                        TextAlign::Center => {
                            let same = match (p0, placement_of(&doc, id)) {
                                (Placement::Projected(a), Placement::Projected(b)) => a.origin == b.origin,
                                (Placement::Uv(a), Placement::Uv(b)) => a.center_px == b.center_px,
                                _ => false,
                            };
                            assert!(same, "{what}: the centre is bit-equal");
                        }
                    }
                }
            }

            // With an Arc warp a spacing change keeps the anchor instead.
            let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
            let mut s = spec(&mut doc, "WTE 2026", TextAlign::Left);
            s.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Arc) });
            let id = place(&mut doc, s, kind, 30.0, em, false);
            let a0 = host(&mut doc, id, [0.0, 0.0]);
            let mut edited = spec_of(&doc, id);
            edited.fx.gaps = vec![Gap { after: 2, em: 0.5 }];
            doc.set_stamp_source(id, StampSource::Text(edited), Commit::Final).unwrap();
            let a1 = host(&mut doc, id, [0.0, 0.0]);
            assert!(a0.distance(a1) < 0.05, "{kind:?} Arc: anchor {a0} -> {a1}");
        }
    }

    /// TEXT 8.1 `a_spacing_edit_keeps_the_alignment_edge`, Letter spacing at 512 px per em, where one raster pixel is
    /// 0.002 em: letter spacing 0 -> 10 % widens the rectangle by 0.7 em within 1 % (the 64 px case above can only
    /// hold one raster pixel), the alignment edge of the ink stays within 1 texel, the em keeps its size.
    #[test]
    fn letter_spacing_widens_the_rectangle_within_one_percent() {
        let side = 2048;
        let em = 300.0;
        for align in [TextAlign::Left, TextAlign::Right, TextAlign::Center] {
            let mut doc = doc_from_raw(samples::flat_quad(), side);
            let s = spec(&mut doc, "WTE 2026", align);
            let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
            let placement = Placement::Uv(UvPlacement {
                set: TextureSetId(0),
                center_px: Vec2::splat(side as f32 / 2.0),
                size_px: Vec2::new(em * w as f32 / s.size_px, em * h as f32 / s.size_px),
                rotation: 0.0,
                flip_x: false,
                flip_y: false,
                clip_to_islands: false,
            });
            let id = doc.add_stamp("T", StampSource::Text(s), placement, TextureSetId(0)).unwrap();
            assert_eq!(spec_of(&doc, id).size_px, 512.0, "{align:?}");
            let axis = x_axis(&mut doc, id);
            let (em0, w0) = (em_host(&mut doc, id), rect_width(&doc, id));
            let (l0, r0) = ink_parts(&cache(&doc, id), axis);

            let mut edited = spec_of(&doc, id);
            edited.letter_spacing = 0.1;
            doc.set_stamp_source(id, StampSource::Text(edited), Commit::Final).unwrap();
            assert_eq!(spec_of(&doc, id).size_px, 512.0, "{align:?}: same raster resolution");
            let (em1, w1) = (em_host(&mut doc, id), rect_width(&doc, id));
            let (l1, r1) = ink_parts(&cache(&doc, id), axis);

            assert!(((em1 / em0) - 1.0).abs().max_element() < 0.001, "{align:?}: em {em0} -> {em1}");
            let widen = 0.7; // seven spaces between eight characters
            let added = (w1 - w0) / em0.x;
            assert!((added - widen).abs() <= 0.01 * widen, "{align:?}: wider by {added} em, not {widen}");
            match align {
                TextAlign::Left => assert!((l1[0] - l0[0]).abs() <= 1.0, "left edge {l0:?} -> {l1:?}"),
                TextAlign::Right => assert!((r1[1] - r0[1]).abs() <= 1.0, "right edge {r0:?} -> {r1:?}"),
                TextAlign::Center => {}
            }
        }
    }

    /// TEXT 8.1 `find_font_prefers_the_project`.
    #[test]
    fn find_font_prefers_the_project() {
        let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
        let x = doc.register_font("X Sans", "Regular", 0, Arc::from(epaint_default_fonts::HACK_REGULAR));
        // The same family and style from other bytes, registered later: the text uses it, so it is found first.
        let y = doc.register_font("x sans", "Regular", 0, Arc::from(default_font_bytes()));
        assert_ne!(x.blob, y.blob);
        let mut s = spec(&mut doc, "Hi", TextAlign::Left);
        s.font = y.clone();
        let id = place(&mut doc, s, Kind::Uv, 0.0, 40.0, false);
        // A font of a text stamp wins over an equal one handed out by `register_font` only; case does not matter.
        assert_eq!(doc.find_font("X SANS", "Regular"), Some(y.clone()));
        assert_eq!(doc.find_font("x sans", "Bold"), None, "the style must match");
        doc.remove_stamp(id).unwrap();
        assert_eq!(doc.find_font("x sans", "Regular"), Some(x.clone()), "then the session list, in its order");
        // A font registered but used by no stamp is found too.
        let unused = doc.register_font("Unused", "Light", 0, Arc::from(default_font_bytes()));
        assert_eq!(doc.find_font("unused", "Light"), Some(unused.clone()));
        // A font whose blob is gone is not (the session list keeps a reference to a missing blob here).
        doc.text_caches.font_refs.retain(|f| f.family != "Unused");
        doc.text_caches.font_refs.push(FontRef { blob: BlobId(4242), ..unused });
        assert_eq!(doc.find_font("unused", "Light"), None);
        // Neither a history entry nor a change of the document.
        let entries = doc.history.undo_len();
        let _ = doc.find_font("x sans", "Regular");
        assert_eq!(doc.history.undo_len(), entries);
    }

    /// A UV text at 512 px per em: one em is 300 texels of a 1024 px quad.
    fn big_uv_text(content: &str) -> (Document, StampId) {
        let mut doc = doc_from_raw(samples::flat_quad(), 1024);
        let s = spec(&mut doc, content, TextAlign::Left);
        let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
        let k = 300.0 / s.size_px;
        let placement = Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::splat(512.0),
            size_px: Vec2::new(w as f32 * k, h as f32 * k),
            rotation: 0.3,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        });
        let id = doc.add_stamp("T", StampSource::Text(s), placement, TextureSetId(0)).unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 512.0);
        (doc, id)
    }

    /// One em on the model (texels, per axis) and the host position of the anchor of a UV text.
    fn uv_em_and_anchor(doc: &mut Document, id: StampId) -> (Vec2, Vec2) {
        let frame = doc.stamp_text_frame(id).unwrap();
        let map = EmMap::new(&placement_of(doc, id), &frame);
        let a = map.to_texel([0.0, 0.0]);
        (Vec2::new(map.to_texel([1.0, 0.0]).distance(a), map.to_texel([0.0, 1.0]).distance(a)), a)
    }

    /// TEXT 8.1 `an_area_overflow_drops_the_resolution`.
    #[test]
    fn an_area_overflow_drops_the_resolution() {
        // "WTE 2026 WTE 2026 WTE 2026 WT" is one line of 16 em (P0's B3 case): 8031 px wide at 512 px per em.
        let (mut doc, id) = big_uv_text("WTE 2026 WTE 2026 WTE 2026 WT");
        let (em0, a0) = uv_em_and_anchor(&mut doc, id);
        let frame = doc.stamp_text_frame(id).unwrap();
        let mut circled = spec_of(&doc, id);
        circled.fx.path = Some(new_circle(&frame, circled.align));
        // Its default Circle does not fit 16.8 megapixels at 512 px per em.
        assert!(matches!(doc.stamp_source_size(&StampSource::Text(circled.clone())), Err(EditError::Text(TextError::TooLarge { .. }))));
        doc.set_stamp_source(id, StampSource::Text(circled), Commit::Final).unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 256.0);
        let (em1, a1) = uv_em_and_anchor(&mut doc, id);
        assert!(((em1 / em0) - 1.0).abs().max_element() < 0.01, "em {em0} -> {em1}");
        assert!(a0.distance(a1) < 0.5, "anchor {a0} -> {a1}");
        let size = doc.cached_text_frame(id).unwrap().size;
        assert!(size[0] as u64 * size[1] as u64 <= crate::text::MAX_STYLED_PIXELS, "{size:?}");
        assert!(!cache(&doc, id).is_empty());
        doc_state(&mut doc);
    }

    /// TEXT 8.1 `a_too_large_edit_drops_the_resolution`.
    #[test]
    fn a_too_large_edit_drops_the_resolution() {
        let (mut doc, id) = big_uv_text("WTE 2026");
        let (em0, a0) = uv_em_and_anchor(&mut doc, id);
        // About 40 em: 20,000 px at 512 and 10,000 at 256 px per em, beyond the 8192 px side; 5,000 px at 128.
        let mut long = spec_of(&doc, id);
        long.content = "WTE 2026 ".repeat(9);
        doc.set_stamp_source(id, StampSource::Text(long.clone()), Commit::Preview).unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 128.0, "a Preview takes the fallback too");
        doc.flush_pending_edit();
        assert_eq!(spec_of(&doc, id).size_px, 128.0, "the final bake keeps the size that fits");
        let (em1, a1) = uv_em_and_anchor(&mut doc, id);
        assert!(((em1 / em0) - 1.0).abs().max_element() < 0.01, "em {em0} -> {em1}");
        assert!(a0.distance(a1) < 0.5, "anchor {a0} -> {a1}");
        assert_eq!(doc.history.undo_len(), 2, "Add Text, Edit Stamp");
        // Undo brings back the old size and placement.
        doc.undo();
        assert_eq!(spec_of(&doc, id).size_px, 512.0);
        doc_state(&mut doc);
        // A text that does not fit even at 32 px per em is refused and changes nothing.
        let before = doc_state(&mut doc);
        let mut huge = spec_of(&doc, id);
        huge.content = "W".repeat(400);
        assert!(matches!(doc.set_stamp_source(id, StampSource::Text(huge), Commit::Final), Err(EditError::Text(_))));
        assert_eq!(doc_state(&mut doc), before);
    }

    /// TEXT 8.1 `anchor_stays_when_an_effect_is_switched_on`.
    #[test]
    fn anchor_stays_when_an_effect_is_switched_on() {
        type Effect = fn(&mut TextSpec, &TextFrame);
        let effects: [(&str, Effect); 4] = [
            ("Arc 50 %", |s, _| s.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Arc) })),
            ("Flag -100 %", |s, _| s.fx.warp = Some(Warp { bend: -1.0, ..Warp::new(WarpStyle::Flag) })),
            ("Circle", |s, f| s.fx.path = Some(new_circle(f, s.align))),
            ("thickening 2 %", |s, _| s.fx.bold_em = 0.02),
        ];
        // A projected text on the sphere, turned by 30 degrees and mirrored.
        let mut doc = doc_from_raw(samples::uv_sphere(64, 32, 1.0), 512);
        let diagonal = doc.mesh.bounds.diagonal();
        let s = spec(&mut doc, "WTE 2026", TextAlign::Left);
        let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
        let hy = 0.15 * h as f32 / TEXT_PLACEMENT_PX;
        let r = 30f32.to_radians();
        let mut p = Projector::from_surface(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::new(r.sin(), r.cos(), 0.0), Vec2::new(hy * w as f32 / h as f32, hy), 0.0);
        p.flip_x = true;
        let p = doc.fit_stamp_depth(&p);
        let id = doc.add_stamp("T", StampSource::Text(s), Placement::Projected(p), TextureSetId(0)).unwrap();
        let world = |doc: &mut Document, em: [f32; 2]| {
            let frame = doc.stamp_text_frame(id).unwrap();
            EmMap::new(&placement_of(doc, id), &frame).to_world(em)
        };
        let em_world = |doc: &mut Document| {
            let a = world(doc, [0.0, 0.0]);
            Vec2::new(world(doc, [1.0, 0.0]).distance(a), world(doc, [0.0, 1.0]).distance(a))
        };
        for (name, effect) in &effects {
            let (a0, e0) = (world(&mut doc, [0.0, 0.0]), em_world(&mut doc));
            let frame = doc.stamp_text_frame(id).unwrap();
            let mut s = spec_of(&doc, id);
            effect(&mut s, &frame);
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Final).unwrap();
            let (a1, e1) = (world(&mut doc, [0.0, 0.0]), em_world(&mut doc));
            assert!(a0.distance(a1) < 1e-4 * diagonal, "projected {name}: anchor {a0} -> {a1}");
            assert!(((e1 / e0) - 1.0).abs().max_element() < 0.001, "projected {name}: em {e0} -> {e1}");
            assert!(!cache(&doc, id).is_empty(), "projected {name}: baked");
            doc.undo();
        }

        // A UV text, turned and mirrored the same way.
        let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
        let s = spec(&mut doc, "WTE 2026", TextAlign::Left);
        let id = place(&mut doc, s, Kind::Uv, 30.0, 40.0, true);
        for (name, effect) in &effects {
            let (a0, e0) = (host(&mut doc, id, [0.0, 0.0]), em_host(&mut doc, id));
            let frame = doc.stamp_text_frame(id).unwrap();
            let mut s = spec_of(&doc, id);
            effect(&mut s, &frame);
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Final).unwrap();
            let (a1, e1) = (host(&mut doc, id, [0.0, 0.0]), em_host(&mut doc, id));
            assert!(a0.distance(a1) < 0.05, "UV {name}: anchor {a0} -> {a1}");
            assert!(((e1 / e0) - 1.0).abs().max_element() < 0.001, "UV {name}: em {e0} -> {e1}");
            doc.undo();
        }
    }

    /// TEXT 8.1 `editing_a_plain_text_gives_the_round_3_placement`: bit-equal to P0's round-3 constants
    /// (`target\round4\perf\text_baseline.txt`, section 2).
    #[test]
    fn editing_a_plain_text_gives_the_round_3_placement() {
        fn edit(doc: &mut Document, id: StampId) {
            let mut s = spec_of(doc, id);
            s.content = "WTE 2026!".to_owned();
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Final).unwrap();
        }
        let start = |doc: &mut Document| {
            let s = spec(doc, "WTE 2026", TextAlign::Left);
            let size = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
            (s, size)
        };
        // a) projected, depth 0.2 by hand, and b) the same with the automatic depth.
        for auto in [false, true] {
            let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
            let (s, [w, h]) = start(&mut doc);
            let hy = 0.4 * h as f32 / TEXT_PLACEMENT_PX / 2.0;
            let p = Projector::from_surface(Vec3::new(0.5, 0.5, 0.0), Vec3::Z, Vec3::Y, Vec2::new(hy * w as f32 / h as f32, hy), 0.2);
            let p = if auto { doc.fit_stamp_depth(&p) } else { p };
            let id = doc.add_stamp("T", StampSource::Text(s), Placement::Projected(p), TextureSetId(0)).unwrap();
            let Placement::Projected(added) = placement_of(&doc, id) else { panic!() };
            assert_eq!(added.half_size, Vec2::new(0.9093751, 0.23125), "auto {auto}: after add");
            edit(&mut doc, id);
            let Placement::Projected(a) = placement_of(&doc, id) else { panic!() };
            assert_eq!([a.half_size.x.to_bits(), a.half_size.y.to_bits()], [0x3f76cccf, 0x3e6ccccd], "auto {auto}");
            assert_eq!((a.origin, a.rotation), (added.origin, added.rotation), "auto {auto}");
            let depth = if auto { 0.096406266 } else { 0.2 };
            assert_eq!((a.depth_front, a.depth_back), (depth, depth), "auto {auto}");
            assert_eq!(spec_of(&doc, id).size_px, 128.0);
        }
        // c) UV, em 40 texels: add_stamp picks 64 px per em and refits.
        let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
        let (s, [w, h]) = start(&mut doc);
        let k = 40.0 / TEXT_PLACEMENT_PX;
        let placement = UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::new(128.0, 128.0),
            size_px: Vec2::new(w as f32 * k, h as f32 * k),
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        };
        let id = doc.add_stamp("T", StampSource::Text(s), Placement::Uv(placement), TextureSetId(0)).unwrap();
        let bits = |v: Vec2| [v.x.to_bits(), v.y.to_bits()];
        let Placement::Uv(added) = placement_of(&doc, id) else { panic!() };
        assert_eq!(spec_of(&doc, id).size_px, 64.0);
        assert_eq!(bits(added.center_px), [0x43000000, 0x43000000]);
        assert_eq!(bits(added.size_px), [0x43372000, 0x423e0000]);
        edit(&mut doc, id);
        let Placement::Uv(a) = placement_of(&doc, id) else { panic!() };
        assert_eq!(bits(a.center_px), [0x43000000, 0x43000000]);
        assert_eq!(bits(a.size_px), [0x43426000, 0x423e0000]);
        assert_eq!((a.rotation, spec_of(&doc, id).size_px), (0.0, 64.0));
    }

    /// TEXT 8.1 `typing_on_a_circle_does_not_move_the_circle`.
    #[test]
    fn typing_on_a_circle_does_not_move_the_circle() {
        for kind in [Kind::Projected, Kind::Uv] {
            let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
            let s = spec(&mut doc, "WTE", TextAlign::Center);
            let id = place(&mut doc, s, kind, 30.0, 20.0, false);
            let frame = doc.stamp_text_frame(id).unwrap();
            let mut s = spec_of(&doc, id);
            s.fx.path = Some(new_circle(&frame, s.align));
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Final).unwrap();
            let Some(TextPath { shape: PathShape::Circle { center, .. }, .. }) = spec_of(&doc, id).fx.path else { panic!() };
            let c0 = host(&mut doc, id, center);
            for letter in [" ", "2", "0", "2", "6"] {
                let mut s = spec_of(&doc, id);
                s.content.push_str(letter);
                doc.set_stamp_source(id, StampSource::Text(s), Commit::Preview).unwrap();
                let c = host(&mut doc, id, center);
                assert!(c0.distance(c) < 0.05, "{kind:?} after {letter:?}: circle centre {c0} -> {c}");
            }
            doc.flush_pending_edit();
            let c = host(&mut doc, id, center);
            assert!(c0.distance(c) < 0.05, "{kind:?} final: circle centre {c0} -> {c}");
        }
    }

    /// TEXT 8.1 `moving_a_curve_point_keeps_the_others`.
    #[test]
    fn moving_a_curve_point_keeps_the_others() {
        for kind in [Kind::Projected, Kind::Uv] {
            let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
            let s = spec(&mut doc, "WTE 2026", TextAlign::Center);
            let id = place(&mut doc, s, kind, 30.0, 20.0, true);
            let frame = doc.stamp_text_frame(id).unwrap();
            let mut s = spec_of(&doc, id);
            s.fx.path = Some(new_curve(&frame));
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Final).unwrap();
            let curve_points = |doc: &Document| match spec_of(doc, id).fx.path {
                Some(TextPath { shape: PathShape::Curve { points }, .. }) => points,
                _ => panic!("not a curve"),
            };
            let before: Vec<Vec2> = curve_points(&doc).into_iter().map(|p| host(&mut doc, id, p)).collect();
            let y0 = curve_points(&doc)[1][1];
            // Drag the middle point up by one em in three Preview steps and a Final.
            for (step, dy) in [(1, -0.3), (2, -0.6), (3, -1.0)] {
                let mut s = spec_of(&doc, id);
                let Some(TextPath { shape: PathShape::Curve { points }, .. }) = &mut s.fx.path else { panic!() };
                points[1][1] = y0 + dy;
                let commit = if step == 3 { Commit::Final } else { Commit::Preview };
                doc.set_stamp_source(id, StampSource::Text(s), commit).unwrap();
                let after: Vec<Vec2> = curve_points(&doc).into_iter().map(|p| host(&mut doc, id, p)).collect();
                for i in [0, 2] {
                    assert!(before[i].distance(after[i]) < 0.05, "{kind:?} step {step}: point {i} {} -> {}", before[i], after[i]);
                }
            }
        }
    }

    /// A projected styled text (Flag warp) on the flat quad, em 60 texels (the 64 px bucket).
    fn styled_doc() -> (Document, StampId) {
        let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
        let mut s = spec(&mut doc, "WTE 2026", TextAlign::Center);
        s.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Flag) });
        let id = place(&mut doc, s, Kind::Projected, 0.0, 60.0, false);
        (doc, id)
    }

    /// Pixels of the picture `text_mips` holds for a stamp, and whether it is a preview.
    fn picture(doc: &Document, id: StampId) -> ([u32; 2], bool) {
        (doc.text_mips[&id].1.size(), doc.text_caches.preview_ids.contains(&id))
    }

    /// TEXT 8.1 `previews_and_one_final_are_one_entry`: Preview edits of a styled text draw at most
    /// `TEXT_EDIT_PREVIEW_PIXELS`, keep the frame of the full picture, and end in one history entry with the full picture.
    #[test]
    fn previews_and_one_final_are_one_entry() {
        let (mut doc, id) = styled_doc();
        let entries = doc.history.undo_len();
        let full = doc.cached_text_frame(id).unwrap();
        assert_eq!(picture(&doc, id), (full.size, false), "add_stamp draws in full");
        // A styled text whose full picture is above 1 megapixel at its size: 512 px per em.
        let mut p = match placement_of(&doc, id) {
            Placement::Projected(p) => p,
            Placement::Uv(_) => unreachable!(),
        };
        p.half_size *= 8.0;
        doc.set_stamp_placement(id, Placement::Projected(p), Commit::Final).unwrap();
        assert_eq!(spec_of(&doc, id).size_px, 512.0);
        let entries_after_scale = doc.history.undo_len();
        assert_eq!(entries_after_scale, entries + 1);
        for (i, bend) in [0.3, 0.1, -0.2, -0.6].into_iter().enumerate() {
            let mut s = spec_of(&doc, id);
            s.fx.warp = Some(Warp { bend, ..Warp::new(WarpStyle::Flag) });
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Preview).unwrap();
            let frame = doc.cached_text_frame(id).unwrap();
            let ([w, h], preview) = picture(&doc, id);
            assert!(preview, "step {i}: a preview picture");
            assert!(w as u64 * h as u64 <= TEXT_EDIT_PREVIEW_PIXELS as u64, "step {i}: {w} x {h}");
            assert!((frame.size[0] as u64 * frame.size[1] as u64) > TEXT_EDIT_PREVIEW_PIXELS as u64, "step {i}: {:?}", frame.size);
            // The frame is the full picture's, whatever the preview resolution.
            assert_eq!(frame.size, doc.stamp_source_size(&StampSource::Text(spec_of(&doc, id))).unwrap(), "step {i}");
            assert_eq!(doc.history.undo_len(), entries_after_scale, "step {i}: still one pending edit");
        }
        let placement_before_final = placement_of(&doc, id);
        doc.flush_pending_edit();
        assert_eq!(doc.history.undo_len(), entries_after_scale + 1, "one entry for the whole drag");
        assert_eq!(doc.undo_label(), Some(EDIT_LABEL));
        let frame = doc.cached_text_frame(id).unwrap();
        assert_eq!(picture(&doc, id), (frame.size, false), "the final picture is full");
        assert_eq!(placement_of(&doc, id), placement_before_final, "nothing moves when the final picture replaces the preview");
        assert!(doc.preview_baked.is_empty());
        doc_state(&mut doc);
    }

    /// TEXT 8.1 `undo_restores_fx_placement_and_pixels`.
    #[test]
    fn undo_restores_fx_placement_and_pixels() {
        let (mut doc, id) = styled_doc();
        let before = doc_state(&mut doc);
        let (s0, p0, c0) = (spec_of(&doc, id), placement_of(&doc, id), cache(&doc, id));
        let mut s = s0.clone();
        s.fx.slant_deg = 12.0;
        s.fx.bold_em = 0.03;
        s.fx.path = Some(TextPath { shape: PathShape::Arc { bend: 0.4 }, start: 0.0, flip: false });
        doc.set_stamp_source(id, StampSource::Text(s.clone()), Commit::Final).unwrap();
        assert_ne!(placement_of(&doc, id), p0);
        assert!(*cache(&doc, id) != *c0);
        let edited = doc_state(&mut doc);
        doc.undo();
        assert_eq!(spec_of(&doc, id), s0);
        assert_eq!(placement_of(&doc, id), p0);
        assert!(*cache(&doc, id) == *c0, "the pixels come back");
        assert_eq!(doc_state(&mut doc), before);
        doc.redo();
        assert_eq!(spec_of(&doc, id).fx, s.fx);
        assert_eq!(doc_state(&mut doc), edited);
    }

    /// TEXT 8.1 `a_locked_text_refuses_fx`.
    #[test]
    fn a_locked_text_refuses_fx() {
        let (mut doc, id) = styled_doc();
        let layer = doc.stamp_layers(id)[0].1;
        doc.set_layer_locked(layer, true).unwrap();
        let before = doc_state(&mut doc);
        let mut s = spec_of(&doc, id);
        s.fx.bold_em = 0.04;
        for commit in [Commit::Preview, Commit::Final] {
            assert!(matches!(doc.set_stamp_source(id, StampSource::Text(s.clone()), commit), Err(EditError::LayerLocked)));
        }
        // The unchanged spec is still accepted as a no-op.
        doc.set_stamp_source(id, StampSource::Text(spec_of(&doc, id)), Commit::Final).unwrap();
        assert_eq!(doc_state(&mut doc), before);
    }

    /// TEXT 8.1 `the_frame_needs_no_pixels_after_undo_and_load`.
    #[test]
    fn the_frame_needs_no_pixels_after_undo_and_load() {
        // A document that can be saved and loaded (its model can be re-imported), with a styled text on the cube.
        let mut doc = Document::sample(crate::doc_build::SampleKind::Cube);
        let hit = doc.raycast(&crate::math::Ray::new(Vec3::new(0.05, 0.1, 10.0), Vec3::NEG_Z)).expect("the cube's front");
        let mut s = spec(&mut doc, "WTE 2026", TextAlign::Center);
        s.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Flag) });
        let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
        let hy = 0.1 * h as f32 / TEXT_PLACEMENT_PX;
        let p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, Vec2::new(hy * w as f32 / h as f32, hy), 0.0);
        let p = doc.fit_stamp_depth(&p);
        let id = doc.add_stamp("T", StampSource::Text(s), Placement::Projected(p), TextureSetId(0)).unwrap();
        let mut s = spec_of(&doc, id);
        s.fx.slant_deg = 10.0;
        doc.set_stamp_source(id, StampSource::Text(s), Commit::Final).unwrap();
        doc.undo();
        assert_eq!(doc.cached_text_frame(id), None, "after undo");
        let count = raster_counter::get();
        let frame = doc.stamp_text_frame(id).unwrap();
        assert_eq!(doc.cached_text_frame(id), Some(frame));
        assert_eq!(raster_counter::get(), count, "no pixels for the frame after undo");

        // After a load.
        let dir = temp_dir("t4_frame_after_load");
        let path = dir.join("frame.wtep");
        let opts = crate::project::SaveOptions {
            png: PngLevel::Fast,
            write_flattened: false,
            keep_backup: false,
            now_unix: 1,
            app_version: "test".to_owned(),
        };
        crate::project::save(&doc.snapshot(), &path, &opts, &NoProgress).unwrap();
        let mut loaded = crate::project::load(&path, &NoProgress).unwrap().document;
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded.cached_text_frame(id), None, "after load");
        let count = raster_counter::get();
        assert_eq!(loaded.stamp_text_frame(id).unwrap(), frame);
        assert_eq!(raster_counter::get(), count, "no pixels for the frame after load");
        // `text_raster_px` reads the frame, not a picture.
        assert_eq!(loaded.text_raster_px(id), doc.text_raster_px(id));
        assert_eq!(raster_counter::get(), count);
    }

    /// TEXT 8.1 `a_full_bake_never_uses_a_preview_picture`.
    #[test]
    fn a_full_bake_never_uses_a_preview_picture() {
        // The reference: the same edit made Final at once.
        let (mut reference, rid) = styled_doc();
        let (mut doc, id) = styled_doc();
        for (d, sid) in [(&mut reference, rid), (&mut doc, id)] {
            let Placement::Projected(mut p) = placement_of(d, sid) else { panic!() };
            p.half_size *= 8.0;
            d.set_stamp_placement(sid, Placement::Projected(p), Commit::Final).unwrap();
        }
        let mut s = spec_of(&doc, id);
        s.fx.warp = Some(Warp { bend: -0.4, ..Warp::new(WarpStyle::Flag) });
        reference.set_stamp_source(rid, StampSource::Text(s.clone()), Commit::Final).unwrap();

        // Preview edits bake the preview picture right away (bakes not deferred).
        doc.set_stamp_source(id, StampSource::Text(s), Commit::Preview).unwrap();
        assert!(picture(&doc, id).1, "a preview picture");
        let preview_cache = cache(&doc, id);
        // A Full re-bake of the stamp (as after a lost cache) draws the full picture first.
        doc.rebake_stamp(id, BakeQuality::Full).unwrap();
        assert!(!picture(&doc, id).1, "the Full bake replaced the preview picture");
        assert!(*cache(&doc, id) == *cache(&reference, rid), "Full bake = the Final edit's bake");
        assert!(*preview_cache != *cache(&doc, id));
        // A Preview bake may use the full picture.
        doc.rebake_stamp(id, BakeQuality::Preview).unwrap();
        assert!(!picture(&doc, id).1);
        doc.flush_pending_edit();
        assert!(*cache(&doc, id) == *cache(&reference, rid));
        assert_eq!(placement_of(&doc, id), placement_of(&reference, rid));
    }

    /// Duplicates share the picture and know whether it is a preview; the frame cache follows the spec.
    #[test]
    fn duplicates_and_removed_stamps_keep_the_caches_consistent() {
        let (mut doc, id) = styled_doc();
        let Placement::Projected(mut p) = placement_of(&doc, id) else { panic!() };
        p.half_size *= 8.0;
        doc.set_stamp_placement(id, Placement::Projected(p), Commit::Final).unwrap();
        let mut s = spec_of(&doc, id);
        s.fx.bold_em = 0.01;
        doc.set_stamp_source(id, StampSource::Text(s), Commit::Preview).unwrap();
        // `duplicate_stamp` flushes the pending edit first: the original is full again, and so is the copy.
        let copy = doc.duplicate_stamp(id).unwrap();
        assert_eq!(picture(&doc, copy), picture(&doc, id));
        assert!(!picture(&doc, copy).1);
        assert_eq!(doc.cached_text_frame(copy), doc.cached_text_frame(id));
        doc.remove_stamp(copy).unwrap();
        assert!(!doc.text_mips.contains_key(&copy) && !doc.text_caches.frames.contains_key(&copy));
        assert!(!doc.text_caches.preview_ids.contains(&copy));
        doc_state(&mut doc);
    }

    /// `stamp_source_frame` applies the too-large fallback, `stamp_source_size` does not; both sanitize.
    #[test]
    fn source_frames_sanitize_and_fall_back() {
        let mut doc = doc_from_raw(samples::flat_quad(), SIDE);
        let mut s = spec(&mut doc, "WTE 2026 WTE 2026 WTE 2026 WT", TextAlign::Left);
        s.size_px = 512.0;
        let frame = doc.stamp_source_frame(&StampSource::Text(s.clone())).unwrap();
        assert_eq!(frame.em_px, 512.0);
        let straight = frame;
        s.fx.path = Some(new_circle(&straight, s.align));
        let frame = doc.stamp_source_frame(&StampSource::Text(s.clone())).unwrap();
        assert_eq!(frame.em_px, 256.0, "the fallback of add_stamp");
        assert!(doc.stamp_source_size(&StampSource::Text(s.clone())).is_err());
        // A hostile spec is sanitized first.
        s.size_px = 64.0;
        s.fx.bold_em = f32::NAN;
        s.fx.slant_deg = 1e9;
        s.fx.gaps = vec![Gap { after: 900, em: f32::INFINITY }];
        let frame = doc.stamp_source_frame(&StampSource::Text(s.clone())).unwrap();
        assert_eq!(frame.size, doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap());
        let raster = doc.text_preview(&s, 1 << 16).unwrap();
        assert_eq!(raster.frame, frame);
        assert!(raster.image.width as u64 * raster.image.height as u64 <= 1 << 16);
        assert!(Arc::ptr_eq(&raster, &doc.text_preview(&s, 1 << 16).unwrap()), "one-entry cache");
        let id = doc.add_stamp("T", StampSource::Text(s), Placement::Uv(UvPlacement {
            set: TextureSetId(0),
            center_px: Vec2::splat(128.0),
            size_px: Vec2::new(frame.size[0] as f32, frame.size[1] as f32) / 4.0,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        }), TextureSetId(0)).unwrap();
        let stored = spec_of(&doc, id);
        assert!(stored.fx.bold_em.is_finite() && stored.fx.slant_deg.abs() <= crate::text::MAX_SLANT_DEG);
        assert!(stored.fx.gaps.is_empty());
    }

    /// TEXT 8.4: `set_stamp_source(.., Preview)` with deferred bakes, a styled text of 40 letters at 256 px per em,
    /// no outline, on the sample sphere: at most 16 ms (release build, idle machine). Run with
    /// `cargo test -p wte_core --release --lib -- --ignored --nocapture text_document_bench`.
    #[test]
    #[ignore = "timing probe"]
    fn text_document_bench() {
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
        let median = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let mut doc = doc_from_raw(samples::uv_sphere(64, 32, 1.0), 2048);
        doc.set_defer_preview_bakes(true);
        let content = "ABCDEFGHIJ abcdefghij KLMNOPQRST 0123456789";
        assert_eq!(content.chars().filter(|c| !c.is_whitespace()).count(), 40);
        let mut s = spec(&mut doc, content, TextAlign::Center);
        s.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Flag) });
        let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
        // About 256 texels per em on the 2048 texture of the sphere.
        let hy = 0.5 * h as f32 / TEXT_PLACEMENT_PX;
        let p = Projector::from_surface(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::Y, Vec2::new(hy * w as f32 / h as f32, hy), 0.0);
        let p = doc.fit_stamp_depth(&p);
        let id = doc.add_stamp("T", StampSource::Text(s), Placement::Projected(p), TextureSetId(0)).unwrap();
        eprintln!("size_px {}", spec_of(&doc, id).size_px);
        let mut times = Vec::new();
        for i in 0..11 {
            let mut s = spec_of(&doc, id);
            s.fx.warp = Some(Warp { bend: 0.3 + 0.02 * i as f32, ..Warp::new(WarpStyle::Flag) });
            let t = std::time::Instant::now();
            doc.set_stamp_source(id, StampSource::Text(s), Commit::Preview).unwrap();
            times.push(ms(t));
        }
        eprintln!("set_stamp_source Preview, 40 letters Flag at {} px per em: median {:.2} ms (limit 16), all {times:.2?}", spec_of(&doc, id).size_px, median(times.clone()));
        let t = std::time::Instant::now();
        doc.flush_pending_edit();
        eprintln!("flush (final picture and Full bake): {:.1} ms", ms(t));

        // The two frames of the too-large fallback: a line of 16 em on its default Circle at 512 and 256 px per em.
        let mut s = spec(&mut doc, "WTE 2026 WTE 2026 WTE 2026 WT", TextAlign::Left);
        s.size_px = 512.0;
        let straight = doc.stamp_source_frame(&StampSource::Text(s.clone())).unwrap();
        s.fx.path = Some(new_circle(&straight, s.align));
        let mut times = Vec::new();
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let frame = doc.stamp_source_frame(&StampSource::Text(s.clone())).unwrap();
            times.push(ms(t));
            assert_eq!(frame.em_px, 256.0);
        }
        eprintln!("too-large fallback frames (512 refused, 256 taken): median {:.3} ms (limit 1)", median(times));
    }

    /// Gate stage 3 (round 4): the frame of `fuzz_path` seed 3 on ngons_tiling (action 151, 4.9 s in the app): a long
    /// text gets a Circle path of 15.29 em. Times the document's part (Preview with deferred bakes, the flush that
    /// bakes the Final picture, the Final) on ngons_tiling, then an image stamp with the same projector as the text on
    /// its Circle, to show what the bake of that footprint costs without any text code. Run with
    /// `cargo.cmd test -p wte_core --release --lib gate_fuzz_circle_text_on_ngons -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe on a local model file"]
    fn gate_fuzz_circle_text_on_ngons() {
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
        let path = std::path::PathBuf::from(r"E:\claude_programs\wrapped_texture_editor\target\compat\harness\shapes\ngons_tiling.obj");
        if !path.exists() {
            eprintln!("SKIP: no {}", path.display());
            return;
        }
        let model = crate::import::import_from_path(&path, &Default::default(), &crate::progress::NoProgress).unwrap();
        let n = model.mesh.sets.len();
        let mut doc =
            Document::from_import(model, vec![crate::doc_build::TextureChoice::Detected; n], &crate::progress::NoProgress).unwrap();
        doc.recomposite_all();
        doc.set_defer_preview_bakes(true);
        let (c, d, b) = (doc.mesh.bounds.center(), doc.mesh.diag(), doc.mesh.bounds);
        // The first hit of rays along -Z, -Y and -X through points near the middle (as `stamp_bench_tiled_model`).
        let mut hits = Vec::new();
        for (fx, fy) in [(0.5f32, 0.5f32), (0.3, 0.6), (0.7, 0.4), (0.45, 0.3), (0.6, 0.7)] {
            let at = |i: usize, f: f32| b.min[i] + f * (b.max[i] - b.min[i]);
            for ray in [
                crate::math::Ray::new(Vec3::new(at(0, fx), at(1, fy), c.z + d), -Vec3::Z),
                crate::math::Ray::new(Vec3::new(at(0, fx), c.y + d, at(2, fy)), -Vec3::Y),
                crate::math::Ray::new(Vec3::new(c.x + d, at(1, fx), at(2, fy)), -Vec3::X),
            ] {
                hits.extend(doc.raycast(&ray));
            }
        }
        let hit = hits.into_iter().next().expect("a hit near the middle");
        let content = "A much longer text stamp line for wrapping";
        let s = spec(&mut doc, content, TextAlign::Left);
        let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
        for width_frac in [0.4f32, 1.0] {
            // The straight line spans `width_frac` of the model's diagonal.
            let hx = 0.5 * width_frac * doc.mesh.bounds.diagonal();
            let half = Vec2::new(hx, hx * h as f32 / w as f32);
            let mut p = Projector::from_surface(hit.position, hit.normal, Vec3::Y, half, half.max_element());
            p.backface_cull = hit.front_face;
            let p = doc.fit_stamp_depth(&p);
            let t = std::time::Instant::now();
            let id = doc.add_stamp("T", StampSource::Text(s.clone()), Placement::Projected(p), TextureSetId(0)).unwrap();
            let add = ms(t);
            let mut circle = spec_of(&doc, id);
            circle.fx.path = Some(TextPath {
                shape: PathShape::Circle { diameter: 15.288544, center: [-0.957_658, -1.643_390_2] },
                start: 0.0,
                flip: false,
            });
            let t = std::time::Instant::now();
            doc.set_stamp_source(id, StampSource::Text(circle.clone()), Commit::Preview).unwrap();
            let preview = ms(t);
            crate::stamp_bake::probe::take();
            let t = std::time::Instant::now();
            doc.flush_pending_edit();
            let flush = ms(t);
            let bake = crate::stamp_bake::probe::take();
            let t = std::time::Instant::now();
            doc.set_stamp_source(id, StampSource::Text(circle.clone()), Commit::Final).unwrap();
            let fin = ms(t);
            let t = std::time::Instant::now();
            doc.recomposite_all();
            let recomposite = ms(t);
            let placed = match doc.stamp(id).unwrap().placement {
                Placement::Projected(q) => q,
                Placement::Uv(_) => unreachable!(),
            };
            eprintln!(
                "line {width_frac} of the diagonal: add {add:.1} ms; Circle: Preview {preview:.1} ms, flush {flush:.1} ms, Final {fin:.1} ms, \
                 recomposite {recomposite:.1} ms; size_px {}, half_size {:?} (straight {:?}), diagonal {:.3}",
                spec_of(&doc, id).size_px,
                placed.half_size,
                half,
                doc.mesh.bounds.diagonal()
            );
            let parts = |s: [u64; 12]| format!("candidates {:.1} ms, raster {:.1} ms, crop {:.1} ms, jobs {}", s[0] as f64 / 1e6, s[1] as f64 / 1e6, s[2] as f64 / 1e6, s[7]);
            eprintln!("  the flush's bakes: {}", parts(bake));
            // An image stamp with the projector of the text on its Circle (round-3 code path: image stamps).
            let blob = doc.add_image_blob("stamp.png", {
                let img = samples::sample_stamp_image(256);
                Arc::from(crate::image_io::encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap())
            }).unwrap();
            crate::stamp_bake::probe::take();
            let t = std::time::Instant::now();
            let image = doc.add_stamp("I", StampSource::Image { blob }, Placement::Projected(placed), TextureSetId(0)).unwrap();
            let image_add = ms(t);
            eprintln!("  the image stamp's bake: {}", parts(crate::stamp_bake::probe::take()));
            let t = std::time::Instant::now();
            doc.recomposite_all();
            eprintln!("  image stamp with the same projector: add (Full bake) {image_add:.1} ms, recomposite {:.1} ms", ms(t));
            doc.remove_stamp(image).unwrap();
            // The same with a large image (as large as the text picture): the round-3 path with a large source.
            let big = {
                let img = samples::sample_stamp_image(4000);
                Arc::from(crate::image_io::encode_png(img.width, img.height, &img.data, PngLevel::Fast).unwrap())
            };
            let blob = doc.add_image_blob("big.png", big).unwrap();
            crate::stamp_bake::probe::take();
            let t = std::time::Instant::now();
            let image = doc.add_stamp("B", StampSource::Image { blob }, Placement::Projected(placed), TextureSetId(0)).unwrap();
            let big_add = ms(t);
            eprintln!("  a 4000 x 4000 image stamp with the same projector: add (Full bake) {big_add:.1} ms; {}", parts(crate::stamp_bake::probe::take()));
            doc.remove_stamp(image).unwrap();
            // A plain text (no round-4 setting: the round-3 painter and bake) moved onto the same projector.
            let plain = doc.add_stamp("P", StampSource::Text(s.clone()), Placement::Projected(p), TextureSetId(0)).unwrap();
            crate::stamp_bake::probe::take();
            let t = std::time::Instant::now();
            doc.set_stamp_placement(plain, Placement::Projected(placed), Commit::Final).unwrap();
            doc.flush_pending_edit();
            eprintln!("  the plain text moved onto the same projector: {:.1} ms; {}", ms(t), parts(crate::stamp_bake::probe::take()));
            doc.remove_stamp(plain).unwrap();
            doc.remove_stamp(id).unwrap();
        }
    }

    /// TEXT 8.4 / plan R4-16: twenty `Preview` edits of a text on an 8192 texture, each followed by the deferred bake
    /// as the app's frame does it (`set_stamp_source(.., Preview)` + `bake_stale_previews`); every edit changes the
    /// content ("WTE 2027" .. "WTE 2046"). The styled text (Arc warp) must have a longest frame of at most 30 ms plus
    /// the round-3 preview-bake time: the median frame of the same twenty plain edits measured on the round-3 tree
    /// (`R4\runs_t4\r3probe`, test `round3_twenty_edits_bench`), passed in milliseconds as `WTE_R3_PREVIEW_MS`.
    /// Without it the median plain frame of this build stands in (printed as such). Asserts the limit. Needs about
    /// 1.5 GB of free memory. Run with
    /// `WTE_R3_PREVIEW_MS=<ms> cargo test -p wte_core --release --lib -- --ignored --nocapture text_document_twenty_edits_bench`.
    #[test]
    #[ignore = "timing probe"]
    fn text_document_twenty_edits_bench() {
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
        let stats = |v: &[f64]| {
            let mut v = v.to_vec();
            v.sort_by(f64::total_cmp);
            (v[v.len() / 2], v[(v.len() * 9).div_ceil(10) - 1], v[v.len() - 1])
        };
        let mut doc = doc_from_raw(samples::uv_sphere(64, 32, 1.0), 8192);
        doc.set_defer_preview_bakes(true);
        let run = |doc: &mut Document, styled: bool| -> Vec<f64> {
            let mut s = spec(doc, "WTE 2026", TextAlign::Center);
            if styled {
                s.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Arc) });
            }
            let [w, h] = doc.stamp_source_size(&StampSource::Text(s.clone())).unwrap();
            let hy = 0.25;
            let p = Projector::from_surface(Vec3::new(0.0, 0.0, 1.0), Vec3::Z, Vec3::Y, Vec2::new(hy * w as f32 / h as f32, hy), 0.0);
            let p = doc.fit_stamp_depth(&p);
            let id = doc.add_stamp("T", StampSource::Text(s), Placement::Projected(p), TextureSetId(0)).unwrap();
            let mut times = Vec::new();
            for i in 1..=20 {
                let mut s = spec_of(doc, id);
                s.content = format!("WTE {}", 2026 + i);
                if styled {
                    s.fx.warp = Some(Warp { bend: 0.3 + 0.02 * i as f32, ..Warp::new(WarpStyle::Arc) });
                }
                let t = std::time::Instant::now();
                doc.set_stamp_source(id, StampSource::Text(s), Commit::Preview).unwrap();
                doc.bake_stale_previews();
                times.push(ms(t));
            }
            let t = std::time::Instant::now();
            doc.flush_pending_edit();
            let flush = ms(t);
            let (median, p90, max) = stats(&times);
            eprintln!(
                "{} text, 20 Preview edits + deferred bakes on 8192, {} px per em: median {median:.2} ms, p90 {p90:.2} ms, longest {max:.2} ms, all {times:.2?}; flush {flush:.1} ms",
                if styled { "styled (Arc 50 %)" } else { "plain" },
                spec_of(doc, id).size_px,
            );
            times
        };
        let plain = run(&mut doc, false);
        let styled = run(&mut doc, true);
        let (plain_median, _, _) = stats(&plain);
        let (_, _, styled_max) = stats(&styled);
        let (baseline, from) = match std::env::var("WTE_R3_PREVIEW_MS").ok().and_then(|v| v.parse::<f64>().ok()) {
            Some(r3) => {
                eprintln!("plain text here: median {plain_median:.2} ms against round 3's {r3:.2} ms");
                (r3, "round 3 (WTE_R3_PREVIEW_MS)")
            }
            None => (plain_median, "this build's plain text: WTE_R3_PREVIEW_MS not set, not a round-3 measurement"),
        };
        let limit = 30.0 + baseline;
        eprintln!("longest styled frame {styled_max:.2} ms, limit 30 + {baseline:.2} ms [{from}] = {limit:.2} ms");
        assert!(styled_max <= limit, "longest styled frame {styled_max:.2} ms over the limit {limit:.2} ms");
    }
}

/// Round 5 (R5-1): deferred Final bakes (ARCHITECTURE.md 5.20 "Deferred Final bakes", plan 1.7 items 1-6).
#[cfg(test)]
mod deferred_final_bake_tests {
    use glam::{Vec2, Vec3};

    use super::*;
    use crate::brush::BrushParams;
    use crate::history::testkit::{Bytes, doc_state};
    use crate::image_io::{PngLevel, encode_png};
    use crate::samples;
    use crate::stamp::{Projector, UvPlacement};
    use crate::testutil::doc_from_raw;
    use crate::text::path::new_circle;
    use crate::text::{TextAlign, TextFx, Warp, WarpStyle};

    /// Bakes every waiting stamp the way the app does (`next_final_bake` -> `run` -> `apply_final_bake`), on this
    /// thread. Returns the number of applied bakes.
    fn pump(doc: &mut Document) -> usize {
        let mut applied = 0;
        while let Some(job) = doc.next_final_bake() {
            match doc.apply_final_bake(job.run()) {
                FinalBake::Applied { .. } => applied += 1,
                other => panic!("a pumped bake was not applied: {other:?}"),
            }
        }
        assert!(!doc.has_unbaked_stamps());
        applied
    }

    /// One layer as the comparison sees it: everything but its id and version.
    type LayerLook = (String, &'static str, Option<StampId>, bool, u32, bool, Bytes);

    /// Stamps, per set the layers (without ids) and the index of the active layer, and the composites.
    #[derive(Debug, PartialEq)]
    struct Look {
        stamps: Vec<StampObject>,
        sets: Vec<(Vec<LayerLook>, Option<usize>)>,
        composites: Vec<Bytes>,
    }

    fn look(doc: &mut Document) -> Look {
        doc.recomposite_all();
        doc.debug_check_structure();
        let sets = doc
            .sets
            .iter()
            .map(|set| {
                let layers = set
                    .layers
                    .iter()
                    .map(|l| {
                        let pixels = Bytes(l.to_dense(set.width, set.height).data);
                        (l.name.clone(), l.kind_name(), l.stamp_id(), l.visible, l.opacity.to_bits(), l.locked, pixels)
                    })
                    .collect();
                (layers, set.active.and_then(|a| set.layer_index(a)))
            })
            .collect();
        let composites = doc.sets.iter().map(|s| Bytes(s.composite.data.clone())).collect();
        Look { stamps: doc.stamps.clone(), sets, composites }
    }

    fn png(image: &PixelBuf) -> Arc<[u8]> {
        Arc::from(encode_png(image.width, image.height, &image.data, PngLevel::Fast).unwrap())
    }

    /// Projector facing -Z onto the plane z = `z`, centred at (cx, cy), image up = +Y.
    fn front(cx: f32, cy: f32, z: f32, half: Vec2, depth: f32) -> Projector {
        Projector::from_surface(Vec3::new(cx, cy, z), Vec3::Z, Vec3::Y, half, depth)
    }

    fn text(doc: &mut Document, content: &str) -> TextSpec {
        TextSpec {
            content: content.to_owned(),
            font: doc.default_font(),
            size_px: TEXT_PLACEMENT_PX,
            color: [250, 240, 20, 255],
            align: TextAlign::Center,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            outline: None,
            fx: TextFx::default(),
        }
    }

    /// A projected placement of `spec` on the front of the two-material model, one em `em` world units high.
    fn text_placement(doc: &mut Document, spec: &TextSpec, cy: f32, em: f32) -> Placement {
        let [w, h] = doc.stamp_source_size(&StampSource::Text(spec.clone())).unwrap();
        let hy = em * h as f32 / spec.size_px / 2.0;
        Placement::Projected(front(0.0, cy, 0.5, Vec2::new(hy * w as f32 / h as f32, hy), 0.6))
    }

    fn scaled(placement: Placement, f: f32) -> Placement {
        match placement {
            Placement::Projected(mut p) => {
                p.half_size *= f;
                Placement::Projected(p)
            }
            Placement::Uv(mut u) => {
                u.size_px *= f;
                Placement::Uv(u)
            }
        }
    }

    fn moved(placement: Placement, d: Vec2) -> Placement {
        match placement {
            Placement::Projected(mut p) => {
                p.origin += Vec3::new(d.x, d.y, 0.0);
                Placement::Projected(p)
            }
            Placement::Uv(mut u) => {
                u.center_px += d * 100.0;
                Placement::Uv(u)
            }
        }
    }

    fn placement_of(doc: &Document, id: StampId) -> Placement {
        doc.stamp(id).unwrap().placement
    }

    fn spec_of(doc: &Document, id: StampId) -> TextSpec {
        match &doc.stamp(id).unwrap().source {
            StampSource::Text(spec) => spec.clone(),
            StampSource::Image { .. } => panic!("not a text stamp"),
        }
    }

    fn two_set_doc(deferred: bool) -> (Document, BlobId) {
        let mut doc = doc_from_raw(samples::two_material_model(), 128);
        doc.set_defer_final_bakes(deferred);
        let blob = doc.add_image_blob("stamp.png", png(&samples::sample_stamp_image(64))).unwrap();
        (doc, blob)
    }

    /// The edit list of item 1: every kind of stamp, every kind of Final, Preview drags, duplicate, undo, redo,
    /// delete. `pump` after every command; returns the look after every command.
    fn run_edits(doc: &mut Document, blob: BlobId) -> Vec<Look> {
        let mut looks = Vec::new();
        let mut step = |doc: &mut Document| {
            pump(doc);
            looks.push(look(doc));
        };
        let image = StampSource::Image { blob };
        // An image stamp projected over both sets (cube front and head), a drag and a tint.
        let p0 = Placement::Projected(front(0.0, 0.5, 0.5, Vec2::splat(0.4), 0.6));
        let img = doc.add_stamp("Img", image.clone(), p0, TextureSetId(0)).unwrap();
        step(doc);
        doc.set_stamp_placement(img, moved(p0, Vec2::new(0.05, 0.0)), Commit::Preview).unwrap();
        doc.set_stamp_placement(img, moved(p0, Vec2::new(0.1, -0.05)), Commit::Preview).unwrap();
        doc.set_stamp_placement(img, moved(p0, Vec2::new(0.1, -0.05)), Commit::Final).unwrap();
        step(doc);
        doc.set_stamp_tint(img, [200, 120, 60, 230], Commit::Final).unwrap();
        step(doc);
        // A UV stamp in set 1, moved.
        let uv = UvPlacement {
            set: TextureSetId(1),
            center_px: Vec2::new(60.0, 40.0),
            size_px: Vec2::new(50.0, 30.0),
            rotation: 0.3,
            flip_x: false,
            flip_y: false,
            clip_to_islands: false,
        };
        let uvs = doc.add_stamp("Uv", image.clone(), Placement::Uv(uv), TextureSetId(1)).unwrap();
        step(doc);
        doc.set_stamp_placement(uvs, moved(Placement::Uv(uv), Vec2::new(0.1, 0.2)), Commit::Final).unwrap();
        step(doc);
        // A plain text, its content edited.
        let plain = text(doc, "Plain");
        let at = text_placement(doc, &plain, 0.2, 0.12);
        let plain_id = doc.add_stamp("Plain", StampSource::Text(plain.clone()), at, TextureSetId(0)).unwrap();
        step(doc);
        let mut longer = plain;
        longer.content = "Plain text".to_owned();
        doc.set_stamp_source(plain_id, StampSource::Text(longer), Commit::Final).unwrap();
        step(doc);
        // A styled text: Arc warp and thickening, then Preview edits and a Final with a Circle path, then a scale
        // (the raster resolution changes: `retune_text`).
        let mut styled = text(doc, "Styled 26");
        styled.fx.warp = Some(Warp { bend: 0.5, ..Warp::new(WarpStyle::Arc) });
        styled.fx.bold_em = 0.04;
        let at = text_placement(doc, &styled, 0.45, 0.15);
        let sty = doc.add_stamp("Styled", StampSource::Text(styled), at, TextureSetId(0)).unwrap();
        step(doc);
        let mut spaced = spec_of(doc, sty);
        spaced.letter_spacing = 0.1;
        doc.set_stamp_source(sty, StampSource::Text(spaced.clone()), Commit::Preview).unwrap();
        spaced.letter_spacing = 0.2;
        doc.set_stamp_source(sty, StampSource::Text(spaced.clone()), Commit::Preview).unwrap();
        let frame = doc.stamp_text_frame(sty).unwrap();
        spaced.fx.warp = None;
        spaced.fx.path = Some(new_circle(&frame, TextAlign::Center));
        doc.set_stamp_source(sty, StampSource::Text(spaced), Commit::Final).unwrap();
        step(doc);
        let at = placement_of(doc, sty);
        doc.set_stamp_placement(sty, scaled(at, 2.5), Commit::Final).unwrap();
        step(doc);
        // Duplicate, undo, redo, delete and undo.
        doc.duplicate_stamp(img).unwrap();
        step(doc);
        assert_eq!(doc.undo().as_deref(), Some("Duplicate Stamp"));
        step(doc);
        assert_eq!(doc.undo().as_deref(), Some("Edit Stamp"));
        step(doc);
        assert_eq!(doc.redo().as_deref(), Some("Edit Stamp"));
        step(doc);
        doc.remove_stamp(uvs).unwrap();
        step(doc);
        doc.undo();
        step(doc);
        // Round 6 (WA12): a wrapped image (a Preview drag and a Final) and a wrapped styled text (Arc warp), toggled
        // to Project and back.
        let wp = Projector { mode: SurfaceMode::Wrap, ..front(0.1, 0.25, 0.5, Vec2::new(0.45, 0.2), 0.6) };
        let wimg = doc.add_stamp("Wrapped", image.clone(), Placement::Projected(wp), TextureSetId(0)).unwrap();
        step(doc);
        doc.set_stamp_placement(wimg, moved(Placement::Projected(wp), Vec2::new(0.05, 0.0)), Commit::Preview).unwrap();
        doc.set_stamp_placement(wimg, moved(Placement::Projected(wp), Vec2::new(0.1, 0.02)), Commit::Final).unwrap();
        step(doc);
        let mut wstyled = text(doc, "Wrap 26");
        wstyled.fx.warp = Some(Warp { bend: 0.3, ..Warp::new(WarpStyle::Arc) });
        let at = match text_placement(doc, &wstyled, -0.2, 0.15) {
            Placement::Projected(p) => Placement::Projected(Projector { mode: SurfaceMode::Wrap, ..p }),
            other => other,
        };
        let wtext = doc.add_stamp("WrapText", StampSource::Text(wstyled), at, TextureSetId(0)).unwrap();
        step(doc);
        if let Placement::Projected(p) = placement_of(doc, wtext) {
            doc.set_stamp_placement(wtext, Placement::Projected(Projector { mode: SurfaceMode::Project, ..p }), Commit::Final).unwrap();
            step(doc);
            doc.set_stamp_placement(wtext, Placement::Projected(p), Commit::Final).unwrap();
            step(doc);
        }
        looks
    }

    /// Plan 1.7 item 1: the same edits with deferred Final bakes (pumped after every command) give the same stamps,
    /// layer order, names, kinds and caches byte for byte as the synchronous bakes (layer ids may differ).
    #[test]
    fn deferred_final_bakes_equal_the_synchronous_ones() {
        let (mut sync, blob) = two_set_doc(false);
        let (mut deferred, blob2) = two_set_doc(true);
        assert_eq!(blob, blob2);
        let a = run_edits(&mut sync, blob);
        let b = run_edits(&mut deferred, blob);
        assert_eq!(a.len(), b.len());
        for (i, (a, b)) in a.iter().zip(&b).enumerate() {
            assert_eq!(a, b, "the documents differ after command {i}");
        }
        // The edits reached both sets and drew real pixels.
        let last = &a[a.len() - 1];
        assert!(last.sets.iter().all(|(layers, _)| layers.iter().any(|l| l.2.is_some())));
        assert_eq!(sync.history.undo_len(), deferred.history.undo_len());
        assert!(!deferred.has_unbaked_stamps() && deferred.bakes.unbaked.is_empty());
    }

    /// A document in deferred mode with an image stamp placed and baked, and that stamp's id.
    fn placed() -> (Document, StampId) {
        let (mut doc, blob) = two_set_doc(true);
        let p = Placement::Projected(front(0.0, 0.5, 0.5, Vec2::splat(0.3), 0.6));
        let id = doc.add_stamp("S", StampSource::Image { blob }, p, TextureSetId(0)).unwrap();
        assert_eq!(pump(&mut doc), 1);
        (doc, id)
    }

    /// Plan 1.7 item 2: a job taken before the stamp is moved, removed, or its edit undone gives `Stale`, and applying
    /// it changes nothing.
    #[test]
    fn a_result_of_an_earlier_state_is_stale() {
        type Change = fn(&mut Document, StampId);
        let changes: [(&str, Change); 4] = [
            ("moved", |d, id| {
                let at = placement_of(d, id);
                d.set_stamp_placement(id, moved(at, Vec2::new(0.1, 0.0)), Commit::Final).unwrap();
            }),
            ("removed", |d, id| d.remove_stamp(id).unwrap()),
            ("undone", |d, _| {
                d.undo().unwrap();
            }),
            ("moved by a Preview drag", |d, id| {
                let at = placement_of(d, id);
                d.set_stamp_placement(id, moved(at, Vec2::new(-0.1, 0.0)), Commit::Preview).unwrap();
            }),
        ];
        for (what, change) in changes {
            let (mut doc, id) = placed();
            let at = placement_of(&doc, id);
            doc.set_stamp_placement(id, scaled(at, 0.8), Commit::Final).unwrap();
            assert!(doc.stamp_is_unbaked(id) && doc.has_unbaked_stamps());
            let job = doc.next_final_bake().expect("a job");
            assert_eq!(job.stamp(), id);
            assert!(!job.is_text());
            let out = job.run();
            change(&mut doc, id);
            let before = doc_state(&mut doc);
            let (revision, state) = (doc.revision(), doc.history_state());
            assert!(matches!(doc.apply_final_bake(out), FinalBake::Stale), "{what}");
            assert_eq!(doc_state(&mut doc), before, "{what}: a stale result changed the document");
            assert_eq!((doc.revision(), doc.history_state()), (revision, state), "{what}");
        }
        // A result for another document (other mesh and maps) is stale too.
        let (mut a, id) = placed();
        let (mut b, _) = placed();
        let at = placement_of(&a, id);
        a.set_stamp_placement(id, scaled(at, 0.8), Commit::Final).unwrap();
        b.set_stamp_placement(id, scaled(at, 0.8), Commit::Final).unwrap();
        let out = a.next_final_bake().unwrap().run();
        assert!(matches!(b.apply_final_bake(out), FinalBake::Stale));
        assert!(b.stamp_is_unbaked(id));
    }

    /// Plan 1.7 item 3: undo restores "still needs its Full bake" with the caches it belongs to.
    #[test]
    fn undo_restores_the_unbaked_flag() {
        let (mut sync, id) = {
            let (mut doc, blob) = two_set_doc(false);
            let p = Placement::Projected(front(0.0, 0.5, 0.5, Vec2::splat(0.3), 0.6));
            let id = doc.add_stamp("S", StampSource::Image { blob }, p, TextureSetId(0)).unwrap();
            (doc, id)
        };
        let (mut doc, id2) = placed();
        assert_eq!(id, id2);
        let at = placement_of(&doc, id);
        let first = moved(at, Vec2::new(0.08, 0.0));
        let second = moved(at, Vec2::new(-0.08, 0.05));
        sync.set_stamp_placement(id, first, Commit::Final).unwrap();
        let want = look(&mut sync);

        let before_first = look(&mut doc);
        doc.set_stamp_placement(id, first, Commit::Final).unwrap();
        assert!(doc.stamp_is_unbaked(id));
        assert_eq!(look(&mut doc).sets, before_first.sets, "nothing is baked at the Final");
        doc.set_stamp_placement(id, second, Commit::Final).unwrap();
        assert_eq!(doc.undo().as_deref(), Some("Edit Stamp"));
        assert!(doc.stamp_is_unbaked(id), "the restored state still needs its Full bake");
        assert_eq!(placement_of(&doc, id), first);
        assert_eq!(pump(&mut doc), 1);
        assert_eq!(look(&mut doc), want);
        // Redo brings the second placement back unbaked; undoing twice gives the baked placement of `placed`.
        assert_eq!(doc.redo().as_deref(), Some("Edit Stamp"));
        assert!(doc.stamp_is_unbaked(id) && placement_of(&doc, id) == second);
        doc.undo();
        assert!(!doc.stamp_is_unbaked(id), "the bake of the first placement was kept by the redo entry");
        doc.undo();
        assert!(!doc.has_unbaked_stamps());
        assert_eq!(look(&mut doc), before_first);
        // A cancelled Preview drag gives the unbaked state back too.
        doc.redo();
        assert!(!doc.stamp_is_unbaked(id));
        doc.redo();
        assert!(doc.stamp_is_unbaked(id));
        doc.set_stamp_placement(id, first, Commit::Preview).unwrap();
        assert!(!doc.has_unbaked_stamps(), "a stamp being edited is not waiting");
        assert!(doc.next_final_bake().is_none());
        assert!(doc.cancel_pending_edit());
        assert!(doc.stamp_is_unbaked(id) && placement_of(&doc, id) == second);
        pump(&mut doc);
    }

    /// Plan 1.7 item 4: merge down and rasterize of an unbaked stamp keep the pixels of the synchronous document.
    #[test]
    fn merge_and_rasterize_of_an_unbaked_stamp_bake_it_first() {
        for rasterize in [false, true] {
            let mut docs = Vec::new();
            for deferred in [false, true] {
                let (mut doc, blob) = two_set_doc(deferred);
                let p = Placement::Projected(front(0.0, 0.5, 0.5, Vec2::splat(0.3), 0.6));
                let id = doc.add_stamp("S", StampSource::Image { blob }, p, TextureSetId(0)).unwrap();
                pump(&mut doc);
                doc.set_stamp_placement(id, scaled(p, 1.3), Commit::Final).unwrap();
                assert_eq!(doc.stamp_is_unbaked(id), deferred);
                let layer = doc.stamp_layers(id)[0].1;
                if rasterize {
                    doc.rasterize_layer(layer).unwrap();
                } else {
                    doc.merge_down(layer).unwrap();
                }
                assert!(!doc.stamp_is_unbaked(id));
                docs.push(look(&mut doc));
            }
            assert_eq!(docs[0], docs[1], "rasterize: {rasterize}");
        }
    }

    /// Plan 1.7 item 5: an output that arrives during a stroke is held (`Busy`) and applies after it.
    #[test]
    fn a_result_during_a_stroke_waits_for_its_end() {
        let (mut doc, id) = placed();
        let at = placement_of(&doc, id);
        doc.set_stamp_placement(id, scaled(at, 0.7), Commit::Final).unwrap();
        let out = doc.next_final_bake().unwrap().run();
        doc.stroke_begin(&BrushParams::default()).unwrap();
        assert!(doc.next_final_bake().is_none(), "no bake starts during a stroke");
        let out = match doc.apply_final_bake(out) {
            FinalBake::Busy(out) => out,
            other => panic!("expected Busy, got {other:?}"),
        };
        assert!(doc.stamp_is_unbaked(id));
        doc.stroke_end();
        assert!(matches!(doc.apply_final_bake(*out), FinalBake::Applied { id: applied, empty: false } if applied == id));
        assert!(!doc.has_unbaked_stamps());
    }

    /// Plan 1.7 item 6: `add_stamp` with deferred Final bakes: one entry, the home layer exists (empty) and is active,
    /// the layers of other covered sets appear when the bake lands (empty ones never); an undo before that removes
    /// everything and the late result is stale.
    #[test]
    fn add_stamp_deferred_shows_the_home_layer_and_bakes_later() {
        let (mut doc, blob) = two_set_doc(true);
        let revision = doc.revision();
        let p = Placement::Projected(front(0.0, 0.5, 0.5, Vec2::splat(0.4), 0.6));
        let id = doc.add_stamp("S", StampSource::Image { blob }, p, TextureSetId(0)).unwrap();
        assert_eq!(doc.history.undo_len(), 1);
        assert_eq!(doc.undo_label(), Some("Add Stamp"));
        let linked = doc.stamp_layers(id);
        assert_eq!(linked.len(), 1, "only the home layer before the bake");
        assert_eq!(linked[0].0, TextureSetId(0));
        assert_eq!(doc.set(TextureSetId(0)).unwrap().active, Some(linked[0].1));
        assert!(doc.layer(linked[0].1).unwrap().content_rect().is_empty());
        assert!(doc.stamp_is_unbaked(id));
        let state = doc.history_state();
        let modified = doc.is_modified();
        let out = doc.next_final_bake().unwrap().run();
        assert!(matches!(doc.apply_final_bake(out), FinalBake::Applied { empty: false, .. }));
        let linked = doc.stamp_layers(id);
        assert_eq!(linked.iter().map(|(s, _)| s.0).collect::<Vec<_>>(), [0, 1], "set 1's layer came with the bake");
        assert!(doc.revision() > revision);
        assert_eq!((doc.history_state(), doc.is_modified()), (state, modified), "a bake is not an edit");
        assert_eq!(doc.history.undo_len(), 1);

        // A stamp that covers only its home set never gets a layer elsewhere; one that misses the model keeps its
        // empty home layer.
        let low = Placement::Projected(front(0.0, -0.2, 0.5, Vec2::splat(0.1), 0.6));
        let only_home = doc.add_stamp("Low", StampSource::Image { blob }, low, TextureSetId(0)).unwrap();
        pump(&mut doc);
        assert_eq!(doc.stamp_layers(only_home).len(), 1);
        let far = Placement::Projected(front(20.0, 20.0, 0.5, Vec2::splat(0.1), 0.2));
        let missed = doc.add_stamp("Far", StampSource::Image { blob }, far, TextureSetId(0)).unwrap();
        let out = doc.next_final_bake().unwrap().run();
        assert!(matches!(doc.apply_final_bake(out), FinalBake::Applied { empty: true, .. }));
        assert_eq!(doc.stamp_layers(missed).len(), 1);

        // Undo before the bake lands: everything goes, and the late result is stale.
        let before = doc_state(&mut doc);
        let text = text(&mut doc, "Late");
        let at = text_placement(&mut doc, &text, 0.5, 0.2);
        let late = doc.add_stamp("Late", StampSource::Text(text), at, TextureSetId(0)).unwrap();
        let job = doc.next_final_bake().unwrap();
        assert!(job.is_text());
        let out = job.run();
        assert_eq!(doc.undo().as_deref(), Some("Add Text"));
        assert!(doc.stamp(late).is_none() && doc.stamp_layers(late).is_empty() && !doc.has_unbaked_stamps());
        assert!(matches!(doc.apply_final_bake(out), FinalBake::Stale));
        assert_eq!(doc_state(&mut doc), before);
        // Redo: the stamp is back, unbaked, and its bake lands.
        doc.redo();
        assert!(doc.stamp_is_unbaked(late));
        assert_eq!(pump(&mut doc), 1);
        assert!(!doc.stamp_layers(late).is_empty());
    }

    /// Duplicates of an unbaked stamp wait too; removing a stamp or its last layer clears the flag; a failing bake
    /// clears it and keeps the earlier caches; turning the mode off bakes everything.
    #[test]
    fn the_flag_follows_duplicates_removals_failures_and_the_mode() {
        let (mut doc, id) = placed();
        let at = placement_of(&doc, id);
        doc.set_stamp_placement(id, scaled(at, 0.7), Commit::Final).unwrap();
        let copy = doc.duplicate_stamp(id).unwrap();
        assert!(doc.stamp_is_unbaked(copy) && doc.stamp_is_unbaked(id));
        doc.remove_stamp(id).unwrap();
        assert!(!doc.stamp_is_unbaked(id) && doc.stamp_is_unbaked(copy));
        // Rasterizing bakes the copy first (rule 8), which may drop an empty layer: take the layers one at a time.
        while let Some(&(_, layer)) = doc.stamp_layers(copy).first() {
            doc.rasterize_layer(layer).unwrap();
        }
        assert!(doc.stamp(copy).is_none() && !doc.has_unbaked_stamps());

        // A text whose font blob is broken cannot start its bake: logged, flag cleared, caches kept.
        let (mut doc, _) = placed();
        let mut spec = text(&mut doc, "Font");
        let at = text_placement(&mut doc, &spec, 0.3, 0.1);
        let t = doc.add_stamp("T", StampSource::Text(spec.clone()), at, TextureSetId(0)).unwrap();
        pump(&mut doc);
        let baked = doc_state(&mut doc);
        spec.font = doc.register_font("Broken", "Regular", 0, Arc::from(&b"garbage"[..]));
        // Validation of the edit itself rejects the broken font (as before).
        assert!(doc.set_stamp_source(t, StampSource::Text(spec), Commit::Final).is_err());
        assert_eq!(doc_state(&mut doc), baked);

        // Mode off: every waiting stamp is baked at once, and the mode is synchronous again.
        let (mut doc, id) = placed();
        let at = placement_of(&doc, id);
        doc.set_stamp_placement(id, scaled(at, 1.2), Commit::Final).unwrap();
        doc.set_stamp_placement(id, scaled(at, 1.4), Commit::Final).unwrap();
        assert!(doc.stamp_is_unbaked(id));
        doc.set_defer_final_bakes(false);
        assert!(!doc.has_unbaked_stamps() && doc.bakes.unbaked.is_empty());
        let after_off = doc_state(&mut doc);
        // An undo to a state recorded while the mode was on (the 1.2 placement, unbaked) bakes it right away.
        assert_eq!(doc.undo().as_deref(), Some("Edit Stamp"));
        assert!(doc.bakes.unbaked.is_empty());
        doc_state(&mut doc);
        doc.redo();
        assert_eq!(doc_state(&mut doc), after_off);
        doc.set_stamp_placement(id, at, Commit::Final).unwrap();
        assert!(!doc.has_unbaked_stamps() && doc.next_final_bake().is_none());
    }

    /// `settle_final_bakes` bakes every waiting stamp except the one being edited; a bake job is `Send`.
    #[test]
    fn settle_bakes_everything_but_an_open_edit() {
        let (mut doc, a) = placed();
        let blob = match doc.stamp(a).unwrap().source {
            StampSource::Image { blob } => blob,
            StampSource::Text(_) => unreachable!(),
        };
        let p = Placement::Projected(front(0.1, 0.2, 0.5, Vec2::splat(0.2), 0.6));
        let b = doc.add_stamp("B", StampSource::Image { blob }, p, TextureSetId(0)).unwrap();
        let at = placement_of(&doc, a);
        doc.set_stamp_placement(a, scaled(at, 0.9), Commit::Final).unwrap();
        doc.set_stamp_placement(a, scaled(at, 0.8), Commit::Preview).unwrap();
        assert!(doc.stamp_is_unbaked(a) && doc.stamp_is_unbaked(b));
        doc.settle_final_bakes();
        assert!(!doc.stamp_is_unbaked(b) && doc.stamp_is_unbaked(a));
        assert!(!doc.has_unbaked_stamps());
        doc.flush_pending_edit();
        assert!(doc.has_unbaked_stamps());
        // The job runs on another thread, inside the core's bake pool.
        let job = doc.next_final_bake().unwrap();
        let out = std::thread::spawn(move || job.run_isolated()).join().unwrap();
        assert!(!out.is_error());
        assert!(matches!(doc.apply_final_bake(out), FinalBake::Applied { .. }));
        assert!(!doc.has_unbaked_stamps());
    }

    /// Check-and-fix (rule 6, the event table's last row): an output that carries an error is `Failed`: the stamp is no
    /// longer waiting, its earlier caches stay, history and revision do not move; a later Final bakes it again.
    #[test]
    fn a_failed_bake_keeps_the_earlier_picture_and_clears_the_flag() {
        let (mut doc, id) = placed();
        let at = placement_of(&doc, id);
        doc.set_stamp_placement(id, scaled(at, 0.8), Commit::Final).unwrap();
        let mut out = doc.next_final_bake().expect("a job").run();
        out.result = Err(EditError::UnknownSet);
        assert!(out.is_error());
        let before = doc_state(&mut doc);
        let (revision, state) = (doc.revision(), doc.history_state());
        assert!(matches!(doc.apply_final_bake(out), FinalBake::Failed { id: failed } if failed == id));
        assert!(!doc.stamp_is_unbaked(id) && !doc.has_unbaked_stamps() && doc.next_final_bake().is_none());
        assert_eq!(doc_state(&mut doc), before, "the earlier caches stay");
        assert_eq!((doc.revision(), doc.history_state()), (revision, state));
        // The next Final waits again and bakes normally.
        doc.set_stamp_placement(id, scaled(at, 0.7), Commit::Final).unwrap();
        assert!(doc.stamp_is_unbaked(id));
        assert_eq!(pump(&mut doc), 1);
    }
}
