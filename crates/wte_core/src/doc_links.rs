// Owner: stickers P5c (STICKER_PLAN 5.3-5.5, 13.1, 13.2; ARCHITECTURE 5.28 "link", 6.13). The editor links of a
// document's stickers: the link map (outside the history, R6; changes set the modified flag), the sticker hash the
// state table compares (`sticker_link_hash`), `link_state`, `prepare_link_file` / `finish_link_write` (the only way
// a link file's content is made; the app writes it with `link::write_link_file`), and `apply_link_update` (one undo
// step "Update Linked Sticker").
//
// Choices the plan leaves open, documented at the items below:
// - The sticker hash covers what goes into the PSD (artboard, layers with their content, transform, mask,
//   adjustments, name, visibility, opacity, blend, clip), **not** layer styles, `locked`, the sticker's name, `keep`
//   or the artboard shape: those never reach the file (5.4 "the stamp's styles and the layer styles are not part of
//   the file"), so changing them in WTE is not "changed in WTE" and asks for no Send.
// - The hash is built from the project-format entries (serde JSON of the layer entries with pictures and masks as
//   their size and CRC-32), so it is the same after a save and a load and across builds that keep the format.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::document::{Commit, Document};
use crate::error::EditError;
use crate::ids::{BlobId, StickerId};
use crate::link::map::import_fitted;
use crate::link::{
    FileStamp, ImportContext, ImportProblem, LinkFile, LinkImport, LinkLayer, LinkLayerWte, LinkMode, LinkProblem, LinkState,
    StickerLink, UPDATE_LABEL, export, import, link_scale, link_state,
};
use crate::math::fnv1a64;
use crate::project::manifest::FileRef;
use crate::project::sticker_entries::{
    AdjustmentEntry, ShapeEntry, StickerContentEntry, StickerLayerEntry, artboard_entry, transform_entry,
};
use crate::psd::{PsdDoc, PsdError, write_sticker_to_vec};
use crate::stamp::BakeQuality;
use crate::sticker::model::{PicturePixels, StickerContent, StickerDoc, StickerLayer};
use crate::sticker::pipeline::sticker_scale;
use crate::sticker::render::StickerRenderInput;

/// A link file's content made by [`Document::prepare_link_file`], not written yet. The app writes `bytes` with
/// `link::write_link_file` and then calls [`Document::finish_link_write`] with the written stamp; if the write fails
/// nothing is changed.
#[derive(Clone, Debug)]
pub struct PreparedLink {
    pub sticker: StickerId,
    pub bytes: Vec<u8>,
    pub crc32: u32,
    /// The link as it will be once the file is written (`synced` gets the written stamp).
    pub link: StickerLink,
}

/// What [`Document::apply_link_update`] did.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkApplied {
    /// The sticker took the file (one undo step "Update Linked Sticker", or none when nothing changed).
    Updated {
        mode: LinkMode,
        /// The merged toast (5.4) when the file came back as one picture for a reason.
        toast: Option<String>,
        /// The colour-profile note (13.2; the app shows it once per link).
        profile_note: Option<String>,
        /// Lines for the log (ids rules).
        log: Vec<String>,
    },
    /// Nothing was applied; the badge shows this state.
    Problem(LinkState),
}

impl Document {
    // ----- queries -----

    /// The link of sticker `id`.
    pub fn link(&self, id: StickerId) -> Option<&StickerLink> {
        self.sticker_state.links.get(&id)
    }

    /// Every link, ascending sticker id (links of stickers no longer in the document included: an undo can bring the
    /// sticker back; a save keeps only links of saved stickers).
    pub fn links(&self) -> impl Iterator<Item = (StickerId, &StickerLink)> {
        self.sticker_state.links.iter().map(|(id, l)| (*id, l))
    }

    /// +1 at every change of the link map (`DocumentSnapshot::links_revision`, `mark_links_saved`).
    pub fn links_revision(&self) -> u64 {
        self.sticker_state.links_revision
    }

    /// Records that the links of revision `revision` (`DocumentSnapshot::links_revision` of the snapshot a save
    /// wrote) are on disk. `mark_saved` does not touch the links: a background save marks both.
    pub fn mark_links_saved(&mut self, revision: u64) {
        self.sticker_state.links_saved = revision;
    }

    /// The hash of sticker `id` that `StickerLink::synced_sticker` holds (module docs): every field that goes into
    /// the PSD by value, pictures and masks by size and CRC-32 (computed once per picture version and cached).
    pub fn sticker_link_hash(&mut self, id: StickerId) -> Result<u64, EditError> {
        let doc = Arc::clone(self.sticker(id).ok_or(EditError::UnknownSticker)?);
        Ok(self.hash_of(&doc))
    }

    /// The state of sticker `id`'s link (5.5) for the file stamp the app observed (`None` = the file is missing).
    pub fn link_state(&mut self, id: StickerId, observed: Option<&FileStamp>) -> Result<LinkState, EditError> {
        let hash = self.sticker_link_hash(id)?;
        let link = self.link(id).ok_or(EditError::InvalidParam("the sticker is not linked"))?;
        Ok(link_state(link, observed, hash))
    }

    // ----- making and changing links -----

    /// The content of sticker `id`'s link file (5.4 "WTE -> PSD"): the PSD bytes at the link scale (the sticker's
    /// Final render scale over its stamps and its picture layers' scales, 1 / 2 / 4) and the link as it will be
    /// once written: the ids of an existing link are kept, its file and editor replaced by `file` / `editor` (the
    /// app passes the existing ones for a Send). Changes nothing. Errors: `UnknownSticker`, `InvalidParam` for a
    /// sticker a PSD cannot hold.
    pub fn prepare_link_file(&mut self, id: StickerId, file: LinkFile, editor: &str) -> Result<PreparedLink, EditError> {
        let doc = Arc::clone(self.sticker(id).ok_or(EditError::UnknownSticker)?);
        let final_scale = self
            .sticker_users(id)
            .iter()
            .filter_map(|u| self.stamp(*u))
            .map(|u| sticker_scale(&doc, self.texels_of(u).y, BakeQuality::Full))
            .fold(1.0f32, f32::max);
        let s = link_scale(&doc, final_scale);
        let input = self.link_render_input(&doc)?;
        let previous = self.link(id).cloned();
        let mut made = export(&input, s, previous.as_ref()).map_err(psd_invalid)?;
        let bytes = write_sticker_to_vec(&made.psd).map_err(psd_invalid)?;
        self.fill_link_wte(&doc, &mut made.layer_map);
        let synced = FileStamp::of_bytes(&bytes, None);
        let link = StickerLink {
            file,
            editor: editor.to_owned(),
            mode: LinkMode::Layers,
            scale: made.scale as f32,
            synced,
            synced_sticker: self.hash_of(&doc),
            layer_map: made.layer_map,
            next_lyid: made.next_lyid,
            editor_native: false,
            problem: None,
        };
        Ok(PreparedLink { sticker: id, crc32: synced.crc32, bytes, link })
    }

    /// Takes a prepared link file that was written (`written` = the stamp `write_link_file` returned): the link is
    /// made or updated, in sync. `InvalidParam` when the written content is not the prepared one.
    pub fn finish_link_write(&mut self, prepared: PreparedLink, written: FileStamp) -> Result<(), EditError> {
        if !written.same_content(&prepared.link.synced) {
            return Err(EditError::InvalidParam("the written link file is not the prepared one"));
        }
        let mut link = prepared.link;
        link.synced = written;
        self.put_link(prepared.sticker, Some(link));
        Ok(())
    }

    /// "Unlink" (5.1): stop following the file (the picture stays, the PSD stays on disk). No-op without a link.
    pub fn unlink(&mut self, id: StickerId) {
        if self.sticker_state.links.contains_key(&id) {
            self.put_link(id, None);
        }
    }

    /// "Relink" of a broken external link, or the editor set at an "Edit in" / PSD-handle drag (13.1).
    pub fn set_link_file_and_editor(&mut self, id: StickerId, file: Option<LinkFile>, editor: Option<&str>) -> Result<(), EditError> {
        let mut link = self.link(id).cloned().ok_or(EditError::InvalidParam("the sticker is not linked"))?;
        if let Some(file) = file {
            link.file = file;
        }
        if let Some(editor) = editor {
            editor.clone_into(&mut link.editor);
        }
        if self.link(id) != Some(&link) {
            self.put_link(id, Some(link));
        }
        Ok(())
    }

    // ----- applying the file -----

    /// Applies a file the watcher read (5.4): the sticker takes it layer by layer or as one picture, one undo step
    /// "Update Linked Sticker" (none when nothing changed); the link is in sync with `stamp` afterwards (outside the
    /// history: an undo of the update leaves "changed in WTE" and nothing is applied again by itself, R6). When the
    /// file cannot be applied (needs a merged picture, canvas too large, over the limits) nothing changes but the
    /// link's problem, and the result is the badge state. `editor_name` / `file_name` are for the texts. Errors:
    /// `UnknownSticker`, `InvalidParam` (not linked), and the errors of `set_sticker` (a locked user, a stroke).
    pub fn apply_link_update(&mut self, id: StickerId, psd: &PsdDoc, stamp: FileStamp, editor_name: &str, file_name: &str) -> Result<LinkApplied, EditError> {
        self.apply_link(id, psd, stamp, editor_name, file_name, false)
    }

    /// "Fit it into the sticker" (5.4, review A18): the file as one merged picture at the scale that fits the canvas
    /// into the artboard limit. As `apply_link_update` otherwise.
    pub fn apply_link_fit(&mut self, id: StickerId, psd: &PsdDoc, stamp: FileStamp, editor_name: &str, file_name: &str) -> Result<LinkApplied, EditError> {
        self.apply_link(id, psd, stamp, editor_name, file_name, true)
    }

    /// Records that the file with `stamp` could not be read: a refused colour mode or depth (13.2: its sentence on
    /// the badge), too large, or damaged. A truncated file is not a problem (the watcher waits for it). Nothing else
    /// changes. Returns the badge state.
    pub fn link_file_refused(&mut self, id: StickerId, err: &PsdError, stamp: FileStamp, file_name: &str) -> Result<LinkState, EditError> {
        let mut link = self.link(id).cloned().ok_or(EditError::InvalidParam("the sticker is not linked"))?;
        let crc32 = stamp.crc32;
        link.problem = match err {
            PsdError::Unsupported(r) => Some(LinkProblem::Refused { crc32, sentence: r.sentence(file_name) }),
            PsdError::TooLarge { width, height } => Some(LinkProblem::TooLarge { crc32, width: *width, height: *height }),
            PsdError::Truncated => None,
            other => Some(LinkProblem::Unreadable { crc32, reason: other.to_string() }),
        };
        self.put_link(id, Some(link));
        self.link_state(id, Some(&stamp))
    }

    // ----- internals -----

    fn apply_link(&mut self, id: StickerId, psd: &PsdDoc, stamp: FileStamp, editor_name: &str, file_name: &str, fit: bool) -> Result<LinkApplied, EditError> {
        let doc = Arc::clone(self.sticker(id).ok_or(EditError::UnknownSticker)?);
        let mut link = self.link(id).cloned().ok_or(EditError::InvalidParam("the sticker is not linked"))?;
        let mut blob_sizes: Vec<(BlobId, [u32; 2])> = Vec::new();
        for l in &doc.layers {
            if let StickerContent::Picture(PicturePixels::Blob(b)) = &l.content
                && let Ok(size) = self.image_size(*b)
            {
                blob_sizes.push((*b, size));
            }
        }
        let layer_hashes: Vec<_> = doc.layers.iter().map(|l| (l.id, self.link_layer_hash(l))).collect();
        let editor_key = link.editor.clone();
        let cx = ImportContext { editor_key: &editor_key, editor_name, file_name, blob_sizes: &blob_sizes, layer_hashes: &layer_hashes };
        let result: Result<LinkImport, ImportProblem> =
            if fit { import_fitted(psd, &doc, &link, &cx) } else { import(psd, &doc, &link, &cx) };
        let crc32 = stamp.crc32;
        let made = match result {
            Ok(made) => made,
            Err(problem) => {
                link.problem = Some(match problem {
                    ImportProblem::NeedsMerged => LinkProblem::NeedsMerged { crc32 },
                    ImportProblem::CanvasTooLarge { width, height } => LinkProblem::CanvasTooLarge { crc32, width, height },
                    ImportProblem::TooLarge { width, height } => LinkProblem::TooLarge { crc32, width, height },
                });
                self.put_link(id, Some(link));
                return Ok(LinkApplied::Problem(self.link_state(id, Some(&stamp))?));
            }
        };
        let LinkImport { sticker, mode, scale, mut layer_map, next_lyid, editor_native, toast, profile_note, log, .. } = made;
        self.set_sticker(crate::doc_stickers::StickerRef::Id(id), move |d| *d = sticker, UPDATE_LABEL, Commit::Final)?;
        let doc = Arc::clone(self.sticker(id).ok_or(EditError::UnknownSticker)?);
        self.fill_link_wte(&doc, &mut layer_map);
        link.synced = stamp;
        link.synced_sticker = self.hash_of(&doc);
        link.mode = mode;
        link.scale = scale;
        link.layer_map = layer_map;
        link.next_lyid = next_lyid;
        link.editor_native = editor_native;
        link.problem = None;
        self.put_link(id, Some(link));
        Ok(LinkApplied::Updated { mode, toast, profile_note, log })
    }

    /// Sets or removes a link; a change bumps the links revision (the modified flag). A change of `problem` alone
    /// does not (check-P5c): problems are not saved, so a refused file must not ask to save the project.
    pub(crate) fn put_link(&mut self, id: StickerId, link: Option<StickerLink>) {
        let st = &mut self.sticker_state;
        let saved_part = |l: &StickerLink| StickerLink { problem: None, ..l.clone() };
        let changed = match link {
            Some(link) => st.links.insert(id, link.clone()).is_none_or(|old| saved_part(&old) != saved_part(&link)),
            None => st.links.remove(&id).is_some(),
        };
        if changed {
            st.links_revision += 1;
        }
    }

    /// Replaces the whole link map without touching the revision (load).
    pub(crate) fn set_loaded_links(&mut self, links: BTreeMap<StickerId, StickerLink>) {
        self.sticker_state.links = links;
    }

    /// The render input of a link export: like `sticker_render_input`, with the fonts of hidden text layers too
    /// (hidden layers are written).
    pub(crate) fn link_render_input(&mut self, doc: &Arc<StickerDoc>) -> Result<StickerRenderInput, EditError> {
        let mut input = self.sticker_render_input(doc)?;
        for layer in &doc.layers {
            if let StickerContent::Text(spec) = &layer.content
                && !layer.visible
            {
                let (own, fallback) = self.text_fonts_owned(spec)?;
                input.fonts.push((layer.id, own, fallback));
            }
        }
        Ok(input)
    }

    /// CRC-32 of a picture or mask's bytes, cached by version (versions are never reused).
    fn pixel_crc(&mut self, version: u64, bytes: &[u8]) -> u32 {
        if version != 0
            && let Some(c) = self.sticker_state.link_crcs.get(&version)
        {
            return *c;
        }
        let c = crc32fast::hash(bytes);
        if version != 0 {
            if self.sticker_state.link_crcs.len() > 4096 {
                self.sticker_state.link_crcs.clear();
            }
            self.sticker_state.link_crcs.insert(version, c);
        }
        c
    }

    /// The hash of `doc` (`sticker_link_hash`).
    fn hash_of(&mut self, doc: &StickerDoc) -> u64 {
        let mut h = fnv1a64(0xcbf2_9ce4_8422_2325, b"wte sticker link hash 1");
        for v in artboard_entry(&doc.artboard) {
            h = fnv1a64(h, &v.to_le_bytes());
        }
        for l in &doc.layers {
            let entry = self.link_layer_entry(l);
            match serde_json::to_vec(&entry) {
                Ok(bytes) => h = fnv1a64(h, &bytes),
                Err(e) => log::warn!("sticker layer {} could not be hashed: {e}", l.id.0),
            }
        }
        h
    }

    /// The project-format entry of `l` with pictures and masks as their size and CRC-32 and no styles (the hashes).
    fn link_layer_entry(&mut self, l: &StickerLayer) -> StickerLayerEntry {
        let file = |w: u32, h: u32, len: usize, crc32: u32| FileRef { entry: format!("{w}x{h}"), size: len as u64, crc32 };
        let content = match &l.content {
            StickerContent::Picture(PicturePixels::Blob(b)) => StickerContentEntry::Picture { blob: Some(*b), file: None },
            StickerContent::Picture(PicturePixels::Owned { pixels, version }) => {
                let crc = self.pixel_crc(*version, &pixels.data);
                StickerContentEntry::Picture { blob: None, file: Some(file(pixels.width, pixels.height, pixels.data.len(), crc)) }
            }
            StickerContent::Text(spec) => StickerContentEntry::Text((&**spec).into()),
            StickerContent::Shape(shape) => StickerContentEntry::Shape(ShapeEntry::from(shape)),
        };
        let mask = l.mask.as_ref().map(|m| {
            let crc = self.pixel_crc(m.version, &m.pixels.data);
            file(m.pixels.width, m.pixels.height, m.pixels.data.len(), crc)
        });
        StickerLayerEntry {
            id: l.id,
            name: l.name.clone(),
            visible: l.visible,
            locked: false,
            opacity: l.opacity,
            blend: l.blend,
            transform: transform_entry(&l.transform),
            content,
            clip: l.clip,
            mask,
            adjustments: l.adjustments.iter().map(AdjustmentEntry::from).collect(),
            styles: None,
        }
    }

    /// check-P5c: the hash of what makes layer `l`'s pixels in the file apart from its place (content, mask,
    /// adjustments, scale, rotation): `LinkLayerWte::hash`. Name, visibility, opacity, blend and clip are left out
    /// (an import takes them from the file anyway), and so are the styles (never in the file).
    pub fn link_layer_hash(&mut self, l: &StickerLayer) -> u64 {
        let mut entry = self.link_layer_entry(l);
        entry.id = crate::ids::StickerLayerId(0);
        entry.name.clear();
        (entry.visible, entry.opacity, entry.blend, entry.clip) = (true, 1.0, crate::layer::BlendMode::Normal, false);
        let mut t = l.transform;
        t.translate = glam::Vec2::ZERO;
        entry.transform = transform_entry(&t);
        match serde_json::to_vec(&entry) {
            Ok(bytes) => fnv1a64(fnv1a64(0xcbf2_9ce4_8422_2325, b"wte link layer hash 1"), &bytes),
            Err(e) => {
                log::warn!("sticker layer {} could not be hashed: {e}", l.id.0);
                0
            }
        }
    }

    /// Fills `LinkLayer::wte` of every map entry from the layer of `doc` it names (check-P5c).
    fn fill_link_wte(&mut self, doc: &StickerDoc, map: &mut [LinkLayer]) {
        let tl = glam::Vec2::new(doc.artboard.x0 as f32, doc.artboard.y0 as f32);
        for e in map {
            e.wte = doc.layer(e.layer).map(|l| LinkLayerWte { hash: self.link_layer_hash(l), at: (l.transform.translate - tl).to_array() });
        }
    }
}

/// A sticker a PSD cannot hold.
fn psd_invalid(e: PsdError) -> EditError {
    match e {
        PsdError::Invalid(why) => EditError::InvalidParam(why),
        _ => EditError::InvalidParam("the sticker cannot be written as a PSD file"),
    }
}
