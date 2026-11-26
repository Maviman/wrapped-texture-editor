// Owner: stickers P3a (STICKER_PLAN 2.6-2.9; ARCHITECTURE 5.20, 6.5, 6.10). The stickers of a document: queries,
// add / remove / promote / detach, `set_sticker` (the one structural edit), stamp styles, the two placements, the
// finished pictures (`stamp_picture`, `sticker_picture`) and the derived caches: renders, per-layer rasters,
// decoded blob pictures and styled stamp pictures (`StickerCaches`).
//
// Choices the plan leaves open, documented at the items below:
// - The pipeline takes the styled path when a stamp's styles have an entry that is on (`StyleStack::any_on`); a stack
//   whose entries are all off draws exactly what no stack draws, so it keeps the bit-identical path too.
// - Caches compare stickers by `Arc::ptr_eq` while holding the `Arc` (never a bare address) and pictures by version.
// - `StickerRef::OfStamp` of an image stamp promotes it inside the edit (P3b, 2.10): `set_sticker` here, the pixel
//   edits in `doc_sticker_edits.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::blob::BlobKind;
use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::ids::{BlobId, StampId, StickerId};
use crate::image_io::decode_image;
use crate::pixel::PixelBuf;
use crate::pixel_ops::PremulMips;
use crate::stamp::{BakeQuality, Placement, Projector, StampObject, StampSource, SurfaceMode};
use crate::sticker::model::{PicturePixels, StickerContent, StickerDoc};
use crate::sticker::pipeline::{
    STYLE_REFERENCE_EM, SourceFrame, Styling, bake_placement_of, placement_texels, sticker_scale, styled_mips,
    styled_straight, work_level,
};
use crate::sticker::render::{LayerCache, StickerRenderInput, render_size, render_with};
use crate::sticker::style::StyleStack;
use crate::sticker::{MAX_PICTURE_SIDE, refit_placement};
use crate::text::RasterQuality;
use crate::text::layout::raster_px;

/// History label of `set_stamp_styles`; its `Commit::Preview` calls fold into one entry.
pub const STYLE_LABEL: &str = "Edit Sticker Style";
/// Most finished sticker renders a document keeps (2.7), and their bytes (pictures and mips together).
const RENDER_CACHE_ENTRIES: usize = 8;
const RENDER_CACHE_BYTES: usize = 512 << 20;

/// Which sticker a content command acts on (2.8).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StickerRef {
    Id(StickerId),
    /// The sticker of this stamp. For an image stamp that is not a sticker yet, a content command promotes it inside
    /// itself (2.10: promotion and edit are one history entry); a text stamp is `InvalidParam`.
    OfStamp(StampId, ShareScope),
}

/// Which stamps a promotion or an edit through a stamp reaches (E6).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShareScope {
    ThisStamp,
    /// Every stamp showing the same image (they then share the sticker).
    AllUsers,
}

/// What a derived (styled or sticker) picture was made from (2.6): compared with `Arc::ptr_eq` while the entry holds
/// the `Arc`, so a reused address can never match.
#[derive(Clone, Debug)]
pub(crate) enum BaseIdentity {
    Blob(BlobId),
    Text(Arc<PremulMips>),
    /// The sticker value and the render scale's bits.
    Sticker(Arc<StickerDoc>, u32),
}

impl BaseIdentity {
    /// Same source picture at the same resolution.
    pub(crate) fn same(&self, other: &BaseIdentity) -> bool {
        match (self, other) {
            (BaseIdentity::Sticker(a, s), BaseIdentity::Sticker(b, t)) => Arc::ptr_eq(a, b) && s == t,
            _ => self.same_source(other),
        }
    }

    /// Same source picture at any resolution (what a Preview accepts).
    pub(crate) fn same_source(&self, other: &BaseIdentity) -> bool {
        match (self, other) {
            (BaseIdentity::Blob(a), BaseIdentity::Blob(b)) => a == b,
            (BaseIdentity::Text(a), BaseIdentity::Text(b)) => Arc::ptr_eq(a, b),
            (BaseIdentity::Sticker(a, _), BaseIdentity::Sticker(b, _)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// A styled stamp picture (2.6 step 4), cached per stamp in `StickerCaches::styled`.
#[derive(Clone, Debug)]
pub(crate) struct StyledEntry {
    pub(crate) base: BaseIdentity,
    pub(crate) styles: StyleStack,
    pub(crate) tint: [u8; 4],
    /// Bits of the style scale (reference scale times working scale).
    pub(crate) scale_bits: u32,
    pub(crate) quality: BakeQuality,
    pub(crate) frame: SourceFrame,
    pub(crate) mips: Arc<PremulMips>,
}

impl StyledEntry {
    /// True when this entry is the picture `styling` asks for on `base`: a Preview takes any entry of the same source,
    /// styles and tint; a Full needs a Full entry of the same resolution and style scale.
    fn serves(&self, base: &BaseIdentity, styling: &Styling, scale: f32) -> bool {
        let same = self.styles == styling.styles && self.tint == styling.tint;
        match styling.quality {
            BakeQuality::Preview => same && self.base.same_source(base),
            BakeQuality::Full => {
                same && self.quality == BakeQuality::Full && self.base.same(base) && self.scale_bits == scale.to_bits()
            }
        }
    }
}

/// A finished sticker render (2.7) and its mips, looked up with `Arc::ptr_eq` (the entry holds the `Arc`).
#[derive(Clone, Debug)]
pub(crate) struct RenderEntry {
    pub(crate) doc: Arc<StickerDoc>,
    pub(crate) scale_bits: u32,
    pub(crate) quality: BakeQuality,
    pub(crate) picture: Arc<PixelBuf>,
    pub(crate) mips: Arc<PremulMips>,
}

impl RenderEntry {
    fn bytes(&self) -> usize {
        self.picture.byte_len() + self.mips.levels.iter().map(PixelBuf::byte_len).sum::<usize>()
    }
}

/// The derived sticker data of a document (2.7). Not persisted, not part of the history.
#[derive(Default)]
pub struct StickerCaches {
    /// Decoded blob pictures (straight RGBA8) for picture layers and finished pictures.
    pub(crate) blob_pictures: HashMap<BlobId, Arc<PixelBuf>>,
    /// Per-layer rasters (`LayerKey`), LRU.
    pub(crate) layers: LayerCache,
    /// Finished renders, least recently used first.
    pub(crate) renders: Vec<RenderEntry>,
    /// Styled stamp pictures, one per stamp.
    pub(crate) styled: HashMap<StampId, StyledEntry>,
}

impl StickerCaches {
    /// Stores a render (an entry of the same sticker, scale and quality is replaced), oldest dropped beyond the limits.
    pub(crate) fn insert_render(&mut self, entry: RenderEntry) {
        self.renders.retain(|e| {
            !(Arc::ptr_eq(&e.doc, &entry.doc) && e.scale_bits == entry.scale_bits && e.quality == entry.quality)
        });
        self.renders.push(entry);
        while self.renders.len() > RENDER_CACHE_ENTRIES
            || (self.renders.len() > 1 && self.renders.iter().map(RenderEntry::bytes).sum::<usize>() > RENDER_CACHE_BYTES)
        {
            self.renders.remove(0);
        }
    }

    /// The cached render of `doc`: exactly at `scale` and `quality` (`exact`), or (a Preview) any render of that
    /// value, the most recent first. Marks the entry most recently used.
    pub(crate) fn find_render(&mut self, doc: &Arc<StickerDoc>, scale: f32, quality: BakeQuality, exact: bool) -> Option<RenderEntry> {
        let i = self.renders.iter().rposition(|e| {
            Arc::ptr_eq(&e.doc, doc) && (!exact || (e.scale_bits == scale.to_bits() && e.quality == quality))
        })?;
        let entry = self.renders.remove(i);
        self.renders.push(entry.clone());
        Some(entry)
    }
}

/// The picture a bake of a stamp places (2.6): its mips (margin included), their frame and the tint the bake applies
/// (`[255; 4]` when the styles took the tint already).
pub(crate) struct BakePicture {
    pub(crate) mips: Arc<PremulMips>,
    pub(crate) frame: SourceFrame,
    pub(crate) tint: [u8; 4],
}

/// The picture of the trivial fast path of a sticker whose one picture is an unchanged blob filling the artboard.
fn trivial_blob(doc: &StickerDoc) -> Option<BlobId> {
    match doc.trivial_picture()? {
        PicturePixels::Blob(b) => Some(*b),
        PicturePixels::Owned { .. } => None,
    }
}

/// `true` when the stamp's styles draw something (the styled path of 2.6).
fn styled(stamp: &StampObject) -> bool {
    stamp.styles.any_on()
}

impl Document {
    // ----- queries -----

    /// The sticker with this id.
    pub fn sticker(&self, id: StickerId) -> Option<&Arc<StickerDoc>> {
        self.sticker_state.map.get(&id)
    }

    /// Every sticker of the document, ascending id.
    pub fn stickers(&self) -> impl Iterator<Item = (StickerId, &Arc<StickerDoc>)> {
        self.sticker_state.map.iter().map(|(id, doc)| (*id, doc))
    }

    /// The stamps showing sticker `id`, ascending.
    pub fn sticker_users(&self, id: StickerId) -> Vec<StampId> {
        self.stamps.iter().filter(|s| s.source == StampSource::Sticker { sticker: id }).map(|s| s.id).collect()
    }

    /// The image stamps showing blob `blob`, ascending (E6).
    pub fn blob_users(&self, blob: BlobId) -> Vec<StampId> {
        self.stamps.iter().filter(|s| s.source == StampSource::Image { blob }).map(|s| s.id).collect()
    }

    /// The sticker of a stamp's source (`None` for image and text stamps).
    pub(crate) fn sticker_of(&self, object: &StampObject) -> Option<Arc<StickerDoc>> {
        match &object.source {
            StampSource::Sticker { sticker } => self.sticker_state.map.get(sticker).cloned(),
            _ => None,
        }
    }

    // ----- commands -----

    /// "Add Sticker": puts `doc` into the document (sanitized; owned pictures get fresh versions). One entry. Errors:
    /// the limits of 2.7 (`StickerLimit`, `InvalidParam`), `UnknownBlob` for a picture blob that is missing.
    pub fn add_sticker(&mut self, doc: StickerDoc) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let mut doc = doc;
        doc.sanitize();
        self.version_new_buffers(&mut doc, None);
        self.check_sticker(&doc)?;
        let before = self.snapshot_structure(&[]);
        let id = StickerId(self.alloc_id());
        self.sticker_state.map.insert(id, Arc::new(doc));
        self.push_structure_edit("Add Sticker", before);
        Ok(id)
    }

    /// "Delete Sticker": removes a sticker no stamp shows. `Err(InvalidParam)` while a stamp uses it.
    pub fn remove_sticker(&mut self, id: StickerId) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        if !self.sticker_state.map.contains_key(&id) {
            return Err(EditError::UnknownSticker);
        }
        if !self.sticker_users(id).is_empty() {
            return Err(EditError::InvalidParam("a stamp shows this sticker"));
        }
        let before = self.snapshot_structure(&[]);
        self.sticker_state.map.remove(&id);
        self.push_structure_edit("Delete Sticker", before);
        Ok(())
    }

    /// "Convert to Sticker" (2.8): image stamp -> a sticker with one `Picture(Blob)` layer, the artboard = the image,
    /// identity transform. No pixel changes and no bake (the sticker renders the image's pixels unchanged, 4.1); the
    /// linked layers keep their ids and caches. `scope`: this stamp only, or every image stamp showing the blob (they
    /// then share the sticker). One entry. Errors: `UnknownStamp`, `InvalidParam` (not an image stamp), `LayerLocked`
    /// (a stamp in scope is locked), `UnknownBlob` / `Image`.
    pub fn promote_stamp(&mut self, stamp: StampId, scope: ShareScope) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let before = self.snapshot_structure(&[]);
        let id = self.promote_no_history(stamp, scope)?;
        self.push_structure_edit("Convert to Sticker", before);
        Ok(id)
    }

    /// `promote_stamp` without a history entry (the caller records it: promotion inside an edit, 2.10). Every error
    /// comes before anything changes.
    pub(crate) fn promote_no_history(&mut self, stamp: StampId, scope: ShareScope) -> Result<StickerId, EditError> {
        let object = self.stamp(stamp).ok_or(EditError::UnknownStamp)?;
        let StampSource::Image { blob } = object.source else {
            return Err(EditError::InvalidParam("only an image stamp can be converted to a sticker"));
        };
        let name = object.name.clone();
        let targets = match scope {
            ShareScope::ThisStamp => vec![stamp],
            ShareScope::AllUsers => self.blob_users(blob),
        };
        if targets.iter().any(|t| self.stamp_locked(*t)) {
            return Err(EditError::LayerLocked);
        }
        let size = self.image_size(blob)?;
        let doc = StickerDoc::from_picture(&name, PicturePixels::Blob(blob), size);
        let id = StickerId(self.alloc_id());
        self.sticker_state.map.insert(id, Arc::new(doc));
        for s in self.stamps.iter_mut().filter(|s| targets.contains(&s.id)) {
            s.source = StampSource::Sticker { sticker: id };
        }
        Ok(id)
    }

    /// "Only This Sticker" (E6): stamp `stamp` gets its own copy of its (shared) sticker under a new id; the pixels stay
    /// shared (`Arc`), so nothing is drawn again. One entry ("Detach Sticker"). Errors: `UnknownStamp`, `InvalidParam`
    /// (not a sticker stamp), `LayerLocked`.
    pub fn detach_sticker(&mut self, stamp: StampId) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let object = self.stamp(stamp).ok_or(EditError::UnknownStamp)?;
        let StampSource::Sticker { sticker } = object.source else {
            return Err(EditError::InvalidParam("the stamp does not show a sticker"));
        };
        if self.stamp_locked(stamp) {
            return Err(EditError::LayerLocked);
        }
        let copy = StickerDoc { keep: false, ..(**self.sticker_state.map.get(&sticker).ok_or(EditError::UnknownSticker)?).clone() };
        let before = self.snapshot_structure(&[]);
        let id = StickerId(self.alloc_id());
        self.sticker_state.map.insert(id, Arc::new(copy));
        if let Some(s) = self.stamps.iter_mut().find(|s| s.id == stamp) {
            s.source = StampSource::Sticker { sticker: id };
        }
        self.push_structure_edit("Detach Sticker", before);
        Ok(id)
    }

    /// The one structural sticker edit (2.8): replaces the sticker value by `edit` applied to a copy (sanitized; new
    /// owned buffers get fresh versions; the limits of 2.7 checked before anything changes). When the artboard changed,
    /// every user's placement is refitted (2.9, an automatic depth with it). Users are re-baked: `Commit::Preview` calls
    /// with the same `label` fold into one pending entry (Preview bakes), `Commit::Final` ends it (Final bakes, deferred
    /// when that mode is on). Nothing changes and no entry is made when the value is unchanged. `OfStamp` of an image
    /// stamp promotes it first without an entry of its own (2.10, review A3): the pending entry's "before" still shows
    /// the image stamp, so a Preview, Preview, Final drag is one undo step and `cancel_pending_edit` brings the image
    /// stamp back (the promotion is recorded even when the edit itself changes nothing). Errors, each before anything
    /// changes: `UnknownSticker`, `UnknownStamp`, `InvalidParam` (a text stamp), `LayerLocked` (a user is locked),
    /// the limits, `StrokeInProgress`.
    pub fn set_sticker(
        &mut self,
        on: StickerRef,
        edit: impl FnOnce(&mut StickerDoc),
        label: &str,
        commit: Commit,
    ) -> Result<StickerId, EditError> {
        self.guard_no_stroke()?;
        let mut promoted = None;
        let id = match self.promotion_of(on)? {
            Some((stamp, scope)) => {
                // A pending edit of another label ends first, so it never contains the promotion.
                if self.pending_edit.as_ref().is_none_or(|p| p.label != label) {
                    self.flush_pending_edit();
                }
                let before = self.snapshot_structure(&[]);
                let id = self.promote_no_history(stamp, scope)?;
                promoted = Some(before);
                id
            }
            None => self.resolve_sticker(on)?,
        };
        let old = Arc::clone(self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?);
        let mut new = (*old).clone();
        edit(&mut new);
        new.sanitize();
        self.version_new_buffers(&mut new, Some(&old));
        if let Err(e) = self.check_sticker(&new) {
            if let Some(before) = promoted {
                self.restore_structure(before);
            }
            return Err(e);
        }
        let changed = new != *old;
        let continues = self.pending_edit.as_ref().is_some_and(|p| p.label == label);
        if !changed && !continues && promoted.is_none() {
            return Ok(id);
        }
        let users = self.sticker_users(id);
        if changed && users.iter().any(|u| self.stamp_locked(*u)) {
            if let Some(before) = promoted {
                self.restore_structure(before);
            }
            return Err(EditError::LayerLocked);
        }
        match promoted {
            Some(before) => self.begin_pending_with(label, before),
            None => self.begin_pending(label),
        }
        if changed {
            let refit = old.artboard != new.artboard;
            let new = Arc::new(new);
            let mut placements = Vec::new();
            if refit {
                for &u in &users {
                    let Some(stamp) = self.stamp(u).cloned() else { continue };
                    let old_frame = self.styled_frame(&stamp);
                    let mut placement = stamp.placement;
                    refit_placement(&mut placement, &old.artboard, &new.artboard);
                    placements.push((u, stamp, placement, old_frame));
                }
            }
            self.sticker_state.map.insert(id, Arc::clone(&new));
            for (u, stamp, placement, old_frame) in placements {
                let moved = StampObject { placement, ..stamp.clone() };
                let new_frame = self.styled_frame(&moved);
                let placement = self.refit_auto_depth_framed(&stamp.placement, &old_frame, placement, &new_frame);
                if let Some(s) = self.stamps.iter_mut().find(|s| s.id == u) {
                    s.placement = placement;
                }
            }
            for &u in &users {
                self.mark_stamp_edited(u, commit);
            }
        }
        if commit == Commit::Final {
            self.flush_pending_edit();
        }
        Ok(id)
    }

    /// "Edit Sticker Style": replaces the stamp-level styles of `stamp` (sanitized); `Commit::Preview` calls fold into
    /// one entry. An automatic depth is refitted when the margin changes (2.6). Errors: `UnknownStamp`, `LayerLocked`,
    /// `StrokeInProgress`, and the errors of preparing the source.
    pub fn set_stamp_styles(&mut self, stamp: StampId, styles: StyleStack, commit: Commit) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        let object = self.stamp(stamp).ok_or(EditError::UnknownStamp)?.clone();
        let styles = styles.sanitized();
        let changed = object.styles != styles;
        let continues = self.pending_edit.as_ref().is_some_and(|p| p.label == STYLE_LABEL);
        if !changed && !continues {
            return Ok(());
        }
        if changed && self.stamp_locked(stamp) {
            return Err(EditError::LayerLocked);
        }
        let new_object = StampObject { styles, ..object.clone() };
        let (old_frame, new_frame) = (self.styled_frame(&object), self.styled_frame(&new_object));
        self.begin_pending(STYLE_LABEL);
        if changed {
            let placement = self.refit_auto_depth_framed(&object.placement, &old_frame, object.placement, &new_frame);
            if let Some(s) = self.stamps.iter_mut().find(|s| s.id == stamp) {
                s.styles = new_object.styles;
                s.placement = placement;
            }
            self.mark_stamp_edited(stamp, commit);
        }
        if commit == Commit::Final {
            self.flush_pending_edit();
        }
        Ok(())
    }

    /// "Paste Style" (E5): gives every stamp in `stamps` the styles `styles`, one entry; each is re-baked (Final). Errors
    /// before anything changes: `UnknownStamp`, `LayerLocked`.
    pub fn set_stamps_styles(&mut self, stamps: &[StampId], styles: &StyleStack) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let styles = styles.sanitized();
        for id in stamps {
            self.stamp(*id).ok_or(EditError::UnknownStamp)?;
            if self.stamp_locked(*id) {
                return Err(EditError::LayerLocked);
            }
        }
        let todo: Vec<StampObject> =
            stamps.iter().filter_map(|id| self.stamp(*id)).filter(|s| s.styles != styles).cloned().collect();
        if todo.is_empty() {
            return Ok(());
        }
        self.begin_pending("Paste Style");
        for object in todo {
            let new_object = StampObject { styles: styles.clone(), ..object.clone() };
            let (old_frame, new_frame) = (self.styled_frame(&object), self.styled_frame(&new_object));
            let placement = self.refit_auto_depth_framed(&object.placement, &old_frame, object.placement, &new_frame);
            if let Some(s) = self.stamps.iter_mut().find(|s| s.id == object.id) {
                s.styles = styles.clone();
                s.placement = placement;
            }
            self.mark_stamp_edited(object.id, Commit::Final);
        }
        self.flush_pending_edit();
        Ok(())
    }

    // ----- the two placements (2.6) -----

    /// The placement the bake of stamp `id` uses (2.6): the stored placement grown by the styles' margin on every side,
    /// for the stamp's current picture (its cached styled picture, else the reference frame from geometry). Without a
    /// margin the stored placement bit for bit. `None` for an unknown id.
    pub fn bake_placement(&self, id: StampId) -> Option<Placement> {
        let stamp = self.stamp(id)?;
        Some(bake_placement_of(&stamp.placement, &self.current_frame(id)))
    }

    /// The bake placement of stamp `id` when it is a wrapped 3D stamp (what its parameterization is made for).
    pub(crate) fn bake_wrapped_projector(&self, id: StampId) -> Option<Projector> {
        match self.bake_placement(id)? {
            Placement::Projected(p) if p.mode == SurfaceMode::Wrap => Some(p),
            _ => None,
        }
    }

    /// The bake placement of stamp `id` when it is projected.
    pub(crate) fn bake_projector(&self, id: StampId) -> Option<Projector> {
        match self.bake_placement(id)? {
            Placement::Projected(p) => Some(p),
            Placement::Uv(_) => None,
        }
    }

    /// The frame of stamp `id`'s current picture without drawing it: margin 0 without styles; with styles the frame of
    /// its cached styled picture when that is the current one, else the reference frame (`stamp_source_frame2`) from
    /// the caches; margin 0 when nothing is known yet.
    pub(crate) fn current_frame(&self, id: StampId) -> SourceFrame {
        let Some(stamp) = self.stamp(id) else { return SourceFrame::plain([0, 0]) };
        if !styled(stamp) {
            return SourceFrame::plain([0, 0]);
        }
        if let Some(entry) = self.current_styled_entry(stamp) {
            return entry.frame;
        }
        let content = match &stamp.source {
            StampSource::Image { blob } => self.image_mips.get(blob).map(|m| m.size()),
            StampSource::Text(_) => self.cached_text_frame(id).map(|f| f.size),
            StampSource::Sticker { sticker } => self.sticker_state.map.get(sticker).map(|doc| {
                let s = sticker_scale(doc, self.texels_of(stamp).y, BakeQuality::Full);
                render_size(doc, s)
            }),
        };
        match content {
            Some(content) => SourceFrame { content, margin: stamp.styles.margin(self.ref_scale(stamp)) },
            None => SourceFrame::plain([0, 0]),
        }
    }

    /// The reference frame of stamp `id` (2.6, `SourceFrame` at working scale 1): content = the base picture's size
    /// (image pixels, the text picture, the sticker render at its Full scale), margin = the styles' margin at the
    /// reference scale. Geometry only (an image may be decoded, a text laid out; nothing is drawn).
    pub fn stamp_source_frame2(&mut self, id: StampId) -> Result<SourceFrame, EditError> {
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?.clone();
        let content = match &stamp.source {
            StampSource::Image { blob } => self.image_size(*blob)?,
            StampSource::Text(_) => self.stamp_text_frame(id)?.size,
            StampSource::Sticker { sticker } => {
                let doc = self.sticker_state.map.get(sticker).ok_or(EditError::UnknownSticker)?;
                render_size(doc, sticker_scale(doc, self.texels_of(&stamp).y, BakeQuality::Full))
            }
        };
        let margin = if styled(&stamp) { stamp.styles.margin(self.ref_scale(&stamp)) } else { 0 };
        Ok(SourceFrame { content, margin })
    }

    /// The reference frame of a (possibly not yet stored) stamp object, for the auto-depth refit: margin 0 without
    /// styles; otherwise as `stamp_source_frame2` (a text from its frame, laid out when not cached).
    pub(crate) fn styled_frame(&mut self, object: &StampObject) -> SourceFrame {
        if !styled(object) {
            return SourceFrame::plain([0, 0]);
        }
        let content = match &object.source {
            StampSource::Image { blob } => self.image_size(*blob).ok(),
            StampSource::Text(spec) => self.frame_of_spec(spec).ok().map(|f| f.size),
            StampSource::Sticker { sticker } => self
                .sticker_state
                .map
                .get(sticker)
                .map(|doc| render_size(doc, sticker_scale(doc, self.texels_of(object).y, BakeQuality::Full))),
        };
        match content {
            Some(content) => SourceFrame { content, margin: object.styles.margin(self.ref_scale(object)) },
            None => SourceFrame::plain([0, 0]),
        }
    }

    /// `new` (a changed placement of a stamp whose picture frame goes from `old_frame` to `new_frame`) with its depth
    /// re-fitted when `old` has an automatic depth, measured on the bake footprints (2.6 "Auto depth"); the depth is
    /// written into the stored placement. Without a margin on either side exactly `refit_auto_depth`.
    pub(crate) fn refit_auto_depth_framed(&self, old: &Placement, old_frame: &SourceFrame, new: Placement, new_frame: &SourceFrame) -> Placement {
        if old_frame.margin == 0 && new_frame.margin == 0 {
            return self.refit_auto_depth(old, new);
        }
        match (old, new) {
            (Placement::Projected(o), Placement::Projected(mut n)) if n != *o || old_frame != new_frame => {
                let Placement::Projected(bo) = bake_placement_of(old, old_frame) else { return new };
                if !self.stamp_depth_is_auto(&bo) {
                    return new;
                }
                let Placement::Projected(bn) = bake_placement_of(&Placement::Projected(n), new_frame) else { return new };
                let fitted = self.fit_stamp_depth(&bn);
                n.depth_front = fitted.depth_front;
                n.depth_back = fitted.depth_back;
                Placement::Projected(n)
            }
            _ => new,
        }
    }

    // ----- finished pictures -----

    /// The finished picture of stamp `stamp` (straight RGBA8): the source at full resolution (image pixels, the full
    /// text picture, the sticker at its Full render scale) and, `with_styles`, the tint and the styles drawn around it
    /// ("what you see": copy, drag out, Export Picture). Never a history command.
    pub fn stamp_picture(&mut self, stamp: StampId, with_styles: bool) -> Result<PixelBuf, EditError> {
        let object = self.stamp(stamp).ok_or(EditError::UnknownStamp)?.clone();
        let base: Arc<PixelBuf> = match &object.source {
            StampSource::Image { blob } => self.blob_picture(*blob)?,
            StampSource::Text(spec) => Arc::new(self.render_spec(spec, RasterQuality::Full)?.image),
            StampSource::Sticker { sticker } => {
                let doc = Arc::clone(self.sticker_state.map.get(sticker).ok_or(EditError::UnknownSticker)?);
                let s = sticker_scale(&doc, self.texels_of(&object).y, BakeQuality::Full);
                self.sticker_render(&doc, s, BakeQuality::Full)?.picture
            }
        };
        if !with_styles {
            return Ok((*base).clone());
        }
        let mut picture = (*base).clone();
        if styled(&object) {
            let styling = Styling {
                styles: object.styles.sanitized(),
                tint: object.tint,
                ref_scale: self.ref_scale(&object),
                texels: glam::Vec2::splat(f32::INFINITY),
                quality: BakeQuality::Full,
            };
            picture = styled_straight(&picture, &styling);
        } else if object.tint != [255; 4] {
            for p in picture.data.as_chunks_mut::<4>().0.iter_mut() {
                for (c, t) in p.iter_mut().zip(object.tint) {
                    *c = crate::math::mul_u8(*c, t);
                }
            }
        }
        Ok(picture)
    }

    /// The finished picture of sticker `id` (a tray sticker, a thumbnail) at render scale `scale` (finite, 1/64..=4;
    /// halved while the render would exceed 4096 pixels a side), with `styles` drawn around it when given (their
    /// lengths in artboard pixels).
    pub fn sticker_picture(&mut self, id: StickerId, styles: Option<&StyleStack>, scale: f32) -> Result<PixelBuf, EditError> {
        let doc = Arc::clone(self.sticker_state.map.get(&id).ok_or(EditError::UnknownSticker)?);
        let mut s = if scale.is_finite() { scale.clamp(1.0 / 64.0, 4.0) } else { 1.0 };
        while s > 1.0 / 64.0 && render_size(&doc, s).iter().any(|&v| v > 4096) {
            s /= 2.0;
        }
        let picture = self.sticker_render(&doc, s, BakeQuality::Full)?.picture;
        match styles.map(StyleStack::sanitized).filter(StyleStack::any_on) {
            Some(styles) => {
                let styling = Styling {
                    styles,
                    tint: [255; 4],
                    ref_scale: s,
                    texels: glam::Vec2::splat(f32::INFINITY),
                    quality: BakeQuality::Full,
                };
                Ok(styled_straight(&picture, &styling))
            }
            None => Ok((*picture).clone()),
        }
    }

    // ----- the picture pipeline (2.6) -----

    /// Picture pixels per style-length unit of a stamp (2.3): image 1, text `size_px / 128`, sticker its render scale.
    pub(crate) fn ref_scale(&self, object: &StampObject) -> f32 {
        match &object.source {
            StampSource::Image { .. } => 1.0,
            StampSource::Text(spec) => raster_px(spec) / STYLE_REFERENCE_EM,
            StampSource::Sticker { sticker } => self
                .sticker_state
                .map
                .get(sticker)
                .map_or(1.0, |doc| sticker_scale(doc, self.texels_of(object).y, BakeQuality::Full)),
        }
    }

    /// Texels the stored placement of `object` covers per axis (`placement_texels`).
    pub(crate) fn texels_of(&self, object: &StampObject) -> glam::Vec2 {
        placement_texels(&object.placement, self.texel_world_size(object.home_set))
    }

    /// The styling of a styled stamp for a bake of `quality` (2.6 steps 2-4).
    fn styling_of(&self, object: &StampObject, ref_scale: f32, quality: BakeQuality) -> Styling {
        Styling { styles: object.styles.sanitized(), tint: object.tint, ref_scale, texels: self.texels_of(object), quality }
    }

    /// The styled entry of `stamp` when it was made for the stamp's current source, styles and tint (any quality).
    fn current_styled_entry(&self, stamp: &StampObject) -> Option<&StyledEntry> {
        let entry = self.sticker_state.cache.styled.get(&stamp.id)?;
        let base = match &stamp.source {
            StampSource::Image { blob } => BaseIdentity::Blob(*blob),
            StampSource::Text(spec) => match self.text_mips.get(&stamp.id) {
                Some((cached, mips)) if cached == spec => BaseIdentity::Text(Arc::clone(mips)),
                _ => return None,
            },
            StampSource::Sticker { sticker } => BaseIdentity::Sticker(Arc::clone(self.sticker_state.map.get(sticker)?), 0),
        };
        (entry.base.same_source(&base) && entry.styles == stamp.styles.sanitized() && entry.tint == stamp.tint).then_some(entry)
    }

    /// The picture a bake of stamp `id` at `quality` places (2.6), on the calling thread, from the caches when they
    /// hold it. Image and text stamps without styles: their mips as before (bit-identical path), the stamp's tint,
    /// margin 0.
    pub(crate) fn bake_picture(&mut self, id: StampId, quality: BakeQuality) -> Result<BakePicture, EditError> {
        let stamp = self.stamp(id).ok_or(EditError::UnknownStamp)?.clone();
        let plain = |mips: Arc<PremulMips>, tint| {
            let frame = SourceFrame::plain(mips.size());
            BakePicture { mips, frame, tint }
        };
        let (base, identity, ref_scale) = match &stamp.source {
            StampSource::Image { blob } => {
                let mips = self.image_source(*blob)?;
                if !styled(&stamp) {
                    return Ok(plain(mips, stamp.tint));
                }
                (mips, BaseIdentity::Blob(*blob), 1.0)
            }
            StampSource::Text(spec) => {
                let mips = self.text_source(id, spec, crate::doc_stamps::raster_quality(quality))?;
                if !styled(&stamp) {
                    return Ok(plain(mips, stamp.tint));
                }
                let identity = BaseIdentity::Text(Arc::clone(&mips));
                // Check P3a: the Preview picture of a large styled text is drawn smaller than its frame (FEATURE_TEXT
                // 5.8, `full == false`); the style lengths shrink with it, so the Preview looks like the Final.
                let shrink = match self.cached_text_frame(id).map(|f| f.size) {
                    Some([fw, _]) if fw > 0 && mips.size()[0] != fw => mips.size()[0] as f32 / fw as f32,
                    _ => 1.0,
                };
                (mips, identity, raster_px(spec) / STYLE_REFERENCE_EM * shrink)
            }
            StampSource::Sticker { sticker } => {
                let doc = Arc::clone(self.sticker_state.map.get(sticker).ok_or(EditError::UnknownSticker)?);
                let s = sticker_scale(&doc, self.texels_of(&stamp).y, quality);
                let (mips, used) = self.sticker_base(&doc, s, quality)?;
                if !styled(&stamp) {
                    return Ok(plain(mips, stamp.tint));
                }
                (mips, BaseIdentity::Sticker(doc, used.to_bits()), used)
            }
        };
        let styling = self.styling_of(&stamp, ref_scale, quality);
        let (_, want_scale, _) = work_level(base.size(), &styling);
        if let Some(entry) = self.sticker_state.cache.styled.get(&id)
            && entry.serves(&identity, &styling, want_scale)
        {
            return Ok(BakePicture { mips: Arc::clone(&entry.mips), frame: entry.frame, tint: [255; 4] });
        }
        let (mips, frame, scale) = styled_mips(&base, &styling);
        let mips = Arc::new(mips);
        let entry = StyledEntry {
            base: identity,
            styles: styling.styles,
            tint: styling.tint,
            scale_bits: scale.to_bits(),
            quality,
            frame,
            mips: Arc::clone(&mips),
        };
        self.sticker_state.cache.styled.insert(id, entry);
        Ok(BakePicture { mips, frame, tint: [255; 4] })
    }

    /// The base mips of a sticker stamp at render scale `s` and the scale they have: the image's own mips for a promoted
    /// image (4.1 trivial fast path at scale 1: nothing is rendered, also for a Preview, whose bake samples the mip
    /// level it needs), else a cached render (a Preview takes any render of this value), else a new render.
    pub(crate) fn sticker_base(&mut self, doc: &Arc<StickerDoc>, s: f32, quality: BakeQuality) -> Result<(Arc<PremulMips>, f32), EditError> {
        if let Some(mips) = self.trivial_mips(doc) {
            return Ok((mips, 1.0));
        }
        let exact = quality == BakeQuality::Full;
        if let Some(e) = self.sticker_state.cache.find_render(doc, s, quality, exact) {
            return Ok((e.mips, f32::from_bits(e.scale_bits)));
        }
        let entry = self.sticker_render(doc, s, quality)?;
        Ok((entry.mips, s))
    }

    /// The image mips of a sticker on the trivial fast path (a promoted image: bit for bit the image stamp's picture at
    /// render scale 1), else `None`.
    pub(crate) fn trivial_mips(&mut self, doc: &StickerDoc) -> Option<Arc<PremulMips>> {
        let blob = trivial_blob(doc)?;
        let mips = self.image_source(blob).ok()?;
        (mips.size() == doc.size()).then_some(mips)
    }

    /// The render of `doc` at scale `s` and `quality` (exactly), from the render cache or drawn here (layer cache used
    /// and filled).
    pub(crate) fn sticker_render(&mut self, doc: &Arc<StickerDoc>, s: f32, quality: BakeQuality) -> Result<RenderEntry, EditError> {
        if let Some(e) = self.sticker_state.cache.find_render(doc, s, quality, true) {
            return Ok(e);
        }
        // Check P3a: the trivial fast path (4.1) at scale 1 shares the picture the document holds already (a promoted
        // photo's decoded pixels and its image mips) instead of copying it and building its mips again (an 8192 px
        // photo: about 600 MB per finished picture). Same bytes as `render_with`'s fast path.
        if s == 1.0
            && let Some(picture) = doc.trivial_picture()
        {
            let shared = match picture {
                PicturePixels::Blob(b) => {
                    let pixels = self.blob_picture(*b).ok();
                    let mips = self.trivial_mips(doc);
                    pixels.zip(mips)
                }
                PicturePixels::Owned { pixels, .. } if [pixels.width, pixels.height] == doc.size() => {
                    Some((Arc::clone(pixels), Arc::new(PremulMips::build(pixels))))
                }
                PicturePixels::Owned { .. } => None,
            };
            if let Some((picture, mips)) = shared
                && [picture.width, picture.height] == doc.size()
            {
                let entry = RenderEntry { doc: Arc::clone(doc), scale_bits: s.to_bits(), quality, picture, mips };
                self.sticker_state.cache.insert_render(entry.clone());
                return Ok(entry);
            }
        }
        let input = self.sticker_render_input(doc)?;
        let rq = crate::doc_stamps::raster_quality(quality);
        let picture = render_with(&input, s, rq, Some(&mut self.sticker_state.cache.layers));
        let mips = Arc::new(PremulMips::build(&picture));
        let entry = RenderEntry { doc: Arc::clone(doc), scale_bits: s.to_bits(), quality, picture: Arc::new(picture), mips };
        self.sticker_state.cache.insert_render(entry.clone());
        Ok(entry)
    }

    /// Everything `sticker::render` needs for `doc` (2.7): decoded pictures of its blob layers (a missing or broken
    /// blob is left out with a warning: that layer draws nothing) and the fonts of its text layers.
    pub(crate) fn sticker_render_input(&mut self, doc: &Arc<StickerDoc>) -> Result<StickerRenderInput, EditError> {
        let mut pictures = Vec::new();
        let mut fonts = Vec::new();
        for layer in &doc.layers {
            match &layer.content {
                StickerContent::Picture(PicturePixels::Blob(b)) => {
                    if pictures.iter().any(|(id, _)| id == b) {
                        continue;
                    }
                    match self.blob_picture(*b) {
                        Ok(p) => pictures.push((*b, p)),
                        Err(e) => log::warn!("sticker layer {}: picture {} cannot be shown: {e}", layer.id.0, b.0),
                    }
                }
                StickerContent::Text(spec) if layer.visible => {
                    let (own, fallback) = self.text_fonts_owned(spec)?;
                    fonts.push((layer.id, own, fallback));
                }
                _ => {}
            }
        }
        Ok(StickerRenderInput { doc: Arc::clone(doc), pictures, fonts })
    }

    /// The decoded pixels (straight RGBA8, at most 8192 px a side) of an image blob, cached.
    pub(crate) fn blob_picture(&mut self, blob: BlobId) -> Result<Arc<PixelBuf>, EditError> {
        if let Some(p) = self.sticker_state.cache.blob_pictures.get(&blob) {
            return Ok(Arc::clone(p));
        }
        let stored = self.blobs.get(blob).filter(|b| b.kind == BlobKind::Image).ok_or(EditError::UnknownBlob)?;
        let decoded = decode_image(&stored.bytes, Some(&stored.name), MAX_PICTURE_SIDE)?;
        let p = Arc::new(decoded.pixels);
        self.sticker_state.cache.blob_pictures.insert(blob, Arc::clone(&p));
        Ok(p)
    }

    /// The render scale of stamp `object`'s sticker for a bake of `quality` (2.6).
    pub(crate) fn sticker_scale_of(&self, object: &StampObject, doc: &StickerDoc, quality: BakeQuality) -> f32 {
        sticker_scale(doc, self.texels_of(object).y, quality)
    }

    /// Drops styled pictures of stamps that no longer exist.
    pub(crate) fn prune_sticker_caches(&mut self) {
        let stamps = &self.stamps;
        self.sticker_state.cache.styled.retain(|id, _| stamps.iter().any(|s| s.id == *id));
    }

    // ----- internals -----

    /// The sticker `on` names. `OfStamp` of a sticker stamp is its sticker; of another stamp `InvalidParam` (an image
    /// stamp is promoted by the content commands before this is asked: `promotion_of`).
    pub(crate) fn resolve_sticker(&self, on: StickerRef) -> Result<StickerId, EditError> {
        match on {
            StickerRef::Id(id) => self.sticker_state.map.contains_key(&id).then_some(id).ok_or(EditError::UnknownSticker),
            StickerRef::OfStamp(stamp, _) => match self.stamp(stamp).ok_or(EditError::UnknownStamp)?.source {
                StampSource::Sticker { sticker } => Ok(sticker),
                _ => Err(EditError::InvalidParam("the stamp does not show a sticker")),
            },
        }
    }

    /// Gives every owned picture and mask of `doc` that is not one of `old`'s buffers (same `Arc` and version) a fresh
    /// version (2.2: a new buffer always gets a new number; two layers sharing one buffer share its number).
    pub(crate) fn version_new_buffers(&mut self, doc: &mut StickerDoc, old: Option<&StickerDoc>) {
        let mut given: Vec<(usize, u64)> = Vec::new();
        let known_picture = |p: &Arc<PixelBuf>, v: u64| {
            old.is_some_and(|o| o.layers.iter().any(|l| matches!(&l.content, StickerContent::Picture(PicturePixels::Owned { pixels, version }) if Arc::ptr_eq(pixels, p) && *version == v)))
        };
        let known_mask = |m: &Arc<crate::pixel::AlphaBuf>, v: u64| {
            old.is_some_and(|o| o.layers.iter().any(|l| l.mask.as_ref().is_some_and(|k| Arc::ptr_eq(&k.pixels, m) && k.version == v)))
        };
        for layer in &mut doc.layers {
            if let StickerContent::Picture(PicturePixels::Owned { pixels, version }) = &mut layer.content
                && !known_picture(pixels, *version)
            {
                let key = Arc::as_ptr(pixels) as usize;
                *version = match given.iter().find(|g| g.0 == key) {
                    Some(g) => g.1,
                    None => {
                        let v = self.next_pixel_version();
                        given.push((key, v));
                        v
                    }
                };
            }
            if let Some(mask) = &mut layer.mask
                && !known_mask(&mask.pixels, mask.version)
            {
                let key = Arc::as_ptr(&mask.pixels) as usize;
                mask.version = match given.iter().find(|g| g.0 == key) {
                    Some(g) => g.1,
                    None => {
                        let v = self.next_pixel_version();
                        given.push((key, v));
                        v
                    }
                };
            }
        }
    }

    /// The limits of 2.7 for `doc`, with the decoded sizes of its blob pictures (`UnknownBlob` for a missing one; a mask
    /// on a blob picture must have the picture's size).
    pub(crate) fn check_sticker(&mut self, doc: &StickerDoc) -> Result<(), EditError> {
        doc.check_limits()?;
        for layer in &doc.layers {
            if let StickerContent::Picture(PicturePixels::Blob(b)) = &layer.content {
                let size = self.image_size(*b)?;
                if let Some(mask) = &layer.mask
                    && [mask.pixels.width, mask.pixels.height] != size
                {
                    return Err(EditError::InvalidParam("a layer mask must have the size of its picture"));
                }
            }
        }
        Ok(())
    }
}
