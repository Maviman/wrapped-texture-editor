// Owner: core-document (C2). Layer commands of the document (ARCHITECTURE.md 5.17): add / remove / duplicate / move /
// merge / rasterize, the layer properties and the thumbnails. Every command follows the mutation rule of 5.14 and
// pushes one `Edit::Structure`; a call that would change nothing returns `Ok` without a history entry.

use std::sync::Arc;

use rayon::prelude::*;

use crate::blend::blend_row;
use crate::composite::opacity_u8;
use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::ids::{LayerId, StampId, TextureSetId};
use crate::layer::{BlendMode, Layer, LayerKind};
use crate::math::IRect;
use crate::pixel::PixelBuf;
use crate::pixel_ops::{thumbnail, thumbnail_with};

/// Whether the active layer of a set can be painted on, and if not, why.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PaintTarget {
    Ready(LayerId),
    NoActiveLayer,
    NotRaster(LayerId),
    Locked(LayerId),
    Hidden(LayerId),
}

/// History label of the opacity slider; all `Commit::Preview` calls of one drag share it.
const OPACITY_LABEL: &str = "Layer Opacity";

impl Document {
    /// The paint target of a set: its active layer if that is a visible, unlocked raster layer, else the reason.
    pub fn paint_target(&self, set: TextureSetId) -> PaintTarget {
        let Some(layer) = self.set(set).and_then(|s| s.active_layer()) else {
            return PaintTarget::NoActiveLayer;
        };
        if !layer.is_raster() {
            PaintTarget::NotRaster(layer.id)
        } else if layer.locked {
            PaintTarget::Locked(layer.id)
        } else if !layer.visible {
            PaintTarget::Hidden(layer.id)
        } else {
            PaintTarget::Ready(layer.id)
        }
    }

    /// New transparent raster layer above the active layer (top if none); becomes active. Name None => "Paint N"
    /// (N = smallest positive integer not used by a "Paint N" layer of that set).
    pub fn add_raster_layer(&mut self, set: TextureSetId, name: Option<&str>) -> Result<LayerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let target = self.set(set).ok_or(EditError::UnknownSet)?;
        let name = match name {
            Some(name) => name.to_owned(),
            None => {
                let used: Vec<u32> = target
                    .layers
                    .iter()
                    .filter_map(|l| l.name.strip_prefix("Paint ")?.parse::<u32>().ok())
                    .collect();
                let n = (1u32..).find(|n| !used.contains(n)).unwrap_or(1);
                format!("Paint {n}")
            }
        };
        let index = target.active.and_then(|a| target.layer_index(a)).map_or(target.layers.len(), |i| i + 1);

        let before = self.snapshot_structure(&[]);
        let id = LayerId(self.alloc_id());
        // Unallocated until the first stroke or fill (`LayerKind::Raster`): a new layer costs no memory.
        let mut layer = Layer::new(id, &name, LayerKind::empty_raster());
        layer.version = self.fresh_version();
        layer.content = Some((layer.version, IRect::EMPTY));
        let target = &mut self.sets[set.index()];
        target.layers.insert(index, layer);
        target.active = Some(id);
        // An all-zero layer never changes the composite (alpha-0 rule, 6.6): nothing to mark dirty.
        self.push_structure_edit("Add Layer", before);
        Ok(id)
    }

    /// Removes a layer (Err(LayerLocked) if it is locked). A stamp/text layer removes the whole stamp (all linked layers in
    /// all sets): exactly `return self.remove_stamp(stamp)`. The layer below (else above, else None) becomes active.
    ///
    /// The active layer changes only when the removed layer was the active one.
    pub fn remove_layer(&mut self, layer: LayerId) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let target = &self.sets[set.index()].layers[index];
        if target.locked {
            return Err(EditError::LayerLocked);
        }
        if let Some(stamp) = target.stamp_id() {
            return self.remove_stamp(stamp);
        }

        let before = self.snapshot_structure(&[layer]);
        let target = &mut self.sets[set.index()];
        target.mark_layer_content_dirty(index);
        target.layers.remove(index);
        if target.active == Some(layer) {
            // `index - 1` is the layer below; after the removal `index` is the layer that was above.
            let neighbour = index.checked_sub(1).or(if target.layers.is_empty() { None } else { Some(0) });
            target.active = neighbour.map(|i| target.layers[i].id);
        }
        self.push_structure_edit("Delete Layer", before);
        Ok(())
    }

    /// Raster: copies pixels into a new unlocked layer "<name> copy" directly above. Stamp/text: exactly
    /// `let s = self.duplicate_stamp(stamp)?;` then the new stamp's linked layer in the same set is returned.
    ///
    /// The raster copy keeps visibility, opacity, blend mode and alpha lock, and becomes the active layer.
    pub fn duplicate_layer(&mut self, layer: LayerId) -> Result<LayerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let source = &self.sets[set.index()].layers[index];
        let pixels = match &source.kind {
            // A real copy now: a shared buffer would be copied by the first dab on either layer, in mid-stroke.
            LayerKind::Raster(buf) => Arc::new(PixelBuf::clone(buf)),
            LayerKind::Stamp { stamp, .. } | LayerKind::Text { stamp, .. } => {
                let s = self.duplicate_stamp(*stamp)?;
                return self
                    .stamp_layers(s)
                    .into_iter()
                    .find(|(linked_set, _)| *linked_set == set)
                    .map(|(_, id)| id)
                    .ok_or(EditError::UnknownLayer);
            }
        };
        let known = source.known_content();
        let mut copy = source.clone();
        copy.name = format!("{} copy", source.name);
        copy.locked = false;
        copy.kind = LayerKind::Raster(pixels);

        let before = self.snapshot_structure(&[]);
        let id = LayerId(self.alloc_id());
        copy.id = id;
        copy.version = self.fresh_version();
        copy.content = known.map(|r| (copy.version, r));
        let target = &mut self.sets[set.index()];
        target.layers.insert(index + 1, copy);
        target.active = Some(id);
        target.mark_layer_content_dirty(index + 1);
        self.push_structure_edit("Duplicate Layer", before);
        Ok(id)
    }

    /// Moves a layer to `to_index` within its set (clamped).
    pub fn move_layer(&mut self, layer: LayerId, to_index: usize) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let (set, from) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let to = to_index.min(self.sets[set.index()].layers.len() - 1);
        if to == from {
            return Ok(());
        }
        let before = self.snapshot_structure(&[]);
        let target = &mut self.sets[set.index()];
        let moved = target.layers.remove(from);
        target.layers.insert(to, moved);
        // The other layers keep their relative order: the composite can change only where the moved layer has content
        // (an all-zero pixel never changes the fold, whatever its position, 6.6).
        target.mark_layer_content_dirty(to);
        self.push_structure_edit("Move Layer", before);
        Ok(())
    }

    /// Merges `layer` into the layer below (must be an unlocked raster; `layer` itself must be unlocked). Returns the surviving layer id.
    ///
    /// Pixel rule (6.5): `below' = blend_pixel(below, top, round(top.opacity * 255), top.blend)`; the result keeps
    /// the lower layer's id, name, opacity and blend. A stamp/text layer is rasterized and merged in one entry.
    pub fn merge_down(&mut self, layer: LayerId) -> Result<LayerId, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        self.bake_stamp_of_layer(layer);
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let target = &self.sets[set.index()];
        let top = &target.layers[index];
        if top.locked {
            return Err(EditError::LayerLocked);
        }
        let Some(below) = index.checked_sub(1).map(|i| &target.layers[i]) else {
            return Err(EditError::CannotMerge("there is no layer below"));
        };
        let LayerKind::Raster(below_pixels) = &below.kind else {
            return Err(EditError::CannotMerge("the layer below is not a paint layer"));
        };
        if below.locked {
            return Err(EditError::CannotMerge("the layer below is locked"));
        }
        let below_id = below.id;
        let stamp = top.stamp_id();

        // Pinned: the lower layer (its pixels are replaced) and, if raster, the upper one (it disappears).
        let pins = if top.is_raster() { vec![below_id, layer] } else { vec![below_id] };
        let before = self.snapshot_structure(&pins);

        // A new buffer: the old one stays untouched in the snapshot (an unallocated lower layer gets its first one).
        let (width, height) = (target.width, target.height);
        let mut merged =
            if below_pixels.data.is_empty() { PixelBuf::new(width, height) } else { PixelBuf::clone(below_pixels) };
        let bound = top.content_rect();
        let (opacity, blend) = (opacity_u8(top.opacity), top.blend);
        let mut rects = Vec::new();
        for view in top.views() {
            let rect = view.rect.intersect(&bound).intersect(&merged.rect());
            if rect.is_empty() {
                continue;
            }
            rects.push(rect);
            let stride = merged.width as usize * 4;
            let (x0, x1) = (rect.x0 as usize * 4, rect.x1 as usize * 4);
            let skip = (rect.x0 - view.rect.x0) as usize * 4;
            merged.data[rect.y0 as usize * stride..rect.y1 as usize * stride].par_chunks_mut(stride).enumerate().for_each(
                |(i, row)| {
                    if let Some(src) = view.row(rect.y0 + i as i32) {
                        blend_row(&mut row[x0..x1], &src[skip..skip + (x1 - x0)], opacity, blend);
                    }
                },
            );
        }

        let target = &mut self.sets[set.index()];
        target.layers[index - 1].kind = LayerKind::Raster(Arc::new(merged));
        target.layers.remove(index);
        if target.active == Some(layer) {
            target.active = Some(below_id);
        }
        if let Some(stamp) = stamp {
            self.unlink_stamp(stamp, set);
        }
        // The lower layer changed, and the upper one vanished, only inside the upper layer's content.
        self.mark_layer_rects_dirty(below_id, &rects);
        self.push_structure_edit("Merge Down", before);
        Ok(below_id)
    }

    /// Converts a stamp/text layer into a raster layer (loses re-editability in that set). Err(LayerLocked) if locked.
    ///
    /// The layer keeps its id, name and properties. If the stamp has no other linked layer it is removed; if the
    /// layer was the one in the stamp's home set, the home set moves to the lowest remaining set id (6.5).
    /// A paint layer is `Err(InvalidParam)`.
    pub fn rasterize_layer(&mut self, layer: LayerId) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        self.bake_stamp_of_layer(layer);
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let target = &self.sets[set.index()];
        let source = &target.layers[index];
        if source.locked {
            return Err(EditError::LayerLocked);
        }
        let (stamp, dense, bound) = match &source.kind {
            LayerKind::Raster(_) => return Err(EditError::InvalidParam("the layer is already a paint layer")),
            LayerKind::Stamp { stamp, cache } | LayerKind::Text { stamp, cache } => {
                (*stamp, cache.to_dense(target.width, target.height), cache.bounds())
            }
        };

        let before = self.snapshot_structure(&[]);
        let version = self.fresh_version();
        let converted = &mut self.sets[set.index()].layers[index];
        converted.kind = LayerKind::Raster(Arc::new(dense));
        converted.version = version;
        converted.content = Some((version, bound));
        self.unlink_stamp(stamp, set);
        // Same pixels as before: the composite does not change.
        self.push_structure_edit("Rasterize Layer", before);
        Ok(())
    }

    /// Round 5, rule 8: before a command reads the cache of a stamp/text layer to keep it as pixels, the layer's stamp
    /// gets its Full bake here when it is still unbaked (deferred Final bakes). A paint layer or an unknown id: no-op.
    /// The bake may remove the layer itself (an empty layer outside the home set); the command then sees an unknown
    /// layer, as it would have after the bake.
    fn bake_stamp_of_layer(&mut self, layer: LayerId) {
        if let Some(stamp) = self.stamp_of_layer(layer) {
            self.bake_unbaked_now(stamp);
        }
    }

    /// Bookkeeping after the linked layer of `stamp` in `set` was rasterized or merged away: a stamp without linked
    /// layers is removed; one that lost the layer of its home set gets the lowest remaining set as its home.
    fn unlink_stamp(&mut self, stamp: StampId, set: TextureSetId) {
        let remaining = self.stamp_layers(stamp);
        match remaining.first() {
            None => {
                self.stamps.retain(|s| s.id != stamp);
                self.text_mips.remove(&stamp);
                self.preview_baked.remove(&stamp);
                self.bakes.unbaked.remove(&stamp);
            }
            Some(&(lowest, _)) => {
                if let Some(object) = self.stamps.iter_mut().find(|s| s.id == stamp)
                    && object.home_set == set
                {
                    object.home_set = lowest;
                }
            }
        }
    }

    /// Shared body of the on/off layer properties: `Ok` without a history entry when nothing changes.
    fn set_layer_flag(
        &mut self,
        layer: LayerId,
        label: &str,
        affects_composite: bool,
        get: impl Fn(&Layer) -> bool,
        set: impl Fn(&mut Layer, bool),
        value: bool,
    ) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let (set_id, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        if get(&self.sets[set_id.index()].layers[index]) == value {
            return Ok(());
        }
        let before = self.snapshot_structure(&[]);
        let target = &mut self.sets[set_id.index()];
        set(&mut target.layers[index], value);
        if affects_composite {
            target.mark_layer_content_dirty(index);
        }
        self.push_structure_edit(label, before);
        Ok(())
    }

    /// Shows or hides a layer.
    pub fn set_layer_visible(&mut self, layer: LayerId, visible: bool) -> Result<(), EditError> {
        let label = if visible { "Show Layer" } else { "Hide Layer" };
        self.set_layer_flag(layer, label, true, |l| l.visible, |l, v| l.visible = v, visible)
    }

    /// Locks or unlocks a layer.
    pub fn set_layer_locked(&mut self, layer: LayerId, locked: bool) -> Result<(), EditError> {
        let label = if locked { "Lock Layer" } else { "Unlock Layer" };
        self.set_layer_flag(layer, label, false, |l| l.locked, |l, v| l.locked = v, locked)
    }

    /// Switches a layer's alpha lock.
    pub fn set_layer_alpha_lock(&mut self, layer: LayerId, on: bool) -> Result<(), EditError> {
        let label = if on { "Lock Alpha" } else { "Unlock Alpha" };
        self.set_layer_flag(layer, label, false, |l| l.alpha_lock, |l, v| l.alpha_lock = v, on)
    }

    /// Changes a layer's blend mode.
    pub fn set_layer_blend(&mut self, layer: LayerId, blend: BlendMode) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        if self.sets[set.index()].layers[index].blend == blend {
            return Ok(());
        }
        let before = self.snapshot_structure(&[]);
        let target = &mut self.sets[set.index()];
        target.layers[index].blend = blend;
        target.mark_layer_content_dirty(index);
        self.push_structure_edit("Blend Mode", before);
        Ok(())
    }

    /// Changes a layer's opacity (0..=1); `Commit::Preview` calls are folded into one history entry.
    ///
    /// The value is clamped to 0..=1; a non-finite value is `Err(InvalidParam)`. `Commit::Final` is by construction
    /// "`Commit::Preview` followed by `flush_pending_edit()`".
    pub fn set_layer_opacity(&mut self, layer: LayerId, opacity: f32, commit: Commit) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        let continues = self.pending_edit.as_ref().is_some_and(|p| p.label == OPACITY_LABEL);
        if !continues {
            self.flush_pending_edit();
        }
        if !opacity.is_finite() {
            return Err(EditError::InvalidParam("opacity must be a finite number"));
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let unchanged = self.sets[set.index()].layers[index].opacity == opacity;
        if unchanged && !continues {
            // Nothing to do and no drag to end: no pending edit, no history entry.
            return Ok(());
        }
        self.begin_pending(OPACITY_LABEL);
        if !unchanged {
            let target = &mut self.sets[set.index()];
            target.layers[index].opacity = opacity;
            target.mark_layer_content_dirty(index);
        }
        if commit == Commit::Final {
            self.flush_pending_edit();
        }
        Ok(())
    }

    /// Renames a layer (a stamp/text layer renames the whole stamp: `rename_stamp`).
    pub fn rename_layer(&mut self, layer: LayerId, name: &str) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let target = &self.sets[set.index()].layers[index];
        if let Some(stamp) = target.stamp_id() {
            return self.rename_stamp(stamp, name);
        }
        if target.name == name {
            return Ok(());
        }
        let before = self.snapshot_structure(&[]);
        self.sets[set.index()].layers[index].name = name.to_owned();
        self.push_structure_edit("Rename Layer", before);
        Ok(())
    }

    /// Selects the active layer of a set. No history, no revision bump.
    pub fn set_active_layer(&mut self, set: TextureSetId, layer: Option<LayerId>) -> Result<(), EditError> {
        self.guard_no_stroke()?;
        let target = self.set_mut(set).ok_or(EditError::UnknownSet)?;
        if let Some(id) = layer
            && target.layer(id).is_none()
        {
            return Err(EditError::UnknownLayer);
        }
        target.active = layer;
        Ok(())
    }

    /// Thumbnail of one layer (longest side = `side`).
    pub fn layer_thumbnail(&self, layer: LayerId, side: u32) -> Option<PixelBuf> {
        let (set, index) = self.find_layer(layer)?;
        let set = &self.sets[set.index()];
        let layer = &set.layers[index];
        Some(thumbnail_with(set.width, set.height, side, |x, y| layer.get(x, y)))
    }

    /// Thumbnail of a set, from the composite.
    pub fn set_thumbnail(&self, set: TextureSetId, side: u32) -> Option<PixelBuf> {
        let set = self.set(set)?;
        Some(thumbnail(set.composite.view(), set.width, set.height, side))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::blend_pixel;
    use crate::composite::flatten_layers;
    use crate::history::Edit;
    use crate::history::testkit::*;
    use crate::samples;
    use crate::testutil::{XorShift, doc_from_raw};

    const S0: TextureSetId = TextureSetId(0);
    const S1: TextureSetId = TextureSetId(1);

    /// Runs `command` and checks: exactly one `Edit::Structure` entry and one revision bump, a changed document,
    /// `undo` restores the state from before byte for byte, `redo` the state from after. Leaves the "after" state.
    fn check_undoable<T>(doc: &mut Document, what: &str, command: impl FnOnce(&mut Document) -> Result<T, EditError>) -> T {
        let before = doc_state(doc);
        let (entries, revision) = (doc.history.undo_len(), doc.revision());
        let value = match command(doc) {
            Ok(value) => value,
            Err(e) => panic!("{what}: {e}"),
        };
        assert_eq!(doc.history.undo_len(), entries + 1, "{what}: exactly one history entry");
        assert_eq!(doc.revision(), revision + 1, "{what}: one revision bump");
        assert!(matches!(doc.history.undo.last().unwrap().edit, Edit::Structure(_)), "{what}: a structure entry");
        let after = doc_state(doc);
        assert_ne!(after, before, "{what}: the document changed");
        assert!(doc.undo().is_some(), "{what}: undo");
        assert_eq!(doc_state(doc), before, "{what}: undo restores the document");
        assert!(doc.redo().is_some(), "{what}: redo");
        assert_eq!(doc_state(doc), after, "{what}: redo restores the result");
        value
    }

    fn ids(doc: &Document, set: TextureSetId) -> Vec<u64> {
        doc.set(set).unwrap().layers.iter().map(|l| l.id.0).collect()
    }

    fn all_layers(doc: &Document) -> Vec<LayerId> {
        doc.sets.iter().flat_map(|s| &s.layers).map(|l| l.id).collect()
    }

    fn assert_rasters_unpinned(doc: &Document, what: &str) {
        for layer in doc.sets.iter().flat_map(|s| &s.layers).filter(|l| l.is_raster()) {
            assert_eq!(Arc::strong_count(raster_arc(doc, layer.id)), 1, "{what}: layer {:?} is pinned", layer.id);
        }
    }

    #[test]
    fn every_layer_command_is_undoable() {
        let mut doc = doc_with_stamp();
        paint_rect(&mut doc, LayerId(3), IRect::new(20, 20, 70, 60), [200, 40, 40, 180]);
        paint_rect(&mut doc, LayerId(1), IRect::new(0, 0, 40, 96), [30, 90, 200, 255]);
        doc.history.clear();
        let initial = doc_state(&mut doc);
        let d = &mut doc;

        let added = check_undoable(d, "add", |d| d.add_raster_layer(S0, None));
        assert_eq!(ids(d, S0), [1, 101, 3, added.0]);
        check_undoable(d, "add named", |d| d.add_raster_layer(S1, Some("Custom")));
        check_undoable(d, "hide", |d| d.set_layer_visible(LayerId(1), false));
        check_undoable(d, "show", |d| d.set_layer_visible(LayerId(1), true));
        check_undoable(d, "hide stamp layer", |d| d.set_layer_visible(LayerId(102), false));
        check_undoable(d, "show stamp layer", |d| d.set_layer_visible(LayerId(102), true));
        check_undoable(d, "lock", |d| d.set_layer_locked(LayerId(3), true));
        check_undoable(d, "unlock", |d| d.set_layer_locked(LayerId(3), false));
        check_undoable(d, "alpha lock", |d| d.set_layer_alpha_lock(LayerId(3), true));
        check_undoable(d, "blend", |d| d.set_layer_blend(LayerId(3), BlendMode::Multiply));
        check_undoable(d, "stamp blend", |d| d.set_layer_blend(LayerId(101), BlendMode::Screen));
        check_undoable(d, "opacity", |d| d.set_layer_opacity(LayerId(3), 0.4, Commit::Final));
        check_undoable(d, "stamp opacity", |d| d.set_layer_opacity(LayerId(101), 0.7, Commit::Final));
        check_undoable(d, "rename", |d| d.rename_layer(LayerId(3), "Renamed"));
        check_undoable(d, "move", |d| d.move_layer(LayerId(3), 0));
        assert_eq!(ids(d, S0), [3, 1, 101, added.0]);
        check_undoable(d, "move stamp layer", |d| d.move_layer(LayerId(101), 99));
        assert_eq!(ids(d, S0), [3, 1, added.0, 101]);
        let copy = check_undoable(d, "duplicate", |d| d.duplicate_layer(LayerId(3)));
        assert_eq!(ids(d, S0), [3, copy.0, 1, added.0, 101]);
        check_undoable(d, "remove", |d| d.remove_layer(copy));
        check_undoable(d, "merge paint layers", |d| d.merge_down(LayerId(1)));
        assert_eq!(ids(d, S0), [3, added.0, 101]);
        check_undoable(d, "merge stamp layer", |d| d.merge_down(LayerId(101)));
        assert_eq!(ids(d, S0), [3, added.0]);
        assert_eq!(d.stamps[0].home_set, S1);
        check_undoable(d, "rasterize", |d| d.rasterize_layer(LayerId(102)));
        assert!(d.stamps.is_empty());
        check_undoable(d, "remove base", |d| d.remove_layer(LayerId(2)));

        // Undo everything, then redo everything.
        let last = doc_state(&mut doc);
        let n = doc.history.undo_len();
        assert_eq!(n, 22);
        while doc.undo().is_some() {}
        assert_eq!(doc_state(&mut doc), initial);
        assert_eq!(doc.history.redo_len(), n);
        while doc.redo().is_some() {}
        assert_eq!(doc_state(&mut doc), last);
        assert_rasters_unpinned(&doc, "after redo-all the history pins only replaced buffers");
    }

    #[test]
    fn random_command_sequences_undo_and_redo_byte_identically() {
        for seed in [1u64, 2, 3] {
            let mut rng = XorShift::new(seed);
            let mut doc = doc_with_stamp();
            doc.history.max_entries = 100_000;
            // states[k] = the document with k history entries and no pending edit; `preview` = its state while a
            // Preview edit is pending.
            let mut states = vec![doc_state(&mut doc)];
            let mut preview: Option<DocState> = None;

            for step in 0..350 {
                let (n_before, pending_before) = (doc.history.undo_len(), doc.has_pending_edit());
                let layers = all_layers(&doc);
                let layer = layers.get(rng.next_u32() as usize % layers.len().max(1)).copied().unwrap_or(LayerId(0));
                let is_raster = doc.layer(layer).is_some_and(|l| l.is_raster());
                // Steps that may change the document without adding an entry.
                let mut free_form = false;
                // Successful undo / redo / cancel.
                let (mut moved, mut undone) = (false, false);
                match rng.next_u32() % 22 {
                    0..=3 => {
                        if is_raster {
                            let x0 = (rng.next_u32() % SIZE) as i32 - 8;
                            let y0 = (rng.next_u32() % SIZE) as i32 - 8;
                            let r = IRect::new(x0, y0, x0 + 1 + (rng.next_u32() % 50) as i32, y0 + 1 + (rng.next_u32() % 50) as i32);
                            // A real stroke flushes the pending edit in stroke_begin.
                            doc.flush_pending_edit();
                            paint_rect(&mut doc, layer, r, rng.next_u32().to_le_bytes());
                        }
                    }
                    4 => {
                        if layers.len() < 12 {
                            let _ = doc.add_raster_layer(TextureSetId(rng.next_u32() % 2), None);
                        }
                    }
                    5 => {
                        // Stamp layers delegate to doc_stamps.rs (another package): paint layers only.
                        if is_raster {
                            let _ = doc.remove_layer(layer);
                        }
                    }
                    6 => {
                        if is_raster && layers.len() < 12 {
                            let _ = doc.duplicate_layer(layer);
                        }
                    }
                    7 => {
                        let _ = doc.move_layer(layer, rng.next_u32() as usize % 6);
                    }
                    8 => {
                        let _ = doc.merge_down(layer);
                    }
                    9 => {
                        let _ = doc.rasterize_layer(layer);
                    }
                    10 => {
                        let _ = doc.set_layer_visible(layer, rng.next_u32().is_multiple_of(2));
                    }
                    11 => {
                        let _ = doc.set_layer_blend(layer, BlendMode::ALL[rng.next_u32() as usize % 7]);
                    }
                    12 | 13 => {
                        let commit = if rng.next_u32().is_multiple_of(3) { Commit::Final } else { Commit::Preview };
                        let _ = doc.set_layer_opacity(layer, rng.next_f32(), commit);
                        free_form = true;
                    }
                    14 => {
                        let _ = doc.set_layer_locked(layer, rng.next_u32().is_multiple_of(4));
                    }
                    15 => {
                        let _ = doc.set_layer_alpha_lock(layer, rng.next_u32().is_multiple_of(2));
                    }
                    16 => {
                        if is_raster {
                            let _ = doc.rename_layer(layer, &format!("name {}", rng.next_u32() % 5));
                        }
                    }
                    // (set_active_layer is deliberately absent: the active layer is not history, and the pixel
                    // entries of this test do not carry it, so it is no function of the history position.)
                    17 => doc.flush_pending_edit(),
                    18 | 19 => {
                        undone = doc.undo().is_some();
                        moved = undone;
                    }
                    20 => moved = doc.redo().is_some(),
                    _ => moved = doc.cancel_pending_edit(),
                }
                let (n, pending) = (doc.history.undo_len(), doc.has_pending_edit());
                let state = doc_state(&mut doc);
                let ctx = format!("seed {seed}, step {step}");
                if moved {
                    assert!(!pending, "{ctx}");
                    if undone && pending_before {
                        // undo() flushed the preview into an entry (which cleared the redo stack) and undid it.
                        states.truncate(n_before + 1);
                        states.push(preview.take().unwrap());
                    }
                    preview = None;
                    assert_eq!(state, states[n], "{ctx}: undo / redo / cancel");
                } else if pending {
                    if pending_before && !free_form {
                        assert_eq!(Some(&state), preview.as_ref(), "{ctx}: nothing ran, so nothing may change");
                    }
                    preview = Some(state);
                } else if pending_before {
                    // The step flushed the pending edit (one entry) and maybe added its own entry.
                    let flushed = preview.take().unwrap();
                    states.truncate(n_before + 1);
                    if n == n_before + 2 {
                        states.push(flushed);
                    } else {
                        assert_eq!(n, n_before + 1, "{ctx}");
                        if !free_form {
                            assert_eq!(state, flushed, "{ctx}: the command did nothing after the flush");
                        }
                    }
                    states.push(state);
                } else if n == n_before {
                    if free_form {
                        states[n] = state;
                    } else {
                        assert_eq!(state, states[n], "{ctx}: no entry, so nothing may change");
                    }
                } else {
                    assert_eq!(n, n_before + 1, "{ctx}");
                    states.truncate(n);
                    states.push(state);
                }
            }

            if doc.has_pending_edit() {
                states.truncate(doc.history.undo_len() + 1);
                states.push(preview.take().unwrap());
            }
            doc.flush_pending_edit();
            let n = doc.history.undo_len();
            assert!(n > 60, "seed {seed}: only {n} entries");
            assert_eq!(doc_state(&mut doc), states[n]);
            for k in (0..n).rev() {
                assert!(doc.undo().is_some());
                assert_eq!(doc_state(&mut doc), states[k], "seed {seed}: undo to {k}");
            }
            assert_eq!(doc.undo(), None);
            for (k, expected) in states.iter().enumerate().take(n + 1).skip(1) {
                assert!(doc.redo().is_some());
                assert_eq!(doc_state(&mut doc), *expected, "seed {seed}: redo to {k}");
            }
            assert_eq!(doc.redo(), None);
        }
    }

    #[test]
    fn merge_down_equals_the_composite_of_the_two_layers() {
        let mut rng = XorShift::new(9);
        for (mode, opacity) in [(BlendMode::Normal, 1.0), (BlendMode::Multiply, 0.6), (BlendMode::Overlay, 0.35), (BlendMode::Add, 1.0)] {
            let mut doc = doc_from_raw(samples::flat_quad(), 80);
            let lower = doc.sets[0].layers[0].id;
            let upper = doc.add_raster_layer(S0, None).unwrap();
            // Random content in both, including transparent texels with hidden colour.
            for layer in [lower, upper] {
                let index = doc.find_layer(layer).unwrap().1;
                doc.sets[0].allocate_raster(index);
                let LayerKind::Raster(buf) = &mut doc.layer_mut(layer).unwrap().kind else { unreachable!() };
                for px in Arc::make_mut(buf).data.as_chunks_mut::<4>().0 {
                    let a = [0, 255, (rng.next_u32() & 0xff) as u8][rng.next_u32() as usize % 3];
                    *px = [rng.next_u32() as u8, rng.next_u32() as u8, rng.next_u32() as u8, a];
                }
                doc.mark_layer_dirty(layer, IRect::from_size(80, 80));
            }
            doc.set_layer_blend(upper, mode).unwrap();
            doc.set_layer_opacity(upper, opacity, Commit::Final).unwrap();
            let composite = doc.flatten(S0, false).unwrap();
            let (below, top) = (raster_arc(&doc, lower).clone(), raster_arc(&doc, upper).clone());

            assert_eq!(doc.merge_down(upper).unwrap(), lower, "{mode:?}");
            assert_eq!(ids(&doc, S0), [lower.0]);
            assert_eq!(doc.sets[0].active, Some(lower));
            let merged = raster_arc(&doc, lower);
            assert_eq!(**merged, composite, "{mode:?}: lower layer Normal at 100% => merge == composite");
            // And pixel by pixel the rule of 6.5.
            let op = crate::composite::opacity_u8(opacity);
            for y in 0..80 {
                for x in 0..80 {
                    assert_eq!(merged.get(x, y), blend_pixel(below.get(x, y), top.get(x, y), op, mode));
                }
            }
            // The surviving layer keeps its own properties.
            let l = doc.layer(lower).unwrap();
            assert_eq!((l.name.as_str(), l.opacity, l.blend), ("Layer 1", 1.0, BlendMode::Normal));
            doc.recomposite_all();
            assert_eq!(doc.composite(S0), Some(&composite));
        }
    }

    #[test]
    fn merge_down_keeps_the_lower_layers_properties_and_only_touches_the_upper_rect() {
        let mut doc = doc_with_stamp();
        doc.set_layer_opacity(LayerId(1), 0.5, Commit::Final).unwrap();
        doc.set_layer_blend(LayerId(1), BlendMode::Screen).unwrap();
        doc.set_active_layer(S0, Some(LayerId(101))).unwrap();
        doc_state(&mut doc);
        doc.take_gpu_dirty(S0);
        let before = raster_arc(&doc, LayerId(1)).clone();
        let cache = doc.layer(LayerId(101)).unwrap();
        let cache_rect = cache.content_rect();
        let cache_data = crate::pixel_ops::crop(&cache.to_dense(SIZE, SIZE), cache_rect).data;
        assert_eq!(doc.merge_down(LayerId(101)).unwrap(), LayerId(1));
        let merged = raster_arc(&doc, LayerId(1)).clone();
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                let want = if cache_rect.contains(x, y) {
                    let i = ((y - cache_rect.y0) as usize * cache_rect.width() as usize + (x - cache_rect.x0) as usize) * 4;
                    let src = [cache_data[i], cache_data[i + 1], cache_data[i + 2], cache_data[i + 3]];
                    blend_pixel(before.get(x as u32, y as u32), src, 255, BlendMode::Normal)
                } else {
                    before.get(x as u32, y as u32)
                };
                assert_eq!(merged.get(x as u32, y as u32), want, "({x}, {y})");
            }
        }
        let l = doc.layer(LayerId(1)).unwrap();
        assert_eq!((l.opacity, l.blend, l.name.as_str()), (0.5, BlendMode::Screen, "Layer 1"));
        assert_eq!(doc.sets[0].active, Some(LayerId(1)), "the active upper layer hands over to the survivor");
        doc_state(&mut doc);
        assert_eq!(doc.take_gpu_dirty(S0), vec![cache_rect], "only the upper layer's rect is recomposited");
    }

    #[test]
    fn property_commands_pin_nothing() {
        let mut doc = doc_with_stamp();
        paint_rect(&mut doc, LayerId(3), IRect::new(0, 0, 96, 96), [1, 2, 3, 200]);
        let l = LayerId(1);
        type Command = Box<dyn Fn(&mut Document) -> Result<(), EditError>>;
        let commands: Vec<(&str, Command)> = vec![
            ("visible", Box::new(move |d| d.set_layer_visible(l, false))),
            ("visible back", Box::new(move |d| d.set_layer_visible(l, true))),
            ("locked", Box::new(move |d| d.set_layer_locked(l, true))),
            ("unlocked", Box::new(move |d| d.set_layer_locked(l, false))),
            ("alpha lock", Box::new(move |d| d.set_layer_alpha_lock(l, true))),
            ("blend", Box::new(move |d| d.set_layer_blend(l, BlendMode::Darken))),
            ("opacity preview", Box::new(move |d| d.set_layer_opacity(l, 0.3, Commit::Preview))),
            ("opacity final", Box::new(move |d| d.set_layer_opacity(l, 0.6, Commit::Final))),
            ("rename", Box::new(move |d| d.rename_layer(l, "Renamed"))),
            ("move", Box::new(move |d| d.move_layer(l, 2))),
            ("add", Box::new(|d| d.add_raster_layer(S0, None).map(|_| ()))),
            ("stamp layer visible", Box::new(|d| d.set_layer_visible(LayerId(101), false))),
            ("duplicate", Box::new(move |d| d.duplicate_layer(l).map(|_| ()))),
        ];
        for (what, command) in &commands {
            let bytes = doc.history_bytes();
            command(&mut doc).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_rasters_unpinned(&doc, what);
            assert_eq!(doc.history_bytes(), bytes, "{what}: the entry costs nothing");

            // A following pixel edit (a stroke flushes the pending edit first) grows the history by its tile patch
            // only, not by a copy of the layer.
            doc.flush_pending_edit();
            assert_rasters_unpinned(&doc, what);
            let bytes = doc.history_bytes();
            let pointer = Arc::as_ptr(raster_arc(&doc, l));
            paint_rect(&mut doc, l, IRect::new(4, 4, 20, 20), [9, 9, 9, 255]);
            assert_eq!(Arc::as_ptr(raster_arc(&doc, l)), pointer, "{what}: the stroke edited the buffer in place");
            let Edit::Pixels { patch, .. } = &doc.history.undo.last().unwrap().edit else { panic!("{what}: not a pixel entry") };
            assert_eq!(doc.history_bytes() - bytes, patch.byte_len(), "{what}");
            assert!(patch.byte_len() <= 64 * 64 * 4 + 64, "{what}");
        }
        // Undo and redo of all of it pins nothing either.
        while doc.undo().is_some() {
            assert_rasters_unpinned(&doc, "undo");
        }
        while doc.redo().is_some() {}
        assert_rasters_unpinned(&doc, "redo");
    }

    #[test]
    fn remove_and_merge_pin_exactly_the_replaced_layers() {
        let newest = |doc: &Document| match &doc.history.undo.last().unwrap().edit {
            Edit::Structure(s) => record_kinds(s),
            _ => panic!("not a structure entry"),
        };
        let bytes = (SIZE * SIZE * 4) as usize;

        // remove_layer: the removed paint layer.
        let mut doc = doc_with_stamp();
        doc.remove_layer(LayerId(3)).unwrap();
        assert_eq!(
            newest(&doc),
            [
                vec![(LayerId(1), "keep"), (LayerId(101), "stamp"), (LayerId(3), "raster")],
                vec![(LayerId(2), "keep"), (LayerId(102), "stamp")]
            ]
        );
        assert_eq!(doc.history_bytes(), bytes, "the removed layer is the only buffer the history owns");
        assert_rasters_unpinned(&doc, "remove");

        // merge_down of two paint layers: the lower layer and the upper one.
        let mut doc = doc_with_stamp();
        doc.move_layer(LayerId(3), 1).unwrap();
        doc.merge_down(LayerId(3)).unwrap();
        assert_eq!(
            newest(&doc),
            [
                vec![(LayerId(1), "raster"), (LayerId(3), "raster"), (LayerId(101), "stamp")],
                vec![(LayerId(2), "keep"), (LayerId(102), "stamp")]
            ]
        );
        assert_eq!(doc.history_bytes(), 2 * bytes);
        assert_rasters_unpinned(&doc, "merge");
        // After undo the history holds the merged result instead; the restored layers are not shared with it.
        doc.undo().unwrap();
        assert_eq!(doc.history_bytes(), bytes);
        assert_rasters_unpinned(&doc, "merge undone");

        // merge_down of a stamp layer: only the lower layer (the cache is stored anyway).
        let mut doc = doc_with_stamp();
        doc.merge_down(LayerId(101)).unwrap();
        assert_eq!(
            newest(&doc),
            [
                vec![(LayerId(1), "raster"), (LayerId(101), "stamp"), (LayerId(3), "keep")],
                vec![(LayerId(2), "keep"), (LayerId(102), "stamp")]
            ]
        );
        // The lower layer's old pixels plus the cache that left the document.
        assert_eq!(doc.history_bytes(), bytes + 40 * 28 * 4);

        // Every other command: none.
        let mut doc = doc_with_stamp();
        doc.add_raster_layer(S0, None).unwrap();
        doc.duplicate_layer(LayerId(1)).unwrap();
        doc.rasterize_layer(LayerId(102)).unwrap();
        for entry in &doc.history.undo {
            let Edit::Structure(s) = &entry.edit else { panic!() };
            assert!(record_kinds(s).iter().flatten().all(|(_, kind)| *kind != "raster"), "{}", entry.label);
        }
    }

    #[test]
    fn versions_change_with_pixels_only() {
        let mut doc = doc_with_stamp();
        let version = |doc: &Document, id: u64| doc.layer_version(LayerId(id)).unwrap();
        let all = |doc: &Document| [1, 2, 3, 101, 102].map(|id| version(doc, id));
        let v0 = all(&doc);
        let mut sorted = v0.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 5, "from_parts assigns distinct versions");
        assert_eq!(doc.layer_version(LayerId(77)), None);

        // Property and order changes, and their undo / redo, keep every version.
        doc.set_layer_visible(LayerId(1), false).unwrap();
        doc.set_layer_locked(LayerId(1), true).unwrap();
        doc.set_layer_alpha_lock(LayerId(3), true).unwrap();
        doc.set_layer_blend(LayerId(101), BlendMode::Add).unwrap();
        doc.set_layer_opacity(LayerId(3), 0.5, Commit::Preview).unwrap();
        doc.set_layer_opacity(LayerId(3), 0.25, Commit::Final).unwrap();
        doc.rename_layer(LayerId(3), "x").unwrap();
        doc.move_layer(LayerId(3), 0).unwrap();
        doc.set_active_layer(S0, Some(LayerId(1))).unwrap();
        assert_eq!(all(&doc), v0);
        while doc.undo().is_some() {
            assert_eq!(all(&doc), v0);
        }
        while doc.redo().is_some() {
            assert_eq!(all(&doc), v0);
        }

        // Pixel edits, and undo / redo of them, give the edited layer (and only it) a fresh version each time.
        let mut seen = v0.to_vec();
        paint_rect(&mut doc, LayerId(3), IRect::new(1, 1, 9, 9), [5, 5, 5, 255]);
        for step in 0..5 {
            let now = all(&doc);
            assert_eq!([now[0], now[1], now[3], now[4]], [v0[0], v0[1], v0[3], v0[4]]);
            assert!(!seen.contains(&now[2]), "step {step}: version {} was used before", now[2]);
            seen.push(now[2]);
            if step % 2 == 0 { doc.undo().unwrap() } else { doc.redo().unwrap() };
        }

        // New and replaced pixel storage gets fresh versions too.
        let added = doc.add_raster_layer(S0, None).unwrap();
        let copy = doc.duplicate_layer(LayerId(1)).unwrap();
        doc.rasterize_layer(LayerId(102)).unwrap();
        doc.merge_down(LayerId(101)).unwrap();
        let mut versions: Vec<u64> = doc.sets.iter().flat_map(|s| &s.layers).map(|l| l.version()).collect();
        let count = versions.len();
        versions.sort_unstable();
        versions.dedup();
        assert_eq!(versions.len(), count, "versions are unique");
        for id in [added, copy, LayerId(102)] {
            assert!(!seen.contains(&doc.layer_version(id).unwrap()));
        }
        // Undo of the rasterize brings the cache back with the version it had.
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert_eq!(version(&doc, 102), v0[4]);
    }

    #[test]
    fn paint_target_reports_the_reason() {
        let mut doc = doc_with_stamp();
        assert_eq!(doc.paint_target(S0), PaintTarget::Ready(LayerId(3)));
        assert_eq!(doc.paint_target(TextureSetId(7)), PaintTarget::NoActiveLayer);
        doc.set_active_layer(S0, Some(LayerId(101))).unwrap();
        assert_eq!(doc.paint_target(S0), PaintTarget::NotRaster(LayerId(101)));
        doc.set_active_layer(S0, None).unwrap();
        assert_eq!(doc.paint_target(S0), PaintTarget::NoActiveLayer);
        doc.set_active_layer(S0, Some(LayerId(1))).unwrap();
        doc.set_layer_visible(LayerId(1), false).unwrap();
        assert_eq!(doc.paint_target(S0), PaintTarget::Hidden(LayerId(1)));
        doc.set_layer_locked(LayerId(1), true).unwrap();
        assert_eq!(doc.paint_target(S0), PaintTarget::Locked(LayerId(1)), "locked wins over hidden");
        doc.set_layer_visible(LayerId(1), true).unwrap();
        assert_eq!(doc.paint_target(S0), PaintTarget::Locked(LayerId(1)));
        // Zero opacity or alpha lock do not block painting.
        doc.set_layer_locked(LayerId(1), false).unwrap();
        doc.set_layer_opacity(LayerId(1), 0.0, Commit::Final).unwrap();
        doc.set_layer_alpha_lock(LayerId(1), true).unwrap();
        assert_eq!(doc.paint_target(S0), PaintTarget::Ready(LayerId(1)));
    }

    #[test]
    fn add_raster_layer_names_and_places_the_layer() {
        let mut doc = doc_with_stamp();
        // Set 0 = [1, 101, 3 "Paint 1" (active)].
        let a = doc.add_raster_layer(S0, None).unwrap();
        assert_eq!(a, LayerId(200), "ids come from next_id");
        assert_eq!(doc.layer(a).unwrap().name, "Paint 2");
        assert_eq!(doc.sets[0].active, Some(a));
        assert_eq!(ids(&doc, S0), [1, 101, 3, 200]);
        // Above the active layer, not on top.
        doc.set_active_layer(S0, Some(LayerId(1))).unwrap();
        let b = doc.add_raster_layer(S0, None).unwrap();
        assert_eq!(ids(&doc, S0), [1, b.0, 101, 3, 200]);
        assert_eq!(doc.layer(b).unwrap().name, "Paint 3");
        // The smallest free number is reused; other names do not count.
        doc.rename_layer(LayerId(3), "Paint 10").unwrap();
        doc.rename_layer(a, "paint 2").unwrap();
        let c = doc.add_raster_layer(S0, None).unwrap();
        assert_eq!(doc.layer(c).unwrap().name, "Paint 1");
        let d = doc.add_raster_layer(S0, None).unwrap();
        assert_eq!(doc.layer(d).unwrap().name, "Paint 2");
        // No active layer: on top. Explicit names are taken as given.
        doc.set_active_layer(S1, None).unwrap();
        let e = doc.add_raster_layer(S1, Some("Details")).unwrap();
        assert_eq!(ids(&doc, S1), [2, 102, e.0]);
        let layer = doc.layer(e).unwrap();
        assert_eq!(layer.name, "Details");
        assert!(layer.visible && !layer.locked && !layer.alpha_lock && layer.opacity == 1.0 && layer.blend == BlendMode::Normal);
        // A new layer is unallocated (finding SP-5): no memory until the first stroke or fill.
        assert!(raster_arc(&doc, e).data.is_empty());
        assert_eq!((raster_arc(&doc, e).width, raster_arc(&doc, e).height), (0, 0));
        assert!(layer.content_rects().is_empty());
        // Each set numbers its own layers.
        let f = doc.add_raster_layer(S1, None).unwrap();
        assert_eq!(doc.layer(f).unwrap().name, "Paint 1");
        assert!(matches!(doc.add_raster_layer(TextureSetId(5), None), Err(EditError::UnknownSet)));
        // A transparent layer changes no composite pixel.
        assert!(!doc.has_pending_composite());
    }

    #[test]
    fn remove_layer_selects_a_neighbour_and_respects_the_lock() {
        let mut doc = doc_from_raw(samples::flat_quad(), 32);
        let base = doc.sets[0].layers[0].id;
        let a = doc.add_raster_layer(S0, None).unwrap();
        let b = doc.add_raster_layer(S0, None).unwrap();
        assert_eq!(ids(&doc, S0), [base.0, a.0, b.0]);
        // Removing a layer that is not active keeps the active layer.
        doc.remove_layer(a).unwrap();
        assert_eq!(doc.sets[0].active, Some(b));
        doc.undo().unwrap();
        // Removing the active layer: the layer below takes over.
        doc.remove_layer(b).unwrap();
        assert_eq!(doc.sets[0].active, Some(a));
        // A locked layer stays.
        doc.set_layer_locked(a, true).unwrap();
        let entries = doc.history.undo_len();
        assert!(matches!(doc.remove_layer(a), Err(EditError::LayerLocked)));
        assert_eq!((doc.history.undo_len(), ids(&doc, S0)), (entries, vec![base.0, a.0]));
        doc.set_layer_locked(a, false).unwrap();
        // The bottom layer has nothing below: the layer above takes over.
        doc.set_active_layer(S0, Some(base)).unwrap();
        doc.remove_layer(base).unwrap();
        assert_eq!(doc.sets[0].active, Some(a));
        // The last layer: no active layer; the composite becomes transparent.
        doc.remove_layer(a).unwrap();
        assert_eq!(doc.sets[0].active, None);
        assert!(doc.sets[0].layers.is_empty());
        doc.recomposite_all();
        assert!(doc.composite(S0).unwrap().data.iter().all(|&v| v == 0));
        assert_eq!(doc.paint_target(S0), PaintTarget::NoActiveLayer);
        // And everything comes back.
        while doc.undo().is_some() {}
        assert_eq!(ids(&doc, S0), [base.0]);
        assert_eq!(doc.sets[0].active, Some(base));
    }

    #[test]
    fn invalid_calls_are_errors_and_change_nothing() {
        let mut doc = doc_with_stamp();
        let before = doc_state(&mut doc);
        let ghost = LayerId(9999);
        assert!(matches!(doc.remove_layer(ghost), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.duplicate_layer(ghost), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.move_layer(ghost, 0), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.merge_down(ghost), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.rasterize_layer(ghost), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_layer_visible(ghost, false), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_layer_locked(ghost, true), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_layer_alpha_lock(ghost, true), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_layer_blend(ghost, BlendMode::Add), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_layer_opacity(ghost, 0.5, Commit::Final), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_layer_opacity(ghost, 0.5, Commit::Preview), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.rename_layer(ghost, "x"), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_active_layer(TextureSetId(9), None), Err(EditError::UnknownSet)));
        assert!(matches!(doc.set_active_layer(S0, Some(ghost)), Err(EditError::UnknownLayer)));
        assert!(matches!(doc.set_active_layer(S0, Some(LayerId(2))), Err(EditError::UnknownLayer)), "a layer of another set");
        assert!(matches!(doc.add_raster_layer(TextureSetId(9), None), Err(EditError::UnknownSet)));
        assert!(doc.layer_thumbnail(ghost, 16).is_none());
        assert!(doc.set_thumbnail(TextureSetId(9), 16).is_none());

        assert!(matches!(doc.set_layer_opacity(LayerId(1), f32::NAN, Commit::Final), Err(EditError::InvalidParam(_))));
        assert!(matches!(doc.set_layer_opacity(LayerId(1), f32::INFINITY, Commit::Preview), Err(EditError::InvalidParam(_))));
        assert!(matches!(doc.rasterize_layer(LayerId(1)), Err(EditError::InvalidParam(_))));
        // merge_down: no layer below, a stamp layer below, locked layers.
        assert!(matches!(doc.merge_down(LayerId(1)), Err(EditError::CannotMerge(_))));
        assert!(matches!(doc.merge_down(LayerId(3)), Err(EditError::CannotMerge(_))));
        assert!(!doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), 0);
        assert_eq!(doc_state(&mut doc), before);
        assert!(!doc.is_modified());

        doc.set_layer_locked(LayerId(101), true).unwrap();
        assert!(matches!(doc.merge_down(LayerId(101)), Err(EditError::LayerLocked)));
        assert!(matches!(doc.rasterize_layer(LayerId(101)), Err(EditError::LayerLocked)));
        doc.set_layer_locked(LayerId(101), false).unwrap();
        doc.set_layer_locked(LayerId(1), true).unwrap();
        assert!(matches!(doc.merge_down(LayerId(101)), Err(EditError::CannotMerge(_))));
        assert_eq!(doc.history.undo_len(), 3);
        // Properties of a locked layer can still be changed (the imported Base is locked by default).
        doc.set_layer_visible(LayerId(1), false).unwrap();
        doc.set_layer_opacity(LayerId(1), 0.5, Commit::Final).unwrap();
        doc.set_layer_blend(LayerId(1), BlendMode::Multiply).unwrap();
        doc.rename_layer(LayerId(1), "Locked").unwrap();
        doc.move_layer(LayerId(1), 1).unwrap();
        doc.duplicate_layer(LayerId(1)).unwrap();
    }

    #[test]
    fn duplicate_copies_pixels_into_an_unlocked_layer_above() {
        let mut doc = doc_with_stamp();
        paint_rect(&mut doc, LayerId(1), IRect::new(3, 3, 30, 30), [7, 8, 9, 100]);
        doc.set_layer_locked(LayerId(1), true).unwrap();
        doc.set_layer_opacity(LayerId(1), 0.5, Commit::Final).unwrap();
        doc.set_layer_blend(LayerId(1), BlendMode::Lighten).unwrap();
        doc.set_layer_alpha_lock(LayerId(1), true).unwrap();
        let copy = doc.duplicate_layer(LayerId(1)).unwrap();
        assert_eq!(ids(&doc, S0), [1, copy.0, 101, 3]);
        let (source, layer) = (doc.layer(LayerId(1)).unwrap(), doc.layer(copy).unwrap());
        assert_eq!(layer.name, "Layer 1 copy");
        assert!(!layer.locked && source.locked);
        assert_eq!((layer.opacity, layer.blend, layer.alpha_lock, layer.visible), (0.5, BlendMode::Lighten, true, true));
        assert_eq!(**raster_arc(&doc, copy), **raster_arc(&doc, LayerId(1)));
        assert!(!layer.same_pixels(source), "a real copy, not a shared buffer");
        assert_ne!(layer.version(), source.version());
        assert_eq!(doc.sets[0].active, Some(copy));
        assert_eq!(doc.undo_label(), Some("Duplicate Layer"));
    }

    #[test]
    fn rasterize_and_merge_keep_the_stamp_bookkeeping_consistent() {
        // Rasterizing the layer of the home set moves the home to the remaining set.
        let mut doc = doc_with_stamp();
        let cache = doc.layer(LayerId(101)).unwrap();
        let rect = cache.content_rect();
        let data = crate::pixel_ops::crop(&cache.to_dense(SIZE, SIZE), rect).data;
        let composite = doc.composite(S0).unwrap().clone();
        doc.rasterize_layer(LayerId(101)).unwrap();
        let layer = doc.layer(LayerId(101)).unwrap();
        assert!(layer.is_raster() && layer.stamp_id().is_none());
        assert_eq!(layer.name, "Decal");
        let pixels = raster_arc(&doc, LayerId(101));
        assert_eq!((pixels.width, pixels.height), (SIZE, SIZE));
        assert_eq!(crate::pixel_ops::crop(pixels, rect).data, data);
        assert_eq!(crate::pixel_ops::content_bounds(pixels.view()), Some(rect));
        assert_eq!(doc.stamps.len(), 1);
        assert_eq!(doc.stamps[0].home_set, S1);
        assert_eq!(doc.stamp_layers(STAMP), [(S1, LayerId(102))]);
        assert!(!doc.has_pending_composite(), "same pixels: nothing to recomposite");
        assert_eq!(doc.composite(S0), Some(&composite));
        assert_eq!(doc.flatten(S0, false).unwrap(), composite);
        // Rasterizing the last linked layer removes the stamp.
        doc.rasterize_layer(LayerId(102)).unwrap();
        assert!(doc.stamps.is_empty());
        assert!(doc.stamp(STAMP).is_none());
        // Undo restores the stamp, its home set and the pointer-equal caches.
        doc.undo().unwrap();
        assert_eq!(doc.stamps[0].home_set, S1);
        doc.undo().unwrap();
        assert_eq!(doc.stamps[0].home_set, S0);
        assert_eq!(doc.layer(LayerId(101)).unwrap().stamp_id(), Some(STAMP));
        assert_eq!(crate::pixel_ops::crop(&doc.layer(LayerId(101)).unwrap().to_dense(SIZE, SIZE), rect).data, data);

        // Rasterizing a layer outside the home set leaves the home alone.
        let mut doc = doc_with_stamp();
        doc.rasterize_layer(LayerId(102)).unwrap();
        assert_eq!(doc.stamps[0].home_set, S0);
        assert_eq!(doc.stamp_layers(STAMP), [(S0, LayerId(101))]);
        // Merging the remaining one away removes the stamp, in one entry.
        let entries = doc.history.undo_len();
        doc.merge_down(LayerId(101)).unwrap();
        assert_eq!(doc.history.undo_len(), entries + 1);
        assert!(doc.stamps.is_empty());
        assert_eq!(ids(&doc, S0), [1, 3]);
        doc.undo().unwrap();
        assert_eq!(doc.stamps.len(), 1);
        assert_eq!(ids(&doc, S0), [1, 101, 3]);
    }

    #[test]
    fn set_active_layer_is_not_history() {
        let mut doc = doc_with_stamp();
        let revision = doc.revision();
        doc.set_active_layer(S0, Some(LayerId(1))).unwrap();
        doc.set_active_layer(S1, None).unwrap();
        assert_eq!((doc.sets[0].active, doc.sets[1].active), (Some(LayerId(1)), None));
        assert_eq!(doc.sets[0].active_layer().map(|l| l.id), Some(LayerId(1)));
        assert_eq!((doc.history.undo_len(), doc.revision()), (0, revision));
        assert!(!doc.is_modified() && !doc.can_undo() && !doc.has_pending_composite());
        // It does not end a pending edit either.
        doc.set_layer_opacity(LayerId(3), 0.5, Commit::Preview).unwrap();
        doc.set_active_layer(S0, Some(LayerId(3))).unwrap();
        assert!(doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), 0);
    }

    #[test]
    fn opacity_previews_fold_into_one_entry() {
        let mut doc = doc_with_stamp();
        let s0 = doc_state(&mut doc);
        let l = LayerId(1);
        for i in 1..=10 {
            doc.set_layer_opacity(l, 1.0 - i as f32 * 0.05, Commit::Preview).unwrap();
            assert!(doc.has_pending_edit());
            assert_eq!(doc.history.undo_len(), 0);
            assert!(doc.has_pending_composite(), "every preview is visible");
            doc.recomposite_all();
        }
        doc.set_layer_opacity(l, 0.3, Commit::Final).unwrap();
        assert!(!doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), 1, "N previews + Final = one entry");
        assert_eq!(doc.undo_label(), Some("Layer Opacity"));
        assert_eq!(doc.layer(l).unwrap().opacity, 0.3);
        let s1 = doc_state(&mut doc);
        assert_eq!(doc.undo().as_deref(), Some("Layer Opacity"));
        assert_eq!(doc_state(&mut doc), s0);
        doc.redo().unwrap();
        assert_eq!(doc_state(&mut doc), s1);

        // Cancel during the drag restores the value from before the first preview; no entry.
        doc.set_layer_opacity(l, 0.9, Commit::Preview).unwrap();
        doc.set_layer_opacity(l, 0.8, Commit::Preview).unwrap();
        assert!(doc.cancel_pending_edit());
        assert_eq!(doc.layer(l).unwrap().opacity, 0.3);
        assert_eq!(doc_state(&mut doc), s1);
        assert_eq!(doc.history.undo_len(), 1);
        assert!(!doc.cancel_pending_edit());

        // Another command ends the drag: the preview becomes its own entry before the command's entry.
        doc.set_layer_opacity(l, 0.9, Commit::Preview).unwrap();
        doc.set_layer_visible(LayerId(3), false).unwrap();
        assert!(!doc.has_pending_edit());
        assert_eq!(doc.history.undo_len(), 3);
        doc.undo().unwrap();
        assert_eq!(doc.layer(l).unwrap().opacity, 0.9);
        doc.undo().unwrap();
        assert_eq!(doc.layer(l).unwrap().opacity, 0.3);

        // Two layers dragged without a Final in between share the pending edit.
        doc.set_layer_opacity(l, 0.5, Commit::Preview).unwrap();
        doc.set_layer_opacity(LayerId(3), 0.5, Commit::Final).unwrap();
        assert_eq!(doc.history.undo_len(), 2);
        doc.undo().unwrap();
        assert_eq!((doc.layer(l).unwrap().opacity, doc.layer(LayerId(3)).unwrap().opacity), (0.3, 1.0));

        // Values are clamped; a Final that changes nothing outside a drag pushes nothing.
        let entries = doc.history.undo_len();
        doc.set_layer_opacity(l, 7.0, Commit::Final).unwrap();
        assert_eq!(doc.layer(l).unwrap().opacity, 1.0);
        doc.set_layer_opacity(l, -3.0, Commit::Final).unwrap();
        assert_eq!(doc.layer(l).unwrap().opacity, 0.0);
        assert_eq!(doc.history.undo_len(), entries + 2);
        doc.set_layer_opacity(l, 0.0, Commit::Final).unwrap();
        doc.set_layer_opacity(l, -1.0, Commit::Preview).unwrap();
        assert_eq!(doc.history.undo_len(), entries + 2);
        assert!(!doc.has_pending_edit());
    }

    #[test]
    fn commands_that_change_nothing_push_nothing() {
        let mut doc = doc_with_stamp();
        let before = doc_state(&mut doc);
        doc.set_layer_visible(LayerId(1), true).unwrap();
        doc.set_layer_locked(LayerId(1), false).unwrap();
        doc.set_layer_alpha_lock(LayerId(1), false).unwrap();
        doc.set_layer_blend(LayerId(1), BlendMode::Normal).unwrap();
        doc.set_layer_opacity(LayerId(1), 1.0, Commit::Final).unwrap();
        doc.rename_layer(LayerId(1), "Layer 1").unwrap();
        doc.move_layer(LayerId(1), 0).unwrap();
        doc.move_layer(LayerId(3), 2).unwrap();
        doc.move_layer(LayerId(3), 50).unwrap();
        assert_eq!(doc.history.undo_len(), 0);
        assert!(!doc.is_modified() && !doc.has_pending_composite() && !doc.has_pending_edit());
        assert_eq!(doc_state(&mut doc), before);
    }

    #[test]
    /// Finding SP-4 (review round 2): property changes of a paint layer recomposite only where it has content, and
    /// an empty paint layer costs nothing (it is skipped by the composite and its changes dirty nothing).
    fn paint_layer_changes_dirty_only_their_content() {
        let mut doc = doc_with_stamp();
        let dirty = |doc: &mut Document| {
            doc.recomposite_all();
            (doc.take_gpu_dirty(S0), doc.take_gpu_dirty(S1))
        };
        dirty(&mut doc);
        let small = IRect::new(20, 30, 28, 34);
        paint_rect(&mut doc, LayerId(3), small, [9, 9, 9, 200]);
        assert_eq!(dirty(&mut doc), (vec![small], vec![]));
        doc.set_layer_opacity(LayerId(3), 0.5, Commit::Preview).unwrap();
        doc.set_layer_opacity(LayerId(3), 0.4, Commit::Final).unwrap();
        assert_eq!(dirty(&mut doc), (vec![small], vec![]));
        doc.set_layer_visible(LayerId(3), false).unwrap();
        assert_eq!(dirty(&mut doc), (vec![small], vec![]));
        doc.set_layer_blend(LayerId(3), BlendMode::Multiply).unwrap();
        assert_eq!(dirty(&mut doc), (vec![small], vec![]));
        for _ in 0..3 {
            doc.undo().unwrap();
            assert_eq!(dirty(&mut doc), (vec![small], vec![]));
        }
        // A new layer is unallocated and empty: nothing it does changes a composite pixel.
        let e = doc.add_raster_layer(S0, None).unwrap();
        doc.set_layer_visible(e, false).unwrap();
        doc.set_layer_visible(e, true).unwrap();
        doc.set_layer_opacity(e, 0.3, Commit::Final).unwrap();
        doc.set_layer_blend(e, BlendMode::Screen).unwrap();
        doc.move_layer(e, 0).unwrap();
        let copy = doc.duplicate_layer(e).unwrap();
        doc.remove_layer(copy).unwrap();
        assert_eq!(dirty(&mut doc), (vec![], vec![]));
        // Merging the small layer down into a copy dirties only its content.
        let base = doc.duplicate_layer(LayerId(1)).unwrap();
        dirty(&mut doc);
        doc.move_layer(LayerId(3), doc.find_layer(base).unwrap().1 + 1).unwrap();
        dirty(&mut doc);
        doc.merge_down(LayerId(3)).unwrap();
        assert_eq!(dirty(&mut doc), (vec![small], vec![]));
        doc_state(&mut doc);
    }

    #[test]
    fn dirty_rects_are_tight() {
        let mut doc = doc_with_stamp();
        let full = IRect::from_size(SIZE, SIZE);
        let stamp_rect = IRect::new(10, 12, 50, 40);
        let dirty = |doc: &mut Document| {
            doc.recomposite_all();
            (doc.take_gpu_dirty(S0), doc.take_gpu_dirty(S1))
        };
        assert_eq!(dirty(&mut doc), (vec![], vec![]));
        doc.set_layer_visible(LayerId(101), false).unwrap();
        assert_eq!(dirty(&mut doc), (vec![stamp_rect], vec![]));
        doc.set_layer_blend(LayerId(101), BlendMode::Multiply).unwrap();
        doc.set_layer_opacity(LayerId(101), 0.5, Commit::Final).unwrap();
        assert_eq!(dirty(&mut doc), (vec![stamp_rect], vec![]));
        doc.set_layer_locked(LayerId(101), true).unwrap();
        doc.set_layer_alpha_lock(LayerId(101), true).unwrap();
        doc.rename_layer(LayerId(1), "n").unwrap();
        doc.add_raster_layer(S0, None).unwrap();
        assert_eq!(dirty(&mut doc), (vec![], vec![]), "these change no composite pixel");
        doc.set_layer_visible(LayerId(2), false).unwrap();
        assert_eq!(dirty(&mut doc), (vec![], vec![full]));
        // Moving the stamp layer below the base: only where the moved layer has content can the fold change.
        doc.move_layer(LayerId(101), 0).unwrap();
        assert_eq!(dirty(&mut doc), (vec![stamp_rect], vec![]));
        // Undo of a visibility toggle of the stamp layer repaints only its rect.
        doc.set_layer_visible(LayerId(101), true).unwrap();
        dirty(&mut doc);
        doc.undo().unwrap();
        assert_eq!(dirty(&mut doc), (vec![stamp_rect], vec![]));
    }

    #[test]
    fn thumbnails_follow_the_set_aspect() {
        let mut doc = doc_with_stamp();
        paint_rect(&mut doc, LayerId(3), IRect::new(0, 0, 96, 96), [255, 0, 0, 255]);
        doc.recomposite_all();
        let t = doc.layer_thumbnail(LayerId(3), 24).unwrap();
        assert_eq!((t.width, t.height), (24, 24));
        assert!(t.data.as_chunks::<4>().0.iter().all(|p| *p == [255, 0, 0, 255]));
        // A cropped stamp layer is shown at its place on the canvas.
        let t = doc.layer_thumbnail(LayerId(101), 48).unwrap();
        assert_eq!((t.width, t.height), (48, 48));
        assert_eq!(t.get(47, 47)[3], 0);
        assert_eq!(t.get(0, 0)[3], 0);
        assert!(t.get(15, 13)[3] > 0, "canvas (30, 26) lies inside the cache");
        // The set thumbnail shows the composite (red on top).
        let t = doc.set_thumbnail(S0, 16).unwrap();
        assert_eq!((t.width, t.height), (16, 16));
        assert!(t.data.as_chunks::<4>().0.iter().all(|p| *p == [255, 0, 0, 255]));
        let t = doc.set_thumbnail(S1, 16).unwrap();
        assert_eq!(t.get(0, 15), [255; 4]);
        assert_eq!(flatten_layers(&doc.sets[1].layers, SIZE, SIZE, false), *doc.composite(S1).unwrap());
    }
}
