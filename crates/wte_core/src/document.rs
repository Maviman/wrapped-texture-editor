//! The document (ARCHITECTURE.md 5.14): data model, lookup accessors, id allocation, revision tracking and the
//! shared mutation helpers. Commands live in `doc_build.rs`, `doc_layers.rs`, `doc_stamps.rs`, `history.rs`,
//! `composite.rs`, `stroke.rs` and `fill.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::blob::BlobStore;
use crate::bvh::{Bvh, Cull, Hit, TriMask};
use crate::error::{EditError, ImportWarning};
use crate::history::History;
use crate::ids::{BlobId, LayerId, StampId, StickerId, TextureSetId};
use crate::import::{ImportOptions, SourceBundle};
use crate::layer::{Layer, LayerKind, TextureSet};
use crate::math::{Aabb, IRect, Ray};
use crate::mesh::MeshData;
use crate::pixel_ops::PremulMips;
use crate::stamp::{StampObject, StampSource};
use crate::sticker::StickerDoc;
use crate::texel_map::TexelMap;
use crate::text::{FontCache, FontRef, TextFrame, TextRaster, TextSpec};
use crate::wrap_param::WrapParam;

/// How a continuous edit (slider drag, gizmo drag) is applied.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Commit {
    /// Apply, no history entry yet.
    Preview,
    /// Apply and push one history entry.
    Final,
}

/// The complete editing state of one project. Lives on the UI thread; `Send` so it can be built on a worker.
pub struct Document {
    pub name: String,
    pub mesh: Arc<MeshData>,
    pub bvh: Arc<Bvh>,
    /// Index == `TextureSetId.0`.
    pub sets: Vec<TextureSet>,
    /// Ascending id.
    pub stamps: Vec<StampObject>,
    pub blobs: BlobStore,
    /// Original model files (R7).
    pub source: Arc<SourceBundle>,
    pub import_options: ImportOptions,
    /// Not persisted.
    pub import_warnings: Vec<ImportWarning>,
    pub history: History,
    /// Opaque to the core; round-tripped in the manifest.
    pub ui_state: serde_json::Value,
    pub(crate) next_id: u64,
    pub(crate) revision: u64,
    /// The history state id (`History::state`) whose content is on disk (`mark_saved`).
    pub(crate) saved_revision: u64,
    pub(crate) stroke: Option<crate::stroke::StrokeSession>,
    /// Boxed (stickers, P3a): the structure snapshot inside it grew with the sticker map; `Document` stays small.
    pub(crate) pending_edit: Option<Box<crate::history::PendingEdit>>,
    /// Stamps whose caches are currently `BakeQuality::Preview` (5.20, 6.7).
    pub(crate) preview_baked: BTreeSet<StampId>,
    /// The deferred bakes of stamps (`preview_stale`, round 5: `unbaked`); boxed so that `Document` stays small.
    pub(crate) bakes: Box<BakeSets>,
    /// When set, a `Commit::Preview` stamp edit only records the change; `bake_stale_previews` bakes it later.
    pub(crate) defer_preview_bakes: bool,
    /// Round 5: when set, ending a stamp edit marks the stamp `unbaked` instead of baking it (`next_final_bake`).
    pub(crate) defer_final_bakes: bool,
    /// Images of the project file this document was loaded from or last saved to (`PngReuse`); not persisted.
    pub(crate) png_reuse: Option<Arc<crate::project::PngReuse>>,
    /// Source of `Layer::version` values; not persisted.
    pub(crate) version_counter: u64,
    pub(crate) fonts: FontCache,
    /// Decoded stamp images.
    pub(crate) image_mips: HashMap<BlobId, Arc<PremulMips>>,
    /// Rasterized text per stamp.
    pub(crate) text_mips: HashMap<StampId, (TextSpec, Arc<PremulMips>)>,
    /// The round-4 text caches (FEATURE_TEXT 5.11, 5.13, 5.14); boxed so that `Document` stays small.
    pub(crate) text_caches: Box<TextCaches>,
    /// Objects-tabs: one flag per object (`mesh.objects`). View state: not in history, snapshots or the manifest.
    pub(crate) hidden_objects: Vec<bool>,
    /// Objects-tabs: +1 at every `set_hidden_objects` that changed something.
    pub(crate) visibility_generation: u64,
    /// Stickers (STICKER_PLAN 2.2, 2.7, 2.8): the sticker map, the picture-version counter and the derived sticker
    /// caches; boxed so that `Document` stays small.
    pub(crate) sticker_state: Box<StickerState>,
}

/// The sticker part of a document (STICKER_PLAN 2.2, 2.7).
#[derive(Default)]
pub(crate) struct StickerState {
    /// The layered stickers stamps show (`StampSource::Sticker`), shared by `Arc`; history snapshots hold `Arc` clones.
    /// Part of the structure state.
    pub(crate) map: BTreeMap<StickerId, Arc<StickerDoc>>,
    /// Source of picture versions (`next_pixel_version`, 2.2); not persisted, never reused.
    pub(crate) pixel_version: u64,
    /// Derived sticker data (renders, layer rasters, decoded blob pictures, styled stamp pictures); not persisted, not
    /// in the history.
    pub(crate) cache: crate::doc_stickers::StickerCaches,
    /// The open sticker stroke (P3b, `sticker_stroke_begin`); excludes a texture stroke and every other command.
    pub(crate) stroke: Option<Box<crate::doc_sticker_edits::StickerStroke>>,
    /// Stickers whose pixels an undo or redo just swapped (`Edit::StickerPixels`): their users are re-baked once the
    /// whole entry is applied (`settle_sticker_pixel_edits`).
    pub(crate) swapped: BTreeSet<StickerId>,
}

/// Most parameterizations a document keeps (least recently used dropped first).
pub(crate) const WRAP_CACHE_SIZE: usize = 16;

/// The parameterizations of wrapped stamps (round 6, 4.4), least recently used first. An entry is used only when it
/// `serves` the stamp's current placement (same mesh `Arc`, anchor, frame, options and bucketed reach), so an entry
/// left behind by an undo or a removed stamp is never wrong; `remove_stamp` drops its entry and every insertion drops
/// entries of stamps that no longer exist.
#[derive(Default)]
pub(crate) struct WrapCache {
    entries: Vec<(StampId, Arc<WrapParam>)>,
    /// Parameterizations built by this document's bakes (tests: a reuse does not count).
    pub(crate) fresh: u64,
}

impl WrapCache {
    /// The entry of `id` (marked as most recently used).
    pub(crate) fn get(&mut self, id: StampId) -> Option<Arc<WrapParam>> {
        let i = self.entries.iter().position(|e| e.0 == id)?;
        let entry = self.entries.remove(i);
        let param = Arc::clone(&entry.1);
        self.entries.push(entry);
        Some(param)
    }

    /// The entry of `id` without changing the order.
    pub(crate) fn peek(&self, id: StampId) -> Option<&Arc<WrapParam>> {
        self.entries.iter().find(|e| e.0 == id).map(|e| &e.1)
    }

    /// Stores `param` for `id`; entries of ids `exists` rejects are dropped, then the oldest beyond the size limit.
    pub(crate) fn insert(&mut self, id: StampId, param: Arc<WrapParam>, exists: impl Fn(StampId) -> bool) {
        self.entries.retain(|e| e.0 != id && exists(e.0));
        self.entries.push((id, param));
        while self.entries.len() > WRAP_CACHE_SIZE {
            self.entries.remove(0);
        }
    }

    pub(crate) fn remove(&mut self, id: StampId) {
        self.entries.retain(|e| e.0 != id);
    }

    /// Drops every entry (objects-tabs: the hidden objects changed; `serves` does not know about visibility).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

/// The round-4 text caches of a document (FEATURE_TEXT 5.11, 5.13, 5.14). Not persisted, not part of the history.
#[derive(Default)]
pub(crate) struct TextCaches {
    /// Frame of the current spec per text stamp (`stamp_text_frame`); a stale entry is recognised by its spec.
    pub(crate) frames: HashMap<StampId, (TextSpec, TextFrame)>,
    /// Text stamps whose cached picture in `text_mips` is a preview, not the full picture.
    pub(crate) preview_ids: BTreeSet<StampId>,
    /// Fonts handed out by `register_font` in this session (`find_font`).
    pub(crate) font_refs: Vec<FontRef>,
    /// The one-entry cache of `text_preview`: spec, pixel limit, picture.
    pub(crate) preview: Option<(TextSpec, u32, Arc<TextRaster>)>,
}

/// The deferred bakes of a document's stamps.
#[derive(Default)]
pub(crate) struct BakeSets {
    /// Deferred Preview bakes (`set_defer_preview_bakes`): stamps whose Preview edit changed them but whose caches
    /// still show an earlier state. A subset of `preview_baked`.
    pub(crate) preview_stale: BTreeSet<StampId>,
    /// Deferred Final bakes (`set_defer_final_bakes`, 5.20, round 5): stamps whose Full bake is still due; their
    /// caches show an earlier state (a Preview, the state before the edit, or nothing for a new stamp). Part of the
    /// structure state: history snapshots carry it (`StructureSnapshot::unbaked`). Empty while the mode is off.
    pub(crate) unbaked: BTreeSet<StampId>,
    /// Round 6: the parameterizations of wrapped stamps (4.4). Derived data: not in snapshots, not saved, not part of
    /// the structure state (kept in this boxed struct so that `Document` stays small).
    pub(crate) wrap_params: WrapCache,
    /// Round 8 (3.4): wrapped stamps' Preview bakes go to jobs (`set_defer_wrap_previews`).
    pub(crate) defer_wrap_previews: bool,
    /// Round 8: the serial of the open pending edit (bumped when the first stamp joins `preview_baked`); a wrap Preview
    /// job's output applies only within the edit it was made in.
    pub(crate) edit_serial: u64,
    /// Round 9 (package B, T7): the welded graph of the mesh for wrapped stamps, built lazily (by the first wrap job or
    /// the warm-up job) and shared by `Arc` with the jobs; a new mesh gets a new cell (`Document::wrap_mesh_cell`).
    pub(crate) wrap_mesh: Option<Arc<crate::wrap_param::WrapMeshCell>>,
}

// Compile-time thread-safety checks (section 4.6).
fn _assert_send<T: Send>() {}
fn _assert_send_sync_static<T: Send + Sync + 'static>() {}
const _: fn() = || {
    _assert_send::<Document>();
    _assert_send_sync_static::<DocumentSnapshot>();
};

impl Document {
    /// The texture set with this id.
    pub fn set(&self, id: TextureSetId) -> Option<&TextureSet> {
        self.sets.get(id.index())
    }

    /// Mutable access to the texture set with this id.
    pub(crate) fn set_mut(&mut self, id: TextureSetId) -> Option<&mut TextureSet> {
        self.sets.get_mut(id.index())
    }

    /// Ids of all texture sets, ascending.
    pub fn set_ids(&self) -> Vec<TextureSetId> {
        (0..self.sets.len() as u32).map(TextureSetId).collect()
    }

    /// The set containing the layer and the layer's index in that set (0 = bottom).
    pub fn find_layer(&self, id: LayerId) -> Option<(TextureSetId, usize)> {
        self.sets.iter().enumerate().find_map(|(s, set)| set.layer_index(id).map(|i| (TextureSetId(s as u32), i)))
    }

    /// The layer with this id, in whichever set it lives.
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        let (set, index) = self.find_layer(id)?;
        self.sets[set.index()].layers.get(index)
    }

    /// Mutable access to the layer with this id.
    pub(crate) fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        let (set, index) = self.find_layer(id)?;
        self.sets[set.index()].layers.get_mut(index)
    }

    /// The stamp with this id.
    pub fn stamp(&self, id: StampId) -> Option<&StampObject> {
        self.stamps.iter().find(|s| s.id == id)
    }

    /// The layers linked to a stamp, ascending set id.
    pub fn stamp_layers(&self, id: StampId) -> Vec<(TextureSetId, LayerId)> {
        let mut out = Vec::new();
        for (s, set) in self.sets.iter().enumerate() {
            for layer in &set.layers {
                if layer.stamp_id() == Some(id) {
                    out.push((TextureSetId(s as u32), layer.id));
                }
            }
        }
        out
    }

    /// True when any layer linked to the stamp is locked: a locked linked layer protects the whole stamp (it cannot be
    /// moved, re-sourced, re-tinted or deleted until that layer is unlocked; renaming stays allowed, as for layers).
    pub fn stamp_locked(&self, id: StampId) -> bool {
        self.sets.iter().flat_map(|s| &s.layers).any(|l| l.locked && l.stamp_id() == Some(id))
    }

    /// The stamp a stamp/text layer is linked to; `None` for paint layers and unknown ids.
    pub fn stamp_of_layer(&self, layer: LayerId) -> Option<StampId> {
        self.layer(layer)?.stamp_id()
    }

    /// The texel maps of all sets; index == set id.
    pub fn texel_maps(&self) -> Vec<Arc<TexelMap>> {
        self.sets.iter().map(|s| Arc::clone(&s.texel_map)).collect()
    }

    /// Mean world units per texel of a set (`texel_map.stats.texel_world`); 1.0 for an unknown set.
    pub fn texel_world_size(&self, set: TextureSetId) -> f32 {
        self.set(set).map_or(1.0, |s| s.texel_map.stats.texel_world)
    }

    /// Nearest surface hit: `Cull::None`, `t` in `[1e-5 * diag, INF)`. Hidden objects are passed through.
    pub fn raycast(&self, ray: &Ray) -> Option<Hit> {
        let t_min = 1e-5 * self.mesh.diag();
        self.bvh.raycast(&self.mesh, ray, t_min, f32::INFINITY, Cull::None)
    }

    // ----- objects-tabs: hidden objects (ARCHITECTURE.md 5.14) -----

    /// One flag per object (`mesh.objects`), all false in a new or loaded document.
    pub fn hidden_objects(&self) -> &[bool] {
        &self.hidden_objects
    }

    /// True when `object` is hidden (false for an unknown index).
    pub fn object_hidden(&self, object: u32) -> bool {
        self.hidden_objects.get(object as usize).copied().unwrap_or(false)
    }

    /// Number of hidden objects.
    pub fn hidden_object_count(&self) -> usize {
        self.hidden_objects.iter().filter(|&&h| h).count()
    }

    /// 0 in a new document; +1 at every call of `set_hidden_objects` that changed something.
    pub fn visibility_generation(&self) -> u64 {
        self.visibility_generation
    }

    /// Replaces the hidden flags (a slice of another length is padded with false / cut). `Ok(false)` = nothing changed.
    /// `Err(EditError::StrokeInProgress)` while a stroke is open. View state: no history entry, no revision change,
    /// `is_modified` unaffected. Swaps `self.bvh` for `bvh.with_hidden(..)` (every ray and sphere query then skips the
    /// hidden triangles) and drops every cached wrap parameterization. A pending edit is left alone; bakes made from
    /// now on see the new visibility, jobs made before keep the BVH they cloned.
    pub fn set_hidden_objects(&mut self, hidden: &[bool]) -> Result<bool, EditError> {
        self.guard_no_stroke()?;
        let n = self.mesh.objects.len();
        let mut flags: Vec<bool> = hidden.iter().copied().take(n).collect();
        flags.resize(n, false);
        if flags == self.hidden_objects {
            return Ok(false);
        }
        let mask = flags.iter().any(|&h| h).then(|| {
            let mesh = &self.mesh;
            Arc::new(TriMask::from_fn(mesh.tri_count(), |t| flags.get(mesh.tri_object_of(t) as usize).copied().unwrap_or(false)))
        });
        self.bvh = Arc::new(self.bvh.with_hidden(mask));
        self.hidden_objects = flags;
        self.visibility_generation += 1;
        self.bakes.wrap_params.clear();
        Ok(true)
    }

    /// Union of the bounds of the visible objects; `mesh.bounds` when nothing is hidden or everything is.
    pub fn visible_bounds(&self) -> Aabb {
        let all: Vec<u32> = (0..self.mesh.object_count()).collect();
        match self.objects_bounds(&all) {
            Some(b) if self.hidden_object_count() > 0 => b,
            _ => self.mesh.bounds,
        }
    }

    /// Union of the bounds of the listed objects that are visible; `None` when there is none (unknown indices are
    /// ignored).
    pub fn objects_bounds(&self, objects: &[u32]) -> Option<Aabb> {
        let mut out: Option<Aabb> = None;
        for &o in objects {
            let Some(info) = self.mesh.objects.get(o as usize) else { continue };
            if self.object_hidden(o) || info.bounds.is_empty() {
                continue;
            }
            out = Some(out.map_or(info.bounds, |b| b.union(&info.bounds)));
        }
        out
    }

    /// Bumped by every content mutation, undo and redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Id of the current history state (section 6.7): fresh for every new history entry, restored by undo and redo.
    /// Equals `revision()` until the first undo or redo after the newest push (so right after load or build).
    pub fn history_state(&self) -> u64 {
        self.history.state
    }

    /// True when the content differs from the last `mark_saved` state: `history_state() != saved`. Undo and redo back
    /// to the saved state clear it; a saved state evicted from the history, or left behind by a new edit after undoing
    /// below it, can never be reached again. A pending `Commit::Preview` edit is not counted here (see
    /// `has_pending_edit`).
    pub fn is_modified(&self) -> bool {
        self.history.state != self.saved_revision
    }

    /// Records that the history state `state` (`history_state()`, or `DocumentSnapshot::history_state` of the snapshot
    /// a background save wrote) is what is on disk.
    pub fn mark_saved(&mut self, state: u64) {
        self.saved_revision = state;
    }

    /// Allocates a fresh id for a layer or stamp (monotonic, never reused, persisted as `next_id`).
    pub(crate) fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// `revision += 1`.
    pub(crate) fn touch(&mut self) {
        self.revision += 1;
    }

    /// Immutable copy for background save/export. O(#layers): `Arc` clones only.
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            name: self.name.clone(),
            mesh: Arc::clone(&self.mesh),
            source: Arc::clone(&self.source),
            import_options: self.import_options,
            sets: self
                .sets
                .iter()
                .map(|s| SetSnapshot {
                    id: s.id,
                    name: s.name.clone(),
                    width: s.width,
                    height: s.height,
                    layers: s.layers.clone(),
                    active: s.active,
                    texel_map: Arc::clone(&s.texel_map),
                })
                .collect(),
            stamps: self.stamps.clone(),
            blobs: self.blobs.clone(),
            ui_state: self.ui_state.clone(),
            next_id: self.next_id,
            revision: self.revision,
            history_state: self.history.state,
            png_reuse: self.png_reuse.clone(),
        }
    }

    /// Records the images of the project file just written (`SaveStats::reuse`) for the next save. `None` forgets.
    pub fn set_png_reuse(&mut self, reuse: Option<crate::project::PngReuse>) {
        self.png_reuse = reuse.map(Arc::new);
    }

    /// `Layer::version` of that layer.
    pub fn layer_version(&self, id: LayerId) -> Option<u64> {
        self.layer(id).map(Layer::version)
    }

    /// `Err(EditError::StrokeInProgress)` while a stroke is open (a texture stroke or a sticker stroke: they exclude
    /// each other, STICKER_PLAN 2.8). First statement of every history-producing command.
    pub(crate) fn guard_no_stroke(&self) -> Result<(), EditError> {
        if self.any_stroke() { Err(EditError::StrokeInProgress) } else { Ok(()) }
    }

    /// True while a texture stroke or a sticker stroke (`sticker_stroke_begin`) is open.
    pub(crate) fn any_stroke(&self) -> bool {
        self.stroke.is_some() || self.sticker_state.stroke.is_some()
    }

    /// THE way to report changed pixels of one layer: clips `r` to the set, adds it to the set's `pending` region and
    /// gives the layer a fresh `version` (`version_counter += 1`). Unknown layer: no-op.
    ///
    /// A paint layer's known content bound (`Layer::content`) grows by `r` and moves to the new version.
    pub(crate) fn mark_layer_dirty(&mut self, layer: LayerId, r: IRect) {
        self.mark_layer_rects_dirty(layer, &[r]);
    }

    /// `mark_layer_dirty` for several rectangles of one layer (one new version).
    pub(crate) fn mark_layer_rects_dirty(&mut self, layer: LayerId, rects: &[IRect]) {
        let Some((set, index)) = self.find_layer(layer) else {
            return;
        };
        let version = self.fresh_version();
        let set = &mut self.sets[set.index()];
        let full = set.full_rect();
        for r in rects {
            set.mark_dirty(*r);
        }
        let target = &mut set.layers[index];
        target.content = match target.known_content() {
            Some(bound) if target.is_raster() => {
                Some((version, rects.iter().fold(bound, |acc, r| acc.union(&r.intersect(&full)))))
            }
            _ => None,
        };
        target.version = version;
    }

    /// A never-before-used `Layer::version` value, for layers created by a command.
    pub(crate) fn fresh_version(&mut self) -> u64 {
        self.version_counter += 1;
        self.version_counter
    }

    /// A never-before-used picture version (STICKER_PLAN 2.2): owned sticker pictures and masks get one at every
    /// change of their bytes. Monotonic for the life of the document, never reused, not saved.
    pub fn next_pixel_version(&mut self) -> u64 {
        self.sticker_state.pixel_version += 1;
        self.sticker_state.pixel_version
    }

    /// `debug_assert!`s the structure invariants of section 6.5. Called at the end of `push_structure_edit`,
    /// `restore_structure`, `flush_pending_edit` and `from_parts`; a no-op in release builds.
    pub(crate) fn debug_check_structure(&self) {
        if !cfg!(debug_assertions) {
            return;
        }

        // `stamps` is sorted by id (strictly ascending, so ids are unique).
        debug_assert!(self.stamps.windows(2).all(|w| w[0].id < w[1].id), "stamps are not sorted by unique id");

        let mut layer_ids: HashSet<LayerId> = HashSet::new();
        for (s, set) in self.sets.iter().enumerate() {
            debug_assert_eq!(set.id.index(), s, "texture set id does not match its index");

            // `active` is None or an existing layer of that set.
            if let Some(active) = set.active {
                debug_assert!(set.layer(active).is_some(), "set {s}: active layer {active:?} does not exist in the set");
            }

            for layer in &set.layers {
                // Layer ids are unique across the document.
                debug_assert!(layer_ids.insert(layer.id), "duplicate layer id {:?}", layer.id);

                match &layer.kind {
                    // Raster buffers have the set's size, or are unallocated (0 x 0); a known content bound holds
                    // every non-zero pixel.
                    LayerKind::Raster(buf) => {
                        let unallocated = buf.width == 0 && buf.height == 0 && buf.data.is_empty();
                        debug_assert!(
                            unallocated
                                || (buf.width == set.width
                                    && buf.height == set.height
                                    && buf.data.len() == set.width as usize * set.height as usize * 4),
                            "set {s}: raster layer {:?} is {}x{}, the set is {}x{}",
                            layer.id,
                            buf.width,
                            buf.height,
                            set.width,
                            set.height
                        );
                        if let Some(bound) = layer.known_content() {
                            let actual = crate::pixel_ops::nonzero_bounds_par(buf).unwrap_or(IRect::EMPTY);
                            debug_assert!(
                                actual.is_empty() || actual.intersect(&bound) == actual,
                                "set {s}: layer {:?} has pixels at {actual:?} outside its content bound {bound:?}",
                                layer.id
                            );
                        }
                    }
                    // Every stamp/text layer references an existing stamp whose source kind matches (a sticker stamp's
                    // layers are image-stamp layers).
                    LayerKind::Stamp { stamp, .. } => {
                        debug_assert!(
                            matches!(
                                self.stamp(*stamp).map(|o| &o.source),
                                Some(StampSource::Image { .. } | StampSource::Sticker { .. })
                            ),
                            "set {s}: stamp layer {:?} does not reference an existing image or sticker stamp ({stamp:?})",
                            layer.id
                        );
                    }
                    LayerKind::Text { stamp, .. } => {
                        debug_assert!(
                            matches!(self.stamp(*stamp).map(|o| &o.source), Some(StampSource::Text(_))),
                            "set {s}: text layer {:?} does not reference an existing text stamp ({stamp:?})",
                            layer.id
                        );
                    }
                }
            }
        }

        // Every stamp has exactly one linked layer in `home_set` and at most one in any other set; a sticker stamp's
        // sticker exists.
        for stamp in &self.stamps {
            if let StampSource::Sticker { sticker } = &stamp.source {
                debug_assert!(self.sticker_state.map.contains_key(sticker), "stamp {:?}: sticker {sticker:?} does not exist", stamp.id);
            }
            debug_assert!(
                stamp.home_set.index() < self.sets.len(),
                "stamp {:?}: home set {:?} does not exist",
                stamp.id,
                stamp.home_set
            );
            for (s, set) in self.sets.iter().enumerate() {
                let linked = set.layers.iter().filter(|l| l.stamp_id() == Some(stamp.id)).count();
                if s == stamp.home_set.index() {
                    debug_assert_eq!(linked, 1, "stamp {:?} has {linked} linked layers in its home set {s}", stamp.id);
                } else {
                    debug_assert!(linked <= 1, "stamp {:?} has {linked} linked layers in set {s}", stamp.id);
                }
            }
        }

        // Outside a pending edit `preview_baked` is empty.
        debug_assert!(
            self.pending_edit.is_some() || self.preview_baked.is_empty(),
            "preview-baked stamps without a pending edit: {:?}",
            self.preview_baked
        );

        // Round 5: `unbaked` holds only ids of existing stamps, and nothing while deferred Final bakes are off.
        debug_assert!(
            self.bakes.unbaked.iter().all(|id| self.stamp(*id).is_some()),
            "unbaked ids without a stamp: {:?}",
            self.bakes.unbaked
        );
        debug_assert!(
            self.defer_final_bakes || self.bakes.unbaked.is_empty(),
            "unbaked stamps while deferred Final bakes are off: {:?}",
            self.bakes.unbaked
        );
    }
}

/// Immutable copy for background save/export. `Send + Sync + 'static`.
#[derive(Clone)]
pub struct DocumentSnapshot {
    pub name: String,
    pub mesh: Arc<MeshData>,
    pub source: Arc<SourceBundle>,
    pub import_options: ImportOptions,
    pub sets: Vec<SetSnapshot>,
    pub stamps: Vec<StampObject>,
    pub blobs: BlobStore,
    pub ui_state: serde_json::Value,
    pub next_id: u64,
    pub revision: u64,
    /// `Document::history_state` at the time of the snapshot: what `mark_saved` takes once the snapshot is on disk.
    pub history_state: u64,
    /// Images of an existing project file that `project::save` may copy (`PngReuse`).
    pub png_reuse: Option<Arc<crate::project::PngReuse>>,
}

/// One texture set inside a `DocumentSnapshot`.
#[derive(Clone)]
pub struct SetSnapshot {
    pub id: TextureSetId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub active: Option<LayerId>,
    pub texel_map: Arc<TexelMap>,
}

#[cfg(test)]
mod objects_tests {
    use glam::{Vec2, Vec3};

    use super::*;
    use crate::doc_build::SampleKind;
    use crate::image_io::{PngLevel, encode_png};
    use crate::pixel::PixelBuf;
    use crate::stamp::{Placement, Projector, StampSource, SurfaceMode, WrapOptions};

    /// Objects-tabs WP2 (3.2): a visibility change drops every cached wrap parameterization (`WrapParam::serves` does
    /// not know about visibility); the next `ensure_wrap_param` builds a fresh one. A ParamOnly job made before a
    /// change is not cached when it lands after it.
    #[test]
    fn hiding_drops_cached_wrap_params() {
        let mut doc = Document::sample(SampleKind::Objects);
        let img = PixelBuf::filled(16, 16, [255; 4]);
        let blob = doc.add_image_blob("w.png", Arc::from(encode_png(16, 16, &img.data, PngLevel::Fast).unwrap())).unwrap();
        let p = Projector::from_surface(Vec3::new(-0.9, 0.0, 0.0), Vec3::Y, Vec3::NEG_Z, Vec2::new(1.2, 0.6), 0.6);
        let p = Projector { mode: SurfaceMode::Wrap, wrap: WrapOptions { jump_gaps: true, overlap: false }, ..p };
        let id = doc.add_stamp("W", StampSource::Image { blob }, Placement::Projected(p), TextureSetId(0)).unwrap();
        let (cached, fresh) = doc.wrap_cache_probe(id);
        assert!(cached.is_some());
        assert!(doc.set_hidden_objects(&[false, true, false]).unwrap());
        assert!(doc.wrap_cache_probe(id).0.is_none(), "dropped by the visibility change");
        assert!(doc.stamp_wrap_stats(id).is_none());
        assert!(doc.ensure_wrap_param(id).is_some());
        assert_eq!(doc.wrap_cache_probe(id).1, fresh + 1, "one fresh build");
        assert!(doc.wrap_cache_probe(id).0.is_some());
        // No change: the cache stays.
        assert!(!doc.set_hidden_objects(&[false, true, false]).unwrap());
        assert!(doc.wrap_cache_probe(id).0.is_some());
        // A late ParamOnly job from the old visibility is not cached.
        assert!(doc.set_hidden_objects(&[]).unwrap());
        let job = doc.wrap_param_job(id).expect("nothing cached: a ParamOnly job");
        assert!(doc.set_hidden_objects(&[false, true, false]).unwrap());
        assert!(!doc.apply_wrap_param(job.run()));
        assert!(doc.wrap_cache_probe(id).0.is_none());
        // One made with the current visibility is.
        let job = doc.wrap_param_job(id).unwrap();
        assert!(doc.apply_wrap_param(job.run()));
        assert!(doc.wrap_cache_probe(id).0.is_some());
    }
}
