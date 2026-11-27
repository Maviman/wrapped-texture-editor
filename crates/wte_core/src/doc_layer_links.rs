// Owner: stickers P12 (STICKER_PLAN 5.9, 13.9 "P12"; ARCHITECTURE 5.14, 5.28 "link", 6.13). The editor links of raster
// paint layers: the link map (outside the history like the sticker links, R6, sharing their revision: the modified
// flag), the layer's pixel CRC the state table compares (`layer_pixels_crc`, cached by layer version), the link file's
// content (`prepare_layer_link_file` / `finish_layer_link_write`; the app writes it with `link::write_link_file`), the
// update (`apply_layer_link_update`: one undo step "Update Linked Layer" through `replace_layer_pixels`), and the
// two layer commands P12 adds: `replace_layer_pixels` (a tile patch through `Edit::Pixels`) and
// `add_paint_layer_with_pixels` (Paste as New Paint Layer).

use std::sync::Arc;

use crate::doc_links::LinkApplied;
use crate::document::Document;
use crate::error::EditError;
use crate::history::{Edit, TilePatch};
use crate::ids::{LayerId, TextureSetId};
use crate::layer::LayerKind;
use crate::link::paint::{TOO_LARGE_TO_LINK, layer_psd, normalise_in_place, normalised_crc, paint_source};
use crate::link::{FileStamp, LinkFile, LinkMode, LinkProblem, LinkState, MAX_PAINT_LINK_SIDE, PAINT_UPDATE_LABEL, PaintLink, state_of};
use crate::math::IRect;
use crate::pixel::PixelBuf;
use crate::psd::{PsdDoc, PsdError, write_sticker_to_vec};

/// A paint layer's link file content made by [`Document::prepare_layer_link_file`], not written yet (as
/// `PreparedLink` for a sticker).
#[derive(Clone, Debug)]
pub struct PreparedPaintLink {
    pub layer: LayerId,
    pub bytes: Vec<u8>,
    pub crc32: u32,
    /// The link as it will be once the file is written.
    pub link: PaintLink,
}

impl Document {
    // ----- queries -----

    /// The link of paint layer `id`.
    pub fn layer_link(&self, id: LayerId) -> Option<&PaintLink> {
        self.sticker_state.layer_links.get(&id)
    }

    /// Every layer link, ascending layer id (links of layers no longer in the document included: an undo can bring the
    /// layer back; a save keeps only links of existing raster layers).
    pub fn layer_links(&self) -> impl Iterator<Item = (LayerId, &PaintLink)> {
        self.sticker_state.layer_links.iter().map(|(id, l)| (*id, l))
    }

    /// The texture size of the set that holds layer `id`.
    pub fn layer_texture_size(&self, id: LayerId) -> Option<[u32; 2]> {
        let (set, _) = self.find_layer(id)?;
        self.set(set).map(|s| [s.width, s.height])
    }

    /// CRC-32 of raster layer `id`'s pixels at its texture's size, normalised (`link::paint::normalised_crc`); cached
    /// by layer version. `UnknownLayer`, `NotRaster`.
    pub fn layer_pixels_crc(&mut self, id: LayerId) -> Result<u32, EditError> {
        let layer = self.layer(id).ok_or(EditError::UnknownLayer)?;
        let version = layer.version();
        if let Some((v, crc)) = self.sticker_state.layer_crcs.get(&id)
            && *v == version
        {
            return Ok(*crc);
        }
        let [w, h] = self.layer_texture_size(id).ok_or(EditError::UnknownLayer)?;
        let LayerKind::Raster(buf) = &self.layer(id).ok_or(EditError::UnknownLayer)?.kind else { return Err(EditError::NotRaster) };
        let crc = normalised_crc(buf, w, h);
        self.sticker_state.layer_crcs.insert(id, (version, crc));
        Ok(crc)
    }

    /// True when layer `id` changed in WTE since its link's last sync (`wte_changed` of 5.5 for a paint layer).
    pub fn layer_link_changed_in_wte(&mut self, id: LayerId) -> bool {
        let Some(synced) = self.layer_link(id).map(|l| l.synced_pixels) else { return false };
        self.layer_crc_unless_stroking(id).is_ok_and(|crc| crc != Some(synced))
    }

    /// The state of layer `id`'s link (5.5) for the file stamp the app observed (`None` = the file is missing).
    pub fn layer_link_state(&mut self, id: LayerId, observed: Option<&FileStamp>) -> Result<LinkState, EditError> {
        let crc = self.layer_crc_unless_stroking(id)?;
        let link = self.layer_link(id).ok_or(EditError::InvalidParam("the layer is not linked"))?;
        Ok(state_of(&link.synced, link.problem.as_ref(), observed, crc != Some(link.synced_pixels)))
    }

    /// `layer_pixels_crc`, but `None` (= changed) while a texture stroke is open and the cached CRC is of an older
    /// version: a stroke on the layer gives it a new version every frame, and the badge must not hash the whole texture
    /// each frame; the stroke changes the layer anyway, and its end computes the CRC once.
    fn layer_crc_unless_stroking(&mut self, id: LayerId) -> Result<Option<u32>, EditError> {
        let version = self.layer(id).ok_or(EditError::UnknownLayer)?.version();
        let cached = self.sticker_state.layer_crcs.get(&id).filter(|(v, _)| *v == version).map(|(_, c)| *c);
        match cached {
            Some(c) => Ok(Some(c)),
            None if self.stroke_active() => Ok(None),
            None => self.layer_pixels_crc(id).map(Some),
        }
    }

    // ----- making and changing links -----

    /// The content of layer `id`'s link file (5.9): a one-layer PSD at the texture's size with the layer's pixels
    /// (straight, normalised; opacity 255, Normal) and the link as it will be once written, its file and editor
    /// `file` / `editor`. Changes nothing. Errors: `UnknownLayer`, `NotRaster` (a stamp or text layer: edit its sticker
    /// instead), `StickerLimit("This texture is too large to link.")` over 8192 a side.
    pub fn prepare_layer_link_file(&mut self, id: LayerId, file: LinkFile, editor: &str) -> Result<PreparedPaintLink, EditError> {
        let [w, h] = self.layer_texture_size(id).ok_or(EditError::UnknownLayer)?;
        if w > MAX_PAINT_LINK_SIDE || h > MAX_PAINT_LINK_SIDE || w == 0 || h == 0 {
            return Err(EditError::StickerLimit(TOO_LARGE_TO_LINK));
        }
        let crc = self.layer_pixels_crc(id)?;
        let layer = self.layer(id).ok_or(EditError::UnknownLayer)?;
        let LayerKind::Raster(buf) = &layer.kind else { return Err(EditError::NotRaster) };
        let psd = layer_psd(&layer.name, buf, w, h);
        let bytes = write_sticker_to_vec(&psd).map_err(|e| match e {
            PsdError::Invalid(why) => EditError::InvalidParam(why),
            _ => EditError::InvalidParam("the layer cannot be written as a PSD file"),
        })?;
        let synced = FileStamp::of_bytes(&bytes, None);
        let link = PaintLink { file, editor: editor.to_owned(), synced, synced_pixels: crc, editor_layers: false, problem: None };
        Ok(PreparedPaintLink { layer: id, crc32: synced.crc32, bytes, link })
    }

    /// Takes a prepared layer link file that was written: the link is made or updated, in sync. `InvalidParam` when
    /// the written content is not the prepared one.
    pub fn finish_layer_link_write(&mut self, prepared: PreparedPaintLink, written: FileStamp) -> Result<(), EditError> {
        if !written.same_content(&prepared.link.synced) {
            return Err(EditError::InvalidParam("the written link file is not the prepared one"));
        }
        let mut link = prepared.link;
        link.synced = written;
        self.put_layer_link(prepared.layer, Some(link));
        Ok(())
    }

    /// "Unlink" of a paint layer (the pixels stay, the PSD stays on disk). No-op without a link.
    pub fn unlink_layer(&mut self, id: LayerId) {
        if self.sticker_state.layer_links.contains_key(&id) {
            self.put_layer_link(id, None);
        }
    }

    /// The editor set at an "Edit Layer in" (13.1), or another file.
    pub fn set_layer_link_file_and_editor(&mut self, id: LayerId, file: Option<LinkFile>, editor: Option<&str>) -> Result<(), EditError> {
        let mut link = self.layer_link(id).cloned().ok_or(EditError::InvalidParam("the layer is not linked"))?;
        if let Some(file) = file {
            link.file = file;
        }
        if let Some(editor) = editor {
            editor.clone_into(&mut link.editor);
        }
        if self.layer_link(id) != Some(&link) {
            self.put_layer_link(id, Some(link));
        }
        Ok(())
    }

    // ----- applying the file -----

    /// Applies a file the watcher read to linked layer `id` (5.9): the file's picture ([`paint_source`]) replaces the
    /// layer's pixels as one undo step "Update Linked Layer" (none when nothing changed); the link is in sync with
    /// `stamp` afterwards (outside the history, R6). A file that cannot be applied (another canvas size, needs a merged
    /// picture, too large) changes nothing but the link's problem; the result is the badge state. Errors:
    /// `UnknownLayer`, `InvalidParam` (not linked), `NotRaster`, `LayerLocked`, `StrokeInProgress`.
    pub fn apply_layer_link_update(&mut self, id: LayerId, psd: &PsdDoc, stamp: FileStamp, editor_name: &str, file_name: &str) -> Result<LinkApplied, EditError> {
        let _ = (editor_name, file_name);
        let mut link = self.layer_link(id).cloned().ok_or(EditError::InvalidParam("the layer is not linked"))?;
        let texture = self.layer_texture_size(id).ok_or(EditError::UnknownLayer)?;
        if !self.layer(id).is_some_and(|l| l.is_raster()) {
            return Err(EditError::NotRaster);
        }
        let picture = match paint_source(psd, texture, &link.editor) {
            Ok(p) => p,
            Err(problem) => {
                link.problem = Some(problem.into_link_problem(stamp.crc32));
                self.put_layer_link(id, Some(link));
                return Ok(LinkApplied::Problem(self.layer_link_state(id, Some(&stamp))?));
            }
        };
        self.replace_layer_pixels(id, picture.pixels, PAINT_UPDATE_LABEL)?;
        link.synced = stamp;
        link.synced_pixels = self.layer_pixels_crc(id)?;
        link.editor_layers = picture.editor_layers;
        link.problem = None;
        self.put_layer_link(id, Some(link));
        let profile_note = psd.profile_note().map(|d| crate::link::texts::profile_note(file_name, d));
        let mode = if picture.merged { LinkMode::Merged } else { LinkMode::Layers };
        Ok(LinkApplied::Updated { mode, toast: None, profile_note, log: Vec::new() })
    }

    /// Records that the file with `stamp` could not be read (a refused colour mode or depth, too large, damaged); a
    /// truncated file is not a problem. Returns the badge state.
    pub fn layer_link_file_refused(&mut self, id: LayerId, err: &PsdError, stamp: FileStamp, file_name: &str) -> Result<LinkState, EditError> {
        let mut link = self.layer_link(id).cloned().ok_or(EditError::InvalidParam("the layer is not linked"))?;
        let crc32 = stamp.crc32;
        link.problem = match err {
            PsdError::Unsupported(r) => Some(LinkProblem::Refused { crc32, sentence: r.sentence(file_name) }),
            PsdError::TooLarge { width, height } => Some(LinkProblem::TooLarge { crc32, width: *width, height: *height }),
            PsdError::Truncated => None,
            other => Some(LinkProblem::Unreadable { crc32, reason: other.to_string() }),
        };
        self.put_layer_link(id, Some(link));
        self.layer_link_state(id, Some(&stamp))
    }

    // ----- layer commands (P12) -----

    /// Replaces the pixels of raster layer `layer` with `pixels` (exactly the texture's size, straight RGBA) as one
    /// undo step `label` (STICKER_PLAN 5.9): an `Edit::Pixels` whose patch holds only the tiles that changed; caches
    /// and views update as after a stroke. Returns false (no entry) when no pixel changed. Errors: `UnknownLayer`,
    /// `NotRaster`, `LayerLocked`, `StrokeInProgress`, `InvalidParam` for another size.
    pub fn replace_layer_pixels(&mut self, layer: LayerId, pixels: PixelBuf, label: &str) -> Result<bool, EditError> {
        self.guard_no_stroke()?;
        self.flush_pending_edit();
        let (set, index) = self.find_layer(layer).ok_or(EditError::UnknownLayer)?;
        let target = self.set(set).ok_or(EditError::UnknownSet)?;
        let (w, h) = (target.width, target.height);
        let info = &target.layers[index];
        if !info.is_raster() {
            return Err(EditError::NotRaster);
        }
        if info.locked {
            return Err(EditError::LayerLocked);
        }
        if pixels.width != w || pixels.height != h || pixels.data.len() != w as usize * h as usize * 4 {
            return Err(EditError::InvalidParam("the picture is not the texture's size"));
        }
        let unallocated = info.raster().is_some_and(|b| b.data.is_empty());
        if unallocated && pixels.data.iter().all(|b| *b == 0) {
            return Ok(false);
        }
        self.sets[set.index()].allocate_raster(index);
        let LayerKind::Raster(buf) = &mut self.sets[set.index()].layers[index].kind else { return Err(EditError::NotRaster) };
        let patch = TilePatch::diff(&**buf, &pixels);
        if patch.is_empty() {
            return Ok(false);
        }
        *buf = Arc::new(pixels);
        let tile = TilePatch::TILE as i32;
        let dirty = patch.tiles.iter().fold(IRect::EMPTY, |acc, t| {
            let (x, y) = (i32::from(t.tx) * tile, i32::from(t.ty) * tile);
            acc.union(&IRect::new(x, y, x + tile, y + tile))
        });
        self.mark_layer_dirty(layer, dirty);
        self.push_edit(label, Edit::Pixels { set, layer, patch });
        Ok(true)
    }

    /// A new raster layer `name` above the active layer of `set` holding `pixels` (the texture's size), active, as one
    /// undo step `label` (Paste as New Paint Layer, 5.9). Errors: `UnknownSet`, `StrokeInProgress`, `InvalidParam`
    /// for another size.
    pub fn add_paint_layer_with_pixels(&mut self, set: TextureSetId, name: &str, pixels: PixelBuf, label: &str) -> Result<LayerId, EditError> {
        let target = self.set(set).ok_or(EditError::UnknownSet)?;
        if pixels.width != target.width || pixels.height != target.height || pixels.data.len() != pixels.width as usize * pixels.height as usize * 4 {
            return Err(EditError::InvalidParam("the picture is not the texture's size"));
        }
        self.guard_no_stroke()?;
        let group = self.begin_edit_group();
        let made = self.add_raster_layer(set, Some(name)).and_then(|id| self.replace_layer_pixels(id, pixels, label).map(|_| id));
        self.end_edit_group(group, label);
        made
    }

    // ----- internals -----

    /// Sets or removes a layer link; a change bumps the shared links revision (a change of `problem` alone does not:
    /// problems are not saved).
    pub(crate) fn put_layer_link(&mut self, id: LayerId, link: Option<PaintLink>) {
        let st = &mut self.sticker_state;
        let saved_part = |l: &PaintLink| PaintLink { problem: None, ..l.clone() };
        let changed = match link {
            Some(link) => st.layer_links.insert(id, link.clone()).is_none_or(|old| saved_part(&old) != saved_part(&link)),
            None => st.layer_links.remove(&id).is_some(),
        };
        if changed {
            st.links_revision += 1;
        }
    }

    /// Replaces the layer link map without touching the revision (load).
    pub(crate) fn set_loaded_layer_links(&mut self, links: std::collections::BTreeMap<LayerId, PaintLink>) {
        self.sticker_state.layer_links = links;
    }
}

/// `src` scaled to fit into `width x height` with its aspect kept and centred on a transparent picture (Paste as New
/// Paint Layer of another size, 5.9; the resampler of 4.7: premultiplied area / bilinear). A picture of exactly that
/// size is returned as it is. The result is normalised (zero RGB under alpha 0).
pub fn fit_centred(src: &PixelBuf, width: u32, height: u32) -> PixelBuf {
    if src.width == width && src.height == height {
        let mut out = src.clone();
        normalise_in_place(&mut out.data);
        return out;
    }
    let mut out = PixelBuf::new(width, height);
    if src.width == 0 || src.height == 0 || width == 0 || height == 0 {
        return out;
    }
    let s = (width as f64 / src.width as f64).min(height as f64 / src.height as f64);
    let fw = ((src.width as f64 * s).round() as u32).clamp(1, width);
    let fh = ((src.height as f64 * s).round() as u32).clamp(1, height);
    let scaled = crate::pixel_ops::resize(src, fw, fh);
    let (x0, y0) = ((width - fw) / 2, (height - fh) / 2);
    for y in 0..fh {
        let dst = (x0 as usize) * 4;
        out.row_mut(y0 + y)[dst..dst + fw as usize * 4].copy_from_slice(scaled.row(y));
    }
    normalise_in_place(&mut out.data);
    out
}

#[cfg(test)]
#[path = "doc_layer_links_tests.rs"]
mod tests;
